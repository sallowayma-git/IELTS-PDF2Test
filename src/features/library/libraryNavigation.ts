import type { LibraryRowV1 } from "./libraryTypes";
import { isReadingPart, type ReadingPart } from "./paperSelection";

export type LibrarySection = "reading" | "listening" | "writing";
export type ReadingPartFilter = "all" | ReadingPart | "unassigned";

export const LIBRARY_SECTIONS: readonly { key: LibrarySection; label: string }[] = [
  { key: "reading", label: "阅读" },
  { key: "listening", label: "听力" },
  { key: "writing", label: "写作" }
];

export const READING_FILTERS: readonly { key: ReadingPartFilter; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "P1", label: "P1" },
  { key: "P2", label: "P2" },
  { key: "P3", label: "P3" },
  { key: "unassigned", label: "未设置" }
];

export function resolveLibrarySection(value?: string): LibrarySection {
  return value === "listening" || value === "writing" ? value : "reading";
}

export function resolveReadingFilter(value?: string): ReadingPartFilter {
  return isReadingPart(value) || value === "unassigned" ? value : "all";
}

export function librarySectionPath(section: LibrarySection, part: ReadingPartFilter = "all"): string {
  const query = new URLSearchParams({ modality: section });
  if (section === "reading" && part !== "all") query.set("part", part);
  return `/library?${query}`;
}

export function matchesReadingFilter(row: LibraryRowV1, filter: ReadingPartFilter): boolean {
  return filter === "all" || (filter === "unassigned" ? !isReadingPart(row.part) : row.part === filter);
}
