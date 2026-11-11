// @vitest-environment jsdom
// 证据层级：单元（confirmedStore 的持久化分层；后端命令以替身代替）。
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ setLibraryItemConfirmed: vi.fn() }));
vi.mock("../../api/workspaceClient", () => ({ setLibraryItemConfirmed: mocks.setLibraryItemConfirmed }));

import { confirmItems, isConfirmed, isConfirmCommandUnavailable, readLocalConfirmedIds, unconfirmItem } from "./confirmedStore";
import type { LibraryRowV1 } from "./libraryTypes";

function row(id: string, confirmedAt: string | null = null): LibraryRowV1 {
  return {
    id, title: id, modality: "reading", stage: "ready", actionableCount: 0, questionTypes: [], updatedAt: "",
    inTrash: false, publishedForced: false, publishedNotLoadable: false, raw: {}, confirmedAt
  };
}

beforeEach(() => {
  window.localStorage.clear();
  mocks.setLibraryItemConfirmed.mockReset();
});

describe("确认入库：后端命令不存在时退回本地", () => {
  it("命令未注册（Tauri 报 command not found）时写入本地兜底集合", async () => {
    mocks.setLibraryItemConfirmed.mockRejectedValue(new Error("Command set_library_item_confirmed not found"));
    await confirmItems(["a", "b"]);
    expect([...readLocalConfirmedIds()].sort()).toEqual(["a", "b"]);
    expect(isConfirmed(row("a"))).toBe(true);
    expect(isConfirmed(row("c"))).toBe(false);
  });

  it("浏览器预览的 requires_tauri_runtime 同样退回本地；退回待确认会从本地集合移除", async () => {
    mocks.setLibraryItemConfirmed.mockRejectedValue(new Error("requires_tauri_runtime:set_library_item_confirmed"));
    await confirmItems(["a"]);
    await unconfirmItem("a");
    expect(readLocalConfirmedIds().has("a")).toBe(false);
    expect(isConfirmed(row("a"))).toBe(false);
  });

  it("后端可用时只调用后端，不写本地集合", async () => {
    mocks.setLibraryItemConfirmed.mockResolvedValue({});
    await confirmItems(["a"]);
    expect(mocks.setLibraryItemConfirmed).toHaveBeenCalledWith("a", true);
    expect(readLocalConfirmedIds().size).toBe(0);
  });

  it("其他错误如实抛出，不退回本地", async () => {
    mocks.setLibraryItemConfirmed.mockRejectedValue(new Error("database is locked"));
    await expect(confirmItems(["a"])).rejects.toThrow("database is locked");
    expect(readLocalConfirmedIds().size).toBe(0);
  });

  it("命令缺失类错误的判断只认命令缺失，不误判业务错误", () => {
    expect(isConfirmCommandUnavailable(new Error("Command set_library_item_confirmed not found"))).toBe(true);
    expect(isConfirmCommandUnavailable(new Error("requires_tauri_runtime:x"))).toBe(true);
    expect(isConfirmCommandUnavailable(new Error("EDIT_VERSION_CONFLICT"))).toBe(false);
  });
});

describe("行上的确认状态是后端 confirmedAt 与本地集合的并集", () => {
  it("后端 confirmedAt 非空即已确认，即使本地集合里没有", () => {
    expect(isConfirmed(row("x", "2026-11-10T00:00:00Z"))).toBe(true);
    expect(isConfirmed(row("y", null))).toBe(false);
  });
});
