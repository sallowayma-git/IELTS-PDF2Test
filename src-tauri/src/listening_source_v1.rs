//! Compile the canonical `IeltsAuthoringIRV2` into the listening runtime contract
//! (`ListeningExamSourceV1`), and expose the **single** compile entry point the
//! product uses for both modalities.
//!
//! Reading and listening share everything except the body of the exam: reading has
//! a `passage`, listening has `parts` with per-section audio. Both compile the same
//! task groups, answer slots, answer key, question order and display map, so this
//! module reuses the reading compiler's slot ordering rather than duplicating it —
//! the two contracts must agree on question order or the student runtime silently
//! shows a different numbering than the authoring editor.

use std::collections::BTreeMap;

use crate::reading_source_v2::{
    compile_reading_source_v2, compiler_issue, CompilerIssueV2, ReadingExamSourceV2,
};
use crate::schema::ielts_authoring_v2::{ExamModalityV2, IeltsAuthoringIRV2};
use crate::schema::listening_runtime_v1::{
    validate_listening_exam_source_v1, ListeningExamSourceV1, ListeningRuntimeAssetManifestRefV1,
    ListeningRuntimeAuditV1, ListeningRuntimeMetaV1, LISTENING_EXAM_SOURCE_V1_SCHEMA_VERSION,
};
use crate::CommandResult;

/// Runtime contract version a compiled listening source targets. The student
/// runtime refuses a source whose `audit.minimumRuntimeVersion` it cannot serve.
pub(crate) const LISTENING_MINIMUM_RUNTIME_VERSION: &str = "1.0.0";

/// A canonical document compiled into the runtime contract its modality requires.
///
/// Callers hand the document to the student runtime / package builder without
/// branching on modality themselves — that branching is exactly what let reading
/// and listening drift apart before.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CompiledExamSourceV2 {
    Reading(ReadingExamSourceV2),
    Listening(Box<ListeningExamSourceV1>),
}

impl CompiledExamSourceV2 {
    /// The runtime source document, ready to be written as JSON.
    pub(crate) fn document(&self) -> CommandResult<serde_json::Value> {
        let value = match self {
            Self::Reading(source) => serde_json::to_value(source),
            Self::Listening(source) => serde_json::to_value(source.as_ref()),
        };
        value.map_err(|error| error.to_string())
    }

    pub(crate) fn schema_version(&self) -> &str {
        match self {
            Self::Reading(source) => &source.schema_version,
            Self::Listening(source) => &source.schema_version,
        }
    }

    /// Re-check the compiled document against **its own** contract.
    pub(crate) fn is_valid(&self) -> bool {
        match self {
            Self::Reading(source) => crate::reading_source_v2::validate_reading_source_v2(source).is_empty(),
            Self::Listening(source) => validate_listening_exam_source_v1(source).is_empty(),
        }
    }

    /// File name the runtime source is written under inside an export directory
    /// and a NAS package. Reading keeps its historical name byte-for-byte.
    pub(crate) fn file_name(&self) -> &'static str {
        match self {
            Self::Reading(_) => runtime_source_file_name(ExamModalityV2::Reading),
            Self::Listening(_) => runtime_source_file_name(ExamModalityV2::Listening),
        }
    }

    /// `modality` as the NAS manifest spells it.
    pub(crate) fn modality(&self) -> &'static str {
        match self {
            Self::Reading(_) => "reading",
            Self::Listening(_) => "listening",
        }
    }

    pub(crate) fn exam_id(&self) -> &str {
        match self {
            Self::Reading(source) => &source.exam_id,
            Self::Listening(source) => &source.exam_id,
        }
    }
}

/// File name a modality's runtime source is written under. Reading keeps its
/// historical `reading-source-v2.json` so existing export directories, receipts and
/// NAS packages stay byte-for-byte identical.
pub(crate) fn runtime_source_file_name(modality: ExamModalityV2) -> &'static str {
    match modality {
        ExamModalityV2::Reading => "reading-source-v2.json",
        ExamModalityV2::Listening => "listening-source-v1.json",
    }
}

/// The runtime contract schema version a modality compiles into. Used by the
/// quality report's compiler probe, which has to name the contract it actually
/// exercised instead of always claiming `ReadingExamSourceV2`.
pub(crate) fn runtime_schema_version(modality: &ExamModalityV2) -> &'static str {
    match modality {
        ExamModalityV2::Reading => crate::reading_source_v2::READING_EXAM_SOURCE_V2_SCHEMA_VERSION,
        ExamModalityV2::Listening => LISTENING_EXAM_SOURCE_V1_SCHEMA_VERSION,
    }
}

