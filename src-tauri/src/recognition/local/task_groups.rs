//! Task grouping, classification and hard closure (plan §6.8, §6.9, §6.11).
//!
//! This module answers the question the geometry layer deliberately does not:
//! given recovered question boundaries, *what kind of task is this*, and is the
//! recovered structure complete enough to be published?
//!
//! Ordering is the point. §6.3 forbids classification before boundary recovery, so
//! this runs strictly after `question_blocks`. Within the module, §6.9's
//! matching-headings test runs first because "List of Headings + roman bank +
//! Paragraph A-G targets" is the one shape that a naive "has a bank" rule would
//! otherwise misread as a feature-matching task.
//!
//! Every check returns stable codes from `ielts_grammar::issue_codes`; the wording
//! of a review message is never the contract.

use super::{
    InstructionZoneCandidate, OptionBankCandidateV1, PageLayoutGraph, QuestionBlockCandidateV1,
    TaskGroupCandidateV1, UnassignedEvidence, VisualStimulusCandidateV1,
};
use crate::ielts_grammar::issue_codes;
use crate::schema::ielts_authoring_v2::TaskTypeV2;
use std::collections::{BTreeMap, BTreeSet};

/// §6.8 `require_question_source_coverage(task, 0.92)`.
const REQUIRED_SOURCE_COVERAGE: f64 = 0.92;
/// A shared bank with fewer options than this cannot satisfy a declared range.
const MIN_BANK_OPTIONS: usize = 3;
/// §6.11: unassigned prose at least this long inside a task's own question span is
/// the signature of "the task type is right but a whole statement was dropped".
const UNASSIGNED_BLOCKER_CHARS: usize = 80;
/// Vertical slack when deciding whether an unassigned line sits inside a task's span.
const SPAN_SLACK_PT: f64 = 6.0;

/// The IELTS fixed response sets §6.8 requires to be offered exactly.
const TFNG_RESPONSES: [&str; 3] = ["true", "false", "not given"];
const YNNG_RESPONSES: [&str; 3] = ["yes", "no", "not given"];

// --------------------------------------------------------------- public entry ---

pub(super) fn build_task_groups(
    pages: &[PageLayoutGraph],
    zones: &[InstructionZoneCandidate],
    blocks: &[QuestionBlockCandidateV1],
    banks: &[OptionBankCandidateV1],
    visuals: &[VisualStimulusCandidateV1],
    unassigned: &[UnassignedEvidence],
) -> Vec<TaskGroupCandidateV1> {
    let assignment = assign_blocks_to_zones(zones, blocks);
    let mut groups = Vec::new();

    // 1. One group per instruction zone, in document order. A zone that declares a
    //    range but received no block is itself evidence (§6.8 requires the declared
    //    questions to exist), so the group is still emitted, empty and flagged.
    for (zone_index, zone) in zones.iter().enumerate() {
        let group_blocks: Vec<&QuestionBlockCandidateV1> = assignment
            .get(&zone_index)
            .into_iter()
            .flatten()
            .filter_map(|block| blocks.iter().find(|candidate| candidate.candidate_id == *block))
            .collect();
        groups.push(build_group(
            format!("group-{}", zone_index + 1),
            zone.page_index,
            zone.question_range,
            zone.expected_numbers.clone(),
            zone.task_hint.clone(),
            Some(zone),
            &group_blocks,
            banks,
            visuals,
            pages,
            unassigned,
        ));
    }

    // 2. Blocks no zone declared. They still form groups, split where the numbering
    //    breaks, so undeclared questions are never silently dropped.
    let orphan_ids: Vec<&String> = blocks
        .iter()
        .filter(|block| !assignment.values().any(|ids| ids.contains(&block.candidate_id)))
        .map(|block| &block.candidate_id)
        .collect();
    for (run_index, run) in contiguous_runs(blocks, &orphan_ids).into_iter().enumerate() {
        let group_blocks: Vec<&QuestionBlockCandidateV1> = run
            .iter()
            .filter_map(|id| blocks.iter().find(|block| block.candidate_id == *id))
            .collect();
        let numbers: Vec<u32> = group_blocks.iter().map(|block| block.question_number).collect();
        let display_range = numbers.first().zip(numbers.last()).map(|(a, b)| [*a, *b]);
        let page_index = group_blocks.first().map(|block| block.page_index).unwrap_or(0);
        groups.push(build_group(
            format!("group-undeclared-{}", run_index + 1),
            page_index,
            display_range,
            numbers,
            None,
            None,
            &group_blocks,
            banks,
            visuals,
            pages,
            unassigned,
        ));
    }

    groups
}

