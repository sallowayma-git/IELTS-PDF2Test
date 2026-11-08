import type { AutoPipelineReport } from "../../types";

export interface AnswerPageStatusView {
  state: "succeeded" | "failed" | "not_executed";
  stateReason?: string;
  message: string;
  detail?: string;
  canRetry: boolean;
}

function reasonFromFailure(failure?: string | null): string | undefined {
  if (!failure) return undefined;
  const lower = failure.toLowerCase();
  if (lower.startsWith("cloud_token_budget_usage_unavailable:")) return "usage_unavailable";
  if (lower.startsWith("cloud_token_budget_exceeded:")) return "budget_exceeded";
  if (lower.startsWith("llm_output_truncated:")) return "output_truncated";
  if (lower.startsWith("vision_answer_evidence_page_not_supplied:")) return "evidence_page_mismatch";
  if (/^llm_http_(401|403):/.test(lower)) return "credentials_invalid";
  if (/^llm_http_(5\d\d|408|425|429):/.test(lower)
    || ["llm_timeout_budget_exhausted:", "llm_http_timeout:", "llm_http_connect_failed:", "llm_http_transport_failed:", "llm_http_body_failed:"].some(prefix => lower.startsWith(prefix))) return "service_unavailable";
  return undefined;
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
  const stored = report?.parser?.visionAnswerExtraction;
  if (stored?.state === "succeeded") return undefined;
  // Older builds persisted budget stops as invalid model content. Correct the
  // display from the retained failure code without changing the saved draft.
  const reason = reasonFromFailure(stored?.failure) ?? stored?.stateReason;
  const extraction = stored ? { ...stored, stateReason: reason,
    state: reason === "service_unavailable" || reason === "credentials_invalid" ? "not_executed" as const : stored.state
  } : undefined;
  if (!extraction?.state) return undefined;
  if (!extraction.attempted) return undefined;
  if (extraction.state === "not_executed" && extraction.stateReason === "no_answer_page") return undefined;

  if (extraction.stateReason === "usage_unavailable" || extraction.stateReason === "budget_exceeded") {
    return {
      state: "not_executed",
      stateReason: extraction.stateReason,
      message: "云端用量限制阻止了答案页识别，题稿仍可编辑。",
      detail: extraction.stateReason === "usage_unavailable"
        ? "服务未返回可确认的 token 用量，这不表示已用完额度。没有写入任何答案。"
        : "本次识别已达到设置的 token 上限。没有写入任何答案。",
      canRetry: false
    };
  }

  if (extraction.state === "not_executed") {
    const credentials = extraction.stateReason === "credentials_invalid";
    const service = extraction.stateReason === "service_unavailable";
    const timeout = /timeout|timed out/i.test(extraction.failure ?? "");
    return {
      state: extraction.state,
      stateReason: extraction.stateReason,
      message: "答案页识别这次未执行完成，题稿仍可编辑。",
      detail: credentials
        ? "云端密钥无效，请到设置里重新填写后再试。没有写入任何答案。"
        : service
          ? timeout ? "请求超时，未取得完整响应。请在设置中延长请求超时后重试；没有写入任何答案。"
            : "云端服务未返回完整响应，请检查服务连接后重试。没有写入任何答案。"
        : "没有写入任何答案。",
      canRetry: true
    };
  }
  if (extraction.state === "failed") {
    return {
      state: extraction.state,
      stateReason: extraction.stateReason,
      message: extraction.stateReason === "output_truncated"
        ? "答案页识别响应被截断，题稿仍可编辑。"
        : extraction.stateReason === "evidence_page_mismatch"
          ? "答案页识别的证据页码与原文件不一致，题稿仍可编辑。"
        : "答案页识别返回的内容无法核验，题稿仍可编辑。",
      detail: extraction.stateReason === "evidence_page_mismatch"
        ? "返回的证据没有指向本次提交的答案页，没有写入任何答案。"
        : "没有写入任何答案。",
      canRetry: true
    };
  }
  return undefined;
}

/** 手动重试之后的回执（只重跑答案页这一步）。 */
export function describeAnswerPageRetry(result: { state?: string; stateReason?: string; answerCount?: number; appliedCount?: number; failure?: string | null } | undefined): string {
  if (!result) return "答案页识别没有返回结果，题稿仍可编辑。";
  if (result.stateReason === "no_cloud_profile") return "还没有连接云端，无法识别答案页。可以到设置里连接，或直接手工填写答案。";
  if (result.state !== "succeeded") {
    result = { ...result, stateReason: reasonFromFailure(result.failure) ?? result.stateReason };
    if (result.stateReason === "service_unavailable" || result.stateReason === "evidence_page_mismatch") {
      const view = answerPageStatusOf({ parser: { visionAnswerExtraction: { ...result, attempted: true, applied: false } } } as AutoPipelineReport);
      if (view) return `${view.message}${view.detail ?? ""}`;
    }
  }
  if (result.stateReason === "usage_unavailable") return "服务未返回可确认的 token 用量，答案页请求已停止，没有写入答案。";
  if (result.stateReason === "budget_exceeded") return "本次识别已达到 token 上限，答案页请求未执行，没有写入答案。";
  if (result.stateReason === "output_truncated") return "答案页识别响应被截断，没有写入答案，请调整云端输出上限后重试。";
  if (result.state === "succeeded") {
    const count = result.answerCount ?? result.appliedCount ?? 0;
    return count > 0 ? `答案页识别完成，写入了 ${count} 个答案。` : "答案页识别完成，没有可写入的答案，请手工填写。";
  }
  if (result.stateReason === "credentials_invalid") return "云端密钥无效，请到设置里重新填写后再试。";
  return "答案页识别仍未成功，题稿仍可编辑，可稍后再试或手工填写答案。";
}
