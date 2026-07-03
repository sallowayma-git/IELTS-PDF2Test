//! 题库 SQLite 存储层（authoring_hub.db）。
//!
//! 设计要点：
//! - 复用已有 `rusqlite 0.31 (bundled)` 依赖，不引入 sqlx/sea-orm/tauri-plugin-sql。
//! - 采用「每次操作打开瞬态连接」策略（与 `export_nas_library.rs` 一致），配合 WAL 模式
//!   保证读写并发安全；这样 `save_job`/`save_writing_job` 等「双写钩子」只需 `root: &Path`
//!   即可访问 DB，无需把 Connection 塞进 AppState、也无需改动现有命令签名。
//! - Phase 1（蓝图 v1）：库从 `library.db` 重命名为 `authoring_hub.db`，消除与 NAS 发布产物
//!   `publish/library.db` 的概念冲突；新增 6 张正式表（library_items/library_item_revisions/
//!   ingest_jobs/source_assets/publish_records/job_artifact_index），旧 `exams` 表保留兼容。
//! - `library_meta` 存 schema 版本与迁移标记。

use crate::{
    LibraryExamDetail, LibraryExamSummary, LibraryFilter, LibraryMetaPatch, LibraryStats,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::CommandResult;

/// 旧库文件名（兼容迁移用）。
const LEGACY_DB_NAME: &str = "library.db";
/// 新库文件名：authoring_hub.db。
const DB_NAME: &str = "authoring_hub.db";

/// 题库 DB 文件路径：<appData>/authoring_hub.db
pub(crate) fn db_path(root: &Path) -> PathBuf {
    root.join(DB_NAME)
}

/// 启动时迁移：若旧 `library.db` 存在且新 `authoring_hub.db` 不存在，则重命名（保留数据）。
/// 幂等：新库已存在时跳过；两者都存在时保留新库（以新库为准）。
pub(crate) fn migrate_db_file(root: &Path) -> CommandResult<()> {
    let legacy = root.join(LEGACY_DB_NAME);
    let current = root.join(DB_NAME);
    if current.exists() {
        return Ok(());
    }
    if legacy.exists() {
        std::fs::rename(&legacy, &current).map_err(|e| format!("migrate_db_rename:{}:{}", legacy.display(), e))?;
        eprintln!("[library] migrated legacy DB {} -> {}", LEGACY_DB_NAME, DB_NAME);
    }
    Ok(())
}

/// 打开一个连接：设置 WAL/外键/忙等超时，并确保 schema 存在（幂等）。
pub(crate) fn open_connection(root: &Path) -> CommandResult<Connection> {
    // 先做文件级迁移（旧 library.db → authoring_hub.db）。
    migrate_db_file(root)?;
    let path = db_path(root);
    let conn = Connection::open(&path).map_err(|error| {
        // 不回传绝对路径（含用户名/目录结构），仅记 stderr，面向前端返回脱敏码。
        eprintln!("[library] open_db failed at {}: {}", path.display(), error);
        "open_db_failed".to_string()
    })?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;\n\
         PRAGMA foreign_keys=ON;\n\
         PRAGMA busy_timeout=5000;",
    )
    .map_err(|error| format!("db_pragma:{}", error))?;
    ensure_schema(&conn)?;
    Ok(conn)
}

/// 建表 + 索引（IF NOT EXISTS，幂等）。
pub(crate) fn ensure_schema(conn: &Connection) -> CommandResult<()> {
    conn.execute_batch(SCHEMA_SQL)
        .map_err(|error| format!("db_schema:{}", error))?;
    Ok(())
}

