import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  deriveRepairScenario,
  diagnoseAnswerClaimL1,
  loadRepairGolden,
  questionLineByNumber,
} from "./cloud-repair-scenario.mjs";

// 证据层级：pure unit。断言的是**验收场景装配本身**，不是产品行为。
//
// 背景：旧装配是「脚本派生期望值」——脚本自己剥掉页脚残留，把结果同时当作候选内容、
// 剧本里的 `fixedPromptText`、以及断言时比的字符串。三者是同一个值，于是
// 「云端能依据原文件修正识别错误」无法被证伪。下面每条用例钉住新装配的一个入口：
// 期望值只来自 golden fixture、错误必须真实存在、剧本里不许出现期望值。

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const golden = loadRepairGolden(repoRoot);
const annotated = golden.recognitionErrors[0];

/** 一份最小真实稿：只有承载 q40 的作答组会被场景用到。 */
function draftWith(promptText, { slotIds = ["q40"], instructions = "Questions 32 - 35 Do the following statements agree with the claims of the writer? In boxes 32-35 write YES / NO / NOT GIVEN." } = {}) {
  return {
    taskGroups: [
      {
        taskId: "group-1",
        taskType: "short_answer",
        instructions: [{ type: "text", text: "Answer question 1 in a few words." }],
        stimulus: [{ type: "text", text: "A verdant ecosystem supports unusual species." }],
        responseGroups: [
          { responseGroupId: "group-1-response-1", slotIds: ["q1"], prompt: [{ type: "text", text: "Describe the ecosystem." }] },
        ],
      },
      {
        taskId: "group-2",
        taskType: "yes_no_not_given",
        instructions: [{ type: "text", text: instructions }],
        responseGroups: [
          { responseGroupId: "group-2-response-32", slotIds: ["q32"], prompt: [{ type: "text", text: "unrelated" }] },
        ],
      },
      {
        taskId: "group-3",
        taskType: "single_choice",
        displayRange: { start: 36, end: 40 },
        instructions: [{ type: "text", text: "Questions 36 - 40 Choose the correct letter, A, B, C or D." }],
        responseGroups: [
          {
            responseGroupId: "group-3-response-40",
            slotIds,
            prompt: [{ type: "text", text: promptText }],
            sourceAnchors: [{ sourceFileId: "file-1", pageIndex: 3 }],
          },
        ],
      },
    ],
    answerSlots: { q1: {}, q40: {} },
    answerKey: {},
  };
}

