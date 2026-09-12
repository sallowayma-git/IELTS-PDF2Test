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

use crate::schema::common::{RectV2, SourceAnchorV2};
use crate::schema::document_ir_v2::{DocumentIRV2, PhysicalRegionKindV2};
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
    pub labels: Vec<String>,
    pub options: Vec<OptionCandidateV2>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnassignedEvidence {
    pub source_node_id: String,
    pub page_index: u32,
    pub reason: String,
    pub text_preview: String,
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
    pub option_banks: Vec<OptionBankCandidateV1>,
    pub visual_stimuli: Vec<VisualStimulusCandidateV1>,
    pub unassigned_evidence: Vec<UnassignedEvidence>,
}

impl QuestionLayoutGraphV1 {
    pub(crate) fn is_supported_schema_version(&self) -> bool {
        self.schema_version == QUESTION_LAYOUT_GRAPH_SCHEMA_VERSION
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
        option_banks: build.option_banks,
        visual_stimuli: build.visual_stimuli,
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
    pub option_banks: Vec<OptionBankCandidateV1>,
    pub visual_stimuli: Vec<VisualStimulusCandidateV1>,
    pub unassigned_evidence: Vec<UnassignedEvidence>,
}
