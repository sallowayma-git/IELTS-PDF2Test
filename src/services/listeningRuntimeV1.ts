import type {
  AnswerSlotV2,
  AnswerValueV2,
  OptionV2,
  ResponseGroupV2,
  TaskGroupV2
} from "../types/ielts-authoring-v2";
import type {
  ListeningAttemptV1,
  ListeningExamSourceV1,
  ListeningPlaybackSnapshotV1
} from "../types/listening-runtime-v1";
import type { AssetDescriptorV2 } from "../types/schema-common-v2";

export type ListeningAttemptValidationModeV1 = "draft" | "submit";

export interface ListeningRuntimeIssueV1 {
  code: string;
  targetId: string;
  message: string;
}

export interface ListeningRuntimeSlotV1 {
  taskId: string;
  responseGroupId: string;
  slot: AnswerSlotV2;
  options: OptionV2[];
}

export interface ListeningRuntimeResponseGroupV1 {
  taskId: string;
  responseGroupId: string;
  kind: ResponseGroupV2["kind"];
  slotIds: string[];
  options: OptionV2[];
  cardinality: ResponseGroupV2["cardinality"];
  assignment: ResponseGroupV2["assignment"];
  scoringPolicy: ResponseGroupV2["scoringPolicy"];
  duplicatePolicy: ResponseGroupV2["duplicatePolicy"];
  allowOptionReuse: boolean;
}

export interface ListeningRuntimeInteractionModelV1 {
  schemaVersion: "ListeningInteractionModelV1";
  examId: string;
  sourceRevision: number;
  taskGroups: Array<{ taskId: string; taskType: TaskGroupV2["taskType"]; responseGroupIds: string[] }>;
  responseGroups: Record<string, ListeningRuntimeResponseGroupV1>;
  slots: Record<string, ListeningRuntimeSlotV1>;
}

export interface ListeningAttemptScoreV1 {
  correct: boolean;
  earnedPoints: number;
  possiblePoints: number;
  slotScores: Record<string, number>;
  responseGroups: Record<string, { correct: boolean; earnedPoints: number; possiblePoints: number }>;
}

export interface ListeningAssetResolutionV1 {
  assetId: string;
  mime: string;
  byteLength: number;
  sha256: string;
  resourceUri: string;
}

export class ListeningRuntimeErrorV1 extends Error {
  constructor(readonly code: string, message: string, readonly targetId?: string) {
    super(message);
    this.name = "ListeningRuntimeErrorV1";
  }
}

function issue(code: string, targetId: string, message: string): ListeningRuntimeIssueV1 {
  return { code, targetId, message };
}

function throwFirstIssue(issues: ListeningRuntimeIssueV1[]): never {
  const first = issues[0] ?? issue("LISTENING_RUNTIME_INVALID", "runtime", "Listening runtime input is invalid.");
  throw new ListeningRuntimeErrorV1(first.code, first.message, first.targetId);
}

function normalizeOptionLabel(label: string): string {
  return label.trim().toLocaleUpperCase("en-US");
}

function normalizeText(value: string, exact: boolean): string {
  const compact = value.trim().replace(/\s+/gu, " ");
  return exact ? compact : compact.toLocaleLowerCase("en-US");
}

function answerLabels(value: AnswerValueV2 | undefined): string[] {
  return value?.kind === "option" ? value.labels.map(normalizeOptionLabel) : [];
}

function isUnanswered(value: AnswerValueV2 | undefined): boolean {
  if (!value || value.kind === "unresolved") return true;
  return value.kind === "option"
    ? value.labels.length === 0
    : value.values.length === 0 || value.values.every((entry) => !entry.trim());
}

function optionsFor(task: TaskGroupV2, response: ResponseGroupV2): OptionV2[] {
  if (response.options?.length) return response.options;
  if (response.optionBankRef && task.optionBank?.optionBankId === response.optionBankRef) return task.optionBank.options;
  return [];
}

