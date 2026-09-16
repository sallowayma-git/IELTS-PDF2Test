//! 后台统一裁决：把三路结果合并成**一份**统一建议。
//!
//! 硬约束（对应任务书第三步）：
//! - 同一 `(target, field)` 只能有一条建议：去重、合并证据，不出现相互冲突的卡。
//! - 允许「证据不足，无法判断」：`Unverifiable` 不得携带建议 patch。
//! - 相互依赖的修正必须整组一致：任何一项不满足自动应用条件，整组转人工。
//! - 自动应用**不依赖模型置信度**，只依赖确定性条件（未被用户修改 + 原文证据 +
//!   低风险字段白名单 + 合并后结构仍然合法）。
//! - 模型调用有明确上限；不足时给出 `Unverifiable` 而不是猜。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::rules::{
    answer_compare_key, answer_is_empty, canonical_answer, compare, CompareInput,
};
use super::source::SourceVerificationV1;
use crate::schema::recognition_v1::{
    reason, ChainStatusSummaryV1, DecisionFieldV1, DecisionItemV1, DecisionResolutionV1,
    DecisionSeverityV1, DecisionStatusV1, DecisionSummaryV1, DecisionTargetTypeV1,
    RecognitionCandidateV1, RecognitionDecisionV1, RECOGNITION_DECISION_V1_SCHEMA_VERSION,
};

pub(crate) struct AdjudicateInput<'a> {
    pub canonical: &'a Value,
    pub local: &'a RecognitionCandidateV1,
    pub cloud: &'a RecognitionCandidateV1,
    pub source: &'a SourceVerificationV1,
    pub batch_id: &'a str,
    pub item_id: &'a str,
    pub job_id: &'a str,
    pub base_edit_version: i64,
    /// 结构校验闭包：把一批 patch 应用到权威稿副本并跑同一套校验；
    /// 返回 `Err` 表示「单独合法、合并后结构损坏」。
    pub validate_batch: &'a dyn Fn(&[Value]) -> Result<(), String>,
}

pub(crate) struct AdjudicationOutcome {
    pub decision: RecognitionDecisionV1,
    /// 已通过全部自动应用条件、且合并后结构合法的 patch（按 decision id 对齐）。
    pub auto_apply_candidates: Vec<String>,
}

fn resolution_rank(resolution: DecisionResolutionV1) -> u8 {
    match resolution {
        DecisionResolutionV1::Agreed => 0,
        DecisionResolutionV1::AutoFixed => 1,
        DecisionResolutionV1::Unverifiable => 2,
        DecisionResolutionV1::NeedsReview => 3,
    }
}

fn severity_rank(severity: DecisionSeverityV1) -> u8 {
    match severity {
        DecisionSeverityV1::Info => 0,
        DecisionSeverityV1::Warning => 1,
        DecisionSeverityV1::Blocker => 2,
    }
}

/// 去重合并：同一 decision_id 只保留一条，取更高的严重度与更强的结论，
/// 证据累积，建议 patch 取第一条非空。
fn merge_duplicates(items: Vec<DecisionItemV1>) -> Vec<DecisionItemV1> {
    let mut merged: BTreeMap<String, DecisionItemV1> = BTreeMap::new();
    for item in items {
        match merged.get_mut(&item.decision_id) {
            None => {
                merged.insert(item.decision_id.clone(), item);
            }
            Some(existing) => {
                if severity_rank(item.severity) > severity_rank(existing.severity) {
                    existing.severity = item.severity;
                }
                if resolution_rank(item.resolution) > resolution_rank(existing.resolution) {
                    existing.resolution = item.resolution;
                    existing.code = item.code.clone();
                    existing.title = item.title.clone();
                    existing.user_message = item.user_message.clone();
                    existing.reason_code = item.reason_code.clone();
                }
                if existing.proposed_patch.is_none() {
                    existing.proposed_patch = item.proposed_patch.clone();
                }
                if existing.undo.is_none() {
                    existing.undo = item.undo.clone();
                }
                if existing.local_value.is_none() {
                    existing.local_value = item.local_value.clone();
                }
                if existing.cloud_value.is_none() {
                    existing.cloud_value = item.cloud_value.clone();
                }
                if existing.source_value.is_none() {
                    existing.source_value = item.source_value.clone();
                }
                for evidence in item.evidence {
                    if !existing.evidence.iter().any(|known| {
                        known.chain == evidence.chain && known.anchor_kind == evidence.anchor_kind
                    }) {
                        existing.evidence.push(evidence);
                    }
                }
                if existing.dependency_group.is_none() {
                    existing.dependency_group = item.dependency_group.clone();
                }
            }
        }
    }
    merged.into_values().collect()
}

