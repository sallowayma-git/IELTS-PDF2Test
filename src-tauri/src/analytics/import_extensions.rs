use super::records::{query, sql_value};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::collections::HashMap;

pub struct Extensions {
    tables: HashMap<String, Vec<Value>>,
}

impl Extensions {
    pub fn read(conn: &Connection, version: u64) -> Result<Self, String> {
        if version == 2 {
            let stored: String = conn
                .query_row(
                    "SELECT value_json FROM exam_kv WHERE key='db_schema_version'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if serde_json::from_str::<u64>(&stored).ok() != Some(2) {
                return Err("快照库版本与 meta 不一致".into());
            }
            for sql in [
                "SELECT response_slot_id,max_score,score FROM reading_answers LIMIT 0",
                "SELECT session_id,student_uuid,abandon_reason FROM exam_attempts LIMIT 0",
                "SELECT attempt_id,event_seq,client_event_id,event_type,clock_id,wall_time,monotonic_ms FROM answer_events LIMIT 0",
                "SELECT attempt_id,exam_id,payload_hash FROM listening_attempts LIMIT 0",
                "SELECT attempt_id,slot_id,is_correct,weight FROM listening_answers LIMIT 0",
            ] { conn.prepare(sql).map_err(|e| format!("V2 快照结构缺失：{e}"))?; }
        }
        let mut tables = HashMap::new();
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
                tables.insert(
                    table.to_string(),
                    query(conn, &format!("SELECT * FROM {table}"), &[])?,
                );
            }
        }
        Ok(Self { tables })
    }

    fn rows(&self, table: &str) -> &[Value] {
        self.tables.get(table).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn apply(&self, conn: &Connection, id: &str) -> Result<(), String> {
        if let Some(a) = self
            .rows("exam_attempts")
            .iter()
            .find(|r| r["attempt_id"] == id)
        {
            conn.execute("UPDATE attempts SET session_id=?2,student_uuid=?3,abandon_reason=?4 WHERE attempt_id=?1",
                params![id,a["session_id"].as_str(),a["student_uuid"].as_str(),a["abandon_reason"].as_str()]).map_err(|e| e.to_string())?;
        }
        let suites: Vec<&Value> = self
            .rows("reading_suite_attempts")
            .iter()
            .filter(|r| r["attempt_id"] == id)
            .collect();
        for row in self.rows("reading_passages") {
            let Some(suite) = suites
                .iter()
                .find(|s| s["suite_attempt_id"] == row["suite_attempt_id"])
            else {
                continue;
            };
            let raw: Value =
                serde_json::from_str(row["raw_submission_json"].as_str().unwrap_or("{}"))
                    .map_err(|e| format!("原始作答 JSON 无效：{e}"))?;
            let aggregate: Value =
                serde_json::from_str(suite["aggregate_json"].as_str().unwrap_or("null"))
                    .unwrap_or(Value::Null);
            let sequence = aggregate
                .as_array()
                .or_else(|| aggregate["lockedSequence"].as_array());
            let pin = sequence.and_then(|s| {
                s.iter()
                    .find(|p| p["index"].as_i64().map(|n| n + 1) == row["passage_index"].as_i64())
            });
            let hash = pin
                .and_then(|p| p["runtimeSha256"].as_str())
                .or_else(|| row["source_hash"].as_str());
            let script = pin.and_then(|p| p["scriptSha256"].as_str());
            conn.execute("UPDATE passages SET highlights_json=?3,note_text=?4,timer_snapshot_json=?5,runtime_sha256=?6,script_sha256=?7 WHERE attempt_id=?1 AND passage_index=?2",
                params![id,row["passage_index"].as_i64(),raw.get("highlights").map(Value::to_string),raw["noteText"].as_str(),raw.get("timerSnapshot").map(Value::to_string),hash,script]).map_err(|e| e.to_string())?;
        }
        for row in self.rows("reading_answers") {
            if !suites
                .iter()
                .any(|s| s["suite_attempt_id"] == row["suite_attempt_id"])
            {
                continue;
            }
            let slot = row["response_slot_id"]
                .as_str()
                .or_else(|| row["question_id"].as_str());
            let max = row["max_score"]
                .as_f64()
                .or_else(|| row["weight"].as_f64())
                .unwrap_or(0.0);
            let score = row["score"].as_f64().or_else(|| {
                row["is_correct"]
                    .as_i64()
                    .map(|n| if n == 1 { max } else { 0.0 })
            });
            if !max.is_finite()
                || max < 0.0
                || score
                    .map(|n| !n.is_finite() || n < 0.0 || n > max)
                    .unwrap_or(false)
            {
                return Err("作答得分超出满分范围".into());
            }
            conn.execute("UPDATE answers SET response_slot_id=?4,max_score=?5,score=?6 WHERE attempt_id=?1 AND passage_index=?2 AND question_id=?3",
                params![id,row["passage_index"].as_i64(),row["question_id"].as_str(),slot,max,score]).map_err(|e| e.to_string())?;
        }
        for row in self
            .rows("writing_submissions")
            .iter()
            .filter(|r| r["attempt_id"] == id)
        {
            conn.execute("UPDATE writings SET prompt_text=?3,content=?4,metadata_json=?5 WHERE attempt_id=?1 AND writing_submission_id=?2",
                params![id,row["writing_submission_id"].as_str(),row["prompt_text"].as_str(),row["content"].as_str(),row["metadata_json"].as_str()]).map_err(|e| e.to_string())?;
        }
        for (table,columns) in [
            ("answer_events","attempt_id,event_seq,client_event_id,event_type,passage_index,question_id,old_answer_json,new_answer_json,wall_time,monotonic_ms,clock_id"),
            ("listening_attempts","attempt_id,exam_id,source_revision,runtime_sha256,script_sha256,started_at,submitted_at,score_correct,score_total,playback_snapshot_json,payload_hash"),
            ("listening_answers","attempt_id,slot_id,question_kind,part_id,user_answer_json,correct_answer_json,is_correct,weight"),
        ] {
            conn.execute(&format!("DELETE FROM {table} WHERE attempt_id=?1"),[id]).map_err(|e| e.to_string())?;
            let cols: Vec<&str> = columns.split(',').collect();
            let marks=vec!["?";cols.len()].join(",");
            for row in self.rows(table).iter().filter(|r| r["attempt_id"] == id) {
                let vals: Vec<_> = cols.iter().map(|c| sql_value(&row[*c])).collect();
                conn.execute(&format!("INSERT INTO {table}({columns}) VALUES({marks})"),rusqlite::params_from_iter(vals)).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}
