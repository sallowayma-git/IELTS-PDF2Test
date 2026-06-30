//! 题库 SQLite 存储层。
//!
//! 设计要点：
//! - 复用已有 `rusqlite 0.31 (bundled)` 依赖，不引入 sqlx/sea-orm/tauri-plugin-sql。
//! - 采用「每次操作打开瞬态连接」策略（与 `export_nas_library.rs` 一致），配合 WAL 模式
//!   保证读写并发安全；这样 `save_job`/`save_writing_job` 等「双写钩子」只需 `root: &Path`
//!   即可访问 DB，无需把 Connection 塞进 AppState、也无需改动现有命令签名。
//! - schema 统一收录阅读 + 写作题目，用 `subject` 列区分；正文整体存 `payload_json`，
//!   元数据建列 + 索引以支持题库列表/筛选/搜索。
//! - `library_meta` 存 schema 版本与迁移标记。

use crate::{
    LibraryExamDetail, LibraryExamSummary, LibraryFilter, LibraryMetaPatch, LibraryStats,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::CommandResult;

/// 题库 DB 文件路径：<appData>/library.db
pub(crate) fn db_path(root: &Path) -> PathBuf {
    root.join("library.db")
}

/// 打开一个连接：设置 WAL/外键/忙等超时，并确保 schema 存在（幂等）。
pub(crate) fn open_connection(root: &Path) -> CommandResult<Connection> {
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
    Ok(LibraryExamSummary {
        id: row.get("id")?,
        exam_id: row.get("exam_id")?,
        title: row.get("title")?,
        subject: row.get("subject")?,
        category: row.get("category")?,
        frequency: row.get("frequency")?,
        status: row.get("status")?,
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

// ── 查询：列表 / 详情 / 搜索 / 统计 / 更新 / 删除 ─────────────────────────

pub(crate) fn list_exams(conn: &Connection, filter: &LibraryFilter) -> CommandResult<Vec<LibraryExamSummary>> {
    let mut sql = format!("SELECT {SUMMARY_COLUMNS} FROM exams WHERE 1=1");
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
    let sql = format!("SELECT {SUMMARY_COLUMNS}, payload_json FROM exams WHERE id=?");
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
        "SELECT {SUMMARY_COLUMNS} FROM exams
         WHERE LOWER(title) LIKE ?1 OR LOWER(IFNULL(exam_id,'')) LIKE ?1 OR LOWER(tags_json) LIKE ?1
         ORDER BY updated_at DESC LIMIT 200"
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
        .query_row("SELECT COUNT(*) FROM exams", [], |row| row.get(0))
        .map_err(|e| format!("stats_total:{}", e))?;

    fn group_count(conn: &Connection, column: &str) -> CommandResult<BTreeMap<String, u32>> {
        let sql = format!("SELECT {column}, COUNT(*) FROM exams GROUP BY {column}");
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
    get_exam_summary(conn, id)
}

/// 取单条 Summary（不含 payload），update_exam_meta 回读用。
fn get_exam_summary(conn: &Connection, id: &str) -> CommandResult<Option<LibraryExamSummary>> {
    let sql = format!("SELECT {SUMMARY_COLUMNS} FROM exams WHERE id=?");
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("get_summary_prepare:{}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_summary)
        .map_err(|e| format!("get_summary_query:{}", e))?;
    match rows.next() {
        None => Ok(None),
        Some(row) => Ok(Some(row.map_err(|e| format!("get_summary_row:{}", e))?)),
    }
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
}
