use super::common::SourceAnchorV2;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const QUALITY_REPORT_V2_SCHEMA_VERSION: &str = "QualityReportV2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverityV2 {
    Info,
    Warning,
    Blocking,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStateV2 {
    Ready,
    ReviewRequired,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTargetTypeV2 {
    Document,
    Page,
    Region,
    Task,
    ResponseGroup,
    Slot,
    Asset,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SuggestedActionV2 {
    AssignRole,
    EditText,
    MergeLines,
    SplitPrompt,
    AttachOptionBank,
    ConfirmTable,
    ConfirmFigure,
    ReplaceAsset,
    EnterAnswer,
    IgnoreWithReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ReviewIssueV2 {
    pub issue_id: String,
    pub code: String,
    pub severity: ReviewSeverityV2,
    pub message: String,
    pub target_type: ReviewTargetTypeV2,
    pub target_id: String,
    pub source_anchors: Vec<SourceAnchorV2>,
    pub suggested_actions: Vec<SuggestedActionV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QualityReportV2 {
    pub schema_version: String,
    pub state: ReadinessStateV2,
    pub document_score: f64,
    pub source_coverage: f64,
    pub task_scores: BTreeMap<String, f64>,
    pub hard_failures: Vec<String>,
    pub issues: Vec<ReviewIssueV2>,
    pub metrics: BTreeMap<String, f64>,
    pub evaluated_at: String,
    pub evaluator_version: String,
}

impl QualityReportV2 {
    pub fn is_supported_schema_version(&self) -> bool {
        self.schema_version == QUALITY_REPORT_V2_SCHEMA_VERSION
    }
}
