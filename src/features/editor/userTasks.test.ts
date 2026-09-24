import { describe, expect, it } from "vitest";
import type { AnswerSlotV2, IeltsAuthoringIRV2, ResponseGroupV2, TaskGroupV2 } from "../../types";
import type { ActionableIssueV1 } from "./actionableIssues";
import { buildEditingAids, buildUserTasks, rootCausesOf, splitVisibleTasks, USER_TASK_VISIBLE_LIMIT } from "./userTasks";

// 证据层级：pure unit。断言的是**本轮任务书第 3/4/5 条**那三条互相牵制的规则：
//   1. 同一问题不重复显示（连续缺答并成区间、同一题组并成一条）；
//   2. 不同问题不被隐藏（未知码降级而不是丢弃；泛化行只在**原因被完整表达**时才隐藏）；
//   3. 每条任务都有真能解决问题的按钮（动作种类受限，且必须指向一个真实目标）。
//
// 这些规则彼此拉扯：「少显示几条」与「别藏问题」在任何一次改动里都只有一个能赢。
// 所以这里既断言「合并发生了」，也断言「一个根因都没丢」。

function group(responseGroupId: string, slotIds: string[]): ResponseGroupV2 {
  return {
    responseGroupId,
    kind: "text_entry",
    slotIds,
    cardinality: { min: 1, max: 1 },
    assignment: "per_slot",
    scoringPolicy: "per_slot_binary",
    duplicatePolicy: "reject_submission",
    allowOptionReuse: false,
    sourceAnchors: [],
    prompt: []
  } as unknown as ResponseGroupV2;
}

function task(taskId: string, responseGroups: ResponseGroupV2[]): TaskGroupV2 {
  return {
    taskId,
    taskType: "sentence_completion",
    responseGroups,
    displayRange: { kind: "range", start: 1, end: 1 },
    instructions: [],
    instructionSignature: {
      normalizedText: "",
      taskType: "sentence_completion",
      expectedQuestionNumbers: [],
      expectedSlotCount: 0,
      evidenceAnchors: [],
      confidence: 1
    },
    sourceAnchors: [],
    quality: { score: 1, sourceCoverage: 1, hardFailures: [] },
    reviewState: "unreviewed"
  } as TaskGroupV2;
}

function slot(slotId: string, questionNumber: number): AnswerSlotV2 {
  return {
    slotId,
    questionNumber,
    displayLabel: String(questionNumber),
    hostType: "prompt",
    interaction: "radio",
    participation: "scoring",
    sourceAnchors: [],
    confidence: 1
  } as AnswerSlotV2;
}

function makeDs(input: {
  taskGroups: TaskGroupV2[];
  answerSlots: Record<string, AnswerSlotV2>;
}): IeltsAuthoringIRV2 {
  return {
    schemaVersion: "IeltsAuthoringIRV2",
    jobId: "job-1",
    exam: { examId: "exam-1", title: "T", language: "en", tags: [], sourceFiles: [] },
    modality: "reading",
    taskGroups: input.taskGroups,
    answerSlots: input.answerSlots,
    answerKey: {},
    assets: [],
    sourceDocumentId: "doc-1"
  } as unknown as IeltsAuthoringIRV2;
}

/** 一条原始问题行。`rootCause` 显式给出，模拟 `mergePublishGateIssues` 的产物。 */
function issue(id: string, code: string, targetId: string, rootCause?: string): ActionableIssueV1 {
  return {
    issueId: id,
    targetId,
    severity: "blocker",
    code,
    userMessage: `m-${id}`,
    source: "gate",
    rootCause: rootCause ?? code
  };
}

/** 三个连续题位的题组（第 11–13 题）。 */
const DS = makeDs({
  taskGroups: [task("task-1", [group("rg-1", ["q11", "q12", "q13"])])],
  answerSlots: { q11: slot("q11", 11), q12: slot("q12", 12), q13: slot("q13", 13) }
});

