import { command } from "./tauriCommands";

export interface PublishItemOutcome {
  itemId: string;
  ok: boolean;
  examId?: string;
  manifestPath?: string;
  assetCount?: number;
  message?: string;
}

export interface PublishBatchOutcome {
  destination: string;
  succeeded: PublishItemOutcome[];
  failed: PublishItemOutcome[];
}

export function describePublishError(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  if (raw.startsWith("publish_check_failed:")) {
    try {
      const check = JSON.parse(raw.slice("publish_check_failed:".length)) as {
        blockers?: Array<{ userMessage?: string }>;
      };
      const first = check.blockers?.find((blocker) => blocker.userMessage)?.userMessage;
      if (first) return first;
    } catch { /* Show the general content error below. */ }
    return "题目还有未完成的内容，请补齐后再次发布。";
  }
  if (raw.includes("authoring_v2_export_blocked")) return "请先补齐题干、选项或答案，再次发布。";
  if (raw.includes("ITEM_DS_NOT_SEEDED")) return "这道题还没有可发布的题稿。";
  if (raw.includes("requires_tauri_runtime")) return "发布到 NAS 需要在桌面应用中运行。";
  if (/permission|denied|readonly|read-only/i.test(raw)) return "目标目录不可写，请检查 NAS 挂载或共享盘权限。";
  return raw;
}

export async function publishItems(
  itemIds: string[], destination: string,
  onProgress?: (done: number, total: number, itemId: string) => void
): Promise<PublishBatchOutcome> {
  if (!itemIds.length) return { destination, succeeded: [], failed: [] };
  onProgress?.(0, itemIds.length, itemIds[0]);
  try {
    const result = await command<PublishBatchOutcome>("publish_items", { input: { itemIds, destination } });
    onProgress?.(itemIds.length, itemIds.length, "");
    return result;
  } catch (error) {
    return { destination, succeeded: [], failed: itemIds.map((itemId) => ({
      itemId, ok: false, message: describePublishError(error)
    })) };
  }
}

export async function publishItem(itemId: string, destination: string): Promise<PublishItemOutcome> {
  const batch = await publishItems([itemId], destination);
  return batch.succeeded[0] ?? batch.failed[0];
}
