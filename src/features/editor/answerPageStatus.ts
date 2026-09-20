import type { AutoPipelineReport } from "../../types";

export interface AnswerPageStatusView {
  state: "succeeded" | "failed" | "not_executed";
  stateReason?: string;
  message: string;
  detail?: string;
  canRetry: boolean;
}

/**
 * Turns the persisted answer-page recognition state into an actionable user
 * message.  Older reports have no state and are intentionally left silent;
 * they cannot safely be interpreted as either "no answer page" or "failed".
 */
export function answerPageStatusOf(report?: AutoPipelineReport): AnswerPageStatusView | undefined {
  const extraction = report?.parser?.visionAnswerExtraction;
  if (!extraction?.state) return undefined;
  // `not_executed/cloud_not_requested` is the normal local-only/no-profile
  // baseline, not a failed attempt.  Keep the workspace quiet until the
  // answer-page path actually ran; an attempted no-page result is still
  // surfaced below because it gives the user the correct manual-fill action.
  if (!extraction.attempted && extraction.stateReason !== "no_answer_page") return undefined;

  const violations = extraction.constraintViolations?.length ?? 0;
  if (extraction.state === "not_executed" && extraction.stateReason === "no_answer_page") {
    return {
      state: extraction.state,
      stateReason: extraction.stateReason,
      message: "原文件没有可识别的扫描答案页，请手工填写答案。",
      canRetry: false
    };
  }
  if (extraction.state === "not_executed") {
    return {
      state: extraction.state,
      stateReason: extraction.stateReason,
      message: "答案页识别这次未执行完成，题稿仍可编辑，请重试。",
      detail: extraction.stateReason === "credentials_invalid"
        ? "视觉服务凭据无效；修正凭据后再试。"
        : "没有写入任何答案。",
      canRetry: true
    };
  }
  if (extraction.state === "failed") {
    return {
      state: extraction.state,
      stateReason: extraction.stateReason,
      message: "答案页识别失败，返回内容无法核验；题稿仍可编辑，请重试。",
      detail: "没有写入任何答案。",
      canRetry: true
    };
  }
  if (violations > 0) {
    return {
      state: extraction.state,
      stateReason: extraction.stateReason,
      message: "答案页识别结果有答案不符合题型约束，已保留为未解析，请核对答案页。",
      detail: `${violations} 条答案未写入题稿。`,
      canRetry: true
    };
  }
  if ((extraction.answerCount ?? 0) === 0) {
    return {
      state: extraction.state,
      stateReason: extraction.stateReason,
      message: "视觉模型检查了答案页但没有产出可核验答案，请手工填写答案。",
      canRetry: true
    };
  }
  return {
    state: extraction.state,
    stateReason: extraction.stateReason,
    message: `答案页已识别 ${extraction.answerCount} 个答案，请在题稿中复核。`,
    canRetry: false
  };
}
