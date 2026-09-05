//! M1（原 P2-T03/P2-T04）：Canonical DS 仓库与事务编辑。
//!
//! 计划 §9.5 的保存链在 Rust 侧落地：单事务内完成版本校验、命令应用、
//! 版本递增、journal 与有界恢复快照。旧 artifact 文件树只作为派生缓存
//! 回写（canonical → shadow 方向），不再是新编辑的事实源。

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::schema::ensure_v2_schema;
use crate::CommandResult;

pub(crate) fn open_library_connection(root: &std::path::Path) -> CommandResult<Connection> {
    let conn = crate::db::open_connection(root)?;
    ensure_v2_schema(&conn)?;
    Ok(conn)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibraryItemRowV2 {
    pub id: String,
    pub modality: String,
    pub title: String,
    pub status: String,
    pub current_edit_version: i64,
    pub has_canonical_ds: bool,
    pub source_asset_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

fn row_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryItemRowV2> {
    let canonical: Option<String> = row.get("canonical_ds_json")?;
    Ok(LibraryItemRowV2 {
        id: row.get("id")?,
        modality: row.get("modality")?,
        title: row.get("title")?,
        status: row.get("status")?,
        current_edit_version: row.get("current_edit_version")?,
        has_canonical_ds: canonical.is_some(),
        source_asset_id: row.get("source_asset_id")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: row.get("deleted_at")?,
    })
}

const ITEM_COLUMNS: &str =
    "id, modality, title, status, current_edit_version, canonical_ds_json, source_asset_id, created_at, updated_at, deleted_at";

pub(crate) fn get_item(
    conn: &Connection,
    item_id: &str,
) -> CommandResult<Option<LibraryItemRowV2>> {
    conn.query_row(
        &format!("SELECT {ITEM_COLUMNS} FROM library_items_v2 WHERE id = ?1"),
        [item_id],
        row_from,
    )
    .optional()
    .map_err(|error| format!("library_v2_get_item:{error}"))
}

pub(crate) fn list_items(
    conn: &Connection,
    include_deleted: bool,
) -> CommandResult<Vec<LibraryItemRowV2>> {
    let filter = if include_deleted {
        ""
    } else {
        " WHERE deleted_at IS NULL"
    };
    let mut statement = conn
        .prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM library_items_v2{filter} ORDER BY updated_at DESC"
        ))
        .map_err(|error| format!("library_v2_list_items:{error}"))?;
    let rows = statement
        .query_map([], row_from)
        .map_err(|error| format!("library_v2_list_items:{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("library_v2_list_items:{error}"))?;
    Ok(rows)
}

pub(crate) struct UpsertItemInput<'a> {
    pub id: &'a str,
    pub modality: &'a str,
    pub title: &'a str,
    pub status: &'a str,
    pub source_asset_id: Option<&'a str>,
}

/// 幂等插入外壳行：已存在时**不覆盖**（迁移不得覆盖用户后来编辑的稿件，计划 §11.2 M2）。
pub(crate) fn upsert_item_shell(
    conn: &Connection,
    input: &UpsertItemInput<'_>,
) -> CommandResult<bool> {
    let now = Utc::now().to_rfc3339();
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO library_items_v2
             (id, modality, title, status, current_edit_version, canonical_ds_json, source_asset_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, NULL, ?5, ?6, ?6)",
            params![input.id, input.modality, input.title, input.status, input.source_asset_id, now],
        )
        .map_err(|error| format!("library_v2_upsert_shell:{error}"))?;
    Ok(inserted > 0)
}

/// 仅当行还没有权威稿时填充（迁移填充语义：永不覆盖已编辑稿件）。
pub(crate) fn seed_canonical_ds(
    conn: &Connection,
    item_id: &str,
    ds_json: &str,
    status: &str,
) -> CommandResult<bool> {
    let now = Utc::now().to_rfc3339();
    let updated = conn
        .execute(
            "UPDATE library_items_v2
             SET canonical_ds_json = ?2, status = ?3, updated_at = ?4
             WHERE id = ?1 AND canonical_ds_json IS NULL",
            params![item_id, ds_json, status, now],
        )
        .map_err(|error| format!("library_v2_seed_ds:{error}"))?;
    Ok(updated > 0)
}

