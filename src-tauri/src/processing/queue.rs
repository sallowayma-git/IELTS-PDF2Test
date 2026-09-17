//! M2（原 P6-T01）：durable 队列的存储操作。
//!
//! 权威状态在 `processing_jobs_v2`（SQLite）；每次执行使用独立 lease owner。
//! event_seq 仅用于前端忽略重复或乱序事件。
//! 这里只有纯 SQL 操作；运行时调度见 [`super::scheduler`]。

use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

use crate::CommandResult;

pub(crate) const STAGE_QUEUED: &str = "queued";
pub(crate) const STAGE_RUNNING: &str = "running";
pub(crate) const STAGE_LOCAL_RECOGNITION: &str = "local_recognition";
pub(crate) const STAGE_CLOUD_RECOGNITION: &str = "cloud_recognition";
/// 本地稿已形成、正在跑原文件核验与统一裁决的阶段。
pub(crate) const STAGE_RECONCILING: &str = "reconciling";
pub(crate) const STAGE_READY_FOR_REVIEW: &str = "ready_for_review";
pub(crate) const STAGE_FAILED: &str = "failed";
pub(crate) const STAGE_CANCELLED: &str = "cancelled";

const LEASE_SECONDS: i64 = 600;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessingJobRow {
    pub id: String,
    pub library_item_id: String,
    pub source_asset_id: String,
    pub stage: String,
    pub local_status: String,
    pub cloud_status: String,
    pub reconcile_status: String,
    pub progress: Value,
    pub actionable_count: i64,
    pub last_error_code: Option<String>,
    pub retry_count: i64,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<String>,
    /// durable 取消标记（G1/P0-2）：运行中取消必须落库，重启后不再复活。
    pub cancel_requested_at: Option<String>,
    pub event_seq: i64,
}

const JOB_COLUMNS: &str = "id, library_item_id, source_asset_id, stage, local_status, cloud_status, \
     reconcile_status, progress_json, actionable_count, last_error_code, retry_count, \
     lease_owner, lease_expires_at, cancel_requested_at, event_seq";

fn row_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProcessingJobRow> {
    let progress_json: String = row.get("progress_json")?;
    Ok(ProcessingJobRow {
        id: row.get("id")?,
        library_item_id: row.get("library_item_id")?,
        source_asset_id: row.get("source_asset_id")?,
        stage: row.get("stage")?,
        local_status: row.get("local_status")?,
        cloud_status: row.get("cloud_status")?,
        reconcile_status: row.get("reconcile_status")?,
        progress: serde_json::from_str(&progress_json).unwrap_or(Value::Null),
        actionable_count: row.get("actionable_count")?,
        last_error_code: row.get("last_error_code")?,
        retry_count: row.get("retry_count")?,
        lease_owner: row.get("lease_owner")?,
        lease_expires_at: row.get("lease_expires_at")?,
        cancel_requested_at: row.get("cancel_requested_at")?,
        event_seq: row.get("event_seq")?,
    })
}

/// 建立处理任务并入队。幂等：同一 id 重复入队被忽略。
pub(crate) fn enqueue(
    conn: &Connection,
    job_id: &str,
    library_item_id: &str,
    source_asset_id: &str,
    progress: &Value,
) -> CommandResult<bool> {
    let now = Utc::now().to_rfc3339();
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO processing_jobs_v2
             (id, library_item_id, source_asset_id, stage, local_status, cloud_status, reconcile_status,
              progress_json, actionable_count, retry_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'queued', 'not_started', 'not_started', 'not_started', ?4, 0, 0, ?5, ?5)",
            params![job_id, library_item_id, source_asset_id, progress.to_string(), now],
        )
        .map_err(|error| format!("processing_enqueue:{error}"))?;
    Ok(inserted > 0)
}

