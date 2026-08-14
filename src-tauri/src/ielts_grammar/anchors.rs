use serde_json::Value;

use super::instruction_zone::{normalize_instruction_text, SemanticLine};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QuestionAnchor {
    pub question_number: u32,
    pub line_index: usize,
    pub line_id: String,
    pub score: f64,
    pub source_anchor: Value,
}

pub(crate) fn detect_question_anchors(
    lines: &[SemanticLine],
    expected_numbers: &[u32],
) -> Vec<QuestionAnchor> {
    let mut anchors = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        let text = normalize_instruction_text(&line.text);
        let Some((number, leading_score)) = parse_leading_question_number(&text) else {
            continue;
        };
        if !expected_numbers.contains(&number) {
            continue;
        }
        let has_prompt = text
            .split_once(|ch: char| matches!(ch, '.' | ')' | ':' | '-'))
            .map(|(_, rest)| !rest.trim().is_empty())
            .unwrap_or(false);
        let score = (leading_score + if has_prompt { 0.1 } else { 0.0 }).min(1.0);
        anchors.push(QuestionAnchor {
            question_number: number,
            line_index,
            line_id: line.id.clone(),
            score,
            source_anchor: line.source_anchor.clone(),
        });
    }
    anchors.sort_by_key(|anchor| (anchor.question_number, anchor.line_index));
    anchors
}

pub(crate) fn anchor_coverage(anchors: &[QuestionAnchor], expected_numbers: &[u32]) -> f64 {
    if expected_numbers.is_empty() {
        return 0.0;
    }
    let found = expected_numbers
        .iter()
        .filter(|number| {
            anchors
                .iter()
                .any(|anchor| &anchor.question_number == *number)
        })
        .count();
    found as f64 / expected_numbers.len() as f64
}

fn parse_leading_question_number(text: &str) -> Option<(u32, f64)> {
    let trimmed = text.trim_start();
    let mut end = 0usize;
    for (index, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 || end > 3 {
        return None;
    }
    let number = trimmed[..end].parse::<u32>().ok()?;
    let remainder = trimmed[end..].trim_start();
    // Bare question numbers are first-class anchors. PDFs commonly emit:
    //   line: "5"
    //   line: "Which extra service does the agency agree to provide?"
    // Rejecting bare numbers forces empty prompts even when the stem is intact
    // on the next geometric line. Page numbers that collide with the expected
    // range are filtered later by expected_numbers + score ranking.
    if remainder.is_empty() {
        return Some((number, 0.62));
    }
    let punctuation = remainder.chars().next();
    let score = if matches!(punctuation, Some('.') | Some(')') | Some(':')) {
        0.9
    } else {
        0.72
    };
    Some((number, score))
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

    #[test]
    fn question_anchor_requires_declared_number() {
        let lines = vec![
            line("q1", "1. First statement"),
            // Bare page numbers outside the declared range stay ignored.
            line("page", "12"),
            line("q3", "3) Third statement"),
        ];
        let anchors = detect_question_anchors(&lines, &[1, 3]);
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchor_coverage(&anchors, &[1, 2, 3]), 2.0 / 3.0);
    }

    #[test]
    fn bare_declared_question_number_is_an_anchor() {
        let lines = vec![
            line("n5", "5"),
            line("stem", "Which extra service does the agency agree to provide?"),
            line("a", "A changing the bed linen"),
        ];
        let anchors = detect_question_anchors(&lines, &[5, 6, 7]);
        assert_eq!(anchors.len(), 1);
        assert_eq!(anchors[0].question_number, 5);
        assert_eq!(anchors[0].line_id, "n5");
    }
}
