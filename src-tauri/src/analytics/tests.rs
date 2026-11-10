//! 分析模块测试：用夹具生成学生端形态的终端快照（建表 SQL 原样抄自
//! IELTS-NASfor-WenDao/server/src/lib/exam/MachineSubmissionStore.ts 的 ensureSchema），
//! 走完整的 导入 → 查询 → 导出 路径。不依赖学生仓库代码。

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde_json::json;

use super::import::{
    import_submission_snapshots, list_failed_imports, list_failed_imports_for_group, ImportReport,
};
use super::queries::{
    build_exam_view, build_overview, build_question_view, build_student_view, identity_issues, load_attempts,
    Scope,
};
use super::status::read_attempt_status;
use super::{ensure_schema, export::write_export, open_analytics};

/// 学生端 ensureSchema() 的建表语句（逐字抄录，V1 表结构）。
const STUDENT_SCHEMA_SQL: &str = r#"
      CREATE TABLE IF NOT EXISTS exam_attempts (
        attempt_id TEXT PRIMARY KEY,
        batch_id TEXT NOT NULL,
        terminal_id TEXT NOT NULL,
        candidate_id TEXT NOT NULL,
        candidate_name TEXT NOT NULL,
        checkin_at TEXT NOT NULL,
        started_at TEXT,
        submitted_at TEXT,
        status TEXT NOT NULL CHECK(status IN ('checked_in', 'in_progress', 'submitted', 'failed')),
        app_version TEXT,
        runtime_json TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
      );

      CREATE TABLE IF NOT EXISTS reading_suite_attempts (
        suite_attempt_id TEXT PRIMARY KEY,
        attempt_id TEXT NOT NULL,
        status TEXT NOT NULL CHECK(status IN ('active', 'completed', 'submitted')),
        started_at TEXT,
        completed_at TEXT,
        submitted_at TEXT,
        total_passages INTEGER NOT NULL DEFAULT 3,
        aggregate_json TEXT,
        FOREIGN KEY(attempt_id) REFERENCES exam_attempts(attempt_id) ON DELETE CASCADE
      );

      CREATE TABLE IF NOT EXISTS reading_passages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        suite_attempt_id TEXT NOT NULL,
        passage_index INTEGER NOT NULL,
        asset_id TEXT NOT NULL,
        exam_id TEXT NOT NULL,
        title TEXT,
        category TEXT,
        source_file TEXT,
        source_hash TEXT,
        started_at TEXT,
        ended_at TEXT,
        duration_sec INTEGER,
        score_info_json TEXT NOT NULL,
        raw_submission_json TEXT NOT NULL,
        UNIQUE(suite_attempt_id, passage_index),
        FOREIGN KEY(suite_attempt_id) REFERENCES reading_suite_attempts(suite_attempt_id) ON DELETE CASCADE
      );

      CREATE TABLE IF NOT EXISTS reading_answers (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        suite_attempt_id TEXT NOT NULL,
        passage_index INTEGER NOT NULL,
        question_id TEXT NOT NULL,
        display_label TEXT,
        question_kind TEXT,
        control TEXT,
        source TEXT,
        match_mode TEXT,
        user_answer_json TEXT NOT NULL,
        correct_answer_json TEXT NOT NULL,
        normalized_user_answer_json TEXT,
        normalized_correct_answer_json TEXT,
        is_correct INTEGER,
        weight REAL NOT NULL DEFAULT 1,
        first_answered_at TEXT,
        last_answered_at TEXT,
        change_count INTEGER NOT NULL DEFAULT 0,
        visit_count INTEGER NOT NULL DEFAULT 0,
        elapsed_ms INTEGER NOT NULL DEFAULT 0,
        duration_ms INTEGER NOT NULL DEFAULT 0,
        UNIQUE(suite_attempt_id, passage_index, question_id)
      );

      CREATE TABLE IF NOT EXISTS writing_submissions (
        writing_submission_id TEXT PRIMARY KEY,
        attempt_id TEXT NOT NULL,
        task_type TEXT NOT NULL CHECK(task_type IN ('task1', 'task2')),
        prompt_text TEXT,
        content TEXT NOT NULL,
        word_count INTEGER NOT NULL,
        started_at TEXT,
        submitted_at TEXT NOT NULL,
        duration_sec INTEGER,
        metadata_json TEXT NOT NULL,
        FOREIGN KEY(attempt_id) REFERENCES exam_attempts(attempt_id) ON DELETE CASCADE
      );

      CREATE TABLE IF NOT EXISTS submission_receipts (
        receipt_id TEXT PRIMARY KEY,
        attempt_id TEXT NOT NULL,
        sink TEXT NOT NULL,
        database_file_name TEXT NOT NULL,
        relative_path TEXT NOT NULL,
        written_at TEXT NOT NULL,
        payload_hash TEXT,
        FOREIGN KEY(attempt_id) REFERENCES exam_attempts(attempt_id) ON DELETE CASCADE
      );

      CREATE TABLE IF NOT EXISTS exam_kv (
        key TEXT PRIMARY KEY,
        value_json TEXT NOT NULL,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
      );
"#;

const BATCH: &str = "B-2026-11";
const PAPER: &str = "paper-A";