/// 原子认领：取一个 queued（或 lease 过期的 running）任务并写 lease。
/// 返回 None 表示当前没有可认领的任务。
pub(crate) fn claim_next(
    conn: &Connection,
    worker_id: &str,
) -> CommandResult<Option<ProcessingJobRow>> {
    let now = Utc::now().to_rfc3339();
    let lease_expires = (Utc::now() + Duration::seconds(LEASE_SECONDS)).to_rfc3339();
    conn.execute_batch("BEGIN IMMEDIATE;")
        .map_err(|error| format!("processing_claim_begin:{error}"))?;
    let claimed = (|| -> CommandResult<Option<ProcessingJobRow>> {
        // G1/A4-F05：lease 过期的 reclaim 不得复活任一阶段已成功的任务——
        // 成功结果一旦落库，reclaim 重跑会覆盖它（数据损坏路径）。
        // G1/P0-2：带 durable 取消标记的任务不得被认领，取消必须兑现。
        let job_id: Option<String> = conn
            .query_row(
                "SELECT id FROM processing_jobs_v2
                 WHERE stage = 'queued'
                    OR (stage IN ('running', 'local_recognition', 'cloud_recognition', 'reconciling')
                        AND (lease_expires_at IS NULL OR lease_expires_at < ?1)
                        AND cancel_requested_at IS NULL
                        AND (local_status IS NULL OR local_status != 'succeeded')
                        AND (cloud_status IS NULL OR cloud_status != 'succeeded'))
                 ORDER BY created_at LIMIT 1",
                params![now],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("processing_claim_select:{error}"))?;
        let Some(job_id) = job_id else {
            return Ok(None);
        };
        let updated = conn
            .execute(
                "UPDATE processing_jobs_v2
                 SET stage = 'running', lease_owner = ?2, lease_expires_at = ?3,
                     local_status = CASE WHEN stage != 'running' THEN 'running' ELSE local_status END,
                     event_seq = event_seq + 1, updated_at = ?1
                 WHERE id = ?4",
                params![now, worker_id, lease_expires, job_id],
            )
            .map_err(|error| format!("processing_claim_update:{error}"))?;
        if updated == 0 {
            return Ok(None);
        }
        let row = conn
            .query_row(
                &format!("SELECT {JOB_COLUMNS} FROM processing_jobs_v2 WHERE id = ?1"),
                [&job_id],
                row_from,
            )
            .optional()
            .map_err(|error| format!("processing_claim_row:{error}"))?;
        Ok(row)
    })();
    match claimed {
        Ok(result) => {
            conn.execute_batch("COMMIT;")
                .map_err(|error| format!("processing_claim_commit:{error}"))?;
            Ok(result)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(error)
        }
    }
}

/// 续租：只有当前持有者可以续。返回 false = lease 已易主/过期（当前 worker 必须放弃提交）。
pub(crate) fn renew_lease(conn: &Connection, job_id: &str, worker_id: &str) -> CommandResult<bool> {
    let lease_expires = (Utc::now() + Duration::seconds(LEASE_SECONDS)).to_rfc3339();
    let updated = conn
        .execute(
            "UPDATE processing_jobs_v2 SET lease_expires_at = ?3, updated_at = ?4
             WHERE id = ?1 AND lease_owner = ?2 AND lease_expires_at >= ?4
               AND stage IN ('running', 'local_recognition', 'cloud_recognition', 'reconciling')",
            params![job_id, worker_id, lease_expires, Utc::now().to_rfc3339()],
        )
        .map_err(|error| format!("processing_lease_renew:{error}"))?;
    Ok(updated > 0)
}

