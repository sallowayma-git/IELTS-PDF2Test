use serde_json::{json, Value};

use crate::schema::ielts_authoring_v2::TaskTypeV2;

use super::instruction_signature::is_completion_task;
use super::instruction_zone::{
    normalize_instruction_text, semantic_lines_from_v2_shadow, SemanticLine,
};

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
    /// Inline slot spans recovered from the page's *drawn* blanks, keyed by the
    /// line that hosts them.  These are only produced for numbers the printed
    /// marker scan could not close, so they never override text evidence.
    pub blank_slots: BlankSlotSpans,
}

/// `line id -> (char start, char end, question number)` insertion spans.
pub(crate) type BlankSlotSpans = std::collections::BTreeMap<String, Vec<(usize, usize, u32)>>;

impl CompletionStructureCandidate {
    pub(crate) fn closes_slots(&self, expected_numbers: &[u32]) -> bool {
        expected_numbers
            .iter()
            .all(|number| self.slot_line_ids.contains_key(number))
    }
}

/// One answer blank the source draws as page *geometry* rather than as text: a
/// short horizontal rule on the writing line.
///
/// A listening form prints its blanks this way — there is no underscore glyph
/// anywhere in the text layer, and one blank per field is exactly one rule of
/// 60-71pt.  Without this evidence the whole section looks like plain prose and
/// the completion task has nowhere to host its slots.
///
/// A rule's page coordinates cannot be compared with the coordinates of the
/// rows the grammar works on: those rows are rebuilt from the V1 document, whose
/// block boxes live in a different vertical space than the physical shadow's
/// rule geometry.  Each rule is therefore resolved here, against the physical
/// shadow's *own* rows, into a row identity plus a position inside that row, so
/// the grammar can act on it without ever mixing the two coordinate systems.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CompletionBlank {
    /// The row the rule sits under with every whitespace run removed.  Both
    /// text layers print the same ink, so this key survives one of them
    /// splitting words at glyph gaps and the other leaving them joined.
    pub line_key: String,
    /// Reading order of the rule: page, then vertical, then horizontal.
    pub page_index: i32,
    pub y: f64,
    /// Where the rule starts *within* the row, as a count of non-whitespace
    /// characters.  `None` means the rule sits past the row's last character,
    /// which is where a form puts a blank after a printed label.
    pub non_space_offset: Option<usize>,
}

/// A rule this short is a bullet leader or a table hairline, not a writing
/// line.  The corpus draws blanks at 60.5-71.5pt; the guard arms keep both a
/// short dash and a long table border out of the blank set.
const MIN_BLANK_RULE_PT: f64 = 30.0;
const MAX_BLANK_RULE_PT: f64 = 120.0;
/// An underline is a flat rule: anything thicker is a box edge or a bar.
const MAX_BLANK_RULE_THICKNESS_PT: f64 = 2.6;

/// Collect the answer blanks a physical `DocumentIRV2` shadow exposes, each one
/// already bound to the shadow row it is drawn under.
///
/// Only axis-aligned flat rules of writing-line length are taken; everything
/// else on the page (borders, separators, rules) is left alone.  The result is
/// sorted by reading order so callers can hand unnumbered blanks to the
/// remaining questions in the order the paper prints them.
pub(crate) fn completion_blanks_from_shadow(shadow: &Value) -> Vec<CompletionBlank> {
    let lines = semantic_lines_from_v2_shadow(shadow);
    if lines.is_empty() {
        return Vec::new();
    }
    let mut blanks = Vec::new();
    for page in shadow
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let page_index = page
            .get("pageIndex")
            .and_then(Value::as_i64)
            .unwrap_or_default() as i32;
        for path in page
            .get("vectorPaths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if path.get("isAxisAlignedRule").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let Some(bbox) = path.get("bbox").and_then(Value::as_object) else {
                continue;
            };
            let (Some(x), Some(y), Some(width), Some(height)) = (
                bbox.get("x").and_then(Value::as_f64),
                bbox.get("y").and_then(Value::as_f64),
                bbox.get("width").and_then(Value::as_f64),
                bbox.get("height").and_then(Value::as_f64),
            ) else {
                continue;
            };
            if height > MAX_BLANK_RULE_THICKNESS_PT
                || width <= MIN_BLANK_RULE_PT
                || width > MAX_BLANK_RULE_PT
            {
                continue;
            }
            let Some(host) = blank_host_line(&lines, page_index, x, width, y) else {
                continue;
            };
            blanks.push(CompletionBlank {
                line_key: strip_whitespace(&host.text),
                page_index,
                y,
                non_space_offset: blank_non_space_offset(host, x),
            });
        }
    }
    blanks.sort_by(|left, right| {
        left.page_index
            .cmp(&right.page_index)
            .then_with(|| left.y.total_cmp(&right.y))
            .then_with(|| left.line_key.cmp(&right.line_key))
    });
    blanks
}

