//! 「题库保存」：发布后冻结可编辑的最终版证据，并删除原文件与过程文件。
//!
//! 产品定义（产品负责人）：一道题发布（严格或放行）后，SQLite 里只保留**一份**可编辑
//! 最终版——就是 `library_items_v2.canonical_ds_json` 本身，这里不复制第二份稿；
//! 原文件（PDF/DOCX）与全部过程文件在发布提交之后删除。之后重新打开仍可编辑、保存、
//! 再次正常发布。
//!
//! 删除原文件会让 physical shadow 消失；质量评估改用这里冻结的证据摘要
//! （`ielts_grammar::quality::FrozenSourceEvidence`），题号集合仍逐题核对。
//!
//! 边界（不可放宽）：
//! - 只在发布**提交之后**清理，且只清理本条目自己的 `jobs/<itemId>/`；
//! - 保留权威稿引用的资产文件（图片等）与作业元数据 `job.json`；
//! - 从不触碰 `<appData>/audio/`（听力托管音频属于可编辑版本的一部分）；
//! - 清理失败不回滚、不失败发布，只如实报告。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::util::{is_safe_path_segment, job_dir};
use crate::CommandResult;

/// 已清理条目上被拒绝的「需要原文件」的操作（重新识别 / 云端修复 / 答案页重试）。
pub(crate) const SOURCE_PURGED_ERROR: &str = "ITEM_SOURCE_PURGED_AFTER_PUBLISH";

const DOCUMENT_V2_SHADOW_FILE: &str = "document-ir-v2.shadow.json";
/// 作业元数据：标题、来源文件名等，不是过程文件，也不含原文件内容。
const KEPT_JOB_METADATA: &str = "job.json";

