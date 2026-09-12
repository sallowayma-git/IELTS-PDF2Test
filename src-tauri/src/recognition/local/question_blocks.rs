//! Geometric question-block assembly (plan §6.3–§6.7).
//!
//! Pipeline implemented here, in the order §6.3 mandates:
//!
//! ```text
//! DocumentIRV2
//!   -> region role segmentation            (segment_region_roles)
//!   -> instruction zone detection          (detect_instruction_zones)
//!   -> question number token detection     (detect_number_tokens)   token-first, not line-first
//!   -> geometric question-block expansion  (assemble_blocks)
//!   -> local option-run detection          (detect_option_run_from_lines)
//!   -> shared option-bank detection        (collect_option_banks)
//!   -> visual stimulus attachment          (collect_visual_stimuli)
//!   -> unassigned evidence ledger          (collect_unassigned_evidence)
//! ```
//!
//! Task-type classification is deliberately absent: §6.3 requires question
//! boundaries to be recovered before anything is called single-choice, matching
//! or completion.

use super::{
    InstructionZoneCandidate, LayoutBuild, OptionBankCandidateV1, OptionCandidateV2,
    OptionRunCandidateV2, PageLayoutGraph, QuestionBlockCandidateV1, QuestionNumberToken,
    RegionLayoutNode, SemanticRegionRole, UnassignedEvidence, VisualStimulusCandidateV1,
};
use crate::ielts_grammar::issue_codes;
use crate::ielts_grammar::question_number::parse_question_expression_detailed;
use crate::schema::common::{RectV2, SourceAnchorV2};
use crate::schema::document_ir_v2::{
    DocumentIRV2, LineNodeV2, PageNodeV2, PhysicalRegionKindV2, RegionNodeV2,
};
use std::collections::BTreeSet;

/// §6.5: tokens below this score are dropped rather than guessed at.
const NUMBER_TOKEN_MIN_SCORE: f64 = 0.55;
/// §6.6 `within_baseline_tolerance`.
const BASELINE_TOLERANCE_PT: f64 = 5.0;
/// §6.6 `vertical_gap_below_threshold`.
const VERTICAL_GAP_LIMIT_PT: f64 = 34.0;
/// Text shorter than this is not "significant" evidence worth reporting.
const MIN_SIGNIFICANT_CHARS: usize = 12;

// ---------------------------------------------------------------- geometry ---

fn right_of(rect: &RectV2) -> f64 {
    rect.x + rect.width
}

fn bottom_of(rect: &RectV2) -> f64 {
    rect.y + rect.height
}

fn vertical_overlap(a: &RectV2, b: &RectV2) -> f64 {
    ((a.y + a.height).min(b.y + b.height) - a.y.max(b.y)).max(0.0)
}

fn horizontal_overlap(a: &RectV2, b: &RectV2) -> f64 {
    ((a.x + a.width).min(b.x + b.width) - a.x.max(b.x)).max(0.0)
}

fn same_row(a: &RectV2, b: &RectV2) -> bool {
    vertical_overlap(a, b) >= (a.height.min(b.height) * 0.5).max(1.0)
}

fn contains_point(rect: &RectV2, x: f64, y: f64) -> bool {
    x >= rect.x && x <= right_of(rect) && y >= rect.y && y <= bottom_of(rect)
}

fn area_of(rect: &RectV2) -> f64 {
    rect.width.max(0.0) * rect.height.max(0.0)
}

// ------------------------------------------------------------- text lookup ---

fn line_text_of<'a>(page: &'a PageNodeV2, line_id: &str) -> Option<&'a str> {
    page.lines
        .iter()
        .find(|line| line.id == line_id)
        .map(|line| line.text.as_str())
}

pub(super) fn region_text(page: &PageNodeV2, region: &RegionNodeV2) -> String {
    region
        .child_line_ids
        .iter()
        .filter_map(|line_id| line_text_of(page, line_id))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn line_for_span<'a>(page: &'a PageNodeV2, span_id: &str) -> Option<&'a LineNodeV2> {
    page.lines
        .iter()
        .find(|line| line.span_ids.iter().any(|id| id == span_id))
}