describe("缺答案：连续题号并成一个区间（任务书第 3 条）", () => {
  it("第 11/12/13 题缺答案 → 一条「第 11–13 题缺少答案」，动作为「去填写」", () => {
    const summary = buildUserTasks(DS, [
      issue("i1", "ANSWER_MISSING", "q11"),
      issue("i2", "ANSWER_MISSING", "q12"),
      issue("i3", "ANSWER_MISSING", "q13")
    ]);
    expect(summary.tasks).toHaveLength(1);
    const [task] = summary.tasks;
    expect(task.kind).toBe("missing-answer");
    expect(task.title).toBe("第 11–13 题缺少答案");
    expect(task.actions.map((a) => a.id)).toEqual(["fill-answer"]);
    // 合并后的任务必须保留**底层问题关联**：三行原始问题都在 covers 里。
    expect(task.covers).toEqual(["i1", "i2", "i3"]);
    expect(rootCausesOf([
      issue("i1", "ANSWER_MISSING", "q11"),
      issue("i2", "ANSWER_MISSING", "q12"),
      issue("i3", "ANSWER_MISSING", "q13")
    ], task)).toEqual(["ANSWER_MISSING"]);
  });

  it("不连续的缺答分成两条任务（不能为了「少显示」把第 11 题和第 20 题并成一条）", () => {
    const ds = makeDs({
      taskGroups: [task("task-1", [group("rg-1", ["q11", "q20"])])],
      answerSlots: { q11: slot("q11", 11), q20: slot("q20", 20) }
    });
    const summary = buildUserTasks(ds, [issue("i1", "ANSWER_MISSING", "q11"), issue("i2", "ANSWER_MISSING", "q20")]);
    expect(summary.tasks.map((t) => t.title)).toEqual(["第 11 题缺少答案", "第 20 题缺少答案"]);
  });

  it("同一道题被本地闭包与门禁各写一行时只出一条（根因别名归一）", () => {
    // 本地记 `ANSWER_UNRESOLVED`、门禁记 `ANSWER_MISSING`；`mergePublishGateIssues` 会把根因
    // 归一成 `ANSWER_MISSING`。这里模拟归一后的两行落在同一个 slot 上。
    const summary = buildUserTasks(DS, [
      issue("i1", "ANSWER_UNRESOLVED", "q11", "ANSWER_MISSING"),
      issue("i2", "ANSWER_MISSING", "q11", "ANSWER_MISSING")
    ]);
    expect(summary.tasks).toHaveLength(1);
    expect(summary.tasks[0].covers).toEqual(["i1", "i2"]);
  });
});

describe("题组内部问题：并成一条「没有识别完整」，两个动作都在（任务书第 3 条）", () => {
  it("题干/stimulus/slot host 指向同一题组时只出一条，动作为「查看原文」", () => {
    const summary = buildUserTasks(DS, [
      issue("i1", "PROMPT_EMPTY", "rg-1"),
      issue("i2", "STIMULUS_MISSING", "rg-1"),
      issue("i3", "SLOT_HOST_MISSING", "rg-1")
    ]);
    expect(summary.tasks).toHaveLength(1);
    const [task] = summary.tasks;
    expect(task.kind).toBe("incomplete-recognition");
    expect(task.title).toBe("第 11–13 题没有识别完整");
    // 两个动作分别可操作 —— 「不同修复动作保留可分别操作的入口」。
    // 不给「重新识别」：重跑不会替换已生成的题稿，按了改不掉这里的问题。
    expect(task.actions.map((a) => a.id)).toEqual(["view-source"]);
    expect(task.actions.map((a) => a.label)).toEqual(["查看原文"]);
  });

  it("指向答案位的问题会归到它所属的题组（而不是散成三条）", () => {
    const summary = buildUserTasks(DS, [
      issue("i1", "PROMPT_EMPTY", "q11"),
      issue("i2", "STIMULUS_MISSING", "q12")
    ]);
    expect(summary.tasks).toHaveLength(1);
    expect(summary.tasks[0].taskId).toBe("incomplete-recognition:rg-1");
  });
});

