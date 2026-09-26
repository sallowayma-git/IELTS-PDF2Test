//! M1（原 P2-T03 / P3-T03 后端半边）：Workspace API。
//!
//! - `get_workspace_item`：读库 + 按需迁移填充；首稿未生成时 `ds=null`（计划 §3 契约）。
//! - `apply_editor_commands`：计划 §9.5 保存链的事务入口。编辑、质量重算与校验
//!   都发生在同一个事务里，提交后**不再**回写任何派生文件或整份覆盖 DB
//!   （原先「提交后再读-刷新-整份写回」是丢稿竞态与假失败的来源，已移除），
//!   也不追加 revision 文件树（P2-T04 的「新编辑不再建 revision」）。
//!   因此导出/发布链改为直接解析 canonical DS，不再依赖 shadow 缓存。
//! - `list_library_items`：题库列表改读 V2 仓库。

use std::path::Path;

use serde_json::{json, Value};

use super::migration::migrate_single_item;
use super::repository::{
    apply_editor_commands_tx, get_canonical_ds, get_item, list_items, open_library_connection,
    ApplyEditorCommandsInput, EditFootprint,
};
use crate::authoring_v2_commands::{
    apply_patch, explicitly_handled_issue_targets, refresh_quality_report_for_targets,
    validate_authoring,
};
use crate::CommandResult;

