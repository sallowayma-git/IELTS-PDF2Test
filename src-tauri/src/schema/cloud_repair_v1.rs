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

/// `report_insufficient_context`：模型认为手里的校核包**不够**时唯一的正确表达。
///
/// 为什么必须有它：以前模型只有「改稿」「裁定」「闭嘴」三种表达。上下文不够时它只有
/// 两条路——硬猜（伪造出处）或者沉默（差异留给用户）。两者都是错的：前者的引文经不起
/// 核对，后者的用户永远不知道云端其实缺了材料。有了这个工具，「不够」变成一条**可记录、
/// 可满足、可升级**的事实。
pub const CLOUD_REPAIR_INSUFFICIENT_CONTEXT_TOOL: &str = "report_insufficient_context";
/// `finish_packet`：声明**本包**处理完毕（整次运行的收尾由后端汇总）。
pub const CLOUD_REPAIR_FINISH_PACKET_TOOL: &str = "finish_packet";

/// 上下文不足时代为落盘的裁定理由码。
///
/// 它同时是「这条差异**没有**被核对过」的标记：预算用尽仍不足时，后端对本包剩余差异
/// 写一条 `cannot_resolve`，理由就是这个码。**绝不能**把它折叠成「已核对」。
pub const CLOUD_RULING_REASON_CONTEXT_INSUFFICIENT: &str = "CONTEXT_INSUFFICIENT";

/// 修复循环允许模型调用的工具名（**唯一真源**）。
///
/// 提示词构造与分发器都必须引用这里，避免「提示词里写了一个、分发器不认」这类漂移。
/// 工具本身就构成越权边界：模型能做的只有「读稿 / 读原文 / 提交一批领域命令 /
/// 对已发现的差异作出裁定 / 声明结束」，不能执行代码、不能改源码、不能直接写导出 JS、
/// 不能标记问题已解决。
///
/// `record_ruling` 为什么必须存在：首遍云端候选**也只是输入**，它同样会错。没有这个
/// 工具，模型只有两种表达方式——改稿（`apply_edits`）或闭嘴（`finish`）。于是「候选
/// 错了、当前稿是对的」这种判断无处安放，差异会被逐条变成人工任务回来问用户，哪怕
/// 模型已经看过原文并确定候选是错的。裁定记录的是**结论**，不是编辑。
///
/// 抓取类工具（`search_source` / `read_page_region` / `read_passage` /
/// `read_candidate`）全部只读、不接受路径、只作用于本 job，且受每包预算约束
/// （见 `cloud_repair::grab::GrabBudget`）。它们存在的理由与校核包是同一件事：
/// 上下文不再一次性给全，模型必须能**主动**取回它真正需要的那一块。
pub const CLOUD_REPAIR_TOOLS: [&str; 11] = [
    "read_draft",
    "read_source",
    "search_source",
    "read_page_region",
    "read_passage",
    "read_candidate",
    "apply_edits",
    "record_ruling",
    "report_insufficient_context",
    "finish_packet",
    "finish",
];

/// 模型可以声明的「我需要什么」的种类（**唯一真源**，prompt 与满足器都引用这里）。
pub const CLOUD_CONTEXT_NEED_KINDS: [&str; 6] = [
    "pages",
    "search",
    "page_region",
    "passage",
    "candidate",
    "draft",
];

/// 一条「上下文不足」的需求。
///
/// 结构刻意宽松（不 `deny_unknown_fields`）：这是从自由文本里的 JSON 解析出来的，
/// 多一个无害字段不该白烧一个模型回合。但 `kind` 必须是 [`CLOUD_CONTEXT_NEED_KINDS`]
/// 之一，且**该带的字段必须带**——否则后端只能猜，而猜出来的上下文正是要避免的东西。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRepairContextNeedV1 {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Value>,
    #[serde(default)]
    pub paragraph_labels: Vec<String>,
    #[serde(default)]
    pub question_numbers: Vec<u32>,
    #[serde(default)]
    pub task_ids: Vec<String>,
}