/// 一篇：pattern 每个字符一题，T=答对、F=答错；category 决定 Part 与题型。
struct FixPassage {
    idx: i64,
    category: &'static str,
    start: &'static str,
    end: &'static str,
    pattern: &'static str,
}

/// 学生端 reading_suite_attempts.aggregate_json 的三种形态（契约 §1.1）。
#[derive(Clone, Copy)]
enum AggForm {
    /// 交卷后的 SuiteAggregateV1：对象带 paperId，并保留锁定序列（数组，元素带 paperId）。
    V11,
    /// 旧形态：交卷时被覆盖成 {"passageCount":N}，不含 paperId。
    Legacy,
    /// 锁定序列数组本体（元素带 paperId）。
    ArrayLocked,
}

struct FixAttempt {
    id: &'static str,
    terminal: &'static str,
    cand_id: &'static str,
    cand_name: &'static str,
    submitted: &'static str,
    passages: Vec<FixPassage>,
    writing: bool,
    agg: AggForm,
    /// 是否额外写 exam_kv 的 paper_id:<attemptId>（学生端锁卷时写入）。
    kv_paper: bool,
}

fn kind_of(category: &str) -> &'static str {
    match category {
        "P1" => "true_false_not_given",
        "P2" => "matching_headings",
        _ => "sentence_completion",
    }
}

fn standard_passages(p1: &'static str, p2: &'static str, p3: &'static str) -> Vec<FixPassage> {
    vec![
        FixPassage { idx: 0, category: "P1", start: "2026-11-07T10:00:00+08:00", end: "2026-11-07T10:20:00+08:00", pattern: p1 },
        // P2 与 P1 区间重叠（用于验证「用时可能重叠」）。
        FixPassage { idx: 1, category: "P2", start: "2026-11-07T10:15:00+08:00", end: "2026-11-07T10:40:00+08:00", pattern: p2 },
        FixPassage { idx: 2, category: "P3", start: "2026-11-07T10:50:00+08:00", end: "2026-11-07T11:05:00+08:00", pattern: p3 },
    ]
}

fn sequential_passages(p1: &'static str, p2: &'static str, p3: &'static str) -> Vec<FixPassage> {
    vec![
        FixPassage { idx: 0, category: "P1", start: "2026-11-07T10:00:00+08:00", end: "2026-11-07T10:20:00+08:00", pattern: p1 },
        FixPassage { idx: 1, category: "P2", start: "2026-11-07T10:30:00+08:00", end: "2026-11-07T10:50:00+08:00", pattern: p2 },
        FixPassage { idx: 2, category: "P3", start: "2026-11-07T11:00:00+08:00", end: "2026-11-07T11:20:00+08:00", pattern: p3 },
    ]
}

/// 锁定序列（学生端 ExamSuitePassage 形态，元素带 paperId）。
fn locked_sequence(attempt: &FixAttempt) -> Vec<serde_json::Value> {
    attempt
        .passages
        .iter()
        .map(|passage| {
            json!({
                "index": passage.idx,
                "assetId": format!("asset-{}-{}", attempt.id, passage.idx),
                "examId": format!("exam-{}", passage.idx),
                "title": format!("Passage {}", passage.idx + 1),
                "category": passage.category,
                "paperId": PAPER
            })
        })
        .collect()
}

fn aggregate_json(attempt: &FixAttempt) -> serde_json::Value {
    match attempt.agg {
        AggForm::V11 => json!({
            "schemaVersion": "SuiteAggregateV1",
            "passageCount": attempt.passages.len(),
            "paperId": PAPER,
            "lockedSequence": locked_sequence(attempt)
        }),
        AggForm::Legacy => json!({ "passageCount": attempt.passages.len() }),
        AggForm::ArrayLocked => json!(locked_sequence(attempt)),
    }
}

