import { describe, expect, it } from "vitest";
import type { RecognitionDecisionItemV1, RecognitionDecisionViewV1 } from "../../api/recognitionClient";
import {
  autoFixedItems,
  canAccept,
  decisionActionLabel,
  decisionTargetId,
  describeStaleness,
  emptyStateMessage,
  groupByDependency,
  hasAnyChainRun,
  isDecided,
  isUndoAlreadyApplied,
  parseUndoPatch,
  pendingDecisionCount,
  reviewItems,
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
    summary: { agreed: 0, autoFixed: 0, needsReview: 0, unverifiable: 0 },
    items,
    ...partial
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

  it("批次过期时给出可读提示并带上两个版本号", () => {
    const v = view([], { stale: true, baseEditVersion: 7, currentEditVersion: 9 });
    const text = describeStaleness(v);
    expect(text).toContain("v7");
    expect(text).toContain("v9");
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
// 后端**没有**持久化的「已撤销」状态码（`RecognitionResolutionV1` 只有
// agreed/auto_fixed/needs_review/unverifiable），所以判据只能取自**权威稿本身**：
// 撤销补丁说「改回哪个值」，稿里那个答案位已经等于它 → 撤销已生效。
// 这是持久化事实，重开/刷新后同样成立；会话内 Set 一刷新就没了。
describe("isUndoAlreadyApplied — 撤销是否生效只看权威稿，不看会话状态", () => {
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
