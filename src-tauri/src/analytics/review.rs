//! Teacher decisions are append-only layers over imported student facts.
use super::{snapshot_v2::rows, DISCLAIMER};
use crate::CommandResult;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn req<'a>(v: &'a Value, key: &str) -> CommandResult<&'a str> {
    v[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("INVALID_INPUT:缺少 {key}"))
}
fn integer(v: &Value, key: &str) -> CommandResult<i64> {
    v[key]
        .as_i64()
        .ok_or_else(|| format!("INVALID_INPUT:{key} 必须是整数"))
}
fn num(v: &Value, key: &str) -> CommandResult<f64> {
    v[key]
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("INVALID_INPUT:{key} 必须是有限数值"))
}
fn boolean(v: &Value, key: &str) -> CommandResult<bool> {
    v[key]
        .as_bool()
        .ok_or_else(|| format!("INVALID_INPUT:{key} 必须是布尔值"))
}
fn one(conn: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> CommandResult<Value> {
    rows(conn, sql, args)?
        .into_iter()
        .next()
        .ok_or("NOT_FOUND:目标记录不存在".into())
}
fn conflict(changed: usize) -> CommandResult<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err("VERSION_CONFLICT:数据已变化，请刷新后重试".into())
    }
}
pub fn audit(
    conn: &Connection,
    operator: &str,
    action: &str,
    kind: &str,
    id: &str,
    before: &Value,
    after: &Value,
    reason: &str,
) -> CommandResult<()> {
    if operator.trim().is_empty() {
        return Err("OPERATOR_REQUIRED:请先填写操作人姓名".into());
    }
    conn.execute("INSERT INTO audit_log(id,at,operator,action,target_type,target_id,before_json,after_json,reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![uid(),now(),operator.trim(),action,kind,id,before.to_string(),after.to_string(),reason]).map_err(|e|format!("AUDIT_WRITE:审计记录保存失败：{e}"))?;
    Ok(())
}
fn bump_attempt(conn: &Connection, id: &str, version: i64) -> CommandResult<()> {
    conflict(
        conn.execute(
            "UPDATE attempts SET version=version+1 WHERE attempt_id=?1 AND version=?2",
            params![id, version],
        )
        .map_err(|e| e.to_string())?,
    )
}

pub fn list_overrides(conn: &Connection, id: &str) -> CommandResult<Value> {
    let items = rows(
        conn,
        "SELECT * FROM score_overrides WHERE attempt_id=? ORDER BY created_at",
        &[&id],
    )?;
    Ok(json!({"n":items.len(),"overrides":items}))
}
fn target(
    conn: &Connection,
    attempt: &str,
    modality: &str,
    index: i64,
    slot: &str,
) -> CommandResult<Value> {
    match modality {
        "reading"=>one(conn,"SELECT is_correct,COALESCE(score,is_correct*weight) AS score,COALESCE(max_score,weight) AS max_score FROM answers WHERE attempt_id=? AND passage_index=? AND response_slot_id=?",&[&attempt,&index,&slot]),
        "listening" if index == -1=>one(conn,"SELECT is_correct,score,max_score FROM listening_answers WHERE attempt_id=? AND slot_id=?",&[&attempt,&slot]),
        _=>Err("INVALID_TARGET:计分目标不合法".into()),
    }
}
fn insert_override(
    conn: &Connection,
    input: &Value,
    operator: &str,
    run: Option<&str>,
) -> CommandResult<Value> {
    let attempt = req(input, "attemptId")?;
    let modality = req(input, "modality")?;
    let index = if modality == "listening" {
        -1
    } else {
        integer(input, "passageIndex")?
    };
    let slot = req(input, "responseSlotId")?;
    let reason = req(input, "reason")?;
    let original = target(conn, attempt, modality, index, slot)?;
    if original["is_correct"].is_null() || original["score"].is_null() {
        return Err("UNGRADABLE:原始作答没有可用判分".into());
    }
    let score = num(input, "newScore")?;
    let correct = boolean(input, "newIsCorrect")?;
    if score < 0.0 || score > original["max_score"].as_f64().unwrap_or(0.0) {
        return Err("SCORE_RANGE:改判得分必须在零与满分之间".into());
    }
    let active:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM score_overrides WHERE attempt_id=?1 AND modality=?2 AND passage_index=?3 AND response_slot_id=?4 AND revoked_at IS NULL)",params![attempt,modality,index,slot],|r|r.get(0)).map_err(|e|e.to_string())?;
    if active {
        return Err("OVERRIDE_CONFLICT:此计分单元已有有效改判，请先撤销".into());
    }
    let id = uid();
    conn.execute("INSERT INTO score_overrides(override_id,attempt_id,modality,passage_index,response_slot_id,original_is_correct,original_score,new_is_correct,new_score,reason,operator,created_at,run_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![id,attempt,modality,index,slot,original["is_correct"].as_i64(),original["score"].as_f64(),correct as i64,score,reason,operator,now(),run]).map_err(|e|format!("OVERRIDE_WRITE:改判保存失败：{e}"))?;
    let result = one(
        conn,
        "SELECT * FROM score_overrides WHERE override_id=?",
        &[&id],
    )?;
    audit(
        conn,
        operator,
        "create_score_override",
        "attempt",
        attempt,
        &original,
        &result,
        reason,
    )?;
    Ok(result)
}
pub fn create_override(conn: &mut Connection, input: &Value) -> CommandResult<Value> {
    let operator = req(input, "operator")?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    bump_attempt(
        &tx,
        req(input, "attemptId")?,
        integer(input, "expectedVersion")?,
    )?;
    let value = insert_override(&tx, input, operator, None)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(value)
}
pub fn revoke_override(conn: &mut Connection, input: &Value) -> CommandResult<Value> {
    let operator = req(input, "operator")?;
    let id = req(input, "overrideId")?;
    let reason = req(input, "reason")?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let old = one(
        &tx,
        "SELECT * FROM score_overrides WHERE override_id=?",
        &[&id],
    )?;
    conflict(tx.execute("UPDATE score_overrides SET revoked_at=?2,version=version+1 WHERE override_id=?1 AND version=?3 AND revoked_at IS NULL",params![id,now(),integer(input,"expectedVersion")?]).map_err(|e|e.to_string())?)?;
    tx.execute(
        "UPDATE attempts SET version=version+1 WHERE attempt_id=?1",
        params![old["attempt_id"].as_str()],
    )
    .map_err(|e| e.to_string())?;
    let after = one(
        &tx,
        "SELECT * FROM score_overrides WHERE override_id=?",
        &[&id],
    )?;
    audit(
        &tx,
        operator,
        "revoke_score_override",
        "attempt",
        old["attempt_id"].as_str().unwrap_or_default(),
        &old,
        &after,
        reason,
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(after)
}
pub fn set_counted(conn: &mut Connection, input: &Value) -> CommandResult<Value> {
    let operator = req(input, "operator")?;
    let id = req(input, "attemptId")?;
    let reason = req(input, "reason")?;
    let counted = boolean(input, "counted")?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let old = one(&tx, "SELECT * FROM attempts WHERE attempt_id=?", &[&id])?;
    if counted && old["status"] != "submitted" {
        return Err("ATTEMPT_NOT_SUBMITTED:未交卷记录不能计入统计".into());
    }
    conflict(tx.execute("UPDATE attempts SET counted=?2,counted_override=1,exclude_reason=?3,version=version+1 WHERE attempt_id=?1 AND version=?4",params![id,counted as i64,reason,integer(input,"expectedVersion")?]).map_err(|e|e.to_string())?)?;
    let after = one(&tx, "SELECT * FROM attempts WHERE attempt_id=?", &[&id])?;
    audit(
        &tx,
        operator,
        "set_attempt_counted",
        "attempt",
        id,
        &old,
        &after,
        reason,
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(after)
}
pub fn list_audit(conn: &Connection, filter: &Value) -> CommandResult<Value> {
    let kind = filter["targetType"].as_str();
    let id = filter["targetId"].as_str();
    let action = filter["action"].as_str();
    let limit = filter["limit"].as_i64().unwrap_or(200).clamp(1, 1000);
    let offset = filter["offset"].as_i64().unwrap_or(0).max(0);
    let n:i64=conn.query_row("SELECT COUNT(*) FROM audit_log WHERE (?1 IS NULL OR target_type=?1) AND (?2 IS NULL OR target_id=?2) AND (?3 IS NULL OR action=?3)",params![kind,id,action],|r|r.get(0)).map_err(|e|e.to_string())?;
    let items=rows(conn,"SELECT * FROM audit_log WHERE (?1 IS NULL OR target_type=?1) AND (?2 IS NULL OR target_id=?2) AND (?3 IS NULL OR action=?3) ORDER BY at DESC,id LIMIT ?4 OFFSET ?5",&[&kind,&id,&action,&limit,&offset])?;
    Ok(json!({"n":n,"audit":items,"offset":offset}))
}

pub fn writing_queue(
    conn: &Connection,
    paper: Option<&str>,
    status: Option<&str>,
) -> CommandResult<Value> {
    let items=rows(conn,"SELECT w.*,a.paper_id,a.candidate_id,a.candidate_name,(SELECT status FROM writing_grades g WHERE g.attempt_id=w.attempt_id AND g.writing_submission_id=w.writing_submission_id ORDER BY created_at DESC LIMIT 1) AS grade_status FROM writings w JOIN attempts a USING(attempt_id) WHERE (?1 IS NULL OR a.paper_id=?1) AND (?2 IS NULL OR COALESCE((SELECT status FROM writing_grades g WHERE g.attempt_id=w.attempt_id AND g.writing_submission_id=w.writing_submission_id ORDER BY created_at DESC LIMIT 1),'ungraded')=?2) ORDER BY w.submitted_at",&[&paper,&status])?;
    Ok(json!({"n":items.len(),"writings":items,"disclaimer":DISCLAIMER}))
}
pub fn save_grade(conn: &mut Connection, input: &Value) -> CommandResult<Value> {
    let operator = req(input, "operator")?;
    let attempt = req(input, "attemptId")?;
    let writing = req(input, "writingSubmissionId")?;
    let expected = integer(input, "expectedVersion")?;
    let scores = [
        num(input, "taTr")?,
        num(input, "cc")?,
        num(input, "lr")?,
        num(input, "gra")?,
    ];
    if scores
        .iter()
        .any(|n| *n < 0.0 || *n > 9.0 || n * 2.0 != (n * 2.0).floor())
    {
        return Err("WRITING_SCORE_RANGE:写作维度须为 0–9，步长 0.5".into());
    }
    let average = scores.iter().sum::<f64>() / 4.0;
    let id = input["gradeId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(uid);
    let comment = input["overallComment"].as_str().unwrap_or_default();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let fact = one(
        &tx,
        "SELECT * FROM writings WHERE attempt_id=? AND writing_submission_id=?",
        &[&attempt, &writing],
    )?;
    let old = rows(&tx, "SELECT * FROM writing_grades WHERE grade_id=?", &[&id])?
        .into_iter()
        .next()
        .unwrap_or(Value::Null);
    if old.is_null() {
        if expected != 0 {
            return Err("VERSION_CONFLICT:新批改版本的 expectedVersion 必须为零".into());
        }
        tx.execute("INSERT INTO writing_grades(grade_id,attempt_id,writing_submission_id,task_type,grader,ta_tr,cc,lr,gra,task_band,overall_comment,status,version,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'draft',1,?12,?12)",params![id,attempt,writing,fact["task_type"].as_str(),operator,scores[0],scores[1],scores[2],scores[3],average,comment,now()]).map_err(|e|e.to_string())?;
    } else {
        if old["attempt_id"] != attempt || old["writing_submission_id"] != writing {
            return Err("TARGET_MISMATCH:批改版本不属于此作文".into());
        }
        conflict(tx.execute("UPDATE writing_grades SET ta_tr=?2,cc=?3,lr=?4,gra=?5,task_band=?6,overall_comment=?7,grader=?8,version=version+1,updated_at=?9 WHERE grade_id=?1 AND version=?10 AND status='draft'",params![id,scores[0],scores[1],scores[2],scores[3],average,comment,operator,now(),expected]).map_err(|e|e.to_string())?)?;
    }
    let after = one(&tx, "SELECT * FROM writing_grades WHERE grade_id=?", &[&id])?;
    audit(
        &tx,
        operator,
        "save_writing_grade",
        "attempt",
        attempt,
        &old,
        &after,
        input["reason"].as_str().unwrap_or("保存写作批改草稿"),
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(after)
}
pub fn transition_grade(
    conn: &mut Connection,
    input: &Value,
    publish: bool,
) -> CommandResult<Value> {
    let id = req(input, "gradeId")?;
    let operator = req(input, "operator")?;
    let expected = integer(input, "expectedVersion")?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let old = one(&tx, "SELECT * FROM writing_grades WHERE grade_id=?", &[&id])?;
    let required = if publish { "reviewed" } else { "draft" };
    if old["status"] != required || old["version"].as_i64() != Some(expected) {
        return Err("VERSION_CONFLICT:批改状态或版本已变化".into());
    }
    let previous = if publish {
        let old_published=rows(&tx,"SELECT * FROM writing_grades WHERE attempt_id=? AND writing_submission_id=? AND status='published'",&[&old["attempt_id"].as_str(),&old["writing_submission_id"].as_str()])?;
        tx.execute("UPDATE writing_grades SET status='reviewed',version=version+1,updated_at=?3 WHERE attempt_id=?1 AND writing_submission_id=?2 AND status='published'",params![old["attempt_id"].as_str(),old["writing_submission_id"].as_str(),now()]).map_err(|e|e.to_string())?;
        old_published
    } else {
        vec![]
    };
    let sql = if publish {
        "UPDATE writing_grades SET status='published',published_at=?2,updated_at=?2,version=version+1 WHERE grade_id=?1 AND version=?3 AND status='reviewed'"
    } else {
        "UPDATE writing_grades SET status='reviewed',reviewed_by=?4,updated_at=?2,version=version+1 WHERE grade_id=?1 AND version=?3 AND status='draft'"
    };
    let changed = if publish {
        tx.execute(sql, params![id, now(), expected])
    } else {
        tx.execute(sql, params![id, now(), expected, operator])
    }
    .map_err(|e| e.to_string())?;
    conflict(changed)?;
    let after = one(&tx, "SELECT * FROM writing_grades WHERE grade_id=?", &[&id])?;
    audit(
        &tx,
        operator,
        if publish {
            "publish_writing_grade"
        } else {
            "review_writing_grade"
        },
        "attempt",
        old["attempt_id"].as_str().unwrap_or_default(),
        &json!({"grade":old,"previousPublished":previous}),
        &after,
        input["reason"].as_str().unwrap_or(if publish {
            "发布写作批改"
        } else {
            "复核写作批改"
        }),
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(after)
}
pub fn annotation(conn: &mut Connection, input: &Value, delete: bool) -> CommandResult<Value> {
    let grade = req(input, "gradeId")?;
    let operator = req(input, "operator")?;
    let expected = integer(input, "expectedVersion")?;
    let id = input["annotationId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(uid);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let record=one(&tx,"SELECT g.*,w.content FROM writing_grades g JOIN writings w USING(attempt_id,writing_submission_id) WHERE grade_id=?",&[&grade])?;
    let old = rows(
        &tx,
        "SELECT * FROM writing_annotations WHERE annotation_id=?",
        &[&id],
    )?
    .into_iter()
    .next()
    .unwrap_or(Value::Null);
    if !old.is_null() && old["grade_id"] != grade {
        return Err("TARGET_MISMATCH:批注不属于此批改版本".into());
    }
    conflict(tx.execute("UPDATE writing_grades SET version=version+1,updated_at=?3 WHERE grade_id=?1 AND version=?2 AND status='draft'",params![grade,expected,now()]).map_err(|e|e.to_string())?)?;
    if delete {
        conflict(
            tx.execute(
                "DELETE FROM writing_annotations WHERE annotation_id=?1 AND grade_id=?2",
                params![id, grade],
            )
            .map_err(|e| e.to_string())?,
        )?;
    } else {
        let start = integer(input, "startOffset")?;
        let end = integer(input, "endOffset")?;
        let quote = req(input, "quotedText")?;
        let category = req(input, "category")?;
        if !["语法", "词汇", "衔接", "任务回应", "其他"].contains(&category) {
            return Err("ANNOTATION_CATEGORY:批注类别无效".into());
        }
        let utf16: Vec<u16> = record["content"]
            .as_str()
            .unwrap_or_default()
            .encode_utf16()
            .collect();
        if start < 0
            || end <= start
            || end as usize > utf16.len()
            || String::from_utf16(&utf16[start as usize..end as usize])
                .ok()
                .as_deref()
                != Some(quote)
        {
            return Err("ANNOTATION_RANGE:批注范围与引用文字不一致".into());
        }
        tx.execute("INSERT INTO writing_annotations(annotation_id,grade_id,start_offset,end_offset,quoted_text,category,comment) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(annotation_id) DO UPDATE SET start_offset=excluded.start_offset,end_offset=excluded.end_offset,quoted_text=excluded.quoted_text,category=excluded.category,comment=excluded.comment",params![id,grade,start,end,quote,category,req(input,"comment")?]).map_err(|e|e.to_string())?;
    }
    let after = if delete {
        Value::Null
    } else {
        one(
            &tx,
            "SELECT * FROM writing_annotations WHERE annotation_id=?",
            &[&id],
        )?
    };
    audit(
        &tx,
        operator,
        if delete {
            "delete_annotation"
        } else {
            "save_annotation"
        },
        "attempt",
        record["attempt_id"].as_str().unwrap_or_default(),
        &old,
        &after,
        "写作批注",
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(json!({"annotation":after,"gradeVersion":expected+1}))
}

fn normalized(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
pub fn preview_rejudge(conn: &Connection, rule: &Value) -> CommandResult<Value> {
    let scope = req(rule, "scope")?;
    let modality = req(rule, "modality")?;
    if !["paper", "exam", "slot"].contains(&scope) || !["reading", "listening"].contains(&modality)
    {
        return Err("REJUDGE_RULE:重判范围无效".into());
    }
    let accepted = rule["acceptedAnswers"]
        .as_array()
        .filter(|a| !a.is_empty() && a.iter().all(|s| s.as_str().is_some()))
        .ok_or("REJUDGE_RULE:请填写可接受的答案变体")?;
    let accepted: Vec<String> = accepted
        .iter()
        .filter_map(Value::as_str)
        .map(normalized)
        .collect();
    let paper = rule["paperId"].as_str();
    let exam = rule["examId"].as_str();
    let slot = rule["responseSlotId"].as_str();
    if (scope == "paper" && paper.is_none())
        || (scope != "paper" && exam.is_none())
        || (scope == "slot" && slot.is_none())
    {
        return Err("REJUDGE_RULE:重判范围缺少标识".into());
    }
    let sql = if modality == "reading" {
        "SELECT a.attempt_id,a.version,a.scorer_version,p.exam_id,r.passage_index,r.response_slot_id,r.user_answer_json,r.correct_answer_json,r.is_correct,COALESCE(r.score,r.is_correct*r.weight) AS score,COALESCE(r.max_score,r.weight) AS max_score,o.override_id FROM answers r JOIN attempts a USING(attempt_id) JOIN passages p USING(attempt_id,passage_index) LEFT JOIN score_overrides o ON o.attempt_id=r.attempt_id AND o.modality='reading' AND o.passage_index=r.passage_index AND o.response_slot_id=r.response_slot_id AND o.revoked_at IS NULL WHERE (?1 IS NULL OR a.paper_id=?1) AND (?2 IS NULL OR p.exam_id=?2) AND (?3 IS NULL OR r.response_slot_id=?3) ORDER BY a.attempt_id,r.passage_index,r.response_slot_id"
    } else {
        "SELECT a.attempt_id,a.version,a.scorer_version,l.exam_id,-1 AS passage_index,r.slot_id AS response_slot_id,r.user_answer_json,r.correct_answer_json,r.is_correct,r.score,r.max_score,o.override_id FROM listening_answers r JOIN attempts a USING(attempt_id) JOIN listening_attempts l USING(attempt_id) LEFT JOIN score_overrides o ON o.attempt_id=r.attempt_id AND o.modality='listening' AND o.response_slot_id=r.slot_id AND o.revoked_at IS NULL WHERE (?1 IS NULL OR a.paper_id=?1) AND (?2 IS NULL OR l.exam_id=?2) AND (?3 IS NULL OR r.slot_id=?3) ORDER BY a.attempt_id,r.slot_id"
    };
    let candidates = rows(conn, sql, &[&paper, &exam, &slot])?;
    let mut affected = Vec::new();
    let mut blocked = Vec::new();
    for row in &candidates {
        let answer = row["user_answer_json"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok());
        let text = answer.as_ref().and_then(Value::as_str).or_else(|| {
            answer
                .as_ref()
                .and_then(Value::as_array)
                .filter(|a| a.len() == 1)
                .and_then(|a| a[0].as_str())
        });
        if row["is_correct"].as_i64() != Some(0)
            || !text.is_some_and(|s| accepted.contains(&normalized(s)))
        {
            continue;
        }
        if !row["override_id"].is_null() {
            blocked.push(row.clone());
        } else {
            affected.push(row.clone());
        }
    }
    let token = super::import::sha256_hex(
        json!({"rule":rule,"candidates":candidates})
            .to_string()
            .as_bytes(),
    );
    Ok(
        json!({"n":candidates.len(),"affectedCount":affected.len(),"affected":affected,"blocked":blocked,"previewToken":token}),
    )
}
pub fn apply_rejudge(conn: &mut Connection, input: &Value) -> CommandResult<Value> {
    let operator = req(input, "operator")?;
    let reason = req(input, "reason")?;
    let token = req(input, "previewToken")?;
    let rule = &input["rule"];
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let preview = preview_rejudge(&tx, rule)?;
    if preview["previewToken"] != token {
        return Err("VERSION_CONFLICT:重判预览已过期，请重新预览".into());
    }
    if preview["blocked"].as_array().is_some_and(|a| !a.is_empty()) {
        return Err("OVERRIDE_CONFLICT:影响范围已有人工改判，请先核对并撤销".into());
    }
    let run = uid();
    let affected = preview["affected"]
        .as_array()
        .ok_or("REJUDGE_RULE:无效预览")?;
    let versions: Vec<Value> = affected
        .iter()
        .map(|a| a["scorer_version"].clone())
        .collect();
    tx.execute("INSERT INTO rejudge_runs(run_id,scope,reason,old_scorer_version,new_rule_json,affected_count,operator,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![run,req(rule,"scope")?,reason,json!(versions).to_string(),rule.to_string(),affected.len() as i64,operator,now()]).map_err(|e|e.to_string())?;
    for row in affected {
        insert_override(
            &tx,
            &json!({"attemptId":row["attempt_id"],"modality":rule["modality"],"passageIndex":row["passage_index"],"responseSlotId":row["response_slot_id"],"newScore":row["max_score"],"newIsCorrect":true,"reason":reason}),
            operator,
            Some(&run),
        )?;
        tx.execute(
            "UPDATE attempts SET version=version+1 WHERE attempt_id=?1",
            params![row["attempt_id"].as_str()],
        )
        .map_err(|e| e.to_string())?;
    }
    audit(
        &tx,
        operator,
        "apply_rejudge",
        "rejudge",
        &run,
        &Value::Null,
        &preview,
        reason,
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(json!({"runId":run,"n":preview["n"],"affectedCount":affected.len()}))
}

pub fn operator_name(conn: &Connection, provided: Option<&str>) -> CommandResult<String> {
    if let Some(s) = provided.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(s.to_string());
    }
    rows(
        conn,
        "SELECT value FROM analytics_settings WHERE key='operator_name'",
        &[],
    )?
    .first()
    .and_then(|v| v["value"].as_str())
    .filter(|s| !s.trim().is_empty())
    .map(str::to_string)
    .ok_or("OPERATOR_REQUIRED:请先设置操作人姓名".into())
}
pub fn set_operator(conn: &mut Connection, input: &Value) -> CommandResult<Value> {
    let operator = req(input, "operator")?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let before = rows(
        &tx,
        "SELECT value FROM analytics_settings WHERE key='operator_name'",
        &[],
    )?;
    tx.execute("INSERT INTO analytics_settings(key,value) VALUES('operator_name',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[operator]).map_err(|e|e.to_string())?;
    audit(
        &tx,
        operator,
        "set_operator_name",
        "settings",
        "operator_name",
        &json!(before),
        &json!({"operator":operator}),
        "设置操作人姓名",
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(json!({"operator":operator}))
}

pub fn merge_identity(conn: &mut Connection, input: &Value) -> CommandResult<Value> {
    let operator = req(input, "operator")?;
    let candidate = req(input, "candidateId")?;
    let name = req(input, "candidateName")?;
    let target = req(input, "mergedInto")?;
    let reason = req(input, "reason")?;
    let expected = integer(input, "expectedVersion")?;
    if candidate == target {
        return Err("IDENTITY_TARGET:不能合并到同一卡号".into());
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    one(
        &tx,
        "SELECT attempt_id FROM attempts WHERE candidate_id=? AND candidate_name=? LIMIT 1",
        &[&candidate, &name],
    )?;
    one(
        &tx,
        "SELECT attempt_id FROM attempts WHERE candidate_id=? LIMIT 1",
        &[&target],
    )?;
    let old = rows(
        &tx,
        "SELECT * FROM candidate_aliases WHERE candidate_id=? AND candidate_name=?",
        &[&candidate, &name],
    )?
    .into_iter()
    .next()
    .unwrap_or(Value::Null);
    let cycle:bool=tx.query_row("WITH RECURSIVE chain(id) AS (SELECT ?1 UNION SELECT a.merged_into FROM candidate_aliases a JOIN chain c ON a.candidate_id=c.id WHERE a.merged_into IS NOT NULL) SELECT EXISTS(SELECT 1 FROM chain WHERE id=?2)",params![target,candidate],|r|r.get(0)).map_err(|e|e.to_string())?;
    if cycle {
        return Err("IDENTITY_CYCLE:身份合并不能形成循环".into());
    }
    if old.is_null() {
        if expected != 0 {
            return Err("VERSION_CONFLICT:身份别名已变化".into());
        }
        tx.execute("INSERT INTO candidate_aliases(candidate_id,candidate_name,student_uuid,merged_into,operator,at) VALUES(?1,?2,?3,?4,?5,?6)",params![candidate,name,input["studentUuid"].as_str(),target,operator,now()]).map_err(|e|e.to_string())?;
    } else {
        conflict(tx.execute("UPDATE candidate_aliases SET student_uuid=?3,merged_into=?4,operator=?5,at=?6,version=version+1 WHERE candidate_id=?1 AND candidate_name=?2 AND version=?7",params![candidate,name,input["studentUuid"].as_str(),target,operator,now(),expected]).map_err(|e|e.to_string())?)?;
    }
    let after = one(
        &tx,
        "SELECT * FROM candidate_aliases WHERE candidate_id=? AND candidate_name=?",
        &[&candidate, &name],
    )?;
    audit(
        &tx,
        operator,
        "merge_candidate_identity",
        "candidate",
        candidate,
        &old,
        &after,
        reason,
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(after)
}
