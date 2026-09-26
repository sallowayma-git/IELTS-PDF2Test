// @vitest-environment jsdom
//
// 编辑模式下选项排序（2026-09-26）：工具条里不再给每个选项放 ↑/↓ 按钮，
// 选项顺序改由选项行左侧的拖动手柄调整（聚焦手柄后也可按上下方向键）。
//
// 证据层级：组件级。渲染真实 ExamCanvas，驱动手柄的 pointer 事件，
// 再用真实 compileStructureAction 把动作编译成补丁；不经过 Tauri/WebView2，
// 所以它不能替代桌面端的拖动验证。

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ExamCanvas, type ExamCanvasStructureAction } from "./ExamCanvas";
import { compileStructureAction } from "./structureActions";
import type { AnswerSlotV2, IeltsAuthoringIRV2, OptionV2, TaskGroupV2 } from "../types";

vi.mock("../api/tauriCommands", () => ({ resolveAuthoringAssetPreview: vi.fn(async () => undefined) }));

function option(optionId: string, label: string, text: string): OptionV2 {
  return { optionId, label, content: [{ id: `t-${optionId}`, type: "text", text, sourceAnchors: [], provenanceStatus: "source" }], sourceAnchors: [] };
}

const slot: AnswerSlotV2 = {
  slotId: "q1", questionNumber: 1, displayLabel: "1", hostType: "prompt", interaction: "radio",
  participation: "scoring", sourceAnchors: [], confidence: 1
};

