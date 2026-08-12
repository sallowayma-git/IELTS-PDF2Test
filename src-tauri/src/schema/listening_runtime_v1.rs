use super::common::{AssetDescriptorV2, AssetKindV2};
use super::ielts_authoring_v2::{
    AnswerSlotParticipationV2, AnswerSlotV2, AnswerValueV2, ListeningPartV2,
    ListeningPlaybackModeV2, ListeningPlaybackPolicyV2, ListeningScopeV2,
    ListeningRecoveryBehaviorV2, ListeningTranscriptV2, RevisionSourceV2, TaskGroupV2,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const LISTENING_EXAM_SOURCE_V1_SCHEMA_VERSION: &str = "ListeningExamSourceV1";
pub const LISTENING_ATTEMPT_V1_SCHEMA_VERSION: &str = "ListeningAttemptV1";
const MIN_CONFIRMED_CUE_CONFIDENCE: f64 = 0.9;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ListeningAudioProbeStatusV1 {
    Passed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ListeningAudioIssueCodeV1 {
    AudioDecodeFailed,
    AudioCodecUnsupported,
    AudioHashMismatch,
    AudioSevereClipping,
    AudioNearSilent,
    AudioCueInvalid,
    AudioPolicyMissing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningAudioProbeV1 {
    pub status: ListeningAudioProbeStatusV1,
    pub provider: String,
    pub provider_version: String,
    pub probed_at: String,
    pub issue_codes: Vec<ListeningAudioIssueCodeV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningRuntimeMediaV1 {
    pub asset_id: String,
    pub mime: String,
    pub codec: String,
    pub container: String,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate_hz: Option<u32>,
    pub sha256: String,
    pub probe: ListeningAudioProbeV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningRuntimeMetaV1 {
    pub title: String,
    pub language: String,
    pub scope: ListeningScopeV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningRuntimeAssetManifestRefV1 {
    pub exam_id: String,
    pub assets: Vec<AssetDescriptorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningRuntimeAuditV1 {
    pub source_schema_version: String,
    pub source_document_id: String,
    pub source_revision: u64,
    pub source_revision_kind: RevisionSourceV2,
    pub minimum_runtime_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningExamSourceV1 {
    pub schema_version: String,
    pub exam_id: String,
    pub meta: ListeningRuntimeMetaV1,
    pub assets: ListeningRuntimeAssetManifestRefV1,
    pub media: ListeningRuntimeMediaV1,
    pub parts: Vec<ListeningPartV2>,
    pub playback_policy: ListeningPlaybackPolicyV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<ListeningTranscriptV2>,
    pub task_groups: Vec<TaskGroupV2>,
    pub answer_slots: BTreeMap<String, AnswerSlotV2>,
    pub answer_key: BTreeMap<String, AnswerValueV2>,
    pub question_order: Vec<String>,
    pub question_display_map: BTreeMap<String, String>,
    pub audit: ListeningRuntimeAuditV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ListeningAttemptStateV1 {
    NotStarted,
    InProgress,
    Submitted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ListeningPlaybackStatusV1 {
    Ready,
    Playing,
    Paused,
    Ended,
    RestartPending,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ListeningPlaybackFailureCodeV1 {
    AudioDecodeFailed,
    AudioCodecUnsupported,
    AudioHashMismatch,
    AudioRecoveryBlocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningPlaybackSnapshotV1 {
    pub media_asset_id: String,
    pub policy_mode: ListeningPlaybackModeV2,
    pub plays_started: u32,
    pub position_ms: u64,
    pub status: ListeningPlaybackStatusV1,
    pub last_transition_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<ListeningPlaybackFailureCodeV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningAttemptV1 {
    pub schema_version: String,
    pub exam_id: String,
    pub source_revision: u64,
    pub answers: BTreeMap<String, AnswerValueV2>,
    pub playback: ListeningPlaybackSnapshotV1,
    pub state: ListeningAttemptStateV1,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningContractIssueV1 {
    pub code: String,
    pub target_id: String,
}

fn push_issue(issues: &mut Vec<ListeningContractIssueV1>, code: &str, target_id: &str) {
    if !issues
        .iter()
        .any(|issue| issue.code == code && issue.target_id == target_id)
    {
        issues.push(ListeningContractIssueV1 {
            code: code.to_string(),
            target_id: target_id.to_string(),
        });
    }
}

pub fn validate_listening_exam_source_v1(
    source: &ListeningExamSourceV1,
) -> Vec<ListeningContractIssueV1> {
    let mut issues = Vec::new();
    if source.schema_version != LISTENING_EXAM_SOURCE_V1_SCHEMA_VERSION {
        push_issue(&mut issues, "RUNTIME_SCHEMA_UNSUPPORTED", "schemaVersion");
        return issues;
    }
    if source.exam_id.is_empty() || source.assets.exam_id != source.exam_id {
        push_issue(&mut issues, "RUNTIME_EXAM_ID_MISMATCH", &source.exam_id);
    }
    if source.audit.source_schema_version != "IeltsAuthoringIRV2" {
        push_issue(
            &mut issues,
            "RUNTIME_SCHEMA_UNSUPPORTED",
            "sourceSchemaVersion",
        );
    }

    match source.media.probe.status {
        ListeningAudioProbeStatusV1::Passed if source.media.probe.issue_codes.is_empty() => {}
        _ => push_issue(&mut issues, "AUDIO_DECODE_FAILED", &source.media.asset_id),
    }
    let media_asset = source
        .assets
        .assets
        .iter()
        .find(|asset| asset.asset_id == source.media.asset_id);
    match media_asset {
        None => push_issue(&mut issues, "ASSET_REFERENCE_MISSING", &source.media.asset_id),
        Some(asset) => {
            if asset.kind != AssetKindV2::Audio || asset.mime != source.media.mime {
                push_issue(&mut issues, "AUDIO_CODEC_UNSUPPORTED", &source.media.asset_id);
            }
            if !asset.sha256.eq_ignore_ascii_case(&source.media.sha256) {
                push_issue(&mut issues, "AUDIO_HASH_MISMATCH", &source.media.asset_id);
            }
            if asset.duration_ms.is_some_and(|duration| duration != source.media.duration_ms) {
                push_issue(&mut issues, "AUDIO_DECODE_FAILED", &source.media.asset_id);
            }
        }
    }

    if (!source.playback_policy.allow_replay && source.playback_policy.max_plays != Some(1))
        || (source.playback_policy.allow_replay && source.playback_policy.max_plays == Some(1))
    {
        push_issue(&mut issues, "AUDIO_POLICY_MISSING", &source.exam_id);
    }
    if matches!(source.playback_policy.mode, ListeningPlaybackModeV2::Mock)
        && (source.playback_policy.allow_pause
            || source.playback_policy.allow_seek
            || source.playback_policy.allow_replay
            || source.playback_policy.max_plays != Some(1)
            || source.playback_policy.refresh_behavior
                != ListeningRecoveryBehaviorV2::ResumeFromSnapshot
            || source.playback_policy.crash_recovery_behavior
                != ListeningRecoveryBehaviorV2::ResumeFromSnapshot)
    {
        push_issue(&mut issues, "AUDIO_POLICY_MISSING", &source.exam_id);
    }

    let mut task_ids = BTreeSet::new();
    let mut tasks_by_id = BTreeMap::new();
    for task in &source.task_groups {
        if !task_ids.insert(task.task_id.clone()) {
            push_issue(&mut issues, "RUNTIME_TASK_ID_DUPLICATE", &task.task_id);
        }
        tasks_by_id.insert(task.task_id.as_str(), task);
    }

    let mut response_slot_ids = BTreeSet::new();
    for task in &source.task_groups {
        for response in &task.response_groups {
            for slot_id in &response.slot_ids {
                if !source.answer_slots.contains_key(slot_id) {
                    push_issue(&mut issues, "QUESTION_NUMBER_MISSING", slot_id);
                }
                if !response_slot_ids.insert(slot_id.clone()) {
                    push_issue(&mut issues, "QUESTION_NUMBER_DUPLICATE", slot_id);
                }
            }
        }
    }
    for slot_id in source.answer_slots.keys() {
        if !response_slot_ids.contains(slot_id) {
            push_issue(&mut issues, "RUNTIME_SLOT_UNASSIGNED", slot_id);
        }
    }

    let all_slot_ids = source.answer_slots.keys().cloned().collect::<BTreeSet<_>>();
    let ordered_slot_ids = source.question_order.iter().cloned().collect::<BTreeSet<_>>();
    if all_slot_ids != ordered_slot_ids || ordered_slot_ids.len() != source.question_order.len() {
        push_issue(&mut issues, "RUNTIME_QUESTION_ORDER_INVALID", &source.exam_id);
    }
    let display_slot_ids = source
        .question_display_map
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if all_slot_ids != display_slot_ids {
        push_issue(&mut issues, "RUNTIME_DISPLAY_MAP_MISMATCH", &source.exam_id);
    }

    let scoring_slots = source
        .answer_slots
        .iter()
        .filter(|(_, slot)| slot.participation == AnswerSlotParticipationV2::Scoring)
        .collect::<Vec<_>>();
    let mut scoring_numbers = BTreeSet::new();
    for (slot_id, slot) in &scoring_slots {
        if !scoring_numbers.insert(slot.question_number) {
            push_issue(&mut issues, "QUESTION_NUMBER_DUPLICATE", slot_id);
        }
        if !source.answer_key.contains_key(*slot_id) {
            push_issue(&mut issues, "ANSWER_KEY_MISSING_SLOT", slot_id);
        }
    }

    let mut part_ids = BTreeSet::new();
    let mut assigned_task_ids = BTreeSet::new();
    let mut previous_cue_end = None;
    let mut expected_numbers_across_parts = BTreeSet::new();
    for part in &source.parts {
        if !part_ids.insert(part.part_id.clone()) {
            push_issue(&mut issues, "LISTENING_PART_ID_DUPLICATE", &part.part_id);
        }
        if let Some(cue) = &part.cue {
            let invalid = cue.start_ms >= cue.end_ms
                || cue.end_ms > source.media.duration_ms
                || previous_cue_end.is_some_and(|end| cue.start_ms < end)
                || !cue.confirmed
                || cue.confidence < MIN_CONFIRMED_CUE_CONFIDENCE;
            if invalid {
                push_issue(&mut issues, "AUDIO_CUE_INVALID", &part.part_id);
            }
            previous_cue_end = Some(cue.end_ms);
        }

        let mut actual_part_numbers = BTreeSet::new();
        for task_id in &part.task_ids {
            if !assigned_task_ids.insert(task_id.clone()) {
                push_issue(&mut issues, "LISTENING_TASK_ASSIGNED_TWICE", task_id);
            }
            let Some(task) = tasks_by_id.get(task_id.as_str()) else {
                push_issue(&mut issues, "LISTENING_TASK_MISSING", task_id);
                continue;
            };
            for response in &task.response_groups {
                for slot_id in &response.slot_ids {
                    if let Some(slot) = source.answer_slots.get(slot_id) {
                        if slot.participation == AnswerSlotParticipationV2::Scoring {
                            actual_part_numbers.insert(slot.question_number);
                        }
                    }
                }
            }
        }
        let expected_part_numbers = part
            .expected_question_numbers
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if expected_part_numbers.len() != part.expected_question_numbers.len()
            || expected_part_numbers != actual_part_numbers
        {
            push_issue(
                &mut issues,
                "LISTENING_PART_QUESTION_SCOPE_MISMATCH",
                &part.part_id,
            );
        }
        for number in expected_part_numbers {
            if !expected_numbers_across_parts.insert(number) {
                push_issue(
                    &mut issues,
                    "QUESTION_NUMBER_DUPLICATE",
                    &part.part_id,
                );
            }
        }
    }
    if assigned_task_ids != task_ids {
        push_issue(&mut issues, "LISTENING_TASK_UNASSIGNED", &source.exam_id);
    }
    if expected_numbers_across_parts != scoring_numbers {
        push_issue(
            &mut issues,
            "LISTENING_QUESTION_SCOPE_MISMATCH",
            &source.exam_id,
        );
    }

    if matches!(source.meta.scope, ListeningScopeV2::CompleteExam) {
        if source.parts.len() != 4 {
            push_issue(
                &mut issues,
                "LISTENING_COMPLETE_PART_COUNT",
                &source.exam_id,
            );
        }
        if source
            .parts
            .iter()
            .any(|part| part.expected_question_numbers.len() != 10)
            || scoring_numbers.len() != 40
            || scoring_numbers != (1..=40).collect::<BTreeSet<_>>()
        {
            push_issue(
                &mut issues,
                "LISTENING_COMPLETE_QUESTION_COUNT",
                &source.exam_id,
            );
        }
    }

    issues
}

pub fn validate_listening_attempt_v1(
    source: &ListeningExamSourceV1,
    attempt: &ListeningAttemptV1,
) -> Vec<ListeningContractIssueV1> {
    let mut issues = Vec::new();
    if attempt.schema_version != LISTENING_ATTEMPT_V1_SCHEMA_VERSION {
        push_issue(
            &mut issues,
            "RUNTIME_ATTEMPT_SCHEMA_UNSUPPORTED",
            &attempt.exam_id,
        );
    }
    if attempt.exam_id != source.exam_id {
        push_issue(
            &mut issues,
            "RUNTIME_ATTEMPT_EXAM_MISMATCH",
            &attempt.exam_id,
        );
    }
    if attempt.source_revision != source.audit.source_revision {
        push_issue(
            &mut issues,
            "RUNTIME_ATTEMPT_REVISION_MISMATCH",
            &attempt.exam_id,
        );
    }
    if attempt.playback.media_asset_id != source.media.asset_id {
        push_issue(
            &mut issues,
            "AUDIO_HASH_MISMATCH",
            &attempt.playback.media_asset_id,
        );
    }
    if attempt.playback.policy_mode != source.playback_policy.mode {
        push_issue(&mut issues, "AUDIO_POLICY_MISSING", &attempt.exam_id);
    }
    if attempt.playback.position_ms > source.media.duration_ms {
        push_issue(
            &mut issues,
            "AUDIO_CUE_INVALID",
            &attempt.playback.media_asset_id,
        );
    }
    if source
        .playback_policy
        .max_plays
        .is_some_and(|limit| attempt.playback.plays_started > limit)
    {
        push_issue(&mut issues, "AUDIO_POLICY_MISSING", &attempt.exam_id);
    }
    match attempt.playback.status {
        ListeningPlaybackStatusV1::Ready => {
            if attempt.playback.plays_started != 0 || attempt.playback.position_ms != 0 {
                push_issue(&mut issues, "AUDIO_POLICY_MISSING", &attempt.exam_id);
            }
        }
        ListeningPlaybackStatusV1::RestartPending => {
            if attempt.playback.plays_started == 0 || attempt.playback.position_ms != 0 {
                push_issue(&mut issues, "AUDIO_POLICY_MISSING", &attempt.exam_id);
            }
        }
        ListeningPlaybackStatusV1::Playing | ListeningPlaybackStatusV1::Paused => {
            if attempt.playback.plays_started == 0
                || attempt.playback.position_ms >= source.media.duration_ms
            {
                push_issue(&mut issues, "AUDIO_POLICY_MISSING", &attempt.exam_id);
            }
        }
        ListeningPlaybackStatusV1::Ended => {
            if attempt.playback.plays_started == 0
                || attempt.playback.position_ms != source.media.duration_ms
            {
                push_issue(&mut issues, "AUDIO_POLICY_MISSING", &attempt.exam_id);
            }
        }
        ListeningPlaybackStatusV1::Failed => {}
    }
    if matches!(attempt.playback.status, ListeningPlaybackStatusV1::Paused)
        && !source.playback_policy.allow_pause
    {
        push_issue(&mut issues, "AUDIO_POLICY_MISSING", &attempt.exam_id);
    }
    if matches!(attempt.playback.status, ListeningPlaybackStatusV1::Failed)
        != attempt.playback.failure_code.is_some()
    {
        push_issue(&mut issues, "AUDIO_POLICY_MISSING", &attempt.exam_id);
    }
    for slot_id in attempt.answers.keys() {
        if !source.answer_slots.contains_key(slot_id) {
            push_issue(&mut issues, "RUNTIME_UNKNOWN_SLOT", slot_id);
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ListeningExamSourceV1 {
        serde_json::from_str(include_str!(
            "../../../fixtures/golden/synthetic/ielts/phase7-listening-part1-source-v1.json"
        ))
        .expect("Phase 7 fixture must deserialize")
    }

    #[test]
    fn partial_practice_contract_does_not_require_forty_questions() {
        let source = fixture();
        assert!(validate_listening_exam_source_v1(&source).is_empty());
        assert_eq!(source.parts.len(), 1);
        assert_eq!(source.answer_slots.len(), 1);
    }

    #[test]
    fn complete_exam_scope_enforces_four_parts_and_forty_questions() {
        let mut source = fixture();
        source.meta.scope = ListeningScopeV2::CompleteExam;
        let issues = validate_listening_exam_source_v1(&source);
        assert!(issues
            .iter()
            .any(|issue| issue.code == "LISTENING_COMPLETE_PART_COUNT"));
        assert!(issues
            .iter()
            .any(|issue| issue.code == "LISTENING_COMPLETE_QUESTION_COUNT"));
    }

    #[test]
    fn invalid_cue_and_audio_hash_fail_closed() {
        let mut source = fixture();
        source.parts[0].cue.as_mut().unwrap().end_ms = source.media.duration_ms + 1;
        source.media.sha256 = "c".repeat(64);
        let issues = validate_listening_exam_source_v1(&source);
        assert!(issues.iter().any(|issue| issue.code == "AUDIO_CUE_INVALID"));
        assert!(issues
            .iter()
            .any(|issue| issue.code == "AUDIO_HASH_MISMATCH"));
    }

    #[test]
    fn attempt_is_bound_to_source_revision_media_and_playback_policy() {
        let source = fixture();
        let attempt = ListeningAttemptV1 {
            schema_version: LISTENING_ATTEMPT_V1_SCHEMA_VERSION.to_string(),
            exam_id: source.exam_id.clone(),
            source_revision: source.audit.source_revision + 1,
            answers: BTreeMap::new(),
            playback: ListeningPlaybackSnapshotV1 {
                media_asset_id: "other-audio".to_string(),
                policy_mode: ListeningPlaybackModeV2::Mock,
                plays_started: 0,
                position_ms: 0,
                status: ListeningPlaybackStatusV1::Ready,
                last_transition_at: "2026-08-12T00:00:00Z".to_string(),
                failure_code: None,
            },
            state: ListeningAttemptStateV1::NotStarted,
            updated_at: "2026-08-12T00:00:00Z".to_string(),
            submitted_at: None,
        };
        let issues = validate_listening_attempt_v1(&source, &attempt);
        assert!(issues
            .iter()
            .any(|issue| issue.code == "RUNTIME_ATTEMPT_REVISION_MISMATCH"));
        assert!(issues
            .iter()
            .any(|issue| issue.code == "AUDIO_HASH_MISMATCH"));
        assert!(issues
            .iter()
            .any(|issue| issue.code == "AUDIO_POLICY_MISSING"));
    }

    #[test]
    fn mock_policy_and_serialized_playback_state_fail_closed() {
        let mut source = fixture();
        source.playback_policy = ListeningPlaybackPolicyV2 {
            mode: ListeningPlaybackModeV2::Mock,
            autoplay: Some(true),
            allow_pause: false,
            allow_seek: false,
            allow_replay: false,
            max_plays: Some(1),
            refresh_behavior: ListeningRecoveryBehaviorV2::ResumeFromSnapshot,
            crash_recovery_behavior: ListeningRecoveryBehaviorV2::ResumeFromSnapshot,
            show_current_time: false,
            show_duration: false,
        };
        assert!(!validate_listening_exam_source_v1(&source)
            .iter()
            .any(|issue| issue.code == "AUDIO_POLICY_MISSING"));

        let attempt = ListeningAttemptV1 {
            schema_version: LISTENING_ATTEMPT_V1_SCHEMA_VERSION.to_string(),
            exam_id: source.exam_id.clone(),
            source_revision: source.audit.source_revision,
            answers: BTreeMap::new(),
            playback: ListeningPlaybackSnapshotV1 {
                media_asset_id: source.media.asset_id.clone(),
                policy_mode: ListeningPlaybackModeV2::Mock,
                plays_started: 0,
                position_ms: 200,
                status: ListeningPlaybackStatusV1::Playing,
                last_transition_at: "2026-08-12T00:00:00Z".to_string(),
                failure_code: None,
            },
            state: ListeningAttemptStateV1::InProgress,
            updated_at: "2026-08-12T00:00:00Z".to_string(),
            submitted_at: None,
        };
        assert!(validate_listening_attempt_v1(&source, &attempt)
            .iter()
            .any(|issue| issue.code == "AUDIO_POLICY_MISSING"));

        source.playback_policy.allow_seek = true;
        assert!(validate_listening_exam_source_v1(&source)
            .iter()
            .any(|issue| issue.code == "AUDIO_POLICY_MISSING"));
    }
}
