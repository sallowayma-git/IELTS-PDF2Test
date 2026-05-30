import type { JobStatus } from "../types";

const labels: Record<JobStatus, string> = {
  Draft: "草稿",
  Uploaded: "已上传",
  Parsed: "已解析",
  SplitReady: "粗切完成",
  AuthoringReady: "编辑中",
  NeedsHumanReview: "待人工",
  ValidationFailed: "校验失败",
  PreviewReady: "可预览",
  ExportReady: "可导出",
  Published: "已发布"
};

export function StatusPill({ status }: { status: JobStatus }) {
  return <span className={`status-pill status-${status.toLowerCase()}`}>{labels[status]}</span>;
}
