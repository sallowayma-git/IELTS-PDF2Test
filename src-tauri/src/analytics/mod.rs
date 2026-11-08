//! 作者端「数据」模块（教师端第一阶段）。
//!
//! 只读消费学生端作答快照与状态信标（契约：docs/plans/2026-11-07-submission-snapshot-contract.md），
//! 结果写入应用数据根下的本地 `analytics.sqlite`。全程不写、不删、不改作答数据目录（NAS）上的任何文件。
//!
//! 判分以学生端写下的 `score_info_json`（及答案行 `is_correct` / `weight`）为准，这里不重判。
//! 只展示原始分与正确率，不展示雅思 Band。

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

use crate::CommandResult;

pub mod export;
pub mod import;
pub mod queries;
pub mod status;
mod schema_v2;
pub(crate) mod records;
mod import_extensions;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod snapshot_compat_tests;

pub const ANALYTICS_DB_FILE: &str = "analytics.sqlite";
/// 单题样本量低于这个数时，只显示人数，不下「难题」结论。
pub const HARD_CONCLUSION_MIN_N: usize = 30;
pub const DISCLAIMER: &str = "机构模拟测试原始分，非 IELTS 官方成绩。";

pub fn analytics_db_path(app_root: &Path) -> PathBuf {
    app_root.join(ANALYTICS_DB_FILE)
}

/// 打开（必要时创建）本地分析库。调用方持有连接；导入需要 `&mut` 以开事务。
pub fn open_analytics(app_root: &Path) -> CommandResult<Connection> {
    std::fs::create_dir_all(app_root).map_err(|error| format!("analytics_root:{error}"))?;
    let conn = Connection::open(analytics_db_path(app_root))
        .map_err(|error| format!("analytics_open:{error}"))?;
    conn.busy_timeout(Duration::from_millis(15_000))
        .map_err(|error| error.to_string())?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| error.to_string())?;
    ensure_schema(&conn)?;
    Ok(conn)
}

pub fn ensure_schema(conn: &Connection) -> CommandResult<()> {
    schema_v2::migrate(conn).map_err(|error| format!("analytics_schema:{error}"))
}

fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> CommandResult<()> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| format!("analytics_schema:{error}"))?;
    let names: Vec<String> = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("analytics_schema:{error}"))?
        .filter_map(Result::ok)
        .collect();
    if !names.iter().any(|name| name == column) {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl};"))
            .map_err(|error| format!("analytics_schema:{error}"))?;
    }
    Ok(())
}

