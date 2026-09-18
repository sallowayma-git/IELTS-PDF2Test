import { describe, expect, it, vi } from "vitest";
import {
  canUndoRepair,
  describeRepairStatus,
  describeVerificationStatus,
  normalizeDecisionView
} from "./recognitionClient";

// `command` 必须被 mock 掉，否则单测会去碰真实 Tauri IPC。
// 用 vi.hoisted 是因为 vi.mock 的工厂会被提升到 import 之前。
const { commandMock } = vi.hoisted(() => ({ commandMock: vi.fn() }));
vi.mock("./tauriCommands", () => ({ command: commandMock }));

// 证据层级：pure unit。断言两件事：
//   1) 后端**实现形状**（chains/actionable/autoApplied）与**契约形状**（cloudStatus/items）
//      都能被归一，不会出现 `云端核验状态未知（undefined）` 这种假信息；
//   2) 缺失字段一律降级成「没跑」，绝不猜成「完成」。

describe("normalizeDecisionView — 契约形状", () => {
  it("按契约字段直通", () => {
    const view = normalizeDecisionView({
      itemId: "item-1",
      batchId: "batch-1",
      baseEditVersion: 7,
      currentEditVersion: 9,
      stale: true,
      localStatus: "succeeded",
      cloudStatus: "unavailable",
      cloudReasonCode: "NO_PROFILE",
      summary: { agreed: 12, autoFixed: 2, needsReview: 1, unverifiable: 3 },
      items: [{ decisionId: "d1", resolution: "needs_review" } as never]
    });
    expect(view.cloudStatus).toBe("unavailable");
    expect(view.localStatus).toBe("succeeded");
    expect(view.currentEditVersion).toBe(9);
    expect(view.stale).toBe(true);
    expect(view.items).toHaveLength(1);
    expect(view.summary).toEqual({ agreed: 12, autoFixed: 2, needsReview: 1, unverifiable: 3 });
  });
});

describe("normalizeDecisionView — 实现形状（chains / actionable / autoApplied）", () => {
  it("chains.state 映射成契约状态，不产生 undefined", () => {
    const view = normalizeDecisionView({
      itemId: "item-1",
      batchId: null,
      editVersion: 0,
      chains: {
        local: { state: "done" },
        cloud: { state: "not_run" },
        source: { state: "not_run" },
        adjudication: { state: "not_run" }
      },
      actionable: [],
      autoApplied: []
    });
    expect(view.localStatus).toBe("succeeded");
    expect(view.cloudStatus).toBe("not_started");
    // 四路链状态都必须被带上来：丢掉 source/adjudication 就只能把「没核验」说成「核验完成」。
    expect(view.sourceStatus).toBe("not_started");
    expect(view.adjudicationStatus).toBe("not_started");
    expect(view.items).toEqual([]);
    // 关键：绝不能是 undefined，否则界面会渲染「未知（undefined）」。
    const text = describeVerificationStatus(view);
    expect(text).not.toContain("undefined");
    expect(text).not.toContain("校验完成");
  });

  it("actionable 与 autoApplied 合成 items，autoApplied 标成 auto_fixed", () => {
    const view = normalizeDecisionView({
      itemId: "item-1",
      chains: { cloud: { state: "failed" } },
      actionable: [{ decisionId: "r1", resolution: "needs_review" } as never],
      autoApplied: [{ decisionId: "a1" } as never]
    });
    expect(view.items.map((item) => item.decisionId)).toEqual(["a1", "r1"]);
    expect(view.items[0].resolution).toBe("auto_fixed");
    expect(view.cloudStatus).toBe("failed");
    expect(view.summary.autoFixed).toBe(1);
    expect(view.summary.needsReview).toBe(1);
  });

  it("完全没有状态字段时降级成 not_started，而不是抛错或猜成完成", () => {
    const view = normalizeDecisionView({ itemId: "item-1" });
    expect(view.cloudStatus).toBe("not_started");
    expect(view.localStatus).toBe("not_started");
    expect(view.currentEditVersion).toBe(0);
    expect(view.items).toEqual([]);
  });
});

