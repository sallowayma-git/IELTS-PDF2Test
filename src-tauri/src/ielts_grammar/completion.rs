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
    // A completion group can carry a shared A-terminal word/phrase bank. The
    // bank belongs to the response control, not to the visible note/summary
    // stimulus. Only consume it after the source closes the declared alphabet;
    // an isolated `A ...` sentence must remain ordinary content.
    let option_bank_line_ids = completion_option_bank_line_ids(lines);
    let context_lines = lines
        .iter()
        .filter(|line| {
            !instruction_ids.contains(line.id.as_str()) || !completion_control_line(&line.text)
        })
        .filter(|line| !slot_source_ids.contains(line.id.as_str()))
        .filter(|line| !option_bank_line_ids.contains(&line.id))
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

fn completion_option_bank_line_ids(lines: &[SemanticLine]) -> std::collections::BTreeSet<String> {
    if lines.is_empty() {
        return std::collections::BTreeSet::new();
    }
    let all_text = lines
        .iter()
        .map(|line| normalize_instruction_text(&line.text))
        .collect::<Vec<_>>();
    let declared_labels = completion_declared_letter_labels(&all_text.join(" "));
    let cue_indices = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            completion_option_bank_heading_or_cue(&line.text).then_some(index)
        })
        .collect::<Vec<_>>();
    if cue_indices.is_empty() || declared_labels.len() < 3 {
        return std::collections::BTreeSet::new();
    }

    // Physical PDF order is often column-major, so A..I need not be adjacent
    // in `lines`. Gather rows by the complete declared label set instead.
    let mut best: Option<(usize, usize, std::collections::BTreeSet<String>)> = None;
    for cue_index in cue_indices {
        let mut seen = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for line in lines.iter().skip(cue_index + 1) {
            let labels = completion_option_labels(&line.text);
            if labels.is_empty() {
                continue;
            }
            let mut accepted_any = false;
            for label in labels {
                if declared_labels.iter().any(|expected| expected == &label) {
                    accepted_any = true;
                    seen.insert(label);
                }
            }
            if accepted_any {
                ids.insert(line.id.clone());
            }
            if declared_labels.iter().all(|label| seen.contains(label)) {
                let size = ids.len();
                if best
                    .as_ref()
                    .is_none_or(|(_, best_size, _)| size > *best_size)
                {
                    best = Some((cue_index, size, ids));
                }
                break;
            }
        }
    }
    let Some((cue_index, row_count, mut ids)) = best else {
        return std::collections::BTreeSet::new();
    };
    let has_structural_heading = lines
        .iter()
        .any(|line| is_completion_structural_option_bank_heading(&line.text));
    if !has_structural_heading && row_count < declared_labels.len() {
        return std::collections::BTreeSet::new();
    }
    // Only consume a heading immediately before the closed bank. A passage
    // may legitimately contain a later `List of ...` heading; removing every
    // such line would silently discard source content after the response
    // bank has already closed.
    let first_bank_index = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| ids.contains(&line.id))
        .map(|(index, _)| index)
        .min()
        .unwrap_or(cue_index);
    for (_, line) in lines
        .iter()
        .enumerate()
        .skip(cue_index)
        .take(first_bank_index.saturating_sub(cue_index).saturating_add(1))
    {
        if is_completion_structural_option_bank_heading(&line.text) {
            ids.insert(line.id.clone());
        }
    }
    ids
}

fn completion_declared_letter_labels(text: &str) -> Vec<String> {
    let normalized = normalize_instruction_text(text).to_ascii_uppercase();
    let compact = normalized
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let terminal = ('C'..='N')
        .rev()
        .find(|label| compact.contains(&format!("A-{label}")))
        .or_else(|| completion_explicit_letter_list_terminal(&normalized));
    terminal
        .map(|terminal| ('A'..=terminal).map(|label| label.to_string()).collect())
        .unwrap_or_default()
}

fn completion_explicit_letter_list_terminal(text: &str) -> Option<char> {
    let tokens = text
        .split_whitespace()
        .map(|token| token.trim_matches(|ch: char| !ch.is_ascii_alphabetic()))
        .filter(|token| token.len() == 1 && token.chars().all(|ch| ch.is_ascii_uppercase()))
        .collect::<Vec<_>>();
    let mut expected = 'A';
    let mut count = 0usize;
    for token in tokens {
        let label = token.chars().next()?;
        if label != expected {
            if count >= 3 {
                return Some(((expected as u8).saturating_sub(1)) as char);
            }
            expected = 'A';
            count = 0;
            continue;
        }
        count += 1;
        expected = ((expected as u8).saturating_add(1)) as char;
    }
    (count >= 3).then_some(((expected as u8).saturating_sub(1)) as char)
}

