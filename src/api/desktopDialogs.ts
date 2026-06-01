import { devFallbackInvoke } from "../services/devFallbackBackend";

const isTauriRuntime = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const DEV_PICKED_PATHS_KEY = "ielts-author-studio.dev-fallback-picked-paths.v1";

export interface PickedPath {
  path: string;
  name: string;
  sizeBytes: number;
}

function nameFromPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

function takeDevPickedPath(): PickedPath | null {
  const fromQuery = new URLSearchParams(window.location.search).get("epic8DevPickedPath");
  if (fromQuery) {
    return { path: fromQuery, name: nameFromPath(fromQuery), sizeBytes: 0 };
  }

  const raw = window.localStorage.getItem(DEV_PICKED_PATHS_KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Array<string | Partial<PickedPath>>;
    if (!Array.isArray(parsed) || parsed.length === 0) return null;
    const [first, ...rest] = parsed;
    if (rest.length) {
      window.localStorage.setItem(DEV_PICKED_PATHS_KEY, JSON.stringify(rest));
    } else {
      window.localStorage.removeItem(DEV_PICKED_PATHS_KEY);
    }
    const path = typeof first === "string" ? first : first.path;
    if (!path) return null;
    return {
      path,
      name: typeof first === "string" ? nameFromPath(path) : first.name ?? nameFromPath(path),
      sizeBytes: typeof first === "string" ? 0 : first.sizeBytes ?? 0
    };
  } catch {
    window.localStorage.removeItem(DEV_PICKED_PATHS_KEY);
    return null;
  }
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

  const preset = takeDevPickedPath();
  if (preset) return preset;

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