/// 相互依赖的修正：同一 response group 内当分派是 unordered_set / ordered_slots 时，
/// 改一个槽位的答案会改变整组语义，必须整组接受。
fn assign_dependency_groups(items: &mut [DecisionItemV1], local: &RecognitionCandidateV1) {
    let mut group_of: BTreeMap<String, String> = BTreeMap::new();
    for group in &local.task_groups {
        for response in &group.response_groups {
            let assignment = response.assignment.as_deref().unwrap_or("per_slot");
            if assignment == "per_slot" {
                continue;
            }
            let dependency = format!("dep:response:{}", response.response_group_id);
            for slot_id in &response.slot_ids {
                group_of.insert(slot_id.clone(), dependency.clone());
            }
        }
    }
    for item in items.iter_mut() {
        if item.field != DecisionFieldV1::Answer {
            continue;
        }
        if let Some(dependency) = group_of.get(&item.target.target_id) {
            item.dependency_group = Some(dependency.clone());
        }
    }
}

fn patch_answer_value(patch: &Value) -> Option<&Value> {
    patch.get("value")
}

/// 自动应用资格（确定性，不使用模型置信度）。
fn auto_apply_eligible(
    item: &DecisionItemV1,
    input: &AdjudicateInput<'_>,
) -> bool {
    if item.resolution != DecisionResolutionV1::NeedsReview {
        return false;
    }
    if item.reason_code == reason::USER_EDITED {
        return false;
    }
    // 低风险白名单：只允许「补空答案」。题干/选项/题型/结构变化一律转人工。
    if item.field != DecisionFieldV1::Answer {
        return false;
    }
    let Some(patch) = item.proposed_patch.as_ref() else { return false };
    let Some(proposed) = patch_answer_value(patch) else { return false };

    let canonical_value = canonical_answer(input.canonical, &item.target.target_id);
    // 1) 只补空，绝不覆盖已有答案。
    if !answer_is_empty(canonical_value) {
        return false;
    }
    // 2) 权威稿必须与本地识别结果一致（用户没有改过这一槽位）。
    let Some(local_slot) = input.local.slot(&item.target.target_id) else { return false };
    let local_key = answer_compare_key(local_slot.answer.as_ref());
    if answer_compare_key(canonical_value) != local_key {
        return false;
    }
    // 3) 建议值必须与原文件断言的值逐字节相同（可靠证据，而不是「大概率对」）。
    let inferred = input.source.suggested(
        DecisionTargetTypeV1::Slot,
        &item.target.target_id,
        DecisionFieldV1::Answer,
    );
    match inferred {
        Some(inferred) => answer_compare_key(Some(inferred)) == answer_compare_key(Some(proposed)),
        None => false,
    }
}

/// 结构校验：把整批自动修正一次性应用到副本上；不合法则整组降级。
fn validate_auto_batch(
    items: &[DecisionItemV1],
    candidates: &[String],
    input: &AdjudicateInput<'_>,
) -> bool {
    let patches: Vec<Value> = items
        .iter()
        .filter(|item| candidates.contains(&item.decision_id))
        .filter_map(|item| item.proposed_patch.clone())
        .collect();
    if patches.is_empty() {
        return true;
    }
    (input.validate_batch)(&patches).is_ok()
}