pub(crate) fn get_workspace_item_core(root: &Path, item_id: &str) -> CommandResult<Value> {
    let conn = open_library_connection(root)?;
    // 按需迁移：首次访问旧 job 时从 artifact（revision/shadow）填充权威稿。
    let seeded = migrate_single_item(root, item_id).unwrap_or_else(|error| {
        eprintln!("[library] on-demand migration failed for {item_id}: {error}");
        false
    });
    let Some(item) = get_item(&conn, item_id)? else {
        return Err(format!("ITEM_NOT_FOUND:{item_id}"));
    };
    let (ds, issues) = if item.has_canonical_ds {
        let (ds, _version) = get_canonical_ds(&conn, item_id)?.ok_or("ITEM_DS_CORRUPT")?;
        let issues = ds
            .pointer("/quality/issues")
            .cloned()
            .filter(|value| value.is_array())
            .unwrap_or_else(|| json!([]));
        (Some(ds), issues)
    } else {
        (None, json!([]))
    };
    // 题库保存：发布后原文件已删除的条目要让工作区知道——需要原文件的操作
    // （重新识别 / 云端修复 / 答案页重试）在界面上禁用并说明原因。
    let final_version = super::final_version::final_version_status(&conn, item_id)?;
    let source_purged = final_version
        .as_ref()
        .and_then(|status| status.get("sourcePurged"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(json!({
        "schemaVersion": "WorkspaceItemV1",
        "item": {
            "itemId": item.id,
            "title": item.title,
            "modality": item.modality,
            "status": item.status,
            "editVersion": item.current_edit_version,
            "hasCanonicalDs": item.has_canonical_ds,
            "updatedAt": item.updated_at,
            "sourcePurged": source_purged
        },
        "finalVersion": final_version,
        "ds": ds,
        "editVersion": item.current_edit_version,
        "issues": issues,
        "seededByOnDemandMigration": seeded,
        // 最近的编辑来源（基线版本 + origin）。前端保存撞上冲突时据此判断是不是只被
        // 机器写入（云端修复 / 答案页识别）挤掉——是就自动重放，不逼用户二选一。
        "recentEdits": recent_edit_origins(&conn, item_id)?
    }))
}

fn recent_edit_origins(conn: &rusqlite::Connection, item_id: &str) -> CommandResult<Value> {
    let mut statement = conn
        .prepare(
            "SELECT base_version, edit_origin FROM editor_journal_v1
              WHERE library_item_id = ?1 ORDER BY id DESC LIMIT 50",
        )
        .map_err(|error| format!("library_v2_recent_edits:{error}"))?;
    let rows = statement
        .query_map([item_id], |row| {
            Ok(json!({
                "baseVersion": row.get::<_, i64>(0)?,
                "origin": row.get::<_, Option<String>>(1)?,
            }))
        })
        .map_err(|error| format!("library_v2_recent_edits:{error}"))?;
    let mut edits = Vec::new();
    for row in rows {
        edits.push(row.map_err(|error| format!("library_v2_recent_edits:{error}"))?);
    }
    Ok(Value::Array(edits))
}

pub(crate) fn apply_editor_commands_core(
    root: &Path,
    input: ApplyEditorCommandsInput,
) -> CommandResult<Value> {
    let mut conn = open_library_connection(root)?;
    // 本次保存的**影响范围**：在事务之外先按「改动前的稿件 + 本批命令」算出来。
    //
    // 为什么用改动前的稿件：`EditFootprint` 要沿祖先链找受影响对象，而改动后那些
    // 对象的身份可能已经变了（甚至被删掉）。改动前算出来的范围是保守且可复现的。
    // 与事务之间若有人抢先保存，CAS 会拒绝本次写入，所以这份范围不会用在过期的稿上。
    //
    // 为什么要带上它：质量重算要**重置这些目标上已过期的 resolution**——内容变了，
    // 人对旧内容作出的「已解决」判断不再成立。人在本批里亲手 `resolveIssue` 过的
    // 目标要扣掉（见 `explicitly_handled_issue_targets`），否则他自己的确认会被
    // 自己的编辑顺手清掉。
    let affected_targets = match get_canonical_ds(&conn, &input.item_id)? {
        Some((before, _)) => {
            let mut affected = EditFootprint::merge(&before, &input.commands).targets;
            // 人在本批里亲手 `resolveIssue` 过的目标要扣掉：那是他对**当前**内容的
            // 判断，不能被他自己的编辑顺手清掉。
            for target in explicitly_handled_issue_targets(&before, &input.commands) {
                affected.remove(&target);
            }
            affected
        }
        None => std::collections::BTreeSet::new(),
    };
    let result = apply_editor_commands_tx(&mut conn, &input, &apply_patch, &|ds| {
        refresh_quality_report_for_targets(root, &input.item_id, ds, &affected_targets)?;
        validate_authoring(ds)
    })?;

    Ok(json!({
        "schemaVersion": "ApplyEditorCommandsResultV1",
        "itemId": result.item_id,
        "editVersion": result.edit_version,
        "appliedCount": result.applied_count,
        "replayed": result.replayed,
        "recoverySnapshotSaved": result.recovery_snapshot_saved
    }))
}

pub(crate) fn list_library_items_core(root: &Path, include_deleted: bool) -> CommandResult<Value> {
    let conn = open_library_connection(root)?;
    let rows = list_items(&conn, include_deleted)?;
    let mut result = Vec::new();
    for mut row in rows {
        // C4：首次列表加载时惰性回填 Part 标签（判不出记 sentinel，避免每次重算；手动来源不动）。
        if row.part_source.is_none() && row.has_canonical_ds {
            backfill_part_label(&conn, &mut row);
        }
        let processing = crate::processing::queue::get_job(&conn, &row.id)?;
        let mut value = serde_json::to_value(row).map_err(|error| error.to_string())?;
        value["processing"] =
            serde_json::to_value(processing).map_err(|error| error.to_string())?;
        result.push(value);
    }
    Ok(Value::Array(result))
}

/// 手动设置某条目的 Part 标签（C4）：来源记为 `manual`，压过一切自动判定。
/// `label` 为空/None 表示清除手动值，回到自动判定（下次列表重新推断）。
pub(crate) fn set_library_item_part_core(
    root: &Path,
    item_id: &str,
    label: Option<&str>,
) -> CommandResult<bool> {
    let conn = open_library_connection(root)?;
    let trimmed = label.map(str::trim).filter(|value| !value.is_empty());
    match trimmed {
        Some(value) => super::repository::set_item_part(&conn, item_id, Some(value), Some("manual")),
        None => super::repository::set_item_part(&conn, item_id, None, None),
    }
}

/// 判不出 Part 时写入的 sentinel：区分「算过但没有」与「还没算过（NULL）」，避免每次列表都重算。
const PART_SOURCE_NONE: &str = "none";

/// 从权威稿 + 标题推断 Part 标签并回填。手动来源不覆盖。
fn backfill_part_label(conn: &rusqlite::Connection, row: &mut super::repository::LibraryItemRowV2) {
    let Ok(Some((ds, _))) = super::repository::get_canonical_ds(conn, &row.id) else {
        return;
    };
    let (lines, numbers) = part_inputs_from_ds(&ds);
    let detected = crate::library::part_detection::detect_part(
        &crate::library::part_detection::PartDetectionInput {
            modality: &row.modality,
            manual_label: None,
            task_type: ds.get("taskType").and_then(Value::as_str),
            source_lines: &lines,
            question_numbers: &numbers,
            filename: &row.title,
        },
    );
    let (label, source) = match detected {
        Some(part) => (Some(part.label), part.source.as_str().to_string()),
        None => (None, PART_SOURCE_NONE.to_string()),
    };
    if let Err(error) =
        super::repository::set_item_part(conn, &row.id, label.as_deref(), Some(&source))
    {
        eprintln!("[library] part backfill failed for {}: {error}", row.id);
        return;
    }
    row.part_label = label;
    row.part_source = Some(source);
}

/// 从权威稿抽取 Part 判定输入：题号（answerKey 的 `q<n>` 键）+ 可能含标题行的文本
/// （passage 标题/正文、exam 标题）。best-effort：抽不到就交给文件名判定。
fn part_inputs_from_ds(ds: &Value) -> (Vec<String>, Vec<u32>) {
    let mut numbers: Vec<u32> = Vec::new();
    if let Some(answer_key) = ds.get("answerKey").and_then(Value::as_object) {
        for key in answer_key.keys() {
            if let Some(digits) = key.strip_prefix('q').or(Some(key.as_str())) {
                if let Ok(number) = digits.parse::<u32>() {
                    numbers.push(number);
                }
            }
        }
    }
    let mut lines: Vec<String> = Vec::new();
    let mut push_strings = |value: Option<&Value>| {
        if let Some(text) = value.and_then(Value::as_str) {
            for line in text.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    lines.push(trimmed.to_string());
                }
            }
        }
    };
    push_strings(ds.pointer("/exam/title"));
    push_strings(ds.pointer("/passage/title"));
    push_strings(ds.pointer("/passage/content"));
    (lines, numbers)
}
