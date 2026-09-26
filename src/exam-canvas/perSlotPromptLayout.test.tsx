// @vitest-environment jsdom
//
// 判断题（TFNG/YNNG）画布版式两个缺陷的组件级回归：
// 1. 逐题题干（per-slot prompt）：response.prompt 的顶层段落被每个 slot 的 hostNodeId
//    认领时，"1  陈述文字" 紧跟该题的 TRUE/FALSE/NOT GIVEN 选项，顶部不再堆整块 prompt；
//    条件不全（缺宿主 / 宿主内嵌 answer_slot / unordered_set）一律保持旧版式。
// 2. 选项 content 与 label 重复（旧草稿 label YES content 也写 YES → "YES YES"）时不再重复显示。
//
// 证据层级：组件级（jsdom 渲染真实 ExamCanvas）。不经 Tauri/WebView2，
// 不能替代桌面端产品工作流验证。

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { ExamCanvas } from "./ExamCanvas";
import type { AnswerSlotV2, ContentNodeV2, IeltsAuthoringIRV2, OptionV2, ResponseGroupV2, TaskGroupV2 } from "../types";

vi.mock("../api/tauriCommands", () => ({ resolveAuthoringAssetPreview: vi.fn(async () => undefined) }));

const STATEMENTS = [
  "Per-slot statement one for question one.",
  "Per-slot statement two for question two.",
  "Per-slot statement three for question three."
];

const text = (id: string, value: string): ContentNodeV2 =>
  ({ id, type: "text", text: value, sourceAnchors: [], provenanceStatus: "source" });

const paragraph = (id: string, children: ContentNodeV2[]): ContentNodeV2 =>
  ({ id, type: "paragraph", children, sourceAnchors: [], provenanceStatus: "source" });

const promptParagraph = (id: string, statement: string): ContentNodeV2 =>
  paragraph(id, [text(`${id}-text`, statement)]);

const defaultPrompt = (): ContentNodeV2[] =>
  STATEMENTS.map((statement, index) => promptParagraph(`group-1-prompt-${index + 1}`, statement));

function tfngSlot(slotId: string, questionNumber: number, hostNodeId?: string): AnswerSlotV2 {
  return {
    slotId, questionNumber, displayLabel: String(questionNumber), hostNodeId, hostType: "prompt",
    interaction: "radio", participation: "scoring", sourceAnchors: [], confidence: 1
  };
}

function tfngOption(optionId: string, label: string, content: string): OptionV2 {
  return { optionId, label, content: content ? [text(`${optionId}-content`, content)] : [], sourceAnchors: [] };
}

function tfngDraft(overrides?: {
  slots?: AnswerSlotV2[];
  prompt?: ContentNodeV2[];
  options?: OptionV2[];
  assignment?: ResponseGroupV2["assignment"];
  taskType?: TaskGroupV2["taskType"];
}): IeltsAuthoringIRV2 {
  const slots = overrides?.slots ?? [
    tfngSlot("q1", 1, "group-1-prompt-1"),
    tfngSlot("q2", 2, "group-1-prompt-2"),
    tfngSlot("q3", 3, "group-1-prompt-3")
  ];
  const prompt = overrides?.prompt ?? defaultPrompt();
  const options = overrides?.options ?? [
    tfngOption("o-true", "TRUE", "TRUE"),
    tfngOption("o-false", "FALSE", "contradiction of the passage"),
    tfngOption("o-ng", "NOT GIVEN", "")
  ];
  const task: TaskGroupV2 = {
    taskId: "task-1",
    taskType: overrides?.taskType ?? "true_false_not_given",
    displayRange: { kind: "range", start: 1, end: 3 },
    instructions: [],
    instructionSignature: { normalizedText: "", taskType: overrides?.taskType ?? "true_false_not_given", expectedQuestionNumbers: [1, 2, 3], expectedSlotCount: 3, evidenceAnchors: [], confidence: 1 },
    responseGroups: [{
      responseGroupId: "group-1", kind: "choice", prompt, slotIds: ["q1", "q2", "q3"], optionBankRef: "bank-1",
      cardinality: { min: 1, max: 1 }, assignment: overrides?.assignment ?? "per_slot", scoringPolicy: "per_slot_binary",
      duplicatePolicy: "reject_submission", allowOptionReuse: false, sourceAnchors: []
    }],
    optionBank: { optionBankId: "bank-1", scope: "task_group", sourceAnchors: [], allowReuse: false, options },
    sourceAnchors: [],
    quality: { score: 1, sourceCoverage: 1, hardFailures: [] },
    reviewState: "unreviewed"
  };
  return {
    schemaVersion: "IeltsAuthoringIRV2",
    jobId: "job-1",
    exam: { examId: "exam-1", title: "T", language: "en", tags: [], sourceFiles: [] },
    modality: "reading",
    taskGroups: [task],
    answerSlots: Object.fromEntries(slots.map((slot) => [slot.slotId, slot])),
    answerKey: {
      q1: { kind: "option", labels: ["TRUE"], assignment: "per_slot" },
      q2: { kind: "option", labels: ["FALSE"], assignment: "per_slot" },
      q3: { kind: "option", labels: ["NOT GIVEN"], assignment: "per_slot" }
    },
    assets: [],
    sourceDocumentId: "doc-1",
    quality: {
      schemaVersion: "QualityReportV2", state: "review_required", documentScore: 0, sourceCoverage: 0, coverageLedger: [],
      coverageStatus: { physicalShadow: "missing", complete: false, significantSourceNodeCount: 0, explainedSourceNodeCount: 0, unassignedSourceNodeIds: [] },
      compilerProbes: {
        v2Runtime: { status: "passed", schemaVersion: "", issueCodes: [], details: [] },
        v1Compatibility: { status: "passed", schemaVersion: "", issueCodes: [], details: [] }
      },
      taskScores: {}, hardFailures: [], issues: [], metrics: {}, evaluatedAt: "", evaluatorVersion: ""
    },
    audit: { revision: 1, source: "auto_extract", humanVerified: false, llmUsed: false, updatedAt: "", notes: [] }
  };
}

