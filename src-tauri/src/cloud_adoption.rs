//! Reception and dependency-unit adoption for editable cloud candidates.
//! Publication completeness is checked later; admission preserves references and human edits.
//! Alignment only produces the repair checklist and source anchors; it never decides admission.

use crate::reconcile::alignment::{AlignmentOutcome, AlignmentReport};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn question_numbers(value: &Value) -> BTreeSet<u32> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|number| u32::try_from(number).ok())
        .collect()
}

fn candidate_question_numbers(authoring: &Value) -> (BTreeSet<u32>, bool) {
    let mut numbers = BTreeSet::new();
    let mut duplicate = false;
    for slot in authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|slots| slots.values())
    {
        let Some(number) = slot
            .get("questionNumber")
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok())
        else {
            continue;
        };
        duplicate |= !numbers.insert(number);
    }
    (numbers, duplicate)
}

fn instruction_question_numbers(authoring: &Value) -> BTreeSet<u32> {
    authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| {
            question_numbers(
                group
                    .pointer("/instructionSignature/expectedQuestionNumbers")
                    .unwrap_or(&Value::Null),
            )
        })
        .collect()
}

fn format_question_numbers(numbers: &[u32]) -> String {
    numbers
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join("、")
}

/// Reception check: only a candidate without a normalized task-group document cannot be used.
/// Partial results, missing answers and disagreement with the local draft are repair work.
pub(crate) fn adoption_rejection_reasons(candidate: &Value, local: &Value) -> Vec<String> {
    let _ = local;
    let authoring = candidate.get("authoring").unwrap_or(&Value::Null);
    if !authoring.is_object() || !authoring.get("taskGroups").is_some_and(Value::is_array) {
        return vec!["云端候选没有可供校核的题组结构".to_string()];
    }
    Vec::new()
}

/// The only conditions that keep the local draft: the candidate is unusable, clearly belongs to
/// another paper, or cannot be cross-checked at all. Everything else is admitted and repaired.
fn catastrophic_reasons(
    candidate: &Value,
    local: &Value,
    alignment: Option<&AlignmentReport>,
) -> Vec<String> {
    let mut reasons = adoption_rejection_reasons(candidate, local);
    if !reasons.is_empty() {
        return reasons;
    }
    let authoring = candidate.get("authoring").unwrap_or(&Value::Null);
    let local_numbers = local_question_numbers(local);
    let (cloud_numbers, _) = candidate_question_numbers(authoring);
    if !local_numbers.is_empty() {
        let overlap = local_numbers.intersection(&cloud_numbers).count();
        if overlap * 2 < local_numbers.len() {
            reasons.push(format!(
                "云端候选题号与原卷识别的题号范围整体不符：仅覆盖 {overlap}/{} 个题号",
                local_numbers.len()
            ));
        }
    }
    match alignment {
        None if local_numbers.is_empty() => reasons
            .push("原卷没有文本层，且没有本地题号可交叉核对，云端候选无法校验".to_string()),
        Some(report)
            if report.passage_sentence_total >= 5
                && report.passage_sentence_matched * 5 < report.passage_sentence_total =>
        {
            reasons.push(format!(
                "云端原文与原卷几乎无法对应（命中 {}/{}），疑似不是同一份卷子",
                report.passage_sentence_matched, report.passage_sentence_total
            ));
        }
        _ => {}
    }
    reasons
}

fn local_question_numbers(local: &Value) -> BTreeSet<u32> {
    local
        .get("slots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|slot| slot.get("questionNumber").and_then(Value::as_u64))
        .filter_map(|number| u32::try_from(number).ok())
        .collect()
}

/// 云端候选一律先进入编辑器；对齐与题型规则只产出复核清单，不再决定「采不采用」。
/// `document_reasons` 非空（灾难性失败）才整份保留本地稿。
#[derive(Debug)]
pub(crate) struct CloudPrimaryPlan {
    /// 进入编辑器的云端题组（对齐不合格的也在内，由复核清单跟进）。
    pub adoptable_task_ids: Vec<String>,
    /// 被复核清单标记的题组及原因；这些题组照常采用云端内容。
    pub flagged_groups: Vec<(String, Vec<String>)>,
    /// 原文未能与原卷对齐的原因（原文仍采用，仅留顾问级复核）。
    pub passage_flags: Vec<String>,
    pub document_reasons: Vec<String>,
    /// 采用但需人复核的记录（题型规则不一致仍采用、无文本层未逐句对照等）。
    pub review_records: Vec<Value>,
    /// 交给云端校核的清单雏形（对齐失败组、题型强反证、说明吞题、题号覆盖、答案未解析/冲突）。
    pub needs_cloud_review: Vec<Value>,
}

/// 有文本层按对齐结果产出复核清单；无文本层只做结构性复核（题型、答案、题号覆盖）。
pub(crate) fn plan_adoption(
    candidate: &Value,
    local: &Value,
    outcome: &AlignmentOutcome,
    local_authoring: Option<&Value>,
) -> CloudPrimaryPlan {
    match outcome {
        AlignmentOutcome::NoTextLayer => plan_unverified_adoption(candidate, local, local_authoring),
        AlignmentOutcome::Assessed(report) => {
            plan_cloud_primary_adoption(candidate, local, report, local_authoring)
        }
    }
}

pub(crate) fn plan_cloud_primary_adoption(
    candidate: &Value,
    local: &Value,
    alignment: &AlignmentReport,
    local_authoring: Option<&Value>,
) -> CloudPrimaryPlan {
    plan_adoption_inner(candidate, local, Some(alignment), local_authoring)
}

fn plan_unverified_adoption(
    candidate: &Value,
    local: &Value,
    local_authoring: Option<&Value>,
) -> CloudPrimaryPlan {
    let mut plan = plan_adoption_inner(candidate, local, None, local_authoring);
    if let (Some(local_authoring), Some(cloud)) = (local_authoring, candidate.get("authoring")) {
        plan.needs_cloud_review
            .extend(unverified_review_entries(local_authoring, cloud));
    }
    plan.review_records.push(serde_json::json!({
        "kind": "source_text_layer_unavailable",
        "note": "原卷没有文本层，云端内容未做逐句对照，仅按题号与结构交叉核对"
    }));
    plan
}

fn plan_adoption_inner(
    candidate: &Value,
    local: &Value,
    alignment: Option<&AlignmentReport>,
    local_authoring: Option<&Value>,
) -> CloudPrimaryPlan {
    let authoring = candidate.get("authoring").unwrap_or(&Value::Null);
    let document_reasons = catastrophic_reasons(candidate, local, alignment);
    let mut group_flags: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut review_records = Vec::new();
    let mut needs_cloud_review = Vec::new();
    let mut passage_flags = Vec::new();

    if let Some(alignment) = alignment {
        // 原文只抽样核对：抽样命中率达标就整体信任云端原文（抽样里对不上的句子只记顾问级提示）；
        // 低于门槛才把整段原文列为一个复核目标。顺序/覆盖/长度只作诊断记录，原卷文字层的
        // 分栏与漏行会让它们对忠实候选误报。
        for miss in &alignment.passage_sample_misses {
            review_records.push(serde_json::json!({
                "kind": "passage_sentence_unverified",
                "nodeId": miss.get("nodeId").cloned().unwrap_or(Value::Null),
                "similarity": miss.get("similarity").cloned().unwrap_or(Value::Null),
                "note": "抽样句与原卷文字层对不上（原文字层可能有提取错误）；顾问级提示，不触发修复"
            }));
        }
        if !alignment.monotonic || !alignment.coverage_ok || !alignment.length_ratio_ok {
            review_records.push(serde_json::json!({
                "kind": "passage_structure_diagnostic",
                "monotonic": alignment.monotonic, "coverageOk": alignment.coverage_ok,
                "lengthRatioOk": alignment.length_ratio_ok,
                "note": "原文顺序/覆盖/长度与原卷文字层有偏差；仅诊断，不触发修复"
            }));
        }
        if !alignment.passage_pass {
            passage_flags.push(format!(
                "原文抽样命中率不足：{}/{}",
                alignment.passage_sample_matched, alignment.passage_sample_total
            ));
            needs_cloud_review.push(serde_json::json!({
                "reason": "passage_not_aligned", "details": passage_flags,
            }));
        }
    }

    // Shared parent regions can contain both roles; only shared physical lines imply swallowed text.
    let mut instruction_regions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut prompt_regions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for node in alignment.iter().flat_map(|report| report.nodes.iter()) {
        let Some(task_id) = &node.task_id else { continue };
        let target = match node.kind_label {
            "instruction" => &mut instruction_regions,
            "prompt" | "option" => &mut prompt_regions,
            _ => continue,
        };
        target
            .entry(task_id.clone())
            .or_default()
            .extend(node.source_line_ids.iter().cloned());
    }

    // 云端与本地一致、且原文抽样通过的单元直接采用，不再做对齐/题型复核。
    let agreeing = match (alignment, local_authoring, authoring) {
        (Some(report), Some(local_doc), cloud_doc) if report.passage_pass => {
            agreeing_cloud_tasks(local_doc, cloud_doc)
        }
        _ => BTreeSet::new(),
    };
    for group in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(task_id) = group.get("taskId").and_then(Value::as_str) else {
            continue;
        };
        if agreeing.contains(task_id) {
            continue;
        }
        // 说明/题干/选项只有严重不符（相似度 < 0.6）才进复核；轻微差异按原卷文字层提取误差处理。
        let mut severe_content = false;
        for node in alignment
            .iter()
            .flat_map(|report| report.nodes.iter())
            .filter(|node| {
                node.task_id.as_deref() == Some(task_id)
                    && matches!(node.kind_label, "instruction" | "prompt" | "option")
                    && node.min_similarity < 0.6
            })
        {
            let mut target = serde_json::json!({
                "taskId": task_id, "nodeId": node.node_id,
                "reason": "content_not_aligned", "similarity": node.min_similarity,
            });
            if matches!(node.kind_label, "prompt" | "option") {
                let owners: Vec<&Value> = group
                    .get("responseGroups")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|response| {
                        ["prompt", "options"].iter().any(|field| {
                            response
                                .get(*field)
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .any(|entry| {
                                    ["id", "optionId"].iter().any(|key| {
                                        entry.get(*key).and_then(Value::as_str)
                                            == Some(node.node_id.as_str())
                                    })
                                })
                        })
                    })
                    .collect();
                if owners.len() == 1 {
                    if let Some(slots) = owners[0].get("slotIds").and_then(Value::as_array) {
                        if slots.len() == 1 {
                            target["slotId"] = slots[0].clone();
                        }
                    }
                }
            }
            needs_cloud_review.push(target);
            severe_content = true;
        }
        if severe_content {
            let mut reasons = alignment
                .and_then(|report| report.group_reasons.get(task_id))
                .cloned()
                .unwrap_or_default();
            if reasons.is_empty() {
                reasons.push("题目内容与原卷严重不符".to_string());
            }
            group_flags
                .entry(task_id.to_string())
                .or_default()
                .extend(reasons);
            continue;
        }
        // 说明与题干/选项落在同一源区域：疑似说明被识别成题目、或题目被吞进说明。
        let overlap = instruction_regions
            .get(task_id)
            .zip(prompt_regions.get(task_id))
            .is_some_and(|(instr, prompt)| !instr.is_disjoint(prompt));
        if overlap {
            group_flags
                .entry(task_id.to_string())
                .or_default()
                .push("说明与题干在原卷上落于同一区域（疑似说明吞题或题目被吞进说明）".to_string());
            needs_cloud_review
                .push(serde_json::json!({"taskId": task_id, "reason": "instruction_stem_overlap"}));
            continue;
        }
        // 题型：以云端为准，本地规则复核；强反证只进复核，不阻止采用。
        match recheck_task_type(group) {
            TypeVerdict::StrongCounter { detected, declared } => {
                group_flags
                    .entry(task_id.to_string())
                    .or_default()
                    .push(format!("题型强反证：说明判为 {detected}，云端声明 {declared}"));
                needs_cloud_review.push(serde_json::json!({
                    "taskId": task_id, "reason": "task_type_counterevidence",
                    "detected": detected, "declared": declared
                }));
                continue;
            }
            TypeVerdict::Review { detected, declared } => {
                review_records.push(serde_json::json!({
                    "taskId": task_id, "kind": "task_type_review",
                    "detected": detected, "declared": declared,
                    "note": "采用云端题型；本地规则判型不一致但云端说明已对齐原卷"
                }));
            }
            TypeVerdict::Accept => {}
        }
    }

    for (task_id, reason) in groups_with_unmapped_passage_hosts(authoring) {
        needs_cloud_review
            .push(serde_json::json!({"taskId": task_id, "reason": "heading_host_unmapped"}));
        group_flags.entry(task_id).or_default().push(reason);
    }

    // 题号覆盖：本地误识别的多余题号不阻止采用，但必须让校核看到。
    let local_numbers = local_question_numbers(local);
    let (cloud_numbers, duplicate_numbers) = candidate_question_numbers(authoring);
    if duplicate_numbers {
        needs_cloud_review.push(serde_json::json!({"reason": "question_number_duplicated"}));
    }
    if !local_numbers.is_empty() {
        let missing: Vec<u32> = local_numbers.difference(&cloud_numbers).copied().collect();
        if !missing.is_empty() {
            needs_cloud_review.push(serde_json::json!({
                "reason": "question_number_uncovered", "questionNumbers": missing,
                "note": format!("云端候选未覆盖本地题号：{}", format_question_numbers(&missing)),
            }));
        }
        let declared = instruction_question_numbers(authoring);
        let undeclared: Vec<u32> = cloud_numbers
            .difference(&local_numbers)
            .copied()
            .filter(|number| !declared.contains(number))
            .collect();
        if !undeclared.is_empty() {
            needs_cloud_review.push(serde_json::json!({
                "reason": "question_number_undeclared", "questionNumbers": undeclared,
                "note": format!(
                    "云端新增题号不在候选自身的说明区范围内：{}",
                    format_question_numbers(&undeclared)
                ),
            }));
        }
    }

    let adoptable_task_ids: Vec<String> = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("taskId").and_then(Value::as_str))
        .map(str::to_string)
        .collect();

    // 采用后仍未解析的计分答案，以及「候选带答案页证据却与答案键冲突」——都是大差异，
    // 进云端校核清单（否则会留下一份看似已采用、答案却缺失或自相矛盾的稿）。
    let answer_key = authoring.get("answerKey").and_then(Value::as_object);
    let answer_slots = authoring.get("answerSlots").and_then(Value::as_object);
    let slot_in_group = |slot_id: &str| -> bool { !group_ids_owning_slot(authoring, slot_id).is_empty() };
    let answer_resolved = |slot_id: &str| -> bool {
        answer_key
            .and_then(|map| map.get(slot_id))
            .is_some_and(|answer| answer.get("kind").and_then(Value::as_str) != Some("unresolved"))
    };
    for (slot_id, slot) in answer_slots.into_iter().flatten() {
        if slot.get("participation").and_then(Value::as_str) != Some("scoring") {
            continue;
        }
        if slot_in_group(slot_id) && !answer_resolved(slot_id) {
            needs_cloud_review
                .push(serde_json::json!({"slotId": slot_id, "reason": "answer_unresolved"}));
        }
    }
    let slot_by_number: BTreeMap<u64, String> = answer_slots
        .into_iter()
        .flatten()
        .filter_map(|(slot_id, slot)| {
            slot.get("questionNumber")
                .and_then(Value::as_u64)
                .map(|number| (number, slot_id.clone()))
        })
        .collect();
    for evidence in authoring
        .get("answerPageEvidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let slot_id = evidence
            .get("slotId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                evidence
                    .get("questionNumber")
                    .and_then(Value::as_u64)
                    .and_then(|number| slot_by_number.get(&number).cloned())
            });
        let Some(slot_id) = slot_id else {
            continue;
        };
        // 只有答案页证据带了**结构化答案**、答案键已解析、两者不一致，才算冲突；
        // 答案未解析走上面的 answer_unresolved，不在这里重复。
        let Some(evidence_answer) = evidence.get("answer") else {
            continue;
        };
        if slot_in_group(&slot_id)
            && answer_resolved(&slot_id)
            && answer_key.and_then(|map| map.get(&slot_id)) != Some(evidence_answer)
        {
            needs_cloud_review.push(
                serde_json::json!({"slotId": slot_id, "reason": "answer_conflicts_answer_page"}),
            );
        }
    }

    CloudPrimaryPlan {
        adoptable_task_ids,
        flagged_groups: group_flags.into_iter().collect(),
        passage_flags,
        document_reasons,
        review_records,
        needs_cloud_review,
    }
}

