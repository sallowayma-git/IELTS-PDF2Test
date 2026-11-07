import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  describeBatchPublishOutcome,
  describePaperPublishOutcome,
  describePublishOutcome,
  publishItems,
  publishOutcomeKind,
  type PublishItemOutcome
} from "./publishClient";

// 证据层级：pure unit（IPC 被 mock）。
// 产品决策（P2 门禁收敛）：点击「发布」即是确认，前端一次点击发出请求、不做预检阻断；
// 发布判定收敛为「编译通过 = 可发布」，**不再带 force 伪装**——放行确认去掉后，
// 后端记录里的 `forced` 只可能是真实放行，审计不再被每次发布污染。
const { commandMock } = vi.hoisted(() => ({ commandMock: vi.fn() }));
vi.mock("./tauriCommands", () => ({ command: commandMock }));

beforeEach(() => {
  commandMock.mockReset();
});

describe("publishItems — 一次点击发出请求（P2：无 force 伪装）", () => {
  it("请求体只有 itemIds/destination：不再带 force 放行确认", async () => {
    commandMock.mockResolvedValue({ destination: "D:/nas", succeeded: [], failed: [] });
    await publishItems(["a", "b"], "D:/nas");
    expect(commandMock).toHaveBeenCalledTimes(1);
    const [name, args] = commandMock.mock.calls[0];
    expect(name).toBe("publish_items");
    expect(args.input.itemIds).toEqual(["a", "b"]);
    expect(args.input.destination).toBe("D:/nas");
    expect(args.input.force).toBeUndefined();
    // 请求体里只有后端认识的字段（后端 deny_unknown_fields）。
    expect(Object.keys(args.input).sort()).toEqual(["destination", "itemIds"]);
  });

  it("单题发布入口已取消：后端拒绝不带 paper 的请求，前端把它收敛成可读的失败结果", async () => {
    commandMock.mockRejectedValue("PUBLISH_REQUIRES_PAPER");
    const batch = await publishItems(["a"], "D:/nas");
    expect(batch.succeeded).toEqual([]);
    expect(batch.failed[0].ok).toBe(false);
    expect(batch.failed[0].message).toContain("组卷栏");
  });
});

describe("发布提示文案", () => {
  const ok = (extra: Partial<PublishItemOutcome>): PublishItemOutcome => ({ itemId: "a", ok: true, ...extra });

  it("严格发布与可加载的放行发布都只说「已发布」", () => {
    expect(describePublishOutcome(ok({ forced: false, studentLoadable: true }))).toBe("已发布");
    expect(describePublishOutcome(ok({ forced: true, studentLoadable: true }))).toBe("已发布");
  });

  it("放行发布绝不使用「发布完成」（验收脚本把它当作干净通过）", () => {
    for (const outcome of [
      ok({ forced: true, studentLoadable: true }),
      ok({ forced: true, studentLoadable: false }),
      ok({ forced: false, studentLoadable: true })
    ]) {
      expect(describePublishOutcome(outcome)).not.toContain("发布完成");
    }
    expect(
      describeBatchPublishOutcome({
        destination: "D:/nas",
        succeeded: [ok({ forced: true, studentLoadable: true })],
        failed: []
      })
    ).not.toContain("发布完成");
  });

  it("学生端打不开时给出唯一的额外提示", () => {
    expect(describePublishOutcome(ok({ forced: true, studentLoadable: false }))).toBe(
      "已发布，但学生端暂时无法打开这道题"
    );
  });

  it("不向用户展示门禁阈值或阻断清单", () => {
    const text = describePublishOutcome(ok({ forced: true, studentLoadable: true }));
    expect(text).not.toMatch(/阻断|门禁|检查|质量|coverage|blocked/i);
  });

  it("批量提示：数量 + 学生端打不开的题数 + 未发布数", () => {
    expect(
      describeBatchPublishOutcome({
        destination: "D:/nas",
        succeeded: [ok({ studentLoadable: true }), ok({ itemId: "b", studentLoadable: false })],
        failed: [{ itemId: "c", ok: false, message: "x" }]
      })
    ).toBe("已发布 2 题 · 其中 1 题学生端暂时无法打开 · 1 题未发布");
  });

  it("失败时透传后端已归一的文案", () => {
    expect(describePublishOutcome({ itemId: "a", ok: false, message: "目标目录不可写，请检查权限。" })).toBe(
      "目标目录不可写，请检查权限。"
    );
  });
});

describe("publishOutcomeKind — 只给验收脚本读的机器分类", () => {
  const base: PublishItemOutcome = { itemId: "item-1", ok: true };

  it("干净发布与放行发布在界面上同一句「已发布」，但分类不同", () => {
    const clean = { ...base, forced: false, studentLoadable: true };
    const forced = { ...base, forced: true, studentLoadable: true };
    expect(describePublishOutcome(clean)).toBe(describePublishOutcome(forced));
    expect(publishOutcomeKind(clean)).toBe("published");
    expect(publishOutcomeKind(forced)).toBe("published_forced");
  });

  it("学生端打不开的放行发布单独归类", () => {
    expect(publishOutcomeKind({ ...base, forced: true, studentLoadable: false }))
      .toBe("published_forced_not_loadable");
  });

  it("门禁 Ready、只是包检查没过的条目：不是 published，也不叫「放行」", () => {
    // 这一条没有任何东西被放行（forced=false），却装不进学生包。旧实现把它归成
    // published_forced_not_loadable，等于对用户和验收脚本都说「用户点过放行」。
    const kind = publishOutcomeKind({ ...base, forced: false, studentLoadable: false });
    expect(kind).toBe("published_not_loadable");
    expect(kind).not.toBe("published");
  });

  it("失败就是失败，不被任何字段改判成已发布", () => {
    expect(publishOutcomeKind({ ...base, ok: false, forced: false, studentLoadable: true })).toBe("failed");
  });
});

describe("组卷结果的顶层状态与锁忙（V1.1 第4条 / P1-1 / P2-2）", () => {
  const titleOf = (id: string) => id;

  it("清单已更新但指针失败：文案说明学生端仍是旧试卷，不得说成已发布", () => {
    const text = describePaperPublishOutcome(
      {
        destination: "D:/nas",
        succeeded: [{ itemId: "a", ok: true }],
        failed: [],
        status: "paper_pointer_stale",
        paper: { ok: false, code: "PAPER_WRITE_FAILED", message: "disk full" }
      },
      titleOf
    );
    expect(text).toBe("清单已更新，但学生端仍是旧试卷，请重试发布。");
    expect(text).not.toContain("已发布");
  });

  it("指针成功但有提交后警告：显示警告而不是失败", () => {
    const text = describePaperPublishOutcome(
      {
        destination: "D:/nas",
        succeeded: [],
        failed: [],
        status: "published_with_warnings",
        warnings: [{ reason: "status_drift" }],
        paper: { ok: true, paperId: "0123456789abcdef", publishedAt: "", batchId: "b", reading: [], listening: [] }
      },
      titleOf
    );
    expect(text).toContain("已发布");
    expect(text).toContain("未完整写入");
  });

  it("发布锁忙：提示另一次发布进行中，不得显示「当前试卷没有更新」", async () => {
    commandMock.mockRejectedValue("nas_package_v2_lock_busy:the lock could not be acquired");
    const batch = await publishItems(["a"], "D:/nas", undefined, { paper: { reading: { P1: "a", P2: "b", P3: "c" }, listening: [] } });
    expect(batch.failed[0].message).toBe("另一次发布正在进行，请稍后查看当前试卷。");
    expect(batch.failed[0].message).not.toContain("没有更新");
  });
});
