//! 把本地稿与云端原始输出统一到同一套 V2 语义候选视图。
//!
//! - 本地链：从 `IeltsAuthoringIRV2` 值抽取（不复制权威，只读）。
//! - 云端链：normalize → schema validate → 分组 salvage。**禁止**用默认值补齐
//!   业务字段：不合法的题组被丢弃并记入 salvage 报告，缺字段的答案保持 `None`。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::ielts_grammar::quality::derive_instruction_signature_for_group;
use crate::schema::cloud_repair_v1::{
    CloudAuthoringCandidateV1, CloudCandidateUnresolvedRegionV1,
    CLOUD_AUTHORING_CANDIDATE_V1_SCHEMA_VERSION,
};
use crate::schema::ielts_authoring_v2::{
    AnswerSlotHostTypeV2, AnswerSlotParticipationV2, AnswerValueV2, InteractionV2, ResponseGroupKindV2,
    TaskTypeV2,
};
use crate::CommandResult;
use crate::schema::recognition_v1::{
    reason, CandidateAssetV1, CandidateOptionBankV1, CandidateOptionV1, CandidateResponseGroupV1,
    CandidateSlotV1, CandidateTaskGroupV1, CandidateUnresolvedRegionV1, ChainKindV1, ChainStatusV1,
    RecognitionCandidateV1, SalvageReportV1, RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION,
};

/// 归一化文本用于比对：折叠空白 + 小写。保留标点（题干标点有语义）。
pub(crate) fn normalize_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// 递归拼接 `ContentNodeV2` 中的文本（`text` 节点），忽略结构节点。
pub(crate) fn nodes_text(nodes: &Value) -> String {
    let mut out = String::new();
    collect_text(nodes, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_text(value: &Value, out: &mut String) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_text(item, out);
            }
        }
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(text);
                }
                return;
            }
            if let Some(children) = map.get("children") {
                collect_text(children, out);
            }
        }
        _ => {}
    }
}

/// 展开 `QuestionNumberExpressionV2` 为题号列表（升序去重）。
pub(crate) fn expand_question_numbers(expression: &Value) -> Vec<u32> {
    let mut numbers: Vec<u32> = Vec::new();
    match expression.get("kind").and_then(Value::as_str) {
        Some("range") => {
            let start = expression.get("start").and_then(Value::as_u64).unwrap_or(0) as u32;
            let end = expression.get("end").and_then(Value::as_u64).unwrap_or(0) as u32;
            for number in start..=end {
                numbers.push(number);
            }
        }
        Some("set") => {
            if let Some(values) = expression.get("values").and_then(Value::as_array) {
                for value in values {
                    if let Some(number) = value.as_u64() {
                        numbers.push(number as u32);
                    }
                }
            }
        }
        Some("mixed") => {
            if let Some(values) = expression.get("values").and_then(Value::as_array) {
                for value in values {
                    if let Some(number) = value.as_u64() {
                        numbers.push(number as u32);
                    } else {
                        numbers.extend(expand_question_numbers(value));
                    }
                }
            }
        }
        _ => {}
    }
    numbers.sort_unstable();
    numbers.dedup();
    numbers
}

/// 自然形态 `kind` → `TaskTypeV2` 的 snake_case 值（见 `ielts_authoring_v2.rs:201`）。
/// 未知题型返回 `None`，由调用方保持原 group 不变，让解析器产出稳定原因码（fail-closed）。
fn cloud_kind_to_task_type(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "single_choice" => "single_choice",
        "multi_choice" => "multiple_choice",
        "true_false_not_given" => "true_false_not_given",
        "yes_no_not_given" => "yes_no_not_given",
        "matching" => "matching_information",
        "heading_matching" => "matching_headings",
        "matching_information" => "matching_information",
        "classification" => "classification",
        "summary_completion" => "summary_completion",
        "note_completion" | "notes_completion" => "summary_completion",
        "table_completion" => "table_completion",
        "diagram_completion" => "diagram_label_completion",
        "plan_map_label_completion" => "plan_map_label_completion",
        "short_answer" => "short_answer",
        "sentence_completion" => "sentence_completion",
        _ => return None,
    })
}

/// 由 `TaskTypeV2` snake_case 派生 `(interaction, hostType)`（见 `ielts_authoring_v2.rs:427`）。
fn interaction_host_for(task_type: &str) -> (&'static str, &'static str) {
    match task_type {
        "single_choice"
        | "multiple_choice"
        | "true_false_not_given"
        | "yes_no_not_given"
        | "classification"
        | "matching_information"
        | "matching_headings"
        | "matching_features"
        | "matching_sentence_endings" => ("radio", "prompt"),
        "summary_completion"
        | "note_completion"
        | "sentence_completion"
        | "short_answer"
        | "form_completion"
        | "flowchart_completion" => ("text", "prompt"),
        "table_completion" => ("text", "table_cell"),
        "diagram_label_completion" | "plan_map_label_completion" => ("dragdrop", "figure_hotspot"),
        _ => ("text", "prompt"),
    }
}

/// 由 `TaskTypeV2` snake_case 派生 `ResponseGroupKindV2`（见 `ielts_authoring_v2.rs:373`）。
fn response_group_kind_for(task_type: &str) -> &'static str {
    match task_type {
        "single_choice" | "multiple_choice" | "true_false_not_given" | "yes_no_not_given" | "classification" => {
            "choice"
        }
        "matching_information" | "matching_headings" => "matching",
        "diagram_label_completion" | "plan_map_label_completion" => "diagram_hotspot",
        _ => "text_entry",
    }
}

/// 题号推导：优先用 `range` 数组 `[start,end]`（start>0）；否则退回解析 `questionIds`
/// 中的数字（剥离前导 `q`）。保证与后续 `expand_question_numbers` 一致。
fn derive_question_numbers(range: &Value, question_ids: &Value) -> Vec<u32> {
    if let Some(arr) = range.as_array() {
        if arr.len() == 2 {
            let start = arr[0].as_u64().filter(|value| *value > 0).unwrap_or(0);
            let end = arr[1].as_u64().unwrap_or(0);
            if start > 0 && end >= start {
                return (start..=end).map(|value| value as u32).collect();
            }
        }
    }
    if let Some(ids) = question_ids.as_array() {
        let mut nums: Vec<u32> = Vec::new();
        for id in ids {
            let parsed = if let Some(s) = id.as_str() {
                let trimmed = s.trim();
                trimmed
                    .strip_prefix('q')
                    .and_then(|rest| rest.parse::<u32>().ok())
                    .or_else(|| trimmed.parse::<u32>().ok())
            } else {
                id.as_u64().map(|value| value as u32)
            };
            if let Some(number) = parsed {
                nums.push(number);
            }
        }
        nums.sort_unstable();
        nums.dedup();
        return nums;
    }
    Vec::new()
}

fn is_contiguous(numbers: &[u32]) -> bool {
    if numbers.len() <= 1 {
        return !numbers.is_empty();
    }
    let mut prev = numbers[0];
    for &number in &numbers[1..] {
        if number != prev + 1 {
            return false;
        }
        prev = number;
    }
    true
}

