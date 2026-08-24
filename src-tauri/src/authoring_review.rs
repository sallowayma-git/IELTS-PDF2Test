use crate::{authoring_pipeline::dynamic_completion_foreign_slots, validator::json_issue};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashSet;

fn coded_issue(code: &str, path: &str, message: &str) -> Value {
    let mut issue = json_issue("AuthoringIR", path, message);
    if let Some(object) = issue.as_object_mut() {
        object.insert("code".to_string(), json!(code));
    }
    issue
}

pub(crate) fn answer_is_empty(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.trim().is_empty(),
        Some(Value::Array(items)) => {
            items.is_empty() || items.iter().all(|item| answer_is_empty(Some(item)))
        }
        Some(Value::Object(items)) => items.is_empty(),
        Some(_) => false,
    }
}

fn value_confidence(value: &Value) -> f64 {
    value
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

fn value_verified(value: &Value) -> bool {
    value
        .get("verified")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn review_warning_count(value: &Value) -> usize {
    value
        .get("reviewWarnings")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|warning| !warning.trim().is_empty())
                .count()
        })
        .unwrap_or(0)
}

fn question_prompt_is_empty(question: &Value) -> bool {
    question
        .get("prompt")
        .and_then(Value::as_str)
        .map(|prompt| prompt.trim().is_empty())
        .unwrap_or(true)
}

fn interaction_type(question: &Value) -> &str {
    question
        .pointer("/interaction/type")
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn choice_option_labels(question: &Value) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(options) = question
        .pointer("/interaction/options")
        .and_then(Value::as_array)
    {
        for option in options {
            if let Some(label) = option.as_str() {
                let trimmed = label.trim();
                if !trimmed.is_empty() {
                    labels.push(trimmed.to_string());
                }
            } else if let Some(label) = option.get("label").and_then(Value::as_str) {
                let trimmed = label.trim();
                if !trimmed.is_empty() {
                    labels.push(trimmed.to_string());
                }
            }
        }
    }
    if labels.is_empty() {
        if let Some(option_texts) = question
            .pointer("/interaction/optionTexts")
            .and_then(Value::as_object)
        {
            for (label, text) in option_texts {
                let label = label.trim();
                if !label.is_empty() && text.is_string() {
                    labels.push(label.to_string());
                }
            }
        }
    }
    labels
}