const SCHEMA_SQL: &str = r#"
-- ── 兼容层：旧 exams 表（Phase 1 保留，逐步迁移到 library_items）──
CREATE TABLE IF NOT EXISTS library_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS exams (
    id             TEXT PRIMARY KEY,
    exam_id        TEXT,
    title          TEXT NOT NULL,
    subject        TEXT NOT NULL CHECK (subject IN ('reading','writing')),
    category       TEXT,
    frequency      TEXT,
    status         TEXT NOT NULL CHECK (status IN ('draft','needs_review','ready','exported')),
    task_type      TEXT,
    tags_json      TEXT NOT NULL DEFAULT '[]',
    payload_json   TEXT NOT NULL,
    source_hash    TEXT,
    issue_errors   INTEGER NOT NULL DEFAULT 0,
    issue_warnings INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_exams_status   ON exams(status);
CREATE INDEX IF NOT EXISTS idx_exams_subject  ON exams(subject);
CREATE INDEX IF NOT EXISTS idx_exams_category ON exams(category);
CREATE INDEX IF NOT EXISTS idx_exams_updated  ON exams(updated_at);

-- ── Phase 1 正式 schema：library-first 主模型 ──────────────────────────────

-- 1. 题库主表（主中心）
CREATE TABLE IF NOT EXISTS library_items (
    id                   TEXT PRIMARY KEY,        -- 稳定 UUID
    subject              TEXT NOT NULL CHECK (subject IN ('reading','writing')),
    content_type         TEXT NOT NULL,           -- 'reading_exam' | 'writing_task'
    title                TEXT NOT NULL,
    category             TEXT,                    -- P1|P2|P3 / task1|task2
    difficulty           TEXT,                    -- low|medium|high（原 frequency）
    status               TEXT NOT NULL CHECK (status IN ('draft','review_required','ready','published','archived')),
    tags_json            TEXT NOT NULL DEFAULT '[]',
    current_revision_id  TEXT,                    -- → library_item_revisions.id
    source_asset_id      TEXT,                    -- → source_assets.id
    linked_ingest_job_id TEXT,                    -- → ingest_jobs.id（来源追溯）
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL,
    deleted_at           TEXT                     -- 软删除（NULL=未删）
);
CREATE INDEX IF NOT EXISTS idx_library_items_status  ON library_items(status) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_library_items_subject ON library_items(subject) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_library_items_deleted ON library_items(deleted_at);

-- 2. 正文版本表（支持版本/追溯）
CREATE TABLE IF NOT EXISTS library_item_revisions (
    id                  TEXT PRIMARY KEY,
    library_item_id     TEXT NOT NULL REFERENCES library_items(id),
    revision_no         INTEGER NOT NULL,
    payload_json        TEXT NOT NULL,           -- ReadingAuthoringIR / WritingJob 正文
    schema_version      TEXT NOT NULL,
    created_from_job_id TEXT,                    -- 由哪个 ingest job 产出
    change_reason       TEXT,
    created_at          TEXT NOT NULL,
    UNIQUE(library_item_id, revision_no)
);
CREATE INDEX IF NOT EXISTS idx_revisions_item ON library_item_revisions(library_item_id);

-- 3. 导入任务表（任务态与题库态分离）
CREATE TABLE IF NOT EXISTS ingest_jobs (
    id                      TEXT PRIMARY KEY,
    kind                    TEXT NOT NULL,       -- 'reading' | 'writing'
    subject                 TEXT,
    status                  TEXT NOT NULL CHECK (status IN ('created','importing','parsing','review_required','authoring','ready_to_publish','published','archived','failed')),
    current_step            TEXT,                -- 保留 WorkflowStep 兼容
    source_asset_id         TEXT,
    linked_library_item_id  TEXT,                -- 产物指向题库条目
    issue_counts_json       TEXT NOT NULL DEFAULT '{}',
    created_at              TEXT NOT NULL,
    updated_at              TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ingest_jobs_status ON ingest_jobs(status);

-- 4. 原始文件表
CREATE TABLE IF NOT EXISTS source_assets (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,                 -- pdf|docx|txt|md|image
    original_name TEXT NOT NULL,
    stored_path   TEXT NOT NULL,                 -- 相对 appData
    sha256        TEXT,
    size_bytes    INTEGER,
    role          TEXT,                          -- MainQuestion|AnswerKey|Explanation|Asset
    created_at    TEXT NOT NULL
);

-- 5. 发布记录表（Phase 3 接管导出时启用）
CREATE TABLE IF NOT EXISTS publish_records (
    id              TEXT PRIMARY KEY,
    library_item_id TEXT NOT NULL REFERENCES library_items(id),
    revision_id     TEXT NOT NULL REFERENCES library_item_revisions(id),
    publish_type    TEXT NOT NULL,               -- single_js|batch_js|nas_library|pack|writing_library
    target          TEXT,
    output_path     TEXT,
    status          TEXT NOT NULL,               -- success|failed
    summary_json    TEXT,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_publish_item ON publish_records(library_item_id);

-- 6. 过程产物索引（只索引路径，不存大文件）
CREATE TABLE IF NOT EXISTS job_artifact_index (
    id            TEXT PRIMARY KEY,
    ingest_job_id TEXT NOT NULL,
    artifact_type TEXT NOT NULL,                 -- document_ir|split_candidates|preview|thumbnail
    stored_path   TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
"#;

// ── library_meta 读写 ──────────────────────────────────────────────────────

pub(crate) fn set_meta(conn: &Connection, key: &str, value: &str) -> CommandResult<()> {
    conn.execute(
        "INSERT INTO library_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )
    .map_err(|error| format!("set_meta:{}:{}", key, error))?;
    Ok(())
}

pub(crate) fn get_meta(conn: &Connection, key: &str) -> CommandResult<Option<String>> {
    let value: Option<String> = conn
        .query_row("SELECT value FROM library_meta WHERE key=?1", params![key], |row| row.get(0))
        .optional()
        .map_err(|error| format!("get_meta:{}:{}", key, error))?;
    Ok(value)
}

// ── ExamRecord：upsert 的输入 ──────────────────────────────────────────────

/// 一条题库记录（已拍平为 DB 列），由 library_commands 从 ImportJob/WritingJob 构造。
pub(crate) struct ExamRecord {
    pub id: String,
    pub exam_id: Option<String>,
    pub title: String,
    pub subject: String, // "reading" | "writing"
    pub category: Option<String>,
    pub frequency: Option<String>,
    pub status: String, // 统一枚举：draft|needs_review|ready|exported
    pub task_type: Option<String>,
    pub tags: Vec<String>,
    pub payload_json: String,
    pub source_hash: Option<String>,
    pub issue_errors: u32,
    pub issue_warnings: u32,
    pub created_at: String, // ISO8601
    pub updated_at: String,
}

/// 插入或更新一条题目（按 id 冲突更新，created_at 保留首次值）。
pub(crate) fn upsert_exam_conn(conn: &Connection, record: &ExamRecord) -> CommandResult<()> {
    let tags_json = serde_json::to_string(&record.tags).map_err(|error| error.to_string())?;
    conn.execute(
        "INSERT INTO exams
            (id, exam_id, title, subject, category, frequency, status, task_type,
             tags_json, payload_json, source_hash, issue_errors, issue_warnings,
             created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
         ON CONFLICT(id) DO UPDATE SET
            exam_id=excluded.exam_id, title=excluded.title, subject=excluded.subject,
            category=excluded.category, frequency=excluded.frequency, status=excluded.status,
            task_type=excluded.task_type, tags_json=excluded.tags_json,
            payload_json=excluded.payload_json, source_hash=excluded.source_hash,
            issue_errors=excluded.issue_errors, issue_warnings=excluded.issue_warnings,
            updated_at=excluded.updated_at",
        params![
            record.id,
            record.exam_id,
            record.title,
            record.subject,
            record.category,
            record.frequency,
            record.status,
            record.task_type,
            tags_json,
            record.payload_json,
            record.source_hash,
            record.issue_errors,
            record.issue_warnings,
            record.created_at,
            record.updated_at,
        ],
    )
    .map_err(|error| format!("upsert_exam:{}:{}", record.id, error))?;
    Ok(())
}

/// 便捷封装：打开瞬态连接后 upsert（供双写钩子使用）。
pub(crate) fn upsert_exam(root: &Path, record: &ExamRecord) -> CommandResult<()> {
    let conn = open_connection(root)?;
    upsert_exam_conn(&conn, record)
}

// ── 行 → Summary 映射 ─────────────────────────────────────────────────────

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryExamSummary> {
    let tags_json: String = row.get("tags_json")?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    let raw_status: String = row.get("status")?;
    Ok(LibraryExamSummary {
        id: row.get("id")?,
        exam_id: row.get("exam_id")?,
        title: row.get("title")?,
        subject: row.get("subject")?,
        category: row.get("category")?,
        frequency: row.get("frequency")?,
        status: normalize_public_status(&raw_status).to_string(),
        task_type: row.get("task_type")?,
        tags,
        source_hash: row.get("source_hash")?,
        issue_errors: row.get("issue_errors")?,
        issue_warnings: row.get("issue_warnings")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

const SUMMARY_COLUMNS: &str = "id, exam_id, title, subject, category, frequency, status, \
     task_type, tags_json, source_hash, issue_errors, issue_warnings, created_at, updated_at";

fn normalize_public_status(status: &str) -> &str {
    match status {
        "review_required" => "needs_review",
        "published" => "exported",
        other => other,
    }
}

fn to_library_item_status(status: &str) -> &str {
    match status {
        "needs_review" => "review_required",
        "exported" => "published",
        other => other,
    }
}

fn active_summary_source_sql() -> String {
    format!(
        "SELECT
            e.id AS id,
            e.exam_id AS exam_id,
            COALESCE(li.title, e.title) AS title,
            COALESCE(li.subject, e.subject) AS subject,
            COALESCE(li.category, e.category) AS category,
            COALESCE(li.difficulty, e.frequency) AS frequency,
            CASE
                WHEN li.id IS NOT NULL THEN CASE li.status
                    WHEN 'review_required' THEN 'needs_review'
                    WHEN 'published' THEN 'exported'
                    ELSE li.status
                END
                ELSE e.status
            END AS status,
            COALESCE(
                e.task_type,
                CASE WHEN COALESCE(li.subject, e.subject)='writing' THEN li.category ELSE NULL END
            ) AS task_type,
            COALESCE(li.tags_json, e.tags_json) AS tags_json,
            e.source_hash AS source_hash,
            e.issue_errors AS issue_errors,
            e.issue_warnings AS issue_warnings,
            COALESCE(li.created_at, e.created_at) AS created_at,
            CASE
                WHEN li.updated_at IS NULL THEN e.updated_at
                WHEN li.updated_at > e.updated_at THEN li.updated_at
                ELSE e.updated_at
            END AS updated_at
         FROM exams e
         LEFT JOIN library_items li ON li.id = e.id
         WHERE li.deleted_at IS NULL"
    )
}

fn active_detail_source_sql() -> String {
    format!(
        "SELECT
            e.id AS id,
            e.exam_id AS exam_id,
            COALESCE(li.title, e.title) AS title,
            COALESCE(li.subject, e.subject) AS subject,
            COALESCE(li.category, e.category) AS category,
            COALESCE(li.difficulty, e.frequency) AS frequency,
            CASE
                WHEN li.id IS NOT NULL THEN CASE li.status
                    WHEN 'review_required' THEN 'needs_review'
                    WHEN 'published' THEN 'exported'
                    ELSE li.status
                END
                ELSE e.status
            END AS status,
            COALESCE(
                e.task_type,
                CASE WHEN COALESCE(li.subject, e.subject)='writing' THEN li.category ELSE NULL END
            ) AS task_type,
            COALESCE(li.tags_json, e.tags_json) AS tags_json,
            e.source_hash AS source_hash,
            e.issue_errors AS issue_errors,
            e.issue_warnings AS issue_warnings,
            COALESCE(li.created_at, e.created_at) AS created_at,
            CASE
                WHEN li.updated_at IS NULL THEN e.updated_at
                WHEN li.updated_at > e.updated_at THEN li.updated_at
                ELSE e.updated_at
            END AS updated_at,
            COALESCE(rev.payload_json, e.payload_json) AS payload_json
         FROM exams e
         LEFT JOIN library_items li ON li.id = e.id
         LEFT JOIN library_item_revisions rev ON rev.id = li.current_revision_id
         WHERE li.deleted_at IS NULL"
    )
}

fn get_legacy_exam_record(conn: &Connection, id: &str) -> CommandResult<Option<ExamRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, exam_id, title, subject, category, frequency, status, task_type,
                    tags_json, payload_json, source_hash, issue_errors, issue_warnings,
                    created_at, updated_at
             FROM exams
             WHERE id=?1",
        )
        .map_err(|e| format!("legacy_exam_prepare:{}", e))?;
    let mut rows = stmt
        .query_map(params![id], |row| {
            let tags_json: String = row.get("tags_json")?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            Ok(ExamRecord {
                id: row.get("id")?,
                exam_id: row.get("exam_id")?,
                title: row.get("title")?,
                subject: row.get("subject")?,
                category: row.get("category")?,
                frequency: row.get("frequency")?,
                status: row.get("status")?,
                task_type: row.get("task_type")?,
                tags,
                payload_json: row.get("payload_json")?,
                source_hash: row.get("source_hash")?,
                issue_errors: row.get("issue_errors")?,
                issue_warnings: row.get("issue_warnings")?,
                created_at: row.get("created_at")?,
                updated_at: row.get("updated_at")?,
            })
        })
        .map_err(|e| format!("legacy_exam_query:{}", e))?;
    match rows.next() {
        None => Ok(None),
        Some(row) => Ok(Some(row.map_err(|e| format!("legacy_exam_row:{}", e))?)),
    }
}

fn infer_revision_schema_version(subject: &str, payload_json: &str) -> String {
    serde_json::from_str::<Value>(payload_json)
        .ok()
        .and_then(|payload| payload.get("schemaVersion").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| {
            if subject == "writing" {
                "WritingExamSourceV1".to_string()
            } else {
                "ReadingAuthoringIRV1".to_string()
            }
        })
}

fn library_item_record_from_exam(record: &ExamRecord) -> LibraryItemRecord {
    LibraryItemRecord {
        id: record.id.clone(),
        subject: record.subject.clone(),
        content_type: if record.subject == "writing" {
            "writing_task".to_string()
        } else {
            "reading_exam".to_string()
        },
        title: record.title.clone(),
        category: record.category.clone(),
        difficulty: record.frequency.clone(),
        status: to_library_item_status(&record.status).to_string(),
        task_type: record.task_type.clone(),
        tags: record.tags.clone(),
        source_asset_id: record.source_hash.clone(),
        linked_ingest_job_id: Some(record.id.clone()),
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
        revision_payload_json: record.payload_json.clone(),
        schema_version: infer_revision_schema_version(&record.subject, &record.payload_json),
        created_from_job_id: Some(record.id.clone()),
        change_reason: Some("legacy_exam_backfill".to_string()),
    }
}

fn library_item_exists(conn: &Connection, id: &str) -> CommandResult<bool> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM library_items WHERE id=?1",
            params![id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|e| format!("library_item_exists:{}:{}", id, e))?
        .is_some();
    Ok(exists)
}

pub(crate) fn ensure_library_item_for_exam(conn: &Connection, id: &str) -> CommandResult<bool> {
    if library_item_exists(conn, id)? {
        return Ok(true);
    }
    let Some(record) = get_legacy_exam_record(conn, id)? else {
        return Ok(false);
    };
    upsert_library_item(conn, &library_item_record_from_exam(&record))?;
    Ok(true)
}

// ── 查询：列表 / 详情 / 搜索 / 统计 / 更新 / 删除 ─────────────────────────

pub(crate) fn list_exams(conn: &Connection, filter: &LibraryFilter) -> CommandResult<Vec<LibraryExamSummary>> {
    let mut sql = format!(
        "SELECT {SUMMARY_COLUMNS} FROM ({}) active WHERE 1=1",
        active_summary_source_sql()
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(subject) = &filter.subject {
        sql.push_str(" AND subject=?");
        args.push(Box::new(subject.clone()));
    }
    if let Some(status) = &filter.status {
        sql.push_str(" AND status=?");
        args.push(Box::new(status.clone()));
    }
    if let Some(category) = &filter.category {
        sql.push_str(" AND category=?");
        args.push(Box::new(category.clone()));
    }
    sql.push_str(" ORDER BY updated_at DESC");
    let limit = filter.limit.unwrap_or(200).clamp(1, 1000) as i64;
    let offset = filter.offset.unwrap_or(0).max(0) as i64;
    sql.push_str(" LIMIT ? OFFSET ?");
    args.push(Box::new(limit));
    args.push(Box::new(offset));

    let params_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("list_exams_prepare:{}", e))?;
    let rows = stmt
        .query_map(params_refs.as_slice(), row_to_summary)
        .map_err(|e| format!("list_exams_query:{}", e))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("list_exams_row:{}", e))?);
    }
    Ok(out)
}

pub(crate) fn get_exam(conn: &Connection, id: &str) -> CommandResult<Option<LibraryExamDetail>> {
    let sql = format!(
        "SELECT {SUMMARY_COLUMNS}, payload_json FROM ({}) active WHERE id=?1 LIMIT 1",
        active_detail_source_sql()
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("get_exam_prepare:{}", e))?;
    let mut rows = stmt
        .query_map(params![id], |row| {
            let summary = row_to_summary(row)?;
            let payload_json: String = row.get("payload_json")?;
            Ok((summary, payload_json))
        })
        .map_err(|e| format!("get_exam_query:{}", e))?;
    match rows.next() {
        None => Ok(None),
        Some(row) => {
            let (summary, payload_json) = row.map_err(|e| format!("get_exam_row:{}", e))?;
            let payload: Value = serde_json::from_str(&payload_json).unwrap_or(Value::Null);
            Ok(Some(LibraryExamDetail { summary, payload }))
        }
    }
}

pub(crate) fn search_exams(conn: &Connection, query: &str) -> CommandResult<Vec<LibraryExamSummary>> {
    let pattern = format!("%{}%", query.trim().to_lowercase());
    let sql = format!(
        "SELECT {SUMMARY_COLUMNS} FROM ({}) active
         WHERE LOWER(title) LIKE ?1 OR LOWER(IFNULL(exam_id,'')) LIKE ?1 OR LOWER(tags_json) LIKE ?1
         ORDER BY updated_at DESC LIMIT 200"
        ,
        active_summary_source_sql()
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("search_exams_prepare:{}", e))?;
    let rows = stmt
        .query_map(params![pattern], row_to_summary)
        .map_err(|e| format!("search_exams_query:{}", e))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("search_exams_row:{}", e))?);
    }
    Ok(out)
}

