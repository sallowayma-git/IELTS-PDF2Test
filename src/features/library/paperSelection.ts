import type { PaperSpec } from "../../api/publishClient";

// 组卷选择的纯逻辑（无 React、无 IPC），便于单测。
// 规则：阅读每个 Part（P1/P2/P3）最多一篇，同一 Part 再勾另一篇就替换；听力可多选。

export const READING_PARTS = ["P1", "P2", "P3"] as const;
export type ReadingPart = (typeof READING_PARTS)[number];

export interface PaperSelection {
  reading: Partial<Record<ReadingPart, string>>;
  listening: Array<{ itemId: string; part: string }>;
}

export const EMPTY_PAPER_SELECTION: PaperSelection = { reading: {}, listening: [] };

export function isReadingPart(value: string | undefined): value is ReadingPart {
  return value === "P1" || value === "P2" || value === "P3";
}

/** 阅读：该 Part 当前选中的不是这篇 → 替换为这篇；就是这篇 → 取消。 */
export function toggleReadingItem(selection: PaperSelection, part: ReadingPart, itemId: string): PaperSelection {
  const reading = { ...selection.reading };
  if (reading[part] === itemId) delete reading[part];
  else reading[part] = itemId;
  return { ...selection, reading };
}

/** 听力：可多选，再点取消。 */
export function toggleListeningItem(selection: PaperSelection, itemId: string, part: string): PaperSelection {
  const exists = selection.listening.some((entry) => entry.itemId === itemId);
  return {
    ...selection,
    listening: exists
      ? selection.listening.filter((entry) => entry.itemId !== itemId)
      : [...selection.listening, { itemId, part }]
  };
}

/** 三篇阅读齐了才能组卷。 */
export function isPaperReady(selection: PaperSelection): boolean {
  return READING_PARTS.every((part) => Boolean(selection.reading[part]));
}

export function missingReadingParts(selection: PaperSelection): ReadingPart[] {
  return READING_PARTS.filter((part) => !selection.reading[part]);
}

/** 组卷要发布的全部题目 id（阅读三篇 + 已选听力）。 */
export function selectedPaperItemIds(selection: PaperSelection): string[] {
  const reading = READING_PARTS.map((part) => selection.reading[part]).filter((id): id is string => Boolean(id));
  return [...reading, ...selection.listening.map((entry) => entry.itemId)];
}

/** 三篇齐了才返回组卷请求体；否则 undefined。 */
export function buildPaperSpec(selection: PaperSelection): PaperSpec | undefined {
  const [p1, p2, p3] = READING_PARTS.map((part) => selection.reading[part]);
  if (!p1 || !p2 || !p3) return undefined;
  return {
    reading: { P1: p1, P2: p2, P3: p3 },
    listening: selection.listening.map((entry) => ({ part: entry.part, itemId: entry.itemId }))
  };
}

export interface PaperCandidate {
  id: string;
  modality: string;
  part?: string;
}

/**
 * 题库刷新后，把选择对齐到当前行：行已删除、题型变了、或 Part 被改到别的槽位，
 * 都从选择里去掉（听力则更新为当前 Part）。避免组卷请求带上已经不符合条件的题。
 */
export function reconcilePaperSelection(selection: PaperSelection, candidates: PaperCandidate[]): PaperSelection {
  const byId = new Map(candidates.map((candidate) => [candidate.id, candidate]));
  const reading: PaperSelection["reading"] = {};
  for (const part of READING_PARTS) {
    const id = selection.reading[part];
    const candidate = id ? byId.get(id) : undefined;
    if (id && candidate && candidate.modality === "reading" && candidate.part === part) reading[part] = id;
  }
  const listening: PaperSelection["listening"] = [];
  for (const entry of selection.listening) {
    const candidate = byId.get(entry.itemId);
    if (candidate && candidate.modality === "listening" && candidate.part) {
      listening.push({ itemId: entry.itemId, part: candidate.part });
    }
  }
  return { reading, listening };
}
