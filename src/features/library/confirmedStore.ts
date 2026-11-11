import { useSyncExternalStore } from "react";
import {
  importLocalConfirmedIds,
  setLibraryItemsConfirmed,
  type LibraryItemConfirmResult,
} from "../../api/workspaceClient";
import { readAppSettings, writeAppSettings } from "../settings/appSettings";
import { isRowConfirmed, type LibraryRowV1 } from "./libraryTypes";

// 用户确认入库（确认状态的唯一入口）。
//
// 识别完成的题不会自动进入题库：用户在「导入」页检查无误后点「确认入库」，
// 题才进入题库；题库里可以「退回待确认」。是否能发布仍由发布门禁决定，与入库无关。
//
// 持久化：
// 1. 真实应用：后端 `set_library_item(s)_confirmed` 写 library_items_v2.confirmed_at，行上的 confirmedAt 是唯一权威。
// 2. 命令缺失（仅浏览器预览 / dev fallback）时退回 localStorage 的 `confirmedLibraryItemIds`。
//    其他错误一律抛给调用方，不静默吞掉。
// 3. 旧版本留在 localStorage 的确认集合，应用启动时经 `import_local_confirmed_ids` 迁移进后端，成功后清空。

/** 判断一个错误是否表示「后端没有这个命令 / 当前不是 Tauri 运行时」。 */
export function isConfirmCommandUnavailable(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error ?? "");
  return /requires_tauri_runtime|command .*not found|not found.*command|unknown command|command not allowed|not allowed.*command|not registered|no such command/i.test(message);
}

/** 单条确认失败的原因（机器码 → 中文）。 */
export function describeConfirmRejection(code: string): string {
  const reason = code.match(/^LIBRARY_ITEM_NOT_CONFIRMABLE:(.+)$/)?.[1];
  if (reason) {
    const reasons: Record<string, string> = {
      processing: "还在识别或排队中，识别完成后再确认",
      failed: "识别失败，请先处理失败原因",
      canonical_missing: "题稿还没有生成，请先完成识别",
      deleted: "已在回收站里",
      not_found: "题目已不存在",
      current_paper_invalid: "所在试卷的当前版本异常，无法确认",
    };
    return reasons[reason] ?? "当前状态不能确认入库";
  }
  if (code.startsWith("LIBRARY_ITEM_IN_USE")) return "已发布或正被 NAS 当前试卷使用，不能退回待确认";
  return "操作没有完成，请稍后重试";
}

/** 一批确认的结果：成功的 id 与逐条失败原因（已转成中文）。 */
export interface ConfirmOutcome {
  confirmed: string[];
  failed: Array<{ itemId: string; reason: string }>;
}

/** 汇总一批失败的人话提示；`titleOf` 把 id 换成题目名（取不到时用 id）。 */
export function summarizeConfirmFailures(outcome: ConfirmOutcome, titleOf: (id: string) => string): string {
  return outcome.failed.map((item) => `「${titleOf(item.itemId)}」${item.reason}`).join("；");
}

function readLocalIds(): string[] {
  try {
    return readAppSettings().confirmedLibraryItemIds ?? [];
  } catch {
    return [];
  }
}

function writeLocalIds(ids: string[]): void {
  try {
    writeAppSettings({ confirmedLibraryItemIds: [...new Set(ids)] });
  } catch (cause) {
    // 本地存储不可用只影响持久化；调用方仍会看到结果。
    console.error("[confirmed] local store write failed", cause);
  }
  notify();
}

// 订阅：本地集合变化时通知界面（后端路径由调用方 refresh 库存储）。
const listeners = new Set<() => void>();
let snapshot: ReadonlySet<string> = new Set();
let snapshotKey: string | undefined;

function notify(): void {
  for (const listener of listeners) listener();
}

/** 每次按本地存储内容取快照；内容不变时复用同一个 Set（useSyncExternalStore 要求稳定引用）。 */
function currentSnapshot(): ReadonlySet<string> {
  const ids = readLocalIds();
  const key = ids.join("\n");
  if (key !== snapshotKey) {
    snapshotKey = key;
    snapshot = new Set(ids);
  }
  return snapshot;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** 本地兜底集合（只读快照）。真实应用中迁移后应为空。 */
export function readLocalConfirmedIds(): ReadonlySet<string> {
  return currentSnapshot();
}

/** 订阅确认集合的变化，供导入页、题库页与角标重算。 */
export function useConfirmedIds(): ReadonlySet<string> {
  return useSyncExternalStore(subscribe, currentSnapshot, currentSnapshot);
}

/** 行是否已确认入库（行上的后端 confirmedAt 为准，见 isRowConfirmed）。 */
export function isConfirmed(row: LibraryRowV1): boolean {
  return isRowConfirmed(row, currentSnapshot());
}

/** 确认入库（批量）。逐条业务拒绝进入 `failed`；命令缺失时整批写本地兜底。 */
export async function confirmItems(ids: string[]): Promise<ConfirmOutcome> {
  return setConfirmed(ids, true);
}

/** 退回待确认（批量入口同样适用；题库页单条调用）。 */
export async function unconfirmItem(id: string): Promise<void> {
  const outcome = await setConfirmed([id], false);
  if (outcome.failed.length) throw new Error(outcome.failed[0].reason);
}

async function setConfirmed(ids: string[], confirmed: boolean): Promise<ConfirmOutcome> {
  const unique = [...new Set(ids)];
  if (unique.length === 0) return { confirmed: [], failed: [] };
  let results: LibraryItemConfirmResult[];
  try {
    results = await setLibraryItemsConfirmed(unique, confirmed);
  } catch (cause) {
    if (!isConfirmCommandUnavailable(cause)) throw cause;
    // 后端命令不可用：本地兜底。
    const current = new Set(readLocalIds());
    for (const itemId of unique) {
      if (confirmed) current.add(itemId);
      else current.delete(itemId);
    }
    writeLocalIds([...current]);
    return { confirmed: unique, failed: [] };
  }
  const outcome: ConfirmOutcome = { confirmed: [], failed: [] };
  for (const result of results) {
    if (result.ok) {
      outcome.confirmed.push(result.itemId);
      // 后端已接管：旧的本地兜底记录若还在，一并清掉（否则行可能因旧记录仍显示为已确认）。
      if (readLocalIds().includes(result.itemId)) writeLocalIds(readLocalIds().filter((id) => id !== result.itemId));
    } else {
      outcome.failed.push({ itemId: result.itemId, reason: describeConfirmRejection(result.error ?? "") });
    }
  }
  return outcome;
}

let localMigration: Promise<boolean> | undefined;

/**
 * 启动时把旧版本留在 localStorage 的确认集合迁移进后端。整个应用会话只执行一次。
 * 成功（后端接受）后清空本地集合并返回 true；失败保留本地集合并返回 false，下次启动再试，不抛错、不阻塞界面。
 */
export function migrateLocalConfirmedOnce(): Promise<boolean> {
  localMigration ??= (async () => {
    const ids = readLocalIds();
    if (ids.length === 0) return false;
    try {
      await importLocalConfirmedIds(ids);
      writeLocalIds([]);
      return true;
    } catch (cause) {
      // 浏览器预览没有该命令：保留本地集合，它仍是唯一的确认记录。
      if (!isConfirmCommandUnavailable(cause)) console.error("[confirmed] local migration failed; will retry next launch", cause);
      return false;
    }
  })();
  return localMigration;
}

/** 仅供测试：重置一次性迁移的缓存。 */
export function resetLocalMigrationForTest(): void {
  localMigration = undefined;
}