pub(crate) fn get_stats(conn: &Connection) -> CommandResult<LibraryStats> {
    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM ({}) active", active_summary_source_sql()),
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("stats_total:{}", e))?;

    fn group_count(conn: &Connection, column: &str) -> CommandResult<BTreeMap<String, u32>> {
        let sql = format!(
            "SELECT {column}, COUNT(*) FROM ({}) active GROUP BY {column}",
            active_summary_source_sql()
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| format!("stats_group_prepare:{}", e))?;
        let rows = stmt
            .query_map([], |row| {
                let key: Option<String> = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok((key.unwrap_or_else(|| "(none)".to_string()), count as u32))
            })
            .map_err(|e| format!("stats_group_query:{}", e))?;
        let mut map = BTreeMap::new();
        for row in rows {
            let (k, v) = row.map_err(|e| format!("stats_group_row:{}", e))?;
            map.insert(k, v);
        }
        Ok(map)
    }

    Ok(LibraryStats {
        total: total as u32,
        by_subject: group_count(conn, "subject")?,
        by_status: group_count(conn, "status")?,
        by_category: group_count(conn, "category")?,
    })
}

pub(crate) fn update_exam_meta(
    conn: &Connection,
    id: &str,
    patch: &LibraryMetaPatch,
) -> CommandResult<Option<LibraryExamSummary>> {
    // 逐字段动态拼 UPDATE，只更新提供的字段。
    let mut sets: Vec<String> = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(title) = &patch.title {
        sets.push("title=?".into());
        args.push(Box::new(title.clone()));
    }
    if let Some(category) = &patch.category {
        sets.push("category=?".into());
        args.push(Box::new(category.clone()));
    }
    if let Some(frequency) = &patch.frequency {
        sets.push("frequency=?".into());
        args.push(Box::new(frequency.clone()));
    }
    if let Some(status) = &patch.status {
        sets.push("status=?".into());
        args.push(Box::new(status.clone()));
    }
    if let Some(task_type) = &patch.task_type {
        sets.push("task_type=?".into());
        args.push(Box::new(task_type.clone()));
    }
    if let Some(tags) = &patch.tags {
        let tags_json = serde_json::to_string(tags).map_err(|e| e.to_string())?;
        sets.push("tags_json=?".into());
        args.push(Box::new(tags_json));
    }
    if sets.is_empty() {
        return get_exam_summary(conn, id);
    }
    sets.push("updated_at=?".into());
    args.push(Box::new(chrono::Utc::now().to_rfc3339()));
    args.push(Box::new(id.to_string()));

    let sql = format!("UPDATE exams SET {} WHERE id=?", sets.join(", "));
    let params_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let affected = conn
        .execute(&sql, params_refs.as_slice())
        .map_err(|e| format!("update_exam_meta:{}", e))?;
    if affected == 0 {
        return Ok(None);
    }
    update_library_item_meta(conn, id, patch)?;
    get_exam_summary(conn, id)
}

