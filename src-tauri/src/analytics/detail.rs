use super::records::query;
use crate::CommandResult;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

fn parsed(v: &Value) -> Value {
    v.as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null)
}
fn intervals(events: &[Value]) -> (Vec<Value>, bool) {
    let mut open: HashMap<i64, &Value> = HashMap::new();
    let mut out = Vec::new();
    let mut complete = !events.is_empty();
    for e in events {
        let Some(part) = e["passage_index"].as_i64() else {
            continue;
        };
        match e["event_type"].as_str().unwrap_or("") {
            "part_enter" | "app_focus" => {
                if open.insert(part, e).is_some() {
                    complete = false;
                }
            }
            "part_leave" | "app_blur" => {
                if let Some(start) = open.remove(&part) {
                    let elapsed = e["monotonic_ms"].as_f64().unwrap_or(0.0)
                        - start["monotonic_ms"].as_f64().unwrap_or(0.0);
                    if e["clock_id"] == start["clock_id"] && elapsed >= 0.0 {
                        out.push(json!({"passageIndex":part,"startedAt":start["wall_time"],"endedAt":e["wall_time"],"durationMs":elapsed,"clockId":e["clock_id"]}));
                    } else {
                        complete = false;
                    }
                } else {
                    complete = false;
                }
            }
            _ => {}
        }
    }
    (out, complete && open.is_empty())
}
pub fn get(conn: &Connection, root: &Path, attempt_id: &str) -> CommandResult<Value> {
    let candidate = query(
        conn,
        "SELECT * FROM attempts WHERE attempt_id=?1",
        &[&attempt_id],
    )?
    .into_iter()
    .next()
    .ok_or("ATTEMPT_NOT_FOUND: 考试记录不存在")?;
    let library = crate::library::repository::open_library_connection(root)?;
    let paper_id = candidate["paper_id"].as_str().unwrap_or("");
    let paper = if paper_id.is_empty() {
        Value::Null
    } else {
        crate::library::paper_archive::get(&library, paper_id)
            .unwrap_or_else(|e| json!({"paperId":paper_id,"archiveUnavailable":true,"reason":e}))
    };
    let events = query(
        conn,
        "SELECT * FROM answer_events WHERE attempt_id=?1 ORDER BY event_seq",
        &[&attempt_id],
    )?;
    let (activity, activity_complete) = intervals(&events);
    let mut passages = query(
        conn,
        "SELECT * FROM passages WHERE attempt_id=?1 ORDER BY passage_index",
        &[&attempt_id],
    )?;
    let overrides = query(
        conn,
        "SELECT * FROM score_overrides WHERE attempt_id=?1 ORDER BY created_at",
        &[&attempt_id],
    )?;
    let mut evidence_conflicts = Vec::new();
    for p in &mut passages {
        let index = p["passage_index"].as_i64().unwrap_or(0);
        let exam = p["exam_id"].as_str().unwrap_or("").to_string();
        let hash = p["runtime_sha256"].as_str().unwrap_or("").to_string();
        let content = crate::library::paper_archive::content(&library, &exam, &hash)?;
        p["contentUnavailable"] = json!(content.is_none());
        p["passage"] = content
            .as_ref()
            .map(|s| s["passage"].clone())
            .unwrap_or(Value::Null);
        p["highlights"] = parsed(&p["highlights_json"]);
        p["timerSnapshot"] = parsed(&p["timer_snapshot_json"]);
        p["activityIntervals"] = json!(activity
            .iter()
            .filter(|e| e["passageIndex"] == index)
            .collect::<Vec<_>>());
        let mut answers = query(
            conn,
            "SELECT * FROM answers WHERE attempt_id=?1 AND passage_index=?2 ORDER BY question_id",
            &[&attempt_id, &index],
        )?;
        let mut delta = 0.0;
        for a in &mut answers {
            let slot = a["response_slot_id"]
                .as_str()
                .or_else(|| a["question_id"].as_str())
                .unwrap_or("")
                .to_string();
            let raw = a["score"].as_f64().or_else(|| {
                a["is_correct"].as_i64().map(|n| {
                    if n == 1 {
                        a["weight"].as_f64().unwrap_or(0.0)
                    } else {
                        0.0
                    }
                })
            });
            let active = overrides.iter().find(|o| {
                o["modality"] == "reading"
                    && o["passage_index"] == index
                    && o["response_slot_id"] == slot
                    && o["revoked_at"].is_null()
                    && o["original_answer_json"] == a["user_answer_json"]
                    && o["original_is_correct"] == a["is_correct"]
                    && o["original_score"].as_f64() == raw
            });
            let effective = active.and_then(|o| o["new_score"].as_f64()).or(raw);
            if active.is_none()
                && overrides.iter().any(|o| {
                    o["modality"] == "reading"
                        && o["passage_index"] == index
                        && o["response_slot_id"] == slot
                        && o["revoked_at"].is_null()
                })
            {
                evidence_conflicts.push(json!({"code":"OVERRIDE_EVIDENCE_CONFLICT","modality":"reading","passageIndex":index,"responseSlotId":slot,"message":"原始作答已变化，旧改判未计入成绩，请重新核对"}));
            }
            delta += effective.unwrap_or_default() - raw.unwrap_or_default();
            a["originalScore"] = json!(raw);
            a["effectiveScore"] = json!(effective);
            a["originalIsCorrect"] = a["is_correct"].clone();
            a["effectiveIsCorrect"] = active
                .map(|o| o["new_is_correct"].clone())
                .unwrap_or_else(|| a["is_correct"].clone());
            a["context"] = crate::library::paper_archive::question(&library, &exam, &hash, &slot)
                .unwrap_or_else(|e| json!({"contentUnavailable":true,"reason":e}));
            a["answerHistory"] = json!(events
                .iter()
                .filter(|e| e["passage_index"] == index
                    && e["question_id"] == slot
                    && e["event_type"] == "answer_changed")
                .collect::<Vec<_>>());
            a["userAnswer"] = parsed(&a["user_answer_json"]);
            a["correctAnswer"] = parsed(&a["correct_answer_json"]);
        }
        p["effectiveScore"] = json!(p["correct"].as_f64().unwrap_or_default() + delta);
        p["answers"] = json!(answers);
    }
    let mut writings = query(
        conn,
        "SELECT * FROM writings WHERE attempt_id=?1 ORDER BY task_type",
        &[&attempt_id],
    )?;
    for w in &mut writings {
        let wid = w["writing_submission_id"].as_str().unwrap();
        let mut grades=query(conn,"SELECT * FROM writing_grades WHERE attempt_id=?1 AND writing_submission_id=?2 ORDER BY created_at DESC",&[&attempt_id,&wid])?;
        for g in &mut grades {
            g["annotations"] = json!(query(
                conn,
                "SELECT * FROM writing_annotations WHERE grade_id=?1 ORDER BY start_offset",
                &[&g["grade_id"].as_str()]
            )?);
        }
        w["grades"] = json!(grades);
    }
    let listening=query(conn,"SELECT l.*,l.score_correct+COALESCE((SELECT SUM(o.new_score-o.original_score) FROM score_overrides o JOIN listening_answers a ON a.attempt_id=o.attempt_id AND a.slot_id=o.response_slot_id WHERE o.attempt_id=l.attempt_id AND o.modality='listening' AND o.revoked_at IS NULL AND o.original_answer_json IS a.user_answer_json AND o.original_is_correct IS a.is_correct AND o.original_score IS a.is_correct*a.weight),0) AS effective_score FROM listening_attempts l WHERE attempt_id=?1",&[&attempt_id])?;
    let listening_answers=query(conn,"SELECT a.*,COALESCE(o.new_score,a.is_correct*a.weight) AS effective_score,COALESCE(o.new_is_correct,a.is_correct) AS effective_is_correct FROM listening_answers a LEFT JOIN score_overrides o ON o.attempt_id=a.attempt_id AND o.modality='listening' AND o.response_slot_id=a.slot_id AND o.revoked_at IS NULL AND o.original_answer_json IS a.user_answer_json AND o.original_is_correct IS a.is_correct AND o.original_score IS a.is_correct*a.weight WHERE a.attempt_id=?1 ORDER BY a.slot_id",&[&attempt_id])?;
    let mut anomalies=query(conn,"SELECT * FROM import_snapshots WHERE attempt_id=?1 AND status='failed' ORDER BY imported_at DESC",&[&attempt_id])?;
    anomalies.extend(evidence_conflicts);
    anomalies.extend(query(conn,"SELECT 'OVERRIDE_EVIDENCE_CONFLICT' AS code,'listening' AS modality,o.response_slot_id,'原始作答已变化，旧改判未计入成绩，请重新核对' AS message FROM score_overrides o LEFT JOIN listening_answers a ON a.attempt_id=o.attempt_id AND a.slot_id=o.response_slot_id WHERE o.attempt_id=?1 AND o.modality='listening' AND o.revoked_at IS NULL AND (a.slot_id IS NULL OR o.original_answer_json IS NOT a.user_answer_json OR o.original_is_correct IS NOT a.is_correct OR o.original_score IS NOT a.is_correct*a.weight)",&[&attempt_id])?);
    let audit=query(conn,"SELECT * FROM audit_log WHERE target_id=?1 OR target_id IN (SELECT grade_id FROM writing_grades WHERE attempt_id=?1) ORDER BY at DESC LIMIT 100",&[&attempt_id])?;
    Ok(
        json!({"n":1,"candidate":candidate,"paper":paper,"passages":passages,"answerEvents":events,"activityComplete":activity_complete,"writings":writings,"listening":listening,"listeningAnswers":listening_answers,"overrides":overrides,"anomalies":anomalies,"audit":audit,"disclaimer":super::DISCLAIMER,"writingScoreLabel":"写作人工评分（0–9）"}),
    )
}
