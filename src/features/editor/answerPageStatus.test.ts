import { describe, expect, it } from "vitest";
import { answerPageStatusOf, describeAnswerPageRetry } from "./answerPageStatus";
import type { AutoPipelineReport } from "../../types";

function report(state: "succeeded" | "failed" | "not_executed", stateReason: string, extra = {}): AutoPipelineReport {
  return {
    jobId: "job-1",
    confidenceThreshold: 0.85,
    llm: {
      suggestionCount: 0,
      appliedCount: 0,
      highConfidenceAppliedGroups: [],
      lowConfidenceGroups: [],
      failures: []
    },
    parser: {
      warnings: [],
      lowConfidenceBlocks: [],
      visionAnswerExtraction: {
        attempted: true,
        applied: false,
        state,
        stateReason,
        ...extra
      }
    },
    validationPassed: false,
    status: "NeedsReview",
    currentStep: "Authoring",
    generatedAt: "2026-09-20T00:00:00Z"
  };
}

describe("answer-page recognition status — 只在可重试的失败时出现", () => {
  it("预算停止不能冒充模型内容无法核验，也不能提供无效重试", () => {
    for (const reason of ["usage_unavailable", "budget_exceeded"]) {
      const view = answerPageStatusOf(report("not_executed", reason));
      expect(view?.canRetry).toBe(false);
      expect(view?.message).toContain("用量限制");
      expect(view?.message).not.toContain("无法核验");
      expect(describeAnswerPageRetry({ state: "not_executed", stateReason: reason })).toContain("没有写入答案");
    }
    expect(answerPageStatusOf(report("not_executed", "usage_unavailable"))?.detail).toContain("不表示已用完");
  });

  it("输出截断有准确的原因", () => {
    expect(answerPageStatusOf(report("failed", "output_truncated"))?.message).toContain("被截断");
    expect(describeAnswerPageRetry({ state: "failed", stateReason: "output_truncated" })).toContain("输出上限");
  });

  it("旧版本保存的预算失败横幅也按真实失败码显示", () => {
    const view = answerPageStatusOf(report("failed", "invalid_response", {
      failure: "cloud_token_budget_usage_unavailable:1:300000;old message"
    }));
    expect(view?.state).toBe("not_executed");
    expect(view?.canRetry).toBe(false);
    expect(view?.detail).toContain("不表示已用完");
  });
  it("服务暂时不可用：出现，并给出只重跑答案页的重试", () => {
    const outage = answerPageStatusOf(report("not_executed", "service_unavailable"));
    expect(outage?.message).toContain("未执行完成");
    expect(outage?.canRetry).toBe(true);
  });

  it("旧失败和重试回执按真实超时原因显示，不误报为无法核验", () => {
    const failure = "llm_timeout_budget_exhausted:llm_http_body_failed:error decoding response body";
    const view = answerPageStatusOf(report("failed", "invalid_response", { failure }));
    expect(view?.state).toBe("not_executed");
    expect(view?.detail).toContain("请求超时");
    expect(view?.message).not.toContain("无法核验");
    expect(describeAnswerPageRetry({ state: "failed", stateReason: "invalid_response", failure })).toContain("请求超时");
  });

  it("证据页码错误说明拒绝原因；成功报告不被残留旧错误覆盖", () => {
    const failure = "vision_answer_evidence_page_not_supplied:0:1:allowed=[5, 6]";
    expect(answerPageStatusOf(report("failed", "invalid_response", { failure }))?.message).toContain("证据页码");
    expect(answerPageStatusOf(report("succeeded", "answers_extracted", { failure: "cloud_token_budget_usage_unavailable:1:300000" }))).toBeUndefined();
  });

  it("视觉输出无法核验：出现，可重试，且说明没有写入任何答案", () => {
    const failed = answerPageStatusOf(report("failed", "invalid_response"));
    expect(failed?.canRetry).toBe(true);
    expect(failed?.detail).toContain("没有写入");
  });

  it("凭据无效：指向设置页", () => {
    const creds = answerPageStatusOf(report("not_executed", "credentials_invalid"));
    expect(creds?.detail).toContain("设置");
    expect(creds?.canRetry).toBe(true);
  });

  it("成功（含部分答案因不符合题型而未写入）不留一条常驻警告——没填的题进安静的未填列表", () => {
    expect(answerPageStatusOf(report("succeeded", "answers_extracted", { answerCount: 12 }))).toBeUndefined();
    expect(answerPageStatusOf(report("succeeded", "answers_extracted", {
      answerCount: 12,
      constraintViolations: [{ code: "ANSWER_OPTION_NOT_ALLOWED" }]
    }))).toBeUndefined();
  });

  it("原文没有答案页 / 没请求云端：不出现（要填的题在未填列表里）", () => {
    expect(answerPageStatusOf(report("not_executed", "no_answer_page"))).toBeUndefined();
    expect(answerPageStatusOf(report("not_executed", "cloud_not_requested", { attempted: false }))).toBeUndefined();
  });
});
describe("describeAnswerPageRetry", () => {
  it("没连云端时如实说，不说「已加入队列」", () => {
    expect(describeAnswerPageRetry({ state: "not_executed", stateReason: "no_cloud_profile" })).toContain("没有连接云端");
  });
  it("成功时说写入了几个答案", () => {
    expect(describeAnswerPageRetry({ state: "succeeded", answerCount: 13 })).toContain("13");
  });
});
