import { describe, expect, it } from "vitest";
import type { RecognitionDecisionItemV1, RecognitionDecisionViewV1 } from "../../api/recognitionClient";
import {
  autoFixedItems,
  canAccept,
  decisionActionLabel,
  decisionStatusLabel,
  decisionTargetId,
  describeStaleness,
  emptyStateMessage,
  groupByDependency,
  hasAnyChainRun,
  isDecided,
  isRecognitionQuiet,
  isUndoAlreadyApplied,
  parseUndoPatch,
  pendingDecisionCount,
  recognitionInFlight,
  reviewItems,
  undoState,
  visibleDecisionItems
} from "./recognitionDecisions";

// 证据层级：pure unit。断言的是契约 §2.4 / §2.7 的**呈现义务**：
// 一致项不出现、info 不出现、无法验证不给「接受」、依赖组必须整组同向。

function item(partial: Partial<RecognitionDecisionItemV1> & Pick<RecognitionDecisionItemV1, "decisionId" | "resolution">): RecognitionDecisionItemV1 {
  return {
    code: "ANSWER_CONFLICT",
    severity: "warning",
    title: "t",
    userMessage: "m",
    target: { targetType: "slot", targetId: "slot-14" },
    field: "answer",
    evidence: [],
    status: "open",
    ...partial
  };
}

function view(items: RecognitionDecisionItemV1[], partial: Partial<RecognitionDecisionViewV1> = {}): RecognitionDecisionViewV1 {
  return {
    schemaVersion: "RecognitionDecisionViewV1",
    itemId: "item-1",
    batchId: "batch-1",
    baseEditVersion: 7,
    currentEditVersion: 7,
    stale: false,
    localStatus: "succeeded",
    cloudStatus: "succeeded",
    // 四路链状态都在视图里。默认给「都跑完了」，个别用例再按需覆盖成 partial / not_run。
    sourceStatus: "succeeded",
    adjudicationStatus: "succeeded",
    summary: { agreed: 0, autoFixed: 0, needsReview: 0, unverifiable: 0 },
    items,
    ...partial,
    // `Partial` 会让 `repair` 变成 `undefined`，而视图契约要求「没有修复记录」是显式的
    // `null`。补一道归一，免得测试用 `undefined` 表达这个意思。
    repair: partial.repair ?? null
  };
}

describe("visibleDecisionItems — 哪些不该出现在问题列表", () => {
  it("agreed 不产生逐项问题", () => {
    const v = view([item({ decisionId: "d1", resolution: "agreed" }), item({ decisionId: "d2", resolution: "needs_review" })]);
    expect(visibleDecisionItems(v).map((i) => i.decisionId)).toEqual(["d2"]);
  });

  it("severity=info 不进主问题列表", () => {
    const v = view([item({ decisionId: "d1", resolution: "needs_review", severity: "info" })]);
    expect(visibleDecisionItems(v)).toEqual([]);
  });

  it("superseded 不再展示", () => {
    const v = view([item({ decisionId: "d1", resolution: "needs_review", status: "superseded" })]);
    expect(visibleDecisionItems(v)).toEqual([]);
  });

  it("undefined / 空 view 返回空数组", () => {
    expect(visibleDecisionItems(undefined)).toEqual([]);
    expect(visibleDecisionItems(view([]))).toEqual([]);
  });
});

describe("auto_fixed 与 needs_review 分流", () => {
  const v = view([
    item({ decisionId: "a1", resolution: "auto_fixed" }),
    item({ decisionId: "r1", resolution: "needs_review" }),
    item({ decisionId: "u1", resolution: "unverifiable" })
  ]);

  it("auto_fixed 单独成组（顶部展示、默认折叠）", () => {
    expect(autoFixedItems(v).map((i) => i.decisionId)).toEqual(["a1"]);
  });

  it("needs_review 与 unverifiable 都属于「需要用户处理」", () => {
    expect(reviewItems(v).map((i) => i.decisionId)).toEqual(["r1", "u1"]);
  });
});

