// 「题库保存」在工作区的两件事：
//   1. 显式「保存」：把待保存的编辑立即刷进题库（自动保存照常工作）；
//   2. 发布后原文件已删除的题目：需要原文件的操作（重新识别 / 云端修复 / 答案页重试）
//      禁用，并说明原因。题目本身仍可编辑、保存、再次发布。

export const SAVED_TO_LIBRARY_NOTICE = "已保存到题库";

export const SOURCE_PURGED_EXPLANATION =
  "原文件已在发布后删除，不能重新识别或重试答案页；题目仍可编辑、保存和发布。";

/** 显式保存：等待全部待保存编辑落盘后才报「已保存到题库」，失败原样上抛给调用方的错误提示。 */
export async function saveToLibrary(flush: () => Promise<void>): Promise<string> {
  await flush();
  return SAVED_TO_LIBRARY_NOTICE;
}

/** 需要原文件的操作是否可用。 */
export function sourceActionsAvailable(sourcePurged: boolean | undefined): boolean {
  return !sourcePurged;
}
