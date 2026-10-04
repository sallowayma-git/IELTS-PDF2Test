import { useCallback, useEffect, useRef, useState } from "react";
import { subscribeProcessing } from "../../api/processingClient";
import {
  deleteLibraryExam,
  emptyRecycleBin,
  listJobs,
  listLibraryExams,
  listTrashedExams,
  permanentlyDeleteExam,
  restoreLibraryExam,
  setLibraryItemPart,
  type EmptyRecycleBinResult
} from "../../api/tauriCommands";
import { getLibraryRow, listLibraryItems, type LibraryItemSummaryV2 } from "../../api/workspaceClient";
import type { ImportJob, LibraryExamSummary } from "../../types";
import { buildRow, type LibraryRowV1 } from "./libraryTypes";

// 题库行 = 处理任务（ImportJob）与题库条目（LibraryExamSummary）按 id 合并。
// 当前数据模型下 library item id 与 job id 相同（见 findings F12），所以 id 可以直接做合并键。
//
// 处理中的阶段由后端持久队列给出（`list_library_items` 附带 processing 状态），
// 进度靠 `processing://item-updated` 事件驱动刷新；原先的 2 秒轮询已删除。

function mergeRows(
  jobs: ImportJob[],
  summaries: LibraryExamSummary[],
  trashed: LibraryExamSummary[],
  v2Items: LibraryItemSummaryV2[]
): LibraryRowV1[] {
  const jobById = new Map(jobs.map((job) => [job.jobId, job]));
  const summaryById = new Map(summaries.map((summary) => [summary.id, summary]));
  const trashedById = new Map(trashed.map((summary) => [summary.id, summary]));
  const v2ById = new Map(v2Items.map((item) => [item.id, item]));

  const rows: LibraryRowV1[] = [];
  const seen = new Set<string>();
  // 活动条目：job 与 summary 的并集，两边都可能单独存在（写作没有 job；刚建的 job 还没有 summary）。
  for (const id of [...jobById.keys(), ...summaryById.keys(), ...v2ById.keys()]) {
    if (seen.has(id) || trashedById.has(id)) continue;
    seen.add(id);
    rows.push(buildRow(id, jobById.get(id), summaryById.get(id), {}, v2ById.get(id)));
  }
  for (const [id, summary] of trashedById) {
    if (seen.has(id)) continue;
    seen.add(id);
    rows.push(buildRow(id, jobById.get(id), summary, { inTrash: true }, v2ById.get(id)));
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
  /** 永久删除单个回收站条目（不可恢复）。调用方须先二次确认。 */
  permanentlyDelete: (id: string) => Promise<void>;
  /** 清空回收站（不可恢复）。返回删除数与被跳过（仍在处理中）的条目。 */
  emptyTrash: () => Promise<EmptyRecycleBinResult>;
  /** 手动设置 Part 标签；`label` 为 null 表示清除、回到自动判定。 */
  setPart: (id: string, label: string | null) => Promise<void>;
}

export function useLibraryStore(): LibraryStore {
  const [rows, setRows] = useState<LibraryRowV1[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | undefined>();
  const [tick, setTick] = useState(0);
  const optimistic = useRef<LibraryRowV1[]>([]);
  const [subscribed, setSubscribed] = useState(false);
  const active = useRef(false);
  const snapshotPending = useRef(true);
  const snapshotGeneration = useRef(0);
  const dirtyItems = useRef(new Set<string>());
  const refreshingItems = useRef(new Set<string>());

  const refresh = useCallback(() => setTick((value) => value + 1), []);

  const load = useCallback(async () => {
    const [jobs, summaries, trashed, v2Items] = await Promise.all([
      listJobs().catch((cause) => {
        console.error("[library] listJobs failed", cause);
        return [] as ImportJob[];
      }),
      listLibraryExams().catch((cause) => {
        console.error("[library] listLibraryExams failed", cause);
        return [] as LibraryExamSummary[];
      }),
      listTrashedExams().catch(() => [] as LibraryExamSummary[]),
      // M1：V2 仓库行（标题/状态权威）；失败时退回旧三源合并。
      listLibraryItems().catch((cause) => {
        console.error("[library] listLibraryItems failed", cause);
        return [] as LibraryItemSummaryV2[];
      })
    ]);
    const merged = mergeRows(jobs, summaries, trashed, v2Items);
    return merged;
  }, []);

  // A single in-flight read per item. Events arriving during that read request a
  // follow-up; an older response never hides a terminal/cancel event.
  const refreshItem = useCallback(async (itemId: string): Promise<void> => {
    if (refreshingItems.current.has(itemId) || snapshotPending.current || !active.current) return;
    refreshingItems.current.add(itemId);
    try {
      while (dirtyItems.current.has(itemId) && !snapshotPending.current && active.current) {
        dirtyItems.current.delete(itemId);
        const generation = snapshotGeneration.current;
        const data = await getLibraryRow(itemId);
        if (!active.current) return;
        if (snapshotPending.current || generation !== snapshotGeneration.current) {
          dirtyItems.current.add(itemId);
          break;
        }
        if (dirtyItems.current.has(itemId)) continue;
        const next = data ? buildRow(itemId, data.job ?? undefined, data.summary ?? undefined,
          { inTrash: data.inTrash }, data.item ?? undefined) : undefined;
        optimistic.current = optimistic.current.filter((row) => row.id !== itemId);
        setRows((current) => {
          const remaining = current.filter((row) => row.id !== itemId);
          return (next ? [...remaining, next] : remaining)
            .sort((a, b) => (b.updatedAt ?? "").localeCompare(a.updatedAt ?? ""));
        });
      }
    } catch (cause) {
      if (active.current) {
        console.error("[library] single-item refresh failed; refreshing snapshot", cause);
        dirtyItems.current.delete(itemId);
        // A terminal event must not silently leave a stale row when the item read fails.
        refresh();
      }
    } finally {
      refreshingItems.current.delete(itemId);
      if (active.current && !snapshotPending.current && dirtyItems.current.has(itemId)) void refreshItem(itemId);
    }
  }, [refresh]);

  useEffect(() => {
    let stopped = false;
    active.current = true;
    let unlisten: (() => void) | undefined;
    // Subscribe before the initial snapshot. Events during its read are buffered,
    // then re-read individually after the snapshot so old list responses cannot win.
    subscribeProcessing((update) => {
      if (stopped) return;
      dirtyItems.current.add(update.itemId);
      void refreshItem(update.itemId);
    }).then((stop) => {
      if (stopped) stop(); else { unlisten = stop; setSubscribed(true); }
    }).catch((cause) => {
      console.error("[library] processing subscription failed", cause);
      if (!stopped) setSubscribed(true);
    });
    window.addEventListener("focus", refresh);
    return () => { stopped = true; active.current = false; unlisten?.(); window.removeEventListener("focus", refresh); };
  }, [refresh, refreshItem]);

  useEffect(() => {
    if (!subscribed) return;
    let cancelled = false;
    snapshotPending.current = true;
    const generation = ++snapshotGeneration.current;
    setError(undefined);
    load()
      .then((next) => {
        if (cancelled || generation !== snapshotGeneration.current) return;
        const known = new Set(next.map((row) => row.id));
        optimistic.current = optimistic.current.filter((row) => !known.has(row.id));
        setRows([...optimistic.current, ...next]);
      })
      .catch((cause) => {
        if (!cancelled) setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        if (cancelled || generation !== snapshotGeneration.current) return;
        setLoading(false);
        snapshotPending.current = false;
        for (const itemId of dirtyItems.current) void refreshItem(itemId);
      });
    return () => { cancelled = true; };
  }, [load, tick, subscribed, refreshItem]);

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

  const permanentlyDelete = useCallback(async (id: string) => {
    await permanentlyDeleteExam(id);
    refresh();
  }, [refresh]);

  const emptyTrash = useCallback(async () => {
    const result = await emptyRecycleBin();
    refresh();
    return result;
  }, [refresh]);

  const setPart = useCallback(async (id: string, label: string | null) => {
    await setLibraryItemPart(id, label);
    refresh();
  }, [refresh]);

  return { rows, loading, error, refresh, prependOptimistic, moveToTrash, restore, permanentlyDelete, emptyTrash, setPart };
}
