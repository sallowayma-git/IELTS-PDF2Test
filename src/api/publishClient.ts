import { exportAuthoringV2, exportNasPackageV2, getAuthoringV2 } from "./tauriCommands";
import { getWorkspaceItem } from "./workspaceClient";
import type { IeltsAuthoringIRV2 } from "../types";

// 发布客户端（计划 §16.12）：把 NAS V2 发布序列从 ExportPage 里抽出来，
// 让题库批量栏和工作区主按钮共用同一条路径，且不再由 UI 解析错误字符串。
//
// M1（原 P7-T02 提前实施）：优先走 DB 权威稿直通——getWorkspaceItem 取
// library_items_v2 的 canonical DS，export 传 authoring 覆盖，后端用 typed
// preflight（当前稿 + 当前 blocker）替代历史痕迹门禁。条目未迁移/无稿时
// 回退 legacy 会话链。
//
// 现状与目标的差距（诚实记录，M6 收敛）：
// - `nas_package_v2` 目前只支持单条 staging/commit，因此这里的批量是「逐条发布 + 汇总结果」，
//   不是计划 §13.6 要求的整批 staging 后一次提交。任一条失败时前面已提交的条目不会回滚。
const MINIMUM_RUNTIME_VERSION = "0.2.0";

export interface PublishItemOutcome {
  itemId: string;
  ok: boolean;
  examId?: string;
  manifestPath?: string;
  assetCount?: number;
  /** 用户可读的失败原因；内部错误码保留在 console 里。 */
  message?: string;
}

export interface PublishBatchOutcome {
  destination: string;
  succeeded: PublishItemOutcome[];
  failed: PublishItemOutcome[];
}

/** 把后端抛出的门禁字符串折成一句用户可读的话。 */
export function describePublishError(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  if (raw.startsWith("publish_check_failed:")) {
    // typed PublishCheckResultV1：直接展示可定位的 blocker 文案（计划 §13.4）。
    try {
      const check = JSON.parse(raw.slice("publish_check_failed:".length)) as {
        blockers?: Array<{ userMessage?: string }>;
      };
      const first = check.blockers?.find((blocker) => blocker.userMessage)?.userMessage;
      if (first) return first;
    } catch {
      // 解析失败时落到下面的通用文案。
    }
    return "这道题还有未完成的内容需要先补齐，补齐后可以再次发布。";
  }
  if (raw.includes("authoring_v2_export_blocked")) {
    return "这道题还有未完成的内容需要先补齐（题干、选项或答案），补齐后可以再次发布。";
  }
  if (raw.includes("nas_package_v2_requires_single_authoring_job")) {
    return "这道题没有可发布的结构化题稿。";
  }
  if (raw.includes("requires_tauri_runtime") || raw.includes("__TAURI_INTERNALS__")) {
    return "发布到 NAS 需要在桌面应用中运行。";
  }
  if (/permission|denied|readonly|read-only/i.test(raw)) {
    return "目标目录不可写，请检查 NAS 挂载或共享盘权限。";
  }
  return raw;
}

async function publishViaCanonicalDs(
  itemId: string,
  destination: string
): Promise<{ ok: true; examId?: string; manifestPath?: string; assetCount?: number } | { ok: false; unsupported: true }> {
  const workspace = await getWorkspaceItem(itemId);
  if (!workspace.ds) return { ok: false, unsupported: true };
  const materialized = await exportAuthoringV2({
    jobId: itemId,
    exportDir: destination,
    editVersion: workspace.editVersion,
    authoring: workspace.ds as unknown as IeltsAuthoringIRV2
  });
  const published = await exportNasPackageV2({
    libraryRoot: destination,
    sourcePath: materialized.receipt.runtimePath,
    assetRoot: materialized.receipt.outputDir,
    examId: materialized.examId,
    minimumRuntimeVersion: MINIMUM_RUNTIME_VERSION
  });
  return {
    ok: true,
    examId: published.examId,
    manifestPath: published.manifestPath,
    assetCount: published.assetCount
  };
}

export async function publishItem(itemId: string, destination: string): Promise<PublishItemOutcome> {
  try {
    const direct = await publishViaCanonicalDs(itemId, destination).catch((error) => {
      const message = error instanceof Error ? error.message : String(error);
      // 未迁移条目回退 legacy 会话链；真正的门禁/发布错误照常上抛。
      if (message.startsWith("ITEM_NOT_FOUND") || message.startsWith("ITEM_DS_NOT_SEEDED")) {
        return { ok: false, unsupported: true } as const;
      }
      throw error;
    });
    if (!direct.ok && "unsupported" in direct) {
      const session = await getAuthoringV2(itemId);
      // 当前数据模型下 library item id 与 job id 相同（findings F12），所以可以直接透传。
      const materialized = await exportAuthoringV2({ jobId: itemId, exportDir: destination, revision: session.revision });
      const published = await exportNasPackageV2({
        libraryRoot: destination,
        sourcePath: materialized.receipt.runtimePath,
        assetRoot: materialized.receipt.outputDir,
        examId: materialized.examId,
        minimumRuntimeVersion: MINIMUM_RUNTIME_VERSION
      });
      return {
        itemId,
        ok: true,
        examId: published.examId,
        manifestPath: published.manifestPath,
        assetCount: published.assetCount
      };
    }
    if (!direct.ok) return { itemId, ok: false, message: "发布失败。" };
    return {
      itemId,
      ok: true,
      examId: direct.examId,
      manifestPath: direct.manifestPath,
      assetCount: direct.assetCount
    };
  } catch (error) {
    console.error(`[publish] item ${itemId} failed`, error);
    return { itemId, ok: false, message: describePublishError(error) };
  }
}

export async function publishItems(
  itemIds: string[],
  destination: string,
  onProgress?: (done: number, total: number, itemId: string) => void
): Promise<PublishBatchOutcome> {
  const succeeded: PublishItemOutcome[] = [];
  const failed: PublishItemOutcome[] = [];
  for (const [index, itemId] of itemIds.entries()) {
    onProgress?.(index, itemIds.length, itemId);
    const outcome = await publishItem(itemId, destination);
    (outcome.ok ? succeeded : failed).push(outcome);
  }
  onProgress?.(itemIds.length, itemIds.length, "");
  return { destination, succeeded, failed };
}
