//! 确定性规则层：题目、段落、选项、答案位、资源与来源覆盖。
//!
//! 只做**规则优先**的判断；规则不足以结论时给出 `Unverifiable`（证据不足，
//! 无法判断），而不是猜。所有比较都在 V2 语义视图上进行（题号 / 题组 / 槽位 /
//! 选项 label），因此本地稿与云端候选天然可比。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use super::candidate::normalize_text;
use super::source::{SourceVerdictV1, SourceVerificationV1};
use crate::schema::recognition_v1::{
    reason, CandidateSlotV1, ChainKindV1, ChainStatusV1, DecisionEvidenceV1, DecisionFieldV1,
    DecisionItemV1, DecisionResolutionV1, DecisionSeverityV1, DecisionStatusV1, DecisionTargetTypeV1,
    DecisionTargetV1, RecognitionCandidateV1,
};

// ── 权威稿读取 helper（只读）────────────────────────────────────────────

pub(crate) fn canonical_slot<'a>(canonical: &'a Value, slot_id: &str) -> Option<&'a Value> {
    canonical.get("answerSlots").and_then(Value::as_object)?.get(slot_id)
}

pub(crate) fn canonical_answer<'a>(canonical: &'a Value, slot_id: &str) -> Option<&'a Value> {
    canonical.get("answerKey").and_then(Value::as_object)?.get(slot_id)
}

pub(crate) fn canonical_slot_has_anchors(canonical: &Value, slot_id: &str) -> bool {
    canonical_slot(canonical, slot_id)
        .and_then(|slot| slot.get("sourceAnchors"))
        .and_then(Value::as_array)
        .map(|anchors| !anchors.is_empty())
        .unwrap_or(false)
}

/// 递归查找带 `id` 的节点并读取 provenance。
pub(crate) fn node_provenance(canonical: &Value, node_id: &str) -> Option<String> {
    fn walk(value: &Value, node_id: &str) -> Option<String> {
        match value {
            Value::Object(map) => {
                if map.get("id").and_then(Value::as_str) == Some(node_id) {
                    return map
                        .get("provenanceStatus")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                map.values().find_map(|child| walk(child, node_id))
            }
            Value::Array(items) => items.iter().find_map(|child| walk(child, node_id)),
            _ => None,
        }
    }
    walk(canonical, node_id)
}

pub(crate) fn is_user_edited(canonical: &Value, node_id: &str) -> bool {
    node_provenance(canonical, node_id).as_deref() == Some("user_edited")
}

/// 「空答案」= 缺失 / `unresolved` / 文本值全空 / 选项标签为空。
pub(crate) fn answer_is_empty(answer: Option<&Value>) -> bool {
    let Some(answer) = answer else { return true };
    match answer.get("kind").and_then(Value::as_str) {
        Some("text") => answer
            .get("values")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .all(|value| value.trim().is_empty())
            })
            .unwrap_or(true),
        Some("option") => answer
            .get("labels")
            .and_then(Value::as_array)
            .map(|labels| labels.is_empty())
            .unwrap_or(true),
        Some("unresolved") | _ => true,
    }
}

/// 答案的规范化比较键。`unresolved` 与缺失等价（都返回空串）。
pub(crate) fn answer_compare_key(answer: Option<&Value>) -> String {
    let Some(answer) = answer else { return String::new() };
    match answer.get("kind").and_then(Value::as_str) {
        Some("text") => {
            let mut values: Vec<String> = answer
                .get("values")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(normalize_text)
                        .filter(|value| !value.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            values.sort();
            values.join("|")
        }
        Some("option") => {
            let mut labels: Vec<String> = answer
                .get("labels")
                .and_then(Value::as_array)
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|label| label.trim().to_uppercase())
                        .filter(|label| !label.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            labels.sort();
            format!(
                "opt:{}:{}",
                labels.join(","),
                answer.get("assignment").and_then(Value::as_str).unwrap_or("per_slot")
            )
        }
        _ => String::new(),
    }
}

fn evidence_for(chain: ChainKindV1, anchor_kind: &str, quote: Option<String>) -> DecisionEvidenceV1 {
    DecisionEvidenceV1 {
        chain,
        anchor_kind: anchor_kind.to_string(),
        page_index: None,
        quote,
        anchor: None,
    }
}

fn source_evidence(target_type: DecisionTargetTypeV1, target_id: &str, field: DecisionFieldV1) -> Vec<DecisionEvidenceV1> {
    vec![DecisionEvidenceV1 {
        chain: ChainKindV1::Source,
        anchor_kind: format!("source_finding:{target_type:?}:{target_id}:{}", field.as_str()),
        page_index: None,
        quote: None,
        anchor: None,
    }]
}

// ── 比较 ───────────────────────────────────────────────────────────────

pub(crate) struct CompareInput<'a> {
    /// 当前权威稿（`IeltsAuthoringIRV2` 值）。
    pub canonical: &'a Value,
    pub local: &'a RecognitionCandidateV1,
    pub cloud: &'a RecognitionCandidateV1,
    pub source: &'a SourceVerificationV1,
}

