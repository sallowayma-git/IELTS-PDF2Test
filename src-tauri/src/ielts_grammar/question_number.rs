use crate::schema::ielts_authoring_v2::{QuestionNumberExpressionV2, QuestionNumberValueV2};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedQuestionExpression {
    pub expression: QuestionNumberExpressionV2,
    pub numbers: Vec<u32>,
    pub raw: String,
    pub normalized: String,
}

#[derive(Debug, Clone, PartialEq)]
enum ExpressionItem {
    Number(u32),
    Range(u32, u32),
}

pub(crate) fn parse_question_expression(text: &str) -> Option<QuestionNumberExpressionV2> {
    parse_question_expression_detailed(text).map(|result| result.expression)
}

pub(crate) fn parse_question_expression_detailed(text: &str) -> Option<ParsedQuestionExpression> {
    let normalized = normalize_question_text(text);
    let (word_start, word_end) = find_question_word(&normalized)?;
    if has_non_question_context(&normalized[..word_start]) {
        return None;
    }

    let raw_tail = &normalized[word_end..];
    let items = parse_expression_items(raw_tail)?;
    let numbers = expand_items(&items)?;
    if numbers.is_empty() || numbers.windows(2).any(|pair| pair[0] >= pair[1]) {
        return None;
    }
    let expression = compact_items(items);
    let raw = raw_tail
        .split(|ch: char| matches!(ch, '.' | ':' | ';'))
        .next()
        .unwrap_or(raw_tail)
        .trim()
        .to_string();
    Some(ParsedQuestionExpression {
        expression,
        numbers,
        raw,
        normalized,
    })
}

pub(crate) fn expand_expression(expression: &QuestionNumberExpressionV2) -> Vec<u32> {
    match expression {
        QuestionNumberExpressionV2::Range { start, end } => (*start..=*end).collect(),
        QuestionNumberExpressionV2::Set { values } => values.clone(),
        QuestionNumberExpressionV2::Mixed { values } => values
            .iter()
            .flat_map(|value| match value {
                QuestionNumberValueV2::Number(number) => vec![*number],
                QuestionNumberValueV2::Range { start, end } => (*start..=*end).collect(),
            })
            .collect(),
    }
}

