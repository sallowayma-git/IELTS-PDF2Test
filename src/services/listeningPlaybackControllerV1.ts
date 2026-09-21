import type { ListeningRecoveryBehaviorV2 } from "../types/ielts-authoring-v2";
import type {
  ListeningExamSourceV1,
  ListeningPlaybackFailureCodeV1,
  ListeningPlaybackSnapshotV1,
  ListeningPlaybackStatusV1
} from "../types/listening-runtime-v1";

export type ListeningPlaybackEventV1 =
  | { type: "play"; at: string }
  | { type: "pause"; at: string; positionMs: number }
  | { type: "seek"; at: string; positionMs: number }
  | { type: "progress"; at: string; positionMs: number }
  | { type: "ended"; at: string }
  | { type: "refresh_recover"; at: string }
  | { type: "crash_recover"; at: string }
  | { type: "fail"; at: string; failureCode: ListeningPlaybackFailureCodeV1 };

export type ListeningPlaybackControllerErrorCodeV1 =
  | "PLAYBACK_SOURCE_BLOCKED"
  | "PLAYBACK_SNAPSHOT_INVALID"
  | "PLAYBACK_TRANSITION_INVALID"
  | "PLAYBACK_PAUSE_FORBIDDEN"
  | "PLAYBACK_SEEK_FORBIDDEN"
  | "PLAYBACK_REPLAY_FORBIDDEN"
  | "PLAYBACK_LIMIT_REACHED"
  | "PLAYBACK_POSITION_INVALID";

export class ListeningPlaybackControllerErrorV1 extends Error {
  constructor(public readonly code: ListeningPlaybackControllerErrorCodeV1, message: string) {
    super(message);
    this.name = "ListeningPlaybackControllerErrorV1";
  }
}

function fail(code: ListeningPlaybackControllerErrorCodeV1, message: string): never {
  throw new ListeningPlaybackControllerErrorV1(code, message);
}

interface PlaybackMediaV1 {
  assetId: string;
  durationMs: number;
  probe?: { status: string; issueCodes: readonly string[] };
}

// Kept local (no value imports) so the controller stays a self-contained module.
function mediaByAssetId(source: ListeningExamSourceV1, assetId: string): PlaybackMediaV1 | undefined {
  if (source.media?.assetId === assetId) return source.media;
  return source.parts.find((part) => part.media?.assetId === assetId)?.media;
}

/** Section audio of `partId`, else complete-exam audio, else the first section audio. */
function startingMedia(source: ListeningExamSourceV1, partId?: string): PlaybackMediaV1 | undefined {
  const part = partId === undefined ? undefined : source.parts.find((entry) => entry.partId === partId);
  if (partId !== undefined && !part) return undefined;
  return part?.media ?? source.media ?? source.parts.find((entry) => entry.media)?.media;
}

function assertPlayableMedia(source: ListeningExamSourceV1, media: PlaybackMediaV1 | undefined): PlaybackMediaV1 {
  if (
    source.schemaVersion !== "ListeningExamSourceV1"
    || !media
    || media.probe?.status !== "passed"
    || media.probe.issueCodes.length > 0
  ) {
    fail("PLAYBACK_SOURCE_BLOCKED", "Listening playback requires a passed audio probe.");
  }
  return media;
}

function assertIntegerPosition(media: PlaybackMediaV1, positionMs: number): void {
  if (!Number.isSafeInteger(positionMs) || positionMs < 0 || positionMs > media.durationMs) {
    fail("PLAYBACK_POSITION_INVALID", "Playback position must be an integer inside the probed duration.");
  }
}

