import type {
  AnswerSlotV2,
  AnswerValueV2,
  ListeningPartV2,
  ListeningPlaybackModeV2,
  ListeningPlaybackPolicyV2,
  ListeningScopeV2,
  ListeningTranscriptV2,
  TaskGroupV2
} from "./ielts-authoring-v2";
import type { AssetDescriptorV2 } from "./schema-common-v2";

export const LISTENING_EXAM_SOURCE_V1_SCHEMA_VERSION = "ListeningExamSourceV1" as const;
export const LISTENING_ATTEMPT_V1_SCHEMA_VERSION = "ListeningAttemptV1" as const;

export type ListeningAudioIssueCodeV1 =
  | "AUDIO_DECODE_FAILED"
  | "AUDIO_CODEC_UNSUPPORTED"
  | "AUDIO_HASH_MISMATCH"
  | "AUDIO_SEVERE_CLIPPING"
  | "AUDIO_NEAR_SILENT"
  | "AUDIO_CUE_INVALID"
  | "AUDIO_POLICY_MISSING";

export interface ListeningAudioProbeV1 {
  status: "passed" | "blocked";
  provider: string;
  providerVersion: string;
  probedAt: string;
  issueCodes: ListeningAudioIssueCodeV1[];
}

export interface ListeningRuntimeMediaV1 {
  assetId: string;
  mime: string;
  codec: string;
  container: string;
  durationMs: number;
  channels?: number;
  sampleRateHz?: number;
  sha256: string;
  probe: ListeningAudioProbeV1;
}

export interface ListeningRuntimeMetaV1 {
  title: string;
  language: string;
  scope: ListeningScopeV2;
}

export interface ListeningRuntimeAssetManifestRefV1 {
  examId: string;
  assets: AssetDescriptorV2[];
}

export interface ListeningRuntimeAuditV1 {
  sourceSchemaVersion: "IeltsAuthoringIRV2";
  sourceDocumentId: string;
  sourceRevision: number;
  sourceRevisionKind: "auto_extract" | "user" | "migration";
  minimumRuntimeVersion: string;
}

export interface ListeningExamSourceV1 {
  schemaVersion: typeof LISTENING_EXAM_SOURCE_V1_SCHEMA_VERSION;
  examId: string;
  meta: ListeningRuntimeMetaV1;
  assets: ListeningRuntimeAssetManifestRefV1;
  media: ListeningRuntimeMediaV1;
  parts: ListeningPartV2[];
  playbackPolicy: ListeningPlaybackPolicyV2;
  transcript?: ListeningTranscriptV2;
  taskGroups: TaskGroupV2[];
  answerSlots: Record<string, AnswerSlotV2>;
  answerKey: Record<string, AnswerValueV2>;
  questionOrder: string[];
  questionDisplayMap: Record<string, string>;
  audit: ListeningRuntimeAuditV1;
}

export type ListeningPlaybackStatusV1 = "ready" | "playing" | "paused" | "ended" | "restart_pending" | "failed";
export type ListeningPlaybackFailureCodeV1 =
  | "AUDIO_DECODE_FAILED"
  | "AUDIO_CODEC_UNSUPPORTED"
  | "AUDIO_HASH_MISMATCH"
  | "AUDIO_RECOVERY_BLOCKED";

export interface ListeningPlaybackSnapshotV1 {
  mediaAssetId: string;
  policyMode: ListeningPlaybackModeV2;
  playsStarted: number;
  positionMs: number;
  status: ListeningPlaybackStatusV1;
  lastTransitionAt: string;
  failureCode?: ListeningPlaybackFailureCodeV1;
}

export interface ListeningAttemptV1 {
  schemaVersion: typeof LISTENING_ATTEMPT_V1_SCHEMA_VERSION;
  examId: string;
  sourceRevision: number;
  answers: Record<string, AnswerValueV2>;
  playback: ListeningPlaybackSnapshotV1;
  state: "not_started" | "in_progress" | "submitted";
  updatedAt: string;
  submittedAt?: string;
}
