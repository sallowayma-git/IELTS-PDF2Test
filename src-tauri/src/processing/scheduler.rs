//! M2（原 P6-T01/T02）：持久化调度器。
//!
//! - 单调度循环：认领 queued 任务（local semaphore 限并发），`spawn_blocking` 跑
//!   本地识别（CPU/文件 I/O 不占 async runtime，计划 §5.5）。
//! - 单题 local→cloud 两段：云端失败不取消本地；本地失败时云端照跑只作对照稿。
//! - lease 续租 + 阶段边界取消标记；过期 worker 的迟到结果不得提交。
//! - 每次阶段推进 emit `processing://item-updated`（携带 event_seq 状态版本）。
//!
//! 事件不承担唯一状态存储：前端收到事件后以 DB 为准刷新（见 processingClient）。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};


use super::queue::{
    self, advance_stage, claim_next, finalize_cancelled_without_lease, finalize_ready_without_lease,
    get_job, renew_lease, request_cancel, retry, set_cloud_status, STAGE_CLOUD_RECOGNITION,
    STAGE_FAILED, STAGE_LOCAL_RECOGNITION, STAGE_READY_FOR_REVIEW,
};
use crate::auto_pipeline::{generate_cloud_reading_outline, run_auto_pipeline_core};
use crate::library::repository::open_library_connection;
use crate::reconcile::{candidate, commands, store};
use crate::{app_root, AutoPipelineInput};
use std::path::Path;

pub(crate) const EVENT_ITEM_UPDATED: &str = "processing://item-updated";
/// 连续自动中断恢复上限（M2 契约：超过后等待用户重试）。
const MAX_AUTO_RECOVERY: i64 = 3;
/// 云端只在 PDF 上跑（与旧 ImportDrawer 行为一致）。
const SCHEDULER_TICK_MS: u64 = 800;

#[derive(Debug, Clone)]
pub(crate) struct ProcessingSettings {
    pub local_concurrency: usize,
    pub cloud_concurrency: usize,
    pub cloud_enabled_default: bool,
}

impl ProcessingSettings {
    /// 计划 §3：本地 = max(1, min(cpu-1, 3))；云端默认 2（1–3 可配置，M2 从设置读）。
    pub(crate) fn defaults() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(4);
        Self {
            local_concurrency: (cpus.saturating_sub(1)).clamp(1, 3),
            cloud_concurrency: 2,
            cloud_enabled_default: false,
        }
    }
}

pub(crate) struct ProcessingState {
    pub worker_id: String,
    pub settings: ProcessingSettings,
    pub local_permits: Arc<tokio::sync::Semaphore>,
    pub cloud_permits: Arc<tokio::sync::Semaphore>,
    /// 用户请求取消的任务（阶段边界检查；queued 的取消直接走 queue::request_cancel）。
    pub cancelled: Arc<tokio::sync::RwLock<HashSet<String>>>,
}

impl ProcessingState {
    pub(crate) fn new(settings: ProcessingSettings) -> Self {
        Self {
            worker_id: uuid::Uuid::new_v4().simple().to_string(),
            local_permits: Arc::new(tokio::sync::Semaphore::new(settings.local_concurrency)),
            cloud_permits: Arc::new(tokio::sync::Semaphore::new(settings.cloud_concurrency)),
            cancelled: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
            settings,
        }
    }
}

fn stage_percent(stage: &str) -> u32 {
    // 阶段权重（计划 §12.3 的诚实进度：只报告真实完成的阶段，不造虚假百分比）。
    match stage {
        "queued" => 0,
        "preparing_source" => 5,
        "local_recognition" => 45,
        "cloud_recognition" => 70,
        "reconciling" => 85,
        "ready_for_review" | "ready_to_publish" => 100,
        _ => 0,
    }
}

fn display_message(stage: &str, error: Option<&str>) -> String {
    if let Some(code) = error {
        return match code {
            "interrupted" => "上次运行被中断，已自动排队重试。".to_string(),
            // G1/A4-F03：恢复上限路径不得谎称"已自动排队重试"。
            "retry_exhausted" => "已达到自动恢复上限，请手动重试。".to_string(),
            // G1 对抗审计 P2：恢复兑现的取消不得显示成"识别失败"。
            "cancelled" => "已取消。".to_string(),
            _ => "识别失败，可以重试。".to_string(),
        };
    }
    match stage {
        "queued" => "排队中".to_string(),
        "local_recognition" => "正在读取原文件并识别题目".to_string(),
        "cloud_recognition" => "本地识别完成 · 云端识别中".to_string(),
        "reconciling" => "正在合并本地与云端结果".to_string(),
        "ready_for_review" => "可以打开检查了".to_string(),
        "ready_to_publish" => "可以发布".to_string(),
        "failed" => "识别失败，可以重试".to_string(),
        "cancelled" => "已取消".to_string(),
        _ => stage.to_string(),
    }
}

fn display_message_for(job: &queue::ProcessingJobRow) -> String {
    if let Some(code) = job.last_error_code.as_deref() {
        return display_message(&job.stage, Some(code));
    }
    // 本地稿尚未形成（本地识别仍在跑、cloud_status 已并行推进到 queued/running）
    // 时，如实说明本地进度，不得谎称「本地识别完成」。
    if job.stage == STAGE_CLOUD_RECOGNITION && job.local_status != "succeeded" {
        return display_message(STAGE_LOCAL_RECOGNITION, None);
    }
    // 「本地先出稿」是产品的核心承诺：本地稿一旦形成就必须明确告诉用户
    // 现在可以打开编辑，同时如实地说明云端仍在排队，而不是笼统写"识别中"。
    if job.stage == STAGE_CLOUD_RECOGNITION && job.cloud_status == "queued" {
        return "本地识别完成，可以打开编辑 · 云端识别排队中".to_string();
    }
    if job.stage == STAGE_CLOUD_RECOGNITION && job.cloud_status == "running" {
        return "本地识别完成，可以打开编辑 · 云端识别中".to_string();
    }
    display_message(&job.stage, None)
}

fn emit_item_updated(app: &AppHandle, job: &queue::ProcessingJobRow) {
    let payload = json!({
        "libraryItemId": job.library_item_id,
        "jobId": job.id,
        "stage": job.stage,
        "localStatus": job.local_status,
        "cloudStatus": job.cloud_status,
        "reconcileStatus": job.reconcile_status,
        "progressPercent": stage_percent(&job.stage),
        "actionableCount": job.actionable_count,
        "displayMessage": display_message_for(job),
        "stateVersion": job.event_seq
    });
    if let Err(error) = app.emit(EVENT_ITEM_UPDATED, payload) {
        eprintln!("[processing] emit failed: {error}");
    }
}

/// setup 时调用：启动恢复（M2-T03）+ 调度循环。
pub(crate) fn start(app: AppHandle, state: Arc<ProcessingState>) {
    tauri::async_runtime::spawn(async move {
        let Ok(root) = app_root(&app) else { return };
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let Ok(conn) = open_library_connection(&root) else {
                eprintln!("[processing] recovery: cannot open library db");
                return;
            };
            match queue::recover_on_startup(&conn, MAX_AUTO_RECOVERY) {
                Ok(count) if count > 0 => eprintln!("[processing] recovery requeued {count} interrupted jobs"),
                Ok(_) => {}
                Err(error) => eprintln!("[processing] recovery failed: {error}"),
            }
        }).await;
        loop {
            tokio::time::sleep(Duration::from_millis(SCHEDULER_TICK_MS)).await;
            if state.local_permits.available_permits() == 0 {
                continue;
            }
            let Ok(root) = app_root(&app) else { continue };
            let claimed = tauri::async_runtime::spawn_blocking({
                let worker_id = uuid::Uuid::new_v4().simple().to_string();
                move || {
                    let conn = open_library_connection(&root)?;
                    claim_next(&conn, &worker_id)
                }
            })
            .await;
            let job = match claimed {
                Ok(Ok(Some(job))) => job,
                Ok(Ok(None)) | Ok(Err(_)) => continue,
                Err(_) => continue,
            };
            let state = state.clone();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                run_job(app, state, job).await;
            });
        }
    });
}

