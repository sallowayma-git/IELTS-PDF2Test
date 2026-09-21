import { describe, expect, it, vi } from "vitest";
import { SAVED_TO_LIBRARY_NOTICE, saveToLibrary, sourceActionsAvailable } from "./finalVersion";

// 证据层级：pure unit。

describe("显式保存", () => {
  it("先把待保存编辑刷进题库，再报「已保存到题库」", async () => {
    const order: string[] = [];
    const flush = vi.fn(async () => {
      order.push("flush");
    });
    const notice = await saveToLibrary(flush);
    order.push("notice");
    expect(flush).toHaveBeenCalledTimes(1);
    expect(order).toEqual(["flush", "notice"]);
    expect(notice).toBe(SAVED_TO_LIBRARY_NOTICE);
    expect(notice).toBe("已保存到题库");
  });

  it("保存失败不得报「已保存」", async () => {
    await expect(saveToLibrary(() => Promise.reject(new Error("library_v2_tx:busy")))).rejects.toThrow();
  });
});

describe("原文件已删除的题目", () => {
  it("需要原文件的操作只在未清理时可用", () => {
    expect(sourceActionsAvailable(undefined)).toBe(true);
    expect(sourceActionsAvailable(false)).toBe(true);
    expect(sourceActionsAvailable(true)).toBe(false);
  });
});