function sourceRevision(source: ListeningExamSourceV1): number {
  return Number.isInteger(source.audit.sourceRevision) && source.audit.sourceRevision >= 0
    ? source.audit.sourceRevision
    : 0;
}

function addIssue(issues: ListeningRuntimeIssueV1[], code: string, targetId: string, message: string): void {
  if (!issues.some((entry) => entry.code === code && entry.targetId === targetId)) issues.push(issue(code, targetId, message));
}

/** Cross-field gate shared by author preview, package staging and student provider. */
export function validateListeningExamSourceV1(source: ListeningExamSourceV1): ListeningRuntimeIssueV1[] {
  const issues: ListeningRuntimeIssueV1[] = [];
  if (source.schemaVersion !== "ListeningExamSourceV1") {
    addIssue(issues, "RUNTIME_SCHEMA_UNSUPPORTED", "schemaVersion", "Only ListeningExamSourceV1 is accepted.");
    return issues;
  }
  if (!source.examId || source.assets.examId !== source.examId) addIssue(issues, "RUNTIME_EXAM_ID_MISMATCH", source.examId || "source", "Source and asset examId must match.");
  if (source.media.probe.status !== "passed" || source.media.probe.issueCodes.length) addIssue(issues, "AUDIO_PROBE_BLOCKED", source.media.assetId, "Audio probe did not pass.");
  const mediaAsset = source.assets.assets.find((asset) => asset.assetId === source.media.assetId);
  if (!mediaAsset) addIssue(issues, "ASSET_REFERENCE_MISSING", source.media.assetId, "Media asset is missing from the source manifest.");
  else {
    if (mediaAsset.kind !== "audio" || mediaAsset.mime !== source.media.mime) addIssue(issues, "AUDIO_CODEC_UNSUPPORTED", source.media.assetId, "Audio descriptor kind or MIME does not match media.");
    if (mediaAsset.sha256.toLowerCase() !== source.media.sha256.toLowerCase()) addIssue(issues, "AUDIO_HASH_MISMATCH", source.media.assetId, "Audio hash does not match the asset descriptor.");
    if (mediaAsset.durationMs !== undefined && mediaAsset.durationMs !== source.media.durationMs) addIssue(issues, "AUDIO_DURATION_MISMATCH", source.media.assetId, "Audio duration does not match the asset descriptor.");
  }
  const policy = source.playbackPolicy;
  if ((!policy.allowReplay && policy.maxPlays !== 1) || (policy.allowReplay && policy.maxPlays === 1)) addIssue(issues, "AUDIO_POLICY_MISSING", source.examId, "Playback policy has an inconsistent replay limit.");
  if (policy.mode === "mock" && (policy.allowPause || policy.allowSeek || policy.allowReplay || policy.maxPlays !== 1 || policy.refreshBehavior !== "resume_from_snapshot" || policy.crashRecoveryBehavior !== "resume_from_snapshot")) {
    addIssue(issues, "AUDIO_POLICY_MISSING", source.examId, "Mock playback must be one-play, no-pause, no-seek and snapshot-resumable.");
  }

  const slotIds = new Set(Object.keys(source.answerSlots));
  const answerIds = new Set(Object.keys(source.answerKey));
  if (answerIds.size !== slotIds.size || [...answerIds].some((slotId) => !slotIds.has(slotId))) addIssue(issues, "RUNTIME_ANSWER_KEY_MISMATCH", source.examId, "Answer key must cover every slot exactly once.");
  const orderIds = new Set(source.questionOrder);
  if (orderIds.size !== source.questionOrder.length || orderIds.size !== slotIds.size || [...orderIds].some((slotId) => !slotIds.has(slotId))) addIssue(issues, "RUNTIME_QUESTION_ORDER_INVALID", source.examId, "questionOrder must contain every slot exactly once.");
  const displayIds = new Set(Object.keys(source.questionDisplayMap));
  if (displayIds.size !== slotIds.size || [...displayIds].some((slotId) => !slotIds.has(slotId))) addIssue(issues, "RUNTIME_DISPLAY_MAP_MISMATCH", source.examId, "questionDisplayMap must cover every slot.");

  const taskIds = new Set<string>();
  const responseIds = new Set<string>();
  const assigned = new Set<string>();
  const taskById = new Map<string, TaskGroupV2>();
  for (const task of source.taskGroups) {
    if (taskIds.has(task.taskId)) addIssue(issues, "RUNTIME_TASK_ID_DUPLICATE", task.taskId, "taskId must be globally unique.");
    taskIds.add(task.taskId);
    taskById.set(task.taskId, task);
    for (const response of task.responseGroups) {
      if (responseIds.has(response.responseGroupId)) addIssue(issues, "RUNTIME_RESPONSE_ID_DUPLICATE", response.responseGroupId, "responseGroupId must be globally unique.");
      responseIds.add(response.responseGroupId);
      const options = optionsFor(task, response);
      if ((response.kind === "choice" || response.kind === "matching" || response.kind === "diagram_hotspot") && options.length === 0) addIssue(issues, "RUNTIME_OPTION_BANK_MISSING", response.responseGroupId, "Choice, matching and hotspot responses require a keyboard option list.");
      const labels = options.map((option) => normalizeOptionLabel(option.label));
      if (new Set(labels).size !== labels.length || labels.some((label) => !label)) addIssue(issues, "RUNTIME_OPTION_LABELS_INVALID", response.responseGroupId, "Keyboard option labels must be unique and non-empty.");
      for (const slotId of response.slotIds) {
        if (!slotIds.has(slotId)) addIssue(issues, "RUNTIME_RESPONSE_SLOT_MISSING", slotId, "Response group references an unknown slot.");
        if (assigned.has(slotId)) addIssue(issues, "RUNTIME_SLOT_ASSIGNED_TWICE", slotId, "A slot may belong to only one response group.");
        assigned.add(slotId);
      }
    }
  }
  for (const slotId of slotIds) {
    if (!assigned.has(slotId)) addIssue(issues, "RUNTIME_SLOT_UNASSIGNED", slotId, "Every scoring slot must belong to one response group.");
    const slot = source.answerSlots[slotId];
    if (slot.slotId !== slotId || source.questionDisplayMap[slotId] !== slot.displayLabel) addIssue(issues, "RUNTIME_SLOT_ID_MISMATCH", slotId, "Slot key, slotId and display map must agree.");
    if (slot.participation !== "scoring") addIssue(issues, "RUNTIME_NON_SCORING_SLOT", slotId, "Published Listening sources only expose scoring slots.");
  }

  const scoringNumbers = new Set(Object.values(source.answerSlots).filter((slot) => slot.participation === "scoring").map((slot) => slot.questionNumber));
  const partNumbers = new Set<number>();
  const assignedTaskIds = new Set<string>();
  let priorCueEnd = -1;
  for (const part of source.parts) {
    for (const taskId of part.taskIds) {
      if (!taskById.has(taskId)) addIssue(issues, "LISTENING_TASK_MISSING", taskId, "Part references an unknown task.");
      if (assignedTaskIds.has(taskId)) addIssue(issues, "LISTENING_TASK_ASSIGNED_TWICE", taskId, "Task is assigned to more than one Part.");
      assignedTaskIds.add(taskId);
    }
    for (const number of part.expectedQuestionNumbers) {
      if (partNumbers.has(number)) addIssue(issues, "QUESTION_NUMBER_DUPLICATE", String(number), "Question number occurs in more than one Part.");
      partNumbers.add(number);
    }
    if (part.cue) {
      if (part.cue.startMs >= part.cue.endMs || part.cue.endMs > source.media.durationMs || part.cue.startMs < priorCueEnd || !part.cue.confirmed || part.cue.confidence < 0.9) addIssue(issues, "AUDIO_CUE_INVALID", part.partId, "Part cue is out of bounds, overlapping, unconfirmed or low-confidence.");
      priorCueEnd = part.cue.endMs;
    }
  }
  if (assignedTaskIds.size !== taskIds.size) addIssue(issues, "LISTENING_TASK_SCOPE_INVALID", source.examId, "Every task must be assigned to exactly one Part.");
  if (partNumbers.size !== scoringNumbers.size || [...scoringNumbers].some((number) => !partNumbers.has(number))) addIssue(issues, "LISTENING_QUESTION_SCOPE_MISMATCH", source.examId, "Part question scope must match scoring slots.");
  if (source.meta.scope === "complete_exam") {
    if (source.parts.length !== 4) addIssue(issues, "LISTENING_COMPLETE_PART_COUNT", source.examId, "A complete exam requires four Parts.");
    if (source.parts.some((part) => part.expectedQuestionNumbers.length !== 10) || scoringNumbers.size !== 40 || [...scoringNumbers].some((number) => number < 1 || number > 40)) addIssue(issues, "LISTENING_COMPLETE_QUESTION_COUNT", source.examId, "A complete exam requires exactly 40 scoring slots, ten per Part.");
  }
  return issues;
}

