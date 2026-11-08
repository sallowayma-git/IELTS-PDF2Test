//! Immutable local evidence of exactly the payload handed to the student runtime.
use super::snapshot_v2::rows;
use crate::CommandResult;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Component, Path},
};

pub fn capture(conn: &Connection, source: &Value) -> CommandResult<()> {
    let payload = crate::export_artifacts::student_package_source(source);
    let bytes = crate::schema::common::canonical_json_bytes_js(&payload);
    let hash = super::import::sha256_hex(&bytes);
    let exam = payload["examId"]
        .as_str()
        .ok_or("ARCHIVE_EXAM_ID:运行时缺少题目标识")?;
    conn.execute("INSERT INTO item_content_snapshots(exam_id,runtime_sha256,runtime_source_json) VALUES(?1,?2,?3) ON CONFLICT DO NOTHING",params![exam,hash,String::from_utf8(bytes).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    Ok(())
}

pub fn record(
    conn: &mut Connection,
    destination: &str,
    pointer: &Value,
    current: bool,
    superseded: Option<&str>,
    outcomes: &[Value],
) -> CommandResult<()> {
    let id = pointer["paperId"]
        .as_str()
        .ok_or("ARCHIVE_PAPER:试卷档案缺少标识")?;
    let batch = pointer["batchId"]
        .as_str()
        .ok_or("ARCHIVE_PAPER:试卷档案缺少批次")?;
    let at = pointer["publishedAt"]
        .as_str()
        .ok_or("ARCHIVE_PAPER:试卷档案缺少发布时间")?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if current {
        tx.execute("UPDATE papers SET status='superseded',superseded_at=COALESCE(superseded_at,?2) WHERE destination=?1 AND status='current' AND paper_id<>?3",params![destination,at,id]).map_err(|e|e.to_string())?;
    }
    tx.execute("INSERT INTO papers(paper_id,batch_id,published_at,superseded_at,destination,pointer_json,status) VALUES(?1,?2,?3,?4,?5,?6,?7)
      ON CONFLICT(paper_id) DO UPDATE SET status=excluded.status,superseded_at=excluded.superseded_at",params![id,batch,at,superseded,destination,pointer.to_string(),if current{"current"}else{"superseded"}]).map_err(|e|e.to_string())?;
    for modality in ["reading", "listening"] {
        for item in pointer[modality].as_array().into_iter().flatten() {
            let outcome = outcomes.iter().find(|o| o["examId"] == item["examId"]);
            tx.execute("INSERT INTO paper_items(paper_id,modality,part,exam_id,library_item_id,edit_version,title,script_sha256,runtime_sha256,asset_manifest_sha256)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(paper_id,modality,part) DO UPDATE SET library_item_id=COALESCE(paper_items.library_item_id,excluded.library_item_id),edit_version=COALESCE(paper_items.edit_version,excluded.edit_version)",params![id,modality,item["part"].as_str(),item["examId"].as_str(),outcome.and_then(|o|o["itemId"].as_str()),outcome.and_then(|o|o["editVersion"].as_i64()),item["title"].as_str(),item["scriptSha256"].as_str(),item["runtimeSha256"].as_str(),item.pointer("/entry/checksums/assetManifestSha256").and_then(Value::as_str)]).map_err(|e|e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

/// NAS history files can be staged before a failed pointer write. Only current/superseded batches are committed evidence.
pub fn recover(app_root: &Path, destination: &str) -> CommandResult<Value> {
    let library = Path::new(destination);
    let reading = crate::export_nas_library::nas_reading_exams_dir(library);
    let current: Value = fs::read(reading.join("current-paper.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let mut conn = super::open_analytics(app_root)?;
    let mut n = 0;
    let mut warnings = Vec::new();
    let history = library.join(".publish-control/papers");
    if let Ok(files) = fs::read_dir(history) {
        for file in files.flatten() {
            if file.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let pointer: Value = match fs::read(file.path())
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
            {
                Some(v) => v,
                None => {
                    warnings.push(json!("历史试卷 JSON 不可读"));
                    continue;
                }
            };
            if pointer["schemaVersion"] != "CurrentPaperV1" {
                continue;
            }
            let is_current = pointer["paperId"] == current["paperId"];
            let batch = pointer["batchId"].as_str().unwrap_or_default();
            if batch.is_empty() || batch.contains(['/', '\\']) || batch == ".." {
                continue;
            }
            let superseded: Value = fs::read(
                library
                    .join(".publish-control/superseded")
                    .join(format!("{batch}.json")),
            )
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
            if !is_current && superseded.is_null() {
                continue;
            }
            record(
                &mut conn,
                destination,
                &pointer,
                is_current,
                superseded["supersededAt"].as_str(),
                &[],
            )?;
            n += 1;
            for modality in ["reading", "listening"] {
                for item in pointer[modality].as_array().into_iter().flatten() {
                    if let Err(e) = recover_content(&conn, &reading, item) {
                        warnings.push(json!({"examId":item["examId"],"message":e}));
                    }
                }
            }
        }
    }
    Ok(json!({"n":n,"warnings":warnings}))
}
fn recover_content(conn: &Connection, root: &Path, item: &Value) -> CommandResult<()> {
    let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM item_content_snapshots WHERE exam_id=?1 AND runtime_sha256=?2)",params![item["examId"].as_str(),item["runtimeSha256"].as_str()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if exists {
        return Ok(());
    }
    let relative = item["script"]
        .as_str()
        .ok_or("CONTENT_UNAVAILABLE:历史脚本路径缺失")?
        .trim_start_matches("./");
    if !relative.starts_with("releases/")
        || Path::new(relative)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("ARCHIVE_PATH:历史脚本路径不安全".into());
    }
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|_| "CONTENT_UNAVAILABLE:历史脚本已清理，无法恢复内容".to_string())?;
    if !path.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
        return Err("ARCHIVE_PATH:历史脚本越界".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if Some(super::import::sha256_hex(&bytes).as_str()) != item["scriptSha256"].as_str() {
        return Err("ARCHIVE_INTEGRITY:历史脚本校验失败".into());
    }
    let source = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let marker = "global.__READING_EXAM_DATA__.register(";
    let args = source
        .split_once(marker)
        .ok_or("ARCHIVE_WRAPPER:历史脚本不是受支持的发布封装")?
        .1;
    let mut key = serde_json::Deserializer::from_str(args).into_iter::<String>();
    let exam = key
        .next()
        .ok_or("ARCHIVE_WRAPPER:缺少 examId")?
        .map_err(|e| e.to_string())?;
    let tail = args[key.byte_offset()..]
        .trim_start()
        .strip_prefix(',')
        .ok_or("ARCHIVE_WRAPPER:缺少内容")?;
    let payload = serde_json::Deserializer::from_str(tail)
        .into_iter::<Value>()
        .next()
        .ok_or("ARCHIVE_WRAPPER:缺少内容")?
        .map_err(|e| e.to_string())?;
    let hash = super::import::sha256_hex(&crate::schema::common::canonical_json_bytes_js(&payload));
    if Some(exam.as_str()) != item["examId"].as_str()
        || Some(hash.as_str()) != item["runtimeSha256"].as_str()
    {
        return Err("ARCHIVE_INTEGRITY:历史运行时校验失败".into());
    }
    capture(conn, &payload)
}

pub fn list(conn: &Connection) -> CommandResult<Value> {
    let items = rows(conn, "SELECT * FROM papers ORDER BY published_at DESC", &[])?;
    Ok(json!({"n":items.len(),"papers":items,"disclaimer":super::DISCLAIMER}))
}
pub fn get(conn: &Connection, id: &str) -> CommandResult<Value> {
    let paper = rows(conn, "SELECT * FROM papers WHERE paper_id=?", &[&id])?
        .into_iter()
        .next()
        .ok_or("PAPER_NOT_FOUND:试卷档案不存在")?;
    let items=rows(conn,"SELECT p.*,s.runtime_source_json IS NOT NULL AS content_available FROM paper_items p LEFT JOIN item_content_snapshots s USING(exam_id,runtime_sha256) WHERE paper_id=? ORDER BY modality,part",&[&id])?;
    Ok(json!({"n":items.len(),"paper":paper,"items":items}))
}
pub fn content(conn: &Connection, exam: &str, hash: &str) -> CommandResult<Value> {
    let items=rows(conn,"SELECT runtime_source_json FROM item_content_snapshots WHERE exam_id=? AND runtime_sha256=?",&[&exam,&hash])?;
    items
        .first()
        .and_then(|r| r["runtime_source_json"].as_str())
        .ok_or_else(|| "CONTENT_UNAVAILABLE:缺少学生锁定版本的内容快照".to_string())
        .and_then(|s| serde_json::from_str(s).map_err(|e| e.to_string()))
}
pub fn question(conn: &Connection, exam: &str, hash: &str, id: &str) -> CommandResult<Value> {
    let source = content(conn, exam, hash)?;
    let slot = source["answerSlots"]
        .get(id)
        .cloned()
        .or_else(|| source["answerKey"].get(id).map(|_| json!({"slotId":id})))
        .ok_or("QUESTION_NOT_FOUND:锁定版本中不存在此计分单元")?;
    let legacy = source["questionGroups"].clone();
    let task_groups = source.get("taskGroups").unwrap_or(&legacy);
    fn references_slot(value: &Value, id: &str) -> bool {
        match value {
            Value::Object(map) => map.iter().any(|(key, v)| {
                (matches!(key.as_str(), "slotId" | "questionId") && v.as_str() == Some(id))
                    || (matches!(key.as_str(), "slotIds" | "questionIds")
                        && v.as_array()
                            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(id))))
                    || references_slot(v, id)
            }),
            Value::Array(a) => a.iter().any(|v| references_slot(v, id)),
            _ => false,
        }
    }
    fn collect(value: &Value, key: &str, out: &mut Vec<Value>) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    if k == key {
                        out.push(v.clone())
                    } else {
                        collect(v, key, out)
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    collect(v, key, out)
                }
            }
            _ => {}
        }
    }
    let groups: Vec<Value> = task_groups
        .as_array()
        .into_iter()
        .flatten()
        .filter(|g| references_slot(g, id))
        .cloned()
        .collect();
    let mut options = Vec::new();
    let mut text = Vec::new();
    let mut html = Vec::new();
    for group in &groups {
        collect(group, "options", &mut options);
        collect(group, "text", &mut text);
        collect(group, "bodyHtml", &mut html);
    }
    let prompt_text = text
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    Ok(
        json!({"n":1,"examId":exam,"runtimeSha256":hash,"questionId":id,"slot":slot,"promptText":prompt_text,"promptHtml":html,"options":options,"optionBanks":source["optionBanks"],"taskGroups":groups,"answer":source["answerKey"][id],"explanation":source["explanations"][id],"explanationAvailable":!source["explanations"][id].is_null()}),
    )
}