fn choice_option_text<'a>(question: &'a Value, label: &str) -> Option<&'a str> {
    if let Some(text) = question
        .pointer("/interaction/optionTexts")
        .and_then(Value::as_object)
        .and_then(|texts| {
            texts.iter().find_map(|(candidate, value)| {
                candidate
                    .trim()
                    .eq_ignore_ascii_case(label.trim())
                    .then_some(value)
            })
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text);
    }
    question
        .pointer("/interaction/options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .find(|option| {
            option
                .get("label")
                .and_then(Value::as_str)
                .map(|candidate| candidate.trim().eq_ignore_ascii_case(label.trim()))
                == Some(true)
        })
        .and_then(|option| option.get("text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn option_labels_are_unique_and_contiguous(labels: &[String]) -> bool {
    let mut unique = HashSet::new();
    if labels
        .iter()
        .map(|label| label.trim().to_ascii_uppercase())
        .any(|label| !unique.insert(label))
    {
        return false;
    }
    let letters = labels
        .iter()
        .map(|label| label.trim())
        .map(|label| {
            let mut chars = label.chars();
            let first = chars.next()?;
            (chars.next().is_none() && first.is_ascii_alphabetic())
                .then_some(first.to_ascii_uppercase())
        })
        .collect::<Option<Vec<_>>>();
    if let Some(letters) = letters {
        return letters
            .iter()
            .enumerate()
            .all(|(index, label)| *label == (b'A' + index as u8) as char);
    }
    let roman_value = |label: &str| match label.trim().to_ascii_lowercase().as_str() {
        "i" => Some(1usize),
        "ii" => Some(2),
        "iii" => Some(3),
        "iv" => Some(4),
        "v" => Some(5),
        "vi" => Some(6),
        "vii" => Some(7),
        "viii" => Some(8),
        "ix" => Some(9),
        "x" => Some(10),
        "xi" => Some(11),
        "xii" => Some(12),
        _ => None,
    };
    let romans = labels
        .iter()
        .map(|label| roman_value(label))
        .collect::<Option<Vec<_>>>();
    romans
        .map(|values| {
            values
                .iter()
                .enumerate()
                .all(|(index, value)| *value == index + 1)
        })
        .unwrap_or(true)
}

fn group_instruction_text(group: &Value) -> String {
    fn append(value: Option<&Value>, out: &mut Vec<String>) {
        match value {
            Some(Value::String(text)) if !text.trim().is_empty() => out.push(text.clone()),
            Some(Value::Array(items)) => {
                for item in items {
                    append(Some(item), out);
                }
            }
            _ => {}
        }
    }

    let mut parts = Vec::new();
    append(group.get("instruction"), &mut parts);
    append(group.get("instructionText"), &mut parts);
    append(group.pointer("/layout/notes"), &mut parts);
    parts.join(" ")
}

fn is_completion_kind(kind: &str) -> bool {
    matches!(
        kind,
        "summary_completion" | "table_completion" | "diagram_completion" | "sentence_completion"
    )
}

fn interaction_has_option_bank(question: &Value) -> bool {
    matches!(
        interaction_type(question),
        "radio" | "checkbox" | "single_choice" | "multi_choice" | "matching" | "classification"
    )
}

fn normalized_label(label: &str) -> String {
    let trimmed = label.trim();
    if trimmed.len() == 1 && trimmed.as_bytes()[0].is_ascii_alphabetic() {
        trimmed.to_ascii_uppercase()
    } else {
        trimmed.to_ascii_lowercase()
    }
}

fn has_duplicate_labels(labels: &[String]) -> bool {
    let mut seen = HashSet::new();
    labels
        .iter()
        .map(|label| normalized_label(label))
        .any(|label| !seen.insert(label))
}

fn labels_match_declared(actual: &[String], declared: &[String]) -> bool {
    actual.len() == declared.len()
        && declared.iter().all(|expected| {
            actual
                .iter()
                .any(|candidate| normalized_label(candidate) == normalized_label(expected))
        })
}

fn declared_letter_labels(group: &Value) -> Option<Vec<String>> {
    // Matching headings use Roman-numeral options. Passage section ranges
    // such as A-H are source anchors, not an alphabetic answer bank.
    if group.get("kind").and_then(Value::as_str) == Some("heading_matching") {
        return None;
    }

    let normalized = group_instruction_text(group).to_ascii_uppercase().replace(
        ['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}'],
        "-",
    );
    let compact = normalized
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    for terminal in ('C'..='Z').rev() {
        let range = format!("A-{terminal}");
        let labels = ('A'..=terminal)
            .map(|label| label.to_string())
            .collect::<Vec<_>>();
        let initial = labels[..labels.len() - 1].join(",");
        let comma = labels.join(",");
        let with_or = format!("{initial}OR{terminal}");
        let with_comma_or = format!("{initial},OR{terminal}");
        let verbal_range = format!("ATO{terminal}");
        if compact.contains(&range)
            || compact.contains(&comma)
            || compact.contains(&with_or)
            || compact.contains(&with_comma_or)
            || compact.contains(&verbal_range)
        {
            return Some(labels);
        }
    }
    None
}

#[derive(Clone, Copy)]
struct ReviewDefect {
    code: &'static str,
    message: &'static str,
}

const PROMPT_MISSING: ReviewDefect = ReviewDefect {
    code: "QUESTION_PROMPT_MISSING",
    message: "Question prompt is empty and must be completed before publish",
};
const QUESTION_SHEET_MISSING: ReviewDefect = ReviewDefect {
    code: "QUESTION_SHEET_MISSING",
    message: "Question prompt must be manually imported from the source before publish",
};
const DUPLICATE_LABELS: ReviewDefect = ReviewDefect {
    code: "OPTION_LABELS_DUPLICATE",
    message: "Choice option labels must be unique before publish",
};
const OPTIONS_INCOMPLETE: ReviewDefect = ReviewDefect {
    code: "OPTION_RUN_INCOMPLETE",
    message: "Choice or matching question needs at least two complete options and a complete source-backed option set before publish",
};
const OPTION_DECLARATION_MISMATCH: ReviewDefect = ReviewDefect {
    code: "OPTION_BANK_DECLARATION_MISMATCH",
    message: "Recovered option labels do not match the complete option range declared by the source instruction",
};
const COMPLETION_BANK_INTERACTION_MISSING: ReviewDefect = ReviewDefect {
    code: "COMPLETION_OPTION_BANK_INTERACTION_MISSING",
    message: "Completion declares a letter option bank but its interaction is free text",
};
const COMPLETION_PROMPT_FOREIGN_SLOTS: ReviewDefect = ReviewDefect {
    code: "COMPLETION_PROMPT_CONTAINS_FOREIGN_SLOTS",
    message: "Completion prompt contains another question's numbered response slot and must be repaired before publish",
};

fn question_hard_defects(
    question: &Value,
    group_kind: &str,
    declared: Option<&[String]>,
    manual_import: bool,
) -> Vec<ReviewDefect> {
    let mut defects = Vec::new();
    if manual_import {
        defects.push(QUESTION_SHEET_MISSING);
    }
    if question_prompt_is_empty(question) {
        defects.push(PROMPT_MISSING);
    }
    if is_completion_kind(group_kind) {
        let number = question
            .get("displayNumber")
            .and_then(Value::as_str)
            .and_then(|display| display.parse::<u32>().ok())
            .or_else(|| {
                question
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| id.strip_prefix('q'))
                    .and_then(|display| display.parse::<u32>().ok())
            });
        let prompt = question
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if number.is_some_and(|number| {
            !dynamic_completion_foreign_slots(prompt, number, 1, 40).is_empty()
        }) {
            defects.push(COMPLETION_PROMPT_FOREIGN_SLOTS);
        }
    }
    if is_completion_kind(group_kind)
        && declared.is_some()
        && !interaction_has_option_bank(question)
    {
        defects.push(COMPLETION_BANK_INTERACTION_MISSING);
    }
    if !interaction_has_option_bank(question) {
        return defects;
    }

    let labels = choice_option_labels(question);
    let duplicate_labels = has_duplicate_labels(&labels);
    if duplicate_labels {
        defects.push(DUPLICATE_LABELS);
    }
    let sequence_incomplete = labels.len() < 2
        || (!duplicate_labels && !option_labels_are_unique_and_contiguous(&labels));
    // Matching-information uses paragraph/section labels whose display text
    // lives in the passage rather than a separate option bank.
    let text_incomplete = group_kind != "matching_information"
        && !matches!(group_kind, "true_false_not_given" | "yes_no_not_given")
        && labels
            .iter()
            .any(|label| choice_option_text(question, label).is_none());
    if sequence_incomplete || text_incomplete {
        defects.push(OPTIONS_INCOMPLETE);
    }
    if declared.is_some_and(|expected| !labels_match_declared(&labels, expected)) {
        defects.push(OPTION_DECLARATION_MISMATCH);
    }
    defects
}

