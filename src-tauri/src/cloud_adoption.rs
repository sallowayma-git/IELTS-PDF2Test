//! Deterministic qualification for promoting the complete cloud authoring candidate.
//!
//! This module deliberately does not trust model confidence or the report's aggregate score.
//! Eligibility is a backend decision over the normalized candidate, compiler probe, stable
//! question identities, and the hard-failure codes emitted by `ielts_grammar::quality`.

use crate::reconcile::alignment::{AlignmentOutcome, AlignmentReport};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The only hard failures that do not make an otherwise complete candidate unsafe to promote.
///
/// `quality.rs` also elevates recognition blockers dynamically, including unknown future codes.
/// Therefore this is intentionally an exemption list: every code not named here is blocking.
/// Answer-value findings can be resolved by later stages. Runtime compilation is exempted only
/// when its complete failure set consists of unresolved-answer findings; structural/compiler
/// errors remain blocking.
fn is_adoption_exempt_hard_failure(code: &str) -> bool {
    use crate::ielts_grammar::issue_codes::*;

    matches!(
        code,
        ANSWER_KEY_MISSING_SLOT
            | ANSWER_WORD_LIMIT_VIOLATION
            | ANSWER_OPTION_NOT_IN_BANK
            | V1_COMPATIBILITY_COMPILER_FAILED
    )
}

fn runtime_failed_only_for_unresolved_answers(authoring: &Value) -> bool {
    let Some(probe) = authoring.pointer("/quality/compilerProbes/v2Runtime") else {
        return false;
    };
    if probe.get("status").and_then(Value::as_str) != Some("failed") {
        return false;
    }
    let codes = probe
        .get("issueCodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let details = probe
        .get("details")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    !codes.is_empty()
        && codes
            .iter()
            .all(|code| *code == "RUNTIME_ANSWER_UNRESOLVED")
        && !details.is_empty()
        && details
            .iter()
            .all(|detail| detail.starts_with("RUNTIME_ANSWER_UNRESOLVED:"))
}

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

/// 文档级（整份）不可采纳原因：状态 / 未解析引用 / 运行时编译 / 题号覆盖。
/// **不含**按题组的质量硬阻断——后者在 [`plan_group_adoption`] 里按题组归并。
fn document_rejection_reasons(candidate: &Value, local: &Value) -> Vec<String> {
    let mut reasons = Vec::new();
    let authoring = candidate.get("authoring").unwrap_or(&Value::Null);

    if candidate.get("status").and_then(Value::as_str) != Some("succeeded") {
        reasons.push("云端候选未完整归一化成功".to_string());
    }
    if candidate
        .get("unresolvedReferences")
        .and_then(Value::as_array)
        .is_some_and(|references| !references.is_empty())
    {
        reasons.push("云端候选仍有未解析的结构引用".to_string());
    }
    let answer_only_runtime_failure = runtime_failed_only_for_unresolved_answers(authoring);
    let runtime_passed = authoring
        .pointer("/quality/compilerProbes/v2Runtime/status")
        .and_then(Value::as_str)
        == Some("passed");
    if !runtime_passed && !answer_only_runtime_failure {
        reasons.push("云端候选未通过学生端 V2 运行时编译".to_string());
    }

    let local_numbers = local
        .get("slots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|slot| slot.get("questionNumber").and_then(Value::as_u64))
        .filter_map(|number| u32::try_from(number).ok())
        .collect::<BTreeSet<_>>();
    let (candidate_numbers, duplicate_candidate_numbers) = candidate_question_numbers(authoring);
    if local_numbers.is_empty() {
        reasons.push("冻结的本地候选没有可核对的题号".to_string());
    }
    if duplicate_candidate_numbers {
        reasons.push("云端候选的答案位重复声明了题号".to_string());
    }
    let missing = local_numbers
        .difference(&candidate_numbers)
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        reasons.push(format!(
            "云端候选未覆盖本地题号：{}",
            format_question_numbers(&missing)
        ));
    }
    let extra = candidate_numbers
        .difference(&local_numbers)
        .copied()
        .collect::<Vec<_>>();
    if !extra.is_empty() {
        let declared = instruction_question_numbers(authoring);
        let undeclared = extra
            .iter()
            .copied()
            .filter(|number| !declared.contains(number))
            .collect::<Vec<_>>();
        if !undeclared.is_empty() {
            reasons.push(format!(
                "云端新增题号不在候选自身的说明区范围内：{}",
                format_question_numbers(&undeclared)
            ));
        }
    }

    reasons
}

/// 非豁免的阻塞硬阻断码集合（`RUNTIME_COMPILER_FAILED` 在仅剩答案未解时豁免）。
fn blocking_hard_failure_codes(authoring: &Value) -> BTreeSet<String> {
    let answer_only_runtime_failure = runtime_failed_only_for_unresolved_answers(authoring);
    authoring
        .pointer("/quality/hardFailures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|code| {
            !is_adoption_exempt_hard_failure(code)
                && !(*code == "RUNTIME_COMPILER_FAILED" && answer_only_runtime_failure)
        })
        .map(str::to_string)
        .collect()
}

/// Return stable, user-readable reasons when the candidate must not be adopted **as a whole**.
pub(crate) fn adoption_rejection_reasons(candidate: &Value, local: &Value) -> Vec<String> {
    let mut reasons = document_rejection_reasons(candidate, local);
    let authoring = candidate.get("authoring").unwrap_or(&Value::Null);
    let blocking = blocking_hard_failure_codes(authoring);
    if !blocking.is_empty() {
        reasons.push(format!(
            "云端候选存在不可豁免的质量硬阻断：{}",
            blocking.into_iter().collect::<Vec<_>>().join("、")
        ));
    }
    reasons
}

/// 候选题组身份是否真实存在于候选稿里。
fn group_exists(authoring: &Value, task_id: &str) -> bool {
    authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|group| group.get("taskId").and_then(Value::as_str) == Some(task_id))
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

