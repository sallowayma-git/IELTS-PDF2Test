import { describe, expect, it } from "vitest";
import type { TaskTypeV2 } from "./ielts-authoring-v2";
import { TASK_PRESENTATION_RULES, ruleFor } from "./taskPresentation";

// Evidence level: pure unit（规则表 JSON 由 Rust 测试生成并比对，这里只验前端查表）。

const ALL_TASK_TYPES = {
  single_choice: true,
  multiple_choice: true,
  true_false_not_given: true,
  yes_no_not_given: true,
  matching_information: true,
  matching_headings: true,
  matching_features: true,
  matching_sentence_endings: true,
  classification: true,
  sentence_completion: true,
  summary_completion: true,
  note_completion: true,
  table_completion: true,
  form_completion: true,
  flowchart_completion: true,
  diagram_label_completion: true,
  plan_map_label_completion: true,
  short_answer: true,
} satisfies Record<TaskTypeV2, true>;

describe("task presentation rules", () => {
  it("has a default rule for every task type", () => {
    for (const taskType of Object.keys(ALL_TASK_TYPES) as TaskTypeV2[]) {
      const rule = ruleFor(taskType, false);
      expect(rule.taskType).toBe(taskType);
      expect(rule.variant).toBe("default");
      expect(rule.hostTypes.length).toBeGreaterThan(0);
    }
    expect(new Set(TASK_PRESENTATION_RULES.map((rule) => rule.taskType)).size).toBe(
      Object.keys(ALL_TASK_TYPES).length,
    );
  });

  it("uses bank variants only for summary and plan/map completion", () => {
    expect(ruleFor("summary_completion", true)).toMatchObject({
      variant: "word_bank",
      presentation: "inline_dropzone",
      hostPlacement: "inline_stimulus",
      interaction: "dragdrop",
      optionSource: "option_bank",
      groupGranularity: "task_group",
    });
    expect(ruleFor("summary_completion", false).presentation).toBe("inline_text");
    expect(ruleFor("plan_map_label_completion", true)).toMatchObject({
      variant: "letters",
      responseKind: "matching",
      presentation: "row_dropzone",
    });
    expect(ruleFor("plan_map_label_completion", false).presentation).toBe("inline_text");
    expect(ruleFor("short_answer", true).variant).toBe("default");
    expect(ruleFor("diagram_label_completion", false)).toMatchObject({
      interaction: "text",
      presentation: "figure_inputs",
      hostPlacement: "figure_below",
    });
    expect(ruleFor("single_choice", false)).toMatchObject({
      optionSource: "per_slot_options",
      optionAlphabet: "letters_abcd",
      groupGranularity: "per_slot",
    });
    expect(ruleFor("multiple_choice", false)).toMatchObject({
      responseKind: "choice",
      assignment: "unordered_set",
      interaction: "checkbox",
      optionSource: "option_bank",
      groupGranularity: "task_group",
    });
  });

  it("places headings before passage paragraphs and keeps matching information as a per-slot matrix", () => {
    expect(ruleFor("matching_headings", true)).toMatchObject({
      presentation: "paragraph_dropzone",
      interaction: "dragdrop",
      hostPlacement: "passage_before_paragraph",
      optionAlphabet: "roman",
      optionReusePolicy: "never",
      exampleSlotPolicy: "no_answer_control",
      hostTypes: ["passage_paragraph"],
      groupGranularity: "task_group",
    });
    expect(ruleFor("matching_information", true)).toMatchObject({
      presentation: "radio_matrix",
      optionSource: "paragraph_map",
      hostPlacement: "question_row",
      groupGranularity: "per_slot",
      optionReusePolicy: "always",
      optionReuseDefault: true,
    });
    expect(ruleFor("matching_features", true)).toMatchObject({
      presentation: "row_dropzone",
      optionReusePolicy: "instruction_controlled",
      optionReuseDefault: false,
      groupGranularity: "task_group",
    });
  });

  it("carries the fixed truth labels for TFNG and YNNG", () => {
    expect(ruleFor("true_false_not_given", false).fixedOptionLabels).toEqual([
      "TRUE",
      "FALSE",
      "NOT GIVEN",
    ]);
    expect(ruleFor("yes_no_not_given", false).fixedOptionLabels).toEqual([
      "YES",
      "NO",
      "NOT GIVEN",
    ]);
  });
});