/// 持有某答案槽的题组身份（按 responseGroups[].slotIds 的字符串成员反查——slotId 在那里是
/// 裸字符串，`group_ids_for_reference` 的按键匹配查不到它）。
fn group_ids_owning_slot(authoring: &Value, slot_id: &str) -> BTreeSet<String> {
    authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|group| {
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
                        .any(|value| value.as_str() == Some(slot_id))
                })
        })
        .filter_map(|group| group.get("taskId").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

/// 收集候选 passage 里所有节点 id（含 paragraphMap 的目标 id），用它判定 heading 宿主段落是否可映射。
fn passage_node_ids(authoring: &Value) -> BTreeSet<String> {
    fn collect(value: &Value, out: &mut BTreeSet<String>) {
        match value {
            Value::Array(items) => items.iter().for_each(|item| collect(item, out)),
            Value::Object(map) => {
                if let Some(id) = map.get("id").and_then(Value::as_str) {
                    out.insert(id.to_string());
                }
                for child in map.values() {
                    collect(child, out);
                }
            }
            _ => {}
        }
    }
    let mut ids = BTreeSet::new();
    if let Some(passage) = authoring.get("passage") {
        collect(passage, &mut ids);
    }
    ids
}

/// heading 宿主段落映射检查：引用了候选 passage 里不存在的段落 id 的题组进复核清单。
fn groups_with_unmapped_passage_hosts(authoring: &Value) -> Vec<(String, String)> {
    let passage_ids = passage_node_ids(authoring);
    let mut failures = Vec::new();
    for (slot_id, slot) in authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if slot.get("hostType").and_then(Value::as_str) != Some("passage_paragraph") {
            continue;
        }
        let host_id = slot.get("hostNodeId").and_then(Value::as_str).unwrap_or("");
        if host_id.is_empty() || !passage_ids.contains(host_id) {
            for owner in group_ids_owning_slot(authoring, slot_id) {
                failures.push((
                    owner,
                    format!("题组的 heading 宿主段落 {host_id} 在候选原文里不存在，无法映射"),
                ));
            }
        }
    }
    failures
}

// ---------------------------------------------------------------------------
// 复核清单：本地确定性规则与对齐结果只决定「哪里要核」，不决定「采不采用」
// ---------------------------------------------------------------------------

/// 说明里有明确图例、判型无歧义的题型；这些若与云端声明冲突即为「强反证」。
fn is_strong_signature_type(wire: &str) -> bool {
    matches!(
        wire,
        "true_false_not_given"
            | "yes_no_not_given"
            | "matching_headings"
            | "matching_information"
            | "matching_features"
            | "matching_sentence_endings"
            | "classification"
    )
}

/// 递归取节点子树里的可见文本。
fn gather_text(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                if !text.is_empty() {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(text);
                }
            }
            for (key, child) in map {
                if key != "text" {
                    gather_text(child, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| gather_text(item, out)),
        _ => {}
    }
}

fn group_instruction_text(group: &Value) -> String {
    let mut out = String::new();
    for node in group
        .get("instructions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        gather_text(node, &mut out);
    }
    out
}

/// 本地确定性判型规则对云端说明文字的复核结论。
enum TypeVerdict {
    /// 规则与云端一致，或规则无判定：直接采纳云端题型。
    Accept,
    /// 规则判出的题型与云端不一致，但非强反证：采纳云端题型并留复核记录。
    Review { detected: String, declared: String },
    /// 强反证：不采纳该组题型。
    StrongCounter { detected: String, declared: String },
}

fn recheck_task_type(group: &Value) -> TypeVerdict {
    let declared = group
        .get("taskType")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let text = group_instruction_text(group);
    let Some(detected) =
        crate::ielts_grammar::classify_instruction_task_type(&text)
    else {
        return TypeVerdict::Accept;
    };
    let detected_wire = serde_json::to_value(&detected)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    if detected_wire.is_empty() || detected_wire == declared {
        return TypeVerdict::Accept;
    }
    if is_strong_signature_type(&detected_wire) {
        TypeVerdict::StrongCounter { detected: detected_wire, declared }
    } else {
        TypeVerdict::Review { detected: detected_wire, declared }
    }
}

/// 本地稿与云端候选逐依赖单元互比：一个依赖单元里没有任何实质差异（比较已忽略空白、大小写、
/// 标识与几何），其云端题组记为「与本地一致」。
fn agreeing_cloud_tasks(local_authoring: &Value, cloud_authoring: &Value) -> BTreeSet<String> {
    let differing_tasks = |target_type: &str, target_id: &str| -> BTreeSet<String> {
        let mut owners = BTreeSet::new();
        for document in [local_authoring, cloud_authoring] {
            for group in groups(document) {
                let Some(task_id) = group.get("taskId").and_then(Value::as_str) else {
                    continue;
                };
                let owns = match target_type {
                    "task_group" => task_id == target_id,
                    "slot" => slot_ids(group).contains(target_id),
                    "response_group" => group
                        .get("responseGroups")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .any(|response| {
                            response.get("responseGroupId").and_then(Value::as_str)
                                == Some(target_id)
                        }),
                    _ => false,
                };
                if owns {
                    owners.insert(task_id.to_string());
                }
            }
        }
        owners
    };
    let mut disagreeing = BTreeSet::new();
    for difference in crate::cloud_repair::candidate_differences(local_authoring, cloud_authoring) {
        let target_type = difference.get("targetType").and_then(Value::as_str).unwrap_or("");
        let target_id = difference.get("targetId").and_then(Value::as_str).unwrap_or("");
        if target_type == "passage" {
            continue;
        }
        let owners = differing_tasks(target_type, target_id);
        if owners.is_empty() {
            // 归不到题组的差异（听力分段等）：保守地让所有单元都不算一致。
            return BTreeSet::new();
        }
        disagreeing.extend(owners);
    }
    comparison_units(local_authoring, cloud_authoring)
        .into_iter()
        .filter(|unit| {
            !unit.cloud_task_ids.is_empty()
                && unit
                    .local_task_ids
                    .iter()
                    .chain(unit.cloud_task_ids.iter())
                    .all(|id| !disagreeing.contains(id))
        })
        .flat_map(|unit| unit.cloud_task_ids)
        .collect()
}

