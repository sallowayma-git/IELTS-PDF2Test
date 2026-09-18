//! 识别闭环的命令层：调度入口 + 安全自动应用 + 人工决策。
//!
//! 三条硬约束在这里落地（对应任务书第四步）：
//!
//! 1. **自动应用靠规则不靠置信度**：可自动应用的项由 [`super::adjudicate`]
//!    用确定性条件筛出；本层在写入前再用**当前**权威稿复核一次
//!    （答案仍为空 + 与识别快照一致），因此「用户已修改」永远赢。
//! 2. **接受走正式 V2 patch/revision 路径**：一律经
//!    `library::repository::apply_editor_commands_tx_with`（CAS + 校验 + 原子提交），
//!    不绕过 V2 保护、不直接写 canonical。决策状态的写入作为该事务的**回调**执行，
//!    因而与题稿改动同生共死——「题稿已改、状态未写」的窗口在结构上不存在
//!    （否则崩溃后重试会被前提复核误判为「用户改过」而拒绝，重启也无法自愈）。
//! 3. **幂等与持久化**：`request_id` 落 `recognition_decision_journal_v1`，
//!    重复提交返回首次结果且不重复写入；过期项落 `Superseded`，失败落 `Failed`。

use std::path::Path;

use serde_json::{json, Value};

use super::engine::{
    classify_cloud_error, persist_outcome, reconcile_batch, resolve_local_snapshot,
    AdjudicationRunner, CloudFailure, ReconcileBatchInput, ReconcileBatchOutcome,
    SourceVerifyRunner,
};
use super::rules::{answer_compare_key, answer_is_empty, canonical_answer, is_user_edited};
use super::store;
use crate::authoring_v2_commands::{apply_patch, refresh_quality_report, validate_authoring};
use crate::library::repository::{
    apply_editor_commands_tx_with, get_canonical_ds, open_library_connection,
    ApplyEditorCommandsInput, EditOrigin,
};
use crate::schema::recognition_v1::{
    reason, ApplyRecognitionDecisionsRequestV1, ApplyRecognitionDecisionsResultV1, ChainStatusSummaryV1,
    DecisionFieldV1, DecisionItemV1, DecisionOutcomeKindV1, DecisionOutcomeV1, DecisionResolutionV1,
    DecisionStatusV1, RecognitionChainStateV1, RecognitionDecisionV1, RecognitionDecisionViewV1,
    StageStateV1, StageStatusV1, APPLY_RECOGNITION_DECISIONS_RESULT_V1_SCHEMA_VERSION,
    RECOGNITION_DECISION_VIEW_V1_SCHEMA_VERSION,
};
use crate::util::{job_dir, read_json_opt};
use crate::CommandResult;

/// 云端全量识别的注入点：真实实现走 LLM 网关；测试注入确定性桩。
pub(crate) type CloudOutlineRunner<'a> =
    &'a dyn Fn(&Path, &str, Option<&str>) -> CommandResult<Value>;

// ── 批次标识 ───────────────────────────────────────────────────────────