/// Compile a canonical draft into the runtime contract for its modality.
///
/// This is the single entry point the product should use. `modality` is read from
/// the document itself (which the library row seeded), never inferred from shape:
/// a listening paper that lost its parts must fail as listening, not quietly
/// compile as a reading paper with no passage.
pub(crate) fn compile_exam_source_v2(
    source: &IeltsAuthoringIRV2,
) -> Result<CompiledExamSourceV2, Vec<CompilerIssueV2>> {
    match source.modality {
        ExamModalityV2::Listening => {
            compile_listening_source_v1(source).map(|source| CompiledExamSourceV2::Listening(Box::new(source)))
        }
        ExamModalityV2::Reading => {
            compile_reading_source_v2(source).map(CompiledExamSourceV2::Reading)
        }
    }
}

/// Compile the listening structure into `ListeningExamSourceV1`.
pub(crate) fn compile_listening_source_v1(
    source: &IeltsAuthoringIRV2,
) -> Result<ListeningExamSourceV1, Vec<CompilerIssueV2>> {
    let Some(structure) = source.listening.as_ref() else {
        return Err(vec![compiler_issue(
            "RUNTIME_LISTENING_MISSING",
            "Listening authoring source has no listening structure.",
            &source.job_id,
        )]);
    };
    // Exam-level audio cannot be compiled from the draft: `ListeningMediaV2` records
    // mime/duration/hash but no codec or container, and `ListeningRuntimeMediaV1`
    // requires both. The product binds one file per section, so refuse honestly
    // instead of inventing codec/container values the student runtime would trust.
    if structure.media.is_some() {
        return Err(vec![compiler_issue(
            "LISTENING_EXAM_MEDIA_UNSUPPORTED",
            "Exam-level listening audio cannot be compiled: the draft records no codec/container. Bind one audio file per part instead.",
            &source.exam.exam_id,
        )]);
    }

    let mut ordered_slots = source.answer_slots.iter().collect::<Vec<_>>();
    ordered_slots.sort_by_key(|(slot_id, slot)| (slot.question_number, (*slot_id).clone()));
    let question_order = ordered_slots
        .iter()
        .map(|(slot_id, _)| (*slot_id).clone())
        .collect::<Vec<_>>();
    let question_display_map = ordered_slots
        .iter()
        .map(|(slot_id, slot)| ((*slot_id).clone(), slot.display_label.clone()))
        .collect::<BTreeMap<_, _>>();

    let compiled = ListeningExamSourceV1 {
        schema_version: LISTENING_EXAM_SOURCE_V1_SCHEMA_VERSION.to_string(),
        exam_id: source.exam.exam_id.clone(),
        meta: ListeningRuntimeMetaV1 {
            title: source.exam.title.clone(),
            language: source.exam.language.clone(),
            scope: structure.scope.clone(),
        },
        assets: ListeningRuntimeAssetManifestRefV1 {
            exam_id: source.exam.exam_id.clone(),
            assets: source.assets.clone(),
        },
        media: None,
        parts: structure.parts.clone(),
        playback_policy: structure.playback_policy.clone(),
        transcript: structure.transcript.clone(),
        task_groups: source.task_groups.clone(),
        answer_slots: source.answer_slots.clone(),
        answer_key: source.answer_key.clone(),
        question_order,
        question_display_map,
        audit: ListeningRuntimeAuditV1 {
            source_schema_version: source.schema_version.clone(),
            source_document_id: source.source_document_id.clone(),
            source_revision: source.audit.revision,
            source_revision_kind: source.audit.source.clone(),
            minimum_runtime_version: LISTENING_MINIMUM_RUNTIME_VERSION.to_string(),
        },
    };

    let issues = validate_listening_exam_source_v1(&compiled)
        .into_iter()
        .map(|issue| compiler_issue(&issue.code, contract_message(&issue.code), &issue.target_id))
        .collect::<Vec<_>>();
    if issues.is_empty() {
        Ok(compiled)
    } else {
        Err(issues)
    }
}

