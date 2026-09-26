pub mod cloud_repair_v1;
pub mod common;
pub mod content_doc_v2;
pub mod document_ir_v2;
pub mod ielts_authoring_v2;
pub mod listening_audio_probe_v1;
pub mod listening_runtime_v1;
pub mod migration_v1;
pub mod quality_report_v2;
pub mod recognition_v1;

pub use cloud_repair_v1::{
    CloudAuthoringCandidateV1, CloudCandidateUnresolvedRegionV1, CloudRepairToolCallV1,
    CloudRepairToolResultV1, CloudRepairToolStatusV1,
};
pub use content_doc_v2::{ContentDocV2, ContentNodeV2};
pub use document_ir_v2::DocumentIRV2;
pub use ielts_authoring_v2::IeltsAuthoringIRV2;
pub use listening_runtime_v1::{ListeningAttemptV1, ListeningExamSourceV1};
pub use quality_report_v2::QualityReportV2;
pub use recognition_v1::{RecognitionCandidateV1, RecognitionDecisionV1};

#[cfg(test)]
mod tests {
    use super::common::canonical_json_bytes;
    use super::{
        CloudAuthoringCandidateV1, CloudRepairToolCallV1, CloudRepairToolResultV1,
        CloudRepairToolStatusV1, ContentDocV2, DocumentIRV2, IeltsAuthoringIRV2, QualityReportV2,
    };
    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use serde_json::{json, Value};

    fn round_trip<T>(value: Value) -> T
    where
        T: DeserializeOwned + Serialize,
    {
        let typed = serde_json::from_value::<T>(value.clone()).expect("schema fixture must parse");
        let encoded = serde_json::to_value(&typed).expect("schema fixture must serialize");
        let decoded =
            serde_json::from_value::<T>(encoded.clone()).expect("schema fixture must parse twice");
        assert_eq!(
            encoded,
            serde_json::to_value(decoded).expect("decoded fixture must serialize")
        );
        typed
    }

    fn empty_quality() -> Value {
        json!({
            "schemaVersion": "QualityReportV2",
            "state": "ready",
            "documentScore": 1.0,
            "sourceCoverage": 1.0,
            "coverageLedger": [],
            "coverageStatus": {
                "physicalShadow": "available",
                "complete": true,
                "significantSourceNodeCount": 0,
                "explainedSourceNodeCount": 0,
                "unassignedSourceNodeIds": []
            },
            "compilerProbes": {
                "v2Runtime": {
                    "status": "passed",
                    "schemaVersion": "ReadingExamSourceV2",
                    "issueCodes": [],
                    "details": []
                },
                "v1Compatibility": {
                    "status": "passed",
                    "schemaVersion": "ReadingExamSourceV1",
                    "issueCodes": [],
                    "details": []
                }
            },
            "taskScores": {},
            "hardFailures": [],
            "issues": [],
            "metrics": {},
            "evaluatedAt": "2026-08-09T00:00:00Z",
            "evaluatorVersion": "phase1-pr01"
        })
    }