describe("groupByDependency — 依赖组必须整组同向", () => {
  it("相同 dependencyGroup 合成一组，独立项 groupId 为 null", () => {
    const groups = groupByDependency([
      item({ decisionId: "d1", resolution: "needs_review", dependencyGroup: "dep:q14-q15" }),
      item({ decisionId: "d2", resolution: "needs_review", dependencyGroup: "dep:q14-q15" }),
      item({ decisionId: "d3", resolution: "needs_review" })
    ]);
    expect(groups).toHaveLength(2);
    expect(groups[0].groupId).toBe("dep:q14-q15");
    expect(groups[0].items.map((i) => i.decisionId)).toEqual(["d1", "d2"]);
    expect(groups[1].groupId).toBeNull();
    expect(groups[1].items.map((i) => i.decisionId)).toEqual(["d3"]);
  });
});

describe("canAccept — 无法验证的项不允许强行选一个版本", () => {
  it("unverifiable 不能接受", () => {
    expect(canAccept(item({ decisionId: "u", resolution: "unverifiable" }))).toBe(false);
  });

  it("open 的 needs_review 可以接受", () => {
    expect(canAccept(item({ decisionId: "r", resolution: "needs_review" }))).toBe(true);
  });

  it("已经决策过的项不再可接受（重复点击幂等）", () => {
    expect(canAccept(item({ decisionId: "r", resolution: "needs_review", status: "accepted" }))).toBe(false);
    expect(canAccept(item({ decisionId: "r", resolution: "needs_review", status: "rejected" }))).toBe(false);
    expect(canAccept(item({ decisionId: "r", resolution: "needs_review", status: "failed" }))).toBe(false);
  });

  it("按钮文案：无法验证只给「保持现状」", () => {
    expect(decisionActionLabel(item({ decisionId: "u", resolution: "unverifiable" }))).toEqual({ accept: "", keep: "保持现状" });
    expect(decisionActionLabel(item({ decisionId: "r", resolution: "needs_review" }))).toEqual({ accept: "采用修正", keep: "保持现状" });
  });

  it("isDecided 覆盖三种终态", () => {
    expect(isDecided(item({ decisionId: "x", resolution: "needs_review", status: "open" }))).toBe(false);
    for (const status of ["accepted", "rejected", "failed"] as const) {
      expect(isDecided(item({ decisionId: "x", resolution: "needs_review", status }))).toBe(true);
    }
  });
});

describe("decisionTargetId — 定位到题面", () => {
  it("优先 nodeId，其次 targetId，再次 taskId", () => {
    expect(decisionTargetId(item({ decisionId: "a", resolution: "needs_review", target: { targetType: "node", targetId: "n1", nodeId: "n9" } }))).toBe("n9");
    expect(decisionTargetId(item({ decisionId: "b", resolution: "needs_review", target: { targetType: "slot", targetId: "slot-3" } }))).toBe("slot-3");
    expect(decisionTargetId(item({ decisionId: "c", resolution: "needs_review", target: { targetType: "task", targetId: "", taskId: "task-7" } }))).toBe("task-7");
  });
});

describe("待处理计数与过期提示", () => {
  it("只统计 open 的需要用户处理项", () => {
    const v = view([
      item({ decisionId: "r1", resolution: "needs_review" }),
      item({ decisionId: "r2", resolution: "needs_review", status: "rejected" }),
      item({ decisionId: "u1", resolution: "unverifiable" }),
      item({ decisionId: "a1", resolution: "auto_fixed" }),
      item({ decisionId: "g1", resolution: "agreed" })
    ]);
    expect(pendingDecisionCount(v)).toBe(2);
  });

  it("批次过期时给出可读提示，且**不带版本号**（版本号不进普通界面）", () => {
    const v = view([], { stale: true, baseEditVersion: 7, currentEditVersion: 9 });
    const text = describeStaleness(v);
    expect(text).toBeTruthy();
    // 本轮任务书第一节：`v1/v2/v3`、批次基线、editVersion 这类内部版本信息不得出现在普通界面。
    expect(text).not.toContain("v7");
    expect(text).not.toContain("v9");
    expect(text).not.toMatch(/\bv\d+\b/);
    // 但必须说清「不会覆盖你的改动」这个用户真正关心的事实。
    expect(text).toContain("不会覆盖");
  });

  it("没过期就没有提示", () => {
    expect(describeStaleness(view([]))).toBeUndefined();
    expect(describeStaleness(undefined)).toBeUndefined();
  });
});