fn choice_option_set_incomplete(question: &Value, group_kind: &str) -> bool {
    question_hard_defects(question, group_kind, None, false)
        .iter()
        .any(|defect| {
            matches!(
                defect.code,
                "OPTION_LABELS_DUPLICATE" | "OPTION_RUN_INCOMPLETE"
            )
        })
}

pub(crate) fn refresh_authoring_review_state(ir: &mut Value) -> u32 {
    let mut needs_review = 0u32;
    let mut total_questions = 0u32;
    let mut verified_questions = 0u32;
    let mut hard_defects = 0u32;

    if let Some(groups) = ir.get_mut("groups").and_then(Value::as_array_mut) {
        for group in groups {
            let group_kind = group
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let group_declared_labels = declared_letter_labels(group);
            let group_requires_manual_import = group
                .get("requiresManualQuestionImport")
                .and_then(Value::as_bool)
                == Some(true);
            let mut group_question_count = 0u32;
            let mut group_verified_questions = 0u32;
            let mut group_hard_defects = 0u32;
            if group_requires_manual_import {
                needs_review += 1;
                hard_defects += 1;
                group_hard_defects += 1;
            }
            if let Some(questions) = group.get_mut("questions").and_then(Value::as_array_mut) {
                for question in questions {
                    total_questions += 1;
                    group_question_count += 1;
                    if value_verified(question) {
                        verified_questions += 1;
                        group_verified_questions += 1;
                    }
                    if value_confidence(question) < 0.85 && !value_verified(question) {
                        needs_review += 1;
                    }
                    let manual_import = question
                        .get("requiresManualQuestionImport")
                        .and_then(Value::as_bool)
                        == Some(true);
                    let defects = question_hard_defects(
                        question,
                        &group_kind,
                        group_declared_labels.as_deref(),
                        manual_import,
                    );
                    if !defects.is_empty() {
                        needs_review += defects.len() as u32;
                        hard_defects += 1;
                        group_hard_defects += 1;
                    }
                }
            }
            let all_group_questions_verified = group_question_count > 0
                && group_question_count == group_verified_questions
                && group_hard_defects == 0;
            if let Some(obj) = group.as_object_mut() {
                obj.insert("verified".to_string(), json!(all_group_questions_verified));
            }
            if value_confidence(group) < 0.85 && !all_group_questions_verified {
                needs_review += 1;
            }
            if review_warning_count(group) > 0 && !all_group_questions_verified {
                needs_review += 1;
            }
        }
    }

    if let Some(audit) = ir.get_mut("audit").and_then(Value::as_object_mut) {
        audit.insert(
            "humanVerified".to_string(),
            json!(
                total_questions > 0 && total_questions == verified_questions && hard_defects == 0
            ),
        );
        audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
    }

    needs_review
}

