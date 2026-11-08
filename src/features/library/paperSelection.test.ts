import { describe, expect, it } from "vitest";
import {
  buildPaperSpec,
  EMPTY_PAPER_SELECTION,
  isPaperReady,
  reconcilePaperSelection,
  selectedPaperItemIds,
  toggleListeningItem,
  toggleReadingItem
} from "./paperSelection";

// 证据层级：pure unit。组卷选择规则：阅读每 Part 最多一篇（同 Part 再选即替换），
// 听力可多选，三篇阅读齐了才能组卷。

describe("阅读：每个 Part 只能选一篇", () => {
  it("同一 Part 再勾另一篇就替换，不会同时留两篇", () => {
    let selection = toggleReadingItem(EMPTY_PAPER_SELECTION, "P1", "a");
    selection = toggleReadingItem(selection, "P1", "b");
    expect(selection.reading.P1).toBe("b");
    expect(Object.values(selection.reading).filter(Boolean)).toEqual(["b"]);
  });

  it("替换一个 Part 不影响其它 Part", () => {
    let selection = toggleReadingItem(EMPTY_PAPER_SELECTION, "P1", "a");
    selection = toggleReadingItem(selection, "P2", "x");
    selection = toggleReadingItem(selection, "P1", "b");
    expect(selection.reading).toEqual({ P1: "b", P2: "x" });
  });

  it("再点已选中的那篇取消该 Part", () => {
    let selection = toggleReadingItem(EMPTY_PAPER_SELECTION, "P3", "c");
    selection = toggleReadingItem(selection, "P3", "c");
    expect(selection.reading.P3).toBeUndefined();
  });
});

describe("听力：可多选", () => {
  it("多篇都保留，再点取消单篇", () => {
    let selection = toggleListeningItem(EMPTY_PAPER_SELECTION, "l1", "Part 1");
    selection = toggleListeningItem(selection, "l2", "Part 2");
    expect(selection.listening.map((entry) => entry.itemId)).toEqual(["l1", "l2"]);
    selection = toggleListeningItem(selection, "l1", "Part 1");
    expect(selection.listening.map((entry) => entry.itemId)).toEqual(["l2"]);
  });
});

describe("三篇齐了才能组卷", () => {
  it("缺任何一个阅读 Part 都不可发布，且不生成请求体", () => {
    let selection = toggleReadingItem(EMPTY_PAPER_SELECTION, "P1", "a");
    selection = toggleReadingItem(selection, "P2", "b");
    expect(isPaperReady(selection)).toBe(false);
    expect(buildPaperSpec(selection)).toBeUndefined();
    selection = toggleReadingItem(selection, "P3", "c");
    expect(isPaperReady(selection)).toBe(true);
  });

  it("请求体字段与后端 PaperSpec 同名（reading.P1/P2/P3 与 listening[].part/itemId）", () => {
    let selection = toggleReadingItem(EMPTY_PAPER_SELECTION, "P1", "a");
    selection = toggleReadingItem(selection, "P2", "b");
    selection = toggleReadingItem(selection, "P3", "c");
    selection = toggleListeningItem(selection, "l1", "Part 3");
    expect(buildPaperSpec(selection)).toEqual({
      reading: { P1: "a", P2: "b", P3: "c" },
      listening: [{ part: "Part 3", itemId: "l1" }]
    });
    expect(selectedPaperItemIds(selection)).toEqual(["a", "b", "c", "l1"]);
  });
});

describe("题库刷新后对齐选择", () => {
  it("已删除、题型不符或 Part 被改走的题从选择里去掉", () => {
    let selection = toggleReadingItem(EMPTY_PAPER_SELECTION, "P1", "a");
    selection = toggleReadingItem(selection, "P2", "b");
    selection = toggleListeningItem(selection, "l1", "Part 1");
    const next = reconcilePaperSelection(selection, [
      { id: "a", modality: "reading", part: "P1" },
      { id: "b", modality: "reading", part: "P3" },
      { id: "l1", modality: "listening", part: "Part 2" }
    ]);
    expect(next.reading).toEqual({ P1: "a" });
    expect(next.listening).toEqual([{ itemId: "l1", part: "Part 2" }]);
  });
});
