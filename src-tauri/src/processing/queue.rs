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
    pub event_seq: i64,
}

const JOB_COLUMNS: &str = "id, library_item_id, source_asset_id, stage, local_status, cloud_status, \
     reconcile_status, progress_json, actionable_count, last_error_code, retry_count, \
     lease_owner, lease_expires_at, event_seq";

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
        let job_id: Option<String> = conn
            .query_row(
                "SELECT id FROM processing_jobs_v2
                 WHERE stage = 'queued'
                    OR (stage IN ('running', 'local_recognition', 'cloud_recognition', 'reconciling')
                        AND (lease_expires_at IS NULL OR lease_expires_at < ?1))
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
) -> CommandResult<Option<i64>> {
    let now = Utc::now().to_rfc3339();
    let lease_expires = (Utc::now() + Duration::seconds(LEASE_SECONDS)).to_rfc3339();
    let updated = conn
        .execute(
            "UPDATE processing_jobs_v2
             SET stage = ?3,
                 local_status = COALESCE(?4, local_status),
                 cloud_status = COALESCE(?5, cloud_status),
                 reconcile_status = COALESCE(?6, reconcile_status),
                 actionable_count = COALESCE(?7, actionable_count),
                 last_error_code = COALESCE(?8, last_error_code),
                 lease_owner = ?2,
                 lease_expires_at = ?9,
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
                now
            ],
        )
        .map_err(|error| format!("processing_advance:{error}"))?;
    if updated == 0 {
        return Ok(None);
    }
    let seq: i64 = conn
        .query_row(
            "SELECT event_seq FROM processing_jobs_v2 WHERE id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("processing_advance_seq:{error}"))?;
    Ok(Some(seq))
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

/// 启动恢复（计划 §12.5）：running 任务标记 interrupted；
/// retry_count < 上限则重新入队，否则转入 action_required 等用户重试。
pub(crate) fn recover_on_startup(conn: &Connection, max_auto_recovery: i64) -> CommandResult<usize> {
    let now = Utc::now().to_rfc3339();
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
             last_error_code = 'interrupted', lease_owner = NULL, lease_expires_at = NULL,
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
                 retry_count = retry_count + 1, lease_owner = NULL, lease_expires_at = NULL,
                 event_seq = event_seq + 1, updated_at = ?2
             WHERE id = ?1 AND stage IN ('failed', 'ready_for_review', 'cancelled')",
            params![job_id, now],
        )
        .map_err(|error| format!("processing_retry:{error}"))?;
    Ok(updated > 0)
}

/// 用户取消：queued 立即取消；running 由 worker 在阶段边界检查取消标记后收尾。
pub(crate) fn request_cancel(conn: &Connection, job_id: &str) -> CommandResult<bool> {
    let now = Utc::now().to_rfc3339();
    let updated = conn
        .execute(
            "UPDATE processing_jobs_v2
             SET stage = 'cancelled', lease_owner = NULL, lease_expires_at = NULL,
                 event_seq = event_seq + 1, updated_at = ?2
             WHERE id = ?1 AND stage = 'queued'",
            params![job_id, now],
        )
        .map_err(|error| format!("processing_cancel:{error}"))?;
    Ok(updated > 0)
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
        let seq = advance_stage(&conn, "job-1", worker_a, STAGE_LOCAL_RECOGNITION, Some("succeeded"), None, None, None, None)
            .unwrap()
            .expect("holder must advance");
        assert_eq!(seq, 2);
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
    }
}
