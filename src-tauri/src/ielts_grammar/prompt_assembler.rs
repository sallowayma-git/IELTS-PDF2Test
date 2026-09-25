use serde_json::Value;

use super::anchors::QuestionAnchor;
use super::instruction_zone::{normalize_instruction_text, SemanticLine};
use super::option_run::detect_option_runs;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PromptResult {
    pub text: String,
    pub source_anchors: Vec<Value>,
    pub source_line_ids: Vec<String>,
    pub source_coverage: f64,
    pub boundary_ambiguous: bool,
}

pub(crate) fn assemble_prompt(
    candidate_prompt: Option<&str>,
    question_number: u32,
    question_anchor: Option<&QuestionAnchor>,
    next_question_anchor: Option<&QuestionAnchor>,
    lines: &[SemanticLine],
    fallback_anchor: Option<Value>,
) -> PromptResult {
    let candidate = candidate_prompt
        .map(normalize_instruction_text)
        .filter(|text| !text.is_empty() && !looks_like_placeholder(text));
    if let Some(text) = candidate {
        return PromptResult {
            text,
            source_anchors: question_anchor
                .map(|anchor| vec![anchor.source_anchor.clone()])
                .or_else(|| fallback_anchor.clone().map(|anchor| vec![anchor]))
                .unwrap_or_default(),
            source_line_ids: question_anchor
                .map(|anchor| vec![anchor.line_id.clone()])
                .unwrap_or_default(),
            source_coverage: if question_anchor.is_some() { 1.0 } else { 0.72 },
            boundary_ambiguous: false,
        };
    }

    let Some(anchor) = question_anchor else {
        return PromptResult {
            text: String::new(),
            source_anchors: fallback_anchor.into_iter().collect(),
            source_line_ids: Vec::new(),
            source_coverage: 0.0,
            boundary_ambiguous: true,
        };
    };
    let start = anchor.line_index;
    let end = next_question_anchor
        .map(|next| next.line_index)
        .unwrap_or(lines.len());
    let option_start = detect_option_runs(&lines[start..end])
        .first()
        .and_then(|run| run.options.first())
        .and_then(|option| {
            lines[start..end]
                .iter()
                .position(|line| line.id == option.line_id)
        })
        .map(|offset| start + offset)
        .unwrap_or(end);
    let mut parts = Vec::new();
    let mut anchors = Vec::new();
    let mut line_ids = Vec::new();
    for line in lines.iter().take(option_start).skip(start) {
        let mut text = normalize_instruction_text(&line.text);
        if line.id == anchor.line_id {
            text = strip_question_prefix(&text, question_number);
        }
        if text.is_empty() || is_instruction_line(&text) {
            continue;
        }
        parts.push(text);
        anchors.push(line.source_anchor.clone());
        line_ids.push(line.id.clone());
    }
    let source_coverage = if parts.is_empty() { 0.0 } else { 1.0 };
    PromptResult {
        text: parts.join(" "),
        source_anchors: anchors,
        source_line_ids: line_ids,
        source_coverage,
        boundary_ambiguous: next_question_anchor.is_none() && option_start == lines.len(),
    }
}

fn strip_question_prefix(text: &str, question_number: u32) -> String {
    let prefix = question_number.to_string();
    let trimmed = text.trim_start();
    if !trimmed.starts_with(&prefix) {
        // A table row prints its number **after** the item text
        // (`Farnley collection 18`), so the row's own text is everything before
        // it. A bare-number line must stay untouched: the empty prompt it
        // produces is the completion channel's signal to read the row below.
        if let Some(rest) = trimmed.strip_suffix(prefix.as_str()) {
            let rest = rest
                .trim_end_matches(|ch: char| matches!(ch, '.' | ')' | ':' | '-' | ' '))
                .trim();
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
        return trimmed.to_string();
    }
    trimmed[prefix.len()..]
        .trim_start_matches(|ch: char| matches!(ch, '.' | ')' | ':' | '-' | ' '))
        .trim()
        .to_string()
}

fn is_instruction_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    super::question_number::starts_with_question_heading(&lower)
        || lower.starts_with("choose ")
        || lower.starts_with("complete ")
        || lower.starts_with("do the following")
        || lower.starts_with("in boxes ")
        || lower.starts_with("write ")
}

fn looks_like_placeholder(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.is_empty()
        || lower.contains("source statement describes a synthetic test condition")
        || lower == "answer"
        || lower == "prompt"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, text: &str) -> SemanticLine {
        SemanticLine {
            id: id.to_string(),
            text: text.to_string(),
            source_anchor: serde_json::json!({"nodeIds":[id]}),
            page_index: 0,
            order: 0,
            role: String::new(),
            bbox: None,
        }
    }

    fn anchor(id: &str, index: usize, number: u32) -> QuestionAnchor {
        QuestionAnchor {
            question_number: number,
            line_index: index,
            line_id: id.to_string(),
            score: 0.9,
            source_anchor: serde_json::json!({"nodeIds":[id]}),
        }
    }

    #[test]
    fn prompt_assembler_stops_at_next_question_and_preserves_anchor() {
        let lines = vec![
            line("q1", "1. First statement"),
            line("q2", "2. Second statement"),
        ];
        let result = assemble_prompt(
            None,
            1,
            Some(&anchor("q1", 0, 1)),
            Some(&anchor("q2", 1, 2)),
            &lines,
            None,
        );
        assert_eq!(result.text, "First statement");
        assert_eq!(result.source_line_ids, vec!["q1"]);
    }

    /// A table row prints its number after the item name, so the row's prompt is
    /// the text **before** it. Reading the whole line as the prompt (or, worse,
    /// leaving it empty) is what blocks the private listening paper.
    #[test]
    fn prompt_drops_a_trailing_question_number() {
        let lines = vec![
            line("r17", "18th-century paintings17"),
            line("r18", "Farnley collection 18"),
        ];
        let seventeen = assemble_prompt(None, 17, Some(&anchor("r17", 0, 17)), Some(&anchor("r18", 1, 18)), &lines, None);
        assert_eq!(seventeen.text, "18th-century paintings");
        let eighteen = assemble_prompt(None, 18, Some(&anchor("r18", 1, 18)), None, &lines, None);
        assert_eq!(eighteen.text, "Farnley collection");
    }

    /// A bare-number anchor must still produce an empty prompt: the completion
    /// channel reads that as "the row below carries my text".
    #[test]
    fn a_bare_number_anchor_keeps_an_empty_prompt() {
        let lines = vec![
            line("n8", "8"),
            line("row", "Some kitchen appliance"),
        ];
        let result = assemble_prompt(None, 8, Some(&anchor("n8", 0, 8)), None, &lines, None);
        assert_eq!(result.text, "Some kitchen appliance");
    }
}
