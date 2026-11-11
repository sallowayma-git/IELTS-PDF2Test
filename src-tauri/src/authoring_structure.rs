//! Atomic structural edits. Removed references and undo deltas share the same semantics as TS.
use crate::CommandResult;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

fn string<'a>(patch: &'a Map<String, Value>, key: &str) -> CommandResult<&'a str> {
    patch
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("AUTHORING_PATCH_FIELD_REQUIRED:{key}"))
}

fn task_index(document: &Value, task_id: &str) -> CommandResult<usize> {
    document["taskGroups"]
        .as_array()
        .and_then(|tasks| tasks.iter().position(|t| t["taskId"] == task_id))
        .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))
}

fn group_index(document: &Value, task: usize, group_id: &str) -> CommandResult<usize> {
    document["taskGroups"][task]["responseGroups"]
        .as_array()
        .and_then(|groups| groups.iter().position(|g| g["responseGroupId"] == group_id))
        .ok_or_else(|| format!("AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:{group_id}"))
}

fn contains_slot(value: &Value) -> bool {
    if value["type"] == "answer_slot" {
        return true;
    }
    match value {
        Value::Array(items) => items.iter().any(contains_slot),
        Value::Object(object) => object.values().any(contains_slot),
        _ => false,
    }
}

fn find_id<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
    if value.get("id").and_then(Value::as_str) == Some(id) {
        return Some(value);
    }
    match value {
        Value::Array(items) => items.iter().find_map(|v| find_id(v, id)),
        Value::Object(object) => object.values().find_map(|v| find_id(v, id)),
        _ => None,
    }
}

fn valid_new_subtree(document: &Value, node: &Value) -> bool {
    fn check(document: &Value, value: &Value, ids: &mut BTreeSet<String>) -> bool {
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            if !ids.insert(id.to_string()) || find_id(document, id).is_some() {
                return false;
            }
        }
        match value {
            Value::Array(items) => items.iter().all(|v| check(document, v, ids)),
            Value::Object(object) => object.values().all(|v| check(document, v, ids)),
            _ => true,
        }
    }
    check(document, node, &mut BTreeSet::new())
}

fn per_slot_prompt(document: &Value, group: &Value) -> bool {
    if group["assignment"] == "unordered_set" {
        return false;
    }
    let (Some(ids), Some(prompt)) = (group["slotIds"].as_array(), group["prompt"].as_array())
    else {
        return false;
    };
    let mut hosts = BTreeSet::new();
    !ids.is_empty()
        && ids.iter().all(|id| {
            let host = id
                .as_str()
                .and_then(|id| document["answerSlots"][id]["hostNodeId"].as_str());
            host.is_some_and(|host| {
                hosts.insert(host)
                    && prompt
                        .iter()
                        .any(|node| node["id"] == host && !contains_slot(node))
            })
        })
}

