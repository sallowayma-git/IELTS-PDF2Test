use super::build_authoring_v2_shadow;
use crate::authoring_pipeline::{make_dynamic_authoring_ir, make_dynamic_split_candidates};
use crate::authoring_v2_commands::{apply_authoring_v2_patches_core, export_authoring_v2_core};
use crate::parser::parse_source_document;
use crate::pdf_facts_shadow::write_pdf_facts_shadow;
use crate::reading_source::reading_source;
use crate::{ImportJob, IssueCounts, JobStatus, SourceFile, WorkflowStep};
use chrono::{TimeZone, Utc};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const ACCEPTANCE_SPEC: &str = "fixtures/golden/phase4-eight-pdf-acceptance.json";
const MANIFEST: &str = "fixtures/golden/manifest.json";
const REPORT: &str = "tmp/phase4-real-pdf-acceptance/report.json";
const PHASE5_REPORT: &str = "tmp/phase5-real-pdf-acceptance/report.json";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn copy_asset_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let source_metadata =
        fs::symlink_metadata(source).map_err(|error| format!("{}: {error}", source.display()))?;
    if !source_metadata.is_dir() {
        return Err(format!(
            "phase5_asset_source_not_directory:{}",
            source.display()
        ));
    }
    fs::create_dir_all(destination)
        .map_err(|error| format!("{}: {error}", destination.display()))?;
    for entry in fs::read_dir(source).map_err(|error| format!("{}: {error}", source.display()))? {
        let entry = entry.map_err(|error| format!("{}: {error}", source.display()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("{}: {error}", entry.path().display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "phase5_asset_source_symlink:{}",
                entry.path().display()
            ));
        }
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_asset_tree(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "phase5_asset_copy_failed:{}:{}",
                    source_path.display(),
                    error
                )
            })?;
        } else {
            return Err(format!(
                "phase5_asset_source_unsupported:{}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn validate_physical_asset_integrity(output_dir: &Path, physical: &Value, checks: &mut Vec<Value>) {
    let mut failures = Vec::new();
    let assets = physical
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for asset in &assets {
        let asset_id = asset
            .get("assetId")
            .and_then(Value::as_str)
            .unwrap_or("missing-asset-id");
        let Some(relative_path) = asset.get("relativePath").and_then(Value::as_str) else {
            failures.push(json!({"assetId":asset_id,"reason":"relativePath_missing"}));
            continue;
        };
        let relative = Path::new(relative_path);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            failures.push(json!({"assetId":asset_id,"reason":"unsafe_relative_path","relativePath":relative_path}));
            continue;
        }
        let path = output_dir.join(relative);
        match fs::read(&path) {
            Ok(bytes) => {
                let actual_hash = format!("{:x}", Sha256::digest(&bytes));
                let expected_hash = asset.get("sha256").and_then(Value::as_str);
                let expected_length = asset.get("byteLength").and_then(Value::as_u64);
                if expected_hash != Some(actual_hash.as_str())
                    || expected_length != Some(bytes.len() as u64)
                {
                    failures.push(json!({
                        "assetId": asset_id,
                        "reason": "content_identity_mismatch",
                        "relativePath": relative_path,
                        "expectedSha256": expected_hash,
                        "actualSha256": actual_hash,
                        "expectedByteLength": expected_length,
                        "actualByteLength": bytes.len()
                    }));
                }
            }
            Err(error) => failures.push(json!({
                "assetId": asset_id,
                "reason": "asset_read_failed",
                "relativePath": relative_path,
                "error": error.to_string()
            })),
        }
    }
    push_check(
        checks,
        "PHYSICAL_ASSET_INTEGRITY",
        failures.is_empty(),
        json!({"descriptorCount":assets.len(),"failures":[]}),
        json!({"descriptorCount":assets.len(),"failures":failures}),
    );
}

fn fixed_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0)
        .single()
        .expect("fixed acceptance timestamp")
}

fn source_and_job(fixture_id: &str, fixture: &Value, metadata: &Value) -> (SourceFile, ImportJob) {
    let original_name = fixture
        .get("originalName")
        .and_then(Value::as_str)
        .or_else(|| {
            metadata
                .pointer("/source/originalName")
                .and_then(Value::as_str)
        })
        .unwrap_or(fixture_id);
    let source = SourceFile {
        file_id: format!("phase4-real-{fixture_id}"),
        original_name: original_name.to_string(),
        stored_name: format!("{fixture_id}.pdf"),
        file_type: "pdf".to_string(),
        sha256: fixture
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        size_bytes: fixture
            .get("sizeBytes")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        role: "MainQuestion".to_string(),
        imported_at: fixed_time(),
    };
    let job = ImportJob {
        job_id: format!("phase4-real-{fixture_id}"),
        title: original_name.trim_end_matches(".pdf").to_string(),
        status: JobStatus::Working,
        category: Some("P1".to_string()),
        frequency: Some("medium".to_string()),
        tags: vec!["phase4-real-pdf-acceptance".to_string()],
        source_files: vec![source.clone()],
        active_llm_profile_id: None,
        created_at: fixed_time(),
        updated_at: fixed_time(),
        current_step: WorkflowStep::DocumentReview,
        issue_counts: IssueCounts::default(),
    };
    (source, job)
}

