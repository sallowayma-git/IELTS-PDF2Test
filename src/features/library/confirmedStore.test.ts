// @vitest-environment jsdom
// 证据层级：单元（confirmedStore 的持久化分层、逐条拒绝文案、启动迁移；后端命令以替身代替）。
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ setLibraryItemsConfirmed: vi.fn(), importLocalConfirmedIds: vi.fn() }));
vi.mock("../../api/workspaceClient", () => ({
  setLibraryItemsConfirmed: mocks.setLibraryItemsConfirmed,
  importLocalConfirmedIds: mocks.importLocalConfirmedIds
}));

import {
  confirmItems, describeConfirmRejection, isConfirmed, isConfirmCommandUnavailable, migrateLocalConfirmedOnce,
  readLocalConfirmedIds, resetLocalMigrationForTest, summarizeConfirmFailures, unconfirmItem
} from "./confirmedStore";
import { isRowConfirmed, type LibraryRowV1 } from "./libraryTypes";

function row(id: string, confirmedAt: string | null | undefined = undefined): LibraryRowV1 {
  return {
    id, title: id, modality: "reading", stage: "ready", actionableCount: 0, questionTypes: [], updatedAt: "",
    inTrash: false, publishedForced: false, publishedNotLoadable: false, raw: {}, confirmedAt
  };
}

const ok = (itemId: string) => ({ itemId, ok: true, row: { id: itemId, confirmedAt: "2026-11-11T00:00:00Z" } });

beforeEach(() => {
  window.localStorage.clear();
  mocks.setLibraryItemsConfirmed.mockReset();
  mocks.importLocalConfirmedIds.mockReset();
  resetLocalMigrationForTest();
});

describe("确认入库：批量命令与逐条结果", () => {
  it("后端可用时只调用批量命令，不写本地集合", async () => {
    mocks.setLibraryItemsConfirmed.mockResolvedValue([ok("a"), ok("b")]);
    const outcome = await confirmItems(["a", "b"]);
    expect(mocks.setLibraryItemsConfirmed).toHaveBeenCalledWith(["a", "b"], true);
    expect(outcome).toEqual({ confirmed: ["a", "b"], failed: [] });
    expect(readLocalConfirmedIds().size).toBe(0);
  });

  it("逐条业务拒绝不影响其余条目，原因映射成中文", async () => {
    mocks.setLibraryItemsConfirmed.mockResolvedValue([
      ok("a"),
      { itemId: "b", ok: false, error: "LIBRARY_ITEM_NOT_CONFIRMABLE:processing" },
      { itemId: "c", ok: false, error: "LIBRARY_ITEM_NOT_CONFIRMABLE:canonical_missing" }
    ]);
    const outcome = await confirmItems(["a", "b", "c"]);
    expect(outcome.confirmed).toEqual(["a"]);
    expect(outcome.failed.map((item) => item.itemId)).toEqual(["b", "c"]);
    expect(outcome.failed[0].reason).toContain("识别");
    expect(outcome.failed[1].reason).toContain("题稿");
    expect(summarizeConfirmFailures(outcome, (id) => `题${id}`)).toBe("「题b」还在识别或排队中，识别完成后再确认；「题c」题稿还没有生成，请先完成识别");
  });

  it("逐条映射覆盖全部确认错误码与在用拒绝", () => {
    for (const reason of ["processing", "failed", "canonical_missing", "deleted", "not_found", "current_paper_invalid"]) {
      expect(describeConfirmRejection(`LIBRARY_ITEM_NOT_CONFIRMABLE:${reason}`)).not.toMatch(/LIBRARY_|:/);
    }
    expect(describeConfirmRejection("LIBRARY_ITEM_IN_USE")).toContain("不能退回待确认");
  });

  it("退回待确认：逐条被拒（在用）时抛出中文原因", async () => {
    mocks.setLibraryItemsConfirmed.mockResolvedValue([{ itemId: "a", ok: false, error: "LIBRARY_ITEM_IN_USE" }]);
    await expect(unconfirmItem("a")).rejects.toThrow("不能退回待确认");
  });
});

