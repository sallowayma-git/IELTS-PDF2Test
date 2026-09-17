//! 云端修复契约：**完整候选**与**工具消息**。
//!
//! 设计约束（见 `Plan With Files/Dual_Recognition/CLOUD_REPAIR_IMPLEMENTATION_BRIEF_2026-09-17.md`
//! 第 5、6 节）：
//!
//! 1. **完整候选必须真的完整**。[`CloudAuthoringCandidateV1`] 内部直接内嵌一份标准化后的
//!    [`IeltsAuthoringIRV2`]，而不是复用 [`super::recognition_v1::RecognitionCandidateV1`]
//!    那份**扁平投影**。原因：投影类型没有 `passage` 字段，instructions / stimulus / prompt
//!    被压成纯文本、选项内容被压平（`reconcile/candidate.rs`），一旦走投影，
//!    「完整候选」就名不副实，后面的修复回合永远看不到被压掉的富内容。
//!    `RecognitionCandidateV1` 继续保留，但只作**比对用投影**。
//! 2. **模型不拥有身份**。job / source 标识、audit、quality、review 状态、稳定 ID 一律由后端
//!    生成或计算，模型只负责识别内容并给出**临时引用**。因此本文件里的身份字段是
//!    后端字段，绝不允许从模型输出直接填充。
//! 3. **映射必须显式**。云端首遍与本地并发，模型只能先用临时 ID 自洽引用；本地完成后由后端
//!    按「来源文件 / 题号 / 题组范围」做唯一映射并重写全部引用。映射结果记在
//!    [`CloudAuthoringCandidateV1::id_map`]，**映射不唯一的一律进
//!    [`CloudAuthoringCandidateV1::unresolved_references`]**，交给修复回合结合原文件决定——
//!    绝不「任取第一个」。
//! 4. **未覆盖必须显式**。读不到的区域、证据不完整的区域记在
//!    [`CloudAuthoringCandidateV1::unresolved_regions`]，**不允许用空数组掩盖**。
//! 5. 工具协议采用**应用层 JSON 消息**（模型输出 JSON → Rust 分发 → 结果回传），
//!    因为现有网关只读 `message.content`、没有 native tools 协议。
//!    **不要**再实现第二套 native function calling。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ielts_authoring_v2::IeltsAuthoringIRV2;
use super::recognition_v1::ChainStatusV1;

pub const CLOUD_AUTHORING_CANDIDATE_V1_SCHEMA_VERSION: &str = "CloudAuthoringCandidateV1";
pub const CLOUD_REPAIR_TOOL_CALL_V1_SCHEMA_VERSION: &str = "CloudRepairToolCallV1";
pub const CLOUD_REPAIR_TOOL_RESULT_V1_SCHEMA_VERSION: &str = "CloudRepairToolResultV1";

/// 修复循环允许模型调用的工具名（**唯一真源**）。
///
/// 提示词构造与分发器都必须引用这里，避免「提示词里写了一个、分发器不认」这类漂移。
/// 工具本身就构成越权边界：模型能做的只有「读稿 / 读原文 / 提交一批领域命令 / 声明结束」，
/// 不能执行代码、不能改源码、不能直接写导出 JS、不能标记问题已解决。
pub const CLOUD_REPAIR_TOOLS: [&str; 4] = ["read_draft", "read_source", "apply_edits", "finish"];

// ── 完整候选 ───────────────────────────────────────────────────────────

