//! M1（原 P2-T03/P2-T04）：Canonical DS 仓库与事务编辑。
//!
//! 计划 §9.5 的保存链在 Rust 侧落地：单事务内完成版本校验、命令应用、
//! 版本递增、journal 与有界恢复快照。旧 artifact 文件树（revision/shadow）
//! 只在导入与按需迁移时作为**来源**读取；数据库编辑不再回写派生文件，
//! canonical DS 是唯一权威稿。

use std::collections::BTreeSet;

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
            "SELECT canonical_ds_json, current_edit_version FROM library_items_v2 WHERE id = ?1 AND canonical_ds_json IS NOT NULL",
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

/// 只读权威稿版本号，**不解析** `canonical_ds_json`。
///
/// 为什么不复用 [`get_canonical_ds`]：事件发射路径每次阶段推进都要读一次版本号，
/// 为了一个整数去反序列化整份稿件是纯浪费；而且这个值要如实反映「权威稿现在是什么
/// 版本」，读不到就是读不到（`None`），不能拿一个解析失败当 0。
///
/// 权威稿尚未落库（`canonical_ds_json IS NULL`）时同样返回 `Some(version)`——
/// 版本号本身是有效的，只是还没有稿；调用方（事件载荷）只关心版本。
pub(crate) fn current_edit_version(
    conn: &Connection,
    item_id: &str,
) -> CommandResult<Option<i64>> {
    conn.query_row(
        "SELECT current_edit_version FROM library_items_v2 WHERE id = ?1",
        [item_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| format!("library_v2_get_edit_version:{error}"))
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
    /// 本次实际触及的稳定目标（排序后）。云端修复据此如实回报「我改了哪些对象」，
    /// 而不是让模型自己声称改了什么。
    pub applied_targets: Vec<String>,
}

const RECOVERY_SNAPSHOT_EVERY: i64 = 20;

/// 一次编辑的**来源**。由调用入口决定，**绝不**从模型输出或前端请求体里取值——
/// 否则模型只要在 JSON 里写一个字段就能把自己的修改伪装成人工修改，从而绕过撤销
/// 归属与保护检查。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditOrigin {
    /// 人在题稿编辑器里做的修改。
    Human,
    /// 云端校核修复通过受限编辑工具做的修改。
    CloudRepair,
    /// 撤销自动修复产生的修改。
    Undo,
}

impl EditOrigin {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            EditOrigin::Human => "human",
            EditOrigin::CloudRepair => "cloud_repair",
            EditOrigin::Undo => "undo",
        }
    }

    /// 这次写入是否代表**人的意志**，因而要留下人工保护目标与 `user_edited` 标记。
    ///
    /// `Undo` 也算：用户撤销一条已应用的自动建议，等于明确表示"这个值我要自己定"，
    /// 若不留保护，下一轮云端修复就会把刚被撤掉的值再写回来。
    /// `CloudRepair` **不算**——机器写入绝不能把自己伪装成人工修改，否则下一轮修复
    /// 会把自己的产物当成人工保护对象，修复能力一轮比一轮弱。
    fn writes_human_protection(self) -> bool {
        matches!(self, EditOrigin::Human | EditOrigin::Undo)
    }

    /// 是否受「人工保护目标」约束。目前只有云端修复受约束；既有规则的自动填空路径
    /// 保留它原有的、更窄的前提复核（`auto_apply_eligible`），不在这里叠加，以免
    /// 改变已经上线并被测试覆盖的行为。
    fn enforces_protection(self, repair_run_id: Option<&str>) -> bool {
        matches!(self, EditOrigin::CloudRepair) && repair_run_id.is_some()
    }
}

/// 一次编辑的**影响范围**：写目标、读依赖，以及被替换 / 删除子树里的全部对象。
///
/// 为什么必须显式算出来，而不是继续只看命令自带的 `nodeId`：
/// - `setAnswer` 真正改写的是**答案槽与答案项**，命令里的 nodeId 指向别处；
/// - `setResponseGroup` / `setOptionBank` 整组替换时，被替换掉的是一棵子树，
///   光看组 id 会漏掉组内的题干、选项、responseGroups 与槽位；
/// - 云端修复需要拿这个集合与**人工保护目标**求交集，漏算就等于放行了越权修改。
///
/// 索引一律用稳定 ID（节点 id / taskId / slotId / responseGroupId），**不用数组下标**：
/// 下标会随排序变化，用它当保护目标既会误伤也会漏保护。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditFootprint {
    pub targets: BTreeSet<String>,
}

/// 打开「收集子树内所有稳定标识」的开关后，会顺带收集的字段名。
///
/// 只列领域里真正被引用的身份字段；其余对象仍然会被递归遍历，只是不会把任意
/// 字符串字段误当成 ID。
const IDENTITY_KEYS: [&str; 5] = ["id", "taskId", "slotId", "responseGroupId", "assetId"];

fn push_identity(value: &Value, out: &mut BTreeSet<String>) {
    let Some(object) = value.as_object() else { return };
    for key in IDENTITY_KEYS {
        if let Some(id) = object.get(key).and_then(Value::as_str) {
            if !id.is_empty() {
                out.insert(id.to_string());
            }
        }
    }
}

/// 递归收集 `value` 自身及其后代的全部稳定标识。
fn collect_subtree_ids(value: &Value, out: &mut BTreeSet<String>) {
    push_identity(value, out);
    match value {
        Value::Array(items) => {
            for item in items {
                collect_subtree_ids(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_subtree_ids(item, out);
            }
        }
        _ => {}
    }
}

fn find_object_by_id<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
    let mut found = BTreeSet::new();
    push_identity(value, &mut found);
    if found.contains(id) {
        return Some(value);
    }
    match value {
        Value::Array(items) => items.iter().find_map(|item| find_object_by_id(item, id)),
        Value::Object(map) => map.values().find_map(|item| find_object_by_id(item, id)),
        _ => None,
    }
}

/// 找出 `id` 的所有祖先标识（不含自身）。改动一个子对象同样会改变其父对象的呈现
/// 与结构，因此祖先必须计入影响范围。
fn find_ancestor_ids(document: &Value, id: &str, stack: &mut Vec<String>) -> Option<Vec<String>> {
    let mut here = BTreeSet::new();
    push_identity(document, &mut here);
    if here.contains(id) {
        return Some(stack.clone());
    }
    let descendants: Vec<&Value> = match document {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map.values().collect(),
        _ => Vec::new(),
    };
    for key in IDENTITY_KEYS {
        if let Some(own) = document.get(key).and_then(Value::as_str) {
            stack.push(own.to_string());
        }
    }
    for child in descendants {
        if let Some(found) = find_ancestor_ids(child, id, stack) {
            return Some(found);
        }
    }
    for key in IDENTITY_KEYS {
        if document.get(key).and_then(Value::as_str).is_some() {
            stack.pop();
        }
    }
    None
}

fn extend_with_context(document: &Value, ids: &[String], out: &mut BTreeSet<String>) {
    for id in ids {
        if id.is_empty() {
            continue;
        }
        out.insert(id.clone());
        let mut stack = Vec::new();
        if let Some(ancestors) = find_ancestor_ids(document, id, &mut stack) {
            out.extend(ancestors);
        }
        if let Some(node) = find_object_by_id(document, id) {
            collect_subtree_ids(node, out);
        }
    }
}

fn strings_of(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(|text| vec![text.to_string()])
        .unwrap_or_default()
}