// 核验状态行（`describeVerificationStatus`）——本轮最重要的契约同步点。
//
// 上一版只有一个 `describeCloudStatus(cloudStatus)`：只要云端跑完就说「云端核验完成。」。
// A3/A4 落地后这句话是**假的**——`chains.source` 会 `partial`（模型通道失败 / 预算耗尽），
// `chains.adjudication` 会 `not_run`（有分歧但没有模型可用）。而且「一条待处理项都没有」
// 也**不等于**核验成功：没有卡片只说明没有要用户动手的东西。
describe("describeVerificationStatus — 只用用户能懂的几句话", () => {
  it("每个稳定状态都有明确文案，且不泄露内部词", () => {
    expect(describeVerificationStatus({ localStatus: "running" })).toBe("正在本机识别…");
    expect(describeVerificationStatus({ cloudStatus: "queued" })).toBe("云端正在校验…");
    expect(describeVerificationStatus({ cloudStatus: "not_started" })).toBe("题稿已生成，可以开始编辑");
    expect(describeVerificationStatus({ cloudStatus: "failed" })).toBe("云端校验暂时不可用，不影响继续编辑");
    expect(describeVerificationStatus({ cloudStatus: "unavailable" })).toBe("云端校验暂时不可用，不影响继续编辑");
    // 「完成」必须四路都跑完；只给 cloudStatus 时另外两路按 `not_started` 处理，
    // 于是**不会**承诺「没有发现问题」——这正是「空列表不等于核验成功」的默认姿态。
    expect(describeVerificationStatus({ cloudStatus: "succeeded" })).toBe("部分内容尚未完成校验");
    expect(describeVerificationStatus({
      cloudStatus: "succeeded",
      sourceStatus: "succeeded",
      adjudicationStatus: "succeeded"
    })).toBe("云端校验完成，没有发现需要处理的问题");
  });

  it("queued 说「正在校验」而不是「可以开始编辑」（本地先出稿时云端正是这个状态）", () => {
    const view = normalizeDecisionView({ itemId: "item-1", chains: { local: { state: "done" }, cloud: { state: "queued" } } });
    expect(view.cloudStatus).toBe("queued");
    const text = describeVerificationStatus(view);
    expect(text).toBe("云端正在校验…");
    expect(text).not.toContain("undefined");
  });

  it("「部分完成」显示为「部分内容尚未完成校验，请检查标出的题目」", () => {
    // source 链 partial（模型通道失败 / 预算耗尽）+ 有待处理项。
    const view = normalizeDecisionView({
      itemId: "item-1",
      chains: { local: { state: "done" }, cloud: { state: "succeeded" }, source: { state: "partial" }, adjudication: { state: "not_run" } },
      actionable: [{ decisionId: "d1", resolution: "needs_review" } as never]
    });
    expect(view.sourceStatus).toBe("partial");
    expect(view.adjudicationStatus).toBe("not_started");
    expect(describeVerificationStatus({ ...view, pendingCount: 1 })).toBe("部分内容尚未完成校验，请检查标出的题目");
  });

  it("**空列表不等于核验成功**：有链没跑完时绝不承诺「没有发现需要处理的问题」", () => {
    // 没有任何待处理项，但 adjudication 是 not_run（有分歧却没有模型可用）。
    const view = normalizeDecisionView({
      itemId: "item-1",
      chains: { local: { state: "done" }, cloud: { state: "succeeded" }, source: { state: "partial" }, adjudication: { state: "not_run" } },
      actionable: []
    });
    expect(view.items).toEqual([]);
    const text = describeVerificationStatus({ ...view, pendingCount: 0 });
    expect(text).toBe("部分内容尚未完成校验");
    expect(text).not.toContain("没有发现需要处理的问题");
  });

  it("只有相关检查确实完成、且没有待处理项时，才显示「校验完成，未发现需要处理的问题」", () => {
    const view = normalizeDecisionView({
      itemId: "item-1",
      chains: { local: { state: "done" }, cloud: { state: "succeeded" }, source: { state: "succeeded" }, adjudication: { state: "succeeded" } },
      actionable: []
    });
    expect(describeVerificationStatus({ ...view, pendingCount: 0 })).toBe("云端校验完成，没有发现需要处理的问题");
  });

  it("有待处理项且没有部分完成时，说「发现 N 处建议」", () => {
    expect(describeVerificationStatus({
      cloudStatus: "succeeded",
      sourceStatus: "succeeded",
      adjudicationStatus: "succeeded",
      pendingCount: 2
    })).toBe("云端发现 2 处建议");
  });

  it("adjudication 为 not_run 但 source 已完成、且没有待处理项时，不算「完成」", () => {
    // A4 的 not_run 在「没有分歧」时是**正常**的（没有分歧就无需裁决），
    // 但当前契约没有「有无分歧」这一位，所以只能保守地说「部分完成」，
    // 不能升级成「没有发现问题」。这条钉住这个保守选择，避免以后被"优化"掉。
    const text = describeVerificationStatus({
      cloudStatus: "succeeded",
      sourceStatus: "succeeded",
      adjudicationStatus: "not_started",
      pendingCount: 0
    });
    expect(text).not.toContain("没有发现需要处理的问题");
  });

  it("canceled 归入 not_started", () => {
    const view = normalizeDecisionView({ itemId: "item-1", chains: { cloud: { state: "canceled" } } });
    expect(view.cloudStatus).toBe("not_started");
  });

  it("reason code 只作内部分类，绝不进用户文案", () => {
    const view = normalizeDecisionView({
      itemId: "item-1",
      chains: {
        cloud: { state: "succeeded" },
        source: { state: "partial", reasonCode: "SOURCE_VERIFY_BUDGET_EXHAUSTED" },
        adjudication: { state: "partial", reasonCode: "ADJUDICATION_BUDGET_EXHAUSTED" }
      },
      actionable: []
    });
    // 原因码**确实**被带进视图（供内部分类与验收脚本比对）。
    expect(view.sourceReasonCode).toBe("SOURCE_VERIFY_BUDGET_EXHAUSTED");
    expect(view.adjudicationReasonCode).toBe("ADJUDICATION_BUDGET_EXHAUSTED");
    const text = describeVerificationStatus({ ...view, pendingCount: 0 });
    expect(text).not.toContain("SOURCE_VERIFY_BUDGET_EXHAUSTED");
    expect(text).not.toContain("ADJUDICATION_BUDGET_EXHAUSTED");
    expect(text).not.toContain("BUDGET");
  });
});

