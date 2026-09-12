import { describe, expect, it } from "vitest";
import type {
  AnswerSlotV2,
  AnswerValueV2,
  IeltsAuthoringIRV2,
  OptionV2,
  ResponseGroupV2,
  TaskGroupV2,
  TaskTypeV2
} from "../../types";
import type { TextNodeV2 } from "../../types/content-doc-v2";
import { blockerCount, deriveActionableIssues } from "./actionableIssues";

// 证据层级：pure unit（计划 §19.1 层 1 / §6.8 硬闭包）。
// 断言「哪些内容缺失会阻塞发布、用户看到什么话、blocker 是否排在 warning 之前」。

function textNode(text: string): TextNodeV2 {
  return { id: `n-${text}`, type: "text", text, sourceAnchors: [], provenanceStatus: "source" };
}

function option(optionId: string, label: string, text: string): OptionV2 {
  return { optionId, label, content: text ? [textNode(text)] : [], sourceAnchors: [] };
}

function group(
  partial: Partial<ResponseGroupV2> & Pick<ResponseGroupV2, "responseGroupId" | "kind" | "slotIds">
): ResponseGroupV2 {
  return {
    cardinality: { min: 1, max: 1 },
    assignment: "per_slot",
    scoringPolicy: "per_slot_binary",
    duplicatePolicy: "reject_submission",
    allowOptionReuse: false,
    sourceAnchors: [],
    ...partial
  };
}

function task(
  partial: Partial<TaskGroupV2> & Pick<TaskGroupV2, "taskId" | "taskType" | "responseGroups">
): TaskGroupV2 {
  return {
    displayRange: { kind: "range", start: 1, end: 1 },
    instructions: [],
    instructionSignature: {
      normalizedText: "",
      taskType: partial.taskType,
      expectedQuestionNumbers: [],
      expectedSlotCount: 0,
      evidenceAnchors: [],
      confidence: 1
    },
    sourceAnchors: [],
    quality: { score: 1, sourceCoverage: 1, hardFailures: [] },
    reviewState: "unreviewed",
    ...partial
  };
}

function slot(
  slotId: string,
  questionNumber: number,
  participation: AnswerSlotV2["participation"] = "scoring"
): AnswerSlotV2 {
  return {
    slotId,
    questionNumber,
    displayLabel: String(questionNumber),
    hostType: "prompt",
    interaction: "radio",
    participation,
    sourceAnchors: [],
    confidence: 1
  };
}

function makeDs(input: {
  taskGroups: TaskGroupV2[];
  answerSlots?: Record<string, AnswerSlotV2>;
  answerKey?: Record<string, AnswerValueV2>;
}): IeltsAuthoringIRV2 {
  return {
    schemaVersion: "IeltsAuthoringIRV2",
    jobId: "job-1",
    exam: { examId: "exam-1", title: "T", language: "en", tags: [], sourceFiles: [] },
    modality: "reading",
    taskGroups: input.taskGroups,
    answerSlots: input.answerSlots ?? {},
    answerKey: input.answerKey ?? {},
    assets: [],
    sourceDocumentId: "doc-1",
    quality: {
      schemaVersion: "QualityReportV2",
      state: "review_required",
      documentScore: 0,
      sourceCoverage: 0,
      coverageLedger: [],
      coverageStatus: {
        physicalShadow: "missing",
        complete: false,
        significantSourceNodeCount: 0,
        explainedSourceNodeCount: 0,
        unassignedSourceNodeIds: []
      },
      compilerProbes: {
        v2Runtime: { status: "passed", schemaVersion: "", issueCodes: [], details: [] },
        v1Compatibility: { status: "passed", schemaVersion: "", issueCodes: [], details: [] }
      },
      taskScores: {},
      hardFailures: [],
      issues: [],
      metrics: {},
      evaluatedAt: "",
      evaluatorVersion: ""
    },
    audit: { revision: 1, source: "auto_extract", humanVerified: false, llmUsed: false, updatedAt: "", notes: [] }
  };
}

describe("deriveActionableIssues — 空输入", () => {
  it("undefined 返回空数组", () => {
    expect(deriveActionableIssues(undefined)).toEqual([]);
  });

  it("没有题组返回空数组", () => {
    expect(deriveActionableIssues(makeDs({ taskGroups: [] }))).toEqual([]);
  });
});

describe("deriveActionableIssues — 选择题闭包", () => {
  const taskType: TaskTypeV2 = "single_choice";

  it("选项组没有选项时产生 OPTION_RUN_INCOMPLETE blocker", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "t1",
          taskType,
          responseGroups: [group({ responseGroupId: "g1", kind: "choice", slotIds: ["q1"], prompt: [textNode("题干")] })]
        })
      ],
      answerSlots: { q1: slot("q1", 1) }
    });
    const issues = deriveActionableIssues(ds);
    expect(issues.map((issue) => issue.code)).toContain("OPTION_RUN_INCOMPLETE");
    expect(issues.find((issue) => issue.code === "OPTION_RUN_INCOMPLETE")?.severity).toBe("blocker");
    expect(issues[0].userMessage).toContain("第 1 题");
  });

  it("某个选项没有正文时产生 OPTION_TEXT_MISSING blocker，并指向该选项", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "t1",
          taskType,
          responseGroups: [
            group({
              responseGroupId: "g1",
              kind: "choice",
              slotIds: ["q1"],
              prompt: [textNode("题干")],
              options: [option("o1", "A", "选项 A"), option("o2", "B", "")]
            })
          ]
        })
      ],
      answerSlots: { q1: slot("q1", 1) }
    });
    const issues = deriveActionableIssues(ds);
    const missing = issues.find((issue) => issue.code === "OPTION_TEXT_MISSING");
    expect(missing?.targetId).toBe("o2");
    expect(missing?.userMessage).toContain("B");
  });

  it("题干为空时产生 QUESTION_PROMPT_MISSING blocker", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "t1",
          taskType,
          responseGroups: [
            group({
              responseGroupId: "g1",
              kind: "choice",
              slotIds: ["q1"],
              prompt: [],
              options: [option("o1", "A", "选项 A")]
            })
          ]
        })
      ],
      answerSlots: { q1: slot("q1", 1) }
    });
    expect(deriveActionableIssues(ds).some((issue) => issue.code === "QUESTION_PROMPT_MISSING")).toBe(true);
  });
});

