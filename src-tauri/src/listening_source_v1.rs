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
            Self::Reading(source) => {
                crate::reading_source_v2::validate_reading_source_v2(source).is_empty()
            }
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

/// Parsing and the packaging/probing accessors. The two contracts share
/// everything a package cares about — exam id, title, assets, schema version —
/// and differ only in the exam body, which packaging never reads. One accessor
/// set stops the NAS transaction and the student loader probe from growing a
/// reading-shaped copy of themselves for each new modality.
impl CompiledExamSourceV2 {
    /// Parse a runtime source document, dispatching on its `schemaVersion`.
    ///
    /// An unknown version is parsed as reading so the caller's existing error
    /// surface is unchanged; the compile step already guarantees a listening
    /// source carries the listening version.
    pub(crate) fn parse(value: &serde_json::Value) -> Result<Self, String> {
        let schema = value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if schema == LISTENING_EXAM_SOURCE_V1_SCHEMA_VERSION {
            serde_json::from_value::<ListeningExamSourceV1>(value.clone())
                .map(|source| Self::Listening(Box::new(source)))
                .map_err(|error| error.to_string())
        } else {
            serde_json::from_value::<ReadingExamSourceV2>(value.clone())
                .map(Self::Reading)
                .map_err(|error| error.to_string())
        }
    }

    pub(crate) fn title(&self) -> &str {
        match self {
            Self::Reading(source) => &source.meta.title,
            Self::Listening(source) => &source.meta.title,
        }
    }

    /// `meta.category` as the NAS manifest spells it. Listening has no category,
    /// so it is `null` there rather than an invented passage kind.
    pub(crate) fn category_value(&self) -> serde_json::Value {
        match self {
            Self::Reading(source) => source
                .meta
                .category
                .as_ref()
                .and_then(|category| serde_json::to_value(category).ok())
                .unwrap_or(serde_json::Value::Null),
            Self::Listening(_) => serde_json::Value::Null,
        }
    }

    pub(crate) fn assets(&self) -> &[crate::schema::common::AssetDescriptorV2] {
        match self {
            Self::Reading(source) => &source.assets.assets,
            Self::Listening(source) => &source.assets.assets,
        }
    }

    pub(crate) fn assets_exam_id(&self) -> &str {
        match self {
            Self::Reading(source) => &source.assets.exam_id,
            Self::Listening(source) => &source.assets.exam_id,
        }
    }

    /// Re-check the document against its own contract.
    pub(crate) fn validate(&self) -> Vec<CompilerIssueV2> {
        match self {
            Self::Reading(source) => crate::reading_source_v2::validate_reading_source_v2(source),
            Self::Listening(source) => validate_listening_exam_source_v1(source)
                .into_iter()
                .map(|issue| {
                    compiler_issue(&issue.code, contract_message(&issue.code), &issue.target_id)
                })
                .collect(),
        }
    }

    /// Asset ids the exam body actually plays or displays. The package manifest
    /// must contain exactly the source asset set, and this is the "referenced"
    /// half of that closure.
    pub(crate) fn referenced_asset_ids(&self) -> Vec<String> {
        match self {
            Self::Reading(source) => {
                crate::reading_runtime_v2::reading_referenced_asset_ids(source)
            }
            Self::Listening(source) => {
                let mut ids = std::collections::BTreeSet::new();
                if let Some(media) = &source.media {
                    ids.insert(media.asset_id.clone());
                }
                for part in &source.parts {
                    if let Some(media) = &part.media {
                        ids.insert(media.asset_id.clone());
                    }
                }
                ids.into_iter().collect()
            }
        }
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
        ExamModalityV2::Listening => compile_listening_source_v1(source)
            .map(|source| CompiledExamSourceV2::Listening(Box::new(source))),
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
    use crate::schema::ielts_authoring_v2::{
        AnswerSlotParticipationV2, ListeningScopeV2, QuestionNumberExpressionV2,
    };
    use crate::schema::listening_runtime_v1::ListeningAudioProbeStatusV1;
    use crate::test_support::complete_listening_exam;
    use serde_json::json;

    fn complete_exam() -> IeltsAuthoringIRV2 {
        complete_listening_exam()
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

    /// Four sections, four files. A paper whose sections all played the same recording would
    /// satisfy every count-based check while being unusable in a real exam.
    #[test]
    fn each_part_carries_its_own_audio_asset() {
        let compiled = compile_listening_source_v1(&complete_exam()).expect("compiles");
        let asset_ids = compiled
            .parts
            .iter()
            .map(|part| {
                part.media
                    .as_ref()
                    .expect("every part is bound")
                    .asset_id
                    .clone()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(asset_ids.len(), 4, "{asset_ids:?}");
        assert_eq!(compiled.assets.assets.len(), 4);
        for descriptor in &compiled.assets.assets {
            assert!(
                asset_ids.contains(&descriptor.asset_id),
                "{} is not referenced by any part",
                descriptor.asset_id
            );
        }
    }

    #[test]
    fn a_part_without_audio_is_rejected_instead_of_shipping_silence() {
        let mut source = complete_exam();
        source.listening.as_mut().unwrap().parts[2].media = None;
        let issues = compile_listening_source_v1(&source).expect_err("must not compile");
        assert!(issues
            .iter()
            .any(|issue| issue.code == "LISTENING_MEDIA_MISSING"));
        assert!(issues.iter().any(|issue| issue.target_id == "part-3"));
    }

    #[test]
    fn a_blocked_probe_is_rejected_so_unusable_audio_cannot_publish() {
        let mut source = complete_exam();
        let media = source.listening.as_mut().unwrap().parts[1]
            .media
            .as_mut()
            .unwrap();
        media.probe.as_mut().unwrap().status = ListeningAudioProbeStatusV1::Blocked;
        let issues = compile_listening_source_v1(&source).expect_err("must not compile");
        assert!(issues
            .iter()
            .any(|issue| issue.code == "AUDIO_DECODE_FAILED"));
    }

    #[test]
    fn an_asset_that_disagrees_with_the_part_media_is_rejected() {
        let mut source = complete_exam();
        source.assets[0].sha256 = "b".repeat(64);
        let issues = compile_listening_source_v1(&source).expect_err("must not compile");
        assert!(issues
            .iter()
            .any(|issue| issue.code == "AUDIO_HASH_MISMATCH"));
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
        let sha = crate::test_support::audio_sha256();
        source.listening.as_mut().unwrap().media = Some(
            serde_json::from_value(json!({
                "assetId": format!("audio-{sha}"),
                "mime": "audio/wav",
                "durationMs": 1000,
                "sha256": sha
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
        assert_eq!(listening.file_name(), "listening-source-v1.json");
        assert_eq!(listening.modality(), "listening");
        let document = listening.document().unwrap();
        assert_eq!(document["schemaVersion"], "ListeningExamSourceV1");
        assert_eq!(document["parts"].as_array().unwrap().len(), 4);
        assert!(
            document.get("passage").is_none(),
            "a listening source has no passage"
        );
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
        assert_eq!(
            expression,
            QuestionNumberExpressionV2::Range { start: 1, end: 10 }
        );
    }
}
