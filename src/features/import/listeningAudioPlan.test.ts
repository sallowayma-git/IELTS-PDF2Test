import { describe, expect, it } from "vitest";
import {
  addAudioEntries,
  addAudioLaterDecision,
  assignmentNotices,
  assignParts,
  bindListeningAssignments,
  listeningCandidates,
  listeningDecision,
  moveEntry,
  naturalCompare,
  probeSummaryFrom,
  readingDecision,
  removeEntry,
  splitImportPlan,
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

describe("import plan", () => {
  const paper = (name: string) => ({ path: `/p/${name}`, name, sizeBytes: 1 });

  it("asks only about files detected as listening, in selection order", () => {
    const files = [paper("r.pdf"), paper("l1.pdf"), paper("u.pdf"), paper("l2.pdf")];
    const queue = listeningCandidates(files, [
      { path: "/p/r.pdf", modality: "reading", cues: [] },
      { path: "/p/l1.pdf", modality: "listening", cues: ["listening:four_parts"] },
      { path: "/p/u.pdf", modality: "unknown", cues: [] },
      { path: "/p/l2.pdf", modality: "listening", cues: [] }
    ]);
    expect(queue.map((item) => [item.file.name, item.cues])).toEqual([
      ["l1.pdf", ["listening:four_parts"]],
      ["l2.pdf", []]
    ]);
  });

  it("splits reading files into one batch and each listening paper into its own import with audio", () => {
    const files = [paper("r.pdf"), paper("l.pdf"), paper("switched.pdf")];
    const split = splitImportPlan(files, {
      "/p/l.pdf": { modality: "listening", audio: [{ partOrdinal: 1, path: "/a/1.mp3", name: "1.mp3" }] },
      "/p/switched.pdf": readingDecision()
    });
    expect(split.reading.map((file) => file.name)).toEqual(["r.pdf", "switched.pdf"]);
    expect(split.listening).toEqual([
      { file: files[1], audio: [{ partOrdinal: 1, path: "/a/1.mp3", name: "1.mp3" }] }
    ]);
  });
});

// 绑定失败的如实上报：任何一段失败都不得吞掉、不得显示成成功（评审指定）。
describe("listening audio bind failures are reported", () => {
  const assignments = [
    { partOrdinal: 1, path: "/a/1.mp3", name: "1.mp3" },
    { partOrdinal: 2, path: "/a/2.mp3", name: "2.mp3" },
    { partOrdinal: 3, path: "/a/3.mp3", name: "3.mp3" },
    { partOrdinal: 4, path: "/a/4.mp3", name: "4.mp3" }
  ];

  it("returns no rejections and binds every part when all binds succeed", async () => {
    const bound: Array<[string, number, string]> = [];
    const rejected = await bindListeningAssignments("item-1", "雅思听力卷", assignments, async (itemId, partOrdinal, path) => {
      bound.push([itemId, partOrdinal, path]);
    });
    expect(rejected).toEqual([]);
    expect(bound).toEqual([
      ["item-1", 1, "/a/1.mp3"],
      ["item-1", 2, "/a/2.mp3"],
      ["item-1", 3, "/a/3.mp3"],
      ["item-1", 4, "/a/4.mp3"]
    ]);
  });

  it("reports a failed part as a rejection with the item title and keeps binding the rest", async () => {
    const bound: number[] = [];
    const rejected = await bindListeningAssignments("item-1", "雅思听力卷", assignments, async (_itemId, partOrdinal) => {
      bound.push(partOrdinal);
      if (partOrdinal === 2) throw new Error("LISTENING_AUDIO_MEDIA_NOT_MIRRORED:part-2 镜像没落盘，界面不能显示成功");
    });
    // 一段失败不得中断后续段。
    expect(bound).toEqual([1, 2, 3, 4]);
    expect(rejected).toEqual([
      {
        name: "2.mp3",
        reason: "音频未能添加到「雅思听力卷」，可在工作区「添加音频」补充。LISTENING_AUDIO_MEDIA_NOT_MIRRORED:part-2 镜像没落盘，界面不能显示成功"
      }
    ]);
  });

  it("turns every failed part into its own rejection instead of one silent success", async () => {
    const rejected = await bindListeningAssignments("item-1", "雅思听力卷", assignments, async () => {
      throw new Error("listening_audio_bind:database is locked");
    });
    expect(rejected.map((entry) => entry.name)).toEqual(["1.mp3", "2.mp3", "3.mp3", "4.mp3"]);
    for (const entry of rejected) {
      expect(entry.reason).toContain("音频未能添加到「雅思听力卷」");
    }
  });
});
