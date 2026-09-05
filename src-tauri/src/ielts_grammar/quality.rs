use chrono::Utc;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

use crate::reading_source::ReadingExamSourceV1;
use crate::reading_source_v2::{compile_reading_source_v2, CompilerIssueV2};
use crate::schema::IeltsAuthoringIRV2;
use crate::validator::validate_reading_source_contract;

use super::issue_codes::*;

#[derive(Debug, Clone, Default)]
struct GroupEvaluation {
    score: f64,
}

#[derive(Debug, Clone, Default)]
struct SourceCoverageSummary {
    score: f64,
    significant_count: usize,
    assigned_count: usize,
    unassigned_ids: Vec<String>,
    ledger: Vec<Value>,
    physical_available: bool,
}

pub(crate) fn evaluate_quality(authoring: &Value, physical_shadow: Option<&Value>) -> Value {
    let groups = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let slots = authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let answer_key = authoring
        .get("answerKey")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut issues = Vec::new();
    let mut hard_failures = Vec::new();
    let mut task_scores = BTreeMap::new();
    let mut expected_numbers = BTreeSet::new();
    let mut actual_numbers = Vec::new();

    validate_exam_id(authoring, &mut issues, &mut hard_failures);
    validate_passage(authoring, &mut issues, &mut hard_failures);
    validate_assets(authoring, physical_shadow, &mut issues, &mut hard_failures);
    validate_identifier_and_reference_closure(authoring, &mut issues, &mut hard_failures);
    validate_provenance(authoring, &mut issues, &mut hard_failures);
    validate_scoring_semantics(authoring, &mut issues, &mut hard_failures);
    validate_source_ownership(authoring, physical_shadow, &mut issues, &mut hard_failures);

    if groups.is_empty() {
        push_issue(
            &mut issues,
            &mut hard_failures,
            issue(
                QUESTION_RANGE_UNPARSED,
                "blocking",
                "未找到可解析的 IELTS 题组范围。",
                "document",
                "document",
                Vec::new(),
                vec!["assign_role", "edit_text"],
            ),
        );
    }

    for group in &groups {
        let task_id = group
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or("unknown-task");
        let evaluation = evaluate_group(
            group,
            &slots,
            &answer_key,
            &mut expected_numbers,
            &mut actual_numbers,
            &mut issues,
            &mut hard_failures,
        );
        task_scores.insert(task_id.to_string(), round(evaluation.score));
    }

    let mut seen = BTreeSet::new();
    for number in actual_numbers {
        if !seen.insert(number) {
            push_issue(
                &mut issues,
                &mut hard_failures,
                issue(
                    QUESTION_NUMBER_DUPLICATE,
                    "blocking",
                    &format!("题号 {number} 被多个 slot 声明。"),
                    "document",
                    "document",
                    Vec::new(),
                    vec!["split_prompt", "edit_text"],
                ),
            );
        }
    }
    for expected in expected_numbers.iter().copied() {
        if !slots.values().any(|slot| {
            slot.get("questionNumber")
                .and_then(Value::as_u64)
                .is_some_and(|number| number as u32 == expected)
        }) {
            push_issue(
                &mut issues,
                &mut hard_failures,
                issue(
                    QUESTION_NUMBER_MISSING,
                    "blocking",
                    &format!("题组声明了题号 {expected}，但没有对应 AnswerSlot。"),
                    "document",
                    "document",
                    Vec::new(),
                    vec!["edit_text", "split_prompt"],
                ),
            );
        }
    }

    let source_summary = source_coverage_summary(authoring, physical_shadow);
    let source_coverage = source_summary.score;
    if !source_summary.physical_available {
        push_issue(
            &mut issues,
            &mut hard_failures,
            issue(
                PHYSICAL_SHADOW_MISSING,
                "warning",
                "缺少 DocumentIRV2 physical shadow，不能证明显著源区域覆盖完整。",
                "document",
                "document",
                Vec::new(),
                vec!["assign_role"],
            ),
        );
    } else if !source_summary.unassigned_ids.is_empty() {
        let mut coverage_issue = issue(
            SIGNIFICANT_REGION_UNASSIGNED,
            "blocking",
            "仍有显著源区域未被题目、passage 或有理由的忽略记录解释。",
            "document",
            "document",
            Vec::new(),
            vec!["assign_role"],
        );
        coverage_issue["details"] = json!({
            "significantSourceNodeCount": source_summary.significant_count,
            "assignedSourceNodeCount": source_summary.assigned_count,
            "unassignedSourceNodeIds": &source_summary.unassigned_ids
        });
        push_issue(&mut issues, &mut hard_failures, coverage_issue);
    }

    let compiler_probes = evaluate_compiler_probes(authoring);
    append_compiler_probe_issue(
        &compiler_probes,
        "v2Runtime",
        RUNTIME_COMPILER_FAILED,
        "ReadingExamSourceV2 runtime compiler 或其 schema validation 失败。",
        &mut issues,
        &mut hard_failures,
    );
    append_compiler_probe_issue(
        &compiler_probes,
        "v1Compatibility",
        V1_COMPATIBILITY_COMPILER_FAILED,
        "V1 compatibility compiler 或 ReadingExamSourceV1 validation 失败。",
        &mut issues,
        &mut hard_failures,
    );

    let document_score = if task_scores.is_empty() {
        0.0
    } else {
        task_scores.values().sum::<f64>() / task_scores.len() as f64
    };
    let has_blocking = !hard_failures.is_empty();
    let has_low_task_score = task_scores.values().any(|score| *score < 0.92);
    // Only UNRESOLVED, BLOCKING issues hold readiness back. This mirrors the export gate's own
    // `unresolved_blockers` predicate exactly (authoring_v2_commands: severity == Blocking and no
    // resolution in {resolved, ignored}).
    //
    // The previous `!issues.is_empty()` disagreed with that gate in two ways that both blocked
    // good documents: a single `info` note made an otherwise-perfect paper unpublishable, and
    // resolving or ignoring an issue changed nothing -- so `preserve_issue_resolutions` went to the
    // trouble of carrying resolutions forward across every save that nothing ever read.
    let unresolved_blocking_issues = issues
        .iter()
        .filter(|issue| {
            issue.get("severity").and_then(Value::as_str) == Some("blocking")
                && issue
                    .get("details")
                    .and_then(|details| details.get("resolution"))
                    .and_then(Value::as_str)
                    .is_none_or(|resolution| !matches!(resolution, "resolved" | "ignored"))
        })
        .count();
    let state = if has_blocking {
        "blocked"
    } else if document_score < 0.95
        || has_low_task_score
        || source_coverage < 0.995
        || unresolved_blocking_issues > 0
    {
        "review_required"
    } else {
        "ready"
    };
    json!({
        "schemaVersion": "QualityReportV2",
        "state": state,
        "documentScore": round(document_score),
        "sourceCoverage": round(source_coverage),
        "coverageLedger": source_summary.ledger,
        "coverageStatus": {
            "physicalShadow": if source_summary.physical_available { "available" } else { "missing" },
            "complete": source_summary.physical_available && source_summary.unassigned_ids.is_empty(),
            "significantSourceNodeCount": source_summary.significant_count,
            "explainedSourceNodeCount": source_summary.assigned_count,
            "unassignedSourceNodeIds": source_summary.unassigned_ids
        },
        "compilerProbes": compiler_probes,
        "taskScores": task_scores,
        "hardFailures": hard_failures,
        "issues": issues,
        "metrics": {
            "taskCount": groups.len() as f64,
            "slotCount": slots.len() as f64,
            "expectedQuestionCount": expected_numbers.len() as f64,
            "sourceCoverage": round(source_coverage),
            "significantSourceNodeCount": source_summary.significant_count as f64,
            "assignedSourceNodeCount": source_summary.assigned_count as f64,
            "unassignedSourceNodeCount": source_summary.unassigned_ids.len() as f64
        },
        "evaluatedAt": Utc::now().to_rfc3339(),
        "evaluatorVersion": "phase4-pr07-hard-gate-v2"
    })
}

fn validate_scoring_semantics(
    authoring: &Value,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let slots = authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for task in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for response in task
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let response_id = response
                .get("responseGroupId")
                .and_then(Value::as_str)
                .unwrap_or("unknown-response");
            let scoring_policy = response.get("scoringPolicy").and_then(Value::as_str);
            if !matches!(
                scoring_policy,
                Some(
                    "per_slot_binary"
                        | "per_slot_ielts_normalized"
                        | "exact_set"
                        | "all_or_nothing"
                )
            ) {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        SCORING_POLICY_UNRESOLVED,
                        "blocking",
                        "ResponseGroup 缺少可执行且明确的 scoring policy。",
                        "response_group",
                        response_id,
                        anchors_from(response),
                        vec!["edit_text"],
                    ),
                );
            }
            for slot_id in response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let participation = slots
                    .get(slot_id)
                    .and_then(|slot| slot.get("participation"))
                    .and_then(Value::as_str);
                let code = match participation {
                    Some("example") => Some(EXAMPLE_SCORING_CONFLICT),
                    Some("non_scoring") => Some(SCORING_POLICY_UNRESOLVED),
                    _ => None,
                };
                if let Some(code) = code {
                    push_issue(
                        issues,
                        hard_failures,
                        issue(
                            code,
                            "blocking",
                            if code == EXAMPLE_SCORING_CONFLICT {
                                "Example slot 被纳入当前计分 response group。"
                            } else {
                                "Non-scoring slot 缺少可证明的排除计分策略。"
                            },
                            "slot",
                            slot_id,
                            slots.get(slot_id).map(anchors_from).unwrap_or_default(),
                            vec!["edit_text"],
                        ),
                    );
                }
            }
        }
    }
}

fn validate_exam_id(authoring: &Value, issues: &mut Vec<Value>, hard_failures: &mut Vec<String>) {
    let exam_id = authoring
        .pointer("/exam/examId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !crate::util::is_safe_path_segment(exam_id) {
        push_issue(
            issues,
            hard_failures,
            issue(
                EXAM_ID_INVALID,
                "blocking",
                "examId 必须是非空、唯一可寻址且路径安全的 ASCII 标识。",
                "document",
                if exam_id.is_empty() {
                    "document"
                } else {
                    exam_id
                },
                Vec::new(),
                vec!["edit_text"],
            ),
        );
    }
}

fn validate_passage(authoring: &Value, issues: &mut Vec<Value>, hard_failures: &mut Vec<String>) {
    if authoring.get("modality").and_then(Value::as_str) == Some("listening") {
        return;
    }
    let content = authoring.pointer("/passage/content");
    let mut text = Vec::new();
    if let Some(nodes) = content.and_then(Value::as_array) {
        collect_text(nodes, &mut text);
    }
    let has_text = text.iter().any(|value| !is_prompt_placeholder(value));
    let has_visual_fallback = content.is_some_and(|value| {
        node_contains_type(value, "image")
            || node_contains_type(value, "figure")
            || node_contains_type(value, "diagram")
    });
    if !has_text && !has_visual_fallback {
        push_issue(
            issues,
            hard_failures,
            issue(
                PASSAGE_CONTENT_MISSING,
                "blocking",
                "Reading passage 必须包含有效文本或明确的 visual fallback。",
                "document",
                authoring
                    .pointer("/exam/examId")
                    .and_then(Value::as_str)
                    .unwrap_or("document"),
                Vec::new(),
                vec!["edit_text", "confirm_figure"],
            ),
        );
    }
}

fn validate_identifier_and_reference_closure(
    authoring: &Value,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let slots = authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let slot_ids = slots.keys().cloned().collect::<BTreeSet<_>>();
    let answer_ids = authoring
        .get("answerKey")
        .and_then(Value::as_object)
        .map(|answers| answers.keys().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();

    for (slot_key, slot) in &slots {
        if slot_key.trim().is_empty()
            || slot.get("slotId").and_then(Value::as_str) != Some(slot_key.as_str())
        {
            push_issue(
                issues,
                hard_failures,
                issue(
                    SLOT_ID_MISMATCH,
                    "blocking",
                    "answerSlots map key 必须与非空 slotId 完全一致。",
                    "slot",
                    slot_key,
                    anchors_from(slot),
                    vec!["edit_text"],
                ),
            );
        }
    }
    for slot_id in slot_ids.difference(&answer_ids) {
        push_issue(
            issues,
            hard_failures,
            issue(
                ANSWER_KEY_MISSING_SLOT,
                "blocking",
                "AnswerSlot 没有对应的 answerKey entry。",
                "slot",
                slot_id,
                slots.get(slot_id).map(anchors_from).unwrap_or_default(),
                vec!["enter_answer"],
            ),
        );
    }
    for answer_id in answer_ids.difference(&slot_ids) {
        push_issue(
            issues,
            hard_failures,
            issue(
                ANSWER_KEY_ORPHAN_SLOT,
                "blocking",
                "answerKey 引用了不存在的 AnswerSlot。",
                "slot",
                answer_id,
                Vec::new(),
                vec!["enter_answer"],
            ),
        );
    }

    let mut task_ids = BTreeSet::new();
    let mut response_ids = BTreeSet::new();
    let mut assignments = slot_ids
        .iter()
        .map(|slot_id| (slot_id.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    for group in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let task_id = group
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if task_id.is_empty() || !task_ids.insert(task_id.to_string()) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    TASK_ID_DUPLICATE,
                    "blocking",
                    "taskId 必须非空且在文档内唯一。",
                    "task",
                    if task_id.is_empty() {
                        "document"
                    } else {
                        task_id
                    },
                    anchors_from(group),
                    vec!["edit_text"],
                ),
            );
        }
        if group.get("taskType").and_then(Value::as_str)
            != group
                .pointer("/instructionSignature/taskType")
                .and_then(Value::as_str)
        {
            push_issue(
                issues,
                hard_failures,
                issue(
                    TASK_TYPE_SIGNATURE_MISMATCH,
                    "blocking",
                    "taskType 与 instruction signature 声明冲突。",
                    "task",
                    if task_id.is_empty() {
                        "document"
                    } else {
                        task_id
                    },
                    anchors_from(group),
                    vec!["edit_text"],
                ),
            );
        }
        let bank_id = group
            .pointer("/optionBank/optionBankId")
            .and_then(Value::as_str);
        for response in group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let response_id = response
                .get("responseGroupId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if response_id.is_empty() || !response_ids.insert(response_id.to_string()) {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        RESPONSE_GROUP_ID_DUPLICATE,
                        "blocking",
                        "responseGroupId 必须非空且在文档内唯一。",
                        "response_group",
                        if response_id.is_empty() {
                            task_id
                        } else {
                            response_id
                        },
                        anchors_from(response),
                        vec!["edit_text"],
                    ),
                );
            }
            if let Some(reference) = response.get("optionBankRef").and_then(Value::as_str) {
                if bank_id != Some(reference) {
                    push_issue(
                        issues,
                        hard_failures,
                        issue(
                            OPTION_BANK_REFERENCE_MISSING,
                            "blocking",
                            "optionBankRef 未闭合到同一 task group 的 option bank。",
                            "response_group",
                            if response_id.is_empty() {
                                task_id
                            } else {
                                response_id
                            },
                            anchors_from(response),
                            vec!["attach_option_bank"],
                        ),
                    );
                }
            }
            let mut local = BTreeSet::new();
            for slot_id in response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !slot_ids.contains(slot_id) || !local.insert(slot_id) {
                    push_issue(
                        issues,
                        hard_failures,
                        issue(
                            SLOT_REFERENCE_MISSING,
                            "blocking",
                            "response group 引用了不存在或重复的 slot。",
                            "response_group",
                            if response_id.is_empty() {
                                task_id
                            } else {
                                response_id
                            },
                            anchors_from(response),
                            vec!["edit_text", "split_prompt"],
                        ),
                    );
                } else if let Some(count) = assignments.get_mut(slot_id) {
                    *count += 1;
                }
            }
        }
    }
    for (slot_id, count) in assignments {
        if count != 1 {
            push_issue(
                issues,
                hard_failures,
                issue(
                    SLOT_GROUP_ASSIGNMENT_INVALID,
                    "blocking",
                    "每个 AnswerSlot 必须恰好属于一个 response group。",
                    "slot",
                    &slot_id,
                    slots.get(&slot_id).map(anchors_from).unwrap_or_default(),
                    vec!["split_prompt", "edit_text"],
                ),
            );
        }
    }
}