/// 写一个学生端形态的终端库。
fn write_terminal_db(path: &Path, attempts: &[FixAttempt]) {
    let _ = fs::remove_file(path);
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(STUDENT_SCHEMA_SQL).unwrap();
    for attempt in attempts {
        conn.execute(
            "INSERT INTO exam_attempts (attempt_id, batch_id, terminal_id, candidate_id, candidate_name, checkin_at,
                started_at, submitted_at, status, app_version, runtime_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, 'submitted', '1.0.0', '{}')",
            params![attempt.id, BATCH, attempt.terminal, attempt.cand_id, attempt.cand_name, "2026-11-07T09:50:00+08:00", attempt.submitted],
        )
        .unwrap();
        let suite_id = format!("suite-{}", attempt.id);
        conn.execute(
            "INSERT INTO reading_suite_attempts (suite_attempt_id, attempt_id, status, started_at, completed_at, submitted_at, total_passages, aggregate_json)
             VALUES (?1, ?2, 'submitted', ?3, ?4, ?4, 3, ?5)",
            params![
                suite_id,
                attempt.id,
                "2026-11-07T10:00:00+08:00",
                attempt.submitted,
                aggregate_json(attempt).to_string()
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO exam_kv (key, value_json) VALUES (?1, '\"reading-scorer/1\"')",
            params![format!("scorer_version:{}", attempt.id)],
        )
        .unwrap();
        if attempt.kv_paper {
            conn.execute(
                "INSERT INTO exam_kv (key, value_json) VALUES (?1, ?2)",
                params![format!("paper_id:{}", attempt.id), json!(PAPER).to_string()],
            )
            .unwrap();
        }
        for passage in &attempt.passages {
            let total = passage.pattern.len() as f64;
            let correct = passage.pattern.chars().filter(|c| *c == 'T').count() as f64;
            let score = json!({
                "gradable": true,
                "correct": correct,
                "total": total,
                "totalQuestions": total,
                "accuracy": correct / total,
                "percentage": (correct / total * 100.0).round(),
                "duration": 1200,
                "source": "practice_reading_session"
            });
            conn.execute(
                "INSERT INTO reading_passages (suite_attempt_id, passage_index, asset_id, exam_id, title, category,
                    started_at, ended_at, duration_sec, score_info_json, raw_submission_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1200, ?9, '{}')",
                params![
                    suite_id,
                    passage.idx,
                    format!("asset-{}-{}", attempt.id, passage.idx),
                    format!("exam-{}", passage.idx),
                    format!("Passage {}", passage.idx + 1),
                    passage.category,
                    passage.start,
                    passage.end,
                    score.to_string()
                ],
            )
            .unwrap();
            for (n, ch) in passage.pattern.chars().enumerate() {
                let is_correct = ch == 'T';
                let question_id = format!("p{}q{}", passage.idx, n + 1);
                let user = if is_correct { "TRUE" } else { "FALSE" };
                conn.execute(
                    "INSERT INTO reading_answers (suite_attempt_id, passage_index, question_id, display_label, question_kind,
                        control, source, match_mode, user_answer_json, correct_answer_json, normalized_user_answer_json,
                        normalized_correct_answer_json, is_correct, weight, change_count, visit_count, elapsed_ms, duration_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'radio', 'answerKey', 'exact', ?6, ?7, ?8, ?9, ?10, 1, ?11, ?12, ?13, ?14)",
                    params![
                        suite_id,
                        passage.idx,
                        question_id,
                        format!("Q{}", passage.idx * 4 + n as i64 + 1),
                        kind_of(passage.category),
                        json!(user).to_string(),
                        json!("TRUE").to_string(),
                        json!([user]).to_string(),
                        json!(["TRUE"]).to_string(),
                        if is_correct { 1 } else { 0 },
                        (n % 3) as i64,
                        ((n as i64) + 2) % 4,
                        (n as i64 + 1) * 1000,
                        ((n as i64) + 2) * 100
                    ],
                )
                .unwrap();
            }
        }
        if attempt.writing {
            conn.execute(
                "INSERT INTO writing_submissions (writing_submission_id, attempt_id, task_type, prompt_text, content,
                    word_count, started_at, submitted_at, duration_sec, metadata_json)
                 VALUES (?1, ?2, 'task2', 'prompt', 'essay', 260, ?3, ?4, 2400, '{}')",
                params![format!("w-{}", attempt.id), attempt.id, "2026-11-07T10:00:00+08:00", attempt.submitted],
            )
            .unwrap();
        }
    }
}

fn sha(bytes: &[u8]) -> String {
    super::import::sha256_hex(bytes)
}

/// 写一份快照（sqlite + meta + 可选 ready）。tamper_sha=true 时 meta 的 sha 故意写错。
fn publish_snapshot(
    submissions: &Path,
    terminal: &str,
    attempt: &str,
    db_bytes: &[u8],
    exported_at: &str,
    tamper_sha: bool,
    with_ready: bool,
) {
    let dir = submissions.join("snapshots").join(terminal);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{attempt}.sqlite")), db_bytes).unwrap();
    let digest = if tamper_sha { "0".repeat(64) } else { sha(db_bytes) };
    let meta = json!({
        "schemaVersion": "SubmissionSnapshotV1",
        "terminalId": terminal,
        "installId": null,
        "attemptId": attempt,
        "batchId": BATCH,
        "paperId": PAPER,
        "candidateId": "C-x",
        "candidateName": "x",
        "submittedAt": exported_at,
        "exportedAt": exported_at,
        "appVersion": "1.0.0",
        "dbSchemaVersion": 1,
        "scorerVersion": "reading-scorer/1",
        "sizeBytes": db_bytes.len(),
        "sha256": digest
    });
    fs::write(dir.join(format!("{attempt}.meta.json")), meta.to_string()).unwrap();
    if with_ready {
        fs::write(dir.join(format!("{attempt}.ready")), "").unwrap();
    } else {
        let _ = fs::remove_file(dir.join(format!("{attempt}.ready")));
    }
}

