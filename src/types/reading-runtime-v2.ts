import type { ContentNodeV2 } from "./content-doc-v2";
import type {
  AnswerSlotV2,
  AnswerValueV2,
  IeltsAuthoringIRV2,
  OptionV2,
  ResponseGroupV2,
  TaskGroupV2
} from "./ielts-authoring-v2";
import type { AssetDescriptorV2 } from "./schema-common-v2";

export const READING_EXAM_SOURCE_V2_SCHEMA_VERSION = "ReadingExamSourceV2" as const;
export const READING_ATTEMPT_V2_SCHEMA_VERSION = "ReadingAttemptV2" as const;

export interface RuntimeExamMetaV2 {
  title: string;
  language: string;
  category?: "P1" | "P2" | "P3";
}

export interface RuntimeAssetManifestRefV2 {
  examId: string;
  assets: AssetDescriptorV2[];
}

export interface RuntimePassageV2 {
  content: ContentNodeV2[];
  paragraphMap?: Record<string, string>;
}

export interface RuntimeAuditV2 {
  sourceSchemaVersion: "IeltsAuthoringIRV2";
  sourceDocumentId: string;
  sourceRevision: number;
  sourceRevisionKind: "auto_extract" | "user" | "migration";
}

/** The serialized V2 source consumed by a student loader. */
export interface ReadingExamSourceV2 {
  schemaVersion: typeof READING_EXAM_SOURCE_V2_SCHEMA_VERSION;
  examId: string;
  meta: RuntimeExamMetaV2;
  assets: RuntimeAssetManifestRefV2;
  passage: RuntimePassageV2;
  // The authoring task shape is intentionally retained in the source bundle so
  // the compiler cannot drop provenance/quality data before a student probe.
  taskGroups: TaskGroupV2[];
  answerSlots: Record<string, AnswerSlotV2>;
  answerKey: Record<string, AnswerValueV2>;
  questionOrder: string[];
  questionDisplayMap: Record<string, string>;
  audit: RuntimeAuditV2;
}

export interface LegacyReadingExamSourceV1 {
  schemaVersion?: string;
  examId?: string;
  [key: string]: unknown;
}

export type NormalizedReadingSourceV2 =
  | { version: "v2"; source: ReadingExamSourceV2 }
  | { version: "v1"; source: LegacyReadingExamSourceV1 };

export interface ReadingRuntimeOptionV2 extends OptionV2 {}

export interface ReadingRuntimeResponseGroupV2 {
  taskId: string;
  responseGroupId: string;
  kind: ResponseGroupV2["kind"];
  slotIds: string[];
  options: ReadingRuntimeOptionV2[];
  cardinality: ResponseGroupV2["cardinality"];
  assignment: ResponseGroupV2["assignment"];
  scoringPolicy: ResponseGroupV2["scoringPolicy"];
  duplicatePolicy: ResponseGroupV2["duplicatePolicy"];
  allowOptionReuse: boolean;
}

export interface ReadingRuntimeSlotV2 {
  taskId: string;
  responseGroupId: string;
  slot: AnswerSlotV2;
  options: ReadingRuntimeOptionV2[];
}

/** Framework-neutral interaction graph. UI code must render this graph, not infer DOM names. */
export interface ReadingRuntimeInteractionModelV2 {
  schemaVersion: "ReadingInteractionModelV2";
  examId: string;
  sourceRevision: number;
  taskGroups: Array<{
    taskId: string;
    taskType: TaskGroupV2["taskType"];
    responseGroupIds: string[];
  }>;
  responseGroups: Record<string, ReadingRuntimeResponseGroupV2>;
  slots: Record<string, ReadingRuntimeSlotV2>;
}

export type ReadingAttemptStateV2 = "in_progress" | "submitted";

export interface ReadingAttemptV2 {
  schemaVersion: typeof READING_ATTEMPT_V2_SCHEMA_VERSION;
  examId: string;
  sourceRevision: number;
  answers: Record<string, AnswerValueV2>;
  state: ReadingAttemptStateV2;
  updatedAt: string;
  submittedAt?: string;
}

export interface ReadingRuntimeIssueV2 {
  code: string;
  targetId: string;
  message: string;
}

export interface ReadingAttemptScoreV2 {
  correct: boolean;
  earnedPoints: number;
  possiblePoints: number;
  slotScores: Record<string, number>;
  responseGroups: Record<string, {
    correct: boolean;
    earnedPoints: number;
    possiblePoints: number;
  }>;
}

export interface ExamAssetManifestV2 {
  schemaVersion: "ExamAssetManifestV2";
  examId: string;
  generatedAt: string;
  assets: Record<string, AssetDescriptorV2>;
}

export interface AssetResolutionV2 {
  assetId: string;
  mime: string;
  byteLength: number;
  sha256: string;
  /** Resolved by the host/provider; never exposed as an arbitrary file URI. */
  resourceUri: string;
}

export class ReadingRuntimeError extends Error {
  readonly code: string;
  readonly targetId?: string;

  constructor(code: string, message: string, targetId?: string) {
    super(message);
    this.name = "ReadingRuntimeError";
    this.code = code;
    this.targetId = targetId;
  }
}

export type ReadingAuthoringSource = IeltsAuthoringIRV2;
