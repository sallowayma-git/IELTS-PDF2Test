//! 云端校核修复：让模型**真的执行编辑**，而不是输出建议卡。
//!
//! 三条职责固定不变：本地快速初稿、云端独立识别、云端校核并执行修复。用户只处理
//! 云端解决不了的剩余问题，不再逐项接受模型建议。
//!
//! 数据对象严格区分（不可互相冒充）：
//! - `localSnapshot`  本地识别那一刻的**不可变证据**（冻结候选）；
//! - `cloudCandidate` 云端独立识别产出的完整候选（独立 artifact，不写权威稿）；
//! - `canonical`      **当前**权威稿，预览与导出唯一的来源。
//!
//! 本模块做两件事：
//! - [`tools`]：受限编辑工具的真实执行器（可信入口、授权检查、事务写入、整批撤销）；
//! - 本文件：**修复回合的编排**——有限轮次、总超时、取消、运行归属，以及
//!   「模型输出工具消息 → Rust 真实执行 → 真实结果回传 → 模型继续修」的闭环。
//!
//! 纪律（写在最显眼处，避免后来者顺手放宽）：
//! 1. 模型**没有**任意执行代码、改源码或直接覆盖导出 JS 的能力；
//! 2. 模型**不能**通过 `resolveIssue`、修改 quality 标记或忽略问题来制造"已完成"；
//! 3. 修改只能落在权威稿的工作副本上，经结构与引用检查后由**一个**事务写入；
//! 4. 原文没有提供的答案不得以"识别修复"的名义生成；
//! 5. 模型 `finish` **不等于**产品完成：剩余问题一律由后端按当前 canonical 重算。

pub(crate) mod grab;
pub(crate) mod packets;
pub(crate) mod tools;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use serde_json::{json, Value};

use crate::library::repository::{get_canonical_ds, human_protected_targets, open_library_connection};
use crate::reconcile::candidate::{nodes_text, normalize_text};
use crate::reconcile::store;
use crate::schema::cloud_repair_v1::{CloudRepairToolCallV1, CloudRepairToolResultV1};
use crate::CommandResult;

/// 默认模型回合上限。每一次读取、格式纠正都计入这个预算。
pub(crate) const DEFAULT_MAX_REPAIR_ROUNDS: u32 = 6;
/// 整次修复的总超时（含 HTTP 重试时间，不叠加旧 A3/A4 的各自预算）。
pub(crate) const DEFAULT_REPAIR_TIMEOUT_MS: u64 = 10 * 60 * 1000;
/// 连续多少次「完全相同的工具调用且没有产生任何进展」就停下。
const REPEAT_LIMIT: u32 = 2;
/// 每个**校核包**的模型回合预算（包模式下按包独立计数）。
const PACKET_MAX_ROUNDS: u32 = 5;
/// 升级阶梯的最高级别（L4 = 后端代记 `cannot_resolve`，理由码 `CONTEXT_INSUFFICIENT`）。
const PACKET_MAX_ESCALATION: u32 = 4;
/// 每多一个校核包，总时限在**基础时限**上再放宽的比例（0.25 = 基础 10 分钟 ⇒ +2.5 分钟/包）。
///
/// 真实模型每轮 16–50 秒、每包最多 5 轮：10 个包按旧常数 10 分钟几乎必然
/// `budget_exhausted`——包是顺序处理的，总时限却是按「整卷一轮」拍的常数。
const PACKET_DEADLINE_RATE: f64 = 0.25;
/// 放宽的封顶：总时限最多是基础时限的 3 倍（基础 10 分钟 ⇒ 最多 30 分钟）。
/// 没有上限的线性放宽等于没有时限。
const PACKET_DEADLINE_MAX_RATIO: f64 = 3.0;

/// 包模式的总时限：按包数在基础时限上线性放宽，封顶 [`PACKET_DEADLINE_MAX_RATIO`]。
///
/// `request.deadline` 是调用方按「整卷一轮」口径给的绝对时刻；这里在**循环开工时**取
/// 「基础时长 = deadline − now」，乘上比例后从同一时刻起算。单包（比例 1）行为与
/// 旧常数完全一致；重切出的新包**不再**二次放宽（否则编辑-重切循环可以无限续期，
/// 时限就名存实亡了）。取消、失败终态、进度上报都不经过它，行为不变。
fn scaled_packet_deadline(
    request_deadline: Instant,
    started_at: Instant,
    packet_count: usize,
) -> Instant {
    let base = request_deadline
        .checked_duration_since(started_at)
        .unwrap_or_default();
    let ratio = 1.0 + PACKET_DEADLINE_RATE * (packet_count.saturating_sub(1)) as f64;
    let ratio = ratio.min(PACKET_DEADLINE_MAX_RATIO);
    started_at + base.mul_f64(ratio)
}

/// 上下文管理方式。
///
/// - `Packets`（生产默认）：本地预切校核包，模型不够就报、就自己去取；
/// - `Legacy`：改造前的行为（每轮附整份原文件 + 整卷上下文）。**只**保留给 L3 的最后
///   手段与回归对照，不对用户暴露（见 [`configured_repair_context_mode`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepairContextMode {
    Packets,
    Legacy,
}

/// 生产默认模式。
pub(crate) const REPAIR_CONTEXT_MODE: RepairContextMode = RepairContextMode::Packets;

/// 本次运行实际使用的模式。
///
/// 诊断用开关 `IELTS_REPAIR_CONTEXT_MODE=legacy` 可以把一次运行打回旧路径，用来做
/// 「同一份卷子、两种模式」的输入量对比（任务书 §6）。它不是用户设置。
pub(crate) fn configured_repair_context_mode() -> RepairContextMode {
    match std::env::var("IELTS_REPAIR_CONTEXT_MODE").ok().as_deref() {
        Some("legacy") => RepairContextMode::Legacy,
        _ => REPAIR_CONTEXT_MODE,
    }
}

pub(crate) const REPAIR_STATUS_RUNNING: &str = "running";
pub(crate) const REPAIR_STATUS_COMPLETED: &str = "completed";
pub(crate) const REPAIR_STATUS_NEEDS_ATTENTION: &str = "needs_attention";
pub(crate) const REPAIR_STATUS_CANCELLED: &str = "cancelled";
pub(crate) const REPAIR_STATUS_BUDGET_EXHAUSTED: &str = "budget_exhausted";
pub(crate) const REPAIR_STATUS_UNAVAILABLE: &str = "unavailable";

/// 一次修复运行的输入。
pub(crate) struct RepairRunRequest<'a> {
    pub root: &'a Path,
    pub item_id: &'a str,
    pub job_id: &'a str,
    pub batch_id: &'a str,
    pub repair_run_id: &'a str,
    pub max_rounds: u32,
    pub deadline: Instant,
    /// 取消探针。由调用方（调度器）提供真实的取消状态。
    pub cancelled: &'a dyn Fn() -> bool,
    /// 进度上报。**每落下一批有效修改就调一次**，调用方据此把「已修几处、现在是哪个
    /// 版本」写进产品状态并通知界面。
    ///
    /// 为什么必须存在：修复循环的真实预算是十分钟。若只在循环结束时上报一次，用户在这
    /// 十分钟里看不到任何变化——稿子已经改好了几处，画布却还是旧的，界面只会说「云端
    /// 识别中」。这条缝以前确实存在，且只表现为「好像有点慢」，因此必须有显式出口。
    ///
    /// `None` = 调用方不需要进度（测试、或没有可写状态的地方）。
    pub progress: Option<&'a dyn Fn(RepairProgress)>,
}

/// 一次进度上报的内容。
///
/// `remaining_tasks` **故意不在这里**：循环进行中的差异正是它正在处理的东西，把中间态
/// 当成用户待办会制造一批「刚列出来就被修掉」的假任务。剩余任务只在循环结束后由后端
/// 按当前 canonical 重算一次。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RepairProgress {
    pub status: &'static str,
    pub round: u32,
    pub applied_count: usize,
    /// 已经落盘、且**此刻仍然有效**的裁定条数。
    ///
    /// 以前这里硬编码 0，理由是「进行中不重算剩余任务」。那个理由对剩余任务是成立的，
    /// 对裁定条数不成立：裁定是**已经发生的事实**（模型已经看过原文、已经下过判断），
    /// 它不需要等循环结束才成立。硬编码 0 的后果是用户在十分钟的修复里，看到的一直是
    /// 「已了结 0 处差异」，循环结束时那个数字突然跳到真实值——一个纯粹由上报方式
    /// 造出来的假象。
    pub adjudicated_count: usize,
    pub edit_version: i64,
}

impl RepairProgress {
    /// 写进批次行的「进行中」摘要。
    ///
    /// 与 [`RepairRunReport::to_json`] 共用同一组键名：前端只认一份形状，多一套字段名
    /// 就意味着前端要维护两套解析，漏一套时表现为「修复中面板空白」。
    pub fn to_json(&self) -> Value {
        json!({
            "status": self.status,
            "editVersion": self.edit_version,
            "appliedCount": self.applied_count,
            "rounds": self.round,
            // 进行中：剩余任务尚未重算，**不能**拿中间差异充数。
            "remainingTasks": [],
            "adjudicatedCount": self.adjudicated_count,
            "finishNote": Value::Null,
            "lastError": Value::Null,
            // 进行中**不**提供撤销入口：循环还在写，此时撤销会与 journal 错位。
            // 撤销只在循环结束后由最终摘要给出（那时 `repairRunId` 才是权威的）。
            "undoAvailable": false,
            "repairRunId": Value::Null,
        })
    }
}

/// 上报一次进度；调用方没给 sink 时什么也不做。
fn report_progress(request: &RepairRunRequest<'_>, progress: RepairProgress) {
    if let Some(sink) = request.progress {
        sink(progress);
    }
}

/// 修复运行的结果。**这是后端按当前 canonical 重算出来的事实**，不是模型的自我描述。
#[derive(Debug, Clone)]
pub(crate) struct RepairRunReport {
    pub status: &'static str,
    pub rounds: u32,
    pub edit_version: i64,
    pub applied_count: usize,
    /// 每个回合的 observation（诊断副本，便于事后复盘真实执行了什么）。
    pub observations: Vec<Value>,
    /// 剩余必须由用户处理的问题（由后端重算，不是模型声称的清单）。
    pub remaining_tasks: Vec<Value>,
    /// 模型对差异作出的裁定条数（含「当前稿对、候选错」与「原文件不足以定论」）。
    ///
    /// 单独给前端：这是「云端替用户了结了多少争议」的唯一可核对数字。没有它，
    /// 用户只能看到「还剩几件事」，看不出云端到底做了什么。
    pub adjudicated_count: usize,
    pub finish_note: Option<String>,
    pub last_error: Option<String>,
    /// 本次修复 run 的标识。
    ///
    /// 必须随摘要一起给前端：撤销入口要用它调用 Rust 批次撤销
    /// （`cloud_repair::tools::undo_repair`）。让前端自己拼 `cloud-repair:{batchId}`
    /// 等于把后端内部命名规则复制到前端——命名一变，撤销就静默失效。
    pub repair_run_id: String,
    /// 包模式的**逐包诊断**（每个包的轮数、升级级别、裁定数、编辑数、上下文不足条数）。
    ///
    /// 只进 `repair_json` 的诊断区，前端展示不变。它存在的理由：包模式最怕的就是
    /// 「输入量下来了、但模型其实什么都没核」——那只能靠逐包记录才看得出来。
    pub packets: Vec<Value>,
    /// 整次运行里「证据无法核验」的条数（原文没有文本层）。
    ///
    /// 摘要如实带出：这些证据**没有**对照原文核验过，不能被「已应用 / 已了结」的
    /// 数字盖成「已核验」。前端暂不展示，但审计与对账都读它。
    pub unverified_evidence: usize,
}

impl RepairRunReport {
    /// 写进批次记录的 `repair_json`（前端只认这一份）。
    pub fn to_json(&self, undo_available: bool) -> Value {
        json!({
            "status": self.status,
            "editVersion": self.edit_version,
            "appliedCount": self.applied_count,
            "rounds": self.rounds,
            "remainingTasks": self.remaining_tasks,
            "adjudicatedCount": self.adjudicated_count,
            "finishNote": self.finish_note,
            "lastError": self.last_error,
            "undoAvailable": undo_available,
            "repairRunId": self.repair_run_id,
            "packets": self.packets,
            "unverifiedEvidence": self.unverified_evidence,
        })
    }
}

