use serde_json::Value;

use super::instruction_zone::{normalize_instruction_text, SemanticLine};
use super::option_run::{detect_option_runs, option_run_value, OptionItemCandidate, OptionRun};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OptionBankCandidate {
    pub title: Option<String>,
    pub run: OptionRun,
    pub allow_reuse: bool,
    pub scope: String,
}

pub(crate) fn detect_option_bank(
    lines: &[SemanticLine],
    instruction_text: &str,
    allow_reuse: Option<bool>,
    matching_semantics: bool,
) -> Option<OptionBankCandidate> {
    let lower = normalize_instruction_text(instruction_text).to_ascii_lowercase();
    let explicit_title = [
        "list of headings",
        "list of people",
        "list of features",
        "list of categories",
        "list of options",
        "choose from the box",
    ]
    .iter()
    .find(|marker| lower.contains(**marker))
    .map(|marker| (*marker).to_string());
    if explicit_title.is_none() && !matching_semantics {
        return None;
    }
    let runs = detect_option_runs(lines);
    let run = runs
        .into_iter()
        .filter(|run| run.options.len() >= 2)
        .find(|run| !run.incomplete);
    run.map(|run| OptionBankCandidate {
        title: explicit_title,
        run,
        allow_reuse: allow_reuse.unwrap_or(false),
        scope: "task".to_string(),
    })
}

