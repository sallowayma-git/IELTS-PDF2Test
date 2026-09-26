import { describe, expect, it } from "vitest";
import type { AnswerValueV2 } from "../types";
import type { ListeningPartView } from "./listeningWorkspace";
import { buildQuestionNavModel, firstUnansweredSlot, type QuestionNavInput } from "./questionNavModel";

// Evidence level: pure unit（底部题号导航的数据推导）。

const task = (taskId: string, ...slotIds: string[]) => ({
  taskId,
  responseGroups: slotIds.map((slotId) => ({ slotIds: [slotId] }))
});

const baseInput = (): QuestionNavInput => ({
  mode: "author",
  taskGroups: [task("g1", "q1", "q2"), task("g2", "q3")],
  questionDisplayMap: { q1: "1", q2: "2", q3: "3" },
  answerSlots: {},
  answerKey: {},
  studentAnswers: {}
});

const listeningPart = (ordinal: number, taskIds: string[], audio?: { playable: boolean }): ListeningPartView =>
  ({ ordinal, label: `Part ${ordinal}`, taskIds, audio }) as ListeningPartView;

describe("reading nav model", () => {
  it("produces a single active Part 1 section in pane order", () => {
    const model = buildQuestionNavModel({
      ...baseInput(),
      answerKey: { q1: { kind: "text", values: ["Paris"] } }
    });
    expect(model.sections).toHaveLength(1);
    const section = model.sections[0];
    expect([section.key, section.ordinal, section.name, section.active, section.switchable]).toEqual([
      "part-1", 1, "Part 1", true, false
    ]);
    expect(section.status).toBe("1 of 3");
    expect(section.questions.map((question) => [question.slotId, question.displayNumber, question.answered])).toEqual([
      ["q1", "1", true],
      ["q2", "2", false],
      ["q3", "3", false]
    ]);
  });

  it("judges author answers from the answer key only", () => {
    const answerKey: Record<string, AnswerValueV2> = {
      textFilled: { kind: "text", values: ["x"] },
      textBlank: { kind: "text", values: [""] },
      textSpaces: { kind: "text", values: ["  "] },
      optionPicked: { kind: "option", labels: ["A"], assignment: "per_slot" },
      optionEmpty: { kind: "option", labels: [], assignment: "per_slot" },
      unresolved: { kind: "unresolved" }
    };
    const model = buildQuestionNavModel({
      ...baseInput(),
      taskGroups: [
        task("g1", "textFilled", "textBlank", "textSpaces", "optionPicked", "optionEmpty", "unresolved")
      ],
      questionDisplayMap: {},
      answerKey
    });
    const answered = model.sections[0].questions.map((question) => [question.slotId, question.answered]);
    expect(answered).toEqual([
      ["textFilled", true],
      ["textBlank", false],
      ["textSpaces", false],
      ["optionPicked", true],
      ["optionEmpty", false],
      ["unresolved", false]
    ]);
  });

  it("judges student answers from preview state and ignores the answer key", () => {
    const model = buildQuestionNavModel({
      ...baseInput(),
      mode: "student",
      answerKey: { q1: { kind: "text", values: ["secret"] }, q3: { kind: "option", labels: ["A"], assignment: "per_slot" } },
      studentAnswers: { q1: ["typed"], q2: [] }
    });
    expect(model.sections[0].questions.map((question) => [question.slotId, question.answered])).toEqual([
      ["q1", true],
      ["q2", false],
      ["q3", false]
    ]);
  });

  it("falls back for display numbers: displayMap → slot displayLabel → slotId", () => {
    const model = buildQuestionNavModel({
      ...baseInput(),
      taskGroups: [task("g1", "a", "b")],
      questionDisplayMap: {},
      answerSlots: { b: { displayLabel: "9" } }
    });
    expect(model.sections[0].questions.map((question) => question.displayNumber)).toEqual(["a", "9"]);
  });
});

