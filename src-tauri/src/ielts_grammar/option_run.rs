use serde_json::{json, Value};

use super::instruction_zone::{normalize_instruction_text, SemanticLine};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OptionItemCandidate {
    pub label: String,
    pub text: String,
    pub line_id: String,
    pub source_anchor: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OptionRun {
    pub options: Vec<OptionItemCandidate>,
    pub source_anchors: Vec<Value>,
    pub incomplete: bool,
}

pub(crate) fn detect_option_runs(lines: &[SemanticLine]) -> Vec<OptionRun> {
    let mut runs = Vec::new();
    let mut current = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let line = &lines[index];
        if let Some(mut option) = parse_option_line(line) {
            // PDF extractors often split "A" and "changing the bed linen" onto
            // separate lines. Absorb the next non-label prose line as option text
            // before deciding whether the run is incomplete.
            if option.text.is_empty() {
                if let Some(continuation) = lines.get(index + 1) {
                    if parse_option_line(continuation).is_none()
                        && !looks_like_question_number_line(&continuation.text)
                    {
                        let continuation_text = normalize_instruction_text(&continuation.text);
                        if !continuation_text.is_empty() {
                            option.text = continuation_text;
                            option.source_anchor = merge_source_anchors(
                                &option.source_anchor,
                                &continuation.source_anchor,
                            );
                            index += 1;
                        }
                    }
                }
            }
            if current.is_empty() || advances_option_sequence(&current, &option) {
                current.push(option);
            } else {
                runs.push(materialize_run(std::mem::take(&mut current)));
                current.push(option);
            }
        } else if !current.is_empty() {
            runs.push(materialize_run(std::mem::take(&mut current)));
        }
        index += 1;
    }
    if !current.is_empty() {
        runs.push(materialize_run(current));
    }
    runs
}

fn looks_like_question_number_line(text: &str) -> bool {
    let trimmed = normalize_instruction_text(text);
    let mut end = 0usize;
    for (index, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    end > 0 && end <= 3 && trimmed[end..].trim().is_empty()
}

fn merge_source_anchors(primary: &Value, secondary: &Value) -> Value {
    let mut merged = primary.clone();
    let mut node_ids = primary
        .get("nodeIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    if let Some(extra) = secondary.get("nodeIds").and_then(Value::as_array) {
        for node_id in extra {
            if !node_ids.iter().any(|existing| existing == node_id) {
                node_ids.push(node_id.clone());
            }
        }
    }
    if !node_ids.is_empty() {
        merged["nodeIds"] = Value::Array(node_ids);
    }
    merged
}

pub(crate) fn expected_option_labels(option_alphabet: Option<&str>) -> Option<Vec<String>> {
    let alphabet = option_alphabet?.trim().to_ascii_uppercase();
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

pub(crate) fn run_matches_alphabet(run: &OptionRun, option_alphabet: Option<&str>) -> bool {
    if run.incomplete {
        return false;
    }
    let Some(expected) = expected_option_labels(option_alphabet) else {
        return true;
    };
    run.options
        .iter()
        .map(|option| option.label.to_ascii_uppercase())
        .eq(expected)
}

pub(crate) fn option_run_value(run: &OptionRun, option_id_prefix: &str) -> Vec<Value> {
    run.options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            json!({
                "optionId": format!("{option_id_prefix}-{}", index + 1),
                "label": option.label,
                "content": [text_node(
                    &format!("{option_id_prefix}-text-{}", index + 1),
                    &option.text,
                    option.source_anchor.clone(),
                )],
                "sourceAnchors": [option.source_anchor]
            })
        })
        .collect()
}

fn parse_option_line(line: &SemanticLine) -> Option<OptionItemCandidate> {
    let text = normalize_instruction_text(&line.text);
    let token_end = text
        .char_indices()
        .find(|(_, ch)| !ch.is_ascii_alphabetic())
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let token = text.get(..token_end)?;
    let is_letter = token.len() == 1 && token.chars().all(|ch| ch.is_ascii_uppercase());
    let is_roman = !token.is_empty()
        && token.len() <= 5
        && token.chars().all(|ch| matches!(ch, 'i' | 'v' | 'x'))
        && roman_value(token).is_some();
    if !is_letter && !is_roman {
        return None;
    }
    let rest = text[token_end..].trim_start();
    let rest = rest
        .strip_prefix('.')
        .or_else(|| rest.strip_prefix(')'))
        .or_else(|| rest.strip_prefix(':'))
        .unwrap_or(rest)
        .trim_start();
    Some(OptionItemCandidate {
        label: token.to_string(),
        text: rest.to_string(),
        line_id: line.id.clone(),
        source_anchor: line.source_anchor.clone(),
    })
}

