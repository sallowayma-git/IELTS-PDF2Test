import { describe, expect, it } from "vitest";
import commonSchema from "../../contracts/common-v2.schema.json";
import contentSchema from "../../contracts/content-doc-v2.schema.json";
import qualitySchema from "../../contracts/quality-report-v2.schema.json";
import probeSchema from "../../contracts/listening-audio-probe-result-v1.schema.json";
import authoringSchema from "../../contracts/ielts-authoring-ir-v2.schema.json";
import Ajv2020 from "ajv/dist/2020";
import fixture from "../../contracts/fixtures/authoring-structure-parity.json";
import { applyAuthoringV2Patches as apply, inverseAuthoringPatch as inverse } from "../services/authoringV2Patches";
import type { AuthoringPatchV2, IeltsAuthoringIRV2 } from "./index";

const initial = fixture.initial as unknown as IeltsAuthoringIRV2;
const patches = fixture.patches as AuthoringPatchV2[];

describe("structural editor patches", () => {
  it("shares every forward document and undo patch with the Rust regression", () => {
    let document = structuredClone(initial);
    const steps = patches.map((patch) => {
      const undo = inverse(document, patch)!;
      const changed = apply(document, [patch]);
      expect(apply(changed, [undo])).toEqual(document);
      const redo = inverse(changed, undo)!;
      expect(apply(apply(changed, [undo]), [redo])).toEqual(changed);
      document = changed;
      return { after: changed, inverse: undo };
    });
    expect(steps).toEqual(fixture.steps);
    expect(document.answerSlots.q6).toBeUndefined();
    expect(document.answerSlots.q7.questionNumber).toBe(7);
    expect(document.taskGroups[0].optionBank).toBeDefined();
    expect(document.taskGroups[0].responseGroups[0].prompt?.some((n) => n.id === "q7-prompt")).toBe(true);
    expect(JSON.stringify(document)).not.toContain('"slotId":"q1"');
  });

  it("keeps passage paragraphs/maps and shared banks; removes an unreferenced bank", () => {
    let document = structuredClone(initial);
    document.answerSlots.q1.hostNodeId = initial.passage!.content[0].id;
    document.answerSlots.q1.hostType = "passage_paragraph";
    const after = apply(document, [patches[0]]);
    expect(after.passage).toMatchObject({ paragraphMap: initial.passage!.paragraphMap });
    expect(after.passage!.content[0]).toEqual(initial.passage!.content[0]);
    document = apply(initial, [patches[2]]);
    expect(document.taskGroups[0].optionBank).toBeDefined();
    // A different response can use inline options while the removed response owns the bank.
    const shared = structuredClone(initial);
    delete shared.taskGroups[0].responseGroups[0].optionBankRef;
    const noBank = apply(shared, [patches[2]]);
    expect(noBank.taskGroups[0].optionBank).toBeUndefined();
    expect(apply(noBank, [inverse(shared, patches[2])!])).toEqual(shared);
  });

  it("cleans nested embedded slots and hotspots, and restores all references on undo", () => {
    const document = structuredClone(initial);
    const group = document.taskGroups[0].responseGroups[0];
    group.prompt = [{ type: "table", id: "table", rows: [{ type: "table_row", id: "row", cells: [{ type: "table_cell", id: "cell", rowSpan: 1, colSpan: 1, children: [{ type: "answer_slot", id: "embedded", slotId: "q1", displayLabel: "1", inline: true, sourceAnchors: [], provenanceStatus: "manual" }], sourceAnchors: [], provenanceStatus: "manual" }], sourceAnchors: [], provenanceStatus: "manual" }], sourceAnchors: [], provenanceStatus: "manual" }];
    const patch: AuthoringPatchV2 = { ...patches[0], nodeId: "embedded" } as AuthoringPatchV2;
    const changed = apply(document, [patch]);
    expect(JSON.stringify(changed)).not.toContain('"slotId":"q1"');
    expect(apply(changed, [inverse(document, patch)!])).toEqual(document);
  });

  it("rejects last-slot, wrong-group and stale undo without mutating the input", () => {
    expect(() => apply(initial, [{ op: "deleteAnswerSlot", taskId: "task-2", responseGroupId: "group-2-0", slotId: "q5" }])).toThrow("LAST_ANSWER_SLOT");
    expect(() => apply(initial, [{ op: "deleteAnswerSlot", taskId: "task-1", responseGroupId: "group-1-0", slotId: "q5" }])).toThrow("SLOT_NOT_FOUND");
    const changed = apply(initial, [patches[0]]);
    const undo = inverse(initial, patches[0])!;
    changed.taskGroups[0].responseGroups[0].prompt!.push({ type: "paragraph", id: "new", children: [], sourceAnchors: [], provenanceStatus: "manual" });
    const before = structuredClone(changed);
    expect(() => apply(changed, [undo])).toThrow("RESTORE_CONFLICT");
    expect(changed).toEqual(before);
  });

  it("manual question content nodes can be deleted and restored", () => {
    const node = { type: "paragraph", id: "new-content", children: [], sourceAnchors: [], provenanceStatus: "manual" } as const;
    const document = apply(initial, [{ op: "insertNode", target: { kind: "taskInstructions", taskId: "task-1" }, index: 0, node: structuredClone(node) as never }]);
    const patch: AuthoringPatchV2 = { op: "deleteNode", nodeId: node.id };
    expect(apply(document, [patch]).taskGroups[0].instructions).toEqual([]);
    expect(apply(apply(document, [patch]), [inverse(document, patch)!])).toEqual(document);
  });

  it("validates all structural patches and undo through the bundled schema", () => {
    const ajv = new Ajv2020({ strict: false, validateFormats: false });
    for (const schema of [commonSchema, contentSchema, qualitySchema, probeSchema, authoringSchema]) ajv.addSchema(schema);
    const validate = ajv.getSchema("https://contracts.ielts-author-studio.dev/phase1/ielts-authoring-ir-v2.schema.json#/$defs/authoringStructurePatch")!;
    for (const patch of [...patches, ...fixture.steps.map((s) => s.inverse)]) expect(validate(patch), JSON.stringify(validate.errors)).toBe(true);
    expect(validate({ op: "deleteTaskGroup" })).toBe(false);
    expect(validate({ op: "deleteResponseGroup", taskId: "t", responseGroupId: "g", extra: true })).toBe(false);
  });

  it("preserves listening audio, declared missing numbers, and unrelated Part ordering", () => {
    const document = fixture.listeningInitial as unknown as IeltsAuthoringIRV2;
    const changed = apply(document, [patches[3]]);
    expect(changed.listening).toEqual(fixture.listeningExpected);
    expect(apply(changed, [inverse(document, patches[3])!])).toEqual(document);
  });

  it("rejects duplicate IDs anywhere inside a new per-slot prompt", () => {
    const patch = patches[1] as Extract<AuthoringPatchV2, { op: "insertAnswerSlot" }>;
    const duplicate = { ...patch, promptNode: { type: "paragraph", id: "new-prompt", children: [{ type: "text", id: "text-1", text: "Duplicate", sourceAnchors: [], provenanceStatus: "manual" }], sourceAnchors: [], provenanceStatus: "manual" } };
    expect(() => apply(initial, [duplicate as AuthoringPatchV2])).toThrow("PROMPT_INVALID");
  });
});
