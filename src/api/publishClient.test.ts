import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  describeBatchPublishOutcome,
  describePublishOutcome,
  publishItem,
  publishItems,
  publishOutcomeKind,
  type PublishItemOutcome
} from "./publishClient";

// 证据层级：pure unit（IPC 被 mock）。
// 产品决策：点击「发布」即是确认。前端**一次点击**就发出带放行确认的请求，
// 不做任何预检/阻断；放行是否真的被用到、门禁结论是什么，只以后端记录为准。
const { commandMock } = vi.hoisted(() => ({ commandMock: vi.fn() }));
vi.mock("./tauriCommands", () => ({ command: commandMock }));

beforeEach(() => {
  commandMock.mockReset();
});

describe("publishItems — 一次点击发出带放行确认的请求", () => {
  it("总是携带 force（confirmedAt = 点击时间），且只调用一次 publish_items", async () => {
    commandMock.mockResolvedValue({ destination: "D:/nas", succeeded: [], failed: [] });
    const before = Date.now();
    await publishItems(["a", "b"], "D:/nas");
    expect(commandMock).toHaveBeenCalledTimes(1);
    const [name, args] = commandMock.mock.calls[0];
    expect(name).toBe("publish_items");
    expect(args.input.itemIds).toEqual(["a", "b"]);
    expect(args.input.destination).toBe("D:/nas");
    expect(args.input.force.acknowledgedReasons).toEqual([]);
    const confirmed = Date.parse(args.input.force.confirmedAt);
    expect(Number.isNaN(confirmed)).toBe(false);
    expect(confirmed).toBeGreaterThanOrEqual(before - 1000);
    // 旧的门禁开关不再存在：请求体里只有后端认识的字段（后端 deny_unknown_fields）。
    expect(Object.keys(args.input).sort()).toEqual(["destination", "force", "itemIds"]);
  });

  it("单题发布同样一步到位，不先拉预检", async () => {
    commandMock.mockResolvedValue({
      destination: "D:/nas",
      succeeded: [{ itemId: "a", ok: true, examId: "x", forced: true, studentLoadable: true }],
      failed: []
    });
    const outcome = await publishItem("a", "D:/nas");
    expect(commandMock).toHaveBeenCalledTimes(1);
    expect(commandMock.mock.calls[0][0]).toBe("publish_items");
    expect(outcome.ok).toBe(true);
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

  it("失败就是失败，不被任何字段改判成已发布", () => {
    expect(publishOutcomeKind({ ...base, ok: false, forced: false, studentLoadable: true })).toBe("failed");
  });
});
