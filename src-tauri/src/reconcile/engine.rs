//! 识别闭环端到端编排：三路结果 → **一份**统一裁决 → 持久化。
//!
//! 这一层只做编排，不做判断：所有判定都在 [`super::rules`] 与
//! [`super::adjudicate`] 里，便于确定性测试。编排层负责的是任务书要求的
//! 「可恢复、有界、不回退」：
//!
//! - **本地先出稿**：本地候选直接从已有权威稿抽取，不等待云端。
//! - **云端失败必须分类**：超时 / 不支持输入 / 非法输出各有稳定原因码，
//!   绝不把「没验证」写成「已验证」（[`classify_cloud_error`]）。
//! - **有界**：模型调用上限 `MAX_ADJUDICATION_MODEL_CALLS`，受约束修复
//!   `MAX_CONSTRAINED_REPAIRS`；不足时输出 `Unverifiable` 而不是猜。
//! - **迟到不覆盖**：裁决项的 `status` 由应用层按版本检查写回，编排层不
//!   直接碰权威稿。

use std::path::Path;

use rusqlite::Connection;
use serde_json::Value;

use super::adjudicate::{adjudicate, AdjudicateInput, AdjudicationOutcome};
use super::candidate::{
    align_cloud_answer_shapes, cloud_candidate_from_value, local_candidate_from_authoring,
    not_run_candidate,
};
use super::source::{verify_against_source, SourceVerificationV1};
use super::store;
use crate::schema::recognition_v1::{
    reason, ChainKindV1, ChainStatusV1, RecognitionCandidateV1, RecognitionChainStateV1,
    RecognitionDecisionV1, StageStateV1, StageStatusV1,
};
use crate::CommandResult;

// ── 云端失败分类 ───────────────────────────────────────────────────────

/// 云端链失败的**分类**结果。失败必须说清「为什么没有验证」，
/// 否则前端只能显示一个无意义的错误。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CloudFailure {
    pub status: ChainStatusV1,
    /// 稳定原因码，取值来自 [`reason`]。
    pub reason_code: String,
    /// 供人阅读的解释（可含原始错误细节，不参与判定）。
    pub message: String,
}

impl CloudFailure {
    pub(crate) fn unusable(reason_code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: ChainStatusV1::Unusable,
            reason_code: reason_code.into(),
            message: message.into(),
        }
    }

    pub(crate) fn not_run(reason_code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: ChainStatusV1::NotRun,
            reason_code: reason_code.into(),
            message: message.into(),
        }
    }
}

/// 把网关/校验层的错误字符串归类成稳定原因码。
///
/// 顺序很重要：先判「不支持输入」，再判「超时」，最后才归到「非法输出」。
/// 归类失败时默认 `MODEL_INVALID_OUTPUT`（保守：宁可要求人工确认，
/// 也不要把解析失败当成「模型不支持」而跳过验证）。
pub(crate) fn classify_cloud_error(error: &str) -> CloudFailure {
    let lower = error.to_ascii_lowercase();
    // 凭据错误先判：它既不是「模型不支持」也不是「输出非法」，重试没有用，
    // 用户要去设置页修正密钥。
    let credentials = [
        "llm_http_401",
        "llm_http_403",
        "invalid_api_key",
        "credentials_invalid",
    ];
    if credentials.iter().any(|needle| lower.contains(needle)) {
        return CloudFailure::unusable(reason::MODEL_CREDENTIALS_INVALID, error);
    }
    let unsupported = [
        "unsupported",
        "not_supported",
        "does_not_support",
        "invalid_model",
        "model_not_found",
        "llm_profile_model_missing",
        "no_profile",
    ];
    if unsupported.iter().any(|needle| lower.contains(needle)) {
        return CloudFailure::not_run(reason::MODEL_UNSUPPORTED_INPUT, error);
    }
    let timeout = [
        "timeout",
        "timed out",
        "deadline",
        "etimedout",
        "operation timed",
    ];
    if timeout.iter().any(|needle| lower.contains(needle)) {
        // 超时是**不可用**而不是「未运行」：确实尝试过，只是没有拿到可用结果。
        return CloudFailure::unusable(reason::MODEL_TIMEOUT, error);
    }
    CloudFailure::unusable(reason::MODEL_INVALID_OUTPUT, error)
}

// ── 编排输入 / 输出 ────────────────────────────────────────────────────

