//! `QuestionLayoutGraphV1` — the geometric intermediate layer (plan §6.2).
//!
//! Built directly from `DocumentIRV2` physical facts. It answers "which physical
//! nodes belong to which question" *before* any task-type classification, which is
//! the ordering §6.3 mandates: classification must not precede boundary recovery.
//!
//! Deviations from the plan's pseudocode §6.2, both deliberate:
//! - `number_anchor` is `Option<SourceAnchorV2>`: a numeric span may carry no source
//!   anchor, and inventing one would fabricate provenance.
//! - `ambiguities` is `Vec<String>` of the stable codes from
//!   `ielts_grammar::issue_codes`; the plan's `RecognitionIssueCode` type does not
//!   exist in this crate and the existing vocabulary is the single source of truth.

mod question_blocks;
mod stimulus;
mod task_groups;

use crate::schema::common::{RectV2, SourceAnchorV2};
use crate::schema::document_ir_v2::{DocumentIRV2, PhysicalRegionKindV2};
use crate::schema::ielts_authoring_v2::{RecognitionBlockerTargetV2, TaskTypeV2};
use crate::CommandResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const QUESTION_LAYOUT_GRAPH_SCHEMA_VERSION: &str = "QuestionLayoutGraphV1";

/// Inferred role of a physical region (plan §6.4). The underlying facts are never
/// mutated; the role is an additional interpretation on top of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SemanticRegionRole {
    Passage,
    QuestionInstruction,
    QuestionPrompt,
    OptionRun,
    SharedOptionBank,
    CompletionStimulus,
    AnswerKey,
    HeaderFooter,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegionLayoutNode {
    pub region_id: String,
    pub kind: PhysicalRegionKindV2,
    pub role: SemanticRegionRole,
    pub role_confidence: f64,
    /// Which signals produced the role, so a reviewer can audit the inference.
    pub role_features: Vec<String>,
    pub bbox: RectV2,
    pub child_line_ids: Vec<String>,
}

