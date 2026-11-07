// @vitest-environment jsdom
import { useState } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { describePaperPublishOutcome } from "../../api/publishClient";
import { LibraryPaperTable } from "./LibraryPaperTable";
import { PaperComposerBar } from "./PaperComposerBar";
import {
  EMPTY_PAPER_SELECTION,
  isPaperReady,
  toggleListeningItem,
  toggleReadingItem,
  type PaperSelection,
  type ReadingPart
} from "./paperSelection";
import type { LibraryRowV1 } from "./libraryTypes";

// 证据层级：组件级（jsdom + Testing Library，IPC 以回调替身代替）。
// 真实 Tauri 界面的验证见 scripts/e2e/tauri-cdp-paper-publish.mjs。

afterEach(cleanup);

function row(id: string, overrides: Partial<LibraryRowV1> = {}): LibraryRowV1 {
  return {
    id,
    title: `题目 ${id}`,
    modality: "reading",
    stage: "ready",
    actionableCount: 0,
    part: undefined,
    questionTypes: [],
    updatedAt: "2026-11-07T00:00:00Z",
    inTrash: false,
    publishedForced: false,
    publishedNotLoadable: false,
    raw: {},
    ...overrides
  };
}

const rows: LibraryRowV1[] = [
  row("p1a", { title: "阅读 A", part: "P1", questionTypes: ["true_false_not_given"] }),
  row("p1b", { title: "阅读 B", part: "P1" }),
  row("p2a", { title: "阅读 C", part: "P2" }),
  row("p3a", { title: "阅读 D", part: "P3" }),
  row("lis1", { title: "听力甲", modality: "listening", part: "Part 1" }),
  row("lis2", { title: "听力乙", modality: "listening", part: "Part 2" })
];

/** 与 LibraryPage 相同的选择语义：把表格回调接到 paperSelection 纯函数上。 */
function Harness({ onRename, onPublish }: { onRename: (id: string, title: string) => Promise<void>; onPublish?: () => void }) {
  const [selection, setSelection] = useState<PaperSelection>(EMPTY_PAPER_SELECTION);
  const [confirming, setConfirming] = useState(false);
  return (
    <>
      <LibraryPaperTable
        rows={rows}
        selection={selection}
        hasCurrentPaper={false}
        currentExamIds={new Set()}
        onToggleReading={(part: ReadingPart, id: string) => setSelection((current) => toggleReadingItem(current, part, id))}
        onToggleListening={(id: string, part: string) => setSelection((current) => toggleListeningItem(current, id, part))}
        onRename={onRename}
        onOpen={() => undefined}
        onSetPart={() => undefined}
        onTrash={() => undefined}
      />
      <PaperComposerBar
        selection={selection}
        titleOf={(id) => rows.find((r) => r.id === id)?.title ?? id}
        publishing={false}
        confirmation={confirming ? { incoming: [], replacing: [] } : undefined}
        onClear={() => setSelection(EMPTY_PAPER_SELECTION)}
        onRequestPublish={() => setConfirming(true)}
        onConfirm={() => {
          setConfirming(false);
          onPublish?.();
        }}
        onCancelConfirm={() => setConfirming(false)}
      />
      <output data-testid="ready">{isPaperReady(selection) ? "ready" : "not-ready"}</output>
    </>
  );
}

function checkbox(id: string): HTMLInputElement {
  const tr = document.querySelector<HTMLTableRowElement>(`tr[data-item-id="${id}"]`);
  if (!tr) throw new Error(`row ${id} not rendered`);
  return tr.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
}

