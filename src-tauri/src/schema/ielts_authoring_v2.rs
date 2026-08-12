use super::common::{AssetDescriptorV2, SourceAnchorV2};
use super::content_doc_v2::{ContentNodeV2, ProvenanceStatusV2};
use super::quality_report_v2::QualityReportV2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const IELTS_AUTHORING_IR_V2_SCHEMA_VERSION: &str = "IeltsAuthoringIRV2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExamModalityV2 {
    Reading,
    Listening,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ExamMetaV2 {
    pub exam_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<PassageCategoryV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<FrequencyV2>,
    pub language: String,
    pub tags: Vec<String>,
    pub source_files: Vec<ExamSourceFileV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PassageCategoryV2 {
    #[serde(rename = "P1")]
    P1,
    #[serde(rename = "P2")]
    P2,
    #[serde(rename = "P3")]
    P3,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FrequencyV2 {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ExamSourceFileV2 {
    pub source_file_id: String,
    pub role: ExamSourceRoleV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExamSourceRoleV2 {
    QuestionPaper,
    AnswerKey,
    Audio,
    Transcript,
    Supplement,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ReadingPassageV2 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub content: Vec<ContentNodeV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paragraph_map: Option<BTreeMap<String, String>>,
    pub source_anchors: Vec<SourceAnchorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ListeningScopeV2 {
    CompleteExam,
    PartialPractice,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningMediaV2 {
    pub asset_id: String,
    pub mime: String,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_rate_hz: Option<u32>,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ListeningCueOriginV2 {
    Manual,
    TimestampedTranscript,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningCueV2 {
    pub start_ms: u64,
    pub end_ms: u64,
    pub origin: ListeningCueOriginV2,
    pub confidence: f64,
    pub confirmed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_anchors: Option<Vec<SourceAnchorV2>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningPartV2 {
    pub part_id: String,
    pub display_label: String,
    pub expected_question_numbers: Vec<u32>,
    pub task_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cue: Option<ListeningCueV2>,
    pub source_anchors: Vec<SourceAnchorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ListeningPlaybackModeV2 {
    Practice,
    Mock,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ListeningRecoveryBehaviorV2 {
    ResumeFromSnapshot,
    RestartIfAllowed,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningPlaybackPolicyV2 {
    pub mode: ListeningPlaybackModeV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoplay: Option<bool>,
    pub allow_pause: bool,
    pub allow_seek: bool,
    pub allow_replay: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_plays: Option<u32>,
    pub refresh_behavior: ListeningRecoveryBehaviorV2,
    pub crash_recovery_behavior: ListeningRecoveryBehaviorV2,
    pub show_current_time: bool,
    pub show_duration: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningTranscriptSegmentV2 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_anchors: Option<Vec<SourceAnchorV2>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningTranscriptV2 {
    pub provided_by_user: bool,
    pub segments: Vec<ListeningTranscriptSegmentV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ListeningStructureV2 {
    pub scope: ListeningScopeV2,
    pub media: ListeningMediaV2,
    pub parts: Vec<ListeningPartV2>,
    pub playback_policy: ListeningPlaybackPolicyV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<ListeningTranscriptV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskTypeV2 {
    SingleChoice,
    MultipleChoice,
    TrueFalseNotGiven,
    YesNoNotGiven,
    MatchingInformation,
    MatchingHeadings,
    MatchingFeatures,
    MatchingSentenceEndings,
    Classification,
    SentenceCompletion,
    SummaryCompletion,
    NoteCompletion,
    TableCompletion,
    FormCompletion,
    FlowchartCompletion,
    DiagramLabelCompletion,
    PlanMapLabelCompletion,
    ShortAnswer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuestionNumberExpressionV2 {
    Range { start: u32, end: u32 },
    Set { values: Vec<u32> },
    Mixed { values: Vec<QuestionNumberValueV2> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum QuestionNumberValueV2 {
    Number(u32),
    Range { start: u32, end: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TaskGroupV2 {
    pub task_id: String,
    pub display_range: QuestionNumberExpressionV2,
    pub task_type: TaskTypeV2,
    pub instructions: Vec<ContentNodeV2>,
    pub instruction_signature: InstructionSignatureV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stimulus: Option<Vec<ContentNodeV2>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_bank: Option<OptionBankV2>,
    pub response_groups: Vec<ResponseGroupV2>,
    pub source_anchors: Vec<SourceAnchorV2>,
    pub quality: GroupQualityV2,
    pub review_state: ReviewStateV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStateV2 {
    Unreviewed,
    Confirmed,
    Edited,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct InstructionSignatureV2 {
    pub normalized_text: String,
    pub task_type: TaskTypeV2,
    pub expected_question_numbers: Vec<u32>,
    pub expected_slot_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_alphabet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_cardinality: Option<CardinalityV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_assignment: Option<AssignmentV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_option_reuse: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_limit: Option<WordLimitV2>,
    pub evidence_anchors: Vec<SourceAnchorV2>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct WordLimitV2 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_words: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_numbers: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words_and_or_number: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct OptionV2 {
    pub option_id: String,
    pub label: String,
    pub content: Vec<ContentNodeV2>,
    pub source_anchors: Vec<SourceAnchorV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance_status: Option<ProvenanceStatusV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct OptionBankV2 {
    pub option_bank_id: String,
    pub scope: OptionBankScopeV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<Vec<ContentNodeV2>>,
    pub options: Vec<OptionV2>,
    pub allow_reuse: bool,
    pub source_anchors: Vec<SourceAnchorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OptionBankScopeV2 {
    TaskGroup,
    Document,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ResponseGroupV2 {
    pub response_group_id: String,
    pub kind: ResponseGroupKindV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Vec<ContentNodeV2>>,
    pub slot_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<OptionV2>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_bank_ref: Option<String>,
    pub cardinality: CardinalityV2,
    pub assignment: AssignmentV2,
    pub scoring_policy: ResponseScoringPolicyV2,
    pub duplicate_policy: DuplicateSelectionPolicyV2,
    pub allow_option_reuse: bool,
    pub source_anchors: Vec<SourceAnchorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseScoringPolicyV2 {
    PerSlotBinary,
    PerSlotIeltsNormalized,
    ExactSet,
    AllOrNothing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateSelectionPolicyV2 {
    RejectSubmission,
    IgnoreDuplicates,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseGroupKindV2 {
    Choice,
    TextEntry,
    Matching,
    DiagramHotspot,
    Composite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CardinalityV2 {
    pub min: u32,
    pub max: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentV2 {
    PerSlot,
    UnorderedSet,
    OrderedSlots,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AnswerSlotV2 {
    pub slot_id: String,
    pub question_number: u32,
    pub display_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_node_id: Option<String>,
    pub host_type: AnswerSlotHostTypeV2,
    pub interaction: InteractionV2,
    pub participation: AnswerSlotParticipationV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<AnswerConstraintsV2>,
    pub source_anchors: Vec<SourceAnchorV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance_status: Option<ProvenanceStatusV2>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerSlotParticipationV2 {
    Scoring,
    Example,
    NonScoring,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerSlotHostTypeV2 {
    Prompt,
    Paragraph,
    TableCell,
    FigureHotspot,
    FlowStep,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InteractionV2 {
    Radio,
    Checkbox,
    Text,
    Select,
    Dragdrop,
    Hotspot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AnswerConstraintsV2 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_words: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_numbers: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_characters: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_option_labels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnswerValueV2 {
    Text {
        values: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        normalization: Option<AnswerNormalizationV2>,
    },
    Option {
        labels: Vec<String>,
        assignment: AnswerAssignmentV2,
    },
    Unresolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerNormalizationV2 {
    IeltsDefault,
    Exact,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerAssignmentV2 {
    PerSlot,
    UnorderedSet,
    Ordered,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct GroupQualityV2 {
    pub score: f64,
    pub source_coverage: f64,
    pub hard_failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RevisionSourceV2 {
    AutoExtract,
    User,
    Migration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AuthoringAuditV2 {
    pub revision: u64,
    pub source: RevisionSourceV2,
    pub human_verified: bool,
    pub llm_used: bool,
    pub updated_at: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct IeltsAuthoringIRV2 {
    pub schema_version: String,
    pub job_id: String,
    pub exam: ExamMetaV2,
    pub modality: ExamModalityV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passage: Option<ReadingPassageV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listening: Option<ListeningStructureV2>,
    pub task_groups: Vec<TaskGroupV2>,
    pub answer_slots: BTreeMap<String, AnswerSlotV2>,
    pub answer_key: BTreeMap<String, AnswerValueV2>,
    pub assets: Vec<AssetDescriptorV2>,
    pub source_document_id: String,
    pub quality: QualityReportV2,
    pub audit: AuthoringAuditV2,
}

impl IeltsAuthoringIRV2 {
    pub fn is_supported_schema_version(&self) -> bool {
        self.schema_version == IELTS_AUTHORING_IR_V2_SCHEMA_VERSION
    }
}
