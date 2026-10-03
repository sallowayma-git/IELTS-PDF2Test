//! Full local transcript retained across attempts; never becomes automatic editing authority.
use super::*;
use std::io::{BufRead, Read, Seek, SeekFrom, Write};

fn path(request: &RepairRunRequest<'_>) -> CommandResult<std::path::PathBuf> {
    crate::util::validate_path_segment("batch_id", request.batch_id)?;
    let job = crate::util::safe_job_dir(request.root, request.job_id)?;
    Ok(job
        .join("recognition")
        .join(format!("{}.repair-history.jsonl", request.batch_id)))
}

pub(super) fn append(
    request: &RepairRunRequest<'_>,
    packet: &Value,
    raw: &Value,
    result: &Value,
) -> CommandResult<()> {
    let path = path(request)?;
    std::fs::create_dir_all(path.parent().ok_or("CLOUD_HISTORY_PATH_INVALID")?)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let end = file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    if end > 0 {
        file.seek(SeekFrom::End(-1)).map_err(|e| e.to_string())?;
        let mut last = [0];
        file.read_exact(&mut last).map_err(|e| e.to_string())?;
        if last[0] != b'\n' {
            // Interrupted final writes are preserved separately; a new attempt cannot join
            // them to a fresh JSON record and corrupt the entire transcript.
            let mut offset = end;
            let mut tail = Vec::new();
            while offset > 0 {
                offset -= 1;
                file.seek(SeekFrom::Start(offset))
                    .map_err(|e| e.to_string())?;
                file.read_exact(&mut last).map_err(|e| e.to_string())?;
                if last[0] == b'\n' {
                    offset += 1;
                    break;
                }
                tail.push(last[0]);
            }
            tail.reverse();
            let partial =
                path.with_extension(format!("interrupted-{}", uuid::Uuid::new_v4().simple()));
            std::fs::write(partial, tail).map_err(|e| e.to_string())?;
            file.set_len(offset).map_err(|e| e.to_string())?;
        }
    }
    file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    let line = serde_json::to_vec(&json!({"repairRunId":request.repair_run_id,"packetId":packet["packetId"],
        "editVersion":packet.pointer("/draftSlice/editVersion").or_else(||packet.get("editVersion")),
        "context":packet,"response":raw,"result":result})).map_err(|e|e.to_string())?;
    file.write_all(&line)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_data())
        .map_err(|e| e.to_string())
}

pub(super) fn restore(request: &RepairRunRequest<'_>, packet: &Value) -> CommandResult<Vec<Value>> {
    let file = match std::fs::File::open(path(request)?) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut reader = std::io::BufReader::new(file);
    let mut results = Vec::new();
    let mut prior = 0usize;
    let mut stale = 0usize;
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        if reader
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?
            == 0
        {
            break;
        }
        if bytes.last() != Some(&b'\n') {
            break;
        } // Only a complete record is recoverable.
        let entry: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("CLOUD_HISTORY_CORRUPT:{e}"))?;
        if entry["packetId"] != packet["packetId"] {
            continue;
        }
        let mut result = entry["result"].clone();
        if !result.is_object() {
            continue;
        }
        prior += 1;
        let fresh = [
            "draftSlice",
            "differences",
            "candidateSlice",
            "cloudCandidateSlice",
            "localSnapshotSlice",
        ]
        .iter()
        .all(|field| entry["context"][*field] == packet[*field]);
        if !fresh {
            stale += 1;
            continue;
        }
        result["restoredFromLocalHistory"] = json!(true);
        results.push(result);
    }
    if stale > 0 {
        results.insert(0,json!({"status":"historical_summary","result":{"priorAttemptCount":prior,"staleFeedbackCount":stale}}));
    }
    Ok(results)
}