function makeDraft(): IeltsAuthoringIRV2 {
  const task: TaskGroupV2 = {
    taskId: "task-1",
    taskType: "multiple_choice",
    displayRange: { kind: "range", start: 1, end: 1 },
    instructions: [],
    instructionSignature: { normalizedText: "", taskType: "multiple_choice", expectedQuestionNumbers: [], expectedSlotCount: 0, evidenceAnchors: [], confidence: 1 },
    responseGroups: [{
      responseGroupId: "rg-1", kind: "choice", slotIds: ["q1"], optionBankRef: "bank-1",
      cardinality: { min: 1, max: 1 }, assignment: "per_slot", scoringPolicy: "per_slot_binary",
      duplicatePolicy: "reject_submission", allowOptionReuse: false, sourceAnchors: []
    }],
    optionBank: { optionBankId: "bank-1", scope: "task_group", sourceAnchors: [], allowReuse: false, options: [option("o-a", "A", "alpha"), option("o-b", "B", "beta"), option("o-c", "C", "gamma")] },
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
    answerSlots: { q1: slot },
    answerKey: { q1: { kind: "option", labels: ["B"], assignment: "per_slot" } },
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

/** jsdom 没有布局：给每个选项行一个 40px 高的假盒子，拖动落点才可计算。 */
function layOutRows(container: HTMLElement) {
  container.querySelectorAll<HTMLElement>("[data-option-row]").forEach((row, index) => {
    row.getBoundingClientRect = () => ({ top: index * 40, height: 40, bottom: index * 40 + 40, left: 0, right: 100, width: 100, x: 0, y: index * 40, toJSON: () => ({}) });
  });
}

function orderAfter(action: ExamCanvasStructureAction) {
  const patch = compileStructureAction(makeDraft(), action);
  if (patch?.op !== "setOptionBank") throw new Error(`unexpected patch ${JSON.stringify(patch)}`);
  return patch.optionBank!.options.map((entry) => `${entry.label}:${entry.optionId}`);
}

afterEach(cleanup);

describe("编辑模式选项编排", () => {
  it("没有选项悬浮工具条（无 ↑/↓、无选项库工具条），增删贴在选项上", () => {
    const { container } = render(<ExamCanvas authoring={makeDraft()} mode="author" onStructureAction={vi.fn()} />);
    expect(screen.queryByLabelText(/上移选项/)).toBeNull();
    expect(screen.queryByLabelText(/下移选项/)).toBeNull();
    expect(screen.queryByRole("toolbar", { name: "编辑选项库" })).toBeNull();
    expect(container.querySelector(".v2-response-group > .v2-author-tools")).toBeNull();
    // 删除 × 在选项行里，添加入口在选项列表下方。
    expect(screen.getByLabelText("删除选项 A").closest("[data-option-row]")?.getAttribute("data-option-id")).toBe("o-a");
    expect(screen.getByLabelText("添加选项").previousElementSibling?.classList.contains("v2-slot-list")).toBe(true);
    expect(screen.getAllByLabelText(/拖动调整选项/)).toHaveLength(3);
  });

  it("行末 × 删除该选项且不会把它设成答案；添加入口加到末尾", () => {
    const onStructureAction = vi.fn();
    const onAnswerChange = vi.fn();
    render(<ExamCanvas authoring={makeDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={onAnswerChange} />);
    fireEvent.click(screen.getByLabelText("删除选项 C"));
    fireEvent.click(screen.getByLabelText("添加选项"));
    expect(onAnswerChange).not.toHaveBeenCalled();
    expect(onStructureAction.mock.calls.map(([action]) => action)).toEqual([
      { type: "option.delete", taskId: "task-1", responseGroupId: "rg-1", optionId: "o-c" },
      { type: "option.add", taskId: "task-1", responseGroupId: "rg-1", afterOptionId: "o-c" }
    ]);
  });

  it("把 C 拖到 A 之前：C 连同字母一起移到最前，答案 B 仍指向 beta", () => {
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={makeDraft()} mode="author" onStructureAction={onStructureAction} />);
    layOutRows(container);
    const handle = screen.getByLabelText(/拖动调整选项 C/);
    fireEvent.pointerDown(handle, { button: 0, pointerId: 1 });
    fireEvent.pointerMove(handle, { clientY: 5, pointerId: 1 });
    expect(container.querySelector('[data-option-id="o-a"]')?.classList.contains("is-drop-before")).toBe(true);
    fireEvent.pointerUp(handle, { clientY: 5, pointerId: 1 });

    expect(onStructureAction).toHaveBeenCalledTimes(1);
    const action = onStructureAction.mock.calls[0][0] as ExamCanvasStructureAction;
    expect(action).toEqual({ type: "option.move", taskId: "task-1", responseGroupId: "rg-1", optionId: "o-c", beforeOptionId: "o-a" });
    expect(orderAfter(action)).toEqual(["C:o-c", "A:o-a", "B:o-b"]);
    expect(container.querySelector(".is-drop-before, .is-drop-after, .is-dragging")).toBeNull();
  });

  it("拖到最后一行下半部分 = 移到末尾；原地放下不产生动作", () => {
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={makeDraft()} mode="author" onStructureAction={onStructureAction} />);
    layOutRows(container);
    const handleA = screen.getByLabelText(/拖动调整选项 A/);
    fireEvent.pointerDown(handleA, { button: 0 });
    fireEvent.pointerMove(handleA, { clientY: 115 });
    fireEvent.pointerUp(handleA);
    const action = onStructureAction.mock.calls[0][0] as ExamCanvasStructureAction;
    expect(action).toMatchObject({ optionId: "o-a", beforeOptionId: undefined });
    expect(orderAfter(action)).toEqual(["B:o-b", "C:o-c", "A:o-a"]);

    // B 放回 B/C 之间（C 之前）等于没动。
    const handleB = screen.getByLabelText(/拖动调整选项 B/);
    fireEvent.pointerDown(handleB, { button: 0 });
    fireEvent.pointerMove(handleB, { clientY: 75 });
    fireEvent.pointerUp(handleB);
    // 只按下不移动也不算拖动。
    fireEvent.pointerDown(handleB, { button: 0 });
    fireEvent.pointerUp(handleB);
    expect(onStructureAction).toHaveBeenCalledTimes(1);
  });

  it("横排选项（TFNG 版式）按水平位置判断落点", () => {
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={makeDraft()} mode="author" onStructureAction={onStructureAction} />);
    // 三个选项排在同一行：每个 80px 宽、间隔 100px。
    container.querySelectorAll<HTMLElement>("[data-option-row]").forEach((row, index) => {
      row.getBoundingClientRect = () => ({ top: 0, height: 30, bottom: 30, left: index * 100, right: index * 100 + 80, width: 80, x: index * 100, y: 0, toJSON: () => ({}) });
    });
    const handleA = screen.getByLabelText(/拖动调整选项 A/);
    fireEvent.pointerDown(handleA, { button: 0 });
    // 落在 C 的右半边 ⇒ 放到末尾。
    fireEvent.pointerMove(handleA, { clientX: 270, clientY: 15 });
    expect(container.querySelector('[data-option-id="o-c"]')?.classList.contains("is-drop-after")).toBe(true);
    fireEvent.pointerUp(handleA);
    // 落在 B 的左半边 ⇒ C 放到 B 之前。
    const handleC = screen.getByLabelText(/拖动调整选项 C/);
    fireEvent.pointerDown(handleC, { button: 0 });
    fireEvent.pointerMove(handleC, { clientX: 110, clientY: 15 });
    fireEvent.pointerUp(handleC);
    expect(onStructureAction.mock.calls.map(([action]) => orderAfter(action))).toEqual([
      ["B:o-b", "C:o-c", "A:o-a"],
      ["A:o-a", "C:o-c", "B:o-b"]
    ]);
  });

  it("指针离开手柄后（移动/松开发生在别处）拖动照样生效；中途卸载则取消", () => {
    const onStructureAction = vi.fn();
    const { container, unmount } = render(<ExamCanvas authoring={makeDraft()} mode="author" onStructureAction={onStructureAction} />);
    layOutRows(container);
    fireEvent.pointerDown(screen.getByLabelText(/拖动调整选项 C/), { button: 0 });
    // 不依赖 pointer capture：事件落在页面其他元素上。
    fireEvent.pointerMove(document.body, { clientY: 45 });
    fireEvent.pointerUp(document.body);
    expect(onStructureAction.mock.calls.map(([action]) => orderAfter(action))).toEqual([["A:o-a", "C:o-c", "B:o-b"]]);

    layOutRows(container);
    fireEvent.pointerDown(screen.getByLabelText(/拖动调整选项 A/), { button: 0 });
    fireEvent.pointerMove(document.body, { clientY: 115 });
    unmount();
    fireEvent.pointerUp(document.body);
    expect(onStructureAction).toHaveBeenCalledTimes(1);
  });

  it("Esc 取消拖动", () => {
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={makeDraft()} mode="author" onStructureAction={onStructureAction} />);
    layOutRows(container);
    fireEvent.pointerDown(screen.getByLabelText(/拖动调整选项 C/), { button: 0 });
    fireEvent.pointerMove(document.body, { clientY: 5 });
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.pointerUp(document.body);
    expect(onStructureAction).not.toHaveBeenCalled();
    expect(container.querySelector(".is-drop-before, .is-drop-after, .is-dragging, .is-reordering")).toBeNull();
  });

  it("点击手柄不会把该选项设成答案；键盘 ↑/↓ 可移动", () => {
    const onStructureAction = vi.fn();
    const onAnswerChange = vi.fn();
    render(<ExamCanvas authoring={makeDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={onAnswerChange} />);
    const handleB = screen.getByLabelText(/拖动调整选项 B/);
    fireEvent.click(handleB);
    expect(onAnswerChange).not.toHaveBeenCalled();

    fireEvent.keyDown(handleB, { key: "ArrowUp" });
    fireEvent.keyDown(handleB, { key: "ArrowDown" });
    fireEvent.keyDown(screen.getByLabelText(/拖动调整选项 C/), { key: "ArrowDown" });
    expect(onStructureAction.mock.calls.map(([action]) => orderAfter(action))).toEqual([
      ["B:o-b", "A:o-a", "C:o-c"],
      ["A:o-a", "C:o-c", "B:o-b"]
    ]);
  });

  it("学生预览没有手柄、没有增删入口，也不挂拖动属性", () => {
    const { container } = render(<ExamCanvas authoring={makeDraft()} mode="student" />);
    expect(screen.queryByLabelText(/拖动调整选项/)).toBeNull();
    expect(screen.queryByLabelText(/删除选项|添加选项/)).toBeNull();
    expect(container.querySelector("[data-option-row], [data-option-list]")).toBeNull();
  });
});
