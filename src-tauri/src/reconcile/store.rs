//! 识别闭环的持久化。
//!
//! 两层职责分开：
//! - **数据库是读取权威**（`recognition_batches_v1` / `recognition_decisions_v1`
//!   / `recognition_decision_journal_v1`）：`get_recognition_decision` 只读数据库，
//!   因此过程文件被清理策略回收也不影响前端。
//! - **job 目录下的 JSON 是证据留痕**：逐条候选与核验结论可事后复查，
//!   但不参与产品决策。

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::source::SourceVerificationV1;
use crate::schema::cloud_repair_v1::CloudAuthoringCandidateV1;
use crate::schema::recognition_v1::{
    ChainStatusSummaryV1, ChainStatusV1, DecisionItemV1, DecisionSummaryV1,
    RecognitionCandidateV1, RecognitionChainStateV1, RecognitionDecisionV1,
    RECOGNITION_DECISION_V1_SCHEMA_VERSION,
};
use crate::util::{read_json_opt, safe_job_dir, write_json};
use crate::CommandResult;

pub(crate) const LOCAL_CANDIDATE_FILE: &str = "local-candidate.json";
pub(crate) const CLOUD_CANDIDATE_FILE: &str = "cloud-candidate.json";
pub(crate) const CLOUD_AUTHORING_CANDIDATE_FILE: &str = "cloud-authoring-candidate.json";
/// 修复运行的摘要（诊断副本）。**完成判据始终是当前 canonical**，不是这份摘要。
pub(crate) const REPAIR_SUMMARY_FILE: &str = "repair.json";
/// 差异裁定记录（**产品状态**，不是诊断副本）。
///
/// 与 `repair.json` 的性质不同：摘要只是给前端看的诊断副本，丢了可以按当前稿重算；
/// 裁定记录的是「模型看过原文之后对某一对内容作出的结论」，重算不出来。丢掉它，
/// 用户就要第二次回答同一个问题——正是本轮要消除的东西。
pub(crate) const REPAIR_RULINGS_FILE: &str = "repair-rulings.json";
pub(crate) const SOURCE_VERIFICATION_FILE: &str = "source-verification.json";
pub(crate) const DECISION_FILE: &str = "decision.json";
pub(crate) const CURRENT_BATCH_FILE: &str = "current.json";

fn recognition_dir(root: &Path, job_id: &str) -> CommandResult<PathBuf> {
    Ok(safe_job_dir(root, job_id)?.join("recognition"))
}

fn artifact_path(root: &Path, job_id: &str, batch_id: &str, name: &str) -> CommandResult<PathBuf> {
    crate::util::validate_path_segment("batch_id", batch_id)?;
    Ok(recognition_dir(root, job_id)?.join(format!("{batch_id}.{name}")))
}

pub(crate) fn write_candidate(
    root: &Path,
    batch_id: &str,
    candidate: &RecognitionCandidateV1,
    file: &str,
) -> CommandResult<PathBuf> {
    let path = artifact_path(root, &candidate.job_id, batch_id, file)?;
    write_json(&path, candidate)?;
    Ok(path)
}

/// 落盘云端「完整候选」artifact。
///
/// 候选只是 artifact：走独立的 `cloud-authoring-candidate.json`，**绝不**碰权威稿
/// （不得调用 `seed_canonical_ds` / `write_canonical_json_atomic`，也不得写
/// `library_items_v2.canonical_ds_json`）。序列化沿用 [`write_candidate`] 的 `write_json`。
pub(crate) fn write_cloud_authoring_candidate(
    root: &Path,
    batch_id: &str,
    candidate: &CloudAuthoringCandidateV1,
) -> CommandResult<PathBuf> {
    let path = artifact_path(root, &candidate.job_id, batch_id, CLOUD_AUTHORING_CANDIDATE_FILE)?;
    write_json(&path, candidate)?;
    Ok(path)
}

/// 落盘一次修复运行的摘要（诊断副本）。
///
/// 与候选一样走独立 artifact，**不碰权威稿**。摘要丢失时前端应回落到「按当前稿重算」，
/// 而不是把丢失当成「修复没发生」。
pub(crate) fn write_repair_summary(
    root: &Path,
    job_id: &str,
    batch_id: &str,
    summary: &Value,
) -> CommandResult<PathBuf> {
    let path = artifact_path(root, job_id, batch_id, REPAIR_SUMMARY_FILE)?;
    write_json(&path, summary)?;
    Ok(path)
}

/// 落盘本批次的差异裁定记录。
pub(crate) fn write_repair_rulings(
    root: &Path,
    job_id: &str,
    batch_id: &str,
    rulings: &Value,
) -> CommandResult<PathBuf> {
    let path = artifact_path(root, job_id, batch_id, REPAIR_RULINGS_FILE)?;
    write_json(&path, rulings)?;
    Ok(path)
}

/// 读取本批次的差异裁定记录。**未写入过返回 `Ok(None)`**，不是错误：
/// 首次运行时本来就没有裁定。
///
/// 读不到**不**降级成空数组：`None`（没裁定过）与 `Some([])`（裁定过、但没有条目）
/// 对调用方是同一件事，所以这里不做区分；但**损坏**必须报错——把一份读不懂的裁定
/// 当成「没有裁定」，会让已经了结的差异重新变成用户任务，而且没有任何迹象。
pub(crate) fn read_repair_rulings(
    root: &Path,
    job_id: &str,
    batch_id: &str,
) -> CommandResult<Option<Value>> {
    let path = artifact_path(root, job_id, batch_id, REPAIR_RULINGS_FILE)?;
    read_json_opt(&path)
}