    fn source_anchor() -> Value {
        json!({
            "sourceFileId": "file-1",
            "pageIndex": 0,
            "nodeIds": ["line-1"],
            "extractionMode": "pdf_native",
            "sourceHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        })
    }

    #[test]
    fn document_ir_v2_round_trip_preserves_schema_version_and_empty_physical_layers() {
        let document: DocumentIRV2 = round_trip(json!({
            "schemaVersion": "DocumentIRV2",
            "documentId": "document-1",
            "jobId": "job-1",
            "sourceFiles": [{
                "sourceFileId": "file-1",
                "originalName": "sample.pdf",
                "mediaType": "application/pdf",
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "byteLength": 1,
                "role": "question_paper"
            }],
            "pages": [{
                "pageIndex": 0,
                "widthPt": 595,
                "heightPt": 842,
                "rotation": 0,
                "glyphs": [],
                "spans": [],
                "lines": [],
                "regions": [],
                "vectorPaths": [],
                "tables": [],
                "assetIds": [],
                "readingOrder": [],
                "quality": {
                    "classification": "empty",
                    "nativeCharacterCount": 0,
                    "unicodeErrorRatio": 0,
                    "duplicateTextRatio": 0,
                    "imageCoverageRatio": 0,
                    "textCoverageRatio": 0,
                    "rotationConfidence": 1,
                    "requiresOcrRegions": [],
                    "warnings": []
                }
            }],
            "assets": [],
            "coverageLedger": [],
            "parser": {
                "provider": "phase1-test",
                "providerVersion": "0.1.0",
                "extractionStartedAt": "2026-08-09T00:00:00Z",
                "extractionCompletedAt": "2026-08-09T00:00:01Z",
                "options": {},
                "warnings": []
            }
        }));
        assert!(document.is_supported_schema_version());
    }

    #[test]
    fn content_doc_v2_round_trip_preserves_typed_nodes_and_provenance() {
        let content: ContentDocV2 = round_trip(json!({
            "schemaVersion": "ContentDocV2",
            "documentId": "document-1",
            "sourceDocumentId": "document-1",
            "root": [{
                "type": "paragraph",
                "id": "paragraph-1",
                "sourceAnchors": [source_anchor()],
                "provenanceStatus": "source",
                "children": [{
                    "type": "text",
                    "id": "text-1",
                    "sourceAnchors": [source_anchor()],
                    "provenanceStatus": "source",
                    "text": "Question "
                }, {
                    "type": "answer_slot",
                    "id": "slot-node-1",
                    "sourceAnchors": [source_anchor()],
                    "provenanceStatus": "derived",
                    "slotId": "slot-1",
                    "displayLabel": "1",
                    "inline": true
                }]
            }]
        }));
        assert!(content.is_supported_schema_version());
    }

    #[test]
    fn authoring_ir_v2_round_trip_preserves_shared_slot_contract() {
        let authoring: IeltsAuthoringIRV2 = round_trip(json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "jobId": "job-1",
            "exam": {
                "examId": "exam-1",
                "title": "Schema fixture",
                "language": "en",
                "tags": [],
                "sourceFiles": [{"sourceFileId":"file-1","role":"question_paper"}]
            },
            "modality": "reading",
            "taskGroups": [],
            "answerSlots": {},
            "answerKey": {
                "slot-1": {"kind":"option","labels":["A","B"],"assignment":"unordered_set"}
            },
            "assets": [],
            "sourceDocumentId": "document-1",
            "quality": empty_quality(),
            "audit": {
                "revision": 0,
                "source": "auto_extract",
                "humanVerified": false,
                "llmUsed": false,
                "updatedAt": "2026-08-09T00:00:00Z",
                "notes": []
            }
        }));
        assert!(authoring.is_supported_schema_version());
    }

    #[test]
    fn quality_report_v2_round_trip_preserves_gate_fields() {
        let report: QualityReportV2 = round_trip(empty_quality());
        assert!(report.is_supported_schema_version());
    }

    #[test]
    fn canonical_json_is_deterministic_and_keeps_array_order() {
        let left = json!({"z": 1, "a": {"y": 2, "b": 3}, "items": [2, 1]});
        let right = json!({"items": [2, 1], "a": {"b": 3, "y": 2}, "z": 1});
        assert_eq!(
            canonical_json_bytes(&left).unwrap(),
            canonical_json_bytes(&right).unwrap()
        );
        assert_ne!(
            canonical_json_bytes(&left).unwrap(),
            canonical_json_bytes(&json!({"items":[1,2],"a":{"b":3,"y":2},"z":1})).unwrap()
        );
    }

    // ── 云端修复契约 ───────────────────────────────────────────────────

    fn golden_authoring_path() -> std::path::PathBuf {
        // `CARGO_MANIFEST_DIR` 指向 `src-tauri`（Windows 上可能带尾部分隔符），
        // golden 稿在仓库根 `fixtures/` 下。
        let manifest = env!("CARGO_MANIFEST_DIR").trim_end_matches(['\\', '/']);
        std::path::Path::new(manifest)
            .parent()
            .expect("src-tauri 必须有父目录")
            .join("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json")
    }

