import type { ListeningAudioAsset } from "../api/listeningAudioClient";
import type { IeltsAuthoringIRV2 } from "../types";

// Listening workspace projection: parts, their managed audio, and which task groups show.
// Audio bindings live in the backend table (keyed by item + part ordinal) until the IR
// carries per-part media; IR parts, when present, provide labels and task mapping.

export const DEFAULT_LISTENING_PARTS = 4;

export interface ListeningPartView {
  ordinal: number;
  label: string;
  /** Empty when the IR has no part → task mapping yet (then every group is shown). */
  taskIds: string[];
  audio?: ListeningAudioAsset;
}

export function isListening(authoring: Pick<IeltsAuthoringIRV2, "modality">): boolean {
  return authoring.modality === "listening";
}

export function listeningStructureMissing(authoring: Pick<IeltsAuthoringIRV2, "taskGroups">): boolean {
  return !authoring.taskGroups?.length;
}

export function listeningParts(
  authoring: Pick<IeltsAuthoringIRV2, "listening">,
  bindings: readonly ListeningAudioAsset[]
): ListeningPartView[] {
  const irParts = authoring.listening?.parts ?? [];
  const maxBound = bindings.reduce((max, binding) => Math.max(max, binding.partOrdinal), 0);
  const count = Math.max(DEFAULT_LISTENING_PARTS, irParts.length, maxBound);
  const byOrdinal = new Map(bindings.map((binding) => [binding.partOrdinal, binding]));
  return Array.from({ length: count }, (_, index) => {
    const ordinal = index + 1;
    const irPart = irParts[index];
    return {
      ordinal,
      label: irPart?.displayLabel || `Part ${ordinal}`,
      taskIds: irPart?.taskIds ?? [],
      audio: byOrdinal.get(ordinal)
    };
  });
}

export function visibleTaskIds(allTaskIds: readonly string[], parts: readonly ListeningPartView[], selected?: number): string[] {
  const part = parts.find((candidate) => candidate.ordinal === selected);
  if (!part || !part.taskIds.length) return [...allTaskIds];
  const wanted = new Set(part.taskIds);
  return allTaskIds.filter((taskId) => wanted.has(taskId));
}
