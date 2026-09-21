//! Independent source-declared question-domain coverage.
//!
//! This is deliberately separate from task-group recognition.  The source text
//! declares the question domain; the canonical draft declares the questions we
//! will ship.  Comparing those two views is the only way to catch a question
//! that both local recognition and a cloud candidate silently omitted.

use super::question_number::parse_question_expression_detailed;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuestionCoverageStatus {
    Complete,
    Missing,
    Undetermined,
}

impl QuestionCoverageStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Missing => "missing",
            Self::Undetermined => "undetermined",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuestionCoverageAssessment {
    pub status: QuestionCoverageStatus,
    pub declared_question_numbers: Vec<u32>,
    pub canonical_question_numbers: Vec<u32>,
    pub missing_question_numbers: Vec<u32>,
    pub extra_question_numbers: Vec<u32>,
    pub declarations: Vec<String>,
    pub unparsed_declarations: Vec<String>,
    pub reason: Option<String>,
}

impl QuestionCoverageAssessment {
    pub(crate) fn as_value(&self) -> Value {
        let mut value = json!({
            "status": self.status.as_str(),
            "declaredQuestionNumbers": self.declared_question_numbers,
            "canonicalQuestionNumbers": self.canonical_question_numbers,
            "missingQuestionNumbers": self.missing_question_numbers,
            "extraQuestionNumbers": self.extra_question_numbers,
            "declarations": self.declarations,
            "unparsedDeclarations": self.unparsed_declarations,
            "reason": self.reason
        });
        if self.reason.is_none() {
            value
                .as_object_mut()
                .expect("question coverage report must be an object")
                .remove("reason");
        }
        value
    }
}

pub(crate) fn assess(
    authoring: &Value,
    physical_shadow: Option<&Value>,
) -> QuestionCoverageAssessment {
    let canonical_question_numbers = canonical_question_numbers(authoring);
    let Some(source_text) = source_text(physical_shadow) else {
        return undetermined(
            canonical_question_numbers.into_iter().collect(),
            "source_text_unavailable",
        );
    };

    let mut declarations = Vec::new();
    let mut unparsed_declarations = Vec::new();
    let mut declared = BTreeSet::new();

    for start in keyword_positions(&source_text, "questions") {
        let raw = declaration_preview(&source_text[start..]);
        declarations.push(raw.clone());
        match parse_numbers_after_marker(&source_text[start..], "questions") {
            Some(numbers) => declared.extend(numbers),
            None => unparsed_declarations.push(raw),
        }
    }

    // IELTS papers often declare the same domain as “Write your answers in
    // boxes 1-13”.  Only treat a box expression as a domain declaration when
    // its nearby context says this is an answer instruction; a random table
    // cell named “box 1” is not a paper-wide question domain.
    for marker in ["boxes", "box"] {
        for start in keyword_positions(&source_text, marker) {
            if !answer_box_context(&source_text, start) {
                continue;
            }
            let raw = declaration_preview(&source_text[start..]);
            declarations.push(raw.clone());
            match parse_numbers_after_marker(&source_text[start..], marker) {
                Some(numbers) => declared.extend(numbers),
                None => unparsed_declarations.push(raw),
            }
        }
    }

    declarations.sort();
    declarations.dedup();
    unparsed_declarations.sort();
    unparsed_declarations.dedup();

    let missing = declared
        .difference(&canonical_question_numbers)
        .copied()
        .collect::<Vec<_>>();
    let extra = canonical_question_numbers
        .difference(&declared)
        .copied()
        .collect::<Vec<_>>();
    let declared_question_numbers = declared.iter().copied().collect::<Vec<_>>();
    let canonical_question_numbers = canonical_question_numbers.into_iter().collect::<Vec<_>>();

    let (status, reason) = if declarations.is_empty() {
        (
            QuestionCoverageStatus::Undetermined,
            Some("question_declaration_missing"),
        )
    } else if !unparsed_declarations.is_empty() {
        (
            QuestionCoverageStatus::Undetermined,
            Some("question_declaration_unparsed"),
        )
    } else if !missing.is_empty() || !extra.is_empty() {
        (QuestionCoverageStatus::Missing, None)
    } else {
        (QuestionCoverageStatus::Complete, None)
    };

    QuestionCoverageAssessment {
        status,
        declared_question_numbers,
        canonical_question_numbers,
        missing_question_numbers: missing,
        extra_question_numbers: extra,
        declarations,
        unparsed_declarations,
        reason: reason.map(str::to_string),
    }
}