fn validate_provenance(
    authoring: &Value,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    for group in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let task_id = group
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or("document");
        let instructions = group.get("instructions").and_then(Value::as_array);
        if instructions.is_none_or(|nodes| {
            nodes.is_empty() || nodes.iter().any(|node| !has_direct_provenance(node))
        }) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    INSTRUCTION_PROVENANCE_MISSING,
                    "blocking",
                    "instructions 必须非空，且每个 instruction 都必须有 source anchor 或明确 manual provenance。",
                    "task",
                    task_id,
                    Vec::new(),
                    vec!["assign_role", "edit_text"],
                ),
            );
        }
        let signature_anchors = group
            .pointer("/instructionSignature/evidenceAnchors")
            .and_then(Value::as_array);
        if signature_anchors.is_none_or(Vec::is_empty) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    INSTRUCTION_SIGNATURE_EVIDENCE_MISSING,
                    "blocking",
                    "instructionSignature.evidenceAnchors 必须包含直接源证据。",
                    "task",
                    task_id,
                    Vec::new(),
                    vec!["assign_role", "edit_text"],
                ),
            );
        }
        let stimulus = group.get("stimulus").and_then(Value::as_array);
        for response in group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let response_id = response
                .get("responseGroupId")
                .and_then(Value::as_str)
                .unwrap_or(task_id);
            let context = response
                .get("prompt")
                .and_then(Value::as_array)
                .filter(|nodes| !nodes.is_empty())
                .or(stimulus);
            if context.is_some_and(|nodes| nodes.iter().any(|node| !has_direct_provenance(node))) {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        PROVENANCE_MISSING,
                        "blocking",
                        "prompt/stimulus 缺少 source anchor 或明确 manual provenance。",
                        "response_group",
                        response_id,
                        anchors_from(response),
                        vec!["assign_role", "edit_text"],
                    ),
                );
            }
            if let Some(options) = response.get("options").and_then(Value::as_array) {
                validate_option_provenance(options, response_id, issues, hard_failures);
            }
        }
        if let Some(options) = group
            .pointer("/optionBank/options")
            .and_then(Value::as_array)
        {
            validate_option_provenance(options, task_id, issues, hard_failures);
        }
    }
    for (slot_id, slot) in authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if !has_direct_provenance(slot) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    PROVENANCE_MISSING,
                    "blocking",
                    "AnswerSlot 缺少 source anchor 或明确 manual provenance。",
                    "slot",
                    slot_id,
                    Vec::new(),
                    vec!["assign_role", "edit_text"],
                ),
            );
        }
    }
}

fn validate_option_provenance(
    options: &[Value],
    target_id: &str,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    for option in options {
        if !has_direct_provenance(option) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    PROVENANCE_MISSING,
                    "blocking",
                    "option 缺少 source anchor 或明确 manual provenance。",
                    "response_group",
                    option
                        .get("optionId")
                        .and_then(Value::as_str)
                        .unwrap_or(target_id),
                    Vec::new(),
                    vec!["assign_role", "edit_text"],
                ),
            );
        }
    }
}

fn has_direct_provenance(value: &Value) -> bool {
    if value.get("provenanceStatus").and_then(Value::as_str) == Some("manual")
        || value
            .get("sourceAnchors")
            .and_then(Value::as_array)
            .is_some_and(|anchors| !anchors.is_empty())
        || value.get("sourceAnchor").is_some_and(Value::is_object)
    {
        true
    } else {
        false
    }
}

fn anchors_from(value: &Value) -> Vec<Value> {
    value
        .get("sourceAnchors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn evaluate_compiler_probes(authoring: &Value) -> Value {
    let typed = typed_authoring_for_probe(authoring);
    let (v2_probe, v1_probe) = match typed {
        Ok(typed) => {
            let v2 = match compile_reading_source_v2(&typed) {
                Ok(runtime) => {
                    let round_trip = serde_json::to_value(&runtime)
                        .map_err(|error| error.to_string())
                        .and_then(|value| {
                            serde_json::from_value::<crate::reading_source_v2::ReadingExamSourceV2>(
                                value,
                            )
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                        });
                    match round_trip {
                        Ok(()) => {
                            compiler_probe("passed", "ReadingExamSourceV2", Vec::new(), Vec::new())
                        }
                        Err(error) => compiler_probe(
                            "failed",
                            "ReadingExamSourceV2",
                            vec!["RUNTIME_SCHEMA_ROUND_TRIP_FAILED".to_string()],
                            vec![error],
                        ),
                    }
                }
                Err(compiler_issues) => compiler_probe_from_v2_issues(compiler_issues),
            };
            let v1 = probe_v1_compatibility(authoring);
            (v2, v1)
        }
        Err(error) => {
            let failed = compiler_probe(
                "failed",
                "IeltsAuthoringIRV2",
                vec![AUTHORING_SCHEMA_INVALID.to_string()],
                vec![error],
            );
            (failed.clone(), failed)
        }
    };
    json!({"v2Runtime": v2_probe, "v1Compatibility": v1_probe})
}

fn typed_authoring_for_probe(authoring: &Value) -> Result<IeltsAuthoringIRV2, String> {
    let mut candidate = authoring.clone();
    candidate["quality"] = probe_quality_placeholder();
    let typed = serde_json::from_value::<IeltsAuthoringIRV2>(candidate)
        .map_err(|error| format!("IeltsAuthoringIRV2 serde validation failed: {error}"))?;
    if !typed.is_supported_schema_version() {
        return Err(format!(
            "unsupported authoring schema version: {}",
            typed.schema_version
        ));
    }
    Ok(typed)
}

fn probe_quality_placeholder() -> Value {
    json!({
        "schemaVersion": "QualityReportV2",
        "state": "review_required",
        "documentScore": 0.0,
        "sourceCoverage": 0.0,
        "coverageLedger": [],
        "coverageStatus": {
            "physicalShadow": "missing",
            "complete": false,
            "significantSourceNodeCount": 0,
            "explainedSourceNodeCount": 0,
            "unassignedSourceNodeIds": []
        },
        "compilerProbes": {
            "v2Runtime": {"status":"failed","schemaVersion":"ReadingExamSourceV2","issueCodes":["PROBE_PENDING"],"details":["probe pending"]},
            "v1Compatibility": {"status":"failed","schemaVersion":"ReadingExamSourceV1","issueCodes":["PROBE_PENDING"],"details":["probe pending"]}
        },
        "taskScores": {},
        "hardFailures": [],
        "issues": [],
        "metrics": {},
        "evaluatedAt": Utc::now().to_rfc3339(),
        "evaluatorVersion": "phase4-pr07-probe-placeholder"
    })
}

fn compiler_probe_from_v2_issues(issues: Vec<CompilerIssueV2>) -> Value {
    let mut codes = issues
        .iter()
        .map(|issue| issue.code.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    codes.sort();
    compiler_probe(
        "failed",
        "ReadingExamSourceV2",
        codes,
        issues
            .into_iter()
            .map(|issue| format!("{}:{}:{}", issue.code, issue.target_id, issue.message))
            .collect(),
    )
}

fn compiler_probe(
    status: &str,
    schema_version: &str,
    issue_codes: Vec<String>,
    details: Vec<String>,
) -> Value {
    json!({
        "status": status,
        "schemaVersion": schema_version,
        "issueCodes": issue_codes,
        "details": details
    })
}

fn append_compiler_probe_issue(
    probes: &Value,
    probe_key: &str,
    code: &str,
    message: &str,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let Some(probe) = probes.get(probe_key) else {
        return;
    };
    if probe.get("status").and_then(Value::as_str) != Some("failed") {
        return;
    }
    let mut value = issue(
        code,
        "blocking",
        message,
        "document",
        "document",
        Vec::new(),
        vec!["edit_text"],
    );
    value["details"] = json!({
        "probe": probe_key,
        "schemaVersion": probe.get("schemaVersion").cloned().unwrap_or(Value::Null),
        "issueCodes": probe.get("issueCodes").cloned().unwrap_or_else(|| json!([])),
        "compilerDetails": probe.get("details").cloned().unwrap_or_else(|| json!([]))
    });
    push_issue(issues, hard_failures, value);
    if probe
        .get("issueCodes")
        .and_then(Value::as_array)
        .is_some_and(|codes| {
            codes
                .iter()
                .any(|item| item.as_str() == Some(AUTHORING_SCHEMA_INVALID))
        })
    {
        push_issue(
            issues,
            hard_failures,
            issue(
                AUTHORING_SCHEMA_INVALID,
                "blocking",
                "IeltsAuthoringIRV2 无法通过 typed schema round-trip。",
                "document",
                "document",
                Vec::new(),
                vec!["edit_text"],
            ),
        );
    }
}

fn probe_v1_compatibility(authoring: &Value) -> Value {
    let source = compile_v1_compatibility_shadow(authoring);
    let typed_result = serde_json::from_value::<ReadingExamSourceV1>(source.clone());
    let mut details = Vec::new();
    let mut codes = BTreeSet::new();
    if let Err(error) = typed_result {
        codes.insert("V1_COMPAT_TYPED_SCHEMA_INVALID".to_string());
        details.push(error.to_string());
    }
    for issue in validate_reading_source_contract(&source) {
        codes.insert(
            issue
                .get("layer")
                .and_then(Value::as_str)
                .map(|layer| format!("V1_{}_VALIDATION_FAILED", layer.to_ascii_uppercase()))
                .unwrap_or_else(|| "V1_SCHEMA_VALIDATION_FAILED".to_string()),
        );
        details.push(format!(
            "{}:{}",
            issue.get("path").and_then(Value::as_str).unwrap_or("$"),
            issue
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("validation failed")
        ));
    }
    if codes.is_empty() {
        compiler_probe("passed", "ReadingExamSourceV1", Vec::new(), Vec::new())
    } else {
        compiler_probe(
            "failed",
            "ReadingExamSourceV1",
            codes.into_iter().collect(),
            details,
        )
    }
}

fn compile_v1_compatibility_shadow(authoring: &Value) -> Value {
    let slots = authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut ordered_slots = slots.iter().collect::<Vec<_>>();
    ordered_slots.sort_by_key(|(slot_id, slot)| {
        (
            slot.get("questionNumber")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX),
            (*slot_id).clone(),
        )
    });
    let question_order = ordered_slots
        .iter()
        .map(|(slot_id, _)| (*slot_id).clone())
        .collect::<Vec<_>>();
    let question_display_map = ordered_slots
        .iter()
        .map(|(slot_id, slot)| {
            (
                (*slot_id).clone(),
                Value::String(
                    slot.get("displayLabel")
                        .and_then(Value::as_str)
                        .unwrap_or(slot_id)
                        .to_string(),
                ),
            )
        })
        .collect::<Map<_, _>>();
    let answer_key = question_order
        .iter()
        .map(|slot_id| {
            (
                slot_id.clone(),
                v1_answer_value(authoring.pointer(&format!("/answerKey/{slot_id}"))),
            )
        })
        .collect::<Map<_, _>>();
    let question_groups = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|group| v1_group_value(group, &slots))
        .collect::<Vec<_>>();
    let passage_html = authoring
        .pointer("/passage/content")
        .map(render_content_text)
        .unwrap_or_default();
    let exam_id = authoring
        .pointer("/exam/examId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    json!({
        "schemaVersion": "ReadingExamSourceV1",
        "examId": exam_id,
        "meta": {
            "title": authoring.pointer("/exam/title").and_then(Value::as_str).unwrap_or("Untitled Reading"),
            "category": authoring.pointer("/exam/category").and_then(Value::as_str).unwrap_or("P1"),
            "frequency": authoring.pointer("/exam/frequency").and_then(Value::as_str).unwrap_or("medium"),
            "pdfFilename": "",
            "legacyPath": "",
            "legacyFilename": "",
            "questionIntroHtml": "<h3>Questions</h3>",
            "questionUmbrellaRanges": []
        },
        "passage": {"blocks":[{"blockId":"passage-main","kind":"html","html":format!("<p>{}</p>", crate::html_escape(&passage_html))}]},
        "questionGroups": question_groups,
        "answerKey": answer_key,
        "sourceRefs": {"primaryHtml":"","primaryProvider":"quality_gate_v2_probe","shuiHtml":null,"shuiPdf":"","ieltsHtml":null},
        "audit": {"matchStatus":"shadow_probe","matchConfidence":0.0,"verifiedAt":null,"notes":"PR-07 read-only V1 compatibility probe"},
        "questionOrder": question_order,
        "questionDisplayMap": question_display_map
    })
}

