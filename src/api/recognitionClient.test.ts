import { describe, expect, it, vi } from "vitest";
import { describeCloudReason, describeCloudStatus, normalizeDecisionView } from "./recognitionClient";

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
    expect(view.items).toEqual([]);
    // 关键：绝不能是 undefined，否则界面会渲染「未知（undefined）」。
    expect(describeCloudStatus(view.cloudStatus, view.cloudReasonCode)).not.toContain("undefined");
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

describe("describeCloudStatus / describeCloudReason — 用户可读且不误导", () => {
  it("每个稳定状态都有明确文案", () => {
    expect(describeCloudStatus("not_started")).toContain("还没有运行");
    expect(describeCloudStatus("running")).toContain("进行中");
    expect(describeCloudStatus("succeeded")).toBe("云端核验完成。");
    expect(describeCloudStatus("failed")).toContain("失败");
    expect(describeCloudStatus("skipped")).toContain("没有运行");
  });

  it("unavailable 优先给出稳定原因码文案", () => {
    expect(describeCloudStatus("unavailable", "MODEL_TIMEOUT")).toContain("超时");
    expect(describeCloudStatus("unavailable", "NO_PROFILE")).toContain("模型");
    expect(describeCloudStatus("unavailable", "SOMETHING_NEW")).toContain("SOMETHING_NEW");
  });

  it("未知状态不渲染 undefined", () => {
    expect(describeCloudStatus(undefined)).toBe("云端核验状态未知。");
    expect(describeCloudStatus("weird_state")).toBe("云端核验状态：weird_state。");
    expect(describeCloudReason(undefined)).toBeUndefined();
  });
});

describe("describeCloudStatus — queued / partial / unusable 必须被正确区分", () => {
  it("queued 说明「排队中」而不是「没跑」（本地先出稿时云端正是这个状态）", () => {
    const view = normalizeDecisionView({ itemId: "item-1", chains: { local: { state: "done" }, cloud: { state: "queued" } } });
    expect(view.cloudStatus).toBe("queued");
    const text = describeCloudStatus(view.cloudStatus, view.cloudReasonCode);
    expect(text).toContain("排队");
    expect(text).not.toContain("还没有运行");
    expect(text).not.toContain("undefined");
  });

  it("partial 说明「只核验了部分内容」", () => {
    const view = normalizeDecisionView({ itemId: "item-1", chains: { cloud: { state: "partial" } } });
    expect(view.cloudStatus).toBe("partial");
    expect(describeCloudStatus(view.cloudStatus)).toContain("部分");
  });

  it("unusable 走 unavailable 的原因码文案，不显示英文原样", () => {
    const view = normalizeDecisionView({ itemId: "item-1", chains: { cloud: { state: "unusable" } } });
    expect(view.cloudStatus).toBe("unavailable");
    expect(describeCloudStatus(view.cloudStatus, "MODEL_UNSUPPORTED_INPUT")).toContain("不支持");
  });

  it("canceled 归入 not_started", () => {
    const view = normalizeDecisionView({ itemId: "item-1", chains: { cloud: { state: "canceled" } } });
    expect(view.cloudStatus).toBe("not_started");
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
      reject: ["d2"]
    });
    expect(result.accepted).toEqual(["d1"]);
    expect(result.editVersion).toBe(4);
    expect(result.replayed).toBe(false);
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
