import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { deleteLibraryExam, listJobs, listLibraryExams, listTrashedExams, restoreLibraryExam } from "../../api/tauriCommands";
import type { ImportJob, LibraryExamSummary } from "../../types";
import { buildRow, isProcessingStage, type LibraryRowV1 } from "./libraryTypes";

// 题库行 = 处理任务（ImportJob）与题库条目（LibraryExamSummary）按 id 合并。
// 当前数据模型下 library item id 与 job id 相同（见 findings F12），所以 id 可以直接做合并键。
//
// 后端还没有 `processing://item-updated` 事件（计划 §5.4 / P5），因此这里只在存在处理中行时
// 做 2 秒轮询；一旦事件通道落地，把 pollWhileProcessing 换成 listen() 订阅即可，其余不变。
const POLL_INTERVAL_MS = 2000;

function mergeRows(jobs: ImportJob[], summaries: LibraryExamSummary[], trashed: LibraryExamSummary[]): LibraryRowV1[] {
  const jobById = new Map(jobs.map((job) => [job.jobId, job]));
  const summaryById = new Map(summaries.map((summary) => [summary.id, summary]));
  const trashedById = new Map(trashed.map((summary) => [summary.id, summary]));

  const rows: LibraryRowV1[] = [];
  const seen = new Set<string>();
  // 活动条目：job 与 summary 的并集，两边都可能单独存在（写作没有 job；刚建的 job 还没有 summary）。
  for (const id of [...jobById.keys(), ...summaryById.keys()]) {
    if (seen.has(id) || trashedById.has(id)) continue;
    seen.add(id);
    rows.push(buildRow(id, jobById.get(id), summaryById.get(id)));
  }
  for (const [id, summary] of trashedById) {
    if (seen.has(id)) continue;
    seen.add(id);
    rows.push(buildRow(id, jobById.get(id), summary, { inTrash: true }));
  }
  return rows.sort((a, b) => (b.updatedAt ?? "").localeCompare(a.updatedAt ?? ""));
}

export interface LibraryStore {
  rows: LibraryRowV1[];
  loading: boolean;
  error?: string;
  refresh: () => void;
  /** 导入刚建好的条目先乐观插入，避免等第一份 PDF 解析完成才出现在列表里（计划 §12.2）。 */
  prependOptimistic: (rows: LibraryRowV1[]) => void;
  moveToTrash: (id: string) => Promise<void>;
  restore: (id: string) => Promise<void>;
}

export function useLibraryStore(): LibraryStore {
  const [rows, setRows] = useState<LibraryRowV1[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | undefined>();
  const [tick, setTick] = useState(0);
  const optimistic = useRef<LibraryRowV1[]>([]);

  const refresh = useCallback(() => setTick((value) => value + 1), []);

  const load = useCallback(async () => {
    const [jobs, summaries, trashed] = await Promise.all([
      listJobs().catch((cause) => {
        console.error("[library] listJobs failed", cause);
        return [] as ImportJob[];
      }),
      listLibraryExams().catch((cause) => {
        console.error("[library] listLibraryExams failed", cause);
        return [] as LibraryExamSummary[];
      }),
      listTrashedExams().catch(() => [] as LibraryExamSummary[])
    ]);
    const merged = mergeRows(jobs, summaries, trashed);
    const known = new Set(merged.map((row) => row.id));
    // 真实数据一到就丢掉同 id 的乐观行。
    optimistic.current = optimistic.current.filter((row) => !known.has(row.id));
    return [...optimistic.current, ...merged];
  }, []);

  useEffect(() => {
    let cancelled = false;
    setError(undefined);
    load()
      .then((next) => {
        if (!cancelled) setRows(next);
      })
      .catch((cause) => {
        if (!cancelled) setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [load, tick]);

  const hasProcessing = useMemo(() => rows.some((row) => isProcessingStage(row.stage)), [rows]);

  useEffect(() => {
    if (!hasProcessing) return;
    const timer = window.setInterval(refresh, POLL_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [hasProcessing, refresh]);

  const prependOptimistic = useCallback((next: LibraryRowV1[]) => {
    optimistic.current = [...next, ...optimistic.current];
    setRows((current) => [...next, ...current]);
  }, []);

  const moveToTrash = useCallback(async (id: string) => {
    await deleteLibraryExam(id);
    refresh();
  }, [refresh]);

  const restore = useCallback(async (id: string) => {
    await restoreLibraryExam(id);
    refresh();
  }, [refresh]);

  return { rows, loading, error, refresh, prependOptimistic, moveToTrash, restore };
}
