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
            "adjudicatedCount": 0,
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
        "field": field,
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

/// 一条差异两侧的内容指纹 `(当前稿一侧, 候选一侧)`。
fn difference_digests(difference: &Value) -> (String, String) {
    (
        canonical_json(difference.get("canonical").unwrap_or(&Value::Null)),
        canonical_json(difference.get("candidate").unwrap_or(&Value::Null)),
    )
}

/// 从裁定记录里找出**对这条差异仍然有效**的那一条。
///
/// 有效性 = 身份一致 **且** 两侧内容指纹都没变。任一侧变了，裁定当时的前提就不存在了：
/// 一次基于旧内容的「我确认当前稿是对的」不能永久掩盖后来才出现的问题。
///
/// 同一条差异有多份裁定时取**最后**一份（最新的判断覆盖旧的）。
fn fresh_ruling_for_difference<'a>(rulings: &'a [Value], difference: &Value) -> Option<&'a Value> {
    let (target_type, target_id, field) = difference_key(difference);
    let (canonical_digest, candidate_digest) = difference_digests(difference);
    rulings.iter().rev().find(|ruling| {
        ruling.get("targetType").and_then(Value::as_str) == Some(target_type.as_str())
            && ruling.get("targetId").and_then(Value::as_str) == Some(target_id.as_str())
            && ruling.get("field").and_then(Value::as_str) == Some(field.as_str())
            && ruling.get("canonicalDigest").and_then(Value::as_str)
                == Some(canonical_digest.as_str())
            && ruling.get("candidateDigest").and_then(Value::as_str)
                == Some(candidate_digest.as_str())
    })
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
        _ => "内容",
    };
    let where_ = match target_type.as_str() {
        "slot" => format!("第 {target_id} 题"),
        "response_group" => format!("题组内的作答区（{target_id}）"),
        "task_group" => format!("题组 {target_id}"),
        other => format!("{other} {target_id}"),
    };
    format!("{where_}的{label}与云端识别结果不一致")
}

/// 机械比对：当前 canonical 与云端完整候选之间**还剩哪些实质差异**。
///
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
        let pages: Vec<Value> = evidence
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
        return Ok(json!({
            "kind": "pdf",
            "sourceFileId": evidence.get("sourceFileId").cloned().unwrap_or(Value::Null),
            "note": "The original PDF is attached to this conversation; the page text below is the extracted text layer.",
            "pages": pages,
        }));
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

