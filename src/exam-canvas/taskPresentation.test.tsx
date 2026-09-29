// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ExamCanvas } from "./ExamCanvas";
import { compileStructureAction } from "./structureActions";
import type { AnswerSlotV2, IeltsAuthoringIRV2, OptionV2, TaskGroupV2 } from "../types";

vi.mock("../api/tauriCommands", () => ({ resolveAuthoringAssetPreview: vi.fn(async () => undefined) }));

function option(optionId: string, label: string, text = label): OptionV2 {
  return { optionId, label, content: [{ id: `${optionId}-text`, type: "text", text, sourceAnchors: [], provenanceStatus: "source" }], sourceAnchors: [] };
}

const quality = {
  schemaVersion: "QualityReportV2" as const, state: "review_required" as const,
  documentScore: 0, sourceCoverage: 0, coverageLedger: [],
  coverageStatus: { physicalShadow: "missing" as const, complete: false, significantSourceNodeCount: 0, explainedSourceNodeCount: 0, unassignedSourceNodeIds: [] },
  compilerProbes: {
    v2Runtime: { status: "passed" as const, schemaVersion: "", issueCodes: [], details: [] },
    v1Compatibility: { status: "passed" as const, schemaVersion: "", issueCodes: [], details: [] }
  },
  taskScores: {}, hardFailures: [], issues: [], metrics: {}, evaluatedAt: "", evaluatorVersion: ""
};

function headingDraft(answerKey: IeltsAuthoringIRV2["answerKey"] = {
  q14: { kind: "unresolved" }, q15: { kind: "unresolved" }
}): IeltsAuthoringIRV2 {
  const slots: Record<string, AnswerSlotV2> = {
    q14: { slotId: "q14", questionNumber: 14, displayLabel: "14", hostNodeId: "passage-a", hostType: "passage_paragraph", interaction: "dragdrop", participation: "scoring", sourceAnchors: [], confidence: 1 },
    q15: { slotId: "q15", questionNumber: 15, displayLabel: "15", hostNodeId: "passage-b", hostType: "passage_paragraph", interaction: "dragdrop", participation: "scoring", sourceAnchors: [], confidence: 1 }
  };
  const task: TaskGroupV2 = {
    taskId: "headings", taskType: "matching_headings", displayRange: { kind: "range", start: 14, end: 15 }, instructions: [],
    instructionSignature: { normalizedText: "", taskType: "matching_headings", expectedQuestionNumbers: [14, 15], expectedSlotCount: 2, evidenceAnchors: [], confidence: 1 },
    responseGroups: [{ responseGroupId: "headings-rg", kind: "matching", slotIds: ["q14", "q15"], optionBankRef: "headings-bank", cardinality: { min: 1, max: 1, exact: 1 }, assignment: "per_slot", scoringPolicy: "per_slot_binary", duplicatePolicy: "reject_submission", allowOptionReuse: false, sourceAnchors: [] }],
    optionBank: { optionBankId: "headings-bank", scope: "task_group", options: [option("heading-i", "i", "First heading"), option("heading-iv", "iv", "Fourth heading")], allowReuse: false, sourceAnchors: [] },
    sourceAnchors: [], quality: { score: 1, sourceCoverage: 1, hardFailures: [] }, reviewState: "unreviewed"
  };
  const paragraph = (id: string, label: string, text: string) => ({ type: "paragraph" as const, id, paragraphLabel: label, children: [{ type: "text" as const, id: `${id}-text`, text, sourceAnchors: [], provenanceStatus: "source" as const }], sourceAnchors: [], provenanceStatus: "source" as const });
  return {
    schemaVersion: "IeltsAuthoringIRV2", jobId: "job-heading", exam: { examId: "exam", title: "Headings", language: "en", tags: [], sourceFiles: [] }, modality: "reading",
    passage: { content: [paragraph("passage-a", "A", "Paragraph A text."), paragraph("passage-b", "B", "Paragraph B text.")], paragraphMap: { A: "passage-a", B: "passage-b" }, sourceAnchors: [] },
    taskGroups: [task], answerSlots: slots, answerKey, assets: [], sourceDocumentId: "source", quality,
    audit: { revision: 1, source: "auto_extract", humanVerified: false, llmUsed: false, updatedAt: "", notes: [] }
  };
}

afterEach(cleanup);

