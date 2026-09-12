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
    self, advance_stage, claim_next, finalize_ready_without_lease, get_job, renew_lease,
    request_cancel, retry, STAGE_CLOUD_RECOGNITION, STAGE_FAILED, STAGE_LOCAL_RECOGNITION,
    STAGE_READY_FOR_REVIEW,
};
use crate::auto_pipeline::{run_auto_pipeline_core, run_cloud_review_core};
use crate::library::repository::open_library_connection;
use crate::{app_root, AutoPipelineInput, RunCloudReviewInput};

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
        "displayMessage": display_message(&job.stage, job.last_error_code.as_deref()),
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

    // ── 本地识别（阻塞线程池；持 local permit）───────────────────────
    let local_permit = state.local_permits.clone().acquire_owned().await;
    let advanced = advance(&app, &state, &job_id, STAGE_LOCAL_RECOGNITION, Some("running"), None, None, None, None).await;
    if advanced.is_none() {
        return; // lease 丢失 / 已取消
    }
    let cancelled = state.cancelled.read().await.contains(&job_id);
    if cancelled {
        finish_cancelled(&app, &state, &job_id).await;
        return;
    }
    let Ok(root) = app_root(&app) else { return };
    let local_result = tauri::async_runtime::spawn_blocking({
        let root = root.clone();
        let job_id = job_id.clone();
        move || {
            run_auto_pipeline_core(
                &root,
                &job_id,
                Some(AutoPipelineInput {
                    confidence_threshold: Some(0.85),
                    execution_mode: Some("localOnly".to_string()),
                    target: Some("editableDraft".to_string()),
                    ..Default::default()
                }),
            )
        }
    })
    .await;
    drop(local_permit);
    let local_result = match local_result {
        Ok(result) => result,
        Err(error) => Err(format!("processing_join:{error}")),
    };
    if let Err(error) = &local_result {
        fail_job(&app, &state, &job_id, &error).await;
        return;
    }
    // G1/A4-F02：本地识别期间发生的取消必须在此兑现；否则无云端路径会
    // 直接推进 ready_for_review，取消被静默吞掉。
    if state.cancelled.read().await.contains(&job_id) {
        finish_cancelled(&app, &state, &job_id).await;
        return;
    }

    // ── 云端识别（可选；独立 permit；失败不取消本地结果）──────────────
    let cloud_enabled = job
        .progress
        .get("cloudEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(state.settings.cloud_enabled_default);
    let is_pdf = job
        .progress
        .get("fileName")
        .and_then(Value::as_str)
        .map(|name| name.to_ascii_lowercase().ends_with(".pdf"))
        .unwrap_or(false);
    let cloud_profile_id = job
        .progress
        .get("cloudProfileId")
        .and_then(Value::as_str)
        .map(str::to_string);
    if cloud_enabled && is_pdf {
        let cancelled = state.cancelled.read().await.contains(&job_id);
        if cancelled {
            finish_cancelled(&app, &state, &job_id).await;
            return;
        }
        let _cloud_permit = state.cloud_permits.clone().acquire_owned().await;
        if advance(
            &app,
            &state,
            &job_id,
            STAGE_CLOUD_RECOGNITION,
            Some("succeeded"),
            Some("running"),
            None,
            None,
            None,
        )
        .await
        .is_none()
        {
            return;
        }
        let cloud_result = tauri::async_runtime::spawn_blocking({
            let root = root.clone();
            let job_id = job_id.clone();
            move || run_cloud_review_core(&root, &job_id, Some(RunCloudReviewInput { profile_id: cloud_profile_id }))
        })
        .await;
        let cloud_status = match cloud_result {
            Ok(Ok(_)) => "succeeded",
            Ok(Err(_)) | Err(_) => "failed",
        };
        // G1/A4-F02：云端执行期间发生的取消必须在推进 ready 之前兑现，
        // 迟到的云端结果不得把已取消的任务推进到可检查状态。
        if state.cancelled.read().await.contains(&job_id) {
            finish_cancelled(&app, &state, &job_id).await;
            return;
        }
        let advance_result = advance(
            &app,
            &state,
            &job_id,
            STAGE_READY_FOR_REVIEW,
            Some("succeeded"),
            Some(cloud_status),
            Some("succeeded"),
            None,
            None,
        )
        .await;
        if advance_result.is_none() {
            // G1 对抗审计 P1-3：lease 丢失（如睡眠唤醒、心跳瞬断）时允许
            // 免 lease 终态收尾——local 已成功 + reclaim 守卫保证无人接手。
            let finalized = tauri::async_runtime::spawn_blocking({
                let root = root.clone();
                let job_id = job_id.clone();
                let cloud_status = cloud_status.to_string();
                move || {
                    let conn = open_library_connection(&root)?;
                    finalize_ready_without_lease(&conn, &job_id, &cloud_status, "succeeded")
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
        return;
    }

    // 无云端：直接到 ready_for_review（本地稿可检查可编辑，计划 §12.3）。
    if advance(
        &app,
        &state,
        &job_id,
        STAGE_READY_FOR_REVIEW,
        Some("succeeded"),
        Some("skipped"),
        Some("skipped"),
        None,
        None,
    )
    .await
    .is_some()
    {
        set_item_status_ready(&app, &job_id).await;
    }
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
) -> Option<i64> {
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
        Ok(Ok(Some(seq))) => {
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
            Some(seq)
        }
        _ => None,
    }
}

async fn fail_job(app: &AppHandle, state: &Arc<ProcessingState>, job_id: &str, error: &str) {
    // 只保留稳定错误码；完整错误在应用日志里。
    let code = error.split(':').next().unwrap_or("processing_failed").to_string();
    let code = code.chars().take(80).collect::<String>();
    if advance(
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
    .await
    .is_some()
    {
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
    advance(app, state, job_id, queue::STAGE_CANCELLED, None, None, None, None, None).await;
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
