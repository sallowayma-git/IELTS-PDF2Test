use super::{archive, review, snapshot_v2::rows, DISCLAIMER};
use crate::CommandResult;
use rusqlite::Connection;
use serde_json::{json, Value};

pub fn attempt_detail(conn: &Connection, id: &str) -> CommandResult<Value> {
    let attempt = rows(conn, "SELECT * FROM attempts WHERE attempt_id=?", &[&id])?
        .into_iter()
        .next()
        .ok_or("ATTEMPT_NOT_FOUND:考试记录不存在")?;
    let identity = rows(
        conn,
        "SELECT * FROM candidate_aliases WHERE candidate_id=? AND candidate_name=?",
        &[
            &attempt["candidate_id"].as_str(),
            &attempt["candidate_name"].as_str(),
        ],
    )?;
    let paper_id = attempt["paper_id"].as_str().unwrap_or_default();
    let paper = archive::get(conn, paper_id).ok();
    let events = rows(
        conn,
        "SELECT * FROM answer_events WHERE attempt_id=? ORDER BY event_seq",
        &[&id],
    )?;
    let intervals = activity_intervals(&events);
    let mut passages = rows(
        conn,
        "SELECT * FROM passages WHERE attempt_id=? ORDER BY passage_index",
        &[&id],
    )?;
    let mut anomalies = Vec::new();
    for p in &mut passages {
        let index = p["passage_index"].as_i64().unwrap_or_default();
        let exam = p["exam_id"].as_str().unwrap_or_default().to_string();
        let hash = p["runtime_sha256"].as_str().unwrap_or_default().to_string();
        match archive::content(conn, &exam, &hash) {
            Ok(source) => {
                p["passage"] = source["passage"].clone();
                p["runtimeSource"] = source;
            }
            Err(e) => {
                anomalies
                    .push(json!({"passageIndex":index,"code":"CONTENT_UNAVAILABLE","message":e}));
                p["contentUnavailable"] = json!(true);
            }
        }
        let mut answers=rows(conn,"SELECT r.*,o.override_id,o.new_is_correct,o.new_score FROM answers r LEFT JOIN score_overrides o ON o.attempt_id=r.attempt_id AND o.modality='reading' AND o.passage_index=r.passage_index AND o.response_slot_id=r.response_slot_id AND o.revoked_at IS NULL WHERE r.attempt_id=? AND r.passage_index=? ORDER BY r.question_id",&[&id,&index])?;
        for answer in &mut answers {
            let slot = answer["response_slot_id"].as_str().unwrap_or_default();
            answer["context"] = archive::question(conn, &exam, &hash, slot)
                .unwrap_or(json!({"contentUnavailable":true}));
            answer["events"] =
                json!(events
                    .iter()
                    .filter(|e| e["passage_index"] == index
                        && e["question_id"] == answer["question_id"])
                    .collect::<Vec<_>>());
            answer["effectiveIsCorrect"] = answer["new_is_correct"]
                .as_i64()
                .map(|n| json!(n != 0))
                .unwrap_or_else(|| {
                    answer["is_correct"]
                        .as_i64()
                        .map(|n| json!(n != 0))
                        .unwrap_or(Value::Null)
                });
            answer["effectiveScore"] = if answer["override_id"].is_null() {
                answer["score"].clone()
            } else {
                answer["new_score"].clone()
            };
        }
        let delta: f64 = answers
            .iter()
            .filter(|a| !a["override_id"].is_null())
            .map(|a| a["new_score"].as_f64().unwrap_or(0.0) - a["score"].as_f64().unwrap_or(0.0))
            .sum();
        p["effectiveScore"] = json!(p["correct"].as_f64().unwrap_or(0.0) + delta);
        p["answers"] = json!(answers);
        p["activityIntervals"] = json!(intervals
            .iter()
            .filter(|e| e["passageIndex"] == index)
            .collect::<Vec<_>>());
    }
    let mut writings = rows(
        conn,
        "SELECT * FROM writings WHERE attempt_id=? ORDER BY task_type",
        &[&id],
    )?;
    for writing in &mut writings {
        let writing_id = writing["writing_submission_id"]
            .as_str()
            .unwrap_or_default();
        let mut grades=rows(conn,"SELECT * FROM writing_grades WHERE attempt_id=? AND writing_submission_id=? ORDER BY created_at",&[&id,&writing_id])?;
        for grade in &mut grades {
            grade["annotations"] = json!(rows(
                conn,
                "SELECT * FROM writing_annotations WHERE grade_id=? ORDER BY start_offset",
                &[&grade["grade_id"].as_str()]
            )?);
        }
        writing["grades"] = json!(grades);
    }
    let mut listening = rows(
        conn,
        "SELECT * FROM listening_attempts WHERE attempt_id=?",
        &[&id],
    )?;
    for l in &mut listening {
        let mut answers=rows(conn,"SELECT r.*,o.override_id,o.new_is_correct,o.new_score,COALESCE(o.new_score,r.score) AS effective_score,COALESCE(o.new_is_correct,r.is_correct) AS effective_is_correct FROM listening_answers r LEFT JOIN score_overrides o ON o.attempt_id=r.attempt_id AND o.modality='listening' AND o.passage_index=-1 AND o.response_slot_id=r.slot_id AND o.revoked_at IS NULL WHERE r.attempt_id=? ORDER BY r.slot_id",&[&id])?;
        for a in &mut answers {
            a["context"] = archive::question(
                conn,
                l["exam_id"].as_str().unwrap_or_default(),
                l["runtime_sha256"].as_str().unwrap_or_default(),
                a["slot_id"].as_str().unwrap_or_default(),
            )
            .unwrap_or(json!({"contentUnavailable":true}));
        }
        l["effectiveScore"] = json!(answers
            .iter()
            .filter_map(|a| a["effective_score"].as_f64())
            .sum::<f64>());
        l["answers"] = json!(answers);
    }
    let audit = review::list_audit(
        conn,
        &json!({"targetType":"attempt","targetId":id,"limit":50}),
    )?;
    let imports=rows(conn,"SELECT file,error,imported_at FROM import_snapshots WHERE attempt_id=? AND status='failed'",&[&id])?;
    Ok(
        json!({"n":1,"attempt":attempt,"identityAliases":identity,"paper":paper,"passages":passages,"events":events,"writings":writings,"listening":listening,"overrides":review::list_overrides(conn,id)?,"anomalies":anomalies,"importExceptions":imports,"audit":audit,"disclaimer":DISCLAIMER}),
    )
}

