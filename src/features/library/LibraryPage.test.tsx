// @vitest-environment jsdom
import { useEffect, useState } from "react";
import { parseRoute } from "../../app/router";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LibraryPage } from "./LibraryPage";
import type { LibraryRowV1 } from "./libraryTypes";

// 组件集成证据：保留真实表格、组卷栏与写作编辑器，仅替换数据访问。
const mocks = vi.hoisted(() => ({ rows: [] as LibraryRowV1[], refresh: vi.fn(), setPart: vi.fn(), listWriting: vi.fn() }));
vi.mock("./libraryStore", () => ({ useLibraryStore: () => ({
  rows: mocks.rows, loading: false, refresh: mocks.refresh, setPart: mocks.setPart
}) }));
vi.mock("../../api/tauriCommands", () => ({ listWritingJobs: mocks.listWriting }));
vi.mock("../settings/appSettings", () => ({ readAppSettings: () => ({}), writeAppSettings: vi.fn() }));

function row(id: string, part?: string, modality: "reading" | "listening" = "reading"): LibraryRowV1 {
  // 题库只显示已确认入库的识别完成题（见 libraryRows）。
  return { id, title: id, part, modality, stage: "ready", actionableCount: 0, questionTypes: [],
    updatedAt: "", inTrash: false, publishedForced: false, publishedNotLoadable: false, raw: {},
    confirmedAt: "2026-11-07T00:00:00Z" };
}
function Harness({ initialHash = "/library" }: { initialHash?: string }) {
  const [route, setRoute] = useState(() => {
    window.history.replaceState(null, "", `#${initialHash}`);
    return parseRoute();
  });
  useEffect(() => {
    const sync = () => setRoute(parseRoute());
    window.addEventListener("hashchange", sync);
    return () => window.removeEventListener("hashchange", sync);
  }, []);
  return <LibraryPage {...route} />;
}
async function navigate(element: HTMLElement) {
  await act(async () => { fireEvent.click(element); await new Promise((resolve) => setTimeout(resolve, 0)); });
}
function tab(name: string) { return within(screen.getByRole("navigation", { name: "题库分类" })).getByRole("link", { name }); }
function filter(name: string) { return within(screen.getByRole("group", { name: "阅读 Part 筛选" })).getByRole("button", { name: new RegExp(`^${name} `) }); }
function checkbox(id: string) { return document.querySelector<HTMLInputElement>(`tr[data-item-id="${id}"] input[type="checkbox"]`)!; }
function ids() { return Array.from(document.querySelectorAll("tr[data-item-id]"), (tr) => tr.getAttribute("data-item-id")); }

beforeEach(() => {
  window.history.replaceState(null, "", "#/library");
  mocks.rows = [row("reading-one", "P1"), row("reading-two", "P2"), row("reading-three", "P3"), row("unassigned"), row("audio-one", "Part 1", "listening")];
  mocks.listWriting.mockResolvedValue([{ jobId: "writing-one", title: "写作测试", taskType: "task1", examId: "writing-one",
    promptText: "原文", suggestedWordCount: 150, status: "Draft", createdAt: "", updatedAt: "" }]);
});
afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe("题库信息分层", () => {
  it("按类别显示题目，阅读按 Part 筛选，并保留未设置入口", async () => {
    render(<Harness />);
    expect(ids()).toEqual(["reading-one", "reading-two", "reading-three", "unassigned"]);
    await navigate(filter("P2"));
    expect(ids()).toEqual(["reading-two"]);
    expect(window.location.hash).toBe("#/library?modality=reading&part=P2");
    await navigate(filter("未设置"));
    expect(ids()).toEqual(["unassigned"]);
    await navigate(tab("听力"));
    expect(ids()).toEqual(["audio-one"]);
    expect(screen.queryByRole("group", { name: "阅读 Part 筛选" })).toBeNull();
    await navigate(tab("阅读"));
    expect(ids()).toEqual(["unassigned"]);
    await waitFor(() => expect(mocks.listWriting).toHaveBeenCalledTimes(1));
  });

  it("跨类别、筛选与搜索保留组卷选择，搜索只作用于当前类别", async () => {
    render(<Harness />);
    fireEvent.click(checkbox("reading-one"));
    await navigate(filter("P2"));
    fireEvent.click(checkbox("reading-two"));
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "no-match" } });
    expect(ids()).toEqual([]);
    await navigate(tab("听力"));
    expect(ids()).toEqual(["audio-one"]);
    fireEvent.click(checkbox("audio-one"));
    expect(screen.getByTestId("paper-slot-listening").textContent).toContain("1 篇");
    await navigate(tab("阅读"));
    expect((screen.getByRole("searchbox") as HTMLInputElement).value).toBe("no-match");
    expect(screen.getByTestId("paper-slot-P1").textContent).toContain("reading-one");
    expect(screen.getByTestId("paper-slot-P2").textContent).toContain("reading-two");
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "" } });
    expect(checkbox("reading-two").checked).toBe(true);
    await navigate(filter("全部"));
    expect(checkbox("reading-one").checked).toBe(true);
    await waitFor(() => expect(mocks.listWriting).toHaveBeenCalledTimes(1));
  });

  it("写作独立显示，切换分类保留未保存的写作草稿", async () => {
    render(<Harness initialHash="/library?modality=writing" />);
    const prompt = await screen.findByRole("textbox", { name: "题目内容" });
    fireEvent.change(prompt, { target: { value: "未保存的草稿" } });
    expect(screen.queryByTestId("paper-composer")).toBeNull();
    expect(screen.queryByRole("searchbox")).toBeNull();
    await navigate(tab("阅读"));
    expect(screen.queryByRole("textbox", { name: "题目内容" })).toBeNull();
    await navigate(tab("写作"));
    expect((screen.getByRole("textbox", { name: "题目内容" }) as HTMLTextAreaElement).value).toBe("未保存的草稿");
    expect(mocks.listWriting).toHaveBeenCalledTimes(1);
  });

  it("深链接与路由回退更新类别和阅读筛选", async () => {
    const view = render(<LibraryPage modality="reading" part="P2" />);
    expect(ids()).toEqual(["reading-two"]);
    view.rerender(<LibraryPage modality="listening" />);
    expect(ids()).toEqual(["audio-one"]);
    view.rerender(<LibraryPage modality="reading" part="P3" />);
    expect(ids()).toEqual(["reading-three"]);
    view.rerender(<LibraryPage modality="reading" part="invalid" />);
    expect(ids()).toHaveLength(4);
    await waitFor(() => expect(mocks.listWriting).toHaveBeenCalledTimes(1));
  });

  it("编辑器带入听力题时自动进入听力并预选", async () => {
    render(<Harness initialHash="/library?publish=1&pick=audio-one" />);
    await waitFor(() => expect(ids()).toEqual(["audio-one"]));
    expect(checkbox("audio-one").checked).toBe(true);
    expect(tab("听力").getAttribute("aria-current")).toBe("page");
    await waitFor(() => expect(mocks.listWriting).toHaveBeenCalledTimes(1));
  });
});
