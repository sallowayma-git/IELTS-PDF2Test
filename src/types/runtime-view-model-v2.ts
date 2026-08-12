import type { ContentNodeV2 } from "./content-doc-v2";
import type {
  AnswerSlotV2,
  AnswerValueV2,
  IeltsAuthoringIRV2,
  OptionV2,
  OptionBankV2,
  ResponseGroupV2,
  TaskGroupV2
} from "./ielts-authoring-v2";
import type { AssetDescriptorV2 } from "./schema-common-v2";

/**
 * Framework-neutral projection consumed by student renderers.
 *
 * The editor owns authoring IR; the runtime renderer must consume this
 * projection so React/Vue/native renderers cannot silently invent different
 * answer-slot or option semantics.
 */
export interface RuntimeViewModelV2 {
  schemaVersion: "RuntimeViewModelV2";
  examId: string;
  title: string;
  passage: ContentNodeV2[];
  taskGroups: RuntimeTaskGroupV2[];
  answerSlots: Record<string, AnswerSlotV2>;
  answerKey: Record<string, AnswerValueV2>;
  questionOrder: string[];
  questionDisplayMap: Record<string, string>;
  assets: AssetDescriptorV2[];
}
export interface RuntimeTaskGroupV2 {
  taskId: string;
  displayRange: TaskGroupV2["displayRange"];
  taskType: TaskGroupV2["taskType"];
  instructions: ContentNodeV2[];
  stimulus?: ContentNodeV2[];
  optionBank?: OptionBankV2;
  responseGroups: ResponseGroupV2[];
}

export interface ReadingInteractionModelV2 {
  schemaVersion: "ReadingInteractionModelV2";
  examId: string;
  taskGroups: ReadingInteractionTaskGroupV2[];
  responseGroups: Record<string, ReadingInteractionResponseGroupV2>;
  slots: Record<string, ReadingInteractionSlotV2>;
}

export interface ReadingInteractionTaskGroupV2 {
  taskId: string;
  taskType: TaskGroupV2["taskType"];
  responseGroupIds: string[];
}

export interface ReadingInteractionResponseGroupV2 {
  taskId: string;
  responseGroupId: string;
  kind: ResponseGroupV2["kind"];
  slotIds: string[];
  options: OptionV2[];
  cardinality: ResponseGroupV2["cardinality"];
  assignment: ResponseGroupV2["assignment"];
  scoringPolicy: ResponseGroupV2["scoringPolicy"];
  duplicatePolicy: ResponseGroupV2["duplicatePolicy"];
  allowOptionReuse: boolean;
}

export interface ReadingInteractionSlotV2 {
  taskId: string;
  responseGroupId: string;
  slot: AnswerSlotV2;
  options: OptionV2[];
}