    fn golden_authoring_value() -> Value {
        let path = golden_authoring_path();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("读取 golden 稿失败 path={path:?} err={error}"));
        serde_json::from_str(&text).expect("golden 稿必须是合法 JSON")
    }

    fn cloud_candidate_fixture(authoring: Value) -> Value {
        json!({
            "schemaVersion": "CloudAuthoringCandidateV1",
            "batchId": "batch-1",
            "itemId": "job-1",
            "jobId": "job-1",
            "sourceFileId": "early-approaches-pdf",
            "sourceSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "baseEditVersion": 1,
            "generatedAt": "2026-09-17T00:00:00Z",
            "status": "succeeded",
            "authoring": authoring,
            "idMap": {"cloud-q14": "q14"},
            "unresolvedReferences": [],
            "unresolvedRegions": [{
                "sourceFileId": "early-approaches-pdf",
                "pageIndex": 3,
                "reason": "page_image_unavailable",
                "detail": "扫描页图不可用，该页内容未被覆盖"
            }],
            "sourceCoverageNotes": ["DOCX 图表证据不完整"],
            "warnings": []
        })
    }

    /// 完整候选必须**逐字**保留富内容，而不是走 `RecognitionCandidateV1` 那份扁平投影。
    ///
    /// 这条测试钉死「完整候选真的完整」：投影类型没有 `passage`，且会把 instructions /
    /// stimulus / prompt 压成纯文本。若哪天有人把候选换回投影类型，下面那条对嵌套
    /// `children` 的断言会立刻变红。
    #[test]
    fn cloud_authoring_candidate_keeps_full_rich_authoring() {
        let authoring = golden_authoring_value();
        let candidate: CloudAuthoringCandidateV1 =
            round_trip(cloud_candidate_fixture(authoring.clone()));

        assert!(candidate.is_supported_schema_version());
        assert!(!candidate.has_unresolved_references());
        // 未覆盖区域与覆盖说明必须原样留存，不能被静默丢掉。
        assert_eq!(candidate.unresolved_regions.len(), 1);
        assert_eq!(candidate.unresolved_regions[0].page_index, 3);
        assert_eq!(candidate.source_coverage_notes.len(), 1);
        assert_eq!(candidate.id_map.get("cloud-q14"), Some(&"q14".to_string()));

        let serialized = serde_json::to_value(&candidate.authoring).expect("重新序列化内嵌稿件");
        // 整份逐字相等：任何字段被丢掉或压平都会在这里露出来。
        assert_eq!(serialized, authoring, "完整候选不得改动内嵌稿件");
        // 再具体点出「嵌套富内容还在」，防止「空壳也算相等」的假通过。
        assert_eq!(
            serialized
                .pointer("/taskGroups/0/responseGroups/0/prompt/0/children/0/text")
                .and_then(Value::as_str),
            Some("Which TWO factors influenced early organisational design?"),
            "嵌套的子节点与文本必须逐字保留，不能被压平成纯文本"
        );
        // 选项库同样属于富内容，不能在候选里消失。
        assert!(
            serialized
                .pointer("/taskGroups/0/optionBank/options")
                .and_then(Value::as_array)
                .map(|options| !options.is_empty())
                .unwrap_or(false),
            "选项库必须保留"
        );
    }

    /// 映射不唯一的引用必须**结构化暴露**，不能静默丢弃、也不能任取第一个。
    #[test]
    fn cloud_authoring_candidate_surfaces_unresolved_references() {
        let mut fixture = cloud_candidate_fixture(golden_authoring_value());
        fixture["unresolvedReferences"] = json!(["cloud-q27"]);
        let candidate: CloudAuthoringCandidateV1 = round_trip(fixture);
        assert!(candidate.has_unresolved_references());
        assert_eq!(
            candidate.unresolved_references,
            vec!["cloud-q27".to_string()]
        );
    }

    #[test]
    fn cloud_repair_tool_call_only_accepts_declared_tools() {
        let call: CloudRepairToolCallV1 = serde_json::from_value(json!({
            "callId": "call-1",
            "tool": "apply_edits",
            "arguments": {"baseVersion": 7, "commands": []}
        }))
        .expect("工具调用必须可解析");
        assert!(call.is_known_tool());
        assert_eq!(call.arguments["baseVersion"], json!(7));

        // 未声明工具不在这里判死，而是要能被解析出来交给分发器给出**具体**错误，
        // 否则模型只会收到一个没有信息量的解析失败。
        let unknown: CloudRepairToolCallV1 = serde_json::from_value(json!({
            "callId": "call-2",
            "tool": "run_shell"
        }))
        .expect("未知工具也要能解析，交给分发器拒绝");
        assert!(!unknown.is_known_tool());
        assert_eq!(
            unknown.arguments,
            Value::Null,
            "缺省 arguments 必须是 Null，而不是空对象"
        );
    }

    #[test]
    fn cloud_repair_tool_result_carries_call_id_and_status() {
        let ok = CloudRepairToolResultV1::ok("call-1", json!({"editVersion": 8}));
        assert_eq!(ok.status, CloudRepairToolStatusV1::Ok);
        assert_eq!(ok.call_id, "call-1");
        assert!(ok.errors.is_empty());
        assert_eq!(ok.schema_version, "CloudRepairToolResultV1");
        assert_eq!(ok.result["editVersion"], json!(8));

        let rejected = CloudRepairToolResultV1::rejected(
            "call-1",
            vec!["EDIT_PROTECTED_TARGET:q14".to_string()],
        );
        assert_eq!(rejected.status, CloudRepairToolStatusV1::Rejected);
        assert_eq!(
            rejected.errors,
            vec!["EDIT_PROTECTED_TARGET:q14".to_string()]
        );
        assert_eq!(rejected.result, Value::Null, "拒绝时不得带 result 冒充成功");
    }
}