describe("确认入库：后端命令不存在时退回本地", () => {
  it("命令未注册（Tauri 报 command not found）时写入本地兜底集合", async () => {
    mocks.setLibraryItemsConfirmed.mockRejectedValue(new Error("Command set_library_items_confirmed not found"));
    await confirmItems(["a", "b"]);
    expect([...readLocalConfirmedIds()].sort()).toEqual(["a", "b"]);
    expect(isConfirmed(row("a"))).toBe(true);
    expect(isConfirmed(row("c"))).toBe(false);
  });

  it("浏览器预览的 requires_tauri_runtime 同样退回本地；退回待确认会从本地集合移除", async () => {
    mocks.setLibraryItemsConfirmed.mockRejectedValue(new Error("requires_tauri_runtime:set_library_items_confirmed"));
    await confirmItems(["a"]);
    mocks.setLibraryItemsConfirmed.mockRejectedValue(new Error("requires_tauri_runtime:set_library_items_confirmed"));
    await unconfirmItem("a");
    expect(readLocalConfirmedIds().has("a")).toBe(false);
  });

  it("后端接管后退回待确认：同时清掉之前留在本地的兜底记录", async () => {
    mocks.setLibraryItemsConfirmed.mockRejectedValueOnce(new Error("Command set_library_items_confirmed not found"));
    await confirmItems(["a"]);
    expect(readLocalConfirmedIds().has("a")).toBe(true);
    mocks.setLibraryItemsConfirmed.mockResolvedValue([{ itemId: "a", ok: true }]);
    await unconfirmItem("a");
    expect(mocks.setLibraryItemsConfirmed).toHaveBeenLastCalledWith(["a"], false);
    expect(readLocalConfirmedIds().has("a")).toBe(false);
  });

  it("其他错误如实抛出，不退回本地", async () => {
    mocks.setLibraryItemsConfirmed.mockRejectedValue(new Error("database is locked"));
    await expect(confirmItems(["a"])).rejects.toThrow("database is locked");
    expect(readLocalConfirmedIds().size).toBe(0);
  });

  it("命令缺失类错误的判断只认命令缺失，不误判业务错误", () => {
    expect(isConfirmCommandUnavailable(new Error("Command set_library_items_confirmed not found"))).toBe(true);
    expect(isConfirmCommandUnavailable(new Error("requires_tauri_runtime:x"))).toBe(true);
    expect(isConfirmCommandUnavailable(new Error("EDIT_VERSION_CONFLICT"))).toBe(false);
  });
});

describe("行上的确认状态：后端 confirmedAt 为权威", () => {
  it("后端 confirmedAt 非空即已确认，即使本地集合里没有", () => {
    expect(isRowConfirmed(row("x", "2026-11-10T00:00:00Z"), new Set())).toBe(true);
  });

  it("后端明确为 null（未确认）时，即使本地残留该 id 也不算已确认", () => {
    expect(isRowConfirmed(row("y", null), new Set(["y"]))).toBe(false);
  });

  it("行上没有后端确认字段（兜底数据）时才看本地集合", () => {
    expect(isRowConfirmed(row("z", undefined), new Set(["z"]))).toBe(true);
    expect(isRowConfirmed(row("z", undefined), new Set())).toBe(false);
  });
});

describe("启动迁移：本地确认集合导入后端", () => {
  it("本地非空：调用导入命令，成功后清空本地集合并报告已变化", async () => {
    window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ confirmedLibraryItemIds: ["a", "b"] }));
    mocks.importLocalConfirmedIds.mockResolvedValue(["a"]);
    await expect(migrateLocalConfirmedOnce()).resolves.toBe(true);
    expect(mocks.importLocalConfirmedIds).toHaveBeenCalledWith(["a", "b"]);
    expect(readLocalConfirmedIds().size).toBe(0);
  });

  it("同一会话只执行一次", async () => {
    window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ confirmedLibraryItemIds: ["a"] }));
    mocks.importLocalConfirmedIds.mockResolvedValue(["a"]);
    await Promise.all([migrateLocalConfirmedOnce(), migrateLocalConfirmedOnce()]);
    await migrateLocalConfirmedOnce();
    expect(mocks.importLocalConfirmedIds).toHaveBeenCalledTimes(1);
  });

  it("导入失败：保留本地集合，不抛错，下次启动再试", async () => {
    window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ confirmedLibraryItemIds: ["a"] }));
    mocks.importLocalConfirmedIds.mockRejectedValue(new Error("database is locked"));
    await expect(migrateLocalConfirmedOnce()).resolves.toBe(false);
    expect([...readLocalConfirmedIds()]).toEqual(["a"]);
  });

  it("本地为空：不调用后端", async () => {
    await expect(migrateLocalConfirmedOnce()).resolves.toBe(false);
    expect(mocks.importLocalConfirmedIds).not.toHaveBeenCalled();
  });
});