impl<'a> CompareInput<'a> {
    fn cloud_usable(&self) -> bool {
        matches!(self.cloud.status, ChainStatusV1::Succeeded | ChainStatusV1::Partial)
    }
}

/// 产出**未去重**的裁决项；去重、依赖组与自动应用资格在 `adjudicate` 中完成。
pub(crate) fn compare(input: &CompareInput<'_>) -> Vec<DecisionItemV1> {
    let mut items = Vec::new();
    compare_slot_coverage(input, &mut items);
    compare_slots(input, &mut items);
    compare_groups(input, &mut items);
    compare_assets(input, &mut items);
    compare_source_coverage(input, &mut items);
    items
}

/// R1：题目（答案位）覆盖。云端可用时才比较，否则会把「云端没跑」说成「云端缺题」。
fn compare_slot_coverage(input: &CompareInput<'_>, items: &mut Vec<DecisionItemV1>) {
    if !input.cloud_usable() {
        return;
    }
    let local_numbers: BTreeSet<u32> = input.local.slots.iter().map(|slot| slot.question_number).collect();
    let cloud_numbers: BTreeSet<u32> = input.cloud.slots.iter().map(|slot| slot.question_number).collect();
    for number in local_numbers.difference(&cloud_numbers) {
        let Some(slot) = input.local.slot_by_question(*number) else { continue };
        items.push(DecisionItemV1 {
            decision_id: DecisionItemV1::decision_id_for(
                DecisionTargetTypeV1::Slot,
                &slot.slot_id,
                DecisionFieldV1::SourceCoverage,
            ),
            resolution: DecisionResolutionV1::NeedsReview,
            code: "CLOUD_SLOT_MISSING".to_string(),
            severity: DecisionSeverityV1::Warning,
            title: format!("第 {number} 题只在本地识别中"),
            user_message: format!("第 {number} 题的云端识别没有给出对应题目，已保留本地结果。"),
            target: DecisionTargetV1 {
                target_type: DecisionTargetTypeV1::Slot,
                target_id: slot.slot_id.clone(),
                task_id: Some(slot.task_id.clone()),
                node_id: slot.host_node_id.clone(),
                question_numbers: vec![*number],
            },
            field: DecisionFieldV1::SourceCoverage,
            evidence: vec![evidence_for(ChainKindV1::Local, "slot", None)],
            local_value: slot.answer.clone(),
            cloud_value: None,
            source_value: None,
            proposed_patch: None,
            undo: None,
            auto_applied: false,
            applied_at: None,
            status: DecisionStatusV1::Open,
            reason_code: reason::SUBSTANTIVE_DIVERGENCE.to_string(),
            dependency_group: None,
        });
    }
    for number in cloud_numbers.difference(&local_numbers) {
        let Some(slot) = input.cloud.slot_by_question(*number) else { continue };
        items.push(DecisionItemV1 {
            decision_id: DecisionItemV1::decision_id_for(
                DecisionTargetTypeV1::Slot,
                &slot.slot_id,
                DecisionFieldV1::SlotPlacement,
            ),
            resolution: DecisionResolutionV1::NeedsReview,
            code: "LOCAL_SLOT_MISSING".to_string(),
            severity: DecisionSeverityV1::Warning,
            title: format!("第 {number} 题只在云端识别中"),
            user_message: format!(
                "本地识别没有找到第 {number} 题，云端识别给出了该题，请确认是否需要补上。"
            ),
            target: DecisionTargetV1 {
                target_type: DecisionTargetTypeV1::Slot,
                target_id: slot.slot_id.clone(),
                task_id: Some(slot.task_id.clone()),
                node_id: None,
                question_numbers: vec![*number],
            },
            field: DecisionFieldV1::SlotPlacement,
            evidence: vec![evidence_for(ChainKindV1::Cloud, "slot", None)],
            local_value: None,
            cloud_value: slot.answer.clone(),
            source_value: None,
            // 插入槽位需要完整的 node + slot + expression，无法从候选单方面安全合成：
            // 交给用户处理（proposal-only），不自动插入。
            proposed_patch: None,
            undo: None,
            auto_applied: false,
            applied_at: None,
            status: DecisionStatusV1::Open,
            reason_code: reason::SUBSTANTIVE_DIVERGENCE.to_string(),
            dependency_group: Some(format!("dep:missing-slot-q{number}")),
        });
    }
}

