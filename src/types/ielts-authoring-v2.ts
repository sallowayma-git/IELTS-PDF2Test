import type {
  AssetDescriptorV2,
  SourceAnchorV2
} from "./schema-common-v2";
import type { ContentNodeV2 } from "./content-doc-v2";
import type { QualityReportV2 } from "./quality-report-v2";

export type ExamModality = "reading" | "listening";

export interface IeltsAuthoringIRV2 {
  schemaVersion: "IeltsAuthoringIRV2";
  jobId: string;
  exam: ExamMetaV2;
  modality: ExamModality;
  passage?: ReadingPassageV2;
  listening?: ListeningStructureV2;
  taskGroups: TaskGroupV2[];
  answerSlots: Record<string, AnswerSlotV2>;
  answerKey: Record<string, AnswerValueV2>;
  assets: AssetDescriptorV2[];
  sourceDocumentId: string;
  quality: QualityReportV2;
  audit: AuthoringAuditV2;
}

export interface ExamMetaV2 {
  examId: string;
  title: string;
  category?: "P1" | "P2" | "P3";
  frequency?: "low" | "medium" | "high";
  language: string;
  tags: string[];
  sourceFiles: Array<{
    sourceFileId: string;
    role: "question_paper" | "answer_key" | "audio" | "transcript" | "supplement";
  }>;
}

export interface ReadingPassageV2 {
  title?: string;
  content: ContentNodeV2[];
  paragraphMap?: Record<string, string>;
  sourceAnchors: SourceAnchorV2[];
}

export interface ListeningStructureV2 {
  scope: ListeningScopeV2;
  media: ListeningMediaV2;
  parts: ListeningPartV2[];
  playbackPolicy: ListeningPlaybackPolicyV2;
  transcript?: ListeningTranscriptV2;
}

export type ListeningScopeV2 = "complete_exam" | "partial_practice";

export interface ListeningMediaV2 {
  assetId: string;
  mime: string;
  durationMs: number;
  channels?: number;
  sampleRateHz?: number;
  sha256: string;
}

export type ListeningCueOriginV2 = "manual" | "timestamped_transcript";

export interface ListeningCueV2 {
  startMs: number;
  endMs: number;
  origin: ListeningCueOriginV2;
  confidence: number;
  confirmed: boolean;
  sourceAnchors?: SourceAnchorV2[];
}

export interface ListeningPartV2 {
  partId: string;
  displayLabel: string;
  expectedQuestionNumbers: number[];
  taskIds: string[];
  cue?: ListeningCueV2;
  sourceAnchors: SourceAnchorV2[];
}

export type ListeningPlaybackModeV2 = "practice" | "mock";
export type ListeningRecoveryBehaviorV2 = "resume_from_snapshot" | "restart_if_allowed" | "block";

export interface ListeningPlaybackPolicyV2 {
  mode: ListeningPlaybackModeV2;
  autoplay?: boolean;
  allowPause: boolean;
  allowSeek: boolean;
  allowReplay: boolean;
  maxPlays?: number;
  refreshBehavior: ListeningRecoveryBehaviorV2;
  crashRecoveryBehavior: ListeningRecoveryBehaviorV2;
  showCurrentTime: boolean;
  showDuration: boolean;
}

export interface ListeningTranscriptSegmentV2 {
  startMs?: number;
  endMs?: number;
  speaker?: string;
  text: string;
  sourceAnchors?: SourceAnchorV2[];
}

export interface ListeningTranscriptV2 {
  providedByUser: boolean;
  segments: ListeningTranscriptSegmentV2[];
}

export type TaskTypeV2 =
  | "single_choice"
  | "multiple_choice"
  | "true_false_not_given"
  | "yes_no_not_given"
  | "matching_information"
  | "matching_headings"
  | "matching_features"
  | "matching_sentence_endings"
  | "classification"
  | "sentence_completion"
  | "summary_completion"
  | "note_completion"
  | "table_completion"
  | "form_completion"
  | "flowchart_completion"
  | "diagram_label_completion"
  | "plan_map_label_completion"
  | "short_answer";

