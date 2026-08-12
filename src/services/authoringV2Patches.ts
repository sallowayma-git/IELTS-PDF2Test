import type {
  AnswerValueV2,
  AuthoringPatchV2,
  IeltsAuthoringIRV2,
  QuestionNumberExpressionV2,
  SourceAnchorV2,
  TaskTypeV2
} from "../types";

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function findObjectById(value: unknown, id: string): JsonObject | undefined {
  if (Array.isArray(value)) {
    for (const item of value) {
      const result = findObjectById(item, id);
      if (result) return result;
    }
    return undefined;
  }
  if (!isObject(value)) return undefined;
  if (value.id === id) return value;
  for (const child of Object.values(value)) {
    const result = findObjectById(child, id);
    if (result) return result;
  }
  return undefined;
}

function findObjectByField(value: unknown, field: string, expected: string): JsonObject | undefined {
  if (Array.isArray(value)) {
    for (const item of value) {
      const result = findObjectByField(item, field, expected);
      if (result) return result;
    }
    return undefined;
  }
  if (!isObject(value)) return undefined;
  if (value[field] === expected) return value;
  for (const child of Object.values(value)) {
    const result = findObjectByField(child, field, expected);
    if (result) return result;
  }
  return undefined;
}

function findEntity(value: unknown, id: string): JsonObject | undefined {
  return findObjectById(value, id)
    ?? findObjectByField(value, "taskId", id)
    ?? findObjectByField(value, "responseGroupId", id)
    ?? findObjectByField(value, "slotId", id);
}

function markUserEdited(object: JsonObject): void {
  if ("provenanceStatus" in object) object.provenanceStatus = "user_edited";
}

function expandExpression(expression: QuestionNumberExpressionV2): number[] {
  if (expression.kind === "range") {
    if (expression.start < 1 || expression.end < expression.start || expression.end - expression.start > 200) {
      throw new Error("AUTHORING_PATCH_EXPRESSION_RANGE_INVALID");
    }
    return Array.from({ length: expression.end - expression.start + 1 }, (_, index) => expression.start + index);
  }
  if (expression.kind === "set") return expression.values;
  return expression.values.flatMap((value) => typeof value === "number"
    ? [value]
    : expandExpression({ kind: "range", start: value.start, end: value.end }));
}

function patchText(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "replaceText" }>): void {
  const node = findObjectById(document, patch.nodeId);
  if (!node || node.type !== "text" || typeof node.text !== "string") {
    throw new Error(`AUTHORING_PATCH_TEXT_NODE_REQUIRED:${patch.nodeId}`);
  }
  const chars = Array.from(node.text);
  if (patch.from < 0 || patch.to < patch.from || patch.to > chars.length) {
    throw new Error(`AUTHORING_PATCH_TEXT_RANGE_INVALID:${patch.nodeId}`);
  }
  node.text = `${chars.slice(0, patch.from).join("")}${patch.text}${chars.slice(patch.to).join("")}`;
  markUserEdited(node);
}

function patchNodeAttrs(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setNodeAttrs" }>): void {
  const node = findObjectById(document, patch.nodeId);
  if (!node) throw new Error(`AUTHORING_PATCH_NODE_NOT_FOUND:${patch.nodeId}`);
  const allowed = new Set(["provenanceStatus", "align", "indentLevel", "level", "altText", "placeholder", "displayLabel", "inline", "label", "slotIds", "display"]);
  for (const key of Object.keys(patch.attrs)) {
    if (!allowed.has(key)) throw new Error(`AUTHORING_PATCH_ATTR_NOT_ALLOWED:${key}`);
    node[key] = patch.attrs[key];
  }
  markUserEdited(node);
}

function patchTaskType(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setTaskType" }>): void {
  const task = findObjectByField(document, "taskId", patch.taskId);
  if (!task) throw new Error(`AUTHORING_PATCH_TASK_NOT_FOUND:${patch.taskId}`);
  task.taskType = patch.taskType;
  if (isObject(task.instructionSignature)) task.instructionSignature.taskType = patch.taskType;
  markUserEdited(task);
}

function patchExpression(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setQuestionExpression" }>): void {
  const task = findObjectByField(document, "taskId", patch.taskId);
  if (!task) throw new Error(`AUTHORING_PATCH_TASK_NOT_FOUND:${patch.taskId}`);
  const numbers = expandExpression(patch.expression);
  task.displayRange = patch.expression;
  if (isObject(task.instructionSignature)) {
    task.instructionSignature.expectedQuestionNumbers = numbers;
    task.instructionSignature.expectedSlotCount = numbers.length;
  }
  markUserEdited(task);
}

function patchResponseGroup(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setResponseGroup" }>): void {
  const task = findObjectByField(document, "taskId", patch.taskId);
  if (!task || !Array.isArray(task.responseGroups)) throw new Error(`AUTHORING_PATCH_TASK_NOT_FOUND:${patch.taskId}`);
  const index = task.responseGroups.findIndex((item) => isObject(item) && item.responseGroupId === patch.responseGroup.responseGroupId);
  if (index < 0) throw new Error(`AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:${patch.responseGroup.responseGroupId}`);
  task.responseGroups[index] = patch.responseGroup;
  markUserEdited(task);
}

function patchAnswer(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setAnswer" }>): void {
  if (!document.answerSlots[patch.slotId]) throw new Error(`AUTHORING_PATCH_SLOT_NOT_FOUND:${patch.slotId}`);
  document.answerKey[patch.slotId] = patch.value;
}

function patchSource(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "bindSource" }>): void {
  const entity = findEntity(document, patch.entityId);
  if (!entity) throw new Error(`AUTHORING_PATCH_ENTITY_NOT_FOUND:${patch.entityId}`);
  entity.sourceAnchors = patch.anchors as SourceAnchorV2[];
  markUserEdited(entity);
}

export function applyAuthoringV2Patches(
  authoring: IeltsAuthoringIRV2,
  patches: AuthoringPatchV2[]
): IeltsAuthoringIRV2 {
  const document = structuredClone(authoring);
  for (const patch of patches) {
    switch (patch.op) {
      case "replaceText": patchText(document, patch); break;
      case "setNodeAttrs": patchNodeAttrs(document, patch); break;
      case "setTaskType": patchTaskType(document, patch); break;
      case "setQuestionExpression": patchExpression(document, patch); break;
      case "setResponseGroup": patchResponseGroup(document, patch); break;
      case "setAnswer": patchAnswer(document, patch); break;
      case "bindSource": patchSource(document, patch); break;
    }
  }
  return document;
}

export function answerValueForSelection(labels: string[]): AnswerValueV2 {
  return { kind: "option", labels, assignment: "unordered_set" };
}

export function answerValueForText(value: string): AnswerValueV2 {
  return { kind: "text", values: [value], normalization: "ielts_default" };
}

export function taskTypeLabel(taskType: TaskTypeV2): string {
  const labels: Partial<Record<TaskTypeV2, string>> = {
    multiple_choice: "多选",
    single_choice: "单选",
    matching_headings: "标题匹配",
    summary_completion: "摘要填空",
    note_completion: "笔记填空",
    table_completion: "表格填空",
    diagram_label_completion: "图表填空"
  };
  return labels[taskType] ?? taskType;
}