/// The line's text with the named span's glyphs removed, so an inline
/// `5 Which ...` stem does not repeat its own question number.
fn line_text_without_span(page: &PageNodeV2, line: &LineNodeV2, span_id: &str) -> String {
    let Some(span) = page.spans.iter().find(|span| span.id == span_id) else {
        return line.text.trim().to_string();
    };
    if span.text.trim().is_empty() {
        return line.text.trim().to_string();
    }
    let mut text = line.text.clone();
    if let Some(position) = text.find(span.text.as_str()) {
        let end = position + span.text.len();
        if text.is_char_boundary(position) && text.is_char_boundary(end) {
            text.replace_range(position..end, "");
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn first_source_anchor(page: &PageNodeV2, line_ids: &[String]) -> Option<SourceAnchorV2> {
    line_ids.iter().find_map(|line_id| {
        page.lines
            .iter()
            .find(|line| line.id == *line_id)
            .and_then(|line| line.source_anchors.first().cloned())
    })
}

fn span_source_anchor(page: &PageNodeV2, span_id: &str) -> Option<SourceAnchorV2> {
    page.spans
        .iter()
        .find(|span| span.id == span_id)
        .and_then(|span| span.source_anchors.first().cloned())
}

// ------------------------------------------------------------ token parsing ---

/// A "small numeric token": 1–3 digits forming 1..=999. Four-digit tokens are
/// almost always years in IELTS papers, and §6.5 rejects them explicitly.
fn parse_small_numeric_token(text: &str) -> Option<u32> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 3 || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value = trimmed.parse::<u32>().ok()?;
    (1..=999).contains(&value).then_some(value)
}

fn is_roman_numeral(value: &str) -> bool {
    matches!(
        value,
        "i" | "ii" | "iii" | "iv" | "v" | "vi" | "vii" | "viii" | "ix" | "x"
    )
}

fn classify_option_label(candidate: &str) -> Option<String> {
    if candidate.chars().count() == 1 {
        let ch = candidate.chars().next()?;
        return ch
            .is_ascii_alphabetic()
            .then(|| ch.to_ascii_uppercase().to_string());
    }
    let lower = candidate.to_ascii_lowercase();
    is_roman_numeral(&lower).then_some(lower)
}

/// Longest-label-first so `ii` is never misread as `i`.
fn leading_option_label(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    for length in (1..=4).rev() {
        let Some(candidate) = trimmed.get(..length) else {
            continue;
        };
        let Some(rest_raw) = trimmed.get(length..) else {
            continue;
        };
        let rest_trimmed = rest_raw.trim_start();
        let separated = rest_trimmed.len() != rest_raw.len()
            || rest_raw.starts_with(['.', ')', ':', ']', '-']);
        if !separated {
            continue;
        }
        let Some(label) = classify_option_label(candidate) else {
            continue;
        };
        let body = rest_trimmed
            .trim_start_matches(['.', ')', ':', ']', '-'])
            .trim()
            .to_string();
        return Some((label, body));
    }
    None
}

fn label_rank(label: &str) -> Option<u32> {
    if label.chars().count() == 1 {
        let ch = label.chars().next()?;
        return ch.is_ascii_uppercase().then(|| ch as u32 - 'A' as u32);
    }
    Some(match label {
        "i" => 0,
        "ii" => 1,
        "iii" => 2,
        "iv" => 3,
        "v" => 4,
        "vi" => 5,
        "vii" => 6,
        "viii" => 7,
        "ix" => 8,
        "x" => 9,
        _ => return None,
    })
}

// --------------------------------------------------------- region roles (§6.4) ---

fn is_chrome(kind: &PhysicalRegionKindV2) -> bool {
    matches!(
        kind,
        PhysicalRegionKindV2::Header
            | PhysicalRegionKindV2::Footer
            | PhysicalRegionKindV2::PageNumber
    )
}

fn is_visual_kind(kind: &PhysicalRegionKindV2) -> bool {
    matches!(
        kind,
        PhysicalRegionKindV2::Figure | PhysicalRegionKindV2::Diagram | PhysicalRegionKindV2::Table
    )
}

fn instruction_marker(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    for marker in [
        "write ",
        "choose ",
        "complete ",
        "do the following",
        "select ",
        "match ",
        "label the",
        "answer the",
        "from the list",
        "according to",
        "in boxes",
        "true",
        "false",
        "not given",
        "yes",
        "no",
    ] {
        if lower.contains(marker) {
            return Some(marker);
        }
    }
    None
}

/// Longest consecutive label run (A/B/C… or i/ii/iii…) with non-empty bodies.
/// Requires at least three labels: a lone `A` is a paragraph label or a passage
/// initial, not an answer option (§6.7).
fn detect_option_run_from_lines<'a>(
    lines: impl IntoIterator<Item = &'a LineNodeV2>,
) -> Option<Vec<OptionCandidateV2>> {
    let mut best: Vec<OptionCandidateV2> = Vec::new();
    let mut current: Vec<OptionCandidateV2> = Vec::new();
    let mut expected_rank: Option<u32> = None;

    for line in lines {
        let parsed = leading_option_label(&line.text);
        match parsed {
            Some((label, body)) if !body.trim().is_empty() => {
                let rank = label_rank(&label);
                let continues = match (expected_rank, rank) {
                    (Some(expected), Some(rank)) => rank == expected,
                    (None, Some(_)) => true,
                    _ => false,
                };
                if !continues {
                    if current.len() > best.len() {
                        best = std::mem::take(&mut current);
                    } else {
                        current.clear();
                    }
                }
                current.push(OptionCandidateV2 {
                    label,
                    label_node_id: line.id.clone(),
                    text: body,
                    text_node_ids: vec![line.id.clone()],
                    bbox: line.bbox.clone(),
                });
                expected_rank = rank.map(|rank| rank + 1);
            }
            _ => {
                if current.len() > best.len() {
                    best = std::mem::take(&mut current);
                } else {
                    current.clear();
                }
                expected_rank = None;
            }
        }
    }
    if current.len() > best.len() {
        best = current;
    }
    (best.len() >= 3).then_some(best)
}

fn region_lines<'a>(page: &'a PageNodeV2, region: &RegionNodeV2) -> Vec<&'a LineNodeV2> {
    region
        .child_line_ids
        .iter()
        .filter_map(|line_id| page.lines.iter().find(|line| line.id == *line_id))
        .collect()
}

fn score_region_role(
    page: &PageNodeV2,
    region: &RegionNodeV2,
    text: &str,
    option_run: bool,
) -> (SemanticRegionRole, f64, Vec<String>) {
    let mut features: Vec<String> = Vec::new();
    let mut scores: Vec<(SemanticRegionRole, f64)> = Vec::new();
    let lower = text.to_ascii_lowercase();
    let line_count = region.child_line_ids.len().max(1);
    let number_leading_lines = region
        .child_line_ids
        .iter()
        .filter(|line_id| {
            line_text_of(page, line_id)
                .and_then(|text| text.split_whitespace().next())
                .and_then(parse_small_numeric_token)
                .is_some()
        })
        .count() as f64;
    let number_density = number_leading_lines / line_count as f64;

    if is_chrome(&region.kind) {
        features.push("region_kind_chrome".to_string());
        scores.push((SemanticRegionRole::HeaderFooter, 1.0));
    } else {
        let near_edge =
            region.bbox.y <= page.height_pt * 0.08 || bottom_of(&region.bbox) >= page.height_pt * 0.92;
        if near_edge && text.chars().count() < 80 {
            features.push("page_edge_short_text".to_string());
            scores.push((SemanticRegionRole::HeaderFooter, 0.55));
        }
    }

    if lower.contains("answer key") || lower.starts_with("answers") {
        features.push("answer_key_signature".to_string());
        scores.push((SemanticRegionRole::AnswerKey, 0.9));
    }

    if lower.contains("question") {
        features.push("questions_word".to_string());
        let mut score = 0.5;
        if let Some(marker) = instruction_marker(text) {
            features.push(format!("instruction_marker:{marker}"));
            score += 0.25;
        }
        if parse_question_expression_detailed(text).is_some() {
            features.push("question_range".to_string());
            score += 0.2;
        }
        scores.push((SemanticRegionRole::QuestionInstruction, score));
    }

    if lower.contains("list of headings") {
        features.push("list_of_headings".to_string());
        scores.push((SemanticRegionRole::SharedOptionBank, 0.9));
    }

    if option_run {
        features.push("option_label_run".to_string());
        scores.push((SemanticRegionRole::OptionRun, 0.75));
    }

    if number_density >= 0.25 {
        features.push("question_number_density".to_string());
        scores.push((
            SemanticRegionRole::QuestionPrompt,
            0.35 + number_density.min(0.4),
        ));
    }

    if is_visual_kind(&region.kind) {
        features.push("visual_region_kind".to_string());
        scores.push((SemanticRegionRole::CompletionStimulus, 0.6));
    }

    let average_line_length = if text.is_empty() {
        0.0
    } else {
        text.chars().count() as f64 / line_count as f64
    };
    if average_line_length >= 45.0
        && !option_run
        && !lower.contains("question")
        && !is_visual_kind(&region.kind)
    {
        features.push("long_prose_run".to_string());
        scores.push((SemanticRegionRole::Passage, 0.4));
    }

    let chosen = scores
        .into_iter()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(role, score)| (role, score.clamp(0.0, 1.0)))
        .unwrap_or((SemanticRegionRole::Unknown, 0.0));
    (chosen.0, chosen.1, features)
}

