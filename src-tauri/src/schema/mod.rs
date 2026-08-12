pub mod common;
pub mod content_doc_v2;
pub mod document_ir_v2;
pub mod ielts_authoring_v2;
pub mod migration_v1;
pub mod quality_report_v2;

pub use content_doc_v2::{ContentDocV2, ContentNodeV2};
pub use document_ir_v2::DocumentIRV2;
pub use ielts_authoring_v2::IeltsAuthoringIRV2;
pub use quality_report_v2::QualityReportV2;

#[cfg(test)]
mod tests {
    use super::common::canonical_json_bytes;
    use super::{ContentDocV2, DocumentIRV2, IeltsAuthoringIRV2, QualityReportV2};
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
}
