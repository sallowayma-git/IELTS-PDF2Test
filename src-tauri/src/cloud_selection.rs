//! Explicit author choices between complete, dependency-closed recognition units.
use crate::{
    cloud_adoption,
    library::repository::{self, ApplyEditorCommandsInput, EditOrigin},
    reconcile::store,
    CommandResult,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};

fn sources(root: &Path, job: &str, batch: &str) -> CommandResult<Option<(Value, Value)>> {
    let Some(local) = store::read_local_authoring_snapshot(root, job, batch)? else {
        return Ok(None);
    };
    let Some(cloud) = store::read_cloud_authoring_candidate(root, job, batch)? else {
        return Ok(None);
    };
    Ok(Some((
        local,
        serde_json::to_value(cloud.authoring).map_err(|e| e.to_string())?,
    )))
}

fn digest(document: &Value, ids: &[String]) -> String {
    let slice = cloud_adoption::unit_document(document, ids);
    let relevant = json!({"taskGroups": slice.get("taskGroups"), "answerSlots": slice.get("answerSlots"), "answerKey": slice.get("answerKey")});
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&relevant).unwrap_or_default())
    )
}

fn new_runtime_errors(before: &Value, after: &Value) -> Vec<String> {
    let previous: BTreeSet<_> = cloud_adoption::editor_runtime_errors(before)
        .into_iter()
        .map(|i| (i.code, i.target_id))
        .collect();
    cloud_adoption::editor_runtime_errors(after)
        .into_iter()
        .filter(|i| !previous.contains(&(i.code.clone(), i.target_id.clone())))
        .map(|i| format!("{}:{}", i.code, i.target_id))
        .collect()
}

fn target_in(document: &Value, task_ids: &[String], target: &str) -> bool {
    let slice = cloud_adoption::unit_document(document, task_ids);
    fn contains(value: &Value, target: &str) -> bool {
        match value {
            Value::String(s) => s == target,
            Value::Array(a) => a.iter().any(|v| contains(v, target)),
            Value::Object(o) => o.iter().any(|(k, v)| k == target || contains(v, target)),
            _ => false,
        }
    }
    // Restrict to group-owned material, not the retained whole passage/assets.
    contains(&slice["taskGroups"], target) || contains(&slice["answerSlots"], target)
}

pub(crate) fn enrich_tasks(
    root: &Path,
    item: &str,
    job: &str,
    batch: &str,
    tasks: Vec<Value>,
) -> CommandResult<Vec<Value>> {
    let Some((local, cloud)) = sources(root, job, batch)? else {
        return Ok(tasks);
    };
    let conn = repository::open_library_connection(root)?;
    let Some((current, _)) = repository::get_canonical_ds(&conn, item)? else {
        return Ok(tasks);
    };
    let repair = store::read_batch_repair(&conn, batch)?.unwrap_or_else(|| json!({}));
    let units = cloud_adoption::comparison_units(&local, &cloud);
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for mut task in tasks {
        if task["action"] != "review_difference" {
            result.push(task);
            continue;
        }
        let targets: Vec<&str> = task["targetIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let Some(unit) = units.iter().find(|u| {
            targets.iter().any(|id| {
                target_in(&local, &u.local_task_ids, id) || target_in(&cloud, &u.cloud_task_ids, id)
            })
        }) else {
            result.push(task);
            continue;
        };
        let ids: Vec<String> = unit
            .local_task_ids
            .iter()
            .chain(&unit.cloud_task_ids)
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let chosen = &repair["selectedComparisons"][&unit.unit_id];
        if chosen["digest"].as_str() == Some(digest(&current, &ids).as_str()) {
            continue;
        }
        if !seen.insert(unit.unit_id.clone()) {
            continue;
        }
        task["comparisonUnitId"] = json!(unit.unit_id);
        task["taskIds"] = json!(ids);
        task["localCandidate"] = cloud_adoption::unit_document(&local, &unit.local_task_ids);
        task["cloudCandidate"] = cloud_adoption::unit_document(&cloud, &unit.cloud_task_ids);
        fn collect_pages(value: &Value, pages: &mut BTreeSet<u64>) {
            match value {
                Value::Object(object) => {
                    if let Some(page) = object.get("pageIndex").and_then(Value::as_u64) {
                        pages.insert(page + 1);
                    }
                    for child in object.values() {
                        collect_pages(child, pages);
                    }
                }
                Value::Array(items) => {
                    for child in items {
                        collect_pages(child, pages);
                    }
                }
                _ => {}
            }
        }
        let mut pages = BTreeSet::new();
        collect_pages(&task["localCandidate"]["taskGroups"], &mut pages);
        collect_pages(&task["cloudCandidate"]["taskGroups"], &mut pages);
        task["sourcePages"] = json!(pages.into_iter().collect::<Vec<_>>());
        for (choice, key) in [(false, "localSelectable"), (true, "cloudSelectable")] {
            let valid =
                cloud_adoption::replace_unit(&current, &local, &cloud, &unit.unit_id, choice)
                    .is_ok_and(|next| new_runtime_errors(&current, &next).is_empty());
            task[key] = json!(valid);
        }
        task["decisionStatus"] = json!(if task["contextInsufficient"] == true {
            "need_context"
        } else {
            "user_choice"
        });
        result.push(task);
    }
    Ok(result)
}

