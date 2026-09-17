//! 把本地稿与云端原始输出统一到同一套 V2 语义候选视图。
//!
//! - 本地链：从 `IeltsAuthoringIRV2` 值抽取（不复制权威，只读）。
//! - 云端链：normalize → schema validate → 分组 salvage。**禁止**用默认值补齐
//!   业务字段：不合法的题组被丢弃并记入 salvage 报告，缺字段的答案保持 `None`。

use std::collections::BTreeMap;

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
}
