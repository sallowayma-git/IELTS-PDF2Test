//! 题库管理命令：CRUD + 统计 + 搜索，以及从 ImportJob/WritingJob 构造 ExamRecord 的双写钩子。
//!
//! 命令薄壳统一写在 lib.rs（与项目既有约定一致），本模块提供 `*_core` 实现供薄壳调用。
//! DB 访问通过 db::open_connection(root) 打开瞬态连接（WAL 模式，桌面工具足够）。

use crate::db::{
    self, ensure_library_item_for_exam, get_exam, get_stats, list_exams, open_connection,
    restore_exam_from_library_item, restore_library_item, search_exams, soft_delete_library_item,
    update_exam_meta, upsert_exam, upsert_exam_conn, upsert_library_item, ExamRecord,
    LibraryItemRecord,
};
use crate::writing_store::{WritingJob, WritingJobStatus};
use crate::{
    CommandResult, ImportJob, JobStatus, LibraryExamDetail, LibraryExamSummary, LibraryFilter,
    LibraryMetaPatch, LibraryStats,
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;

/// Processing-only reads do not load a document or scan unrelated library rows.
pub(crate) fn get_library_item_processing_core(root: &Path, item_id: &str) -> CommandResult<Value> {
    crate::util::validate_path_segment("item_id", item_id)?;
    let conn = crate::library::repository::open_library_connection(root)?;
    serde_json::to_value(crate::processing::queue::get_job_by_library_item(&conn, item_id)?)
        .map_err(|error| error.to_string())
}

/// Refresh a single library row while retaining the existing four-source merge contract.
pub(crate) fn get_library_row_core(root: &Path, item_id: &str) -> CommandResult<Value> {
    let dir = crate::util::safe_job_dir(root, item_id)?;
    let item = crate::library::commands::get_library_item_summary_core(root, item_id)?;
    let conn = crate::library::repository::open_library_connection(root)?;
    let (summary, in_trash) = crate::db::get_library_row_summary(&conn, item_id)?;
    // Initial listLibraryItems excludes deleted rows. Match that merge contract
    // so an event cannot change a trash row's title/Part until the next full refresh.
    let item = if in_trash { Value::Null } else { item };
    let job = crate::util::read_json_opt(&dir.join("job.json"))?;
    if item.is_null() && summary.is_none() && job.is_none() {
        return Ok(Value::Null);
    }
    Ok(serde_json::json!({"job":job,"summary":summary,"item":item,"inTrash":in_trash}))
}

// ── 统一 status 枚举映射 ───────────────────────────────────────────────────
// 阅读 JobStatus 与写作 WritingJobStatus → 统一枚举 draft|needs_review|ready|exported。
// 集中一处，避免散落。

pub(crate) fn status_from_reading(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Working => "draft",
        JobStatus::NeedsReview => "needs_review",
        JobStatus::DraftSaved | JobStatus::ExportReady => "ready",
        JobStatus::Exported | JobStatus::Cleaned => "exported",
    }
}

pub(crate) fn status_from_writing(status: &WritingJobStatus) -> &'static str {
    match status {
        WritingJobStatus::Draft => "draft",
        WritingJobStatus::ExportReady => "ready",
        WritingJobStatus::Exported => "exported",
    }
}

// ── 从 ImportJob 构造 ExamRecord ───────────────────────────────────────────
// payload 用 authoring-ir.json（若存在）的整体；否则用 job.json 本身。
// 这样题库详情页能展示完整的 passage/groups/answerKey。

pub(crate) fn exam_record_from_reading_job(job: &ImportJob, payload: Value) -> ExamRecord {
    let source_hash = job.source_files.first().map(|f| f.sha256.clone());
    ExamRecord {
        id: job.job_id.clone(),
        exam_id: job.category.as_ref().map(|_| job.job_id.clone()), // 阅读暂用 job_id（导出时才生成正式 examId）
        title: job.title.clone(),
        subject: "reading".to_string(),
        category: job.category.clone(),
        frequency: job.frequency.clone(),
        status: status_from_reading(&job.status).to_string(),
        task_type: None,
        tags: job.tags.clone(),
        payload_json: serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()),
        source_hash,
        issue_errors: job.issue_counts.errors,
        issue_warnings: job.issue_counts.warnings,
        created_at: to_iso(&job.created_at),
        updated_at: to_iso(&job.updated_at),
    }
}

/// 读取某阅读 job 的 authoring-ir.json 作为 payload。
/// 仅在文件「不存在」时回退到 job 自身序列化；若文件存在但读取/解析失败，
/// 返回错误（由调用方决定是否保留 DB 旧 payload，避免静默覆盖完整题稿）。
fn reading_payload(root: &Path, job: &ImportJob) -> Result<Value, ReadingPayloadError> {
    let ir_path = crate::util::job_dir(root, &job.job_id).join("authoring-ir.json");
    if !ir_path.exists() {
        return Ok(serde_json::to_value(job).unwrap_or(Value::Null));
    }
    // 文件存在：读取必须成功，否则报错而非静默回退。
    match crate::util::read_json_opt(&ir_path) {
        Ok(Some(ir)) => Ok(ir),
        Ok(None) => Ok(serde_json::to_value(job).unwrap_or(Value::Null)),
        Err(e) => Err(ReadingPayloadError::ReadFailed(e)),
    }
}

#[derive(Debug)]
enum ReadingPayloadError {
    ReadFailed(String),
}

/// 双写钩子入口：阅读 job 保存后调用。失败记日志但不阻断主流程。
/// 当 authoring-ir.json 存在但读取失败时，跳过本次 DB 写入（避免用 job.json
/// 静默覆盖 DB 中已有的完整题稿 payload），仅记日志。
pub(crate) fn upsert_reading_job(root: &Path, job: &ImportJob) -> CommandResult<()> {
    let payload = match reading_payload(root, job) {
        Ok(v) => v,
        Err(ReadingPayloadError::ReadFailed(e)) => {
            eprintln!(
                "[library] upsert_reading_job skipped for {}: authoring-ir.json read failed (DB payload preserved): {}",
                job.job_id, e
            );
            return Ok(());
        }
    };
    let record = exam_record_from_reading_job(job, payload);
    upsert_exam(root, &record)?;
    // Phase 1：同步写正式主模型 library_items + revisions（尊重软删除：已软删除则不复活）。
    if let Err(e) = upsert_library_item_from_exam(root, &record, "ReadingAuthoringIRV1") {
        eprintln!(
            "[library] upsert_library_item (reading) failed for {}: {}",
            job.job_id, e
        );
    }
    Ok(())
}