/// A4：分歧裁决的模型通道。
///
/// 参数是本批**待裁定的分歧项**（`{decisionId, questionNumber, local, cloud, source}`），
/// 返回值是模型原始 JSON（`{"rulings": [...]}`）。与云端识别注入点同一约定：
/// **边界上是 JSON，解析与校验在网关侧完成**，判定逻辑只消费已验证的结构。
///
/// `None` = 没有可用模型：分歧项全部原样留在 `NeedsReview`，行为与未接入 A4 时逐字一致。
pub(crate) type AdjudicationRunner<'a> = &'a dyn Fn(&[Value]) -> Result<Value, ModelCallFailure>;

/// A3：原文件核验的模型通道。
///
/// 参数是本批**原文件无法确定性判定的槽位**（`{slotId, questionNumber, localValue}`），
/// 返回值是模型原始 JSON（`{"findings": [...]}`）。与另外两个注入点同一约定：
/// **边界上是 JSON，解析与校验在网关侧完成**，判定逻辑只消费已验证的结构。
///
/// 刻意**不把 `document_ir` 传进来**：模型要看的必须是**原文件本身**（PDF 附件或
/// 独立抽取的原文文本），而不是本地识别产物。若把 `document_ir` 递给 runner，
/// 就等于给了它一条「用本地识别结果当作原文证据」的捷径，而那条捷径正是
/// 「把没验证写成已验证」的入口。
///
/// `None` = 没有可用模型：只做确定性核验，结果与未接入 A3 时逐字一致。
pub(crate) type SourceVerifyRunner<'a> = &'a dyn Fn(&[Value]) -> Result<Value, ModelCallFailure>;

/// 模型调用失败的两类原因。A3（原文件核验）与 A4（分歧裁决）**共用同一套语义**。
///
/// 刻意用枚举而不是错误字符串：**「预算耗尽（没有尝试）」与「调用过但失败」是语义不同的
/// 两件事**。混成一个字符串后，迟早会有人写 `error.contains("timeout")` 来区分，而
/// `*_BUDGET_EXHAUSTED` 里恰好没有 timeout，于是预算耗尽被归类成「模型非法输出」
/// ——用户就会看到一条错的解释。
///
/// 两个通道各持一份枚举，是因为两者都可能独立失败：核验跑通而裁决被拒是完全正常的组合，
/// 合用一个枚举只会让上游分不清是谁失败的。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ModelCallFailure {
    /// 本次运行不再有预算（超过单批软上限，或调用次数已达上限）。
    BudgetExhausted,
    /// 真的调用了模型，但没有拿到可用结果（超时 / 非法输出 / 不支持输入）。
    /// 里面的字符串交给 [`classify_cloud_error`] 归类，**不另造一套分类**。
    Model(String),
}

pub(crate) struct ReconcileBatchInput<'a> {
    pub item_id: &'a str,
    pub job_id: &'a str,
    pub batch_id: &'a str,
    pub source_file_id: &'a str,
    pub source_sha256: &'a str,
    pub base_edit_version: i64,
    /// 当前权威稿（`IeltsAuthoringIRV2` JSON）。本地链与用户修改判定都基于它。
    pub canonical: &'a Value,
    /// 原文件语义（`DocumentIRV2` JSON）。云端核验与原文证据都基于它。
    pub document_ir: Option<&'a Value>,
    /// **识别当时的本地结果快照**。
    ///
    /// 关键：这里必须传「批次生成时冻结的本地候选」，而不是用当前权威稿
    /// 现场重投影。原因见 [`resolve_local_snapshot`]：只有快照与当前权威稿
    /// 的差异才能识别「云端运行期间用户改了稿」，迟到结果才不会覆盖用户修改。
    pub local_snapshot: Option<RecognitionCandidateV1>,
    /// 云端全量识别结果：原始 `CloudReadingOutlineV1` JSON，或分类后的失败。
    pub cloud: Result<Value, CloudFailure>,
    /// 结构校验闭包：把一批 patch 应用到权威稿副本并跑同一套校验。
    pub validate_batch: &'a dyn Fn(&[Value]) -> Result<(), String>,
    /// A3：原文件核验的模型通道。`None` = 无可用模型，只做确定性核验。
    pub source_verifier: Option<SourceVerifyRunner<'a>>,
    /// A4：分歧裁决的模型通道。`None` = 无可用模型，分歧全部留在 `NeedsReview`。
    pub adjudicator: Option<AdjudicationRunner<'a>>,
}

