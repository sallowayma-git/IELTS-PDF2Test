import { useSyncExternalStore } from "react";
import { setLibraryItemConfirmed } from "../../api/workspaceClient";
import { readAppSettings, writeAppSettings } from "../settings/appSettings";
import { isRowConfirmed, type LibraryRowV1 } from "./libraryTypes";

// 用户确认入库（确认状态的唯一入口）。
//
// 识别完成的题不会自动进入题库：用户在「导入」页检查无误后点「确认入库」，
// 题才进入题库；题库里可以「退回待确认」。是否能发布仍由发布门禁决定，与入库无关。
//
// 持久化分两层：
// 1. 优先调用后端命令 `set_library_item_confirmed`（写 library_items_v2.confirmed_at）。
// 2. 后端还没有该命令（命令未注册 / 仅浏览器预览）时，退回本地存储的
//    `confirmedLibraryItemIds`（appSettings，localStorage）。其他错误一律抛给调用方，不静默吞掉。
// 行上的确认状态 = 后端 confirmedAt 非空 或 本地集合包含该 id（见 isRowConfirmed）。

/** 判断一个错误是否表示「后端没有这个命令 / 当前不是 Tauri 运行时」。 */
export function isConfirmCommandUnavailable(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error ?? "");
  return /requires_tauri_runtime|command .*not found|not found.*command|unknown command|command not allowed|not allowed.*command|not registered|no such command/i.test(message);
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

/** 本地兜底集合（只读快照）。 */
export function readLocalConfirmedIds(): ReadonlySet<string> {
  return currentSnapshot();
}

/** 订阅确认集合的变化，供导入页、题库页与角标重算。 */
export function useConfirmedIds(): ReadonlySet<string> {
  return useSyncExternalStore(subscribe, currentSnapshot, currentSnapshot);
}

/** 行是否已确认入库（后端 confirmedAt 或本地兜底集合）。 */
export function isConfirmed(row: LibraryRowV1): boolean {
  return isRowConfirmed(row, currentSnapshot());
}

/** 确认入库。后端不可用时写入本地兜底；其他错误抛出（已成功的 id 保持已写入）。 */
export async function confirmItems(ids: string[]): Promise<void> {
  await setConfirmed(ids, true);
}

/** 退回待确认。 */
export async function unconfirmItem(id: string): Promise<void> {
  await setConfirmed([id], false);
}

async function setConfirmed(ids: string[], confirmed: boolean): Promise<void> {
  let localFallback = false;
  for (const itemId of ids) {
    if (!localFallback) {
      try {
        await setLibraryItemConfirmed(itemId, confirmed);
        // 后端已接管后，旧的本地兜底记录若还在，退回时必须一并清掉，否则行会因并集而仍显示为已确认。
        if (!confirmed && readLocalIds().includes(itemId)) writeLocalIds(readLocalIds().filter((id) => id !== itemId));
        continue;
      } catch (cause) {
        if (!isConfirmCommandUnavailable(cause)) throw cause;
        localFallback = true;
      }
    }
    // 后端命令不可用：本地兜底。
    const current = new Set(readLocalIds());
    if (confirmed) current.add(itemId);
    else current.delete(itemId);
    writeLocalIds([...current]);
  }
}
