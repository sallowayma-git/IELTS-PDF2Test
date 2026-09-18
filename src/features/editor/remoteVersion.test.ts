import { describe, expect, it } from "vitest";
import {
  decideRemoteVersionAction,
  describeDeferredRemoteRefresh,
  shouldApplyDeferredRemoteRefresh
} from "./remoteVersion";

/**
 * 这条规则的两种错法都很贵，所以单独断言：
 *   - 一律重拉 → 用户打字时后台事件到来，草稿被覆盖，输入凭空消失；
 *   - 一律不重拉 → 云端自主修复了内容，编辑器永远不知道，用户在过期题面上继续改。
 */
describe("decideRemoteVersionAction — 远端版本推进时的处置", () => {
  it("远端版本不高于本地已知版本 → 忽略（这是自己刚保存引起的回声）", () => {
    expect(decideRemoteVersionAction({ incoming: 5, current: 5, unsavedCount: 0 })).toBe("ignore");
    expect(decideRemoteVersionAction({ incoming: 4, current: 5, unsavedCount: 0 })).toBe("ignore");
    // 脏的时候同样是忽略：回声不该因为「本地脏」而变成一次推迟刷新。
    expect(decideRemoteVersionAction({ incoming: 5, current: 5, unsavedCount: 3 })).toBe("ignore");
  });

  it("远端确实更新 + 本地干净 → 立即重读", () => {
    expect(decideRemoteVersionAction({ incoming: 6, current: 5, unsavedCount: 0 })).toBe("reload");
  });

  it("远端确实更新 + 本地有未保存修改 → 推迟，不覆盖用户正在编辑的内容", () => {
    expect(decideRemoteVersionAction({ incoming: 6, current: 5, unsavedCount: 1 })).toBe("defer");
  });

  it("版本未知（null/undefined）时按可能有变更保守处理，绝不丢掉通知", () => {
    expect(decideRemoteVersionAction({ incoming: null, current: 5, unsavedCount: 0 })).toBe("reload");
    expect(decideRemoteVersionAction({ incoming: undefined, current: 5, unsavedCount: 0 })).toBe("reload");
    expect(decideRemoteVersionAction({ incoming: null, current: 5, unsavedCount: 2 })).toBe("defer");
  });

  it("版本号非有限数时按未知处理", () => {
    expect(decideRemoteVersionAction({ incoming: Number.NaN, current: 5, unsavedCount: 0 })).toBe("reload");
    expect(decideRemoteVersionAction({ incoming: Number.POSITIVE_INFINITY, current: 5, unsavedCount: 0 })).toBe("reload");
  });
});

describe("shouldApplyDeferredRemoteRefresh — 保存排空后是否补读", () => {
  it("没有推迟记录时不补读", () => {
    expect(shouldApplyDeferredRemoteRefresh(undefined, 5)).toBe(false);
  });

  it("记录的远端版本仍高于当前版本 → 补读", () => {
    expect(shouldApplyDeferredRemoteRefresh({ version: 7 }, 5)).toBe(true);
  });

  it("保存本身已经把版本推到位 → 不再补读（那次变更已经在稿里了）", () => {
    expect(shouldApplyDeferredRemoteRefresh({ version: 7 }, 7)).toBe(false);
    expect(shouldApplyDeferredRemoteRefresh({ version: 7 }, 8)).toBe(false);
  });

  it("版本未知 → 补读一次以确认，不能因为拿不到版本号就把变更忘掉", () => {
    expect(shouldApplyDeferredRemoteRefresh({}, 5)).toBe(true);
    expect(shouldApplyDeferredRemoteRefresh({ version: undefined }, 5)).toBe(true);
  });
});

describe("describeDeferredRemoteRefresh — 推迟期间的如实说明", () => {
  it("有版本号时说明版本", () => {
    expect(describeDeferredRemoteRefresh({ version: 7 })).toContain("版本 7");
  });

  it("没有版本号时不编造版本", () => {
    const text = describeDeferredRemoteRefresh({});
    // 断言的是「没有编造一个**具体**版本号」，不是「不含『版本』二字」——
    // 正文里的「加载最新版本」本来就该出现。
    expect(text).not.toMatch(/版本\s*\d/);
    expect(text).toContain("已被更新");
  });

  it("不把原因说成「云端自动修复」——版本推进也可能来自另一个窗口的人为编辑", () => {
    expect(describeDeferredRemoteRefresh({ version: 7 })).not.toContain("修复");
  });

  it("明确告诉用户修改没有丢，且会加载最新版本", () => {
    const text = describeDeferredRemoteRefresh({ version: 7 });
    expect(text).toContain("正在保存");
    expect(text).toContain("会自动加载最新版本");
  });
});