fn v1_group_value(group: &Value, slots: &Map<String, Value>) -> Value {
    let task_id = group
        .get("taskId")
        .and_then(Value::as_str)
        .unwrap_or("task");
    let kind = v1_question_kind(
        group
            .get("taskType")
            .and_then(Value::as_str)
            .unwrap_or("short_answer"),
    );
    let slot_ids = group
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
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut body = format!("<section id=\"{}\">", crate::html_escape(task_id));
    let lead = group_prompt_text(group);
    if !lead.is_empty() {
        body.push_str(&format!("<p>{}</p>", crate::html_escape(&lead)));
    }
    for response in group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let response_slot_ids = response
            .get("slotIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let shared_unordered = response.get("assignment").and_then(Value::as_str)
            == Some("unordered_set")
            && response_slot_ids.len() > 1;
        if shared_unordered {
            let displays = response_slot_ids
                .iter()
                .map(|slot_id| {
                    slots
                        .get(*slot_id)
                        .and_then(|slot| slot.get("displayLabel"))
                        .and_then(Value::as_str)
                        .unwrap_or(slot_id)
                })
                .collect::<Vec<_>>()
                .join(" and ");
            let shared_name = response_slot_ids.join("_");
            let shared_question_ids = response_slot_ids.join(",");
            body.push_str(&format!(
                "<div class=\"question shared-response\"><strong>{}</strong>",
                crate::html_escape(&displays)
            ));
            for (label, text) in options_for_slot(group, response_slot_ids[0]) {
                body.push_str(&format!(
                    "<label><input type=\"checkbox\" name=\"{}\" data-question-ids=\"{}\" value=\"{}\"> {} {}</label>",
                    crate::html_escape(&shared_name),
                    crate::html_escape(&shared_question_ids),
                    crate::html_escape(&label),
                    crate::html_escape(&label),
                    crate::html_escape(&text)
                ));
            }
            body.push_str("</div>");
            continue;
        }
        for slot_id in response_slot_ids {
            let display = slots
                .get(slot_id)
                .and_then(|slot| slot.get("displayLabel"))
                .and_then(Value::as_str)
                .unwrap_or(slot_id);
            let options = options_for_slot(group, slot_id);
            body.push_str(&format!(
                "<div class=\"question\"><strong>{}</strong>",
                crate::html_escape(display)
            ));
            if options.is_empty() {
                body.push_str(&format!(
                    "<input type=\"text\" id=\"{}_input\" name=\"{}\">",
                    crate::html_escape(slot_id),
                    crate::html_escape(slot_id)
                ));
            } else {
                body.push_str(&format!(
                    "<select name=\"{}\" id=\"{}_input\">",
                    crate::html_escape(slot_id),
                    crate::html_escape(slot_id)
                ));
                for (label, text) in options {
                    body.push_str(&format!(
                        "<option value=\"{}\">{} {}</option>",
                        crate::html_escape(&label),
                        crate::html_escape(&label),
                        crate::html_escape(&text)
                    ));
                }
                body.push_str("</select>");
            }
            body.push_str("</div>");
        }
    }
    body.push_str("</section>");
    json!({
        "groupId": task_id,
        "kind": kind,
        "questionIds": slot_ids,
        "bodyHtml": body,
        "leadHtml": format!("<p>{}</p>", crate::html_escape(&lead)),
        "allowOptionReuse": group.pointer("/instructionSignature/allowOptionReuse").and_then(Value::as_bool).unwrap_or(false)
    })
}

fn v1_question_kind(task_type: &str) -> &'static str {
    match task_type {
        "multiple_choice" => "multi_choice",
        "matching_headings" => "heading_matching",
        "matching_information" => "matching_information",
        "matching_features" | "matching_sentence_endings" => "matching",
        "classification" => "classification",
        "summary_completion" | "note_completion" | "form_completion" | "flowchart_completion" => {
            "summary_completion"
        }
        "table_completion" => "table_completion",
        "diagram_label_completion" | "plan_map_label_completion" => "diagram_completion",
        "sentence_completion" => "sentence_completion",
        "true_false_not_given" => "true_false_not_given",
        "yes_no_not_given" => "yes_no_not_given",
        "single_choice" => "single_choice",
        _ => "short_answer",
    }
}

fn options_for_slot(group: &Value, slot_id: &str) -> Vec<(String, String)> {
    let response = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(slot_id)))
        });
    let options = response
        .and_then(|response| response.get("options").and_then(Value::as_array))
        .or_else(|| {
            group
                .pointer("/optionBank/options")
                .and_then(Value::as_array)
        });
    options
        .into_iter()
        .flatten()
        .map(|option| {
            (
                option
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                option
                    .get("content")
                    .map(render_content_text)
                    .unwrap_or_default(),
            )
        })
        .collect()
}

fn v1_answer_value(answer: Option<&Value>) -> Value {
    let Some(answer) = answer else {
        return Value::String(String::new());
    };
    let values = match answer.get("kind").and_then(Value::as_str) {
        Some("option") => answer.get("labels"),
        Some("text") => answer.get("values"),
        _ => None,
    }
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
    if values.len() == 1 {
        values[0].clone()
    } else {
        Value::Array(values)
    }
}

fn render_content_text(value: &Value) -> String {
    let mut parts = Vec::new();
    collect_text_value(value, &mut parts);
    parts
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_text_value(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| collect_text_value(item, out)),
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                out.push(text.to_string());
            }
            for (key, child) in object {
                if key != "text" {
                    collect_text_value(child, out);
                }
            }
        }
        _ => {}
    }
}

fn evaluate_group(
    group: &Value,
    slots: &Map<String, Value>,
    answer_key: &Map<String, Value>,
    expected_numbers: &mut BTreeSet<u32>,
    actual_numbers: &mut Vec<u32>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) -> GroupEvaluation {
    let hard_failure_count_before = hard_failures.len();
    let task_id = group
        .get("taskId")
        .and_then(Value::as_str)
        .unwrap_or("unknown-task");
    let anchors = group
        .get("sourceAnchors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let signature = group.get("instructionSignature");
    let task_type = signature
        .and_then(|value| value.get("taskType"))
        .and_then(Value::as_str)
        .unwrap_or("short_answer");
    let expected = signature
        .and_then(|value| value.get("expectedQuestionNumbers"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .map(|number| number as u32)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if group
        .get("recognitionWarnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|warning| warning.starts_with("task_type_conflict:"))
    {
        push_issue(
            issues,
            hard_failures,
            issue(
                TASK_TYPE_CONFLICT,
                "blocking",
                "题型指令与恢复出的结构证据冲突，必须确认题型后才能发布。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "assign_role"],
            ),
        );
    }
    expected_numbers.extend(expected.iter().copied());
    let group_slot_ids = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|slot_id| slot_id.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();
    let unique_group_slot_ids = group_slot_ids.iter().cloned().collect::<BTreeSet<_>>();
    let expected_slot_count = signature
        .and_then(|value| value.get("expectedSlotCount"))
        .and_then(Value::as_u64)
        .unwrap_or(expected.len() as u64) as usize;
    let display_numbers = display_range_numbers(group.get("displayRange"));
    if expected_slot_count != unique_group_slot_ids.len()
        || (!display_numbers.is_empty()
            && display_numbers != expected.iter().copied().collect::<BTreeSet<_>>())
    {
        push_issue(
            issues,
            hard_failures,
            issue(
                CARDINALITY_SLOT_MISMATCH,
                "blocking",
                "displayRange、expectedSlotCount、expectedQuestionNumbers 与实际 slots 必须一致。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "split_prompt"],
            ),
        );
    }
    for slot_id in &group_slot_ids {
        if let Some(slot) = slots.get(slot_id) {
            if let Some(number) = slot.get("questionNumber").and_then(Value::as_u64) {
                actual_numbers.push(number as u32);
            }
        }
    }
    let slot_coverage = if expected.is_empty() {
        0.0
    } else {
        expected
            .iter()
            .filter(|number| {
                group_slot_ids.iter().any(|slot_id| {
                    slots
                        .get(slot_id)
                        .and_then(|slot| slot.get("questionNumber"))
                        .and_then(Value::as_u64)
                        .is_some_and(|actual| actual as u32 == **number)
                })
            })
            .count() as f64
            / expected.len() as f64
    };
    if slot_coverage < 1.0 {
        push_issue(
            issues,
            hard_failures,
            issue(
                QUESTION_NUMBER_MISSING,
                "blocking",
                "题组的 expected question numbers 与 slots 不一致。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "split_prompt"],
            ),
        );
    }

    let prompt_text = group_prompt_text(group);
    let prompt_coverage = if prompt_text.is_empty() { 0.0 } else { 1.0 };
    if prompt_text.is_empty() && !group_slot_ids.is_empty() {
        push_issue(
            issues,
            hard_failures,
            issue(
                PROMPT_EMPTY,
                "blocking",
                "计分 slot 没有可定位的题干或 stimulus。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "split_prompt"],
            ),
        );
    }
    if group_has_ambiguous_prompt(group) && !group_slot_ids.is_empty() {
        push_issue(
            issues,
            hard_failures,
            issue(
                PROMPT_BOUNDARY_AMBIGUOUS,
                "blocking",
                "题干边界无法从 source evidence 中确定。",
                "task",
                task_id,
                anchors.clone(),
                vec!["split_prompt", "edit_text"],
            ),
        );
    }

    let signature_confidence = signature
        .and_then(|value| value.get("confidence"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if signature_confidence < 0.7 {
        push_issue(
            issues,
            hard_failures,
            issue(
                INSTRUCTION_SIGNATURE_UNRESOLVED,
                "warning",
                "题型 instruction signature 证据不足。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "assign_role"],
            ),
        );
    }

    let option_completeness = validate_options(
        group,
        task_type,
        task_id,
        anchors.clone(),
        issues,
        hard_failures,
    );
    let cardinality_score = validate_cardinality(
        group,
        task_type,
        task_id,
        anchors.clone(),
        issues,
        hard_failures,
    );
    if is_completion_type(task_type) {
        if signature.and_then(|value| value.get("wordLimit")).is_none() {
            push_issue(
                issues,
                hard_failures,
                issue(
                    WORD_LIMIT_UNPARSED,
                    "blocking",
                    "completion 题组未解析出 IELTS word limit。",
                    "task",
                    task_id,
                    anchors.clone(),
                    vec!["edit_text", "confirm_table"],
                ),
            );
        }
        validate_completion_host(
            group,
            slots,
            task_id,
            anchors.clone(),
            issues,
            hard_failures,
        );
    }
    validate_answers(
        group,
        answer_key,
        task_id,
        anchors.clone(),
        signature,
        issues,
        hard_failures,
    );
    validate_visual_task(
        group,
        task_type,
        task_id,
        anchors.clone(),
        issues,
        hard_failures,
    );

    let source_anchor_coverage = if anchors.is_empty() { 0.0 } else { 1.0 };
    let type_consistency = if task_type == "short_answer" {
        0.72
    } else {
        1.0
    };
    let score = 0.18 * signature_confidence
        + 0.18 * slot_coverage
        + 0.16 * prompt_coverage
        + 0.14 * option_completeness
        + 0.10 * source_anchor_coverage
        + 0.08 * cardinality_score
        + 0.08 * type_consistency
        + 0.08
            * if hard_failures.len() == hard_failure_count_before {
                1.0
            } else {
                0.0
            };
    GroupEvaluation {
        score: score.clamp(0.0, 1.0),
    }
}

fn display_range_numbers(value: Option<&Value>) -> BTreeSet<u32> {
    let Some(value) = value else {
        return BTreeSet::new();
    };
    match value.get("kind").and_then(Value::as_str) {
        Some("range") => {
            let start = value
                .get("start")
                .and_then(Value::as_u64)
                .unwrap_or_default() as u32;
            let end = value.get("end").and_then(Value::as_u64).unwrap_or_default() as u32;
            if start == 0 || end < start {
                BTreeSet::new()
            } else {
                (start..=end).collect()
            }
        }
        Some("set") => value
            .get("values")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|number| number as u32)
            .collect(),
        Some("mixed") => {
            let mut output = BTreeSet::new();
            for item in value
                .get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(number) = item.as_u64() {
                    output.insert(number as u32);
                } else if let (Some(start), Some(end)) = (
                    item.get("start").and_then(Value::as_u64),
                    item.get("end").and_then(Value::as_u64),
                ) {
                    if start > 0 && end >= start {
                        output.extend((start as u32)..=(end as u32));
                    }
                }
            }
            output
        }
        _ => BTreeSet::new(),
    }
}

fn validate_options(
    group: &Value,
    task_type: &str,
    task_id: &str,
    anchors: Vec<Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) -> f64 {
    let expected_labels = group
        .pointer("/instructionSignature/optionAlphabet")
        .and_then(Value::as_str)
        .and_then(expected_labels_from_alphabet);
    let response_groups = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let choice_required = matches!(
        task_type,
        "single_choice"
            | "multiple_choice"
            | "true_false_not_given"
            | "yes_no_not_given"
            | "matching_information"
            | "matching_headings"
            | "matching_features"
            | "matching_sentence_endings"
            | "classification"
    );
    if !choice_required {
        return 1.0;
    }
    let mut total = 0usize;
    let mut complete = 0usize;
    for response in response_groups {
        let options = response
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let bank_ref = response.get("optionBankRef").and_then(Value::as_str);
        let bank_options = bank_ref.and_then(|bank_id| {
            let bank = group.get("optionBank")?;
            (bank.get("optionBankId").and_then(Value::as_str) == Some(bank_id))
                .then(|| bank.get("options").and_then(Value::as_array))
                .flatten()
        });
        let bank_complete = bank_options.is_some_and(|items| {
            items.len() >= 2 && items.iter().all(option_has_renderable_content)
        });
        if options.is_empty() && !bank_complete {
            push_issue(
                issues,
                hard_failures,
                issue(
                    if task_type.starts_with("matching") || task_type == "classification" {
                        OPTION_BANK_MISSING
                    } else {
                        OPTION_RUN_INCOMPLETE
                    },
                    "blocking",
                    "题组需要选项或公共 option bank，但未找到闭合的选项集合。",
                    "response_group",
                    response
                        .get("responseGroupId")
                        .and_then(Value::as_str)
                        .unwrap_or(task_id),
                    anchors.clone(),
                    vec!["attach_option_bank", "edit_text"],
                ),
            );
            total += 1;
            continue;
        }
        total += 1;
        if !options.is_empty() {
            let nonempty = options.iter().all(option_has_renderable_content);
            let labels_match = expected_labels
                .as_ref()
                .is_none_or(|expected| option_labels(&options) == *expected);
            if nonempty && labels_match {
                complete += 1;
            } else {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        if nonempty {
                            OPTION_ALPHABET_MISMATCH
                        } else {
                            OPTION_RUN_INCOMPLETE
                        },
                        "blocking",
                        "固定选项存在 label，但缺少可渲染的 option text。",
                        "response_group",
                        response
                            .get("responseGroupId")
                            .and_then(Value::as_str)
                            .unwrap_or(task_id),
                        anchors.clone(),
                        vec!["edit_text"],
                    ),
                );
            }
        } else if bank_complete {
            let labels_match = expected_labels.as_ref().is_none_or(|expected| {
                bank_options.is_some_and(|items| option_labels(items) == *expected)
            });
            if labels_match {
                complete += 1;
            } else {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        OPTION_ALPHABET_MISMATCH,
                        "blocking",
                        "option bank labels 与 instruction signature 的 alphabet 不一致。",
                        "response_group",
                        response
                            .get("responseGroupId")
                            .and_then(Value::as_str)
                            .unwrap_or(task_id),
                        anchors.clone(),
                        vec!["attach_option_bank", "edit_text"],
                    ),
                );
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        complete as f64 / total as f64
    }
}

fn validate_cardinality(
    group: &Value,
    task_type: &str,
    task_id: &str,
    anchors: Vec<Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) -> f64 {
    let Some(signature) = group.get("instructionSignature") else {
        return 0.0;
    };
    let expected = signature
        .get("selectionCardinality")
        .and_then(|value| value.get("exact"))
        .and_then(Value::as_u64)
        .map(|number| number as usize);
    let Some(expected) = expected else {
        return 1.0;
    };
    let responses = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let response_slot_count = responses
        .iter()
        .map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        })
        .sum::<usize>();
    let group_expected = signature
        .get("expectedQuestionNumbers")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let shared_multiple_choice = task_type == "multiple_choice" && expected > 1;
    let response_policy_valid = responses.iter().all(|response| {
        let slot_count = response
            .get("slotIds")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let cardinality = response.get("cardinality");
        let exact = cardinality
            .and_then(|value| value.get("exact"))
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let min = cardinality
            .and_then(|value| value.get("min"))
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let max = cardinality
            .and_then(|value| value.get("max"))
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let assignment = response.get("assignment").and_then(Value::as_str);
        let allow_reuse = response.get("allowOptionReuse").and_then(Value::as_bool);
        if shared_multiple_choice {
            slot_count == expected
                && exact == Some(expected)
                && min == Some(expected)
                && max == Some(expected)
                && assignment == Some("unordered_set")
                && allow_reuse == Some(false)
                && response_option_count(group, response) > expected
        } else {
            match assignment {
                Some("per_slot") => {
                    slot_count > 0
                        && exact == Some(expected)
                        && min == Some(expected)
                        && max == Some(expected)
                }
                Some("unordered_set") | Some("ordered_slots") => {
                    slot_count > 0
                        && exact == Some(slot_count)
                        && min == Some(slot_count)
                        && max == Some(slot_count)
                }
                _ => false,
            }
        }
    });
    let valid = if task_type == "multiple_choice" {
        response_slot_count == group_expected
            && expected <= response_slot_count
            && response_policy_valid
    } else {
        response_policy_valid
    };
    if !response_policy_valid {
        push_issue(
            issues,
            hard_failures,
            issue(
                RESPONSE_GROUP_POLICY_MISMATCH,
                "blocking",
                &format!("题组 {task_id} 的 response group policy 与 instruction 不一致。"),
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "split_prompt"],
            ),
        );
    }
    if !valid && response_policy_valid {
        push_issue(
            issues,
            hard_failures,
            issue(
                CARDINALITY_SLOT_MISMATCH,
                "blocking",
                &format!("题组 {task_id} 的 Choose-N cardinality 与 slots 不一致。"),
                "task",
                task_id,
                anchors,
                vec!["split_prompt", "edit_text"],
            ),
        );
        0.0
    } else {
        1.0
    }
}

