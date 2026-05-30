import { devFallbackInvoke } from "../services/devFallbackBackend";

const isTauriRuntime = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export interface PickedPath {
  path: string;
  name: string;
  sizeBytes: number;
}

function nameFromPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

export async function chooseSourceFile(): Promise<PickedPath | null> {
  if (isTauriRuntime()) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "IELTS source documents", extensions: ["pdf", "docx", "txt", "md"] }]
    });
    if (!selected || Array.isArray(selected)) return null;
    return { path: selected, name: nameFromPath(selected), sizeBytes: 0 };
  }

  const fallback = window.prompt("非 Tauri 开发预览：输入本地文件路径或文件名", "demo-reading.pdf");
  if (!fallback) return null;
  return { path: fallback, name: nameFromPath(fallback), sizeBytes: 0 };
}

export async function chooseExportDirectory(): Promise<string | null> {
  if (isTauriRuntime()) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({ multiple: false, directory: true });
    if (!selected || Array.isArray(selected)) return null;
    return selected;
  }

  const fallback = await devFallbackInvoke<string | null>("choose_export_dir");
  return fallback ?? "local://exports";
}
