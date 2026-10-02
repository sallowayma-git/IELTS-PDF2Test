// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it } from "vitest";
import { useConfirmedHints } from "./useConfirmedHints";
import type { UserTaskV1 } from "./userTasks";

const hint: UserTaskV1 = {
  taskId: "difference-q10", kind: "cloud-difference", severity: "warning",
  title: "第 10 题题干有差异", comparison: { current: "Local", cloud: "Cloud", cloudLabel: "云端" },
  actions: [{ id: "view-source", label: "查看原文", targetId: "q10" }], covers: []
};
beforeEach(() => localStorage.clear());
afterEach(cleanup);

it("同一提示的新差异重新出现，题目之间的确认状态互不影响", () => {
  const { result, rerender } = renderHook(({ itemId }) => useConfirmedHints(itemId), { initialProps: { itemId: "first" } });
  act(() => result.current.confirm(hint));
  expect(result.current.isConfirmed(hint)).toBe(true);
  expect(result.current.isConfirmed({ ...hint, comparison: { ...hint.comparison!, cloud: "New cloud result" } })).toBe(false);
  rerender({ itemId: "second" });
  expect(result.current.isConfirmed(hint)).toBe(false);
  rerender({ itemId: "first" });
  expect(result.current.isConfirmed(hint)).toBe(true);
});
