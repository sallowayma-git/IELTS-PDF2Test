use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidationLayerReportV1 {
    pub layer: String,
    pub passed: bool,
    pub issue_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidationReportV1 {
    pub job_id: String,
    pub passed: bool,
    pub layers: Vec<ValidationLayerReportV1>,
    pub issues: Vec<Value>,
    pub generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Value>,
}

impl ValidationReportV1 {
    pub(crate) fn to_value(&self) -> Value {
        serde_json::to_value(self)
            .expect("ValidationReportV1 only contains JSON-serializable fields")
    }
}

pub(crate) fn qid_sort_key(qid: &str) -> Option<u32> {
    qid.strip_prefix('q')?.parse::<u32>().ok()
}

pub(crate) fn allowed_question_kind(kind: &str) -> bool {
    matches!(
        kind,
        "single_choice"
            | "multi_choice"
            | "true_false_not_given"
            | "yes_no_not_given"
            | "matching"
            | "heading_matching"
            | "matching_information"
            | "classification"
            | "summary_completion"
            | "table_completion"
            | "diagram_completion"
            | "short_answer"
            | "sentence_completion"
    )
}

fn html_start_tags(html: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        if rest.starts_with('/') || rest.starts_with('!') {
            continue;
        }
        let Some(end) = rest.find('>') else {
            break;
        };
        tags.push(format!("<{}>", &rest[..end]));
        rest = &rest[end + 1..];
    }
    tags
}

fn html_tag_name(tag: &str) -> String {
    tag.trim_start_matches('<')
        .trim_end_matches('>')
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn html_attr_map(tag: &str) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    let inner = tag.trim_start_matches('<').trim_end_matches('>');
    let mut index = inner.find(char::is_whitespace).unwrap_or(inner.len());
    let bytes = inner.as_bytes();
    while index < inner.len() {
        while index < inner.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= inner.len() {
            break;
        }
        let key_start = index;
        while index < inner.len()
            && !bytes[index].is_ascii_whitespace()
            && !matches!(bytes[index], b'=' | b'/' | b'>')
        {
            index += 1;
        }
        let key = inner[key_start..index].trim().to_ascii_lowercase();
        while index < inner.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let mut value = String::new();
        if index < inner.len() && bytes[index] == b'=' {
            index += 1;
            while index < inner.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index < inner.len() && matches!(bytes[index], b'"' | b'\'') {
                let quote = bytes[index];
                index += 1;
                let value_start = index;
                while index < inner.len() && bytes[index] != quote {
                    index += 1;
                }
                value = inner[value_start..index].to_string();
                if index < inner.len() {
                    index += 1;
                }
            } else {
                let value_start = index;
                while index < inner.len() && !bytes[index].is_ascii_whitespace() {
                    index += 1;
                }
                value = inner[value_start..index].trim_end_matches('/').to_string();
            }
        }
        if !key.is_empty() {
            attrs.insert(key, value);
        }
    }
    attrs
}

fn html_control_tags(html: &str) -> Vec<String> {
    html_start_tags(html)
        .into_iter()
        .filter(|tag| {
            let name = html_tag_name(tag);
            matches!(name.as_str(), "input" | "select" | "textarea")
                || tag.contains("paragraph-dropzone")
                || tag.contains("match-dropzone")
                || tag.contains("drop-target-summary")
        })
        .collect()
}

fn control_question_id(attrs: &HashMap<String, String>) -> Option<String> {
    for key in ["name", "data-question", "data-question-id", "data-target"] {
        if let Some(value) = attrs.get(key).filter(|value| !value.trim().is_empty()) {
            return Some(value.to_string());
        }
    }
    attrs.get("id").and_then(|id| {
        if id.ends_with("_input") {
            Some(id.trim_end_matches("_input").to_string())
        } else if !id.trim().is_empty() {
            Some(id.to_string())
        } else {
            None
        }
    })
}

fn has_collectible_control(html: &str, qid: &str) -> bool {
    html_control_tags(html)
        .iter()
        .any(|tag| control_question_id(&html_attr_map(tag)).as_deref() == Some(qid))
}

