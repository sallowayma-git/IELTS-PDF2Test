//! 考试状态信标（契约 §2）：`<submissionsRoot>/status/<terminalId>/<attemptId>.json`。
//!
//! 只用于「已签到 / 进行中 / 已交卷 / 作废」计数；与快照（已导入 / 有效计分）分开显示，互不推断。
//! 信标损坏只记入 errors，不影响其他信标与导入。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

pub const ATTEMPT_STATUS_SCHEMA_VERSION: &str = "AttemptStatusV1";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttemptBeacon {
    pub terminal_id: String,
    pub attempt_id: String,
    pub batch_id: String,
    pub paper_id: String,
    pub candidate_id: String,
    pub candidate_name: String,
    /// 原始信标状态：checked_in | in_progress | submitted | failed。
    pub status: String,
    pub updated_at: String,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeaconScan {
    pub beacons: Vec<AttemptBeacon>,
    pub errors: Vec<String>,
}

const KNOWN_STATUSES: [&str; 4] = ["checked_in", "in_progress", "submitted", "failed"];

pub fn read_attempt_status(submissions_root: &Path) -> BeaconScan {
    let mut scan = BeaconScan::default();
    let status_dir = submissions_root.join("status");
    let Ok(terminals) = fs::read_dir(&status_dir) else {
        return scan;
    };
    // 同一 attempt 只保留 updatedAt 最新的一条（正常情况下每个 attempt 一个文件）。
    let mut latest: HashMap<String, AttemptBeacon> = HashMap::new();
    for terminal in terminals.flatten() {
        let terminal_path = terminal.path();
        if !terminal_path.is_dir() {
            continue;
        }
        let Ok(files) = fs::read_dir(&terminal_path) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            match parse_beacon(&path) {
                Ok(beacon) => {
                    let keep = latest
                        .get(&beacon.attempt_id)
                        .map(|existing| beacon.updated_at > existing.updated_at)
                        .unwrap_or(true);
                    if keep {
                        latest.insert(beacon.attempt_id.clone(), beacon);
                    }
                }
                Err(error) => scan.errors.push(format!("{}：{error}", name)),
            }
        }
    }
    let mut beacons: Vec<AttemptBeacon> = latest.into_values().collect();
    beacons.sort_by(|a, b| a.batch_id.cmp(&b.batch_id).then(a.attempt_id.cmp(&b.attempt_id)));
    scan.beacons = beacons;
    scan
}

fn parse_beacon(path: &Path) -> Result<AttemptBeacon, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("不可读：{error}"))?;
    let value: Value = serde_json::from_str(&text).map_err(|error| format!("解析失败：{error}"))?;
    if value.get("schemaVersion").and_then(Value::as_str) != Some(ATTEMPT_STATUS_SCHEMA_VERSION) {
        return Err("schemaVersion 不是 AttemptStatusV1".into());
    }
    let text_of = |key: &str| -> Result<String, String> {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("缺少 {key}"))
    };
    let status = text_of("status")?;
    if !KNOWN_STATUSES.contains(&status.as_str()) {
        return Err(format!("未知 status：{status}"));
    }
    Ok(AttemptBeacon {
        terminal_id: text_of("terminalId")?,
        attempt_id: text_of("attemptId")?,
        batch_id: text_of("batchId")?,
        paper_id: value
            .get("paperId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        candidate_id: text_of("candidateId")?,
        candidate_name: text_of("candidateName")?,
        status,
        updated_at: text_of("updatedAt")?,
    })
}