pub(crate) fn adjudicate(input: AdjudicateInput<'_>) -> AdjudicationOutcome {
    let raw = compare(&CompareInput {
        canonical: input.canonical,
        local: input.local,
        cloud: input.cloud,
        source: input.source,
    });
    let mut items = merge_duplicates(raw);
    assign_dependency_groups(&mut items, input.local);

    // ── 自动应用资格 + 依赖组一致性 ─────────────────────────────────
    let mut eligible: BTreeSet<String> = items
        .iter()
        .filter(|item| auto_apply_eligible(item, &input))
        .map(|item| item.decision_id.clone())
        .collect();

    // 依赖组：只要组内有任一项不可自动应用，整组转人工（避免结构损坏）。
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in &items {
        if let Some(group) = &item.dependency_group {
            groups.entry(group.clone()).or_default().push(item.decision_id.clone());
        }
    }
    for (_, members) in groups.iter() {
        if members.iter().any(|id| !eligible.contains(id)) {
            for id in members {
                eligible.remove(id);
            }
        }
    }

    // 合并后结构校验：单独合法、合并后损坏 → 全部降级。
    if !validate_auto_batch(&items, &eligible.clone().into_iter().collect::<Vec<_>>(), &input) {
        for item in items.iter_mut() {
            if eligible.contains(&item.decision_id) {
                item.reason_code = reason::DEPENDENCY_BLOCKED.to_string();
                item.title = format!("{}（需整组确认）", item.title);
                item.user_message =
                    format!("{}相关修正在合并后会让题目结构不完整，请整组确认。", item.user_message);
            }
        }
        eligible.clear();
    }

    // ── 汇总输出 ────────────────────────────────────────────────────
    let mut summary = DecisionSummaryV1::default();
    let mut visible: Vec<DecisionItemV1> = Vec::new();
    for mut item in items {
        match item.resolution {
            DecisionResolutionV1::Agreed => {
                summary.agreed += 1;
                continue; // 一致内容后台留记录，前端不产生逐项问题
            }
            DecisionResolutionV1::AutoFixed => {
                summary.auto_fixed += 1;
            }
            DecisionResolutionV1::NeedsReview => {
                summary.needs_review += 1;
            }
            DecisionResolutionV1::Unverifiable => {
                summary.unverifiable += 1;
            }
        }
        // 契约：无法判断的项不得携带建议 patch。
        if item.resolution == DecisionResolutionV1::Unverifiable {
            item.proposed_patch = None;
            item.undo = None;
            if item.severity == DecisionSeverityV1::Blocker {
                item.severity = DecisionSeverityV1::Warning;
            }
        }
        visible.push(item);
    }

    let decision = RecognitionDecisionV1 {
        schema_version: RECOGNITION_DECISION_V1_SCHEMA_VERSION.to_string(),
        batch_id: input.batch_id.to_string(),
        item_id: input.item_id.to_string(),
        job_id: input.job_id.to_string(),
        base_edit_version: input.base_edit_version,
        generated_at: chrono::Utc::now().to_rfc3339(),
        chain_status: ChainStatusSummaryV1 {
            local: input.local.status,
            cloud: input.cloud.status,
            source: input.source.status,
            cloud_reason_code: input.cloud.reason_code.clone(),
            source_reason_code: input.source.reason_code.clone(),
        },
        items: visible,
        summary,
    };
    AdjudicationOutcome {
        decision,
        auto_apply_candidates: eligible.into_iter().collect(),
    }
}

/// 自动修正成功写入后调用：把对应项翻成 `AutoFixed` 并记录时间与撤销信息。
pub(crate) fn mark_auto_applied(
    decision: &mut RecognitionDecisionV1,
    applied: &[String],
    undo_by_id: &BTreeMap<String, Value>,
    at: &str,
) {
    for item in decision.items.iter_mut() {
        if !applied.contains(&item.decision_id) {
            continue;
        }
        item.resolution = DecisionResolutionV1::AutoFixed;
        item.auto_applied = true;
        item.applied_at = Some(at.to_string());
        item.status = DecisionStatusV1::Accepted;
        if let Some(undo) = undo_by_id.get(&item.decision_id) {
            item.undo = Some(undo.clone());
        }
    }
    recompute_summary(decision);
}

