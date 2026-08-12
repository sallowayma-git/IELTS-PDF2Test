use crate::schema::common::AssetDescriptorV2;
use crate::schema::content_doc_v2::ContentNodeV2;
use crate::schema::ielts_authoring_v2::{
    AnswerAssignmentV2, AnswerSlotParticipationV2, AnswerSlotV2, AnswerValueV2, AssignmentV2,
    DuplicateSelectionPolicyV2, IeltsAuthoringIRV2, OptionBankScopeV2, PassageCategoryV2,
    ResponseGroupKindV2, ResponseGroupV2, ResponseScoringPolicyV2, RevisionSourceV2, TaskGroupV2,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const READING_EXAM_SOURCE_V2_SCHEMA_VERSION: &str = "ReadingExamSourceV2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeExamMetaV2 {
    pub title: String,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<PassageCategoryV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeAssetManifestRefV2 {
    pub exam_id: String,
    pub assets: Vec<AssetDescriptorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimePassageV2 {
    pub content: Vec<ContentNodeV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paragraph_map: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeAuditV2 {
    pub source_schema_version: String,
    pub source_document_id: String,
    pub source_revision: u64,
    pub source_revision_kind: RevisionSourceV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReadingExamSourceV2 {
    pub schema_version: String,
    pub exam_id: String,
    pub meta: RuntimeExamMetaV2,
    pub assets: RuntimeAssetManifestRefV2,
    pub passage: RuntimePassageV2,
    pub task_groups: Vec<TaskGroupV2>,
    pub answer_slots: BTreeMap<String, AnswerSlotV2>,
    pub answer_key: BTreeMap<String, AnswerValueV2>,
    pub question_order: Vec<String>,
    pub question_display_map: BTreeMap<String, String>,
    pub audit: RuntimeAuditV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CompilerIssueV2 {
    pub code: String,
    pub message: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScoreResultV2 {
    pub correct: bool,
    pub earned_points: u32,
    pub possible_points: u32,
    pub slot_scores: BTreeMap<String, u32>,
}

pub(crate) fn compile_reading_source_v2(
    source: &IeltsAuthoringIRV2,
) -> Result<ReadingExamSourceV2, Vec<CompilerIssueV2>> {
    let Some(passage) = source.passage.as_ref() else {
        return Err(vec![compiler_issue(
            "RUNTIME_PASSAGE_MISSING",
            "Reading authoring source has no passage.",
            &source.job_id,
        )]);
    };
    let mut ordered_slots = source.answer_slots.iter().collect::<Vec<_>>();
    ordered_slots.sort_by_key(|(slot_id, slot)| (slot.question_number, (*slot_id).clone()));
    let question_order = ordered_slots
        .iter()
        .map(|(slot_id, _)| (*slot_id).clone())
        .collect::<Vec<_>>();
    let question_display_map = ordered_slots
        .iter()
        .map(|(slot_id, slot)| ((*slot_id).clone(), slot.display_label.clone()))
        .collect::<BTreeMap<_, _>>();
    let runtime = ReadingExamSourceV2 {
        schema_version: READING_EXAM_SOURCE_V2_SCHEMA_VERSION.to_string(),
        exam_id: source.exam.exam_id.clone(),
        meta: RuntimeExamMetaV2 {
            title: source.exam.title.clone(),
            language: source.exam.language.clone(),
            category: source.exam.category.clone(),
        },
        assets: RuntimeAssetManifestRefV2 {
            exam_id: source.exam.exam_id.clone(),
            assets: source.assets.clone(),
        },
        passage: RuntimePassageV2 {
            content: passage.content.clone(),
            paragraph_map: passage.paragraph_map.clone(),
        },
        task_groups: source.task_groups.clone(),
        answer_slots: source.answer_slots.clone(),
        answer_key: source.answer_key.clone(),
        question_order,
        question_display_map,
        audit: RuntimeAuditV2 {
            source_schema_version: source.schema_version.clone(),
            source_document_id: source.source_document_id.clone(),
            source_revision: source.audit.revision,
            source_revision_kind: source.audit.source.clone(),
        },
    };
    let issues = validate_reading_source_v2(&runtime);
    if issues.is_empty() {
        Ok(runtime)
    } else {
        Err(issues)
    }
}

pub(crate) fn validate_reading_source_v2(source: &ReadingExamSourceV2) -> Vec<CompilerIssueV2> {
    let mut issues = Vec::new();
    if source.schema_version != READING_EXAM_SOURCE_V2_SCHEMA_VERSION {
        issues.push(compiler_issue(
            "RUNTIME_SCHEMA_UNSUPPORTED",
            "Unsupported ReadingExamSource schema version.",
            &source.schema_version,
        ));
    }
    let ordered_slot_ids = source
        .question_order
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let answer_slot_ids = source
        .answer_slots
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if source.question_order.len() != source.answer_slots.len()
        || ordered_slot_ids.len() != source.question_order.len()
        || ordered_slot_ids != answer_slot_ids
    {
        issues.push(compiler_issue(
            "RUNTIME_QUESTION_ORDER_INVALID",
            "questionOrder must contain every slot exactly once.",
            &source.exam_id,
        ));
    }
    if source
        .question_display_map
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != answer_slot_ids
    {
        issues.push(compiler_issue(
            "RUNTIME_DISPLAY_MAP_MISMATCH",
            "questionDisplayMap must contain exactly every answer slot.",
            &source.exam_id,
        ));
    }
    if source
        .answer_key
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != answer_slot_ids
    {
        issues.push(compiler_issue(
            "RUNTIME_ANSWER_KEY_MISMATCH",
            "answerKey must contain exactly every answer slot.",
            &source.exam_id,
        ));
    }
    let mut assigned_slots = BTreeSet::new();
    let mut response_ids = BTreeSet::new();
    let mut task_ids = BTreeSet::new();
    let mut option_bank_ids = BTreeSet::new();
    for task in &source.task_groups {
        if !task_ids.insert(task.task_id.as_str()) {
            issues.push(compiler_issue(
                "RUNTIME_TASK_ID_DUPLICATE",
                "taskId must be globally unique.",
                &task.task_id,
            ));
        }
        if let Some(bank) = task.option_bank.as_ref() {
            if !option_bank_ids.insert(bank.option_bank_id.as_str()) {
                issues.push(compiler_issue(
                    "RUNTIME_OPTION_BANK_ID_DUPLICATE",
                    "optionBankId must be globally unique.",
                    &bank.option_bank_id,
                ));
            }
            if !matches!(bank.scope, OptionBankScopeV2::TaskGroup) {
                issues.push(compiler_issue(
                    "RUNTIME_OPTION_BANK_SCOPE_UNSUPPORTED",
                    "The Phase 4 runtime slice accepts task_group option banks only.",
                    &bank.option_bank_id,
                ));
            }
        }
        for response in &task.response_groups {
            validate_response_group(task, response, source, &mut issues);
            if !response_ids.insert(response.response_group_id.as_str()) {
                issues.push(compiler_issue(
                    "RUNTIME_RESPONSE_ID_DUPLICATE",
                    "responseGroupId must be globally unique.",
                    &response.response_group_id,
                ));
            }
            for slot_id in &response.slot_ids {
                if !assigned_slots.insert(slot_id.as_str()) {
                    issues.push(compiler_issue(
                        "RUNTIME_SLOT_ASSIGNED_TWICE",
                        "A slot may belong to only one response group.",
                        slot_id,
                    ));
                }
            }
        }
    }
    if assigned_slots != answer_slot_ids {
        issues.push(compiler_issue(
            "RUNTIME_SLOT_UNASSIGNED",
            "Every answer slot must belong to exactly one response group.",
            &source.exam_id,
        ));
    }
    for (slot_id, slot) in &source.answer_slots {
        if slot.slot_id != *slot_id || !source.question_order.contains(slot_id) {
            issues.push(compiler_issue(
                "RUNTIME_SLOT_ID_MISMATCH",
                "answerSlots key, slotId and questionOrder must agree.",
                slot_id,
            ));
        }
        if !source.answer_key.contains_key(slot_id) {
            issues.push(compiler_issue(
                "RUNTIME_ANSWER_KEY_MISSING",
                "Every runtime slot requires an answer key entry.",
                slot_id,
            ));
        }
        if source.question_display_map.get(slot_id) != Some(&slot.display_label) {
            issues.push(compiler_issue(
                "RUNTIME_DISPLAY_MAP_MISMATCH",
                "questionDisplayMap must match the slot display label.",
                slot_id,
            ));
        }
        if !matches!(slot.participation, AnswerSlotParticipationV2::Scoring) {
            issues.push(compiler_issue(
                "RUNTIME_SLOT_PARTICIPATION_UNSUPPORTED",
                "The Phase 4 runtime slice accepts scoring slots only.",
                slot_id,
            ));
        }
    }
    issues
}

fn validate_response_group(
    task: &TaskGroupV2,
    response: &ResponseGroupV2,
    source: &ReadingExamSourceV2,
    issues: &mut Vec<CompilerIssueV2>,
) {
    if response.slot_ids.is_empty() {
        issues.push(compiler_issue(
            "RUNTIME_RESPONSE_SLOT_EMPTY",
            "Response group must contain at least one slot.",
            &response.response_group_id,
        ));
    }
    for slot_id in &response.slot_ids {
        if !source.answer_slots.contains_key(slot_id) {
            issues.push(compiler_issue(
                "RUNTIME_RESPONSE_SLOT_MISSING",
                "Response group references an unknown answer slot.",
                slot_id,
            ));
        }
    }
    if response.slot_ids.iter().collect::<BTreeSet<_>>().len() != response.slot_ids.len() {
        issues.push(compiler_issue(
            "RUNTIME_SLOT_ASSIGNED_TWICE",
            "A response group must not contain duplicate slot IDs.",
            &response.response_group_id,
        ));
    }
    let resolved_bank_options = response.option_bank_ref.as_deref().and_then(|bank_ref| {
        let bank = task.option_bank.as_ref()?;
        (bank.option_bank_id == bank_ref).then_some(&bank.options)
    });
    let requires_options = matches!(
        response.kind,
        ResponseGroupKindV2::Choice | ResponseGroupKindV2::Matching
    ) || response.options.is_some()
        || response.option_bank_ref.is_some();
    if resolved_bank_options.is_none() && response.option_bank_ref.is_some() {
        issues.push(compiler_issue(
            "RUNTIME_OPTION_BANK_MISSING",
            "optionBankRef does not resolve within the task.",
            &response.response_group_id,
        ));
    }
    let option_values = response.options.as_ref().or(resolved_bank_options);
    if requires_options && option_values.is_none() {
        issues.push(compiler_issue(
            "RUNTIME_OPTION_BANK_MISSING",
            "An option response requires inline options or a resolvable optionBankRef.",
            &response.response_group_id,
        ));
    }
    let options = option_values.map(Vec::len).unwrap_or(0);
    if matches!(response.kind, ResponseGroupKindV2::TextEntry)
        && (response.options.is_some() || response.option_bank_ref.is_some())
    {
        issues.push(compiler_issue(
            "RUNTIME_RESPONSE_KIND_OPTION_SOURCE_MISMATCH",
            "text_entry responses must not declare inline options or an option bank.",
            &response.response_group_id,
        ));
    }
    if matches!(response.kind, ResponseGroupKindV2::TextEntry)
        && !matches!(response.assignment, AssignmentV2::PerSlot)
    {
        issues.push(compiler_issue(
            "RUNTIME_RESPONSE_ASSIGNMENT_KIND_MISMATCH",
            "text_entry responses require per_slot assignment.",
            &response.response_group_id,
        ));
    }
    for slot_id in &response.slot_ids {
        let answer = source.answer_key.get(slot_id);
        let kind_matches = match response.kind {
            ResponseGroupKindV2::Choice | ResponseGroupKindV2::Matching => {
                // An unresolved answer is an honest draft state when the source
                // answer page is image-only. It must keep the quality gate
                // blocked, but it does not make the structural runtime graph
                // malformed. Typed values remain strict once an answer exists.
                matches!(
                    answer,
                    Some(AnswerValueV2::Option { .. } | AnswerValueV2::Unresolved)
                )
            }
            ResponseGroupKindV2::TextEntry => {
                matches!(
                    answer,
                    Some(AnswerValueV2::Text { .. } | AnswerValueV2::Unresolved)
                )
            }
            _ => true,
        };
        if !kind_matches {
            issues.push(compiler_issue(
                "RUNTIME_RESPONSE_ANSWER_KIND_MISMATCH",
                "response kind and answer key kind must agree.",
                slot_id,
            ));
        }
    }
    if let Some(option_values) = option_values {
        let labels = option_values
            .iter()
            .map(|option| option.label.trim().to_ascii_uppercase())
            .collect::<Vec<_>>();
        if labels.iter().any(String::is_empty)
            || labels.iter().collect::<BTreeSet<_>>().len() != labels.len()
        {
            issues.push(compiler_issue(
                "RUNTIME_OPTION_LABELS_INVALID",
                "Runtime option labels must be non-empty and unique.",
                &response.response_group_id,
            ));
        }
        let label_set = labels.iter().cloned().collect::<BTreeSet<_>>();
        for slot_id in &response.slot_ids {
            if answer_labels(source.answer_key.get(slot_id))
                .iter()
                .any(|label| !label_set.contains(label))
            {
                issues.push(compiler_issue(
                    "RUNTIME_ANSWER_OPTION_INVALID",
                    "An answer label is not present in the resolved options.",
                    slot_id,
                ));
            }
        }
    }
    if matches!(response.assignment, AssignmentV2::UnorderedSet) {
        let expected = response.cardinality.exact.unwrap_or(0) as usize;
        if expected == 0
            || expected != response.slot_ids.len()
            || response.cardinality.min as usize != expected
            || response.cardinality.max as usize != expected
        {
            issues.push(compiler_issue(
                "RUNTIME_RESPONSE_CARDINALITY_INVALID",
                "unordered_set cardinality must be a positive exact value equal to its slot count.",
                &response.response_group_id,
            ));
        }
        if !matches!(
            response.scoring_policy,
            ResponseScoringPolicyV2::PerSlotIeltsNormalized
        ) {
            issues.push(compiler_issue(
                "RUNTIME_RESPONSE_SCORING_POLICY_UNSUPPORTED",
                "unordered_set requires per_slot_ielts_normalized scoring.",
                &response.response_group_id,
            ));
        }
        if !matches!(
            response.duplicate_policy,
            DuplicateSelectionPolicyV2::RejectSubmission
        ) {
            issues.push(compiler_issue(
                "RUNTIME_RESPONSE_DUPLICATE_POLICY_UNSUPPORTED",
                "unordered_set requires reject_submission duplicate handling.",
                &response.response_group_id,
            ));
        }
        let bank_allows_reuse = response.option_bank_ref.as_ref().is_some_and(|bank_ref| {
            task.option_bank
                .as_ref()
                .is_some_and(|bank| bank.option_bank_id == *bank_ref && bank.allow_reuse)
        });
        if response.allow_option_reuse || bank_allows_reuse {
            issues.push(compiler_issue(
                "RUNTIME_RESPONSE_OPTION_REUSE_UNSUPPORTED",
                "unordered_set requires response and option bank reuse to be disabled.",
                &response.response_group_id,
            ));
        }
        if options <= expected {
            issues.push(compiler_issue(
                "RUNTIME_RESPONSE_OPTIONS_INSUFFICIENT",
                "unordered_set requires more resolved options than answer slots.",
                &response.response_group_id,
            ));
        }
        let expected_labels = normalized_labels_for_slots(&response.slot_ids, &source.answer_key);
        let unique_expected = expected_labels.iter().collect::<BTreeSet<_>>();
        let answer_policy_valid = expected_labels.len() == expected
            && unique_expected.len() == expected
            && response.slot_ids.iter().all(|slot_id| {
                matches!(
                    source.answer_key.get(slot_id),
                    Some(AnswerValueV2::Option {
                        labels,
                        assignment: AnswerAssignmentV2::UnorderedSet,
                    }) if labels.len() == 1
                )
            });
        if !answer_policy_valid {
            issues.push(compiler_issue(
                "RUNTIME_ANSWER_KEY_POLICY_INVALID",
                "unordered_set requires one unique option label per slot and unordered_set answer assignment.",
                &response.response_group_id,
            ));
        }
    }
}

pub(crate) fn score_response_group(
    group: &ResponseGroupV2,
    answers: &BTreeMap<String, AnswerValueV2>,
    answer_key: &BTreeMap<String, AnswerValueV2>,
) -> ScoreResultV2 {
    let possible_points = group.slot_ids.len() as u32;
    if matches!(group.assignment, AssignmentV2::UnorderedSet) {
        let submitted_by_slot = group
            .slot_ids
            .iter()
            .map(|slot_id| {
                (
                    slot_id.clone(),
                    normalized_labels_for_value(answers.get(slot_id)),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let submitted_values = submitted_by_slot
            .values()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let submitted = submitted_values.iter().cloned().collect::<BTreeSet<_>>();
        let expected = normalized_labels_for_slots(&group.slot_ids, answer_key)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let duplicates_present = submitted.len() != submitted_values.len();
        let duplicates_allowed = matches!(
            group.duplicate_policy,
            DuplicateSelectionPolicyV2::IgnoreDuplicates
        );
        let valid_submission = (!duplicates_present || duplicates_allowed)
            && submitted.len() <= group.cardinality.max as usize
            && submitted_by_slot.values().all(|labels| labels.len() <= 1);
        let slot_scores = group
            .slot_ids
            .iter()
            .map(|slot_id| {
                let labels = &submitted_by_slot[slot_id];
                let score = valid_submission && labels.len() == 1 && expected.contains(&labels[0]);
                (slot_id.clone(), u32::from(score))
            })
            .collect::<BTreeMap<_, _>>();
        let matched_points = slot_scores.values().sum::<u32>().min(possible_points);
        let earned_points = match group.scoring_policy {
            ResponseScoringPolicyV2::PerSlotIeltsNormalized
            | ResponseScoringPolicyV2::PerSlotBinary => matched_points,
            ResponseScoringPolicyV2::ExactSet | ResponseScoringPolicyV2::AllOrNothing => {
                u32::from(valid_submission && submitted == expected) * possible_points
            }
        };
        let correct = !expected.is_empty()
            && valid_submission
            && submitted.len() == group.cardinality.exact.unwrap_or(group.cardinality.max) as usize
            && submitted == expected;
        return ScoreResultV2 {
            correct,
            earned_points,
            possible_points,
            slot_scores,
        };
    }
    let slot_scores = group
        .slot_ids
        .iter()
        .map(|slot_id| {
            (
                slot_id.clone(),
                u32::from(answers.get(slot_id) == answer_key.get(slot_id)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let earned_points = slot_scores.values().sum();
    ScoreResultV2 {
        correct: earned_points == possible_points && possible_points > 0,
        earned_points,
        possible_points,
        slot_scores,
    }
}

fn normalized_labels_for_slots(
    slot_ids: &[String],
    values: &BTreeMap<String, AnswerValueV2>,
) -> Vec<String> {
    slot_ids
        .iter()
        .filter_map(|slot_id| values.get(slot_id))
        .flat_map(|value| match value {
            AnswerValueV2::Option { labels, .. } => labels.clone(),
            AnswerValueV2::Text { values, .. } => values.clone(),
            AnswerValueV2::Unresolved => Vec::new(),
        })
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalized_labels_for_value(value: Option<&AnswerValueV2>) -> Vec<String> {
    match value {
        Some(AnswerValueV2::Option { labels, .. }) => labels.clone(),
        Some(AnswerValueV2::Text { values, .. }) => values.clone(),
        Some(AnswerValueV2::Unresolved) | None => Vec::new(),
    }
    .into_iter()
    .map(|value| value.trim().to_ascii_uppercase())
    .filter(|value| !value.is_empty())
    .collect()
}

fn answer_labels(value: Option<&AnswerValueV2>) -> BTreeSet<String> {
    match value {
        Some(AnswerValueV2::Option { labels, .. }) => labels
            .iter()
            .map(|label| label.trim().to_ascii_uppercase())
            .filter(|label| !label.is_empty())
            .collect(),
        _ => BTreeSet::new(),
    }
}

fn compiler_issue(code: &str, message: &str, target_id: &str) -> CompilerIssueV2 {
    CompilerIssueV2 {
        code: code.to_string(),
        message: message.to_string(),
        target_id: target_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ielts_authoring_v2::{CardinalityV2, IeltsAuthoringIRV2};
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;

    fn fixture() -> IeltsAuthoringIRV2 {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json");
        let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        serde_json::from_value(value).unwrap()
    }

    fn invalid_fixture_matrix() -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/golden/synthetic/ielts/reading-v2-invalid-fixture-matrix.json");
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn option_answer(labels: &[&str]) -> AnswerValueV2 {
        AnswerValueV2::Option {
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            assignment: AnswerAssignmentV2::UnorderedSet,
        }
    }

    #[test]
    fn early_approaches_compiles_to_one_shared_runtime_response() {
        let runtime = compile_reading_source_v2(&fixture()).unwrap();
        assert_eq!(runtime.schema_version, "ReadingExamSourceV2");
        assert_eq!(runtime.question_order, vec!["q14", "q15"]);
        let task = &runtime.task_groups[0];
        assert_eq!(task.option_bank.as_ref().unwrap().options.len(), 5);
        let response = &task.response_groups[0];
        assert_eq!(response.slot_ids, vec!["q14", "q15"]);
        assert_eq!(
            response.cardinality,
            CardinalityV2 {
                min: 2,
                max: 2,
                exact: Some(2)
            }
        );
        assert!(matches!(response.assignment, AssignmentV2::UnorderedSet));
        let prompt = serde_json::to_value(response.prompt.as_ref().unwrap()).unwrap();
        assert!(prompt.as_array().is_some_and(|nodes| {
            !nodes.is_empty()
                && nodes.iter().all(|node| {
                    node["sourceAnchors"]
                        .as_array()
                        .is_some_and(|anchors| !anchors.is_empty())
                })
        }));
        assert!(task
            .option_bank
            .as_ref()
            .unwrap()
            .options
            .iter()
            .all(|option| !option.source_anchors.is_empty()));
        assert!(runtime
            .answer_slots
            .values()
            .all(|slot| !slot.source_anchors.is_empty()));
    }

    #[test]
    fn shared_response_scoring_is_order_independent() {
        let runtime = compile_reading_source_v2(&fixture()).unwrap();
        let response = &runtime.task_groups[0].response_groups[0];
        let answers = BTreeMap::from([
            (
                "q14".to_string(),
                AnswerValueV2::Option {
                    labels: vec!["D".to_string()],
                    assignment: AnswerAssignmentV2::UnorderedSet,
                },
            ),
            (
                "q15".to_string(),
                AnswerValueV2::Option {
                    labels: vec!["B".to_string()],
                    assignment: AnswerAssignmentV2::UnorderedSet,
                },
            ),
        ]);
        assert_eq!(
            score_response_group(response, &answers, &runtime.answer_key),
            ScoreResultV2 {
                correct: true,
                earned_points: 2,
                possible_points: 2,
                slot_scores: BTreeMap::from([("q14".to_string(), 1), ("q15".to_string(), 1)])
            }
        );
    }

    #[test]
    fn shared_response_awards_one_point_for_one_correct_unique_label() {
        let runtime = compile_reading_source_v2(&fixture()).unwrap();
        let response = &runtime.task_groups[0].response_groups[0];
        let answers = BTreeMap::from([
            (
                "q14".to_string(),
                AnswerValueV2::Option {
                    labels: vec!["B".to_string()],
                    assignment: AnswerAssignmentV2::UnorderedSet,
                },
            ),
            (
                "q15".to_string(),
                AnswerValueV2::Option {
                    labels: vec!["A".to_string()],
                    assignment: AnswerAssignmentV2::UnorderedSet,
                },
            ),
        ]);
        assert_eq!(
            score_response_group(response, &answers, &runtime.answer_key),
            ScoreResultV2 {
                correct: false,
                earned_points: 1,
                possible_points: 2,
                slot_scores: BTreeMap::from([("q14".to_string(), 1), ("q15".to_string(), 0)])
            }
        );
    }

    #[test]
    fn shared_response_rejects_duplicate_and_extra_labels_and_scores_zero_matches() {
        let runtime = compile_reading_source_v2(&fixture()).unwrap();
        let response = &runtime.task_groups[0].response_groups[0];
        let cases = [
            (
                BTreeMap::from([
                    ("q14".to_string(), option_answer(&["A"])),
                    ("q15".to_string(), option_answer(&["C"])),
                ]),
                0,
                BTreeMap::from([("q14".to_string(), 0), ("q15".to_string(), 0)]),
            ),
            (
                BTreeMap::from([
                    ("q14".to_string(), option_answer(&["B"])),
                    ("q15".to_string(), option_answer(&["B"])),
                ]),
                0,
                BTreeMap::from([("q14".to_string(), 0), ("q15".to_string(), 0)]),
            ),
            (
                BTreeMap::from([
                    ("q14".to_string(), option_answer(&["B", "D"])),
                    ("q15".to_string(), option_answer(&["A"])),
                ]),
                0,
                BTreeMap::from([("q14".to_string(), 0), ("q15".to_string(), 0)]),
            ),
        ];
        for (answers, expected_points, expected_slots) in cases {
            let score = score_response_group(response, &answers, &runtime.answer_key);
            assert!(!score.correct);
            assert_eq!(score.earned_points, expected_points);
            assert_eq!(score.slot_scores, expected_slots);
        }
    }

    #[test]
    fn text_entry_response_and_text_answers_do_not_require_an_option_bank() {
        let mut runtime = compile_reading_source_v2(&fixture()).unwrap();
        let task = &mut runtime.task_groups[0];
        task.option_bank = None;
        let response = &mut task.response_groups[0];
        response.kind = ResponseGroupKindV2::TextEntry;
        response.options = None;
        response.option_bank_ref = None;
        response.assignment = AssignmentV2::PerSlot;
        response.cardinality = CardinalityV2 {
            min: 1,
            max: 1,
            exact: Some(1),
        };
        response.scoring_policy = ResponseScoringPolicyV2::PerSlotBinary;
        for slot_id in ["q14", "q15"] {
            runtime.answer_key.insert(
                slot_id.to_string(),
                AnswerValueV2::Text {
                    values: vec![format!("answer-{slot_id}")],
                    normalization: None,
                },
            );
        }

        let issue_codes = validate_reading_source_v2(&runtime)
            .into_iter()
            .map(|issue| issue.code)
            .collect::<BTreeSet<_>>();
        assert!(!issue_codes.contains("RUNTIME_OPTION_BANK_MISSING"));
        assert!(issue_codes.is_empty(), "unexpected issues: {issue_codes:?}");
    }

    #[test]
    fn invalid_fixture_matrix_has_stable_runtime_issue_codes() {
        let matrix = invalid_fixture_matrix();
        assert_eq!(
            matrix["schemaVersion"].as_str(),
            Some("ReadingV2InvalidFixtureMatrixV1")
        );
        for case in matrix["cases"].as_array().unwrap() {
            let mutation = case["mutation"].as_str().unwrap();
            let expected_code = case["expectedCode"].as_str().unwrap();
            let mut runtime = compile_reading_source_v2(&fixture()).unwrap();
            match mutation {
                "schema_version" => {
                    runtime.schema_version = "UnsupportedReadingSourceV2".to_string();
                }
                "question_order" => {
                    runtime.question_order = vec!["q14".to_string(), "q14".to_string()];
                }
                "orphan_slot" => {
                    let mut orphan = runtime.answer_slots["q15"].clone();
                    orphan.slot_id = "q16".to_string();
                    orphan.question_number = 16;
                    orphan.display_label = "16".to_string();
                    runtime.answer_slots.insert("q16".to_string(), orphan);
                    runtime
                        .answer_key
                        .insert("q16".to_string(), option_answer(&["A"]));
                    runtime.question_order.push("q16".to_string());
                    runtime
                        .question_display_map
                        .insert("q16".to_string(), "16".to_string());
                }
                "slot_id_mismatch" => {
                    runtime.answer_slots.get_mut("q14").unwrap().slot_id = "q999".to_string();
                }
                "duplicate_slot" => {
                    runtime.task_groups[0].response_groups[0]
                        .slot_ids
                        .push("q14".to_string());
                }
                "empty_response" => {
                    runtime.task_groups[0].response_groups[0].slot_ids.clear();
                }
                "missing_response_slot" => {
                    runtime.task_groups[0].response_groups[0].slot_ids[0] = "q999".to_string();
                }
                "duplicate_response_id" => {
                    let mut duplicate = runtime.task_groups[0].response_groups[0].clone();
                    duplicate.slot_ids.clear();
                    runtime.task_groups[0].response_groups.push(duplicate);
                }
                "duplicate_task_id" => {
                    let mut duplicate = runtime.task_groups[0].clone();
                    duplicate.response_groups.clear();
                    duplicate.option_bank = None;
                    runtime.task_groups.push(duplicate);
                }
                "duplicate_option_bank_id" => {
                    let mut duplicate = runtime.task_groups[0].clone();
                    duplicate.task_id = "early-approaches-q16-17".to_string();
                    duplicate.response_groups.clear();
                    runtime.task_groups.push(duplicate);
                }
                "missing_bank" => {
                    runtime.task_groups[0].response_groups[0].option_bank_ref =
                        Some("missing-bank".to_string());
                }
                "missing_option_source" => {
                    runtime.task_groups[0].response_groups[0].option_bank_ref = None;
                }
                "inline_options_dangling_bank" => {
                    let options = runtime.task_groups[0]
                        .option_bank
                        .as_ref()
                        .unwrap()
                        .options
                        .clone();
                    let response = &mut runtime.task_groups[0].response_groups[0];
                    response.options = Some(options);
                    response.option_bank_ref = Some("missing-bank".to_string());
                }
                "duplicate_option_label" => {
                    runtime.task_groups[0].option_bank.as_mut().unwrap().options[1].label =
                        "A".to_string();
                }
                "cardinality" => {
                    runtime.task_groups[0].response_groups[0].cardinality.exact = Some(1);
                }
                "answer_key" => {
                    runtime.answer_key.remove("q15");
                }
                "answer_option" => {
                    runtime
                        .answer_key
                        .insert("q14".to_string(), option_answer(&["Z"]));
                }
                "answer_key_policy" => {
                    runtime
                        .answer_key
                        .insert("q15".to_string(), option_answer(&["B"]));
                }
                "display_map" => {
                    runtime
                        .question_display_map
                        .insert("q15".to_string(), "99".to_string());
                }
                "scoring_policy" => {
                    runtime.task_groups[0].response_groups[0].scoring_policy =
                        ResponseScoringPolicyV2::ExactSet;
                }
                "duplicate_policy" => {
                    runtime.task_groups[0].response_groups[0].duplicate_policy =
                        DuplicateSelectionPolicyV2::IgnoreDuplicates;
                }
                "text_entry_option_answer" => {
                    let response = &mut runtime.task_groups[0].response_groups[0];
                    response.kind = ResponseGroupKindV2::TextEntry;
                    response.assignment = AssignmentV2::PerSlot;
                    response.cardinality = CardinalityV2 {
                        min: 1,
                        max: 1,
                        exact: Some(1),
                    };
                    response.options = None;
                    response.option_bank_ref = None;
                    runtime.task_groups[0].option_bank = None;
                }
                "text_entry_option_bank" => {
                    let response = &mut runtime.task_groups[0].response_groups[0];
                    response.kind = ResponseGroupKindV2::TextEntry;
                    response.assignment = AssignmentV2::PerSlot;
                    response.cardinality = CardinalityV2 {
                        min: 1,
                        max: 1,
                        exact: Some(1),
                    };
                }
                "choice_text_answer" => {
                    runtime.answer_key.insert(
                        "q14".to_string(),
                        AnswerValueV2::Text {
                            values: vec!["alpha".to_string()],
                            normalization: None,
                        },
                    );
                }
                "matching_text_answer" => {
                    runtime.task_groups[0].response_groups[0].kind = ResponseGroupKindV2::Matching;
                    runtime.answer_key.insert(
                        "q14".to_string(),
                        AnswerValueV2::Text {
                            values: vec!["alpha".to_string()],
                            normalization: None,
                        },
                    );
                }
                "unknown_response_kind"
                | "unknown_assignment"
                | "unknown_scoring_policy"
                | "unknown_duplicate_policy" => continue,
                "option_reuse" => {
                    runtime.task_groups[0].response_groups[0].allow_option_reuse = true;
                }
                "insufficient_options" => {
                    runtime.task_groups[0]
                        .option_bank
                        .as_mut()
                        .unwrap()
                        .options
                        .truncate(2);
                }
                "option_bank_scope" => {
                    runtime.task_groups[0].option_bank.as_mut().unwrap().scope =
                        OptionBankScopeV2::Document;
                }
                "unsupported_non_scoring" => {
                    runtime.answer_slots.get_mut("q15").unwrap().participation =
                        AnswerSlotParticipationV2::NonScoring;
                }
                unknown => panic!("unknown invalid fixture mutation: {unknown}"),
            }
            let issue_codes = validate_reading_source_v2(&runtime)
                .into_iter()
                .map(|issue| issue.code)
                .collect::<BTreeSet<_>>();
            assert!(
                issue_codes.contains(expected_code),
                "{} expected {expected_code}, got {issue_codes:?}",
                case["id"].as_str().unwrap()
            );
        }
    }

    #[test]
    fn invalid_fixture_matrix_unknown_enums_are_rejected_by_typed_deserialization() {
        let matrix = invalid_fixture_matrix();
        for mutation in [
            "unknown_response_kind",
            "unknown_assignment",
            "unknown_scoring_policy",
            "unknown_duplicate_policy",
        ] {
            let mut value =
                serde_json::to_value(compile_reading_source_v2(&fixture()).unwrap()).unwrap();
            let response = &mut value["taskGroups"][0]["responseGroups"][0];
            match mutation {
                "unknown_response_kind" => {
                    response["kind"] = Value::String("unsupported_kind".into())
                }
                "unknown_assignment" => {
                    response["assignment"] = Value::String("unsupported_assignment".into())
                }
                "unknown_scoring_policy" => {
                    response["scoringPolicy"] = Value::String("unsupported_scoring_policy".into())
                }
                "unknown_duplicate_policy" => {
                    response["duplicatePolicy"] =
                        Value::String("unsupported_duplicate_policy".into())
                }
                _ => unreachable!(),
            }
            assert!(serde_json::from_value::<ReadingExamSourceV2>(value).is_err());
            assert!(matrix["cases"]
                .as_array()
                .unwrap()
                .iter()
                .any(|case| { case["mutation"].as_str() == Some(mutation) }));
        }
    }

    #[test]
    #[ignore = "writes the PR-06 cross-repository fixture to an explicit temporary path"]
    fn writes_pr06_reading_source_v2_fixture() {
        let target = std::env::var("IELTS_READING_V2_FIXTURE_PATH")
            .expect("IELTS_READING_V2_FIXTURE_PATH must name the temporary output file");
        let target = PathBuf::from(target);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let runtime = compile_reading_source_v2(&fixture()).unwrap();
        fs::write(target, serde_json::to_vec_pretty(&runtime).unwrap()).unwrap();
    }
}
