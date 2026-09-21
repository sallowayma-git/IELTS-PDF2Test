import { applyAuthoringV2Patches } from "../../services/authoringV2Patches";
import type { AuthoringPatchV2, IeltsAuthoringIRV2 } from "../../types";

export interface ConflictRebaseResult {
  /** 以服务端最新版本为基线、重放本地未保存修改后的题稿。 */
  rebased: IeltsAuthoringIRV2;
  /** 成功重放、仍待保存的补丁。 */
  applied: AuthoringPatchV2[];
  /** 因原文已改动而无法重放的补丁数量。 */
  dropped: number;
}

/**
 * 保存冲突后的出路（计划 §9.10 / findings 冲突死路）。
 *
 * 编辑器遇到 `EDIT_VERSION_CONFLICT` 时，本地未保存的修改仍然有效，只是基线过期。
 * 这里把补丁按原顺序重放到服务端最新版本上：某条补丁因目标节点已被别处改动而
 * 无法应用时**停下**，已应用的部分继续保存，未应用的部分计入 `dropped` 由调用方
 * 明确告知用户——既不静默丢弃本地修改，也不覆盖服务端的新版本。
 */
export function rebasePendingPatches(
  base: IeltsAuthoringIRV2,
  patches: AuthoringPatchV2[]
): ConflictRebaseResult {
  const applied: AuthoringPatchV2[] = [];
  let rebased = base;
  for (const patch of patches) {
    try {
      rebased = applyAuthoringV2Patches(rebased, [patch]);
      applied.push(patch);
    } catch {
      break;
    }
  }
  return { rebased, applied, dropped: patches.length - applied.length };
}

/**
 * 冲突恢复之后必须让用户看到的那条提示；无需提示时返回 `undefined`。
 *
 * 为什么单独抽出来：这条信息**不能**走 `saveMessage`。`saveMessage` 属于保存状态机，
 * 保存循环每次迭代都会把它清空，紧随其后的「已保存」会把丢弃提示覆盖掉，用户就会
 * 以为所有改动都落盘了——那是静默数据丢失。调用方必须把它写进独立的、需要用户
 * 手动关闭的提示状态，因此「什么时候必须有提示」本身就是一条要断言的规则。
 */
export function conflictRecoveryNotice(
  appliedCount: number,
  droppedCount: number
): string | undefined {
  if (droppedCount <= 0) return undefined;
  return `已把前 ${appliedCount} 项修改重新应用到最新版本；从第 ${appliedCount + 1} 项起的 ${droppedCount} 项因原文已被改动而无法应用，没有保存，请手动补回。`;
}

/** 后端编辑日志里的一行（`get_workspace_item.recentEdits`）。 */
export interface RecentEditV1 {
  baseVersion: number;
  /** `human` | `cloud_repair` | `answer_page_recognition` | `undo` | null（来源不明）。 */
  origin?: string | null;
}

/**
 * 本地基线之后的所有写入是否**全是机器写入**。
 *
 * 只有这种情况才自动重放：云端修复 / 答案页识别是后台在写，用户没有做任何需要他
 * 二选一的事。只要夹着一次人工写入（另一个窗口）、来源不明、或日志不完整（条数对不上
 * 版本差），就不猜，交回给用户。
 */
export function conflictWasMachineOnly(input: {
  localBase: number;
  remoteVersion: number;
  recentEdits: RecentEditV1[] | undefined;
}): boolean {
  const { localBase, remoteVersion, recentEdits } = input;
  if (!Array.isArray(recentEdits) || remoteVersion <= localBase) return false;
  const since = recentEdits.filter((edit) => edit.baseVersion >= localBase && edit.baseVersion < remoteVersion);
  if (since.length !== remoteVersion - localBase) return false;
  return since.every((edit) => typeof edit.origin === "string" && edit.origin !== "human" && edit.origin !== "undo");
}

export interface LatestWorkspaceV1 {
  ds: IeltsAuthoringIRV2;
  editVersion: number;
  recentEdits?: RecentEditV1[];
}

export type AutoRebaseOutcome =
  | { kind: "rebased"; latest: LatestWorkspaceV1; rebase: ConflictRebaseResult }
  | { kind: "manual" };

/**
 * 保存冲突的**自动**出路：冲突只由机器写入造成时，读最新版本、把本地修改重放上去。
 *
 * 重放失败的补丁**丢弃而不强写**（`rebasePendingPatches` 的语义），由调用方如实提示；
 * 读取失败或冲突里有人工写入时返回 `manual`，界面才出现「重试保存 / 放弃本地修改」。
 */
export async function tryAutoRebase(input: {
  localBase: number;
  outstanding: AuthoringPatchV2[];
  fetchLatest: () => Promise<LatestWorkspaceV1>;
}): Promise<AutoRebaseOutcome> {
  let latest: LatestWorkspaceV1;
  try {
    latest = await input.fetchLatest();
  } catch {
    return { kind: "manual" };
  }
  if (!latest?.ds) return { kind: "manual" };
  if (!conflictWasMachineOnly({ localBase: input.localBase, remoteVersion: latest.editVersion, recentEdits: latest.recentEdits })) {
    return { kind: "manual" };
  }
  return { kind: "rebased", latest, rebase: rebasePendingPatches(latest.ds, input.outstanding) };
}
