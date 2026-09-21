import { describe, expect, it } from "vitest";
import fourPart from "../../fixtures/golden/synthetic/ielts/phase7-listening-four-part-media-source-v1.json";
import singlePart from "../../fixtures/golden/synthetic/ielts/phase7-listening-part1-source-v1.json";
import type { ListeningExamSourceV1 } from "../types/listening-runtime-v1";
import { validateListeningExamSourceV1 } from "./listeningRuntimeV1";
import {
  createListeningPlaybackSnapshotV1,
  transitionListeningPlaybackV1,
  validateListeningPlaybackSnapshotV1
} from "./listeningPlaybackControllerV1";

function clone(value: unknown): ListeningExamSourceV1 {
  return structuredClone(value) as ListeningExamSourceV1;
}

function codes(source: ListeningExamSourceV1): string[] {
  return validateListeningExamSourceV1(source).map((issue) => `${issue.code}:${issue.targetId}`);
}

describe("listening section media (one audio file per part)", () => {
  it("accepts a four-part source with four distinct section media and no exam media", () => {
    const source = clone(fourPart);
    expect(source.media).toBeUndefined();
    expect(new Set(source.parts.map((part) => part.media?.assetId)).size).toBe(4);
    expect(codes(source)).toEqual([]);
    expect(JSON.parse(JSON.stringify(source))).toEqual(fourPart);
  });

  it("still accepts the single complete-exam media fixture", () => {
    expect(codes(clone(singlePart))).toEqual([]);
  });

  it("rejects a section media hash mismatch", () => {
    const source = clone(fourPart);
    source.parts[2].media!.sha256 = "f".repeat(64);
    expect(codes(source)).toContain("AUDIO_HASH_MISMATCH:audio-section-3");
  });

  it("rejects a section media that references an unknown asset", () => {
    const source = clone(fourPart);
    source.parts[1].media!.assetId = "audio-ghost";
    expect(codes(source)).toContain("ASSET_REFERENCE_MISSING:audio-ghost");
  });

  it("rejects a part with neither section media nor exam media", () => {
    const source = clone(fourPart);
    delete source.parts[3].media;
    expect(codes(source)).toContain("LISTENING_MEDIA_MISSING:part-4");
  });

  it("requires a passed probe on section media", () => {
    const source = clone(fourPart);
    delete source.parts[0].media!.probe;
    expect(codes(source)).toContain("AUDIO_PROBE_BLOCKED:audio-section-1");
  });

  it("bounds a cue by the part's own media", () => {
    const source = clone(fourPart);
    source.parts[0].cue!.endMs = 4000;
    expect(codes(source)).toContain("AUDIO_CUE_INVALID:part-1");
  });

  it("binds a playback snapshot to the selected section media", () => {
    const source = clone(fourPart);
    const snapshot = createListeningPlaybackSnapshotV1(source, "2026-09-21T00:00:00Z", "part-2");
    expect(snapshot.mediaAssetId).toBe("audio-section-2");
    const playing = transitionListeningPlaybackV1(source, snapshot, { type: "play", at: "2026-09-21T00:00:01Z" });
    const ended = transitionListeningPlaybackV1(source, playing, { type: "ended", at: "2026-09-21T00:00:02Z" });
    expect(ended.positionMs).toBe(2500);
    expect(validateListeningPlaybackSnapshotV1(source, { ...ended, mediaAssetId: "audio-ghost" })).toContain("media_asset_mismatch");
  });
});