/// 无文本层（扫描件、DOCX）没有对齐结果时的复核清单回退：逐题目单元比较本地稿与云端候选，
/// 比较前做与对齐相同的规范化（字距、连字符、空白、引号），不含 passage，避免干净候选触发修复。
pub(crate) fn unverified_review_entries(local_authoring: &Value, cloud_authoring: &Value) -> Vec<Value> {
    fn loosely_equal(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::String(a), Value::String(b)) => {
                crate::reconcile::alignment::loose_text_key(a)
                    == crate::reconcile::alignment::loose_text_key(b)
            }
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| loosely_equal(x, y))
            }
            (Value::Object(a), Value::Object(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(key, x)| b.get(key).is_some_and(|y| loosely_equal(x, y)))
            }
            _ => left == right,
        }
    }
    let owning_task = |response_group_id: &str| -> Option<String> {
        groups(cloud_authoring)
            .into_iter()
            .chain(groups(local_authoring))
            .find(|group| {
                group
                    .get("responseGroups")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|response| {
                        response.get("responseGroupId").and_then(Value::as_str)
                            == Some(response_group_id)
                    })
            })
            .and_then(|group| group.get("taskId").and_then(Value::as_str).map(str::to_string))
    };
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for difference in crate::cloud_repair::candidate_differences(local_authoring, cloud_authoring) {
        let target_type = difference.get("targetType").and_then(Value::as_str).unwrap_or("");
        let target_id = difference.get("targetId").and_then(Value::as_str).unwrap_or("");
        if target_type == "passage"
            || loosely_equal(
                difference.get("canonical").unwrap_or(&Value::Null),
                difference.get("candidate").unwrap_or(&Value::Null),
            )
        {
            continue;
        }
        let key = match target_type {
            "slot" => "slotId",
            "task_group" => "taskId",
            _ => "taskId",
        };
        let id = match target_type {
            "slot" | "task_group" => target_id.to_string(),
            "response_group" => match owning_task(target_id) {
                Some(task) => task,
                None => continue,
            },
            _ => continue,
        };
        if seen.insert((key, id.clone())) {
            entries.push(serde_json::json!({
                key: id,
                "reason": "content_differs",
                "field": difference.get("field").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    entries
}

/// 已采用（进入编辑器的）题组覆盖的题号清单，供采用记录与后续修复循环使用。
pub(crate) fn adopted_question_numbers(authoring: &Value, adopted_task_ids: &[String]) -> Vec<u32> {
    let adopted: BTreeSet<&str> = adopted_task_ids.iter().map(String::as_str).collect();
    let mut numbers = BTreeSet::new();
    for group in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(task_id) = group.get("taskId").and_then(Value::as_str) else {
            continue;
        };
        if adopted.contains(task_id) {
            numbers.extend(group_question_numbers(authoring, group));
        }
    }
    numbers.into_iter().collect()
}

/// 在对象树里找到 id 匹配的节点并盖上真实来源锚点与 provenanceStatus=source。
fn set_node_provenance(value: &mut Value, id: &str, anchors: &[Value]) -> bool {
    if let Value::Object(map) = value {
        let matches = ["id", "optionId", "slotId"]
            .iter()
            .any(|key| map.get(*key).and_then(Value::as_str) == Some(id));
        if matches {
            map.insert("sourceAnchors".to_string(), Value::Array(anchors.to_vec()));
            map.insert("provenanceStatus".to_string(), Value::String("source".to_string()));
            return true;
        }
        for child in map.values_mut() {
            if set_node_provenance(child, id, anchors) {
                return true;
            }
        }
        false
    } else if let Value::Array(items) = value {
        items.iter_mut().any(|item| set_node_provenance(item, id, anchors))
    } else {
        false
    }
}

/// 把对齐产出的真实锚点盖到对上原卷的云端节点上，并用说明节点锚点补齐题组的
/// instructionSignature.evidenceAnchors。对不上的节点保持原 provenanceStatus，不冒充来源，
/// 留给修复循环补证或发布门禁。
fn stamp_alignment_provenance(merged: &mut Value, alignment: &AlignmentReport) {
    let mut group_evidence: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for node in &alignment.nodes {
        if !node.aligned || node.anchors.is_empty() || node.node_id.is_empty() {
            continue;
        }
        set_node_provenance(merged, &node.node_id, &node.anchors);
        if node.kind_label == "instruction" {
            if let Some(task_id) = &node.task_id {
                group_evidence
                    .entry(task_id.clone())
                    .or_default()
                    .extend(node.anchors.iter().cloned());
            }
        }
    }
    for group in merged
        .get_mut("taskGroups")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let Some(task_id) = group.get("taskId").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if let Some(anchors) = group_evidence.get(&task_id) {
            let signature = group
                .as_object_mut()
                .unwrap()
                .entry("instructionSignature")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(object) = signature.as_object_mut() {
                object.insert("evidenceAnchors".to_string(), Value::Array(anchors.clone()));
            }
        }
    }
}

/// 云端候选先盖上对齐得到的来源锚点，再按依赖单元进入编辑器（与人工编辑同一 CAS/journal 事务）。
pub(crate) fn adopt_cloud_primary(
    root: &Path,
    item_id: &str,
    batch_id: &str,
    candidate_base_version: i64,
    cloud_authoring: &Value,
    alignment: Option<&AlignmentReport>,
) -> Result<AdoptionCommit, String> {
    let mut stamped = cloud_authoring.clone();
    if let Some(alignment) = alignment {
        stamp_alignment_provenance(&mut stamped, alignment);
    }
    adopt_cloud_candidate(root, item_id, batch_id, candidate_base_version, &stamped)
}

#[derive(Debug)]
pub(crate) struct AdoptionCommit {
    pub edit_version: i64,
    pub preserved_group_ids: Vec<String>,
    pub applied_targets: Vec<String>,
    pub adopted_task_ids: Vec<String>,
    pub deferred_task_ids: Vec<String>,
    /// 本地有、云端候选没有的依赖单元：保留本地稿并交复核（删除需要看原卷）。
    pub local_only_task_ids: Vec<String>,
}

fn group_question_numbers(document: &Value, group: &Value) -> BTreeSet<u32> {
    let slots = document.get("answerSlots").and_then(Value::as_object);
    let mut numbers = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        })
        .filter_map(|slot_id| {
            slots?
                .get(slot_id)
                .and_then(|slot| slot.get("questionNumber"))
                .and_then(Value::as_u64)
                .and_then(|number| u32::try_from(number).ok())
        })
        .collect::<BTreeSet<_>>();
    if !numbers.is_empty() {
        return numbers;
    }
    let Some(range) = group.get("displayRange") else {
        return numbers;
    };
    match range.get("kind").and_then(Value::as_str) {
        Some("range") => {
            if let (Some(start), Some(end)) = (
                range.get("start").and_then(Value::as_u64),
                range.get("end").and_then(Value::as_u64),
            ) {
                if start <= end && end.saturating_sub(start) <= 200 {
                    numbers.extend((start..=end).filter_map(|number| u32::try_from(number).ok()));
                }
            }
        }
        Some("set") => numbers.extend(question_numbers(
            range.get("values").unwrap_or(&Value::Null),
        )),
        Some("mixed") => {
            if let Some(values) = range.get("values").and_then(Value::as_array) {
                for value in values {
                    if let Some(number) = value.as_u64().and_then(|n| u32::try_from(n).ok()) {
                        numbers.insert(number);
                    } else if let (Some(start), Some(end)) = (
                        value.get("start").and_then(Value::as_u64),
                        value.get("end").and_then(Value::as_u64),
                    ) {
                        if start <= end && end.saturating_sub(start) <= 200 {
                            numbers.extend((start..=end).filter_map(|n| u32::try_from(n).ok()));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    numbers
}

/// Stable dependency unit shared by automatic adoption and explicit author selection.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComparisonUnit {
    pub unit_id: String,
    pub local_task_ids: Vec<String>,
    pub cloud_task_ids: Vec<String>,
}

fn groups(document: &Value) -> Vec<&Value> {
    document
        .get("taskGroups")
        .and_then(Value::as_array)
        .map(|groups| groups.iter().collect())
        .unwrap_or_default()
}

fn slot_ids(group: &Value) -> BTreeSet<String> {
    group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn dependency_refs(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if matches!(
                    key.as_str(),
                    "optionBankId"
                        | "nodeId"
                        | "sourceNodeId"
                        | "regionId"
                        | "responseGroupId"
                        | "hostNodeId"
                        | "stimulusId"
                ) {
                    if let Some(id) = child.as_str() {
                        refs.insert(format!("{key}:{id}"));
                    }
                }
                dependency_refs(child, refs);
            }
        }
        Value::Array(items) => {
            for child in items {
                dependency_refs(child, refs);
            }
        }
        _ => {}
    }
}

pub(crate) fn comparison_units(local: &Value, cloud: &Value) -> Vec<ComparisonUnit> {
    let entries: Vec<(bool, &Value, &Value)> = groups(local)
        .into_iter()
        .map(|g| (false, local, g))
        .chain(groups(cloud).into_iter().map(|g| (true, cloud, g)))
        .collect();
    let mut parent: Vec<usize> = (0..entries.len()).collect();
    fn root(parent: &[usize], mut i: usize) -> usize {
        while parent[i] != i {
            i = parent[i];
        }
        i
    }
    let numbers: Vec<_> = entries
        .iter()
        .map(|(_, doc, group)| group_question_numbers(doc, group))
        .collect();
    let refs: Vec<_> = entries
        .iter()
        .map(|(_, _, group)| {
            let mut refs = BTreeSet::new();
            dependency_refs(group, &mut refs);
            refs
        })
        .collect();
    for i in 0..entries.len() {
        for j in 0..i {
            let same_id = entries[i].2.get("taskId") == entries[j].2.get("taskId");
            let left = crate::reconcile::candidate::normalize_text(
                &crate::reconcile::candidate::nodes_text(
                    entries[i].2.get("stimulus").unwrap_or(&Value::Null),
                ),
            );
            let right = crate::reconcile::candidate::normalize_text(
                &crate::reconcile::candidate::nodes_text(
                    entries[j].2.get("stimulus").unwrap_or(&Value::Null),
                ),
            );
            if same_id
                || !numbers[i].is_disjoint(&numbers[j])
                || !refs[i].is_disjoint(&refs[j])
                || (!left.is_empty() && left == right)
            {
                let a = root(&parent, i);
                let b = root(&parent, j);
                parent[a] = b;
            }
        }
    }
    let mut components =
        std::collections::BTreeMap::<usize, (BTreeSet<String>, BTreeSet<String>)>::new();
    for (i, (is_cloud, _, group)) in entries.iter().enumerate() {
        if let Some(id) = group.get("taskId").and_then(Value::as_str) {
            let pair = components.entry(root(&parent, i)).or_default();
            if *is_cloud {
                pair.1.insert(id.to_string());
            } else {
                pair.0.insert(id.to_string());
            }
        }
    }
    components
        .into_values()
        .map(|(local_ids, cloud_ids)| {
            let bytes = serde_json::to_vec(&(&local_ids, &cloud_ids)).unwrap_or_default();
            ComparisonUnit {
                unit_id: format!("unit-{}", &crate::hash_bytes(&bytes)[..16]),
                local_task_ids: local_ids.into_iter().collect(),
                cloud_task_ids: cloud_ids.into_iter().collect(),
            }
        })
        .collect()
}

/// Full renderable document projection, not a lossy comparison digest.
pub(crate) fn unit_document(document: &Value, task_ids: &[String]) -> Value {
    let mut selected = document.clone();
    let selected_groups: Vec<Value> = groups(document)
        .into_iter()
        .filter(|g| {
            g.get("taskId")
                .and_then(Value::as_str)
                .is_some_and(|id| task_ids.iter().any(|x| x == id))
        })
        .cloned()
        .collect();
    let ids: BTreeSet<String> = selected_groups.iter().flat_map(slot_ids).collect();
    selected["taskGroups"] = serde_json::json!(selected_groups);
    for key in ["answerSlots", "answerKey"] {
        if let Some(map) = selected.get_mut(key).and_then(Value::as_object_mut) {
            map.retain(|id, _| ids.contains(id));
        }
    }
    if let Some(parts) = selected
        .pointer_mut("/listening/parts")
        .and_then(Value::as_array_mut)
    {
        for part in parts.iter_mut() {
            if let Some(ids) = part.get_mut("taskIds").and_then(Value::as_array_mut) {
                ids.retain(|id| {
                    id.as_str()
                        .is_some_and(|id| task_ids.iter().any(|x| x == id))
                });
            }
        }
        parts.retain(|p| {
            p.get("taskIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| !ids.is_empty())
        });
    }
    selected
}

pub(crate) fn replace_unit(
    current: &Value,
    local: &Value,
    cloud: &Value,
    unit_id: &str,
    use_cloud: bool,
) -> Result<Value, String> {
    let unit = comparison_units(local, cloud)
        .into_iter()
        .find(|unit| unit.unit_id == unit_id)
        .ok_or_else(|| "CLOUD_COMPARISON_UNIT_STALE".to_string())?;
    let selected = if use_cloud { cloud } else { local };
    let ids = if use_cloud {
        &unit.cloud_task_ids
    } else {
        &unit.local_task_ids
    };
    let mut all_ids: BTreeSet<String> = unit
        .local_task_ids
        .iter()
        .chain(unit.cloud_task_ids.iter())
        .cloned()
        .collect();
    let mut numbers = BTreeSet::new();
    for document in [local, cloud] {
        for group in groups(document) {
            if group
                .get("taskId")
                .and_then(Value::as_str)
                .is_some_and(|id| all_ids.contains(id))
            {
                numbers.extend(group_question_numbers(document, group));
            }
        }
    }
    let mut removed_slots = BTreeSet::new();
    let mut insertion = None;
    let mut retained = Vec::new();
    for group in groups(current) {
        let matching = group
            .get("taskId")
            .and_then(Value::as_str)
            .is_some_and(|id| all_ids.contains(id))
            || !group_question_numbers(current, group).is_disjoint(&numbers);
        if matching {
            insertion.get_or_insert(retained.len());
            removed_slots.extend(slot_ids(group));
            if let Some(id) = group.get("taskId").and_then(Value::as_str) {
                all_ids.insert(id.to_string());
            }
        } else {
            retained.push(group.clone());
        }
    }
    let bundle = unit_document(selected, ids);
    let replacement = bundle
        .get("taskGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let at = insertion.unwrap_or(retained.len());
    retained.splice(at..at, replacement);
    let mut merged = current.clone();
    merged["taskGroups"] = serde_json::json!(retained);
    for key in ["answerSlots", "answerKey"] {
        let map = merged
            .get_mut(key)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("cloud_unit_missing:{key}"))?;
        map.retain(|id, _| !removed_slots.contains(id));
        if let Some(new) = bundle.get(key).and_then(Value::as_object) {
            map.extend(new.clone());
        }
    }
    // Restore listening membership together with groups; remove stale memberships first.
    if let Some(parts) = merged
        .pointer_mut("/listening/parts")
        .and_then(Value::as_array_mut)
    {
        for part in parts.iter_mut() {
            if let Some(tasks) = part.get_mut("taskIds").and_then(Value::as_array_mut) {
                tasks.retain(|id| !id.as_str().is_some_and(|id| all_ids.contains(id)));
            }
        }
        for selected_part in bundle
            .pointer("/listening/parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(existing) = parts
                .iter_mut()
                .find(|p| p.get("partId") == selected_part.get("partId"))
            {
                if let (Some(tasks), Some(new)) = (
                    existing.get_mut("taskIds").and_then(Value::as_array_mut),
                    selected_part.get("taskIds").and_then(Value::as_array),
                ) {
                    tasks.extend(new.clone());
                }
            } else {
                parts.push(selected_part.clone());
            }
        }
        parts.retain(|p| {
            p.get("taskIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| !ids.is_empty())
        });
    }
    Ok(merged)
}

/// Publication completeness is intentionally excluded from the editor admission check.
pub(crate) fn editor_runtime_errors(
    document: &Value,
) -> Vec<crate::reading_source_v2::CompilerIssueV2> {
    let typed = match serde_json::from_value::<crate::schema::IeltsAuthoringIRV2>(document.clone())
    {
        Ok(typed) => typed,
        Err(error) => {
            return vec![crate::reading_source_v2::CompilerIssueV2 {
                code: "AUTHORING_SCHEMA_INVALID".to_string(),
                message: error.to_string(),
                target_id: "document".to_string(),
            }]
        }
    };
    match crate::listening_source_v1::compile_exam_source_v2(&typed) {
        Ok(_) => Vec::new(),
        Err(issues) => issues
            .into_iter()
            .filter(|issue| {
                !matches!(
                    issue.code.as_str(),
                    "RUNTIME_ANSWER_UNRESOLVED"
                        | "RUNTIME_ANSWER_KEY_MISSING"
                        | "RUNTIME_ANSWER_KEY_MISMATCH"
                        | "RUNTIME_ANSWER_OPTION_INVALID"
                        | "RUNTIME_ANSWER_WORD_LIMIT_VIOLATION"
                        | "RUNTIME_ANSWER_KEY_POLICY_INVALID"
                        | "RUNTIME_TEXT_SLOT_ANSWER_NOT_TEXT"
                        | "RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION"
                        | "RUNTIME_RESPONSE_ANSWER_KIND_MISMATCH"
                        | "RUNTIME_HOTSPOT_ANSWER_NOT_OPTION"
                        | "LISTENING_MEDIA_MISSING"
                        | "AUDIO_DECODE_FAILED"
                        | "AUDIO_CODEC_UNSUPPORTED"
                        | "AUDIO_HASH_MISMATCH"
                        | "AUDIO_CUE_INVALID"
                        | "AUDIO_POLICY_MISSING"
                        | "LISTENING_COMPLETE_PART_COUNT"
                        | "LISTENING_COMPLETE_QUESTION_COUNT"
                )
            })
            .collect(),
    }
}

fn prepare_partial_adoption(current: &Value, cloud: &Value) -> (Value, Vec<String>, Vec<String>) {
    let units = comparison_units(current, cloud);
    let mut merged = current.clone();
    // Passage remains independently editable; use a normalized cloud passage when renderable.
    if let Some(passage) = cloud.get("passage") {
        merged["passage"] = passage.clone();
    }
    let baseline_errors: BTreeSet<_> = editor_runtime_errors(current)
        .into_iter()
        .map(|i| (i.code, i.target_id))
        .collect();
    if editor_runtime_errors(&merged)
        .iter()
        .any(|i| !baseline_errors.contains(&(i.code.clone(), i.target_id.clone())))
    {
        merged = current.clone();
    }
    let mut adopted = Vec::new();
    let mut deferred = Vec::new();
    for unit in units {
        if unit.cloud_task_ids.is_empty() {
            continue;
        } // deletions need source review
        let mut projected = unit_document(cloud, &unit.cloud_task_ids);
        projected["modality"] = serde_json::json!("reading");
        if projected["passage"].is_null() {
            projected["passage"] = serde_json::json!({"title":"","content":[],"sourceAnchors":[]});
        }
        if !editor_runtime_errors(&projected).is_empty() {
            deferred.extend(unit.cloud_task_ids);
            continue;
        }
        match replace_unit(&merged, current, cloud, &unit.unit_id, true) {
            Ok(trial)
                if editor_runtime_errors(&trial)
                    .iter()
                    .all(|i| baseline_errors.contains(&(i.code.clone(), i.target_id.clone()))) =>
            {
                merged = trial;
                adopted.extend(unit.cloud_task_ids);
            }
            _ => deferred.extend(unit.cloud_task_ids),
        }
    }
    (merged, adopted, deferred)
}

fn object_contains_identity(value: &Value, id: &str) -> bool {
    if [
        "id",
        "taskId",
        "slotId",
        "responseGroupId",
        "assetId",
        "optionId",
    ]
    .iter()
    .any(|key| value.get(key).and_then(Value::as_str) == Some(id))
    {
        return true;
    }
    match value {
        Value::Array(items) => items.iter().any(|item| object_contains_identity(item, id)),
        Value::Object(map) => map.values().any(|item| object_contains_identity(item, id)),
        _ => false,
    }
}

fn group_ids_for_reference(document: &Value, id: &str) -> BTreeSet<String> {
    document
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|group| object_contains_identity(group, id))
        .filter_map(|group| group.get("taskId").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn command_group_ids(command: &Value, document: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for value in [
        command.get("taskId"),
        command.get("replacesTaskId"),
        command.pointer("/taskGroup/taskId"),
        command.pointer("/target/taskId"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    {
        ids.insert(value.to_string());
    }
    for key in [
        "nodeId",
        "parentId",
        "slotId",
        "responseGroupId",
        "optionId",
    ] {
        if let Some(id) = command.get(key).and_then(Value::as_str) {
            ids.extend(group_ids_for_reference(document, id));
        }
    }
    for key in ["nodeId", "responseGroupId", "optionId"] {
        if let Some(id) = command
            .pointer(&format!("/target/{key}"))
            .and_then(Value::as_str)
        {
            ids.extend(group_ids_for_reference(document, id));
        }
    }
    ids
}

fn put_answer_slot(current: &Value, cloud: &mut Value, slot_id: &str) -> Result<(), String> {
    let current_slot = current
        .pointer(&format!("/answerSlots/{slot_id}"))
        .cloned()
        .ok_or_else(|| format!("human_answer_slot_missing:{slot_id}"))?;
    let question_number = current_slot
        .get("questionNumber")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("human_answer_slot_question_missing:{slot_id}"))?;
    let removed = cloud
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|slots| slots.iter())
        .filter(|(key, value)| {
            key.as_str() == slot_id
                || value.get("questionNumber").and_then(Value::as_u64) == Some(question_number)
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    if let Some(slots) = cloud.get_mut("answerSlots").and_then(Value::as_object_mut) {
        for key in &removed {
            slots.remove(key);
        }
        slots.insert(slot_id.to_string(), current_slot);
    } else {
        return Err("cloud_candidate_answer_slots_missing".to_string());
    }
    if let Some(answers) = cloud.get_mut("answerKey").and_then(Value::as_object_mut) {
        for key in removed {
            answers.remove(&key);
        }
        match current.pointer(&format!("/answerKey/{slot_id}")) {
            Some(answer) => {
                answers.insert(slot_id.to_string(), answer.clone());
            }
            None => {
                answers.remove(slot_id);
            }
        }
    }
    Ok(())
}

fn preserve_groups(
    current: &Value,
    cloud: &mut Value,
    seed_group_ids: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let current_groups = current
        .get("taskGroups")
        .and_then(Value::as_array)
        .ok_or_else(|| "current_task_groups_missing".to_string())?;
    let cloud_groups = cloud
        .get("taskGroups")
        .and_then(Value::as_array)
        .ok_or_else(|| "cloud_candidate_task_groups_missing".to_string())?;
    let seed_groups = current_groups
        .iter()
        .filter(|group| {
            group
                .get("taskId")
                .and_then(Value::as_str)
                .is_some_and(|task_id| seed_group_ids.contains(task_id))
        })
        .collect::<Vec<_>>();
    if seed_groups.is_empty() {
        return Err("human_edited_task_group_not_found".to_string());
    }
    let seed_numbers = seed_groups
        .iter()
        .flat_map(|group| group_question_numbers(current, group))
        .collect::<BTreeSet<_>>();
    let matching_cloud_indexes = cloud_groups
        .iter()
        .enumerate()
        .filter(|(_, group)| {
            let same_id = group
                .get("taskId")
                .and_then(Value::as_str)
                .is_some_and(|task_id| seed_group_ids.contains(task_id));
            same_id || !group_question_numbers(cloud, group).is_disjoint(&seed_numbers)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if matching_cloud_indexes.is_empty() {
        return Err("cloud_candidate_has_no_group_for_human_target".to_string());
    }
    let first_cloud_index = matching_cloud_indexes[0];
    let cloud_group_numbers = matching_cloud_indexes
        .iter()
        .flat_map(|index| group_question_numbers(cloud, &cloud_groups[*index]))
        .collect::<BTreeSet<_>>();
    let affected_numbers = seed_numbers
        .union(&cloud_group_numbers)
        .copied()
        .collect::<BTreeSet<_>>();

    // Keep every canonical group participating in the cloud groups being replaced. This is
    // conservative for a split/merge and ensures a user's structural edit cannot strand slots.
    let restore_groups = current_groups
        .iter()
        .enumerate()
        .filter(|(_, group)| {
            !group_question_numbers(current, group).is_disjoint(&affected_numbers)
                || group
                    .get("taskId")
                    .and_then(Value::as_str)
                    .is_some_and(|task_id| seed_group_ids.contains(task_id))
        })
        .map(|(index, group)| (index, group.clone()))
        .collect::<Vec<_>>();
    let preserved_ids = restore_groups
        .iter()
        .filter_map(|(_, group)| group.get("taskId").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();

    let groups = cloud
        .get_mut("taskGroups")
        .and_then(Value::as_array_mut)
        .unwrap();
    for index in matching_cloud_indexes.iter().rev() {
        groups.remove(*index);
    }
    let insertion_index = first_cloud_index.min(groups.len());
    for (offset, (_, group)) in restore_groups.iter().enumerate() {
        let index = (insertion_index + offset).min(groups.len());
        groups.insert(index, group.clone());
    }

    let cloud_slot_ids = cloud
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|slots| slots.iter())
        .filter(|(_, slot)| {
            slot.get("questionNumber")
                .and_then(Value::as_u64)
                .is_some_and(|number| affected_numbers.contains(&(number as u32)))
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    if let Some(slots) = cloud.get_mut("answerSlots").and_then(Value::as_object_mut) {
        for slot_id in &cloud_slot_ids {
            slots.remove(slot_id);
        }
    }
    if let Some(answers) = cloud.get_mut("answerKey").and_then(Value::as_object_mut) {
        for slot_id in &cloud_slot_ids {
            answers.remove(slot_id);
        }
    }
    let current_slots = current
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|slots| slots.iter())
        .filter(|(_, slot)| {
            slot.get("questionNumber")
                .and_then(Value::as_u64)
                .is_some_and(|number| affected_numbers.contains(&(number as u32)))
        })
        .map(|(slot_id, slot)| (slot_id.clone(), slot.clone()))
        .collect::<Vec<_>>();
    for (slot_id, slot) in current_slots {
        cloud["answerSlots"]
            .as_object_mut()
            .ok_or_else(|| "cloud_candidate_answer_slots_missing".to_string())?
            .insert(slot_id.clone(), slot);
        if let Some(answer) = current.pointer(&format!("/answerKey/{slot_id}")) {
            cloud["answerKey"]
                .as_object_mut()
                .ok_or_else(|| "cloud_candidate_answer_key_missing".to_string())?
                .insert(slot_id, answer.clone());
        }
    }
    Ok(preserved_ids)
}

fn preserve_listening_part(current: &Value, cloud: &mut Value, part_id: &str) -> bool {
    let Some(current_part) = current
        .pointer("/listening/parts")
        .and_then(Value::as_array)
        .and_then(|parts| {
            parts
                .iter()
                .find(|part| part.get("partId").and_then(Value::as_str) == Some(part_id))
        })
        .cloned()
    else {
        return false;
    };
    let Some(parts) = cloud
        .pointer_mut("/listening/parts")
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    if let Some(index) = parts
        .iter()
        .position(|part| part.get("partId").and_then(Value::as_str) == Some(part_id))
    {
        parts[index] = current_part;
        true
    } else {
        false
    }
}

/// Rebase saved user edits from the journal onto a freshly normalized cloud candidate.
/// Structural question-group edits preserve the complete original group and its answer slots.
pub(crate) fn merge_human_edits(
    current: &Value,
    cloud: &mut Value,
    journal: &[Value],
) -> Result<Vec<String>, String> {
    let mut preserved_groups = BTreeSet::new();
    for entry in journal {
        let payload = entry.get("command").unwrap_or(entry);
        let commands = payload
            .get("commands")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for command in commands {
            let op = command
                .get("op")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if op == "setAnswer" {
                if let Some(slot_id) = command.get("slotId").and_then(Value::as_str) {
                    put_answer_slot(current, cloud, slot_id)?;
                }
                continue;
            }
            if op == "setListeningPartMedia" {
                let part_id = command
                    .get("partId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "human_listening_part_id_missing".to_string())?;
                if !preserve_listening_part(current, cloud, part_id) {
                    return Err(format!("human_listening_part_not_rebased:{part_id}"));
                }
                continue;
            }
            let group_wide = matches!(
                op,
                "setTaskType"
                    | "setQuestionExpression"
                    | "setResponseCardinality"
                    | "setResponseGroup"
                    | "setOptionBank"
                    | "insertAnswerSlot"
                    | "deleteAnswerSlot"
                    | "upsertTaskGroupBundle"
                    | "insertNode"
                    | "deleteNode"
                    | "moveNode"
            );
            if group_wide {
                let groups = command_group_ids(&command, current);
                preserved_groups.extend(preserve_groups(current, cloud, &groups)?);
                continue;
            }
            if op == "replaceContent" {
                let kind = command.pointer("/target/kind").and_then(Value::as_str);
                if kind == Some("passage") {
                    cloud["passage"] = current.get("passage").cloned().unwrap_or(Value::Null);
                    continue;
                }
                if matches!(kind, Some("responsePrompt" | "option" | "node")) {
                    let id = ["responseGroupId", "optionId", "nodeId"]
                        .into_iter()
                        .find_map(|key| {
                            command
                                .pointer(&format!("/target/{key}"))
                                .and_then(Value::as_str)
                        });
                    if let Some(id) = id {
                        if let Some(value) =
                            crate::library::repository::find_object_by_id(current, id)
                        {
                            if crate::library::repository::replace_object_by_id(cloud, id, value) {
                                continue;
                            }
                        }
                    }
                }
                let groups = command_group_ids(&command, current);
                if !groups.is_empty() {
                    preserved_groups.extend(preserve_groups(current, cloud, &groups)?);
                    continue;
                }
                return Err("human_content_container_not_rebased".to_string());
            }
            let direct_id = command
                .get("nodeId")
                .and_then(Value::as_str)
                .or_else(|| command.get("assetId").and_then(Value::as_str))
                .or_else(|| command.get("targetId").and_then(Value::as_str));
            if matches!(
                op,
                "replaceText"
                    | "setNodeAttrs"
                    | "cropAsset"
                    | "setHotspot"
                    | "removeHotspot"
                    | "bindSource"
            ) {
                if let Some(id) = direct_id {
                    if let Some(value) = crate::library::repository::find_object_by_id(current, id)
                    {
                        if crate::library::repository::replace_object_by_id(cloud, id, value) {
                            continue;
                        }
                    }
                    let groups = command_group_ids(&command, current);
                    if !groups.is_empty() {
                        preserved_groups.extend(preserve_groups(current, cloud, &groups)?);
                        continue;
                    }
                    if object_contains_identity(current.get("passage").unwrap_or(&Value::Null), id)
                    {
                        cloud["passage"] = current.get("passage").cloned().unwrap_or(Value::Null);
                        continue;
                    }
                }
                return Err(format!("human_edit_target_not_rebased:{op}"));
            }
            return Err(format!("human_edit_rebase_unsupported:{op}"));
        }

        // An explicit user undo carries restored target IDs in the journal result rather than
        // replayable editor commands. Preserve each current value (or its owning whole group).
        if payload.get("undoRepairRunId").is_some() {
            let targets = entry
                .pointer("/result/restored")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            for target in targets {
                if let Some(slot_id) = target.strip_prefix("answerKey:") {
                    put_answer_slot(current, cloud, slot_id)?;
                } else if target.starts_with("$task-group:") {
                    let task_id = target.trim_start_matches("$task-group:");
                    preserved_groups.extend(preserve_groups(
                        current,
                        cloud,
                        &BTreeSet::from([task_id.to_string()]),
                    )?);
                } else if let Some(value) =
                    crate::library::repository::find_object_by_id(current, target)
                {
                    if !crate::library::repository::replace_object_by_id(cloud, target, value) {
                        let groups = group_ids_for_reference(current, target);
                        if !groups.is_empty() {
                            preserved_groups.extend(preserve_groups(current, cloud, &groups)?);
                        }
                    }
                }
            }
        }
    }
    Ok(preserved_groups.into_iter().collect())
}

/// Promote renderable dependency units of a cloud candidate through the canonical editor CAS/journal transaction.
/// A version conflict is retried from a fresh canonical + human journal snapshot.
pub(crate) fn adopt_cloud_candidate(
    root: &Path,
    item_id: &str,
    batch_id: &str,
    candidate_base_version: i64,
    cloud_authoring: &Value,
) -> Result<AdoptionCommit, String> {
    let run_id = crate::cloud_repair::repair_run_id_for(batch_id);
    let request_id = format!("cloud-candidate-adoption:{batch_id}");
    let mut last_conflict = None;
    for _attempt in 0..3 {
        let mut conn = crate::library::repository::open_library_connection(root)?;
        let Some((current, current_version)) =
            crate::library::repository::get_canonical_ds(&conn, item_id)?
        else {
            return Err("cloud_candidate_adoption_canonical_missing".to_string());
        };
        if candidate_base_version > current_version {
            return Err(format!(
                "cloud_candidate_adoption_base_ahead:base={candidate_base_version}:current={current_version}"
            ));
        }
        let journal = crate::library::repository::human_editor_commands_since(
            &conn,
            item_id,
            candidate_base_version,
        )?;
        let (mut merged, mut adopted_task_ids, mut deferred_task_ids) =
            prepare_partial_adoption(&current, cloud_authoring);
        if adopted_task_ids.is_empty() {
            return Err(
                "CLOUD_CANDIDATE_NO_RENDERABLE_UNITS:候选已接收，题组需要先经校核修复".to_string(),
            );
        }
        if let Some(assets) = current.get("assets") {
            merged["assets"] = assets.clone();
        }
        let local_only_task_ids: Vec<String> = comparison_units(&current, cloud_authoring)
            .into_iter()
            .filter(|unit| unit.cloud_task_ids.is_empty())
            .flat_map(|unit| unit.local_task_ids)
            .collect();
        let preserved_group_ids = merge_human_edits(&current, &mut merged, &journal)?;
        // A preserved human group remains on the current side of comparison, not the adopted side.
        let preserved: BTreeSet<_> = preserved_group_ids.iter().collect();
        adopted_task_ids.retain(|id| {
            if preserved.contains(id) {
                deferred_task_ids.push(id.clone());
                false
            } else {
                true
            }
        });
        let baseline: BTreeSet<_> = editor_runtime_errors(&current)
            .into_iter()
            .map(|i| (i.code, i.target_id))
            .collect();
        let introduced: Vec<_> = editor_runtime_errors(&merged)
            .into_iter()
            .filter(|i| !baseline.contains(&(i.code.clone(), i.target_id.clone())))
            .map(|i| format!("{}:{}", i.code, i.target_id))
            .collect();
        if !introduced.is_empty() {
            return Err(format!(
                "CLOUD_CANDIDATE_REBASE_STRUCTURE_INVALID:{}",
                introduced.join(",")
            ));
        }
        if let (Some(current_audit), Some(audit)) = (
            current.get("audit").and_then(Value::as_object),
            merged.get_mut("audit").and_then(Value::as_object_mut),
        ) {
            audit.insert(
                "revision".to_string(),
                serde_json::json!(current_audit
                    .get("revision")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .saturating_add(1)),
            );
            audit.insert(
                "source".to_string(),
                serde_json::json!("cloud_candidate_adoption"),
            );
            audit.insert(
                "humanVerified".to_string(),
                current_audit
                    .get("humanVerified")
                    .cloned()
                    .unwrap_or(serde_json::Value::Bool(false)),
            );
            audit.insert(
                "updatedAt".to_string(),
                serde_json::json!(chrono::Utc::now().to_rfc3339()),
            );
        }
        let command = serde_json::json!({
            "op": "replaceAuthoringDocument",
            "authoring": merged,
        });
        let result = crate::library::repository::apply_editor_commands_tx_with(
            &mut conn,
            &crate::library::repository::ApplyEditorCommandsInput {
                item_id: item_id.to_string(),
                base_version: current_version,
                request_id: Some(request_id.clone()),
                commands: vec![command],
                title: None,
            },
            crate::library::repository::EditOrigin::CloudCandidateAdoption,
            Some(&run_id),
            &|document, patch| {
                if patch.get("op").and_then(Value::as_str) == Some("replaceAuthoringDocument") {
                    let authoring = patch
                        .get("authoring")
                        .filter(|value| value.is_object())
                        .ok_or_else(|| "cloud_candidate_adoption_document_invalid".to_string())?;
                    *document = authoring.clone();
                    Ok(())
                } else {
                    crate::authoring_v2_commands::apply_patch(document, patch)
                }
            },
            &|document| {
                crate::authoring_v2_commands::refresh_quality_report(root, item_id, document)?;
                crate::authoring_v2_commands::validate_authoring(document)
            },
            &|_, _| Ok(()),
        );
        match result {
            Ok(result) => {
                return Ok(AdoptionCommit {
                    edit_version: result.edit_version,
                    preserved_group_ids,
                    applied_targets: result.applied_targets,
                    adopted_task_ids,
                    deferred_task_ids,
                    local_only_task_ids,
                });
            }
            Err(error) if error.starts_with("EDIT_VERSION_CONFLICT") => {
                last_conflict = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_conflict.unwrap_or_else(|| "cloud_candidate_adoption_conflict".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn candidate() -> Value {
        let mut authoring: Value = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .expect("committed fixture parses");
        authoring["quality"]["compilerProbes"]["v2Runtime"]["status"] = json!("passed");
        authoring["quality"]["hardFailures"] = json!([]);
        json!({
            "status": "succeeded",
            "unresolvedReferences": [],
            "authoring": authoring
        })
    }

    fn local() -> Value {
        json!({"slots": [
            {"slotId":"q14", "questionNumber":14},
            {"slotId":"q15", "questionNumber":15}
        ]})
    }

    #[test]
    fn listening_candidate_without_audio_is_editable_and_keeps_active_runtime_contract() {
        let typed = crate::test_support::complete_listening_exam();
        let mut current = serde_json::to_value(typed).unwrap();
        for part in current["listening"]["parts"].as_array_mut().unwrap() {
            part.as_object_mut().unwrap().remove("media");
        }
        let cloud = current.clone();
        let (_, adopted, deferred) = prepare_partial_adoption(&current, &cloud);
        assert!(deferred.is_empty(), "{deferred:?}");
        assert_eq!(adopted.len(), cloud["taskGroups"].as_array().unwrap().len());
        let typed = serde_json::from_value::<crate::schema::IeltsAuthoringIRV2>(cloud).unwrap();
        assert!(
            crate::listening_source_v1::compile_exam_source_v2(&typed)
                .unwrap_err()
                .iter()
                .any(|i| i.code == "LISTENING_MEDIA_MISSING"),
            "Publication still requires usable audio"
        );
    }
    #[test]
    fn complete_runtime_valid_candidate_covers_local_numbers_and_keeps_q_ids() {
        let cloud = candidate();
        assert!(adoption_rejection_reasons(&cloud, &local()).is_empty());
        assert_eq!(
            cloud.pointer("/authoring/answerSlots/q14/slotId"),
            Some(&json!("q14"))
        );
        assert_eq!(
            cloud.pointer("/authoring/answerSlots/q15/slotId"),
            Some(&json!("q15"))
        );
    }

    #[test]
    fn reception_accepts_partial_candidates_and_defers_all_publication_findings() {
        let mut cloud = candidate();
        cloud["status"] = json!("partial");
        cloud["unresolvedReferences"] = json!(["bad-group"]);
        cloud["authoring"]["quality"]["hardFailures"] =
            json!(["PROVENANCE_MISSING", "RUNTIME_COMPILER_FAILED"]);
        cloud["authoring"]["quality"]["compilerProbes"]["v2Runtime"]["status"] = json!("failed");
        assert!(adoption_rejection_reasons(&cloud, &json!({"slots":[]})).is_empty());
    }

    #[test]
    fn local_false_question_cannot_reject_a_received_cloud_candidate() {
        let cloud = candidate();
        let local = json!({"slots":[{"questionNumber":999}]});
        assert!(adoption_rejection_reasons(&cloud, &local).is_empty());
        assert!(!adoption_rejection_reasons(&json!({"authoring":null}), &local).is_empty());
    }

    #[test]
    fn missing_answers_are_editor_admissible_but_broken_slot_references_are_not() {
        let mut document = candidate()["authoring"].clone();
        document["answerKey"]["q14"] = json!({"kind":"unresolved"});
        assert!(
            editor_runtime_errors(&document).is_empty(),
            "{:?}",
            editor_runtime_errors(&document)
        );
        document["taskGroups"][0]["responseGroups"][0]["slotIds"] = json!(["missing-slot"]);
        assert!(!editor_runtime_errors(&document).is_empty());
    }

    #[test]
    fn dependency_unit_groups_local_split_against_cloud_merge_and_choice_is_complete() {
        let local = json!({"taskGroups":[
            {"taskId":"a","displayRange":{"kind":"set","values":[1]},"responseGroups":[{"slotIds":["q1"]}]},
            {"taskId":"b","displayRange":{"kind":"set","values":[2]},"responseGroups":[{"slotIds":["q2"]}]}
        ], "answerSlots":{"q1":{"questionNumber":1},"q2":{"questionNumber":2}}, "answerKey":{"q1":{},"q2":{}}});
        let cloud = json!({"taskGroups":[{"taskId":"c","displayRange":{"kind":"set","values":[1,2]},"responseGroups":[{"slotIds":["q1","q2"]}]}],"answerSlots":{"q1":{"questionNumber":1},"q2":{"questionNumber":2}},"answerKey":{"q1":{},"q2":{}}});
        let units = comparison_units(&local, &cloud);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].local_task_ids, vec!["a", "b"]);
        let selected = replace_unit(&local, &local, &cloud, &units[0].unit_id, true).unwrap();
        assert_eq!(selected["taskGroups"], cloud["taskGroups"]);
        assert_eq!(selected["answerSlots"], cloud["answerSlots"]);
        assert_eq!(
            replace_unit(&selected, &local, &cloud, &units[0].unit_id, false).unwrap(),
            local
        );
    }

    #[test]
    fn one_invalid_group_does_not_prevent_nine_renderable_groups_adopting() {
        let base = candidate()["authoring"].clone();
        let mut local = base.clone();
        let mut cloud = base.clone();
        let mut local_groups = Vec::new();
        let mut cloud_groups = Vec::new();
        let mut slots = serde_json::Map::new();
        let mut answers = serde_json::Map::new();
        for i in 0..10 {
            let offset = i * 2;
            let mut group = base["taskGroups"][0].clone();
            let serialized = serde_json::to_string(&group)
                .unwrap()
                .replace("q14", &format!("q{}", 14 + offset))
                .replace("q15", &format!("q{}", 15 + offset))
                .replace("early-approaches-q14-15", &format!("group-{i}"));
            group = serde_json::from_str(&serialized).unwrap();
            group["taskId"] = json!(format!("group-{i}"));
            group["displayRange"] = json!({"kind":"set","values":[14+offset,15+offset]});
            // Each unit has its own response/option identities.
            let serialized = serde_json::to_string(&group)
                .unwrap()
                .replace("early-approaches-", &format!("g{i}-"));
            group = serde_json::from_str(&serialized).unwrap();
            let mut cloud_group = group.clone();
            cloud_group["instructions"][0]["children"][0]["text"] =
                json!(format!("Cloud instructions {i}"));
            if i == 9 {
                cloud_group["responseGroups"][0]["slotIds"] = json!(["missing-slot"]);
            }
            local_groups.push(group);
            cloud_groups.push(cloud_group);
            for number in [14 + offset, 15 + offset] {
                let original = if number % 2 == 0 { "q14" } else { "q15" };
                let id = format!("q{number}");
                let mut slot = base["answerSlots"][original].clone();
                slot["slotId"] = json!(id);
                slot["questionNumber"] = json!(number);
                slot["displayLabel"] = json!(number.to_string());
                // use generated response/node identities for runtime closure
                if let Some(rg) = slot.get_mut("responseGroupId") {
                    *rg = json!(rg
                        .as_str()
                        .unwrap()
                        .replace("early-approaches-", &format!("g{i}-"))
                        .replace("q14", &format!("q{}", 14 + offset))
                        .replace("q15", &format!("q{}", 15 + offset)));
                }
                if let Some(host) = slot.get_mut("hostNodeId") {
                    *host = json!(host
                        .as_str()
                        .unwrap()
                        .replace("early-approaches-", &format!("g{i}-"))
                        .replace("q14", &format!("q{}", 14 + offset))
                        .replace("q15", &format!("q{}", 15 + offset)));
                }
                slots.insert(id.clone(), slot);
                answers.insert(id, base["answerKey"][original].clone());
            }
        }
        for doc in [&mut local, &mut cloud] {
            doc["answerSlots"] = json!(slots);
            doc["answerKey"] = json!(answers);
        }
        local["taskGroups"] = json!(local_groups);
        cloud["taskGroups"] = json!(cloud_groups);
        let (merged, adopted, deferred) = prepare_partial_adoption(&local, &cloud);
        assert_eq!(
            adopted.len(),
            9,
            "deferred={deferred:?}, local errors={:?}",
            editor_runtime_errors(&local)
        );
        assert_eq!(deferred.len(), 1);
        assert_eq!(merged["taskGroups"][9], local["taskGroups"][9]);
    }

    #[test]
    fn human_text_and_answer_edits_are_rebased_onto_the_cloud_candidate() {
        let current = json!({
            "passage": {"content": [{"id":"p-text", "type":"text", "text":"local user text"}]},
            "taskGroups": [{
                "taskId":"task-1", "displayRange":{"kind":"range","start":14,"end":14},
                "instructions":[{"id":"instruction-text", "type":"text", "text":"user instruction"}],
                "responseGroups":[{"responseGroupId":"rg-1", "slotIds":["q14"]}]
            }],
            "answerSlots":{"q14":{"slotId":"q14", "questionNumber":14}},
            "answerKey":{"q14":{"kind":"text", "values":["user answer"]}}
        });
        let mut cloud = json!({
            "passage": {"content": [{"id":"p-text", "type":"text", "text":"cloud text"}]},
            "taskGroups": [{
                "taskId":"task-1", "displayRange":{"kind":"range","start":14,"end":14},
                "instructions":[{"id":"instruction-text", "type":"text", "text":"cloud instruction"}],
                "responseGroups":[{"responseGroupId":"rg-1", "slotIds":["q14"]}]
            }],
            "answerSlots":{"q14":{"slotId":"q14", "questionNumber":14}},
            "answerKey":{"q14":{"kind":"text", "values":["cloud answer"]}}
        });
        let journal = [json!({"command":{"commands":[
            {"op":"replaceText", "nodeId":"p-text"},
            {"op":"setAnswer", "slotId":"q14"}
        ]}, "result":{}})];

        merge_human_edits(&current, &mut cloud, &journal).unwrap();
        assert_eq!(
            cloud.pointer("/passage/content/0/text"),
            Some(&json!("local user text"))
        );
        assert_eq!(
            cloud.pointer("/answerKey/q14/values/0"),
            Some(&json!("user answer"))
        );
        assert_eq!(
            cloud.pointer("/taskGroups/0/instructions/0/text"),
            Some(&json!("cloud instruction"))
        );
    }

    #[test]
    fn structural_user_edit_preserves_whole_group_and_its_slots() {
        let current = json!({
            "taskGroups": [{
                "taskId":"task-1", "displayRange":{"kind":"range","start":14,"end":15},
                "optionBank":{"optionBankId":"user-bank", "options":[{"optionId":"u-a","label":"A"}]},
                "responseGroups":[{"responseGroupId":"rg-1", "slotIds":["q14","q15"]}]
            }],
            "answerSlots":{
                "q14":{"slotId":"q14", "questionNumber":14},
                "q15":{"slotId":"q15", "questionNumber":15}
            },
            "answerKey":{
                "q14":{"kind":"text", "values":["user 14"]},
                "q15":{"kind":"text", "values":["user 15"]}
            }
        });
        let mut cloud = json!({
            "taskGroups": [{
                "taskId":"task-1", "displayRange":{"kind":"range","start":14,"end":15},
                "optionBank":{"optionBankId":"cloud-bank", "options":[{"optionId":"c-a","label":"A"}]},
                "responseGroups":[{"responseGroupId":"rg-1", "slotIds":["q14","q15"]}]
            }],
            "answerSlots":{
                "q14":{"slotId":"q14", "questionNumber":14},
                "q15":{"slotId":"q15", "questionNumber":15}
            },
            "answerKey":{
                "q14":{"kind":"text", "values":["cloud 14"]},
                "q15":{"kind":"text", "values":["cloud 15"]}
            }
        });
        let journal = [json!({"command":{"commands":[
            {"op":"setOptionBank", "taskId":"task-1"}
        ]}, "result":{}})];

        let preserved = merge_human_edits(&current, &mut cloud, &journal).unwrap();
        assert_eq!(preserved, vec!["task-1"]);
        assert_eq!(
            cloud["taskGroups"][0]["optionBank"]["optionBankId"],
            "user-bank"
        );
        assert_eq!(
            cloud.pointer("/answerKey/q14/values/0"),
            Some(&json!("user 14"))
        );
        assert_eq!(
            cloud.pointer("/answerKey/q15/values/0"),
            Some(&json!("user 15"))
        );
    }

    // 云端候选一律先进入编辑器：对齐与规则只产出复核清单，不决定采用与否。

    use crate::reconcile::alignment::{AlignmentOutcome, AlignmentReport, NodeAlignment};

    fn cp_node(
        node_id: &str,
        kind: &'static str,
        task_id: Option<&str>,
        src: &[&str],
        aligned: bool,
    ) -> NodeAlignment {
        NodeAlignment {
            node_id: node_id.to_string(),
            kind_label: kind,
            task_id: task_id.map(str::to_string),
            sentence_count: 1,
            matched_sentences: if aligned { 1 } else { 0 },
            min_similarity: if aligned { 1.0 } else { 0.0 },
            mean_similarity: if aligned { 1.0 } else { 0.0 },
            source_node_ids: src.iter().map(|s| s.to_string()).collect(),
            source_line_ids: src.iter().map(|s| s.to_string()).collect(),
            anchors: src.iter().map(|s| json!({"nodeIds": [s]})).collect(),
            has_invented: !aligned,
            aligned,
        }
    }

    fn cp_report(passage_pass: bool, groups: &[(&str, bool)], nodes: Vec<NodeAlignment>) -> AlignmentReport {
        AlignmentReport {
            nodes,
            passage_pass,
            passage_sample_total: 10,
            passage_sample_matched: if passage_pass { 10 } else { 4 },
            passage_sample_misses: Vec::new(),
            passage_sentence_total: 10,
            passage_sentence_matched: if passage_pass { 10 } else { 4 },
            passage_invented: Vec::new(),
            passage_invented_ok: true,
            length_ratio: 1.0,
            length_ratio_ok: true,
            coverage: 1.0,
            coverage_ok: true,
            order_violations: 0,
            order_in_order_ratio: 1.0,
            monotonic: true,
            group_aligned: groups.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            group_reasons: BTreeMap::new(),
            samples: Vec::new(),
        }
    }

    const TFNG_INSTRUCTION: &str = "Do the following statements agree with the information given in the reading passage? Write TRUE if the statement agrees with the information, FALSE if the statement contradicts the information, NOT GIVEN if there is no information on this.";

    fn cp_candidate(groups: Value, slots: Value) -> Value {
        json!({
            "status": "succeeded",
            "unresolvedReferences": [],
            "authoring": {
                "passage": {"content": []},
                "taskGroups": groups,
                "answerSlots": slots,
                "answerKey": {},
                "quality": {"compilerProbes": {"v2Runtime": {"status": "passed"}}, "hardFailures": [], "issues": []}
            }
        })
    }

    fn cp_local(numbers: &[u32]) -> Value {
        json!({"slots": numbers
            .iter()
            .map(|n| json!({"slotId": format!("q{n}"), "questionNumber": n}))
            .collect::<Vec<_>>()})
    }

    fn tfng_group(task_id: &str, slot: &str) -> Value {
        json!({
            "taskId": task_id, "taskType": "true_false_not_given",
            "instructions": [{"id": format!("{task_id}-i"), "type": "text", "text": TFNG_INSTRUCTION}],
            "responseGroups": [{"responseGroupId": format!("{task_id}-rg"), "slotIds": [slot]}]
        })
    }

    fn entries_with<'a>(plan: &'a CloudPrimaryPlan, reason: &str) -> Vec<&'a Value> {
        plan.needs_cloud_review
            .iter()
            .filter(|entry| entry.get("reason").and_then(Value::as_str) == Some(reason))
            .collect()
    }

    #[test]
    fn heading_group_with_unmapped_passage_host_is_flagged_but_still_adopted() {
        let mut cloud = candidate();
        cloud["authoring"]["quality"]["issues"] = json!([]);
        cloud["authoring"]["answerSlots"]["q14"]["hostType"] = json!("passage_paragraph");
        cloud["authoring"]["answerSlots"]["q14"]["hostNodeId"] = json!("invented-paragraph");
        let plan = plan_unverified_adoption(&cloud, &local(), None);
        assert!(plan.document_reasons.is_empty(), "{:?}", plan.document_reasons);
        assert!(plan.adoptable_task_ids.contains(&"early-approaches-q14-15".to_string()));
        assert!(plan.flagged_groups.iter().any(|(id, reasons)| id
            == "early-approaches-q14-15"
            && reasons.iter().any(|reason| reason.contains("invented-paragraph"))));
        assert_eq!(entries_with(&plan, "heading_host_unmapped").len(), 1);
    }

    #[test]
    fn blocking_quality_findings_never_reject_a_received_candidate() {
        let mut cloud = candidate();
        cloud["authoring"]["quality"]["issues"] = json!([{
            "code":"SIGNIFICANT_REGION_UNASSIGNED","severity":"blocking",
            "targetType":"recognition","targetId":"document", "message":"显著源节点未认领"
        }]);
        cloud["authoring"]["quality"]["hardFailures"] = json!(["PROVENANCE_MISSING"]);
        let plan = plan_unverified_adoption(&cloud, &local(), None);
        assert!(plan.document_reasons.is_empty(), "{:?}", plan.document_reasons);
        assert_eq!(plan.adoptable_task_ids, vec!["early-approaches-q14-15".to_string()]);
    }

    #[test]
    fn shared_region_between_instruction_and_prompt_is_not_a_swallowed_question() {
        let candidate = cp_candidate(json!([tfng_group("g1", "q1")]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let mut instruction = cp_node("g1-i", "instruction", Some("g1"), &["shared-region", "line-1"], true);
        let mut prompt = cp_node("p1", "prompt", Some("g1"), &["shared-region", "line-2"], true);
        instruction.source_line_ids = BTreeSet::from(["line-1".to_string()]);
        prompt.source_line_ids = BTreeSet::from(["line-2".to_string()]);
        let report = cp_report(true, &[("g1", true)], vec![instruction, prompt]);
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
        assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()]);
        assert!(plan.needs_cloud_review.is_empty());
    }

    #[test]
    fn only_severe_content_mismatch_enters_the_review_list_and_the_group_is_still_adopted() {
        for (node_id, kind, expected_slot) in [
            ("i1", "instruction", None),
            ("p1", "prompt", Some("q1")),
            ("o1", "option", Some("q1")),
        ] {
            let groups = json!([{
                "taskId": "g1", "taskType": "true_false_not_given",
                "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
                "responseGroups": [
                    {"responseGroupId": "rg1", "slotIds": ["q1"],
                     "prompt": [{"id": "p1", "type": "text", "text": "cloud first question"}],
                     "options": [{"optionId": "o1", "text": "cloud first option"}]},
                    {"responseGroupId": "rg2", "slotIds": ["q2"],
                     "prompt": [{"id": "p2", "type": "text", "text": "cloud second question"}]}
                ]
            }]);
            let candidate = cp_candidate(groups, json!({
                "q1": {"slotId": "q1", "questionNumber": 1},
                "q2": {"slotId": "q2", "questionNumber": 2}
            }));
            let mut suspect = cp_node(node_id, kind, Some("g1"), &["suspect-region"], true);
            suspect.sentence_count = 10;
            suspect.matched_sentences = 9;
            suspect.min_similarity = 0.55;
            suspect.mean_similarity = 0.955;
            suspect.has_invented = true;
            let report = cp_report(true, &[("g1", true)], vec![
                suspect, cp_node("p2", "prompt", Some("g1"), &["r2"], true),
            ]);
            let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1, 2]), &report, None);
            assert!(plan.document_reasons.is_empty(), "{:?}", plan.document_reasons);
            assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()], "对不上的单元也先进入编辑器");
            assert!(plan.flagged_groups.iter().any(|(id, _)| id == "g1"));
            let targets = entries_with(&plan, "content_not_aligned");
            assert_eq!(targets.len(), 1, "{kind} 的严重不符必须独立进复核：{:?}", plan.needs_cloud_review);
            assert_eq!(targets[0].get("taskId").and_then(Value::as_str), Some("g1"));
            assert_eq!(targets[0].get("nodeId").and_then(Value::as_str), Some(node_id));
            assert_eq!(targets[0].get("slotId").and_then(Value::as_str), expected_slot);
            assert_eq!(targets[0].get("similarity"), Some(&json!(0.55)));
        }
    }

    #[test]
    fn shared_option_with_severe_mismatch_is_reviewed_by_node_not_by_slot() {
        let candidate = cp_candidate(json!([{
            "taskId": "g1", "taskType": "true_false_not_given",
            "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
            "optionBank": {"options": [{"optionId": "bank-o1", "text": "shared choice"}]},
            "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1", "q2"]}]
        }]), json!({
            "q1": {"slotId": "q1", "questionNumber": 1},
            "q2": {"slotId": "q2", "questionNumber": 2}
        }));
        let mut suspect = cp_node("bank-o1", "option", Some("g1"), &["r1"], false);
        suspect.min_similarity = 0.59;
        let report = cp_report(true, &[("g1", true)], vec![suspect]);
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1, 2]), &report, None);
        assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()]);
        assert!(plan.needs_cloud_review.iter().any(|entry| {
            entry.get("reason").and_then(Value::as_str) == Some("content_not_aligned")
                && entry.get("taskId").and_then(Value::as_str) == Some("g1")
                && entry.get("nodeId").and_then(Value::as_str) == Some("bank-o1")
                && entry.get("slotId").is_none()
        }), "共享选项应按节点复核，不误绑单题：{:?}", plan.needs_cloud_review);
    }

    #[test]
    fn review_threshold_stays_at_point_six_and_group_ratio_alone_does_not_trigger_repair() {
        let candidate = cp_candidate(json!([tfng_group("g1", "q1")]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        for similarity in [0.6, 0.7, 0.8] {
            let mut node = cp_node("g1-i", "instruction", Some("g1"), &["r1"], true);
            node.min_similarity = similarity;
            let report = cp_report(true, &[("g1", true)], vec![node]);
            let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
            assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()]);
            assert!(plan.needs_cloud_review.is_empty(), "相似度 {similarity} 不应进入复核");
        }
        // 组内对齐比例不足但没有任何严重不符的节点：原卷文字层提取误差，不触发修复。
        let report = cp_report(true, &[("g1", false)], vec![]);
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
        assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()]);
        assert!(plan.needs_cloud_review.is_empty(), "{:?}", plan.needs_cloud_review);
    }

    #[test]
    fn aligned_candidate_is_adopted_whole_with_an_empty_review_list() {
        let candidate = cp_candidate(json!([tfng_group("g1", "q1")]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let report = cp_report(
            true,
            &[("g1", true)],
            vec![
                cp_node("g1-i", "instruction", Some("g1"), &["r1"], true),
                cp_node("p1", "prompt", Some("g1"), &["r2"], true),
            ],
        );
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
        assert!(plan.document_reasons.is_empty(), "{:?}", plan.document_reasons);
        assert!(plan.passage_flags.is_empty());
        assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()]);
        assert!(plan.flagged_groups.is_empty());
        assert!(plan.needs_cloud_review.is_empty(), "干净候选必须零复核目标");
    }

    #[test]
    fn passage_misses_are_advisory_and_a_failed_sample_is_one_passage_target() {
        let candidate = cp_candidate(json!([tfng_group("g1", "q1")]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let mut report = cp_report(true, &[("g1", true)], vec![cp_node("g1-i", "instruction", Some("g1"), &["r1"], true)]);
        report.passage_sample_misses = vec![
            json!({"nodeId": "p-1", "sentence": "x", "similarity": 0.4}),
            json!({"nodeId": "p-2", "sentence": "y", "similarity": 0.5}),
        ];
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
        assert!(plan.needs_cloud_review.is_empty(), "抽样里对不上的句子只是顾问级提示：{:?}", plan.needs_cloud_review);
        assert_eq!(
            plan.review_records
                .iter()
                .filter(|record| record["kind"] == "passage_sentence_unverified")
                .count(),
            2
        );

        let report = cp_report(false, &[("g1", true)], vec![cp_node("g1-i", "instruction", Some("g1"), &["r1"], true)]);
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
        assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()], "原文抽样不过也不阻止题组进入编辑器");
        assert!(!plan.passage_flags.is_empty());
        assert_eq!(entries_with(&plan, "passage_not_aligned").len(), 1, "整段原文只列一个复核目标");
        assert!(plan.needs_cloud_review.iter().all(|entry| entry.get("nodeId").is_none()));
    }

    #[test]
    fn passage_order_coverage_and_length_deviations_are_diagnostics_not_repair_targets() {
        let candidate = cp_candidate(json!([tfng_group("g1", "q1")]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let mut report = cp_report(true, &[("g1", true)], vec![cp_node("g1-i", "instruction", Some("g1"), &["r1"], true)]);
        report.monotonic = false;
        report.coverage_ok = false;
        report.length_ratio_ok = false;
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
        assert!(plan.needs_cloud_review.is_empty(), "{:?}", plan.needs_cloud_review);
        assert!(plan.review_records.iter().any(|r| r["kind"] == "passage_structure_diagnostic"));
    }

    #[test]
    fn unresolved_answer_in_adopted_group_enters_the_review_list() {
        let candidate = cp_candidate(
            json!([tfng_group("g1", "q1")]),
            json!({"q1": {"slotId": "q1", "questionNumber": 1, "participation": "scoring"}}),
        );
        let report = cp_report(
            true,
            &[("g1", true)],
            vec![
                cp_node("g1-i", "instruction", Some("g1"), &["r1"], true),
                cp_node("p1", "prompt", Some("g1"), &["r2"], true),
            ],
        );
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
        assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()]);
        assert!(
            plan.needs_cloud_review.iter().any(|entry| entry.get("slotId").and_then(Value::as_str) == Some("q1")
                && entry.get("reason").and_then(Value::as_str) == Some("answer_unresolved")),
            "{:?}",
            plan.needs_cloud_review
        );
    }

    #[test]
    fn strong_task_type_counterevidence_is_reviewed_but_the_group_is_still_adopted() {
        let mut group = tfng_group("g1", "q1");
        group["taskType"] = json!("single_choice");
        let candidate = cp_candidate(json!([group]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let report = cp_report(true, &[("g1", true)], vec![cp_node("g1-i", "instruction", Some("g1"), &["r1"], true)]);
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
        assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()]);
        assert!(plan.flagged_groups.iter().any(|(id, reasons)| id == "g1" && reasons.iter().any(|r| r.contains("题型强反证"))));
        assert_eq!(entries_with(&plan, "task_type_counterevidence").len(), 1);
    }

    #[test]
    fn partial_question_coverage_is_reviewed_not_rejected_but_a_foreign_paper_keeps_the_local_draft() {
        let candidate = cp_candidate(json!([tfng_group("g1", "q1")]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let report = cp_report(true, &[("g1", true)], vec![cp_node("g1-i", "instruction", Some("g1"), &["r1"], true)]);
        // 本地 1、2、3，云端只有 1 与 2 的一部分：覆盖 1/3 < 一半 ⇒ 明显不是同一份卷子。
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1, 2, 3]), &report, None);
        assert!(plan.document_reasons.iter().any(|r| r.contains("整体不符")), "{:?}", plan.document_reasons);
        // 覆盖 1/2：不属于灾难性失败，缺失题号只进复核清单。
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1, 2]), &report, None);
        assert!(plan.document_reasons.is_empty(), "{:?}", plan.document_reasons);
        let uncovered = entries_with(&plan, "question_number_uncovered");
        assert_eq!(uncovered.len(), 1);
        assert_eq!(uncovered[0]["questionNumbers"], json!([2]));
        // 本地误识别的多余题号（云端多出 2）不阻止候选。
        let cloud_extra = cp_candidate(
            json!([tfng_group("g1", "q1"), tfng_group("g2", "q2")]),
            json!({"q1": {"slotId": "q1", "questionNumber": 1}, "q2": {"slotId": "q2", "questionNumber": 2}}),
        );
        let plan = plan_cloud_primary_adoption(&cloud_extra, &cp_local(&[1]), &report, None);
        assert!(plan.document_reasons.is_empty(), "{:?}", plan.document_reasons);
    }

    #[test]
    fn a_passage_that_barely_matches_the_source_is_a_catastrophic_failure() {
        let candidate = cp_candidate(json!([tfng_group("g1", "q1")]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let mut report = cp_report(false, &[("g1", true)], vec![]);
        report.passage_sentence_total = 20;
        report.passage_sentence_matched = 2;
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
        assert!(plan.document_reasons.iter().any(|r| r.contains("不是同一份卷子")), "{:?}", plan.document_reasons);
        let malformed = plan_cloud_primary_adoption(&json!({"authoring": null}), &cp_local(&[1]), &report, None);
        assert!(!malformed.document_reasons.is_empty());
    }

    #[test]
    fn instruction_and_stem_on_the_same_source_lines_are_reviewed() {
        let groups = json!([{
            "taskId": "g1", "taskType": "true_false_not_given",
            "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
            "responseGroups": [{"responseGroupId": "rg1", "prompt": [{"id": "p1", "type": "text", "text": "stem"}], "slotIds": ["q1"]}]
        }]);
        let candidate = cp_candidate(groups, json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let report = cp_report(
            true,
            &[("g1", true)],
            vec![
                cp_node("i1", "instruction", Some("g1"), &["shared-region"], true),
                cp_node("p1", "prompt", Some("g1"), &["shared-region"], true),
            ],
        );
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report, None);
        assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()]);
        assert!(plan.flagged_groups.iter().any(|(id, reasons)| id == "g1" && reasons.iter().any(|r| r.contains("同一区域"))));
        assert_eq!(entries_with(&plan, "instruction_stem_overlap").len(), 1);
    }

    #[test]
    fn unit_where_cloud_equals_local_is_adopted_without_any_alignment_review() {
        let local_doc = candidate()["authoring"].clone();
        let task_id = "early-approaches-q14-15";
        let instruction_id = local_doc["taskGroups"][0]["instructions"][0]["id"].as_str().unwrap().to_string();
        let mut severe = cp_node(&instruction_id, "instruction", Some(task_id), &["r1"], false);
        severe.min_similarity = 0.3;
        let report = cp_report(true, &[(task_id, false)], vec![severe.clone()]);
        let cloud = json!({"authoring": local_doc.clone()});
        let agreeing = plan_cloud_primary_adoption(&cloud, &local(), &report, Some(&local_doc));
        assert!(
            entries_with(&agreeing, "content_not_aligned").is_empty(),
            "云端与本地一致且原文抽样通过，原卷文字层的误差不应触发修复：{:?}",
            agreeing.needs_cloud_review
        );
        // 云端与本地不一致的同一单元仍按严重不符进复核。
        let mut changed = local_doc.clone();
        changed["taskGroups"][0]["instructions"][0]["children"][0]["text"] = json!("Cloud rewrote the instruction");
        let differing = plan_cloud_primary_adoption(&json!({"authoring": changed}), &local(), &report, Some(&local_doc));
        assert_eq!(entries_with(&differing, "content_not_aligned").len(), 1, "{:?}", differing.needs_cloud_review);
        // 原文抽样不过时，一致也不能免检。
        let failed_sample = cp_report(false, &[(task_id, false)], vec![severe]);
        let not_trusted = plan_cloud_primary_adoption(&cloud, &local(), &failed_sample, Some(&local_doc));
        assert_eq!(entries_with(&not_trusted, "content_not_aligned").len(), 1);
    }

    #[test]
    fn scanned_source_is_adopted_with_structural_review_only_unless_nothing_can_cross_check_it() {
        let candidate = cp_candidate(json!([tfng_group("g1", "q1")]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let plan = plan_adoption(&candidate, &cp_local(&[1]), &AlignmentOutcome::NoTextLayer, None);
        assert!(plan.document_reasons.is_empty(), "{:?}", plan.document_reasons);
        assert_eq!(plan.adoptable_task_ids, vec!["g1".to_string()]);
        assert!(plan.review_records.iter().any(|r| r["kind"] == "source_text_layer_unavailable"));
        let blind = plan_adoption(&candidate, &cp_local(&[]), &AlignmentOutcome::NoTextLayer, None);
        assert!(blind.document_reasons.iter().any(|r| r.contains("无法校验")), "{:?}", blind.document_reasons);
    }

    #[test]
    fn without_a_text_layer_only_real_differences_reach_repair_not_spacing_or_hyphenation() {
        let local_doc = candidate()["authoring"].clone();
        let mut cloud_doc = local_doc.clone();
        // 字距、连字符、空白、引号差异：原卷文字层提取误差，不进复核。
        cloud_doc["taskGroups"][0]["instructions"][0]["children"][0]["text"] = json!(
            local_doc["taskGroups"][0]["instructions"][0]["children"][0]["text"]
                .as_str()
                .unwrap()
                .replace(' ', "  ")
                .replace("which", "w h i c h")
        );
        assert!(unverified_review_entries(&local_doc, &cloud_doc).is_empty(), "{:?}", unverified_review_entries(&local_doc, &cloud_doc));
        // 答案真不一样：必须进复核，并定位到槽位。
        let mut answer_changed = local_doc.clone();
        answer_changed["answerKey"]["q14"] = json!({"kind":"option","labels":["D"],"assignment":"unordered_set"});
        let entries = unverified_review_entries(&local_doc, &answer_changed);
        assert!(entries.iter().any(|entry| entry["slotId"] == "q14"), "{entries:?}");
        // passage 整体差异不进题目单元复核。
        let mut passage_changed = local_doc.clone();
        passage_changed["passage"]["title"] = json!("A different title");
        assert!(unverified_review_entries(&local_doc, &passage_changed).is_empty());
    }

    #[test]
    fn stamp_alignment_provenance_only_marks_nodes_that_aligned() {
        let mut merged = json!({
            "passage": {"content": []},
            "taskGroups": [{"taskId": "g1", "instructions": [
                {"id": "i1", "type": "text", "text": "x", "provenanceStatus": "model_inferred"},
                {"id": "i2", "type": "text", "text": "y", "provenanceStatus": "model_inferred"}
            ]}]
        });
        let report = cp_report(true, &[("g1", true)], vec![
            cp_node("i1", "instruction", Some("g1"), &["r1"], true),
            cp_node("i2", "instruction", Some("g1"), &["r2"], false),
        ]);
        stamp_alignment_provenance(&mut merged, &report);
        assert_eq!(merged.pointer("/taskGroups/0/instructions/0/provenanceStatus"), Some(&json!("source")));
        assert!(merged
            .pointer("/taskGroups/0/instructions/0/sourceAnchors")
            .and_then(Value::as_array)
            .is_some_and(|anchors| !anchors.is_empty()));
        assert!(merged
            .pointer("/taskGroups/0/instructionSignature/evidenceAnchors")
            .and_then(Value::as_array)
            .is_some_and(|anchors| !anchors.is_empty()));
        assert_eq!(
            merged.pointer("/taskGroups/0/instructions/1/provenanceStatus"),
            Some(&json!("model_inferred")),
            "对不上的节点不得冒充 source"
        );
    }

    fn db_fixture(local_doc: &Value) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("cloud-adoption-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let conn = crate::library::repository::open_library_connection(&root).unwrap();
        conn.execute(
            "INSERT INTO library_items_v2 (id,modality,title,status,canonical_ds_json,created_at,updated_at) VALUES ('job-1','reading','Fixture','action_required',?1,'now','now')",
            [local_doc.to_string()],
        )
        .unwrap();
        root
    }

    #[test]
    fn every_renderable_unit_enters_the_editor_even_when_misaligned_and_a_broken_unit_stays_local() {
        let local_doc = candidate()["authoring"].clone();
        let root = db_fixture(&local_doc);
        let mut cloud = local_doc.clone();
        cloud["taskGroups"][0]["instructions"][0]["children"][0]["text"] = json!("Cloud rewrote the instruction");
        // 云端多出的一组引用了不存在的槽位：无法在编辑器渲染，必须留在本地一侧并报 deferred。
        cloud["taskGroups"].as_array_mut().unwrap().push(json!({
            "taskId": "broken-group", "taskType": "true_false_not_given",
            "displayRange": {"kind": "set", "values": [31]},
            "responseGroups": [{"responseGroupId": "broken-rg", "slotIds": ["missing-slot"]}]
        }));
        let report = cp_report(
            true,
            &[("early-approaches-q14-15", false)],
            vec![cp_node("x", "instruction", Some("early-approaches-q14-15"), &["r1"], false)],
        );
        let result = adopt_cloud_primary(&root, "job-1", "batch-1", 1, &cloud, Some(&report)).expect("adoption commits");
        assert_eq!(result.adopted_task_ids, vec!["early-approaches-q14-15".to_string()], "对齐不合格的单元也要先采用云端内容");
        assert_eq!(result.deferred_task_ids, vec!["broken-group".to_string()]);
        let conn = crate::library::repository::open_library_connection(&root).unwrap();
        let (saved, _) = crate::library::repository::get_canonical_ds(&conn, "job-1").unwrap().unwrap();
        assert_eq!(
            saved["taskGroups"][0]["instructions"][0]["children"][0]["text"],
            "Cloud rewrote the instruction"
        );
        assert!(
            !saved["taskGroups"].as_array().unwrap().iter().any(|group| group["taskId"] == "broken-group"),
            "无法渲染的单元不写进正式稿"
        );
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn units_present_only_locally_are_reported_so_they_reach_review() {
        let base = candidate()["authoring"].clone();
        // 本地多出一个独立单元（ids 全部换一套）：云端候选没有它。
        let rename = |text: String| text.replace("q14", "q44").replace("q15", "q45").replace("early-approaches", "other");
        let mut local_doc = base.clone();
        let mut extra_group: Value = serde_json::from_str(&rename(serde_json::to_string(&base["taskGroups"][0]).unwrap())).unwrap();
        // 题号与材料都不同，才不会被并进同一个依赖单元。
        extra_group["stimulus"] = json!([]);
        extra_group["displayRange"] = json!({"kind": "set", "values": [44, 45]});
        local_doc["taskGroups"].as_array_mut().unwrap().push(extra_group);
        for (from, to) in [("q14", "q44"), ("q15", "q45")] {
            let slot: Value = serde_json::from_str(&rename(serde_json::to_string(&base["answerSlots"][from]).unwrap())).unwrap();
            local_doc["answerSlots"][to] = slot;
            local_doc["answerSlots"][to]["questionNumber"] = json!(if to == "q44" { 44 } else { 45 });
            local_doc["answerKey"][to] = base["answerKey"][from].clone();
        }
        let root = db_fixture(&local_doc);
        let mut cloud = base.clone();
        cloud["taskGroups"][0]["instructions"][0]["children"][0]["text"] = json!("Cloud rewrote the instruction");
        let result = adopt_cloud_primary(&root, "job-1", "batch-1", 1, &cloud, None).expect("adoption commits");
        assert_eq!(result.adopted_task_ids, vec!["early-approaches-q14-15".to_string()]);
        assert_eq!(result.local_only_task_ids, vec!["other-q44-15".to_string()], "本地独有的单元保留并上报，不被静默删除");
        let conn = crate::library::repository::open_library_connection(&root).unwrap();
        let (saved, _) = crate::library::repository::get_canonical_ds(&conn, "job-1").unwrap().unwrap();
        assert!(saved["taskGroups"].as_array().unwrap().iter().any(|group| group["taskId"] == "other-q44-15"));
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }
}