/// 阻塞质量问题按题组归并。返回（题组 taskId → 原因）与无法归属到任何题组的**文档级**原因。
/// 归属规则：`task` 目标即 taskId；`response_group` / `slot` 目标经引用查其所属题组；
/// 归不到题组的（passage、整体覆盖、answerKey 结构等）落文档级——不静默忽略。
fn hard_failures_by_group(candidate: &Value) -> (BTreeMap<String, Vec<String>>, Vec<String>) {
    let authoring = candidate.get("authoring").unwrap_or(&Value::Null);
    let answer_only_runtime_failure = runtime_failed_only_for_unresolved_answers(authoring);
    let mut by_group: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut document_level: Vec<String> = Vec::new();
    for issue in authoring
        .pointer("/quality/issues")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if issue.get("severity").and_then(Value::as_str) != Some("blocking") {
            continue;
        }
        let code = issue.get("code").and_then(Value::as_str).unwrap_or_default();
        if is_adoption_exempt_hard_failure(code)
            || (code == "RUNTIME_COMPILER_FAILED" && answer_only_runtime_failure)
        {
            continue;
        }
        let target_type = issue.get("targetType").and_then(Value::as_str).unwrap_or("");
        let target_id = issue.get("targetId").and_then(Value::as_str).unwrap_or("");
        let message = issue.get("message").and_then(Value::as_str).unwrap_or(code);
        let owners: BTreeSet<String> = match target_type {
            "task" if group_exists(authoring, target_id) => {
                BTreeSet::from([target_id.to_string()])
            }
            "response_group" => group_ids_for_reference(authoring, target_id),
            "slot" => group_ids_owning_slot(authoring, target_id),
            _ => BTreeSet::new(),
        };
        let reason = format!("{code}：{message}");
        if owners.is_empty() {
            document_level.push(reason);
        } else {
            for owner in owners {
                by_group.entry(owner).or_default().push(reason.clone());
            }
        }
    }
    (by_group, document_level)
}

/// 收集候选 passage 里所有节点 id（含 paragraphMap 的目标 id）。最终文档保留本地 passage，
/// 候选 passage 即本地 passage，用它判定 heading 宿主段落是否可映射。
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

/// heading 宿主段落映射检查：引用了本地 passage 里不存在的段落 id 的题组判为不合格。
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
                    format!("题组的 heading 宿主段落 {host_id} 在本地原文里不存在，无法映射"),
                ));
            }
        }
    }
    failures
}

/// 按题组采纳的判定结果。`document_reasons` 非空 ⇒ 整份不采纳（有归不到题组的硬阻断）。
#[derive(Debug)]
pub(crate) struct GroupAdoptionPlan {
    pub document_reasons: Vec<String>,
    pub qualified_task_ids: Vec<String>,
    pub unqualified: Vec<(String, Vec<String>)>,
}

/// 逐题组评估采纳：文档级阻断整份拒；否则合格题组采纳、不合格保留本地并逐条给原因。
pub(crate) fn plan_group_adoption(candidate: &Value, local: &Value) -> GroupAdoptionPlan {
    let authoring = candidate.get("authoring").unwrap_or(&Value::Null);
    let mut document_reasons = document_rejection_reasons(candidate, local);
    let (mut group_failures, document_level_hard) = hard_failures_by_group(candidate);
    document_reasons.extend(document_level_hard);
    for (task_id, reason) in groups_with_unmapped_passage_hosts(authoring) {
        group_failures.entry(task_id).or_default().push(reason);
    }
    let qualified_task_ids = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("taskId").and_then(Value::as_str))
        .filter(|task_id| !group_failures.contains_key(*task_id))
        .map(str::to_string)
        .collect();
    GroupAdoptionPlan {
        document_reasons,
        qualified_task_ids,
        unqualified: group_failures.into_iter().collect(),
    }
}

// ---------------------------------------------------------------------------
// 云端为主的采纳（依赖本地对齐校验：云端内容对上原卷即整体覆盖本地）
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


/// 云端为主的采纳判定。原文与各题组独立决定；`document_reasons` 非空表示文档级致命问题、整份拒。
#[derive(Debug)]
pub(crate) struct CloudPrimaryPlan {
    /// 是否整体采用云端原文（段落切分/标签/paragraphMap）；否则保留本地原文。
    pub adopt_passage: bool,
    pub passage_reasons: Vec<String>,
    pub qualified_task_ids: Vec<String>,
    pub unqualified: Vec<(String, Vec<String>)>,
    pub document_reasons: Vec<String>,
    /// 采纳但需人复核的记录（题型规则不一致仍采纳、非题目内容未覆盖等）。
    pub review_records: Vec<Value>,
    /// 交给云端校核的清单雏形（对齐失败组、题型强反证、说明吞题）。
    pub needs_cloud_review: Vec<Value>,
}

/// 采纳计划：有文本层走云端为主；扫描卷无文本层退回保守的按题组（本地基底）。
pub(crate) enum AdoptionPlan {
    Conservative(GroupAdoptionPlan),
    CloudPrimary(CloudPrimaryPlan),
}

pub(crate) fn plan_adoption(
    candidate: &Value,
    local: &Value,
    outcome: &AlignmentOutcome,
) -> AdoptionPlan {
    match outcome {
        AlignmentOutcome::NoTextLayer => AdoptionPlan::Conservative(plan_group_adoption(candidate, local)),
        AlignmentOutcome::Assessed(report) => {
            AdoptionPlan::CloudPrimary(plan_cloud_primary_adoption(candidate, local, report))
        }
    }
}

/// heading 宿主段落可映射性。采纳云端原文时校验云端 passage；不采纳时暂保守——
/// 有 passage_paragraph 宿主的组交人工/云端复核（按标签映射本地待后续细化）。
fn heading_host_failures(authoring: &Value, adopt_passage: bool) -> Vec<(String, String)> {
    if adopt_passage {
        return groups_with_unmapped_passage_hosts(authoring);
    }
    let mut failures = Vec::new();
    for (slot_id, slot) in authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if slot.get("hostType").and_then(Value::as_str) == Some("passage_paragraph") {
            for owner in group_ids_owning_slot(authoring, slot_id) {
                failures.push((owner, "未采纳云端原文，段落宿主需按标签映射本地或人工复核".to_string()));
            }
        }
    }
    failures
}