// -------------------------------------------------------------- zone assignment ---

/// Maps each zone index to the block ids it owns. A block belongs to the zone that
/// declares its number; when several zones declare the same number the one on the
/// nearest page wins, since a range may legitimately span pages.
fn assign_blocks_to_zones(
    zones: &[InstructionZoneCandidate],
    blocks: &[QuestionBlockCandidateV1],
) -> BTreeMap<usize, Vec<String>> {
    let mut assignment: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for block in blocks {
        let owner = zones
            .iter()
            .enumerate()
            .filter(|(_, zone)| zone.expected_numbers.contains(&block.question_number))
            .min_by_key(|(_, zone)| zone.page_index.abs_diff(block.page_index));
        if let Some((index, _)) = owner {
            assignment
                .entry(index)
                .or_default()
                .push(block.candidate_id.clone());
        }
    }
    assignment
}

/// Splits block ids into runs whose question numbers are contiguous.
fn contiguous_runs(blocks: &[QuestionBlockCandidateV1], ids: &[&String]) -> Vec<Vec<String>> {
    let mut ordered: Vec<&QuestionBlockCandidateV1> = ids
        .iter()
        .filter_map(|id| blocks.iter().find(|block| block.candidate_id == **id))
        .collect();
    ordered.sort_by_key(|block| (block.page_index, block.question_number));
    let mut runs: Vec<Vec<String>> = Vec::new();
    let mut previous: Option<u32> = None;
    for block in ordered {
        let starts_new_run = previous.is_some_and(|previous| previous + 1 != block.question_number);
        if runs.is_empty() || starts_new_run {
            runs.push(Vec::new());
        }
        runs.last_mut()
            .expect("run was just pushed")
            .push(block.candidate_id.clone());
        previous = Some(block.question_number);
    }
    runs
}

// ------------------------------------------------------------------ one group ---

#[allow(clippy::too_many_arguments)]
fn build_group(
    group_id: String,
    page_index: u32,
    display_range: Option<[u32; 2]>,
    declared_numbers: Vec<u32>,
    task_hint: Option<String>,
    zone: Option<&InstructionZoneCandidate>,
    blocks: &[&QuestionBlockCandidateV1],
    banks: &[OptionBankCandidateV1],
    visuals: &[VisualStimulusCandidateV1],
    pages: &[PageLayoutGraph],
    unassigned: &[UnassignedEvidence],
) -> TaskGroupCandidateV1 {
    let mut page_indices: Vec<u32> = blocks.iter().map(|block| block.page_index).collect();
    if page_indices.is_empty() {
        page_indices.push(page_index);
    }
    page_indices.sort_unstable();
    page_indices.dedup();

    let mut question_numbers: Vec<u32> = blocks.iter().map(|block| block.question_number).collect();
    question_numbers.sort_unstable();
    question_numbers.dedup();

    let instruction_text = zone.map(|zone| zone.text.as_str()).unwrap_or_default();
    let bank = resolve_option_bank(blocks, page_indices.as_slice(), banks);
    let stimulus_refs = resolve_stimuli(&question_numbers, page_indices.as_slice(), visuals);
    let task_type = classify(instruction_text, task_hint.as_deref(), blocks, bank);

    let mut issues = Vec::new();
    check_declared_range_present(&mut issues, &declared_numbers, blocks);
    if task_type.is_none() {
        push_issue(&mut issues, issue_codes::INSTRUCTION_SIGNATURE_UNRESOLVED);
    }
    match task_type.as_ref() {
        Some(TaskTypeV2::SingleChoice) | Some(TaskTypeV2::MultipleChoice) => {
            check_choice_closure(&mut issues, blocks, bank);
        }
        Some(TaskTypeV2::TrueFalseNotGiven) => {
            check_statement_closure(&mut issues, instruction_text, blocks, &TFNG_RESPONSES);
        }
        Some(TaskTypeV2::YesNoNotGiven) => {
            check_statement_closure(&mut issues, instruction_text, blocks, &YNNG_RESPONSES);
        }
        Some(TaskTypeV2::MatchingHeadings)
        | Some(TaskTypeV2::MatchingInformation)
        | Some(TaskTypeV2::MatchingFeatures)
        | Some(TaskTypeV2::Classification) => {
            check_matching_closure(&mut issues, blocks, bank);
        }
        _ => {}
    }
    check_span_unassigned(&mut issues, blocks, unassigned, pages);

    let confidence = group_confidence(blocks, bank, task_type.as_ref());

    TaskGroupCandidateV1 {
        group_id,
        page_indices,
        display_range,
        question_numbers,
        task_type,
        task_hint,
        block_ids: blocks
            .iter()
            .map(|block| block.candidate_id.clone())
            .collect(),
        option_bank_ref: bank.map(|bank| bank.bank_id.clone()),
        stimulus_refs,
        issues,
        confidence,
    }
}