export interface TaskGroupV2 {
  taskId: string;
  displayRange: QuestionNumberExpressionV2;
  taskType: TaskTypeV2;
  instructions: ContentNodeV2[];
  instructionSignature: InstructionSignatureV2;
  recognitionWarnings?: string[];
  stimulus?: ContentNodeV2[];
  optionBank?: OptionBankV2;
  responseGroups: ResponseGroupV2[];
  sourceAnchors: SourceAnchorV2[];
  quality: GroupQualityV2;
  reviewState: "unreviewed" | "confirmed" | "edited";
}

export type QuestionNumberExpressionV2 =
  | { kind: "range"; start: number; end: number }
  | { kind: "set"; values: number[] }
  | { kind: "mixed"; values: Array<number | { start: number; end: number }> };

export interface InstructionSignatureV2 {
  normalizedText: string;
  taskType: TaskTypeV2;
  expectedQuestionNumbers: number[];
  expectedSlotCount: number;
  optionAlphabet?: "A-D" | "A-E" | "A-I" | "roman" | "paragraph_letters" | string;
  selectionCardinality?: { min: number; max: number; exact?: number };
  answerAssignment?: "per_slot" | "unordered_set" | "ordered_slots";
  allowOptionReuse?: boolean;
  wordLimit?: { maxWords?: number; maxNumbers?: number; wordsAndOrNumber?: boolean };
  evidenceAnchors: SourceAnchorV2[];
  confidence: number;
}

export interface OptionV2 {
  optionId: string;
  label: string;
  content: ContentNodeV2[];
  sourceAnchors: SourceAnchorV2[];
  provenanceStatus?: "source" | "derived" | "user_edited" | "manual";
}

export interface OptionBankV2 {
  optionBankId: string;
  scope: "task_group" | "document";
  title?: ContentNodeV2[];
  options: OptionV2[];
  allowReuse: boolean;
  sourceAnchors: SourceAnchorV2[];
}

export interface ResponseGroupV2 {
  responseGroupId: string;
  kind: "choice" | "text_entry" | "matching" | "diagram_hotspot" | "composite";
  prompt?: ContentNodeV2[];
  slotIds: string[];
  options?: OptionV2[];
  optionBankRef?: string;
  cardinality: { min: number; max: number; exact?: number };
  assignment: "per_slot" | "unordered_set" | "ordered_slots";
  scoringPolicy: "per_slot_binary" | "per_slot_ielts_normalized" | "exact_set" | "all_or_nothing";
  duplicatePolicy: "reject_submission" | "ignore_duplicates";
  allowOptionReuse: boolean;
  sourceAnchors: SourceAnchorV2[];
}

export interface AnswerSlotV2 {
  slotId: string;
  questionNumber: number;
  displayLabel: string;
  hostNodeId?: string;
  hostType: "prompt" | "paragraph" | "table_cell" | "figure_hotspot" | "flow_step";
  interaction: "radio" | "checkbox" | "text" | "select" | "dragdrop" | "hotspot";
  participation: "scoring" | "example" | "non_scoring";
  constraints?: {
    maxWords?: number;
    maxNumbers?: number;
    maxCharacters?: number;
    acceptedOptionLabels?: string[];
  };
  sourceAnchors: SourceAnchorV2[];
  provenanceStatus?: "source" | "derived" | "user_edited" | "manual";
  confidence: number;
}

export type AnswerValueV2 =
  | { kind: "text"; values: string[]; normalization?: "ielts_default" | "exact" }
  | { kind: "option"; labels: string[]; assignment: "per_slot" | "unordered_set" | "ordered" }
  | { kind: "unresolved" };

export interface GroupQualityV2 {
  score: number;
  sourceCoverage: number;
  hardFailures: string[];
}

export interface AuthoringAuditV2 {
  revision: number;
  source: "auto_extract" | "user" | "migration";
  humanVerified: boolean;
  llmUsed: boolean;
  updatedAt: string;
  notes: string[];
}

export const IELTS_AUTHORING_IR_V2_SCHEMA_VERSION = "IeltsAuthoringIRV2" as const;
