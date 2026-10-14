// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ExamCanvas } from "./ExamCanvas";
import type { IeltsAuthoringIRV2 } from "../types";
import fixture from "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json";

afterEach(cleanup);

function emptyPassage(text: string): IeltsAuthoringIRV2 {
  const authoring = structuredClone(fixture);
  authoring.passage.content[0].children[0].text = text;
  return authoring as unknown as IeltsAuthoringIRV2;
}

describe("cleared passage text", () => {
  it.each(["", "   "])("remains visible and editable after loading %j", (text) => {
    const onTextCommand = vi.fn();
    render(<ExamCanvas authoring={emptyPassage(text)} mode="author" onTextCommand={onTextCommand} />);
    const placeholder = screen.getByRole("button", { name: "编辑文字：（空白）" });
    expect(placeholder.textContent).toBe("点击输入文字");
    fireEvent.click(placeholder);
    const editor = screen.getByRole("textbox", { name: "编辑题目文字" });
    fireEvent.change(editor, { target: { value: "Rewritten passage" } });
    fireEvent.keyDown(editor, { key: "Enter" });
    expect(onTextCommand).toHaveBeenCalledWith({ nodeId: "passage-p1-text", expectedText: text, text: "Rewritten passage" });
  });

  it("does not put the author placeholder in the student preview", () => {
    render(<ExamCanvas authoring={emptyPassage("")} mode="student" />);
    expect(screen.queryByText("点击输入文字")).toBeNull();
  });
});