fn completion_option_bank_heading_or_cue(text: &str) -> bool {
    let normalized = normalize_instruction_text(text);
    let lower = normalized.to_ascii_lowercase();
    is_completion_structural_option_bank_heading(&normalized)
        || [
            "list of words",
            "list of options",
            "list of phrases",
            "list of endings",
            "using the list",
            "using the words",
            "using the phrases",
            "from the list",
            "from the box",
            "in the box below",
            "in the box above",
            "options below",
            "words below",
            "phrases below",
            "endings below",
        ]
        .iter()
        .any(|cue| lower.contains(cue))
}

fn is_completion_structural_option_bank_heading(text: &str) -> bool {
    let normalized = normalize_instruction_text(text);
    let lower = normalized.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("list of ") else {
        return false;
    };
    !rest.trim().is_empty()
        && normalized.split_whitespace().count() <= 8
        && !normalized.ends_with(['.', '?', '!', ';', ':'])
}

fn completion_option_labels(text: &str) -> Vec<String> {
    let normalized = normalize_instruction_text(text);
    let tokens = normalized
        .split_whitespace()
        .map(|token| token.trim_matches(|ch: char| !ch.is_ascii_alphabetic()))
        .collect::<Vec<_>>();
    if !tokens
        .first()
        .is_some_and(|token| token.len() == 1 && token.chars().all(|ch| ch.is_ascii_uppercase()))
    {
        return Vec::new();
    }
    let marker_indices = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            (token.len() == 1 && token.chars().all(|ch| ch.is_ascii_uppercase())).then_some(index)
        })
        .collect::<Vec<_>>();
    marker_indices
        .iter()
        .enumerate()
        .filter_map(|(position, index)| {
            let end = marker_indices
                .get(position + 1)
                .copied()
                .unwrap_or(tokens.len());
            // A bare `A` row is not a closed option. Keeping it in the
            // completion context is safer than dropping source text merely
            // because the label sequence happens to be complete.
            (!tokens[*index + 1..end].is_empty()).then(|| tokens[*index].to_string())
        })
        .collect()
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
    matches!(
        ch,
        '_' | '.'
            | '＿'
            | '…'
            | '⋯'
            | '·'
            | '□'
            | '-'
            | '‐'
            | '‑'
            | '‒'
            | '–'
            | '—'
            | '﹘'
            | '﹣'
            | '－'
    )
}

fn completion_blank_run_end(text: &str, start: usize) -> Option<usize> {
    let mut cursor = start;
    let mut width = 0usize;
    let mut saw_box = false;
    while let Some(ch) = text[cursor..].chars().next() {
        if !completion_blank_char(ch) {
            break;
        }
        saw_box |= ch == '□';
        // A single Unicode ellipsis glyph represents the same visual blank
        // width as three dots.  Treat it as a real marker while still
        // requiring at least two ordinary punctuation/underscore cells so a
        // lone dash or period cannot become an answer slot.
        width += if matches!(ch, '…' | '⋯') { 3 } else { 1 };
        cursor += ch.len_utf8();
    }
    (saw_box || width >= 2).then_some(cursor)
}