/// R2：答案 + 答案位交互语义。
fn compare_slots(input: &CompareInput<'_>, items: &mut Vec<DecisionItemV1>) {
    for slot in &input.local.slots {
        let canonical_answer = canonical_answer(input.canonical, &slot.slot_id);
        let cloud_slot = input.cloud.slot_by_question(slot.question_number);
        let source_confirmed = input
            .source
            .is_confirmed(DecisionTargetTypeV1::Slot, &slot.slot_id, DecisionFieldV1::Answer);
        let source_suggested = input
            .source
            .suggested(DecisionTargetTypeV1::Slot, &slot.slot_id, DecisionFieldV1::Answer)
            .cloned();
        let source_verdict = input
            .source
            .verdict(DecisionTargetTypeV1::Slot, &slot.slot_id, DecisionFieldV1::Answer);

        let local_key = answer_compare_key(slot.answer.as_ref());
        let cloud_key = cloud_slot.and_then(|cloud| answer_compare_key(cloud.answer.as_ref()).into());
        let cloud_key = cloud_key.unwrap_or_default();
        let cloud_answer = cloud_slot.and_then(|cloud| cloud.answer.clone());
        let cloud_usable = input.cloud_usable();

        // 用户已经改过这一槽位：迟到结果一律 proposal-only，不做自动修正。
        let user_edited = canonical_slot(input.canonical, &slot.slot_id)
            .map(|value| is_user_edited(input.canonical, value.get("slotId").and_then(Value::as_str).unwrap_or("")))
            .unwrap_or(false)
            || !matches!(
                answer_compare_key(canonical_answer),
                key if key.is_empty()
            ) && answer_compare_key(canonical_answer) != local_key;

        let mut evidence: Vec<DecisionEvidenceV1> = Vec::new();
        if slot.has_source_evidence {
            evidence.push(evidence_for(ChainKindV1::Local, "slot_anchor", None));
        }
        if cloud_slot.map(|value| value.has_source_evidence).unwrap_or(false) {
            evidence.push(evidence_for(ChainKindV1::Cloud, "group_quote", None));
        }

        // ── 一致分支 ────────────────────────────────────────────────
        if cloud_usable && !cloud_key.is_empty() && cloud_key == local_key {
            // 两路一致、但**原文件明确给出不同值**：这正是「两路都错」的最危险
            // 情形。不能因为本地与云端一致就对外宣称已确认，也不能降级成
            // 「证据不足」——原文件在这里是确凿证据，必须作为实质分歧暴露。
            if let Some(suggested) = source_suggested.clone() {
                let suggested_key = answer_compare_key(Some(&suggested));
                if !suggested_key.is_empty() && suggested_key != local_key {
                    items.push(build_item(
                        &slot.slot_id,
                        slot,
                        DecisionFieldV1::Answer,
                        DecisionResolutionV1::NeedsReview,
                        "ANSWER_SOURCE_CONFLICT",
                        DecisionSeverityV1::Blocker,
                        format!("第 {} 题答案与原文不一致", slot.question_number),
                        format!(
                            "第 {} 题的本地与云端结果一致，但原文件中是「{}」，请确认采用哪一个。",
                            slot.question_number,
                            answer_display(&suggested)
                        ),
                        slot.answer.clone(),
                        cloud_answer.clone(),
                        Some(suggested.clone()),
                        Some(answer_patch(&slot.slot_id, &suggested)),
                        evidence,
                        reason::SUBSTANTIVE_DIVERGENCE,
                        user_edited,
                    ));
                    continue;
                }
            }
            if source_confirmed {
                items.push(build_item(
                    &slot.slot_id,
                    slot,
                    DecisionFieldV1::Answer,
                    DecisionResolutionV1::Agreed,
                    "ANSWER_AGREED",
                    DecisionSeverityV1::Info,
                    format!("第 {} 题三路一致", slot.question_number),
                    format!("第 {} 题的答案已由本地、云端与原文一致确认。", slot.question_number),
                    slot.answer.clone(),
                    cloud_answer.clone(),
                    None,
                    None,
                    evidence,
                    reason::RULES_MATCH,
                    user_edited,
                ));
            } else {
                // 验收项 4：结论一致但缺少原文证据 → 不得判为已验证。
                items.push(build_item(
                    &slot.slot_id,
                    slot,
                    DecisionFieldV1::Answer,
                    DecisionResolutionV1::Unverifiable,
                    "ANSWER_AGREED_UNVERIFIED",
                    DecisionSeverityV1::Info,
                    format!("第 {} 题缺少原文证据", slot.question_number),
                    format!(
                        "第 {} 题的本地与云端结果一致，但没有可核验的原文证据，未标记为已验证。",
                        slot.question_number
                    ),
                    slot.answer.clone(),
                    cloud_answer.clone(),
                    None,
                    None,
                    evidence,
                    reason::NO_SOURCE_EVIDENCE,
                    user_edited,
                ));
            }
            continue;
        }

        // ── 分歧 / 缺失分支 ─────────────────────────────────────────
        let (resolution, code, severity, title, message, proposed, reason_code) =
            if let Some(suggested) = source_suggested.clone() {
                // 原文件给出了不同答案：这是可以落到具体 patch 的实质分歧。
                let suggested_key = answer_compare_key(Some(&suggested));
                if source_verdict == Some(SourceVerdictV1::Suggested) {
                    (
                        DecisionResolutionV1::NeedsReview,
                        "ANSWER_FILL_FROM_SOURCE",
                        DecisionSeverityV1::Warning,
                        format!("第 {} 题可以按原文补全答案", slot.question_number),
                        format!(
                            "第 {} 题本地没有答案，原文件中是「{}」，可以按原文补上。",
                            slot.question_number,
                            answer_display(&suggested)
                        ),
                        Some(answer_patch(&slot.slot_id, &suggested)),
                        reason::RULES_MATCH,
                    )
                } else if !suggested_key.is_empty() && suggested_key != local_key {
                    (
                        DecisionResolutionV1::NeedsReview,
                        "ANSWER_SOURCE_CONFLICT",
                        DecisionSeverityV1::Blocker,
                        format!("第 {} 题答案与原文不一致", slot.question_number),
                        format!(
                            "第 {} 题的答案在原文件中是「{}」，本地识别得到的是「{}」，请确认采用哪一个。",
                            slot.question_number,
                            answer_display(&suggested),
                            answer_display(slot.answer.as_ref().unwrap_or(&Value::Null))
                        ),
                        Some(answer_patch(&slot.slot_id, &suggested)),
                        reason::SUBSTANTIVE_DIVERGENCE,
                    )
                } else {
                    (
                        DecisionResolutionV1::NeedsReview,
                        "ANSWER_CONFLICT",
                        DecisionSeverityV1::Blocker,
                        format!("第 {} 题两路答案不一致", slot.question_number),
                        format!("第 {} 题的本地与云端答案不同，请确认。", slot.question_number),
                        None,
                        reason::SUBSTANTIVE_DIVERGENCE,
                    )
                }
            } else if !cloud_usable {
                if source_confirmed {
                    items.push(build_item(
                        &slot.slot_id,
                        slot,
                        DecisionFieldV1::Answer,
                        DecisionResolutionV1::Agreed,
                        "ANSWER_SOURCE_CONFIRMED",
                        DecisionSeverityV1::Info,
                        format!("第 {} 题已由原文确认", slot.question_number),
                        format!("第 {} 题的答案已由原文件确认。", slot.question_number),
                        slot.answer.clone(),
                        cloud_answer.clone(),
                        None,
                        None,
                        evidence,
                        reason::RULES_MATCH,
                        user_edited,
                    ));
                } else {
                    items.push(build_item(
                        &slot.slot_id,
                        slot,
                        DecisionFieldV1::Answer,
                        DecisionResolutionV1::Unverifiable,
                        "ANSWER_EVIDENCE_MISSING",
                        DecisionSeverityV1::Info,
                        format!("第 {} 题无法验证", slot.question_number),
                        format!(
                            "第 {} 题缺少云端结果与原文证据，无法判断答案是否正确。",
                            slot.question_number
                        ),
                        slot.answer.clone(),
                        None,
                        None,
                        None,
                        evidence,
                        reason::EVIDENCE_MISSING,
                        user_edited,
                    ));
                }
                continue;
            } else if answer_is_empty(slot.answer.as_ref()) && !cloud_key.is_empty() {
                // 本地缺答案、云端有答案：只有原文确认时才允许自动补全。
                (
                    DecisionResolutionV1::NeedsReview,
                    "ANSWER_MISSING_LOCAL",
                    DecisionSeverityV1::Blocker,
                    format!("第 {} 题缺少答案", slot.question_number),
                    format!("第 {} 题本地识别没有答案，云端识别给出了答案，请确认。", slot.question_number),
                    cloud_answer
                        .as_ref()
                        .map(|value| answer_patch(&slot.slot_id, value)),
                    reason::SUBSTANTIVE_DIVERGENCE,
                )
            } else {
                (
                    DecisionResolutionV1::Unverifiable,
                    "ANSWER_CONFLICT_UNVERIFIED",
                    DecisionSeverityV1::Warning,
                    format!("第 {} 题答案无法判断", slot.question_number),
                    format!(
                        "第 {} 题的本地与云端答案不同，且缺少原文证据，无法判断哪一个正确。",
                        slot.question_number
                    ),
                    None,
                    reason::EVIDENCE_MISSING,
                )
            };

        let mut item = build_item(
            &slot.slot_id,
            slot,
            DecisionFieldV1::Answer,
            resolution,
            code,
            severity,
            title,
            message,
            slot.answer.clone(),
            cloud_answer.clone(),
            proposed,
            None,
            evidence,
            reason_code,
            user_edited,
        );
        item.source_value = source_suggested;
        items.push(item);
    }
}