/// 与 publish_snapshot 相同，但 meta 的字段可用 extra 覆盖（异常路径：版本过新、安装不一致、批次不同等）。
fn publish_snapshot_with_meta(
    submissions: &Path,
    terminal: &str,
    attempt: &str,
    db_bytes: &[u8],
    exported_at: &str,
    extra: serde_json::Value,
) {
    let dir = submissions.join("snapshots").join(terminal);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{attempt}.sqlite")), db_bytes).unwrap();
    let mut meta = json!({
        "schemaVersion": "SubmissionSnapshotV1",
        "terminalId": terminal,
        "installId": null,
        "attemptId": attempt,
        "batchId": BATCH,
        "paperId": PAPER,
        "candidateId": "C-x",
        "candidateName": "x",
        "submittedAt": exported_at,
        "exportedAt": exported_at,
        "appVersion": "1.0.0",
        "dbSchemaVersion": 1,
        "scorerVersion": "reading-scorer/1",
        "sizeBytes": db_bytes.len(),
        "sha256": sha(db_bytes)
    });
    if let (Some(base), Some(over)) = (meta.as_object_mut(), extra.as_object()) {
        for (key, value) in over {
            base.insert(key.clone(), value.clone());
        }
    }
    fs::write(dir.join(format!("{attempt}.meta.json")), meta.to_string()).unwrap();
    fs::write(dir.join(format!("{attempt}.ready")), "").unwrap();
}

fn write_status(submissions: &Path, terminal: &str, attempt: &str, status: &str, cand: (&str, &str), updated: &str) {
    let dir = submissions.join("status").join(terminal);
    fs::create_dir_all(&dir).unwrap();
    let body = json!({
        "schemaVersion": "AttemptStatusV1",
        "attemptId": attempt,
        "terminalId": terminal,
        "batchId": BATCH,
        "paperId": PAPER,
        "candidateId": cand.0,
        "candidateName": cand.1,
        "status": status,
        "updatedAt": updated,
        "appVersion": "1.0.0"
    });
    fs::write(dir.join(format!("{attempt}.json")), body.to_string()).unwrap();
}

struct Env {
    submissions: PathBuf,
    app: PathBuf,
}

fn fresh_env(name: &str) -> Env {
    let base = std::env::temp_dir().join(format!("analytics-test-{name}-{}", uuid::Uuid::new_v4()));
    let submissions = base.join("submissions");
    let app = base.join("app");
    fs::create_dir_all(&submissions).unwrap();
    fs::create_dir_all(&app).unwrap();
    Env { submissions, app }
}

fn t1_attempts(a1_p3: &'static str) -> Vec<FixAttempt> {
    vec![
        FixAttempt {
            id: "A1",
            agg: AggForm::V11,
            kv_paper: false,
            terminal: "T1",
            cand_id: "C-001",
            cand_name: "张三",
            submitted: "2026-11-07T11:10:00+08:00",
            // P1 3/4，P2 4/4，P3 由参数决定（seed 用 FFTF = 1/4）。P1/P2 区间重叠。
            passages: standard_passages("TTFT", "TTTT", a1_p3),
            writing: true,
        },
        FixAttempt {
            id: "A2",
            agg: AggForm::Legacy,
            kv_paper: true,
            terminal: "T1",
            cand_id: "C-002",
            cand_name: "李四",
            submitted: "2026-11-07T11:20:00+08:00",
            passages: sequential_passages("TFFF", "TTFF", "TTTT"),
            writing: false,
        },
    ]
}

fn t2_attempts() -> Vec<FixAttempt> {
    vec![
        FixAttempt {
            id: "A3",
            agg: AggForm::ArrayLocked,
            kv_paper: false,
            terminal: "T2",
            cand_id: "C-002",
            cand_name: "李四四",
            submitted: "2026-11-07T11:30:00+08:00",
            passages: standard_passages("TTTT", "TTTT", "TTTT"),
            writing: false,
        },
        FixAttempt {
            id: "A4",
            agg: AggForm::Legacy,
            kv_paper: true,
            terminal: "T2",
            cand_id: "C-004",
            cand_name: "张三",
            submitted: "2026-11-07T11:40:00+08:00",
            passages: standard_passages("FFFF", "TFFF", "FFFF"),
            writing: false,
        },
    ]
}

fn db_bytes(name: &str, attempts: &[FixAttempt]) -> Vec<u8> {
    let path = std::env::temp_dir().join(format!("analytics-fixture-{name}-{}.sqlite", uuid::Uuid::new_v4()));
    write_terminal_db(&path, attempts);
    let bytes = fs::read(&path).unwrap();
    let _ = fs::remove_file(&path);
    bytes
}