describe("listening nav model", () => {
  const input = () => ({
    ...baseInput(),
    listeningParts: [
      listeningPart(1, ["g1"]),
      listeningPart(2, ["g2"]),
      listeningPart(3, []),
      listeningPart(4, [])
    ]
  });

  it("splits sections by part and marks the selected part active", () => {
    const model = buildQuestionNavModel({ ...input(), selectedPart: 2 });
    expect(model.sections.map((section) => [section.ordinal, section.name, section.active, section.switchable])).toEqual([
      [1, "Part 1", false, true],
      [2, "Part 2", true, false],
      [3, "Part 3", false, true],
      [4, "Part 4", false, true]
    ]);
    expect(model.sections[0].questions.map((question) => question.slotId)).toEqual(["q1", "q2"]);
    expect(model.sections[1].questions.map((question) => question.slotId)).toEqual(["q3"]);
  });

  it("defaults the active part to Part 1 and reports per-part progress", () => {
    const model = buildQuestionNavModel({
      ...input(),
      mode: "student",
      studentAnswers: { q1: ["a"], q3: ["b"] }
    });
    expect(model.sections[0].status).toBe("1 of 2");
    expect(model.sections[1].status).toBe("1 of 1");
    expect(model.sections[0].active).toBe(true);
  });

  it("mirrors visibleTaskIds for parts without a task mapping (all groups show)", () => {
    const model = buildQuestionNavModel({ ...input(), selectedPart: 3 });
    expect(model.sections[2].questions.map((question) => question.slotId)).toEqual(["q1", "q2", "q3"]);
    expect(model.sections[2].status).toBe("0 of 3");
  });

  it("keeps every part switchable when no part maps tasks (all groups show)", () => {
    // 回退修复：完全无映射的听力稿不再退化成单一 Part 1——四个 Part 都可切换，
    // 每个 section 的题目集合与 visibleTaskIds 一致（显示全部题组）。
    const model = buildQuestionNavModel({
      ...baseInput(),
      listeningParts: [listeningPart(1, []), listeningPart(2, []), listeningPart(3, []), listeningPart(4, [])],
      selectedPart: 2
    });
    expect(model.sections).toHaveLength(4);
    expect(model.sections.map((section) => [section.ordinal, section.active, section.switchable])).toEqual([
      [1, false, true],
      [2, true, false],
      [3, false, true],
      [4, false, true]
    ]);
    for (const section of model.sections) {
      expect(section.questions.map((question) => question.slotId)).toEqual(["q1", "q2", "q3"]);
      expect(section.status).toBe("0 of 3");
    }
  });

  it("carries audio markers from part bindings (blocked / missing)", () => {
    const model = buildQuestionNavModel({
      ...baseInput(),
      listeningParts: [
        listeningPart(1, ["g1"], { playable: false }),
        listeningPart(2, ["g2"], { playable: true }),
        listeningPart(3, []),
        listeningPart(4, [])
      ],
      selectedPart: 1
    });
    expect(model.sections[0].audioBlocked).toBe(true);
    expect(model.sections[0].audioMissing).toBe(false);
    expect(model.sections[1].audioBlocked).toBe(false);
    expect(model.sections[1].audioMissing).toBe(false);
    expect(model.sections[2].audioMissing).toBe(true);
    expect(model.sections[3].audioMissing).toBe(true);
  });
});

describe("firstUnansweredSlot", () => {
  it("returns the first unanswered question in reading order", () => {
    const model = buildQuestionNavModel({
      ...baseInput(),
      answerKey: {
        q1: { kind: "text", values: ["x"] },
        q2: { kind: "text", values: ["y"] }
      }
    });
    expect(firstUnansweredSlot(model)).toBe("q3");
    expect(firstUnansweredSlot(buildQuestionNavModel({ ...baseInput(), answerKey: { q1: { kind: "text", values: ["x"] }, q2: { kind: "text", values: ["y"] }, q3: { kind: "text", values: ["z"] } } }))).toBeUndefined();
  });

  it("prefers the active listening part before other parts", () => {
    const shared = {
      ...baseInput(),
      mode: "student" as const,
      listeningParts: [listeningPart(1, ["g1"]), listeningPart(2, ["g2"])],
      selectedPart: 1
    };
    // 当前 Part 1 还有未作答：即使 Part 2 也没答完，也先定位当前 Part。
    expect(firstUnansweredSlot(buildQuestionNavModel({ ...shared, studentAnswers: { q1: ["a"] } }))).toBe("q2");
    // 当前 Part 1 已全部作答：落空后按顺序找到 Part 2 的未作答题。
    expect(firstUnansweredSlot(buildQuestionNavModel({ ...shared, studentAnswers: { q1: ["a"], q2: ["b"] } }))).toBe("q3");
  });
});