fn group_confidence(
    blocks: &[&QuestionBlockCandidateV1],
    bank: Option<&OptionBankCandidateV1>,
    task_type: Option<&TaskTypeV2>,
) -> f64 {
    if blocks.is_empty() {
        return 0.0;
    }
    let boundary = blocks
        .iter()
        .map(|block| block.boundary_confidence)
        .fold(1.0f64, f64::min);
    let coverage = blocks
        .iter()
        .map(|block| block.source_coverage)
        .fold(1.0f64, f64::min);
    let mut confidence = (boundary * 0.5 + coverage * 0.5).clamp(0.0, 1.0);
    if task_type.is_none() {
        confidence *= 0.6;
    }
    if bank.is_none() && blocks.iter().all(|block| block.option_run.is_none()) {
        confidence *= 0.8;
    }
    confidence.clamp(0.0, 1.0)
}

// -------------------------------------------------------------- bank / stimulus ---

fn resolve_option_bank<'a>(
    blocks: &[&QuestionBlockCandidateV1],
    page_indices: &[u32],
    banks: &'a [OptionBankCandidateV1],
) -> Option<&'a OptionBankCandidateV1> {
    // An explicit reference wins.
    let referenced: Option<&str> = blocks
        .iter()
        .find_map(|block| block.shared_option_bank_ref.as_deref());
    if let Some(referenced) = referenced {
        return banks.iter().find(|bank| bank.bank_id == referenced);
    }
    // A block with its own option run is a self-contained choice task.
    if blocks.iter().any(|block| block.option_run.is_some()) {
        return None;
    }
    // Otherwise a single bank on the group's pages is the group's bank.
    let mut on_pages = banks
        .iter()
        .filter(|bank| page_indices.contains(&bank.page_index));
    let first = on_pages.next()?;
    on_pages.next().is_none().then_some(first)
}

fn resolve_stimuli(
    question_numbers: &[u32],
    page_indices: &[u32],
    visuals: &[VisualStimulusCandidateV1],
) -> Vec<String> {
    visuals
        .iter()
        .filter(|visual| page_indices.contains(&visual.page_index))
        .filter(|visual| {
            visual.question_refs.is_empty()
                || visual
                    .question_refs
                    .iter()
                    .any(|number| question_numbers.contains(number))
        })
        .map(|visual| visual.stimulus_id.clone())
        .collect()
}

// ----------------------------------------------------------------- classification ---

