// @vitest-environment jsdom
// Hook/IPC boundary evidence: structural commands retain the existing cloud-lock contract.
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import fixture from "../../contracts/fixtures/authoring-structure-parity.json";
import type { AuthoringPatchV2 } from "./index";

const applyEditorCommands = vi.fn();
const getWorkspaceItem = vi.fn();
vi.mock("../api/workspaceClient", () => ({
  applyEditorCommands: (...args: unknown[]) => applyEditorCommands(...args),
  getWorkspaceItem: (...args: unknown[]) => getWorkspaceItem(...args),
}));
import { useCanonicalEditor } from "../features/editor/useCanonicalEditor";

beforeEach(() => {
  applyEditorCommands.mockReset();
  getWorkspaceItem.mockReset();
  localStorage.clear();
  getWorkspaceItem.mockResolvedValue({ item: { title: "Rows" }, ds: fixture.initial, editVersion: 1, recentEdits: [] });
});
afterEach(cleanup);

it.each([fixture.patches[0], fixture.patches[2], fixture.patches[3], fixture.extraCases[0].patches[0], {op:"renumberQuestions",scope:"exam",startAt:11}])("locks and later saves $op without losing the structural edit", async (patch) => {
  applyEditorCommands.mockRejectedValueOnce(new Error("CLOUD_REVIEW_IN_PROGRESS:item"));
  applyEditorCommands.mockResolvedValue({ editVersion: 2, appliedCount: 1, replayed: false });
  const { result } = renderHook(() => useCanonicalEditor("item"));
  await waitFor(() => expect(result.current.loading).toBe(false));
  act(() => result.current.applyPatch(patch as AuthoringPatchV2));
  await waitFor(() => expect(result.current.saveState).toBe("locked"));
  const changed = structuredClone(result.current.draft);
  expect(result.current.pendingCount).toBe(1);
  expect(result.current.saveMessage).toBeFalsy();
  await act(async () => { await result.current.flush(); });
  await waitFor(() => expect(result.current.saveState).toBe("saved"));
  expect(result.current.draft).toEqual(changed);
  expect(applyEditorCommands.mock.calls[1][0].commands).toEqual([patch]);
  act(() => result.current.undo());
  expect(result.current.draft).toEqual(fixture.initial);
  act(() => result.current.redo());
  expect(result.current.draft).toEqual(changed);
});
