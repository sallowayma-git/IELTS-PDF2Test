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
    /// A listening part (section). Parts own task groups, so neither `task` nor
    /// `region` can address them without lying about what the id points at.
    Part,
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
#[serde(rename_all = "snake_case")]
pub enum PhysicalShadowStatusV2 {
    Available,
    Missing,
    /// 发布后原文件与 physical shadow 已按「题库保存」规则删除；节点覆盖结论取自
    /// 发布时冻结的证据摘要，不再是可以重新计算的事实。
    VerifiedAtPublishSourcePurged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CoverageDispositionV2 {
    Assigned,
    IgnoredWithReason,
    Unassigned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QualityCoverageEntryV2 {
    pub source_node_id: String,
    pub significant: bool,
    pub disposition: CoverageDispositionV2,
    pub target_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CoverageStatusV2 {
    pub physical_shadow: PhysicalShadowStatusV2,
    pub complete: bool,
    pub significant_source_node_count: u64,
    pub explained_source_node_count: u64,
    pub unassigned_source_node_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuestionCoverageStateV2 {
    Complete,
    Missing,
    Undetermined,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QuestionCoverageReportV2 {
    pub status: QuestionCoverageStateV2,
    pub declared_question_numbers: Vec<u32>,
    pub canonical_question_numbers: Vec<u32>,
    pub missing_question_numbers: Vec<u32>,
    pub extra_question_numbers: Vec<u32>,
    pub declarations: Vec<String>,
    pub unparsed_declarations: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CompilerProbeStatusV2 {
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CompilerProbeV2 {
    pub status: CompilerProbeStatusV2,
    pub schema_version: String,
    pub issue_codes: Vec<String>,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CompilerProbesV2 {
    pub v2_runtime: CompilerProbeV2,
    pub v1_compatibility: CompilerProbeV2,
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
    pub coverage_ledger: Vec<QualityCoverageEntryV2>,
    pub coverage_status: CoverageStatusV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_coverage: Option<QuestionCoverageReportV2>,
    pub compiler_probes: CompilerProbesV2,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(value: &impl Serialize) -> String {
        serde_json::to_value(value)
            .expect("线上枚举必须可序列化")
            .as_str()
            .expect("线上枚举必须序列化为字符串")
            .to_string()
    }

    fn published_enum(schema: &Value, definition: &str, property: &str) -> Vec<String> {
        let mut values: Vec<String> = schema["$defs"][definition]["properties"][property]["enum"]
            .as_array()
            .unwrap_or_else(|| panic!("契约 schema 必须声明 {definition}.{property} 的 enum"))
            .iter()
            .map(|value| value.as_str().expect("enum 必须是字符串").to_string())
            .collect();
        values.sort();
        values
    }

    /// 契约护栏：Rust 的线上枚举是 `targetType` 的唯一来源，必须与
    /// `contracts/quality-report-v2.schema.json` 逐字一致。
    ///
    /// 这曾**真实漂移过**：听力 Part 被加进 Rust/TS 的 `ReviewTargetType`
    /// （质量门禁要用 `targetType: "part"` 指向某个 Section），但 schema 的 enum
    /// 没跟着加 —— 结果是真实产出会被自家发布的契约 schema 拒绝。
    /// `verify-schema-contract.mjs` 只比文件哈希，比不出这种「类型与 schema 不一致」。
    #[test]
    fn review_target_type_matches_the_published_schema() {
        let schema: Value = serde_json::from_str(include_str!(
            "../../../contracts/quality-report-v2.schema.json"
        ))
        .expect("契约 schema 必须是合法 JSON");
        let mut shipped = vec![
            wire(&ReviewTargetTypeV2::Document),
            wire(&ReviewTargetTypeV2::Page),
            wire(&ReviewTargetTypeV2::Region),
            wire(&ReviewTargetTypeV2::Task),
            wire(&ReviewTargetTypeV2::ResponseGroup),
            wire(&ReviewTargetTypeV2::Slot),
            wire(&ReviewTargetTypeV2::Asset),
            wire(&ReviewTargetTypeV2::Part),
        ];
        shipped.sort();
        assert_eq!(
            published_enum(&schema, "reviewIssue", "targetType"),
            shipped,
            "ReviewTargetTypeV2 与已发布契约 schema 漂移"
        );
    }

    /// 同一类漂移的另一半：发布后原文件被删的状态也曾只加在 Rust 侧。
    #[test]
    fn physical_shadow_status_matches_the_published_schema() {
        let schema: Value = serde_json::from_str(include_str!(
            "../../../contracts/quality-report-v2.schema.json"
        ))
        .expect("契约 schema 必须是合法 JSON");
        let mut shipped = vec![
            wire(&PhysicalShadowStatusV2::Available),
            wire(&PhysicalShadowStatusV2::Missing),
            wire(&PhysicalShadowStatusV2::VerifiedAtPublishSourcePurged),
        ];
        shipped.sort();
        assert_eq!(
            published_enum(&schema, "coverageStatus", "physicalShadow"),
            shipped,
            "PhysicalShadowStatusV2 与已发布契约 schema 漂移"
        );
    }
}
