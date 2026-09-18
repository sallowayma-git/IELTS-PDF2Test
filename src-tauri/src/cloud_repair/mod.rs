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
    pub finish_note: Option<String>,
    pub last_error: Option<String>,
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
            "finishNote": self.finish_note,
            "lastError": self.last_error,
            "undoAvailable": undo_available,
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
    _context: &Value,
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
fn remaining_tasks(
    root: &Path,
    item_id: &str,
    job_id: &str,
    batch_id: &str,
) -> CommandResult<Vec<Value>> {
    let conn = open_library_connection(root)?;
    let Some((canonical, _)) = get_canonical_ds(&conn, item_id)? else {
        return Ok(Vec::new());
    };
    drop(conn);

    let candidate = store::read_cloud_authoring_candidate(root, job_id, batch_id)?;
    let mut tasks = Vec::new();

    if let Some(candidate) = candidate.as_ref() {
        if let Ok(candidate_value) = serde_json::to_value(&candidate.authoring) {
            for difference in candidate_differences(&canonical, &candidate_value) {
                let target_id = difference
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let field = difference
                    .get("field")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                tasks.push(json!({
                    "userTaskId": format!("cloud-diff:{target_id}:{field}"),
                    "targetIds": [target_id],
                    "message": format!("云端识别与原稿在 {field} 上仍有差异，自动修复未能解决"),
                    "action": "review_difference",
                    "blocking": false,
                }));
            }
        }
    }

    // 当前稿上仍未处理的阻断性问题：**判据与发布门禁同一份**
    // （`authoring_v2_commands::blocking_issue_unresolved`）。这里曾内联同一段谓词，
    // 一旦发布门禁改了判据、这里没跟上，就会出现「修复循环说还剩问题、预检却说能发布」
    // 的同稿不同判——用户被留在两套说法中间。
    //
    // 也不能用「校验器没报错」来消除内容疑问：`unresolved` 只统计 blocking issue，
    // 模型自己报的未解疑问留在 `candidate_differences` 那一支里，两者都要保留。
    for issue in crate::authoring_v2_commands::unresolved_blocking_issues(&canonical) {
        let target_id = issue
            .get("targetId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        tasks.push(json!({
            "userTaskId": format!("quality:{}:{}", issue.get("code").and_then(Value::as_str).unwrap_or(""), target_id),
            "targetIds": [target_id],
            "questionNumbers": [],
            "message": issue.get("message").cloned().unwrap_or(Value::Null),
            "action": "fix_blocking_issue",
            "blocking": true,
        }));
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
        if let Some(count) = applied {
            applied_count += count;
            // 写成功之后必须重读上下文：版本变了，模型手里的 baseVersion 已过期。
            context = build_repair_context(request.root, request.item_id, request.job_id, request.batch_id)?;
            repeats.clear();
        }
        observations.push(serde_json::to_value(&result).unwrap_or(Value::Null));
        if is_finish {
            finish_note = call
                .arguments
                .get("note")
                .and_then(Value::as_str)
                .map(str::to_string);
            break;
        }
    }
    if rounds >= request.max_rounds && status == REPAIR_STATUS_COMPLETED && finish_note.is_none() {
        status = REPAIR_STATUS_BUDGET_EXHAUSTED;
    }

    // 最终完成状态由**后端**判定：模型说"都修好了"不算数。
    let remaining = remaining_tasks(request.root, request.item_id, request.job_id, request.batch_id)?;
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
        finish_note,
        last_error,
    })
}

#[cfg(test)]
mod tests;
