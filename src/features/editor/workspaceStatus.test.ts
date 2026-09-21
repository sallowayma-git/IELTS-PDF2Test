import { describe, expect, it } from "vitest";
import { processingNoteOf } from "./workspaceStatus";

// 工作区标题下的那行小字：以前按 `job.currentStep === "LlmReview"` 判断，而真实流程里
// currentStep 在本地识别结束时就停在 `Authoring`，于是「云端识别中」从来不出现。
describe("processingNoteOf — 以真实处理任务状态为准", () => {
  it("云端识别 / 云端自动检查阶段：告诉用户本地已完成、可以边改边等", () => {
    expect(processingNoteOf({ stage: "cloud_recognition", localStatus: "succeeded", cloudStatus: "running" })).toBe("本地已完成 · 云端自动检查中");
    expect(processingNoteOf({ stage: "reconciling", localStatus: "succeeded", cloudStatus: "running" })).toBe("本地已完成 · 云端自动检查中");
  });
  it("本机还在读", () => {
    expect(processingNoteOf({ stage: "local_recognition", localStatus: "running", cloudStatus: "queued" })).toBe("正在本机识别…");
  });
  it("结束之后不留小字", () => {
    expect(processingNoteOf({ stage: "ready_for_review", localStatus: "succeeded", cloudStatus: "succeeded" })).toBeUndefined();
    expect(processingNoteOf(undefined)).toBeUndefined();
  });
});
