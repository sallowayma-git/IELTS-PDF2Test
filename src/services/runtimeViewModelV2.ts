import type { IeltsAuthoringIRV2 } from "../types/ielts-authoring-v2";
import type {
  ReadingInteractionModelV2,
  RuntimeTaskGroupV2,
  RuntimeViewModelV2
} from "../types/runtime-view-model-v2";

function runtimeTaskGroup(task: IeltsAuthoringIRV2["taskGroups"][number]): RuntimeTaskGroupV2 {
  return {
    taskId: task.taskId,
    displayRange: task.displayRange,
    taskType: task.taskType,
    instructions: task.instructions,
    stimulus: task.stimulus,
    optionBank: task.optionBank,
    responseGroups: task.responseGroups
  };
}
/** Build the single runtime contract used by the Phase 5 student preview. */
export function buildRuntimeViewModelV2(authoring: IeltsAuthoringIRV2): RuntimeViewModelV2 {
  const questionOrder = Object.values(authoring.answerSlots)
    .sort((left, right) => left.questionNumber - right.questionNumber || left.slotId.localeCompare(right.slotId))
    .map((slot) => slot.slotId);
  const questionDisplayMap = Object.values(authoring.answerSlots).reduce<Record<string, string>>((result, slot) => {
    result[slot.slotId] = slot.displayLabel;
    return result;
  }, {});
  return {
    schemaVersion: "RuntimeViewModelV2",
    examId: authoring.exam.examId,
    title: authoring.exam.title,
    passage: authoring.passage?.content ?? [],
    taskGroups: authoring.taskGroups.map(runtimeTaskGroup),
    answerSlots: authoring.answerSlots,
    answerKey: authoring.answerKey,
    questionOrder,
    questionDisplayMap,
    assets: authoring.assets
  };
}

function responseOptions(task: RuntimeTaskGroupV2, response: RuntimeTaskGroupV2["responseGroups"][number]) {
  if (response.options?.length) return response.options;
  if (response.optionBankRef && task.optionBank?.optionBankId === response.optionBankRef) return task.optionBank.options;
  return [];
}

/** Build the editor's framework-neutral interaction projection from the same runtime view model. */
export function buildReadingInteractionModelV2(runtime: RuntimeViewModelV2): ReadingInteractionModelV2 {
  const responseGroups: ReadingInteractionModelV2["responseGroups"] = {};
  const slots: ReadingInteractionModelV2["slots"] = {};
  const taskGroups: ReadingInteractionModelV2["taskGroups"] = [];
  for (const task of runtime.taskGroups) {
    const responseGroupIds: string[] = [];
    for (const response of task.responseGroups) {
      const options = responseOptions(task, response);
      responseGroups[response.responseGroupId] = {
        taskId: task.taskId,
        responseGroupId: response.responseGroupId,
        kind: response.kind,
        slotIds: [...response.slotIds],
        options,
        cardinality: response.cardinality,
        assignment: response.assignment,
        scoringPolicy: response.scoringPolicy,
        duplicatePolicy: response.duplicatePolicy,
        allowOptionReuse: response.allowOptionReuse
      };
      responseGroupIds.push(response.responseGroupId);
      for (const slotId of response.slotIds) {
        const slot = runtime.answerSlots[slotId];
        if (!slot) continue;
        slots[slotId] = {
          taskId: task.taskId,
          responseGroupId: response.responseGroupId,
          slot,
          options
        };
      }
    }
    taskGroups.push({ taskId: task.taskId, taskType: task.taskType, responseGroupIds });
  }
  return {
    schemaVersion: "ReadingInteractionModelV2",
    examId: runtime.examId,
    taskGroups,
    responseGroups,
    slots
  };
}
