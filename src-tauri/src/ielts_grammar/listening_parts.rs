//! Listening part (section) structure recognition — pure, not yet wired.
//!
//! A Listening question paper is divided into four parts headed `SECTION 1`
//! .. `SECTION 4` (or `PART 1` .. `PART 4`). PDFs whose text layer carries no
//! space glyphs render these as `SECTION1`, so both spellings are accepted.
//! Each `Questions a-b` heading that follows a part heading opens one task
//! group belonging to that part. The module only reads ordered line text; the
//! caller wires the result into `ListeningPartV2` (see the handoff note in the
//! listening epic) and supplies real task-group ids when it has them.
//!
//! Declared from `authoring_pipeline.rs` via `#[path]` until
//! `ielts_grammar/mod.rs` declares `pub(crate) mod listening_parts;`.

use crate::ielts_grammar::question_number::{
    parse_question_expression_detailed, starts_with_question_heading,
};

/// One `Questions a-b` heading found inside a part.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ListeningQuestionGroup {
    /// Stable id derived from the declared range (`listening-q1-4`).
    pub group_id: String,
    pub question_numbers: Vec<u32>,
    pub heading_line_index: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DetectedListeningPart {
    /// 1-based part number as printed (`SECTION 3` -> 3).
    pub ordinal: u32,
    /// Canonical display label, e.g. `SECTION 3` / `PART 3`.
    pub display_label: String,
    pub heading_line_index: usize,
    /// Sorted union of the question numbers declared by the part's groups.
    pub question_numbers: Vec<u32>,
    pub groups: Vec<ListeningQuestionGroup>,
}