export function validateListeningPlaybackSnapshotV1(
  source: ListeningExamSourceV1,
  snapshot: ListeningPlaybackSnapshotV1
): string[] {
  const issues: string[] = [];
  const media = mediaByAssetId(source, snapshot.mediaAssetId);
  if (!media) issues.push("media_asset_mismatch");
  const durationMs = media?.durationMs ?? 0;
  if (snapshot.policyMode !== source.playbackPolicy.mode) issues.push("policy_mode_mismatch");
  if (!Number.isSafeInteger(snapshot.playsStarted) || snapshot.playsStarted < 0) issues.push("plays_started_invalid");
  if (!Number.isSafeInteger(snapshot.positionMs) || snapshot.positionMs < 0 || snapshot.positionMs > durationMs) {
    issues.push("position_invalid");
  }
  if (source.playbackPolicy.maxPlays !== undefined && snapshot.playsStarted > source.playbackPolicy.maxPlays) {
    issues.push("play_limit_exceeded");
  }
  if (!source.playbackPolicy.allowReplay && snapshot.playsStarted > 1) issues.push("replay_forbidden");
  if (snapshot.status === "ready" && (snapshot.playsStarted !== 0 || snapshot.positionMs !== 0)) issues.push("ready_state_invalid");
  if (snapshot.status === "restart_pending" && (snapshot.playsStarted === 0 || snapshot.positionMs !== 0)) issues.push("restart_state_invalid");
  if (["playing", "paused"].includes(snapshot.status) && (snapshot.playsStarted === 0 || snapshot.positionMs >= durationMs)) {
    issues.push("active_state_invalid");
  }
  if (snapshot.status === "paused" && !source.playbackPolicy.allowPause) issues.push("pause_forbidden");
  if (snapshot.status === "ended" && (snapshot.playsStarted === 0 || snapshot.positionMs !== durationMs)) {
    issues.push("ended_state_invalid");
  }
  if ((snapshot.status === "failed") !== Boolean(snapshot.failureCode)) issues.push("failure_state_invalid");
  return [...new Set(issues)];
}

function assertSnapshot(source: ListeningExamSourceV1, snapshot: ListeningPlaybackSnapshotV1): void {
  const issues = validateListeningPlaybackSnapshotV1(source, snapshot);
  if (issues.length > 0) fail("PLAYBACK_SNAPSHOT_INVALID", `Invalid playback snapshot: ${issues.join(",")}`);
}

/**
 * Starts a playback session. With per-section audio, `partId` selects which
 * section file the snapshot binds to; otherwise the complete-exam audio is used.
 */
export function createListeningPlaybackSnapshotV1(
  source: ListeningExamSourceV1,
  at: string,
  partId?: string
): ListeningPlaybackSnapshotV1 {
  const media = assertPlayableMedia(source, startingMedia(source, partId));
  return {
    mediaAssetId: media.assetId,
    policyMode: source.playbackPolicy.mode,
    playsStarted: 0,
    positionMs: 0,
    status: "ready",
    lastTransitionAt: at
  };
}

function canStartAnotherPlay(source: ListeningExamSourceV1, snapshot: ListeningPlaybackSnapshotV1): void {
  if (snapshot.playsStarted > 0 && !source.playbackPolicy.allowReplay) {
    fail("PLAYBACK_REPLAY_FORBIDDEN", "The source policy does not allow replay.");
  }
  if (source.playbackPolicy.maxPlays !== undefined && snapshot.playsStarted >= source.playbackPolicy.maxPlays) {
    fail("PLAYBACK_LIMIT_REACHED", "The source playback limit has been reached.");
  }
}

function recoveryFailure(snapshot: ListeningPlaybackSnapshotV1, at: string): ListeningPlaybackSnapshotV1 {
  return { ...snapshot, status: "failed", failureCode: "AUDIO_RECOVERY_BLOCKED", lastTransitionAt: at };
}

function recover(
  source: ListeningExamSourceV1,
  snapshot: ListeningPlaybackSnapshotV1,
  behavior: ListeningRecoveryBehaviorV2,
  at: string
): ListeningPlaybackSnapshotV1 {
  if (snapshot.status === "failed") return { ...snapshot, lastTransitionAt: at };
  if (behavior === "block") return recoveryFailure(snapshot, at);
  if (behavior === "resume_from_snapshot") return { ...snapshot, lastTransitionAt: at };
  if (snapshot.playsStarted === 0) return { ...snapshot, positionMs: 0, status: "ready", lastTransitionAt: at };
  if (!source.playbackPolicy.allowReplay) return recoveryFailure(snapshot, at);
  if (source.playbackPolicy.maxPlays !== undefined && snapshot.playsStarted >= source.playbackPolicy.maxPlays) {
    return recoveryFailure(snapshot, at);
  }
  return { ...snapshot, positionMs: 0, status: "restart_pending", lastTransitionAt: at };
}

