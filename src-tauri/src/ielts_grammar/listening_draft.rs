//! Listening draft structure: turn recognised section boundaries into a
//! `ListeningStructureV2` and bind every part to the task groups the shared
//! reading grammar already produced for the same question numbers.
//!
//! The draft carries **no passage** (a listening paper has no reading text) and
//! no media: audio is bound later, through a real edit transaction, because the
//! managed-audio table lives in SQLite and the draft builder never reads it.

use serde_json::{json, Value};

use super::evidence::source_anchor_from_job;
use super::instruction_zone::SemanticLine;
use super::listening_parts::{detect_listening_parts, DetectedListeningPart};
use super::question_number::expand_expression;
use crate::schema::ielts_authoring_v2::QuestionNumberExpressionV2;

/// Warning code: a task group's questions fall outside every recognised part.
pub(crate) const LISTENING_TASK_OUTSIDE_PARTS: &str = "LISTENING_TASK_OUTSIDE_PARTS";

pub(crate) struct ListeningDraftStructure {
    pub structure: Value,
    /// Stable codes from section detection plus draft-level warnings.
    pub warnings: Vec<String>,
    pub part_count: usize,
}

fn push_warning(warnings: &mut Vec<String>, code: &str) {
    if !warnings.iter().any(|existing| existing == code) {
        warnings.push(code.to_string());
    }
}

/// Playback policy for an authored paper: a normal practice player. `mock` is a
/// publishing decision (T2), not something local recognition should choose.
fn default_playback_policy() -> Value {
    json!({
        "mode": "practice",
        "autoplay": false,
        "allowPause": true,
        "allowSeek": true,
        "allowReplay": true,
        "refreshBehavior": "resume_from_snapshot",
        "crashRecoveryBehavior": "resume_from_snapshot",
        "showCurrentTime": true,
        "showDuration": true
    })
}

/// Question numbers a task group covers, from its `displayRange` expression.
fn group_question_numbers(group: &Value) -> Vec<u32> {
    group
        .get("displayRange")
        .cloned()
        .and_then(|value| serde_json::from_value::<QuestionNumberExpressionV2>(value).ok())
        .map(|expression| expand_expression(&expression))
        .unwrap_or_default()
}

fn part_anchor(
    part: &DetectedListeningPart,
    lines: &[SemanticLine],
    source_file_id: &str,
    source_hash: &str,
    source_type: &str,
) -> Value {
    if let Some(line) = lines.get(part.heading_line_index) {
        if line.source_anchor.is_object() {
            return line.source_anchor.clone();
        }
    }
    source_anchor_from_job(
        source_file_id,
        source_hash,
        source_type,
        Vec::new(),
        lines
            .get(part.heading_line_index)
            .map(|line| line.page_index)
            .unwrap_or(0),
        None,
    )
}

