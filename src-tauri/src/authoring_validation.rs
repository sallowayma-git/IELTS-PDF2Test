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
    let layers_to_replace = sidecar_layers
        .iter()
        .filter_map(|layer| {
            layer
                .get("layer")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    // 缺省语义必须是**合并**，不是**替换**。
    //
    // 这里曾经有两个叠加的缺省，合起来构成一条静默改判通道：
    //   1. `layers` 缺失 ⇒ `layers_to_replace` 退化成 ["ReadingExamSourceV1","DomProtocol"]；
    //   2. `replaceExistingLayers` 缺失 ⇒ true。
    // 于是一份"什么都不说"的侧车输出（`{}`）会把 base 上这两层的 error 整层删掉，
    // 再由下面的 `!has_error_issues(merged)` 把 passed 重算成 true —— 校验器什么都没说，
    // 结论却从"不能发布"变成"可以发布"。
    //
    // `runtime_validation.rs:340` 处显式置 false 说明调用方本来就期望合并语义；缺省与它相反
    // 是纯粹的意外。现在把缺省对齐到实际期望，并把"空列表 = 替换默认两层"这条回退删掉：
    // 空列表就是"不替换任何层"。
    //
    // 替换能力本身保留（协议未变），但必须由调用方**显式**写 `replaceExistingLayers: true`。
    let replace_existing_layers = sidecar
        .get("replaceExistingLayers")
        .and_then(Value::as_bool)
        .unwrap_or(false);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn error_issue(layer: &str, message: &str) -> Value {
        json!({
            "issueId": format!("issue-{}", message),
            "severity": "error",
            "layer": layer,
            "path": "$.probe",
            "message": message
        })
    }

    fn base_report_with_two_errors() -> Value {
        json!({
            "jobId": "job-sidecar-overlay",
            "passed": false,
            "layers": [],
            "issues": [
                error_issue("ReadingExamSourceV1", "rust says q14 is missing"),
                error_issue("DomProtocol", "rust says dropzone is broken")
            ]
        })
    }

    fn issue_count(report: &Value) -> usize {
        report
            .get("issues")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0)
    }

    /// 反例 ①：空侧车输出（无 `layers`、无 `issues`、无 `replaceExistingLayers`）。
    ///
    /// 修前：`layers_to_replace` 退化为 `["ReadingExamSourceV1","DomProtocol"]`，
    /// `replaceExistingLayers` 缺省 true ⇒ 两条 Rust error 被删掉，`passed` 翻成 true。
    /// 修后：缺省是**合并**，Rust 的两条 error 必须原样保留。
    #[test]
    fn sidecar_report_without_layers_must_not_erase_rust_errors() {
        let mut base = base_report_with_two_errors();
        merge_sidecar_validation(&mut base, json!({}));

        assert_eq!(
            base.get("passed").and_then(Value::as_bool),
            Some(false),
            "空的侧车输出不得把 Rust 自查的 error 抹掉"
        );
        assert_eq!(
            issue_count(&base),
            2,
            "两层的 Rust error 必须全部保留：{}",
            serde_json::to_string_pretty(&base).unwrap_or_default()
        );
    }

    /// 反例 ②：侧车只报自己的 layer、不比 base 少一个字段，但 base 有 DomProtocol error。
    #[test]
    fn sidecar_issues_are_merged_not_substituted() {
        let mut base = base_report_with_two_errors();
        merge_sidecar_validation(
            &mut base,
            json!({
                "layers": [{"layer": "ReadingExamSourceV1", "passed": true, "issueCount": 0}],
                "issues": []
            }),
        );

        assert_eq!(base.get("passed").and_then(Value::as_bool), Some(false));
        assert_eq!(issue_count(&base), 2);
    }

    /// 反向：显式要求替换时，仍允许替换（能力保留，但必须显式）。
    #[test]
    fn explicit_replace_existing_layers_still_replaces() {
        let mut base = base_report_with_two_errors();
        merge_sidecar_validation(
            &mut base,
            json!({
                "layers": [{"layer": "ReadingExamSourceV1", "passed": true, "issueCount": 0}],
                "issues": [],
                "replaceExistingLayers": true
            }),
        );

        assert_eq!(
            base.get("passed").and_then(Value::as_bool),
            Some(false),
            "DomProtocol 未被列入替换范围，它的 error 必须留下"
        );
        assert_eq!(issue_count(&base), 1);
    }

    /// 侧车自带 warning 时必须并进来（不能被替换语义吞掉）。
    #[test]
    fn sidecar_warnings_are_preserved_in_merge() {
        let mut base = json!({
            "jobId": "job-sidecar-warning",
            "passed": true,
            "layers": [],
            "issues": []
        });
        merge_sidecar_validation(
            &mut base,
            json!({
                "issues": [{
                    "issueId": "issue-w",
                    "severity": "warning",
                    "layer": "ReadingExamSourceV1",
                    "path": "$.probe",
                    "message": "node parity warning"
                }]
            }),
        );

        assert_eq!(issue_count(&base), 1);
        assert_eq!(base.get("passed").and_then(Value::as_bool), Some(true));
    }
}
