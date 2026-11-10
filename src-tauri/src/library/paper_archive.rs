use crate::analytics::records::query;
use crate::CommandResult;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};
use std::path::Path;

pub fn store_content(
    conn: &Connection,
    destination: &Path,
    exam: &str,
    hash: &str,
    source: &Value,
) -> CommandResult<()> {
    let source = crate::export_artifacts::student_package_source(source);
    let bytes = crate::schema::common::canonical_json_bytes_js(&source);
    if crate::analytics::import::sha256_hex(&bytes) != hash {
        return Err("CONTENT_HASH_MISMATCH: 发布内容与运行时指纹不一致".into());
    }
    conn.execute("INSERT OR IGNORE INTO item_content_snapshots(exam_id,runtime_sha256,runtime_source_json,created_at) VALUES(?1,?2,?3,?4)",params![exam,hash,source.to_string(),chrono::Utc::now().to_rfc3339()]).map_err(|e|e.to_string())?;
    conn.execute(
        "INSERT OR IGNORE INTO paper_archive_destinations(destination) VALUES(?1)",
        [destination.to_string_lossy().as_ref()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn record_pointer(
    conn: &Connection,
    destination: &Path,
    pointer: &Value,
    outcomes: &[Value],
) -> CommandResult<()> {
    record_paper(conn, destination, pointer, outcomes, true, None)
}
fn record_paper(
    conn: &Connection,
    destination: &Path,
    pointer: &Value,
    outcomes: &[Value],
    current: bool,
    superseded: Option<&str>,
) -> CommandResult<()> {
    if pointer["schemaVersion"] != "CurrentPaperV1" {
        return Err("PAPER_INVALID: 试卷指针版本无效".into());
    }
    let id = pointer["paperId"]
        .as_str()
        .ok_or("PAPER_INVALID: 缺少试卷标识")?;
    let at = pointer["publishedAt"]
        .as_str()
        .ok_or("PAPER_INVALID: 缺少发布时间")?;
    let batch = pointer["batchId"]
        .as_str()
        .ok_or("PAPER_INVALID: 缺少发布批次")?;
    let dest = destination.to_string_lossy();
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if current {
        tx.execute("UPDATE papers SET status='superseded',superseded_at=COALESCE(superseded_at,?2) WHERE destination=?1 AND paper_id<>?3 AND status='current'",params![dest,at,id]).map_err(|e|e.to_string())?;
    }
    tx.execute("INSERT INTO papers(paper_id,batch_id,published_at,destination,pointer_json,status,superseded_at,archive_state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(paper_id) DO UPDATE SET status=excluded.status,superseded_at=COALESCE(excluded.superseded_at,papers.superseded_at),pointer_json=excluded.pointer_json,archive_state=CASE WHEN papers.archive_state='confirmed' THEN 'confirmed' ELSE excluded.archive_state END",params![id,batch,at,dest,pointer.to_string(),if current{"current"}else{"superseded"},superseded,if current{"confirmed"}else{"unconfirmed"}]).map_err(|e|e.to_string())?;
    for modality in ["reading", "listening"] {
        for part in pointer[modality]
            .as_array()
            .ok_or("PAPER_INVALID: 缺少试卷条目")?
        {
            let exam = part["examId"]
                .as_str()
                .ok_or("PAPER_INVALID: 缺少题目标识")?;
            let outcome = outcomes.iter().find(|v| v["examId"] == exam);
            tx.execute("INSERT INTO paper_items(paper_id,modality,part,exam_id,library_item_id,edit_version,title,script_sha256,runtime_sha256,asset_manifest_sha256) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(paper_id,modality,part,exam_id) DO UPDATE SET library_item_id=COALESCE(paper_items.library_item_id,excluded.library_item_id),edit_version=COALESCE(paper_items.edit_version,excluded.edit_version)",params![id,modality,part["part"].as_str(),exam,outcome.and_then(|v|v["itemId"].as_str()),outcome.and_then(|v|v["editVersion"].as_i64()),part["title"].as_str().unwrap_or(exam),part["scriptSha256"].as_str(),part["runtimeSha256"].as_str(),part["entry"]["checksums"]["assetManifestSha256"].as_str()]).map_err(|e|e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

/// Reconcile only pointers proven live on disk. Isolated history files are not publication evidence.
pub fn recover(conn: &Connection) -> CommandResult<Value> {
    let destinations = query(
        conn,
        "SELECT destination FROM paper_archive_destinations UNION SELECT DISTINCT destination FROM publish_records_v2",
        &[],
    )?;
    let mut recovered = 0;
    let mut errors = Vec::new();
    for row in destinations {
        let normalized = crate::export_nas_library::normalize_nas_library_root(Path::new(
            row["destination"].as_str().unwrap_or(""),
        ));
        let destination = normalized.as_path();
        let reading = crate::export_nas_library::nas_reading_exams_dir(destination);
        match crate::util::read_json::<Value>(&reading.join("current-paper.json")) {
            Ok(pointer) => {
                match record_pointer(conn, destination, &pointer, &[]) {
                    Ok(()) => recovered += 1,
                    Err(e) => {
                        errors.push(e);
                        continue;
                    }
                }
                recover_contents(conn, destination, &reading, &pointer, &mut errors);
                recover_history(
                    conn,
                    destination,
                    &reading,
                    &pointer,
                    &mut recovered,
                    &mut errors,
                );
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    Ok(json!({"recovered":recovered,"errors":errors}))
}

fn recover_history(
    conn: &Connection,
    destination: &Path,
    reading: &Path,
    current: &Value,
    count: &mut usize,
    errors: &mut Vec<String>,
) {
    let history = destination.join(".publish-control/papers");
    let Ok(entries) = std::fs::read_dir(history) else {
        return;
    };
    for file in entries.flatten() {
        if file.path().extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(old) = crate::util::read_json::<Value>(&file.path()) else {
            continue;
        };
        if old["paperId"] == current["paperId"] {
            continue;
        }
        let Some(batch) = old["batchId"].as_str() else {
            continue;
        };
        if crate::util::validate_path_segment("batchId", batch).is_err() {
            continue;
        }
        let proof = destination
            .join(".publish-control/superseded")
            .join(format!("{batch}.json"));
        let proof = crate::util::read_json::<Value>(&proof).unwrap_or(Value::Null);
        let at = proof["supersededAt"]
            .as_str()
            .filter(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok());
        match record_paper(conn, destination, &old, &[], false, at) {
            Ok(()) => {
                *count += 1;
                recover_contents(conn, destination, reading, &old, errors)
            }
            Err(e) => errors.push(e),
        }
    }
}
fn recover_contents(
    conn: &Connection,
    destination: &Path,
    reading: &Path,
    pointer: &Value,
    errors: &mut Vec<String>,
) {
    for kind in ["reading", "listening"] {
        for part in pointer[kind].as_array().into_iter().flatten() {
            let Some(exam) = part["examId"].as_str() else {
                continue;
            };
            let Some(hash) = part["runtimeSha256"].as_str() else {
                continue;
            };
            if content(conn, exam, hash).ok().flatten().is_some() {
                continue;
            }
            let attempt = (|| -> CommandResult<()> {
                let script = part["script"]
                    .as_str()
                    .ok_or("历史条目缺少脚本路径")?
                    .trim_start_matches("./");
                let relative = Path::new(script);
                let batch = pointer["batchId"].as_str().ok_or("历史条目缺少批次")?;
                if relative
                    .components()
                    .any(|p| !matches!(p, std::path::Component::Normal(_)))
                    || !script.starts_with(&format!("releases/{batch}/"))
                {
                    return Err("历史脚本路径不在锁定批次内".into());
                }
                let path = reading.join(relative);
                let real = path
                    .canonicalize()
                    .map_err(|e| format!("历史内容已不可用：{exam}: {e}"))?;
                if !real.starts_with(reading.canonicalize().map_err(|e| e.to_string())?) {
                    return Err("历史脚本真实路径越界".into());
                }
                let bytes = std::fs::read(real).map_err(|e| e.to_string())?;
                if crate::analytics::import::sha256_hex(&bytes)
                    != part["scriptSha256"].as_str().unwrap_or("")
                {
                    return Err("历史脚本校验和不符".into());
                }
                let source = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
                let rest = source
                    .split_once("global.__READING_EXAM_DATA__.register(")
                    .ok_or("历史脚本不是已知的注册格式")?
                    .1;
                let mut first = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
                let key = first
                    .next()
                    .ok_or("历史脚本缺少注册标识")?
                    .map_err(|e| e.to_string())?;
                if key != exam {
                    return Err("历史脚本注册标识不符".into());
                }
                let rest = rest[first.byte_offset()..]
                    .trim_start()
                    .strip_prefix(',')
                    .ok_or("历史脚本缺少注册内容")?
                    .trim_start();
                let payload = serde_json::Deserializer::from_str(rest)
                    .into_iter::<Value>()
                    .next()
                    .ok_or("历史脚本内容缺失")?
                    .map_err(|e| e.to_string())?;
                store_content(conn, destination, exam, hash, &payload)
            })();
            if let Err(e) = attempt {
                errors.push(e);
            }
        }
    }
}

pub fn list(conn: &Connection) -> CommandResult<Value> {
    let recovery = recover(conn)?;
    let items = query(
        conn,
        "SELECT * FROM papers ORDER BY published_at DESC,paper_id",
        &[],
    )?;
    Ok(json!({"n":items.len(),"papers":items,"recovery":recovery}))
}
pub fn get(conn: &Connection, id: &str) -> CommandResult<Value> {
    let paper = query(conn, "SELECT * FROM papers WHERE paper_id=?1", &[&id])?
        .into_iter()
        .next()
        .ok_or("PAPER_NOT_FOUND: 试卷档案不存在")?;
    let items=query(conn,"SELECT p.*,s.exam_id IS NOT NULL AS content_available FROM paper_items p LEFT JOIN item_content_snapshots s ON s.exam_id=p.exam_id AND s.runtime_sha256=p.runtime_sha256 WHERE paper_id=?1 ORDER BY modality,part",&[&id])?;
    Ok(json!({"paper":paper,"items":items,"n":items.len()}))
}
pub fn content(conn: &Connection, exam: &str, hash: &str) -> CommandResult<Option<Value>> {
    let raw:Option<String>=conn.query_row("SELECT runtime_source_json FROM item_content_snapshots WHERE exam_id=?1 AND runtime_sha256=?2",params![exam,hash],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    raw.map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        .transpose()
}
pub fn question(conn: &Connection, exam: &str, hash: &str, qid: &str) -> CommandResult<Value> {
    let Some(source) = content(conn, exam, hash)? else {
        return Ok(
            json!({"examId":exam,"runtimeSha256":hash,"questionId":qid,"contentUnavailable":true,"reason":"锁定版本内容快照缺失","n":0}),
        );
    };
    let slot = source["answerSlots"]
        .get(qid)
        .cloned()
        .ok_or("QUESTION_NOT_FOUND: 锁定版本中不存在该计分单元")?;
    let groups: Vec<Value> = source["taskGroups"]
        .as_array()
        .map(|groups| {
            groups
                .iter()
                .filter(|g| {
                    g["responseGroups"]
                        .as_array()
                        .map(|rs| {
                            rs.iter().any(|r| {
                                r["slotIds"]
                                    .as_array()
                                    .map(|ids| ids.iter().any(|i| i == qid))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let options: Vec<Value> = groups
        .iter()
        .flat_map(|g| {
            let mut values = g["optionBank"]["options"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for response in g["responseGroups"].as_array().into_iter().flatten() {
                values.extend(response["options"].as_array().cloned().unwrap_or_default());
            }
            values
        })
        .collect();
    let explanation = source["explanations"]
        .get(qid)
        .or_else(|| slot.get("explanation"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(
        json!({"examId":exam,"runtimeSha256":hash,"questionId":qid,"slot":slot,"questionText":content_text(&json!(groups)),"options":options,"explanation":explanation,"explanationAvailable":!explanation.is_null(),"taskGroups":groups,"correctAnswer":source["answerKey"][qid],"passage":source["passage"],"passageText":content_text(&source["passage"]),"contentUnavailable":false,"n":1}),
    )
}

fn content_text(value: &Value) -> String {
    match value {
        Value::Array(xs) => xs
            .iter()
            .map(content_text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(map) => map
            .iter()
            .filter_map(|(key, v)| {
                if ["text", "prompt", "html"].contains(&key.as_str()) {
                    v.as_str().map(str::to_string)
                } else if v.is_array() || v.is_object() {
                    Some(content_text(v))
                } else {
                    None
                }
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(exam: &str) -> Value {
        json!({"schemaVersion":"ReadingExamSourceV2","examId":exam,"passage":{"content":[{"type":"text","text":"Locked original paragraph"}]},"answerSlots":{"q1":{"slotId":"q1"},"q2":{"slotId":"q2"}},"answerKey":{"q1":"A","q2":"B"},"taskGroups":[{"taskId":"one","instructions":[{"type":"text","text":"Only first question"}],"responseGroups":[{"slotIds":["q1"]}]},{"taskId":"two","instructions":[{"type":"text","text":"Second question"}],"responseGroups":[{"slotIds":["q2"]}]}]})
    }
    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(include_str!("paper_archive.sql")).unwrap();
        c.execute_batch("CREATE TABLE publish_records_v2(destination TEXT)")
            .unwrap();
        c
    }
    #[test]
    fn context_is_frozen_by_hash_and_includes_only_the_owning_group() {
        let c = db();
        let source = source("exam");
        let hash =
            crate::analytics::import::sha256_hex(&crate::schema::common::canonical_json_bytes_js(
                &crate::export_artifacts::student_package_source(&source),
            ));
        store_content(&c, Path::new("F:/fixture"), "exam", &hash, &source).unwrap();
        let context = question(&c, "exam", &hash, "q1").unwrap();
        assert_eq!(context["taskGroups"].as_array().unwrap().len(), 1);
        assert!(context["questionText"]
            .as_str()
            .unwrap()
            .contains("Only first"));
        assert!(!context["questionText"].as_str().unwrap().contains("Second"));
        assert_eq!(
            question(&c, "exam", "missing", "q1").unwrap()["contentUnavailable"],
            true
        );
        assert!(store_content(&c, Path::new("F:/fixture"), "exam", "incorrect", &source).is_err());
    }
    #[test]
    fn recovery_is_idempotent_and_does_not_assert_unproven_history_was_published() {
        let c = db();
        let root = std::env::temp_dir().join(format!("archive-recovery-{}", uuid::Uuid::new_v4()));
        let reading = crate::export_nas_library::nas_reading_exams_dir(&root);
        std::fs::create_dir_all(&reading).unwrap();
        let mut parts = Vec::new();
        for n in 1..=3 {
            let exam = format!("exam{n}");
            let source = source(&exam);
            let wrapper = crate::export_artifacts::build_wrapper(&source).unwrap();
            let hash = crate::analytics::import::sha256_hex(
                &crate::schema::common::canonical_json_bytes_js(
                    &crate::export_artifacts::student_package_source(&source),
                ),
            );
            let script = format!("releases/batch/{exam}.js");
            let path = reading.join(&script);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, &wrapper).unwrap();
            parts.push(json!({"part":format!("P{n}"),"examId":exam,"title":exam,"runtimeSha256":hash,"scriptSha256":crate::analytics::import::sha256_hex(wrapper.as_bytes()),"script":script}));
        }
        let pointer = json!({"schemaVersion":"CurrentPaperV1","paperId":"current","batchId":"batch","publishedAt":"2026-11-08T00:00:00Z","reading":parts,"listening":[]});
        std::fs::write(reading.join("current-paper.json"), pointer.to_string()).unwrap();
        let history = root.join(".publish-control/papers");
        std::fs::create_dir_all(&history).unwrap();
        let mut orphan = pointer.clone();
        orphan["paperId"] = json!("orphan");
        std::fs::write(history.join("orphan.json"), orphan.to_string()).unwrap();
        c.execute(
            "INSERT INTO paper_archive_destinations(destination) VALUES(?1)",
            [root.to_string_lossy().as_ref()],
        )
        .unwrap();
        recover(&c).unwrap();
        recover(&c).unwrap();
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM papers", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            c.query_row(
                "SELECT archive_state FROM papers WHERE paper_id='orphan'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "unconfirmed"
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM item_content_snapshots", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            3
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
