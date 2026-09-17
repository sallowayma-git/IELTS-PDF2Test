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

use super::candidate::align_answer_value;
use super::engine::{classify_cloud_error, ModelCallFailure, AdjudicationRunner};
use super::rules::{
    answer_compare_key, answer_is_empty, answer_patch, canonical_answer, compare, CompareInput,
};
use super::source::SourceVerificationV1;
use crate::schema::recognition_v1::{
    reason, ChainKindV1, ChainStatusSummaryV1, ChainStatusV1, DecisionEvidenceV1, DecisionFieldV1,
    DecisionItemV1, DecisionResolutionV1, DecisionSeverityV1, DecisionStatusV1, DecisionSummaryV1,
    DecisionTargetTypeV1, RecognitionCandidateV1, RecognitionDecisionV1, StageStateV1,
    StageStatusV1, RECOGNITION_DECISION_V1_SCHEMA_VERSION,
};

/// 单次裁决请求最多携带的分歧项数。
///
/// 「预算 2」只有在**按批打包**时才成立：一份 40 题的卷子若有 8 处分歧，按项调用会
/// 超预算 4 倍；打包成一次请求才是 1 次主裁决 + 1 次受约束修复。超出软上限的项
/// **留在待确认**并带 `ADJUDICATION_BUDGET_EXHAUSTED`——不静默丢弃，也不静默超支。
pub(crate) const ADJUDICATION_BATCH_SOFT_LIMIT: usize = 12;

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
    /// A4：分歧裁决的模型通道。`None` = 无可用模型，分歧全部留在 `NeedsReview`。
    pub adjudicator: Option<AdjudicationRunner<'a>>,
}