impl CloudRepairContextNeedV1 {
    /// 结构校验。错误必须**具体到该补哪个字段**，模型才能一次改对。
    pub fn validate(&self) -> Result<(), String> {
        if !CLOUD_CONTEXT_NEED_KINDS.contains(&self.kind.as_str()) {
            return Err(format!(
                "CLOUD_NEED_UNKNOWN_KIND:{}: allowed are {}",
                self.kind,
                CLOUD_CONTEXT_NEED_KINDS.join(", ")
            ));
        }
        match self.kind.as_str() {
            "pages" => match (self.from, self.to) {
                (Some(from), _) if from >= 1 => Ok(()),
                _ => Err("CLOUD_NEED_MALFORMED:pages: needs {\"from\": N} (and optionally \"to\")".to_string()),
            },
            "search" => match self.quote.as_deref().map(str::trim) {
                Some(quote) if !quote.is_empty() => Ok(()),
                _ => Err("CLOUD_NEED_MALFORMED:search: needs a non-empty \"quote\"".to_string()),
            },
            "page_region" => match self.page_index {
                Some(page) if page >= 1 => Ok(()),
                _ => Err("CLOUD_NEED_MALFORMED:page_region: needs {\"pageIndex\": N} (1-based)".to_string()),
            },
            "passage" => {
                if self.paragraph_labels.is_empty() && self.question_numbers.is_empty() {
                    Err(
                        "CLOUD_NEED_MALFORMED:passage: needs \"paragraphLabels\" or \"questionNumbers\""
                            .to_string(),
                    )
                } else {
                    Ok(())
                }
            }
            "candidate" => {
                if self.task_ids.is_empty() && self.question_numbers.is_empty() {
                    Err(
                        "CLOUD_NEED_MALFORMED:candidate: needs \"taskIds\" or \"questionNumbers\""
                            .to_string(),
                    )
                } else {
                    Ok(())
                }
            }
            "draft" => {
                if self.task_ids.is_empty() && self.question_numbers.is_empty() {
                    Err(
                        "CLOUD_NEED_MALFORMED:draft: needs \"taskIds\" or \"questionNumbers\""
                            .to_string(),
                    )
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }
}


/// 裁定的两种结论。**只有这两种**：模型不能通过裁定声称「已修好」。
///
/// 刻意不提供「已修复」这类取值：修好没有修好，看的是当前稿本身与程序校验，
/// 不是模型的一句话。
pub const CLOUD_RULING_CURRENT_IS_CORRECT: &str = "current_is_correct";
pub const CLOUD_RULING_CANNOT_RESOLVE: &str = "cannot_resolve";

/// 一条差异裁定：模型对**某个具体差异**给出的结论。
///
/// 关键在最后两个字段：裁定绑定它当时看到的那一对内容（当前稿一侧 / 候选一侧）。
/// 任一侧后来变了，这份裁定的前提就不存在了，必须重新评估——否则一次基于旧内容的
/// 「我确认当前稿是对的」会永久掩盖后来才出现的问题。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRepairRulingV1 {
    pub target_type: String,
    pub target_id: String,
    pub field: String,
    /// [`CLOUD_RULING_CURRENT_IS_CORRECT`] 或 [`CLOUD_RULING_CANNOT_RESOLVE`]。
    pub ruling: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 原文依据（pageIndex 为 **1-based**）。裁定「当前稿对」必须能指出出处。
    #[serde(default)]
    pub evidence: Vec<Value>,
    /// 裁定当时「当前稿」一侧的内容指纹。
    pub canonical_digest: String,
    /// 裁定当时「云端候选」一侧的内容指纹。
    pub candidate_digest: String,
    /// 第几个回合作出的（诊断用）。
    pub recorded_at_round: u32,
}

impl CloudRepairRulingV1 {
    pub fn is_known_kind(&self) -> bool {
        matches!(
            self.ruling.as_str(),
            CLOUD_RULING_CURRENT_IS_CORRECT | CLOUD_RULING_CANNOT_RESOLVE
        )
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn need(value: Value) -> Result<(), String> {
        serde_json::from_value::<CloudRepairContextNeedV1>(value)
            .expect("need 必须能反序列化")
            .validate()
    }

    /// 六种需求各自「该带的字段必须带」——缺了就报**具体到该补哪个字段**的错误。
    /// 后端没法替模型猜它想要哪一页；猜出来的上下文正是这套协议要避免的东西。
    #[test]
    fn every_context_need_kind_requires_its_own_fields() {
        for kind in CLOUD_CONTEXT_NEED_KINDS {
            let error = need(json!({"kind": kind}))
                .expect_err("只给 kind 必须被拒（该带什么都没说）");
            assert!(
                error.starts_with(&format!("CLOUD_NEED_MALFORMED:{kind}")),
                "{kind}: 错误必须具体到该补什么：{error}"
            );
        }
    }

    /// 每种需求的**合法**形状必须通过——否则模型照契约交也会被拒，等于死循环。
    #[test]
    fn the_declared_shape_of_every_context_need_kind_passes() {
        for value in [
            json!({"kind": "pages", "from": 7, "to": 7}),
            json!({"kind": "search", "quote": "Questions 14-20"}),
            json!({"kind": "page_region", "pageIndex": 3, "bbox": [0, 0, 1, 1]}),
            json!({"kind": "passage", "paragraphLabels": ["C", "D"]}),
            json!({"kind": "candidate", "taskIds": ["cloud-tg-1"]}),
            json!({"kind": "draft", "questionNumbers": [14, 15]}),
        ] {
            need(value.clone()).unwrap_or_else(|error| panic!("{value} 必须通过：{error}"));
        }
    }

    /// 未知 kind 必须被拒，且把允许的取值列出来。
    #[test]
    fn an_unknown_context_need_kind_is_rejected_with_the_allowed_list() {
        let error = need(json!({"kind": "everything"})).expect_err("未知 kind 必须被拒");
        assert!(error.starts_with("CLOUD_NEED_UNKNOWN_KIND:everything"), "{error}");
        assert!(error.contains("pages"), "必须列出允许的取值：{error}");
    }

    /// 空白引文不算引文：拿空白去搜原文只会得到「搜不到」，白烧一个回合。
    #[test]
    fn a_blank_search_quote_is_not_a_quote() {
        let error = need(json!({"kind": "search", "quote": "   "})).expect_err("空白引文必须被拒");
        assert!(error.starts_with("CLOUD_NEED_MALFORMED:search"), "{error}");
    }

    /// 第 0 页不存在：页号一律 1-based，0 是无效页索引。
    #[test]
    fn page_numbers_are_one_based() {
        assert!(need(json!({"kind": "pages", "from": 0})).is_err());
        assert!(need(json!({"kind": "page_region", "pageIndex": 0})).is_err());
        assert!(need(json!({"kind": "pages", "from": 1})).is_ok());
    }

    /// 上下文不足是**三态**里独立的一态：它必须有自己的工具名与理由码，
    /// 不能被折叠成「已核对」。
    #[test]
    fn insufficient_context_has_its_own_tool_and_reason_code() {
        assert!(CLOUD_REPAIR_TOOLS.contains(&CLOUD_REPAIR_INSUFFICIENT_CONTEXT_TOOL));
        assert!(CLOUD_REPAIR_TOOLS.contains(&CLOUD_REPAIR_FINISH_PACKET_TOOL));
        assert_eq!(CLOUD_RULING_REASON_CONTEXT_INSUFFICIENT, "CONTEXT_INSUFFICIENT");
        // 裁定语义只有两种，理由码不在其中——它描述的是「没核对过」，不是一种结论。
        assert_ne!(CLOUD_RULING_REASON_CONTEXT_INSUFFICIENT, CLOUD_RULING_CURRENT_IS_CORRECT);
        assert_ne!(CLOUD_RULING_REASON_CONTEXT_INSUFFICIENT, CLOUD_RULING_CANNOT_RESOLVE);
    }
}
