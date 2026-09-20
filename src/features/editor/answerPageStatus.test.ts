import { describe, expect, it } from "vitest";
import { answerPageStatusOf } from "./answerPageStatus";
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

describe("answer-page recognition status", () => {
  it("keeps no-answer-page distinct from a service outage", () => {
    const noPage = answerPageStatusOf(report("not_executed", "no_answer_page"));
    const outage = answerPageStatusOf(report("not_executed", "service_unavailable"));

    expect(noPage?.message).toContain("没有可识别的扫描答案页");
    expect(noPage?.canRetry).toBe(false);
    expect(outage?.message).toContain("未执行完成");
    expect(outage?.canRetry).toBe(true);
  });

  it("offers retry for failed and invalid-credential recognition", () => {
    expect(answerPageStatusOf(report("failed", "invalid_response"))?.canRetry).toBe(true);
    expect(answerPageStatusOf(report("not_executed", "credentials_invalid"))?.detail).toContain("凭据无效");
  });

  it("does not present an outage action when cloud recognition was not requested", () => {
    expect(answerPageStatusOf(report("not_executed", "cloud_not_requested", { attempted: false }))).toBeUndefined();
  });

  it("reports semantic violations as unresolved work", () => {
    const status = answerPageStatusOf(report("succeeded", "answers_extracted", {
      answerCount: 12,
      constraintViolations: [{ code: "ANSWER_OPTION_NOT_ALLOWED" }]
    }));
    expect(status?.message).toContain("不符合题型约束");
    expect(status?.canRetry).toBe(true);
  });
});