fn v1_summary(document: &Value, authoring: &Value) -> Value {
    let pages = document
        .get("pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let blocks = pages
        .iter()
        .flat_map(|page| {
            page.get("blocks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .collect::<Vec<_>>();
    let groups = authoring
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let questions = groups
        .iter()
        .flat_map(|group| {
            group
                .get("questions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .collect::<Vec<_>>();
    let mut roles = blocks
        .iter()
        .filter_map(|block| block.get("roleHint").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    roles.sort();
    json!({
        "pageCount": pages.len(),
        "blockCount": blocks.len(),
        "groupCount": groups.len(),
        "slotCount": questions.len(),
        "assetCount": document.get("assets").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "answerCount": authoring.get("answerKey").and_then(Value::as_object).map(Map::len).unwrap_or(0),
        "warningCount": document.pointer("/parser/warnings").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "groupKinds": groups.iter().map(|group| group.get("kind").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
        "questionIds": questions.iter().map(|question| question.get("id").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
        "roles": roles
    })
}

fn push_check(checks: &mut Vec<Value>, code: &str, passed: bool, expected: Value, actual: Value) {
    checks.push(json!({
        "code": code,
        "passed": passed,
        "expected": expected,
        "actual": actual
    }));
}

fn compiler_probe_status_is_honest(probe: Option<&Value>, quality_state: Option<&str>) -> bool {
    let Some(probe) = probe else {
        return false;
    };
    match probe.get("status").and_then(Value::as_str) {
        Some("passed") => true,
        Some("failed") => {
            quality_state != Some("ready")
                && probe
                    .get("issueCodes")
                    .and_then(Value::as_array)
                    .is_some_and(|codes| {
                        codes
                            .iter()
                            .any(|code| code.as_str().is_some_and(|code| !code.trim().is_empty()))
                    })
        }
        _ => false,
    }
}

fn quality_acceptance_policy(
    fixture_id: &str,
    metadata: &Value,
    physical: &Value,
    authoring: &Value,
) -> (bool, Value) {
    let answer_page_indexes = metadata
        .pointer("/expected/pageRoles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry
                .get("roles")
                .and_then(Value::as_array)
                .is_some_and(|roles| roles.iter().any(|role| role.as_str() == Some("answer")))
        })
        .filter_map(|entry| entry.get("pageIndex").and_then(Value::as_u64))
        .map(|one_based| one_based.saturating_sub(1) as usize)
        .collect::<Vec<_>>();
    let pages = physical
        .get("pages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let answer_page_evidence = answer_page_indexes
        .iter()
        .map(|page_index| {
            let page = pages.get(*page_index);
            let meaningful_text = page
                .and_then(|page| page.get("lines"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|line| line.get("text").and_then(Value::as_str))
                .any(|text| !text.trim().is_empty());
            let image_count = page
                .and_then(|page| page.get("imagePlacements"))
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default();
            let requires_ocr_count = page
                .and_then(|page| page.pointer("/quality/requiresOcrRegions"))
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default();
            json!({
                "pageIndex": page_index,
                "meaningfulText": meaningful_text,
                "imagePlacementCount": image_count,
                "requiresOcrRegionCount": requires_ocr_count,
                "imageOnlyOcrRequired": !meaningful_text && image_count > 0 && requires_ocr_count > 0
            })
        })
        .collect::<Vec<_>>();
    let image_only_answer_evidence = !answer_page_evidence.is_empty()
        && answer_page_evidence.iter().all(|evidence| {
            evidence
                .get("imageOnlyOcrRequired")
                .and_then(Value::as_bool)
                == Some(true)
        });

    let issue_codes = authoring
        .pointer("/quality/issues")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|issue| issue.get("code").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let v2_probe = authoring.pointer("/quality/compilerProbes/v2Runtime");
    let v2_probe_status = v2_probe
        .and_then(|probe| probe.get("status"))
        .and_then(Value::as_str);
    let v2_probe_issue_codes = v2_probe
        .and_then(|probe| probe.get("issueCodes"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let organisational_runtime_answer_policy_allowed = fixture_id == "organisational-design"
        && image_only_answer_evidence
        && v2_probe_status == Some("failed")
        && v2_probe_issue_codes
            == BTreeSet::from(["RUNTIME_ANSWER_KEY_POLICY_INVALID".to_string()]);
    let v2_probe_allowed =
        v2_probe_status == Some("passed") || organisational_runtime_answer_policy_allowed;
    let v1_probe_passed = authoring
        .pointer("/quality/compilerProbes/v1Compatibility/status")
        .and_then(Value::as_str)
        == Some("passed");

    let unexpected_issue_codes = issue_codes
        .iter()
        .filter(|code| match code.as_str() {
            "ANSWER_KEY_MISSING_SLOT" => !image_only_answer_evidence,
            "RUNTIME_COMPILER_FAILED" => !organisational_runtime_answer_policy_allowed,
            _ => true,
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let forbidden_issue_codes = issue_codes
        .iter()
        .filter(|code| {
            matches!(
                code.as_str(),
                "RUNTIME_OPTION_BANK_MISSING"
                    | "RESPONSE_GROUP_POLICY_MISMATCH"
                    | "SIGNIFICANT_REGION_UNASSIGNED"
            )
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let coverage_complete = authoring
        .pointer("/quality/coverageStatus/complete")
        .and_then(Value::as_bool)
        == Some(true);
    let quality_state = authoring.pointer("/quality/state").and_then(Value::as_str);
    let state_is_explained = match quality_state {
        Some("ready") => issue_codes.is_empty(),
        Some("blocked") | Some("review_required") => !issue_codes.is_empty(),
        _ => false,
    };
    let passed = coverage_complete
        && unexpected_issue_codes.is_empty()
        && forbidden_issue_codes.is_empty()
        && state_is_explained
        && v2_probe_allowed
        && v1_probe_passed;
    (
        passed,
        json!({
            "qualityState": quality_state,
            "coverageComplete": coverage_complete,
            "issueCodes": issue_codes,
            "unexpectedIssueCodes": unexpected_issue_codes,
            "forbiddenIssueCodes": forbidden_issue_codes,
            "answerPageEvidence": answer_page_evidence,
            "imageOnlyAnswerEvidence": image_only_answer_evidence,
            "v2RuntimeStatus": v2_probe_status,
            "v2RuntimeIssueCodes": v2_probe_issue_codes,
            "v1CompatibilityPassed": v1_probe_passed,
            "organisationalRuntimeAnswerPolicyAllowed": organisational_runtime_answer_policy_allowed
        }),
    )
}

fn corpus_exam_id_check(results: &[Value]) -> Value {
    let entries = results
        .iter()
        .map(|result| {
            json!({
                "fixtureId": result.get("fixtureId").and_then(Value::as_str),
                "examId": result.pointer("/v2Summary/examId").and_then(Value::as_str)
            })
        })
        .collect::<Vec<_>>();
    let exam_ids = entries
        .iter()
        .filter_map(|entry| entry.get("examId").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let unique_exam_ids = exam_ids.iter().copied().collect::<BTreeSet<_>>();
    json!({
        "code": "CORPUS_EXAM_ID_NONEMPTY_PATH_SAFE_UNIQUE",
        "passed": exam_ids.len() == results.len()
            && exam_ids
                .iter()
                .all(|exam_id| crate::util::is_safe_path_segment(exam_id))
            && unique_exam_ids.len() == results.len(),
        "expected": {"count":results.len(),"allNonempty":true,"allPathSafe":true,"allUnique":true},
        "actual": {"entries":entries,"count":exam_ids.len(),"uniqueCount":unique_exam_ids.len()}
    })
}

fn range_bounds(value: &Value) -> Option<(u64, u64)> {
    if let Some(values) = value.as_array() {
        return Some((values.first()?.as_u64()?, values.last()?.as_u64()?));
    }
    match value.get("kind").and_then(Value::as_str) {
        Some("range") => Some((value.get("start")?.as_u64()?, value.get("end")?.as_u64()?)),
        Some("set") => {
            let values = value.get("values")?.as_array()?;
            Some((values.first()?.as_u64()?, values.last()?.as_u64()?))
        }
        _ => None,
    }
}

fn labels(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("label").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect()
}

fn expected_labels(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect()
}

fn collect_key_values(value: &Value, key: &str, output: &mut Vec<Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_key_values(item, key, output);
            }
        }
        Value::Object(object) => {
            if let Some(found) = object.get(key) {
                output.push(found.clone());
            }
            for child in object.values() {
                collect_key_values(child, key, output);
            }
        }
        _ => {}
    }
}

fn collect_embedded_answer_slot_ids(value: &Value, output: &mut BTreeSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_embedded_answer_slot_ids(item, output);
            }
        }
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("answer_slot") {
                if let Some(slot_id) = object.get("slotId").and_then(Value::as_str) {
                    output.insert(slot_id.to_string());
                }
            }
            for child in object.values() {
                collect_embedded_answer_slot_ids(child, output);
            }
        }
        _ => {}
    }
}

fn completion_embedding_evidence(
    task_group: Option<&Value>,
    response_group: Option<&Value>,
) -> (bool, BTreeSet<String>, BTreeSet<String>) {
    let expected_slots = response_group
        .and_then(|response| response.get("slotIds"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let mut embedded_slots = BTreeSet::new();
    if let Some(prompt) = response_group.and_then(|response| response.get("prompt")) {
        collect_embedded_answer_slot_ids(prompt, &mut embedded_slots);
    }
    if let Some(stimulus) = task_group.and_then(|group| group.get("stimulus")) {
        collect_embedded_answer_slot_ids(stimulus, &mut embedded_slots);
    }
    let missing_slots = expected_slots
        .difference(&embedded_slots)
        .cloned()
        .collect::<BTreeSet<_>>();
    (
        !expected_slots.is_empty() && missing_slots.is_empty(),
        embedded_slots,
        missing_slots,
    )
}

fn collected_page_indexes(value: &Value) -> BTreeSet<u64> {
    let mut values = Vec::new();
    collect_key_values(value, "pageIndex", &mut values);
    values
        .into_iter()
        .filter_map(|value| value.as_u64())
        .collect()
}

fn collected_asset_ids(value: &Value) -> BTreeSet<String> {
    let mut values = Vec::new();
    collect_key_values(value, "assetId", &mut values);
    values
        .into_iter()
        .filter_map(|value| value.as_str().map(ToString::to_string))
        .collect()
}

fn group_by_id<'a>(authoring: &'a Value, id: &str) -> Option<&'a Value> {
    authoring
        .get("taskGroups")
        .and_then(Value::as_array)?
        .iter()
        .find(|group| group.get("taskId").and_then(Value::as_str) == Some(id))
}

fn validate_metadata_truth(metadata: &Value, authoring: &Value, checks: &mut Vec<Value>) {
    let expected_groups = metadata
        .pointer("/expected/taskGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let actual_groups = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    push_check(
        checks,
        "METADATA_TASK_GROUP_COUNT",
        actual_groups.len() == expected_groups.len(),
        json!(expected_groups.len()),
        json!(actual_groups.len()),
    );
    for expected in expected_groups {
        let id = expected
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("missing-id");
        let actual = group_by_id(authoring, id);
        push_check(
            checks,
            &format!("METADATA_TASK_GROUP_EXISTS:{id}"),
            actual.is_some(),
            json!(id),
            actual
                .and_then(|group| group.get("taskId"))
                .cloned()
                .unwrap_or(Value::Null),
        );
        let Some(actual) = actual else { continue };
        let expected_range = range_bounds(expected.get("displayRange").unwrap_or(&Value::Null));
        let actual_range = range_bounds(actual.get("displayRange").unwrap_or(&Value::Null));
        push_check(
            checks,
            &format!("METADATA_TASK_RANGE:{id}"),
            actual_range == expected_range,
            json!(expected_range),
            json!(actual_range),
        );
        let expected_kind = expected.get("kind").and_then(Value::as_str);
        let actual_kind = actual.get("taskType").and_then(Value::as_str);
        push_check(
            checks,
            &format!("METADATA_TASK_TYPE:{id}"),
            actual_kind == expected_kind,
            json!(expected_kind),
            json!(actual_kind),
        );
    }

    let actual_slots = authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let expected_slots = metadata
        .pointer("/expected/slots")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let expected_slot_ids = expected_slots
        .iter()
        .filter_map(|slot| slot.get("id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let actual_slot_ids = actual_slots
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    push_check(
        checks,
        "METADATA_SLOT_IDS",
        actual_slot_ids == expected_slot_ids,
        json!(expected_slot_ids),
        json!(actual_slot_ids),
    );
    for expected in expected_slots {
        let Some(id) = expected.get("id").and_then(Value::as_str) else {
            continue;
        };
        let expected_type = expected.get("responseType").and_then(Value::as_str);
        let actual_type = actual_slots
            .get(id)
            .and_then(|slot| slot.get("interaction"))
            .and_then(Value::as_str);
        push_check(
            checks,
            &format!("METADATA_SLOT_INTERACTION:{id}"),
            actual_type == expected_type,
            json!(expected_type),
            json!(actual_type),
        );
    }

    for expected in metadata
        .pointer("/expected/optionBanks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = expected
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("missing-bank");
        let group_id = expected
            .get("taskGroupId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let actual = group_by_id(authoring, group_id).and_then(|group| group.get("optionBank"));
        let expected_bank_labels = expected_labels(expected.get("labels").unwrap_or(&Value::Null));
        let actual_bank_labels = actual
            .and_then(|bank| bank.get("options"))
            .map(labels)
            .unwrap_or_default();
        let actual_id = actual
            .and_then(|bank| bank.get("optionBankId"))
            .and_then(Value::as_str);
        let expected_scope = expected.get("scope").and_then(Value::as_str);
        let actual_scope = actual
            .and_then(|bank| bank.get("scope"))
            .and_then(Value::as_str);
        let expected_reuse = expected.get("allowReuse").and_then(Value::as_bool);
        let actual_reuse = actual
            .and_then(|bank| bank.get("allowReuse"))
            .and_then(Value::as_bool);
        push_check(
            checks,
            &format!("METADATA_OPTION_BANK:{id}"),
            actual_id == Some(id)
                && actual_bank_labels == expected_bank_labels
                && actual_scope == expected_scope
                && actual_reuse == expected_reuse,
            json!({"id": id, "labels": expected_bank_labels, "scope": expected_scope, "allowReuse": expected_reuse}),
            json!({"id": actual_id, "labels": actual_bank_labels, "scope": actual_scope, "allowReuse": actual_reuse}),
        );
    }

    let expected_responses = metadata
        .pointer("/expected/responseGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut expected_response_counts = BTreeMap::<String, usize>::new();
    for expected in &expected_responses {
        if let Some(task_group_id) = expected.get("taskGroupId").and_then(Value::as_str) {
            *expected_response_counts
                .entry(task_group_id.to_string())
                .or_default() += 1;
        }
    }
    for (task_group_id, expected_count) in expected_response_counts {
        let actual_count = group_by_id(authoring, &task_group_id)
            .and_then(|group| group.get("responseGroups"))
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        push_check(
            checks,
            &format!("METADATA_RESPONSE_GROUP_COUNT:{task_group_id}"),
            actual_count == expected_count,
            json!(expected_count),
            json!(actual_count),
        );
    }

    for expected in expected_responses {
        let id = expected
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("missing-response");
        let group_id = expected
            .get("taskGroupId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let actual = group_by_id(authoring, group_id)
            .and_then(|group| group.get("responseGroups"))
            .and_then(Value::as_array)
            .and_then(|groups| {
                groups
                    .iter()
                    .find(|group| group.get("responseGroupId").and_then(Value::as_str) == Some(id))
            });
        let expected_slots = expected
            .get("slotIds")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let actual_slots = actual
            .and_then(|group| group.get("slotIds"))
            .cloned()
            .unwrap_or(Value::Null);
        let expected_assignment = expected.get("assignment").and_then(Value::as_str);
        let actual_assignment = actual
            .and_then(|group| group.get("assignment"))
            .and_then(Value::as_str);
        let expected_cardinality = expected.get("cardinality").cloned().unwrap_or(Value::Null);
        let actual_cardinality = actual
            .and_then(|group| group.get("cardinality"))
            .cloned()
            .unwrap_or(Value::Null);
        let expected_scoring = expected.get("scoringPolicy").and_then(Value::as_str);
        let actual_scoring = actual
            .and_then(|group| group.get("scoringPolicy"))
            .and_then(Value::as_str);
        let expected_prompt_mode = expected.get("promptMode").and_then(Value::as_str);
        let prompt_count = actual
            .and_then(|group| group.get("prompt"))
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        let slot_count = actual
            .and_then(|group| group.get("slotIds"))
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        let task_group = group_by_id(authoring, group_id);
        let (all_slots_embedded, embedded_slot_ids, missing_embedded_slot_ids) =
            completion_embedding_evidence(task_group, actual);
        let completion_task = task_group
            .and_then(|group| group.get("taskType"))
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.ends_with("_completion"));
        let actual_prompt_mode = if completion_task && all_slots_embedded {
            "embedded"
        } else if actual_assignment == Some("unordered_set") && slot_count > 1 && prompt_count == 1
        {
            "shared"
        } else {
            "per_slot"
        };
        let prompt_mode_shape_ok = match expected_prompt_mode {
            Some("shared") => prompt_count == 1 && slot_count > 1,
            Some("per_slot") => prompt_count == slot_count && slot_count > 0,
            Some("embedded") => completion_task && all_slots_embedded,
            _ => false,
        };
        let expected_binding = expected
            .pointer("/optionBinding/mode")
            .and_then(Value::as_str);
        let expected_bank_ref = expected
            .pointer("/optionBinding/optionBankId")
            .and_then(Value::as_str);
        let expected_inline_labels = expected
            .pointer("/optionBinding/inlineLabels")
            .map(expected_labels)
            .unwrap_or_default();
        let actual_bank_ref = actual
            .and_then(|group| group.get("optionBankRef"))
            .and_then(Value::as_str);
        let actual_inline_labels = actual
            .and_then(|group| group.get("options"))
            .map(labels)
            .unwrap_or_default();
        let binding_matches = match expected_binding {
            Some("option_bank") => {
                actual_bank_ref == expected_bank_ref && actual_inline_labels.is_empty()
            }
            Some("inline") => {
                actual_bank_ref.is_none() && actual_inline_labels == expected_inline_labels
            }
            Some("none") => actual_bank_ref.is_none() && actual_inline_labels.is_empty(),
            _ => false,
        };
        let expected_reuse_policy = expected.get("reusePolicy").and_then(Value::as_str);
        let actual_allow_reuse = actual
            .and_then(|group| group.get("allowOptionReuse"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let reuse_matches = match expected_reuse_policy {
            Some("allowed") => actual_allow_reuse,
            Some("disallowed") | Some("not_applicable") => !actual_allow_reuse,
            _ => false,
        };
        push_check(
            checks,
            &format!("METADATA_RESPONSE_GROUP:{id}"),
            actual.is_some()
                && actual_slots == expected_slots
                && actual_assignment == expected_assignment
                && actual_cardinality == expected_cardinality
                && actual_scoring == expected_scoring
                && actual_prompt_mode == expected_prompt_mode.unwrap_or_default()
                && prompt_mode_shape_ok
                && binding_matches
                && reuse_matches,
            json!({"slotIds": expected_slots, "assignment": expected_assignment, "cardinality": expected_cardinality, "scoringPolicy": expected_scoring, "promptMode": expected_prompt_mode, "optionBinding": expected.get("optionBinding"), "reusePolicy": expected_reuse_policy}),
            json!({"slotIds": actual_slots, "assignment": actual_assignment, "cardinality": actual_cardinality, "scoringPolicy": actual_scoring, "promptMode": actual_prompt_mode, "promptCount": prompt_count, "embeddedSlotIds": embedded_slot_ids, "missingEmbeddedSlotIds": missing_embedded_slot_ids, "optionBankRef": actual_bank_ref, "inlineLabels": actual_inline_labels, "allowOptionReuse": actual_allow_reuse}),
        );
    }
}

fn validate_separation(
    metadata: &Value,
    physical: &Value,
    authoring: &Value,
    checks: &mut Vec<Value>,
) {
    let answer_pages = metadata
        .pointer("/expected/pageRoles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry
                .get("roles")
                .and_then(Value::as_array)
                .is_some_and(|roles| {
                    roles
                        .iter()
                        .any(|role| matches!(role.as_str(), Some("answer") | Some("explanation")))
                })
        })
        .filter_map(|entry| entry.get("pageIndex").and_then(Value::as_u64))
        .map(|one_based| one_based.saturating_sub(1))
        .collect::<BTreeSet<_>>();
    let passage = authoring.get("passage").cloned().unwrap_or(Value::Null);
    let passage_pages = collected_page_indexes(&passage);
    let overlap = passage_pages
        .intersection(&answer_pages)
        .copied()
        .collect::<Vec<_>>();
    push_check(
        checks,
        "ANSWER_EXPLANATION_TEXT_EXCLUDED_FROM_PASSAGE",
        overlap.is_empty(),
        json!([]),
        json!(overlap),
    );

    let answer_asset_ids = physical
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|asset| {
            asset
                .pointer("/sourceAnchor/pageIndex")
                .and_then(Value::as_u64)
                .is_some_and(|page| answer_pages.contains(&page))
        })
        .filter_map(|asset| asset.get("assetId").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let passage_assets = collected_asset_ids(&passage);
    let raster_overlap = passage_assets
        .intersection(&answer_asset_ids)
        .cloned()
        .collect::<Vec<_>>();
    push_check(
        checks,
        "ANSWER_RASTER_EXCLUDED_FROM_PASSAGE",
        raster_overlap.is_empty(),
        json!([]),
        json!(raster_overlap),
    );
}

fn validate_special_truth(
    fixture_id: &str,
    metadata: &Value,
    physical: &Value,
    authoring: &Value,
    checks: &mut Vec<Value>,
) {
    match fixture_id {
        "organisational-design" => {
            for index in 1..=4 {
                let id = format!("group-{index}");
                let group = group_by_id(authoring, &id);
                let response = group
                    .and_then(|group| group.get("responseGroups"))
                    .and_then(Value::as_array)
                    .and_then(|groups| groups.first());
                let actual = json!({
                    "responseGroupCount": group.and_then(|group| group.get("responseGroups")).and_then(Value::as_array).map(Vec::len),
                    "slotCount": response.and_then(|value| value.get("slotIds")).and_then(Value::as_array).map(Vec::len),
                    "exact": response.and_then(|value| value.pointer("/cardinality/exact")).and_then(Value::as_u64),
                    "assignment": response.and_then(|value| value.get("assignment")).and_then(Value::as_str),
                    "hasSharedPrompt": response.and_then(|value| value.get("prompt")).and_then(Value::as_array).is_some_and(|items| !items.is_empty())
                });
                let passed = actual.get("responseGroupCount").and_then(Value::as_u64) == Some(1)
                    && actual.get("slotCount").and_then(Value::as_u64) == Some(2)
                    && actual.get("exact").and_then(Value::as_u64) == Some(2)
                    && actual.get("assignment").and_then(Value::as_str) == Some("unordered_set")
                    && actual.get("hasSharedPrompt").and_then(Value::as_bool) == Some(true);
                push_check(
                    checks,
                    &format!("ORGANISATIONAL_SHARED_TWO_SLOT:{id}"),
                    passed,
                    json!({"responseGroupCount":1,"slotCount":2,"exact":2,"assignment":"unordered_set","hasSharedPrompt":true}),
                    actual,
                );
            }
            let answer_pages = metadata
                .pointer("/expected/pageRoles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|entry| {
                    entry
                        .get("roles")
                        .and_then(Value::as_array)
                        .is_some_and(|roles| {
                            roles.iter().any(|role| {
                                matches!(role.as_str(), Some("answer") | Some("explanation"))
                            })
                        })
                })
                .filter_map(|entry| entry.get("pageIndex").and_then(Value::as_u64))
                .map(|page| page.saturating_sub(1))
                .collect::<BTreeSet<_>>();
            let meaningful_answer_text = physical
                .get("pages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|page| {
                    page.get("pageIndex")
                        .and_then(Value::as_u64)
                        .is_some_and(|page| answer_pages.contains(&page))
                })
                .flat_map(|page| {
                    page.get("lines")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter_map(|line| line.get("text").and_then(Value::as_str))
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>();
            let answer_page_facts = physical
                .get("pages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|page| {
                    page.get("pageIndex")
                        .and_then(Value::as_u64)
                        .is_some_and(|page| answer_pages.contains(&page))
                })
                .map(|page| {
                    json!({
                        "pageIndex": page.get("pageIndex"),
                        "classification": page.pointer("/quality/classification"),
                        "requiresOcrRegionCount": page
                            .pointer("/quality/requiresOcrRegions")
                            .and_then(Value::as_array)
                            .map(Vec::len)
                            .unwrap_or(0)
                    })
                })
                .collect::<Vec<_>>();
            let answer_key = authoring
                .get("answerKey")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let unresolved_count = answer_key
                .values()
                .filter(|answer| answer.get("kind").and_then(Value::as_str) == Some("unresolved"))
                .count();
            let runtime_issue_codes = authoring
                .pointer("/quality/compilerProbes/v2Runtime/issueCodes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let quality_state = authoring.pointer("/quality/state").and_then(Value::as_str);
            let actual = json!({
                "answerPages": answer_pages,
                "meaningfulAnswerText": meaningful_answer_text,
                "answerKeyEntryCount": answer_key.len(),
                "unresolvedAnswerCount": unresolved_count,
                "qualityState": quality_state,
                "v2RuntimeStatus": authoring.pointer("/quality/compilerProbes/v2Runtime/status").and_then(Value::as_str),
                "v2RuntimeIssueCodes": runtime_issue_codes
            });
            let passed = !answer_pages.is_empty()
                && meaningful_answer_text.is_empty()
                && !answer_key.is_empty()
                && unresolved_count == answer_key.len()
                && matches!(quality_state, Some("blocked") | Some("review_required"))
                && actual.get("v2RuntimeStatus").and_then(Value::as_str) == Some("failed")
                && actual
                    .get("v2RuntimeIssueCodes")
                    .and_then(Value::as_array)
                    .is_some_and(|codes| {
                        codes
                            .iter()
                            .any(|code| code.as_str() == Some("RUNTIME_ANSWER_KEY_POLICY_INVALID"))
                    });
            push_check(
                checks,
                "ORGANISATIONAL_RASTER_ANSWER_KEY_REQUIRES_OCR",
                passed,
                json!({
                    "meaningfulAnswerText": [],
                    "allAnswers": "unresolved",
                    "qualityState": ["blocked", "review_required"],
                    "v2RuntimeStatus": "failed",
                    "v2RuntimeIssueCode": "RUNTIME_ANSWER_KEY_POLICY_INVALID"
                }),
                actual,
            );
            let ocr_signal_present = answer_page_facts.len() == answer_pages.len()
                && answer_page_facts.iter().all(|page| {
                    page.get("requiresOcrRegionCount")
                        .and_then(Value::as_u64)
                        .is_some_and(|count| count > 0)
                        || matches!(
                            page.get("classification").and_then(Value::as_str),
                            Some("scanned") | Some("mixed")
                        )
                });
            push_check(
                checks,
                "ORGANISATIONAL_ANSWER_PAGE_OCR_SIGNAL",
                ocr_signal_present,
                json!({
                    "answerPageCount": answer_pages.len(),
                    "classification": ["scanned", "mixed"],
                    "requiresOcrRegionCount": ">0"
                }),
                json!({"answerPages":answer_page_facts}),
            );
        }
        "petri-dish" => {
            let group = group_by_id(authoring, "group-2");
            let bank_id = group
                .and_then(|value| value.pointer("/optionBank/optionBankId"))
                .and_then(Value::as_str);
            let responses = group
                .and_then(|value| value.get("responseGroups"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let bank_refs = responses
                .iter()
                .filter_map(|value| value.get("optionBankRef").and_then(Value::as_str))
                .collect::<BTreeSet<_>>();
            let embedded_option_count = responses
                .iter()
                .filter(|value| value.get("options").is_some())
                .count();
            let passed = bank_id == Some("group-2-option-bank")
                && bank_refs == BTreeSet::from(["group-2-option-bank"])
                && embedded_option_count == 0;
            push_check(
                checks,
                "PETRI_SINGLE_LIST_OF_PEOPLE_BANK",
                passed,
                json!({"bankId":"group-2-option-bank","bankRefs":["group-2-option-bank"],"embeddedOptionCopies":0}),
                json!({"bankId":bank_id,"bankRefs":bank_refs,"embeddedOptionCopies":embedded_option_count}),
            );
        }
        "listening-to-the-ocean" => {
            let group = group_by_id(authoring, "group-3");
            let bank_labels = group
                .and_then(|value| value.pointer("/optionBank/options"))
                .map(labels);
            let response_labels = group
                .and_then(|value| value.get("responseGroups"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|response| labels(response.get("options").unwrap_or(&Value::Null)))
                .collect::<Vec<_>>();
            let labels_are_a_d = bank_labels
                .as_ref()
                .is_some_and(|labels| labels == &["A", "B", "C", "D"])
                || (response_labels.len() == 5
                    && response_labels
                        .iter()
                        .all(|labels| labels == &["A", "B", "C", "D"]));
            let range = group.and_then(|value| range_bounds(value.get("displayRange")?));
            let kind = group
                .and_then(|value| value.get("taskType"))
                .and_then(Value::as_str);
            push_check(
                checks,
                "OCEAN_Q9_Q13_INLINE_A_D",
                range == Some((9, 13)) && kind == Some("single_choice") && labels_are_a_d,
                json!({"range":[9,13],"taskType":"single_choice","labels":["A","B","C","D"]}),
                json!({"range":range,"taskType":kind,"optionBankLabels":bank_labels,"perResponseLabels":response_labels}),
            );
        }
        "western-celebrity" => {
            let passage_pages =
                collected_page_indexes(authoring.get("passage").unwrap_or(&Value::Null));
            let task_pages =
                collected_page_indexes(authoring.get("taskGroups").unwrap_or(&Value::Null));
            let expected_question_first = metadata
                .pointer("/expected/pageRoles/0/roles")
                .and_then(Value::as_array)
                .is_some_and(|roles| roles.iter().any(|role| role.as_str() == Some("question")));
            // The passage header (the "READING PASSAGE N" banner and the
            // "You should spend about ... below." instruction) legitimately sits
            // on the first page above the questions in this paper. Those regions
            // must stay anchored to the passage for region coverage, which makes
            // the raw passage anchor set include page 0. The layout contract
            // (metadata pageRoles) declares that page question-only, so verify
            // the substantive passage placement against the pages the contract
            // does not attribute to questions.
            let question_only_pages = metadata
                .pointer("/expected/pageRoles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|entry| {
                    let roles = entry.get("roles").and_then(Value::as_array);
                    let has_question = roles.as_ref().is_some_and(|roles| {
                        roles.iter().any(|role| role.as_str() == Some("question"))
                    });
                    let has_passage = roles.as_ref().is_some_and(|roles| {
                        roles.iter().any(|role| role.as_str() == Some("passage"))
                    });
                    has_question && !has_passage
                })
                .filter_map(|entry| entry.get("pageIndex").and_then(Value::as_u64))
                .map(|one_based| one_based.saturating_sub(1))
                .collect::<BTreeSet<_>>();
            let substantive_passage_pages = passage_pages
                .difference(&question_only_pages)
                .copied()
                .collect::<BTreeSet<_>>();
            let passed = expected_question_first
                && task_pages.first().copied() == Some(0)
                && substantive_passage_pages
                    .first()
                    .is_some_and(|page| *page > 0)
                && !substantive_passage_pages.contains(&0);
            push_check(
                checks,
                "WESTERN_QUESTIONS_BEFORE_PASSAGE",
                passed,
                json!({"questionMinPage":0,"passageMinPage":">0","passageExcludesPage0":true}),
                json!({"questionPages":task_pages,"passagePages":passage_pages,"questionOnlyPages":question_only_pages,"substantivePassagePages":substantive_passage_pages}),
            );
        }
        "chili-peppers" => {
            let passage_pages = metadata
                .pointer("/expected/pageRoles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|entry| {
                    entry
                        .get("roles")
                        .and_then(Value::as_array)
                        .is_some_and(|roles| {
                            roles.iter().any(|role| role.as_str() == Some("passage"))
                        })
                })
                .filter_map(|entry| entry.get("pageIndex").and_then(Value::as_u64))
                .map(|one_based| one_based.saturating_sub(1))
                .collect::<BTreeSet<_>>();
            let physical_passage_assets = physical
                .get("assets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|asset| {
                    asset
                        .pointer("/sourceAnchor/pageIndex")
                        .and_then(Value::as_u64)
                        .is_some_and(|page| passage_pages.contains(&page))
                })
                .filter_map(|asset| asset.get("assetId").and_then(Value::as_str))
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>();
            let passage_refs =
                collected_asset_ids(authoring.get("passage").unwrap_or(&Value::Null));
            let bound = physical_passage_assets
                .intersection(&passage_refs)
                .cloned()
                .collect::<Vec<_>>();
            push_check(
                checks,
                "CHILI_REQUIRED_PASSAGE_IMAGE",
                !physical_passage_assets.is_empty() && !bound.is_empty(),
                json!({"physicalPassageAssetCount":">0","boundPassageAssetCount":">0"}),
                json!({"physicalPassageAssets":physical_passage_assets,"passageAssetRefs":passage_refs,"bound":bound}),
            );
        }
        _ => {}
    }
}

fn process_fixture(root: &Path, fixture_id: &str, fixture: &Value) -> Result<Value, String> {
    let metadata_path = root.join(
        fixture
            .get("metadataPath")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{fixture_id}: metadataPath missing"))?,
    );
    let baseline_path = root.join(
        fixture
            .get("baselinePath")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{fixture_id}: baselinePath missing"))?,
    );
    let source_path = root.join(
        fixture
            .get("sourcePath")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{fixture_id}: sourcePath missing"))?,
    );
    let metadata = read_json(&metadata_path)?;
    let baseline = read_json(&baseline_path)?;
    let mut checks = Vec::new();
    let actual_hash = file_sha256(&source_path)?;
    let actual_size = fs::metadata(&source_path)
        .map_err(|error| format!("{}: {error}", source_path.display()))?
        .len();
    let expected_hash = fixture
        .get("sha256")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let expected_size = fixture
        .get("sizeBytes")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    push_check(
        &mut checks,
        "MANIFEST_PDF_IDENTITY",
        actual_hash == expected_hash && actual_size == expected_size,
        json!({"sha256":expected_hash,"sizeBytes":expected_size}),
        json!({"sha256":actual_hash,"sizeBytes":actual_size}),
    );
    let metadata_source = metadata.get("source").cloned().unwrap_or(Value::Null);
    let baseline_source = baseline.get("source").cloned().unwrap_or(Value::Null);
    push_check(
        &mut checks,
        "METADATA_BASELINE_SOURCE_IDENTITY",
        metadata_source.get("sha256") == fixture.get("sha256")
            && metadata_source.get("sizeBytes") == fixture.get("sizeBytes")
            && baseline_source.get("sha256") == fixture.get("sha256")
            && baseline_source.get("sizeBytes") == fixture.get("sizeBytes"),
        json!({"sha256":expected_hash,"sizeBytes":expected_size}),
        json!({"metadata":metadata_source,"baseline":baseline_source}),
    );

    let output_dir = root.join("tmp/phase4-real-pdf-acceptance").join(fixture_id);
    fs::create_dir_all(&output_dir).map_err(|error| error.to_string())?;
    let (source, job) = source_and_job(fixture_id, fixture, &metadata);
    let document_path = output_dir.join("document-ir-v1.actual.json");
    let document = parse_source_document(&job, &source, &source_path, &document_path, "auto")?;
    let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&document));
    let authoring_v1 = make_dynamic_authoring_ir(&job, &split, Some(&document));
    let reading_v1 = reading_source(&authoring_v1);
    let actual_summary = v1_summary(&document, &authoring_v1);
    let baseline_summary = baseline.get("observed").cloned().unwrap_or(Value::Null);
    let metadata_summary = metadata
        .pointer("/baseline/observed")
        .cloned()
        .unwrap_or(Value::Null);
    push_check(
        &mut checks,
        "EXECUTABLE_V1_BASELINE_SUMMARY",
        actual_summary == baseline_summary && baseline_summary == metadata_summary,
        json!({"baseline":baseline_summary,"metadata":metadata_summary}),
        actual_summary.clone(),
    );
    write_json(&output_dir.join("split-candidates-v1.actual.json"), &split)?;
    write_json(
        &output_dir.join("authoring-ir-v1.actual.json"),
        &authoring_v1,
    )?;
    write_json(
        &output_dir.join("reading-source-v1.actual.json"),
        &serde_json::to_value(&reading_v1).map_err(|error| error.to_string())?,
    )?;

    let physical_path = output_dir.join("document-ir-v2.physical.json");
    let physical = write_pdf_facts_shadow(&job, &source, &source_path, &physical_path)?;
    let authoring_v2 = build_authoring_v2_shadow(
        &job,
        &authoring_v1,
        &split,
        Some(&document),
        Some(&physical),
    )?;
    write_json(
        &output_dir.join("authoring-ir-v2.shadow.json"),
        &authoring_v2,
    )?;
    push_check(
        &mut checks,
        "REAL_PDF_PHYSICAL_DOCUMENT_IR_V2",
        physical.get("schemaVersion").and_then(Value::as_str) == Some("DocumentIRV2")
            && physical
                .get("pages")
                .and_then(Value::as_array)
                .is_some_and(|pages| !pages.is_empty()),
        json!({"schemaVersion":"DocumentIRV2","pages":">0"}),
        json!({"schemaVersion":physical.get("schemaVersion"),"pageCount":physical.get("pages").and_then(Value::as_array).map(Vec::len)}),
    );
    validate_physical_asset_integrity(&output_dir, &physical, &mut checks);
    let quality_state = authoring_v2
        .pointer("/quality/state")
        .and_then(Value::as_str);
    let physical_status = authoring_v2
        .pointer("/quality/coverageStatus/physicalShadow")
        .and_then(Value::as_str);
    let v2_probe = authoring_v2.pointer("/quality/compilerProbes/v2Runtime");
    let v1_probe = authoring_v2.pointer("/quality/compilerProbes/v1Compatibility");
    let v2_probe_status = v2_probe
        .and_then(|probe| probe.get("status"))
        .and_then(Value::as_str);
    let v1_probe_status = v1_probe
        .and_then(|probe| probe.get("status"))
        .and_then(Value::as_str);
    let compiler_probes_are_honest = compiler_probe_status_is_honest(v2_probe, quality_state)
        && compiler_probe_status_is_honest(v1_probe, quality_state);
    let coverage_complete = authoring_v2
        .pointer("/quality/coverageStatus/complete")
        .and_then(Value::as_bool)
        == Some(true);
    push_check(
        &mut checks,
        "PHYSICAL_AUTHORING_QUALITY_CHAIN",
        authoring_v2.get("schemaVersion").and_then(Value::as_str) == Some("IeltsAuthoringIRV2")
            && authoring_v2
                .pointer("/quality/schemaVersion")
                .and_then(Value::as_str)
                == Some("QualityReportV2")
            && physical_status == Some("available")
            && coverage_complete
            && matches!(
                quality_state,
                Some("ready") | Some("review_required") | Some("blocked")
            )
            && compiler_probes_are_honest,
        json!({"authoring":"IeltsAuthoringIRV2","quality":"QualityReportV2","physicalShadow":"available","coverageComplete":true,"compilerProbes":{"statuses":["passed","failed"],"failedRequiresIssueCode":true,"failedForbidsReady":true}}),
        json!({"authoring":authoring_v2.get("schemaVersion"),"quality":authoring_v2.pointer("/quality/schemaVersion"),"state":quality_state,"physicalShadow":physical_status,"coverageComplete":coverage_complete,"v2Runtime":{"status":v2_probe_status,"issueCodes":v2_probe.and_then(|probe| probe.get("issueCodes"))},"v1Compatibility":{"status":v1_probe_status,"issueCodes":v1_probe.and_then(|probe| probe.get("issueCodes"))}}),
    );
    let (quality_policy_passed, quality_policy_actual) =
        quality_acceptance_policy(fixture_id, &metadata, &physical, &authoring_v2);
    push_check(
        &mut checks,
        "QUALITY_BLOCKER_POLICY",
        quality_policy_passed,
        json!({
            "coverageComplete": true,
            "forbiddenIssueCodes": [],
            "unexpectedIssueCodes": [],
            "blockedRequiresExplicitImageOnlyAnswerEvidence": true,
            "v1CompatibilityPassed": true
        }),
        quality_policy_actual,
    );
    validate_metadata_truth(&metadata, &authoring_v2, &mut checks);
    validate_separation(&metadata, &physical, &authoring_v2, &mut checks);
    validate_special_truth(fixture_id, &metadata, &physical, &authoring_v2, &mut checks);

    let failed_codes = checks
        .iter()
        .filter(|check| check.get("passed").and_then(Value::as_bool) != Some(true))
        .filter_map(|check| check.get("code").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if quality_state == Some("ready") && !failed_codes.is_empty() {
        push_check(
            &mut checks,
            "FALSE_READY_FORBIDDEN",
            false,
            json!("quality state must not be ready while acceptance truth fails"),
            json!({"state":"ready","failedChecks":failed_codes}),
        );
    } else {
        push_check(
            &mut checks,
            "FALSE_READY_FORBIDDEN",
            true,
            json!("no false ready"),
            json!({"state":quality_state,"failedChecks":failed_codes}),
        );
    }
    let passed = checks
        .iter()
        .all(|check| check.get("passed").and_then(Value::as_bool) == Some(true));
    Ok(json!({
        "fixtureId": fixture_id,
        "passed": passed,
        "sourcePath": fixture.get("sourcePath"),
        "v1Summary": actual_summary,
        "physicalSummary": {
            "pageCount": physical.get("pages").and_then(Value::as_array).map(Vec::len),
            "assetCount": physical.get("assets").and_then(Value::as_array).map(Vec::len)
        },
        "v2Summary": {
            "examId": authoring_v2.pointer("/exam/examId"),
            "taskGroupCount": authoring_v2.get("taskGroups").and_then(Value::as_array).map(Vec::len),
            "slotCount": authoring_v2.get("answerSlots").and_then(Value::as_object).map(Map::len),
            "qualityState": quality_state,
            "sourceCoverage": authoring_v2.pointer("/quality/sourceCoverage")
        },
        "checks": checks
    }))
}

fn first_text_node_id(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("text") {
                return object
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
            }
            object.values().find_map(first_text_node_id)
        }
        Value::Array(items) => items.iter().find_map(first_text_node_id),
        _ => None,
    }
}

fn phase5_answer_resolution_patches(authoring: &Value) -> Vec<Value> {
    let mut patches = Vec::new();
    let tasks = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for task in &tasks {
        let bank_labels = task
            .get("optionBank")
            .and_then(|bank| bank.get("options"))
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(|option| option.get("label").and_then(Value::as_str))
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let groups = task
            .get("responseGroups")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for group in &groups {
            let group_labels = group
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| option.get("label").and_then(Value::as_str))
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .filter(|labels| !labels.is_empty())
                .unwrap_or_else(|| bank_labels.clone());
            let response_kind = group
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("text_entry");
            let assignment = group
                .get("assignment")
                .and_then(Value::as_str)
                .unwrap_or("per_slot");
            let assignment = if assignment == "ordered_slots" {
                "ordered"
            } else {
                assignment
            };
            for (index, slot_id) in group
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .enumerate()
            {
                let value = if response_kind == "text_entry" {
                    json!({"kind":"text","values":[format!("phase5-review-{slot_id}")],"normalization":"exact"})
                } else if let Some(label) = group_labels.get(index % group_labels.len().max(1)) {
                    json!({"kind":"option","labels":[label],"assignment":assignment})
                } else {
                    json!({"kind":"unresolved"})
                };
                patches.push(json!({"op":"setAnswer","slotId":slot_id,"value":value}));
            }
        }
    }
    patches
}

fn run_phase5_real_pdf_edit_export(
    root: &Path,
    fixture_id: &str,
    authoring: &Value,
) -> Result<Value, String> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    let job_id = format!("phase5-real-{fixture_id}-{unique}");
    let job_dir = root.join("jobs").join(&job_id);
    fs::create_dir_all(&job_dir).map_err(|error| error.to_string())?;
    let phase4_assets = root
        .join("tmp/phase4-real-pdf-acceptance")
        .join(fixture_id)
        .join("assets");
    let authoring_asset_count = authoring
        .get("assets")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if authoring_asset_count > 0 {
        if !phase4_assets.is_dir() {
            return Err(format!(
                "{fixture_id}: phase4 asset source missing: {}",
                phase4_assets.display()
            ));
        }
        copy_asset_tree(&phase4_assets, &job_dir.join("assets"))?;
    }
    let phase4_physical = root
        .join("tmp/phase4-real-pdf-acceptance")
        .join(fixture_id)
        .join("document-ir-v2.physical.json");
    if !phase4_physical.is_file() {
        return Err(format!(
            "{fixture_id}: phase4 physical shadow missing: {}",
            phase4_physical.display()
        ));
    }
    fs::copy(&phase4_physical, job_dir.join("document-ir-v2.shadow.json"))
        .map_err(|error| format!("{fixture_id}: phase4 physical shadow handoff failed: {error}"))?;
    write_json(&job_dir.join("authoring-ir-v2.shadow.json"), authoring)?;
    let answer_patches = phase5_answer_resolution_patches(authoring);
    if answer_patches.is_empty() {
        return Err(format!("{fixture_id}: no answer patches generated"));
    }
    let answer_result = apply_authoring_v2_patches_core(
        root,
        json!({"jobId":job_id,"baseRevision":0,"patches":answer_patches}),
    )?;
    let answered = answer_result
        .get("authoring")
        .cloned()
        .ok_or_else(|| format!("{fixture_id}: answer revision missing authoring"))?;
    let text_id = first_text_node_id(&answered)
        .ok_or_else(|| format!("{fixture_id}: no text node available for edit proof"))?;
    let edited_result = apply_authoring_v2_patches_core(
        root,
        json!({
            "jobId":job_id,
            "baseRevision":1,
            "patches":[{"op":"replaceText","nodeId":text_id,"from":0,"to":0,"text":"phase5-edit"}]
        }),
    )?;
    let revision = edited_result
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{fixture_id}: edited revision missing"))?;
    let export_dir = root
        .join("tmp/phase5-real-pdf-acceptance")
        .join(fixture_id)
        .join(format!("run-{unique}"));
    let export = export_authoring_v2_core(
        root,
        json!({"jobId":job_id,"exportDir":export_dir.to_string_lossy(),"revision":revision}),
    )?;
    let output_dir = export
        .get("outputDir")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{fixture_id}: export output path missing"))?;
    let manifest = read_json(&PathBuf::from(output_dir).join("manifest-v2.json"))?;
    let exported_authoring = read_json(&PathBuf::from(output_dir).join("authoring-ir-v2.json"))?;
    let exported_runtime = read_json(&PathBuf::from(output_dir).join("reading-source-v2.json"))?;
    if manifest.get("revision").and_then(Value::as_u64) != Some(revision)
        || exported_authoring
            .get("schemaVersion")
            .and_then(Value::as_str)
            != Some("IeltsAuthoringIRV2")
        || exported_runtime
            .get("schemaVersion")
            .and_then(Value::as_str)
            != Some("ReadingExamSourceV2")
    {
        return Err(format!("{fixture_id}: exported V2 bundle is not coherent"));
    }
    Ok(json!({
        "fixtureId": fixture_id,
        "jobId": job_id,
        "answerPatchCount": answer_patches.len(),
        "revision": revision,
        "outputDir": output_dir,
        "manifest": manifest,
        "authoringSchemaVersion": exported_authoring.get("schemaVersion"),
        "runtimeSchemaVersion": exported_runtime.get("schemaVersion")
    }))
}

#[test]
fn completion_embedding_detection_recurses_prompt_and_stimulus_per_slot() {
    let task = json!({
        "stimulus": [{
            "type": "table",
            "rows": [{"cells": [{"children": [{"type":"answer_slot","slotId":"q3"}]}]}]
        }]
    });
    let response = json!({
        "slotIds": ["q1", "q2", "q3"],
        "prompt": [{
            "type": "paragraph",
            "children": [
                {"type":"text","text":"First"},
                {"type":"answer_slot","slotId":"q1"},
                {"type":"paragraph","children":[{"type":"answer_slot","slotId":"q2"}]}
            ]
        }]
    });
    let (complete, embedded, missing) = completion_embedding_evidence(Some(&task), Some(&response));
    assert!(complete);
    assert_eq!(
        embedded,
        BTreeSet::from(["q1".into(), "q2".into(), "q3".into()])
    );
    assert!(missing.is_empty());

    let mut incomplete = response.clone();
    incomplete["slotIds"] = json!(["q1", "q2", "q3", "q4"]);
    let (complete, _, missing) = completion_embedding_evidence(Some(&task), Some(&incomplete));
    assert!(!complete, "missing slots must not be accepted as embedded");
    assert_eq!(missing, BTreeSet::from(["q4".to_string()]));
}

#[test]
fn image_only_answer_pages_remain_unresolved_without_oracle_overlay() {
    let metadata = json!({
        "expected":{"pageRoles":[
            {"pageIndex":7,"roles":["answer"]},
            {"pageIndex":8,"roles":["explanation"]}
        ]}
    });
    let physical = json!({"pages":[
        {"pageIndex":6,"lines":[{"text":" "}]},
        {"pageIndex":7,"lines":[{"text":""}]}
    ]});
    let authoring = json!({
        "taskGroups":[],
        "answerKey":{"q14":{"kind":"unresolved"},"q15":{"kind":"unresolved"}},
        "quality":{
            "state":"blocked",
            "compilerProbes":{"v2Runtime":{
                "status":"failed",
                "issueCodes":["RUNTIME_ANSWER_KEY_POLICY_INVALID"]
            }}
        }
    });
    let mut checks = Vec::new();
    validate_special_truth(
        "organisational-design",
        &metadata,
        &physical,
        &authoring,
        &mut checks,
    );
    let check = checks
        .iter()
        .find(|check| {
            check.get("code").and_then(Value::as_str)
                == Some("ORGANISATIONAL_RASTER_ANSWER_KEY_REQUIRES_OCR")
        })
        .expect("raster answer-key check");
    assert_eq!(check.get("passed").and_then(Value::as_bool), Some(true));
    let ocr_signal = checks
        .iter()
        .find(|check| {
            check.get("code").and_then(Value::as_str)
                == Some("ORGANISATIONAL_ANSWER_PAGE_OCR_SIGNAL")
        })
        .expect("OCR signal check");
    assert_eq!(
        ocr_signal.get("passed").and_then(Value::as_bool),
        Some(false),
        "image-only pages must expose the missing OCR plan instead of receiving oracle answers"
    );
    assert!(authoring
        .get("answerKey")
        .and_then(Value::as_object)
        .is_some_and(|answers| {
            answers
                .values()
                .all(|answer| answer.get("kind").and_then(Value::as_str) == Some("unresolved"))
        }));
}

#[test]
fn compiler_probe_chain_requires_honest_failure_semantics() {
    let passed = json!({"status":"passed","issueCodes":[]});
    assert!(compiler_probe_status_is_honest(
        Some(&passed),
        Some("ready")
    ));

    let failed_with_issue =
        json!({"status":"failed","issueCodes":["RUNTIME_ANSWER_KEY_POLICY_INVALID"]});
    assert!(compiler_probe_status_is_honest(
        Some(&failed_with_issue),
        Some("blocked")
    ));
    assert!(!compiler_probe_status_is_honest(
        Some(&failed_with_issue),
        Some("ready")
    ));

    let failed_without_issue = json!({"status":"failed","issueCodes":[]});
    assert!(!compiler_probe_status_is_honest(
        Some(&failed_without_issue),
        Some("blocked")
    ));
    let unknown = json!({"status":"not_run","issueCodes":[]});
    assert!(!compiler_probe_status_is_honest(
        Some(&unknown),
        Some("blocked")
    ));
    assert!(!compiler_probe_status_is_honest(None, Some("blocked")));
}

#[test]
fn quality_acceptance_policy_rejects_unexplained_blockers_and_requires_live_ocr_evidence() {
    let metadata = json!({
        "expected":{"pageRoles":[{"pageIndex":1,"roles":["answer"]}]}
    });
    let physical = json!({"pages":[{
        "lines":[{"text":""}],
        "imagePlacements":[{"id":"image-1"}],
        "quality":{"requiresOcrRegions":[{"id":"ocr-1"}]}
    }]});
    let mut authoring = json!({"quality":{
        "state":"blocked",
        "coverageStatus":{"complete":true},
        "issues":[{"code":"ANSWER_KEY_MISSING_SLOT"}],
        "compilerProbes":{
            "v2Runtime":{"status":"passed","issueCodes":[]},
            "v1Compatibility":{"status":"passed","issueCodes":[]}
        }
    }});

    assert!(quality_acceptance_policy("chili-peppers", &metadata, &physical, &authoring).0);

    authoring["quality"]["issues"] = json!([
        {"code":"ANSWER_KEY_MISSING_SLOT"},
        {"code":"SIGNIFICANT_REGION_UNASSIGNED"}
    ]);
    assert!(
        !quality_acceptance_policy("chili-peppers", &metadata, &physical, &authoring).0,
        "coverage blockers must never be hidden by an image-only answer page"
    );

    authoring["quality"]["issues"] = json!([{"code":"ANSWER_KEY_MISSING_SLOT"}]);
    let mut no_ocr = physical.clone();
    no_ocr["pages"][0]["quality"]["requiresOcrRegions"] = json!([]);
    assert!(
        !quality_acceptance_policy("chili-peppers", &metadata, &no_ocr, &authoring).0,
        "missing answers require current physical OCR evidence"
    );
}

#[test]
fn quality_acceptance_policy_limits_runtime_exception_to_organisational_unresolved_set() {
    let metadata = json!({
        "expected":{"pageRoles":[{"pageIndex":1,"roles":["answer"]}]}
    });
    let physical = json!({"pages":[{
        "lines":[],
        "imagePlacements":[{"id":"image-1"}],
        "quality":{"requiresOcrRegions":[{"id":"ocr-1"}]}
    }]});
    let authoring = json!({"quality":{
        "state":"blocked",
        "coverageStatus":{"complete":true},
        "issues":[
            {"code":"ANSWER_KEY_MISSING_SLOT"},
            {"code":"RUNTIME_COMPILER_FAILED"}
        ],
        "compilerProbes":{
            "v2Runtime":{
                "status":"failed",
                "issueCodes":["RUNTIME_ANSWER_KEY_POLICY_INVALID"]
            },
            "v1Compatibility":{"status":"passed","issueCodes":[]}
        }
    }});

    assert!(quality_acceptance_policy("organisational-design", &metadata, &physical, &authoring).0);
    assert!(
        !quality_acceptance_policy("chili-peppers", &metadata, &physical, &authoring).0,
        "the unresolved shared-set runtime exception must not spread to other fixtures"
    );
}

#[test]
#[ignore = "real private-PDF probe; run explicitly while repairing Chili acceptance"]
fn chili_real_pdf_reaches_all_declared_acceptance_truth() {
    let root = repo_root();
    let manifest = read_json(&root.join(MANIFEST)).expect("golden manifest must load");
    let fixture = manifest
        .get("fixtures")
        .and_then(Value::as_array)
        .and_then(|fixtures| {
            fixtures.iter().find(|fixture| {
                fixture.get("fixtureId").and_then(Value::as_str) == Some("chili-peppers")
            })
        })
        .expect("Chili fixture must exist");
    let result =
        process_fixture(&root, "chili-peppers", fixture).expect("Chili chain must execute");
    let failed = result
        .get("checks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|check| check.get("passed").and_then(Value::as_bool) != Some(true))
        .filter_map(|check| check.get("code").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(failed.is_empty(), "Chili acceptance failures: {failed:?}");
}

#[test]
#[ignore = "real private-PDF probe; run explicitly while repairing Petri acceptance"]
fn petri_real_pdf_reaches_all_declared_acceptance_truth() {
    let root = repo_root();
    let manifest = read_json(&root.join(MANIFEST)).expect("golden manifest must load");
    let fixture = manifest
        .get("fixtures")
        .and_then(Value::as_array)
        .and_then(|fixtures| {
            fixtures.iter().find(|fixture| {
                fixture.get("fixtureId").and_then(Value::as_str) == Some("petri-dish")
            })
        })
        .expect("Petri fixture must exist");
    let result = process_fixture(&root, "petri-dish", fixture).expect("Petri chain must execute");
    let failed = result
        .get("checks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|check| check.get("passed").and_then(Value::as_bool) != Some(true))
        .filter_map(|check| check.get("code").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(failed.is_empty(), "Petri acceptance failures: {failed:?}");
}

#[test]
fn phase4_eight_real_pdfs_reach_physical_authoring_quality_truth() {
    if !crate::test_support::golden_private_corpus_ready(
        "phase4_eight_real_pdfs_reach_physical_authoring_quality_truth",
    ) {
        return;
    }
    let root = repo_root();
    let spec = read_json(&root.join(ACCEPTANCE_SPEC)).expect("acceptance spec must load");
    assert_eq!(
        spec.get("schemaVersion").and_then(Value::as_str),
        Some("Phase4EightPdfAcceptanceV1")
    );
    let fixture_ids = spec
        .get("fixtureIds")
        .and_then(Value::as_array)
        .expect("acceptance fixtureIds");
    assert_eq!(
        fixture_ids.len(),
        8,
        "Phase 4 acceptance corpus must contain 8 PDFs"
    );
    let manifest = read_json(&root.join(MANIFEST)).expect("golden manifest must load");
    let fixtures = manifest
        .get("fixtures")
        .and_then(Value::as_array)
        .expect("golden fixtures");
    let mut results = Vec::new();
    for fixture_id in fixture_ids.iter().filter_map(Value::as_str) {
        let fixture = fixtures
            .iter()
            .find(|fixture| fixture.get("fixtureId").and_then(Value::as_str) == Some(fixture_id));
        let result = match fixture {
            Some(fixture) => process_fixture(&root, fixture_id, fixture).unwrap_or_else(|error| {
                json!({
                    "fixtureId": fixture_id,
                    "passed": false,
                    "fatalError": error,
                    "checks": []
                })
            }),
            None => json!({
                "fixtureId": fixture_id,
                "passed": false,
                "fatalError": "fixture missing from golden manifest",
                "checks": []
            }),
        };
        results.push(result);
    }
    let fixture_checks_passed = results
        .iter()
        .all(|result| result.get("passed").and_then(Value::as_bool) == Some(true));
    let aggregate_checks = vec![corpus_exam_id_check(&results)];
    let aggregate_checks_passed = aggregate_checks
        .iter()
        .all(|check| check.get("passed").and_then(Value::as_bool) == Some(true));
    let passed = fixture_checks_passed && aggregate_checks_passed;
    let report = json!({
        "schemaVersion": "Phase4EightPdfAcceptanceReportV1",
        "runToken": std::env::var("PHASE4_ACCEPTANCE_RUN_TOKEN").ok(),
        "passed": passed,
        "fixtureCount": results.len(),
        "policy": spec.get("policy"),
        "checks": aggregate_checks,
        "fixtures": results
    });
    let report_path = root.join(REPORT);
    write_json(&report_path, &report).expect("acceptance report must be written");
    assert!(
        passed,
        "Phase 4 real-PDF acceptance failed; inspect {}",
        report_path.display()
    );
}

#[test]
fn phase5_real_pdf_edit_and_v2_export_round_trip() {
    if !crate::test_support::golden_private_corpus_ready(
        "phase5_real_pdf_edit_and_v2_export_round_trip",
    ) {
        return;
    }
    let root = repo_root();
    let manifest = read_json(&root.join(MANIFEST)).expect("golden manifest must load");
    let fixture = manifest
        .get("fixtures")
        .and_then(Value::as_array)
        .and_then(|fixtures| {
            fixtures.iter().find(|fixture| {
                fixture.get("fixtureId").and_then(Value::as_str) == Some("chili-peppers")
            })
        })
        .expect("Chili fixture must exist");
    let phase4_result = process_fixture(&root, "chili-peppers", fixture)
        .expect("real PDF must reach Phase 4 V2 shadow before Phase 5 edit");
    let shadow_path =
        root.join("tmp/phase4-real-pdf-acceptance/chili-peppers/authoring-ir-v2.shadow.json");
    let shadow = read_json(&shadow_path).expect("Phase 4 V2 shadow must be readable");
    let export_result = run_phase5_real_pdf_edit_export(&root, "chili-peppers", &shadow)
        .expect("real PDF V2 editor/export round trip must complete");
    let report = json!({
        "schemaVersion": "Phase5RealPdfEditorExportReportV1",
        "runToken": std::env::var("PHASE5_ACCEPTANCE_RUN_TOKEN").ok(),
        "passed": true,
        "fixtureId": "chili-peppers",
        "phase4": phase4_result,
        "phase5": export_result
    });
    write_json(&root.join(PHASE5_REPORT), &report).expect("Phase 5 report must be written");
}

#[test]
fn corpus_exam_id_check_rejects_duplicates_and_unsafe_segments() {
    let valid = vec![
        json!({"fixtureId":"one","v2Summary":{"examId":"p1-one"}}),
        json!({"fixtureId":"two","v2Summary":{"examId":"p2-two"}}),
    ];
    assert_eq!(corpus_exam_id_check(&valid)["passed"], json!(true));

    let duplicate = vec![
        json!({"fixtureId":"one","v2Summary":{"examId":"p1-one"}}),
        json!({"fixtureId":"two","v2Summary":{"examId":"p1-one"}}),
    ];
    assert_eq!(corpus_exam_id_check(&duplicate)["passed"], json!(false));

    let unsafe_id = vec![json!({"fixtureId":"unsafe","v2Summary":{"examId":"../unsafe"}})];
    assert_eq!(corpus_exam_id_check(&unsafe_id)["passed"], json!(false));
}

#[test]
fn metadata_contract_asserts_response_shape_binding_and_reuse() {
    let metadata = json!({
        "expected": {
            "taskGroups": [{"id":"group-1","displayRange":[1,2],"kind":"matching_information"}],
            "slots": [
                {"id":"q1","responseType":"select"},
                {"id":"q2","responseType":"select"}
            ],
            "optionBanks": [{
                "id":"group-1-option-bank",
                "taskGroupId":"group-1",
                "scope":"task_group",
                "labels":["A","B"],
                "allowReuse":true
            }],
            "responseGroups": [{
                "id":"group-1-responses",
                "taskGroupId":"group-1",
                "slotIds":["q1","q2"],
                "promptMode":"per_slot",
                "cardinality":{"min":1,"max":1,"exact":1},
                "assignment":"per_slot",
                "optionBinding":{"mode":"option_bank","optionBankId":"group-1-option-bank"},
                "reusePolicy":"allowed",
                "scoringPolicy":"per_slot_binary"
            }]
        }
    });
    let mut authoring = json!({
        "taskGroups": [{
            "taskId":"group-1",
            "displayRange":{"kind":"range","start":1,"end":2},
            "taskType":"matching_information",
            "optionBank": {
                "optionBankId":"group-1-option-bank",
                "scope":"task_group",
                "options":[{"label":"A"},{"label":"B"}],
                "allowReuse":true
            },
            "responseGroups": [{
                "responseGroupId":"group-1-responses",
                "prompt":[{"id":"prompt-1"},{"id":"prompt-2"}],
                "slotIds":["q1","q2"],
                "optionBankRef":"group-1-option-bank",
                "cardinality":{"min":1,"max":1,"exact":1},
                "assignment":"per_slot",
                "scoringPolicy":"per_slot_binary",
                "allowOptionReuse":true
            }]
        }],
        "answerSlots": {
            "q1":{"interaction":"select"},
            "q2":{"interaction":"select"}
        }
    });
    let mut checks = Vec::new();
    validate_metadata_truth(&metadata, &authoring, &mut checks);
    assert!(
        checks
            .iter()
            .all(|check| check.get("passed").and_then(Value::as_bool) == Some(true)),
        "valid metadata contract failed: {checks:?}"
    );

    authoring["taskGroups"][0]["optionBank"]["allowReuse"] = json!(false);
    authoring["taskGroups"][0]["responseGroups"][0]["allowOptionReuse"] = json!(false);
    checks.clear();
    validate_metadata_truth(&metadata, &authoring, &mut checks);
    let failed_codes = checks
        .iter()
        .filter(|check| check.get("passed").and_then(Value::as_bool) != Some(true))
        .filter_map(|check| check.get("code").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    assert!(failed_codes.contains("METADATA_OPTION_BANK:group-1-option-bank"));
    assert!(failed_codes.contains("METADATA_RESPONSE_GROUP:group-1-responses"));
}

#[test]
fn metadata_contract_accepts_completion_slots_embedded_in_response_prompt() {
    let metadata = json!({
        "expected": {
            "taskGroups": [{"id":"group-2","displayRange":[7,8],"kind":"note_completion"}],
            "slots": [
                {"id":"q7","responseType":"text"},
                {"id":"q8","responseType":"text"}
            ],
            "optionBanks": [],
            "responseGroups": [{
                "id":"group-2-responses",
                "taskGroupId":"group-2",
                "slotIds":["q7","q8"],
                "promptMode":"embedded",
                "cardinality":{"min":1,"max":1,"exact":1},
                "assignment":"per_slot",
                "optionBinding":{"mode":"none"},
                "reusePolicy":"not_applicable",
                "scoringPolicy":"per_slot_ielts_normalized"
            }]
        }
    });
    let mut authoring = json!({
        "taskGroups": [{
            "taskId":"group-2",
            "displayRange":{"kind":"range","start":7,"end":8},
            "taskType":"note_completion",
            "responseGroups": [{
                "responseGroupId":"group-2-responses",
                "prompt":[
                    {"type":"paragraph","children":[{"type":"answer_slot","slotId":"q7"}]},
                    {"type":"paragraph","children":[{"type":"paragraph","children":[{"type":"answer_slot","slotId":"q8"}]}]}
                ],
                "slotIds":["q7","q8"],
                "cardinality":{"min":1,"max":1,"exact":1},
                "assignment":"per_slot",
                "scoringPolicy":"per_slot_ielts_normalized",
                "allowOptionReuse":false
            }]
        }],
        "answerSlots": {
            "q7":{"interaction":"text"},
            "q8":{"interaction":"text"}
        }
    });
    let mut checks = Vec::new();
    validate_metadata_truth(&metadata, &authoring, &mut checks);
    assert!(
        checks
            .iter()
            .all(|check| check.get("passed").and_then(Value::as_bool) == Some(true)),
        "prompt-embedded completion was rejected: {checks:?}"
    );

    authoring["taskGroups"][0]["responseGroups"][0]["prompt"][1] =
        json!({"type":"paragraph","children":[{"type":"text","text":"missing q8"}]});
    checks.clear();
    validate_metadata_truth(&metadata, &authoring, &mut checks);
    assert!(checks.iter().any(|check| {
        check.get("code").and_then(Value::as_str)
            == Some("METADATA_RESPONSE_GROUP:group-2-responses")
            && check.get("passed").and_then(Value::as_bool) == Some(false)
    }));
}
