// @vitest-environment jsdom
// Real canvas rendering and interaction; this does not exercise native Rust IPC.
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ExamCanvas } from "./ExamCanvas";
import { SelectionInspector } from "../features/editor/SelectionInspector";
import { ListeningHeader } from "./ListeningHeader";
import { findTargetElement } from "../features/editor/locate";
import type { IeltsAuthoringIRV2 } from "../types";
import fixture from "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json";

afterEach(cleanup);
const draft = () => structuredClone(fixture) as unknown as IeltsAuthoringIRV2;

describe("cloud review only disables editing", () => {
  it("keeps text selection and question navigation, while disabling text, choices and structure edits", () => {
    const onSelect = vi.fn();
    const onTextCommand = vi.fn();
    const onAnswerChange = vi.fn();
    const onStructureAction = vi.fn();
    const { container, rerender } = render(<ExamCanvas authoring={draft()} mode="author" locked
      {...{ onSelect, onTextCommand, onAnswerChange, onStructureAction }} />);
    expect(screen.getByTestId("exam-canvas-v2-author").hasAttribute("inert")).toBe(false);
    const text = container.querySelector<HTMLElement>('[data-editor-id="passage-p1-text"]')!;
    fireEvent.click(text);
    fireEvent.keyDown(text, { key: "Enter" });
    expect(screen.queryByRole("textbox", { name: "编辑题目文字" })).toBeNull();
    expect(screen.queryByLabelText("添加选项")).toBeNull();
    expect(screen.queryByLabelText(/拖动调整选项/)).toBeNull();
    const choices = container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]');
    expect(choices.length).toBeGreaterThan(0);
    choices.forEach((input) => { expect(input.disabled).toBe(true); fireEvent.click(input); });
    const slot = findTargetElement("q14", draft())!;
    slot.scrollIntoView = vi.fn();
    fireEvent.click(screen.getByRole("button", { name: "14" }));
    expect(onSelect).toHaveBeenCalledWith("q14");
    expect(slot.scrollIntoView).toHaveBeenCalled();
    expect(onTextCommand).not.toHaveBeenCalled();
    expect(onAnswerChange).not.toHaveBeenCalled();
    expect(onStructureAction).not.toHaveBeenCalled();
    rerender(<ExamCanvas authoring={draft()} mode="author" locked={false}
      {...{ onSelect, onTextCommand, onAnswerChange, onStructureAction }} />);
    fireEvent.click(container.querySelector('[data-editor-id="passage-p1-text"]')!);
    expect(screen.getByRole("textbox", { name: "编辑题目文字" })).toBeTruthy();
  });

  it("makes text answers read-only and cancels an editor already open when review begins", () => {
    const authoring = draft();
    authoring.taskGroups[0].responseGroups[0].assignment = "per_slot";
    authoring.taskGroups[0].responseGroups[0].kind = "text_entry";
    for (const slot of Object.values(authoring.answerSlots)) slot.interaction = "text";
    const onTextCommand = vi.fn();
    const onAnswerChange = vi.fn();
    const { container, rerender } = render(<ExamCanvas authoring={authoring} mode="author" onTextCommand={onTextCommand} onAnswerChange={onAnswerChange} />);
    fireEvent.click(container.querySelector('[data-editor-id="passage-p1-text"]')!);
    expect(screen.getByRole("textbox", { name: "编辑题目文字" })).toBeTruthy();
    rerender(<ExamCanvas authoring={authoring} mode="author" locked onTextCommand={onTextCommand} onAnswerChange={onAnswerChange} />);
    expect(screen.queryByRole("textbox", { name: "编辑题目文字" })).toBeNull();
    const answers = container.querySelectorAll<HTMLInputElement>(".v2-text-answer");
    expect(answers.length).toBeGreaterThan(0);
    answers.forEach((input) => { expect(input.readOnly).toBe(true); fireEvent.change(input, { target: { value: "changed" } }); });
    expect(onTextCommand).not.toHaveBeenCalled();
    expect(onAnswerChange).not.toHaveBeenCalled();
  });

  it("keeps image settings browsable and closable while preventing patches", () => {
    const authoring = draft();
    authoring.passage!.content.push({ type: "figure", id: "qa-image", assetId: "asset-1", sourceAnchors: [], provenanceStatus: "source", hotspots: [], display: {} });
    const onPatch = vi.fn();
    const onClose = vi.fn();
    const { container } = render(<SelectionInspector draft={authoring} selectedId="qa-image" locked onPatch={onPatch} onClose={onClose} />);
    expect(container.querySelector("fieldset")?.disabled).toBe(true);
    fireEvent.change(container.querySelector('input[type="number"]')!, { target: { value: "0.3" } });
    fireEvent.click(screen.getByRole("button", { name: "关闭设置" }));
    expect(onPatch).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("disables audio replacement without making the listening header inert", () => {
    const { container } = render(<ListeningHeader itemId="qa" mode="author" locked selectedPart={1}
      parts={[{ ordinal: 1, label: "Part 1", taskIds: [] }]} onAudioChanged={vi.fn()} />);
    expect((screen.getByTestId("listening-add-audio") as HTMLButtonElement).disabled).toBe(true);
    expect(container.querySelector("[inert]")).toBeNull();
  });
});