/// 取单条 Summary（不含 payload），update_exam_meta 回读用。
fn get_exam_summary(conn: &Connection, id: &str) -> CommandResult<Option<LibraryExamSummary>> {
    let sql = format!(
        "SELECT {SUMMARY_COLUMNS} FROM ({}) active WHERE id=?1 LIMIT 1",
        active_summary_source_sql()
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("get_summary_prepare:{}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_summary)
        .map_err(|e| format!("get_summary_query:{}", e))?;
    match rows.next() {
        None => Ok(None),
        Some(row) => Ok(Some(row.map_err(|e| format!("get_summary_row:{}", e))?)),
    }
}

fn update_library_item_meta(conn: &Connection, id: &str, patch: &LibraryMetaPatch) -> CommandResult<()> {
    if !library_item_exists(conn, id)? {
        return Ok(());
    }

    let mut sets: Vec<String> = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(title) = &patch.title {
        sets.push("title=?".into());
        args.push(Box::new(title.clone()));
    }
    if let Some(category) = &patch.category {
        sets.push("category=?".into());
        args.push(Box::new(category.clone()));
    } else if let Some(task_type) = &patch.task_type {
        sets.push("category=?".into());
        args.push(Box::new(task_type.clone()));
    }
    if let Some(frequency) = &patch.frequency {
        sets.push("difficulty=?".into());
        args.push(Box::new(frequency.clone()));
    }
    if let Some(status) = &patch.status {
        sets.push("status=?".into());
        args.push(Box::new(to_library_item_status(status).to_string()));
    }
    if let Some(tags) = &patch.tags {
        let tags_json = serde_json::to_string(tags).map_err(|e| e.to_string())?;
        sets.push("tags_json=?".into());
        args.push(Box::new(tags_json));
    }
    if sets.is_empty() {
        return Ok(());
    }

    sets.push("updated_at=?".into());
    args.push(Box::new(chrono::Utc::now().to_rfc3339()));
    args.push(Box::new(id.to_string()));

    let sql = format!("UPDATE library_items SET {} WHERE id=?", sets.join(", "));
    let params_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    conn.execute(&sql, params_refs.as_slice())
        .map_err(|e| format!("update_library_item_meta:{}", e))?;
    Ok(())
}

pub(crate) fn delete_exam(conn: &Connection, id: &str) -> CommandResult<bool> {
    let affected = conn
        .execute("DELETE FROM exams WHERE id=?", params![id])
        .map_err(|e| format!("delete_exam:{}", e))?;
    Ok(affected > 0)
}

/// 便捷封装：打开瞬态连接后删除一条题目（供 delete_job/delete_writing_job 同步使用）。
pub(crate) fn delete_exam_by_id(root: &Path, id: &str) -> CommandResult<()> {
    let conn = open_connection(root)?;
    delete_exam(&conn, id)?;
    Ok(())
}

/// 清理 DB 中 id 不在给定「存活 id 集合」内的孤儿行。
/// 供迁移阶段调用：删掉 DB 中已无对应 job.json/writing-job.json 的记录
/// （之前删除文件时 DB 删除失败留下的孤儿）。
pub(crate) fn prune_orphan_exams(conn: &Connection, live_ids: &std::collections::HashSet<String>) -> CommandResult<usize> {
    let all: Vec<String> = {
        let mut stmt = conn.prepare("SELECT id FROM exams").map_err(|e| format!("prune_prepare:{}", e))?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).map_err(|e| format!("prune_query:{}", e))?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r.map_err(|e| format!("prune_row:{}", e))?);
        }
        v
    };
    let mut removed = 0usize;
    for id in &all {
        if !live_ids.contains(id) {
            conn.execute("DELETE FROM exams WHERE id=?1", params![id])
                .map_err(|e| format!("prune_delete:{}:{}", id, e))?;
            removed += 1;
        }
    }
    Ok(removed)
}

