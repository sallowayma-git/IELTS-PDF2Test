//! Read extensions from a verified LOCAL copy, never a live NAS shard.
use crate::CommandResult;
use rusqlite::{params, types::ValueRef, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn rows(
    conn: &Connection,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> CommandResult<Vec<Value>> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("DATA_QUERY:数据查询失败：{e}"))?;
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mapped = stmt
        .query_map(args, |row| {
            let mut obj = serde_json::Map::new();
            for (i, name) in names.iter().enumerate() {
                let v = match row.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(v) => json!(v),
                    ValueRef::Real(v) => json!(v),
                    ValueRef::Text(v) => Value::String(String::from_utf8_lossy(v).into_owned()),
                    ValueRef::Blob(_) => Value::Null,
                };
                obj.insert(name.clone(), v);
            }
            Ok(Value::Object(obj))
        })
        .map_err(|e| e.to_string())?;
    mapped.map(|r| r.map_err(|e| e.to_string())).collect()
}

#[derive(Default)]
pub struct Extensions {
    tables: HashMap<String, Vec<Value>>,
    locks: HashMap<String, Value>,
}
impl Extensions {
    pub fn read(conn: &Connection) -> CommandResult<Self> {
        let mut out = Self::default();
        for table in [
            "exam_attempts",
            "reading_suite_attempts",
            "reading_passages",
            "reading_answers",
            "writing_submissions",
            "answer_events",
            "listening_attempts",
            "listening_answers",
        ] {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    [table],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if exists {
                out.tables.insert(
                    table.into(),
                    rows(conn, &format!("SELECT * FROM {table}"), &[])?,
                );
            }
        }
        for suite in out
            .tables
            .get("reading_suite_attempts")
            .into_iter()
            .flatten()
        {
            let aggregate = suite["aggregate_json"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null);
            let seq = if aggregate.is_array() {
                aggregate
            } else {
                aggregate["lockedSequence"].clone()
            };
            out.locks.insert(
                suite["suite_attempt_id"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                seq,
            );
        }
        Ok(out)
    }
    pub fn import(&self, conn: &Connection, attempt: &str) -> CommandResult<()> {
        let suites: Vec<&str> = self
            .tables
            .get("reading_suite_attempts")
            .into_iter()
            .flatten()
            .filter(|r| r["attempt_id"] == attempt)
            .filter_map(|r| r["suite_attempt_id"].as_str())
            .collect();
        for row in self
            .tables
            .get("exam_attempts")
            .into_iter()
            .flatten()
            .filter(|r| r["attempt_id"] == attempt)
        {
            conn.execute("UPDATE attempts SET session_id=?2,student_uuid=?3,abandon_reason=?4 WHERE attempt_id=?1",params![attempt,text(row,"session_id"),text(row,"student_uuid"),text(row,"abandon_reason")]).map_err(|e|e.to_string())?;
        }
        for row in self
            .tables
            .get("reading_passages")
            .into_iter()
            .flatten()
            .filter(|r| suites.contains(&r["suite_attempt_id"].as_str().unwrap_or_default()))
        {
            let raw = text(row, "raw_submission_json");
            let submission = raw
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null);
            let index = row["passage_index"].as_i64().unwrap_or_default();
            let sequence = self
                .locks
                .get(row["suite_attempt_id"].as_str().unwrap_or_default());
            let locked = sequence.and_then(Value::as_array).and_then(|a| {
                a.iter()
                    .find(|p| p["examId"] == row["exam_id"] || p["assetId"] == row["asset_id"])
            });
            let hash = |name: &str| {
                locked
                    .and_then(|p| {
                        p.get(name)
                            .or_else(|| p.pointer(&format!("/pin/checksums/{name}")))
                    })
                    .and_then(Value::as_str)
            };
            conn.execute("UPDATE passages SET raw_submission_json=?3,highlights_json=?4,note_text=?5,timer_snapshot_json=?6,runtime_sha256=?7,script_sha256=?8 WHERE attempt_id=?1 AND passage_index=?2",
              params![attempt,index,raw,submission["highlights"].to_string(),submission["noteText"].as_str(),submission["timerSnapshot"].to_string(),hash("runtimeSha256"),hash("scriptSha256")]).map_err(|e|e.to_string())?;
        }
        for row in self
            .tables
            .get("reading_answers")
            .into_iter()
            .flatten()
            .filter(|r| suites.contains(&r["suite_attempt_id"].as_str().unwrap_or_default()))
        {
            let slot = text(row, "response_slot_id").or_else(|| text(row, "question_id"));
            let max = row["max_score"]
                .as_f64()
                .or(row["weight"].as_f64())
                .unwrap_or(0.0);
            let score = row["score"]
                .as_f64()
                .or_else(|| row["is_correct"].as_i64().map(|c| c as f64 * max));
            conn.execute("UPDATE answers SET response_slot_id=?4,max_score=?5,score=?6 WHERE attempt_id=?1 AND passage_index=?2 AND question_id=?3",
              params![attempt,row["passage_index"].as_i64(),text(row,"question_id"),slot,max,score]).map_err(|e|e.to_string())?;
        }
        for row in self
            .tables
            .get("writing_submissions")
            .into_iter()
            .flatten()
            .filter(|r| r["attempt_id"] == attempt)
        {
            conn.execute("UPDATE writings SET prompt_text=?3,content=?4,metadata_json=?5 WHERE attempt_id=?1 AND writing_submission_id=?2",
              params![attempt,text(row,"writing_submission_id"),text(row,"prompt_text"),text(row,"content"),text(row,"metadata_json")]).map_err(|e|e.to_string())?;
        }
        for table in ["answer_events", "listening_attempts", "listening_answers"] {
            let columns: Vec<String> = rows(conn, &format!("PRAGMA table_info({table})"), &[])?
                .iter()
                .filter_map(|v| v["name"].as_str().map(str::to_string))
                .collect();
            for row in self
                .tables
                .get(table)
                .into_iter()
                .flatten()
                .filter(|r| r["attempt_id"] == attempt)
            {
                let available: Vec<&str> = columns
                    .iter()
                    .map(String::as_str)
                    .filter(|c| row.get(*c).is_some())
                    .collect();
                let values: Vec<rusqlite::types::Value> =
                    available.iter().map(|c| sql_value(&row[*c])).collect();
                let placeholders = vec!["?"; available.len()].join(",");
                conn.execute(
                    &format!(
                        "INSERT INTO {table}({}) VALUES({placeholders}) ON CONFLICT DO NOTHING",
                        available.join(",")
                    ),
                    rusqlite::params_from_iter(values),
                )
                .map_err(|e| format!("SNAPSHOT_IMPORT:导入 {table} 失败：{e}"))?;
            }
        }
        super::schema_v2::refresh_counted(conn)
    }
    pub fn validate_writings(&self, conn: &Connection, attempt: &str) -> CommandResult<()> {
        for row in self
            .tables
            .get("writing_submissions")
            .into_iter()
            .flatten()
            .filter(|r| r["attempt_id"] == attempt)
        {
            let old: Option<String> = conn
                .query_row(
                    "SELECT content FROM writings WHERE attempt_id=?1 AND writing_submission_id=?2",
                    params![attempt, text(row, "writing_submission_id")],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .flatten();
            if old
                .as_deref()
                .is_some_and(|s| Some(s) != text(row, "content"))
            {
                return Err("SNAPSHOT_FACT_CONFLICT:已交卷作文与原始记录冲突".into());
            }
        }
        let suites: Vec<&str> = self
            .tables
            .get("reading_suite_attempts")
            .into_iter()
            .flatten()
            .filter(|r| r["attempt_id"] == attempt)
            .filter_map(|r| r["suite_attempt_id"].as_str())
            .collect();
        for row in self
            .tables
            .get("reading_passages")
            .into_iter()
            .flatten()
            .filter(|r| suites.contains(&r["suite_attempt_id"].as_str().unwrap_or_default()))
        {
            let previous:Option<String>=conn.query_row("SELECT raw_submission_json FROM passages WHERE attempt_id=?1 AND passage_index=?2",params![attempt,row["passage_index"].as_i64()],|r|r.get(0)).optional().map_err(|e|e.to_string())?.flatten();
            if let Some(old) = previous {
                let new = text(row, "raw_submission_json").unwrap_or("null");
                if old != new
                    && serde_json::from_str::<Value>(&old).ok()
                        != serde_json::from_str::<Value>(new).ok()
                {
                    return Err("SNAPSHOT_FACT_CONFLICT:已交卷原始阅读记录发生冲突".into());
                }
            }
        }
        Ok(())
    }
}
fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v[k].as_str()
}
fn sql_value(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as S;
    match v {
        Value::Null => S::Null,
        Value::Bool(b) => S::Integer(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(S::Integer)
            .unwrap_or_else(|| S::Real(n.as_f64().unwrap_or_default())),
        Value::String(s) => S::Text(s.clone()),
        _ => S::Text(v.to_string()),
    }
}
