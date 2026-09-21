/**
 * 工作区标题下的处理进度小字，**以处理任务的真实阶段为准**（`processing_jobs_v2.stage`）。
 *
 * 以前按 `job.currentStep === "LlmReview"` 判断，而 currentStep 在本地识别结束时就停在
 * `Authoring`，之后的云端识别与最长十分钟的自动检查都不再动它，于是这行字从来不出现。
 */
export function processingNoteOf(
  processing: { stage?: string; localStatus?: string; cloudStatus?: string } | null | undefined
): string | undefined {
  switch (processing?.stage) {
    case "queued":
    case "running":
    case "local_recognition":
      return "正在本机识别…";
    case "cloud_recognition":
    case "reconciling":
      return "本地已完成 · 云端自动检查中";
    default:
      return undefined;
  }
}
