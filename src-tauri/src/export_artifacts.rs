use crate::util::{safe_path_segment, validate_path_segment};
use crate::CommandResult;
use chrono::Utc;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub(crate) struct ReadingAssetBundle {
    pub exam_id: String,
    pub source: Value,
    pub wrapper_js: String,
    pub manifest_js: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PackSource {
    pub fallback_exam_id: String,
    pub source: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct PackEntryBundle {
    pub entries: Vec<(String, Vec<u8>)>,
    pub pack_manifest: Value,
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
    Ok(format!(
        "window.__READING_EXAM_MANIFEST__ = {};\n",
        serde_json::to_string_pretty(&Value::Object(manifest)).map_err(|error| error.to_string())?
    ))
}

pub(crate) fn build_pack_manifest(input: &Value, sources: &[Value]) -> CommandResult<Value> {
    let exams = sources
        .iter()
        .enumerate()
        .map(|(index, source)| -> CommandResult<Value> {
            let exam_id = safe_exam_id(source)?;
            Ok(json!({
                "order": index + 1,
                "examId": exam_id,
                "title": source.pointer("/meta/title").and_then(Value::as_str).unwrap_or("Untitled Reading"),
                "category": source.pointer("/meta/category").and_then(Value::as_str).unwrap_or("P1"),
                "frequency": source.pointer("/meta/frequency").and_then(Value::as_str).unwrap_or("medium"),
                "script": format!("reading-exams/{}.js", exam_id)
            }))
        })
        .collect::<CommandResult<Vec<_>>>()?;

    Ok(json!({
        "schemaVersion": "ReadingExamPackV1",
        "packId": input.get("packId").and_then(Value::as_str).unwrap_or("pack-local"),
        "version": input.get("version").and_then(Value::as_str).unwrap_or("0.1.0"),
        "institution": input.get("institution").and_then(Value::as_str).unwrap_or("internal"),
        "description": input.get("description").and_then(Value::as_str).unwrap_or(""),
        "validFrom": input.get("validFrom").cloned().unwrap_or(Value::Null),
        "validTo": input.get("validTo").cloned().unwrap_or(Value::Null),
        "generatedAt": Utc::now().to_rfc3339(),
        "assetsRoot": "reading-exams",
        "exams": exams
    }))
}

pub(crate) fn build_pack_entry_bundle(
    input: &Value,
    sources: &[PackSource],
) -> CommandResult<PackEntryBundle> {
    if sources.is_empty() {
        return Err("pack_requires_at_least_one_source".to_string());
    }

    let mut normalized_sources = Vec::with_capacity(sources.len());
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for item in sources {
        let exam_id = item
            .source
            .get("examId")
            .and_then(Value::as_str)
            .map(|_| safe_exam_id(&item.source))
            .unwrap_or_else(|| {
                Ok(safe_path_segment("exam_id", &item.fallback_exam_id)?.to_string())
            })?;
        let mut source = item.source.clone();
        if source.get("examId").and_then(Value::as_str).is_none() {
            source
                .as_object_mut()
                .ok_or_else(|| "pack_source_must_be_object".to_string())?
                .insert("examId".to_string(), Value::String(exam_id.clone()));
        }
        let wrapper = build_wrapper(&source)?;
        entries.push((
            format!("reading-exams/{}.js", exam_id),
            wrapper.into_bytes(),
        ));
        normalized_sources.push(source);
    }

    let manifest_js = build_manifest(&normalized_sources)?;
    let pack_manifest = build_pack_manifest(input, &normalized_sources)?;
    let pack_json =
        serde_json::to_string_pretty(&pack_manifest).map_err(|error| error.to_string())?;
    entries.insert(
        0,
        (
            "reading-exams/manifest.js".to_string(),
            manifest_js.into_bytes(),
        ),
    );
    entries.insert(0, ("pack.json".to_string(), pack_json.into_bytes()));

    Ok(PackEntryBundle {
        entries,
        pack_manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_entry_bundle_normalizes_missing_exam_id_to_fallback() {
        let input = json!({
            "packId": "pack-fixture",
            "jobIds": ["job-fixture"]
        });
        let source = json!({
            "schemaVersion": "ReadingExamSourceV1",
            "meta": {"title": "Fixture", "category": "P1"},
            "questionGroups": [],
            "answerKey": {},
            "questionOrder": [],
            "questionDisplayMap": {}
        });

        let bundle = build_pack_entry_bundle(
            &input,
            &[PackSource {
                fallback_exam_id: "job-fixture".to_string(),
                source,
            }],
        )
        .unwrap();

        let files = bundle
            .entries
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            files,
            vec![
                "pack.json",
                "reading-exams/manifest.js",
                "reading-exams/job-fixture.js"
            ]
        );
        assert_eq!(
            bundle
                .pack_manifest
                .pointer("/exams/0/examId")
                .and_then(Value::as_str),
            Some("job-fixture")
        );
        let wrapper = String::from_utf8(bundle.entries[2].1.clone()).unwrap();
        assert!(wrapper.contains("\"job-fixture\""));
    }
}
