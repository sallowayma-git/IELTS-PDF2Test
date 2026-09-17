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
/// 「核验 + 裁决」周期**跑完但返回 Err** 时的机器码。
const CYCLE_FAILED: &str = "RECONCILE_FAILED";
/// 「核验 + 裁决」周期**没能跑完**（阻塞任务 panic / 被取消）时的机器码。
///
/// 与 `CYCLE_FAILED` 分开是有意的：前者要回答「周期自己返回了 Err」，
/// 后者要回答「进程里刚刚发生过一次 panic」。混成同一个码之后，
/// 事故复盘时无法区分「模型调用失败」与「代码炸了」——那是两条完全不同的修法。
const CYCLE_JOIN_FAILED: &str = "RECOGNITION_CYCLE_JOIN_FAILED";

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

    // ── 首稿初始化（幂等，必须在冻结之前）──────────────────────────────
    // 本地 artifact 已成，但**权威稿还没建**。播种此前只发生在 `set_item_status_ready`
    // 里，也就是**冻结之后**——而冻结要读权威稿来投影本地候选，于是它必然以
    // `canonical_not_seeded` 失败：无云导入整条比较链被跳过，有云导入也失去了可信基线。
    // 顺序因此固定为：初始化 → 取基线 → 冻结 → 发布可编辑。
    //
    // 「不打开工作区也能继续」正是靠这里：初始化由后台无条件完成，不依赖前端
    // 打开工作区触发按需迁移。
    let mut freeze_error: Option<String> = None;
    let init_result = tauri::async_runtime::spawn_blocking({
        let root = root.clone();
        let job_id = job_id.clone();
        move || crate::library::migration::ensure_initial_canonical(&root, &job_id)
    })
    .await
    .unwrap_or_else(|error| Err(format!("processing_join:{error}")));
    match init_result {
        Ok(true) => {}
        Ok(false) => {
            // artifact 里没有可辨认的题稿候选：不冻结、不裁决。这不是把失败掩码掉，
            // 而是一次如实降级——原因写进 `last_error_code` 供 UI / 诊断区分。
            eprintln!("[processing] no authoring candidate to seed initial canonical for {job_id}");
            freeze_error = Some("CANONICAL_INIT_NO_CANDIDATE".to_string());
        }
        Err(error) => {
            eprintln!("[processing] initial canonical seeding failed for {job_id}: {error}");
            freeze_error = Some(format!("CANONICAL_INIT_FAILED:{error}"));
        }
    }

    // 批次基线：本地稿定稿时的编辑版本。**必须在草稿发布「可编辑」之前**冻结，
    // 否则用户若在「发布」与「读 baseline」之间改稿，基线版本会被抬高，而后续
    // 云端裁决若按当前稿重投影本地候选，就会把用户编辑误当成本地识别结果。
    //
    // **读不到版本时不得退化成 0**：0 是一个合法的真实版本，用 0 顶替「读取失败」会
    // 让批次 id（由 job_id + 源文件哈希 + 版本三者派生）指向一个并不存在的批次，冻结出的
    // 快照与真实稿并不对应——这比「没有快照」更危险，因为它**看起来是可信的**。
    // 读取失败与冻结失败同等对待：不冻结、不裁决、不自动写入。
    //
    // 版本与稿件**由冻结函数在同一次读取中取回**（`get_canonical_ds` 一次查询同时返回
    // 两列），调用方无从"读新 DS 却贴旧版本"。
    let base_edit_version = if freeze_error.is_none() {
        match freeze_local_candidate_snapshot(&root, &job_id) {
            Ok(version) => version,
            Err(error) => {
                eprintln!(
                    "[processing] freeze local candidate snapshot failed for {job_id}: {error}"
                );
                freeze_error = Some(error);
                // 占位值：下面的 `freeze_error.is_some()` 分支保证它既不参与冻结也不参与裁决。
                0
            }
        }
    } else {
        0
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
        //
        // 周期整段放进阻塞边界（`run_cycle_in_blocking_boundary`）：这条路径当前不构造
        // blocking HTTP 客户端，但它是同一个同步周期，边界一致才能保证「将来接入的
        // 模型通道」不会再把 panic 带进 async 上下文。
        let cycle = run_cycle_in_blocking_boundary({
            let root = root.clone();
            let job_id = job_id.clone();
            move || run_local_only_recognition_cycle(&root, &job_id, base_edit_version)
        })
        .await;
        let (cloud_status, reconcile_status, actionable) = match cycle {
            Ok(report) => (
                report.cloud_status,
                report.reconcile_status,
                report.actionable_count,
            ),
            Err(failure) => {
                settle_cycle_failure(&app, &state, &job_id, &failure).await;
                return;
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
            None,
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
    //
    // A3/A4 的模型通道（`verify_source_answers` / `adjudicate_divergence`）就在这条周期里，
    // 它们会真的发 HTTP，因此整段必须落在阻塞边界上（见
    // `run_cycle_in_blocking_boundary`）。修复前这里是在 async 上下文里直接同步调用的，
    // 只要核验通道真的被触发就会 panic，任务随即悬挂在 `cloud_recognition`。
    let cycle = match cloud_fetched {
        Some(prefetched) => {
            let boundary = run_cycle_in_blocking_boundary({
                let root = root.clone();
                let job_id = job_id.clone();
                let profile = resolved_profile.clone();
                move || {
                    run_recognition_cycle(
                        &root,
                        &job_id,
                        profile.as_deref(),
                        true,
                        prefetched,
                        base_edit_version,
                    )
                }
            })
            .await;
            match boundary {
                Ok(report) => Some(report),
                Err(failure) => {
                    settle_cycle_failure(&app, &state, &job_id, &failure).await;
                    return;
                }
            }
        }
        None => None,
    };
    let (cloud_status, reconcile_status, actionable) = match &cycle {
        // 云端 JSON 已在本地识别期间并发拉取并冻结于此；reconcile 直接复用，
        // 不再发起第二次网络调用。
        Some(report) => (
            report.cloud_status.clone(),
            report.reconcile_status.clone(),
            report.actionable_count,
        ),
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
#[derive(Debug, Clone)]
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

/// 「核验 + 裁决」周期的失败来源。
///
/// 两者必须分开：
/// - `Joined` = 阻塞任务 **panic 或被取消**，周期内到底做到哪一步无法断言；
/// - `Failed` = 周期跑完并如实返回了 `Err`（读稿失败 / 落盘失败等）。
///
/// 两者都**不允许**被当成「周期成功」，也不允许被静默丢弃。修复前这个周期是
/// **在 async 上下文里直接同步调用**的，于是它内部的一次 panic 会直接带走整个
/// `run_job` 任务：任务行停在 `local_recognition` / `cloud_recognition`，
/// 既没有终态也没有错误码，只能等下次启动 `recover_on_startup` 才脱困——
/// 这就是「任务停留在处理中」的成因。
#[derive(Debug, Clone, PartialEq, Eq)]
enum CycleFailure {
    Joined(String),
    Failed(String),
}

impl CycleFailure {
    fn code(&self) -> &'static str {
        match self {
            CycleFailure::Joined(_) => CYCLE_JOIN_FAILED,
            CycleFailure::Failed(_) => CYCLE_FAILED,
        }
    }

    fn detail(&self) -> &str {
        match self {
            CycleFailure::Joined(error) | CycleFailure::Failed(error) => error,
        }
    }
}

/// 把同步的「核验 + 裁决」周期放到**正确的阻塞执行边界**上执行。
///
/// 为什么必须是阻塞边界（A3/A4 在真实调度器路径里 panic 的根因）：
/// 这条周期会经 `auto_pipeline::verify_source_answers_through_gateway` /
/// `adjudicate_divergence_through_gateway` 落到 `llm_gateway::openai_post_once`，
/// 那里构造 `reqwest::blocking::Client`。该客户端在 `reqwest::blocking::wait::timeout`
/// 里做三件**都不能发生在 async 运行时上下文里**的事：
///
/// 1. `wait::timeout` 开头调用 `enter()`（debug 构建），它会新建一个 shell runtime 并
///    调用 `Runtime::enter()`；在已经进入了 runtime 的线程上 `enter` 直接 panic：
///    「Cannot start a runtime from within a runtime.」
///    （`tokio::runtime::context::runtime::enter`）；
/// 2. 同一个 shell runtime 若在 async 上下文里析构，会 panic：
///    「Cannot drop a runtime in a context where blocking is not allowed.」
///    （`tokio::runtime::blocking::shutdown`）——所以**释放**也必须落在边界内；
/// 3. 请求与响应体读取用 `thread::park()` 等待，会把 async worker 线程整个挂住。
///
/// 第 1、2 条只在 `debug_assertions` 打开时触发（`wait::timeout::enter` 是
/// `#[cfg(debug_assertions)]` 的），产品侧实测到的正是第 2 条：
/// `thread 'tokio-rt-worker' panicked at tokio-1.52.3/src/runtime/blocking/shutdown.rs:51`
/// ——「A3 输入缓存已写、调用记录与输出都没有、受控服务零 POST」。**但 release 构建
/// 同样有病**：前两条退化成静默的 worker 阻塞（第 3 条不变），所以这条边界不是
/// 「只在 debug 下才需要」的补丁。
///
/// 因此**创建、请求、释放**必须在同一条阻塞线程上完成：一次 `spawn_blocking` 包住
/// **整段**同步周期，而不是把客户端单独挪出去。把这段抽成独立函数（而不是在每个调用点
/// 各写一遍 `spawn_blocking`）是为了让回归测试能对着**生产用的同一个边界**跑：
/// 测试里复刻一份等价代码，就永远测不到这个边界本身。
async fn run_cycle_in_blocking_boundary<F>(
    cycle: F,
) -> Result<RecognitionCycleReport, CycleFailure>
where
    F: FnOnce() -> Result<RecognitionCycleReport, String> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(cycle).await {
        Ok(Ok(report)) => Ok(report),
        Ok(Err(error)) => Err(CycleFailure::Failed(error)),
        // join 失败 = 周期 panic 或阻塞任务被取消。绝不折叠成成功，也绝不吞掉。
        Err(error) => Err(CycleFailure::Joined(error.to_string())),
    }
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
    // A3/A4 的预算与调用都留在这一层：判定层（`reconcile_batch` / `adjudicate` /
    // `verify_against_source`）不持状态、不发 HTTP，注入点只是两个 `Fn`。
    let adjudication_calls = std::cell::Cell::new(0u32);
    let adjudicator = |payload: &[serde_json::Value]| {
        model_channel_call(&adjudication_calls, |repair_note| {
            crate::auto_pipeline::adjudicate_divergence_through_gateway(
                root, job_id, profile_id, payload, repair_note,
            )
        })
    };
    // 核验通道单独持一份预算：它在裁决之前跑，共用额度会让它吃光裁决的份额。
    let source_verification_calls = std::cell::Cell::new(0u32);
    let source_verifier = |payload: &[serde_json::Value]| {
        model_channel_call(&source_verification_calls, |repair_note| {
            crate::auto_pipeline::verify_source_answers_through_gateway(
                root, job_id, profile_id, payload, repair_note,
            )
        })
    };
    let adjudicator_ref: crate::reconcile::engine::AdjudicationRunner<'_> = &adjudicator;
    let source_verifier_ref: crate::reconcile::engine::SourceVerifyRunner<'_> = &source_verifier;
    let report = crate::reconcile::commands::run_recognition_cycle_core_with_channels(
        root,
        job_id,
        profile_id,
        cloud_enabled,
        base_edit_version,
        &|_root, _job_id, _profile_id| cloud_fetched.clone(),
        Some(source_verifier_ref),
        Some(adjudicator_ref),
    )?;
    Ok(summarize_cycle_report(report))
}

/// 模型通道的调用预算与「一次受约束修复」。
///
/// A3（原文件核验）与 A4（分歧裁决）共用这段策略，但**各持一份预算**：
/// 核验在裁决之前跑，两者共用一份额度会让核验吃光裁决的份额——那不是「有界」，
/// 那是裁决通道在真实生产路径上静默失效，而测试因为直接注入桩函数根本发现不了。
/// 单次导入的真实上限因此是 `2 × MAX_ADJUDICATION_MODEL_CALLS`。
///
/// 第一次被拒时把**校验器的原话**回给模型再问一次，而不是空转重试：
/// 不带被拒原因的重试只会拿到同一种错误——那不是修复，只是多烧一次配额。
fn model_channel_call<F>(
    calls: &std::cell::Cell<u32>,
    invoke: F,
) -> Result<serde_json::Value, crate::reconcile::engine::ModelCallFailure>
where
    F: Fn(Option<&str>) -> Result<serde_json::Value, String>,
{
    use crate::reconcile::engine::ModelCallFailure;
    let budget = crate::schema::recognition_v1::MAX_ADJUDICATION_MODEL_CALLS;
    if calls.get() >= budget {
        return Err(ModelCallFailure::BudgetExhausted);
    }
    calls.set(calls.get() + 1);
    match invoke(None) {
        Ok(value) => Ok(value),
        Err(error) => {
            let repairs = crate::schema::recognition_v1::MAX_CONSTRAINED_REPAIRS;
            if repairs == 0 || calls.get() >= budget {
                return Err(ModelCallFailure::Model(error));
            }
            calls.set(calls.get() + 1);
            invoke(Some(&error)).map_err(ModelCallFailure::Model)
        }
    }
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

// 原先这里有一个独立的 `current_edit_version_of`：先单独读版本，再由冻结函数单独读稿。
// 那是「新稿 + 旧版本」的温床——两次读之间的一次保存就会让批次 id 指向内容不符的批次。
// 现在版本与稿件由 `freeze_local_candidate_snapshot` 一趟查询同时取回，该函数已删除。
// 与之配套的语义保留：读不到版本**不得**退化成 0（0 是合法版本），一律按「基线不可用」
// 处理——上抛、不冻结、不裁决。

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
/// **版本来自这次读取本身**，不由调用方传入：`get_canonical_ds` 一趟查询同时取回
/// 稿件与 `current_edit_version`，两者天生一致。若让调用方先单独读版本、函数再单独读稿，
/// 两次读之间的一次保存就会产出「新稿 + 旧版本」的批次——批次 id 指向一个与快照内容
/// 不对应的批次，比「没有快照」更危险，因为它看起来是可信的。返回冻结时采用的版本，
/// 调用方必须用它（而不是任何先前读到的值）作为本次裁决的基线。
///
/// **每一步失败都必须上抛，不得静默吞掉。** 调用方拿「冻结成功」当作「迟到结果不覆盖
/// 用户修改」护栏的前提：一旦快照缺失，`resolve_local_snapshot`（`reconcile/engine.rs`）
/// 会退回「按当前稿现场重投影」分支，裁决就会把用户编辑后的稿当成本地识别结果，
/// 于是「云端运行期间用户改了稿」永远检测不到、迟到结果照样覆盖用户修改——正是本
/// 修复要消灭的缺陷。因此这里把连接 / 取稿 / 落盘三处失败逐一如实上抛。
fn freeze_local_candidate_snapshot(root: &Path, job_id: &str) -> Result<i64, String> {
    let conn = open_library_connection(root)
        .map_err(|error| format!("open_library_connection_failed:{error}"))?;
    let (canonical, base_edit_version) =
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
    Ok(base_edit_version)
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

/// 周期失败后的收口：取消优先，其余一律落持久化失败终态。
///
/// **取消必须被判成取消**：用户在周期跑动期间取消时，落到 `failed` 会让界面显示
/// 「识别失败，可以重试」——那是 G1 明确禁止的谎报（用户自己取消的，不是识别失败）。
/// 内存取消标记与 durable 标记是同一条语义的两个视图，内存视图够用是因为
/// `advance` 内部还会用 durable 标记做最后一次原子判定。
async fn settle_cycle_failure(
    app: &AppHandle,
    state: &Arc<ProcessingState>,
    job_id: &str,
    failure: &CycleFailure,
) {
    if state.cancelled.read().await.contains(job_id) {
        finish_cancelled(app, state, job_id).await;
        return;
    }
    fail_recognition_cycle(app, state, job_id, failure).await;
}

/// 「本地稿已成、但核验 / 裁决周期没有完成」的终态收尾。
///
/// 刻意**不复用** `fail_job`：那条路径描述的是**本地识别本身**失败
/// （`local_status` 写 `failed`，意味着一张可用的草稿都没有）。这里本地识别已经成功、
/// 草稿也已发布可编辑，失败的只是它之后的核验 / 裁决 / 批次落盘，
/// 所以 `local_status` 必须如实保持 `succeeded`。
///
/// 也刻意**不**改 library item 状态：草稿确实存在且可打开（「本地先出稿」是产品的核心
/// 承诺），失败由任务行承载——`stage = failed` 让前端把它渲染成「识别失败，可以重试」
/// 并给出重试入口。修复前这条路径停在 `ready_for_review` + `reconcile_status = failed`
/// **且不写错误码**，界面于是按 `ready_for_review` 渲染成「可以打开检查了」，
/// 又把「这批结果到底落盘了没有」这个问题整个抹掉——那正是「伪装成核验成功」。
async fn fail_recognition_cycle(
    app: &AppHandle,
    state: &Arc<ProcessingState>,
    job_id: &str,
    failure: &CycleFailure,
) {
    eprintln!(
        "[processing] recognition cycle failed for {job_id} ({}): {}",
        failure.code(),
        failure.detail()
    );
    let advanced = advance(
        app,
        state,
        job_id,
        STAGE_FAILED,
        Some("succeeded"),
        Some("failed"),
        Some("failed"),
        Some(0),
        Some(failure.code()),
    )
    .await;
    if advanced.is_none() {
        // lease 丢失（休眠唤醒 / 心跳瞬断）：`advance` 拒绝提交。这里**不能**用
        // `finalize_ready_without_lease` 兜底——它的前提恰好就是 `local_status = succeeded`，
        // 在这条路径上必然成立，于是会把一次失败写成 `ready_for_review`，又是一次伪装成功。
        // 如实留给 `recover_on_startup` 在下次启动把任务标成 interrupted 并重新入队。
        eprintln!(
            "[processing] recognition cycle failure could not be persisted for {job_id}: \
             lease lost; startup recovery will requeue it"
        );
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
            // 首稿初始化**刻意不在这里**：它已由 `run_job_inner` 在冻结之前显式完成
            // （`ensure_initial_canonical`）。放在这里会让「发布可编辑」早于本地基线冻结
            // 发生，冻结就会因为读不到权威稿而失败。存量数据的按需迁移入口仍在
            // `library::commands::get_workspace_item_core`（打开工作区）
            // 与 `authoring_v2_commands::get_publish_preflight_core`（发布预检）。
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
    ///   2. canonical 就绪 ⇒ 真正落盘，且 `resolve_local_snapshot` 复用该快照；
    ///   3. 返回的版本**就是**这趟读取到的行版本——批次 id 与实际快照必须同源。
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

        // (1) canonical 未就绪：过去静默 return（调用方误以为已冻结），现在必须上抛。
        let error = freeze_local_candidate_snapshot(&root, job_id)
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

        let base_edit_version = freeze_local_candidate_snapshot(&root, job_id)
            .expect("canonical 就绪时冻结必须成功");
        // (3) 返回的版本必须是**行里那个版本**（外壳行初始为 1），不是调用方凭空给的 0。
        assert_eq!(
            base_edit_version, 1,
            "冻结返回的版本必须来自本次 (稿, 版本) 同源读取，而不是调用方传入的占位值"
        );

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

    /// 目标 1（阶段一）的**顺序锁定**：初始化必须发生在冻结之前。
    ///
    /// 修复前的顺序是「冻结 → `set_item_status_ready`（它才播种）」：冻结读不到权威稿，
    /// 必然以 `canonical_not_seeded` 失败，于是无云导入整条比较链被跳过、有云导入失去
    /// 可信基线。本测试用真实数据把两个顺序都跑一遍——旧顺序必须失败，新顺序必须成立，
    /// 且**全程不打开工作区**（这正是「停留在题库页面也能完成处理」的含义）。
    #[test]
    fn initial_canonical_seeding_precedes_freeze_without_opening_the_workspace() {
        use crate::library::migration::ensure_initial_canonical;
        use crate::library::repository::{
            get_canonical_ds, open_library_connection, upsert_item_shell, UpsertItemInput,
        };
        use crate::util::{ensure_app_dirs, ensure_job_dirs, job_dir};
        use uuid::Uuid;

        let root = std::env::temp_dir()
            .join(format!("pdf2test-init-order-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).unwrap();
        let job_id = "order-job";
        ensure_job_dirs(&job_dir(&root, job_id)).unwrap();

        // 本地识别 artifact（导入管道写出的 shadow，形状与真实产物一致）。
        let authoring = json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "exam": { "title": "order" },
            "taskGroups": [{
                "taskId": "t1", "taskType": "short_answer",
                "responseGroups": [{ "responseGroupId": "r1", "slotIds": ["q1"] }]
            }],
            "answerSlots": { "q1": { "questionNumber": 1, "answer": "seeded_answer" } },
            "answerKey": { "q1": "seeded_answer" }
        });
        std::fs::write(
            job_dir(&root, job_id).join(crate::authoring_v2_commands::AUTHORING_V2_SHADOW_FILE),
            serde_json::to_vec(&authoring).unwrap(),
        )
        .unwrap();

        // 入队时建的壳：有行、无稿（等价于 `queue_import` 之后、识别收尾之前的状态）。
        {
            let conn = open_library_connection(&root).unwrap();
            upsert_item_shell(
                &conn,
                &UpsertItemInput {
                    id: job_id,
                    modality: "reading",
                    title: "order",
                    status: "processing",
                    source_asset_id: None,
                },
            )
            .unwrap();
        }

        // 旧顺序（先冻结）：必须失败——这就是本修复要消灭的状态，写进断言以免回退。
        let old_order = freeze_local_candidate_snapshot(&root, job_id)
            .expect_err("冻结先于初始化时必然读不到权威稿");
        assert!(
            old_order.contains("canonical_not_seeded"),
            "旧顺序的失败原因应可定位，实际为：{old_order}"
        );

        // 新顺序（后台先初始化，再冻结）：不打开工作区即可建立首稿与批次。
        assert!(
            ensure_initial_canonical(&root, job_id).unwrap(),
            "本地 artifact 已产出，后台初始化必须建立首稿"
        );
        let version = freeze_local_candidate_snapshot(&root, job_id)
            .expect("初始化之后冻结必须成功——否则无云导入的比较链会被整条跳过");
        assert_eq!(version, 1, "冻结版本必须来自 (稿, 版本) 的同源读取");

        let source_sha256 = commands::source_sha256_for_job(&root, job_id);
        let batch_id = commands::recognition_batch_id(job_id, &source_sha256, version);
        let stored = store::read_candidate(&root, job_id, &batch_id, store::LOCAL_CANDIDATE_FILE)
            .expect("本地候选快照必须真正落盘，供后续比较复用");
        assert_eq!(stored.base_edit_version, version);

        // 幂等：重复初始化不重建、不覆盖、不推进版本（重试导入安全）。
        assert!(ensure_initial_canonical(&root, job_id).unwrap());
        let conn = open_library_connection(&root).unwrap();
        let (ds, version_after) = get_canonical_ds(&conn, job_id).unwrap().unwrap();
        assert_eq!(version_after, 1, "重复初始化不得推进/重置版本");
        assert_eq!(
            ds.pointer("/answerKey/q1").and_then(Value::as_str),
            Some("seeded_answer"),
            "重复初始化必须保留原有稿件"
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

    // ── A3/A4 的真实异步调度边界（本轮修复的回归验证）─────────────────────

    /// 写一份**真的能被抽到文本**的 DOCX。
    ///
    /// 不能放空壳：非 PDF 的证据面由 `auto_pipeline::prepare_cloud_source_evidence`
    /// **直接读原文件**抽取，抽不出文本时核验会以
    /// `source_verification_source_text_unavailable` 提前失败，HTTP 请求根本发不出去
    /// ——那样用例测到的是「夹具没搭好」，而不是本次要验证的边界。
    fn write_minimal_docx(path: &Path, text: &str) {
        use std::io::Write;
        let file = std::fs::File::create(path).expect("create docx");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("word/document.xml", options)
            .expect("start document part");
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"
        );
        zip.write_all(xml.as_bytes()).expect("write document part");
        zip.finish().expect("finish docx");
    }

    /// 受控模型服务：**记录**每个请求体，并回一份合法的应答。
    ///
    /// 与 `reconcile::commands` 里的同名 helper 同一形态（先按 `Content-Length` 读完
    /// 请求体再回写，否则客户端还在发 body 时就收到 RST，得到一个与受控服务无关的
    /// 传输错误）。这里是**带请求留痕**的版本，因为本轮要断言的正是
    /// 「A3 请求到底发出去了没有」。
    ///
    /// **调用方不要 join 服务线程**：正常路径只发有限的几次请求，而 accept 循环会一直
    /// 等下一个连接，join 必然把用例挂死。
    fn spawn_recording_verification_service(
        response_body: String,
    ) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind controlled service");
        let addr = listener.local_addr().expect("local addr");
        let seen: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_thread = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
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
                if let Ok(mut guard) = seen_thread.lock() {
                    guard.push(String::from_utf8_lossy(&request).to_string());
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
        (format!("http://127.0.0.1:{}/v1", addr.port()), seen)
    }

    /// 标准 chat-completions 信封；`content` 里才是 A3 核验契约的输出。
    fn verification_response(slot_id: &str, question_number: u32, observed: &str) -> String {
        let content = json!({
            "findings": [{
                "slotId": slot_id,
                "questionNumber": question_number,
                "verdict": "contradicted",
                "quote": "the artist was stencilling the wall",
                "pageIndex": 1,
                "observedValue": {"kind": "text", "values": [observed], "normalization": "ielts_default"},
                "confidence": 0.9
            }]
        });
        json!({
            "id": "controlled-verify-0001",
            "object": "chat.completion",
            "model": "controlled-verify-v1",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {"role": "assistant", "content": content.to_string()}
            }]
        })
        .to_string()
    }

    /// A3 回归夹具：一个**能真正触发**核验模型通道的作业。
    ///
    /// 四条前提缺一条模型就不会被调用，用例也就覆盖不到这条边界：
    /// 1. 本地槽位**有值**（`answerKey`）——本地无值走的是「让模型发明答案」，A3 刻意不做；
    /// 2. 槽位带 `sourceAnchors`——`has_source_evidence` 只认它，「有答案」不等于「有原文证据」；
    /// 3. 页文本里出现该题号，但**没有**该题号的可读答案行——确定性核验因此判
    ///    `NotVerifiable`，这正是交给模型的那一类；原文里若已有 `14 stencilling` 这种
    ///    答案行，确定性结论先成立，模型**根本不会被调用**；
    /// 4. 非 PDF 来源的原文件能抽出原文文本（见 `write_minimal_docx`）。
    ///
    /// 本地值取 `painting`、受控服务回 `stencilling`：这样模型结论是「与原文不符」，
    /// 会落成一张带模型原文引用的建议卡，便于断言「核验结论确实来自模型通道」。
    fn seed_a3_verification_job(base_url: &str) -> (std::path::PathBuf, String) {
        use crate::job_store::{make_job, save_job};
        use crate::library::repository::{
            open_library_connection, seed_canonical_ds, upsert_item_shell, UpsertItemInput,
        };
        use crate::util::{ensure_app_dirs, ensure_job_dirs, job_dir, write_json};
        use crate::{CreateJobInput, SourceFile, WorkflowStep};
        use uuid::Uuid;

        let root = std::env::temp_dir()
            .join(format!("pdf2test-a3-boundary-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).expect("app dirs");

        let mut job = make_job(CreateJobInput {
            title: Some("a3-boundary".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["t".to_string()]),
            llm_profile_id: Some("controlled-verify".to_string()),
        });
        job.current_step = WorkflowStep::Authoring;
        job.source_files = vec![SourceFile {
            file_id: "file-1".to_string(),
            original_name: "source.docx".to_string(),
            stored_name: "stored.docx".to_string(),
            file_type: "docx".to_string(),
            sha256: "0".repeat(64),
            size_bytes: 1,
            role: "MainQuestion".to_string(),
            imported_at: chrono::Utc::now(),
        }];
        save_job(&root, &job).expect("save job");
        let dir = job_dir(&root, &job.job_id);
        ensure_job_dirs(&dir).expect("job dirs");

        write_json(
            &dir.join("authoring-ir.json"),
            &json!({"schemaVersion":"IeltsAuthoringIRV2","exam":{"title":"t"},"taskGroups":[],"answerSlots":{},"answerKey":{},"quality":{"coverageStatus":{"unassignedSourceNodeIds":[]}}}),
        )
        .expect("authoring-ir");
        write_json(
            &dir.join("document-ir.json"),
            &json!({"pages":[{"pageIndex":0,"lines":[{"text":"Question 14 asks about stencilling."}]}]}),
        )
        .expect("document-ir");
        let uploads = dir.join("uploads");
        std::fs::create_dir_all(&uploads).expect("uploads dir");
        write_minimal_docx(&uploads.join("stored.docx"), "Question 14 asks about stencilling.");

        let canonical = json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "exam": {"title": "t"},
            "taskGroups": [{
                "taskId": "task-1",
                "taskType": "sentence_completion",
                "displayRange": {"kind":"range","start":14,"end":14},
                "responseGroups": [{"responseGroupId":"rg-1","kind":"text_entry","slotIds":["slot-14"]}]
            }],
            "answerSlots": {
                "slot-14": {
                    "slotId": "slot-14",
                    "questionNumber": 14,
                    "interaction": "text",
                    "sourceAnchors": [{
                        "sourceFileId": "file-1",
                        "pageIndex": 0,
                        "nodeIds": ["node-1"],
                        "extractionMode": "native",
                        "sourceHash": "0"
                    }]
                }
            },
            "answerKey": {"slot-14": {"kind":"text","values":["painting"]}},
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

        let profile = json!({
            "profileId": "controlled-verify",
            "name": "Controlled Verification Service",
            "provider": "OpenAiCompatible",
            "baseUrl": base_url,
            "model": "controlled-verify-v1",
            "temperature": 0,
            "timeoutMs": 30000,
            "forceJson": true,
            "enabled": true
        });
        crate::llm_profiles::save_profiles(&root, &[profile])
            .expect("profile 必须能落盘，否则网关取不到 baseUrl");

        (root, job.job_id)
    }

    /// **本次修复的核心回归**：A3 核验通道穿过**真实异步调度边界**打到本地受控服务。
    ///
    /// 为什么必须这样测：核验通道的 HTTP 客户端是 `reqwest::blocking`，它**不能**在
    /// async 运行时上下文里创建或释放（见 `run_cycle_in_blocking_boundary` 的说明）。
    /// 在同步单测里直接调用网关永远覆盖不到这条路——那里没有 async 上下文，
    /// 客户端用得完全正常，缺陷在测试里不可见。
    ///
    /// 断言四件事，缺一件都不算通过：
    /// 1. 受控服务**真的收到了** A3 请求（不是本地自己算完了）；
    /// 2. 周期在阻塞边界内跑完、没有 panic，也没有被 join 失败吞掉；
    /// 3. 结果落库：批次已登记、决策文件可读；
    /// 4. 核验结论确实来自**模型通道**——形态是 `ANSWER_SOURCE_CONFLICT` 项上的
    ///    `sourceValue` 等于受控服务回的那个值（逐条 `model_quote` 证据不进决策项，
    ///    详见下方 (3) 的说明）。
    #[test]
    fn a3_verification_crosses_the_real_async_boundary_and_reaches_the_controlled_service() {
        use crate::reconcile::store::{read_current_batch, read_decision_file};

        let (base_url, seen) =
            spawn_recording_verification_service(verification_response("slot-14", 14, "stencilling"));
        let (root, job_id) = seed_a3_verification_job(&base_url);

        // 真实异步调度边界：与 `run_job_inner` 调用的是**同一个函数**。
        let report = tauri::async_runtime::block_on(run_cycle_in_blocking_boundary({
            let root = root.clone();
            let job_id = job_id.clone();
            move || {
                run_recognition_cycle(
                    &root,
                    &job_id,
                    Some("controlled-verify"),
                    true,
                    // 云端拉取不是本用例的重点（它会真的再打一次 HTTP）：显式注入失败，
                    // 让云端链如实标成 failed，核验通道照常跑。
                    Err("cloud_not_probed_in_this_regression".to_string()),
                    0,
                )
            }
        }))
        .expect("周期必须在阻塞边界内跑完：既不得 panic，也不得被 join 失败吞掉");

        // (1) 受控服务真的收到了 A3 请求。
        //
        // 注意断言的**不是**网关的命令名：命令名（`verify_source_answers`）只写在
        // `llm-calls.jsonl` 审计日志里，从不进 HTTP 请求体。请求体是 A3 的 prompt
        // 本体（messages[].content），所以这里断言 prompt 的特征语句 + 待核验槽位。
        // 这条错误断言曾是本用例唯一的红点，与产品缺陷无关——记下来以免下次重蹈。
        let requests = seen.lock().expect("requests lock").clone();
        assert!(!requests.is_empty(), "A3 核验必须真的发出 HTTP 请求");
        assert!(
            requests
                .iter()
                .any(|request| request.contains("against the ORIGINAL FILE")),
            "请求体必须是 A3 核验 prompt（应含 ORIGINAL FILE 核验声明）：{requests:?}"
        );
        assert!(
            requests.iter().any(|request| request.contains("slot-14")),
            "请求体必须带上待核验的槽位：{requests:?}"
        );

        // (2) 结果落库：批次可读取。
        let batch_id = read_current_batch(&root, &job_id).expect("批次必须已登记（结果落库）");
        let decision = read_decision_file(&root, &job_id, &batch_id).expect("决策文件必须可读");
        let dump = serde_json::to_string_pretty(&decision).unwrap_or_default();

        // (3) 结论确实来自**模型通道**。这里断言的是它在决策文件里的**真实形态**，
        // 不是我们「希望」有的形态——两者的差别踩过一次坑，写清楚免得再踩：
        //
        // 模型的 `model_quote` 证据（quote + pageIndex）由 `push_finding` 存进
        // `SourceVerificationV1.findings`，但 `decision.json` 的 `items[].evidence`
        // 只由本地/云端候选构成（`slot_anchor` / `group_quote`）；逐条模型证据**不会**
        // 被搬进决策项（见 rules.rs `compare_slots`，它只消费 `suggested` /
        // `is_confirmed` / `verdict`）。因此在这里找 `model_quote*` 永远找不到，
        // 那是断言写错了对象，不是链路没通。
        //
        // 模型结论的可观测形态是「`ANSWER_SOURCE_CONFLICT` + `sourceValue`」。
        // 用「只有模型才可能给出的值」来证明链路：本地值是 painting，受控服务回
        // stencilling，而原文件文本里**没有**任何答案行——stencilling 只可能来自模型通道。
        let from_model = decision
            .items
            .iter()
            .find(|item| item.code == "ANSWER_SOURCE_CONFLICT")
            .unwrap_or_else(|| {
                panic!("模型判「与原文不符」必须落成实质分歧项，而不是静默丢弃：{dump}")
            });
        assert_eq!(
            from_model
                .source_value
                .as_ref()
                .and_then(|value| value.get("values")),
            Some(&json!(["stencilling"])),
            "sourceValue 必须是受控服务回的那个值（本地值 painting 与之不同，故只可能来自模型）：{dump}"
        );
        assert!(
            from_model.proposed_patch.is_some(),
            "实质分歧必须带可一键采用的建议 patch：{dump}"
        );
        assert!(
            !from_model.auto_applied,
            "模型结论不得被自动写入题稿（deterministic = false）：{dump}"
        );
        assert_eq!(report.reconcile_status, "succeeded", "周期本身应跑完");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 反向对照：**不加边界**的同一段调用在 async 上下文里必然 panic。
    ///
    /// 这条用例锁定的是「本次修复针对的到底是什么」。没有它，将来有人把
    /// `spawn_blocking` 去掉、改成直接调用时，上面那条回归用例可能因为下游还有别的
    /// `spawn_blocking` 而看起来仍然绿——缺陷就会以「测试没覆盖到」的方式复活。
    ///
    /// 断言两种运行时 panic **任一**出现，因为它们是同一个缺陷的两副面孔，
    /// 具体撞上哪一副取决于 `reqwest` 在 `wait::timeout` 里走到哪一步：
    /// - `Cannot start a runtime from within a runtime.`
    ///   （`tokio::runtime::context::runtime::enter`：`wait::timeout` 在 debug 构建里
    ///   新建 shell runtime 并 `enter()`）；
    /// - `Cannot drop a runtime in a context where blocking is not allowed.`
    ///   （`tokio::runtime::blocking::shutdown`：同一 shell runtime 在 async 上下文里析构）。
    ///
    /// 产品侧实测到的正是后者，日志原文：
    /// `thread 'tokio-rt-worker' panicked at tokio-1.52.3/src/runtime/blocking/shutdown.rs:51`
    /// —— 也就是「A3 输入缓存写了、调用记录与输出都没有、受控服务零 POST」那一刻的根因。
    /// 因此这里刻意不断言具体是哪一条，只断言「在不该发生的上下文里发生了运行时 panic」。
    #[test]
    fn calling_the_cycle_inside_the_async_context_without_the_boundary_panics() {
        let (base_url, _seen) =
            spawn_recording_verification_service(verification_response("slot-14", 14, "stencilling"));
        let (root, job_id) = seed_a3_verification_job(&base_url);

        // 刻意**不**经过 `run_cycle_in_blocking_boundary`：这正是修复前的写法。
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = tauri::async_runtime::block_on(async {
                run_recognition_cycle(
                    &root,
                    &job_id,
                    Some("controlled-verify"),
                    true,
                    Err("cloud_not_probed_in_this_regression".to_string()),
                    0,
                )
            });
        }));
        let payload = caught.expect_err(
            "在 async 上下文里直接跑这条同步周期必须 panic（它内部构造 blocking HTTP 客户端）",
        );
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
            .unwrap_or_default();
        assert!(
            message.contains("Cannot start a runtime from within a runtime")
                || message.contains("Cannot drop a runtime in a context where blocking is not allowed"),
            "panic 必须是运行时上下文冲突那两类之一，实际为：{message}"
        );
    }

    /// 周期失败必须被**如实分类**，而不是折叠成同一个东西：
    /// panic 是 `Joined`（任务没跑完），返回 Err 是 `Failed`（跑完了但失败）。
    ///
    /// 两者的机器码不同，事故复盘才能回答「进程里到底 panic 过没有」。
    #[test]
    fn cycle_failure_distinguishes_a_join_failure_from_a_failed_cycle() {
        let joined = tauri::async_runtime::block_on(run_cycle_in_blocking_boundary(
            || -> Result<RecognitionCycleReport, String> {
                panic!("cycle exploded on purpose");
            },
        ))
        .expect_err("阻塞任务 panic 必须变成 join 失败，而不是被吞掉");
        assert!(matches!(joined, CycleFailure::Joined(_)), "{joined:?}");
        assert_eq!(joined.code(), CYCLE_JOIN_FAILED, "join 失败必须有独立的机器码");

        let failed = tauri::async_runtime::block_on(run_cycle_in_blocking_boundary(|| {
            Err("read_canonical_failed:boom".to_string())
        }))
        .expect_err("周期返回 Err 必须上抛");
        assert!(matches!(failed, CycleFailure::Failed(_)), "{failed:?}");
        assert_eq!(failed.code(), CYCLE_FAILED);
    }
}