/// R3：题组题型、题干、以及答案位的交互语义。
fn compare_groups(input: &CompareInput<'_>, items: &mut Vec<DecisionItemV1>) {
    for group in &input.local.task_groups {
        let cloud_group = input.cloud.group(&group.task_id).or_else(|| {
            // 云端 taskId 可能与本地不同：退化为按题号集合匹配。
            input.cloud.task_groups.iter().find(|candidate| {
                let mut left: Vec<u32> = super::candidate::expand_question_numbers(&candidate.display_range);
                let mut right: Vec<u32> =
                    super::candidate::expand_question_numbers(&group.display_range);
                left.sort_unstable();
                right.sort_unstable();
                !left.is_empty() && left == right
            })
        });
        let Some(cloud_group) = cloud_group else { continue };

        // 题型
        if group.task_type != cloud_group.task_type {
            items.push(DecisionItemV1 {
                decision_id: DecisionItemV1::decision_id_for(
                    DecisionTargetTypeV1::Task,
                    &group.task_id,
                    DecisionFieldV1::GroupKind,
                ),
                resolution: DecisionResolutionV1::NeedsReview,
                code: "TASK_TYPE_CONFLICT".to_string(),
                severity: DecisionSeverityV1::Warning,
                title: format!("题型不一致：{}", group.task_id),
                user_message: format!(
                    "本地识别为「{}」，云端识别为「{}」，请确认题目类型。",
                    group.task_type, cloud_group.task_type
                ),
                target: DecisionTargetV1 {
                    target_type: DecisionTargetTypeV1::Task,
                    target_id: group.task_id.clone(),
                    task_id: Some(group.task_id.clone()),
                    node_id: None,
                    question_numbers: super::candidate::expand_question_numbers(&group.display_range),
                },
                field: DecisionFieldV1::GroupKind,
                evidence: vec![evidence_for(ChainKindV1::Cloud, "group_kind", None)],
                local_value: Some(Value::String(group.task_type.clone())),
                cloud_value: Some(Value::String(cloud_group.task_type.clone())),
                source_value: None,
                // 题型属于实质变化：不做自动应用。
                proposed_patch: Some(json!({
                    "op": "setTaskType",
                    "taskId": group.task_id,
                    "taskType": cloud_group.task_type,
                    "preserveProvenance": true
                })),
                undo: None,
                auto_applied: false,
                applied_at: None,
                status: DecisionStatusV1::Open,
                reason_code: reason::SUBSTANTIVE_DIVERGENCE.to_string(),
                dependency_group: Some(format!("dep:task-type:{}", group.task_id)),
            });
        }

        // 题干
        let local_prompt = normalize_text(&group.instructions_text);
        let cloud_prompt = normalize_text(&cloud_group.instructions_text);
        if !local_prompt.is_empty() && !cloud_prompt.is_empty() && local_prompt != cloud_prompt {
            let source_confirmed = input.source.is_confirmed(
                DecisionTargetTypeV1::Task,
                &group.task_id,
                DecisionFieldV1::Prompt,
            );
            items.push(DecisionItemV1 {
                decision_id: DecisionItemV1::decision_id_for(
                    DecisionTargetTypeV1::Task,
                    &group.task_id,
                    DecisionFieldV1::Prompt,
                ),
                resolution: DecisionResolutionV1::NeedsReview,
                code: "PROMPT_CONFLICT".to_string(),
                severity: DecisionSeverityV1::Warning,
                title: format!("题干不一致：{}", group.task_id),
                user_message: "该题组的题干在本地与云端识别中不同，请确认。".to_string(),
                target: DecisionTargetV1 {
                    target_type: DecisionTargetTypeV1::Task,
                    target_id: group.task_id.clone(),
                    task_id: Some(group.task_id.clone()),
                    node_id: None,
                    question_numbers: super::candidate::expand_question_numbers(&group.display_range),
                },
                field: DecisionFieldV1::Prompt,
                evidence: vec![
                    evidence_for(ChainKindV1::Local, "instructions", Some(group.instructions_text.clone())),
                    evidence_for(ChainKindV1::Cloud, "instructions", Some(cloud_group.instructions_text.clone())),
                ],
                local_value: Some(Value::String(group.instructions_text.clone())),
                cloud_value: Some(Value::String(cloud_group.instructions_text.clone())),
                source_value: None,
                // 题干是 rich content（ContentNodeV2）：单方面替换会破坏结构，只做 review。
                proposed_patch: None,
                undo: None,
                auto_applied: false,
                applied_at: None,
                status: DecisionStatusV1::Open,
                reason_code: if source_confirmed {
                    reason::SUBSTANTIVE_DIVERGENCE.to_string()
                } else {
                    reason::EVIDENCE_MISSING.to_string()
                },
                dependency_group: None,
            });
        }

        // 选项库
        match (&group.option_bank, &cloud_group.option_bank) {
            (Some(local_bank), Some(cloud_bank)) => {
                let local_labels = option_label_text_map(&local_bank.options);
                let cloud_labels = option_label_text_map(&cloud_bank.options);
                if local_labels != cloud_labels {
                    items.push(DecisionItemV1 {
                        decision_id: DecisionItemV1::decision_id_for(
                            DecisionTargetTypeV1::Task,
                            &group.task_id,
                            DecisionFieldV1::OptionBank,
                        ),
                        resolution: DecisionResolutionV1::NeedsReview,
                        code: "OPTION_BANK_CONFLICT".to_string(),
                        severity: DecisionSeverityV1::Warning,
                        title: format!("选项不一致：{}", group.task_id),
                        user_message: "该题组的选项内容在本地与云端识别中不同，请确认。".to_string(),
                        target: DecisionTargetV1 {
                            target_type: DecisionTargetTypeV1::Task,
                            target_id: group.task_id.clone(),
                            task_id: Some(group.task_id.clone()),
                            node_id: None,
                            question_numbers: super::candidate::expand_question_numbers(&group.display_range),
                        },
                        field: DecisionFieldV1::OptionBank,
                        evidence: vec![evidence_for(ChainKindV1::Cloud, "option_bank", None)],
                        local_value: Some(serde_json::to_value(&local_bank.options).unwrap_or(Value::Null)),
                        cloud_value: Some(serde_json::to_value(&cloud_bank.options).unwrap_or(Value::Null)),
                        source_value: None,
                        proposed_patch: Some(json!({
                            "op": "setOptionBank",
                            "taskId": group.task_id,
                            "optionBank": {
                                "optionBankId": group
                                    .option_bank
                                    .as_ref()
                                    .map(|bank| bank.option_bank_id.clone())
                                    .unwrap_or_else(|| "option-bank".to_string()),
                                "scope": "task_group",
                                "options": cloud_bank.options,
                                "allowReuse": cloud_bank.allow_reuse,
                                "sourceAnchors": []
                            },
                            "preserveProvenance": true
                        })),
                        undo: None,
                        auto_applied: false,
                        applied_at: None,
                        status: DecisionStatusV1::Open,
                        reason_code: reason::SUBSTANTIVE_DIVERGENCE.to_string(),
                        dependency_group: Some(format!("dep:option-bank:{}", group.task_id)),
                    });
                }
            }
            (None, Some(cloud_bank)) if !cloud_bank.options.is_empty() => {
                items.push(DecisionItemV1 {
                    decision_id: DecisionItemV1::decision_id_for(
                        DecisionTargetTypeV1::Task,
                        &group.task_id,
                        DecisionFieldV1::OptionBank,
                    ),
                    resolution: DecisionResolutionV1::NeedsReview,
                    code: "OPTION_BANK_MISSING_LOCAL".to_string(),
                    severity: DecisionSeverityV1::Warning,
                    title: format!("缺少选项：{}", group.task_id),
                    user_message: "云端识别给出了该题组的选项，本地识别没有，请确认。".to_string(),
                    target: DecisionTargetV1 {
                        target_type: DecisionTargetTypeV1::Task,
                        target_id: group.task_id.clone(),
                        task_id: Some(group.task_id.clone()),
                        node_id: None,
                        question_numbers: super::candidate::expand_question_numbers(&group.display_range),
                    },
                    field: DecisionFieldV1::OptionBank,
                    evidence: vec![evidence_for(ChainKindV1::Cloud, "option_bank", None)],
                    local_value: None,
                    cloud_value: Some(serde_json::to_value(&cloud_bank.options).unwrap_or(Value::Null)),
                    source_value: None,
                    proposed_patch: None,
                    undo: None,
                    auto_applied: false,
                    applied_at: None,
                    status: DecisionStatusV1::Open,
                    reason_code: reason::SUBSTANTIVE_DIVERGENCE.to_string(),
                    dependency_group: Some(format!("dep:option-bank:{}", group.task_id)),
                });
            }
            _ => {}
        }
    }

    // 答案位交互语义（本地稿本身的一致性）：与编译规则同源，避免「能编辑但发不出去」。
    for slot in &input.local.slots {
        let canonical_slot_value = canonical_slot(input.canonical, &slot.slot_id);
        let Some(canonical_slot_value) = canonical_slot_value else { continue };
        let interaction = canonical_slot_value
            .get("interaction")
            .and_then(Value::as_str)
            .unwrap_or("");
        let answer = canonical_answer(input.canonical, &slot.slot_id);
        if interaction.is_empty() || answer.map(|value| value.get("kind").and_then(Value::as_str).is_none()).unwrap_or(true) {
            continue;
        }
        let answer_kind = answer.and_then(|value| value.get("kind")).and_then(Value::as_str).unwrap_or("");
        let mismatch = matches!(interaction, "text")
            && !matches!(answer_kind, "text" | "unresolved")
            || matches!(interaction, "radio" | "checkbox" | "select" | "dragdrop" | "hotspot")
                && !matches!(answer_kind, "option" | "unresolved");
        if mismatch {
            items.push(DecisionItemV1 {
                decision_id: DecisionItemV1::decision_id_for(
                    DecisionTargetTypeV1::Slot,
                    &slot.slot_id,
                    DecisionFieldV1::SlotInteraction,
                ),
                resolution: DecisionResolutionV1::NeedsReview,
                code: "SLOT_INTERACTION_ANSWER_MISMATCH".to_string(),
                severity: DecisionSeverityV1::Blocker,
                title: format!("第 {} 题作答方式与答案类型不一致", slot.question_number),
                user_message: format!(
                    "第 {} 题的作答方式是「{}」，答案却是「{}」，该题无法提交，请修改。",
                    slot.question_number, interaction, answer_kind
                ),
                target: DecisionTargetV1 {
                    target_type: DecisionTargetTypeV1::Slot,
                    target_id: slot.slot_id.clone(),
                    task_id: Some(slot.task_id.clone()),
                    node_id: None,
                    question_numbers: vec![slot.question_number],
                },
                field: DecisionFieldV1::SlotInteraction,
                evidence: vec![evidence_for(ChainKindV1::Local, "slot", None)],
                local_value: Some(Value::String(interaction.to_string())),
                cloud_value: None,
                source_value: Some(Value::String(answer_kind.to_string())),
                proposed_patch: None,
                undo: None,
                auto_applied: false,
                applied_at: None,
                status: DecisionStatusV1::Open,
                reason_code: reason::SUBSTANTIVE_DIVERGENCE.to_string(),
                dependency_group: Some(format!("dep:slot-interaction:{}", slot.slot_id)),
            });
        }
    }
}