fn slot_ids(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// Remove only owned per-slot prompts, retaining shared prompts and passage paragraphs.
fn remove_slots(document: &mut Value, ids: &BTreeSet<String>) {
    let mut hosts = BTreeSet::new();
    for task in document["taskGroups"].as_array().into_iter().flatten() {
        for group in task["responseGroups"].as_array().into_iter().flatten() {
            if per_slot_prompt(document, group) {
                for id in slot_ids(&group["slotIds"]).intersection(ids) {
                    if let Some(host) = document["answerSlots"][id]["hostNodeId"].as_str() {
                        hosts.insert(host.to_string());
                    }
                }
            }
        }
    }
    for (id, slot) in document["answerSlots"].as_object().into_iter().flatten() {
        if !ids.contains(id) {
            if let Some(host) = slot["hostNodeId"].as_str() {
                hosts.remove(host);
            }
        }
    }
    // Embedded slot nodes and hotspots can appear inside nested tables, captions and diagrams.
    fn clean(
        value: &mut Value,
        ids: &BTreeSet<String>,
        hosts: &BTreeSet<String>,
        removed_nodes: &mut BTreeSet<String>,
    ) {
        match value {
            Value::Array(items) => {
                items.retain(|node| {
                    let remove = (node["type"] == "answer_slot"
                        && node["slotId"].as_str().is_some_and(|id| ids.contains(id)))
                        || node["id"].as_str().is_some_and(|id| hosts.contains(id));
                    if remove {
                        collect_node_ids(node, removed_nodes);
                    }
                    !remove
                });
                for item in items {
                    clean(item, ids, hosts, removed_nodes);
                }
            }
            Value::Object(object) => {
                if let Some(Value::Array(slots)) = object.get_mut("slotIds") {
                    slots.retain(|id| !id.as_str().is_some_and(|id| ids.contains(id)));
                }
                if let Some(Value::Array(hotspots)) = object.get_mut("hotspots") {
                    hotspots.retain(|h| !h["slotId"].as_str().is_some_and(|id| ids.contains(id)));
                }
                for child in object.values_mut() {
                    clean(child, ids, hosts, removed_nodes);
                }
            }
            _ => {}
        }
    }
    let mut removed_nodes = BTreeSet::new();
    clean(document, ids, &hosts, &mut removed_nodes);
    for task in document["taskGroups"].as_array_mut().into_iter().flatten() {
        for group in task["responseGroups"].as_array_mut().into_iter().flatten() {
            if let Some(slots) = group["slotIds"].as_array_mut() {
                slots.retain(|id| !id.as_str().is_some_and(|id| ids.contains(id)));
            }
        }
    }
    for key in ["answerSlots", "answerKey"] {
        if let Some(table) = document[key].as_object_mut() {
            for id in ids {
                table.remove(id);
            }
        }
    }
    clean_bindings(document, &removed_nodes);
}

fn collect_node_ids(value: &Value, ids: &mut BTreeSet<String>) {
    if let Some(id) = value.get("id").and_then(Value::as_str) {
        ids.insert(id.to_string());
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_node_ids(item, ids);
            }
        }
        Value::Object(object) => {
            for child in object.values() {
                collect_node_ids(child, ids);
            }
        }
        _ => {}
    }
}

fn clean_bindings(document: &mut Value, removed: &BTreeSet<String>) {
    if let Some(map) = document
        .pointer_mut("/passage/paragraphMap")
        .and_then(Value::as_object_mut)
    {
        map.retain(|_, id| !id.as_str().is_some_and(|id| removed.contains(id)));
    }
    for slot in document["answerSlots"]
        .as_object_mut()
        .into_iter()
        .flat_map(|slots| slots.values_mut())
    {
        if slot["hostNodeId"]
            .as_str()
            .is_some_and(|id| removed.contains(id))
        {
            slot.as_object_mut().unwrap().remove("hostNodeId");
        }
    }
}