/// 云端首遍识别产出的**完整候选**。
///
/// 它是**候选**，不是权威稿：落盘走独立的候选 artifact，
/// **不得**调用 `seed_canonical_ds`、不得写 `library_items_v2.canonical_ds_json`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAuthoringCandidateV1 {
    pub schema_version: String,
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
    /// 后端标准化后的完整稿件：`passage` / `taskGroups`（含 instructions、stimulus、
    /// prompts、选项库、responseGroups）/ `answerSlots` / `answerKey` / `assets` 全部保留。
    pub authoring: IeltsAuthoringIRV2,
    /// 模型临时 ID → 后端稳定 ID。重写引用后，稿件里出现过的临时引用必须**全部**
    /// 在本表有对应项，或有明确的未解析记录。
    #[serde(default)]
    pub id_map: BTreeMap<String, String>,
    /// 无法唯一映射的引用（典型：同一题号在两处出现、临时 ID 指向的范围跨题组）。
    ///
    /// 这些**不静默丢弃**：交给修复回合结合原文件决定，绝不任取第一个。
    #[serde(default)]
    pub unresolved_references: Vec<String>,
    /// 未覆盖 / 读不到的区域。空数组表示「确实全覆盖」，不是「没统计」。
    #[serde(default)]
    pub unresolved_regions: Vec<CloudCandidateUnresolvedRegionV1>,
    /// 来源覆盖情况的人话说明（例如 DOCX 图表证据不完整、扫描件页图不可用）。
    #[serde(default)]
    pub source_coverage_notes: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl CloudAuthoringCandidateV1 {
    pub fn is_supported_schema_version(&self) -> bool {
        self.schema_version == CLOUD_AUTHORING_CANDIDATE_V1_SCHEMA_VERSION
    }

    /// 是否存在**尚未处理**的映射缺口：有未解析引用 ⇒ 候选不能直接进入写入。
    pub fn has_unresolved_references(&self) -> bool {
        !self.unresolved_references.is_empty()
    }
}

/// 候选里显式标注的「没读到 / 读不全」区域。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CloudCandidateUnresolvedRegionV1 {
    pub source_file_id: String,
    /// **1-based**，与产品既有约定一致（`0` 是无效页索引，见
    /// `cloud_outline_group_quote_invalid`）。
    pub page_index: u32,
    /// 稳定原因码，便于统计与前端分组。
    pub reason: String,
    /// 给人和模型看的具体说明。
    pub detail: String,
}

// ── 工具消息 ───────────────────────────────────────────────────────────

/// 模型返回的一个工具调用。
///
/// 解析入口是**自由文本里的 JSON**（`message.content`），因此结构保持宽松：
/// 解析失败由调用层转成具体的错误码回给模型，而不是在这里 `deny_unknown_fields`
/// 一刀切——那会让一次无害的多余字段白烧一个模型回合。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRepairToolCallV1 {
    pub call_id: String,
    /// 必须是 [`CLOUD_REPAIR_TOOLS`] 之一；分发器负责校验。
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
}

impl CloudRepairToolCallV1 {
    pub fn is_known_tool(&self) -> bool {
        CLOUD_REPAIR_TOOLS.contains(&self.tool.as_str())
    }
}

/// 工具调用的执行结果（**回给模型的事实**，不是模型的自我描述）。
///
/// 与 [`crate::cloud_repair::tools::CloudEditStatus`] 的区别：后者描述**一次编辑批次**的
/// 结果（applied / rejected），本枚举描述**一次工具调用**的结果。`Ok` 只表示工具本身
/// 执行成功（例如一次读取），不代表内容已被修复。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudRepairToolStatusV1 {
    /// 工具执行成功。
    Ok,
    /// 请求不合法或越权，被**明确拒绝**（模型可据具体错误调整后重试）。
    Rejected,
    /// 后端执行失败（内部错误）。与 `Rejected` 区分：前者是模型该改，后者不该重试同一输入。
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRepairToolResultV1 {
    pub schema_version: String,
    /// 与请求的 `callId` 严格配对；下游按它拼接 observation。
    pub call_id: String,
    pub status: CloudRepairToolStatusV1,
    /// 工具的实质返回（读到的片段、applied 目标、editVersion 等）。
    #[serde(default)]
    pub result: Value,
    /// 具体错误码与原因。**必须具体**：模型要能据此缩小修复范围，
    /// 而不是收到一句笼统的「失败」。
    #[serde(default)]
    pub errors: Vec<String>,
}

impl CloudRepairToolResultV1 {
    pub fn ok(call_id: &str, result: Value) -> Self {
        Self {
            schema_version: CLOUD_REPAIR_TOOL_RESULT_V1_SCHEMA_VERSION.to_string(),
            call_id: call_id.to_string(),
            status: CloudRepairToolStatusV1::Ok,
            result,
            errors: Vec::new(),
        }
    }

    pub fn rejected(call_id: &str, errors: Vec<String>) -> Self {
        Self {
            schema_version: CLOUD_REPAIR_TOOL_RESULT_V1_SCHEMA_VERSION.to_string(),
            call_id: call_id.to_string(),
            status: CloudRepairToolStatusV1::Rejected,
            result: Value::Null,
            errors,
        }
    }
}