pub(crate) fn write_source_verification(
    root: &Path,
    job_id: &str,
    batch_id: &str,
    verification: &SourceVerificationV1,
) -> CommandResult<PathBuf> {
    let path = artifact_path(root, job_id, batch_id, SOURCE_VERIFICATION_FILE)?;
    let payload = serde_json::json!({
        "status": verification.status,
        "reasonCode": verification.reason_code,
        "pageCount": verification.page_texts.len(),
        "questionTokens": verification.question_tokens.iter().collect::<Vec<_>>(),
        "findings": verification
            .findings
            .values()
            .map(|finding| serde_json::json!({
                "targetType": finding.target_type.as_str(),
                "targetId": finding.target_id,
                "field": finding.field.as_str(),
                "verdict": format!("{:?}", finding.verdict).to_lowercase(),
                "reasonCode": finding.reason_code,
                "suggestedValue": finding.suggested_value,
            }))
            .collect::<Vec<_>>(),
    });
    write_json(&path, &payload)?;
    Ok(path)
}

pub(crate) fn write_decision(
    root: &Path,
    job_id: &str,
    batch_id: &str,
    decision: &RecognitionDecisionV1,
) -> CommandResult<PathBuf> {
    let path = artifact_path(root, job_id, batch_id, DECISION_FILE)?;
    write_json(&path, decision)?;
    Ok(path)
}

/// 记录「当前批次」：重试复用同一 batch_id，前端读取也只认最新批次。
pub(crate) fn write_current_batch(root: &Path, job_id: &str, batch_id: &str) -> CommandResult<()> {
    let path = recognition_dir(root, job_id)?.join(CURRENT_BATCH_FILE);
    write_json(
        &path,
        &serde_json::json!({
            "batchId": batch_id,
            "updatedAt": chrono::Utc::now().to_rfc3339()
        }),
    )
}

pub(crate) fn read_current_batch(root: &Path, job_id: &str) -> Option<String> {
    let path = recognition_dir(root, job_id).ok()?.join(CURRENT_BATCH_FILE);
    let value = read_json_opt(&path).ok().flatten()?;
    value.get("batchId").and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn read_candidate(
    root: &Path,
    job_id: &str,
    batch_id: &str,
    file: &str,
) -> Option<RecognitionCandidateV1> {
    let path = artifact_path(root, job_id, batch_id, file).ok()?;
    let value = read_json_opt(&path).ok().flatten()?;
    serde_json::from_value(value).ok()
}

/// 读取云端「完整候选」artifact。
///
/// 语义与 [`read_candidate`] 对齐：文件不存在 ⇒ `Ok(None)`（不是错误）；
/// 文件损坏 / 反序列化失败 ⇒ 明确错误码 `cloud_authoring_candidate_corrupt:...`，
/// 不静默吞掉（避免把坏候选当成「没有候选」）。
pub(crate) fn read_cloud_authoring_candidate(
    root: &Path,
    job_id: &str,
    batch_id: &str,
) -> CommandResult<Option<CloudAuthoringCandidateV1>> {
    let path = artifact_path(root, job_id, batch_id, CLOUD_AUTHORING_CANDIDATE_FILE)?;
    let Some(value) = read_json_opt(&path)? else {
        return Ok(None);
    };
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| format!("cloud_authoring_candidate_corrupt:{error}"))
}

pub(crate) fn read_decision_file(
    root: &Path,
    job_id: &str,
    batch_id: &str,
) -> Option<RecognitionDecisionV1> {
    let path = artifact_path(root, job_id, batch_id, DECISION_FILE).ok()?;
    let value = read_json_opt(&path).ok().flatten()?;
    serde_json::from_value(value).ok()
}

// ── 数据库（读取权威）──────────────────────────────────────────────────

pub(crate) struct BatchRow {
    pub batch_id: String,
    pub library_item_id: String,
    pub job_id: String,
    pub base_edit_version: i64,
    pub source_sha256: String,
    pub chain_status: ChainStatusSummaryV1,
    pub summary: DecisionSummaryV1,
    /// 四阶段完整状态。旧行可能为 `None`（迁移前写入），由调用方降级重建。
    pub chain_state: Option<RecognitionChainStateV1>,
    /// 云端自主修复摘要（`repair` 契约）。`None` = 没有修复记录（旧批次，或本次无云
    /// 导入）。**调用方不得把 `None` 当成 completed**——它只表示「不知道」。
    pub repair: Option<Value>,
    pub updated_at: String,
}

