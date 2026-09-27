// @vitest-environment jsdom
//
// C1 修复回归：识别进行中打开工作区 / 后台机器写入并发时，编辑保存不能永久变红。
// 这里在 hook 层复现两条曾经会红的路径，并断言修复后能自动重放保存成功；
// 真正的人工冲突仍交给用户（不在这里断言"自动成功"，见 conflictRecovery.test.ts）。
//
// 证据层级：hook 单元（mock workspaceClient 的 IPC）。真实应用行为由
// scripts/e2e/tauri-cdp-edit-save-stress.mjs 验收。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

const applyEditorCommands = vi.fn();
const getWorkspaceItem = vi.fn();

vi.mock("../../api/workspaceClient", () => ({
  applyEditorCommands: (...args: unknown[]) => applyEditorCommands(...args),
  getWorkspaceItem: (...args: unknown[]) => getWorkspaceItem(...args),
}));

import { useCanonicalEditor } from "./useCanonicalEditor";

const sampleDs = (title: string) => ({ schemaVersion: "IeltsAuthoringIRV2", jobId: "job-1", exam: { title } });
const conflict = (current: number, base: number) => new Error(`EDIT_VERSION_CONFLICT:current=${current}:base=${base}`);

beforeEach(() => {
  applyEditorCommands.mockReset();
  getWorkspaceItem.mockReset();
  try { localStorage.clear(); } catch { /* jsdom */ }
});
afterEach(() => cleanup());

describe("useCanonicalEditor 冲突后恢复（C1）", () => {
  it("后台机器写入引发冲突后，新的编辑批次能自动重放并保存成功（autoRebaseTried 每批次复位）", async () => {
    // 加载在 version 5。
    getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: sampleDs("T"), editVersion: 5, recentEdits: [] });
    // 编辑 A：首发冲突 → 自动重放读到 v6（机器写入）→ 重放中又被机器推到 v7 → 本批次不再重放 → 卡 conflict。
    applyEditorCommands.mockRejectedValueOnce(conflict(6, 5));
    getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: sampleDs("T"), editVersion: 6, recentEdits: [{ baseVersion: 5, origin: "cloud_repair" }] });
    applyEditorCommands.mockRejectedValueOnce(conflict(7, 6));
    // 编辑 B（新批次）：修复后重新获得一次重放机会 → 读到 v7（区间全机器）→ 重放 → 保存成功。
    applyEditorCommands.mockRejectedValueOnce(conflict(7, 6));
    getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: sampleDs("T"), editVersion: 7, recentEdits: [{ baseVersion: 5, origin: "cloud_repair" }, { baseVersion: 6, origin: "cloud_repair" }] });
    applyEditorCommands.mockResolvedValueOnce({ editVersion: 8, appliedCount: 0, replayed: false });

    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.draft).toBeTruthy();

    act(() => result.current.setTitle("Edit A"));
    await waitFor(() => expect(result.current.saveState).toBe("conflict"), { timeout: 4000 });

    act(() => result.current.setTitle("Edit B"));
    await waitFor(() => expect(result.current.saveState).toBe("saved"), { timeout: 4000 });
    expect(result.current.version).toBe(8);
  });

  it("反例：区间内有人工来源写入（另一个窗口）时不自动重放，交用户处理（不静默覆盖）", async () => {
    getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: sampleDs("T"), editVersion: 5, recentEdits: [] });
    // 保存冲突：拉最新时区间里有一条 human 来源写入 → conflictWasMachineOnly=false → 不自动重放。
    applyEditorCommands.mockRejectedValueOnce(conflict(6, 5));
    getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: sampleDs("T"), editVersion: 6, recentEdits: [{ baseVersion: 5, origin: "human" }] });

    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    act(() => result.current.setTitle("我的改动"));
    await waitFor(() => expect(result.current.saveState).toBe("conflict"), { timeout: 4000 });
    // 只发起过一次保存尝试，没有自动重放覆盖对方的人工写入；本地改动仍在待保存队列里。
    expect(applyEditorCommands).toHaveBeenCalledTimes(1);
    expect(result.current.pendingCount).toBeGreaterThan(0);
  });

  it("识别进行中打开（ds=null）：采纳后端版本、标题编辑先暂存不以 base 0 发出；seed 后补发成功", async () => {
    // 首次加载：权威稿未 seed，但后端返回 editVersion=1。
    getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: null, editVersion: 1, recentEdits: [] });
    const { result } = renderHook(() => useCanonicalEditor("it-1"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.loadError).toContain("ITEM_DS_NOT_SEEDED");
    // 采纳了后端版本（不是停在 0）。
    expect(result.current.version).toBe(1);

    // 标题编辑：没有 draft，暂存不发（不得触发 applyEditorCommands）。
    act(() => result.current.setTitle("改个名字"));
    await new Promise((r) => setTimeout(r, 700)); // 超过 debounce
    expect(applyEditorCommands).not.toHaveBeenCalled();

    // 模拟 seed 完成后到达的处理事件 → 重试加载，这次拿到权威稿；补发暂存的标题。
    getWorkspaceItem.mockResolvedValueOnce({ item: { title: "T" }, ds: sampleDs("T"), editVersion: 1, recentEdits: [] });
    applyEditorCommands.mockResolvedValueOnce({ editVersion: 2, appliedCount: 0, replayed: false });
    act(() => result.current.noteRemoteVersion(1));
    await waitFor(() => expect(applyEditorCommands).toHaveBeenCalledTimes(1), { timeout: 4000 });
    // 补发用的是采纳后的 base 1，不是 0。
    expect(applyEditorCommands.mock.calls[0][0]).toMatchObject({ baseVersion: 1, title: "改个名字" });
    await waitFor(() => expect(result.current.saveState).toBe("saved"), { timeout: 4000 });
  });
});