fn referenced_slot_ids(group: &Value) -> BTreeSet<String> {
    let mut slots = BTreeSet::new();
    if let Some(groups) = group.get("responseGroups").and_then(Value::as_array) {
        for response_group in groups {
            if let Some(ids) = response_group.get("slotIds").and_then(Value::as_array) {
                for id in ids.iter().filter_map(Value::as_str) {
                    slots.insert(id.to_string());
                }
            }
        }
    }
    slots
}

/// 一个题组**拥有**的全部对象：组子树 + 组内 responseGroups 引用的槽位。
///
/// 槽位存放在稿件顶层的 `answerSlots` 里，并不是题组的后代，所以「整体替换题组」
/// 的命令若只按子树计算，就会漏掉槽位与答案——正是这里需要显式补上的原因。
fn task_group_owned_ids(document: &Value, task_id: &str) -> BTreeSet<String> {
    let mut owned = BTreeSet::new();
    let Some(groups) = document.get("taskGroups").and_then(Value::as_array) else {
        return owned;
    };
    for group in groups {
        if group.get("taskId").and_then(Value::as_str) != Some(task_id) {
            continue;
        }
        collect_subtree_ids(group, &mut owned);
        owned.extend(referenced_slot_ids(group));
    }
    owned
}

/// 某个槽位归属的题组对象（题组子树 + 题组自身的祖先），**不含同组的其他槽位**。
///
/// 有意不包括兄弟槽位：用户改过 q15 的答案，不应该让云端连 q14 的答案都修不了。
/// 「不同目标的已保存人工改动不应导致整卷永远停止修复」说的就是这种边界。
fn task_groups_owning_slot(document: &Value, slot_id: &str) -> BTreeSet<String> {
    let mut owners = BTreeSet::new();
    let Some(groups) = document.get("taskGroups").and_then(Value::as_array) else {
        return owners;
    };
    for group in groups {
        if !referenced_slot_ids(group).contains(slot_id) {
            continue;
        }
        let Some(task_id) = group.get("taskId").and_then(Value::as_str) else {
            continue;
        };
        owners.insert(task_id.to_string());
        collect_subtree_ids(group, &mut owners);
        let mut stack = Vec::new();
        if let Some(ancestors) = find_ancestor_ids(document, task_id, &mut stack) {
            owners.extend(ancestors);
        }
        // 组内 responseGroups 也一并保护：换一个槽位的答案不该顺手改动结构。
        for response_group in group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = response_group
                .get("responseGroupId")
                .and_then(Value::as_str)
            {
                owners.insert(id.to_string());
            }
        }
    }
    owners
}

impl EditFootprint {
    /// 由**单条**命令推出影响范围。必须对着**编辑前**的稿件计算：`replaceContent` /
    /// `deleteNode` 覆盖的是那棵被替换掉的旧子树，编辑之后就找不到它了。
    pub(crate) fn for_command(document: &Value, command: &Value) -> Self {
        let mut targets = BTreeSet::new();
        let op = command.get("op").and_then(Value::as_str).unwrap_or_default();
        let group_roots = |task_id: &str, out: &mut BTreeSet<String>| {
            out.extend(task_group_owned_ids(document, task_id));
            extend_with_context(document, &[task_id.to_string()], out);
        };
        match op {
            // 单节点类：目标节点 + 其祖先 + 其子树（删除 / 整块替换会一并带走子树）。
            "replaceText" | "setNodeAttrs" => {
                extend_with_context(document, &strings_of(command.get("nodeId")), &mut targets);
            }
            "deleteNode" => {
                extend_with_context(document, &strings_of(command.get("nodeId")), &mut targets);
            }
            // 容器内容类：`target` 描述容器（node / passage / taskInstructions /
            // taskStimulus / responsePrompt / option），必须逐个 kind 解析——用 `nodeId`
            // 一把梭会把大部分命令算成"没有影响范围"，等于没有保护。
            "replaceContent" | "insertNode" | "moveNode" => {
                let mut roots = Vec::new();
                if let Some(target) = command.get("target").and_then(Value::as_object) {
                    let kind = target.get("kind").and_then(Value::as_str).unwrap_or_default();
                    match kind {
                        "node" => roots.extend(strings_of(target.get("nodeId"))),
                        "taskInstructions" | "taskStimulus" => {
                            roots.extend(strings_of(target.get("taskId")))
                        }
                        "responsePrompt" => roots.extend(strings_of(target.get("responseGroupId"))),
                        "option" => roots.extend(strings_of(target.get("optionId"))),
                        "passage" => {
                            // 整篇正文没有稳定 id：保守地把整棵 passage 子树计入，
                            // 这样"替换正文"永远不会悄悄覆盖用户改过的段落。
                            if let Some(passage) = document.get("passage") {
                                collect_subtree_ids(passage, &mut targets);
                            }
                        }
                        _ => {}
                    }
                }
                roots.extend(strings_of(command.get("parentId")));
                roots.extend(strings_of(command.get("nodeId")));
                if let Some(node) = command.get("node") {
                    collect_subtree_ids(node, &mut targets);
                }
                extend_with_context(document, &roots, &mut targets);
                // 插入 / 移动会改变所属题组的呈现范围。
                for task_id in task_group_ids_of_node(document, command.get("parentId"))
                    .into_iter()
                    .chain(task_group_ids_of_bundle(command))
                {
                    group_roots(&task_id, &mut targets);
                }
            }
            // 题组整体类：整组（题干、选项、responseGroups）连同组内槽位与答案。
            "setTaskType"
            | "setQuestionExpression"
            | "setResponseCardinality"
            | "setResponseGroup"
            | "setOptionBank" => {
                if let Some(task_id) = command.get("taskId").and_then(Value::as_str) {
                    group_roots(task_id, &mut targets);
                }
                if let Some(group_id) = command
                    .get("responseGroup")
                    .and_then(|group| group.get("responseGroupId"))
                    .and_then(Value::as_str)
                {
                    extend_with_context(document, &[group_id.to_string()], &mut targets);
                }
            }
            "insertAnswerSlot" | "deleteAnswerSlot" => {
                if let Some(task_id) = command.get("taskId").and_then(Value::as_str) {
                    group_roots(task_id, &mut targets);
                }
                let mut roots = strings_of(command.get("slotId"));
                roots.extend(strings_of(command.get("nodeId")));
                if let Some(slot_id) = command
                    .get("slot")
                    .and_then(|slot| slot.get("slotId"))
                    .and_then(Value::as_str)
                {
                    roots.push(slot_id.to_string());
                }
                extend_with_context(document, &roots, &mut targets);
                for slot_id in command
                    .get("slot")
                    .and_then(|slot| slot.get("slotId"))
                    .and_then(Value::as_str)
                    .into_iter()
                    .chain(command.get("slotId").and_then(Value::as_str))
                {
                    targets.extend(task_groups_owning_slot(document, slot_id));
                }
            }
            "setAnswer" => {
                // 答案存在两处：`answerSlots[slotId]` 与 `answerKey[slotId]`。两者**同键**，
                // 所以只要 slotId 进了目标集，两处就都被覆盖。
                //
                // 有意**不**把所属题组算进来：题组级目标是"整组替换"这种粗粒度操作的
                // 范围。若改一个答案就把整组标记为已触及，则用户改过 q15 的答案会让
                // 云端连 q14 的答案都改不动——那正是"不同目标的人工改动不应让整卷
                // 永远停止修复"要避免的过度封锁。
                let roots = strings_of(command.get("slotId"));
                extend_with_context(document, &roots, &mut targets);
            }
            "cropAsset" | "setHotspot" | "removeHotspot" => {
                let mut roots = strings_of(command.get("nodeId"));
                roots.extend(strings_of(command.get("assetId")));
                extend_with_context(document, &roots, &mut targets);
            }
            // `upsertTaskGroupBundle` 是整组原子操作：目标 = 新旧两个组的全部对象。
            "upsertTaskGroupBundle" => {
                let mut ids = task_group_ids_of_bundle(command);
                if let Some(task_id) = command.get("replacesTaskId").and_then(Value::as_str) {
                    ids.insert(task_id.to_string());
                }
                for task_id in ids {
                    group_roots(&task_id, &mut targets);
                }
                for slot in command
                    .get("answerSlots")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(slot_id) = slot.get("slotId").and_then(Value::as_str) {
                        extend_with_context(document, &[slot_id.to_string()], &mut targets);
                        targets.extend(task_groups_owning_slot(document, slot_id));
                    }
                }
            }
            // 未知 / 不允许的命令：影响范围取「整个稿件」这一最保守的边界，
            // 让它必然与任何人工保护目标相交，从而在授权层就被拒掉。
            _ => {
                collect_subtree_ids(document, &mut targets);
            }
        }
        EditFootprint { targets }
    }

    /// 合并一批命令的影响范围（整批的授权判定以此为准）。
    pub(crate) fn merge(document: &Value, commands: &[Value]) -> Self {
        let mut merged = EditFootprint::default();
        for command in commands {
            merged.targets.extend(EditFootprint::for_command(document, command).targets);
        }
        merged
    }

    /// 与人工保护目标相交时返回**第一个**冲突目标（返回具体目标，好让模型缩小范围）。
    pub(crate) fn first_conflict(&self, protected: &BTreeSet<String>) -> Option<String> {
        self.targets.iter().find(|id| protected.contains(*id)).cloned()
    }

    /// 对外的稳定形态（排序的数组，便于前端与日志阅读）。
    pub(crate) fn sorted_targets(&self) -> Vec<String> {
        self.targets.iter().cloned().collect()
    }
}