/// Recover a completion word/phrase bank whose rows are often emitted in
/// column-major order.  Unlike ordinary choice runs, a completion bank is
/// accepted only when the instruction declares an A-terminal alphabet and
/// every declared label has non-empty source text.
pub(crate) fn detect_completion_option_bank(
    lines: &[SemanticLine],
    instruction_text: &str,
    allow_reuse: Option<bool>,
    expected_slot_count: usize,
) -> Option<OptionBankCandidate> {
    let all_text = std::iter::once(instruction_text.to_string())
        .chain(lines.iter().map(|line| line.text.clone()))
        .collect::<Vec<_>>()
        .join(" ");
    let lower = normalize_instruction_text(&all_text).to_ascii_lowercase();
    let has_bank_cue = [
        "list of words",
        "list of options",
        "list of phrases",
        "list of endings",
        "using the list",
        "using the words",
        "using the phrases",
        "from the list",
        "from the box",
        "in the box below",
        "in the box above",
        "options below",
        "words below",
        "phrases below",
        "endings below",
    ]
    .iter()
    .any(|cue| lower.contains(cue));
    if !has_bank_cue {
        return None;
    }
    let expected = declared_letter_labels(&all_text);
    if expected.len() < 3 {
        return None;
    }
    let cue_index = lines
        .iter()
        .position(|line| {
            let text = normalize_instruction_text(&line.text).to_ascii_lowercase();
            completion_bank_cue(&text)
        })
        .unwrap_or(0);
    let mut by_label = std::collections::BTreeMap::<String, OptionItemCandidate>::new();
    for line in lines.iter().skip(cue_index + 1) {
        for (label, text) in inline_completion_options(&line.text) {
            if expected.iter().any(|item| item == &label)
                && !text.trim().is_empty()
                && !by_label.contains_key(&label)
            {
                by_label.insert(
                    label.clone(),
                    OptionItemCandidate {
                        label,
                        text,
                        line_id: line.id.clone(),
                        source_anchor: line.source_anchor.clone(),
                    },
                );
            }
        }
    }
    if expected.iter().any(|label| !by_label.contains_key(label)) {
        return None;
    }
    let has_structural_heading = lines
        .iter()
        .any(|line| is_structural_bank_heading(&normalize_instruction_text(&line.text)));
    let distinct_label_rows = by_label
        .values()
        .map(|option| option.line_id.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    // Without a visible `List of ...` heading, require one source row per
    // declared label.  This keeps a prose line such as `A study compared B
    // with C` from becoming a selectable bank merely because the instruction
    // mentions a list and an A-C range.
    if !has_structural_heading && distinct_label_rows < expected.len() {
        return None;
    }
    let options = expected
        .iter()
        .filter_map(|label| by_label.remove(label))
        .collect::<Vec<_>>();
    let source_anchors = options
        .iter()
        .map(|option| option.source_anchor.clone())
        .collect::<Vec<_>>();
    let first_option_index = options
        .iter()
        .filter_map(|option| lines.iter().position(|line| line.id == option.line_id))
        .min()
        .unwrap_or(lines.len());
    let title = lines
        .iter()
        .take(first_option_index.saturating_add(1))
        .rev()
        .map(|line| normalize_instruction_text(&line.text))
        .find(|text| is_structural_bank_heading(text));
    // A closed bank with fewer choices than scoring slots cannot satisfy the
    // task without reuse. Treat that cardinality evidence as authoritative
    // even when the PDF omitted the usual "may be used more than once" line.
    // This keeps the runtime interaction usable for real IELTS summary/note
    // tasks while still preserving an explicit true/false policy whenever the
    // source has enough choices to make it meaningful.
    let inferred_reuse = expected_slot_count > options.len();
    Some(OptionBankCandidate {
        title,
        run: OptionRun {
            options,
            source_anchors,
            incomplete: false,
        },
        allow_reuse: allow_reuse.unwrap_or(false) || inferred_reuse,
        scope: "task".to_string(),
    })
}

fn completion_bank_cue(text: &str) -> bool {
    [
        "list of words",
        "list of options",
        "list of phrases",
        "list of endings",
        "using the list",
        "using the words",
        "using the phrases",
        "from the list",
        "from the box",
        "in the box below",
        "in the box above",
        "options below",
        "words below",
        "phrases below",
        "endings below",
    ]
    .iter()
    .any(|cue| text.contains(cue))
}

fn is_structural_bank_heading(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("list of ") else {
        return false;
    };
    !rest.trim().is_empty()
        && text.split_whitespace().count() <= 8
        && !text.ends_with(['.', '?', '!', ';', ':'])
}

fn declared_letter_labels(text: &str) -> Vec<String> {
    let normalized = normalize_instruction_text(text).to_ascii_uppercase();
    let compact = normalized
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let terminal = ('C'..='N')
        .rev()
        .find(|label| compact.contains(&format!("A-{label}")))
        .or_else(|| {
            let tokens = normalized
                .split_whitespace()
                .map(|token| token.trim_matches(|ch: char| !ch.is_ascii_alphabetic()))
                .collect::<Vec<_>>();
            let mut expected = 'A';
            let mut count = 0usize;
            for token in tokens {
                if token.len() != 1 || !token.chars().all(|ch| ch.is_ascii_uppercase()) {
                    continue;
                }
                let label = token.chars().next()?;
                if label == expected {
                    count += 1;
                    expected = ((expected as u8).saturating_add(1)) as char;
                } else if count >= 3 {
                    break;
                }
            }
            (count >= 3).then_some(((expected as u8).saturating_sub(1)) as char)
        });
    terminal
        .map(|terminal| ('A'..=terminal).map(|label| label.to_string()).collect())
        .unwrap_or_default()
}

fn inline_completion_options(text: &str) -> Vec<(String, String)> {
    let normalized = normalize_instruction_text(text);
    let tokens = normalized
        .split_whitespace()
        .map(|token| token.trim_matches(|ch: char| !ch.is_ascii_alphabetic()))
        .collect::<Vec<_>>();
    let marker_indices = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            (token.len() == 1 && token.chars().all(|ch| ch.is_ascii_uppercase())).then_some(index)
        })
        .collect::<Vec<_>>();
    // A completion bank row is a labelled source row, not arbitrary prose
    // containing the letters A/B/C.  Requiring the first token to be a label
    // prevents a normal sentence such as `A study compared B with C` from
    // being promoted when the instruction happens to mention a word list.
    if marker_indices.first().copied() != Some(0) {
        return Vec::new();
    }
    marker_indices
        .iter()
        .enumerate()
        .filter_map(|(position, index)| {
            let label = tokens[*index].to_string();
            let end = marker_indices
                .get(position + 1)
                .copied()
                .unwrap_or(tokens.len());
            let text = tokens[*index + 1..end].join(" ");
            (!text.is_empty()).then_some((label, text))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(id: &str, text: &str) -> SemanticLine {
        SemanticLine {
            id: id.to_string(),
            text: text.to_string(),
            source_anchor: json!({
                "sourceFileId":"source-1",
                "pageIndex":0,
                "nodeIds":[id],
                "extractionMode":"pdf_native",
                "sourceHash":"a".repeat(64)
            }),
            page_index: 0,
            order: 0,
            role: String::new(),
            bbox: None,
        }
    }

    #[test]
    fn ordinary_choice_options_are_not_promoted_to_a_shared_bank() {
        let lines = [
            line("a", "A First answer"),
            line("b", "B Second answer"),
            line("c", "C Third answer"),
            line("d", "D Fourth answer"),
        ];
        assert!(
            detect_option_bank(&lines, "Choose the correct letter, A-D.", None, false).is_none()
        );
        assert!(detect_option_bank(&lines, "List of People", Some(true), true).is_some());
    }

    #[test]
    fn absent_bank_title_is_omitted_instead_of_serialized_as_null() {
        let lines = [
            line("a", "A First answer"),
            line("b", "B Second answer"),
            line("c", "C Third answer"),
        ];
        let candidate = detect_option_bank(
            &lines,
            "Match each statement with the correct option.",
            Some(false),
            true,
        )
        .expect("matching bank without a title");
        assert!(candidate.title.is_none());

        let value = option_bank_value(&candidate, "task-1");
        assert!(value.get("title").is_none());
        serde_json::from_value::<crate::schema::ielts_authoring_v2::OptionBankV2>(value)
            .expect("schema-compatible option bank");
    }

    #[test]
    fn list_of_headings_keeps_the_complete_lowercase_roman_bank() {
        let lines = [
            line("i", "i First heading"),
            line("ii", "ii Second heading"),
            line("iii", "iii Third heading"),
            line("iv", "iv Fourth heading"),
        ];
        let bank = detect_option_bank(&lines, "List of Headings", Some(false), true)
            .expect("roman heading bank");
        assert_eq!(
            bank.run
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            vec!["i", "ii", "iii", "iv"]
        );
        assert!(!bank.allow_reuse);
    }

    #[test]
    fn completion_bank_recovers_column_major_a_to_i_rows() {
        let lines = [
            line(
                "instruction",
                "Complete the summary using a word A-I from the box.",
            ),
            line("q36", "One method would 36 ______ asteroids at the planet."),
            line("bank-title", "List of words"),
            line("bank-a", "A cover"),
            line("bank-g", "G power"),
            line("bank-d", "D increase"),
            line("bank-b", "B create"),
            line("bank-e", "E land"),
            line("bank-h", "H rise"),
            line("bank-c", "C hit"),
            line("bank-f", "F drive"),
            line("bank-i", "I shoot"),
        ];
        let bank = detect_completion_option_bank(
            &lines,
            "Questions 36-40 Complete the summary.",
            Some(false),
            2,
        )
        .expect("closed completion bank");
        assert_eq!(
            bank.run
                .options
                .iter()
                .map(|option| (option.label.as_str(), option.text.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("A", "cover"),
                ("B", "create"),
                ("C", "hit"),
                ("D", "increase"),
                ("E", "land"),
                ("F", "drive"),
                ("G", "power"),
                ("H", "rise"),
                ("I", "shoot"),
            ]
        );
        assert_eq!(bank.title, Some("List of words".to_string()));
    }

    #[test]
    fn completion_bank_does_not_promote_letters_inside_ordinary_prose() {
        let lines = [
            line(
                "instruction",
                "Complete the summary using words from the list A-C.",
            ),
            line(
                "prose",
                "A study compared B with C before the final review.",
            ),
        ];
        assert!(detect_completion_option_bank(
            &lines,
            "Questions 1-2 Complete the summary.",
            Some(false),
            2,
        )
        .is_none());
    }

    #[test]
    fn completion_bank_forces_reuse_when_slots_outnumber_choices() {
        let lines = [
            line("heading", "List of words"),
            line("a", "A north"),
            line("b", "B south"),
            line("c", "C east"),
        ];
        let bank = detect_completion_option_bank(
            &lines,
            "Complete the notes using a word A-C from the box.",
            Some(false),
            4,
        )
        .expect("closed completion bank");
        assert!(bank.allow_reuse);
    }
}

pub(crate) fn option_bank_value(candidate: &OptionBankCandidate, task_id: &str) -> Value {
    let mut value = serde_json::json!({
        "optionBankId": format!("{task_id}-option-bank"),
        "scope": "task_group",
        "options": option_run_value(&candidate.run, &format!("{task_id}-option")),
        "allowReuse": candidate.allow_reuse,
        "sourceAnchors": candidate.run.source_anchors
    });
    if let Some(title) = candidate.title.as_ref() {
        value["title"] = serde_json::json!([text_node(
            &format!("{task_id}-option-bank-title"),
            title,
            candidate.run.source_anchors.first().cloned().unwrap_or_else(|| serde_json::json!({"sourceFileId":"unknown-source","pageIndex":-1,"nodeIds":[],"extractionMode":"manual","sourceHash":"unknown"}))
        )]);
    }
    value
}

fn text_node(id: &str, text: &str, source_anchor: Value) -> Value {
    serde_json::json!({
        "type": "text",
        "id": id,
        "sourceAnchors": [source_anchor],
        "provenanceStatus": "source",
        "text": text
    })
}