/// 当前 canonical 编辑版本（前端「建议是否过期」的判据）。
pub(crate) fn current_edit_version(
    conn: &Connection,
    item_id: &str,
) -> CommandResult<Option<i64>> {
    conn.query_row(
        "SELECT current_edit_version FROM library_items_v2 WHERE id = ?1",
        [item_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(|error| format!("recognition_current_edit_version:{error}"))
}

/// 按批次 id 读取汇总行（人工决策直接以 batchId 定位）。
pub(crate) fn load_batch_by_id(
    conn: &Connection,
    batch_id: &str,
) -> CommandResult<Option<BatchRow>> {
    query_batch_row(
        conn,
        "SELECT batch_id, library_item_id, job_id, base_edit_version, source_sha256,
                local_status, cloud_status, source_status,
                cloud_reason_code, source_reason_code,
                agreed_count, auto_fixed_count, needs_review_count, unverifiable_count,
                stages_json, repair_json, updated_at
         FROM recognition_batches_v1 WHERE batch_id = ?1",
        params![batch_id],
    )
}

pub(crate) fn upsert_batch(conn: &Connection, decision: &RecognitionDecisionV1) -> CommandResult<()> {
    upsert_batch_with_stages(conn, decision, None)
}

/// 写入批次汇总。`chain_state` 为四阶段完整状态（含 queued/running/canceled）。
pub(crate) fn upsert_batch_with_stages(
    conn: &Connection,
    decision: &RecognitionDecisionV1,
    chain_state: Option<&RecognitionChainStateV1>,
) -> CommandResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let chain = &decision.chain_status;
    let stages_json = match chain_state {
        Some(state) => serde_json::to_string(state).map_err(|error| error.to_string())?,
        None => "{}".to_string(),
    };
    conn.execute(
        "INSERT INTO recognition_batches_v1
            (batch_id, library_item_id, job_id, base_edit_version, source_sha256,
             local_status, cloud_status, source_status, cloud_reason_code, source_reason_code,
             stages_json,
             agreed_count, auto_fixed_count, needs_review_count, unverifiable_count,
             created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
         ON CONFLICT(batch_id) DO UPDATE SET
             base_edit_version = excluded.base_edit_version,
             local_status = excluded.local_status,
             cloud_status = excluded.cloud_status,
             source_status = excluded.source_status,
             cloud_reason_code = excluded.cloud_reason_code,
             source_reason_code = excluded.source_reason_code,
             stages_json = excluded.stages_json,
             agreed_count = excluded.agreed_count,
             auto_fixed_count = excluded.auto_fixed_count,
             needs_review_count = excluded.needs_review_count,
             unverifiable_count = excluded.unverifiable_count,
             updated_at = excluded.updated_at",
        params![
            decision.batch_id,
            decision.item_id,
            decision.job_id,
            decision.base_edit_version,
            chain.local.as_str(),
            chain.cloud.as_str(),
            chain.source.as_str(),
            chain.cloud_reason_code,
            chain.source_reason_code,
            stages_json,
            decision.summary.agreed as i64,
            decision.summary.auto_fixed as i64,
            decision.summary.needs_review as i64,
            decision.summary.unverifiable as i64,
            now,
        ],
    )
    .map_err(|error| format!("recognition_upsert_batch:{error}"))?;
    Ok(())
}

pub(crate) fn upsert_batch_source_sha(
    conn: &Connection,
    batch_id: &str,
    source_sha256: &str,
) -> CommandResult<()> {
    conn.execute(
        "UPDATE recognition_batches_v1 SET source_sha256 = ?2, updated_at = ?3 WHERE batch_id = ?1",
        params![batch_id, source_sha256, chrono::Utc::now().to_rfc3339()],
    )
    .map_err(|error| format!("recognition_batch_sha:{error}"))?;
    Ok(())
}

/// 整批替换裁决项：同一批次内的旧项被删除，`decision_id` 唯一约束保证
/// 「同一问题只有一张建议卡」不会在存储层被破坏。
pub(crate) fn replace_decision_items(
    conn: &Connection,
    decision: &RecognitionDecisionV1,
) -> CommandResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "DELETE FROM recognition_decisions_v1 WHERE batch_id = ?1",
        [&decision.batch_id],
    )
    .map_err(|error| format!("recognition_clear_decisions:{error}"))?;
    for item in &decision.items {
        insert_decision_item(conn, &decision.item_id, &decision.batch_id, item, &now)?;
    }
    Ok(())
}

fn insert_decision_item(
    conn: &Connection,
    item_id: &str,
    batch_id: &str,
    item: &DecisionItemV1,
    now: &str,
) -> CommandResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO recognition_decisions_v1
            (batch_id, decision_id, library_item_id, resolution, code, severity,
             target_type, target_id, field, status, item_json, applied_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            batch_id,
            item.decision_id,
            item_id,
            item.resolution.as_str(),
            item.code,
            format!("{:?}", item.severity).to_lowercase(),
            item.target.target_type.as_str(),
            item.target.target_id,
            item.field.as_str(),
            format!("{:?}", item.status).to_lowercase(),
            serde_json::to_string(item).map_err(|error| error.to_string())?,
            item.applied_at,
            now,
        ],
    )
    .map_err(|error| format!("recognition_insert_decision:{error}"))?;
    Ok(())
}