/// 取本地快照：优先复用批次已冻结的候选，否则按当前权威稿现场投影。
///
/// 复用条件是「同一 batch_id + 同一 base_edit_version」——batch_id 本身由
/// `(job_id, source_sha256, base_edit_version)` 派生（见
/// [`super::commands::recognition_batch_id`]），因此同一输入与版本的重试
/// 必然复用同一快照，既保证幂等，也让用户修改可被检出。
/// 冻结快照是否可复用：批次 id、基线版本、源文件哈希三者一致，且确为本地链。
///
/// 抽成独立判据，是为了让调用方（[`reconcile_batch`]）能直接问「这次裁决采用的本地
/// 基线到底是不是冻结快照」，而不必自行复刻一遍条件。**两处口径一旦分叉，「基线是否
/// 可信」的判断就会与实际采用的快照不一致**——那正是「冻结失败后仍然自动写入」的成因。
pub(crate) fn frozen_snapshot_is_reusable(
    stored: Option<&RecognitionCandidateV1>,
    batch_id: &str,
    base_edit_version: i64,
    source_sha256: &str,
) -> bool {
    let Some(stored) = stored else {
        return false;
    };
    stored.batch_id == batch_id
        && stored.base_edit_version == base_edit_version
        && stored.source_sha256 == source_sha256
        && stored.chain == ChainKindV1::Local
}

pub(crate) fn resolve_local_snapshot(
    stored: Option<RecognitionCandidateV1>,
    canonical: &Value,
    batch_id: &str,
    item_id: &str,
    job_id: &str,
    source_file_id: &str,
    source_sha256: &str,
    base_edit_version: i64,
) -> RecognitionCandidateV1 {
    if let Some(stored) = stored {
        if frozen_snapshot_is_reusable(Some(&stored), batch_id, base_edit_version, source_sha256) {
            return stored;
        }
    }
    local_candidate_from_authoring(
        canonical,
        batch_id,
        item_id,
        job_id,
        source_file_id,
        source_sha256,
        base_edit_version,
    )
}

pub(crate) struct ReconcileBatchOutcome {
    pub decision: RecognitionDecisionV1,
    /// 四阶段状态（本地 / 云端 / 核验 / 裁决），供前端解释。
    pub chains: RecognitionChainStateV1,
    /// 符合全部自动应用条件、且合并后结构合法的 patch（与 decision id 对齐）。
    pub auto_apply_candidates: Vec<String>,
    /// 本次裁决采用的本地基线是否**确实是冻结快照**（决定自动写入是否有前提保护）。
    ///
    /// 为 `false` 时 [`ReconcileBatchOutcome::auto_apply_candidates`] 必为空：没有可信基线
    /// 就不允许自动改题稿，见 [`frozen_snapshot_is_reusable`] 与
    /// [`crate::reconcile::adjudicate::refuse_auto_apply_without_frozen_baseline`]。
    pub local_baseline_frozen: bool,
    /// 证据留痕：三条链的原始结论。
    pub local: RecognitionCandidateV1,
    pub cloud: RecognitionCandidateV1,
    pub source: SourceVerificationV1,
}

/// 由本地候选推导链路状态：有槽位即成功；丢弃过题组即部分可用。
fn local_stage_status(local: &RecognitionCandidateV1) -> StageStatusV1 {
    if local.slots.is_empty() && local.task_groups.is_empty() {
        return StageStatusV1::with_reason(
            StageStateV1::Unusable,
            reason::EVIDENCE_MISSING,
            "本地稿没有可识别的题目结构。",
        );
    }
    if local.unresolved_regions.is_empty() {
        StageStatusV1::new(StageStateV1::Succeeded)
    } else {
        StageStatusV1::with_reason(
            StageStateV1::Partial,
            reason::SALVAGE_PARTIAL,
            "本地稿有部分区域未能识别，仍需人工确认。",
        )
    }
}

fn cloud_stage_status(
    cloud: &RecognitionCandidateV1,
    failure: Option<&CloudFailure>,
) -> StageStatusV1 {
    match failure {
        Some(failure) => StageStatusV1::with_reason(
            StageStateV1::from(failure.status),
            failure.reason_code.clone(),
            failure.message.clone(),
        ),
        None => {
            let state = StageStateV1::from(cloud.status);
            match &cloud.reason_code {
                Some(code) => StageStatusV1::with_reason(
                    state,
                    code.clone(),
                    "云端识别只有部分题组通过校验，其余需要人工确认。",
                ),
                None => StageStatusV1::new(state),
            }
        }
    }
}