pub(crate) fn authoring_review_issues(ir: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    for group in ir
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let group_kind = group
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let group_declared_labels = declared_letter_labels(group);
        let group_id = group
            .get("groupId")
            .and_then(Value::as_str)
            .unwrap_or("unknown-group");
        if value_confidence(group) < 0.85 && !value_verified(group) {
            issues.push(coded_issue(
                "GROUP_VERIFICATION_REQUIRED",
                &format!("$.groups[{}].verified", group_id),
                "Low-confidence group requires human verification before publish",
            ));
        }
        for warning in group
            .get("reviewWarnings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|warning| !warning.trim().is_empty())
        {
            if !value_verified(group) {
                issues.push(coded_issue(
                    "GROUP_VERIFICATION_REQUIRED",
                    &format!("$.groups[{}].reviewWarnings", group_id),
                    &format!(
                        "Question-group classification warning requires author review: {}",
                        warning
                    ),
                ));
            }
        }
        if group
            .get("requiresManualQuestionImport")
            .and_then(Value::as_bool)
            == Some(true)
        {
            issues.push(coded_issue(
                "QUESTION_SHEET_MISSING",
                &format!("$.groups[{}].questions", group_id),
                "Umbrella question range requires manually imported concrete prompts before publish",
            ));
        }
        for question in group
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let qid = question
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("unknown-question");
            let manual_import = question
                .get("requiresManualQuestionImport")
                .and_then(Value::as_bool)
                == Some(true);
            let question_path = format!("$.groups[{}].questions[{}]", group_id, qid);
            for defect in question_hard_defects(
                question,
                group_kind,
                group_declared_labels.as_deref(),
                manual_import,
            ) {
                let suffix = if matches!(
                    defect.code,
                    "QUESTION_PROMPT_MISSING"
                        | "QUESTION_SHEET_MISSING"
                        | "COMPLETION_PROMPT_CONTAINS_FOREIGN_SLOTS"
                ) {
                    ".prompt"
                } else {
                    ".interaction.options"
                };
                issues.push(coded_issue(
                    defect.code,
                    &format!("{}{}", question_path, suffix),
                    defect.message,
                ));
            }
            if value_confidence(question) < 0.85 && !value_verified(question) {
                issues.push(coded_issue(
                    "QUESTION_VERIFICATION_REQUIRED",
                    &format!("{}.verified", question_path),
                    "Low-confidence question requires human verification before publish",
                ));
            }
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_specialized_matching_and_response_legends_clear_review_gate() {
        let mut ir = json!({
            "groups": [
                {
                    "groupId": "matching-information",
                    "kind": "matching_information",
                    "confidence": 0.95,
                    "verified": true,
                    "questions": [{
                        "id": "q1",
                        "prompt": "Which paragraph contains the following information?",
                        "interaction": {
                            "type": "matching",
                            "options": ["A", "B", "C"]
                        },
                        "confidence": 0.95,
                        "verified": true
                    }]
                },
                {
                    "groupId": "heading-matching",
                    "kind": "heading_matching",
                    "confidence": 0.95,
                    "verified": true,
                    "questions": [{
                        "id": "q2",
                        "prompt": "Paragraph A",
                        "interaction": {
                            "type": "matching",
                            "options": ["i", "ii", "iii"],
                            "optionTexts": {
                                "i": "A surprising beginning",
                                "ii": "A change of direction",
                                "iii": "An unresolved problem"
                            }
                        },
                        "confidence": 0.95,
                        "verified": true
                    }]
                },
                {
                    "groupId": "tfng",
                    "kind": "true_false_not_given",
                    "confidence": 0.95,
                    "verified": true,
                    "questions": [{
                        "id": "q3",
                        "prompt": "The study began in 2010.",
                        "interaction": {
                            "type": "radio",
                            "options": ["TRUE", "FALSE", "NOT GIVEN"]
                        },
                        "confidence": 0.95,
                        "verified": true
                    }]
                },
                {
                    "groupId": "ynng",
                    "kind": "yes_no_not_given",
                    "confidence": 0.95,
                    "verified": true,
                    "questions": [{
                        "id": "q4",
                        "prompt": "The writer approves of the proposal.",
                        "interaction": {
                            "type": "radio",
                            "options": ["YES", "NO", "NOT GIVEN"]
                        },
                        "confidence": 0.95,
                        "verified": true
                    }]
                }
            ],
            "audit": {"humanVerified": false}
        });

        assert_eq!(refresh_authoring_review_state(&mut ir), 0);
        assert_eq!(ir.pointer("/audit/humanVerified"), Some(&json!(true)));
        assert!(
            authoring_review_issues(&ir).is_empty(),
            "valid specialized interactions must not be blocked: {:?}",
            authoring_review_issues(&ir)
        );
    }

    #[test]
    fn matching_and_classification_banks_fail_closed_when_text_is_missing() {
        let missing_heading = json!({
            "interaction": {
                "type": "matching",
                "options": ["i", "ii", "iii"],
                "optionTexts": {
                    "i": "A surprising beginning",
                    "ii": "A change of direction"
                }
            }
        });
        assert!(choice_option_set_incomplete(
            &missing_heading,
            "heading_matching"
        ));

        let missing_classification = json!({
            "interaction": {
                "type": "matching",
                "options": ["A", "B", "C"],
                "optionTexts": {
                    "A": "First researcher",
                    "B": "Second researcher"
                }
            }
        });
        assert!(choice_option_set_incomplete(
            &missing_classification,
            "classification"
        ));

        // Matching-information does not need duplicate display text from the
        // passage, but an interaction with fewer than two answer labels is
        // still unusable and must remain blocked.
        let missing_paragraph_labels = json!({
            "interaction": {"type": "matching", "options": ["A"]}
        });
        assert!(choice_option_set_incomplete(
            &missing_paragraph_labels,
            "matching_information"
        ));
    }

    #[test]
    fn duplicate_and_non_contiguous_choice_labels_are_hard_defects() {
        for (id, options, expected_code) in [
            (
                "duplicate",
                json!([
                    {"label": "A", "text": "first"},
                    {"label": "A", "text": "duplicate"},
                    {"label": "B", "text": "second"}
                ]),
                "OPTION_LABELS_DUPLICATE",
            ),
            (
                "gap",
                json!([
                    {"label": "A", "text": "first"},
                    {"label": "C", "text": "third"}
                ]),
                "OPTION_RUN_INCOMPLETE",
            ),
        ] {
            let mut ir = json!({
                "groups": [{
                    "groupId": id,
                    "kind": "single_choice",
                    "confidence": 0.99,
                    "questions": [{
                        "id": "q1",
                        "prompt": "Choose the best answer.",
                        "interaction": {"type": "radio", "options": options},
                        "confidence": 0.99,
                        "verified": true
                    }]
                }],
                "audit": {"humanVerified": true}
            });
            refresh_authoring_review_state(&mut ir);
            assert_eq!(ir.pointer("/audit/humanVerified"), Some(&json!(false)));
            let issues = authoring_review_issues(&ir);
            assert!(
                issues
                    .iter()
                    .any(|issue| issue.get("code") == Some(&json!(expected_code))),
                "expected {expected_code}, got {issues:?}"
            );
        }
    }

    #[test]
    fn foreign_numbered_completion_slot_is_a_stable_hard_gate() {
        let mut ir = json!({
            "groups": [{
                "groupId": "flowchart",
                "kind": "diagram_completion",
                "confidence": 0.99,
                "verified": true,
                "questions": [{
                    "id": "q11",
                    "displayNumber": "11",
                    "prompt": "_______. Once extracted, the limestone passes through a 13 ______ process.",
                    "interaction": {"type": "text"},
                    "confidence": 0.99,
                    "verified": true,
                    "requiresManualQuestionImport": true
                }]
            }],
            "audit": {"humanVerified": true}
        });

        refresh_authoring_review_state(&mut ir);
        assert_eq!(ir.pointer("/audit/humanVerified"), Some(&json!(false)));
        let issues = authoring_review_issues(&ir);
        assert!(issues.iter().any(|issue| {
            issue.get("code") == Some(&json!("COMPLETION_PROMPT_CONTAINS_FOREIGN_SLOTS"))
                && issue
                    .get("path")
                    .and_then(Value::as_str)
                    .is_some_and(|path| path.ends_with(".prompt"))
        }));

        ir.pointer_mut("/groups/0/questions/0/prompt")
            .expect("prompt")
            .clone_from(&json!(
                "The first answer is ______. 12 The next displayed question contains a ______ response."
            ));
        let issues = authoring_review_issues(&ir);
        assert!(issues.iter().any(|issue| {
            issue.get("code") == Some(&json!("COMPLETION_PROMPT_CONTAINS_FOREIGN_SLOTS"))
        }));
    }

    #[test]
    fn declared_letter_bank_is_closed_and_completion_interaction_cannot_downgrade() {
        let mut declared_mismatch = json!({
            "groups": [{
                "groupId": "declared-ak",
                "kind": "single_choice",
                "instructionText": "Choose the correct letter, A-K.",
                "confidence": 0.99,
                "questions": [{
                    "id": "q1",
                    "prompt": "Which option is correct?",
                    "interaction": {
                        "type": "radio",
                        "options": [
                            {"label": "A", "text": "one"},
                            {"label": "B", "text": "two"},
                            {"label": "C", "text": "three"},
                            {"label": "D", "text": "four"}
                        ]
                    },
                    "confidence": 0.99,
                    "verified": true
                }]
            }],
            "audit": {"humanVerified": true}
        });
        refresh_authoring_review_state(&mut declared_mismatch);
        assert_eq!(
            declared_mismatch.pointer("/audit/humanVerified"),
            Some(&json!(false))
        );
        assert!(authoring_review_issues(&declared_mismatch)
            .iter()
            .any(|issue| issue.get("code") == Some(&json!("OPTION_BANK_DECLARATION_MISMATCH"))));

        let mut completion_text = json!({
            "groups": [{
                "groupId": "completion-bank",
                "kind": "summary_completion",
                "instruction": ["Choose the correct letter, A-H."],
                "layout": {"notes": "The summary contains numbered gaps."},
                "confidence": 0.99,
                "questions": [{
                    "id": "q1",
                    "prompt": "The summary statement",
                    "interaction": {"type": "text"},
                    "confidence": 0.99,
                    "verified": true
                }]
            }],
            "audit": {"humanVerified": true}
        });
        refresh_authoring_review_state(&mut completion_text);
        assert_eq!(
            completion_text.pointer("/audit/humanVerified"),
            Some(&json!(false))
        );
        let issues = authoring_review_issues(&completion_text);
        assert!(issues
            .iter()
            .any(|issue| issue.get("code")
                == Some(&json!("COMPLETION_OPTION_BANK_INTERACTION_MISSING"))));
    }

    #[test]
    fn embedded_options_and_all_instruction_sources_are_valid() {
        let mut ir = json!({
            "groups": [{
                "groupId": "embedded",
                "kind": "single_choice",
                "instruction": ["Choose the correct"],
                "instructionText": "letter from the list below.",
                "layout": {"notes": "The available letters are A-C."},
                "confidence": 0.99,
                "questions": [{
                    "id": "q1",
                    "prompt": "Which service is included?",
                    "interaction": {
                        "type": "radio",
                        "options": [
                            {"label": "A", "text": "breakfast"},
                            {"label": "B", "text": "parking"},
                            {"label": "C", "text": "laundry"}
                        ]
                    },
                    "confidence": 0.99,
                    "verified": true
                }]
            }],
            "audit": {"humanVerified": false}
        });
        assert_eq!(refresh_authoring_review_state(&mut ir), 0);
        assert_eq!(ir.pointer("/audit/humanVerified"), Some(&json!(true)));
        assert!(authoring_review_issues(&ir).is_empty());
    }

    #[test]
    fn hard_and_review_issues_have_stable_codes() {
        let ir = json!({
            "groups": [{
                "groupId": "low-confidence",
                "kind": "short_answer",
                "confidence": 0.2,
                "reviewWarnings": ["ambiguous layout"],
                "questions": [{
                    "id": "q1",
                    "prompt": "A prompt",
                    "interaction": {"type": "text"},
                    "confidence": 0.2,
                    "verified": false
                }]
            }]
        });
        let issues = authoring_review_issues(&ir);
        assert!(!issues.is_empty());
        assert!(issues.iter().all(|issue| issue
            .get("code")
            .and_then(Value::as_str)
            .is_some_and(|code| !code.is_empty())));
    }
}
