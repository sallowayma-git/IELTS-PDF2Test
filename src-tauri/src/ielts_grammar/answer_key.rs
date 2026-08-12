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
    let value = answer_key
        .get(slot_id)
        .or_else(|| answer_key.get(&format!("q{question_number}")))
        .cloned()
        .unwrap_or(Value::Null);
    if value.is_null() || value.as_str().is_some_and(|text| text.trim().is_empty()) {
        return json!({"kind":"unresolved"});
    }
    if let Some(text) = value.as_str() {
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

fn normalize_answer(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(text.trim().to_string()),
        other => other,
    }
}
