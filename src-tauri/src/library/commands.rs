//! M1（原 P2-T03 / P3-T03 后端半边）：Workspace API。
//!
//! - `get_workspace_item`：读库 + 按需迁移填充；首稿未生成时 `ds=null`（计划 §3 契约）。
//! - `apply_editor_commands`：计划 §9.5 保存链的事务入口；提交后把 canonical DS
//!   同步为 artifact shadow（**派生方向** canonical → 缓存，供现有 export/publish 链读取），
//!   不再追加 revision 文件树（P2-T04 的「新编辑不再建 revision」）。
//! - `list_library_items`：题库列表改读 V2 仓库。

use std::path::Path;

use serde_json::{json, Value};

use super::migration::migrate_single_item;
use super::repository::{
    apply_editor_commands_tx, get_canonical_ds, get_item, list_items, open_library_connection,
    ApplyEditorCommandsInput,
};
use crate::authoring_v2_commands::{apply_patch, refresh_quality_report, validate_authoring};
use crate::util::job_dir;
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
    Ok(json!({
        "schemaVersion": "WorkspaceItemV1",
        "item": {
            "itemId": item.id,
            "title": item.title,
            "modality": item.modality,
            "status": item.status,
            "editVersion": item.current_edit_version,
            "hasCanonicalDs": item.has_canonical_ds,
            "updatedAt": item.updated_at
        },
        "ds": ds,
        "editVersion": item.current_edit_version,
        "issues": issues,
        "seededByOnDemandMigration": seeded
    }))
}

pub(crate) fn apply_editor_commands_core(
    root: &Path,
    input: ApplyEditorCommandsInput,
) -> CommandResult<Value> {
    let mut conn = open_library_connection(root)?;
    let item_id = input.item_id.clone();
    let result = apply_editor_commands_tx(&mut conn, &input, &apply_patch, &validate_authoring)?;

    // 提交后派生同步：canonical DS（权威）→ shadow 缓存（现有 export/publish 读这里）。
    // 顺带重算质量块（依赖物理 shadow，缺失时保留原质量块）并回写 DB 派生字段。
    let (ds, _version) = get_canonical_ds(&conn, &item_id)?.ok_or("ITEM_DS_CORRUPT")?;
    let mut derived = ds;
    let quality_result = refresh_quality_report(root, &item_id, &mut derived);
    if let Err(error) = &quality_result {
        eprintln!("[library] quality refresh skipped for {item_id}: {error}");
    }
    let state = derived
        .pointer("/quality/state")
        .and_then(Value::as_str)
        .unwrap_or("action_required")
        .to_string();
    let status = if state == "ready" {
        "ready"
    } else {
        "action_required"
    };
    persist_derived_state(&conn, &item_id, &derived, status)?;

    let shadow_path = job_dir(root, &item_id).join("authoring-ir-v2.shadow.json");
    if let Some(parent) = shadow_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&shadow_path, derived.to_string())
        .map_err(|error| format!("library_v2_shadow_sync:{error}"))?;

    Ok(json!({
        "schemaVersion": "ApplyEditorCommandsResultV1",
        "itemId": result.item_id,
        "editVersion": result.edit_version,
        "appliedCount": result.applied_count,
        "replayed": result.replayed,
        "recoverySnapshotSaved": result.recovery_snapshot_saved,
        "status": status
    }))
}

/// 只更新派生字段（canonical_ds_json 的质量块 + status），不推进版本、不记 journal。
fn persist_derived_state(
    conn: &rusqlite::Connection,
    item_id: &str,
    ds: &Value,
    status: &str,
) -> CommandResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE library_items_v2 SET canonical_ds_json = ?2, status = ?3, updated_at = ?4
         WHERE id = ?1",
        rusqlite::params![item_id, ds.to_string(), status, now],
    )
    .map_err(|error| format!("library_v2_derived_update:{error}"))?;
    Ok(())
}

pub(crate) fn list_library_items_core(root: &Path, include_deleted: bool) -> CommandResult<Value> {
    let conn = open_library_connection(root)?;
    let rows = list_items(&conn, include_deleted)?;
    serde_json::to_value(rows).map_err(|error| format!("library_v2_list_encode:{error}"))
}
