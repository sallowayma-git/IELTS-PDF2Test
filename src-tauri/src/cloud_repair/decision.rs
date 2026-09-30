//! Backend-constructed, dependency-closed cloud choices. Model output never supplies a document.
use super::RepairRunRequest;
use crate::{
    library::repository::{self, ApplyEditorCommandsInput, EditOrigin},
    reconcile::store,
    CommandResult,
};
use serde_json::{json, Value};

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
    let selected: Vec<_> = units
        .iter()
        .filter(|unit| {
            entries.iter().any(|entry| {
                owns(unit, entry["targetId"].as_str().unwrap_or(""))
                    || entry["comparisonUnitId"].as_str() == Some(unit.unit_id.as_str())
            })
        })
        .collect();
    let select_passage = entries
        .iter()
        .any(|entry| entry["targetType"] == "passage" || entry["targetId"] == "passage");
    if selected.is_empty() && !select_passage {
        return Err("CLOUD_DECISION_UNIT_MISSING".into());
    }
    let mut updated = current.clone();
    let mut commands = Vec::new();
    for unit in &selected {
        updated =
            crate::cloud_adoption::replace_unit(&updated, &local, &cloud, &unit.unit_id, true)?;
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
        updated["passage"] = cloud["passage"].clone();
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
    Ok(selected.len() + usize::from(select_passage))
}