export function assertListeningExamSourceV1(source: ListeningExamSourceV1): void {
  const issues = validateListeningExamSourceV1(source);
  if (issues.length) throwFirstIssue(issues);
}

export function buildListeningRuntimeInteractionModel(source: ListeningExamSourceV1): ListeningRuntimeInteractionModelV1 {
  assertListeningExamSourceV1(source);
  const responseGroups: Record<string, ListeningRuntimeResponseGroupV1> = {};
  const slots: Record<string, ListeningRuntimeSlotV1> = {};
  const taskGroups: ListeningRuntimeInteractionModelV1["taskGroups"] = [];
  for (const task of source.taskGroups) {
    const responseGroupIds: string[] = [];
    for (const response of task.responseGroups) {
      const options = optionsFor(task, response);
      responseGroups[response.responseGroupId] = { taskId: task.taskId, responseGroupId: response.responseGroupId, kind: response.kind, slotIds: [...response.slotIds], options, cardinality: response.cardinality, assignment: response.assignment, scoringPolicy: response.scoringPolicy, duplicatePolicy: response.duplicatePolicy, allowOptionReuse: response.allowOptionReuse };
      responseGroupIds.push(response.responseGroupId);
      for (const slotId of response.slotIds) slots[slotId] = { taskId: task.taskId, responseGroupId: response.responseGroupId, slot: source.answerSlots[slotId], options };
    }
    taskGroups.push({ taskId: task.taskId, taskType: task.taskType, responseGroupIds });
  }
  return { schemaVersion: "ListeningInteractionModelV1", examId: source.examId, sourceRevision: sourceRevision(source), taskGroups, responseGroups, slots };
}