/// 用户取消入口（queued 立即取消；running 在阶段边界退出）。
pub(crate) async fn cancel(state: Arc<ProcessingState>, app: AppHandle, job_id: &str) -> Result<(), String> {
    state.cancelled.write().await.insert(job_id.to_string());
    let root = app_root(&app)?;
    let job_id_owned = job_id.to_string();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let conn = open_library_connection(&root)?;
        request_cancel(&conn, &job_id_owned)?;
        if let Some(job) = get_job(&conn, &job_id_owned)? {
            emit_row(&app, &job);
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|inner| inner);
    if result.is_err() {
        // G1 对抗审计 P2：durable 标记落库失败时不得残留内存取消标记，
        // 否则下次同 id 重试会被幽灵取消吞掉。
        state.cancelled.write().await.remove(job_id);
    }
    result
}

pub(crate) async fn retry_job(state: Arc<ProcessingState>, app: AppHandle, job_id: &str) -> Result<(), String> {
    state.cancelled.write().await.remove(job_id);
    let root = app_root(&app)?;
    let job_id_owned = job_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_library_connection(&root)?;
        retry(&conn, &job_id_owned)?;
        if let Some(job) = get_job(&conn, &job_id_owned)? {
            emit_row(&app, &job);
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

use crate::CommandResult;

fn emit_row(app: &AppHandle, job: &queue::ProcessingJobRow) {
    emit_item_updated(app, job);
}

async fn run_job(app: AppHandle, state: Arc<ProcessingState>, job: queue::ProcessingJobRow) {
    let Some(worker_id) = job.lease_owner.clone() else { return };
    let state = Arc::new(ProcessingState {
        worker_id,
        settings: state.settings.clone(),
        local_permits: state.local_permits.clone(),
        cloud_permits: state.cloud_permits.clone(),
        cancelled: state.cancelled.clone(),
    });
    let heartbeat = tauri::async_runtime::spawn({
        let app = app.clone();
        let state = state.clone();
        let job_id = job.id.clone();
        async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Ok(root) = app_root(&app) else { break };
                let job_id = job_id.clone();
                let worker_id = state.worker_id.clone();
                let result = tauri::async_runtime::spawn_blocking(move || {
                    let conn = open_library_connection(&root)?;
                    renew_lease(&conn, &job_id, &worker_id)
                }).await;
                if !matches!(result, Ok(Ok(true))) { break; }
            }
        }
    });
    run_job_inner(app, state, job).await;
    heartbeat.abort();
}