fn dropzone_tags(html: &str) -> Vec<String> {
    html_start_tags(html)
        .into_iter()
        .filter(|tag| {
            tag.contains("paragraph-dropzone")
                || tag.contains("match-dropzone")
                || tag.contains("drop-target-summary")
        })
        .collect()
}

fn has_valid_dropzone(html: &str, qid: &str) -> bool {
    dropzone_tags(html)
        .iter()
        .any(|tag| control_question_id(&html_attr_map(tag)).as_deref() == Some(qid))
}

fn has_invalid_dropzone(html: &str) -> bool {
    dropzone_tags(html)
        .iter()
        .any(|tag| control_question_id(&html_attr_map(tag)).is_none())
}

pub(crate) fn validate_reading_source_contract(source: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    if source.get("schemaVersion").and_then(Value::as_str) != Some("ReadingExamSourceV1") {
        issues.push(json_issue(
            "ReadingExamSourceV1",
            "$.schemaVersion",
            "schemaVersion must be ReadingExamSourceV1",
        ));
    }
    if source
        .get("examId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .is_empty()
    {
        issues.push(json_issue(
            "ReadingExamSourceV1",
            "$.examId",
            "examId is required",
        ));
    }
    if source
        .pointer("/meta/title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .is_empty()
    {
        issues.push(json_issue(
            "ReadingExamSourceV1",
            "$.meta.title",
            "meta.title is required",
        ));
    }
    if source
        .pointer("/passage/blocks")
        .and_then(Value::as_array)
        .map(Vec::is_empty)
        .unwrap_or(true)
    {
        issues.push(json_issue(
            "ReadingExamSourceV1",
            "$.passage.blocks",
            "passage.blocks cannot be empty",
        ));
    }
    let groups = source
        .get("questionGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if groups.is_empty() {
        issues.push(json_issue(
            "ReadingExamSourceV1",
            "$.questionGroups",
            "questionGroups cannot be empty",
        ));
    }
    let answer_key = source
        .get("answerKey")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if answer_key.is_empty() {
        issues.push(json_warning(
            "ReadingExamSourceV1",
            "$.answerKey",
            "answerKey is empty; unanswered questions will be exported without scoring data",
        ));
    }

    let mut covered = HashSet::<String>::new();
    for group in &groups {
        let group_id = group
            .get("groupId")
            .and_then(Value::as_str)
            .unwrap_or("unknown-group");
        let kind = group
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !allowed_question_kind(kind) {
            issues.push(json_issue(
                "ReadingExamSourceV1",
                &format!("$.questionGroups.{}.kind", group_id),
                &format!("{} is not an allowed group kind", kind),
            ));
        }
        if matches!(
            kind,
            "matching" | "heading_matching" | "matching_information" | "classification"
        ) && !group
            .get("allowOptionReuse")
            .map(Value::is_boolean)
            .unwrap_or(false)
        {
            issues.push(json_issue(
                "ReadingExamSourceV1",
                &format!("$.questionGroups.{}.allowOptionReuse", group_id),
                "matching/classification groups must explicitly set allowOptionReuse",
            ));
        }
        let html = group
            .get("bodyHtml")
            .and_then(Value::as_str)
            .unwrap_or_default();
        for qid in group
            .get("questionIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            covered.insert(qid.to_string());
            if !answer_key.contains_key(qid) {
                issues.push(json_warning(
                    "ReadingExamSourceV1",
                    &format!("$.answerKey.{}", qid),
                    &format!(
                        "{} is missing from answerKey and will be exported without scoring data",
                        qid
                    ),
                ));
            }
            if !has_collectible_control(html, qid) {
                issues.push(json_issue(
                    "DomProtocol",
                    &format!("$.questionGroups.{}.bodyHtml", group_id),
                    &format!("No collectible control found for {}", qid),
                ));
            }
            if html.contains("dropzone") || html.contains("drop-target") {
                if !has_valid_dropzone(html, qid) && !has_collectible_control(html, qid) {
                    issues.push(json_issue(
                        "DomProtocol",
                        &format!("$.questionGroups.{}.bodyHtml", group_id),
                        &format!("No valid dropzone target found for {}", qid),
                    ));
                }
                if has_invalid_dropzone(html) {
                    issues.push(json_issue(
                        "DomProtocol",
                        &format!("$.questionGroups.{}.bodyHtml", group_id),
                        "Dropzone is missing data-question/data-question-id/data-target or id fallback",
                    ));
                }
            }
        }
    }
    for qid in answer_key.keys() {
        if !covered.contains(qid) {
            issues.push(json_issue(
                "ReadingExamSourceV1",
                "$.questionGroups",
                &format!(
                    "{} from answerKey is not covered by any question group",
                    qid
                ),
            ));
        }
    }
    let question_order = source
        .get("questionOrder")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if question_order.len() != covered.len() {
        issues.push(json_issue(
            "ReadingExamSourceV1",
            "$.questionOrder",
            "questionOrder length must equal covered question count",
        ));
    }
    let mut order_set = HashSet::<String>::new();
    for qid in question_order.iter().filter_map(Value::as_str) {
        if !order_set.insert(qid.to_string()) {
            issues.push(json_issue(
                "ReadingExamSourceV1",
                "$.questionOrder",
                &format!("Duplicate question id in questionOrder: {}", qid),
            ));
        }
    }
    for qid in &covered {
        if !order_set.contains(qid) {
            issues.push(json_issue(
                "ReadingExamSourceV1",
                "$.questionOrder",
                &format!("{} is missing from questionOrder", qid),
            ));
        }
    }
    let display_map = source
        .get("questionDisplayMap")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for qid in question_order.iter().filter_map(Value::as_str) {
        if !covered.contains(qid) {
            issues.push(json_issue(
                "ReadingExamSourceV1",
                "$.questionOrder",
                &format!("{} is not covered by any question group", qid),
            ));
        }
        if !display_map.contains_key(qid) {
            issues.push(json_issue(
                "ReadingExamSourceV1",
                &format!("$.questionDisplayMap.{}", qid),
                &format!("{} is missing original display number", qid),
            ));
        }
    }
    issues
}

pub(crate) fn json_issue(layer: &str, path: &str, message: &str) -> Value {
    json_issue_with_severity("error", layer, path, message)
}

pub(crate) fn json_warning(layer: &str, path: &str, message: &str) -> Value {
    json_issue_with_severity("warning", layer, path, message)
}

fn json_issue_with_severity(severity: &str, layer: &str, path: &str, message: &str) -> Value {
    json!({"issueId": format!("issue-{}", Uuid::new_v4().simple()), "severity":severity, "layer":layer, "path":path, "message":message, "fixHint": null})
}

pub(crate) fn is_error_issue(issue: &Value) -> bool {
    issue
        .get("severity")
        .and_then(Value::as_str)
        .unwrap_or("error")
        == "error"
}

pub(crate) fn has_error_issues(issues: &[Value]) -> bool {
    issues.iter().any(is_error_issue)
}

pub(crate) fn validation_layers(issues: &[Value]) -> Vec<ValidationLayerReportV1> {
    [
        "AuthoringIR",
        "ReadingExamSourceV1",
        "DomProtocol",
        "RuntimePreview",
    ]
    .iter()
    .map(|layer| {
        let issue_count = issues
            .iter()
            .filter(|issue| issue.get("layer").and_then(Value::as_str) == Some(*layer))
            .count() as u32;
        let error_count = issues
            .iter()
            .filter(|issue| {
                issue.get("layer").and_then(Value::as_str) == Some(*layer) && is_error_issue(issue)
            })
            .count() as u32;
        let warning_count = issues
            .iter()
            .filter(|issue| {
                issue.get("layer").and_then(Value::as_str) == Some(*layer)
                    && issue.get("severity").and_then(Value::as_str) == Some("warning")
            })
            .count() as u32;
        ValidationLayerReportV1 {
            layer: (*layer).to_string(),
            passed: error_count == 0,
            issue_count,
            error_count: Some(error_count),
            warning_count: Some(warning_count),
        }
    })
    .collect()
}