/// 原文件核验链的状态。
///
/// 两种情形必须分开说：
/// - **模型通道失败**（真的调用了但没拿到可用结论）→ 如实上报 `Partial` + 该失败的原因码。
///   此时结论确实比预期弱，「确定性抽取的结论仍然可用」必须让用户看见。
/// - **模型通道没跑**（没配模型 / 没文本 / 没有待核验项）→ 与未接入 A3 时逐字一致，
///   **不因此改写原因码**：否则未配模型的用户会看到满屏「模型未参与核验」，
///   而真正的原因（原文没有可核验的文本证据）反而消失了。
fn source_stage_status(source: &SourceVerificationV1) -> StageStatusV1 {
    if let Some(code) = source.unusable_reason() {
        return StageStatusV1::with_reason(
            StageStateV1::Partial,
            code,
            "原文件核验的模型通道未给出可用结论，本批结论仅来自确定性抽取。",
        );
    }
    let state = StageStateV1::from(source.status);
    match &source.reason_code {
        Some(code) => StageStatusV1::with_reason(
            state,
            code.clone(),
            "原文件没有可核验的文本证据，结论只能标记为无法判断。",
        ),
        None => StageStatusV1::new(state),
    }
}

/// 三路 → 一份裁决。纯函数（无 IO），便于确定性测试。
pub(crate) fn reconcile_batch(input: ReconcileBatchInput<'_>) -> ReconcileBatchOutcome {
    // 1) 本地链：复用批次冻结快照；没有快照时按当前权威稿投影（首次识别）。
    //
    // 先判定基线是否可信，再取快照：自动写入的前提保护依赖于此（见下）。
    let baseline_frozen = frozen_snapshot_is_reusable(
        input.local_snapshot.as_ref(),
        input.batch_id,
        input.base_edit_version,
        input.source_sha256,
    );
    let local = resolve_local_snapshot(
        input.local_snapshot,
        input.canonical,
        input.batch_id,
        input.item_id,
        input.job_id,
        input.source_file_id,
        input.source_sha256,
        input.base_edit_version,
    );

    // 2) 云端链：成功 → 归一为候选；失败 → 带原因码的不可用候选。
    let (mut cloud, cloud_failure) = match &input.cloud {
        Ok(raw) => (
            cloud_candidate_from_value(
                raw,
                input.batch_id,
                input.item_id,
                input.job_id,
                input.source_file_id,
                input.source_sha256,
                input.base_edit_version,
            ),
            None,
        ),
        Err(failure) => (
            not_run_candidate(
                ChainKindV1::Cloud,
                input.batch_id,
                input.item_id,
                input.job_id,
                input.source_file_id,
                input.source_sha256,
                input.base_edit_version,
                &failure.reason_code,
            ),
            Some(failure.clone()),
        ),
    };

    // 2b) 形状对齐：云端答案若与本地同题答案「形状不同但值相同」（如本地 option / 云端
    // text），对齐为本地形状，避免 `answer_compare_key` 的 shape-sensitive 比较制造假分歧。
    // 只对齐形状、不改动值；本地无答案或云端值无法抽字符串时保持原样。
    align_cloud_answer_shapes(&mut cloud, &local);

    // 3) 原文件核验：只产出问题与修正建议，不产出第二份权威稿。
    let local_slots: Vec<(String, u32, Option<Value>, bool, String)> = local
        .slots
        .iter()
        .map(|slot| {
            (
                slot.slot_id.clone(),
                slot.question_number,
                slot.answer.clone(),
                slot.has_source_evidence,
                String::new(),
            )
        })
        .collect();
    let local_groups: Vec<(String, Vec<u32>)> = local
        .task_groups
        .iter()
        .map(|group| {
            (
                group.task_id.clone(),
                super::source::group_question_numbers(&group.display_range),
            )
        })
        .collect();
    let source = verify_against_source(
        input.document_ir,
        &local_slots,
        &local_groups,
        input.source_verifier,
    );

    // 4) 统一裁决：合并去重、依赖分组、自动应用资格、结构校验。
    let AdjudicationOutcome {
        decision,
        auto_apply_candidates,
        adjudication,
    } = adjudicate(AdjudicateInput {
        canonical: input.canonical,
        local: &local,
        cloud: &cloud,
        source: &source,
        batch_id: input.batch_id,
        item_id: input.item_id,
        job_id: input.job_id,
        base_edit_version: input.base_edit_version,
        validate_batch: input.validate_batch,
        adjudicator: input.adjudicator,
    });

    // 4b) 无可信基线 ⇒ 撤销自动应用资格。
    //
    // `auto_apply_eligible` 的守卫「权威稿当前值 == 本地识别结果」只在本地是**冻结快照**
    // 时成立。快照缺失时基线退化为「当前稿的现场重投影」，两边恒等、守卫等价于不存在，
    // 于是自动写入会覆盖用户的编辑。冻结失败是可诊断的异常（磁盘/读稿失败），
    // **不能因为「无云分支没有迟到的云端结果」就放行**——本地裁决自己也会自动写入。
    let mut decision = decision;
    let mut auto_apply_candidates = auto_apply_candidates;
    if !baseline_frozen && !auto_apply_candidates.is_empty() {
        super::adjudicate::refuse_auto_apply_without_frozen_baseline(
            &mut decision,
            &auto_apply_candidates,
        );
        auto_apply_candidates.clear();
    }

    let chains = RecognitionChainStateV1 {
        local: local_stage_status(&local),
        cloud: cloud_stage_status(&cloud, cloud_failure.as_ref()),
        source: source_stage_status(&source),
        // 裁决链状态**由裁决本身如实给出**。原先硬编码 `Succeeded`，等于无论模型有没有
        // 跑过都对 UI 说「裁决成功」——这是「把没跑的写成跑过了」，必须由裁决层回报。
        adjudication,
    };

    ReconcileBatchOutcome {
        decision,
        chains,
        auto_apply_candidates,
        local_baseline_frozen: baseline_frozen,
        local,
        cloud,
        source,
    }
}

