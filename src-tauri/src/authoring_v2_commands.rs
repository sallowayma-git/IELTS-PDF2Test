//! Phase 5 structured-authoring persistence.
//!
//! This module is deliberately separate from the V1 authoring commands.  A
//! V2 edit starts from the Phase 4 shadow artifact (or from an existing V2
//! revision), applies a small, explicit patch vocabulary, validates the full
//! typed authoring document, and appends an immutable revision.  The legacy
//! `authoring-ir.json` file is never rewritten here.

use crate::artifact_store::{
    append_revision, list_revision_records, read_revision, recover_current_revision,
    RevisionSourceV2,
};
use crate::schema::IeltsAuthoringIRV2;
use crate::util::{job_dir, read_json_opt, safe_job_dir};
use crate::CommandResult;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::path::Path;

const AUTHORING_V2_SHADOW_FILE: &str = "authoring-ir-v2.shadow.json";
const SESSION_SCHEMA_VERSION: &str = "AuthoringEditorSessionV1";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyAuthoringV2PatchesInput {
    pub job_id: String,
    pub base_revision: u64,
    pub patches: Vec<Value>,
}

pub(crate) fn get_authoring_v2_core(root: &Path, job_id: &str) -> CommandResult<Value> {
    safe_job_dir(root, job_id)?;
    build_editor_session(root, job_id)
}

pub(crate) fn apply_authoring_v2_patches_core(root: &Path, input: Value) -> CommandResult<Value> {
    let input: ApplyAuthoringV2PatchesInput = serde_json::from_value(input)
        .map_err(|error| format!("authoring_v2_invalid_patch_request:{error}"))?;
    safe_job_dir(root, &input.job_id)?;
    if input.patches.is_empty() {
        return Err("authoring_v2_patches_required".to_string());
    }
    if input.patches.len() > 200 {
        return Err("authoring_v2_patch_batch_too_large:max=200".to_string());
    }

    let (mut authoring, current_revision) = load_current_authoring(root, &input.job_id)?;
    if current_revision != input.base_revision {
        return Err(format!(
            "revision_conflict:current={current_revision}:base={}",
            input.base_revision
        ));
    }

    for patch in &input.patches {
        apply_patch(&mut authoring, patch)?;
    }
    mark_user_audit(&mut authoring, input.base_revision.saturating_add(1));
    validate_authoring(&authoring)?;

    let result = append_revision(
        root,
        &input.job_id,
        input.base_revision,
        RevisionSourceV2::User,
        &authoring,
        &input.patches,
    )?;

    Ok(json!({
        "schemaVersion": SESSION_SCHEMA_VERSION,
        "jobId": input.job_id,
        "authoring": authoring,
        "revision": result.current.revision,
        "source": "revision",
        "revisions": list_revision_records(root, &input.job_id)?,
        "v1FilesRemainReadable": true,
        "savedPatchCount": input.patches.len(),
    }))
}

fn build_editor_session(root: &Path, job_id: &str) -> CommandResult<Value> {
    let (authoring, revision) = load_current_authoring(root, job_id)?;
    let source = if revision > 0 { "revision" } else { "shadow" };
    Ok(json!({
        "schemaVersion": SESSION_SCHEMA_VERSION,
        "jobId": job_id,
        "authoring": authoring,
        "revision": revision,
        "source": source,
        "revisions": list_revision_records(root, job_id)?,
        "v1FilesRemainReadable": true,
    }))
}

fn load_current_authoring(root: &Path, job_id: &str) -> CommandResult<(Value, u64)> {
    let current = recover_current_revision(root, job_id)?;
    if current.revision > 0 {
        let value = read_revision(root, job_id, current.revision)?;
        validate_authoring(&value)?;
        return Ok((value, current.revision));
    }

    let path = job_dir(root, job_id).join(AUTHORING_V2_SHADOW_FILE);
    let value = read_json_opt(&path)?.ok_or_else(|| {
        format!(
            "AUTHORING_V2_NOT_AVAILABLE:shadow_missing:{}",
            path.display()
        )
    })?;
    validate_authoring(&value)?;
    Ok((value, 0))
}

fn validate_authoring(value: &Value) -> CommandResult<()> {
    if value.get("schemaVersion").and_then(Value::as_str) != Some("IeltsAuthoringIRV2") {
        return Err("AUTHORING_SCHEMA_INVALID:expected=IeltsAuthoringIRV2".to_string());
    }
    serde_json::from_value::<IeltsAuthoringIRV2>(value.clone())
        .map(|_| ())
        .map_err(|error| format!("AUTHORING_SCHEMA_INVALID:{error}"))
}

