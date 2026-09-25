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
        let leading = parse_leading_question_number(&text);
        let trailing = parse_trailing_question_number(&text);
        // A table row can print its question number **after** the item text. When
        // the line opens with an ordinal (`18th-century paintings17`) the leading
        // parse reads the item name's own digits as a question number, so the
        // trailing one — the row's real number — has to win.
        let parsed = match (leading, trailing) {
            (Some((leading_number, _)), Some((trailing_number, score)))
                if opens_with_an_ordinal(&text) && leading_number != trailing_number =>
            {
                Some((trailing_number, score))
            }
            (Some(parsed), _) => Some(parsed),
            (None, Some(parsed)) => Some(parsed),
            (None, None) => None,
        };
        let Some((number, leading_score)) = parsed else {
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

/// `18th-century paintings` — the opening digits are an ordinal, not a question
/// number. Only consulted when the same line also carries a trailing number, so
/// a genuine question line is never reinterpreted.
fn opens_with_an_ordinal(text: &str) -> bool {
    let trimmed = text.trim_start();
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return false;
    }
    matches!(
        trimmed[digits..].get(..2).map(str::to_ascii_lowercase).as_deref(),
        Some("th") | Some("st") | Some("nd") | Some("rd")
    )
}

/// A table row can print its question number **after** the item text
/// (`Farnley collection 18`, `PEST21`). The leading parse cannot see it, so the
/// row would produce no anchor at all and its item text would be attributed to
/// the *next* question instead — leaving every row in the block with an empty
/// prompt, which the quality gate then reads as `PROMPT_BOUNDARY_AMBIGUOUS`.
///
/// Range headings (`Questions 11-16`) and paper banners (`SECTION2`) end in
/// digits too; those digits are not a row's number.
fn parse_trailing_question_number(text: &str) -> Option<(u32, f64)> {
    let trimmed = text.trim_end();
    let mut start = trimmed.len();
    for (index, ch) in trimmed.char_indices().rev() {
        if ch.is_ascii_digit() {
            start = index;
        } else {
            break;
        }
    }
    if start == trimmed.len() || trimmed.len() - start > 3 {
        return None;
    }
    let number = trimmed[start..].parse::<u32>().ok()?;
    let before = trimmed[..start].trim_end();
    // A bare number is the leading parse's job; it is also how a completion row
    // says "my blank sits on the next line".
    if before.is_empty() || before.ends_with('-') || before.ends_with('\u{2013}') {
        return None;
    }
    let last_word = before
        .rsplit(|ch: char| !ch.is_alphanumeric())
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if last_word.is_empty() || TRAILING_NUMBER_STOP_WORDS.contains(&last_word.as_str()) {
        return None;
    }
    Some((number, 0.68))
}

/// Words that end a paper banner or a range heading rather than an item name.
const TRAILING_NUMBER_STOP_WORDS: &[&str] = &[
    "section",
    "part",
    "unit",
    "module",
    "passage",
    "test",
    "page",
    "question",
    "questions",
];

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
            line(
                "stem",
                "Which extra service does the agency agree to provide?",
            ),
            line("a", "A changing the bed linen"),
        ];
        let anchors = detect_question_anchors(&lines, &[5, 6, 7]);
        assert_eq!(anchors.len(), 1);
        assert_eq!(anchors[0].question_number, 5);
        assert_eq!(anchors[0].line_id, "n5");
    }

    /// The private listening paper prints its feature-match rows as a table whose
    /// question number comes **after** the item name, and one row glues the two
    /// together without a space. Every row must still become an anchor for its own
    /// number: without anchors the row prompt is empty and the group is blocked
    /// with `PROMPT_BOUNDARY_AMBIGUOUS`.
    #[test]
    fn a_table_row_anchors_on_its_trailing_question_number() {
        let lines = vec![
            line("head", "Information"),
            line("r17", "18th-century paintings17"),
            line("r18", "Farnley collection 18"),
            line("r19", "Kitchen appliances 19"),
            line("r20", "Fashion gallery 20"),
        ];
        let anchors = detect_question_anchors(&lines, &[17, 18, 19, 20]);
        let observed = anchors
            .iter()
            .map(|anchor| (anchor.question_number, anchor.line_id.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            observed,
            vec![
                (17, "r17".to_string()),
                (18, "r18".to_string()),
                (19, "r19".to_string()),
                (20, "r20".to_string()),
            ],
            "`18th-century` is an ordinal, not question 18; each row's own number is the trailing one"
        );
    }

    /// Range headings and paper banners end in digits too, and those digits must
    /// never become a row anchor.
    #[test]
    fn range_headings_and_banners_are_not_row_anchors() {
        let lines = vec![
            line("section", "SECTION2"),
            line("range", "Questions 11-16"),
            line("read", "Read the text and answer questions 11-20"),
            line("en_dash", "Questions 21\u{2013}25"),
        ];
        let anchors = detect_question_anchors(&lines, &[1, 2, 11, 16, 20, 21, 25]);
        assert!(
            anchors.is_empty(),
            "none of these lines carries a row number: {anchors:?}"
        );
    }

    /// A line that already parses as a leading number keeps that reading even when
    /// it ends in digits as well.
    #[test]
    fn a_leading_question_number_still_wins() {
        let lines = vec![line("q", "3 Which century saw the most growth in 1900?")];
        let anchors = detect_question_anchors(&lines, &[3, 1900]);
        assert_eq!(anchors.len(), 1);
        assert_eq!(anchors[0].question_number, 3);
    }
}