pub(crate) fn get_canonical_ds(
    conn: &Connection,
    item_id: &str,
) -> CommandResult<Option<(Value, i64)>> {
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT canonical_ds_json, current_edit_version FROM library_items_v2 WHERE id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("library_v2_get_ds:{error}"))?;
    match row {
        None => Ok(None),
        Some((json, version)) => {
            let ds = serde_json::from_str(&json)
                .map_err(|error| format!("library_v2_ds_corrupt:{error}"))?;
            Ok(Some((ds, version)))
        }
    }
}

pub(crate) fn rename_item(conn: &Connection, item_id: &str, title: &str) -> CommandResult<bool> {
    let now = Utc::now().to_rfc3339();
    let updated = conn
        .execute(
            "UPDATE library_items_v2 SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![item_id, title, now],
        )
        .map_err(|error| format!("library_v2_rename:{error}"))?;
    Ok(updated > 0)
}

pub(crate) fn set_item_status(conn: &Connection, item_id: &str, status: &str) -> CommandResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE library_items_v2 SET status = ?2, updated_at = ?3 WHERE id = ?1",
        params![item_id, status, now],
    )
    .map_err(|error| format!("library_v2_set_status:{error}"))?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApplyEditorCommandsInput {
    #[serde(rename = "itemId")]
    pub item_id: String,
    #[serde(rename = "baseVersion")]
    pub base_version: i64,
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    /// EditorCommandV1 编译后的补丁批次（AuthoringPatchV2），整批成功或整批回滚。
    pub commands: Vec<Value>,
    /// 可选标题保存（工作区 header 原位编辑）；与命令同事务提交。
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyEditorCommandsResult {
    pub item_id: String,
    pub edit_version: i64,
    pub applied_count: usize,
    pub recovery_snapshot_saved: bool,
    /// 幂等重放命中：本次调用没有重新应用任何命令。
    pub replayed: bool,
}

const RECOVERY_SNAPSHOT_EVERY: i64 = 20;

/// 计划 §8.7：用户编辑过的节点必须带 `provenanceStatus = user_edited`，
/// 迟到的 cloud 候选不得自动覆盖。旧链的 mark_user_edited 只在节点已有
/// provenance 字段时更新；DB 链在事务层补齐可靠标记（无字段的新节点也标记）。
fn mark_command_target_user_edited(document: &mut Value, command: &Value) {
    let Some(node_id) = command.get("nodeId").and_then(Value::as_str) else {
        return;
    };
    if command.get("preserveProvenance").and_then(Value::as_bool) == Some(true)
        || command.get("restoreProvenanceStatus").is_some()
    {
        return;
    }
    if let Some(node) = find_object_by_id_mut(document, node_id) {
        node.insert(
            "provenanceStatus".to_string(),
            Value::String("user_edited".to_string()),
        );
    }
}

fn find_object_by_id_mut<'a>(
    value: &'a mut Value,
    id: &str,
) -> Option<&'a mut serde_json::Map<String, Value>> {
    if value.get("id").and_then(Value::as_str) == Some(id) {
        return value.as_object_mut();
    }
    match value {
        Value::Array(items) => items
            .iter_mut()
            .find_map(|item| find_object_by_id_mut(item, id)),
        Value::Object(map) => map
            .values_mut()
            .find_map(|item| find_object_by_id_mut(item, id)),
        _ => None,
    }
}

