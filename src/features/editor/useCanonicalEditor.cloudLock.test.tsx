// @vitest-environment jsdom
//
// 云端校核期间后端返回 CLOUD_REVIEW_IN_PROGRESS：前端不得当普通"保存失败"变红，
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
  passage: { content: [{ type: "paragraph", id: "p1", children: [{ type: "text", id: "t1", text: "Original passage" }] }] },
  taskGroups: [],
  answerSlots: { s1: { slotId: "s1", questionNumber: 1, interaction: "text" } },
  answerKey: { s1: { kind: "text", values: ["v0"] } },
});
const setAnswer = (value: string) => ({ op: "setAnswer", slotId: "s1", value: { kind: "text", values: [value], normalization: "ielts_default" } }) as never;
const setText = (expectedText: string, text: string) => ({ op: "set_text" as const, nodeId: "t1", expectedText, text });
function passageText(draft: ReturnType<typeof useCanonicalEditor>["draft"]): string | undefined {
  const paragraph = draft?.passage?.content[0];
  const text = paragraph?.type === "paragraph" ? paragraph.children[0] : undefined;
  return text?.type === "text" ? text.text : undefined;
}

beforeEach(() => {
  applyEditorCommands.mockReset();
  getWorkspaceItem.mockReset();
  try { localStorage.clear(); } catch { /* jsdom */ }
});
afterEach(() => cleanup());

describe("useCanonicalEditor 云端校核锁", () => {
  it.each(["Short", "A much longer cloud-reviewed paragraph 🌍"])(
    "云端把原文长度改成 %s 时，整段人工修改必须完整保存",
    async (cloudText) => {
      getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
      const cloud = sampleDs();
      cloud.passage.content[0].children[0].text = cloudText;
      getWorkspaceItem.mockResolvedValue({ item: { title: "T" }, ds: cloud, editVersion: 2,
        recentEdits: [{ baseVersion: 1, origin: "cloud_repair" }] });
      applyEditorCommands.mockRejectedValueOnce(new Error("EDIT_VERSION_CONFLICT:current=2:base=1"));
      applyEditorCommands.mockResolvedValue({ editVersion: 3, appliedCount: 1, replayed: false });
      const { result } = renderHook(() => useCanonicalEditor("it-1"));
      await waitFor(() => expect(result.current.loading).toBe(false));
      act(() => result.current.applyCommand(setText("Original passage", "人工改写 🌍")));
      await waitFor(() => expect(result.current.saveState).toBe("saved"));
      expect(passageText(result.current.draft)).toBe("人工改写 🌍");
      expect(result.current.saveNotice).toBeUndefined();
      expect(applyEditorCommands.mock.calls[1][0].commands[0]).toMatchObject({
        from: 0, to: Array.from(cloudText).length, text: "人工改写 🌍", expectedText: cloudText
      });
    });

  it("云端推进版本后，读取最新稿期间新增的编辑也必须保存", async () => {
    getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
    let resolveLatest!: (value: unknown) => void;
    getWorkspaceItem.mockImplementationOnce(() => new Promise((resolve) => { resolveLatest = resolve; }));
    applyEditorCommands.mockRejectedValueOnce(new Error("EDIT_VERSION_CONFLICT:current=2:base=1"));
    applyEditorCommands.mockResolvedValue({ editVersion: 3, appliedCount: 2, replayed: false });
    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.applyCommand(setText("Original passage", "first")));
    await waitFor(() => expect(getWorkspaceItem).toHaveBeenCalledTimes(2));
    act(() => result.current.applyCommand(setText("first", "second")));
    await act(async () => resolveLatest({ item: { title: "T" }, ds: sampleDs(), editVersion: 2,
      recentEdits: [{ baseVersion: 1, origin: "cloud_repair" }] }));
    await waitFor(() => expect(result.current.saveState).toBe("saved"));
    expect(applyEditorCommands.mock.calls[1][0].commands.map((patch: { text: string }) => patch.text)).toEqual(["first", "second"]);
    expect(passageText(result.current.draft)).toBe("second");
    expect(result.current.pendingCount).toBe(0);
  });

  it("云端自动刷新迟到时，不得把已保存的版本和内容退回旧稿", async () => {
    getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
    let resolveReload!: (value: unknown) => void;
    getWorkspaceItem.mockImplementationOnce(() => new Promise((resolve) => { resolveReload = resolve; }));
    applyEditorCommands.mockResolvedValue({ editVersion: 3, appliedCount: 1, replayed: false });
    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.noteRemoteVersion(2));
    await waitFor(() => expect(getWorkspaceItem).toHaveBeenCalledTimes(2));
    act(() => result.current.applyCommand(setText("Original passage", "saved")));
    await act(async () => { await result.current.flush(); });
    await act(async () => resolveReload({ item: { title: "T" }, ds: sampleDs(), editVersion: 2, recentEdits: [] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.version).toBe(3);
    expect(passageText(result.current.draft)).toBe("saved");
  });

  it("云端刷新读取期间尚未提交的原文编辑保持完整，随后保存", async () => {
    getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
    let resolveReload!: (value: unknown) => void;
    getWorkspaceItem.mockImplementationOnce(() => new Promise((resolve) => { resolveReload = resolve; }));
    applyEditorCommands.mockResolvedValue({ editVersion: 2, appliedCount: 1, replayed: false });
    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.noteRemoteVersion(2));
    await waitFor(() => expect(getWorkspaceItem).toHaveBeenCalledTimes(2));
    act(() => result.current.applyCommand(setText("Original passage", "pending passage")));
    await act(async () => resolveReload({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] }));
    expect(passageText(result.current.draft)).toBe("pending passage");
    expect(result.current.pendingCount).toBe(1);
    await act(async () => { await result.current.flush(); });
    expect(result.current.saveState).toBe("saved");
    expect(applyEditorCommands.mock.calls[0][0].commands[0].text).toBe("pending passage");
  });

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