/// 标准场景：T1(A1,A2)、T2(A3,A4) 正常；T3 含 sha 不符、坏库、无 ready 三类异常；另有坏状态信标。
fn seed_standard(env: &Env) {
    let t1 = db_bytes("t1", &t1_attempts("FFTF"));
    publish_snapshot(&env.submissions, "T1", "A1", &t1, "2026-11-07T11:10:05+08:00", false, true);
    publish_snapshot(&env.submissions, "T1", "A2", &t1, "2026-11-07T11:10:05+08:00", false, true);
    // 注意：t1 由 t1_attempts("FFTF") 生成，A1 的 P3 = 1/4。
    let t2 = db_bytes("t2", &t2_attempts());
    publish_snapshot(&env.submissions, "T2", "A3", &t2, "2026-11-07T11:40:05+08:00", false, true);
    // T3：A5 meta sha 写错。
    let t3 = db_bytes("t3", &t2_attempts());
    publish_snapshot(&env.submissions, "T3", "A5", &t3, "2026-11-07T12:00:00+08:00", true, true);
    // T3：A6 文件不是 sqlite（sha 与 meta 一致，但库坏）。
    let corrupt = b"this is definitely not a sqlite database file".to_vec();
    publish_snapshot(&env.submissions, "T3", "A6", &corrupt, "2026-11-07T12:01:00+08:00", false, true);
    // T3：A7 有 sqlite 但没有 ready。
    publish_snapshot(&env.submissions, "T3", "A7", &t3, "2026-11-07T12:02:00+08:00", false, false);

    write_status(&env.submissions, "T1", "A1", "submitted", ("C-001", "张三"), "2026-11-07T11:10:00+08:00");
    write_status(&env.submissions, "T1", "A2", "submitted", ("C-002", "李四"), "2026-11-07T11:20:00+08:00");
    write_status(&env.submissions, "T1", "A8", "in_progress", ("C-005", "王五"), "2026-11-07T11:50:00+08:00");
    write_status(&env.submissions, "T1", "A9", "checked_in", ("C-006", "赵六"), "2026-11-07T11:55:00+08:00");
    write_status(&env.submissions, "T2", "A3", "submitted", ("C-002", "李四四"), "2026-11-07T11:30:00+08:00");
    write_status(&env.submissions, "T2", "A4", "submitted", ("C-004", "张三"), "2026-11-07T11:40:00+08:00");
    write_status(&env.submissions, "T2", "A10", "failed", ("C-007", "钱七"), "2026-11-07T11:45:00+08:00");
    // 坏信标：不影响其他。
    let bad = env.submissions.join("status").join("T3");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("broken.json"), "{ not json").unwrap();
}

fn open(env: &Env) -> Connection {
    open_analytics(&env.app).unwrap()
}

fn run_import(env: &Env, conn: &mut Connection) -> ImportReport {
    import_submission_snapshots(conn, &env.app, &env.submissions).unwrap()
}

