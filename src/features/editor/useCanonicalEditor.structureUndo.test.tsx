// @vitest-environment jsdom
//
// 结构删除 / 添加的撤销与重做（hook 层）：编译出的结构补丁经 applyPatch 进入同一条撤销栈，
// Ctrl+Z 的 undo 与工具栏按钮是同一个函数。证据层级：hook 单元（workspaceClient 以替身代替）。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

const applyEditorCommands = vi.fn();
const getWorkspaceItem = vi.fn();

vi.mock("../../api/workspaceClient", () => ({
  applyEditorCommands: (...args: unknown[]) => applyEditorCommands(...args),
  getWorkspaceItem: (...args: unknown[]) => getWorkspaceItem(...args),
}));

import { useCanonicalEditor } from "./useCanonicalEditor";
import { compileStructureAction } from "../../exam-canvas/structureActions";
import { structureDraft } from "../../exam-canvas/structureDeleteFixture";
import type { IeltsAuthoringIRV2 } from "../../types";

const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

function workspaceFor(ds: IeltsAuthoringIRV2) {
  return { item: { title: "Structure" }, ds, editVersion: 1, recentEdits: [] };
}

beforeEach(() => {
  applyEditorCommands.mockReset();
  getWorkspaceItem.mockReset();
  applyEditorCommands.mockImplementation((batch: { baseVersion: number }) => Promise.resolve({ editVersion: batch.baseVersion + 1, appliedCount: 1, replayed: false }));
  try { localStorage.clear(); } catch { /* jsdom */ }
});
afterEach(() => cleanup());

async function loaded(initial: IeltsAuthoringIRV2) {
  getWorkspaceItem.mockResolvedValue(workspaceFor(clone(initial)));
  const hook = renderHook(() => useCanonicalEditor("structure-item"));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  return hook;
}

describe("结构删除与添加进入撤销栈", () => {
  it("删除一题：Ctrl+Z（undo）恢复，重做再删，题号与组内顺序都回到原样", async () => {
    const initial = structureDraft();
    const { result } = await loaded(initial);
    const before = clone(result.current.draft!);

    act(() => result.current.applyPatch(compileStructureAction(result.current.draft!, { type: "answer-slot.delete", slotId: "q15" })!));
    expect(result.current.draft!.answerSlots.q15).toBeUndefined();
    expect(result.current.canUndo).toBe(true);

    act(() => result.current.undo());
    expect(result.current.draft).toEqual(before);
    expect(result.current.canRedo).toBe(true);

    act(() => result.current.redo());
    expect(result.current.draft!.answerSlots.q15).toBeUndefined();
    expect(result.current.draft!.taskGroups[0].responseGroups[0].slotIds).toEqual(["q14"]);
  });

  it("添加一题后撤销：新题消失并恢复原稿", async () => {
    const { result } = await loaded(structureDraft());
    const before = clone(result.current.draft!);
    act(() => result.current.applyPatch(compileStructureAction(result.current.draft!, { type: "answer-slot.append", taskId: "headings", responseGroupId: "rg-a" })!));
    expect(result.current.draft!.taskGroups[0].responseGroups[0].slotIds).toHaveLength(3);
    act(() => result.current.undo());
    expect(result.current.draft).toEqual(before);
  });

  it("删除整道大题后撤销：大题、题目与答案全部回来", async () => {
    const { result } = await loaded(structureDraft());
    const before = clone(result.current.draft!);
    act(() => result.current.applyPatch(compileStructureAction(result.current.draft!, { type: "task-group.delete", taskId: "headings" })!));
    expect(result.current.draft!.taskGroups).toHaveLength(0);
    act(() => result.current.undo());
    expect(result.current.draft).toEqual(before);
    expect(result.current.draft!.answerKey.q16).toEqual({ kind: "unresolved" });
  });

  it("多步结构编辑按后进先出撤销", async () => {
    const { result } = await loaded(structureDraft());
    const original = clone(result.current.draft!);
    act(() => result.current.applyPatch(compileStructureAction(result.current.draft!, { type: "answer-slot.delete", slotId: "q15" })!));
    const afterFirst = clone(result.current.draft!);
    act(() => result.current.applyPatch(compileStructureAction(result.current.draft!, { type: "response-group.delete", taskId: "headings", responseGroupId: "rg-b" })!));
    act(() => result.current.undo());
    expect(result.current.draft).toEqual(afterFirst);
    act(() => result.current.undo());
    expect(result.current.draft).toEqual(original);
  });
});