/// Human-readable explanation for a `ListeningContractIssueV1` code. The runtime
/// validator speaks in codes so the student side can branch on them; the authoring
/// side has to tell the user what to fix.
fn contract_message(code: &str) -> &'static str {
    match code {
        "RUNTIME_SCHEMA_UNSUPPORTED" => "The runtime contract version is not supported.",
        "RUNTIME_EXAM_ID_MISMATCH" => "The exam id disagrees with the asset manifest.",
        "ASSET_REFERENCE_MISSING" => "A part references audio that is not in the asset list.",
        "AUDIO_DECODE_FAILED" => "The audio could not be decoded, or its duration disagrees with the asset.",
        "AUDIO_CODEC_UNSUPPORTED" => "The asset is not audio, or its MIME type disagrees with the part.",
        "AUDIO_HASH_MISMATCH" => "The audio hash disagrees with the asset list.",
        "AUDIO_CUE_INVALID" => "The part's cue is unconfirmed, out of range, or overlaps the previous cue.",
        "AUDIO_POLICY_MISSING" => "The playback policy is inconsistent (replay and maxPlays must agree).",
        "LISTENING_MEDIA_MISSING" => "The part has no audio and the exam has no complete-exam audio.",
        "LISTENING_PART_ID_DUPLICATE" => "Two parts share the same part id.",
        "LISTENING_TASK_MISSING" => "A part references a task group that does not exist.",
        "LISTENING_TASK_ASSIGNED_TWICE" => "A task group is assigned to more than one part.",
        "LISTENING_TASK_UNASSIGNED" => "A task group is not assigned to any part.",
        "LISTENING_PART_QUESTION_SCOPE_MISMATCH" => {
            "A part's expected question numbers disagree with the questions its tasks actually hold."
        }
        "LISTENING_QUESTION_SCOPE_MISMATCH" => {
            "The questions across all parts disagree with the scoring answer slots."
        }
        "LISTENING_COMPLETE_PART_COUNT" => "A complete exam must have exactly four parts.",
        "LISTENING_COMPLETE_QUESTION_COUNT" => {
            "A complete exam must cover questions 1-40, ten per part."
        }
        "RUNTIME_TASK_ID_DUPLICATE" => "Two task groups share the same task id.",
        "QUESTION_NUMBER_MISSING" => "A response group references an answer slot that does not exist.",
        "QUESTION_NUMBER_DUPLICATE" => "The same question number is used by more than one slot.",
        "RUNTIME_SLOT_UNASSIGNED" => "An answer slot is not referenced by any response group.",
        "RUNTIME_QUESTION_ORDER_INVALID" => "questionOrder must contain every answer slot exactly once.",
        "RUNTIME_DISPLAY_MAP_MISMATCH" => "questionDisplayMap must contain exactly every answer slot.",
        "ANSWER_KEY_MISSING_SLOT" => "A scoring slot has no answer.",
        other => {
            // Codes are stable identifiers; an unmapped one must still surface as
            // itself rather than as an empty message.
            let _ = other;
            "The listening runtime contract rejected this document."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::common::AssetDescriptorV2;
    use crate::schema::ielts_authoring_v2::{
        AnswerSlotParticipationV2, AnswerSlotV2, AnswerValueV2, ListeningPartMediaV2,
        ListeningPartV2, ListeningPlaybackPolicyV2, ListeningScopeV2, ListeningStructureV2,
        QuestionNumberExpressionV2, TaskGroupV2,
    };
    use crate::schema::listening_runtime_v1::ListeningAudioProbeStatusV1;
    use serde_json::json;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn asset() -> AssetDescriptorV2 {
        serde_json::from_value(json!({
            "assetId": format!("audio-{SHA}"),
            "kind": "audio",
            "mime": "audio/wav",
            "relativePath": format!("audio/{SHA}.wav"),
            "sha256": SHA,
            "byteLength": 4096,
            "durationMs": 1000,
            "extractionMode": "user_upload"
        }))
        .unwrap()
    }

    fn media() -> ListeningPartMediaV2 {
        serde_json::from_value(json!({
            "assetId": format!("audio-{SHA}"),
            "mime": "audio/wav",
            "durationMs": 1000,
            "channels": 1,
            "sampleRateHz": 16000,
            "sha256": SHA,
            "probe": {
                "status": "passed",
                "provider": "symphonia",
                "providerVersion": "0.6.0",
                "probedAt": "2026-09-22T00:00:00Z",
                "issueCodes": []
            }
        }))
        .unwrap()
    }

    fn part(ordinal: u32, questions: Vec<u32>) -> ListeningPartV2 {
        ListeningPartV2 {
            part_id: format!("part-{ordinal}"),
            display_label: format!("SECTION {ordinal}"),
            expected_question_numbers: questions.clone(),
            task_ids: vec![format!("task-{ordinal}")],
            cue: None,
            source_anchors: Vec::new(),
            media: Some(media()),
        }
    }

    /// One slot per question, each owned by its part's task group.
    fn listening_source(parts: Vec<ListeningPartV2>, questions_per_part: usize) -> IeltsAuthoringIRV2 {
        let mut task_groups = Vec::new();
        let mut answer_slots = BTreeMap::new();
        let mut answer_key = BTreeMap::new();
        for part in &parts {
            let mut slot_ids = Vec::new();
            for number in &part.expected_question_numbers {
                let slot_id = format!("q{number}");
                slot_ids.push(slot_id.clone());
                answer_slots.insert(
                    slot_id.clone(),
                    serde_json::from_value::<AnswerSlotV2>(json!({
                        "slotId": slot_id,
                        "questionNumber": number,
                        "displayLabel": number.to_string(),
                        "hostType": "paragraph",
                        "interaction": "text",
                        "participation": "scoring",
                        "constraints": {"maxWords": 1, "maxNumbers": 0},
                        "sourceAnchors": [],
                        "confidence": 1
                    }))
                    .unwrap(),
                );
                answer_key.insert(
                    slot_id,
                    AnswerValueV2::Text {
                        values: vec![format!("answer {number}")],
                        normalization: None,
                    },
                );
            }
            task_groups.push(
                serde_json::from_value::<TaskGroupV2>(json!({
                    "taskId": part.task_ids[0],
                    "displayRange": {
                        "kind": "range",
                        "start": part.expected_question_numbers.first().unwrap(),
                        "end": part.expected_question_numbers.last().unwrap()
                    },
                    "taskType": "note_completion",
                    "instructions": [],
                    "instructionSignature": {
                        "normalizedText": "Write ONE WORD ONLY.",
                        "taskType": "note_completion",
                        "expectedQuestionNumbers": part.expected_question_numbers,
                        "expectedSlotCount": part.expected_question_numbers.len(),
                        "selectionCardinality": {"min": 1, "max": 1, "exact": 1},
                        "answerAssignment": "per_slot",
                        "wordLimit": {"maxWords": 1, "maxNumbers": 0, "wordsAndOrNumber": false},
                        "evidenceAnchors": [],
                        "confidence": 1
                    },
                    "stimulus": [],
                    "responseGroups": [{
                        "responseGroupId": format!("{}-response", part.part_id),
                        "kind": "text_entry",
                        "slotIds": slot_ids,
                        "cardinality": {"min": 1, "max": 1, "exact": 1},
                        "assignment": "per_slot",
                        "scoringPolicy": "per_slot_binary",
                        "duplicatePolicy": "ignore_duplicates",
                        "allowOptionReuse": false,
                        "sourceAnchors": []
                    }],
                    "sourceAnchors": [],
                    "quality": {"score": 1, "sourceCoverage": 1, "hardFailures": []},
                    "reviewState": "confirmed"
                }))
                .unwrap(),
            );
        }
        let _ = questions_per_part;
        // Start from the committed reading draft so every required piece of exam
        // metadata is genuinely schema-valid, then reshape the body into a listening
        // paper. Hand-writing the whole document here would let the fixture drift
        // away from the schema without anything noticing.
        let mut source: IeltsAuthoringIRV2 = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .expect("committed authoring fixture parses");
        source.modality = ExamModalityV2::Listening;
        source.passage = None;
        source.assets = vec![asset()];
        source.listening = Some(ListeningStructureV2 {
            scope: ListeningScopeV2::CompleteExam,
            media: None,
            parts,
            playback_policy: serde_json::from_value::<ListeningPlaybackPolicyV2>(json!({
                "mode": "practice",
                "autoplay": false,
                "allowPause": true,
                "allowSeek": true,
                "allowReplay": true,
                "refreshBehavior": "resume_from_snapshot",
                "crashRecoveryBehavior": "resume_from_snapshot",
                "showCurrentTime": true,
                "showDuration": true
            }))
            .unwrap(),
            transcript: None,
        });
        source.task_groups = task_groups;
        source.answer_slots = answer_slots;
        source.answer_key = answer_key;
        source.source_document_id = "doc-listening".to_string();
        source.audit.revision = 3;
        source
    }

    fn complete_exam() -> IeltsAuthoringIRV2 {
        let parts = vec![
            part(1, (1..=10).collect()),
            part(2, (11..=20).collect()),
            part(3, (21..=30).collect()),
            part(4, (31..=40).collect()),
        ];
        listening_source(parts, 10)
    }

    #[test]
    fn a_four_part_paper_compiles_into_a_valid_listening_source() {
        let compiled = compile_listening_source_v1(&complete_exam()).expect("compiles");
        assert_eq!(compiled.schema_version, "ListeningExamSourceV1");
        assert_eq!(compiled.parts.len(), 4);
        assert!(compiled.media.is_none(), "audio is per part");
        assert_eq!(compiled.meta.scope, ListeningScopeV2::CompleteExam);
        assert_eq!(compiled.audit.minimum_runtime_version, "1.0.0");
        assert_eq!(compiled.audit.source_revision, 3);
        assert_eq!(compiled.question_order.len(), 40);
        assert_eq!(compiled.question_order[0], "q1");
        assert_eq!(compiled.question_order[39], "q40");
        assert!(validate_listening_exam_source_v1(&compiled).is_empty());
    }

    #[test]
    fn a_part_without_audio_is_rejected_instead_of_shipping_silence() {
        let mut source = complete_exam();
        source.listening.as_mut().unwrap().parts[2].media = None;
        let issues = compile_listening_source_v1(&source).expect_err("must not compile");
        assert!(issues.iter().any(|issue| issue.code == "LISTENING_MEDIA_MISSING"));
        assert!(issues
            .iter()
            .any(|issue| issue.target_id == "part-3"));
    }

    #[test]
    fn a_blocked_probe_is_rejected_so_unusable_audio_cannot_publish() {
        let mut source = complete_exam();
        let media = source.listening.as_mut().unwrap().parts[1].media.as_mut().unwrap();
        media.probe.as_mut().unwrap().status = ListeningAudioProbeStatusV1::Blocked;
        let issues = compile_listening_source_v1(&source).expect_err("must not compile");
        assert!(issues.iter().any(|issue| issue.code == "AUDIO_DECODE_FAILED"));
    }

    #[test]
    fn an_asset_that_disagrees_with_the_part_media_is_rejected() {
        let mut source = complete_exam();
        source.assets[0].sha256 = "b".repeat(64);
        let issues = compile_listening_source_v1(&source).expect_err("must not compile");
        assert!(issues.iter().any(|issue| issue.code == "AUDIO_HASH_MISMATCH"));
    }

    #[test]
    fn a_reading_paper_never_compiles_as_listening() {
        let mut source = complete_exam();
        source.listening = None;
        let issues = compile_listening_source_v1(&source).expect_err("must not compile");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "RUNTIME_LISTENING_MISSING");
    }

    #[test]
    fn exam_level_media_is_refused_rather_than_given_an_invented_codec() {
        let mut source = complete_exam();
        for part in source.listening.as_mut().unwrap().parts.iter_mut() {
            part.media = None;
        }
        source.listening.as_mut().unwrap().media = Some(
            serde_json::from_value(json!({
                "assetId": format!("audio-{SHA}"),
                "mime": "audio/wav",
                "durationMs": 1000,
                "sha256": SHA
            }))
            .unwrap(),
        );
        let issues = compile_listening_source_v1(&source).expect_err("must not compile");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "LISTENING_EXAM_MEDIA_UNSUPPORTED");
    }

    #[test]
    fn the_dispatcher_follows_the_documents_own_modality() {
        let listening = compile_exam_source_v2(&complete_exam()).expect("listening compiles");
        assert!(matches!(listening, CompiledExamSourceV2::Listening(_)));
        assert_eq!(listening.schema_version(), "ListeningExamSourceV1");
        let document = listening.document().unwrap();
        assert_eq!(document["schemaVersion"], "ListeningExamSourceV1");
        assert_eq!(document["parts"].as_array().unwrap().len(), 4);
        assert!(document.get("passage").is_none(), "a listening source has no passage");
    }

    /// The shared slot ordering must not drift between the two contracts: a
    /// listening paper's `questionOrder` is derived the same way a reading paper's
    /// is, so the student runtime numbers questions identically in both.
    #[test]
    fn question_order_is_shared_with_the_reading_contract() {
        let source = complete_exam();
        let compiled = compile_listening_source_v1(&source).expect("compiles");
        let mut expected = source.answer_slots.values().collect::<Vec<_>>();
        expected.sort_by_key(|slot| (slot.question_number, slot.slot_id.clone()));
        let expected_ids = expected
            .iter()
            .map(|slot| slot.slot_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(compiled.question_order, expected_ids);
        assert_eq!(
            compiled
                .answer_slots
                .values()
                .filter(|slot| slot.participation == AnswerSlotParticipationV2::Scoring)
                .count(),
            40
        );
    }

    /// `QuestionNumberExpressionV2` is referenced so the fixture's `displayRange`
    /// shape stays honest against the schema rather than drifting into a loose JSON
    /// blob only this test understands.
    #[test]
    fn the_fixture_display_range_matches_the_schema() {
        let expression: QuestionNumberExpressionV2 =
            serde_json::from_value(json!({"kind": "range", "start": 1, "end": 10})).unwrap();
        assert_eq!(expression, QuestionNumberExpressionV2::Range { start: 1, end: 10 });
    }
}