pub(crate) fn normalize_question_text(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            '\u{00a0}' | '\u{2007}' | '\u{202f}' => ' ',
            _ => ch,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn question_expression_end(text: &str) -> Option<usize> {
    let normalized = normalize_question_text(text);
    let (_, mut cursor) = find_question_word(&normalized)?;
    loop {
        let number = parse_number_at(&normalized, cursor)?;
        cursor = number.1;
        while cursor < normalized.len()
            && normalized[cursor..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_whitespace())
        {
            cursor += normalized[cursor..].chars().next()?.len_utf8();
        }
        if cursor < normalized.len() && normalized[cursor..].starts_with('-') {
            cursor += 1;
            let end = parse_number_at(&normalized, cursor)?;
            cursor = end.1;
        } else if normalized[cursor..].to_ascii_lowercase().starts_with("to ") {
            cursor += 2;
            let end = parse_number_at(&normalized, cursor)?;
            cursor = end.1;
        }
        let before_separator = cursor;
        while cursor < normalized.len()
            && normalized[cursor..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_whitespace())
        {
            cursor += normalized[cursor..].chars().next()?.len_utf8();
        }
        if cursor < normalized.len() && normalized[cursor..].starts_with(',') {
            cursor += 1;
            continue;
        }
        if normalized[cursor..]
            .to_ascii_lowercase()
            .starts_with("and ")
        {
            cursor += 3;
            continue;
        }
        cursor = before_separator;
        break;
    }
    Some(cursor)
}

fn find_question_word(text: &str) -> Option<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    for word in ["questions", "question"] {
        let mut offset = 0usize;
        while let Some(relative) = lower[offset..].find(word) {
            let start = offset + relative;
            let end = start + word.len();
            let before_ok = start == 0
                || !lower[..start]
                    .chars()
                    .last()
                    .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
            let after_ok = end == lower.len()
                || !lower[end..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
            if before_ok && after_ok {
                return Some((start, end));
            }
            offset = end;
        }
    }
    None
}

fn has_non_question_context(prefix: &str) -> bool {
    let lower = prefix.to_ascii_lowercase();
    ["in boxes", "box", "passage", "part", "section", "page"]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn parse_expression_items(text: &str) -> Option<Vec<ExpressionItem>> {
    let mut cursor = 0usize;
    let mut items = Vec::new();
    let mut expect_number = true;
    while cursor < text.len() {
        while cursor < text.len()
            && text[cursor..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_whitespace() || (expect_number && ch == ','))
        {
            cursor += text[cursor..].chars().next()?.len_utf8();
        }
        if cursor >= text.len() {
            break;
        }
        if !expect_number {
            while cursor < text.len()
                && text[cursor..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_whitespace())
            {
                cursor += text[cursor..].chars().next()?.len_utf8();
            }
            if cursor < text.len() && text[cursor..].starts_with(',') {
                cursor += 1;
                expect_number = true;
                continue;
            }
            let remaining = text[cursor..].to_ascii_lowercase();
            if remaining.starts_with("and ") || remaining == "and" {
                cursor += 3;
                expect_number = true;
                continue;
            }
            break;
        }
        let start = parse_number_at(text, cursor)?;
        cursor = start.1;
        while cursor < text.len()
            && text[cursor..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_whitespace())
        {
            cursor += text[cursor..].chars().next()?.len_utf8();
        }
        if cursor < text.len() && text[cursor..].starts_with('-') {
            cursor += 1;
            while cursor < text.len()
                && text[cursor..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_whitespace())
            {
                cursor += text[cursor..].chars().next()?.len_utf8();
            }
            let end = parse_number_at(text, cursor)?;
            if end.0 <= start.0 {
                return None;
            }
            cursor = end.1;
            items.push(ExpressionItem::Range(start.0, end.0));
        } else if text[cursor..].to_ascii_lowercase().starts_with("to ") {
            cursor += 2;
            let end = parse_number_at(text, cursor)?;
            if end.0 <= start.0 {
                return None;
            }
            cursor = end.1;
            items.push(ExpressionItem::Range(start.0, end.0));
        } else {
            items.push(ExpressionItem::Number(start.0));
        }
        expect_number = false;
    }
    (!items.is_empty()).then_some(items)
}

fn parse_number_at(text: &str, mut cursor: usize) -> Option<(u32, usize)> {
    while cursor < text.len()
        && text[cursor..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_whitespace())
    {
        cursor += text[cursor..].chars().next()?.len_utf8();
    }
    let start = cursor;
    while cursor < text.len()
        && text[cursor..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_digit())
    {
        cursor += text[cursor..].chars().next()?.len_utf8();
    }
    (cursor > start)
        .then(|| text[start..cursor].parse::<u32>().ok())
        .flatten()
        .map(|number| (number, cursor))
}

fn expand_items(items: &[ExpressionItem]) -> Option<Vec<u32>> {
    let mut numbers = Vec::new();
    for item in items {
        match item {
            ExpressionItem::Number(number) => numbers.push(*number),
            ExpressionItem::Range(start, end) => {
                if *end - *start > 500 {
                    return None;
                }
                numbers.extend(*start..=*end);
            }
        }
    }
    Some(numbers)
}

fn compact_items(items: Vec<ExpressionItem>) -> QuestionNumberExpressionV2 {
    if items.len() == 1 {
        if let ExpressionItem::Range(start, end) = items[0] {
            return QuestionNumberExpressionV2::Range { start, end };
        }
    }
    if items
        .iter()
        .all(|item| matches!(item, ExpressionItem::Number(_)))
    {
        return QuestionNumberExpressionV2::Set {
            values: items
                .into_iter()
                .map(|item| match item {
                    ExpressionItem::Number(number) => number,
                    ExpressionItem::Range(_, _) => unreachable!(),
                })
                .collect(),
        };
    }
    QuestionNumberExpressionV2::Mixed {
        values: items
            .into_iter()
            .map(|item| match item {
                ExpressionItem::Number(number) => QuestionNumberValueV2::Number(number),
                ExpressionItem::Range(start, end) => QuestionNumberValueV2::Range { start, end },
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_range_dash_and_to_variants() {
        for text in ["Questions 1–6", "Questions 1 - 6", "Questions 1 to 6"] {
            assert_eq!(
                parse_question_expression(text),
                Some(QuestionNumberExpressionV2::Range { start: 1, end: 6 })
            );
        }
    }

    #[test]
    fn parses_sets_and_mixed_ranges_without_expanding_the_expression() {
        assert_eq!(
            parse_question_expression("Questions 14 and 15"),
            Some(QuestionNumberExpressionV2::Set {
                values: vec![14, 15]
            })
        );
        assert_eq!(
            parse_question_expression("Questions 27–30 and 36–40"),
            Some(QuestionNumberExpressionV2::Mixed {
                values: vec![
                    QuestionNumberValueV2::Range { start: 27, end: 30 },
                    QuestionNumberValueV2::Range { start: 36, end: 40 },
                ]
            })
        );
        assert_eq!(
            parse_question_expression_detailed("Questions 11, 12 and 13")
                .unwrap()
                .numbers,
            vec![11, 12, 13]
        );
    }

    #[test]
    fn rejects_embedded_number_context_and_invalid_ranges() {
        assert!(parse_question_expression("In boxes 1–6 write answers").is_none());
        assert!(parse_question_expression("Reading Passage 1").is_none());
        assert!(parse_question_expression("Questions 6–1").is_none());
        assert!(parse_question_expression("Questions 1–1000").is_none());
    }

    #[test]
    fn keeps_heading_expression_separate_from_following_instruction() {
        let parsed = parse_question_expression_detailed(
            "Questions 1-3 Do the following statements agree with Reading Passage 1?",
        )
        .unwrap();
        assert_eq!(parsed.numbers, vec![1, 2, 3]);
    }
}