// ── 从 WritingJob 构造 ExamRecord ──────────────────────────────────────────

pub(crate) fn exam_record_from_writing_job(job: &WritingJob) -> ExamRecord {
    let payload = serde_json::to_value(job).unwrap_or(Value::Null);
    ExamRecord {
        id: job.job_id.clone(),
        exam_id: Some(job.exam_id.clone()),
        title: job.title.clone(),
        subject: "writing".to_string(),
        category: Some(job.task_type.clone()), // 写作的 category 即 task_type
        frequency: None,
        status: status_from_writing(&job.status).to_string(),
        task_type: Some(job.task_type.clone()),
        tags: Vec::new(),
        payload_json: serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()),
        source_hash: None,
        issue_errors: 0,
        issue_warnings: 0,
        created_at: to_iso(&job.created_at),
        updated_at: to_iso(&job.updated_at),
    }
}

/// 双写钩子入口：写作 job 保存后调用。
pub(crate) fn upsert_writing_job(root: &Path, job: &WritingJob) -> CommandResult<()> {
    let record = exam_record_from_writing_job(job);
    upsert_exam(root, &record)?;
    // Phase 1：同步写正式主模型 library_items + revisions。
    if let Err(e) = upsert_library_item_from_exam(root, &record, "WritingExamSourceV1") {
        eprintln!(
            "[library] upsert_library_item (writing) failed for {}: {}",
            job.job_id, e
        );
    }
    Ok(())
}

fn to_iso(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

// ── Phase 1：ExamRecord → LibraryItemRecord 转换 + 正式主模型双写 ───────────

/// 旧 exams 状态 → 新 library_items 状态映射。
/// draft→draft, needs_review→review_required, ready→ready, exported→published。
fn to_library_item_status(exam_status: &str) -> &'static str {
    match exam_status {
        "draft" => "draft",
        "needs_review" => "review_required",
        "ready" => "ready",
        "exported" => "published",
        _ => "draft",
    }
}

/// 把 ExamRecord 转成 LibraryItemRecord 并写入 library_items + revisions。
/// 尊重软删除：识别完成回写时若该条目已被软删（v1 或 v2），新建/更新的 v1 行随后继承删除态，
/// 不让「exams 行在、library_items 无删除标记」把已删条目复活回活动列表。
fn upsert_library_item_from_exam(
    root: &Path,
    exam: &ExamRecord,
    schema_version: &str,
) -> CommandResult<()> {
    // 需读 v2 判删除态，用 library 连接（含 v1+v2）。
    let conn = crate::library::repository::open_library_connection(root)?;
    let soft_deleted = is_library_item_soft_deleted(&conn, &exam.id);
    let content_type = if exam.subject == "writing" {
        "writing_task"
    } else {
        "reading_exam"
    };
    let item = LibraryItemRecord {
        id: exam.id.clone(),
        subject: exam.subject.clone(),
        content_type: content_type.to_string(),
        title: exam.title.clone(),
        category: exam.category.clone(),
        difficulty: exam.frequency.clone(),
        status: to_library_item_status(&exam.status).to_string(),
        task_type: exam.task_type.clone(),
        tags: exam.tags.clone(),
        source_asset_id: exam.source_hash.clone(),
        linked_ingest_job_id: Some(exam.id.clone()),
        created_at: exam.created_at.clone(),
        updated_at: exam.updated_at.clone(),
        revision_payload_json: exam.payload_json.clone(),
        schema_version: schema_version.to_string(),
        created_from_job_id: Some(exam.id.clone()),
        change_reason: Some("ingest_save".to_string()),
    };
    upsert_library_item(&conn, &item)?;
    if soft_deleted {
        // 让刚写入的 v1 行继承删除态（并补 v2），识别完成不复活已删条目。
        soft_delete_library_item(&conn, &exam.id)?;
    }
    Ok(())
}

// ── core 实现（供 lib.rs 命令薄壳调用）─────────────────────────────────────

pub(crate) fn list_library_exams_core(
    root: &Path,
    filter: Option<LibraryFilter>,
) -> CommandResult<Vec<LibraryExamSummary>> {
    let conn = open_connection(root)?;
    list_exams(&conn, &filter.unwrap_or_default())
}

pub(crate) fn get_library_exam_core(
    root: &Path,
    id: &str,
) -> CommandResult<Option<LibraryExamDetail>> {
    let conn = open_connection(root)?;
    get_exam(&conn, id)
}

pub(crate) fn update_library_exam_meta_core(
    root: &Path,
    id: &str,
    patch: LibraryMetaPatch,
) -> CommandResult<Option<LibraryExamSummary>> {
    let conn = open_connection(root)?;
    // 先查 summary 判断 subject，决定回写哪个 JSON 源文件。
    let before = match get_exam(&conn, id)? {
        Some(d) => d.summary,
        None => return Ok(None),
    };
    // 更新 DB 元数据。
    let updated = match update_exam_meta(&conn, id, &patch)? {
        Some(s) => s,
        None => return Ok(None),
    };
    // 回写 JSON 源文件，保证导出链路读到的元数据与题库一致（避免「改了不生效」）。
    // 回写失败记日志但不回滚 DB（DB 是查询主源，文件是导出源；二者短时不一致可被下次双写纠正）。
    if let Err(e) = write_back_meta_to_source(root, &before.subject, id, &patch) {
        eprintln!(
            "[library] write_back_meta_to_source failed for {}: {}",
            id, e
        );
    }
    Ok(Some(updated))
}

