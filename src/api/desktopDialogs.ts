import { devFallbackInvoke } from "../services/devFallbackBackend";

const isTauriRuntime = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const DEV_PICKED_PATHS_KEY = "ielts-author-studio.dev-fallback-picked-paths.v1";

export interface PickedPath {
  path: string;
  name: string;
  sizeBytes: number;
  titleHint?: string;
  textContent?: string;
  requiresDesktopParser?: boolean;
}

function nameFromPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

function takeDevPickedPath(): PickedPath | null {
  const fromQuery = new URLSearchParams(window.location.search).get("epic8DevPickedPath");
  if (fromQuery) {
    const name = nameFromPath(fromQuery);
    return { path: fromQuery, name, sizeBytes: 0, titleHint: cleanFileStem(name) };
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
    const name = typeof first === "string" ? nameFromPath(path) : first.name ?? nameFromPath(path);
    return {
      path,
      name,
      sizeBytes: typeof first === "string" ? 0 : first.sizeBytes ?? 0,
      titleHint: typeof first === "string" ? cleanFileStem(name) : first.titleHint ?? cleanFileStem(name)
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
    const name = nameFromPath(selected);
    return { path: selected, name, sizeBytes: 0, titleHint: cleanFileStem(name) };
  }

  const preset = takeDevPickedPath();
  if (preset) return preset;

  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".pdf,.docx,.txt,.md,application/pdf,text/plain,text/markdown,application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    input.style.position = "fixed";
    input.style.left = "-9999px";
    input.addEventListener("change", async () => {
      const file = input.files?.[0];
      input.remove();
      if (!file) {
        resolve(null);
        return;
      }
      resolve({
        path: file.name,
        name: file.name,
        sizeBytes: file.size,
        ...(await browserFileMetadata(file))
      });
    }, { once: true });
    document.body.appendChild(input);
    input.click();
  });
}

function cleanFileStem(name: string): string {
  return name
    .replace(/\.[^.]+$/, "")
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

async function browserFileMetadata(file: File): Promise<Pick<PickedPath, "titleHint" | "textContent" | "requiresDesktopParser">> {
  if (/\.(txt|md)$/i.test(file.name) || /^text\//.test(file.type)) {
    const text = await file.text();
    const candidate = text
      .split(/\r?\n/)
      .map((line) => line.replace(/^#+\s*/, "").trim())
      .find((line) => line.length >= 4
        && line.length <= 120
        && !/^questions?\s+\d/i.test(line)
        && !/^answers?\b/i.test(line)
        && !/^reading passage\s+\d/i.test(line)
        && !/^you should spend\b/i.test(line));
    return { titleHint: candidate || cleanFileStem(file.name) || undefined, textContent: text };
  }

  const fromName = cleanFileStem(file.name);
  return { titleHint: fromName || undefined, requiresDesktopParser: true };
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