/// The shadow row a rule is drawn under: same page, and the closest row bottom.
fn blank_host_line<'a>(
    lines: &'a [SemanticLine],
    page_index: i32,
    x: f64,
    width: f64,
    y: f64,
) -> Option<&'a SemanticLine> {
    lines
        .iter()
        .filter(|line| blank_hosts_line(line, page_index, y))
        // The rule has to at least reach the row's left edge; a rule that ends
        // before the row starts belongs to something else on the line above.
        .filter(|line| line_box(line).is_some_and(|(x0, _, _, _)| x + width > x0))
        .min_by(|left, right| {
            blank_line_distance(left, y).total_cmp(&blank_line_distance(right, y))
        })
}

pub(crate) fn recover_completion_structure(
    task_type: &TaskTypeV2,
    lines: &[SemanticLine],
    instruction_line_ids: &[String],
    expected_numbers: &[u32],
) -> CompletionStructureCandidate {
    recover_completion_structure_with_blanks(
        task_type,
        lines,
        instruction_line_ids,
        expected_numbers,
        &[],
    )
}

/// Same recovery, but the caller also supplies the page's *drawn* blanks.
///
/// The drawn blanks are consulted only for numbers the printed-marker scan
/// could not close.  A paper whose blanks are glyph runs (`___`, `…`, `□`) or
/// whose numbers are all printed therefore produces byte-identical output to
/// [`recover_completion_structure`]; the extra evidence is a fallback, never a
/// replacement.
pub(crate) fn recover_completion_structure_with_blanks(
    task_type: &TaskTypeV2,
    lines: &[SemanticLine],
    instruction_line_ids: &[String],
    expected_numbers: &[u32],
    blanks: &[CompletionBlank],
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
    let mut blank_slots = BlankSlotSpans::new();
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
    assign_drawn_blank_slots(
        lines,
        expected_numbers,
        blanks,
        &mut slot_line_ids,
        &mut blank_slots,
    );
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
                && !super::question_number::starts_with_question_heading(&lower)
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
        blank_slots,
    }
}

/// Attach the page's drawn blanks to the numbers still missing a host.
///
/// Two shapes occur in the listening corpus and both end up here:
///
/// * the blank sits next to a *printed* number (`Two 2 ____ bedrooms`), in
///   which case that number owns it and the slot consumes the number token;
/// * the paper prints no number on the field at all (`Day of job: ____`), in
///   which case the blanks are handed to the remaining expected numbers in
///   reading order — the same order the paper numbers them.
///
/// Blanks arrive bound to the *shadow* row they were drawn under; a blank only
/// counts here when that row is one of the rows this group actually owns, which
/// is what keeps a rule belonging to another column or another group out.
fn assign_drawn_blank_slots(
    lines: &[SemanticLine],
    expected_numbers: &[u32],
    blanks: &[CompletionBlank],
    slot_line_ids: &mut std::collections::BTreeMap<u32, String>,
    blank_slots: &mut BlankSlotSpans,
) {
    if blanks.is_empty() || expected_numbers.is_empty() {
        return;
    }
    let mut by_key = std::collections::BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        by_key.entry(strip_whitespace(&line.text)).or_insert(index);
    }
    let mut unowned = expected_numbers
        .iter()
        .copied()
        .filter(|number| !slot_line_ids.contains_key(number))
        .collect::<Vec<_>>();
    let mut claimed_lines = std::collections::BTreeSet::new();
    let mut pending = Vec::new();
    for blank in blanks {
        let Some(index) = by_key.get(&blank.line_key).copied() else {
            continue;
        };
        let line = &lines[index];
        if claimed_lines.contains(&line.id) {
            continue;
        }
        // A printed expected number on the host row that the glyph scan could
        // not close owns this blank: the blank *is* its missing blank run.
        let printed = unowned
            .iter()
            .copied()
            .find(|number| !completion_number_marker_spans(&line.text, *number).is_empty());
        match printed {
            Some(number) => {
                let Some((start, end)) = completion_number_marker_spans(&line.text, number)
                    .into_iter()
                    .next()
                else {
                    continue;
                };
                unowned.retain(|candidate| *candidate != number);
                claimed_lines.insert(line.id.clone());
                slot_line_ids
                    .entry(number)
                    .or_insert_with(|| line.id.clone());
                blank_slots
                    .entry(line.id.clone())
                    .or_default()
                    .push((start, end, number));
            }
            // No printed number: defer so the blanks keep their reading order
            // against the remaining questions.
            None => pending.push((line.id.clone(), blank.non_space_offset)),
        }
    }
    for (line_id, non_space_offset) in pending {
        let Some(number) = unowned.first().copied() else {
            break;
        };
        let Some(line) = lines.iter().find(|line| line.id == line_id) else {
            continue;
        };
        let offset = match non_space_offset {
            Some(index) => offset_of_non_space(&line.text, index),
            None => line.text.len(),
        };
        unowned.retain(|candidate| *candidate != number);
        slot_line_ids
            .entry(number)
            .or_insert_with(|| line_id.clone());
        blank_slots
            .entry(line_id)
            .or_default()
            .push((offset, offset, number));
    }
    for spans in blank_slots.values_mut() {
        spans.sort_by_key(|(start, _, _)| *start);
    }
}

