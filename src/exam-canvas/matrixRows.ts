import type { OptionV2, ResponseGroupV2 } from "../types";
import { matchingRowsFor, type MatchingRow } from "./renderers/MatchingMatrix";

// 「这道大题是否以矩阵渲染」的唯一判定。画布（ExamCanvas）与结构编译（structureActions）
// 都必须走这里：两处各写一份判断迟早漂移，会出现「画面是矩阵、添加却退回逐题」的错位。
export interface MatrixRuntimeLike {
  questionDisplayMap: Record<string, string>;
  answerSlots: Record<string, { interaction?: string }>;
}

export interface MatrixTaskLike {
  taskType: string;
  responseGroups: readonly ResponseGroupV2[];
  optionBank?: { options: OptionV2[] } | null;
}

export function matrixRowsForTask(
  runtime: MatrixRuntimeLike,
  task: MatrixTaskLike,
  paragraphMap?: Record<string, string>
): MatchingRow[] {
  return matchingRowsFor(
    task.responseGroups,
    runtime.questionDisplayMap,
    (slotId) => runtime.answerSlots[slotId]?.interaction,
    (response) => response.options?.length ? response.options : task.optionBank?.options ?? [],
    task.taskType,
    paragraphMap
  );
}