/// §6.8 / §6.9 classification, in the order the plan fixes.
fn classify(
    instruction: &str,
    task_hint: Option<&str>,
    blocks: &[&QuestionBlockCandidateV1],
    bank: Option<&OptionBankCandidateV1>,
) -> Option<TaskTypeV2> {
    let lower = normalize(instruction);
    let stems = blocks
        .iter()
        .map(|block| block.stem_text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let stems_lower = normalize(&stems);
    let bank_lower = bank
        .map(|bank| normalize(bank.title.as_deref().unwrap_or_default()))
        .unwrap_or_default();
    let bank_is_roman = bank.is_some_and(|bank| {
        !bank.labels.is_empty() && bank.labels.iter().all(|label| is_roman_label(label))
    });
    let bank_is_alpha = bank.is_some_and(|bank| {
        !bank.labels.is_empty() && bank.labels.iter().all(|label| is_alpha_label(label))
    });

    // §6.9 first, and explicitly: a heading bank plus paragraph targets is never a
    // generic feature-matching task.
    let has_paragraph_targets = stems_lower.contains("paragraph");
    if bank_lower.contains("list of headings") && bank_is_roman && has_paragraph_targets {
        return Some(TaskTypeV2::MatchingHeadings);
    }

    if lower.contains("true") && lower.contains("false") {
        return Some(TaskTypeV2::TrueFalseNotGiven);
    }
    if lower.contains("yes") && lower.contains("no") {
        return Some(TaskTypeV2::YesNoNotGiven);
    }

    if mentions_multiple_answers(&lower) {
        return Some(TaskTypeV2::MultipleChoice);
    }
    if lower.contains("choose the correct letter")
        || lower.contains("choose the correct answer")
        || blocks.iter().any(|block| block.option_run.is_some())
    {
        return Some(TaskTypeV2::SingleChoice);
    }

    if lower.contains("which paragraph contains") || lower.contains("which section contains") {
        return Some(TaskTypeV2::MatchingInformation);
    }
    if lower.contains("classify") {
        return Some(TaskTypeV2::Classification);
    }
    if lower.contains("match each") || lower.contains("match the") || bank_is_alpha {
        return Some(TaskTypeV2::MatchingFeatures);
    }

    if let Some(completion) = classify_completion(&lower) {
        return Some(completion);
    }
    if lower.contains("answer the questions below") || lower.contains("no more than") {
        return Some(TaskTypeV2::ShortAnswer);
    }
    // A hint carried over from the region-role pass is weaker evidence than the
    // wording, but better than refusing to classify at all.
    match task_hint {
        Some("matching_headings") => Some(TaskTypeV2::MatchingHeadings),
        Some("true_false_not_given") => Some(TaskTypeV2::TrueFalseNotGiven),
        Some("yes_no_not_given") => Some(TaskTypeV2::YesNoNotGiven),
        Some("single_choice") => Some(TaskTypeV2::SingleChoice),
        Some("matching") => Some(TaskTypeV2::MatchingFeatures),
        _ => None,
    }
}

/// `choose TWO letters`, `which THREE of the following`, `write two answers`.
fn mentions_multiple_answers(lower: &str) -> bool {
    const COUNT_WORDS: [&str; 5] = ["two", "three", "four", "2", "3"];
    let has_count = COUNT_WORDS.iter().any(|word| lower.contains(word));
    let multiple_marker = lower.contains("two letters")
        || lower.contains("three letters")
        || lower.contains("choose two")
        || lower.contains("choose three")
        || lower.contains("which two")
        || lower.contains("which three")
        || lower.contains("two answers")
        || lower.contains("three answers")
        || lower.contains("more than one answer");
    multiple_marker || (has_count && lower.contains("from the list"))
}

fn classify_completion(lower: &str) -> Option<TaskTypeV2> {
    if !lower.contains("complete") {
        return None;
    }
    if lower.contains("table") {
        return Some(TaskTypeV2::TableCompletion);
    }
    if lower.contains("flow-chart") || lower.contains("flowchart") {
        return Some(TaskTypeV2::FlowchartCompletion);
    }
    if lower.contains("note") {
        return Some(TaskTypeV2::NoteCompletion);
    }
    if lower.contains("summary") {
        return Some(TaskTypeV2::SummaryCompletion);
    }
    if lower.contains("diagram") || lower.contains("label the") {
        return Some(TaskTypeV2::DiagramLabelCompletion);
    }
    if lower.contains("table") {
        return Some(TaskTypeV2::TableCompletion);
    }
    if lower.contains("sentence") {
        return Some(TaskTypeV2::SentenceCompletion);
    }
    Some(TaskTypeV2::NoteCompletion)
}

fn is_roman_label(label: &str) -> bool {
    matches!(
        label,
        "i" | "ii" | "iii" | "iv" | "v" | "vi" | "vii" | "viii" | "ix" | "x"
    )
}

fn is_alpha_label(label: &str) -> bool {
    label.chars().count() == 1
        && label
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_uppercase())
}

