use super::records::query;
use crate::CommandResult;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn listening(
    conn: &Connection,
    batch: &str,
    paper: &str,
    original: bool,
) -> CommandResult<Value> {
    let rows=query(conn,"SELECT a.attempt_id,a.candidate_name,l.exam_id,l.runtime_sha256,b.slot_id,b.question_kind,b.part_id,b.weight,CASE WHEN ?3 THEN b.is_correct*b.weight ELSE COALESCE(o.new_score,b.is_correct*b.weight) END AS score,CASE WHEN ?3 THEN b.is_correct ELSE COALESCE(o.new_is_correct,b.is_correct) END AS is_correct FROM attempts a JOIN listening_attempts l USING(attempt_id) JOIN listening_answers b ON b.attempt_id=a.attempt_id LEFT JOIN score_overrides o ON o.attempt_id=b.attempt_id AND o.modality='listening' AND o.response_slot_id=b.slot_id AND o.revoked_at IS NULL AND o.original_answer_json IS b.user_answer_json AND o.original_is_correct IS b.is_correct AND o.original_score IS b.is_correct*b.weight WHERE a.batch_id=?1 AND a.paper_id=?2 AND a.status='submitted' AND a.counted=1 AND l.score_total>0 ORDER BY a.attempt_id,l.exam_id,b.slot_id",&[&batch,&paper,&original])?;
    let mut students: BTreeMap<String, Value> = BTreeMap::new();
    let mut questions: BTreeMap<String, (usize, f64, f64, Value)> = BTreeMap::new();
    let mut kinds: BTreeMap<String, (BTreeSet<String>, f64, f64)> = BTreeMap::new();
    let mut parts: BTreeMap<String, (BTreeSet<String>, f64, f64)> = BTreeMap::new();
    for row in &rows {
        let attempt = row["attempt_id"].as_str().unwrap();
        let score = row["score"].as_f64().unwrap_or_default();
        let weight = row["weight"].as_f64().unwrap_or_default();
        let key = format!(
            "{}:{}:{}",
            row["exam_id"].as_str().unwrap_or(""),
            row["runtime_sha256"].as_str().unwrap_or(""),
            row["slot_id"].as_str().unwrap_or("")
        );
        let student=students.entry(attempt.to_string()).or_insert_with(||json!({"attemptId":attempt,"candidateName":row["candidate_name"],"score":0.0,"total":0.0,"cells":{}}));
        student["score"] = json!(student["score"].as_f64().unwrap() + score);
        student["total"] = json!(student["total"].as_f64().unwrap() + weight);
        student["cells"][&key] = json!({"score":score,"isCorrect":row["is_correct"]});
        if weight <= 0.0 {
            continue;
        }
        let q=questions.entry(key).or_insert_with(||(0,0.0,0.0,json!({"examId":row["exam_id"],"runtimeSha256":row["runtime_sha256"],"slotId":row["slot_id"],"questionKind":row["question_kind"]})));
        q.0 += 1;
        q.1 += score;
        q.2 += weight;
        let k = kinds
            .entry(row["question_kind"].as_str().unwrap_or("").to_string())
            .or_default();
        k.0.insert(attempt.to_string());
        k.1 += score;
        k.2 += weight;
        let part = parts
            .entry(row["part_id"].as_str().unwrap_or("unknown").to_string())
            .or_default();
        part.0.insert(attempt.to_string());
        part.1 += score;
        part.2 += weight;
    }
    let mut questions: Vec<Value> = questions
        .into_iter()
        .map(|(key, (n, correct, total, mut q))| {
            q["key"] = json!(key);
            q["n"] = json!(n);
            q["correctPoints"] = json!(correct);
            q["totalPoints"] = json!(total);
            q["accuracy"] = json!(super::ratio(correct, total));
            q["hardConclusion"] = json!(n >= super::HARD_CONCLUSION_MIN_N);
            q
        })
        .collect();
    questions.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
    let mut hard = questions.clone();
    hard.sort_by(|a, b| {
        a["accuracy"]
            .as_f64()
            .partial_cmp(&b["accuracy"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hard.truncate(10);
    let kinds:Vec<_>=kinds.into_iter().map(|(kind,(ids,score,total))|json!({"kind":kind,"n":ids.len(),"correctPoints":score,"totalPoints":total,"accuracy":super::ratio(score,total)})).collect();
    let scores: Vec<_> = students
        .values()
        .map(|s| s["score"].as_f64().unwrap())
        .collect();
    let mut histogram: BTreeMap<i64, usize> = BTreeMap::new();
    for student in students.values() {
        let total = student["total"].as_f64().unwrap_or_default();
        let accuracy = if total > 0.0 {
            student["score"].as_f64().unwrap_or_default() / total * 100.0
        } else {
            0.0
        };
        let lower = ((accuracy / 10.0).floor() as i64 * 10).min(90);
        *histogram.entry(lower).or_default() += 1;
    }
    let distribution: Vec<_> = (0..10).map(|i| json!({"fromPercentage":i*10,"toPercentage":(i+1)*10,"upperInclusive":i==9,"n":histogram.get(&(i*10)).copied().unwrap_or(0)})).collect();
    let parts: Vec<_> = parts.into_iter().map(|(part_id,(ids,score,total))| json!({"partId":part_id,"n":ids.len(),"correctPoints":score,"totalPoints":total,"accuracy":super::ratio(score,total)})).collect();
    let n = students.len();
    let mean = if n == 0 {
        None
    } else {
        Some(scores.iter().sum::<f64>() / n as f64)
    };
    Ok(
        json!({"n":n,"scoreMode":if original{"original"}else{"effective"},"score":{"n":n,"mean":mean,"values":scores},"distribution":distribution,"parts":parts,"questions":questions,"hardQuestions":hard,"kinds":kinds,"matrix":students.values().collect::<Vec<_>>(),"students":students.values().collect::<Vec<_>>(),"disclaimer":super::DISCLAIMER}),
    )
}