describe("hasAnyChainRun / emptyStateMessage — 空列表不能冒充「没有问题」", () => {
  it("四路都没跑时，说的是「还没有可核对的结果」", () => {
    const v = view([], { localStatus: "not_started", cloudStatus: "not_started" });
    expect(hasAnyChainRun(v)).toBe(false);
    expect(emptyStateMessage(v)).toContain("还没有产出");
  });

  it("至少一路跑过且没有待处理项时，才说「没有问题」", () => {
    const v = view([], { localStatus: "succeeded", cloudStatus: "not_started" });
    expect(hasAnyChainRun(v)).toBe(true);
    expect(emptyStateMessage(v)).toBe("识别结果没有需要你确认的地方。");
  });

  it("有待处理项但都已决策时，说「都已经处理过了」", () => {
    const v = view([item({ decisionId: "r1", resolution: "needs_review", status: "rejected" })]);
    expect(emptyStateMessage(v)).toBe("这些问题都已经处理过了。");
  });

  it("undefined view 走「还没有可核对的结果」", () => {
    expect(hasAnyChainRun(undefined)).toBe(false);
    expect(emptyStateMessage(undefined)).toContain("还没有产出");
  });
});

// 空面板收敛（本轮任务书第二节）：四颗全零的计数胶囊不携带信息，
// 却要从题稿身上拿走一整块面板高度。收敛的判据必须**保守**——
// 漏收敛只是多占一点空间，误收敛会把该给用户看的东西藏起来。
describe("isRecognitionQuiet — 只有真的没什么可说时才收敛成一行", () => {
  it("没有任何条目、四个计数全零 → 收敛", () => {
    expect(isRecognitionQuiet(view([]))).toBe(true);
  });

  it("有待确认的条目 → 不收敛", () => {
    expect(isRecognitionQuiet(view([item({ decisionId: "r1", resolution: "needs_review" })]))).toBe(false);
  });

  it("无法验证的条目 → 不收敛", () => {
    expect(isRecognitionQuiet(view([item({ decisionId: "u1", resolution: "unverifiable" })]))).toBe(false);
  });

  it("已自动修正的条目 → 不收敛（撤销入口必须还在）", () => {
    const v = view([item({ decisionId: "a1", resolution: "auto_fixed" })], {
      summary: { agreed: 0, autoFixed: 1, needsReview: 0, unverifiable: 0 }
    });
    expect(isRecognitionQuiet(v)).toBe(false);
  });

  it("条目都已决策（rejected）也仍然要显示处理结果 → 不收敛", () => {
    const v = view([item({ decisionId: "r1", resolution: "needs_review", status: "rejected" })]);
    expect(isRecognitionQuiet(v)).toBe(false);
  });

  it("条目为空但计数非零 → 不收敛（计数本身就是要给用户看的信息）", () => {
    const v = view([], { summary: { agreed: 12, autoFixed: 0, needsReview: 0, unverifiable: 0 } });
    expect(isRecognitionQuiet(v)).toBe(false);
  });

  it("批次过期 → 不收敛（过期提示必须被看到）", () => {
    const v = view([], { stale: true });
    expect(isRecognitionQuiet(v)).toBe(false);
  });

  it("拿不到视图 → 不收敛（那是「不知道」，不是「没什么可说」）", () => {
    expect(isRecognitionQuiet(undefined)).toBe(false);
  });
});

