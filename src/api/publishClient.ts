import { command } from "./tauriCommands";
import { toUserFacingError } from "../utils/userFacingError";

export interface PublishItemOutcome {
  itemId: string;
  ok: boolean;
  examId?: string;
  manifestPath?: string;
  assetCount?: number;
  message?: string;
  /** 后端记录：这次发布时门禁结论不是 Ready、由用户点击发布显式放行。只用于审计/测试，不驱动界面分支。 */
  forced?: boolean;
  /** 学生端能否打开这道题（未解析答案或编译不过时为 false，只写了授权快照）。 */
  studentLoadable?: boolean;
  /**
   * 包检查（组装 + 学生加载器探针）失败的原因码。只有放行发布时这一条装不进学生包
   * 才会有——整批照常发布，这一条降级为 authoring-only。原始机器码只供审计，不渲染。
   */
  packageError?: string;
  publishRecordId?: string;
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

export const PUBLISHED_NOTICE = "已发布";
export const PUBLISHED_NOT_LOADABLE_NOTICE = "已发布，但学生端暂时无法打开这道题";

/**
 * 单题发布提示。产品决策：用户点了「发布」就是确认，不向用户展示门禁阈值或阻断清单；
 * 严格发布与学生端可加载的放行发布说的是同一句「已发布」。放行与否以后端发布记录为准，
 * 这里刻意**不**用「发布完成」——验收脚本把那四个字当作干净通过。
 */
export function describePublishOutcome(outcome: PublishItemOutcome): string {
  if (!outcome.ok) return outcome.message ?? "发布失败。";
  return outcome.studentLoadable === false ? PUBLISHED_NOT_LOADABLE_NOTICE : PUBLISHED_NOTICE;
}

/**
 * 发布结果的**机器可读**分类，只挂在 DOM 的 `data-publish-outcome` 上供验收脚本读取，
 * 不渲染成文字（产品决策：不向用户展示放行与否）。脚本据此区分干净发布与放行发布，
 * 而不是去匹配提示文案。
 */
export type PublishOutcomeKind =
  | "published"
  | "published_forced"
  | "published_not_loadable"
  | "published_forced_not_loadable"
  | "failed";

/**
 * 与后端 `ItemPublication::item_status()` **同构**：同时说出「门禁是否被放行」与
 * 「学生端能不能打开」。`studentLoadable === false` 时永远不是 `published`——
 * 门禁 Ready、只是包检查没过的条目走 `published_not_loadable`（没有任何东西被放行），
 * 别把它叫成 `published_forced_not_loadable` 而掩盖「用户根本没点放行」这件事。
 */
export function publishOutcomeKind(outcome: PublishItemOutcome): PublishOutcomeKind {
  if (!outcome.ok) return "failed";
  if (outcome.studentLoadable === false) {
    return outcome.forced ? "published_forced_not_loadable" : "published_not_loadable";
  }
  return outcome.forced ? "published_forced" : "published";
}

export function describeBatchPublishOutcome(outcome: PublishBatchOutcome): string {
  const parts = [`已发布 ${outcome.succeeded.length} 题`];
  const notLoadable = outcome.succeeded.filter((item) => item.studentLoadable === false).length;
  if (notLoadable) parts.push(`其中 ${notLoadable} 题学生端暂时无法打开`);
  if (outcome.failed.length) parts.push(`${outcome.failed.length} 题未发布`);
  return parts.join(" · ");
}

/**
 * 一次点击发布：请求里总是带着放行确认（`confirmedAt` = 点击时间）。
 * 后端照常计算门禁；只有结论确实不是 Ready 时这份确认才被使用并记录为放行。
 */
export async function publishItems(
  itemIds: string[], destination: string,
  onProgress?: (done: number, total: number, itemId: string) => void
): Promise<PublishBatchOutcome> {
  if (!itemIds.length) return { destination, succeeded: [], failed: [] };
  onProgress?.(0, itemIds.length, itemIds[0]);
  try {
    const force = { confirmedAt: new Date().toISOString(), acknowledgedReasons: [] as string[] };
    const result = await command<PublishBatchOutcome>("publish_items", { input: { itemIds, destination, force } });
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