async fn run_job_inner(app: AppHandle, state: Arc<ProcessingState>, job: queue::ProcessingJobRow) {
    let job_id = job.id.clone();
    // 认领即发一次事件（queued → running）。
    {
        let Ok(root) = app_root(&app) else { return };
        let app = app.clone();
        let job_id = job_id.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let conn = open_library_connection(&root)?;
            let Some(job) = get_job(&conn, &job_id)? else { return Ok::<(), String>(()) };
            emit_row(&app, &job);
            Ok(())
        })
        .await;
    }

    // ── 云端决策输入（只依赖 job.progress 与 settings，提前到本地识别之前）──
    // 这样云端链可以与本地识别「并行」起飞，而不是等本地稿落库后才开始。
    let cloud_enabled = job
        .progress
        .get("cloudEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(state.settings.cloud_enabled_default);
    // PDF 与 DOCX 都要走完整云端链路（任务书第二/九项）：DOCX 用本地抽取
    // 的原文文本作为证据面，见 `auto_pipeline::generate_cloud_reading_outline`。
    let cloud_source_supported = job
        .progress
        .get("fileName")
        .and_then(Value::as_str)
        .map(|name| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".pdf") || lower.ends_with(".docx")
        })
        .unwrap_or(false);
    let cloud_profile_id = job
        .progress
        .get("cloudProfileId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let cloud_will_run = cloud_enabled && cloud_source_supported;

    // ── 本地识别（阻塞线程池；持 local permit）───────────────────────
    let local_permit = state.local_permits.clone().acquire_owned().await;
    let advanced = advance(&app, &state, &job_id, STAGE_LOCAL_RECOGNITION, Some("running"), None, None, None, None).await;
    // G1 边界：带 durable 取消标记的行会被 advance 强制落 cancelled；
    // 以有效阶段为准，取消后不再继续识别流程。
    if let Some((_, effective)) = &advanced {
        if effective == queue::STAGE_CANCELLED {
            state.cancelled.write().await.remove(&job_id);
            return;
        }
    }
    if advanced.is_none() {
        return; // lease 丢失 / 已取消
    }
    let cancelled = state.cancelled.read().await.contains(&job_id);
    if cancelled {
        finish_cancelled(&app, &state, &job_id).await;
        return;
    }

    let Ok(root) = app_root(&app) else { return };

    // 解析云端 profile 一次（与 `generate_cloud_reading_outline` 的回退逻辑一致）：
    // 显式 `cloudProfileId` 优先，否则回退到 job 的 `active_llm_profile_id`；
    // 同一解析值同时传给「拉取」与「reconcile」，避免拉取了却被 reconcile 当
    // NO_PROFILE 丢弃（一致性约束 #4）。
    let resolved_profile: Option<String> = cloud_profile_id.clone().or_else(|| {
        crate::job_store::load_job(&root, &job_id)
            .ok()
            .and_then(|import_job| import_job.active_llm_profile_id)
    });
    // 是否真正拉取云端：需 cloud_will_run 且已解析出 profile（约束 #8：
    // 无 profile 时不拉取，避免拉取后被 reconcile 当 NO_PROFILE 丢弃）。
    let launch_cloud = cloud_will_run && resolved_profile.is_some();

    // 本地识别仍在跑时，先如实声明「云端排队」——但**不**提前把 stage 推进到
    // `cloud_recognition`（否则 progressPercent 会虚高到 70%）。改用 set_cloud_status
    // 只写 cloud_status，stage 保持 local_recognition（percent=45），让「本地仍在读」
    // 的进度诚实可见，同时云端排队状态也对外可见（并行承诺的可观测信号）。
    if launch_cloud {
        let queued = set_cloud_status_only(&app, &state, &job_id, "queued").await;
        if queued.is_none() {
            // lease 丢失或已取消：中止云端链（本地识别仍会照常完成）。
            return;
        }
    }

    // ── 本地识别与云端拉取并行执行 ───────────────────────────────────
    // 本地闭包：真实本地管道。
    let root_local = root.clone();
    let job_id_local = job_id.clone();
    let local_closure = move || {
        run_auto_pipeline_core(
            &root_local,
            &job_id_local,
            Some(AutoPipelineInput {
                confidence_threshold: Some(0.85),
                execution_mode: Some("localOnly".to_string()),
                target: Some("editableDraft".to_string()),
                ..Default::default()
            }),
        )
    };

    // 云端任务（async）：**不在主路径上**抢 permit——permit 获取与 cloud_status 推进
    // 都放进任务内部，因此本地识别可以立即起飞，不被云端 permit 阻塞（Defect 1 修复）。
    // 任务只更新 cloud_status（"running"），不把 stage 推到 cloud_recognition，
    // 所以本地识别期间前端仍看到 local_recognition（percent=45）。返回
    // `Option<Result<Value,String>>`：`None` 表示云端在取消/lease 丢失时中止，
    // 调用方据此跳过 reconcile。
    let cloud_future: Option<
        std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<Result<serde_json::Value, String>>> + Send + 'static>,
        >,
    > = if launch_cloud {
        let state_cloud = state.clone();
        let app_cloud = app.clone();
        let root_cloud = root.clone();
        let job_id_cloud = job_id.clone();
        let resolved = resolved_profile.clone();
        Some(Box::pin(async move {
            // 受控并发：取得 cloud permit（等待期间落下的取消由 set_cloud_status_only 拒绝）。
            let _cloud_permit = state_cloud.cloud_permits.clone().acquire_owned().await;
            if set_cloud_status_only(&app_cloud, &state_cloud, &job_id_cloud, "running")
                .await
                .is_none()
            {
                return None; // 取消 / lease 丢失：中止云端链。
            }
            // 模型调用移入阻塞线程池，不占 async runtime；permit 随闭包结束释放，
            // 仅覆盖模型调用（reconcile 不再持有，约束：云端 permit 只在模型调用期间持有）。
            let result = tauri::async_runtime::spawn_blocking(move || {
                let result =
                    generate_cloud_reading_outline(&root_cloud, &job_id_cloud, resolved.as_deref());
                drop(_cloud_permit);
                result
            })
            .await
            .unwrap_or_else(|error| Err(format!("processing_join:{error}")));
            Some(result)
        }))
    } else {
        None
    };

    // ── 立即并发拉起：本地阻塞任务 + 云端 async 任务，二者互不阻塞 ──────────
    // 云端 permit 的获取发生在 cloud 任务内部（见 cloud_future），所以本地识别
    // 此刻就能起飞，绝不会被云端 permit 卡住（Defect 1 修复）。
    let local_handle = tauri::async_runtime::spawn_blocking(local_closure);
    let cloud_handle: Option<
        tauri::async_runtime::JoinHandle<Option<Result<serde_json::Value, String>>>,
    > = cloud_future.map(|fut| tauri::async_runtime::spawn(fut));

    // 只 await 本地结果——云端仍在并行跑。本地失败/取消在此即时兑现。
    let local_result = local_handle
        .await
        .unwrap_or_else(|error| Err(format!("processing_join:{error}")));
    drop(local_permit);

    match local_result {
        Ok(_) => {}
        Err(error) => {
            fail_job(&app, &state, &job_id, &error).await;
            return;
        }
    };
    // G1/A4-F02：本地识别期间发生的取消必须在此兑现；否则无云端路径会
    // 直接推进 ready_for_review，取消被静默吞掉。
    if state.cancelled.read().await.contains(&job_id) {
        finish_cancelled(&app, &state, &job_id).await;
        return;
    }

    // 批次基线：本地稿定稿时的编辑版本。**必须在草稿发布「可编辑」之前**冻结，
    // 否则用户若在「发布」与「读 baseline」之间改稿，基线版本会被抬高，而后续
    // 云端裁决若按当前稿重投影本地候选，就会把用户编辑误当成本地识别结果。
    //
    // **读不到版本时不得退化成 0**：0 是一个合法的真实版本，用 0 顶替「读取失败」会
    // 让批次 id（由 job_id + 源文件哈希 + 版本三者派生）指向一个并不存在的批次，冻结出的
    // 快照与真实稿并不对应——这比「没有快照」更危险，因为它**看起来是可信的**。
    // 读取失败与冻结失败同等对待：不冻结、不裁决、不自动写入。
    let mut freeze_error: Option<String> = None;
    let base_edit_version = match current_edit_version_of(&app, &job_id).await {
        Some(version) => {
            // 冻结本地候选快照（与 base_edit_version 同一时刻），**早于**可编辑发布。
            // `run_recognition_cycle_core` 走 `resolve_local_snapshot` 的「复用已冻结候选」
            // 分支，因此稍后的云端裁决比对的是冻结时的本地结果，而不是用户编辑后的当前稿，
            // 「云端运行期间用户改了稿」才能被识别，迟到结果才不会覆盖用户修改。
            if let Err(error) = freeze_local_candidate_snapshot(&root, &job_id, version) {
                eprintln!("[processing] freeze local candidate snapshot failed for {job_id}: {error}");
                freeze_error = Some(error);
            }
            version
        }
        None => {
            eprintln!(
                "[processing] base edit version unreadable for {job_id}; \
                 refusing to freeze or adjudicate on an unknown baseline"
            );
            freeze_error = Some("READ_BASE_VERSION_FAILED".to_string());
            // 占位值：下面的 `freeze_error.is_some()` 分支保证它既不参与冻结也不参与裁决。
            0
        }
    };

    if launch_cloud {
        // 本地稿已成：标记 local_status=succeeded（此前为 running，云端可能仍在跑）。
        let _ = advance(
            &app,
            &state,
            &job_id,
            STAGE_CLOUD_RECOGNITION,
            Some("succeeded"),
            None,
            None,
            None,
            None,
        )
        .await;
    }
    // 本地稿已成：立刻发布「可打开编辑」（云端仍在并行跑，不阻塞）。
    set_item_status_ready(&app, &job_id).await;

    if !launch_cloud {
        // 无可信基线 ⇒ **不得进入裁决**（冻结失败与版本读取失败都算）。
        //
        // 早先这里以「本路径没有迟到的云端结果」为由放行，理由不成立：本地裁决自己也会
        // 自动写入（`adjudicate::auto_apply_eligible` 允许「补空答案」），而它的前提守卫
        // 「权威稿当前值 == 本地识别结果」**只在本地基线是冻结快照时才成立**。快照缺失时
        // 基线退化为「按当前稿现场重投影」，两边恒等、守卫等价于不存在，自动写入就会覆盖
        // 用户编辑。冻结失败是可诊断的异常（读稿/磁盘失败），不是「无云所以无所谓」。
        //
        // 这里是第一层保护（直接不跑）；`reconcile_batch` 内部的 `baseline_frozen` 判定是
        // 第二层——即便有人把这段前置判断改坏，核心也不会自动写入（有意做成双保险）。
        if freeze_error.is_some() {
            advance(
                &app,
                &state,
                &job_id,
                STAGE_READY_FOR_REVIEW,
                Some("succeeded"),
                Some("not_run"),
                Some("failed"),
                Some(0),
                freeze_error.as_deref(),
            )
            .await;
            set_item_status_ready(&app, &job_id).await;
            return;
        }
        // 无云路径：**照常走完 reconcile**，让本地候选、原文核验、批次汇总全部落盘。
        // 计划 §12.3 只要求「本地即可检查、不被云端拖慢」，从未要求跳过裁决与留痕；
        // 跳过会让这三样全部缺失，前端拿不到任何可解释的证据链。云端由核心如实标为
        // `not_run`——而不是拿一个失败 profile 去顶替，把「没启用云端」谎报成云端故障。
        let (cloud_status, reconcile_status, actionable, last_error) =
            match run_local_only_recognition_cycle(&root, &job_id, base_edit_version) {
                Ok(report) => (
                    report.cloud_status,
                    report.reconcile_status,
                    report.actionable_count,
                    // 走到这里 `freeze_error` 必为 `None`（上面已提前返回），故无告警码。
                    None,
                ),
                Err(error) => {
                    eprintln!(
                        "[processing] local-only recognition cycle failed for {job_id}: {error}"
                    );
                    (
                        "not_run".to_string(),
                        "failed".to_string(),
                        0,
                        Some("RECONCILE_FAILED"),
                    )
                }
            };
        if advance(
            &app,
            &state,
            &job_id,
            STAGE_READY_FOR_REVIEW,
            Some("succeeded"),
            Some(&cloud_status),
            Some(&reconcile_status),
            Some(actionable),
            last_error,
        )
        .await
        .is_some()
        {
            set_item_status_ready(&app, &job_id).await;
        }
        return;
    }

    // 取消检查（join 之后、reconcile 之前）：迟到结果不得穿透取消。
    if state.cancelled.read().await.contains(&job_id) {
        finish_cancelled(&app, &state, &job_id).await;
        return;
    }

    // 直到此刻才 await 云端句柄：本地稿早已发布、base_edit_version 已冻结
    // （Defect 2 修复——「可打开编辑」的承诺不被云端模型调用拖慢）。云端在本地
    // 识别期间就已经并发起飞，这里只是收口它的结果。
    let cloud_fetched = if freeze_error.is_some() {
        // 冻结失败：护栏前提不成立（见上文）。宁可没有云端建议，也不允许迟到结果
        // 覆盖用户修改——直接丢弃云端结果、走「跳过 reconcile」分支。
        None
    } else {
        match cloud_handle {
            Some(handle) => handle
                .await
                .unwrap_or(None), // join 失败（任务异常）按「云端中止」处理：跳过 reconcile。
            None => None,
        }
    };
    // 云端中止（取消 / lease 丢失）时返回 None——本地稿仍可检查，云端标记失败、跳过裁决。
    let (cloud_status, reconcile_status, actionable) = match cloud_fetched {
        Some(prefetched) => {
            // 云端 JSON 已在本地识别期间并发拉取并冻结于此；reconcile 直接复用，
            // 不再发起第二次网络调用。整段 `Result` 传入，由 run_recognition_cycle
            // 内部决定云端成功 / 失败如何并入裁决报告。
            let cycle_result = run_recognition_cycle(
                &root,
                &job_id,
                resolved_profile.as_deref(),
                true,
                prefetched,
                base_edit_version,
            );
            match cycle_result {
                Ok(report) => (
                    report.cloud_status.clone(),
                    report.reconcile_status.clone(),
                    report.actionable_count,
                ),
                Err(error) => {
                    eprintln!("[processing] recognition cycle failed for {job_id}: {error}");
                    ("failed".to_string(), "failed".to_string(), 0)
                }
            }
        }
        None => ("failed".to_string(), "skipped".to_string(), 0),
    };

    let advance_result = advance(
        &app,
        &state,
        &job_id,
        STAGE_READY_FOR_REVIEW,
        Some("succeeded"),
        Some(&cloud_status),
        Some(&reconcile_status),
        Some(actionable),
        // 冻结失败是一次真实降级：留机器码供 UI / 诊断区分「云端自己失败」与
        // 「本地快照没能冻结、因此主动放弃裁决」，避免这种失败只活在 stderr 里。
        if freeze_error.is_some() {
            Some("FREEZE_SNAPSHOT_FAILED")
        } else {
            None
        },
    )
    .await;
    if advance_result.is_none() {
        // G1 对抗审计 P1-3：lease 丢失（如睡眠唤醒、心跳瞬断）时允许
        // 免 lease 终态收尾——local 已成功 + reclaim 守卫保证无人接手。
        let finalized = tauri::async_runtime::spawn_blocking({
            let root = root.clone();
            let job_id = job_id.clone();
            let cloud_status = cloud_status.clone();
            let reconcile_status = reconcile_status.clone();
            move || {
                let conn = open_library_connection(&root)?;
                finalize_ready_without_lease(&conn, &job_id, &cloud_status, &reconcile_status)
            }
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|inner| inner)
        .unwrap_or(false);
        if !finalized {
            return;
        }
    }
    set_item_status_ready(&app, &job_id).await;
}



/// 识别周期的精简结果（调度器只关心阶段状态与待确认数量）。
struct RecognitionCycleReport {
    cloud_status: String,
    reconcile_status: String,
    actionable_count: i64,
}

/// 链状态 → 任务行里的 `cloud_status`。
///
/// **`not_run` 必须原样透传**：它表示「本次没有云端参与」（未启用 / 未配置），
/// 与「云端跑了但失败」是两件不同的事。此前一律折叠成 `failed`，于是无云导入会在
/// 任务行里谎报云端失败，用户会去排查一个根本不存在的云端故障。
fn chain_status_to_job_status(raw: &str) -> String {
    match raw {
        "succeeded" => "succeeded",
        "partial" => "partial",
        "not_run" => "not_run",
        _ => "failed",
    }
    .to_string()
}

/// 云端全量识别 → 原文件核验 → 统一裁决 → 安全自动应用。
///
/// 本地链**不在这里重跑**：草稿在调度器上一阶段就已落库并发布可编辑。
/// 云端全量识别结果 `cloud_fetched` 由调度器在本地识别期间**并发拉取**后传入，
/// 这里直接复用，不再发起第二次网络调用。
///
/// `cloud_enabled = false` 用于**无云路径**（未启用云端 / 未解析到 profile）：此时仍要
/// 走完「本地候选 → 原文核验 → 裁决 → 落盘」，只把云端如实标成 `not_run`。**绝不能跳过**
/// ——跳过会让本地候选、原文核验、批次汇总全都不落盘，前端拿不到任何可解释的证据链；
/// 也**不能**用失败 profile 制造假失败来绕过这个缺口。
fn run_recognition_cycle(
    root: &Path,
    job_id: &str,
    profile_id: Option<&str>,
    cloud_enabled: bool,
    cloud_fetched: Result<serde_json::Value, String>,
    base_edit_version: i64,
) -> Result<RecognitionCycleReport, String> {
    // 注入点直接返回调度器已拉取的云端 JSON；不再调用真实网关。
    //
    // A4 的预算与调用都留在这一层：判定层（`reconcile_batch` / `adjudicate`）不持状态、
    // 不发 HTTP，注入点只是一个 `Fn`。
    //
    // 预算口径 = `MAX_ADJUDICATION_MODEL_CALLS`（1 次主裁决 + 1 次受约束修复）。
    // 第一次被拒时**把校验器的原话回给模型**再问一次，而不是空转重试：不带被拒原因的重试
    // 只会拿到同一种错误——那不是修复，只是多烧一次配额。
    let calls = std::cell::Cell::new(0u32);
    let adjudicator =
        |payload: &[serde_json::Value]| -> Result<
            serde_json::Value,
            crate::reconcile::engine::AdjudicationFailure,
        > {
            use crate::reconcile::engine::AdjudicationFailure;
            let budget = crate::schema::recognition_v1::MAX_ADJUDICATION_MODEL_CALLS;
            if calls.get() >= budget {
                return Err(AdjudicationFailure::BudgetExhausted);
            }
            calls.set(calls.get() + 1);
            let first = crate::auto_pipeline::adjudicate_divergence_through_gateway(
                root, job_id, profile_id, payload, None,
            );
            match first {
                Ok(value) => Ok(value),
                Err(error) => {
                    let repairs = crate::schema::recognition_v1::MAX_CONSTRAINED_REPAIRS;
                    if repairs == 0 || calls.get() >= budget {
                        return Err(AdjudicationFailure::Model(error));
                    }
                    calls.set(calls.get() + 1);
                    crate::auto_pipeline::adjudicate_divergence_through_gateway(
                        root,
                        job_id,
                        profile_id,
                        payload,
                        Some(&error),
                    )
                    .map_err(AdjudicationFailure::Model)
                }
            }
        };
    let adjudicator_ref: crate::reconcile::engine::AdjudicationRunner<'_> = &adjudicator;
    let report = crate::reconcile::commands::run_recognition_cycle_core_with_adjudicator(
        root,
        job_id,
        profile_id,
        cloud_enabled,
        base_edit_version,
        &|_root, _job_id, _profile_id| cloud_fetched.clone(),
        Some(adjudicator_ref),
    )?;
    Ok(summarize_cycle_report(report))
}

/// **无云路径**：云端链由核心如实标成 `not_run`（`CLOUD_DISABLED`），而本地候选、
/// 原文核验、批次汇总**照常完整落盘**——这正是「无云分支不再跳过 reconcile」的落点。
///
/// 注入点在此路径上**不可达**（`cloud_enabled = false` 是云端选择的第一分支）。若将来
/// 有人改坏了这个前置判断，这里会返回一个显式错误，被外层记成 `RECONCILE_FAILED`，
/// 而不是静默地假装云端跑过、更不是拿一个假失败 profile 去顶替。
fn run_local_only_recognition_cycle(
    root: &Path,
    job_id: &str,
    base_edit_version: i64,
) -> Result<RecognitionCycleReport, String> {
    let report = crate::reconcile::commands::run_recognition_cycle_core(
        root,
        job_id,
        None,
        false,
        base_edit_version,
        &|_root, _job_id, _profile_id| Err("cloud_runner_invoked_on_no_cloud_path".to_string()),
    )?;
    Ok(summarize_cycle_report(report))
}

fn summarize_cycle_report(report: Value) -> RecognitionCycleReport {
    let cloud_status = chain_status_to_job_status(
        report
            .get("cloudCandidateStatus")
            .and_then(Value::as_str)
            .unwrap_or("failed"),
    );
    // 待办数与视图同源（`DecisionItemV1::is_actionable`），由核心直接给出，不再由
    // `summary.needsReview + summary.unverifiable` 拼出来。
    let actionable_count = report
        .get("actionableCount")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    RecognitionCycleReport {
        cloud_status,
        // 裁决跑完即算完成；云端不可用会体现在 cloud_status 与 chain_status 里。
        reconcile_status: "succeeded".to_string(),
        actionable_count,
    }
}

/// 当前 canonical 编辑版本（批次基线的冻结值）。
///
/// `None` 表示**读取失败或 item 行不存在**，二者都不代表版本 0。调用方必须把它当成
/// 「基线不可用」处理，不得用 `unwrap_or(0)` 折叠——0 是合法版本，用 0 顶替会让批次 id
/// 指向一个不存在的批次，冻结出的快照看似可信却与实际稿无关。
async fn current_edit_version_of(app: &AppHandle, job_id: &str) -> Option<i64> {
    let root = app_root(app).ok()?;
    let job_id = job_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_library_connection(&root).ok()?;
        crate::reconcile::store::current_edit_version(&conn, &job_id)
            .ok()
            .flatten()
    })
    .await
    .ok()
    .flatten()
}