// 撤销必须走编辑器事务（契约 §4.4 / §6.1）。
// 这里曾经把「撤销」实现成 reject 决策，而后端 reject 只改状态、不碰权威稿，
// 于是界面说「已保持现状」而自动修正仍留在稿里 —— 假完成。
describe("parseUndoPatch — 撤销补丁必须能被真实应用，认不出来就不给按钮", () => {
  it("后端 undo_patch_for 产出的 setAnswer 形状被接受", () => {
    const patch = parseUndoPatch({ op: "setAnswer", slotId: "slot-14", value: { kind: "unresolved" }, preserveProvenance: true });
    expect(patch).toEqual({ op: "setAnswer", slotId: "slot-14", value: { kind: "unresolved" } });
  });

  it("文本答案的旧值原样保留", () => {
    const patch = parseUndoPatch({ op: "setAnswer", slotId: "slot-3", value: { kind: "text", values: ["maps"] } });
    expect(patch).toEqual({ op: "setAnswer", slotId: "slot-3", value: { kind: "text", values: ["maps"] } });
  });

  it("缺 slotId / value 不是对象 / op 不是 setAnswer 都返回 undefined", () => {
    expect(parseUndoPatch({ op: "setAnswer", value: { kind: "unresolved" } })).toBeUndefined();
    expect(parseUndoPatch({ op: "setAnswer", slotId: "slot-1", value: "unresolved" })).toBeUndefined();
    expect(parseUndoPatch({ op: "deleteNode", nodeId: "n1" })).toBeUndefined();
    expect(parseUndoPatch({ op: "setAnswer", slotId: "", value: { kind: "unresolved" } })).toBeUndefined();
  });

  it("null / undefined / 数组一律不可撤销", () => {
    expect(parseUndoPatch(null)).toBeUndefined();
    expect(parseUndoPatch(undefined)).toBeUndefined();
    expect(parseUndoPatch([])).toBeUndefined();
    expect(parseUndoPatch("undo")).toBeUndefined();
  });

  it("auto_fixed 项缺 undo 时不产生撤销补丁（界面因此不给按钮）", () => {
    const v = view([item({ decisionId: "a1", resolution: "auto_fixed" })]);
    const [auto] = autoFixedItems(v);
    expect(parseUndoPatch(auto.undo)).toBeUndefined();
  });
});

// 任务书：「接入后端持久化撤销，不再以会话内 Set 作为完成依据。」
// 会话内 Set 一刷新就没了，不能当判据 —— 判据必须来自**持久化事实**。
// 后端本轮新增了 `DecisionStatusV1::Undone`（撤销与回滚在同一编辑事务里落盘），
// 所以**首选**判据是 `status === "undone"`（见 `undoState`）。
// `isUndoAlreadyApplied` 退居次要：兜住废弃编辑器补丁路径写下的历史数据
// （只回滚了权威稿、状态仍停在 `accepted`，只能靠稿里的值认出来）。
describe("isUndoAlreadyApplied — 权威稿的值是否已等于撤销目标（次要判据）", () => {
  const undo = { op: "setAnswer", slotId: "slot-3", value: { kind: "text", values: ["maps"] } };

  it("稿里的值已等于撤销目标值 → 已撤销（重开后同样成立）", () => {
    expect(isUndoAlreadyApplied(undo, { "slot-3": { kind: "text", values: ["maps"] } })).toBe(true);
  });

  it("稿里的值还是自动修正后的值 → 未撤销（要给按钮）", () => {
    expect(isUndoAlreadyApplied(undo, { "slot-3": { kind: "text", values: ["diaries"] } })).toBe(false);
  });

  it("看的是补丁指定的那个答案位，别的位相同不算", () => {
    expect(isUndoAlreadyApplied(undo, { "slot-4": { kind: "text", values: ["maps"] } })).toBe(false);
  });

  it("选项位按 labels 比，不比 values", () => {
    const optionUndo = { op: "setAnswer", slotId: "q1", value: { kind: "option", labels: ["TRUE"] } };
    expect(isUndoAlreadyApplied(optionUndo, { q1: { kind: "option", labels: ["TRUE"] } })).toBe(true);
    expect(isUndoAlreadyApplied(optionUndo, { q1: { kind: "option", labels: ["FALSE"] } })).toBe(false);
  });

  it("kind 不同不算同一个值（text vs option）", () => {
    expect(isUndoAlreadyApplied(undo, { "slot-3": { kind: "option", labels: ["maps"] } })).toBe(false);
  });

  it("答案位缺失 / 稿为空 → 未撤销（不谎报已撤销）", () => {
    expect(isUndoAlreadyApplied(undo, {})).toBe(false);
    expect(isUndoAlreadyApplied(undo, undefined)).toBe(false);
  });

  it("撤销补丁本身不可解析 → 未撤销（界面会走「没有可撤销的信息」那条分支）", () => {
    expect(isUndoAlreadyApplied(null, { "slot-3": { kind: "text", values: ["maps"] } })).toBe(false);
    expect(isUndoAlreadyApplied({ op: "deleteNode", nodeId: "n1" }, {})).toBe(false);
  });
});

