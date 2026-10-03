//! Backend-constructed, dependency-closed cloud choices. Model output never supplies a document.
use super::RepairRunRequest;
use crate::{
    library::repository::{self, ApplyEditorCommandsInput, EditOrigin},
    reconcile::store,
    CommandResult,
};
use serde_json::{json, Value};

// Only content fields in independent single-slot responses may be mixed. Structural
// differences, shared option banks and multi-slot answers still use complete units.
pub(super) fn independent_field(
    local: &Value,
    cloud: &Value,
    current: &Value,
    entry: &Value,
) -> bool {
    let target = entry["targetId"].as_str().unwrap_or("");
    let kind = entry["targetType"].as_str().unwrap_or("");
    let field = entry["field"].as_str().unwrap_or("");
    if !matches!(
        (kind, field),
        ("slot", "answer") | ("response_group", "prompt")
    ) {
        return false;
    }
    fn find<'a>(doc: &'a Value, target: &str, kind: &str) -> Option<(&'a Value, &'a Value)> {
        for group in doc["taskGroups"].as_array()? {
            for response in group["responseGroups"].as_array()? {
                let owns = if kind == "slot" {
                    response["slotIds"]
                        .as_array()
                        .is_some_and(|ids| ids.iter().any(|id| id == target))
                } else {
                    response["responseGroupId"] == target
                };
                if owns {
                    return Some((group, response));
                }
            }
        }
        None
    }
    let Some((a, ra)) = find(local, target, kind) else {
        return false;
    };
    let Some((b, rb)) = find(cloud, target, kind) else {
        return false;
    };
    let Some((c, rc)) = find(current, target, kind) else {
        return false;
    };
    for response in [ra, rb, rc] {
        if response["slotIds"]
            .as_array()
            .is_none_or(|ids| ids.len() != 1)
            || !response["optionBankRef"].is_null()
            || response["assignment"]
                .as_str()
                .is_some_and(|a| a != "per_slot")
        {
            return false;
        }
    }
    let slot = ra["slotIds"][0].as_str().unwrap_or("");
    let a_slot = super::comparison_content(&local["answerSlots"][slot]);
    if a_slot != super::comparison_content(&cloud["answerSlots"][slot])
        || a_slot != super::comparison_content(&current["answerSlots"][slot])
    {
        return false;
    }
    fn structure(group: &Value) -> Value {
        let mut group = super::comparison_content(group);
        if let Some(responses) = group["responseGroups"].as_array_mut() {
            for response in responses {
                if let Some(object) = response.as_object_mut() {
                    object.remove("prompt");
                }
            }
        }
        group
    }
    structure(a) == structure(b) && structure(a) == structure(c)
}

