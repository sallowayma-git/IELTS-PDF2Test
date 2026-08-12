use serde_json::{json, Value};

use crate::schema::ielts_authoring_v2::TaskTypeV2;

use super::instruction_signature::is_completion_task;
use super::instruction_zone::SemanticLine;

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