/// 持久化单项状态（接受 / 拒绝 / 过期 / 失败）。
///
/// 列与 JSON 必须一致：读取侧（[`load_decision_items`]）以 `item_json` 为权威，
/// 若只更新列而 JSON 里仍是 `open`，前端会一直看到未处理状态并重复提交。
/// 因此这里统一以传入的 `status` 覆写 JSON 的 `status` 字段后再落库。
pub(crate) fn set_decision_status(
    conn: &Connection,
    batch_id: &str,
    decision_id: &str,
    status: &str,
    item_json: &str,
    applied_at: Option<&str>,
) -> CommandResult<()> {
    let mut item: Value = serde_json::from_str(item_json)
        .map_err(|error| format!("recognition_decision_json_invalid:{error}"))?;
    if let Some(object) = item.as_object_mut() {
        object.insert("status".to_string(), Value::String(status.to_string()));
    }
    let item_json = serde_json::to_string(&item).map_err(|error| error.to_string())?;
    let updated = conn
        .execute(
            "UPDATE recognition_decisions_v1
             SET status = ?3, item_json = ?4, applied_at = ?5, updated_at = ?6
             WHERE batch_id = ?1 AND decision_id = ?2",
            params![
                batch_id,
                decision_id,
                status,
                item_json,
                applied_at,
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .map_err(|error| format!("recognition_set_status:{error}"))?;
    if updated == 0 {
        return Err(format!("RECOGNITION_DECISION_NOT_FOUND:{decision_id}"));
    }
    Ok(())
}

/// 读取条目最新批次的汇总行（数据库为读取权威）。
pub(crate) fn load_latest_batch(
    conn: &Connection,
    item_id: &str,
) -> CommandResult<Option<BatchRow>> {
    query_batch_row(
        conn,
        "SELECT batch_id, library_item_id, job_id, base_edit_version, source_sha256,
                local_status, cloud_status, source_status,
                cloud_reason_code, source_reason_code,
                agreed_count, auto_fixed_count, needs_review_count, unverifiable_count,
                stages_json, repair_json, updated_at
         FROM recognition_batches_v1
         WHERE library_item_id = ?1
         ORDER BY created_at DESC, rowid DESC LIMIT 1",
        params![item_id],
    )
}

#[allow(clippy::type_complexity)]
fn query_batch_row(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
) -> CommandResult<Option<BatchRow>> {
    let row = conn
        .query_row(sql, args, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, i64>(13)?,
                row.get::<_, String>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, String>(16)?,
            ))
        })
        .optional()
        .map_err(|error| format!("recognition_load_batch:{error}"))?;
    let Some((
        batch_id,
        library_item_id,
        job_id,
        base_edit_version,
        source_sha256,
        local_status,
        cloud_status,
        source_status,
        cloud_reason,
        source_reason,
        agreed,
        auto_fixed,
        needs_review,
        unverifiable,
        stages_json,
        repair_json,
        updated_at,
    )) = row
    else {
        return Ok(None);
    };
    Ok(Some(BatchRow {
        batch_id,
        library_item_id,
        job_id,
        base_edit_version,
        source_sha256,
        chain_status: ChainStatusSummaryV1 {
            local: parse_chain_status(&local_status),
            cloud: parse_chain_status(&cloud_status),
            source: parse_chain_status(&source_status),
            cloud_reason_code: cloud_reason,
            source_reason_code: source_reason,
        },
        summary: DecisionSummaryV1 {
            agreed: agreed as u32,
            auto_fixed: auto_fixed as u32,
            needs_review: needs_review as u32,
            unverifiable: unverifiable as u32,
        },
        chain_state: serde_json::from_str::<RecognitionChainStateV1>(&stages_json)
            .ok()
            .filter(|_| stages_json != "{}"),
        // 坏 JSON 不静默成 `None`（那会让「修复记录损坏」看起来像「没做过修复」）；
        // 但也绝不因此让整行读取失败——批次状态本身仍然可用。
        repair: repair_json.and_then(|json| match serde_json::from_str::<Value>(&json) {
            Ok(value) => Some(value),
            Err(_) => Some(serde_json::json!({
                "status": "unavailable",
                "reasonCode": "repair_json_corrupt",
            })),
        }),
        updated_at,
    }))
}

/// 写入批次的云端修复摘要（与批次状态同库提交）。
///
/// 只写这一列，不碰其它批次字段：修复摘要由修复循环单独产出，批次状态由识别周期产出，
/// 两者互不覆盖。`batch_id` 不存在时返回错误——绝不 INSERT 出一个没有识别依据的批次行。
pub(crate) fn write_batch_repair(
    conn: &Connection,
    batch_id: &str,
    repair: &Value,
) -> CommandResult<()> {
    let json = serde_json::to_string(repair)
        .map_err(|error| format!("recognition_repair_json_serialize:{error}"))?;
    let affected = conn
        .execute(
            "UPDATE recognition_batches_v1 SET repair_json = ?2, updated_at = ?3 WHERE batch_id = ?1",
            params![batch_id, json, chrono::Utc::now().to_rfc3339()],
        )
        .map_err(|error| format!("recognition_write_batch_repair:{error}"))?;
    if affected == 0 {
        return Err(format!("recognition_batch_missing:{batch_id}"));
    }
    Ok(())
}

/// 云端**真的跑过、但没交出可用结果**时，把批次行的 cloud 阶段改写成真实终态。
///
/// 为什么必须单独有这一格：批次行是本地周期建出来的，而本地周期按设计以
/// `cloud_enabled = false` 运行，于是它写下的是 `not_run` / `CLOUD_DISABLED`，
/// 消息字面是「本次导入未启用云端识别」。`write_batch_repair` 只写 `repair_json`，
/// 从不碰这一格。于是一次真实的云端超时会在库里留下互相矛盾的两行：
/// 任务行 `cloud_status = failed` 且 `last_error_code` 带着真实超时，批次行却说
/// 用户没启用云端——而前端状态行读的是**批次行**，用户看到的是
/// 「题稿已生成，可以开始编辑」，那次超时在界面上完全消失。
///
/// 2026-09-20 真实网关跑 `demanding-reading-passage-3.pdf` 时实测到这一点：
/// `progress_json` 明写 `cloudEnabled:true`，同一行的 `cloud_reason_code` 却是
/// `CLOUD_DISABLED`。
#[allow(dead_code)]
pub(crate) fn write_batch_cloud_failure(
    conn: &Connection,
    batch_id: &str,
    state: &str,
    reason_code: &str,
    message: &str,
) -> CommandResult<()> {
    write_batch_cloud_stage(conn, batch_id, state, state, Some(reason_code), message)
}