fn validate_completion_host(
    group: &Value,
    slots: &Map<String, Value>,
    task_id: &str,
    anchors: Vec<Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let slot_ids = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|slot_id| slot_id.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();
    let mut content_roots = group
        .get("stimulus")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    content_roots.extend(
        group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|response| response.get("prompt").and_then(Value::as_array))
            .flatten()
            .cloned(),
    );

    // Textual completion is a single visible document, not a collection of
    // detached question prompts.  Once the geometry pass has recovered the
    // numbered rows, every expected slot must be hosted exactly once by the
    // canonical stimulus tree.  Accepting a slot that only exists in a
    // response prompt makes a fragmented/empty stimulus look publishable and
    // is the source of the previous false-green completion reports.  Table,
    // flowchart and figure tasks have their own structural host checks below,
    // so this closure applies to paragraph-like completion only.
    let task_type = group
        .get("instructionSignature")
        .and_then(|signature| signature.get("taskType"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let inline_completion = matches!(
        task_type,
        "sentence_completion" | "summary_completion" | "note_completion" | "form_completion"
    );
    if inline_completion {
        let stimulus = group
            .get("stimulus")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let expected_numbers = group
            .get("instructionSignature")
            .and_then(|signature| signature.get("expectedQuestionNumbers"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|number| format!("q{number}"))
            .collect::<Vec<_>>();
        let missing_inline_slots = expected_numbers
            .iter()
            .filter(|slot_id| count_slot_nodes_in_roots(stimulus, slot_id) != 1)
            .cloned()
            .collect::<Vec<_>>();
        if !missing_inline_slots.is_empty() {
            let mut closure_issue = issue(
                SLOT_HOST_MISSING,
                "blocking",
                "文本 completion 的 canonical stimulus 必须为每个题号提供唯一的 inline answer slot。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "split_prompt"],
            );
            closure_issue["details"] = json!({
                "missingInlineSlotIds": missing_inline_slots,
                "expectedInlineSlotCount": expected_numbers.len()
            });
            push_issue(issues, hard_failures, closure_issue);
        }
    }
    let invalid = slot_ids.iter().any(|slot_id| {
        let Some(slot) = slots.get(slot_id) else {
            return true;
        };
        let host_id = slot
            .get("hostNodeId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let host_type = slot
            .get("hostType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        host_id.is_empty()
            || !content_roots
                .iter()
                .any(|node| node_contains_slot_host(node, host_id, host_type, slot_id))
    });
    if invalid {
        push_issue(
            issues,
            hard_failures,
            issue(
                SLOT_HOST_MISSING,
                "blocking",
                "completion slot 没有可渲染的宿主节点。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "confirm_table"],
            ),
        );
    }
    let duplicated = slot_ids.iter().any(|slot_id| {
        content_roots
            .iter()
            .map(|node| count_answer_slot_nodes(node, slot_id))
            .sum::<usize>()
            > 1
    });
    if duplicated {
        push_issue(
            issues,
            hard_failures,
            issue(
                SLOT_HOST_DUPLICATE,
                "blocking",
                "同一个 completion slot 被渲染到多个内容节点中。",
                "task",
                task_id,
                anchors,
                vec!["edit_text", "confirm_table"],
            ),
        );
    }
}

fn count_slot_nodes_in_roots(nodes: &[Value], slot_id: &str) -> usize {
    nodes
        .iter()
        .map(|node| count_answer_slot_nodes(node, slot_id))
        .sum()
}

fn count_answer_slot_nodes(node: &Value, slot_id: &str) -> usize {
    let own = usize::from(
        node.get("type").and_then(Value::as_str) == Some("answer_slot")
            && node.get("slotId").and_then(Value::as_str) == Some(slot_id),
    );
    own + ["children", "items", "rows", "cells", "steps"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .map(|child| count_answer_slot_nodes(child, slot_id))
        .sum::<usize>()
}

fn node_contains_slot_host(node: &Value, host_id: &str, host_type: &str, slot_id: &str) -> bool {
    if host_type == "figure_hotspot" {
        if node
            .get("hotspots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|hotspot| {
                hotspot.get("hotspotId").and_then(Value::as_str) == Some(host_id)
                    && hotspot.get("slotId").and_then(Value::as_str) == Some(slot_id)
                    && hotspot
                        .get("normalizedRect")
                        .is_some_and(valid_normalized_rect)
            })
        {
            return true;
        }
    } else if node.get("id").and_then(Value::as_str) == Some(host_id)
        && node_contains_answer_slot(node, slot_id)
    {
        return true;
    }
    ["children", "items", "rows", "cells", "steps"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .any(|child| node_contains_slot_host(child, host_id, host_type, slot_id))
}

fn node_contains_answer_slot(node: &Value, slot_id: &str) -> bool {
    if node.get("type").and_then(Value::as_str) == Some("answer_slot")
        && node.get("slotId").and_then(Value::as_str) == Some(slot_id)
    {
        return true;
    }
    ["children", "items", "rows", "cells", "steps"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .any(|child| node_contains_answer_slot(child, slot_id))
}

fn valid_normalized_rect(value: &Value) -> bool {
    let Some(items) = value.as_array().filter(|items| items.len() == 4) else {
        return false;
    };
    let Some(rect) = items.iter().map(Value::as_f64).collect::<Option<Vec<_>>>() else {
        return false;
    };
    rect.iter().all(|value| value.is_finite())
        && rect[0] >= 0.0
        && rect[1] >= 0.0
        && rect[2] > 0.0
        && rect[3] > 0.0
        && rect[0] + rect[2] <= 1.0
        && rect[1] + rect[3] <= 1.0
}

fn validate_answers(
    group: &Value,
    answer_key: &Map<String, Value>,
    task_id: &str,
    anchors: Vec<Value>,
    signature: Option<&Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let slot_ids = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|slot_id| slot_id.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();
    for slot_id in slot_ids {
        let answer = answer_key.get(&slot_id);
        let unresolved = answer
            .and_then(|value| value.get("kind"))
            .and_then(Value::as_str)
            == Some("unresolved");
        if unresolved || !answer_key.contains_key(&slot_id) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    ANSWER_KEY_MISSING_SLOT,
                    "blocking",
                    "计分 slot 没有可验证的答案 key。",
                    "slot",
                    &slot_id,
                    anchors.clone(),
                    vec!["enter_answer"],
                ),
            );
            continue;
        }
        if let Some(answer) = answer {
            validate_answer_value(
                group,
                &slot_id,
                answer,
                signature,
                anchors.clone(),
                issues,
                hard_failures,
            );
        }
    }
    let _ = task_id;
}

fn validate_answer_value(
    group: &Value,
    slot_id: &str,
    answer: &Value,
    signature: Option<&Value>,
    anchors: Vec<Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    match answer.get("kind").and_then(Value::as_str) {
        Some("option") => {
            let allowed = response_option_labels(group);
            let answer_labels = answer
                .get("labels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|label| label.to_ascii_uppercase())
                .collect::<Vec<_>>();
            if !allowed.is_empty() && answer_labels.iter().any(|label| !allowed.contains(label)) {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        ANSWER_OPTION_NOT_IN_BANK,
                        "blocking",
                        "answer key 的选项 label 不在该题组的 option run 或 option bank 中。",
                        "slot",
                        slot_id,
                        anchors,
                        vec!["attach_option_bank", "enter_answer"],
                    ),
                );
            }
        }
        Some("text") => {
            let values = answer
                .get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            let max_words = signature
                .and_then(|value| value.get("wordLimit"))
                .and_then(|value| value.get("maxWords"))
                .and_then(Value::as_u64)
                .map(|value| value as usize);
            let max_numbers = signature
                .and_then(|value| value.get("wordLimit"))
                .and_then(|value| value.get("maxNumbers"))
                .and_then(Value::as_u64)
                .map(|value| value as usize);
            let violates = values.iter().any(|value| {
                let words = value.split_whitespace().count();
                let numbers = value
                    .split_whitespace()
                    .filter(|token| token.chars().all(|ch| ch.is_ascii_digit()))
                    .count();
                max_words.is_some_and(|limit| words > limit)
                    || max_numbers.is_some_and(|limit| numbers > limit)
            });
            if violates {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        ANSWER_WORD_LIMIT_VIOLATION,
                        "blocking",
                        "answer key 超出 instruction signature 声明的 word/number limit。",
                        "slot",
                        slot_id,
                        anchors,
                        vec!["enter_answer", "edit_text"],
                    ),
                );
            }
        }
        _ => {}
    }
}

fn response_option_labels(group: &Value) -> BTreeSet<String> {
    let mut labels = BTreeSet::new();
    if let Some(responses) = group.get("responseGroups").and_then(Value::as_array) {
        for response in responses {
            if let Some(options) = response.get("options").and_then(Value::as_array) {
                labels.extend(options.iter().filter_map(|option| {
                    option
                        .get("label")
                        .and_then(Value::as_str)
                        .map(|label| label.to_ascii_uppercase())
                }));
            }
        }
    }
    if let Some(options) = group
        .pointer("/optionBank/options")
        .and_then(Value::as_array)
    {
        labels.extend(options.iter().filter_map(|option| {
            option
                .get("label")
                .and_then(Value::as_str)
                .map(|label| label.to_ascii_uppercase())
        }));
    }
    labels
}

fn expected_labels_from_alphabet(alphabet: &str) -> Option<BTreeSet<String>> {
    let alphabet = alphabet.trim().to_ascii_uppercase();
    let (start, end) = alphabet.split_once('-')?;
    let start = start.trim().chars().next()?;
    let end = end.trim().chars().next()?;
    if !start.is_ascii_uppercase() || !end.is_ascii_uppercase() || start > end {
        return None;
    }
    Some(
        (start as u8..=end as u8)
            .map(|value| (value as char).to_string())
            .collect(),
    )
}

fn option_labels(options: &[Value]) -> BTreeSet<String> {
    options
        .iter()
        .filter_map(|option| option.get("label").and_then(Value::as_str))
        .map(|label| label.trim().to_ascii_uppercase())
        .filter(|label| !label.is_empty())
        .collect()
}

fn response_option_count(group: &Value, response: &Value) -> usize {
    response
        .get("options")
        .and_then(Value::as_array)
        .map(Vec::len)
        .or_else(|| {
            let bank_ref = response.get("optionBankRef").and_then(Value::as_str)?;
            let bank = group.get("optionBank")?;
            (bank.get("optionBankId").and_then(Value::as_str) == Some(bank_ref))
                .then(|| bank.get("options").and_then(Value::as_array).map(Vec::len))
                .flatten()
        })
        .unwrap_or(0)
}