function slotValueIssues(slot: AnswerSlotV2, group: ListeningRuntimeResponseGroupV1, value: AnswerValueV2 | undefined, mode: ListeningAttemptValidationModeV1): ListeningRuntimeIssueV1[] {
  if (isUnanswered(value)) return [];
  const issues: ListeningRuntimeIssueV1[] = [];
  if (slot.interaction === "text" && value?.kind !== "text") return [issue("RUNTIME_ANSWER_KIND_MISMATCH", slot.slotId, "Text slots accept only text answers.")];
  if (slot.interaction !== "text" && value?.kind !== "option") return [issue("RUNTIME_ANSWER_KIND_MISMATCH", slot.slotId, "Choice, matching and hotspot slots accept option answers.")];
  if (value?.kind === "option") {
    const labels = value.labels.map(normalizeOptionLabel);
    const allowed = slot.constraints?.acceptedOptionLabels?.length ? new Set(slot.constraints.acceptedOptionLabels.map(normalizeOptionLabel)) : new Set(group.options.map((option) => normalizeOptionLabel(option.label)));
    if (labels.some((label) => !label || (allowed.size > 0 && !allowed.has(label)))) issues.push(issue("RUNTIME_OPTION_INVALID", slot.slotId, "Answer contains an option outside the keyboard option list."));
    if (new Set(labels).size !== labels.length || ((slot.interaction === "radio" || slot.interaction === "select" || slot.interaction === "hotspot") && labels.length !== 1)) issues.push(issue("RUNTIME_DUPLICATE_SELECTION", slot.slotId, "Single-select and hotspot slots require one unique option."));
  }
  if (value?.kind === "text") {
    const values = value.values.map((entry) => entry.trim());
    if (mode === "submit" && (values.length === 0 || values.some((entry) => !entry))) issues.push(issue("RUNTIME_ANSWER_REQUIRED", slot.slotId, "A text slot must contain a non-empty answer before submit."));
    for (const text of values) {
      const wordCount = text ? text.split(/\s+/u).filter(Boolean).length : 0;
      const numberCount = text.match(/\d+/gu)?.length ?? 0;
      if (slot.constraints?.maxWords !== undefined && wordCount > slot.constraints.maxWords) issues.push(issue("RUNTIME_WORD_LIMIT_EXCEEDED", slot.slotId, "Answer exceeds the word limit."));
      if (slot.constraints?.maxNumbers !== undefined && numberCount > slot.constraints.maxNumbers) issues.push(issue("RUNTIME_NUMBER_LIMIT_EXCEEDED", slot.slotId, "Answer exceeds the number limit."));
      if (slot.constraints?.maxCharacters !== undefined && [...text].length > slot.constraints.maxCharacters) issues.push(issue("RUNTIME_CHARACTER_LIMIT_EXCEEDED", slot.slotId, "Answer exceeds the character limit."));
    }
  }
  return issues;
}