fn mark_user_audit(document: &mut Value, revision: u64) {
    if let Some(audit) = document.get_mut("audit").and_then(Value::as_object_mut) {
        audit.insert("revision".to_string(), json!(revision));
        audit.insert("source".to_string(), json!("user"));
        audit.insert("humanVerified".to_string(), json!(false));
        audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
    }
}

fn apply_patch(document: &mut Value, patch: &Value) -> CommandResult<()> {
    let object = patch
        .as_object()
        .ok_or_else(|| "authoring_v2_patch_must_be_object".to_string())?;
    let op = required_string(object, "op")?;
    match op {
        "replaceText" => replace_text(document, object),
        "setNodeAttrs" => set_node_attrs(document, object),
        "setTaskType" => set_task_type(document, object),
        "setQuestionExpression" => set_question_expression(document, object),
        "setResponseGroup" => set_response_group(document, object),
        "setAnswer" => set_answer(document, object),
        "bindSource" => bind_source(document, object),
        _ => Err(format!("AUTHORING_PATCH_UNSUPPORTED:{op}")),
    }
}

fn replace_text(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let node_id = required_string(patch, "nodeId")?;
    let from = required_u64(patch, "from")? as usize;
    let to = required_u64(patch, "to")? as usize;
    let text = required_string(patch, "text")?;
    if text.chars().count() > 100_000 {
        return Err("AUTHORING_PATCH_TEXT_TOO_LARGE:max=100000".to_string());
    }
    let node = find_object_mut_by_id(document, node_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"))?;
    if node.get("type").and_then(Value::as_str) != Some("text") {
        return Err(format!("AUTHORING_PATCH_TEXT_NODE_REQUIRED:{node_id}"));
    }
    let original = node
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("AUTHORING_PATCH_TEXT_MISSING:{node_id}"))?;
    let chars = original.chars().collect::<Vec<_>>();
    if from > to || to > chars.len() {
        return Err(format!(
            "AUTHORING_PATCH_TEXT_RANGE_INVALID:{node_id}:from={from}:to={to}:length={}",
            chars.len()
        ));
    }
    let mut next = chars[..from].iter().collect::<String>();
    next.push_str(text);
    next.extend(chars[to..].iter());
    node.insert("text".to_string(), Value::String(next));
    mark_user_edited(node);
    Ok(())
}

