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

const CLOUD_REVIEW_STAGES = [
  "queued", "running", "preparing_source", "local_recognition", "cloud_recognition", "reconciling"
];

/**
 * 云端校核是否进行中——与后端 `processing/queue.rs::cloud_review_in_progress` 的
 * stage + cloud_status 判据对齐，作为前端锁定人工编辑的依据。
 *
 * 后端还有一条 `cancel_requested_at IS NULL`：用户请求停止后立即解锁。该字段不在这里的
 * 处理状态里，由工作区的本地"已请求停止"标记先行解锁，随后阶段离开校核集时自然收敛。
 */
export function cloudReviewInProgress(
  processing: { stage?: string; cloudStatus?: string } | null | undefined
): boolean {
  if (!processing?.stage || !CLOUD_REVIEW_STAGES.includes(processing.stage)) return false;
  return processing.cloudStatus === "queued" || processing.cloudStatus === "running" || processing.stage === "reconciling";
}
