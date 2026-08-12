import type {
  AnswerValueV2,
  AuthoringContentTargetV2,
  AuthoringNodeAttributeV2,
  AuthoringPatchV2,
  IeltsAuthoringIRV2,
  QuestionNumberExpressionV2,
  ResponseGroupV2,
  SourceAnchorV2,
  TaskTypeV2
} from "../types";
import type { ContentNodeV2, DiagramHotspotV2 } from "../types/content-doc-v2";

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
  const allowed = new Set(["provenanceStatus", "align", "indentLevel", "level", "altText", "placeholder", "displayLabel", "inline", "label", "slotIds", "display", "crop"]);
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
    switch (patch.op) {
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
      case "setAnswer": patchAnswer(document, patch); break;
      case "bindSource": patchSource(document, patch); break;
      case "resolveIssue": patchResolveIssue(document, patch); break;
    }
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
