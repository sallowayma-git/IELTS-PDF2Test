// @vitest-environment jsdom
//
// 撤销/重做栈封顶 10 步，超出丢最早。证据层级：hook 单元（mock workspaceClient）。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

const applyEditorCommands = vi.fn();
const getWorkspaceItem = vi.fn();

vi.mock("../../api/workspaceClient", () => ({
  applyEditorCommands: (...args: unknown[]) => applyEditorCommands(...args),
  getWorkspaceItem: (...args: unknown[]) => getWorkspaceItem(...args),
}));

import { useCanonicalEditor } from "./useCanonicalEditor";

const sampleDs = () => ({
  schemaVersion: "IeltsAuthoringIRV2",
  exam: { title: "T" },
  taskGroups: [],
  answerSlots: { s1: { slotId: "s1", questionNumber: 1, interaction: "text" } },
  answerKey: { s1: { kind: "text", values: ["v0"] } },
});
const setAnswer = (value: string) => ({ op: "setAnswer", slotId: "s1", value: { kind: "text", values: [value], normalization: "ielts_default" } }) as never;

beforeEach(() => {
  applyEditorCommands.mockReset();
  getWorkspaceItem.mockReset();
  applyEditorCommands.mockImplementation((batch: { baseVersion: number }) => Promise.resolve({ editVersion: batch.baseVersion + 1, appliedCount: 1, replayed: false }));
  try { localStorage.clear(); } catch { /* jsdom */ }
});
afterEach(() => cleanup());

describe("useCanonicalEditor 撤销栈上限", () => {
  it("连续 12 次编辑后最多只能撤销 10 步，最早的两步被丢弃", async () => {
    getWorkspaceItem.mockResolvedValue({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    for (let i = 1; i <= 12; i += 1) {
      act(() => result.current.applyPatch(setAnswer(`v${i}`)));
    }
    expect(result.current.canUndo).toBe(true);

    let undos = 0;
    while (result.current.canUndo && undos < 50) {
      act(() => result.current.undo());
      undos += 1;
    }
    expect(undos).toBe(10);
    // 丢掉了 v0→v1、v1→v2 两条逆补丁，撤到底只能回到第 2 次编辑后的状态。
    expect((result.current.draft as unknown as { answerKey: { s1: { values: string[] } } }).answerKey.s1.values[0]).toBe("v2");
  });

  it("重做栈同样封顶 10 步", async () => {
    getWorkspaceItem.mockResolvedValue({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    for (let i = 1; i <= 12; i += 1) {
      act(() => result.current.applyPatch(setAnswer(`v${i}`)));
    }
    while (result.current.canUndo) act(() => result.current.undo());
    let redos = 0;
    while (result.current.canRedo && redos < 50) {
      act(() => result.current.redo());
      redos += 1;
    }
    expect(redos).toBe(10);
  });
});
