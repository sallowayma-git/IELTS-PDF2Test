import { devFallbackInvoke } from "../services/devFallbackBackend";
import type { DocumentIr } from "../types";

const isTauriRuntime = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const DEV_PICKED_PATHS_KEY = "ielts-author-studio.dev-fallback-picked-paths.v1";

export interface PickedPath {
  path: string;
  name: string;
  sizeBytes: number;
  titleHint?: string;
  textContent?: string;
  binaryContentBase64?: string;
  parsedDocumentIr?: DocumentIr;
  requiresDesktopParser?: boolean;
}

interface TauriPickedPath {
  path: string;
  name: string;
  sizeBytes: number;
  titleHint?: string;
  requiresDesktopParser?: boolean;
}

function nameFromPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

function takeDevPickedPath(): PickedPath | null {
  const fromQuery = new URLSearchParams(window.location.search).get("epic8DevPickedPath");
  if (fromQuery) {
    const name = nameFromPath(fromQuery);
    return { path: fromQuery, name, sizeBytes: 0, titleHint: cleanFileStem(name), requiresDesktopParser: false };
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
    const preset = typeof first === "string" ? undefined : first;
    return {
      path,
      name,
      sizeBytes: preset?.sizeBytes ?? 0,
      titleHint: preset?.titleHint ?? cleanFileStem(name),
      // 预置项声明的是 Partial<PickedPath>，因此必须把真实内容一起带出来。
      // 之前这里丢掉了 textContent/binaryContentBase64，导致浏览器开发预览下
      // 预置文件永远拿不到真实解析内容（只能走桌面解析）。
      textContent: preset?.textContent,
      binaryContentBase64: preset?.binaryContentBase64,
      requiresDesktopParser: preset?.requiresDesktopParser ?? !/\.(txt|md|pdf|docx)$/i.test(name)
    };
  } catch {
    window.localStorage.removeItem(DEV_PICKED_PATHS_KEY);
    return null;
  }
}

export async function chooseSourceFiles(): Promise<PickedPath[]> {
  if (isTauriRuntime()) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      multiple: true,
      directory: false,
      filters: [{ name: "IELTS source documents", extensions: ["pdf", "docx", "txt", "md"] }]
    });
    if (!selected) return [];
    const selectedPaths = Array.isArray(selected) ? selected : [selected];
    return selectedPaths.map((path) => {
      const name = nameFromPath(path);
      return { path, name, sizeBytes: 0, titleHint: cleanFileStem(name) };
    });
  }

  const preset = takeDevPickedPath();
  if (preset) return [preset];

  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    input.accept = ".pdf,.docx,.txt,.md,application/pdf,text/plain,text/markdown,application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    input.style.position = "fixed";
    input.style.left = "-9999px";
    input.addEventListener("change", async () => {
      const files = Array.from(input.files ?? []);
      input.remove();
      if (!files.length) {
        resolve([]);
        return;
      }
      const picked = await Promise.all(files.map(async (file) => ({
        path: file.name,
        name: file.name,
        sizeBytes: file.size,
        ...(await browserFileMetadata(file))
      })));
      resolve(picked);
    }, { once: true });
    document.body.appendChild(input);
    input.click();
  });
}

export async function choosePdfFolderSources(): Promise<PickedPath[]> {
  if (isTauriRuntime()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<TauriPickedPath[]>("pick_pdf_folder_sources");
  }

  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    (input as HTMLInputElement & { webkitdirectory?: boolean }).webkitdirectory = true;
    input.accept = ".pdf,application/pdf";
    input.style.position = "fixed";
    input.style.left = "-9999px";
    input.addEventListener("change", async () => {
      const files = Array.from(input.files ?? []).filter((file) => /\.pdf$/i.test(file.name));
      input.remove();
      if (!files.length) {
        resolve([]);
        return;
      }
      const picked = await Promise.all(files.map(async (file) => ({
        path: file.webkitRelativePath || file.name,
        name: file.name,
        sizeBytes: file.size,
        titleHint: cleanFileStem(file.name),
        binaryContentBase64: await fileToBase64(file),
        requiresDesktopParser: false
      })));
      resolve(picked);
    }, { once: true });
    document.body.appendChild(input);
    input.click();
  });
}

export async function chooseSourceFile(): Promise<PickedPath | null> {
  const files = await chooseSourceFiles();
  return files[0] ?? null;
}

function cleanFileStem(name: string): string {
  return name
    .replace(/\.[^.]+$/, "")
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

async function fileToBase64(file: File): Promise<string> {
  const buffer = await file.arrayBuffer();
  let binary = "";
  const bytes = new Uint8Array(buffer);
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunkSize));
  }
  return window.btoa(binary);
}

async function browserFileMetadata(file: File): Promise<Pick<PickedPath, "titleHint" | "textContent" | "binaryContentBase64" | "requiresDesktopParser">> {
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
  if (/\.(pdf|docx)$/i.test(file.name)) {
    return { titleHint: fromName || undefined, binaryContentBase64: await fileToBase64(file) };
  }
  return { titleHint: fromName || undefined, requiresDesktopParser: true };
}

export async function chooseExportDirectory(): Promise<string | null> {
  if (isTauriRuntime()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<string | null>("choose_export_dir");
  }

  const fallback = await devFallbackInvoke<string | null>("choose_export_dir");
  return fallback ?? "local://exports";
}
