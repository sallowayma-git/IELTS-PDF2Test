// @vitest-environment jsdom
//
// D3：崩溃恢复只存增量。checkpoint 不再把整份稿写进 localStorage；重开时从服务端
// 权威稿 + 未提交命令重建，版本已变则走既有冲突流程。证据层级：hook 单元。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

const applyEditorCommands = vi.fn();
const getWorkspaceItem = vi.fn();

vi.mock("../../api/workspaceClient", () => ({
  applyEditorCommands: (...args: unknown[]) => applyEditorCommands(...args),
  getWorkspaceItem: (...args: unknown[]) => getWorkspaceItem(...args),
}));

import { useCanonicalEditor } from "./useCanonicalEditor";

const RECOVERY_KEY = "ielts-author-studio.workspace-recovery.v1:it-1";
const sampleDs = () => ({
  schemaVersion: "IeltsAuthoringIRV2",
  exam: { title: "T" },
  taskGroups: [],
  answerSlots: { s1: { slotId: "s1", questionNumber: 1, interaction: "text" } },
  answerKey: { s1: { kind: "text", values: ["v0"] } },
});
const setAnswer = (value: string) => ({ op: "setAnswer", slotId: "s1", value: { kind: "text", values: [value], normalization: "ielts_default" } });
const conflict = (current: number, base: number) => new Error(`EDIT_VERSION_CONFLICT:current=${current}:base=${base}`);
const answerOf = (result: { current: { draft?: unknown } }) => (result.current.draft as { answerKey: { s1: { values: string[] } } }).answerKey.s1.values[0];

// 该 vitest/jsdom 环境未启用 localStorage；用内存实现顶替，才能验证 checkpoint 落盘与重开恢复。
function installMemoryLocalStorage(): void {
  const store = new Map<string, string>();
  const mock = {
    getItem: (key: string) => (store.has(key) ? store.get(key)! : null),
    setItem: (key: string, value: string) => { store.set(key, String(value)); },
    removeItem: (key: string) => { store.delete(key); },
    clear: () => store.clear(),
    key: (index: number) => [...store.keys()][index] ?? null,
    get length() { return store.size; },
  };
  Object.defineProperty(globalThis, "localStorage", { value: mock, configurable: true, writable: true });
}

beforeEach(() => {
  applyEditorCommands.mockReset();
  getWorkspaceItem.mockReset();
  installMemoryLocalStorage();
});
afterEach(() => cleanup());

describe("useCanonicalEditor 崩溃恢复只存增量（D3）", () => {
  it("checkpoint 不写整份稿，只存 version/pending", async () => {
    applyEditorCommands.mockRejectedValue(new Error("kept-pending")); // 保存不成功，恢复记录保留
    getWorkspaceItem.mockResolvedValue({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    act(() => result.current.applyPatch(setAnswer("v1") as never));
    const raw = localStorage.getItem(RECOVERY_KEY);
    expect(raw).toBeTruthy();
    const record = JSON.parse(raw!);
    expect(record.draft).toBeUndefined();
    expect(record.pending.length).toBe(1);
    expect(record.version).toBe(1);
  });

  it("重开时服务端版本一致：从权威稿重放未提交命令并补发成功", async () => {
    localStorage.setItem(RECOVERY_KEY, JSON.stringify({ version: 1, pending: [setAnswer("recovered")] }));
    getWorkspaceItem.mockResolvedValue({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
    applyEditorCommands.mockResolvedValue({ editVersion: 2, appliedCount: 1, replayed: false });

    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    await waitFor(() => expect(answerOf(result)).toBe("recovered"));
    await waitFor(() => expect(result.current.saveState).toBe("saved"), { timeout: 4000 });
    expect(result.current.version).toBe(2);
  });

  it("旧格式恢复记录（含整份稿）：忽略陈旧整稿，仍从服务端+增量重建", async () => {
    const staleDraft = sampleDs();
    staleDraft.answerKey.s1.values[0] = "STALE_SHOULD_BE_IGNORED";
    localStorage.setItem(RECOVERY_KEY, JSON.stringify({ draft: staleDraft, version: 1, pending: [setAnswer("recovered")] }));
    getWorkspaceItem.mockResolvedValue({ item: { title: "T" }, ds: sampleDs(), editVersion: 1, recentEdits: [] });
    applyEditorCommands.mockResolvedValue({ editVersion: 2, appliedCount: 1, replayed: false });

    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    await waitFor(() => expect(answerOf(result)).toBe("recovered"));
  });

  it("重开时服务端版本已变且区间有人工写入：进入人工冲突，不静默覆盖", async () => {
    localStorage.setItem(RECOVERY_KEY, JSON.stringify({ version: 1, pending: [setAnswer("recovered")] }));
    getWorkspaceItem.mockResolvedValue({ item: { title: "T" }, ds: sampleDs(), editVersion: 2, recentEdits: [{ baseVersion: 1, origin: "human" }] });
    applyEditorCommands.mockRejectedValue(conflict(2, 1));

    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    await waitFor(() => expect(result.current.saveState).toBe("conflict"), { timeout: 4000 });
    expect(result.current.pendingCount).toBeGreaterThan(0);
  });
});