fn normalize(text: &str) -> String {
    text.to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// --------------------------------------------------------------------- closures ---

/// §6.8: a declared range whose numbers produced no block means the paper asks for
/// questions local recognition never found.
fn check_declared_range_present(
    issues: &mut Vec<String>,
    declared: &[u32],
    blocks: &[&QuestionBlockCandidateV1],
) {
    let found: BTreeSet<u32> = blocks.iter().map(|block| block.question_number).collect();
    if declared.iter().any(|number| !found.contains(number)) {
        push_issue(issues, issue_codes::QUESTION_NUMBER_MISSING);
    }
}

/// §6.8 single choice / multiple choice closure.
fn check_choice_closure(
    issues: &mut Vec<String>,
    blocks: &[&QuestionBlockCandidateV1],
    bank: Option<&OptionBankCandidateV1>,
) {
    for block in blocks {
        if block.stem_text.trim().is_empty() {
            push_issue(issues, issue_codes::PROMPT_EMPTY);
        }
        if block
            .ambiguities
            .iter()
            .any(|code| code == issue_codes::PROMPT_BOUNDARY_AMBIGUOUS)
        {
            push_issue(issues, issue_codes::PROMPT_BOUNDARY_AMBIGUOUS);
        }
        if block.source_coverage < REQUIRED_SOURCE_COVERAGE {
            push_issue(issues, issue_codes::SIGNIFICANT_REGION_UNASSIGNED);
        }
    }

    // `require_expected_option_labels`: a choice task needs options from somewhere —
    // either the block's own run or a shared bank.
    let has_answer_options = bank.is_some() || blocks.iter().any(|block| block.option_run.is_some());
    if !has_answer_options {
        push_issue(issues, issue_codes::OPTION_LABEL_MISSING);
        push_issue(issues, issue_codes::OPTION_RUN_INCOMPLETE);
    }

    for block in blocks {
        let Some(run) = block.option_run.as_ref() else {
            continue;
        };
        let mut labels = BTreeSet::new();
        for option in &run.options {
            if option.label.trim().is_empty() {
                push_issue(issues, issue_codes::OPTION_LABEL_MISSING);
            }
            if option.text.trim().is_empty() {
                push_issue(issues, issue_codes::OPTION_TEXT_MISSING);
            }
            // `require_unique_option_labels`
            if !labels.insert(option.label.clone()) {
                push_issue(issues, issue_codes::OPTION_RUN_INCOMPLETE);
            }
        }
        if run.options.len() < MIN_BANK_OPTIONS {
            push_issue(issues, issue_codes::OPTION_RUN_INCOMPLETE);
        }
    }

    if let Some(bank) = bank {
        if bank.options.len() < MIN_BANK_OPTIONS {
            push_issue(issues, issue_codes::OPTION_RUN_INCOMPLETE);
        }
        for option in &bank.options {
            if option.text.trim().is_empty() {
                push_issue(issues, issue_codes::OPTION_TEXT_MISSING);
            }
            if option.label.trim().is_empty() {
                push_issue(issues, issue_codes::OPTION_LABEL_MISSING);
            }
        }
    }
}

/// §6.8 true/false/not-given and yes/no/not-given closure: every statement must be
/// present, and the paper must offer exactly the fixed IELTS response set.
fn check_statement_closure(
    issues: &mut Vec<String>,
    instruction: &str,
    blocks: &[&QuestionBlockCandidateV1],
    expected: &[&str; 3],
) {
    for block in blocks {
        if block.stem_text.trim().is_empty() {
            push_issue(issues, issue_codes::PROMPT_EMPTY);
        }
        if block.source_coverage < REQUIRED_SOURCE_COVERAGE {
            push_issue(issues, issue_codes::SIGNIFICANT_REGION_UNASSIGNED);
        }
    }
    let lower = normalize(instruction);
    if !expected.iter().all(|response| lower.contains(response)) {
        push_issue(issues, issue_codes::OPTION_RUN_INCOMPLETE);
    }
}

/// §6.8 matching closure: every item needs a prompt, and the bank must exist and be
/// fully populated.
fn check_matching_closure(
    issues: &mut Vec<String>,
    blocks: &[&QuestionBlockCandidateV1],
    bank: Option<&OptionBankCandidateV1>,
) {
    for block in blocks {
        if block.stem_text.trim().is_empty() {
            push_issue(issues, issue_codes::PROMPT_EMPTY);
        }
    }
    let Some(bank) = bank else {
        push_issue(issues, issue_codes::OPTION_BANK_MISSING);
        return;
    };
    if bank.options.len() < MIN_BANK_OPTIONS {
        push_issue(issues, issue_codes::OPTION_BANK_MISSING);
    }
    if bank.options.iter().any(|option| option.text.trim().is_empty()) {
        push_issue(issues, issue_codes::OPTION_TEXT_MISSING);
    }
}

/// §6.11: unassigned prose inside a task's own question span is a dropped statement,
/// not page furniture.
fn check_span_unassigned(
    issues: &mut Vec<String>,
    blocks: &[&QuestionBlockCandidateV1],
    unassigned: &[UnassignedEvidence],
    pages: &[PageLayoutGraph],
) {
    if blocks.is_empty() || unassigned.is_empty() {
        return;
    }
    let question_pages: BTreeSet<u32> = blocks.iter().map(|block| block.page_index).collect();
    let span_top = blocks
        .iter()
        .map(|block| block.number_bbox.y)
        .fold(f64::INFINITY, f64::min);
    // The last question's span runs to the bottom of its page, so the same page
    // height the geometry layer used closes the interval here.
    let span_bottom = pages
        .iter()
        .filter(|page| question_pages.contains(&page.page_index))
        .map(|page| page.height_pt)
        .fold(f64::NEG_INFINITY, f64::max);

    let dropped = unassigned.iter().any(|evidence| {
        question_pages.contains(&evidence.page_index)
            && evidence.text_char_count >= UNASSIGNED_BLOCKER_CHARS
            && evidence.bbox.y >= span_top - SPAN_SLACK_PT
            && evidence.bbox.y <= span_bottom + SPAN_SLACK_PT
    });
    if dropped {
        push_issue(issues, issue_codes::SIGNIFICANT_REGION_UNASSIGNED);
    }
}

fn push_issue(issues: &mut Vec<String>, code: &str) {
    if !issues.iter().any(|existing| existing == code) {
        issues.push(code.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(id: &str, number: u32, stem: &str) -> QuestionBlockCandidateV1 {
        QuestionBlockCandidateV1 {
            candidate_id: id.to_string(),
            question_number: number,
            page_index: 0,
            number_anchor: None,
            number_bbox: crate::schema::common::RectV2 {
                x: 72.0,
                y: 100.0 + number as f64 * 20.0,
                width: 8.0,
                height: 12.0,
                unit: crate::schema::common::CoordinateUnitV2::Pt,
                origin: crate::schema::common::CoordinateOriginV2::TopLeft,
                page_rotation: 0,
                normalized: None,
            },
            stem_node_ids: vec![format!("{id}-line")],
            stem_text: stem.to_string(),
            option_run: None,
            shared_option_bank_ref: None,
            visual_object_refs: Vec::new(),
            source_coverage: 1.0,
            boundary_confidence: 1.0,
            ambiguities: Vec::new(),
        }
    }

    fn bank(id: &str, title: Option<&str>, labels: &[&str]) -> OptionBankCandidateV1 {
        OptionBankCandidateV1 {
            bank_id: id.to_string(),
            page_index: 0,
            region_id: Some(format!("{id}-region")),
            title: title.map(ToString::to_string),
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            options: labels
                .iter()
                .map(|label| super::super::OptionCandidateV2 {
                    label: (*label).to_string(),
                    label_node_id: format!("{id}-{label}-label"),
                    text: format!("option {label}"),
                    text_node_ids: vec![format!("{id}-{label}-text")],
                    bbox: crate::schema::common::RectV2 {
                        x: 72.0,
                        y: 600.0,
                        width: 200.0,
                        height: 12.0,
                        unit: crate::schema::common::CoordinateUnitV2::Pt,
                        origin: crate::schema::common::CoordinateOriginV2::TopLeft,
                        page_rotation: 0,
                        normalized: None,
                    },
                })
                .collect(),
            confidence: 0.9,
        }
    }

    fn zone(hint: Option<&str>, text: &str, numbers: Vec<u32>) -> InstructionZoneCandidate {
        InstructionZoneCandidate {
            zone_id: "zone-0-1".to_string(),
            page_index: 0,
            region_id: Some("r-instruction".to_string()),
            question_range: numbers.first().zip(numbers.last()).map(|(a, b)| [*a, *b]),
            expected_numbers: numbers,
            task_hint: hint.map(ToString::to_string),
            text: text.to_string(),
            source_anchor: None,
            confidence: 0.95,
        }
    }

    fn page() -> PageLayoutGraph {
        PageLayoutGraph {
            page_index: 0,
            width_pt: 595.0,
            height_pt: 842.0,
            rotation: 0,
            regions: Vec::new(),
            number_tokens: Vec::new(),
            text_node_count: 0,
        }
    }

    #[test]
    fn matching_headings_takes_priority_over_generic_matching() {
        // §6.9: list-of-headings title + roman bank + paragraph targets.
        let blocks = vec![
            block("b14", 14, "Paragraph A"),
            block("b15", 15, "Paragraph B"),
        ];
        let banks = vec![bank(
            "bank-0-1",
            Some("List of Headings"),
            &["i", "ii", "iii"],
        )];
        let refs: Vec<&QuestionBlockCandidateV1> = blocks.iter().collect();
        let groups = build_task_groups(
            &[page()],
            &[zone(Some("matching_headings"), "Questions 14-15 Choose the correct heading for each paragraph from the list of headings below.", vec![14, 15])],
            &blocks,
            &banks,
            &[],
            &[],
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].task_type, Some(TaskTypeV2::MatchingHeadings));
        assert_eq!(groups[0].option_bank_ref.as_deref(), Some("bank-0-1"));
        assert!(!groups[0]
            .issues
            .contains(&issue_codes::INSTRUCTION_SIGNATURE_UNRESOLVED.to_string()));
        // A roman bank that only offers three headings for two items is complete; the
        // group must not be blocked for a missing bank.
        assert!(!groups[0]
            .issues
            .contains(&issue_codes::OPTION_BANK_MISSING.to_string()));
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn alpha_bank_without_headings_stays_a_feature_match() {
        let blocks = vec![block("b1", 1, "Curie"), block("b2", 2, "Einstein")];
        let banks = vec![bank("bank-0-1", Some("List of People"), &["A", "B", "C"])];
        let groups = build_task_groups(
            &[page()],
            &[zone(None, "Questions 1-2 Match each scientist with a discovery.", vec![1, 2])],
            &blocks,
            &banks,
            &[],
            &[],
        );
        assert_eq!(groups[0].task_type, Some(TaskTypeV2::MatchingFeatures));
    }

    #[test]
    fn multiple_choice_wording_is_not_read_as_single_choice() {
        let blocks = vec![block("b1", 1, "Which TWO developments are mentioned?")];
        let groups = build_task_groups(
            &[page()],
            &[zone(
                None,
                "Questions 1 Choose TWO letters, A-E.",
                vec![1],
            )],
            &blocks,
            &[],
            &[],
            &[],
        );
        assert_eq!(groups[0].task_type, Some(TaskTypeV2::MultipleChoice));
        // No options anywhere: the closure must say so rather than pass silently.
        assert!(groups[0]
            .issues
            .contains(&issue_codes::OPTION_LABEL_MISSING.to_string()));
    }

    #[test]
    fn tfng_requires_the_exact_fixed_response_set() {
        let blocks = vec![block("b1", 1, "The harbour was closed in 1990.")];
        let complete = build_task_groups(
            &[page()],
            &[zone(
                None,
                "Questions 1 Do the following statements agree with the information? Write TRUE, FALSE or NOT GIVEN.",
                vec![1],
            )],
            &blocks,
            &[],
            &[],
            &[],
        );
        assert_eq!(complete[0].task_type, Some(TaskTypeV2::TrueFalseNotGiven));
        assert!(!complete[0]
            .issues
            .contains(&issue_codes::OPTION_RUN_INCOMPLETE.to_string()));

        let incomplete = build_task_groups(
            &[page()],
            &[zone(
                None,
                "Questions 1 Do the following statements agree? Write TRUE or FALSE.",
                vec![1],
            )],
            &blocks,
            &[],
            &[],
            &[],
        );
        assert!(incomplete[0]
            .issues
            .contains(&issue_codes::OPTION_RUN_INCOMPLETE.to_string()));
    }

    #[test]
    fn undeclared_questions_still_form_groups_and_missing_declared_ones_block() {
        // Zone declares 1-3 but only 1 and 2 were recovered, and question 9 exists
        // with no instruction at all.
        let blocks = vec![
            block("b1", 1, "one"),
            block("b2", 2, "two"),
            block("b9", 9, "nine"),
        ];
        let groups = build_task_groups(
            &[page()],
            &[zone(
                None,
                "Questions 1-3 Choose the correct letter A, B, C or D.",
                vec![1, 2, 3],
            )],
            &blocks,
            &[],
            &[],
            &[],
        );
        assert_eq!(groups.len(), 2);
        assert!(groups[0]
            .issues
            .contains(&issue_codes::QUESTION_NUMBER_MISSING.to_string()));
        assert_eq!(groups[1].group_id, "group-undeclared-1");
        assert_eq!(groups[1].question_numbers, vec![9]);
    }

    #[test]
    fn unassigned_prose_inside_a_question_span_is_a_blocker() {
        let blocks = vec![block("b1", 1, "one"), block("b2", 2, "two")];
        let dropped = UnassignedEvidence {
            source_node_id: "line-lost".to_string(),
            page_index: 0,
            reason: "not_assigned_to_a_question_boundary".to_string(),
            text_preview: "A whole statement that no question boundary claimed".to_string(),
            text_char_count: 120,
            bbox: crate::schema::common::RectV2 {
                x: 72.0,
                y: 140.0,
                width: 400.0,
                height: 12.0,
                unit: crate::schema::common::CoordinateUnitV2::Pt,
                origin: crate::schema::common::CoordinateOriginV2::TopLeft,
                page_rotation: 0,
                normalized: None,
            },
        };
        let groups = build_task_groups(
            &[page()],
            &[zone(
                None,
                "Questions 1-2 Choose the correct letter A, B, C or D.",
                vec![1, 2],
            )],
            &blocks,
            &[],
            &[],
            &[dropped],
        );
        assert!(groups[0]
            .issues
            .contains(&issue_codes::SIGNIFICANT_REGION_UNASSIGNED.to_string()));
    }

    #[test]
    fn short_unassigned_furniture_does_not_block() {
        let blocks = vec![block("b1", 1, "one")];
        let noise = UnassignedEvidence {
            source_node_id: "line-noise".to_string(),
            page_index: 0,
            reason: "not_assigned_to_a_question_boundary".to_string(),
            text_preview: "Page 4".to_string(),
            text_char_count: 6,
            bbox: crate::schema::common::RectV2 {
                x: 300.0,
                y: 800.0,
                width: 40.0,
                height: 12.0,
                unit: crate::schema::common::CoordinateUnitV2::Pt,
                origin: crate::schema::common::CoordinateOriginV2::TopLeft,
                page_rotation: 0,
                normalized: None,
            },
        };
        let groups = build_task_groups(
            &[page()],
            &[zone(None, "Questions 1 Choose the correct letter A, B, C or D.", vec![1])],
            &blocks,
            &[],
            &[],
            &[noise],
        );
        assert!(!groups[0]
            .issues
            .contains(&issue_codes::SIGNIFICANT_REGION_UNASSIGNED.to_string()));
    }
}