// ── Phase 1：library_items + library_item_revisions 正式主模型 ──────────────
//
// upsert_library_item 同时写 library_items（元数据）+ library_item_revisions（正文版本）。
// 每次保存生成新 revision（revision_no 递增），current_revision_id 指向最新版。
// 软删除：delete 置 deleted_at，restore 清空 deleted_at。

/// 构造一条 library_item upsert 输入（由 library_commands 从 ExamRecord 派生）。
#[derive(Clone)]
pub(crate) struct LibraryItemRecord {
    pub id: String,                     // 稳定 UUID（首版用 job_id，后续保持）
    pub subject: String,                // reading|writing
    pub content_type: String,           // reading_exam|writing_task
    pub title: String,
    pub category: Option<String>,
    pub difficulty: Option<String>,     // 原 frequency
    pub status: String,                 // draft|review_required|ready|published|archived
    pub task_type: Option<String>,
    pub tags: Vec<String>,
    pub source_asset_id: Option<String>,
    pub linked_ingest_job_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    // revision 内容
    pub revision_payload_json: String,
    pub schema_version: String,         // ReadingAuthoringIRV1 | WritingExamSourceV1
    pub created_from_job_id: Option<String>,
    pub change_reason: Option<String>,
}

/// 插入或更新 library_item，并写入一条新 revision。
/// 首次创建：插入 item + revision_no=1。后续更新：更新 item 元数据 + 新增 revision_no+1。
/// 用事务保证 item 与 revision 原子写入。
pub(crate) fn upsert_library_item(conn: &Connection, record: &LibraryItemRecord) -> CommandResult<String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("lib_item_begin:{}", e))?;
    // 查现有 item 与最大 revision_no。
    let existing: Option<(Option<String>, i64)> = tx
        .query_row(
            "SELECT current_revision_id, COALESCE((SELECT MAX(revision_no) FROM library_item_revisions WHERE library_item_id=?1),0) FROM library_items WHERE id=?1",
            params![record.id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|e| format!("lib_item_lookup:{}", e))?;
    let tags_json = serde_json::to_string(&record.tags).map_err(|e| e.to_string())?;

    let revision_id = format!("rev-{}-{}", &record.id[..record.id.len().min(12)], uuid::Uuid::new_v4().simple());
    let revision_no = existing.map(|(_, n)| n + 1).unwrap_or(1);

    // upsert item。
    tx.execute(
        "INSERT INTO library_items
            (id, subject, content_type, title, category, difficulty, status, tags_json,
             current_revision_id, source_asset_id, linked_ingest_job_id, created_at, updated_at, deleted_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,NULL)
         ON CONFLICT(id) DO UPDATE SET
            subject=excluded.subject, content_type=excluded.content_type, title=excluded.title,
            category=excluded.category, difficulty=excluded.difficulty, status=excluded.status,
            tags_json=excluded.tags_json, current_revision_id=excluded.current_revision_id,
            source_asset_id=excluded.source_asset_id, linked_ingest_job_id=excluded.linked_ingest_job_id,
            updated_at=excluded.updated_at",
        params![
            record.id, record.subject, record.content_type, record.title, record.category,
            record.difficulty, record.status, tags_json, revision_id, record.source_asset_id,
            record.linked_ingest_job_id, record.created_at, record.updated_at,
        ],
    )
    .map_err(|e| format!("lib_item_upsert:{}:{}", record.id, e))?;

    // 插入新 revision。
    tx.execute(
        "INSERT INTO library_item_revisions
            (id, library_item_id, revision_no, payload_json, schema_version, created_from_job_id, change_reason, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            revision_id, record.id, revision_no, record.revision_payload_json,
            record.schema_version, record.created_from_job_id, record.change_reason, record.updated_at,
        ],
    )
    .map_err(|e| format!("lib_revision_insert:{}:{}", record.id, e))?;

    tx.commit().map_err(|e| format!("lib_item_commit:{}", e))?;
    Ok(revision_id)
}

