import { useState } from "react";
import type { UserTaskV1 } from "./userTasks";

const prefix = "ielts-author-studio.confirmed-hints.v1:";
// Match the content as well as the ID: a fresh cloud result must be reviewed again.
export function hintFingerprint(task: UserTaskV1): string {
  return JSON.stringify([task.taskId, task.kind, task.title, task.detail, task.comparison, task.actions]);
}
function read(itemId: string): string[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(prefix + itemId) ?? "[]");
    return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string") : [];
  } catch { return []; }
}
export function useConfirmedHints(itemId: string) {
  const [state, setState] = useState(() => ({ itemId, entries: read(itemId) }));
  const entries = state.itemId === itemId ? state.entries : read(itemId);
  function save(next: string[]) {
    // Persist before hiding: if storage fails the caller can report it and retain the hint.
    localStorage.setItem(prefix + itemId, JSON.stringify(next));
    setState({ itemId, entries: next });
  }
  return {
    isConfirmed: (task: UserTaskV1) => entries.includes(hintFingerprint(task)),
    confirm: (task: UserTaskV1) => save([...new Set([...entries, hintFingerprint(task)])]),
    reset: () => save([]),
    confirmedCount: entries.length
  };
}
