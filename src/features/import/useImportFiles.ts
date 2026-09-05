import { useCallback, useRef, useState } from "react";
import { createImportJob, importSourceFile, listLlmProfiles, runAutoPipeline, runCloudReview } from "../../api/tauriCommands";
import type { PickedPath } from "../../api/desktopDialogs";
import { buildRow, type LibraryRowV1 } from "../library/libraryTypes";
import { readAppSettings } from "../settings/appSettings";

// 两阶段导入（计划 §12.2）。
//
// 阶段 A（快）：createImportJob + importSourceFile。只做元数据与文件落地，N 份文件全部建行后立刻
//   关闭抽屉，用户马上在题库看到 N 行 —— 不等第一份 PDF 解析完成。
// 阶段 B（慢，后台）：runAutoPipeline 本地识别，随后可选 runCloudReview，按并发上限调度。
//
// 注意：阶段 B 目前仍在页面进程内调度。这不是计划 §5 要求的 Rust/SQLite durable queue，
// 应用退出会中断未完成的任务。但相比 `UnifiedPreview` 的 localStorage queue + lease +
// window.__IELTS_CLOUD_REVIEW_WORKER__ 全局，这里没有任何跨页全局状态，P5 迁到 Rust 时
// 只需把 runStage B 换成 enqueue 调用。
const MAX_IMPORT_FILE_BYTES = 128 * 1024 * 1024;
const LOCAL_PLACEHOLDER_PROFILE = "profile-local-placeholder";

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(bytes >= 10 * 1024 * 1024 ? 0 : 1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

/** 把后端的机器错误码换成用户可读文案；未知错误原样透出，不吞掉。 */
export function formatImportError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  const tooLarge = message.match(/^source_file_too_large:max_bytes=(\d+):size_bytes=(\d+):path=(.+)$/);
  if (tooLarge) {
    const [, maxBytes, sizeBytes, filePath] = tooLarge;
    const name = filePath.split(/[\/]/).filter(Boolean).pop() ?? filePath;
    return `“${name}”超过导入上限（${formatBytes(Number(sizeBytes))}，上限 ${formatBytes(Number(maxBytes))}）。请拆分或压缩后重试。`;
  }
  return message;
}

export interface ImportRejection {
  name: string;
  reason: string;
}

/** 阶段 A 的结果：哪些文件建行成功、哪些当场被拒。 */
export interface ImportBatchResult {
  rows: LibraryRowV1[];
  rejected: ImportRejection[];
}

function preflight(files: PickedPath[]): { accepted: PickedPath[]; rejected: ImportRejection[] } {
  const accepted: PickedPath[] = [];
  const rejected: ImportRejection[] = [];
  for (const file of files) {
    if (Number(file.sizeBytes ?? 0) > MAX_IMPORT_FILE_BYTES) {
      rejected.push({ name: file.name, reason: `超过导入上限 ${formatBytes(MAX_IMPORT_FILE_BYTES)}` });
      continue;
    }
    if (file.requiresDesktopParser) {
      rejected.push({ name: file.name, reason: "无法读取真实内容，请改用 PDF/DOCX/TXT/MD" });
      continue;
    }
    accepted.push(file);
  }
  return { accepted, rejected };
}

/** 固定并发的任务池；单个任务失败不影响其它任务（计划 §12.2 末段）。 */
async function runPool<T>(items: T[], limit: number, worker: (item: T) => Promise<void>): Promise<void> {
  let cursor = 0;
  const runners = Array.from({ length: Math.max(1, Math.min(limit, items.length)) }, async () => {
    while (cursor < items.length) {
      const item = items[cursor++];
      try {
        await worker(item);
      } catch (error) {
        console.error("[import] background stage failed", error);
      }
    }
  });
  await Promise.all(runners);
}

