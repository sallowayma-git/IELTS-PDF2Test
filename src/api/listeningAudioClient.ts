import { command } from "./tauriCommands";

export interface ModalityDetection {
  path: string;
  modality: "listening" | "reading" | "unknown";
  cues: string[];
}

export interface ListeningAudioAsset {
  itemId: string;
  partOrdinal: number;
  managedPath: string;
  sha256: string;
  sizeBytes: number;
  mime: string | null;
  durationMs: number | null;
  probe: unknown;
  originalName: string;
  createdAt: string;
  playable: boolean;
  issueCodes: string[];
}

export interface ListeningAudioStatus {
  itemId: string;
  bindings: ListeningAudioAsset[];
  audioReady: boolean;
  blockers: string[];
}

export interface FolderAudioFile {
  path: string;
  name: string;
  sizeBytes: number;
}

const isTauriRuntime = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/**
 * Listening hint per source file. Outside the desktop runtime there is no file path to
 * read, so every file is reported `unknown` and the import stays on the reading path.
 */
export async function detectImportModality(paths: string[]): Promise<ModalityDetection[]> {
  if (!isTauriRuntime()) return paths.map((path) => ({ path, modality: "unknown", cues: [] }));
  return command<ModalityDetection[]>("detect_import_modality", { paths });
}

export function probeListeningAudioFiles(paths: string[]) {
  return command<unknown[]>("probe_listening_audio_files", { paths });
}

export function listListeningAudioFolder(folder: string) {
  return command<FolderAudioFile[]>("list_listening_audio_folder", { folder });
}

export function bindListeningAudio(itemId: string, partOrdinal: number, path: string) {
  return command<ListeningAudioAsset>("bind_listening_audio", { input: { itemId, partOrdinal, path } });
}

export function unbindListeningAudio(itemId: string, partOrdinal: number) {
  return command<boolean>("unbind_listening_audio", { itemId, partOrdinal });
}

export function getListeningAudio(itemId: string, verify = false) {
  return command<ListeningAudioStatus>("get_listening_audio", { itemId, verify });
}

/** Webview URL for a managed file (asset protocol; CSP media-src allows it). */
export async function managedAudioUrl(managedPath: string): Promise<string | undefined> {
  if (!isTauriRuntime()) return undefined;
  const { convertFileSrc } = await import("@tauri-apps/api/core");
  return convertFileSrc(managedPath);
}
