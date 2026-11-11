// @vitest-environment jsdom
//
// 编辑页选项增删的可见性（作者模式）：
// - 选项池型题目（拖放）的每个选项都有可见可点的 × 按钮，派发 option.delete；
// - 只剩一个选项时 × 仍渲染但禁用，并说明原因；
// - 「＋ 添加选项」在选项池内部末尾，而不是题目列表下方；
// - 删除被后端规则拒绝（选项已用作答案）时，编译阶段抛出的人话提示。
//
// 证据层级：组件级（jsdom）。CSS 可见性（常显、hover 加深、chip 内定位）无法在 jsdom 断言，
// 由真实应用的 CDP 脚本 scripts/e2e/tauri-cdp-confirm-into-library.mjs 截图证明。

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { ExamCanvas } from "./ExamCanvas";
import { compileStructureAction } from "./structureActions";
import type { AnswerSlotV2, IeltsAuthoringIRV2, OptionV2, TaskGroupV2 } from "../types";

vi.mock("../api/tauriCommands", () => ({ resolveAuthoringAssetPreview: vi.fn(async () => undefined) }));

function option(optionId: string, label: string, text = label): OptionV2 {
  return { optionId, label, content: [{ id: `${optionId}-text`, type: "text", text, sourceAnchors: [], provenanceStatus: "source" }], sourceAnchors: [] };
}

const quality = {
  schemaVersion: "QualityReportV2" as const, state: "review_required" as const,
  documentScore: 0, sourceCoverage: 0, coverageLedger: [],
  coverageStatus: { physicalShadow: "missing" as const, complete: false, significantSourceNodeCount: 0, explainedSourceNodeCount: 0, unassignedSourceNodeIds: [] },
  compilerProbes: {
    v2Runtime: { status: "passed" as const, schemaVersion: "", issueCodes: [], details: [] },
    v1Compatibility: { status: "passed" as const, schemaVersion: "", issueCodes: [], details: [] }
  },
  taskScores: {}, hardFailures: [], issues: [], metrics: {}, evaluatedAt: "", evaluatorVersion: ""
};

/** 选项池型题目：两个拖放答案位（宿主是题目本身，不在正文段落里），共享一个选项池。 */
function poolDraft(options: OptionV2[], answerKey: IeltsAuthoringIRV2["answerKey"] = { q14: { kind: "unresolved" }, q15: { kind: "unresolved" } }): IeltsAuthoringIRV2 {
  const slots: Record<string, AnswerSlotV2> = {
    q14: { slotId: "q14", questionNumber: 14, displayLabel: "14", hostType: "prompt", interaction: "dragdrop", participation: "scoring", sourceAnchors: [], confidence: 1 },
    q15: { slotId: "q15", questionNumber: 15, displayLabel: "15", hostType: "prompt", interaction: "dragdrop", participation: "scoring", sourceAnchors: [], confidence: 1 }
  };
  const task: TaskGroupV2 = {
    taskId: "headings", taskType: "matching_features", displayRange: { kind: "range", start: 14, end: 15 }, instructions: [],
    instructionSignature: { normalizedText: "", taskType: "matching_features", expectedQuestionNumbers: [14, 15], expectedSlotCount: 2, evidenceAnchors: [], confidence: 1 },
    responseGroups: [{ responseGroupId: "headings-rg", kind: "matching", slotIds: ["q14", "q15"], optionBankRef: "headings-bank", cardinality: { min: 1, max: 1, exact: 1 }, assignment: "per_slot", scoringPolicy: "per_slot_binary", duplicatePolicy: "reject_submission", allowOptionReuse: false, sourceAnchors: [] }],
    optionBank: { optionBankId: "headings-bank", scope: "task_group", options, allowReuse: false, sourceAnchors: [] },
    sourceAnchors: [], quality: { score: 1, sourceCoverage: 1, hardFailures: [] }, reviewState: "unreviewed"
  };
  return {
    schemaVersion: "IeltsAuthoringIRV2", jobId: "job-pool", exam: { examId: "exam", title: "Pool", language: "en", tags: [], sourceFiles: [] }, modality: "reading",
    passage: { content: [], paragraphMap: {}, sourceAnchors: [] },
    taskGroups: [task], answerSlots: slots, answerKey, assets: [], sourceDocumentId: "source", quality,
    audit: { revision: 1, source: "auto_extract", humanVerified: false, llmUsed: false, updatedAt: "", notes: [] }
  };
}

