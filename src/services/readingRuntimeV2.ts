import type {
  AnswerSlotV2,
  AnswerValueV2,
  IeltsAuthoringIRV2,
  OptionV2,
  ResponseGroupV2,
  TaskGroupV2
} from "../types/ielts-authoring-v2";
import type { AssetDescriptorV2 } from "../types/schema-common-v2";
import {
  READING_ATTEMPT_V2_SCHEMA_VERSION,
  READING_EXAM_SOURCE_V2_SCHEMA_VERSION,
  type AssetResolutionV2,
  type ExamAssetManifestV2,
  type NormalizedReadingSourceV2,
  type ReadingAttemptScoreV2,
  type ReadingAttemptV2,
  type LegacyReadingExamSourceV1,
  type ReadingExamSourceV2,
  type ReadingRuntimeInteractionModelV2,
  type ReadingRuntimeIssueV2,
  type ReadingRuntimeOptionV2,
  type ReadingRuntimeResponseGroupV2,
  type ReadingRuntimeSlotV2,
  ReadingRuntimeError
} from "../types/reading-runtime-v2";

export type AttemptValidationModeV2 = "draft" | "submit";

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function issue(code: string, targetId: string, message: string): ReadingRuntimeIssueV2 {
  return { code, targetId, message };
}

function throwFirstIssue(issues: ReadingRuntimeIssueV2[]): never {
  const first = issues[0] ?? issue("RUNTIME_INVALID", "runtime", "Reading runtime input is invalid.");
  throw new ReadingRuntimeError(first.code, first.message, first.targetId);
}

function normalizeOptionLabel(label: string): string {
  return label.trim().toLocaleUpperCase("en-US");
}

function normalizeText(value: string, exact: boolean): string {
  const compact = value.trim().replace(/\s+/gu, " ");
  return exact ? compact : compact.toLocaleLowerCase("en-US");
}

function answerLabels(value: AnswerValueV2 | undefined): string[] {
  if (!value || value.kind !== "option") return [];
  return value.labels.map(normalizeOptionLabel);
}

function answerTexts(value: AnswerValueV2 | undefined): string[] {
  if (!value || value.kind !== "text") return [];
  return value.values.map((entry) => entry.trim());
}

function isUnanswered(value: AnswerValueV2 | undefined): boolean {
  if (!value || value.kind === "unresolved") return true;
  if (value.kind === "option") return value.labels.length === 0;
  return value.values.length === 0 || value.values.every((entry) => !entry.trim());
}

function optionSet(options: OptionV2[]): Set<string> {
  return new Set(options.map((option) => normalizeOptionLabel(option.label)));
}

function responseOptions(task: TaskGroupV2, response: ResponseGroupV2): OptionV2[] {
  if (response.options?.length) return response.options;
  if (response.optionBankRef && task.optionBank?.optionBankId === response.optionBankRef) {
    return task.optionBank.options;
  }
  return [];
}

function sourceRevision(source: ReadingExamSourceV2): number {
  return Number.isInteger(source.audit.sourceRevision) && source.audit.sourceRevision >= 0
    ? source.audit.sourceRevision
    : 0;
}

/**
 * Perform the cross-field checks that JSON Schema cannot express. This is the
 * boundary used by both the preview and a future student loader.
 */