impl DetectedListeningPart {
    pub(crate) fn task_group_ids(&self) -> Vec<String> {
        self.groups
            .iter()
            .map(|group| group.group_id.clone())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ListeningPartsResult {
    pub parts: Vec<DetectedListeningPart>,
    /// Stable warning codes (`LISTENING_...`) for the review surface.
    pub warnings: Vec<String>,
}

/// Parse a line that is exactly a part heading: `SECTION 1`, `Section1`,
/// `PART 4`, `Part 2:`. Running text that merely mentions a section
/// (`Section 1 explains ...`) is rejected, as is any ordinal outside 1-4.
pub(crate) fn parse_part_heading(text: &str) -> Option<(String, u32)> {
    let upper = text.trim().to_ascii_uppercase();
    let (keyword, rest) = ["SECTION", "PART"]
        .iter()
        .find_map(|keyword| upper.strip_prefix(keyword).map(|rest| (*keyword, rest)))?;
    // The whole line must be the heading: keyword, optional space, one digit,
    // optional trailing punctuation.
    let rest = rest
        .trim_start()
        .trim_end_matches(|ch: char| matches!(ch, ':' | '.' | '-') || ch.is_whitespace());
    let mut chars = rest.chars();
    let digit = chars.next()?.to_digit(10)?;
    if chars.next().is_some() || !(1..=4).contains(&digit) {
        return None;
    }
    Some((format!("{keyword} {digit}"), digit))
}

fn push_warning(warnings: &mut Vec<String>, code: &str) {
    if !warnings.iter().any(|existing| existing == code) {
        warnings.push(code.to_string());
    }
}

/// Group ordered lines into Listening parts. A part runs from its heading to
/// the next part heading; every line that *starts* with a `Questions a-b`
/// heading inside it opens one group. `next to questions 17-20` inside an
/// instruction never opens a group.
pub(crate) fn detect_listening_parts(lines: &[&str]) -> ListeningPartsResult {
    let mut result = ListeningPartsResult::default();
    for (line_index, line) in lines.iter().enumerate() {
        if let Some((display_label, ordinal)) = parse_part_heading(line) {
            if result
                .parts
                .last()
                .is_some_and(|previous| ordinal <= previous.ordinal)
            {
                push_warning(&mut result.warnings, "LISTENING_PART_ORDER_INVALID");
            }
            result.parts.push(DetectedListeningPart {
                ordinal,
                display_label,
                heading_line_index: line_index,
                question_numbers: Vec::new(),
                groups: Vec::new(),
            });
            continue;
        }
        if !starts_with_question_heading(line) {
            continue;
        }
        let Some(parsed) = parse_question_expression_detailed(line) else {
            continue;
        };
        let (Some(first), Some(last)) = (parsed.numbers.first(), parsed.numbers.last()) else {
            continue;
        };
        let group = ListeningQuestionGroup {
            group_id: format!("listening-q{first}-{last}"),
            question_numbers: parsed.numbers.clone(),
            heading_line_index: line_index,
        };
        match result.parts.last_mut() {
            Some(part) => part.groups.push(group),
            None => push_warning(
                &mut result.warnings,
                "LISTENING_QUESTIONS_BEFORE_FIRST_PART",
            ),
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for part in &mut result.parts {
        let mut numbers = part
            .groups
            .iter()
            .flat_map(|group| group.question_numbers.iter().copied())
            .collect::<Vec<_>>();
        numbers.sort_unstable();
        if numbers.windows(2).any(|pair| pair[0] == pair[1])
            || numbers.iter().any(|number| !seen.insert(*number))
        {
            push_warning(&mut result.warnings, "LISTENING_QUESTION_NUMBER_DUPLICATE");
        }
        numbers.dedup();
        if numbers.is_empty() {
            push_warning(&mut result.warnings, "LISTENING_PART_WITHOUT_QUESTIONS");
        }
        part.question_numbers = numbers;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::Path;

    fn synthetic_paper(compact: bool) -> Vec<String> {
        let sep = if compact { "" } else { " " };
        let mut lines = vec!["Listening Approximately 30 minutes".to_string()];
        let layout: [(u32, &[(u32, u32)]); 4] = [
            (1, &[(1, 4), (5, 7), (8, 10)]),
            (2, &[(11, 16), (17, 20)]),
            (3, &[(21, 25), (26, 30)]),
            (4, &[(31, 40)]),
        ];
        for (section, groups) in layout {
            lines.push(format!("SECTION{sep}{section}"));
            lines.push(format!(
                "Read the text and answer questions {}-{}",
                groups[0].0,
                groups.last().unwrap().1
            ));
            for (start, end) in groups {
                lines.push(format!("Questions{sep}{start}-{end}"));
                lines.push("Choose the correct answer.".to_string());
                for number in *start..=*end {
                    lines.push(format!("{number} The speaker says the section is open"));
                }
            }
        }
        lines
    }

    #[test]
    fn part_heading_accepts_spaced_and_compact_forms_only() {
        assert_eq!(
            parse_part_heading("SECTION 1"),
            Some(("SECTION 1".into(), 1))
        );
        assert_eq!(
            parse_part_heading("SECTION1"),
            Some(("SECTION 1".into(), 1))
        );
        assert_eq!(
            parse_part_heading("  Section 3:"),
            Some(("SECTION 3".into(), 3))
        );
        assert_eq!(parse_part_heading("PART 4"), Some(("PART 4".into(), 4)));
        assert_eq!(parse_part_heading("Part2"), Some(("PART 2".into(), 2)));
        for text in [
            "Section 1 explains why visitors choose the museum",
            "SECTION 5",
            "SECTION 0",
            "SECTION",
            "Sections 1-4",
            "Particle 2",
            "part of the section",
            "Choose the correct section.",
        ] {
            assert_eq!(parse_part_heading(text), None, "{text:?}");
        }
    }

    #[test]
    fn synthetic_four_section_paper_yields_four_parts_covering_1_to_40() {
        for compact in [false, true] {
            let owned = synthetic_paper(compact);
            let lines = owned.iter().map(String::as_str).collect::<Vec<_>>();
            let result = detect_listening_parts(&lines);
            assert!(result.warnings.is_empty(), "{result:?}");
            assert_eq!(result.parts.len(), 4, "compact={compact}");
            let ordinals = result
                .parts
                .iter()
                .map(|part| part.ordinal)
                .collect::<Vec<_>>();
            assert_eq!(ordinals, vec![1, 2, 3, 4]);
            for (index, part) in result.parts.iter().enumerate() {
                let first = index as u32 * 10 + 1;
                assert_eq!(
                    part.question_numbers,
                    (first..first + 10).collect::<Vec<_>>()
                );
            }
            let group_ids = result
                .parts
                .iter()
                .flat_map(|part| part.task_group_ids())
                .collect::<Vec<_>>();
            assert_eq!(
                group_ids,
                vec![
                    "listening-q1-4",
                    "listening-q5-7",
                    "listening-q8-10",
                    "listening-q11-16",
                    "listening-q17-20",
                    "listening-q21-25",
                    "listening-q26-30",
                    "listening-q31-40"
                ]
            );
        }
    }

    #[test]
    fn passage_text_mentioning_sections_and_choosing_creates_no_parts() {
        let lines = [
            "Reading Passage 1",
            "Section 1 of the report explains how visitors choose a museum.",
            "Part of the section was closed. Choose wisely, the guide said.",
            "The questions in section 2 were harder.",
        ];
        let result = detect_listening_parts(&lines);
        assert!(result.parts.is_empty(), "{result:?}");
    }

    #[test]
    fn question_groups_before_any_part_and_out_of_order_parts_are_reported() {
        let lines = [
            "Questions 1-4",
            "SECTION 2",
            "Questions 11-20",
            "SECTION 1",
            "Questions 5-10",
        ];
        let result = detect_listening_parts(&lines);
        assert!(result
            .warnings
            .contains(&"LISTENING_QUESTIONS_BEFORE_FIRST_PART".to_string()));
        assert!(result
            .warnings
            .contains(&"LISTENING_PART_ORDER_INVALID".to_string()));
    }

    /// Module-level acceptance on the real paper (skips when the private PDF
    /// is absent or pdfium is unavailable): the product pdfium parser's block
    /// text must yield SECTION 1-4 with eight groups covering 1-40.
    #[test]
    fn real_listening_fixture_yields_four_parts_eight_groups_forty_questions() {
        let input = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/golden/private-real/listening-vol7-t9.pdf");
        if !input.exists() || crate::pdf_geometry::pdfium_library_path().is_none() {
            return;
        }
        let output = std::env::temp_dir().join(format!(
            "pdf2test-listening-parts-{}.json",
            std::process::id()
        ));
        let mut job = crate::job_store::make_job(crate::CreateJobInput {
            title: Some("Listening parts regression".to_string()),
            ..Default::default()
        });
        let source = crate::SourceFile {
            file_id: "listening-vol7-t9".to_string(),
            original_name: "listening-vol7-t9.pdf".to_string(),
            stored_name: "listening-vol7-t9.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: "fixture".to_string(),
            size_bytes: std::fs::metadata(&input)
                .map(|meta| meta.len())
                .unwrap_or(0),
            role: "MainQuestion".to_string(),
            imported_at: chrono::Utc::now(),
        };
        job.source_files = vec![source.clone()];
        let Ok(ir) =
            crate::pdf_geometry::parse_pdf_with_pdfium(&job, &source, &input, &output, "auto")
        else {
            return;
        };
        let _ = std::fs::remove_file(&output);
        let texts = ir["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|page| page["blocks"].as_array().cloned().unwrap_or_default())
            .filter_map(|block| {
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        let lines = texts.iter().map(String::as_str).collect::<Vec<_>>();
        let result = detect_listening_parts(&lines);
        assert!(result.warnings.is_empty(), "{result:?}");
        assert_eq!(result.parts.len(), 4, "{result:?}");
        let group_ranges = result
            .parts
            .iter()
            .map(|part| {
                part.groups
                    .iter()
                    .map(|group| {
                        (
                            *group.question_numbers.first().unwrap(),
                            *group.question_numbers.last().unwrap(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            group_ranges,
            vec![
                vec![(1, 4), (5, 7), (8, 10)],
                vec![(11, 16), (17, 20)],
                vec![(21, 25), (26, 30)],
                vec![(31, 40)],
            ]
        );
        let all = result
            .parts
            .iter()
            .flat_map(|part| part.question_numbers.clone())
            .collect::<Vec<_>>();
        assert_eq!(all, (1..=40).collect::<Vec<_>>());
        eprintln!(
            "listening-vol7-t9: {} parts, {} groups, {} questions",
            result.parts.len(),
            result
                .parts
                .iter()
                .map(|part| part.groups.len())
                .sum::<usize>(),
            all.len()
        );
    }
}