fn validate_visual_task(
    group: &Value,
    task_type: &str,
    task_id: &str,
    anchors: Vec<Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let stimulus = group.get("stimulus").and_then(Value::as_array);
    let has_diagram = stimulus
        .into_iter()
        .flatten()
        .any(|node| node_contains_type(node, "diagram"));
    let has_table = stimulus
        .into_iter()
        .flatten()
        .any(|node| node_contains_type(node, "table"));
    let has_flowchart = stimulus
        .into_iter()
        .flatten()
        .any(|node| node_contains_type(node, "flowchart"));
    let diagram_task = matches!(
        task_type,
        "diagram_label_completion" | "plan_map_label_completion"
    );
    if diagram_task && !has_diagram {
        push_issue(
            issues,
            hard_failures,
            issue(
                ASSET_REFERENCE_MISSING,
                "blocking",
                "diagram/map 题组没有可验证的 figure asset reference。",
                "task",
                task_id,
                anchors.clone(),
                vec!["replace_asset", "confirm_figure"],
            ),
        );
        if group_has_figure_slots(group) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    SLOT_OUTSIDE_FIGURE,
                    "blocking",
                    "hotspot slot 没有落在可定位的 figure 节点内。",
                    "task",
                    task_id,
                    anchors.clone(),
                    vec!["confirm_figure", "edit_text"],
                ),
            );
        }
    }
    if diagram_task && has_diagram && !diagram_hotspots_are_closed(group) {
        push_issue(
            issues,
            hard_failures,
            issue(
                HOTSPOT_GEOMETRY_INVALID,
                "blocking",
                "diagram/map hotspots 未覆盖全部 slots，或 normalizedRect 无效。",
                "task",
                task_id,
                anchors.clone(),
                vec!["confirm_figure", "edit_text"],
            ),
        );
    }
    if task_type == "table_completion" && !has_table {
        push_issue(
            issues,
            hard_failures,
            issue(
                SLOT_HOST_MISSING,
                "blocking",
                "table completion 没有可渲染的 table stimulus。",
                "task",
                task_id,
                anchors.clone(),
                vec!["confirm_table", "edit_text"],
            ),
        );
    }
    if task_type == "flowchart_completion" && !has_flowchart {
        push_issue(
            issues,
            hard_failures,
            issue(
                SLOT_HOST_MISSING,
                "blocking",
                "flowchart completion 没有可渲染的 flowchart stimulus。",
                "task",
                task_id,
                anchors,
                vec!["confirm_figure", "edit_text"],
            ),
        );
    }
}

fn diagram_hotspots_are_closed(group: &Value) -> bool {
    let slot_ids = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| response.get("slotIds").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    if slot_ids.is_empty() {
        return false;
    }
    let hotspots = group
        .get("stimulus")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(collect_hotspots)
        .collect::<Vec<_>>();
    let hotspot_slots = hotspots
        .iter()
        .filter(|hotspot| {
            hotspot
                .get("normalizedRect")
                .is_some_and(valid_normalized_rect)
                && hotspot
                    .get("hotspotId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.trim().is_empty())
        })
        .filter_map(|hotspot| hotspot.get("slotId").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    hotspot_slots == slot_ids && hotspots.len() == slot_ids.len()
}

fn collect_hotspots(node: &Value) -> Vec<&Value> {
    let mut hotspots = node
        .get("hotspots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    for key in ["children", "items", "rows", "cells", "steps"] {
        for child in node
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            hotspots.extend(collect_hotspots(child));
        }
    }
    hotspots
}

fn group_has_figure_slots(group: &Value) -> bool {
    group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| response.get("slotIds").and_then(Value::as_array))
        .flatten()
        .any(|slot_id| {
            group
                .get("taskType")
                .and_then(Value::as_str)
                .is_some_and(|task_type| {
                    matches!(
                        task_type,
                        "diagram_label_completion" | "plan_map_label_completion"
                    )
                })
                && slot_id.as_str().is_some()
        })
}

fn node_contains_type(node: &Value, expected: &str) -> bool {
    if node.get("type").and_then(Value::as_str) == Some(expected) {
        return true;
    }
    ["children", "items", "rows", "cells", "steps"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .any(|child| node_contains_type(child, expected))
}

fn validate_assets(
    authoring: &Value,
    physical_shadow: Option<&Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let assets = authoring
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let asset_ids = assets
        .iter()
        .filter_map(|asset| asset.get("assetId").and_then(Value::as_str))
        .filter(|asset_id| !asset_id.trim().is_empty())
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let mut seen_asset_ids = BTreeSet::new();
    let physical_assets = physical_shadow
        .and_then(|physical| physical.get("assets"))
        .and_then(Value::as_array)
        .map(|assets| {
            assets
                .iter()
                .filter_map(|asset| {
                    asset
                        .get("assetId")
                        .and_then(Value::as_str)
                        .map(|id| (id.to_string(), asset))
                })
                .collect::<BTreeMap<_, _>>()
        });
    for asset in &assets {
        let asset_id = asset
            .get("assetId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if asset
            .pointer("/diagramQuestionRegion/recoveryStatus")
            .and_then(Value::as_str)
            == Some("ocr_required")
            || asset
                .pointer("/diagramQuestionRegion/numberClosure")
                .and_then(Value::as_bool)
                == Some(false)
        {
            push_issue(
                issues,
                hard_failures,
                issue(
                    DIAGRAM_QUESTION_REGION_OCR_REQUIRED,
                    "blocking",
                    "图示题区域已保留，但栅格题号和标签尚未完成 OCR 与编号闭包。",
                    "asset",
                    if asset_id.is_empty() {
                        "unknown-asset"
                    } else {
                        asset_id
                    },
                    Vec::new(),
                    vec!["confirm_figure", "edit_text"],
                ),
            );
        }
        if !asset_id.is_empty() && !seen_asset_ids.insert(asset_id.to_string()) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    ASSET_ID_DUPLICATE,
                    "blocking",
                    "assetId 必须在文档内唯一。",
                    "asset",
                    asset_id,
                    Vec::new(),
                    vec!["replace_asset"],
                ),
            );
        }
        let relative_path = asset
            .get("relativePath")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if asset_id.trim().is_empty() || relative_path.trim().is_empty() {
            push_issue(
                issues,
                hard_failures,
                issue(
                    ASSET_REFERENCE_MISSING,
                    "blocking",
                    "asset descriptor 缺少 assetId 或 relativePath。",
                    "asset",
                    if asset_id.is_empty() {
                        "unknown-asset"
                    } else {
                        asset_id
                    },
                    Vec::new(),
                    vec!["replace_asset"],
                ),
            );
        }
        if !relative_path_is_safe(relative_path) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    ASSET_PATH_UNSAFE,
                    "blocking",
                    "asset descriptor 必须使用不含 traversal、drive prefix 或绝对路径的相对路径。",
                    "asset",
                    if asset_id.is_empty() {
                        "unknown-asset"
                    } else {
                        asset_id
                    },
                    Vec::new(),
                    vec!["replace_asset"],
                ),
            );
        }
        let hash = asset
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if hash.len() != 64 || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    ASSET_HASH_MISMATCH,
                    "blocking",
                    "asset descriptor 的 sha256 不是可验证的 SHA-256。",
                    "asset",
                    if asset_id.is_empty() {
                        "unknown-asset"
                    } else {
                        asset_id
                    },
                    Vec::new(),
                    vec!["replace_asset"],
                ),
            );
        }
        if let Some(physical_assets) = &physical_assets {
            match physical_assets.get(asset_id) {
                None => push_issue(
                    issues,
                    hard_failures,
                    issue(
                        ASSET_REFERENCE_MISSING,
                        "blocking",
                        "authoring asset 在 physical shadow 中不存在。",
                        "asset",
                        if asset_id.is_empty() {
                            "unknown-asset"
                        } else {
                            asset_id
                        },
                        Vec::new(),
                        vec!["replace_asset"],
                    ),
                ),
                Some(physical_asset)
                    if physical_asset.get("sha256").and_then(Value::as_str) != Some(hash) =>
                {
                    push_issue(
                        issues,
                        hard_failures,
                        issue(
                            ASSET_HASH_MISMATCH,
                            "blocking",
                            "authoring asset hash 与 physical shadow 不一致。",
                            "asset",
                            asset_id,
                            Vec::new(),
                            vec!["replace_asset"],
                        ),
                    );
                }
                _ => {}
            }
        }
    }
    let mut referenced = BTreeSet::new();
    collect_asset_references(authoring.get("passage"), &mut referenced);
    collect_asset_references(authoring.get("taskGroups"), &mut referenced);
    for asset_id in referenced.difference(&asset_ids) {
        push_issue(
            issues,
            hard_failures,
            issue(
                ASSET_REFERENCE_MISSING,
                "blocking",
                "content node 引用了不存在的 asset descriptor。",
                "asset",
                asset_id,
                Vec::new(),
                vec!["replace_asset"],
            ),
        );
    }
}

fn relative_path_is_safe(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    !normalized.is_empty()
        && !normalized.starts_with('/')
        && !normalized.contains(':')
        && normalized
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn collect_asset_references(value: Option<&Value>, out: &mut BTreeSet<String>) {
    let Some(value) = value else {
        return;
    };
    match value {
        Value::Array(items) => {
            for item in items {
                collect_asset_references(Some(item), out);
            }
        }
        Value::Object(object) => {
            for key in ["assetId", "visualFallbackAssetId"] {
                if let Some(asset_id) = object.get(key).and_then(Value::as_str) {
                    if !asset_id.trim().is_empty() {
                        out.insert(asset_id.to_string());
                    }
                }
            }
            for child in object.values() {
                collect_asset_references(Some(child), out);
            }
        }
        _ => {}
    }
}

fn group_prompt_text(group: &Value) -> String {
    let mut texts = Vec::new();
    if let Some(nodes) = group.get("stimulus").and_then(Value::as_array) {
        collect_text(nodes, &mut texts);
    }
    if let Some(responses) = group.get("responseGroups").and_then(Value::as_array) {
        for response in responses {
            if let Some(nodes) = response.get("prompt").and_then(Value::as_array) {
                collect_text(nodes, &mut texts);
            }
        }
    }
    texts.join(" ").trim().to_string()
}

fn group_has_ambiguous_prompt(group: &Value) -> bool {
    group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|response| response.get("prompt"))
        .flat_map(|prompt| prompt.as_array().into_iter().flatten())
        .any(node_contains_prompt_placeholder)
}

fn option_has_renderable_content(option: &Value) -> bool {
    option
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|content| {
            !content.is_empty()
                && content.iter().any(|node| {
                    node.get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| {
                            let normalized = text.trim();
                            !normalized.is_empty() && normalized != "[missing option text]"
                        })
                })
        })
}

fn is_prompt_placeholder(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    normalized == "[prompt pending review]"
        || normalized == "[shared prompt pending review]"
        || normalized == "prompt pending review"
}

