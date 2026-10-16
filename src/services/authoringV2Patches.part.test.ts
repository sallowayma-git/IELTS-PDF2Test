import { describe, expect, it } from "vitest";
import type { AuthoringPatchV2, IeltsAuthoringIRV2 } from "../types";
import { applyAuthoringV2Patches, inverseAuthoringPatch } from "./authoringV2Patches";

describe("reading Part patch", () => {
  const draft = {
    modality: "reading", exam: { examId: "p1-stable", title: "24. P1 The Pearls", category: "P1" }
  } as IeltsAuthoringIRV2;

  it("round trips through JSON and inverse without changing identity or title", () => {
    const patch: AuthoringPatchV2 = JSON.parse('{"op":"setExamCategory","category":"P3"}');
    const inverse = inverseAuthoringPatch(draft, patch)!;
    const changed = applyAuthoringV2Patches(draft, [patch]);
    expect(changed.exam).toEqual({ ...draft.exam, category: "P3" });
    expect(draft.exam.category).toBe("P1");
    expect(applyAuthoringV2Patches(changed, [inverse])).toEqual(draft);
  });

  it("rejects unsupported categories and listening documents", () => {
    expect(() => applyAuthoringV2Patches(draft, [{ op: "setExamCategory", category: "Part 1" } as never])).toThrow("CATEGORY_INVALID");
    expect(() => applyAuthoringV2Patches({ ...draft, modality: "listening" }, [{ op: "setExamCategory", category: "P2" }])).toThrow("READING_REQUIRED");
  });
});