describe("deriveActionableIssues — 匹配题共享选项库", () => {
  it("匹配题没有共享选项库时产生 SHARED_OPTION_BANK_MISSING blocker", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "t1",
          taskType: "matching_headings",
          responseGroups: [group({ responseGroupId: "g1", kind: "matching", slotIds: ["q1"], prompt: [textNode("题干")] })]
        })
      ],
      answerSlots: { q1: slot("q1", 1) }
    });
    const issue = deriveActionableIssues(ds).find((item) => item.code === "SHARED_OPTION_BANK_MISSING");
    expect(issue?.severity).toBe("blocker");
    expect(issue?.targetId).toBe("t1");
  });

  it("匹配题有共享选项库时不产生该 blocker", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "t1",
          taskType: "matching_features",
          optionBank: {
            optionBankId: "bank-1",
            scope: "task_group",
            options: [option("o1", "A", "feature A")],
            allowReuse: true,
            sourceAnchors: []
          },
          responseGroups: [group({ responseGroupId: "g1", kind: "matching", slotIds: ["q1"], prompt: [textNode("题干")] })]
        })
      ],
      answerSlots: { q1: slot("q1", 1) }
    });
    expect(deriveActionableIssues(ds).some((issue) => issue.code === "SHARED_OPTION_BANK_MISSING")).toBe(false);
  });
});

describe("deriveActionableIssues — 答案位", () => {
  const baseTask = (slotIds: string[] = ["q1", "q2", "q3"]): TaskGroupV2 =>
    task({
      taskId: "t1",
      taskType: "sentence_completion",
      responseGroups: [group({ responseGroupId: "g1", kind: "text_entry", slotIds })]
    });

  it("缺少答案产生 ANSWER_MISSING warning", () => {
    const ds = makeDs({
      taskGroups: [baseTask()],
      answerSlots: { q1: slot("q1", 1), q2: slot("q2", 2), q3: slot("q3", 3) },
      answerKey: { q1: { kind: "text", values: ["answer"] } }
    });
    const issues = deriveActionableIssues(ds).filter((issue) => issue.code === "ANSWER_MISSING");
    expect(issues.map((issue) => issue.targetId).sort()).toEqual(["q2", "q3"]);
    expect(issues.every((issue) => issue.severity === "warning")).toBe(true);
  });

  it("unresolved 答案产生 ANSWER_UNRESOLVED warning", () => {
    const ds = makeDs({
      taskGroups: [baseTask(["q1"])],
      answerSlots: { q1: slot("q1", 1) },
      answerKey: { q1: { kind: "unresolved" } }
    });
    expect(deriveActionableIssues(ds).some((issue) => issue.code === "ANSWER_UNRESOLVED")).toBe(true);
  });

  it("空文本答案同样算缺失", () => {
    const ds = makeDs({
      taskGroups: [baseTask(["q1"])],
      answerSlots: { q1: slot("q1", 1) },
      answerKey: { q1: { kind: "text", values: ["   "] } }
    });
    expect(deriveActionableIssues(ds).some((issue) => issue.code === "ANSWER_MISSING")).toBe(true);
  });

  it("非评分答案位被跳过", () => {
    const ds = makeDs({
      taskGroups: [baseTask(["q1", "q2"])],
      answerSlots: { q1: slot("q1", 1, "example"), q2: slot("q2", 2, "non_scoring") },
      answerKey: {}
    });
    expect(deriveActionableIssues(ds)).toEqual([]);
  });

  it("选项答案有 label 时不报缺失", () => {
    const ds = makeDs({
      taskGroups: [baseTask(["q1"])],
      answerSlots: { q1: slot("q1", 1) },
      answerKey: { q1: { kind: "option", labels: ["A"], assignment: "per_slot" } }
    });
    expect(deriveActionableIssues(ds)).toEqual([]);
  });
});

describe("deriveActionableIssues — 排序与计数", () => {
  it("blocker 排在 warning 之前", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "t1",
          taskType: "single_choice",
          responseGroups: [
            group({ responseGroupId: "g1", kind: "choice", slotIds: ["q1"], prompt: [], options: [option("o1", "A", "A")] })
          ]
        })
      ],
      answerSlots: { q1: slot("q1", 1) },
      answerKey: {}
    });
    const issues = deriveActionableIssues(ds);
    const firstWarning = issues.findIndex((issue) => issue.severity === "warning");
    const lastBlocker = issues.map((issue) => issue.severity).lastIndexOf("blocker");
    expect(lastBlocker).toBeLessThan(firstWarning);
    expect(blockerCount(issues)).toBeGreaterThan(0);
  });

  it("blockerCount 只数 blocker", () => {
    expect(
      blockerCount([
        { issueId: "a", targetId: "a", severity: "blocker", code: "OPTION_TEXT_MISSING", userMessage: "" },
        { issueId: "b", targetId: "b", severity: "warning", code: "ANSWER_MISSING", userMessage: "" }
      ])
    ).toBe(1);
  });
});