/// Row identity that survives the two text layers disagreeing about spaces.
fn strip_whitespace(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

/// Character offset after the `index`-th non-whitespace character of `text`.
fn offset_of_non_space(text: &str, index: usize) -> usize {
    if index == 0 {
        return 0;
    }
    let mut seen = 0usize;
    for (offset, ch) in text.char_indices() {
        if ch.is_whitespace() {
            continue;
        }
        seen += 1;
        if seen == index {
            return offset + ch.len_utf8();
        }
    }
    text.len()
}

/// Whether `line` is a row the rule could be drawn under: same page, and close
/// enough to the row's box bottom.
fn blank_hosts_line(line: &SemanticLine, page_index: i32, y: f64) -> bool {
    line.page_index == page_index && blank_line_distance(line, y) <= BLANK_LINE_BAND_PT
}

/// Distance from the rule's `y` to the bottom of the row's text box.  A written
/// blank is drawn just under the baseline, so the owning row is the one whose
/// box bottom the rule sits closest to.
fn blank_line_distance(line: &SemanticLine, y: f64) -> f64 {
    match line_box(line) {
        Some((_, _, _, bottom)) => (y - bottom).abs(),
        None => f64::INFINITY,
    }
}

/// Rows further than this from a rule are not its owner.  Row pitch on the
/// corpus is ~38pt, so half a pitch cannot reach a neighbouring field.
const BLANK_LINE_BAND_PT: f64 = 18.0;

/// `(x0, top, x1, bottom)` from a shadow row's `[x, y, width, height]` box.
///
/// Only physical-shadow rows reach this helper, and that layer always emits the
/// corner-plus-extent form with a top-left origin, so no guessing is needed
/// between the two encodings the older V1 documents used.
fn line_box(line: &SemanticLine) -> Option<(f64, f64, f64, f64)> {
    let [x, y, width, height] = line.bbox?;
    Some((x, y, x + width.max(0.0), y + height.max(0.0)))
}

/// Where a drawn blank interrupts its row's text: the character whose
/// interpolated horizontal position best matches the rule's left edge, counted
/// as non-whitespace characters so a caller holding a differently spaced copy of
/// the same row can land on the same place.
///
/// `None` means "past the row's text" — either the rule is drawn beyond the last
/// character (the usual form shape, `Day of job: ____`) or the row's own box is
/// left of the rule's start.  A caller turns that into "append at the end".
fn blank_non_space_offset(line: &SemanticLine, blank_x: f64) -> Option<usize> {
    let text = &line.text;
    if text.is_empty() {
        return None;
    }
    let (x0, _, x1, _) = line_box(line)?;
    if blank_x >= x1 || x0 >= blank_x {
        return None;
    }
    let width = (x1 - x0).max(1.0);
    let length = text.len().max(1);
    let mut best: Option<(f64, usize)> = None;
    for (offset, _) in text.char_indices() {
        let estimated = x0 + (offset as f64 / length as f64) * width;
        let distance = (estimated - blank_x).abs();
        if best.is_none_or(|(best_distance, _)| distance < best_distance) {
            best = Some((distance, offset));
        }
    }
    best.map(|(_, offset)| {
        text[..offset]
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .count()
    })
}

fn completion_control_line(text: &str) -> bool {
    let lower = normalize_instruction_text(text).to_ascii_lowercase();
    let lower = lower.trim();
    super::question_number::starts_with_question_heading(lower)
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

/// Combine printed-marker spans with the spans recovered from drawn blanks.
///
/// A printed marker is stronger evidence — it carries the number's own glyphs —
/// so it wins whenever both describe the same question, and numbers are never
/// rendered twice (the quality gate counts one inline host per slot).
fn merge_slot_spans(
    printed: Vec<(usize, usize, u32)>,
    from_blanks: Vec<(usize, usize, u32)>,
) -> Vec<(usize, usize, u32)> {
    let mut merged = printed;
    let mut seen = merged
        .iter()
        .map(|(_, _, number)| *number)
        .collect::<std::collections::BTreeSet<_>>();
    for span in from_blanks {
        if seen.insert(span.2) {
            merged.push(span);
        }
    }
    merged.sort_by_key(|(start, end, _)| (*start, *end));
    merged
}

/// The blank spans that apply to `line`, translated into offsets of `body`
/// (the row text with any leading bullet removed).
fn blank_spans_for_line(
    blank_slots: &BlankSlotSpans,
    line: &SemanticLine,
    bullet_start: usize,
    body_len: usize,
    expected_numbers: &[u32],
) -> Vec<(usize, usize, u32)> {
    let Some(spans) = blank_slots.get(&line.id) else {
        return Vec::new();
    };
    spans
        .iter()
        .filter(|(_, _, number)| expected_numbers.contains(number))
        .map(|(start, end, number)| {
            let start = start.saturating_sub(bullet_start).min(body_len);
            let end = end.saturating_sub(bullet_start).max(start).min(body_len);
            (start, end, *number)
        })
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
    completion_context_nodes_with_slots(
        task_id,
        container_kind,
        lines,
        &[],
        &[],
        "answer",
        &BlankSlotSpans::new(),
    )
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
    blank_slots: &BlankSlotSpans,
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
        if let Some(slot_node) = completion_slot_line_node(
            task_id,
            container_kind,
            line,
            expected_numbers,
            placeholder,
            blank_slots,
        ) {
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
    blank_slots: &BlankSlotSpans,
) -> Option<Value> {
    let text = normalize_instruction_text(&line.text);
    let bullet_start = completion_bullet_body_start(&text).unwrap_or(0);
    let body = &text[bullet_start..];
    let spans = merge_slot_spans(
        completion_slot_marker_spans(body, expected_numbers),
        blank_spans_for_line(blank_slots, line, bullet_start, body.len(), expected_numbers),
    );
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
            &structure.blank_slots,
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
            &candidate.blank_slots,
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
            &bullet_candidate.blank_slots,
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
            &ellipsis_candidate.blank_slots,
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
            &candidate.blank_slots,
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
            &candidate.blank_slots,
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
            &BlankSlotSpans::new(),
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
    // ---------------------------------------------------------------------
    // The drawn-blank channel (F-Q2-1).
    //
    // A listening form prints its blanks as page geometry: the text layer
    // carries the printed question number but no underscore glyph anywhere, so
    // the marker scan finds no host row and every completion group reports
    // `SLOT_HOST_MISSING` with nowhere for the student to type.  These tests
    // cover the fallback evidence and the real paper's product chain.
    // ---------------------------------------------------------------------

    /// A physical-shadow row: corner plus extent, top-left origin.
    fn shadow_row(id: &str, text: &str, x: f64, y: f64, width: f64) -> Value {
        json!({
            "id": id,
            "text": text,
            "bbox": {
                "x": x, "y": y, "width": width, "height": 21.95,
                "unit": "pt", "origin": "top-left", "pageRotation": 0
            }
        })
    }

    fn shadow_rule(x: f64, y: f64, width: f64, height: f64) -> Value {
        json!({
            "isAxisAlignedRule": true,
            "bbox": {"x": x, "y": y, "width": width, "height": height}
        })
    }

    fn shadow_page(page_index: i64, lines: Vec<Value>, rules: Vec<Value>) -> Value {
        json!({"pageIndex": page_index, "lines": lines, "vectorPaths": rules})
    }

    fn count_slot_nodes(value: &Value, slot_id: &str) -> usize {
        match value {
            Value::Array(items) => items
                .iter()
                .map(|item| count_slot_nodes(item, slot_id))
                .sum(),
            Value::Object(object) => {
                let here = usize::from(
                    object.get("type").and_then(Value::as_str) == Some("answer_slot")
                        && object.get("slotId").and_then(Value::as_str) == Some(slot_id),
                );
                here + object
                    .values()
                    .map(|child| count_slot_nodes(child, slot_id))
                    .sum::<usize>()
            }
            _ => 0,
        }
    }

    #[test]
    fn drawn_blank_rules_close_numbers_the_text_layer_cannot() {
        // `Location: in the 1` — the form prints the number and *draws* the
        // blank, so the text layer carries the number but no marker to close it.
        let shadow = json!({"pages": [shadow_page(
            0,
            vec![shadow_row("p001-l0010", "Location:inthe1", 92.04, 527.46, 161.82)],
            vec![shadow_rule(259.4, 552.2, 66.0, 0.01)],
        )]});
        let blanks = completion_blanks_from_shadow(&shadow);
        assert_eq!(blanks.len(), 1, "one writing-line rule, one blank");
        assert_eq!(blanks[0].line_key, "Location:inthe1");
        assert_eq!(blanks[0].page_index, 0);
        assert_eq!(
            blanks[0].non_space_offset, None,
            "the rule is drawn past the row's last character"
        );

        // The grammar's own copy of that row: the same ink, split at word gaps,
        // and deliberately declared on a *different* page number so a coordinate
        // comparison between the two layers could never pass this test.
        let mut row = line("b039", "Location: in the 1");
        row.page_index = 7;
        row.bbox = Some([92.04, 992.47, 368.8, 8.4]);

        let without = recover_completion_structure(
            &TaskTypeV2::FormCompletion,
            std::slice::from_ref(&row),
            &[],
            &[1],
        );
        assert!(
            !without.closes_slots(&[1]),
            "the text-marker scan alone cannot host this blank"
        );

        let with = recover_completion_structure_with_blanks(
            &TaskTypeV2::FormCompletion,
            std::slice::from_ref(&row),
            &[],
            &[1],
            &blanks,
        );
        assert!(with.closes_slots(&[1]), "the drawn blank hosts it");
        assert_eq!(
            with.slot_line_ids.get(&1).map(String::as_str),
            Some("b039")
        );

        let nodes = completion_context_nodes_with_slots(
            "task-drawn",
            with.container_kind,
            &with.context_lines,
            &with.slot_lines,
            &[1],
            "answer",
            &with.blank_slots,
        );
        assert_eq!(nodes.len(), 1, "{nodes:#?}");
        let children = nodes[0]["children"].as_array().expect("row children");
        assert_eq!(children.len(), 2, "{nodes:#?}");
        assert_eq!(children[0]["text"], json!("Location: in the "));
        assert_eq!(children[1]["type"], json!("answer_slot"));
        assert_eq!(children[1]["slotId"], json!("q1"));
        assert_eq!(count_slot_nodes(&nodes[0], "q1"), 1);
    }

    #[test]
    fn a_printed_glyph_blank_is_never_replaced_by_a_drawn_one() {
        // Reading papers print the blank as an underscore run.  A rule can run
        // beside such a row too; the printed marker must keep owning the slot,
        // and the row must not gain a second one.
        let shadow = json!({"pages": [shadow_page(
            0,
            vec![shadow_row("p001-l0001", "Theresultwas31___", 92.0, 300.0, 200.0)],
            vec![shadow_rule(92.0, 324.0, 66.0, 0.01)],
        )]});
        let blanks = completion_blanks_from_shadow(&shadow);
        assert_eq!(blanks.len(), 1, "{blanks:#?}");

        let row = line("b001", "The result was 31 ___");
        let plain = recover_completion_structure(
            &TaskTypeV2::SummaryCompletion,
            std::slice::from_ref(&row),
            &[],
            &[31],
        );
        assert!(plain.closes_slots(&[31]), "glyph marker closes it");
        let with_rule = recover_completion_structure_with_blanks(
            &TaskTypeV2::SummaryCompletion,
            std::slice::from_ref(&row),
            &[],
            &[31],
            &blanks,
        );
        assert!(
            with_rule.blank_slots.is_empty(),
            "printed evidence owns the row"
        );
        assert_eq!(with_rule.slot_line_ids, plain.slot_line_ids);
        assert_eq!(with_rule.slot_lines, plain.slot_lines);

        let render = |structure: &CompletionStructureCandidate| {
            completion_context_nodes_with_slots(
                "task-glyph",
                structure.container_kind,
                &structure.context_lines,
                &structure.slot_lines,
                &[31],
                "answer",
                &structure.blank_slots,
            )
        };
        assert_eq!(
            render(&with_rule),
            render(&plain),
            "a rule must not perturb a row the glyph scan already closed"
        );
        assert_eq!(count_slot_nodes(&render(&with_rule)[0], "q31"), 1);
    }

    #[test]
    fn only_writing_line_rules_become_answer_blanks() {
        let shadow = json!({"pages": [
            shadow_page(
                0,
                vec![
                    shadow_row("p002-l0013", "Dayofjob:", 92.0, 500.0, 130.0),
                    shadow_row("p002-l0016", "Maximumlengthofjob:10", 105.0, 600.0, 203.0),
                ],
                vec![
                    // A bar, not an underline.
                    shadow_rule(92.0, 524.0, 66.0, 6.0),
                    // A short leader.
                    shadow_rule(92.0, 524.0, 20.0, 0.01),
                    // A table border, far wider than a writing line.
                    shadow_rule(92.0, 524.0, 200.0, 0.01),
                    // A rule that stops before the row it would sit near.
                    shadow_rule(5.0, 524.0, 40.0, 0.01),
                    // A rule with no row inside the band.
                    shadow_rule(92.0, 5.0, 66.0, 0.01),
                    // The one real blank.
                    shadow_rule(225.0, 524.0, 66.0, 0.01),
                ],
            ),
            // A rule on a page with no rows cannot be attached to anything.
            shadow_page(1, Vec::new(), vec![shadow_rule(92.0, 10.0, 66.0, 0.01)]),
        ]});
        let blanks = completion_blanks_from_shadow(&shadow);
        assert_eq!(blanks.len(), 1, "{blanks:#?}");
        assert_eq!(blanks[0].line_key, "Dayofjob:");
        assert_eq!(blanks[0].page_index, 0);
        assert_eq!(blanks[0].non_space_offset, None, "rule past the row's text");
        assert!(
            blanks.iter().all(|blank| blank.line_key != "Maximumlengthofjob:10"),
            "the neighbouring row is not this rule's owner"
        );
    }

    /// The real private listening paper after parse + split, plus the handles the
    /// later stages need. `None` when the fixture or pdfium is unavailable, which
    /// is the skip contract the other real-paper probes in this crate use.
    struct RealListeningPaper {
        job: crate::ImportJob,
        source: crate::SourceFile,
        pdf: std::path::PathBuf,
        work_dir: std::path::PathBuf,
        document: Value,
        split: Value,
    }

    fn real_listening_paper() -> Option<RealListeningPaper> {
        let pdf = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/golden/private-real/listening-vol7-t9.pdf");
        if !pdf.exists() || crate::pdf_geometry::pdfium_library_path().is_none() {
            return None;
        }
        let work_dir =
            std::env::temp_dir().join(format!("pdf2test-listening-paper-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&work_dir);
        let mut job = crate::job_store::make_job(crate::CreateJobInput {
            title: Some("Listening real-paper probe".to_string()),
            ..Default::default()
        });
        let source = crate::SourceFile {
            file_id: "listening-vol7-t9".to_string(),
            original_name: "listening-vol7-t9.pdf".to_string(),
            stored_name: "listening-vol7-t9.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: "fixture".to_string(),
            size_bytes: std::fs::metadata(&pdf).map(|meta| meta.len()).unwrap_or(0),
            role: "MainQuestion".to_string(),
            imported_at: chrono::Utc::now(),
        };
        job.source_files = vec![source.clone()];
        let document = crate::parser::parse_source_document(
            &job,
            &source,
            &pdf,
            &work_dir.join("document-ir.json"),
            "auto",
        )
        .ok()?;
        let split = crate::authoring_pipeline::make_dynamic_split_candidates(
            &job.job_id,
            &job,
            Some(&document),
        );
        Some(RealListeningPaper {
            job,
            source,
            pdf,
            work_dir,
            document,
            split,
        })
    }

    /// A split candidate must own exactly the source region it claims. The
    /// option-run recovery passes graft blocks onto `block_ids` *after* the
    /// evidence snapshot, so a candidate whose `sectionEvidence` does not cover
    /// its `block_ids` silently drops those rows: the group reads as a truncated
    /// option bank (group-6 lost E-G and group-5 lost six rows) and the dropped
    /// rows then count as unassigned source.
    #[test]
    fn real_listening_split_evidence_covers_every_claimed_block() {
        let Some(paper) = real_listening_paper() else {
            return;
        };
        let candidates = paper
            .split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(!candidates.is_empty(), "the paper splits into question groups");
        let mut violations = Vec::new();
        for candidate in &candidates {
            let block_ids = candidate
                .get("blockIds")
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let covered = candidate
                .get("sectionEvidence")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|evidence| evidence.get("blockId").and_then(Value::as_str))
                .collect::<std::collections::BTreeSet<_>>();
            let missing = block_ids
                .iter()
                .filter(|id| !covered.contains(id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                violations.push(json!({
                    "range": candidate.get("questionRange").cloned().unwrap_or(Value::Null),
                    "kindHint": candidate.get("kindHint").cloned().unwrap_or(Value::Null),
                    "missingFromSectionEvidence": missing
                }));
            }
        }
        assert!(
            violations.is_empty(),
            "{}",
            serde_json::to_string_pretty(&violations).unwrap_or_default()
        );
    }


    /// Runs the real listening paper through the product chain and returns the
    /// built authoring document, or `None` when the private fixture or pdfium is
    /// unavailable (the other real-paper probes in this crate skip the same way).
    fn build_real_listening_shadow() -> Option<Value> {
        let RealListeningPaper {
            job,
            source,
            pdf,
            work_dir,
            document,
            split,
        } = real_listening_paper()?;
        let v1 = crate::authoring_pipeline::make_dynamic_authoring_ir(&job, &split, Some(&document));
        let physical = crate::pdf_facts_shadow::write_pdf_facts_shadow_with_v1(
            &job,
            &source,
            &pdf,
            &work_dir.join("document-ir-v2.shadow.json"),
            Some(&document),
        )
        .ok()?;
        crate::ielts_grammar::build_authoring_v2_shadow_for_modality(
            &job,
            &v1,
            &split,
            Some(&document),
            Some(&physical),
            crate::schema::ielts_authoring_v2::ExamModalityV2::Listening,
        )
        .ok()
    }

    /// Real-paper acceptance for the drawn-blank channel: through the product
    /// chain, every completion row of the listening form must host exactly one
    /// inline answer slot, so a student can answer all forty questions.
    ///
    /// Skips when the private fixture or pdfium is unavailable, like the other
    /// real-paper probes in this crate.
    #[test]
    fn real_listening_form_rows_host_their_answer_slots() {
        let Some(authoring) = build_real_listening_shadow() else {
            return;
        };
        let groups = authoring
            .get("taskGroups")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let slot_ids_of = |group: &Value| {
            group
                .get("responseGroups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|response| {
                    response
                        .get("slotIds")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        };
        let mut detail = Vec::new();
        let mut violations = Vec::new();
        let mut hosted = std::collections::BTreeSet::new();
        for group in &groups {
            let task_id = group
                .get("taskId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let task_type = group
                .get("taskType")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let slot_ids = slot_ids_of(group);
            hosted.extend(slot_ids.iter().cloned());
            let stimulus = group.get("stimulus").cloned().unwrap_or(Value::Null);
            let misses = if task_type.ends_with("_completion") {
                slot_ids
                    .iter()
                    .filter(|slot_id| count_slot_nodes(&stimulus, slot_id) != 1)
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            detail.push(json!({
                "taskId": task_id,
                "taskType": task_type,
                "slotIds": slot_ids,
                "inlineSlotMisses": misses
            }));
            if !misses.is_empty() {
                violations.push(json!({"taskId": task_id, "missingInlineSlots": misses}));
            }
        }

        assert!(
            violations.is_empty(),
            "every completion row needs its own inline answer slot: {}",
            serde_json::to_string_pretty(&json!({
                "violations": violations,
                "groups": detail
            }))
            .unwrap_or_default()
        );

        let expected_all = (1..=40).map(|number| format!("q{number}")).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            hosted, expected_all,
            "the paper declares forty answerable questions"
        );

        let hard_failures = authoring
            .pointer("/quality/hardFailures")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        eprintln!(
            "listening-vol7-t9 inline slots: {} groups, {} completion groups, {} misses; hard failures {:?}",
            groups.len(),
            detail
                .iter()
                .filter(|entry| entry["taskType"]
                    .as_str()
                    .is_some_and(|kind| kind.ends_with("_completion")))
                .count(),
            violations.len(),
            hard_failures
        );
        assert!(
            !hard_failures
                .iter()
                .any(|code| code.as_str() == Some("SLOT_HOST_MISSING")),
            "drawn blanks must clear SLOT_HOST_MISSING; remaining failures: {hard_failures:?}"
        );
    }

    /// `Questions 17-20 Choose FOUR correct answers, A-F` and `Questions 21-25
    /// Choose FIVE correct letters, A-G` are feature matches against one shared
    /// bank. Through the product chain each of those groups must
    ///   * be typed `matching_features`,
    ///   * carry one option bank whose labels are exactly the declared alphabet,
    ///   * bind every response group to that bank, and
    ///   * leave no option- or type-related blocker behind.
    ///
    /// Without it q17-q25 render no control at all, so the paper can never be
    /// published no matter how many answers the user types.
    #[test]
    fn real_listening_choose_n_groups_bind_their_shared_option_bank() {
        let Some(authoring) = build_real_listening_shadow() else {
            return;
        };
        let groups = authoring
            .get("taskGroups")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut observed = Vec::new();
        let mut violations = Vec::new();
        for group in &groups {
            let start = group
                .pointer("/displayRange/start")
                .and_then(Value::as_u64);
            let end = group.pointer("/displayRange/end").and_then(Value::as_u64);
            let declared = match (start, end) {
                (Some(17), Some(20)) => "A-F",
                (Some(21), Some(25)) => "A-G",
                _ => continue,
            };
            let task_id = group
                .get("taskId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let task_type = group
                .get("taskType")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let labels = group
                .pointer("/optionBank/options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| option.get("label").and_then(Value::as_str))
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let option_texts = group
                .pointer("/optionBank/options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| {
                            let label = option.get("label").and_then(Value::as_str)?;
                            let text = option
                                .pointer("/content/0/text")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            Some(format!("{label}={text}"))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let expected_labels = ('A'..=declared.chars().last().unwrap())
                .map(|label| label.to_string())
                .collect::<Vec<_>>();
            let responses = group
                .get("responseGroups")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let bank_bound = !responses.is_empty()
                && responses
                    .iter()
                    .all(|response| response.get("optionBankRef").is_some());
            let shown = json!({
                "taskId": task_id,
                "taskType": task_type,
                "expectedLabels": expected_labels,
                "optionLabels": labels,
                "optionTexts": option_texts,
                "bankBound": bank_bound
            });
            observed.push(shown.clone());
            if task_type != "matching_features" || labels != expected_labels || !bank_bound {
                violations.push(shown);
            }
        }

        assert_eq!(
            observed.len(),
            2,
            "the paper has exactly two choose-N groups: {observed:?}"
        );
        assert!(
            violations.is_empty(),
            "{}",
            serde_json::to_string_pretty(&json!({
                "violations": violations,
                "observed": observed
            }))
            .unwrap_or_default()
        );

        let codes = authoring
            .pointer("/quality/hardFailures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        for blocked in [
            "TASK_TYPE_CONFLICT",
            "OPTION_RUN_INCOMPLETE",
            "OPTION_BANK_MISSING",
            "OPTION_ALPHABET_MISMATCH",
            "RESPONSE_GROUP_POLICY_MISMATCH",
        ] {
            assert!(
                !codes.contains(&blocked),
                "{blocked} still blocks the paper: {codes:?}"
            );
        }
        eprintln!("listening-vol7-t9 choose-N groups: {observed:?}; hard failures {codes:?}");
    }

}
