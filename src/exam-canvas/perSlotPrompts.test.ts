import { describe, expect, it } from "vitest";
import type { ContentNodeV2, OptionV2, ResponseGroupV2 } from "../types";
import { perSlotPrompts } from "./perSlotPrompts";
// 选项 label/content 重复兜底的纯函数与学生端共用同一条规则，实现落在 tfngOptions.ts，
// 单测放在这里与逐题题干判定一起覆盖（都是画布版式的两个纯函数守卫）。
import { optionContentDuplicatesLabel } from "./tfngOptions";

// Evidence level: pure unit（per-slot prompt 判定 + 选项 content 重复兜底）。

const text = (id: string, value: string): ContentNodeV2 =>
  ({ id, type: "text", text: value, sourceAnchors: [], provenanceStatus: "source" });

const paragraph = (id: string, children: ContentNodeV2[]): ContentNodeV2 =>
  ({ id, type: "paragraph", children, sourceAnchors: [], provenanceStatus: "source" });

const promptParagraph = (id: string, statement: string): ContentNodeV2 =>
  paragraph(id, [text(`${id}-text`, statement)]);

const slotWithHost = (slotId: string, hostNodeId?: string) => ({
  slotId,
  hostNodeId,
});

const slotTable = {
  q1: slotWithHost("q1", "group-1-prompt-1"),
  q2: slotWithHost("q2", "group-1-prompt-2"),
  q3: slotWithHost("q3", "group-1-prompt-3")
};

const prompt = [
  promptParagraph("group-1-prompt-1", "Statement one."),
  promptParagraph("group-1-prompt-2", "Statement two."),
  promptParagraph("group-1-prompt-3", "Statement three.")
];

const response = (overrides: Partial<ResponseGroupV2> = {}): ResponseGroupV2 => ({
  responseGroupId: "group-1",
  kind: "choice",
  prompt,
  slotIds: ["q1", "q2", "q3"],
  cardinality: { min: 1, max: 1 },
  assignment: "per_slot",
  scoringPolicy: "per_slot_binary",
  duplicatePolicy: "reject_submission",
  allowOptionReuse: false,
  sourceAnchors: [],
  ...overrides
});

describe("perSlotPrompts 判定", () => {
  it("三个 slot 的 hostNodeId 一一命中 prompt 顶层段落时启用，返回每个 slot 的宿主节点与认领集合", () => {
    const layout = perSlotPrompts(response(), slotTable);
    expect(layout).toBeDefined();
    expect(layout!.hostNodeBySlotId.get("q1")?.id).toBe("group-1-prompt-1");
    expect(layout!.hostNodeBySlotId.get("q2")?.id).toBe("group-1-prompt-2");
    expect(layout!.hostNodeBySlotId.get("q3")?.id).toBe("group-1-prompt-3");
    expect([...layout!.claimedNodeIds]).toEqual(["group-1-prompt-1", "group-1-prompt-2", "group-1-prompt-3"]);
  });

  it("两个 slot 指向同一段（宿主不两两不同）→ 不启用，避免半拆", () => {
    const slots = { q1: slotWithHost("q1", "group-1-prompt-1"), q2: slotWithHost("q2", "group-1-prompt-1") };
    expect(perSlotPrompts(response({ slotIds: ["q1", "q2"] }), slots)).toBeUndefined();
  });

  it("assignment 为 unordered_set（共享多选）→ 不启用", () => {
    expect(perSlotPrompts(response({ assignment: "unordered_set" }), slotTable)).toBeUndefined();
  });

  it("slotIds 为空 → 不启用（顶部 prompt 必须保持原样）", () => {
    expect(perSlotPrompts(response({ slotIds: [] }), {})).toBeUndefined();
  });

  it("slot 在 answerSlots 里缺失、或没有 hostNodeId → 不启用", () => {
    expect(perSlotPrompts(response(), { q1: slotWithHost("q1", "group-1-prompt-1"), q2: slotWithHost("q2", "group-1-prompt-2") })).toBeUndefined();
    expect(perSlotPrompts(response(), { q1: slotWithHost("q1", "group-1-prompt-1"), q2: slotWithHost("q2"), q3: slotWithHost("q3", "group-1-prompt-3") })).toBeUndefined();
  });

  it("宿主不在 prompt 顶层（找不到 id）→ 不启用", () => {
    expect(perSlotPrompts(response(), { ...slotTable, q2: slotWithHost("q2", "missing-node") })).toBeUndefined();
  });

  it("宿主段落内部嵌 answer_slot（embedded 填空）→ 不启用", () => {
    const embeddedPrompt = [
      promptParagraph("group-1-prompt-1", "Statement one."),
      paragraph("group-1-prompt-2", [
        text("group-1-prompt-2-text", "Statement two."),
        { id: "slot-q2", type: "answer_slot", slotId: "q2", displayLabel: "2", inline: true, sourceAnchors: [], provenanceStatus: "source" }
      ]),
      promptParagraph("group-1-prompt-3", "Statement three.")
    ];
    expect(perSlotPrompts(response({ prompt: embeddedPrompt }), slotTable)).toBeUndefined();
  });

  it("宿主节点本身就是 answer_slot → 不启用", () => {
    const slotHostedPrompt: ContentNodeV2[] = [
      promptParagraph("group-1-prompt-1", "Statement one."),
      { id: "group-1-prompt-2", type: "answer_slot", slotId: "q2", displayLabel: "2", inline: true, sourceAnchors: [], provenanceStatus: "source" },
      promptParagraph("group-1-prompt-3", "Statement three.")
    ];
    expect(perSlotPrompts(response({ prompt: slotHostedPrompt }), slotTable)).toBeUndefined();
  });

  it("response / prompt 结构异常一律不启用", () => {
    expect(perSlotPrompts(undefined, slotTable)).toBeUndefined();
    expect(perSlotPrompts(null, slotTable)).toBeUndefined();
    expect(perSlotPrompts(response({ prompt: [] }), slotTable)).toBeUndefined();
    expect(perSlotPrompts({ ...response(), prompt: undefined } as unknown as ResponseGroupV2, slotTable)).toBeUndefined();
    expect(perSlotPrompts({ ...response(), slotIds: undefined } as unknown as ResponseGroupV2, slotTable)).toBeUndefined();
  });

  it("answerSlots 表本身缺失 → 不启用", () => {
    expect(perSlotPrompts(response(), undefined)).toBeUndefined();
  });
});

const option = (optionId: string, label: string, content: string): OptionV2 => ({
  optionId,
  label,
  content: content ? [text(`${optionId}-content`, content)] : [],
  sourceAnchors: []
});

describe("optionContentDuplicatesLabel 兜底", () => {
  it("content 纯文本 trim 后与 label 相同（大小写不敏感）→ 判定为重复", () => {
    expect(optionContentDuplicatesLabel(option("o1", "TRUE", "TRUE"))).toBe(true);
    expect(optionContentDuplicatesLabel(option("o2", "YES", " yes "))).toBe(true);
    expect(optionContentDuplicatesLabel(option("o3", "not given", "Not Given"))).toBe(true);
  });

  it("content 与 label 不同、或 content 为空 → 正常显示，不判重复", () => {
    expect(optionContentDuplicatesLabel(option("o4", "FALSE", "contradiction of the passage"))).toBe(false);
    expect(optionContentDuplicatesLabel(option("o5", "NOT GIVEN", ""))).toBe(false);
    expect(optionContentDuplicatesLabel(undefined)).toBe(false);
  });
});