// 撤销入口的判据优先级：**后端持久化状态**优先于权威稿的值。
// 这是本轮「撤销按钮改调正式后端撤销命令」的直接配套 —— 命令成功后后端把状态写成
// `undone`，界面必须据此收掉按钮；只靠值比对会漏掉「值恰好相同」以外的所有情形。
describe("undoState — 撤销入口该显示成什么", () => {
  const undo = { op: "setAnswer", slotId: "slot-3", value: { kind: "text", values: ["maps"] } };
  const autoFixed = (partial: Partial<RecognitionDecisionItemV1> = {}) =>
    item({ decisionId: "a1", resolution: "auto_fixed", undo, status: "accepted", ...partial });

  it("后端已持久化 undone → 已撤销（哪怕权威稿的值还没刷新过来）", () => {
    // 关键：answerKey 仍是自动修正后的值，但状态说已撤销 —— 状态赢。
    expect(undoState(autoFixed({ status: "undone" }), { "slot-3": { kind: "text", values: ["diaries"] } })).toBe("undone");
  });

  it("状态未更新但权威稿已等于撤销目标 → 已撤销（兜住废弃编辑器补丁路径的历史数据）", () => {
    expect(undoState(autoFixed({ status: "accepted" }), { "slot-3": { kind: "text", values: ["maps"] } })).toBe("undone");
  });

  it("有撤销补丁、值也还没回去 → 给按钮", () => {
    expect(undoState(autoFixed({ status: "accepted" }), { "slot-3": { kind: "text", values: ["diaries"] } })).toBe("available");
  });

  it("没有撤销补丁 → 不可撤销（不给按了不生效的按钮）", () => {
    expect(undoState(item({ decisionId: "a2", resolution: "auto_fixed", status: "accepted" }), undefined)).toBe("unavailable");
  });

  it("值比对只对 auto_fixed 生效：待确认项不会被误报成已撤销", () => {
    // 用户自己把某个待确认项的答案位改成了与撤销目标相同的值 —— 那不是「撤销已生效」。
    const review = item({ decisionId: "r1", resolution: "needs_review", undo, status: "open" });
    expect(undoState(review, { "slot-3": { kind: "text", values: ["maps"] } })).toBe("available");
  });

  it("用户改过目标时**仍然给按钮**：保护在后端，让用户看到那句话", () => {
    // 后端会以 `USER_EDITED_AFTER_APPLY` 拒绝并给出文案。悄悄藏起按钮反而让用户
    // 不知道「我的修改赢了」。
    expect(undoState(autoFixed(), { "slot-3": { kind: "text", values: ["我自己的答案"] } })).toBe("available");
  });
});

