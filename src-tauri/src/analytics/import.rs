//! 快照导入（契约 §1）。
//!
//! 扫描 `<submissionsRoot>/snapshots/<terminalId>/*.ready`，每个 `.ready` 对应一组
//! `<attemptId>.sqlite` + `<attemptId>.meta.json`。流程：
//!   1. 读 meta.json，校验 schemaVersion；
//!   2. 把 .sqlite 复制到本地临时目录，校验副本的 sizeBytes 与 sha256；
//!   3. 同一 (文件, sha256) 已成功导入 → skipped；
//!   4. 只读打开副本，PRAGMA quick_check；
//!   5. 在一个事务里把整库里已交卷的 attempt 按 attempt_id UPSERT 进分析库，
//!      同一 attempt 只接受 exportedAt 更新的快照；
//!   6. 记录 import_snapshots。单个快照失败只影响它自己（回滚 + 记 failed）。
//!
//! 只读 NAS：只 read 源文件，所有写入都在本地 analytics.sqlite 与临时目录。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::CommandResult;

pub const SNAPSHOT_SCHEMA_VERSION: &str = "SubmissionSnapshotV1";
/// 作者端支持的学生端库结构版本（exam_kv.db_schema_version）。
pub const SUPPORTED_DB_SCHEMA_VERSION: u64 = 2;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotOutcome {
    pub file: String,
    pub status: String,
    pub attempt_id: Option<String>,
    pub attempts_imported: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub scanned: usize,
    pub ok: usize,
    pub skipped: usize,
    pub failed: usize,
    pub attempts_imported: usize,
    pub outcomes: Vec<SnapshotOutcome>,
    pub finished_at: String,
    /// 作答数据目录下是否存在 snapshots 目录（不存在时提示「尚未有快照」）。
    pub snapshots_dir_found: bool,
}

/// 一组 ready 快照。
struct Candidate {
    file_key: String,
    stem: String,
    sqlite_path: PathBuf,
    meta_path: PathBuf,
}

pub fn import_submission_snapshots(
    conn: &mut Connection,
    app_root: &Path,
    submissions_root: &Path,
) -> CommandResult<ImportReport> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut report = ImportReport { finished_at: now.clone(), ..ImportReport::default() };
    let snapshots_dir = submissions_root.join("snapshots");
    if !snapshots_dir.is_dir() {
        return Ok(report);
    }
    report.snapshots_dir_found = true;

    // Each concurrent import owns its copies and cleanup.
    let tmp_dir = app_root.join("analytics-tmp").join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&tmp_dir).map_err(|error| format!("analytics_tmp:{error}"))?;

    for (candidate, ready) in scan_candidates(&snapshots_dir)? {
        report.scanned += 1;
        let outcome = if ready {
            import_one(conn, &tmp_dir, &candidate, &now)
        } else {
            // 未就绪（缺 .ready）：不读取、不导入，只进异常队列；补齐 .ready 后下次导入会自动处理。
            let hints = read_hints_best_effort(&candidate.meta_path);
            let error = "缺少 .ready 标记：快照尚未就绪，已忽略".to_string();
            let _ = record_import(conn, &candidate.file_key, "", &hints, &now, "failed", Some(&error));
            SnapshotOutcome {
                file: candidate.file_key.clone(),
                status: "failed".into(),
                attempt_id: Some(candidate.stem.clone()),
                attempts_imported: 0,
                error: Some(error),
            }
        };
        match outcome.status.as_str() {
            "ok" => report.ok += 1,
            "skipped" => report.skipped += 1,
            _ => report.failed += 1,
        }
        report.attempts_imported += outcome.attempts_imported;
        report.outcomes.push(outcome);
    }
    let _ = fs::remove_dir_all(&tmp_dir);
    Ok(report)
}

/// 返回 (候选, 是否 ready)。缺 .ready 的 .sqlite 也作为候选返回，由调用方记为未就绪异常。
fn scan_candidates(snapshots_dir: &Path) -> CommandResult<Vec<(Candidate, bool)>> {
    let mut items = Vec::new();
    let mut terminals: Vec<PathBuf> = fs::read_dir(snapshots_dir)
        .map_err(|error| format!("snapshots_read:{error}"))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    terminals.sort();
    for terminal_dir in terminals {
        let terminal_id = terminal_dir
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut ready = BTreeSet::new();
        let mut sqlite_stems = BTreeSet::new();
        for entry in fs::read_dir(&terminal_dir).map_err(|error| format!("terminal_read:{error}"))? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            // `.<name>.<rand>.partial` 等临时文件以点开头，一律忽略。
            if name.starts_with('.') {
                continue;
            }
            if let Some(stem) = name.strip_suffix(".ready") {
                ready.insert(stem.to_string());
            } else if let Some(stem) = name.strip_suffix(".sqlite") {
                sqlite_stems.insert(stem.to_string());
            }
        }
        for stem in &ready {
            items.push((candidate_for(&terminal_id, &terminal_dir, stem), true));
        }
        for stem in sqlite_stems.difference(&ready) {
            items.push((candidate_for(&terminal_id, &terminal_dir, stem), false));
        }
    }
    Ok(items)
}