/// Performance timestamps reset on app restart; never bridge separate clocks or invent missing intervals.
pub fn activity_intervals(events: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut active: Option<&Value> = None;
    let mut current_part: Option<i64> = None;
    for e in events {
        let kind = e["event_type"].as_str().unwrap_or_default();
        if matches!(kind, "part_leave" | "app_blur" | "part_enter") {
            if let Some(start) = active.take() {
                let from = start["monotonic_ms"].as_f64().unwrap_or_default();
                let to = e["monotonic_ms"].as_f64().unwrap_or_default();
                let wall = |v: &Value| {
                    v["wall_time"]
                        .as_str()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                };
                let gap = wall(e)
                    .zip(wall(start))
                    .map(|(b, a)| (b - a).num_milliseconds() as f64);
                if to >= from && gap.is_some_and(|g| g >= 0.0 && (g - (to - from)).abs() < 5000.0) {
                    out.push(json!({"passageIndex":current_part,"enteredAt":start["wall_time"],"leftAt":e["wall_time"],"durationMs":to-from}));
                }
            }
        }
        if kind == "part_enter" {
            current_part = e["passage_index"].as_i64();
            active = Some(e);
        }
        if kind == "part_leave" {
            current_part = None;
        }
        if kind == "app_focus" && current_part.is_some() && active.is_none() {
            active = Some(e);
        }
    }
    out
}