export function transitionListeningPlaybackV1(
  source: ListeningExamSourceV1,
  snapshot: ListeningPlaybackSnapshotV1,
  event: ListeningPlaybackEventV1
): ListeningPlaybackSnapshotV1 {
  const media = assertPlayableMedia(source, mediaByAssetId(source, snapshot.mediaAssetId));
  assertSnapshot(source, snapshot);
  let next: ListeningPlaybackSnapshotV1;
  switch (event.type) {
    case "play":
      if (snapshot.status === "paused") {
        next = { ...snapshot, status: "playing", lastTransitionAt: event.at };
        break;
      }
      if (!["ready", "ended", "restart_pending"].includes(snapshot.status)) {
        fail("PLAYBACK_TRANSITION_INVALID", `Cannot play from ${snapshot.status}.`);
      }
      canStartAnotherPlay(source, snapshot);
      next = {
        ...snapshot,
        playsStarted: snapshot.playsStarted + 1,
        positionMs: snapshot.status === "ready" ? snapshot.positionMs : 0,
        status: "playing",
        lastTransitionAt: event.at
      };
      break;
    case "pause":
      if (!source.playbackPolicy.allowPause) fail("PLAYBACK_PAUSE_FORBIDDEN", "Pause is disabled by source policy.");
      if (snapshot.status !== "playing") fail("PLAYBACK_TRANSITION_INVALID", `Cannot pause from ${snapshot.status}.`);
      assertIntegerPosition(media, event.positionMs);
      if (event.positionMs < snapshot.positionMs) fail("PLAYBACK_POSITION_INVALID", "Pause position cannot move backwards.");
      next = {
        ...snapshot,
        positionMs: event.positionMs,
        status: event.positionMs === media.durationMs ? "ended" : "paused",
        lastTransitionAt: event.at
      };
      break;
    case "seek":
      if (!source.playbackPolicy.allowSeek) fail("PLAYBACK_SEEK_FORBIDDEN", "Seek is disabled by source policy.");
      if (!["playing", "paused"].includes(snapshot.status)) fail("PLAYBACK_TRANSITION_INVALID", `Cannot seek from ${snapshot.status}.`);
      assertIntegerPosition(media, event.positionMs);
      next = {
        ...snapshot,
        positionMs: event.positionMs,
        status: event.positionMs === media.durationMs ? "ended" : snapshot.status,
        lastTransitionAt: event.at
      };
      break;
    case "progress":
      if (snapshot.status !== "playing") fail("PLAYBACK_TRANSITION_INVALID", `Cannot advance from ${snapshot.status}.`);
      assertIntegerPosition(media, event.positionMs);
      if (event.positionMs < snapshot.positionMs) fail("PLAYBACK_POSITION_INVALID", "Progress cannot move backwards; use seek.");
      next = {
        ...snapshot,
        positionMs: event.positionMs,
        status: event.positionMs === media.durationMs ? "ended" : "playing",
        lastTransitionAt: event.at
      };
      break;
    case "ended":
      if (snapshot.status !== "playing") fail("PLAYBACK_TRANSITION_INVALID", `Cannot end from ${snapshot.status}.`);
      next = { ...snapshot, positionMs: media.durationMs, status: "ended", lastTransitionAt: event.at };
      break;
    case "refresh_recover":
      next = recover(source, snapshot, source.playbackPolicy.refreshBehavior, event.at);
      break;
    case "crash_recover":
      next = recover(source, snapshot, source.playbackPolicy.crashRecoveryBehavior, event.at);
      break;
    case "fail":
      next = { ...snapshot, status: "failed", failureCode: event.failureCode, lastTransitionAt: event.at };
      break;
  }
  assertSnapshot(source, next);
  return next;
}
