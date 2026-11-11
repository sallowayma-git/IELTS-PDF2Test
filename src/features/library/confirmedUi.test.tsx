// @vitest-environment jsdom
// 证据层级：组件级（导入页确认按钮 / 全部确认、题库页退回待确认；IPC 与 store 以替身代替，
// 本地兜底使用真实的 appSettings / localStorage）。
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LibraryRowV1 } from "./libraryTypes";

const mocks = vi.hoisted(() => ({
  rows: [] as LibraryRowV1[],
  refresh: vi.fn(),
  setLibraryItemConfirmed: vi.fn()
}));
vi.mock("./libraryStore", () => ({
  useLibraryStore: () => ({
    rows: mocks.rows, loading: false, error: undefined, refresh: mocks.refresh, prependOptimistic: vi.fn(),
    moveToTrash: vi.fn(), restore: vi.fn(), permanentlyDelete: vi.fn(), emptyTrash: vi.fn(), setPart: vi.fn()
  })
}));
vi.mock("../../api/workspaceClient", () => ({ setLibraryItemConfirmed: mocks.setLibraryItemConfirmed }));
vi.mock("../import/ImportDrawer", () => ({ ImportDrawer: () => null }));
vi.mock("../import/useImportFiles", () => ({
  useImportFiles: () => ({ busy: false, stageMessage: undefined, error: undefined, importFiles: vi.fn() })
}));
vi.mock("../../api/processingClient", () => ({ retryProcessing: vi.fn(), describeRetryOutcome: vi.fn() }));
vi.mock("../../app/router", () => ({ go: vi.fn(), workspacePath: (id: string) => `#/workspace/${id}` }));

import { ImportPage } from "../import/ImportPage";
import { LibraryPaperTable } from "./LibraryPaperTable";
import { EMPTY_PAPER_SELECTION } from "./paperSelection";

function row(id: string, overrides: Partial<LibraryRowV1> = {}): LibraryRowV1 {
  return {
    id, title: `题目 ${id}`, modality: "reading", stage: "action_required", actionableCount: 0, questionTypes: [],
    updatedAt: "2026-11-07T00:00:00Z", inTrash: false, publishedForced: false, publishedNotLoadable: false, raw: {},
    part: "P1", ...overrides
  };
}

beforeEach(() => {
  window.localStorage.clear();
  mocks.setLibraryItemConfirmed.mockReset();
  mocks.refresh.mockReset();
  // 后端命令未注册：确认走本地兜底（真实 appSettings 写入 localStorage）。
  mocks.setLibraryItemConfirmed.mockRejectedValue(new Error("Command set_library_item_confirmed not found"));
  window.history.replaceState(null, "", "#/import");
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("导入页「待确认」", () => {
  it("未确认的识别完成题显示确认按钮；单行确认后离开待确认组", async () => {
    mocks.rows = [row("a"), row("b", { part: undefined, stage: "ready" })];
    render(<ImportPage />);
    const group = screen.getByTestId("import-group-review");
    expect(within(group).getByRole("heading", { name: "待确认" })).toBeTruthy();
    expect(within(group).getAllByTestId("import-row-confirm")).toHaveLength(2);

    fireEvent.click(group.querySelector<HTMLElement>('[data-item-id="a"] [data-testid="import-row-confirm"]')!);
    await waitFor(() => expect(group.querySelectorAll('[data-testid="import-row-confirm"]')).toHaveLength(1));
    expect(group.querySelector('[data-item-id="a"]')).toBeNull();
    expect(screen.getByTestId("import-notice").textContent).toContain("已进入题库");
  });

  it("未设置 Part 的阅读题确认时允许入库，但提示入库后在题库里设置 Part", async () => {
    mocks.rows = [row("nopart", { part: undefined })];
    render(<ImportPage />);
    fireEvent.click(screen.getByTestId("import-row-confirm"));
    await waitFor(() => expect(screen.getByTestId("import-notice").textContent).toContain("还没有设置 Part"));
    expect(screen.getByTestId("import-group-review").textContent).toContain("没有待确认的题目");
  });

  it("全部确认入库需要二次确认，取消则不写入", () => {
    mocks.rows = [row("a"), row("b")];
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<ImportPage />);
    fireEvent.click(screen.getByTestId("import-confirm-all"));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("确认 2 项"));
    expect(mocks.setLibraryItemConfirmed).not.toHaveBeenCalled();
    expect(screen.getByTestId("import-group-review").textContent).toContain("题目 a");
  });

  it("全部确认入库：确认后整组离开待确认", async () => {
    mocks.rows = [row("a"), row("b")];
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<ImportPage />);
    fireEvent.click(screen.getByTestId("import-confirm-all"));
    await waitFor(() => expect(screen.getByTestId("import-group-review").textContent).toContain("没有待确认的题目"));
    expect(screen.getByTestId("import-notice").textContent).toContain("已进入题库（2 项）");
  });

  it("确认失败（非命令缺失）时如实提示，题仍留在待确认", async () => {
    mocks.setLibraryItemConfirmed.mockRejectedValue(new Error("database is locked"));
    mocks.rows = [row("a")];
    render(<ImportPage />);
    fireEvent.click(screen.getByTestId("import-row-confirm"));
    await waitFor(() => expect(screen.getByTestId("import-notice").textContent).not.toBe(""));
    expect(screen.getByTestId("import-notice").textContent).not.toContain("已进入题库");
    expect(screen.getByTestId("import-group-review").textContent).toContain("题目 a");
  });
});

describe("题库页「退回待确认」", () => {
  it("未发布且不在当前试卷的行显示退回按钮；已发布或在当前试卷中的行不显示", () => {
    const onUnconfirm = vi.fn();
    const rows = [
      row("draft", { stage: "ready", confirmedAt: "2026-11-07T00:00:00Z", examId: "e1" }),
      row("published", { stage: "published", examId: "e2" }),
      row("inpaper", { stage: "ready", confirmedAt: "2026-11-07T00:00:00Z", examId: "e3" })
    ];
    render(
      <LibraryPaperTable
        rows={rows}
        modality="reading"
        selection={EMPTY_PAPER_SELECTION}
        hasCurrentPaper
        currentExamIds={new Set(["e3"])}
        onToggleReading={vi.fn()}
        onToggleListening={vi.fn()}
        onRename={vi.fn()}
        onOpen={vi.fn()}
        onSetPart={vi.fn()}
        onTrash={vi.fn()}
        onUnconfirm={onUnconfirm}
      />
    );
    const buttons = screen.queryAllByTestId("library-row-unconfirm");
    expect(buttons).toHaveLength(1);
    fireEvent.click(buttons[0]);
    expect(onUnconfirm).toHaveBeenCalledWith("draft");
  });
});