pub(super) fn apply_cloud_units(
    request: &RepairRunRequest<'_>,
    entries: &[Value],
    base_version: i64,
    call_id: &str,
) -> CommandResult<usize> {
    let local =
        store::read_local_authoring_snapshot(request.root, request.job_id, request.batch_id)?
            .ok_or("CLOUD_LOCAL_SNAPSHOT_MISSING")?;
    let cloud =
        store::read_cloud_authoring_candidate(request.root, request.job_id, request.batch_id)?
            .ok_or("CLOUD_CANDIDATE_MISSING")?;
    let cloud = serde_json::to_value(cloud.authoring).map_err(|e| e.to_string())?;
    let mut conn = repository::open_library_connection(request.root)?;
    let (current, version) =
        repository::get_canonical_ds(&conn, request.item_id)?.ok_or("ITEM_DS_NOT_SEEDED")?;
    if version != base_version {
        return Err(format!(
            "EDIT_VERSION_CONFLICT:current={version}:base={base_version}"
        ));
    }
    let units = crate::cloud_adoption::comparison_units(&local, &cloud);
    let owns = |unit: &crate::cloud_adoption::ComparisonUnit, target: &str| {
        unit.local_task_ids
            .iter()
            .chain(&unit.cloud_task_ids)
            .any(|id| id == target)
            || [&local, &cloud].iter().any(|doc| {
                let ids = if std::ptr::eq(*doc, &local) {
                    &unit.local_task_ids
                } else {
                    &unit.cloud_task_ids
                };
                let slice = crate::cloud_adoption::unit_document(doc, ids);
                fn has(value: &Value, target: &str) -> bool {
                    match value {
                        Value::String(s) => s == target,
                        Value::Array(a) => a.iter().any(|v| has(v, target)),
                        Value::Object(m) => m.iter().any(|(k, v)| k == target || has(v, target)),
                        _ => false,
                    }
                }
                has(&slice["taskGroups"], target) || has(&slice["answerSlots"], target)
            })
    };
    let independent: Vec<_> = entries
        .iter()
        .filter(|entry| independent_field(&local, &cloud, &current, entry))
        .collect();
    let selected: Vec<_> = units
        .iter()
        .filter(|unit| {
            entries.iter().any(|entry| {
                !independent_field(&local, &cloud, &current, entry)
                    && (owns(unit, entry["targetId"].as_str().unwrap_or(""))
                        || entry["comparisonUnitId"].as_str() == Some(unit.unit_id.as_str()))
            })
        })
        .collect();
    let select_passage = entries
        .iter()
        .any(|entry| entry["targetType"] == "passage" || entry["targetId"] == "passage");
    if selected.is_empty() && independent.is_empty() && !select_passage {
        return Err("CLOUD_DECISION_UNIT_MISSING".into());
    }
    // A comparison unit is dependency closed: contradictory per-question choices cannot
    // split a shared option bank or answer region between candidate versions.
    for unit in &selected {
        let mut choices = std::collections::BTreeSet::new();
        for entry in entries {
            if owns(unit, entry["targetId"].as_str().unwrap_or(""))
                || entry["comparisonUnitId"].as_str() == Some(unit.unit_id.as_str())
            {
                choices.insert(entry["decision"].as_str().unwrap_or("use_cloud"));
            }
        }
        if choices.len() > 1 {
            return Err(format!("CLOUD_DECISION_CONFLICTING_UNIT:{}", unit.unit_id));
        }
    }
    let mut updated = current.clone();
    let mut commands = Vec::new();
    for entry in &independent {
        let source = if entry["decision"] == "use_local" {
            &local
        } else {
            &cloud
        };
        let target = entry["targetId"].as_str().unwrap_or("");
        if entry["targetType"] == "slot" {
            updated["answerKey"][target] = source["answerKey"][target].clone();
            commands.push(
                json!({"op":"setAnswer","slotId":target,"value":source["answerKey"][target]}),
            );
        } else {
            let prompt = source["taskGroups"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|g| g["responseGroups"].as_array().into_iter().flatten())
                .find(|r| r["responseGroupId"] == target)
                .map(|r| r["prompt"].clone())
                .ok_or("CLOUD_DECISION_RESPONSE_MISSING")?;
            for group in updated["taskGroups"].as_array_mut().into_iter().flatten() {
                for response in group["responseGroups"].as_array_mut().into_iter().flatten() {
                    if response["responseGroupId"] == target {
                        response["prompt"] = prompt.clone();
                    }
                }
            }
            commands.push(json!({"op":"replaceContent","target":{"kind":"responsePrompt","responseGroupId":target},"content":prompt}));
        }
    }
    for unit in &selected {
        updated = crate::cloud_adoption::replace_unit(
            &updated,
            &local,
            &cloud,
            &unit.unit_id,
            entries
                .iter()
                .find(|entry| {
                    owns(unit, entry["targetId"].as_str().unwrap_or(""))
                        || entry["comparisonUnitId"].as_str() == Some(unit.unit_id.as_str())
                })
                .is_none_or(|entry| entry["decision"] != "use_local"),
        )?;
        for id in unit.local_task_ids.iter().chain(&unit.cloud_task_ids) {
            let group = updated["taskGroups"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|g| g["taskId"].as_str() == Some(id));
            commands.push(match group { Some(group) => json!({"op":"upsertTaskGroupBundle","taskGroup":group,"comparisonUnitId":unit.unit_id}), None => json!({"op":"removeTaskGroup","taskId":id}) });
        }
    }
    if select_passage {
        let choices: std::collections::BTreeSet<_> = entries
            .iter()
            .filter(|entry| entry["targetType"] == "passage" || entry["targetId"] == "passage")
            .map(|entry| entry["decision"].as_str().unwrap_or("use_cloud"))
            .collect();
        if choices.len() > 1 {
            return Err("CLOUD_DECISION_CONFLICTING_PASSAGE".into());
        }
        updated["passage"] = if choices.contains("use_local") {
            local["passage"].clone()
        } else {
            cloud["passage"].clone()
        };
    }
    if updated == current {
        return Ok(0);
    }
    let previous: std::collections::BTreeSet<_> =
        crate::cloud_adoption::editor_runtime_errors(&current)
            .into_iter()
            .map(|i| (i.code, i.target_id))
            .collect();
    let introduced: Vec<_> = crate::cloud_adoption::editor_runtime_errors(&updated)
        .into_iter()
        .filter(|i| !previous.contains(&(i.code.clone(), i.target_id.clone())))
        .map(|i| format!("{}:{}", i.code, i.target_id))
        .collect();
    if !introduced.is_empty() {
        return Err(format!(
            "CLOUD_DECISION_STRUCTURE_INVALID:{}",
            introduced.join(",")
        ));
    }
    // Internal command captures the complete before/after document delta for whole-run undo.
    commands.push(json!({"op":"replaceAuthoringDocument","authoring":updated}));
    let first = std::cell::Cell::new(true);
    let input = ApplyEditorCommandsInput {
        item_id: request.item_id.into(),
        base_version,
        request_id: Some(format!(
            "cloud-decision:{}:{call_id}",
            request.repair_run_id
        )),
        commands,
        title: None,
    };
    repository::apply_editor_commands_tx_with(
        &mut conn,
        &input,
        EditOrigin::CloudRepair,
        Some(request.repair_run_id),
        &|doc, _| {
            if first.replace(false) {
                *doc = updated.clone();
            }
            Ok(())
        },
        &|doc| {
            crate::authoring_v2_commands::refresh_quality_report(
                request.root,
                request.item_id,
                doc,
            )?;
            crate::authoring_v2_commands::validate_authoring(doc)
        },
        &|_, _| Ok(()),
    )?;
    Ok(selected.len() + independent.len() + usize::from(select_passage))
}

