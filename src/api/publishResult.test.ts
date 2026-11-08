import { describe, expect, it } from "vitest";
import {
  describePublishResult,
  PAPER_POINTER_STALE_NOTICE,
  PUBLISH_LOCK_BUSY_NOTICE,
  type PublishBatchOutcome
} from "./publishClient";

const titleOf = (id: string) => `题-${id}`;
const base: PublishBatchOutcome = { destination: "/nas", succeeded: [], failed: [] };
const okItem = { itemId: "a", ok: true, studentLoadable: true };

describe("describePublishResult（审计 A5：按 status 给唯一一致的提示）", () => {
  it("published：当前试卷已发布，不再附带「已发布 N 题」", () => {
    const text = describePublishResult(
      {
        ...base,
        status: "published",
        succeeded: [okItem],
        paper: { ok: true, paperId: "abcdef1234567890", publishedAt: "t", batchId: "b", reading: [], listening: [] }
      },
      titleOf
    );
    expect(text).toBe("当前试卷已发布（abcdef12）");
    expect(text).not.toMatch(/学生端仍是旧试卷/);
  });

  it("published_with_warnings：已发布，但说明有记录未完整写入", () => {
    const text = describePublishResult(
      {
        ...base,
        status: "published_with_warnings",
        warnings: ["x", "y"],
        paper: { ok: true, paperId: "abcdef1234567890", publishedAt: "t", batchId: "b", reading: [], listening: [] }
      },
      titleOf
    );
    expect(text).toContain("当前试卷已发布（abcdef12）");
    expect(text).toContain("2 项记录未完整写入");
  });

  it("paper_pointer_stale：清单更新但学生端仍是旧试卷，不说成已发布", () => {
    const text = describePublishResult(
      {
        ...base,
        status: "paper_pointer_stale",
        succeeded: [okItem],
        paper: { ok: false, code: "PAPER_WRITE_FAILED", message: "rename" }
      },
      titleOf
    );
    expect(text).toBe(PAPER_POINTER_STALE_NOTICE);
    expect(text).not.toMatch(/已发布/);
  });

  it("paper_incomplete：缺题、旧试卷保持不变，列出题名", () => {
    const text = describePublishResult(
      {
        ...base,
        status: "paper_incomplete",
        paper: { ok: false, code: "PAPER_INCOMPLETE", missing: ["p2"] }
      },
      titleOf
    );
    expect(text).toBe("试卷中有题目未能发布，当前试卷保持不变：题-p2");
    expect(text).not.toMatch(/已发布/);
  });

  it("发布锁忙：不说成「当前试卷没有更新」，给出锁忙提示", () => {
    const text = describePublishResult(
      { ...base, failed: [{ itemId: "a", ok: false, message: PUBLISH_LOCK_BUSY_NOTICE }] },
      titleOf
    );
    expect(text).toBe(PUBLISH_LOCK_BUSY_NOTICE);
  });

  it("无组卷结果且无失败原因：当前试卷没有更新", () => {
    expect(describePublishResult({ ...base }, titleOf)).toBe("当前试卷没有更新。");
  });
});