describe("泛化行：只有阻塞原因被**完整**表达时才隐藏（任务书第 4 条）", () => {
  it("有具体任务且原因被完整表达 → 泛化行隐藏", () => {
    const summary = buildUserTasks(DS, [
      issue("i1", "ANSWER_MISSING", "q11"),
      issue("g1", "QUALITY_NOT_READY", "QUALITY_NOT_READY", "QUALITY_NOT_READY")
    ]);
    // 只剩那条具体的缺答任务。
    expect(summary.tasks).toHaveLength(1);
    expect(summary.tasks[0].kind).toBe("missing-answer");
    // 被隐藏的行仍计入 mergedRowCount（验收脚本据此断言「真的合并了」）。
    expect(summary.mergedRowCount).toBeGreaterThan(0);
  });

  it("`QUALITY_HARD_FAILURE` 带着**具体**根因时不算泛化行，必须按根因归类而不是丢掉", () => {
    // 门禁用 `QUALITY_HARD_FAILURE` 这个 code 承载「有硬失败」，根因在 `internal` 里
    // （`mergePublishGateIssues` 把它写进 `rootCause`）。这类行的根因是**具体**的，
    // 按 `GENERIC_ONLY` 一律当泛化丢掉就会藏起一条真实阻断。
    const summary = buildUserTasks(DS, [issue("g2", "QUALITY_HARD_FAILURE", "q12", "ANSWER_MISSING")]);
    expect(summary.tasks).toHaveLength(1);
    expect(summary.tasks[0].kind).toBe("missing-answer");
    expect(summary.tasks[0].title).toBe("第 12 题缺少答案");
  });

  it("**没有**具体任务时泛化行是唯一线索，必须显示", () => {
    const summary = buildUserTasks(DS, [issue("g1", "QUALITY_NOT_READY", "QUALITY_NOT_READY", "QUALITY_NOT_READY")]);
    expect(summary.tasks).toHaveLength(1);
    expect(summary.tasks[0].kind).toBe("structure-incomplete");
    expect(summary.tasks[0].title).toBe("这道题的结构可能还不完整");
  });

  it("**原因没有被表达**的失败不被隐藏：门禁报 A 与 B，只有 A 有任务，B 必须仍出现", () => {
    // 这是本轮修正的关键一条。上一版写的是「有任意具体任务就隐藏全部泛化/结构行」——
    // 那会藏掉尚未被解释的发布失败：用户看到「还有 1 处需要处理」，而发布其实被另外一条拦着。
    const summary = buildUserTasks(DS, [
      issue("i1", "ANSWER_MISSING", "q11"),
      issue("s1", "RUNTIME_COMPILER_FAILED", "document", "RUNTIME_COMPILER_FAILED")
    ]);
    const kinds = summary.tasks.map((t) => t.kind);
    expect(kinds).toContain("missing-answer");
    // 编译器失败没有被任何具体任务表达 → 必须如实再给一条。
    expect(kinds).toContain("structure-incomplete");
    const unexplained = summary.tasks.find((t) => t.taskId === "structure-incomplete:unexplained");
    expect(unexplained).toBeDefined();
    expect(unexplained!.covers).toEqual(["s1"]);
  });

  it("RUNTIME_COMPILER_FAILED 与它自己的具体原因并存时，只显示具体原因", () => {
    // 编译器失败与「缺答案」是**同一件事**的两条记录（都指向 q11）时不该占两行。
    const summary = buildUserTasks(DS, [
      issue("i1", "ANSWER_MISSING", "q11"),
      issue("s1", "RUNTIME_COMPILER_FAILED", "q11", "ANSWER_MISSING")
    ]);
    expect(summary.tasks.map((t) => t.kind)).toEqual(["missing-answer"]);
  });
});

describe("未知码不丢弃（「不同问题不被隐藏」的兜底）", () => {
  it("没见过的质量码降级成「没有处理好 → 查看原文」，而不是被丢掉", () => {
    const summary = buildUserTasks(DS, [issue("i1", "SOMETHING_BRAND_NEW", "q11")]);
    expect(summary.tasks).toHaveLength(1);
    expect(summary.tasks[0].kind).toBe("processing-failed");
    expect(summary.tasks[0].actions.map((a) => a.id)).toEqual(["view-source"]);
    expect(summary.tasks[0].covers).toEqual(["i1"]);
  });

  it("列表被截断的提示不进普通界面（它只是内部说明）", () => {
    const summary = buildUserTasks(DS, [issue("t1", "BLOCKER_LIST_TRUNCATED", "BLOCKER_LIST_TRUNCATED", "BLOCKER_LIST_TRUNCATED")]);
    expect(summary.tasks).toEqual([]);
  });
});

describe("每条任务都有真能解决问题的按钮（任务书第 5 条）", () => {
  it("所有动作都在允许集合内，且都带一个非空目标", () => {
    const summary = buildUserTasks(DS, [
      issue("i1", "ANSWER_MISSING", "q11"),
      issue("i2", "PROMPT_EMPTY", "rg-1"),
      issue("i3", "ASSET_MISSING", "q12"),
      issue("i4", "SOMETHING_BRAND_NEW", "q13")
    ]);
    const allowed = new Set(["fill-answer", "view-source"]);
    for (const task of summary.tasks) {
      expect(task.actions.length).toBeGreaterThan(0);
      for (const action of task.actions) {
        expect(allowed.has(action.id)).toBe(true);
        expect(action.targetId.trim().length).toBeGreaterThan(0);
        expect(action.label.trim().length).toBeGreaterThan(0);
      }
    }
  });

  it("不提供「确认」「忽略」这类点了不改变门禁结果的按钮", () => {
    const summary = buildUserTasks(DS, [issue("i1", "ANSWER_MISSING", "q11"), issue("i2", "PROMPT_EMPTY", "rg-1")]);
    const labels = summary.tasks.flatMap((t) => t.actions.map((a) => a.label));
    expect(labels.some((label) => /确认|忽略/.test(label))).toBe(false);
  });
});