/// 把 group 的 `evidence.quotes` 映射成 slot 级 `evidence`（`{pageIndex, quote}`）。
fn map_group_quotes(group: &Map<String, Value>) -> Value {
    let quotes = group
        .get("evidence")
        .and_then(Value::as_object)
        .and_then(|evidence| evidence.get("quotes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mapped: Vec<Value> = quotes
        .iter()
        .filter_map(|quote| {
            let page = quote.get("pageIndex").and_then(Value::as_u64)?;
            let text = quote.get("text").and_then(Value::as_str)?;
            Some(json!({"pageIndex": page, "quote": text}))
        })
        .collect();
    json!(mapped)
}

/// 把任意答案值归一成 `AnswerValueV2` 形态；不猜测 `option` vs `text`。
/// - 已是 `text`/`option`/`unresolved` 对象 → 原样透传（不伪造）。
/// - 字符串 → `{"kind":"text","values":[s],"normalization":"ielts_default"}`。
/// - 字符串数组 → 同上，全部值。
/// - 其余 → `None`（交由解析器按缺答案处理）。
fn to_answer_value(value: &Value) -> Option<Value> {
    if let Some(object) = value.as_object() {
        if let Some(kind) = object.get("kind").and_then(Value::as_str) {
            if matches!(kind, "text" | "option" | "unresolved") {
                return Some(value.clone());
            }
        }
    }
    match value {
        Value::String(s) => Some(json!({"kind":"text","values":[s],"normalization":"ielts_default"})),
        Value::Array(arr) => {
            let values: Vec<String> = arr.iter().filter_map(Value::as_str).map(str::to_string).collect();
            if values.is_empty() {
                None
            } else {
                Some(json!({"kind":"text","values":values,"normalization":"ielts_default"}))
            }
        }
        _ => None,
    }
}

/// 把「自然形态」云端输出确定性适配为 `cloud_candidate_from_value` 期望的**内部形态**。
///
/// 设计约束（fail-closed）：
/// - 若任一 group 同时带 `taskId` 与 `taskType`，视为已是内部形态，**原样透传**
///   （既有内部形态测试零改动）。
/// - 仅做标识符 / 枚举派生，**绝不伪造任何业务内容**：答案、题干、选项文本均原样搬运；
///   缺字段则保持缺失，让既有 fail-closed 逻辑按原因码丢弃该组。
/// - 未知 / 缺失 `kind` 的 group 保持原样，由解析器产生其稳定原因码（不静默成功）。
pub(crate) fn expand_natural_cloud_shape(raw: &Value) -> Value {
    let groups = raw.get("groups").and_then(Value::as_array).cloned();
    let Some(groups) = groups else {
        return raw.clone();
    };
    // 内部形态探测：任一 group 同时带 taskId 与 taskType → 直接透传。
    if groups.iter().any(|group| {
        let Some(object) = group.as_object() else { return false };
        object.contains_key("taskId") && object.contains_key("taskType")
    }) {
        return raw.clone();
    }

    let answer_key = raw.get("answerKey").cloned().unwrap_or_else(|| json!({}));
    let mut new_groups = Vec::with_capacity(groups.len());
    for (index, group) in groups.iter().enumerate() {
        let Some(object) = group.as_object() else {
            new_groups.push(group.clone());
            continue;
        };
        let kind = object.get("kind").and_then(Value::as_str).unwrap_or("");
        let Some(task_type) = cloud_kind_to_task_type(kind) else {
            // 未知题型：保持原 group 不变，解析器按原因码丢弃（fail-closed）。
            new_groups.push(group.clone());
            continue;
        };
        let range_value = object.get("range").cloned().unwrap_or(Value::Null);
        let question_ids = object.get("questionIds").cloned().unwrap_or_else(|| json!([]));
        let numbers = derive_question_numbers(&range_value, &question_ids);
        if numbers.is_empty() {
            new_groups.push(group.clone());
            continue;
        }
        let task_id = format!("cloud-task-{}", index + 1);
        let response_group_id = format!("cloud-rg-{}", index + 1);
        let (interaction, host_type) = interaction_host_for(task_type);
        let response_group_kind = response_group_kind_for(task_type);
        let group_answers = object.get("answers").cloned().unwrap_or_else(|| json!({}));
        let model_slots = object.get("slots").and_then(Value::as_array).cloned().unwrap_or_default();

        let mut slot_ids: Vec<String> = Vec::with_capacity(numbers.len());
        let mut new_slots: Vec<Value> = Vec::with_capacity(numbers.len());
        for &number in &numbers {
            let slot_id = format!("cloud-q{}", number);
            slot_ids.push(slot_id.clone());
            let model_slot = model_slots
                .iter()
                .find(|slot| slot.get("questionNumber").and_then(Value::as_u64) == Some(number as u64));
            let raw_answer: Option<Value> = model_slot
                .and_then(|slot| slot.get("answer").cloned())
                .filter(|value| !value.is_null())
                .or_else(|| group_answers.get(number.to_string()).cloned().filter(|value| !value.is_null()))
                .or_else(|| group_answers.get(format!("q{}", number)).cloned().filter(|value| !value.is_null()))
                .or_else(|| answer_key.get(number.to_string()).cloned().filter(|value| !value.is_null()))
                .or_else(|| answer_key.get(format!("q{}", number)).cloned().filter(|value| !value.is_null()));
            let answer = raw_answer.as_ref().and_then(to_answer_value);
            let evidence = model_slot
                .and_then(|slot| slot.get("evidence"))
                .filter(|value| Value::is_array(*value))
                .cloned()
                .unwrap_or_else(|| map_group_quotes(object));
            new_slots.push(json!({
                "slotId": slot_id,
                "questionNumber": number,
                "displayLabel": number.to_string(),
                "taskId": task_id,
                "responseGroupId": response_group_id,
                "interaction": interaction,
                "hostType": host_type,
                "participation": "scoring",
                "answer": answer,
                "evidence": evidence,
            }));
        }

        let range_internal = if is_contiguous(&numbers) {
            json!({"kind":"range","start":numbers[0],"end":*numbers.last().unwrap()})
        } else {
            json!({"kind":"set","values":numbers})
        };

        let mut new_group = object.clone();
        new_group.insert("taskId".to_string(), json!(task_id));
        new_group.insert("taskType".to_string(), json!(task_type));
        new_group.insert("range".to_string(), range_internal);
        new_group.insert(
            "responseGroups".to_string(),
            json!([{
                "responseGroupId": response_group_id,
                "kind": response_group_kind,
                "slotIds": slot_ids,
            }]),
        );
        new_group.entry("instructionsText".to_string()).or_insert_with(|| json!(""));
        if !new_group.contains_key("stimulusText") {
            if let Some(notes) = object.get("notesText") {
                new_group.insert("stimulusText".to_string(), notes.clone());
            }
        }
        new_group.insert("slots".to_string(), json!(new_slots));
        new_groups.push(Value::Object(new_group));
    }

    let mut out = raw.clone();
    if let Some(object) = out.as_object_mut() {
        object.insert("groups".to_string(), json!(new_groups));
    }
    out
}

/// 云端答案归一：基于**交互语义**与**合法映射**，而非按本地 `kind` 强制改写。
///
/// `answer_compare_key`（`rules.rs:85`）对 `text` / `option` 给出不同键，于是同一答案
/// 因 authoring 约定不同（本地 `option`、云端 `text`）被误判为实质分歧。但「按本地
/// `kind` 重新编码云端」会**伪造一致或抹平真分歧**：例如选项任务里云端报的是选项
/// *文本*、本地报的是 *label*，强制把云端文本塞进 `labels[]` 要么凭空造出一个匹配，
/// 要么掩盖了二者其实不同。
///
/// 新语义：只有当存在**合法映射**时才改写云端，且改写后的值必须是云端原值经映射得到
/// 的等价表示（保留云端自己的值）：
///   1. 大小写 / 空白规范化；等价 label 拼法（如 `b`↔`B`）视为同一 label。
///   2. `label ↔ 选项文本` 经**选项库**解析（选项库在 `local.group(task_id).option_bank`）。
///   3. 无合法映射（如：选项任务、云端报的是选项文本、本地却无选项库可解析）→ **保留
///      云端原值**，由裁决层暴露真实分歧，绝不强行 unification。
///
/// 仅有当：本地该题有答案、云端答案能抽出字符串、且二者 `kind` 不同时才尝试改写；
/// 否则原样保留（不引入假一致）。
pub(crate) fn align_cloud_answer_shapes(cloud: &mut RecognitionCandidateV1, local: &RecognitionCandidateV1) {
    for cloud_slot in &mut cloud.slots {
        let Some(local_slot) = local.slot_by_question(cloud_slot.question_number) else {
            continue;
        };
        let Some(local_answer) = &local_slot.answer else {
            continue;
        };
        let Some(cloud_answer) = &cloud_slot.answer else {
            continue;
        };
        // 选项库在本地题组上，按 slot 的 task_id 取；对齐点即可达（无需上层额外传递）。
        let bank = local
            .group(&local_slot.task_id)
            .and_then(|group| group.option_bank.as_ref());
        // 无合法映射 → 保留云端原值（其原始 kind/值），交由裁决层暴露真实分歧。
        if let Some(answer) = align_answer_value(cloud_answer, local_answer, bank) {
            cloud_slot.answer = Some(answer);
        }
    }
}

/// **形状对齐内核**：把 `value` 按 `target_shape` 的形状重编码；无法合法映射时返回 `None`。
///
/// 这是「按目标形状重编码答案」的**唯一实现**。任何新增的答案来源（云端候选、A3 的
/// `observedValue`、A4 的 `rulings[].value`）都必须调用它，**不允许各自再写一份**：
/// `answer_compare_key` 是形状敏感的（`text` → `values.join("|")`，
/// `option` → `opt:LABELS:assignment`），形状口径一旦分叉，就会在**每一道题**上制造假分歧。
///
/// 返回 `None` 的两种情形都表示「**不要改写**」，调用方必须原样保留输入值：
/// 1. 形状已经一致（两侧比较键都会做规范化，改写只会引入假一致）；
/// 2. 不存在合法映射（如选项任务里云端报的是选项文本、而本地没有可解析的选项库）
///    —— 此时必须让真实分歧上浮，**绝不**强行 unification。
pub(crate) fn align_answer_value(
    value: &Value,
    target_shape: &Value,
    bank: Option<&CandidateOptionBankV1>,
) -> Option<Value> {
    let value_kind = value.get("kind").and_then(Value::as_str)?;
    let target_kind = target_shape.get("kind").and_then(Value::as_str)?;
    if value_kind == target_kind {
        return None;
    }
    match (target_kind, value_kind) {
        ("option", "text") => align_text_to_option(value, target_shape, bank),
        ("text", "option") => align_option_to_text(value, target_shape, bank),
        _ => None,
    }
}

/// 抽出答案里的字符串（兼容 `text`/`option` 两种形状，以及裸字符串）。
///
/// 供 source.rs 共用：模型核验给出的 `observedValue` 也要先化成字符串，
/// 再统一按本地形状重编码，避免两处各写一套答案形状解析。
pub(crate) fn answer_strings(answer: &Value) -> Vec<String> {
    match answer.get("kind").and_then(Value::as_str) {
        Some("text") => answer
            .get("values")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
        Some("option") => answer
            .get("labels")
            .and_then(Value::as_array)
            .map(|labels| labels.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
        _ => match answer {
            Value::String(s) => vec![s.clone()],
            _ => Vec::new(),
        },
    }
}

/// 本地 `option` + 云端 `text`：把云端文本按合法映射改写为 option label 形式。
///
/// 返回 `Some` 表示已建立合法映射（改写），`None` 表示无法合法映射（保留原值）。
fn align_text_to_option(
    cloud_answer: &Value,
    local_answer: &Value,
    bank: Option<&CandidateOptionBankV1>,
) -> Option<Value> {
    let local_labels: Vec<String> = local_answer
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(Value::as_str)
                .map(|label| label.trim().to_uppercase())
                .collect()
        })
        .unwrap_or_default();
    let cloud_strings = answer_strings(cloud_answer);
    if cloud_strings.is_empty() {
        return None;
    }
    // label ↔ 文本 双向映射（仅来自选项库；无 bank 时只有 label 自映射）。
    let label_to_text: BTreeMap<String, String> = bank
        .map(|bank| {
            bank.options
                .iter()
                .map(|option| (option.label.trim().to_uppercase(), normalize_text(&option.text)))
                .collect()
        })
        .unwrap_or_default();
    let text_to_label: BTreeMap<String, String> =
        label_to_text.iter().map(|(label, text)| (text.clone(), label.clone())).collect();
    let assignment = local_answer
        .get("assignment")
        .and_then(Value::as_str)
        .unwrap_or("per_slot")
        .to_string();

    let mut mapped_labels: Vec<String> = Vec::with_capacity(cloud_strings.len());
    for string in &cloud_strings {
        let normalized = normalize_text(string);
        let upper = string.trim().to_uppercase();
        // 1) 直接匹配本地 label（等价 label 拼法 / 大小写）→ 保留云端原字符串。
        if local_labels.iter().any(|label| label == &upper) {
            mapped_labels.push(string.clone());
            continue;
        }
        // 2) 经选项库把「选项文本」解析为 label（大小写/空白已规范化）。
        if let Some(label) = text_to_label.get(&normalized).or_else(|| text_to_label.get(&upper)) {
            mapped_labels.push(label.clone());
            continue;
        }
        // 3) 无法建立合法映射（如：云端报的是选项文本但无 bank 可解析）→ 保留原值。
        return None;
    }
    Some(json!({"kind": "option", "labels": mapped_labels, "assignment": assignment}))
}

/// 本地 `text` + 云端 `option`：把云端 label 经选项库改写为文本形式。
///
/// 仅当选项库能把每个 label 解析成文本时才改写；否则保留云端原值（真实分歧上浮）。
fn align_option_to_text(
    cloud_answer: &Value,
    _local_answer: &Value,
    bank: Option<&CandidateOptionBankV1>,
) -> Option<Value> {
    let cloud_labels = answer_strings(cloud_answer);
    if cloud_labels.is_empty() {
        return None;
    }
    let label_to_text: BTreeMap<String, String> = bank.map(|bank| {
        bank.options
            .iter()
            .map(|option| (option.label.trim().to_uppercase(), normalize_text(&option.text)))
            .collect()
    })?;
    let mut mapped_texts: Vec<String> = Vec::with_capacity(cloud_labels.len());
    for label in &cloud_labels {
        match label_to_text.get(&label.trim().to_uppercase()) {
            Some(text) => mapped_texts.push(text.clone()),
            // 无 bank 或 bank 中没有该 label → 无法合法映射到文本，保留原值。
            None => return None,
        }
    }
    Some(json!({"kind": "text", "values": mapped_texts, "normalization": "ielts_default"}))
}

fn options_from_value(array: Option<&Value>) -> Vec<CandidateOptionV1> {
    let mut options = Vec::new();
    let Some(items) = array.and_then(Value::as_array) else {
        return options;
    };
    for item in items {
        let Some(label) = item.get("label").and_then(Value::as_str) else {
            continue;
        };
        let text = item
            .get("content")
            .map(nodes_text)
            .or_else(|| item.get("text").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let option_id = item
            .get("optionId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("opt-{label}"));
        options.push(CandidateOptionV1 {
            option_id,
            label: label.to_string(),
            text: text.trim().to_string(),
        });
    }
    options
}

// ── 本地链 ─────────────────────────────────────────────────────────────

/// 从权威稿（`IeltsAuthoringIRV2` 值）抽取本地候选。只读，不产生第二份权威稿。
pub(crate) fn local_candidate_from_authoring(
    authoring: &Value,
    batch_id: &str,
    item_id: &str,
    job_id: &str,
    source_file_id: &str,
    source_sha256: &str,
    base_edit_version: i64,
) -> RecognitionCandidateV1 {
    let generated_at = chrono::Utc::now().to_rfc3339();
    let answer_key = authoring.get("answerKey").cloned().unwrap_or_else(|| json!({}));
    let slot_values = authoring.get("answerSlots").and_then(Value::as_object);

    // 槽位 → (taskId, responseGroupId) 由题组的 responseGroups[].slotIds 反查。
    let mut ownership: Vec<(String, String, String)> = Vec::new();
    let groups = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for group in &groups {
        let Some(task_id) = group.get("taskId").and_then(Value::as_str) else {
            continue;
        };
        if let Some(response_groups) = group.get("responseGroups").and_then(Value::as_array) {
            for response in response_groups {
                let Some(response_id) = response.get("responseGroupId").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(slot_ids) = response.get("slotIds").and_then(Value::as_array) {
                    for slot_id in slot_ids.iter().filter_map(Value::as_str) {
                        ownership.push((
                            slot_id.to_string(),
                            task_id.to_string(),
                            response_id.to_string(),
                        ));
                    }
                }
            }
        }
    }

    let mut task_groups = Vec::new();
    for group in &groups {
        let Some(task_id) = group.get("taskId").and_then(Value::as_str) else {
            continue;
        };
        let task_type = group
            .get("taskType")
            .and_then(Value::as_str)
            .unwrap_or("short_answer")
            .to_string();
        let display_range = group
            .get("displayRange")
            .cloned()
            .unwrap_or_else(|| json!({"kind": "range", "start": 0, "end": 0}));
        let instructions_text = group.get("instructions").map(nodes_text).unwrap_or_default();
        let stimulus_text = group.get("stimulus").map(nodes_text).filter(|text| !text.is_empty());
        let option_bank = group.get("optionBank").and_then(Value::as_object).map(|bank| {
            CandidateOptionBankV1 {
                option_bank_id: bank
                    .get("optionBankId")
                    .and_then(Value::as_str)
                    .unwrap_or("option-bank")
                    .to_string(),
                options: options_from_value(bank.get("options")),
                allow_reuse: bank.get("allowReuse").and_then(Value::as_bool).unwrap_or(false),
            }
        });
        let response_groups = group
            .get("responseGroups")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|response| {
                        let response_id =
                            response.get("responseGroupId").and_then(Value::as_str)?.to_string();
                        Some(CandidateResponseGroupV1 {
                            response_group_id: response_id,
                            kind: response
                                .get("kind")
                                .and_then(Value::as_str)
                                .unwrap_or("composite")
                                .to_string(),
                            prompt: response.get("prompt").map(nodes_text).filter(|t| !t.is_empty()),
                            slot_ids: response
                                .get("slotIds")
                                .and_then(Value::as_array)
                                .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
                                .unwrap_or_default(),
                            options: response.get("options").map(|value| options_from_value(Some(value))),
                            cardinality: response.get("cardinality").cloned(),
                            assignment: response.get("assignment").and_then(Value::as_str).map(str::to_string),
                            allow_option_reuse: response
                                .get("allowOptionReuse")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let confidence = group
            .get("instructionSignature")
            .and_then(|signature| signature.get("confidence"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let warnings = group
            .get("recognitionWarnings")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        task_groups.push(CandidateTaskGroupV1 {
            task_id: task_id.to_string(),
            display_range,
            task_type,
            instructions_text,
            stimulus_text,
            option_bank,
            response_groups,
            source_anchors: group
                .get("sourceAnchors")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            confidence,
            warnings,
        });
    }

    let mut slots = Vec::new();
    for (slot_id, owner_task, owner_response) in &ownership {
        let Some(slot) = slot_values.and_then(|map| map.get(slot_id)) else {
            continue;
        };
        let question_number = slot.get("questionNumber").and_then(Value::as_u64).unwrap_or(0) as u32;
        let source_anchors = slot
            .get("sourceAnchors")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let answer = answer_key.get(slot_id).cloned();
        slots.push(CandidateSlotV1 {
            slot_id: slot_id.clone(),
            question_number,
            display_label: slot
                .get("displayLabel")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            task_id: owner_task.clone(),
            response_group_id: owner_response.clone(),
            interaction: slot
                .get("interaction")
                .and_then(Value::as_str)
                .unwrap_or("text")
                .to_string(),
            host_type: slot.get("hostType").and_then(Value::as_str).unwrap_or("prompt").to_string(),
            host_node_id: slot.get("hostNodeId").and_then(Value::as_str).map(str::to_string),
            participation: slot
                .get("participation")
                .and_then(Value::as_str)
                .unwrap_or("scoring")
                .to_string(),
            // 「本地有答案」不等于「有原文证据」：只认 source anchors 非空。
            has_source_evidence: !source_anchors.is_empty(),
            answer,
        });
    }
    slots.sort_by_key(|slot| slot.question_number);

    let assets = authoring
        .get("assets")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|asset| {
                    let asset_id = asset.get("assetId").and_then(Value::as_str)?.to_string();
                    Some(CandidateAssetV1 {
                        asset_id,
                        sha256: asset.get("sha256").and_then(Value::as_str).unwrap_or_default().to_string(),
                        mime: asset.get("mime").and_then(Value::as_str).map(str::to_string),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let status = if task_groups.is_empty() {
        ChainStatusV1::Partial
    } else {
        ChainStatusV1::Succeeded
    };
    RecognitionCandidateV1 {
        schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
        chain: ChainKindV1::Local,
        batch_id: batch_id.to_string(),
        item_id: item_id.to_string(),
        job_id: job_id.to_string(),
        source_file_id: source_file_id.to_string(),
        source_sha256: source_sha256.to_string(),
        base_edit_version,
        generated_at,
        status,
        reason_code: None,
        task_groups,
        slots,
        unresolved_regions: Vec::new(),
        assets,
        salvage: None,
        warnings: Vec::new(),
    }
}

// ── 云端链 ─────────────────────────────────────────────────────────────

/// 云端原始输出 → 候选。返回 `(candidate, dropped_group_reasons)`。
///
/// fail-closed：题组结构不合法即丢弃（不补齐、不降级为 `short_answer`），
/// 并在 salvage 报告里留下稳定原因码；`reason` 只保留分组级 code，不打原始 JSON。
pub(crate) fn cloud_candidate_from_value(
    raw: &Value,
    batch_id: &str,
    item_id: &str,
    job_id: &str,
    source_file_id: &str,
    source_sha256: &str,
    base_edit_version: i64,
) -> RecognitionCandidateV1 {
    let generated_at = chrono::Utc::now().to_rfc3339();
    // 桥接：把「自然形态」云端输出（模型原始 contract）确定性适配为内部形态。
    // 内部形态（group 同时带 taskId 与 taskType）会原样透传，既有测试零改动。
    let expanded = expand_natural_cloud_shape(raw);
    let raw = &expanded;
    let mut dropped: Vec<String> = Vec::new();
    let mut task_groups: Vec<CandidateTaskGroupV1> = Vec::new();
    let mut slots: Vec<CandidateSlotV1> = Vec::new();
    let mut unresolved_regions: Vec<CandidateUnresolvedRegionV1> = Vec::new();
    let mut assets: Vec<CandidateAssetV1> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let groups = raw
        .get("groups")
        .or_else(|| raw.get("taskGroups"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total_groups = groups.len() as u32;

    for (index, group) in groups.iter().enumerate() {
        let Some(object) = group.as_object() else {
            dropped.push(format!("cloud_group_not_object:{index}"));
            continue;
        };
        let Some(task_id) = object.get("taskId").and_then(Value::as_str) else {
            dropped.push(format!("cloud_group_task_id_missing:{index}"));
            continue;
        };
        let Some(task_type_raw) = object.get("taskType").and_then(Value::as_str) else {
            dropped.push(format!("cloud_group_task_type_missing:{task_id}"));
            continue;
        };
        // 枚举合法性用权威 schema 类型校验，杜绝「未知题型降级为 short_answer」。
        if serde_json::from_value::<TaskTypeV2>(json!(task_type_raw)).is_err() {
            dropped.push(format!("cloud_group_task_type_invalid:{task_id}:{task_type_raw}"));
            continue;
        }
        let range = object.get("range").cloned().unwrap_or(Value::Null);
        let numbers = expand_question_numbers(&range);
        if numbers.is_empty() {
            dropped.push(format!("cloud_group_range_invalid:{task_id}"));
            continue;
        }
        let response_raw = object
            .get("responseGroups")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut response_groups: Vec<CandidateResponseGroupV1> = Vec::new();
        let mut group_slot_ids: Vec<(String, String)> = Vec::new();
        let mut group_valid = true;
        for response in &response_raw {
            let Some(response_id) = response.get("responseGroupId").and_then(Value::as_str) else {
                group_valid = false;
                dropped.push(format!("cloud_response_group_id_missing:{task_id}"));
                break;
            };
            let kind_raw = response.get("kind").and_then(Value::as_str).unwrap_or("");
            if serde_json::from_value::<ResponseGroupKindV2>(json!(kind_raw)).is_err() {
                group_valid = false;
                dropped.push(format!("cloud_response_group_kind_invalid:{task_id}:{response_id}"));
                break;
            }
            response_groups.push(CandidateResponseGroupV1 {
                response_group_id: response_id.to_string(),
                kind: kind_raw.to_string(),
                prompt: response.get("prompt").and_then(Value::as_str).map(str::to_string),
                slot_ids: response
                    .get("slotIds")
                    .and_then(Value::as_array)
                    .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default(),
                options: response.get("options").map(|value| options_from_value(Some(value))),
                cardinality: response.get("cardinality").cloned(),
                assignment: response.get("assignment").and_then(Value::as_str).map(str::to_string),
                allow_option_reuse: response
                    .get("allowOptionReuse")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
            for slot_id in response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                group_slot_ids.push((slot_id.to_string(), response_id.to_string()));
            }
        }
        if !group_valid {
            continue;
        }
        let slot_raw = object
            .get("slots")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if slot_raw.len() != numbers.len() {
            dropped.push(format!(
                "cloud_group_slot_count_mismatch:{task_id}:declared={}:actual={}",
                numbers.len(),
                slot_raw.len()
            ));
            continue;
        }
        let mut group_slots: Vec<CandidateSlotV1> = Vec::new();
        let mut slot_valid = true;
        for slot in &slot_raw {
            match normalize_cloud_slot(slot, task_id, &group_slot_ids) {
                Ok(normalized) => group_slots.push(normalized),
                Err(code) => {
                    dropped.push(format!("{code}:{task_id}"));
                    slot_valid = false;
                    break;
                }
            }
        }
        if !slot_valid {
            continue;
        }
        group_slots.sort_by_key(|slot| slot.question_number);
        let declared: Vec<u32> = group_slots.iter().map(|slot| slot.question_number).collect();
        if declared != numbers {
            dropped.push(format!("cloud_group_question_numbers_mismatch:{task_id}"));
            continue;
        }
        let option_bank = object.get("optionBank").and_then(Value::as_object).map(|bank| {
            CandidateOptionBankV1 {
                option_bank_id: bank
                    .get("optionBankId")
                    .and_then(Value::as_str)
                    .unwrap_or("option-bank")
                    .to_string(),
                options: options_from_value(bank.get("options")),
                allow_reuse: bank.get("allowReuse").and_then(Value::as_bool).unwrap_or(false),
            }
        });
        slots.extend(group_slots);
        task_groups.push(CandidateTaskGroupV1 {
            task_id: task_id.to_string(),
            display_range: range,
            task_type: task_type_raw.to_string(),
            instructions_text: object
                .get("instructionsText")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            stimulus_text: object
                .get("stimulusText")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_string),
            option_bank,
            response_groups,
            source_anchors: Vec::new(),
            confidence: object.get("confidence").and_then(Value::as_f64).unwrap_or(0.0),
            warnings: object
                .get("warnings")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
        });
    }

    if let Some(regions) = raw.get("unresolvedRegions").and_then(Value::as_array) {
        for region in regions {
            unresolved_regions.push(CandidateUnresolvedRegionV1 {
                page_index: region.get("pageIndex").and_then(Value::as_u64).unwrap_or(0) as u32,
                note: region
                    .get("note")
                    .and_then(Value::as_str)
                    .unwrap_or("未识别的区域")
                    .to_string(),
                reason_code: region.get("reasonCode").and_then(Value::as_str).map(str::to_string),
            });
        }
    }
    if let Some(list) = raw.get("assets").and_then(Value::as_array) {
        for asset in list {
            if let Some(asset_id) = asset.get("assetId").and_then(Value::as_str) {
                assets.push(CandidateAssetV1 {
                    asset_id: asset_id.to_string(),
                    sha256: asset.get("sha256").and_then(Value::as_str).unwrap_or_default().to_string(),
                    mime: asset.get("mime").and_then(Value::as_str).map(str::to_string),
                });
            }
        }
    }

    slots.sort_by_key(|slot| slot.question_number);
    let kept_groups = task_groups.len() as u32;
    let dropped_groups = total_groups.saturating_sub(kept_groups);
    let salvage = if dropped_groups > 0 {
        warnings.push(reason::SALVAGE_PARTIAL.to_string());
        Some(SalvageReportV1 {
            total_groups,
            kept_groups,
            dropped_groups,
            dropped_reasons: dropped,
            repairs_used: 0,
        })
    } else {
        None
    };
    // 状态判定：全部保留 → succeeded；保留了一部分 → partial；一个都没保住 → unusable。
    // 特别注意：「声称成功但一个题组都没有」是无效输出（fail-closed），
    // 不得因为 `groups` 缺失而把云端链记成 Succeeded。
    let status = if total_groups == 0 {
        ChainStatusV1::Unusable
    } else if kept_groups == total_groups {
        ChainStatusV1::Succeeded
    } else if kept_groups > 0 {
        ChainStatusV1::Partial
    } else {
        ChainStatusV1::Unusable
    };

    RecognitionCandidateV1 {
        schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
        chain: ChainKindV1::Cloud,
        batch_id: batch_id.to_string(),
        item_id: item_id.to_string(),
        job_id: job_id.to_string(),
        source_file_id: source_file_id.to_string(),
        source_sha256: source_sha256.to_string(),
        base_edit_version,
        generated_at,
        status,
        reason_code: match status {
            ChainStatusV1::Unusable => Some(reason::MODEL_INVALID_OUTPUT.to_string()),
            ChainStatusV1::Partial => Some(reason::SALVAGE_PARTIAL.to_string()),
            _ => None,
        },
        task_groups,
        slots,
        unresolved_regions,
        assets,
        salvage,
        warnings,
    }
}

fn normalize_cloud_slot(
    slot: &Value,
    task_id: &str,
    ownership: &[(String, String)],
) -> Result<CandidateSlotV1, String> {
    let Some(object) = slot.as_object() else {
        return Err("cloud_slot_not_object".to_string());
    };
    let Some(slot_id) = object.get("slotId").and_then(Value::as_str) else {
        return Err("cloud_slot_id_missing".to_string());
    };
    let Some(question_number) = object.get("questionNumber").and_then(Value::as_u64) else {
        return Err("cloud_slot_question_number_missing".to_string());
    };
    if question_number == 0 || question_number > 200 {
        return Err("cloud_slot_question_number_invalid".to_string());
    }
    let interaction = object
        .get("interaction")
        .and_then(Value::as_str)
        .ok_or_else(|| "cloud_slot_interaction_missing".to_string())?;
    if serde_json::from_value::<InteractionV2>(json!(interaction)).is_err() {
        return Err("cloud_slot_interaction_invalid".to_string());
    }
    let host_type = object
        .get("hostType")
        .and_then(Value::as_str)
        .ok_or_else(|| "cloud_slot_host_type_missing".to_string())?;
    if serde_json::from_value::<AnswerSlotHostTypeV2>(json!(host_type)).is_err() {
        return Err("cloud_slot_host_type_invalid".to_string());
    }
    let participation = object
        .get("participation")
        .and_then(Value::as_str)
        .unwrap_or("scoring");
    if serde_json::from_value::<AnswerSlotParticipationV2>(json!(participation)).is_err() {
        return Err("cloud_slot_participation_invalid".to_string());
    }
    let answer = match object.get("answer") {
        None | Some(Value::Null) => None,
        Some(value) => {
            // 答案形状必须能被权威 `AnswerValueV2` 解析；否则视为该题无答案（不伪造）。
            serde_json::from_value::<AnswerValueV2>(value.clone())
                .map(|_| value.clone())
                .map_err(|_| "cloud_slot_answer_invalid".to_string())?
                .into()
        }
    };
    let evidence = object.get("evidence").and_then(Value::as_array);
    let has_source_evidence = evidence
        .map(|items| {
            items.iter().any(|item| {
                item.get("quote")
                    .and_then(Value::as_str)
                    .map(|quote| !quote.trim().is_empty())
                    .unwrap_or(false)
                    || item.get("bbox").map(|bbox| !bbox.is_null()).unwrap_or(false)
            })
        })
        .unwrap_or(false);
    let response_group_id = ownership
        .iter()
        .find(|(id, _)| id == slot_id)
        .map(|(_, response)| response.clone())
        .unwrap_or_default();
    Ok(CandidateSlotV1 {
        slot_id: slot_id.to_string(),
        question_number: question_number as u32,
        display_label: object
            .get("displayLabel")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        task_id: task_id.to_string(),
        response_group_id,
        interaction: interaction.to_string(),
        host_type: host_type.to_string(),
        host_node_id: object.get("hostNodeId").and_then(Value::as_str).map(str::to_string),
        participation: participation.to_string(),
        answer,
        has_source_evidence,
    })
}

/// 构造一条「未运行」候选（缺配置 / 能力不足 / 被跳过），必须带稳定原因码。
pub(crate) fn not_run_candidate(
    chain: ChainKindV1,
    batch_id: &str,
    item_id: &str,
    job_id: &str,
    source_file_id: &str,
    source_sha256: &str,
    base_edit_version: i64,
    reason_code: &str,
) -> RecognitionCandidateV1 {
    RecognitionCandidateV1 {
        schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
        chain,
        batch_id: batch_id.to_string(),
        item_id: item_id.to_string(),
        job_id: job_id.to_string(),
        source_file_id: source_file_id.to_string(),
        source_sha256: source_sha256.to_string(),
        base_edit_version,
        generated_at: chrono::Utc::now().to_rfc3339(),
        status: ChainStatusV1::NotRun,
        reason_code: Some(reason_code.to_string()),
        task_groups: Vec::new(),
        slots: Vec::new(),
        unresolved_regions: Vec::new(),
        assets: Vec::new(),
        salvage: None,
        warnings: Vec::new(),
    }
}

/// 把候选投影回 `IeltsAuthoringIRV2` 的**比对视图**（不是权威稿）。
/// 用于原文件核验与差异定位；字段命名与权威 schema 一致，便于统一比较。
pub(crate) fn candidate_semantic_view(candidate: &RecognitionCandidateV1) -> Value {
    let mut slots = Map::new();
    for slot in &candidate.slots {
        slots.insert(
            slot.slot_id.clone(),
            json!({
                "slotId": slot.slot_id,
                "questionNumber": slot.question_number,
                "displayLabel": slot.display_label,
                "hostType": slot.host_type,
                "interaction": slot.interaction,
                "participation": slot.participation,
                "taskId": slot.task_id,
                "responseGroupId": slot.response_group_id,
                "hostNodeId": slot.host_node_id,
                "hasSourceEvidence": slot.has_source_evidence,
            }),
        );
    }
    let mut answer_key = Map::new();
    for slot in &candidate.slots {
        if let Some(answer) = &slot.answer {
            answer_key.insert(slot.slot_id.clone(), answer.clone());
        }
    }
    json!({
        "taskGroups": candidate.task_groups,
        "answerSlots": Value::Object(slots),
        "answerKey": Value::Object(answer_key),
        "assets": candidate.assets,
    })
}

/// 机械重写：把稿件里所有「承载身份/引用」的字段，按 `id_map`（临时 id → 稳定 id）
/// 替换掉，并如实报告**映射不上**的引用与**目标键冲突**。
///
/// # 身份不靠字符串前缀猜
///
/// 「哪些 id 属于临时空间」由调用方用 `temp_ids` **显式给出**，而不是看 id 长什么样。
/// 于是三类引用被彻底分开：
///
/// | 类别 | 判据 | 处理 |
/// |---|---|---|
/// | 临时引用（模型自造，形如 `cloud-q14`） | 在 `temp_ids` 里 | 在 `id_map` 里就改写；不在就**原样保留并上报** |
/// | 既有稳定引用（后端已分配） | 不在 `temp_ids` 里 | 原样保留，**不上报**（它不是缺口） |
/// | 源文档引用 | `nodeIds` / `sourceTableId` 字段 | 永不重写、永不上报 |
///
/// 第三行的实现**不靠字段值猜**：直接按字段名跳过，所以正文里出现同形字符串也不会被改。
///
/// # 冲突即整篇放弃（原子性保证）
/// 只要任意一处 slot-keyed map（`answerSlots` / `answerKey`）出现目标键冲突，本次改写**整篇作废**：
/// 文档保持调用前的原样，`ReferenceRewriteOutcome::applied` 为 `false` 且 `conflicts` 非空。
/// 调用方**收到的是「全有或全无」**——绝不会留下「键没改、值却改了」的半截、内部不一致的文档。
///
/// # 实际覆盖的字段清单
/// 下列字段的值（或数组元素）按 map 替换；属于临时空间但无映射的原样保留并计入
/// `unmapped`（资源引用除外，见下）：
///
/// - `taskGroups[].taskId`
/// - `taskGroups[].optionBank.optionBankId`
/// - `taskGroups[].optionBank.options[].optionId`
/// - `taskGroups[].responseGroups[].responseGroupId`
/// - `taskGroups[].responseGroups[].slotIds[]`（逐元素）
/// - `taskGroups[].responseGroups[].optionBankRef`
/// - `taskGroups[].responseGroups[].options[].optionId`
/// - `answerSlots` 的**键**（以 slotId 为键），以及每个值的 `slotId`、`hostNodeId`
/// - `answerKey` 的**键**（以 slotId 为键）
/// - 内容节点（`ContentNodeV2`）的 `id`，递归覆盖所有出现位置
///   （passage.content、instructions、stimulus、option content、figure/image 等）
/// - `answer_slot` 内容节点的 `slotId`
/// - `listening.parts[].taskIds[]`（引用 `taskGroups[].taskId`，属草稿 id 空间；
///   证据：`schema/listening_runtime_v1.rs` 正是拿它去由 `task_groups[].taskId`
///   建的表里查）
/// - `assets[].assetId`、`content[].visualFallbackAssetId`：**仅当 map 中存在映射时才换**。
///   资源 id 由后端登记、本就稳定；模型引用了**不存在的**资源属「资源校验失败」，
///   不是「临时 id 映射失败」，故**不**计入 `unmapped`——混进来会让真正的映射缺口
///   被资源噪声淹没
///
/// # 明确不重写
/// - `sourceAnchors[].nodeIds[]`（源文档节点，重写会破坏溯源）
/// - `sourceTableId`（`schema/content_doc_v2.rs` 指向**源文档**里的表，同理）
/// - `paragraphMap`：键值语义在代码里**没有定义**（全仓只有 `reading_source_v2.rs`
///   一次 `clone()`，从未用于查表），键到底是「标签→节点 id」还是「节点 id→标签」
///   无从判断，猜着重写会改坏内容。**若日后定义了语义，需回来补。**
/// - 任何非上述名单的字段（特别是正文 `text` 节点）
///
/// - 纯函数：无 IO、无随机、无时间戳；同一输入必得同一输出。
///
/// # 参数
/// - `document`：已标准化的稿件（`IeltsAuthoringIRV2` 形态）的 `Value`，原地改写。
/// - `id_map`：`临时 id → 稳定 id` 的映射表（推导映射不属于本函数职责）。
/// - `temp_ids`：**模型临时 id 的全集**（含尚未解析的那些）。用它区分「临时引用」与
///   「既有稳定引用」；不传它就只能靠猜，那正是要避免的。
pub(crate) fn rewrite_authoring_references(
    document: &mut Value,
    id_map: &BTreeMap<String, String>,
    temp_ids: &BTreeSet<String>,
) -> ReferenceRewriteOutcome {
    // 在克隆上改写，冲突则整篇放弃——保证「全有或全无」，绝不留下半截文档。
    let mut accumulator = RewriteAccumulator::default();
    let mut working = document.clone();
    rewrite_value(&mut working, id_map, temp_ids, &mut accumulator);
    if !accumulator.conflicts.is_empty() {
        // 原子性边界：任何冲突都意味着文档保持调用前原样，一个字节都不动。
        return ReferenceRewriteOutcome {
            unmapped: accumulator.unmapped.into_iter().collect(),
            conflicts: accumulator.conflicts.into_iter().collect(),
            applied: false,
        };
    }
    *document = working;
    ReferenceRewriteOutcome {
        unmapped: accumulator.unmapped.into_iter().collect(),
        conflicts: vec![],
        // 只有确实发生过至少一次实际改写才算 true；见 `RewriteAccumulator::applied`。
        applied: accumulator.applied,
    }
}

/// 引用重写的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceRewriteOutcome {
    /// 属于临时空间、但没有映射可用的引用（已原样保留在稿件里）。排序去重。
    pub unmapped: Vec<String>,
    /// 目标键冲突：多个源键映射到同一目标，或与未被映射的保留键相撞。
    /// 命中冲突时整个改写**整篇作废**（`applied == false`），文档保持原样。
    pub conflicts: Vec<String>,
    /// 本次调用是否**至少完成了一次实际改写**。
    ///
    /// `false` 表示没有任何引用被替换（映射为空，或映射与本文档中的引用完全不相交）。
    /// **空映射绝不能当作「引用全部解析成功」**——那正是这个字段要区分的东西。
    pub applied: bool,
}

/// 内部累加器：`BTreeSet` 顺带保证排序去重，不依赖调用方事后整理。
#[derive(Default)]
struct RewriteAccumulator {
    unmapped: BTreeSet<String>,
    conflicts: BTreeSet<String>,
    /// 至少发生过一次实际替换 / 键重命名。空映射或映射与本文档引用完全不相交时为 `false`。
    applied: bool,
}

/// 递归改写：对象按具名字段处理，数组递归，标量不动。
fn rewrite_value(
    value: &mut Value,
    id_map: &BTreeMap<String, String>,
    temp_ids: &BTreeSet<String>,
    outcome: &mut RewriteAccumulator,
) {
    match value {
        Value::Object(map) => rewrite_object(map, id_map, temp_ids, outcome),
        Value::Array(items) => {
            for item in items.iter_mut() {
                rewrite_value(item, id_map, temp_ids, outcome);
            }
        }
        _ => {}
    }
}

fn rewrite_object(
    map: &mut Map<String, Value>,
    id_map: &BTreeMap<String, String>,
    temp_ids: &BTreeSet<String>,
    outcome: &mut RewriteAccumulator,
) {
    // 1) 以 slotId 为键的 map：重命名键。
    rename_slot_keyed_map(map, "answerSlots", id_map, temp_ids, outcome);
    rename_slot_keyed_map(map, "answerKey", id_map, temp_ids, outcome);

    // 2) 遍历具名字段。
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        let Some(value) = map.get_mut(&key) else { continue };
        match key.as_str() {
            // 源文档引用：**按字段名**跳过，不靠值猜。`nodeIds` 指向源文档的节点、
            // `sourceTableId` 指向源文档里的表——重写它们会破坏溯源。
            "nodeIds" | "sourceTableId" => {}
            // 资源引用：仅当 map 中有映射才换；绝不臆造，且**不**计入未映射列表
            // （资源 id 由后端登记、本就稳定；未知资源属资源校验失败，不是映射缺口）。
            "assetId" | "visualFallbackAssetId" => rewrite_asset_id(value, id_map, outcome),
            "taskId" | "optionBankId" | "responseGroupId" | "optionBankRef"
            | "slotId" | "hostNodeId" | "optionId" | "id" => {
                rewrite_scalar_ref(value, id_map, temp_ids, outcome);
            }
            // 引用 id 的数组：`slotIds`（答案槽）与 listening 的 `taskIds`（题组）。
            "slotIds" | "taskIds" => {
                if let Value::Array(items) = value {
                    for item in items.iter_mut() {
                        rewrite_scalar_ref(item, id_map, temp_ids, outcome);
                    }
                }
            }
            _ => rewrite_value(value, id_map, temp_ids, outcome),
        }
    }
}

/// 重命名以 slotId 为键的 map（`answerSlots` / `answerKey`）的键。
///
/// **在独立对象里构建结果，冲突就整块放弃**——绝不在原 map 上边删边插：那种写法在
/// 「目标键已被另一个源键占用」或「映射互换（`A→B`、`B→A`）」时会静默覆盖内容，
/// 而答案恰恰是最不能丢的东西。
///
/// 冲突判据（任一命中即**整块保持原样**并如实上报，不做半截重写）：
/// - 两个源键映射到**同一目标**（`A→C` 与 `B→C`）；
/// - 某源键的目标与**另一个未映射的保留键**相撞（`A→B`，而 `B` 自身保留为 `B`）。
///
/// 此处的提前返回只是**防卫性**兜底：真正的原子性保证在 `rewrite_authoring_references`
/// 外层——冲突一旦进入 `accumulator.conflicts`，整篇改写会被丢弃、`document` 原样不动。
fn rename_slot_keyed_map(
    map: &mut Map<String, Value>,
    name: &str,
    id_map: &BTreeMap<String, String>,
    temp_ids: &BTreeSet<String>,
    outcome: &mut RewriteAccumulator,
) {
    let Some(Value::Object(inner)) = map.get_mut(name) else { return };

    // 先在独立对象里算好；任何冲突都不回写。
    let mut planned: Map<String, Value> = Map::new();
    // 目标键 → 源键，用于检测「两个源键挤向同一目标」。
    let mut origin: BTreeMap<String, String> = BTreeMap::new();
    let mut conflicts: BTreeSet<String> = BTreeSet::new();
    let mut changed = false;

    for (old, value) in inner.iter() {
        let new = match id_map.get(old) {
            Some(stable) => {
                changed = true;
                stable.clone()
            }
            None => {
                if temp_ids.contains(old) {
                    outcome.unmapped.insert(old.clone());
                }
                old.clone()
            }
        };
        if let Some(previous) = origin.get(&new) {
            conflicts.insert(format!("{name}: {previous} 与 {old} 同时指向 {new}"));
            continue;
        }
        origin.insert(new.clone(), old.clone());
        planned.insert(new, value.clone());
    }

    // 冲突优先判定：整块保持原样（防卫性兜底；权威保证见外层整篇放弃）。
    if !conflicts.is_empty() {
        outcome.conflicts.extend(conflicts);
        return;
    }
    // 一个键都没改名 ⇒ 零改动。空映射必须走这条路径返回，保证文档字节级不变。
    if !changed {
        return;
    }
    // 至少重命名了一次键，记一笔实际改写。
    outcome.applied = true;
    *inner = planned;
}

/// 改写单个字符串引用：在 map 中则替换；属于临时空间但无映射则原样保留并计入
/// `unmapped`；其余（**既有稳定引用**）原样保留且**不上报**——它不是缺口。
/// 非字符串（如 `null`）不动、不计（不改 null、不臆造）。
fn rewrite_scalar_ref(
    value: &mut Value,
    id_map: &BTreeMap<String, String>,
    temp_ids: &BTreeSet<String>,
    outcome: &mut RewriteAccumulator,
) {
    if let Value::String(current) = value {
        if let Some(stable) = id_map.get(current) {
            *current = stable.clone();
            outcome.applied = true;
        } else if temp_ids.contains(current) {
            outcome.unmapped.insert(current.clone());
        }
    }
}

/// 资源引用（`assetId` / `visualFallbackAssetId`）：仅当 map 中存在映射才换；
/// 绝不臆造，且**不**计入未映射列表。
fn rewrite_asset_id(value: &mut Value, id_map: &BTreeMap<String, String>, outcome: &mut RewriteAccumulator) {
    if let Value::String(current) = value {
        if let Some(stable) = id_map.get(current) {
            *current = stable.clone();
            outcome.applied = true;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// 云端「完整候选」：标准化 / 身份对齐 / 引用重写
//
// 产品授权（见 `Plan With Files/Dual_Recognition/CLOUD_REPAIR_IMPLEMENTATION_BRIEF_2026-09-17.md`）：
// 本地几何识别只负责快速初稿，云端**独立识别**产出完整候选（正文 / 题组 / 选项 / 作答
// 位置 / 答案），再由云端校核修复真实调用编辑工具。云端可以纠正本地「确定性结论」。
//
// 本段负责把模型输出接上后端身份，三条纪律：
//
// 1. **模型不拥有身份**。job / source / audit / quality / reviewState / 稳定 ID 全部由后端
//    生成或计算；模型只给内容与临时引用。
// 2. **能唯一对齐就复用，不能就分配，确实不确定才留歧义**。响应组、选项库、内容节点
//    **不得**因为「没有现成映射算法」而整类降级成人工问题；那是把后端的活推给用户。
// 3. **引用重写全有或全无**。冲突即整篇放弃（复用 `rewrite_authoring_references`）。
// ─────────────────────────────────────────────────────────────────────

/// 参与身份对齐的**标量引用字段**（这些字段的值属于 ID 空间）。
const REFERENCE_SCALAR_FIELDS: [&str; 11] = [
    "id",
    "taskId",
    "optionBankId",
    "responseGroupId",
    "optionBankRef",
    "slotId",
    "hostNodeId",
    "optionId",
    "assetId",
    "visualFallbackAssetId",
    "assetRef",
];

/// 参与身份对齐的**引用数组字段**。
const REFERENCE_ARRAY_FIELDS: [&str; 3] = ["slotIds", "taskIds", "assetIds"];

/// 云端完整候选的**后端身份与元信息**。模型无权提供其中任何一项。
pub(crate) struct CloudAuthoringIdentity<'a> {
    pub job_id: &'a str,
    pub item_id: &'a str,
    pub batch_id: &'a str,
    pub source_file_id: &'a str,
    pub source_sha256: &'a str,
    pub base_edit_version: i64,
    pub generated_at: &'a str,
    /// `ExamMetaV2` 形状的考试元信息（由 job 构造，模型无权填写）。
    pub exam: Value,
    pub modality: &'a str,
    pub source_document_id: &'a str,
    /// 后端登记的抽取方式（`ExtractionModeV2`）。模型无从得知，锚点缺该字段时由后端补。
    pub extraction_mode: &'a str,
}

/// 标准化结果：**尚未**做质量重算与类型化，调用方负责补上（见 `auto_pipeline`）。
pub(crate) struct NormalizedCloudAuthoring {
    pub status: ChainStatusV1,
    pub reason_code: Option<String>,
    pub document: Value,
    pub id_map: BTreeMap<String, String>,
    pub unresolved_references: Vec<String>,
    pub unresolved_regions: Vec<CloudCandidateUnresolvedRegionV1>,
    pub source_coverage_notes: Vec<String>,
    pub warnings: Vec<String>,
    /// 分块识别时，失败的块没有覆盖到的题号（升序去重）。非空 ⇒ 候选至多 `Partial`。
    pub uncovered_question_numbers: Vec<u32>,
}

/// 权威稿里一个题组的**身份索引**（只读）。
struct CanonicalGroupIndex {
    task_id: String,
    task_type: String,
    numbers: BTreeSet<u32>,
    response_groups: Vec<(String, BTreeSet<String>)>,
    option_bank_id: Option<String>,
}

/// 权威稿里一个答案槽的身份索引。
struct CanonicalSlotIndex {
    key: String,
    slot_id: String,
    question_number: u32,
    task_id: Option<String>,
}

/// 题组对齐结论。**不确定时绝不任取第一个**。
enum GroupMatch {
    /// 唯一对应当前稿件的题组。
    Unique(usize),
    /// 云端识别出的新增题组。
    New,
    /// 有多个候选，无法唯一确定。
    Ambiguous(Vec<String>),
}

/// 收集一份稿件里**全部属于 ID 空间**的字符串（含 `answerSlots` / `answerKey` 的键）。
///
/// 用途有两个：界定「临时 ID 全集」（重写器据此区分临时引用与既有稳定引用），
/// 以及统计已占用 ID（分配新 ID 时避免撞车）。
fn collect_reference_ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "answerSlots" || key == "answerKey" {
                    if let Some(entries) = child.as_object() {
                        for (slot_key, slot) in entries {
                            out.insert(slot_key.clone());
                            collect_reference_ids(slot, out);
                        }
                        continue;
                    }
                }
                if REFERENCE_SCALAR_FIELDS.contains(&key.as_str()) {
                    if let Some(text) = child.as_str() {
                        out.insert(text.to_string());
                    }
                } else if REFERENCE_ARRAY_FIELDS.contains(&key.as_str()) {
                    if let Some(items) = child.as_array() {
                        for item in items {
                            if let Some(text) = item.as_str() {
                                out.insert(text.to_string());
                            }
                        }
                    }
                }
                collect_reference_ids(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_reference_ids(item, out);
            }
        }
        _ => {}
    }
}

/// 分配一个当前未被占用的稳定 ID（确定性：同一输入必得同一结果）。
fn alloc_stable_id(preferred: &str, used: &BTreeSet<String>) -> String {
    if !used.contains(preferred) {
        return preferred.to_string();
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{preferred}-{suffix}");
        if !used.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// 权威稿的题组身份索引。
fn canonical_group_index(canonical: &Value) -> Vec<CanonicalGroupIndex> {
    let Some(groups) = canonical.get("taskGroups").and_then(Value::as_array) else {
        return Vec::new();
    };
    groups
        .iter()
        .filter_map(|group| {
            let task_id = group.get("taskId").and_then(Value::as_str)?.to_string();
            let task_type = group
                .get("taskType")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let numbers: BTreeSet<u32> = group
                .get("displayRange")
                .map(expand_question_numbers)
                .unwrap_or_default()
                .into_iter()
                .collect();
            let mut response_groups = Vec::new();
            if let Some(items) = group.get("responseGroups").and_then(Value::as_array) {
                for item in items {
                    let Some(id) = item.get("responseGroupId").and_then(Value::as_str) else {
                        continue;
                    };
                    let slot_ids: BTreeSet<String> = item
                        .get("slotIds")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    response_groups.push((id.to_string(), slot_ids));
                }
            }
            let option_bank_id = group
                .get("optionBank")
                .and_then(|bank| bank.get("optionBankId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(CanonicalGroupIndex {
                task_id,
                task_type,
                numbers,
                response_groups,
                option_bank_id,
            })
        })
        .collect()
}

/// 权威稿的答案槽身份索引。
///
/// 所属题组通过 `responseGroups[].slotIds` **反查**得到，而不是靠 `qN` 命名规则猜。
fn canonical_slot_index(canonical: &Value, groups: &[CanonicalGroupIndex]) -> Vec<CanonicalSlotIndex> {
    let Some(slots) = canonical.get("answerSlots").and_then(Value::as_object) else {
        return Vec::new();
    };
    // slotId 字段值 → 所属 taskId（由题组的 responseGroups 建立）。
    let mut owner_by_ref: BTreeMap<String, String> = BTreeMap::new();
    for group in groups {
        for (_, slot_ids) in &group.response_groups {
            for reference in slot_ids {
                owner_by_ref
                    .entry(reference.clone())
                    .or_insert_with(|| group.task_id.clone());
            }
        }
    }
    slots
        .iter()
        .filter_map(|(key, slot)| {
            let slot_id = slot
                .get("slotId")
                .and_then(Value::as_str)
                .unwrap_or(key)
                .to_string();
            let question_number = slot.get("questionNumber").and_then(Value::as_u64)? as u32;
            let task_id = owner_by_ref
                .get(&slot_id)
                .or_else(|| owner_by_ref.get(key))
                .cloned();
            Some(CanonicalSlotIndex {
                key: key.clone(),
                slot_id,
                question_number,
                task_id,
            })
        })
        .collect()
}

/// 云端题组声明的答案槽引用（`responseGroups[].slotIds`）。
fn cloud_group_slot_refs(group: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Some(groups) = group.get("responseGroups").and_then(Value::as_array) {
        for item in groups {
            if let Some(ids) = item.get("slotIds").and_then(Value::as_array) {
                for id in ids {
                    if let Some(text) = id.as_str() {
                        out.insert(text.to_string());
                    }
                }
            }
        }
    }
    out
}

/// 云端题组的题号集合：优先 `displayRange`，缺失时由它声明的答案槽题号回填。
fn cloud_group_numbers(group: &Value, draft: &Value) -> BTreeSet<u32> {
    let mut numbers: BTreeSet<u32> = group
        .get("displayRange")
        .map(expand_question_numbers)
        .unwrap_or_default()
        .into_iter()
        .collect();
    if !numbers.is_empty() {
        return numbers;
    }
    let refs = cloud_group_slot_refs(group);
    if let Some(slots) = draft.get("answerSlots").and_then(Value::as_object) {
        for (key, slot) in slots {
            let slot_id = slot.get("slotId").and_then(Value::as_str).unwrap_or(key);
            if refs.contains(key) || refs.contains(slot_id) {
                if let Some(number) = slot.get("questionNumber").and_then(Value::as_u64) {
                    numbers.insert(number as u32);
                }
            }
        }
    }
    numbers
}

/// 把云端题组对齐到当前权威稿的题组。
///
/// 判据（依次收紧到放宽，**唯一才认**）：
/// 1. 题型相同 **且** 题号有交集；
/// 2. 退一步，只要题号有交集（题型本身可能就是云端要纠正的对象）。
///
/// 任何一层出现多个候选都算歧义——交给修复回合结合原文判断，绝不任取第一个。
fn match_task_group(
    cloud_type: &str,
    cloud_numbers: &BTreeSet<u32>,
    canonical: &[CanonicalGroupIndex],
) -> GroupMatch {
    if cloud_numbers.is_empty() {
        return GroupMatch::New;
    }
    let overlapping: Vec<usize> = canonical
        .iter()
        .enumerate()
        .filter(|(_, group)| group.numbers.intersection(cloud_numbers).next().is_some())
        .map(|(index, _)| index)
        .collect();
    if overlapping.is_empty() {
        return GroupMatch::New;
    }
    let strict: Vec<usize> = overlapping
        .iter()
        .copied()
        .filter(|index| canonical[*index].task_type == cloud_type)
        .collect();
    let chosen = if strict.len() == 1 {
        strict
    } else if strict.len() > 1 {
        return GroupMatch::Ambiguous(
            strict
                .iter()
                .map(|index| canonical[*index].task_id.clone())
                .collect(),
        );
    } else if overlapping.len() == 1 {
        overlapping
    } else {
        return GroupMatch::Ambiguous(
            overlapping
                .iter()
                .map(|index| canonical[*index].task_id.clone())
                .collect(),
        );
    };
    GroupMatch::Unique(chosen[0])
}

/// 为内容节点分配稳定 ID：**同一容器内位置相同且节点类型相同**才复用权威稿的 ID，
/// 否则由后端分配新 ID。
///
/// 为什么可以按位置复用：只有在题组已经被**唯一对齐**之后才会走到这里，此时两个容器的
/// 语义位置是同一条；类型不同说明该位置确实换了节点，那就必须是新身份。
/// 注意 `nodeIds`（源文档锚点）**不**在这里，也不在任何重写范围内——它指向源文档节点，
/// 与题稿节点是两个命名空间。
fn assign_node_ids(
    cloud: &Value,
    canonical: Option<&Value>,
    prefix: &str,
    counter: &mut usize,
    id_map: &mut BTreeMap<String, String>,
) {
    match cloud {
        Value::Array(items) => {
            let canonical_items = canonical.and_then(Value::as_array);
            for (index, item) in items.iter().enumerate() {
                let counterpart = canonical_items.and_then(|arr| arr.get(index));
                assign_node_ids(item, counterpart, prefix, counter, id_map);
            }
        }
        Value::Object(map) => {
            if let Some(old) = map.get("id").and_then(Value::as_str) {
                let same_shape = canonical
                    .and_then(|value| value.get("type"))
                    .and_then(Value::as_str)
                    .zip(map.get("type").and_then(Value::as_str))
                    .map(|(left, right)| left == right)
                    .unwrap_or(false);
                let reused = if same_shape {
                    canonical
                        .and_then(|value| value.get("id"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                } else {
                    None
                };
                let stable = reused.unwrap_or_else(|| {
                    *counter += 1;
                    format!("{prefix}-cn-{counter}")
                });
                id_map.insert(old.to_string(), stable);
            }
            if let Some(children) = map.get("children") {
                let canonical_children = canonical.and_then(|value| value.get("children"));
                assign_node_ids(children, canonical_children, prefix, counter, id_map);
            }
        }
        _ => {}
    }
}

/// 按标签在权威稿的选项库里找同名选项（选项 ID 的身份来自**标签**，不是数组下标）。
fn canonical_option_id_for_label(canonical_bank: Option<&Value>, label: &str) -> Option<String> {
    canonical_bank?
        .get("options")?
        .as_array()?
        .iter()
        .find(|option| option.get("label").and_then(Value::as_str) == Some(label))?
        .get("optionId")?
        .as_str()
        .map(str::to_string)
}

/// 为一个云端题组内部的对象分配稳定 ID（内容节点、选项、提示等）。
fn assign_group_inner_ids(
    cloud_group: &Value,
    canonical_group: Option<&Value>,
    stable_task_id: &str,
    id_map: &mut BTreeMap<String, String>,
) {
    let mut counter = 0usize;

    for container in ["instructions", "stimulus"] {
        if let Some(cloud_nodes) = cloud_group.get(container) {
            let canonical_nodes = canonical_group.and_then(|group| group.get(container));
            assign_node_ids(cloud_nodes, canonical_nodes, stable_task_id, &mut counter, id_map);
        }
    }

    if let Some(cloud_bank) = cloud_group.get("optionBank") {
        let canonical_bank = canonical_group.and_then(|group| group.get("optionBank"));
        if let Some(title) = cloud_bank.get("title") {
            let canonical_title = canonical_bank.and_then(|bank| bank.get("title"));
            assign_node_ids(title, canonical_title, stable_task_id, &mut counter, id_map);
        }
        if let Some(options) = cloud_bank.get("options").and_then(Value::as_array) {
            for (index, option) in options.iter().enumerate() {
                let label = option.get("label").and_then(Value::as_str).unwrap_or("");
                if let Some(old) = option.get("optionId").and_then(Value::as_str) {
                    let stable = canonical_option_id_for_label(canonical_bank, label)
                        .unwrap_or_else(|| {
                            if label.is_empty() {
                                format!("{stable_task_id}-opt-{}", index + 1)
                            } else {
                                format!("{stable_task_id}-opt-{label}")
                            }
                        });
                    id_map.insert(old.to_string(), stable);
                }
                if let Some(content) = option.get("content") {
                    let canonical_content = canonical_bank
                        .and_then(|bank| bank.get("options"))
                        .and_then(Value::as_array)
                        .and_then(|arr| {
                            arr.iter()
                                .find(|candidate| {
                                    candidate.get("label").and_then(Value::as_str) == Some(label)
                                })
                        })
                        .and_then(|candidate| candidate.get("content"));
                    assign_node_ids(content, canonical_content, stable_task_id, &mut counter, id_map);
                }
            }
        }
    }

    if let Some(response_groups) = cloud_group.get("responseGroups").and_then(Value::as_array) {
        for response_group in response_groups {
            if let Some(prompt) = response_group.get("prompt") {
                // 提示节点没有天然对应物：只有该题组被唯一对齐时才按位置复用。
                let canonical_prompt = canonical_group
                    .and_then(|group| group.get("responseGroups"))
                    .and_then(Value::as_array)
                    .and_then(|arr| arr.first())
                    .and_then(|group| group.get("prompt"));
                assign_node_ids(prompt, canonical_prompt, stable_task_id, &mut counter, id_map);
            }
            if let Some(options) = response_group.get("options").and_then(Value::as_array) {
                for option in options {
                    if let Some(content) = option.get("content") {
                        assign_node_ids(content, None, stable_task_id, &mut counter, id_map);
                    }
                }
            }
        }
    }
}

/// 驼峰 / 短横线 / 空格 → snake_case（枚举别名归一用）。
fn to_snake_case(input: &str) -> String {
    let mut out = String::new();
    for (index, ch) in input.trim().chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == ' ' {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    out
}

/// 把某个字段的值收敛到枚举的 snake_case 写法。
///
/// 未知值**原样保留**（不猜），由后续反序列化如实报错——这是 fail-closed：
/// 宁可让网关拿到一个具体的 schema 错误回给模型，也不要把一个不认识的题型
/// 悄悄改成别的题型。
fn normalize_enum_field(map: &mut Map<String, Value>, key: &str, aliases: &[(&str, &str)]) {
    let Some(raw) = map.get(key).and_then(Value::as_str) else {
        return;
    };
    let snake = to_snake_case(raw);
    let resolved = aliases
        .iter()
        .find(|(from, _)| *from == snake.as_str())
        .map(|(_, to)| (*to).to_string())
        .unwrap_or(snake);
    map.insert(key.to_string(), Value::String(resolved));
}

/// 保留白名单键，其余剥离。
fn retain_keys(map: &mut Map<String, Value>, allowed: &[&str]) {
    map.retain(|key, _| allowed.contains(&key.as_str()));
}

/// 补齐来源锚点的**后端**字段。
///
/// `extractionMode` 与 `sourceHash` 属于后端登记的事实，模型无从得知：让它们缺失
/// 直接导致 `deny_unknown_fields` / 必填字段缺失而拒掉整份候选。这里按后端身份补上，
/// 并剥掉契约外的键（模型多写一个字段不该烧掉整次识别）。
fn normalize_source_anchor(anchor: &mut Value, identity: &CloudAuthoringIdentity<'_>) {
    let Some(map) = anchor.as_object_mut() else {
        return;
    };
    let file_id = map
        .get("sourceFileId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| identity.source_file_id.to_string());
    map.insert("sourceFileId".to_string(), json!(file_id));
    let page_index = map.get("pageIndex").and_then(Value::as_i64).unwrap_or(0);
    map.insert("pageIndex".to_string(), json!(page_index));
    let node_ids = map
        .get("nodeIds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    map.insert("nodeIds".to_string(), json!(node_ids));
    if map
        .get("extractionMode")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        map.insert(
            "extractionMode".to_string(),
            json!(identity.extraction_mode),
        );
    }
    if map
        .get("sourceHash")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        map.insert("sourceHash".to_string(), json!(identity.source_sha256));
    }
    retain_keys(
        map,
        &[
            "sourceFileId",
            "pageIndex",
            "nodeIds",
            "bbox",
            "nativeBBox",
            "displayBBox",
            "pdfToDisplay",
            "charRange",
            "ooxmlPath",
            "relationshipId",
            "extractionMode",
            "sourceHash",
            "variants",
        ],
    );
}

/// 递归补齐稿件里所有来源锚点。
fn sanitize_anchors(value: &mut Value, identity: &CloudAuthoringIdentity<'_>) {
    match value {
        Value::Object(map) => {
            for key in ["sourceAnchors", "evidenceAnchors"] {
                if let Some(Value::Array(items)) = map.get_mut(key) {
                    for item in items.iter_mut() {
                        normalize_source_anchor(item, identity);
                    }
                }
            }
            if let Some(anchor) = map.get_mut("sourceAnchor") {
                normalize_source_anchor(anchor, identity);
            }
            for (_, child) in map.iter_mut() {
                sanitize_anchors(child, identity);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                sanitize_anchors(item, identity);
            }
        }
        _ => {}
    }
}

/// 内容节点的后端字段：`sourceAnchors` 缺省为空数组，`provenanceStatus` 一律由后端定为
/// `source`（模型转写的是原文件内容；模型写什么来源标记都不采信——它无权声称
/// `user_edited`，那会影响人工保护判定）。子节点递归处理。
fn fill_content_node_defaults(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items.iter_mut() {
                fill_content_node_defaults(item);
            }
        }
        Value::Object(map) => {
            if map.get("type").map(Value::is_string).unwrap_or(false) {
                if !map.get("sourceAnchors").map(Value::is_array).unwrap_or(false) {
                    map.insert("sourceAnchors".to_string(), json!([]));
                }
                map.insert("provenanceStatus".to_string(), json!("source"));
            }
            for key in ["children", "items", "rows", "cells", "caption"] {
                if let Some(child) = map.get_mut(key) {
                    fill_content_node_defaults(child);
                }
            }
        }
        _ => {}
    }
}

fn ensure_source_anchors(map: &mut Map<String, Value>) {
    if !map.get("sourceAnchors").map(Value::is_array).unwrap_or(false) {
        map.insert("sourceAnchors".to_string(), json!([]));
    }
}

/// 补齐**后端拥有**、模型被明确告知不要输出的字段。
///
/// 输出契约告诉模型：`sourceAnchors` 可选、`provenanceStatus` 不许写。那么照契约回复的
/// 模型必然缺这两个字段——而 `IeltsAuthoringIRV2` 把它们定为必填。不在这里补，照做的
/// 模型就会在 finalize 被整份拒绝。只补这两类后端字段，**不补任何内容字段**。
fn fill_backend_owned_defaults(draft: &mut Value) {
    let Some(document) = draft.as_object_mut() else {
        return;
    };
    if let Some(passage) = document.get_mut("passage").and_then(Value::as_object_mut) {
        ensure_source_anchors(passage);
        if let Some(content) = passage.get_mut("content") {
            fill_content_node_defaults(content);
        }
    }
    if let Some(groups) = document.get_mut("taskGroups").and_then(Value::as_array_mut) {
        for group in groups.iter_mut() {
            let Some(group_map) = group.as_object_mut() else {
                continue;
            };
            for key in ["instructions", "stimulus"] {
                if let Some(nodes) = group_map.get_mut(key) {
                    fill_content_node_defaults(nodes);
                }
            }
            if let Some(bank) = group_map.get_mut("optionBank").and_then(Value::as_object_mut) {
                ensure_source_anchors(bank);
                if let Some(title) = bank.get_mut("title") {
                    fill_content_node_defaults(title);
                }
                if let Some(options) = bank.get_mut("options").and_then(Value::as_array_mut) {
                    for option in options.iter_mut().filter_map(Value::as_object_mut) {
                        ensure_source_anchors(option);
                        option.remove("provenanceStatus");
                        if let Some(content) = option.get_mut("content") {
                            fill_content_node_defaults(content);
                        }
                    }
                }
            }
            if let Some(response_groups) =
                group_map.get_mut("responseGroups").and_then(Value::as_array_mut)
            {
                for response in response_groups.iter_mut().filter_map(Value::as_object_mut) {
                    ensure_source_anchors(response);
                    if let Some(prompt) = response.get_mut("prompt") {
                        fill_content_node_defaults(prompt);
                    }
                    if let Some(options) = response.get_mut("options").and_then(Value::as_array_mut)
                    {
                        for option in options.iter_mut().filter_map(Value::as_object_mut) {
                            ensure_source_anchors(option);
                            option.remove("provenanceStatus");
                            if let Some(content) = option.get_mut("content") {
                                fill_content_node_defaults(content);
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(slots) = document.get_mut("answerSlots").and_then(Value::as_object_mut) {
        for slot in slots.values_mut().filter_map(Value::as_object_mut) {
            ensure_source_anchors(slot);
            slot.remove("provenanceStatus");
        }
    }
}

/// 把模型草稿收敛到 `IeltsAuthoringIRV2` 的**精确**形状。
///
/// 只做三件事：枚举别名归一、锚点补后端字段、剥掉契约外的键。**不动内容**——
/// 文本、选项、答案一律原样搬运，缺失就保持缺失（由解析如实报错）。
fn sanitize_cloud_authoring_draft(draft: &mut Value, identity: &CloudAuthoringIdentity<'_>) {
    sanitize_anchors(draft, identity);
    fill_backend_owned_defaults(draft);

    let Some(document) = draft.as_object_mut() else {
        return;
    };

    if let Some(groups) = document.get_mut("taskGroups").and_then(Value::as_array_mut) {
        for group in groups.iter_mut() {
            let Some(group_map) = group.as_object_mut() else {
                continue;
            };
            normalize_enum_field(group_map, "taskType", &[]);
            if let Some(bank) = group_map.get_mut("optionBank").and_then(Value::as_object_mut) {
                normalize_enum_field(bank, "scope", &[]);
                retain_keys(
                    bank,
                    &[
                        "optionBankId",
                        "scope",
                        "title",
                        "options",
                        "allowReuse",
                        "sourceAnchors",
                    ],
                );
                if let Some(options) = bank.get_mut("options").and_then(Value::as_array_mut) {
                    for option in options.iter_mut() {
                        let Some(option_map) = option.as_object_mut() else {
                            continue;
                        };
                        retain_keys(
                            option_map,
                            &["optionId", "label", "content", "sourceAnchors", "provenanceStatus"],
                        );
                    }
                }
            }
            if let Some(response_groups) =
                group_map.get_mut("responseGroups").and_then(Value::as_array_mut)
            {
                for response_group in response_groups.iter_mut() {
                    let Some(response_map) = response_group.as_object_mut() else {
                        continue;
                    };
                    normalize_enum_field(response_map, "kind", &[]);
                    normalize_enum_field(response_map, "assignment", &[]);
                    normalize_enum_field(response_map, "scoringPolicy", &[]);
                    normalize_enum_field(response_map, "duplicatePolicy", &[]);
                    retain_keys(
                        response_map,
                        &[
                            "responseGroupId",
                            "kind",
                            "prompt",
                            "slotIds",
                            "options",
                            "optionBankRef",
                            "cardinality",
                            "assignment",
                            "scoringPolicy",
                            "duplicatePolicy",
                            "allowOptionReuse",
                            "sourceAnchors",
                        ],
                    );
                }
            }
        }
    }

    if let Some(slots) = document.get_mut("answerSlots").and_then(Value::as_object_mut) {
        for (_, slot) in slots.iter_mut() {
            let Some(slot_map) = slot.as_object_mut() else {
                continue;
            };
            normalize_enum_field(slot_map, "hostType", &[]);
            normalize_enum_field(slot_map, "interaction", &[("drag_drop", "dragdrop")]);
            normalize_enum_field(slot_map, "participation", &[]);
            if let Some(constraints) = slot_map
                .get_mut("constraints")
                .and_then(Value::as_object_mut)
            {
                retain_keys(
                    constraints,
                    &[
                        "maxWords",
                        "maxNumbers",
                        "maxCharacters",
                        "acceptedOptionLabels",
                    ],
                );
            }
            retain_keys(
                slot_map,
                &[
                    "slotId",
                    "questionNumber",
                    "displayLabel",
                    "hostNodeId",
                    "hostType",
                    "interaction",
                    "participation",
                    "constraints",
                    "sourceAnchors",
                    "provenanceStatus",
                    "confidence",
                ],
            );
        }
    }

    if let Some(keys) = document.get_mut("answerKey").and_then(Value::as_object_mut) {
        for (_, entry) in keys.iter_mut() {
            let Some(entry_map) = entry.as_object_mut() else {
                continue;
            };
            normalize_enum_field(entry_map, "kind", &[]);
            normalize_enum_field(entry_map, "normalization", &[]);
            // 答案值的 assignment 是 `AnswerAssignmentV2`（ordered），与响应组的
            // `AssignmentV2`（ordered_slots）不是同一个枚举，这里如实收敛。
            normalize_enum_field(entry_map, "assignment", &[("ordered_slots", "ordered")]);
            retain_keys(entry_map, &["kind", "values", "normalization", "labels", "assignment"]);
        }
    }
}

/// 模型给出的听力 Part 结构在模型输出里的键（外层，与 `warnings` 同级）。
const CLOUD_LISTENING_PARTS_KEY: &str = "listeningParts";

/// 模型**无权提供**的音频事实字段。
///
/// 音频是内容寻址的资产：`assetId` 就是它的哈希，文件不在模型手上。模型若「顺手」
/// 编一个 `media`，抄进稿件就会得到一条指向不存在资产的引用——候选看着完整，
/// 直到打包/发布时才炸。所以这些字段一律丢弃并留痕，绝不采信。
const CLOUD_LISTENING_FORBIDDEN_FIELDS: [&str; 8] = [
    "media",
    "assets",
    "assetId",
    "assetIds",
    "sha256",
    "mime",
    "durationMs",
    "relativePath",
];

fn question_number_set(value: &Value) -> BTreeSet<u32> {
    value
        .get("expectedQuestionNumbers")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|number| u32::try_from(number).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// 分配一个未被占用的 Part 稳定 ID（`part-1`、`part-2`、…）。
///
/// 从最小可用序号开始取，而不是「已有数量 + 1」：后者在删除过 Part 之后会撞上
/// 仍然存在的 ID。
fn allocate_part_id(used: &mut BTreeSet<String>) -> String {
    let mut ordinal = 1u32;
    loop {
        let candidate = format!("part-{ordinal}");
        if !used.contains(&candidate) {
            used.insert(candidate.clone());
            return candidate;
        }
        ordinal += 1;
    }
}

/// 把模型给的 Part 结构套进 `draft.listening`。
///
/// 分工是明确的：**模型给结构**（标签、题号、题组归属），**用户与后端给音频**。
///
/// - 题号集合与已有 Part 相同 ⇒ 就是同一个 Part：复用它的稳定 `partId`、标签、`cue`
///   与 `media`。这样一次云端候选不会把用户绑好的 Section 音频抹掉。
/// - 真正新增的 Part 才由后端分配身份，且**不带**任何音频。
/// - 模型给的 `media` / 资产字段一律丢弃并逐条留痕。
/// - 模型没被问过 `scope` / `playbackPolicy`：已有就沿用；确实没有才给缺省值，
///   并留下「这是派生/缺省」的说明，让人能看见并改。
fn apply_cloud_listening_parts(
    draft: &mut Value,
    canonical: Option<&Value>,
    raw: &Value,
    identity: &CloudAuthoringIdentity<'_>,
    warnings: &mut Vec<String>,
) {
    if identity.modality != "listening" {
        return;
    }
    let canonical_listening = canonical
        .and_then(|value| value.get("listening"))
        .filter(|value| value.is_object())
        .cloned();
    // `listeningParts` 可能落在模型输出外层（`raw`），也可能已经并进稿（分块合并后）。
    let model_parts = raw
        .get(CLOUD_LISTENING_PARTS_KEY)
        .or_else(|| draft.get(CLOUD_LISTENING_PARTS_KEY))
        .and_then(Value::as_array)
        .cloned();
    if let Some(object) = draft.as_object_mut() {
        object.remove(CLOUD_LISTENING_PARTS_KEY);
    }

    let carry_over = |draft: &mut Value, reason: &str, warnings: &mut Vec<String>| {
        if let Some(listening) = canonical_listening.clone() {
            if let Some(object) = draft.as_object_mut() {
                object.insert("listening".to_string(), listening);
            }
            warnings.push(format!("cloud_listening_parts_missing:{reason}"));
        }
    };

    let Some(model_parts) = model_parts else {
        // 模型没给结构：已有听力结构（连同用户绑的音频）必须原样保留，绝不能丢。
        carry_over(draft, "模型未返回 Part 结构", warnings);
        return;
    };
    if model_parts.is_empty() {
        carry_over(draft, "模型返回了空的 Part 列表", warnings);
        return;
    }

    let canonical_parts: Vec<&Value> = canonical_listening
        .as_ref()
        .and_then(|value| value.get("parts"))
        .and_then(Value::as_array)
        .map(|parts| parts.iter().collect())
        .unwrap_or_default();
    let mut used_part_ids: BTreeSet<String> = canonical_parts
        .iter()
        .filter_map(|part| part.get("partId").and_then(Value::as_str).map(str::to_string))
        .collect();

    let mut parts: Vec<Value> = Vec::new();
    // 已经见过的题号集合。分块识别时同一段可能被不止一块汇报（模型常顺手把别段也
    // 列一遍），而两条覆盖同一批题号的分段会复用同一个稳定 `partId`——于是
    // `listening.parts` 里出现两个 `part-3`，下游任何按 partId 建索引的地方互相覆盖。
    let mut seen_numbers: BTreeSet<Vec<u32>> = BTreeSet::new();
    for (index, model_part) in model_parts.iter().enumerate() {
        let Some(object) = model_part.as_object() else {
            warnings.push(format!("cloud_listening_part_not_object:{}", index + 1));
            continue;
        };
        let display_label = object
            .get("displayLabel")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if display_label.is_empty() {
            warnings.push(format!("cloud_listening_part_label_missing:{}", index + 1));
            continue;
        }
        let numbers = question_number_set(model_part);
        if numbers.is_empty() {
            warnings.push(format!(
                "cloud_listening_part_numbers_missing:{display_label}"
            ));
            continue;
        }
        if !seen_numbers.insert(numbers.iter().copied().collect::<Vec<u32>>()) {
            warnings.push(format!("cloud_listening_part_duplicate:{display_label}"));
            continue;
        }
        let dropped: Vec<&str> = CLOUD_LISTENING_FORBIDDEN_FIELDS
            .iter()
            .copied()
            .filter(|field| object.contains_key(*field))
            .collect();
        if !dropped.is_empty() {
            warnings.push(format!(
                "cloud_listening_part_media_dropped:{display_label}:{}",
                dropped.join(",")
            ));
        }

        let existing = canonical_parts
            .iter()
            .find(|part| question_number_set(part) == numbers);
        let mut part = Map::new();
        match existing {
            Some(existing) => {
                // 已存在的 Part：身份、标签、音频、cue 都是既有事实，模型只贡献题组归属。
                for key in ["partId", "displayLabel", "media", "cue"] {
                    if let Some(value) = existing.get(key) {
                        part.insert(key.to_string(), value.clone());
                    }
                }
                part.insert(
                    "sourceAnchors".to_string(),
                    existing
                        .get("sourceAnchors")
                        .cloned()
                        .unwrap_or_else(|| json!([])),
                );
            }
            None => {
                part.insert(
                    "partId".to_string(),
                    json!(allocate_part_id(&mut used_part_ids)),
                );
                part.insert("displayLabel".to_string(), json!(display_label.clone()));
                part.insert("sourceAnchors".to_string(), json!([]));
            }
        }
        part.insert(
            "expectedQuestionNumbers".to_string(),
            json!(numbers.iter().copied().collect::<Vec<u32>>()),
        );
        // `taskIds` 保持模型给的临时引用，交给统一的引用重写接到后端身份上。
        part.insert(
            "taskIds".to_string(),
            Value::Array(
                object
                    .get("taskIds")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            ),
        );
        parts.push(Value::Object(part));
    }

    if parts.is_empty() {
        carry_over(draft, "模型给的 Part 结构无法套用", warnings);
        return;
    }

    let scope = match canonical_listening
        .as_ref()
        .and_then(|value| value.get("scope"))
    {
        Some(scope) => scope.clone(),
        None => {
            // 模型没被问过 scope。按 Part 数与题号范围如实派生，并留痕让人复核。
            let numbers: BTreeSet<u32> = parts
                .iter()
                .flat_map(|part| question_number_set(part))
                .collect();
            let complete =
                parts.len() == 4 && numbers.len() == 40 && numbers.iter().copied().eq(1..=40);
            let derived = if complete {
                "complete_exam"
            } else {
                "partial_practice"
            };
            warnings.push(format!("cloud_listening_scope_derived:{derived}"));
            json!(derived)
        }
    };
    let playback_policy = match canonical_listening
        .as_ref()
        .and_then(|value| value.get("playbackPolicy"))
    {
        Some(policy) => policy.clone(),
        None => {
            warnings.push("cloud_listening_playback_policy_defaulted".to_string());
            json!({
                "mode": "practice",
                "autoplay": false,
                "allowPause": true,
                "allowSeek": true,
                "allowReplay": true,
                "refreshBehavior": "resume_from_snapshot",
                "crashRecoveryBehavior": "resume_from_snapshot",
                "showCurrentTime": true,
                "showDuration": true
            })
        }
    };

    let mut listening = Map::new();
    listening.insert("scope".to_string(), scope);
    listening.insert("parts".to_string(), Value::Array(parts));
    listening.insert("playbackPolicy".to_string(), playback_policy);
    if let Some(transcript) = canonical_listening
        .as_ref()
        .and_then(|value| value.get("transcript"))
    {
        listening.insert("transcript".to_string(), transcript.clone());
    }
    if let Some(object) = draft.as_object_mut() {
        object.insert("listening".to_string(), Value::Object(listening));
    }
}

/// 把模型输出标准化成一份**完整**的 `IeltsAuthoringIRV2` 值（含后端身份）。
///
/// 输入 `raw` 是模型原始 JSON：`{"authoring": {...}}` 或直接就是稿件对象。
/// 输出 `document` 是**完整富内容**（passage / instructions / stimulus / prompt / 选项库 /
/// responseGroups / answerSlots / answerKey 全部保留），绝不压平成纯文本投影。
pub(crate) fn normalize_cloud_authoring(
    identity: &CloudAuthoringIdentity<'_>,
    canonical: Option<&Value>,
    raw: &Value,
) -> CommandResult<NormalizedCloudAuthoring> {
    let mut warnings: Vec<String> = Vec::new();

    let mut draft = raw
        .get("authoring")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| raw.clone());
    if !draft.is_object() {
        return Err("cloud_authoring_candidate_not_object".to_string());
    }
    // 先做一次契约收敛：枚举别名归一 + 锚点补齐后端来源字段 + 剥掉多余键。
    // 目的是让「无害的写法差异」不要升级成整份候选反序列化失败。
    sanitize_cloud_authoring_draft(&mut draft, identity);

    let mut source_coverage_notes: Vec<String> = raw
        .get("sourceCoverageNotes")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let unresolved_regions: Vec<CloudCandidateUnresolvedRegionV1> = raw
        .get("unresolvedRegions")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| format!("cloud_authoring_unresolved_regions_invalid:{error}"))?
        .unwrap_or_default();
    if let Some(items) = raw.get("warnings").and_then(Value::as_array) {
        warnings.extend(items.iter().filter_map(Value::as_str).map(str::to_string));
    }
    // 分块识别里失败的块：题号如实带出，并成为用户可见的覆盖说明。
    let uncovered_question_numbers: Vec<u32> = raw
        .get("uncoveredQuestionNumbers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|number| u32::try_from(number).ok())
                .collect::<BTreeSet<u32>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default();
    if !uncovered_question_numbers.is_empty() {
        source_coverage_notes.push(format!(
            "云端识别未覆盖第 {} 题（该部分的云端请求失败），这些题只有本地识别结果。",
            format_question_numbers(&uncovered_question_numbers)
        ));
    }
    // Listening 的 Part 结构由模型提供（标签、题号、题组归属），音频事实由后端与
    // 用户提供。这里把它套进 `draft.listening`，好让后面的引用重写统一处理
    // `parts[].taskIds`；音频/资产字段一律丢弃并留痕。
    apply_cloud_listening_parts(&mut draft, canonical, raw, identity, &mut warnings);

    let groups: Vec<Value> = draft
        .get("taskGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if groups.is_empty() {
        // 没有题组就没有可用的完整候选。如实标 `unusable`，不假装识别成功。
        return Ok(NormalizedCloudAuthoring {
            status: ChainStatusV1::Unusable,
            reason_code: Some("cloud_authoring_candidate_no_task_groups".to_string()),
            document: Value::Null,
            id_map: BTreeMap::new(),
            unresolved_references: Vec::new(),
            unresolved_regions,
            source_coverage_notes,
            warnings,
            uncovered_question_numbers,
        });
    }

    // 临时 ID 全集：界定「模型临时空间」与「既有稳定引用」。
    let mut temp_ids: BTreeSet<String> = BTreeSet::new();
    collect_reference_ids(&draft, &mut temp_ids);

    // 已占用的稳定 ID：来自当前权威稿 + 本次已分配的。
    let mut used: BTreeSet<String> = BTreeSet::new();
    if let Some(canonical) = canonical {
        collect_reference_ids(canonical, &mut used);
    }

    let canonical_groups = canonical.map(canonical_group_index).unwrap_or_default();
    let canonical_slots = canonical
        .map(|value| canonical_slot_index(value, &canonical_groups))
        .unwrap_or_default();

    let mut id_map: BTreeMap<String, String> = BTreeMap::new();
    let mut unresolved: BTreeSet<String> = BTreeSet::new();

    // ── 1) 题组身份 ────────────────────────────────────────────────
    // cloud_group_index -> (stable task id 或 None 表示保留临时 ID)
    let mut group_stable: Vec<Option<String>> = Vec::with_capacity(groups.len());
    let mut group_canonical: Vec<Option<usize>> = Vec::with_capacity(groups.len());
    for (index, group) in groups.iter().enumerate() {
        let cloud_task_id = group
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let cloud_type = group.get("taskType").and_then(Value::as_str).unwrap_or("");
        let numbers = cloud_group_numbers(group, &draft);
        match match_task_group(cloud_type, &numbers, &canonical_groups) {
            GroupMatch::Unique(target) => {
                let stable = canonical_groups[target].task_id.clone();
                if !cloud_task_id.is_empty() && cloud_task_id != stable {
                    id_map.insert(cloud_task_id, stable.clone());
                }
                used.insert(stable.clone());
                group_stable.push(Some(stable));
                group_canonical.push(Some(target));
            }
            GroupMatch::New => {
                let preferred = if numbers.is_empty() {
                    format!("cloud-tg-{}", index + 1)
                } else {
                    let list = numbers
                        .iter()
                        .map(|number| number.to_string())
                        .collect::<Vec<_>>()
                        .join("-");
                    format!("cloud-tg-{list}")
                };
                let stable = alloc_stable_id(&preferred, &used);
                used.insert(stable.clone());
                if !cloud_task_id.is_empty() && cloud_task_id != stable {
                    id_map.insert(cloud_task_id, stable.clone());
                }
                group_stable.push(Some(stable));
                group_canonical.push(None);
            }
            GroupMatch::Ambiguous(candidates) => {
                // 歧义**不**静默挑选：题组自身的身份保留在临时空间（随后被记进
                // `unresolved_references`），交给修复回合结合原文继续判断。
                //
                // 但**组内对象照常分配后端 ID**：响应组 / 选项库 / 内容节点没有现成映射
                // 算法，不等于它们要整类降级成人工问题——那是把后端的活推给用户。
                let _ = candidates;
                unresolved.insert(format!("task_group:{cloud_task_id}:ambiguous"));
                let prefix = if cloud_task_id.is_empty() {
                    format!("cloud-tg-{}", index + 1)
                } else {
                    cloud_task_id.clone()
                };
                group_stable.push(Some(prefix));
                group_canonical.push(None);
            }
        }
    }

    // ── 2) 答案槽身份 ──────────────────────────────────────────────
    // 云端槽位引用（key 或 slotId）→ 所属云端题组下标。
    let mut cloud_owner: BTreeMap<String, usize> = BTreeMap::new();
    for (index, group) in groups.iter().enumerate() {
        for reference in cloud_group_slot_refs(group) {
            cloud_owner.entry(reference).or_insert(index);
        }
    }
    let cloud_slots: Map<String, Value> = draft
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    for (key, slot) in &cloud_slots {
        let cloud_slot_id = slot
            .get("slotId")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_string();
        let Some(question_number) = slot.get("questionNumber").and_then(Value::as_u64) else {
            unresolved.insert(format!("slot:{key}:missing_question_number"));
            continue;
        };
        let owner = cloud_owner
            .get(key)
            .or_else(|| cloud_owner.get(&cloud_slot_id))
            .copied();
        let owner_task_id = owner
            .and_then(|index| group_stable.get(index).cloned().flatten());
        let candidates: Vec<&CanonicalSlotIndex> = canonical_slots
            .iter()
            .filter(|candidate| candidate.question_number == question_number as u32)
            .filter(|candidate| match &owner_task_id {
                Some(task_id) => candidate.task_id.as_deref() == Some(task_id.as_str()),
                None => true,
            })
            .collect();
        let stable = match candidates.len() {
            1 => Some(candidates[0].key.clone()),
            0 => None,
            _ => {
                unresolved.insert(format!("slot:{key}:ambiguous"));
                continue;
            }
        };
        let stable = stable.unwrap_or_else(|| {
            let preferred = format!("q{question_number}");
            alloc_stable_id(&preferred, &used)
        });
        used.insert(stable.clone());
        if key != &stable {
            id_map.insert(key.clone(), stable.clone());
        }
        if cloud_slot_id != stable {
            id_map.insert(cloud_slot_id, stable);
        }
    }

    // ── 3) 响应组 / 选项库身份 ─────────────────────────────────────
    for (index, group) in groups.iter().enumerate() {
        let Some(stable_task_id) = group_stable[index].clone() else {
            continue;
        };
        let canonical_group = group_canonical[index].map(|target| &canonical_groups[target]);

        if let Some(bank) = group.get("optionBank") {
            if let Some(cloud_bank_id) = bank.get("optionBankId").and_then(Value::as_str) {
                let stable = canonical_group
                    .and_then(|group| group.option_bank_id.clone())
                    .unwrap_or_else(|| format!("{stable_task_id}-options"));
                used.insert(stable.clone());
                if cloud_bank_id != stable {
                    id_map.insert(cloud_bank_id.to_string(), stable);
                }
            }
        }

        if let Some(response_groups) = group.get("responseGroups").and_then(Value::as_array) {
            for (position, response_group) in response_groups.iter().enumerate() {
                let Some(cloud_id) = response_group
                    .get("responseGroupId")
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                // 该响应组覆盖的槽位（已重写为稳定 ID 的话优先用稳定集合）。
                let slots: BTreeSet<String> = response_group
                    .get("slotIds")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(|value| {
                                id_map
                                    .get(value)
                                    .cloned()
                                    .unwrap_or_else(|| value.to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let stable = canonical_group
                    .and_then(|group| {
                        let matches: Vec<&(String, BTreeSet<String>)> = group
                            .response_groups
                            .iter()
                            .filter(|(_, slot_ids)| !slot_ids.is_empty() && *slot_ids == slots)
                            .collect();
                        if matches.len() == 1 {
                            Some(matches[0].0.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| format!("{stable_task_id}-rg-{}", position + 1));
                used.insert(stable.clone());
                if cloud_id != stable {
                    id_map.insert(cloud_id.to_string(), stable);
                }
            }
        }
    }

    // ── 4) 题组内部对象（内容节点 / 选项）──────────────────────────
    for (index, group) in groups.iter().enumerate() {
        let Some(stable_task_id) = group_stable[index].clone() else {
            continue;
        };
        let canonical_group = group_canonical[index].map(|target| {
            canonical
                .and_then(|value| value.get("taskGroups"))
                .and_then(Value::as_array)
                .and_then(|arr| arr.get(target))
        });
        assign_group_inner_ids(
            group,
            canonical_group.flatten(),
            &stable_task_id,
            &mut id_map,
        );
    }

    // ── 5) 引用重写（全有或全无）──────────────────────────────────
    let outcome = rewrite_authoring_references(&mut draft, &id_map, &temp_ids);
    if !outcome.conflicts.is_empty() {
        return Err(format!(
            "cloud_authoring_reference_conflict:{}",
            outcome.conflicts.join(";")
        ));
    }
    unresolved.extend(outcome.unmapped.iter().cloned());

    // ── 6) 组装后端字段 ────────────────────────────────────────────
    let mut document = Map::new();
    document.insert(
        "schemaVersion".to_string(),
        json!(crate::schema::ielts_authoring_v2::IELTS_AUTHORING_IR_V2_SCHEMA_VERSION),
    );
    document.insert("jobId".to_string(), json!(identity.job_id));
    document.insert("exam".to_string(), identity.exam.clone());
    document.insert("modality".to_string(), json!(identity.modality));
    if let Some(passage) = draft.get("passage") {
        if passage.is_object() {
            document.insert("passage".to_string(), passage.clone());
        }
    }
    // 听力结构同理：必须取**重写之后**的 `draft.listening`，否则 `parts[].taskIds`
    // 还是模型的临时引用，候选看着完整、其实一个题组都没接上。
    if let Some(listening) = draft.get("listening") {
        if listening.is_object() {
            document.insert("listening".to_string(), listening.clone());
        }
    }

    // 重写之后再取题组：`groups` 是重写**前**的克隆，里面的 taskId 仍是临时 ID，
    // 直接拿它装配会产出「身份没接上」的候选（看着完整、其实全是临时引用）。
    let rewritten_groups: Vec<Value> = draft
        .get("taskGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut sanitized_groups: Vec<Value> = Vec::with_capacity(rewritten_groups.len());
    for (index, group) in rewritten_groups.iter().enumerate() {
        let mut next = Map::new();
        for key in [
            "taskId",
            "displayRange",
            "taskType",
            "instructions",
            "stimulus",
            "optionBank",
            "responseGroups",
            "sourceAnchors",
        ] {
            if let Some(value) = group.get(key) {
                next.insert(key.to_string(), value.clone());
            }
        }
        // 模型漏填 taskId 时用后端分配的身份补上，而不是留下空串。
        let has_task_id = next
            .get("taskId")
            .and_then(Value::as_str)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        if !has_task_id {
            if let Some(Some(stable)) = group_stable.get(index) {
                next.insert("taskId".to_string(), json!(stable));
            }
        }
        next.entry("instructions".to_string())
            .or_insert_with(|| json!([]));
        next.entry("responseGroups".to_string())
            .or_insert_with(|| json!([]));
        next.entry("sourceAnchors".to_string())
            .or_insert_with(|| json!([]));
        // 后端派生 / 后端裁定，模型无权填写。
        let mut group_value = Value::Object(next);
        derive_instruction_signature_for_group(&mut group_value);
        let numbers: Vec<u32> = group_value
            .get("displayRange")
            .map(expand_question_numbers)
            .unwrap_or_default();
        let task_type = group_value
            .get("taskType")
            .cloned()
            .unwrap_or_else(|| json!("short_answer"));
        let needs_signature = group_value.get("instructionSignature").is_none();
        if let Some(object) = group_value.as_object_mut() {
            if needs_signature {
                // 派生器无从推断（instructions 为空或无文本）时给一个**显式 confidence=0**
                // 的最小签名：缺字段会让整份候选反序列化失败，那是把「识别不完整」升级成
                // 「候选完全不可用」，代价远大于一条低置信度签名。
                object.insert(
                    "instructionSignature".to_string(),
                    json!({
                        "normalizedText": "",
                        "taskType": task_type,
                        "expectedQuestionNumbers": numbers,
                        "expectedSlotCount": numbers.len(),
                        "evidenceAnchors": [],
                        "confidence": 0.0
                    }),
                );
            }
            object.insert(
                "quality".to_string(),
                json!({"score": 0.0, "sourceCoverage": 0.0, "hardFailures": []}),
            );
            object.insert("reviewState".to_string(), json!("unreviewed"));
        }
        sanitized_groups.push(group_value);
    }
    document.insert("taskGroups".to_string(), json!(sanitized_groups));

    document.insert(
        "answerSlots".to_string(),
        draft.get("answerSlots").cloned().unwrap_or_else(|| json!({})),
    );
    document.insert(
        "answerKey".to_string(),
        draft.get("answerKey").cloned().unwrap_or_else(|| json!({})),
    );
    // 资源只引用后端已登记的资源目录项；候选不臆造文件地址，也不改写权威稿的资源表。
    if raw.get("assets").is_some() {
        warnings.push("cloud_authoring_candidate_assets_not_applied".to_string());
    }
    document.insert("assets".to_string(), json!([]));
    document.insert(
        "sourceDocumentId".to_string(),
        json!(identity.source_document_id),
    );
    document.insert(
        "quality".to_string(),
        placeholder_quality_report(identity.generated_at),
    );
    document.insert(
        "audit".to_string(),
        json!({
            "revision": 0,
            "source": "auto_extract",
            "humanVerified": false,
            "llmUsed": true,
            "updatedAt": identity.generated_at,
            "notes": ["云端完整候选：由后端标准化，未写入权威稿"],
        }),
    );

    let status = if unresolved.is_empty() && uncovered_question_numbers.is_empty() {
        ChainStatusV1::Succeeded
    } else {
        ChainStatusV1::Partial
    };
    let reason_code = if !uncovered_question_numbers.is_empty() {
        Some("cloud_authoring_candidate_chunks_failed".to_string())
    } else if unresolved.is_empty() {
        None
    } else {
        Some("cloud_authoring_candidate_unresolved_references".to_string())
    };

    Ok(NormalizedCloudAuthoring {
        status,
        reason_code,
        document: Value::Object(document),
        id_map,
        unresolved_references: unresolved.into_iter().collect(),
        unresolved_regions,
        source_coverage_notes,
        warnings,
        uncovered_question_numbers,
    })
}

/// 把标准化结果装配成可落盘的 [`CloudAuthoringCandidateV1`]。
///
/// 后端身份在这里写进契约：`jobId` / `batchId` / `sourceFileId` / `sourceSha256` /
/// `baseEditVersion` 只从 [`CloudAuthoringIdentity`] 取，模型输出里的同名字段一律不采信。
pub(crate) fn cloud_authoring_candidate_from_normalized(
    identity: &CloudAuthoringIdentity<'_>,
    normalized: NormalizedCloudAuthoring,
) -> CommandResult<CloudAuthoringCandidateV1> {
    if normalized.document.is_null() {
        // 没有可用候选：契约要求 `authoring` 是完整稿件，构造不出就如实失败，
        // 绝不拿一个空壳冒充「完整候选」。
        return Err(normalized
            .reason_code
            .clone()
            .unwrap_or_else(|| "cloud_authoring_candidate_unusable".to_string()));
    }
    let authoring = serde_json::from_value(normalized.document)
        .map_err(|error| format!("cloud_authoring_candidate_schema_invalid:{error}"))?;
    Ok(CloudAuthoringCandidateV1 {
        schema_version: CLOUD_AUTHORING_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
        batch_id: identity.batch_id.to_string(),
        item_id: identity.item_id.to_string(),
        job_id: identity.job_id.to_string(),
        source_file_id: identity.source_file_id.to_string(),
        source_sha256: identity.source_sha256.to_string(),
        base_edit_version: identity.base_edit_version,
        generated_at: identity.generated_at.to_string(),
        status: normalized.status,
        reason_code: normalized.reason_code,
        authoring,
        id_map: normalized.id_map,
        unresolved_references: normalized.unresolved_references,
        unresolved_regions: normalized.unresolved_regions,
        source_coverage_notes: normalized.source_coverage_notes,
        warnings: normalized.warnings,
    })
}

/// 候选的**占位**质量块：明确标注「尚未评估」，绝不伪装成已通过。
///
/// 真实评估由调用方在拿到 `root` 后调用 `refresh_quality_report` 覆盖（见 `auto_pipeline`）。
fn placeholder_quality_report(generated_at: &str) -> Value {
    json!({
        "schemaVersion": "QualityReportV2",
        "state": "review_required",
        "documentScore": 0.0,
        "sourceCoverage": 0.0,
        "coverageLedger": [],
        "coverageStatus": {
            "physicalShadow": "missing",
            "complete": false,
            "significantSourceNodeCount": 0,
            "explainedSourceNodeCount": 0,
            "unassignedSourceNodeIds": []
        },
        "compilerProbes": {
            "v2Runtime": {
                "status": "failed",
                "schemaVersion": "ReadingExamSourceV2",
                "issueCodes": ["CLOUD_CANDIDATE_QUALITY_NOT_EVALUATED"],
                "details": ["候选尚未评估；调用方须用 refresh_quality_report 覆盖"]
            },
            "v1Compatibility": {
                "status": "failed",
                "schemaVersion": "ReadingExamSourceV1",
                "issueCodes": ["CLOUD_CANDIDATE_QUALITY_NOT_EVALUATED"],
                "details": ["候选尚未评估；调用方须用 refresh_quality_report 覆盖"]
            }
        },
        "taskScores": {},
        "hardFailures": [],
        "issues": [],
        "metrics": {},
        "evaluatedAt": generated_at,
        "evaluatorVersion": "cloud_authoring_candidate_placeholder"
    })
}

// ─────────────────────────────────────────────────────────────────────
// 云端候选分块（S3）
//
// 整卷一次请求在真实网关上会超时（212 KB PDF 在 120 s 预算内出不完整份输出）。
// 分块计划来自**原文件自己声明的题号**（`Questions 14-26`），绝不来自本地识别的结论——
// 云端必须保持独立，否则「本地漏了一段、云端也跟着漏」。每块的临时 id 在合并前加上
// 块命名空间：所有块都会用 cloud-tg-1 / cloud-rg-1 这样的临时 id，直接合并会让
// 引用重写把两个题组接到同一组 id 上。
// ─────────────────────────────────────────────────────────────────────

/// 一块的题量上限：没有 passage 级声明时，相邻题组声明合并到不超过这个数。
const MAX_CHUNK_QUESTIONS: usize = 14;

/// 一个候选分块：一段原文件声明的题号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateChunk {
    pub label: String,
    pub question_numbers: Vec<u32>,
}

impl CandidateChunk {
    pub(crate) fn new(mut question_numbers: Vec<u32>) -> Self {
        question_numbers.sort_unstable();
        question_numbers.dedup();
        let label = format!("Questions {}", format_question_numbers(&question_numbers));
        Self {
            label,
            question_numbers,
        }
    }

    pub(crate) fn as_value(&self) -> Value {
        json!({"label": self.label, "questionNumbers": self.question_numbers})
    }
}

/// `[1,2,3,5]` → `1-3, 5`。
pub(crate) fn format_question_numbers(numbers: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut index = 0;
    while index < numbers.len() {
        let start = numbers[index];
        let mut end = start;
        while index + 1 < numbers.len() && numbers[index + 1] == end + 1 {
            index += 1;
            end = numbers[index];
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        index += 1;
    }
    parts.join(", ")
}

/// 从原文件文本规划候选分块。
///
/// 1. 取出全部 `Questions …` 声明（与来源覆盖检查同一套解析）；
/// 2. 被别的声明完全包含的声明丢掉（`Questions 1-5` 在 `Questions 1-13` 之内）；
///    部分重叠的合并——剩下的就是 passage 级（或题组级）的不相交块；
/// 3. 相邻小块按顺序合并，直到超过 [`MAX_CHUNK_QUESTIONS`]；
/// 4. 最后不足两块 ⇒ 返回空计划，调用方回到一次整卷请求。
pub(crate) fn plan_candidate_chunks(source_text: &str) -> Vec<CandidateChunk> {
    let blocks: Vec<BTreeSet<u32>> = crate::ielts_grammar::source_coverage::declared_question_blocks(source_text)
        .into_iter()
        .map(|numbers| numbers.into_iter().collect::<BTreeSet<u32>>())
        .collect();
    // 保留极大块（不被任何更大的块严格包含），相同块去重。
    let mut maximal: Vec<BTreeSet<u32>> = Vec::new();
    for block in &blocks {
        let contained = blocks
            .iter()
            .any(|other| other.len() > block.len() && block.is_subset(other));
        if !contained && !maximal.contains(block) {
            maximal.push(block.clone());
        }
    }
    maximal.sort_by_key(|block| block.iter().next().copied().unwrap_or(0));
    // 部分重叠（区间相交）的合并成一块，保证各块不相交。
    let mut disjoint: Vec<BTreeSet<u32>> = Vec::new();
    for block in maximal {
        let first = block.iter().next().copied().unwrap_or(0);
        match disjoint.last_mut() {
            Some(last) if last.iter().next_back().copied().unwrap_or(0) >= first => {
                last.extend(block);
            }
            _ => disjoint.push(block),
        }
    }
    // 相邻小块按顺序打包。
    let mut packed: Vec<BTreeSet<u32>> = Vec::new();
    for block in disjoint {
        match packed.last_mut() {
            Some(last) if last.len() + block.len() <= MAX_CHUNK_QUESTIONS => last.extend(block),
            _ => packed.push(block),
        }
    }
    if packed.len() < 2 {
        return Vec::new();
    }
    packed
        .into_iter()
        .map(|block| CandidateChunk::new(block.into_iter().collect()))
        .collect()
}

/// 给一块的临时 id 加上块命名空间（`c2-cloud-tg-1`），**全有或全无**地重写所有引用。
///
/// 资源引用（`assetId` 等）不加前缀：那是后端登记的稳定 id，不属于模型的临时空间。
fn namespace_chunk_ids(chunk_output: &mut Value, prefix: &str) -> CommandResult<()> {
    let mut temp_ids: BTreeSet<String> = BTreeSet::new();
    collect_reference_ids(chunk_output, &mut temp_ids);
    let mut asset_ids: BTreeSet<String> = BTreeSet::new();
    collect_asset_ids(chunk_output, &mut asset_ids);
    let id_map: BTreeMap<String, String> = temp_ids
        .iter()
        .filter(|id| !asset_ids.contains(*id) && !id.is_empty())
        .map(|id| (id.clone(), format!("{prefix}{id}")))
        .collect();
    let outcome = rewrite_authoring_references(chunk_output, &id_map, &temp_ids);
    if !outcome.conflicts.is_empty() {
        return Err(format!(
            "cloud_authoring_chunk_namespace_conflict:{}",
            outcome.conflicts.join(";")
        ));
    }
    // listeningParts[].taskIds 也在重写范围内（`taskIds` 按字段名递归处理）。
    Ok(())
}

fn collect_asset_ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if matches!(key.as_str(), "assetId" | "visualFallbackAssetId" | "assetRef") {
                    if let Some(text) = child.as_str() {
                        out.insert(text.to_string());
                    }
                } else if key == "assetIds" {
                    for item in child.as_array().into_iter().flatten() {
                        if let Some(text) = item.as_str() {
                            out.insert(text.to_string());
                        }
                    }
                }
                collect_asset_ids(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_asset_ids(item, out);
            }
        }
        _ => {}
    }
}

/// 把各块的模型输出合并成一份整卷候选原始 JSON（交给 `normalize_cloud_authoring`）。
///
/// - 成功的块：先加块命名空间，再拼接 taskGroups / answerSlots / answerKey / 说明类数组；
/// - 失败的块：错误进 `warnings`，题号进 `uncoveredQuestionNumbers`（标准化据此给出
///   `Partial` 与用户可见的覆盖说明）；
/// - 全部失败 ⇒ `Err`（带每块的原因），不拿空壳冒充候选。
pub(crate) fn merge_candidate_chunks(
    results: Vec<(CandidateChunk, CommandResult<Value>)>,
) -> CommandResult<Value> {
    let mut task_groups: Vec<Value> = Vec::new();
    let mut answer_slots = Map::new();
    let mut answer_key = Map::new();
    let mut unresolved_regions: Vec<Value> = Vec::new();
    let mut coverage_notes: Vec<Value> = Vec::new();
    let mut warnings: Vec<Value> = Vec::new();
    let mut listening_parts: Vec<Value> = Vec::new();
    let mut uncovered: BTreeSet<u32> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();
    let mut chunks_meta: Vec<Value> = Vec::new();
    let mut succeeded = 0usize;

    for (index, (chunk, result)) in results.into_iter().enumerate() {
        let prefix = format!("c{}-", index + 1);
        let mut output = match result {
            Ok(value) => value
                .get("authoring")
                .filter(|inner| inner.is_object())
                .cloned()
                .map(|mut inner| {
                    // 说明类字段可能在外层：一并带进来。
                    for key in ["unresolvedRegions", "sourceCoverageNotes", "warnings", "listeningParts"] {
                        if let Some(extra) = value.get(key) {
                            inner[key] = extra.clone();
                        }
                    }
                    inner
                })
                .unwrap_or(value),
            Err(error) => {
                uncovered.extend(chunk.question_numbers.iter().copied());
                warnings.push(json!(format!("cloud_candidate_chunk_failed:{}:{error}", chunk.label)));
                failures.push(format!("{}:{error}", chunk.label));
                chunks_meta.push(json!({"label": chunk.label, "questionNumbers": chunk.question_numbers, "ok": false, "error": error}));
                continue;
            }
        };
        if let Err(error) = namespace_chunk_ids(&mut output, &prefix) {
            uncovered.extend(chunk.question_numbers.iter().copied());
            warnings.push(json!(format!("cloud_candidate_chunk_failed:{}:{error}", chunk.label)));
            failures.push(format!("{}:{error}", chunk.label));
            chunks_meta.push(json!({"label": chunk.label, "questionNumbers": chunk.question_numbers, "ok": false, "error": error}));
            continue;
        }
        succeeded += 1;
        chunks_meta.push(json!({"label": chunk.label, "questionNumbers": chunk.question_numbers, "ok": true}));
        task_groups.extend(output.get("taskGroups").and_then(Value::as_array).cloned().unwrap_or_default());
        if let Some(slots) = output.get("answerSlots").and_then(Value::as_object) {
            answer_slots.extend(slots.clone());
        }
        if let Some(keys) = output.get("answerKey").and_then(Value::as_object) {
            answer_key.extend(keys.clone());
        }
        for (key, target) in [
            ("unresolvedRegions", &mut unresolved_regions),
            ("sourceCoverageNotes", &mut coverage_notes),
            ("warnings", &mut warnings),
            ("listeningParts", &mut listening_parts),
        ] {
            target.extend(output.get(key).and_then(Value::as_array).cloned().unwrap_or_default());
        }
    }
    if succeeded == 0 {
        return Err(format!(
            "cloud_authoring_candidate_all_chunks_failed:{}",
            failures.join(" | ")
        ));
    }
    let mut merged = json!({
        "taskGroups": task_groups,
        "answerSlots": answer_slots,
        "answerKey": answer_key,
        "unresolvedRegions": unresolved_regions,
        "sourceCoverageNotes": coverage_notes,
        "warnings": warnings,
        "uncoveredQuestionNumbers": uncovered.into_iter().collect::<Vec<u32>>(),
        "cloudChunks": chunks_meta,
    });
    if !listening_parts.is_empty() {
        merged["listeningParts"] = json!(listening_parts);
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authoring_fixture() -> Value {
        json!({
            "taskGroups": [{
                "taskId": "task-1",
                "displayRange": {"kind": "range", "start": 14, "end": 15},
                "taskType": "sentence_completion",
                "instructions": [{"type":"text","id":"t0","text":"Complete the sentences."}],
                "responseGroups": [{
                    "responseGroupId": "rg-1",
                    "kind": "text_entry",
                    "slotIds": ["slot-14", "slot-15"],
                    "cardinality": {"min":1,"max":1,"exact":1},
                    "assignment": "per_slot",
                    "allowOptionReuse": false
                }],
                "sourceAnchors": [],
                "instructionSignature": {"confidence": 0.9},
                "quality": {"score": 1.0, "sourceCoverage": 1.0, "hardFailures": []},
                "reviewState": "unreviewed"
            }],
            "answerSlots": {
                "slot-14": {
                    "slotId": "slot-14", "questionNumber": 14, "displayLabel": "14",
                    "hostType": "prompt", "interaction": "text", "participation": "scoring",
                    "sourceAnchors": [{"sourceFileId":"file-1","pageIndex":0,"nodeIds":["n1"],"extractionMode":"pdf_native","sourceHash":"a"}],
                    "confidence": 0.9
                },
                "slot-15": {
                    "slotId": "slot-15", "questionNumber": 15, "displayLabel": "15",
                    "hostType": "prompt", "interaction": "text", "participation": "scoring",
                    "sourceAnchors": [],
                    "confidence": 0.9
                }
            },
            "answerKey": {
                "slot-14": {"kind":"text","values":["stencilling"]},
                "slot-15": {"kind":"text","values":["books"]}
            },
            "assets": [{"assetId":"img-1","sha256":"b","mime":"image/png"}]
        })
    }

    #[test]
    fn local_candidate_maps_slots_and_evidence_honestly() {
        let candidate = local_candidate_from_authoring(
            &authoring_fixture(),
            "batch-1",
            "item-1",
            "job-1",
            "file-1",
            &"a".repeat(64),
            4,
        );
        assert_eq!(candidate.status, ChainStatusV1::Succeeded);
        assert_eq!(candidate.task_groups.len(), 1);
        assert_eq!(candidate.slots.len(), 2);
        assert_eq!(candidate.slots[0].task_id, "task-1");
        assert_eq!(candidate.slots[0].response_group_id, "rg-1");
        // 「有答案」与「有原文证据」必须分开：slot-15 有答案但没有 anchors。
        assert!(candidate.slots[0].has_source_evidence);
        assert!(!candidate.slots[1].has_source_evidence);
        assert_eq!(candidate.assets.len(), 1);
    }

    #[test]
    fn cloud_candidate_salvages_good_groups_and_drops_invalid_ones() {
        let raw = json!({
            "groups": [
                {
                    "taskId": "task-1",
                    "range": {"kind": "range", "start": 14, "end": 14},
                    "taskType": "single_choice",
                    "instructionsText": "Choose the correct letter.",
                    "responseGroups": [{"responseGroupId":"rg-1","kind":"choice","slotIds":["slot-14"]}],
                    "optionBank": {"optionBankId":"ob-1","options":[{"optionId":"o1","label":"A","text":"alpha"}],"allowReuse":false},
                    "slots": [{
                        "slotId":"slot-14","questionNumber":14,"displayLabel":"14",
                        "interaction":"radio","hostType":"prompt","participation":"scoring",
                        "answer":{"kind":"option","labels":["A"],"assignment":"per_slot"},
                        "evidence":[{"pageIndex":2,"quote":"14 alpha"}]
                    }],
                    "confidence": 0.8
                },
                {
                    "taskId": "task-2",
                    "range": {"kind": "range", "start": 15, "end": 15},
                    "taskType": "totally_made_up_type",
                    "responseGroups": [],
                    "slots": [],
                    "confidence": 0.5
                }
            ]
        });
        let candidate = cloud_candidate_from_value(
            &raw, "batch-1", "item-1", "job-1", "file-1", &"a".repeat(64), 4,
        );
        assert_eq!(candidate.status, ChainStatusV1::Partial);
        assert_eq!(candidate.task_groups.len(), 1);
        assert_eq!(candidate.task_groups[0].task_id, "task-1");
        assert!(candidate.slots[0].has_source_evidence);
        let salvage = candidate.salvage.expect("salvage report required");
        assert_eq!(salvage.total_groups, 2);
        assert_eq!(salvage.kept_groups, 1);
        assert_eq!(salvage.dropped_groups, 1);
        assert!(salvage.dropped_reasons.iter().any(|r| r.contains("task_type_invalid")));
        assert_eq!(candidate.reason_code.as_deref(), Some(reason::SALVAGE_PARTIAL));
    }

    #[test]
    fn cloud_candidate_is_unusable_when_every_group_is_invalid() {
        let raw = json!({"groups": [{"taskId": "task-1", "range": {"kind":"set","values":[1]},
            "taskType": "nonsense", "responseGroups": [], "slots": []}]});
        let candidate = cloud_candidate_from_value(
            &raw, "batch-1", "item-1", "job-1", "file-1", &"a".repeat(64), 4,
        );
        assert_eq!(candidate.status, ChainStatusV1::Unusable);
        assert_eq!(candidate.reason_code.as_deref(), Some(reason::MODEL_INVALID_OUTPUT));
    }

    #[test]
    fn expand_question_numbers_handles_range_set_and_mixed() {
        assert_eq!(expand_question_numbers(&json!({"kind":"range","start":3,"end":5})), vec![3, 4, 5]);
        assert_eq!(expand_question_numbers(&json!({"kind":"set","values":[7,6]})), vec![6, 7]);
        assert_eq!(
            expand_question_numbers(&json!({"kind":"mixed","values":[1, {"kind":"range","start":3,"end":4}]})),
            vec![1, 3, 4]
        );
    }

    #[test]
    fn not_run_candidate_always_carries_a_reason_code() {
        let candidate = not_run_candidate(
            ChainKindV1::Cloud, "batch-1", "item-1", "job-1", "file-1", &"a".repeat(64), 1,
            reason::NO_PROFILE,
        );
        assert_eq!(candidate.status, ChainStatusV1::NotRun);
        assert_eq!(candidate.reason_code.as_deref(), Some("NO_PROFILE"));
        assert!(candidate.task_groups.is_empty());
    }

    #[test]
    fn natural_cloud_shape_is_no_longer_discarded() {
        // 真实校验通过的「自然形态」：range 是数组、kind、questionIds、notesText、
        // confidence、evidence.quotes，外加 answerKey。修复前会被整体丢弃（status=Unusable）。
        let raw = json!({
            "title": "Sample Paper",
            "groups": [{
                "kind": "true_false_not_given",
                "range": [1, 5],
                "layoutHint": "list",
                "questionIds": ["q1", "q2", "q3", "q4", "q5"],
                "instructionsText": "Do the following statements agree with the claims of the writer?",
                "stimulusText": "Cats are mysterious animals.",
                "notesText": "",
                "confidence": 0.9,
                "evidence": {"quotes": [{"pageIndex": 2, "text": "visible excerpt"}]},
                "slots": [
                    {"questionNumber": 1, "prompt": "Q1", "answer": "TRUE", "evidence": [{"pageIndex": 2, "quote": "q1 excerpt"}]},
                    {"questionNumber": 2, "prompt": "Q2", "answer": "FALSE", "evidence": [{"pageIndex": 2, "quote": "q2 excerpt"}]},
                    {"questionNumber": 3, "prompt": "Q3", "answer": "NOT GIVEN", "evidence": [{"pageIndex": 2, "quote": "q3 excerpt"}]},
                    {"questionNumber": 4, "prompt": "Q4", "answer": "TRUE", "evidence": [{"pageIndex": 2, "quote": "q4 excerpt"}]},
                    {"questionNumber": 5, "prompt": "Q5", "answer": "FALSE", "evidence": [{"pageIndex": 2, "quote": "q5 excerpt"}]}
                ]
            }],
            "answerKey": {"1": "TRUE", "2": "FALSE", "3": "NOT GIVEN", "4": "TRUE", "5": "FALSE"},
            "confidence": 0.9,
            "warnings": []
        });
        let candidate = cloud_candidate_from_value(&raw, "batch-1", "item-1", "job-1", "file-1", &"a".repeat(64), 4);
        assert_eq!(candidate.status, ChainStatusV1::Succeeded);
        assert_eq!(candidate.salvage, None);
        assert_eq!(candidate.task_groups.len(), 1);
        assert_eq!(candidate.slots.len(), 5);
        let numbers: Vec<u32> = candidate.slots.iter().map(|slot| slot.question_number).collect();
        assert_eq!(numbers, vec![1, 2, 3, 4, 5]);
        // 每题答案与 answerKey 一致（归一成文本形态，未伪造）。
        assert_eq!(
            candidate.slots[0].answer,
            Some(json!({"kind": "text", "values": ["TRUE"], "normalization": "ielts_default"}))
        );
        assert_eq!(
            candidate.slots[2].answer,
            Some(json!({"kind": "text", "values": ["NOT GIVEN"], "normalization": "ielts_default"}))
        );
        assert!(candidate.slots[0].has_source_evidence);
    }

    #[test]
    fn non_contiguous_natural_group_yields_matching_slots() {
        // questionIds 不连续（q1,q3,q5）：退回解析题号，仍得到 3 个对应题号的槽位。
        let raw = json!({
            "title": "Paper",
            "groups": [{
                "kind": "short_answer",
                "questionIds": ["q1", "q3", "q5"],
                "layoutHint": "list",
                "instructionsText": "Answer with words.",
                "stimulusText": "Some passage.",
                "notesText": "",
                "confidence": 0.8,
                "evidence": {"quotes": [{"pageIndex": 1, "text": "excerpt"}]},
                "slots": [
                    {"questionNumber": 1, "prompt": "Q1", "answer": "cat"},
                    {"questionNumber": 3, "prompt": "Q3", "answer": "dog"},
                    {"questionNumber": 5, "prompt": "Q5", "answer": "bird"}
                ]
            }],
            "answerKey": {"1": "cat", "3": "dog", "5": "bird"},
            "confidence": 0.8,
            "warnings": []
        });
        let candidate = cloud_candidate_from_value(&raw, "batch-1", "item-1", "job-1", "file-1", &"a".repeat(64), 4);
        assert_eq!(candidate.status, ChainStatusV1::Succeeded);
        let numbers: Vec<u32> = candidate.slots.iter().map(|slot| slot.question_number).collect();
        assert_eq!(numbers, vec![1, 3, 5]);
        assert_eq!(candidate.slots.len(), 3);
    }

    #[test]
    fn align_cloud_answer_shapes_removes_false_divergence() {
        use crate::reconcile::rules::answer_compare_key;
        let local = RecognitionCandidateV1 {
            schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
            chain: ChainKindV1::Local,
            batch_id: "b".into(),
            item_id: "i".into(),
            job_id: "j".into(),
            source_file_id: "f".into(),
            source_sha256: "a".repeat(64),
            base_edit_version: 0,
            generated_at: "x".into(),
            status: ChainStatusV1::Succeeded,
            reason_code: None,
            task_groups: vec![],
            slots: vec![CandidateSlotV1 {
                slot_id: "slot-14".into(),
                question_number: 14,
                display_label: "14".into(),
                task_id: "t".into(),
                response_group_id: "rg".into(),
                interaction: "radio".into(),
                host_type: "prompt".into(),
                host_node_id: None,
                participation: "scoring".into(),
                answer: Some(json!({"kind": "option", "labels": ["B"], "assignment": "unordered_set"})),
                has_source_evidence: false,
            }],
            unresolved_regions: vec![],
            assets: vec![],
            salvage: None,
            warnings: vec![],
        };
        let mut cloud = RecognitionCandidateV1 {
            schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
            chain: ChainKindV1::Cloud,
            batch_id: "b".into(),
            item_id: "i".into(),
            job_id: "j".into(),
            source_file_id: "f".into(),
            source_sha256: "a".repeat(64),
            base_edit_version: 0,
            generated_at: "x".into(),
            status: ChainStatusV1::Succeeded,
            reason_code: None,
            task_groups: vec![],
            slots: vec![CandidateSlotV1 {
                slot_id: "cloud-q14".into(),
                question_number: 14,
                display_label: "14".into(),
                task_id: "t".into(),
                response_group_id: "rg".into(),
                interaction: "text".into(),
                host_type: "prompt".into(),
                host_node_id: None,
                participation: "scoring".into(),
                answer: Some(json!({"kind": "text", "values": ["b"]})),
                has_source_evidence: false,
            }],
            unresolved_regions: vec![],
            assets: vec![],
            salvage: None,
            warnings: vec![],
        };
        let local_key = answer_compare_key(local.slot_by_question(14).and_then(|slot| slot.answer.as_ref()));
        let before = answer_compare_key(cloud.slot_by_question(14).and_then(|slot| slot.answer.as_ref()));
        assert_ne!(local_key, before, "前置：对齐前形状不同，会被判为分歧");
        align_cloud_answer_shapes(&mut cloud, &local);
        let after = answer_compare_key(cloud.slot_by_question(14).and_then(|slot| slot.answer.as_ref()));
        assert_eq!(local_key, after, "对齐后云端与本地比较键必须一致（消除假分歧）");
        // 且保留云端自己的值，仅改形状为 option + 同 assignment。
        assert_eq!(
            cloud.slots[0].answer,
            Some(json!({"kind": "option", "labels": ["b"], "assignment": "unordered_set"}))
        );
    }

    /// Fix 2：选项任务里云端报的是**选项文本**（而非 label）、本地又无选项库可解析时，
    /// 不得把云端文本强行改写成本地 label 形状伪造一致；应保留云端原值暴露真实分歧。
    #[test]
    fn align_preserves_cloud_text_when_option_text_cannot_be_resolved() {
        let local = RecognitionCandidateV1 {
            schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
            chain: ChainKindV1::Local,
            batch_id: "b".into(),
            item_id: "i".into(),
            job_id: "j".into(),
            source_file_id: "f".into(),
            source_sha256: "a".repeat(64),
            base_edit_version: 0,
            generated_at: "x".into(),
            status: ChainStatusV1::Succeeded,
            reason_code: None,
            task_groups: vec![],
            slots: vec![CandidateSlotV1 {
                slot_id: "slot-14".into(),
                question_number: 14,
                display_label: "14".into(),
                task_id: "t".into(),
                response_group_id: "rg".into(),
                interaction: "radio".into(),
                host_type: "prompt".into(),
                host_node_id: None,
                participation: "scoring".into(),
                answer: Some(json!({"kind": "option", "labels": ["B"], "assignment": "per_slot"})),
                has_source_evidence: false,
            }],
            unresolved_regions: vec![],
            assets: vec![],
            salvage: None,
            warnings: vec![],
        };
        let mut cloud = RecognitionCandidateV1 {
            schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
            chain: ChainKindV1::Cloud,
            batch_id: "b".into(),
            item_id: "i".into(),
            job_id: "j".into(),
            source_file_id: "f".into(),
            source_sha256: "a".repeat(64),
            base_edit_version: 0,
            generated_at: "x".into(),
            status: ChainStatusV1::Succeeded,
            reason_code: None,
            task_groups: vec![],
            slots: vec![CandidateSlotV1 {
                slot_id: "cloud-q14".into(),
                question_number: 14,
                display_label: "14".into(),
                task_id: "t".into(),
                response_group_id: "rg".into(),
                interaction: "text".into(),
                host_type: "prompt".into(),
                host_node_id: None,
                participation: "scoring".into(),
                answer: Some(json!({"kind": "text", "values": ["the cold weather"]})),
                has_source_evidence: false,
            }],
            unresolved_regions: vec![],
            assets: vec![],
            salvage: None,
            warnings: vec![],
        };
        align_cloud_answer_shapes(&mut cloud, &local);
        // 没有选项库可把「the cold weather」解析为 label，云端原值必须原样保留。
        assert_eq!(
            cloud.slots[0].answer,
            Some(json!({"kind": "text", "values": ["the cold weather"]}))
        );
    }

    /// Fix 2：选项任务，云端报选项文本，且本地选项库能把文本解析为 label → 合法对齐。
    #[test]
    fn align_resolves_option_text_to_label_via_bank() {
        let bank = CandidateOptionBankV1 {
            option_bank_id: "ob".into(),
            allow_reuse: false,
            options: vec![
                CandidateOptionV1 {
                    option_id: "o1".into(),
                    label: "A".into(),
                    text: "warm climate".into(),
                },
                CandidateOptionV1 {
                    option_id: "o2".into(),
                    label: "B".into(),
                    text: "the cold weather".into(),
                },
            ],
        };
        let local = RecognitionCandidateV1 {
            schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
            chain: ChainKindV1::Local,
            batch_id: "b".into(),
            item_id: "i".into(),
            job_id: "j".into(),
            source_file_id: "f".into(),
            source_sha256: "a".repeat(64),
            base_edit_version: 0,
            generated_at: "x".into(),
            status: ChainStatusV1::Succeeded,
            reason_code: None,
            task_groups: vec![CandidateTaskGroupV1 {
                task_id: "t".into(),
                display_range: json!({"kind": "range", "start": 14, "end": 14}),
                task_type: "single_choice".into(),
                instructions_text: String::new(),
                stimulus_text: None,
                option_bank: Some(bank),
                response_groups: vec![],
                source_anchors: vec![],
                confidence: 1.0,
                warnings: vec![],
            }],
            slots: vec![CandidateSlotV1 {
                slot_id: "slot-14".into(),
                question_number: 14,
                display_label: "14".into(),
                task_id: "t".into(),
                response_group_id: "rg".into(),
                interaction: "radio".into(),
                host_type: "prompt".into(),
                host_node_id: None,
                participation: "scoring".into(),
                answer: Some(json!({"kind": "option", "labels": ["B"], "assignment": "per_slot"})),
                has_source_evidence: false,
            }],
            unresolved_regions: vec![],
            assets: vec![],
            salvage: None,
            warnings: vec![],
        };
        let mut cloud = RecognitionCandidateV1 {
            schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
            chain: ChainKindV1::Cloud,
            batch_id: "b".into(),
            item_id: "i".into(),
            job_id: "j".into(),
            source_file_id: "f".into(),
            source_sha256: "a".repeat(64),
            base_edit_version: 0,
            generated_at: "x".into(),
            status: ChainStatusV1::Succeeded,
            reason_code: None,
            task_groups: vec![],
            slots: vec![CandidateSlotV1 {
                slot_id: "cloud-q14".into(),
                question_number: 14,
                display_label: "14".into(),
                task_id: "t".into(),
                response_group_id: "rg".into(),
                interaction: "text".into(),
                host_type: "prompt".into(),
                host_node_id: None,
                participation: "scoring".into(),
                answer: Some(json!({"kind": "text", "values": ["the cold weather"]})),
                has_source_evidence: false,
            }],
            unresolved_regions: vec![],
            assets: vec![],
            salvage: None,
            warnings: vec![],
        };
        align_cloud_answer_shapes(&mut cloud, &local);
        // 经选项库把云端文本「the cold weather」解析为 label B，与本地一致。
        assert_eq!(
            cloud.slots[0].answer,
            Some(json!({"kind": "option", "labels": ["B"], "assignment": "per_slot"}))
        );
    }

    #[test]
    fn unknown_natural_kind_fails_closed_with_reason_code() {
        let raw = json!({
            "title": "Paper",
            "groups": [{
                "kind": "totally_made_up_type",
                "range": [1, 3],
                "layoutHint": "list",
                "questionIds": ["q1", "q2", "q3"],
                "notesText": "",
                "confidence": 0.5,
                "evidence": {"quotes": [{"pageIndex": 1, "text": "excerpt"}]}
            }],
            "answerKey": {},
            "confidence": 0.5,
            "warnings": []
        });
        let candidate = cloud_candidate_from_value(&raw, "batch-1", "item-1", "job-1", "file-1", &"a".repeat(64), 4);
        assert_ne!(candidate.status, ChainStatusV1::Succeeded);
        assert!(candidate.reason_code.is_some());
        assert_eq!(candidate.task_groups.len(), 0);
    }

    #[test]
    fn missing_natural_kind_fails_closed_with_reason_code() {
        let raw = json!({
            "title": "Paper",
            "groups": [{
                "range": [1, 3],
                "layoutHint": "list",
                "questionIds": ["q1", "q2", "q3"],
                "notesText": "",
                "confidence": 0.5,
                "evidence": {"quotes": [{"pageIndex": 1, "text": "excerpt"}]}
            }],
            "answerKey": {},
            "confidence": 0.5,
            "warnings": []
        });
        let candidate = cloud_candidate_from_value(&raw, "batch-1", "item-1", "job-1", "file-1", &"a".repeat(64), 4);
        assert_ne!(candidate.status, ChainStatusV1::Succeeded);
        assert!(candidate.reason_code.is_some());
    }

    /// Step 0 内核：`align_answer_value` 是**唯一**的形状对齐实现，双向都必须成立，
    /// 且对齐后的比较键必须与目标形状相等——否则 `answer_compare_key` 会在每道题上
    /// 制造假分歧。
    #[test]
    fn align_answer_value_aligns_both_directions_to_equal_compare_keys() {
        let bank = CandidateOptionBankV1 {
            option_bank_id: "ob".into(),
            allow_reuse: false,
            options: vec![
                CandidateOptionV1 { option_id: "o1".into(), label: "A".into(), text: "warm climate".into() },
                CandidateOptionV1 { option_id: "o2".into(), label: "B".into(), text: "the cold weather".into() },
            ],
        };
        let option_shape = json!({"kind": "option", "labels": ["B"], "assignment": "per_slot"});
        let text_shape = json!({"kind": "text", "values": ["the cold weather"]});

        // text → option（本地 option 形状）：云端文本经选项库解析为 label。
        let aligned_up = align_answer_value(&text_shape, &option_shape, Some(&bank))
            .expect("文本能经选项库解析为 label，必须对齐");
        assert_eq!(
            crate::reconcile::rules::answer_compare_key(Some(&aligned_up)),
            crate::reconcile::rules::answer_compare_key(Some(&option_shape)),
            "对齐后必须与目标形状比较键相等，否则会制造假分歧"
        );

        // option → text（本地 text 形状）：云端 label 经选项库解析为文本。
        let aligned_down = align_answer_value(&option_shape, &text_shape, Some(&bank))
            .expect("label 能经选项库解析为文本，必须对齐");
        assert_eq!(
            crate::reconcile::rules::answer_compare_key(Some(&aligned_down)),
            crate::reconcile::rules::answer_compare_key(Some(&text_shape)),
            "反向对齐同样必须使比较键相等"
        );
    }

    /// 形状已一致时返回 `None`（= 不改写）。改写只会引入假一致。
    #[test]
    fn align_answer_value_leaves_equal_shapes_untouched() {
        let bank = CandidateOptionBankV1 { option_bank_id: "ob".into(), allow_reuse: false, options: vec![] };
        let shape = json!({"kind": "text", "values": ["x"]});
        assert_eq!(align_answer_value(&shape, &shape, Some(&bank)), None);
        assert_eq!(align_answer_value(&shape, &shape, None), None);
    }

    /// 没有合法映射时必须返回 `None`（保留原值），绝不强行 unification。
    #[test]
    fn align_answer_value_returns_none_without_a_legal_mapping() {
        let option_shape = json!({"kind": "option", "labels": ["B"], "assignment": "per_slot"});
        let text_value = json!({"kind": "text", "values": ["the cold weather"]});
        assert_eq!(align_answer_value(&text_value, &option_shape, None), None, "无选项库 → 无法映射，保留原值");

        // 选项库存在但没有该文本 → 同样无法映射。
        let bank = CandidateOptionBankV1 {
            option_bank_id: "ob".into(),
            allow_reuse: false,
            options: vec![CandidateOptionV1 { option_id: "o1".into(), label: "A".into(), text: "warm climate".into() }],
        };
        assert_eq!(align_answer_value(&text_value, &option_shape, Some(&bank)), None);

        // 缺少 `kind` 的值不参与对齐（返回 None，由调用方原样保留）。
        assert_eq!(align_answer_value(&json!({"values": ["x"]}), &option_shape, Some(&bank)), None);
    }

// ── rewrite_authoring_references 机械重写 ──────────────────────────────

fn rewrite_fixture() -> Value {
    json!({
        "schemaVersion": "IeltsAuthoringIRV2",
        "jobId": "job-1",
        "sourceDocumentId": "doc-1",
        "taskGroups": [{
            "taskId": "cloud-task-1",
            "displayRange": {"kind": "range", "start": 14, "end": 15},
            "taskType": "sentence_completion",
            "instructions": [{"type": "text", "id": "cloud-node-instr", "text": "Complete."}],
            "optionBank": {
                "optionBankId": "cloud-ob-1",
                "scope": "task_group",
                "options": [{"optionId": "cloud-opt-1", "label": "A", "content": [{"type": "text", "id": "cloud-node-opt", "text": "alpha"}]}],
                "allowReuse": false,
                "sourceAnchors": []
            },
            "responseGroups": [{
                "responseGroupId": "cloud-rg-1",
                "kind": "text_entry",
                "slotIds": ["cloud-q14", "cloud-q15"],
                "optionBankRef": "cloud-ob-1",
                "options": [{"optionId": "cloud-opt-2", "label": "B", "content": []}],
                "cardinality": {"min": 1, "max": 1},
                "assignment": "per_slot",
                "scoringPolicy": "per_slot_binary",
                "duplicatePolicy": "ignore_duplicates",
                "allowOptionReuse": false,
                "sourceAnchors": [{"sourceFileId": "file-1", "pageIndex": 0, "nodeIds": ["cloud-node-1"], "extractionMode": "pdf_native", "sourceHash": "a"}]
            }],
            "sourceAnchors": [{"sourceFileId": "file-1", "pageIndex": 0, "nodeIds": ["cloud-node-1"], "extractionMode": "pdf_native", "sourceHash": "a"}],
            "quality": {"score": 1.0, "sourceCoverage": 1.0, "hardFailures": []},
            "reviewState": "unreviewed"
        }],
        "answerSlots": {
            "cloud-q14": {"slotId": "cloud-q14", "questionNumber": 14, "displayLabel": "14", "hostType": "prompt", "interaction": "text", "participation": "scoring", "hostNodeId": "cloud-node-host", "sourceAnchors": [], "confidence": 0.9},
            "cloud-q15": {"slotId": "cloud-q15", "questionNumber": 15, "displayLabel": "15", "hostType": "prompt", "interaction": "text", "participation": "scoring", "hostNodeId": null, "sourceAnchors": [], "confidence": 0.9}
        },
        "answerKey": {
            "cloud-q14": {"kind": "text", "values": ["books"]},
            "cloud-q15": {"kind": "text", "values": ["pen"]}
        },
        "assets": [{"assetId": "cloud-asset-1", "kind": "raster_image", "mime": "image/png", "relativePath": "a.png", "sha256": "b", "byteLength": 1, "extractionMode": "embedded"}],
        "passage": {
            "title": "P",
            "content": [
                {"type": "paragraph", "id": "cloud-node-para", "sourceAnchors": [], "provenanceStatus": "source", "children": [
                    {"type": "answer_slot", "id": "cloud-node-ans", "slotId": "cloud-q14", "displayLabel": "14", "inline": true}
                ]}
            ],
            "sourceAnchors": []
        },
        "quality": {"score": 1.0},
        "audit": {"revision": 1, "source": "auto_extract", "humanVerified": false, "llmUsed": true, "updatedAt": "x", "notes": []}
    })
}

fn rewrite_map() -> BTreeMap<String, String> {
    [
        ("cloud-task-1", "task-1"),
        ("cloud-ob-1", "ob-1"),
        ("cloud-opt-1", "opt-1"),
        ("cloud-opt-2", "opt-2"),
        ("cloud-rg-1", "rg-1"),
        ("cloud-q14", "slot-14"),
        ("cloud-q15", "slot-15"),
        ("cloud-node-1", "node-1"),
        ("cloud-node-host", "node-host"),
        ("cloud-node-instr", "node-instr"),
        ("cloud-node-opt", "node-opt"),
        ("cloud-node-para", "node-para"),
        ("cloud-node-ans", "node-ans"),
        ("cloud-asset-1", "asset-1"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// `rewrite_map` 中所有临时 id 的全集，作为 `temp_ids` 参数传给被测函数。
fn rewrite_temp_ids() -> BTreeSet<String> {
    rewrite_map().keys().cloned().collect()
}

/// 测试 1：全字段重写。逐个 pointer 断言，确保每一处都被换掉。
#[test]
fn rewrite_covers_every_reference_field() {
    let mut doc = rewrite_fixture();
    let outcome = rewrite_authoring_references(&mut doc, &rewrite_map(), &rewrite_temp_ids());
    assert!(outcome.unmapped.is_empty(), "所有引用都应被映射，实际未映射: {0:?}", outcome.unmapped);
    assert!(outcome.applied, "映射命中了文档里的引用，应当发生实际改写");
    assert!(outcome.conflicts.is_empty(), "无冲突");

    assert_eq!(doc.pointer("/taskGroups/0/taskId").and_then(Value::as_str), Some("task-1"));
    assert_eq!(doc.pointer("/taskGroups/0/optionBank/optionBankId").and_then(Value::as_str), Some("ob-1"));
    assert_eq!(doc.pointer("/taskGroups/0/optionBank/options/0/optionId").and_then(Value::as_str), Some("opt-1"));
    assert_eq!(doc.pointer("/taskGroups/0/responseGroups/0/responseGroupId").and_then(Value::as_str), Some("rg-1"));
    assert_eq!(doc.pointer("/taskGroups/0/responseGroups/0/slotIds/0").and_then(Value::as_str), Some("slot-14"));
    assert_eq!(doc.pointer("/taskGroups/0/responseGroups/0/slotIds/1").and_then(Value::as_str), Some("slot-15"));
    assert_eq!(doc.pointer("/taskGroups/0/responseGroups/0/optionBankRef").and_then(Value::as_str), Some("ob-1"));
    assert_eq!(doc.pointer("/taskGroups/0/responseGroups/0/options/0/optionId").and_then(Value::as_str), Some("opt-2"));
    assert_eq!(doc.pointer("/taskGroups/0/instructions/0/id").and_then(Value::as_str), Some("node-instr"));
    assert_eq!(doc.pointer("/taskGroups/0/optionBank/options/0/content/0/id").and_then(Value::as_str), Some("node-opt"));

    // answerSlots / answerKey 的键被重命名，原键消失。
    assert!(doc.pointer("/answerSlots/cloud-q14").is_none());
    assert!(doc.pointer("/answerSlots/slot-14").is_some());
    assert!(doc.pointer("/answerKey/cloud-q14").is_none());
    assert!(doc.pointer("/answerKey/slot-14").is_some());
    assert_eq!(doc.pointer("/answerSlots/slot-14/slotId").and_then(Value::as_str), Some("slot-14"));
    assert_eq!(doc.pointer("/answerSlots/slot-14/hostNodeId").and_then(Value::as_str), Some("node-host"));
    assert_eq!(doc.pointer("/answerSlots/slot-15/hostNodeId"), Some(&Value::Null), "null 不得被改写");
    assert_eq!(doc.pointer("/answerSlots/slot-15/slotId").and_then(Value::as_str), Some("slot-15"));

    // 内容节点 id 与 answer_slot 节点的 slotId。
    assert_eq!(doc.pointer("/passage/content/0/id").and_then(Value::as_str), Some("node-para"));
    assert_eq!(doc.pointer("/passage/content/0/children/0/id").and_then(Value::as_str), Some("node-ans"));
    assert_eq!(doc.pointer("/passage/content/0/children/0/slotId").and_then(Value::as_str), Some("slot-14"));
    assert_eq!(doc.pointer("/assets/0/assetId").and_then(Value::as_str), Some("asset-1"));

    // 规则 3：sourceAnchors[].nodeIds 即使等于 map 的某个 key（`cloud-node-1`）也不得被改写。
    assert_eq!(doc.pointer("/taskGroups/0/sourceAnchors/0/nodeIds/0").and_then(Value::as_str), Some("cloud-node-1"));
    assert_eq!(doc.pointer("/taskGroups/0/responseGroups/0/sourceAnchors/0/nodeIds/0").and_then(Value::as_str), Some("cloud-node-1"));
}

/// 测试 2：answerSlots 与 answerKey 的键被重命名（最容易漏的一处）。
#[test]
fn rewrite_renames_answer_slots_and_answer_key_keys() {
    let mut doc = json!({
        "answerSlots": {
            "cloud-q14": {"slotId": "cloud-q14"},
            "cloud-q15": {"slotId": "cloud-q15"}
        },
        "answerKey": {
            "cloud-q14": {"kind": "text", "values": ["x"]},
            "cloud-q15": {"kind": "text", "values": ["y"]}
        }
    });
    let map: BTreeMap<String, String> =
        [("cloud-q14", "slot-14"), ("cloud-q15", "slot-15")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let temp_ids: BTreeSet<String> = ["cloud-q14", "cloud-q15"].iter().map(|s| s.to_string()).collect();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);
    assert!(outcome.unmapped.is_empty());
    assert!(outcome.applied, "answerSlots / answerKey 键被重命名，应当发生实际改写");

    assert!(doc.pointer("/answerSlots/cloud-q14").is_none());
    assert!(doc.pointer("/answerSlots/slot-14").is_some());
    assert!(doc.pointer("/answerSlots/cloud-q15").is_none());
    assert!(doc.pointer("/answerSlots/slot-15").is_some());
    assert!(doc.pointer("/answerKey/cloud-q14").is_none());
    assert!(doc.pointer("/answerKey/slot-14").is_some());
    assert!(doc.pointer("/answerKey/cloud-q15").is_none());
    assert!(doc.pointer("/answerKey/slot-15").is_some());
}

/// 测试 3：source anchors 不被重写（构造一个源节点 id 恰好等于 map 的某个 key 的用例）。
#[test]
fn rewrite_never_touches_source_anchor_node_ids() {
    // 把所有其它引用字段都放进 map，使它们被正常改写、不进入未映射列表；
    // 本测试唯一要验证的是 `sourceAnchors[].nodeIds[]` 即便等于某个 map key 也绝不改写。
    let mut doc = json!({
        "taskGroups": [{
            "taskId": "cloud-node-1",
            "sourceAnchors": [{"sourceFileId": "file-1", "pageIndex": 0, "nodeIds": ["cloud-node-1"], "extractionMode": "pdf_native", "sourceHash": "a"}]
        }],
        "answerSlots": {
            "cloud-node-1": {"slotId": "cloud-node-1", "sourceAnchors": [{"sourceFileId": "file-1", "pageIndex": 0, "nodeIds": ["cloud-node-1"], "extractionMode": "pdf_native", "sourceHash": "a"}]}
        }
    });
    // map 里放 `cloud-node-1 -> node-1`，用来证明：即便源节点 id 等于某个 map key，也不改写。
    let map: BTreeMap<String, String> =
        [("cloud-node-1", "node-1")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let temp_ids: BTreeSet<String> = ["cloud-node-1"].iter().map(|s| s.to_string()).collect();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);
    assert!(outcome.unmapped.is_empty());
    assert_eq!(doc.pointer("/taskGroups/0/sourceAnchors/0/nodeIds/0").and_then(Value::as_str), Some("cloud-node-1"));
    // 钥匙被重命名后，仍能在新键下验证源节点 id 未被改写。
    assert_eq!(doc.pointer("/answerSlots/node-1/sourceAnchors/0/nodeIds/0").and_then(Value::as_str), Some("cloud-node-1"));
}

/// 测试 4：未映射引用原样保留，且出现在返回列表里；列表有序去重
/// （两个未映射引用 cloud-q88 / cloud-q99，其中 cloud-q99 重复出现）。
#[test]
fn rewrite_preserves_unmapped_refs_and_reports_them_sorted_dedup() {
    let mut doc = json!({
        "answerSlots": {
            "cloud-q99": {"slotId": "cloud-q99"},
            "cloud-q88": {"slotId": "cloud-q88"}
        },
        "answerKey": {
            "cloud-q99": {"kind": "text", "values": ["x"]}
        }
    });
    // 非空 map，但只含一个与本稿无关的映射，确保 cloud-q88 / cloud-q99 都映射不上。
    let map: BTreeMap<String, String> =
        [("cloud-q14", "slot-14")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let temp_ids: BTreeSet<String> = ["cloud-q88", "cloud-q99"].iter().map(|s| s.to_string()).collect();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);

    // 原样保留。
    assert_eq!(doc.pointer("/answerSlots/cloud-q99/slotId").and_then(Value::as_str), Some("cloud-q99"));
    assert_eq!(doc.pointer("/answerSlots/cloud-q88/slotId").and_then(Value::as_str), Some("cloud-q88"));
    assert!(doc.pointer("/answerKey/cloud-q99").is_some());

    // 返回列表有序去重：cloud-q99 在 answerSlots 键、值 slotId、answerKey 键各出现一次 → 去重为一个。
    assert_eq!(outcome.unmapped, vec!["cloud-q88".to_string(), "cloud-q99".to_string()]);
    assert!(!outcome.applied, "映射与本文档引用完全不相交，没有任何改写发生");
}

/// 测试 5：空 map ⇒ 前后完全相等 + 返回空。
#[test]
fn rewrite_with_empty_map_is_a_noop() {
    let mut doc = rewrite_fixture();
    let before = doc.clone();
    let map: BTreeMap<String, String> = BTreeMap::new();
    // 空 temp_ids：空 map 下不应有任何 key 被误报为 unmapped。
    let temp_ids: BTreeSet<String> = BTreeSet::new();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);
    assert_eq!(doc, before, "空 map 时文档必须字节级不变");
    assert!(outcome.unmapped.is_empty());
    assert!(!outcome.applied, "空映射不得表示引用已全部解析成功");
}

/// 钉死：两个源键映射到同一目标时整篇放弃——文档原样不动，连内容都不得改写。
#[test]
fn rewrite_reports_conflict_when_two_sources_share_one_target() {
    let mut doc = json!({
        "answerSlots": {
            "cloud-a": {"slotId": "cloud-a", "questionNumber": 1},
            "cloud-b": {"slotId": "cloud-b", "questionNumber": 2}
        }
    });
    let before = doc.clone();
    let map: BTreeMap<String, String> =
        [("cloud-a", "slot-x"), ("cloud-b", "slot-x")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let temp_ids: BTreeSet<String> = ["cloud-a", "cloud-b"].iter().map(|s| s.to_string()).collect();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);
    assert!(!outcome.conflicts.is_empty(), "两源键挤向同一目标必须报冲突");
    assert!(
        outcome.conflicts.iter().any(|c| c.contains("answerSlots")),
        "冲突信息应点名 answerSlots: {0:?}", outcome.conflicts
    );
    assert!(!outcome.applied, "冲突时不应发生任何改写");
    assert_eq!(doc, before, "冲突必须整篇放弃，文档原样不动");
}

/// 钉死：键互换（A→B、B→A）时内容跟着键走且零丢失——绝不能静默覆盖。
#[test]
fn rewrite_swaps_slot_keys_without_losing_content() {
    let mut doc = json!({
        "answerSlots": {
            "cloud-a": {"slotId": "cloud-a", "questionNumber": 1},
            "cloud-b": {"slotId": "cloud-b", "questionNumber": 2}
        },
        "answerKey": {
            "cloud-a": {"kind": "text", "values": ["A1"]},
            "cloud-b": {"kind": "text", "values": ["B1"]}
        }
    });
    let map: BTreeMap<String, String> =
        [("cloud-a", "cloud-b"), ("cloud-b", "cloud-a")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let temp_ids: BTreeSet<String> = ["cloud-a", "cloud-b"].iter().map(|s| s.to_string()).collect();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);
    assert!(outcome.conflicts.is_empty(), "互换不冲突");
    assert!(outcome.applied, "至少发生了键重命名");
    // 两个键都还在。
    assert!(doc.pointer("/answerSlots/cloud-a").is_some(), "cloud-a 键应仍在");
    assert!(doc.pointer("/answerSlots/cloud-b").is_some(), "cloud-b 键应仍在");
    // 内容跟着键走：原 cloud-b 的 questionNumber=2 现在在 cloud-a 键下。
    assert_eq!(
        doc.pointer("/answerSlots/cloud-a/questionNumber").and_then(Value::as_i64),
        Some(2)
    );
    // slotId 自身也被换到对应稳定 id。
    assert_eq!(
        doc.pointer("/answerSlots/cloud-a/slotId").and_then(Value::as_str),
        Some("cloud-a")
    );
    // answerKey 的内容同样跟着键走，cloud-a 键下应是原 cloud-b 的 ["B1"]。
    assert_eq!(doc.pointer("/answerKey/cloud-a/values"), Some(&json!(["B1"])));
}

/// 钉死：目标键撞上一个未被映射、须保留的键时，整篇放弃、文档原样不动。
#[test]
fn rewrite_reports_conflict_when_target_key_is_a_preserved_key() {
    let mut doc = json!({
        "answerSlots": {
            "cloud-a": {"slotId": "cloud-a"},
            "cloud-b": {"slotId": "cloud-b"}
        }
    });
    let before = doc.clone();
    // 仅映射 cloud-a → cloud-b；cloud-b 在 temp_ids 中但未被映射，须保留为 cloud-b，
    // 于是与 cloud-a 的目标相撞。
    let map: BTreeMap<String, String> =
        [("cloud-a", "cloud-b")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let temp_ids: BTreeSet<String> = ["cloud-a", "cloud-b"].iter().map(|s| s.to_string()).collect();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);
    assert!(!outcome.conflicts.is_empty(), "目标键撞上保留键必须报冲突");
    assert_eq!(doc, before, "冲突必须整篇放弃，文档原样不动");
    assert!(!outcome.applied, "冲突时不应发生任何改写");
}

/// 钉死：listening.parts[].taskIds[] 逐元素改写，未映射的引用保持原位并如实上报。
#[test]
fn rewrite_rewrites_listening_part_task_ids() {
    let mut doc = json!({
        "listening": {"parts": [{"partId": "p1", "taskIds": ["cloud-task-1", "cloud-task-2"]}]}
    });
    let map: BTreeMap<String, String> =
        [("cloud-task-1", "task-1")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let temp_ids: BTreeSet<String> = ["cloud-task-1", "cloud-task-2"].iter().map(|s| s.to_string()).collect();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);
    assert_eq!(doc.pointer("/listening/parts/0/taskIds/0").and_then(Value::as_str), Some("task-1"));
    assert_eq!(doc.pointer("/listening/parts/0/taskIds/1").and_then(Value::as_str), Some("cloud-task-2"));
    assert_eq!(outcome.unmapped, vec!["cloud-task-2".to_string()]);
    assert!(outcome.applied, "cloud-task-1 被改写，应当 applied");
}

/// 钉死：源文档引用（sourceTableId / sourceAnchors[].nodeIds）永不被重写、永不上报未映射。
#[test]
fn rewrite_never_rewrites_source_table_id() {
    let mut doc = json!({
        "reading": {"tables": [{"sourceTableId": "cloud-table-1"}]},
        "taskGroups": [{"sourceAnchors": [{"sourceFileId": "f1", "pageIndex": 1, "nodeIds": ["cloud-table-1"]}]}]
    });
    let map: BTreeMap<String, String> =
        [("cloud-table-1", "table-1")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let temp_ids: BTreeSet<String> = ["cloud-table-1"].iter().map(|s| s.to_string()).collect();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);
    assert_eq!(doc.pointer("/reading/tables/0/sourceTableId").and_then(Value::as_str), Some("cloud-table-1"));
    assert_eq!(
        doc.pointer("/taskGroups/0/sourceAnchors/0/nodeIds/0").and_then(Value::as_str),
        Some("cloud-table-1")
    );
    assert!(outcome.unmapped.is_empty(), "源文档引用不是草稿空间缺口，绝不报未映射");
}

/// 钉死：已有稳定引用（非 temp_ids）即便与某个映射目标同形也不算缺口、不触发 applied。
#[test]
fn rewrite_does_not_report_existing_stable_refs_as_unmapped() {
    let mut doc = json!({
        "taskGroups": [{
            "taskId": "task-1",
            "responseGroups": [{"responseGroupId": "rg-1", "slotIds": ["slot-14"]}]
        }]
    });
    let map: BTreeMap<String, String> =
        [("cloud-q14", "slot-14")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let temp_ids: BTreeSet<String> = ["cloud-q14"].iter().map(|s| s.to_string()).collect();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);
    assert!(outcome.unmapped.is_empty(), "稳定引用不是缺口，依赖 temp_ids 而非字符串前缀区分");
    assert!(outcome.conflicts.is_empty());
    assert!(!outcome.applied, "映射未命中本文档任何引用，applied 应为 false");
}

/// 钉死：未知的（后端未登记的）资源 id 属资源校验失败，绝不报为未映射缺口。
#[test]
fn rewrite_does_not_report_unknown_asset_ids_as_unmapped() {
    let mut doc = json!({
        "assets": [{"assetId": "asset-unknown"}]
    });
    let map: BTreeMap<String, String> =
        [("cloud-asset-1", "asset-1")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let temp_ids: BTreeSet<String> = ["cloud-asset-1"].iter().map(|s| s.to_string()).collect();
    let outcome = rewrite_authoring_references(&mut doc, &map, &temp_ids);
    assert_eq!(doc.pointer("/assets/0/assetId").and_then(Value::as_str), Some("asset-unknown"));
    assert!(outcome.unmapped.is_empty(), "未知资源不是 id 映射缺口");
}
}

/// 云端完整候选标准化与身份对齐的测试。
///
/// 与既有 `tests` 模块分开，避免混进「扁平候选投影」的历史用例里；
/// 这里的断言针对**完整富内容 + 后端身份**这一组新契约。
#[cfg(test)]
mod cloud_authoring_tests {
    use super::*;

    fn golden_authoring() -> Value {
        let manifest = env!("CARGO_MANIFEST_DIR").trim_end_matches(['\\', '/']);
        let path = std::path::Path::new(manifest)
            .parent()
            .expect("src-tauri 必须有父目录")
            .join("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("读取 golden 稿失败 path={path:?} err={error}"));
        serde_json::from_str(&text).expect("golden 稿必须是合法 JSON")
    }

    fn identity() -> CloudAuthoringIdentity<'static> {
        CloudAuthoringIdentity {
            job_id: "job-1",
            item_id: "job-1",
            batch_id: "batch-1",
            source_file_id: "early-approaches-pdf",
            source_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            base_edit_version: 3,
            generated_at: "2026-09-18T00:00:00Z",
            exam: json!({
                "examId": "early-approaches",
                "title": "Early Approaches to Organisational Design",
                "category": "P3",
                "frequency": "medium",
                "language": "en",
                "tags": ["cloud"],
                "sourceFiles": [{"sourceFileId": "early-approaches-pdf", "role": "question_paper"}]
            }),
            modality: "reading",
            source_document_id: "early-approaches-document",
            extraction_mode: "pdf_native",
        }
    }

    fn paragraph(id: &str, child: &str, text: &str) -> Value {
        json!({
            "type": "paragraph",
            "id": id,
            "sourceAnchors": [],
            "provenanceStatus": "source",
            "children": [{
                "type": "text",
                "id": child,
                "sourceAnchors": [],
                "provenanceStatus": "source",
                "text": text
            }]
        })
    }

    /// 构造一份「模型原始输出」形态的完整候选草稿：全部使用**临时 ID**。
    fn cloud_draft(numbers: &[u32], suffix: &str) -> Value {
        let task_group_id = format!("{suffix}-tg-1");
        let response_group_id = format!("{suffix}-rg-1");
        let option_bank_id = format!("{suffix}-ob-1");
        let instruction_id = format!("{suffix}-ins");
        let instruction_text_id = format!("{suffix}-ins-text");
        let prompt_id = format!("{suffix}-prompt");
        let prompt_text_id = format!("{suffix}-prompt-text");
        let slot_refs: Vec<String> = numbers.iter().map(|n| format!("{suffix}-q{n}")).collect();
        let options: Vec<Value> = ["A", "B", "C", "D", "E"]
            .iter()
            .map(|label| {
                json!({
                    "optionId": format!("{suffix}-opt-{label}"),
                    "label": label,
                    "content": [{
                        "type": "text",
                        "id": format!("{suffix}-opt-{label}-text"),
                        "sourceAnchors": [],
                        "provenanceStatus": "source",
                        "text": format!("factor {label}")
                    }],
                    "sourceAnchors": []
                })
            })
            .collect();

        let mut slots = Map::new();
        for number in numbers {
            slots.insert(
                format!("{suffix}-q{number}"),
                json!({
                    "slotId": format!("{suffix}-q{number}"),
                    "questionNumber": number,
                    "displayLabel": number.to_string(),
                    "hostNodeId": prompt_id,
                    "hostType": "prompt",
                    "interaction": "checkbox",
                    "participation": "scoring",
                    "sourceAnchors": [],
                    "confidence": 0.9
                }),
            );
        }
        let mut answer_key = Map::new();
        let labels = ["B", "D", "A", "C", "E"];
        for (index, number) in numbers.iter().enumerate() {
            let label = labels.get(index).copied().unwrap_or("A");
            answer_key.insert(
                format!("{suffix}-q{number}"),
                json!({"kind": "option", "labels": [label], "assignment": "unordered_set"}),
            );
        }

        json!({
            "taskGroups": [{
                "taskId": task_group_id,
                "displayRange": {"kind": "set", "values": numbers},
                "taskType": "multiple_choice",
                "instructions": [paragraph(
                    &instruction_id,
                    &instruction_text_id,
                    "Choose TWO letters, A-E."
                )],
                "optionBank": {
                    "optionBankId": option_bank_id.clone(),
                    "scope": "task_group",
                    "options": options,
                    "allowReuse": false,
                    "sourceAnchors": []
                },
                "responseGroups": [{
                    "responseGroupId": response_group_id,
                    "kind": "choice",
                    "prompt": [paragraph(
                        &prompt_id,
                        &prompt_text_id,
                        "Which TWO factors influenced early organisational design?"
                    )],
                    "slotIds": slot_refs,
                    "optionBankRef": option_bank_id,
                    "cardinality": {"min": 2, "max": 2, "exact": 2},
                    "assignment": "unordered_set",
                    "scoringPolicy": "per_slot_ielts_normalized",
                    "duplicatePolicy": "reject_submission",
                    "allowOptionReuse": false,
                    "sourceAnchors": []
                }],
                "sourceAnchors": []
            }],
            "answerSlots": Value::Object(slots),
            "answerKey": Value::Object(answer_key),
            "assets": []
        })
    }

    /// 能唯一对应当前稿件的对象：**复用** canonical 的稳定 ID，富内容一字不丢。
    #[test]
    fn cloud_authoring_reuses_canonical_identity_and_keeps_rich_content() {
        let canonical = golden_authoring();
        let raw = json!({"authoring": cloud_draft(&[14, 15], "cloud")});
        let normalized =
            normalize_cloud_authoring(&identity(), Some(&canonical), &raw).expect("标准化必须成功");

        assert_eq!(normalized.status, ChainStatusV1::Succeeded);
        assert!(
            normalized.unresolved_references.is_empty(),
            "唯一对齐时不应留下未解析引用：{:?}",
            normalized.unresolved_references
        );

        let candidate =
            cloud_authoring_candidate_from_normalized(&identity(), normalized).expect("必须可装配");
        let group = &candidate.authoring.task_groups[0];

        // 题组 / 响应组 / 选项库 / 选项：全部接到 canonical 的稳定 ID 上。
        assert_eq!(group.task_id, "early-approaches-q14-15");
        assert_eq!(
            group.response_groups[0].response_group_id,
            "early-approaches-shared-response"
        );
        assert_eq!(
            group.option_bank.as_ref().unwrap().option_bank_id,
            "early-approaches-options"
        );
        assert_eq!(
            group.option_bank.as_ref().unwrap().options[0].option_id,
            "option-a"
        );
        assert!(candidate.authoring.answer_slots.contains_key("q14"));
        assert!(candidate.authoring.answer_slots.contains_key("q15"));
        assert_eq!(candidate.authoring.answer_slots["q14"].slot_id, "q14");
        // `hostNodeId` 必须指向 canonical 的提示节点，不能残留临时 ID。
        assert_eq!(
            candidate.authoring.answer_slots["q14"].host_node_id.as_deref(),
            Some("early-approaches-shared-prompt")
        );

        // 完整富内容必须原样保留（不是被压平成纯文本的投影）。
        let prompt = group.response_groups[0].prompt.as_ref().unwrap();
        assert_eq!(
            nodes_text(&serde_json::to_value(prompt).unwrap()),
            "Which TWO factors influenced early organisational design?"
        );
        let option_content = &group.option_bank.as_ref().unwrap().options[0].content;
        assert_eq!(
            nodes_text(&serde_json::to_value(option_content).unwrap()),
            "factor A"
        );
        assert_eq!(
            candidate.authoring.answer_key["q14"],
            crate::schema::ielts_authoring_v2::AnswerValueV2::Option {
                labels: vec!["B".to_string()],
                assignment: crate::schema::ielts_authoring_v2::AnswerAssignmentV2::UnorderedSet,
            }
        );
    }

    /// 云端识别出的**新增**对象：后端分配稳定 ID，绝不整类降级成人工问题。
    #[test]
    fn cloud_authoring_new_objects_get_backend_identity_without_degrading() {
        let canonical = golden_authoring();
        let raw = json!({"authoring": cloud_draft(&[16, 17], "cloud")});
        let normalized =
            normalize_cloud_authoring(&identity(), Some(&canonical), &raw).expect("标准化必须成功");

        assert_eq!(normalized.status, ChainStatusV1::Succeeded);
        assert!(
            normalized.unresolved_references.is_empty(),
            "新增对象由后端分配身份，不应产生未解析引用：{:?}",
            normalized.unresolved_references
        );

        let candidate =
            cloud_authoring_candidate_from_normalized(&identity(), normalized).expect("必须可装配");
        let group = &candidate.authoring.task_groups[0];
        assert_eq!(group.task_id, "cloud-tg-16-17");
        assert!(candidate.authoring.answer_slots.contains_key("q16"));
        assert!(candidate.authoring.answer_slots.contains_key("q17"));
        // 响应组 / 选项库 / 选项 / 内容节点都必须拿到后端 ID。
        assert_eq!(group.response_groups[0].response_group_id, "cloud-tg-16-17-rg-1");
        assert_eq!(
            group.option_bank.as_ref().unwrap().option_bank_id,
            "cloud-tg-16-17-options"
        );
        assert!(!candidate.id_map.is_empty(), "临时 ID 必须建立映射");
        // 槽位引用必须自洽：响应组声明的 slotIds 就是 answerSlots 的键。
        let declared = &group.response_groups[0].slot_ids;
        assert_eq!(declared, &vec!["q16".to_string(), "q17".to_string()]);
        assert!(declared
            .iter()
            .all(|slot| candidate.authoring.answer_slots.contains_key(slot)));
    }

    /// 题组身份**确实无法确定**时：如实保留歧义，绝不任取第一个。
    #[test]
    fn cloud_authoring_ambiguous_group_identity_is_reported_not_guessed() {
        let mut canonical = golden_authoring();
        // 造一个真实的歧义：两份 canonical 题组都覆盖 q14/q15。
        let duplicate = canonical["taskGroups"][0].clone();
        canonical["taskGroups"]
            .as_array_mut()
            .expect("taskGroups 必须是数组")
            .push(duplicate);

        let raw = json!({"authoring": cloud_draft(&[14, 15], "cloud")});
        let normalized =
            normalize_cloud_authoring(&identity(), Some(&canonical), &raw).expect("标准化必须成功");

        assert_eq!(normalized.status, ChainStatusV1::Partial);
        assert!(
            normalized
                .unresolved_references
                .iter()
                .any(|entry| entry.contains("ambiguous")),
            "歧义必须如实上报：{:?}",
            normalized.unresolved_references
        );

        let candidate =
            cloud_authoring_candidate_from_normalized(&identity(), normalized).expect("必须可装配");
        // 身份没有被伪造：题组仍留在临时空间。
        assert_eq!(candidate.authoring.task_groups[0].task_id, "cloud-tg-1");
        // 但组内对象照常拿到后端 ID —— 歧义只影响题组自身的身份判定。
        assert_eq!(
            candidate.authoring.task_groups[0].response_groups[0].response_group_id,
            "cloud-tg-1-rg-1"
        );
    }

    /// 后端身份字段只从 `identity` 取：模型在输出里伪造 jobId / batchId 一律不采信。
    #[test]
    fn cloud_authoring_ignores_model_supplied_backend_identity() {
        let canonical = golden_authoring();
        let mut draft = cloud_draft(&[14, 15], "cloud");
        draft["jobId"] = json!("attacker-job");
        draft["schemaVersion"] = json!("SomethingElse");
        draft["audit"] = json!({"humanVerified": true, "source": "human"});
        draft["quality"] = json!({"state": "ready"});
        let raw = json!({"authoring": draft});

        let normalized =
            normalize_cloud_authoring(&identity(), Some(&canonical), &raw).expect("标准化必须成功");
        let candidate =
            cloud_authoring_candidate_from_normalized(&identity(), normalized).expect("必须可装配");

        assert_eq!(candidate.job_id, "job-1");
        assert_eq!(candidate.batch_id, "batch-1");
        assert_eq!(candidate.authoring.job_id, "job-1");
        assert_eq!(candidate.authoring.schema_version, "IeltsAuthoringIRV2");
        assert!(!candidate.authoring.audit.human_verified, "模型不得声称已人工核验");
        assert!(candidate.authoring.audit.llm_used);
        assert_ne!(
            candidate.authoring.quality.state,
            crate::schema::quality_report_v2::ReadinessStateV2::Ready,
            "候选质量块不得被模型写成 ready"
        );
    }

    /// 没有题组时如实标 `unusable`，不拿空壳冒充完整候选。
    #[test]
    fn cloud_authoring_without_task_groups_is_unusable_not_a_shell() {
        let canonical = golden_authoring();
        let raw = json!({"authoring": {"taskGroups": [], "answerSlots": {}, "answerKey": {}}});
        let normalized =
            normalize_cloud_authoring(&identity(), Some(&canonical), &raw).expect("标准化必须返回结果");
        assert_eq!(normalized.status, ChainStatusV1::Unusable);
        assert_eq!(
            normalized.reason_code.as_deref(),
            Some("cloud_authoring_candidate_no_task_groups")
        );
        assert!(
            cloud_authoring_candidate_from_normalized(&identity(), normalized).is_err(),
            "不可用候选不得装配成 CloudAuthoringCandidateV1"
        );
    }

    /// 未覆盖区域与来源覆盖说明必须原样带出，不允许用空数组掩盖。
    #[test]
    fn cloud_authoring_carries_unresolved_regions_verbatim() {
        let canonical = golden_authoring();
        let raw = json!({
            "authoring": cloud_draft(&[14, 15], "cloud"),
            "unresolvedRegions": [{
                "sourceFileId": "early-approaches-pdf",
                "pageIndex": 4,
                "reason": "page_image_unavailable",
                "detail": "扫描页图不可用"
            }],
            "sourceCoverageNotes": ["DOCX 图表证据不完整"]
        });
        let normalized =
            normalize_cloud_authoring(&identity(), Some(&canonical), &raw).expect("标准化必须成功");
        assert_eq!(normalized.unresolved_regions.len(), 1);
        assert_eq!(normalized.unresolved_regions[0].page_index, 4);
        assert_eq!(
            normalized.source_coverage_notes,
            vec!["DOCX 图表证据不完整".to_string()]
        );
    }

    /// 输出契约不再要求 `passage`（它是最大的一块输出，却没有任何环节读它）。
    /// 证明：有无 passage，finalize 都成功；修复回合看到的差异清单完全相同。
    #[test]
    fn candidate_without_a_passage_finalizes_and_yields_the_same_differences() {
        let canonical = golden_authoring();
        let without = json!({"authoring": cloud_draft(&[14, 15], "cloud")});
        let mut with_draft = cloud_draft(&[14, 15], "cloud");
        with_draft["passage"] = json!({
            "title": "Early approaches",
            "content": [paragraph("cloud-passage-p1", "cloud-passage-t1", "A long passage body.")],
            "sourceAnchors": []
        });
        let with = json!({"authoring": with_draft});

        let finalize = |raw: &Value| {
            let normalized = normalize_cloud_authoring(&identity(), Some(&canonical), raw)
                .expect("标准化必须成功");
            cloud_authoring_candidate_from_normalized(&identity(), normalized)
                .expect("没有 passage 也必须能装配")
        };
        let candidate_without = finalize(&without);
        let candidate_with = finalize(&with);
        assert!(candidate_without.authoring.passage.is_none());

        let differences = |candidate: &CloudAuthoringCandidateV1| {
            crate::cloud_repair::candidate_differences(
                &canonical,
                &serde_json::to_value(&candidate.authoring).unwrap(),
            )
        };
        assert_eq!(
            differences(&candidate_without),
            differences(&candidate_with),
            "差异清单不得依赖 passage"
        );
    }

    // ── S3：候选分块 + 合并 ───────────────────────────────────────────────

    fn chunk(numbers: &[u32]) -> CandidateChunk {
        CandidateChunk::new(numbers.to_vec())
    }

    /// 分块计划来自**原文件自己声明的题号**（含 no-space 与 glyph-spaced 写法），
    /// 按 passage 级声明切块。
    #[test]
    fn chunk_plan_follows_the_passage_declarations_of_the_original_file() {
        let text = "READING PASSAGE 1\n\
You should spend about 20 minutes on Questions 1-13, which are based on Reading Passage 1.\n\
Questions 1-5\nDo the following statements agree...\n\
Questions6-13\nComplete the notes.\n\
READING PASSAGE 2\n\
You should spend about 20 minutes on Questions 14-26\n\
Questions 14-20\nQuestions 21-26\n\
You should spend about 20 minutes on Questions 2 7 – 4 0\n\
Questions 2 7 – 3 1\nQuestions 32-40\n";
        let plan = plan_candidate_chunks(text);
        let ranges: Vec<Vec<u32>> = plan.iter().map(|chunk| chunk.question_numbers.clone()).collect();
        assert_eq!(
            ranges,
            vec![(1..=13).collect::<Vec<u32>>(), (14..=26).collect(), (27..=40).collect()]
        );
        assert_eq!(plan[2].label, "Questions 27-40");
    }

    /// 没有 passage 级声明时，相邻题组声明合并到一块不超过上限；只剩一块 ⇒ 空计划
    /// （调用方回到一次整卷请求）。没有任何声明 ⇒ 空计划。
    #[test]
    fn chunk_plan_packs_small_groups_and_falls_back_to_one_request() {
        assert!(plan_candidate_chunks("Questions 1-5\nQuestions 6-9\nQuestions 10-13\n").is_empty());
        assert!(plan_candidate_chunks("A passage with no question declarations.").is_empty());
        let plan = plan_candidate_chunks(
            "Questions 1-7\nQuestions 8-13\nQuestions 14-20\nQuestions 21-26\n",
        );
        let ranges: Vec<Vec<u32>> = plan.iter().map(|chunk| chunk.question_numbers.clone()).collect();
        assert_eq!(ranges, vec![(1..=13).collect::<Vec<u32>>(), (14..=26).collect()]);
    }

    /// 两块都用 `cloud-tg-1` / `cloud-rg-1` / `cloud-ob-1` 这类临时 id：不加命名空间直接合并，
    /// 两个题组会共用同一组 id（引用重写必然冲突或张冠李戴）。合并后必须无冲突地标准化，
    /// 并按 (题型, 题号) 映射到权威稿的对应题组。
    #[test]
    fn chunks_reusing_the_same_temporary_ids_merge_and_map_onto_canonical_groups() {
        let canonical = golden_authoring();
        let first = cloud_draft(&[14, 15], "cloud");
        let second = cloud_draft(&[16, 17], "cloud");
        assert_eq!(first["taskGroups"][0]["taskId"], second["taskGroups"][0]["taskId"], "测试前提：临时 id 相撞");

        let merged = merge_candidate_chunks(vec![
            (chunk(&[14, 15]), Ok(first)),
            (chunk(&[16, 17]), Ok(second)),
        ])
        .expect("两块都成功必须能合并");
        let task_ids: Vec<&str> = merged["taskGroups"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|group| group["taskId"].as_str())
            .collect();
        assert_eq!(task_ids.len(), 2);
        assert_ne!(task_ids[0], task_ids[1], "合并后临时 id 必须带块命名空间");

        let normalized =
            normalize_cloud_authoring(&identity(), Some(&canonical), &merged).expect("合并稿必须能标准化");
        assert_eq!(normalized.status, ChainStatusV1::Succeeded, "{:?}", normalized.unresolved_references);
        assert!(normalized.uncovered_question_numbers.is_empty());
        let candidate =
            cloud_authoring_candidate_from_normalized(&identity(), normalized).expect("必须可装配");
        let groups = &candidate.authoring.task_groups;
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].task_id, "early-approaches-q14-15", "14-15 必须接到权威稿题组");
        assert_eq!(groups[1].task_id, "cloud-tg-16-17", "16-17 是新题组，由后端分配身份");
        assert_ne!(
            groups[0].response_groups[0].response_group_id,
            groups[1].response_groups[0].response_group_id
        );
        for key in ["q14", "q15", "q16", "q17"] {
            assert!(candidate.authoring.answer_slots.contains_key(key), "缺 {key}");
        }
        assert_eq!(
            candidate.authoring.answer_slots["q16"].host_node_id.as_deref(),
            groups[1].response_groups[0]
                .prompt
                .as_ref()
                .and_then(|prompt| serde_json::to_value(&prompt[0]).ok())
                .and_then(|node| node["id"].as_str().map(str::to_string))
                .as_deref(),
            "第二块的 hostNodeId 必须指向第二块自己的提示节点"
        );
    }

    /// 一块失败：候选是 Partial，未覆盖的题号如实列出，并成为用户可见的覆盖说明——
    /// 不是整份失败，也不是假装覆盖了。
    #[test]
    fn one_failing_chunk_yields_a_partial_candidate_with_uncovered_numbers() {
        let canonical = golden_authoring();
        let merged = merge_candidate_chunks(vec![
            (chunk(&[14, 15]), Ok(cloud_draft(&[14, 15], "cloud"))),
            (chunk(&[16, 17]), Err("llm_timeout_budget_exhausted:llm_http_timeout".to_string())),
        ])
        .expect("只要有一块成功就不是整份失败");
        let normalized =
            normalize_cloud_authoring(&identity(), Some(&canonical), &merged).expect("必须能标准化");
        assert_eq!(normalized.status, ChainStatusV1::Partial);
        assert_eq!(normalized.uncovered_question_numbers, vec![16, 17]);
        assert_eq!(
            normalized.reason_code.as_deref(),
            Some("cloud_authoring_candidate_chunks_failed")
        );
        assert!(
            normalized
                .source_coverage_notes
                .iter()
                .any(|note| note.contains("16") && note.contains("17")),
            "未覆盖题号必须成为覆盖说明：{:?}",
            normalized.source_coverage_notes
        );
        assert!(normalized
            .warnings
            .iter()
            .any(|warning| warning.contains("llm_timeout_budget_exhausted")));
        let candidate =
            cloud_authoring_candidate_from_normalized(&identity(), normalized).expect("部分候选仍可装配");
        assert_eq!(candidate.status, ChainStatusV1::Partial);

        let all_failed = merge_candidate_chunks(vec![
            (chunk(&[14, 15]), Err("llm_http_500:a".to_string())),
            (chunk(&[16, 17]), Err("llm_http_500:b".to_string())),
        ]);
        let error = all_failed.expect_err("全部失败必须如实失败");
        assert!(error.contains("llm_http_500:a") && error.contains("llm_http_500:b"), "{error}");
    }

    // ── 听力候选：Part 结构 ────────────────────────────────────────────────

    fn listening_identity() -> CloudAuthoringIdentity<'static> {
        CloudAuthoringIdentity {
            modality: "listening",
            ..identity()
        }
    }

    fn listening_canonical() -> Value {
        serde_json::to_value(crate::test_support::complete_listening_exam())
            .expect("听力夹具必须可序列化")
    }

    fn numbers(range: std::ops::RangeInclusive<u32>) -> Vec<u32> {
        range.collect()
    }

    /// 模型给不出的音频事实必须被**丢掉**（不是照抄），并如实记 warning。
    ///
    /// 模型唯一能提供的是 Part 的**结构**（标签、题号、题组归属）；音频是内容寻址的
    /// 资产，模型看不到文件也拿不到哈希。若把它给的 `media` 抄进稿件，就会产出
    /// 一条指向不存在资产的引用——看着完整，打包时才炸。
    #[test]
    fn cloud_authoring_applies_the_models_listening_parts_and_drops_media() {
        let canonical = listening_canonical();
        let mut raw = json!({"authoring": cloud_draft(&[1, 2], "cloud")});
        raw["listeningParts"] = json!([{
            "displayLabel": "Part 1",
            "expectedQuestionNumbers": [1, 2],
            "taskIds": ["cloud-tg-1"],
            // 模型无权提供的音频事实：必须被丢掉并留痕。
            "media": {
                "assetId": "audio-model-invented",
                "mime": "audio/mpeg",
                "durationMs": 600000,
                "sha256": "b".repeat(64)
            },
            "assets": [{"assetId": "audio-model-invented"}]
        }]);

        let normalized = normalize_cloud_authoring(&listening_identity(), Some(&canonical), &raw)
            .expect("标准化必须成功");
        let document = &normalized.document;

        let parts = document
            .pointer("/listening/parts")
            .and_then(Value::as_array)
            .expect("模型给了 Part 结构，稿件里就必须有 listening.parts");
        assert_eq!(parts.len(), 1, "只有模型给出的那一个 Part");
        assert_eq!(parts[0]["displayLabel"], json!("Part 1"));
        assert_eq!(parts[0]["expectedQuestionNumbers"], json!([1, 2]));
        assert!(
            parts[0].get("media").is_none(),
            "模型给的 media 必须被丢掉，不能进稿件：{}",
            parts[0]["media"]
        );
        assert!(
            parts[0].get("assets").is_none(),
            "模型给的 assets 必须被丢掉"
        );
        // taskIds 必须接到后端身份上，不能残留临时 ID。
        let task_ids = parts[0]["taskIds"].as_array().expect("taskIds 必须是数组");
        assert_eq!(task_ids.len(), 1);
        assert_ne!(
            task_ids[0], json!("cloud-tg-1"),
            "Part 的 taskIds 必须被改写成稳定 ID"
        );
        assert!(
            normalized
                .warnings
                .iter()
                .any(|warning| warning.contains("cloud_listening_part_media_dropped")
                    && warning.contains("Part 1")),
            "丢掉模型的音频字段必须留痕：{:?}",
            normalized.warnings
        );
    }

    /// 同一个 Part（题号集合相同）已有音频绑定：**复用**它的身份与音频，只换归属。
    #[test]
    fn cloud_authoring_reuses_an_existing_part_identity_and_keeps_its_audio() {
        let canonical = listening_canonical();
        let bound_asset_id = canonical
            .pointer("/listening/parts/0/media/assetId")
            .and_then(Value::as_str)
            .expect("夹具的第一个 Part 必须带音频")
            .to_string();
        let mut raw = json!({"authoring": cloud_draft(&numbers(1..=10), "cloud")});
        raw["listeningParts"] = json!([{
            "displayLabel": "Part 1",
            "expectedQuestionNumbers": numbers(1..=10),
            "taskIds": ["cloud-tg-1"]
        }]);

        let normalized = normalize_cloud_authoring(&listening_identity(), Some(&canonical), &raw)
            .expect("标准化必须成功");
        let part = &normalized.document.pointer("/listening/parts/0").expect("必须有 Part");

        assert_eq!(
            part["partId"],
            json!("part-1"),
            "题号集合相同就是同一个 Part，必须复用它的稳定 ID"
        );
        assert_eq!(
            part["media"]["assetId"],
            json!(bound_asset_id),
            "用户绑好的 Section 音频不能被云端候选抹掉"
        );
        assert_eq!(
            part["displayLabel"],
            json!("SECTION 1"),
            "已存在的 Part 标签属于人工可见内容，模型不得改写"
        );
        assert_eq!(
            part["taskIds"],
            json!(["task-1"]),
            "题号集合相同即同一个题组，复用稳定 ID"
        );
    }

    /// 模型没给 Part 结构时，**绝不能**把已有听力结构（连同用户绑的音频）丢掉。
    #[test]
    fn cloud_authoring_keeps_the_bound_listening_audio_when_the_model_sends_no_parts() {
        let canonical = listening_canonical();
        let raw = json!({"authoring": cloud_draft(&numbers(1..=10), "cloud")});

        let normalized = normalize_cloud_authoring(&listening_identity(), Some(&canonical), &raw)
            .expect("标准化必须成功");
        let parts = normalized
            .document
            .pointer("/listening/parts")
            .and_then(Value::as_array)
            .expect("没有新结构时也必须保留既有 Part");
        assert_eq!(parts.len(), 4, "四个 Section 一个都不能少");
        for (index, part) in parts.iter().enumerate() {
            assert!(
                part.get("media").is_some(),
                "第 {} 个 Section 的音频被丢掉了",
                index + 1
            );
        }
        assert_eq!(
            normalized.document.pointer("/listening/scope"),
            Some(&json!("complete_exam"))
        );
        assert!(
            normalized
                .warnings
                .iter()
                .any(|warning| warning.contains("cloud_listening_parts_missing")),
            "模型没给 Part 结构必须留痕：{:?}",
            normalized.warnings
        );
    }

    /// 分块识别时同一段可能被**不止一块**汇报（模型顺手把别段也列了一遍）。
    ///
    /// 合并后必须只留一条：两条覆盖同一批题号的分段会各自复用同一个稳定 `partId`，
    /// 于是 `listening.parts` 里出现两个 `part-3`——下游任何按 partId 建索引的地方
    /// （打包、学生端播放器、分段裁定）都会互相覆盖，而且没人会察觉。
    #[test]
    fn cloud_authoring_dedupes_a_part_reported_by_more_than_one_chunk() {
        let canonical = listening_canonical();
        let section = |label: &str, range: std::ops::RangeInclusive<u32>, task: &str| {
            json!({
                "displayLabel": label,
                "expectedQuestionNumbers": numbers(range),
                "taskIds": [task]
            })
        };
        // 两块各自负责一段；第二块「顺手」把前一段也列了一遍。
        let merged = merge_candidate_chunks(vec![
            (
                chunk(&numbers(21..=30)),
                Ok(json!({
                    "authoring": cloud_draft(&numbers(21..=30), "cloud-a"),
                    "listeningParts": [section("Part 3", 21..=30, "cloud-a-tg-1")],
                })),
            ),
            (
                chunk(&numbers(31..=40)),
                Ok(json!({
                    "authoring": cloud_draft(&numbers(31..=40), "cloud-b"),
                    "listeningParts": [
                        section("Part 3", 21..=30, "cloud-b-tg-1"),
                        section("Part 4", 31..=40, "cloud-b-tg-1"),
                    ],
                })),
            ),
        ])
        .expect("两块都成功必须能合并");

        let normalized = normalize_cloud_authoring(&listening_identity(), Some(&canonical), &merged)
            .expect("标准化必须成功");
        let parts = normalized
            .document
            .pointer("/listening/parts")
            .and_then(Value::as_array)
            .expect("合并稿必须带 listening.parts");

        let mut part_ids: Vec<&str> = parts
            .iter()
            .filter_map(|part| part["partId"].as_str())
            .collect();
        let unique_before = part_ids.len();
        part_ids.sort_unstable();
        part_ids.dedup();
        assert_eq!(
            part_ids.len(),
            unique_before,
            "分段身份必须唯一，不能出现两个同 partId 的分段：{parts:?}"
        );
        assert_eq!(parts.len(), 2, "两个不同的题号集合只该产出两个分段：{parts:?}");

        let part_3 = parts
            .iter()
            .find(|part| part["expectedQuestionNumbers"] == json!(numbers(21..=30)))
            .unwrap_or_else(|| panic!("必须有覆盖 21-30 的分段：{parts:?}"));
        assert_eq!(part_3["partId"], json!("part-3"));
        assert!(
            part_3.get("media").is_some(),
            "复用既有分段必须保留用户绑好的音频：{part_3}"
        );
        let part_4 = parts
            .iter()
            .find(|part| part["expectedQuestionNumbers"] == json!(numbers(31..=40)))
            .unwrap_or_else(|| panic!("必须有覆盖 31-40 的分段：{parts:?}"));
        assert_eq!(part_4["partId"], json!("part-4"));
        assert!(
            part_4.get("media").is_some(),
            "复用既有分段必须保留用户绑好的音频：{part_4}"
        );
        assert!(
            normalized
                .warnings
                .iter()
                .any(|warning| warning.contains("cloud_listening_part_duplicate")),
            "丢掉重复的分段必须留痕：{:?}",
            normalized.warnings
        );
    }
}
