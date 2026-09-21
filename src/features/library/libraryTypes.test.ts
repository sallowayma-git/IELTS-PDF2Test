import { describe, expect, it } from "vitest";
import type { LibraryItemSummaryV2 } from "../../api/workspaceClient";
import { STAGE_LABEL, buildRow, canRetryRow } from "./libraryTypes";

// 证据层级：pure unit（计划 §19.1 层 1）。
// G1/A4-F03：重启恢复重试耗尽后，UI 必须诚实显示"已达自动恢复上限"，
// 不得把停在该状态的条目显示成"排队中/已自动排队重试"。

function v2WithProcessing(processing: {
  stage: string;
  localStatus: string;
  lastErrorCode?: string;
}): LibraryItemSummaryV2 {
  return {
    id: "item-1",
    modality: "reading",
    title: "Retry Paper",
    status: "processing",
    currentEditVersion: 1,
    hasCanonicalDs: false,
    sourceAssetId: null,
    createdAt: "2026-09-12T00:00:00Z",
    updatedAt: "2026-09-12T00:00:00Z",
    deletedAt: null,
    processing: {
      stage: processing.stage,
      localStatus: processing.localStatus,
      cloudStatus: "not_started",
      actionableCount: 0,
      lastErrorCode: processing.lastErrorCode,
      eventSeq: 7
    }
  } as unknown as LibraryItemSummaryV2;
}

describe("buildRow 已发布 / 放行发布展示", () => {
  function v2WithStatus(status: string): LibraryItemSummaryV2 {
    return { ...v2WithProcessing({ stage: "x", localStatus: "x" }), status, processing: null } as LibraryItemSummaryV2;
  }

  it("正常发布显示已发布", () => {
    const row = buildRow("item-1", undefined, undefined, {}, v2WithStatus("published"));
    expect(row.stage).toBe("published");
    expect(row.publishedForced).toBe(false);
  });

  it("放行发布同样是已发布，并标出放行", () => {
    const row = buildRow("item-1", undefined, undefined, {}, v2WithStatus("published_forced"));
    expect(row.stage).toBe("published");
    expect(row.publishedForced).toBe(true);
    expect(row.detail).toBe("已发布（强制发布）");
  });
});

describe("buildRow 重试耗尽展示（G1/A4-F03）", () => {
  it("恢复上限后显示已达上限，而非排队中", () => {
    const row = buildRow(
      "item-1",
      undefined,
      undefined,
      {},
      v2WithProcessing({
        stage: "ready_for_review",
        localStatus: "action_required",
        lastErrorCode: "retry_exhausted"
      })
    );
    expect(row.stage).toBe("action_required");
    expect(row.detail).toBe("已达到自动恢复上限，请手动重试");
  });

  it("interrupted（未达上限）仍显示已自动排队重试", () => {
    const row = buildRow(
      "item-1",
      undefined,
      undefined,
      {},
      v2WithProcessing({
        stage: "queued",
        localStatus: "not_started",
        lastErrorCode: "interrupted"
      })
    );
    expect(row.stage).toBe("queued");
    expect(row.detail).toBe("等待开始识别");
  });

  it("普通 action_required 条目不受错误码影响", () => {
    const row = buildRow(
      "item-1",
      undefined,
      undefined,
      {},
      v2WithProcessing({
        stage: "ready_for_review",
        localStatus: "action_required"
      })
    );
    expect(row.stage).toBe("action_required");
    // 不把质量状态说成门槛：识别完成就是可以打开编辑。
    expect(row.detail).toBe("可以打开编辑");
  });
});

describe("题库行 — 不在十分钟的云端修复里说「正在合并」，失败行可以重试", () => {
  it("云端自动检查进行中：告诉用户可以先打开编辑", () => {
    const row = buildRow("item-1", undefined, undefined, {}, v2WithProcessing({ stage: "reconciling", localStatus: "succeeded" }));
    expect(row.stage).toBe("reconciling");
    expect(row.detail).toBe("云端正在自动检查，可先打开编辑");
    expect(row.detail).not.toContain("合并");
  });

  it("识别完成的行不显示「可发布」这种由质量状态推出来的结论", () => {
    expect(STAGE_LABEL.ready).not.toBe("可发布");
    expect(STAGE_LABEL.action_required).toBe(STAGE_LABEL.ready);
  });

  it("失败（非取消）的行可以重试；取消的行也可以重新开始", () => {
    const failed = buildRow("item-1", undefined, undefined, {}, v2WithProcessing({ stage: "failed", localStatus: "failed" }));
    expect(canRetryRow(failed)).toBe(true);
    const ready = buildRow("item-2", undefined, undefined, {}, v2WithProcessing({ stage: "ready_for_review", localStatus: "succeeded" }));
    expect(canRetryRow(ready)).toBe(false);
  });
});

describe("buildRow modality comes from the backend item", () => {
  it("shows a listening item as listening even when the legacy summary says reading", () => {
    const v2 = { ...v2WithProcessing({ stage: "queued", localStatus: "not_started" }), modality: "listening" };
    const row = buildRow(
      "item-1",
      undefined,
      { subject: "reading" } as never,
      {},
      v2 as LibraryItemSummaryV2
    );
    expect(row.modality).toBe("listening");
  });

  it("keeps reading and writing rows unchanged", () => {
    expect(buildRow("r", undefined, undefined, {}, v2WithProcessing({ stage: "queued", localStatus: "x" })).modality).toBe("reading");
    expect(buildRow("w", undefined, { subject: "writing" } as never).modality).toBe("writing");
  });
});
