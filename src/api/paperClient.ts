import { command } from "./tauriCommands";

// 当前试卷（契约 CurrentPaperV1，见 docs/plans/2026-11-07-current-paper-contract.md）。
// 字段名与学生端读取的文件完全一致，不得在这里改名。

export interface CurrentPaperEntry {
  part: string;
  examId: string;
  title: string;
  script: string;
  scriptSha256: string;
  runtimeSha256: string;
}

export interface CurrentPaperV1 {
  schemaVersion: "CurrentPaperV1" | string;
  paperId: string;
  publishedAt: string;
  batchId: string;
  reading: CurrentPaperEntry[];
  listening: CurrentPaperEntry[];
}

/** 读取 NAS 上的当前试卷；文件不存在时为 null。 */
export function getCurrentPaper(destination: string): Promise<CurrentPaperV1 | null> {
  return command<CurrentPaperV1 | null>("get_current_paper", { destination });
}
