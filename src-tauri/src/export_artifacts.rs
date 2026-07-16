use crate::util::validate_path_segment;
use crate::CommandResult;
use chrono::{DateTime, FixedOffset, Local, SecondsFormat};
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub(crate) struct ReadingAssetBundle {
    pub exam_id: String,
    pub source: Value,
    pub wrapper_js: String,
    pub manifest_js: String,
}
pub(crate) fn build_reading_asset_bundle(source: &Value) -> CommandResult<ReadingAssetBundle> {
    let exam_id = safe_exam_id(source)?;
    let wrapper_js = build_wrapper(source)?;
    let manifest_js = build_manifest(std::slice::from_ref(source))?;
    Ok(ReadingAssetBundle {
        exam_id,
        source: source.clone(),
        wrapper_js,
        manifest_js,
    })
}

pub(crate) fn safe_exam_id(source: &Value) -> CommandResult<String> {
    let exam_id = source
        .get("examId")
        .and_then(Value::as_str)
        .unwrap_or("local-authoring-exam");
    validate_path_segment("exam_id", exam_id)?;
    Ok(exam_id.to_string())
}

pub(crate) fn build_wrapper(source: &Value) -> CommandResult<String> {
    let exam_id = safe_exam_id(source)?;
    let exam_id_json = serde_json::to_string(&exam_id).map_err(|error| error.to_string())?;
    let source_json = serde_json::to_string_pretty(source).map_err(|error| error.to_string())?;
    Ok(format!("(function registerReadingExamData(global) {{\n  'use strict';\n  if (!global.__READING_EXAM_DATA__ || typeof global.__READING_EXAM_DATA__.register !== \"function\") {{\n    throw new Error(\"reading_exam_registry_missing\");\n  }}\n  global.__READING_EXAM_DATA__.register({}, {});\n}})(typeof window !== \"undefined\" ? window : globalThis);\n", exam_id_json, source_json))
}

pub(crate) fn build_manifest(sources: &[Value]) -> CommandResult<String> {
    let mut manifest = serde_json::Map::new();
    for source in sources {
        let exam_id = safe_exam_id(source)?;
        manifest.insert(exam_id.to_string(), json!({
            "examId": exam_id,
            "dataKey": exam_id,
            "script": format!("./{}.js", exam_id),
            "title": source.pointer("/meta/title").and_then(Value::as_str).unwrap_or("Untitled Reading"),
            "category": source.pointer("/meta/category").and_then(Value::as_str).unwrap_or("P1")
        }));
    }
    let generated_at = Local::now().fixed_offset();
    manifest.insert(
        "_meta".to_string(),
        build_manifest_metadata(&generated_at, manifest.len()),
    );
    Ok(format!(
        "window.__READING_EXAM_MANIFEST__ = {};\n",
        serde_json::to_string_pretty(&Value::Object(manifest)).map_err(|error| error.to_string())?
    ))
}

fn build_manifest_metadata(generated_at: &DateTime<FixedOffset>, asset_count: usize) -> Value {
    json!({
        "schemaVersion": "ReadingExamManifestV1",
        "batchId": generated_at.format("BATCH-%Y%m%d-%H%M%S-%3f").to_string(),
        "generatedAt": generated_at.to_rfc3339_opts(SecondsFormat::Millis, false),
        "assetCount": asset_count
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    #[test]
    fn manifest_metadata_uses_one_local_timestamp_and_counts_assets() {
        let generated_at = FixedOffset::east_opt(8 * 60 * 60)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 12, 23, 45, 6)
            .single()
            .unwrap()
            .with_nanosecond(789_000_000)
            .unwrap();

        let metadata = build_manifest_metadata(&generated_at, 3);

        assert_eq!(
            metadata,
            json!({
                "schemaVersion": "ReadingExamManifestV1",
                "batchId": "BATCH-20260712-234506-789",
                "generatedAt": "2026-07-12T23:45:06.789+08:00",
                "assetCount": 3
            })
        );
    }

    #[test]
    fn manifest_keeps_exam_entries_unchanged_and_adds_metadata() {
        let source = json!({
            "examId": "reading-p1-001",
            "meta": {
                "title": "Reading fixture",
                "category": "P1"
            }
        });

        let manifest_js = build_manifest(&[source]).unwrap();
        let manifest_json = manifest_js
            .strip_prefix("window.__READING_EXAM_MANIFEST__ = ")
            .and_then(|value| value.strip_suffix(";\n"))
            .unwrap();
        let manifest: Value = serde_json::from_str(manifest_json).unwrap();

        assert_eq!(
            manifest.get("reading-p1-001"),
            Some(&json!({
                "examId": "reading-p1-001",
                "dataKey": "reading-p1-001",
                "script": "./reading-p1-001.js",
                "title": "Reading fixture",
                "category": "P1"
            }))
        );
        assert_eq!(
            manifest
                .pointer("/_meta/schemaVersion")
                .and_then(Value::as_str),
            Some("ReadingExamManifestV1")
        );
        assert_eq!(
            manifest
                .pointer("/_meta/assetCount")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert!(manifest
            .pointer("/_meta/batchId")
            .and_then(Value::as_str)
            .is_some_and(|value| value.starts_with("BATCH-")));
        assert!(manifest
            .pointer("/_meta/generatedAt")
            .and_then(Value::as_str)
            .is_some_and(|value| DateTime::parse_from_rfc3339(value).is_ok()));
    }
}