#[test]
fn import_queries_and_exceptions_end_to_end() {
    let env = fresh_env("main");
    seed_standard(&env);
    let mut conn = open(&env);
    let report = run_import(&env, &mut conn);

    // 5 个 ready + 1 个未就绪。A1 成功（连带导入同库的 A2），A2 文件因同库 exportedAt 不新而 skipped，
    // A3 成功（连带 A4），A5/A6 失败，A7 缺 ready 进异常。
    assert_eq!(report.scanned, 6, "{report:?}");
    assert_eq!(report.ok, 2, "{report:?}");
    assert_eq!(report.skipped, 1, "{report:?}");
    assert_eq!(report.failed, 3, "{report:?}");
    assert_eq!(report.attempts_imported, 4, "{report:?}");
    let failed_files: Vec<&str> = report
        .outcomes
        .iter()
        .filter(|o| o.status == "failed")
        .map(|o| o.file.as_str())
        .collect();
    assert!(failed_files.contains(&"snapshots/T3/A5.sqlite"));
    assert!(failed_files.contains(&"snapshots/T3/A6.sqlite"));
    assert!(failed_files.contains(&"snapshots/T3/A7.sqlite"));
    let a5 = report.outcomes.iter().find(|o| o.file.ends_with("A5.sqlite")).unwrap();
    assert!(a5.error.as_deref().unwrap().contains("sha256 不符"), "{a5:?}");

    // 只读源：NAS 上的文件与 ready 没有被改动。
    assert!(env.submissions.join("snapshots/T3/A5.ready").exists());
    assert!(env.submissions.join("snapshots/T3/A7.sqlite").exists());

    // 单次考试（batch B-2026-11 / paper-A）：有效 N = 4。
    let view = build_exam_view(&conn, BATCH, PAPER).unwrap();
    assert_eq!(view.n, 4);
    assert!(view.excluded.is_empty());
    // 原始分：A1 = 3+4+1 = 8，A2 = 1+2+4 = 7，A3 = 12，A4 = 0+1+0 = 1。
    let mut raws: Vec<f64> = view.students.iter().map(|s| s.raw_score).collect();
    raws.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(raws, vec![1.0, 7.0, 8.0, 12.0]);
    assert_eq!(view.score.mean, Some(7.0));
    assert_eq!(view.score.median, Some(7.5));
    assert_eq!(view.score.max, Some(12.0));
    assert_eq!(view.score.min, Some(1.0));
    assert_eq!(view.score.max_total, Some(12.0));
    // 直方图总人数 = N。
    assert_eq!(view.histogram.iter().map(|b| b.count).sum::<usize>(), 4);

    // Part：P1 8/16，P2 11/16，P3 9/16。
    let p = |i: usize| (view.parts[i].correct, view.parts[i].total, view.parts[i].n);
    assert_eq!(view.parts.len(), 3);
    assert_eq!(p(0), (8.0, 16.0, 4));
    assert_eq!(p(1), (11.0, 16.0, 4));
    assert_eq!(p(2), (9.0, 16.0, 4));
    assert_eq!(view.parts[0].label, "P1");
    assert_eq!(view.parts[0].accuracy, Some(0.5));

    // 题型正确率分母 = 得分点数。
    let tfng = view.kinds.iter().find(|k| k.kind == "true_false_not_given").unwrap();
    assert_eq!((tfng.points, tfng.correct_points), (16.0, 8.0));
    assert_eq!(tfng.accuracy, Some(0.5));

    // N < 30：所有题都不下难题结论；Top10 仍列出并带样本量。
    assert!(view.questions.iter().all(|q| q.n == 4 && !q.hard_conclusion));
    assert!(!view.hard_questions.is_empty() && view.hard_questions.len() <= 10);

    // 学生×题目矩阵：列数 = 题数，行数 = N。
    assert_eq!(view.matrix.len(), 4);
    assert!(view.matrix.iter().all(|row| row.cells.len() == view.questions.len()));

    // 单个学生 A1：两篇区间重叠（P1/P2）；错题 = P1 1 题 + P3 3 题。
    let student = build_student_view(&conn, "A1").unwrap();
    assert_eq!(student.raw_score, 8.0);
    assert!(student.passages.iter().filter(|p| p.overlap).count() >= 2 || student.overlap_note.is_some());
    assert!(student.overlap_note.is_some());
    assert!(student.passages.iter().any(|p| p.overlap));
    assert_eq!(student.wrong_answers.len(), 4);
    assert_eq!(student.writings.len(), 1);
    assert_eq!(student.writings[0].word_count, Some(260));
    assert_eq!(student.writings[0].duration_sec, Some(2400));
    let wrong = student.wrong_answers.iter().find(|w| w.question_id == "p0q3").unwrap();
    assert_eq!(wrong.user_answer, "FALSE");
    assert_eq!(wrong.correct_answer, "TRUE");
    assert_eq!(wrong.question_kind, "true_false_not_given");

    // 单题 p0q1：A1/A2/A3 答对（TRUE），A4 答错（FALSE）。N=4，正确率 3/4。
    let question = build_question_view(&conn, BATCH, PAPER, 0, "p0q1").unwrap();
    assert_eq!(question.n, 4);
    assert_eq!(question.correct_count, 3);
    assert_eq!(question.accuracy, Some(0.75));
    assert!(!question.hard_conclusion);
    assert_eq!(question.distribution.iter().map(|b| b.count).sum::<usize>(), 4);
    assert_eq!(question.distribution[0].answer, "TRUE");
    assert_eq!(question.distribution[0].count, 3);

    // 身份核对：C-002 对应「李四」「李四四」；「张三」对应 C-001 与 C-004。
    let attempts = load_attempts(&conn, &Scope::All).unwrap();
    let issues = identity_issues(&attempts);
    assert_eq!(issues.len(), 2, "{issues:?}");
    assert!(issues.iter().any(|i| i.kind == "same_id_diff_name" && i.candidate_id == "C-002"));
    assert!(issues.iter().any(|i| i.kind == "same_name_diff_id" && i.candidate_name == "张三"));

    // 状态信标：坏信标记入 errors，其余照常汇总。
    let scan = read_attempt_status(&env.submissions);
    assert_eq!(scan.beacons.len(), 7);
    assert_eq!(scan.errors.len(), 1);

    // 批次总览：信标 + 分析库 + 异常。
    let overview = build_overview(&conn, &scan).unwrap();
    let row = overview
        .batches
        .iter()
        .find(|b| b.batch_id == BATCH && b.paper_id == PAPER)
        .expect("group row");
    assert_eq!(row.checked_in, 1);
    assert_eq!(row.in_progress, 1);
    assert_eq!(row.submitted_beacon, 4);
    assert_eq!(row.voided, 1);
    assert_eq!(row.imported, 4);
    assert_eq!(row.valid_scored, 4);
    // A5(sha 不符)、A6(坏库)、A7(缺 ready) 的 meta 都带本批次，全部归入该组异常。
    assert_eq!(row.import_exceptions, 3);
    assert_eq!(row.last_submitted_at.as_deref(), Some("2026-11-07T11:40:00+08:00"));
    assert_eq!(overview.exceptions.len(), 3);
}

#[test]
fn reimport_is_idempotent() {
    let env = fresh_env("idem");
    seed_standard(&env);
    let mut conn = open(&env);
    run_import(&env, &mut conn);
    let before = build_exam_view(&conn, BATCH, PAPER).unwrap();

    let again = run_import(&env, &mut conn);
    assert_eq!(again.ok, 0, "{again:?}");
    assert_eq!(again.attempts_imported, 0);
    // A1/A3 已成功导入过 -> skipped；A2 同库且 exportedAt 不新 -> skipped。
    assert_eq!(again.skipped, 3, "{again:?}");
    assert_eq!(again.failed, 3, "{again:?}");
    let after = build_exam_view(&conn, BATCH, PAPER).unwrap();
    assert_eq!(before, after);
    // 异常队列不因重复导入而膨胀。
    let overview = build_overview(&conn, &read_attempt_status(&env.submissions)).unwrap();
    assert_eq!(overview.exceptions.len(), 3);
}

