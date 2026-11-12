import type { AuthoringPatchV2, IeltsAuthoringIRV2 } from "../types";
import { applyAuthoringV2Patches, orderedTaskSlots } from "../services/authoringV2Patches";
import { userMessageOf } from "../utils/userFacingError";
import { compileStructureAction } from "./structureActions";
import type { ExamCanvasStructureAction } from "./ExamCanvas";

/** 显式重新编号（用户确认后才派发）。scope=task 只动一道大题，exam 动全卷。 */
export type RenumberAction = Extract<ExamCanvasStructureAction, { type: "questions.renumber" }>;

/** 连续区间写成「1–6」，否则逐个列出；与题号范围提示保持一致。 */
export function formatQuestionNumbers(numbers: number[]): string {
  if (!numbers.length) return "无";
  const contiguous = numbers.length > 1 && numbers.every((value, index) => index === 0 || value === numbers[index - 1] + 1);
  return contiguous ? `${numbers[0]}–${numbers[numbers.length - 1]}` : numbers.join("、");
}

/**
 * 预览：按编译时的顺序（大题顺序 → 题内嵌入/列表顺序）列出范围内的原题号与改后题号。
 * 改后题号用当前稿真的应用一次补丁得到，与保存后的结果一致；补丁被拒绝时直接抛出。
 */
export function renumberPreview(current: IeltsAuthoringIRV2, action: RenumberAction): { before: number[]; after: number[] } {
  const patch = compileStructureAction(current, action);
  if (!patch) throw new Error("这次重新编号没有生效，请重试。");
  const tasks = current.taskGroups.filter((task) => action.scope === "exam" || task.taskId === action.taskId);
  const ids = tasks.flatMap(orderedTaskSlots);
  const applied = applyAuthoringV2Patches(current, [patch as AuthoringPatchV2]);
  return {
    before: ids.map((id) => current.answerSlots[id]?.questionNumber ?? 0),
    after: ids.map((id) => applied.answerSlots[id]?.questionNumber ?? 0),
  };
}

/** 二次确认：写明题号范围前后变化，并说明答案与统计不受影响、可撤销。返回 false 表示不执行。 */
export function confirmRenumber(current: IeltsAuthoringIRV2, action: RenumberAction): boolean {
  let preview: { before: number[]; after: number[] };
  try {
    preview = renumberPreview(current, action);
  } catch (error) {
    window.alert(userMessageOf(error, "这次重新编号没有生效，请重试。"));
    return false;
  }
  if (preview.before.join(",") === preview.after.join(",")) {
    window.alert("题号已经是连续的，无需重新编号。");
    return false;
  }
  const scope = action.scope === "exam" ? "全卷" : "本题组";
  return window.confirm(`按顺序重新编号${scope}：\n原题号 ${formatQuestionNumbers(preview.before)} → 新题号 ${formatQuestionNumbers(preview.after)}。\n\n题号会改变，答案与统计不受影响，可撤销（Ctrl+Z）。\n确定重新编号？`);
}
