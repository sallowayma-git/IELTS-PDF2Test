import type {
  AnswerValueV2,
  IeltsAuthoringIRV2,
  QuestionNumberExpressionV2,
  ResponseGroupV2,
  TaskTypeV2
} from "./ielts-authoring-v2";
import type { RevisionRecordV2 } from "./artifact-store-v2";
import type { SourceAnchorV2 } from "./schema-common-v2";

export type AuthoringEditorSourceV2 = "shadow" | "revision" | "fixture";

export interface AuthoringEditorSessionV2 {
  schemaVersion: "AuthoringEditorSessionV1";
  jobId: string;
  authoring: IeltsAuthoringIRV2;
  revision: number;
  source: AuthoringEditorSourceV2;
  revisions: RevisionRecordV2[];
  v1FilesRemainReadable: true;
  savedPatchCount?: number;
}

export type AuthoringPatchV2 =
  | { op: "replaceText"; nodeId: string; from: number; to: number; text: string }
  | { op: "setNodeAttrs"; nodeId: string; attrs: Record<string, unknown> }
  | { op: "setTaskType"; taskId: string; taskType: TaskTypeV2 }
  | { op: "setQuestionExpression"; taskId: string; expression: QuestionNumberExpressionV2 }
  | { op: "setResponseGroup"; taskId: string; responseGroup: ResponseGroupV2 }
  | { op: "setAnswer"; slotId: string; value: AnswerValueV2 }
  | { op: "bindSource"; entityId: string; anchors: SourceAnchorV2[] };

export interface ApplyAuthoringV2PatchesInput {
  jobId: string;
  baseRevision: number;
  patches: AuthoringPatchV2[];
}

export interface AuthoringEditorRecoveryV2 {
  schemaVersion: "AuthoringEditorRecoveryV1";
  jobId: string;
  baseRevision: number;
  updatedAt: string;
  patches: AuthoringPatchV2[];
}
