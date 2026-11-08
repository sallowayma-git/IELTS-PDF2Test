use super::snapshot_v2::rows;
use crate::CommandResult;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

pub fn statistics(conn: &Connection, paper_id: &str, original: bool) -> CommandResult<Value> {
    let samples=rows(conn,"SELECT a.attempt_id,a.candidate_id,a.candidate_name,r.*,l.exam_id,o.new_is_correct,o.new_score,o.override_id FROM listening_answers r JOIN attempts a USING(attempt_id) JOIN listening_attempts l USING(attempt_id) LEFT JOIN score_overrides o ON o.attempt_id=r.attempt_id AND o.modality='listening' AND o.passage_index=-1 AND o.response_slot_id=r.slot_id AND o.revoked_at IS NULL WHERE a.paper_id=? AND a.status='submitted' AND a.counted=1 ORDER BY a.attempt_id,r.slot_id",&[&paper_id])?;
    let mut students: BTreeMap<String, Value> = BTreeMap::new();
    let mut slots: BTreeMap<(String, String), (HashSet<String>, f64, f64)> = BTreeMap::new();
    let mut kinds: BTreeMap<String, (HashSet<String>, f64, f64)> = BTreeMap::new();
    for r in samples {
        let id = r["attempt_id"].as_str().unwrap_or_default().to_string();
        let score = if !original && !r["override_id"].is_null() {
            r["new_score"].as_f64()
        } else {
            r["score"].as_f64()
        };
        let Some(score) = score else { continue };
        let max = r["max_score"].as_f64().unwrap_or_default();
        let student=students.entry(id.clone()).or_insert_with(||json!({"attemptId":id,"candidateId":r["candidate_id"],"candidateName":r["candidate_name"],"score":0.0,"maxScore":0.0,"slots":[]}));
        student["score"] = json!(student["score"].as_f64().unwrap_or_default() + score);
        student["maxScore"] = json!(student["maxScore"].as_f64().unwrap_or_default() + max);
        student["slots"].as_array_mut().unwrap().push(json!({"examId":r["exam_id"],"slotId":r["slot_id"],"score":score,"maxScore":max,"originalScore":r["score"]}));
        let key = (
            r["exam_id"].as_str().unwrap_or_default().to_string(),
            r["slot_id"].as_str().unwrap_or_default().to_string(),
        );
        let s = slots.entry(key).or_default();
        s.0.insert(id.clone());
        s.1 += score;
        s.2 += max;
        let k = kinds
            .entry(r["question_kind"].as_str().unwrap_or("unknown").to_string())
            .or_default();
        k.0.insert(id);
        k.1 += score;
        k.2 += max;
    }
    let mut questions:Vec<Value>=slots.into_iter().map(|((exam,slot),(ids,score,max))|json!({"examId":exam,"slotId":slot,"n":ids.len(),"score":score,"maxScore":max,"accuracy":super::ratio(score,max),"conclusionAvailable":ids.len()>=super::HARD_CONCLUSION_MIN_N})).collect();
    questions.sort_by(|a, b| {
        a["accuracy"]
            .as_f64()
            .partial_cmp(&b["accuracy"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let hard: Vec<Value> = questions
        .iter()
        .filter(|q| q["conclusionAvailable"] == true)
        .take(10)
        .cloned()
        .collect();
    let kinds:Vec<Value>=kinds.into_iter().map(|(kind,(ids,score,max))|json!({"kind":kind,"n":ids.len(),"score":score,"maxScore":max,"accuracy":super::ratio(score,max)})).collect();
    let students: Vec<Value> = students.into_values().collect();
    Ok(
        json!({"paperId":paper_id,"n":students.len(),"original":original,"students":students,"questions":questions,"kinds":kinds,"hardTop10":hard,"disclaimer":super::DISCLAIMER}),
    )
}