fn refresh_numbers(document: &mut Value, task_id: &str) -> CommandResult<()> {
    let index = task_index(document, task_id)?;
    let task = &document["taskGroups"][index];
    let ids: BTreeSet<String> = task["responseGroups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|g| slot_ids(&g["slotIds"]))
        .collect();
    let numbers: BTreeSet<u64> = ids
        .iter()
        .filter_map(|id| document["answerSlots"][id]["questionNumber"].as_u64())
        .collect();
    let expression = json!({"kind":"set","values":numbers});
    if numbers.is_empty() {
        return Err("AUTHORING_PATCH_EXPRESSION_EMPTY".into());
    }
    crate::authoring_v2_commands::set_question_expression(
        document,
        json!({"taskId":task_id,"expression":expression})
            .as_object()
            .unwrap(),
    )?;
    // Listening part identities and original question numbers stay stable.
    Ok(())
}

fn refresh_listening(document: &mut Value, before: &Value) {
    fn task_numbers(document: &Value, ids: &BTreeSet<String>) -> BTreeSet<u64> {
        document["taskGroups"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["taskId"].as_str().is_some_and(|id| ids.contains(id)))
            .flat_map(|t| t["responseGroups"].as_array().into_iter().flatten())
            .flat_map(|g| slot_ids(&g["slotIds"]))
            .filter_map(|id| document["answerSlots"][&id]["questionNumber"].as_u64())
            .collect()
    }
    let tasks: BTreeSet<String> = document["taskGroups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["taskId"].as_str())
        .map(str::to_string)
        .collect();
    let mut parts = document
        .pointer("/listening/parts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for part in &mut parts {
        let old_ids = slot_ids(&part["taskIds"]);
        let old_numbers = task_numbers(before, &old_ids);
        let new_numbers = task_numbers(document, &old_ids);
        if let Some(ids) = part["taskIds"].as_array_mut() {
            ids.retain(|id| id.as_str().is_some_and(|id| tasks.contains(id)));
        }
        if let Some(numbers) = part["expectedQuestionNumbers"].as_array_mut() {
            numbers.retain(|n| {
                n.as_u64()
                    .is_none_or(|n| !old_numbers.contains(&n) || new_numbers.contains(&n))
            });
            for n in new_numbers.difference(&old_numbers) {
                if !numbers.iter().any(|v| v.as_u64() == Some(*n)) {
                    numbers.push(json!(n));
                }
            }
        }
    }
    // Empty Parts retain their audio/cues and missing-question declarations. Quality
    // gates report incomplete exams; deleting questions must not delete managed media.
    if let Some(target) = document.pointer_mut("/listening/parts") {
        *target = json!(parts);
    }
}

fn delete_task(document: &mut Value, task_id: &str) -> CommandResult<()> {
    let index = task_index(document, task_id)?;
    let task = document["taskGroups"][index].clone();
    let ids: BTreeSet<String> = task["responseGroups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|g| slot_ids(&g["slotIds"]))
        .collect();
    remove_slots(document, &ids);
    document["taskGroups"].as_array_mut().unwrap().remove(index);
    // A document-scoped bank may have been referenced by another group; keep it
    // with that group's owning task rather than leave a dangling reference.
    preserve_referenced_bank(document, &task);
    let mut removed_nodes = BTreeSet::new();
    collect_node_ids(&task, &mut removed_nodes);
    removed_nodes.retain(|id| find_id(document, id).is_none());
    clean_bindings(document, &removed_nodes);
    Ok(())
}

fn preserve_referenced_bank(document: &mut Value, deleted: &Value) {
    let Some(bank_id) = deleted
        .pointer("/optionBank/optionBankId")
        .and_then(Value::as_str)
    else {
        return;
    };
    for task in document["taskGroups"].as_array_mut().into_iter().flatten() {
        if task["optionBank"].is_null()
            && task["responseGroups"]
                .as_array()
                .is_some_and(|groups| groups.iter().any(|g| g["optionBankRef"] == bank_id))
        {
            task["optionBank"] = deleted["optionBank"].clone();
        }
    }
}

pub(crate) fn apply(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let mut next = document.clone();
    apply_inner(&mut next, patch)?;
    if patch.get("op").and_then(Value::as_str) != Some("restoreStructure") {
        refresh_listening(&mut next, document);
    }
    *document = next;
    Ok(())
}

fn apply_inner(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let op = string(patch, "op")?;
    if op == "restoreStructure" {
        return restore(document, patch);
    }
    let task_id = string(patch, "taskId")?;
    let ti = task_index(document, task_id)?;
    if op == "deleteTaskGroup" {
        return delete_task(document, task_id);
    }
    let response_id = string(patch, "responseGroupId")?;
    let gi = group_index(document, ti, response_id)?;
    let group = document["taskGroups"][ti]["responseGroups"][gi].clone();
    if op == "deleteResponseGroup" {
        if document["taskGroups"][ti]["responseGroups"]
            .as_array()
            .unwrap()
            .len()
            == 1
        {
            return delete_task(document, task_id);
        }
        let ids = slot_ids(&group["slotIds"]);
        remove_slots(document, &ids);
        document["taskGroups"][ti]["responseGroups"]
            .as_array_mut()
            .unwrap()
            .remove(gi);
        let bank_id = group["optionBankRef"].as_str();
        if let Some(bank_id) = bank_id {
            let referenced = document["taskGroups"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|t| t["responseGroups"].as_array().into_iter().flatten())
                .any(|g| g["optionBankRef"] == bank_id);
            if !referenced && document["taskGroups"][ti]["optionBank"]["optionBankId"] == bank_id {
                document["taskGroups"][ti]
                    .as_object_mut()
                    .unwrap()
                    .remove("optionBank");
            }
        }
        let mut removed = BTreeSet::new();
        collect_node_ids(&group, &mut removed);
        removed.retain(|id| find_id(document, id).is_none());
        clean_bindings(document, &removed);
        return refresh_numbers(document, task_id);
    }
    if op == "deleteAnswerSlot" {
        let id = string(patch, "slotId")?;
        if !slot_ids(&group["slotIds"]).contains(id) || document["answerSlots"][id].is_null() {
            return Err(format!("AUTHORING_PATCH_SLOT_NOT_FOUND:{id}"));
        }
        if group["slotIds"].as_array().unwrap().len() <= 1 {
            return Err("AUTHORING_PATCH_LAST_ANSWER_SLOT".into());
        }
        if let Some(node_id) = patch.get("nodeId").and_then(Value::as_str) {
            if !find_id(document, node_id)
                .is_some_and(|n| n["type"] == "answer_slot" && n["slotId"] == id)
            {
                return Err(format!("AUTHORING_PATCH_SLOT_NODE_NOT_FOUND:{id}"));
            }
        }
        remove_slots(document, &BTreeSet::from([id.to_string()]));
    } else if op == "insertAnswerSlot" {
        let mut slot = patch
            .get("slot")
            .and_then(Value::as_object)
            .cloned()
            .ok_or("AUTHORING_PATCH_SLOT_REQUIRED")?;
        let id = string(&slot, "slotId")?.to_string();
        if !document["answerSlots"][&id].is_null() {
            return Err(format!("AUTHORING_PATCH_SLOT_ALREADY_EXISTS:{id}"));
        }
        let value = patch
            .get("value")
            .ok_or("AUTHORING_PATCH_ANSWER_REQUIRED")?
            .clone();
        let index = patch
            .get("slotIndex")
            .and_then(Value::as_u64)
            .ok_or("AUTHORING_PATCH_FIELD_REQUIRED:slotIndex")? as usize;
        if index > group["slotIds"].as_array().unwrap().len() {
            return Err(format!("AUTHORING_PATCH_INDEX_INVALID:{index}"));
        }
        if let Some(node) = patch.get("node") {
            if node["type"] != "answer_slot" || node["slotId"] != id {
                return Err("AUTHORING_PATCH_SLOT_NODE_MISMATCH".into());
            }
            let node_id = node["id"]
                .as_str()
                .ok_or("AUTHORING_PATCH_SLOT_NODE_MISMATCH")?;
            if find_id(document, node_id).is_some() {
                return Err(format!("AUTHORING_PATCH_SLOT_ALREADY_EXISTS:{id}"));
            }
            crate::authoring_v2_commands::insert_node(document, patch)?;
        } else if patch.contains_key("promptNode") || per_slot_prompt(document, &group) {
            let prompt = patch.get("promptNode").cloned().unwrap_or_else(|| json!({"type":"paragraph","id":format!("{id}-prompt"),"children":[{"type":"text","id":format!("{id}-prompt-text"),"text":"新题干","sourceAnchors":[],"provenanceStatus":"manual"}],"sourceAnchors":[],"provenanceStatus":"manual"}));
            let host = prompt["id"]
                .as_str()
                .ok_or("AUTHORING_PATCH_PROMPT_INVALID")?;
            if !valid_new_subtree(document, &prompt) || contains_slot(&prompt) {
                return Err("AUTHORING_PATCH_PROMPT_INVALID".into());
            }
            slot.insert("hostNodeId".into(), json!(host));
            slot.insert("hostType".into(), json!("prompt"));
            let group = &mut document["taskGroups"][ti]["responseGroups"][gi];
            if group["prompt"].is_null() {
                group["prompt"] = json!([]);
            }
            // Keep shared introductory prompt nodes in their original positions.
            let prompt_index = group["prompt"].as_array().unwrap().len();
            group["prompt"]
                .as_array_mut()
                .unwrap()
                .insert(prompt_index, prompt);
        }
        document["taskGroups"][ti]["responseGroups"][gi]["slotIds"]
            .as_array_mut()
            .unwrap()
            .insert(index, json!(id));
        document["answerSlots"]
            .as_object_mut()
            .ok_or("AUTHORING_PATCH_ANSWER_SLOTS_MISSING")?
            .insert(id.clone(), json!(slot));
        document["answerKey"]
            .as_object_mut()
            .ok_or("AUTHORING_PATCH_ANSWER_KEY_MISSING")?
            .insert(id, value);
    }
    // Existing callers can provide a desired expression. It is still validated by
    // setQuestionExpression; new row-based actions derive it from surviving slots.
    if patch.contains_key("expression") {
        crate::authoring_v2_commands::set_question_expression(document, patch)?;
        Ok(())
    } else {
        refresh_numbers(document, task_id)
    }
}

fn restore(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    for change in patch
        .get("changes")
        .and_then(Value::as_array)
        .ok_or("AUTHORING_PATCH_RESTORE_INVALID")?
    {
        let path: Vec<&str> = change["path"]
            .as_array()
            .ok_or("AUTHORING_PATCH_RESTORE_INVALID")?
            .iter()
            .map(|v| v.as_str().ok_or("AUTHORING_PATCH_RESTORE_INVALID"))
            .collect::<Result<_, _>>()?;
        if path.is_empty()
            || path
                .iter()
                .any(|key| ["__proto__", "prototype", "constructor"].contains(key))
            || ![
                "taskGroups",
                "answerSlots",
                "answerKey",
                "passage",
                "listening",
            ]
            .contains(&path[0])
        {
            return Err("AUTHORING_PATCH_RESTORE_INVALID".into());
        }
        let mut parent = &mut *document;
        for key in &path[..path.len() - 1] {
            parent = if parent.is_array() {
                let index = key
                    .parse::<usize>()
                    .map_err(|_| "AUTHORING_PATCH_RESTORE_INVALID")?;
                parent.get_mut(index)
            } else {
                parent.get_mut(*key)
            }
            .ok_or("AUTHORING_PATCH_RESTORE_CONFLICT")?;
        }
        let key = path.last().unwrap();
        if let Some(array) = parent.as_array_mut() {
            let index = key
                .parse::<usize>()
                .map_err(|_| "AUTHORING_PATCH_RESTORE_INVALID")?;
            if index >= array.len() || index.to_string() != *key || change.get("after").is_none() {
                return Err("AUTHORING_PATCH_RESTORE_INVALID".into());
            }
            if array.get(index) != change.get("after") {
                return Err("AUTHORING_PATCH_RESTORE_CONFLICT".into());
            }
            let before = change
                .get("before")
                .ok_or("AUTHORING_PATCH_RESTORE_INVALID")?;
            array[index] = before.clone();
        } else {
            let object = parent
                .as_object_mut()
                .ok_or("AUTHORING_PATCH_RESTORE_INVALID")?;
            if object.get(*key) != change.get("after") {
                return Err("AUTHORING_PATCH_RESTORE_CONFLICT".into());
            }
            if let Some(before) = change.get("before") {
                object.insert(key.to_string(), before.clone());
            } else {
                object.remove(*key);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring_v2_commands::apply_patch;

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../contracts/fixtures/authoring-structure-parity.json"
        ))
        .unwrap()
    }

    #[test]
    fn rust_and_ts_share_forward_and_inverse_documents() {
        let fixture = fixture();
        let mut document = fixture["initial"].clone();
        let mut inverses = Vec::new();
        for (patch, step) in fixture["patches"]
            .as_array()
            .unwrap()
            .iter()
            .zip(fixture["steps"].as_array().unwrap())
        {
            let before = document.clone();
            apply_patch(&mut document, patch).unwrap();
            assert_eq!(document, step["after"], "parity for {}", patch["op"]);
            apply_patch(&mut document, &step["inverse"]).unwrap();
            assert_eq!(document, before, "undo for {}", patch["op"]);
            apply_patch(&mut document, patch).unwrap();
            assert_eq!(document, step["after"], "redo for {}", patch["op"]);
            inverses.push(step["inverse"].clone());
        }
        for inverse in inverses.iter().rev() {
            apply_patch(&mut document, inverse).unwrap();
        }
        assert_eq!(document, fixture["initial"]);
    }

    #[test]
    fn invalid_slot_deletions_and_stale_undo_are_atomic() {
        let fixture = fixture();
        let initial = fixture["initial"].clone();
        for (patch, code) in [
            (
                json!({"op":"deleteAnswerSlot","taskId":"task-2","responseGroupId":"group-2-0","slotId":"q5"}),
                "AUTHORING_PATCH_LAST_ANSWER_SLOT",
            ),
            (
                json!({"op":"deleteAnswerSlot","taskId":"task-1","responseGroupId":"group-1-0","slotId":"q5"}),
                "AUTHORING_PATCH_SLOT_NOT_FOUND:q5",
            ),
            (
                json!({"op":"deleteAnswerSlot","taskId":"task-1","responseGroupId":"group-1-0","slotId":"q1","nodeId":"prompt-1"}),
                "AUTHORING_PATCH_SLOT_NODE_NOT_FOUND:q1",
            ),
        ] {
            let mut document = initial.clone();
            assert_eq!(apply_patch(&mut document, &patch).unwrap_err(), code);
            assert_eq!(document, initial);
        }
        let mut document = fixture["steps"][0]["after"].clone();
        document["taskGroups"][0]["responseGroups"][0]["prompt"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"new","type":"paragraph","children":[]}));
        let before = document.clone();
        assert_eq!(
            apply_patch(&mut document, &fixture["steps"][0]["inverse"]).unwrap_err(),
            "AUTHORING_PATCH_RESTORE_CONFLICT"
        );
        assert_eq!(document, before);
    }

    #[test]
    fn manual_question_nodes_round_trip_and_nested_references_are_removed() {
        let mut document = fixture()["initial"].clone();
        let node = json!({"type":"paragraph","id":"manual","children":[],"provenanceStatus":"manual","sourceAnchors":[]});
        let insert = json!({"op":"insertNode","target":{"kind":"taskInstructions","taskId":"task-1"},"index":0,"node":node});
        apply_patch(&mut document, &insert).unwrap();
        let before = document.clone();
        apply_patch(&mut document, &json!({"op":"deleteNode","nodeId":"manual"})).unwrap();
        apply_patch(&mut document, &insert).unwrap();
        assert_eq!(document, before);
        document["taskGroups"][0]["stimulus"] = json!([{"type":"flowchart","id":"flow","steps":[{"type":"flow_step","id":"step","slotIds":["q1","q2"],"children":[{"type":"answer_slot","id":"embedded","slotId":"q1"}]}]}]);
        apply_patch(&mut document,&json!({"op":"deleteAnswerSlot","taskId":"task-1","responseGroupId":"group-1-0","slotId":"q1","nodeId":"embedded"})).unwrap();
        assert_eq!(
            document["taskGroups"][0]["stimulus"][0]["steps"][0]["slotIds"],
            json!(["q2"])
        );
        assert!(!document.to_string().contains("\"slotId\":\"q1\""));
    }

    #[test]
    fn save_reopen_and_compile_rows_and_groups_without_dangling_references() {
        use crate::library::{commands, repository};
        let root = std::env::temp_dir().join(format!("structure-save-{}", uuid::Uuid::new_v4()));
        crate::util::ensure_app_dirs(&root).unwrap();
        let conn = repository::open_library_connection(&root).unwrap();
        let mut initial = fixture()["initial"].clone();
        initial["passage"]["content"]
            .as_array_mut()
            .unwrap()
            .retain(|n| n["id"] != "diagram");
        repository::upsert_item_shell(
            &conn,
            &repository::UpsertItemInput {
                id: "item",
                modality: "reading",
                title: "Rows",
                status: "action_required",
                source_asset_id: None,
            },
        )
        .unwrap();
        repository::seed_canonical_ds(&conn, "item", &initial.to_string(), "action_required")
            .unwrap();
        for (version, patch) in [
            fixture()["patches"][0].clone(),
            json!({"op":"deleteTaskGroup","taskId":"task-2"}),
        ]
        .into_iter()
        .enumerate()
        {
            commands::apply_editor_commands_core(
                &root,
                repository::ApplyEditorCommandsInput {
                    item_id: "item".into(),
                    base_version: version as i64 + 1,
                    request_id: None,
                    title: None,
                    commands: vec![patch],
                },
            )
            .unwrap();
        }
        drop(conn);
        let workspace = commands::get_workspace_item_core(&root, "item").unwrap();
        let ds = &workspace["ds"];
        assert!(ds["answerSlots"].get("q1").is_none());
        assert!(ds["answerSlots"].get("q5").is_none());
        assert_eq!(ds["answerSlots"]["q6"]["questionNumber"], 6);
        assert!(ds["quality"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["code"] == "QUESTION_NUMBER_NONCONTIGUOUS" && i["severity"] == "warning"));
        let preview = crate::authoring_v2_commands::compile_authoring_preview_core(ds);
        assert_eq!(preview["ok"], true, "{preview}");
    }

    #[test]
    fn deletion_tombstones_survive_cloud_id_regeneration_and_undo() {
        let fixture = fixture();
        let initial = fixture["initial"].clone();
        let patch = json!({"op":"deleteTaskGroup","taskId":"task-2"});
        let mut current = initial.clone();
        apply_patch(&mut current, &patch).unwrap();
        let mut cloud = initial.clone();
        cloud["taskGroups"][1]["taskId"] = json!("regenerated-task");
        let journal = vec![
            json!({"command":{"commands":[patch]},"result":{"removedSlots":[{"slotId":"q5","questionNumber":5}],"removedTaskIds":["task-2"]}}),
        ];
        crate::cloud_adoption::merge_human_edits(&current, &mut cloud, &journal).unwrap();
        assert_eq!(cloud["answerSlots"], current["answerSlots"]);
        assert!(!cloud["taskGroups"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["taskId"] == "regenerated-task"));
        // Current state is final authority when a later undo restored the task.
        let mut cloud = initial.clone();
        crate::cloud_adoption::merge_human_edits(&initial, &mut cloud, &journal).unwrap();
        assert_eq!(cloud["answerSlots"], initial["answerSlots"]);
    }

    #[test]
    fn listening_deletion_preserves_media_missing_evidence_and_order() {
        let fixture = fixture();
        let mut document = fixture["listeningInitial"].clone();
        apply_patch(&mut document, &fixture["patches"][3]).unwrap();
        assert_eq!(document["listening"], fixture["listeningExpected"]);
    }

    #[test]
    fn per_slot_prompt_requires_unique_descendant_ids() {
        let fixture = fixture();
        let mut document = fixture["initial"].clone();
        let mut patch = fixture["patches"][1].clone();
        patch["promptNode"] = json!({"type":"paragraph","id":"new-prompt","children":[{"type":"text","id":"text-1","text":"Duplicate"}]});
        assert_eq!(
            apply_patch(&mut document, &patch).unwrap_err(),
            "AUTHORING_PATCH_PROMPT_INVALID"
        );
        assert_eq!(document, fixture["initial"]);
    }

    #[test]
    fn every_structural_patch_and_undo_obeys_the_backend_cloud_lock() {
        use crate::library::{commands, repository};
        let fixture = fixture();
        let root = std::env::temp_dir().join(format!("structure-lock-{}", uuid::Uuid::new_v4()));
        crate::util::ensure_app_dirs(&root).unwrap();
        let conn = repository::open_library_connection(&root).unwrap();
        repository::upsert_item_shell(
            &conn,
            &repository::UpsertItemInput {
                id: "item",
                modality: "reading",
                title: "Rows",
                status: "action_required",
                source_asset_id: None,
            },
        )
        .unwrap();
        repository::seed_canonical_ds(
            &conn,
            "item",
            &fixture["initial"].to_string(),
            "action_required",
        )
        .unwrap();
        crate::processing::queue::enqueue(&conn, "job", "item", "source", &Value::Null).unwrap();
        conn.execute("UPDATE processing_jobs_v2 SET stage='reconciling',cloud_status='running' WHERE id='job'",[]).unwrap();
        for patch in fixture["patches"]
            .as_array()
            .unwrap()
            .iter()
            .chain(std::iter::once(&fixture["steps"][0]["inverse"]))
        {
            assert!(
                !crate::cloud_repair::tools::MODEL_ALLOWED_OPS
                    .contains(&patch["op"].as_str().unwrap())
                    || patch["op"] == "insertAnswerSlot"
            );
            let error = commands::apply_editor_commands_core(
                &root,
                repository::ApplyEditorCommandsInput {
                    item_id: "item".into(),
                    base_version: 1,
                    request_id: None,
                    title: None,
                    commands: vec![patch.clone()],
                },
            )
            .unwrap_err();
            assert_eq!(error, "CLOUD_REVIEW_IN_PROGRESS:item");
        }
        assert_eq!(
            repository::get_canonical_ds(&conn, "item")
                .unwrap()
                .unwrap()
                .0,
            fixture["initial"]
        );
    }
}