pub(crate) fn choose(
    root: &Path,
    item: &str,
    unit_id: &str,
    choice: &str,
    base_version: i64,
) -> CommandResult<Value> {
    if choice != "local" && choice != "cloud" {
        return Err("COMPARISON_CHOICE_INVALID".into());
    }
    let mut conn = repository::open_library_connection(root)?;
    let batch = store::load_latest_batch(&conn, item)?.ok_or("COMPARISON_BATCH_MISSING")?;
    let (local, cloud) =
        sources(root, &batch.job_id, &batch.batch_id)?.ok_or("COMPARISON_SOURCES_MISSING")?;
    let units = cloud_adoption::comparison_units(&local, &cloud);
    let unit = units
        .iter()
        .find(|u| u.unit_id == unit_id)
        .ok_or("COMPARISON_UNIT_MISSING")?;
    let (current, version) =
        repository::get_canonical_ds(&conn, item)?.ok_or("ITEM_DS_NOT_SEEDED")?;
    let ids: Vec<String> = unit
        .local_task_ids
        .iter()
        .chain(&unit.cloud_task_ids)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let repair = store::read_batch_repair(&conn, &batch.batch_id)?.unwrap_or_else(|| json!({}));
    let old = &repair["selectedComparisons"][unit_id];
    if old["choice"] == choice && old["digest"].as_str() == Some(digest(&current, &ids).as_str()) {
        return Ok(json!({"editVersion":version,"replayed":true}));
    }
    if version != base_version {
        return Err(format!(
            "EDIT_VERSION_CONFLICT:current={version}:base={base_version}"
        ));
    }
    let updated =
        cloud_adoption::replace_unit(&current, &local, &cloud, unit_id, choice == "cloud")?;
    let introduced = new_runtime_errors(&current, &updated);
    if !introduced.is_empty() {
        return Err(format!(
            "COMPARISON_STRUCTURE_INVALID:{}",
            introduced.join(",")
        ));
    }
    // Commands describe the entire footprint; content itself is constructed from trusted snapshots.
    let mut footprint_ids: BTreeSet<String> = ids.iter().cloned().collect();
    for document in [&current, &updated] {
        for group in document["taskGroups"].as_array().into_iter().flatten() {
            if let Some(id) = group["taskId"].as_str() {
                let before = current["taskGroups"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|g| g["taskId"] == id);
                let after = updated["taskGroups"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|g| g["taskId"] == id);
                if before != after {
                    footprint_ids.insert(id.to_string());
                }
            }
        }
    }
    let mut commands: Vec<Value> = footprint_ids.iter().map(|id| json!({"op":"upsertTaskGroupBundle", "taskGroup":{"taskId":id}, "comparisonUnitId":unit_id, "choice":choice})).collect();
    // New slot IDs must be protected too: the group footprint is computed against
    // the pre-choice document, where those slots may not exist yet.
    let mut changed_slots = BTreeSet::new();
    for document in [&current, &updated] {
        for key in ["answerSlots", "answerKey"] {
            for id in document[key]
                .as_object()
                .into_iter()
                .flat_map(|map| map.keys())
            {
                if current[key].get(id) != updated[key].get(id) {
                    changed_slots.insert(id.clone());
                }
            }
        }
    }
    commands.extend(changed_slots.into_iter().map(|slot_id| json!({"op":"setAnswer", "slotId":slot_id, "comparisonUnitId":unit_id, "choice":choice})));
    let input = ApplyEditorCommandsInput {
        item_id: item.into(),
        base_version,
        request_id: Some(format!(
            "comparison:{}:{unit_id}:{choice}:{base_version}",
            batch.batch_id
        )),
        commands,
        title: None,
    };
    let first = std::cell::Cell::new(true);
    let result = repository::apply_editor_commands_tx_with(
        &mut conn,
        &input,
        EditOrigin::Human,
        None,
        &|doc, _| {
            if first.replace(false) {
                *doc = updated.clone();
            }
            Ok(())
        },
        &|doc| {
            crate::authoring_v2_commands::refresh_quality_report(root, &batch.job_id, doc)?;
            crate::authoring_v2_commands::validate_authoring(doc)
        },
        &|tx, _| {
            let latest = store::load_latest_batch(tx, item)?.ok_or("COMPARISON_BATCH_MISSING")?;
            if latest.batch_id != batch.batch_id {
                return Err("COMPARISON_BATCH_STALE".into());
            }
            let (saved, _) = repository::get_canonical_ds(tx, item)?.ok_or("ITEM_DS_NOT_SEEDED")?;
            let mut report =
                store::read_batch_repair(tx, &batch.batch_id)?.unwrap_or_else(|| json!({}));
            if !report["selectedComparisons"].is_object() {
                report["selectedComparisons"] = json!({});
            }
            report["selectedComparisons"][unit_id] =
                json!({"choice":choice,"digest":digest(&saved,&ids)});
            store::write_batch_repair(tx, &batch.batch_id, &report)
        },
    )?;
    // Remaining tasks are recomputed by the normal read command after this committed write.
    serde_json::to_value(result).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap()
    }
    fn setup() -> (std::path::PathBuf, Value, Value, String) {
        let root = std::env::temp_dir().join(format!("cloud-selection-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let local = fixture();
        let mut cloud = local.clone();
        cloud["taskGroups"][0]["instructions"][0]["children"][0]["text"] =
            json!("Cloud corrected instructions");
        let conn = repository::open_library_connection(&root).unwrap();
        conn.execute("INSERT INTO library_items_v2 (id,modality,title,status,canonical_ds_json,created_at,updated_at) VALUES ('job-1','reading','Fixture','action_required',?1,'now','now')",[local.to_string()]).unwrap();
        conn.execute("INSERT INTO recognition_batches_v1 (batch_id,library_item_id,job_id,base_edit_version,created_at,updated_at) VALUES ('batch-1','job-1','job-1',1,'now','now')",[]).unwrap();
        store::write_local_authoring_snapshot(&root, "batch-1", "job-1", 1, "sha", &local).unwrap();
        let candidate = serde_json::from_value(json!({"schemaVersion":"CloudAuthoringCandidateV1","batchId":"batch-1","itemId":"job-1","jobId":"job-1","sourceFileId":"source","sourceSha256":"sha","baseEditVersion":1,"generatedAt":"now","status":"succeeded","authoring":cloud,"idMap":{},"unresolvedReferences":[],"unresolvedRegions":[],"sourceCoverageNotes":[],"warnings":[]})).unwrap();
        store::write_cloud_authoring_candidate(&root, "batch-1", &candidate).unwrap();
        let unit = cloud_adoption::comparison_units(&local, &cloud)
            .into_iter()
            .find(|u| {
                u.local_task_ids.contains(
                    &local["taskGroups"][0]["taskId"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                )
            })
            .unwrap()
            .unit_id;
        (root, local, cloud, unit)
    }
    #[test]
    fn selection_validation_ignores_missing_answers_but_rejects_new_dangling_references() {
        let current = fixture();
        let mut missing_answer = current.clone();
        missing_answer["answerKey"] = json!({});
        assert!(
            new_runtime_errors(&current, &missing_answer).is_empty(),
            "{:?}",
            new_runtime_errors(&current, &missing_answer)
        );
        let mut broken = current.clone();
        broken["taskGroups"][0]["responseGroups"][0]["slotIds"]
            .as_array_mut()
            .unwrap()
            .push(json!("missing-slot"));
        assert!(!new_runtime_errors(&current, &broken).is_empty());
    }
    #[test]
    fn publication_gate_is_postponed_until_the_explicit_choice_is_settled() {
        let (root, local, _, unit) = setup();
        let check = |doc: &Value, scope| {
            crate::authoring_validation::publish_verdict(&root, "job-1", doc, Some(1), scope)
                .reasons()
                .iter()
                .any(|reason| reason.code == "RECOGNITION_CHOICE_UNRESOLVED")
        };
        assert!(check(
            &local,
            crate::authoring_validation::PublishScope::CanonicalDirect
        ));
        assert!(check(
            &local,
            crate::authoring_validation::PublishScope::FullDerived
        ));
        choose(&root, "job-1", &unit, "cloud", 1).unwrap();
        let conn = repository::open_library_connection(&root).unwrap();
        let (chosen, _) = repository::get_canonical_ds(&conn, "job-1")
            .unwrap()
            .unwrap();
        assert!(!check(
            &chosen,
            crate::authoring_validation::PublishScope::CanonicalDirect
        ));
        assert!(!check(
            &chosen,
            crate::authoring_validation::PublishScope::FullDerived
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn explicit_choice_is_atomic_versioned_idempotent_and_human_protected() {
        let (root, local, cloud, unit) = setup();
        assert!(choose(&root, "job-1", &unit, "cloud", 0)
            .unwrap_err()
            .starts_with("EDIT_VERSION_CONFLICT"));
        let result = choose(&root, "job-1", &unit, "cloud", 1).unwrap();
        assert_eq!(result["editVersion"], 2);
        let replay = choose(&root, "job-1", &unit, "cloud", 1).unwrap();
        assert_eq!(replay["replayed"], true);
        let conn = repository::open_library_connection(&root).unwrap();
        let (saved, version) = repository::get_canonical_ds(&conn, "job-1")
            .unwrap()
            .unwrap();
        assert_eq!(version, 2);
        assert_eq!(
            saved["taskGroups"][0]["instructions"][0]["children"][0]["text"],
            cloud["taskGroups"][0]["instructions"][0]["children"][0]["text"]
        );
        assert_ne!(
            saved["taskGroups"][0]["instructions"],
            local["taskGroups"][0]["instructions"]
        );
        assert!(!repository::human_protected_targets(&conn, "job-1", &saved)
            .unwrap()
            .is_empty());
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn chosen_unit_disappears_but_quality_problem_survives() {
        let (root, local, _, unit) = setup();
        let task = json!({"userTaskId":"difference","targetIds":[local["taskGroups"][0]["taskId"]],"action":"review_difference"});
        let before = enrich_tasks(&root, "job-1", "job-1", "batch-1", vec![task.clone()]).unwrap();
        assert_eq!(before[0]["comparisonUnitId"], unit);
        choose(&root, "job-1", &unit, "local", 1).unwrap();
        let quality = json!({"userTaskId":"quality","targetIds":[local["taskGroups"][0]["taskId"]],"action":"fix_structure"});
        let after = enrich_tasks(
            &root,
            "job-1",
            "job-1",
            "batch-1",
            vec![task, quality.clone()],
        )
        .unwrap();
        assert_eq!(after, vec![quality]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