fn set_node_attrs(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let node_id = required_string(patch, "nodeId")?;
    let attrs = patch
        .get("attrs")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_ATTRS_REQUIRED".to_string())?;
    let node = find_object_mut_by_id(document, node_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"))?;
    for (key, value) in attrs {
        if !is_safe_node_attribute(key) {
            return Err(format!("AUTHORING_PATCH_ATTR_NOT_ALLOWED:{key}"));
        }
        node.insert(key.clone(), value.clone());
    }
    mark_user_edited(node);
    Ok(())
}

fn set_task_type(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let task_id = required_string(patch, "taskId")?;
    let task_type = required_string(patch, "taskType")?;
    if !is_supported_task_type(task_type) {
        return Err(format!("AUTHORING_PATCH_TASK_TYPE_INVALID:{task_type}"));
    }
    let task = find_object_by_field_mut(document, "taskId", task_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?;
    task.insert("taskType".to_string(), Value::String(task_type.to_string()));
    if let Some(signature) = task
        .get_mut("instructionSignature")
        .and_then(Value::as_object_mut)
    {
        signature.insert("taskType".to_string(), Value::String(task_type.to_string()));
    }
    mark_user_edited(task);
    Ok(())
}

fn set_question_expression(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let task_id = required_string(patch, "taskId")?;
    let expression = patch
        .get("expression")
        .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_REQUIRED".to_string())?;
    let expected_numbers = expand_question_expression(expression)?;
    let task = find_object_by_field_mut(document, "taskId", task_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?;
    task.insert("displayRange".to_string(), expression.clone());
    if let Some(signature) = task
        .get_mut("instructionSignature")
        .and_then(Value::as_object_mut)
    {
        signature.insert(
            "expectedQuestionNumbers".to_string(),
            json!(expected_numbers),
        );
        signature.insert(
            "expectedSlotCount".to_string(),
            json!(expected_numbers.len()),
        );
    }
    mark_user_edited(task);
    Ok(())
}

fn set_response_group(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let task_id = required_string(patch, "taskId")?;
    let response = patch
        .get("responseGroup")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_RESPONSE_GROUP_REQUIRED".to_string())?;
    let response_id = response
        .get("responseGroupId")
        .and_then(Value::as_str)
        .ok_or_else(|| "AUTHORING_PATCH_RESPONSE_GROUP_ID_REQUIRED".to_string())?;
    let task = find_object_by_field_mut(document, "taskId", task_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?;
    let groups = task
        .get_mut("responseGroups")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("AUTHORING_PATCH_RESPONSE_GROUPS_MISSING:{task_id}"))?;
    let target = groups
        .iter_mut()
        .find(|item| item.get("responseGroupId").and_then(Value::as_str) == Some(response_id));
    let Some(target) = target else {
        return Err(format!(
            "AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:{response_id}"
        ));
    };
    *target = Value::Object(response.clone());
    mark_user_edited(task);
    Ok(())
}

fn set_answer(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let slot_id = required_string(patch, "slotId")?;
    let value = patch
        .get("value")
        .ok_or_else(|| "AUTHORING_PATCH_ANSWER_REQUIRED".to_string())?;
    let slots = document
        .get("answerSlots")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_ANSWER_SLOTS_MISSING".to_string())?;
    if !slots.contains_key(slot_id) {
        return Err(format!("AUTHORING_PATCH_SLOT_NOT_FOUND:{slot_id}"));
    }
    let answer = document
        .get_mut("answerKey")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "AUTHORING_PATCH_ANSWER_KEY_MISSING".to_string())?;
    if !matches!(
        value.get("kind").and_then(Value::as_str),
        Some("text" | "option" | "unresolved")
    ) {
        return Err(format!("AUTHORING_PATCH_ANSWER_KIND_INVALID:{slot_id}"));
    }
    answer.insert(slot_id.to_string(), value.clone());
    Ok(())
}

fn bind_source(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let entity_id = required_string(patch, "entityId")?;
    let anchors = patch
        .get("anchors")
        .and_then(Value::as_array)
        .ok_or_else(|| "AUTHORING_PATCH_ANCHORS_REQUIRED".to_string())?;
    if anchors.iter().any(|anchor| !anchor.is_object()) {
        return Err("AUTHORING_PATCH_ANCHORS_INVALID".to_string());
    }
    let entity = find_object_by_any_id_mut(document, entity_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_ENTITY_NOT_FOUND:{entity_id}"))?;
    entity.insert("sourceAnchors".to_string(), Value::Array(anchors.clone()));
    mark_user_edited(entity);
    Ok(())
}

fn required_string<'a>(object: &'a Map<String, Value>, key: &str) -> CommandResult<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("AUTHORING_PATCH_FIELD_REQUIRED:{key}"))
}

fn required_u64(object: &Map<String, Value>, key: &str) -> CommandResult<u64> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("AUTHORING_PATCH_FIELD_REQUIRED:{key}"))
}

