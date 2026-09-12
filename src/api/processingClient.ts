import { command } from "./tauriCommands";
import type { PickedPath } from "./desktopDialogs";

export interface ProcessingState {
  stage: string;
  localStatus: string;
  cloudStatus: string;
  actionableCount: number;
  lastErrorCode?: string;
  eventSeq: number;
}

export function importFiles(input: { files: PickedPath[]; cloudEnabled: boolean; cloudProfileId?: string }) {
  return command<{ created: Array<{ itemId: string; title: string }>; rejected: Array<{ name: string; reason: string }> }>("import_files", { input });
}
export function retryProcessing(itemId: string) { return command<void>("retry_processing", { itemId }); }
export function cancelProcessing(itemId: string) { return command<void>("cancel_processing", { itemId }); }

export async function subscribeProcessing(onUpdate: (itemId: string) => void): Promise<() => void> {
  if (!("__TAURI_INTERNALS__" in window)) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  const versions = new Map<string, number>();
  return listen<{ libraryItemId: string; stateVersion: number }>("processing://item-updated", ({ payload }) => {
    if ((versions.get(payload.libraryItemId) ?? -1) >= payload.stateVersion) return;
    versions.set(payload.libraryItemId, payload.stateVersion);
    onUpdate(payload.libraryItemId);
  });
}
