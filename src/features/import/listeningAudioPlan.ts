// Pure logic for the listening import dialog: which files are audio, how they map to
// parts, and what the dialog hands back to the importer. Kept free of React/Tauri so the
// ordering and assignment rules are unit-testable.

export type ImportModalityChoice = "reading" | "listening";

export interface AudioProbeSummary {
  status: "passed" | "blocked";
  durationMs?: number;
  issueCodes: string[];
}

export interface AudioEntry {
  path: string;
  name: string;
  sizeBytes?: number;
  probe?: AudioProbeSummary;
}

export interface AudioAssignment {
  partOrdinal: number;
  path: string;
  name: string;
}

export interface ListeningImportDecision {
  modality: ImportModalityChoice;
  /** Empty means "add audio later": the item is created but is not audio-ready. */
  audio: AudioAssignment[];
}

export const AUDIO_EXTENSIONS = ["mp3", "m4a", "wav"] as const;
export const EXPECTED_LISTENING_PARTS = 4;
export const MAX_LISTENING_PARTS = 8;

export function fileNameOf(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

export function isAudioPath(path: string): boolean {
  const match = /\.([a-z0-9]+)$/i.exec(path);
  return Boolean(match && (AUDIO_EXTENSIONS as readonly string[]).includes(match[1].toLowerCase()));
}

/** "Part 2" < "Part 10"; case-insensitive; ties broken by the raw string. Mirrors the backend. */
export function naturalCompare(left: string, right: string): number {
  const tokens = (value: string) => value.toLowerCase().match(/\d+|\D+/g) ?? [];
  const a = tokens(left);
  const b = tokens(right);
  for (let index = 0; index < Math.min(a.length, b.length); index += 1) {
    const x = a[index];
    const y = b[index];
    const xNum = /^\d/.test(x);
    const yNum = /^\d/.test(y);
    if (xNum && yNum) {
      const diff = Number(x) - Number(y);
      if (diff !== 0) return diff;
    } else if (xNum !== yNum) {
      return xNum ? -1 : 1;
    } else if (x !== y) {
      return x < y ? -1 : 1;
    }
  }
  if (a.length !== b.length) return a.length - b.length;
  return left < right ? -1 : left > right ? 1 : 0;
}

/**
 * Adds dropped/picked files. Non-audio paths are ignored (reported back), duplicates keep
 * their current position, and each new batch is appended in natural name order so that
 * dropping "Part 1..4" in one go lands on parts 1..4.
 */
export function addAudioEntries(
  current: readonly AudioEntry[],
  incoming: ReadonlyArray<string | AudioEntry>
): { entries: AudioEntry[]; ignored: string[] } {
  const known = new Set(current.map((entry) => entry.path));
  const ignored: string[] = [];
  const fresh: AudioEntry[] = [];
  for (const item of incoming) {
    const entry: AudioEntry = typeof item === "string" ? { path: item, name: fileNameOf(item) } : item;
    if (!isAudioPath(entry.name || entry.path)) {
      ignored.push(entry.name || entry.path);
      continue;
    }
    if (known.has(entry.path)) continue;
    known.add(entry.path);
    fresh.push(entry);
  }
  fresh.sort((left, right) => naturalCompare(left.name, right.name));
  return { entries: [...current, ...fresh].slice(0, MAX_LISTENING_PARTS), ignored };
}

export function moveEntry(entries: readonly AudioEntry[], index: number, delta: -1 | 1): AudioEntry[] {
  const target = index + delta;
  if (index < 0 || index >= entries.length || target < 0 || target >= entries.length) return [...entries];
  const next = [...entries];
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

export function removeEntry(entries: readonly AudioEntry[], index: number): AudioEntry[] {
  return entries.filter((_, position) => position !== index);
}

export function withProbe(entries: readonly AudioEntry[], path: string, probe: AudioProbeSummary): AudioEntry[] {
  return entries.map((entry) => (entry.path === path ? { ...entry, probe } : entry));
}

/** Position in the list is the part: entry 0 plays for Part 1. */
export function assignParts(entries: readonly AudioEntry[]): AudioAssignment[] {
  return entries.map((entry, index) => ({ partOrdinal: index + 1, path: entry.path, name: entry.name }));
}

export interface AssignmentNotice {
  kind: "count" | "blocked";
  message: string;
}

/** Advisory only: nothing here blocks the import; blocked audio is stored and shown later. */
export function assignmentNotices(entries: readonly AudioEntry[]): AssignmentNotice[] {
  const notices: AssignmentNotice[] = [];
  if (entries.length && entries.length !== EXPECTED_LISTENING_PARTS) {
    notices.push({
      kind: "count",
      message: `当前 ${entries.length} 个音频，雅思听力通常为 ${EXPECTED_LISTENING_PARTS} 个 Part。`
    });
  }
  const blocked = entries
    .map((entry, index) => ({ entry, part: index + 1 }))
    .filter(({ entry }) => entry.probe?.status === "blocked");
  if (blocked.length) {
    notices.push({
      kind: "blocked",
      message: `Part ${blocked.map(({ part }) => part).join("、")} 的音频无法通过检查，导入后需要替换。`
    });
  }
  return notices;
}

export function listeningDecision(entries: readonly AudioEntry[]): ListeningImportDecision {
  return { modality: "listening", audio: assignParts(entries) };
}

export function readingDecision(): ListeningImportDecision {
  return { modality: "reading", audio: [] };
}

export function addAudioLaterDecision(): ListeningImportDecision {
  return { modality: "listening", audio: [] };
}

export function formatDuration(ms?: number): string {
  if (ms === undefined || !Number.isFinite(ms)) return "";
  const total = Math.round(ms / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

const ISSUE_LABEL: Record<string, string> = {
  AUDIO_DECODE_FAILED: "无法解码",
  AUDIO_CODEC_UNSUPPORTED: "格式不支持",
  AUDIO_HASH_MISMATCH: "文件已变化",
  AUDIO_SEVERE_CLIPPING: "严重削波",
  AUDIO_NEAR_SILENT: "几乎无声",
  AUDIO_PROBE_UNREADABLE: "检查结果不可读"
};

export function describeIssues(codes: readonly string[]): string {
  return codes.map((code) => ISSUE_LABEL[code] ?? code).join("、");
}

export function probeSummaryFrom(raw: unknown): AudioProbeSummary | undefined {
  if (typeof raw !== "object" || raw === null) return undefined;
  const record = raw as { durationMs?: unknown; probe?: { status?: unknown; issueCodes?: unknown } };
  const status = record.probe?.status;
  if (status !== "passed" && status !== "blocked") return undefined;
  const issueCodes = Array.isArray(record.probe?.issueCodes)
    ? record.probe!.issueCodes.filter((code): code is string => typeof code === "string")
    : [];
  return {
    status,
    durationMs: typeof record.durationMs === "number" ? record.durationMs : undefined,
    issueCodes
  };
}