/// 把批次行 cloud 阶段改写成云端**真实终态**（成功 / 部分 / 取消 / 不可用）。
///
/// `chain_status` 写 `cloud_status` 列（`ChainStatusV1`），`stage_state` 写
/// `stages_json.cloud.state`（`StageStateV1`，可表达 `canceled`）。`reason_code` 为
/// `None` 表示「无异常」。
pub(crate) fn write_batch_cloud_stage(
    conn: &Connection,
    batch_id: &str,
    chain_status: &str,
    stage_state: &str,
    reason_code: Option<&str>,
    message: &str,
) -> CommandResult<()> {
    let state = chain_status;
    let now = chrono::Utc::now().to_rfc3339();
    let stages_raw: Option<String> = conn
        .query_row(
            "SELECT stages_json FROM recognition_batches_v1 WHERE batch_id = ?1",
            params![batch_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("recognition_batch_stages_read:{error}"))?
        .flatten();
    // 阶段快照按需修补：只改 cloud 这一格，其余三路保持本地周期写下的真实值。
    let mut stages = stages_raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let mut cloud = json!({
        "state": stage_state,
        "message": message,
        "updatedAt": now,
    });
    if let Some(code) = reason_code {
        cloud["reasonCode"] = json!(code);
    }
    stages["cloud"] = cloud;
    let stages_json = serde_json::to_string(&stages)
        .map_err(|error| format!("recognition_batch_stages_serialize:{error}"))?;
    let affected = conn
        .execute(
            "UPDATE recognition_batches_v1
                SET cloud_status = ?2, cloud_reason_code = ?3, stages_json = ?4, updated_at = ?5
              WHERE batch_id = ?1",
            params![batch_id, state, reason_code, stages_json, now],
        )
        .map_err(|error| format!("recognition_write_batch_cloud_failure:{error}"))?;
    if affected == 0 {
        return Err(format!("recognition_batch_missing:{batch_id}"));
    }
    Ok(())
}

/// 读取条目最新批次的裁决（数据库为读取权威）。
pub(crate) fn load_latest_decision(
    conn: &Connection,
    item_id: &str,
) -> CommandResult<Option<RecognitionDecisionV1>> {
    let Some(batch) = load_latest_batch(conn, item_id)? else {
        return Ok(None);
    };
    let items = load_decision_items(conn, &batch.batch_id)?;
    Ok(Some(RecognitionDecisionV1 {
        schema_version: RECOGNITION_DECISION_V1_SCHEMA_VERSION.to_string(),
        batch_id: batch.batch_id,
        item_id: batch.library_item_id,
        job_id: batch.job_id,
        base_edit_version: batch.base_edit_version,
        generated_at: batch.updated_at,
        chain_status: batch.chain_status,
        items,
        summary: batch.summary,
    }))
}

/// 读取某批次全部裁决项（按 decision_id 稳定排序）。
pub(crate) fn load_decision_items(
    conn: &Connection,
    batch_id: &str,
) -> CommandResult<Vec<DecisionItemV1>> {
    let mut statement = conn
        .prepare(
            "SELECT item_json FROM recognition_decisions_v1 WHERE batch_id = ?1 ORDER BY decision_id",
        )
        .map_err(|error| format!("recognition_load_decisions:{error}"))?;
    let rows = statement
        .query_map([batch_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("recognition_load_decisions:{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("recognition_load_decisions:{error}"))?;
    Ok(rows
        .into_iter()
        .filter_map(|json| serde_json::from_str::<DecisionItemV1>(&json).ok())
        .collect())
}

/// 读取单条裁决项（幂等重放与状态回写都需要它）。
pub(crate) fn load_decision_item(
    conn: &Connection,
    batch_id: &str,
    decision_id: &str,
) -> CommandResult<Option<DecisionItemV1>> {
    conn.query_row(
        "SELECT item_json FROM recognition_decisions_v1 WHERE batch_id = ?1 AND decision_id = ?2",
        params![batch_id, decision_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|error| format!("recognition_load_decision_item:{error}"))
    .map(|row| row.and_then(|json| serde_json::from_str::<DecisionItemV1>(&json).ok()))
}

fn parse_chain_status(raw: &str) -> ChainStatusV1 {
    match raw {
        "succeeded" => ChainStatusV1::Succeeded,
        "partial" => ChainStatusV1::Partial,
        "unusable" => ChainStatusV1::Unusable,
        _ => ChainStatusV1::NotRun,
    }
}

// ── 幂等日志 ───────────────────────────────────────────────────────────

pub(crate) fn journal_lookup(
    conn: &Connection,
    request_id: &str,
) -> CommandResult<Option<(String, String, i64, String, String)>> {
    conn.query_row(
        "SELECT library_item_id, batch_id, base_edit_version, payload_json, result_json
         FROM recognition_decision_journal_v1 WHERE request_id = ?1",
        [request_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )
    .optional()
    .map_err(|error| format!("recognition_journal_lookup:{error}"))
}

pub(crate) fn journal_insert(
    conn: &Connection,
    request_id: &str,
    item_id: &str,
    batch_id: &str,
    base_edit_version: i64,
    payload_json: &str,
    result_json: &str,
) -> CommandResult<()> {
    conn.execute(
        "INSERT INTO recognition_decision_journal_v1
            (request_id, library_item_id, batch_id, base_edit_version, payload_json, result_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            request_id,
            item_id,
            batch_id,
            base_edit_version,
            payload_json,
            result_json,
            chrono::Utc::now().to_rfc3339()
        ],
    )
    .map_err(|error| format!("recognition_journal_insert:{error}"))?;
    Ok(())
}

/// 批次是否已经存在（重试复用 batch_id 的依据）。
pub(crate) fn batch_exists(conn: &Connection, batch_id: &str) -> CommandResult<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM recognition_batches_v1 WHERE batch_id = ?1",
            [batch_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("recognition_batch_exists:{error}"))?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::schema::ensure_v2_schema;
    use crate::schema::recognition_v1::{
        reason, DecisionFieldV1, DecisionResolutionV1, DecisionSeverityV1, DecisionStatusV1,
        DecisionTargetTypeV1, DecisionTargetV1, StageStateV1, StageStatusV1,
    };
    use serde_json::json;

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_v2_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO library_items_v2 (id, modality, title, status, created_at, updated_at)
             VALUES ('item-1', 'reading', 't', 'processing', '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();
        conn
    }

    fn decision(batch_id: &str) -> RecognitionDecisionV1 {
        RecognitionDecisionV1 {
            schema_version: RECOGNITION_DECISION_V1_SCHEMA_VERSION.to_string(),
            batch_id: batch_id.to_string(),
            item_id: "item-1".to_string(),
            job_id: "job-1".to_string(),
            base_edit_version: 3,
            generated_at: "2026-09-15T00:00:00Z".to_string(),
            chain_status: ChainStatusSummaryV1 {
                local: ChainStatusV1::Succeeded,
                cloud: ChainStatusV1::Partial,
                source: ChainStatusV1::Partial,
                cloud_reason_code: Some(reason::SALVAGE_PARTIAL.to_string()),
                source_reason_code: None,
            },
            items: vec![DecisionItemV1 {
                decision_id: "d:slot:slot-14:answer".to_string(),
                resolution: DecisionResolutionV1::NeedsReview,
                code: "ANSWER_CONFLICT".to_string(),
                severity: DecisionSeverityV1::Blocker,
                title: "t".to_string(),
                user_message: "m".to_string(),
                target: DecisionTargetV1 {
                    target_type: DecisionTargetTypeV1::Slot,
                    target_id: "slot-14".to_string(),
                    task_id: Some("task-1".to_string()),
                    node_id: None,
                    question_numbers: vec![14],
                },
                field: DecisionFieldV1::Answer,
                evidence: vec![],
                local_value: Some(json!({"kind":"text","values":["a"]})),
                cloud_value: Some(json!({"kind":"text","values":["b"]})),
                source_value: None,
                proposed_patch: None,
                undo: None,
                auto_applied: false,
                applied_at: None,
                status: DecisionStatusV1::Open,
                reason_code: reason::SUBSTANTIVE_DIVERGENCE.to_string(),
                dependency_group: None,
            }],
            summary: DecisionSummaryV1 {
                agreed: 4,
                auto_fixed: 1,
                needs_review: 1,
                unverifiable: 2,
            },
        }
    }

    #[test]
    fn repair_summary_round_trips_and_never_fakes_completion() {
        let conn = memory();
        let decision = decision("batch-1");
        upsert_batch(&conn, &decision).unwrap();

        // 没写过修复摘要 ⇒ `None`（「没有修复记录」）。前端必须按「不知道」降级，
        // 不得当成 completed —— 否则一次无云导入会被显示成「云端已修好」。
        let row = load_batch_by_id(&conn, "batch-1").unwrap().unwrap();
        assert!(row.repair.is_none(), "没写过摘要时必须是没有记录，而不是 completed");

        let summary = json!({
            "status": "needs_attention",
            "editVersion": 3,
            "appliedCount": 2,
            "remainingTasks": [{"userTaskId": "u1", "blocking": true}],
            "undoAvailable": true
        });
        write_batch_repair(&conn, "batch-1", &summary).unwrap();
        let row = load_batch_by_id(&conn, "batch-1").unwrap().unwrap();
        let repair = row.repair.expect("摘要必须能读回来");
        assert_eq!(repair["status"], "needs_attention");
        assert_eq!(repair["appliedCount"], 2);
        assert_eq!(repair["undoAvailable"], true);

        // 同一行也走 `load_latest_batch`（两条 SELECT 的列必须一致）。
        let latest = load_latest_batch(&conn, "item-1").unwrap().unwrap();
        assert_eq!(latest.repair.unwrap()["status"], "needs_attention");

        // 批次不存在：报错，绝不 INSERT 出一个没有识别依据的批次行。
        let error = write_batch_repair(&conn, "batch-nope", &summary).unwrap_err();
        assert!(error.contains("recognition_batch_missing"), "{error}");

        // 摘要损坏：不能静默成 `None`（那会让「修复记录损坏」看起来像「没做过修复」），
        // 也不能让整行读取失败——批次状态本身仍然可用。
        conn.execute(
            "UPDATE recognition_batches_v1 SET repair_json = '{oops' WHERE batch_id = 'batch-1'",
            [],
        )
        .unwrap();
        let row = load_batch_by_id(&conn, "batch-1").unwrap().unwrap();
        let repair = row.repair.expect("损坏也要给出可解释的降级值");
        assert_eq!(repair["reasonCode"], "repair_json_corrupt");
        assert_eq!(row.batch_id, "batch-1");
    }

    #[test]
    fn decision_round_trips_through_the_database() {
        let conn = memory();
        let decision = decision("batch-1");
        upsert_batch(&conn, &decision).unwrap();
        replace_decision_items(&conn, &decision).unwrap();
        let loaded = load_latest_decision(&conn, "item-1").unwrap().expect("decision must load");
        assert_eq!(loaded.batch_id, "batch-1");
        assert_eq!(loaded.base_edit_version, 3);
        assert_eq!(loaded.summary.agreed, 4);
        assert_eq!(loaded.summary.auto_fixed, 1);
        assert_eq!(loaded.chain_status.cloud, ChainStatusV1::Partial);
        assert_eq!(
            loaded.chain_status.cloud_reason_code.as_deref(),
            Some(reason::SALVAGE_PARTIAL)
        );
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].decision_id, "d:slot:slot-14:answer");
    }

    /// 「同一问题只有一张建议卡」的存储层保证是**按批次**的：同一批次内
    /// `decision_id` 唯一（主键），跨批次保留历史，但读取只认最新批次——
    /// 因此前端永远不会同时看到两张相互冲突的卡。
    #[test]
    fn decision_id_is_unique_within_a_batch_and_latest_batch_wins() {
        let conn = memory();
        let mut decision = decision("batch-1");
        upsert_batch(&conn, &decision).unwrap();
        replace_decision_items(&conn, &decision).unwrap();
        decision.batch_id = "batch-2".to_string();
        upsert_batch(&conn, &decision).unwrap();
        replace_decision_items(&conn, &decision).unwrap();

        for batch in ["batch-1", "batch-2"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM recognition_decisions_v1
                     WHERE batch_id = ?1 AND decision_id = 'd:slot:slot-14:answer'",
                    [batch],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "{batch} 内同一 decision_id 只能有一行");
        }

        // 同一批次重复写入是替换语义：不会产生重复行。
        replace_decision_items(&conn, &decision).unwrap();
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM recognition_decisions_v1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 2, "每个批次各留一行历史，重复写入不膨胀");

        // 读取只认最新批次。
        let loaded = load_latest_decision(&conn, "item-1").unwrap().unwrap();
        assert_eq!(loaded.batch_id, "batch-2");
        assert_eq!(loaded.items.len(), 1);
    }

    #[test]
    fn decision_status_update_is_idempotent_and_reports_missing_rows() {
        let conn = memory();
        let decision = decision("batch-1");
        upsert_batch(&conn, &decision).unwrap();
        replace_decision_items(&conn, &decision).unwrap();
        let item_json = serde_json::to_string(&decision.items[0]).unwrap();
        set_decision_status(&conn, "batch-1", "d:slot:slot-14:answer", "accepted", &item_json, None).unwrap();
        set_decision_status(&conn, "batch-1", "d:slot:slot-14:answer", "accepted", &item_json, None).unwrap();
        let loaded = load_latest_decision(&conn, "item-1").unwrap().unwrap();
        assert_eq!(loaded.items[0].status, DecisionStatusV1::Accepted);
        assert!(set_decision_status(&conn, "batch-1", "missing", "accepted", "{}", None).is_err());
    }

    #[test]
    fn journal_round_trip_supports_idempotent_replays() {
        let conn = memory();
        journal_insert(&conn, "req-1", "item-1", "batch-1", 3, "{}", "{\"ok\":true}").unwrap();
        let loaded = journal_lookup(&conn, "req-1").unwrap().expect("journal entry");
        assert_eq!(loaded.0, "item-1");
        assert_eq!(loaded.1, "batch-1");
        assert_eq!(loaded.2, 3);
        assert_eq!(loaded.4, "{\"ok\":true}");
        assert!(journal_lookup(&conn, "req-2").unwrap().is_none());
        // 同一 request_id 重复写入必须失败（唯一约束）。
        assert!(journal_insert(&conn, "req-1", "item-1", "batch-1", 3, "{}", "{}").is_err());
    }

    // ── 云端完整候选 artifact ─────────────────────────────────────────────

    use crate::schema::cloud_repair_v1::CloudAuthoringCandidateV1;

    fn golden_authoring_path() -> std::path::PathBuf {
        // 与 `schema/mod.rs` 的 `golden_authoring_path()` 取齐：
        // `CARGO_MANIFEST_DIR` 指向 `src-tauri`，golden 稿在仓库根 `fixtures/` 下。
        let manifest = env!("CARGO_MANIFEST_DIR").trim_end_matches(['\\', '/']);
        std::path::Path::new(manifest)
            .parent()
            .expect("src-tauri 必须有父目录")
            .join("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json")
    }

    /// 用真实 golden fixture 构造候选，避免手写精简稿被 schema 拒。
    fn golden_candidate() -> CloudAuthoringCandidateV1 {
        let path = golden_authoring_path();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("读取 golden 稿失败 path={path:?} err={error}"));
        let authoring: Value =
            serde_json::from_str(&text).expect("golden 稿必须是合法 JSON");
        serde_json::from_value(json!({
            "schemaVersion": "CloudAuthoringCandidateV1",
            "batchId": "batch-1",
            "itemId": "job-1",
            "jobId": "job-1",
            "sourceFileId": "early-approaches-pdf",
            "sourceSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "baseEditVersion": 1,
            "generatedAt": "2026-09-17T00:00:00Z",
            "status": "succeeded",
            "authoring": authoring,
            "idMap": {"cloud-q14": "q14"},
            "unresolvedReferences": [],
            "unresolvedRegions": [{
                "sourceFileId": "early-approaches-pdf",
                "pageIndex": 3,
                "reason": "page_image_unavailable",
                "detail": "扫描页图不可用，该页内容未被覆盖"
            }],
            "sourceCoverageNotes": ["DOCX 图表证据不完整"],
            "warnings": []
        }))
        .expect("golden 候选必须可解析")
    }

    fn temp_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "reconcile-store-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    #[test]
    fn cloud_authoring_candidate_round_trips_with_full_rich_authoring() {
        let original = golden_candidate();
        let root = temp_root();
        let path = write_cloud_authoring_candidate(&root, "batch-1", &original).unwrap();
        assert!(path.exists(), "候选 artifact 必须落到磁盘");

        let readback = read_cloud_authoring_candidate(&root, "job-1", "batch-1")
            .unwrap()
            .expect("必须能读回候选");

        // 整份逐字相等：任何字段被丢掉或压平都会在这里露出来。
        assert_eq!(
            serde_json::to_value(&readback).expect("重新序列化读回候选"),
            serde_json::to_value(&original).expect("重新序列化原始候选"),
            "往返必须逐字相等"
        );
        // 再具体点出「嵌套富内容还在」，防止「空壳也算相等」的假通过。
        let authoring = serde_json::to_value(&readback.authoring).expect("重新序列化内嵌稿件");
        assert_eq!(
            authoring
                .pointer("/taskGroups/0/responseGroups/0/prompt/0/children/0/text")
                .and_then(Value::as_str),
            Some("Which TWO factors influenced early organisational design?"),
            "嵌套的子节点与文本必须逐字保留，不能被压平成纯文本"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 候选只是 artifact：写候选不得碰权威稿 `library_items_v2.canonical_ds_json`，
    /// 也不得新增/删除 `library_items_v2` 的任何行。
    #[test]
    fn writing_cloud_authoring_candidate_does_not_touch_the_library_db() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        let conn = crate::library::repository::open_library_connection(&root).unwrap();

        // 既有一条真实行，canonical 内容取自 golden 稿，便于断言「完全不变」。
        let seeded = serde_json::to_string(&golden_candidate().authoring).unwrap();
        conn.execute(
            "INSERT INTO library_items_v2
                (id, modality, title, status, current_edit_version, canonical_ds_json,
                 source_asset_id, created_at, updated_at, deleted_at)
             VALUES ('item-1', 'reading', 't', 'processing', 1, ?1, NULL,
                     '2026-01-01', '2026-01-01', NULL)",
            [seeded.clone()],
        )
        .unwrap();

        let row_count_before: i64 =
            conn.query_row("SELECT COUNT(*) FROM library_items_v2", [], |row| row.get(0))
                .unwrap();
        let canonical_before: String = conn
            .query_row(
                "SELECT canonical_ds_json FROM library_items_v2 WHERE id = 'item-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // 关键一步：写候选。**绝不**打开数据库。
        let written = write_cloud_authoring_candidate(&root, "batch-1", &golden_candidate()).unwrap();
        assert!(written.exists(), "候选必须落到独立 artifact，而非库里");

        let row_count_after: i64 =
            conn.query_row("SELECT COUNT(*) FROM library_items_v2", [], |row| row.get(0))
                .unwrap();
        let canonical_after: String = conn
            .query_row(
                "SELECT canonical_ds_json FROM library_items_v2 WHERE id = 'item-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(row_count_before, 1, "写候选前必须恰好一条行");
        assert_eq!(row_count_after, 1, "写候选不得新增/删除 library_items_v2 行");
        assert_eq!(
            canonical_after, canonical_before,
            "写候选不得改动任何行的 canonical_ds_json"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_cloud_authoring_candidate_returns_none_when_absent() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        let result = read_cloud_authoring_candidate(&root, "job-1", "never-written").unwrap();
        assert!(
            result.is_none(),
            "读未写入的 batch 必须返回 Ok(None)，不是错误"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_cloud_authoring_candidate_reports_corrupt_with_explicit_code() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        // 造一个「能解析、但不符合候选 schema」的文件：绕过写函数（写函数不会产出坏 JSON），
        // 但命中 `from_value` 的反序列化失败分支，从而验证显式错误码映射。
        let dir = root.join("jobs").join("job-1").join("recognition");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("batch-1.cloud-authoring-candidate.json");
        std::fs::write(&path, r#"{"foo":"bar"}"#).unwrap();

        let result = read_cloud_authoring_candidate(&root, "job-1", "batch-1");
        assert!(
            result.is_err(),
            "损坏文件必须返回明确错误，而非 None 或 panic"
        );
        assert!(
            result.unwrap_err().contains("cloud_authoring_candidate_corrupt"),
            "错误码必须指明候选损坏（反序列化失败）"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 本地周期写下的 `CLOUD_DISABLED` 必须能被一次真实云端失败改写掉。
    ///
    /// 修复前 `write_batch_repair` 只写 `repair_json`，这一格永远停在
    /// 「本次导入未启用云端识别」——而前端状态行读的正是这一格。
    #[test]
    fn a_real_cloud_failure_overwrites_the_local_cycle_cloud_disabled_stage() {
        let conn = memory();
        let mut decision = decision("batch-cloud-fail");
        // 先复现本地周期的写入：cloud 如实标 not_run/CLOUD_DISABLED。
        decision.chain_status.cloud = ChainStatusV1::NotRun;
        decision.chain_status.cloud_reason_code = Some(reason::CLOUD_DISABLED.to_string());
        let stages = RecognitionChainStateV1 {
            local: StageStatusV1::new(StageStateV1::Succeeded),
            cloud: StageStatusV1::with_reason(
                StageStateV1::NotRun,
                reason::CLOUD_DISABLED,
                "本次导入未启用云端识别。",
            ),
            source: StageStatusV1::new(StageStateV1::NotRun),
            adjudication: StageStatusV1::new(StageStateV1::Succeeded),
        };
        upsert_batch_with_stages(&conn, &decision, Some(&stages)).unwrap();
        let before = load_batch_by_id(&conn, "batch-cloud-fail").unwrap().unwrap();
        assert_eq!(
            before.chain_status.cloud_reason_code.as_deref(),
            Some(reason::CLOUD_DISABLED)
        );

        write_batch_cloud_failure(
            &conn,
            "batch-cloud-fail",
            "unusable",
            reason::MODEL_TIMEOUT,
            "llm_timeout_budget_exhausted:llm_http_timeout",
        )
        .unwrap();

        let after = load_batch_by_id(&conn, "batch-cloud-fail").unwrap().unwrap();
        assert_eq!(
            after.chain_status.cloud_reason_code.as_deref(),
            Some(reason::MODEL_TIMEOUT)
        );
        let stages_json: String = conn
            .query_row(
                "SELECT stages_json FROM recognition_batches_v1 WHERE batch_id = 'batch-cloud-fail'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let stages: Value = serde_json::from_str(&stages_json).unwrap();
        assert_eq!(stages["cloud"]["state"], json!("unusable"));
        assert_eq!(stages["cloud"]["reasonCode"], json!(reason::MODEL_TIMEOUT));
        assert!(
            !stages_json.contains("未启用云端识别"),
            "云端真的跑过，界面不能再说用户没启用：{stages_json}"
        );
        // 其余三路必须原样保留（只改 cloud 这一格）。
        assert_eq!(stages["local"]["state"], json!("succeeded"));
        assert_eq!(stages["adjudication"]["state"], json!("succeeded"));

        // 不存在的批次必须报错，不能静默成功。
        assert!(write_batch_cloud_failure(
            &conn,
            "batch-missing",
            "unusable",
            reason::MODEL_TIMEOUT,
            "x"
        )
        .is_err());
    }
}
