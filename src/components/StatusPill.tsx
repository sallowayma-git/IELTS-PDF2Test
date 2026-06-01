import type { JobStatus } from "../types";

const labels: Record<JobStatus, string> = {
  Working: "处理中",
  NeedsReview: "待审核",
  DraftSaved: "草稿已保存",
  ExportReady: "可导出",
  Exported: "已导出",
  Cleaned: "已清理"
};

export function StatusPill({ status }: { status: JobStatus }) {
  return <span className={`status-pill status-${status.toLowerCase()}`}>{labels[status]}</span>;
}