fn advances_option_sequence(current: &[OptionItemCandidate], next: &OptionItemCandidate) -> bool {
    let Some(previous) = current.last() else {
        return true;
    };
    label_sequence_value(&previous.label)
        .zip(label_sequence_value(&next.label))
        .is_some_and(|(previous, next)| next > previous)
}

fn materialize_run(options: Vec<OptionItemCandidate>) -> OptionRun {
    let labels_are_contiguous = options.windows(2).all(|pair| {
        label_sequence_value(&pair[0].label)
            .zip(label_sequence_value(&pair[1].label))
            .is_some_and(|(previous, next)| next == previous + 1)
    });
    let incomplete = options.len() < 2
        || !labels_are_contiguous
        || options.iter().any(|option| option.text.is_empty());
    let source_anchors = options
        .iter()
        .map(|option| option.source_anchor.clone())
        .collect();
    OptionRun {
        options,
        source_anchors,
        incomplete,
    }
}

fn label_sequence_value(label: &str) -> Option<u32> {
    if label.len() == 1 {
        let ch = label.chars().next()?;
        if ch.is_ascii_uppercase() {
            return Some(ch as u32 - 'A' as u32 + 1);
        }
    }
    roman_value(label).map(|value| 100 + value)
}

fn roman_value(label: &str) -> Option<u32> {
    let mut total = 0;
    let mut previous = 0;
    for ch in label.chars().rev() {
        let value = match ch {
            'i' => 1,
            'v' => 5,
            'x' => 10,
            _ => return None,
        };
        if value < previous {
            total -= value;
        } else {
            total += value;
            previous = value;
        }
    }
    (total > 0).then_some(total)
}

fn text_node(id: &str, text: &str, source_anchor: Value) -> Value {
    json!({
        "type": "text",
        "id": id,
        "sourceAnchors": [source_anchor],
        "provenanceStatus": "source",
        "text": if text.is_empty() { "[missing option text]" } else { text }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, text: &str) -> SemanticLine {
        SemanticLine {
            id: id.to_string(),
            text: text.to_string(),
            source_anchor: json!({"nodeIds":[id]}),
            page_index: 0,
            order: 0,
            role: String::new(),
            bbox: None,
        }
    }

    #[test]
    fn detects_monotonic_option_runs_and_marks_empty_labels_incomplete() {
        let runs = detect_option_runs(&[
            line("a", "A First option"),
            line("b", "B Second option"),
            line("c", "C"),
            line("d", "D Fourth option"),
        ]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].options.len(), 4);
        // Bare "C" absorbs the next prose line "D Fourth option" only when that
        // line is not itself an option label. Here D is a label, so C stays empty
        // and the run remains incomplete.
        assert!(runs[0].incomplete);
    }

    #[test]
    fn split_label_and_text_lines_form_complete_option_run() {
        let runs = detect_option_runs(&[
            line("a", "A"),
            line("a-text", "changing the bed linen"),
            line("b", "B"),
            line("b-text", "washing the windows"),
            line("c", "C"),
            line("c-text", "cleaning the fridge"),
        ]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].options.len(), 3);
        assert!(!runs[0].incomplete);
        assert_eq!(runs[0].options[0].text, "changing the bed linen");
        assert_eq!(runs[0].options[2].label, "C");
    }

    #[test]
    fn ordinary_paragraph_a_is_not_an_option_without_sequence_evidence() {
        let runs = detect_option_runs(&[line("a", "A study of western celebrity")]);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].incomplete);
    }

    #[test]
    fn missing_middle_label_keeps_the_whole_run_incomplete() {
        let runs = detect_option_runs(&[
            line("a", "A First option"),
            line("b", "B Second option"),
            line("d", "D Fourth option"),
        ]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].options.len(), 3);
        assert!(runs[0].incomplete);
        assert!(!run_matches_alphabet(&runs[0], Some("A-D")));
    }

    #[test]
    fn detects_lowercase_roman_heading_bank_without_promoting_words() {
        let runs = detect_option_runs(&[
            line("title", "Questions about celebrity"),
            line("i", "i First heading"),
            line("ii", "ii Second heading"),
            line("iii", "iii Third heading"),
            line("iv", "iv Fourth heading"),
        ]);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0]
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            vec!["i", "ii", "iii", "iv"]
        );
        assert!(!runs[0].incomplete);
    }
}
