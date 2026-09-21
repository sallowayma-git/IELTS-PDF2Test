use super::common::{AssetDescriptorV2, AssetKindV2};
use super::ielts_authoring_v2::{
    AnswerSlotParticipationV2, AnswerSlotV2, AnswerValueV2, ListeningPartV2,
    ListeningPlaybackModeV2, ListeningPlaybackPolicyV2, ListeningRecoveryBehaviorV2,
    ListeningScopeV2, ListeningStructureV2, ListeningTranscriptV2, RevisionSourceV2, TaskGroupV2,
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
    /// Complete-exam audio. Optional when every part carries section `media`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<ListeningRuntimeMediaV1>,
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

fn probe_passed(probe: Option<&ListeningAudioProbeV1>) -> bool {
    probe.is_some_and(|probe| {
        probe.status == ListeningAudioProbeStatusV1::Passed && probe.issue_codes.is_empty()
    })
}

/// Closes one media reference against the asset manifest: the asset must exist,
/// be audio of the same MIME, and match the declared hash and duration.
fn check_media_asset(
    issues: &mut Vec<ListeningContractIssueV1>,
    assets: &[AssetDescriptorV2],
    asset_id: &str,
    mime: &str,
    sha256: &str,
    duration_ms: u64,
) {
    match assets.iter().find(|asset| asset.asset_id == asset_id) {
        None => push_issue(issues, "ASSET_REFERENCE_MISSING", asset_id),
        Some(asset) => {
            if asset.kind != AssetKindV2::Audio || asset.mime != mime {
                push_issue(issues, "AUDIO_CODEC_UNSUPPORTED", asset_id);
            }
            if !asset.sha256.eq_ignore_ascii_case(sha256) {
                push_issue(issues, "AUDIO_HASH_MISMATCH", asset_id);
            }
            if asset
                .duration_ms
                .is_some_and(|duration| duration != duration_ms)
            {
                push_issue(issues, "AUDIO_DECODE_FAILED", asset_id);
            }
        }
    }
}

/// Duration of the exam-level or section media a playback snapshot binds to.
fn playback_media_duration_ms(source: &ListeningExamSourceV1, asset_id: &str) -> Option<u64> {
    source
        .media
        .iter()
        .filter(|media| media.asset_id == asset_id)
        .map(|media| media.duration_ms)
        .chain(
            source
                .parts
                .iter()
                .filter_map(|part| part.media.as_ref())
                .filter(|media| media.asset_id == asset_id)
                .map(|media| media.duration_ms),
        )
        .next()
}

/// Media closure for the authoring-side `ListeningStructureV2`: either one
/// complete-exam `media` or section media on every part. Every declared media
/// reference must resolve to an audio asset with the same hash. Probes are not
/// required while authoring; the published source requires passed probes.
pub fn validate_listening_structure_media_v2(
    structure: &ListeningStructureV2,
    assets: &[AssetDescriptorV2],
) -> Vec<ListeningContractIssueV1> {
    let mut issues = Vec::new();
    if let Some(media) = &structure.media {
        check_media_asset(
            &mut issues,
            assets,
            &media.asset_id,
            &media.mime,
            &media.sha256,
            media.duration_ms,
        );
    }
    for part in &structure.parts {
        match &part.media {
            Some(media) => check_media_asset(
                &mut issues,
                assets,
                &media.asset_id,
                &media.mime,
                &media.sha256,
                media.duration_ms,
            ),
            None if structure.media.is_none() => {
                push_issue(&mut issues, "LISTENING_MEDIA_MISSING", &part.part_id)
            }
            None => {}
        }
    }
    issues
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

    if let Some(media) = &source.media {
        if !probe_passed(Some(&media.probe)) {
            push_issue(&mut issues, "AUDIO_DECODE_FAILED", &media.asset_id);
        }
        check_media_asset(
            &mut issues,
            &source.assets.assets,
            &media.asset_id,
            &media.mime,
            &media.sha256,
            media.duration_ms,
        );
    }
    for part in &source.parts {
        match &part.media {
            Some(media) => {
                if !probe_passed(media.probe.as_ref()) {
                    push_issue(&mut issues, "AUDIO_DECODE_FAILED", &media.asset_id);
                }
                check_media_asset(
                    &mut issues,
                    &source.assets.assets,
                    &media.asset_id,
                    &media.mime,
                    &media.sha256,
                    media.duration_ms,
                );
            }
            None if source.media.is_none() => {
                push_issue(&mut issues, "LISTENING_MEDIA_MISSING", &part.part_id);
            }
            None => {}
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
    let ordered_slot_ids = source
        .question_order
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if all_slot_ids != ordered_slot_ids || ordered_slot_ids.len() != source.question_order.len() {
        push_issue(
            &mut issues,
            "RUNTIME_QUESTION_ORDER_INVALID",
            &source.exam_id,
        );
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
    let mut previous_cue_end_by_media: BTreeMap<String, u64> = BTreeMap::new();
    let mut expected_numbers_across_parts = BTreeSet::new();
    for part in &source.parts {
        if !part_ids.insert(part.part_id.clone()) {
            push_issue(&mut issues, "LISTENING_PART_ID_DUPLICATE", &part.part_id);
        }
        if let Some(cue) = &part.cue {
            // A cue is relative to the audio file the part plays from: its own
            // section media, else the complete-exam media.
            let (media_id, duration_ms) = match (&part.media, &source.media) {
                (Some(media), _) => (media.asset_id.as_str(), Some(media.duration_ms)),
                (None, Some(media)) => (media.asset_id.as_str(), Some(media.duration_ms)),
                (None, None) => ("", None),
            };
            let previous_cue_end = previous_cue_end_by_media.get(media_id).copied();
            let invalid = cue.start_ms >= cue.end_ms
                || duration_ms.is_none_or(|duration| cue.end_ms > duration)
                || previous_cue_end.is_some_and(|end| cue.start_ms < end)
                || !cue.confirmed
                || cue.confidence < MIN_CONFIRMED_CUE_CONFIDENCE;
            if invalid {
                push_issue(&mut issues, "AUDIO_CUE_INVALID", &part.part_id);
            }
            previous_cue_end_by_media.insert(media_id.to_string(), cue.end_ms);
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
                push_issue(&mut issues, "QUESTION_NUMBER_DUPLICATE", &part.part_id);
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
    let playback_duration = playback_media_duration_ms(source, &attempt.playback.media_asset_id);
    if playback_duration.is_none() {
        push_issue(
            &mut issues,
            "AUDIO_HASH_MISMATCH",
            &attempt.playback.media_asset_id,
        );
    }
    if attempt.playback.policy_mode != source.playback_policy.mode {
        push_issue(&mut issues, "AUDIO_POLICY_MISSING", &attempt.exam_id);
    }
    let media_duration_ms = playback_duration
        .or_else(|| source.media.as_ref().map(|media| media.duration_ms))
        .unwrap_or(0);
    if attempt.playback.position_ms > media_duration_ms {
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
                || attempt.playback.position_ms >= media_duration_ms
            {
                push_issue(&mut issues, "AUDIO_POLICY_MISSING", &attempt.exam_id);
            }
        }
        ListeningPlaybackStatusV1::Ended => {
            if attempt.playback.plays_started == 0
                || attempt.playback.position_ms != media_duration_ms
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

    const FOUR_PART_FIXTURE: &str = include_str!(
        "../../../fixtures/golden/synthetic/ielts/phase7-listening-four-part-media-source-v1.json"
    );

    fn four_part_fixture() -> ListeningExamSourceV1 {
        serde_json::from_str(FOUR_PART_FIXTURE).expect("four-part media fixture must deserialize")
    }

    fn codes(issues: &[ListeningContractIssueV1]) -> Vec<(&str, &str)> {
        issues
            .iter()
            .map(|issue| (issue.code.as_str(), issue.target_id.as_str()))
            .collect()
    }

    #[test]
    fn four_part_source_with_distinct_section_media_round_trips_and_validates() {
        let source = four_part_fixture();
        assert!(source.media.is_none());
        assert_eq!(source.parts.len(), 4);
        let asset_ids = source
            .parts
            .iter()
            .map(|part| part.media.as_ref().expect("part media").asset_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(asset_ids.len(), 4);
        assert_eq!(validate_listening_exam_source_v1(&source), Vec::new());

        let reserialized = serde_json::to_value(&source).unwrap();
        assert!(reserialized.get("media").is_none());
        for part in reserialized["parts"].as_array().unwrap() {
            assert!(part["media"]["probe"]["status"] == "passed");
        }
        let reparsed: ListeningExamSourceV1 = serde_json::from_value(reserialized).unwrap();
        assert_eq!(reparsed, source);
    }

    #[test]
    fn part_media_hash_mismatch_is_rejected() {
        let mut source = four_part_fixture();
        source.parts[2].media.as_mut().unwrap().sha256 = "f".repeat(64);
        let issues = validate_listening_exam_source_v1(&source);
        assert!(codes(&issues).contains(&("AUDIO_HASH_MISMATCH", "audio-section-3")));
    }

    #[test]
    fn part_media_with_unknown_asset_is_rejected() {
        let mut source = four_part_fixture();
        source.parts[1].media.as_mut().unwrap().asset_id = "audio-ghost".to_string();
        let issues = validate_listening_exam_source_v1(&source);
        assert!(codes(&issues).contains(&("ASSET_REFERENCE_MISSING", "audio-ghost")));
    }

    #[test]
    fn part_media_probe_must_pass() {
        let mut source = four_part_fixture();
        source.parts[0].media.as_mut().unwrap().probe = None;
        let issues = validate_listening_exam_source_v1(&source);
        assert!(codes(&issues).contains(&("AUDIO_DECODE_FAILED", "audio-section-1")));
    }

    #[test]
    fn part_without_media_and_no_exam_media_is_rejected() {
        let mut source = four_part_fixture();
        source.parts[3].media = None;
        let issues = validate_listening_exam_source_v1(&source);
        assert!(codes(&issues).contains(&("LISTENING_MEDIA_MISSING", "part-4")));

        let single = fixture();
        let mut uncovered = four_part_fixture();
        for part in &mut uncovered.parts {
            part.media = None;
        }
        uncovered
            .assets
            .assets
            .push(single.assets.assets[0].clone());
        uncovered.media = single.media.clone();
        let issues = validate_listening_exam_source_v1(&uncovered);
        assert!(!issues
            .iter()
            .any(|issue| issue.code == "LISTENING_MEDIA_MISSING"));
    }

    #[test]
    fn part_cue_is_bounded_by_its_own_media() {
        let mut source = four_part_fixture();
        // Section 1 media is 1500 ms; a cue that fits the longer section 4 media
        // is still out of bounds for section 1.
        source.parts[0].cue.as_mut().unwrap().end_ms = 4000;
        let issues = validate_listening_exam_source_v1(&source);
        assert!(codes(&issues).contains(&("AUDIO_CUE_INVALID", "part-1")));
        // Cues restart per media file, so overlapping absolute ranges across
        // different section files are not an overlap.
        let fresh = four_part_fixture();
        assert!(!validate_listening_exam_source_v1(&fresh)
            .iter()
            .any(|issue| issue.code == "AUDIO_CUE_INVALID"));
    }

    #[test]
    fn attempt_playback_may_bind_to_a_section_media() {
        let source = four_part_fixture();
        let attempt = ListeningAttemptV1 {
            schema_version: LISTENING_ATTEMPT_V1_SCHEMA_VERSION.to_string(),
            exam_id: source.exam_id.clone(),
            source_revision: source.audit.source_revision,
            answers: BTreeMap::new(),
            playback: ListeningPlaybackSnapshotV1 {
                media_asset_id: "audio-section-2".to_string(),
                policy_mode: ListeningPlaybackModeV2::Practice,
                plays_started: 1,
                position_ms: 2500,
                status: ListeningPlaybackStatusV1::Ended,
                last_transition_at: "2026-09-21T00:00:00Z".to_string(),
                failure_code: None,
            },
            state: ListeningAttemptStateV1::InProgress,
            updated_at: "2026-09-21T00:00:00Z".to_string(),
            submitted_at: None,
        };
        assert_eq!(validate_listening_attempt_v1(&source, &attempt), Vec::new());
        let mut unknown = attempt.clone();
        unknown.playback.media_asset_id = "audio-ghost".to_string();
        assert!(validate_listening_attempt_v1(&source, &unknown)
            .iter()
            .any(|issue| issue.code == "AUDIO_HASH_MISMATCH"));
    }

    #[test]
    fn authoring_structure_accepts_exam_media_or_complete_part_media() {
        use crate::schema::ielts_authoring_v2::ListeningStructureV2;
        let source = four_part_fixture();
        let mut structure = ListeningStructureV2 {
            scope: ListeningScopeV2::CompleteExam,
            media: None,
            parts: source.parts.clone(),
            playback_policy: source.playback_policy.clone(),
            transcript: None,
        };
        let assets = source.assets.assets.clone();
        assert_eq!(
            validate_listening_structure_media_v2(&structure, &assets),
            Vec::new()
        );

        structure.parts[0].media.as_mut().unwrap().sha256 = "e".repeat(64);
        structure.parts[1].media.as_mut().unwrap().asset_id = "audio-ghost".to_string();
        structure.parts[2].media = None;
        let issues = validate_listening_structure_media_v2(&structure, &assets);
        let found = codes(&issues);
        assert!(found.contains(&("AUDIO_HASH_MISMATCH", "audio-section-1")));
        assert!(found.contains(&("ASSET_REFERENCE_MISSING", "audio-ghost")));
        assert!(found.contains(&("LISTENING_MEDIA_MISSING", "part-3")));

        // A single complete-exam audio covers every part.
        let single = fixture();
        let whole = single.media.clone().unwrap();
        let mut single_structure = ListeningStructureV2 {
            scope: ListeningScopeV2::PartialPractice,
            media: Some(crate::schema::ielts_authoring_v2::ListeningMediaV2 {
                asset_id: whole.asset_id.clone(),
                mime: whole.mime.clone(),
                duration_ms: whole.duration_ms,
                channels: whole.channels,
                sample_rate_hz: whole.sample_rate_hz,
                sha256: whole.sha256.clone(),
            }),
            parts: single.parts.clone(),
            playback_policy: single.playback_policy.clone(),
            transcript: None,
        };
        assert_eq!(
            validate_listening_structure_media_v2(&single_structure, &single.assets.assets),
            Vec::new()
        );
        single_structure.media.as_mut().unwrap().sha256 = "d".repeat(64);
        assert!(codes(&validate_listening_structure_media_v2(
            &single_structure,
            &single.assets.assets
        ))
        .contains(&("AUDIO_HASH_MISMATCH", "audio-part1")));
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
        source.parts[0].cue.as_mut().unwrap().end_ms =
            source.media.as_ref().unwrap().duration_ms + 1;
        source.media.as_mut().unwrap().sha256 = "c".repeat(64);
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
                media_asset_id: source.media.as_ref().unwrap().asset_id.clone(),
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