export function validateReadingExamSourceV2(source: ReadingExamSourceV2): ReadingRuntimeIssueV2[] {
  const issues: ReadingRuntimeIssueV2[] = [];
  if (source.schemaVersion !== READING_EXAM_SOURCE_V2_SCHEMA_VERSION) {
    issues.push(issue("RUNTIME_SCHEMA_UNSUPPORTED", "schemaVersion", "Only ReadingExamSourceV2 is accepted."));
    return issues;
  }
  if (!source.examId || source.assets.examId !== source.examId) {
    issues.push(issue("RUNTIME_EXAM_ID_MISMATCH", source.examId || "runtime", "Runtime and asset manifest examId must match."));
  }
  const slotIds = new Set(Object.keys(source.answerSlots));
  const orderedIds = new Set(source.questionOrder);
  if (orderedIds.size !== source.questionOrder.length || orderedIds.size !== slotIds.size || [...orderedIds].some((id) => !slotIds.has(id))) {
    issues.push(issue("RUNTIME_QUESTION_ORDER_INVALID", source.examId, "questionOrder must contain each answer slot exactly once."));
  }
  const displayIds = new Set(Object.keys(source.questionDisplayMap));
  if (displayIds.size !== slotIds.size || [...displayIds].some((id) => !slotIds.has(id))) {
    issues.push(issue("RUNTIME_DISPLAY_MAP_MISMATCH", source.examId, "questionDisplayMap must contain exactly every answer slot."));
  }
  const answerIds = new Set(Object.keys(source.answerKey));
  if (answerIds.size !== slotIds.size || [...answerIds].some((id) => !slotIds.has(id))) {
    issues.push(issue("RUNTIME_ANSWER_KEY_MISMATCH", source.examId, "answerKey must contain exactly every answer slot."));
  }
  const taskIds = new Set<string>();
  const responseIds = new Set<string>();
  const assigned = new Set<string>();
  for (const task of source.taskGroups) {
    if (taskIds.has(task.taskId)) issues.push(issue("RUNTIME_TASK_ID_DUPLICATE", task.taskId, "taskId must be globally unique."));
    taskIds.add(task.taskId);
    for (const response of task.responseGroups) {
      if (responseIds.has(response.responseGroupId)) issues.push(issue("RUNTIME_RESPONSE_ID_DUPLICATE", response.responseGroupId, "responseGroupId must be globally unique."));
      responseIds.add(response.responseGroupId);
      const options = responseOptions(task, response);
      if ((response.kind === "choice" || response.kind === "matching") && options.length === 0) {
        issues.push(issue("RUNTIME_OPTION_BANK_MISSING", response.responseGroupId, "Choice and matching responses require a resolvable option source."));
      }
      const labels = options.map((option) => normalizeOptionLabel(option.label));
      if (new Set(labels).size !== labels.length || labels.some((label) => !label)) {
        issues.push(issue("RUNTIME_OPTION_LABELS_INVALID", response.responseGroupId, "Runtime option labels must be non-empty and unique."));
      }
      for (const slotId of response.slotIds) {
        if (!slotIds.has(slotId)) issues.push(issue("RUNTIME_RESPONSE_SLOT_MISSING", slotId, "Response group references an unknown answer slot."));
        if (assigned.has(slotId)) issues.push(issue("RUNTIME_SLOT_ASSIGNED_TWICE", slotId, "An answer slot may belong to only one response group."));
        assigned.add(slotId);
      }
    }
  }
  for (const slotId of slotIds) {
    if (!assigned.has(slotId)) issues.push(issue("RUNTIME_SLOT_UNASSIGNED", slotId, "Every answer slot must belong to one response group."));
    const slot = source.answerSlots[slotId];
    if (slot.slotId !== slotId || source.questionDisplayMap[slotId] !== slot.displayLabel) {
      issues.push(issue("RUNTIME_SLOT_ID_MISMATCH", slotId, "Slot key, slotId and display map must agree."));
    }
  }
  return issues;
}

export function assertReadingExamSourceV2(source: ReadingExamSourceV2): void {
  const issues = validateReadingExamSourceV2(source);
  if (issues.length) throwFirstIssue(issues);
}

/**
 * Keep the legacy payload opaque. V1 continues through its existing renderer;
 * it is never reparsed into V2 slots as a best-effort guess.
 */
export function normalizeReadingSource(raw: unknown): NormalizedReadingSourceV2 {
  if (!isRecord(raw)) throw new ReadingRuntimeError("RUNTIME_SOURCE_INVALID", "Reading source must be an object.");
  if (raw.schemaVersion === READING_EXAM_SOURCE_V2_SCHEMA_VERSION) {
    const source = raw as unknown as ReadingExamSourceV2;
    assertReadingExamSourceV2(source);
    return { version: "v2", source };
  }
  const schemaVersion = typeof raw.schemaVersion === "string" ? raw.schemaVersion : undefined;
  if (!schemaVersion || schemaVersion === "ReadingExamSourceV1" || schemaVersion === "ReadingPracticePayloadV1" || "groups" in raw || "questionGroups" in raw) {
    return { version: "v1", source: raw as LegacyReadingExamSourceV1 };
  }
  throw new ReadingRuntimeError("RUNTIME_SCHEMA_UNSUPPORTED", `Unsupported reading source schema: ${schemaVersion}.`);
}

