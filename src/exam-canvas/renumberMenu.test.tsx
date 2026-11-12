// @vitest-environment jsdom
// 大题「更多」菜单的「按顺序重新编号本题组」：二次确认后派发 questions.renumber；取消则不派发。
// 错误码映射（矩阵追加 / 重新编号）也在此一并覆盖。证据层级：组件级（jsdom）+ 单元。
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { ExamCanvas } from "./ExamCanvas";
import { structureDraft } from "./structureDeleteFixture";
import { toUserFacingError } from "../utils/userFacingError";

vi.mock("../api/tauriCommands", () => ({ resolveAuthoringAssetPreview: vi.fn(async () => undefined) }));

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function outOfOrderDraft() {
  const draft = structureDraft();
  draft.answerSlots.q15.questionNumber = 17;
  draft.answerSlots.q15.displayLabel = "17";
  return draft;
}

function renumberItem(container: HTMLElement) {
  return [...container.querySelectorAll<HTMLButtonElement>(".v2-task-header .v2-structure-menu-items button")]
    .find((button) => button.textContent === "按顺序重新编号本题组")!;
}

describe("大题菜单：按顺序重新编号本题组", () => {
  it("确认框写明题号前后变化与「题号会改变，答案与统计不受影响，可撤销」；确认后派发 questions.renumber", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={outOfOrderDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    fireEvent.click(renumberItem(container));
    expect(confirm).toHaveBeenCalledTimes(1);
    const message = String(confirm.mock.calls[0][0]);
    expect(message).toContain("原题号 14、17、16 → 新题号 14–16");
    expect(message).toContain("题号会改变，答案与统计不受影响，可撤销");
    expect(onStructureAction).toHaveBeenCalledWith({ type: "questions.renumber", scope: "task", taskId: "headings" });
  });

  it("取消确认则不派发", () => {
    vi.spyOn(window, "confirm").mockReturnValue(false);
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={outOfOrderDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    fireEvent.click(renumberItem(container));
    expect(onStructureAction).not.toHaveBeenCalled();
  });

  it("题号已经连续时不弹确认，直接说明无需重新编号", () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const alert = vi.spyOn(window, "alert").mockImplementation(() => undefined);
    const onStructureAction = vi.fn();
    const { container } = render(<ExamCanvas authoring={structureDraft()} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    fireEvent.click(renumberItem(container));
    expect(confirm).not.toHaveBeenCalled();
    expect(alert).toHaveBeenCalledWith("题号已经是连续的，无需重新编号。");
    expect(onStructureAction).not.toHaveBeenCalled();
  });
});

describe("结构错误码 → 中文", () => {
  it.each([
    ["AUTHORING_PATCH_RESPONSE_GROUP_TEMPLATE_REQUIRED", "没有可参照的作答组"],
    ["AUTHORING_PATCH_RESPONSE_GROUP_ALREADY_EXISTS:rg-x", "作答组编号已被占用"],
    ["AUTHORING_PATCH_RESPONSE_GROUP_SLOT_MISMATCH", "作答位与作答组不一致"],
    ["AUTHORING_PATCH_RENUMBER_COLLISION:4", "与其他题组的第 4 题冲突，请改用全卷重新编号"],
    ["AUTHORING_PATCH_RENUMBER_SCOPE_INVALID", "重新编号的范围无效"],
    ["AUTHORING_PATCH_RENUMBER_START_INVALID", "起始题号无效"],
    ["AUTHORING_PATCH_RENUMBER_SLOT_REUSED", "同一道题出现在多处"],
  ])("%s", (raw, expected) => {
    expect(toUserFacingError(new Error(raw), "x").userMessage).toContain(expected);
  });
});