fn segment_region_roles(page: &PageNodeV2) -> Vec<RegionLayoutNode> {
    page.regions
        .iter()
        .map(|region| {
            let text = region_text(page, region);
            let option_run = detect_option_run_from_lines(region_lines(page, region)).is_some();
            let (role, role_confidence, role_features) =
                score_region_role(page, region, &text, option_run);
            RegionLayoutNode {
                region_id: region.id.clone(),
                kind: region.kind.clone(),
                role,
                role_confidence,
                role_features,
                bbox: region.bbox.clone(),
                child_line_ids: region.child_line_ids.clone(),
            }
        })
        .collect()
}

// ------------------------------------------------------- instruction zones ---

fn task_hint_from_text(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let hint = if lower.contains("heading") {
        "matching_headings"
    } else if lower.contains("true") && lower.contains("false") {
        "true_false_not_given"
    } else if lower.contains("yes") && lower.contains("no") {
        "yes_no_not_given"
    } else if lower.contains("choose the correct letter") {
        "single_choice"
    } else if lower.contains("label the diagram") {
        "diagram_label"
    } else if lower.contains("complete the") {
        "completion"
    } else if lower.contains("match") {
        "matching"
    } else {
        return None;
    };
    Some(hint.to_string())
}

fn detect_instruction_zones(
    page: &PageNodeV2,
    regions: &[RegionLayoutNode],
) -> Vec<InstructionZoneCandidate> {
    regions
        .iter()
        .filter(|region| region.role == SemanticRegionRole::QuestionInstruction)
        .enumerate()
        .map(|(index, region)| {
            let text = page
                .regions
                .iter()
                .find(|candidate| candidate.id == region.region_id)
                .map(|candidate| region_text(page, candidate))
                .unwrap_or_default();
            let parsed = parse_question_expression_detailed(&text);
            let expected_numbers = parsed
                .as_ref()
                .map(|parsed| parsed.numbers.clone())
                .unwrap_or_default();
            let question_range = expected_numbers
                .first()
                .zip(expected_numbers.last())
                .map(|(first, last)| [*first, *last]);
            InstructionZoneCandidate {
                zone_id: format!("zone-{}-{}", page.page_index, index + 1),
                page_index: page.page_index,
                region_id: Some(region.region_id.clone()),
                question_range,
                expected_numbers,
                task_hint: task_hint_from_text(&text),
                text,
                source_anchor: first_source_anchor(page, &region.child_line_ids),
                confidence: region.role_confidence,
            }
        })
        .collect()
}

// -------------------------------------------------- question number tokens ---

fn looks_like_measurement_context(page: &PageNodeV2, line: &LineNodeV2, span_id: &str) -> bool {
    let Some(span) = page.spans.iter().find(|span| span.id == span_id) else {
        return false;
    };
    let after = line
        .text
        .find(span.text.as_str())
        .map(|position| &line.text[position + span.text.len()..])
        .unwrap_or("");
    let after = after.trim_start();
    after.starts_with('%')
        || after.starts_with('£')
        || after.starts_with('$')
        || after.starts_with("per ")
        || after.starts_with("kg")
        || after.starts_with("km")
        || after.starts_with("cm")
        || after.starts_with("mm")
}

struct NumberTokenInputs<'a> {
    page: &'a PageNodeV2,
    token_line: &'a LineNodeV2,
    token_id: &'a str,
    value: u32,
    expected: Option<&'a [u32]>,
    token_font_size: Option<f64>,
    median_font_size: Option<f64>,
    /// True when the token sits in the question area — either the prompt block or
    /// that question's own option run. Single-choice prompts and their options are
    /// one physical region, so both count as "the left edge of the question area".
    in_question_area: bool,
    in_passage_region: bool,
}

/// §6.5 additive scoring. The weights mirror the plan so a reviewer can diff them.
fn score_number_token(inputs: &NumberTokenInputs<'_>) -> (f64, Vec<String>) {
    let mut score = 0.0;
    let mut features = Vec::new();

    if inputs
        .expected
        .is_some_and(|expected| expected.contains(&inputs.value))
    {
        score += 0.30;
        features.push("in_instruction_range".to_string());
    }

    if inputs.in_question_area {
        score += 0.15;
        features.push("question_area_left_edge".to_string());
    }

    let has_neighbour_text = inputs.page.lines.iter().any(|line| {
        line.id != inputs.token_line.id
            && same_row(&line.bbox, &inputs.token_line.bbox)
            && line.bbox.x > right_of(&inputs.token_line.bbox)
            && !line.text.trim().is_empty()
    });
    if has_neighbour_text || inputs.token_line.text.trim().chars().count() > 3 {
        score += 0.10;
        features.push("text_adjacent".to_string());
    }

    if let (Some(token_size), Some(median_size)) =
        (inputs.token_font_size, inputs.median_font_size)
    {
        if (token_size - median_size).abs() <= 0.75 {
            score += 0.10;
            features.push("consistent_number_font".to_string());
        }
    }

    if inputs.in_passage_region {
        score -= 0.20;
        features.push("inside_passage".to_string());
    }
    if (1900..=2100).contains(&inputs.value) {
        score -= 0.25;
        features.push("looks_like_year".to_string());
    }
    if looks_like_measurement_context(inputs.page, inputs.token_line, inputs.token_id) {
        score -= 0.25;
        features.push("looks_like_measurement".to_string());
    }

    (score, features)
}

