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

/// Whether a line opens with a `Question(s) <n>` heading. Accepts the compact
/// `Questions1-4` form that PDFs without space glyphs produce, but never a
/// longer word such as `Questionnaire`.
pub(crate) fn starts_with_question_heading(text: &str) -> bool {
    let lower = text
        .trim_start()
        .trim_start_matches('#')
        .trim_start()
        .to_ascii_lowercase();
    let rest = lower
        .strip_prefix("questions")
        .or_else(|| lower.strip_prefix("question"));
    rest.and_then(|rest| rest.chars().next())
        .is_some_and(|ch| ch.is_whitespace() || ch.is_ascii_digit())
}

/// Known IELTS instruction phrases, in canonical lowercase spacing.
fn instruction_cue_lexicon() -> &'static [String] {
    static LEXICON: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    LEXICON.get_or_init(|| {
        let counts = ["one", "two", "three", "four", "five", "six"];
        let mut phrases = Vec::new();
        for count in counts {
            phrases.push(format!("no more than {count} words"));
            phrases.push(format!("no more than {count} word"));
            phrases.push(format!("choose {count} correct answers"));
            phrases.push(format!("choose {count} correct letters"));
            phrases.push(format!("choose {count} answers"));
            phrases.push(format!("choose {count} letters"));
        }
        for container in [
            "form",
            "notes",
            "table",
            "summary",
            "sentences",
            "flow-chart",
            "diagram",
            "map",
            "plan",
        ] {
            phrases.push(format!("complete the {container} below"));
            phrases.push(format!("complete the {container}"));
        }
        for phrase in [
            "one word only",
            "choose the correct letter",
            "choose the correct answer",
            "write the correct letter",
            "and/or a number",
        ] {
            phrases.push(phrase.to_string());
        }
        let key = |phrase: &String| phrase.chars().filter(|ch| !ch.is_whitespace()).count();
        phrases.sort_by(|left, right| key(right).cmp(&key(left)));
        phrases
    })
}

/// Match `phrase` at `start` ignoring whitespace in the text (so both
/// `completetheformbelow` and `an d/o r a num ber` match). A `/` in the phrase
/// optionally matches one of `/ | i l 1` (`ANDIORANUMBER` is a real text-layer
/// rendering of `AND/OR A NUMBER`). Returns the end index on a match.
fn match_spacing_insensitive(text: &[char], start: usize, phrase: &str) -> Option<usize> {
    let mut index = start;
    let mut first = true;
    for expected in phrase.chars().filter(|ch| !ch.is_whitespace()) {
        if !first {
            while text.get(index).is_some_and(|ch| ch.is_whitespace()) {
                index += 1;
            }
        }
        first = false;
        if expected == '/' {
            if text
                .get(index)
                .is_some_and(|ch| matches!(ch, '/' | '|' | 'i' | 'l' | '1'))
            {
                index += 1;
            }
            continue;
        }
        if text.get(index) != Some(&expected) {
            return None;
        }
        index += 1;
    }
    Some(index)
}

/// Derived grammar view of instruction text for cue matching: ASCII-lowercased,
/// with known instruction phrases re-spaced to their canonical form when the
/// source lost (or split) the word spaces. Text whose phrases are already
/// canonically spaced is returned unchanged apart from case. The source text
/// and its evidence are never rewritten; only cue matching reads this view.
pub(crate) fn instruction_cue_text(text: &str) -> String {
    let lower = super::instruction_zone::normalize_instruction_text(text).to_ascii_lowercase();
    let chars = lower.chars().collect::<Vec<_>>();
    let lexicon = instruction_cue_lexicon();
    let mut output = String::with_capacity(lower.len() + 16);
    let mut index = 0;
    while index < chars.len() {
        let replacement = lexicon.iter().find_map(|phrase| {
            let end = match_spacing_insensitive(&chars, index, phrase)?;
            let raw = chars[index..end].iter().collect::<String>();
            (raw != *phrase).then_some((phrase, end))
        });
        match replacement {
            Some((phrase, end)) => {
                if output
                    .chars()
                    .last()
                    .is_some_and(|ch| ch.is_ascii_alphanumeric())
                {
                    output.push(' ');
                }
                output.push_str(phrase);
                if chars.get(end).is_some_and(|ch| ch.is_ascii_alphanumeric()) {
                    output.push(' ');
                }
                index = end;
            }
            None => {
                output.push(chars[index]);
                index += 1;
            }
        }
    }
    output
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
                    // `questions1-10` is a real PDF text-layer output.  A digit
                    // immediately after the keyword is therefore valid; letters
                    // and `_` still mean this is part of another word.
                    .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_');
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

    #[test]
    fn parses_compact_questions_without_a_space_before_the_number() {
        assert_eq!(
            parse_question_expression("questions1-10"),
            Some(QuestionNumberExpressionV2::Range { start: 1, end: 10 })
        );
    }

    #[test]
    fn question_heading_gate_accepts_compact_and_spaced_forms_only() {
        for heading in [
            "Questions 1-4",
            "Questions1-4",
            "questions1-10",
            "  QUESTIONS 17-20",
            "Question 5",
            "Question5",
        ] {
            assert!(starts_with_question_heading(heading), "{heading:?}");
        }
        for text in [
            "Questionnaire results were mixed",
            "questioning the survey",
            "The questions in this section are hard",
            "Section 1 explains the questions",
            "",
        ] {
            assert!(!starts_with_question_heading(text), "{text:?}");
        }
    }

    #[test]
    fn instruction_cue_text_respaces_compact_instruction_phrases() {
        for (compact, spaced) in [
            ("Completetheformbelow", "complete the form below"),
            (
                "WriteNOMORETHANTWOWORDSforeachanswer.",
                "write no more than two words foreachanswer.",
            ),
            (
                "ChooseFOURcorrectanswers,A-F",
                "choose four correct answers,a-f",
            ),
            (
                "WriteNOMORETHANTWOWORDSANDIORANUMBER",
                "write no more than two words and/or a number",
            ),
            (
                "Write NO MORE THAN TWO WORDS AN D/O R A NUM BER fo r each answer.",
                "write no more than two words and/or a number fo r each answer.",
            ),
        ] {
            assert_eq!(instruction_cue_text(compact), spaced, "{compact:?}");
        }
    }

    #[test]
    fn instruction_cue_text_leaves_spaced_text_unchanged_apart_from_case() {
        for text in [
            "Questions 1-3 Do the following statements agree with Reading Passage 1?",
            "Complete the summary using the list of words and phrases, A-H, below.",
            "Choose the correct letter, A, B, C or D. Write NO MORE THAN TWO WORDS AND/OR A NUMBER.",
            "The committee could choose four new members for the section next year.",
        ] {
            assert_eq!(instruction_cue_text(text), text.to_ascii_lowercase(), "{text:?}");
        }
    }
}
