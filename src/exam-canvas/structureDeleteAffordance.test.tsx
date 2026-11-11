// @vitest-environment jsdom
//
// 编辑页结构删除 / 添加入口（作者模式）：
// - 每个逐题行末尾有 ×「删除第 N 题」，点击派发 answer-slot.delete；组内只剩一题时禁用并说明；
// - 逐题列表底部「＋ 添加一题」派发 answer-slot.append，指向所在作答组；
// - 作答组（多组时）与整道大题的「⋯」次级菜单：删除前必须二次确认，确认框写明题号范围与撤销提示；
// - 云端校核锁定或非作者模式时，这些入口都不渲染。
//
// 证据层级：组件级（jsdom）。CSS 的常显/尺寸由真实应用的 CDP 脚本截图证明。

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { ExamCanvas } from "./ExamCanvas";
import { structureDraft } from "./structureDeleteFixture";

vi.mock("../api/tauriCommands", () => ({ resolveAuthoringAssetPreview: vi.fn(async () => undefined) }));

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("逐题删除与添加", () => {
  it("每个逐题行末尾有 ×，点击派发 answer-slot.delete（不带 nodeId）", () => {
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={structureDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    const rows = container.querySelectorAll<HTMLElement>('.v2-slot-question[data-question-id="q14"], .v2-slot-question[data-question-id="q15"]');
    expect(rows).toHaveLength(2);
    const remove = rows[1].querySelector<HTMLButtonElement>(".v2-question-delete")!;
    expect(remove.getAttribute("aria-label")).toBe("删除第 15 题");
    expect(remove.disabled).toBe(false);
    fireEvent.click(remove);
    expect(onStructureAction).toHaveBeenCalledWith({ type: "answer-slot.delete", slotId: "q15" });
  });

  it("组内只剩一题时 × 禁用，并说明每组至少保留一题、可删除整组", () => {
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={structureDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    const last = container.querySelector<HTMLButtonElement>('.v2-slot-question[data-question-id="q16"] .v2-question-delete')!;
    expect(last.disabled).toBe(true);
    expect(last.title).toBe("每组至少保留一题，可删除整组");
    fireEvent.click(last);
    expect(onStructureAction).not.toHaveBeenCalled();
  });

  it("「＋ 添加一题」在每个作答组的逐题列表底部，派发 answer-slot.append 指向该组", () => {
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={structureDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    const groupA = container.querySelector<HTMLElement>('section.v2-response-group[data-response-group-id="rg-a"]')!;
    const add = groupA.querySelector<HTMLButtonElement>(".v2-question-append")!;
    expect(add.textContent).toContain("添加一题");
    expect(add.parentElement?.classList.contains("v2-slot-list")).toBe(true);
    expect(add.parentElement?.lastElementChild).toBe(add);
    fireEvent.click(add);
    expect(onStructureAction).toHaveBeenCalledWith({ type: "answer-slot.append", taskId: "headings", responseGroupId: "rg-a" });
  });
});

describe("作答组与整道大题的删除（次级菜单 + 二次确认）", () => {
  it("多作答组时每组都有「⋯」菜单；确认框取消则不派发", () => {
    const onStructureAction = vi.fn();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const { container } = render(<ExamCanvas authoring={structureDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    const groupB = container.querySelector<HTMLElement>('section.v2-response-group[data-response-group-id="rg-b"]')!;
    const deleteGroup = Array.from(groupB.querySelectorAll<HTMLButtonElement>(".v2-structure-menu-items button")).find((button) => button.textContent === "删除这组题")!;
    fireEvent.click(deleteGroup);
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(confirm.mock.calls[0][0]).toContain("第 16 题");
    expect(confirm.mock.calls[0][0]).toContain("撤销");
    expect(onStructureAction).not.toHaveBeenCalled();
  });

  it("确认后派发 response-group.delete", () => {
    const onStructureAction = vi.fn();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const { container } = render(<ExamCanvas authoring={structureDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    const groupB = container.querySelector<HTMLElement>('section.v2-response-group[data-response-group-id="rg-b"]')!;
    fireEvent.click(Array.from(groupB.querySelectorAll<HTMLButtonElement>(".v2-structure-menu-items button")).find((button) => button.textContent === "删除这组题")!);
    expect(onStructureAction).toHaveBeenCalledWith({ type: "response-group.delete", taskId: "headings", responseGroupId: "rg-b" });
  });

  it("整道大题的菜单：确认框写明题号范围（第 14–16 题），确认后派发 task-group.delete", () => {
    const onStructureAction = vi.fn();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const { container } = render(<ExamCanvas authoring={structureDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    const header = container.querySelector<HTMLElement>('.v2-task-header')!;
    fireEvent.click(Array.from(header.querySelectorAll<HTMLButtonElement>(".v2-structure-menu-items button")).find((button) => button.textContent === "删除整道大题")!);
    expect(confirm.mock.calls[0][0]).toContain("第 14–16 题");
    expect(confirm.mock.calls[0][0]).toContain("可用撤销");
    expect(onStructureAction).toHaveBeenCalledWith({ type: "task-group.delete", taskId: "headings" });
  });

  it("只有一个作答组的大题不再提供「删除这组题」，只留整道大题", () => {
    const draft = structureDraft();
    draft.taskGroups[0].responseGroups = [draft.taskGroups[0].responseGroups[0]];
    delete draft.answerSlots.q16;
    delete draft.answerKey.q16;
    const { container } = render(<ExamCanvas authoring={draft} mode="author" onStructureAction={vi.fn()} onAnswerChange={vi.fn()} />);
    const menus = container.querySelectorAll(".v2-structure-menu");
    expect(menus).toHaveLength(1);
    expect(menus[0].closest(".v2-task-header")).toBeTruthy();
  });
});

describe("入口的可见条件", () => {
  it("云端校核锁定（locked）时不渲染任何结构删除/添加入口", () => {
    const { container } = render(<ExamCanvas authoring={structureDraft()} mode="author" locked onStructureAction={vi.fn()} onAnswerChange={vi.fn()} />);
    expect(container.querySelector(".v2-question-delete")).toBeNull();
    expect(container.querySelector(".v2-question-append")).toBeNull();
    expect(container.querySelector(".v2-structure-menu")).toBeNull();
  });

  it("学生预览不渲染任何结构入口", () => {
    const { container } = render(<ExamCanvas authoring={structureDraft()} mode="student" onAnswerChange={vi.fn()} />);
    expect(container.querySelector(".v2-question-delete")).toBeNull();
    expect(container.querySelector(".v2-question-append")).toBeNull();
    expect(container.querySelector(".v2-structure-menu")).toBeNull();
  });
});