const twoOptions = () => [option("heading-i", "i", "First heading"), option("heading-iv", "iv", "Fourth heading")];

afterEach(cleanup);

describe("选项池型题目：选项的删除入口", () => {
  it("每个选项都有可见可点的 ×，点击派发 option.delete", () => {
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={poolDraft(twoOptions())} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    const pool = container.querySelector<HTMLElement>('[data-option-pool-task="headings"]')!;
    const deletes = pool.querySelectorAll<HTMLButtonElement>(".v2-option-delete");
    expect(deletes).toHaveLength(2);
    expect(deletes[0].disabled).toBe(false);
    expect(deletes[0].getAttribute("aria-label")).toBe("删除选项 i");
    fireEvent.click(deletes[0]);
    expect(onStructureAction).toHaveBeenCalledWith({ type: "option.delete", taskId: "headings", responseGroupId: "headings-rg", optionId: "heading-i" });
  });

  it("只剩一个选项时 × 仍渲染但禁用，并说明至少保留一个", () => {
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={poolDraft([option("heading-i", "i", "First heading")])} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    const remove = container.querySelector<HTMLButtonElement>('[data-option-pool-task="headings"] .v2-option-delete')!;
    expect(remove).toBeTruthy();
    expect(remove.disabled).toBe(true);
    expect(remove.title).toBe("至少保留一个选项");
    fireEvent.click(remove);
    expect(onStructureAction).not.toHaveBeenCalled();
  });

  it("「＋ 添加选项」在选项池内部末尾，点击派发 option.add 并指向最后一个选项之后", () => {
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={poolDraft(twoOptions())} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    const pool = container.querySelector<HTMLElement>('[data-option-pool-task="headings"]')!;
    const add = pool.querySelector<HTMLButtonElement>(".v2-option-add");
    expect(add).toBeTruthy();
    expect(add!.textContent).toContain("添加选项");
    // 池内部末尾：是池的直接子元素，且排在选项列表之后。
    expect(add!.parentElement).toBe(pool);
    expect(pool.lastElementChild).toBe(add);
    // 题目列表（v2-slot-list）里不再有「添加选项」。
    expect(container.querySelectorAll(".v2-option-add")).toHaveLength(1);
    fireEvent.click(add!);
    expect(onStructureAction).toHaveBeenCalledWith({ type: "option.add", taskId: "headings", responseGroupId: "headings-rg", afterOptionId: "heading-iv" });
  });

  it("非作者模式不渲染 × 与添加按钮", () => {
    const { container } = render(<ExamCanvas authoring={poolDraft(twoOptions())} mode="student" onAnswerChange={vi.fn()} />);
    expect(container.querySelector(".v2-option-delete")).toBeNull();
    expect(container.querySelector(".v2-option-add")).toBeNull();
  });
});

describe("后端规则拒绝的删除：给出人话提示", () => {
  it("选项已用作本题答案时，编译删除动作抛出可直接展示的中文提示", () => {
    const draft = poolDraft(twoOptions(), { q14: { kind: "option", labels: ["i"], assignment: "per_slot" }, q15: { kind: "unresolved" } });
    expect(() => compileStructureAction(draft, { type: "option.delete", taskId: "headings", responseGroupId: "headings-rg", optionId: "heading-i" }))
      .toThrow("这个选项已用作本题答案，请先修改答案。");
  });

  it("删除最后一个选项时抛出「至少保留一个选项」", () => {
    const draft = poolDraft([option("heading-i", "i", "First heading")]);
    expect(() => compileStructureAction(draft, { type: "option.delete", taskId: "headings", responseGroupId: "headings-rg", optionId: "heading-i" }))
      .toThrow("至少保留一个选项。");
  });
});
