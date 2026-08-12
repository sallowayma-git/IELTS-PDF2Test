use serde_json::Value;

use super::instruction_zone::{normalize_instruction_text, SemanticLine};
use super::option_run::{detect_option_runs, option_run_value, OptionRun};

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