/// 发布提交之后冻结最终版证据。
///
/// 证据来源优先级：
/// 1. 本条目的 physical shadow 仍在（首次发布）⇒ 按事实计算；
/// 2. 已清理（再次发布）⇒ 沿用上一次冻结的题号声明与节点覆盖结论，只刷新版本/时间；
/// 3. 都没有 ⇒ 如实冻结「未能判定」（空声明集合），之后仍报 Undetermined。
pub(crate) fn freeze_final_version(
    conn: &Connection,
    root: &Path,
    item_id: &str,
    edit_version: i64,
    publish_record_id: &str,
    ds: &Value,
) -> CommandResult<Value> {
    let shadow = crate::util::read_json_opt(&job_dir(root, item_id).join(DOCUMENT_V2_SHADOW_FILE))
        .ok()
        .flatten()
        .filter(|shadow| crate::authoring_v2_commands::physical_shadow_matches_authoring(shadow, ds));
    let previous: Option<Value> = conn
        .query_row(
            "SELECT evidence_json FROM library_final_versions_v2 WHERE library_item_id = ?1",
            [item_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("final_version_read:{error}"))?
        .and_then(|text| serde_json::from_str(&text).ok());
    let mut evidence = match (shadow.as_ref(), previous.as_ref()) {
        (Some(shadow), _) => crate::ielts_grammar::quality::publish_evidence_summary(ds, Some(shadow)),
        (None, Some(previous))
            if crate::ielts_grammar::quality::FrozenSourceEvidence::from_value(previous).is_some() =>
        {
            let mut carried = previous.clone();
            if let Some(frozen) = crate::ielts_grammar::quality::FrozenSourceEvidence::from_value(previous) {
                carried["questionCoverage"] =
                    crate::ielts_grammar::source_coverage::assess_against_frozen_declaration(
                        ds,
                        &frozen.declared_question_numbers,
                        &frozen.declarations,
                    )
                    .as_value();
            }
            carried
        }
        _ => crate::ielts_grammar::quality::publish_evidence_summary(ds, None),
    };
    let now = Utc::now().to_rfc3339();
    evidence["publishedEditVersion"] = json!(edit_version);
    evidence["publishedAt"] = json!(now);
    evidence["evidenceSource"] = json!(if shadow.is_some() {
        "physical_shadow"
    } else if previous.is_some() {
        "carried_from_previous_publish"
    } else {
        "unavailable_at_publish"
    });
    conn.execute(
        "INSERT INTO library_final_versions_v2
         (library_item_id, edit_version, published_at, publish_record_id, evidence_json,
          source_purged_at, purge_report_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?3)
         ON CONFLICT(library_item_id) DO UPDATE SET
             edit_version = excluded.edit_version,
             published_at = excluded.published_at,
             publish_record_id = excluded.publish_record_id,
             evidence_json = excluded.evidence_json,
             updated_at = excluded.updated_at",
        params![item_id, edit_version, now, publish_record_id, evidence.to_string()],
    )
    .map_err(|error| format!("final_version_write:{error}"))?;
    Ok(evidence)
}

/// 删除本条目 job 目录里除「资产文件 + job.json」以外的一切（原文件、shadow、
/// revision、报告、导出历史……）。逐项尽力删除，失败项进报告，不中断。
///
/// 不跟随符号链接：链接本身被删除，链接指向的内容不动。
pub(crate) fn purge_source_artifacts(root: &Path, item_id: &str, ds: &Value) -> Value {
    let mut report = json!({
        "schemaVersion": "SourcePurgeReportV1",
        "itemId": item_id,
        "removedFiles": 0,
        "removedDirs": 0,
        "keptFiles": [],
        "failed": []
    });
    if !is_safe_path_segment(item_id) {
        report["failed"] = json!([{"path": item_id, "error": "unsafe_item_id"}]);
        return report;
    }
    let dir = job_dir(root, item_id);
    if !dir.is_dir() {
        return report;
    }
    let kept = kept_relative_paths(ds);
    report["keptFiles"] = json!(kept
        .iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>());
    let mut removed_files = 0usize;
    let mut removed_dirs = 0usize;
    let mut failed = Vec::new();
    purge_dir(&dir, Path::new(""), &kept, &mut removed_files, &mut removed_dirs, &mut failed);
    // 解析缓存（`<appData>/cache/parser/`）在 job 目录之外，但同样是**本条目**的源产物：
    // 它是原文抽取的结果，发布后不该留在磁盘上。修前这一步漏了，于是发布后这道题的
    // 解析产物仍躺在缓存里，与「发布后只保留可编辑最终版」相违。
    // 归属按精确身份匹配（见 `cleanup::cleanup_parser_cache_for_job`），id 相近的
    // 另一条不受牵连。失败只记进报告，绝不让发布失败或回滚——包已经对学生可见了。
    let parser_cache = match crate::cleanup::cleanup_parser_cache_for_job(root, item_id) {
        Ok(()) => json!({"cleaned": true}),
        Err(error) => {
            failed.push(json!({"path": "cache/parser", "error": error}));
            json!({"cleaned": false})
        }
    };
    report["removedFiles"] = json!(removed_files);
    report["removedDirs"] = json!(removed_dirs);
    report["failed"] = json!(failed);
    report["parserCache"] = parser_cache;
    report["purgedAt"] = json!(Utc::now().to_rfc3339());
    report
}

fn kept_relative_paths(ds: &Value) -> BTreeSet<PathBuf> {
    let mut kept = BTreeSet::new();
    kept.insert(PathBuf::from(KEPT_JOB_METADATA));
    for asset in ds.get("assets").and_then(Value::as_array).into_iter().flatten() {
        let Some(relative) = asset.get("relativePath").and_then(Value::as_str) else {
            continue;
        };
        let path = PathBuf::from(relative.trim());
        let safe = !path.as_os_str().is_empty()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        if safe {
            kept.insert(path);
        }
    }
    kept
}

/// 返回该目录（相对路径 `relative`）下是否还有被保留的内容。
fn purge_dir(
    absolute: &Path,
    relative: &Path,
    kept: &BTreeSet<PathBuf>,
    removed_files: &mut usize,
    removed_dirs: &mut usize,
    failed: &mut Vec<Value>,
) -> bool {
    let entries = match fs::read_dir(absolute) {
        Ok(entries) => entries,
        Err(error) => {
            failed.push(json!({"path": relative.to_string_lossy(), "error": error.to_string()}));
            return true;
        }
    };
    let mut keeps_anything = false;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failed.push(json!({"path": relative.to_string_lossy(), "error": error.to_string()}));
                keeps_anything = true;
                continue;
            }
        };
        let child_relative = relative.join(entry.file_name());
        let child_absolute = entry.path();
        let metadata = match fs::symlink_metadata(&child_absolute) {
            Ok(metadata) => metadata,
            Err(error) => {
                failed.push(json!({"path": child_relative.to_string_lossy(), "error": error.to_string()}));
                keeps_anything = true;
                continue;
            }
        };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            let child_keeps = purge_dir(&child_absolute, &child_relative, kept, removed_files, removed_dirs, failed);
            if child_keeps {
                keeps_anything = true;
            } else if let Err(error) = fs::remove_dir(&child_absolute) {
                failed.push(json!({"path": child_relative.to_string_lossy(), "error": error.to_string()}));
                keeps_anything = true;
            } else {
                *removed_dirs += 1;
            }
            continue;
        }
        if kept.contains(&child_relative) {
            keeps_anything = true;
            continue;
        }
        // Windows 的目录符号链接/联接点要用 remove_dir 删除链接本身（不进入目标）。
        let removal = fs::remove_file(&child_absolute).or_else(|error| {
            if metadata.file_type().is_symlink() {
                fs::remove_dir(&child_absolute)
            } else {
                Err(error)
            }
        });
        match removal {
            Ok(()) => *removed_files += 1,
            Err(error) => {
                failed.push(json!({"path": child_relative.to_string_lossy(), "error": error.to_string()}));
                keeps_anything = true;
            }
        }
    }
    keeps_anything
}

