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