fn node_contains_prompt_placeholder(node: &Value) -> bool {
    if node
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(is_prompt_placeholder)
    {
        return true;
    }
    ["children", "items", "rows", "cells"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .any(node_contains_prompt_placeholder)
}

fn collect_text(nodes: &[Value], out: &mut Vec<String>) {
    for node in nodes {
        if let Some(text) = node.get("text").and_then(Value::as_str) {
            if !text.trim().is_empty() && !is_prompt_placeholder(text) {
                out.push(text.trim().to_string());
            }
        }
        for key in ["children", "items", "rows", "cells"] {
            if let Some(children) = node.get(key).and_then(Value::as_array) {
                collect_text(children, out);
            }
        }
    }
}

fn is_completion_type(value: &str) -> bool {
    matches!(
        value,
        "sentence_completion"
            | "summary_completion"
            | "note_completion"
            | "table_completion"
            | "form_completion"
            | "flowchart_completion"
            | "diagram_label_completion"
            | "plan_map_label_completion"
    )
}

fn calculate_source_coverage(authoring: &Value, physical_shadow: Option<&Value>) -> f64 {
    source_coverage_summary(authoring, physical_shadow).score
}

fn validate_source_ownership(
    authoring: &Value,
    physical_shadow: Option<&Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let Some(physical) = physical_shadow.filter(|value| physical_shadow_is_usable(value)) else {
        return;
    };
    let significant_ids = significant_physical_nodes(physical)
        .into_keys()
        .collect::<BTreeSet<_>>();
    if significant_ids.is_empty() {
        return;
    }

    let mut owners_by_node = BTreeMap::<String, BTreeSet<String>>::new();
    if let Some(passage) = authoring.get("passage") {
        let mut node_ids = BTreeSet::new();
        collect_direct_anchor_node_ids(passage, &mut node_ids);
        for node_id in node_ids.intersection(&significant_ids) {
            owners_by_node
                .entry(node_id.clone())
                .or_default()
                .insert("passage".to_string());
        }
    }
    for group in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let owner = group
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or("unknown-task")
            .to_string();
        let mut node_ids = BTreeSet::new();
        collect_direct_anchor_node_ids(group, &mut node_ids);
        for node_id in node_ids.intersection(&significant_ids) {
            owners_by_node
                .entry(node_id.clone())
                .or_default()
                .insert(owner.clone());
        }
    }

    let conflicts = owners_by_node
        .into_iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(source_node_id, owners)| {
            json!({
                "sourceNodeId": source_node_id,
                "owners": owners.into_iter().collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    if conflicts.is_empty() {
        return;
    }

    let mut ownership_issue = issue(
        SOURCE_OWNERSHIP_CONFLICT,
        "blocking",
        "同一显著源节点被 passage 或多个题组重复占有，题组边界尚未闭合。",
        "document",
        "document",
        Vec::new(),
        vec!["split_prompt", "assign_role"],
    );
    ownership_issue["details"] = json!({
        "conflictCount": conflicts.len(),
        "conflicts": conflicts.into_iter().take(50).collect::<Vec<_>>()
    });
    push_issue(issues, hard_failures, ownership_issue);
}

fn collect_direct_anchor_node_ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_direct_anchor_node_ids(item, out);
            }
        }
        Value::Object(object) => {
            for node_id in object
                .get("sourceAnchors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|anchor| anchor.get("nodeIds").and_then(Value::as_array))
                .flatten()
                .filter_map(Value::as_str)
            {
                out.insert(node_id.to_string());
            }
            for node_id in object
                .get("sourceAnchor")
                .and_then(Value::as_object)
                .and_then(|anchor| anchor.get("nodeIds"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                out.insert(node_id.to_string());
            }
            for (key, child) in object {
                if !matches!(
                    key.as_str(),
                    "quality" | "coverageLedger" | "coverageStatus"
                ) {
                    collect_direct_anchor_node_ids(child, out);
                }
            }
        }
        _ => {}
    }
}

fn source_text_key(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn source_coverage_summary(
    authoring: &Value,
    physical_shadow: Option<&Value>,
) -> SourceCoverageSummary {
    let Some(physical) = physical_shadow.filter(|physical| physical_shadow_is_usable(physical))
    else {
        return SourceCoverageSummary::default();
    };
    let source_nodes = significant_physical_nodes(physical);
    if source_nodes.is_empty() {
        return SourceCoverageSummary::default();
    }
    let mut targets = BTreeMap::<String, BTreeSet<String>>::new();
    collect_anchor_targets(authoring, None, &mut targets);
    for asset_id in authoring
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| asset.get("assetId").and_then(Value::as_str))
    {
        targets
            .entry(asset_id.to_string())
            .or_default()
            .insert(asset_id.to_string());
    }
    let ignored = physical_ignored_reasons(physical);
    let mut ledger = Vec::new();
    let mut unassigned_ids = Vec::new();
    let mut assigned_count = 0usize;
    for (source_node_id, aliases) in &source_nodes {
        let mut target_id_set = aliases
            .iter()
            .chain(std::iter::once(source_node_id))
            .flat_map(|alias| targets.get(alias).into_iter().flatten().cloned())
            .collect::<BTreeSet<_>>();
        for alias in aliases {
            let Some(text_key) = alias.strip_prefix("__text:") else {
                continue;
            };
            if text_key.len() < 12 {
                continue;
            }
            for (target_key, target_values) in &targets {
                let Some(target_text) = target_key.strip_prefix("__text:") else {
                    continue;
                };
                if target_text.len() >= 12
                    && (target_text.contains(text_key) || text_key.contains(target_text))
                {
                    target_id_set.extend(target_values.iter().cloned());
                }
            }
        }
        let target_ids = target_id_set.into_iter().collect::<Vec<_>>();
        // Regions are containers over their child lines/spans/glyphs.  A
        // semantic anchor may legitimately point at a child node without
        // naming the container id itself; close that parent through its
        // already-expanded aliases before declaring it unassigned.
        let child_target_ids = aliases
            .iter()
            .flat_map(|alias| targets.get(alias).into_iter().flatten().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let target_ids = target_ids
            .into_iter()
            .chain(child_target_ids)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let reason = ignored
            .get(source_node_id)
            .cloned()
            .or_else(|| aliases.iter().find_map(|alias| ignored.get(alias).cloned()));
        let disposition = if !target_ids.is_empty() {
            assigned_count += 1;
            "assigned"
        } else if reason.is_some() {
            assigned_count += 1;
            "ignored_with_reason"
        } else {
            unassigned_ids.push(source_node_id.clone());
            "unassigned"
        };
        let mut entry = json!({
            "sourceNodeId": source_node_id,
            "significant": true,
            "disposition": disposition,
            "targetIds": target_ids
        });
        if let Some(reason) = reason {
            entry["reason"] = Value::String(reason);
        }
        ledger.push(entry);
    }
    SourceCoverageSummary {
        score: assigned_count as f64 / source_nodes.len() as f64,
        significant_count: source_nodes.len(),
        assigned_count,
        unassigned_ids,
        ledger,
        physical_available: true,
    }
}

fn physical_shadow_is_usable(physical: &Value) -> bool {
    physical.get("schemaVersion").and_then(Value::as_str) == Some("DocumentIRV2")
        && physical
            .get("documentId")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        && physical
            .get("jobId")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        && physical
            .get("sourceFiles")
            .and_then(Value::as_array)
            .is_some_and(|sources| !sources.is_empty())
        && physical
            .get("pages")
            .and_then(Value::as_array)
            .is_some_and(|pages| !pages.is_empty())
}

fn collect_anchor_targets(
    value: &Value,
    inherited_target: Option<&str>,
    out: &mut BTreeMap<String, BTreeSet<String>>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_anchor_targets(item, inherited_target, out);
            }
        }
        Value::Object(object) => {
            let target = [
                "slotId",
                "responseGroupId",
                "optionId",
                "optionBankId",
                "taskId",
                "id",
                "assetId",
                "examId",
            ]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str))
            .or(inherited_target)
            .unwrap_or("document");
            for node_id in object
                .get("sourceAnchors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|anchor| anchor.get("nodeIds").and_then(Value::as_array))
                .flatten()
                .filter_map(Value::as_str)
            {
                out.entry(node_id.to_string())
                    .or_default()
                    .insert(target.to_string());
            }
            for key in ["text", "html", "textPreview"] {
                if let Some(text_key) = object
                    .get(key)
                    .and_then(Value::as_str)
                    .map(source_text_key)
                    .filter(|key| key.len() >= 12)
                {
                    out.entry(format!("__text:{text_key}"))
                        .or_default()
                        .insert(target.to_string());
                }
            }
            for node_id in object
                .get("sourceAnchor")
                .and_then(Value::as_object)
                .and_then(|anchor| anchor.get("nodeIds"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                out.entry(node_id.to_string())
                    .or_default()
                    .insert(target.to_string());
            }
            for (key, child) in object {
                if !matches!(
                    key.as_str(),
                    "quality" | "coverageLedger" | "coverageStatus"
                ) {
                    collect_anchor_targets(child, Some(target), out);
                }
            }
        }
        _ => {}
    }
}

fn physical_ignored_reasons(physical: &Value) -> BTreeMap<String, String> {
    let mut ignored = physical
        .get("coverageLedger")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry.get("disposition").and_then(Value::as_str) == Some("ignored_with_reason")
        })
        .filter_map(|entry| {
            let id = entry.get("sourceNodeId").and_then(Value::as_str)?;
            let reason = entry
                .get("reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.trim().is_empty())?;
            Some((id.to_string(), reason.to_string()))
        })
        .collect::<BTreeMap<_, _>>();

    for page in physical
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let line_texts = page
            .get("lines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|line| {
                let id = line.get("id").and_then(Value::as_str)?;
                let text = line.get("text").and_then(Value::as_str).unwrap_or_default();
                Some((id.to_string(), text.to_string()))
            })
            .collect::<BTreeMap<_, _>>();
        let ocr_page = page
            .pointer("/quality/classification")
            .and_then(Value::as_str)
            == Some("scanned")
            && page
                .pointer("/quality/requiresOcrRegions")
                .and_then(Value::as_array)
                .is_some_and(|regions| !regions.is_empty());

        for region in page
            .get("regions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(region_id) = region.get("id").and_then(Value::as_str) else {
                continue;
            };
            if ignored.contains_key(region_id) {
                continue;
            }
            let kind = region
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let child_line_ids = region
                .get("childLineIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            let region_text = child_line_ids
                .iter()
                .filter_map(|line_id| line_texts.get(*line_id))
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            let has_semantic_text = region_text.chars().any(char::is_alphanumeric);
            let child_object_count = region
                .get("childObjectIds")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let narrow_region = region
                .get("bbox")
                .and_then(Value::as_object)
                .and_then(|bbox| bbox.get("width"))
                .and_then(Value::as_f64)
                .is_some_and(|width| width <= 8.0);

            // The PDF extractor can materialize a one-column sliver as a table
            // with an empty line and a synthetic table object. It carries no
            // authorable text or asset, so keep it explained without treating
            // it as a real table task.
            if kind == "table" && !has_semantic_text && child_object_count > 0 && narrow_region {
                ignored.insert(
                    region_id.to_string(),
                    "narrow_empty_table_layout_artifact".to_string(),
                );
                continue;
            }

            let compact_text = source_text_key(&region_text);
            if kind == "table"
                && (compact_text.starts_with("youshouldspendabout") || compact_text == "2below")
            {
                ignored.insert(
                    region_id.to_string(),
                    "exam_instruction_layout_fragment".to_string(),
                );
                continue;
            }

            if ocr_page
                && (compact_text.contains("答案")
                    || compact_text.contains("answer")
                    || compact_text.contains("explanation")
                    || compact_text.contains("分析")
                    || compact_text.contains("下页")
                    || compact_text.contains("nextpage"))
            {
                ignored.insert(
                    region_id.to_string(),
                    "answer_explanation_overlay_on_ocr_page".to_string(),
                );
            }
        }
    }
    ignored
}

fn significant_physical_nodes(physical: &Value) -> BTreeMap<String, BTreeSet<String>> {
    fn insert_node(
        output: &mut BTreeMap<String, BTreeSet<String>>,
        id: Option<&str>,
        aliases: impl IntoIterator<Item = String>,
    ) {
        let Some(id) = id.filter(|id| !id.trim().is_empty()) else {
            return;
        };
        output.entry(id.to_string()).or_default().extend(aliases);
    }

    fn string_values(value: Option<&Value>) -> Vec<String> {
        value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect()
    }

    let mut output = BTreeMap::new();
    for page in physical
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let spans = page
            .get("spans")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let span_glyph_ids = spans
            .iter()
            .filter_map(|span| {
                span.get("id")
                    .and_then(Value::as_str)
                    .map(|id| (id.to_string(), string_values(span.get("glyphIds"))))
            })
            .collect::<BTreeMap<_, _>>();
        let lines = page
            .get("lines")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut referenced_span_ids = BTreeSet::new();
        let mut line_aliases = BTreeMap::<String, Vec<String>>::new();
        for line in lines {
            let span_ids = string_values(line.get("spanIds"));
            referenced_span_ids.extend(span_ids.iter().cloned());
            let aliases =
                span_ids
                    .iter()
                    .cloned()
                    .chain(span_ids.iter().flat_map(|span_id| {
                        span_glyph_ids.get(span_id).into_iter().flatten().cloned()
                    }))
                    .collect::<Vec<_>>();
            if let Some(line_id) = line.get("id").and_then(Value::as_str) {
                line_aliases.insert(line_id.to_string(), aliases);
            }
        }
        let referenced_glyph_ids = span_glyph_ids
            .values()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut region_line_ids = BTreeSet::new();
        let mut region_aliases = BTreeMap::<String, Vec<String>>::new();
        let object_aliases = ["annotations", "imagePlacements"]
            .into_iter()
            .flat_map(|field| {
                page.get(field)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(|item| {
                let id = item.get("id").and_then(Value::as_str)?;
                let aliases = item
                    .get("assetId")
                    .and_then(Value::as_str)
                    .map(|asset_id| vec![asset_id.to_string()])
                    .unwrap_or_default();
                Some((id.to_string(), aliases))
            })
            .collect::<BTreeMap<_, _>>();
        for region in page
            .get("regions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let child_lines = string_values(region.get("childLineIds"));
            region_line_ids.extend(child_lines.iter().cloned());
            let child_objects = string_values(region.get("childObjectIds"));
            let mut aliases =
                child_lines
                    .iter()
                    .cloned()
                    .chain(child_lines.iter().flat_map(|line_id| {
                        line_aliases.get(line_id).into_iter().flatten().cloned()
                    }))
                    .chain(child_objects.iter().cloned())
                    .chain(child_objects.iter().flat_map(|object_id| {
                        object_aliases.get(object_id).into_iter().flatten().cloned()
                    }))
                    .collect::<Vec<_>>();
            let region_text = child_lines
                .iter()
                .filter_map(|line_id| {
                    lines
                        .iter()
                        .find(|line| line.get("id").and_then(Value::as_str) == Some(line_id))
                        .and_then(|line| line.get("text").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join(" ");
            let region_text_key = source_text_key(&region_text);
            if region_text_key.len() >= 12 {
                aliases.push(format!("__text:{region_text_key}"));
            }
            if let Some(region_id) = region.get("id").and_then(Value::as_str) {
                region_aliases.insert(region_id.to_string(), aliases.clone());
            }
            let kind = region
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if matches!(kind, "header" | "footer" | "page_number" | "rule") {
                continue;
            }
            let resolved_child_lines = child_lines
                .iter()
                .filter_map(|line_id| {
                    lines
                        .iter()
                        .find(|line| line.get("id").and_then(Value::as_str) == Some(line_id))
                })
                .collect::<Vec<_>>();
            let has_semantic_text = resolved_child_lines.iter().any(|line| {
                line.get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.chars().any(char::is_alphanumeric))
            });
            let unresolved_explicit_lines =
                !child_lines.is_empty() && resolved_child_lines.len() < child_lines.len();
            let structural_kind = !matches!(kind, "text" | "title");
            if !has_semantic_text
                && child_objects.is_empty()
                && !unresolved_explicit_lines
                && !structural_kind
            {
                continue;
            }
            insert_node(
                &mut output,
                region.get("id").and_then(Value::as_str),
                aliases,
            );
        }
        for line in lines {
            let id = line.get("id").and_then(Value::as_str);
            if id.is_some_and(|id| region_line_ids.contains(id)) {
                continue;
            }
            let aliases = id
                .and_then(|id| line_aliases.get(id))
                .cloned()
                .unwrap_or_default();
            insert_node(&mut output, id, aliases);
        }
        for span in spans {
            let id = span.get("id").and_then(Value::as_str);
            if id.is_some_and(|id| referenced_span_ids.contains(id)) {
                continue;
            }
            let aliases = id
                .and_then(|id| span_glyph_ids.get(id))
                .cloned()
                .unwrap_or_default();
            insert_node(&mut output, id, aliases);
        }
        for glyph in page
            .get("glyphs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = glyph.get("id").and_then(Value::as_str);
            let meaningful = glyph
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty());
            if !meaningful || id.is_some_and(|id| referenced_glyph_ids.contains(id)) {
                continue;
            }
            insert_node(&mut output, id, Vec::new());
        }
        let mut table_border_path_ids = BTreeSet::new();
        for table in page
            .get("tables")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut aliases = Vec::new();
            for cell in table
                .get("cells")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(cell_id) = cell.get("cellId").and_then(Value::as_str) {
                    aliases.push(cell_id.to_string());
                }
                let content_region_ids = string_values(cell.get("contentRegionIds"));
                aliases.extend(content_region_ids.iter().cloned());
                aliases.extend(content_region_ids.iter().flat_map(|region_id| {
                    region_aliases.get(region_id).into_iter().flatten().cloned()
                }));
                let border_ids = string_values(cell.get("borderEvidence"));
                table_border_path_ids.extend(border_ids.iter().cloned());
                aliases.extend(border_ids);
            }
            insert_node(
                &mut output,
                table.get("id").and_then(Value::as_str),
                aliases,
            );
        }
        for path in page
            .get("vectorPaths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let path_id = path.get("id").and_then(Value::as_str);
            if path.get("isAxisAlignedRule").and_then(Value::as_bool) == Some(true)
                || path_id.is_some_and(|id| table_border_path_ids.contains(id))
            {
                continue;
            }
            insert_node(&mut output, path_id, Vec::new());
        }
        for field in ["annotations", "imagePlacements"] {
            for item in page
                .get(field)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let aliases = item
                    .get("assetId")
                    .and_then(Value::as_str)
                    .map(|id| vec![id.to_string()])
                    .unwrap_or_default();
                insert_node(&mut output, item.get("id").and_then(Value::as_str), aliases);
            }
        }
        for item in page
            .get("markedContent")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let meaningful = ["actualText", "altText"].iter().any(|key| {
                item.get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.trim().is_empty())
            });
            if meaningful {
                insert_node(
                    &mut output,
                    item.get("id").and_then(Value::as_str),
                    Vec::new(),
                );
            }
        }
    }
    for asset in physical
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        insert_node(
            &mut output,
            asset.get("assetId").and_then(Value::as_str),
            Vec::new(),
        );
    }
    output
}