export function validateListeningAttemptV1(source: ListeningExamSourceV1, attempt: ListeningAttemptV1, mode: ListeningAttemptValidationModeV1 = attempt.state === "submitted" ? "submit" : "draft"): ListeningRuntimeIssueV1[] {
  const issues: ListeningRuntimeIssueV1[] = [];
  try { assertListeningExamSourceV1(source); } catch (error) { return [issue(error instanceof ListeningRuntimeErrorV1 ? error.code : "LISTENING_SOURCE_INVALID", "source", error instanceof Error ? error.message : String(error))]; }
  const model = buildListeningRuntimeInteractionModel(source);
  if (attempt.schemaVersion !== "ListeningAttemptV1") issues.push(issue("RUNTIME_ATTEMPT_SCHEMA_UNSUPPORTED", "attempt", "Unsupported Listening attempt schema."));
  if (attempt.examId !== source.examId) issues.push(issue("RUNTIME_ATTEMPT_EXAM_MISMATCH", "attempt", "Attempt examId does not match source."));
  if (attempt.sourceRevision !== sourceRevision(source)) issues.push(issue("RUNTIME_ATTEMPT_REVISION_MISMATCH", "attempt", "Attempt must use the same source revision."));
  if (attempt.playback.mediaAssetId !== source.media.assetId || attempt.playback.policyMode !== source.playbackPolicy.mode) issues.push(issue("AUDIO_ATTEMPT_BINDING_INVALID", "playback", "Attempt playback is not bound to source media and policy."));
  if (attempt.playback.positionMs < 0 || attempt.playback.positionMs > source.media.durationMs || (attempt.playback.status === "ready" && (attempt.playback.playsStarted !== 0 || attempt.playback.positionMs !== 0)) || (attempt.playback.status !== "ready" && attempt.playback.status !== "failed" && attempt.playback.playsStarted === 0)) issues.push(issue("AUDIO_PLAYBACK_STATE_INVALID", "playback", "Serialized playback snapshot violates the source duration or play-count contract."));
  if (source.playbackPolicy.maxPlays !== undefined && attempt.playback.playsStarted > source.playbackPolicy.maxPlays) issues.push(issue("AUDIO_POLICY_MISSING", "playback", "Attempt exceeds the configured play limit."));
  for (const [slotId, value] of Object.entries(attempt.answers)) {
    const runtimeSlot = model.slots[slotId];
    if (!runtimeSlot) { issues.push(issue("RUNTIME_UNKNOWN_SLOT", slotId, "Attempt contains an unknown slotId.")); continue; }
    issues.push(...slotValueIssues(runtimeSlot.slot, model.responseGroups[runtimeSlot.responseGroupId], value, mode));
  }
  for (const group of Object.values(model.responseGroups)) {
    const scoringSlotIds = group.slotIds.filter((slotId) => source.answerSlots[slotId]?.participation === "scoring");
    const answeredSlotIds = scoringSlotIds.filter((slotId) => !isUnanswered(attempt.answers[slotId]));
    if (mode === "submit") for (const slotId of scoringSlotIds) if (isUnanswered(attempt.answers[slotId])) issues.push(issue("RUNTIME_ANSWER_REQUIRED", slotId, "Every scoring slot is required before submit."));
    const labels = group.slotIds.flatMap((slotId) => answerLabels(attempt.answers[slotId]));
    const selected = group.assignment === "per_slot" ? answeredSlotIds.length : labels.length;
    if (group.duplicatePolicy === "reject_submission" && new Set(labels).size !== labels.length) issues.push(issue("RUNTIME_DUPLICATE_SELECTION", group.responseGroupId, "This response group rejects duplicate labels."));
    if (selected > group.cardinality.max || (group.cardinality.exact !== undefined && selected > group.cardinality.exact)) issues.push(issue("RUNTIME_CARDINALITY_EXCEEDED", group.responseGroupId, "Response group selection cardinality is exceeded."));
    if (mode === "submit" && (selected < group.cardinality.min || (group.cardinality.exact !== undefined && selected !== group.cardinality.exact))) issues.push(issue("RUNTIME_CARDINALITY_INVALID", group.responseGroupId, "Response group selection cardinality is incomplete."));
  }
  return issues;
}