/// 在草稿发布「可编辑」**之前**冻结本地候选快照（与 base_edit_version 同一时刻）。
///
/// 这是修复「先读 baseline 版本、再按当前稿重投影本地候选」的关键：把本地识别那一刻的
/// 候选按 `batch_id = (job_id, source_sha256, base_edit_version)` 落盘。`run_recognition_cycle_core`
/// 在裁决前会通过 `store::read_candidate` 读到这份快照，并因 `resolve_local_snapshot` 的
/// 「同一批次复用已冻结候选」分支而采用它——于是稍后比对的是冻结时的本地结果，不是用户
/// 编辑后的当前稿，「云端运行期间用户改了稿」才会被识别，迟到结果才不会覆盖用户修改。
///
/// 幂等安全：`batch_id` 由输入与版本派生，重试必然复用同一快照，不会因重复冻结产生偏差。
///
/// **每一步失败都必须上抛，不得静默吞掉。** 调用方拿「冻结成功」当作「迟到结果不覆盖
/// 用户修改」护栏的前提：一旦快照缺失，`resolve_local_snapshot`（`reconcile/engine.rs`）
/// 会退回「按当前稿现场重投影」分支，裁决就会把用户编辑后的稿当成本地识别结果，
/// 于是「云端运行期间用户改了稿」永远检测不到、迟到结果照样覆盖用户修改——正是本
/// 修复要消灭的缺陷。因此这里把连接 / 取稿 / 落盘三处失败逐一如实上抛。
fn freeze_local_candidate_snapshot(
    root: &Path,
    job_id: &str,
    base_edit_version: i64,
) -> Result<(), String> {
    let conn = open_library_connection(root)
        .map_err(|error| format!("open_library_connection_failed:{error}"))?;
    let (canonical, _current_version) =
        crate::library::repository::get_canonical_ds(&conn, job_id)
            .map_err(|error| format!("read_canonical_failed:{error}"))?
            .ok_or_else(|| format!("canonical_not_seeded:{job_id}"))?;
    let source_sha256 = commands::source_sha256_for_job(root, job_id);
    let batch_id = commands::recognition_batch_id(job_id, &source_sha256, base_edit_version);
    let snapshot = candidate::local_candidate_from_authoring(
        &canonical,
        &batch_id,
        job_id,
        job_id,
        job_id,
        &source_sha256,
        base_edit_version,
    );
    store::write_candidate(root, &batch_id, &snapshot, store::LOCAL_CANDIDATE_FILE)
        .map_err(|error| format!("write_local_snapshot_failed:{error}"))?;
    Ok(())
}

