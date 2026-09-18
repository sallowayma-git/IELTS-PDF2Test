//! 把本地稿与云端原始输出统一到同一套 V2 语义候选视图。
//!
//! - 本地链：从 `IeltsAuthoringIRV2` 值抽取（不复制权威，只读）。
//! - 云端链：normalize → schema validate → 分组 salvage。**禁止**用默认值补齐
//!   业务字段：不合法的题组被丢弃并记入 salvage 报告，缺字段的答案保持 `None`。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::schema::ielts_authoring_v2::{
    AnswerSlotHostTypeV2, AnswerSlotParticipationV2, AnswerValueV2, InteractionV2, ResponseGroupKindV2,
    TaskTypeV2,
};
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
