import { command } from "./tauriCommands";
import type { PickedPath } from "./desktopDialogs";

export interface ProcessingState {
  stage: string;
  localStatus: string;
  cloudStatus: string;
  actionableCount: number;
  lastErrorCode?: string;
  eventSeq: number;
}

/**
 * `processing://item-updated` 的载荷。
 *
 * `editVersion` 是**权威稿当前版本号**，由后端在发事件时读取，不是「本次事件改了它」。
 * 它存在是为了让消费方能区分两种情况：
 *   - `editVersion === 本地已知版本` → 这是我自己刚保存引起的回声，**不要**重拉，
 *     重拉会把自己正在编辑的内容覆盖掉；
 *   - `editVersion > 本地已知版本` → 权威稿真的被别人/云端改了（云端自主修复就是
 *     这种情况），工作区必须刷新。
 * 只按 `stateVersion` 判断做不到这件事：序号只说明「有事件」，不说明「稿变了」。
 *
 * 后端读不到版本号时发 `null`（条目被删等），消费方按「版本未知」保守处理。
 */
export interface ProcessingItemUpdate {
  itemId: string;
  stateVersion: number;
  editVersion?: number;
}

export type ImportModality = "reading" | "listening";
export function importFiles(input: { files: PickedPath[]; cloudEnabled: boolean; cloudProfileId?: string; modality?: ImportModality }) {
  return command<{ created: Array<{ itemId: string; title: string }>; rejected: Array<{ name: string; reason: string }> }>("import_files", { input });
}
/** 后端 `retry_processing` 的返回：`queued = false` 表示任务正在跑 / 已在排队，这次没有新入队。 */
export function retryQueued(result: { queued?: boolean } | null | undefined): boolean {
  return result?.queued === true;
}

/** 重新识别的回执文案：没入队就如实说，绝不一律「已加入识别队列」。 */
export function describeRetryOutcome(queued: boolean): string {
  return queued
    ? "已加入识别队列，会按当前的云端设置重新识别。题稿仍可编辑。"
    : "这道题正在识别中，没有重复加入队列；完成后结果会自动更新。";
}

/** 重新识别。返回是否**真的**加入了队列。云端设置由后端按此刻的模型连接重新解析。 */
export async function retryProcessing(itemId: string): Promise<boolean> {
  return retryQueued(await command<{ queued?: boolean } | null>("retry_processing", { itemId }));
}
/** 只重跑答案页识别（不重新入队整条流水线）。服务暂时不可用时后端会自动再试一次。 */
export function retryAnswerPageRecognition(itemId: string) {
  return command<{ state?: string; stateReason?: string; answerCount?: number; appliedCount?: number }>("retry_answer_page_recognition", { itemId });
}
export function cancelProcessing(itemId: string) { return command<void>("cancel_processing", { itemId }); }

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/** 把原始载荷收窄成消费方要的形状。字段缺失/类型不对时**不猜**：如实返回 undefined。 */
function toItemUpdate(payload: unknown): ProcessingItemUpdate | undefined {
  if (!isRecord(payload)) return undefined;
  const itemId = payload.libraryItemId;
  const stateVersion = payload.stateVersion;
  if (typeof itemId !== "string" || itemId.length === 0) return undefined;
  if (typeof stateVersion !== "number" || !Number.isFinite(stateVersion)) return undefined;
  const editVersion = payload.editVersion;
  return {
    itemId,
    stateVersion,
    editVersion: typeof editVersion === "number" && Number.isFinite(editVersion) ? editVersion : undefined
  };
}

/**
 * 去重 + 收窄：放行则返回事件并**就地更新** `versions`，否则返回 `undefined`。
 *
 * 抽成纯函数是为了能单独断言「哪些事件会被丢掉」——这条规则一旦写错，症状是
 * 「内容改了但面板不刷新」，而那种缺陷在界面上几乎不可观察（看起来只是慢）。
 *
 * 去重规则：同一 `itemId` 只放行**严格大于**已见 `stateVersion` 的事件。后端在每次
 * 阶段推进、以及每次内容提交落盘后都会推高该序号（`queue::bump_event_seq`），所以
 * 「内容改了」这类通知不会被丢掉——这是后端的责任，在这里放宽去重只会让同一事件被
 * 重复消费、刷新次数失控。
 *
 * `editVersion` 缺失**不**导致事件被丢：它只是让消费方走「版本未知」的保守分支。
 * 拿不到一个附加字段就整条丢弃，等于把一次真实的阶段推进也一起扔掉。
 */
export function acceptProcessingUpdate(
  versions: Map<string, number>,
  payload: unknown
): ProcessingItemUpdate | undefined {
  const update = toItemUpdate(payload);
  if (!update) return undefined;
  if ((versions.get(update.itemId) ?? -1) >= update.stateVersion) return undefined;
  versions.set(update.itemId, update.stateVersion);
  return update;
}

export async function subscribeProcessing(
  onUpdate: (update: ProcessingItemUpdate) => void
): Promise<() => void> {
  if (!("__TAURI_INTERNALS__" in window)) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  const versions = new Map<string, number>();
  return listen<unknown>("processing://item-updated", ({ payload }) => {
    const update = acceptProcessingUpdate(versions, payload);
    if (update) onUpdate(update);
  });
}
