//! Reception and dependency-unit adoption for editable cloud candidates.
//! Publication completeness is checked later; admission preserves references and human edits.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

fn question_numbers(value: &Value) -> BTreeSet<u32> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|number| u32::try_from(number).ok())
        .collect()
}

/// Return stable, user-readable reasons when the candidate must not be adopted.
pub(crate) fn adoption_rejection_reasons(candidate: &Value, local: &Value) -> Vec<String> {
    // Reception is not publication: partial results, missing answers and local disagreement
    // must reach repair. Only a missing normalized document makes adoption impossible.
    let _ = local;
    let authoring = candidate.get("authoring").unwrap_or(&Value::Null);
    if !authoring.is_object() || !authoring.get("taskGroups").is_some_and(Value::is_array) {
        return vec!["云端候选没有可供校核的题组结构".to_string()];
    }
    Vec::new()
}

#[derive(Debug)]
pub(crate) struct AdoptionCommit {
    pub edit_version: i64,
    pub preserved_group_ids: Vec<String>,
    pub applied_targets: Vec<String>,
    pub adopted_task_ids: Vec<String>,
    pub deferred_task_ids: Vec<String>,
}

fn group_question_numbers(document: &Value, group: &Value) -> BTreeSet<u32> {
    let slots = document.get("answerSlots").and_then(Value::as_object);
    let mut numbers = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        })
        .filter_map(|slot_id| {
            slots?
                .get(slot_id)
                .and_then(|slot| slot.get("questionNumber"))
                .and_then(Value::as_u64)
                .and_then(|number| u32::try_from(number).ok())
        })
        .collect::<BTreeSet<_>>();
    if !numbers.is_empty() {
        return numbers;
    }
    let Some(range) = group.get("displayRange") else {
        return numbers;
    };
    match range.get("kind").and_then(Value::as_str) {
        Some("range") => {
            if let (Some(start), Some(end)) = (
                range.get("start").and_then(Value::as_u64),
                range.get("end").and_then(Value::as_u64),
            ) {
                if start <= end && end.saturating_sub(start) <= 200 {
                    numbers.extend((start..=end).filter_map(|number| u32::try_from(number).ok()));
                }
            }
        }
        Some("set") => numbers.extend(question_numbers(
            range.get("values").unwrap_or(&Value::Null),
        )),
        Some("mixed") => {
            if let Some(values) = range.get("values").and_then(Value::as_array) {
                for value in values {
                    if let Some(number) = value.as_u64().and_then(|n| u32::try_from(n).ok()) {
                        numbers.insert(number);
                    } else if let (Some(start), Some(end)) = (
                        value.get("start").and_then(Value::as_u64),
                        value.get("end").and_then(Value::as_u64),
                    ) {
                        if start <= end && end.saturating_sub(start) <= 200 {
                            numbers.extend((start..=end).filter_map(|n| u32::try_from(n).ok()));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    numbers
}

/// Stable dependency unit shared by automatic adoption and explicit author selection.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComparisonUnit {
    pub unit_id: String,
    pub local_task_ids: Vec<String>,
    pub cloud_task_ids: Vec<String>,
}

fn groups(document: &Value) -> Vec<&Value> {
    document
        .get("taskGroups")
        .and_then(Value::as_array)
        .map(|groups| groups.iter().collect())
        .unwrap_or_default()
}

fn slot_ids(group: &Value) -> BTreeSet<String> {
    group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn dependency_refs(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if matches!(
                    key.as_str(),
                    "optionBankId"
                        | "nodeId"
                        | "sourceNodeId"
                        | "regionId"
                        | "responseGroupId"
                        | "hostNodeId"
                        | "stimulusId"
                ) {
                    if let Some(id) = child.as_str() {
                        refs.insert(format!("{key}:{id}"));
                    }
                }
                dependency_refs(child, refs);
            }
        }
        Value::Array(items) => {
            for child in items {
                dependency_refs(child, refs);
            }
        }
        _ => {}
    }
}

pub(crate) fn comparison_units(local: &Value, cloud: &Value) -> Vec<ComparisonUnit> {
    let entries: Vec<(bool, &Value, &Value)> = groups(local)
        .into_iter()
        .map(|g| (false, local, g))
        .chain(groups(cloud).into_iter().map(|g| (true, cloud, g)))
        .collect();
    let mut parent: Vec<usize> = (0..entries.len()).collect();
    fn root(parent: &[usize], mut i: usize) -> usize {
        while parent[i] != i {
            i = parent[i];
        }
        i
    }
    let numbers: Vec<_> = entries
        .iter()
        .map(|(_, doc, group)| group_question_numbers(doc, group))
        .collect();
    let refs: Vec<_> = entries
        .iter()
        .map(|(_, _, group)| {
            let mut refs = BTreeSet::new();
            dependency_refs(group, &mut refs);
            refs
        })
        .collect();
    for i in 0..entries.len() {
        for j in 0..i {
            let same_id = entries[i].2.get("taskId") == entries[j].2.get("taskId");
            let left = crate::reconcile::candidate::normalize_text(
                &crate::reconcile::candidate::nodes_text(
                    entries[i].2.get("stimulus").unwrap_or(&Value::Null),
                ),
            );
            let right = crate::reconcile::candidate::normalize_text(
                &crate::reconcile::candidate::nodes_text(
                    entries[j].2.get("stimulus").unwrap_or(&Value::Null),
                ),
            );
            if same_id
                || !numbers[i].is_disjoint(&numbers[j])
                || !refs[i].is_disjoint(&refs[j])
                || (!left.is_empty() && left == right)
            {
                let a = root(&parent, i);
                let b = root(&parent, j);
                parent[a] = b;
            }
        }
    }
    let mut components =
        std::collections::BTreeMap::<usize, (BTreeSet<String>, BTreeSet<String>)>::new();
    for (i, (is_cloud, _, group)) in entries.iter().enumerate() {
        if let Some(id) = group.get("taskId").and_then(Value::as_str) {
            let pair = components.entry(root(&parent, i)).or_default();
            if *is_cloud {
                pair.1.insert(id.to_string());
            } else {
                pair.0.insert(id.to_string());
            }
        }
    }
    components
        .into_values()
        .map(|(local_ids, cloud_ids)| {
            let bytes = serde_json::to_vec(&(&local_ids, &cloud_ids)).unwrap_or_default();
            ComparisonUnit {
                unit_id: format!("unit-{}", &crate::hash_bytes(&bytes)[..16]),
                local_task_ids: local_ids.into_iter().collect(),
                cloud_task_ids: cloud_ids.into_iter().collect(),
            }
        })
        .collect()
}

/// Full renderable document projection, not a lossy comparison digest.
pub(crate) fn unit_document(document: &Value, task_ids: &[String]) -> Value {
    let mut selected = document.clone();
    let selected_groups: Vec<Value> = groups(document)
        .into_iter()
        .filter(|g| {
            g.get("taskId")
                .and_then(Value::as_str)
                .is_some_and(|id| task_ids.iter().any(|x| x == id))
        })
        .cloned()
        .collect();
    let ids: BTreeSet<String> = selected_groups.iter().flat_map(slot_ids).collect();
    selected["taskGroups"] = serde_json::json!(selected_groups);
    for key in ["answerSlots", "answerKey"] {
        if let Some(map) = selected.get_mut(key).and_then(Value::as_object_mut) {
            map.retain(|id, _| ids.contains(id));
        }
    }
    if let Some(parts) = selected
        .pointer_mut("/listening/parts")
        .and_then(Value::as_array_mut)
    {
        for part in parts.iter_mut() {
            if let Some(ids) = part.get_mut("taskIds").and_then(Value::as_array_mut) {
                ids.retain(|id| {
                    id.as_str()
                        .is_some_and(|id| task_ids.iter().any(|x| x == id))
                });
            }
        }
        parts.retain(|p| {
            p.get("taskIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| !ids.is_empty())
        });
    }
    selected
}

pub(crate) fn replace_unit(
    current: &Value,
    local: &Value,
    cloud: &Value,
    unit_id: &str,
    use_cloud: bool,
) -> Result<Value, String> {
    let unit = comparison_units(local, cloud)
        .into_iter()
        .find(|unit| unit.unit_id == unit_id)
        .ok_or_else(|| "CLOUD_COMPARISON_UNIT_STALE".to_string())?;
    let selected = if use_cloud { cloud } else { local };
    let ids = if use_cloud {
        &unit.cloud_task_ids
    } else {
        &unit.local_task_ids
    };
    let mut all_ids: BTreeSet<String> = unit
        .local_task_ids
        .iter()
        .chain(unit.cloud_task_ids.iter())
        .cloned()
        .collect();
    let mut numbers = BTreeSet::new();
    for document in [local, cloud] {
        for group in groups(document) {
            if group
                .get("taskId")
                .and_then(Value::as_str)
                .is_some_and(|id| all_ids.contains(id))
            {
                numbers.extend(group_question_numbers(document, group));
            }
        }
    }
    let mut removed_slots = BTreeSet::new();
    let mut insertion = None;
    let mut retained = Vec::new();
    for group in groups(current) {
        let matching = group
            .get("taskId")
            .and_then(Value::as_str)
            .is_some_and(|id| all_ids.contains(id))
            || !group_question_numbers(current, group).is_disjoint(&numbers);
        if matching {
            insertion.get_or_insert(retained.len());
            removed_slots.extend(slot_ids(group));
            if let Some(id) = group.get("taskId").and_then(Value::as_str) {
                all_ids.insert(id.to_string());
            }
        } else {
            retained.push(group.clone());
        }
    }
    let bundle = unit_document(selected, ids);
    let replacement = bundle
        .get("taskGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let at = insertion.unwrap_or(retained.len());
    retained.splice(at..at, replacement);
    let mut merged = current.clone();
    merged["taskGroups"] = serde_json::json!(retained);
    for key in ["answerSlots", "answerKey"] {
        let map = merged
            .get_mut(key)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("cloud_unit_missing:{key}"))?;
        map.retain(|id, _| !removed_slots.contains(id));
        if let Some(new) = bundle.get(key).and_then(Value::as_object) {
            map.extend(new.clone());
        }
    }
    // Restore listening membership together with groups; remove stale memberships first.
    if let Some(parts) = merged
        .pointer_mut("/listening/parts")
        .and_then(Value::as_array_mut)
    {
        for part in parts.iter_mut() {
            if let Some(tasks) = part.get_mut("taskIds").and_then(Value::as_array_mut) {
                tasks.retain(|id| !id.as_str().is_some_and(|id| all_ids.contains(id)));
            }
        }
        for selected_part in bundle
            .pointer("/listening/parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(existing) = parts
                .iter_mut()
                .find(|p| p.get("partId") == selected_part.get("partId"))
            {
                if let (Some(tasks), Some(new)) = (
                    existing.get_mut("taskIds").and_then(Value::as_array_mut),
                    selected_part.get("taskIds").and_then(Value::as_array),
                ) {
                    tasks.extend(new.clone());
                }
            } else {
                parts.push(selected_part.clone());
            }
        }
        parts.retain(|p| {
            p.get("taskIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| !ids.is_empty())
        });
    }
    Ok(merged)
}

/// Publication completeness is intentionally excluded from the editor admission check.
pub(crate) fn editor_runtime_errors(
    document: &Value,
) -> Vec<crate::reading_source_v2::CompilerIssueV2> {
    let typed = match serde_json::from_value::<crate::schema::IeltsAuthoringIRV2>(document.clone())
    {
        Ok(typed) => typed,
        Err(error) => {
            return vec![crate::reading_source_v2::CompilerIssueV2 {
                code: "AUTHORING_SCHEMA_INVALID".to_string(),
                message: error.to_string(),
                target_id: "document".to_string(),
            }]
        }
    };
    match crate::listening_source_v1::compile_exam_source_v2(&typed) {
        Ok(_) => Vec::new(),
        Err(issues) => issues
            .into_iter()
            .filter(|issue| {
                !matches!(
                    issue.code.as_str(),
                    "RUNTIME_ANSWER_UNRESOLVED"
                        | "RUNTIME_ANSWER_KEY_MISSING"
                        | "RUNTIME_ANSWER_KEY_MISMATCH"
                        | "RUNTIME_ANSWER_OPTION_INVALID"
                        | "RUNTIME_ANSWER_WORD_LIMIT_VIOLATION"
                        | "RUNTIME_ANSWER_KEY_POLICY_INVALID"
                        | "RUNTIME_TEXT_SLOT_ANSWER_NOT_TEXT"
                        | "RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION"
                        | "RUNTIME_RESPONSE_ANSWER_KIND_MISMATCH"
                        | "RUNTIME_HOTSPOT_ANSWER_NOT_OPTION"
                        | "LISTENING_MEDIA_MISSING"
                        | "AUDIO_DECODE_FAILED"
                        | "AUDIO_CODEC_UNSUPPORTED"
                        | "AUDIO_HASH_MISMATCH"
                        | "AUDIO_CUE_INVALID"
                        | "AUDIO_POLICY_MISSING"
                        | "LISTENING_COMPLETE_PART_COUNT"
                        | "LISTENING_COMPLETE_QUESTION_COUNT"
                )
            })
            .collect(),
    }
}

fn prepare_partial_adoption(current: &Value, cloud: &Value) -> (Value, Vec<String>, Vec<String>) {
    let units = comparison_units(current, cloud);
    let mut merged = current.clone();
    // Passage remains independently editable; use a normalized cloud passage when renderable.
    if let Some(passage) = cloud.get("passage") {
        merged["passage"] = passage.clone();
    }
    let baseline_errors: BTreeSet<_> = editor_runtime_errors(current)
        .into_iter()
        .map(|i| (i.code, i.target_id))
        .collect();
    if editor_runtime_errors(&merged)
        .iter()
        .any(|i| !baseline_errors.contains(&(i.code.clone(), i.target_id.clone())))
    {
        merged = current.clone();
    }
    let mut adopted = Vec::new();
    let mut deferred = Vec::new();
    for unit in units {
        if unit.cloud_task_ids.is_empty() {
            continue;
        } // deletions need source review
        let mut projected = unit_document(cloud, &unit.cloud_task_ids);
        projected["modality"] = serde_json::json!("reading");
        if projected["passage"].is_null() {
            projected["passage"] = serde_json::json!({"title":"","content":[],"sourceAnchors":[]});
        }
        if !editor_runtime_errors(&projected).is_empty() {
            deferred.extend(unit.cloud_task_ids);
            continue;
        }
        match replace_unit(&merged, current, cloud, &unit.unit_id, true) {
            Ok(trial)
                if editor_runtime_errors(&trial)
                    .iter()
                    .all(|i| baseline_errors.contains(&(i.code.clone(), i.target_id.clone()))) =>
            {
                merged = trial;
                adopted.extend(unit.cloud_task_ids);
            }
            _ => deferred.extend(unit.cloud_task_ids),
        }
    }
    (merged, adopted, deferred)
}

fn object_contains_identity(value: &Value, id: &str) -> bool {
    if [
        "id",
        "taskId",
        "slotId",
        "responseGroupId",
        "assetId",
        "optionId",
    ]
    .iter()
    .any(|key| value.get(key).and_then(Value::as_str) == Some(id))
    {
        return true;
    }
    match value {
        Value::Array(items) => items.iter().any(|item| object_contains_identity(item, id)),
        Value::Object(map) => map.values().any(|item| object_contains_identity(item, id)),
        _ => false,
    }
}

fn group_ids_for_reference(document: &Value, id: &str) -> BTreeSet<String> {
    document
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|group| object_contains_identity(group, id))
        .filter_map(|group| group.get("taskId").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn command_group_ids(command: &Value, document: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for value in [
        command.get("taskId"),
        command.get("replacesTaskId"),
        command.pointer("/taskGroup/taskId"),
        command.pointer("/target/taskId"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    {
        ids.insert(value.to_string());
    }
    for key in [
        "nodeId",
        "parentId",
        "slotId",
        "responseGroupId",
        "optionId",
    ] {
        if let Some(id) = command.get(key).and_then(Value::as_str) {
            ids.extend(group_ids_for_reference(document, id));
        }
    }
    for key in ["nodeId", "responseGroupId", "optionId"] {
        if let Some(id) = command
            .pointer(&format!("/target/{key}"))
            .and_then(Value::as_str)
        {
            ids.extend(group_ids_for_reference(document, id));
        }
    }
    ids
}

fn put_answer_slot(current: &Value, cloud: &mut Value, slot_id: &str) -> Result<(), String> {
    let current_slot = current
        .pointer(&format!("/answerSlots/{slot_id}"))
        .cloned()
        .ok_or_else(|| format!("human_answer_slot_missing:{slot_id}"))?;
    let question_number = current_slot
        .get("questionNumber")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("human_answer_slot_question_missing:{slot_id}"))?;
    let removed = cloud
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|slots| slots.iter())
        .filter(|(key, value)| {
            key.as_str() == slot_id
                || value.get("questionNumber").and_then(Value::as_u64) == Some(question_number)
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    if let Some(slots) = cloud.get_mut("answerSlots").and_then(Value::as_object_mut) {
        for key in &removed {
            slots.remove(key);
        }
        slots.insert(slot_id.to_string(), current_slot);
    } else {
        return Err("cloud_candidate_answer_slots_missing".to_string());
    }
    if let Some(answers) = cloud.get_mut("answerKey").and_then(Value::as_object_mut) {
        for key in removed {
            answers.remove(&key);
        }
        match current.pointer(&format!("/answerKey/{slot_id}")) {
            Some(answer) => {
                answers.insert(slot_id.to_string(), answer.clone());
            }
            None => {
                answers.remove(slot_id);
            }
        }
    }
    Ok(())
}

fn preserve_groups(
    current: &Value,
    cloud: &mut Value,
    seed_group_ids: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let current_groups = current
        .get("taskGroups")
        .and_then(Value::as_array)
        .ok_or_else(|| "current_task_groups_missing".to_string())?;
    let cloud_groups = cloud
        .get("taskGroups")
        .and_then(Value::as_array)
        .ok_or_else(|| "cloud_candidate_task_groups_missing".to_string())?;
    let seed_groups = current_groups
        .iter()
        .filter(|group| {
            group
                .get("taskId")
                .and_then(Value::as_str)
                .is_some_and(|task_id| seed_group_ids.contains(task_id))
        })
        .collect::<Vec<_>>();
    if seed_groups.is_empty() {
        return Err("human_edited_task_group_not_found".to_string());
    }
    let seed_numbers = seed_groups
        .iter()
        .flat_map(|group| group_question_numbers(current, group))
        .collect::<BTreeSet<_>>();
    let matching_cloud_indexes = cloud_groups
        .iter()
        .enumerate()
        .filter(|(_, group)| {
            let same_id = group
                .get("taskId")
                .and_then(Value::as_str)
                .is_some_and(|task_id| seed_group_ids.contains(task_id));
            same_id || !group_question_numbers(cloud, group).is_disjoint(&seed_numbers)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if matching_cloud_indexes.is_empty() {
        return Err("cloud_candidate_has_no_group_for_human_target".to_string());
    }
    let first_cloud_index = matching_cloud_indexes[0];
    let cloud_group_numbers = matching_cloud_indexes
        .iter()
        .flat_map(|index| group_question_numbers(cloud, &cloud_groups[*index]))
        .collect::<BTreeSet<_>>();
    let affected_numbers = seed_numbers
        .union(&cloud_group_numbers)
        .copied()
        .collect::<BTreeSet<_>>();

    // Keep every canonical group participating in the cloud groups being replaced. This is
    // conservative for a split/merge and ensures a user's structural edit cannot strand slots.
    let restore_groups = current_groups
        .iter()
        .enumerate()
        .filter(|(_, group)| {
            !group_question_numbers(current, group).is_disjoint(&affected_numbers)
                || group
                    .get("taskId")
                    .and_then(Value::as_str)
                    .is_some_and(|task_id| seed_group_ids.contains(task_id))
        })
        .map(|(index, group)| (index, group.clone()))
        .collect::<Vec<_>>();
    let preserved_ids = restore_groups
        .iter()
        .filter_map(|(_, group)| group.get("taskId").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();

    let groups = cloud
        .get_mut("taskGroups")
        .and_then(Value::as_array_mut)
        .unwrap();
    for index in matching_cloud_indexes.iter().rev() {
        groups.remove(*index);
    }
    let insertion_index = first_cloud_index.min(groups.len());
    for (offset, (_, group)) in restore_groups.iter().enumerate() {
        let index = (insertion_index + offset).min(groups.len());
        groups.insert(index, group.clone());
    }

    let cloud_slot_ids = cloud
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|slots| slots.iter())
        .filter(|(_, slot)| {
            slot.get("questionNumber")
                .and_then(Value::as_u64)
                .is_some_and(|number| affected_numbers.contains(&(number as u32)))
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    if let Some(slots) = cloud.get_mut("answerSlots").and_then(Value::as_object_mut) {
        for slot_id in &cloud_slot_ids {
            slots.remove(slot_id);
        }
    }
    if let Some(answers) = cloud.get_mut("answerKey").and_then(Value::as_object_mut) {
        for slot_id in &cloud_slot_ids {
            answers.remove(slot_id);
        }
    }
    let current_slots = current
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|slots| slots.iter())
        .filter(|(_, slot)| {
            slot.get("questionNumber")
                .and_then(Value::as_u64)
                .is_some_and(|number| affected_numbers.contains(&(number as u32)))
        })
        .map(|(slot_id, slot)| (slot_id.clone(), slot.clone()))
        .collect::<Vec<_>>();
    for (slot_id, slot) in current_slots {
        cloud["answerSlots"]
            .as_object_mut()
            .ok_or_else(|| "cloud_candidate_answer_slots_missing".to_string())?
            .insert(slot_id.clone(), slot);
        if let Some(answer) = current.pointer(&format!("/answerKey/{slot_id}")) {
            cloud["answerKey"]
                .as_object_mut()
                .ok_or_else(|| "cloud_candidate_answer_key_missing".to_string())?
                .insert(slot_id, answer.clone());
        }
    }
    Ok(preserved_ids)
}

fn preserve_listening_part(current: &Value, cloud: &mut Value, part_id: &str) -> bool {
    let Some(current_part) = current
        .pointer("/listening/parts")
        .and_then(Value::as_array)
        .and_then(|parts| {
            parts
                .iter()
                .find(|part| part.get("partId").and_then(Value::as_str) == Some(part_id))
        })
        .cloned()
    else {
        return false;
    };
    let Some(parts) = cloud
        .pointer_mut("/listening/parts")
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    if let Some(index) = parts
        .iter()
        .position(|part| part.get("partId").and_then(Value::as_str) == Some(part_id))
    {
        parts[index] = current_part;
        true
    } else {
        false
    }
}

/// Rebase saved user edits from the journal onto a freshly normalized cloud candidate.
/// Structural question-group edits preserve the complete original group and its answer slots.
pub(crate) fn merge_human_edits(
    current: &Value,
    cloud: &mut Value,
    journal: &[Value],
) -> Result<Vec<String>, String> {
    let mut preserved_groups = BTreeSet::new();
    for entry in journal {
        let payload = entry.get("command").unwrap_or(entry);
        let commands = payload
            .get("commands")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for command in commands {
            let op = command
                .get("op")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if op == "setAnswer" {
                if let Some(slot_id) = command.get("slotId").and_then(Value::as_str) {
                    put_answer_slot(current, cloud, slot_id)?;
                }
                continue;
            }
            if op == "setListeningPartMedia" {
                let part_id = command
                    .get("partId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "human_listening_part_id_missing".to_string())?;
                if !preserve_listening_part(current, cloud, part_id) {
                    return Err(format!("human_listening_part_not_rebased:{part_id}"));
                }
                continue;
            }
            let group_wide = matches!(
                op,
                "setTaskType"
                    | "setQuestionExpression"
                    | "setResponseCardinality"
                    | "setResponseGroup"
                    | "setOptionBank"
                    | "insertAnswerSlot"
                    | "deleteAnswerSlot"
                    | "upsertTaskGroupBundle"
                    | "insertNode"
                    | "deleteNode"
                    | "moveNode"
            );
            if group_wide {
                let groups = command_group_ids(&command, current);
                preserved_groups.extend(preserve_groups(current, cloud, &groups)?);
                continue;
            }
            if op == "replaceContent" {
                let kind = command.pointer("/target/kind").and_then(Value::as_str);
                if kind == Some("passage") {
                    cloud["passage"] = current.get("passage").cloned().unwrap_or(Value::Null);
                    continue;
                }
                if matches!(kind, Some("responsePrompt" | "option" | "node")) {
                    let id = ["responseGroupId", "optionId", "nodeId"]
                        .into_iter()
                        .find_map(|key| {
                            command
                                .pointer(&format!("/target/{key}"))
                                .and_then(Value::as_str)
                        });
                    if let Some(id) = id {
                        if let Some(value) =
                            crate::library::repository::find_object_by_id(current, id)
                        {
                            if crate::library::repository::replace_object_by_id(cloud, id, value) {
                                continue;
                            }
                        }
                    }
                }
                let groups = command_group_ids(&command, current);
                if !groups.is_empty() {
                    preserved_groups.extend(preserve_groups(current, cloud, &groups)?);
                    continue;
                }
                return Err("human_content_container_not_rebased".to_string());
            }
            let direct_id = command
                .get("nodeId")
                .and_then(Value::as_str)
                .or_else(|| command.get("assetId").and_then(Value::as_str))
                .or_else(|| command.get("targetId").and_then(Value::as_str));
            if matches!(
                op,
                "replaceText"
                    | "setNodeAttrs"
                    | "cropAsset"
                    | "setHotspot"
                    | "removeHotspot"
                    | "bindSource"
            ) {
                if let Some(id) = direct_id {
                    if let Some(value) = crate::library::repository::find_object_by_id(current, id)
                    {
                        if crate::library::repository::replace_object_by_id(cloud, id, value) {
                            continue;
                        }
                    }
                    let groups = command_group_ids(&command, current);
                    if !groups.is_empty() {
                        preserved_groups.extend(preserve_groups(current, cloud, &groups)?);
                        continue;
                    }
                    if object_contains_identity(current.get("passage").unwrap_or(&Value::Null), id)
                    {
                        cloud["passage"] = current.get("passage").cloned().unwrap_or(Value::Null);
                        continue;
                    }
                }
                return Err(format!("human_edit_target_not_rebased:{op}"));
            }
            return Err(format!("human_edit_rebase_unsupported:{op}"));
        }

        // An explicit user undo carries restored target IDs in the journal result rather than
        // replayable editor commands. Preserve each current value (or its owning whole group).
        if payload.get("undoRepairRunId").is_some() {
            let targets = entry
                .pointer("/result/restored")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            for target in targets {
                if let Some(slot_id) = target.strip_prefix("answerKey:") {
                    put_answer_slot(current, cloud, slot_id)?;
                } else if target.starts_with("$task-group:") {
                    let task_id = target.trim_start_matches("$task-group:");
                    preserved_groups.extend(preserve_groups(
                        current,
                        cloud,
                        &BTreeSet::from([task_id.to_string()]),
                    )?);
                } else if let Some(value) =
                    crate::library::repository::find_object_by_id(current, target)
                {
                    if !crate::library::repository::replace_object_by_id(cloud, target, value) {
                        let groups = group_ids_for_reference(current, target);
                        if !groups.is_empty() {
                            preserved_groups.extend(preserve_groups(current, cloud, &groups)?);
                        }
                    }
                }
            }
        }
    }
    Ok(preserved_groups.into_iter().collect())
}

/// Promote a qualified full candidate using the canonical editor CAS/journal transaction.
/// A version conflict is retried from a fresh canonical + human journal snapshot.
pub(crate) fn adopt_cloud_candidate(
    root: &Path,
    item_id: &str,
    batch_id: &str,
    candidate_base_version: i64,
    cloud_authoring: &Value,
) -> Result<AdoptionCommit, String> {
    let run_id = crate::cloud_repair::repair_run_id_for(batch_id);
    let request_id = format!("cloud-candidate-adoption:{batch_id}");
    let mut last_conflict = None;
    for _attempt in 0..3 {
        let mut conn = crate::library::repository::open_library_connection(root)?;
        let Some((current, current_version)) =
            crate::library::repository::get_canonical_ds(&conn, item_id)?
        else {
            return Err("cloud_candidate_adoption_canonical_missing".to_string());
        };
        if candidate_base_version > current_version {
            return Err(format!(
                "cloud_candidate_adoption_base_ahead:base={candidate_base_version}:current={current_version}"
            ));
        }
        let journal = crate::library::repository::human_editor_commands_since(
            &conn,
            item_id,
            candidate_base_version,
        )?;
        let (mut merged, mut adopted_task_ids, mut deferred_task_ids) =
            prepare_partial_adoption(&current, cloud_authoring);
        if adopted_task_ids.is_empty() {
            return Err(
                "CLOUD_CANDIDATE_NO_RENDERABLE_UNITS:候选已接收，题组需要先经校核修复".to_string(),
            );
        }
        if let Some(assets) = current.get("assets") {
            merged["assets"] = assets.clone();
        }
        let preserved_group_ids = merge_human_edits(&current, &mut merged, &journal)?;
        // A preserved human group remains on the current side of comparison, not the adopted side.
        let preserved: BTreeSet<_> = preserved_group_ids.iter().collect();
        adopted_task_ids.retain(|id| {
            if preserved.contains(id) {
                deferred_task_ids.push(id.clone());
                false
            } else {
                true
            }
        });
        let baseline: BTreeSet<_> = editor_runtime_errors(&current)
            .into_iter()
            .map(|i| (i.code, i.target_id))
            .collect();
        let introduced: Vec<_> = editor_runtime_errors(&merged)
            .into_iter()
            .filter(|i| !baseline.contains(&(i.code.clone(), i.target_id.clone())))
            .map(|i| format!("{}:{}", i.code, i.target_id))
            .collect();
        if !introduced.is_empty() {
            return Err(format!(
                "CLOUD_CANDIDATE_REBASE_STRUCTURE_INVALID:{}",
                introduced.join(",")
            ));
        }
        if let (Some(current_audit), Some(audit)) = (
            current.get("audit").and_then(Value::as_object),
            merged.get_mut("audit").and_then(Value::as_object_mut),
        ) {
            audit.insert(
                "revision".to_string(),
                serde_json::json!(current_audit
                    .get("revision")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .saturating_add(1)),
            );
            audit.insert(
                "source".to_string(),
                serde_json::json!("cloud_candidate_adoption"),
            );
            audit.insert(
                "humanVerified".to_string(),
                current_audit
                    .get("humanVerified")
                    .cloned()
                    .unwrap_or(serde_json::Value::Bool(false)),
            );
            audit.insert(
                "updatedAt".to_string(),
                serde_json::json!(chrono::Utc::now().to_rfc3339()),
            );
        }
        let command = serde_json::json!({
            "op": "replaceAuthoringDocument",
            "authoring": merged,
        });
        let result = crate::library::repository::apply_editor_commands_tx_with(
            &mut conn,
            &crate::library::repository::ApplyEditorCommandsInput {
                item_id: item_id.to_string(),
                base_version: current_version,
                request_id: Some(request_id.clone()),
                commands: vec![command],
                title: None,
            },
            crate::library::repository::EditOrigin::CloudCandidateAdoption,
            Some(&run_id),
            &|document, patch| {
                if patch.get("op").and_then(Value::as_str) == Some("replaceAuthoringDocument") {
                    let authoring = patch
                        .get("authoring")
                        .filter(|value| value.is_object())
                        .ok_or_else(|| "cloud_candidate_adoption_document_invalid".to_string())?;
                    *document = authoring.clone();
                    Ok(())
                } else {
                    crate::authoring_v2_commands::apply_patch(document, patch)
                }
            },
            &|document| {
                crate::authoring_v2_commands::refresh_quality_report(root, item_id, document)?;
                crate::authoring_v2_commands::validate_authoring(document)
            },
            &|_, _| Ok(()),
        );
        match result {
            Ok(result) => {
                return Ok(AdoptionCommit {
                    edit_version: result.edit_version,
                    preserved_group_ids,
                    applied_targets: result.applied_targets,
                    adopted_task_ids,
                    deferred_task_ids,
                });
            }
            Err(error) if error.starts_with("EDIT_VERSION_CONFLICT") => {
                last_conflict = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_conflict.unwrap_or_else(|| "cloud_candidate_adoption_conflict".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn candidate() -> Value {
        let mut authoring: Value = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .expect("committed fixture parses");
        authoring["quality"]["compilerProbes"]["v2Runtime"]["status"] = json!("passed");
        authoring["quality"]["hardFailures"] = json!([]);
        json!({
            "status": "succeeded",
            "unresolvedReferences": [],
            "authoring": authoring
        })
    }

    fn local() -> Value {
        json!({"slots": [
            {"slotId":"q14", "questionNumber":14},
            {"slotId":"q15", "questionNumber":15}
        ]})
    }

    #[test]
    fn listening_candidate_without_audio_is_editable_and_keeps_active_runtime_contract() {
        let typed = crate::test_support::complete_listening_exam();
        let mut current = serde_json::to_value(typed).unwrap();
        for part in current["listening"]["parts"].as_array_mut().unwrap() {
            part.as_object_mut().unwrap().remove("media");
        }
        let cloud = current.clone();
        let (_, adopted, deferred) = prepare_partial_adoption(&current, &cloud);
        assert!(deferred.is_empty(), "{deferred:?}");
        assert_eq!(adopted.len(), cloud["taskGroups"].as_array().unwrap().len());
        let typed = serde_json::from_value::<crate::schema::IeltsAuthoringIRV2>(cloud).unwrap();
        assert!(
            crate::listening_source_v1::compile_exam_source_v2(&typed)
                .unwrap_err()
                .iter()
                .any(|i| i.code == "LISTENING_MEDIA_MISSING"),
            "Publication still requires usable audio"
        );
    }
    #[test]
    fn complete_runtime_valid_candidate_covers_local_numbers_and_keeps_q_ids() {
        let cloud = candidate();
        assert!(adoption_rejection_reasons(&cloud, &local()).is_empty());
        assert_eq!(
            cloud.pointer("/authoring/answerSlots/q14/slotId"),
            Some(&json!("q14"))
        );
        assert_eq!(
            cloud.pointer("/authoring/answerSlots/q15/slotId"),
            Some(&json!("q15"))
        );
    }

    #[test]
    fn reception_accepts_partial_candidates_and_defers_all_publication_findings() {
        let mut cloud = candidate();
        cloud["status"] = json!("partial");
        cloud["unresolvedReferences"] = json!(["bad-group"]);
        cloud["authoring"]["quality"]["hardFailures"] =
            json!(["PROVENANCE_MISSING", "RUNTIME_COMPILER_FAILED"]);
        cloud["authoring"]["quality"]["compilerProbes"]["v2Runtime"]["status"] = json!("failed");
        assert!(adoption_rejection_reasons(&cloud, &json!({"slots":[]})).is_empty());
    }

    #[test]
    fn local_false_question_cannot_reject_a_received_cloud_candidate() {
        let cloud = candidate();
        let local = json!({"slots":[{"questionNumber":999}]});
        assert!(adoption_rejection_reasons(&cloud, &local).is_empty());
        assert!(!adoption_rejection_reasons(&json!({"authoring":null}), &local).is_empty());
    }

    #[test]
    fn missing_answers_are_editor_admissible_but_broken_slot_references_are_not() {
        let mut document = candidate()["authoring"].clone();
        document["answerKey"]["q14"] = json!({"kind":"unresolved"});
        assert!(
            editor_runtime_errors(&document).is_empty(),
            "{:?}",
            editor_runtime_errors(&document)
        );
        document["taskGroups"][0]["responseGroups"][0]["slotIds"] = json!(["missing-slot"]);
        assert!(!editor_runtime_errors(&document).is_empty());
    }

    #[test]
    fn dependency_unit_groups_local_split_against_cloud_merge_and_choice_is_complete() {
        let local = json!({"taskGroups":[
            {"taskId":"a","displayRange":{"kind":"set","values":[1]},"responseGroups":[{"slotIds":["q1"]}]},
            {"taskId":"b","displayRange":{"kind":"set","values":[2]},"responseGroups":[{"slotIds":["q2"]}]}
        ], "answerSlots":{"q1":{"questionNumber":1},"q2":{"questionNumber":2}}, "answerKey":{"q1":{},"q2":{}}});
        let cloud = json!({"taskGroups":[{"taskId":"c","displayRange":{"kind":"set","values":[1,2]},"responseGroups":[{"slotIds":["q1","q2"]}]}],"answerSlots":{"q1":{"questionNumber":1},"q2":{"questionNumber":2}},"answerKey":{"q1":{},"q2":{}}});
        let units = comparison_units(&local, &cloud);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].local_task_ids, vec!["a", "b"]);
        let selected = replace_unit(&local, &local, &cloud, &units[0].unit_id, true).unwrap();
        assert_eq!(selected["taskGroups"], cloud["taskGroups"]);
        assert_eq!(selected["answerSlots"], cloud["answerSlots"]);
        assert_eq!(
            replace_unit(&selected, &local, &cloud, &units[0].unit_id, false).unwrap(),
            local
        );
    }

    #[test]
    fn one_invalid_group_does_not_prevent_nine_renderable_groups_adopting() {
        let base = candidate()["authoring"].clone();
        let mut local = base.clone();
        let mut cloud = base.clone();
        let mut local_groups = Vec::new();
        let mut cloud_groups = Vec::new();
        let mut slots = serde_json::Map::new();
        let mut answers = serde_json::Map::new();
        for i in 0..10 {
            let offset = i * 2;
            let mut group = base["taskGroups"][0].clone();
            let serialized = serde_json::to_string(&group)
                .unwrap()
                .replace("q14", &format!("q{}", 14 + offset))
                .replace("q15", &format!("q{}", 15 + offset))
                .replace("early-approaches-q14-15", &format!("group-{i}"));
            group = serde_json::from_str(&serialized).unwrap();
            group["taskId"] = json!(format!("group-{i}"));
            group["displayRange"] = json!({"kind":"set","values":[14+offset,15+offset]});
            // Each unit has its own response/option identities.
            let serialized = serde_json::to_string(&group)
                .unwrap()
                .replace("early-approaches-", &format!("g{i}-"));
            group = serde_json::from_str(&serialized).unwrap();
            let mut cloud_group = group.clone();
            cloud_group["instructions"][0]["children"][0]["text"] =
                json!(format!("Cloud instructions {i}"));
            if i == 9 {
                cloud_group["responseGroups"][0]["slotIds"] = json!(["missing-slot"]);
            }
            local_groups.push(group);
            cloud_groups.push(cloud_group);
            for number in [14 + offset, 15 + offset] {
                let original = if number % 2 == 0 { "q14" } else { "q15" };
                let id = format!("q{number}");
                let mut slot = base["answerSlots"][original].clone();
                slot["slotId"] = json!(id);
                slot["questionNumber"] = json!(number);
                slot["displayLabel"] = json!(number.to_string());
                // use generated response/node identities for runtime closure
                if let Some(rg) = slot.get_mut("responseGroupId") {
                    *rg = json!(rg
                        .as_str()
                        .unwrap()
                        .replace("early-approaches-", &format!("g{i}-"))
                        .replace("q14", &format!("q{}", 14 + offset))
                        .replace("q15", &format!("q{}", 15 + offset)));
                }
                if let Some(host) = slot.get_mut("hostNodeId") {
                    *host = json!(host
                        .as_str()
                        .unwrap()
                        .replace("early-approaches-", &format!("g{i}-"))
                        .replace("q14", &format!("q{}", 14 + offset))
                        .replace("q15", &format!("q{}", 15 + offset)));
                }
                slots.insert(id.clone(), slot);
                answers.insert(id, base["answerKey"][original].clone());
            }
        }
        for doc in [&mut local, &mut cloud] {
            doc["answerSlots"] = json!(slots);
            doc["answerKey"] = json!(answers);
        }
        local["taskGroups"] = json!(local_groups);
        cloud["taskGroups"] = json!(cloud_groups);
        let (merged, adopted, deferred) = prepare_partial_adoption(&local, &cloud);
        assert_eq!(
            adopted.len(),
            9,
            "deferred={deferred:?}, local errors={:?}",
            editor_runtime_errors(&local)
        );
        assert_eq!(deferred.len(), 1);
        assert_eq!(merged["taskGroups"][9], local["taskGroups"][9]);
    }

    #[test]
    fn human_text_and_answer_edits_are_rebased_onto_the_cloud_candidate() {
        let current = json!({
            "passage": {"content": [{"id":"p-text", "type":"text", "text":"local user text"}]},
            "taskGroups": [{
                "taskId":"task-1", "displayRange":{"kind":"range","start":14,"end":14},
                "instructions":[{"id":"instruction-text", "type":"text", "text":"user instruction"}],
                "responseGroups":[{"responseGroupId":"rg-1", "slotIds":["q14"]}]
            }],
            "answerSlots":{"q14":{"slotId":"q14", "questionNumber":14}},
            "answerKey":{"q14":{"kind":"text", "values":["user answer"]}}
        });
        let mut cloud = json!({
            "passage": {"content": [{"id":"p-text", "type":"text", "text":"cloud text"}]},
            "taskGroups": [{
                "taskId":"task-1", "displayRange":{"kind":"range","start":14,"end":14},
                "instructions":[{"id":"instruction-text", "type":"text", "text":"cloud instruction"}],
                "responseGroups":[{"responseGroupId":"rg-1", "slotIds":["q14"]}]
            }],
            "answerSlots":{"q14":{"slotId":"q14", "questionNumber":14}},
            "answerKey":{"q14":{"kind":"text", "values":["cloud answer"]}}
        });
        let journal = [json!({"command":{"commands":[
            {"op":"replaceText", "nodeId":"p-text"},
            {"op":"setAnswer", "slotId":"q14"}
        ]}, "result":{}})];

        merge_human_edits(&current, &mut cloud, &journal).unwrap();
        assert_eq!(
            cloud.pointer("/passage/content/0/text"),
            Some(&json!("local user text"))
        );
        assert_eq!(
            cloud.pointer("/answerKey/q14/values/0"),
            Some(&json!("user answer"))
        );
        assert_eq!(
            cloud.pointer("/taskGroups/0/instructions/0/text"),
            Some(&json!("cloud instruction"))
        );
    }

    #[test]
    fn structural_user_edit_preserves_whole_group_and_its_slots() {
        let current = json!({
            "taskGroups": [{
                "taskId":"task-1", "displayRange":{"kind":"range","start":14,"end":15},
                "optionBank":{"optionBankId":"user-bank", "options":[{"optionId":"u-a","label":"A"}]},
                "responseGroups":[{"responseGroupId":"rg-1", "slotIds":["q14","q15"]}]
            }],
            "answerSlots":{
                "q14":{"slotId":"q14", "questionNumber":14},
                "q15":{"slotId":"q15", "questionNumber":15}
            },
            "answerKey":{
                "q14":{"kind":"text", "values":["user 14"]},
                "q15":{"kind":"text", "values":["user 15"]}
            }
        });
        let mut cloud = json!({
            "taskGroups": [{
                "taskId":"task-1", "displayRange":{"kind":"range","start":14,"end":15},
                "optionBank":{"optionBankId":"cloud-bank", "options":[{"optionId":"c-a","label":"A"}]},
                "responseGroups":[{"responseGroupId":"rg-1", "slotIds":["q14","q15"]}]
            }],
            "answerSlots":{
                "q14":{"slotId":"q14", "questionNumber":14},
                "q15":{"slotId":"q15", "questionNumber":15}
            },
            "answerKey":{
                "q14":{"kind":"text", "values":["cloud 14"]},
                "q15":{"kind":"text", "values":["cloud 15"]}
            }
        });
        let journal = [json!({"command":{"commands":[
            {"op":"setOptionBank", "taskId":"task-1"}
        ]}, "result":{}})];

        let preserved = merge_human_edits(&current, &mut cloud, &journal).unwrap();
        assert_eq!(preserved, vec!["task-1"]);
        assert_eq!(
            cloud["taskGroups"][0]["optionBank"]["optionBankId"],
            "user-bank"
        );
        assert_eq!(
            cloud.pointer("/answerKey/q14/values/0"),
            Some(&json!("user 14"))
        );
        assert_eq!(
            cloud.pointer("/answerKey/q15/values/0"),
            Some(&json!("user 15"))
        );
    }
}
