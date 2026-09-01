use serde_json::{json, Value};

use crate::schema::ielts_authoring_v2::TaskTypeV2;

use super::instruction_signature::is_completion_task;
use super::instruction_zone::{normalize_instruction_text, SemanticLine};

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
    /// Physical rows that contain one or more expected answer markers.  They
    /// are kept separate from `context_lines` so callers can render the row
    /// text and its inline slots without making a slot row look like a second
    /// piece of context.
    pub slot_lines: Vec<SemanticLine>,
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
        // A physical question row can remain inside the instruction zone when
        // its number is embedded after a bullet (for example, `• ... 7 ___`).
        // A slot marker is stronger evidence than the zone's coarse boundary;
        // keep scanning such a line instead of treating it as prose.
        let has_slot_marker = expected_numbers
            .iter()
            .any(|number| completion_line_contains_slot_marker(&line.text, *number));
        if instruction_ids.contains(line.id.as_str())
            && !has_slot_marker
            && completion_control_line(&line.text)
        {
            continue;
        }
        for number in expected_numbers {
            if completion_line_contains_slot_marker(&line.text, *number) {
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
    let slot_lines = lines
        .iter()
        .filter(|line| slot_source_ids.contains(&line.id))
        .cloned()
        .collect::<Vec<_>>();
    let context_lines = lines
        .iter()
        .filter(|line| {
            !instruction_ids.contains(line.id.as_str()) || !completion_control_line(&line.text)
        })
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
        slot_lines,
    }
}

fn completion_control_line(text: &str) -> bool {
    let lower = normalize_instruction_text(text).to_ascii_lowercase();
    let lower = lower.trim();
    lower.starts_with("questions ")
        || lower.starts_with("question ")
        || lower.starts_with("complete ")
        || lower.starts_with("choose ")
        || lower.starts_with("write ")
        || lower.starts_with("use no more")
        || lower.starts_with("use one ")
        || lower.starts_with("fill in ")
        || lower.starts_with("look at ")
        || lower.starts_with("answers in boxes")
        || lower.starts_with("write your answers")
        || lower.starts_with("you should spend")
}

fn completion_line_contains_number_marker(text: &str, number: u32) -> bool {
    !completion_number_marker_spans(text, number).is_empty()
}

fn completion_number_marker_spans(text: &str, number: u32) -> Vec<(usize, usize)> {
    let marker = number.to_string();
    text.match_indices(&marker)
        .filter_map(|(start, _)| {
            let before = text[..start].chars().next_back();
            let end = start + marker.len();
            let after = text[end..].chars().next();
            let valid = !before.is_some_and(|ch| ch.is_ascii_digit())
                && !after.is_some_and(|ch| ch.is_ascii_digit())
                && (start == 0
                    || before.is_some_and(|ch| {
                        ch.is_whitespace() || matches!(ch, '(' | '[' | '_' | '.' | ':' | '-')
                    }))
                && after.is_none_or(|ch| {
                    ch.is_whitespace()
                        || completion_blank_char(ch)
                        || matches!(ch, ')' | ']' | '.' | ':' | '_' | '-')
                });
            valid.then_some((start, end))
        })
        .collect()
}

fn completion_line_contains_slot_marker(text: &str, number: u32) -> bool {
    completion_slot_marker_spans(text, &[number])
        .iter()
        .any(|(_, _, candidate)| *candidate == number)
}

fn completion_blank_char(ch: char) -> bool {
    matches!(ch, '_' | '＿' | '…' | '·' | '□')
}

fn completion_blank_run_end(text: &str, start: usize) -> Option<usize> {
    let mut cursor = start;
    let mut count = 0usize;
    let mut saw_box = false;
    while let Some(ch) = text[cursor..].chars().next() {
        if !completion_blank_char(ch) {
            break;
        }
        saw_box |= ch == '□';
        count += 1;
        cursor += ch.len_utf8();
    }
    (saw_box || count >= 2).then_some(cursor)
}

