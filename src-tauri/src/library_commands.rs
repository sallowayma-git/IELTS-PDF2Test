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
/// 尊重软删除：若该 item 已被软删除（deleted_at 非空），跳过本次写入，避免「复活」。
fn upsert_library_item_from_exam(
    root: &Path,
    exam: &ExamRecord,
    schema_version: &str,
) -> CommandResult<()> {
    let conn = open_connection(root)?;
    if is_library_item_soft_deleted(&conn, &exam.id) {
        return Ok(()); // 已软删除，不复活
    }
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
    let conn = open_connection(root)?;
    // 删除入口统一落到 library_items 软删除；若历史数据尚未建 item，则先从旧 exams 回填一份。
    // 旧 exams 行保留，活动查询通过 deleted_at 过滤，这样恢复只需清 deleted_at 即可回到活动列表。
    if !ensure_library_item_for_exam(&conn, id)? {
        return Ok(false);
    }
    let soft_deleted = soft_delete_library_item(&conn, id)?;
    Ok(soft_deleted)
}

pub(crate) fn restore_library_exam_core(root: &Path, id: &str) -> CommandResult<bool> {
    let conn = open_connection(root)?;
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

/// 检查某 library_item 是否已被软删除（供双写钩子判断是否跳过复活）。
fn is_library_item_soft_deleted(conn: &Connection, id: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM library_items WHERE id=?1 AND deleted_at IS NOT NULL",
        rusqlite::params![id],
        |_| Ok(()),
    )
    .optional()
    .map(|o| o.is_some())
    .unwrap_or(false)
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