describe("applyRecognitionDecisions — IPC 参数包装（真实 E2E 才发现的一层漂移）", () => {
  // Tauri 命令签名是 `fn apply_recognition_decisions(input: Value, app: AppHandle)`，
  // 因此 IPC 参数必须整体包在 `input` 键里。字段名对齐但没包 `input` 时，真实后端报：
  //   invalid args `input` for command `apply_recognition_decisions`: missing required key input
  //
  // 契约漂移检查器只比对 Rust **结构体**字段名，看不到命令包装层，所以它当时报
  // 「0 处破坏性不一致」而写路径其实是坏的。这条测试把这个盲区钉住。
  it("请求整体包在 input 键里，且 accept/reject 由 decisions 派生", async () => {
    commandMock.mockReset();
    commandMock.mockResolvedValueOnce({
      schemaVersion: "ApplyRecognitionDecisionsResultV1",
      requestId: "r1",
      batchId: "b1",
      editVersionBefore: 3,
      editVersionAfter: 4,
      replayed: false,
      outcomes: [{ decisionId: "d1", kind: "applied", message: "ok" }],
      view: { itemId: "i1", batchId: "b1", editVersion: 4, chains: {}, actionable: [], autoApplied: [] }
    });
    const { applyRecognitionDecisions } = await import("./recognitionClient");
    const result = await applyRecognitionDecisions({
      itemId: "i1",
      batchId: "b1",
      baseEditVersion: 3,
      requestId: "r1",
      decisions: [
        { decisionId: "d1", action: "accept" },
        { decisionId: "d2", action: "reject" }
      ]
    });

    expect(commandMock).toHaveBeenCalledTimes(1);
    const [name, args] = commandMock.mock.calls[0] as [string, Record<string, unknown>];
    expect(name).toBe("apply_recognition_decisions");
    // 关键断言：顶层只能有 input 一个键，不能把 requestId/batchId/... 平铺上去。
    expect(Object.keys(args)).toEqual(["input"]);
    expect(args.input).toEqual({
      requestId: "r1",
      batchId: "b1",
      baseEditVersion: 3,
      accept: ["d1"],
      reject: ["d2"],
      // `undo` 即使为空也必须发：Rust 侧是 `#[serde(default)]`，三个数组齐发能让 journal
      // 里的请求体完整反映用户意图，重放时不会因为缺字段而语义不同。
      undo: []
    });
    expect(result.accepted).toEqual(["d1"]);
    expect(result.editVersion).toBe(4);
    expect(result.replayed).toBe(false);
  });

  it("action:'undo' 落到 wire 的 undo[] 上，且 undone 归一成独立列表（不是 accepted/stale/failed）", async () => {
    commandMock.mockReset();
    commandMock.mockResolvedValueOnce({
      schemaVersion: "ApplyRecognitionDecisionsResultV1",
      requestId: "r3",
      batchId: "b3",
      editVersionBefore: 6,
      editVersionAfter: 7,
      replayed: false,
      outcomes: [
        { decisionId: "d1", kind: "undone", message: "已撤销" },
        // 重复撤销：后端返回 superseded + RECOGNITION_ALREADY_RESOLVED，不是失败。
        { decisionId: "d2", kind: "superseded", reasonCode: "RECOGNITION_ALREADY_RESOLVED", message: "该建议已撤销" },
        // 用户后来改过目标：后端拒绝回滚。
        { decisionId: "d3", kind: "failed", reasonCode: "USER_EDITED_AFTER_APPLY", message: "该槽位在自动修正之后已被修改" }
      ],
      view: {
        itemId: "i1",
        batchId: "b3",
        editVersion: 7,
        chains: {},
        actionable: [],
        autoApplied: [{ decisionId: "d1", resolution: "auto_fixed", status: "undone" } as never]
      }
    });
    const { applyRecognitionDecisions } = await import("./recognitionClient");
    const result = await applyRecognitionDecisions({
      itemId: "i1",
      batchId: "b3",
      baseEditVersion: 6,
      requestId: "r3",
      decisions: [{ decisionId: "d1", action: "undo" }]
    });

    const [, args] = commandMock.mock.calls[0] as [string, { input: Record<string, unknown> }];
    expect(args.input.undo).toEqual(["d1"]);
    expect(args.input.accept).toEqual([]);
    expect(args.input.reject).toEqual([]);

    expect(result.undone).toEqual(["d1"]);
    expect(result.accepted).toEqual([]);
    // 重复撤销走 stale，不报成失败 —— 否则用户点两次就会看到一条吓人的错误。
    expect(result.stale).toEqual(["d2"]);
    expect(result.failed.map((f) => f.code)).toEqual(["USER_EDITED_AFTER_APPLY"]);
    expect(result.summary.undone).toBe(1);
    expect(result.editVersion).toBe(7);
  });

  it("superseded 归入 stale 而不是 failed", async () => {
    commandMock.mockReset();
    commandMock.mockResolvedValueOnce({
      schemaVersion: "ApplyRecognitionDecisionsResultV1",
      requestId: "r2",
      batchId: "b2",
      editVersionBefore: 5,
      editVersionAfter: 5,
      replayed: false,
      outcomes: [
        { decisionId: "d9", kind: "superseded", message: "用户已改过" },
        { decisionId: "d8", kind: "failed", reasonCode: "APPLY_REJECTED", message: "写入被拒" }
      ],
      view: { itemId: "i1", batchId: "b2", editVersion: 5, chains: {}, actionable: [], autoApplied: [] }
    });
    const { applyRecognitionDecisions } = await import("./recognitionClient");
    const result = await applyRecognitionDecisions({
      itemId: "i1",
      batchId: "b2",
      baseEditVersion: 5,
      requestId: "r2",
      decisions: [{ decisionId: "d9", action: "accept" }]
    });
    expect(result.stale).toEqual(["d9"]);
    expect(result.failed.map((f) => f.decisionId)).toEqual(["d8"]);
    expect(result.failed[0].code).toBe("APPLY_REJECTED");
  });
});