pub(crate) struct AdjudicationOutcome {
    pub decision: RecognitionDecisionV1,
    /// 已通过全部自动应用条件、且合并后结构合法的 patch（按 decision id 对齐）。
    pub auto_apply_candidates: Vec<String>,
    /// 裁决链的**如实**状态（A4）。原先在 `engine.rs` 硬编码成 `Succeeded`，
    /// 等于无论模型有没有跑过都对 UI 说「裁决成功」。现在由裁决层回报。
    pub adjudication: StageStatusV1,
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
    //
    // 只认**确定性抽取**得到的建议：模型读原文件得出的值不带确定性保证，让它充当本守卫的
    // 输入等于让一次模型回复直接授权改题稿，与模块级约束（自动应用只依赖确定性条件）相悖。
    // 用户仍然能看到模型给出的建议并一键接受——差别只在「谁来敲下这一下」。
    let inferred = input.source.deterministic_suggested(
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

// ── A4：分歧裁决的模型通道 ─────────────────────────────────────────────

/// 该分歧项是否**值得**交给模型。
///
/// 判据刻意不看 `code`（规则码是前端契约词表，拿它做控制流等于把展示层当逻辑层）：
/// - 云端链必须可用：云端没跑时没有任何可裁决的对手方，交模型只是白花钱；
/// - 字段必须是答案：题干/选项/结构类改动单方面替换会破坏题稿，一律留人工；
/// - 「用户已改过」与「依赖组被阻塞」是确定性规则的结论，不该被模型翻案；
/// - 三条链必须真的不一致（见 [`has_answer_divergence`]）。
fn needs_adjudication(item: &DecisionItemV1, cloud_usable: bool) -> bool {
    cloud_usable
        && item.field == DecisionFieldV1::Answer
        && matches!(
            item.resolution,
            DecisionResolutionV1::NeedsReview | DecisionResolutionV1::Unverifiable
        )
        && item.reason_code != reason::USER_EDITED
        && item.reason_code != reason::DEPENDENCY_BLOCKED
        && has_answer_divergence(item)
}

/// 三条链在**答案层面**是否不一致。
///
/// 只看**确实给出了值**的链（`None` = 该链没有结论，不算一种取值），但空答案是
/// **一种取值**：`{local: 空, cloud: "stencilling"}` 正是「本地缺答案」的分歧。
/// 比较用 `answer_compare_key`，因为形状不同但值相同的答案**不是**分歧——那正是
/// [`align_answer_value`] 要消除的假分歧。
fn has_answer_divergence(item: &DecisionItemV1) -> bool {
    let distinct: BTreeSet<String> = [
        item.local_value.as_ref(),
        item.cloud_value.as_ref(),
        item.source_value.as_ref(),
    ]
    .into_iter()
    .flatten()
    .map(|value| answer_compare_key(Some(value)))
    .collect();
    distinct.len() > 1
}

enum RulingOutcome {
    /// 已落实为具体建议（值取自某条链）。
    Applied,
    /// 模型给了三条链上都不存在的值：**不采纳**，只如实告知。
    NotCorroborated,
    /// 没有拿到可用裁定（模型明示无法裁定 / 本次未覆盖该项 / 预算 / 调用失败）。
    Unruled,
}

/// 把一条裁定落到决策项上。**只改建议与解释，绝不改「这条修正是否已写入」。**
///
/// 硬规则：**模型只能「选择」，不能「发明」。** 裁决值必须与它 `chosen` 指向的那条链
/// 的值逐键一致（形状对齐之后）。否则它就是一个三路都不存在的答案——那属于新增论断，
/// 而 A4 的职责是在既有结论里挑一个，不是造一个新答案。
fn apply_ruling(
    item: &mut DecisionItemV1,
    ruling: &Value,
    local: &RecognitionCandidateV1,
    canonical: &Value,
) -> RulingOutcome {
    let rationale = ruling
        .get("rationale")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let Some(chosen) = ruling.get("chosen").and_then(Value::as_str) else {
        return mark_unruled(item, reason::ADJUDICATION_DECLINED, "模型没有给出采用哪一路的结论");
    };
    if chosen == "unresolved" {
        return mark_unruled(item, reason::ADJUDICATION_DECLINED, "模型核阅后仍无法裁定");
    }
    let chain_value = match chosen {
        "local" => item.local_value.clone(),
        "cloud" => item.cloud_value.clone(),
        "source" => item.source_value.clone(),
        _ => None,
    };
    let Some(raw_value) = ruling.get("value").filter(|value| !value.is_null()) else {
        return mark_unruled(item, reason::ADJUDICATION_DECLINED, "模型没有给出裁决值");
    };

    // 形状对齐：目标形状优先取**本地作者约定**（与云端候选对齐同源）；本地无答案时
    // 退回权威稿；两者都取不到才用模型原值（网关适配器已保证它是合法 `AnswerValueV2`）。
    let target_shape = local
        .slot(&item.target.target_id)
        .and_then(|slot| slot.answer.clone())
        .or_else(|| canonical_answer(canonical, &item.target.target_id).cloned());
    let bank = local
        .slot(&item.target.target_id)
        .and_then(|slot| local.group(&slot.task_id).and_then(|group| group.option_bank.as_ref()));
    let aligned = target_shape
        .as_ref()
        .and_then(|shape| align_answer_value(raw_value, shape, bank))
        .unwrap_or_else(|| raw_value.clone());

    let corroborated = chain_value
        .as_ref()
        .map(|value| {
            answer_compare_key(Some(value)) == answer_compare_key(Some(&aligned))
                && !answer_compare_key(Some(value)).is_empty()
        })
        .unwrap_or(false);
    if !corroborated {
        item.reason_code = reason::ADJUDICATION_VALUE_NOT_CORROBORATED.to_string();
        item.user_message = format!(
            "{}（模型给出的答案在本地/云端/原文三路中都不存在，不作为建议采纳，请人工确认）",
            item.user_message
        );
        return RulingOutcome::NotCorroborated;
    }

    let label = match chosen {
        "local" => "本地识别",
        "source" => "原文件",
        _ => "云端识别",
    };
    let reason_text = if rationale.is_empty() {
        "未给出理由".to_string()
    } else {
        format!("理由：{rationale}")
    };
    item.proposed_patch = Some(answer_patch(&item.target.target_id, &aligned));
    // 「无法判断」的项拿到了一条有落地形态的建议 → 转成待确认：契约规定
    // `Unverifiable` 项不得携带 patch，而带 patch 的项对用户就是「一键确认」。
    //
    // **刻意不改自动应用资格**：自动写入仍然只认确定性证据（源断言），
    // 见 [`auto_apply_eligible`] 的第 3 条守卫。让模型输出直接解锁自动改稿，
    // 会推翻本模块「自动应用不依赖模型置信度」的硬约束。
    if item.resolution == DecisionResolutionV1::Unverifiable {
        item.resolution = DecisionResolutionV1::NeedsReview;
    }
    item.user_message = format!("{}（模型核阅后建议采用{label}的结果，{reason_text}，请确认）", item.user_message);
    item.evidence.push(DecisionEvidenceV1 {
        chain: match chosen {
            "local" => ChainKindV1::Local,
            "source" => ChainKindV1::Source,
            _ => ChainKindV1::Cloud,
        },
        anchor_kind: "adjudication_ruling".to_string(),
        page_index: None,
        quote: if rationale.is_empty() { None } else { Some(rationale) },
        anchor: Some(serde_json::json!({
            "chosen": chosen,
            "corroborated": true,
            "confidence": ruling.get("confidence").cloned().unwrap_or(Value::Null),
        })),
    });
    RulingOutcome::Applied
}

fn mark_unruled(item: &mut DecisionItemV1, code: &str, note: &str) -> RulingOutcome {
    item.reason_code = code.to_string();
    item.user_message = format!("{}（{note}，未作为已核验结论，请人工确认）", item.user_message);
    RulingOutcome::Unruled
}

/// A4 主流程：挑分歧 → 交模型 → 落实裁定 → **如实**回报裁决链状态。
///
/// 插入点必须在 [`assign_dependency_groups`]（依赖组已定型）之后、资格计算之前：
/// 裁定会改写 `proposed_patch`，而 `auto_apply_eligible` 读的正是它。
///
/// 三条不许违反的语义：
/// 1. **没有模型可用时不碰任何项。** 确定性规则的结论原样保留（`None` 路径与
///    未接入 A4 时逐项一致），只在链状态上如实写「裁决未运行」；
/// 2. **没裁定 ≠ 裁过了。** 预算耗尽、模型拒绝、调用失败、模型漏答，全部留在
///    `NeedsReview` 并带上各自的原因码；
/// 3. **链状态由实际发生的事决定。** 全部有裁定 → `Succeeded`；部分 → `Partial`；
///    一个都没拿到 → `NotRun`/`Unusable` + 原因码。
fn apply_adjudication(items: &mut [DecisionItemV1], input: &AdjudicateInput<'_>) -> StageStatusV1 {
    let cloud_usable = matches!(
        input.cloud.status,
        ChainStatusV1::Succeeded | ChainStatusV1::Partial
    );
    let eligible: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| needs_adjudication(item, cloud_usable))
        .map(|(index, _)| index)
        .collect();
    if eligible.is_empty() {
        // 没有需要模型裁定的分歧：确定性裁决**确实**跑完了，如实报成功。
        return StageStatusV1::new(StageStateV1::Succeeded);
    }
    let Some(runner) = input.adjudicator else {
        // 语义 1：不碰项，只如实说明「这次没有模型参与裁决」。
        return StageStatusV1::with_reason(
            StageStateV1::NotRun,
            reason::ADJUDICATION_MODEL_UNAVAILABLE,
            "本次运行没有可用的裁决模型，分歧项全部留给人工确认。",
        );
    };

    // 单批软上限：超出部分如实标成预算耗尽（不静默丢弃、不静默超支）。
    let (batch, overflow) = eligible.split_at(eligible.len().min(ADJUDICATION_BATCH_SOFT_LIMIT));
    for index in overflow {
        mark_unruled(
            &mut items[*index],
            reason::ADJUDICATION_BUDGET_EXHAUSTED,
            "本次单批裁决项数已达上限",
        );
    }

    let request: Vec<Value> = batch
        .iter()
        .map(|index| adjudication_request_item(&items[*index]))
        .collect();
    let response = match runner(&request) {
        Ok(response) => response,
        Err(failure) => {
            let (state, code, message) = match failure {
                ModelCallFailure::BudgetExhausted => (
                    StageStateV1::NotRun,
                    reason::ADJUDICATION_BUDGET_EXHAUSTED.to_string(),
                    "本次运行的裁决预算已耗尽，分歧项全部留给人工确认。".to_string(),
                ),
                ModelCallFailure::Model(error) => {
                    // 复用云端链的错误分类，不另造一套。
                    let classified = classify_cloud_error(&error);
                    (
                        StageStateV1::from(classified.status),
                        classified.reason_code,
                        classified.message,
                    )
                }
            };
            for index in batch {
                mark_unruled(&mut items[*index], &code, &message);
            }
            return StageStatusV1::with_reason(state, code, message);
        }
    };

    let rulings = response
        .get("rulings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut applied = 0usize;
    let mut declined = 0usize;
    for index in batch {
        let item = &mut items[*index];
        let decision_id = item.decision_id.clone();
        let Some(ruling) = rulings.iter().find(|ruling| {
            ruling.get("decisionId").and_then(Value::as_str) == Some(decision_id.as_str())
        }) else {
            // 模型漏答：**不能**当作「已经裁过了」，如实记成未获裁定。
            mark_unruled(item, reason::ADJUDICATION_DECLINED, "模型本次未对该项给出裁定");
            declined += 1;
            continue;
        };
        match apply_ruling(item, ruling, input.local, input.canonical) {
            RulingOutcome::Applied => applied += 1,
            RulingOutcome::NotCorroborated => declined += 1,
            RulingOutcome::Unruled => declined += 1,
        }
    }

    // 汇总项数才是分母：`eligible` 里被软上限挡下的项已经单独标过预算耗尽。
    let total = batch.len();
    let unresolved = declined + overflow.len();
    if unresolved == 0 {
        StageStatusV1::new(StageStateV1::Succeeded)
    } else {
        let code = if overflow.is_empty() {
            reason::ADJUDICATION_DECLINED
        } else {
            reason::ADJUDICATION_BUDGET_EXHAUSTED
        };
        StageStatusV1::with_reason(
            StageStateV1::Partial,
            code,
            format!(
                "本次共 {total} 处分歧交模型裁定：{applied} 处已给出建议，{unresolved} 处未获得可用裁定，仍待人工确认。"
            ),
        )
    }
}

/// 交模型的最小上下文：**只给这一项的三路取值与题号**，不给整份稿子。
///
/// 不给指令文本/选项库是刻意的：A4 的任务是「在已有结论里挑一个」，
/// 给得越多越容易被模型用来编造第四条路。
fn adjudication_request_item(item: &DecisionItemV1) -> Value {
    serde_json::json!({
        "decisionId": item.decision_id,
        "targetId": item.target.target_id,
        "taskId": item.target.task_id,
        "questionNumber": item.target.question_numbers.first(),
        "field": item.field.as_str(),
        "local": item.local_value,
        "cloud": item.cloud_value,
        "source": item.source_value,
    })
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

    // ── A4：模型裁决 ───────────────────────────────────────────────
    //
    // 位置是硬约束：必须在依赖组定型**之后**（裁决不该改变组的划分），
    // 且在资格计算**之前**（裁决会改写 `proposed_patch`，而资格读的就是它）。
    let adjudication = apply_adjudication(&mut items, &input);

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
        adjudication,
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
            None,
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
            adjudicator: None,
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
            None,
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
            adjudicator: None,
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
            None,
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
            adjudicator: None,
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
            None,
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
            adjudicator: None,
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
            None,
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
            adjudicator: None,
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
            None,
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
            adjudicator: None,
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

    // ── A4：分歧裁决的模型通道 ─────────────────────────────────────────

    /// 三路分歧的固定场景：本地 `stencilling`、云端 `painting`、原文 `carving`。
    fn adjudication_fixture() -> (
        RecognitionCandidateV1,
        RecognitionCandidateV1,
        SourceVerificationV1,
        Value,
    ) {
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
        let source = verify_against_source(
            Some(&document(&["14 carving"])),
            &[(
                "slot-14".to_string(),
                14,
                Some(json!({"kind":"text","values":["stencilling"]})),
                true,
                String::new(),
            )],
            &[],
            None,
        );
        let canonical = canonical_with(Some(json!({"kind":"text","values":["stencilling"]})), None);
        (local, cloud, source, canonical)
    }

    fn answer_item(outcome: &AdjudicationOutcome) -> &DecisionItemV1 {
        outcome
            .decision
            .items
            .iter()
            .find(|item| item.field == DecisionFieldV1::Answer)
            .expect("答案层分歧必须产出一张建议卡")
    }

    /// A4 的价值：模型在三条链里挑一条 → 该项拿到**具体建议**与可复核的裁定留痕。
    ///
    /// 同时钉住一条硬约束：**模型裁定不解锁自动写入**。自动应用仍然只认确定性
    /// 证据（原文断言），否则本模块「自动应用不依赖模型置信度」的承诺就作废了。
    #[test]
    fn adjudication_ruling_attaches_a_recommendation_without_unlocking_auto_apply() {
        let (local, cloud, source, canonical) = adjudication_fixture();
        let validate = no_validation();
        let stub = |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
            Ok(json!({"rulings":[{
                "decisionId": "d:slot:slot-14:answer",
                "chosen": "cloud",
                "value": {"kind":"text","values":["painting"]},
                "confidence": 0.8,
                "rationale": "The attached page lists painting for question 14."
            }]}))
        };
        let adjudicator: AdjudicationRunner<'_> = &stub;
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
            adjudicator: Some(adjudicator),
        });