pub(crate) fn mark_source_purged(conn: &Connection, item_id: &str, report: &Value) -> CommandResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE library_final_versions_v2
         SET source_purged_at = COALESCE(source_purged_at, ?2), purge_report_json = ?3, updated_at = ?2
         WHERE library_item_id = ?1",
        params![item_id, now, report.to_string()],
    )
    .map_err(|error| format!("final_version_mark_purged:{error}"))?;
    Ok(())
}

/// 已清理条目的最终版状态（给工作区/题库展示用）；未发布或未清理返回 `None`。
pub(crate) fn final_version_status(conn: &Connection, item_id: &str) -> CommandResult<Option<Value>> {
    conn.query_row(
        "SELECT edit_version, published_at, source_purged_at FROM library_final_versions_v2
         WHERE library_item_id = ?1",
        [item_id],
        |row| {
            let purged_at: Option<String> = row.get(2)?;
            Ok(json!({
                "publishedEditVersion": row.get::<_, i64>(0)?,
                "publishedAt": row.get::<_, String>(1)?,
                "sourcePurged": purged_at.is_some(),
                "sourcePurgedAt": purged_at
            }))
        },
    )
    .optional()
    .map_err(|error| format!("final_version_status:{error}"))
}

/// 只读连接：可在另一条连接持有写事务时调用（WAL），不跑迁移、不抢写锁。
/// 数据库或表不存在一律视为「没有冻结证据」——即回到今天的行为（缺 shadow 照旧阻断），
/// 永远不会因为读不到而把条目当成已清理。
fn open_read_only(root: &Path) -> Option<Connection> {
    let path = crate::db::db_path(root);
    if !path.is_file() {
        return None;
    }
    Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

/// 已清理条目的冻结证据；未清理（包括「缺 shadow 但从未发布清理」）返回 `None`。
pub(crate) fn load_purged_evidence(
    root: &Path,
    item_id: &str,
) -> Option<crate::ielts_grammar::quality::FrozenSourceEvidence> {
    let conn = open_read_only(root)?;
    let text: String = conn
        .query_row(
            "SELECT evidence_json FROM library_final_versions_v2
             WHERE library_item_id = ?1 AND source_purged_at IS NOT NULL",
            [item_id],
            |row| row.get(0),
        )
        .ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    crate::ielts_grammar::quality::FrozenSourceEvidence::from_value(&value)
}

pub(crate) fn is_source_purged(root: &Path, item_id: &str) -> bool {
    let Some(conn) = open_read_only(root) else {
        return false;
    };
    conn.query_row(
        "SELECT 1 FROM library_final_versions_v2
         WHERE library_item_id = ?1 AND source_purged_at IS NOT NULL",
        [item_id],
        |_| Ok(()),
    )
    .is_ok()
}

/// 需要原文件的操作（重新识别 / 云端修复 / 答案页重试）在已清理条目上明确报错。
pub(crate) fn ensure_source_available(root: &Path, item_id: &str) -> CommandResult<()> {
    if is_source_purged(root, item_id) {
        return Err(format!("{SOURCE_PURGED_ERROR}:{item_id}"));
    }
    Ok(())
}