/// 编辑事务（计划 §9.5 / §3 接口契约）：版本校验 → 逐条应用 → 校验 →
/// 版本递增 → journal → 有界恢复快照，整批成功或整批回滚。
pub(crate) fn apply_editor_commands_tx(
    conn: &mut Connection,
    input: &ApplyEditorCommandsInput,
    apply_patch: &dyn Fn(&mut Value, &Value) -> CommandResult<()>,
    validate_ds: &dyn Fn(&Value) -> CommandResult<()>,
) -> CommandResult<ApplyEditorCommandsResult> {
    if input.commands.is_empty() && input.title.is_none() {
        return Err("EDITOR_COMMANDS_REQUIRED".to_string());
    }

    // 幂等重放：同一 request_id 直接返回上次结果，不重复应用。
    if let Some(request_id) = input.request_id.as_deref() {
        let replay: Option<i64> = conn
            .query_row(
                "SELECT base_version + 1 FROM editor_journal_v1 WHERE request_id = ?1",
                [request_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("library_v2_journal_lookup:{error}"))?;
        if let Some(version) = replay {
            return Ok(ApplyEditorCommandsResult {
                item_id: input.item_id.clone(),
                edit_version: version,
                applied_count: 0,
                recovery_snapshot_saved: false,
                replayed: true,
            });
        }
    }

    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("library_v2_tx:{error}"))?;

    let row: Option<(Option<String>, i64)> = transaction
        .query_row(
            "SELECT canonical_ds_json, current_edit_version FROM library_items_v2 WHERE id = ?1",
            [&input.item_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("library_v2_tx_load:{error}"))?;
    let (ds_json, current_version) =
        row.ok_or_else(|| format!("ITEM_NOT_FOUND:{}", input.item_id))?;
    if current_version != input.base_version {
        return Err(format!(
            "EDIT_VERSION_CONFLICT:current={current_version}:base={}",
            input.base_version
        ));
    }
    let ds_json = ds_json.ok_or_else(|| format!("ITEM_DS_NOT_SEEDED:{}", input.item_id))?;
    let mut ds: Value =
        serde_json::from_str(&ds_json).map_err(|error| format!("library_v2_ds_corrupt:{error}"))?;

    for command in &input.commands {
        apply_patch(&mut ds, command)?;
        mark_command_target_user_edited(&mut ds, command);
    }
    if let Some(title) = input.title.as_deref() {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err("EDITOR_TITLE_REQUIRED".to_string());
        }
        if let Some(exam) = ds.get_mut("exam").and_then(Value::as_object_mut) {
            exam.insert("title".to_string(), Value::String(trimmed.to_string()));
        }
    }
    validate_ds(&ds)?;

    let next_version = current_version + 1;
    let now = Utc::now().to_rfc3339();
    transaction
        .execute(
            "UPDATE library_items_v2
             SET canonical_ds_json = ?2, current_edit_version = ?3, updated_at = ?4
             WHERE id = ?1 AND current_edit_version = ?5",
            params![
                &input.item_id,
                ds.to_string(),
                next_version,
                now,
                current_version
            ],
        )
        .map_err(|error| format!("library_v2_tx_update:{error}"))?;
    if let Some(title) = input.title.as_deref() {
        transaction
            .execute(
                "UPDATE library_items_v2 SET title = ?2, updated_at = ?3 WHERE id = ?1",
                params![&input.item_id, title.trim(), now],
            )
            .map_err(|error| format!("library_v2_tx_title:{error}"))?;
    }
    transaction
        .execute(
            "INSERT INTO editor_journal_v1 (library_item_id, base_version, request_id, command_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                &input.item_id,
                input.base_version,
                input.request_id,
                serde_json::to_string(&input.commands)
                    .map_err(|error| format!("library_v2_journal_encode:{error}"))?,
                now
            ],
        )
        .map_err(|error| format!("library_v2_journal_insert:{error}"))?;

    // 有界恢复：每 RECOVERY_SNAPSHOT_EVERY 次保存刷新一次 last-good 快照。
    let recovery_saved = if next_version % RECOVERY_SNAPSHOT_EVERY == 0 {
        transaction
            .execute(
                "INSERT INTO library_item_recovery_v1 (library_item_id, edit_version, snapshot_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(library_item_id) DO UPDATE SET
                    edit_version = excluded.edit_version,
                    snapshot_json = excluded.snapshot_json,
                    updated_at = excluded.updated_at",
                params![&input.item_id, next_version, ds.to_string(), now],
            )
            .map_err(|error| format!("library_v2_recovery:{error}"))?;
        true
    } else {
        false
    };

    transaction
        .commit()
        .map_err(|error| format!("library_v2_tx_commit:{error}"))?;

    Ok(ApplyEditorCommandsResult {
        item_id: input.item_id.clone(),
        edit_version: next_version,
        applied_count: input.commands.len(),
        recovery_snapshot_saved: recovery_saved,
        replayed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::schema::ensure_v2_schema;

    fn memory_repo() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_v2_schema(&conn).unwrap();
        conn
    }

    fn sample_ds(title: &str) -> Value {
        serde_json::json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "exam": { "title": title },
            "passage": { "contentDoc": { "nodes": [
                { "id": "p1", "type": "paragraph", "children": [
                    { "id": "t1", "type": "text", "text": "hello world" }
                ]}
            ]}},
            "taskGroups": [],
            "answerSlots": {},
            "answerKey": {}
        })
    }

    fn noop_validate(_: &Value) -> CommandResult<()> {
        Ok(())
    }

    fn patch_replace_text(node_id: &str, text: &str) -> Value {
        serde_json::json!({ "op": "replaceText", "nodeId": node_id, "from": 0, "to": 5, "text": text })
    }

    #[test]
    fn shell_upsert_is_idempotent_and_seed_never_overwrites() {
        let conn = memory_repo();
        let input = UpsertItemInput {
            id: "it-1",
            modality: "reading",
            title: "Paper 1",
            status: "ready",
            source_asset_id: None,
        };
        assert!(upsert_item_shell(&conn, &input).unwrap());
        assert!(
            !upsert_item_shell(&conn, &input).unwrap(),
            "重复插入必须幂等"
        );
        assert!(
            seed_canonical_ds(&conn, "it-1", &sample_ds("Paper 1").to_string(), "ready").unwrap()
        );
        // 已有 DS 时再次 seed 不覆盖（保护用户编辑）。
        assert!(
            !seed_canonical_ds(&conn, "it-1", &sample_ds("other").to_string(), "ready").unwrap()
        );
        let (ds, version) = get_canonical_ds(&conn, "it-1").unwrap().unwrap();
        assert_eq!(ds.pointer("/exam/title").unwrap(), "Paper 1");
        assert_eq!(version, 1);
    }

    #[test]
    fn editor_transaction_rejects_stale_base_version_without_writing() {
        let conn = memory_repo();
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: "it-1",
                modality: "reading",
                title: "t",
                status: "ready",
                source_asset_id: None,
            },
        )
        .unwrap();
        seed_canonical_ds(&conn, "it-1", &sample_ds("t").to_string(), "ready").unwrap();

        let input = ApplyEditorCommandsInput {
            item_id: "it-1".into(),
            base_version: 99,
            request_id: None,
            commands: vec![patch_replace_text("t1", "HELLO")],
            title: None,
        };
        let mut conflict_conn = conn;
        let error =
            apply_editor_commands_tx(&mut conflict_conn, &input, &|_, _| Ok(()), &noop_validate)
                .err()
                .expect("stale base version must fail");
        assert!(error.starts_with("EDIT_VERSION_CONFLICT"), "{error}");
        let (_, version) = get_canonical_ds(&conflict_conn, "it-1").unwrap().unwrap();
        assert_eq!(version, 1, "冲突时不得推进版本");
    }

    #[test]
    fn editor_transaction_applies_commands_and_journals() {
        let conn = memory_repo();
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: "it-1",
                modality: "reading",
                title: "t",
                status: "ready",
                source_asset_id: None,
            },
        )
        .unwrap();
        seed_canonical_ds(&conn, "it-1", &sample_ds("t").to_string(), "ready").unwrap();

        let mut tx_conn = conn;
        let input = ApplyEditorCommandsInput {
            item_id: "it-1".into(),
            base_version: 1,
            request_id: Some("req-1".into()),
            commands: vec![patch_replace_text("t1", "HELLO")],
            title: Some("新标题".into()),
        };
        // 复用真实的 patch 语义（replaceText by nodeId + user_edited 标记）。
        let result = apply_editor_commands_tx(
            &mut tx_conn,
            &input,
            &|document, patch| crate::authoring_v2_commands::apply_patch(document, patch),
            &noop_validate,
        )
        .unwrap();
        assert_eq!(result.edit_version, 2);
        assert_eq!(result.applied_count, 1);
        assert!(!result.replayed);

        let (ds, version) = get_canonical_ds(&tx_conn, "it-1").unwrap().unwrap();
        assert_eq!(version, 2);
        assert_eq!(ds.pointer("/exam/title").unwrap(), "新标题");
        let node = find_in_ds(&ds, "t1").expect("node must exist");
        assert_eq!(
            node.get("text").unwrap(),
            "HELLO world",
            "replaceText 用 HELLO 替换前 5 个字符"
        );
        assert_eq!(
            node.get("provenanceStatus").and_then(Value::as_str),
            Some("user_edited"),
            "patch 语义必须带 user_edited 保护（计划 §8.7）"
        );

        // 幂等重放：同一 request_id 返回同版本、不再应用。
        let replay =
            apply_editor_commands_tx(&mut tx_conn, &input, &|_, _| Ok(()), &noop_validate).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.edit_version, 2);
        let (_, version_after) = get_canonical_ds(&tx_conn, "it-1").unwrap().unwrap();
        assert_eq!(version_after, 2);
    }

    fn find_in_ds<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
        if value.get("id").and_then(Value::as_str) == Some(id) {
            return Some(value);
        }
        match value {
            Value::Array(items) => items.iter().find_map(|item| find_in_ds(item, id)),
            Value::Object(map) => map.values().find_map(|item| find_in_ds(item, id)),
            _ => None,
        }
    }
}