/** 旧版式断言：整块 prompt 在顶部 .v2-response-prompt，题号后没有题干文字。 */
function expectLegacyLayout(container: HTMLElement) {
  const topPrompt = container.querySelector(".v2-response-prompt");
  expect(topPrompt).not.toBeNull();
  for (const statement of STATEMENTS) expect(topPrompt!.textContent).toContain(statement);
  STATEMENTS.forEach((statement, index) => {
    const question = container.querySelector(`[data-question-id="q${index + 1}"]`);
    expect(question).not.toBeNull();
    expect(question!.querySelector(".v2-slot-number")?.textContent).toBe(String(index + 1));
    expect(question!.textContent).not.toContain(statement);
    expect(question!.querySelector(".v2-slot-prompt")).toBeNull();
  });
}

afterEach(cleanup);

describe("判断题逐题题干（per-slot prompt）", () => {
  it.each(["author", "student"] as const)("全部宿主命中：题号后紧跟该题陈述，顶部不再渲染整块 prompt（%s 模式）", (mode) => {
    const { container } = render(
      <ExamCanvas authoring={tfngDraft()} mode={mode} onTextChange={mode === "author" ? vi.fn() : undefined} />
    );
    expect(container.querySelector(".v2-response-prompt")).toBeNull();
    STATEMENTS.forEach((statement, index) => {
      const question = container.querySelector(`[data-question-id="q${index + 1}"]`);
      expect(question).not.toBeNull();
      expect(question!.querySelector(".v2-slot-number")?.textContent).toBe(String(index + 1));
      expect(question!.textContent).toContain(statement);
      // 题干在题号之后的同一 flex 容器里（v2-slot-question-label），选项列表跟在其后。
      const promptInQuestion = question!.querySelector(".v2-slot-question-label .v2-slot-prompt");
      expect(promptInQuestion).not.toBeNull();
      expect(promptInQuestion!.textContent).toContain(statement);
      expect(question!.querySelector(".v2-choice-options")).not.toBeNull();
      expect(question!.querySelector(".v2-choice-options .v2-choice-item")).not.toBeNull();
    });
  });

  it("作者模式下逐题题干仍是可原位编辑的 ContentNodes 文本（点开后能提交）", () => {
    const onTextChange = vi.fn();
    const { container } = render(<ExamCanvas authoring={tfngDraft()} mode="author" onTextChange={onTextChange} />);
    const statement = container.querySelector('[data-question-id="q2"] [data-editor-id="group-1-prompt-2-text"]');
    expect(statement).not.toBeNull();
    expect(statement!.classList.contains("v2-author-editable")).toBe(true);
    fireEvent.click(statement!);
    const editor = container.querySelector<HTMLTextAreaElement>('[data-question-id="q2"] textarea.inline-text-editor');
    expect(editor).not.toBeNull();
    fireEvent.change(editor!, { target: { value: "edited statement two" } });
    fireEvent.blur(editor!);
    expect(onTextChange).toHaveBeenCalledTimes(1);
    expect(onTextChange.mock.calls[0][0]).toMatchObject({ id: "group-1-prompt-2-text", text: "edited statement two" });
  });

  it("部分段落未被认领：顶部只渲染剩余公共段，逐题题干照常生效", () => {
    const prompt = [...defaultPrompt(), promptParagraph("group-1-prompt-4", "Shared group instruction paragraph.")];
    const { container } = render(<ExamCanvas authoring={tfngDraft({ prompt })} mode="student" />);
    const topPrompt = container.querySelector(".v2-response-prompt");
    expect(topPrompt).not.toBeNull();
    expect(topPrompt!.textContent).toContain("Shared group instruction paragraph.");
    for (const statement of STATEMENTS) expect(topPrompt!.textContent).not.toContain(statement);
    expect(container.querySelector('[data-question-id="q1"]')!.textContent).toContain(STATEMENTS[0]);
  });

  it("反例1：某个 slot 的 hostNodeId 在 prompt 顶层找不到 → 保持旧版式", () => {
    const slots = [tfngSlot("q1", 1, "group-1-prompt-1"), tfngSlot("q2", 2, "missing-node"), tfngSlot("q3", 3, "group-1-prompt-3")];
    const { container } = render(<ExamCanvas authoring={tfngDraft({ slots })} mode="author" />);
    expectLegacyLayout(container);
  });

  it("反例2：某个宿主段落内部含 answer_slot 节点 → 保持旧版式", () => {
    const prompt = [
      promptParagraph("group-1-prompt-1", STATEMENTS[0]),
      paragraph("group-1-prompt-2", [
        text("group-1-prompt-2-text", STATEMENTS[1]),
        { id: "slot-q2", type: "answer_slot", slotId: "q2", displayLabel: "2", inline: true, sourceAnchors: [], provenanceStatus: "source" }
      ]),
      promptParagraph("group-1-prompt-3", STATEMENTS[2])
    ];
    const { container } = render(<ExamCanvas authoring={tfngDraft({ prompt })} mode="student" />);
    expectLegacyLayout(container);
  });

  it("反例3：assignment 为 unordered_set（共享多选）→ 保持旧版式", () => {
    const { container } = render(<ExamCanvas authoring={tfngDraft({ assignment: "unordered_set" })} mode="student" />);
    // unordered 分支走共享 fieldset（题号在 v2-slot-chip 上，没有 v2-slot-number）：
    // 整块 prompt 仍留在顶部，任何题干都不下沉。
    const topPrompt = container.querySelector(".v2-response-prompt");
    expect(topPrompt).not.toBeNull();
    for (const statement of STATEMENTS) expect(topPrompt!.textContent).toContain(statement);
    expect(container.querySelector(".v2-shared-selection")).not.toBeNull();
    expect(container.querySelector(".v2-slot-prompt")).toBeNull();
  });
});