// ── 云端自主修复摘要（`repair`）────────────────────────────────────────────
//
// 这一组锁的是「没有修复记录 ≠ 已修复」这条底线。旧批次与无云导入都没有这个字段，
// 一旦被归一成 `{status:"completed"}`，界面会把一次没跑过云端的导入显示成「已修好」。

describe("repair — 没有记录绝不被说成完成", () => {
  it("缺失 / 非法形状一律归一成 null（= 没有修复记录）", () => {
    expect(normalizeDecisionView({ itemId: "item-1" }).repair).toBeNull();
    expect(normalizeDecisionView({ itemId: "item-1", repair: null }).repair).toBeNull();
    // 形状不对（没有 status）也算「没有记录」，不能当成一个半成品状态去渲染。
    expect(
      normalizeDecisionView({ itemId: "item-1", repair: { appliedCount: 3 } as never }).repair
    ).toBeNull();
  });

  it("有记录时原样带出，包括撤销要用的 repairRunId", () => {
    const view = normalizeDecisionView({
      itemId: "item-1",
      repair: {
        status: "needs_attention",
        appliedCount: 2,
        undoAvailable: true,
        repairRunId: "cloud-repair:batch-1",
        remainingTasks: [{ userTaskId: "u1", blocking: true }]
      }
    });
    expect(view.repair?.status).toBe("needs_attention");
    expect(view.repair?.repairRunId).toBe("cloud-repair:batch-1");
  });

  it("`null` 说成「未进行云端修复」，绝不说成完成", () => {
    expect(describeRepairStatus(null)).toBe("未进行云端修复");
    expect(describeRepairStatus(undefined)).toBe("未进行云端修复");
  });

  it("completed 也会把剩余条数说出来——收工不等于整份稿没问题", () => {
    expect(describeRepairStatus({ status: "completed" })).toBe("云端已自动修复");
    expect(
      describeRepairStatus({
        status: "completed",
        remainingTasks: [{ userTaskId: "u1" }, { userTaskId: "u2" }]
      })
    ).toBe("云端已自动修复，还有 2 处待处理");
    expect(
      describeRepairStatus({ status: "needs_attention", remainingTasks: [{ userTaskId: "u1" }] })
    ).toBe("云端已自动修复，还有 1 处需要你确认");
  });

  it("降级状态是「不影响继续编辑」，不是失败", () => {
    expect(describeRepairStatus({ status: "budget_exhausted" })).toBe(
      "云端修复达到本轮上限，剩余问题需要你处理"
    );
    expect(describeRepairStatus({ status: "unavailable" })).toBe(
      "云端修复未能完成，不影响继续编辑"
    );
    expect(describeRepairStatus({ status: "cancelled" })).toBe("云端修复已取消");
    // 未知状态不能猜成完成。
    expect(describeRepairStatus({ status: "something_new" })).toBe("云端修复状态未知");
  });

  it("撤销入口只在「真的写过修改且有 runId」时可用", () => {
    expect(canUndoRepair(null)).toBe(false);
    expect(canUndoRepair({ status: "completed", appliedCount: 0, undoAvailable: false })).toBe(false);
    // 有可撤销标记但没有 runId：撤销无从下手（后端按 runId 定位 journal），不能放行。
    expect(canUndoRepair({ status: "completed", undoAvailable: true })).toBe(false);
    expect(
      canUndoRepair({ status: "completed", undoAvailable: true, repairRunId: "cloud-repair:b1" })
    ).toBe(true);
  });

  it("整轮撤销走 undo_cloud_repair，且原样传 runId（不让前端拼字符串）", async () => {
    commandMock.mockReset();
    commandMock.mockResolvedValue({ status: "undone" });
    const { undoCloudRepair } = await import("./recognitionClient");
    await undoCloudRepair("item-1", "cloud-repair:batch-1", 7);
    expect(commandMock).toHaveBeenCalledWith("undo_cloud_repair", {
      itemId: "item-1",
      repairRunId: "cloud-repair:batch-1",
      baseVersion: 7
    });
  });
});