/// 自动修正写入失败（版本冲突 / 事务失败）：如实降级为待确认，不谎称已修。
pub(crate) fn mark_auto_apply_failed(
    decision: &mut RecognitionDecisionV1,
    failed: &[(String, String)],
) {
    for item in decision.items.iter_mut() {
        let Some((_, code)) = failed.iter().find(|(id, _)| id == &item.decision_id) else {
            continue;
        };
        item.resolution = DecisionResolutionV1::NeedsReview;
        item.auto_applied = false;
        item.applied_at = None;
        item.status = DecisionStatusV1::Failed;
        item.reason_code = code.clone();
        item.user_message = format!("{}（自动应用未成功，请手动确认）", item.user_message);
    }
    recompute_summary(decision);
}

/// 本地基线不可信（冻结快照缺失/批次不匹配）时调用：撤销自动应用资格，全部转人工。
///
/// 这些项本来符合 [`auto_apply_eligible`]，但该函数的第 2 条守卫——「权威稿当前值 ==
/// 本地识别结果」——只有在本地基线**确实是冻结快照**时才成立。快照缺失时
/// [`crate::reconcile::engine::resolve_local_snapshot`] 会退回「按当前权威稿现场重投影」，
/// 于是两边恒等，守卫退化为空操作；此时自动写入等于在无保护的前提下改题稿。
///
/// 因此这里把它们留在 `NeedsReview`（仍可人工确认，**不静默丢弃**），只把原因码与文案
/// 换成可解释的 [`reason::BASELINE_NOT_FROZEN`]，并重算汇总。
pub(crate) fn refuse_auto_apply_without_frozen_baseline(
    decision: &mut RecognitionDecisionV1,
    candidates: &[String],
) {
    for item in decision.items.iter_mut() {
        if !candidates.contains(&item.decision_id) {
            continue;
        }
        item.auto_applied = false;
        item.applied_at = None;
        item.reason_code = reason::BASELINE_NOT_FROZEN.to_string();
        item.user_message = format!(
            "{}（本地基线快照不可用，已改为人工确认，不会自动写入）",
            item.user_message
        );
    }
    recompute_summary(decision);
}

/// 由**最终**项状态重算汇总。任何改动 `item.resolution` 的步骤之后都必须调用它。
///
/// 为什么不能增量改计数：自动应用会把项从 `NeedsReview` 翻成 `AutoFixed`。若只把
/// `auto_fixed` 加一而不把 `needs_review` 减一，同一项就被两个计数同时统计——前端会
/// 凭空显示「还有若干项待确认」，而后台 `actionableCount` 也跟着一起虚高。重算而不是
/// 增量，是让这两个数字与 `items` 的真实状态永远自洽的唯一办法。
///
/// `agreed` 由调用方保留原值：一致项按契约**不入 `items`**，无法从项推导。
pub(crate) fn recompute_summary(decision: &mut RecognitionDecisionV1) {
    let agreed = decision.summary.agreed;
    let count = |resolution: DecisionResolutionV1| -> u32 {
        decision
            .items
            .iter()
            .filter(|item| item.resolution == resolution)
            .count() as u32
    };
    decision.summary = DecisionSummaryV1 {
        agreed,
        auto_fixed: count(DecisionResolutionV1::AutoFixed),
        needs_review: count(DecisionResolutionV1::NeedsReview),
        unverifiable: count(DecisionResolutionV1::Unverifiable),
    };
}

/// 是否还需要（且允许）一次模型裁决：只对「实质分歧但缺证据」的项。
/// 调用方必须同时检查 `MAX_ADJUDICATION_MODEL_CALLS` 预算。
pub(crate) fn needs_model_adjudication(decision: &RecognitionDecisionV1) -> bool {
    decision.items.iter().any(|item| {
        item.resolution == DecisionResolutionV1::NeedsReview
            && item.proposed_patch.is_none()
            && item.reason_code != reason::DEPENDENCY_BLOCKED
    })
}