/// 软删除：置 deleted_at（不物理删除，可恢复）。
pub(crate) fn soft_delete_library_item(conn: &Connection, id: &str) -> CommandResult<bool> {
    let affected = conn
        .execute(
            "UPDATE library_items SET deleted_at=?1, updated_at=?1 WHERE id=?2 AND deleted_at IS NULL",
            params![now_iso(), id],
        )
        .map_err(|e| format!("soft_delete:{}:{}", id, e))?;
    Ok(affected > 0)
}

/// 恢复软删除：清空 deleted_at。
pub(crate) fn restore_library_item(conn: &Connection, id: &str) -> CommandResult<bool> {
    let affected = conn
        .execute(
            "UPDATE library_items SET deleted_at=NULL, updated_at=?1 WHERE id=?2 AND deleted_at IS NOT NULL",
            params![now_iso(), id],
        )
        .map_err(|e| format!("restore:{}:{}", id, e))?;
    Ok(affected > 0)
}

fn exam_record_from_library_item(conn: &Connection, id: &str) -> CommandResult<Option<ExamRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT li.id, li.subject, li.title, li.category, li.difficulty, li.status, li.tags_json,
                    li.created_at, li.updated_at, rev.payload_json
             FROM library_items li
             LEFT JOIN library_item_revisions rev ON rev.id = li.current_revision_id
             WHERE li.id=?1",
        )
        .map_err(|e| format!("library_item_exam_prepare:{}", e))?;
    let mut rows = stmt
        .query_map(params![id], |row| {
            let tags_json: String = row.get("tags_json")?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            let payload_json: String = row.get::<_, Option<String>>("payload_json")?.unwrap_or_else(|| "{}".to_string());
            let payload_value = serde_json::from_str::<Value>(&payload_json).unwrap_or(Value::Null);
            let subject: String = row.get("subject")?;
            let category: Option<String> = row.get("category")?;
            let task_type = if subject == "writing" {
                payload_value
                    .get("taskType")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| category.clone())
            } else {
                None
            };
            let exam_id = if subject == "writing" {
                payload_value
                    .get("examId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                payload_value
                    .get("exam")
                    .and_then(Value::as_object)
                    .and_then(|exam| exam.get("examId"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| payload_value.get("examId").and_then(Value::as_str).map(str::to_string))
                    .or_else(|| category.as_ref().map(|_| id.to_string()))
            };

            Ok(ExamRecord {
                id: row.get("id")?,
                exam_id,
                title: row.get("title")?,
                subject,
                category,
                frequency: row.get("difficulty")?,
                status: normalize_public_status(&row.get::<_, String>("status")?).to_string(),
                task_type,
                tags,
                payload_json,
                source_hash: None,
                issue_errors: 0,
                issue_warnings: 0,
                created_at: row.get("created_at")?,
                updated_at: row.get("updated_at")?,
            })
        })
        .map_err(|e| format!("library_item_exam_query:{}", e))?;
    match rows.next() {
        None => Ok(None),
        Some(row) => Ok(Some(row.map_err(|e| format!("library_item_exam_row:{}", e))?)),
    }
}