describe("headline 与折叠", () => {
  it("没有问题时不生成卡片，也不下「可以导出」这种结论", () => {
    const summary = buildUserTasks(DS, []);
    expect(summary.tasks).toEqual([]);
    expect(summary.headline).toBe("没有需要补充的内容");
  });

  it("有问题时说「还有 N 处可以补充」", () => {
    const summary = buildUserTasks(DS, [issue("i1", "ANSWER_MISSING", "q11"), issue("i2", "PROMPT_EMPTY", "rg-1")]);
    expect(summary.headline).toBe("还有 2 处可以补充");
  });

  it("超过阈值先折叠，展开后全部可见（不再是「仅显示前 N 条」）", () => {
    const slots: Record<string, AnswerSlotV2> = {};
    const issues: ActionableIssueV1[] = [];
    for (let index = 0; index < USER_TASK_VISIBLE_LIMIT + 3; index += 1) {
      // 题号间隔 2，保证每条都自成一条任务。
      const number = index * 2 + 1;
      slots[`q${number}`] = slot(`q${number}`, number);
      issues.push(issue(`i${index}`, "ANSWER_MISSING", `q${number}`));
    }
    const ds = makeDs({ taskGroups: [task("task-1", [group("rg-1", Object.keys(slots))])], answerSlots: slots });
    const summary = buildUserTasks(ds, issues);
    expect(summary.tasks).toHaveLength(USER_TASK_VISIBLE_LIMIT + 3);
    const collapsed = splitVisibleTasks(summary.tasks, false);
    expect(collapsed.visible).toHaveLength(USER_TASK_VISIBLE_LIMIT);
    expect(collapsed.hiddenCount).toBe(3);
    const expanded = splitVisibleTasks(summary.tasks, true);
    expect(expanded.visible).toHaveLength(USER_TASK_VISIBLE_LIMIT + 3);
    expect(expanded.hiddenCount).toBe(0);
  });
});

// ── 草稿就绪（F-R15-5）────────────────────────────────────────────────
//
// 这条规则是从一次**间歇的真实失败**里长出来的：`preflight`（后端门禁）与草稿是两条并行的
// 异步链，门禁先回来而草稿还没读进来时，`slotIdsOfTarget` 因为拿不到 `answerSlots` 一律返回空，
// 带题号的缺答问题就退化成 `missing-answer:unnumbered`。任务卡看着正常，点「去填写」却定位不到
// 任何元素——因为题面此刻也还没渲染出那道题（实测同一份构建/夹具，一次题号解析成功、一次退化）。
//
// 所以「没有草稿」必须返回 `ready:false`，而**不是**「没问题」：后者会让界面在题稿打开之前
// 就宣称「可以导出」，正是任务书第 2 条要禁的那类假信息。
describe("草稿就绪：没有题稿时不生成任务，也不宣称「可以导出」（F-R15-5）", () => {
  it("ds 为 undefined 时 ready=false、没有任何任务，且 headline 不是「可以导出」", () => {
    const summary = buildUserTasks(undefined, [issue("i1", "ANSWER_MISSING", "q11")]);
    expect(summary.ready).toBe(false);
    expect(summary.tasks).toHaveLength(0);
    expect(summary.blockerCount).toBe(0);
    // 关键：这里的「空」不能读成「没问题」。
    expect(summary.headline).not.toBe("可以导出");
  });

  it("同一批问题在草稿就绪后落到题号上，而不是退化成 unnumbered", () => {
    const issues = [issue("i1", "ANSWER_MISSING", "q11"), issue("i2", "ANSWER_MISSING", "q12")];
    // 草稿没来：一条任务都不生成（而不是生成一张点不动的 `unnumbered` 卡）。
    const before = buildUserTasks(undefined, issues);
    expect(before.ready).toBe(false);
    expect(before.tasks).toHaveLength(0);
    // 草稿来了：同样的问题落到具体题号上，动作目标也是真实存在的题位。
    const after = buildUserTasks(DS, issues);
    expect(after.ready).toBe(true);
    expect(after.tasks).toHaveLength(1);
    expect(after.tasks[0].taskId).toBe("missing-answer:q11+q12");
    expect(after.tasks[0].actions[0].targetId).toBe("q11");
  });

  it("有草稿且确实没有问题 → ready=true，这时才说「没有需要补充的内容」", () => {
    const summary = buildUserTasks(DS, []);
    expect(summary.ready).toBe(true);
    expect(summary.tasks).toHaveLength(0);
    expect(summary.headline).toBe("没有需要补充的内容");
  });
});