fn completion_marker_separator_end(text: &str, start: usize) -> usize {
    let mut cursor = start;
    let mut punctuation_seen = false;
    while let Some(ch) = text[cursor..].chars().next() {
        if ch.is_whitespace() {
            cursor += ch.len_utf8();
        } else if !punctuation_seen
            && matches!(
                ch,
                '.' | ')' | ']' | ':' | '-' | '‐' | '‑' | '–' | '—' | '、'
            )
        {
            // Consume at most one display-number delimiter (`1.`/`1)`/`1:`
            // or `1 -`) and leave a repeated punctuation run for
            // completion_blank_run_end.  The previous loop swallowed dotted
            // and dashed blanks before the slot detector could see them.
            punctuation_seen = true;
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
    if vertical_gap < current_height * 0.55
        || vertical_gap > (previous_height + current_height) * 2.4
    {
        return false;
    }

    // Most PDF engines preserve an indentation on wrapped lines, but a
    // summary/form row that contains a blank is often emitted at exactly the
    // same x coordinate as the preceding physical row.  Recover that shape
    // only when the geometry is adjacent and the text itself proves a
    // continuation: the previous row is unfinished and the next row starts
    // with a lower-case word.  Headings and bullet rows deliberately stay
    // separate, otherwise `Overview` followed by a sentence could be folded
    // into one paragraph.
    let indented = current_x > previous_x + 5.0;
    let same_column = (current_x - previous_x).abs() <= 5.0;
    let previous_text = normalize_instruction_text(&previous.text);
    let previous_terminal = previous_text
        .trim_end()
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, '.' | '?' | '!' | ';' | ':'));
    let current_starts_lowercase = current_text
        .trim_start()
        .chars()
        .next()
        .is_some_and(char::is_lowercase);
    let previous_is_bullet = completion_bullet_body_start(&previous_text).is_some();
    let previous_has_blank = previous_text.chars().any(completion_blank_char);
    let previous_role_is_heading = previous.role.to_ascii_lowercase().contains("heading")
        || previous.role.to_ascii_lowercase().contains("title");
    let previous_heading_like = previous_role_is_heading
        || (!previous_has_blank
            && !previous_is_bullet
            && previous_text.split_whitespace().count() <= 2
            && previous_text
                .chars()
                .find(|ch| !ch.is_whitespace())
                .is_some_and(char::is_uppercase));
    let same_column_lowercase = same_column
        && !previous_is_bullet
        && !previous_heading_like
        && !previous_terminal
        && current_starts_lowercase;

    indented || same_column_lowercase
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
    let separator_needed = children.last().is_some_and(|node| {
        if let Some(text) = node.get("text").and_then(Value::as_str) {
            return !text.ends_with(char::is_whitespace);
        }
        if node.get("type").and_then(Value::as_str) != Some("answer_slot") {
            return false;
        }
        // A blank at the end of a physical row is represented by an
        // answer_slot node rather than text.  The next wrapped source row
        // still needs a visible word boundary (`q1` + `continued`), unless
        // the text immediately before the slot already ends in whitespace.
        children
            .iter()
            .rev()
            .skip(1)
            .find_map(|child| child.get("text").and_then(Value::as_str))
            .map(|text| !text.ends_with(char::is_whitespace))
            .unwrap_or(true)
    });
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

fn completion_slot_numbers(text: &str, expected_numbers: &[u32]) -> Vec<u32> {
    completion_slot_marker_spans(text, expected_numbers)
        .into_iter()
        .map(|(_, _, number)| number)
        .collect()
}

fn completion_inline_children(
    prefix: &str,
    text: &str,
    source_anchor: &Value,
    expected_numbers: &[u32],
    placeholder: &str,
) -> Vec<Value> {
    let spans = completion_slot_marker_spans(text, expected_numbers);
    if spans.is_empty() {
        return vec![completion_text_node(prefix, text, source_anchor)];
    }
    let mut children = Vec::new();
    let mut cursor = 0usize;
    for (index, (start, end, number)) in spans.iter().enumerate() {
        if *start > cursor {
            let part = text[cursor..*start].to_string();
            if !part.trim().is_empty() {
                children.push(completion_text_node(
                    &format!("{prefix}-text-{index}"),
                    &part,
                    source_anchor,
                ));
            }
        }
        children.push(answer_slot_node(
            &format!("q{number}"),
            &number.to_string(),
            source_anchor.clone(),
            placeholder,
        ));
        cursor = *end;
    }
    if cursor < text.len() {
        let tail = text[cursor..].to_string();
        if !tail.trim().is_empty() {
            children.push(completion_text_node(
                &format!("{prefix}-text-tail"),
                &tail,
                source_anchor,
            ));
        }
    }
    children
}

/// Build a table only when at least one source row visibly carries an answer
/// marker.  A bare table heading/row count is not sufficient evidence to
/// invent qN slots: some PDFs put the answer boxes in an image or omit the
/// question page entirely.  Those cases remain source-backed paragraphs and
/// are blocked by the V2 quality gate for manual repair.
pub(crate) fn completion_table_node(
    task_id: &str,
    structure: &CompletionStructureCandidate,
    expected_numbers: &[u32],
    placeholder: &str,
) -> Option<Value> {
    if structure.slot_lines.is_empty() || expected_numbers.is_empty() {
        return None;
    }
    let mut ordered = structure
        .context_lines
        .iter()
        .chain(structure.slot_lines.iter())
        .enumerate()
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(index, line)| (line.page_index, line.order, *index));
    // Keep the physical table window around the explicit answer rows.  The
    // question group may carry a short heading/header prefix, while unrelated
    // passage prose can sit immediately before it in a flattened text layer.
    // Eight rows is deliberately generous for multi-line table headers but
    // still prevents us from turning an entire page into a table.
    let slot_ids = structure
        .slot_lines
        .iter()
        .map(|line| line.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let first_slot_index = ordered
        .iter()
        .position(|(_, line)| slot_ids.contains(line.id.as_str()))
        .unwrap_or(0);
    let last_slot_index = ordered
        .iter()
        .rposition(|(_, line)| slot_ids.contains(line.id.as_str()))
        .unwrap_or(first_slot_index);
    let window_start = first_slot_index.saturating_sub(8);
    let mut rows = Vec::new();
    let mut row_index = 0usize;
    let mut table_slot_count = 0usize;
    for (position, (_, line)) in ordered.into_iter().enumerate() {
        if position < window_start || position > last_slot_index {
            continue;
        }
        let text = normalize_instruction_text(&line.text);
        if text.is_empty() {
            continue;
        }
        let cells = text
            .split('|')
            .enumerate()
            .map(|(cell_index, raw_cell)| {
                let cell_text = raw_cell.trim();
                let cell_slots = completion_slot_numbers(cell_text, expected_numbers);
                table_slot_count += cell_slots.len();
                let cell_id = cell_slots
                    .first()
                    .map(|number| format!("{task_id}-table-cell-q{number}"))
                    .unwrap_or_else(|| format!("{task_id}-table-cell-{row_index}-{cell_index}"));
                let children = completion_inline_children(
                    &cell_id,
                    cell_text,
                    &line.source_anchor,
                    expected_numbers,
                    placeholder,
                );
                let header_scope = (rows.is_empty() && cell_slots.is_empty()).then_some("column");
                json!({
                    "type": "table_cell",
                    "id": cell_id,
                    "sourceAnchors": [line.source_anchor.clone()],
                    "provenanceStatus": "derived",
                    "rowSpan": 1,
                    "colSpan": 1,
                    "headerScope": header_scope.unwrap_or("none"),
                    "children": [{
                        "type": "paragraph",
                        "id": format!("{task_id}-table-cell-{row_index}-{cell_index}-paragraph"),
                        "sourceAnchors": [line.source_anchor.clone()],
                        "provenanceStatus": "derived",
                        "children": children
                    }]
                })
            })
            .collect::<Vec<_>>();
        if cells.is_empty() {
            continue;
        }
        rows.push(json!({
            "type": "table_row",
            "id": format!("{task_id}-table-row-{row_index}"),
            "sourceAnchors": [line.source_anchor.clone()],
            "provenanceStatus": "derived",
            "cells": cells
        }));
        row_index += 1;
    }
    (table_slot_count > 0 && !rows.is_empty()).then(|| {
        json!({
            "type": "table",
            "id": format!("{task_id}-table"),
            "sourceAnchors": structure
                .context_lines
                .iter()
                .chain(structure.slot_lines.iter())
                .map(|line| line.source_anchor.clone())
                .collect::<Vec<_>>(),
            "provenanceStatus": "derived",
            "rows": rows
        })
    })
}

/// Project each explicitly numbered flow-chart row into a flow step.  The
/// previous renderer regenerated a prompt by searching all lines for a
/// leading number, which lost wrapped text and could attach a neighbouring
/// column.  Slot-bearing physical rows are the authoritative step boundary.
pub(crate) fn completion_flowchart_node(
    task_id: &str,
    structure: &CompletionStructureCandidate,
    expected_numbers: &[u32],
    placeholder: &str,
) -> Option<Value> {
    if structure.slot_lines.is_empty() || expected_numbers.is_empty() {
        return None;
    }
    let mut slot_lines = structure.slot_lines.iter().collect::<Vec<_>>();
    slot_lines.sort_by_key(|line| (line.page_index, line.order));
    let mut steps = Vec::new();
    for (index, line) in slot_lines.into_iter().enumerate() {
        let text = normalize_instruction_text(&line.text);
        let numbers = completion_slot_numbers(&text, expected_numbers);
        if numbers.is_empty() {
            continue;
        }
        let children = completion_slot_line_node(
            task_id,
            CompletionContainerKind::Flowchart,
            line,
            expected_numbers,
            placeholder,
        )
        .map(|node| vec![node])
        .unwrap_or_default();
        if children.is_empty() {
            continue;
        }
        let label = numbers
            .first()
            .map(u32::to_string)
            .unwrap_or_else(|| (index + 1).to_string());
        let step_id = if numbers.len() == 1 {
            format!("{task_id}-flow-step-q{}", numbers[0])
        } else {
            format!("{task_id}-flow-step-{index}")
        };
        steps.push(json!({
            "type": "flow_step",
            "id": step_id,
            "sourceAnchors": [line.source_anchor.clone()],
            "provenanceStatus": "derived",
            "label": label,
            "children": children,
            "slotIds": numbers.iter().map(|number| format!("q{number}")).collect::<Vec<_>>()
        }));
    }
    (!steps.is_empty()).then(|| {
        json!({
            "type": "flowchart",
            "id": format!("{task_id}-flowchart"),
            "sourceAnchors": structure
                .slot_lines
                .iter()
                .map(|line| line.source_anchor.clone())
                .collect::<Vec<_>>(),
            "provenanceStatus": "derived",
            "steps": steps
        })
    })
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
    fn dotted_dashed_and_single_ellipsis_blanks_are_recovered() {
        let lines = vec![
            line("q1", "The first answer 1 .... appears here"),
            line("q2", "The second answer 2 --- appears here"),
            line("q3", "The third answer 3 … appears here"),
        ];
        let candidate =
            recover_completion_structure(&TaskTypeV2::SummaryCompletion, &lines, &[], &[1, 2, 3]);
        assert!(candidate.closes_slots(&[1, 2, 3]));
        let nodes = completion_context_nodes_with_slots(
            "task-punctuation",
            candidate.container_kind,
            &candidate.context_lines,
            &candidate.slot_lines,
            &[1, 2, 3],
            "answer",
        );
        let slots = nodes
            .iter()
            .flat_map(|node| node.get("children").and_then(Value::as_array))
            .flatten()
            .filter(|node| node.get("type") == Some(&json!("answer_slot")))
            .filter_map(|node| node.get("slotId").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(slots, vec!["q1", "q2", "q3"]);
    }

    #[test]
    fn wrapped_text_after_a_terminal_slot_keeps_a_word_boundary() {
        let mut first = line("q1", "The result was 1 ______");
        first.bbox = Some([20.0, 100.0, 180.0, 10.0]);
        let mut continuation = line("tail", "confirmed by later testing");
        continuation.order = 1;
        continuation.bbox = Some([20.0, 112.0, 180.0, 10.0]);
        let candidate = recover_completion_structure(
            &TaskTypeV2::SummaryCompletion,
            &[first.clone(), continuation.clone()],
            &[],
            &[1],
        );
        let nodes = completion_context_nodes_with_slots(
            "task-slot-tail",
            candidate.container_kind,
            &candidate.context_lines,
            &candidate.slot_lines,
            &[1],
            "answer",
        );
        let text = nodes[0]["children"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|node| node.get("text").and_then(Value::as_str))
            .collect::<String>();
        assert_eq!(text, "The result was confirmed by later testing");
        assert_eq!(nodes[0]["children"][1]["type"], json!("answer_slot"));
    }

    #[test]
    fn closed_shared_word_bank_is_not_rendered_as_completion_stimulus() {
        let lines = vec![
            line(
                "heading",
                "Complete the summary using a word A-I from the box.",
            ),
            line("q36", "One method would 36 ______ asteroids at the planet."),
            line(
                "q37",
                "The rockets would take years to 37 ______ the distance.",
            ),
            line("bank-title", "List of words"),
            line("bank-a", "A cover"),
            line("bank-g", "G power"),
            line("bank-d", "D increase"),
            line("bank-b", "B create"),
            line("bank-e", "E land"),
            line("bank-h", "H rise"),
            line("bank-c", "C hit"),
            line("bank-f", "F drive"),
            line("bank-i", "I shoot"),
        ];
        let candidate = recover_completion_structure(
            &TaskTypeV2::SummaryCompletion,
            &lines,
            &["heading".to_string()],
            &[36, 37],
        );
        assert!(candidate.closes_slots(&[36, 37]));
        assert!(candidate
            .context_lines
            .iter()
            .all(|line| !line.id.starts_with("bank-")));
        assert_eq!(candidate.context_lines.len(), 0);
        assert_eq!(candidate.slot_lines.len(), 2);
    }

    #[test]
    fn incomplete_shared_word_bank_remains_source_context() {
        let lines = vec![
            line(
                "heading",
                "Complete the notes using a word A-C from the box.",
            ),
            line("q1", "The first result 1 ______ was recorded."),
            line("bank-title", "List of words"),
            line("bank-a", "A north"),
            line("bank-b", "B south"),
            // C is present as a label only; the bank is not closed and must
            // not be silently removed from the visible source stimulus.
            line("bank-c", "C"),
        ];
        let candidate = recover_completion_structure(
            &TaskTypeV2::NoteCompletion,
            &lines,
            &["heading".to_string()],
            &[1],
        );
        assert!(candidate
            .context_lines
            .iter()
            .any(|line| line.id == "bank-title"));
        assert!(candidate
            .context_lines
            .iter()
            .any(|line| line.id == "bank-c"));
    }

    #[test]
    fn same_column_lowercase_rows_merge_but_headings_and_bullets_do_not() {
        let mut previous = line("previous", "The summary sentence continues here");
        previous.bbox = Some([20.0, 100.0, 180.0, 10.0]);
        let mut current = line("current", "with a lower-case continuation");
        current.bbox = Some([20.0, 112.0, 180.0, 10.0]);
        assert!(completion_should_merge_continuation(
            Some(&previous),
            &current,
            &current.text
        ));
        let nodes = completion_context_nodes_with_slots(
            "task-wrapped",
            CompletionContainerKind::Paragraph,
            &[previous.clone(), current.clone()],
            &[],
            &[],
            "answer",
        );
        assert_eq!(nodes.len(), 1);
        assert_eq!(
            nodes[0]["children"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|node| node.get("text").and_then(Value::as_str))
                .collect::<String>(),
            "The summary sentence continues here with a lower-case continuation"
        );

        let mut heading = line("heading", "Overview");
        heading.bbox = Some([20.0, 100.0, 180.0, 10.0]);
        assert!(!completion_should_merge_continuation(
            Some(&heading),
            &current,
            &current.text
        ));

        let mut bullet = line("bullet", "• key finding");
        bullet.bbox = Some([20.0, 100.0, 180.0, 10.0]);
        assert!(!completion_should_merge_continuation(
            Some(&bullet),
            &current,
            &current.text
        ));

        let mut finished = line("finished", "The summary ends.");
        finished.bbox = Some([20.0, 100.0, 180.0, 10.0]);
        assert!(!completion_should_merge_continuation(
            Some(&finished),
            &current,
            &current.text
        ));
    }

    #[test]
    fn table_node_uses_only_source_rows_with_visible_slots() {
        let lines = vec![
            line("heading", "Label | Answer"),
            line("q1", "First item | 1 ______"),
            line("q2", "Second item | 2 ______"),
        ];
        let candidate =
            recover_completion_structure(&TaskTypeV2::TableCompletion, &lines, &[], &[1, 2]);
        let table = completion_table_node("task-table", &candidate, &[1, 2], "answer")
            .expect("visible source rows should produce a table");
        assert_eq!(table["type"], json!("table"));
        assert_eq!(table["rows"].as_array().map(Vec::len), Some(3));
        let slot_count = table["rows"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row["cells"].as_array().unwrap())
            .flat_map(|cell| cell["children"][0]["children"].as_array().unwrap())
            .filter(|node| node["type"] == json!("answer_slot"))
            .count();
        assert_eq!(slot_count, 2);
    }

    #[test]
    fn flowchart_node_uses_numbered_source_rows_as_steps() {
        let lines = vec![
            line("start", "Start"),
            line("q1", "1 inspect the sample ______"),
            line("q2", "2 record the result ______"),
        ];
        let candidate =
            recover_completion_structure(&TaskTypeV2::FlowchartCompletion, &lines, &[], &[1, 2]);
        let flowchart = completion_flowchart_node("task-flow", &candidate, &[1, 2], "answer")
            .expect("visible source rows should produce a flowchart");
        assert_eq!(flowchart["type"], json!("flowchart"));
        assert_eq!(flowchart["steps"].as_array().map(Vec::len), Some(2));
        assert_eq!(flowchart["steps"][0]["slotIds"], json!(["q1"]));
        assert_eq!(flowchart["steps"][1]["slotIds"], json!(["q2"]));
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