/// 权威稿的题组索引（`taskId` → 组）。
fn groups_by_id(document: &Value) -> BTreeMap<String, &Value> {
    document
        .get("taskGroups")
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter_map(|group| {
                    let task_id = group.get("taskId").and_then(Value::as_str)?;
                    Some((task_id.to_string(), group))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 一个题组在「整卷范围索引」里的紧凑摘要。
///
/// 模型必须能看到**整份文档**的范围，不能只看到已经发现的差异就宣称整卷核验完成。
fn group_index_entry(group: &Value) -> Value {
    let numbers: Vec<u64> = group
        .get("displayRange")
        .map(crate::reconcile::candidate::expand_question_numbers)
        .unwrap_or_default()
        .into_iter()
        .map(u64::from)
        .collect();
    let slot_ids: Vec<String> = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter_map(|response| response.get("slotIds").and_then(Value::as_array))
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    json!({
        "taskId": group.get("taskId").cloned().unwrap_or(Value::Null),
        "taskType": group.get("taskType").cloned().unwrap_or(Value::Null),
        "questionNumbers": numbers,
        "slotIds": slot_ids,
        "instructionsText": nodes_text(group.get("instructions").unwrap_or(&Value::Null)),
        "optionBankId": group.pointer("/optionBank/optionBankId").cloned().unwrap_or(Value::Null),
    })
}

/// 听力 Part 按 `partId` 建索引。阅读卷、或候选没带听力结构时是空表。
fn listening_parts_by_id(document: &Value) -> BTreeMap<String, &Value> {
    document
        .get("listening")
        .and_then(|listening| listening.get("parts"))
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    let part_id = part.get("partId").and_then(Value::as_str)?;
                    Some((part_id.to_string(), part))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 一个听力 Part 在差异报告里的摘要。
///
/// **刻意不含 `media`**：音频是内容寻址的后端事实（`assetId` 就是哈希），把哈希抄进
/// 给模型看的上下文既没用又等于泄题——模型无权也无法核对它。
fn part_index_entry(part: &Value) -> Value {
    json!({
        "partId": part.get("partId").cloned().unwrap_or(Value::Null),
        "displayLabel": part.get("displayLabel").cloned().unwrap_or(Value::Null),
        "expectedQuestionNumbers": part
            .get("expectedQuestionNumbers")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "taskIds": part.get("taskIds").cloned().unwrap_or_else(|| json!([])),
    })
}

/// Part 裁定所依赖的内容：这一段的身份与范围（同样排除 `media`）。
///
/// 排除的理由和 `part_index_entry` 一致，但这里还多一层：把音频指纹算进裁定前提，
/// 会让「用户重新绑定音频」无端作废一条本来有效的分段裁定，白跑一轮模型。
fn part_context(document: &Value, part_id: &str) -> Value {
    let mut entry = listening_parts_by_id(document)
        .get(part_id)
        .map(|part| (*part).clone())
        .unwrap_or(Value::Null);
    if let Some(object) = entry.as_object_mut() {
        object.remove("media");
    }
    entry
}

/// 当前稿件里需要处理的诊断（阻断与警告分开标注，模型必须知道哪些是硬问题）。
fn quality_issues(document: &Value) -> Vec<Value> {
    document
        .pointer("/quality/issues")
        .and_then(Value::as_array)
        .map(|issues| {
            issues
                .iter()
                .filter_map(|issue| {
                    let object = issue.as_object()?;
                    let severity = object.get("severity").and_then(Value::as_str)?;
                    if severity == "info" {
                        return None;
                    }
                    Some(json!({
                        "code": object.get("code").cloned().unwrap_or(Value::Null),
                        "severity": severity,
                        "message": object.get("message").cloned().unwrap_or(Value::Null),
                        "targetType": object.get("targetType").cloned().unwrap_or(Value::Null),
                        "targetId": object.get("targetId").cloned().unwrap_or(Value::Null),
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 选项库摘要：标签 + 文本（用于差异比对与人工核对，不用于写入）。
fn option_bank_digest(group: &Value) -> Vec<Value> {
    group
        .pointer("/optionBank/options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .map(|option| {
                    json!({
                        "label": option.get("label").cloned().unwrap_or(Value::Null),
                        "text": nodes_text(option.get("content").unwrap_or(&Value::Null)),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 一个题组覆盖的答案槽（键 → 答案值）。
fn group_answers(document: &Value, group: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    if document.get("answerSlots").and_then(Value::as_object).is_none() {
        return out;
    }
    let keys: BTreeSet<String> = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter_map(|response| response.get("slotIds").and_then(Value::as_array))
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let Some(answers) = document.get("answerKey").and_then(Value::as_object) else {
        return out;
    };
    for key in keys {
        out.insert(
            key.clone(),
            answers.get(&key).cloned().unwrap_or(Value::Null),
        );
    }
    out
}

fn push_difference(out: &mut Vec<Value>, target_type: &str, target_id: &str, field: &str, current: Value, candidate: Value) {
    out.push(json!({
        "targetType": target_type,
        "targetId": target_id,
        "field": field.clone(),
        "canonical": current,
        "candidate": candidate,
    }));
}

/// 一条差异的身份：`(targetType, targetId, field)`。
///
/// 裁定的作用域就是它。**不是**按数组下标或文本相似度匹配——数组顺序不是稳定身份，
/// 文本会在裁定之后被改掉。
fn difference_key(difference: &Value) -> (String, String, String) {
    let part = |key: &str| {
        difference
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    (part("targetType"), part("targetId"), part("field"))
}

/// 把任意 JSON 序列化成**与对象键顺序无关**的规范字符串。
///
/// 为什么不能直接 `serde_json::to_string`：指纹要跨进程比较（裁定落盘、下次运行再读），
/// 而 `Value::Object` 的迭代顺序取决于构造路径。用键排序后的规范形式，才能保证
/// 「内容没变 ⇒ 指纹相同」。
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner = keys
                .iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canonical_json(&map[*key])
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical_json).collect::<Vec<_>>().join(",")
        ),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// 裁定**所依赖的内容**的指纹（只看当前 canonical 一侧）。
///
/// 为什么光有「差异两侧的内容指纹」不够：裁定说的是「基于当前稿的某个状态，候选读错了」。
/// 差异两侧逐字没变、但**当前稿里裁定所依据的那部分内容**变了，这条裁定就已经变质，
/// 继续拿它压住差异等于用旧结论回答新问题。两个真实形状：
///
///   · 裁定「答案 B 对」，依据是选项库把 B 映射到某个词；随后**选项库被改**——
///     答案值一个字没变，`canonicalDigest` 也就没变，但裁定的依据已经没了。
///   · 裁定「这一组保持现状」（`task_group` 级差异）。那一类差异的指纹取自
///     [`group_index_entry`]——一个给模型看的**摘要**，只含 taskId / 题型 / 题号 /
///     slotIds / 说明文字 / 选项库 id。随后该组的 stimulus、题面、选项文本、答案
///     全被改写，摘要里一个字都没变，裁定照样「有效」。
///
/// 所以指纹按**目标**取该目标所依赖的完整内容，而不是取摘要。范围刻意取到「承载它的
/// 那个题组」，因为答案/选项/题面在语义上是互相定义的（改了选项库，答案的含义就变了）。
fn target_context_fingerprint(canonical: &Value, target_type: &str, target_id: &str) -> String {
    let groups = canonical.get("taskGroups").and_then(Value::as_array);
    let group_containing = |predicate: &dyn Fn(&Value) -> bool| -> Value {
        groups
            .into_iter()
            .flatten()
            .find(|group| predicate(group))
            .cloned()
            .unwrap_or(Value::Null)
    };
    match target_type {
        "slot" => {
            let slot = canonical.pointer(&format!("/answerSlots/{target_id}"));
            let answer = canonical.pointer(&format!("/answerKey/{target_id}"));
            let owning = group_containing(&|group: &Value| {
                group
                    .get("responseGroups")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|response| {
                        response
                            .get("slotIds")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .any(|id| id.as_str() == Some(target_id))
                    })
            });
            canonical_json(&json!({
                "slot": slot.cloned().unwrap_or(Value::Null),
                "answer": answer.cloned().unwrap_or(Value::Null),
                "group": owning,
            }))
        }
        "response_group" => {
            let owning = group_containing(&|group: &Value| {
                group
                    .get("responseGroups")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|response| {
                        response.get("responseGroupId").and_then(Value::as_str) == Some(target_id)
                    })
            });
            canonical_json(&owning)
        }
        // 听力 Part 的裁定前提就是这一段的身份与范围（不含音频事实）。
        "part" => canonical_json(&part_context(canonical, target_id)),
        // `task_group` 与其余：**整组内容**。刻意比 `group_index_entry` 宽——
        // 索引摘要是给模型看的概览，不是裁定的依据。
        _ => {
            let group = group_containing(&|group: &Value| {
                group.get("taskId").and_then(Value::as_str) == Some(target_id)
            });
            canonical_json(&group)
        }
    }
}

/// 一条差异的**完整**前提指纹 `(当前稿一侧, 候选一侧, 裁定依据)`。
fn difference_digests(difference: &Value) -> (String, String, String) {
    (
        canonical_json(difference.get("canonical").unwrap_or(&Value::Null)),
        canonical_json(difference.get("candidate").unwrap_or(&Value::Null)),
        difference
            .get("contextDigest")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    )
}

/// 从裁定记录里找出**对这条差异仍然有效**的那一条。
///
/// 有效性 = 身份一致 **且** 两侧内容指纹与**依据内容指纹**都没变。任一项变了，裁定
/// 当时的前提就不存在了：一次基于旧内容的「我确认当前稿是对的」不能永久掩盖后来才
/// 出现的问题，也不能在依据被改写后继续生效。
///
/// 同一条差异有多份裁定时取**最后**一份（最新的判断覆盖旧的）。
fn fresh_ruling_for_difference<'a>(rulings: &'a [Value], difference: &Value) -> Option<&'a Value> {
    let (target_type, target_id, field) = difference_key(difference);
    let (canonical_digest, candidate_digest, context_digest) = difference_digests(difference);
    rulings.iter().rev().find(|ruling| {
        ruling.get("targetType").and_then(Value::as_str) == Some(target_type.as_str())
            && ruling.get("targetId").and_then(Value::as_str) == Some(target_id.as_str())
            && ruling.get("field").and_then(Value::as_str) == Some(field.as_str())
            && ruling.get("canonicalDigest").and_then(Value::as_str)
                == Some(canonical_digest.as_str())
            && ruling.get("candidateDigest").and_then(Value::as_str)
                == Some(candidate_digest.as_str())
            // 旧裁定（本条改动之前落盘的）没有这个字段 → 取空串 → 与真实指纹不等 →
            // 自然失效重评。宁可多问一次，也不能拿一条依据不明的旧结论压住差异。
            && ruling.get("contextDigest").and_then(Value::as_str).unwrap_or_default()
                == context_digest.as_str()
    })
}

/// 「上下文不足」这条用户任务的固定文案。
///
/// 为什么必须与「云端查过但定不了」分开：前者是**云端没拿到材料**，后者是云端拿到了
/// 材料但定不下结论。混成一句会让用户以为云端已经核过原文——那正是「上下文不足绝不
/// 算作已核对」这条边界要挡住的东西。措辞按任务书 §4.2 固定。
fn context_insufficient_message(ruling: &Value) -> String {
    let numbers: Vec<u64> = ruling
        .get("questionNumbers")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    let subject = match numbers.as_slice() {
        [] => "这处差异".to_string(),
        [one] => format!("第 {one} 题"),
        [first, .., last] => format!("第 {first}-{last} 题"),
    };
    format!("云端没能拿到足够的原文来判断{subject}，请对照原文确认")
}

/// 一条差异的人话说明（任务文案用）。
fn describe_difference(difference: &Value) -> String {
    let (target_type, target_id, field) = difference_key(difference);
    let label = match field.as_str() {
        "answer" => "答案",
        "prompt" => "题面",
        "instructions" => "作答说明",
        "stimulus" => "材料",
        "option_bank" => "选项库",
        "task_group" => "整组",
        "part_boundary" => "分段范围",
        "part_label" => "段落标签",
        "part_tasks" => "所属题组",
        _ => "内容",
    };
    let where_ = match target_type.as_str() {
        "slot" => format!("第 {target_id} 题"),
        "response_group" => format!("题组内的作答区（{target_id}）"),
        "task_group" => format!("题组 {target_id}"),
        "part" => format!("听力 Part {target_id}"),
        other => format!("{other} {target_id}"),
    };
    format!("{where_}的{label}与云端识别结果不一致")
}

/// 当下**仍然有效**的裁定条数（「云端替用户了结了多少争议」的唯一可核对数字）。
///
/// 为什么不能用 `rulings.len()`：那份列表是 append-only 的累积记录，里面同时躺着
/// 历史的（上一批留下的）、重复的（模型对同一条差异反复裁定）、以及**已经失效的**
/// （差异两侧内容变了，指纹对不上）。把它们算成「云端替用户了结了多少争议」是虚报：
/// 用户会看到一个比实际大得多、且会随重跑次数单调增长的数字。
///
/// 这里只数「此刻仍然存在于候选差异里、且裁定仍然有效」的差异，并按身份去重。
///
/// 边界（刻意如此）：被模型**改掉**（而非裁定掉）的差异不再出现在候选差异里，因此
/// 不计入这里——它已经计入 `appliedCount`（「已自动修正 N 处」）。两个数字不重叠。
///
/// 第二条边界：理由码为 [`CLOUD_RULING_REASON_CONTEXT_INSUFFICIENT`] 的裁定**不算**。
/// 那一条的语义是「云端根本没拿到材料，这条差异没被核对过」——它确实会进用户清单
/// （见 `remaining_tasks` 里 `contextInsufficient` 那一支），但把它同时算进「云端替用户
/// 了结了多少争议」，用户就会一边看到「已了结 1 处差异」，一边看到「云端没能拿到足够的
/// 原文来判断第 14-15 题」。TASK §2 的「上下文不足不得算作已核对」正是拦这个。
fn effective_adjudicated_count(canonical: &Value, candidate: &Value, rulings: &[Value]) -> usize {
    candidate_differences(canonical, candidate)
        .iter()
        .filter(|difference| {
            fresh_ruling_for_difference(rulings, difference).is_some_and(|ruling| {
                ruling.get("reason").and_then(Value::as_str)
                    != Some(crate::schema::cloud_repair_v1::CLOUD_RULING_REASON_CONTEXT_INSUFFICIENT)
            })
        })
        .map(difference_key)
        .collect::<BTreeSet<_>>()
        .len()
}

/// 从库里读当前 canonical + 候选，算有效裁定条数。读不到就返回 0（不谎报）。
fn adjudicated_count_now(
    root: &Path,
    item_id: &str,
    job_id: &str,
    batch_id: &str,
    rulings: &[Value],
) -> usize {
    let Ok(conn) = open_library_connection(root) else {
        return 0;
    };
    let Ok(Some((canonical, _))) = get_canonical_ds(&conn, item_id) else {
        return 0;
    };
    drop(conn);
    let Ok(Some(candidate)) = store::read_cloud_authoring_candidate(root, job_id, batch_id) else {
        return 0;
    };
    let Ok(candidate_value) = serde_json::to_value(&candidate.authoring) else {
        return 0;
    };
    effective_adjudicated_count(&canonical, &candidate_value, rulings)
}

/// 修复 run 的标识。**唯一**的产生处：循环内部与调度器兜底必须给出同一个值，
/// 否则撤销入口会拿着一个后端认不出的 id 去调用。
pub(crate) fn repair_run_id_for(batch_id: &str) -> String {
    format!("cloud-repair:{batch_id}")
}

/// 修复循环**没能**给出报告时的终态摘要（供调度器在拿到 Err 时兜底落盘）。
///
/// 为什么调度器也要能做这件事：循环内部的失败一律转成了报告（见 `run_repair_loop`），
/// 但有两类失败发生在循环**之外**，只会得到一个 Err：
///   · `run_blocking` 的 join 失败（任务 panic / 阻塞线程被取消）；
///   · `announced.is_none()`（开工前 lease 就丢了）。
/// 那时批次行里留着的还是循环写过的 `running`——不兜底就会永久停在那里。
pub(crate) fn unavailable_summary(
    root: &Path,
    item_id: &str,
    batch_id: &str,
    error: &str,
) -> Value {
    let edit_version = open_library_connection(root)
        .ok()
        .and_then(|conn| get_canonical_ds(&conn, item_id).ok().flatten())
        .map(|(_, version)| version)
        .unwrap_or(0);
    json!({
        "status": REPAIR_STATUS_UNAVAILABLE,
        "editVersion": edit_version,
        "appliedCount": 0,
        "rounds": 0,
        "remainingTasks": [],
        "adjudicatedCount": 0,
        "finishNote": Value::Null,
        "lastError": error,
        "undoAvailable": false,
        "repairRunId": repair_run_id_for(batch_id),
        "packets": [],
    })
}

/// 循环内部失败时的终态报告。
///
/// 为什么不让 `run_repair_loop` 直接返回 Err：循环**一开工就把批次行写成了 `running`**，
/// 而它有若干条可以提前返回的路径。那些路径一旦触发，调用方拿不到报告，也就没有任何
/// 人把 `running` 改掉——批次行会永久停在 running，而同一时刻 job 行已经是 failed。
/// 两个界面互相矛盾，用户永远看到「云端正在自动修复」。
///
/// 所以：循环内部的可恢复失败**一律**转成这份 `unavailable` 报告返回。Err 只留给
/// 「连报告都构造不出来」的情况（例如初始上下文建不出来且读不到版本），由调用方兜底。
fn failure_report(request: &RepairRunRequest<'_>, error: String) -> RepairRunReport {
    RepairRunReport {
        status: REPAIR_STATUS_UNAVAILABLE,
        rounds: 0,
        edit_version: current_canonical(request)
            .ok()
            .flatten()
            .map(|(_, version)| version)
            .unwrap_or(0),
        applied_count: 0,
        observations: Vec::new(),
        remaining_tasks: Vec::new(),
        adjudicated_count: 0,
        finish_note: None,
        last_error: Some(error),
        repair_run_id: request.repair_run_id.to_string(),
        packets: Vec::new(),
        unverified_evidence: 0,
    }
}

/// 机械比对：当前 canonical 与云端完整候选之间**还剩哪些实质差异**。///
/// 这是给修复模型看的上下文，也是「剩余问题」重算的输入。只做确定性比较，不调用模型。
pub(crate) fn candidate_differences(canonical: &Value, candidate: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let current_groups = groups_by_id(canonical);
    let candidate_groups = groups_by_id(candidate);

    for (task_id, candidate_group) in &candidate_groups {
        let Some(current_group) = current_groups.get(task_id) else {
            out.push(json!({
                "targetType": "task_group",
                "targetId": task_id,
                "field": "task_group",
                "canonical": Value::Null,
                "candidate": group_index_entry(candidate_group),
            }));
            continue;
        };
        for (field, pointer) in [
            ("instructions", "/instructions"),
            ("stimulus", "/stimulus"),
        ] {
            let current_text = nodes_text(current_group.get(pointer.trim_start_matches('/')).unwrap_or(&Value::Null));
            let candidate_text = nodes_text(candidate_group.get(pointer.trim_start_matches('/')).unwrap_or(&Value::Null));
            if normalize_text(&current_text) != normalize_text(&candidate_text) {
                push_difference(
                    &mut out,
                    "task_group",
                    task_id,
                    field,
                    json!(current_text),
                    json!(candidate_text),
                );
            }
        }
        // 响应组提示（按 responseGroupId 对齐）。
        let candidate_responses: BTreeMap<String, &Value> = candidate_group
            .get("responseGroups")
            .and_then(Value::as_array)
            .map(|groups| {
                groups
                    .iter()
                    .filter_map(|response| {
                        let id = response.get("responseGroupId").and_then(Value::as_str)?;
                        Some((id.to_string(), response))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let current_responses: BTreeMap<String, &Value> = current_group
            .get("responseGroups")
            .and_then(Value::as_array)
            .map(|groups| {
                groups
                    .iter()
                    .filter_map(|response| {
                        let id = response.get("responseGroupId").and_then(Value::as_str)?;
                        Some((id.to_string(), response))
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (response_id, candidate_response) in &candidate_responses {
            let current_prompt = current_responses
                .get(response_id)
                .map(|response| nodes_text(response.get("prompt").unwrap_or(&Value::Null)))
                .unwrap_or_default();
            let candidate_prompt = nodes_text(candidate_response.get("prompt").unwrap_or(&Value::Null));
            if normalize_text(&current_prompt) != normalize_text(&candidate_prompt) {
                push_difference(
                    &mut out,
                    "response_group",
                    response_id,
                    "prompt",
                    json!(current_prompt),
                    json!(candidate_prompt),
                );
            }
        }
        // 选项库。
        let current_options = option_bank_digest(current_group);
        let candidate_options = option_bank_digest(candidate_group);
        if serde_json::to_string(&current_options).ok() != serde_json::to_string(&candidate_options).ok() {
            push_difference(
                &mut out,
                "task_group",
                task_id,
                "option_bank",
                json!(current_options),
                json!(candidate_options),
            );
        }
        // 答案。
        let current_answers = group_answers(canonical, current_group);
        let candidate_answers = group_answers(candidate, candidate_group);
        for (slot_id, candidate_answer) in &candidate_answers {
            let current_answer = current_answers.get(slot_id).cloned().unwrap_or(Value::Null);
            if current_answer != *candidate_answer {
                push_difference(
                    &mut out,
                    "slot",
                    slot_id,
                    "answer",
                    current_answer,
                    candidate_answer.clone(),
                );
            }
        }
    }

    for (task_id, current_group) in &current_groups {
        if !candidate_groups.contains_key(task_id) {
            out.push(json!({
                "targetType": "task_group",
                "targetId": task_id,
                "field": "task_group",
                "canonical": group_index_entry(current_group),
                "candidate": Value::Null,
            }));
        }
    }

    // ── 听力 Part 边界 ────────────────────────────────────────────────
    // Part 的身份由后端分配、按**题号集合**复用（见 `apply_cloud_listening_parts`），
    // 所以「同一个 partId」就等于「同一段音频范围」。模型把题号重新切段时会产出新的
    // id：旧的消失、新的出现。这正是要报出来的边界变化——不报，修复模型就看不见
    // 自己动了分界，用户也看不到「云端把 Section 3 拆成了两段」。
    let current_parts = listening_parts_by_id(canonical);
    let candidate_parts = listening_parts_by_id(candidate);
    for (part_id, candidate_part) in &candidate_parts {
        match current_parts.get(part_id) {
            Some(current_part) => {
                for (field, pointer) in [("part_label", "/displayLabel"), ("part_tasks", "/taskIds")]
                {
                    let current_value = current_part.pointer(pointer).cloned().unwrap_or(Value::Null);
                    let candidate_value =
                        candidate_part.pointer(pointer).cloned().unwrap_or(Value::Null);
                    if current_value != candidate_value {
                        push_difference(
                            &mut out,
                            "part",
                            part_id,
                            field,
                            current_value,
                            candidate_value,
                        );
                    }
                }
            }
            // 当前稿里没有这一段：候选新增了一个分段。
            None => push_difference(
                &mut out,
                "part",
                part_id,
                "part_boundary",
                Value::Null,
                part_index_entry(candidate_part),
            ),
        }
    }
    for (part_id, current_part) in &current_parts {
        if !candidate_parts.contains_key(part_id) {
            // 候选里没有这一段：模型把它并进了别的分段，或整个丢了。
            push_difference(
                &mut out,
                "part",
                part_id,
                "part_boundary",
                part_index_entry(current_part),
                Value::Null,
            );
        }
    }

    // 每条差异都带上「裁定所依赖的内容」的指纹。裁定只在**依据**也没变时才算数，
    // 否则一条基于旧选项库/旧题面的结论会继续压住已经变质的内容（见
    // `target_context_fingerprint`）。放在这里统一算，是因为只有这一层同时看得到
    // 当前稿与差异的身份。
    for difference in out.iter_mut() {
        let (target_type, target_id, _) = difference_key(difference);
        let digest = target_context_fingerprint(canonical, &target_type, &target_id);
        if let Some(object) = difference.as_object_mut() {
            object.insert("contextDigest".to_string(), json!(digest));
        }
    }
    out
}

/// 构建修复上下文：**只读**，不调用模型、不写任何东西。
///
/// 首轮就把原文范围索引、当前稿、云端候选与差异、质量诊断、人工保护目标一次性给到，
/// 减少无意义的来回读取。
pub(crate) fn build_repair_context(
    root: &Path,
    item_id: &str,
    job_id: &str,
    batch_id: &str,
) -> CommandResult<Value> {
    let (canonical, edit_version, protected) = {
        let conn = open_library_connection(root)?;
        let (canonical, edit_version) = get_canonical_ds(&conn, item_id)?
            .ok_or_else(|| format!("ITEM_DS_NOT_SEEDED:{item_id}"))?;
        let protected = human_protected_targets(&conn, item_id, &canonical)?;
        (canonical, edit_version, protected)
    };

    let candidate = store::read_cloud_authoring_candidate(root, job_id, batch_id)?;
    let candidate_authoring = candidate
        .as_ref()
        .and_then(|candidate| serde_json::to_value(&candidate.authoring).ok())
        .unwrap_or(Value::Null);
    let differences = if candidate_authoring.is_null() {
        Vec::new()
    } else {
        candidate_differences(&canonical, &candidate_authoring)
    };

    let source_file_id = canonical
        .pointer("/exam/sourceFiles/0/sourceFileId")
        .and_then(Value::as_str)
        .unwrap_or(job_id)
        .to_string();

    Ok(json!({
        "itemId": item_id,
        "jobId": job_id,
        "batchId": batch_id,
        "sourceFileId": source_file_id,
        "editVersion": edit_version,
        // 整卷范围索引：模型必须看到整份文档，而不是只看到已发现的差异。
        "documentIndex": canonical
            .get("taskGroups")
            .and_then(Value::as_array)
            .map(|groups| groups.iter().map(group_index_entry).collect::<Vec<_>>())
            .unwrap_or_default(),
        "slotIds": canonical
            .get("answerSlots")
            .and_then(Value::as_object)
            .map(|slots| slots.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default(),
        "qualityIssues": quality_issues(&canonical),
        "protectedTargets": protected.into_iter().collect::<Vec<_>>(),
        "cloudCandidate": {
            "status": candidate.as_ref().map(|candidate| candidate.status.as_str()).unwrap_or("not_run"),
            "unresolvedReferences": candidate.as_ref().map(|candidate| candidate.unresolved_references.clone()).unwrap_or_default(),
            "unresolvedRegions": candidate.as_ref().map(|candidate| candidate.unresolved_regions.clone()).unwrap_or_default(),
            "sourceCoverageNotes": candidate.as_ref().map(|candidate| candidate.source_coverage_notes.clone()).unwrap_or_default(),
        },
        "differences": differences,
    }))
}

/// `read_draft`：按题组 / 题号读取当前稿片段（含稳定 ID 与当前答案）。
pub(crate) fn read_draft_section(
    canonical: &Value,
    edit_version: i64,
    arguments: &Value,
) -> Value {
    let requested_groups: BTreeSet<String> = arguments
        .get("taskGroupIds")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let requested_numbers: BTreeSet<u32> = arguments
        .get("questionNumbers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .map(|number| number as u32)
                .collect()
        })
        .unwrap_or_default();

    let groups: Vec<Value> = canonical
        .get("taskGroups")
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter(|group| {
                    if requested_groups.is_empty() && requested_numbers.is_empty() {
                        return true;
                    }
                    let task_id = group.get("taskId").and_then(Value::as_str).unwrap_or("");
                    if requested_groups.contains(task_id) {
                        return true;
                    }
                    group
                        .get("displayRange")
                        .map(crate::reconcile::candidate::expand_question_numbers)
                        .unwrap_or_default()
                        .iter()
                        .any(|number| requested_numbers.contains(number))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    // 只回本次选中题组覆盖的答案槽与答案，避免把整卷内容反复灌给模型。
    let selected_slots: BTreeSet<String> = groups
        .iter()
        .filter_map(|group| group.get("responseGroups").and_then(Value::as_array))
        .flatten()
        .filter_map(|response| response.get("slotIds").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let filter_map = |pointer: &str| -> Value {
        let Some(entries) = canonical.pointer(pointer).and_then(Value::as_object) else {
            return json!({});
        };
        let mut out = serde_json::Map::new();
        for (key, value) in entries {
            if selected_slots.contains(key) {
                out.insert(key.clone(), value.clone());
            }
        }
        Value::Object(out)
    };

    json!({
        "editVersion": edit_version,
        "taskGroups": groups,
        "answerSlots": filter_map("/answerSlots"),
        "answerKey": filter_map("/answerKey"),
    })
}

/// 原文件**逐页纯文本**（解析器层，不是语义识别结论）。
///
/// 为什么要单独抽出来：`read_source` 的 note 一直写着「下面的页文本就是抽取出来的文本层」，
/// 但 PDF 分支实际只回页图——而模型本来就已经拿到了整份 PDF，这次调用没带来任何新信息。
/// 更要紧的是：模型无法**引用**原文的句子，只能凭图片印象转述，于是「云端是照着原文件
/// 改的」这句话在证据上不可证伪（引文可以随口编）。
///
/// 只读**解析器层**产物，按优先级：
///   1. `document-ir.json` 的逐页 lines/spans（规范抽取）；
///   2. `document-ir-v2.shadow.compare.json` 的 `v1Text`（V1 仍是权威抽取文本）。
///
/// **绝不**读 `authoring-ir.json`：那是语义识别结论，正是要被纠正的东西，
/// 拿它当「原文证据」就是自我印证（本地抽错了、云端照着错的一起错）。
///
/// # 页号约定（实测，别改错）
///
/// 返回的 key 一律是 **1-based**，因为要 join 的是 `read_source` 自己的页对象，
/// 而它来自 `cache/vision/pdf-images.json`——那份产物的页码是 1-based
/// （`page-004-rendered.png` 对应 `pageIndex: 4`）。
/// 但 `DocumentIR` / `DocumentIRV2` 比对报告的 `pageIndex` 是 **0-based**：
/// 实测同一页在 pdf-images 里是 `pageIndex: 4`、在比对报告里是 `pageIndex: 3`，
/// 而权威稿锚点给出的也是 0-based 的 `pageIndex: 3`（节点 id 前缀 `p004-`）。
/// 两者相差 1，这里统一到 1-based，否则模型会引到隔壁页的句子——
/// 那比不给文本更糟：它看起来有出处，出处却是错的。
fn source_page_texts(root: &Path, job_id: &str) -> BTreeMap<u64, String> {
    let dir = crate::util::job_dir(root, job_id);
    let mut out: BTreeMap<u64, String> = BTreeMap::new();

    if let Ok(Some(ir)) = crate::util::read_json_opt(&dir.join("document-ir.json")) {
        for page in ir.get("pages").and_then(Value::as_array).into_iter().flatten() {
            let Some(index) = page.get("pageIndex").and_then(Value::as_u64) else {
                continue;
            };
            let mut text = String::new();
            if let Some(lines) = page.get("lines").and_then(Value::as_array) {
                for line in lines {
                    if let Some(value) = line.get("text").and_then(Value::as_str) {
                        if !value.trim().is_empty() {
                            text.push_str(value);
                            text.push('\n');
                        }
                    }
                }
            } else if let Some(spans) = page.get("spans").and_then(Value::as_array) {
                for span in spans {
                    if let Some(value) = span.get("text").and_then(Value::as_str) {
                        text.push_str(value);
                    }
                }
                text.push('\n');
            }
            if !text.trim().is_empty() {
                out.insert(index + 1, text.trim_end().to_string());
            }
        }
    }
    if !out.is_empty() {
        return out;
    }

    if let Ok(Some(report)) =
        crate::util::read_json_opt(&dir.join("document-ir-v2.shadow.compare.json"))
    {
        for page in report
            .get("pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(index) = page.get("pageIndex").and_then(Value::as_u64) else {
                continue;
            };
            let text = page
                .get("v1Text")
                .and_then(Value::as_str)
                .or_else(|| page.get("v2Text").and_then(Value::as_str))
                .unwrap_or("");
            if !text.trim().is_empty() {
                out.insert(index + 1, text.trim_end().to_string());
            }
        }
    }
    out
}

/// 把逐页原文文本贴到 `read_source` 要返回的页对象上。
///
/// **保持页对象原有字段不变**（页图、尺寸、渲染信息都还要用），只增补 `text`。
/// 抽成纯函数是为了能直接测「哪一页拿到哪段文本」——那正是最容易写错、也最难在
/// 端到端里定位的一步（页号 0-based / 1-based 混淆时，模型会引到隔壁页的句子）。
fn attach_page_texts(pages: &[Value], page_texts: &BTreeMap<u64, String>) -> Vec<Value> {
    pages
        .iter()
        .map(|page| {
            let mut page = page.clone();
            let index = page
                .get("pageIndex")
                .and_then(Value::as_u64)
                .or_else(|| page.get("page").and_then(Value::as_u64));
            if let Some(text) = index.and_then(|index| page_texts.get(&index)) {
                page["text"] = json!(text);
            }
            page
        })
        .collect()
}

/// `read_source` 的 PDF 分支：把原文件页与抽取出来的文本层拼成返回体。
///
/// 单独成函数是为了**能被真实驱动**。以前这段逻辑内联在 `read_source_evidence` 里，
/// 而后者要先跑 PDF 视觉抽取（Python sidecar），单测跑不起来，于是测试只能直接调
/// `attach_page_texts`——助手函数是被测到了，「PDF 分支到底有没有真的接上文本层」
/// 却没人管：把 `attach_page_texts` 的调用删掉（退回「只回页图」），测试照样全绿。
/// 变异检查就是这样把它抓出来的。
pub(crate) fn pdf_read_source_response(
    evidence: &Value,
    page_from: Option<u64>,
    page_to: Option<u64>,
    page_texts: &BTreeMap<u64, String>,
) -> Value {
    let selected: Vec<Value> = evidence
        .get("pages")
        .and_then(Value::as_array)
        .map(|pages| {
            pages
                .iter()
                .filter(|page| {
                    let Some(from) = page_from else { return true };
                    let index = page
                        .get("pageIndex")
                        .and_then(Value::as_u64)
                        .or_else(|| page.get("page").and_then(Value::as_u64))
                        .unwrap_or(0);
                    index >= from && index <= page_to.unwrap_or(from)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let pages = attach_page_texts(&selected, page_texts);
    let quoted_pages = pages.iter().filter(|page| page.get("text").is_some()).count();
    json!({
        "kind": "pdf",
        "sourceFileId": evidence.get("sourceFileId").cloned().unwrap_or(Value::Null),
        // 如实说明这次到底给了什么：抽不出文本层时不能继续声称「下面就是文本层」，
        // 否则模型会以为自己读到了原文，从而编造引文。
        "note": if quoted_pages > 0 {
            "The original PDF is attached to this conversation. Each returned page also carries the text layer extracted from the original file — cite it verbatim when you justify an edit."
        } else {
            "The original PDF is attached to this conversation. No text layer could be extracted for these pages; read the attached file itself."
        },
        "pagesWithText": quoted_pages,
        "pages": pages,
    })
}

/// `read_source`：读取原文件证据。
///
/// **不接受任意路径**：来源固定由后端按 job 解析，模型只能选页范围或给一段引文。
pub(crate) fn read_source_evidence(
    root: &Path,
    job_id: &str,
    arguments: &Value,
) -> CommandResult<Value> {
    let evidence = crate::auto_pipeline::cloud_source_evidence(root, job_id)?;
    let page_from = arguments.get("pageIndex").and_then(Value::as_u64);
    let page_to = arguments
        .get("pageTo")
        .and_then(Value::as_u64)
        .or(page_from);
    let quote = arguments
        .get("quote")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let kind = evidence.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind == "pdf" {
        // 原文文本一并带上：模型才能**逐字引用**它引以为据的那句话，
        // 而不是只能凭页图转述（见 `source_page_texts` 的说明）。
        let page_texts = source_page_texts(root, job_id);
        return Ok(pdf_read_source_response(
            &evidence,
            page_from,
            page_to,
            &page_texts,
        ));
    }

    let text = evidence.get("text").and_then(Value::as_str).unwrap_or("");
    if text.is_empty() {
        return Ok(json!({
            "kind": "text",
            "sourceFileId": evidence.get("sourceFileId").cloned().unwrap_or(Value::Null),
            "note": "No source text could be extracted for this job.",
            "text": "",
        }));
    }
    // 有引文就返回引文周围的窗口；否则返回开头一段（并如实说明被截断）。
    let (slice, truncated) = match quote.and_then(|needle| text.find(needle)) {
        Some(position) => {
            let start = text[..position]
                .char_indices()
                .rev()
                .nth(800)
                .map(|(index, _)| index)
                .unwrap_or(0);
            let end = text[position..]
                .char_indices()
                .nth(1200)
                .map(|(index, _)| position + index)
                .unwrap_or(text.len());
            (text[start..end].to_string(), start > 0 || end < text.len())
        }
        None => {
            let end = text
                .char_indices()
                .nth(4000)
                .map(|(index, _)| index)
                .unwrap_or(text.len());
            (text[..end].to_string(), end < text.len())
        }
    };
    Ok(json!({
        "kind": "text",
        "sourceFileId": evidence.get("sourceFileId").cloned().unwrap_or(Value::Null),
        "quoteFound": quote.map(|needle| text.contains(needle)).unwrap_or(false),
        "truncated": truncated,
        "text": slice,
    }))
}

/// 解析模型返回的工具调用。
///
/// 解析失败必须**具体**：模型要能据此改对，而不是收到一句笼统的"格式错误"。
fn parse_tool_call(raw: &Value) -> Result<CloudRepairToolCallV1, String> {
    let object = raw
        .as_object()
        .ok_or_else(|| "CLOUD_REPAIR_STEP_NOT_OBJECT".to_string())?;
    let call_id = object
        .get("callId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "CLOUD_REPAIR_STEP_CALL_ID_MISSING".to_string())?;
    let tool = object
        .get("tool")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "CLOUD_REPAIR_STEP_TOOL_MISSING".to_string())?;
    let call = CloudRepairToolCallV1 {
        call_id: call_id.to_string(),
        tool: tool.to_string(),
        arguments: object.get("arguments").cloned().unwrap_or(Value::Null),
    };
    if !call.is_known_tool() {
        return Err(format!("CLOUD_REPAIR_UNKNOWN_TOOL:{tool}"));
    }
    Ok(call)
}

/// 包模式下执行一次工具调用需要的东西。
///
/// 抓取类工具全部只读、不接受路径、只作用于本 job，并**共用同一个包内预算**
/// （[`grab::GrabBudget`]）：`read_source` 一次最多 3 页、每包累计最多 6 页、最多 3 次
/// 抓取。这正是「包」这件事能成立的前提——否则模型只要反复 `read_source` 就能把整卷
/// 重新读回来，预切就成了形式主义。
struct PacketTools<'a> {
    source: &'a packets::SourcePageIndex,
    budget: &'a mut grab::GrabBudget,
    /// 本包允许 `read_draft` / `read_candidate` 读到的题组。
    task_ids: BTreeSet<String>,
    /// 本包覆盖的题号。
    question_numbers: Vec<u32>,
}

impl PacketTools<'_> {
    /// 请求的题组 / 题号是否落在本包范围内。空请求 = 没给选择器。
    fn scope_error(&self, task_ids: &[String], numbers: &[u32]) -> Option<String> {
        if task_ids.is_empty() && numbers.is_empty() {
            return Some(
                "CLOUD_DRAFT_SCOPE_REQUIRED: this is a repair packet, so pass the taskGroupIds or \
                 questionNumbers you saw in the packet; the whole paper is not available here"
                    .to_string(),
            );
        }
        let hits_task = task_ids.iter().any(|id| self.task_ids.contains(id));
        let hits_number = numbers.iter().any(|number| self.question_numbers.contains(number));
        if hits_task || hits_number {
            return None;
        }
        Some(format!(
            "CLOUD_DRAFT_OUTSIDE_PACKET: taskGroupIds={task_ids:?} questionNumbers={numbers:?} are \
             not in this packet (packet taskIds={:?} questionNumbers={:?}); ask for what is in \
             scope, or use read_candidate / report_insufficient_context",
            self.task_ids.iter().cloned().collect::<Vec<_>>(),
            self.question_numbers
        ))
    }
}

/// 执行一次允许的工具调用，返回**真实**结果。
/// 引文核验用的**完整原文**文本层（P9）。
///
/// - PDF：`document-ir` 的逐页行文本。包模式直接复用抓取工具手里的那份全量索引——
///   模型看到的行与核验用的行**同源**，不存在「核验用另一套文本」的缝；
/// - DOCX / TXT / MD：从原始文件独立抽取的全文（没有页的概念）；
/// - 什么都读不到（扫描件 / 解析产物缺失）⇒ `Unavailable`：不拒绝，标 unverifiable。
fn evidence_source_text(
    request: &RepairRunRequest<'_>,
    context: &Value,
    packet_tools: Option<&PacketTools<'_>>,
) -> tools::EvidenceSourceContext {
    // 包模式的 PDF：抓取工具的 `source` 就是整份原文索引（不是包切片），零额外 I/O。
    if let Some(tools) = packet_tools {
        if tools.source.kind == "pdf" {
            return source_context(request, tools.source.source_file_id.clone(), paged_source_text(tools.source));
        }
    }
    // 只需要**身份**（id / 类型）：曾经在这里调 `cloud_source_evidence`，PDF 会因此被
    // 整份重渲染一遍（每轮一次，白吃修复时限）。身份走只读入口；全文按类型分头取。
    let source_meta = crate::auto_pipeline::cloud_source_identity(request.root, request.job_id)
        .unwrap_or(Value::Null);
    let source_file_id = source_meta
        .get("sourceFileId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let text = match source_meta.get("kind").and_then(Value::as_str) {
        Some("text") => {
            // 非 PDF 的全文：只读抽取（原文件直读，不渲染、不写盘）。
            let text = crate::auto_pipeline::cloud_source_text_evidence(
                request.root,
                request.job_id,
            )
            .ok()
            .and_then(|meta| meta.get("text").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
            if text.trim().is_empty() {
                tools::EvidenceSourceText::Unavailable
            } else {
                tools::EvidenceSourceText::Whole(text)
            }
        }
        Some("pdf") => {
            let index = load_packet_source_index(request, context);
            paged_source_text(&index)
        }
        _ => tools::EvidenceSourceText::Unavailable,
    };
    source_context(request, source_file_id, text)
}

/// 主试卷 id + 作业内其它真实源文件 id → 引文核验上下文。
///
/// 其它源文件 id 来自作业的 `sourceFiles`（如单独上传的答案文件，role `AnswerKey`）：
/// 它们真实存在，但修复链的证据面从不包含其内容——引用它们的证据核验不了，
/// 标 unverifiable；不在这两个集合里的 id 是编造的来源，整批拒绝。
fn source_context(
    request: &RepairRunRequest<'_>,
    main_source_file_id: String,
    text: tools::EvidenceSourceText,
) -> tools::EvidenceSourceContext {
    let mut main = main_source_file_id;
    if main.is_empty() {
        // 原文索引缺 sourceFileId 时的既有兜底（与 load_packet_source_index 一致）。
        main = request.job_id.to_string();
    }
    // 作业源文件清单：读得到才做三分类（编造的来源要拒绝）；读不到（无 job /
    // 元数据缺失）就不对「id 存不存在」下编造的结论，全部如实标 unverifiable。
    let known_source_file_ids = crate::job_store::load_job(request.root, request.job_id)
        .ok()
        .map(|job| {
            job.source_files
                .iter()
                .map(|source| source.file_id.clone())
                .collect::<BTreeSet<String>>()
        });
    tools::EvidenceSourceContext {
        main_source_file_id: main,
        text,
        known_source_file_ids,
    }
}

fn paged_source_text(source: &packets::SourcePageIndex) -> tools::EvidenceSourceText {
    if source.lines.is_empty() {
        return tools::EvidenceSourceText::Unavailable;
    }
    // 「真实存在的页」= 有文本的页 ∪ 有页图的页。扫描页有图无文本，正是要在
    // 引文核验里区别对待的那种页。
    let mut existing_pages: BTreeSet<u32> = source.lines.keys().copied().collect();
    existing_pages.extend(source.page_images.keys().copied());
    tools::EvidenceSourceText::Paged {
        pages: source
            .lines
            .iter()
            .map(|(page, page_lines)| {
                (
                    *page,
                    page_lines
                        .iter()
                        .map(|line| line.text.clone())
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            })
            .collect(),
        existing_pages,
    }
}

/// 给裁定证据逐条盖上核验结果（`verified` / `unverifiable`）。
///
/// 标记跟着裁定记录一起落进 `repair-rulings` artifact——那是裁定的 journal：
/// 「原文没有文本层」的裁定必须能被事后看出**没有核验过**，而不是只留下一个
/// 看不出出处的结论。
fn annotate_evidence_verification(mut evidence: Vec<Value>, unverifiable: &[usize]) -> Vec<Value> {
    for (index, entry) in evidence.iter_mut().enumerate() {
        if let Some(object) = entry.as_object_mut() {
            let verification = if unverifiable.contains(&index) {
                "unverifiable"
            } else {
                "verified"
            };
            object.insert("verification".to_string(), json!(verification));
        }
    }
    evidence
}

/// 从工具结果里读「多少条证据没有核验」。apply_edits 回数组（条目下标），
/// record_ruling 回数字（条数），两种形状都收。
fn evidence_unverifiable_count(result: &Value) -> usize {
    match result.get("evidenceUnverifiable") {
        Some(Value::Array(items)) => items.len(),
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0) as usize,
        _ => 0,
    }
}

fn execute_tool(
    request: &RepairRunRequest<'_>,
    call: &CloudRepairToolCallV1,
    round: u32,
    context: &Value,
    packet: Option<&mut PacketTools<'_>>,
) -> (CloudRepairToolResultV1, Option<usize>) {
    let mut packet = packet;
    match call.tool.as_str() {
        "read_draft" => {
            // 包模式下**默认范围限定本包**：这是任务书 §4.2 对 `read_draft` 的要求，
            // 也是「一次调用不能把整卷拿回来」的又一道闸。模型只能读它正在核的那一块。
            if let Some(tools) = packet.as_deref() {
                let requested_groups: Vec<String> = call
                    .arguments
                    .get("taskGroupIds")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                let requested_numbers: Vec<u32> = call
                    .arguments
                    .get("questionNumbers")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(Value::as_u64).map(|n| n as u32).collect())
                    .unwrap_or_default();
                if let Some(error) = tools.scope_error(&requested_groups, &requested_numbers) {
                    return (CloudRepairToolResultV1::rejected(&call.call_id, vec![error]), None);
                }
            }
            let canonical = match current_canonical(request) {
                Ok(Some((document, version))) => (document, version),
                Ok(None) => {
                    return (
                        CloudRepairToolResultV1::rejected(
                            &call.call_id,
                            vec![format!("ITEM_DS_NOT_SEEDED:{}", request.item_id)],
                        ),
                        None,
                    )
                }
                Err(error) => {
                    return (CloudRepairToolResultV1::rejected(&call.call_id, vec![error]), None)
                }
            };
            (
                CloudRepairToolResultV1::ok(
                    &call.call_id,
                    read_draft_section(&canonical.0, canonical.1, &call.arguments),
                ),
                None,
            )
        }
        "read_source" => {
            // 包模式走**收紧版**：必须给页范围或引文，单次 ≤ 3 页。legacy 保留旧行为
            // （L3 的最后手段与回归对照），否则「一次拿回整卷」这条路就还在。
            let result = match packet.as_deref_mut() {
                Some(tools) => grab::read_source(tools.source, &call.arguments, tools.budget),
                None => read_source_evidence(request.root, request.job_id, &call.arguments),
            };
            match result {
                Ok(value) => (CloudRepairToolResultV1::ok(&call.call_id, value), None),
                Err(error) => (CloudRepairToolResultV1::rejected(&call.call_id, vec![error]), None),
            }
        }
        "search_source" => match packet.as_deref_mut() {
            Some(tools) => match grab::search_source(tools.source, &call.arguments, tools.budget) {
                Ok(value) => (CloudRepairToolResultV1::ok(&call.call_id, value), None),
                Err(error) => (CloudRepairToolResultV1::rejected(&call.call_id, vec![error]), None),
            },
            None => (
                CloudRepairToolResultV1::rejected(
                    &call.call_id,
                    vec!["CLOUD_GRAB_ONLY_IN_PACKET_MODE:search_source".to_string()],
                ),
                None,
            ),
        },
        "read_page_region" => match packet.as_deref_mut() {
            Some(tools) => match grab::read_page_region(
                request.root,
                request.job_id,
                tools.source,
                &call.arguments,
                tools.budget,
            ) {
                Ok(value) => (CloudRepairToolResultV1::ok(&call.call_id, value), None),
                Err(error) => (CloudRepairToolResultV1::rejected(&call.call_id, vec![error]), None),
            },
            None => (
                CloudRepairToolResultV1::rejected(
                    &call.call_id,
                    vec!["CLOUD_GRAB_ONLY_IN_PACKET_MODE:read_page_region".to_string()],
                ),
                None,
            ),
        },
        "read_passage" => match packet.as_deref_mut() {
            Some(tools) => match grab::read_passage(tools.source, &call.arguments, tools.budget) {
                Ok(value) => (CloudRepairToolResultV1::ok(&call.call_id, value), None),
                Err(error) => (CloudRepairToolResultV1::rejected(&call.call_id, vec![error]), None),
            },
            None => (
                CloudRepairToolResultV1::rejected(
                    &call.call_id,
                    vec!["CLOUD_GRAB_ONLY_IN_PACKET_MODE:read_passage".to_string()],
                ),
                None,
            ),
        },
        "read_candidate" => {
            let Some(tools) = packet.as_deref_mut() else {
                return (
                    CloudRepairToolResultV1::rejected(
                        &call.call_id,
                        vec!["CLOUD_GRAB_ONLY_IN_PACKET_MODE:read_candidate".to_string()],
                    ),
                    None,
                );
            };
            let requested_groups: Vec<String> = call
                .arguments
                .get("taskIds")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            let requested_numbers: Vec<u32> = call
                .arguments
                .get("questionNumbers")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_u64).map(|n| n as u32).collect())
                .unwrap_or_default();
            // 候选切片的 id 是**云端** id，与本地 taskId 不同名，所以允许两套：包内本地
            // taskId，以及候选切片里出现过的 taskId。
            let mut allowed = tools.task_ids.clone();
            for group in context
                .pointer("/candidateSlice/taskGroups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(task_id) = group.get("taskId").and_then(Value::as_str) {
                    allowed.insert(task_id.to_string());
                }
            }
            let scoped = PacketTools {
                source: tools.source,
                budget: tools.budget,
                task_ids: allowed,
                question_numbers: tools.question_numbers.clone(),
            };
            if let Some(error) = scoped.scope_error(&requested_groups, &requested_numbers) {
                return (CloudRepairToolResultV1::rejected(&call.call_id, vec![error]), None);
            }
            let candidate = store::read_cloud_authoring_candidate(
                request.root,
                request.job_id,
                request.batch_id,
            )
            .ok()
            .flatten()
            .and_then(|candidate| serde_json::to_value(&candidate.authoring).ok())
            .unwrap_or(Value::Null);
            if candidate.is_null() {
                return (
                    CloudRepairToolResultV1::rejected(
                        &call.call_id,
                        vec![format!("CLOUD_CANDIDATE_UNAVAILABLE:{}", request.batch_id)],
                    ),
                    None,
                );
            }
            (
                CloudRepairToolResultV1::ok(
                    &call.call_id,
                    read_draft_section(&candidate, 0, &call.arguments),
                ),
                None,
            )
        }
        "report_insufficient_context" => {
            let Some(tools) = packet.as_deref_mut() else {
                return (
                    CloudRepairToolResultV1::rejected(
                        &call.call_id,
                        vec![
                            "CLOUD_REPAIR_INSUFFICIENT_CONTEXT_OUTSIDE_PACKET: there is no packet \
                             to report against in this mode"
                                .to_string(),
                        ],
                    ),
                    None,
                );
            };
            let reported_packet_id = call
                .arguments
                .get("packetId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let active_packet_id = context
                .get("packetId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if reported_packet_id != active_packet_id {
                return (
                    CloudRepairToolResultV1::rejected(
                        &call.call_id,
                        vec![format!(
                            "CLOUD_REPAIR_INSUFFICIENT_CONTEXT_PACKET_MISMATCH: expected active packet {active_packet_id:?}, got {reported_packet_id:?}"
                        )],
                    ),
                    None,
                );
            }
            if call
                .arguments
                .get("reason")
                .and_then(Value::as_str)
                .is_none_or(|reason| reason.trim().is_empty())
            {
                return (
                    CloudRepairToolResultV1::rejected(
                        &call.call_id,
                        vec![
                            "CLOUD_REPAIR_INSUFFICIENT_CONTEXT_REASON_REQUIRED: explain why the active packet is insufficient"
                                .to_string(),
                        ],
                    ),
                    None,
                );
            }
            let needs = call
                .arguments
                .get("needs")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if needs.is_empty() {
                return (
                    CloudRepairToolResultV1::rejected(
                        &call.call_id,
                        vec![
                            "CLOUD_REPAIR_INSUFFICIENT_CONTEXT_NO_NEEDS: say exactly what you are \
                             missing (pages / quote / paragraph labels / candidate slice)"
                                .to_string(),
                        ],
                    ),
                    None,
                );
            }
            let mut errors = Vec::new();
            for need in &needs {
                match serde_json::from_value::<
                    crate::schema::cloud_repair_v1::CloudRepairContextNeedV1,
                >(need.clone())
                {
                    Ok(parsed) => {
                        if let Err(error) = parsed.validate() {
                            errors.push(error);
                        }
                    }
                    Err(error) => errors.push(format!("CLOUD_NEED_MALFORMED:{error}")),
                }
            }
            if !errors.is_empty() {
                return (CloudRepairToolResultV1::rejected(&call.call_id, errors), None);
            }
            let (satisfied, unsatisfied, deferred) = grab::satisfy_needs(
                request.root,
                request.job_id,
                tools.source,
                &needs,
                tools.budget,
            );
            // `candidate` / `draft` 需求只有这一层能满足（它同时看得到权威稿与候选）。
            let mut fetched = satisfied;
            let mut unsatisfied = unsatisfied;
            for need in deferred {
                let kind = need.get("kind").and_then(Value::as_str).unwrap_or("");
                let numbers: Vec<u32> = need
                    .get("questionNumbers")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(Value::as_u64).map(|n| n as u32).collect())
                    .unwrap_or_default();
                let task_ids: Vec<String> = need
                    .get("taskIds")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                if let Some(error) = tools.scope_error(&task_ids, &numbers) {
                    unsatisfied.push(error);
                    continue;
                }
                let arguments = json!({"taskGroupIds": task_ids, "questionNumbers": numbers});
                let slice = if kind == "candidate" {
                    store::read_cloud_authoring_candidate(
                        request.root,
                        request.job_id,
                        request.batch_id,
                    )
                    .ok()
                    .flatten()
                    .and_then(|candidate| serde_json::to_value(&candidate.authoring).ok())
                    .map(|candidate| read_draft_section(&candidate, 0, &arguments))
                } else {
                    current_canonical(request)
                        .ok()
                        .flatten()
                        .map(|(canonical, version)| read_draft_section(&canonical, version, &arguments))
                };
                match slice {
                    Some(slice) => fetched.push(json!({"kind": kind, "result": slice})),
                    None => unsatisfied.push(format!(
                        "CLOUD_NEED_UNSATISFIED:{kind}: the {kind} slice could not be read"
                    )),
                }
            }
            (
                CloudRepairToolResultV1::ok(
                    &call.call_id,
                    json!({
                        "status": "needs_answered",
                        "reason": call.arguments.get("reason").cloned().unwrap_or(Value::Null),
                        "satisfied": fetched,
                        "unsatisfied": unsatisfied,
                        "budget": {
                            "calls": tools.budget.calls,
                            "pages": tools.budget.pages,
                            "bytes": tools.budget.bytes,
                        },
                        "noteForModel": "The fetched evidence is merged into this packet and will \
                                         be in your next request. Anything under \"unsatisfied\" was \
                                         NOT fetched — do not assume it.",
                    }),
                ),
                None,
            )
        }
        "apply_edits" => {
            let Some(commands) = call.arguments.get("commands").and_then(Value::as_array) else {
                return (
                    CloudRepairToolResultV1::rejected(
                        &call.call_id,
                        vec!["CLOUD_EDIT_NO_COMMANDS".to_string()],
                    ),
                    None,
                );
            };
            // baseVersion 必须由模型给出：后端**不**替它补一个"当前版本"，否则并发
            // 冲突检测就退化成不存在（模型会拿一份旧快照去覆盖用户的新编辑）。
            let Some(base_version) = call.arguments.get("baseVersion").and_then(Value::as_i64)
            else {
                return (
                    CloudRepairToolResultV1::rejected(
                        &call.call_id,
                        vec![
                            "CLOUD_EDIT_BASE_VERSION_MISSING: call read_draft first and pass the editVersion you based your edits on"
                                .to_string(),
                        ],
                    ),
                    None,
                );
            };
            let evidence = call
                .arguments
                .get("evidence")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let edit_request = tools::CloudEditRequest {
                item_id: request.item_id.to_string(),
                repair_run_id: request.repair_run_id.to_string(),
                base_version,
                round: i64::from(round),
                tool_call_id: call.call_id.clone(),
                commands: commands.clone(),
                evidence,
            };
            // 引文对照**完整原文**文本层（P9）：编造的引文整批拒绝；没有文本层时
            // 标 unverifiable，不拒绝也不算已核验。
            let source_text = evidence_source_text(request, context, packet.as_deref());
            match tools::apply_cloud_edits(request.root, &edit_request, &source_text) {
                Ok(outcome) => {
                    let applied = matches!(outcome.status, tools::CloudEditStatus::Applied);
                    let applied_count = outcome.applied_count;
                    let evidence_unverifiable = outcome.evidence_unverifiable.clone();
                    let result = json!({
                        "status": match outcome.status {
                            tools::CloudEditStatus::Applied => "applied",
                            tools::CloudEditStatus::Rejected => "rejected",
                        },
                        "editVersion": outcome.edit_version,
                        "appliedCount": outcome.applied_count,
                        "appliedTargets": outcome.applied_targets,
                        "strippedKeys": outcome.stripped_keys,
                        "introducedHardFailures": outcome.introduced_hard_failures,
                        // 原文没有文本层时这里非空：这些证据**没有**被核验过，
                        // 摘要据此如实呈现，不能被「已应用」盖成「已核实」。
                        "evidenceUnverifiable": evidence_unverifiable,
                        "errors": outcome.errors,
                    });
                    let tool_result = if applied {
                        CloudRepairToolResultV1::ok(&call.call_id, result)
                    } else {
                        CloudRepairToolResultV1 {
                            schema_version:
                                crate::schema::cloud_repair_v1::CLOUD_REPAIR_TOOL_RESULT_V1_SCHEMA_VERSION
                                    .to_string(),
                            call_id: call.call_id.clone(),
                            status: crate::schema::cloud_repair_v1::CloudRepairToolStatusV1::Rejected,
                            result,
                            errors: outcome.errors.clone(),
                        }
                    };
                    (
                        tool_result,
                        if applied { Some(applied_count) } else { None },
                    )
                }
                Err(error) => (
                    CloudRepairToolResultV1::rejected(&call.call_id, vec![error]),
                    None,
                ),
            }
        }
        "record_ruling" => {
            // 裁定必须指向**上下文里确实存在**的一条差异。否则模型可以凭空造一条裁定，
            // 把一件它没看过的事情标成「已了结」——那正是「模型不能制造已完成」这条
            // 纪律要挡住的形状。
            let differences = context
                .get("differences")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let Some(entries) = call.arguments.get("rulings").and_then(Value::as_array) else {
                return (
                    CloudRepairToolResultV1::rejected(
                        &call.call_id,
                        vec!["CLOUD_RULING_NO_RULINGS".to_string()],
                    ),
                    None,
                );
            };
            let mut recorded = Vec::new();
            let mut errors = Vec::new();
            let mut evidence_unverifiable_total = 0usize;
            // P9：裁定证据与 apply_edits 走**同一套**引文核验，对照同一份完整原文。
            let source_text = evidence_source_text(request, context, packet.as_deref());
            for (entry_index, entry) in entries.iter().enumerate() {
                let target_type = entry
                    .get("targetType")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let target_id = entry
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let field = entry
                    .get("field")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let ruling = entry
                    .get("ruling")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if !matches!(
                    ruling.as_str(),
                    crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT
                        | crate::schema::cloud_repair_v1::CLOUD_RULING_CANNOT_RESOLVE
                ) {
                    errors.push(format!(
                        "CLOUD_RULING_UNKNOWN_KIND:{ruling}: allowed are \
                         current_is_correct (the current draft is right and the candidate is wrong) \
                         and cannot_resolve (the original file does not settle it)"
                    ));
                    continue;
                }
                let Some(difference) = differences.iter().find(|difference| {
                    difference_key(difference) == (target_type.clone(), target_id.clone(), field.clone())
                }) else {
                    errors.push(format!(
                        "CLOUD_RULING_NO_SUCH_DIFFERENCE:{target_type}:{target_id}:{field}: \
                         rule only on differences listed in the context"
                    ));
                    continue;
                };
                let (canonical_digest, candidate_digest, context_digest) = difference_digests(difference);
                // P9：裁定证据与 apply_edits 走**同一套**校验——先结构，再引文对照完整原文。
                // 编造引文的裁定不得记录：那等于允许模型给它没看过的结论盖章。
                // 原文没有文本层时照常记录，但每条证据标 unverifiable，不算已核验。
                let evidence_entries = entry
                    .get("evidence")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let structural: Vec<String> = tools::validate_evidence(&evidence_entries)
                    .into_iter()
                    .map(|problem| format!("CLOUD_RULING_EVIDENCE_INVALID:{entry_index}:{problem}"))
                    .collect();
                if !structural.is_empty() {
                    errors.extend(structural);
                    continue;
                }
                let (quote_problems, unverifiable) =
                    tools::verify_evidence_quotes(&evidence_entries, &source_text);
                if !quote_problems.is_empty() {
                    // quote_problems 的下标是**本条裁定证据数组内**的下标；
                    // 前面拼上裁定下标，模型才能定位是第几条裁定的第几条证据。
                    errors.extend(quote_problems.into_iter().map(|problem| {
                        match problem.rsplit_once(':') {
                            Some((code, index)) => format!("{code}:{entry_index}:{index}"),
                            None => format!("{problem}:{entry_index}"),
                        }
                    }));
                    continue;
                }
                evidence_unverifiable_total += unverifiable.len();
                let annotated_evidence =
                    annotate_evidence_verification(evidence_entries, &unverifiable);
                recorded.push(json!({
                    "targetType": target_type,
                    "targetId": target_id,
                    "field": field.clone(),
                    "ruling": ruling,
                    "reason": entry.get("reason").cloned().unwrap_or(Value::Null),
                    "evidence": annotated_evidence,
                    // 绑定裁定当时看到的这一对内容；任一侧后来变了，这条裁定作废重评。
                    "canonicalDigest": canonical_digest,
                    "candidateDigest": candidate_digest,
                    // 再绑定**裁定所依赖的内容**（承载它的题组 / 作答区）。只绑差异两侧
                    // 是不够的：答案值没变但选项库被改，裁定的依据就已经没了。
                    "contextDigest": context_digest,
                    "recordedAtRound": round,
                }));
            }
            if recorded.is_empty() {
                return (CloudRepairToolResultV1::rejected(&call.call_id, errors), None);
            }
            (
                CloudRepairToolResultV1::ok(
                    &call.call_id,
                    json!({
                        "status": "recorded",
                        "recorded": recorded,
                        "errors": errors,
                        // 没有文本层时非 0：这些裁定**记了**，但它们的证据没有被核验过。
                        "evidenceUnverifiable": evidence_unverifiable_total,
                        "noteForModel": "Recorded rulings remove adjudicated differences from the user's list. \
                                         They cannot remove structural problems found by the backend validator.",
                    }),
                ),
                None,
            )
        }
        "finish_packet" => (
            CloudRepairToolResultV1::ok(
                &call.call_id,
                json!({
                    "status": "packet_finished",
                    "packetId": call.arguments.get("packetId").cloned().unwrap_or(Value::Null),
                    "note": call.arguments.get("note").cloned().unwrap_or(Value::Null),
                    "remaining": call.arguments.get("unresolved").cloned().unwrap_or_else(|| json!([])),
                    "noteForModel": "This packet is closed. The backend still recomputes what is left \
                                     from the current canonical, and it may open a new packet for a \
                                     difference you changed.",
                }),
            ),
            None,
        ),
        "finish" => (
            CloudRepairToolResultV1::ok(
                &call.call_id,
                json!({
                    "status": "finished",
                    "note": call.arguments.get("note").cloned().unwrap_or(Value::Null),
                    "remaining": call.arguments.get("unresolved").cloned().unwrap_or_else(|| json!([])),
                    "noteForModel": "The backend recomputes the remaining work from the current canonical; your list is context, not the verdict.",
                }),
            ),
            None,
        ),
        other => (
            CloudRepairToolResultV1::rejected(
                &call.call_id,
                vec![format!("CLOUD_REPAIR_UNKNOWN_TOOL:{other}")],
            ),
            None,
        ),
    }
}

fn current_canonical(root_and_item: &RepairRunRequest<'_>) -> CommandResult<Option<(Value, i64)>> {
    let conn = open_library_connection(root_and_item.root)?;
    get_canonical_ds(&conn, root_and_item.item_id)
}

/// 一条剩余任务的**用户动作身份**：`(真实目标, 修理的种类)`。
///
/// 为什么不能用 `userTaskId` 去重：那个 id 带来源前缀（`quality:` / `cloud-diff:` /
/// `cloud-question:` / `cloud-coverage:`），于是同一个目标、同一件要做的事，会因为从
/// 不同来源冒出来而各占一条。最典型的一条：q15 缺答案会同时产出
///   `quality:ANSWER_KEY_MISSING_SLOT:q15`（blocking）
///   `cloud-diff:slot:q15:answer`（非 blocking）
/// 用户看到的是「必须补答案」加一条「答案和云端不一致」——同一件事说两遍，其中一条
/// 还被说轻了。模型再提一句就变三条。
///
/// 为什么不能拿 `action` 字符串当身份：`fix_blocking_issue` 把「缺答案」和「结构不完整」
/// 盖成同一个动作，按它合并会把两件**不同**的修理并成一条，用户照做也修不好其中一件。
/// 所以身份里放的是**修理的种类**（`repairFamily`），不是动作的类别。
fn repair_key(task: &Value) -> (String, String) {
    let family = task
        .get("repairFamily")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let target = task
        .get("targetIds")
        .and_then(Value::as_array)
        .and_then(|ids| ids.first())
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        // 没有具体目标的任务（文档级问题、来源覆盖缺口、没给 targetId 的疑问）：
        // 拿 `userTaskId` 当身份。它是唯一的，因此这类任务**不会**被合并——
        // 这正是要的：两条不同的文档级问题不是同一件事，不能因为都没有目标就并成一条。
        .unwrap_or_else(|| {
            task.get("userTaskId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        });
    (target, family)
}

/// 质量问题代码 → 修理种类。
///
/// 只有「补答案」这一族需要跨源归并（`cloud-diff:slot:…:answer` 说的是同一件事）。
/// 其余代码**各自成一族**——这样「缺答案」与「结构不完整」永远不会被并成一条。
fn repair_family_for_issue_code(code: &str) -> String {
    match code {
        "ANSWER_KEY_MISSING_SLOT" | "ANSWER_MISSING" | "ANSWER_KEY_UNRESOLVED" => {
            "answer".to_string()
        }
        other => format!("issue:{other}"),
    }
}

/// 差异字段 → 修理种类。与质量侧共用 `answer` 这一族，其余各成一族。
fn repair_family_for_difference_field(field: &str) -> String {
    match field {
        "answer" => "answer".to_string(),
        other => format!("difference:{other}"),
    }
}

/// 把一条任务并入清单：按 [`repair_key`] 去重，同一件事只留**最严重**的那条。
fn push_repair_task(
    tasks: &mut Vec<Value>,
    by_key: &mut BTreeMap<(String, String), usize>,
    task: Value,
) {
    let key = repair_key(&task);
    match by_key.get(&key) {
        Some(&index) => {
            let existing_blocking = tasks[index]
                .get("blocking")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let incoming_blocking = task
                .get("blocking")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            // 同一件事被两个来源说了两遍，其中一条说轻了。留严重的那条。
            if incoming_blocking && !existing_blocking {
                tasks[index] = task;
            }
        }
        None => {
            by_key.insert(key, tasks.len());
            tasks.push(task);
        }
    }
}

/// 一条阻断任务的可定位目标。
///
/// `targetId` 缺失时以前直接落成空串，前端把它过滤掉，于是出现一条**阻断但零动作**
/// 的任务：用户看到「不处理不能导出」，却没有任何可以按的东西。阻断任务必须能被处理，
/// 所以这里按三级取目标：
///   1. `targetId` 本身；
///   2. 问题的 `sourceAnchors` 里第一个**内容节点 id**——那是一个真实存在的题面节点，
///      定位得到（实测 `SLOT_ID_MISMATCH` 这类文档级问题常常只带锚点、不带 targetId）；
///   3. 都没有 → 空，由调用方给出兜底动作（`review_source`）。
fn blocking_target_ids(issue: &Value) -> Vec<String> {
    let direct = issue
        .get("targetId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(target_id) = direct {
        return vec![target_id.to_string()];
    }
    let anchored = issue
        .get("sourceAnchors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|anchor| anchor.get("nodeIds").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .find(|value| !value.is_empty());
    match anchored {
        Some(node_id) => vec![node_id.to_string()],
        None => Vec::new(),
    }
}

/// 按**当前 canonical** 重算剩余必须由用户处理的问题。
///
/// 关键纪律：不能再去比较「冻结的本地稿」——那会把已经被云端修好的项目重新复活。
/// 这里比较的是**当前稿 vs 云端候选**的剩余差异，加上当前稿上仍然阻断的质量问题。
/// 剩余必须由用户处理的问题。**四路取并集**，缺一路都会漏：
///
/// ```text
/// 当前稿的真实质量问题（程序判定）
/// ＋ 尚未裁定的内容差异
/// ＋ 模型明确留下的未解疑问
/// ＋ 来源覆盖缺口
/// ```
///
/// 两条容易写反的地方，写在这里免得后来者顺手改回去：
///
/// 1. **「仍与初始候选不同」≠「需要用户决定」**。首遍云端候选同样会错。模型看过原文
///    之后判「候选错、当前稿对」的差异已经被了结，再拿它问用户就是逼人重复回答一个
///    已经有答案的问题——这正是本轮要消除的东西。所以差异必须先过
///    [`fresh_ruling_for_difference`]。
/// 2. **程序校验通过不能消掉模型的未解疑问**。质量问题是**当前稿**的机械/结构事实，
///    模型无法通过任何工具消除它们（`record_ruling` 只看差异，不看质量问题）；
///    反过来，模型报的疑问也不会因为校验器没报错就消失。两者是独立的两路。
fn remaining_tasks(
    root: &Path,
    item_id: &str,
    job_id: &str,
    batch_id: &str,
    rulings: &[Value],
    model_questions: &[Value],
) -> CommandResult<Vec<Value>> {
    let conn = open_library_connection(root)?;
    let Some((canonical, _)) = get_canonical_ds(&conn, item_id)? else {
        return Ok(Vec::new());
    };
    drop(conn);

    let candidate = store::read_cloud_authoring_candidate(root, job_id, batch_id)?;
    let mut tasks: Vec<Value> = Vec::new();
    // 去重键 → 已在 `tasks` 里的下标。**跨源**去重（见 `repair_key`）。
    let mut by_key: BTreeMap<(String, String), usize> = BTreeMap::new();

    // ── 1) 当前稿的真实质量问题 ────────────────────────────────────────────
    //
    // 判据与发布门禁**同一份**（`authoring_v2_commands::blocking_issue_unresolved`）。
    // 这里曾内联同一段谓词，一旦发布门禁改了判据、这里没跟上，就会出现「修复循环说
    // 还剩问题、预检却说能发布」的同稿不同判——用户被留在两套说法中间。
    //
    // 这一路**不看裁定**：模型不能通过 `record_ruling` 消除结构错误。
    for issue in crate::authoring_v2_commands::unresolved_blocking_issues(&canonical) {
        let code = issue.get("code").and_then(Value::as_str).unwrap_or("");
        let target_ids = blocking_target_ids(&issue);
        // 身份不能是 `(code, 空目标)`：两条**不同**的文档级问题会因此折叠成一条，
        // 第二条的 message 被静默丢掉（用户只看到其中一条，还以为只有一条）。
        // `issueId` 是「事实」的指纹（`quality.rs::issue`）：同一事实重算稳定，
        // 不同事实必然不同——正好用来兜底。
        let identity = match target_ids.first() {
            Some(target_id) => target_id.clone(),
            None => issue
                .get("issueId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        };
        let task_id = format!("quality:{code}:{identity}");
        push_repair_task(
            &mut tasks,
            &mut by_key,
            json!({
                "userTaskId": task_id,
                "targetIds": target_ids,
                "questionNumbers": [],
                "message": issue.get("message").cloned().unwrap_or(Value::Null),
                // 阻断任务必须有真能做完的动作。有目标时前端给「定位/去题面修改」；
                // 没有目标（纯数据级缺陷，题面上无处可改）时给 `review_source`，
                // 前端据此渲染「打开原文件核对」——那是唯一真能推进它的动作。
                "action": if target_ids.is_empty() { "review_source" } else { "fix_blocking_issue" },
                "blocking": true,
                "repairFamily": repair_family_for_issue_code(code),
            }),
        );
    }

    // ── 2) 尚未裁定的内容差异 ─────────────────────────────────────────────
    if let Some(candidate) = candidate.as_ref() {
        if let Ok(candidate_value) = serde_json::to_value(&candidate.authoring) {
            for difference in candidate_differences(&canonical, &candidate_value) {
                let (target_type, target_id, field) = difference_key(&difference);
                let task_id = format!("cloud-diff:{target_type}:{target_id}:{field}");
                match fresh_ruling_for_difference(rulings, &difference) {
                    // 已裁定「当前稿对、候选错」：差异**已了结**，不再问用户。
                    Some(ruling)
                        if ruling.get("ruling").and_then(Value::as_str)
                            == Some(crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT) =>
                    {
                        continue;
                    }
                    // 已裁定「原文件不足以定论」：仍然要人看，但**带上模型的结论与出处**，
                    // 而不是让用户从零开始重新判断一遍。
                    Some(ruling) => {
                        // 「上下文不足」与「查过但定不了」是两件事，必须分开说。
                        let insufficient = ruling.get("reason").and_then(Value::as_str)
                            == Some(crate::schema::cloud_repair_v1::CLOUD_RULING_REASON_CONTEXT_INSUFFICIENT);
                        let message = if insufficient {
                            context_insufficient_message(&ruling)
                        } else {
                            format!(
                                "{}；云端已查过原文件但无法定论：{}",
                                describe_difference(&difference),
                                ruling.get("reason").and_then(Value::as_str).unwrap_or("未说明理由")
                            )
                        };
                        let mut task = json!({
                            "userTaskId": task_id,
                            "targetIds": [target_id],
                            "message": message,
                            "action": "review_difference",
                            "blocking": false,
                            // 当前值与云端值一并给前端：任务里要能直接看到「现在是什么、云端读到的是什么」。
                            "field": field.clone(),
                            "currentValue": difference.get("canonical").cloned().unwrap_or(Value::Null),
                            "cloudValue": difference.get("candidate").cloned().unwrap_or(Value::Null),
                            "evidence": ruling.get("evidence").cloned().unwrap_or_else(|| json!([])),
                            "repairFamily": repair_family_for_difference_field(&field),
                        });
                        if insufficient {
                            // 明标出来：**这条差异没有被核对过**。三态不坍缩
                            // （not_executed / insufficient_context / passed）靠的就是它。
                            task["contextInsufficient"] = json!(true);
                        }
                        push_repair_task(&mut tasks, &mut by_key, task);
                    }
                    // 没裁定过，或裁定已被内容变化作废：这才是真正需要用户看的差异。
                    None => {
                        push_repair_task(
                            &mut tasks,
                            &mut by_key,
                            json!({
                                "userTaskId": task_id,
                                "targetIds": [target_id],
                                "message": describe_difference(&difference),
                                "action": "review_difference",
                                "blocking": false,
                                // 当前值与云端值一并给前端：任务里要能直接看到「现在是什么、云端读到的是什么」。
                                "field": field.clone(),
                                "currentValue": difference.get("canonical").cloned().unwrap_or(Value::Null),
                                "cloudValue": difference.get("candidate").cloned().unwrap_or(Value::Null),
                                "repairFamily": repair_family_for_difference_field(&field),
                            }),
                        );
                    }
                }
            }
        }
    }

    // ── 3) 模型明确留下的未解疑问 ─────────────────────────────────────────
    //
    // `finish.unresolved` 以前只进工具结果、不进最终清单，于是模型明明报了疑问，
    // 只要它没表现为候选差异或程序阻塞，就会被整份丢掉——用户看到的是「可以导出」。
    for (index, question) in model_questions.iter().enumerate() {
        let target_id = question
            .get("targetId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let task_id = match target_id.is_empty() {
            true => format!("cloud-question:{index}"),
            false => format!("cloud-question:{target_id}:{index}"),
        };
        let message = question
            .get("message")
            .or_else(|| question.get("question"))
            .or_else(|| question.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("云端在校核时留下了未能确认的疑问")
            .to_string();
        push_repair_task(
            &mut tasks,
            &mut by_key,
            json!({
                "userTaskId": task_id,
                "targetIds": if target_id.is_empty() { json!([]) } else { json!([target_id]) },
                "message": format!("云端未能确认：{message}"),
                // 有具体目标就定位到题面，没有就是文档级疑问，打开原文件抽屉核对。
                "action": if target_id.is_empty() { "review_source" } else { "review_difference" },
                "blocking": false,
                "evidence": question.get("evidence").cloned().unwrap_or_else(|| json!([])),
                // 疑问**各自成族**（带序号）：两条疑问即便指向同一个目标，也是模型报的
                // 两件不同的事，合并会把其中一条的 message 静默吃掉。宁可多一条，
                // 也不能丢掉模型明确交出来的疑问——那正是上一轮修过的缺陷。
                "repairFamily": format!("question:{index}"),
            }),
        );
    }

    // ── 4) 来源覆盖缺口 ───────────────────────────────────────────────────
    //
    // 与第 1 路的 `SIGNIFICANT_REGION_UNASSIGNED` 互补而非重复：那一条说的是「当前稿
    // 有源区域没被任何题组接住」（稿的问题），这一路说的是「云端读不到原文件的这一块」
    // （证据的问题）。云端没读到的地方，用户有权知道——否则他会以为整份文件都核过了。
    if let Some(candidate) = candidate.as_ref() {
        for (index, region) in candidate.unresolved_regions.iter().enumerate() {
            let task_id = format!("cloud-coverage:{}:{}:{index}", region.source_file_id, region.page_index);
            push_repair_task(
                &mut tasks,
                &mut by_key,
                json!({
                    "userTaskId": task_id,
                    "targetIds": [],
                    "message": format!(
                        "原文件第 {} 页云端未能读全（{}）：{}",
                        region.page_index, region.reason, region.detail
                    ),
                    "action": "review_source",
                    "blocking": false,
                    "repairFamily": format!("coverage:{index}"),
                }),
            );
        }
        for (index, note) in candidate.source_coverage_notes.iter().enumerate() {
            let task_id = format!("cloud-coverage-note:{index}");
            push_repair_task(
                &mut tasks,
                &mut by_key,
                json!({
                    "userTaskId": task_id,
                    "targetIds": [],
                    "message": note.clone(),
                    "action": "review_source",
                    "blocking": false,
                    "repairFamily": format!("coverage-note:{index}"),
                }),
            );
        }
    }

    // `repairFamily` 是**内部**去重键，不进契约：前端只认 `userTaskId` / `targetIds` /
    // `action` / `blocking` / `message` / `evidence`。留着它等于把一条随时会改的实现细节
    // 暴露给前端，迟早有人开始依赖它。
    for task in tasks.iter_mut() {
        if let Some(object) = task.as_object_mut() {
            object.remove("repairFamily");
        }
    }

    Ok(tasks)
}

/// 网关错误里，哪些是「回复收到了、但被校验器拒绝」——值得带着原因再问一次。
///
/// 传输类错误（超时、限流耗尽、5xx、连不上）不在其中：服务端没给出可纠正的回复，
/// 重问同一句话只会再烧一次预算。
fn is_constrained_retry_rejection(error: &str) -> bool {
    error.starts_with("cloud_repair_step_")
        || error.starts_with("llm_json_parse_failed")
        || error.starts_with("llm_output_truncated")
        || error.starts_with("llm_empty_content")
}

/// 读路径用：按**当前稿**重算剩余任务。
///
/// 与修复循环收尾时同一个 [`remaining_tasks`]，输入取自落盘的裁定与模型疑问。
/// 额外一条：**人已经动过的目标**上的非阻断任务（候选差异、模型疑问）不再挂出来——
/// 用户在那里做了决定，再拿云端的另一个值去问他就是逼他重复回答。阻断任务是当前稿
/// 的事实，照常保留。
pub(crate) fn current_remaining_tasks(
    root: &Path,
    item_id: &str,
    job_id: &str,
    batch_id: &str,
) -> CommandResult<Vec<Value>> {
    let stored = store::read_repair_rulings(root, job_id, batch_id)?;
    let list = |key: &str| -> Vec<Value> {
        stored
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let rulings = list("rulings");
    let questions = list("modelQuestions");
    let tasks = remaining_tasks(root, item_id, job_id, batch_id, &rulings, &questions)?;
    let conn = open_library_connection(root)?;
    let protected = match get_canonical_ds(&conn, item_id)? {
        Some((canonical, _)) => human_protected_targets(&conn, item_id, &canonical)?,
        None => Default::default(),
    };
    Ok(tasks
        .into_iter()
        .filter(|task| {
            if task.get("blocking").and_then(Value::as_bool) == Some(true) {
                return true;
            }
            let touched = task
                .get("targetIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|id| protected.contains(id) || protected.contains(&format!("answerKey:{id}")));
            !touched
        })
        .collect())
}

/// 把批次行里的修复摘要换成**此刻**的剩余任务（读路径）。
///
/// - `running`：不重算（进行中本来就没有可信清单）；
/// - 损坏 / 未知形状：原样返回；
/// - 重算失败：原样返回快照（宁可旧，不可空——空清单会被读成「没有要处理的」）；
/// - `needs_attention` 且重算后为空 → `completed`；`completed` 且重算后非空 →
///   `needs_attention`（状态跟着事实走，否则状态行与清单自相矛盾）。
pub(crate) fn refresh_repair_summary(
    root: &Path,
    item_id: &str,
    job_id: &str,
    batch_id: &str,
    repair: &Value,
) -> Value {
    let status = repair.get("status").and_then(Value::as_str).unwrap_or("");
    if status == REPAIR_STATUS_RUNNING || repair.get("reasonCode").is_some() || !repair.is_object() {
        return repair.clone();
    }
    let Ok(tasks) = current_remaining_tasks(root, item_id, job_id, batch_id) else {
        return repair.clone();
    };
    let mut refreshed = repair.clone();
    let next_status = match status {
        REPAIR_STATUS_NEEDS_ATTENTION if tasks.is_empty() => REPAIR_STATUS_COMPLETED,
        REPAIR_STATUS_COMPLETED if !tasks.is_empty() => REPAIR_STATUS_NEEDS_ATTENTION,
        other => other,
    };
    refreshed["status"] = json!(next_status);
    refreshed["remainingTasks"] = Value::Array(tasks);
    refreshed
}

/// 修复循环的编排（**对外入口**）。
///
/// `step` 是**注入的**网关调用：`(context, observations) -> 模型原始 JSON`。
/// 生产实现走真实网关（并附带原文件证据）；测试注入确定性桩。
///
/// 有限轮次 + 总超时 + 取消 + 运行归属：模型永远跑不完也没关系——循环退出后
/// 一律由后端按当前 canonical 重算剩余问题。
///
/// **终态保证**：这个函数只要返回，就一定带回一份**终态**报告（completed /
/// needs_attention / cancelled / budget_exhausted / unavailable），不会留下 `running`。
/// 理由见 [`failure_report`]：循环开工就把批次行写成 running，任何提前返回都会让
/// 批次行永久停在 running，而 job 行已经是 failed——两个界面互相矛盾。
pub(crate) fn run_repair_loop<F>(
    request: &RepairRunRequest<'_>,
    step: F,
) -> CommandResult<RepairRunReport>
where
    F: FnMut(&Value, &[Value]) -> CommandResult<Value>,
{
    run_repair_loop_in_mode(request, configured_repair_context_mode(), step)
}

/// 指定模式的编排入口（测试与两种模式对比用）。
pub(crate) fn run_repair_loop_in_mode<F>(
    request: &RepairRunRequest<'_>,
    mode: RepairContextMode,
    step: F,
) -> CommandResult<RepairRunReport>
where
    F: FnMut(&Value, &[Value]) -> CommandResult<Value>,
{
    match mode {
        RepairContextMode::Packets => run_packet_repair_loop(request, step),
        RepairContextMode::Legacy => run_legacy_repair_loop(request, step),
    }
}

/// 循环结束时的**共享收尾**：落盘裁定与疑问、按当前 canonical 重算剩余任务、判定终态。
///
/// 抽出来的理由：包模式与 legacy 模式的推进方式完全不同，但**收尾必须完全一样**。
/// 「剩余问题按当前 canonical 重算」这条纪律一旦在两处各写一遍，迟早只改一处——
/// 那正是「模型说修好了但用户清单里还挂着」这类矛盾的来源。
fn finish_repair_run(
    request: &RepairRunRequest<'_>,
    outcome: RepairRunOutcome,
) -> CommandResult<RepairRunReport> {
    let RepairRunOutcome {
        rounds,
        applied_count,
        observations,
        rulings,
        model_questions,
        finish_note,
        packets,
        unverified_evidence,
        mut status,
        mut last_error,
    } = outcome;
    // 裁定落盘。**即使这一轮没跑完也要写**：模型已经作出的判断是用户不必再回答的东西，
    // 不能因为预算耗尽就把它们一起丢掉。
    //
    // 写失败同样不能提前 return：裁定是「报告」，已经落地的修改不因它失败而回滚，
    // 但终态必须如实降级（否则用户会以为裁定都存住了）。
    if let Err(error) = store::write_repair_rulings(
        request.root,
        request.job_id,
        request.batch_id,
        // 模型留下的疑问一并落盘：读路径要按当前稿重算剩余任务，没有它们就只能
        // 在「丢掉模型的疑问」和「永远用冻结快照」之间二选一。
        &json!({ "rulings": rulings, "modelQuestions": model_questions }),
    ) {
        last_error = Some(error);
        if status == REPAIR_STATUS_COMPLETED {
            status = REPAIR_STATUS_UNAVAILABLE;
        }
    }

    // 最终完成状态由**后端**判定：模型说"都修好了"不算数。
    let remaining = match remaining_tasks(
        request.root,
        request.item_id,
        request.job_id,
        request.batch_id,
        &rulings,
        &model_questions,
    ) {
        Ok(tasks) => tasks,
        Err(error) => {
            // 算不出剩余任务时**不能**返回空清单当「没问题」：空清单在前端等于
            // 「没有需要你处理的事」。降级为 unavailable，让前端按**状态**判断，
            // 而不是按清单长度判断（见 `repairHeadline`）。
            last_error = Some(error);
            if status == REPAIR_STATUS_COMPLETED {
                status = REPAIR_STATUS_UNAVAILABLE;
            }
            Vec::new()
        }
    };
    if status == REPAIR_STATUS_COMPLETED && !remaining.is_empty() {
        status = REPAIR_STATUS_NEEDS_ATTENTION;
    }
    let edit_version = current_canonical(request)
        .ok()
        .flatten()
        .map(|(_, version)| version)
        .unwrap_or(0);

    Ok(RepairRunReport {
        status,
        rounds,
        edit_version,
        applied_count,
        observations,
        remaining_tasks: remaining,
        // 只数**此刻仍然有效**的裁定，不是 `rulings.len()`（那份是 append-only 的
        // 累积记录，含历史 / 重复 / 已失效的条目，见 `effective_adjudicated_count`）。
        adjudicated_count: adjudicated_count_now(
            request.root,
            request.item_id,
            request.job_id,
            request.batch_id,
            &rulings,
        ),
        finish_note,
        // 正常收工的运行不该带一条非空的 `lastError`：那会让界面把一次成功的修复
        // 显示成失败。`last_error` 只描述**终态**失败原因。
        last_error: if status == REPAIR_STATUS_COMPLETED {
            None
        } else {
            last_error
        },
        repair_run_id: request.repair_run_id.to_string(),
        packets,
        unverified_evidence,
    })
}

/// 循环跑完之后的原始状态（收尾前的中间态）。
struct RepairRunOutcome {
    status: &'static str,
    rounds: u32,
    applied_count: usize,
    observations: Vec<Value>,
    rulings: Vec<Value>,
    model_questions: Vec<Value>,
    finish_note: Option<String>,
    last_error: Option<String>,
    packets: Vec<Value>,
    /// 整次运行里「证据没有核验」的条数（原文没有文本层）。摘要必须如实带出，
    /// 不能被「已应用 / 已了结」的数字盖成「已核验」。
    unverified_evidence: usize,
}

/// 改造前的修复循环（legacy）：每轮附整份原文件 + 整卷上下文。
///
/// **只**保留给 L3 的最后手段与回归对照（任务书 §4.3）。生产默认走
/// [`run_packet_repair_loop`]；这条路径的语义一字未改，正是为了「改造前后行为可比」。
fn run_legacy_repair_loop<F>(
    request: &RepairRunRequest<'_>,
    mut step: F,
) -> CommandResult<RepairRunReport>
where
    F: FnMut(&Value, &[Value]) -> CommandResult<Value>,
{
    // 上下文建不出来就什么也做不了。但**仍然要返回报告**（而不是 Err）：调用方据此
    // 才能把终态写进批次行。这是唯一在写 `running` 之前就返回的路径。
    let mut context = match build_repair_context(
        request.root,
        request.item_id,
        request.job_id,
        request.batch_id,
    ) {
        Ok(context) => context,
        Err(error) => return Ok(failure_report(request, error)),
    };
    let mut observations: Vec<Value> = Vec::new();
    let mut applied_count = 0usize;
    let mut unverified_evidence = 0usize;
    let mut rounds = 0u32;
    let mut finish_note: Option<String> = None;
    let mut last_error: Option<String> = None;
    let mut status = REPAIR_STATUS_COMPLETED;
    // 模型**真的**调用过 `finish`。预算耗尽的判据必须是它，不能拿 `finish_note.is_none()`
    // 顶替：`note` 是可选字段，模型正常收工但没写 note，会被误报成「预算耗尽」。
    let mut finished = false;
    let mut repeats: BTreeMap<String, u32> = BTreeMap::new();
    // 已落盘的裁定先读回来：重试 / 重启后再跑一次修复，不该让用户第二次回答同一个问题。
    // 读回来的裁定是否仍然有效由**内容指纹**决定（见 `fresh_ruling_for_difference`），
    // 所以这里不需要额外判断「是不是同一轮」。
    let mut rulings: Vec<Value> = match store::read_repair_rulings(
        request.root,
        request.job_id,
        request.batch_id,
    ) {
        Ok(rulings) => rulings
            .and_then(|value| value.get("rulings").and_then(Value::as_array).cloned())
            .unwrap_or_default(),
        Err(error) => {
            // 读不回旧裁定不该让整次修复失败——但必须如实记下来：这次可能重复问了
            // 用户一个上次已经回答过的问题。
            last_error = Some(error);
            Vec::new()
        }
    };
    let mut model_questions: Vec<Value> = Vec::new();

    // 开工就先落一次 `running`：修复循环最长十分钟，用户在它结束之前就该能看到
    // 「云端正在自动修复」，而不是对着上一次的旧状态猜。
    let start_version = current_canonical(request)
        .ok()
        .flatten()
        .map(|(_, version)| version)
        .unwrap_or(0);
    report_progress(
        request,
        RepairProgress {
            status: REPAIR_STATUS_RUNNING,
            round: 0,
            applied_count: 0,
            // 读回来的旧裁定**已经**是事实，不用等循环跑完才算。按当前差异筛一遍，
            // 只算此刻仍然有效的那些（见 `effective_adjudicated_count`）。
            adjudicated_count: adjudicated_count_now(
                request.root,
                request.item_id,
                request.job_id,
                request.batch_id,
                &rulings,
            ),
            edit_version: start_version,
        },
    );

    while rounds < request.max_rounds {
        if (request.cancelled)() {
            status = REPAIR_STATUS_CANCELLED;
            break;
        }
        if Instant::now() >= request.deadline {
            status = REPAIR_STATUS_BUDGET_EXHAUSTED;
            break;
        }
        rounds += 1;
        let raw = match step(&context, &observations) {
            Ok(raw) => raw,
            // 回复**收到了**但被校验器拒绝（未知工具、坏 JSON、截断）：同一回合内给模型
            // **一次**带原因的受约束重试。原因以 `repairNote` 放进观察，网关据此把它写进
            // prompt。传输类错误（超时、5xx、连不上）不重试——重问同一句话没有意义。
            Err(error)
                if is_constrained_retry_rejection(&error)
                    && Instant::now() < request.deadline
                    && !(request.cancelled)() =>
            {
                observations.push(json!({
                    "schemaVersion": "CloudRepairToolResultV1",
                    "callId": Value::Null,
                    "status": "rejected",
                    "errors": [error.clone()],
                    "repairNote": error.clone(),
                }));
                match step(&context, &observations) {
                    Ok(raw) => raw,
                    Err(second) => {
                        last_error = Some(format!("{second};first_rejection={error}"));
                        status = REPAIR_STATUS_UNAVAILABLE;
                        break;
                    }
                }
            }
            Err(error) => {
                last_error = Some(error);
                status = REPAIR_STATUS_UNAVAILABLE;
                break;
            }
        };
        let call = match parse_tool_call(&raw) {
            Ok(call) => call,
            Err(error) => {
                // 解析失败也算一个回合：把具体错误回给模型，让它改对再交。
                //
                // **不写进 `last_error`**：这是一次可恢复的往返（错误已经回给模型，
                // 它下一轮可能就改对了），而 `last_error` 描述的是**终态**失败原因。
                // 以前写进去且从不清理，于是一次「中途被拒一次、随后正常收工」的修复，
                // 会带着一条非空的 lastError 返回——界面把它当失败显示，用户以为
                // 云端出错了，实际这次修复是成功的。具体错误在 observations 里，
                // 诊断看得到，不丢信息。
                observations.push(json!({
                    "schemaVersion": "CloudRepairToolResultV1",
                    "callId": raw.get("callId").cloned().unwrap_or(Value::Null),
                    "status": "rejected",
                    "errors": [error],
                }));
                continue;
            }
        };

        // 完全相同的调用重复且无进展 ⇒ 停下，不再烧预算。
        let fingerprint = format!(
            "{}:{}",
            call.tool,
            serde_json::to_string(&call.arguments).unwrap_or_default()
        );
        let counter = repeats.entry(fingerprint).or_insert(0);
        *counter += 1;
        if *counter > REPEAT_LIMIT {
            observations.push(
                serde_json::to_value(CloudRepairToolResultV1::rejected(
                    &call.call_id,
                    vec!["CLOUD_REPAIR_NO_PROGRESS: repeated identical tool call with no new information".to_string()],
                ))
                .unwrap_or(Value::Null),
            );
            status = REPAIR_STATUS_NEEDS_ATTENTION;
            break;
        }

        let is_finish = call.tool == "finish";
        let (result, applied) = execute_tool(request, &call, rounds, &context, None);
        unverified_evidence += evidence_unverifiable_count(&result.result);
        // 裁定：从**工具真实返回**里取，不重新解释一遍模型输入——否则「记录了什么」
        // 与「回给模型什么」可能不一致，而落盘的必须是后者（模型据此继续推理）。
        if call.tool == "record_ruling" {
            if let Some(recorded) = result.result.get("recorded").and_then(Value::as_array) {
                rulings.extend(recorded.iter().cloned());
            }
        }
        if let Some(count) = applied {
            applied_count += count;
            // 写成功之后必须重读上下文：版本变了，模型手里的 baseVersion 已过期。
            match build_repair_context(
                request.root,
                request.item_id,
                request.job_id,
                request.batch_id,
            ) {
                Ok(next) => {
                    context = next;
                    repeats.clear();
                    // 每落下一批有效修改立刻上报：调用方据此把新版本与已修数量写进产品
                    // 状态、并发出事件让画布跟上。**不等循环结束**——否则用户看到的是
                    // 「改了但界面没变」。
                    report_progress(
                        request,
                        RepairProgress {
                            status: REPAIR_STATUS_RUNNING,
                            round: rounds,
                            applied_count,
                            adjudicated_count: adjudicated_count_now(
                                request.root,
                                request.item_id,
                                request.job_id,
                                request.batch_id,
                                &rulings,
                            ),
                            edit_version: context
                                .get("editVersion")
                                .and_then(Value::as_i64)
                                .unwrap_or(0),
                        },
                    );
                }
                Err(error) => {
                    // 修改**已经落库**，只是读不回新上下文。不能提前 return：
                    // 那样批次行会停在 running，而已落地的修改也没有任何摘要描述。
                    // 记下错误、降级为 unavailable，继续走下面的收尾（重算剩余问题）。
                    last_error = Some(error);
                    status = REPAIR_STATUS_UNAVAILABLE;
                    break;
                }
            }
        }
        observations.push(serde_json::to_value(&result).unwrap_or(Value::Null));
        if is_finish {
            finished = true;
            finish_note = call
                .arguments
                .get("note")
                .and_then(Value::as_str)
                .map(str::to_string);
            // 模型明确留下的未解疑问。**必须进最终清单**：它不表现为候选差异、也不
            // 表现为程序阻塞，只在 `finish` 里说了一次；以前丢掉它，用户看到的就是
            // 「可以导出」，而模型其实报过疑问。
            if let Some(unresolved) = call.arguments.get("unresolved").and_then(Value::as_array) {
                model_questions = unresolved
                    .iter()
                    .map(|entry| match entry {
                        // 允许模型直接给一句话，不强迫它为每条疑问编一个 targetId。
                        Value::String(text) => json!({ "message": text }),
                        other => other.clone(),
                    })
                    .collect();
            }
            break;
        }
    }
    // 预算耗尽 = 轮次用满**且模型没有收工**。判据是 `finished`，不是 `finish_note.is_none()`
    // ——后者会把「正常 finish 但没写 note」误判成预算耗尽（`note` 是可选字段）。
    if rounds >= request.max_rounds && status == REPAIR_STATUS_COMPLETED && !finished {
        status = REPAIR_STATUS_BUDGET_EXHAUSTED;
    }

    finish_repair_run(
        request,
        RepairRunOutcome {
            status,
            rounds,
            applied_count,
            observations,
            rulings,
            model_questions,
            finish_note,
            last_error,
            packets: Vec::new(),
            unverified_evidence,
        },
    )
}

/// 包模式的修复循环：**逐包推进**。
///
/// 与 legacy 的三点根本差别：
/// 1. 每轮的上下文是**一个校核包**（本地预切、范围明确），不是整卷 + 整份原文件；
/// 2. 包内的观察**不跨包累积**（换包清空）——否则第 5 个包会背着前 4 个包的噪声，
///    而它看到的上下文本该是自足的；
/// 3. 上下文不够时有一条**明说的出口**（`report_insufficient_context`）与逐级升级的
///    阶梯，最差也如实变成用户清单里的一条「云端没能拿到足够的原文」。
///
/// 全局约束仍是总超时 + 取消 + 运行归属；每包另有自己的回合与抓取预算。
fn run_packet_repair_loop<F>(
    request: &RepairRunRequest<'_>,
    step: F,
) -> CommandResult<RepairRunReport>
where
    F: FnMut(&Value, &[Value]) -> CommandResult<Value>,
{
    run_packet_repair_loop_with_clock(request, step, &Instant::now)
}

/// 同 [`run_packet_repair_loop`]，但时钟可注入：`now` 返回「当前时刻」。
///
/// 测试用它驱动**虚拟时钟**：每「轮」把钟拨快固定的模型延迟，deadline 判定变成
/// 虚拟时间上的纯算术——「总时限按包数放宽装不装得下」不再依赖墙钟与机器负载
/// （真实 sleep 的版本在全量并行时会被挤爆，余量只有约 2 秒）。
fn run_packet_repair_loop_with_clock<F>(
    request: &RepairRunRequest<'_>,
    mut step: F,
    now: &dyn Fn() -> Instant,
) -> CommandResult<RepairRunReport>
where
    F: FnMut(&Value, &[Value]) -> CommandResult<Value>,
{
    // 上下文建不出来就什么也做不了，但**仍然要返回报告**（见 `failure_report`）。
    let mut context = match build_repair_context(
        request.root,
        request.item_id,
        request.job_id,
        request.batch_id,
    ) {
        Ok(context) => context,
        Err(error) => return Ok(failure_report(request, error)),
    };
    let source_index = load_packet_source_index(request, &context);
    let mut queue: std::collections::VecDeque<Value> =
        match plan_repair_packets(request, &context, &source_index) {
            Ok(packets) => packets.into(),
            Err(error) => return Ok(failure_report(request, error)),
        };
    // P11：总时限按包数线性放宽（封顶 3× 基础）。循环内所有截止判断都用这个值；
    // `request.deadline` 保持调用方给的原始值，仅供这里换算。
    let loop_started = now();
    let deadline = scaled_packet_deadline(request.deadline, loop_started, queue.len());

    let mut rulings: Vec<Value> = match store::read_repair_rulings(
        request.root,
        request.job_id,
        request.batch_id,
    ) {
        Ok(rulings) => rulings
            .and_then(|value| value.get("rulings").and_then(Value::as_array).cloned())
            .unwrap_or_default(),
        Err(error) => {
            // 读不回旧裁定不该让整次修复失败——但必须如实记下来（这次可能重复问了
            // 用户一个上次已经回答过的问题）。
            let _ = error;
            Vec::new()
        }
    };
    let mut model_questions: Vec<Value> = Vec::new();
    let mut observations: Vec<Value> = Vec::new();
    let mut applied_count = 0usize;
    let mut unverified_evidence = 0usize;
    let mut rounds = 0u32;
    let mut status = REPAIR_STATUS_COMPLETED;
    let mut last_error: Option<String> = None;
    let mut finish_note: Option<String> = None;
    let mut finished = false;
    let mut used_full_source = false;
    let mut packet_reports: Vec<Value> = Vec::new();
    // 有包**没做完**（轮数用尽 / 无进展 / 撞上全局闸）。它决定「队列跑空」之后该报
    // `completed` 还是 `budget_exhausted`：每包都收工了却报「预算耗尽」，用户会以为
    // 云端跑超时了——那和「云端跑完了」是两件事。
    let mut incomplete = false;
    // 已经收工的包（按**稳定 id**）。重切之后按 id 过滤，已做完的不会被重新排队。
    let mut done_packets: BTreeSet<String> = BTreeSet::new();
    // 全局回合上限按包数派生：任务书给的是「每包 5 轮」，而调用方的 `max_rounds`
    // （legacy 默认 6）是**整卷**口径。若照搬，第二个包起就会被饿死——那会让「包」
    // 反而比整卷更贵。真正的全局约束是总超时，这里只做一道防止无限重切的闸。
    let mut global_round_cap = PACKET_MAX_ROUNDS * (queue.len().max(1) as u32);

    let start_version = current_canonical(request)
        .ok()
        .flatten()
        .map(|(_, version)| version)
        .unwrap_or(0);
    report_progress(
        request,
        RepairProgress {
            status: REPAIR_STATUS_RUNNING,
            round: 0,
            applied_count: 0,
            adjudicated_count: adjudicated_count_now(
                request.root,
                request.item_id,
                request.job_id,
                request.batch_id,
                &rulings,
            ),
            edit_version: start_version,
        },
    );

    while let Some(mut packet) = queue.pop_front() {
        if (request.cancelled)() {
            status = REPAIR_STATUS_CANCELLED;
            break;
        }
        if now() >= deadline {
            status = REPAIR_STATUS_BUDGET_EXHAUSTED;
            break;
        }
        if rounds >= global_round_cap {
            status = REPAIR_STATUS_BUDGET_EXHAUSTED;
            break;
        }

        let packet_id = packet
            .get("packetId")
            .and_then(Value::as_str)
            .unwrap_or("pkt-unknown")
            .to_string();
        let question_numbers: Vec<u32> = packet
            .get("questionNumbers")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_u64).map(|n| n as u32).collect())
            .unwrap_or_default();
        let task_ids: BTreeSet<String> = packet
            .get("taskIds")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let mut level = packet
            .get("escalationLevel")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        let mut budget = grab::GrabBudget::new();
        // 包内观察：换包清空，不跨包累积。
        let mut packet_observations: Vec<Value> = Vec::new();
        let mut packet_rounds = 0u32;
        let mut packet_rulings = 0usize;
        let mut packet_edits = 0usize;
        let mut packet_insufficient = 0usize;
        let mut packet_unverified = 0usize;
        let mut packet_status = "rounds_exhausted";
        let mut repeats: BTreeMap<String, u32> = BTreeMap::new();
        let mut escalated = false;
        // 该收摊了：记录完本包诊断就退出外层循环（取消 / 超时 / 全局预算 / 模型 finish /
        // 网关不可用）。用标志而不是 `break 'packets`，是为了**不让这一包的诊断丢掉**。
        let mut stop_all = false;

        loop {
            if (request.cancelled)() {
                status = REPAIR_STATUS_CANCELLED;
                packet_status = "cancelled";
                stop_all = true;
                break;
            }
            if now() >= deadline {
                status = REPAIR_STATUS_BUDGET_EXHAUSTED;
                packet_status = "deadline";
                stop_all = true;
                break;
            }
            if rounds >= global_round_cap {
                status = REPAIR_STATUS_BUDGET_EXHAUSTED;
                packet_status = "global_round_budget";
                stop_all = true;
                break;
            }
            if packet_rounds >= PACKET_MAX_ROUNDS {
                break;
            }
            packet_rounds += 1;
            rounds += 1;
            let raw = match step(&packet, &packet_observations) {
                Ok(raw) => raw,
                // 与 legacy 同一条规则：回复**收到了**但被校验器拒绝 ⇒ 同一回合内给
                // **一次**带原因的受约束重试；传输类错误不重试。
                Err(error)
                    if is_constrained_retry_rejection(&error)
                        && now() < deadline
                        && !(request.cancelled)() =>
                {
                    packet_observations.push(json!({
                        "schemaVersion": "CloudRepairToolResultV1",
                        "callId": Value::Null,
                        "status": "rejected",
                        "errors": [error.clone()],
                        "repairNote": error.clone(),
                    }));
                    match step(&packet, &packet_observations) {
                        Ok(raw) => raw,
                        Err(second) => {
                            last_error = Some(format!("{second};first_rejection={error}"));
                            status = REPAIR_STATUS_UNAVAILABLE;
                            packet_status = "unavailable";
                            break;
                        }
                    }
                }
                Err(error) => {
                    last_error = Some(error);
                    status = REPAIR_STATUS_UNAVAILABLE;
                    packet_status = "unavailable";
                    break;
                }
            };
            let call = match parse_tool_call(&raw) {
                Ok(call) => call,
                Err(error) => {
                    // 解析失败也算一个回合：把具体错误回给模型，让它改对再交。
                    packet_observations.push(json!({
                        "schemaVersion": "CloudRepairToolResultV1",
                        "callId": raw.get("callId").cloned().unwrap_or(Value::Null),
                        "status": "rejected",
                        "errors": [error],
                    }));
                    continue;
                }
            };
        // 指纹里带上**本包当前升级级别**。理由：模型连着两轮说「不够」时，后端在中间
        // 已经给它加了材料（L2 整页图 / L3 整份原文）——那不是「原地打转」，而是升级
        // 阶梯在推进。若不带上级别，L1 的第三次重复就会被判成 `no_progress` 而**掐断
        // 阶梯**，本包永远到不了 L4，最后只好谎报「预算耗尽」。
        let fingerprint = format!(
            "{}:{}:{}",
            call.tool,
            level,
            serde_json::to_string(&call.arguments).unwrap_or_default()
        );
        let counter = repeats.entry(fingerprint).or_insert(0);
        *counter += 1;
        if *counter > REPEAT_LIMIT {
            packet_observations.push(
                serde_json::to_value(CloudRepairToolResultV1::rejected(
                    &call.call_id,
                    vec![
                        "CLOUD_REPAIR_NO_PROGRESS: repeated identical tool call with no new information"
                            .to_string(),
                    ],
                ))
                .unwrap_or(Value::Null),
            );
            packet_status = "no_progress";
            break;
        }

            let is_finish_packet = call.tool == crate::schema::cloud_repair_v1::CLOUD_REPAIR_FINISH_PACKET_TOOL;
            let is_finish = call.tool == "finish";
            let is_insufficient =
                call.tool == crate::schema::cloud_repair_v1::CLOUD_REPAIR_INSUFFICIENT_CONTEXT_TOOL;
            let (result, applied) = {
                let mut tools = PacketTools {
                    source: &source_index,
                    budget: &mut budget,
                    task_ids: task_ids.clone(),
                    question_numbers: question_numbers.clone(),
                };
                execute_tool(request, &call, rounds, &packet, Some(&mut tools))
            };
            // P9：没有文本层时的「证据未核验」如实累计——进逐包诊断与整次摘要。
            packet_unverified += evidence_unverifiable_count(&result.result);
            unverified_evidence += evidence_unverifiable_count(&result.result);
            // 抓取工具的调用本身是 L1 尝试，即使来源不可用；上下文不足则只在调用真的
            // 被接受时计入 L1。被拒的旧 packetId / malformed need 不是一次有效报告。
            let valid_insufficient = is_insufficient
                && result.status == crate::schema::cloud_repair_v1::CloudRepairToolStatusV1::Ok;
            if valid_insufficient
                || matches!(
                    call.tool.as_str(),
                    "read_source" | "search_source" | "read_page_region" | "read_passage"
                        | "read_candidate"
                )
            {
                level = level.max(1);
            }
            // 包自身的级别必须与这里推进的 `level` 同步。请求体（以及 `llm-calls.jsonl`）
            // 读的就是 `context.escalationLevel` 这个字段：不同步的话，「这一轮到底在 L 几」
            // 会有两个答案——诊断说 L1，模型看到的却是 L0。
            packet["escalationLevel"] = json!(level);
            if call.tool == "record_ruling" {
                if let Some(recorded) = result.result.get("recorded").and_then(Value::as_array) {
                    packet_rulings += recorded.len();
                    rulings.extend(recorded.iter().cloned());
                }
            }
            if valid_insufficient {
                packet_insufficient += 1;
                // 取到的证据**并入本包**，下一轮请求就带着它。
                merge_fetched_evidence(&mut packet, &result);
                let unsatisfied = result
                    .result
                    .get("unsatisfied")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                if unsatisfied > 0 || budget.exhausted() {
                    // 抓取预算用尽（或需求本身取不到）⇒ 升级一档，由后端加材料。
                    level = escalate_packet(
                        request,
                        &source_index,
                        &mut packet,
                        level,
                        &mut used_full_source,
                    );
                    escalated = true;
                    if level >= PACKET_MAX_ESCALATION {
                        packet_status = "context_insufficient";
                        break;
                    }
                }
            }
            packet_observations.push(serde_json::to_value(&result).unwrap_or(Value::Null));
            observations.push(serde_json::to_value(&result).unwrap_or(Value::Null));
            if is_finish_packet || is_finish {
                let unresolved = call.arguments.get("unresolved").and_then(Value::as_array);
                if let Some(unresolved) = unresolved {
                    model_questions.extend(unresolved.iter().map(|entry| match entry {
                        Value::String(text) => json!({ "message": text }),
                        other => other.clone(),
                    }));
                }
                if is_finish {
                    finished = true;
                    finish_note = call
                        .arguments
                        .get("note")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    packet_status = "run_finished";
                    stop_all = true;
                    break;
                }
                packet_status = "finished";
                done_packets.insert(packet_id.clone());
                break;
            }
            if let Some(count) = applied {
                applied_count += count;
                packet_edits += count;
                // 写成功之后必须重读上下文（版本变了）并**重切受影响的包**。
                match build_repair_context(
                    request.root,
                    request.item_id,
                    request.job_id,
                    request.batch_id,
                ) {
                    Ok(next) => context = next,
                    Err(error) => {
                        // 修改**已经落库**，只是读不回新上下文：不能提前 return（那样批次行
                        // 会停在 running），记下错误、降级为 unavailable，走统一收尾。
                        last_error = Some(error);
                        status = REPAIR_STATUS_UNAVAILABLE;
                        packet_status = "unavailable";
                        break;
                    }
                }
                match plan_repair_packets(request, &context, &source_index) {
                    Ok(next) => {
                        // 重切：已收工的包（按稳定 id）不再排队；本包若仍有差异会以**新切片**
                        // 重新排队，`editVersion` 与目标 id 都刷新过。
                        queue = next
                            .into_iter()
                            .filter(|candidate| {
                                candidate
                                    .get("packetId")
                                    .and_then(Value::as_str)
                                    .is_some_and(|id| !done_packets.contains(id))
                            })
                            .collect();
                        global_round_cap = rounds
                            + PACKET_MAX_ROUNDS * (queue.len().max(1) as u32);
                    }
                    Err(error) => {
                        last_error = Some(error);
                        status = REPAIR_STATUS_UNAVAILABLE;
                        packet_status = "unavailable";
                        break;
                    }
                }
                report_progress(
                    request,
                    RepairProgress {
                        status: REPAIR_STATUS_RUNNING,
                        round: rounds,
                        applied_count,
                        adjudicated_count: adjudicated_count_now(
                            request.root,
                            request.item_id,
                            request.job_id,
                            request.batch_id,
                            &rulings,
                        ),
                        edit_version: context
                            .get("editVersion")
                            .and_then(Value::as_i64)
                            .unwrap_or(0),
                    },
                );
                packet_status = "edited";
                break;
            }
        }

        // ── L4：模型始终拿不到足够材料 ⇒ 后端**代记** cannot_resolve ─────────
        //
        // 这是「上下文不足绝不变成猜一个」的落点：既不编答案，也不假装核对过。剩下的
        // 差异带着理由码 `CONTEXT_INSUFFICIENT` 进用户清单，文案明说云端没拿到材料。
        if packet_status == "context_insufficient" {
            let forced =
                force_context_insufficient_rulings(&packet, &mut rulings, rounds.max(1) as u32);
            packet_rulings += forced;
        }
        // 「这一包没做完」：轮数用尽、原地打转、或撞上全局闸。取消 / 网关不可用 / 截止
        // 时间已经各自把 `status` 降级了，这里只管**队列跑空**时该怎么收尾。
        if matches!(
            packet_status,
            "rounds_exhausted" | "no_progress" | "global_round_budget" | "deadline"
        ) {
            incomplete = true;
        }
        packet_reports.push(json!({
            "packetId": packet_id,
            "escalationLevel": level,
            "escalated": escalated,
            "status": packet_status,
            "rounds": packet_rounds,
            "rulings": packet_rulings,
            "edits": packet_edits,
            "insufficientContext": packet_insufficient,
            "evidenceUnverifiable": packet_unverified,
            "questionNumbers": question_numbers,
            "scopePages": packet.pointer("/scope/pages").cloned().unwrap_or_else(|| json!([])),
            "estimatedInputTokens": packet_token_estimate(&packet),
        }));
        if status == REPAIR_STATUS_UNAVAILABLE || stop_all {
            break;
        }
    }

    // 队列跑空之后，终态由**后端**按事实判定，三种情况必须分开：
    //
    // - 有包被判「上下文不足」⇒ `needs_attention`：不是预算不够，而是**材料不够**，
    //   用户有事可做（对照原文确认）；报成 `budget_exhausted` 会被读成「云端跑超时了」。
    // - 有包没做完（轮数用尽 / 原地打转 / 撞闸）⇒ `budget_exhausted`，如实说没跑完。
    // - 每包都收工（`finish_packet` / 落地过编辑）且队列自然跑空 ⇒ `completed`。包模式下
    //   `finish_packet` 就是**正常收工**，`finish` 只用来提前结束整次运行；此时再报
    //   「预算耗尽」是在冤枉一次成功的校核。
    //
    // 注意 `remaining_tasks` 仍会在收尾处按当前 canonical 重算：稿子里还有没解决的差异
    // 时，状态会被抬成 `needs_attention`（见 `finish_repair_run`）。
    if status == REPAIR_STATUS_COMPLETED && !finished {
        let any_context_insufficient = packet_reports
            .iter()
            .any(|packet| packet.get("status").and_then(Value::as_str) == Some("context_insufficient"));
        status = if any_context_insufficient {
            REPAIR_STATUS_NEEDS_ATTENTION
        } else if incomplete {
            REPAIR_STATUS_BUDGET_EXHAUSTED
        } else {
            REPAIR_STATUS_COMPLETED
        };
    }

    finish_repair_run(
        request,
        RepairRunOutcome {
            status,
            rounds,
            applied_count,
            observations,
            rulings,
            model_questions,
            finish_note,
            last_error,
            packets: packet_reports,
            unverified_evidence,
        },
    )
}

/// 读原文页索引（逐行文本 / 页图 / 答案页 / 段落）。
///
/// 读不到**不**让整次修复失败：包仍然带着稿件切片与差异，只是原文证据为空——那会被
/// `scopeManifest` 如实写出来，模型据此可以 `report_insufficient_context`。
fn load_packet_source_index(
    request: &RepairRunRequest<'_>,
    context: &Value,
) -> packets::SourcePageIndex {
    // 这里只需要来源**身份**（id / 类型）。曾经调 `cloud_source_evidence`，它会把整份
    // PDF 重新渲染一遍并覆盖页图缓存：测试种好的页图被一次失败的渲染清空，区域图
    // 附不上、编辑落不了库；真实卷子上还每次白吃一段修复时限（2026-09-25 质量方复核
    // 的根因）。只要身份就走只读入口，绝不渲染。
    let source_meta = crate::auto_pipeline::cloud_source_identity(request.root, request.job_id)
        .unwrap_or(Value::Null);
    let source_file_id = source_meta
        .get("sourceFileId")
        .and_then(Value::as_str)
        .or_else(|| context.get("sourceFileId").and_then(Value::as_str))
        .unwrap_or(request.job_id)
        .to_string();
    let kind = source_meta
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    grab::load_source_index(request.root, request.job_id, &source_file_id, kind)
}

/// 按当前差异切包，并把区域图裁剪出来。
fn plan_repair_packets(
    request: &RepairRunRequest<'_>,
    context: &Value,
    source_index: &packets::SourcePageIndex,
) -> CommandResult<Vec<Value>> {
    let canonical = current_canonical(request)?
        .map(|(document, _)| document)
        .unwrap_or(Value::Null);
    let candidate = store::read_cloud_authoring_candidate(
        request.root,
        request.job_id,
        request.batch_id,
    )?
    .and_then(|candidate| serde_json::to_value(&candidate.authoring).ok())
    .unwrap_or(Value::Null);
    let differences: Vec<Value> = context
        .get("differences")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let blocking_issues = crate::authoring_v2_commands::unresolved_blocking_issues(&canonical);
    let protected: BTreeSet<String> = context
        .get("protectedTargets")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let edit_version = context.get("editVersion").and_then(Value::as_i64).unwrap_or(0);

    let planned = packets::plan_packets(&packets::PacketPlanInput {
        canonical: &canonical,
        candidate: &candidate,
        differences: &differences,
        blocking_issues: &blocking_issues,
        protected: &protected,
        source: source_index,
        edit_version,
    });
    Ok(planned
        .into_iter()
        .map(|mut packet| {
            let packet_id = packet
                .get("packetId")
                .and_then(Value::as_str)
                .unwrap_or("pkt")
                .to_string();
            let regions: Vec<Value> = packet
                .pointer("/sourceEvidence/regions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let materialized = grab::materialize_regions(
                request.root,
                request.job_id,
                &packet_id,
                source_index,
                &regions,
            );
            packet["sourceEvidence"]["regions"] = json!(materialized);
            enforce_packet_budget(&mut packet);
            packet
        })
        .collect())
}

/// 单包输入的粗估（字符 / 4 + 每张图固定值），与 `packets.rs` 同一套系数。
fn packet_token_estimate(packet: &Value) -> usize {
    let chars = serde_json::to_string(packet)
        .map(|text| text.chars().count())
        .unwrap_or(0);
    let images = packet
        .pointer("/sourceEvidence/regions")
        .and_then(Value::as_array)
        .map(|regions| regions.iter().filter(|region| !region["image"].is_null()).count())
        .unwrap_or(0);
    chars / packets::PACKET_CHARS_PER_TOKEN + images * packets::PACKET_IMAGE_TOKENS
}

/// 超预算时按 §4.3 的退让顺序「整页图 → 区域图」丢**最少够用**的图，并**如实写明**少了什么。
///
/// 静默丢图比丢文字更危险：文字还在包里，模型至少知道自己读到了什么；而一张「本该
/// 附上但没附」的图会让模型以为自己看过那一块。
///
/// 为什么不是「一超预算就把图全清掉」：那样最坏情况下会把 L2 刚补的整页图连同区域图
/// 一起丢掉，而任务书 §4.3 的退让阶梯是逐级的（整页图 → 区域图 → 缩小裁剪 → 拆包）。
/// 每张图按固定值计费，所以「至少得丢几张」可以直接算出来，多丢一张都是白丢。
fn enforce_packet_budget(packet: &mut Value) {
    let estimate = packet_token_estimate(packet);
    if estimate <= packets::PACKET_TOKEN_BUDGET {
        return;
    }
    let Some(regions) = packet
        .pointer("/sourceEvidence/regions")
        .and_then(Value::as_array)
    else {
        return;
    };
    if regions.is_empty() {
        return;
    }
    // 只有真的带了图的条目才占 token（`packet_token_estimate` 同一判据）。
    let costly: Vec<usize> = (0..regions.len())
        .filter(|index| !regions[*index]["image"].is_null())
        .collect();
    if costly.is_empty() {
        return;
    }
    let excess = estimate - packets::PACKET_TOKEN_BUDGET;
    let per_image = packets::PACKET_IMAGE_TOKENS.max(1);
    let must_drop = ((excess + per_image - 1) / per_image).min(costly.len());
    // 退让顺序：整页图信息量最小（只是「这一页长这样」），先丢；仍不够才动区域图。
    let mut ordered = costly;
    ordered.sort_by_key(|index| {
        let whole_page = regions[*index].get("bbox").map_or(true, Value::is_null);
        (if whole_page { 0u8 } else { 1u8 }, *index)
    });
    let drop_set: BTreeSet<usize> = ordered.into_iter().take(must_drop).collect();
    let kept: Vec<Value> = regions
        .iter()
        .enumerate()
        .filter(|(index, _)| !drop_set.contains(index))
        .map(|(_, region)| region.clone())
        .collect();
    if let Some(list) = packet
        .pointer_mut("/sourceEvidence/regions")
        .and_then(Value::as_array_mut)
    {
        *list = kept;
    }
    let still_over = packet_token_estimate(packet) > packets::PACKET_TOKEN_BUDGET;
    if let Some(object) = packet.as_object_mut() {
        let note = if still_over {
            // 图全丢完了还是超预算：这时**没有**下一步退让可走。单题组的正文既不裁也不拆
            // （`packets.rs::plan_packets` 的拆分只按题组），所以如实写清楚，而不是留一个
            // 看起来已经退让到位的包。
            format!(
                "{must_drop} image(s) were dropped: the packet exceeded the {}-token budget. \
                 Whole-page images go first, then region crops. Dropping every image was still \
                 not enough — the text layer of one task group is never trimmed or split \
                 further, so this packet goes out above the per-packet budget (still far below \
                 the whole paper).",
                packets::PACKET_TOKEN_BUDGET
            )
        } else {
            format!(
                "{must_drop} image(s) were dropped: the packet exceeded the {}-token \
                 budget. Whole-page images go first, then region crops. The text layer for \
                 the pages in scope is still below.",
                packets::PACKET_TOKEN_BUDGET
            )
        };
        object.insert("budgetNote".to_string(), json!(note));
    }
}

/// 升级之后按预算退让，并把**实际结果**写进 `escalationNote`。
///
/// 为什么不能只调 [`enforce_packet_budget`]：L2 写下的 note 是「已附整页图」，而预算可能
/// 紧接着就把那些图清掉。那样包里会同时躺着「L2 已附整页图」和「N 张图被丢」两句互相
/// 矛盾的话——模型看到的是一张图都没有，而诊断说它看过。这里让 note 跟着**实际留下的
/// 图**走：没发生的事不许写成发生了。
fn apply_budget_after_escalation(packet: &mut Value) {
    let image_count = |packet: &Value| -> usize {
        packet
            .pointer("/sourceEvidence/regions")
            .and_then(Value::as_array)
            .map(|regions| {
                regions
                    .iter()
                    .filter(|region| !region["image"].is_null())
                    .count()
            })
            .unwrap_or(0)
    };
    let before = image_count(packet);
    enforce_packet_budget(packet);
    let after = image_count(packet);
    if before <= after {
        return;
    }
    let Some(note) = packet
        .pointer("/scopeManifest/escalationNote")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return;
    };
    packet["scopeManifest"]["escalationNote"] = json!(format!(
        "{note} Correction: {} of those image(s) were then dropped because the packet exceeds \
         the {}-token budget; the text layer for the pages in scope is still below.",
        before - after,
        packets::PACKET_TOKEN_BUDGET
    ));
}

/// 把一次 `report_insufficient_context` 取到的证据并入本包。
fn merge_fetched_evidence(packet: &mut Value, result: &CloudRepairToolResultV1) {
    let Some(satisfied) = result.result.get("satisfied").and_then(Value::as_array) else {
        return;
    };
    let mut fetched: Vec<Value> = packet
        .pointer("/sourceEvidence/fetched")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for entry in satisfied {
        let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
        let value = entry.get("result").cloned().unwrap_or(Value::Null);
        match kind {
            "pages" => {
                for page in value
                    .get("pages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    upsert_packet_page(packet, page);
                }
                fetched.push(json!({"kind": "pages", "pageIndexes": value
                    .get("pages")
                    .and_then(Value::as_array)
                    .map(|pages| pages.iter().filter_map(|page| page.get("pageIndex").cloned()).collect::<Vec<_>>())
                    .unwrap_or_default()}));
            }
            "page_region" => {
                // 取到的页图必须进 `regions`，网关才会把它作为图片附到下一轮请求上。
                let mut regions: Vec<Value> = packet
                    .pointer("/sourceEvidence/regions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                regions.push(json!({
                    "pageIndex": value.get("pageIndex").cloned().unwrap_or(Value::Null),
                    "bbox": Value::Null,
                    "taskIds": [],
                    "image": value.get("image").cloned().unwrap_or(Value::Null),
                    "note": "fetched by report_insufficient_context",
                }));
                packet["sourceEvidence"]["regions"] = json!(regions);
                fetched.push(json!({"kind": "page_region", "pageIndex": value.get("pageIndex").cloned().unwrap_or(Value::Null)}));
            }
            other => {
                fetched.push(json!({"kind": other, "result": value}));
            }
        }
    }
    packet["sourceEvidence"]["fetched"] = json!(fetched);
    enforce_packet_budget(packet);
}

/// 把一页原文并入包的 `sourceEvidence.pages`（同页替换，不同页追加），并同步 `scope.pages`。
fn upsert_packet_page(packet: &mut Value, page: &Value) {
    let Some(index) = page.get("pageIndex").cloned() else {
        return;
    };
    let mut pages: Vec<Value> = packet
        .pointer("/sourceEvidence/pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut replaced = false;
    for existing in pages.iter_mut() {
        if existing.get("pageIndex") == Some(&index) {
            *existing = json!({"pageIndex": index, "lines": page.get("lines").cloned().unwrap_or_else(|| json!([]))});
            replaced = true;
            break;
        }
    }
    if !replaced {
        pages.push(json!({"pageIndex": index, "lines": page.get("lines").cloned().unwrap_or_else(|| json!([]))}));
    }
    pages.sort_by_key(|page| page.get("pageIndex").and_then(Value::as_u64).unwrap_or(0));
    packet["sourceEvidence"]["pages"] = json!(pages);

    if let Some(number) = index.as_u64() {
        let mut scope: Vec<u64> = packet
            .pointer("/scope/pages")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        if !scope.contains(&number) {
            scope.push(number);
            scope.sort_unstable();
            packet["scope"]["pages"] = json!(scope);
        }
    }
}

/// 升级一档。返回新的级别。
///
/// - **L1** 抓取工具 / `report_insufficient_context`（模型自己取）；
/// - **L2** 后端把本包范围内的页整页附上（模型不必再自己找）；
/// - **L3** 本包附一次整份原文件（`attachFullSource`，每次运行最多一次）；
/// - **L4** 不再升级，由调用方代记 `cannot_resolve`。
fn escalate_packet(
    request: &RepairRunRequest<'_>,
    source_index: &packets::SourcePageIndex,
    packet: &mut Value,
    level: u32,
    used_full_source: &mut bool,
) -> u32 {
    let next = (level + 1).min(PACKET_MAX_ESCALATION);
    match next {
        2 => {
            // 整页图：包内范围的所有页，缺哪页补哪页。
            let packet_id = packet
                .get("packetId")
                .and_then(Value::as_str)
                .unwrap_or("pkt")
                .to_string();
            let mut regions: Vec<Value> = packet
                .pointer("/sourceEvidence/regions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            // 「这一页已经有图了」必须按**真的带了图**算，不能只看有没有 region 条目：
            // `grab::materialize_regions` 在这一卷没有页图产物时会给每条 region 写
            // `"image": null`，而 `enforce_packet_budget` 只删**带图**的条目 —— null
            // 条目会原样留下。按 `pageIndex` 收集的话，`scope.pages ⊆ region 页` 时
            // `wanted` 会变成 0，note 就会写出「every page in scope already had an image
            // in this packet」——一张图都没有（P7 审计 #2 找到的 A-19 残留）。
            let existing: BTreeSet<u64> = regions
                .iter()
                .filter(|region| !region["image"].is_null())
                .filter_map(|region| region.get("pageIndex").and_then(Value::as_u64))
                .collect();
            let requests: Vec<Value> = packet
                .pointer("/scope/pages")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_u64).collect::<Vec<_>>())
                .unwrap_or_default()
                .into_iter()
                .filter(|page| !existing.contains(page))
                .map(|page| json!({"pageIndex": page, "bbox": Value::Null, "taskIds": [], "image": Value::Null}))
                .collect();
            let wanted = requests.len();
            let added = grab::materialize_regions(
                request.root,
                request.job_id,
                &packet_id,
                source_index,
                &requests,
            );
            // note 必须写**这一次实际附上了几张**。两个坑都要避开：
            // ① 拿不到页图（例如这一卷没有渲染产物）时照抄「已附整页图」，就是让模型
            //    以为自己看过那一块——静默丢图最危险的那一种；
            // ② 数**整包**带图的 region 会把计划期就裁好的区域图、以及模型自己取回的
            //    页图一并算进去，于是 L2 一张都没附也会报「已附 N 张」。
            // 所以只数 `added`（这一档新加的）里真的带上图的那些。
            let attached = added
                .iter()
                .filter(|region| !region["image"].is_null())
                .count();
            regions.extend(added);
            packet["sourceEvidence"]["regions"] = json!(regions);
            packet["scopeManifest"]["escalationNote"] = if wanted == 0 {
                json!(
                    "L2: every page in scope already had an image in this packet, so there was \
                     nothing more to attach."
                )
            } else if attached > 0 {
                json!(format!(
                    "L2: the backend attached whole-page images for {attached} page(s) in scope."
                ))
            } else {
                json!(
                    "L2: no page image was available for this source, so no whole-page image could be attached."
                )
            };
        }
        3 => {
            if !*used_full_source {
                *used_full_source = true;
                packet["attachFullSource"] = json!(true);
                packet["scopeManifest"]["escalationNote"] = json!(
                    "L3: the whole original file is attached to this packet (last resort, once per run)."
                );
            } else {
                packet["scopeManifest"]["escalationNote"] = json!(
                    "L3 is exhausted for this run: the whole original file was already attached to another packet, so this packet keeps its text scope only."
                );
            }
        }
        _ => {}
    }
    packet["escalationLevel"] = json!(next);
    apply_budget_after_escalation(packet);
    next
}

/// L4：对本包**仍然没有有效裁定**的差异，后端代记 `cannot_resolve`。
///
/// 为什么必须由后端代记：模型没做到这一步时（预算用尽、或它只是沉默），那些差异会
/// 原样落进用户清单而**没有任何说明**——用户看到的是「云端跑完了但没告诉我为什么」。
/// 代记之后每一条都带上理由码，清单里的文案明说「云端没能拿到足够的原文」。
fn force_context_insufficient_rulings(
    packet: &Value,
    rulings: &mut Vec<Value>,
    round: u32,
) -> usize {
    let numbers: Vec<Value> = packet
        .get("questionNumbers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let packet_id = packet.get("packetId").cloned().unwrap_or(Value::Null);
    let mut forced = 0usize;
    for difference in packet
        .get("differences")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if fresh_ruling_for_difference(rulings, difference).is_some() {
            continue;
        }
        let (target_type, target_id, field) = difference_key(difference);
        let (canonical_digest, candidate_digest, context_digest) =
            difference_digests(difference);
        rulings.push(json!({
            "targetType": target_type,
            "targetId": target_id,
            "field": field,
            "ruling": crate::schema::cloud_repair_v1::CLOUD_RULING_CANNOT_RESOLVE,
            "reason": crate::schema::cloud_repair_v1::CLOUD_RULING_REASON_CONTEXT_INSUFFICIENT,
            // 题号让用户清单能说清「第几题」，也让「上下文不足」这条记录可解释。
            "questionNumbers": numbers,
            "evidence": [],
            "canonicalDigest": canonical_digest,
            "candidateDigest": candidate_digest,
            "contextDigest": context_digest,
            "recordedAtRound": round,
            "packetId": packet_id,
            "recordedBy": "backend",
        }));
        forced += 1;
    }
    forced
}

#[cfg(test)]
mod tests;