describe("cloud-repair 场景装配", () => {
  it("原文裁定按完整题号行定位，不把 Questions 36–40 范围标题当作第 40 题引文", () => {
    const lines = [
      { pageIndex: 4, text: "Questions 36 – 40 Choose the correct letter." },
      { pageIndex: 4, text: "40 The writer recommends that social history must" },
      { pageIndex: 4, text: "140 This is another question." },
    ];

    expect(questionLineByNumber(lines, 40)).toEqual(lines[1]);
    expect(questionLineByNumber(lines, 4)).toBeNull();
  });

  it("本地识别真实出错时：期望值来自 fixture，改前取真实稿，且剧本里没有期望值", () => {
    const draft = draftWith(annotated.localDraftContains);
    const derived = deriveRepairScenario(draft, golden);

    expect(derived.ok).toBe(true);
    // 改前 = 真实稿读出来的；改后 = 人工标注的原文件真值。
    expect(derived.fix.before).toBe(annotated.localDraftContains);
    expect(derived.fix.after).toBe(annotated.originalFileSays);
    expect(derived.fix.responseGroupId).toBe("group-3-response-40");
    expect(derived.fix.sourcePageOneBased).toBe(annotated.sourcePage.oneBased);
    // 剧本里**不许**出现期望值：出现就意味着受控服务不必读原文件，链条退回自证。
    expect(JSON.stringify(derived.plan)).not.toContain(annotated.originalFileSays);
    expect(derived.plan).not.toHaveProperty("fixedPromptText");
    // 目标定位给的是 slotId，受控服务要在真实稿里自己找。
    expect(derived.plan.fixSlotIds).toEqual(annotated.target.slotIds);
    expect(derived.rule).toMatchObject({
      targetType: "response_group",
      targetId: derived.fix.responseGroupId,
      field: "prompt",
      candidate: annotated.originalFileSays,
      challenger: annotated.localDraftContains,
    });
    expect(derived.plan.rulings[0]).toMatchObject({
      targetType: "response_group",
      targetId: derived.fix.responseGroupId,
      field: "prompt",
      ruling: "current_is_correct",
    });
    expect(derived.claim).toMatchObject({
      slotId: "q1",
      searchPages: [Number(golden.source.pageCount)],
    });
    expect(derived.plan.answerClaim).toMatchObject({
      slotId: derived.claim.slotId,
      searchPages: derived.claim.searchPages,
    });
  });

  it("本地识别没有出错时：如实说不适用，**不注入**错误", () => {
    const draft = draftWith("The writer recommends that to be effective, social history must");
    const derived = deriveRepairScenario(draft, golden);

    expect(derived.ok).toBe(false);
    expect(derived.reason).toContain("没有产出");
    // 关键：返回里没有任何候选样本——不能自己造一个错再「修好」。
    expect(derived.candidate).toBeUndefined();
    expect(derived.plan).toBeUndefined();
    // 实测值要如实带出来，报告里才能看出「是本地没错，还是脚本挑剔」。
    expect(derived.observed).toBe("The writer recommends that to be effective, social history must");
    expect(derived.expected).toBe(annotated.localDraftContains);
  });

  it("目标按 slotId 定位：slotId 对不上就不硬套", () => {
    const draft = draftWith(annotated.localDraftContains, { slotIds: ["q39"] });
    const derived = deriveRepairScenario(draft, golden);

    expect(derived.ok).toBe(false);
    expect(derived.reason).toContain("找不到承载");
  });

  it("候选里题面被改成原文件真值，而输入稿本身不被改动", () => {
    const draft = draftWith(annotated.localDraftContains);
    const before = JSON.stringify(draft);
    const derived = deriveRepairScenario(draft, golden);

    expect(derived.ok).toBe(true);
    const candidatePrompt = derived.candidate.taskGroups
      .find((group) => group.taskId === "group-3")
      .responseGroups.find((response) => response.responseGroupId === "group-3-response-40")
      .prompt[0].text;
    expect(candidatePrompt).toBe(annotated.originalFileSays);
    const localInstructions = draft.taskGroups.find((group) => group.taskId === "group-2").instructions;
    const candidateInstructions = derived.candidate.taskGroups.find((group) => group.taskId === "group-2").instructions;
    expect(candidateInstructions).toEqual(localInstructions);
    // 真实稿必须原样不动：脚本只读它。
    expect(JSON.stringify(draft)).toBe(before);
  });

  it("剧本里没有任何答案值：原文件没有答案页，答案不能被编造", () => {
    const derived = deriveRepairScenario(draftWith(annotated.localDraftContains), golden);
    expect(derived.ok).toBe(true);
    const plan = JSON.stringify(derived.plan);
    for (const slotId of Object.keys(golden.expectedAnswers ?? {})) {
      // 只允许出现 slotId 本身（作为目标），不允许出现任何答案值。
      expect(plan).not.toMatch(new RegExp(`"${slotId}"\\s*:\\s*\\{[^}]*"(labels|values)"`, "u"));
    }
    expect(golden.expectedAnswers.q40).toBeNull();
  });

  it("没有未解析答案位时拒绝生成缺少 W1 主张的场景", () => {
    const draft = draftWith(annotated.localDraftContains);
    draft.answerKey = {
      q1: { kind: "text", values: ["filled"] },
      q32: { kind: "text", values: ["filled"] },
      q40: { kind: "text", values: ["filled"] },
    };

    const derived = deriveRepairScenario(draft, golden);

    expect(derived.ok).toBe(false);
    expect(derived.reason).toContain("W1");
  });
});

