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
    let mut conn = open_library_connection(root)?;
    // 按需迁移：首次访问旧 job 时从 artifact（revision/shadow）填充权威稿。
    let seeded = migrate_single_item(root, item_id).unwrap_or_else(|error| {
        eprintln!("[library] on-demand migration failed for {item_id}: {error}");
        false
    });
    align_reading_part_best_effort(&mut conn, item_id);
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
    let mut conn = open_library_connection(root)?;
    // Only load full documents for mismatched reading rows. Normal list refreshes
    // must retain the lightweight summary path rather than deserialize every exam.
    let mismatched = (|| -> CommandResult<Vec<String>> {
        let mut query = conn.prepare("SELECT id FROM library_items_v2
            WHERE modality='reading' AND (?1 OR deleted_at IS NULL)
              AND json_valid(canonical_ds_json)
              AND CASE WHEN json_valid(canonical_ds_json)
                  THEN (part_source IS NULL OR part_label IS NOT json_extract(canonical_ds_json,'$.exam.category'))
                  ELSE 0 END")
            .map_err(|e| format!("part_alignment_query:{e}"))?;
        let ids = query.query_map([include_deleted], |row| row.get::<_, String>(0))
            .map_err(|e| format!("part_alignment_query:{e}"))?
            .collect::<Result<Vec<_>,_>>().map_err(|e| format!("part_alignment_rows:{e}"))?;
        Ok(ids)
    })().unwrap_or_else(|error| {
        eprintln!("[library] part alignment scan skipped: {error}");
        Vec::new()
    });
    for id in mismatched {
        align_reading_part_best_effort(&mut conn, &id);
    }
    let mut rows = list_items(&conn, include_deleted)?;
    // 首次加载惰性判定 Part：判定（读 DS）先做，写入合并进一个事务、且带 part_source IS NULL
    // 守卫——避免读路径上 N 次串行写，也不覆盖并发的手动设置。判不出记 sentinel，之后不再重算。
    let mut pending: Vec<(String, Option<String>, String)> = Vec::new();
    for row in rows.iter_mut() {
        if row.part_source.is_some() || !row.has_canonical_ds {
            continue;
        }
        if let Some((label, source)) = compute_part_for_row(&conn, row) {
            row.part_label = label.clone();
            row.part_source = Some(source.clone());
            pending.push((row.id.clone(), label, source));
        }
    }
    if !pending.is_empty() {
        if let Err(error) = persist_part_backfill(&conn, &pending) {
            // 回填失败不影响列表返回：下次加载会再试（part_source 仍为 NULL）。
            eprintln!("[library] part backfill batch failed: {error}");
        }
    }

    let mut result = Vec::new();
    for row in rows {
        let processing = crate::processing::queue::get_job_by_library_item(&conn, &row.id)?;
        let mut value = serde_json::to_value(row).map_err(|error| error.to_string())?;
        value["processing"] =
            serde_json::to_value(processing).map_err(|error| error.to_string())?;
        result.push(value);
    }
    Ok(Value::Array(result))
}

/// Refresh one library row after a processing event without loading the whole library.
/// Uses the same Part detection/backfill and serialized fields as the initial list.
pub(crate) fn get_library_item_summary_core(root: &Path, item_id: &str) -> CommandResult<Value> {
    let mut conn = open_library_connection(root)?;
    align_reading_part_best_effort(&mut conn, item_id);
    let Some(mut row) = get_item(&conn, item_id)? else {
        return Ok(Value::Null);
    };
    if row.part_source.is_none() && row.has_canonical_ds {
        if let Some((label, source)) = compute_part_for_row(&conn, &row) {
            row.part_label = label.clone();
            row.part_source = Some(source.clone());
            if let Err(error) = persist_part_backfill(&conn, &[(row.id.clone(), label, source)]) {
                eprintln!("[library] part backfill failed for {item_id}: {error}");
            }
        }
    }
    let processing = crate::processing::queue::get_job_by_library_item(&conn, item_id)?;
    let mut value = serde_json::to_value(row).map_err(|error| error.to_string())?;
    value["processing"] = serde_json::to_value(processing).map_err(|error| error.to_string())?;
    Ok(value)
}