pub(crate) fn restore_exam_from_library_item(conn: &Connection, id: &str) -> CommandResult<bool> {
    if get_legacy_exam_record(conn, id)?.is_some() {
        return Ok(true);
    }
    let Some(record) = exam_record_from_library_item(conn, id)? else {
        return Ok(false);
    };
    upsert_exam_conn(conn, &record)?;
    Ok(true)
}

/// 列出已软删除的题库条目（回收站）。
pub(crate) fn list_trashed_items(conn: &Connection) -> CommandResult<Vec<LibraryExamSummary>> {
    let sql = "SELECT id, NULL AS exam_id, title, subject, category, difficulty AS frequency, status, \
               CASE WHEN subject='writing' THEN category ELSE NULL END AS task_type, \
               tags_json, NULL AS source_hash, 0 AS issue_errors, 0 AS issue_warnings, \
               created_at, updated_at \
               FROM library_items WHERE deleted_at IS NOT NULL ORDER BY updated_at DESC";
    let mut stmt = conn.prepare(sql).map_err(|e| format!("trash_prepare:{}", e))?;
    let rows = stmt.query_map([], row_to_summary).map_err(|e| format!("trash_query:{}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("trash_row:{}", e))?);
    }
    Ok(out)
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        conn
    }

    fn sample_record(id: &str, subject: &str, status: &str) -> ExamRecord {
        ExamRecord {
            id: id.to_string(),
            exam_id: Some(format!("exam-{id}")),
            title: format!("Title {id}"),
            subject: subject.to_string(),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            status: status.to_string(),
            task_type: if subject == "writing" { Some("task1".to_string()) } else { None },
            tags: vec!["test".to_string()],
            payload_json: r#"{"hello":"world"}"#.to_string(),
            source_hash: Some("abc".to_string()),
            issue_errors: 0,
            issue_warnings: 1,
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-02T00:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = mem_conn();
        // 二次建表不应报错。
        ensure_schema(&conn).unwrap();
    }

    #[test]
    fn upsert_inserts_then_updates() {
        let conn = mem_conn();
        upsert_exam_conn(&conn, &sample_record("r1", "reading", "draft")).unwrap();
        let detail = get_exam(&conn, "r1").unwrap().unwrap();
        assert_eq!(detail.summary.title, "Title r1");
        assert_eq!(detail.summary.status, "draft");
        assert_eq!(detail.payload["hello"], "world");

        // 更新同 id：title/status 变，created_at 保留。
        let mut rec = sample_record("r1", "reading", "ready");
        rec.title = "Updated".into();
        rec.created_at = "2026-01-09T00:00:00+00:00".into(); // 不应覆盖
        upsert_exam_conn(&conn, &rec).unwrap();
        let detail = get_exam(&conn, "r1").unwrap().unwrap();
        assert_eq!(detail.summary.title, "Updated");
        assert_eq!(detail.summary.status, "ready");
        assert_eq!(detail.summary.created_at, "2026-01-01T00:00:00+00:00");
    }

    #[test]
    fn list_filters_by_subject_and_status() {
        let conn = mem_conn();
        upsert_exam_conn(&conn, &sample_record("r1", "reading", "draft")).unwrap();
        upsert_exam_conn(&conn, &sample_record("r2", "reading", "ready")).unwrap();
        upsert_exam_conn(&conn, &sample_record("w1", "writing", "draft")).unwrap();

        let all = list_exams(&conn, &LibraryFilter::default()).unwrap();
        assert_eq!(all.len(), 3);

        let reading = list_exams(
            &conn,
            &LibraryFilter { subject: Some("reading".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(reading.len(), 2);

        let ready = list_exams(
            &conn,
            &LibraryFilter { status: Some("ready".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "r2");
    }

    #[test]
    fn search_matches_title_and_tags() {
        let conn = mem_conn();
        upsert_exam_conn(&conn, &sample_record("r1", "reading", "draft")).unwrap();
        let hits = search_exams(&conn, "title r1").unwrap();
        assert_eq!(hits.len(), 1);
        let hits = search_exams(&conn, "test").unwrap(); // tag
        assert_eq!(hits.len(), 1);
        let misses = search_exams(&conn, "nope").unwrap();
        assert!(misses.is_empty());
    }

    #[test]
    fn stats_group_counts() {
        let conn = mem_conn();
        upsert_exam_conn(&conn, &sample_record("r1", "reading", "draft")).unwrap();
        upsert_exam_conn(&conn, &sample_record("r2", "reading", "ready")).unwrap();
        upsert_exam_conn(&conn, &sample_record("w1", "writing", "draft")).unwrap();
        let stats = get_stats(&conn).unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.by_subject.get("reading"), Some(&2));
        assert_eq!(stats.by_subject.get("writing"), Some(&1));
        assert_eq!(stats.by_status.get("draft"), Some(&2));
        assert_eq!(stats.by_status.get("ready"), Some(&1));
    }

    #[test]
    fn update_meta_partial_and_delete() {
        let conn = mem_conn();
        upsert_exam_conn(&conn, &sample_record("r1", "reading", "draft")).unwrap();
        let updated = update_exam_meta(
            &conn,
            "r1",
            &LibraryMetaPatch { title: Some("New".into()), status: Some("ready".into()), ..Default::default() },
        )
        .unwrap()
        .unwrap();
        assert_eq!(updated.title, "New");
        assert_eq!(updated.status, "ready");

        assert!(delete_exam(&conn, "r1").unwrap());
        assert!(get_exam(&conn, "r1").unwrap().is_none());
        assert!(!delete_exam(&conn, "missing").unwrap());
    }

    #[test]
    fn meta_get_set() {
        let conn = mem_conn();
        assert!(get_meta(&conn, "k").unwrap().is_none());
        set_meta(&conn, "k", "v1").unwrap();
        assert_eq!(get_meta(&conn, "k").unwrap().as_deref(), Some("v1"));
        set_meta(&conn, "k", "v2").unwrap();
        assert_eq!(get_meta(&conn, "k").unwrap().as_deref(), Some("v2"));
    }

    #[test]
    fn check_constraint_rejects_invalid_status_and_subject() {
        let conn = mem_conn();
        // 非法 status 应被 CHECK 拒绝。
        let mut bad_status = sample_record("x1", "reading", "INVALID_STATUS");
        let res = upsert_exam_conn(&conn, &bad_status);
        assert!(res.is_err(), "invalid status must be rejected");
        // 非法 subject 应被 CHECK 拒绝。
        bad_status.status = "draft".into();
        bad_status.subject = "math".into();
        let res = upsert_exam_conn(&conn, &bad_status);
        assert!(res.is_err(), "invalid subject must be rejected");
        // 合法值可通过。
        let good = sample_record("x1", "reading", "draft");
        assert!(upsert_exam_conn(&conn, &good).is_ok());
    }

    // ── Phase 1：library_items + revisions + 软删除 测试 ──────────────────────

    fn sample_library_item(id: &str, subject: &str, status: &str) -> LibraryItemRecord {
        LibraryItemRecord {
            id: id.to_string(),
            subject: subject.to_string(),
            content_type: if subject == "writing" { "writing_task" } else { "reading_exam" }.to_string(),
            title: format!("Item {id}"),
            category: Some("P1".to_string()),
            difficulty: Some("medium".to_string()),
            status: status.to_string(),
            task_type: if subject == "writing" { Some("task1".to_string()) } else { None },
            tags: vec!["t".to_string()],
            source_asset_id: None,
            linked_ingest_job_id: Some(id.to_string()),
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-02T00:00:00+00:00".to_string(),
            revision_payload_json: r#"{"passage":{}}"#.to_string(),
            schema_version: "ReadingAuthoringIRV1".to_string(),
            created_from_job_id: Some(id.to_string()),
            change_reason: Some("test".to_string()),
        }
    }

    #[test]
    fn library_item_upsert_creates_revisions() {
        let conn = mem_conn();
        let rec = sample_library_item("lib-1", "reading", "draft");
        let rev1 = upsert_library_item(&conn, &rec).unwrap();
        // 第二次保存：应生成 revision_no=2，current_revision_id 更新。
        let mut rec2 = rec.clone();
        rec2.updated_at = "2026-01-03T00:00:00+00:00".to_string();
        let rev2 = upsert_library_item(&conn, &rec2).unwrap();
        assert_ne!(rev1, rev2, "each save must create a new revision id");
        // item 的 current_revision_id 指向最新。
        let current: String = conn.query_row("SELECT current_revision_id FROM library_items WHERE id='lib-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(current, rev2);
        // 两条 revision 存在，revision_no 递增。
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM library_item_revisions WHERE library_item_id='lib-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2);
        let max_no: i64 = conn.query_row("SELECT MAX(revision_no) FROM library_item_revisions WHERE library_item_id='lib-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(max_no, 2);
    }

    #[test]
    fn soft_delete_and_restore() {
        let conn = mem_conn();
        upsert_library_item(&conn, &sample_library_item("lib-2", "reading", "ready")).unwrap();
        // 软删除。
        assert!(soft_delete_library_item(&conn, "lib-2").unwrap());
        // 已删则不能再删（返回 false）。
        assert!(!soft_delete_library_item(&conn, "lib-2").unwrap());
        // 回收站能查到。
        let trash = list_trashed_items(&conn).unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].id, "lib-2");
        // 恢复。
        assert!(restore_library_item(&conn, "lib-2").unwrap());
        let trash2 = list_trashed_items(&conn).unwrap();
        assert!(trash2.is_empty(), "restored item must leave trash");
    }

    #[test]
    fn active_queries_follow_soft_delete_and_normalize_statuses() {
        let conn = mem_conn();
        upsert_exam_conn(&conn, &sample_record("r-soft", "reading", "needs_review")).unwrap();
        assert!(ensure_library_item_for_exam(&conn, "r-soft").unwrap());

        let active = list_exams(&conn, &LibraryFilter::default()).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, "needs_review");

        assert!(soft_delete_library_item(&conn, "r-soft").unwrap());
        assert!(list_exams(&conn, &LibraryFilter::default()).unwrap().is_empty());
        assert!(get_exam(&conn, "r-soft").unwrap().is_none());
        assert!(search_exams(&conn, "r-soft").unwrap().is_empty());
        assert_eq!(get_stats(&conn).unwrap().total, 0);

        let trash = list_trashed_items(&conn).unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].status, "needs_review");

        assert!(restore_library_item(&conn, "r-soft").unwrap());
        assert_eq!(list_exams(&conn, &LibraryFilter::default()).unwrap().len(), 1);
    }

    #[test]
    fn migrate_db_file_renames_legacy() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("ielts-migtest-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        // 旧 library.db 存在，新库不存在 → 应重命名。
        fs::write(dir.join("library.db"), b"placeholder").unwrap();
        migrate_db_file(&dir).unwrap();
        assert!(!dir.join("library.db").exists(), "legacy db must be renamed away");
        assert!(dir.join("authoring_hub.db").exists(), "new db must exist");
        // 二次调用幂等：新库已存在，跳过。
        fs::write(dir.join("library.db"), b"stale").unwrap();
        migrate_db_file(&dir).unwrap();
        // 新库不被覆盖（仍是 placeholder 内容）。
        let content = fs::read(dir.join("authoring_hub.db")).unwrap();
        assert_eq!(content, b"placeholder", "existing new db must not be overwritten");
        let _ = fs::remove_dir_all(&dir);
    }
}