        assert_eq!(
            outcome.adjudication,
            StageStatusV1::new(StageStateV1::Succeeded),
            "全部项都拿到裁定 → 裁决链必须如实报成功"
        );
        let item = answer_item(&outcome);
        assert_eq!(
            item.proposed_patch.as_ref().map(|patch| patch["value"].clone()),
            Some(json!({"kind":"text","values":["painting"]})),
            "裁定选中的链值必须落成具体建议 patch"
        );
        assert!(
            item.user_message.contains("云端识别") && item.user_message.contains("理由"),
            "用户必须看到「模型建议采用哪一路、为什么」：{}",
            item.user_message
        );
        assert!(
            item.evidence
                .iter()
                .any(|evidence| evidence.anchor_kind == "adjudication_ruling"),
            "裁定必须留下可机器复核的留痕，而不是只写进 user_message"
        );
        // 关键不变量：模型裁定**不**进入自动应用资格。
        assert!(
            outcome.auto_apply_candidates.is_empty(),
            "模型裁定不得解锁自动写入（自动应用只认确定性证据）"
        );
        assert_ne!(item.resolution, DecisionResolutionV1::AutoFixed);
    }

    /// 预算耗尽：**没有尝试**过模型，绝不能记成「裁过了」。
    #[test]
    fn adjudication_budget_exhaustion_never_counts_as_ruled() {
        let (local, cloud, source, canonical) = adjudication_fixture();
        let validate = no_validation();
        let stub =
            |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
                Err(ModelCallFailure::BudgetExhausted)
            };
        let adjudicator: AdjudicationRunner<'_> = &stub;
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
            adjudicator: Some(adjudicator),
        });

        assert_eq!(outcome.adjudication.state, StageStateV1::NotRun);
        assert_eq!(
            outcome.adjudication.reason_code.as_deref(),
            Some(reason::ADJUDICATION_BUDGET_EXHAUSTED)
        );
        let item = answer_item(&outcome);
        assert_eq!(item.reason_code, reason::ADJUDICATION_BUDGET_EXHAUSTED);
        assert!(
            matches!(
                item.resolution,
                DecisionResolutionV1::NeedsReview | DecisionResolutionV1::Unverifiable
            ),
            "未经裁定的项必须留在待确认，实际为 {:?}",
            item.resolution
        );
        assert!(outcome.auto_apply_candidates.is_empty());
    }

    /// 模型明示无法裁定：同样是「没裁」，如实上报且不改写结论。
    #[test]
    fn adjudication_declined_keeps_the_item_in_review() {
        let (local, cloud, source, canonical) = adjudication_fixture();
        let validate = no_validation();
        let stub = |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
            Ok(json!({"rulings":[{
                "decisionId": "d:slot:slot-14:answer",
                "chosen": "unresolved",
                "rationale": "The attached page does not settle question 14."
            }]}))
        };
        let adjudicator: AdjudicationRunner<'_> = &stub;
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
            adjudicator: Some(adjudicator),
        });

        assert_eq!(outcome.adjudication.state, StageStateV1::Partial);
        assert_eq!(
            outcome.adjudication.reason_code.as_deref(),
            Some(reason::ADJUDICATION_DECLINED)
        );
        let item = answer_item(&outcome);
        assert_eq!(item.reason_code, reason::ADJUDICATION_DECLINED);
        assert_eq!(item.resolution, DecisionResolutionV1::NeedsReview);
    }

    /// **模型只能「选择」，不能「发明」。** 裁决值不在三条链上 → 不采纳为建议。
    #[test]
    fn adjudication_rejects_a_value_that_no_chain_gives() {
        let (local, cloud, source, canonical) = adjudication_fixture();
        let validate = no_validation();
        let stub = |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
            Ok(json!({"rulings":[{
                "decisionId": "d:slot:slot-14:answer",
                "chosen": "cloud",
                "value": {"kind":"text","values":["invented"]},
                "confidence": 0.99,
                "rationale": "Trust me."
            }]}))
        };
        let adjudicator: AdjudicationRunner<'_> = &stub;
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
            adjudicator: Some(adjudicator),
        });

        let item = answer_item(&outcome);
        assert_eq!(
            item.reason_code,
            reason::ADJUDICATION_VALUE_NOT_CORROBORATED
        );
        assert_eq!(
            item.proposed_patch.as_ref().map(|patch| patch["value"].clone()),
            Some(json!({"kind":"text","values":["carving"]})),
            "被拒的裁定不得改写建议；建议必须仍是确定性规则的结论"
        );
        assert!(outcome.auto_apply_candidates.is_empty());
    }

    /// 回归护栏：**不注入裁决通道时，决策项逐项保持确定性规则的结论。**
    ///
    /// 这是「A4 打开/关闭不改行为」的唯一保证。裁决链状态会如实变成
    /// `not_run`（那是修掉「硬编码 Succeeded」的必然结果），但**项本身不被触碰**：
    /// 一旦这里开始改写 `reason_code`，未配置模型的用户就会看到一片
    /// 「模型未参与裁决」，而真正的分歧原因（实质分歧 / 缺证据）反而消失了。
    #[test]
    fn missing_adjudicator_leaves_items_untouched() {
        let (local, cloud, source, canonical) = adjudication_fixture();
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
            adjudicator: None,
        });

        assert_eq!(outcome.adjudication.state, StageStateV1::NotRun);
        assert_eq!(
            outcome.adjudication.reason_code.as_deref(),
            Some(reason::ADJUDICATION_MODEL_UNAVAILABLE)
        );
        let item = answer_item(&outcome);
        assert!(
            !item.reason_code.starts_with("ADJUDICATION_"),
            "没有模型时不得给项盖上裁决类原因码，实际为 {}",
            item.reason_code
        );
    }

    /// 没有分歧就没有裁决需求：确定性裁决本身跑完了 → 如实报成功（不是 `not_run`）。
    #[test]
    fn adjudication_is_succeeded_when_there_is_nothing_to_decide() {
        let local = candidate(
            ChainKindV1::Local,
            vec![slot("slot-14", 14, Some(json!({"kind":"text","values":["stencilling"]})), true)],
            ChainStatusV1::Succeeded,
        );
        let cloud = candidate(
            ChainKindV1::Cloud,
            vec![slot("slot-14", 14, Some(json!({"kind":"text","values":["Stencilling"]})), true)],
            ChainStatusV1::Succeeded,
        );
        let source = verify_against_source(
            Some(&document(&["14 stencilling"])),
            &[(
                "slot-14".to_string(),
                14,
                Some(json!({"kind":"text","values":["stencilling"]})),
                true,
                String::new(),
            )],
            &[("task-1".to_string(), vec![14, 15])],
            None,
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
            adjudicator: None,
        });
        assert_eq!(outcome.adjudication, StageStatusV1::new(StageStateV1::Succeeded));
    }
}
