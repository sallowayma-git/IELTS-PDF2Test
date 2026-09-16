//! 识别闭环契约（本地链 / 云端链 / 原文件核验 → 统一裁决）。
//!
//! 设计约束（见 `Plan With Files/Dual_Recognition/RECOGNITION_LOOP_CONTRACT.md`）：
//!
//! 1. **唯一权威语义结构是 `IeltsAuthoringIRV2`**。云端候选与原文件核验都不得
//!    产出第二份权威稿；它们只产出**候选**（[`RecognitionCandidateV1`]）与**建议**
//!    （[`DecisionItemV1`]），经正式 V2 patch 路径才写回权威稿。
//! 2. 四条链路结果必须可区分：本地 / 云端 / 原文件 / 裁决。每条链路都有
//!    [`ChainStatusV1`] + 稳定 `reason_code`，**禁止**用空数组或默认值掩盖失败。
//! 3. 用户可见的「问题」与其后台记录分离：`agreed` 与 `info` 不产生逐项问题。
//! 4. 裁决允许「证据不足，无法判断」（[`DecisionResolutionV1::Unverifiable`]），
//!    禁止在无证据时强行选一个版本。

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION: &str = "RecognitionCandidateV1";
pub const RECOGNITION_DECISION_V1_SCHEMA_VERSION: &str = "RecognitionDecisionV1";
pub const RECOGNITION_DECISION_VIEW_V1_SCHEMA_VERSION: &str = "RecognitionDecisionViewV1";
pub const APPLY_RECOGNITION_DECISIONS_RESULT_V1_SCHEMA_VERSION: &str =
    "ApplyRecognitionDecisionsResultV1";
/// 单次导入的裁决模型调用上限（含核验与一次受约束修复）。禁止无限递归校验。
pub const MAX_ADJUDICATION_MODEL_CALLS: u32 = 2;
/// 受约束修复最多一次（与计划 §7.8 一致）。
pub const MAX_CONSTRAINED_REPAIRS: u32 = 1;

// ── 链路与状态 ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainKindV1 {
    Local,
    Cloud,
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainStatusV1 {
    /// 全量可用。
    Succeeded,
    /// 部分可用：可用题组已保留，其余进 `unresolved_regions`。
    Partial,
    /// 完全不可用（输出无法校验且修复失败）。
    Unusable,
    /// 未运行（未配置 / 不支持输入 / 被跳过）。
    #[default]
    NotRun,
}

impl ChainStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Partial => "partial",
            Self::Unusable => "unusable",
            Self::NotRun => "not_run",
        }
    }
}

// ── 稳定原因码 ─────────────────────────────────────────────────────────

pub mod reason {
    pub const NO_PROFILE: &str = "NO_PROFILE";
    pub const CLOUD_DISABLED: &str = "CLOUD_DISABLED";
    pub const MODEL_UNSUPPORTED_INPUT: &str = "MODEL_UNSUPPORTED_INPUT";
    pub const MODEL_TIMEOUT: &str = "MODEL_TIMEOUT";
    pub const MODEL_INVALID_OUTPUT: &str = "MODEL_INVALID_OUTPUT";
    pub const SALVAGE_PARTIAL: &str = "SALVAGE_PARTIAL";
    pub const ADJUDICATION_FAILED: &str = "ADJUDICATION_FAILED";
    /// 三路一致且规则命中：在后台留记录，不产生逐项问题。
    pub const RULES_MATCH: &str = "RULES_MATCH";
    /// 实质分歧且证据成立 → 待确认。
    pub const SUBSTANTIVE_DIVERGENCE: &str = "SUBSTANTIVE_DIVERGENCE";
    /// 结论一致但缺少原文证据 → 不得视为已验证。
    pub const NO_SOURCE_EVIDENCE: &str = "NO_SOURCE_EVIDENCE";
    /// 完全无证据面（无法判断）。
    pub const EVIDENCE_MISSING: &str = "EVIDENCE_MISSING";
    /// 目标已被用户修改，迟到结果不得覆盖。
    pub const USER_EDITED: &str = "USER_EDITED";
    /// **修正写入之后**用户又改动了同一目标：原修正不再生效，其 resolution 随之失效。
    ///
    /// 与 [`USER_EDITED`] 区分：后者是「决策当时目标已被改」（迟到结果不得覆盖），
    /// 本码是「决策已生效、事后被改」（撤销必须拒绝，否则回滚会覆盖用户的新改动）。
    pub const USER_EDITED_AFTER_APPLY: &str = "USER_EDITED_AFTER_APPLY";
    /// 自动应用规则拒绝（低风险条件不成立）。
    pub const AUTO_APPLY_RULE_REJECTED: &str = "AUTO_APPLY_RULE_REJECTED";
    /// 相互依赖的修正中至少一项不可自动应用。
    pub const DEPENDENCY_BLOCKED: &str = "DEPENDENCY_BLOCKED";
    pub const SOURCE_FILE_UNREADABLE: &str = "SOURCE_FILE_UNREADABLE";
}