describe("task presentation in the authoring canvas", () => {
  it("places each heading target before its source paragraph and offers a shared Roman heading pool", () => {
    const { container } = render(<ExamCanvas authoring={headingDraft()} mode="author" onAnswerChange={vi.fn()} onStructureAction={vi.fn()} />);
    const targetA = container.querySelector('[data-answer-drop-slot="q14"]')!;
    const paragraphA = container.querySelector('[data-editor-id="passage-a"]')!;
    expect(Boolean(targetA.compareDocumentPosition(paragraphA) & Node.DOCUMENT_POSITION_FOLLOWING)).toBe(true);
    expect(targetA.textContent).toContain("Paragraph A (14)");
    expect(container.querySelectorAll('[data-option-pool-task="headings"] [data-option-label]')).toHaveLength(2);
    expect(screen.getByText("First heading")).toBeTruthy();
  });

  it("uses pointer drops for answers, clears from a target to the pool, and swaps occupied targets", () => {
    const onAnswerChange = vi.fn();
    const { container, rerender } = render(<ExamCanvas authoring={headingDraft()} mode="author" onAnswerChange={onAnswerChange} />);
    const poolOption = container.querySelector('[data-option-label="i"]')!;
    const targetA = container.querySelector('[data-answer-drop-slot="q14"]')!;
    fireEvent.pointerDown(poolOption, { button: 0, pointerId: 1, clientX: 1, clientY: 1 });
    fireEvent.pointerMove(targetA, { pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(targetA, { pointerId: 1, clientX: 10, clientY: 10 });
    expect(onAnswerChange).toHaveBeenLastCalledWith("q14", { kind: "option", labels: ["i"], assignment: "per_slot" });

    onAnswerChange.mockClear();
    const assigned = headingDraft({ q14: { kind: "option", labels: ["i"], assignment: "per_slot" }, q15: { kind: "option", labels: ["iv"], assignment: "per_slot" } });
    rerender(<ExamCanvas authoring={assigned} mode="author" onAnswerChange={onAnswerChange} />);
    const source = container.querySelector('[data-answer-drop-slot="q14"]')!;
    const target = container.querySelector('[data-answer-drop-slot="q15"]')!;
    fireEvent.pointerDown(source, { button: 0, pointerId: 2, clientX: 1, clientY: 1 });
    fireEvent.pointerMove(target, { pointerId: 2, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(target, { pointerId: 2, clientX: 10, clientY: 10 });
    expect(onAnswerChange).toHaveBeenNthCalledWith(1, "q14", { kind: "option", labels: ["iv"], assignment: "per_slot" });
    expect(onAnswerChange).toHaveBeenNthCalledWith(2, "q15", { kind: "option", labels: ["i"], assignment: "per_slot" });

    onAnswerChange.mockClear();
    const pool = container.querySelector('[data-option-pool-task="headings"]')!;
    fireEvent.pointerDown(source, { button: 0, pointerId: 3, clientX: 1, clientY: 1 });
    fireEvent.pointerMove(pool, { pointerId: 3, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(pool, { pointerId: 3, clientX: 10, clientY: 10 });
    expect(onAnswerChange).toHaveBeenCalledWith("q14", { kind: "option", labels: [], assignment: "per_slot" });
  });

  it("clears a placed heading when clicked", () => {
    const onAnswerChange = vi.fn();
    const assigned = headingDraft({ q14: { kind: "option", labels: ["iv"], assignment: "per_slot" }, q15: { kind: "unresolved" } });
    const { container } = render(<ExamCanvas authoring={assigned} mode="author" onAnswerChange={onAnswerChange} />);
    fireEvent.click(container.querySelector('[data-answer-drop-slot="q14"]')!);
    expect(onAnswerChange).toHaveBeenCalledWith("q14", { kind: "option", labels: [], assignment: "per_slot" });
  });

  it("renders a dragdrop control on each ordinary matching response row", () => {
    const draft = headingDraft();
    const task = draft.taskGroups[0];
    task.taskType = "matching_features";
    task.responseGroups[0].slotIds = ["q14", "q15"];
    draft.answerSlots.q14 = { ...draft.answerSlots.q14, hostNodeId: undefined, hostType: "prompt", interaction: "dragdrop" };
    draft.answerSlots.q15 = { ...draft.answerSlots.q15, hostNodeId: undefined, hostType: "prompt", interaction: "dragdrop" };
    const onAnswerChange = vi.fn();
    const { container } = render(<ExamCanvas authoring={draft} mode="author" onAnswerChange={onAnswerChange} />);
    const firstRow = container.querySelector('.v2-slot-question[data-question-id="q14"]')!;
    const target = firstRow.querySelector('[data-answer-drop-slot="q14"]')!;
    expect(target.textContent).toContain("Drop option here");
    expect(container.querySelectorAll('[data-option-pool-task="headings"] [data-option-label]')).toHaveLength(2);
    const poolOption = container.querySelector('[data-option-label="i"]')!;
    fireEvent.pointerDown(poolOption, { button: 0, pointerId: 5, clientX: 1, clientY: 1 });
    fireEvent.pointerMove(target, { pointerId: 5, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(target, { pointerId: 5, clientX: 10, clientY: 10 });
    expect(onAnswerChange).toHaveBeenCalledWith("q14", { kind: "option", labels: ["i"], assignment: "per_slot" });
  });

  it("greys out a consumed option when reuse is forbidden and lets the author change a slot's paragraph", () => {
    const onStructureAction = vi.fn();
    const assigned = headingDraft({ q14: { kind: "option", labels: ["i"], assignment: "per_slot" }, q15: { kind: "unresolved" } });
    const { container } = render(<ExamCanvas authoring={assigned} mode="author" onStructureAction={onStructureAction} onAnswerChange={vi.fn()} />);
    expect(container.querySelector<HTMLButtonElement>('[data-option-label="i"]')?.disabled).toBe(true);
    const targetSelector = screen.getByLabelText("Paragraph target for 14");
    fireEvent.change(targetSelector, { target: { value: "passage-b" } });
    expect(onStructureAction).toHaveBeenCalledWith({ type: "answer-slot.host.set", slotId: "q14", hostNodeId: "passage-b" });
    expect(compileStructureAction(assigned, { type: "answer-slot.host.set", slotId: "q14", hostNodeId: "passage-b" })).toEqual({ op: "setAnswerSlotHost", slotId: "q14", hostNodeId: "passage-b" });
  });

  it("renders the select interaction as a dropdown and keeps matching-information as a radio matrix", () => {
    const draft = headingDraft();
    const selectTask = structuredClone(draft.taskGroups[0]);
    selectTask.taskId = "select-task";
    selectTask.taskType = "plan_map_label_completion";
    selectTask.responseGroups[0].responseGroupId = "select-rg";
    selectTask.responseGroups[0].kind = "choice";
    selectTask.optionBank!.optionBankId = "select-bank";
    selectTask.responseGroups[0].optionBankRef = "select-bank";
    draft.taskGroups = [selectTask];
    draft.answerSlots.q14 = { ...draft.answerSlots.q14, hostType: "prompt", hostNodeId: undefined, interaction: "select" };
    draft.answerSlots.q15 = { ...draft.answerSlots.q15, hostType: "prompt", hostNodeId: undefined, interaction: "select" };
    const { container } = render(<ExamCanvas authoring={draft} mode="student" />);
    expect(container.querySelectorAll('select[data-question-id]').length).toBe(2);
    expect(container.querySelector('input[type="radio"][name="q14"]')).toBeNull();

    const matrixDraft = headingDraft();
    const matrixTask = matrixDraft.taskGroups[0];
    matrixTask.taskType = "matching_information";
    matrixTask.responseGroups = ["q14", "q15"].map((slotId, index) => ({
      ...matrixTask.responseGroups[0], responseGroupId: `matrix-rg-${index}`, slotIds: [slotId], kind: "matching" as const
    }));
    matrixTask.optionBank!.options = [option("paragraph-a", "A", "Paragraph A"), option("paragraph-b", "B", "Paragraph B")];
    matrixDraft.answerSlots.q14 = { ...matrixDraft.answerSlots.q14, hostNodeId: undefined, hostType: "prompt", interaction: "radio" };
    matrixDraft.answerSlots.q15 = { ...matrixDraft.answerSlots.q15, hostNodeId: undefined, hostType: "prompt", interaction: "radio" };
    const matrix = render(<ExamCanvas authoring={matrixDraft} mode="student" />);
    expect(matrix.container.querySelector('[data-testid="matching-matrix"]')).not.toBeNull();
    expect(matrix.container.querySelectorAll('[data-testid="matching-matrix"] input[type="radio"]')).toHaveLength(4);
  });

  it("keeps a word-bank summary answer inline and exposes its shared option pool", () => {
    const draft = headingDraft({ q14: { kind: "unresolved" }, q15: { kind: "unresolved" } });
    const task = draft.taskGroups[0];
    task.taskId = "summary";
    task.taskType = "summary_completion";
    task.displayRange = { kind: "range", start: 14, end: 14 };
    task.responseGroups = [{ ...task.responseGroups[0], responseGroupId: "summary-rg", slotIds: ["q14"], kind: "matching", optionBankRef: "headings-bank" }];
    task.stimulus = [{
      type: "paragraph", id: "summary-text", sourceAnchors: [], provenanceStatus: "source",
      children: [
        { type: "text", id: "summary-before", text: "A sentence with ", sourceAnchors: [], provenanceStatus: "source" },
        { type: "answer_slot", id: "summary-answer-node", slotId: "q14", displayLabel: "14", inline: true, sourceAnchors: [], provenanceStatus: "source" },
        { type: "text", id: "summary-after", text: " at the end.", sourceAnchors: [], provenanceStatus: "source" }
      ]
    }];
    draft.answerSlots.q14 = { ...draft.answerSlots.q14, hostType: "paragraph", hostNodeId: "summary-text", interaction: "dragdrop" };
    delete draft.answerSlots.q15;
    delete draft.answerKey.q15;
    const onAnswerChange = vi.fn();
    const { container } = render(<ExamCanvas authoring={draft} mode="author" onAnswerChange={onAnswerChange} />);
    const inlineTarget = container.querySelector('[data-answer-drop-slot="q14"]')!;
    expect(inlineTarget.closest(".v2-stimulus")).not.toBeNull();
    expect(container.querySelector('.v2-slot-question[data-question-id="q14"]')).toBeNull();
    const poolOption = container.querySelector('[data-option-label="i"]')!;
    fireEvent.pointerDown(poolOption, { button: 0, pointerId: 4, clientX: 1, clientY: 1 });
    fireEvent.pointerMove(inlineTarget, { pointerId: 4, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(inlineTarget, { pointerId: 4, clientX: 10, clientY: 10 });
    expect(onAnswerChange).toHaveBeenCalledWith("q14", { kind: "option", labels: ["i"], assignment: "per_slot" });
  });
});