/// 逐节点/逐题组按对齐结果决定采纳，云端为主。
pub(crate) fn plan_cloud_primary_adoption(
    candidate: &Value,
    local: &Value,
    alignment: &AlignmentReport,
) -> CloudPrimaryPlan {
    let authoring = candidate.get("authoring").unwrap_or(&Value::Null);
    let mut document_reasons = document_rejection_reasons(candidate, local);
    let (mut group_failures, document_level_hard) = hard_failures_by_group(candidate);
    document_reasons.extend(document_level_hard);

    let adopt_passage = alignment.passage_pass
        && alignment.monotonic
        && alignment.coverage_ok
        && alignment.length_ratio_ok
        && alignment.passage_invented_ok;
    let mut passage_reasons = Vec::new();
    if !adopt_passage {
        if !alignment.passage_pass {
            passage_reasons.push(format!(
                "原文命中率不足：{}/{}",
                alignment.passage_sentence_matched, alignment.passage_sentence_total
            ));
        }
        if !alignment.monotonic {
            passage_reasons.push(format!("命中顺序非单调（逆序 {} 处）", alignment.order_violations));
        }
        if !alignment.coverage_ok {
            passage_reasons.push(format!("显著源区域覆盖不足：{:.2}", alignment.coverage));
        }
        if !alignment.length_ratio_ok {
            passage_reasons.push(format!("云端原文长度比异常：{:.2}", alignment.length_ratio));
        }
        if !alignment.passage_invented_ok {
            passage_reasons.push(format!(
                "原文凭空句超出容忍：{} 句",
                alignment.passage_invented.len()
            ));
        }
    }

    // Shared parent regions can contain both roles; only shared physical lines imply swallowed text.
    let mut instruction_regions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut prompt_regions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for node in &alignment.nodes {
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

    let mut review_records = Vec::new();
    let mut needs_cloud_review = Vec::new();
    for group in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(task_id) = group.get("taskId").and_then(Value::as_str) else {
            continue;
        };
        if group_failures.contains_key(task_id) {
            continue;
        }
        // A single invented sentence needs repair even when its siblings keep the group ratio high.
        for node in alignment.nodes.iter().filter(|node| {
            node.task_id.as_deref() == Some(task_id)
                && matches!(node.kind_label, "instruction" | "prompt" | "option")
                && node.min_similarity < 0.6
        }) {
            let mut target = serde_json::json!({
                "taskId": task_id, "nodeId": node.node_id,
                "reason": "content_not_aligned", "similarity": node.min_similarity,
            });
            if matches!(node.kind_label, "prompt" | "option") {
                let owners: Vec<&Value> = group.get("responseGroups")
                    .and_then(Value::as_array).into_iter().flatten()
                    .filter(|response| ["prompt", "options"].iter().any(|field| {
                        response.get(*field).and_then(Value::as_array).into_iter().flatten()
                            .any(|entry| ["id", "optionId"].iter().any(|key| {
                                entry.get(*key).and_then(Value::as_str) == Some(node.node_id.as_str())
                            }))
                    })).collect();
                if owners.len() == 1 {
                    if let Some(slots) = owners[0].get("slotIds").and_then(Value::as_array) {
                        if slots.len() == 1 {
                            target["slotId"] = slots[0].clone();
                        }
                    }
                }
            }
            needs_cloud_review.push(target);
        }
        // 题目内容（说明/题干/选项）未对齐 → 该组不采纳，进云端校核清单。
        if alignment.group_aligned.get(task_id) == Some(&false) {
            let mut reasons = alignment
                .group_reasons
                .get(task_id)
                .cloned()
                .unwrap_or_default();
            if reasons.is_empty() {
                reasons.push("题目内容未对齐原卷".to_string());
            }
            group_failures.entry(task_id.to_string()).or_default().extend(reasons);
            needs_cloud_review.push(serde_json::json!({"taskId": task_id, "reason": "content_not_aligned"}));
            continue;
        }
        // 说明与题干/选项落在同一源区域：疑似说明被识别成题目、或题目被吞进说明。
        let overlap = instruction_regions
            .get(task_id)
            .zip(prompt_regions.get(task_id))
            .is_some_and(|(instr, prompt)| !instr.is_disjoint(prompt));
        if overlap {
            group_failures
                .entry(task_id.to_string())
                .or_default()
                .push("说明与题干在原卷上落于同一区域（疑似说明吞题或题目被吞进说明）".to_string());
            needs_cloud_review.push(serde_json::json!({"taskId": task_id, "reason": "instruction_stem_overlap"}));
            continue;
        }
        // 题型：以云端为准，本地规则复核；仅强反证不采纳该组题型。
        match recheck_task_type(group) {
            TypeVerdict::StrongCounter { detected, declared } => {
                group_failures.entry(task_id.to_string()).or_default().push(format!(
                    "题型强反证：说明判为 {detected}，云端声明 {declared}"
                ));
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
                    "note": "采纳云端题型；本地规则判型不一致但云端说明已对齐原卷"
                }));
            }
            TypeVerdict::Accept => {}
        }
    }


    for (task_id, reason) in heading_host_failures(authoring, adopt_passage) {
        group_failures.entry(task_id).or_default().push(reason);
    }

    // 原文整体采纳、但含少量凭空句：原文照采，这些句子单列为修复目标，交修复循环核对原卷。
    if adopt_passage {
        for entry in &alignment.passage_invented {
            let node_id = entry.get("nodeId").and_then(Value::as_str).unwrap_or_default();
            needs_cloud_review.push(serde_json::json!({
                "nodeId": node_id,
                "reason": "passage_sentence_unverified",
                "similarity": entry.get("similarity").cloned().unwrap_or(Value::Null),
            }));
        }
    }

    let qualified_task_ids: Vec<String> = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("taskId").and_then(Value::as_str))
        .filter(|task_id| !group_failures.contains_key(*task_id))
        .map(str::to_string)
        .collect();

    // 采纳后仍未解析的计分答案，以及「候选带答案页证据却与答案键冲突」——都是大差异，
    // 进云端校核清单（否则会留下一份看似已采纳、答案却缺失或自相矛盾的稿）。
    let qualified_set: BTreeSet<&str> = qualified_task_ids.iter().map(String::as_str).collect();
    let answer_key = authoring.get("answerKey").and_then(Value::as_object);
    let answer_slots = authoring.get("answerSlots").and_then(Value::as_object);
    let slot_in_qualified = |slot_id: &str| -> bool {
        group_ids_owning_slot(authoring, slot_id)
            .iter()
            .any(|owner| qualified_set.contains(owner.as_str()))
    };
    let answer_resolved = |slot_id: &str| -> bool {
        answer_key
            .and_then(|map| map.get(slot_id))
            .is_some_and(|answer| answer.get("kind").and_then(Value::as_str) != Some("unresolved"))
    };
    for (slot_id, slot) in answer_slots.into_iter().flatten() {
        if slot.get("participation").and_then(Value::as_str) != Some("scoring") {
            continue;
        }
        if slot_in_qualified(slot_id) && !answer_resolved(slot_id) {
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
        if slot_in_qualified(&slot_id)
            && answer_resolved(&slot_id)
            && answer_key.and_then(|map| map.get(&slot_id)) != Some(evidence_answer)
        {
            needs_cloud_review.push(
                serde_json::json!({"slotId": slot_id, "reason": "answer_conflicts_answer_page"}),
            );
        }
    }

    CloudPrimaryPlan {
        adopt_passage,
        passage_reasons,
        qualified_task_ids,
        unqualified: group_failures.into_iter().collect(),
        document_reasons,
        review_records,
        needs_cloud_review,
    }
}