pub(crate) fn delete_library_exam_core(root: &Path, id: &str) -> CommandResult<bool> {
    // 用 library 连接（含 v1+v2）：删除态以 library_items_v2 为准，识别中条目只有 v2 外壳、无 v1 行。
    let conn = crate::library::repository::open_library_connection(root)?;
    // 能从旧 exams 回填 v1 就顺带回填；回填不了也不早退——soft_delete 会标记 v2（及存在的 v1），
    // 识别中的 v2-only 条目照样删得掉（旧代码在此 return Ok(false) 造成「识别中删除空操作」）。
    ensure_library_item_for_exam(&conn, id)?;
    soft_delete_library_item(&conn, id)
}

pub(crate) fn restore_library_exam_core(root: &Path, id: &str) -> CommandResult<bool> {
    let conn = crate::library::repository::open_library_connection(root)?;
    let restored = restore_library_item(&conn, id)?;
    if restored {
        let _ = restore_exam_from_library_item(&conn, id)?;
    }
    Ok(restored)
}

pub(crate) fn list_trashed_exams_core(root: &Path) -> CommandResult<Vec<LibraryExamSummary>> {
    let conn = open_connection(root)?;
    crate::db::list_trashed_items(&conn)
}

/// 活动处理阶段：处于这些阶段说明还有 worker 在写这道题，永久删除会与之竞争、留下孤儿写。
fn is_actively_processing(stage: &str) -> bool {
    use crate::processing::queue::{
        STAGE_CLOUD_RECOGNITION, STAGE_LOCAL_RECOGNITION, STAGE_QUEUED, STAGE_RECONCILING,
        STAGE_RUNNING,
    };
    matches!(
        stage,
        STAGE_QUEUED
            | STAGE_RUNNING
            | STAGE_LOCAL_RECOGNITION
            | STAGE_CLOUD_RECOGNITION
            | STAGE_RECONCILING
    )
}

/// 永久删除单个回收站条目。三条约束：
/// - 只删已在回收站的条目（`deleted_at IS NOT NULL`），否则拒绝 `NOT_IN_TRASH`；
/// - 仍在识别/排队中则拒绝 `ITEM_STILL_PROCESSING`：运行中任务只能异步取消，此刻删行会与
///   仍持租约的 worker 竞争、被写回成孤儿；终态或无任务才放行；
/// - 先删数据库（单事务、枚举全表），再删文件——DB 提交后条目即从所有列表消失。
pub(crate) fn permanently_delete_library_exam_core(root: &Path, id: &str) -> CommandResult<bool> {
    // 用 v2 连接：清理与处理状态检查要读 v2 表，旧 `open_connection` 在未建 v2 的路径上会缺表。
    let conn = crate::library::repository::open_library_connection(root)?;
    if !is_library_item_soft_deleted(&conn, id) {
        return Err(format!("NOT_IN_TRASH:{id}"));
    }
    if let Some(job) = crate::processing::queue::get_job(&conn, id)? {
        if is_actively_processing(&job.stage) {
            return Err(format!("ITEM_STILL_PROCESSING:{}", job.stage));
        }
    }
    let removed = crate::db::purge_all_rows_for_item(&conn, id)?;
    eprintln!("[library] permanent delete {id}: purged {removed} db rows");
    // 释放 DB 连接后再删文件（受管音频清理会另开连接）。文件删除失败只记日志、不回滚：DB 已一致。
    drop(conn);
    if let Err(error) = crate::job_commands::delete_job_artifacts(root, id) {
        eprintln!("[library] permanent delete {id}: file cleanup failed (db already purged): {error}");
    }
    // 写作条目的权威内容在 writing-jobs/<id>/，不在 job 目录里；复用 delete_writing_job 的目录清理
    // （非写作条目该目录不存在，无副作用），否则永久删除写作题会残留 writing-job.json。
    if let Err(error) = crate::writing_store::delete_writing_job(root, id) {
        eprintln!("[library] permanent delete {id}: writing dir cleanup failed: {error}");
    }
    Ok(true)
}

/// 清空回收站：逐个永久删除所有回收站条目；仍在处理中的条目跳过并把 `(id, reason)` 收集回报，
/// 不阻断其余条目。返回 `(deleted_count, skipped)`。
pub(crate) fn empty_recycle_bin_core(root: &Path) -> CommandResult<(usize, Vec<(String, String)>)> {
    let trashed = {
        let conn = open_connection(root)?;
        crate::db::list_trashed_items(&conn)?
    };
    let mut deleted = 0usize;
    let mut skipped: Vec<(String, String)> = Vec::new();
    for summary in trashed {
        match permanently_delete_library_exam_core(root, &summary.id) {
            Ok(true) => deleted += 1,
            Ok(false) => {}
            Err(reason) => skipped.push((summary.id.clone(), reason)),
        }
    }
    Ok((deleted, skipped))
}

/// 是否已软删除：v1 或 v2 任一表标记已删都算（识别中条目只有 v2 外壳）。永久删除门禁与
/// 识别完成回写守卫共用，确保任何一张表都不把已删条目当活动。
fn is_library_item_soft_deleted(conn: &Connection, id: &str) -> bool {
    let marked = |sql: &str| {
        conn.query_row(sql, rusqlite::params![id], |_| Ok(()))
            .optional()
            .map(|o: Option<()>| o.is_some())
            .unwrap_or(false)
    };
    marked("SELECT 1 FROM library_items_v2 WHERE id=?1 AND deleted_at IS NOT NULL")
        || marked("SELECT 1 FROM library_items WHERE id=?1 AND deleted_at IS NOT NULL")
}

