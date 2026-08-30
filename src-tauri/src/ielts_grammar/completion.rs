use serde_json::{json, Value};

use crate::schema::ielts_authoring_v2::TaskTypeV2;

use super::instruction_signature::is_completion_task;
use super::instruction_zone::SemanticLine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionContainerKind {
    Paragraph,
    Note,
    Form,
    Table,
    Flowchart,
    Diagram,
}

#[derive(Debug, Clone)]
pub(crate) struct CompletionStructureCandidate {
    pub container_kind: CompletionContainerKind,
    pub context_lines: Vec<SemanticLine>,
    pub slot_line_ids: std::collections::BTreeMap<u32, String>,
}

impl CompletionStructureCandidate {
    pub(crate) fn closes_slots(&self, expected_numbers: &[u32]) -> bool {
        expected_numbers
            .iter()
            .all(|number| self.slot_line_ids.contains_key(number))
    }
}

pub(crate) fn recover_completion_structure(
    task_type: &TaskTypeV2,
    lines: &[SemanticLine],
    instruction_line_ids: &[String],
    expected_numbers: &[u32],
) -> CompletionStructureCandidate {
    let container_kind = match task_type {
        TaskTypeV2::NoteCompletion => CompletionContainerKind::Note,
        TaskTypeV2::FormCompletion => CompletionContainerKind::Form,
        TaskTypeV2::TableCompletion => CompletionContainerKind::Table,
        TaskTypeV2::FlowchartCompletion => CompletionContainerKind::Flowchart,
        TaskTypeV2::DiagramLabelCompletion | TaskTypeV2::PlanMapLabelCompletion => {
            CompletionContainerKind::Diagram
        }
        _ => CompletionContainerKind::Paragraph,
    };
    let instruction_ids = instruction_line_ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let mut slot_line_ids = std::collections::BTreeMap::new();
    for line in lines {
        if instruction_ids.contains(line.id.as_str()) {
            continue;
        }
        for number in expected_numbers {
            if completion_line_contains_number_marker(&line.text, *number) {
                slot_line_ids
                    .entry(*number)
                    .or_insert_with(|| line.id.clone());
            }
        }
    }
    let slot_source_ids = slot_line_ids
        .values()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let context_lines = lines
        .iter()
        .filter(|line| !instruction_ids.contains(line.id.as_str()))
        .filter(|line| !slot_source_ids.contains(line.id.as_str()))
        .filter(|line| {
            let role = line.role.to_ascii_lowercase();
            !role.contains("answer") && !role.contains("passage")
        })
        .filter(|line| {
            let lower = line.text.to_ascii_lowercase();
            !lower.trim().is_empty()
                && !lower.trim_start().starts_with("questions ")
                && !lower.contains("complete the ")
                && !lower.contains("write no more than")
                && !lower.contains("write one word")
                && !lower.contains("choose one word")
        })
        .cloned()
        .collect();
    CompletionStructureCandidate {
        container_kind,
        context_lines,
        slot_line_ids,
    }
}

fn completion_line_contains_number_marker(text: &str, number: u32) -> bool {
    let marker = number.to_string();
    text.match_indices(&marker).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let end = start + marker.len();
        let after = text[end..].chars().next();
        !before.is_some_and(|ch| ch.is_ascii_digit())
            && !after.is_some_and(|ch| ch.is_ascii_digit())
            && (start == 0
                || before.is_some_and(|ch| {
                    ch.is_whitespace() || matches!(ch, '(' | '[' | '_' | '.' | ':' | '-')
                }))
            && after.is_none_or(|ch| {
                ch.is_whitespace() || matches!(ch, ')' | ']' | '.' | ':' | '_' | '-')
            })
    })
}

pub(crate) fn completion_host_type(task_type: &TaskTypeV2) -> &'static str {
    match task_type {
        TaskTypeV2::TableCompletion => "table_cell",
        TaskTypeV2::FlowchartCompletion => "flow_step",
        TaskTypeV2::DiagramLabelCompletion | TaskTypeV2::PlanMapLabelCompletion => "figure_hotspot",
        _ if is_completion_task(task_type) => "paragraph",
        _ => "prompt",
    }
}

pub(crate) fn completion_stimulus_has_structure(
    task_type: &TaskTypeV2,
    lines: &[SemanticLine],
) -> bool {
    if !is_completion_task(task_type) {
        return false;
    }
    let joined = lines
        .iter()
        .map(|line| line.text.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    match task_type {
        TaskTypeV2::TableCompletion => joined.contains("|") || joined.contains("table"),
        TaskTypeV2::FlowchartCompletion => joined.contains("flow") || joined.contains("start"),
        TaskTypeV2::DiagramLabelCompletion | TaskTypeV2::PlanMapLabelCompletion => {
            joined.contains("diagram") || joined.contains("map") || joined.contains("plan")
        }
        _ => true,
    }
}

pub(crate) fn answer_slot_node(
    slot_id: &str,
    display_label: &str,
    source_anchor: Value,
    placeholder: &str,
) -> Value {
    json!({
        "type": "answer_slot",
        "id": format!("slot-node-{slot_id}"),
        "sourceAnchors": [source_anchor],
        "provenanceStatus": "derived",
        "slotId": slot_id,
        "displayLabel": display_label,
        "inline": true,
        "placeholder": placeholder
    })
}

pub(crate) fn completion_placeholder(task_type: &TaskTypeV2) -> &'static str {
    match task_type {
        TaskTypeV2::TableCompletion => "table answer",
        TaskTypeV2::FlowchartCompletion => "flowchart answer",
        TaskTypeV2::DiagramLabelCompletion | TaskTypeV2::PlanMapLabelCompletion => "label",
        _ => "answer",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(id: &str, text: &str) -> SemanticLine {
        SemanticLine {
            id: id.to_string(),
            text: text.to_string(),
            source_anchor: json!({"nodeIds":[id]}),
            page_index: 0,
            order: 0,
            role: "question".to_string(),
            bbox: None,
        }
    }

    #[test]
    fn note_structure_separates_headings_from_slot_rows() {
        let lines = vec![
            line("instruction", "Questions 31-32 Complete the notes below"),
            line("heading", "Findings"),
            line("q31", "The first result was 31 ______"),
            line("q32", "Feedback focused on 32 ______"),
        ];
        let candidate = recover_completion_structure(
            &TaskTypeV2::NoteCompletion,
            &lines,
            &["instruction".to_string()],
            &[31, 32],
        );
        assert_eq!(candidate.container_kind, CompletionContainerKind::Note);
        assert!(candidate.closes_slots(&[31, 32]));
        assert_eq!(
            candidate
                .context_lines
                .iter()
                .map(|line| line.id.as_str())
                .collect::<Vec<_>>(),
            vec!["heading"]
        );
    }

    #[test]
    fn number_markers_do_not_match_inside_larger_numbers() {
        assert!(!completion_line_contains_number_marker(
            "The study began in 2019.",
            20
        ));
        assert!(completion_line_contains_number_marker(
            "The result was 20 ______.",
            20
        ));
    }
}