/// 手动设置某条目的 Part 标签：来源记为 `manual`，压过一切自动判定。
/// 阅读通过权威稿 CAS 事务同步；听力 Part 1–4 / 写作 Task 1/2 仍仅保存题库列。
/// 清除手动值后重新判定；判不出时保留原 category，来源记为 none。
pub(crate) fn set_library_item_part_core(
    root: &Path,
    item_id: &str,
    label: Option<&str>,
) -> CommandResult<bool> {
    let mut conn = open_library_connection(root)?;
    let trimmed = label.map(str::trim).filter(|value| !value.is_empty());
    let Some(row) = get_item(&conn, item_id)? else { return Ok(false); };
    if row.modality == "reading" {
        if let Some(value) = trimmed {
            if !matches!(value, "P1" | "P2" | "P3") {
                return Err("READING_PART_INVALID:expected=P1|P2|P3".to_string());
            }
        }
        if row.has_canonical_ds {
            write_reading_part(&mut conn, &row, trimmed)?;
            return Ok(true);
        }
    }
    match trimmed {
        Some(value) => super::repository::set_item_part(&conn, item_id, Some(value), Some("manual")),
        None => super::repository::set_item_part(&conn, item_id, None, None),
    }
}

fn write_reading_part(
    conn: &mut rusqlite::Connection,
    row: &super::repository::LibraryItemRowV2,
    manual: Option<&str>,
) -> CommandResult<()> {
    let (ds, version) = get_canonical_ds(conn, &row.id)?.ok_or("ITEM_DS_NOT_SEEDED")?;
    if version != row.current_edit_version { return Err("EDIT_VERSION_CONFLICT:part_assignment".to_string()); }
    let detected = compute_part_for_row(conn, row);
    let (category, source) = if let Some(label) = manual {
        (label.to_string(), "manual".to_string())
    } else {
        match detected {
            Some((Some(label), source)) => (label, source),
            _ => (ds.pointer("/exam/category").and_then(Value::as_str).unwrap_or("P1").to_string(), PART_SOURCE_NONE.to_string()),
        }
    };
    commit_reading_part(conn, row, version, &category, &source, super::repository::EditOrigin::Human)
}

fn commit_reading_part(
    conn: &mut rusqlite::Connection,
    row: &super::repository::LibraryItemRowV2,
    version: i64,
    category: &str,
    source: &str,
    origin: super::repository::EditOrigin,
) -> CommandResult<()> {
    let input = ApplyEditorCommandsInput {
        item_id: row.id.clone(), base_version: version, request_id: None, title: None,
        commands: vec![json!({"op":"setExamCategory", "category":category})],
    };
    super::repository::apply_editor_commands_tx_with(
        conn, &input, origin, None,
        &apply_patch, &|ds| validate_authoring(ds),
        &|tx, _| super::repository::set_item_part(tx, &row.id, Some(category), Some(source)).map(|_| ()),
    )?;
    Ok(())
}

/// Reads must survive a failed migration or old, schema-incomplete document. The
/// write transaction rolls back; later reads retry. Explicit edit commands do not
/// use this wrapper and still report their errors to the author.
fn align_reading_part_best_effort(conn: &mut rusqlite::Connection, id: &str) {
    if let Err(error) = align_reading_part(conn, id) {
        eprintln!("[library] part alignment skipped for {id}: {error}");
    }
}