describe("选项 label/content 重复兜底", () => {
  it("content 与 label 相同（TRUE/TRUE）只渲染一次；content 不同（FALSE + 说明文字）两者都显示", () => {
    const { container } = render(<ExamCanvas authoring={tfngDraft()} mode="student" />);
    const trueRow = container.querySelector('input[value="TRUE"]')!.closest("label")!;
    expect(trueRow.textContent!.match(/TRUE/g)).toHaveLength(1);
    const falseRow = container.querySelector('input[value="FALSE"]')!.closest("label")!;
    expect(falseRow.textContent).toContain("FALSE");
    expect(falseRow.textContent).toContain("contradiction of the passage");
  });

  it("大小写/空白差异也算重复：label YES content \" yes \" 不渲染成 YES YES", () => {
    const options = [
      tfngOption("o-yes", "YES", " yes "),
      tfngOption("o-no", "NO", "no in the passage"),
      tfngOption("o-ng", "NOT GIVEN", "")
    ];
    const { container } = render(<ExamCanvas authoring={tfngDraft({ taskType: "yes_no_not_given", options })} mode="student" />);
    const yesRow = container.querySelector('input[value="YES"]')!.closest("label")!;
    // 大小写不敏感：label 的 YES 与 content 的 " yes " 算重复，合计只出现一次。
    expect(yesRow.textContent!.toLowerCase().match(/yes/g)).toHaveLength(1);
    const noRow = container.querySelector('input[value="NO"]')!.closest("label")!;
    expect(noRow.textContent).toContain("no in the passage");
  });
});