/// 阶段推进 + 事件序号自增。只有当前 lease 持有者可以推进。
/// 返回 `Some((event_seq, 有效阶段))`：带 durable 取消标记的行会被强制落
/// cancelled（G1 取消 TOCTOU），调用方必须以返回的有效阶段为准决定后续
/// 动作（例如不得再启动云调用），不得假设 stage 就是入参。
pub(crate) fn advance_stage(
    conn: &Connection,
    job_id: &str,
    worker_id: &str,
    stage: &str,
    local_status: Option<&str>,
    cloud_status: Option<&str>,
    reconcile_status: Option<&str>,
    actionable_count: Option<i64>,
    last_error_code: Option<&str>,
) -> CommandResult<Option<(i64, String)>> {
    let now = Utc::now().to_rfc3339();
    let lease_expires = (Utc::now() + Duration::seconds(LEASE_SECONDS)).to_rfc3339();
    // G1/P0-1：终态（ready_for_review/failed/cancelled）释放 lease，避免
    // 完成的任务残留过期 lease，也杜绝终态行被后续 renew/advance 命中。
    let clear_lease = matches!(
        stage,
        STAGE_READY_FOR_REVIEW | STAGE_FAILED | STAGE_CANCELLED
    );
    // G1 对抗审计 P1-1（取消 TOCTOU）：内存取消检查与 advance 提交之间存在
    // 窗口；durable 标记与推进在同一条 UPDATE 内判定——带取消标记的行推进
    // 任何阶段时强制落 cancelled 并释放 lease，取消不可能被迟到结果穿透。
    //
    // `last_error_code` 必须与阶段**同一条语句内**保持一致：取消不是失败，被强制落
    // cancelled 的行不得带上本次推进本来要写的失败码，否则 `display_message` 会按错误码
    // 把用户自己取消的任务显示成「识别失败，可以重试」。
    let updated = conn
        .execute(
            "UPDATE processing_jobs_v2
             SET stage = CASE WHEN cancel_requested_at IS NOT NULL THEN 'cancelled' ELSE ?3 END,
                 local_status = COALESCE(?4, local_status),
                 cloud_status = COALESCE(?5, cloud_status),
                 reconcile_status = COALESCE(?6, reconcile_status),
                 actionable_count = COALESCE(?7, actionable_count),
                 last_error_code = CASE
                     WHEN cancel_requested_at IS NOT NULL THEN 'cancelled'
                     ELSE COALESCE(?8, last_error_code)
                 END,
                 lease_owner = CASE WHEN ?11 OR cancel_requested_at IS NOT NULL THEN NULL ELSE ?2 END,
                 lease_expires_at = CASE WHEN ?11 OR cancel_requested_at IS NOT NULL THEN NULL ELSE ?9 END,
                 event_seq = event_seq + 1,
                 updated_at = ?10
             WHERE id = ?1 AND lease_owner = ?2 AND lease_expires_at >= ?10",
            params![
                job_id,
                worker_id,
                stage,
                local_status,
                cloud_status,
                reconcile_status,
                actionable_count,
                last_error_code,
                lease_expires,
                now,
                clear_lease
            ],
        )
        .map_err(|error| format!("processing_advance:{error}"))?;
    if updated == 0 {
        return Ok(None);
    }
    let (seq, effective_stage): (i64, String) = conn
        .query_row(
            "SELECT event_seq, stage FROM processing_jobs_v2 WHERE id = ?1",
            [job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("processing_advance_seq:{error}"))?;
    Ok(Some((seq, effective_stage)))
}

/// 仅更新 `cloud_status`，不切换 `stage`。用于「本地识别仍在跑、云端已排队/起飞」
/// 的可观测信号：此时 `stage` 应忠实停留在 `local_recognition`（percent=45），
/// 而不是被提前推进到 `cloud_recognition`（percent=70），否则进度条会短暂虚高。
/// 带 durable 取消标记或 lease 丢失时返回 `None`，由调用方中止云端链。
pub(crate) fn set_cloud_status(
    conn: &Connection,
    job_id: &str,
    worker_id: &str,
    cloud_status: &str,
) -> CommandResult<Option<(i64, String)>> {
    let now = Utc::now().to_rfc3339();
    // 取消优先：durable 取消标记存在时拒绝推进，返回 None 让调度器中止云端。
    let cancelled = conn
        .query_row(
            "SELECT 1 FROM processing_jobs_v2 WHERE id = ?1 AND cancel_requested_at IS NOT NULL",
            [job_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| format!("processing_cloud_status_cancel:{error}"))?
        .is_some();
    if cancelled {
        return Ok(None);
    }
    let updated = conn
        .execute(
            "UPDATE processing_jobs_v2
             SET cloud_status = ?3,
                 event_seq = event_seq + 1,
                 updated_at = ?4
             WHERE id = ?1 AND lease_owner = ?2 AND lease_expires_at >= ?4",
            params![job_id, worker_id, cloud_status, now],
        )
        .map_err(|error| format!("processing_cloud_status:{error}"))?;
    if updated == 0 {
        return Ok(None);
    }
    let (seq, effective_stage): (i64, String) = conn
        .query_row(
            "SELECT event_seq, stage FROM processing_jobs_v2 WHERE id = ?1",
            [job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("processing_advance_seq:{error}"))?;
    Ok(Some((seq, effective_stage)))
}

/// 读取一行（事件 payload 与调度器用）。
pub(crate) fn get_job(conn: &Connection, job_id: &str) -> CommandResult<Option<ProcessingJobRow>> {
    conn.query_row(
        &format!("SELECT {JOB_COLUMNS} FROM processing_jobs_v2 WHERE id = ?1"),
        [job_id],
        row_from,
    )
    .optional()
    .map_err(|error| format!("processing_get:{error}"))
}

/// G1 对抗审计 P1-3：lease 丢失后的终态收尾。本地已成功 + 仍在运行阶段时，
/// reclaim 守卫保证没有其他 worker 能接手（不会与在跑 worker 竞争），允许
/// 免 lease 提交 ready_for_review，避免任务在"云端识别中"悬挂到重启。
/// 带 durable 取消标记的行拒绝收尾（取消优先）。返回 false = 条件不满足。
pub(crate) fn finalize_ready_without_lease(
    conn: &Connection,
    job_id: &str,
    cloud_status: &str,
    reconcile_status: &str,
) -> CommandResult<bool> {
    let now = Utc::now().to_rfc3339();
    let updated = conn
        .execute(
            "UPDATE processing_jobs_v2
             SET stage = 'ready_for_review',
                 cloud_status = COALESCE(?2, cloud_status),
                 reconcile_status = COALESCE(?3, reconcile_status),
                 lease_owner = NULL, lease_expires_at = NULL,
                 event_seq = event_seq + 1, updated_at = ?4
             WHERE id = ?1
               AND stage IN ('running', 'local_recognition', 'cloud_recognition', 'reconciling')
               AND local_status = 'succeeded'
               AND cancel_requested_at IS NULL",
            params![job_id, cloud_status, reconcile_status, now],
        )
        .map_err(|error| format!("processing_finalize:{error}"))?;
    Ok(updated > 0)
}

/// G1 边界（复核 B）：lease 丢失后取消的收尾。带 durable 取消标记的运行中
/// 行免 lease 落 cancelled——reclaim 守卫保证带标记的行不会被任何 worker
/// 认领，迟到提交不会与在跑 worker 竞争；没有这一步，取消只能靠重启恢复
/// 兑现。返回 false = 条件不满足。
pub(crate) fn finalize_cancelled_without_lease(
    conn: &Connection,
    job_id: &str,
) -> CommandResult<bool> {
    let now = Utc::now().to_rfc3339();
    let updated = conn
        .execute(
            "UPDATE processing_jobs_v2
             SET stage = 'cancelled', lease_owner = NULL, lease_expires_at = NULL,
                 last_error_code = 'cancelled',
                 event_seq = event_seq + 1, updated_at = ?2
             WHERE id = ?1
               AND stage IN ('running', 'local_recognition', 'cloud_recognition', 'reconciling')
               AND cancel_requested_at IS NOT NULL",
            params![job_id, now],
        )
        .map_err(|error| format!("processing_finalize_cancelled:{error}"))?;
    Ok(updated > 0)
}

/// 启动恢复（计划 §12.5）：running 任务标记 interrupted；
/// retry_count < 上限则重新入队，否则转入 action_required 等用户重试。
/// G1/A4-F03：恢复上限路径的文案必须是"已达重试上限"，不得谎称已自动重试。
/// G1/P0-2：用户已取消（durable 标记）的任务直接落 cancelled，不复活。
pub(crate) fn recover_on_startup(conn: &Connection, max_auto_recovery: i64) -> CommandResult<usize> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE processing_jobs_v2
         SET stage = 'cancelled', lease_owner = NULL, lease_expires_at = NULL,
             last_error_code = 'cancelled', event_seq = event_seq + 1, updated_at = ?1
         WHERE stage IN ('running', 'local_recognition', 'cloud_recognition', 'reconciling')
           AND cancel_requested_at IS NOT NULL",
        params![now],
    )
    .map_err(|error| format!("processing_recovery_cancelled:{error}"))?;
    let requeued = conn
        .execute(
            "UPDATE processing_jobs_v2
             SET stage = 'queued', lease_owner = NULL, lease_expires_at = NULL,
                 last_error_code = 'interrupted', retry_count = retry_count + 1,
                 event_seq = event_seq + 1, updated_at = ?2
             WHERE stage IN ('running', 'local_recognition', 'cloud_recognition', 'reconciling') AND retry_count < ?1",
            params![max_auto_recovery, now],
        )
        .map_err(|error| format!("processing_recovery_requeue:{error}"))?;
    conn.execute(
        "UPDATE processing_jobs_v2
         SET stage = 'ready_for_review', local_status = 'action_required',
             last_error_code = 'retry_exhausted', lease_owner = NULL, lease_expires_at = NULL,
             event_seq = event_seq + 1, updated_at = ?2
         WHERE stage IN ('running', 'local_recognition', 'cloud_recognition', 'reconciling') AND retry_count >= ?1",
        params![max_auto_recovery, now],
    )
    .map_err(|error| format!("processing_recovery_action_required:{error}"))?;
    Ok(requeued)
}

/// 用户重试：failed/可重试条目重新入队。
pub(crate) fn retry(conn: &Connection, job_id: &str) -> CommandResult<bool> {
    let now = Utc::now().to_rfc3339();
    let updated = conn
        .execute(
            "UPDATE processing_jobs_v2
             SET stage = 'queued', local_status = 'not_started', cloud_status = 'not_started',
                 reconcile_status = 'not_started', last_error_code = NULL,
                 cancel_requested_at = NULL,
                 retry_count = retry_count + 1, lease_owner = NULL, lease_expires_at = NULL,
                 event_seq = event_seq + 1, updated_at = ?2
             WHERE id = ?1 AND stage IN ('failed', 'ready_for_review', 'cancelled')",
            params![job_id, now],
        )
        .map_err(|error| format!("processing_retry:{error}"))?;
    Ok(updated > 0)
}

/// 用户取消：queued 立即取消；running 落 durable 取消标记，由 worker 在阶段
/// 边界检查后收尾（G1/P0-2：标记落库，重启恢复时也必须兑现，不得复活）。
pub(crate) fn request_cancel(conn: &Connection, job_id: &str) -> CommandResult<bool> {
    let now = Utc::now().to_rfc3339();
    let cancelled = conn
        .execute(
            "UPDATE processing_jobs_v2
             SET stage = 'cancelled', lease_owner = NULL, lease_expires_at = NULL,
                 cancel_requested_at = ?2,
                 event_seq = event_seq + 1, updated_at = ?2
             WHERE id = ?1 AND stage = 'queued'",
            params![job_id, now],
        )
        .map_err(|error| format!("processing_cancel:{error}"))?;
    if cancelled > 0 {
        return Ok(true);
    }
    // running 阶段：只落标记，stage 仍由持有 lease 的 worker 推进到终态。
    let marked = conn
        .execute(
            "UPDATE processing_jobs_v2
             SET cancel_requested_at = ?2, event_seq = event_seq + 1, updated_at = ?2
             WHERE id = ?1 AND stage IN ('running', 'local_recognition', 'cloud_recognition', 'reconciling')",
            params![job_id, now],
        )
        .map_err(|error| format!("processing_cancel_mark:{error}"))?;
    Ok(marked > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::schema::ensure_v2_schema;

    fn memory_queue() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_v2_schema(&conn).unwrap();
        conn
    }

    fn seed_item(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO library_items_v2 (id, modality, title, status, created_at, updated_at)
             VALUES (?1, 'reading', 't', 'processing', '2026-01-01', '2026-01-01')",
            [id],
        )
        .unwrap();
    }

    #[test]
    fn enqueue_claim_and_stale_worker_cannot_commit() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        assert!(enqueue(&conn, "job-1", "it-1", "asset-1", &serde_json::json!({"cloudEnabled": false})).unwrap());
        assert!(!enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap(), "重复入队幂等");

        let worker_a = "worker-a";
        let claimed = claim_next(&conn, worker_a).unwrap().expect("job must be claimable");
        assert_eq!(claimed.id, "job-1");
        assert_eq!(claimed.stage, "running");

        // 第二个 worker 认领不到（没有 queued，lease 未过期）。
        assert!(claim_next(&conn, "worker-b").unwrap().is_none());

        // 阶段推进：只有持有者可以。
        let (seq, effective) = advance_stage(&conn, "job-1", worker_a, STAGE_LOCAL_RECOGNITION, Some("succeeded"), None, None, None, None)
            .unwrap()
            .expect("holder must advance");
        assert_eq!(seq, 2);
        assert_eq!(effective, STAGE_LOCAL_RECOGNITION, "无取消标记时有效阶段即目标阶段");
        // 他人推进被拒。
        assert!(advance_stage(&conn, "job-1", "worker-b", STAGE_FAILED, Some("failed"), None, None, None, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn expired_lease_is_reclaimable_and_recovery_requeues() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-a").unwrap();

        // 人为把 lease 过期时间拨回过去。
        conn.execute(
            "UPDATE processing_jobs_v2 SET stage = 'local_recognition', lease_expires_at = '2020-01-01T00:00:00Z' WHERE id = 'job-1'",
            [],
        )
        .unwrap();
        assert!(!renew_lease(&conn, "job-1", "worker-a").unwrap());
        let reclaimed = claim_next(&conn, "worker-b").unwrap().expect("expired lease must be reclaimable");
        assert_eq!(reclaimed.lease_owner.as_deref(), Some("worker-b"));

        // 启动恢复：running 任务按 retry_count 重新入队或转 action_required。
        assert!(advance_stage(&conn, "job-1", "worker-a", STAGE_FAILED, None, None, None, None, None).unwrap().is_none());
        advance_stage(&conn, "job-1", "worker-b", STAGE_CLOUD_RECOGNITION, None, None, None, None, None).unwrap();
        let requeued = recover_on_startup(&conn, 3).unwrap();
        assert_eq!(requeued, 1);
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_QUEUED);
        assert_eq!(row.last_error_code.as_deref(), Some("interrupted"));
    }

    #[test]
    fn retry_and_cancel_only_touch_expected_stages() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap();
        // queued 状态不能 retry（还没失败）。
        assert!(!retry(&conn, "job-1").unwrap());
        // queued 可以直接取消。
        assert!(request_cancel(&conn, "job-1").unwrap());
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_CANCELLED);
        // cancelled 之后可以 retry（重新入队）。
        assert!(retry(&conn, "job-1").unwrap());
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_QUEUED);
        assert!(row.cancel_requested_at.is_none(), "retry 必须清除 durable 取消标记");
    }

    // ── G1 数据安全护栏回归 ────────────────────────────────────────────

    /// G1/A4-F05：lease 过期的 reclaim 不得复活任一阶段已成功的任务。
    #[test]
    fn reclaim_never_resurrects_succeeded_stages() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-a").unwrap();
        // 本地已成功、推进到云端阶段后 worker 死亡、lease 过期。
        advance_stage(&conn, "job-1", "worker-a", STAGE_CLOUD_RECOGNITION, Some("succeeded"), Some("running"), None, None, None).unwrap();
        conn.execute(
            "UPDATE processing_jobs_v2 SET lease_expires_at = '2020-01-01T00:00:00Z' WHERE id = 'job-1'",
            [],
        )
        .unwrap();
        assert!(
            claim_next(&conn, "worker-b").unwrap().is_none(),
            "local 已成功的任务不得被 reclaim 重跑"
        );
        // 云端也成功（worker 在收尾前死亡）：恢复 lease 后收尾，终态必须释放 lease。
        conn.execute(
            "UPDATE processing_jobs_v2 SET lease_expires_at = '2099-01-01T00:00:00Z' WHERE id = 'job-1'",
            [],
        )
        .unwrap();
        advance_stage(&conn, "job-1", "worker-a", STAGE_READY_FOR_REVIEW, None, Some("succeeded"), Some("succeeded"), None, None).unwrap();
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.lease_owner, None, "G1/P0-1：终态必须释放 lease");
        // 对照：两个阶段都未成功的过期任务仍可 reclaim（恢复路径不受影响）。
        enqueue(&conn, "job-2", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-c").unwrap();
        advance_stage(&conn, "job-2", "worker-c", STAGE_LOCAL_RECOGNITION, Some("running"), None, None, None, None).unwrap();
        conn.execute(
            "UPDATE processing_jobs_v2 SET lease_expires_at = '2020-01-01T00:00:00Z' WHERE id = 'job-2'",
            [],
        )
        .unwrap();
        assert!(claim_next(&conn, "worker-d").unwrap().is_some());
    }

    /// G1/A4-F03：恢复上限路径必须写 retry_exhausted（UI 据此停止谎报"已自动重试"）。
    #[test]
    fn recover_on_startup_marks_retry_exhausted() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-a").unwrap();
        conn.execute(
            "UPDATE processing_jobs_v2 SET retry_count = 3 WHERE id = 'job-1'",
            [],
        )
        .unwrap();
        let requeued = recover_on_startup(&conn, 3).unwrap();
        assert_eq!(requeued, 0);
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_READY_FOR_REVIEW);
        assert_eq!(row.local_status, "action_required");
        assert_eq!(
            row.last_error_code.as_deref(),
            Some("retry_exhausted"),
            "恢复上限后不得保留 interrupted（谎称已自动重试）"
        );
    }

    /// G1 对抗审计 P1-1（取消 TOCTOU）：内存取消检查与 advance 提交之间
    /// 发生的取消必须被 advance 原子捕获——带 durable 标记的行推进任何
    /// 阶段都强制落 cancelled 并释放 lease，迟到结果不得穿透到 ready。
    #[test]
    fn advance_with_durable_cancel_marker_lands_cancelled() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-a").unwrap();
        advance_stage(&conn, "job-1", "worker-a", STAGE_CLOUD_RECOGNITION, Some("succeeded"), Some("running"), None, None, None).unwrap();

        // 用户在检查之后、提交之前取消（落 durable 标记，不动 stage）。
        conn.execute(
            "UPDATE processing_jobs_v2 SET cancel_requested_at = '2026-09-12T00:00:00Z' WHERE id = 'job-1'",
            [],
        )
        .unwrap();

        // worker 按原计划推进 ready：必须被强制落 cancelled。
        let (seq, effective) = advance_stage(&conn, "job-1", "worker-a", STAGE_READY_FOR_REVIEW, Some("succeeded"), Some("succeeded"), Some("succeeded"), None, None)
            .unwrap()
            .expect("lease 仍有效时推进必须成功（但落 cancelled）");
        assert!(seq > 0);
        assert_eq!(effective, STAGE_CANCELLED, "调用方拿到的必须是有效阶段");
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_CANCELLED, "迟到结果不得穿透取消");
        assert_eq!(row.lease_owner, None);
    }

    /// 取消与失败码必须在**同一条推进语句内**保持一致：被强制落 cancelled 的行不得
    /// 保留本次推进本来要写的失败码。否则 `display_message` 先读错误码，会把用户自己
    /// 取消的任务显示成「识别失败，可以重试」——把取消谎报成失败（G1/A4-F03 禁止）。
    #[test]
    fn coerced_cancel_does_not_keep_the_failure_code_written_by_the_same_advance() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-a").unwrap();
        conn.execute(
            "UPDATE processing_jobs_v2 SET cancel_requested_at = '2026-09-12T00:00:00Z' WHERE id = 'job-1'",
            [],
        )
        .unwrap();

        // worker 按原计划落失败终态（带机器码）。
        let (_, effective) = advance_stage(
            &conn,
            "job-1",
            "worker-a",
            STAGE_FAILED,
            Some("succeeded"),
            Some("failed"),
            Some("failed"),
            Some(0),
            Some("RECONCILE_FAILED"),
        )
        .unwrap()
        .expect("lease 仍有效时推进必须成功（但落 cancelled）");
        assert_eq!(effective, STAGE_CANCELLED);
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_CANCELLED);
        assert_eq!(
            row.last_error_code.as_deref(),
            Some("cancelled"),
            "取消不是失败：被强制落 cancelled 的行不得带上同时刻写入的失败码"
        );
    }

    /// G1 边界（复核 B）：等待 cloud permit / 推进期间取消——advance 返回的
    /// 有效阶段是 cancelled，调度器据此退出且不再启动云调用（数据层语义）。
    #[test]
    fn cloud_stage_advance_reports_coerced_cancel() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-a").unwrap();
        conn.execute(
            "UPDATE processing_jobs_v2 SET cancel_requested_at = '2026-09-12T00:00:00Z' WHERE id = 'job-1'",
            [],
        )
        .unwrap();
        let (_, effective) = advance_stage(&conn, "job-1", "worker-a", STAGE_CLOUD_RECOGNITION, Some("succeeded"), Some("running"), None, None, None)
            .unwrap()
            .expect("lease 仍有效");
        assert_eq!(effective, STAGE_CANCELLED);
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_CANCELLED);
        assert_eq!(row.lease_owner, None);
    }

    /// G1 边界（复核 B）：lease 过期后取消——advance 无法提交时，免 lease
    /// 收尾必须能落 cancelled（取消不必等重启兑现）；无 durable 标记则拒绝。
    #[test]
    fn finalize_cancelled_without_lease_requires_durable_marker() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-a").unwrap();
        advance_stage(&conn, "job-1", "worker-a", STAGE_LOCAL_RECOGNITION, Some("running"), None, None, None, None).unwrap();

        // 无 durable 标记：拒绝收尾（内存标记丢失时不得凭空取消）。
        assert!(!finalize_cancelled_without_lease(&conn, "job-1").unwrap());

        // 用户取消（durable 标记落库）后 worker 的 lease 过期、advance 失败：
        // 免 lease 收尾兑现取消。
        assert!(request_cancel(&conn, "job-1").unwrap());
        conn.execute(
            "UPDATE processing_jobs_v2 SET lease_expires_at = '2020-01-01T00:00:00Z' WHERE id = 'job-1'",
            [],
        )
        .unwrap();
        assert!(advance_stage(&conn, "job-1", "worker-a", STAGE_CANCELLED, None, None, None, None, None)
            .unwrap()
            .is_none());
        assert!(finalize_cancelled_without_lease(&conn, "job-1").unwrap());
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_CANCELLED);
        assert_eq!(row.lease_owner, None);
        assert_eq!(row.last_error_code.as_deref(), Some("cancelled"));
    }

    /// G1 对抗审计 P1-3：lease 丢失后的免 lease 终态收尾；取消标记优先、
    /// 未完成阶段拒绝收尾。
    #[test]
    fn finalize_ready_without_lease_only_for_succeeded_local() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-a").unwrap();
        advance_stage(&conn, "job-1", "worker-a", STAGE_CLOUD_RECOGNITION, Some("succeeded"), Some("running"), None, None, None).unwrap();

        // local 未成功时拒绝收尾。
        conn.execute("UPDATE processing_jobs_v2 SET local_status = 'running' WHERE id = 'job-1'", []).unwrap();
        assert!(!finalize_ready_without_lease(&conn, "job-1", "succeeded", "succeeded").unwrap());

        // local 成功后允许收尾（模拟 lease 已丢、advance 返回 None 的场景）。
        conn.execute("UPDATE processing_jobs_v2 SET local_status = 'succeeded' WHERE id = 'job-1'", []).unwrap();
        assert!(finalize_ready_without_lease(&conn, "job-1", "succeeded", "succeeded").unwrap());
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_READY_FOR_REVIEW);
        assert_eq!(row.cloud_status, "succeeded");
        assert_eq!(row.lease_owner, None);

        // 带 durable 取消标记的行拒绝收尾（取消优先于迟到结果）。
        enqueue(&conn, "job-2", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-b").unwrap();
        advance_stage(&conn, "job-2", "worker-b", STAGE_CLOUD_RECOGNITION, Some("succeeded"), Some("running"), None, None, None).unwrap();
        conn.execute(
            "UPDATE processing_jobs_v2 SET cancel_requested_at = '2026-09-12T00:00:00Z' WHERE id = 'job-2'",
            [],
        )
        .unwrap();
        assert!(!finalize_ready_without_lease(&conn, "job-2", "succeeded", "succeeded").unwrap());
    }

    /// G1/A4-F02 + P0-2：运行中取消必须落 durable 标记；重启恢复兑现取消，
    /// 不得把已取消任务重新入队；认领路径不得捡起带取消标记的任务。
    #[test]
    fn durable_cancel_survives_restart() {
        let conn = memory_queue();
        seed_item(&conn, "it-1");
        enqueue(&conn, "job-1", "it-1", "asset-1", &Value::Null).unwrap();
        claim_next(&conn, "worker-a").unwrap();
        advance_stage(&conn, "job-1", "worker-a", STAGE_LOCAL_RECOGNITION, Some("running"), None, None, None, None).unwrap();

        // 用户取消运行中任务：stage 不变（worker 收尾），但标记落库。
        assert!(request_cancel(&conn, "job-1").unwrap());
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_LOCAL_RECOGNITION);
        assert!(row.cancel_requested_at.is_some(), "运行中取消必须持久化");

        // lease 过期后也不得被其他 worker 认领（取消必须兑现）。
        conn.execute(
            "UPDATE processing_jobs_v2 SET lease_expires_at = '2020-01-01T00:00:00Z' WHERE id = 'job-1'",
            [],
        )
        .unwrap();
        assert!(claim_next(&conn, "worker-b").unwrap().is_none());

        // 重启恢复：带取消标记的任务直接落 cancelled，不复活。
        recover_on_startup(&conn, 3).unwrap();
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_CANCELLED);

        // 用户重试后标记清除，任务可重新入队。
        assert!(retry(&conn, "job-1").unwrap());
        let row = get_job(&conn, "job-1").unwrap().unwrap();
        assert_eq!(row.stage, STAGE_QUEUED);
        assert!(row.cancel_requested_at.is_none());
    }
}