/// Historical manual choices win. An automatic label predating category edits
/// can repair the old import's default P1, without acquiring human protection.
/// Other rows project canonical category. All content writes retain CAS and locks.
fn align_reading_part(conn: &mut rusqlite::Connection, item_id: &str) -> CommandResult<()> {
    let Some(row) = get_item(conn, item_id)? else { return Ok(()); };
    if row.modality != "reading" || !row.has_canonical_ds { return Ok(()); }
    let Some((ds, version)) = get_canonical_ds(conn, item_id)? else { return Ok(()); };
    if version != row.current_edit_version { return Err("EDIT_VERSION_CONFLICT:part_alignment".to_string()); }
    let category = ds.pointer("/exam/category").and_then(Value::as_str);
    if row.part_source.as_deref() == Some("manual") {
        if let Some(label) = row.part_label.as_deref().filter(|v| matches!(*v, "P1" | "P2" | "P3")) {
            if category != Some(label) {
                write_reading_part(conn, &row, Some(label))?;
            }
        }
    } else {
        if super::repository::is_automatic_part_source(row.part_source.as_deref())
            && !super::repository::has_human_category_edit(conn, item_id)?
        {
            if let Some(label) = row.part_label.as_deref()
                .filter(|v| matches!(*v, "P1" | "P2" | "P3") && Some(*v) != category)
            {
                return commit_reading_part(conn, &row, version, label,
                    row.part_source.as_deref().unwrap(), super::repository::EditOrigin::PartBackfill);
            }
        }
        if let Some(category) = category.filter(|v| matches!(*v, "P1" | "P2" | "P3")) {
            // Guard the projection against a concurrent save or new manual choice.
            conn.execute("UPDATE library_items_v2 SET part_label=?2, part_source=COALESCE(part_source,'canonical')
                WHERE id=?1 AND current_edit_version=?3 AND (part_source IS NULL OR part_source != 'manual')",
                rusqlite::params![item_id, category, row.current_edit_version])
                .map_err(|e| format!("part_projection:{e}"))?;
        }
    }
    Ok(())
}

/// 判不出 Part 时写入的 sentinel：区分「算过但没有」与「还没算过（NULL）」，避免每次列表都重算。
const PART_SOURCE_NONE: &str = "none";

/// 从权威稿 + 标题判定该行的 Part（只读、不写库）。返回 `(标签, 来源)`；判不出时标签为
/// None、来源为 sentinel。DS 读不到则返回 None（本行不参与回填）。
fn compute_part_for_row(
    conn: &rusqlite::Connection,
    row: &super::repository::LibraryItemRowV2,
) -> Option<(Option<String>, String)> {
    let (ds, _) = super::repository::get_canonical_ds(conn, &row.id).ok()??;
    if row.modality == "reading" {
        return Some(match super::part_detection::reading_part_from_document(&row.title, &ds) {
            Some(part) => (Some(part.label), part.source.as_str().to_string()),
            None => (None, PART_SOURCE_NONE.to_string()),
        });
    }
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
    Some(match detected {
        Some(part) => (Some(part.label), part.source.as_str().to_string()),
        None => (None, PART_SOURCE_NONE.to_string()),
    })
}

/// 把一批 Part 回填写入同一个 IMMEDIATE 事务，每条都带 `part_source IS NULL` 守卫。
fn persist_part_backfill(
    conn: &rusqlite::Connection,
    pending: &[(String, Option<String>, String)],
) -> CommandResult<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("part_backfill_begin:{error}"))?;
    for (id, label, source) in pending {
        super::repository::backfill_item_part(&tx, id, label.as_deref(), source)?;
    }
    tx.commit()
        .map_err(|error| format!("part_backfill_commit:{error}"))
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

#[cfg(test)]
mod part_sync_tests {
    use super::*;
    use super::super::repository::{seed_canonical_ds, upsert_item_shell, UpsertItemInput};

