import { describe, expect, it } from "vitest";
import type { IeltsAuthoringIRV2 } from "../types";
import { isListening, listeningParts, listeningStructureMissing, visibleTaskIds } from "./listeningWorkspace";

// Evidence level: pure unit (listening workspace projection).

function draft(overrides: Partial<IeltsAuthoringIRV2> = {}): IeltsAuthoringIRV2 {
  return {
    schemaVersion: "IeltsAuthoringIRV2",
    jobId: "item-1",
    modality: "listening",
    exam: { examId: "e", title: "T" },
    taskGroups: [],
    answerSlots: {},
    answerKey: {},
    assets: [],
    ...overrides
  } as unknown as IeltsAuthoringIRV2;
}

const binding = (partOrdinal: number, playable = true) => ({
  itemId: "item-1",
  partOrdinal,
  managedPath: `C:/app/audio/item-1/${partOrdinal}.mp3`,
  sha256: "a",
  sizeBytes: 1,
  mime: "audio/mpeg",
  durationMs: 1000,
  probe: {},
  originalName: `Part ${partOrdinal}.mp3`,
  createdAt: "",
  playable,
  issueCodes: playable ? [] : ["AUDIO_DECODE_FAILED"]
});

describe("listening workspace", () => {
  it("recognises listening drafts only", () => {
    expect(isListening(draft())).toBe(true);
    expect(isListening(draft({ modality: "reading" }))).toBe(false);
  });

  it("an empty listening draft reports the structure as not recognised", () => {
    expect(listeningStructureMissing(draft())).toBe(true);
    expect(listeningStructureMissing(draft({ taskGroups: [{ taskId: "g1" }] as never }))).toBe(false);
  });

  it("offers four parts by default and attaches audio by ordinal", () => {
    const parts = listeningParts(draft(), [binding(2), binding(1, false)]);
    expect(parts.map((part) => [part.ordinal, part.label, part.audio?.originalName ?? null])).toEqual([
      [1, "Part 1", "Part 1.mp3"],
      [2, "Part 2", "Part 2.mp3"],
      [3, "Part 3", null],
      [4, "Part 4", null]
    ]);
    expect(parts[0].audio?.playable).toBe(false);
  });

  it("extends beyond four parts when more audio is bound, and uses IR parts when present", () => {
    expect(listeningParts(draft(), [binding(5)]).length).toBe(5);
    const withIr = draft({
      listening: {
        parts: [
          { partId: "p1", displayLabel: "Section 1", expectedQuestionNumbers: [], taskIds: ["g1"], sourceAnchors: [] },
          { partId: "p2", displayLabel: "Section 2", expectedQuestionNumbers: [], taskIds: ["g2"], sourceAnchors: [] }
        ]
      } as never
    });
    const parts = listeningParts(withIr, []);
    expect(parts.slice(0, 2).map((part) => [part.label, part.taskIds])).toEqual([
      ["Section 1", ["g1"]],
      ["Section 2", ["g2"]]
    ]);
  });

  it("filters task groups by the selected part only when the part maps groups", () => {
    const groups = ["g1", "g2", "g3"];
    const parts = listeningParts(
      draft({ listening: { parts: [{ partId: "p1", displayLabel: "Part 1", expectedQuestionNumbers: [], taskIds: ["g2"], sourceAnchors: [] }] } as never }),
      []
    );
    expect(visibleTaskIds(groups, parts, 1)).toEqual(["g2"]);
    expect(visibleTaskIds(groups, parts, 2)).toEqual(groups);
    expect(visibleTaskIds(groups, parts, undefined)).toEqual(groups);
  });
});
