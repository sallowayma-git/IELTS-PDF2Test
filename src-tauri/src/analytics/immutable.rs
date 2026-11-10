//! A later export may fill missing evidence, never replace submitted facts.
use super::records::query;
use crate::CommandResult;
use rusqlite::Connection;
use serde_json::Value;

const FACTS: &[&str] = &[
    "SELECT terminal_id,batch_id,candidate_id,candidate_name,checkin_at,started_at,submitted_at,status FROM attempts WHERE attempt_id=?1",
    "SELECT suite_attempt_id,status,started_at,completed_at,submitted_at,total_passages FROM suites WHERE attempt_id=?1 ORDER BY suite_attempt_id",
    "SELECT passage_index,suite_attempt_id,asset_id,exam_id,title,category,started_at,ended_at,duration_sec,gradable,correct,total,percentage FROM passages WHERE attempt_id=?1 ORDER BY passage_index",
    "SELECT passage_index,question_id,display_label,question_kind,control,match_mode,user_answer_json,correct_answer_json,normalized_user_answer_json,normalized_correct_answer_json,is_correct,weight,response_slot_id,max_score,score,first_answered_at,last_answered_at,change_count,visit_count,elapsed_ms,duration_ms FROM answers WHERE attempt_id=?1 ORDER BY passage_index,question_id",
    "SELECT writing_submission_id,task_type,word_count,started_at,submitted_at,duration_sec FROM writings WHERE attempt_id=?1 ORDER BY writing_submission_id",
];

// Keys identify an existing evidence row. Non-null evidence is immutable;
// additional events/listening evidence and previously unavailable fields may arrive later.
const EVIDENCE: &[(&str, &[&str])] = &[
    ("SELECT attempt_id,session_id,student_uuid,abandon_reason,app_version,scorer_version FROM attempts WHERE attempt_id=?1", &["attempt_id"]),
    ("SELECT passage_index,raw_submission_json,highlights_json,note_text,timer_snapshot_json,runtime_sha256,script_sha256 FROM passages WHERE attempt_id=?1", &["passage_index"]),
    ("SELECT writing_submission_id,prompt_text,content,metadata_json FROM writings WHERE attempt_id=?1", &["writing_submission_id"]),
    ("SELECT * FROM answer_events WHERE attempt_id=?1", &["event_seq"]),
    ("SELECT * FROM listening_attempts WHERE attempt_id=?1", &["attempt_id"]),
    ("SELECT * FROM listening_answers WHERE attempt_id=?1", &["slot_id"]),
];

pub struct ImportedFacts {
    facts: Vec<Vec<Value>>,
    evidence: Vec<Vec<Value>>,
}

fn normalize(mut rows: Vec<Value>) -> Vec<Value> {
    for row in &mut rows {
        for (key, value) in row.as_object_mut().unwrap() {
            if key.ends_with("_json") {
                if let Some(parsed) = value.as_str().and_then(|s| serde_json::from_str(s).ok()) {
                    *value = parsed;
                }
            }
        }
    }
    rows
}

impl ImportedFacts {
    pub fn read(conn: &Connection, attempt: &str) -> CommandResult<Option<Self>> {
        let facts: Vec<Vec<Value>> = FACTS
            .iter()
            .map(|sql| query(conn, sql, &[&attempt]).map(normalize))
            .collect::<Result<_, _>>()?;
        if facts[0].is_empty() {
            return Ok(None);
        }
        let evidence = EVIDENCE
            .iter()
            .map(|(sql, _)| query(conn, sql, &[&attempt]).map(normalize))
            .collect::<Result<_, _>>()?;
        Ok(Some(Self { facts, evidence }))
    }

    pub fn verify(&self, conn: &Connection, attempt: &str) -> CommandResult<()> {
        let after =
            Self::read(conn, attempt)?.ok_or("SNAPSHOT_FACT_CONFLICT:已导入的考试记录缺失")?;
        let conflict = self.facts != after.facts
            || self.evidence.iter().zip(&after.evidence).zip(EVIDENCE).any(
                |((before, after), (_, keys))| {
                    before.iter().any(|old| {
                        let Some(new) = after
                            .iter()
                            .find(|new| keys.iter().all(|key| old[*key] == new[*key]))
                        else {
                            return true;
                        };
                        old.as_object()
                            .unwrap()
                            .iter()
                            .any(|(key, value)| !value.is_null() && value != &new[key])
                    })
                },
            );
        if conflict {
            return Err(
                "SNAPSHOT_FACT_CONFLICT:已交卷的作答、判分或正文与原始记录不一致，已保留原始记录"
                    .into(),
            );
        }
        Ok(())
    }
}