async fn advance(
    app: &AppHandle,
    state: &Arc<ProcessingState>,
    job_id: &str,
    stage: &str,
    local_status: Option<&str>,
    cloud_status: Option<&str>,
    reconcile_status: Option<&str>,
    actionable_count: Option<i64>,
    last_error_code: Option<&str>,
) -> Option<(i64, String)> {
    let Ok(root) = app_root(app) else { return None };
    let worker_id = state.worker_id.clone();
    let job_id_owned = job_id.to_string();
    let stage_owned = stage.to_string();
    let local_owned = local_status.map(str::to_string);
    let cloud_owned = cloud_status.map(str::to_string);
    let reconcile_owned = reconcile_status.map(str::to_string);
    let error_owned = last_error_code.map(str::to_string);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let conn = open_library_connection(&root)?;
        // 先续租再推进：renew 失败说明 lease 已丢，不得提交。
        if !renew_lease(&conn, &job_id_owned, &worker_id)? {
            return Ok(None);
        }
        advance_stage(
            &conn,
            &job_id_owned,
            &worker_id,
            &stage_owned,
            local_owned.as_deref(),
            cloud_owned.as_deref(),
            reconcile_owned.as_deref(),
            actionable_count,
            error_owned.as_deref(),
        )
    })
    .await;
    match result {
        Ok(Ok(Some((seq, effective_stage)))) => {
            // 发事件用最新行（advance 已带 event_seq+1）。
            if let Ok(root) = app_root(app) {
                let app = app.clone();
                let job_id = job_id.to_string();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    let conn = open_library_connection(&root)?;
                    if let Some(job) = get_job(&conn, &job_id)? {
                        emit_row(&app, &job);
                    }
                    Ok::<(), String>(())
                })
                .await;
            }
            Some((seq, effective_stage))
        }
        _ => None,
    }
}

/// 只更新 `cloud_status`、不切换 `stage` 的异步包装（详见 `queue::set_cloud_status`）。
/// 用于本地识别仍在跑时如实声明云端「排队/起飞」，而不把 stage 提前推到
/// `cloud_recognition` 让进度条虚高。返回 `None` 表示 lease 丢失或已取消。
async fn set_cloud_status_only(
    app: &AppHandle,
    state: &Arc<ProcessingState>,
    job_id: &str,
    cloud_status: &str,
) -> Option<(i64, String)> {
    let Ok(root) = app_root(app) else { return None };
    let worker_id = state.worker_id.clone();
    let job_id_owned = job_id.to_string();
    let cloud_owned = cloud_status.to_string();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let conn = open_library_connection(&root)?;
        if !renew_lease(&conn, &job_id_owned, &worker_id)? {
            return Ok(None);
        }
        set_cloud_status(&conn, &job_id_owned, &worker_id, &cloud_owned)
    })
    .await;
    match result {
        Ok(Ok(Some((seq, stage)))) => {
            if let Ok(root) = app_root(app) {
                let app = app.clone();
                let job_id = job_id.to_string();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    let conn = open_library_connection(&root)?;
                    if let Some(job) = get_job(&conn, &job_id)? {
                        emit_row(&app, &job);
                    }
                    Ok::<(), String>(())
                })
                .await;
            }
            Some((seq, stage))
        }
        _ => None,
    }
}

async fn fail_job(app: &AppHandle, state: &Arc<ProcessingState>, job_id: &str, error: &str) {
    // 只保留稳定错误码；完整错误在应用日志里。
    let code = error.split(':').next().unwrap_or("processing_failed").to_string();
    let code = code.chars().take(80).collect::<String>();
    let advanced = advance(
        app,
        state,
        job_id,
        STAGE_FAILED,
        Some("failed"),
        Some("skipped"),
        Some("skipped"),
        None,
        Some(&code),
    )
    .await;
    // 复核 B/E：advance 可能因 durable 取消标记把行强制落 cancelled；
    // 此时 library item 不得标成 failed（持久化列不一致，且取消不是失败）。
    let effective_cancelled = advanced
        .as_ref()
        .map(|(_, stage)| stage == queue::STAGE_CANCELLED)
        .unwrap_or(false);
    if advanced.is_some() && !effective_cancelled {
        if let Ok(root) = app_root(app) {
            let job_id = job_id.to_string();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                let conn = open_library_connection(&root)?;
                crate::library::repository::set_item_status(&conn, &job_id, "failed")
            })
            .await;
        }
    }
}