/// A candidate question-number token found by token/geometry-first scanning
/// (plan §6.5), not by "first digits at the start of a line".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuestionNumberToken {
    pub value: u32,
    pub node_id: String,
    pub page_index: u32,
    pub bbox: RectV2,
    pub score: f64,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstructionZoneCandidate {
    pub zone_id: String,
    pub page_index: u32,
    pub region_id: Option<String>,
    pub question_range: Option<[u32; 2]>,
    pub expected_numbers: Vec<u32>,
    pub task_hint: Option<String>,
    /// Raw instruction text, kept so classification (§6.8) and the compile stage
    /// can read the wording instead of re-deriving it from the region.
    pub text: String,
    pub source_anchor: Option<SourceAnchorV2>,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OptionCandidateV2 {
    pub label: String,
    pub label_node_id: String,
    pub text: String,
    pub text_node_ids: Vec<String>,
    pub bbox: RectV2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OptionRunCandidateV2 {
    pub run_id: String,
    pub labels: Vec<String>,
    pub options: Vec<OptionCandidateV2>,
    pub label_column_x: Option<f64>,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OptionBankCandidateV1 {
    pub bank_id: String,
    pub page_index: u32,
    pub region_id: Option<String>,
    /// The bank's heading, e.g. "List of Headings". §6.9 needs the wording to tell a
    /// heading bank apart from a feature bank.
    pub title: Option<String>,
    pub labels: Vec<String>,
    pub options: Vec<OptionCandidateV2>,
    pub confidence: f64,
}

/// One physical table cell (§6.10). Spans come from the physical table, never from
/// a per-question reconstruction, so a cell that spans rows stays a spanning cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableCellCandidateV1 {
    pub cell_id: String,
    pub row: u32,
    pub col: u32,
    pub row_span: u32,
    pub col_span: u32,
    pub text: String,
    pub bbox: RectV2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableRowCandidateV1 {
    pub row: u32,
    pub cells: Vec<TableCellCandidateV1>,
}

/// A stimulus compiled from `DocumentIRV2.pages[].tables[]` (§6.10). §6.10 forbids
/// rebuilding a table as one question per row, so this is a faithful projection of
/// the physical table plus the issues that make it unusable as-is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableStimulusCandidateV1 {
    pub stimulus_id: String,
    pub page_index: u32,
    pub table_id: String,
    pub bbox: RectV2,
    pub rows: Vec<TableRowCandidateV1>,
    /// Physical fallback crop, when the table's cell text is not trustworthy.
    pub asset_id: Option<String>,
    pub confidence: f64,
    pub issues: Vec<String>,
}

/// A figure/diagram slot overlaid on the source crop (§6.10). Coordinates are
/// normalized to the stimulus bbox, matching the plan's `normalizedRect` contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VisualHotspotCandidateV1 {
    pub slot_id: String,
    pub question_number: u32,
    pub normalized_rect: [f64; 4],
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VisualStimulusCandidateV1 {
    pub stimulus_id: String,
    pub page_index: u32,
    pub region_id: String,
    pub kind: PhysicalRegionKindV2,
    pub bbox: RectV2,
    pub confidence: f64,
    /// Question numbers whose search interval overlaps this stimulus.
    pub question_refs: Vec<u32>,
    /// Source crop backing the hybrid (§6.10). `None` when the figure is vector
    /// drawn with no raster asset; the slot geometry is still reported.
    pub asset_id: Option<String>,
    /// Slot overlays. Empty when no question could be attached.
    pub hotspots: Vec<VisualHotspotCandidateV1>,
    /// Stable issue codes: an unattachable slot must be reported, not dropped.
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnassignedEvidence {
    pub source_node_id: String,
    pub page_index: u32,
    pub reason: String,
    pub text_preview: String,
    /// Full character count of the line, kept separately because `text_preview` is
    /// truncated and the §6.11 blocker is a size threshold.
    pub text_char_count: usize,
    pub bbox: RectV2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuestionBlockCandidateV1 {
    pub candidate_id: String,
    pub question_number: u32,
    pub page_index: u32,
    pub number_anchor: Option<SourceAnchorV2>,
    pub number_bbox: RectV2,
    pub stem_node_ids: Vec<String>,
    pub stem_text: String,
    pub option_run: Option<OptionRunCandidateV2>,
    pub shared_option_bank_ref: Option<String>,
    pub visual_object_refs: Vec<String>,
    /// Fraction of visible text area inside the block's search interval that was
    /// assigned to the stem or its option run.
    pub source_coverage: f64,
    pub boundary_confidence: f64,
    /// Stable issue codes (see `ielts_grammar::issue_codes`).
    pub ambiguities: Vec<String>,
}

/// One recognised task group: a declared question range, the physical blocks that
/// satisfy it, and the task family inferred from the wording (§6.8) and geometry
/// (§6.9). Carries its own blocking issues so the Ready gate has a single place to
/// read (§6.8) instead of re-deriving them from block-level fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskGroupCandidateV1 {
    pub group_id: String,
    pub page_indices: Vec<u32>,
    /// First and last question number of the group, as declared by the instruction.
    pub display_range: Option<[u32; 2]>,
    pub question_numbers: Vec<u32>,
    /// `None` means the wording could not be resolved to a task family. The group
    /// still carries its blocks so nothing is lost; the Ready gate blocks on
    /// `INSTRUCTION_SIGNATURE_UNRESOLVED`.
    pub task_type: Option<TaskTypeV2>,
    pub task_hint: Option<String>,
    pub block_ids: Vec<String>,
    pub option_bank_ref: Option<String>,
    pub stimulus_refs: Vec<String>,
    /// Stable issue codes from `ielts_grammar::issue_codes`.
    pub issues: Vec<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageLayoutGraph {
    pub page_index: u32,
    pub width_pt: f64,
    pub height_pt: f64,
    pub rotation: u16,
    pub regions: Vec<RegionLayoutNode>,
    pub number_tokens: Vec<QuestionNumberToken>,
    pub text_node_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuestionLayoutGraphV1 {
    pub schema_version: String,
    pub document_id: String,
    pub job_id: String,
    pub pages: Vec<PageLayoutGraph>,
    pub instruction_zones: Vec<InstructionZoneCandidate>,
    pub question_blocks: Vec<QuestionBlockCandidateV1>,
    pub task_groups: Vec<TaskGroupCandidateV1>,
    pub option_banks: Vec<OptionBankCandidateV1>,
    pub visual_stimuli: Vec<VisualStimulusCandidateV1>,
    pub table_stimuli: Vec<TableStimulusCandidateV1>,
    pub unassigned_evidence: Vec<UnassignedEvidence>,
}

impl QuestionLayoutGraphV1 {
    pub(crate) fn is_supported_schema_version(&self) -> bool {
        self.schema_version == QUESTION_LAYOUT_GRAPH_SCHEMA_VERSION
    }

    /// The Ready gate for local recognition (§6.8 simple-task closure plus the
    /// §6.11 unassigned-evidence blocker).
    ///
    /// Deduplicated and sorted so the result is comparable across runs and can be
    /// used to decide publishability without depending on discovery order.
    pub(crate) fn blocking_issues(&self) -> Vec<String> {
        let mut codes: Vec<String> = self
            .question_blocks
            .iter()
            .flat_map(|block| block.ambiguities.iter().cloned())
            .chain(
                self.task_groups
                    .iter()
                    .flat_map(|group| group.issues.iter().cloned()),
            )
            .chain(
                self.table_stimuli
                    .iter()
                    .flat_map(|stimulus| stimulus.issues.iter().cloned()),
            )
            .chain(
                self.visual_stimuli
                    .iter()
                    .flat_map(|stimulus| stimulus.issues.iter().cloned()),
            )
            .collect();
        codes.sort();
        codes.dedup();
        codes
    }

    /// Blocking issues with the questions each one applies to, for review surfaces.
    pub(crate) fn blocking_issue_targets(&self) -> Vec<RecognitionBlockerTargetV2> {
        let mut targets: Vec<RecognitionBlockerTargetV2> = self
            .question_blocks
            .iter()
            .flat_map(|block| {
                block.ambiguities.iter().map(|code| RecognitionBlockerTargetV2 {
                    code: code.clone(),
                    target: format!("q{}", block.question_number),
                })
            })
            .chain(self.task_groups.iter().flat_map(|group| {
                group.issues.iter().map(|code| RecognitionBlockerTargetV2 {
                    code: code.clone(),
                    target: group.group_id.clone(),
                })
            }))
            .collect();
        targets.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then(left.target.cmp(&right.target))
        });
        targets.dedup();
        targets
    }
}

/// Build the layout graph from validated physical facts.
pub(crate) fn build_question_layout_graph(document: &DocumentIRV2) -> QuestionLayoutGraphV1 {
    let build = question_blocks::build_layout(document);
    QuestionLayoutGraphV1 {
        schema_version: QUESTION_LAYOUT_GRAPH_SCHEMA_VERSION.to_string(),
        document_id: document.document_id.clone(),
        job_id: document.job_id.clone(),
        pages: build.pages,
        instruction_zones: build.instruction_zones,
        question_blocks: build.question_blocks,
        task_groups: build.task_groups,
        option_banks: build.option_banks,
        visual_stimuli: build.visual_stimuli,
        table_stimuli: build.table_stimuli,
        unassigned_evidence: build.unassigned_evidence,
    }
}

/// Producer schema gate: reject anything that is not a supported `DocumentIRV2`
/// before the graph is built. Mirrors the physical producer's gate so the V2
/// consumer can never silently operate on a half-understood document.
pub(crate) fn question_layout_graph_from_document_value(
    value: &Value,
) -> CommandResult<QuestionLayoutGraphV1> {
    let document: DocumentIRV2 = serde_json::from_value(value.clone())
        .map_err(|error| format!("QUESTION_LAYOUT_GRAPH_DOCUMENT_IR_INVALID:{error}"))?;
    if !document.is_supported_schema_version() {
        return Err(format!(
            "QUESTION_LAYOUT_GRAPH_DOCUMENT_IR_UNSUPPORTED_VERSION:{}",
            document.schema_version
        ));
    }
    Ok(build_question_layout_graph(&document))
}

pub(crate) fn question_layout_graph_to_value(
    graph: &QuestionLayoutGraphV1,
) -> CommandResult<Value> {
    serde_json::to_value(graph)
        .map_err(|error| format!("QUESTION_LAYOUT_GRAPH_SERIALIZE_FAILED:{error}"))
}

/// Consumer schema gate: the counterpart of the producer gate above. A consumer
/// must refuse a graph written by a different schema generation rather than
/// reading it optimistically.
pub(crate) fn question_layout_graph_from_value(
    value: &Value,
) -> CommandResult<QuestionLayoutGraphV1> {
    let graph: QuestionLayoutGraphV1 = serde_json::from_value(value.clone())
        .map_err(|error| format!("QUESTION_LAYOUT_GRAPH_INVALID:{error}"))?;
    if !graph.is_supported_schema_version() {
        return Err(format!(
            "QUESTION_LAYOUT_GRAPH_UNSUPPORTED_VERSION:{}",
            graph.schema_version
        ));
    }
    Ok(graph)
}

pub(crate) struct LayoutBuild {
    pub pages: Vec<PageLayoutGraph>,
    pub instruction_zones: Vec<InstructionZoneCandidate>,
    pub question_blocks: Vec<QuestionBlockCandidateV1>,
    pub task_groups: Vec<TaskGroupCandidateV1>,
    pub option_banks: Vec<OptionBankCandidateV1>,
    pub visual_stimuli: Vec<VisualStimulusCandidateV1>,
    pub table_stimuli: Vec<TableStimulusCandidateV1>,
    pub unassigned_evidence: Vec<UnassignedEvidence>,
}