async function resolveCloudProfileId(): Promise<string | undefined> {
  try {
    const profiles = await listLlmProfiles();
    return profiles.find((profile) => profile.enabled && profile.profileId !== LOCAL_PLACEHOLDER_PROFILE)?.profileId;
  } catch (error) {
    console.error("[import] listLlmProfiles failed; continuing local-only", error);
    return undefined;
  }
}

export interface UseImportFiles {
  busy: boolean;
  /** 阶段 A 的进度文案，例如「正在建立 3/12」。 */
  stageMessage?: string;
  error?: string;
  /** 后台仍在识别的条目数，供题库页显示一个整体提示。 */
  backgroundCount: number;
  importFiles: (files: PickedPath[], options?: { cloudEnabled?: boolean }) => Promise<ImportBatchResult>;
  clearError: () => void;
}

export function useImportFiles(onRowsChanged: () => void): UseImportFiles {
  const [busy, setBusy] = useState(false);
  const [stageMessage, setStageMessage] = useState<string | undefined>();
  const [error, setError] = useState<string | undefined>();
  const [backgroundCount, setBackgroundCount] = useState(0);
  const changed = useRef(onRowsChanged);
  changed.current = onRowsChanged;

  const importFiles = useCallback(async (files: PickedPath[], options: { cloudEnabled?: boolean } = {}) => {
    const empty: ImportBatchResult = { rows: [], rejected: [] };
    if (!files.length) return empty;
    setBusy(true);
    setError(undefined);
    const { accepted, rejected } = preflight(files);
    const created: { jobId: string; file: PickedPath }[] = [];
    const rows: LibraryRowV1[] = [];
    try {
      // ---- 阶段 A：建行。逐个文件失败只影响该文件。 ----
      for (const [index, file] of accepted.entries()) {
        setStageMessage(`正在建立 ${index + 1}/${accepted.length}：${file.name}`);
        try {
          const job = await createImportJob({ title: file.titleHint || file.name.replace(/\.[^.]+$/, "") });
          await importSourceFile(job.jobId, file.path, "MainQuestion", file.sizeBytes, file.textContent, file.binaryContentBase64);
          created.push({ jobId: job.jobId, file });
          rows.push(buildRow(job.jobId, job, undefined));
        } catch (caught) {
          rejected.push({ name: file.name, reason: formatImportError(caught) });
        }
      }
    } finally {
      setBusy(false);
      setStageMessage(undefined);
    }

    if (!created.length) {
      if (rejected.length) setError(`${rejected.length} 份文件未能导入，详见列表。`);
      return { rows, rejected };
    }

    // ---- 阶段 B：后台识别。不 await，让调用方立刻关闭抽屉。 ----
    const cloudProfileId = options.cloudEnabled === false ? undefined : await resolveCloudProfileId();
    setBackgroundCount((current) => current + created.length);
    void (async () => {
      const cloudEligible: { jobId: string; file: PickedPath }[] = [];
      const { localConcurrency, cloudConcurrency } = readAppSettings();
      await runPool(created, localConcurrency, async ({ jobId, file }) => {
        await runAutoPipeline(jobId, { confidenceThreshold: 0.85, executionMode: "localOnly", target: "editableDraft" });
        // 本地稿一落地就刷新，用户可以立刻打开这一题（计划 §2.1「已有本地结果时先显示本地草稿」）。
        changed.current();
        if (cloudProfileId && /\.pdf$/i.test(file.name)) cloudEligible.push({ jobId, file });
      });
      if (cloudProfileId && cloudEligible.length) {
        await runPool(cloudEligible, cloudConcurrency, async ({ jobId }) => {
          await runCloudReview(jobId, { profileId: cloudProfileId });
          changed.current();
        });
      }
      setBackgroundCount((current) => Math.max(0, current - created.length));
      changed.current();
    })();

    if (rejected.length) setError(`${rejected.length} 份文件未能导入，其余已开始识别。`);
    return { rows, rejected };
  }, []);

  return { busy, stageMessage, error, backgroundCount, importFiles, clearError: () => setError(undefined) };
}