/// Build the `listening` block of an authoring draft.
///
/// `lines` is the same ordered semantic line slice the rest of the draft builder
/// uses; `task_groups` are the already-built task-group objects. A task group is
/// assigned to the part that contains **all** of its question numbers, so the
/// part's `taskIds` and `expectedQuestionNumbers` stay consistent with the
/// slots the runtime contract checks against.
pub(crate) fn build_listening_structure(
    lines: &[SemanticLine],
    task_groups: &[Value],
    source_file_id: &str,
    source_hash: &str,
    source_type: &str,
) -> ListeningDraftStructure {
    let texts = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>();
    let detected = detect_listening_parts(&texts);
    let mut warnings = detected.warnings.clone();

    let mut task_ids_by_part: Vec<Vec<String>> = vec![Vec::new(); detected.parts.len()];
    for group in task_groups {
        let Some(task_id) = group.get("taskId").and_then(Value::as_str) else {
            continue;
        };
        let numbers = group_question_numbers(group);
        if numbers.is_empty() {
            continue;
        }
        match detected
            .parts
            .iter()
            .position(|part| numbers.iter().all(|number| part.question_numbers.contains(number)))
        {
            Some(index) => task_ids_by_part[index].push(task_id.to_string()),
            None => push_warning(&mut warnings, LISTENING_TASK_OUTSIDE_PARTS),
        }
    }

    let groups_by_id = task_groups
        .iter()
        .filter_map(|group| {
            group
                .get("taskId")
                .and_then(Value::as_str)
                .map(|task_id| (task_id.to_string(), group))
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    let mut parts = Vec::with_capacity(detected.parts.len());
    for (index, part) in detected.parts.iter().enumerate() {
        let task_ids = task_ids_by_part[index].clone();
        let mut numbers = task_ids
            .iter()
            .filter_map(|task_id| groups_by_id.get(task_id))
            .flat_map(|group| group_question_numbers(group))
            .collect::<Vec<_>>();
        numbers.sort_unstable();
        numbers.dedup();
        if numbers.is_empty() {
            numbers = part.question_numbers.clone();
        }
        parts.push(json!({
            "partId": format!("part-{}", part.ordinal),
            "displayLabel": part.display_label,
            "expectedQuestionNumbers": numbers,
            "taskIds": task_ids,
            "sourceAnchors": [part_anchor(part, lines, source_file_id, source_hash, source_type)],
        }));
    }

    let scope = if parts.len() == 4 {
        "complete_exam"
    } else {
        "partial_practice"
    };
    let part_count = parts.len();
    ListeningDraftStructure {
        structure: json!({
            "scope": scope,
            "parts": parts,
            "playbackPolicy": default_playback_policy(),
        }),
        warnings,
        part_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ielts_grammar::instruction_zone::SemanticLine;

    fn lines(texts: &[&str]) -> Vec<SemanticLine> {
        texts
            .iter()
            .enumerate()
            .map(|(index, text)| SemanticLine {
                id: format!("line-{index}"),
                text: (*text).to_string(),
                source_anchor: json!({
                    "sourceFileId": "question-paper",
                    "pageIndex": index / 10,
                    "nodeIds": [format!("line-{index}")],
                    "extractionMode": "pdf_native",
                    "sourceHash": "a".repeat(64)
                }),
                page_index: (index / 10) as i32,
                order: index,
                role: "body".to_string(),
                bbox: None,
            })
            .collect()
    }

    fn task_group(task_id: &str, start: u32, end: u32) -> Value {
        json!({
            "taskId": task_id,
            "displayRange": {"kind": "range", "start": start, "end": end},
            "sourceAnchors": []
        })
    }

    fn synthetic_lines() -> Vec<String> {
        let mut out = vec!["Listening".to_string()];
        for (section, groups) in [
            (1u32, vec![(1u32, 4u32), (5, 7), (8, 10)]),
            (2, vec![(11, 16), (17, 20)]),
            (3, vec![(21, 25), (26, 30)]),
            (4, vec![(31, 40)]),
        ] {
            out.push(format!("SECTION {section}"));
            for (start, end) in groups {
                out.push(format!("Questions {start}-{end}"));
            }
        }
        out
    }

    #[test]
    fn synthetic_four_section_paper_yields_four_parts_with_their_task_groups() {
        let owned = synthetic_lines();
        let refs = owned.iter().map(String::as_str).collect::<Vec<_>>();
        let groups = vec![
            task_group("task-1", 1, 4),
            task_group("task-2", 5, 7),
            task_group("task-3", 8, 10),
            task_group("task-4", 11, 16),
            task_group("task-5", 17, 20),
            task_group("task-6", 21, 25),
            task_group("task-7", 26, 30),
            task_group("task-8", 31, 40),
        ];
        let draft = build_listening_structure(&lines(&refs), &groups, "f", "h", "pdf");
        assert!(draft.warnings.is_empty(), "{:?}", draft.warnings);
        assert_eq!(draft.part_count, 4);
        assert_eq!(draft.structure["scope"], "complete_exam");
        let parts = draft.structure["parts"].as_array().unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|part| part["partId"].as_str().unwrap().to_string())
                .collect::<Vec<_>>(),
            vec!["part-1", "part-2", "part-3", "part-4"]
        );
        assert_eq!(parts[0]["taskIds"], json!(["task-1", "task-2", "task-3"]));
        assert_eq!(parts[1]["taskIds"], json!(["task-4", "task-5"]));
        assert_eq!(parts[3]["taskIds"], json!(["task-8"]));
        assert_eq!(
            parts[0]["expectedQuestionNumbers"],
            json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
        );
        assert_eq!(
            parts[3]["expectedQuestionNumbers"],
            json!((31..=40).collect::<Vec<u32>>())
        );
        assert!(parts[0]["media"].is_null(), "audio is bound later");
    }

    #[test]
    fn a_task_group_outside_every_part_is_reported_not_dropped() {
        let owned = synthetic_lines();
        let refs = owned.iter().map(String::as_str).collect::<Vec<_>>();
        let groups = vec![task_group("task-1", 1, 4), task_group("task-stray", 41, 42)];
        let draft = build_listening_structure(&lines(&refs), &groups, "f", "h", "pdf");
        assert!(draft
            .warnings
            .contains(&LISTENING_TASK_OUTSIDE_PARTS.to_string()));
        let parts = draft.structure["parts"].as_array().unwrap();
        assert_eq!(parts[0]["taskIds"], json!(["task-1"]));
        assert!(parts
            .iter()
            .all(|part| !part["taskIds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == "task-stray")));
    }

    #[test]
    fn a_reading_paper_with_no_section_headings_yields_no_parts() {
        let owned = ["READING PASSAGE 1", "Questions 1-13", "The text"];
        let refs = owned.to_vec();
        let draft = build_listening_structure(
            &lines(&refs),
            &[task_group("task-1", 1, 13)],
            "f",
            "h",
            "pdf",
        );
        assert_eq!(draft.part_count, 0);
        assert_eq!(draft.structure["scope"], "partial_practice");
        assert_eq!(draft.structure["parts"], json!([]));
    }

    /// Draft-level acceptance on the real paper (skips when the private PDF or
    /// pdfium is unavailable). Detection alone is already covered; this ties the
    /// real SECTION 1-4 boundaries to the shared task groups and checks the
    /// draft's own contract: four parts, no group outside every part, and
    /// 1-40 covered exactly once.
    #[test]
    fn the_real_paper_forms_four_parts_covering_one_to_forty() {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/golden/private-real/listening-vol7-t9.pdf");
        if !input.exists() || crate::pdf_geometry::pdfium_library_path().is_none() {
            return;
        }
        let output = std::env::temp_dir().join(format!(
            "pdf2test-listening-draft-{}.json",
            std::process::id()
        ));
        let mut job = crate::job_store::make_job(crate::CreateJobInput {
            title: Some("Listening draft regression".to_string()),
            ..Default::default()
        });
        let source = crate::SourceFile {
            file_id: "listening-vol7-t9".to_string(),
            original_name: "listening-vol7-t9.pdf".to_string(),
            stored_name: "listening-vol7-t9.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: "fixture".to_string(),
            size_bytes: std::fs::metadata(&input).map(|meta| meta.len()).unwrap_or(0),
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

        let mut semantic = Vec::new();
        for (page_index, page) in ir["pages"].as_array().into_iter().flatten().enumerate() {
            for block in page["blocks"].as_array().into_iter().flatten() {
                let Some(text) = block.get("text").and_then(Value::as_str) else {
                    continue;
                };
                let order = semantic.len();
                semantic.push(SemanticLine {
                    id: format!("line-{order}"),
                    text: text.to_string(),
                    source_anchor: json!({
                        "sourceFileId": "listening-vol7-t9",
                        "pageIndex": page_index,
                        "nodeIds": [format!("line-{order}")],
                        "extractionMode": "pdf_native",
                        "sourceHash": "fixture"
                    }),
                    page_index: page_index as i32,
                    order,
                    role: "body".to_string(),
                    bbox: None,
                });
            }
        }

        // Task groups stand in for the shared grammar's output: one group per
        // question range the paper prints. Which part each lands in is the
        // draft's decision, and is what this test checks.
        let texts = semantic
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        let detected = detect_listening_parts(&texts);
        assert_eq!(detected.parts.len(), 4, "{detected:?}");
        let groups = detected
            .parts
            .iter()
            .flat_map(|part| part.groups.iter())
            .enumerate()
            .map(|(index, group)| {
                task_group(
                    &format!("task-{}", index + 1),
                    *group.question_numbers.first().unwrap(),
                    *group.question_numbers.last().unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(groups.len(), 8, "eight question groups on the real paper");

        let draft =
            build_listening_structure(&semantic, &groups, "listening-vol7-t9", "fixture", "pdf");
        assert!(draft.warnings.is_empty(), "{:?}", draft.warnings);
        assert_eq!(draft.part_count, 4);
        assert_eq!(draft.structure["scope"], "complete_exam");
        let parts = draft.structure["parts"].as_array().unwrap();
        let all = parts
            .iter()
            .flat_map(|part| {
                part["expectedQuestionNumbers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|number| number.as_u64().unwrap() as u32)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            all,
            (1..=40).collect::<Vec<u32>>(),
            "every question number 1-40 appears exactly once, in order"
        );
        assert_eq!(
            parts
                .iter()
                .map(|part| part["taskIds"].as_array().unwrap().len())
                .collect::<Vec<_>>(),
            vec![3, 2, 2, 1],
            "groups stay in the part whose question range contains them"
        );
    }
}