    fn fixture(title: &str, modality: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("part-sync-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let conn = open_library_connection(&root).unwrap();
        upsert_item_shell(&conn, &UpsertItemInput { id: "item", modality, title, status: "ready", source_asset_id: None }).unwrap();
        let mut ds: Value = serde_json::from_str(include_str!("../../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json")).unwrap();
        ds["exam"]["category"] = json!("P1");
        ds["exam"]["examId"] = json!("p1-stable");
        ds["exam"]["title"] = json!(title);
        ds["taskGroups"] = json!([]);
        ds["answerSlots"] = json!({});
        ds["answerKey"] = json!({});
        ds["passage"] = json!({"content":[],"sourceAnchors":[]});
        seed_canonical_ds(&conn, "item", &ds.to_string(), "ready").unwrap();
        root
    }

    #[test]
    fn failed_lazy_alignment_does_not_block_reads_or_change_the_bad_item() {
        let root = fixture("Normal", "reading");
        let conn = open_library_connection(&root).unwrap();
        let (mut invalid, _) = get_canonical_ds(&conn, "item").unwrap().unwrap();
        // Use the current quality format so opening does not independently run
        // the pre-existing V1 compatibility-quality migration. This regression
        // isolates failed Part alignment and still compares the entire document.
        invalid["quality"]["compilerProbes"]["v1Compatibility"]["status"] = json!("skipped");
        invalid["passage"].as_object_mut().unwrap().remove("sourceAnchors");
        assert!(validate_authoring(&invalid).is_err());
        upsert_item_shell(&conn, &UpsertItemInput { id:"bad", modality:"reading", title:"Legacy", status:"ready", source_asset_id:None }).unwrap();
        seed_canonical_ds(&conn,"bad",&invalid.to_string(),"ready").unwrap();
        super::super::repository::set_item_part(&conn,"bad",Some("P3"),Some("manual")).unwrap();
        let before = get_canonical_ds(&conn,"bad").unwrap();
        for _ in 0..2 {
            let list = list_library_items_core(&root,false).unwrap();
            assert_eq!(list.as_array().unwrap().len(),2);
            let workspace = get_workspace_item_core(&root,"bad").unwrap();
            assert_eq!(workspace["ds"],invalid);
            assert_eq!(workspace["editVersion"],1);
            assert_eq!(get_library_item_summary_core(&root,"bad").unwrap()["partLabel"],"P3");
            assert_eq!(get_canonical_ds(&conn,"bad").unwrap(),before);
        }
        // User actions still fail loudly and roll back; only opportunistic reads
        // get the best-effort wrapper.
        assert!(set_library_item_part_core(&root,"bad",Some("P2")).unwrap_err().contains("AUTHORING_SCHEMA_INVALID"));
        assert!(apply_editor_commands_core(&root,ApplyEditorCommandsInput {
            item_id:"bad".into(),base_version:1,request_id:None,title:None,
            commands:vec![json!({"op":"setExamCategory","category":"P2"})],
        }).is_err());
        assert_eq!(get_canonical_ds(&conn,"bad").unwrap(),before);
        assert_eq!(get_item(&conn,"bad").unwrap().unwrap().part_label.as_deref(),Some("P3"));
    }

    #[test]
    fn historical_automatic_part_repairs_once_without_human_protection() {
        for source in ["title","content","range","filename"] {
            let root = fixture("Legacy", "reading");
            let conn = open_library_connection(&root).unwrap();
            super::super::repository::set_item_part(&conn,"item",Some("P2"),Some(source)).unwrap();
            let before_protection: Option<String> = conn.query_row("SELECT protected_edits_json FROM library_items_v2 WHERE id='item'",[],|row|row.get(0)).unwrap();
            assert_eq!(list_library_items_core(&root,false).unwrap()[0]["partLabel"],"P2");
            list_library_items_core(&root,false).unwrap();
            get_workspace_item_core(&root,"item").unwrap();
            get_library_item_summary_core(&root,"item").unwrap();
            let (ds,version) = get_canonical_ds(&conn,"item").unwrap().unwrap();
            assert_eq!((ds["exam"]["category"].as_str(),version),(Some("P2"),2));
            assert_eq!(ds["exam"]["examId"],"p1-stable");
            assert_eq!(get_item(&conn,"item").unwrap().unwrap().part_source.as_deref(),Some(source));
            assert_eq!(recent_edit_origins(&conn,"item").unwrap()[0]["origin"],"part_backfill");
            assert!(!super::super::repository::has_human_category_edit(&conn,"item").unwrap());
            let after_protection: Option<String> = conn.query_row("SELECT protected_edits_json FROM library_items_v2 WHERE id='item'",[],|row|row.get(0)).unwrap();
            assert_eq!(after_protection,before_protection);
        }
    }

    #[test]
    fn human_category_history_survives_pruning_and_prevents_automatic_override() {
        let root = fixture("Legacy", "reading");
        let mut conn = open_library_connection(&root).unwrap();
        set_library_item_part_core(&root,"item",Some("P3")).unwrap();
        for base_version in 2..204 {
            super::super::repository::apply_editor_commands_tx(&mut conn,&ApplyEditorCommandsInput {
                item_id:"item".into(),base_version,request_id:None,title:Some("Renamed".into()),commands:vec![],
            },&apply_patch,&|ds|validate_authoring(ds)).unwrap();
        }
        assert!(super::super::repository::has_human_category_edit(&conn,"item").unwrap());
        let journal_count: i64 = conn.query_row("SELECT COUNT(*) FROM editor_journal_v1 WHERE library_item_id='item'",[],|row|row.get(0)).unwrap();
        assert!(journal_count <= 201);
        super::super::repository::set_item_part(&conn,"item",Some("P2"),Some("range")).unwrap();
        let before = get_canonical_ds(&conn,"item").unwrap();
        list_library_items_core(&root,false).unwrap();
        assert_eq!(get_canonical_ds(&conn,"item").unwrap(),before);
        assert_eq!(get_item(&conn,"item").unwrap().unwrap().part_label.as_deref(),Some("P3"));
    }

    #[test]
    fn unrelated_save_preserves_legacy_auto_label_until_system_backfill_can_run() {
        let root = fixture("Legacy", "reading");
        let conn = open_library_connection(&root).unwrap();
        super::super::repository::set_item_part(&conn,"item",Some("P2"),Some("range")).unwrap();
        apply_editor_commands_core(&root,ApplyEditorCommandsInput {
            item_id:"item".into(),base_version:1,request_id:None,title:Some("Renamed".into()),commands:vec![],
        }).unwrap();
        assert_eq!(get_item(&conn,"item").unwrap().unwrap().part_label.as_deref(),Some("P2"));
        assert_eq!(get_canonical_ds(&conn,"item").unwrap().unwrap().0["exam"]["category"],"P1");
        crate::processing::queue::enqueue(&conn,"job","item","source",&json!({})).unwrap();
        conn.execute("UPDATE processing_jobs_v2 SET stage='cloud_recognition', cloud_status='running' WHERE id='job'",[]).unwrap();
        list_library_items_core(&root,false).unwrap();
        assert_eq!(get_canonical_ds(&conn,"item").unwrap().unwrap().1,2);
        conn.execute("UPDATE processing_jobs_v2 SET stage='ready_for_review', cloud_status='succeeded' WHERE id='job'",[]).unwrap();
        list_library_items_core(&root,false).unwrap();
        let (ds,version) = get_canonical_ds(&conn,"item").unwrap().unwrap();
        assert_eq!((ds["exam"]["category"].as_str(),version),(Some("P2"),3));
        assert_eq!(recent_edit_origins(&conn,"item").unwrap()[0]["origin"],"part_backfill");
    }

    #[test]
    fn reading_part_assignment_is_versioned_atomic_and_clear_redetects() {
        let root = fixture("P2 sample", "reading");
        assert!(set_library_item_part_core(&root, "item", Some("P3")).unwrap());
        let conn = open_library_connection(&root).unwrap();
        let (ds, version) = get_canonical_ds(&conn, "item").unwrap().unwrap();
        assert_eq!(version, 2);
        assert_eq!(ds["exam"]["category"], "P3");
        assert_eq!(ds["exam"]["examId"], "p1-stable");
        let row = get_item(&conn, "item").unwrap().unwrap();
        assert_eq!(row.part_label.as_deref(), Some("P3"));
        assert_eq!(row.part_source.as_deref(), Some("manual"));
        assert_eq!(recent_edit_origins(&conn, "item").unwrap()[0]["origin"], "human");
        assert!(apply_editor_commands_core(&root, ApplyEditorCommandsInput { item_id:"item".into(), base_version:1,
            request_id:None, title:None, commands:vec![json!({"op":"setExamCategory","category":"P1"})] }).unwrap_err().contains("EDIT_VERSION_CONFLICT"));
        set_library_item_part_core(&root, "item", None).unwrap();
        let (ds, version) = get_canonical_ds(&conn, "item").unwrap().unwrap();
        assert_eq!(version, 3);
        assert_eq!(ds["exam"]["category"], "P2");
        assert_eq!(get_item(&conn, "item").unwrap().unwrap().part_source.as_deref(), Some("filename"));
    }

    #[test]
    fn invalid_reading_label_rolls_back_and_unknown_clear_preserves_category() {
        let root = fixture("Unknown", "reading");
        assert!(set_library_item_part_core(&root, "item", Some("Part 1")).unwrap_err().contains("READING_PART_INVALID"));
        let conn = open_library_connection(&root).unwrap();
        assert_eq!(get_canonical_ds(&conn,"item").unwrap().unwrap().1,1);
        set_library_item_part_core(&root, "item", Some("P3")).unwrap();
        set_library_item_part_core(&root, "item", None).unwrap();
        let (ds, version) = get_canonical_ds(&conn, "item").unwrap().unwrap();
        assert_eq!(version,3);
        assert_eq!(ds["exam"]["category"],"P3");
        assert_eq!(get_item(&conn,"item").unwrap().unwrap().part_source.as_deref(),Some("none"));
    }

    #[test]
    fn historical_manual_mismatch_aligns_once_and_editor_save_projects_to_library() {
        let root = fixture("24. P1 The Pearls", "reading");
        let conn = open_library_connection(&root).unwrap();
        super::super::repository::set_item_part(&conn,"item",Some("P3"),Some("manual")).unwrap();
        list_library_items_core(&root,false).unwrap();
        get_workspace_item_core(&root,"item").unwrap();
        list_library_items_core(&root,false).unwrap();
        let (ds, version) = get_canonical_ds(&conn,"item").unwrap().unwrap();
        assert_eq!(version,2);
        assert_eq!(ds["exam"]["category"],"P3");
        apply_editor_commands_core(&root, ApplyEditorCommandsInput { item_id:"item".into(), base_version:version,
            request_id:None, title:None, commands:vec![json!({"op":"setExamCategory","category":"P2"})] }).unwrap();
        let row = get_item(&conn,"item").unwrap().unwrap();
        assert_eq!(row.part_label.as_deref(),Some("P2"));
        assert_eq!(row.title,"24. P1 The Pearls");
    }

    #[test]
    fn listening_and_writing_labels_do_not_change_document_or_version() {
        for (modality,label) in [("listening","Part 4"),("writing","Task 2")] {
            let root = fixture("Other",modality);
            let conn = open_library_connection(&root).unwrap();
            let before = get_canonical_ds(&conn,"item").unwrap();
            set_library_item_part_core(&root,"item",Some(label)).unwrap();
            assert_eq!(get_canonical_ds(&conn,"item").unwrap(),before);
            assert_eq!(get_item(&conn,"item").unwrap().unwrap().part_label.as_deref(),Some(label));
        }
    }

    #[test]
    fn cloud_review_locks_part_and_lazy_repair_waits_for_unlock() {
        let root = fixture("P1 sample", "reading");
        let conn = open_library_connection(&root).unwrap();
        crate::processing::queue::enqueue(&conn,"job","item","source",&json!({})).unwrap();
        conn.execute("UPDATE processing_jobs_v2 SET stage='cloud_recognition', cloud_status='running' WHERE id='job'",[]).unwrap();
        let before = get_canonical_ds(&conn,"item").unwrap();
        assert!(set_library_item_part_core(&root,"item",Some("P3")).unwrap_err().contains("CLOUD_REVIEW_IN_PROGRESS"));
        super::super::repository::set_item_part(&conn,"item",Some("P3"),Some("manual")).unwrap();
        list_library_items_core(&root,false).unwrap();
        assert_eq!(get_canonical_ds(&conn,"item").unwrap(),before);
        conn.execute("UPDATE processing_jobs_v2 SET stage='ready_for_review', cloud_status='succeeded' WHERE id='job'",[]).unwrap();
        list_library_items_core(&root,false).unwrap();
        assert_eq!(get_canonical_ds(&conn,"item").unwrap().unwrap().0["exam"]["category"],"P3");
    }
}
