use crate::validator::json_issue;
use chrono::Utc;
use serde_json::{json, Value};

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
            } else if let Some(text) = option.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
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
                let text = text.as_str().unwrap_or("").trim();
                if !label.is_empty() && !text.is_empty() {
                    labels.push(label.to_string());
                }
            }
        }
    }
    labels
}

fn choice_option_set_incomplete(question: &Value) -> bool {
    // Authoring interaction types are radio/checkbox; group kinds stay single_choice/multi_choice.
    match interaction_type(question) {
        "radio" | "checkbox" | "single_choice" | "multi_choice" => {
            choice_option_labels(question).len() < 2
        }
        _ => false,
    }
}

pub(crate) fn refresh_authoring_review_state(ir: &mut Value) -> u32 {
    let mut needs_review = 0u32;
    let mut total_questions = 0u32;
    let mut verified_questions = 0u32;

    if let Some(groups) = ir.get_mut("groups").and_then(Value::as_array_mut) {
        for group in groups {
            let mut group_question_count = 0u32;
            let mut group_verified_questions = 0u32;
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
                    if question_prompt_is_empty(question) {
                        needs_review += 1;
                    }
                    if choice_option_set_incomplete(question) {
                        needs_review += 1;
                    }
                }
            }
            let all_group_questions_verified =
                group_question_count > 0 && group_question_count == group_verified_questions;
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
            json!(total_questions > 0 && total_questions == verified_questions),
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
        let group_id = group
            .get("groupId")
            .and_then(Value::as_str)
            .unwrap_or("unknown-group");
        if value_confidence(group) < 0.85 && !value_verified(group) {
            issues.push(json_issue(
                "AuthoringIR",
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
                issues.push(json_issue(
                    "AuthoringIR",
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
            && !value_verified(group)
        {
            issues.push(json_issue(
                "AuthoringIR",
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
            if question
                .get("requiresManualQuestionImport")
                .and_then(Value::as_bool)
                == Some(true)
                && !value_verified(question)
            {
                issues.push(json_issue(
                    "AuthoringIR",
                    &format!("$.groups[{}].questions[{}].prompt", group_id, qid),
                    "Question prompt must be manually imported from the source before publish",
                ));
            }
            // Empty prompt is a hard product gate: student runtime cannot render a usable item.
            // Marking verified alone must not clear this — the author must supply text.
            if question_prompt_is_empty(question) {
                issues.push(json_issue(
                    "AuthoringIR",
                    &format!("$.groups[{}].questions[{}].prompt", group_id, qid),
                    "Question prompt is empty and must be completed before publish",
                ));
            }
            if choice_option_set_incomplete(question) {
                issues.push(json_issue(
                    "AuthoringIR",
                    &format!(
                        "$.groups[{}].questions[{}].interaction.options",
                        group_id, qid
                    ),
                    "Single/multi choice question needs at least two complete options before publish",
                ));
            }
            if value_confidence(question) < 0.85 && !value_verified(question) {
                issues.push(json_issue(
                    "AuthoringIR",
                    &format!("$.groups[{}].questions[{}].verified", group_id, qid),
                    "Low-confidence question requires human verification before publish",
                ));
            }
        }
    }
    issues
}