/// 题组 id → 该组内节点的反向索引（`insertNode` 需要知道父节点属于哪个组）。
fn task_group_ids_of_node(document: &Value, node_id: Option<&Value>) -> BTreeSet<String> {
    let mut owners = BTreeSet::new();
    let Some(node_id) = node_id.and_then(Value::as_str) else {
        return owners;
    };
    let Some(groups) = document.get("taskGroups").and_then(Value::as_array) else {
        return owners;
    };
    for group in groups {
        let mut ids = BTreeSet::new();
        collect_subtree_ids(group, &mut ids);
        if ids.contains(node_id) {
            if let Some(task_id) = group.get("taskId").and_then(Value::as_str) {
                owners.insert(task_id.to_string());
            }
        }
    }
    owners
}

fn task_group_ids_of_bundle(command: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    if let Some(task_id) = command
        .get("taskGroup")
        .and_then(|group| group.get("taskId"))
        .and_then(Value::as_str)
    {
        ids.insert(task_id.to_string());
    }
    ids
}

/// 人工保护目标：**人在编辑器里改过的对象**，云端修复不得覆盖。
///
/// 优先读 `protected_edits_json`（v5 起由人工编辑事务同事务维护，寿命不受 journal
/// 裁剪影响）。列为空时从稿件里的 `user_edited` 标记惰性导出，覆盖历史数据——
/// 自动写入的补丁带 `preserveProvenance: true`，不会盖出这个标记，因此它是可靠的。
pub(crate) fn human_protected_targets(
    conn: &Connection,
    item_id: &str,
    document: &Value,
) -> CommandResult<BTreeSet<String>> {
    let stored: Option<Option<String>> = conn
        .query_row(
            "SELECT protected_edits_json FROM library_items_v2 WHERE id = ?1",
            [item_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("library_v2_protected_read:{error}"))?;
    let raw = stored.flatten();
    if let Some(json) = raw {
        let parsed: Value =
            serde_json::from_str(&json).map_err(|error| format!("library_v2_protected_parse:{error}"))?;
        let mut targets = BTreeSet::new();
        if let Some(items) = parsed.get("targets").and_then(Value::as_array) {
            for id in items.iter().filter_map(Value::as_str) {
                targets.insert(id.to_string());
            }
        }
        return Ok(targets);
    }
    Ok(user_edited_targets_from_document(document))
}

/// 从稿件里的 `user_edited` 标记导出保护目标（历史数据的惰性迁移）。
fn user_edited_targets_from_document(document: &Value) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    fn walk(value: &Value, targets: &mut BTreeSet<String>) {
        if value.get("provenanceStatus").and_then(Value::as_str) == Some("user_edited") {
            let mut ids = BTreeSet::new();
            push_identity(value, &mut ids);
            targets.extend(ids);
        }
        match value {
            Value::Array(items) => {
                for item in items {
                    walk(item, targets);
                }
            }
            Value::Object(map) => {
                for item in map.values() {
                    walk(item, targets);
                }
            }
            _ => {}
        }
    }
    walk(document, &mut targets);
    targets
}