/// 本地分析库表结构。`paper_id` 为空（旧套题）时存空串，便于做分组键。
pub const ANALYTICS_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS import_snapshots (
  file TEXT NOT NULL,
  sha256 TEXT NOT NULL DEFAULT '',
  terminal_id TEXT,
  attempt_id TEXT,
  batch_id TEXT,
  paper_id TEXT,
  exported_at TEXT,
  imported_at TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('ok', 'skipped', 'failed')),
  error TEXT,
  PRIMARY KEY(file, sha256)
);
CREATE TABLE IF NOT EXISTS attempts (
  attempt_id TEXT PRIMARY KEY,
  terminal_id TEXT NOT NULL,
  batch_id TEXT NOT NULL,
  paper_id TEXT NOT NULL DEFAULT '',
  candidate_id TEXT NOT NULL,
  candidate_name TEXT NOT NULL,
  checkin_at TEXT,
  started_at TEXT,
  submitted_at TEXT,
  status TEXT NOT NULL,
  app_version TEXT,
  scorer_version TEXT,
  install_id TEXT,
  exported_at TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  source_file TEXT NOT NULL,
  imported_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_attempts_group ON attempts(batch_id, paper_id);
CREATE TABLE IF NOT EXISTS suites (
  suite_attempt_id TEXT PRIMARY KEY,
  attempt_id TEXT NOT NULL,
  status TEXT,
  started_at TEXT,
  completed_at TEXT,
  submitted_at TEXT,
  total_passages INTEGER NOT NULL DEFAULT 3
);
CREATE INDEX IF NOT EXISTS idx_suites_attempt ON suites(attempt_id);
CREATE TABLE IF NOT EXISTS passages (
  attempt_id TEXT NOT NULL,
  passage_index INTEGER NOT NULL,
  suite_attempt_id TEXT NOT NULL,
  asset_id TEXT,
  exam_id TEXT,
  title TEXT,
  category TEXT,
  started_at TEXT,
  ended_at TEXT,
  duration_sec INTEGER,
  gradable INTEGER NOT NULL,
  correct REAL NOT NULL,
  total REAL NOT NULL,
  percentage REAL,
  PRIMARY KEY(attempt_id, passage_index)
);
CREATE TABLE IF NOT EXISTS answers (
  attempt_id TEXT NOT NULL,
  passage_index INTEGER NOT NULL,
  question_id TEXT NOT NULL,
  display_label TEXT,
  question_kind TEXT,
  control TEXT,
  match_mode TEXT,
  user_answer_json TEXT,
  correct_answer_json TEXT,
  normalized_user_answer_json TEXT,
  normalized_correct_answer_json TEXT,
  is_correct INTEGER,
  weight REAL NOT NULL,
  first_answered_at TEXT,
  last_answered_at TEXT,
  change_count INTEGER NOT NULL DEFAULT 0,
  visit_count INTEGER NOT NULL DEFAULT 0,
  elapsed_ms INTEGER NOT NULL DEFAULT 0,
  duration_ms INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(attempt_id, passage_index, question_id)
);
CREATE TABLE IF NOT EXISTS writings (
  attempt_id TEXT NOT NULL,
  writing_submission_id TEXT NOT NULL,
  task_type TEXT,
  word_count INTEGER,
  started_at TEXT,
  submitted_at TEXT,
  duration_sec INTEGER,
  PRIMARY KEY(attempt_id, writing_submission_id)
);
"#;

/// 学生端 `score_info_json` 的我们关心的部分（判分结果，只读）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoreInfo {
    pub gradable: bool,
    /// 得分点数（学生端按 weight 加总后的 correct）。
    pub correct: f64,
    /// 满分点数（学生端按 weight 加总后的 total）。
    pub total: f64,
    pub percentage: Option<f64>,
}

pub fn parse_score_info(raw: &str) -> Option<ScoreInfo> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let gradable = value.get("gradable").and_then(|v| v.as_bool()).unwrap_or(false);
    let correct = value.get("correct").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let total = value
        .get("total")
        .and_then(|v| v.as_f64())
        .or_else(|| value.get("totalQuestions").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
    let percentage = value.get("percentage").and_then(|v| v.as_f64());
    Some(ScoreInfo { gradable, correct, total, percentage })
}

pub fn ratio(numerator: f64, denominator: f64) -> Option<f64> {
    if denominator > 0.0 { Some(numerator / denominator) } else { None }
}

/// 把学生端存的答案 JSON 转成可读文本：字符串原样，数组用 " / " 连接，空值显示「未作答」。
pub fn display_answer(raw: Option<&str>) -> String {
    let Some(raw) = raw else { return "（未作答）".to_string() };
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Null) => "（未作答）".to_string(),
        Ok(serde_json::Value::String(text)) => {
            if text.trim().is_empty() { "（未作答）".to_string() } else { text }
        }
        Ok(serde_json::Value::Array(items)) => {
            let parts: Vec<String> = items
                .iter()
                .map(|item| match item {
                    serde_json::Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .filter(|text| !text.trim().is_empty())
                .collect();
            if parts.is_empty() { "（未作答）".to_string() } else { parts.join(" / ") }
        }
        Ok(other) => other.to_string(),
        Err(_) => raw.to_string(),
    }
}

/// 与前端一致的 Part 标签：优先用学生端 category（P1/P2/P3），否则按序号给 P{n}。
pub fn passage_label(category: Option<&str>, passage_index: i64) -> String {
    match category.map(str::trim) {
        Some(text) if !text.is_empty() => text.to_string(),
        _ => format!("P{}", passage_index + 1),
    }
}