#[test]
fn newer_exported_at_replaces_and_older_is_ignored() {
    let env = fresh_env("newer");
    seed_standard(&env);
    let mut conn = open(&env);
    run_import(&env, &mut conn);
    assert_eq!(build_student_view(&conn, "A1").unwrap().raw_score, 8.0);

    // 更新：A1 的 P3 全对，exportedAt 更晚 -> 覆盖。
    let newer = db_bytes("t1-newer", &t1_attempts("TTTT"));
    publish_snapshot(&env.submissions, "T1", "A1", &newer, "2026-11-07T13:00:00+08:00", false, true);
    let report = run_import(&env, &mut conn);
    assert!(report.attempts_imported >= 1, "{report:?}");
    assert_eq!(build_student_view(&conn, "A1").unwrap().raw_score, 11.0);

    // 旧快照（exportedAt 更早，内容为全错）再次出现 -> 不覆盖。
    let older = db_bytes("t1-older", &t1_attempts("FFFF"));
    publish_snapshot(&env.submissions, "T1", "A1", &older, "2026-11-07T10:30:00+08:00", false, true);
    run_import(&env, &mut conn);
    assert_eq!(build_student_view(&conn, "A1").unwrap().raw_score, 11.0);
}

#[test]
fn exam_without_snapshots_is_empty_and_schema_is_stable() {
    let env = fresh_env("empty");
    let mut conn = open(&env);
    ensure_schema(&conn).unwrap();
    let report = run_import(&env, &mut conn);
    assert_eq!(report.scanned, 0);
    assert!(!report.snapshots_dir_found);
    let view = build_exam_view(&conn, BATCH, PAPER).unwrap();
    assert_eq!(view.n, 0);
    assert_eq!(view.score.mean, None);
    assert!(view.students.is_empty());
}

#[test]
fn export_writes_eight_sheets_to_chosen_path() {
    let env = fresh_env("export");
    seed_standard(&env);
    let mut conn = open(&env);
    run_import(&env, &mut conn);
    let out = env.app.join("exam-export.xlsx");
    let summary = write_export(&conn, BATCH, PAPER, &out).unwrap();
    assert_eq!(summary.n, 4);
    assert_eq!(summary.sheets.len(), 8);
    assert!(summary.sheets.iter().any(|s|s=="改判后成绩"));
    assert!(summary.sheets.iter().any(|s|s=="写作成绩"));
    assert!(summary.sheets.iter().any(|s|s=="听力"));
    let bytes = fs::read(&out).unwrap();
    assert!(bytes.len() > 2000);
    assert_eq!(&bytes[0..2], b"PK", "xlsx is a zip container");
}

#[test]
fn valid_only_when_all_passages_are_present() {
    let env = fresh_env("partial");
    // 只有一篇的 attempt：套题 total=3，篇数不足，不计入。
    let partial = vec![FixAttempt {
        id: "A20",
        agg: AggForm::Legacy,
        kv_paper: false,
        terminal: "T9",
        cand_id: "C-900",
        cand_name: "半套",
        submitted: "2026-11-07T11:00:00+08:00",
        passages: vec![FixPassage {
            idx: 0,
            category: "P1",
            start: "2026-11-07T10:00:00+08:00",
            end: "2026-11-07T10:20:00+08:00",
            pattern: "TTTT",
        }],
        writing: false,
    }];
    let bytes = db_bytes("partial", &partial);
    publish_snapshot(&env.submissions, "T9", "A20", &bytes, "2026-11-07T11:00:05+08:00", false, true);
    let mut conn = open(&env);
    let report = run_import(&env, &mut conn);
    assert_eq!(report.ok, 1, "{report:?}");
    let view = build_exam_view(&conn, BATCH, PAPER).unwrap();
    assert_eq!(view.n, 0);
    assert_eq!(view.excluded.len(), 1);
    assert!(view.excluded[0].reason.contains("篇数不足"), "{:?}", view.excluded);
}

/// 回归（审计 A1）：同一终端 3 人依次考试，每人交卷后导出一份整库快照。
/// 交卷后 aggregate 是旧形态 {"passageCount":3}，没有 exam_kv paper_id，套题归属只能来自快照 meta.paperId；
/// 后面快照里的前序 attempt 不得被空 paperId 覆盖。
#[test]
fn same_terminal_sequential_exams_keep_paper_ids() {
    let env = fresh_env("sequential");
    let mut conn = open(&env);
    let people = [
        ("A31", "C-301", "王一", "2026-11-07T10:10:00+08:00", "2026-11-07T10:10:05+08:00"),
        ("A32", "C-302", "王二", "2026-11-07T10:30:00+08:00", "2026-11-07T10:30:05+08:00"),
        ("A33", "C-303", "王三", "2026-11-07T10:50:00+08:00", "2026-11-07T10:50:05+08:00"),
    ];
    let mut done: Vec<FixAttempt> = Vec::new();
    for (id, cand, name, submitted, exported) in people {
        done.push(FixAttempt {
            id,
            terminal: "T5",
            cand_id: cand,
            cand_name: name,
            submitted,
            passages: sequential_passages("TTFT", "TTTF", "FTTT"),
            writing: false,
            agg: AggForm::Legacy,
            kv_paper: false,
        });
        let bytes = db_bytes(&format!("t5-{id}"), &done);
        publish_snapshot(&env.submissions, "T5", id, &bytes, exported, false, true);
        run_import(&env, &mut conn);
    }
    let rows: Vec<(String, String)> = conn
        .prepare("SELECT attempt_id, paper_id FROM attempts ORDER BY attempt_id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        vec![
            ("A31".to_string(), PAPER.to_string()),
            ("A32".to_string(), PAPER.to_string()),
            ("A33".to_string(), PAPER.to_string()),
        ]
    );
    let view = build_exam_view(&conn, BATCH, PAPER).unwrap();
    assert_eq!(view.n, 3);
    assert!(list_failed_imports(&conn).unwrap().is_empty());
}

