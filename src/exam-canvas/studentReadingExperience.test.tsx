// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import fixture from "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json";
import type { AuthoringPatchV2, IeltsAuthoringIRV2 } from "../types";
import { ExamCanvas } from "./ExamCanvas";
import { wrapRangeTextWithHighlight } from "./highlightDom";
import { applyAuthoringV2Patches, inverseAuthoringPatch } from "../services/authoringV2Patches";
import { SelectionInspector } from "../features/editor/SelectionInspector";
vi.mock("../api/tauriCommands", () => ({ resolveAuthoringAssetPreview: vi.fn(async () => undefined) }));
afterEach(cleanup);
function draft(): IeltsAuthoringIRV2 {
  const result = structuredClone(fixture) as IeltsAuthoringIRV2;
  result.passage!.content.unshift({ id: "title", type: "heading", level: 2, children: [{ id: "title-text", type: "text", text: "A reading title", sourceAnchors: [], provenanceStatus: "source" }], sourceAnchors: [], provenanceStatus: "source" });
  return result;
}
describe("student reading experience", () => {
  it("hides editor task labels, preserves instructions, and exposes three zoom levels only in student mode", () => {
    const source = draft();
    const { container, rerender } = render(<ExamCanvas authoring={source} mode="student" />);
    expect(container.querySelector(".v2-task-header h2,.v2-editor-task-badge")).toBeNull();
    expect(container.querySelector(".v2-instruction")!.textContent).not.toBe("");
    const canvas = container.querySelector<HTMLElement>(".exam-canvas-v2")!;
    for (const [size, px] of [["normal", "16px"], ["large", "18px"], ["xlarge", "20px"]]) {
      fireEvent.click(screen.getByRole("button", { name: "Text size " + size }));
      expect(canvas.style.getPropertyValue("--student-font-size")).toBe(px);
      expect(canvas.querySelectorAll(":scope > .reading-pane").length).toBe(2);
    }
    rerender(<ExamCanvas authoring={source} mode="author" />);
    expect(screen.queryByRole("group", { name: "Text size" })).toBeNull();
    expect(canvas.style.getPropertyValue("--student-font-size")).toBe("");
    expect(container.querySelector(".v2-editor-task-badge")).not.toBeNull();
  });
  it("highlights and removes selected passage and question text without changing authoring", () => {
    const source = draft();
    const before = JSON.stringify(source);
    const { container } = render(<ExamCanvas authoring={source} mode="student" />);
    for (const selector of [".v2-passage-pane .v2-text", ".v2-question-pane .v2-text"]) {
      const node = container.querySelector(selector)!.firstChild!;
      const range = document.createRange();
      range.setStart(node, 0); range.setEnd(node, Math.min(8, node.textContent!.length));
      range.getBoundingClientRect = () => ({ top: 140, left: 50, bottom: 160 } as DOMRect);
      window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range);
      fireEvent(document, new Event("selectionchange"));
      fireEvent.click(screen.getByRole("button", { name: "Highlight" }));
      const highlight = container.querySelector<HTMLElement>(".hl")!;
      expect(highlight).not.toBeNull();
      highlight.getBoundingClientRect = () => ({ top: 140, left: 50, bottom: 160 } as DOMRect);
      fireEvent.click(highlight);
      fireEvent.click(screen.getByRole("button", { name: "Remove highlight" }));
      expect(container.querySelector(".hl")).toBeNull();
    }
    expect(JSON.stringify(source)).toBe(before);
  });
  it("keeps input, choice and drag control DOM and bindings intact across a selection", () => {
    const root = document.createElement("div");
    root.innerHTML = '<span>before</span><input value="answer"><label><input type="checkbox">Choice</label><button draggable="true">Drag option</button><select><option>Option</option></select><span>after</span>';
    document.body.append(root);
    const controls = [...root.querySelectorAll("input,button,select")];
    const changed = vi.fn();
    controls.forEach((node) => node.addEventListener("click", changed));
    const range = document.createRange(); range.selectNodeContents(root);
    expect(wrapRangeTextWithHighlight(range, { kind: "highlight" }).applied).toBe(true);
    expect([...root.querySelectorAll("input,button,select")]).toEqual(controls);
    expect(root.querySelector("button .hl,select .hl")).toBeNull();
    controls.forEach((control) => fireEvent.click(control));
    expect(changed).toHaveBeenCalledTimes(controls.length);
    expect((controls[0] as HTMLInputElement).value).toBe("answer");
    expect((controls[1] as HTMLInputElement).checked).toBe(true);
    root.remove();
  });
  it("patches fontScale through undo/redo and serialization, and rejects arbitrary sizes", () => {
    const source = draft();
    const patch: AuthoringPatchV2 = { op: "setNodeAttrs", nodeId: "title", attrs: { fontScale: 1.3 } };
    const inverse = inverseAuthoringPatch(source, patch)!;
    const updated = applyAuthoringV2Patches(source, [patch]);
    expect(updated.passage!.content[0]).toHaveProperty("fontScale", 1.3);
    expect(applyAuthoringV2Patches(updated, [inverse])).toEqual(source);
    const reopened = JSON.parse(JSON.stringify(applyAuthoringV2Patches(applyAuthoringV2Patches(updated, [inverse]), [patch])));
    expect(reopened.passage.content[0].fontScale).toBe(1.3);
    expect(() => applyAuthoringV2Patches(source, [{ ...patch, attrs: { fontScale: 42 as 1 } }])).toThrow("FONT_SCALE_INVALID");
    const { container } = render(<ExamCanvas authoring={reopened} mode="student" />);
    expect(container.querySelector<HTMLElement>(".v2-heading")!.style.getPropertyValue("--node-font-scale")).toBe("1.3");
  });
  it("exposes heading size when inline heading text is selected", () => {
    const onPatch = vi.fn();
    render(<SelectionInspector draft={draft()} selectedId="title-text" onPatch={onPatch} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("标题字号"), { target: { value: "1.5" } });
    expect(onPatch).toHaveBeenCalledWith({ op: "setNodeAttrs", nodeId: "title", attrs: { fontScale: 1.5 } });
  });
});
