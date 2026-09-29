import type { AnswerSlotV2, ResponseGroupV2, TaskTypeV2 } from "./ielts-authoring-v2";
import rulesJson from "./taskPresentationRules.json";

// 规则数据由 src-tauri/src/schema/task_presentation.rs 生成并由其测试比对，不要手改 JSON。

export type PresentationKind =
  | "paragraph_dropzone"
  | "row_dropzone"
  | "inline_dropzone"
  | "radio_list"
  | "radio_matrix"
  | "checkbox_set"
  | "inline_text"
  | "figure_inputs";

export type HostPlacement =
  | "passage_before_paragraph"
  | "question_row"
  | "prompt_after_question_number"
  | "inline_stimulus"
  | "table_cell"
  | "flow_step"
  | "figure_below";

export type ExampleSlotPolicy = "no_answer_control";

export type OptionSource =
  | "none"
  | "fixed_truth_labels"
  | "per_slot_options"
  | "group_options"
  | "option_bank"
  | "paragraph_map";

export type OptionAlphabet =
  | "none"
  | "fixed_truth"
  | "letters"
  | "letters_abcd"
  | "roman"
  | "paragraph_letters";

export type GroupGranularity = "per_slot" | "task_group";

export type OptionReusePolicy = "not_applicable" | "always" | "never" | "instruction_controlled";

export type TaskPresentationVariant = "default" | "word_bank" | "letters";

export interface TaskPresentationRule {
  taskType: TaskTypeV2;
  variant: TaskPresentationVariant;
  responseKind: ResponseGroupV2["kind"];
  assignment: ResponseGroupV2["assignment"];
  interaction: AnswerSlotV2["interaction"];
  /** 第一个为默认宿主；其余为明确允许的兼容宿主。 */
  hostTypes: AnswerSlotV2["hostType"][];
  hostPlacement: HostPlacement;
  presentation: PresentationKind;
  exampleSlotPolicy: ExampleSlotPolicy;
  optionSource: OptionSource;
  optionAlphabet: OptionAlphabet;
  fixedOptionLabels: string[];
  optionReusePolicy: OptionReusePolicy;
  optionReuseDefault: boolean;
  groupGranularity: GroupGranularity;
}

export const TASK_PRESENTATION_RULES = rulesJson as readonly TaskPresentationRule[];

export function ruleFor(taskType: TaskTypeV2, hasOptionBank: boolean): TaskPresentationRule {
  const variant: TaskPresentationVariant =
    hasOptionBank && taskType === "summary_completion"
      ? "word_bank"
      : hasOptionBank && taskType === "plan_map_label_completion"
        ? "letters"
        : "default";
  const rule = TASK_PRESENTATION_RULES.find(
    (candidate) => candidate.taskType === taskType && candidate.variant === variant,
  );
  if (!rule) {
    throw new Error(`task presentation rule missing: ${taskType}/${variant}`);
  }
  return rule;
}