fn sanitize_segment(input: &str) -> String {
    input
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

/// 批次 id 由 `(job_id, source_sha256, base_edit_version)` 派生。
///
/// 这是「同一输入及版本的重试不产生重复应用」的根据：重试算出同一个
/// batch_id，于是复用同一份识别快照、同一批 decision_id，写入路径上的
/// 幂等键也相同，重复应用不可能发生。
pub(crate) fn recognition_batch_id(
    job_id: &str,
    source_sha256: &str,
    base_edit_version: i64,
) -> String {
    let sha = if source_sha256.is_empty() {
        "nosha".to_string()
    } else {
        source_sha256.chars().take(12).collect::<String>()
    };
    format!(
        "rec-{}-v{}-{}",
        sanitize_segment(job_id),
        base_edit_version,
        sanitize_segment(&sha)
    )
}

/// 取任务主源文件的 sha256（幂等键的输入之一）。取不到时返回空串，
/// 由 [`recognition_batch_id`] 降级为 `nosha`（仍然确定性）。
pub(crate) fn source_sha256_for_job(root: &Path, job_id: &str) -> String {
    let Ok(job) = crate::job_store::load_job(root, job_id) else {
        return String::new();
    };
    job.source_files
        .iter()
        .find(|file| file.role == "main")
        .or_else(|| job.source_files.first())
        .map(|file| file.sha256.clone())
        .unwrap_or_default()
}

// ── 读取视图 ───────────────────────────────────────────────────────────

fn chain_state_from_summary(summary: &ChainStatusSummaryV1) -> RecognitionChainStateV1 {
    RecognitionChainStateV1 {
        local: StageStatusV1::new(summary.local.into()),
        cloud: match &summary.cloud_reason_code {
            Some(code) => StageStatusV1::with_reason(summary.cloud.into(), code.clone(), String::new()),
            None => StageStatusV1::new(summary.cloud.into()),
        },
        source: match &summary.source_reason_code {
            Some(code) => StageStatusV1::with_reason(summary.source.into(), code.clone(), String::new()),
            None => StageStatusV1::new(summary.source.into()),
        },
        adjudication: StageStatusV1::new(StageStateV1::Succeeded),
    }
}

/// 建议的「前提」是否仍然成立：生成建议时所依据的**目标内容**有没有被改动。
///
/// - 答案字段：当前权威稿的答案必须仍等于建议生成时的本地值（`local_value`）；
/// - 其他字段：没有通用的「当前值读取器」，改看权威稿里的编辑痕迹——目标干净即视为
///   未改动。这正好覆盖「用户只改了别处」的情形，而不是一有版本变化就整批作废；
/// - 读不到权威稿：无法证明目标未被改动，保守返回 `false`（宁可要求人工确认，不写脏）。
fn resolution_premise_holds(canonical: Option<&Value>, item: &DecisionItemV1) -> bool {
    let Some(canonical) = canonical else {
        return false;
    };
    if item.field == DecisionFieldV1::Answer {
        return answer_compare_key(canonical_answer(canonical, &item.target.target_id))
            == answer_compare_key(item.local_value.as_ref());
    }
    let node_id = item
        .target
        .node_id
        .as_deref()
        .unwrap_or(&item.target.target_id);
    !is_user_edited(canonical, node_id) && !is_user_edited(canonical, &item.target.target_id)
}

/// 这条**已写入的修正**是否仍然生效：权威稿里该槽位的答案是否还等于当时写入的值。
///
/// 这是「内容变化后 resolution 失效」的判据。写入值取自 `proposed_patch` 的
/// `setAnswer.value`——**不能用 `undo` 里的 `value`**，那是被覆盖掉的旧值。返回：
/// - `Some(true)`：修正仍在位；
/// - `Some(false)`：用户之后改过该槽位 ⇒ 原 resolution 的前提已经消失（撤销必须拒绝，
///   否则回滚会连同用户的新改动一起覆盖；视图也不得再把它示为「已修正」）；
/// - `None`：无法判定（非 `setAnswer` 补丁 / 缺值 / 读不到权威稿）。调用方自行取舍：
///   撤销取保守（拒绝），展示取保守（保持原状，不凭空宣称失效）。
fn applied_answer_still_in_place(canonical: Option<&Value>, item: &DecisionItemV1) -> Option<bool> {
    let patch = item.proposed_patch.as_ref()?;
    if patch.get("op").and_then(Value::as_str) != Some("setAnswer") {
        return None;
    }
    let slot_id = patch.get("slotId").and_then(Value::as_str)?;
    let applied = patch.get("value")?;
    let canonical = canonical?;
    Some(
        answer_compare_key(canonical_answer(canonical, slot_id))
            == answer_compare_key(Some(applied)),
    )
}

/// 读取视图。除过滤待办外，还负责「**内容变化后的 resolution 失效**」的呈现。
///
/// 规则：
/// - 已写入的修正（`status == Accepted`，含 `AutoFixed` 与手动接受）：若
///   `applied_answer_still_in_place` 为 `Some(false)`（目标此后又被用户改动）⇒ 该
///   resolution **失效**，改以 `Superseded`（`USER_EDITED_AFTER_APPLY`）呈现；
/// - **只对 `Accepted` 生效**：`Open` 项本来就应该与建议值不同（尚未写入），若一并判定
///   会把所有待确认项误标为过期；
/// - `Rejected`：拒绝的语义就是「保持原样」，用户在别处的改动不会使该决定失效，故不随
///   内容变化失效——这也是「不因一次改稿就把所有决定作废」；
/// - `Undone` / `Superseded` / `Open` / `Failed`：不参与该规则。
///
/// 这是**呈现层**判定，不写库；持久化由 `apply_recognition_decisions_core` 随下一次
/// 应用收敛（读命令不做写入）。
fn build_view(
    batch: &store::BatchRow,
    items: Vec<DecisionItemV1>,
    current_edit_version: i64,
    canonical: Option<&Value>,
) -> RecognitionDecisionViewV1 {
    let items: Vec<DecisionItemV1> = items
        .into_iter()
        .map(|mut item| {
            if item.status == DecisionStatusV1::Accepted
                && applied_answer_still_in_place(canonical, &item) == Some(false)
            {
                item.status = DecisionStatusV1::Superseded;
                item.reason_code = reason::USER_EDITED_AFTER_APPLY.to_string();
            }
            item
        })
        .collect();
    let mut actionable = Vec::new();
    let mut auto_applied = Vec::new();
    for item in items {
        // 「仍生效的自动修正」：已写入权威稿**且仍然在位**。
        //
        // 两个条件都不可少：
        // - `resolution == AutoFixed` 必须有 `status == Accepted` 陪跑。仅看 `resolution`
        //   会把 `Undone`（用户已撤销）与 `Superseded`（目标此后被用户改动）也收进来——
        //   撤销只翻 `status`，resolutions 不变。那两种状态下权威稿里**已经不存在**这条
        //   修正了，继续以「已自动修正」展示会凭空多出一个事实，甚至给出撤销入口。
        //   这正是「撤销闭环」与「内容变化后失效」两条规则在呈现层的落点。
        if item.resolution == DecisionResolutionV1::AutoFixed
            && item.status == DecisionStatusV1::Accepted
        {
            auto_applied.push(item);
            continue;
        }
        // 待办语义只由 `DecisionItemV1::is_actionable` 定义（唯一判据）：`Open`
        // （可人工确认）与 `Failed`（自动应用失败、权威稿其实没被修正）都是待办；
        // `Accepted`/`Rejected`/`Superseded`/`Undone` 都不是。调度器的
        // `actionableCount` 用同一函数，两处口径不会分叉。
        if !item.is_actionable() {
            continue;
        }
        actionable.push(item);
    }
    RecognitionDecisionViewV1 {
        schema_version: RECOGNITION_DECISION_VIEW_V1_SCHEMA_VERSION.to_string(),
        item_id: batch.library_item_id.clone(),
        job_id: batch.job_id.clone(),
        batch_id: batch.batch_id.clone(),
        base_edit_version: batch.base_edit_version,
        edit_version: current_edit_version,
        stale: batch.base_edit_version < current_edit_version,
        generated_at: batch.updated_at.clone(),
        chains: batch
            .chain_state
            .clone()
            .unwrap_or_else(|| chain_state_from_summary(&batch.chain_status)),
        summary: batch.summary.clone(),
        actionable,
        auto_applied,
        // 修复摘要随批次一起读出。**没有就如实为空**，前端按「未进行云端修复」降级；
        // 绝不在缺失时编一个 completed 出来。
        repair: batch.repair.clone(),
    }
}

/// `get_recognition_decision` 的实现：只读数据库。
pub(crate) fn get_recognition_decision_core(
    root: &Path,
    item_id: &str,
) -> CommandResult<Value> {
    let conn = open_library_connection(root)?;
    let Some(batch) = store::load_latest_batch(&conn, item_id)? else {
        // 尚未产生批次不是错误：如实返回空视图，前端显示「识别中/未开始」。
        return Ok(json!({
            "schemaVersion": RECOGNITION_DECISION_VIEW_V1_SCHEMA_VERSION,
            "itemId": item_id,
            "jobId": Value::Null,
            "batchId": Value::Null,
            "baseEditVersion": 0,
            "editVersion": 0,
            "stale": false,
            "generatedAt": Value::Null,
            "chains": {
                "local": {"state": StageStateV1::NotRun.as_str()},
                "cloud": {"state": StageStateV1::NotRun.as_str()},
                "source": {"state": StageStateV1::NotRun.as_str()},
                "adjudication": {"state": StageStateV1::NotRun.as_str()}
            },
            "summary": {"agreed": 0, "autoFixed": 0, "needsReview": 0, "unverifiable": 0},
            "actionable": [],
            "autoApplied": []
        }));    };
    let items = store::load_decision_items(&conn, &batch.batch_id)?;
    let current = store::current_edit_version(&conn, item_id)?.unwrap_or(batch.base_edit_version);
    // 权威稿用于判定「已写入的修正是否仍生效」。读不到时传 `None`：判定取保守
    // （保持原状），绝不凭空宣称某条修正已失效。
    let canonical = get_canonical_ds(&conn, item_id)?.map(|(ds, _)| ds);
    let view = build_view(&batch, items, current, canonical.as_ref());
    serde_json::to_value(view).map_err(|error| error.to_string())
}

// ── 完整识别周期（本地已有稿 → 云端 → 核验 → 裁决 → 自动应用）──────────

/// 自动应用条件在**写入前**用当前权威稿复核。
///
/// 单靠裁决阶段的条件不够：裁决与写入之间用户可能已经改过稿。这里要求
/// 目标槽位「仍然是空的」且「与识别快照一致」，两个条件都满足才算未被修改。
fn still_auto_applicable(
    canonical: &Value,
    snapshot_answer: Option<&Value>,
    slot_id: &str,
    proposed: &Value,
) -> bool {
    let current = canonical_answer(canonical, slot_id);
    if !answer_is_empty(current) {
        return false;
    }
    if answer_compare_key(current) != answer_compare_key(snapshot_answer) {
        return false;
    }
    // 建议值本身必须仍然非空，否则是无效 patch。
    !answer_is_empty(Some(proposed))
}

fn proposed_answer(item: &DecisionItemV1) -> Option<&Value> {
    item.proposed_patch.as_ref()?.get("value")
}

/// 把一批 patch 经正式 V2 路径原子写入；返回 `(applied_ids, failures)`。
///
/// 版本冲突时**重新读取**权威稿逐项复核，只有仍然满足自动应用条件的项才
/// 用新版本重试一次。有界：最多两轮，不做无限重试。
#[allow(clippy::type_complexity)]
fn apply_patches_with_recheck(
    root: &Path,
    item_id: &str,
    base_edit_version: i64,
    decision: &RecognitionDecisionV1,
    candidates: &[String],
    snapshot_answers: &std::collections::BTreeMap<String, Option<Value>>,
    request_id: &str,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut eligible: Vec<String> = candidates.to_vec();
    let mut attempt_base = base_edit_version;
    let mut last_error = String::new();

    for _attempt in 0..2 {
        if eligible.is_empty() {
            break;
        }
        let commands: Vec<Value> = eligible
            .iter()
            .filter_map(|id| decision.item(id))
            .filter_map(|item| item.proposed_patch.clone())
            .collect();
        if commands.is_empty() {
            break;
        }
        let Ok(mut conn) = open_library_connection(root) else {
            return (Vec::new(), eligible.into_iter().map(|id| (id, "recognition_db_open_failed".to_string())).collect());
        };
        // 来源 = `CloudRepair` 且**不带** repair_run_id：规则自动填空不是人工修改，
        // 不得写进人工保护目标（否则云端修复就再也改不动本地规则的错误结论）；
        // 同时它又不属于任何一轮云端修复，因此不进入撤销 / 保护校验的范围。
        let result = apply_editor_commands_tx_with(
            &mut conn,
            &ApplyEditorCommandsInput {
                item_id: item_id.to_string(),
                base_version: attempt_base,
                request_id: Some(request_id.to_string()),
                commands,
                title: None,
            },
            EditOrigin::CloudRepair,
            None,
            &apply_patch,
            &|ds| {
                refresh_quality_report(root, item_id, ds)?;
                validate_authoring(ds)
            },
            &|_, _| Ok(()),
        );
        match result {
            Ok(_) => return (eligible, Vec::new()),
            Err(error) if error.starts_with("EDIT_VERSION_CONFLICT") => {
                last_error = error;
                let Ok(fresh) = open_library_connection(root)
                    .and_then(|conn| get_canonical_ds(&conn, item_id))
                else {
                    break;
                };
                let Some((canonical, current)) = fresh else { break };
                if current == attempt_base {
                    break;
                }
                // 只用「目标确实还是空的且未被改动」的项重试。
                eligible.retain(|id| {
                    let Some(item) = decision.item(id) else { return false };
                    let Some(proposed) = proposed_answer(item) else { return false };
                    still_auto_applicable(
                        &canonical,
                        snapshot_answers.get(id).and_then(|value| value.as_ref()),
                        &item.target.target_id,
                        proposed,
                    )
                });
                attempt_base = current;
            }
            Err(error) => {
                last_error = error;
                break;
            }
        }
    }

    let code = if last_error.starts_with("EDIT_VERSION_CONFLICT") {
        reason::USER_EDITED.to_string()
    } else if last_error.is_empty() {
        "recognition_apply_failed".to_string()
    } else {
        last_error.chars().take(80).collect()
    };
    (
        Vec::new(),
        eligible.into_iter().map(|id| (id, code.clone())).collect(),
    )
}

/// 完整识别周期：云端全量识别 → 原文件核验 → 统一裁决 → 安全自动应用。
///
/// 幂等保证：
/// - `batch_id` 由输入与基线版本派生，重试复用同一批次与同一份快照。
/// - 自动应用使用固定 `request_id = recognition-auto:<batch_id>`，重放不重复写入。
/// - 版本冲突时逐项复核，用户修改永远优先。
///
/// 本入口**不带**裁决注入点：等价于「本次运行没有裁决模型」，分歧项原样留给人工。
/// 真实模型通道见 [`run_recognition_cycle_core_with_adjudicator`]。
pub(crate) fn run_recognition_cycle_core(
    root: &Path,
    job_id: &str,
    profile_id: Option<&str>,
    cloud_enabled: bool,
    base_edit_version: i64,
    cloud_runner: CloudOutlineRunner<'_>,
) -> CommandResult<Value> {
    run_recognition_cycle_core_with_adjudicator(
        root,
        job_id,
        profile_id,
        cloud_enabled,
        base_edit_version,
        cloud_runner,
        None,
    )
}

/// 同上，但显式注入 A4 的**分歧裁决通道**。A3 的核验通道仍为 `None`。
///
/// `adjudicator = None` 时**不碰任何决策项**：确定性规则的逐项结论完整保留，
/// 只在裁决链状态上如实写「本次没有模型参与裁决」。
pub(crate) fn run_recognition_cycle_core_with_adjudicator(
    root: &Path,
    job_id: &str,
    profile_id: Option<&str>,
    cloud_enabled: bool,
    base_edit_version: i64,
    cloud_runner: CloudOutlineRunner<'_>,
    adjudicator: Option<AdjudicationRunner<'_>>,
) -> CommandResult<Value> {
    run_recognition_cycle_core_with_channels(
        root,
        job_id,
        profile_id,
        cloud_enabled,
        base_edit_version,
        cloud_runner,
        None,
        adjudicator,
    )
}

/// 同上，但把**两个模型通道**都显式注入：A3 的原文件核验、A4 的分歧裁决。
///
/// 为什么两个通道都是参数而不是在这里就地构造：
/// 1. 与 `cloud_runner` 同一约定——IO 在边界注入，判定层（`reconcile_batch` /
///    `adjudicate` / `verify_against_source`）因此保持可确定性测试；
/// 2. 「谁来提供模型、预算多少」是运行时决策（profile 是否存在、是否启用云端），
///    不该写死在核心里。调用方（调度器）给真实的、带预算的闭包；测试给桩或不给。
///
/// 两个通道**各自持预算、各自可能失败**：核验先跑（它的结论会成为裁决的输入之一），
/// 裁决后跑。任一为 `None` 时该通道完全缺席，其行为与未接入该能力时逐字一致。
pub(crate) fn run_recognition_cycle_core_with_channels(
    root: &Path,
    job_id: &str,
    profile_id: Option<&str>,
    cloud_enabled: bool,
    base_edit_version: i64,
    cloud_runner: CloudOutlineRunner<'_>,
    source_verifier: Option<SourceVerifyRunner<'_>>,
    adjudicator: Option<AdjudicationRunner<'_>>,
) -> CommandResult<Value> {
    let (canonical, current_version) = {
        let conn = open_library_connection(root)?;
        get_canonical_ds(&conn, job_id)?.ok_or_else(|| format!("ITEM_DS_NOT_SEEDED:{job_id}"))?
    };
    let source_sha256 = source_sha256_for_job(root, job_id);
    let batch_id = recognition_batch_id(job_id, &source_sha256, base_edit_version);
    let document_ir = read_json_opt(&job_dir(root, job_id).join("document-ir.json"))?;

    // 复用批次冻结快照（重试幂等），否则按当前稿投影。
    let stored_local = store::read_candidate(root, job_id, &batch_id, store::LOCAL_CANDIDATE_FILE);
    let local_snapshot = resolve_local_snapshot(
        stored_local,
        &canonical,
        &batch_id,
        job_id,
        job_id,
        &job_id.to_string(),
        &source_sha256,
        base_edit_version,
    );

    // 云端链：真实调用或分类失败（未配置 / 不支持输入 / 超时 / 非法输出）。
    let cloud: Result<Value, CloudFailure> = if !cloud_enabled {
        Err(CloudFailure::not_run(
            reason::CLOUD_DISABLED,
            "本次导入未启用云端识别。",
        ))
    } else if profile_id.is_none() {
        Err(CloudFailure::not_run(
            reason::NO_PROFILE,
            "没有可用的云端识别配置。",
        ))
    } else {
        cloud_runner(root, job_id, profile_id).map_err(|error| classify_cloud_error(&error))
    };

    let validate_batch = |patches: &[Value]| -> Result<(), String> {
        let mut probe = canonical.clone();
        for patch in patches {
            apply_patch(&mut probe, patch)?;
        }
        refresh_quality_report(root, job_id, &mut probe)?;
        validate_authoring(&probe)
    };

    let mut outcome: ReconcileBatchOutcome = reconcile_batch(ReconcileBatchInput {
        item_id: job_id,
        job_id,
        batch_id: &batch_id,
        source_file_id: job_id,
        source_sha256: &source_sha256,
        base_edit_version,
        canonical: &canonical,
        document_ir: document_ir.as_ref(),
        local_snapshot: Some(local_snapshot),
        cloud,
        validate_batch: &validate_batch,
        source_verifier,
        adjudicator,
    });

    // 自动应用前的快照答案表：写入前复核用。
    let snapshot_answers: std::collections::BTreeMap<String, Option<Value>> = outcome
        .decision
        .items
        .iter()
        .map(|item| {
            (
                item.decision_id.clone(),
                outcome
                    .local
                    .slot(&item.target.target_id)
                    .and_then(|slot| slot.answer.clone()),
            )
        })
        .collect();

    let auto_candidates = outcome.auto_apply_candidates.clone();
    let auto_request_id = format!("recognition-auto:{batch_id}");
    let (applied, failures) = if auto_candidates.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        apply_patches_with_recheck(
            root,
            job_id,
            current_version,
            &outcome.decision,
            &auto_candidates,
            &snapshot_answers,
            &auto_request_id,
        )
    };

    let now = chrono::Utc::now().to_rfc3339();
    if !applied.is_empty() {
        let undo_by_id: std::collections::BTreeMap<String, Value> = outcome
            .decision
            .items
            .iter()
            .filter(|item| applied.contains(&item.decision_id))
            .filter_map(|item| {
                super::adjudicate::undo_patch_for(item).map(|undo| (item.decision_id.clone(), undo))
            })
            .collect();
        super::adjudicate::mark_auto_applied(&mut outcome.decision, &applied, &undo_by_id, &now);
    }
    if !failures.is_empty() {
        super::adjudicate::mark_auto_apply_failed(&mut outcome.decision, &failures);
    }

    persist_outcome(root, &open_library_connection(root)?, &outcome)?;

    let final_version =
        store::current_edit_version(&open_library_connection(root)?, job_id)?.unwrap_or(current_version);
    // 待办数与视图同源：都用 `DecisionItemV1::is_actionable`。**不能**再用
    // `summary.needsReview + summary.unverifiable` 代替——那是 resolution 轴的分解，
    // 既漏掉 `Failed`（不可忽略的硬失败），又把「已自动修正」的项算进去过。
    let actionable_count = outcome
        .decision
        .items
        .iter()
        .filter(|item| item.is_actionable())
        .count() as i64;
    Ok(json!({
        "schemaVersion": "RecognitionCycleReportV1",
        "batchId": outcome.decision.batch_id,
        "itemId": job_id,
        "baseEditVersion": base_edit_version,
        "editVersion": final_version,
        "chains": outcome.chains,
        "summary": outcome.decision.summary,
        "actionableCount": actionable_count,
        "autoApplied": applied,
        "autoApplyFailed": failures.iter().map(|(id, code)| json!({"decisionId": id, "reasonCode": code})).collect::<Vec<_>>(),
        "localCandidateStatus": outcome.local.status,
        "cloudCandidateStatus": outcome.cloud.status,
        // 本地基线是否可信。为 false 时 `autoApply` 必为空——前端据此解释「为什么这次
        // 没有自动修正」，而不是把它当成「没有可修正项」。
        "localBaselineFrozen": outcome.local_baseline_frozen
    }))
}

// ── 人工决策（接受 / 拒绝）──────────────────────────────────────────────