fn detect_number_tokens(
    page: &PageNodeV2,
    regions: &[RegionLayoutNode],
    expected: Option<&[u32]>,
) -> Vec<QuestionNumberToken> {
    let mut font_sizes: Vec<f64> = page
        .spans
        .iter()
        .filter(|span| parse_small_numeric_token(&span.text).is_some())
        .filter_map(|span| span.style.font_size_pt)
        .collect();
    font_sizes.sort_by(f64::total_cmp);
    let median_font_size = font_sizes.get(font_sizes.len() / 2).copied();

    let mut tokens: Vec<QuestionNumberToken> = Vec::new();
    for span in &page.spans {
        let Some(value) = parse_small_numeric_token(&span.text) else {
            continue;
        };
        let Some(token_line) = line_for_span(page, &span.id) else {
            continue;
        };

        let center_x = span.bbox.x + span.bbox.width / 2.0;
        let center_y = span.bbox.y + span.bbox.height / 2.0;
        let owning_region = regions
            .iter()
            .find(|region| contains_point(&region.bbox, center_x, center_y));

        if owning_region.is_some_and(|region| region.role == SemanticRegionRole::HeaderFooter) {
            continue;
        }
        let in_question_area = owning_region.is_some_and(|region| {
            matches!(
                region.role,
                SemanticRegionRole::QuestionPrompt | SemanticRegionRole::OptionRun
            )
        });
        let in_passage_region =
            owning_region.is_some_and(|region| region.role == SemanticRegionRole::Passage);

        let (score, features) = score_number_token(&NumberTokenInputs {
            page,
            token_line,
            token_id: &span.id,
            value,
            expected,
            token_font_size: span.style.font_size_pt,
            median_font_size,
            in_question_area,
            in_passage_region,
        });
        tokens.push(QuestionNumberToken {
            value,
            node_id: span.id.clone(),
            page_index: page.page_index,
            bbox: span.bbox.clone(),
            score,
            features,
        });
    }

    // Second pass: a monotonic neighbour is strong evidence (§6.5 +0.20). Applied
    // after sorting by reading position so "the next question number" is defined.
    tokens.sort_by(|left, right| {
        left.bbox
            .y
            .total_cmp(&right.bbox.y)
            .then(left.bbox.x.total_cmp(&right.bbox.x))
    });
    let mut previous: Option<u32> = None;
    for token in tokens.iter_mut() {
        if previous.is_some_and(|previous| previous + 1 == token.value) {
            token.score += 0.20;
            token.features.push("monotonic_sequence".to_string());
        }
        previous = Some(token.value);
    }
    tokens.retain(|token| token.score >= NUMBER_TOKEN_MIN_SCORE);
    tokens
}

// --------------------------------------------------------- block expansion ---

struct StemAssembly {
    node_ids: Vec<String>,
    text: String,
    boundary_confidence: f64,
    ambiguities: Vec<String>,
}

fn assemble_stem(
    page: &PageNodeV2,
    token: &QuestionNumberToken,
    next_token: Option<&QuestionNumberToken>,
    reserved: &BTreeSet<String>,
) -> StemAssembly {
    let Some(token_line) = line_for_span(page, &token.node_id) else {
        return StemAssembly {
            node_ids: Vec::new(),
            text: String::new(),
            boundary_confidence: 0.2,
            ambiguities: vec![issue_codes::PROMPT_EMPTY.to_string()],
        };
    };

    let mut node_ids: Vec<String> = Vec::new();
    let mut fragments: Vec<String> = Vec::new();
    let mut ambiguities: Vec<String> = Vec::new();
    let mut confidence = 1.0f64;

    let interval_bottom = next_token
        .map(|next| next.bbox.y)
        .unwrap_or(page.height_pt);

    // Same row: the token's own line (inline stem) then text nodes to its right.
    let inline = line_text_without_span(page, token_line, &token.node_id);
    if !inline.is_empty() {
        node_ids.push(token_line.id.clone());
        fragments.push(inline);
    }
    let mut same_row_lines: Vec<&LineNodeV2> = page
        .lines
        .iter()
        .filter(|line| {
            line.id != token_line.id
                && same_row(&line.bbox, &token_line.bbox)
                && line.bbox.x >= right_of(&token_line.bbox) - BASELINE_TOLERANCE_PT
                && !line.text.trim().is_empty()
                && !reserved.contains(&line.id)
                && leading_option_label(&line.text).is_none()
        })
        .collect();
    same_row_lines.sort_by(|left, right| left.bbox.x.total_cmp(&right.bbox.x));
    for line in &same_row_lines {
        node_ids.push(line.id.clone());
        fragments.push(line.text.trim().to_string());
    }

    // Wrapped continuation: keep taking lines below until the boundary breaks.
    let anchor = same_row_lines.last().copied().unwrap_or(token_line);
    let mut baseline_bottom = bottom_of(&anchor.bbox);
    let mut below: Vec<&LineNodeV2> = page
        .lines
        .iter()
        .filter(|line| {
            line.bbox.y >= baseline_bottom - BASELINE_TOLERANCE_PT && line.bbox.y < interval_bottom
        })
        .collect();
    below.sort_by(|left, right| left.bbox.y.total_cmp(&right.bbox.y));
    for line in below {
        if line.id == anchor.id || node_ids.contains(&line.id) {
            continue;
        }
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        // A shared option bank or the answer key is never part of a question
        // stem, even when the previous question's interval is still open.
        if reserved.contains(&line.id) {
            break;
        }
        let gap = line.bbox.y - baseline_bottom;
        if gap > VERTICAL_GAP_LIMIT_PT {
            break;
        }
        if leading_option_label(text).is_some() {
            break;
        }
        // A line that is only a question number ends the previous stem.
        let is_bare_number_line = page.spans.iter().any(|span| {
            line.span_ids.contains(&span.id) && parse_small_numeric_token(&span.text).is_some()
        });
        if is_bare_number_line {
            break;
        }
        if horizontal_overlap(&line.bbox, &anchor.bbox) <= 0.0 {
            ambiguities.push(issue_codes::PROMPT_BOUNDARY_AMBIGUOUS.to_string());
            confidence -= 0.15;
            break;
        }
        node_ids.push(line.id.clone());
        fragments.push(text.to_string());
        baseline_bottom = bottom_of(&line.bbox);
    }

    let text = fragments
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if text.is_empty() || node_ids.is_empty() {
        if !ambiguities.iter().any(|code| code == issue_codes::PROMPT_EMPTY) {
            ambiguities.push(issue_codes::PROMPT_EMPTY.to_string());
        }
        confidence = confidence.min(0.2);
    }

    StemAssembly {
        node_ids,
        text,
        boundary_confidence: confidence.clamp(0.0, 1.0),
        ambiguities,
    }
}

