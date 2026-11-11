import type {
  AnswerValueV2,
  AuthoringContentTargetV2,
  AuthoringNodeAttributeV2,
  AuthoringPatchV2,
  AuthoringStructureChangeV2,
  IeltsAuthoringIRV2,
  QuestionNumberExpressionV2,
  ResponseGroupV2,
  SourceAnchorV2,
  TaskTypeV2
} from "../types";
import type { ContentNodeV2, DiagramHotspotV2 } from "../types/content-doc-v2";
import { taskTypeLabel as displayTaskTypeLabel } from "../utils/displayLabels";

type JsonObject = Record<string, unknown>;

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isContentNode(value: unknown): value is ContentNodeV2 {
  return isObject(value) && typeof value.type === "string" && typeof value.id === "string";
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
    ?? findObjectByField(value, "slotId", id)
    ?? findObjectByField(value, "optionId", id);
}

function markUserEdited(object: JsonObject, preserveProvenance = false, restoreProvenanceStatus?: string): void {
  if (restoreProvenanceStatus && "provenanceStatus" in object) object.provenanceStatus = restoreProvenanceStatus;
  else if (!preserveProvenance && "provenanceStatus" in object) object.provenanceStatus = "user_edited";
}

function answerSlotIdsInNodes(nodes: ContentNodeV2[]): Set<string> {
  const ids = new Set<string>();
  const visit = (node: ContentNodeV2) => {
    if (node.type === "answer_slot") ids.add(node.slotId);
    for (const child of childArrays(node as unknown as JsonObject)) child.nodes.forEach(visit);
  };
  nodes.forEach(visit);
  return ids;
}