describe("isDecided — undone 是已解决，不是失败", () => {
  it("undone 算已决策（不能再重复操作）", () => {
    expect(isDecided(item({ decisionId: "d1", resolution: "auto_fixed", status: "undone" }))).toBe(true);
  });

  it("open 不算已决策", () => {
    expect(isDecided(item({ decisionId: "d2", resolution: "needs_review", status: "open" }))).toBe(false);
  });
});

describe("decisionStatusLabel — 已撤销不能被显示成「处理失败」", () => {
  // 这里曾经是嵌套三元：`undone` 掉进 else 分支 → 用户成功撤销了，界面却报错。
  it("undone 有自己的文案，且不含「失败」", () => {
    const label = decisionStatusLabel(item({ decisionId: "d1", resolution: "auto_fixed", status: "undone" }));
    expect(label).toContain("已撤销");
    expect(label).not.toContain("失败");
  });

  it("accepted / rejected 文案不变", () => {
    expect(decisionStatusLabel(item({ decisionId: "d2", resolution: "auto_fixed", status: "accepted" }))).toBe("已采用");
    expect(decisionStatusLabel(item({ decisionId: "d3", resolution: "needs_review", status: "rejected" }))).toBe("已保持现状");
  });

  it("failed 显示为「处理失败，请重试」，**不带错误码**", () => {
    // 本轮任务书第一节：错误码（`APPLY_REJECTED` / `USER_EDITED_AFTER_APPLY`）是给日志和
    // 开发者看的，普通界面出现这种词只会让用户困惑。失败必须给出**下一步动作**。
    const label = decisionStatusLabel(item({ decisionId: "d4", resolution: "needs_review", status: "failed", code: "APPLY_REJECTED" }));
    expect(label).toBe("处理失败，请重试");
    expect(label).not.toContain("APPLY_REJECTED");
  });
});

describe("recognitionInFlight — 结果还在路上时面板不能把「没有结果」定格", () => {
  // 回归 F-R14-1：批次是裁决之后才落盘的，面板通常先于批次打开；批次落地那一刻
  // 外层重拉键的四个分量全都不变（`job.currentStep` 早已是 Authoring），于是面板
  // 会一直显示「识别还没有产出可核对的结果」，而 IPC 已经能读到几十条候选。
  it("一次都没读到视图 ⇒ 在途（可能是识别还没落盘，也可能是读命令失败）", () => {
    expect(recognitionInFlight(undefined)).toBe(true);
  });

  it("没有批次 ⇒ 在途：「没有批次」只说明结论还没生成", () => {
    // 后端在无批次时如实返回空视图（`get_recognition_decision_core`），
    // 因此 batchId 为空是**未生成**，不是「没有问题」。
    expect(recognitionInFlight(view([], { batchId: "" }))).toBe(true);
  });

  it("本地/云端在排队或运行 ⇒ 在途", () => {
    expect(recognitionInFlight(view([], { localStatus: "running" }))).toBe(true);
    expect(recognitionInFlight(view([], { cloudStatus: "queued" }))).toBe(true);
    // 「本地先出稿、云端仍排队」是最常见的落点：本地已 succeeded，云端还 queued。
    expect(recognitionInFlight(view([], { localStatus: "succeeded", cloudStatus: "queued" }))).toBe(true);
  });

  it("批次已到且两条链都到终态 ⇒ 不在途（不再空转轮询）", () => {
    expect(recognitionInFlight(view([], { localStatus: "succeeded", cloudStatus: "succeeded" }))).toBe(false);
    // 云端没跑（not_run→not_started）也是终态：不能因为「云端没跑」就无限轮询。
    expect(recognitionInFlight(view([], { localStatus: "succeeded", cloudStatus: "not_started" }))).toBe(false);
    expect(recognitionInFlight(view([], { localStatus: "succeeded", cloudStatus: "failed" }))).toBe(false);
  });
});