describe("唯一一份编辑辅助清单：本地 + 发布前检查 + 云端剩下的，每个题位只出一条", () => {
  it("云端剩下的「q12 缺答案」与本地缺答撞在同一题位：只出一条", () => {
    const summary = buildEditingAids(DS, [issue("i1", "ANSWER_MISSING", "q12")], [
      { userTaskId: "quality:ANSWER_KEY_MISSING_SLOT:q12", targetIds: ["q12"], message: "计分 slot 没有可验证的答案 key。" }
    ]);
    expect(summary.tasks).toHaveLength(1);
    expect(summary.tasks[0].title).toBe("第 12 题缺少答案");
  });

  it("用户补上之后（本地与云端重算都不再报）这一条消失", () => {
    const summary = buildEditingAids(DS, [], []);
    expect(summary.tasks).toHaveLength(0);
    expect(summary.headline).toBe("没有需要补充的内容");
  });

  it("后端内部词不进界面：质量码走本地话术，差异写成「现在是 X，云端读到的是 Y」", () => {
    const summary = buildEditingAids(DS, [], [
      { userTaskId: "quality:SLOT_GROUP_ASSIGNMENT_INVALID:rg-1", targetIds: ["rg-1"], message: "题组的 expected question numbers 与 slots 不一致" },
      {
        userTaskId: "cloud-diff:slot:q13:answer",
        targetIds: ["q13"],
        message: "第 q13 题的答案与云端识别结果不一致",
        field: "answer",
        currentValue: { kind: "text", values: ["river"] },
        cloudValue: { kind: "text", values: ["rivers"] }
      },
      { userTaskId: "cloud-coverage:pdf-1:3:0", targetIds: [], message: "原文件第 3 页云端未能读全（PAGE_UNREADABLE）：ocr failed" }
    ]);
    const text = summary.tasks.map((task) => `${task.title} ${task.detail ?? ""}`).join("\n");
    expect(text).not.toMatch(/expected question numbers|slot|q13|PAGE_UNREADABLE|ocr failed/);
    expect(text).toContain("第 13 题的答案：现在是「river」，云端读到的是「rivers」");
    expect(text).toContain("原文件第 3 页");
  });

  it("「云端没能拿到足够的原文」与「查过但定不了论」在界面上是两句不同的话", () => {
    const [insufficient, undecided] = buildEditingAids(DS, [], [
      {
        userTaskId: "cloud-diff:slot:q13:answer",
        targetIds: ["q13"],
        message: "云端没能拿到足够的原文来判断第 13 题，请对照原文确认",
        field: "answer",
        currentValue: { kind: "text", values: ["river"] },
        cloudValue: { kind: "text", values: ["rivers"] },
        contextInsufficient: true
      },
      {
        userTaskId: "cloud-diff:slot:q12:answer",
        targetIds: ["q12"],
        message: "第 12 题的答案与云端识别结果不一致；云端已查过原文件但无法定论：答案区被裁掉",
        field: "answer",
        currentValue: { kind: "text", values: ["B"] },
        cloudValue: { kind: "text", values: ["A"] }
      }
    ]).tasks;

    // 材料没到手：说「没能拿到足够的原文」，且**不**说成「对照过原文件」。
    expect(insufficient.detail).toContain("没能拿到足够的原文");
    expect(insufficient.detail).not.toContain("对照原文件后没能定论");
    // 看过了但定不了：反过来。
    expect(undecided.detail).toContain("对照原文件后没能定论");
    expect(undecided.detail).not.toContain("没能拿到足够的原文");
    // 两者的标题仍然如实说「现在是什么、云端读到的是什么」。
    expect(insufficient.title).toBe("第 13 题的答案：现在是「river」，云端读到的是「rivers」");
    expect(undecided.title).toBe("第 12 题的答案：现在是「B」，云端读到的是「A」");
  });

  it("没有门槛话术：不出现「不能导出」「阻断」「可以导出」", () => {
    const summary = buildEditingAids(DS, [
      issue("i1", "ANSWER_MISSING", "q11"),
      issue("s1", "RUNTIME_COMPILER_FAILED", "document", "RUNTIME_COMPILER_FAILED"),
      issue("x1", "SOMETHING_BRAND_NEW", "q13")
    ], []);
    const text = [summary.headline, ...summary.tasks.flatMap((task) => [task.title, task.detail ?? "", ...task.actions.map((a) => a.label)])].join("\n");
    expect(text).not.toMatch(/导出|阻断|发不出去|重新识别/);
  });
});