function ensureAnswerSlotsRemain(before: ContentNodeV2[], after: ContentNodeV2[]): void {
  const next = answerSlotIdsInNodes(after);
  const removed = [...answerSlotIdsInNodes(before)].filter((slotId) => !next.has(slotId));
  if (removed.length) throw new Error(`AUTHORING_PATCH_ANSWER_SLOT_LOSS:${removed.join(",")}`);
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

function childArrays(node: JsonObject): Array<{ key: string; nodes: ContentNodeV2[] }> {
  const keys = ["children", "items", "rows", "cells", "caption", "steps"];
  return keys.flatMap((key) => {
    const value = node[key];
    return Array.isArray(value) && value.every((item) => isContentNode(item))
      ? [{ key, nodes: value as ContentNodeV2[] }]
      : [];
  });
}

function contentRoots(document: IeltsAuthoringIRV2): Array<{ target: AuthoringContentTargetV2; nodes: ContentNodeV2[]; owner?: JsonObject }> {
  const roots: Array<{ target: AuthoringContentTargetV2; nodes: ContentNodeV2[]; owner?: JsonObject }> = [];
  if (document.passage) roots.push({ target: { kind: "passage" }, nodes: document.passage.content, owner: document.passage as unknown as JsonObject });
  for (const task of document.taskGroups) {
    roots.push({ target: { kind: "taskInstructions", taskId: task.taskId }, nodes: task.instructions, owner: task as unknown as JsonObject });
    if (task.stimulus) roots.push({ target: { kind: "taskStimulus", taskId: task.taskId }, nodes: task.stimulus, owner: task as unknown as JsonObject });
    for (const group of task.responseGroups) {
      if (group.prompt) roots.push({ target: { kind: "responsePrompt", responseGroupId: group.responseGroupId }, nodes: group.prompt, owner: group as unknown as JsonObject });
    }
    if (task.optionBank) {
      for (const option of task.optionBank.options) {
        roots.push({ target: { kind: "option", optionId: option.optionId }, nodes: option.content, owner: option as unknown as JsonObject });
      }
    }
  }
  return roots;
}

function rootForTarget(document: IeltsAuthoringIRV2, target: AuthoringContentTargetV2): { nodes: ContentNodeV2[]; owner?: JsonObject } {
  if (target.kind === "node") {
    const node = findObjectById(document, target.nodeId);
    if (!node) throw new Error(`AUTHORING_PATCH_NODE_NOT_FOUND:${target.nodeId}`);
    const child = childArrays(node)[0];
    if (!child) throw new Error(`AUTHORING_PATCH_NODE_NOT_CONTAINER:${target.nodeId}`);
    return { nodes: child.nodes, owner: node };
  }
  const root = contentRoots(document).find((candidate) => {
    if (candidate.target.kind !== target.kind) return false;
    if (target.kind === "passage") return true;
    if (target.kind === "taskInstructions" || target.kind === "taskStimulus") {
      return candidate.target.kind === target.kind && candidate.target.taskId === target.taskId;
    }
    if (target.kind === "responsePrompt") {
      return candidate.target.kind === target.kind && candidate.target.responseGroupId === target.responseGroupId;
    }
    return candidate.target.kind === "option" && candidate.target.optionId === target.optionId;
  });
  if (!root) throw new Error(`AUTHORING_PATCH_CONTENT_TARGET_NOT_FOUND:${target.kind}`);
  return { nodes: root.nodes, owner: root.owner };
}

function everyContentRoot(document: IeltsAuthoringIRV2): ContentNodeV2[][] {
  return contentRoots(document).map((root) => root.nodes);
}

function validateIndex(index: number, length: number): void {
  if (!Number.isInteger(index) || index < 0 || index > length) throw new Error(`AUTHORING_PATCH_INDEX_INVALID:${index}`);
}

function patchText(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "replaceText" }>): void {
  const node = findObjectById(document, patch.nodeId);
  if (!node || node.type !== "text" || typeof node.text !== "string") {
    throw new Error(`AUTHORING_PATCH_TEXT_NODE_REQUIRED:${patch.nodeId}`);
  }
  if (patch.expectedText !== undefined && patch.expectedText !== node.text) {
    throw new Error(`AUTHORING_PATCH_TEXT_CONFLICT:${patch.nodeId}`);
  }
  const chars = Array.from(node.text);
  if (patch.from < 0 || patch.to < patch.from || patch.to > chars.length) {
    throw new Error(`AUTHORING_PATCH_TEXT_RANGE_INVALID:${patch.nodeId}`);
  }
  node.text = `${chars.slice(0, patch.from).join("")}${patch.text}${chars.slice(patch.to).join("")}`;
  markUserEdited(node, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

function patchNodeAttrs(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setNodeAttrs" }>): void {
  const node = findObjectById(document, patch.nodeId);
  if (!node) throw new Error(`AUTHORING_PATCH_NODE_NOT_FOUND:${patch.nodeId}`);
  const allowed = new Set(["assetId", "rowSpan", "colSpan", "headerScope", "provenanceStatus", "align", "indentLevel", "level", "fontScale", "altText", "placeholder", "displayLabel", "inline", "label", "slotIds", "display", "crop"]);
  if ("fontScale" in patch.attrs && (node.type !== "heading" || ![0.9, 1, 1.15, 1.3, 1.5].includes(patch.attrs.fontScale as number))) {
    throw new Error("AUTHORING_PATCH_FONT_SCALE_INVALID");
  }
  for (const key of Object.keys(patch.attrs)) {
    if (!allowed.has(key)) throw new Error(`AUTHORING_PATCH_ATTR_NOT_ALLOWED:${key}`);
    node[key] = patch.attrs[key as keyof typeof patch.attrs];
  }
  for (const key of patch.removeAttrs ?? []) {
    if (!allowed.has(key)) throw new Error(`AUTHORING_PATCH_ATTR_NOT_ALLOWED:${key}`);
    delete node[key];
  }
  markUserEdited(node, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

function patchReplaceContent(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "replaceContent" }>): void {
  const root = rootForTarget(document, patch.target);
  ensureAnswerSlotsRemain(root.nodes, patch.content);
  root.nodes.splice(0, root.nodes.length, ...structuredClone(patch.content));
  if (root.owner) markUserEdited(root.owner, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

function patchInsertNode(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "insertNode" }>): void {
  const root = patch.parentId
    ? rootForTarget(document, { kind: "node", nodeId: patch.parentId })
    : rootForTarget(document, patch.target);
  validateIndex(patch.index, root.nodes.length);
  root.nodes.splice(patch.index, 0, structuredClone(patch.node));
  if (root.owner) markUserEdited(root.owner);
}

function removeNode(nodes: ContentNodeV2[], nodeId: string): { node: ContentNodeV2; index: number } | undefined {
  const index = nodes.findIndex((node) => node.id === nodeId);
  if (index >= 0) {
    const [node] = nodes.splice(index, 1);
    return { node, index };
  }
  for (const node of nodes) {
    for (const child of childArrays(node as unknown as JsonObject)) {
      const removed = removeNode(child.nodes, nodeId);
      if (removed) return removed;
    }
  }
  return undefined;
}

function containsAnswerSlot(node: ContentNodeV2): boolean {
  if (node.type === "answer_slot") return true;
  return childArrays(node as unknown as JsonObject).some(({ nodes }) => nodes.some(containsAnswerSlot));
}

function patchDeleteNode(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "deleteNode" }>): void {
  const location = locateContentNode(document, patch.nodeId);
  if (!location) throw new Error(`AUTHORING_PATCH_NODE_NOT_FOUND:${patch.nodeId}`);
  if (containsAnswerSlot(location.node) && !patch.allowAnswerSlotRemoval) {
    throw new Error(`AUTHORING_PATCH_NODE_CONTAINS_ANSWER_SLOT:${patch.nodeId}`);
  }
  for (const root of everyContentRoot(document)) {
    const removed = removeNode(root, patch.nodeId);
    if (removed) return;
  }
  throw new Error(`AUTHORING_PATCH_NODE_NOT_FOUND:${patch.nodeId}`);
}

function patchMoveNode(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "moveNode" }>): void {
  const source = everyContentRoot(document).map((nodes) => removeNode(nodes, patch.nodeId)).find(Boolean);
  if (!source) throw new Error(`AUTHORING_PATCH_NODE_NOT_FOUND:${patch.nodeId}`);
  const root = patch.parentId
    ? rootForTarget(document, { kind: "node", nodeId: patch.parentId })
    : rootForTarget(document, patch.target);
  validateIndex(patch.index, root.nodes.length);
  root.nodes.splice(patch.index, 0, source.node);
  if (root.owner) markUserEdited(root.owner);
}

function validateNormalizedRect(rect: readonly number[], label: string): asserts rect is [number, number, number, number] {
  if (rect.length !== 4 || rect.some((value) => !Number.isFinite(value) || value < 0 || value > 1)) {
    throw new Error(`AUTHORING_PATCH_${label}_RECT_INVALID`);
  }
  if (rect[2] <= 0 || rect[3] <= 0 || rect[0] + rect[2] > 1 || rect[1] + rect[3] > 1) {
    throw new Error(`AUTHORING_PATCH_${label}_RECT_OUT_OF_BOUNDS`);
  }
}

function patchCropAsset(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "cropAsset" }>): void {
  const node = findObjectById(document, patch.nodeId);
  if (!node || !["figure", "image", "diagram"].includes(String(node.type))) throw new Error(`AUTHORING_PATCH_ASSET_NODE_REQUIRED:${patch.nodeId}`);
  if (patch.crop) validateNormalizedRect(patch.crop, "CROP");
  if (patch.crop) node.crop = [...patch.crop];
  else delete node.crop;
  markUserEdited(node, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

function patchHotspot(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setHotspot" }>): void {
  const node = findObjectById(document, patch.nodeId);
  if (!node || !["figure", "diagram"].includes(String(node.type))) throw new Error(`AUTHORING_PATCH_HOTSPOT_NODE_REQUIRED:${patch.nodeId}`);
  if (!patch.hotspot.hotspotId || !patch.hotspot.slotId) throw new Error("AUTHORING_PATCH_HOTSPOT_ID_REQUIRED");
  validateNormalizedRect(patch.hotspot.normalizedRect, "HOTSPOT");
  if (patch.hotspot.labelAnchor) {
    if (patch.hotspot.labelAnchor.length !== 2 || patch.hotspot.labelAnchor.some((value) => value < 0 || value > 1)) throw new Error("AUTHORING_PATCH_HOTSPOT_LABEL_ANCHOR_INVALID");
  }
  const hotspots = Array.isArray(node.hotspots) ? node.hotspots as DiagramHotspotV2[] : [];
  const index = hotspots.findIndex((item) => item.hotspotId === patch.hotspot.hotspotId);
  if (index >= 0) hotspots[index] = structuredClone(patch.hotspot);
  else hotspots.push(structuredClone(patch.hotspot));
  node.hotspots = hotspots;
  markUserEdited(node, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

function patchRemoveHotspot(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "removeHotspot" }>): void {
  const node = findObjectById(document, patch.nodeId);
  if (!node || !["figure", "diagram"].includes(String(node.type))) throw new Error(`AUTHORING_PATCH_HOTSPOT_NODE_REQUIRED:${patch.nodeId}`);
  const hotspots = Array.isArray(node.hotspots) ? node.hotspots as DiagramHotspotV2[] : [];
  const next = hotspots.filter((hotspot) => hotspot.hotspotId !== patch.hotspotId);
  if (next.length === hotspots.length) throw new Error(`AUTHORING_PATCH_HOTSPOT_NOT_FOUND:${patch.hotspotId}`);
  node.hotspots = next;
  markUserEdited(node, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

function patchTaskType(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setTaskType" }>): void {
  const task = findObjectByField(document, "taskId", patch.taskId);
  if (!task) throw new Error(`AUTHORING_PATCH_TASK_NOT_FOUND:${patch.taskId}`);
  task.taskType = patch.taskType;
  if (isObject(task.instructionSignature)) task.instructionSignature.taskType = patch.taskType;
  markUserEdited(task, patch.preserveProvenance, patch.restoreProvenanceStatus);
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
  markUserEdited(task, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

function patchResponseCardinality(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setResponseCardinality" }>): void {
  const task = document.taskGroups.find((candidate) => candidate.taskId === patch.taskId);
  const group = task?.responseGroups.find((candidate) => candidate.responseGroupId === patch.responseGroupId) as unknown as JsonObject | undefined;
  if (!group) throw new Error(`AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:${patch.responseGroupId}`);
  const { min, max, exact } = patch.cardinality;
  if (!Number.isInteger(min) || !Number.isInteger(max) || min < 0 || max < min || (exact !== undefined && (exact < min || exact > max))) throw new Error("AUTHORING_PATCH_CARDINALITY_INVALID");
  group.cardinality = structuredClone(patch.cardinality);
  markUserEdited(group, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

function patchResponseGroup(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setResponseGroup" }>): void {
  const task = findObjectByField(document, "taskId", patch.taskId);
  if (!task || !Array.isArray(task.responseGroups)) throw new Error(`AUTHORING_PATCH_TASK_NOT_FOUND:${patch.taskId}`);
  const index = task.responseGroups.findIndex((item) => isObject(item) && item.responseGroupId === patch.responseGroup.responseGroupId);
  if (index < 0) throw new Error(`AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:${patch.responseGroup.responseGroupId}`);
  task.responseGroups[index] = structuredClone(patch.responseGroup);
  markUserEdited(task, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

function patchOptionBank(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setOptionBank" }>): void {
  const task = document.taskGroups.find((candidate) => candidate.taskId === patch.taskId);
  if (!task) throw new Error(`AUTHORING_PATCH_TASK_NOT_FOUND:${patch.taskId}`);
  task.optionBank = patch.optionBank ? structuredClone(patch.optionBank) : undefined;
  markUserEdited(task as unknown as JsonObject, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

// Structural edits traverse container wrappers too (table rows/cells, flow steps).
function visitJson(value: unknown, visit: (object: JsonObject) => void): void {
  if (Array.isArray(value)) value.forEach((child) => visitJson(child, visit));
  else if (isObject(value)) { visit(value); Object.values(value).forEach((child) => visitJson(child, visit)); }
}

function hasEmbeddedSlot(value: unknown): boolean {
  let found = false;
  visitJson(value, (object) => { if (object.type === "answer_slot") found = true; });
  return found;
}

function validNewSubtree(document: IeltsAuthoringIRV2, node: ContentNodeV2): boolean {
  const ids = new Set<string>();
  let valid = true;
  visitJson(node, (value) => {
    if (typeof value.id === "string") {
      if (ids.has(value.id) || findObjectById(document, value.id)) valid = false;
      ids.add(value.id);
    }
  });
  return valid;
}

function perSlotPrompt(document: IeltsAuthoringIRV2, group: ResponseGroupV2): boolean {
  const hosts = new Set<string>();
  return group.assignment !== "unordered_set" && group.slotIds.length > 0 && group.slotIds.every((id) => {
    const host = document.answerSlots[id]?.hostNodeId;
    const node = group.prompt?.find((node) => node.id === host);
    if (!host || !node || hosts.has(host) || hasEmbeddedSlot(node)) return false;
    hosts.add(host);
    return true;
  });
}

function collectNodeIds(value: unknown): Set<string> {
  const ids = new Set<string>();
  visitJson(value, (node) => { if (typeof node.id === "string") ids.add(node.id); });
  return ids;
}

function cleanBindings(document: IeltsAuthoringIRV2, removed: Set<string>): void {
  const map = document.passage?.paragraphMap;
  if (map) for (const [label, id] of Object.entries(map)) if (removed.has(id)) delete map[label];
  for (const slot of Object.values(document.answerSlots)) if (slot.hostNodeId && removed.has(slot.hostNodeId)) delete slot.hostNodeId;
}

function removeSlots(document: IeltsAuthoringIRV2, ids: Set<string>): void {
  const hosts = new Set<string>();
  for (const task of document.taskGroups) for (const group of task.responseGroups) {
    if (perSlotPrompt(document, group)) for (const id of group.slotIds) {
      const host = document.answerSlots[id]?.hostNodeId;
      if (ids.has(id) && host) hosts.add(host);
    }
  }
  for (const [id, slot] of Object.entries(document.answerSlots)) if (!ids.has(id) && slot.hostNodeId) hosts.delete(slot.hostNodeId);
  const removed = new Set<string>();
  const clean = (value: unknown): void => {
    if (Array.isArray(value)) {
      for (let index = value.length - 1; index >= 0; index -= 1) {
        const node = value[index];
        if (isObject(node) && ((node.type === "answer_slot" && ids.has(String(node.slotId))) || hosts.has(String(node.id)))) {
          for (const id of collectNodeIds(node)) removed.add(id);
          value.splice(index, 1);
        } else clean(node);
      }
    } else if (isObject(value)) {
      if (Array.isArray(value.slotIds)) value.slotIds = value.slotIds.filter((id) => !ids.has(String(id)));
      if (Array.isArray(value.hotspots)) value.hotspots = value.hotspots.filter((h) => !isObject(h) || !ids.has(String(h.slotId)));
      Object.values(value).forEach(clean);
    }
  };
  clean(document);
  for (const task of document.taskGroups) for (const group of task.responseGroups) group.slotIds = group.slotIds.filter((id) => !ids.has(id));
  for (const id of ids) { delete document.answerSlots[id]; delete document.answerKey[id]; }
  cleanBindings(document, removed);
}

function refreshListening(document: IeltsAuthoringIRV2, before: IeltsAuthoringIRV2): void {
  if (!document.listening) return;
  const taskNumbers = (draft: IeltsAuthoringIRV2, ids: string[]) => new Set(draft.taskGroups.filter((t) => ids.includes(t.taskId))
    .flatMap((t) => t.responseGroups.flatMap((g) => g.slotIds)).map((id) => draft.answerSlots[id]?.questionNumber).filter((n): n is number => Number.isInteger(n)));
  for (const part of document.listening.parts) {
    const oldNumbers = taskNumbers(before, part.taskIds);
    const newNumbers = taskNumbers(document, part.taskIds);
    part.taskIds = part.taskIds.filter((id) => document.taskGroups.some((t) => t.taskId === id));
    part.expectedQuestionNumbers = part.expectedQuestionNumbers.filter((n) => !oldNumbers.has(n) || newNumbers.has(n));
    for (const n of [...newNumbers].filter((n) => !oldNumbers.has(n)).sort((a, b) => a - b)) if (!part.expectedQuestionNumbers.includes(n)) part.expectedQuestionNumbers.push(n);
  }
}

function refreshNumbers(document: IeltsAuthoringIRV2, taskId: string, expression?: QuestionNumberExpressionV2): void {
  const task = document.taskGroups.find((t) => t.taskId === taskId)!;
  const numbers = [...new Set(task.responseGroups.flatMap((g) => g.slotIds).map((id) => document.answerSlots[id]?.questionNumber)
    .filter((n): n is number => Number.isInteger(n)))].sort((a, b) => a - b);
  if (!expression && numbers.length === 0) throw new Error("AUTHORING_PATCH_EXPRESSION_EMPTY");
  patchExpression(document, { op: "setQuestionExpression", taskId, expression: expression ?? { kind: "set", values: numbers } });
}

function patchInsertAnswerSlot(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "insertAnswerSlot" }>): void {
  const task = document.taskGroups.find((candidate) => candidate.taskId === patch.taskId);
  if (!task) throw new Error(`AUTHORING_PATCH_TASK_NOT_FOUND:${patch.taskId}`);
  const group = task.responseGroups.find((candidate) => candidate.responseGroupId === patch.responseGroupId);
  if (!group) throw new Error(`AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:${patch.responseGroupId}`);
  const id = patch.slot.slotId;
  if (document.answerSlots[id]) throw new Error(`AUTHORING_PATCH_SLOT_ALREADY_EXISTS:${id}`);
  validateIndex(patch.slotIndex, group.slotIds.length);
  const slot = structuredClone(patch.slot);
  if (patch.node) {
    if (patch.node.type !== "answer_slot" || patch.node.slotId !== id) throw new Error("AUTHORING_PATCH_SLOT_NODE_MISMATCH");
    if (findObjectById(document, patch.node.id)) throw new Error(`AUTHORING_PATCH_SLOT_ALREADY_EXISTS:${id}`);
    if (!patch.target) throw new Error("AUTHORING_PATCH_CONTENT_TARGET_REQUIRED");
    const root = patch.parentId ? rootForTarget(document, { kind: "node", nodeId: patch.parentId }) : rootForTarget(document, patch.target);
    validateIndex(patch.index!, root.nodes.length);
    root.nodes.splice(patch.index!, 0, structuredClone(patch.node));
  } else if (patch.promptNode || perSlotPrompt(document, group)) {
    const prompt: ContentNodeV2 = structuredClone(patch.promptNode ?? {
      type: "paragraph", id: `${id}-prompt`, children: [{ type: "text", id: `${id}-prompt-text`, text: "新题干", sourceAnchors: [], provenanceStatus: "manual" }], sourceAnchors: [], provenanceStatus: "manual"
    });
    if (!prompt.id || !validNewSubtree(document, prompt) || hasEmbeddedSlot(prompt)) throw new Error("AUTHORING_PATCH_PROMPT_INVALID");
    slot.hostNodeId = prompt.id; slot.hostType = "prompt";
    (group.prompt ??= []).push(prompt);
  }
  group.slotIds.splice(patch.slotIndex, 0, id);
  document.answerSlots[id] = slot;
  document.answerKey[id] = structuredClone(patch.value);
  refreshNumbers(document, patch.taskId, patch.expression);
}

function patchDeleteAnswerSlot(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "deleteAnswerSlot" }>): void {
  const task = document.taskGroups.find((candidate) => candidate.taskId === patch.taskId);
  if (!task) throw new Error(`AUTHORING_PATCH_TASK_NOT_FOUND:${patch.taskId}`);
  const group = task.responseGroups.find((candidate) => candidate.responseGroupId === patch.responseGroupId);
  if (!group) throw new Error(`AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:${patch.responseGroupId}`);
  if (!group.slotIds.includes(patch.slotId) || !document.answerSlots[patch.slotId]) throw new Error(`AUTHORING_PATCH_SLOT_NOT_FOUND:${patch.slotId}`);
  if (group.slotIds.length <= 1) throw new Error("AUTHORING_PATCH_LAST_ANSWER_SLOT");
  if (patch.nodeId) {
    const node = findObjectById(document, patch.nodeId);
    if (!node || node.type !== "answer_slot" || node.slotId !== patch.slotId) throw new Error(`AUTHORING_PATCH_SLOT_NODE_NOT_FOUND:${patch.slotId}`);
  }
  removeSlots(document, new Set([patch.slotId]));
  refreshNumbers(document, patch.taskId, patch.expression);
}

function patchDeleteTaskGroup(document: IeltsAuthoringIRV2, taskId: string): void {
  const task = document.taskGroups.find((t) => t.taskId === taskId);
  if (!task) throw new Error(`AUTHORING_PATCH_TASK_NOT_FOUND:${taskId}`);
  removeSlots(document, new Set(task.responseGroups.flatMap((g) => g.slotIds)));
  document.taskGroups = document.taskGroups.filter((t) => t.taskId !== taskId);
  if (task.optionBank) for (const other of document.taskGroups) {
    if (!other.optionBank && other.responseGroups.some((g) => g.optionBankRef === task.optionBank!.optionBankId)) other.optionBank = structuredClone(task.optionBank);
  }
  const removed = new Set([...collectNodeIds(task)].filter((id) => !findObjectById(document, id)));
  cleanBindings(document, removed);
}

function patchDeleteResponseGroup(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "deleteResponseGroup" }>): void {
  const task = document.taskGroups.find((t) => t.taskId === patch.taskId);
  if (!task) throw new Error(`AUTHORING_PATCH_TASK_NOT_FOUND:${patch.taskId}`);
  const group = task.responseGroups.find((g) => g.responseGroupId === patch.responseGroupId);
  if (!group) throw new Error(`AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:${patch.responseGroupId}`);
  if (task.responseGroups.length === 1) return patchDeleteTaskGroup(document, patch.taskId);
  removeSlots(document, new Set(group.slotIds));
  task.responseGroups = task.responseGroups.filter((g) => g.responseGroupId !== patch.responseGroupId);
  if (group.optionBankRef && !document.taskGroups.some((t) => t.responseGroups.some((g) => g.optionBankRef === group.optionBankRef))
    && task.optionBank?.optionBankId === group.optionBankRef) delete task.optionBank;
  const removed = new Set([...collectNodeIds(group)].filter((id) => !findObjectById(document, id)));
  cleanBindings(document, removed);
  refreshNumbers(document, patch.taskId);
}

// Undo stores only changed paths and checks the post-edit value before restoring.
// This prevents a delayed undo from silently overwriting intervening cloud edits.
function equalJson(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((value, i) => equalJson(value, b[i]));
  if (isObject(a) && isObject(b)) return Object.keys(a).length === Object.keys(b).length && Object.keys(a).every((key) => Object.hasOwn(b, key) && equalJson(a[key], b[key]));
  return false;
}

function structureChanges(before: unknown, after: unknown, path: string[] = []): AuthoringStructureChangeV2[] {
  if (equalJson(before, after)) return [];
  if (isObject(before) && isObject(after)) return [...new Set([...Object.keys(before), ...Object.keys(after)])]
    .flatMap((key) => structureChanges(before[key], after[key], [...path, key]));
  if (Array.isArray(before) && Array.isArray(after) && before.length === after.length
      && before.every((v, i) => !isObject(v) || !isObject(after[i]) || (v.id ?? v.taskId ?? v.responseGroupId) === (after[i].id ?? after[i].taskId ?? after[i].responseGroupId))) {
    return before.flatMap((v, i) => structureChanges(v, after[i], [...path, String(i)]));
  }
  return [{ path, ...(before !== undefined ? { before: structuredClone(before) } : {}), ...(after !== undefined ? { after: structuredClone(after) } : {}) }];
}

function patchRestoreStructure(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "restoreStructure" }>): void {
  for (const change of patch.changes) {
    const { path } = change;
    if (!path.length || !["taskGroups", "answerSlots", "answerKey", "passage", "listening"].includes(path[0]) || path.some((key) => ["__proto__", "prototype", "constructor"].includes(key))) throw new Error("AUTHORING_PATCH_RESTORE_INVALID");
    let parent: any = document;
    for (const key of path.slice(0, -1)) {
      if (parent === null || typeof parent !== "object" || !Object.hasOwn(parent, key)) throw new Error("AUTHORING_PATCH_RESTORE_CONFLICT");
      parent = parent[key];
    }
    const key = path[path.length - 1];
    if (Array.isArray(parent) && (!/^(0|[1-9][0-9]*)$/.test(key) || Number(key) >= parent.length || !Object.hasOwn(change, "before") || !Object.hasOwn(change, "after"))) throw new Error("AUTHORING_PATCH_RESTORE_INVALID");
    if (!parent || typeof parent !== "object" || !equalJson(parent[key], change.after)) throw new Error("AUTHORING_PATCH_RESTORE_CONFLICT");
    if (Object.hasOwn(change, "before")) parent[key] = structuredClone(change.before);
    else if (Array.isArray(parent)) throw new Error("AUTHORING_PATCH_RESTORE_INVALID");
    else delete parent[key];
  }
}

function patchAnswer(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "setAnswer" }>): void {
  if (!document.answerSlots[patch.slotId]) throw new Error(`AUTHORING_PATCH_SLOT_NOT_FOUND:${patch.slotId}`);
  document.answerKey[patch.slotId] = patch.value;
}

function patchSource(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "bindSource" }>): void {
  const entity = findEntity(document, patch.entityId);
  if (!entity) throw new Error(`AUTHORING_PATCH_ENTITY_NOT_FOUND:${patch.entityId}`);
  entity.sourceAnchors = patch.anchors as SourceAnchorV2[];
  markUserEdited(entity, patch.preserveProvenance, patch.restoreProvenanceStatus);
}

function patchResolveIssue(document: IeltsAuthoringIRV2, patch: Extract<AuthoringPatchV2, { op: "resolveIssue" }>): void {
  const issue = document.quality.issues.find((candidate) => candidate.issueId === patch.issueId);
  if (!issue) throw new Error(`AUTHORING_PATCH_ISSUE_NOT_FOUND:${patch.issueId}`);
  issue.details = { ...(issue.details ?? {}), resolution: patch.resolution, ...(patch.note ? { note: patch.note } : {}) };
}

export function applyAuthoringV2Patches(authoring: IeltsAuthoringIRV2, patches: AuthoringPatchV2[]): IeltsAuthoringIRV2 {
  const document = structuredClone(authoring);
  for (const patch of patches) {
    const structural = ["insertAnswerSlot", "deleteAnswerSlot", "deleteResponseGroup", "deleteTaskGroup"].includes(patch.op);
    const beforeStructure = structural ? structuredClone(document) : undefined;
    switch (patch.op) {
      case "setExamCategory":
        if (document.modality && document.modality !== "reading") throw new Error("AUTHORING_PATCH_READING_REQUIRED");
        if (!["P1", "P2", "P3"].includes(patch.category)) throw new Error("AUTHORING_PATCH_CATEGORY_INVALID");
        document.exam.category = patch.category;
        break;
      case "replaceText": patchText(document, patch); break;
      case "setNodeAttrs": patchNodeAttrs(document, patch); break;
      case "replaceContent": patchReplaceContent(document, patch); break;
      case "insertNode": patchInsertNode(document, patch); break;
      case "deleteNode": patchDeleteNode(document, patch); break;
      case "moveNode": patchMoveNode(document, patch); break;
      case "cropAsset": patchCropAsset(document, patch); break;
      case "setHotspot": patchHotspot(document, patch); break;
      case "removeHotspot": patchRemoveHotspot(document, patch); break;
      case "setTaskType": patchTaskType(document, patch); break;
      case "setQuestionExpression": patchExpression(document, patch); break;
      case "setResponseCardinality": patchResponseCardinality(document, patch); break;
      case "setResponseGroup": patchResponseGroup(document, patch); break;
      case "setOptionBank": patchOptionBank(document, patch); break;
      case "insertAnswerSlot": patchInsertAnswerSlot(document, patch); break;
      case "deleteAnswerSlot": patchDeleteAnswerSlot(document, patch); break;
      case "deleteTaskGroup": patchDeleteTaskGroup(document, patch.taskId); break;
      case "deleteResponseGroup": patchDeleteResponseGroup(document, patch); break;
      case "restoreStructure": patchRestoreStructure(document, patch); break;
      case "setAnswer": patchAnswer(document, patch); break;
      case "bindSource": patchSource(document, patch); break;
      case "resolveIssue": patchResolveIssue(document, patch); break;
    }
    if (beforeStructure) refreshListening(document, beforeStructure);
  }
  return document;
}

export interface ContentNodeLocationV2 {
  target: AuthoringContentTargetV2;
  parentId?: string;
  index: number;
  node: ContentNodeV2;
}

function locateInNodes(nodes: ContentNodeV2[], wantedId: string, target: AuthoringContentTargetV2, parentId?: string): ContentNodeLocationV2 | undefined {
  for (let index = 0; index < nodes.length; index += 1) {
    const node = nodes[index];
    if (node.id === wantedId) return { target, parentId, index, node };
    for (const child of childArrays(node as unknown as JsonObject)) {
      const found = locateInNodes(child.nodes, wantedId, { kind: "node", nodeId: node.id }, node.id);
      if (found) return found;
    }
  }
  return undefined;
}

export function locateContentNode(authoring: IeltsAuthoringIRV2, nodeId: string): ContentNodeLocationV2 | undefined {
  for (const root of contentRoots(authoring)) {
    const found = locateInNodes(root.nodes, nodeId, root.target);
    if (found) return found;
  }
  return undefined;
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

function responseGroup(authoring: IeltsAuthoringIRV2, taskId: string, responseGroupId: string): ResponseGroupV2 | undefined {
  const task = authoring.taskGroups.find((candidate) => candidate.taskId === taskId);
  return task?.responseGroups.find((group) => group.responseGroupId === responseGroupId);
}

export function inverseAuthoringPatch(authoring: IeltsAuthoringIRV2, patch: AuthoringPatchV2): AuthoringPatchV2 | undefined {
  switch (patch.op) {
    case "setExamCategory": return { op: "setExamCategory", category: authoring.exam.category ?? "P1" };
    case "replaceText": {
      const node = findObjectById(authoring, patch.nodeId);
      if (!node || typeof node.text !== "string") return undefined;
      const chars = Array.from(node.text);
      return { op: "replaceText", nodeId: patch.nodeId, from: patch.from, to: patch.from + Array.from(patch.text).length, text: chars.slice(patch.from, patch.to).join(""), preserveProvenance: true, restoreProvenanceStatus: typeof node.provenanceStatus === "string" ? node.provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined };
    }
    case "setNodeAttrs": {
      const node = findObjectById(authoring, patch.nodeId);
      if (!node) return undefined;
      const attrs: AuthoringNodeAttributeV2 = {};
      const removeAttrs: Array<keyof AuthoringNodeAttributeV2> = [];
      for (const key of Object.keys(patch.attrs) as Array<keyof AuthoringNodeAttributeV2>) {
        if (key in node) attrs[key] = clone(node[key]) as never;
        else removeAttrs.push(key);
      }
      for (const key of patch.removeAttrs ?? []) {
        if (key in node) attrs[key] = clone(node[key]) as never;
      }
      return { op: "setNodeAttrs", nodeId: patch.nodeId, attrs, ...(removeAttrs.length ? { removeAttrs } : {}), preserveProvenance: true, restoreProvenanceStatus: typeof node.provenanceStatus === "string" ? node.provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined };
    }
    case "replaceContent": {
      try {
        const root = rootForTarget(authoring, patch.target);
        return { op: "replaceContent", target: clone(patch.target), content: clone(root.nodes), preserveProvenance: true, restoreProvenanceStatus: typeof root.owner?.provenanceStatus === "string" ? root.owner.provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined };
      } catch {
        return undefined;
      }
    }
    case "insertNode":
      // A user delete must remain fail-closed when it would remove an answer
      // slot. The inverse of a just-inserted node is different: it restores
      // the exact previous tree, so it is explicitly allowed to remove the
      // slot that the same patch introduced.
      return { op: "deleteNode", nodeId: patch.node.id, allowAnswerSlotRemoval: true };
    case "deleteNode": {
      const location = locateContentNode(authoring, patch.nodeId);
      return location ? { op: "insertNode", target: clone(location.target), parentId: location.parentId, index: location.index, node: clone(location.node) } : undefined;
    }
    case "moveNode": {
      const location = locateContentNode(authoring, patch.nodeId);
      return location ? { op: "moveNode", nodeId: patch.nodeId, target: clone(location.target), parentId: location.parentId, index: location.index } : undefined;
    }
    case "cropAsset": {
      const node = findObjectById(authoring, patch.nodeId);
      const crop = Array.isArray(node?.crop) ? node.crop as [number, number, number, number] : null;
      return { op: "cropAsset", nodeId: patch.nodeId, crop: crop ? clone(crop) : null, preserveProvenance: true, restoreProvenanceStatus: typeof node?.provenanceStatus === "string" ? node.provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined };
    }
    case "setHotspot": {
      const node = findObjectById(authoring, patch.nodeId);
      const hotspots = Array.isArray(node?.hotspots) ? node.hotspots as DiagramHotspotV2[] : [];
      const previous = hotspots.find((hotspot) => hotspot.hotspotId === patch.hotspot.hotspotId);
      return previous ? { op: "setHotspot", nodeId: patch.nodeId, hotspot: clone(previous), preserveProvenance: true, restoreProvenanceStatus: typeof node?.provenanceStatus === "string" ? node.provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined } : { op: "removeHotspot", nodeId: patch.nodeId, hotspotId: patch.hotspot.hotspotId, preserveProvenance: true, restoreProvenanceStatus: typeof node?.provenanceStatus === "string" ? node.provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined };
    }
    case "removeHotspot": {
      const node = findObjectById(authoring, patch.nodeId);
      const hotspots = Array.isArray(node?.hotspots) ? node.hotspots as DiagramHotspotV2[] : [];
      const previous = hotspots.find((hotspot) => hotspot.hotspotId === patch.hotspotId);
      return previous ? { op: "setHotspot", nodeId: patch.nodeId, hotspot: clone(previous), preserveProvenance: true, restoreProvenanceStatus: typeof node?.provenanceStatus === "string" ? node.provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined } : undefined;
    }
    case "setTaskType": {
      const task = authoring.taskGroups.find((candidate) => candidate.taskId === patch.taskId);
      return task ? { op: "setTaskType", taskId: patch.taskId, taskType: task.taskType, preserveProvenance: true, restoreProvenanceStatus: typeof (task as unknown as JsonObject).provenanceStatus === "string" ? (task as unknown as JsonObject).provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined } : undefined;
    }
    case "setQuestionExpression": {
      const task = authoring.taskGroups.find((candidate) => candidate.taskId === patch.taskId);
      return task ? { op: "setQuestionExpression", taskId: patch.taskId, expression: clone(task.displayRange), preserveProvenance: true, restoreProvenanceStatus: typeof (task as unknown as JsonObject).provenanceStatus === "string" ? (task as unknown as JsonObject).provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined } : undefined;
    }
    case "setResponseCardinality": {
      const group = responseGroup(authoring, patch.taskId, patch.responseGroupId);
      return group ? { op: "setResponseCardinality", taskId: patch.taskId, responseGroupId: patch.responseGroupId, cardinality: clone(group.cardinality), preserveProvenance: true, restoreProvenanceStatus: typeof (group as unknown as JsonObject).provenanceStatus === "string" ? (group as unknown as JsonObject).provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined } : undefined;
    }
    case "setResponseGroup": {
      const group = responseGroup(authoring, patch.taskId, patch.responseGroup.responseGroupId);
      return group ? { op: "setResponseGroup", taskId: patch.taskId, responseGroup: clone(group), preserveProvenance: true, restoreProvenanceStatus: typeof (group as unknown as JsonObject).provenanceStatus === "string" ? (group as unknown as JsonObject).provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined } : undefined;
    }
    case "setOptionBank": {
      const task = authoring.taskGroups.find((candidate) => candidate.taskId === patch.taskId);
      return task ? { op: "setOptionBank", taskId: patch.taskId, optionBank: clone(task.optionBank ?? null), preserveProvenance: true, restoreProvenanceStatus: typeof (task as unknown as JsonObject).provenanceStatus === "string" ? (task as unknown as JsonObject).provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined } : undefined;
    }
    case "insertAnswerSlot":
    case "deleteAnswerSlot":
    case "deleteResponseGroup":
    case "deleteTaskGroup": {
      const after = applyAuthoringV2Patches(authoring, [patch]);
      const changes = structureChanges(authoring, after).filter((change) => ["taskGroups", "answerSlots", "answerKey", "passage", "listening"].includes(change.path[0]));
      return { op: "restoreStructure", taskIds: [patch.taskId], changes };
    }
    case "restoreStructure": return { op: "restoreStructure", taskIds: clone(patch.taskIds), changes: patch.changes.map(({ path, before, after }) => ({ path: clone(path), ...(after !== undefined ? { before: clone(after) } : {}), ...(before !== undefined ? { after: clone(before) } : {}) })) };
    case "setAnswer":
      return { op: "setAnswer", slotId: patch.slotId, value: clone(authoring.answerKey[patch.slotId] ?? { kind: "unresolved" }) };
    case "bindSource": {
      const entity = findEntity(authoring, patch.entityId);
      return entity && Array.isArray(entity.sourceAnchors) ? { op: "bindSource", entityId: patch.entityId, anchors: clone(entity.sourceAnchors as SourceAnchorV2[]), preserveProvenance: true, restoreProvenanceStatus: typeof entity.provenanceStatus === "string" ? entity.provenanceStatus as "source" | "derived" | "user_edited" | "manual" : undefined } : undefined;
    }
    case "resolveIssue": {
      const issue = authoring.quality.issues.find((candidate) => candidate.issueId === patch.issueId);
      const resolution = issue?.details && typeof issue.details.resolution === "string" ? issue.details.resolution : undefined;
      return resolution === "resolved" || resolution === "ignored" ? { op: "resolveIssue", issueId: patch.issueId, resolution } : undefined;
    }
  }
}

export function answerValueForSelection(labels: string[]): AnswerValueV2 {
  return { kind: "option", labels, assignment: "unordered_set" };
}

export function answerValueForText(value: string): AnswerValueV2 {
  return { kind: "text", values: [value], normalization: "ielts_default" };
}

/**
 * 题型 → 用户文案。
 *
 * 这里**不再自己维护一份表**：同一份映射曾经同时存在于本文件与 `utils/displayLabels`，
 * 而题面渲染用的是后者。两张表一旦分叉，「题型名翻译」就会在不同界面里给出不同答案。
 * 现在统一委托给 `displayLabels.taskTypeLabel`。
 */
export function taskTypeLabel(taskType: TaskTypeV2): string {
  return displayTaskTypeLabel(taskType);
}
