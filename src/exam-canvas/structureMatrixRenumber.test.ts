// 矩阵追加（单题作答组）与显式重新编号的编译、预览与撤销（证据层级：单元，TS 镜像补丁器）。
import { describe, expect, it } from "vitest";
import { applyAuthoringV2Patches, inverseAuthoringPatch } from "../services/authoringV2Patches";
import { buildRuntimeViewModelV2 } from "../services/runtimeViewModelV2";
import { compileStructureAction } from "./structureActions";
import { matrixRowsForTask } from "./matrixRows";
import { structureDraft } from "./structureDeleteFixture";
import type { IeltsAuthoringIRV2 } from "../types";

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

/** 三个单题作答组、选项库共享、逐选框作答：满足矩阵前提。 */
function matrixDraft(): IeltsAuthoringIRV2 {
  const draft = structureDraft();
  const task = draft.taskGroups[0];
  const [groupA, groupB] = task.responseGroups;
  task.responseGroups = [
    { ...groupA, slotIds: ["q14"] },
    { ...groupA, responseGroupId: "rg-c", slotIds: ["q15"] },
    groupB,
  ];
  for (const slotId of ["q14", "q15", "q16"]) draft.answerSlots[slotId].interaction = "radio";
  return draft;
}

function matrixRows(draft: IeltsAuthoringIRV2) {
  const runtime = buildRuntimeViewModelV2(draft);
  return matrixRowsForTask(runtime, runtime.taskGroups[0], draft.passage?.paragraphMap);
}

describe("矩阵追加：answer-slot.append 在矩阵上编译为 insertResponseGroup", () => {
  it("矩阵任务：新增一个单题作答组，index 为当前组数，不内联选项，题号 = 全卷最大 + 1", () => {
    const draft = matrixDraft();
    const patch = compileStructureAction(draft, { type: "answer-slot.append", taskId: "headings", responseGroupId: "rg-b" });
    expect(patch?.op).toBe("insertResponseGroup");
    if (patch?.op !== "insertResponseGroup") throw new Error("expected insertResponseGroup");
    expect(patch.taskId).toBe("headings");
    expect(patch.index).toBe(3);
    expect(patch.responseGroup.slotIds).toEqual([patch.slot.slotId]);
    expect(patch.responseGroup.kind).toBe("matching");
    expect(patch.responseGroup.assignment).toBe("per_slot");
    expect(patch.responseGroup.options).toBeUndefined();
    expect(patch.responseGroup.optionBankRef).toBeUndefined();
    expect(patch.slot.questionNumber).toBe(17);
    expect(patch.slot.displayLabel).toBe("17");
    expect(patch.slot.interaction).toBe("radio");
    expect("expression" in patch).toBe(false);
  });

  it("应用后仍是矩阵（新行可作答）；逆补丁恢复原稿", () => {
    const draft = matrixDraft();
    const before = clone(draft);
    const patch = compileStructureAction(draft, { type: "answer-slot.append", taskId: "headings", responseGroupId: "rg-b" })!;
    const after = applyAuthoringV2Patches(draft, [patch]);
    expect(after.taskGroups[0].responseGroups).toHaveLength(4);
    const added = after.taskGroups[0].responseGroups[3].slotIds[0];
    expect(after.answerSlots[added].questionNumber).toBe(17);
    expect(after.answerKey[added]).toEqual({ kind: "unresolved" });
    expect(matrixRows(after)).toHaveLength(4);
    expect(applyAuthoringV2Patches(after, [inverseAuthoringPatch(draft, patch)!])).toEqual(before);
  });

  it("非矩阵的逐题列表仍编译为 insertAnswerSlot（不新增作答组）", () => {
    const draft = structureDraft();
    expect(matrixRows(draft)).toHaveLength(0);
    const patch = compileStructureAction(draft, { type: "answer-slot.append", taskId: "headings", responseGroupId: "rg-a" });
    expect(patch?.op).toBe("insertAnswerSlot");
  });
});