/// R6：资源闭包。
fn compare_assets(input: &CompareInput<'_>, items: &mut Vec<DecisionItemV1>) {
    let canonical_assets: BTreeMap<String, String> = input
        .canonical
        .get("assets")
        .and_then(Value::as_array)
        .map(|assets| {
            assets
                .iter()
                .filter_map(|asset| {
                    Some((
                        asset.get("assetId").and_then(Value::as_str)?.to_string(),
                        asset.get("sha256").and_then(Value::as_str).unwrap_or("").to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    for asset in &input.local.assets {
        let canonical_sha = canonical_assets.get(&asset.asset_id);
        if canonical_sha.is_none() || canonical_sha.is_some_and(|sha| sha != &asset.sha256) {
            items.push(DecisionItemV1 {
                decision_id: DecisionItemV1::decision_id_for(
                    DecisionTargetTypeV1::Asset,
                    &asset.asset_id,
                    DecisionFieldV1::Asset,
                ),
                resolution: DecisionResolutionV1::NeedsReview,
                code: "ASSET_INTEGRITY_MISMATCH".to_string(),
                severity: DecisionSeverityV1::Blocker,
                title: "资源与权威稿不一致".to_string(),
                user_message: "有一个图片/资源在识别结果与权威稿之间不一致，发布前需要确认。".to_string(),
                target: DecisionTargetV1 {
                    target_type: DecisionTargetTypeV1::Asset,
                    target_id: asset.asset_id.clone(),
                    task_id: None,
                    node_id: None,
                    question_numbers: vec![],
                },
                field: DecisionFieldV1::Asset,
                evidence: vec![evidence_for(ChainKindV1::Local, "asset", None)],
                local_value: Some(Value::String(asset.sha256.clone())),
                cloud_value: canonical_sha.cloned().map(Value::String),
                source_value: None,
                proposed_patch: None,
                undo: None,
                auto_applied: false,
                applied_at: None,
                status: DecisionStatusV1::Open,
                reason_code: reason::SUBSTANTIVE_DIVERGENCE.to_string(),
                dependency_group: None,
            });
        }
    }
}

/// R7：来源覆盖（权威稿的质量账本，不是新增判断）。
fn compare_source_coverage(input: &CompareInput<'_>, items: &mut Vec<DecisionItemV1>) {
    let unassigned = input
        .canonical
        .pointer("/quality/coverageStatus/unassignedSourceNodeIds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if unassigned.is_empty() {
        return;
    }
    items.push(DecisionItemV1 {
        decision_id: DecisionItemV1::decision_id_for(
            DecisionTargetTypeV1::Document,
            "document",
            DecisionFieldV1::SourceCoverage,
        ),
        resolution: DecisionResolutionV1::NeedsReview,
        code: "SIGNIFICANT_SOURCE_TEXT_UNASSIGNED".to_string(),
        severity: DecisionSeverityV1::Warning,
        title: format!("有 {} 处原文内容没有被归入任何题目", unassigned.len()),
        user_message: format!(
            "原文件中有 {} 处内容没有被分配到题目里，请确认是否遗漏了题组。",
            unassigned.len()
        ),
        target: DecisionTargetV1 {
            target_type: DecisionTargetTypeV1::Document,
            target_id: "document".to_string(),
            task_id: None,
            node_id: None,
            question_numbers: vec![],
        },
        field: DecisionFieldV1::SourceCoverage,
        evidence: vec![evidence_for(ChainKindV1::Source, "coverage_ledger", None)],
        local_value: None,
        cloud_value: None,
        source_value: Some(Value::Array(unassigned)),
        proposed_patch: None,
        undo: None,
        auto_applied: false,
        applied_at: None,
        status: DecisionStatusV1::Open,
        reason_code: reason::SUBSTANTIVE_DIVERGENCE.to_string(),
        dependency_group: None,
    });
}

// ── 小工具 ─────────────────────────────────────────────────────────────

pub(crate) fn option_label_text_map(options: &[crate::schema::recognition_v1::CandidateOptionV1]) -> BTreeMap<String, String> {
    options
        .iter()
        .map(|option| (option.label.trim().to_uppercase(), normalize_text(&option.text)))
        .collect()
}

pub(crate) fn answer_patch(slot_id: &str, value: &Value) -> Value {
    json!({
        "op": "setAnswer",
        "slotId": slot_id,
        "value": value,
        "preserveProvenance": true
    })
}

fn answer_display(value: &Value) -> String {
    match value.get("kind").and_then(Value::as_str) {
        Some("text") => value
            .get("values")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default(),
        Some("option") => value
            .get("labels")
            .and_then(Value::as_array)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default(),
        _ => "（未给出）".to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_item(
    slot_id: &str,
    slot: &CandidateSlotV1,
    field: DecisionFieldV1,
    resolution: DecisionResolutionV1,
    code: &str,
    severity: DecisionSeverityV1,
    title: String,
    user_message: String,
    local_value: Option<Value>,
    cloud_value: Option<Value>,
    proposed_patch: Option<Value>,
    undo: Option<Value>,
    evidence: Vec<DecisionEvidenceV1>,
    reason_code: &str,
    user_edited: bool,
) -> DecisionItemV1 {
    DecisionItemV1 {
        decision_id: DecisionItemV1::decision_id_for(DecisionTargetTypeV1::Slot, slot_id, field),
        resolution,
        code: code.to_string(),
        severity,
        title,
        user_message,
        target: DecisionTargetV1 {
            target_type: DecisionTargetTypeV1::Slot,
            target_id: slot_id.to_string(),
            task_id: Some(slot.task_id.clone()),
            node_id: slot.host_node_id.clone(),
            question_numbers: vec![slot.question_number],
        },
        field,
        evidence,
        local_value,
        cloud_value,
        source_value: None,
        proposed_patch: if user_edited { None } else { proposed_patch },
        undo,
        auto_applied: false,
        applied_at: None,
        status: DecisionStatusV1::Open,
        reason_code: if user_edited {
            reason::USER_EDITED.to_string()
        } else {
            reason_code.to_string()
        },
        dependency_group: None,
    }
}

/// 供 adjudicate 复用的证据构造（source finding → evidence）。
pub(crate) fn source_finding_evidence(
    target_type: DecisionTargetTypeV1,
    target_id: &str,
    field: DecisionFieldV1,
) -> Vec<DecisionEvidenceV1> {
    source_evidence(target_type, target_id, field)
}

/// 供 adjudicate 判断原文件是否「明确反对」当前值。
pub(crate) fn source_contradicts(
    source: &SourceVerificationV1,
    target_type: DecisionTargetTypeV1,
    target_id: &str,
    field: DecisionFieldV1,
) -> bool {
    source
        .finding(target_type, target_id, field)
        .map(|finding| finding.verdict == SourceVerdictV1::Contradicted)
        .unwrap_or(false)
}
