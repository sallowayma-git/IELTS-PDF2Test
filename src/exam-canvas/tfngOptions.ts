import type { OptionV2 } from "../types";

/**
 * 判定一组选项是否属于 TRUE/FALSE/NOT GIVEN（或 YES/NO/NOT GIVEN）这类短标签题。
 *
 * 基准页对这类题有专门版式（.tfng-options：横向 flex、窄间距），题面在选项
 * 全部命中短标签时给容器追加 .v2-tfng-options；比较只看 option.label，
 * trim + 大小写不敏感，避免因为大小写差异丢掉版式。
 */

const TFNG_LABELS = new Set(["TRUE", "FALSE", "YES", "NO", "NOT GIVEN"]);

export function isTfngLabel(label: string): boolean {
  return TFNG_LABELS.has(label.trim().toUpperCase());
}

/** 选项组非空且每一项都是判断题短标签时才命中；普通 A/B/C 选项库不受影响。 */
export function isTfngOptionSet(options: readonly OptionV2[]): boolean {
  return options.length > 0 && options.every((option) => isTfngLabel(option.label));
}
