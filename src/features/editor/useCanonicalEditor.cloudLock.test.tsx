// @vitest-environment jsdom
//
// D1：云端校核期间后端返回 CLOUD_REVIEW_IN_PROGRESS。前端不得当普通"保存失败"变红，
// 也不得丢用户修改——保留待保存队列、进入 locked 态，解锁后由工作区补发。证据层级：hook 单元。

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
  try { localStorage.clear(); } catch { /* jsdom */ }
});
afterEach(() => cleanup());

describe("useCanonicalEditor 云端校核锁（D1）", () => {
  it("CLOUD_REVIEW_IN_PROGRESS：进入 locked 态，不变红、不丢修改", async () => {
    getWorkspaceItem.mockResolvedValue({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
    applyEditorCommands.mockRejectedValue(new Error("CLOUD_REVIEW_IN_PROGRESS:it-1"));

    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    act(() => result.current.applyPatch(setAnswer("v1")));
    await waitFor(() => expect(result.current.saveState).toBe("locked"), { timeout: 4000 });
    expect(result.current.pendingCount).toBeGreaterThan(0);
    expect(result.current.saveMessage).toBeFalsy();
  });

  it("云端校核结束后：flush 补发被暂存的修改，保存成功", async () => {
    getWorkspaceItem.mockResolvedValue({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
    applyEditorCommands.mockRejectedValueOnce(new Error("CLOUD_REVIEW_IN_PROGRESS:it-1"));
    applyEditorCommands.mockResolvedValue({ editVersion: 2, appliedCount: 1, replayed: false });

    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.applyPatch(setAnswer("v1")));
    await waitFor(() => expect(result.current.saveState).toBe("locked"), { timeout: 4000 });

    await act(async () => { await result.current.flush(); });
    await waitFor(() => expect(result.current.saveState).toBe("saved"), { timeout: 4000 });
    expect(result.current.version).toBe(2);
  });
});
