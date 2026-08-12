import type {
  AnswerValueV2,
  IeltsAuthoringIRV2,
  QuestionNumberExpressionV2,
  ResponseGroupV2,
  TaskTypeV2
} from "./ielts-authoring-v2";
import type { ContentNodeV2, DiagramHotspotV2 } from "./content-doc-v2";
import type { RevisionRecordV2 } from "./artifact-store-v2";
import type { SourceAnchorV2 } from "./schema-common-v2";
import type { ProvenanceStatus } from "./schema-common-v2";

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

export type AuthoringContentTargetV2 =
  | { kind: "passage" }
  | { kind: "taskInstructions"; taskId: string }
  | { kind: "taskStimulus"; taskId: string }
  | { kind: "responsePrompt"; responseGroupId: string }
  | { kind: "option"; optionId: string }
  | { kind: "node"; nodeId: string };

export type AuthoringNodeAttributeV2 = {
  align?: "left" | "center" | "right" | "justify";
  indentLevel?: number;
  level?: number;
  altText?: string;
  placeholder?: string;
  displayLabel?: string;
  inline?: boolean;
  label?: string;
  slotIds?: string[];
  display?: Record<string, unknown>;
  crop?: [number, number, number, number];
  provenanceStatus?: "source" | "derived" | "user_edited" | "manual";
};

export type AuthoringPatchV2 =
  | { op: "replaceText"; nodeId: string; from: number; to: number; text: string; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "setNodeAttrs"; nodeId: string; attrs: AuthoringNodeAttributeV2; removeAttrs?: Array<keyof AuthoringNodeAttributeV2>; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "replaceContent"; target: AuthoringContentTargetV2; content: ContentNodeV2[]; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "insertNode"; target: AuthoringContentTargetV2; index: number; node: ContentNodeV2; parentId?: string }
  | { op: "deleteNode"; nodeId: string; allowAnswerSlotRemoval?: boolean }
  | { op: "moveNode"; nodeId: string; target: AuthoringContentTargetV2; index: number; parentId?: string }
  | { op: "cropAsset"; nodeId: string; crop: [number, number, number, number] | null; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "setHotspot"; nodeId: string; hotspot: DiagramHotspotV2; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "removeHotspot"; nodeId: string; hotspotId: string; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "setTaskType"; taskId: string; taskType: TaskTypeV2; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "setQuestionExpression"; taskId: string; expression: QuestionNumberExpressionV2; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "setResponseCardinality"; taskId: string; responseGroupId: string; cardinality: ResponseGroupV2["cardinality"]; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "setResponseGroup"; taskId: string; responseGroup: ResponseGroupV2; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "setAnswer"; slotId: string; value: AnswerValueV2 }
  | { op: "bindSource"; entityId: string; anchors: SourceAnchorV2[]; preserveProvenance?: boolean; restoreProvenanceStatus?: ProvenanceStatus }
  | { op: "resolveIssue"; issueId: string; resolution: "resolved" | "ignored"; note?: string };

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

export interface AuthoringV2ExportResultV2 {
  receipt: {
    schemaVersion: "AuthoringV2ExportReceiptV1";
    jobId: string;
    examId: string;
    revision: number;
    outputDir: string;
    authoringPath: string;
    runtimePath: string;
    manifestPath: string;
    v1FilesRemainReadable: true;
    pdfPerQuestionLlmRepair: false;
  };
  outputDir: string;
  revision: number;
  examId: string;
}
