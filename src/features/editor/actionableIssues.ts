import type { ContentNodeV2, IeltsAuthoringIRV2, ResponseGroupV2, TaskGroupV2 } from "../../types";

// ActionableIssue（计划 §8.5）：只呈现「第 12 题题干可能缺少下一行」这类可执行问题，
// 不把置信度、hash、schema 名当成用户任务。
//
// 后端的 typed ActionableIssue 与 reconciliation 是 P8 的工作。在它落地之前，这里对
// 当前 Canonical DS 做计划 §6.8 的硬闭包检查 —— 检查项与将来后端一致，
// 因此 P8 迁到后端时可以直接对比两边结果。
export type IssueSeverity = "blocker" | "warning";

export type IssueCode =
  | "QUESTION_PROMPT_MISSING"
  | "OPTION_TEXT_MISSING"
  | "OPTION_RUN_INCOMPLETE"
  | "SHARED_OPTION_BANK_MISSING"
  | "ANSWER_MISSING"
  | "ANSWER_UNRESOLVED";

export interface ActionableIssueV1 {
  issueId: string;
  /** 点击问题时要定位到的 DS 节点或实体 id。 */
  targetId: string;
  severity: IssueSeverity;
  code: IssueCode;
  /** 用户可读的一句话，例如「第 18 题 B 选项未识别」。 */
  userMessage: string;
}

const CHOICE_TASK_TYPES = new Set(["single_choice", "multiple_choice", "true_false_not_given", "yes_no_not_given"]);
const MATCHING_TASK_TYPES = new Set([
  "matching_information",
  "matching_headings",
  "matching_features",
  "matching_sentence_endings",
  "classification"
]);

function textOf(nodes: ContentNodeV2[] | undefined): string {
  if (!nodes?.length) return "";
  return nodes
    .map((node) => {
      if (node.type === "text") return node.text;
      if ("children" in node) return textOf(node.children);
      if ("items" in node) return node.items.map((item) => textOf(item.children)).join(" ");
      if ("rows" in node) return node.rows.map((row) => row.cells.map((cell) => textOf(cell.children)).join(" ")).join(" ");
      if ("steps" in node) return node.steps.map((step) => textOf(step.children)).join(" ");
      return "";
    })
    .join("");
}

/** 一个答案位在题面上的显示编号，用于「第 N 题」文案。 */
function displayLabel(ds: IeltsAuthoringIRV2, slotId: string): string {
  const slot = ds.answerSlots[slotId];
  if (!slot) return slotId;
  return slot.displayLabel || String(slot.questionNumber);
}

function questionsLabel(ds: IeltsAuthoringIRV2, group: ResponseGroupV2): string {
  const labels = group.slotIds.map((slotId) => displayLabel(ds, slotId)).filter(Boolean);
  if (!labels.length) return "这一题";
  if (labels.length === 1) return `第 ${labels[0]} 题`;
  return `第 ${labels.join("、")} 题`;
}

function checkChoiceGroup(ds: IeltsAuthoringIRV2, task: TaskGroupV2, group: ResponseGroupV2, issues: ActionableIssueV1[]): void {
  const options = group.options?.length ? group.options : task.optionBank?.options ?? [];
  const label = questionsLabel(ds, group);
  if (!options.length) {
    issues.push({
      issueId: `${group.responseGroupId}:options-missing`,
      targetId: group.responseGroupId,
      severity: "blocker",
      code: "OPTION_RUN_INCOMPLETE",
      userMessage: `${label}没有识别到选项。`
    });
    return;
  }
  for (const option of options) {
    if (textOf(option.content).trim()) continue;
    issues.push({
      issueId: `${option.optionId}:text-missing`,
      targetId: option.optionId,
      severity: "blocker",
      code: "OPTION_TEXT_MISSING",
      userMessage: `${label} ${option.label} 选项没有正文。`
    });
  }
}

function checkMatchingTask(ds: IeltsAuthoringIRV2, task: TaskGroupV2, issues: ActionableIssueV1[]): void {
  const bank = task.optionBank?.options ?? [];
  if (!bank.length) {
    issues.push({
      issueId: `${task.taskId}:bank-missing`,
      targetId: task.taskId,
      severity: "blocker",
      code: "SHARED_OPTION_BANK_MISSING",
      userMessage: "这组匹配题没有识别到共享选项列表。"
    });
  }
}

/** 对当前 Canonical DS 做硬闭包检查。返回顺序按题号，blocker 在前。 */
export function deriveActionableIssues(ds: IeltsAuthoringIRV2 | undefined): ActionableIssueV1[] {
  if (!ds) return [];
  const issues: ActionableIssueV1[] = [];

  for (const task of ds.taskGroups) {
    if (MATCHING_TASK_TYPES.has(task.taskType)) checkMatchingTask(ds, task, issues);

    for (const group of task.responseGroups) {
      const label = questionsLabel(ds, group);

      // 题干闭包：选择题与匹配题的每一项都必须有非空题干（计划 §2.2「任一简单题 prompt == empty 必须产生 blocker」）。
      const needsPrompt = CHOICE_TASK_TYPES.has(task.taskType) || MATCHING_TASK_TYPES.has(task.taskType);
      if (needsPrompt && !textOf(group.prompt).trim()) {
        issues.push({
          issueId: `${group.responseGroupId}:prompt-missing`,
          targetId: group.responseGroupId,
          severity: "blocker",
          code: "QUESTION_PROMPT_MISSING",
          userMessage: `${label}的题干是空的，需要补上。`
        });
      }

      if (CHOICE_TASK_TYPES.has(task.taskType) || group.kind === "choice") {
        checkChoiceGroup(ds, task, group, issues);
      }

      for (const slotId of group.slotIds) {
        const slot = ds.answerSlots[slotId];
        if (slot && slot.participation !== "scoring") continue;
        const answer = ds.answerKey[slotId];
        if (!answer || answer.kind === "unresolved") {
          issues.push({
            issueId: `${slotId}:answer-missing`,
            targetId: slotId,
            severity: "warning",
            code: answer?.kind === "unresolved" ? "ANSWER_UNRESOLVED" : "ANSWER_MISSING",
            userMessage: `第 ${displayLabel(ds, slotId)} 题还没有答案。`
          });
          continue;
        }
        const empty = answer.kind === "text"
          ? !answer.values.some((value) => value.trim())
          : answer.kind === "option" && !answer.labels.length;
        if (empty) {
          issues.push({
            issueId: `${slotId}:answer-empty`,
            targetId: slotId,
            severity: "warning",
            code: "ANSWER_MISSING",
            userMessage: `第 ${displayLabel(ds, slotId)} 题还没有答案。`
          });
        }
      }
    }
  }

  return issues.sort((a, b) => (a.severity === b.severity ? 0 : a.severity === "blocker" ? -1 : 1));
}

export function blockerCount(issues: readonly ActionableIssueV1[]): number {
  return issues.filter((issue) => issue.severity === "blocker").length;
}