/// 把三路证据与裁决写入 job 目录，并把批次汇总与逐项裁决写入数据库。
///
/// 数据库是前端读取权威；job 目录 JSON 只是证据留痕，被清理策略回收不
/// 影响产品决策。
pub(crate) fn persist_outcome(
    root: &Path,
    conn: &Connection,
    outcome: &ReconcileBatchOutcome,
) -> CommandResult<()> {
    let batch_id = outcome.decision.batch_id.clone();
    let job_id = outcome.decision.job_id.clone();
    store::write_candidate(root, &batch_id, &outcome.local, store::LOCAL_CANDIDATE_FILE)?;
    store::write_candidate(root, &batch_id, &outcome.cloud, store::CLOUD_CANDIDATE_FILE)?;
    store::write_source_verification(root, &job_id, &batch_id, &outcome.source)?;
    store::write_decision(root, &job_id, &batch_id, &outcome.decision)?;
    store::write_current_batch(root, &job_id, &batch_id)?;

    store::upsert_batch_with_stages(conn, &outcome.decision, Some(&outcome.chains))?;
    store::replace_decision_items(conn, &outcome.decision)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_input_is_not_run_but_timeout_is_unusable() {
        let unsupported = classify_cloud_error("model_not_supported_for_pdf_input");
        assert_eq!(unsupported.status, ChainStatusV1::NotRun);
        assert_eq!(unsupported.reason_code, reason::MODEL_UNSUPPORTED_INPUT);

        let timeout = classify_cloud_error("openai request timeout after 60s");
        assert_eq!(timeout.status, ChainStatusV1::Unusable);
        assert_eq!(timeout.reason_code, reason::MODEL_TIMEOUT);

        // 未知错误保守归为「非法输出」，并要求人工确认，而不是静默跳过。
        let unknown = classify_cloud_error("cloud_outline_direct_pdf_failed_and_no_images");
        assert_eq!(unknown.status, ChainStatusV1::Unusable);
        assert_eq!(unknown.reason_code, reason::MODEL_INVALID_OUTPUT);
    }

    #[test]
    fn missing_profile_is_reported_as_unsupported_not_silently_skipped() {
        let failure = classify_cloud_error("llm_profile_model_missing");
        assert_eq!(failure.status, ChainStatusV1::NotRun);
        assert_eq!(failure.reason_code, reason::MODEL_UNSUPPORTED_INPUT);
    }
}