fn assemble_blocks(
    page: &PageNodeV2,
    tokens: &[QuestionNumberToken],
    reserved: &BTreeSet<String>,
    consumed: &mut BTreeSet<String>,
) -> Vec<QuestionBlockCandidateV1> {
    let mut blocks = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let next_token = tokens.get(index + 1);
        let stem = assemble_stem(page, token, next_token, reserved);
        let interval_bottom = next_token
            .map(|next| next.bbox.y)
            .unwrap_or(page.height_pt);

        let interval_lines: Vec<&LineNodeV2> = page
            .lines
            .iter()
            .filter(|line| {
                line.bbox.y >= token.bbox.y - BASELINE_TOLERANCE_PT
                    && line.bbox.y < interval_bottom
                    && !line.text.trim().is_empty()
                    && !reserved.contains(&line.id)
            })
            .collect();

        // A local option run belongs to this block when it sits inside the interval.
        let option_run = detect_option_run_from_lines(interval_lines.iter().copied()).map(|options| {
            OptionRunCandidateV2 {
                run_id: format!("run-{}-{}", page.page_index, token.value),
                labels: options.iter().map(|option| option.label.clone()).collect(),
                label_column_x: options.first().map(|option| option.bbox.x),
                options,
                confidence: 0.8,
            }
        });

        for node_id in stem.node_ids.iter() {
            consumed.insert(node_id.clone());
        }
        if let Some(run) = option_run.as_ref() {
            for option in &run.options {
                for node_id in &option.text_node_ids {
                    consumed.insert(node_id.clone());
                }
            }
        }

        let interval_area: f64 = interval_lines.iter().map(|line| area_of(&line.bbox)).sum();
        let assigned_area: f64 = interval_lines
            .iter()
            .filter(|line| consumed.contains(&line.id))
            .map(|line| area_of(&line.bbox))
            .sum();
        let source_coverage = if interval_area <= 0.0 {
            1.0
        } else {
            (assigned_area / interval_area).clamp(0.0, 1.0)
        };

        let mut boundary_confidence = stem.boundary_confidence;
        if option_run.is_some() {
            boundary_confidence = (boundary_confidence + 0.1).clamp(0.0, 1.0);
        }

        blocks.push(QuestionBlockCandidateV1 {
            candidate_id: format!("block-{}-{}", page.page_index, token.value),
            question_number: token.value,
            page_index: page.page_index,
            number_anchor: span_source_anchor(page, &token.node_id),
            number_bbox: token.bbox.clone(),
            stem_node_ids: stem.node_ids,
            stem_text: stem.text,
            option_run,
            shared_option_bank_ref: None,
            visual_object_refs: Vec::new(),
            source_coverage,
            boundary_confidence,
            ambiguities: stem.ambiguities,
        });
    }
    blocks
}

// ------------------------------------------------------------ option banks ---

fn collect_option_banks(
    page: &PageNodeV2,
    regions: &[RegionLayoutNode],
    consumed: &mut BTreeSet<String>,
) -> Vec<OptionBankCandidateV1> {
    regions
        .iter()
        .filter(|region| region.role == SemanticRegionRole::SharedOptionBank)
        .enumerate()
        .filter_map(|(index, region)| {
            let source = page
                .regions
                .iter()
                .find(|candidate| candidate.id == region.region_id)?;
            let options = detect_option_run_from_lines(region_lines(page, source))?;
            // Lines above the first option label are the bank's heading ("List of
            // Headings"). §6.9 needs it to separate a heading bank from a feature bank.
            let first_label_line = options.first().map(|option| option.label_node_id.as_str());
            let title = region_lines(page, source)
                .into_iter()
                .take_while(|line| Some(line.id.as_str()) != first_label_line)
                .map(|line| line.text.trim())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let title = (!title.is_empty()).then_some(title);
            for option in &options {
                for node_id in &option.text_node_ids {
                    consumed.insert(node_id.clone());
                }
            }
            Some(OptionBankCandidateV1 {
                bank_id: format!("bank-{}-{}", page.page_index, index + 1),
                page_index: page.page_index,
                region_id: Some(region.region_id.clone()),
                title,
                labels: options.iter().map(|option| option.label.clone()).collect(),
                options,
                confidence: region.role_confidence,
            })
        })
        .collect()
}

// --------------------------------------------------------- visual stimuli ---

fn collect_visual_stimuli(
    page: &PageNodeV2,
    regions: &[RegionLayoutNode],
    tokens: &[QuestionNumberToken],
) -> Vec<VisualStimulusCandidateV1> {
    regions
        .iter()
        .enumerate()
        .filter(|(_, region)| is_visual_kind(&region.kind))
        .map(|(index, region)| {
            let question_refs = tokens
                .iter()
                .filter(|token| {
                    token.bbox.y <= bottom_of(&region.bbox) && bottom_of(&token.bbox) >= region.bbox.y
                })
                .map(|token| token.value)
                .collect();
            VisualStimulusCandidateV1 {
                stimulus_id: format!("stimulus-{}-{}", page.page_index, index + 1),
                page_index: page.page_index,
                region_id: region.region_id.clone(),
                kind: region.kind.clone(),
                bbox: region.bbox.clone(),
                confidence: region.role_confidence,
                question_refs,
                // Crop and slot overlay are resolved by `stimulus::build_stimulus`,
                // which needs the settled question blocks.
                asset_id: None,
                hotspots: Vec::new(),
                issues: Vec::new(),
            }
        })
        .collect()
}

// ------------------------------------------------------ unassigned ledger ---

fn collect_unassigned_evidence(
    page: &PageNodeV2,
    regions: &[RegionLayoutNode],
    banks: &[OptionBankCandidateV1],
    consumed: &BTreeSet<String>,
) -> Vec<UnassignedEvidence> {
    // Roles whose lines are explained by the graph itself rather than by a
    // question boundary: page chrome, the instruction zone, passage prose and
    // the answer key.
    let mut exonerated: BTreeSet<&str> = regions
        .iter()
        .filter(|region| {
            matches!(
                region.role,
                SemanticRegionRole::HeaderFooter
                    | SemanticRegionRole::QuestionInstruction
                    | SemanticRegionRole::Passage
                    | SemanticRegionRole::AnswerKey
            )
        })
        .flat_map(|region| region.child_line_ids.iter().map(String::as_str))
        .collect();

    // A recognised shared bank is emitted as its own artifact, so its lines —
    // including the "List of Headings" heading, which no question consumes — are
    // accounted for. Exoneration is keyed on an actually emitted bank: a
    // bank-shaped region that failed the label-run test stays in the ledger.
    for bank in banks {
        let Some(region_id) = bank.region_id.as_deref() else {
            continue;
        };
        if let Some(region) = regions
            .iter()
            .find(|region| region.region_id == region_id)
        {
            exonerated.extend(region.child_line_ids.iter().map(String::as_str));
        }
    }

    page.lines
        .iter()
        .filter(|line| {
            let text = line.text.trim();
            text.chars().count() >= MIN_SIGNIFICANT_CHARS
                && !consumed.contains(&line.id)
                && !exonerated.contains(line.id.as_str())
        })
        .map(|line| UnassignedEvidence {
            source_node_id: line.id.clone(),
            page_index: page.page_index,
            reason: "not_assigned_to_a_question_boundary".to_string(),
            text_preview: line.text.trim().chars().take(80).collect::<String>(),
            text_char_count: line.text.trim().chars().count(),
            bbox: line.bbox.clone(),
        })
        .collect()
}

// ------------------------------------------------------------------ orchestrator ---