fn find_object_mut_by_id<'a>(value: &'a mut Value, id: &str) -> Option<&'a mut Map<String, Value>> {
    match value {
        Value::Object(object) => {
            if object.get("id").and_then(Value::as_str) == Some(id) {
                return Some(object);
            }
            for child in object.values_mut() {
                if let Some(found) = find_object_mut_by_id(child, id) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(found) = find_object_mut_by_id(item, id) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_object_by_field_mut<'a>(
    value: &'a mut Value,
    field: &str,
    expected: &str,
) -> Option<&'a mut Map<String, Value>> {
    match value {
        Value::Object(object) => {
            if object.get(field).and_then(Value::as_str) == Some(expected) {
                return Some(object);
            }
            for child in object.values_mut() {
                if let Some(found) = find_object_by_field_mut(child, field, expected) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(found) = find_object_by_field_mut(item, field, expected) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_object_by_any_id_mut<'a>(
    value: &'a mut Value,
    expected: &str,
) -> Option<&'a mut Map<String, Value>> {
    match value {
        Value::Object(object) => {
            let matches_identifier = ["id", "taskId", "responseGroupId", "slotId"]
                .iter()
                .any(|field| object.get(*field).and_then(Value::as_str) == Some(expected));
            if matches_identifier {
                return Some(object);
            }
            for child in object.values_mut() {
                if let Some(found) = find_object_by_any_id_mut(child, expected) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(found) = find_object_by_any_id_mut(item, expected) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn mark_user_edited(object: &mut Map<String, Value>) {
    if object.contains_key("provenanceStatus") {
        object.insert(
            "provenanceStatus".to_string(),
            Value::String("user_edited".to_string()),
        );
    }
}

fn is_safe_node_attribute(key: &str) -> bool {
    matches!(
        key,
        "provenanceStatus"
            | "align"
            | "indentLevel"
            | "level"
            | "altText"
            | "placeholder"
            | "displayLabel"
            | "inline"
            | "label"
            | "slotIds"
            | "display"
    )
}

fn is_supported_task_type(value: &str) -> bool {
    matches!(
        value,
        "single_choice"
            | "multiple_choice"
            | "true_false_not_given"
            | "yes_no_not_given"
            | "matching_information"
            | "matching_headings"
            | "matching_features"
            | "matching_sentence_endings"
            | "classification"
            | "sentence_completion"
            | "summary_completion"
            | "note_completion"
            | "table_completion"
            | "form_completion"
            | "flowchart_completion"
            | "diagram_label_completion"
            | "plan_map_label_completion"
            | "short_answer"
    )
}

fn expand_question_expression(expression: &Value) -> CommandResult<Vec<u64>> {
    let object = expression
        .as_object()
        .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_INVALID".to_string())?;
    match object.get("kind").and_then(Value::as_str) {
        Some("range") => {
            let start = object
                .get("start")
                .and_then(Value::as_u64)
                .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_START_REQUIRED".to_string())?;
            let end = object
                .get("end")
                .and_then(Value::as_u64)
                .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_END_REQUIRED".to_string())?;
            if start == 0 || end < start || end - start > 200 {
                return Err("AUTHORING_PATCH_EXPRESSION_RANGE_INVALID".to_string());
            }
            Ok((start..=end).collect())
        }
        Some("set") => parse_number_array(object.get("values")),
        Some("mixed") => {
            let values = object
                .get("values")
                .and_then(Value::as_array)
                .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_VALUES_REQUIRED".to_string())?;
            let mut numbers = Vec::new();
            for value in values {
                if let Some(number) = value.as_u64() {
                    if number == 0 {
                        return Err("AUTHORING_PATCH_EXPRESSION_NUMBER_INVALID".to_string());
                    }
                    numbers.push(number);
                } else {
                    numbers.extend(expand_question_expression(&json!({
                        "kind": "range",
                        "start": value.get("start").and_then(Value::as_u64),
                        "end": value.get("end").and_then(Value::as_u64),
                    }))?);
                }
            }
            if numbers.is_empty() || numbers.len() > 200 {
                return Err("AUTHORING_PATCH_EXPRESSION_VALUES_INVALID".to_string());
            }
            Ok(numbers)
        }
        _ => Err("AUTHORING_PATCH_EXPRESSION_KIND_INVALID".to_string()),
    }
}

fn parse_number_array(value: Option<&Value>) -> CommandResult<Vec<u64>> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_VALUES_REQUIRED".to_string())?;
    if values.is_empty() || values.len() > 200 {
        return Err("AUTHORING_PATCH_EXPRESSION_VALUES_INVALID".to_string());
    }
    let mut numbers = Vec::with_capacity(values.len());
    for value in values {
        let number = value
            .as_u64()
            .filter(|number| *number > 0)
            .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_NUMBER_INVALID".to_string())?;
        numbers.push(number);
    }
    Ok(numbers)
}

#[cfg(test)]
mod tests {
    use super::{apply_patch, expand_question_expression};
    use serde_json::json;

    fn text_document() -> serde_json::Value {
        json!({
            "type": "paragraph",
            "id": "paragraph-1",
            "provenanceStatus": "source",
            "children": [{
                "type": "text",
                "id": "text-1",
                "provenanceStatus": "source",
                "text": "Choose TWO letters."
            }]
        })
    }

    #[test]
    fn replace_text_uses_character_offsets_and_marks_provenance() {
        let mut document = text_document();
        apply_patch(
            &mut document,
            &json!({"op":"replaceText","nodeId":"text-1","from":7,"to":10,"text":"THREE"}),
        )
        .unwrap();
        assert_eq!(document["children"][0]["text"], "Choose THREE letters.");
        assert_eq!(document["children"][0]["provenanceStatus"], "user_edited");
    }

    #[test]
    fn expression_expansion_is_bounded_and_deterministic() {
        assert_eq!(
            expand_question_expression(
                &json!({"kind":"mixed","values":[14,{"start":15,"end":16}]})
            )
            .unwrap(),
            vec![14, 15, 16]
        );
        assert!(expand_question_expression(&json!({"kind":"range","start":9,"end":8})).is_err());
    }

    #[test]
    fn unsupported_node_attributes_are_rejected() {
        let mut document = text_document();
        assert!(apply_patch(
            &mut document,
            &json!({"op":"setNodeAttrs","nodeId":"text-1","attrs":{"id":"forbidden"}}),
        )
        .is_err());
    }
}