// 听力分段差异：云端可以改分界（把两段并成一段、或拆开），但用户必须看懂**哪一段**
// 差在哪、能去核对原文。后端给的 `part-5` / `part_boundary` 是内部身份，不进界面。
const LISTENING_DS = {
  ...makeDs({
    taskGroups: [task("task-3", [group("rg-3", ["q21", "q22", "q23"])])],
    answerSlots: { q21: slot("q21", 21), q22: slot("q22", 22), q23: slot("q23", 23) }
  }),
  modality: "listening",
  listening: {
    scope: "complete_exam",
    media: null,
    parts: [
      { partId: "part-3", displayLabel: "SECTION 3", expectedQuestionNumbers: [21, 22, 23], taskIds: ["task-3"] },
      { partId: "part-4", displayLabel: "SECTION 4", expectedQuestionNumbers: [31, 32, 33], taskIds: ["task-4"] }
    ],
    playbackPolicy: { mode: "practice" },
    transcript: null
  }
} as unknown as IeltsAuthoringIRV2;

describe("听力分段差异：说得出是哪一段、差在哪", () => {
  it("云端新加了一段 → 标题里有那一段的名字与题号范围", () => {
    const summary = buildEditingAids(LISTENING_DS, [], [
      {
        userTaskId: "cloud-diff:part:part-5:part_boundary",
        targetIds: ["part-5"],
        message: "听力 Part part-5的分段范围与云端识别结果不一致",
        field: "part_boundary",
        currentValue: null,
        cloudValue: {
          partId: "part-5",
          displayLabel: "SECTION 3",
          expectedQuestionNumbers: [21, 22, 23],
          taskIds: ["task-3", "task-4"]
        }
      }
    ]);
    expect(summary.tasks).toHaveLength(1);
    const [row] = summary.tasks;
    expect(row.kind).toBe("cloud-difference");
    expect(row.title).toContain("分段");
    expect(row.title).toContain("SECTION 3");
    expect(row.title).toContain("第 21–23 题");
    const text = row.title + " " + (row.detail ?? "");
    expect(text).not.toMatch(/part-5|part_boundary|targetType|cloud-diff|task-3/);
    expect(row.actions.map((action) => action.id)).toEqual(["view-source"]);
  });

  it("被并掉的那一段（当前稿里还在）也说得出是哪一段", () => {
    const summary = buildEditingAids(LISTENING_DS, [], [
      {
        userTaskId: "cloud-diff:part:part-4:part_boundary",
        targetIds: ["part-4"],
        message: "听力 Part part-4的分段范围与云端识别结果不一致",
        field: "part_boundary",
        currentValue: {
          partId: "part-4",
          displayLabel: "SECTION 4",
          expectedQuestionNumbers: [31, 32, 33],
          taskIds: ["task-4"]
        },
        cloudValue: null
      }
    ]);
    expect(summary.tasks).toHaveLength(1);
    expect(summary.tasks[0].title).toContain("SECTION 4");
    expect(summary.tasks[0].title).toContain("第 31–33 题");
  });

  it("分段的名称变了也走分段话术，而不是「这一处的内容」", () => {
    const summary = buildEditingAids(LISTENING_DS, [], [
      {
        userTaskId: "cloud-diff:part:part-3:part_label",
        targetIds: ["part-3"],
        message: "听力 Part part-3的段落标签与云端识别结果不一致",
        field: "part_label",
        currentValue: "SECTION 3",
        cloudValue: "SECTION THREE"
      }
    ]);
    expect(summary.tasks).toHaveLength(1);
    const [row] = summary.tasks;
    expect(row.title).toContain("SECTION 3（第 21–23 题）");
    expect(row.title).toContain("分段名称");
  });
});