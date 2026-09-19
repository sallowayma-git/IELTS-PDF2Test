import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { deriveRepairScenario, loadRepairGolden } from "./cloud-repair-scenario.mjs";

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
    answerSlots: { q40: {} },
    answerKey: {},
  };
}

describe("cloud-repair 场景装配", () => {
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