/// 撤销 patch：把建议 patch 反转为「恢复旧值」。仅支持 `setAnswer`（当前唯一自动字段）。
pub(crate) fn undo_patch_for(item: &DecisionItemV1) -> Option<Value> {
    if item.field != DecisionFieldV1::Answer {
        return None;
    }
    let patch = item.proposed_patch.as_ref()?;
    if patch.get("op").and_then(Value::as_str) != Some("setAnswer") {
        return None;
    }
    let slot_id = patch.get("slotId").and_then(Value::as_str)?;
    let previous = item
        .local_value
        .clone()
        .unwrap_or_else(|| serde_json::json!({"kind": "unresolved"}));
    Some(super::rules::answer_patch(slot_id, &previous))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::recognition_v1::{
        CandidateResponseGroupV1, CandidateSlotV1, CandidateTaskGroupV1, ChainKindV1, ChainStatusV1,
        DecisionEvidenceV1, DecisionTargetV1, RecognitionCandidateV1,
        RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION,
    };
    use crate::reconcile::source::verify_against_source;
    use serde_json::json;

    fn slot(slot_id: &str, number: u32, answer: Option<Value>, anchors: bool) -> CandidateSlotV1 {
        CandidateSlotV1 {
            slot_id: slot_id.to_string(),
            question_number: number,
            display_label: number.to_string(),
            task_id: "task-1".to_string(),
            response_group_id: "rg-1".to_string(),
            interaction: "text".to_string(),
            host_type: "prompt".to_string(),
            host_node_id: None,
            participation: "scoring".to_string(),
            answer,
            has_source_evidence: anchors,
        }
    }

    fn candidate(chain: ChainKindV1, slots: Vec<CandidateSlotV1>, status: ChainStatusV1) -> RecognitionCandidateV1 {
        RecognitionCandidateV1 {
            schema_version: RECOGNITION_CANDIDATE_V1_SCHEMA_VERSION.to_string(),
            chain,
            batch_id: "batch-1".to_string(),
            item_id: "item-1".to_string(),
            job_id: "job-1".to_string(),
            source_file_id: "file-1".to_string(),
            source_sha256: "a".repeat(64),
            base_edit_version: 1,
            generated_at: "2026-09-15T00:00:00Z".to_string(),
            status,
            reason_code: None,
            task_groups: vec![CandidateTaskGroupV1 {
                task_id: "task-1".to_string(),
                display_range: json!({"kind":"range","start":14,"end":15}),
                task_type: "sentence_completion".to_string(),
                instructions_text: "Complete the sentences.".to_string(),
                stimulus_text: None,
                option_bank: None,
                response_groups: vec![CandidateResponseGroupV1 {
                    response_group_id: "rg-1".to_string(),
                    kind: "text_entry".to_string(),
                    prompt: None,
                    slot_ids: vec!["slot-14".to_string(), "slot-15".to_string()],
                    options: None,
                    cardinality: None,
                    assignment: Some("per_slot".to_string()),
                    allow_option_reuse: false,
                }],
                source_anchors: vec![],
                confidence: 0.9,
                warnings: vec![],
            }],
            slots,
            unresolved_regions: vec![],
            assets: vec![],
            salvage: None,
            warnings: vec![],
        }
    }

    fn canonical_with(answer_14: Option<Value>, answer_15: Option<Value>) -> Value {
        let mut key = serde_json::Map::new();
        if let Some(value) = answer_14 {
            key.insert("slot-14".to_string(), value);
        }
        if let Some(value) = answer_15 {
            key.insert("slot-15".to_string(), value);
        }
        json!({
            "taskGroups": [],
            "answerSlots": {
                "slot-14": {"slotId":"slot-14","questionNumber":14,"interaction":"text","sourceAnchors":[]},
                "slot-15": {"slotId":"slot-15","questionNumber":15,"interaction":"text","sourceAnchors":[]}
            },
            "answerKey": Value::Object(key),
            "quality": {"coverageStatus": {"unassignedSourceNodeIds": []}}
        })
    }

    fn document(pages: &[&str]) -> Value {
        json!({
            "pages": pages.iter().enumerate().map(|(index, text)| json!({
                "pageIndex": index,
                "lines": text.lines().map(|line| json!({"text": line})).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        })
    }

    fn no_validation() -> impl Fn(&[Value]) -> Result<(), String> {
        |_patches: &[Value]| Ok(())
    }

    /// 验收项 2：云端完整识别与本地稿一致 → 不产生多余的待确认项。
    #[test]
    fn agreement_with_source_evidence_produces_no_review_items() {
        let local = candidate(
            ChainKindV1::Local,
            vec![
                slot("slot-14", 14, Some(json!({"kind":"text","values":["stencilling"]})), true),
                slot("slot-15", 15, Some(json!({"kind":"text","values":["books"]})), true),
            ],
            ChainStatusV1::Succeeded,
        );
        let cloud = candidate(
            ChainKindV1::Cloud,
            vec![
                slot("slot-14", 14, Some(json!({"kind":"text","values":["Stencilling"]})), true),
                slot("slot-15", 15, Some(json!({"kind":"text","values":["books"]})), true),
            ],
            ChainStatusV1::Succeeded,
        );
        let source = verify_against_source(
            Some(&document(&["14 stencilling\n15 books"])),
            &[
                ("slot-14".to_string(), 14, Some(json!({"kind":"text","values":["stencilling"]})), true, String::new()),
                ("slot-15".to_string(), 15, Some(json!({"kind":"text","values":["books"]})), true, String::new()),
            ],
            &[("task-1".to_string(), vec![14, 15])],
        );
        let canonical = canonical_with(
            Some(json!({"kind":"text","values":["stencilling"]})),
            Some(json!({"kind":"text","values":["books"]})),
        );
        let validate = no_validation();
        let outcome = adjudicate(AdjudicateInput {
            canonical: &canonical,
            local: &local,
            cloud: &cloud,
            source: &source,
            batch_id: "batch-1",
            item_id: "item-1",
            job_id: "job-1",
            base_edit_version: 1,
            validate_batch: &validate,
        });
        assert_eq!(outcome.decision.summary.agreed, 2);
        assert_eq!(outcome.decision.summary.needs_review, 0);
        assert_eq!(outcome.decision.summary.unverifiable, 0);
        assert!(outcome.decision.items.is_empty(), "一致内容不得出现在问题列表");
        assert!(outcome.auto_apply_candidates.is_empty());
    }

    /// 验收项 4：各路一致但缺原文证据 → 不能被当成已验证。
    #[test]
    fn agreement_without_source_evidence_is_unverifiable_not_agreed() {
        let local = candidate(
            ChainKindV1::Local,
            vec![slot("slot-14", 14, Some(json!({"kind":"text","values":["stencilling"]})), false)],
            ChainStatusV1::Succeeded,
        );
        let cloud = candidate(
            ChainKindV1::Cloud,
            vec![slot("slot-14", 14, Some(json!({"kind":"text","values":["stencilling"]})), false)],
            ChainStatusV1::Succeeded,
        );
        let source = verify_against_source(
            None,
            &[("slot-14".to_string(), 14, Some(json!({"kind":"text","values":["stencilling"]})), false, String::new())],
            &[],
        );
        let canonical = canonical_with(Some(json!({"kind":"text","values":["stencilling"]})), None);
        let validate = no_validation();
        let outcome = adjudicate(AdjudicateInput {
            canonical: &canonical,
            local: &local,
            cloud: &cloud,
            source: &source,
            batch_id: "batch-1",
            item_id: "item-1",
            job_id: "job-1",
            base_edit_version: 1,
            validate_batch: &validate,
        });
        assert_eq!(outcome.decision.summary.agreed, 0, "缺少原文证据不得判为已确认");
        assert_eq!(outcome.decision.summary.unverifiable, 1);
        let item = &outcome.decision.items[0];
        assert_eq!(item.reason_code, reason::NO_SOURCE_EVIDENCE);
        assert!(item.proposed_patch.is_none(), "无法判断时不得给出建议 patch");
    }

    /// 验收项 3：本地与云端有分歧 → 只形成一份统一建议。
    #[test]
    fn divergence_merges_into_a_single_review_item() {
        let local = candidate(
            ChainKindV1::Local,
            vec![slot("slot-14", 14, Some(json!({"kind":"text","values":["stencilling"]})), true)],
            ChainStatusV1::Succeeded,
        );
        let cloud = candidate(
            ChainKindV1::Cloud,
            vec![slot("slot-14", 14, Some(json!({"kind":"text","values":["painting"]})), true)],
            ChainStatusV1::Succeeded,
        );
        // 原文既不同意本地也不同 cloud → 无法判断。
        let source = verify_against_source(
            Some(&document(&["14 carving"])),
            &[("slot-14".to_string(), 14, Some(json!({"kind":"text","values":["stencilling"]})), true, String::new())],
            &[],
        );
        let canonical = canonical_with(Some(json!({"kind":"text","values":["stencilling"]})), None);
        let validate = no_validation();
        let outcome = adjudicate(AdjudicateInput {
            canonical: &canonical,
            local: &local,
            cloud: &cloud,
            source: &source,
            batch_id: "batch-1",
            item_id: "item-1",
            job_id: "job-1",
            base_edit_version: 1,
            validate_batch: &validate,
        });
        let answer_items: Vec<_> = outcome
            .decision
            .items
            .iter()
            .filter(|item| item.field == DecisionFieldV1::Answer)
            .collect();
        assert_eq!(answer_items.len(), 1, "同一问题必须只有一张建议卡");
        assert_eq!(answer_items[0].decision_id, "d:slot:slot-14:answer");
    }

    /// 验收项 6：自动修正符合规则 → 自动应用；不符合 → 转人工。
    #[test]
    fn empty_answer_is_auto_filled_only_when_the_source_asserts_the_value() {
        let local = candidate(
            ChainKindV1::Local,
            vec![slot("slot-14", 14, Some(json!({"kind":"unresolved"})), true)],
            ChainStatusV1::Succeeded,
        );
        let cloud = candidate(
            ChainKindV1::Cloud,
            vec![slot("slot-14", 14, Some(json!({"kind":"text","values":["stencilling"]})), true)],
            ChainStatusV1::Succeeded,
        );
        let source = verify_against_source(
            Some(&document(&["14 stencilling"])),
            &[("slot-14".to_string(), 14, Some(json!({"kind":"unresolved"})), true, String::new())],
            &[],
        );
        let canonical = canonical_with(Some(json!({"kind":"unresolved"})), None);
        let validate = no_validation();
        let outcome = adjudicate(AdjudicateInput {
            canonical: &canonical,
            local: &local,
            cloud: &cloud,
            source: &source,
            batch_id: "batch-1",
            item_id: "item-1",
            job_id: "job-1",
            base_edit_version: 1,
            validate_batch: &validate,
        });
        assert_eq!(outcome.auto_apply_candidates.len(), 1);
        assert_eq!(outcome.auto_apply_candidates[0], "d:slot:slot-14:answer");
        let item = &outcome.decision.items[0];
        assert_eq!(item.resolution, DecisionResolutionV1::NeedsReview, "写入前不得谎称已修正");
        assert!(item.proposed_patch.is_some());
    }

    /// 自动修正不得覆盖用户已有的答案（即使原文有不同值）。
    #[test]
    fn non_empty_answer_is_never_auto_overwritten() {
        let local = candidate(
            ChainKindV1::Local,
            vec![slot("slot-14", 14, Some(json!({"kind":"text","values":["stencilling"]})), true)],
            ChainStatusV1::Succeeded,
        );
        let cloud = candidate(
            ChainKindV1::Cloud,
            vec![slot("slot-14", 14, Some(json!({"kind":"text","values":["stencilling"]})), true)],
            ChainStatusV1::Succeeded,
        );
        let source = verify_against_source(
            Some(&document(&["14 painting"])),
            &[("slot-14".to_string(), 14, Some(json!({"kind":"text","values":["stencilling"]})), true, String::new())],
            &[],
        );
        let canonical = canonical_with(Some(json!({"kind":"text","values":["stencilling"]})), None);
        let validate = no_validation();
        let outcome = adjudicate(AdjudicateInput {
            canonical: &canonical,
            local: &local,
            cloud: &cloud,
            source: &source,
            batch_id: "batch-1",
            item_id: "item-1",
            job_id: "job-1",
            base_edit_version: 1,
            validate_batch: &validate,
        });
        assert!(outcome.auto_apply_candidates.is_empty(), "已有答案不得被自动覆盖");
        assert_eq!(outcome.decision.summary.needs_review, 1);
    }

    /// 依赖组：合并后结构不合法则整组降级为待确认。
    #[test]
    fn batch_that_breaks_structure_downgrades_the_whole_group() {
        let local = candidate(
            ChainKindV1::Local,
            vec![slot("slot-14", 14, Some(json!({"kind":"unresolved"})), true)],
            ChainStatusV1::Succeeded,
        );
        let cloud = candidate(
            ChainKindV1::Cloud,
            vec![slot("slot-14", 14, Some(json!({"kind":"text","values":["stencilling"]})), true)],
            ChainStatusV1::Succeeded,
        );
        let source = verify_against_source(
            Some(&document(&["14 stencilling"])),
            &[("slot-14".to_string(), 14, Some(json!({"kind":"unresolved"})), true, String::new())],
            &[],
        );
        let canonical = canonical_with(Some(json!({"kind":"unresolved"})), None);
        let validate = |_patches: &[Value]| Err("AUTHORING_SCHEMA_INVALID".to_string());
        let outcome = adjudicate(AdjudicateInput {
            canonical: &canonical,
            local: &local,
            cloud: &cloud,
            source: &source,
            batch_id: "batch-1",
            item_id: "item-1",
            job_id: "job-1",
            base_edit_version: 1,
            validate_batch: &validate,
        });
        assert!(outcome.auto_apply_candidates.is_empty());
        assert_eq!(outcome.decision.items[0].reason_code, reason::DEPENDENCY_BLOCKED);
        assert_eq!(outcome.decision.summary.needs_review, 1);
    }

    #[test]
    fn undo_patch_restores_the_previous_value() {
        let item = DecisionItemV1 {
            decision_id: "d:slot:slot-14:answer".to_string(),
            resolution: DecisionResolutionV1::AutoFixed,
            code: "ANSWER_FILL_FROM_SOURCE".to_string(),
            severity: DecisionSeverityV1::Warning,
            title: "t".to_string(),
            user_message: "m".to_string(),
            target: DecisionTargetV1 {
                target_type: DecisionTargetTypeV1::Slot,
                target_id: "slot-14".to_string(),
                task_id: None,
                node_id: None,
                question_numbers: vec![14],
            },
            field: DecisionFieldV1::Answer,
            evidence: vec![DecisionEvidenceV1 {
                chain: ChainKindV1::Source,
                anchor_kind: "x".to_string(),
                page_index: None,
                quote: None,
                anchor: None,
            }],
            local_value: Some(json!({"kind":"unresolved"})),
            cloud_value: None,
            source_value: None,
            proposed_patch: Some(json!({"op":"setAnswer","slotId":"slot-14","value":{"kind":"text","values":["stencilling"]}})),
            undo: None,
            auto_applied: true,
            applied_at: None,
            status: DecisionStatusV1::Accepted,
            reason_code: reason::RULES_MATCH.to_string(),
            dependency_group: None,
        };
        let undo = undo_patch_for(&item).expect("undo must exist for an applied setAnswer");
        assert_eq!(undo["value"]["kind"], json!("unresolved"));
    }
}