function cloneAttempt(attempt: ListeningAttemptV1, now: Date): ListeningAttemptV1 {
  return { ...attempt, answers: { ...attempt.answers }, playback: { ...attempt.playback }, state: "in_progress", updatedAt: now.toISOString(), submittedAt: undefined };
}

function assertAttemptBoundary(source: ListeningExamSourceV1, attempt: ListeningAttemptV1): void {
  if (attempt.schemaVersion !== "ListeningAttemptV1") throw new ListeningRuntimeErrorV1("RUNTIME_ATTEMPT_SCHEMA_UNSUPPORTED", "Unsupported Listening attempt schema.", attempt.examId);
  if (attempt.examId !== source.examId) throw new ListeningRuntimeErrorV1("RUNTIME_ATTEMPT_EXAM_MISMATCH", "Attempt examId does not match source.", attempt.examId);
  if (attempt.sourceRevision !== sourceRevision(source)) throw new ListeningRuntimeErrorV1("RUNTIME_ATTEMPT_REVISION_MISMATCH", "Attempt must use the same source revision.", attempt.examId);
}

export function createListeningAttempt(source: ListeningExamSourceV1, now = new Date(), playback?: ListeningPlaybackSnapshotV1): ListeningAttemptV1 {
  assertListeningExamSourceV1(source);
  return { schemaVersion: "ListeningAttemptV1", examId: source.examId, sourceRevision: sourceRevision(source), answers: {}, playback: playback ?? { mediaAssetId: source.media.assetId, policyMode: source.playbackPolicy.mode, playsStarted: 0, positionMs: 0, status: "ready", lastTransitionAt: now.toISOString() }, state: "in_progress", updatedAt: now.toISOString() };
}

