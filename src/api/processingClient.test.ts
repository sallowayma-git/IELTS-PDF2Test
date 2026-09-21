import { describe, expect, it } from "vitest";
import { acceptProcessingUpdate, describeRetryOutcome, retryQueued } from "./processingClient";

/**
 * 这些用例守住的是**一条看不见的规则**：哪些 `processing://item-updated` 会被丢掉。
 *
 * 它写错时的症状是「内容改了但工作区面板不刷新」——界面上看起来只是慢，几乎无法
 * 从肉眼发现。所以规则本身必须被单独断言。
 */
describe("acceptProcessingUpdate — 事件收窄与去重", () => {
  const payload = (over: Record<string, unknown> = {}) => ({
    libraryItemId: "item-1",
    stateVersion: 3,
    editVersion: 7,
    stage: "ready_for_review",
    ...over
  });

  it("首次收到的事件被放行，并带上 editVersion", () => {
    const versions = new Map<string, number>();
    const update = acceptProcessingUpdate(versions, payload());
    expect(update).toEqual({ itemId: "item-1", stateVersion: 3, editVersion: 7 });
    expect(versions.get("item-1")).toBe(3);
  });

  it("序号不大于已见值时被丢弃（乱序/重复事件）", () => {
    const versions = new Map([["item-1", 5]]);
    expect(acceptProcessingUpdate(versions, payload({ stateVersion: 5 }))).toBeUndefined();
    expect(acceptProcessingUpdate(versions, payload({ stateVersion: 4 }))).toBeUndefined();
    // 被丢弃的事件**不能**把已见序号改小，否则后续一条合法的旧序号会被重新放行。
    expect(versions.get("item-1")).toBe(5);
  });

  it("序号更大时放行（内容提交推高序号后必须能被消费）", () => {
    const versions = new Map([["item-1", 5]]);
    const update = acceptProcessingUpdate(versions, payload({ stateVersion: 6, editVersion: 8 }));
    expect(update).toEqual({ itemId: "item-1", stateVersion: 6, editVersion: 8 });
  });

  it("不同条目各自独立计数", () => {
    const versions = new Map([["item-1", 9]]);
    const update = acceptProcessingUpdate(versions, payload({ libraryItemId: "item-2", stateVersion: 1 }));
    expect(update?.itemId).toBe("item-2");
  });

  it("editVersion 缺失不导致整条事件被丢（否则阶段推进也一起没了）", () => {
    const versions = new Map<string, number>();
    const update = acceptProcessingUpdate(versions, payload({ editVersion: undefined }));
    expect(update).toEqual({ itemId: "item-1", stateVersion: 3, editVersion: undefined });
  });

  it("editVersion 为 null 时如实记为未知，不猜成 0", () => {
    const versions = new Map<string, number>();
    const update = acceptProcessingUpdate(versions, payload({ editVersion: null }));
    expect(update?.editVersion).toBeUndefined();
  });

  it("editVersion 为 NaN/Infinity 时同样记为未知", () => {
    expect(acceptProcessingUpdate(new Map(), payload({ editVersion: Number.NaN }))?.editVersion).toBeUndefined();
    expect(acceptProcessingUpdate(new Map(), payload({ editVersion: Number.POSITIVE_INFINITY }))?.editVersion).toBeUndefined();
  });

  it("载荷形状不对时整条丢弃，不产生半个事件", () => {
    const versions = new Map<string, number>();
    expect(acceptProcessingUpdate(versions, null)).toBeUndefined();
    expect(acceptProcessingUpdate(versions, "nope")).toBeUndefined();
    expect(acceptProcessingUpdate(versions, {})).toBeUndefined();
    expect(acceptProcessingUpdate(versions, payload({ libraryItemId: "" }))).toBeUndefined();
    expect(acceptProcessingUpdate(versions, payload({ libraryItemId: 42 }))).toBeUndefined();
    // 序号必须是数字：字符串 "3" 不能靠隐式比较蒙混过关。
    expect(acceptProcessingUpdate(versions, payload({ stateVersion: "3" }))).toBeUndefined();
    expect(acceptProcessingUpdate(versions, payload({ stateVersion: Number.NaN }))).toBeUndefined();
    expect(versions.size).toBe(0);
  });
});

describe("重新识别 — 如实报告有没有加入队列", () => {
  it("后端说没入队（正在识别中）时，绝不说「已加入识别队列」", () => {
    expect(retryQueued({ queued: false })).toBe(false);
    const text = describeRetryOutcome(false);
    expect(text).not.toContain("已加入");
    expect(text).toContain("正在识别");
  });

  it("入队成功时说明会用当前的云端设置", () => {
    expect(retryQueued({ queued: true })).toBe(true);
    expect(describeRetryOutcome(true)).toContain("已加入识别队列");
  });

  it("旧后端没有返回值：不猜成功", () => {
    expect(retryQueued(undefined)).toBe(false);
  });
});