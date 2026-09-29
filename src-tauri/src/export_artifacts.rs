use crate::schema::common::canonical_json_bytes_js;
use crate::util::validate_path_segment;
use crate::CommandResult;
use chrono::{DateTime, FixedOffset, Local, SecondsFormat};
use serde_json::{json, Value};

/// 学生端题包 payload 里删除的编辑元数据键。
///
/// 学生端（`apps/student-exam`）与 NAS 服务端加载器都不读取这些字段：
/// `sourceAnchors`/`provenanceStatus`/`evidenceAnchors` 是编辑器回跳定位用的来源锚；
/// `instructionSignature`/`recognitionWarnings`/`reviewState`/`quality`/`displayRange`
/// 是识别与审校的编辑元数据。学生渲染与判分必需的字段（节点 `id`、`hostNodeId`、
/// `cue.confidence`、`answerKey`、`audit` 等）一律保留。
const STUDENT_PACKAGE_STRIPPED_KEYS: &[&str] = &[
    "sourceAnchors",
    "provenanceStatus",
    "evidenceAnchors",
    "instructionSignature",
    "recognitionWarnings",
    "reviewState",
    "quality",
    "displayRange",
];

/// 正式稿投影成学生端题包 payload：剥离编辑元数据，保留学生运行时读取的一切。
pub(crate) fn student_package_source(source: &Value) -> Value {
    fn strip(value: &Value) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.iter().map(strip).collect()),
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .filter(|(key, _)| !STUDENT_PACKAGE_STRIPPED_KEYS.contains(&key.as_str()))
                    .map(|(key, child)| (key.clone(), strip(child)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    strip(source)
}

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
    // 学生端用 JSON.stringify 复算 runtimeSha256：嵌入文本必须是 canonical
    // （键排序、紧凑）形态，见 schema::common::canonical_json_bytes_js。
    let source_json = String::from_utf8(canonical_json_bytes_js(&student_package_source(source)))
        .map_err(|error| error.to_string())?;
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

    fn anchor() -> Value {
        json!({
            "sourceFileId": "source-pdf-1",
            "pageIndex": 0,
            "nodeIds": ["region-1", "line-1"],
            "bbox": {"x": 1.0, "y": 2.0, "width": 3.0, "height": 4.0, "unit": "pt", "origin": "top-left", "pageRotation": 0.0},
            "extractionMode": "pdf_native",
            "sourceHash": "a".repeat(64)
        })
    }

    fn student_package_sample() -> Value {
        json!({
            "schemaVersion": "ReadingExamSourceV2",
            "examId": "package-projection",
            "meta": {"title": "Projection", "language": "en", "category": "P1"},
            "assets": {"examId": "package-projection", "assets": []},
            "passage": {"content": [{
                "id": "p1",
                "type": "paragraph",
                "provenanceStatus": "source",
                "sourceAnchors": [anchor()],
                "children": [{"id": "t1", "type": "text", "provenanceStatus": "source", "sourceAnchors": [anchor()], "text": "Passage text."}]
            }]},
            "taskGroups": [{
                "taskId": "task-1",
                "taskType": "yes_no_not_given",
                "displayRange": "Q1-2",
                "instructions": [],
                "instructionSignature": {"normalizedText": "Questions 1-2", "taskType": "yes_no_not_given", "expectedQuestionNumbers": [1, 2], "expectedSlotCount": 2, "evidenceAnchors": [anchor()], "confidence": 0.9},
                "recognitionWarnings": ["warn-1"],
                "optionBank": {"optionBankId": "bank-1", "scope": "task_group", "options": [
                    {"optionId": "o-yes", "label": "YES", "content": [{"id": "c1", "type": "text", "provenanceStatus": "source", "sourceAnchors": [], "text": "YES"}], "sourceAnchors": []}
                ], "allowReuse": false, "sourceAnchors": []},
                "responseGroups": [{"responseGroupId": "rg-1", "kind": "choice", "prompt": [{"id": "prompt-1", "type": "paragraph", "provenanceStatus": "source", "sourceAnchors": [anchor()], "children": []}], "slotIds": ["q1"], "cardinality": {"min": 1, "max": 1, "exact": 1}, "assignment": "per_slot", "scoringPolicy": "per_slot_binary", "duplicatePolicy": "reject_submission", "allowOptionReuse": false, "sourceAnchors": []}],
                "quality": {"score": 0.9, "sourceCoverage": 1.0, "hardFailures": []},
                "reviewState": "confirmed",
                "sourceAnchors": [anchor()]
            }],
            "answerSlots": {"q1": {"slotId": "q1", "questionNumber": 1, "displayLabel": "1", "hostNodeId": "prompt-1", "hostType": "prompt", "interaction": "select", "participation": "scoring", "confidence": 0.9, "provenanceStatus": "user_edited", "sourceAnchors": [anchor()]}},
            "answerKey": {"q1": {"kind": "option", "labels": ["YES"], "assignment": "per_slot"}},
            "questionOrder": ["q1"],
            "questionDisplayMap": {"q1": "1"},
            "audit": {"sourceSchemaVersion": "IeltsAuthoringIRV2", "sourceDocumentId": "doc-1", "sourceRevision": 3, "sourceRevisionKind": "user_edited"}
        })
    }

    fn student_heading_package_sample() -> Value {
        let mut source = student_package_sample();
        source["passage"]["paragraphMap"] = json!({"A": "p1"});
        source["passage"]["content"][0]["paragraphLabel"] = json!("A");
        source["taskGroups"][0]["taskType"] = json!("matching_headings");
        source["taskGroups"][0]["instructionSignature"]["taskType"] = json!("matching_headings");
        source["taskGroups"][0]["optionBank"]["options"] = json!([
            {"optionId": "o-i", "label": "i", "content": [{"id": "c1", "type": "text", "provenanceStatus": "source", "sourceAnchors": [], "text": "First heading"}], "sourceAnchors": []},
            {"optionId": "o-iv", "label": "iv", "content": [{"id": "c2", "type": "text", "provenanceStatus": "source", "sourceAnchors": [], "text": "Fourth heading"}], "sourceAnchors": []}
        ]);
        source["taskGroups"][0]["responseGroups"][0]["kind"] = json!("matching");
        source["taskGroups"][0]["responseGroups"][0]["optionBankRef"] = json!("bank-1");
        source["answerSlots"]["q1"]["hostNodeId"] = json!("p1");
        source["answerSlots"]["q1"]["hostType"] = json!("passage_paragraph");
        source["answerSlots"]["q1"]["interaction"] = json!("dragdrop");
        source["answerKey"]["q1"]["labels"] = json!(["i"]);
        source
    }

    #[test]
    fn student_package_wrapper_drops_editing_metadata_and_stays_compact() {
        let wrapper = build_wrapper(&student_package_sample()).unwrap();
        let marker = "__READING_EXAM_DATA__.register(";
        let marker_pos = wrapper.find(marker).expect("wrapper registers exam data");
        let payload_start = wrapper[marker_pos..].find('{').unwrap() + marker_pos;
        let payload_text = &wrapper[payload_start..];
        let payload: Value = serde_json::Deserializer::from_str(payload_text)
            .into_iter::<Value>()
            .next()
            .expect("wrapper embeds a JSON payload")
            .expect("wrapper payload is valid JSON");

        for stripped in [
            "sourceAnchors",
            "provenanceStatus",
            "instructionSignature",
            "evidenceAnchors",
            "recognitionWarnings",
            "reviewState",
            "quality",
            "displayRange",
        ] {
            assert!(
                serde_json::to_string(&payload)
                    .unwrap()
                    .contains(&format!("\"{stripped}\""))
                    == false,
                "student payload must not carry editing metadata key {stripped}"
            );
        }
        // 学生端按 JSON.stringify 复算 runtimeSha256：嵌入文本必须已是紧凑 canonical 形态。
        assert!(
            !wrapper.contains("\n  \""),
            "wrapper payload must be compact"
        );

        // 学生端与 NAS 服务端读取器真正用到的字段必须原样保留。
        assert_eq!(
            payload.pointer("/passage/content/0/children/0/text"),
            Some(&json!("Passage text."))
        );
        assert_eq!(payload.pointer("/passage/content/0/id"), Some(&json!("p1")));
        assert_eq!(
            payload.pointer("/taskGroups/0/optionBank/options/0/content/0/text"),
            Some(&json!("YES"))
        );
        assert_eq!(
            payload.pointer("/taskGroups/0/responseGroups/0/kind"),
            Some(&json!("choice"))
        );
        assert_eq!(
            payload.pointer("/taskGroups/0/responseGroups/0/prompt/0/id"),
            Some(&json!("prompt-1"))
        );
        assert_eq!(
            payload.pointer("/answerKey/q1/labels"),
            Some(&json!(["YES"]))
        );
        assert_eq!(payload.pointer("/audit/sourceRevision"), Some(&json!(3)));
    }

    #[test]
    fn student_package_wrapper_preserves_heading_targets_and_shared_bank() {
        let wrapper = build_wrapper(&student_heading_package_sample()).unwrap();
        let marker = "__READING_EXAM_DATA__.register(";
        let marker_pos = wrapper.find(marker).expect("wrapper registers exam data");
        let payload_start = wrapper[marker_pos..].find('{').unwrap() + marker_pos;
        let payload: Value = serde_json::Deserializer::from_str(&wrapper[payload_start..])
            .into_iter::<Value>()
            .next()
            .expect("wrapper embeds a JSON payload")
            .expect("wrapper payload is valid JSON");

        assert_eq!(
            payload.pointer("/passage/paragraphMap/A"),
            Some(&json!("p1"))
        );
        assert_eq!(
            payload.pointer("/passage/content/0/paragraphLabel"),
            Some(&json!("A"))
        );
        assert_eq!(
            payload.pointer("/answerSlots/q1/hostNodeId"),
            Some(&json!("p1"))
        );
        assert_eq!(
            payload.pointer("/answerSlots/q1/hostType"),
            Some(&json!("passage_paragraph"))
        );
        assert_eq!(
            payload.pointer("/answerSlots/q1/interaction"),
            Some(&json!("dragdrop"))
        );
        assert_eq!(
            payload.pointer("/taskGroups/0/responseGroups/0/optionBankRef"),
            Some(&json!("bank-1"))
        );
        assert_eq!(
            payload.pointer("/taskGroups/0/optionBank/options/0/label"),
            Some(&json!("i"))
        );
        assert_eq!(
            payload.pointer("/taskGroups/0/optionBank/options/0/content/0/text"),
            Some(&json!("First heading"))
        );
        assert_eq!(payload.pointer("/answerKey/q1/labels"), Some(&json!(["i"])));
    }

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