/// 套题归属来源冲突（审计 A1）：exam_kv 与 aggregate 不一致 → 按优先级采用 exam_kv，并记入异常。
#[test]
fn paper_id_conflict_is_anomaly_and_priority_source_wins() {
    let env = fresh_env("paper-conflict");
    let mut conn = open(&env);
    let attempts = vec![FixAttempt {
        id: "A41",
        terminal: "T6",
        cand_id: "C-41",
        cand_name: "冲突",
        submitted: "2026-11-07T11:00:00+08:00",
        passages: standard_passages("TTTT", "TTTT", "TTTT"),
        writing: false,
        agg: AggForm::V11,
        kv_paper: false,
    }];
    let path = std::env::temp_dir().join(format!("analytics-conflict-{}.sqlite", uuid::Uuid::new_v4()));
    write_terminal_db(&path, &attempts);
    Connection::open(&path)
        .unwrap()
        .execute("INSERT INTO exam_kv (key, value_json) VALUES ('paper_id:A41', '\"paper-OTHER\"')", [])
        .unwrap();
    let bytes = fs::read(&path).unwrap();
    let _ = fs::remove_file(&path);
    publish_snapshot(&env.submissions, "T6", "A41", &bytes, "2026-11-07T11:00:05+08:00", false, true);

    let report = run_import(&env, &mut conn);
    assert_eq!(report.ok, 1, "{report:?}");
    let paper: String = conn
        .query_row("SELECT paper_id FROM attempts WHERE attempt_id = 'A41'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(paper, "paper-OTHER");
    let failed = list_failed_imports(&conn).unwrap();
    assert!(
        failed.iter().any(|r| r.error.as_deref().unwrap_or("").contains("套题归属来源不一致")),
        "{failed:?}"
    );
}

/// 快照版本过新（dbSchemaVersion > 作者端支持版本）→ 不导入，进异常队列。
#[test]
fn newer_db_schema_version_is_not_imported() {
    let env = fresh_env("schema");
    let mut conn = open(&env);
    let bytes = db_bytes("schema", &t2_attempts());
    publish_snapshot_with_meta(
        &env.submissions,
        "T2",
        "A3",
        &bytes,
        "2026-11-07T11:40:05+08:00",
        json!({ "dbSchemaVersion": 3 }),
    );
    let report = run_import(&env, &mut conn);
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(report.attempts_imported, 0);
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM attempts", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 0);
    let failed = list_failed_imports(&conn).unwrap();
    assert!(failed.iter().any(|r| r.error.as_deref().unwrap_or("").contains("请升级作者端")), "{failed:?}");
}

/// 同一 attemptId 来自另一台安装（installId 不同）→ 异常，已导入记录不被覆盖。
#[test]
fn same_attempt_from_other_install_is_not_overwritten() {
    let env = fresh_env("install");
    let mut conn = open(&env);
    let first = db_bytes("inst-1", &t1_attempts("FFTF"));
    publish_snapshot_with_meta(
        &env.submissions,
        "T1",
        "A1",
        &first,
        "2026-11-07T11:10:05+08:00",
        json!({ "installId": "install-X" }),
    );
    run_import(&env, &mut conn);
    assert_eq!(build_student_view(&conn, "A1").unwrap().raw_score, 8.0);

    let newer = db_bytes("inst-2", &t1_attempts("TTTT"));
    publish_snapshot_with_meta(
        &env.submissions,
        "T1",
        "A1",
        &newer,
        "2026-11-07T13:00:00+08:00",
        json!({ "installId": "install-Y" }),
    );
    run_import(&env, &mut conn);
    assert_eq!(build_student_view(&conn, "A1").unwrap().raw_score, 8.0);
    let failed = list_failed_imports(&conn).unwrap();
    assert!(failed.iter().any(|r| r.error.as_deref().unwrap_or("").contains("不同安装")), "{failed:?}");
}

/// 「导入异常」导出只含本批次/试卷的失败；其他批次的失败仍在总览异常里。
#[test]
fn exception_export_is_scoped_to_group() {
    let env = fresh_env("scope");
    seed_standard(&env);
    let mut conn = open(&env);
    run_import(&env, &mut conn);
    let bytes = db_bytes("other", &t2_attempts());
    publish_snapshot_with_meta(
        &env.submissions,
        "T9",
        "A90",
        &bytes,
        "2026-11-07T12:30:00+08:00",
        json!({ "batchId": "B-OTHER", "sha256": "0".repeat(64) }),
    );
    run_import(&env, &mut conn);
    let all = list_failed_imports(&conn).unwrap();
    assert!(all.iter().any(|r| r.file == "snapshots/T9/A90.sqlite"), "{all:?}");
    let scoped = list_failed_imports_for_group(&conn, BATCH, PAPER).unwrap();
    assert!(scoped.iter().all(|r| r.batch_id.as_deref() != Some("B-OTHER")), "{scoped:?}");
    assert_eq!(scoped.len(), 3, "{scoped:?}");
}
