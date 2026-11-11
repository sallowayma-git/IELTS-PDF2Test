// 结构删除 / 添加的编译结果（证据层级：单元）。
// 同时用 TS 镜像补丁器验证：补丁能真正应用，且逆补丁（撤销）把稿件恢复到原样。
import { describe, expect, it } from "vitest";
import { applyAuthoringV2Patches, inverseAuthoringPatch } from "../services/authoringV2Patches";
import { compileStructureAction } from "./structureActions";
import { structureDraft } from "./structureDeleteFixture";

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

describe("逐题删除：answer-slot.delete", () => {
  it("列表型题目不带 nodeId，也不传 expression（由后端按保留题号计算）", () => {
    const patch = compileStructureAction(structureDraft(), { type: "answer-slot.delete", slotId: "q15" });
    expect(patch).toEqual({ op: "deleteAnswerSlot", taskId: "headings", responseGroupId: "rg-a", slotId: "q15" });
    expect(patch && "nodeId" in patch).toBe(false);
    expect(patch && "expression" in patch).toBe(false);
  });

  it("嵌入正文的答案位仍带上 nodeId", () => {
    const patch = compileStructureAction(structureDraft(), { type: "answer-slot.delete", slotId: "q15", nodeId: "slot-node-15" });
    expect(patch).toMatchObject({ op: "deleteAnswerSlot", nodeId: "slot-node-15", slotId: "q15" });
  });

  it("应用后只删掉该题；逆补丁恢复原稿", () => {
    const draft = structureDraft();
    const before = clone(draft);
    const patch = compileStructureAction(draft, { type: "answer-slot.delete", slotId: "q15" })!;
    const after = applyAuthoringV2Patches(draft, [patch]);
    expect(after.answerSlots.q15).toBeUndefined();
    expect(after.answerKey.q15).toBeUndefined();
    expect(after.taskGroups[0].responseGroups[0].slotIds).toEqual(["q14"]);
    const inverse = inverseAuthoringPatch(draft, patch)!;
    expect(applyAuthoringV2Patches(after, [inverse])).toEqual(before);
  });

  it("组内只剩一题时拒绝，提示整组删除", () => {
    expect(() => compileStructureAction(structureDraft(), { type: "answer-slot.delete", slotId: "q16" }))
      .toThrow("每组至少保留一题，可删除整组。");
  });

  it("找不到的答案位给出人话提示，而不是静默", () => {
    expect(() => compileStructureAction(structureDraft(), { type: "answer-slot.delete", slotId: "q99" }))
      .toThrow("这个答案位已不在当前题稿里");
  });
});

describe("逐题添加：answer-slot.append", () => {
  it("默认接在同组末尾：题号取全卷最大题号 + 1，逐题模式不带节点字段", () => {
    const patch = compileStructureAction(structureDraft(), { type: "answer-slot.append", taskId: "headings", responseGroupId: "rg-a" });
    expect(patch).toMatchObject({ op: "insertAnswerSlot", taskId: "headings", responseGroupId: "rg-a", slotIndex: 2, value: { kind: "unresolved" } });
    expect(patch && "node" in patch).toBe(false);
    expect(patch && "target" in patch).toBe(false);
    expect(patch && "index" in patch).toBe(false);
    expect(patch && "expression" in patch).toBe(false);
    if (patch?.op !== "insertAnswerSlot") throw new Error("expected insertAnswerSlot");
    expect(patch.slot.questionNumber).toBe(17);
    expect(patch.slot.displayLabel).toBe("17");
    expect(patch.slot.interaction).toBe("dragdrop");
  });

  it("指定 afterSlotId 时插到该题之后", () => {
    const patch = compileStructureAction(structureDraft(), { type: "answer-slot.append", taskId: "headings", responseGroupId: "rg-a", afterSlotId: "q14" });
    expect(patch).toMatchObject({ op: "insertAnswerSlot", slotIndex: 1 });
  });

  it("应用后新题出现在组内；逆补丁删除它并恢复原稿", () => {
    const draft = structureDraft();
    const before = clone(draft);
    const patch = compileStructureAction(draft, { type: "answer-slot.append", taskId: "headings", responseGroupId: "rg-a" })!;
    const after = applyAuthoringV2Patches(draft, [patch]);
    const group = after.taskGroups[0].responseGroups[0];
    expect(group.slotIds).toHaveLength(3);
    const added = group.slotIds[2];
    expect(after.answerSlots[added].questionNumber).toBe(17);
    expect(after.answerKey[added]).toEqual({ kind: "unresolved" });
    const inverse = inverseAuthoringPatch(draft, patch)!;
    expect(applyAuthoringV2Patches(after, [inverse])).toEqual(before);
  });

  it("逆补丁遇到之后的改动时拒绝恢复（RESTORE_CONFLICT），不留下半次撤销", () => {
    const draft = structureDraft();
    const deleted = applyAuthoringV2Patches(draft, [compileStructureAction(draft, { type: "answer-slot.delete", slotId: "q15" })!]);
    const inverse = inverseAuthoringPatch(draft, compileStructureAction(draft, { type: "answer-slot.delete", slotId: "q15" })!)!;
    const changedElsewhere = applyAuthoringV2Patches(deleted, [compileStructureAction(deleted, { type: "answer-slot.append", taskId: "headings", responseGroupId: "rg-a" })!]);
    expect(() => applyAuthoringV2Patches(changedElsewhere, [inverse])).toThrow(/AUTHORING_PATCH_RESTORE_CONFLICT/);
  });
});

describe("删除作答组与整道大题", () => {
  it("response-group.delete：删除整组及其题目，逆补丁恢复", () => {
    const draft = structureDraft();
    const before = clone(draft);
    const patch = compileStructureAction(draft, { type: "response-group.delete", taskId: "headings", responseGroupId: "rg-b" })!;
    expect(patch).toEqual({ op: "deleteResponseGroup", taskId: "headings", responseGroupId: "rg-b" });
    const after = applyAuthoringV2Patches(draft, [patch]);
    expect(after.taskGroups[0].responseGroups.map((group) => group.responseGroupId)).toEqual(["rg-a"]);
    expect(after.answerSlots.q16).toBeUndefined();
    expect(applyAuthoringV2Patches(after, [inverseAuthoringPatch(draft, patch)!])).toEqual(before);
  });

  it("task-group.delete：删除整道大题，逆补丁恢复", () => {
    const draft = structureDraft();
    const before = clone(draft);
    const patch = compileStructureAction(draft, { type: "task-group.delete", taskId: "headings" })!;
    expect(patch).toEqual({ op: "deleteTaskGroup", taskId: "headings" });
    const after = applyAuthoringV2Patches(draft, [patch]);
    expect(after.taskGroups).toHaveLength(0);
    expect(Object.keys(after.answerSlots)).toHaveLength(0);
    expect(applyAuthoringV2Patches(after, [inverseAuthoringPatch(draft, patch)!])).toEqual(before);
  });

  it("不存在的大题或作答组给出人话提示", () => {
    expect(() => compileStructureAction(structureDraft(), { type: "task-group.delete", taskId: "nope" })).toThrow("这道大题已不在当前题稿里");
    expect(() => compileStructureAction(structureDraft(), { type: "response-group.delete", taskId: "headings", responseGroupId: "nope" })).toThrow("这组题已不在当前题稿里");
  });
});
