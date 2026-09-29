use serde_json::{json, Map, Value};

pub(crate) fn answer_key_from_v1(authoring: &Value) -> Map<String, Value> {
    let mut answers = Map::new();
    let source = authoring
        .get("answerKey")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (key, value) in source {
        let normalized = normalize_answer(value);
        answers.insert(key, normalized);
    }
    answers
}

pub(crate) fn answer_value_for_slot(
    answer_key: &Map<String, Value>,
    slot_id: &str,
    question_number: u32,
) -> Value {
    answer_value_for_slot_with_option_labels(answer_key, slot_id, question_number, &[])
}

pub(crate) fn answer_value_for_slot_with_option_labels(
    answer_key: &Map<String, Value>,
    slot_id: &str,
    question_number: u32,
    option_labels: &[String],
) -> Value {
    let value = answer_key
        .get(slot_id)
        .or_else(|| answer_key.get(&format!("q{question_number}")))
        .cloned()
        .unwrap_or(Value::Null);
    if value.is_null() || value.as_str().is_some_and(|text| text.trim().is_empty()) {
        return json!({"kind":"unresolved"});
    }
    if let Some(text) = value.as_str() {
        if let Some(labels) = parse_option_labels(text, option_labels) {
            return json!({"kind":"option","labels":labels,"assignment":"per_slot"});
        }
        let labels = text
            .split(|ch: char| matches!(ch, ',' | '/' | '&' | ' '))
            .map(|part| part.trim().to_ascii_uppercase())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if labels
            .iter()
            .all(|label| label.len() == 1 && label.as_bytes()[0].is_ascii_uppercase())
            && !labels.is_empty()
        {
            return json!({"kind":"option","labels":labels,"assignment":"per_slot"});
        }
        return json!({"kind":"text","values":[text.trim()],"normalization":"ielts_default"});
    }
    json!({"kind":"unresolved"})
}

pub(crate) fn option_labels_for_slot(task_groups: &[Value], slot_id: &str) -> Vec<String> {
    for task in task_groups {
        let responses = task.get("responseGroups").and_then(Value::as_array);
        let Some(response) = responses.into_iter().flatten().find(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .is_some_and(|slot_ids| slot_ids.iter().any(|id| id.as_str() == Some(slot_id)))
        }) else {
            continue;
        };
        let response_options = response
            .get("options")
            .and_then(Value::as_array)
            .filter(|options| !options.is_empty());
        let task_options = task
            .pointer("/optionBank/options")
            .and_then(Value::as_array);
        return response_options
            .or(task_options)
            .into_iter()
            .flatten()
            .filter_map(|option| option.get("label").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect();
    }
    Vec::new()
}

fn parse_option_labels(text: &str, allowed: &[String]) -> Option<Vec<String>> {
    if allowed.is_empty() {
        return None;
    }
    let is_truth_label_set = is_fixed_truth_label_set(allowed);
    let canonical = |value: &str| -> Option<String> {
        let normalized = normalize_label(value);
        let matches: Vec<&String> = allowed
            .iter()
            .filter(|label| {
                let candidate = normalize_label(label);
                candidate == normalized
                    || (is_truth_label_set
                        && candidate.replace(' ', "") == normalized.replace(' ', ""))
            })
            .collect();
        if matches.len() == 1 {
            return Some(matches[0].clone());
        }
        if matches.is_empty()
            && is_truth_label_set
            && matches_fixed_label_alias(&normalized.replace(' ', ""))
        {
            let aliases: Vec<&String> = allowed
                .iter()
                .filter(|label| normalize_label(label).replace(' ', "") == "NOTGIVEN")
                .collect();
            return (aliases.len() == 1).then(|| aliases[0].clone());
        }
        None
    };
    if let Some(label) = canonical(text) {
        return Some(vec![label]);
    }
    let lower = text.to_ascii_lowercase();
    let pieces = lower
        .split(|ch: char| matches!(ch, ',' | '/' | '&' | ';'))
        .flat_map(|piece| piece.split(" and "))
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if pieces.len() < 2 {
        return None;
    }
    let labels = pieces
        .iter()
        .map(|piece| canonical(piece.trim()))
        .collect::<Option<Vec<_>>>()?;
    Some(labels)
}

fn normalize_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase()
}

fn matches_fixed_label_alias(label: &str) -> bool {
    matches!(label, "NOTSTATED" | "NOTMENTIONED")
}

fn is_fixed_truth_label_set(allowed: &[String]) -> bool {
    let labels: Vec<String> = allowed.iter().map(|label| normalize_label(label)).collect();
    let has = |label: &str| labels.iter().any(|candidate| candidate == label);
    has("NOT GIVEN") && ((has("TRUE") && has("FALSE")) || (has("YES") && has("NO")))
}
fn normalize_answer(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(text.trim().to_string()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_truth_labels_are_stored_as_option_answers() {
        let answers = Map::from_iter([("q1".to_string(), json!("TRUE"))]);
        let result = answer_value_for_slot_with_option_labels(
            &answers,
            "q1",
            1,
            &[
                "TRUE".to_string(),
                "FALSE".to_string(),
                "NOT GIVEN".to_string(),
            ],
        );
        assert_eq!(
            result,
            json!({"kind":"option","labels":["TRUE"],"assignment":"per_slot"})
        );
    }

    #[test]
    fn fixed_truth_alias_uses_the_printed_option_label() {
        let answers = Map::from_iter([("q1".to_string(), json!("NOT STATED"))]);
        let result = answer_value_for_slot_with_option_labels(
            &answers,
            "q1",
            1,
            &[
                "TRUE".to_string(),
                "FALSE".to_string(),
                "NOT GIVEN".to_string(),
            ],
        );
        assert_eq!(
            result,
            json!({"kind":"option","labels":["NOT GIVEN"],"assignment":"per_slot"})
        );
    }

    #[test]
    fn pdf_spaced_fixed_truth_answer_uses_the_printed_option_label() {
        let answers = Map::from_iter([("q1".to_string(), json!("T R U E"))]);
        let result = answer_value_for_slot_with_option_labels(
            &answers,
            "q1",
            1,
            &[
                "TRUE".to_string(),
                "FALSE".to_string(),
                "NOT GIVEN".to_string(),
            ],
        );
        assert_eq!(
            result,
            json!({"kind":"option","labels":["TRUE"],"assignment":"per_slot"})
        );
    }

    #[test]
    fn answer_outside_the_choice_labels_keeps_its_text_shape() {
        let answers = Map::from_iter([("q1".to_string(), json!("eggs"))]);
        let result = answer_value_for_slot_with_option_labels(
            &answers,
            "q1",
            1,
            &[
                "TRUE".to_string(),
                "FALSE".to_string(),
                "NOT GIVEN".to_string(),
            ],
        );
        assert_eq!(
            result,
            json!({"kind":"text","values":["eggs"],"normalization":"ielts_default"})
        );
    }
}
