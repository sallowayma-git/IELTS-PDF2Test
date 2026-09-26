import type { AnswerValueV2 } from "../types";
import type { ListeningPartView } from "./listeningWorkspace";

/**
 * 底部题号导航（QuestionNavBar）的数据推导，纯函数、可单测。
 *
 * - 阅读稿：只有一个 section，名称 "Part 1"，永远 active、不可切换（基准阅读页同款）。
 * - 听力稿：listeningParts 的**每个** Part 都是一个可切换 section（没有 Part → 题组映射的
 *   Part 与 visibleTaskIds 一样显示全部题组）；section 的题目集合与 ExamCanvas 的
 *   shownTaskIds（visibleTaskIds）显示语义保持一致，切 Part 就是
 *   ExamCanvas 的 selectedPart state，页面与底部导航永远看同一份状态。
 * - 题位顺序 = 题面出现顺序（taskGroup → responseGroup → slotIds），与逐题列表一致。
 */

/** 导航只需要题组的骨架：taskId + 每个 responseGroup 的题位顺序。 */
export interface QuestionNavTaskGroup {
  taskId: string;
  responseGroups: ReadonlyArray<{ slotIds: readonly string[] }>;
}

export interface QuestionNavQuestion {
  slotId: string;
  displayNumber: string;
  answered: boolean;
  /** 预留字段：题位当前不可点时置位；现有推导路径都不产生。 */
  disabled?: boolean;
}

export interface QuestionNavSection {
  key: string;
  /** Part 序号（从 1 开始）；听力点击非当前 section 切换时回传给 ExamCanvas。 */
  ordinal: number;
  name: string;
  /** 当前 section 的已答进度文案，如 "3 of 13"。 */
  status: string;
  active: boolean;
  /** 只有非当前的听力 section 可点击切换。 */
  switchable: boolean;
  /** 听力 Part 音频标记：blocked = 绑定了音频但不可播放（红色，旧头部 is-blocked 语义）。 */
  audioBlocked?: boolean;
  /** 听力 Part 音频标记：missing = 该 Part 还没有音频（名字后缀「 ·」，旧头部同款）。 */
  audioMissing?: boolean;
  questions: QuestionNavQuestion[];
}

export interface QuestionNavModel {
  sections: QuestionNavSection[];
}

export interface QuestionNavInput {
  mode: "author" | "student";
  taskGroups: readonly QuestionNavTaskGroup[];
  questionDisplayMap: Readonly<Record<string, string>>;
  /** displayNumber 的回退来源（questionDisplayMap 缺该题位时用 displayLabel）。 */
  answerSlots: Readonly<Record<string, { displayLabel?: string }>>;
  /** author 模式的已答判据：题稿答案键（只读，导航绝不写回）。 */
  answerKey: Readonly<Record<string, AnswerValueV2>>;
  /** student 模式的已答判据：预览本地作答（永不写回题稿）。 */
  studentAnswers: Readonly<Record<string, readonly string[]>>;
  /** 听力稿的 Part 视图（ExamCanvas 传 listeningParts(authoring, bindings)，
   *  **带上音频绑定**，与 ListeningHeader 同一份数据）；缺省视为阅读稿。 */
  listeningParts?: readonly ListeningPartView[];
  /** 听力当前 Part（ExamCanvas 的 selectedPart state）。 */
  selectedPart?: number;
}

/** author 已答：text 看 values、option 看 labels，忽略空白串。 */
function isAnsweredKey(value: AnswerValueV2 | undefined): boolean {
  if (!value || value.kind === "unresolved") return false;
  const values = value.kind === "text" ? value.values : value.labels;
  return values.some((candidate) => candidate.trim().length > 0);
}

/** student 已答：预览本地作答非空（空白串不算）。 */
function isAnsweredPreview(values: readonly string[] | undefined): boolean {
  return (values ?? []).some((candidate) => candidate.trim().length > 0);
}

function statusOf(questions: readonly QuestionNavQuestion[]): string {
  return `${questions.filter((question) => question.answered).length} of ${questions.length}`;
}

export function buildQuestionNavModel(input: QuestionNavInput): QuestionNavModel {
  const { mode, taskGroups, questionDisplayMap, answerSlots, answerKey, studentAnswers } = input;
  // 按题面出现顺序收集题位，并记录题位 → 题组（听力按 Part 过滤时要用）。
  const slotIds: string[] = [];
  const taskIdOfSlot = new Map<string, string>();
  for (const task of taskGroups) {
    for (const response of task.responseGroups) {
      for (const slotId of response.slotIds) {
        if (!slotId || taskIdOfSlot.has(slotId)) continue;
        taskIdOfSlot.set(slotId, task.taskId);
        slotIds.push(slotId);
      }
    }
  }
  const questionOf = (slotId: string): QuestionNavQuestion => ({
    slotId,
    displayNumber: questionDisplayMap[slotId] ?? answerSlots[slotId]?.displayLabel ?? slotId,
    answered: mode === "author" ? isAnsweredKey(answerKey[slotId]) : isAnsweredPreview(studentAnswers[slotId])
  });

  const parts = input.listeningParts;
  if (!parts?.length) {
    // 阅读稿：单一 "Part 1"，永远 active、不可切换（基准阅读页同款）。
    const questions = slotIds.map(questionOf);
    return {
      sections: [{
        key: "part-1",
        ordinal: 1,
        name: "Part 1",
        status: statusOf(questions),
        active: true,
        switchable: false,
        questions
      }]
    };
  }

  // 听力稿：**每个 Part 都生成一个可切换 section**——不管它有没有 Part → 题组映射。
  // 没映射的 Part 与 visibleTaskIds 的行为一致（显示全部题组），保证导航计数和
  // 右侧题面永远对得上；在此之前「完全无映射退化为单一 Part 1」的捷径让底部导航
  // 失去了 Part 切换能力（与旧 ListeningHeader 行为相比是回退），已移除。
  const allTaskIds = new Set(taskGroups.map((task) => task.taskId));
  const currentPart = input.selectedPart ?? parts[0].ordinal;
  return {
    sections: parts.map((part) => {
      const visible = part.taskIds.length ? new Set(part.taskIds) : allTaskIds;
      const questions = slotIds
        .filter((slotId) => visible.has(taskIdOfSlot.get(slotId) ?? ""))
        .map(questionOf);
      const active = part.ordinal === currentPart;
      return {
        key: `part-${part.ordinal}`,
        ordinal: part.ordinal,
        name: part.label,
        status: statusOf(questions),
        active,
        switchable: !active,
        audioBlocked: Boolean(part.audio && !part.audio.playable),
        audioMissing: !part.audio,
        questions
      };
    })
  };
}

/** #submit-btn 的定位目标：先看当前 Part 的第一道未作答，再按顺序看其它 Part。 */
export function firstUnansweredSlot(model: QuestionNavModel): string | undefined {
  const active = model.sections.find((section) => section.active);
  const ordered = active
    ? [active, ...model.sections.filter((section) => section !== active)]
    : [...model.sections];
  for (const section of ordered) {
    const question = section.questions.find((candidate) => !candidate.answered && !candidate.disabled);
    if (question) return question.slotId;
  }
  return undefined;
}