describe("W1 答案主张 L1 证据绑定", () => {
  const claim = { slotId: "q32", searchPages: [5] };
  const repairRounds = [
    {
      stamp: 1,
      packetId: "pkt-prompt",
      packetPages: [],
      escalationLevel: 0,
      differenceTargets: [{ targetType: "response_group", targetId: "response-1", field: "prompt" }],
    },
    {
      stamp: 2,
      packetId: "pkt-answer",
      packetPages: [2],
      escalationLevel: 0,
      toolObservations: [],
      differenceTargets: [{ targetType: "slot", targetId: "q32", field: "answer" }],
    },
    {
      stamp: 3,
      packetId: "pkt-answer",
      packetPages: [2],
      escalationLevel: 1,
      toolObservations: [{
        callId: "read-answer-page",
        status: "ok",
        pageIndexes: [5],
      }],
      differenceTargets: [{ targetType: "slot", targetId: "q32", field: "answer" }],
    },
  ];

  it("只接受答案主张包成功抓取搜索页并进入 L1", () => {
    const result = diagnoseAnswerClaimL1({
      claim,
      repairRounds,
      callRecords: [
        { packetId: "pkt-prompt", escalationLevel: 1, pagesIncluded: [5] },
        { packetId: "pkt-answer", escalationLevel: 0, pagesIncluded: [2] },
        { packetId: "pkt-answer", escalationLevel: 1, pagesIncluded: [2] },
      ],
      toolCalls: [{ packetId: "pkt-answer", callId: "read-answer-page", tool: "read_source" }],
    });

    expect(result.ok).toBe(true);
    expect(result.qualifyingPacketIds).toEqual(["pkt-answer"]);
  });

  it("report_insufficient_context 满足答案搜索页也算有效的同包 L1", () => {
    const result = diagnoseAnswerClaimL1({
      claim,
      repairRounds: repairRounds.map((round) => round.stamp === 3
        ? {
            ...round,
            toolObservations: [{
              callId: "need-answer-page",
              status: "ok",
              pageIndexes: [5],
            }],
          }
        : round),
      callRecords: [
        { packetId: "pkt-answer", escalationLevel: 0, pagesIncluded: [2] },
        { packetId: "pkt-answer", escalationLevel: 1, pagesIncluded: [2] },
      ],
      toolCalls: [{ packetId: "pkt-answer", callId: "need-answer-page", tool: "report_insufficient_context" }],
    });

    expect(result.ok).toBe(true);
    expect(result.qualifyingPacketIds).toEqual(["pkt-answer"]);
  });

  it("无关包升级或答案包升级到别的页都不能满足 W1", () => {
    const result = diagnoseAnswerClaimL1({
      claim,
      repairRounds: repairRounds.map((round) => round.stamp === 3
        ? {
            ...round,
            toolObservations: [{
              callId: "read-answer-page",
              status: "ok",
              pageIndexes: [4],
            }],
          }
        : round),
      callRecords: [
        { packetId: "pkt-prompt", escalationLevel: 1, pagesIncluded: [5] },
        { packetId: "pkt-answer", escalationLevel: 0, pagesIncluded: [2] },
        { packetId: "pkt-answer", escalationLevel: 1, pagesIncluded: [2] },
      ],
      toolCalls: [{ packetId: "pkt-answer", callId: "read-answer-page", tool: "read_source" }],
    });

    expect(result.ok).toBe(false);
    expect(result.problems).toContain(
      "答案差异所在的包没有从缺少搜索页的首轮请求，经成功的同页抓取进入 L1",
    );
  });
});

describe("golden fixture 自身", () => {
  it("绑定的是具体一份原文件（哈希 + 页号），并声明了缺失的答案页", () => {
    expect(golden.source.sha256).toMatch(/^[0-9a-f]{64}$/u);
    expect(golden.source.format).toBe("pdf");
    expect(annotated.sourcePage.oneBased).toBeGreaterThanOrEqual(1);
    expect(annotated.originalFileQuote).toContain(annotated.originalFileSays);
    expect(golden.answerKeyAbsence.slots.length).toBeGreaterThan(0);
  });
});
