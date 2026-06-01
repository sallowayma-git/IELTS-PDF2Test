use crate::reading_source::{question_order_from_authoring, reading_source};
use crate::validator::{
    has_error_issues, json_issue, qid_sort_key, validate_reading_source_contract,
    validation_layers, ValidationReportV1,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub(crate) fn validate_authoring(job_id: &str, authoring: Option<&Value>) -> Value {
    let mut issues = Vec::new();
    if let Some(ir) = authoring {
        if ir
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .is_empty()
        {
            issues.push(json_issue(
                "AuthoringIR",
                "$.exam.examId",
                "examId is required",
            ));
        }
        if ir
            .get("groups")
            .and_then(Value::as_array)
            .map(|items| items.is_empty())
            .unwrap_or(true)
        {
            issues.push(json_issue(
                "AuthoringIR",
                "$.groups",
                "At least one question group is required",
            ));
        }
        let source = reading_source(ir);
        issues.extend(validate_reading_source_contract(&source));

        let question_order = question_order_from_authoring(ir);
        let mut seen_qids = HashSet::new();
        let mut duplicate_qids = HashSet::new();
        for qid in &question_order {
            if !seen_qids.insert(qid.clone()) {
                duplicate_qids.insert(qid.clone());
            }
        }
        for qid in duplicate_qids {
            issues.push(json_issue(
                "AuthoringIR",
                "$.questionOrder",
                &format!("Duplicate question id in questionOrder: {}", qid),
            ));
        }

        let mut numeric_order = question_order
            .iter()
            .filter_map(|qid| qid_sort_key(qid))
            .collect::<Vec<_>>();
        numeric_order.sort_unstable();
        numeric_order.dedup();
        if let (Some(first), Some(last)) = (
            numeric_order.first().copied(),
            numeric_order.last().copied(),
        ) {
            let expected_len = (last - first + 1) as usize;
            if expected_len != numeric_order.len() {
                issues.push(json_issue(
                    "ReadingExamSourceV1",
                    "$.questionOrder",
                    &format!(
                        "questionOrder must be numerically continuous from q{} to q{}",
                        first, last
                    ),
                ));
            }
        }

        let mut display_seen: HashMap<String, String> = HashMap::new();
        for question in ir
            .get("groups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|group| {
                group
                    .get("questions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
        {
            if let Some(qid) = question.get("id").and_then(Value::as_str) {
                let display = question
                    .get("displayNumber")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if display.is_empty() {
                    issues.push(json_issue(
                        "ReadingExamSourceV1",
                        &format!("$.questionDisplayMap.{}", qid),
                        "questionDisplayMap display number cannot be empty",
                    ));
                } else if let Some(existing_qid) =
                    display_seen.insert(display.clone(), qid.to_string())
                {
                    issues.push(json_issue(
                        "ReadingExamSourceV1",
                        "$.questionDisplayMap",
                        &format!(
                            "Duplicate display number {} for {} and {}",
                            display, existing_qid, qid
                        ),
                    ));
                }
            }
        }
    } else {
        issues.push(json_issue("AuthoringIR", "$", "Authoring IR is missing"));
    }

    ValidationReportV1 {
        job_id: job_id.to_string(),
        passed: !has_error_issues(&issues),
        layers: validation_layers(&issues),
        issues,
        generated_at: Utc::now().to_rfc3339(),
        runtime: None,
    }
    .to_value()
}

pub(crate) fn merge_sidecar_validation(base: &mut Value, sidecar: Value) {
    let Some(base_obj) = base.as_object_mut() else {
        return;
    };
    let sidecar_issues = sidecar
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let sidecar_layers = sidecar
        .get("layers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut layers_to_replace = sidecar_layers
        .iter()
        .filter_map(|layer| {
            layer
                .get("layer")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    if layers_to_replace.is_empty() {
        layers_to_replace.extend(["ReadingExamSourceV1".to_string(), "DomProtocol".to_string()]);
    }
    let replace_existing_layers = sidecar
        .get("replaceExistingLayers")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut merged_issues = base_obj
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|issue| {
            if !replace_existing_layers {
                return true;
            }
            issue
                .get("layer")
                .and_then(Value::as_str)
                .map(|layer| !layers_to_replace.iter().any(|item| item == layer))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    merged_issues.extend(sidecar_issues);

    let layers = validation_layers(&merged_issues);
    base_obj.insert(
        "passed".to_string(),
        json!(!has_error_issues(&merged_issues)),
    );
    base_obj.insert("layers".to_string(), json!(layers));
    base_obj.insert("issues".to_string(), json!(merged_issues));
    if let Some(runtime) = sidecar.get("runtime") {
        base_obj.insert("runtime".to_string(), runtime.clone());
    }
}

pub(crate) fn merge_validation_issues(report: &mut Value, extra_issues: Vec<Value>) {
    if extra_issues.is_empty() {
        return;
    }
    let Some(obj) = report.as_object_mut() else {
        return;
    };
    let mut issues = obj
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    issues.extend(extra_issues);
    let layers = validation_layers(&issues);
    obj.insert("passed".to_string(), json!(!has_error_issues(&issues)));
    obj.insert("layers".to_string(), json!(layers));
    obj.insert("issues".to_string(), json!(issues));
}
