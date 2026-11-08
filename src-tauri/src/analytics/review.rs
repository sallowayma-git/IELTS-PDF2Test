//! Teacher-owned facts. Every successful mutation and its audit share a transaction.
use super::records::query;
use crate::CommandResult;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::path::Path;

pub fn text<'a>(v: &'a Value, key: &str) -> CommandResult<&'a str> {
    v[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("INVALID_REQUEST: {key} 不能为空"))
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub fn settings(root: &Path) -> CommandResult<Value> {
    let p = super::analytics_db_path(root);
    if !p.is_file() {
        return Ok(json!({"operatorName":""}));
    }
    let conn = Connection::open_with_flags(p, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let name: Option<String> = conn
        .query_row(
            "SELECT operator_name FROM review_settings WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(json!({"operatorName":name.unwrap_or_default()}))
}
pub fn operator(root: &Path) -> CommandResult<String> {
    let v = settings(root)?;
    Ok(text(&v, "operatorName")
        .map_err(|_| "OPERATOR_REQUIRED: 请先在设置中填写操作人姓名".to_string())?
        .to_string())
}
pub fn save_settings(root: &Path, input: &Value) -> CommandResult<Value> {
    let name = text(input, "operatorName")?;
    if name.chars().count() > 100 {
        return Err("INVALID_OPERATOR: 操作人姓名过长".into());
    }
    let value = json!({"operatorName":name});
    let mut conn = super::open_analytics(root)?;
    let before = settings(root)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO review_settings(id,operator_name,updated_at) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET operator_name=excluded.operator_name,updated_at=excluded.updated_at",params![name,now()]).map_err(|e|e.to_string())?;
    audit(
        &tx,
        name,
        "save_review_settings",
        "settings",
        "operator",
        &before,
        &value,
        "设置操作人姓名",
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(value)
}
pub fn audit(
    conn: &Connection,
    op: &str,
    action: &str,
    kind: &str,
    target: &str,
    before: &Value,
    after: &Value,
    reason: &str,
) -> CommandResult<()> {
    conn.execute("INSERT INTO audit_log(id,at,operator,action,target_type,target_id,before_json,after_json,reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id(),now(),op,action,kind,target,before.to_string(),after.to_string(),reason]).map_err(|e|e.to_string())?;
    Ok(())
}
pub fn cas_attempt(conn: &Connection, attempt: &str, expected: i64) -> CommandResult<()> {
    let n=conn.execute("UPDATE attempts SET review_version=review_version+1 WHERE attempt_id=?1 AND review_version=?2 AND status='submitted'",params![attempt,expected]).map_err(|e|e.to_string())?;
    if n != 1 {
        return Err("VERSION_CONFLICT: 考试不存在、未交卷或复核版本已变化，请刷新后重试".into());
    }
    Ok(())
}
pub fn create_override(
    conn: &Connection,
    input: &Value,
    op: &str,
    run: Option<&str>,
) -> CommandResult<Value> {
    let attempt = text(input, "attemptId")?;
    let slot = text(input, "responseSlotId")?;
    let reason = text(input, "reason")?;
    let modality = input["modality"].as_str().unwrap_or("reading");
    let passage = if modality == "listening" {
        -1
    } else {
        input["passageIndex"]
            .as_i64()
            .ok_or("INVALID_REQUEST: 缺少 passageIndex")?
    };
    let original=match modality {
        "reading"=>query(conn,"SELECT is_correct,COALESCE(score,CASE is_correct WHEN 1 THEN weight WHEN 0 THEN 0 END) AS score,COALESCE(max_score,weight) AS max_score,user_answer_json FROM answers WHERE attempt_id=?1 AND passage_index=?2 AND COALESCE(response_slot_id,question_id)=?3",&[&attempt,&passage,&slot])?,
        "listening"=>query(conn,"SELECT is_correct,is_correct*weight AS score,weight AS max_score,user_answer_json FROM listening_answers WHERE attempt_id=?1 AND slot_id=?2",&[&attempt,&slot])?,
        _=>return Err("INVALID_MODALITY: 作答类型无效".into()),
    }.into_iter().next().ok_or("SLOT_NOT_FOUND: 该考试中不存在此计分单元")?;
    let score = input["newScore"]
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or("INVALID_SCORE: 得分必须为有限数字")?;
    let max = original["max_score"].as_f64().unwrap_or(0.0);
    let correct = input["newIsCorrect"]
        .as_bool()
        .ok_or("INVALID_REQUEST: 缺少改判结果")?;
    if score < 0.0 || score > max || (correct && score != max) {
        return Err("INVALID_SCORE: 改判分数越界或与判对状态不一致".into());
    }
    let override_id = id();
    let at = now();
    conn.execute("INSERT INTO score_overrides(override_id,attempt_id,modality,passage_index,response_slot_id,original_is_correct,original_score,original_answer_json,new_is_correct,new_score,reason,operator,created_at,run_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![override_id,attempt,modality,passage,slot,original["is_correct"].as_i64(),original["score"].as_f64(),original["user_answer_json"].as_str(),correct as i64,score,reason,op,at,run]).map_err(|e|if e.to_string().contains("UNIQUE"){"OVERRIDE_CONFLICT: 该计分单元已有有效改判，请先撤销".into()}else{e.to_string()})?;
    let result = json!({"overrideId":override_id,"attemptId":attempt,"newScore":score,"newIsCorrect":correct});
    audit(
        conn,
        op,
        "create_score_override",
        "attempt",
        attempt,
        &original,
        &result,
        reason,
    )?;
    Ok(result)
}

pub fn mutate(
    conn: &mut Connection,
    root: &Path,
    action: &str,
    input: &Value,
) -> CommandResult<Value> {
    let op = operator(root)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let result = match action {
        "create_score_override" => {
            let attempt = text(input, "attemptId")?;
            let expected = input["expectedVersion"]
                .as_i64()
                .ok_or("INVALID_REQUEST: 缺少复核版本")?;
            cas_attempt(&tx, attempt, expected)?;
            create_override(&tx, input, &op, None)?
        }
        "revoke_score_override" => {
            let oid = text(input, "overrideId")?;
            let reason = text(input, "reason")?;
            let before = query(
                &tx,
                "SELECT * FROM score_overrides WHERE override_id=?1 AND revoked_at IS NULL",
                &[&oid],
            )?
            .into_iter()
            .next()
            .ok_or("OVERRIDE_NOT_FOUND: 有效改判不存在")?;
            let attempt = before["attempt_id"].as_str().unwrap();
            cas_attempt(
                &tx,
                attempt,
                input["expectedVersion"]
                    .as_i64()
                    .ok_or("INVALID_REQUEST: 缺少复核版本")?,
            )?;
            tx.execute(
                "UPDATE score_overrides SET revoked_at=?2 WHERE override_id=?1",
                params![oid, now()],
            )
            .map_err(|e| e.to_string())?;
            let after = json!({"overrideId":oid,"revoked":true});
            audit(
                &tx, &op, action, "attempt", attempt, &before, &after, reason,
            )?;
            after
        }
        "set_attempt_counted" => {
            let attempt = text(input, "attemptId")?;
            let reason = text(input, "reason")?;
            let counted = input["counted"]
                .as_bool()
                .ok_or("INVALID_REQUEST: counted 必须为布尔值")?;
            let before = query(
                &tx,
                "SELECT counted,exclude_reason,review_version FROM attempts WHERE attempt_id=?1",
                &[&attempt],
            )?
            .into_iter()
            .next()
            .ok_or("ATTEMPT_NOT_FOUND: 考试不存在")?;
            cas_attempt(
                &tx,
                attempt,
                input["expectedVersion"]
                    .as_i64()
                    .ok_or("INVALID_REQUEST: 缺少复核版本")?,
            )?;
            tx.execute(
                "UPDATE attempts SET counted=?2,exclude_reason=?3 WHERE attempt_id=?1",
                params![
                    attempt,
                    counted as i64,
                    if counted { None } else { Some(reason) }
                ],
            )
            .map_err(|e| e.to_string())?;
            let after = json!({"attemptId":attempt,"counted":counted});
            audit(
                &tx, &op, action, "attempt", attempt, &before, &after, reason,
            )?;
            after
        }
        _ => return Err("UNKNOWN_COMMAND: 不支持的复核命令".into()),
    };
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}

pub fn list_audit(conn: &Connection, filter: &Value) -> CommandResult<Value> {
    let target = filter["targetId"].as_str();
    let action = filter["action"].as_str();
    let limit = filter["limit"].as_i64().unwrap_or(100).clamp(1, 500);
    let offset = filter["offset"].as_i64().unwrap_or(0).max(0);
    let rows=query(conn,"SELECT * FROM audit_log WHERE (?1 IS NULL OR target_id=?1) AND (?2 IS NULL OR action=?2) ORDER BY at DESC,id LIMIT ?3 OFFSET ?4",&[&target,&action,&limit,&offset])?;
    let n:i64=conn.query_row("SELECT COUNT(*) FROM audit_log WHERE (?1 IS NULL OR target_id=?1) AND (?2 IS NULL OR action=?2)",params![target,action],|r|r.get(0)).map_err(|e|e.to_string())?;
    Ok(json!({"n":n,"items":rows}))
}