/// 已采纳（合格题组）覆盖的题号清单，供采纳记录与后续修复循环使用。
pub(crate) fn adopted_question_numbers(authoring: &Value, qualified_task_ids: &[String]) -> Vec<u32> {
    let qualified: BTreeSet<&str> = qualified_task_ids.iter().map(String::as_str).collect();
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
        if qualified.contains(task_id) {
            numbers.extend(group_question_numbers(authoring, group));
        }
    }
    numbers.into_iter().collect()
}

/// 以云端为基底组装采纳后文档：不采纳云端原文时换回本地 passage；不合格题组换回本地题组。
fn build_cloud_primary_document(current: &Value, cloud: &Value, plan: &CloudPrimaryPlan) -> Value {
    let mut merged = cloud.clone();
    if !plan.adopt_passage {
        match current.get("passage") {
            Some(passage) => {
                merged["passage"] = passage.clone();
            }
            None => {
                if let Some(map) = merged.as_object_mut() {
                    map.remove("passage");
                }
            }
        }
    }
    // 不合格题组：把本地题组（含其槽位与答案）换回到云端基底上。复用按题组换入原语。
    let restore: Vec<String> = plan.unqualified.iter().map(|(id, _)| id.clone()).collect();
    adopt_qualified_groups(&mut merged, current, &restore);
    merged
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

/// 把对齐产出的真实锚点盖到已采纳的云端节点上（采纳云端原文时含 passage，合格题组含说明/
/// 题干/选项/槽位），并用说明节点锚点补齐题组的 instructionSignature.evidenceAnchors，
/// 使既有来源类门禁在真实对齐证据上放行。
fn stamp_alignment_provenance(
    merged: &mut Value,
    alignment: &AlignmentReport,
    adopt_passage: bool,
    qualified: &BTreeSet<String>,
) {
    let mut group_evidence: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for node in &alignment.nodes {
        if !node.aligned || node.anchors.is_empty() || node.node_id.is_empty() {
            continue;
        }
        let adopted = match &node.task_id {
            None => adopt_passage,
            Some(task_id) => qualified.contains(task_id),
        };
        if !adopted {
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


/// 以云端为主提交采纳：云端整体覆盖本地（原文不合格换回本地原文、不合格题组换回本地题组），
/// 已采纳的云端节点盖上真实对齐锚点。走与人工编辑一致的 CAS/journal 事务，版本冲突重试。
pub(crate) fn adopt_cloud_primary(
    root: &Path,
    item_id: &str,
    batch_id: &str,
    candidate_base_version: i64,
    cloud_authoring: &Value,
    plan: &CloudPrimaryPlan,
    alignment: &AlignmentReport,
) -> Result<AdoptionCommit, String> {
    let run_id = crate::cloud_repair::repair_run_id_for(batch_id);
    let request_id = format!("cloud-candidate-adoption:{batch_id}");
    let qualified: BTreeSet<String> = plan.qualified_task_ids.iter().cloned().collect();
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
        // 云端为主：以云端 authoring 为基底组装，再盖真实对齐锚点，最后 rebase 用户编辑。
        let mut merged = build_cloud_primary_document(&current, cloud_authoring, plan);
        stamp_alignment_provenance(&mut merged, alignment, plan.adopt_passage, &qualified);
        let preserved_group_ids = merge_human_edits(&current, &mut merged, &journal)?;
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
            audit.insert("source".to_string(), serde_json::json!("cloud_candidate_adoption"));
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

#[derive(Debug)]
pub(crate) struct AdoptionCommit {
    pub edit_version: i64,
    pub preserved_group_ids: Vec<String>,
    pub applied_targets: Vec<String>,
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

/// 一个题组声明的答案槽 id（来自 responseGroups[].slotIds）。
fn group_slot_ids(group: &Value) -> Vec<String> {
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
                .filter_map(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

/// 以本地稿为基底，把**合格**的云端题组换入：替换/新增题组，并覆盖该组槽位的
/// answerSlots 与 answerKey（optionBank 随题组一起换入）。passage、paragraphMap 及其它
/// 非题组字段一律保留基底（本地），不被云端字段污染。
fn adopt_qualified_groups(base: &mut Value, cloud: &Value, qualified: &[String]) {
    for task_id in qualified {
        let Some(cloud_group) = cloud
            .get("taskGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|group| group.get("taskId").and_then(Value::as_str) == Some(task_id.as_str()))
            .cloned()
        else {
            continue;
        };
        let slot_ids = group_slot_ids(&cloud_group);

        let base_groups = base
            .get_mut("taskGroups")
            .and_then(Value::as_array_mut)
            .expect("本地稿必有 taskGroups 数组");
        match base_groups
            .iter()
            .position(|group| group.get("taskId").and_then(Value::as_str) == Some(task_id.as_str()))
        {
            Some(index) => base_groups[index] = cloud_group,
            None => base_groups.push(cloud_group),
        }

        for slot_id in &slot_ids {
            if let Some(slot) = cloud.pointer(&format!("/answerSlots/{slot_id}")).cloned() {
                if let Some(slots) = base.get_mut("answerSlots").and_then(Value::as_object_mut) {
                    slots.insert(slot_id.clone(), slot);
                }
            }
            if let Some(keys) = base.get_mut("answerKey").and_then(Value::as_object_mut) {
                match cloud.pointer(&format!("/answerKey/{slot_id}")) {
                    Some(answer) => {
                        keys.insert(slot_id.clone(), answer.clone());
                    }
                    None => {
                        keys.remove(slot_id);
                    }
                }
            }
        }
    }
}

/// Promote a qualified full candidate using the canonical editor CAS/journal transaction.
/// A version conflict is retried from a fresh canonical + human journal snapshot.
pub(crate) fn adopt_cloud_candidate(
    root: &Path,
    item_id: &str,
    batch_id: &str,
    candidate_base_version: i64,
    cloud_authoring: &Value,
    qualified_task_ids: &[String],
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
        let mut merged = current.clone();
        // 以本地为基底，只换入合格的云端题组；passage / paragraphMap / 其它非题组字段保留本地。
        adopt_qualified_groups(&mut merged, cloud_authoring, qualified_task_ids);
        let preserved_group_ids = merge_human_edits(&current, &mut merged, &journal)?;
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

fn format_question_numbers(numbers: &[u32]) -> String {
    numbers
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join("、")
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
    fn answer_value_and_v1_compatibility_failures_are_exempt_but_other_hard_failures_reject() {
        let mut cloud = candidate();
        for code in [
            "ANSWER_KEY_MISSING_SLOT",
            "ANSWER_WORD_LIMIT_VIOLATION",
            "ANSWER_OPTION_NOT_IN_BANK",
            "V1_COMPATIBILITY_COMPILER_FAILED",
        ] {
            cloud["authoring"]["quality"]["hardFailures"] = json!([code]);
            assert!(
                adoption_rejection_reasons(&cloud, &local()).is_empty(),
                "expected {code} to be exempt"
            );
        }

        for code in [
            "PROVENANCE_MISSING",
            "QUESTION_BLOCK_MISSING",
            "MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED",
            "VISUAL_FALLBACK_ASSET_NOT_MATERIALIZED",
            "SOME_NEW_BLOCKER",
        ] {
            cloud["authoring"]["quality"]["hardFailures"] = json!([code]);
            let reasons = adoption_rejection_reasons(&cloud, &local());
            assert!(
                reasons.iter().any(|reason| reason.contains(code)),
                "expected {code} to reject adoption"
            );
        }
    }

    #[test]
    fn candidate_must_cover_local_questions_and_extras_must_be_in_its_own_instructions() {
        let mut cloud = candidate();
        cloud["authoring"]["answerSlots"]
            .as_object_mut()
            .unwrap()
            .remove("q15");
        let reasons = adoption_rejection_reasons(&cloud, &local());
        assert!(reasons
            .iter()
            .any(|reason| reason.contains("未覆盖本地题号：15")));

        let mut cloud = candidate();
        let mut slot = cloud["authoring"]["answerSlots"]["q15"].clone();
        slot["slotId"] = json!("q16");
        slot["questionNumber"] = json!(16);
        cloud["authoring"]["answerSlots"]
            .as_object_mut()
            .unwrap()
            .insert("q16".into(), slot);
        let reasons = adoption_rejection_reasons(&cloud, &local());
        assert!(reasons
            .iter()
            .any(|reason| reason.contains("新增题号不在候选自身的说明区范围内：16")));

        cloud["authoring"]["taskGroups"][0]["instructionSignature"]["expectedQuestionNumbers"] =
            json!([14, 15, 16]);
        assert!(adoption_rejection_reasons(&cloud, &local()).is_empty());
    }

    #[test]
    fn partial_or_uncompiled_candidate_is_never_adopted() {
        let mut cloud = candidate();
        cloud["status"] = json!("partial");
        cloud["authoring"]["quality"]["compilerProbes"]["v2Runtime"]["status"] = json!("failed");
        let reasons = adoption_rejection_reasons(&cloud, &local());
        assert!(reasons.iter().any(|reason| reason.contains("未完整归一化")));
        assert!(reasons.iter().any(|reason| reason.contains("运行时编译")));
    }

    #[test]
    fn only_unresolved_answer_failures_do_not_block_cloud_adoption() {
        let mut cloud = candidate();
        cloud["authoring"]["answerKey"]["q14"] = json!({"kind":"unresolved"});
        cloud["authoring"]["quality"]["compilerProbes"]["v2Runtime"] = json!({
            "status": "failed",
            "issueCodes": ["RUNTIME_ANSWER_UNRESOLVED"],
            "details": ["RUNTIME_ANSWER_UNRESOLVED:q14:No printed answer is available."]
        });
        cloud["authoring"]["quality"]["hardFailures"] = json!(["RUNTIME_COMPILER_FAILED"]);

        assert!(
            adoption_rejection_reasons(&cloud, &local()).is_empty(),
            "答案未解析应交给后续答案页步骤，不得阻止结构合格候选整体采纳"
        );
    }

    #[test]
    fn runtime_failure_with_any_non_answer_error_still_rejects_cloud_adoption() {
        let mut cloud = candidate();
        cloud["authoring"]["quality"]["compilerProbes"]["v2Runtime"] = json!({
            "status": "failed",
            "issueCodes": ["RUNTIME_ANSWER_UNRESOLVED", "RUNTIME_OPTION_CONTENT_INVALID"],
            "details": [
                "RUNTIME_ANSWER_UNRESOLVED:q14:No printed answer is available.",
                "RUNTIME_OPTION_CONTENT_INVALID:option-a:Option content is not renderable."
            ]
        });
        cloud["authoring"]["quality"]["hardFailures"] = json!(["RUNTIME_COMPILER_FAILED"]);

        let reasons = adoption_rejection_reasons(&cloud, &local());
        assert!(reasons.iter().any(|reason| reason.contains("运行时编译")));
        assert!(reasons
            .iter()
            .any(|reason| reason.contains("RUNTIME_COMPILER_FAILED")));
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

    // T4 按题组采纳判定。

    #[test]
    fn cloudfix_all_groups_qualified_when_no_blocking_issue() {
        let mut cloud = candidate();
        cloud["authoring"]["quality"]["issues"] = json!([]);
        let plan = plan_group_adoption(&cloud, &local());
        assert!(
            plan.document_reasons.is_empty(),
            "不该有文档级原因：{:?}",
            plan.document_reasons
        );
        assert!(plan.unqualified.is_empty());
        assert!(plan
            .qualified_task_ids
            .iter()
            .any(|id| id == "early-approaches-q14-15"));
    }

    #[test]
    fn cloudfix_group_hard_block_does_not_sink_other_qualified_groups() {
        let mut cloud = candidate();
        cloud["authoring"]["taskGroups"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "taskId": "group-2",
                "responseGroups": [{"responseGroupId":"group-2-rg","slotIds":[]}]
            }));
        cloud["authoring"]["quality"]["issues"] = json!([{
            "code":"PROVENANCE_MISSING","severity":"blocking",
            "targetType":"task","targetId":"early-approaches-q14-15",
            "message":"prompt/stimulus 缺少 source anchor"
        }]);
        let plan = plan_group_adoption(&cloud, &local());
        assert!(
            plan.document_reasons.is_empty(),
            "组级阻断不应升级成文档级：{:?}",
            plan.document_reasons
        );
        assert!(
            plan.unqualified
                .iter()
                .any(|(id, _)| id == "early-approaches-q14-15"),
            "有阻断的组应不合格：{:?}",
            plan.unqualified
        );
        assert!(
            plan.qualified_task_ids.iter().any(|id| id == "group-2"),
            "合格组不应被拖垮：{:?}",
            plan.qualified_task_ids
        );
        assert!(!plan
            .qualified_task_ids
            .iter()
            .any(|id| id == "early-approaches-q14-15"));
    }

    #[test]
    fn cloudfix_document_level_block_rejects_whole_candidate() {
        let mut cloud = candidate();
        cloud["authoring"]["quality"]["issues"] = json!([{
            "code":"SIGNIFICANT_REGION_UNASSIGNED","severity":"blocking",
            "targetType":"recognition","targetId":"document",
            "message":"显著源节点未认领"
        }]);
        let plan = plan_group_adoption(&cloud, &local());
        assert!(
            plan.document_reasons
                .iter()
                .any(|reason| reason.contains("SIGNIFICANT_REGION_UNASSIGNED")),
            "归不到题组的阻断必须落文档级、整份拒：{:?}",
            plan.document_reasons
        );
    }

    #[test]
    fn cloudfix_heading_group_with_unmapped_passage_host_is_unqualified() {
        let mut cloud = candidate();
        cloud["authoring"]["quality"]["issues"] = json!([]);
        cloud["authoring"]["answerSlots"]["q14"]["hostType"] = json!("passage_paragraph");
        cloud["authoring"]["answerSlots"]["q14"]["hostNodeId"] = json!("invented-paragraph");
        let plan = plan_group_adoption(&cloud, &local());
        assert!(
            plan.unqualified.iter().any(|(id, reasons)| id == "early-approaches-q14-15"
                && reasons.iter().any(|reason| reason.contains("invented-paragraph"))),
            "heading 宿主段落映射不上的组应不合格：{:?}",
            plan.unqualified
        );
    }

    // ------- 云端为主采纳（M2）-------

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


    #[test]
    fn cloud_primary_does_not_confuse_shared_region_with_swallowed_question() {
        let candidate = cp_candidate(json!([{
            "taskId": "g1", "taskType": "true_false_not_given",
            "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
            "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1"]}]
        }]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let mut instruction = cp_node("i1", "instruction", Some("g1"), &["shared-region", "line-1"], true);
        let mut prompt = cp_node("p1", "prompt", Some("g1"), &["shared-region", "line-2"], true);
        instruction.source_line_ids = BTreeSet::from(["line-1".to_string()]);
        prompt.source_line_ids = BTreeSet::from(["line-2".to_string()]);
        let report = cp_report(true, &[("g1", true)], vec![instruction, prompt]);
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report);
        assert_eq!(plan.qualified_task_ids, vec!["g1".to_string()]);
        assert!(plan.needs_cloud_review.is_empty());
    }

    #[test]
    fn cloud_primary_flags_low_similarity_question_nodes_without_rejecting_siblings() {
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
            // Nine of ten sentences align, so both the node and its group pass the aggregate
            // ratio despite one sentence being below the independent review threshold.
            suspect.sentence_count = 10;
            suspect.matched_sentences = 9;
            suspect.min_similarity = 0.55;
            suspect.mean_similarity = 0.955;
            suspect.has_invented = true;
            let report = cp_report(true, &[("g1", true)], vec![
                suspect, cp_node("p2", "prompt", Some("g1"), &["r2"], true),
            ]);
            let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1, 2]), &report);
            assert!(plan.document_reasons.is_empty(), "{:?}", plan.document_reasons);
            assert_eq!(plan.qualified_task_ids, vec!["g1".to_string()]);
            assert!(plan.unqualified.is_empty(), "同组其他题仍应采纳");
            let targets = plan.needs_cloud_review.iter().filter(|entry| {
                entry.get("reason").and_then(Value::as_str) == Some("content_not_aligned")
            }).collect::<Vec<_>>();
            assert_eq!(targets.len(), 1, "{kind} 的低相似句必须独立进复核：{:?}", plan.needs_cloud_review);
            assert_eq!(targets[0].get("taskId").and_then(Value::as_str), Some("g1"));
            assert_eq!(targets[0].get("nodeId").and_then(Value::as_str), Some(node_id));
            assert_eq!(targets[0].get("slotId").and_then(Value::as_str), expected_slot);
            assert_eq!(targets[0].get("similarity"), Some(&json!(0.55)));
            let merged = build_cloud_primary_document(
                &json!({"passage": {"content": []}, "taskGroups": [], "answerSlots": {}, "answerKey": {}}),
                candidate.get("authoring").unwrap(), &plan,
            );
            assert_eq!(merged.pointer("/taskGroups/0/responseGroups/1/prompt/0/text"), Some(&json!("cloud second question")));
        }
    }

    #[test]
    fn cloud_primary_flags_low_similarity_shared_option_by_node() {
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
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1, 2]), &report);
        assert_eq!(plan.qualified_task_ids, vec!["g1".to_string()]);
        assert!(plan.needs_cloud_review.iter().any(|entry| {
            entry.get("reason").and_then(Value::as_str) == Some("content_not_aligned")
                && entry.get("taskId").and_then(Value::as_str) == Some("g1")
                && entry.get("nodeId").and_then(Value::as_str) == Some("bank-o1")
                && entry.get("slotId").is_none()
        }), "共享选项应按节点复核，不误绑单题：{:?}", plan.needs_cloud_review);
    }

    #[test]
    fn cloud_primary_keeps_review_threshold_strict_and_group_rejection_independent() {
        let candidate = cp_candidate(json!([{
            "taskId": "g1", "taskType": "true_false_not_given",
            "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
            "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1"]}]
        }]), json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        for similarity in [0.6, 0.7, 0.8] {
            let mut node = cp_node("i1", "instruction", Some("g1"), &["r1"], true);
            node.min_similarity = similarity;
            let report = cp_report(true, &[("g1", true)], vec![node]);
            let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report);
            assert_eq!(plan.qualified_task_ids, vec!["g1".to_string()]);
            assert!(plan.needs_cloud_review.is_empty(), "相似度 {similarity} 不应进入低相似句复核");
        }
        let report = cp_report(true, &[("g1", false)], vec![]);
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report);
        assert!(plan.qualified_task_ids.is_empty());
        assert!(plan.needs_cloud_review.iter().any(|entry| {
            entry.get("taskId").and_then(Value::as_str) == Some("g1")
                && entry.get("reason").and_then(Value::as_str) == Some("content_not_aligned")
        }), "题组整体未通过仍应拒绝并进入复核");
    }

    #[test]
    fn cloud_primary_adopts_whole_candidate_when_aligned() {
        let groups = json!([{
            "taskId": "g1", "taskType": "true_false_not_given",
            "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
            "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1"]}]
        }]);
        let candidate = cp_candidate(groups, json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let report = cp_report(
            true,
            &[("g1", true)],
            vec![
                cp_node("i1", "instruction", Some("g1"), &["r1"], true),
                cp_node("p1", "prompt", Some("g1"), &["r2"], true),
            ],
        );
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report);
        assert!(plan.document_reasons.is_empty(), "{:?}", plan.document_reasons);
        assert!(plan.adopt_passage, "对齐全过应整体采用云端原文");
        assert_eq!(plan.qualified_task_ids, vec!["g1".to_string()]);
        assert!(plan.unqualified.is_empty());
    }

    #[test]
    fn cloud_primary_keeps_local_passage_but_still_adopts_aligned_groups() {
        let groups = json!([{
            "taskId": "g1", "taskType": "true_false_not_given",
            "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
            "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1"]}]
        }]);
        let candidate = cp_candidate(groups, json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let report = cp_report(
            false,
            &[("g1", true)],
            vec![
                cp_node("i1", "instruction", Some("g1"), &["r1"], true),
                cp_node("p1", "prompt", Some("g1"), &["r2"], true),
            ],
        );
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report);
        assert!(!plan.adopt_passage, "原文不达标应保留本地原文");
        assert!(!plan.passage_reasons.is_empty(), "应给出原文不采纳原因");
        assert_eq!(plan.qualified_task_ids, vec!["g1".to_string()], "题目仍应独立采纳");
    }

    #[test]
    fn cloud_primary_flags_unresolved_answer_in_adopted_group() {
        // 题组内容对齐、被整体采纳，但计分槽位答案仍未解析 → 单列进云端校核（大差异）。
        let groups = json!([{
            "taskId": "g1", "taskType": "true_false_not_given",
            "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
            "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1"]}]
        }]);
        let candidate = cp_candidate(
            groups,
            json!({"q1": {"slotId": "q1", "questionNumber": 1, "participation": "scoring"}}),
        );
        let report = cp_report(
            true,
            &[("g1", true)],
            vec![
                cp_node("i1", "instruction", Some("g1"), &["r1"], true),
                cp_node("p1", "prompt", Some("g1"), &["r2"], true),
            ],
        );
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report);
        assert_eq!(plan.qualified_task_ids, vec!["g1".to_string()], "题组仍应被采纳");
        assert!(
            plan.needs_cloud_review.iter().any(|entry| entry
                .get("slotId")
                .and_then(Value::as_str)
                == Some("q1")
                && entry.get("reason").and_then(Value::as_str) == Some("answer_unresolved")),
            "采纳后仍未解析的计分答案必须进云端校核：{:?}",
            plan.needs_cloud_review
        );
    }

    #[test]
    fn cloud_primary_rejects_group_on_strong_task_type_counterevidence() {
        // 说明是 TFNG 图例，云端却声明单选 → 强反证，不采纳该组题型。
        let groups = json!([{
            "taskId": "g1", "taskType": "single_choice",
            "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
            "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1"]}]
        }]);
        let candidate = cp_candidate(groups, json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let report = cp_report(
            true,
            &[("g1", true)],
            vec![
                cp_node("i1", "instruction", Some("g1"), &["r1"], true),
                cp_node("p1", "prompt", Some("g1"), &["r2"], true),
            ],
        );
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report);
        assert!(!plan.qualified_task_ids.contains(&"g1".to_string()));
        assert!(plan
            .unqualified
            .iter()
            .any(|(id, reasons)| id == "g1" && reasons.iter().any(|r| r.contains("题型强反证"))));
        assert!(plan
            .needs_cloud_review
            .iter()
            .any(|item| item.get("reason").and_then(Value::as_str) == Some("task_type_counterevidence")));
    }


    #[test]
    fn cloud_primary_rejects_whole_document_when_question_numbers_uncovered() {
        let groups = json!([{
            "taskId": "g1", "taskType": "true_false_not_given",
            "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
            "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1"]}]
        }]);
        // 云端只覆盖题号 1，本地声明 1、2 → 文档级题号覆盖失败。
        let candidate = cp_candidate(groups, json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let report = cp_report(true, &[("g1", true)], vec![cp_node("i1", "instruction", Some("g1"), &["r1"], true)]);
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1, 2]), &report);
        assert!(
            plan.document_reasons.iter().any(|r| r.contains("未覆盖本地题号")),
            "题号覆盖不全应落文档级：{:?}",
            plan.document_reasons
        );
    }

    #[test]
    fn cloud_primary_flags_instruction_swallowing_question() {
        // 说明与题干命中同一源区域 → 疑似说明吞题 / 题目被吞进说明。
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
        let plan = plan_cloud_primary_adoption(&candidate, &cp_local(&[1]), &report);
        assert!(plan
            .unqualified
            .iter()
            .any(|(id, reasons)| id == "g1" && reasons.iter().any(|r| r.contains("同一区域"))));
        assert!(plan
            .needs_cloud_review
            .iter()
            .any(|item| item.get("reason").and_then(Value::as_str) == Some("instruction_stem_overlap")));
    }

    #[test]
    fn scanned_pdf_falls_back_to_conservative_plan() {
        let groups = json!([{
            "taskId": "g1", "taskType": "true_false_not_given",
            "instructions": [{"id": "i1", "type": "text", "text": TFNG_INSTRUCTION}],
            "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1"]}]
        }]);
        let candidate = cp_candidate(groups, json!({"q1": {"slotId": "q1", "questionNumber": 1}}));
        let plan = plan_adoption(&candidate, &cp_local(&[1]), &AlignmentOutcome::NoTextLayer);
        assert!(matches!(plan, AdoptionPlan::Conservative(_)), "扫描卷无文本层应退回保守按题组");
    }

    #[test]
    fn build_cloud_primary_document_swaps_local_for_rejected_parts() {
        let current = json!({
            "passage": {"content": [{"id": "p", "type": "text", "text": "local passage"}]},
            "taskGroups": [{"taskId": "g1", "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1"]}]}],
            "answerSlots": {"q1": {"slotId": "q1", "questionNumber": 1}},
            "answerKey": {"q1": {"kind": "text", "values": ["local"]}}
        });
        let cloud = json!({
            "passage": {"content": [{"id": "p", "type": "text", "text": "cloud passage"}]},
            "taskGroups": [{"taskId": "g1", "responseGroups": [{"responseGroupId": "rg1", "slotIds": ["q1"]}]}],
            "answerSlots": {"q1": {"slotId": "q1", "questionNumber": 1}},
            "answerKey": {"q1": {"kind": "text", "values": ["cloud"]}}
        });
        let reject = CloudPrimaryPlan {
            adopt_passage: false,
            passage_reasons: vec!["x".into()],
            qualified_task_ids: vec![],
            unqualified: vec![("g1".to_string(), vec!["y".into()])],
            document_reasons: vec![],
            review_records: vec![],
            needs_cloud_review: vec![],
        };
        let merged = build_cloud_primary_document(&current, &cloud, &reject);
        assert_eq!(merged.pointer("/passage/content/0/text"), Some(&json!("local passage")));
        assert_eq!(merged.pointer("/answerKey/q1/values/0"), Some(&json!("local")));

        let adopt = CloudPrimaryPlan { adopt_passage: true, unqualified: vec![], ..reject_clone() };
        let merged = build_cloud_primary_document(&current, &cloud, &adopt);
        assert_eq!(merged.pointer("/passage/content/0/text"), Some(&json!("cloud passage")));
        assert_eq!(merged.pointer("/answerKey/q1/values/0"), Some(&json!("cloud")));
    }

    fn reject_clone() -> CloudPrimaryPlan {
        CloudPrimaryPlan {
            adopt_passage: false,
            passage_reasons: vec![],
            qualified_task_ids: vec![],
            unqualified: vec![],
            document_reasons: vec![],
            review_records: vec![],
            needs_cloud_review: vec![],
        }
    }

    #[test]
    fn stamp_alignment_provenance_marks_adopted_cloud_nodes() {
        let mut merged = json!({
            "passage": {"content": []},
            "taskGroups": [{"taskId": "g1", "instructions": [{"id": "i1", "type": "text", "text": "x"}]}]
        });
        let report = cp_report(true, &[("g1", true)], vec![cp_node("i1", "instruction", Some("g1"), &["r1"], true)]);
        let qualified = BTreeSet::from(["g1".to_string()]);
        stamp_alignment_provenance(&mut merged, &report, true, &qualified);
        assert_eq!(
            merged.pointer("/taskGroups/0/instructions/0/provenanceStatus"),
            Some(&json!("source"))
        );
        assert!(merged
            .pointer("/taskGroups/0/instructions/0/sourceAnchors")
            .and_then(Value::as_array)
            .is_some_and(|anchors| !anchors.is_empty()));
        assert!(merged
            .pointer("/taskGroups/0/instructionSignature/evidenceAnchors")
            .and_then(Value::as_array)
            .is_some_and(|anchors| !anchors.is_empty()));
    }
}