/// 把人工编辑触及的目标并进 `protected_edits_json`（同事务写入）。
fn merge_protected_edits(
    conn: &Connection,
    item_id: &str,
    added: &BTreeSet<String>,
) -> CommandResult<()> {
    if added.is_empty() {
        return Ok(());
    }
    let existing: Option<Option<String>> = conn
        .query_row(
            "SELECT protected_edits_json FROM library_items_v2 WHERE id = ?1",
            [item_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("library_v2_protected_read:{error}"))?;
    let mut targets = match existing.flatten() {
        Some(json) => {
            let parsed: Value = serde_json::from_str(&json)
                .map_err(|error| format!("library_v2_protected_parse:{error}"))?;
            parsed
                .get("targets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        }
        None => BTreeSet::new(),
    };
    targets.extend(added.iter().cloned());
    let payload = serde_json::json!({
        "targets": targets.iter().cloned().collect::<Vec<_>>(),
        "updatedAt": Utc::now().to_rfc3339(),
    });
    conn.execute(
        "UPDATE library_items_v2 SET protected_edits_json = ?2 WHERE id = ?1",
        params![item_id, payload.to_string()],
    )
    .map_err(|error| format!("library_v2_protected_write:{error}"))?;
    Ok(())
}

/// 受影响**根对象**的 before / after（撤销与审计的依据）。
///
/// 只记录命令直接点名的那几个对象，不记录整棵下钻的子树：后者等于每个回合都存一份
/// 近整卷的历史版本，既臃肿又与「有界恢复」的既定纪律冲突。单个目标过大时如实记为
/// `tooLarge` 并**放弃该目标的撤销能力**，而不是截断出一份假的 before。
const MAX_CHANGE_ENTRY_BYTES: usize = 64 * 1024;

fn capture_change_targets(commands: &[Value]) -> Vec<String> {
    let mut roots = BTreeSet::new();
    for command in commands {
        let op = command.get("op").and_then(Value::as_str).unwrap_or_default();
        for key in ["nodeId", "slotId", "taskId", "responseGroupId", "optionId", "replacesTaskId"] {
            if let Some(id) = command.get(key).and_then(Value::as_str) {
                roots.insert(id.to_string());
            }
        }
        // 答案条目必须单独记一份：它与槽位对象同键但没有任何身份字段，抓不到它
        // 就等于撤销后答案值仍然是模型写的那一份。
        if let Some(slot_id) = command.get("slotId").and_then(Value::as_str) {
            roots.insert(format!("{ANSWER_KEY_PREFIX}{slot_id}"));
        }
        if op == "upsertTaskGroupBundle" {
            if let Some(task_id) = command
                .get("taskGroup")
                .and_then(|group| group.get("taskId"))
                .and_then(Value::as_str)
            {
                roots.insert(task_id.to_string());
            }
            for slot in command
                .get("answerSlots")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(slot_id) = slot.get("slotId").and_then(Value::as_str) {
                    roots.insert(slot_id.to_string());
                }
            }
        }
    }
    roots.into_iter().collect()
}

/// `answerKey` 条目的目标键前缀。
///
/// 为什么需要它：`answerKey` 是一个**以 slotId 为键**的对象，条目本身没有任何身份
/// 字段，`find_object_by_id` 找不到它。而 `setAnswer` 改写的恰恰只有这个条目——不显式
/// 记一条，撤销就会"把槽位对象放回去了，但答案值还是模型写的那一份"，那比不能撤销更坏：
/// 界面说已撤销，内容却没回来。
const ANSWER_KEY_PREFIX: &str = "answerKey:";

fn read_change_value(document: &Value, key: &str) -> Value {
    if let Some(slot_id) = key.strip_prefix(ANSWER_KEY_PREFIX) {
        return document
            .get("answerKey")
            .and_then(|answers| answers.get(slot_id))
            .cloned()
            .unwrap_or(Value::Null);
    }
    find_object_by_id(document, key).cloned().unwrap_or(Value::Null)
}

fn write_change_value(document: &mut Value, key: &str, replacement: &Value) -> bool {
    if let Some(slot_id) = key.strip_prefix(ANSWER_KEY_PREFIX) {
        let Some(answers) = document.get_mut("answerKey").and_then(Value::as_object_mut) else {
            return false;
        };
        if replacement.is_null() {
            answers.remove(slot_id);
        } else {
            answers.insert(slot_id.to_string(), replacement.clone());
        }
        return true;
    }
    replace_object_by_id(document, key, replacement)
}

fn snapshot_change(document: &Value, targets: &[String]) -> Value {
    let mut before = serde_json::Map::new();
    for id in targets {
        let value = read_change_value(document, id);
        if serde_json::to_vec(&value).map(|bytes| bytes.len()).unwrap_or(usize::MAX)
            > MAX_CHANGE_ENTRY_BYTES
        {
            before.insert(id.clone(), serde_json::json!({ "tooLarge": true }));
        } else {
            before.insert(id.clone(), value);
        }
    }
    serde_json::json!({ "before": before })
}

fn with_after_snapshot(change: Value, document: &Value, targets: &[String]) -> Value {
    let Value::Object(mut map) = change else {
        return change;
    };
    let after = targets
        .iter()
        .map(|id| (id.clone(), read_change_value(document, id)))
        .collect::<serde_json::Map<_, _>>();
    map.insert("after".to_string(), Value::Object(after));
    Value::Object(map)
}

/// 在稿件里把 `id` 对应的对象**原位**替换成 `replacement`；`Value::Null` 表示删除该对象。
///
/// 「原位」很重要：撤销题组替换时必须把它放回 `taskGroups` 原来的位置，否则题号顺序
/// 会变，而题号顺序是学生端渲染与答案映射的输入。
fn replace_object_by_id(document: &mut Value, id: &str, replacement: &Value) -> bool {
    fn ident_of(value: &Value) -> Option<&str> {
        for key in IDENTITY_KEYS {
            if let Some(found) = value.get(key).and_then(Value::as_str) {
                return Some(found);
            }
        }
        None
    }
    if ident_of(document) == Some(id) {
        if replacement.is_null() {
            // 根对象无法"删除"，交给调用方按目标类型处理。
            return false;
        }
        *document = replacement.clone();
        return true;
    }
    match document {
        Value::Array(items) => {
            for index in 0..items.len() {
                if ident_of(&items[index]) == Some(id) {
                    if replacement.is_null() {
                        items.remove(index);
                    } else {
                        items[index] = replacement.clone();
                    }
                    return true;
                }
                if replace_object_by_id(&mut items[index], id, replacement) {
                    return true;
                }
            }
            false
        }
        Value::Object(map) => {
            let key = map
                .iter()
                .find(|(_, value)| ident_of(value) == Some(id))
                .map(|(key, _)| key.clone());
            if let Some(key) = key {
                if replacement.is_null() {
                    map.remove(&key);
                } else {
                    map.insert(key, replacement.clone());
                }
                return true;
            }
            let nested: Vec<String> = map.keys().cloned().collect();
            for key in nested {
                if let Some(child) = map.get_mut(&key) {
                    if replace_object_by_id(child, id, replacement) {
                        return true;
                    }
                }
            }
            false
        }
        _ => false,
    }
}

/// 撤销整轮自动修复的结果。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UndoRepairRunOutcome {
    pub item_id: String,
    pub edit_version: i64,
    /// 成功回滚到 before 的目标。
    pub restored: Vec<String>,
    /// 被跳过的目标（已被用户或其他写入改过、或 before 快照过大无法忠实回填）。
    pub skipped: Vec<String>,
}

/// 撤销整轮自动修复：把该 run 触及的目标回滚到各自的 before（任务书 §4.3）。
///
/// 安全性来自逐目标的三条判据，缺一不可：
///  1. 目标当前值必须**仍等于**本轮修复写下的最后 after —— 否则说明这之后有人改过它，
///     强行回填就是把别人的修改抹掉；
///  2. before 快照必须可用（未被 `tooLarge` 降级）—— 否则回填出来的不是真实旧值；
///  3. 只回填本轮触及的目标，**不整份恢复旧稿**：无关目标上的新修改必须原样保留。
///
/// 找不到该 run 的记录时返回 `EDIT_REPAIR_UNDO_UNAVAILABLE`：记录已被裁剪的轮次就是
/// 不可撤销的，如实报错，不给一个点了没用的假按钮。
pub(crate) fn undo_cloud_repair_run(
    conn: &mut Connection,
    item_id: &str,
    repair_run_id: &str,
    base_version: i64,
    prepare_ds: &dyn Fn(&mut Value) -> CommandResult<()>,
) -> CommandResult<UndoRepairRunOutcome> {
    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("library_v2_tx:{error}"))?;

    let rows: Vec<String> = {
        let mut statement = transaction
            .prepare(
                "SELECT COALESCE(change_json, 'null') FROM editor_journal_v1
                  WHERE library_item_id = ?1 AND repair_run_id = ?2 AND edit_origin = 'cloud_repair'
                  ORDER BY id ASC",
            )
            .map_err(|error| format!("library_v2_undo_prepare:{error}"))?;
        // 先收进局部变量再作为块的值返回：直接在尾部返回会让 `statement` 的借用活得
        // 比它自己更久（临时值在块尾才析构），编译不过。
        let rows = statement
            .query_map(params![item_id, repair_run_id], |row| row.get::<_, String>(0))
            .map_err(|error| format!("library_v2_undo_query:{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("library_v2_undo_rows:{error}"))?;
        rows
    };
    if rows.is_empty() {
        return Err("EDIT_REPAIR_UNDO_UNAVAILABLE".to_string());
    }

    // 每个目标取「首个 before」与「最后 after」。
    let mut first_before: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut last_after: serde_json::Map<String, Value> = serde_json::Map::new();
    for raw in &rows {
        let parsed: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
        if let Some(before) = parsed.get("before").and_then(Value::as_object) {
            for (id, value) in before {
                first_before.entry(id.clone()).or_insert_with(|| value.clone());
            }
        }
        if let Some(after) = parsed.get("after").and_then(Value::as_object) {
            for (id, value) in after {
                last_after.insert(id.clone(), value.clone());
            }
        }
    }

    let row: Option<(Option<String>, i64)> = transaction
        .query_row(
            "SELECT canonical_ds_json, current_edit_version FROM library_items_v2 WHERE id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("library_v2_tx_load:{error}"))?;
    let (ds_json, current_version) = row.ok_or_else(|| format!("ITEM_NOT_FOUND:{item_id}"))?;
    if current_version != base_version {
        return Err(format!(
            "EDIT_VERSION_CONFLICT:current={current_version}:base={base_version}"
        ));
    }
    let ds_json = ds_json.ok_or_else(|| format!("ITEM_DS_NOT_SEEDED:{item_id}"))?;
    let mut ds: Value =
        serde_json::from_str(&ds_json).map_err(|error| format!("library_v2_ds_corrupt:{error}"))?;

    let mut restored = Vec::new();
    let mut skipped = Vec::new();
    let mut change_before = serde_json::Map::new();
    let mut change_after = serde_json::Map::new();
    for (id, before) in &first_before {
        let current = read_change_value(&ds, id);
        let expected_after = last_after.get(id).cloned().unwrap_or(Value::Null);
        if before.get("tooLarge").and_then(Value::as_bool) == Some(true) {
            skipped.push(id.clone());
            continue;
        }
        if current != expected_after {
            // 目标已被本轮修复之外的写入改过：保留现状，绝不强行恢复旧值。
            skipped.push(id.clone());
            continue;
        }
        change_before.insert(id.clone(), current.clone());
        // 必须走 `write_change_value` 而不是 `replace_object_by_id`：`answerKey:<slotId>`
        // 条目的值是普通 JSON（选项数组），内部没有任何身份字段，按 id 找对象永远找不到。
        if write_change_value(&mut ds, id, before) {
            change_after.insert(id.clone(), before.clone());
            restored.push(id.clone());
        } else {
            // 稿件里已经没有这个对象（例如撤销一个新建对象）：什么都没改，如实跳过。
            change_before.remove(id);
            skipped.push(id.clone());
        }
    }
    if restored.is_empty() {
        return Err("EDIT_REPAIR_UNDO_NO_TARGETS".to_string());
    }

    prepare_ds(&mut ds)?;
    let next_version = current_version + 1;
    let now = Utc::now().to_rfc3339();
    cas_write_canonical(
        &transaction,
        item_id,
        current_version,
        next_version,
        &ds,
        edit_status_for(&ds),
        &now,
    )?;
    let change = serde_json::json!({
        "before": Value::Object(change_before),
        "after": Value::Object(change_after),
    });
    let result_summary = serde_json::json!({
        "status": "undone",
        "repairRunId": repair_run_id,
        "restored": restored,
        "skipped": skipped,
        "editVersion": next_version,
    });
    insert_journal_row(
        &transaction,
        &JournalWrite {
            item_id,
            base_version,
            request_id: None,
            payload: &serde_json::json!({"undoRepairRunId": repair_run_id}),
            origin: EditOrigin::Undo,
            repair_run_id: Some(repair_run_id),
            change: &change,
            result: &result_summary,
            now: &now,
        },
    )?;
    prune_journal(&transaction, item_id)?;
    transaction
        .commit()
        .map_err(|error| format!("library_v2_tx_commit:{error}"))?;

    Ok(UndoRepairRunOutcome {
        item_id: item_id.to_string(),
        edit_version: next_version,
        restored,
        skipped,
    })
}

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

/// 权威稿写入后的库状态：质量报告说 `ready` 就 `ready`，否则 `action_required`。
fn edit_status_for(ds: &Value) -> &'static str {
    if ds.pointer("/quality/state").and_then(Value::as_str) == Some("ready") {
        "ready"
    } else {
        "action_required"
    }
}

/// 权威稿的 CAS 写入：`current_edit_version` 必须仍是期望值。
fn cas_write_canonical(
    transaction: &rusqlite::Transaction<'_>,
    item_id: &str,
    expected_version: i64,
    next_version: i64,
    ds: &Value,
    status: &str,
    now: &str,
) -> CommandResult<()> {
    let updated = transaction
        .execute(
            "UPDATE library_items_v2
             SET canonical_ds_json = ?2, current_edit_version = ?3, updated_at = ?4, status = ?6
             WHERE id = ?1 AND current_edit_version = ?5",
            params![item_id, ds.to_string(), next_version, now, expected_version, status],
        )
        .map_err(|error| format!("library_v2_tx_update:{error}"))?;

    // SQLite reports a CAS miss as `Ok(0)`, not as a database error.  Treating that
    // as success lets the caller journal/export a write that never reached the
    // canonical document.  Read the current row only to make the conflict
    // diagnosable; the transaction is rolled back by the caller on this error.
    if updated == 0 {
        let current: Option<i64> = transaction
            .query_row(
                "SELECT current_edit_version FROM library_items_v2 WHERE id = ?1",
                [item_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("library_v2_tx_read_version:{error}"))?;
        return Err(match current {
            Some(current) => format!(
                "EDIT_VERSION_CONFLICT:current={current}:base={expected_version}"
            ),
            None => format!("ITEM_NOT_FOUND:{item_id}"),
        });
    }
    Ok(())
}

struct JournalWrite<'a> {
    item_id: &'a str,
    base_version: i64,
    request_id: Option<&'a str>,
    payload: &'a Value,
    origin: EditOrigin,
    repair_run_id: Option<&'a str>,
    change: &'a Value,
    result: &'a Value,
    now: &'a str,
}

fn insert_journal_row(
    transaction: &rusqlite::Transaction<'_>,
    write: &JournalWrite<'_>,
) -> CommandResult<()> {
    transaction
        .execute(
            "INSERT INTO editor_journal_v1
             (library_item_id, base_version, request_id, command_json, created_at,
              edit_origin, repair_run_id, change_json, result_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                write.item_id,
                write.base_version,
                write.request_id,
                write.payload.to_string(),
                write.now,
                write.origin.as_str(),
                write.repair_run_id,
                write.change.to_string(),
                write.result.to_string()
            ],
        )
        .map_err(|error| format!("library_v2_journal_insert:{error}"))?;
    Ok(())
}

/// 裁剪：人工记录保持原有的 200 条上限；自动修复记录**按 run 保留最近 5 轮**。
///
/// 既不能一律保留（journal 会无限增长），也不能一律按 200 条删：撤销所需的
/// before/after 一旦被普通裁剪提前删掉，撤销按钮就会变成假的。
fn prune_journal(transaction: &rusqlite::Transaction<'_>, item_id: &str) -> CommandResult<()> {
    transaction
        .execute(
            "DELETE FROM editor_journal_v1
              WHERE library_item_id = ?1
                AND (
                  (repair_run_id IS NULL AND id NOT IN (
                     SELECT id FROM editor_journal_v1
                      WHERE library_item_id = ?1 AND repair_run_id IS NULL
                      ORDER BY id DESC LIMIT 200))
                  OR
                  (repair_run_id IS NOT NULL AND repair_run_id NOT IN (
                     SELECT repair_run_id FROM editor_journal_v1
                      WHERE library_item_id = ?1 AND repair_run_id IS NOT NULL
                      GROUP BY repair_run_id ORDER BY MAX(id) DESC LIMIT 5))
                )",
            [item_id],
        )
        .map_err(|error| format!("library_v2_journal_prune:{error}"))?;
    Ok(())
}

/// 编辑事务（计划 §9.5 / §3 接口契约）：版本校验 → 逐条应用 → 校验 →
/// 版本递增 → journal → 有界恢复快照，整批成功或整批回滚。
pub(crate) fn apply_editor_commands_tx(
    conn: &mut Connection,
    input: &ApplyEditorCommandsInput,
    apply_patch: &dyn Fn(&mut Value, &Value) -> CommandResult<()>,
    prepare_ds: &dyn Fn(&mut Value) -> CommandResult<()>,
) -> CommandResult<ApplyEditorCommandsResult> {
    // 默认来源 = 人工：现有调用点都是编辑器 / 兼容迁移路径，人工语义不变。
    apply_editor_commands_tx_with(
        conn,
        input,
        EditOrigin::Human,
        None,
        apply_patch,
        prepare_ds,
        &|_, _| Ok(()),
    )
}

/// 同 [`apply_editor_commands_tx`]，但允许调用方在**同一个事务内**追加自己的写入。
///
/// `on_content_committed` 在内容写入（权威稿、版本号、编辑日志）全部完成后、`commit()`
/// 之前被调用，参数为该事务（`Transaction` 解引用为 `Connection`）与新的编辑版本号。
/// 它里面的任何写入与内容改动**同生共死**：返回 `Err` 则整个事务回滚，内容一字不改。
///
/// 为什么需要这个口子：调用方若在事务外用**另一条连接**写自己的状态，进程在「内容已提交、
/// 状态未提交」之间退出，就会留下内容与状态互相矛盾的局面。做成同事务后，该窗口在结构上
/// 不存在——只剩「都没写」与「都写了」两种可能。
///
/// **重放路径不调用该回调**：命中 `editor_journal_v1` 说明本事务此前已成功提交过，
/// 回调的写入当时已随内容一并落盘，再调一次反而会重复写入。
pub(crate) fn apply_editor_commands_tx_with(
    conn: &mut Connection,
    input: &ApplyEditorCommandsInput,
    origin: EditOrigin,
    repair_run_id: Option<&str>,
    apply_patch: &dyn Fn(&mut Value, &Value) -> CommandResult<()>,
    prepare_ds: &dyn Fn(&mut Value) -> CommandResult<()>,
    on_content_committed: &dyn Fn(&Connection, i64) -> CommandResult<()>,
) -> CommandResult<ApplyEditorCommandsResult> {
    if input.commands.is_empty() && input.title.is_none() {
        return Err("EDITOR_COMMANDS_REQUIRED".to_string());
    }

    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("library_v2_tx:{error}"))?;
    let payload = serde_json::json!({"commands": input.commands, "title": input.title});
    // Retries must identify the same item, base version and payload.
    if let Some(request_id) = input.request_id.as_deref() {
        let replay: Option<(String, i64, String)> = transaction
            .query_row(
                "SELECT library_item_id, base_version, command_json FROM editor_journal_v1 WHERE request_id = ?1",
                [request_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| format!("library_v2_journal_lookup:{error}"))?;
        if let Some((item_id, base_version, command_json)) = replay {
            let previous: Value = serde_json::from_str(&command_json).map_err(|error| error.to_string())?;
            if item_id != input.item_id || base_version != input.base_version || previous != payload {
                return Err("EDIT_REQUEST_ID_REUSED".to_string());
            }
            return Ok(ApplyEditorCommandsResult {
                item_id: input.item_id.clone(),
                edit_version: base_version + 1,
                applied_count: 0,
                recovery_snapshot_saved: false,
                replayed: true,
                applied_targets: Vec::new(),
            });
        }
    }

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

    // 影响范围必须对着**编辑前**的稿件算：`replaceContent` / `deleteNode` 覆盖的是
    // 那棵被替换掉的旧子树，编辑之后就找不到它了。
    let footprint = EditFootprint::merge(&ds, &input.commands);

    // 云端修复的授权边界。放在事务内、版本复核之后重新计算，而不是只信调用方的预检：
    // 预检与提交之间可能有人工保存落地，此刻的保护目标集合才是真正生效的那一份。
    // 冲突时回报**具体**目标，让模型能缩小修复范围，而不是笼统地"你没权限"。
    if origin.enforces_protection(repair_run_id) {
        let protected = human_protected_targets(&transaction, &input.item_id, &ds)?;
        if let Some(conflict) = footprint.first_conflict(&protected) {
            return Err(format!("EDIT_PROTECTED_TARGET:{conflict}"));
        }
    }

    // 可信来源需要的 before 快照：撤销整轮修复时，把每个目标的首个 before 与最后
    // after 合并。只记命令直接点名的根对象，不下钻整棵子树。
    let change_targets = capture_change_targets(&input.commands);
    let change = snapshot_change(&ds, &change_targets);

    for command in &input.commands {
        apply_patch(&mut ds, command)?;
        // 只有**人工**编辑才盖 `user_edited`：云端修复与撤销都不能把自己的改动
        // 伪装成人工改动，否则下一轮修复会把自己的产物当成人写的保护起来。
        if origin.writes_human_protection() {
            mark_command_target_user_edited(&mut ds, command);
        }
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
    prepare_ds(&mut ds)?;

    let next_version = current_version + 1;
    let now = Utc::now().to_rfc3339();
    cas_write_canonical(
        &transaction,
        &input.item_id,
        current_version,
        next_version,
        &ds,
        edit_status_for(&ds),
        &now,
    )?;
    if let Some(title) = input.title.as_deref() {
        transaction
            .execute(
                "UPDATE library_items_v2 SET title = ?2, updated_at = ?3 WHERE id = ?1",
                params![&input.item_id, title.trim(), now],
            )
            .map_err(|error| format!("library_v2_tx_title:{error}"))?;
    }
    // after 快照与 before 配对：撤销要的是「首个 before + 最后 after」，缺一半就无法
    // 判断某个目标是否仍是本轮修复写下的值（那正是「能不能安全回滚」的判据）。
    let change = with_after_snapshot(change, &ds, &change_targets);
    let result_summary = serde_json::json!({
        "status": "applied",
        "appliedCount": input.commands.len(),
        "editVersion": next_version,
        "targets": footprint.sorted_targets(),
    });
    insert_journal_row(
        &transaction,
        &JournalWrite {
            item_id: &input.item_id,
            base_version: input.base_version,
            request_id: input.request_id.as_deref(),
            payload: &payload,
            origin,
            repair_run_id,
            change: &change,
            result: &result_summary,
            now: &now,
        },
    )?;

    // 人工编辑的保护目标与内容同事务落库：这样「稿子改了、保护目标没改」的窗口
    // 在结构上不存在——而那个窗口会让云端修复有机会覆盖刚刚编辑过的对象。
    if origin.writes_human_protection() {
        merge_protected_edits(&transaction, &input.item_id, &footprint.targets)?;
    }

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

    prune_journal(&transaction, &input.item_id)?;

    // 调用方的附加写入（如识别决策状态）与内容改动同一事务：失败则整体回滚。
    on_content_committed(&transaction, next_version)?;

    transaction
        .commit()
        .map_err(|error| format!("library_v2_tx_commit:{error}"))?;

    Ok(ApplyEditorCommandsResult {
        item_id: input.item_id.clone(),
        edit_version: next_version,
        applied_count: input.commands.len(),
        recovery_snapshot_saved: recovery_saved,
        replayed: false,
        applied_targets: footprint.sorted_targets(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::schema::ensure_v2_schema;
    use serde_json::json;

    fn memory_repo() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
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

    fn noop_validate(_: &mut Value) -> CommandResult<()> {
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
    fn canonical_cas_does_not_report_success_when_expected_version_is_stale() {
        let mut conn = memory_repo();
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
        seed_canonical_ds(&conn, "it-1", &sample_ds("before").to_string(), "ready").unwrap();

        let transaction = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let error = cas_write_canonical(
            &transaction,
            "it-1",
            99,
            100,
            &sample_ds("after"),
            "ready",
            "2026-09-19T00:00:00Z",
        )
        .expect_err("CAS 未匹配时不得返回成功");
        assert!(
            error.starts_with("EDIT_VERSION_CONFLICT"),
            "应明确报告版本冲突，实际为：{error}"
        );
        transaction.rollback().unwrap();

        let (ds, version) = get_canonical_ds(&conn, "it-1").unwrap().unwrap();
        assert_eq!(ds.pointer("/exam/title").and_then(Value::as_str), Some("before"));
        assert_eq!(version, 1, "CAS 未匹配时不得写入新稿或推进版本");
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

    // ── 阶段二：可信机器编辑入口（来源 / 影响范围 / 保护 / 撤销）────────────

    fn grouped_ds() -> Value {
        serde_json::json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "exam": { "title": "t" },
            "taskGroups": [{
                "taskId": "task-1",
                "taskType": "sentence_completion",
                "requiredness": "required",
                "status": "included",
                "responseGroups": [{
                    "responseGroupId": "rg-1",
                    "kind": "text_entry",
                    "slotIds": ["slot-14", "slot-15"]
                }]
            }],
            "answerSlots": {
                "slot-14": { "slotId": "slot-14", "questionNumber": 14, "interaction": "text" },
                "slot-15": { "slotId": "slot-15", "questionNumber": 15, "interaction": "text" }
            },
            "answerKey": {
                "slot-14": { "kind": "text", "values": ["stencilling"] },
                "slot-15": { "kind": "text", "values": ["frame"] }
            },
            "quality": { "state": "action_required", "hardFailures": [], "issues": [] }
        })
    }

    fn grouped_item() -> Connection {
        let conn = memory_repo();
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: "it-1",
                modality: "reading",
                title: "t",
                status: "action_required",
                source_asset_id: None,
            },
        )
        .unwrap();
        seed_canonical_ds(&conn, "it-1", &grouped_ds().to_string(), "action_required").unwrap();
        conn
    }

    fn real_patch() -> impl Fn(&mut Value, &Value) -> CommandResult<()> {
        |document: &mut Value, patch: &Value| {
            crate::authoring_v2_commands::apply_patch(document, patch)
        }
    }

    fn set_answer(slot: &str, answer: &str) -> Value {
        serde_json::json!({
            "op": "setAnswer",
            "slotId": slot,
            "value": { "kind": "text", "values": [answer] }
        })
    }

    fn run_edit(
        conn: &mut Connection,
        commands: Vec<Value>,
        origin: EditOrigin,
        repair_run_id: Option<&str>,
        base_version: i64,
    ) -> CommandResult<ApplyEditorCommandsResult> {
        apply_editor_commands_tx_with(
            conn,
            &ApplyEditorCommandsInput {
                item_id: "it-1".into(),
                base_version,
                request_id: None,
                commands,
                title: None,
            },
            origin,
            repair_run_id,
            &real_patch(),
            &noop_validate,
            &|_, _| Ok(()),
        )
    }

    fn protected_of(conn: &Connection) -> BTreeSet<String> {
        let (ds, _) = get_canonical_ds(conn, "it-1").unwrap().unwrap();
        human_protected_targets(conn, "it-1", &ds).unwrap()
    }

    /// 影响范围必须覆盖命令**真正改写**的对象，而不是只看 `nodeId`。
    /// `setAnswer` 改的是答案槽与答案项（同键），命令里没有指向它们的 nodeId。
    #[test]
    fn edit_footprint_targets_the_slot_set_answer_actually_writes() {
        let ds = grouped_ds();
        let footprint = EditFootprint::for_command(&ds, &set_answer("slot-14", "x"));
        assert!(footprint.targets.contains("slot-14"), "答案槽必须被覆盖");
        assert!(
            !footprint.targets.contains("slot-15"),
            "不得牵连同组的其他槽位——否则一个槽位的人工改动会锁死整组"
        );
    }

    /// 题组整体替换必须覆盖组内题干、选项、responseGroups **以及槽位**。
    /// 槽位存放在顶层 `answerSlots`，不是题组子树的一部分，只按子树算会漏掉。
    #[test]
    fn edit_footprint_for_group_replace_covers_its_answer_slots() {
        let ds = grouped_ds();
        let footprint = EditFootprint::for_command(
            &ds,
            &json!({"op": "setOptionBank", "taskId": "task-1", "optionBank": {"options": []}}),
        );
        assert!(footprint.targets.contains("task-1"));
        assert!(footprint.targets.contains("rg-1"));
        assert!(
            footprint.targets.contains("slot-14") && footprint.targets.contains("slot-15"),
            "整组替换必须连带保护组内槽位与答案"
        );
    }

    /// 未知命令的影响范围取「整个稿件」这一最保守边界：它必然与任何人工保护目标
    /// 相交，从而在授权层被拒掉，而不是因为"没算出目标"被放行。
    #[test]
    fn edit_footprint_for_unknown_op_is_conservative() {
        let ds = grouped_ds();
        let footprint = EditFootprint::for_command(&ds, &json!({"op": "somethingNew"}));
        assert!(footprint.targets.contains("task-1"));
        assert!(footprint.targets.contains("slot-14"));
    }

    /// 人工编辑留下保护目标；云端修复碰到它就整批被拒，并回报**具体**目标。
    /// 同时验证不过度封锁：另一个槽位仍然可以修。
    #[test]
    fn human_edit_protects_its_targets_from_cloud_repair() {
        let mut conn = grouped_item();

        run_edit(
            &mut conn,
            vec![set_answer("slot-14", "user_value")],
            EditOrigin::Human,
            None,
            1,
        )
        .unwrap();
        let protected = protected_of(&conn);
        assert!(protected.contains("slot-14"), "人工改的槽位必须成为保护目标：{protected:?}");
        assert!(!protected.contains("slot-15"), "未触碰的槽位不应被保护");

        // 云端要改同一个槽位 ⇒ 拒绝，且拒绝理由是那个**具体**目标。
        let error = run_edit(
            &mut conn,
            vec![set_answer("slot-14", "cloud_value")],
            EditOrigin::CloudRepair,
            Some("run-1"),
            2,
        )
        .expect_err("人工保护目标不得被云端覆盖");
        assert!(error.starts_with("EDIT_PROTECTED_TARGET"), "{error}");
        assert!(error.contains("slot-14"), "必须回报具体目标：{error}");

        // 权威稿一字未改，版本未推进。
        let (ds, version) = get_canonical_ds(&conn, "it-1").unwrap().unwrap();
        assert_eq!(ds.pointer("/answerKey/slot-14/values/0").unwrap(), "user_value");
        assert_eq!(version, 2, "被拒的写入不得推进版本");

        // 另一个槽位仍可修：不同目标的人工改动不应让整卷停止修复。
        let outcome = run_edit(
            &mut conn,
            vec![set_answer("slot-15", "cloud_fixed")],
            EditOrigin::CloudRepair,
            Some("run-1"),
            2,
        )
        .unwrap();
        assert_eq!(outcome.edit_version, 3);
        let (ds, _) = get_canonical_ds(&conn, "it-1").unwrap().unwrap();
        assert_eq!(ds.pointer("/answerKey/slot-15/values/0").unwrap(), "cloud_fixed");
    }

    /// 云端修复不得给自己盖 `user_edited`，也不得写进人工保护目标。
    /// 否则下一轮修复会把自己的产物当成人写的保护起来，能力逐轮衰减。
    #[test]
    fn cloud_repair_never_marks_its_own_writes_as_human() {
        let mut conn = grouped_item();
        run_edit(
            &mut conn,
            vec![set_answer("slot-14", "cloud_value")],
            EditOrigin::CloudRepair,
            Some("run-7"),
            1,
        )
        .unwrap();

        assert!(
            protected_of(&conn).is_empty(),
            "机器写入不得进入人工保护目标集合"
        );
        let (ds, _) = get_canonical_ds(&conn, "it-1").unwrap().unwrap();
        let marked = ds
            .pointer("/answerSlots/slot-14/provenanceStatus")
            .and_then(Value::as_str);
        assert_ne!(marked, Some("user_edited"), "机器写入不得伪装成人工修改");

        // journal 如实记录来源与批次归属。
        let (origin, run): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT edit_origin, repair_run_id FROM editor_journal_v1 WHERE library_item_id = 'it-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(origin.as_deref(), Some("cloud_repair"));
        assert_eq!(run.as_deref(), Some("run-7"));
    }

    /// 撤销整轮修复：结构（槽位与答案）一并回滚，**无关目标上的后续人工修改保留**。
    #[test]
    fn undo_repair_run_restores_targets_and_keeps_unrelated_edits() {
        let mut conn = grouped_item();

        // 第一轮修复：q14 从空答案补成 stencilling（这里从既有值改起，效果等价）。
        run_edit(
            &mut conn,
            vec![set_answer("slot-14", "cloud_round_1")],
            EditOrigin::CloudRepair,
            Some("run-A"),
            1,
        )
        .unwrap();
        // 第二轮修复：继续改 q15。
        run_edit(
            &mut conn,
            vec![set_answer("slot-15", "cloud_round_2")],
            EditOrigin::CloudRepair,
            Some("run-A"),
            2,
        )
        .unwrap();
        // 用户随后把 slot-15 改成自己的值（与修复方向无关的目标）。
        run_edit(
            &mut conn,
            vec![set_answer("slot-15", "user_value")],
            EditOrigin::Human,
            None,
            3,
        )
        .unwrap();

        let outcome = undo_cloud_repair_run(&mut conn, "it-1", "run-A", 4, &noop_validate).unwrap();
        let (ds, version) = get_canonical_ds(&conn, "it-1").unwrap().unwrap();
        assert_eq!(outcome.edit_version, version);
        assert!(
            outcome.restored.contains(&"slot-14".to_string()),
            "仍等于修复值的 q14 必须回滚：{:?}",
            outcome.restored
        );
        assert!(
            outcome.skipped.contains(&"answerKey:slot-15".to_string()),
            "已被用户改过的 q15 答案必须跳过，不能把用户的修改抹掉：restored={:?} skipped={:?}",
            outcome.restored,
            outcome.skipped
        );
        assert_eq!(
            ds.pointer("/answerKey/slot-14/values/0").unwrap(),
            "stencilling",
            "回滚到本轮修复之前的旧值"
        );
        assert_eq!(
            ds.pointer("/answerKey/slot-15/values/0").unwrap(),
            "user_value",
            "无关目标的人工修改必须保留"
        );
    }

    /// 记录已被裁剪的轮次如实不可撤销，不给一个点了没用的假按钮。
    #[test]
    fn undo_reports_unavailable_when_the_run_has_no_journal_rows() {
        let mut conn = grouped_item();
        let error = undo_cloud_repair_run(&mut conn, "it-1", "run-missing", 1, &noop_validate)
            .expect_err("没有记录的轮次必须如实报不可撤销");
        assert_eq!(error, "EDIT_REPAIR_UNDO_UNAVAILABLE");
    }

    /// 撤销只在**目标仍是本轮写下的值**时才回滚；版本冲突不得静默重放。
    #[test]
    fn undo_refuses_when_the_base_version_is_stale() {
        let mut conn = grouped_item();
        run_edit(
            &mut conn,
            vec![set_answer("slot-14", "cloud_value")],
            EditOrigin::CloudRepair,
            Some("run-B"),
            1,
        )
        .unwrap();
        let error = undo_cloud_repair_run(&mut conn, "it-1", "run-B", 1, &noop_validate)
            .expect_err("过期的基线版本必须被拒，而不是按旧前提回滚");
        assert!(error.starts_with("EDIT_VERSION_CONFLICT"), "{error}");
    }

    /// 历史数据：`protected_edits_json` 为空时从稿件里的 `user_edited` 标记惰性导出。
    #[test]
    fn protected_targets_are_derived_from_legacy_user_edited_marks() {
        let conn = memory_repo();
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: "it-1",
                modality: "reading",
                title: "t",
                status: "action_required",
                source_asset_id: None,
            },
        )
        .unwrap();
        let mut ds = grouped_ds();
        ds["answerSlots"]["slot-14"]["provenanceStatus"] = json!("user_edited");
        seed_canonical_ds(&conn, "it-1", &ds.to_string(), "action_required").unwrap();

        let protected = protected_of(&conn);
        assert!(
            protected.contains("slot-14"),
            "旧稿的 user_edited 标记必须被导出为保护目标：{protected:?}"
        );
        assert!(!protected.contains("slot-15"));
    }

    /// 迟到的人工编辑不得被模型基于旧版本的补丁覆盖（提交时 CAS 挡住）。
    #[test]
    fn late_human_edit_blocks_a_cloud_patch_built_on_the_old_version() {
        let mut conn = grouped_item();
        run_edit(
            &mut conn,
            vec![set_answer("slot-15", "user_value")],
            EditOrigin::Human,
            None,
            1,
        )
        .unwrap();
        // 模型在版本 1 上做出的补丁：此刻稿子已到版本 2，必须被拒。
        let error = run_edit(
            &mut conn,
            vec![set_answer("slot-15", "cloud_late")],
            EditOrigin::CloudRepair,
            Some("run-late"),
            1,
        )
        .expect_err("基于旧版本的补丁必须被 CAS 拒绝，不得覆盖运行期间的人工修改");
        assert!(error.starts_with("EDIT_VERSION_CONFLICT"), "{error}");
        let (ds, version) = get_canonical_ds(&conn, "it-1").unwrap().unwrap();
        assert_eq!(version, 2);
        assert_eq!(ds.pointer("/answerKey/slot-15/values/0").unwrap(), "user_value");
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
