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
  /** 仅组卷发布（带 `paper`）时返回：当前试卷指针是否已更新。 */
  paper?: PaperPublishResult;
  /**
   * 本次发布的顶层结论。`published`：清单与当前试卷都已更新；`published_with_warnings`：
   * 已更新，但提交后有记录未完整写入（见 `warnings`）；`paper_pointer_stale`：清单已更新，
   * 但当前试卷指针没写成，学生端仍是旧试卷；`paper_incomplete`：本次什么都没提交。
   */
  status?: "published" | "published_with_warnings" | "paper_pointer_stale" | "paper_incomplete";
  /** 提交之后发生、但没有回滚包的问题（如发布记录写入失败、状态漂移）。 */
  warnings?: unknown[];
  batchId?: string;
}

/** 组卷请求（与后端 `PaperSpec` 同形，字段名不可改）。阅读三篇必填，听力可为空。 */
export interface PaperSpec {
  reading: { P1: string; P2: string; P3: string };
  listening: Array<{ part: string; itemId: string }>;
}

/** 组卷结果。失败时当前试卷保持不变；清单里已发布的题照常保留。 */
export type PaperPublishResult =
  | { ok: true; paperId: string; publishedAt: string; batchId: string; reading: unknown[]; listening: unknown[] }
  | { ok: false; code: "PAPER_INCOMPLETE"; missing: string[] }
  | { ok: false; code: "PAPER_WRITE_FAILED"; message?: string };

/** 清单已更新、但指针没写成：学生端仍是旧试卷。文案与产品约定一致，不得改成「发布成功」。 */
export const PAPER_POINTER_STALE_NOTICE = "清单已更新，但学生端仍是旧试卷，请重试发布。";
/** 另一次发布占着 NAS 上的发布锁：不得显示成「当前试卷没有更新」，提示后由调用方刷新当前试卷。 */
export const PUBLISH_LOCK_BUSY_NOTICE = "另一次发布正在进行，请稍后查看当前试卷。";

/**
 * 组卷发布的结果文案。`titleOf` 把缺失的题目 id 译成名称，用户看到的是题名而不是 id。
 * 组卷结果缺失（后端没返回 paper）时只说明当前试卷没有更新。
 */
export function describePaperPublishOutcome(outcome: PublishBatchOutcome, titleOf: (itemId: string) => string): string {
  const paper = outcome.paper;
  if (!paper) {
    return outcome.status === "paper_pointer_stale" ? PAPER_POINTER_STALE_NOTICE : "当前试卷没有更新。";
  }
  if (paper.ok) {
    const id = paper.paperId.slice(0, 8);
    return outcome.warnings?.length
      ? `当前试卷已发布（${id}），但有 ${outcome.warnings.length} 项记录未完整写入，请在 NAS 发布记录中核对。`
      : `当前试卷已发布（${id}）`;
  }
  if (paper.code === "PAPER_INCOMPLETE") {
    const names = paper.missing.map(titleOf).join("、");
    return `试卷中有题目未能发布，当前试卷保持不变：${names}`;
  }
  return PAPER_POINTER_STALE_NOTICE;
}

/** 发布错误一律经用户文案层收敛；原始机器码不再直接渲染（audit A7-F04）。 */
export function describePublishError(error: unknown): string {
  if (String(error).includes("PUBLISH_REQUIRES_PAPER")) return "单题发布已取消，请到题库的组卷栏选三篇阅读后发布为当前试卷。";
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
 * 一次点击发布（P2 门禁收敛）。
 *
 * 前端**不再**永远带 `force`：发布判定收敛为「编译通过 = 可发布」，其余（答案未填、
 * 低置信、保真度低、未人工确认）是编辑器与发布对话框里展示的警告。放行确认
 * （`force`）随伪装一起去掉——它此前的唯一效果是让每次发布都被后端记成「用户放行」，
 * 门禁结论被悄悄覆盖，审计里分不清用户到底确认过什么。
 */
/**
 * 组卷发布（唯一的发布入口，契约 V1.1 第5条：单题发布已取消，后端不带 paper 会拒绝）。
 *
 * 前端**不**带 `force`：发布判定是「编译通过 = 可发布」，其余是编辑器里的警告。
 */
export async function publishItems(
  itemIds: string[], destination: string,
  onProgress?: (done: number, total: number, itemId: string) => void,
  options?: { paper?: PaperSpec }
): Promise<PublishBatchOutcome> {
  if (!itemIds.length) return { destination, succeeded: [], failed: [] };
  onProgress?.(0, itemIds.length, itemIds[0]);
  // 只有组卷时才带 `paper` 字段：请求体只含后端认识的字段（后端 deny_unknown_fields）。
  const input: Record<string, unknown> = { itemIds, destination };
  if (options?.paper) input.paper = options.paper;
  try {
    const result = await command<PublishBatchOutcome>("publish_items", { input });
    onProgress?.(itemIds.length, itemIds.length, "");
    return result;
  } catch (error) {
    const message = String(error).includes("nas_package_v2_lock_busy")
      ? PUBLISH_LOCK_BUSY_NOTICE
      : describePublishError(error);
    return { destination, succeeded: [], failed: itemIds.map((itemId) => ({
      itemId, ok: false, message
    })) };
  }
}