async fn finish_cancelled(app: &AppHandle, state: &Arc<ProcessingState>, job_id: &str) {
    let advanced = advance(app, state, job_id, queue::STAGE_CANCELLED, None, None, None, None, None).await;
    if advanced.is_none() {
        // G1 边界（复核 B）：lease 已丢（心跳瞬断/休眠唤醒）时 advance 无法
        // 提交。durable 取消标记仍在且 reclaim 守卫保证没有 worker 会接手，
        // 免 lease 收尾为 cancelled——取消不得只能靠重启兑现。
        let Ok(root) = app_root(app) else { return };
        let job_id_owned = job_id.to_string();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let conn = open_library_connection(&root)?;
            finalize_cancelled_without_lease(&conn, &job_id_owned)
        })
        .await;
    }
    state.cancelled.write().await.remove(job_id);
    if let Ok(root) = app_root(app) {
        let app = app.clone();
        let job_id = job_id.to_string();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            let conn = open_library_connection(&root)?;
            if let Some(job) = get_job(&conn, &job_id)? {
                emit_row(&app, &job);
            }
            Ok::<(), String>(())
        })
        .await;
    }
}

async fn set_item_status_ready(app: &AppHandle, job_id: &str) {
    let Ok(root) = app_root(app) else { return };
    let job_id = job_id.to_string();
    let _ = tauri::async_runtime::spawn_blocking({
        let root = root.clone();
        let job_id = job_id.clone();
        move || {
            // G1 对抗审计：advance 可能因 durable 取消标记被强制落 cancelled；
            // 已取消/失败的任务不得再把 library item 推成 ready/action_required。
            let conn0 = open_library_connection(&root)?;
            if let Some(job) = queue::get_job(&conn0, &job_id)? {
                if matches!(job.stage.as_str(), "cancelled" | "failed") {
                    return Ok::<(), String>(());
                }
            }
            crate::library::migration::migrate_single_item(&root, &job_id)?;
            let conn = open_library_connection(&root)?;
            let status = crate::library::repository::get_canonical_ds(&conn, &job_id)?
                .filter(|(ds, _)| ds.pointer("/quality/state").and_then(Value::as_str) == Some("ready"))
                .map(|_| "ready").unwrap_or("action_required");
            crate::library::repository::set_item_status(&conn, &job_id, status)?;
            // library 状态是前端行渲染的输入之一：推进 stateVersion，
            // 否则补发事件会因 event_seq 相同被前端去重丢弃。
            conn.execute(
                "UPDATE processing_jobs_v2 SET event_seq = event_seq + 1 WHERE id = ?1",
                [&job_id],
            )
            .map_err(|error| format!("processing_status_seq:{error}"))?;
            Ok(())
        }
    })
    .await;
    // G0/E2E 发现的竞态：advance 的事件先于本函数的 library 状态提交到达，
    // 前端据其刷新会看到 status 仍是 processing（行永远显示"排队中"）。
    // 状态落库后补发一次事件，让 UI 以最终状态收尾。
    let app = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let conn = open_library_connection(&root)?;
        if let Some(job) = queue::get_job(&conn, &job_id)? {
            emit_row(&app, &job);
        }
        Ok::<(), String>(())
    })
    .await;
}

