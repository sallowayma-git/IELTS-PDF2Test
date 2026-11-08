import { describe, expect, it } from "vitest";
import { replacedPaperTitles } from "./LibraryPage";
import { importBadgeCount, importQueueGroups, libraryRows, type LibraryRowV1, type LibraryStageV1 } from "./libraryTypes";

// 证据层级：pure unit。导入 / 题库 的行归属与导入角标（同一份行模型，过滤不同）。

function row(id: string, stage: LibraryStageV1, extra: Partial<LibraryRowV1> = {}): LibraryRowV1 {
  return {
    id,
    title: id,
    modality: "reading",
    stage,
    actionableCount: 0,
    questionTypes: [],
    updatedAt: "2026-11-07T00:00:00Z",
    inTrash: false,
    publishedForced: false,
    publishedNotLoadable: false,
    raw: {},
    ...extra
  };
}

describe("导入页的三组队列", () => {
  it("识别中 / 待检查 / 失败各归其组，已可发布的不在导入页", () => {
    const rows = [
      row("q", "queued"),
      row("l", "local"),
      row("c", "cloud"),
      row("r", "reconciling"),
      row("a", "action_required"),
      row("f", "failed"),
      row("ok", "ready"),
      row("pub", "published")
    ];
    const groups = importQueueGroups(rows);
    expect(groups.working.map((r) => r.id).sort()).toEqual(["c", "l", "q", "r"]);
    expect(groups.review.map((r) => r.id)).toEqual(["a"]);
    expect(groups.failed.map((r) => r.id)).toEqual(["f"]);
    expect(importBadgeCount(rows)).toBe(6);
  });

  it("回收站与写作不进导入队列", () => {
    const rows = [row("trashed", "failed", { inTrash: true }), row("w", "action_required", { modality: "writing" })];
    expect(importQueueGroups(rows)).toEqual({ working: [], review: [], failed: [] });
    expect(importBadgeCount(rows)).toBe(0);
  });
});

describe("题库页只显示可发布与已发布", () => {
  it("待检查、识别中、失败的题不出现在题库，写作也不在表格里", () => {
    const rows = [
      row("ready", "ready"),
      row("pub", "published"),
      row("review", "action_required"),
      row("working", "local"),
      row("failed", "failed"),
      row("trashed", "ready", { inTrash: true }),
      row("writing", "ready", { modality: "writing" })
    ];
    expect(libraryRows(rows).map((r) => r.id)).toEqual(["ready", "pub"]);
  });
});

describe("确认框里的被替换试卷", () => {
  it("列出当前试卷的阅读与听力题名；没有当前试卷时为空", () => {
    expect(replacedPaperTitles(null)).toEqual([]);
    expect(replacedPaperTitles(undefined)).toEqual([]);
    const paper = {
      schemaVersion: "CurrentPaperV1", paperId: "p", publishedAt: "", batchId: "b",
      reading: [
        { part: "P1", examId: "e1", title: "阅读甲", script: "", scriptSha256: "", runtimeSha256: "" },
        { part: "P2", examId: "e2", title: "阅读乙", script: "", scriptSha256: "", runtimeSha256: "" }
      ],
      listening: [{ part: "Part 1", examId: "e3", title: "听力丙", script: "", scriptSha256: "", runtimeSha256: "" }]
    };
    expect(replacedPaperTitles(paper)).toEqual(["阅读甲", "阅读乙", "听力丙"]);
  });
});
