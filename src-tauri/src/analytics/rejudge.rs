use super::{
    records::query,
    review::{audit, cas_attempt, create_override, id, now, operator, text},
};
use crate::CommandResult;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

fn normalize(v: &Value) -> String {
    v.as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
pub fn preview(conn: &Connection, rule: &Value) -> CommandResult<Value> {
    let scope = text(rule, "scope")?;
    let reason = text(rule, "reason")?;
    if !["paper", "exam", "slot"].contains(&scope) {
        return Err("INVALID_REJUDGE_RULE: 重判范围无效".into());
    }
    let paper = rule["paperId"].as_str();
    let exam = rule["examId"].as_str();
    let slot = rule["responseSlotId"].as_str();
    let hash = rule["runtimeSha256"].as_str();
    let modality = rule["modality"].as_str().unwrap_or("reading");
    if !["reading", "listening"].contains(&modality) {
        return Err("INVALID_REJUDGE_RULE: 重判作答类型无效".into());
    }
    if (scope == "paper" && paper.is_none())
        || (scope != "paper" && (exam.is_none() || hash.is_none()))
        || (scope == "slot" && slot.is_none())
    {
        return Err("INVALID_REJUDGE_RULE: 重判范围必须指定试卷或题目版本及计分单元".into());
    }
    let accepted = rule["acceptedAnswers"]
        .as_array()
        .filter(|xs| {
            !xs.is_empty()
                && xs
                    .iter()
                    .all(|x| x.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false))
        })
        .ok_or("INVALID_REJUDGE_RULE: 请提供可接受答案列表")?;
    let candidates = if modality == "reading" {
        query(conn,"SELECT a.attempt_id,a.review_version,a.scorer_version,b.passage_index,COALESCE(b.response_slot_id,b.question_id) AS response_slot_id,b.user_answer_json,COALESCE(b.max_score,b.weight) AS max_score FROM attempts a JOIN answers b USING(attempt_id) JOIN passages p ON p.attempt_id=b.attempt_id AND p.passage_index=b.passage_index WHERE a.status='submitted' AND (?1 IS NULL OR a.paper_id=?1) AND (?2 IS NULL OR p.exam_id=?2) AND (?3 IS NULL OR COALESCE(b.response_slot_id,b.question_id)=?3) AND (?4 IS NULL OR p.runtime_sha256=?4) AND b.is_correct=0 AND NOT EXISTS(SELECT 1 FROM score_overrides o WHERE o.attempt_id=a.attempt_id AND o.modality='reading' AND o.passage_index=b.passage_index AND o.response_slot_id=COALESCE(b.response_slot_id,b.question_id) AND o.revoked_at IS NULL) ORDER BY a.attempt_id,b.passage_index,b.question_id",&[&paper,&exam,&slot,&hash])?
    } else {
        query(conn,"SELECT a.attempt_id,a.review_version,a.scorer_version,-1 AS passage_index,b.slot_id AS response_slot_id,b.user_answer_json,b.weight AS max_score FROM attempts a JOIN listening_answers b USING(attempt_id) JOIN listening_attempts p USING(attempt_id) WHERE a.status='submitted' AND (?1 IS NULL OR a.paper_id=?1) AND (?2 IS NULL OR p.exam_id=?2) AND (?3 IS NULL OR b.slot_id=?3) AND (?4 IS NULL OR p.runtime_sha256=?4) AND b.is_correct=0 AND NOT EXISTS(SELECT 1 FROM score_overrides o WHERE o.attempt_id=a.attempt_id AND o.modality='listening' AND o.response_slot_id=b.slot_id AND o.revoked_at IS NULL) ORDER BY a.attempt_id,b.slot_id",&[&paper,&exam,&slot,&hash])?
    };
    let affected: Vec<Value> = candidates
        .into_iter()
        .filter(|r| {
            let answer: Value =
                serde_json::from_str(r["user_answer_json"].as_str().unwrap_or("null"))
                    .unwrap_or(Value::Null);
            let one = if let Some(xs) = answer.as_array() {
                if xs.len() == 1 {
                    &xs[0]
                } else {
                    return false;
                }
            } else {
                &answer
            };
            one.is_string() && accepted.iter().any(|a| normalize(a) == normalize(one))
        })
        .collect();
    let token = super::import::sha256_hex(
        json!({"rule":rule,"affected":affected})
            .to_string()
            .as_bytes(),
    );
    Ok(
        json!({"n":affected.len(),"affected":affected,"previewToken":token,"reason":reason,"modality":modality}),
    )
}
pub fn apply(conn: &mut Connection, root: &Path, input: &Value) -> CommandResult<Value> {
    let op = operator(root)?;
    let rule = input.get("rule").ok_or("INVALID_REQUEST: 缺少重判规则")?;
    let expected = text(input, "previewToken")?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let current = preview(&tx, rule)?;
    if current["previewToken"] != expected {
        return Err("REJUDGE_CONFLICT: 预览后的作答或复核版本已变化，请重新预览".into());
    }
    let run = id();
    let rows = current["affected"].as_array().unwrap();
    let mut versions = BTreeMap::new();
    for r in rows {
        versions.insert(
            r["attempt_id"].as_str().unwrap(),
            r["review_version"].as_i64().unwrap(),
        );
    }
    for (attempt, version) in versions {
        cas_attempt(&tx, attempt, version)?;
    }
    for r in rows {
        create_override(
            &tx,
            &json!({"attemptId":r["attempt_id"],"modality":current["modality"],"passageIndex":r["passage_index"],"responseSlotId":r["response_slot_id"],"newScore":r["max_score"],"newIsCorrect":true,"reason":rule["reason"]}),
            &op,
            Some(&run),
        )?;
    }
    let old: Vec<Value> = rows.iter().map(|r| r["scorer_version"].clone()).collect();
    tx.execute("INSERT INTO rejudge_runs(run_id,scope,reason,old_scorer_version,new_rule_json,affected_count,operator,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![run,rule["scope"].as_str(),text(rule,"reason")?,json!(old).to_string(),rule.to_string(),rows.len() as i64,op,now()]).map_err(|e|e.to_string())?;
    let result = json!({"runId":run,"n":rows.len()});
    audit(
        &tx,
        &op,
        "apply_rejudge",
        "rejudge_run",
        &run,
        &current,
        &result,
        text(rule, "reason")?,
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