/// 原文件已在发布后删除时，用发布时冻结的「原文声明题号集合」核对当前稿。
///
/// 冻结集合为空（发布时就没能解析出声明）⇒ 仍是 Undetermined，与今天无 shadow 时
/// 的结论一致，不因为原文件被删而升级成 Complete。
pub(crate) fn assess_against_frozen_declaration(
    authoring: &Value,
    frozen_declared: &[u32],
    frozen_declarations: &[String],
) -> QuestionCoverageAssessment {
    let canonical = canonical_question_numbers(authoring);
    if frozen_declared.is_empty() {
        return undetermined(
            canonical.into_iter().collect(),
            "frozen_declaration_unavailable_source_purged",
        );
    }
    let declared = frozen_declared.iter().copied().collect::<BTreeSet<_>>();
    let missing = declared.difference(&canonical).copied().collect::<Vec<_>>();
    let extra = canonical.difference(&declared).copied().collect::<Vec<_>>();
    let status = if missing.is_empty() && extra.is_empty() {
        QuestionCoverageStatus::Complete
    } else {
        QuestionCoverageStatus::Missing
    };
    QuestionCoverageAssessment {
        status,
        declared_question_numbers: declared.into_iter().collect(),
        canonical_question_numbers: canonical.into_iter().collect(),
        missing_question_numbers: missing,
        extra_question_numbers: extra,
        declarations: frozen_declarations.to_vec(),
        unparsed_declarations: Vec::new(),
        reason: None,
    }
}

fn undetermined(canonical_question_numbers: Vec<u32>, reason: &str) -> QuestionCoverageAssessment {
    QuestionCoverageAssessment {
        status: QuestionCoverageStatus::Undetermined,
        declared_question_numbers: Vec::new(),
        canonical_question_numbers,
        missing_question_numbers: Vec::new(),
        extra_question_numbers: Vec::new(),
        declarations: Vec::new(),
        unparsed_declarations: Vec::new(),
        reason: Some(reason.to_string()),
    }
}

fn canonical_question_numbers(authoring: &Value) -> BTreeSet<u32> {
    authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(_, slot)| slot.get("questionNumber").and_then(Value::as_u64))
        .filter_map(|number| u32::try_from(number).ok())
        .collect()
}

fn source_text(physical_shadow: Option<&Value>) -> Option<String> {
    let physical = physical_shadow?;
    if physical.get("schemaVersion").and_then(Value::as_str) != Some("DocumentIRV2") {
        return None;
    }
    let pages = physical.get("pages").and_then(Value::as_array)?;
    let mut parts = Vec::new();
    for page in pages {
        let mut page_parts = Vec::new();
        if let Some(lines) = page.get("lines").and_then(Value::as_array) {
            for line in lines {
                if let Some(text) = line.get("text").and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        page_parts.push(text.trim().to_string());
                    }
                }
            }
        }
        if page_parts.is_empty() {
            if let Some(spans) = page.get("spans").and_then(Value::as_array) {
                for span in spans {
                    if let Some(text) = span.get("text").and_then(Value::as_str) {
                        if !text.trim().is_empty() {
                            page_parts.push(text.trim().to_string());
                        }
                    }
                }
            }
        }
        parts.extend(page_parts);
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn keyword_positions(text: &str, keyword: &str) -> Vec<usize> {
    let lower = text.to_ascii_lowercase();
    let mut positions = Vec::new();
    let mut cursor = 0usize;
    while cursor < lower.len() {
        let Some(relative) = lower[cursor..].find(keyword) else {
            break;
        };
        let start = cursor + relative;
        let end = start + keyword.len();
        let before_ok = start == 0
            || !lower[..start]
                .chars()
                .last()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        let after_ok = end == lower.len()
            || !lower[end..]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_');
        if before_ok && after_ok {
            positions.push(start);
        }
        cursor = end;
    }
    positions
}