fn completion_marker_separator_end(text: &str, start: usize) -> usize {
    let mut cursor = start;
    while let Some(ch) = text[cursor..].chars().next() {
        if ch.is_whitespace() || matches!(ch, '.' | ')' | ']' | ':' | '-' | '、') {
            cursor += ch.len_utf8();
        } else {
            break;
        }
    }
    cursor
}

/// Return `(start, end, question_number)` spans for expected answer rows.
/// The number itself is part of the consumed span so that the inline slot
/// displays the number exactly once.  A number is accepted only when it is
/// attached to a visible answer blank; this prevents years and other prose
/// numbers from becoming scoring slots.
fn completion_slot_marker_spans(text: &str, expected_numbers: &[u32]) -> Vec<(usize, usize, u32)> {
    if expected_numbers.is_empty() || text.is_empty() {
        return Vec::new();
    }
    let all_markers = expected_numbers
        .iter()
        .flat_map(|number| {
            completion_number_marker_spans(text, *number)
                .into_iter()
                .map(move |(start, end)| (start, end, *number))
        })
        .collect::<Vec<_>>();
    let mut markers = Vec::new();
    for (start, end, number) in all_markers.iter().copied() {
        let next_marker = all_markers
            .iter()
            .filter(|(_, candidate_end, _)| *candidate_end > end)
            .map(|(candidate_start, _, _)| *candidate_start)
            .min()
            .unwrap_or(text.len());
        let separator_end = completion_marker_separator_end(text, end);
        let mut blank_end = completion_blank_run_end(text, separator_end);
        if blank_end.is_none() {
            let search_end = next_marker.max(separator_end).min(text.len());
            let mut cursor = separator_end;
            while cursor < search_end {
                let Some(ch) = text[cursor..].chars().next() else {
                    break;
                };
                if completion_blank_char(ch) {
                    if let Some(end) = completion_blank_run_end(text, cursor) {
                        blank_end = Some(end);
                        break;
                    }
                }
                cursor += ch.len_utf8();
            }
        }
        let Some(blank_end) = blank_end else {
            continue;
        };
        if blank_end <= start || blank_end > next_marker {
            continue;
        }
        markers.push((start, blank_end, number));
    }
    markers.sort_by_key(|(start, _, _)| *start);
    let mut seen_numbers = std::collections::BTreeSet::new();
    markers
        .into_iter()
        .filter(|(_, _, number)| seen_numbers.insert(*number))
        .collect()
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

/// Project the non-answer portion of a completion task into the same
/// structural primitives used by the renderer.  The old path emitted one
/// paragraph per line and capped the result at twelve lines, which silently
/// dropped the tail of long notes/summaries.  We retain every source-backed
/// context line and group adjacent bullet rows into a real list so wrapping or
/// a long completion body cannot flatten/lose its visible structure.
pub(crate) fn completion_context_nodes(
    task_id: &str,
    container_kind: CompletionContainerKind,
    lines: &[SemanticLine],
) -> Vec<Value> {
    completion_context_nodes_with_slots(task_id, container_kind, lines, &[], &[], "answer")
}

/// Build completion stimulus nodes while retaining physical rows that carry
/// answer blanks.  The legacy `completion_context_nodes` wrapper remains
/// available for callers that only have context prose; the V2 task builder
/// uses this variant so a student sees one canonical stimulus row with an
/// inline `answer_slot`, rather than a second list of question prompts.
pub(crate) fn completion_context_nodes_with_slots(
    task_id: &str,
    container_kind: CompletionContainerKind,
    context_lines: &[SemanticLine],
    slot_lines: &[SemanticLine],
    expected_numbers: &[u32],
    placeholder: &str,
) -> Vec<Value> {
    let mut ordered_lines = context_lines
        .iter()
        .chain(slot_lines.iter())
        .enumerate()
        .collect::<Vec<_>>();
    // `SemanticLine::order` is the geometry order within the group. Keep the
    // input index as an explicit tie-breaker: synthetic/unit fixtures (and a
    // few flattened PDFs) legitimately give several lines the same order,
    // and sorting those ties by id would move a heading behind its bullets.
    ordered_lines.sort_by_key(|(index, line)| (line.page_index, line.order, *index));
    let mut nodes = Vec::new();
    let mut pending_bullets: Vec<(&SemanticLine, String)> = Vec::new();

    let flush_bullets = |nodes: &mut Vec<Value>, bullets: &mut Vec<(&SemanticLine, String)>| {
        if bullets.is_empty() {
            return;
        }
        let list_index = nodes.len();
        let list_id = format!("{task_id}-stimulus-list-{list_index}");
        let anchors = bullets
            .iter()
            .map(|(line, _)| line.source_anchor.clone())
            .collect::<Vec<_>>();
        let items = bullets
            .iter()
            .enumerate()
            .map(|(index, (line, text))| {
                let item_id = format!("{list_id}-item-{index}");
                let paragraph_id = format!("{item_id}-paragraph");
                let text_id = format!("{paragraph_id}-text");
                json!({
                    "type": "list_item",
                    "id": item_id,
                    "sourceAnchors": [line.source_anchor.clone()],
                    "provenanceStatus": "derived",
                    "children": [{
                        "type": "paragraph",
                        "id": paragraph_id,
                        "sourceAnchors": [line.source_anchor.clone()],
                        "provenanceStatus": "derived",
                        "children": [{
                            "type": "text",
                            "id": text_id,
                            "sourceAnchors": [line.source_anchor.clone()],
                            "provenanceStatus": "source",
                            "text": text
                        }]
                    }]
                })
            })
            .collect::<Vec<_>>();
        nodes.push(json!({
            "type": "bullet_list",
            "id": list_id,
            "sourceAnchors": anchors,
            "provenanceStatus": "derived",
            "items": items
        }));
        bullets.clear();
    };

    let mut previous_line: Option<&SemanticLine> = None;
    for (_, line) in ordered_lines {
        let text = normalize_instruction_text(&line.text);
        if text.trim().is_empty() {
            continue;
        }
        if let Some(slot_node) =
            completion_slot_line_node(task_id, container_kind, line, expected_numbers, placeholder)
        {
            let can_merge = completion_should_merge_continuation(previous_line, line, &text);
            flush_bullets(&mut nodes, &mut pending_bullets);
            if !can_merge || !append_completion_continuation(&mut nodes, &slot_node) {
                nodes.push(slot_node);
            }
            previous_line = Some(line);
            continue;
        }
        if let Some(item_text) = completion_bullet_item_text(&text) {
            pending_bullets.push((line, item_text));
            previous_line = Some(line);
            continue;
        }
        let can_merge = completion_should_merge_continuation(previous_line, line, &text);
        flush_bullets(&mut nodes, &mut pending_bullets);
        let id = format!("{task_id}-stimulus-{}", line.id);
        let looks_like_heading = matches!(
            container_kind,
            CompletionContainerKind::Note | CompletionContainerKind::Form
        ) && text.split_whitespace().count() <= 8
            && !text.ends_with(['.', '?', '!', ';', ':']);
        let node = if looks_like_heading {
            json!({
                "type": "heading",
                "id": id,
                "sourceAnchors": [line.source_anchor.clone()],
                "provenanceStatus": "source",
                "level": 3,
                "children": [{
                    "type": "text",
                    "id": format!("{id}-text"),
                    "sourceAnchors": [line.source_anchor.clone()],
                    "provenanceStatus": "source",
                    "text": text
                }]
            })
        } else {
            json!({
                "type": "paragraph",
                "id": id,
                "sourceAnchors": [line.source_anchor.clone()],
                "provenanceStatus": "derived",
                "children": [{
                    "type": "text",
                    "id": format!("{id}-text"),
                    "sourceAnchors": [line.source_anchor.clone()],
                    "provenanceStatus": "source",
                    "text": text
                }]
            })
        };
        if !can_merge || !append_completion_continuation(&mut nodes, &node) {
            nodes.push(node);
        }
        previous_line = Some(line);
    }
    flush_bullets(&mut nodes, &mut pending_bullets);
    nodes
}

/// A PDF frequently emits an indented continuation row as a separate block
/// (for example `- ... their` followed by `8 ______`).  Treat that row as a
/// continuation of the previous paragraph/list item when its geometry proves
/// it is a wrapped line, otherwise a visible answer blank would be rendered on
/// its own line and the canonical stimulus would lose the sentence shape.
fn completion_should_merge_continuation(
    previous: Option<&SemanticLine>,
    current: &SemanticLine,
    current_text: &str,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if previous.page_index != current.page_index
        || completion_bullet_body_start(current_text).is_some()
    {
        return false;
    }
    let Some(previous_bbox) = previous.bbox else {
        return false;
    };
    let Some(current_bbox) = current.bbox else {
        return false;
    };
    let previous_x = previous_bbox[0];
    let current_x = current_bbox[0];
    let previous_height = previous_bbox[3].abs().max(1.0);
    let current_height = current_bbox[3].abs().max(1.0);
    let vertical_gap = (current_bbox[1] - previous_bbox[1]).abs();
    // A wrapped continuation is indented and remains within roughly two text
    // lines of its predecessor.  A small minimum gap avoids merging a table
    // row or a distinct paragraph that happens to share the same left edge.
    current_x > previous_x + 5.0
        && vertical_gap >= current_height * 0.55
        && vertical_gap <= (previous_height + current_height) * 2.4
}

fn append_completion_continuation(nodes: &mut [Value], incoming: &Value) -> bool {
    let Some(incoming_children) = incoming.get("children").and_then(Value::as_array) else {
        return false;
    };
    if incoming_children.is_empty() {
        return false;
    }
    let Some(last) = nodes.last_mut() else {
        return false;
    };
    let target = if last.get("type").and_then(Value::as_str) == Some("paragraph") {
        Some(last)
    } else if last.get("type").and_then(Value::as_str) == Some("bullet_list") {
        last.get_mut("items")
            .and_then(Value::as_array_mut)
            .and_then(|items| items.last_mut())
            .and_then(|item| item.get_mut("children"))
            .and_then(Value::as_array_mut)
            .and_then(|children| children.last_mut())
            .filter(|node| node.get("type").and_then(Value::as_str) == Some("paragraph"))
    } else {
        None
    };
    let Some(target) = target else {
        return false;
    };
    let target_id = target
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("paragraph")
        .to_string();
    let Some(children) = target.get_mut("children").and_then(Value::as_array_mut) else {
        return false;
    };
    let separator_needed = children
        .last()
        .and_then(|node| node.get("text"))
        .and_then(Value::as_str)
        .is_some_and(|text| !text.ends_with(char::is_whitespace));
    if separator_needed {
        let anchor = incoming
            .get("sourceAnchors")
            .and_then(Value::as_array)
            .and_then(|anchors| anchors.first())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let separator_id = format!("{}-continuation-space-{}", target_id, children.len());
        children.push(completion_text_node(&separator_id, " ", &anchor));
    }
    children.extend(incoming_children.iter().cloned());
    if let (Some(target_anchors), Some(incoming_anchors)) = (
        target
            .get_mut("sourceAnchors")
            .and_then(Value::as_array_mut),
        incoming.get("sourceAnchors").and_then(Value::as_array),
    ) {
        for anchor in incoming_anchors {
            if !target_anchors.iter().any(|existing| existing == anchor) {
                target_anchors.push(anchor.clone());
            }
        }
    }
    true
}

fn completion_text_node(id: &str, text: &str, source_anchor: &Value) -> Value {
    json!({
        "type": "text",
        "id": id,
        "sourceAnchors": [source_anchor.clone()],
        "provenanceStatus": "source",
        "text": text
    })
}

fn completion_bullet_body_start(text: &str) -> Option<usize> {
    let leading = text.len().saturating_sub(text.trim_start().len());
    let body = &text[leading..];
    let first = body.chars().next()?;
    let first_end = first.len_utf8();
    if matches!(
        first,
        '•' | '·' | '▪' | '‣' | '○' | '◦' | '*' | '–' | '—' | '-'
    ) || (first == 'o'
        && body[first_end..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace))
    {
        let mut cursor = leading + first_end;
        while let Some(ch) = text[cursor..].chars().next() {
            if ch.is_whitespace() {
                cursor += ch.len_utf8();
            } else {
                break;
            }
        }
        return Some(cursor);
    }
    if first.is_ascii_lowercase() && body[first_end..].starts_with(')') {
        let mut cursor = leading + first_end + 1;
        while let Some(ch) = text[cursor..].chars().next() {
            if ch.is_whitespace() {
                cursor += ch.len_utf8();
            } else {
                break;
            }
        }
        return Some(cursor);
    }
    None
}

fn completion_slot_line_node(
    task_id: &str,
    _container_kind: CompletionContainerKind,
    line: &SemanticLine,
    expected_numbers: &[u32],
    placeholder: &str,
) -> Option<Value> {
    let text = normalize_instruction_text(&line.text);
    let bullet_start = completion_bullet_body_start(&text).unwrap_or(0);
    let body = &text[bullet_start..];
    let spans = completion_slot_marker_spans(body, expected_numbers);
    if spans.is_empty() {
        return None;
    }

    let paragraph_id = format!("{task_id}-stimulus-{}", line.id);
    let mut children = Vec::new();
    let mut cursor = 0usize;
    for (index, (start, end, number)) in spans.iter().enumerate() {
        if *start > cursor {
            let text_part = body[cursor..*start].to_string();
            if !text_part.trim().is_empty() {
                children.push(completion_text_node(
                    &format!("{paragraph_id}-text-{index}"),
                    &text_part,
                    &line.source_anchor,
                ));
            }
        }
        children.push(answer_slot_node(
            &format!("q{number}"),
            &number.to_string(),
            line.source_anchor.clone(),
            placeholder,
        ));
        cursor = *end;
    }
    if cursor < body.len() {
        let text_part = body[cursor..].to_string();
        if !text_part.trim().is_empty() {
            children.push(completion_text_node(
                &format!("{paragraph_id}-text-tail"),
                &text_part,
                &line.source_anchor,
            ));
        }
    }
    let paragraph = json!({
        "type": "paragraph",
        "id": paragraph_id,
        "sourceAnchors": [line.source_anchor.clone()],
        "provenanceStatus": "derived",
        "children": children
    });
    if bullet_start == 0 {
        return Some(paragraph);
    }
    let list_id = format!("{task_id}-stimulus-list-{}", line.id);
    let item_id = format!("{list_id}-item");
    Some(json!({
        "type": "bullet_list",
        "id": list_id,
        "sourceAnchors": [line.source_anchor.clone()],
        "provenanceStatus": "derived",
        "items": [{
            "type": "list_item",
            "id": item_id,
            "sourceAnchors": [line.source_anchor.clone()],
            "provenanceStatus": "derived",
            "children": [paragraph]
        }]
    }))
}

fn completion_bullet_item_text(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    let rest = chars.as_str().trim_start();
    let is_bullet = matches!(
        first,
        '•' | '·' | '▪' | '‣' | '○' | '◦' | '*' | '–' | '—' | '-'
    ) || (first == 'o'
        && chars
            .as_str()
            .chars()
            .next()
            .is_some_and(char::is_whitespace))
        || (first.is_ascii_lowercase() && rest.starts_with(')'));
    if !is_bullet || rest.is_empty() {
        return None;
    }
    let rest = if first.is_ascii_lowercase() && rest.starts_with(')') {
        rest[1..].trim_start()
    } else {
        rest
    };
    (!rest.is_empty()).then(|| rest.to_string())
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
        assert_eq!(
            candidate
                .slot_lines
                .iter()
                .map(|line| line.id.as_str())
                .collect::<Vec<_>>(),
            vec!["q31", "q32"]
        );
    }

    #[test]
    fn slot_bearing_context_nodes_emit_each_inline_slot_once() {
        let lines = vec![
            line("heading", "Findings"),
            line("q31", "The first result was 31 ______."),
            line("q32", "Feedback focused on 32 ______."),
        ];
        let candidate =
            recover_completion_structure(&TaskTypeV2::NoteCompletion, &lines, &[], &[31, 32]);
        let nodes = completion_context_nodes_with_slots(
            "task-inline",
            candidate.container_kind,
            &candidate.context_lines,
            &candidate.slot_lines,
            &[31, 32],
            "answer",
        );
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0]["type"], json!("heading"));
        assert_eq!(nodes[1]["type"], json!("paragraph"));
        assert_eq!(nodes[2]["type"], json!("paragraph"));
        assert_eq!(nodes[1]["id"], json!("task-inline-stimulus-q31"));
        assert_eq!(nodes[2]["id"], json!("task-inline-stimulus-q32"));
        let slots = nodes
            .iter()
            .flat_map(|node| {
                node.get("children")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter(|node| node.get("type") == Some(&json!("answer_slot")))
            .collect::<Vec<_>>();
        assert_eq!(slots.len(), 2);
        assert_eq!(
            slots
                .iter()
                .filter_map(|node| node.get("slotId").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["q31", "q32"]
        );
        assert!(nodes[1]["children"].as_array().unwrap().iter().any(|node| {
            node.get("text").and_then(Value::as_str) == Some("The first result was ")
        }));

        let bullet = line(
            "q7",
            "• some animals destroy the seeds, preventing 7 __________",
        );
        let bullet_candidate = recover_completion_structure(
            &TaskTypeV2::NoteCompletion,
            std::slice::from_ref(&bullet),
            &["q7".to_string()],
            &[7],
        );
        assert!(bullet_candidate.closes_slots(&[7]));
        let bullet_nodes = completion_context_nodes_with_slots(
            "task-bullet",
            bullet_candidate.container_kind,
            &bullet_candidate.context_lines,
            &bullet_candidate.slot_lines,
            &[7],
            "answer",
        );
        assert_eq!(bullet_nodes.len(), 1);
        assert_eq!(bullet_nodes[0]["type"], json!("bullet_list"));
        assert_eq!(
            bullet_nodes[0]["items"][0]["children"][0]["children"][1]["slotId"],
            json!("q7")
        );

        let ellipsis = line("q24", "reading 24…………. exposed celebrities");
        assert!(completion_line_contains_slot_marker(&ellipsis.text, 24));
        let ellipsis_candidate = recover_completion_structure(
            &TaskTypeV2::SummaryCompletion,
            std::slice::from_ref(&ellipsis),
            &[],
            &[24],
        );
        let ellipsis_nodes = completion_context_nodes_with_slots(
            "task-ellipsis",
            ellipsis_candidate.container_kind,
            &ellipsis_candidate.context_lines,
            &ellipsis_candidate.slot_lines,
            &[24],
            "answer",
        );
        assert_eq!(ellipsis_nodes.len(), 1);
        assert_eq!(ellipsis_nodes[0]["children"][1]["slotId"], json!("q24"));
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

    #[test]
    fn context_nodes_keep_long_completion_tails_and_group_bullets() {
        let mut lines = vec![line("heading", "Findings")];
        lines.extend((0..14).map(|index| {
            line(
                &format!("bullet-{index}"),
                &format!("• observation {index}"),
            )
        }));
        lines.push(line("tail", "The final observation remains source backed."));

        let nodes = completion_context_nodes("task-1", CompletionContainerKind::Note, &lines);
        assert_eq!(
            nodes.first().and_then(|node| node.get("type")),
            Some(&json!("heading"))
        );
        assert_eq!(
            nodes.get(1).and_then(|node| node.get("type")),
            Some(&json!("bullet_list"))
        );
        assert_eq!(
            nodes
                .get(1)
                .and_then(|node| node.get("items"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(14)
        );
        assert_eq!(nodes.len(), 3);
        assert!(nodes.iter().any(|node| {
            node.get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|child| {
                    child.get("text").and_then(Value::as_str)
                        == Some("The final observation remains source backed.")
                })
        }));
    }

    #[test]
    fn context_nodes_preserve_source_anchors_for_list_items() {
        let lines = vec![line("a", "a) first item"), line("b", "b) second item")];
        let nodes = completion_context_nodes("task-2", CompletionContainerKind::Form, &lines);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0]["type"], json!("bullet_list"));
        assert_eq!(
            nodes[0]["items"][0]["children"][0]["children"][0]["text"],
            json!("first item")
        );
        assert_eq!(
            nodes[0]["items"][1]["sourceAnchors"][0]["nodeIds"],
            json!(["b"])
        );
    }
}