fn candidate_for(terminal_id: &str, terminal_dir: &Path, stem: &str) -> Candidate {
    Candidate {
        file_key: format!("snapshots/{terminal_id}/{stem}.sqlite"),
        stem: stem.to_string(),
        sqlite_path: terminal_dir.join(format!("{stem}.sqlite")),
        meta_path: terminal_dir.join(format!("{stem}.meta.json")),
    }
}

fn import_one(conn: &mut Connection, tmp_dir: &Path, candidate: &Candidate, now: &str) -> SnapshotOutcome {
    match import_one_inner(conn, tmp_dir, candidate, now) {
        Ok(outcome) => outcome,
        Err(error) => {
            // 失败记录尽量保留 meta 里的批次/卷，异常队列才能按批次归属。
            let hints = read_hints_best_effort(&candidate.meta_path);
            let _ = record_import(conn, &candidate.file_key, "", &hints, now, "failed", Some(&error));
            SnapshotOutcome {
                file: candidate.file_key.clone(),
                status: "failed".into(),
                attempt_id: Some(candidate.stem.clone()),
                attempts_imported: 0,
                error: Some(error),
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
struct MetaHints {
    terminal_id: Option<String>,
    attempt_id: Option<String>,
    batch_id: Option<String>,
    paper_id: Option<String>,
    exported_at: Option<String>,
    install_id: Option<String>,
}

fn read_hints_best_effort(meta_path: &Path) -> MetaHints {
    let Ok(text) = fs::read_to_string(meta_path) else {
        return MetaHints::default();
    };
    let Ok(meta) = serde_json::from_str::<Value>(&text) else {
        return MetaHints::default();
    };
    MetaHints {
        terminal_id: string_field(&meta, "terminalId"),
        attempt_id: string_field(&meta, "attemptId"),
        batch_id: string_field(&meta, "batchId"),
        paper_id: string_field(&meta, "paperId"),
        exported_at: string_field(&meta, "exportedAt"),
        install_id: string_field(&meta, "installId"),
    }
}

fn import_one_inner(
    conn: &mut Connection,
    tmp_dir: &Path,
    candidate: &Candidate,
    now: &str,
) -> Result<SnapshotOutcome, String> {
    let meta_text = fs::read_to_string(&candidate.meta_path)
        .map_err(|error| format!("meta.json 不可读：{error}"))?;
    let meta: Value =
        serde_json::from_str(&meta_text).map_err(|error| format!("meta.json 解析失败：{error}"))?;
    let schema = meta.get("schemaVersion").and_then(Value::as_str).ok_or("快照缺少 schemaVersion")?;
    // 版本过新：不导入，记入异常队列（契约 §1.1）。
    let db_schema_version = match meta.get("dbSchemaVersion") {
        None if schema == SNAPSHOT_SCHEMA_VERSION => 1,
        Some(v) => v.as_u64().ok_or("快照 dbSchemaVersion 格式错误")?,
        None => return Err("快照缺少 dbSchemaVersion".into()),
    };
    if db_schema_version > SUPPORTED_DB_SCHEMA_VERSION {
        return Err(format!(
            "快照版本过新（dbSchemaVersion={db_schema_version}，作者端支持至 {SUPPORTED_DB_SCHEMA_VERSION}），请升级作者端"
        ));
    }
    if !matches!((schema, db_schema_version), ("SubmissionSnapshotV1", 1) | ("SubmissionSnapshotV2", 2)) {
        return Err("快照 schemaVersion 与 dbSchemaVersion 不一致或不受支持".into());
    }
    let hints = MetaHints {
        terminal_id: string_field(&meta, "terminalId"),
        attempt_id: string_field(&meta, "attemptId"),
        batch_id: string_field(&meta, "batchId"),
        paper_id: string_field(&meta, "paperId"),
        exported_at: string_field(&meta, "exportedAt"),
        install_id: string_field(&meta, "installId"),
    };
    let expected_sha = string_field(&meta, "sha256")
        .map(|value| value.to_ascii_lowercase())
        .ok_or("meta.json 缺少 sha256")?;
    let expected_size = meta
        .get("sizeBytes")
        .and_then(Value::as_u64)
        .ok_or("meta.json 缺少 sizeBytes")?;
    let attempt_hint = hints.attempt_id.clone().ok_or("meta.json 缺少 attemptId")?;
    let exported_at = hints.exported_at.clone().ok_or("meta.json 缺少 exportedAt")?;

    // 复制到本地临时目录（源 NAS 只读，写入只发生在本地）。
    let source_bytes = fs::read(&candidate.sqlite_path).map_err(|error| format!("快照不可读：{error}"))?;
    let local_copy = tmp_dir.join(format!("{}.sqlite", uuid::Uuid::new_v4()));
    fs::write(&local_copy, &source_bytes).map_err(|error| format!("复制到临时目录失败：{error}"))?;
    drop(source_bytes);
    let result = verify_and_import(
        conn,
        &local_copy,
        candidate,
        &hints,
        &attempt_hint,
        &exported_at,
        expected_size,
        &expected_sha,
        now,
        db_schema_version,
    );
    let _ = fs::remove_file(&local_copy);
    result
}

#[allow(clippy::too_many_arguments)]
fn verify_and_import(
    conn: &mut Connection,
    local_copy: &Path,
    candidate: &Candidate,
    hints: &MetaHints,
    attempt_hint: &str,
    exported_at: &str,
    expected_size: u64,
    expected_sha: &str,
    now: &str,
    db_schema_version: u64,
) -> Result<SnapshotOutcome, String> {
    let file = candidate.file_key.clone();
    let copy_bytes = fs::read(local_copy).map_err(|error| format!("读取临时副本失败：{error}"))?;
    if copy_bytes.len() as u64 != expected_size {
        return Err(format!("sizeBytes 不符：meta={expected_size} 实际={}", copy_bytes.len()));
    }
    let actual_sha = sha256_hex(&copy_bytes);
    drop(copy_bytes);
    if actual_sha != expected_sha {
        return Err(format!("sha256 不符：meta={expected_sha} 实际={actual_sha}"));
    }

    // 同一文件内容已成功导入 → skipped（只刷新导入时间，不重复写库）。
    let already_ok: Option<String> = conn
        .query_row(
            "SELECT status FROM import_snapshots WHERE file = ?1 AND sha256 = ?2 AND status = 'ok'",
            params![file, actual_sha],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if already_ok.is_some() {
        conn.execute(
            "UPDATE import_snapshots SET imported_at = ?3 WHERE file = ?1 AND sha256 = ?2",
            params![file, actual_sha, now],
        )
        .map_err(|error| error.to_string())?;
        return Ok(SnapshotOutcome {
            file,
            status: "skipped".into(),
            attempt_id: Some(attempt_hint.to_string()),
            attempts_imported: 0,
            error: Some("同一内容已导入".into()),
        });
    }

    let source = read_source_db(local_copy, db_schema_version)?;
    if !source.attempts.iter().any(|attempt| attempt.attempt_id == attempt_hint) {
        return Err(format!("快照内找不到已交卷的 attempt：{attempt_hint}"));
    }

    let tx = conn.transaction().map_err(|error| error.to_string())?;
    let mut imported = 0usize;
    // 异常（attemptId, 说明）：不影响其他 attempt 的导入，只进异常队列。
    let mut anomalies: Vec<(String, String)> = source.paper_conflicts.clone();
    for attempt in &source.attempts {
        let existing: Option<(String, String, Option<String>, String)> = tx
            .query_row(
                "SELECT exported_at, terminal_id, install_id, paper_id FROM attempts WHERE attempt_id = ?1",
                params![attempt.attempt_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        // 套题 ID 按 attempt 归属：优先快照里的 exam_kv / 套题聚合 / 锁定序列，其次（仅对本快照指向的 attempt）meta.paperId。
        // 整库里别的 attempt 不能套用这份 meta 的 paperId。
        let incoming_paper = source
            .paper_ids
            .get(&attempt.attempt_id)
            .cloned()
            .or_else(|| {
                if attempt.attempt_id == attempt_hint {
                    hints.paper_id.clone()
                } else {
                    None
                }
            })
            .unwrap_or_default();
        if let Some((existing_exported, existing_terminal, existing_install, existing_paper)) = existing {
            let identity_conflict = if existing_terminal != attempt.terminal_id {
                Some(format!(
                    "同一 attemptId 来自不同终端（已导入 {existing_terminal}，本快照 {}），未覆盖",
                    attempt.terminal_id
                ))
            } else {
                match (existing_install.as_deref(), hints.install_id.as_deref()) {
                    (Some(old), Some(new)) if old != new => {
                        Some(format!("同一 attemptId 来自不同安装（已导入 {old}，本快照 {new}），未覆盖"))
                    }
                    _ => None,
                }
            };
            if let Some(message) = identity_conflict {
                if attempt.attempt_id == attempt_hint {
                    return Err(message);
                }
                anomalies.push((attempt.attempt_id.clone(), message));
                continue;
            }
            if !incoming_paper.is_empty() && !existing_paper.is_empty() && incoming_paper != existing_paper {
                anomalies.push((
                    attempt.attempt_id.clone(),
                    format!("套题归属与已导入记录不一致（已导入 {existing_paper}，本快照 {incoming_paper}），保留已导入值"),
                ));
            }
            if !is_newer(exported_at, &existing_exported) {
                continue;
            }
        }
        write_attempt(
            &tx,
            attempt,
            &incoming_paper,
            hints.install_id.as_deref(),
            &source,
            exported_at,
            &actual_sha,
            &file,
            now,
        )?;
        imported += 1;
    }
    for (attempt_id, message) in &anomalies {
        record_anomaly(&tx, &file, attempt_id, hints, now, message)?;
    }
    let status = if imported > 0 { "ok" } else { "skipped" };
    let error = if imported > 0 {
        None
    } else {
        Some("快照中的 attempt 均不新于已导入版本（exportedAt 未更新）".to_string())
    };
    record_import(&tx, &file, &actual_sha, hints, now, status, error.as_deref())?;
    if imported > 0 {
        tx.execute("DELETE FROM import_snapshots WHERE file = ?1 AND status = 'failed'", params![file])
            .map_err(|error| error.to_string())?;
    }
    tx.commit().map_err(|error| error.to_string())?;
    Ok(SnapshotOutcome {
        file,
        status: status.into(),
        attempt_id: Some(attempt_hint.to_string()),
        attempts_imported: imported,
        error,
    })
}

#[allow(clippy::too_many_arguments)]
fn write_attempt(
    tx: &rusqlite::Transaction,
    attempt: &SrcAttempt,
    paper_id: &str,
    install_id: Option<&str>,
    source: &SourceDb,
    exported_at: &str,
    sha: &str,
    file: &str,
    now: &str,
) -> Result<(), String> {
    let id = attempt.attempt_id.as_str();
    let original = super::immutable::ImportedFacts::read(tx, id)?;
    // 子表整组替换：先清掉该 attempt 的旧子行，再 UPSERT 新快照的行。
    for table in ["answers", "passages", "writings", "suites"] {
        tx.execute(&format!("DELETE FROM {table} WHERE attempt_id = ?1"), params![id])
            .map_err(|error| error.to_string())?;
    }
    tx.execute(
        "INSERT INTO attempts (attempt_id, terminal_id, batch_id, paper_id, candidate_id, candidate_name,
            checkin_at, started_at, submitted_at, status, app_version, scorer_version,
            exported_at, sha256, source_file, imported_at, install_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT(attempt_id) DO UPDATE SET
            terminal_id = excluded.terminal_id, batch_id = excluded.batch_id,
            paper_id = CASE WHEN attempts.paper_id <> '' THEN attempts.paper_id ELSE excluded.paper_id END,
            install_id = COALESCE(attempts.install_id, excluded.install_id),
            candidate_id = excluded.candidate_id, candidate_name = excluded.candidate_name,
            checkin_at = excluded.checkin_at, started_at = excluded.started_at,
            submitted_at = excluded.submitted_at, status = excluded.status,
            app_version = excluded.app_version, scorer_version = excluded.scorer_version,
            exported_at = excluded.exported_at, sha256 = excluded.sha256,
            source_file = excluded.source_file, imported_at = excluded.imported_at",
        params![
            id,
            attempt.terminal_id,
            attempt.batch_id,
            paper_id,
            attempt.candidate_id,
            attempt.candidate_name,
            attempt.checkin_at,
            attempt.started_at,
            attempt.submitted_at,
            attempt.status,
            attempt.app_version,
            source.scorer_versions.get(id),
            exported_at,
            sha,
            file,
            now,
            install_id
        ],
    )
    .map_err(|error| format!("写入 attempts 失败：{error}"))?;

    for suite in source.suites.iter().filter(|suite| suite.attempt_id == id) {
        tx.execute(
            "INSERT INTO suites (suite_attempt_id, attempt_id, status, started_at, completed_at, submitted_at, total_passages)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(suite_attempt_id) DO UPDATE SET
                attempt_id = excluded.attempt_id, status = excluded.status, started_at = excluded.started_at,
                completed_at = excluded.completed_at, submitted_at = excluded.submitted_at,
                total_passages = excluded.total_passages",
            params![
                suite.suite_attempt_id,
                suite.attempt_id,
                suite.status,
                suite.started_at,
                suite.completed_at,
                suite.submitted_at,
                suite.total_passages
            ],
        )
        .map_err(|error| format!("写入 suites 失败：{error}"))?;
    }
    for passage in source.passages.iter().filter(|passage| passage.attempt_id == id) {
        let score = crate::analytics::parse_score_info(&passage.score_info_json).ok_or_else(|| {
            format!("score_info_json 无法解析（passage {}）", passage.passage_index)
        })?;
        tx.execute(
            "INSERT INTO passages (attempt_id, passage_index, suite_attempt_id, asset_id, exam_id, title, category,
                started_at, ended_at, duration_sec, gradable, correct, total, percentage)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(attempt_id, passage_index) DO UPDATE SET
                suite_attempt_id = excluded.suite_attempt_id, asset_id = excluded.asset_id, exam_id = excluded.exam_id,
                title = excluded.title, category = excluded.category, started_at = excluded.started_at,
                ended_at = excluded.ended_at, duration_sec = excluded.duration_sec, gradable = excluded.gradable,
                correct = excluded.correct, total = excluded.total, percentage = excluded.percentage",
            params![
                id,
                passage.passage_index,
                passage.suite_attempt_id,
                passage.asset_id,
                passage.exam_id,
                passage.title,
                passage.category,
                passage.started_at,
                passage.ended_at,
                passage.duration_sec,
                if score.gradable { 1 } else { 0 },
                score.correct,
                score.total,
                score.percentage
            ],
        )
        .map_err(|error| format!("写入 passages 失败：{error}"))?;
    }
    for answer in source.answers.iter().filter(|answer| answer.attempt_id == id) {
        tx.execute(
            "INSERT INTO answers (attempt_id, passage_index, question_id, display_label, question_kind, control,
                match_mode, user_answer_json, correct_answer_json, normalized_user_answer_json,
                normalized_correct_answer_json, is_correct, weight, first_answered_at, last_answered_at,
                change_count, visit_count, elapsed_ms, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
             ON CONFLICT(attempt_id, passage_index, question_id) DO UPDATE SET
                display_label = excluded.display_label, question_kind = excluded.question_kind,
                control = excluded.control, match_mode = excluded.match_mode,
                user_answer_json = excluded.user_answer_json, correct_answer_json = excluded.correct_answer_json,
                normalized_user_answer_json = excluded.normalized_user_answer_json,
                normalized_correct_answer_json = excluded.normalized_correct_answer_json,
                is_correct = excluded.is_correct, weight = excluded.weight,
                first_answered_at = excluded.first_answered_at, last_answered_at = excluded.last_answered_at,
                change_count = excluded.change_count, visit_count = excluded.visit_count,
                elapsed_ms = excluded.elapsed_ms, duration_ms = excluded.duration_ms",
            params![
                id,
                answer.passage_index,
                answer.question_id,
                answer.display_label,
                answer.question_kind,
                answer.control,
                answer.match_mode,
                answer.user_answer_json,
                answer.correct_answer_json,
                answer.normalized_user_answer_json,
                answer.normalized_correct_answer_json,
                answer.is_correct,
                answer.weight,
                answer.first_answered_at,
                answer.last_answered_at,
                answer.change_count,
                answer.visit_count,
                answer.elapsed_ms,
                answer.duration_ms
            ],
        )
        .map_err(|error| format!("写入 answers 失败：{error}"))?;
    }
    for writing in source.writings.iter().filter(|writing| writing.attempt_id == id) {
        tx.execute(
            "INSERT INTO writings (attempt_id, writing_submission_id, task_type, word_count, started_at, submitted_at, duration_sec)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(attempt_id, writing_submission_id) DO UPDATE SET
                task_type = excluded.task_type, word_count = excluded.word_count, started_at = excluded.started_at,
                submitted_at = excluded.submitted_at, duration_sec = excluded.duration_sec",
            params![
                id,
                writing.writing_submission_id,
                writing.task_type,
                writing.word_count,
                writing.started_at,
                writing.submitted_at,
                writing.duration_sec
            ],
        )
        .map_err(|error| format!("写入 writings 失败：{error}"))?;
    }
    source.extensions.apply(tx, id)?;
    if let Some(original) = original { original.verify(tx, id)?; }
    Ok(())
}

/// 记一条 attempt 级异常（归属冲突、终端/安装不一致等）。文件键带 `#attempt:<id>`，与快照级记录互不覆盖。
fn record_anomaly(
    conn: &Connection,
    file: &str,
    attempt_id: &str,
    hints: &MetaHints,
    now: &str,
    message: &str,
) -> Result<(), String> {
    let key = format!("{file}#attempt:{attempt_id}");
    let attempt_hints = MetaHints { attempt_id: Some(attempt_id.to_string()), ..hints.clone() };
    record_import(conn, &key, "anomaly", &attempt_hints, now, "failed", Some(message))
}

/// 写 import_snapshots。`conn` 既可以是连接也可以是事务（经 Deref 到 Connection）。
fn record_import(
    conn: &Connection,
    file: &str,
    sha: &str,
    hints: &MetaHints,
    now: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO import_snapshots (file, sha256, terminal_id, attempt_id, batch_id, paper_id, exported_at, imported_at, status, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(file, sha256) DO UPDATE SET
            terminal_id = excluded.terminal_id, attempt_id = excluded.attempt_id, batch_id = excluded.batch_id,
            paper_id = excluded.paper_id, exported_at = excluded.exported_at, imported_at = excluded.imported_at,
            status = excluded.status, error = excluded.error",
        params![
            file,
            sha,
            hints.terminal_id,
            hints.attempt_id,
            hints.batch_id,
            hints.paper_id,
            hints.exported_at,
            now,
            status,
            error
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// 快照库里读出的已交卷 attempt 及子表（只读副本）。
struct SourceDb {
    extensions: super::import_extensions::Extensions,
    attempts: Vec<SrcAttempt>,
    suites: Vec<SrcSuite>,
    passages: Vec<SrcPassage>,
    answers: Vec<SrcAnswer>,
    writings: Vec<SrcWriting>,
    scorer_versions: HashMap<String, String>,
    paper_ids: HashMap<String, String>,
    /// (attemptId, 异常说明)：同一 attempt 的套题归属来源互相矛盾。
    paper_conflicts: Vec<(String, String)>,
}

struct SrcAttempt {
    attempt_id: String,
    terminal_id: String,
    batch_id: String,
    candidate_id: String,
    candidate_name: String,
    checkin_at: Option<String>,
    started_at: Option<String>,
    submitted_at: Option<String>,
    status: String,
    app_version: Option<String>,
}

struct SrcSuite {
    suite_attempt_id: String,
    attempt_id: String,
    status: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    submitted_at: Option<String>,
    total_passages: i64,
}

struct SrcPassage {
    attempt_id: String,
    suite_attempt_id: String,
    passage_index: i64,
    asset_id: Option<String>,
    exam_id: Option<String>,
    title: Option<String>,
    category: Option<String>,
    started_at: Option<String>,
    ended_at: Option<String>,
    duration_sec: Option<i64>,
    score_info_json: String,
}

struct SrcAnswer {
    attempt_id: String,
    passage_index: i64,
    question_id: String,
    display_label: Option<String>,
    question_kind: Option<String>,
    control: Option<String>,
    match_mode: Option<String>,
    user_answer_json: Option<String>,
    correct_answer_json: Option<String>,
    normalized_user_answer_json: Option<String>,
    normalized_correct_answer_json: Option<String>,
    is_correct: Option<i64>,
    weight: f64,
    first_answered_at: Option<String>,
    last_answered_at: Option<String>,
    change_count: i64,
    visit_count: i64,
    elapsed_ms: i64,
    duration_ms: i64,
}

struct SrcWriting {
    attempt_id: String,
    writing_submission_id: String,
    task_type: Option<String>,
    word_count: Option<i64>,
    started_at: Option<String>,
    submitted_at: Option<String>,
    duration_sec: Option<i64>,
}

/// 只读打开副本，读出已交卷的数据。失败即整份快照 failed。
fn read_source_db(path: &Path, version: u64) -> Result<SourceDb, String> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("副本无法只读打开：{error}"))?;
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| format!("quick_check 失败：{error}"))?;
    if check != "ok" {
        return Err(format!("quick_check 未通过：{check}"));
    }
    let attempts = query_rows(
        &conn,
        "SELECT attempt_id, batch_id, terminal_id, candidate_id, candidate_name, checkin_at, started_at,
                submitted_at, status, app_version
         FROM exam_attempts WHERE status IN ('submitted','failed')",
        |row| {
            Ok(SrcAttempt {
                attempt_id: row.get(0)?,
                batch_id: row.get(1)?,
                terminal_id: row.get(2)?,
                candidate_id: row.get(3)?,
                candidate_name: row.get(4)?,
                checkin_at: row.get(5)?,
                started_at: row.get(6)?,
                submitted_at: row.get(7)?,
                status: row.get(8)?,
                app_version: row.get(9)?,
            })
        },
    )?;
    let submitted: HashSet<String> = attempts.iter().map(|attempt| attempt.attempt_id.clone()).collect();

    let suites: Vec<SrcSuite> = query_rows(
        &conn,
        "SELECT suite_attempt_id, attempt_id, status, started_at, completed_at, submitted_at, total_passages
         FROM reading_suite_attempts",
        |row| {
            Ok(SrcSuite {
                suite_attempt_id: row.get(0)?,
                attempt_id: row.get(1)?,
                status: row.get(2)?,
                started_at: row.get(3)?,
                completed_at: row.get(4)?,
                submitted_at: row.get(5)?,
                total_passages: row.get::<_, Option<i64>>(6)?.unwrap_or(3),
            })
        },
    )?
    .into_iter()
    .filter(|suite| submitted.contains(&suite.attempt_id))
    .collect();
    let suite_to_attempt: HashMap<String, String> = suites
        .iter()
        .map(|suite| (suite.suite_attempt_id.clone(), suite.attempt_id.clone()))
        .collect();

    let passages: Vec<SrcPassage> = query_rows(
        &conn,
        "SELECT suite_attempt_id, passage_index, asset_id, exam_id, title, category, started_at, ended_at,
                duration_sec, score_info_json
         FROM reading_passages",
        |row| {
            Ok(SrcPassage {
                attempt_id: String::new(),
                suite_attempt_id: row.get(0)?,
                passage_index: row.get(1)?,
                asset_id: row.get(2)?,
                exam_id: row.get(3)?,
                title: row.get(4)?,
                category: row.get(5)?,
                started_at: row.get(6)?,
                ended_at: row.get(7)?,
                duration_sec: row.get(8)?,
                score_info_json: row.get(9)?,
            })
        },
    )?
    .into_iter()
    .filter_map(|mut passage| {
        passage.attempt_id = suite_to_attempt.get(&passage.suite_attempt_id)?.clone();
        Some(passage)
    })
    .collect();

    let answers: Vec<SrcAnswer> = query_rows(
        &conn,
        "SELECT suite_attempt_id, passage_index, question_id, display_label, question_kind, control, match_mode,
                user_answer_json, correct_answer_json, normalized_user_answer_json, normalized_correct_answer_json,
                is_correct, weight, first_answered_at, last_answered_at, change_count, visit_count, elapsed_ms, duration_ms
         FROM reading_answers",
        |row| {
            Ok(SrcAnswer {
                attempt_id: row.get::<_, String>(0)?,
                passage_index: row.get(1)?,
                question_id: row.get(2)?,
                display_label: row.get(3)?,
                question_kind: row.get(4)?,
                control: row.get(5)?,
                match_mode: row.get(6)?,
                user_answer_json: row.get(7)?,
                correct_answer_json: row.get(8)?,
                normalized_user_answer_json: row.get(9)?,
                normalized_correct_answer_json: row.get(10)?,
                is_correct: row.get(11)?,
                weight: row.get::<_, Option<f64>>(12)?.unwrap_or(0.0),
                first_answered_at: row.get(13)?,
                last_answered_at: row.get(14)?,
                change_count: row.get::<_, Option<i64>>(15)?.unwrap_or(0),
                visit_count: row.get::<_, Option<i64>>(16)?.unwrap_or(0),
                elapsed_ms: row.get::<_, Option<i64>>(17)?.unwrap_or(0),
                duration_ms: row.get::<_, Option<i64>>(18)?.unwrap_or(0),
            })
        },
    )?
    .into_iter()
    .filter_map(|mut answer| {
        // 答案行里的 attempt_id 字段暂存的是 suite_attempt_id，这里换成真正的 attempt_id。
        answer.attempt_id = suite_to_attempt.get(&answer.attempt_id)?.clone();
        Some(answer)
    })
    .collect();

    let writings: Vec<SrcWriting> = query_rows(
        &conn,
        "SELECT attempt_id, writing_submission_id, task_type, word_count, started_at, submitted_at, duration_sec
         FROM writing_submissions",
        |row| {
            Ok(SrcWriting {
                attempt_id: row.get(0)?,
                writing_submission_id: row.get(1)?,
                task_type: row.get(2)?,
                word_count: row.get(3)?,
                started_at: row.get(4)?,
                submitted_at: row.get(5)?,
                duration_sec: row.get(6)?,
            })
        },
    )?
    .into_iter()
    .filter(|writing| submitted.contains(&writing.attempt_id))
    .collect();

    let mut scorer_versions = HashMap::new();
    for attempt in &attempts {
        let raw: Option<String> = conn
            .query_row(
                "SELECT value_json FROM exam_kv WHERE key = ?1",
                params![format!("scorer_version:{}", attempt.attempt_id)],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some(raw) = raw {
            let text = serde_json::from_str::<Value>(&raw)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or(raw);
            scorer_versions.insert(attempt.attempt_id.clone(), text);
        }
    }
    // 套题归属（契约 §1.1）：每个 attempt 的 paperId 按来源优先级取值，不一致记入 paper_conflicts。
    let mut paper_ids = HashMap::new();
    let mut paper_conflicts = Vec::new();
    for attempt in &attempts {
        let kv: Option<String> = conn
            .query_row(
                "SELECT value_json FROM exam_kv WHERE key = ?1",
                params![format!("paper_id:{}", attempt.attempt_id)],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let kv_paper = kv
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|value| value.as_str().map(str::trim).filter(|text| !text.is_empty()).map(str::to_string));
        let mut candidates: Vec<String> = kv_paper.into_iter().collect();
        for suite in suites.iter().filter(|suite| suite.attempt_id == attempt.attempt_id) {
            let aggregate: Option<String> = conn
                .query_row(
                    "SELECT aggregate_json FROM reading_suite_attempts WHERE suite_attempt_id = ?1",
                    params![suite.suite_attempt_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .flatten();
            if let Some(raw) = aggregate {
                candidates.extend(paper_ids_from_aggregate(&raw));
            }
        }
        if let Some(first) = candidates.first().cloned() {
            if let Some(other) = candidates.iter().find(|candidate| **candidate != first) {
                paper_conflicts.push((
                    attempt.attempt_id.clone(),
                    format!("套题归属来源不一致：{first} 与 {other}，已采用优先级最高的 {first}"),
                ));
            }
            paper_ids.insert(attempt.attempt_id.clone(), first);
        }
    }

    let extensions = super::import_extensions::Extensions::read(&conn, version)?;
    Ok(SourceDb { extensions, attempts, suites, passages, answers, writings, scorer_versions, paper_ids, paper_conflicts })
}

/// 从 reading_suite_attempts.aggregate_json 取 paperId（契约 §1.1 优先级 2、3）。
/// 支持三种形态：SuiteAggregateV1 / 旧 `{"passageCount":N}`（无值）/ 锁定序列数组（元素带 paperId）。
fn paper_ids_from_aggregate(raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut push = |item: Option<&Value>| {
        if let Some(text) = item.and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()) {
            out.push(text.to_string());
        }
    };
    match &value {
        Value::Object(map) => {
            push(map.get("paperId"));
            if let Some(Value::Array(sequence)) = map.get("lockedSequence") {
                for entry in sequence {
                    push(entry.get("paperId"));
                }
            }
        }
        Value::Array(sequence) => {
            for entry in sequence {
                push(entry.get("paperId"));
            }
        }
        _ => {}
    }
    out
}

fn query_rows<T, F>(conn: &Connection, sql: &str, mut map: F) -> Result<Vec<T>, String>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = conn.prepare(sql).map_err(|error| format!("查询失败：{error}"))?;
    let rows = statement
        .query_map([], |row| map(row))
        .map_err(|error| format!("查询失败：{error}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|error| format!("读取行失败：{error}"))?);
    }
    Ok(out)
}

/// exportedAt 严格更新才覆盖；无法解析时退回字符串比较。
fn is_newer(incoming: &str, existing: &str) -> bool {
    match (
        chrono::DateTime::parse_from_rfc3339(incoming),
        chrono::DateTime::parse_from_rfc3339(existing),
    ) {
        (Ok(a), Ok(b)) => a > b,
        _ => incoming > existing,
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|byte| format!("{:02x}", byte)).collect()
}

/// 导入记录（异常队列用）。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportRecord {
    pub file: String,
    pub status: String,
    pub terminal_id: Option<String>,
    pub attempt_id: Option<String>,
    pub batch_id: Option<String>,
    pub paper_id: Option<String>,
    pub exported_at: Option<String>,
    pub imported_at: String,
    pub error: Option<String>,
}

pub fn list_failed_imports(conn: &Connection) -> CommandResult<Vec<ImportRecord>> {
    list_failed_imports_where(conn, None)
}

/// 只列出某一批次/试卷相关的异常（导出「导入异常」页用）。
/// 试卷列为空（快照缺 paperId、旧套题）的异常仍归入该批次。
pub fn list_failed_imports_for_group(
    conn: &Connection,
    batch_id: &str,
    paper_id: &str,
) -> CommandResult<Vec<ImportRecord>> {
    list_failed_imports_where(conn, Some((batch_id, paper_id)))
}

fn list_failed_imports_where(conn: &Connection, group: Option<(&str, &str)>) -> CommandResult<Vec<ImportRecord>> {
    let (filter, args): (&str, Vec<String>) = match group {
        None => ("", Vec::new()),
        Some((batch, paper)) => (
            " AND batch_id = ?1 AND COALESCE(paper_id, '') IN (?2, '')",
            vec![batch.to_string(), paper.to_string()],
        ),
    };
    let sql = format!(
        "SELECT file, status, terminal_id, attempt_id, batch_id, paper_id, exported_at, imported_at, error
         FROM import_snapshots WHERE status = 'failed'{filter} ORDER BY imported_at DESC, file"
    );
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
            Ok(ImportRecord {
                file: row.get(0)?,
                status: row.get(1)?,
                terminal_id: row.get(2)?,
                attempt_id: row.get(3)?,
                batch_id: row.get(4)?,
                paper_id: row.get(5)?,
                exported_at: row.get(6)?,
                imported_at: row.get(7)?,
                error: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|error| error.to_string())?);
    }
    Ok(out)
}

pub fn last_import_at(conn: &Connection) -> CommandResult<Option<String>> {
    conn.query_row(
        "SELECT MAX(imported_at) FROM import_snapshots WHERE status = 'ok'",
        [],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}