export function buildReadingRuntimeInteractionModel(source: ReadingExamSourceV2): ReadingRuntimeInteractionModelV2 {
  assertReadingExamSourceV2(source);
  const responseGroups: Record<string, ReadingRuntimeResponseGroupV2> = {};
  const slots: Record<string, ReadingRuntimeSlotV2> = {};
  const taskGroups: ReadingRuntimeInteractionModelV2["taskGroups"] = [];
  for (const task of source.taskGroups) {
    const responseGroupIds: string[] = [];
    for (const response of task.responseGroups) {
      const options = responseOptions(task, response);
      const runtimeResponse: ReadingRuntimeResponseGroupV2 = {
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
      responseGroups[response.responseGroupId] = runtimeResponse;
      responseGroupIds.push(response.responseGroupId);
      for (const slotId of response.slotIds) {
        slots[slotId] = {
          taskId: task.taskId,
          responseGroupId: response.responseGroupId,
          slot: source.answerSlots[slotId],
          options
        };
      }
    }
    taskGroups.push({ taskId: task.taskId, taskType: task.taskType, responseGroupIds });
  }
  return {
    schemaVersion: "ReadingInteractionModelV2",
    examId: source.examId,
    sourceRevision: sourceRevision(source),
    taskGroups,
    responseGroups,
    slots
  };
}

export function createReadingAttempt(source: ReadingExamSourceV2, now = new Date()): ReadingAttemptV2 {
  assertReadingExamSourceV2(source);
  return {
    schemaVersion: READING_ATTEMPT_V2_SCHEMA_VERSION,
    examId: source.examId,
    sourceRevision: sourceRevision(source),
    answers: {},
    state: "in_progress",
    updatedAt: now.toISOString()
  };
}

function slotValueIssues(
  slot: AnswerSlotV2,
  group: ReadingRuntimeResponseGroupV2,
  value: AnswerValueV2 | undefined,
  mode: AttemptValidationModeV2
): ReadingRuntimeIssueV2[] {
  if (isUnanswered(value)) return [];
  const issues: ReadingRuntimeIssueV2[] = [];
  if (slot.interaction === "text" && value?.kind !== "text") {
    issues.push(issue("RUNTIME_ANSWER_KIND_MISMATCH", slot.slotId, "Text slots accept only text answers."));
    return issues;
  }
  if (slot.interaction !== "text" && value?.kind !== "option") {
    issues.push(issue("RUNTIME_ANSWER_KIND_MISMATCH", slot.slotId, "Choice, matching and hotspot slots accept option answers."));
    return issues;
  }
  if (value?.kind === "option") {
    const labels = value.labels.map(normalizeOptionLabel);
    const allowed = slot.constraints?.acceptedOptionLabels?.length
      ? new Set(slot.constraints.acceptedOptionLabels.map(normalizeOptionLabel))
      : optionSet(group.options);
    if (labels.some((label) => !label || (allowed.size > 0 && !allowed.has(label)))) {
      issues.push(issue("RUNTIME_OPTION_INVALID", slot.slotId, "Answer contains an option that is not in the resolved option bank."));
    }
    if (new Set(labels).size !== labels.length || (slot.interaction === "radio" || slot.interaction === "select") && labels.length !== 1) {
      issues.push(issue("RUNTIME_DUPLICATE_SELECTION", slot.slotId, "Radio/select slots require exactly one unique option."));
    }
    if (slot.interaction === "checkbox" && labels.length === 0 && mode === "submit") {
      issues.push(issue("RUNTIME_ANSWER_REQUIRED", slot.slotId, "A checkbox slot must have a selection before submit."));
    }
  }
  if (value?.kind === "text") {
    const values = value.values.map((entry) => entry.trim());
    if (values.some((entry) => !entry) && mode === "submit") {
      issues.push(issue("RUNTIME_ANSWER_REQUIRED", slot.slotId, "A text slot must contain a non-empty answer before submit."));
    }
    const constraints = slot.constraints;
    for (const text of values) {
      const wordCount = text ? text.split(/\s+/u).filter(Boolean).length : 0;
      const numberCount = text.match(/\d+/gu)?.length ?? 0;
      if (constraints?.maxWords !== undefined && wordCount > constraints.maxWords) {
        issues.push(issue("RUNTIME_WORD_LIMIT_EXCEEDED", slot.slotId, `Answer exceeds the ${constraints.maxWords}-word limit.`));
      }
      if (constraints?.maxNumbers !== undefined && numberCount > constraints.maxNumbers) {
        issues.push(issue("RUNTIME_NUMBER_LIMIT_EXCEEDED", slot.slotId, `Answer exceeds the ${constraints.maxNumbers}-number limit.`));
      }
      if (constraints?.maxCharacters !== undefined && [...text].length > constraints.maxCharacters) {
        issues.push(issue("RUNTIME_CHARACTER_LIMIT_EXCEEDED", slot.slotId, `Answer exceeds the ${constraints.maxCharacters}-character limit.`));
      }
    }
  }
  return issues;
}

function selectedCount(value: AnswerValueV2 | undefined): number {
  if (!value || value.kind === "unresolved") return 0;
  if (value.kind === "option") return value.labels.length;
  return value.values.some((entry) => entry.trim()) ? 1 : 0;
}

export function validateReadingAttempt(
  source: ReadingExamSourceV2,
  attempt: ReadingAttemptV2,
  mode: AttemptValidationModeV2 = attempt.state === "submitted" ? "submit" : "draft"
): ReadingRuntimeIssueV2[] {
  try {
    assertReadingExamSourceV2(source);
  } catch (error) {
    return [issue(error instanceof ReadingRuntimeError ? error.code : "RUNTIME_SOURCE_INVALID", "source", error instanceof Error ? error.message : String(error))];
  }
  const model = buildReadingRuntimeInteractionModel(source);
  const issues: ReadingRuntimeIssueV2[] = [];
  if (attempt.schemaVersion !== READING_ATTEMPT_V2_SCHEMA_VERSION) issues.push(issue("RUNTIME_ATTEMPT_SCHEMA_UNSUPPORTED", "attempt", "Unsupported attempt schema version."));
  if (attempt.examId !== source.examId) issues.push(issue("RUNTIME_ATTEMPT_EXAM_MISMATCH", "attempt", "Attempt examId does not match the runtime source."));
  if (attempt.sourceRevision !== sourceRevision(source)) issues.push(issue("RUNTIME_ATTEMPT_REVISION_MISMATCH", "attempt", "Attempt must be resumed against the same source revision."));
  for (const slotId of Object.keys(attempt.answers)) {
    const runtimeSlot = model.slots[slotId];
    if (!runtimeSlot) {
      issues.push(issue("RUNTIME_UNKNOWN_SLOT", slotId, "Attempt contains an answer for an unknown slotId."));
      continue;
    }
    const group = model.responseGroups[runtimeSlot.responseGroupId];
    issues.push(...slotValueIssues(runtimeSlot.slot, group, attempt.answers[slotId], mode));
  }
  for (const group of Object.values(model.responseGroups)) {
    const scoringSlotIds = group.slotIds.filter((slotId) => source.answerSlots[slotId]?.participation === "scoring");
    const answeredSlotIds = scoringSlotIds.filter((slotId) => !isUnanswered(attempt.answers[slotId]));
    if (mode === "submit") {
      for (const slotId of scoringSlotIds) {
        if (isUnanswered(attempt.answers[slotId])) issues.push(issue("RUNTIME_ANSWER_REQUIRED", slotId, "Every scoring answer slot is required before submit."));
      }
    }
    const selectedLabels = group.slotIds.flatMap((slotId) => answerLabels(attempt.answers[slotId]));
    const selected = group.assignment === "per_slot" ? answeredSlotIds.length : selectedLabels.length;
    const exact = group.cardinality.exact;
    if (group.duplicatePolicy === "reject_submission" && !group.allowOptionReuse && new Set(selectedLabels).size !== selectedLabels.length) {
      issues.push(issue("RUNTIME_DUPLICATE_SELECTION", group.responseGroupId, "This response group rejects duplicate option selections."));
    }
    if (exact !== undefined && selected > exact) issues.push(issue("RUNTIME_CARDINALITY_EXCEEDED", group.responseGroupId, `Response group accepts exactly ${exact} selections.`));
    if (selected > group.cardinality.max) issues.push(issue("RUNTIME_CARDINALITY_EXCEEDED", group.responseGroupId, `Response group accepts at most ${group.cardinality.max} selections.`));
    if (mode === "submit" && selected < group.cardinality.min) issues.push(issue("RUNTIME_CARDINALITY_UNDERFLOW", group.responseGroupId, `Response group requires at least ${group.cardinality.min} selections.`));
    if (mode === "submit" && exact !== undefined && selected !== exact) issues.push(issue("RUNTIME_CARDINALITY_INVALID", group.responseGroupId, `Response group requires exactly ${exact} selections.`));
  }
  return issues;
}

function cloneAttempt(attempt: ReadingAttemptV2, now: Date): ReadingAttemptV2 {
  return { ...attempt, answers: { ...attempt.answers }, updatedAt: now.toISOString(), state: "in_progress", submittedAt: undefined };
}

function assertAttemptResumeBoundary(source: ReadingExamSourceV2, attempt: ReadingAttemptV2): void {
  if (attempt.schemaVersion !== READING_ATTEMPT_V2_SCHEMA_VERSION) {
    throw new ReadingRuntimeError("RUNTIME_ATTEMPT_SCHEMA_UNSUPPORTED", "Unsupported attempt schema version.", attempt.examId);
  }
  if (attempt.examId !== source.examId) {
    throw new ReadingRuntimeError("RUNTIME_ATTEMPT_EXAM_MISMATCH", "Attempt examId does not match the runtime source.", attempt.examId);
  }
  if (attempt.sourceRevision !== sourceRevision(source)) {
    throw new ReadingRuntimeError("RUNTIME_ATTEMPT_REVISION_MISMATCH", "Attempt must be resumed against the same source revision.", attempt.examId);
  }
}

export function setReadingSlotAnswer(
  source: ReadingExamSourceV2,
  attempt: ReadingAttemptV2,
  slotId: string,
  value: AnswerValueV2,
  now = new Date()
): ReadingAttemptV2 {
  if (attempt.state === "submitted") throw new ReadingRuntimeError("RUNTIME_ATTEMPT_FINALIZED", "Submitted attempts cannot be edited.", attempt.examId);
  assertAttemptResumeBoundary(source, attempt);
  const model = buildReadingRuntimeInteractionModel(source);
  const runtimeSlot = model.slots[slotId];
  if (!runtimeSlot) throw new ReadingRuntimeError("RUNTIME_UNKNOWN_SLOT", "Cannot answer an unknown slotId.", slotId);
  const issues = slotValueIssues(runtimeSlot.slot, model.responseGroups[runtimeSlot.responseGroupId], value, "draft");
  if (issues.length) throwFirstIssue(issues);
  const next = cloneAttempt(attempt, now);
  next.answers[slotId] = value;
  const group = model.responseGroups[runtimeSlot.responseGroupId];
  const groupIssues = validateReadingAttempt(source, next, "draft").filter((entry) => entry.targetId === group.responseGroupId || entry.targetId === slotId);
  if (groupIssues.length) throwFirstIssue(groupIssues);
  return next;
}

export function clearReadingSlotAnswer(source: ReadingExamSourceV2, attempt: ReadingAttemptV2, slotId: string, now = new Date()): ReadingAttemptV2 {
  if (attempt.state === "submitted") throw new ReadingRuntimeError("RUNTIME_ATTEMPT_FINALIZED", "Submitted attempts cannot be edited.", attempt.examId);
  assertAttemptResumeBoundary(source, attempt);
  const model = buildReadingRuntimeInteractionModel(source);
  if (!model.slots[slotId]) throw new ReadingRuntimeError("RUNTIME_UNKNOWN_SLOT", "Cannot clear an unknown slotId.", slotId);
  const next = cloneAttempt(attempt, now);
  delete next.answers[slotId];
  return next;
}

export function submitReadingAttempt(source: ReadingExamSourceV2, attempt: ReadingAttemptV2, now = new Date()): ReadingAttemptV2 {
  const next: ReadingAttemptV2 = { ...attempt, state: "submitted", updatedAt: now.toISOString(), submittedAt: now.toISOString() };
  const issues = validateReadingAttempt(source, next, "submit");
  if (issues.length) throwFirstIssue(issues);
  return next;
}

function optionAnswersEqual(actual: AnswerValueV2 | undefined, expected: AnswerValueV2 | undefined): boolean {
  if (!actual || !expected || actual.kind !== "option" || expected.kind !== "option") return false;
  const actualLabels = actual.labels.map(normalizeOptionLabel);
  const expectedLabels = expected.labels.map(normalizeOptionLabel);
  if (actual.assignment === "ordered" || expected.assignment === "ordered") return actualLabels.join("\u0000") === expectedLabels.join("\u0000");
  return actualLabels.length === expectedLabels.length && actualLabels.every((label) => expectedLabels.includes(label)) && expectedLabels.every((label) => actualLabels.includes(label));
}

function textAnswersEqual(actual: AnswerValueV2 | undefined, expected: AnswerValueV2 | undefined): boolean {
  if (!actual || !expected || actual.kind !== "text" || expected.kind !== "text") return false;
  const exact = expected.normalization === "exact";
  const actualValues = actual.values.map((value) => normalizeText(value, exact));
  const expectedValues = expected.values.map((value) => normalizeText(value, exact));
  return actualValues.some((value) => expectedValues.includes(value));
}

function groupScore(
  source: ReadingExamSourceV2,
  model: ReadingRuntimeInteractionModelV2,
  group: ReadingRuntimeResponseGroupV2,
  answers: Record<string, AnswerValueV2>
): { correct: boolean; earnedPoints: number; possiblePoints: number; slotScores: Record<string, number> } {
  const scoringSlotIds = group.slotIds.filter((slotId) => source.answerSlots[slotId]?.participation === "scoring");
  const slotScores: Record<string, number> = {};
  for (const slotId of scoringSlotIds) slotScores[slotId] = 0;
  const possiblePoints = scoringSlotIds.length;
  if (group.assignment === "unordered_set") {
    const expected = scoringSlotIds.flatMap((slotId) => answerLabels(source.answerKey[slotId]));
    const submitted = scoringSlotIds.flatMap((slotId) => answerLabels(answers[slotId]));
    const expectedSet = new Set(expected);
    const submittedSet = new Set(submitted);
    const setCorrect = submitted.length === expected.length && submittedSet.size === submitted.length && expectedSet.size === expected.length && submittedSet.size === expectedSet.size && [...expectedSet].every((label) => submittedSet.has(label));
    if (setCorrect) for (const slotId of scoringSlotIds) slotScores[slotId] = 1;
    else {
      for (const slotId of scoringSlotIds) {
        const labels = answerLabels(answers[slotId]);
        slotScores[slotId] = labels.some((label) => expectedSet.has(label)) ? 1 : 0;
      }
    }
    if (group.scoringPolicy === "all_or_nothing" || group.scoringPolicy === "exact_set") {
      if (!setCorrect) for (const slotId of scoringSlotIds) slotScores[slotId] = 0;
    }
    return { correct: setCorrect, earnedPoints: Object.values(slotScores).reduce((sum, value) => sum + value, 0), possiblePoints, slotScores };
  }
  for (const slotId of scoringSlotIds) {
    const actual = answers[slotId];
    const expected = source.answerKey[slotId];
    slotScores[slotId] = expected?.kind === "text" ? Number(textAnswersEqual(actual, expected)) : Number(optionAnswersEqual(actual, expected));
  }
  const correct = Object.values(slotScores).every((value) => value === 1);
  if (group.scoringPolicy === "all_or_nothing" && !correct) for (const slotId of scoringSlotIds) slotScores[slotId] = 0;
  return { correct, earnedPoints: Object.values(slotScores).reduce((sum, value) => sum + value, 0), possiblePoints, slotScores };
}

export function scoreReadingAttempt(source: ReadingExamSourceV2, attempt: ReadingAttemptV2): ReadingAttemptScoreV2 {
  const issues = validateReadingAttempt(source, attempt, "submit");
  if (issues.length) throwFirstIssue(issues);
  const model = buildReadingRuntimeInteractionModel(source);
  const responseGroups: ReadingAttemptScoreV2["responseGroups"] = {};
  const slotScores: Record<string, number> = {};
  let earnedPoints = 0;
  let possiblePoints = 0;
  for (const group of Object.values(model.responseGroups)) {
    const score = groupScore(source, model, group, attempt.answers);
    responseGroups[group.responseGroupId] = { correct: score.correct, earnedPoints: score.earnedPoints, possiblePoints: score.possiblePoints };
    Object.assign(slotScores, score.slotScores);
    earnedPoints += score.earnedPoints;
    possiblePoints += score.possiblePoints;
  }
  return { correct: earnedPoints === possiblePoints, earnedPoints, possiblePoints, slotScores, responseGroups };
}

export function safeAssetRelativePath(relativePath: string): string {
  if (!relativePath || relativePath.includes("\0") || relativePath.includes("://") || relativePath.includes("\\")) {
    throw new ReadingRuntimeError("ASSET_PATH_UNSAFE", "Asset paths must be relative POSIX paths.", relativePath);
  }
  if (relativePath.startsWith("/") || relativePath.startsWith("//") || /^[A-Za-z]:/u.test(relativePath)) {
    throw new ReadingRuntimeError("ASSET_PATH_UNSAFE", "Absolute, UNC and drive-qualified asset paths are rejected.", relativePath);
  }
  if (["⁄", "∕", "╱", "⧸"].some((separator) => relativePath.includes(separator))) {
    throw new ReadingRuntimeError("ASSET_PATH_UNSAFE", "Unicode slash lookalikes are rejected in asset paths.", relativePath);
  }
  const segments = relativePath.split("/");
  if (segments.some((segment) => !segment || segment === "." || segment === "..")) {
    throw new ReadingRuntimeError("ASSET_PATH_UNSAFE", "Asset paths cannot contain empty, dot or parent components.", relativePath);
  }
  return segments.join("/");
}

function allowedAssetMime(mime: string): boolean {
  return mime.startsWith("image/") || mime.startsWith("audio/") || mime === "application/octet-stream";
}

export function resolveAssetDescriptor(manifest: ExamAssetManifestV2, examId: string, assetId: string): AssetDescriptorV2 {
  if (manifest.schemaVersion !== "ExamAssetManifestV2" || manifest.examId !== examId) {
    throw new ReadingRuntimeError("ASSET_MANIFEST_MISMATCH", "Asset manifest does not belong to this exam.", examId);
  }
  const descriptor = manifest.assets[assetId];
  if (!descriptor) throw new ReadingRuntimeError("ASSET_MISSING", "Requested asset is missing from the manifest.", assetId);
  safeAssetRelativePath(descriptor.relativePath);
  if (!/^[a-f0-9]{64}$/iu.test(descriptor.sha256) || descriptor.byteLength < 0 || !allowedAssetMime(descriptor.mime)) {
    throw new ReadingRuntimeError("ASSET_DESCRIPTOR_INVALID", "Asset descriptor failed hash, size or MIME policy.", assetId);
  }
  return descriptor;
}

export async function resolveAsset(
  manifest: ExamAssetManifestV2,
  examId: string,
  assetId: string,
  provider: (descriptor: AssetDescriptorV2) => AssetResolutionV2 | Promise<AssetResolutionV2>
): Promise<AssetResolutionV2> {
  const descriptor = resolveAssetDescriptor(manifest, examId, assetId);
  const resolved = await provider(descriptor);
  if (resolved.assetId !== assetId || resolved.mime !== descriptor.mime || resolved.byteLength !== descriptor.byteLength || resolved.sha256.toLowerCase() !== descriptor.sha256.toLowerCase()) {
    throw new ReadingRuntimeError("ASSET_RESOLUTION_INTEGRITY_FAILED", "Asset provider returned metadata different from the manifest.", assetId);
  }
  if (/^(?:https?:|file:)/iu.test(resolved.resourceUri)) {
    throw new ReadingRuntimeError("ASSET_URI_UNSAFE", "Runtime assets must use a controlled local resource URI.", assetId);
  }
  return resolved;
}

export function buildReadingSourceV2FromAuthoring(authoring: IeltsAuthoringIRV2): ReadingExamSourceV2 {
  const source: ReadingExamSourceV2 = {
    schemaVersion: READING_EXAM_SOURCE_V2_SCHEMA_VERSION,
    examId: authoring.exam.examId,
    meta: {
      title: authoring.exam.title,
      language: authoring.exam.language,
      category: authoring.exam.category
    },
    assets: { examId: authoring.exam.examId, assets: authoring.assets },
    passage: {
      content: authoring.passage?.content ?? [],
      paragraphMap: authoring.passage?.paragraphMap
    },
    taskGroups: authoring.taskGroups,
    answerSlots: authoring.answerSlots,
    answerKey: authoring.answerKey,
    questionOrder: Object.values(authoring.answerSlots)
      .sort((left, right) => left.questionNumber - right.questionNumber || left.slotId.localeCompare(right.slotId))
      .map((slot) => slot.slotId),
    questionDisplayMap: Object.values(authoring.answerSlots).reduce<Record<string, string>>((result, slot) => {
      result[slot.slotId] = slot.displayLabel;
      return result;
    }, {}),
    audit: {
      sourceSchemaVersion: authoring.schemaVersion,
      sourceDocumentId: authoring.sourceDocumentId,
      sourceRevision: authoring.audit.revision,
      sourceRevisionKind: authoring.audit.source
    }
  };
  assertReadingExamSourceV2(source);
  return source;
}
