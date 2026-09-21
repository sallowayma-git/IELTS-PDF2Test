import { describe, expect, it } from "vitest";
import {
  addAudioEntries,
  addAudioLaterDecision,
  assignmentNotices,
  assignParts,
  listeningDecision,
  moveEntry,
  naturalCompare,
  probeSummaryFrom,
  readingDecision,
  removeEntry,
  withProbe
} from "./listeningAudioPlan";

// Evidence level: pure unit (dialog ordering/assignment rules only).

describe("listening audio ordering", () => {
  it("sorts a dropped batch naturally so Part 10 follows Part 2", () => {
    const { entries, ignored } = addAudioEntries([], [
      "C:/cd/Part 10.mp3",
      "C:/cd/Part 2.mp3",
      "C:/cd/part 1.MP3",
      "C:/cd/cover.jpg"
    ]);
    expect(entries.map((entry) => entry.name)).toEqual(["part 1.MP3", "Part 2.mp3", "Part 10.mp3"]);
    expect(ignored).toEqual(["cover.jpg"]);
  });

  it("appends later batches after existing entries and ignores duplicates", () => {
    const first = addAudioEntries([], ["/a/S1.mp3", "/a/S2.mp3"]).entries;
    const second = addAudioEntries(first, ["/a/S2.mp3", "/a/S4.mp3", "/a/S3.mp3"]).entries;
    expect(second.map((entry) => entry.name)).toEqual(["S1.mp3", "S2.mp3", "S3.mp3", "S4.mp3"]);
  });

  it("compares numbers numerically", () => {
    expect(naturalCompare("Section 9", "Section 10")).toBeLessThan(0);
    expect(naturalCompare("b", "a")).toBeGreaterThan(0);
  });
});

describe("part assignment", () => {
  const four = addAudioEntries([], ["/x/1.mp3", "/x/2.mp3", "/x/3.mp3", "/x/4.mp3"]).entries;

  it("maps list position to part ordinal", () => {
    expect(assignParts(four).map((assignment) => [assignment.partOrdinal, assignment.name])).toEqual([
      [1, "1.mp3"],
      [2, "2.mp3"],
      [3, "3.mp3"],
      [4, "4.mp3"]
    ]);
  });

  it("reorders by moving an entry up or down", () => {
    const moved = moveEntry(four, 3, -1);
    expect(assignParts(moved).map((assignment) => assignment.name)).toEqual(["1.mp3", "2.mp3", "4.mp3", "3.mp3"]);
    expect(moveEntry(four, 0, -1)).toEqual(four);
    expect(moveEntry(four, 3, 1)).toEqual(four);
  });

  it("removing an entry renumbers the following parts", () => {
    expect(assignParts(removeEntry(four, 1)).map((assignment) => [assignment.partOrdinal, assignment.name])).toEqual([
      [1, "1.mp3"],
      [2, "3.mp3"],
      [3, "4.mp3"]
    ]);
  });

  it("warns (without blocking) about an unusual count and blocked probes", () => {
    expect(assignmentNotices(four)).toEqual([]);
    const blocked = withProbe(four.slice(0, 3), "/x/2.mp3", { status: "blocked", issueCodes: ["AUDIO_DECODE_FAILED"] });
    const kinds = assignmentNotices(blocked).map((notice) => notice.kind);
    expect(kinds).toEqual(["count", "blocked"]);
    expect(assignmentNotices(blocked)[1].message).toContain("Part 2");
  });
});

describe("dialog decisions", () => {
  it("confirming listening carries the ordered audio", () => {
    const entries = addAudioEntries([], ["/y/b.mp3", "/y/a.mp3"]).entries;
    expect(listeningDecision(entries)).toEqual({
      modality: "listening",
      audio: [
        { partOrdinal: 1, path: "/y/a.mp3", name: "a.mp3" },
        { partOrdinal: 2, path: "/y/b.mp3", name: "b.mp3" }
      ]
    });
  });

  it("switching to reading drops any audio", () => {
    expect(readingDecision()).toEqual({ modality: "reading", audio: [] });
  });

  it("add audio later keeps listening with no audio", () => {
    expect(addAudioLaterDecision()).toEqual({ modality: "listening", audio: [] });
  });

  it("reads probe summaries from the backend result shape", () => {
    expect(probeSummaryFrom({ durationMs: 1000, probe: { status: "passed", issueCodes: [] } })).toEqual({
      status: "passed",
      durationMs: 1000,
      issueCodes: []
    });
    expect(probeSummaryFrom({ probe: { status: "weird" } })).toBeUndefined();
  });
});