/// Lines that belong to a region whose role makes them structurally off-limits to
/// question-block geometry: a shared option bank or the answer key.
///
/// These sit *below* the questions, so the last question's interval is open to the
/// page bottom and would otherwise absorb the whole bank as if it were that
/// question's own option run (§6.7).
///
/// Passage and instruction lines are deliberately *not* reserved: completion tasks
/// interleave question stems with passage text, and instruction lines can sit
/// between question intervals. Those cases belong to task classification (§6.8+),
/// not to boundary recovery.
fn reserved_lines(regions: &[RegionLayoutNode]) -> BTreeSet<String> {
    regions
        .iter()
        .filter(|region| {
            matches!(
                region.role,
                SemanticRegionRole::SharedOptionBank | SemanticRegionRole::AnswerKey
            )
        })
        .flat_map(|region| region.child_line_ids.iter().cloned())
        .collect()
}

pub(super) fn build_layout(document: &DocumentIRV2) -> LayoutBuild {
    let mut pages = Vec::with_capacity(document.pages.len());
    let mut instruction_zones = Vec::new();
    let mut question_blocks = Vec::new();
    let mut option_banks = Vec::new();
    let mut visual_stimuli = Vec::new();
    let mut table_stimuli = Vec::new();
    let mut unassigned_evidence = Vec::new();

    for page in &document.pages {
        let regions = segment_region_roles(page);
        let zones = detect_instruction_zones(page, &regions);
        let expected: Vec<u32> = zones
            .iter()
            .flat_map(|zone| zone.expected_numbers.iter().copied())
            .collect();
        let expected_ref = (!expected.is_empty()).then_some(expected.as_slice());

        let tokens = detect_number_tokens(page, &regions, expected_ref);

        let reserved = reserved_lines(&regions);
        let mut consumed: BTreeSet<String> = BTreeSet::new();
        let mut blocks = assemble_blocks(page, &tokens, &reserved, &mut consumed);
        let banks = collect_option_banks(page, &regions, &mut consumed);
        let unassigned = collect_unassigned_evidence(page, &regions, &banks, &consumed);

        // A page with exactly one shared bank lets a matching block point at it
        // without inventing a slot-level assignment.
        if banks.len() == 1 {
            let bank_id = banks[0].bank_id.clone();
            for block in blocks.iter_mut() {
                if block.option_run.is_none() {
                    block.shared_option_bank_ref = Some(bank_id.clone());
                }
            }
        }

        // §6.10 stimulus is compiled from the settled blocks: a hotspot is slot
        // geometry, so it must see the same block objects the graph exports.
        let region_visuals = collect_visual_stimuli(page, &regions, &tokens);
        let stimulus = super::stimulus::build_stimulus(page, &blocks, region_visuals);

        pages.push(PageLayoutGraph {
            page_index: page.page_index,
            width_pt: page.width_pt,
            height_pt: page.height_pt,
            rotation: page.rotation,
            regions,
            number_tokens: tokens,
            text_node_count: page.lines.len(),
        });
        instruction_zones.extend(zones);
        question_blocks.extend(blocks);
        option_banks.extend(banks);
        visual_stimuli.extend(stimulus.visuals);
        table_stimuli.extend(stimulus.tables);
        unassigned_evidence.extend(unassigned);
    }

    // §6.8 classification and the §6.8/§6.11 hard closures run once the whole
    // document is segmented: a declarative range may span pages, so a per-page pass
    // cannot decide whether a group is complete.
    let task_groups = super::task_groups::build_task_groups(
        &pages,
        &instruction_zones,
        &question_blocks,
        &option_banks,
        &visual_stimuli,
        &unassigned_evidence,
    );

    LayoutBuild {
        pages,
        instruction_zones,
        question_blocks,
        task_groups,
        option_banks,
        visual_stimuli,
        table_stimuli,
        unassigned_evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recognition::local::{
        question_layout_graph_from_document_value, question_layout_graph_from_value,
        question_layout_graph_to_value, QUESTION_LAYOUT_GRAPH_SCHEMA_VERSION,
    };
    use serde_json::{json, Value};

    /// Builds a physical `DocumentIRV2` page. Geometry is arbitrary but internally
    /// consistent: A4 at 595x842, 12pt line boxes, question column starting at x=72.
    #[derive(Default)]
    struct PageParts {
        lines: Vec<Value>,
        spans: Vec<Value>,
        regions: Vec<Value>,
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Value {
        json!({
            "x": x,
            "y": y,
            "width": width,
            "height": height,
            "unit": "pt",
            "origin": "top-left",
            "pageRotation": 0
        })
    }

    fn anchor(node_id: &str) -> Value {
        json!({
            "sourceFileId": "file-1",
            "pageIndex": 0,
            "nodeIds": [node_id],
            "extractionMode": "pdf_native",
            "sourceHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        })
    }

    fn span(id: &str, text: &str, x: f64, y: f64, width: f64) -> Value {
        json!({
            "id": id,
            "glyphIds": [],
            "text": text,
            "bbox": rect(x, y, width, 12.0),
            "style": { "fontSizePt": 11.0 },
            "whitespaceBefore": "source",
            "whitespaceAfter": "none",
            "confidence": 1.0,
            "sourceAnchors": [anchor(id)]
        })
    }

    impl PageParts {
        fn line(&mut self, id: &str, x: f64, y: f64, width: f64, spans: Vec<Value>) {
            let text: String = spans
                .iter()
                .map(|span| span["text"].as_str().unwrap_or_default())
                .collect();
            let span_ids: Vec<Value> = spans.iter().map(|span| span["id"].clone()).collect();
            let source_order = self.lines.len() as u32;
            self.spans.extend(spans);
            self.lines.push(json!({
                "id": id,
                "spanIds": span_ids,
                "text": text,
                "bbox": rect(x, y, width, 12.0),
                "writingMode": "horizontal-tb",
                "indentationPt": 0.0,
                "sourceOrder": source_order,
                "confidence": 1.0,
                "sourceAnchors": [anchor(id)]
            }));
        }

        fn region(&mut self, id: &str, kind: &str, bbox: Value, line_ids: &[&str]) {
            self.regions.push(json!({
                "id": id,
                "kind": kind,
                "bbox": bbox,
                "childLineIds": line_ids,
                "childObjectIds": [],
                "confidence": 1.0,
                "sourceAnchors": [anchor(id)]
            }));
        }

        fn document(&self) -> Value {
            json!({
                "schemaVersion": "DocumentIRV2",
                "documentId": "document-1",
                "jobId": "job-1",
                "sourceFiles": [{
                    "sourceFileId": "file-1",
                    "originalName": "sample.pdf",
                    "mediaType": "application/pdf",
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "byteLength": 1,
                    "role": "question_paper"
                }],
                "pages": [{
                    "pageIndex": 0,
                    "widthPt": 595.0,
                    "heightPt": 842.0,
                    "rotation": 0,
                    "glyphs": [],
                    "spans": self.spans,
                    "lines": self.lines,
                    "regions": self.regions,
                    "vectorPaths": [],
                    "tables": [],
                    "assetIds": [],
                    "readingOrder": [],
                    "quality": {
                        "classification": "born_digital",
                        "nativeCharacterCount": 1,
                        "unicodeErrorRatio": 0.0,
                        "duplicateTextRatio": 0.0,
                        "imageCoverageRatio": 0.0,
                        "textCoverageRatio": 1.0,
                        "rotationConfidence": 1.0,
                        "requiresOcrRegions": [],
                        "warnings": []
                    }
                }],
                "assets": [],
                "coverageLedger": [],
                "parser": {
                    "provider": "p4-t02-test",
                    "providerVersion": "0.1.0",
                    "extractionStartedAt": "2026-09-12T00:00:00Z",
                    "extractionCompletedAt": "2026-09-12T00:00:01Z",
                    "options": {},
                    "warnings": []
                }
            })
        }
    }

    /// A "List of Headings" matching task: four short-answer-style prompts whose
    /// answers come from a bank at the bottom of the page. The bank sits inside the
    /// last question's open interval, which is exactly the trap this test guards.
    fn matching_task_page() -> PageParts {
        let mut page = PageParts::default();
        page.line(
            "h-line",
            72.0,
            24.0,
            200.0,
            vec![span("h-1", "IELTS Reading Practice Test 1", 72.0, 24.0, 200.0)],
        );
        page.line(
            "i-1",
            72.0,
            70.0,
            90.0,
            vec![span("i-1s", "Questions 1-4", 72.0, 70.0, 90.0)],
        );
        page.line(
            "i-2",
            72.0,
            86.0,
            220.0,
            vec![span(
                "i-2s",
                "Choose the correct letter A, B, C or D.",
                72.0,
                86.0,
                220.0,
            )],
        );

        page.line(
            "q-1",
            72.0,
            124.0,
            200.0,
            vec![
                span("n-1", "1", 72.0, 124.0, 8.0),
                span("t-1", " The harbour was closed to shipping.", 80.0, 124.0, 192.0),
            ],
        );
        page.line(
            "q-2",
            72.0,
            152.0,
            220.0,
            vec![
                span("n-2", "2", 72.0, 152.0, 8.0),
                span(
                    "t-2",
                    " Coastal erosion increased after the storm.",
                    80.0,
                    152.0,
                    212.0,
                ),
            ],
        );
        // Question 3 wraps onto a continuation line, which must join its stem.
        page.line(
            "q-3a",
            72.0,
            180.0,
            180.0,
            vec![
                span("n-3", "3", 72.0, 180.0, 8.0),
                span("t-3", " Local fishermen opposed the new", 80.0, 180.0, 172.0),
            ],
        );
        page.line(
            "q-3b",
            72.0,
            196.0,
            220.0,
            vec![span(
                "t-4",
                "regulations introduced by the council.",
                72.0,
                196.0,
                220.0,
            )],
        );
        page.line(
            "q-4",
            72.0,
            224.0,
            210.0,
            vec![
                span("n-4", "4", 72.0, 224.0, 8.0),
                span(
                    "t-5",
                    " The council approved the plan in June.",
                    80.0,
                    224.0,
                    202.0,
                ),
            ],
        );

        page.line(
            "bank-h",
            72.0,
            300.0,
            120.0,
            vec![span("b-h", "List of Headings", 72.0, 300.0, 120.0)],
        );
        for (index, (label, body)) in [
            ("A", " Historic harbour"),
            ("B", " Coastal town"),
            ("C", " Fishing village"),
            ("D", " Riverside market"),
            ("E", " Northern port"),
        ]
        .iter()
        .enumerate()
        {
            let y = 320.0 + index as f64 * 16.0;
            let line_id = format!("b-{}", label.to_lowercase());
            let label_span = format!("b-{}-label", label.to_lowercase());
            let text_span = format!("b-{}-text", label.to_lowercase());
            page.line(
                &line_id,
                72.0,
                y,
                180.0,
                vec![
                    span(&label_span, label, 72.0, y, 10.0),
                    span(&text_span, body, 82.0, y, 170.0),
                ],
            );
        }

        page.region("r-header", "header", rect(60.0, 20.0, 480.0, 18.0), &["h-line"]);
        page.region(
            "r-instruction",
            "text",
            rect(60.0, 64.0, 480.0, 40.0),
            &["i-1", "i-2"],
        );
        page.region(
            "r-prompt",
            "text",
            rect(60.0, 118.0, 480.0, 120.0),
            &["q-1", "q-2", "q-3a", "q-3b", "q-4"],
        );
        page.region(
            "r-bank",
            "text",
            rect(60.0, 294.0, 480.0, 120.0),
            &["bank-h", "b-a", "b-b", "b-c", "b-d", "b-e"],
        );
        page
    }

    #[test]
    fn builds_question_blocks_and_keeps_the_shared_bank_out_of_the_last_question() {
        let document = matching_task_page().document();
        let graph = question_layout_graph_from_document_value(&document).expect("graph");

        assert_eq!(graph.schema_version, QUESTION_LAYOUT_GRAPH_SCHEMA_VERSION);
        assert_eq!(graph.document_id, "document-1");
        assert_eq!(graph.job_id, "job-1");

        // Instruction zone: the range is parsed by the existing grammar helper,
        // not by a second implementation of "Questions N-M".
        assert_eq!(graph.instruction_zones.len(), 1);
        let zone = &graph.instruction_zones[0];
        assert_eq!(zone.expected_numbers, vec![1, 2, 3, 4]);
        assert_eq!(zone.question_range, Some([1, 4]));
        assert_eq!(zone.task_hint.as_deref(), Some("single_choice"));

        // Region roles: chrome, instruction, prompt and shared bank are separated.
        let page = &graph.pages[0];
        let role_of = |region_id: &str| {
            page.regions
                .iter()
                .find(|region| region.region_id == region_id)
                .map(|region| region.role)
                .expect("region role")
        };
        assert_eq!(role_of("r-header"), SemanticRegionRole::HeaderFooter);
        assert_eq!(role_of("r-instruction"), SemanticRegionRole::QuestionInstruction);
        assert_eq!(role_of("r-prompt"), SemanticRegionRole::QuestionPrompt);
        assert_eq!(role_of("r-bank"), SemanticRegionRole::SharedOptionBank);

        assert_eq!(page.number_tokens.len(), 4);
        for token in &page.number_tokens {
            assert!(
                token.score >= NUMBER_TOKEN_MIN_SCORE,
                "token {} scored {}",
                token.value,
                token.score
            );
        }

        // Question boundaries, including the wrapped continuation of question 3.
        let numbers: Vec<u32> = graph
            .question_blocks
            .iter()
            .map(|block| block.question_number)
            .collect();
        assert_eq!(numbers, vec![1, 2, 3, 4]);
        let stem_of = |number: u32| {
            graph
                .question_blocks
                .iter()
                .find(|block| block.question_number == number)
                .expect("block")
                .stem_text
                .clone()
        };
        assert_eq!(stem_of(1), "The harbour was closed to shipping.");
        assert_eq!(stem_of(2), "Coastal erosion increased after the storm.");
        assert_eq!(
            stem_of(3),
            "Local fishermen opposed the new regulations introduced by the council."
        );
        assert_eq!(stem_of(4), "The council approved the plan in June.");

        // The trap: question 4's interval is open to the page bottom, so without
        // reserving bank lines it would claim the whole bank as its own option run.
        assert!(
            graph
                .question_blocks
                .iter()
                .all(|block| block.option_run.is_none()),
            "a shared bank must not be read as a per-question option run"
        );

        assert_eq!(graph.option_banks.len(), 1);
        let bank = &graph.option_banks[0];
        assert_eq!(bank.labels, vec!["A", "B", "C", "D", "E"]);
        assert_eq!(bank.options.len(), 5);
        assert_eq!(bank.options[0].text, "Historic harbour");

        // Exactly one bank on the page: every prompt can point at it.
        assert!(
            graph
                .question_blocks
                .iter()
                .all(|block| block.shared_option_bank_ref.as_deref() == Some(bank.bank_id.as_str()))
        );

        for block in &graph.question_blocks {
            assert_eq!(block.source_coverage, 1.0, "{}", block.candidate_id);
            assert!(block.number_anchor.is_some());
        }

        // Nothing significant is left unaccounted for: header and instruction lines
        // are exonerated by role, and every other line was consumed.
        assert!(
            graph.unassigned_evidence.is_empty(),
            "unexpected unassigned evidence: {:?}",
            graph.unassigned_evidence
        );
    }

    /// A single-choice task: the prompt and its option run live in one physical
    /// region, so the number token must still be recognised inside it.
    #[test]
    fn detects_a_local_option_run_and_excludes_it_from_the_stem() {
        let mut page = PageParts::default();
        page.line(
            "i-1",
            72.0,
            70.0,
            90.0,
            vec![span("i-1s", "Questions 1-2", 72.0, 70.0, 90.0)],
        );
        page.line(
            "i-2",
            72.0,
            86.0,
            220.0,
            vec![span(
                "i-2s",
                "Choose the correct letter A, B, C or D.",
                72.0,
                86.0,
                220.0,
            )],
        );
        page.line(
            "q-1",
            72.0,
            124.0,
            230.0,
            vec![
                span("n-1", "1", 72.0, 124.0, 8.0),
                span(
                    "t-1",
                    " What is the capital city of the country?",
                    80.0,
                    124.0,
                    222.0,
                ),
            ],
        );
        page.line(
            "q-2",
            72.0,
            152.0,
            220.0,
            vec![
                span("n-2", "2", 72.0, 152.0, 8.0),
                span(
                    "t-2",
                    " Which city hosts the annual festival?",
                    80.0,
                    152.0,
                    212.0,
                ),
            ],
        );
        for (index, (label, city)) in [
            ("A", " London"),
            ("B", " Paris"),
            ("C", " Rome"),
            ("D", " Berlin"),
        ]
        .iter()
        .enumerate()
        {
            let y = 180.0 + index as f64 * 16.0;
            page.line(
                &format!("o-{}", label.to_lowercase()),
                72.0,
                y,
                160.0,
                vec![
                    span(&format!("o-{}-label", label.to_lowercase()), label, 72.0, y, 10.0),
                    span(&format!("o-{}-text", label.to_lowercase()), city, 82.0, y, 150.0),
                ],
            );
        }
        page.region(
            "r-instruction",
            "text",
            rect(60.0, 64.0, 480.0, 40.0),
            &["i-1", "i-2"],
        );
        page.region(
            "r-prompt",
            "text",
            rect(60.0, 118.0, 480.0, 130.0),
            &["q-1", "q-2", "o-a", "o-b", "o-c", "o-d"],
        );

        let graph = question_layout_graph_from_document_value(&page.document()).expect("graph");

        let numbers: Vec<u32> = graph
            .question_blocks
            .iter()
            .map(|block| block.question_number)
            .collect();
        assert_eq!(numbers, vec![1, 2]);

        let first = &graph.question_blocks[0];
        assert_eq!(first.stem_text, "What is the capital city of the country?");
        assert!(
            first.option_run.is_none(),
            "the option run belongs to question 2, not question 1"
        );

        let second = &graph.question_blocks[1];
        assert_eq!(second.stem_text, "Which city hosts the annual festival?");
        let run = second.option_run.as_ref().expect("option run");
        assert_eq!(run.labels, vec!["A", "B", "C", "D"]);
        assert_eq!(run.options[0].text, "London");
        assert_eq!(run.options[3].text, "Berlin");
        assert_eq!(second.source_coverage, 1.0);

        // No shared bank on this page, so no block may claim one.
        assert!(graph.option_banks.is_empty());
        assert!(
            graph
                .question_blocks
                .iter()
                .all(|block| block.shared_option_bank_ref.is_none())
        );
        assert!(graph.unassigned_evidence.is_empty());
    }

    #[test]
    fn graph_value_round_trips_through_the_consumer_gate() {
        let document = matching_task_page().document();
        let graph = question_layout_graph_from_document_value(&document).expect("graph");
        let value = question_layout_graph_to_value(&graph).expect("serialize");
        let restored = question_layout_graph_from_value(&value).expect("deserialize");
        assert_eq!(restored, graph);
    }

    #[test]
    fn producer_gate_rejects_foreign_and_unsupported_documents() {
        let malformed = question_layout_graph_from_document_value(&json!({"schemaVersion": "Nope"}));
        assert!(malformed
            .expect_err("malformed document must be rejected")
            .starts_with("QUESTION_LAYOUT_GRAPH_DOCUMENT_IR_INVALID:"));

        let mut document = matching_task_page().document();
        document["schemaVersion"] = json!("DocumentIRV1");
        let unsupported = question_layout_graph_from_document_value(&document);
        assert!(unsupported
            .expect_err("unsupported version must be rejected")
            .starts_with("QUESTION_LAYOUT_GRAPH_DOCUMENT_IR_UNSUPPORTED_VERSION:"));

        let mut graph_value = question_layout_graph_to_value(
            &question_layout_graph_from_document_value(&matching_task_page().document())
                .expect("graph"),
        )
        .expect("serialize");
        graph_value["schemaVersion"] = json!("QuestionLayoutGraphV0");
        let wrong_graph = question_layout_graph_from_value(&graph_value);
        assert!(wrong_graph
            .expect_err("foreign graph generation must be rejected")
            .starts_with("QUESTION_LAYOUT_GRAPH_UNSUPPORTED_VERSION:"));
    }
}