/// 把题库元数据编辑回写对应的 JSON 源文件（jobs/<id>/job.json 或 writing-jobs/<id>/writing-job.json）。
/// 这样导出链路（读 JSON）与题库（读 DB）保持一致。
fn write_back_meta_to_source(
    root: &Path,
    subject: &str,
    id: &str,
    patch: &LibraryMetaPatch,
) -> CommandResult<()> {
    if subject == "writing" {
        let mut job = crate::writing_store::load_writing_job(root, id)?;
        if let Some(title) = &patch.title {
            job.title = title.clone();
        }
        if let Some(task_type) = &patch.task_type {
            if task_type == "task1" || task_type == "task2" {
                job.task_type = task_type.clone();
            }
        }
        if let Some(status) = &patch.status {
            job.status = library_status_to_writing(status);
        }
        job.updated_at = chrono::Utc::now();
        // save_writing_job 会触发双写 DB（幂等，值一致）。
        crate::writing_store::save_writing_job(root, &job)?;
    } else {
        let mut job = crate::job_store::load_job(root, id)?;
        if let Some(title) = &patch.title {
            job.title = title.clone();
        }
        if let Some(category) = &patch.category {
            job.category = Some(category.clone());
        }
        if let Some(frequency) = &patch.frequency {
            job.frequency = Some(frequency.clone());
        }
        if let Some(tags) = &patch.tags {
            job.tags = tags.clone();
        }
        if let Some(status) = &patch.status {
            // `exported` 是 Library 视图里 `Exported | Cleaned` 的合并态。
            // 如果底层任务已经是 Cleaned，仅编辑元数据时不要把它降级回 Exported。
            if status == "exported" && job.status == crate::JobStatus::Cleaned {
                // Preserve Cleaned semantics.
            } else if let Some(mapped) = library_status_to_reading(status) {
                job.status = mapped;
            }
        }
        // save_job 会触发双写 DB（幂等，值一致），并刷新 updated_at。
        crate::job_store::update_job(root, id, |j| {
            *j = job.clone();
        })?;
    }
    Ok(())
}

fn library_status_to_reading(status: &str) -> Option<crate::JobStatus> {
    use crate::JobStatus;
    match status {
        "draft" => Some(JobStatus::Working),
        "needs_review" => Some(JobStatus::NeedsReview),
        "ready" => Some(JobStatus::DraftSaved),
        "exported" => Some(JobStatus::Exported),
        _ => None,
    }
}

fn library_status_to_writing(status: &str) -> crate::writing_store::WritingJobStatus {
    use crate::writing_store::WritingJobStatus;
    match status {
        "ready" => WritingJobStatus::ExportReady,
        "exported" => WritingJobStatus::Exported,
        _ => WritingJobStatus::Draft,
    }
}

pub(crate) fn search_library_exams_core(
    root: &Path,
    query: &str,
) -> CommandResult<Vec<LibraryExamSummary>> {
    let conn = open_connection(root)?;
    search_exams(&conn, query)
}

pub(crate) fn get_library_stats_core(root: &Path) -> CommandResult<LibraryStats> {
    let conn = open_connection(root)?;
    get_stats(&conn)
}

// ── 一次性数据迁移 ─────────────────────────────────────────────────────────
// 把现有 jobs/*/job.json (+ authoring-ir.json) 与 writing-jobs/*/writing-job.json
// 全量导入 DB。幂等：用 library_meta.migration_done_v1 标记，已迁移则跳过。
//
// 事务边界：整个迁移包在一个事务内，任一记录 upsert 失败则回滚且「不」写
// migration_done_v1，下次启动会重试。失败原因记入日志而非静默 continue。

