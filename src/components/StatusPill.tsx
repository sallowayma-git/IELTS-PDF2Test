import type { JobStatus } from "../types";
import { jobStatusLabel } from "../utils/displayLabels";

export function StatusPill({ status }: { status: JobStatus }) {
  return <span className={`status-pill status-${status.toLowerCase()}`}>{jobStatusLabel(status)}</span>;
}