describe("组卷选择（表格 + 组卷栏）", () => {
  it("同一 Part 勾另一篇会替换；三篇齐了才能发布", () => {
    const onPublish = vi.fn();
    render(<Harness onRename={vi.fn()} onPublish={onPublish} />);
    const publish = screen.getByTestId("paper-publish") as HTMLButtonElement;
    expect(publish.disabled).toBe(true);

    fireEvent.click(checkbox("p1a"));
    expect(checkbox("p1a").checked).toBe(true);
    fireEvent.click(checkbox("p1b"));
    expect(checkbox("p1a").checked).toBe(false);
    expect(checkbox("p1b").checked).toBe(true);
    expect(screen.getByTestId("paper-slot-P1").textContent).toContain("阅读 B");

    fireEvent.click(checkbox("p2a"));
    expect(publish.disabled).toBe(true);
    expect(screen.getByTestId("paper-composer-hint").textContent).toContain("P3");

    fireEvent.click(checkbox("p3a"));
    expect(screen.getByTestId("ready").textContent).toBe("ready");
    expect(publish.disabled).toBe(false);
    fireEvent.click(publish);
    expect(onPublish).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("paper-confirm-ok"));
    expect(onPublish).toHaveBeenCalledTimes(1);
  });

  it("听力可多选，未标 Part 的题不能勾选", () => {
    render(<Harness onRename={vi.fn()} />);
    fireEvent.click(checkbox("lis1"));
    fireEvent.click(checkbox("lis2"));
    expect(checkbox("lis1").checked).toBe(true);
    expect(checkbox("lis2").checked).toBe(true);
    expect(screen.getByTestId("paper-slot-listening").textContent).toContain("2 篇");
  });
});

describe("名称单元格编辑", () => {
  it("单击进入编辑，Enter 保存（去掉首尾空白）", async () => {
    const onRename = vi.fn().mockResolvedValue(undefined);
    render(<Harness onRename={onRename} />);
    fireEvent.click(screen.getByText("阅读 A"));
    const input = screen.getByLabelText("题目名称") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "  新名字  " } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(onRename).toHaveBeenCalledWith("p1a", "新名字"));
    expect(screen.queryByLabelText("题目名称")).toBeNull();
  });

  it("Esc 取消：不保存，名称保持原样", () => {
    const onRename = vi.fn().mockResolvedValue(undefined);
    render(<Harness onRename={onRename} />);
    fireEvent.click(screen.getByText("阅读 A"));
    const input = screen.getByLabelText("题目名称");
    fireEvent.change(input, { target: { value: "不要这个" } });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(onRename).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("题目名称")).toBeNull();
    expect(screen.getByText("阅读 A")).toBeTruthy();
  });

  it("失焦同样保存；Enter 之后的失焦不会重复保存", async () => {
    const onRename = vi.fn().mockResolvedValue(undefined);
    render(<Harness onRename={onRename} />);
    fireEvent.click(screen.getByText("阅读 C"));
    const input = screen.getByLabelText("题目名称");
    fireEvent.change(input, { target: { value: "失焦保存" } });
    fireEvent.blur(input);
    await waitFor(() => expect(onRename).toHaveBeenCalledTimes(1));
    expect(onRename).toHaveBeenCalledWith("p2a", "失焦保存");
  });

  it("空名不允许：停留在编辑态并提示，不调用保存", () => {
    const onRename = vi.fn().mockResolvedValue(undefined);
    render(<Harness onRename={onRename} />);
    fireEvent.click(screen.getByText("阅读 A"));
    const input = screen.getByLabelText("题目名称");
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onRename).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("名称不能为空");
    expect(screen.getByLabelText("题目名称")).toBeTruthy();
  });

  it("保存失败：回滚为原名并提示原因", async () => {
    const onRename = vi.fn().mockRejectedValue(new Error("名称保存失败，请稍后再试。"));
    render(<Harness onRename={onRename} />);
    fireEvent.click(screen.getByText("阅读 A"));
    const input = screen.getByLabelText("题目名称");
    fireEvent.change(input, { target: { value: "会失败的名字" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("名称保存失败"));
    expect(screen.getByText("阅读 A")).toBeTruthy();
    expect(screen.queryByText("会失败的名字")).toBeNull();
  });
});

describe("组卷结果文案", () => {
  it("缺题时列出题名，并说明当前试卷保持不变", () => {
    const text = describePaperPublishOutcome(
      {
        destination: "D:/nas",
        succeeded: [],
        failed: [],
        paper: { ok: false, code: "PAPER_INCOMPLETE", missing: ["p2a"] }
      },
      (id) => rows.find((r) => r.id === id)?.title ?? id
    );
    expect(text).toContain("阅读 C");
    expect(text).toContain("保持不变");
  });

  it("成功时给出编号前 8 位", () => {
    const text = describePaperPublishOutcome(
      {
        destination: "D:/nas",
        succeeded: [],
        failed: [],
        paper: { ok: true, paperId: "0123456789abcdef", publishedAt: "", batchId: "", reading: [], listening: [] }
      },
      (id) => id
    );
    expect(text).toContain("01234567");
    expect(text).not.toContain("89abcdef");
  });
});
