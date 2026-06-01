use crate::authoring_pipeline::{dynamic_block_id, dynamic_block_text, dynamic_document_blocks};
use crate::util::{job_dir, read_json_opt, write_json};
use crate::validator::json_issue;
use crate::{hash_bytes, CommandResult};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceReviewV1 {
    pub schema_version: String,
    pub job_id: String,
    pub required: bool,
    pub resolved: bool,
    pub stale: bool,
    pub fingerprint: String,
    pub parser_warnings: Vec<String>,
    pub low_confidence_blocks: Vec<String>,
    pub resolved_at: Option<String>,
    pub note: Option<String>,
}

struct SourceReviewDraft<'a> {
    job_id: &'a str,
    required: bool,
    resolved: bool,
    stale: bool,
    fingerprint: String,
    parser_warnings: Vec<String>,
    low_confidence_blocks: Vec<String>,
    saved: Option<&'a Value>,
}

impl SourceReviewV1 {
    fn new(draft: SourceReviewDraft<'_>) -> Self {
        Self {
            schema_version: "SourceReviewV1".to_string(),
            job_id: draft.job_id.to_string(),
            required: draft.required,
            resolved: draft.resolved,
            stale: draft.stale,
            fingerprint: draft.fingerprint,
            parser_warnings: draft.parser_warnings,
            low_confidence_blocks: draft.low_confidence_blocks,
            resolved_at: draft
                .saved
                .and_then(|value| value.get("resolvedAt"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            note: draft
                .saved
                .and_then(|value| value.get("note"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
        }
    }

    fn to_value(&self) -> CommandResult<Value> {
        serde_json::to_value(self).map_err(|error| error.to_string())
    }

    fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

pub(crate) fn parser_warnings(doc: Option<&Value>) -> Vec<String> {
    doc.and_then(|value| value.pointer("/parser/warnings"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect()
}

pub(crate) fn low_confidence_block_ids(doc: Option<&Value>, threshold: f64) -> Vec<String> {
    dynamic_document_blocks(doc)
        .iter()
        .filter(|block| {
            block
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(1.0)
                < threshold
        })
        .map(dynamic_block_id)
        .collect()
}

fn source_review_path(root: &Path, job_id: &str) -> PathBuf {
    job_dir(root, job_id).join("source-review.json")
}

pub(crate) fn source_review_fingerprint(doc: Option<&Value>) -> String {
    let payload = json!({
        "sourceFileId": doc.and_then(|value| value.pointer("/parser/sourceFileId")).cloned().unwrap_or(Value::Null),
        "sourceStoredName": doc.and_then(|value| value.pointer("/parser/sourceStoredName")).cloned().unwrap_or(Value::Null),
        "provider": doc.and_then(|value| value.pointer("/parser/provider")).cloned().unwrap_or(Value::Null),
        "mode": doc.and_then(|value| value.pointer("/parser/mode")).cloned().unwrap_or(Value::Null),
        "parserWarnings": parser_warnings(doc),
        "lowConfidenceBlocks": dynamic_document_blocks(doc)
            .iter()
            .filter(|block| block.get("confidence").and_then(Value::as_f64).unwrap_or(1.0) < 0.5)
            .map(|block| json!({
                "blockId": dynamic_block_id(block),
                "confidence": block.get("confidence").cloned().unwrap_or(Value::Null),
                "roleHint": block.get("roleHint").cloned().unwrap_or(Value::Null),
                "textHash": hash_bytes(dynamic_block_text(block).as_bytes())
            }))
            .collect::<Vec<_>>()
    });
    serde_json::to_vec(&payload)
        .map(|bytes| hash_bytes(&bytes))
        .unwrap_or_else(|_| hash_bytes(b"source-review-fingerprint-error"))
}

pub(crate) fn source_review_status(
    root: &Path,
    job_id: &str,
    doc: Option<&Value>,
) -> CommandResult<Value> {
    let parser_warnings = parser_warnings(doc);
    let low_confidence_blocks = low_confidence_block_ids(doc, 0.5);
    let required = !parser_warnings.is_empty() || !low_confidence_blocks.is_empty();
    let fingerprint = source_review_fingerprint(doc);
    let saved = read_json_opt(&source_review_path(root, job_id))?;
    let saved_fingerprint = saved
        .as_ref()
        .and_then(|value| value.get("fingerprint"))
        .and_then(Value::as_str);
    let saved_resolved = saved
        .as_ref()
        .and_then(|value| value.get("resolved"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stale = required && saved_resolved && saved_fingerprint != Some(fingerprint.as_str());
    let resolved = !required || (saved_resolved && !stale);
    SourceReviewV1::new(SourceReviewDraft {
        job_id,
        required,
        resolved,
        stale,
        fingerprint,
        parser_warnings,
        low_confidence_blocks,
        saved: saved.as_ref(),
    })
    .to_value()
}

pub(crate) fn write_source_review_status(
    root: &Path,
    job_id: &str,
    doc: Option<&Value>,
    resolved: bool,
    note: Option<String>,
) -> CommandResult<Value> {
    let review_value = source_review_status(root, job_id, doc)?;
    let mut review = SourceReviewV1::from_value(&review_value)
        .ok_or_else(|| "invalid_source_review_status".to_string())?;
    review.resolved = resolved || !review.required;
    review.stale = false;
    review.resolved_at = resolved.then(|| Utc::now().to_rfc3339());
    review.note = note;
    let review = review.to_value()?;
    write_json(&source_review_path(root, job_id), &review)?;
    Ok(review)
}

pub(crate) fn source_review_issues(review: &Value) -> Vec<Value> {
    let typed = SourceReviewV1::from_value(review);
    let required = typed
        .as_ref()
        .map(|review| review.required)
        .unwrap_or_else(|| {
            review
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });
    let resolved = typed
        .as_ref()
        .map(|review| review.resolved)
        .unwrap_or_else(|| {
            review
                .get("resolved")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });
    if !required || resolved {
        return Vec::new();
    }

    let mut issues = Vec::new();
    let parser_warnings = typed.as_ref().map(|review| {
        review
            .parser_warnings
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    });
    let warning_iter: Box<dyn Iterator<Item = &str> + '_> = if let Some(warnings) = &parser_warnings
    {
        Box::new(warnings.iter().copied())
    } else {
        Box::new(
            review
                .get("parserWarnings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        )
    };
    for warning in warning_iter {
        issues.push(json_issue(
            "AuthoringIR",
            "$.sourceReview.parserWarnings",
            &format!(
                "Parser warning must be manually resolved before publish: {}",
                warning
            ),
        ));
    }
    let low_confidence_blocks = typed.as_ref().map(|review| {
        review
            .low_confidence_blocks
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    });
    let block_iter: Box<dyn Iterator<Item = &str> + '_> =
        if let Some(blocks) = &low_confidence_blocks {
            Box::new(blocks.iter().copied())
        } else {
            Box::new(
                review
                    .get("lowConfidenceBlocks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str),
            )
        };
    for block_id in block_iter {
        issues.push(json_issue(
            "AuthoringIR",
            &format!("$.sourceReview.lowConfidenceBlocks[{}]", block_id),
            "Low-confidence parsed block requires source review before publish",
        ));
    }
    if issues.is_empty() {
        issues.push(json_issue(
            "AuthoringIR",
            "$.sourceReview.resolved",
            "Source document review must be resolved before publish",
        ));
    }
    issues
}
