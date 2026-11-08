use super::{
    records::query,
    review::{audit, id, now, operator, text},
};
use crate::CommandResult;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::Path;

fn grade(conn: &Connection, gid: &str) -> CommandResult<Value> {
    query(
        conn,
        "SELECT * FROM writing_grades WHERE grade_id=?1",
        &[&gid],
    )?
    .into_iter()
    .next()
    .ok_or("GRADE_NOT_FOUND: 批改版本不存在".into())
}
fn expected(input: &Value) -> CommandResult<i64> {
    input["expectedVersion"]
        .as_i64()
        .ok_or("INVALID_REQUEST: 缺少批改版本".into())
}
fn dimension(input: &Value, key: &str) -> CommandResult<f64> {
    input[key]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 9.0 && (n * 2.0).fract().abs() < 1e-9)
        .ok_or_else(|| format!("INVALID_WRITING_SCORE: {key} 必须为 0–9，步长 0.5"))
}
fn check_draft(value: &Value, version: i64) -> CommandResult<()> {
    if value["version"].as_i64() != Some(version) || value["status"] != "draft" {
        return Err("VERSION_CONFLICT: 批改版本已变化或不再是草稿，请刷新".into());
    }
    Ok(())
}
pub fn mutate(
    conn: &mut Connection,
    root: &Path,
    action: &str,
    input: &Value,
) -> CommandResult<Value> {
    let op = operator(root)?;
    let version = expected(input)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let at = now();
    let reason = input["reason"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("人工写作批改");
    let result = if action == "save_writing_grade" {
        let attempt = text(input, "attemptId")?;
        let wid = text(input, "writingSubmissionId")?;
        let writing = query(
            &tx,
            "SELECT * FROM writings WHERE attempt_id=?1 AND writing_submission_id=?2",
            &[&attempt, &wid],
        )?
        .into_iter()
        .next()
        .ok_or("WRITING_NOT_FOUND: 此考试中不存在该作文")?;
        let values = [
            dimension(input, "taTr")?,
            dimension(input, "cc")?,
            dimension(input, "lr")?,
            dimension(input, "gra")?,
        ];
        let average = values.iter().sum::<f64>() / 4.0;
        let gid = input["gradeId"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(id);
        let existing = query(
            &tx,
            "SELECT * FROM writing_grades WHERE grade_id=?1",
            &[&gid],
        )?
        .into_iter()
        .next();
        if let Some(ref old) = existing {
            check_draft(old, version)?;
            if old["attempt_id"] != attempt || old["writing_submission_id"] != wid {
                return Err("GRADE_TARGET_MISMATCH: 批改不属于该作文".into());
            }
            tx.execute("UPDATE writing_grades SET ta_tr=?2,cc=?3,lr=?4,gra=?5,task_band=?6,overall_comment=?7,updated_at=?8,version=version+1 WHERE grade_id=?1 AND version=?9",params![gid,values[0],values[1],values[2],values[3],average,input["overallComment"].as_str().unwrap_or(""),at,version]).map_err(|e|e.to_string())?;
        } else {
            if version != 0 {
                return Err("VERSION_CONFLICT: 新批改的 expectedVersion 必须为 0".into());
            }
            tx.execute("INSERT INTO writing_grades(grade_id,attempt_id,writing_submission_id,task_type,grader,ta_tr,cc,lr,gra,task_band,overall_comment,status,version,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'draft',1,?12,?12)",params![gid,attempt,wid,writing["task_type"].as_str().unwrap_or("task2"),op,values[0],values[1],values[2],values[3],average,input["overallComment"].as_str().unwrap_or(""),at]).map_err(|e|e.to_string())?;
        }
        let after = grade(&tx, &gid)?;
        audit(
            &tx,
            &op,
            action,
            "writing_grade",
            &gid,
            &existing.unwrap_or(Value::Null),
            &after,
            reason,
        )?;
        after
    } else {
        let gid = text(input, "gradeId")?;
        let before = grade(&tx, gid)?;
        if before["version"].as_i64() != Some(version) {
            return Err("VERSION_CONFLICT: 批改版本已变化，请刷新".into());
        }
        match action {
            "review_writing_grade" | "publish_writing_grade" => {
                let required = if action == "review_writing_grade" {
                    "draft"
                } else {
                    "reviewed"
                };
                if before["status"] != required {
                    return Err("GRADE_STATE_CONFLICT: 批改状态不允许此操作".into());
                }
                if action == "review_writing_grade" {
                    tx.execute("UPDATE writing_grades SET status='reviewed',reviewed_by=?2,updated_at=?3,version=version+1 WHERE grade_id=?1",params![gid,op,at]).map_err(|e|e.to_string())?;
                } else {
                    let old=query(&tx,"SELECT * FROM writing_grades WHERE attempt_id=?1 AND writing_submission_id=?2 AND status='published'",&[&before["attempt_id"].as_str(),&before["writing_submission_id"].as_str()])?;
                    tx.execute("UPDATE writing_grades SET status='superseded',version=version+1,updated_at=?3 WHERE attempt_id=?1 AND writing_submission_id=?2 AND status='published'",params![before["attempt_id"].as_str(),before["writing_submission_id"].as_str(),at]).map_err(|e|e.to_string())?;
                    for prior in old {
                        audit(
                            &tx,
                            &op,
                            "supersede_writing_grade",
                            "writing_grade",
                            prior["grade_id"].as_str().unwrap(),
                            &prior,
                            &json!({"status":"superseded","replacement":gid}),
                            reason,
                        )?;
                    }
                    tx.execute("UPDATE writing_grades SET status='published',published_at=?2,updated_at=?2,version=version+1 WHERE grade_id=?1",params![gid,at]).map_err(|e|e.to_string())?;
                }
            }
            "save_annotation" => {
                check_draft(&before, version)?;
                let category = text(input, "category")?;
                if !["语法", "词汇", "衔接", "任务回应", "其他"].contains(&category) {
                    return Err("INVALID_ANNOTATION: 批注类别无效".into());
                }
                let content = query(
                    &tx,
                    "SELECT content FROM writings WHERE attempt_id=?1 AND writing_submission_id=?2",
                    &[
                        &before["attempt_id"].as_str(),
                        &before["writing_submission_id"].as_str(),
                    ],
                )?
                .into_iter()
                .next()
                .ok_or("WRITING_NOT_FOUND: 作文不存在")?;
                let units: Vec<u16> = content["content"]
                    .as_str()
                    .unwrap_or("")
                    .encode_utf16()
                    .collect();
                let start = input["startOffset"]
                    .as_u64()
                    .ok_or("INVALID_ANNOTATION: 起始位置无效")?
                    as usize;
                let end = input["endOffset"]
                    .as_u64()
                    .ok_or("INVALID_ANNOTATION: 结束位置无效")? as usize;
                let quote = input["quotedText"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("INVALID_ANNOTATION: 作文引用不能为空")?;
                let comment = text(input, "comment")?;
                if start >= end
                    || end > units.len()
                    || String::from_utf16(&units[start..end]).ok().as_deref() != Some(quote)
                {
                    return Err("ANNOTATION_QUOTE_MISMATCH: 批注位置与作文引用不一致".into());
                }
                let aid = input["annotationId"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(id);
                let prior = query(
                    &tx,
                    "SELECT * FROM writing_annotations WHERE annotation_id=?1",
                    &[&aid],
                )?
                .into_iter()
                .next();
                if prior
                    .as_ref()
                    .map(|p| p["grade_id"] != gid)
                    .unwrap_or(false)
                {
                    return Err("ANNOTATION_TARGET_MISMATCH: 批注不属于该批改版本".into());
                }
                tx.execute("INSERT INTO writing_annotations(annotation_id,grade_id,start_offset,end_offset,quoted_text,category,comment) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(annotation_id) DO UPDATE SET start_offset=excluded.start_offset,end_offset=excluded.end_offset,quoted_text=excluded.quoted_text,category=excluded.category,comment=excluded.comment",params![aid,gid,start as i64,end as i64,quote,category,comment]).map_err(|e|e.to_string())?;
                tx.execute(
                    "UPDATE writing_grades SET version=version+1,updated_at=?2 WHERE grade_id=?1",
                    params![gid, at],
                )
                .map_err(|e| e.to_string())?;
                audit(
                    &tx,
                    &op,
                    action,
                    "writing_annotation",
                    &aid,
                    &prior.unwrap_or(Value::Null),
                    input,
                    reason,
                )?;
            }
            "delete_annotation" => {
                check_draft(&before, version)?;
                let aid = text(input, "annotationId")?;
                let old = query(
                    &tx,
                    "SELECT * FROM writing_annotations WHERE annotation_id=?1 AND grade_id=?2",
                    &[&aid, &gid],
                )?
                .into_iter()
                .next()
                .ok_or("ANNOTATION_NOT_FOUND: 批注不存在或不属于该批改")?;
                tx.execute(
                    "DELETE FROM writing_annotations WHERE annotation_id=?1",
                    [aid],
                )
                .map_err(|e| e.to_string())?;
                tx.execute(
                    "UPDATE writing_grades SET version=version+1,updated_at=?2 WHERE grade_id=?1",
                    params![gid, at],
                )
                .map_err(|e| e.to_string())?;
                audit(
                    &tx,
                    &op,
                    action,
                    "writing_annotation",
                    aid,
                    &old,
                    &Value::Null,
                    reason,
                )?;
            }
            _ => return Err("UNKNOWN_COMMAND: 不支持的批改命令".into()),
        }
        let after = grade(&tx, gid)?;
        audit(
            &tx,
            &op,
            action,
            "writing_grade",
            gid,
            &before,
            &after,
            reason,
        )?;
        after
    };
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
pub fn queue(conn: &Connection, paper: Option<&str>, status: Option<&str>) -> CommandResult<Value> {
    let rows=query(conn,"SELECT w.*,a.paper_id,a.candidate_name,g.grade_id,g.status,g.version FROM writings w JOIN attempts a USING(attempt_id) LEFT JOIN writing_grades g ON g.grade_id=(SELECT g2.grade_id FROM writing_grades g2 WHERE g2.attempt_id=w.attempt_id AND g2.writing_submission_id=w.writing_submission_id ORDER BY g2.created_at DESC,g2.grade_id DESC LIMIT 1) WHERE (?1 IS NULL OR a.paper_id=?1) AND (?2 IS NULL OR COALESCE(g.status,'ungraded')=?2) ORDER BY w.submitted_at,w.attempt_id",&[&paper,&status])?;
    Ok(
        json!({"n":rows.len(),"items":rows,"scoreLabel":"写作人工评分（0–9）","disclaimer":super::DISCLAIMER}),
    )
}