/// 执行一次允许的工具调用，返回**真实**结果。
fn execute_tool(
    request: &RepairRunRequest<'_>,
    call: &CloudRepairToolCallV1,
    round: u32,
    context: &Value,
) -> (CloudRepairToolResultV1, Option<usize>) {
    match call.tool.as_str() {
        "read_draft" => {
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
        "read_source" => match read_source_evidence(request.root, request.job_id, &call.arguments) {
            Ok(value) => (CloudRepairToolResultV1::ok(&call.call_id, value), None),
            Err(error) => (CloudRepairToolResultV1::rejected(&call.call_id, vec![error]), None),
        },
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
            match tools::apply_cloud_edits(request.root, &edit_request) {
                Ok(outcome) => {
                    let applied = matches!(outcome.status, tools::CloudEditStatus::Applied);
                    let applied_count = outcome.applied_count;
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
            for entry in entries {
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
                let (canonical_digest, candidate_digest) = difference_digests(difference);
                recorded.push(json!({
                    "targetType": target_type,
                    "targetId": target_id,
                    "field": field,
                    "ruling": ruling,
                    "reason": entry.get("reason").cloned().unwrap_or(Value::Null),
                    "evidence": entry.get("evidence").cloned().unwrap_or_else(|| json!([])),
                    // 绑定裁定当时看到的这一对内容；任一侧后来变了，这条裁定作废重评。
                    "canonicalDigest": canonical_digest,
                    "candidateDigest": candidate_digest,
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
                        "noteForModel": "Recorded rulings remove adjudicated differences from the user's list. \
                                         They cannot remove structural problems found by the backend validator.",
                    }),
                ),
                None,
            )
        }
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
    let mut tasks = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    // ── 1) 当前稿的真实质量问题 ────────────────────────────────────────────
    //
    // 判据与发布门禁**同一份**（`authoring_v2_commands::blocking_issue_unresolved`）。
    // 这里曾内联同一段谓词，一旦发布门禁改了判据、这里没跟上，就会出现「修复循环说
    // 还剩问题、预检却说能发布」的同稿不同判——用户被留在两套说法中间。
    //
    // 这一路**不看裁定**：模型不能通过 `record_ruling` 消除结构错误。
    for issue in crate::authoring_v2_commands::unresolved_blocking_issues(&canonical) {
        let target_id = issue
            .get("targetId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let task_id = format!(
            "quality:{}:{}",
            issue.get("code").and_then(Value::as_str).unwrap_or(""),
            target_id
        );
        if !seen.insert(task_id.clone()) {
            continue;
        }
        tasks.push(json!({
            "userTaskId": task_id,
            "targetIds": [target_id],
            "questionNumbers": [],
            "message": issue.get("message").cloned().unwrap_or(Value::Null),
            "action": "fix_blocking_issue",
            "blocking": true,
        }));
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
                        if !seen.insert(task_id.clone()) {
                            continue;
                        }
                        tasks.push(json!({
                            "userTaskId": task_id,
                            "targetIds": [target_id],
                            "message": format!(
                                "{}；云端已查过原文件但无法定论：{}",
                                describe_difference(&difference),
                                ruling.get("reason").and_then(Value::as_str).unwrap_or("未说明理由")
                            ),
                            "action": "review_difference",
                            "blocking": false,
                            "evidence": ruling.get("evidence").cloned().unwrap_or_else(|| json!([])),
                        }));
                    }
                    // 没裁定过，或裁定已被内容变化作废：这才是真正需要用户看的差异。
                    None => {
                        if !seen.insert(task_id.clone()) {
                            continue;
                        }
                        tasks.push(json!({
                            "userTaskId": task_id,
                            "targetIds": [target_id],
                            "message": describe_difference(&difference),
                            "action": "review_difference",
                            "blocking": false,
                        }));
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
        if !seen.insert(task_id.clone()) {
            continue;
        }
        let message = question
            .get("message")
            .or_else(|| question.get("question"))
            .or_else(|| question.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("云端在校核时留下了未能确认的疑问")
            .to_string();
        tasks.push(json!({
            "userTaskId": task_id,
            "targetIds": if target_id.is_empty() { json!([]) } else { json!([target_id]) },
            "message": format!("云端未能确认：{message}"),
            // 有具体目标就定位到题面，没有就是文档级疑问，打开原文件抽屉核对。
            "action": if target_id.is_empty() { "review_source" } else { "review_difference" },
            "blocking": false,
            "evidence": question.get("evidence").cloned().unwrap_or_else(|| json!([])),
        }));
    }

    // ── 4) 来源覆盖缺口 ───────────────────────────────────────────────────
    //
    // 与第 1 路的 `SIGNIFICANT_REGION_UNASSIGNED` 互补而非重复：那一条说的是「当前稿
    // 有源区域没被任何题组接住」（稿的问题），这一路说的是「云端读不到原文件的这一块」
    // （证据的问题）。云端没读到的地方，用户有权知道——否则他会以为整份文件都核过了。
    if let Some(candidate) = candidate.as_ref() {
        for (index, region) in candidate.unresolved_regions.iter().enumerate() {
            let task_id = format!("cloud-coverage:{}:{}:{index}", region.source_file_id, region.page_index);
            if !seen.insert(task_id.clone()) {
                continue;
            }
            tasks.push(json!({
                "userTaskId": task_id,
                "targetIds": [],
                "message": format!(
                    "原文件第 {} 页云端未能读全（{}）：{}",
                    region.page_index, region.reason, region.detail
                ),
                "action": "review_source",
                "blocking": false,
            }));
        }
        for (index, note) in candidate.source_coverage_notes.iter().enumerate() {
            let task_id = format!("cloud-coverage-note:{index}");
            if !seen.insert(task_id.clone()) {
                continue;
            }
            tasks.push(json!({
                "userTaskId": task_id,
                "targetIds": [],
                "message": note.clone(),
                "action": "review_source",
                "blocking": false,
            }));
        }
    }

    Ok(tasks)
}

/// 修复循环的编排。
///
/// `step` 是**注入的**网关调用：`(context, observations) -> 模型原始 JSON`。
/// 生产实现走真实网关（并附带原文件证据）；测试注入确定性桩。
///
/// 有限轮次 + 总超时 + 取消 + 运行归属：模型永远跑不完也没关系——循环退出后
/// 一律由后端按当前 canonical 重算剩余问题。
pub(crate) fn run_repair_loop<F>(
    request: &RepairRunRequest<'_>,
    mut step: F,
) -> CommandResult<RepairRunReport>
where
    F: FnMut(&Value, &[Value]) -> CommandResult<Value>,
{
    let mut context = build_repair_context(request.root, request.item_id, request.job_id, request.batch_id)?;
    let mut observations: Vec<Value> = Vec::new();
    let mut applied_count = 0usize;
    let mut rounds = 0u32;
    let mut finish_note: Option<String> = None;
    let mut last_error: Option<String> = None;
    let mut status = REPAIR_STATUS_COMPLETED;
    let mut repeats: BTreeMap<String, u32> = BTreeMap::new();
    // 已落盘的裁定先读回来：重试 / 重启后再跑一次修复，不该让用户第二次回答同一个问题。
    // 读回来的裁定是否仍然有效由**内容指纹**决定（见 `fresh_ruling_for_difference`），
    // 所以这里不需要额外判断「是不是同一轮」。
    let mut rulings: Vec<Value> = store::read_repair_rulings(request.root, request.job_id, request.batch_id)?
        .and_then(|value| value.get("rulings").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let mut model_questions: Vec<Value> = Vec::new();

    // 开工就先落一次 `running`：修复循环最长十分钟，用户在它结束之前就该能看到
    // 「云端正在自动修复」，而不是对着上一次的旧状态猜。
    report_progress(
        request,
        RepairProgress {
            status: REPAIR_STATUS_RUNNING,
            round: 0,
            applied_count: 0,
            edit_version: current_canonical(request)?
                .map(|(_, version)| version)
                .unwrap_or(0),
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
                observations.push(json!({
                    "schemaVersion": "CloudRepairToolResultV1",
                    "callId": raw.get("callId").cloned().unwrap_or(Value::Null),
                    "status": "rejected",
                    "errors": [error],
                }));
                last_error = Some(error);
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
        let (result, applied) = execute_tool(request, &call, rounds, &context);
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
            context = build_repair_context(request.root, request.item_id, request.job_id, request.batch_id)?;
            repeats.clear();
            // 每落下一批有效修改立刻上报：调用方据此把新版本与已修数量写进产品状态、
            // 并发出事件让画布跟上。**不等循环结束**——否则用户看到的是「改了但界面没变」。
            report_progress(
                request,
                RepairProgress {
                    status: REPAIR_STATUS_RUNNING,
                    round: rounds,
                    applied_count,
                    edit_version: context
                        .get("editVersion")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                },
            );
        }
        observations.push(serde_json::to_value(&result).unwrap_or(Value::Null));
        if is_finish {
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
    if rounds >= request.max_rounds && status == REPAIR_STATUS_COMPLETED && finish_note.is_none() {
        status = REPAIR_STATUS_BUDGET_EXHAUSTED;
    }

    // 裁定落盘。**即使这一轮没跑完也要写**：模型已经作出的判断是用户不必再回答的东西，
    // 不能因为预算耗尽就把它们一起丢掉。
    store::write_repair_rulings(
        request.root,
        request.job_id,
        request.batch_id,
        &json!({ "rulings": rulings }),
    )?;

    // 最终完成状态由**后端**判定：模型说"都修好了"不算数。
    let remaining = remaining_tasks(
        request.root,
        request.item_id,
        request.job_id,
        request.batch_id,
        &rulings,
        &model_questions,
    )?;
    if status == REPAIR_STATUS_COMPLETED && !remaining.is_empty() {
        status = REPAIR_STATUS_NEEDS_ATTENTION;
    }
    let edit_version = current_canonical(request)?
        .map(|(_, version)| version)
        .unwrap_or(0);

    Ok(RepairRunReport {
        status,
        rounds,
        edit_version,
        applied_count,
        observations,
        remaining_tasks: remaining,
        adjudicated_count: rulings.len(),
        finish_note,
        last_error,
        repair_run_id: request.repair_run_id.to_string(),
    })
}

#[cfg(test)]
mod tests;
