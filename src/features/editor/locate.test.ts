import { describe, expect, it } from "vitest";
import type { IeltsAuthoringIRV2 } from "../../types";
import { contentNodeIdsForSlot, locateCandidateIds } from "./locate";

/** 最小草稿形状：只需 answerSlots 与 taskGroups 的可遍历结构。 */
function draftOf(value: unknown): IeltsAuthoringIRV2 {
  return value as IeltsAuthoringIRV2;
}

describe("contentNodeIdsForSlot", () => {
  it("在题组 stimulus 的嵌套内容节点里找到内联答案位的宿主 id", () => {
    const draft = draftOf({
      taskGroups: [
        {
          taskId: "group-1",
          stimulus: [
            { type: "text", id: "group-1-stimulus-b0", text: "The museum opens at" },
            { type: "answer_slot", id: "slot-node-q27", slotId: "q27" },
            { type: "text", id: "group-1-stimulus-b2", text: "each morning." }
          ]
        }
      ]
    });
    expect(contentNodeIdsForSlot(draft, "q27")).toEqual(["slot-node-q27"]);
  });

  it("未知答案位、空草稿、空 slotId 都返回空数组", () => {
    const draft = draftOf({ taskGroups: [{ stimulus: [{ type: "text", id: "t1", text: "x" }] }] });
    expect(contentNodeIdsForSlot(draft, "q40")).toEqual([]);
    expect(contentNodeIdsForSlot(undefined, "q40")).toEqual([]);
    expect(contentNodeIdsForSlot(draft, "")).toEqual([]);
  });

  it("同一答案位被多个节点承载时按遍历顺序全部返回", () => {
    const draft = draftOf({
      taskGroups: [
        { taskId: "g1", prompt: [{ type: "answer_slot", id: "node-a", slotId: "q3" }] },
        { taskId: "g2", stimulus: [{ type: "answer_slot", id: "node-b", slotId: "q3" }] }
      ]
    });
    expect(contentNodeIdsForSlot(draft, "q3")).toEqual(["node-a", "node-b"]);
  });

  it("循环引用不会造成死循环", () => {
    const stimulus: Record<string, unknown> = { type: "answer_slot", id: "slot-node-q1", slotId: "q1" };
    const group: Record<string, unknown> = { taskId: "g1", stimulus: [stimulus] };
    (stimulus as { parent?: unknown }).parent = group;
    (group as { self?: unknown }).self = group;
    expect(contentNodeIdsForSlot(draftOf({ taskGroups: [group] }), "q1")).toEqual(["slot-node-q1"]);
  });
});

describe("locateCandidateIds", () => {
  it("按 slotId → hostNodeId → 内容节点 id 的顺序给出候选并去重", () => {
    const draft = draftOf({
      answerSlots: { q27: { hostNodeId: "group-1-stimulus-b032" } },
      taskGroups: [
        { stimulus: [{ type: "answer_slot", id: "slot-node-q27", slotId: "q27" }] }
      ]
    });
    expect(locateCandidateIds(draft, "q27")).toEqual(["q27", "group-1-stimulus-b032", "slot-node-q27"]);
  });

  it("hostNodeId 缺失时候选只剩 slotId 与内容节点 id", () => {
    const draft = draftOf({
      taskGroups: [{ stimulus: [{ type: "answer_slot", id: "slot-node-q5", slotId: "q5" }] }]
    });
    expect(locateCandidateIds(draft, "q5")).toEqual(["q5", "slot-node-q5"]);
  });
});