fn parse_numbers_after_marker(text: &str, marker: &str) -> Option<Vec<u32>> {
    let tail = text.get(marker.len()..)?;
    // pdfium can expose a multi-digit glyph run as `2 7` instead of `27`.
    // Compact only whitespace between adjacent digits in this numeric
    // declaration; ordinary word spacing remains untouched.
    let candidate = compact_glyph_spaced_digits(&format!("Questions{tail}"));
    parse_question_expression_detailed(&candidate).map(|parsed| parsed.numbers)
}

fn compact_glyph_spaced_digits(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut compacted = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index].is_whitespace()
            && compacted
                .chars()
                .last()
                .is_some_and(|previous| previous.is_ascii_digit())
        {
            let mut next = index + 1;
            while next < chars.len() && chars[next].is_whitespace() {
                next += 1;
            }
            if next < chars.len() && chars[next].is_ascii_digit() {
                index = next;
                continue;
            }
        }
        compacted.push(chars[index]);
        index += 1;
    }
    compacted
}

fn answer_box_context(text: &str, start: usize) -> bool {
    let lower = text.to_ascii_lowercase();
    let prefix = &lower[..start];
    let context = prefix
        .chars()
        .skip(prefix.chars().count().saturating_sub(120))
        .collect::<String>();
    context.contains("answer") || context.contains("write")
}

fn declaration_preview(text: &str) -> String {
    text.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn physical(lines: &[&str]) -> Value {
        json!({
            "schemaVersion":"DocumentIRV2",
            "pages":[{"lines": lines.iter().enumerate().map(|(i, text)| json!({"id":format!("line-{i}"),"text":text})).collect::<Vec<_>>() }]
        })
    }

    #[test]
    fn source_declaration_accepts_compact_question_text_and_answer_boxes() {
        let authoring = json!({"answerSlots": {
            "q1": {"questionNumber": 1},
            "q2": {"questionNumber": 2},
            "q3": {"questionNumber": 3}
        }});
        let assessment = assess(
            &authoring,
            Some(&physical(&[
                "questions1-3",
                "Write your answers in boxes 1-3",
            ])),
        );
        assert_eq!(assessment.status, QuestionCoverageStatus::Complete);
        assert_eq!(assessment.declared_question_numbers, vec![1, 2, 3]);
    }

    #[test]
    fn source_declaration_accepts_answer_boxes_without_a_questions_heading() {
        let authoring = json!({"answerSlots": {
            "q1": {"questionNumber": 1},
            "q2": {"questionNumber": 2},
            "q3": {"questionNumber": 3}
        }});
        let assessment = assess(
            &authoring,
            Some(&physical(&[
                "Write your answers in boxes 1-3 on your answer sheet.",
            ])),
        );
        assert_eq!(assessment.status, QuestionCoverageStatus::Complete);
        assert_eq!(assessment.declared_question_numbers, vec![1, 2, 3]);
    }

    #[test]
    fn source_declaration_falls_back_to_spans_when_lines_are_empty() {
        let authoring = json!({"answerSlots": {
            "q1": {"questionNumber": 1},
            "q2": {"questionNumber": 2}
        }});
        let physical = json!({
            "schemaVersion":"DocumentIRV2",
            "pages":[{
                "lines":[],
                "spans":[{"text":"Questions 1-2"}]
            }]
        });
        let assessment = assess(&authoring, Some(&physical));
        assert_eq!(assessment.status, QuestionCoverageStatus::Complete);
    }

    #[test]
    fn source_declaration_rejoins_glyph_spaced_question_digits() {
        let authoring = json!({"answerSlots": {
            "q27": {"questionNumber": 27},
            "q28": {"questionNumber": 28},
            "q29": {"questionNumber": 29},
            "q30": {"questionNumber": 30},
            "q31": {"questionNumber": 31}
        }});
        let assessment = assess(
            &authoring,
            Some(&physical(&[
                "Questions 2 7 – 3 1",
                "Write your answers in boxes 2 7 - 3 1 on your answer sheet.",
            ])),
        );
        assert_eq!(assessment.status, QuestionCoverageStatus::Complete);
        assert_eq!(
            assessment.declared_question_numbers,
            vec![27, 28, 29, 30, 31]
        );
    }
}