fn issue(
    code: &str,
    severity: &str,
    message: &str,
    target_type: &str,
    target_id: &str,
    source_anchors: Vec<Value>,
    suggested_actions: Vec<&str>,
) -> Value {
    json!({
        "issueId": format!("phase4-{code}-{target_id}"),
        "code": code,
        "severity": severity,
        "message": message,
        "targetType": target_type,
        "targetId": target_id,
        "sourceAnchors": source_anchors,
        "suggestedActions": suggested_actions
    })
}

fn push_issue(issues: &mut Vec<Value>, hard_failures: &mut Vec<String>, value: Value) {
    let code = value
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if value.get("severity").and_then(Value::as_str) == Some("blocking") {
        if !hard_failures.contains(&code) {
            hard_failures.push(code);
        }
    }
    if !issues.iter().any(|item| item == &value) {
        issues.push(value);
    }
}

fn round(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn early_approaches() -> Value {
        serde_json::from_str(include_str!(
            "../../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .expect("checked-in PR-06 fixture must be valid JSON")
    }

    fn collect_fixture_node_ids(value: &Value, output: &mut BTreeSet<String>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    collect_fixture_node_ids(item, output);
                }
            }
            Value::Object(object) => {
                for node_id in object
                    .get("sourceAnchors")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .flat_map(|anchor| anchor.get("nodeIds").and_then(Value::as_array))
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    output.insert(node_id.to_string());
                }
                for (key, child) in object {
                    if key != "quality" {
                        collect_fixture_node_ids(child, output);
                    }
                }
            }
            _ => {}
        }
    }

    fn valid_physical_shadow(authoring: &Value) -> Value {
        let mut node_ids = BTreeSet::new();
        collect_fixture_node_ids(authoring, &mut node_ids);
        let source_hash = "a".repeat(64);
        let physical = json!({
            "schemaVersion":"DocumentIRV2",
            "documentId":authoring.get("sourceDocumentId").and_then(Value::as_str).unwrap_or("document-1"),
            "jobId":authoring.get("jobId").and_then(Value::as_str).unwrap_or("job-1"),
            "sourceFiles":[{
                "sourceFileId":"source-pdf-1",
                "originalName":"early-approaches.pdf",
                "mediaType":"application/pdf",
                "sha256":source_hash,
                "byteLength":1,
                "role":"question_paper"
            }],
            "pages":[{
                "pageIndex":0,
                "widthPt":612.0,
                "heightPt":792.0,
                "rotation":0,
                "glyphs":[],
                "spans":[],
                "lines":[],
                "regions":[{
                    "id":"region-question-surface",
                    "kind":"text",
                    "bbox":{"x":10.0,"y":10.0,"width":500.0,"height":200.0,"unit":"pt","origin":"top-left","pageRotation":0},
                    "childLineIds":node_ids,
                    "childObjectIds":[],
                    "confidence":1.0,
                    "sourceAnchors":[{"sourceFileId":"source-pdf-1","pageIndex":0,"nodeIds":["region-question-surface"],"extractionMode":"pdf_native","sourceHash":source_hash}]
                }],
                "vectorPaths":[],
                "tables":[],
                "assetIds":[],
                "readingOrder":["region-question-surface"],
                "quality":{
                    "classification":"born_digital",
                    "nativeCharacterCount":100,
                    "unicodeErrorRatio":0.0,
                    "duplicateTextRatio":0.0,
                    "imageCoverageRatio":0.0,
                    "textCoverageRatio":1.0,
                    "rotationConfidence":1.0,
                    "requiresOcrRegions":[],
                    "warnings":[]
                }
            }],
            "assets":[],
            "coverageLedger":[{"sourceNodeId":"region-question-surface","disposition":"unassigned","targetIds":[],"reason":"semantic assignment is evaluated by QualityReportV2"}],
            "parser":{"provider":"phase4-test","providerVersion":"1","extractionStartedAt":"2026-08-10T00:00:00Z","extractionCompletedAt":"2026-08-10T00:00:01Z","options":{},"warnings":[]}
        });
        serde_json::from_value::<crate::schema::DocumentIRV2>(physical.clone())
            .expect("quality proof physical shadow must be a typed DocumentIRV2");
        physical
    }

    fn issue_for_target(report: &Value, code: &str, target_id: &str) -> bool {
        report
            .get("issues")
            .and_then(Value::as_array)
            .is_some_and(|issues| {
                issues.iter().any(|issue| {
                    issue.get("code").and_then(Value::as_str) == Some(code)
                        && issue.get("targetId").and_then(Value::as_str) == Some(target_id)
                })
            })
    }

    #[test]
    fn quality_gate_blocks_range_only_group_without_slots_and_answers() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {"taskType":"single_choice","expectedQuestionNumbers":[1,2],"confidence":0.95},
                "instructions": [{"type":"text","text":"Choose the correct letter."}],
                "responseGroups": []
            }],
            "answerSlots": {},
            "answerKey": {}
        });
        let report = evaluate_quality(&authoring, None);
        assert_eq!(report.get("state").and_then(Value::as_str), Some("blocked"));
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty()));
    }

    #[test]
    fn quality_gate_reports_review_when_only_source_coverage_is_weak() {
        let authoring = json!({
            "taskGroups": [],
            "answerSlots": {},
            "answerKey": {}
        });
        let report = evaluate_quality(&authoring, None);
        assert_eq!(report.get("state").and_then(Value::as_str), Some("blocked"));
    }

    #[test]
    fn quality_gate_does_not_count_instruction_text_as_question_prompt() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {"taskType":"single_choice","expectedQuestionNumbers":[1],"confidence":0.95},
                "instructions": [{"type":"text","text":"Choose the correct letter."}],
                "responseGroups": [{
                    "responseGroupId":"task-1-response-1",
                    "slotIds":["q1"],
                    "options": [{"content":[{"text":"A choice"}]}]
                }]
            }],
            "answerSlots": {"q1":{"questionNumber":1}},
            "answerKey": {"q1":{"kind":"option","labels":["A"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| { items.iter().any(|item| item.as_str() == Some(PROMPT_EMPTY)) }));
    }

    #[test]
    fn quality_gate_rejects_answer_label_outside_renderable_options() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {"taskType":"single_choice","expectedQuestionNumbers":[1],"confidence":0.95},
                "stimulus": [{"type":"paragraph","text":"Select the correct answer."}],
                "responseGroups": [{
                    "responseGroupId":"task-1-response-1",
                    "slotIds":["q1"],
                    "options": [
                        {"label":"A","content":[{"text":"First choice"}]},
                        {"label":"B","content":[{"text":"Second choice"}]}
                    ]
                }]
            }],
            "answerSlots": {"q1":{"questionNumber":1}},
            "answerKey": {"q1":{"kind":"option","labels":["C"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some(ANSWER_OPTION_NOT_IN_BANK))
            }));
    }

    #[test]
    fn quality_gate_rejects_completion_answer_over_word_limit() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {
                    "taskType":"sentence_completion",
                    "expectedQuestionNumbers":[1],
                    "wordLimit":{"maxWords":1},
                    "confidence":0.95
                },
                "stimulus": [{"type":"paragraph","text":"Complete the sentence."}],
                "responseGroups": [{"responseGroupId":"task-1-response-1","slotIds":["q1"]}]
            }],
            "answerSlots": {"q1":{"questionNumber":1,"hostNodeId":"paragraph-1"}},
            "answerKey": {"q1":{"kind":"text","values":["two words"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some(ANSWER_WORD_LIMIT_VIOLATION))
            }));
    }

    #[test]
    fn quality_gate_rejects_diagram_task_without_figure_stimulus() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {
                    "taskType":"diagram_label_completion",
                    "expectedQuestionNumbers":[1],
                    "wordLimit":{"maxWords":1},
                    "confidence":0.95
                },
                "stimulus": [{"type":"paragraph","text":"Label the diagram."}],
                "responseGroups": [{"responseGroupId":"task-1-response-1","slotIds":["q1"]}]
            }],
            "answerSlots": {"q1":{"questionNumber":1,"hostNodeId":"figure-1"}},
            "answerKey": {"q1":{"kind":"text","values":["label"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some(ASSET_REFERENCE_MISSING))
            }));
    }

    #[test]
    fn quality_gate_rejects_diagram_task_without_closed_hotspots() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {"taskType":"diagram_label_completion","expectedQuestionNumbers":[1],"wordLimit":{"maxWords":1},"confidence":0.95},
                "stimulus": [{"type":"diagram","id":"figure-1","hotspots":[]}],
                "responseGroups": [{"responseGroupId":"task-1-response-1","slotIds":["q1"]}]
            }],
            "answerSlots": {"q1":{"questionNumber":1,"hostNodeId":"hotspot-1","hostType":"figure_hotspot"}},
            "answerKey": {"q1":{"kind":"text","values":["label"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| items
                .iter()
                .any(|item| item.as_str() == Some(HOTSPOT_GEOMETRY_INVALID))));
    }

    #[test]
    fn quality_gate_rejects_shared_response_policy_mismatch() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {"taskType":"multiple_choice","expectedQuestionNumbers":[1,2],"optionAlphabet":"A-D","selectionCardinality":{"min":2,"max":2,"exact":2},"confidence":0.95},
                "stimulus": [{"type":"paragraph","text":"Choose two factors."}],
                "responseGroups": [{"responseGroupId":"task-1-response-1","slotIds":["q1","q2"],"options":[{"label":"A","content":[{"text":"A"}]},{"label":"B","content":[{"text":"B"}]},{"label":"C","content":[{"text":"C"}]},{"label":"D","content":[{"text":"D"}]}],"cardinality":{"min":2,"max":2,"exact":2},"assignment":"per_slot","allowOptionReuse":false}]
            }],
            "answerSlots": {"q1":{"questionNumber":1},"q2":{"questionNumber":2}},
            "answerKey": {"q1":{"kind":"option","labels":["A"]},"q2":{"kind":"option","labels":["B"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| items
                .iter()
                .any(|item| item.as_str() == Some(RESPONSE_GROUP_POLICY_MISMATCH))));
    }

    #[test]
    fn per_slot_cardinality_is_valid_for_multiple_slots_when_each_slot_takes_one_answer() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "taskType":"summary_completion",
                "instructionSignature": {
                    "taskType":"summary_completion",
                    "expectedQuestionNumbers":[1,2],
                    "selectionCardinality":{"min":1,"max":1,"exact":1},
                    "wordLimit":{"maxWords":1,"maxNumbers":1,"wordsAndOrNumber":true},
                    "confidence":0.95
                },
                "stimulus": [{"type":"paragraph","text":"Complete both gaps."}],
                "responseGroups": [{
                    "responseGroupId":"task-1-response-1",
                    "slotIds":["q1","q2"],
                    "cardinality":{"min":1,"max":1,"exact":1},
                    "assignment":"per_slot",
                    "scoringPolicy":"per_slot_ielts_normalized",
                    "allowOptionReuse":false
                }]
            }],
            "answerSlots": {
                "q1":{"questionNumber":1,"hostNodeId":"p1","participation":"scoring"},
                "q2":{"questionNumber":2,"hostNodeId":"p2","participation":"scoring"}
            },
            "answerKey": {
                "q1":{"kind":"text","values":["one"]},
                "q2":{"kind":"text","values":["two"]}
            }
        });
        let report = evaluate_quality(&authoring, None);
        assert!(!report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == RESPONSE_GROUP_POLICY_MISMATCH));
    }

    #[test]
    fn unresolved_scoring_and_scored_example_use_stable_blocking_codes() {
        let baseline = early_approaches();
        let physical = valid_physical_shadow(&baseline);
        let mut missing_policy = baseline.clone();
        missing_policy["taskGroups"][0]["responseGroups"][0]
            .as_object_mut()
            .unwrap()
            .remove("scoringPolicy");
        let report = evaluate_quality(&missing_policy, Some(&physical));
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == SCORING_POLICY_UNRESOLVED));

        let mut example = baseline;
        example["answerSlots"]["q14"]["participation"] = json!("example");
        let report = evaluate_quality(&example, Some(&physical));
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == EXAMPLE_SCORING_CONFLICT));
    }

    #[test]
    fn source_coverage_uses_unique_physical_ledger_ids() {
        let authoring = json!({"sourceAnchors":[{"nodeIds":["line-1"]}]});
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[{"regions":[],"lines":[{"id":"line-1"},{"id":"line-2"}]}],
            "assets":[],
            "coverageLedger":[
                {"sourceNodeId":"line-1","disposition":"unassigned","targetIds":[]},
                {"sourceNodeId":"line-2","disposition":"unassigned","targetIds":[]}
            ]
        });
        assert_eq!(calculate_source_coverage(&authoring, Some(&physical)), 0.5);
        let report = evaluate_quality(&authoring, Some(&physical));
        let details = report
            .get("issues")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|issue| {
                issue.get("code").and_then(Value::as_str) == Some(SIGNIFICANT_REGION_UNASSIGNED)
            })
            .and_then(|issue| issue.get("details"))
            .unwrap();
        assert_eq!(
            details.get("unassignedSourceNodeIds"),
            Some(&json!(["line-2"]))
        );
    }

    #[test]
    fn source_ownership_blocks_the_same_physical_line_in_two_tasks() {
        let authoring = json!({
            "taskGroups": [
                {"taskId":"task-1","sourceAnchors":[{"nodeIds":["line-shared"]}]},
                {"taskId":"task-2","sourceAnchors":[{"nodeIds":["line-shared"]}]}
            ]
        });
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[{"lines":[{"id":"line-shared","text":"Question source"}],"regions":[]}]
        });
        let mut issues = Vec::new();
        let mut hard_failures = Vec::new();
        validate_source_ownership(&authoring, Some(&physical), &mut issues, &mut hard_failures);
        assert!(hard_failures
            .iter()
            .any(|code| code == SOURCE_OWNERSHIP_CONFLICT));
    }

    #[test]
    fn completion_duplicate_slot_nodes_are_blocking() {
        let group = json!({
            "taskId":"task-1",
            "responseGroups":[{
                "slotIds":["q1"],
                "prompt":[
                    {"type":"paragraph","id":"host-1","children":[{"type":"answer_slot","slotId":"q1"}]},
                    {"type":"paragraph","id":"host-2","children":[{"type":"answer_slot","slotId":"q1"}]}
                ]
            }]
        });
        let slots = serde_json::from_value::<Map<String, Value>>(json!({
            "q1":{"slotId":"q1","hostNodeId":"host-1","hostType":"paragraph"}
        }))
        .unwrap();
        let mut issues = Vec::new();
        let mut hard_failures = Vec::new();
        validate_completion_host(
            &group,
            &slots,
            "task-1",
            Vec::new(),
            &mut issues,
            &mut hard_failures,
        );
        assert!(hard_failures.iter().any(|code| code == SLOT_HOST_DUPLICATE));
    }

    #[test]
    fn completion_requires_inline_slot_closure_in_canonical_stimulus() {
        let group = json!({
            "taskId":"task-closure",
            "instructionSignature": {
                "taskType":"summary_completion",
                "expectedQuestionNumbers":[1,2],
                "wordLimit":{"maxWords":1},
                "confidence":0.95
            },
            "stimulus":[{"type":"paragraph","id":"stimulus-1","children":[{"type":"text","text":"A summary without its gaps."}]}],
            "responseGroups":[{"responseGroupId":"response-closure","slotIds":["q1","q2"],"prompt":[]}]
        });
        let slots = serde_json::from_value::<Map<String, Value>>(json!({
            "q1":{"slotId":"q1","hostNodeId":"stimulus-1","hostType":"paragraph"},
            "q2":{"slotId":"q2","hostNodeId":"stimulus-1","hostType":"paragraph"}
        }))
        .unwrap();
        let mut issues = Vec::new();
        let mut hard_failures = Vec::new();
        validate_completion_host(
            &group,
            &slots,
            "task-closure",
            Vec::new(),
            &mut issues,
            &mut hard_failures,
        );
        assert!(hard_failures.iter().any(|code| code == SLOT_HOST_MISSING));
        assert!(issues.iter().any(|value| {
            value
                .get("details")
                .and_then(|details| details.get("missingInlineSlotIds"))
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.len() == 2)
        }));
    }

    #[test]
    fn source_coverage_blocks_orphan_spans_and_glyphs() {
        let authoring = json!({"sourceAnchors":[{"nodeIds":["line-1"]}]});
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[{
                "regions":[],
                "lines":[{"id":"line-1","spanIds":["span-1"]}],
                "spans":[
                    {"id":"span-1","glyphIds":["glyph-1"]},
                    {"id":"span-orphan","glyphIds":["glyph-2"]}
                ],
                "glyphs":[
                    {"id":"glyph-1","text":"A"},
                    {"id":"glyph-2","text":"B"},
                    {"id":"glyph-orphan","text":"C"},
                    {"id":"glyph-whitespace","text":" "}
                ]
            }],
            "assets":[]
        });

        let summary = source_coverage_summary(&authoring, Some(&physical));
        assert_eq!(summary.significant_count, 3);
        assert_eq!(summary.assigned_count, 1);
        assert_eq!(
            summary.unassigned_ids,
            vec!["glyph-orphan".to_string(), "span-orphan".to_string()]
        );
        assert!(!summary
            .unassigned_ids
            .contains(&"glyph-whitespace".to_string()));
        assert!((summary.score - (1.0 / 3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn table_regions_expand_to_glyphs_and_border_paths_are_not_separate_orphans() {
        let authoring = json!({"sourceAnchors":[{"nodeIds":["glyph-1"]}]});
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[{
                "glyphs":[{"id":"glyph-1"}],
                "spans":[{"id":"span-1","glyphIds":["glyph-1"]}],
                "lines":[{"id":"line-1","spanIds":["span-1"]}],
                "regions":[{"id":"region-1","kind":"table","childLineIds":["line-1"],"childObjectIds":[]}],
                "tables":[{"id":"table-1","cells":[{
                    "cellId":"cell-1",
                    "contentRegionIds":["region-1"],
                    "borderEvidence":["path-border"]
                }]}],
                "vectorPaths":[{"id":"path-border","isAxisAlignedRule":false}]
            }],
            "assets":[]
        });

        let summary = source_coverage_summary(&authoring, Some(&physical));
        assert_eq!(summary.significant_count, 2);
        assert_eq!(summary.assigned_count, 2);
        assert!(summary.unassigned_ids.is_empty());
    }

    #[test]
    fn source_coverage_explains_narrow_empty_tables_and_ocr_answer_notes() {
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[
                {
                    "quality":{"classification":"born_digital","requiresOcrRegions":[]},
                    "lines":[{"id":"blank-line","text":""}],
                    "regions":[{"id":"empty-table","kind":"table","bbox":{"width":3.0,"height":12.0},"childLineIds":["blank-line"],"childObjectIds":["table-1"]}]
                },
                {
                    "quality":{"classification":"scanned","requiresOcrRegions":[{}]},
                    "lines":[{"id":"note-line","text":"Q3答案可能有争议，争议分析见下页"}],
                    "regions":[{"id":"answer-note","kind":"text","childLineIds":["note-line"],"childObjectIds":[]}]
                }
            ],
            "assets":[]
        });
        let summary = source_coverage_summary(&json!({}), Some(&physical));
        assert_eq!(summary.significant_count, 2);
        assert_eq!(summary.assigned_count, 2);
        assert!(summary.unassigned_ids.is_empty());
        assert!(summary
            .ledger
            .iter()
            .all(|entry| entry.get("disposition").and_then(Value::as_str)
                == Some("ignored_with_reason")));
    }

    #[test]
    fn matching_authoring_asset_descriptor_closes_the_physical_asset() {
        let authoring = json!({"assets":[{"assetId":"asset-1"}]});
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[{}],
            "assets":[{"assetId":"asset-1"}]
        });
        let summary = source_coverage_summary(&authoring, Some(&physical));
        assert_eq!(summary.significant_count, 1);
        assert_eq!(summary.assigned_count, 1);
        assert!(summary.unassigned_ids.is_empty());
    }

    #[test]
    fn unsafe_assets_and_missing_references_are_blocking() {
        let authoring = json!({
            "taskGroups": [],
            "answerSlots": {},
            "answerKey": {},
            "assets": [{"assetId":"figure-1","relativePath":"../figure.png","sha256":"bad"}],
            "passage": [{"type":"image","assetId":"missing"}]
        });
        let report = evaluate_quality(&authoring, None);
        let hard = report
            .get("hardFailures")
            .and_then(Value::as_array)
            .unwrap();
        assert!(hard
            .iter()
            .any(|item| item.as_str() == Some(ASSET_PATH_UNSAFE)));
        assert!(hard
            .iter()
            .any(|item| item.as_str() == Some(ASSET_REFERENCE_MISSING)));
    }

    #[test]
    fn ready_fixture_proves_coverage_compilers_and_shared_v1_semantics() {
        let authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        let report = evaluate_quality(&authoring, Some(&physical));
        assert_eq!(report["state"], "ready", "{report:#}");
        assert_eq!(report["coverageStatus"]["complete"], true);
        assert_eq!(report["compilerProbes"]["v2Runtime"]["status"], "passed");
        assert_eq!(
            report["compilerProbes"]["v1Compatibility"]["status"],
            "passed"
        );
        serde_json::from_value::<crate::schema::QualityReportV2>(report.clone())
            .expect("ready report must deserialize as QualityReportV2");

        let v1 = compile_v1_compatibility_shadow(&authoring);
        let html = v1["questionGroups"][0]["bodyHtml"].as_str().unwrap();
        assert_eq!(html.matches("type=\"checkbox\"").count(), 5);
        assert_eq!(html.matches("shared-response").count(), 1);
        assert!(!html.contains("<select"));
    }

    #[test]
    fn missing_malformed_and_empty_physical_shadows_cannot_be_ready() {
        let authoring = early_approaches();
        for physical in [
            None,
            Some(json!({"schemaVersion":"DocumentIRV2"})),
            Some({
                let mut empty = valid_physical_shadow(&authoring);
                empty["pages"][0]["regions"] = json!([]);
                empty["coverageLedger"] = json!([]);
                empty
            }),
        ] {
            let report = evaluate_quality(&authoring, physical.as_ref());
            assert_eq!(report["state"], "review_required", "{report:#}");
            assert_eq!(report["coverageStatus"]["physicalShadow"], "missing");
        }
    }

    #[test]
    fn unassigned_significant_region_is_blocking_but_child_anchor_assigns_parent_region() {
        let authoring = early_approaches();
        let assigned = valid_physical_shadow(&authoring);
        assert_eq!(
            evaluate_quality(&authoring, Some(&assigned))["state"],
            "ready"
        );

        let mut unassigned = assigned;
        unassigned["pages"][0]["regions"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id":"region-unassigned","kind":"figure",
                "bbox":{"x":10.0,"y":300.0,"width":100.0,"height":100.0,"unit":"pt","origin":"top-left","pageRotation":0},
                "childLineIds":[],"childObjectIds":[],"confidence":1.0,
                "sourceAnchors":[{"sourceFileId":"source-pdf-1","pageIndex":0,"nodeIds":["region-unassigned"],"extractionMode":"pdf_native","sourceHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]
            }));
        let report = evaluate_quality(&authoring, Some(&unassigned));
        assert_eq!(report["state"], "blocked");
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == SIGNIFICANT_REGION_UNASSIGNED));
    }

    #[test]
    fn quality_hard_invariant_mutations_have_stable_issue_codes() {
        let baseline = early_approaches();
        let physical = valid_physical_shadow(&baseline);

        let mut invalid_exam = baseline.clone();
        invalid_exam["exam"]["examId"] = json!("../unsafe");
        assert!(
            evaluate_quality(&invalid_exam, Some(&physical))["hardFailures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|code| code == EXAM_ID_INVALID)
        );

        let mut no_passage = baseline.clone();
        no_passage["passage"]["content"] = json!([]);
        assert!(
            evaluate_quality(&no_passage, Some(&physical))["hardFailures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|code| code == PASSAGE_CONTENT_MISSING)
        );

        let mut bad_count = baseline.clone();
        bad_count["taskGroups"][0]["instructionSignature"]["expectedSlotCount"] = json!(3);
        assert!(
            evaluate_quality(&bad_count, Some(&physical))["hardFailures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|code| code == CARDINALITY_SLOT_MISMATCH)
        );

        let mut missing_provenance = baseline.clone();
        missing_provenance["answerSlots"]["q14"]["sourceAnchors"] = json!([]);
        let report = evaluate_quality(&missing_provenance, Some(&physical));
        assert!(issue_for_target(&report, PROVENANCE_MISSING, "q14"));

        let mut manual_provenance = missing_provenance;
        manual_provenance["answerSlots"]["q14"]["provenanceStatus"] = json!("manual");
        let report = evaluate_quality(&manual_provenance, Some(&physical));
        assert!(!issue_for_target(&report, PROVENANCE_MISSING, "q14"));
    }

    #[test]
    fn instruction_and_signature_provenance_are_required_for_ready() {
        let baseline = early_approaches();
        let physical = valid_physical_shadow(&baseline);
        let task_id = baseline["taskGroups"][0]["taskId"]
            .as_str()
            .unwrap()
            .to_owned();

        let ready = evaluate_quality(&baseline, Some(&physical));
        assert_eq!(ready["state"], "ready", "{ready:#}");
        assert!(!issue_for_target(
            &ready,
            INSTRUCTION_PROVENANCE_MISSING,
            &task_id
        ));
        assert!(!issue_for_target(
            &ready,
            INSTRUCTION_SIGNATURE_EVIDENCE_MISSING,
            &task_id
        ));

        let mut empty_instructions = baseline.clone();
        empty_instructions["taskGroups"][0]["instructions"] = json!([]);
        let report = evaluate_quality(&empty_instructions, Some(&physical));
        assert_eq!(report["state"], "blocked");
        assert!(issue_for_target(
            &report,
            INSTRUCTION_PROVENANCE_MISSING,
            &task_id
        ));

        let mut unanchored_instruction = baseline.clone();
        unanchored_instruction["taskGroups"][0]["instructions"][0]["sourceAnchors"] = json!([]);
        let report = evaluate_quality(&unanchored_instruction, Some(&physical));
        assert_eq!(report["state"], "blocked");
        assert!(issue_for_target(
            &report,
            INSTRUCTION_PROVENANCE_MISSING,
            &task_id
        ));

        let mut empty_signature_evidence = baseline;
        empty_signature_evidence["taskGroups"][0]["instructionSignature"]["evidenceAnchors"] =
            json!([]);
        let report = evaluate_quality(&empty_signature_evidence, Some(&physical));
        assert_eq!(report["state"], "blocked");
        assert!(issue_for_target(
            &report,
            INSTRUCTION_SIGNATURE_EVIDENCE_MISSING,
            &task_id
        ));
    }

    #[test]
    fn duplicate_assets_and_physical_hash_mismatch_are_blocking() {
        let baseline = early_approaches();
        let mut duplicate = baseline.clone();
        let descriptor = json!({
            "assetId": "figure-1",
            "kind": "raster_image",
            "mime": "image/png",
            "relativePath": "assets/figure-1.png",
            "sha256": "b".repeat(64),
            "byteLength": 12,
            "extractionMode": "embedded"
        });
        duplicate["assets"] = json!([descriptor.clone(), descriptor]);
        let report = evaluate_quality(&duplicate, Some(&valid_physical_shadow(&baseline)));
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == ASSET_ID_DUPLICATE));

        let mut mismatched = baseline.clone();
        mismatched["assets"] = json!([{
            "assetId": "figure-1",
            "kind": "raster_image",
            "mime": "image/png",
            "relativePath": "assets/figure-1.png",
            "sha256": "b".repeat(64),
            "byteLength": 12,
            "extractionMode": "embedded"
        }]);
        let mut physical = valid_physical_shadow(&baseline);
        physical["assets"] = json!([{
            "assetId": "figure-1",
            "sha256": "c".repeat(64)
        }]);
        let report = evaluate_quality(&mismatched, Some(&physical));
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == ASSET_HASH_MISMATCH));
    }
}