export function setListeningSlotAnswer(source: ListeningExamSourceV1, attempt: ListeningAttemptV1, slotId: string, value: AnswerValueV2, now = new Date()): ListeningAttemptV1 {
  if (attempt.state === "submitted") throw new ListeningRuntimeErrorV1("RUNTIME_ATTEMPT_FINALIZED", "Submitted attempts cannot be edited.", attempt.examId);
  assertAttemptBoundary(source, attempt);
  const model = buildListeningRuntimeInteractionModel(source);
  const runtimeSlot = model.slots[slotId];
  if (!runtimeSlot) throw new ListeningRuntimeErrorV1("RUNTIME_UNKNOWN_SLOT", "Cannot answer an unknown slotId.", slotId);
  const issues = slotValueIssues(runtimeSlot.slot, model.responseGroups[runtimeSlot.responseGroupId], value, "draft");
  if (issues.length) throwFirstIssue(issues);
  const next = cloneAttempt(attempt, now);
  next.answers[slotId] = value;
  const groupIssues = validateListeningAttemptV1(source, next, "draft").filter((entry) => entry.targetId === slotId || entry.targetId === runtimeSlot.responseGroupId);
  if (groupIssues.length) throwFirstIssue(groupIssues);
  return next;
}

export function clearListeningSlotAnswer(source: ListeningExamSourceV1, attempt: ListeningAttemptV1, slotId: string, now = new Date()): ListeningAttemptV1 {
  if (attempt.state === "submitted") throw new ListeningRuntimeErrorV1("RUNTIME_ATTEMPT_FINALIZED", "Submitted attempts cannot be edited.", attempt.examId);
  assertAttemptBoundary(source, attempt);
  if (!buildListeningRuntimeInteractionModel(source).slots[slotId]) throw new ListeningRuntimeErrorV1("RUNTIME_UNKNOWN_SLOT", "Cannot clear an unknown slotId.", slotId);
  const next = cloneAttempt(attempt, now);
  delete next.answers[slotId];
  return next;
}

export function submitListeningAttempt(source: ListeningExamSourceV1, attempt: ListeningAttemptV1, now = new Date()): ListeningAttemptV1 {
  assertAttemptBoundary(source, attempt);
  const next: ListeningAttemptV1 = { ...attempt, state: "submitted", updatedAt: now.toISOString(), submittedAt: now.toISOString() };
  const issues = validateListeningAttemptV1(source, next, "submit");
  if (issues.length) throwFirstIssue(issues);
  return next;
}

function answersEqual(actual: AnswerValueV2 | undefined, expected: AnswerValueV2 | undefined): boolean {
  if (!actual || !expected || actual.kind !== expected.kind) return false;
  if (actual.kind === "text" && expected.kind === "text") {
    const exact = expected.normalization === "exact";
    return actual.values.some((value) => expected.values.some((candidate) => normalizeText(value, exact) === normalizeText(candidate, exact)));
  }
  if (actual.kind === "option" && expected.kind === "option") {
    const left = actual.labels.map(normalizeOptionLabel);
    const right = expected.labels.map(normalizeOptionLabel);
    return actual.assignment === "ordered" || expected.assignment === "ordered" ? left.join("\u0000") === right.join("\u0000") : left.length === right.length && left.every((label) => right.includes(label));
  }
  return false;
}

