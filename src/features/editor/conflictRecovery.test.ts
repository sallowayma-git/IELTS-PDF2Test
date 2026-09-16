import { describe, expect, it } from "vitest";
import { conflictRecoveryNotice, rebasePendingPatches } from "./conflictRecovery";
import type { AuthoringPatchV2, IeltsAuthoringIRV2 } from "../../types";

// 纯逻辑测试（vitest.config.ts 层 1）：验证保存冲突后的重放语义。
// 产品级证据仍然只在真实 Tauri/WebView2 里产生。

function baseDocument(): IeltsAuthoringIRV2 {
  return {
    schemaVersion: "IeltsAuthoringIRV2",
    exam: { id: "exam-1", title: "Original title" },
    questionGroups: [
      { id: "group-1", answerSlots: [{ id: "slot-1", displayLabel: "14" }, { id: "slot-2", displayLabel: "15" }] }
    ]
  } as unknown as IeltsAuthoringIRV2;
}

function setDisplayLabel(nodeId: string, displayLabel: string): AuthoringPatchV2 {
  return { op: "setNodeAttrs", nodeId, attrs: { displayLabel } } as unknown as AuthoringPatchV2;
}

function nodeById(document: IeltsAuthoringIRV2, id: string): { displayLabel?: string } {
  return (document as unknown as { questionGroups: { answerSlots: Array<{ id: string, displayLabel?: string }> }[] })
    .questionGroups[0].answerSlots.find((slot) => slot.id === id)!;
}

describe("rebasePendingPatches", () => {
  it("把本地未保存补丁重放到服务端最新版本上", () => {
    const base = baseDocument();
    nodeById(base, "slot-1").displayLabel = "14 (服务端已改)";

    const result = rebasePendingPatches(base, [setDisplayLabel("slot-2", "15 本地")]);

    expect(result.applied).toHaveLength(1);
    expect(result.dropped).toBe(0);
    // 服务端的新版本必须保留：本地补丁只落在它自己的目标上。
    expect(nodeById(result.rebased, "slot-1").displayLabel).toBe("14 (服务端已改)");
    expect(nodeById(result.rebased, "slot-2").displayLabel).toBe("15 本地");
  });

  it("某条补丁无法应用时停下，不静默丢弃其余修改", () => {
    const result = rebasePendingPatches(baseDocument(), [
      setDisplayLabel("slot-1", "14 本地"),
      setDisplayLabel("slot-deleted-on-server", "16 本地"),
      setDisplayLabel("slot-2", "15 本地")
    ]);

    expect(result.applied.map((patch) => (patch as { nodeId: string }).nodeId)).toEqual(["slot-1"]);
    expect(result.dropped).toBe(2);
    expect(nodeById(result.rebased, "slot-1").displayLabel).toBe("14 本地");
    // 停在失败点之后：后面的补丁没有被偷偷应用。
    expect(nodeById(result.rebased, "slot-2").displayLabel).toBe("15");
  });

  it("没有待保存修改时不改动服务端版本", () => {
    const base = baseDocument();
    const result = rebasePendingPatches(base, []);

    expect(result.applied).toHaveLength(0);
    expect(result.dropped).toBe(0);
    expect(nodeById(result.rebased, "slot-1").displayLabel).toBe("14");
  });
});

// 丢弃提示必须走独立状态：saveMessage 会被随后的「已保存」覆盖，
// 用户就再也看不到「有改动没保存」——那是静默数据丢失。
describe("conflictRecoveryNotice", () => {
  it("只要有补丁未能应用就必须给出提示", () => {
    const notice = conflictRecoveryNotice(1, 2);
    expect(notice).toBeDefined();
    // 说清「保住了几项」「从第几项起丢了」「丢了几项」。
    expect(notice).toContain("前 1 项");
    expect(notice).toContain("第 2 项起");
    expect(notice).toContain("2 项");
  });

  it("全部重放成功时不需要额外提示", () => {
    expect(conflictRecoveryNotice(3, 0)).toBeUndefined();
  });

  it("一项都没能重放时也要提示（不能因为 applied 为 0 就不提示）", () => {
    const notice = conflictRecoveryNotice(0, 4);
    expect(notice).toBeDefined();
    expect(notice).toContain("4 项");
  });
});
