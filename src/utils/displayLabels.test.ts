import { describe, expect, it } from "vitest";
import type { ValidationIssue } from "../types";
import {
  jobStatusLabel,
  libraryStatusLabel,
  normalizeLibraryStatus,
  runtimeModeLabel,
  validationIssueDisplay,
  validationLayerLabel,
  workflowStepLabel
} from "./displayLabels";

// 证据层级：pure unit（计划 §19.1 层 1 / §15 文案收敛）。
// 断言「内部枚举 -> 用户文案」的映射与兜底，防止把原始枚举/机器串直接展示给用户。

function issue(partial: Partial<ValidationIssue>): ValidationIssue {
  return {
    issueId: "i1",
    severity: "warning",
    layer: "AuthoringIR",
    path: "$.answerKey.q1",
    message: "Question answer is empty",
    ...partial
  };
}

describe("normalizeLibraryStatus", () => {
  it("把后端别名归一化到前端枚举", () => {
    expect(normalizeLibraryStatus("review_required")).toBe("needs_review");
    expect(normalizeLibraryStatus("published")).toBe("exported");
  });

  it("已知枚举原样返回", () => {
    expect(normalizeLibraryStatus("draft")).toBe("draft");
    expect(normalizeLibraryStatus("ready")).toBe("ready");
  });

  it("空值与未知值返回 undefined", () => {
    expect(normalizeLibraryStatus(undefined)).toBeUndefined();
    expect(normalizeLibraryStatus("")).toBeUndefined();
    expect(normalizeLibraryStatus("bogus")).toBeUndefined();
  });
});

describe("状态文案", () => {
  it("libraryStatusLabel 归一化后取中文，未知值原样回显", () => {
    expect(libraryStatusLabel("review_required")).toBe("待审核");
    expect(libraryStatusLabel("exported")).toBe("已发布");
    expect(libraryStatusLabel("bogus")).toBe("bogus");
    expect(libraryStatusLabel(undefined)).toBe("未知状态");
  });

  it("jobStatusLabel / workflowStepLabel 有兜底", () => {
    expect(jobStatusLabel("Working")).toBe("处理中");
    expect(jobStatusLabel(undefined)).toBe("未知状态");
    expect(jobStatusLabel("NotAStatus")).toBe("NotAStatus");
    expect(workflowStepLabel("Split")).toBe("后台识别题组与答案");
    expect(workflowStepLabel(undefined)).toBe("未知步骤");
  });

  it("validationLayerLabel 与 runtimeModeLabel 的兜底", () => {
    expect(validationLayerLabel("AuthoringIR")).toBe("可编辑题稿");
    expect(validationLayerLabel("UnknownLayer" as never)).toBe("UnknownLayer");
    expect(runtimeModeLabel(undefined)).toBe("未运行");
    expect(runtimeModeLabel("real")).toBe("真实预览已通过");
    expect(runtimeModeLabel("static-rust")).toBe("基础检查已通过");
    expect(runtimeModeLabel("fallback")).toBe("开发预览检查");
    expect(runtimeModeLabel("weird")).toBe("weird");
  });
});

describe("validationIssueDisplay", () => {
  it("答案缺失：定位到具体题号并给出人话", () => {
    const view = validationIssueDisplay(issue({}));
    expect(view.title).toBe("答案 · Q1");
    expect(view.detail).toBe("题目未设置答案，将作为未评分题导出。");
    expect(view.action).toContain("可选项");
  });

  it("error 级答案问题的动作更强", () => {
    const view = validationIssueDisplay(issue({ severity: "error" }));
    expect(view.action).toContain("忽略检查");
  });

  it("源文档解析提醒带页码", () => {
    const view = validationIssueDisplay(
      issue({ path: "$.sourceReview.page3", message: "Parser warning must be manually resolved on page 3" })
    );
    expect(view.title).toBe("源文档审核");
    expect(view.detail).toBe("第 3 页没有可提取文字，需要完成视觉识别或人工确认。");
    expect(view.action).toContain("源文档审核页");
  });

  it("云端对照问题定位到题号", () => {
    const view = validationIssueDisplay(
      issue({ path: "cloudComparison.answerKey.q2", message: "Cloud differs from local" })
    );
    expect(view.title).toBe("云端整卷对照 · Q2");
    expect(view.action).toContain("云端/本地差异");
  });

  it("fixHint 优先于默认动作", () => {
    const view = validationIssueDisplay(issue({ fixHint: "手动指定答案后重试。" }));
    expect(view.action).toBe("手动指定答案后重试。");
  });

  it("未知 message 原样展示", () => {
    const view = validationIssueDisplay(issue({ path: "$.unknown", message: "Some raw detail" }));
    expect(view.detail).toBe("Some raw detail");
  });
});
