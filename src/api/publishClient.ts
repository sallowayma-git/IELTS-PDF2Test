import { command } from "./tauriCommands";
import { toUserFacingError } from "../utils/userFacingError";

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

/** 发布错误一律经用户文案层收敛；原始机器码不再直接渲染（audit A7-F04）。 */
export function describePublishError(error: unknown): string {
  return toUserFacingError(error, "发布失败，请稍后重试。").userMessage;
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