pub(super) fn validate_unit_choices(
    request: &RepairRunRequest<'_>,
    entries: &[Value],
) -> CommandResult<()> {
    if entries
        .iter()
        .filter(|e| matches!(e["choice"].as_str(), Some("Cloud" | "Local")))
        .count()
        < 2
    {
        return Ok(());
    }
    let local =
        store::read_local_authoring_snapshot(request.root, request.job_id, request.batch_id)?
            .ok_or("CLOUD_LOCAL_SNAPSHOT_MISSING")?;
    let cloud =
        store::read_cloud_authoring_candidate(request.root, request.job_id, request.batch_id)?
            .ok_or("CLOUD_CANDIDATE_MISSING")?;
    let cloud = serde_json::to_value(cloud.authoring).map_err(|e| e.to_string())?;
    fn has(value: &Value, target: &str) -> bool {
        match value {
            Value::String(s) => s == target,
            Value::Array(a) => a.iter().any(|v| has(v, target)),
            Value::Object(m) => m.iter().any(|(k, v)| k == target || has(v, target)),
            _ => false,
        }
    }
    let conn = repository::open_library_connection(request.root)?;
    let current = repository::get_canonical_ds(&conn, request.item_id)?
        .ok_or("ITEM_DS_NOT_SEEDED")?
        .0;
    for unit in crate::cloud_adoption::comparison_units(&local, &cloud) {
        let a = crate::cloud_adoption::unit_document(&local, &unit.local_task_ids);
        let b = crate::cloud_adoption::unit_document(&cloud, &unit.cloud_task_ids);
        let choices: std::collections::BTreeSet<_> = entries
            .iter()
            .filter(|entry| {
                let target = entry["targetId"].as_str().unwrap_or("");
                has(&a["taskGroups"], target)
                    || has(&a["answerSlots"], target)
                    || has(&b["taskGroups"], target)
                    || has(&b["answerSlots"], target)
                    || entry["comparisonUnitId"].as_str() == Some(unit.unit_id.as_str())
            })
            .filter_map(|entry| {
                entry["choice"]
                    .as_str()
                    .filter(|choice| matches!(*choice, "Cloud" | "Local"))
            })
            .collect();
        if choices.len() > 1
            && entries
                .iter()
                .filter(|entry| {
                    let target = entry["targetId"].as_str().unwrap_or("");
                    has(&a["taskGroups"], target)
                        || has(&a["answerSlots"], target)
                        || has(&b["taskGroups"], target)
                        || has(&b["answerSlots"], target)
                })
                .any(|entry| !independent_field(&local, &cloud, &current, entry))
        {
            return Err(format!("CLOUD_DECISION_CONFLICTING_UNIT:{}", unit.unit_id));
        }
    }
    Ok(())
}