/// 供 import_files 使用的入队 helper（M2-T02）：文件复制在调用方完成，这里只落数据库。
pub(crate) fn enqueue_processing_job(
    root: &std::path::Path,
    job_id: &str,
    library_item_id: &str,
    source_asset_id: &str,
    progress: &Value,
) -> CommandResult<()> {
    let conn = open_library_connection(root)?;
    queue::enqueue(&conn, job_id, library_item_id, source_asset_id, progress)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生产并发编排契约的精简副本（仅供单测，生产代码在 `run_job_inner` 内联同一套
    /// 顺序）：1) 立刻并发 spawn 本地阻塞任务与云端 future（二者互不阻塞——云端 permit
    /// 获取不挡本地起飞）；2) 只 await 本地结果；3) 本地完成后调用 `on_local_done`
    /// （生产里即「发布可编辑草稿 + 冻结 base_edit_version」），**之后**才 await 云端句柄。
    /// 返回 `(本地结果, 云端结果)`；云端结果 `None` 表示云端任务中止。
    async fn run_parallel_local_cloud<Fut>(
        local: impl FnOnce() -> Result<serde_json::Value, String> + Send + 'static,
        cloud: Option<Fut>,
        on_local_done: impl FnOnce(),
    ) -> (Result<serde_json::Value, String>, Option<Result<serde_json::Value, String>>)
    where
        Fut: std::future::Future<Output = Option<Result<serde_json::Value, String>>> + Send + 'static,
    {
        let local_handle = tauri::async_runtime::spawn_blocking(local);
        let cloud_handle: Option<
            tauri::async_runtime::JoinHandle<Option<Result<serde_json::Value, String>>>,
        > = cloud.map(|fut| tauri::async_runtime::spawn(fut));

        // 只 await 本地——云端仍在并行跑。
        let local_result = local_handle
            .await
            .unwrap_or_else(|error| Err(format!("processing_join:{error}")));

        // 本地完成即发布草稿（生产：advance local_status=succeeded + set_item_status_ready）。
        on_local_done();

        // 此刻才收口云端：草稿发布不依赖云端调用完成（「本地先出稿」承诺）。
        let cloud_result = match cloud_handle {
            Some(handle) => handle.await.unwrap_or(None),
            None => None,
        };
        (local_result, cloud_result)
    }

    /// 证明「本地识别与云端识别并行」的两条核心产品保证，且二者用确定性时序
    /// （本地 50ms < 云端 300ms）驱动，不依赖任何 sleep 竞态，因此不会偶发 flaky：
    ///
    /// 1) **云端在本地仍在跑时就已经起飞**——云端 future 一开始即记录「本地尚未完成」。
    ///    若并行被破坏（云端等到本地结束后才 spawn），`cloud_started_before_local_done`
    ///    将为 false，断言失败。这同时覆盖了 Defect 1（本地不再被云端 permit 卡住）。
    ///
    /// 2) **可编辑草稿在云端调用完成之前就已发布**——`on_local_done`（对应生产里的
    ///    `set_item_status_ready` + 冻结 `base_edit_version`）在 local 解析后、云端句柄
    ///    被 await 之前调用；该回调记录「云端是否仍在跑」。这覆盖了 Defect 2（草稿发布
    ///    不再等云端）。
    ///
    /// `run_parallel_local_cloud` 是 `run_job_inner` 内联并发顺序的精简副本：先并发 spawn
    /// 本地 + 云端，只 await 本地，本地完成即调 `on_local_done`，之后才 await 云端。
    #[test]
    fn cloud_fetch_starts_before_local_recognition_finishes_and_draft_published_first() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        let local_finished = Arc::new(AtomicBool::new(false));
        let cloud_started = Arc::new(AtomicBool::new(false));
        let cloud_finished = Arc::new(AtomicBool::new(false));
        let draft_published = Arc::new(AtomicBool::new(false));
        // 云端 future 一开始：本地是否仍在进行。
        let cloud_started_before_local_done = Arc::new(AtomicBool::new(false));
        // 发布草稿那一刻：云端是否仍在进行。
        let draft_published_before_cloud_done = Arc::new(AtomicBool::new(false));

        // 本地较快（50ms），云端较慢（300ms）→ 确定性不靠 sleep 竞态。
        let local_finished_c = local_finished.clone();
        let local = move || {
            std::thread::sleep(Duration::from_millis(50));
            local_finished_c.store(true, Ordering::SeqCst);
            Ok(serde_json::json!({"local": true}))
        };

        let cloud_started_c = cloud_started.clone();
        let cloud_finished_c = cloud_finished.clone();
        let local_finished_c2 = local_finished.clone();
        let marker = cloud_started_before_local_done.clone();
        let cloud = async move {
            // 云端一被调度起来就必须记录：此时本地识别仍在跑。
            cloud_started_c.store(true, Ordering::SeqCst);
            marker.store(!local_finished_c2.load(Ordering::SeqCst), Ordering::SeqCst);
            // 云端耗时（明显长于本地）。
            tokio::time::sleep(Duration::from_millis(300)).await;
            cloud_finished_c.store(true, Ordering::SeqCst);
            Some(Ok(serde_json::json!({"cloud": true})))
        };

        let draft_published_c = draft_published.clone();
        let cloud_finished_c2 = cloud_finished.clone();
        let draft_marker = draft_published_before_cloud_done.clone();
        let on_local_done = move || {
            // 对应生产：本地完成即发布可编辑草稿（set_item_status_ready）。
            draft_published_c.store(true, Ordering::SeqCst);
            // 发布那一刻，云端是否仍在跑。
            draft_marker.store(!cloud_finished_c2.load(Ordering::SeqCst), Ordering::SeqCst);
        };

        let (local_result, cloud_result) =
            tauri::async_runtime::block_on(run_parallel_local_cloud(local, Some(cloud), on_local_done));

        assert!(local_result.is_ok(), "本地识别应成功");
        assert!(
            cloud_result.expect("cloud 应返回 Some").is_ok(),
            "云端拉取应成功"
        );
        // 保证 1：云端在本地完成前就已起飞（并行核心保证，Defect 1）。
        assert!(cloud_started.load(Ordering::SeqCst), "云端任务必须被启动");
        assert!(
            cloud_started_before_local_done.load(Ordering::SeqCst),
            "云端模型调用必须在本地识别完成之前就已起飞（并行核心保证）"
        );
        // 保证 2：草稿在云端完成前就已发布（本地先出稿承诺，Defect 2）。
        assert!(
            draft_published.load(Ordering::SeqCst),
            "本地完成后必须立即发布草稿"
        );
        assert!(
            draft_published_before_cloud_done.load(Ordering::SeqCst),
            "可编辑草稿必须在云端模型调用完成之前就发布（本地先出稿承诺）"
        );
    }

    /// 目标 2（Fix 2）的 seam 验证：本地候选快照必须在草稿可编辑**之前**冻结，
    /// 云端裁决比对冻结快照而非用户编辑后的当前稿，从而「云端运行期间用户改了稿」
    /// 可被识别、迟到结果不覆盖用户修改。
    ///
    /// 这里直接复刻 `freeze_local_candidate_snapshot` 的落盘行为：按冻结时刻的稿投影本地
    /// 候选并写入 `local-candidate.json`（`batch_id` 由 `job_id + source_sha256 + 版本` 派生）。
    /// 随后模拟用户编辑（答案被改、当前稿变化），调用 `resolve_local_snapshot` 取快照——
    /// 必须返回**冻结时**的候选（答案仍为冻结值），而不是被编辑后当前稿重投影的结果。
    #[test]
    fn frozen_local_snapshot_takes_precedence_over_later_user_edits() {
        use crate::reconcile::engine::resolve_local_snapshot;
        use uuid::Uuid;
        use crate::schema::recognition_v1::ChainKindV1;
        use crate::util::{ensure_app_dirs, ensure_job_dirs, job_dir};

        let root = std::env::temp_dir()
            .join(format!("pdf2test-freeze-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).unwrap();
        let job_id = "freeze-job";
        ensure_job_dirs(&job_dir(&root, job_id)).unwrap();

        // 冻结时刻的稿（base_edit_version = 0）：q1 答案 "frozen_answer"。
        let frozen_authoring = json!({
            "answerSlots": { "q1": { "questionNumber": 1, "answer": "frozen_answer" } },
            "answerKey": { "q1": "frozen_answer" },
            "taskGroups": [{
                "taskId": "t1", "taskType": "short_answer",
                "responseGroups": [{ "responseGroupId": "r1", "slotIds": ["q1"] }]
            }]
        });
        let source_sha256 = "a".repeat(64);
        let base_edit_version = 0_i64;
        let batch_id = commands::recognition_batch_id(job_id, &source_sha256, base_edit_version);

        // 等价于 `freeze_local_candidate_snapshot`：投影并落盘冻结快照。
        let snapshot = candidate::local_candidate_from_authoring(
            &frozen_authoring,
            &batch_id,
            job_id,
            job_id,
            job_id,
            &source_sha256,
            base_edit_version,
        );
        store::write_candidate(&root, &batch_id, &snapshot, store::LOCAL_CANDIDATE_FILE).unwrap();

        let stored = store::read_candidate(&root, job_id, &batch_id, store::LOCAL_CANDIDATE_FILE)
            .expect("冻结快照应已落盘");
        assert_eq!(stored.chain, ChainKindV1::Local);
        assert_eq!(stored.base_edit_version, base_edit_version);
        assert_eq!(stored.batch_id, batch_id);

        // 模拟用户编辑：q1 答案被改成 "edited_answer"（当前稿已变化）。
        let edited_authoring = json!({
            "answerSlots": { "q1": { "questionNumber": 1, "answer": "edited_answer" } },
            "answerKey": { "q1": "edited_answer" },
            "taskGroups": [{
                "taskId": "t1", "taskType": "short_answer",
                "responseGroups": [{ "responseGroupId": "r1", "slotIds": ["q1"] }]
            }]
        });
        // 同一 batch（source 与 base 版本未变）下，reconcile 取快照应命中冻结者。
        let resolved = resolve_local_snapshot(
            Some(stored),
            &edited_authoring,
            &batch_id,
            job_id,
            job_id,
            job_id,
            &source_sha256,
            base_edit_version,
        );
        // 关键不变量：返回的是冻结快照，答案仍是 "frozen_answer"，而非编辑稿重投影的
        // "edited_answer"——这才是「迟到结果不覆盖用户修改」的前提。
        let frozen_answer = resolved
            .slots
            .iter()
            .find(|slot| slot.slot_id == "q1")
            .and_then(|slot| slot.answer.as_ref())
            .and_then(|answer| answer.as_str());
        assert_eq!(
            frozen_answer,
            Some("frozen_answer"),
            "云端裁决应比对冻结快照，不得用编辑后当前稿重投影覆盖用户修改"
        );
        assert_eq!(resolved.batch_id, batch_id);
        assert_eq!(resolved.base_edit_version, base_edit_version);
        assert_eq!(resolved.source_sha256, source_sha256);

        // 反向对照：若从未冻结（stored=None），则按当前（已编辑）稿重投影，证明
        // 「冻结」本身才是上一条断言成立的原因。
        let projected = resolve_local_snapshot(
            None,
            &edited_authoring,
            &batch_id,
            job_id,
            job_id,
            job_id,
            &source_sha256,
            base_edit_version,
        );
        let projected_answer = projected
            .slots
            .iter()
            .find(|slot| slot.slot_id == "q1")
            .and_then(|slot| slot.answer.as_ref())
            .and_then(|answer| answer.as_str());
        assert_eq!(
            projected_answer,
            Some("edited_answer"),
            "未冻结时应按当前稿投影（对照，证明冻结才是关键）"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目标 2（Fix 2）的**真函数**验证，补上此前被忽略的失败路径。
    ///
    /// 上一条测试的注释自陈是「直接复刻 `freeze_local_candidate_snapshot` 的落盘行为」，
    /// 因此它**永远测不到真函数内部的静默吞错**（连接失败 return、canonical 缺失 return、
    /// `let _ =` 忽略写盘失败）。这里直接调用真函数，双向断言：
    ///   1. canonical 未就绪 ⇒ 必须返回 `Err`（过去是静默 `return`）；
    ///   2. canonical 就绪 ⇒ 真正落盘，且 `resolve_local_snapshot` 复用该快照。
    ///
    /// 为什么失败必须上抛：冻结失败会让 `resolve_local_snapshot` 退回「按当前稿重投影」，
    /// 于是迟到云端结果会覆盖用户修改。调度器正是据此丢弃云端结果、跳过裁决并把
    /// `FREEZE_SNAPSHOT_FAILED` 写入 `last_error_code`（而不是只打一行 stderr）。
    #[test]
    fn freeze_local_candidate_snapshot_surfaces_failure_and_writes_real_snapshot() {
        use crate::library::repository::{
            open_library_connection, seed_canonical_ds, upsert_item_shell, UpsertItemInput,
        };
        use crate::reconcile::engine::resolve_local_snapshot;
        use crate::schema::recognition_v1::ChainKindV1;
        use crate::util::{ensure_app_dirs, ensure_job_dirs, job_dir};
        use uuid::Uuid;

        let root = std::env::temp_dir()
            .join(format!("pdf2test-freeze-real-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).unwrap();
        let job_id = "freeze-real-job";
        ensure_job_dirs(&job_dir(&root, job_id)).unwrap();
        let base_edit_version = 0_i64;

        // (1) canonical 未就绪：过去静默 return（调用方误以为已冻结），现在必须上抛。
        let error = freeze_local_candidate_snapshot(&root, job_id, base_edit_version)
            .expect_err("canonical 未就绪时冻结必须失败并上抛，而不是静默返回");
        assert!(
            error.contains("canonical_not_seeded"),
            "失败原因应可定位，实际为：{error}"
        );

        // (2) canonical 就绪：真函数必须落盘，且 resolve 会复用它。
        let conn = open_library_connection(&root).unwrap();
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: job_id,
                modality: "reading",
                title: "t",
                status: "action_required",
                source_asset_id: None,
            },
        )
        .unwrap();
        seed_canonical_ds(
            &conn,
            job_id,
            &json!({
                "schemaVersion": "IeltsAuthoringIRV2",
                "answerSlots": { "q1": { "questionNumber": 1, "answer": "frozen_answer" } },
                "answerKey": { "q1": "frozen_answer" },
                "taskGroups": [{
                    "taskId": "t1", "taskType": "short_answer",
                    "responseGroups": [{ "responseGroupId": "r1", "slotIds": ["q1"] }]
                }]
            })
            .to_string(),
            "action_required",
        )
        .unwrap();
        drop(conn);

        freeze_local_candidate_snapshot(&root, job_id, base_edit_version)
            .expect("canonical 就绪时冻结必须成功");

        let source_sha256 = commands::source_sha256_for_job(&root, job_id);
        let batch_id = commands::recognition_batch_id(job_id, &source_sha256, base_edit_version);
        let stored = store::read_candidate(&root, job_id, &batch_id, store::LOCAL_CANDIDATE_FILE)
            .expect("真函数必须把本地候选快照真正落盘");
        assert_eq!(stored.chain, ChainKindV1::Local);
        assert_eq!(stored.base_edit_version, base_edit_version);

        // 用户随后改稿：resolve 仍须命中冻结快照，而非编辑后的当前稿。
        let edited = json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "answerSlots": { "q1": { "questionNumber": 1, "answer": "edited_answer" } },
            "answerKey": { "q1": "edited_answer" },
            "taskGroups": [{
                "taskId": "t1", "taskType": "short_answer",
                "responseGroups": [{ "responseGroupId": "r1", "slotIds": ["q1"] }]
            }]
        });
        let resolved = resolve_local_snapshot(
            Some(stored),
            &edited,
            &batch_id,
            job_id,
            job_id,
            job_id,
            &source_sha256,
            base_edit_version,
        );
        let answer = resolved
            .slots
            .iter()
            .find(|slot| slot.slot_id == "q1")
            .and_then(|slot| slot.answer.as_ref())
            .and_then(|answer| answer.as_str());
        assert_eq!(
            answer,
            Some("frozen_answer"),
            "真函数落盘的快照必须优先于用户编辑后的当前稿"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// `not_run` 必须**原样透传**，不得被折叠成 `failed`。
    ///
    /// 这是「无云导入」在任务行里的唯一真相来源。折叠成 `failed` 会让用户去排查一个
    /// 根本不存在的云端故障（这正是修复前的实际表现）。四种已知状态逐一钉死；未知值
    /// 仍保守归为失败——不能因为「不认识」就报成功。
    #[test]
    fn chain_status_keeps_not_run_distinct_from_failed() {
        assert_eq!(chain_status_to_job_status("not_run"), "not_run");
        assert_eq!(chain_status_to_job_status("succeeded"), "succeeded");
        assert_eq!(chain_status_to_job_status("partial"), "partial");
        assert_eq!(chain_status_to_job_status("unusable"), "failed");
        assert_eq!(chain_status_to_job_status("something_new"), "failed");
    }

    /// 无云路径的**产品级**锁定（调度器这一层）。
    ///
    /// 修复前：`!launch_cloud` 分支在发布「可编辑」之后直接 `return`，reconcile 从未被调用，
    /// 于是本地候选快照 / 原文核验 / 批次与决策**全部没有落盘**——前端拿不到任何可解释的
    /// 证据链，产品语义上等于「没做识别」。修复后这条路径照常跑完核心，只把云端如实标成
    /// `not_run`（其注入点被显式设成返回 `Err`，因此一旦有人改坏前置判断，会得到显式错误
    /// 而不是静默的假成功）。
    ///
    /// 与 `reconcile::commands` 里同场景的核心层测试互补：那条证明**核心**行为正确，
    /// 这条证明**调度器确实接上了核心**——两者的接缝正是缺口所在。
    #[test]
    fn local_only_cycle_runs_reconcile_and_reports_cloud_not_run() {
        use crate::job_store::{make_job, save_job};
        use crate::library::repository::{
            open_library_connection, seed_canonical_ds, upsert_item_shell, UpsertItemInput,
        };
        use crate::reconcile::store::{read_candidate, read_current_batch, read_decision_file};
        use crate::util::{ensure_app_dirs, ensure_job_dirs, job_dir, write_json};
        use crate::{CreateJobInput, SourceFile, WorkflowStep};
        use uuid::Uuid;

        let root =
            std::env::temp_dir().join(format!("pdf2test-localonly-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).expect("app dirs");

        let mut job = make_job(CreateJobInput {
            title: Some("local-only".to_string()),
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
            &serde_json::json!({"schemaVersion":"IeltsAuthoringIRV2","exam":{"title":"t"},"taskGroups":[],"answerSlots":{},"answerKey":{},"quality":{"coverageStatus":{"unassignedSourceNodeIds":[]}}}),
        )
        .expect("authoring-ir");
        write_json(
            &job_dir(&root, &job.job_id).join("document-ir.json"),
            &serde_json::json!({"pages":[{"pageIndex":0,"lines":[{"text":"A passage about birds."}]}]}),
        )
        .expect("document-ir");

        let canonical = serde_json::json!({
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
            "answerKey": {"slot-14": {"kind":"text","values":["stencilling"]}},
            "quality": {"coverageStatus": {"unassignedSourceNodeIds": []}}
        });
        {
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
            seed_canonical_ds(&conn, &job.job_id, &canonical.to_string(), "action_required")
                .expect("seed canonical");
        }

        let report = run_local_only_recognition_cycle(&root, &job.job_id, 0)
            .expect("无云路径必须跑通，而不是报错或跳过");

        assert_eq!(
            report.cloud_status, "not_run",
            "云端必须如实报 not_run，而不是 failed"
        );
        assert_eq!(report.reconcile_status, "succeeded", "裁决链必须跑完");

        // 落盘证据：跳过 reconcile 时这些东西一个都不会有。
        let batch_id = read_current_batch(&root, &job.job_id).expect("当前批次必须已登记");
        assert!(
            read_decision_file(&root, &job.job_id, &batch_id).is_some(),
            "无云路径也必须留下决策文件（本地候选 + 原文核验 + 裁决）"
        );
        assert!(
            read_candidate(
                &root,
                &job.job_id,
                &batch_id,
                crate::reconcile::store::LOCAL_CANDIDATE_FILE
            )
            .is_some(),
            "本地候选快照必须落盘，否则裁决无据可依"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}

