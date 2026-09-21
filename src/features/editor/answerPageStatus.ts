import type { AutoPipelineReport } from "../../types";

export interface AnswerPageStatusView {
  state: "succeeded" | "failed" | "not_executed";
  stateReason?: string;
  message: string;
  detail?: string;
  canRetry: boolean;
}

/**
 * 答案页识别的工作区提示：**只在可重试的失败时出现**。
 *
 * 成功（哪怕部分答案因不符合题型而没写入）、原文没有答案页、没请求云端都不出提示：
 * 还没答案的题会出现在安静的「还没有答案的题」列表里，用户在那儿填就行。一条常驻的
 * 警告横幅只会让人以为「还有什么没处理完」。
 *
 * 旧报告没有 `state` 字段，既不能当「没有答案页」也不能当「失败」，保持安静。
 */
export function answerPageStatusOf(report?: AutoPipelineReport): AnswerPageStatusView | undefined {
  const extraction = report?.parser?.visionAnswerExtraction;
  if (!extraction?.state) return undefined;
  if (!extraction.attempted) return undefined;
  if (extraction.state === "not_executed" && extraction.stateReason === "no_answer_page") return undefined;

  if (extraction.state === "not_executed") {
    const credentials = extraction.stateReason === "credentials_invalid";
    return {
      state: extraction.state,
      stateReason: extraction.stateReason,
      message: "答案页识别这次未执行完成，题稿仍可编辑。",
      detail: credentials
        ? "云端密钥无效，请到设置里重新填写后再试。没有写入任何答案。"
        : "没有写入任何答案。",
      canRetry: true
    };
  }
  if (extraction.state === "failed") {
    return {
      state: extraction.state,
      stateReason: extraction.stateReason,
      message: "答案页识别返回的内容无法核验，题稿仍可编辑。",
      detail: "没有写入任何答案。",
      canRetry: true
    };
  }
  return undefined;
}

/** 手动重试之后的回执（只重跑答案页这一步）。 */
export function describeAnswerPageRetry(result: { state?: string; stateReason?: string; answerCount?: number; appliedCount?: number } | undefined): string {
  if (!result) return "答案页识别没有返回结果，题稿仍可编辑。";
  if (result.stateReason === "no_cloud_profile") return "还没有连接云端，无法识别答案页。可以到设置里连接，或直接手工填写答案。";
  if (result.state === "succeeded") {
    const count = result.answerCount ?? result.appliedCount ?? 0;
    return count > 0 ? `答案页识别完成，写入了 ${count} 个答案。` : "答案页识别完成，没有可写入的答案，请手工填写。";
  }
  if (result.stateReason === "credentials_invalid") return "云端密钥无效，请到设置里重新填写后再试。";
  return "答案页识别仍未成功，题稿仍可编辑，可稍后再试或手工填写答案。";
}
