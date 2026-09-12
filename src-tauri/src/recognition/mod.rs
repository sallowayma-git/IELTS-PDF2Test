//! Local V2 recognition (plan §6 / P4-T02).
//!
//! This module is the new recognition layer that reads the physical `DocumentIRV2`
//! facts directly instead of starting from V1 `questionGroupCandidates`. It exists
//! alongside `ielts_grammar` while the main path is migrated (plan §17.7 maps
//! `ielts_grammar/*` onto `recognition/local/*`), so the two must not be conflated:
//!
//! - `recognition::local` consumes `DocumentIRV2` (physical geometry) and produces the
//!   `QuestionLayoutGraphV1` intermediate described in plan §6.2.
//! - `ielts_grammar` still consumes the V1 IR and its candidates.
//!
//! `QuestionLayoutGraphV1` answers "which physical nodes belong to which question"
//! before any task-type classification happens (plan §6.3): classification must not
//! precede boundary recovery.

pub(crate) mod local;

use crate::artifact_store::write_canonical_json_atomic;
use crate::util::read_json_opt;
use crate::CommandResult;
use serde_json::Value;
use std::path::Path;

/// Product-path artifact: the geometric layout graph derived from the job's
/// physical `DocumentIRV2`.
pub(crate) const QUESTION_LAYOUT_GRAPH_ARTIFACT_FILE: &str = "question-layout-graph.json";
/// Recorded when the graph could not be derived. Non-fatal by design.
pub(crate) const QUESTION_LAYOUT_GRAPH_ERROR_FILE: &str = "question-layout-graph.error.json";

/// Persist `QuestionLayoutGraphV1` next to the physical document it was derived
/// from, and return it so the caller can report counts.
///
/// This is the product-path entry point of P4-T02: the recognition main path
/// reads `DocumentIRV2` directly, never the V1 `questionGroupCandidates`.
///
/// The write is verified by reading the artifact back through the consumer
/// schema gate. A graph that cannot be re-read is not a usable handoff to the
/// recognition main path, so it must not be reported as written.
pub(crate) fn write_question_layout_graph_artifact(
    document_value: &Value,
    output_path: &Path,
) -> CommandResult<local::QuestionLayoutGraphV1> {
    let graph = local::question_layout_graph_from_document_value(document_value)?;
    let value = local::question_layout_graph_to_value(&graph)?;
    write_canonical_json_atomic(output_path, &value)?;
    let persisted = read_json_opt(output_path)?
        .ok_or_else(|| "QUESTION_LAYOUT_GRAPH_ARTIFACT_MISSING_AFTER_WRITE".to_string())?;
    local::question_layout_graph_from_value(&persisted)
}