// ── 链路候选（统一 V2 语义视图）─────────────────────────────────────────

/// 与 `IeltsAuthoringIRV2` 同语义的候选视图：足够比对题目、段落、选项、
/// 答案位、资源与来源覆盖，但不承载任何编辑权威（不写 canonical）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionCandidateV1 {
    pub schema_version: String,
    pub chain: ChainKindV1,
    pub batch_id: String,
    pub item_id: String,
    pub job_id: String,
    pub source_file_id: String,
    pub source_sha256: String,
    /// 生成该候选时的 canonical edit_version 基线。
    pub base_edit_version: i64,
    pub generated_at: String,
    pub status: ChainStatusV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub task_groups: Vec<CandidateTaskGroupV1>,
    #[serde(default)]
    pub slots: Vec<CandidateSlotV1>,
    #[serde(default)]
    pub unresolved_regions: Vec<CandidateUnresolvedRegionV1>,
    #[serde(default)]
    pub assets: Vec<CandidateAssetV1>,
    #[serde(default)]
    pub salvage: Option<SalvageReportV1>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl RecognitionCandidateV1 {
    pub fn is_supported_schema_version(&self) -> bool {
        self.schema_version == RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION
    }

    pub fn group(&self, task_id: &str) -> Option<&CandidateTaskGroupV1> {
        self.task_groups.iter().find(|group| group.task_id == task_id)
    }

    pub fn slot(&self, slot_id: &str) -> Option<&CandidateSlotV1> {
        self.slots.iter().find(|slot| slot.slot_id == slot_id)
    }

    /// 题号 → 槽位（对齐主键，见 §8.2 的确定性对齐键）。
    pub fn slot_by_question(&self, question_number: u32) -> Option<&CandidateSlotV1> {
        self.slots
            .iter()
            .find(|slot| slot.question_number == question_number)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateTaskGroupV1 {
    pub task_id: String,
    /// `QuestionNumberExpressionV2` 形态（`{"kind":"range","start":1,"end":5}` 等）。
    pub display_range: Value,
    /// `TaskTypeV2` 的 snake_case 值。
    pub task_type: String,
    pub instructions_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stimulus_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_bank: Option<CandidateOptionBankV1>,
    #[serde(default)]
    pub response_groups: Vec<CandidateResponseGroupV1>,
    #[serde(default)]
    pub source_anchors: Vec<Value>,
    pub confidence: f64,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateOptionBankV1 {
    pub option_bank_id: String,
    pub options: Vec<CandidateOptionV1>,
    pub allow_reuse: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateOptionV1 {
    pub option_id: String,
    pub label: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateResponseGroupV1 {
    pub response_group_id: String,
    /// `ResponseGroupKindV2` 的 snake_case 值。
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default)]
    pub slot_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<CandidateOptionV1>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignment: Option<String>,
    #[serde(default)]
    pub allow_option_reuse: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateSlotV1 {
    pub slot_id: String,
    pub question_number: u32,
    pub display_label: String,
    pub task_id: String,
    pub response_group_id: String,
    /// `InteractionV2` 的 lowercase 值。
    pub interaction: String,
    /// `AnswerSlotHostTypeV2` 的 snake_case 值。
    pub host_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_node_id: Option<String>,
    /// `AnswerSlotParticipationV2` 的 snake_case 值。
    pub participation: String,
    /// `AnswerValueV2` 形态；`None` = 该链路未给出答案。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<Value>,
    /// 该链路是否给出了可核验的原文证据（quote/bbox/answerKey 行）。
    #[serde(default)]
    pub has_source_evidence: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateUnresolvedRegionV1 {
    pub page_index: u32,
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateAssetV1 {
    pub asset_id: String,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SalvageReportV1 {
    pub total_groups: u32,
    pub kept_groups: u32,
    pub dropped_groups: u32,
    #[serde(default)]
    pub dropped_reasons: Vec<String>,
    pub repairs_used: u32,
}

// ── 统一裁决 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionResolutionV1 {
    /// 三路一致：后台留记录，前端**不产生逐项问题**。
    Agreed,
    /// 符合自动应用规则且已原子写入权威稿。
    AutoFixed,
    /// 实质变化或证据不足：生成**一条**待确认建议。
    NeedsReview,
    /// 证据不足，无法判断：不得强行选一个版本。
    Unverifiable,
}

impl DecisionResolutionV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agreed => "agreed",
            Self::AutoFixed => "auto_fixed",
            Self::NeedsReview => "needs_review",
            Self::Unverifiable => "unverifiable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatusV1 {
    Open,
    Accepted,
    Rejected,
    /// 迟到/过期：批次基线早于用户修改，未应用。
    Superseded,
    /// 自动应用写入失败：权威稿未改动，但「用户最需要手动确认」的项因此诞生，
    /// 必须保留在 `actionable` 里（见 `build_view`），否则会凭空消失。
    Failed,
    /// 自动修正已被用户撤销：权威稿已回滚到修正前的值，决策视为已处理，
    /// 不再提供「撤销」按钮，也不会重新落回 `actionable`。
    Undone,
}

impl DecisionStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Superseded => "superseded",
            Self::Failed => "failed",
            Self::Undone => "undone",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSeverityV1 {
    Info,
    Warning,
    Blocker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionFieldV1 {
    Answer,
    Prompt,
    Options,
    OptionBank,
    SlotInteraction,
    GroupKind,
    SlotPlacement,
    Asset,
    SourceCoverage,
    SlotParticipation,
}

impl DecisionFieldV1 {
    /// **必须与 `serde` 的 `snake_case` 渲染逐字一致。**
    ///
    /// 这个值同时出现在三处，任何不一致都会静默破坏契约：
    /// - `decisionId` 的末段（`d:<targetType>:<targetId>:<field>`）；
    /// - 原文件核验证据的 `anchorKind`（`source_finding:…:<field>`）；
    /// - `recognition_decisions_v1` 的字段键。
    ///
    /// 回归护栏见测试 `as_str_matches_serde_for_every_wire_enum`。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Answer => "answer",
            Self::Prompt => "prompt",
            Self::Options => "options",
            Self::OptionBank => "option_bank",
            Self::SlotInteraction => "slot_interaction",
            Self::GroupKind => "group_kind",
            Self::SlotPlacement => "slot_placement",
            Self::Asset => "asset",
            Self::SourceCoverage => "source_coverage",
            Self::SlotParticipation => "slot_participation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTargetTypeV1 {
    Document,
    Task,
    ResponseGroup,
    Slot,
    Node,
    Asset,
}

impl DecisionTargetTypeV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Task => "task",
            Self::ResponseGroup => "response_group",
            Self::Slot => "slot",
            Self::Node => "node",
            Self::Asset => "asset",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionTargetV1 {
    pub target_type: DecisionTargetTypeV1,
    pub target_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default)]
    pub question_numbers: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionEvidenceV1 {
    pub chain: ChainKindV1,
    /// `page_quote` / `bbox` / `answer_key` / `asset` / `none`。
    pub anchor_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionItemV1 {
    /// 稳定去重键：`d:<targetType>:<targetId>:<field>`。
    pub decision_id: String,
    pub resolution: DecisionResolutionV1,
    pub code: String,
    pub severity: DecisionSeverityV1,
    pub title: String,
    pub user_message: String,
    pub target: DecisionTargetV1,
    pub field: DecisionFieldV1,
    #[serde(default)]
    pub evidence: Vec<DecisionEvidenceV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_value: Option<Value>,
    /// 建议 patch（`AuthoringPatchV2` 形态，可直接经正式 V2 patch 路径应用）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_patch: Option<Value>,
    /// 自动修正时的逆 patch，供前端撤销。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<Value>,
    #[serde(default)]
    pub auto_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<String>,
    pub status: DecisionStatusV1,
    pub reason_code: String,
    /// 相互依赖的修正共用同一 id：必须整组接受或整组拒绝。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_group: Option<String>,
}

impl DecisionItemV1 {
    pub fn decision_id_for(target_type: DecisionTargetTypeV1, target_id: &str, field: DecisionFieldV1) -> String {
        format!("d:{}:{}:{}", target_type.as_str(), target_id, field.as_str())
    }

    /// 这条裁决是否构成「**必须由用户处理**」的待办。**待办语义的唯一判据**。
    ///
    /// `build_view` 的 `actionable` 与调度器的 `actionableCount` 都必须用它，
    /// 不得各写一份条件——两处口径一旦分叉，前端显示与后台计数就会互相矛盾。
    ///
    /// 两类都要计入，**且都不得被当成「已处理」过滤掉**：
    /// - 可人工确认的问题：`Open`（实质变化 `NeedsReview` / 证据不足 `Unverifiable`）；
    /// - 不可忽略的硬失败：`Failed`（自动应用写入失败——权威稿其实**没有被修正**，
    ///   把它过滤掉等于把最该看见的问题藏起来）。
    ///
    /// 一律**不是**待办：`Accepted` / `Rejected`（用户已处理）、`Superseded`（已过期）、
    /// `Undone`（用户已撤销）；`AutoFixed`（已自动写入权威稿）也不算待办。
    pub fn is_actionable(&self) -> bool {
        self.resolution != DecisionResolutionV1::AutoFixed
            && matches!(self.status, DecisionStatusV1::Open | DecisionStatusV1::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DecisionSummaryV1 {
    /// 三路一致（后台留记录，不产生逐项问题）。**一致项不入 `items`**，故该值无法从
    /// `items` 推导，只能由裁决层在构造时确定，之后不再变动。
    pub agreed: u32,
    /// 已自动写入权威稿。与 `needs_review` **互斥**——自动应用把项翻成 `AutoFixed` 后，
    /// 必须同时从 `needs_review` 里减掉，否则同一项会被两个计数同时统计。
    pub auto_fixed: u32,
    /// 需要人工确认（含自动应用失败而降级为待确认的项）。
    pub needs_review: u32,
    /// 证据不足、无法判断；契约上不携带建议 patch。
    pub unverifiable: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionDecisionV1 {
    pub schema_version: String,
    pub batch_id: String,
    pub item_id: String,
    pub job_id: String,
    pub base_edit_version: i64,
    pub generated_at: String,
    /// 各链路结果，供前端解释「为什么有些项无法验证」。
    pub chain_status: ChainStatusSummaryV1,
    #[serde(default)]
    pub items: Vec<DecisionItemV1>,
    pub summary: DecisionSummaryV1,
}

impl RecognitionDecisionV1 {
    pub fn is_supported_schema_version(&self) -> bool {
        self.schema_version == RECOGNITION_DECISION_V1_SCHEMA_VERSION
    }

    pub fn item(&self, decision_id: &str) -> Option<&DecisionItemV1> {
        self.items.iter().find(|item| item.decision_id == decision_id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainStatusSummaryV1 {
    pub local: ChainStatusV1,
    pub cloud: ChainStatusV1,
    pub source: ChainStatusV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_reason_code: Option<String>,
}

// ── 各阶段运行状态 ─────────────────────────────────────────────────────

/// 阶段状态：前端据此解释「为什么现在还没有可确认项」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStateV1 {
    /// 已入队但未开始（例如本地稿已可用、云端仍在排队）。
    Queued,
    Running,
    Succeeded,
    /// 部分可用：可用部分已保留，其余进入 unresolved。
    Partial,
    /// 完全不可用（校验失败且受约束修复失败）。
    Unusable,
    /// 未运行（未配置 / 不支持输入 / 被取消）。
    NotRun,
    Failed,
    Canceled,
}

impl StageStateV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Partial => "partial",
            Self::Unusable => "unusable",
            Self::NotRun => "not_run",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Unusable | Self::NotRun | Self::Failed | Self::Canceled
        )
    }
}

/// 单阶段状态 + 稳定原因码。`reason_code` 为 `None` 只表示「无异常」，
/// **不**表示「已验证」——已验证由 `ChainStatusV1::Succeeded` 表达。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageStatusV1 {
    pub state: StageStateV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl StageStatusV1 {
    pub fn new(state: StageStateV1) -> Self {
        Self {
            state,
            reason_code: None,
            message: None,
            updated_at: None,
        }
    }

    pub fn with_reason(state: StageStateV1, reason_code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            state,
            reason_code: Some(reason_code.into()),
            message: Some(message.into()),
            updated_at: None,
        }
    }
}

/// 四阶段运行状态：本地识别 / 云端识别 / 原文件核验 / 统一裁决。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionChainStateV1 {
    pub local: StageStatusV1,
    pub cloud: StageStatusV1,
    pub source: StageStatusV1,
    pub adjudication: StageStatusV1,
}

// ── 前端读取视图 ───────────────────────────────────────────────────────

/// `get_recognition_decision` 的返回：把阶段状态与裁决结果合成一次读取。
///
/// 契约要点：
/// - `actionable` 只含 `needs_review` / `unverifiable`，**不含** `agreed`（后台留记录）
///   与 `auto_fixed`（属于已完成的修正，走 `auto_applied` 供解释与撤销）。
/// - `stale = true` 时 `base_edit_version < edit_version`：用户在裁决之后改过稿，
///   建议已过期，接受前必须重新核验。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionDecisionViewV1 {
    pub schema_version: String,
    pub item_id: String,
    pub job_id: String,
    pub batch_id: String,
    /// 生成该建议时的 canonical 编辑版本基线。
    pub base_edit_version: i64,
    /// 当前 canonical 编辑版本。
    pub edit_version: i64,
    pub stale: bool,
    pub generated_at: String,
    pub chains: RecognitionChainStateV1,
    pub summary: DecisionSummaryV1,
    /// 需要用户处理的项：`needs_review` + `unverifiable` + `failed`
    /// （自动应用失败的项必须留在这里，否则会变成「用户最需要处理的建议消失」）。
    #[serde(default)]
    pub actionable: Vec<DecisionItemV1>,
    /// 已自动应用的修正记录（可解释、可撤销），不构成「问题」。
    #[serde(default)]
    pub auto_applied: Vec<DecisionItemV1>,
}

impl RecognitionDecisionViewV1 {
    pub fn is_supported_schema_version(&self) -> bool {
        self.schema_version == RECOGNITION_DECISION_VIEW_V1_SCHEMA_VERSION
    }

    /// 是否还有用户需要处理的内容。
    pub fn has_actionable_items(&self) -> bool {
        !self.actionable.is_empty()
    }
}

// ── 决策应用（接受 / 拒绝）────────────────────────────────────────────

/// `apply_recognition_decisions` 的请求。`request_id` 是幂等键：
/// 同一 `request_id` 重复提交只生效一次，返回首次结果并置 `replayed = true`。
///
/// **`deny_unknown_fields` 是安全边界，不是风格选择。**
/// 本命令的请求体曾经以 `decisions:[{decisionId,action}]` 的形式被发送
/// （v1 设计文档的形状）。因为 serde 默认忽略未知字段、`accept`/`reject` 又带
/// `#[serde(default)]`，那种请求会被解析成「accept 与 reject 都为空」，
/// 于是命令**成功地什么都没做**并回 `outcomes: []`——用户以为已处理，
/// 权威稿其实一字未改。拒绝未知字段可让这类错误立刻暴露为
/// `recognition_invalid_input:…`，而不是伪装成成功。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyRecognitionDecisionsRequestV1 {
    pub request_id: String,
    pub batch_id: String,
    /// 调用方看到的编辑版本；与服务端不一致则整批按「过期」处理。
    pub base_edit_version: i64,
    #[serde(default)]
    pub accept: Vec<String>,
    #[serde(default)]
    pub reject: Vec<String>,
    /// 撤销已自动修正的项：回滚权威稿到修正前的值，并把决策状态持久化为
    /// `Undone`。与 `accept`/`reject` 互斥（同一项不能同时接受又撤销）。
    #[serde(default)]
    pub undo: Vec<String>,
}

impl ApplyRecognitionDecisionsRequestV1 {
    /// 在**任何持久化之前**执行的结构校验。
    ///
    /// 返回 `Err` 的请求绝不写入 journal——否则一次语义为空的调用会被记成
    /// 「已处理」，重试时还会被幂等路径当成成功重放。
    pub fn validate(&self) -> Result<(), String> {
        if self.request_id.trim().is_empty() {
            return Err("RECOGNITION_REQUEST_ID_EMPTY".to_string());
        }
        if self.batch_id.trim().is_empty() {
            return Err("RECOGNITION_BATCH_ID_EMPTY".to_string());
        }
        if self.base_edit_version < 0 {
            return Err("RECOGNITION_BASE_VERSION_INVALID".to_string());
        }
        if self.accept.is_empty() && self.reject.is_empty() && self.undo.is_empty() {
            // 空请求、以及「发错字段名」的旧格式请求都会落到这里。
            return Err("RECOGNITION_NO_DECISIONS".to_string());
        }
        let conflicts: Vec<&str> = self
            .accept
            .iter()
            .chain(self.reject.iter())
            .chain(self.undo.iter())
            .filter(|id| {
                let in_accept = self.accept.contains(id);
                let in_reject = self.reject.contains(id);
                let in_undo = self.undo.contains(id);
                (in_accept as u8 + in_reject as u8 + in_undo as u8) > 1
            })
            .map(String::as_str)
            .collect();
        if !conflicts.is_empty() {
            // 同一条既接受又拒绝/撤销没有确定语义，必须整体拒绝而不是二选一。
            return Err(format!("RECOGNITION_DECISION_CONFLICT:{}", conflicts.join(",")));
        }
        if self
            .accept
            .iter()
            .chain(self.reject.iter())
            .chain(self.undo.iter())
            .any(|id| id.trim().is_empty())
        {
            return Err("RECOGNITION_DECISION_ID_EMPTY".to_string());
        }
        Ok(())
    }

    /// 同一 decision 重复出现时只保留一次（顺序稳定），避免同一 id 既落
    /// `applied` 又落 `superseded` 这种自相矛盾的结果。
    pub fn normalized(&self) -> Self {
        let mut seen = std::collections::HashSet::new();
        Self {
            request_id: self.request_id.clone(),
            batch_id: self.batch_id.clone(),
            base_edit_version: self.base_edit_version,
            accept: self
                .accept
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect(),
            reject: self
                .reject
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect(),
            undo: self
                .undo
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcomeKindV1 {
    /// 已通过正式 V2 patch 路径原子写入。
    Applied,
    Rejected,
    /// 过期：批次基线早于用户修改，未写入。
    Superseded,
    Failed,
    /// 自动修正已被撤销：权威稿已回滚，决策状态持久化为 `Undone`。
    Undone,
}

/// 单项处理结果：接受 / 拒绝 / 过期 / 失败四态必须可区分。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionOutcomeV1 {
    pub decision_id: String,
    pub kind: DecisionOutcomeKindV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<String>,
    /// 仅 `Applied` 且可撤销时存在。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRecognitionDecisionsResultV1 {
    pub schema_version: String,
    pub request_id: String,
    pub batch_id: String,
    pub edit_version_before: i64,
    pub edit_version_after: i64,
    /// 同一 `request_id` 的重复提交：未再次写入，返回首次结果。
    pub replayed: bool,
    #[serde(default)]
    pub outcomes: Vec<DecisionOutcomeV1>,
    /// 处理后的最新视图，调用方无需二次读取。
    pub view: RecognitionDecisionViewV1,
}

// ── 阶段状态归一 ───────────────────────────────────────────────────────

impl From<ChainStatusV1> for StageStateV1 {
    fn from(status: ChainStatusV1) -> Self {
        match status {
            ChainStatusV1::Succeeded => Self::Succeeded,
            ChainStatusV1::Partial => Self::Partial,
            ChainStatusV1::Unusable => Self::Unusable,
            ChainStatusV1::NotRun => Self::NotRun,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stage_state_terminality_is_explicit() {
        assert!(!StageStateV1::Queued.is_terminal());
        assert!(!StageStateV1::Running.is_terminal());
        for state in [
            StageStateV1::Succeeded,
            StageStateV1::Partial,
            StageStateV1::Unusable,
            StageStateV1::NotRun,
            StageStateV1::Failed,
            StageStateV1::Canceled,
        ] {
            assert!(state.is_terminal(), "{state:?} must be terminal");
        }
    }

    #[test]
    fn decision_view_distinguishes_stale_batches() {
        let view = RecognitionDecisionViewV1 {
            schema_version: RECOGNITION_DECISION_VIEW_V1_SCHEMA_VERSION.to_string(),
            item_id: "item-1".to_string(),
            job_id: "job-1".to_string(),
            batch_id: "batch-1".to_string(),
            base_edit_version: 2,
            edit_version: 5,
            stale: true,
            generated_at: "2026-09-15T00:00:00Z".to_string(),
            chains: RecognitionChainStateV1 {
                local: StageStatusV1::new(StageStateV1::Succeeded),
                cloud: StageStatusV1::with_reason(
                    StageStateV1::Partial,
                    reason::SALVAGE_PARTIAL,
                    "部分题组无法校验",
                ),
                source: StageStatusV1::new(StageStateV1::NotRun),
                adjudication: StageStatusV1::new(StageStateV1::Succeeded),
            },
            summary: DecisionSummaryV1::default(),
            actionable: vec![],
            auto_applied: vec![],
        };
        assert!(view.is_supported_schema_version());
        assert!(view.stale);
        assert!(!view.has_actionable_items());
        let encoded = serde_json::to_value(&view).unwrap();
        assert_eq!(encoded["chains"]["cloud"]["reasonCode"], json!(reason::SALVAGE_PARTIAL));
        assert_eq!(encoded["chains"]["source"]["state"], json!("not_run"));
    }

    #[test]
    fn apply_result_keeps_four_outcome_kinds_distinct() {
        let result = ApplyRecognitionDecisionsResultV1 {
            schema_version: APPLY_RECOGNITION_DECISIONS_RESULT_V1_SCHEMA_VERSION.to_string(),
            request_id: "req-1".to_string(),
            batch_id: "batch-1".to_string(),
            edit_version_before: 3,
            edit_version_after: 4,
            replayed: false,
            outcomes: vec![
                DecisionOutcomeV1 {
                    decision_id: "d:slot:slot-14:answer".to_string(),
                    kind: DecisionOutcomeKindV1::Applied,
                    reason_code: None,
                    message: "已接受".to_string(),
                    applied_at: Some("2026-09-15T00:00:00Z".to_string()),
                    undo: Some(json!({"op":"setAnswer"})),
                },
                DecisionOutcomeV1 {
                    decision_id: "d:slot:slot-15:answer".to_string(),
                    kind: DecisionOutcomeKindV1::Superseded,
                    reason_code: Some(reason::USER_EDITED.to_string()),
                    message: "已被用户修改，建议过期".to_string(),
                    applied_at: None,
                    undo: None,
                },
                DecisionOutcomeV1 {
                    decision_id: "d:slot:slot-16:answer".to_string(),
                    kind: DecisionOutcomeKindV1::Rejected,
                    reason_code: None,
                    message: "已拒绝".to_string(),
                    applied_at: None,
                    undo: None,
                },
                DecisionOutcomeV1 {
                    decision_id: "d:slot:slot-17:answer".to_string(),
                    kind: DecisionOutcomeKindV1::Failed,
                    reason_code: Some("library_v2_edit_version_conflict".to_string()),
                    message: "写入冲突".to_string(),
                    applied_at: None,
                    undo: None,
                },
            ],
            view: RecognitionDecisionViewV1 {
                schema_version: RECOGNITION_DECISION_VIEW_V1_SCHEMA_VERSION.to_string(),
                item_id: "item-1".to_string(),
                job_id: "job-1".to_string(),
                batch_id: "batch-1".to_string(),
                base_edit_version: 3,
                edit_version: 4,
                stale: false,
                generated_at: "2026-09-15T00:00:00Z".to_string(),
                chains: RecognitionChainStateV1 {
                    local: StageStatusV1::new(StageStateV1::Succeeded),
                    cloud: StageStatusV1::new(StageStateV1::Succeeded),
                    source: StageStatusV1::new(StageStateV1::Succeeded),
                    adjudication: StageStatusV1::new(StageStateV1::Succeeded),
                },
                summary: DecisionSummaryV1::default(),
                actionable: vec![],
                auto_applied: vec![],
            },
        };
        let encoded = serde_json::to_value(&result).unwrap();
        let kinds: Vec<&str> = encoded["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|outcome| outcome["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, vec!["applied", "superseded", "rejected", "failed"]);
    }

    #[test]
    fn candidate_round_trips_and_indexes_by_question() {
        let candidate = RecognitionCandidateV1 {
            schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
            chain: ChainKindV1::Local,
            batch_id: "batch-1".to_string(),
            item_id: "item-1".to_string(),
            job_id: "job-1".to_string(),
            source_file_id: "file-1".to_string(),
            source_sha256: "a".repeat(64),
            base_edit_version: 3,
            generated_at: "2026-09-15T00:00:00Z".to_string(),
            status: ChainStatusV1::Succeeded,
            reason_code: None,
            task_groups: vec![],
            slots: vec![CandidateSlotV1 {
                slot_id: "slot-14".to_string(),
                question_number: 14,
                display_label: "14".to_string(),
                task_id: "task-1".to_string(),
                response_group_id: "rg-1".to_string(),
                interaction: "radio".to_string(),
                host_type: "prompt".to_string(),
                host_node_id: None,
                participation: "scoring".to_string(),
                answer: Some(json!({"kind":"option","labels":["B"],"assignment":"per_slot"})),
                has_source_evidence: true,
            }],
            unresolved_regions: vec![],
            assets: vec![],
            salvage: None,
            warnings: vec![],
        };
        let encoded = serde_json::to_value(&candidate).unwrap();
        let decoded: RecognitionCandidateV1 = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, candidate);
        assert!(decoded.is_supported_schema_version());
        assert_eq!(
            decoded.slot_by_question(14).map(|slot| slot.slot_id.as_str()),
            Some("slot-14")
        );
        assert!(decoded.slot_by_question(15).is_none());
    }

    #[test]
    fn decision_id_is_stable_and_collision_free_across_fields() {
        let answer = DecisionItemV1::decision_id_for(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Answer);
        let prompt = DecisionItemV1::decision_id_for(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Prompt);
        assert_eq!(answer, "d:slot:slot-14:answer");
        assert_ne!(answer, prompt);
    }

    /// 契约护栏：`as_str()` 是三处外部可见字符串的唯一来源
    /// （`decisionId` 末段 / 原文件核验 `anchorKind` 末段 / `recognition_decisions_v1` 字段键），
    /// 必须与 serde 的线上渲染逐字一致。
    ///
    /// 这曾**真实漂移过一次**：`DecisionFieldV1::as_str` 返回 camelCase
    /// （`optionBank`），而线上 `field` 因 `rename_all = "snake_case"` 是
    /// `option_bank`。后果是 `decisionId` 末段与 `field` 不一致，且不满足
    /// `contracts/recognition-decision-view-v1.schema.json` 的
    /// `^d:[a-z_]+:.+:[a-z_]+$`——即契约 schema 会拒绝真实产出。
    #[test]
    fn as_str_matches_serde_for_every_wire_enum() {
        fn wire<T: Serialize>(value: &T) -> String {
            serde_json::to_value(value)
                .expect("线上枚举必须可序列化")
                .as_str()
                .expect("线上枚举必须序列化为字符串")
                .to_string()
        }

        for field in [
            DecisionFieldV1::Answer,
            DecisionFieldV1::Prompt,
            DecisionFieldV1::Options,
            DecisionFieldV1::OptionBank,
            DecisionFieldV1::SlotInteraction,
            DecisionFieldV1::GroupKind,
            DecisionFieldV1::SlotPlacement,
            DecisionFieldV1::Asset,
            DecisionFieldV1::SourceCoverage,
            DecisionFieldV1::SlotParticipation,
        ] {
            assert_eq!(field.as_str(), wire(&field), "DecisionFieldV1 漂移");
            let decision_id =
                DecisionItemV1::decision_id_for(DecisionTargetTypeV1::Slot, "slot-1", field);
            let tail = decision_id.rsplit(':').next().expect("decisionId 必有末段");
            assert_eq!(tail, field.as_str(), "decisionId 末段与 field 不一致：{decision_id}");
            assert!(
                tail.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "decisionId 末段含非 [a-z_] 字符，违反契约 schema 的 pattern：{decision_id}"
            );
        }
        for target in [
            DecisionTargetTypeV1::Document,
            DecisionTargetTypeV1::Task,
            DecisionTargetTypeV1::ResponseGroup,
            DecisionTargetTypeV1::Slot,
            DecisionTargetTypeV1::Node,
            DecisionTargetTypeV1::Asset,
        ] {
            assert_eq!(target.as_str(), wire(&target), "DecisionTargetTypeV1 漂移");
        }
        for resolution in [
            DecisionResolutionV1::Agreed,
            DecisionResolutionV1::AutoFixed,
            DecisionResolutionV1::NeedsReview,
            DecisionResolutionV1::Unverifiable,
        ] {
            assert_eq!(resolution.as_str(), wire(&resolution), "DecisionResolutionV1 漂移");
        }
        for status in [
            DecisionStatusV1::Open,
            DecisionStatusV1::Accepted,
            DecisionStatusV1::Rejected,
            DecisionStatusV1::Superseded,
            DecisionStatusV1::Failed,
            DecisionStatusV1::Undone,
        ] {
            assert_eq!(status.as_str(), wire(&status), "DecisionStatusV1 漂移");
        }
        for chain in [
            ChainStatusV1::Succeeded,
            ChainStatusV1::Partial,
            ChainStatusV1::Unusable,
            ChainStatusV1::NotRun,
        ] {
            assert_eq!(chain.as_str(), wire(&chain), "ChainStatusV1 漂移");
        }
        for stage in [
            StageStateV1::Queued,
            StageStateV1::Running,
            StageStateV1::Succeeded,
            StageStateV1::Partial,
            StageStateV1::Unusable,
            StageStateV1::NotRun,
            StageStateV1::Failed,
            StageStateV1::Canceled,
        ] {
            assert_eq!(stage.as_str(), wire(&stage), "StageStateV1 漂移");
        }
    }

    #[test]
    fn unverifiable_decisions_must_not_carry_a_proposed_patch() {
        // 契约约束（在 adjudicate 中强制）：无法判断的项不得给出建议 patch，
        // 否则前端会出现「证据不足但要求你选一个版本」的矛盾。
        let item = DecisionItemV1 {
            decision_id: DecisionItemV1::decision_id_for(
                DecisionTargetTypeV1::Slot,
                "slot-9",
                DecisionFieldV1::Answer,
            ),
            resolution: DecisionResolutionV1::Unverifiable,
            code: "ANSWER_EVIDENCE_MISSING".to_string(),
            severity: DecisionSeverityV1::Warning,
            title: "第 9 题缺少答案证据".to_string(),
            user_message: "第 9 题的答案缺少可核验的原文证据，暂不做判断。".to_string(),
            target: DecisionTargetV1 {
                target_type: DecisionTargetTypeV1::Slot,
                target_id: "slot-9".to_string(),
                task_id: None,
                node_id: None,
                question_numbers: vec![9],
            },
            field: DecisionFieldV1::Answer,
            evidence: vec![],
            local_value: None,
            cloud_value: None,
            source_value: None,
            proposed_patch: None,
            undo: None,
            auto_applied: false,
            applied_at: None,
            status: DecisionStatusV1::Open,
            reason_code: reason::EVIDENCE_MISSING.to_string(),
            dependency_group: None,
        };
        assert!(item.proposed_patch.is_none());
        assert_eq!(
            serde_json::to_value(&item).unwrap()["resolution"],
            json!("unverifiable")
        );
    }
}