/// `apply_recognition_decisions` 的实现。
pub(crate) fn apply_recognition_decisions_core(
    root: &Path,
    request: ApplyRecognitionDecisionsRequestV1,
) -> CommandResult<Value> {
    // 结构校验必须最先执行，且早于 journal：语义为空的请求一旦被记进 journal，
    // 就会被幂等路径当成「已处理」原样重放，错误从此固化。
    request.validate()?;
    let request = request.normalized();

    // 两条连接：`conn` 只读（幂等查询、批次/决策项、权威稿）；`conn_tx` 承载编辑事务，
    // 决策状态也经它写入（因而与题稿同事务）。见 `apply_editor_commands_tx_with`。
    let conn = open_library_connection(root)?;
    let payload = serde_json::to_string(&request).map_err(|error| error.to_string())?;

    // 幂等：同一 request_id 重复提交只生效一次。请求体不同则报错（复用 id 是 bug）。
    if let Some((item_id, batch_id, base_version, stored_payload, result_json)) =
        store::journal_lookup(&conn, &request.request_id)?
    {
        let _ = (&item_id, &batch_id);
        if base_version != request.base_edit_version || stored_payload != payload {
            return Err("RECOGNITION_REQUEST_ID_REUSED".to_string());
        }
        let mut result: ApplyRecognitionDecisionsResultV1 =
            serde_json::from_str(&result_json).map_err(|error| error.to_string())?;
        result.replayed = true;
        return serde_json::to_value(result).map_err(|error| error.to_string());
    }

    let Some(batch) = store::load_batch_by_id(&conn, &request.batch_id)? else {
        return Err(format!("RECOGNITION_BATCH_NOT_FOUND:{}", request.batch_id));
    };

    let item_id = batch.library_item_id.clone();
    let mut items = store::load_decision_items(&conn, &batch.batch_id)?;
    // `current_edit_version` 返回 `Ok(None)` 仅当 item 行不存在；查询失败经 `?` 直接上抛，
    // **不会**被折成版本号。这里回退到批次基线而非 0：0 是一个合法的真实版本，
    // 用它顶替「读不到」会让批次 id 与冻结快照对不上（对比 `scheduler.rs` 中已修的同名陷阱）。
    let before = store::current_edit_version(&conn, &item_id)?.unwrap_or(batch.base_edit_version);
    // 权威稿在本次调用开头读一次，供**撤销前提复核**与**接受前提复核**共用。
    // 其间只有 `reject` 会写库，而它只动 `recognition_decisions_v1`、不碰权威稿，
    // 所以这份读在整段流程内始终有效。读命令不写库。
    let canonical = get_canonical_ds(&conn, &item_id)?;
    let canonical_value = canonical.as_ref().map(|(value, _)| value.clone());

    let mut outcomes: Vec<DecisionOutcomeV1> = Vec::new();
    let now = chrono::Utc::now().to_rfc3339();

    // ── 拒绝：只改状态，不碰权威稿 ──────────────────────────────────
    for decision_id in &request.reject {
        let Some(index) = items.iter().position(|item| &item.decision_id == decision_id) else {
            outcomes.push(DecisionOutcomeV1 {
                decision_id: decision_id.clone(),
                kind: DecisionOutcomeKindV1::Failed,
                reason_code: Some("RECOGNITION_DECISION_NOT_FOUND".to_string()),
                message: "该建议已不存在，可能已被新批次替换。".to_string(),
                applied_at: None,
                undo: None,
            });
            continue;
        };
        items[index].status = DecisionStatusV1::Rejected;
        store::set_decision_status(
            &conn,
            &batch.batch_id,
            decision_id,
            DecisionStatusV1::Rejected.as_str(),
            &serde_json::to_string(&items[index]).map_err(|error| error.to_string())?,
            None,
        )?;
        outcomes.push(DecisionOutcomeV1 {
            decision_id: decision_id.clone(),
            kind: DecisionOutcomeKindV1::Rejected,
            reason_code: None,
            message: "已拒绝该建议，权威稿未改动。".to_string(),
            applied_at: None,
            undo: None,
        });
    }

    // ── 撤销：回滚权威稿 + 持久化「已撤销」状态 ─────────────────────
    // 收集待回滚项与逆 patch；实际写入复用下方已打开的 `conn_tx`，与接受写入
    // 走同一条 V2 patch 路径。幂等由外层 `request_id` 日志保证（见文件尾部的
    // journal_insert / 重放短路）。
    let mut undo_indices: Vec<usize> = Vec::new();
    let mut undo_commands: Vec<Value> = Vec::new();
    for decision_id in &request.undo {
        let Some(index) = items.iter().position(|item| &item.decision_id == decision_id) else {
            outcomes.push(DecisionOutcomeV1 {
                decision_id: decision_id.clone(),
                kind: DecisionOutcomeKindV1::Failed,
                reason_code: Some("RECOGNITION_DECISION_NOT_FOUND".to_string()),
                message: "该建议已不存在，可能已被新批次替换。".to_string(),
                applied_at: None,
                undo: None,
            });
            continue;
        };
        let item = &items[index];
        // 已撤销：重放或重复提交，原样跳过，不产生第二次回滚。
        if item.status == DecisionStatusV1::Undone {
            outcomes.push(DecisionOutcomeV1 {
                decision_id: decision_id.clone(),
                kind: DecisionOutcomeKindV1::Superseded,
                reason_code: Some("RECOGNITION_ALREADY_RESOLVED".to_string()),
                message: "该建议已撤销，本次不再重复回滚。".to_string(),
                applied_at: item.applied_at.clone(),
                undo: None,
            });
            continue;
        }
        // 只有「已自动修正」的项才可被撤销（其他状态没有可回滚的权威稿改动）。
        if item.status != DecisionStatusV1::Accepted || item.resolution != DecisionResolutionV1::AutoFixed {
            outcomes.push(DecisionOutcomeV1 {
                decision_id: decision_id.clone(),
                kind: DecisionOutcomeKindV1::Failed,
                reason_code: Some("RECOGNITION_NOT_UNDOABLE".to_string()),
                message: "该建议当前状态不可撤销（未被自动修正或未处于已修正状态）。".to_string(),
                applied_at: None,
                undo: None,
            });
            continue;
        }
        let Some(undo) = item.undo.clone() else {
            outcomes.push(DecisionOutcomeV1 {
                decision_id: decision_id.clone(),
                kind: DecisionOutcomeKindV1::Failed,
                reason_code: Some("RECOGNITION_NO_UNDO_PATCH".to_string()),
                message: "该自动修正没有可回滚的撤销补丁，请在题面上手动改回原值。".to_string(),
                applied_at: None,
                undo: None,
            });
            continue;
        };
        // 前提复核：当时写入的值必须**仍在权威稿里**。用户若在自动修正之后又改过该
        // 槽位（`Some(false)`），回滚会把用户的新改动一并覆盖——必须拒绝。
        //
        // 视图侧已把这类项呈现为 `Superseded` + `USER_EDITED_AFTER_APPLY`，但**呈现不等于
        // 强制**：数据库里它仍是 `Accepted`，若只靠前端不显示撤销入口，一个直接调用命令的
        // 客户端就能绕过。执行层必须自己守住这条边界。
        // 判定不出（`None`：非 setAnswer 补丁 / 缺值 / 读不到权威稿）时取保守——但保守的
        // 方向是「允许撤销」还是「拒绝」？此处选**放行**：`None` 多因补丁不是答案写入，
        // 那不是「用户改过」的证据，拒绝会平白阻断正常的撤销。
        if applied_answer_still_in_place(canonical_value.as_ref(), item) == Some(false) {
            outcomes.push(DecisionOutcomeV1 {
                decision_id: decision_id.clone(),
                kind: DecisionOutcomeKindV1::Failed,
                reason_code: Some(reason::USER_EDITED_AFTER_APPLY.to_string()),
                message: "该槽位在自动修正之后已被修改，撤销会覆盖你的改动，已拒绝。".to_string(),
                applied_at: item.applied_at.clone(),
                undo: None,
            });
            continue;
        }
        undo_commands.push(undo);
        undo_indices.push(index);
    }

    // ── 接受：过版本检查 → 原子写入 ─────────────────────────────────
    let mut accepted_indices: Vec<usize> = Vec::new();
    for decision_id in &request.accept {
        let Some(index) = items.iter().position(|item| &item.decision_id == decision_id) else {
            outcomes.push(DecisionOutcomeV1 {
                decision_id: decision_id.clone(),
                kind: DecisionOutcomeKindV1::Failed,
                reason_code: Some("RECOGNITION_DECISION_NOT_FOUND".to_string()),
                message: "该建议已不存在，可能已被新批次替换。".to_string(),
                applied_at: None,
                undo: None,
            });
            continue;
        };
        let item = &items[index];
        if item.status != DecisionStatusV1::Open {
            outcomes.push(DecisionOutcomeV1 {
                decision_id: decision_id.clone(),
                kind: DecisionOutcomeKindV1::Superseded,
                reason_code: Some("RECOGNITION_ALREADY_RESOLVED".to_string()),
                message: "该建议已处理过，本次不再重复写入。".to_string(),
                applied_at: item.applied_at.clone(),
                undo: None,
            });
            continue;
        }
        // 过期检查：批次基线早于当前版本时，逐项确认目标未被用户改动。
        if item.resolution == DecisionResolutionV1::Unverifiable {
            outcomes.push(DecisionOutcomeV1 {
                decision_id: decision_id.clone(),
                kind: DecisionOutcomeKindV1::Failed,
                reason_code: Some(reason::EVIDENCE_MISSING.to_string()),
                message: "证据不足，无法判断；请人工修改题稿而不是套用建议。".to_string(),
                applied_at: None,
                undo: None,
            });
            continue;
        }
        if item.proposed_patch.is_none() {
            outcomes.push(DecisionOutcomeV1 {
                decision_id: decision_id.clone(),
                kind: DecisionOutcomeKindV1::Superseded,
                reason_code: Some(reason::AUTO_APPLY_RULE_REJECTED.to_string()),
                message: "该建议不含可应用的修正补丁，请在编辑器中手动修改。".to_string(),
                applied_at: None,
                undo: None,
            });
            continue;
        }
        accepted_indices.push(index);
    }

    let mut conn_tx = open_library_connection(root)?;
    let mut applied_indices: Vec<usize> = Vec::new();
    let mut superseded: Vec<(usize, String)> = Vec::new();

    if !accepted_indices.is_empty() {
        // 逐项版本复核：建议的前提是否仍然成立（目标内容相对生成建议时未被改动）。
        // 判定集中在 `resolution_premise_holds`，此处不再内联复刻，避免两处口径分叉。
        let mut runnable: Vec<usize> = Vec::new();
        for index in &accepted_indices {
            let item = &items[*index];
            if resolution_premise_holds(canonical_value.as_ref(), item) {
                runnable.push(*index);
            } else {
                superseded.push((*index, reason::USER_EDITED.to_string()));
            }
        }
        let commands: Vec<Value> = runnable
            .iter()
            .filter_map(|index| items[*index].proposed_patch.clone())
            .collect();
        if !commands.is_empty() {
            // 待落库的项**先算好**（含撤销补丁）。事务回调必须能当 `Fn` 用，因此不能在回调里
            // 改 `items`；内存副本也要等写入成功后再更新——反过来的话，「写失败」时会出
            // 内存说已接受、数据库说仍待办，返回给前端的 view 与 outcomes 自相矛盾。
            let accept_writes: Vec<(usize, DecisionItemV1)> = runnable
                .iter()
                .map(|index| {
                    let mut persisted = items[*index].clone();
                    persisted.auto_applied = false;
                    persisted.applied_at = Some(now.clone());
                    persisted.undo = super::adjudicate::undo_patch_for(&items[*index]);
                    (*index, persisted)
                })
                .collect();
            // 决策状态写入与权威稿写入**同一事务**（`apply_editor_commands_tx_with` 的
            // 回调在 commit 之前执行）。若在事务外单独写，进程在「题稿已改、状态未写」
            // 之间退出就会留下「题稿已含修正值、决策仍是 Open」的矛盾态：重试会当成
            // 待办再走一遍，视图也不认得这次写入。同事务后该窗口在结构上不存在。
            let attempt = apply_editor_commands_tx_with(
                &mut conn_tx,
                &ApplyEditorCommandsInput {
                    item_id: item_id.clone(),
                    base_version: before,
                    request_id: Some(format!("recognition-accept:{}", request.request_id)),
                    commands,
                    title: None,
                },
                // 用户逐项接受建议 = 人的决定，因此留下人工保护目标：下一轮云端修复
                // 不得把用户刚接受的值再改回去。
                EditOrigin::Human,
                None,
                &apply_patch,
                &|ds| {
                    refresh_quality_report(root, &item_id, ds)?;
                    validate_authoring(ds)
                },
                &|tx, _version| {
                    for (_, item) in &accept_writes {
                        store::set_decision_status(
                            tx,
                            &batch.batch_id,
                            &item.decision_id,
                            DecisionStatusV1::Accepted.as_str(),
                            &serde_json::to_string(item).map_err(|error| error.to_string())?,
                            Some(&now),
                        )?;
                    }
                    Ok(())
                },
            );
            if attempt.is_ok() {
                applied_indices = runnable;
                // 写入已提交，才让内存副本跟上。
                for (index, persisted) in accept_writes {
                    items[index] = persisted;
                }
            } else {
                // 整批失败：如实落 Failed，不谎称已应用。
                let code = attempt
                    .err()
                    .unwrap_or_else(|| "recognition_apply_failed".to_string())
                    .chars()
                    .take(80)
                    .collect::<String>();
                for index in runnable {
                    outcomes.push(DecisionOutcomeV1 {
                        decision_id: items[index].decision_id.clone(),
                        kind: DecisionOutcomeKindV1::Failed,
                        reason_code: Some(code.clone()),
                        message: "写入失败，题稿未改动，请重试。".to_string(),
                        applied_at: None,
                        undo: None,
                    });
                }
            }
        }
    }

    for (index, code) in superseded {
        let item = &mut items[index];
        item.status = DecisionStatusV1::Superseded;
        item.reason_code = code.clone();
        item.user_message = format!("{}（题稿已被修改，建议过期）", item.user_message);
        store::set_decision_status(
            &conn,
            &batch.batch_id,
            &item.decision_id,
            DecisionStatusV1::Superseded.as_str(),
            &serde_json::to_string(item).map_err(|error| error.to_string())?,
            None,
        )?;
        outcomes.push(DecisionOutcomeV1 {
            decision_id: item.decision_id.clone(),
            kind: DecisionOutcomeKindV1::Superseded,
            reason_code: Some(code.clone()),
            message: "目标已被修改，建议过期；未覆盖你的改动。".to_string(),
            applied_at: None,
            undo: None,
        });
    }

    for index in &applied_indices {
        // 状态与撤销补丁已在上面的事务回调里写好（与题稿同生共死），这里只汇总结果。
        let item = &items[*index];
        outcomes.push(DecisionOutcomeV1 {
            decision_id: item.decision_id.clone(),
            kind: DecisionOutcomeKindV1::Applied,
            reason_code: None,
            message: "已接受并写入题稿。".to_string(),
            applied_at: item.applied_at.clone(),
            undo: item.undo.clone(),
        });
    }

    // ── 撤销写入：回滚权威稿到修正前的值，并把决策持久化为 Undone ──────
    if !undo_commands.is_empty() {
        let undo_base = store::current_edit_version(&conn, &item_id)?.unwrap_or(before);
        let attempt = apply_editor_commands_tx_with(
            &mut conn_tx,
            &ApplyEditorCommandsInput {
                item_id: item_id.clone(),
                base_version: undo_base,
                request_id: Some(format!("recognition-undo:{}", request.request_id)),
                commands: undo_commands,
                title: None,
            },
            // 用户撤销一条已应用的自动建议：值回到用户这边，同样留下保护目标。
            EditOrigin::Undo,
            None,
            &apply_patch,
            &|ds| {
                refresh_quality_report(root, &item_id, ds)?;
                validate_authoring(ds)
            },
            // 撤销的状态写入同样与回滚同一事务。这里是**故障恢复的关键**：若状态写在
            // 事务外，「题稿已回滚、Undone 未落库」的窗口就会留下一个仍然自称
            // `Accepted/AutoFixed`、但目标值其实已回到修正前的项——重试会被
            // `applied_answer_still_in_place` 误判为「用户改过」而拒绝，重启后也无法自愈。
            &|tx, _version| {
                for index in &undo_indices {
                    let item = &items[*index];
                    // 只读 `items`（回调须能当 `Fn` 用）；`set_decision_status` 会把落库 JSON
                    // 的 `status` 覆写成传入值，所以此刻内存仍是 `Accepted` 不影响持久化。
                    store::set_decision_status(
                        tx,
                        &batch.batch_id,
                        &item.decision_id,
                        DecisionStatusV1::Undone.as_str(),
                        &serde_json::to_string(item).map_err(|error| error.to_string())?,
                        item.applied_at.as_deref(),
                    )?;
                }
                Ok(())
            },
        );
        if attempt.is_ok() {
            for index in &undo_indices {
                // 写入已提交，才让内存副本跟上（写失败时内存不得先宣称已撤销）。
                items[*index].status = DecisionStatusV1::Undone;
                let item = &items[*index];
                outcomes.push(DecisionOutcomeV1 {
                    decision_id: item.decision_id.clone(),
                    kind: DecisionOutcomeKindV1::Undone,
                    reason_code: None,
                    message: "已撤销自动修正，题稿已改回修正前的值。".to_string(),
                    applied_at: None,
                    undo: None,
                });
            }
        } else {
            let code = attempt
                .err()
                .unwrap_or_else(|| "recognition_undo_failed".to_string())
                .chars()
                .take(80)
                .collect::<String>();
            for index in &undo_indices {
                outcomes.push(DecisionOutcomeV1 {
                    decision_id: items[*index].decision_id.clone(),
                    kind: DecisionOutcomeKindV1::Failed,
                    reason_code: Some(code.clone()),
                    message: "撤销写入失败，题稿未改动，请重试。".to_string(),
                    applied_at: None,
                    undo: None,
                });
            }
        }
    }

    let after = store::current_edit_version(&conn, &item_id)?.unwrap_or(before);
    // 写后重读**同一批次**的行：必须按 batch_id 取（`load_batch_by_id`）。
    // 这里曾经误用 `load_latest_batch(conn, item_id)`，但传入的是 `batch.batch_id`——
    // 该函数按 `library_item_id` 过滤，于是恒查不到、被 `unwrap_or_else` 静默回退，
    // 「从数据库重读」这条路径实际是死代码。
    let refreshed = store::load_batch_by_id(&conn, &batch.batch_id)?
        .map(|row| {
            let canonical = get_canonical_ds(&conn, &item_id).ok().flatten().map(|(ds, _)| ds);
            build_view(&row, items.clone(), after, canonical.as_ref())
        })
        .unwrap_or_else(|| build_view(&batch, items.clone(), after, None));

    let result = ApplyRecognitionDecisionsResultV1 {
        schema_version: APPLY_RECOGNITION_DECISIONS_RESULT_V1_SCHEMA_VERSION.to_string(),
        request_id: request.request_id.clone(),
        batch_id: batch.batch_id.clone(),
        edit_version_before: before,
        edit_version_after: after,
        replayed: false,
        outcomes,
        view: refreshed,
    };
    let result_json = serde_json::to_string(&result).map_err(|error| error.to_string())?;
    store::journal_insert(
        &conn,
        &request.request_id,
        &item_id,
        &batch.batch_id,
        request.base_edit_version,
        &payload,
        &result_json,
    )?;
    Ok(result_json
        .parse::<Value>()
        .unwrap_or_else(|_| json!({"schemaVersion": APPLY_RECOGNITION_DECISIONS_RESULT_V1_SCHEMA_VERSION})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_store::{make_job, save_job};
    use crate::library::repository::{
        open_library_connection, seed_canonical_ds, upsert_item_shell, UpsertItemInput,
    };
    use crate::reconcile::store::read_decision_file;
    use crate::schema::recognition_v1::{
        reason, ChainStatusV1, DecisionFieldV1, DecisionSeverityV1, DecisionSummaryV1, DecisionTargetTypeV1,
        DecisionTargetV1, RECOGNITION_DECISION_V1_SCHEMA_VERSION,
    };
    use crate::util::{ensure_app_dirs, ensure_job_dirs, job_dir, write_json};
    use crate::CommandResult;
    use crate::SourceFile;
    // `CreateJobInput` / `WorkflowStep` 定义在 crate 根；`job_store` 与
    // `workflow_state` 里的同名 `use` 是私有导入，靠它们转发拿不到类型。
    use crate::{CreateJobInput, WorkflowStep};
    use serde_json::{json, Value};
    use std::path::PathBuf;
    use uuid::Uuid;

    /// 端到端验证的夹具：建临时库 + 作业 + 已播种的 V2 权威稿 + document-ir，
    /// 让 `run_recognition_cycle_core` 能走「本地候选投影 → 云端桩 → 三路裁决」全链路。
    fn seed_bridge_job(canonical: &Value, doc_ir: &Value) -> (PathBuf, String) {
        let root = std::env::temp_dir().join(format!("pdf2test-bridge-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).expect("app dirs");
        let mut job = make_job(CreateJobInput {
            title: Some("bridge".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["t".to_string()]),
            llm_profile_id: None,
        });
        job.current_step = WorkflowStep::Authoring;
        job.source_files = vec![SourceFile {
            file_id: "file-1".to_string(),
            original_name: "source.pdf".to_string(),
            stored_name: "stored.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: "0".repeat(64),
            size_bytes: 1,
            role: "MainQuestion".to_string(),
            imported_at: chrono::Utc::now(),
        }];
        save_job(&root, &job).expect("save job");
        ensure_job_dirs(&job_dir(&root, &job.job_id)).expect("job dirs");
        write_json(
            &job_dir(&root, &job.job_id).join("authoring-ir.json"),
            &json!({"schemaVersion":"IeltsAuthoringIRV2","exam":{"title":"t"},"taskGroups":[],"answerSlots":{},"answerKey":{},"quality":{"coverageStatus":{"unassignedSourceNodeIds":[]}}}),
        )
        .expect("authoring-ir");
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), doc_ir).expect("document-ir");
        let conn = open_library_connection(&root).expect("db");
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: &job.job_id,
                modality: "reading",
                title: "t",
                status: "action_required",
                source_asset_id: None,
            },
        )
        .expect("shell");
        seed_canonical_ds(&conn, &job.job_id, &canonical.to_string(), "action_required").expect("seed");
        (root, job.job_id.clone())
    }

    /// 返回「自然形态」云端输出（模型真实 contract）：group 只有 `kind`/`range`/`questionIds`
    /// 等，没有 `taskId`/`taskType` —— 这才是 `expand_natural_cloud_shape` 要适配的输入。
    fn natural_cloud_shape(kind: &str, answer: Value, question_number: u32) -> Value {
        json!({
            "title": "Generated Reading Paper",
            "groups": [{
                "kind": kind,
                "range": [question_number, question_number],
                "layoutHint": "list",
                "questionIds": [format!("q{}", question_number)],
                "notesText": "Complete the sentence with ONE word from the passage.",
                "confidence": 0.9,
                "evidence": {"quotes": [{"pageIndex": 1, "text": "source excerpt"}]},
                "instructionsText": "Complete the sentences.",
                "slots": [{"questionNumber": question_number, "answer": answer, "evidence": {"quotes": [{"pageIndex":1,"quote":"source excerpt"}]}}]
            }],
            "answerKey": {},
            "confidence": 0.9,
            "warnings": []
        })
    }

    /// 云端桩：返回模型真实的「自然形态」（答案用 text 形状）。
    ///
    /// 刻意用 `fn` item 而不是闭包：`CloudOutlineRunner` 是
    /// `&dyn Fn(&Path, &str, Option<&str>)`，捕获型闭包会被推断成某个具体
    /// 生命周期，从而报 "implementation of `Fn` is not general enough"；
    /// fn item 天生满足高阶生命周期约束。
    fn stub_cloud_runner(
        _root: &Path,
        _job_id: &str,
        _profile_id: Option<&str>,
    ) -> CommandResult<Value> {
        Ok(natural_cloud_shape(
            "sentence_completion",
            json!({"kind":"text","values":["stencilling"],"normalization":"ielts_default"}),
            14,
        ))
    }

    /// 同上，答案文本为 "B"，用于验证形状对齐（本地 option / 云端 text）不再制造假分歧。
    fn stub_cloud_runner_answer_b(
        _root: &Path,
        _job_id: &str,
        _profile_id: Option<&str>,
    ) -> CommandResult<Value> {
        Ok(natural_cloud_shape(
            "single_choice",
            json!({"kind":"text","values":["B"],"normalization":"ielts_default"}),
            14,
        ))
    }

    #[test]
    fn cloud_natural_shape_drives_full_reconcile_cycle() {
        let (root, job_id) = seed_bridge_job(
            &json!({
                "schemaVersion": "IeltsAuthoringIRV2",
                "exam": {"title": "t"},
                "taskGroups": [{
                    "taskId": "task-1",
                    "taskType": "sentence_completion",
                    "displayRange": {"kind":"range","start":14,"end":14},
                    "responseGroups": [{"responseGroupId":"rg-1","kind":"text_entry","slotIds":["slot-14"]}]
                }],
                "answerSlots": {
                    "slot-14": {"slotId":"slot-14","questionNumber":14,"interaction":"text","sourceAnchors":[]}
                },
                "answerKey": {"slot-14": {"kind":"unresolved"}},
                "quality": {"coverageStatus": {"unassignedSourceNodeIds": []}}
            }),
            // 故意不含答案文本：避免触发自动补全，只验证云端答案「参与裁决」。
            &json!({"pages":[{"pageIndex":0,"lines":[{"text":"A passage about birds and weather."}]}]}),
        );

        let report = run_recognition_cycle_core(
            &root,
            &job_id,
            Some("test-profile"),
            true,
            0,
            &stub_cloud_runner,
        )
        .expect("cycle must run");

        // (1) 云端链不再被判为 unusable/failed：自然形态被正确解析、题组全保留。
        let cloud_status = report["cloudCandidateStatus"].as_str().expect("cloudCandidateStatus present");
        assert!(
            cloud_status == "succeeded" || cloud_status == "partial",
            "云端链应可用，实际为 {cloud_status}"
        );

        // (2) 云端答案确实参与了裁决：存在带 cloud_value 的逐项建议。
        let batch_id = report["batchId"].as_str().expect("batchId present");
        let decision = read_decision_file(&root, &job_id, batch_id).expect("decision written to disk");
        assert!(
            decision.items.iter().any(|item| item.cloud_value.is_some()),
            "至少有一项裁决引用了云端答案（cloud_value 非空）"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cloud_shape_mismatch_does_not_spawn_false_divergence() {
        let (root, job_id) = seed_bridge_job(
            &json!({
                "schemaVersion": "IeltsAuthoringIRV2",
                "exam": {"title": "t"},
                "taskGroups": [{
                    "taskId": "task-1",
                    // 单选：作答方式必须与 option 答案自洽。否则会触发
                    // SLOT_INTERACTION_ANSWER_MISMATCH（与本题要验证的形状对齐无关的 blocker）。
                    "taskType": "single_choice",
                    "displayRange": {"kind":"range","start":14,"end":14},
                    "responseGroups": [{"responseGroupId":"rg-1","kind":"choice","slotIds":["slot-14"]}]
                }],
                "answerSlots": {
                    "slot-14": {"slotId":"slot-14","questionNumber":14,"interaction":"radio","sourceAnchors":[]}
                },
                "answerKey": {"slot-14": {"kind":"option","labels":["B"],"assignment":"per_slot"}},
                "quality": {"coverageStatus": {"unassignedSourceNodeIds": []}}
            }),
            &json!({"pages":[{"pageIndex":0,"lines":[{"text":"A passage about climate."}]}]}),
        );

        // 同一答案 "B"：本地用 option 形状，云端用 text 形状 → 形状敏感比较会误判分歧。
        let report = run_recognition_cycle_core(
            &root,
            &job_id,
            Some("test-profile"),
            true,
            0,
            &stub_cloud_runner_answer_b,
        )
        .expect("cycle must run");

        let cloud_status = report["cloudCandidateStatus"].as_str().expect("cloudCandidateStatus");
        assert!(
            cloud_status == "succeeded" || cloud_status == "partial",
            "云端链应可用，实际 {cloud_status}"
        );

        let batch_id = report["batchId"].as_str().expect("batchId");
        let decision = read_decision_file(&root, &job_id, batch_id).expect("decision written");

        // 断言收窄说明（原断言「全局不存在 SUBSTANTIVE_DIVERGENCE 项」过宽且已废弃）：
        // `reason::SUBSTANTIVE_DIVERGENCE` 在全仓有 13 处产出点
        // （compare_slot_coverage / compare_slots / compare_groups / compare_assets /
        //  compare_source_coverage ...），全局无该 reason 并不能隔离本测试真正要验证的
        // 维度——「同一答案因 AnswerValueV2 形状不同（本地 option / 云端 text）而被误判为
        // 实质分歧」。因此这里只锁定 **slot-14 的 answer 字段** 这一维度。
        let slot_answer_divergent: Vec<&str> = decision
            .items
            .iter()
            .filter(|item| {
                item.target.target_type == DecisionTargetTypeV1::Slot
                    && item.target.target_id == "slot-14"
                    && item.field == DecisionFieldV1::Answer
                    && item.reason_code == reason::SUBSTANTIVE_DIVERGENCE
            })
            .map(|item| item.decision_id.as_str())
            .collect();
        assert!(
            slot_answer_divergent.is_empty(),
            "同一答案 \"B\" 仅因形状不同（本地 option / 云端 text）就在 slot-14 产出实质分歧，\
             说明 align_cloud_answer_shapes 未生效：{:?}",
            slot_answer_divergent
        );

        // 更强、更直接的证据：slot-14 的答案项上，本地值与云端值必须**字节一致**。
        // 云端原始形态是 text[B]，本地是 option[B]/per_slot；两者相等即证明云端已被
        // 重新编码为本地形状（对齐逻辑真正跑通），而非「两边恰好都没有值」。
        let aligned = decision.items.iter().find(|item| {
            item.target.target_type == DecisionTargetTypeV1::Slot
                && item.target.target_id == "slot-14"
                && item.field == DecisionFieldV1::Answer
                && item.local_value.is_some()
                && item.cloud_value.is_some()
        });
        let aligned = aligned.expect(
            "应存在一项 slot-14 的答案裁决同时携带 local_value 与 cloud_value，\
             否则本测试并未真正覆盖「本地 vs 云端」这一维度",
        );
        assert_eq!(
            aligned.local_value, aligned.cloud_value,
            "同一答案 \"B\" 的本地值与云端值应经形状对齐后完全一致（decision_id={}）",
            aligned.decision_id
        );

        // 云端答案仍应参与（提供值）。
        assert!(
            decision.items.iter().any(|item| item.cloud_value.is_some()),
            "云端答案应参与裁决"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn batch_id_is_deterministic_and_path_safe() {
        let first = recognition_batch_id("job/with:weird*chars", "abcdef0123456789", 3);
        let second = recognition_batch_id("job/with:weird*chars", "abcdef0123456789", 3);
        assert_eq!(first, second, "同一输入必须得到同一批次 id");
        assert!(!first.contains('/') && !first.contains(':') && !first.contains('*'));
        assert_ne!(first, recognition_batch_id("job/with:weird*chars", "abcdef0123456789", 4));
        assert_eq!(
            recognition_batch_id("job-1", "", 1),
            recognition_batch_id("job-1", "", 1)
        );
    }

    #[test]
    fn version_bump_changes_the_batch() {
        // 重试（版本不变）复用批次；用户改稿后新批次，避免拿旧建议覆盖新稿。
        let a = recognition_batch_id("job-1", "deadbeef0000", 7);
        let b = recognition_batch_id("job-1", "deadbeef0000", 8);
        assert_ne!(a, b);
    }

    #[test]
    fn auto_apply_recheck_rejects_a_slot_the_user_already_filled() {
        let canonical = serde_json::json!({
            "answerKey": {"slot-1": {"kind": "text", "values": ["carving"]}}
        });
        let snapshot = serde_json::json!({"kind": "unresolved"});
        let proposed = serde_json::json!({"kind": "text", "values": ["stencilling"]});
        assert!(
            !still_auto_applicable(&canonical, Some(&snapshot), "slot-1", &proposed),
            "用户已经填过答案，不得自动覆盖"
        );
    }

    #[test]
    fn auto_apply_recheck_accepts_an_untouched_empty_slot() {
        let canonical = serde_json::json!({
            "answerKey": {"slot-1": {"kind": "unresolved"}}
        });
        let snapshot = serde_json::json!({"kind": "unresolved"});
        let proposed = serde_json::json!({"kind": "text", "values": ["stencilling"]});
        assert!(still_auto_applicable(&canonical, Some(&snapshot), "slot-1", &proposed));
    }

    fn decision_item(
        target_id: &str,
        status: DecisionStatusV1,
        resolution: DecisionResolutionV1,
    ) -> DecisionItemV1 {
        DecisionItemV1 {
            decision_id: format!("d:slot:{target_id}:answer"),
            resolution,
            code: "ANSWER_CONFLICT".to_string(),
            severity: DecisionSeverityV1::Blocker,
            title: "t".to_string(),
            user_message: "m".to_string(),
            target: DecisionTargetV1 {
                target_type: DecisionTargetTypeV1::Slot,
                target_id: target_id.to_string(),
                task_id: None,
                node_id: None,
                question_numbers: vec![1],
            },
            field: DecisionFieldV1::Answer,
            evidence: vec![],
            local_value: None,
            cloud_value: None,
            source_value: None,
            proposed_patch: None,
            undo: None,
            auto_applied: false,
            applied_at: None,
            status,
            reason_code: reason::SUBSTANTIVE_DIVERGENCE.to_string(),
            dependency_group: None,
        }
    }

    /// `cmd-state` 修复的回归锁定：**自动应用失败的项（`Failed`）必须留在 `actionable`**。
    ///
    /// 它是「用户最需要处理」的项——自动写入失败、权威稿其实没有被修正，若被当成
    /// 「已处理」过滤掉，界面上不会出现任何提示，静默丢掉的恰恰是最该看见的问题。
    /// 同时反向锁定不得过度暴露：已被用户处理（接受/拒绝）、已过期（`Superseded`）、
    /// 已撤销（`Undone`）的项不得重新冒出来；`AutoFixed` 只能进 `auto_applied`。
    ///
    /// 此前该分支**零测试**（审计发现的唯一缺口：产品改动已落地，但没有任何断言保护它）。
    #[test]
    fn build_view_keeps_failed_items_actionable_but_hides_handled_ones() {
        let batch = store::BatchRow {
            batch_id: "batch-1".to_string(),
            library_item_id: "item-1".to_string(),
            job_id: "job-1".to_string(),
            base_edit_version: 3,
            source_sha256: "a".repeat(64),
            chain_status: ChainStatusSummaryV1 {
                local: ChainStatusV1::Succeeded,
                cloud: ChainStatusV1::Succeeded,
                source: ChainStatusV1::Succeeded,
                cloud_reason_code: None,
                source_reason_code: None,
            },
            summary: DecisionSummaryV1 {
                agreed: 0,
                auto_fixed: 1,
                needs_review: 4,
                unverifiable: 0,
            },
            chain_state: None,
            // 本次没有修复记录：调用方必须按「不知道」降级，不得当成 completed。
            repair: None,
            updated_at: "2026-09-15T00:00:00Z".to_string(),
        };

        let items = vec![
            decision_item("open-1", DecisionStatusV1::Open, DecisionResolutionV1::NeedsReview),
            // 自动应用写入失败：权威稿未改动，必须保留为待办。
            decision_item("failed-1", DecisionStatusV1::Failed, DecisionResolutionV1::NeedsReview),
            // 已处理 / 已过期 / 已撤销：不得重新成为待办。
            decision_item("accepted-1", DecisionStatusV1::Accepted, DecisionResolutionV1::NeedsReview),
            decision_item("rejected-1", DecisionStatusV1::Rejected, DecisionResolutionV1::NeedsReview),
            decision_item("superseded-1", DecisionStatusV1::Superseded, DecisionResolutionV1::NeedsReview),
            decision_item("undone-1", DecisionStatusV1::Undone, DecisionResolutionV1::NeedsReview),
            // 已自动写入：只进 auto_applied。
            decision_item("autofixed-1", DecisionStatusV1::Accepted, DecisionResolutionV1::AutoFixed),
        ];

        let view = build_view(&batch, items, 5, None);

        let mut actionable_ids: Vec<&str> = view
            .actionable
            .iter()
            .map(|item| item.target.target_id.as_str())
            .collect();
        actionable_ids.sort_unstable();
        assert_eq!(
            actionable_ids,
            vec!["failed-1", "open-1"],
            "只有 open 与 failed 是待办；失败项若被过滤，用户将看不到必须处理的问题"
        );

        let auto_ids: Vec<&str> = view
            .auto_applied
            .iter()
            .map(|item| item.target.target_id.as_str())
            .collect();
        assert_eq!(auto_ids, vec!["autofixed-1"], "AutoFixed 只能进 auto_applied");

        // 视图自身的陈旧判定：批次基线 3 早于当前编辑版本 5。
        assert_eq!(view.base_edit_version, 3);
        assert_eq!(view.edit_version, 5);
        assert!(view.stale, "base_edit_version < edit_version 时必须标记 stale");
    }

    /// `cmd-state` 任务的另一半：**接受 / 撤销后的状态必须跨「重开」存活**。
    ///
    /// 机制：状态落在 `recognition_decisions_v1.item_json`（`store::set_decision_status`），
    /// 重开时由 `load_decision_items` 读回，`build_view` 再按状态过滤——因此用户处理过的项
    /// 不会重新变成待办，也不会凭空消失。此前**没有任何「文件库 + 断连重开」的测试**
    /// 覆盖这条链路：`store.rs` 里那条用的是内存库，语义上根本测不到「重开」。
    #[test]
    fn decision_status_survives_a_reopen_and_never_returns_to_actionable() {
        let root =
            std::env::temp_dir().join(format!("pdf2test-reopen-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).expect("app dirs");
        let item_id = "reopen-item";

        let decision = RecognitionDecisionV1 {
            schema_version: RECOGNITION_DECISION_V1_SCHEMA_VERSION.to_string(),
            batch_id: "batch-1".to_string(),
            item_id: item_id.to_string(),
            job_id: "job-1".to_string(),
            base_edit_version: 0,
            generated_at: "2026-09-16T00:00:00Z".to_string(),
            chain_status: ChainStatusSummaryV1 {
                local: ChainStatusV1::Succeeded,
                cloud: ChainStatusV1::Succeeded,
                source: ChainStatusV1::Succeeded,
                cloud_reason_code: None,
                source_reason_code: None,
            },
            items: vec![decision_item(
                "slot-1",
                DecisionStatusV1::Open,
                DecisionResolutionV1::NeedsReview,
            )],
            summary: DecisionSummaryV1 {
                agreed: 0,
                auto_fixed: 0,
                needs_review: 1,
                unverifiable: 0,
            },
        };

        // ── 第一次开库：写入批次与待办项，然后用户「接受」该项。──────────────
        {
            let conn = open_library_connection(&root).expect("db");
            upsert_item_shell(
                &conn,
                &UpsertItemInput {
                    id: item_id,
                    modality: "reading",
                    title: "t",
                    status: "action_required",
                    source_asset_id: None,
                },
            )
            .expect("shell");
            store::upsert_batch(&conn, &decision).expect("batch");
            store::replace_decision_items(&conn, &decision).expect("items");

            let row = store::load_latest_batch(&conn, item_id)
                .expect("load")
                .expect("batch row");
            let items = store::load_decision_items(&conn, "batch-1").expect("items");
            assert_eq!(
                build_view(&row, items, 0, None).actionable.len(),
                1,
                "尚未处理的项必须出现在待办中"
            );

            let mut accepted = decision.items[0].clone();
            accepted.status = DecisionStatusV1::Accepted;
            accepted.auto_applied = false;
            accepted.applied_at = Some("2026-09-16T00:00:01Z".to_string());
            store::set_decision_status(
                &conn,
                "batch-1",
                &accepted.decision_id,
                DecisionStatusV1::Accepted.as_str(),
                &serde_json::to_string(&accepted).expect("json"),
                accepted.applied_at.as_deref(),
            )
            .expect("accept must be persisted");
        } // 连接在此断开——等价于应用退出。

        // ── 第二次开库（重开）：接受状态必须仍在，且不得重新成为待办。────────
        let conn = open_library_connection(&root).expect("reopen db");
        let row = store::load_latest_batch(&conn, item_id)
            .expect("load")
            .expect("batch row");
        let items = store::load_decision_items(&conn, "batch-1").expect("items");
        assert_eq!(items.len(), 1, "重开后裁决项数量不得变化");
        assert_eq!(
            items[0].status,
            DecisionStatusV1::Accepted,
            "接受状态必须跨重开存活（这是「重开不丢已处理状态」的机制本身）"
        );
        assert!(
            build_view(&row, items, 0, None).actionable.is_empty(),
            "已接受的项在重开后不得重新冒出来"
        );

        drop(conn);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 云端桩哨兵：无云路径下**绝不允许被调用**。用它当哨兵——一旦核心在
    /// `cloud_enabled = false` 时仍去请求云端，本测试立刻 panic，而不是静默通过。
    fn sentinel_cloud_runner(
        _root: &Path,
        _job_id: &str,
        _profile_id: Option<&str>,
    ) -> CommandResult<Value> {
        panic!("cloud runner must never be invoked when cloud_enabled = false");
    }

    /// 一份「单题 sentence_completion」的最小权威稿，答案位 `slot-14` 的当前值为 `answer`。
    /// 仅供**无云路径**测试使用：那条路径只读不写（没有云端建议可自动应用），
    /// 因而不需要能通过 `validate_authoring` 的完整稿。
    /// 凡是**会写库**的用例一律用 [`canonical_from_golden`]，理由见该函数。
    fn sentence_completion_canonical(answer: &Value) -> Value {
        json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "exam": {"title": "t"},
            "taskGroups": [{
                "taskId": "task-1",
                "taskType": "sentence_completion",
                "displayRange": {"kind":"range","start":14,"end":14},
                "responseGroups": [{"responseGroupId":"rg-1","kind":"text_entry","slotIds":["slot-14"]}]
            }],
            "answerSlots": {
                "slot-14": {"slotId":"slot-14","questionNumber":14,"interaction":"text","sourceAnchors":[]}
            },
            "answerKey": {"slot-14": answer.clone()},
            "quality": {"coverageStatus": {"unassignedSourceNodeIds": []}}
        })
    }

    /// 自动修正写入的值（正 patch 的 value）：夹具选项库里的 **B**。
    fn applied_option() -> Value {
        json!({"kind":"option","labels":["B"],"assignment":"unordered_set"})
    }

    /// 修正**之前**的值（逆 patch 要恢复的目标）：夹具选项库里的 **A**。
    fn original_option() -> Value {
        json!({"kind":"option","labels":["A"],"assignment":"unordered_set"})
    }

    /// 一份**合法**的 V2 权威稿（取自 `fixtures/golden/synthetic/ielts/
    /// early-approaches-authoring-v2.json`），答案位 `q14` 的当前值被替换为 `slot_answer`。
    ///
    /// 为什么必须用真实夹具，而不是手写一份最小 JSON：撤销与接受都要经
    /// `apply_editor_commands_tx` → `refresh_quality_report` + `validate_authoring`，
    /// 那是**强类型 schema 校验**。手写的精简稿会因缺 `displayLabel` 这类必填字段被拒
    /// （`AUTHORING_SCHEMA_INVALID:missing field ...`），失败原因就变成「夹具不合格」，
    /// 把真正要验证的撤销/失效逻辑掩盖掉——这正是第一版测试踩到的坑。
    fn canonical_from_golden(slot_answer: &Value) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read fixture {}: {error}", path.display()));
        let mut canonical: Value = serde_json::from_str(&raw).expect("fixture must be valid JSON");
        canonical["answerKey"]["q14"] = slot_answer.clone();
        canonical
    }

    /// 播种一个「已自动修正、等待用户查看」的批次。
    ///
    /// `slot_value` 是权威稿 `q14` 的**当前值**，它决定这条修正是否仍然生效：
    /// - 等于 [`applied_option`] ⇒ 修正仍在位（撤销应放行、视图应示为已修正）；
    /// - 其它值 ⇒ 用户改过该槽位（撤销应被拒、视图应示为失效）。
    ///
    /// 返回 `(root, item_id, batch_id, item)`。
    fn seed_applied_batch(
        tag: &str,
        slot_value: &Value,
    ) -> (PathBuf, String, String, DecisionItemV1) {
        let root =
            std::env::temp_dir().join(format!("pdf2test-{tag}-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).expect("app dirs");
        let item_id = format!("{tag}-item");
        let batch_id = format!("batch-{tag}");

        let mut item = decision_item(
            "q14",
            DecisionStatusV1::Accepted,
            DecisionResolutionV1::AutoFixed,
        );
        item.field = DecisionFieldV1::Answer;
        item.local_value = Some(original_option());
        item.proposed_patch = Some(crate::reconcile::rules::answer_patch("q14", &applied_option()));
        item.undo = Some(crate::reconcile::rules::answer_patch(
            "q14",
            &original_option(),
        ));
        item.auto_applied = true;
        item.applied_at = Some("2026-09-16T00:00:00Z".to_string());

        let decision = RecognitionDecisionV1 {
            schema_version: RECOGNITION_DECISION_V1_SCHEMA_VERSION.to_string(),
            batch_id: batch_id.clone(),
            item_id: item_id.clone(),
            job_id: item_id.clone(),
            base_edit_version: 0,
            generated_at: "2026-09-16T00:00:00Z".to_string(),
            chain_status: ChainStatusSummaryV1 {
                local: ChainStatusV1::Succeeded,
                cloud: ChainStatusV1::NotRun,
                source: ChainStatusV1::Succeeded,
                cloud_reason_code: Some(reason::CLOUD_DISABLED.to_string()),
                source_reason_code: None,
            },
            items: vec![item.clone()],
            summary: DecisionSummaryV1 {
                agreed: 0,
                auto_fixed: 1,
                needs_review: 0,
                unverifiable: 0,
            },
        };

        let conn = open_library_connection(&root).expect("db");
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: &item_id,
                modality: "reading",
                title: "t",
                status: "action_required",
                source_asset_id: None,
            },
        )
        .expect("shell");
        seed_canonical_ds(
            &conn,
            &item_id,
            &canonical_from_golden(slot_value).to_string(),
            "action_required",
        )
        .expect("seed canonical");
        store::upsert_batch(&conn, &decision).expect("batch");
        store::replace_decision_items(&conn, &decision).expect("items");
        drop(conn);

        (root, item_id, batch_id, item)
    }

    /// 当前权威稿里 `q14` 的答案值。
    fn q14_answer(root: &Path, item_id: &str) -> Value {
        let conn = open_library_connection(root).expect("db");
        let (canonical, _) = get_canonical_ds(&conn, item_id)
            .expect("read canonical")
            .expect("canonical present");
        canonical_answer(&canonical, "q14")
            .cloned()
            .expect("q14 answer present")
    }

    /// 无云分支的回归锁定（R1）：`cloud_enabled = false` 时**仍要跑完 reconcile 并落盘**。
    ///
    /// 修复前该分支在发布「可编辑」之后直接 `return`：本地候选快照、原文核验、批次与
    /// 决策**全部缺失**——前端拿不到任何可解释的证据链，产品语义上等于「没做识别」。
    /// 修复后只把云端如实标成 `not_run`（`CLOUD_DISABLED`），其余链路照常走完。
    ///
    /// 「不得用假失败 profile 绕过产品缺口」这一条由哨兵桩守住：注入点若被调用即 panic。
    #[test]
    fn no_cloud_branch_still_runs_reconcile_and_persists_local_evidence() {
        let (root, job_id) = seed_bridge_job(
            &sentence_completion_canonical(&json!({"kind":"text","values":["stencilling"]})),
            &json!({"pages":[{"pageIndex":0,"lines":[{"text":"A passage about birds and weather."}]}]}),
        );

        let report =
            run_recognition_cycle_core(&root, &job_id, None, false, 0, &sentinel_cloud_runner)
                .expect("无云路径仍必须跑完 reconcile，而不是跳过");

        // (1) 云端如实呈现「未运行」，而不是被折叠成 failed。
        assert_eq!(
            report["cloudCandidateStatus"].as_str(),
            Some("not_run"),
            "未启用云端必须呈现为 not_run；谎报 failed 会让用户以为云端出故障"
        );
        // (2) 本地候选正常产出。
        assert_eq!(
            report["localCandidateStatus"].as_str(),
            Some("succeeded"),
            "无云路径下本地候选必须正常产出"
        );
        // (3) 本地链真的跑过（「跳过 reconcile 后返回空壳」会把这里留成 not_run）。
        assert_ne!(
            report["chains"]["local"]["state"].as_str(),
            Some("not_run"),
            "本地链为 not_run 说明 reconcile 根本没跑——正是修复前跳过分支的症状"
        );
        // (4) 批次、决策与本地候选快照都已落盘：这就是前端可解释性的全部来源。
        let batch_id = report["batchId"].as_str().expect("batchId present");
        let decision = read_decision_file(&root, &job_id, batch_id)
            .expect("无云路径也必须留下决策文件（本地候选 + 原文核验 + 裁决）");
        assert_eq!(decision.batch_id, batch_id);
        assert!(
            store::read_candidate(&root, &job_id, batch_id, store::LOCAL_CANDIDATE_FILE).is_some(),
            "本地候选快照必须落盘，否则裁决无据可依"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 撤销闭环的持久化（R5）。与上面「接受状态跨重开存活」形成对称覆盖。
    ///
    /// 端到端走一次**真实撤销命令**：回滚权威稿 → 持久化 `Undone` → 重开数据库仍为
    /// `Undone`，且该状态在**所有出口**都成立——不再出现在待办、不再以「已自动修正」
    /// 展示、也不能再被撤销一次。
    ///
    /// 之所以要「重开」而不是同连接复读：`store::set_decision_status` 写的是 SQLite，
    /// 只有断连重开才能排除「其实只改了内存里的 `items`」这种假通过。
    #[test]
    fn undo_rolls_back_canonical_and_survives_a_reopen() {
        // 权威稿当前值 == 修正写入的值 ⇒ 撤销应当放行。
        let (root, item_id, batch_id, item) = seed_applied_batch("undo", &applied_option());
        // 夹具里那条 undo patch 恢复的值。
        let restore = original_option();

        let result = apply_recognition_decisions_core(
            &root,
            ApplyRecognitionDecisionsRequestV1 {
                request_id: "req-undo-1".to_string(),
                batch_id: batch_id.clone(),
                base_edit_version: 0,
                accept: vec![],
                reject: vec![],
                undo: vec![item.decision_id.clone()],
            },
        )
        .expect("undo must run");

        // (1) 结果如实报 Undone，而不是静默成功。
        assert_eq!(
            result["outcomes"][0]["kind"].as_str(),
            Some("undone"),
            "撤销结果必须如实报 undone：{result}"
        );
        // (2) 权威稿真的回到修正前的值——这是「回滚」的实质。
        assert_eq!(
            q14_answer(&root, &item_id),
            restore,
            "撤销后 q14 必须回到修正前的值"
        );
        // (3) 撤销项在本连接内就已不再待办，也不再以「已自动修正」身份展示。
        {
            let conn = open_library_connection(&root).expect("db");
            let row = store::load_batch_by_id(&conn, &batch_id).expect("load").expect("row");
            let items = store::load_decision_items(&conn, &batch_id).expect("items");
            let (canonical, _) = get_canonical_ds(&conn, &item_id).expect("ok").expect("present");
            let view = build_view(&row, items, 0, Some(&canonical));
            assert!(
                view.actionable.is_empty(),
                "已撤销的项不得重新成为待办（否则用户会被要求重复处理同一件事）"
            );
            assert!(
                view.auto_applied.is_empty(),
                "已撤销的项不得继续以「已自动修正」身份展示：\
                 撤销只翻 status，resolution 仍是 AutoFixed，若只看 resolution 就会漏在这"
            );
        } // 连接断开——等价于应用退出。

        // ── 重开数据库：撤销状态必须仍在，且所有出口一致。────────────────────
        let conn = open_library_connection(&root).expect("reopen db");
        let items = store::load_decision_items(&conn, &batch_id).expect("items");
        assert_eq!(items.len(), 1, "重开后裁决项数量不得变化");
        assert_eq!(
            items[0].status,
            DecisionStatusV1::Undone,
            "撤销状态必须跨重开存活（这是「撤销闭环」的机制本身）"
        );
        let row = store::load_batch_by_id(&conn, &batch_id).expect("load").expect("row");
        let canonical = get_canonical_ds(&conn, &item_id).expect("ok").expect("present").0;
        let view = build_view(&row, items, 0, Some(&canonical));
        assert!(view.actionable.is_empty(), "重开后已撤销项不得回到待办");
        assert!(view.auto_applied.is_empty(), "重开后已撤销项不得回到 autoApplied");

        drop(conn);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── 故障注入辅助（Task 25）─────────────────────────────────────────────

    /// 让 `recognition_decisions_v1` 的 UPDATE 必然失败，用来构造
    /// 「题稿已改、决策状态写不进」的窗口。
    ///
    /// 用 SQLite 触发器而不是测试专用开关：注入点落在**真实产品路径**上。被测的
    /// `apply_recognition_decisions_core` 自己按 `root` 开连接，看到的是同一个库文件里的
    /// 触发器，因此产品代码里不需要留任何测试专用分支——测的是产品，不是替身。
    fn fail_decision_status_writes(root: &Path) {
        let conn = open_library_connection(root).expect("db");
        conn.execute_batch(
            "CREATE TRIGGER zz_fail_decision_status BEFORE UPDATE ON recognition_decisions_v1
             BEGIN SELECT RAISE(ABORT, 'injected_decision_status_write_failure'); END;",
        )
        .expect("create trigger");
    }

    fn allow_decision_status_writes(root: &Path) {
        let conn = open_library_connection(root).expect("db");
        conn.execute_batch("DROP TRIGGER IF EXISTS zz_fail_decision_status;")
            .expect("drop trigger");
    }

    /// 断连重开后的编辑版本（每次都新建连接，排除「只改了内存」的假通过）。
    fn edit_version(root: &Path, item_id: &str) -> i64 {
        let conn = open_library_connection(root).expect("db");
        store::current_edit_version(&conn, item_id)
            .expect("read version")
            .expect("item row present")
    }

    fn status_of(root: &Path, batch_id: &str) -> DecisionStatusV1 {
        let conn = open_library_connection(root).expect("db");
        store::load_decision_items(&conn, batch_id)
            .expect("items")
            .into_iter()
            .next()
            .expect("item present")
            .status
    }

    fn undo_request(
        request_id: &str,
        batch_id: &str,
        decision_id: &str,
    ) -> ApplyRecognitionDecisionsRequestV1 {
        ApplyRecognitionDecisionsRequestV1 {
            request_id: request_id.to_string(),
            batch_id: batch_id.to_string(),
            base_edit_version: 0,
            accept: vec![],
            reject: vec![],
            undo: vec![decision_id.to_string()],
        }
    }

    /// Task 25：撤销的故障恢复。
    ///
    /// 注入「题稿已回滚、`Undone` 状态写入失败」的窗口，验证该窗口**不可能留下半截状态**，
    /// 并验证重试与重启后内容 / 决策状态 / 幂等结果三者一致。
    ///
    /// 为什么不能拿「正常完成后重开」代替：那条路径只证明了成功情形可持久化，完全没有
    /// 触及「内容已提交、状态未提交」这一中间态——而它恰恰是崩溃会留下的东西。修法是把
    /// 状态写入并入题稿事务（`apply_editor_commands_tx_with`），因此本用例的真正断言是
    /// **状态写不进去时，题稿的回滚也必须一并作废**。
    #[test]
    fn undo_status_write_failure_rolls_the_canonical_rollback_back_with_it() {
        let (root, item_id, batch_id, item) = seed_applied_batch("undo-atomic", &applied_option());
        let version_before = edit_version(&root, &item_id);
        assert_eq!(q14_answer(&root, &item_id), applied_option(), "前置：修正值在位");

        fail_decision_status_writes(&root);

        let failed = apply_recognition_decisions_core(
            &root,
            undo_request("req-undo-atomic-1", &batch_id, &item.decision_id),
        )
        .expect("撤销失败必须如实回报，而不是把错误变成 panic");

        // (1) 结果如实报失败，不谎称已撤销。
        assert_eq!(
            failed["outcomes"][0]["kind"].as_str(),
            Some("failed"),
            "状态写不进去时不得报 undone：{failed}"
        );
        assert_eq!(
            failed["view"]["autoApplied"].as_array().map(Vec::len),
            Some(1),
            "内存视图也不得宣称已撤销，否则 view 与 outcomes 自相矛盾：{failed}"
        );
        // (2) **题稿没有被回滚**——状态写失败必须把内容写一并作废。这是本次修复的要害：
        //     修复前状态写在事务外，题稿会停在「已回到修正前、状态仍是已修正」的矛盾态。
        assert_eq!(
            q14_answer(&root, &item_id),
            applied_option(),
            "状态写入失败时，题稿回滚必须随之作废（不能只成功一半）"
        );
        // (3) 版本号未推进：编辑日志与内容一并未提交（同事务回滚的旁证）。
        assert_eq!(
            edit_version(&root, &item_id),
            version_before,
            "事务回滚后编辑版本不得推进"
        );
        // (4) 决策仍为 Accepted：内容与状态一致，不存在「回滚了但没记」。
        assert_eq!(status_of(&root, &batch_id), DecisionStatusV1::Accepted);

        // 撤除注入 → 换**新** request_id 重试（真实 UI 重试即如此）→ 必须收敛。
        allow_decision_status_writes(&root);
        let retry = apply_recognition_decisions_core(
            &root,
            undo_request("req-undo-atomic-2", &batch_id, &item.decision_id),
        )
        .expect("retry must run");
        assert_eq!(
            retry["outcomes"][0]["kind"].as_str(),
            Some("undone"),
            "撤除故障后重试必须成功：{retry}"
        );
        assert_eq!(q14_answer(&root, &item_id), original_option(), "重试后题稿已回滚");
        assert_eq!(status_of(&root, &batch_id), DecisionStatusV1::Undone, "重试后状态已落库");

        // (5) 重启（断连重开）后内容与状态仍自洽——这是「重启后一致」的断言。
        assert_eq!(status_of(&root, &batch_id), DecisionStatusV1::Undone);
        assert_eq!(q14_answer(&root, &item_id), original_option());

        // (6) 幂等一致性：同一 request_id 再提交，按日志原样重放，不再产生第二次回滚。
        let replay = apply_recognition_decisions_core(
            &root,
            undo_request("req-undo-atomic-2", &batch_id, &item.decision_id),
        )
        .expect("replay must run");
        assert_eq!(replay["replayed"].as_bool(), Some(true), "同 id 重放必须标记 replayed");
        assert_eq!(q14_answer(&root, &item_id), original_option());
        assert_eq!(status_of(&root, &batch_id), DecisionStatusV1::Undone);

        // (7) 失败的那次请求同样被记进幂等日志：同 id 重放得到**原样的失败结果**且不改状态。
        //     这是刻意的——重试必须换新 request_id，让「重试」不可能悄悄产生与首次不同的
        //     结果；否则同一请求在重试后成功、会让「同一请求只生效一次」的契约失效。
        let replay_failed = apply_recognition_decisions_core(
            &root,
            undo_request("req-undo-atomic-1", &batch_id, &item.decision_id),
        )
        .expect("replay of the failed request must run");
        assert_eq!(replay_failed["replayed"].as_bool(), Some(true));
        assert_eq!(replay_failed["outcomes"][0]["kind"].as_str(), Some("failed"));
        assert_eq!(
            status_of(&root, &batch_id),
            DecisionStatusV1::Undone,
            "重放失败结果不得改变现状"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Task 26：无可信本地基线时**不得自动写入**。
    ///
    /// 自动应用的守卫之一是「权威稿当前值 == 本地识别结果」（保证用户没改过该槽位）。
    /// 该守卫只有在本地基线是**冻结快照**时才成立：`resolve_local_snapshot` 在快照缺失或
    /// 批次不匹配时会退回「按当前权威稿现场重投影」，两边恒等、守卫退化为空操作。
    ///
    /// 本用例是受控实验：**唯一变量是本地快照**，其余输入完全相同。因此
    /// 「可信基线有候选」构成了非空性对照，使「不可信基线零候选」不是一句空断言。
    #[test]
    fn reconcile_without_a_frozen_baseline_refuses_to_auto_apply() {
        let batch_id = "batch-baseline";
        let source_sha = "a".repeat(64);
        let mut canonical = sentence_completion_canonical(&json!({"kind": "unresolved"}));
        // 非空 sourceAnchors ⇒ 本地「有原文证据面却没填值」，原文里又有可读答案行
        // ⇒ 原文核验给出 `Suggested`，这正是自动补空的触发条件。
        canonical["answerSlots"]["slot-14"]["sourceAnchors"] = json!([{
            "sourceFileId": "file-1",
            "pageIndex": 0,
            "nodeIds": ["line-14"],
            "extractionMode": "pdf_native",
            "sourceHash": "b".repeat(64)
        }]);
        let document_ir = json!({"pages":[{"pageIndex":0,"lines":[{"text":"14 stencilling"}]}]});

        // 同一套输入，只换本地快照。
        let run = |snapshot: Option<crate::schema::recognition_v1::RecognitionCandidateV1>| {
            let validate = |_patches: &[Value]| -> Result<(), String> { Ok(()) };
            reconcile_batch(ReconcileBatchInput {
                item_id: "item-1",
                job_id: "item-1",
                batch_id,
                source_file_id: "file-1",
                source_sha256: &source_sha,
                base_edit_version: 0,
                canonical: &canonical,
                document_ir: Some(&document_ir),
                local_snapshot: snapshot,
                cloud: Ok(natural_cloud_shape(
                    "sentence_completion",
                    json!({"kind":"text","values":["stencilling"]}),
                    14,
                )),
                validate_batch: &validate,
                source_verifier: None,
                adjudicator: None,
            })
        };
        let frozen_local = || {
            crate::reconcile::candidate::local_candidate_from_authoring(
                &canonical,
                batch_id,
                "item-1",
                "item-1",
                "file-1",
                &source_sha,
                0,
            )
        };

        // (1) 可信基线：基线被复用，且**确实**产出候选（非空性对照）。
        let frozen = run(Some(frozen_local()));
        assert!(frozen.local_baseline_frozen, "批次参数一致的本地快照必须被复用");
        assert!(
            !frozen.auto_apply_candidates.is_empty(),
            "本夹具在可信基线下必须产出自动写入候选，否则下一条断言是空的"
        );

        // (2) 快照缺失：基线不可信 ⇒ 候选必须清零。
        let unfrozen = run(None);
        assert!(!unfrozen.local_baseline_frozen, "快照缺失 ⇒ 基线不可信");
        assert!(
            unfrozen.auto_apply_candidates.is_empty(),
            "无可信基线时不得产出任何自动写入候选（否则会覆盖用户编辑）"
        );
        // (3) 被拒的项**没有消失**：仍留在 items 里、带可解释原因码、且仍是待办，
        //     由人工确认而不是静默丢弃。
        let blocked: Vec<_> = unfrozen
            .decision
            .items
            .iter()
            .filter(|item| item.reason_code == reason::BASELINE_NOT_FROZEN)
            .collect();
        assert!(
            !blocked.is_empty(),
            "被拒绝自动应用的项必须带 BASELINE_NOT_FROZEN 原因码留下，不能静默消失"
        );
        assert!(
            blocked.iter().all(|item| item.is_actionable()),
            "被拒绝自动应用的项必须仍是待办（否则用户看不见、也改不了）"
        );
    }

    /// 极简 HTTP 服务：对收到的每个请求回一份固定的 chat-completions 响应。
    ///
    /// 这是「受控模型服务」在测试里的落地形态；给前端用的可执行版本是
    /// `scripts/controlled-llm-service.mjs`（同一个样本文件、同一种应答）。
    ///
    /// 返回 `(base_url, 停止句柄)`。**调用方不要 join 服务线程**：网关只在解析失败时重试，
    /// 正常路径只发一次请求，而 accept 循环会一直等下一个连接——join 必然把用例挂死。
    /// 线程随测试进程结束而结束，端口是随机分配的，不会互相干扰。
    fn spawn_controlled_model_service(response_body: String) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind controlled service");
        let addr = listener.local_addr().expect("local addr");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                // 必须把请求体读完再回写：否则客户端还在发 body 时会收到 RST，
                // 得到一个与「受控服务」无关的传输错误，把要验证的东西掩盖掉。
                let mut request = Vec::<u8>::new();
                let mut chunk = [0u8; 4096];
                let mut header_end: Option<usize> = None;
                let mut content_length = 0usize;
                loop {
                    if let Some(end) = header_end {
                        if request.len() >= end + content_length {
                            break;
                        }
                    }
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(read) => {
                            request.extend_from_slice(&chunk[..read]);
                            if header_end.is_none() {
                                if let Some(position) = request
                                    .windows(4)
                                    .position(|window| window == b"\r\n\r\n")
                                {
                                    header_end = Some(position + 4);
                                    let headers =
                                        String::from_utf8_lossy(&request[..position]).to_lowercase();
                                    content_length = headers
                                        .lines()
                                        .find_map(|line| line.strip_prefix("content-length:"))
                                        .and_then(|value| value.trim().parse::<usize>().ok())
                                        .unwrap_or(0);
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    response_body.as_bytes().len(),
                    response_body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{}/v1", addr.port())
    }

    fn controlled_llm_outline() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/controlled-llm/reading-outline.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read fixture {}: {error}", path.display()));
        let mut outline: Value = serde_json::from_str(&raw).expect("fixture must be valid JSON");
        // `_comment` 只是给人看的说明，不参与协议。
        if let Some(object) = outline.as_object_mut() {
            object.remove("_comment");
        }
        outline
    }

    /// Task 27：受控模型服务 → **真实网关** → 比较 → 持久化。
    ///
    /// 与既有用例的分工必须分清，**两层证据不可互相代替**：
    /// - `stub_cloud_runner` 那类用例把云端结果**直接塞进注入点**，验证的是裁决逻辑；
    /// - 本用例验证**网关这一段本身**：真起一个 HTTP 服务、真发请求，真走
    ///   `openai_chat_content` → `parse_llm_json_content` → `validate_cloud_outline_output`
    ///   的解析与校验，再把解析结果交给核心裁决、落盘成候选与决策。
    ///
    /// 样本与预期：`fixtures/controlled-llm/reading-outline.json` +
    /// `fixtures/controlled-llm/expected-decisions.json`；前端可执行版本是
    /// `scripts/controlled-llm-service.mjs` 加一份指向它的 profile。
    ///
    /// 刻意**不**覆盖 `make_cloud_paper_generation_input` 的取原文与拼 prompt 部分——
    /// 那段依赖真实文件的抽取产物，属于导入链路的职责；本用例只给网关喂一份带
    /// `sourceText` 的输入。这个边界是明说的，不是省略。
    #[test]
    fn controlled_model_service_drives_candidates_through_the_real_gateway() {
        let outline = controlled_llm_outline();

        // 受控服务：应答体是标准 chat-completions 信封，`content` 里才是 outline JSON。
        let response_body = json!({
            "id": "controlled-llm-0001",
            "object": "chat.completion",
            "model": "controlled-outline-v1",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {"role": "assistant", "content": outline.to_string()}
            }]
        })
        .to_string();
        let base_url = spawn_controlled_model_service(response_body);

        let (root, job_id) = seed_bridge_job(
            &json!({
                "schemaVersion": "IeltsAuthoringIRV2",
                "exam": {"title": "t"},
                "taskGroups": [{
                    "taskId": "task-1",
                    "taskType": "sentence_completion",
                    "displayRange": {"kind":"range","start":14,"end":14},
                    "responseGroups": [{"responseGroupId":"rg-1","kind":"text_entry","slotIds":["slot-14"]}]
                }],
                "answerSlots": {
                    "slot-14": {"slotId":"slot-14","questionNumber":14,"interaction":"text","sourceAnchors":[]}
                },
                "answerKey": {"slot-14": {"kind":"unresolved"}},
                "quality": {"coverageStatus": {"unassignedSourceNodeIds": []}}
            }),
            &json!({"pages":[{"pageIndex":0,"lines":[{"text":"A passage about birds and weather."}]}]}),
        );

        // profile 指向受控服务。落盘格式与 `llm_profiles.rs::profiles_path` 一致。
        let profile = json!({
            "profileId": "controlled-outline",
            "name": "Controlled Outline Service",
            "provider": "OpenAiCompatible",
            "baseUrl": base_url,
            "model": "controlled-outline-v1",
            "temperature": 0,
            "timeoutMs": 60000,
            "forceJson": true,
            "enabled": true
        });
        assert!(
            crate::llm_profiles::save_profiles(&root, &[profile.clone()]).is_ok(),
            "profile 必须能落盘，否则网关取不到 baseUrl"
        );
        let profile = crate::llm_profiles::find_profile(&root, "controlled-outline")
            .expect("profile readable through the same path the gateway uses");

        // ── 1) 真实网关：真 HTTP + 真解析 + 真校验 ────────────────────────────
        let parsed = crate::llm_gateway::run_llm_gateway(
            &root,
            &job_id,
            "generate_pdf_reading_outline",
            &json!({
                "profile": profile,
                "sourceText": "A passage about birds and weather.",
            }),
            None,
        )
        .expect("受控服务必须能让真实网关成功返回");

        assert_eq!(
            parsed["groups"][0]["kind"].as_str(),
            outline["groups"][0]["kind"].as_str(),
            "网关解析后的题组类型必须与样本一致：{parsed}"
        );
        assert_eq!(
            parsed["groups"][0]["slots"][0]["answer"], outline["groups"][0]["slots"][0]["answer"],
            "网关解析后的答案值必须与样本一致（说明走的是解析路径，不是把样本直接透传）"
        );
        // 网关的观测记录：本命令确实调用过，且成功。这是「走的是真实网关」的旁证。
        let calls = std::fs::read_to_string(job_dir(&root, &job_id).join("llm-calls.jsonl"))
            .expect("网关必须留下 llm-calls.jsonl");
        assert!(
            calls.contains("generate_pdf_reading_outline") && calls.contains("\"ok\":true"),
            "网关调用记录里必须有本次成功的调用：{calls}"
        );

        // ── 2) 比较 + 持久化：把网关结果按调度器的方式交给核心裁决 ─────────────
        let cloud_value = parsed.clone();
        let report = run_recognition_cycle_core(
            &root,
            &job_id,
            Some("controlled-outline"),
            true,
            0,
            &move |_root, _job_id, _profile_id| Ok(cloud_value.clone()),
        )
        .expect("裁决必须跑完");

        let cloud_status = report["cloudCandidateStatus"].as_str().unwrap_or("");
        assert!(
            cloud_status == "succeeded" || cloud_status == "partial",
            "受控服务应答合法，云端链必须可用，实际为 {cloud_status}"
        );

        let batch_id = report["batchId"].as_str().expect("batchId present").to_string();
        let decision = read_decision_file(&root, &job_id, &batch_id)
            .expect("决策必须落盘（数据库与 job 目录各一份）");

        // 把逐项裁决投影成稳定形态，供人工审阅与后续固化为预期样本。
        let projection: Vec<Value> = decision
            .items
            .iter()
            .map(|item| {
                json!({
                    "decisionId": item.decision_id,
                    "targetId": item.target.target_id,
                    "field": item.field,
                    "resolution": item.resolution,
                    "reasonCode": item.reason_code,
                    "status": item.status,
                    "isActionable": item.is_actionable(),
                    "localValue": item.local_value,
                    "cloudValue": item.cloud_value,
                    "sourceValue": item.source_value,
                    "hasProposedPatch": item.proposed_patch.is_some(),
                    "autoApplyEligible": report["autoApplied"]
                        .as_array()
                        .map(|applied| applied.iter().any(|id| id == &json!(item.decision_id)))
                        .unwrap_or(false),
                })
            })
            .collect();
        let projection = json!({
            "cloudCandidateStatus": cloud_status,
            "localBaselineFrozen": report["localBaselineFrozen"],
            "items": projection,
        });

        // ── 3) 与预期样本逐字对齐 ──────────────────────────────────────────────
        //
        // 断言的是**投影**（稳定字段）而不是整份 item：内部字段增删不应该把用例变成噪音，
        // 但凡是前端看得见、或决定它能否被处理的东西（resolution / status / 待办 /
        // 有无补丁 / 三个来源的值 / 是否能自动应用）都必须被钉住。
        let expected_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/controlled-llm/expected-decisions.json");
        let expected_raw = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", expected_path.display()));
        let mut expected: Value =
            serde_json::from_str(&expected_raw).expect("expected fixture must be valid JSON");
        if let Some(object) = expected.as_object_mut() {
            object.remove("_comment");
        }
        assert_eq!(
            projection, expected,
            "受控服务场景的实际结果与预期样本不一致；若这是有意的行为变更，请同步更新 \
             fixtures/controlled-llm/expected-decisions.json"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 内容变化后 resolution 失效（R3）：**修正写入之后用户又改了同一槽位**时，
    /// 原修正的前提已消失，视图不得再把它呈现为「已修正」，撤销也必须被拒绝。
    ///
    /// 两个出口各自独立断言，因为它们是两条防线：
    /// - 呈现层（`build_view`）：`Accepted` ⇒ `Superseded` + `USER_EDITED_AFTER_APPLY`；
    /// - 执行层（撤销命令）：即便前端不显示撤销入口，直接调用命令也必须被拒——
    ///   否则回滚会把用户的新改动一并覆盖。
    ///
    /// 反向对照见 `applied_resolution_stays_visible_while_the_slot_is_untouched`：
    /// 未改动时同一套代码必须仍然呈现为已修正，证明该规则不会「一有改动就整批作废」。
    #[test]
    fn applied_resolution_is_superseded_once_the_user_edits_the_slot() {
        // 权威稿当前值 != 修正写入的值 ⇒ 用户改过该槽位，原修正的前提已消失。
        // 用夹具选项库里的 **C**：既可被 `validate_authoring` 接受，也确实是「另一个答案」。
        let user_edit = json!({"kind":"option","labels":["C"],"assignment":"unordered_set"});
        let (root, item_id, batch_id, item) = seed_applied_batch("supersede", &user_edit);

        // 呈现层：不得再示为「已自动修正」，也不得变成一个「待办」。
        {
            let conn = open_library_connection(&root).expect("db");
            let row = store::load_batch_by_id(&conn, &batch_id).expect("load").expect("row");
            let items = store::load_decision_items(&conn, &batch_id).expect("items");
            let (canonical, _) = get_canonical_ds(&conn, &item_id).expect("ok").expect("present");
            let view = build_view(&row, items, 1, Some(&canonical));

            assert!(
                view.auto_applied.is_empty(),
                "槽位已被用户改动，这条修正不再生效，不得继续以「已自动修正」展示"
            );
            assert!(
                view.actionable.is_empty(),
                "失效项不是待办（用户已经自己处理了该槽位），不得要求他再确认一次"
            );
        }

        // 执行层防线：直接调用撤销命令也必须被拒绝。
        let refused = apply_recognition_decisions_core(
            &root,
            ApplyRecognitionDecisionsRequestV1 {
                request_id: "req-supersede-refused".to_string(),
                batch_id: batch_id.clone(),
                base_edit_version: 0,
                accept: vec![],
                reject: vec![],
                undo: vec![item.decision_id.clone()],
            },
        )
        .expect("命令本身不应报错，而应如实返回 failed 结果");

        assert_eq!(
            refused["outcomes"][0]["kind"].as_str(),
            Some("failed"),
            "用户已改动的槽位不得被回滚覆盖：{refused}"
        );
        assert_eq!(
            refused["outcomes"][0]["reasonCode"].as_str(),
            Some(reason::USER_EDITED_AFTER_APPLY),
            "必须给出精确原因码，便于前端解释为何不能撤销"
        );
        assert_eq!(
            q14_answer(&root, &item_id),
            user_edit,
            "被拒绝的撤销绝不能改动权威稿——用户的新改动必须原样保留"
        );

        drop(open_library_connection(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 上一条测试的反向对照：**目标未被改动**时，同一条「失效」规则不得误伤——
    /// 修正仍生效、仍呈现为已自动修正，且撤销照常放行。
    ///
    /// 没有这条对照，「失效规则」可以被实现成「只要编辑版本变了就整批作废」而测试全绿，
    /// 那正是要避免的过度失效。
    #[test]
    fn applied_resolution_stays_visible_while_the_slot_is_untouched() {
        // 权威稿当前值 == 修正写入的值 ⇒ 修正仍然生效。
        let (root, item_id, batch_id, item) = seed_applied_batch("intact", &applied_option());

        {
            let conn = open_library_connection(&root).expect("db");
            let row = store::load_batch_by_id(&conn, &batch_id).expect("load").expect("row");
            let items = store::load_decision_items(&conn, &batch_id).expect("items");
            let (canonical, _) = get_canonical_ds(&conn, &item_id).expect("ok").expect("present");
            // 编辑版本推进到 1（用户改了**别的**地方），但 q14 保持修正后的值。
            let view = build_view(&row, items, 1, Some(&canonical));
            assert_eq!(
                view.auto_applied.len(),
                1,
                "目标未被改动时，修正仍然生效，必须照常呈现为已自动修正"
            );
            assert!(view.actionable.is_empty(), "生效中的自动修正不是待办");
        }

        let undone = apply_recognition_decisions_core(
            &root,
            ApplyRecognitionDecisionsRequestV1 {
                request_id: "req-intact-undo".to_string(),
                batch_id: batch_id.clone(),
                base_edit_version: 0,
                accept: vec![],
                reject: vec![],
                undo: vec![item.decision_id.clone()],
            },
        )
        .expect("undo must run");
        assert_eq!(
            undone["outcomes"][0]["kind"].as_str(),
            Some("undone"),
            "目标未被改动时撤销必须放行（否则该规则会误伤正常撤销）：{undone}"
        );

        drop(open_library_connection(&root));
        let _ = std::fs::remove_dir_all(&root);
    }
}
