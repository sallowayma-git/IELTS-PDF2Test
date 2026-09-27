// @vitest-environment jsdom
//
// C1 相关修复：识别面板「接受/驳回/撤销」提交前必须先 flush 编辑器待保存修改，
// 否则用户会撞上「和自己冲突」的红色提示。先红后绿：旧实现（直接 applyRecognitionDecisions）
// 两条断言都会红。
//
// 证据层级：组件级（渲染真实 RecognitionPanel，mock recognitionClient 的 IPC + beforeApply）。

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

const applyRecognitionDecisions = vi.fn();
const getRecognitionDecision = vi.fn();
const undoCloudRepair = vi.fn();

vi.mock("../../api/recognitionClient", async (importActual) => {
  const actual = await importActual<typeof import("../../api/recognitionClient")>();
  return {
    ...actual,
    applyRecognitionDecisions: (...a: unknown[]) => applyRecognitionDecisions(...a),
    getRecognitionDecision: (...a: unknown[]) => getRecognitionDecision(...a),
    undoCloudRepair: (...a: unknown[]) => undoCloudRepair(...a),
  };
});

import { RecognitionPanel } from "./RecognitionPanel";

function viewWithOneReviewItem() {
  return {
    schemaVersion: "RecognitionDecisionViewV1",
    itemId: "it-1",
    batchId: "b-1",
    baseEditVersion: 5,
    currentEditVersion: 5,
    stale: false,
    localStatus: "succeeded",
    cloudStatus: "succeeded",
    sourceStatus: "succeeded",
    adjudicationStatus: "succeeded",
    cloudReasonCode: null,
    sourceReasonCode: null,
    adjudicationReasonCode: null,
    summary: { agreed: 0, autoFixed: 0, needsReview: 1, unverifiable: 0 },
    items: [
      {
        decisionId: "d1", resolution: "needs_review", code: "ANSWER_MISMATCH", severity: "warning",
        title: "待确认：q1", userMessage: "建议改为 B", target: { kind: "slot", slotId: "q1" },
        field: "answer", evidence: [], localValue: "A", cloudValue: "B", proposedPatch: { op: "setAnswer" },
        status: "open", dependencyGroup: null,
      },
    ],
    repair: null,
  };
}

afterEach(() => { cleanup(); applyRecognitionDecisions.mockReset(); getRecognitionDecision.mockReset(); undoCloudRepair.mockReset(); });

describe("RecognitionPanel 提交前先 flush 编辑器（C1）", () => {
  it("接受建议前先 await beforeApply(flush)，再 applyRecognitionDecisions", async () => {
    getRecognitionDecision.mockResolvedValue(viewWithOneReviewItem());
    applyRecognitionDecisions.mockResolvedValue({ accepted: ["d1"], rejected: [], undone: [], stale: [], failed: [], replayed: false });
    const calls: string[] = [];
    const beforeApply = vi.fn(async () => { calls.push("flush"); });
    applyRecognitionDecisions.mockImplementation(async () => { calls.push("apply"); return { accepted: ["d1"], rejected: [], undone: [], stale: [], failed: [], replayed: false }; });

    render(
      <RecognitionPanel itemId="it-1" editVersion={5} refreshKey="k" onLocate={() => {}} onOpenSource={() => {}} onApplied={() => {}} answerKey={undefined} beforeApply={beforeApply} />
    );
    const acceptBtn = await screen.findByTestId("workspace-recognition-accept-d1");
    await act(async () => { fireEvent.click(acceptBtn); });
    await waitFor(() => expect(applyRecognitionDecisions).toHaveBeenCalledTimes(1));
    expect(beforeApply).toHaveBeenCalled();
    expect(calls).toEqual(["flush", "apply"]); // flush 必须先于 apply
  });

  it("flush 失败时不提交决策，并给出提示", async () => {
    getRecognitionDecision.mockResolvedValue(viewWithOneReviewItem());
    const beforeApply = vi.fn(async () => { throw new Error("EDIT_VERSION_CONFLICT:current=6:base=5"); });

    render(
      <RecognitionPanel itemId="it-1" editVersion={5} refreshKey="k" onLocate={() => {}} onOpenSource={() => {}} onApplied={() => {}} answerKey={undefined} beforeApply={beforeApply} />
    );
    const acceptBtn = await screen.findByTestId("workspace-recognition-accept-d1");
    await act(async () => { fireEvent.click(acceptBtn); });
    await waitFor(() => expect(beforeApply).toHaveBeenCalled());
    expect(applyRecognitionDecisions).not.toHaveBeenCalled();
    // 面板出现提示（不静默）。
    await waitFor(() => expect(document.querySelector('[data-testid="workspace-recognition"]')?.textContent ?? "").toContain("未能保存"));
  });
});