pub(crate) fn migrate_existing_into_library(root: &Path) -> CommandResult<usize> {
    let mut conn = open_connection(root)?;
    if matches!(db::get_meta(&conn, "migration_done_v1")?, Some(_)) {
        return Ok(0);
    }

    // 收集所有待迁移记录，先全部构造好，再在事务内统一写入。
    // 这样事务内不再有文件 IO，缩短事务持有时间。
    enum MigRecord {
        Reading(ExamRecord),
        Writing(ExamRecord),
    }
    let mut records: Vec<MigRecord> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    // 记录所有「文件存在且成功解析」的 job_id，用于清理 DB 中无对应文件的孤儿行。
    let mut live_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 阅读
    let jobs_root = root.join("jobs");
    if let Ok(entries) = std::fs::read_dir(&jobs_root) {
        for entry in entries.flatten() {
            let job_json = entry.path().join("job.json");
            if !job_json.exists() {
                continue;
            }
            let job: ImportJob = match crate::util::read_json(&job_json) {
                Ok(j) => j,
                Err(e) => {
                    skipped.push(format!(
                        "reading job.json read failed: {}: {}",
                        entry.path().display(),
                        e
                    ));
                    continue;
                }
            };
            let payload = match reading_payload(root, &job) {
                Ok(v) => v,
                Err(ReadingPayloadError::ReadFailed(e)) => {
                    // authoring-ir 损坏：用 job 自身兜底，保证至少元数据入库（与双写钩子
                    // 的「保留 DB 旧 payload」语义不同——迁移期 DB 为空，无旧 payload 可保留）。
                    skipped.push(format!(
                        "reading authoring-ir read failed, fallback to job: {}: {}",
                        job.job_id, e
                    ));
                    serde_json::to_value(&job).unwrap_or(Value::Null)
                }
            };
            live_ids.insert(job.job_id.clone());
            records.push(MigRecord::Reading(exam_record_from_reading_job(
                &job, payload,
            )));
        }
    }

    // 写作
    let wjobs_root = root.join("writing-jobs");
    if let Ok(entries) = std::fs::read_dir(&wjobs_root) {
        for entry in entries.flatten() {
            let job_json = entry.path().join("writing-job.json");
            if !job_json.exists() {
                continue;
            }
            let job: WritingJob = match crate::util::read_json(&job_json) {
                Ok(j) => j,
                Err(e) => {
                    skipped.push(format!(
                        "writing job.json read failed: {}: {}",
                        entry.path().display(),
                        e
                    ));
                    continue;
                }
            };
            live_ids.insert(job.job_id.clone());
            records.push(MigRecord::Writing(exam_record_from_writing_job(&job)));
        }
    }

    for note in &skipped {
        eprintln!("[library] migration skipped: {}", note);
    }

    // 事务内统一写入：任一失败则回滚，不写 migration_done_v1，下次重试。
    let tx = conn
        .transaction()
        .map_err(|e| format!("migrate_begin:{}", e))?;
    let mut count = 0usize;
    for rec in &records {
        let r = match rec {
            MigRecord::Reading(r) => r,
            MigRecord::Writing(r) => r,
        };
        upsert_exam_conn(&tx, r)?;
        count += 1;
    }
    // 清理 DB 中无对应 job.json/writing-job.json 的孤儿行（之前删文件时 DB 删除失败残留）。
    let pruned = db::prune_orphan_exams(&tx, &live_ids)?;
    if pruned > 0 {
        eprintln!("[library] migration pruned {} orphan exam rows", pruned);
    }
    db::set_meta(&tx, "migration_done_v1", "1")?;
    tx.commit().map_err(|e| format!("migrate_commit:{}", e))?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImportJob, IssueCounts, JobStatus, LibraryMetaPatch, WorkflowStep};
    use chrono::Utc;
    use std::fs;
    use std::path::PathBuf;

    /// 构造一个临时 appData 根目录，内含一个阅读 job（job.json + authoring-ir.json）。
    fn make_reading_appdata() -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("ielts-lib-test-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(crate::util::job_dir(&root, "import-test-1")).unwrap();
        let now = Utc::now();
        let job = ImportJob {
            job_id: "import-test-1".to_string(),
            title: "Original Title".to_string(),
            status: JobStatus::DraftSaved,
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: vec!["old".to_string()],
            source_files: vec![],
            active_llm_profile_id: None,
            created_at: now,
            updated_at: now,
            current_step: WorkflowStep::Authoring,
            issue_counts: IssueCounts::default(),
        };
        crate::util::write_json(
            &crate::util::job_dir(&root, "import-test-1").join("job.json"),
            &job,
        )
        .unwrap();
        // authoring-ir.json：最小可用结构，含 exam.title。
        let ir = serde_json::json!({
            "schemaVersion": "ReadingAuthoringIRV1",
            "jobId": "import-test-1",
            "exam": { "examId": "import-test-1", "title": "Original Title", "category": "P1", "frequency": "medium", "tags": ["old"] },
            "passage": { "title": "P", "htmlBlocks": [], "sourceBlockIds": [] },
            "groups": [],
            "answerKey": {},
            "questionOrder": [],
            "questionDisplayMap": {},
            "audit": { "llmUsed": false, "humanVerified": false, "issues": [], "revision": 0, "updatedAt": now.to_rfc3339() }
        });
        crate::util::write_json(
            &crate::util::job_dir(&root, "import-test-1").join("authoring-ir.json"),
            &ir,
        )
        .unwrap();
        root
    }

    fn make_writing_appdata() -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("ielts-lib-wtest-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(crate::util::writing_job_dir(&root, "writing-test-1")).unwrap();
        let now = Utc::now();
        let job = WritingJob {
            job_id: "writing-test-1".to_string(),
            title: "Original Writing".to_string(),
            task_type: "task1".to_string(),
            exam_id: "wt-test-1".to_string(),
            prompt_text: "prompt".to_string(),
            suggested_word_count: 150,
            status: WritingJobStatus::Draft,
            created_at: now,
            updated_at: now,
        };
        crate::util::write_json(
            &crate::util::writing_job_dir(&root, "writing-test-1").join("writing-job.json"),
            &job,
        )
        .unwrap();
        root
    }

    fn cleanup(root: &PathBuf) {
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn incremental_library_row_preserves_reading_writing_and_trash_without_payload_reads() {
        let root = make_reading_appdata();
        migrate_existing_into_library(&root).unwrap();
        // Summary refresh must not parse unchanged document artifacts or revision payloads.
        fs::write(crate::util::job_dir(&root, "import-test-1").join("authoring-ir.json"), "invalid JSON").unwrap();
        let row = get_library_row_core(&root, "import-test-1").unwrap();
        let conn = crate::library::repository::open_library_connection(&root).unwrap();
        assert_eq!(row["summary"], serde_json::to_value(list_exams(&conn, &LibraryFilter::default()).unwrap().remove(0)).unwrap());
        assert_eq!(row["job"]["jobId"], "import-test-1");
        assert_eq!(row["inTrash"], false);
        assert!(get_library_row_core(&root, "missing").unwrap().is_null());
        assert!(get_library_item_processing_core(&root, "missing").unwrap().is_null());
        cleanup(&root);

        let root = make_writing_appdata();
        migrate_existing_into_library(&root).unwrap();
        let row = get_library_row_core(&root, "writing-test-1").unwrap();
        assert!(row["job"].is_null());
        assert_eq!(row["summary"]["subject"], "writing");
        let conn = crate::library::repository::open_library_connection(&root).unwrap();
        assert!(delete_library_exam_core(&root, "writing-test-1").unwrap());
        let row = get_library_row_core(&root, "writing-test-1").unwrap();
        assert_eq!(row["inTrash"], true);
        assert_eq!(row["summary"], serde_json::to_value(crate::db::list_trashed_items(&conn).unwrap().remove(0)).unwrap());
        cleanup(&root);
    }

    #[test]
    fn incremental_library_row_and_processing_follow_item_identity_and_terminal_state() {
        let root = make_reading_appdata();
        let conn = crate::library::repository::open_library_connection(&root).unwrap();
        crate::library::repository::upsert_item_shell(&conn, &crate::library::repository::UpsertItemInput {
            id: "import-test-1", modality: "listening", title: "V2 Title", status: "processing", source_asset_id: None,
        }).unwrap();
        // Processing job IDs need not equal library item IDs.
        crate::processing::queue::enqueue(&conn, "worker-job", "import-test-1", "source", &Value::Null).unwrap();
        assert_eq!(get_library_item_processing_core(&root, "import-test-1").unwrap()["stage"], "queued");
        conn.execute("UPDATE processing_jobs_v2 SET stage='cancelled', event_seq=5 WHERE id='worker-job'", []).unwrap();
        let row = get_library_row_core(&root, "import-test-1").unwrap();
        assert_eq!(row["item"]["title"], "V2 Title");
        assert_eq!(row["item"]["modality"], "listening");
        assert_eq!(row["item"]["processing"]["stage"], "cancelled");
        assert_eq!(row["item"]["processing"]["eventSeq"], 5);
        crate::db::soft_delete_library_item(&conn, "import-test-1").unwrap();
        let row = get_library_row_core(&root, "import-test-1").unwrap();
        assert_eq!(row["inTrash"], true);
        assert_eq!(row["summary"]["title"], "V2 Title");
        assert!(row["item"].is_null(), "trash refresh must match the full list's deleted-V2 exclusion");
        cleanup(&root);
    }

    #[test]
    fn migrate_imports_reading_and_writing() {
        let root = make_reading_appdata();
        // 再加一个写作 job 到同一 root。
        {
            let wroot = make_writing_appdata();
            let wsrc = crate::util::writing_job_dir(&wroot, "writing-test-1");
            let wdst = crate::util::writing_job_dir(&root, "writing-test-1");
            fs::create_dir_all(&wdst).unwrap();
            fs::copy(wsrc.join("writing-job.json"), wdst.join("writing-job.json")).unwrap();
            let _ = fs::remove_dir_all(wroot);
        }
        let n = migrate_existing_into_library(&root).unwrap();
        assert_eq!(n, 2);
        // 二次迁移幂等：标记已存在，返回 0。
        let n2 = migrate_existing_into_library(&root).unwrap();
        assert_eq!(n2, 0);
        // 题库能查到 2 条。
        let conn = crate::db::open_connection(&root).unwrap();
        let list = crate::db::list_exams(&conn, &crate::LibraryFilter::default()).unwrap();
        assert_eq!(list.len(), 2);
        cleanup(&root);
    }

    #[test]
    fn update_meta_writes_back_to_job_json_and_survives_resave() {
        let root = make_reading_appdata();
        migrate_existing_into_library(&root).unwrap();

        // 题库改 title。
        let updated = update_library_exam_meta_core(
            &root,
            "import-test-1",
            LibraryMetaPatch {
                title: Some("New Title".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(updated.title, "New Title");

        // job.json 应已同步回写新 title（这是审查者 P1 的核心验证点）。
        let job: ImportJob =
            crate::util::read_json(&crate::util::job_dir(&root, "import-test-1").join("job.json"))
                .unwrap();
        assert_eq!(
            job.title, "New Title",
            "job.json title must be synced from library edit"
        );

        // 模拟「后续任务保存」：直接调 save_job（不经题库），双写应保留新 title 而非覆盖回旧值。
        crate::job_store::save_job(&root, &job).unwrap();
        let conn = crate::db::open_connection(&root).unwrap();
        let detail = crate::db::get_exam(&conn, "import-test-1")
            .unwrap()
            .unwrap();
        assert_eq!(
            detail.summary.title, "New Title",
            "DB title must survive a resave (no silent overwrite)"
        );
        cleanup(&root);
    }

    #[test]
    fn delete_library_exam_moves_to_trash_and_restore_brings_it_back() {
        let root = make_reading_appdata();
        migrate_existing_into_library(&root).unwrap();

        // 题库删除。
        let removed = delete_library_exam_core(&root, "import-test-1").unwrap();
        assert!(removed);

        // 源 job 目录保留，后续任务仍可继续保存；真正隐藏依赖软删除标记。
        assert!(
            crate::util::job_dir(&root, "import-test-1").exists(),
            "source job dir should stay on disk"
        );

        // 活动查询应隐藏该行，回收站可见。
        let conn = crate::db::open_connection(&root).unwrap();
        assert!(crate::db::get_exam(&conn, "import-test-1")
            .unwrap()
            .is_none());
        assert!(
            list_library_exams_core(&root, None).unwrap().is_empty(),
            "soft-deleted item must leave active list"
        );
        let trash = list_trashed_exams_core(&root).unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].id, "import-test-1");
        assert_eq!(
            trash[0].status, "ready",
            "trash status must be normalized to public enum"
        );

        // 模拟「后续保存」：双写应尊重软删除态，不应把条目偷偷复活回活动列表。
        let job: ImportJob =
            crate::util::read_json(&crate::util::job_dir(&root, "import-test-1").join("job.json"))
                .unwrap();
        crate::job_store::save_job(&root, &job).unwrap();
        assert!(
            list_library_exams_core(&root, None).unwrap().is_empty(),
            "resave must not revive a trashed item"
        );

        // 恢复后回到活动列表。
        assert!(restore_library_exam_core(&root, "import-test-1").unwrap());
        let active = list_library_exams_core(&root, None).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "import-test-1");
        assert!(list_trashed_exams_core(&root).unwrap().is_empty());
        cleanup(&root);
    }

    #[test]
    fn permanently_delete_purges_all_tables_and_files() {
        use crate::library::repository::{
            get_item, open_library_connection, seed_canonical_ds, upsert_item_shell,
            ApplyEditorCommandsInput, UpsertItemInput,
        };
        let root = make_reading_appdata();
        migrate_existing_into_library(&root).unwrap(); // 旧 library_items 行

        // 建 v2 权威稿 + 一次编辑（产生 editor_journal_v1 行），证明 v2 表也被清干净。
        {
            let conn = open_library_connection(&root).unwrap();
            upsert_item_shell(
                &conn,
                &UpsertItemInput {
                    id: "import-test-1",
                    modality: "reading",
                    title: "Trash me",
                    status: "ready",
                    source_asset_id: None,
                },
            )
            .unwrap();
            let ds = fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"),
            )
            .unwrap();
            let ds: serde_json::Value = serde_json::from_slice(&ds).unwrap();
            seed_canonical_ds(&conn, "import-test-1", &ds.to_string(), "ready").unwrap();
        }
        // writing-jobs/<id> 目录模拟写作条目的权威内容，断言永久删除会一并删掉。
        let writing_dir = crate::util::writing_job_dir(&root, "import-test-1");
        fs::create_dir_all(&writing_dir).unwrap();
        fs::write(writing_dir.join("writing-job.json"), b"{}").unwrap();
        crate::library::commands::apply_editor_commands_core(
            &root,
            ApplyEditorCommandsInput {
                item_id: "import-test-1".to_string(),
                base_version: 1,
                request_id: Some("perma-test".to_string()),
                commands: vec![],
                title: Some("Trash me edited".to_string()),
            },
        )
        .unwrap();

        assert!(delete_library_exam_core(&root, "import-test-1").unwrap());
        assert_eq!(list_trashed_exams_core(&root).unwrap().len(), 1);

        assert!(permanently_delete_library_exam_core(&root, "import-test-1").unwrap());

        let conn = crate::db::open_connection(&root).unwrap();
        // 再枚举清一次应删 0 行 —— 所有表都不再残留该 id。
        assert_eq!(
            crate::db::purge_all_rows_for_item(&conn, "import-test-1").unwrap(),
            0,
            "永久删除后不应还有任何表残留该 id"
        );
        assert!(get_item(&conn, "import-test-1").unwrap().is_none());
        assert!(crate::db::get_exam(&conn, "import-test-1").unwrap().is_none());
        assert!(
            !crate::util::job_dir(&root, "import-test-1").exists(),
            "永久删除必须删掉 job 目录"
        );
        assert!(
            !writing_dir.exists(),
            "永久删除必须删掉 writing-jobs 目录（写作条目权威内容不留孤儿）"
        );
        assert!(!restore_library_exam_core(&root, "import-test-1").unwrap());
        assert!(list_trashed_exams_core(&root).unwrap().is_empty());
        cleanup(&root);
    }

    #[test]
    fn permanently_delete_refuses_item_not_in_trash() {
        let root = make_reading_appdata();
        migrate_existing_into_library(&root).unwrap();
        let err = permanently_delete_library_exam_core(&root, "import-test-1").unwrap_err();
        assert!(
            err.starts_with("NOT_IN_TRASH"),
            "应拒绝未在回收站的条目，实得：{err}"
        );
        assert_eq!(list_library_exams_core(&root, None).unwrap().len(), 1);
        cleanup(&root);
    }

    #[test]
    fn permanently_delete_refuses_item_still_processing() {
        use crate::library::repository::{open_library_connection, upsert_item_shell, UpsertItemInput};
        let root = make_reading_appdata();
        migrate_existing_into_library(&root).unwrap();
        // processing_jobs_v2.library_item_id 外键指向 library_items_v2；导入流程里 job_id
        // 与库条目 id 同值。先建 v2 壳行再挂一个 queued 任务，模拟“已进回收站但仍在识别/排队”。
        {
            let conn = open_library_connection(&root).unwrap();
            upsert_item_shell(
                &conn,
                &UpsertItemInput {
                    id: "import-test-1",
                    modality: "reading",
                    title: "Trash me",
                    status: "processing",
                    source_asset_id: None,
                },
            )
            .unwrap();
            crate::processing::queue::enqueue(
                &conn,
                "import-test-1",
                "import-test-1",
                "asset-1",
                &serde_json::Value::Null,
            )
            .unwrap();
        }
        assert!(delete_library_exam_core(&root, "import-test-1").unwrap());
        let err = permanently_delete_library_exam_core(&root, "import-test-1").unwrap_err();
        assert!(
            err.starts_with("ITEM_STILL_PROCESSING"),
            "识别/排队中的条目应拒绝永久删除，实得：{err}"
        );
        // 拒绝后仍留在回收站，未被误删成孤儿。
        assert_eq!(list_trashed_exams_core(&root).unwrap().len(), 1);
        cleanup(&root);
    }

    #[test]
    fn empty_recycle_bin_purges_every_trashed_item() {
        let root = make_reading_appdata();
        migrate_existing_into_library(&root).unwrap();
        assert!(delete_library_exam_core(&root, "import-test-1").unwrap());
        assert_eq!(list_trashed_exams_core(&root).unwrap().len(), 1);

        let (deleted, skipped) = empty_recycle_bin_core(&root).unwrap();
        assert_eq!(deleted, 1);
        assert!(skipped.is_empty(), "不应有跳过项：{skipped:?}");
        assert!(list_trashed_exams_core(&root).unwrap().is_empty());
        assert!(!crate::util::job_dir(&root, "import-test-1").exists());
        cleanup(&root);
    }

    // 识别进行中删除：此刻条目只有 v2 外壳（无 v1 library_items/exams）。删除必须生效，
    // 且识别完成回写 v1 后不得复活——v1、v2 两个活动查询都不含它，回收站恰好一条。
    #[test]
    fn delete_during_recognition_hides_item_and_survives_recognition_completion() {
        use crate::library::repository::{open_library_connection, upsert_item_shell, UpsertItemInput};
        let root = make_reading_appdata(); // 仅磁盘 job 文件，DB 为空（未 migrate）
        {
            let conn = open_library_connection(&root).unwrap();
            upsert_item_shell(
                &conn,
                &UpsertItemInput {
                    id: "import-test-1",
                    modality: "reading",
                    title: "Recognizing",
                    status: "processing",
                    source_asset_id: None,
                },
            )
            .unwrap();
            crate::processing::queue::enqueue(
                &conn,
                "import-test-1",
                "import-test-1",
                "asset-1",
                &serde_json::Value::Null,
            )
            .unwrap();
        }

        let v2_ids = |root: &std::path::Path| -> Vec<String> {
            crate::library::commands::list_library_items_core(root, false)
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| v.get("id").and_then(|x| x.as_str()).map(String::from))
                .collect()
        };

        assert!(
            delete_library_exam_core(&root, "import-test-1").unwrap(),
            "识别中删除应生效（v2 外壳条目），实得 false"
        );
        assert!(
            !v2_ids(&root).contains(&"import-test-1".to_string()),
            "删除后 v2 活动列表不应含该条：{:?}",
            v2_ids(&root)
        );
        assert!(
            !list_library_exams_core(&root, None)
                .unwrap()
                .iter()
                .any(|s| s.id == "import-test-1"),
            "删除后 v1 活动列表不应含该条"
        );
        assert_eq!(
            list_trashed_exams_core(&root).unwrap().len(),
            1,
            "删除后回收站应恰好一条"
        );

        // 模拟识别完成：worker 经双写钩子把 exam 写入 v1（save_job → upsert_reading_job）。
        let job: ImportJob = crate::util::read_json(
            &crate::util::job_dir(&root, "import-test-1").join("job.json"),
        )
        .unwrap();
        crate::job_store::save_job(&root, &job).unwrap();

        assert!(
            !v2_ids(&root).contains(&"import-test-1".to_string()),
            "识别完成不得把条目复活回 v2 活动列表：{:?}",
            v2_ids(&root)
        );
        assert!(
            !list_library_exams_core(&root, None)
                .unwrap()
                .iter()
                .any(|s| s.id == "import-test-1"),
            "识别完成不得把条目复活回 v1 活动列表"
        );
        assert_eq!(
            list_trashed_exams_core(&root).unwrap().len(),
            1,
            "识别完成后回收站应仍恰好一条（不重复）"
        );
        cleanup(&root);
    }

    #[test]
    fn part_label_backfills_from_ds_and_manual_overrides() {
        use crate::library::repository::{
            open_library_connection, seed_canonical_ds, upsert_item_shell, UpsertItemInput,
        };
        let root = make_reading_appdata();
        {
            let conn = open_library_connection(&root).unwrap();
            upsert_item_shell(
                &conn,
                &UpsertItemInput {
                    id: "import-test-1",
                    modality: "reading",
                    title: "no filename hint",
                    status: "ready",
                    source_asset_id: None,
                },
            )
            .unwrap();
            // synthetic 阅读稿 answerKey = q14/q15 → 题号范围落在 Passage 2。
            let ds = fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"),
            )
            .unwrap();
            let ds: serde_json::Value = serde_json::from_slice(&ds).unwrap();
            seed_canonical_ds(&conn, "import-test-1", &ds.to_string(), "ready").unwrap();
        }

        let find = |value: &serde_json::Value| -> serde_json::Value {
            value
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == "import-test-1")
                .unwrap()
                .clone()
        };

        let listed = crate::library::commands::list_library_items_core(&root, false).unwrap();
        let row = find(&listed);
        assert_eq!(row["partLabel"], "P2", "题号 14–15 应判为 Passage 2");
        let source = row["partSource"].as_str().unwrap();
        assert!(
            source == "range" || source == "content",
            "自动判定来源应为 range/content，实得 {source}"
        );

        assert!(
            crate::library::commands::set_library_item_part_core(&root, "import-test-1", Some("P1"))
                .unwrap()
        );
        let row = find(&crate::library::commands::list_library_items_core(&root, false).unwrap());
        assert_eq!(row["partLabel"], "P1");
        assert_eq!(row["partSource"], "manual");

        // 清除手动值应回到自动判定，而不是停留在手动值。
        assert!(
            crate::library::commands::set_library_item_part_core(&root, "import-test-1", None)
                .unwrap()
        );
        let row = find(&crate::library::commands::list_library_items_core(&root, false).unwrap());
        assert_eq!(row["partLabel"], "P2", "清除手动值后应回到自动判定");
        cleanup(&root);
    }

    #[test]
    fn restore_rehydrates_legacy_exam_row_deleted_by_old_flow() {
        let root = make_reading_appdata();
        migrate_existing_into_library(&root).unwrap();
        assert!(delete_library_exam_core(&root, "import-test-1").unwrap());

        // 模拟历史旧逻辑：软删 item 后又把旧 exams 行物理删掉。
        {
            let conn = crate::db::open_connection(&root).unwrap();
            assert!(crate::db::delete_exam(&conn, "import-test-1").unwrap());
        }
        assert_eq!(list_trashed_exams_core(&root).unwrap().len(), 1);

        // 恢复时应自动从 library_items/current revision 回填 exams，重新出现在活动列表。
        assert!(restore_library_exam_core(&root, "import-test-1").unwrap());
        let conn = crate::db::open_connection(&root).unwrap();
        let detail = crate::db::get_exam(&conn, "import-test-1")
            .unwrap()
            .unwrap();
        assert_eq!(detail.summary.id, "import-test-1");
        assert_eq!(detail.summary.status, "ready");
        cleanup(&root);
    }

    #[test]
    fn update_meta_keeps_cleaned_reading_jobs_cleaned() {
        let root = make_reading_appdata();
        {
            let mut job: ImportJob = crate::util::read_json(
                &crate::util::job_dir(&root, "import-test-1").join("job.json"),
            )
            .unwrap();
            job.status = JobStatus::Cleaned;
            crate::util::write_json(
                &crate::util::job_dir(&root, "import-test-1").join("job.json"),
                &job,
            )
            .unwrap();
        }
        migrate_existing_into_library(&root).unwrap();

        let updated = update_library_exam_meta_core(
            &root,
            "import-test-1",
            LibraryMetaPatch {
                title: Some("Cleaned Title".into()),
                status: Some("exported".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(updated.status, "exported");

        let job: ImportJob =
            crate::util::read_json(&crate::util::job_dir(&root, "import-test-1").join("job.json"))
                .unwrap();
        assert_eq!(
            job.status,
            JobStatus::Cleaned,
            "editing exported metadata must not downgrade a cleaned reading job"
        );
        cleanup(&root);
    }

    #[test]
    fn migrate_prunes_orphan_db_rows() {
        let root = make_reading_appdata();
        // 先迁移，建立正常行。
        migrate_existing_into_library(&root).unwrap();
        // 手动塞一条 DB 孤儿行（无对应 job 文件）。
        let orphan = ExamRecord {
            id: "import-orphan-ghost".to_string(),
            exam_id: None,
            title: "Ghost".to_string(),
            subject: "reading".to_string(),
            category: Some("P1".to_string()),
            frequency: None,
            status: "draft".to_string(),
            task_type: None,
            tags: vec![],
            payload_json: "{}".to_string(),
            source_hash: None,
            issue_errors: 0,
            issue_warnings: 0,
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-01T00:00:00+00:00".to_string(),
        };
        {
            let conn = crate::db::open_connection(&root).unwrap();
            crate::db::upsert_exam_conn(&conn, &orphan).unwrap();
        }
        // 重置迁移标记，再次迁移应 prune 孤儿（因为孤儿无对应 job.json）。
        {
            let conn = crate::db::open_connection(&root).unwrap();
            conn.execute("DELETE FROM library_meta WHERE key='migration_done_v1'", [])
                .unwrap();
        }
        migrate_existing_into_library(&root).unwrap();
        let conn = crate::db::open_connection(&root).unwrap();
        assert!(
            crate::db::get_exam(&conn, "import-orphan-ghost")
                .unwrap()
                .is_none(),
            "orphan must be pruned"
        );
        assert!(
            crate::db::get_exam(&conn, "import-test-1")
                .unwrap()
                .is_some(),
            "live row must remain"
        );
        cleanup(&root);
    }
}