export function scoreListeningAttempt(source: ListeningExamSourceV1, attempt: ListeningAttemptV1): ListeningAttemptScoreV1 {
  const issues = validateListeningAttemptV1(source, attempt, "submit");
  if (issues.length) throwFirstIssue(issues);
  const model = buildListeningRuntimeInteractionModel(source);
  const slotScores: Record<string, number> = {};
  const responseGroups: ListeningAttemptScoreV1["responseGroups"] = {};
  let earnedPoints = 0;
  let possiblePoints = 0;
  for (const group of Object.values(model.responseGroups)) {
    const scoringSlots = group.slotIds.filter((slotId) => source.answerSlots[slotId]?.participation === "scoring");
    const scores: Record<string, number> = {};
    for (const slotId of scoringSlots) scores[slotId] = Number(answersEqual(attempt.answers[slotId], source.answerKey[slotId]));
    if (group.assignment === "unordered_set") {
      const expected = scoringSlots.flatMap((slotId) => answerLabels(source.answerKey[slotId]));
      const submitted = scoringSlots.flatMap((slotId) => answerLabels(attempt.answers[slotId]));
      const exact = expected.length === submitted.length && new Set(expected).size === expected.length && new Set(submitted).size === submitted.length && expected.every((label) => submitted.includes(label));
      if (exact) for (const slotId of scoringSlots) scores[slotId] = 1;
      if (!exact && (group.scoringPolicy === "exact_set" || group.scoringPolicy === "all_or_nothing")) for (const slotId of scoringSlots) scores[slotId] = 0;
    }
    Object.assign(slotScores, scores);
    const earned = Object.values(scores).reduce((sum, value) => sum + value, 0);
    const possible = scoringSlots.length;
    responseGroups[group.responseGroupId] = { correct: earned === possible, earnedPoints: earned, possiblePoints: possible };
    earnedPoints += earned;
    possiblePoints += possible;
  }
  return { correct: earnedPoints === possiblePoints, earnedPoints, possiblePoints, slotScores, responseGroups };
}

export function resolveListeningAssetDescriptor(manifest: { schemaVersion: string; examId: string; assets: Record<string, AssetDescriptorV2> }, examId: string, assetId: string): AssetDescriptorV2 {
  if (manifest.schemaVersion !== "ExamAssetManifestV2" || manifest.examId !== examId) throw new ListeningRuntimeErrorV1("ASSET_MANIFEST_MISMATCH", "Listening asset manifest does not belong to this exam.", examId);
  const descriptor = manifest.assets[assetId];
  if (!descriptor) throw new ListeningRuntimeErrorV1("ASSET_MISSING", "Requested Listening asset is missing.", assetId);
  if (!descriptor.mime.startsWith("audio/") || !/^[a-f0-9]{64}$/iu.test(descriptor.sha256) || descriptor.byteLength < 0) throw new ListeningRuntimeErrorV1("ASSET_DESCRIPTOR_INVALID", "Listening asset descriptor failed MIME/hash/size policy.", assetId);
  if (!descriptor.relativePath || descriptor.relativePath.includes("\\") || descriptor.relativePath.includes(":") || descriptor.relativePath.startsWith("/") || descriptor.relativePath.split("/").some((part) => !part || part === "." || part === "..")) throw new ListeningRuntimeErrorV1("ASSET_PATH_UNSAFE", "Listening asset path must be a relative POSIX path.", descriptor.relativePath);
  return descriptor;
}

export async function resolveListeningAsset(
  manifest: { schemaVersion: string; examId: string; assets: Record<string, AssetDescriptorV2> },
  examId: string,
  assetId: string,
  provider: (descriptor: AssetDescriptorV2) => ListeningAssetResolutionV1 | Promise<ListeningAssetResolutionV1>
): Promise<ListeningAssetResolutionV1> {
  const descriptor = resolveListeningAssetDescriptor(manifest, examId, assetId);
  const resolved = await provider(descriptor);
  if (resolved.assetId !== assetId || resolved.mime !== descriptor.mime || resolved.byteLength !== descriptor.byteLength || resolved.sha256.toLowerCase() !== descriptor.sha256.toLowerCase()) throw new ListeningRuntimeErrorV1("ASSET_RESOLUTION_INTEGRITY_FAILED", "Listening provider returned metadata different from the manifest.", assetId);
  if (/^(?:https?:|file:)/iu.test(resolved.resourceUri)) throw new ListeningRuntimeErrorV1("ASSET_URI_UNSAFE", "Listening assets must use a controlled local resource URI.", assetId);
  return resolved;
}
