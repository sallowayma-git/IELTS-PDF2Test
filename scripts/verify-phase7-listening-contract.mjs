import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import Ajv2020 from "ajv/dist/2020.js";
import ts from "typescript";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
// M0（续建计划 §2）：旧结论「peer 仓库在 ../NAS」指向已不存在的目录；真实学生端仓库
// 位于 ../IELTS-NASfor-WenDao（README: Electron + apps/student-exam + server）。
// 优先 NAS_PEER_REPO 环境变量，其次旧路径（兼容旧机器布局），最后真实路径。
const peerRoot = [process.env.NAS_PEER_REPO, join(repoRoot, "../NAS"), join(repoRoot, "../IELTS-NASfor-WenDao")]
  .filter(Boolean)
  .map((candidate) => resolve(candidate))
  .find((candidate) => existsSync(candidate));
const PEER_PREFIX = "../NAS/";

/** requiredFiles 数组里 `../NAS/...` 条目按 peer 根解析，其余按本仓根解析。 */
function requiredFilePath(file) {
  return file.startsWith(PEER_PREFIX)
    ? join(peerRoot ?? repoRoot, file.slice(PEER_PREFIX.length))
    : join(repoRoot, file);
}
const fixturePath = "fixtures/golden/synthetic/ielts/phase7-listening-part1-source-v1.json";
const audioProbeFixturePath = "fixtures/golden/synthetic/ielts/phase7-listening-audio-probe-result-v1.json";
const familiesFixturePath = "fixtures/golden/synthetic/ielts/phase7-listening-families-v1.json";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function readJson(relativePath) {
  return JSON.parse(readFileSync(join(repoRoot, relativePath), "utf8"));
}

function sha256(relativePath) {
  return createHash("sha256").update(readFileSync(join(repoRoot, relativePath))).digest("hex");
}

function buildAttempt(source) {
  return {
    schemaVersion: "ListeningAttemptV1",
    examId: source.examId,
    sourceRevision: source.audit.sourceRevision,
    answers: {},
    playback: {
      mediaAssetId: source.media.assetId,
      policyMode: source.playbackPolicy.mode,
      playsStarted: 0,
      positionMs: 0,
      status: "ready",
      lastTransitionAt: "2026-08-12T00:00:00Z"
    },
    state: "not_started",
    updatedAt: "2026-08-12T00:00:00Z"
  };
}

function buildListeningAuthoring(source, quality) {
  return {
    schemaVersion: "IeltsAuthoringIRV2",
    jobId: "phase7-listening-contract",
    exam: {
      examId: source.examId,
      title: source.meta.title,
      language: source.meta.language,
      tags: ["phase7", "synthetic"],
      sourceFiles: [
        { sourceFileId: "question-paper", role: "question_paper" },
        { sourceFileId: source.media.assetId, role: "audio" }
      ]
    },
    modality: "listening",
    listening: {
      scope: source.meta.scope,
      media: {
        assetId: source.media.assetId,
        mime: source.media.mime,
        durationMs: source.media.durationMs,
        ...(source.media.channels ? { channels: source.media.channels } : {}),
        ...(source.media.sampleRateHz ? { sampleRateHz: source.media.sampleRateHz } : {}),
        sha256: source.media.sha256
      },
      parts: source.parts,
      playbackPolicy: source.playbackPolicy,
      ...(source.transcript ? { transcript: source.transcript } : {})
    },
    taskGroups: source.taskGroups,
    answerSlots: source.answerSlots,
    answerKey: source.answerKey,
    assets: source.assets.assets,
    sourceDocumentId: source.audit.sourceDocumentId,
    quality,
    audit: {
      revision: source.audit.sourceRevision,
      source: source.audit.sourceRevisionKind,
      humanVerified: true,
      llmUsed: false,
      updatedAt: "2026-08-12T00:00:00Z",
      notes: ["Phase 7 schema contract fixture"]
    }
  };
}

function semanticIssueCodes(source) {
  const codes = new Set();
  const audio = source.assets.assets.find((asset) => asset.assetId === source.media.assetId);
  if (!audio) codes.add("ASSET_REFERENCE_MISSING");
  else {
    if (audio.kind !== "audio" || audio.mime !== source.media.mime) codes.add("AUDIO_CODEC_UNSUPPORTED");
    if (audio.sha256.toLowerCase() !== source.media.sha256.toLowerCase()) codes.add("AUDIO_HASH_MISMATCH");
    if (audio.durationMs !== undefined && audio.durationMs !== source.media.durationMs) codes.add("AUDIO_DECODE_FAILED");
  }
  if (source.media.probe.status !== "passed" || source.media.probe.issueCodes.length > 0) codes.add("AUDIO_DECODE_FAILED");
  if (
    (!source.playbackPolicy.allowReplay && source.playbackPolicy.maxPlays !== 1)
    || (source.playbackPolicy.allowReplay && source.playbackPolicy.maxPlays === 1)
  ) {
    codes.add("AUDIO_POLICY_MISSING");
  }
  if (
    source.playbackPolicy.mode === "mock"
    && (
      source.playbackPolicy.allowPause
      || source.playbackPolicy.allowSeek
      || source.playbackPolicy.allowReplay
      || source.playbackPolicy.maxPlays !== 1
      || source.playbackPolicy.refreshBehavior !== "resume_from_snapshot"
      || source.playbackPolicy.crashRecoveryBehavior !== "resume_from_snapshot"
    )
  ) {
    codes.add("AUDIO_POLICY_MISSING");
  }

  const taskIds = new Set(source.taskGroups.map((task) => task.taskId));
  const assignedTaskIds = new Set();
  const scoringNumbers = new Set(
    Object.values(source.answerSlots)
      .filter((slot) => slot.participation === "scoring")
      .map((slot) => slot.questionNumber)
  );
  const expectedNumbers = new Set();
  let priorCueEnd = -1;
  for (const part of source.parts) {
    for (const taskId of part.taskIds) {
      if (!taskIds.has(taskId) || assignedTaskIds.has(taskId)) codes.add("LISTENING_TASK_SCOPE_INVALID");
      assignedTaskIds.add(taskId);
    }
    for (const number of part.expectedQuestionNumbers) {
      if (expectedNumbers.has(number)) codes.add("QUESTION_NUMBER_DUPLICATE");
      expectedNumbers.add(number);
    }
    if (part.cue) {
      if (
        part.cue.startMs >= part.cue.endMs
        || part.cue.endMs > source.media.durationMs
        || part.cue.startMs < priorCueEnd
        || !part.cue.confirmed
        || part.cue.confidence < 0.9
      ) codes.add("AUDIO_CUE_INVALID");
      priorCueEnd = part.cue.endMs;
    }
  }
  for (const task of source.taskGroups) {
    for (const response of task.responseGroups ?? []) {
      const optionCount = response.options?.length
        ?? (response.optionBankRef && task.optionBank?.optionBankId === response.optionBankRef ? task.optionBank.options?.length : 0)
        ?? 0;
      if (["choice", "matching", "diagram_hotspot"].includes(response.kind) && !optionCount) {
        codes.add("LISTENING_KEYBOARD_ALTERNATIVE_MISSING");
      }
      if (response.kind === "diagram_hotspot" && !response.slotIds.every((slotId) => source.answerSlots[slotId]?.interaction === "hotspot")) {
        codes.add("LISTENING_HOTSPOT_INTERACTION_MISMATCH");
      }
    }
  }
  if (assignedTaskIds.size !== taskIds.size || [...taskIds].some((id) => !assignedTaskIds.has(id))) codes.add("LISTENING_TASK_SCOPE_INVALID");
  if (expectedNumbers.size !== scoringNumbers.size || [...scoringNumbers].some((number) => !expectedNumbers.has(number))) {
    codes.add("LISTENING_QUESTION_SCOPE_MISMATCH");
  }
  if (source.meta.scope === "complete_exam") {
    if (source.parts.length !== 4) codes.add("LISTENING_COMPLETE_PART_COUNT");
    if (source.parts.some((part) => part.expectedQuestionNumbers.length !== 10) || scoringNumbers.size !== 40) {
      codes.add("LISTENING_COMPLETE_QUESTION_COUNT");
    }
  }
  return [...codes].sort();
}

function sourceAnchor() {
  return {
    sourceFileId: "question-paper",
    pageIndex: 0,
    nodeIds: ["phase7-families"],
    extractionMode: "pdf_native",
    sourceHash: "a".repeat(64)
  };
}

function textNode(id, text) {
  return {
    type: "text",
    id,
    sourceAnchors: [sourceAnchor()],
    provenanceStatus: "source",
    text
  };
}

function familyOption(label, text) {
  return {
    optionId: `family-option-${label}`,
    label,
    content: [{
      type: "paragraph",
      id: `family-option-paragraph-${label}`,
      sourceAnchors: [sourceAnchor()],
      provenanceStatus: "source",
      children: [textNode(`family-option-text-${label}`, text)]
    }],
    sourceAnchors: [sourceAnchor()],
    provenanceStatus: "source"
  };
}

function buildFamiliesFixture(base, manifest) {
  const source = structuredClone(base);
  source.examId = "phase7-listening-families";
  source.meta.title = "Listening official task family fixture";
  source.meta.scope = "partial_practice";
  source.audit.sourceDocumentId = "phase7-listening-families-document";
  source.taskGroups = [];
  source.answerSlots = {};
  source.answerKey = {};
  source.questionOrder = [];
  source.questionDisplayMap = {};
  const taskIds = [];
  let questionNumber = 1;
  for (const family of manifest.families) {
    const taskId = `family-${family.taskType}`;
    const slotIds = [];
    const slotCount = family.sharedSlots || 1;
    const task = {
      taskId,
      displayRange: { kind: "set", values: [] },
      taskType: family.taskType,
      instructions: [{
        type: "paragraph",
        id: `${taskId}-instruction`,
        sourceAnchors: [sourceAnchor()],
        provenanceStatus: "source",
        children: [textNode(`${taskId}-instruction-text`, `Complete the ${family.label}.`)]
      }],
      instructionSignature: {
        normalizedText: `Complete the ${family.label}.`,
        taskType: family.taskType,
        expectedQuestionNumbers: [],
        expectedSlotCount: slotCount,
        selectionCardinality: { min: slotCount, max: slotCount, exact: slotCount },
        answerAssignment: "per_slot",
        evidenceAnchors: [sourceAnchor()],
        confidence: 1
      },
      stimulus: [{
        type: "paragraph",
        id: `${taskId}-stimulus`,
        sourceAnchors: [sourceAnchor()],
        provenanceStatus: "source",
        children: [textNode(`${taskId}-stimulus-text`, `${family.label} stimulus`)]
      }],
      responseGroups: [],
      sourceAnchors: [sourceAnchor()],
      quality: { score: 1, sourceCoverage: 1, hardFailures: [] },
      reviewState: "confirmed"
    };
    for (let index = 0; index < slotCount; index += 1) {
      const slotId = `family-q${questionNumber}`;
      const hostNodeId = `${taskId}-slot-${index + 1}`;
      slotIds.push(slotId);
      source.questionOrder.push(slotId);
      source.questionDisplayMap[slotId] = String(questionNumber);
      const interaction = family.interaction;
      source.answerSlots[slotId] = {
        slotId,
        questionNumber,
        displayLabel: String(questionNumber),
        hostNodeId,
        hostType: interaction === "hotspot" ? "figure_hotspot" : "paragraph",
        interaction,
        participation: "scoring",
        ...(interaction === "text" ? { constraints: { maxWords: 2, maxNumbers: 0 } } : {}),
        sourceAnchors: [sourceAnchor()],
        confidence: 1
      };
      source.answerKey[slotId] = interaction === "text"
        ? { kind: "text", values: [`answer-${questionNumber}`], normalization: "ielts_default" }
        : { kind: "option", labels: ["A"], assignment: "per_slot" };
      task.displayRange.values.push(questionNumber);
      task.instructionSignature.expectedQuestionNumbers.push(questionNumber);
      questionNumber += 1;
    }
    const response = {
      responseGroupId: `${taskId}-response`,
      kind: interactionKindForFamily(family.interaction),
      slotIds,
      cardinality: { min: slotCount, max: slotCount, exact: slotCount },
      assignment: "per_slot",
      scoringPolicy: "per_slot_binary",
      duplicatePolicy: "ignore_duplicates",
      allowOptionReuse: false,
      sourceAnchors: [sourceAnchor()]
    };
    if (family.interaction !== "text") {
      response.options = [familyOption("A", "Option A"), familyOption("B", "Option B"), familyOption("C", "Option C")];
      response.duplicatePolicy = "reject_submission";
    }
    task.responseGroups.push(response);
    source.taskGroups.push(task);
    taskIds.push(taskId);
  }
  source.parts = [{
    ...structuredClone(base.parts[0]),
    partId: "part-1-families",
    displayLabel: "PART 1",
    expectedQuestionNumbers: [...source.questionOrder.map((slotId) => source.answerSlots[slotId].questionNumber)],
    taskIds,
    cue: { ...structuredClone(base.parts[0].cue), startMs: 0, endMs: 1000 }
  }];
  source.assets.examId = source.examId;
  source.assets.assets[0].assetId = "audio-part1";
  source.media.assetId = "audio-part1";
  return source;
}

function interactionKindForFamily(interaction) {
  if (interaction === "text") return "text_entry";
  if (interaction === "hotspot") return "diagram_hotspot";
  return interaction === "select" ? "matching" : "choice";
}

function buildCompleteScopeFixture(base) {
  const source = structuredClone(base);
  source.examId = "phase7-listening-complete";
  source.meta.title = "Listening complete scope fixture";
  source.meta.scope = "complete_exam";
  source.taskGroups = [];
  source.answerSlots = {};
  source.answerKey = {};
  source.questionOrder = [];
  source.questionDisplayMap = {};
  const parts = [];
  for (let partIndex = 0; partIndex < 4; partIndex += 1) {
    const taskIds = [];
    for (let offset = 0; offset < 10; offset += 1) {
      const number = partIndex * 10 + offset + 1;
      const taskId = `complete-part-${partIndex + 1}-q${number}`;
      const slotId = `complete-q${number}`;
      const task = structuredClone(base.taskGroups[0]);
      task.taskId = taskId;
      task.displayRange = { kind: "set", values: [number] };
      task.instructionSignature.expectedQuestionNumbers = [number];
      task.instructionSignature.expectedSlotCount = 1;
      task.instructionSignature.taskType = "form_completion";
      task.responseGroups[0].responseGroupId = `${taskId}-response`;
      task.responseGroups[0].slotIds = [slotId];
      task.sourceAnchors = [sourceAnchor()];
      taskIds.push(taskId);
      source.taskGroups.push(task);
      source.answerSlots[slotId] = {
        ...structuredClone(base.answerSlots.q1),
        slotId,
        questionNumber: number,
        displayLabel: String(number),
        hostNodeId: `${taskId}-host`,
        sourceAnchors: [sourceAnchor()]
      };
      source.answerKey[slotId] = { kind: "text", values: [`answer-${number}`], normalization: "ielts_default" };
      source.questionOrder.push(slotId);
      source.questionDisplayMap[slotId] = String(number);
    }
    parts.push({
      ...structuredClone(base.parts[0]),
      partId: `complete-part-${partIndex + 1}`,
      displayLabel: `PART ${partIndex + 1}`,
      expectedQuestionNumbers: Array.from({ length: 10 }, (_, index) => partIndex * 10 + index + 1),
      taskIds,
      cue: { ...structuredClone(base.parts[0].cue), startMs: partIndex * 250, endMs: (partIndex + 1) * 250 }
    });
  }
  source.parts = parts;
  source.assets.examId = source.examId;
  return source;
}

const requiredFiles = [
  "contracts/listening-exam-source-v1.schema.json",
  "contracts/listening-attempt-v1.schema.json",
  "contracts/listening-audio-probe-result-v1.schema.json",
  "src/types/listening-runtime-v1.ts",
  "src/types/listening-audio-probe-v1.ts",
  "src/services/listeningRuntimeV1.ts",
  "src/services/listeningPlaybackControllerV1.ts",
  "scripts/verify-phase7-listening-package.mjs",
  "../NAS/server/src/lib/library/listening/listening-v1-loader.ts",
  "../NAS/server/src/lib/library/listening/NasJsDirectListeningAssetProvider.ts",
  "../NAS/server/src/lib/library/listening/ListeningLibraryProviderFactory.ts",
  "../NAS/server/src/lib/library/listening/listening-generated-loader.ts",
  "../NAS/server/src/lib/exam/ExamListeningService.ts",
  "../NAS/apps/student-exam/src/modules/listening-engine/contracts-v1.ts",
  "../NAS/apps/student-exam/src/modules/listening-engine/useListeningAttempt.ts",
  "../NAS/apps/student-exam/src/modules/listening-engine/featureFlag.ts",
  "../NAS/apps/student-exam/src/pages/ListeningExamPage.vue",
  "src-tauri/src/schema/listening_runtime_v1.rs",
  "src-tauri/src/schema/listening_audio_probe_v1.rs",
  fixturePath,
  audioProbeFixturePath,
  familiesFixturePath,
  "Files/IELTS_Document_Recognition_Phase_7_Progress_CN.md"
];
for (const file of requiredFiles) assert(existsSync(requiredFilePath(file)), `Phase 7 required file missing: ${file}`);

const manifest = readJson("contracts/contract-manifest.json");
for (const [schemaName, path] of Object.entries({
  ListeningExamSourceV1: "listening-exam-source-v1.schema.json",
  ListeningAttemptV1: "listening-attempt-v1.schema.json",
  ListeningAudioProbeResultV1: "listening-audio-probe-result-v1.schema.json"
})) {
  const entry = manifest.schemas?.[schemaName];
  assert(entry?.path === path, `${schemaName} is missing from contract manifest`);
  assert(entry.sha256 === sha256(`contracts/${path}`), `${schemaName} contract hash is stale`);
}

const schemaFiles = Object.values(manifest.schemas).map((entry) => entry.path);
const schemas = schemaFiles.map((file) => readJson(`contracts/${file}`));
const ajv = new Ajv2020({ allErrors: true, strict: true, strictRequired: false, validateFormats: false });
for (const schema of schemas) ajv.addSchema(schema);

const source = readJson(fixturePath);
const validateSource = ajv.getSchema("https://contracts.ielts-author-studio.dev/phase1/listening-exam-source-v1.schema.json");
const validateAttempt = ajv.getSchema("https://contracts.ielts-author-studio.dev/phase1/listening-attempt-v1.schema.json");
const validateAudioProbe = ajv.getSchema("https://contracts.ielts-author-studio.dev/phase1/listening-audio-probe-result-v1.schema.json");
const validateAuthoring = ajv.getSchema("https://contracts.ielts-author-studio.dev/phase1/ielts-authoring-ir-v2.schema.json");
assert(validateSource?.(source), `Listening source fixture failed schema validation: ${JSON.stringify(validateSource?.errors)}`);
assert(semanticIssueCodes(source).length === 0, `Listening source fixture failed semantic validation: ${semanticIssueCodes(source).join(",")}`);

const attempt = buildAttempt(source);
assert(validateAttempt?.(attempt), `Listening attempt fixture failed schema validation: ${JSON.stringify(validateAttempt?.errors)}`);
const audioProbe = readJson(audioProbeFixturePath);
assert(validateAudioProbe?.(audioProbe), `Listening audio probe fixture failed schema validation: ${JSON.stringify(validateAudioProbe?.errors)}`);
const invalidPassedProbe = structuredClone(audioProbe);
invalidPassedProbe.probe.issueCodes.push("AUDIO_NEAR_SILENT");
assert(!validateAudioProbe?.(invalidPassedProbe), "Passed audio probe accepted a blocking issue code");
const baseQuality = readJson("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json").quality;
const listeningAuthoring = buildListeningAuthoring(source, baseQuality);
assert(validateAuthoring?.(listeningAuthoring), `Listening authoring fixture failed schema validation: ${JSON.stringify(validateAuthoring?.errors)}`);

const familyManifest = readJson(familiesFixturePath);
const familiesSource = buildFamiliesFixture(source, familyManifest);
assert(validateSource?.(familiesSource), `Listening family fixture failed schema validation: ${JSON.stringify(validateSource?.errors)}`);
assert(semanticIssueCodes(familiesSource).length === 0, `Listening family fixture failed semantic validation: ${semanticIssueCodes(familiesSource).join(",")}`);
const familyTypes = new Set(familiesSource.taskGroups.map((task) => task.taskType));
for (const family of familyManifest.families) assert(familyTypes.has(family.taskType), `Listening family fixture is missing ${family.taskType}`);
assert(familiesSource.taskGroups.filter((task) => task.taskType === "short_answer")[0].responseGroups[0].slotIds.length === 2, "Shared short-answer family did not preserve two slots under one response group");
assert(familiesSource.taskGroups.filter((task) => task.taskType.includes("map") || task.taskType.includes("diagram")).every((task) => task.responseGroups[0].options.length >= 3), "Map/diagram family is missing keyboard options");

const completeSource = buildCompleteScopeFixture(source);
assert(validateSource?.(completeSource), `Complete Listening fixture failed schema validation: ${JSON.stringify(validateSource?.errors)}`);
assert(semanticIssueCodes(completeSource).length === 0, `Complete Listening fixture failed semantic validation: ${semanticIssueCodes(completeSource).join(",")}`);
assert(completeSource.parts.length === 4 && completeSource.parts.every((part) => part.expectedQuestionNumbers.length === 10), "Complete Listening fixture did not build four ten-question Parts");
assert(Object.keys(completeSource.answerSlots).length === 40, "Complete Listening fixture did not build forty scoring slots");

const completeButPartial = structuredClone(source);
completeButPartial.meta.scope = "complete_exam";
assert(semanticIssueCodes(completeButPartial).includes("LISTENING_COMPLETE_PART_COUNT"), "Complete exam scope accepted one Part");
assert(semanticIssueCodes(completeButPartial).includes("LISTENING_COMPLETE_QUESTION_COUNT"), "Complete exam scope accepted one question");

const invalidCue = structuredClone(source);
invalidCue.parts[0].cue.endMs = source.media.durationMs + 1;
assert(semanticIssueCodes(invalidCue).includes("AUDIO_CUE_INVALID"), "Out-of-range cue was accepted");

const invalidHash = structuredClone(source);
invalidHash.media.sha256 = "c".repeat(64);
assert(semanticIssueCodes(invalidHash).includes("AUDIO_HASH_MISMATCH"), "Audio hash mismatch was accepted");

const mockSource = structuredClone(source);
mockSource.playbackPolicy = {
  mode: "mock",
  autoplay: true,
  allowPause: false,
  allowSeek: false,
  allowReplay: false,
  maxPlays: 1,
  refreshBehavior: "resume_from_snapshot",
  crashRecoveryBehavior: "resume_from_snapshot",
  showCurrentTime: false,
  showDuration: false
};
assert(validateSource?.(mockSource), `Mock source policy failed schema validation: ${JSON.stringify(validateSource?.errors)}`);
assert(semanticIssueCodes(mockSource).length === 0, `Mock source policy failed semantic validation: ${semanticIssueCodes(mockSource).join(",")}`);
const invalidMockPolicy = structuredClone(mockSource);
invalidMockPolicy.playbackPolicy.allowSeek = true;
assert(!validateSource?.(invalidMockPolicy), "Mock source schema accepted seek=true");
assert(semanticIssueCodes(invalidMockPolicy).includes("AUDIO_POLICY_MISSING"), "Mock semantic gate accepted seek=true");

const runtimeTypes = readFileSync(join(repoRoot, "src/types/listening-runtime-v1.ts"), "utf8");
for (const token of ["ListeningExamSourceV1", "ListeningAttemptV1", "ListeningAudioProbeV1", "ListeningPlaybackSnapshotV1"]) {
  assert(runtimeTypes.includes(token), `Listening TypeScript mirror is missing ${token}`);
}
const audioProbeTypes = readFileSync(join(repoRoot, "src/types/listening-audio-probe-v1.ts"), "utf8");
for (const token of ["ListeningAudioProbeResultV1", "ListeningAudioSignalMetricsV1", "DEFAULT_LISTENING_AUDIO_PROBE_POLICY_V1"]) {
  assert(audioProbeTypes.includes(token), `Listening audio probe TypeScript mirror is missing ${token}`);
}
const playbackController = readFileSync(join(repoRoot, "src/services/listeningPlaybackControllerV1.ts"), "utf8");
for (const token of ["createListeningPlaybackSnapshotV1", "transitionListeningPlaybackV1", "validateListeningPlaybackSnapshotV1", "PLAYBACK_REPLAY_FORBIDDEN"]) {
  assert(playbackController.includes(token), `Listening playback controller is missing ${token}`);
}

const playbackTranspile = ts.transpileModule(playbackController, {
  compilerOptions: {
    module: ts.ModuleKind.ESNext,
    target: ts.ScriptTarget.ES2022,
    verbatimModuleSyntax: false
  },
  fileName: "src/services/listeningPlaybackControllerV1.ts",
  reportDiagnostics: true
});
const playbackTranspileErrors = (playbackTranspile.diagnostics ?? []).filter((diagnostic) => diagnostic.category === ts.DiagnosticCategory.Error);
assert(playbackTranspileErrors.length === 0, `Playback controller transpile failed: ${playbackTranspileErrors.map((error) => error.messageText).join(",")}`);
const playbackApi = await import(`data:text/javascript;base64,${Buffer.from(playbackTranspile.outputText).toString("base64")}`);

function assertPlaybackError(operation, expectedCode) {
  try {
    operation();
  } catch (error) {
    assert(error?.code === expectedCode, `Expected ${expectedCode}, received ${error?.code ?? error}`);
    return;
  }
  throw new Error(`Expected playback operation to fail with ${expectedCode}`);
}

const at = (second) => `2026-08-12T00:00:${String(second).padStart(2, "0")}Z`;
const initialPlayback = playbackApi.createListeningPlaybackSnapshotV1(source, at(0));
const practicePlaying = playbackApi.transitionListeningPlaybackV1(source, initialPlayback, { type: "play", at: at(1) });
const practiceProgress = playbackApi.transitionListeningPlaybackV1(source, practicePlaying, { type: "progress", positionMs: 200, at: at(2) });
const practiceRefresh = playbackApi.transitionListeningPlaybackV1(source, practiceProgress, { type: "refresh_recover", at: at(3) });
assert(practiceRefresh.playsStarted === 1 && practiceRefresh.positionMs === 200 && practiceRefresh.status === "playing", "Practice refresh did not resume the serialized snapshot");
const practicePaused = playbackApi.transitionListeningPlaybackV1(source, practiceRefresh, { type: "pause", positionMs: 300, at: at(4) });
const practiceCrash = playbackApi.transitionListeningPlaybackV1(source, practicePaused, { type: "crash_recover", at: at(5) });
assert(practiceCrash.playsStarted === 1 && practiceCrash.positionMs === 300 && practiceCrash.status === "paused", "Practice crash recovery did not preserve the serialized snapshot");
const practiceSeek = playbackApi.transitionListeningPlaybackV1(source, practiceCrash, { type: "seek", positionMs: 400, at: at(6) });
const practiceResume = playbackApi.transitionListeningPlaybackV1(source, practiceSeek, { type: "play", at: at(7) });
const practiceEnded = playbackApi.transitionListeningPlaybackV1(source, practiceResume, { type: "ended", at: at(8) });
const practiceReplay = playbackApi.transitionListeningPlaybackV1(source, practiceEnded, { type: "play", at: at(9) });
assert(practiceReplay.playsStarted === 2 && practiceReplay.positionMs === 0, "Practice replay did not consume a new play");

const mockInitial = playbackApi.createListeningPlaybackSnapshotV1(mockSource, at(0));
const mockPlaying = playbackApi.transitionListeningPlaybackV1(mockSource, mockInitial, { type: "play", at: at(1) });
const mockProgress = playbackApi.transitionListeningPlaybackV1(mockSource, mockPlaying, { type: "progress", positionMs: 200, at: at(2) });
const mockRefresh = playbackApi.transitionListeningPlaybackV1(mockSource, mockProgress, { type: "refresh_recover", at: at(3) });
assert(mockRefresh.playsStarted === 1 && mockRefresh.positionMs === 200 && mockRefresh.status === "playing", "Mock refresh reset play count or position");
assertPlaybackError(() => playbackApi.transitionListeningPlaybackV1(mockSource, mockRefresh, { type: "pause", positionMs: 300, at: at(4) }), "PLAYBACK_PAUSE_FORBIDDEN");
assertPlaybackError(() => playbackApi.transitionListeningPlaybackV1(mockSource, mockRefresh, { type: "seek", positionMs: 300, at: at(4) }), "PLAYBACK_SEEK_FORBIDDEN");
assertPlaybackError(
  () => playbackApi.transitionListeningPlaybackV1(mockSource, { ...mockRefresh, playsStarted: 0 }, { type: "refresh_recover", at: at(4) }),
  "PLAYBACK_SNAPSHOT_INVALID"
);
const mockEnded = playbackApi.transitionListeningPlaybackV1(mockSource, mockRefresh, { type: "ended", at: at(5) });
assertPlaybackError(() => playbackApi.transitionListeningPlaybackV1(mockSource, mockEnded, { type: "play", at: at(6) }), "PLAYBACK_REPLAY_FORBIDDEN");

const restartSource = structuredClone(source);
restartSource.playbackPolicy.maxPlays = 2;
restartSource.playbackPolicy.crashRecoveryBehavior = "restart_if_allowed";
const restartInitial = playbackApi.createListeningPlaybackSnapshotV1(restartSource, at(0));
const restartPlaying = playbackApi.transitionListeningPlaybackV1(restartSource, restartInitial, { type: "play", at: at(1) });
const restartPending = playbackApi.transitionListeningPlaybackV1(restartSource, restartPlaying, { type: "crash_recover", at: at(2) });
assert(restartPending.status === "restart_pending" && restartPending.playsStarted === 1, "Restart recovery did not preserve consumed play count");
const restarted = playbackApi.transitionListeningPlaybackV1(restartSource, restartPending, { type: "play", at: at(3) });
assert(restarted.playsStarted === 2, "Restart recovery did not consume the second play");
const exhaustedRecovery = playbackApi.transitionListeningPlaybackV1(restartSource, restarted, { type: "crash_recover", at: at(4) });
assert(exhaustedRecovery.status === "failed" && exhaustedRecovery.failureCode === "AUDIO_RECOVERY_BLOCKED", "Exhausted crash recovery did not fail closed");

const blockedAudioSource = structuredClone(source);
blockedAudioSource.media.probe.status = "blocked";
blockedAudioSource.media.probe.issueCodes = ["AUDIO_DECODE_FAILED"];
assertPlaybackError(() => playbackApi.createListeningPlaybackSnapshotV1(blockedAudioSource, at(0)), "PLAYBACK_SOURCE_BLOCKED");

const listeningRuntimeSource = readFileSync(join(repoRoot, "src/services/listeningRuntimeV1.ts"), "utf8");
const listeningRuntimeTranspile = ts.transpileModule(listeningRuntimeSource, {
  compilerOptions: {
    module: ts.ModuleKind.ESNext,
    target: ts.ScriptTarget.ES2022,
    verbatimModuleSyntax: false
  },
  fileName: "src/services/listeningRuntimeV1.ts",
  reportDiagnostics: true
});
const listeningRuntimeErrors = (listeningRuntimeTranspile.diagnostics ?? []).filter((diagnostic) => diagnostic.category === ts.DiagnosticCategory.Error);
assert(listeningRuntimeErrors.length === 0, `Listening runtime service transpile failed: ${listeningRuntimeErrors.map((error) => error.messageText).join(",")}`);
const listeningRuntimeApi = await import(`data:text/javascript;base64,${Buffer.from(listeningRuntimeTranspile.outputText).toString("base64")}`);
const listeningAttempt = listeningRuntimeApi.createListeningAttempt(source, new Date("2026-08-12T00:00:00.000Z"));
const answeredListeningAttempt = listeningRuntimeApi.setListeningSlotAnswer(source, listeningAttempt, "q1", { kind: "text", values: ["Jones"], normalization: "ielts_default" }, new Date("2026-08-12T00:00:01.000Z"));
const submittedListeningAttempt = listeningRuntimeApi.submitListeningAttempt(source, answeredListeningAttempt, new Date("2026-08-12T00:00:02.000Z"));
const listeningScore = listeningRuntimeApi.scoreListeningAttempt(source, submittedListeningAttempt);
assert(submittedListeningAttempt.state === "submitted" && listeningScore.correct && listeningScore.earnedPoints === 1, "Listening one-Part form completion did not complete slotId submit/scoring");
assertPlaybackError(() => listeningRuntimeApi.setListeningSlotAnswer(source, listeningAttempt, "unknown-slot", { kind: "text", values: ["x"] }), "RUNTIME_UNKNOWN_SLOT");
assertPlaybackError(() => listeningRuntimeApi.submitListeningAttempt(source, { ...answeredListeningAttempt, sourceRevision: answeredListeningAttempt.sourceRevision + 1 }), "RUNTIME_ATTEMPT_REVISION_MISMATCH");
assertPlaybackError(() => listeningRuntimeApi.submitListeningAttempt(source, listeningAttempt), "RUNTIME_ANSWER_REQUIRED");
assertPlaybackError(() => listeningRuntimeApi.createListeningAttempt(blockedAudioSource), "AUDIO_PROBE_BLOCKED");

const flags = readFileSync(join(repoRoot, "src/config/featureFlags.ts"), "utf8");
assert(flags.includes("listeningV1: false"), "Listening V1 feature flag must remain disabled by default");
const listeningRoute = readFileSync(join(peerRoot ?? repoRoot, "server/src/routes/exam.ts"), "utf8");
assert(listeningRoute.includes("/api/exam/listening/:examId/submit"), "Listening submit route is missing");
assert(listeningRoute.includes("/api/exam/listening/:examId/progress") && listeningRoute.includes("assertListeningFeatureEnabled"), "Listening progress route or feature gate is missing");
assert(!listeningRoute.includes("source: loaded.source"), "Listening route must not expose the answer key source to the student");
const listeningService = readFileSync(join(peerRoot ?? repoRoot, "server/src/lib/exam/ExamListeningService.ts"), "utf8");
for (const token of ["sourceRevision", "mediaSha256", "attemptId", "listening_attempt_answers_incomplete", "validatePlaybackSnapshot", "saveProgress", "scoreAnswer"]) {
  assert(listeningService.includes(token), `Listening service is missing ${token}`);
}
const listeningPage = readFileSync(join(peerRoot ?? repoRoot, "apps/student-exam/src/pages/ListeningExamPage.vue"), "utf8");
for (const token of ["audioReady", "allowSeek", "allowReplay", "ListeningPlaybackSnapshotV1", "assetManifest", "getListeningAssetUrl", "saveProgress", "startSuite", "ReadingExam", "submitAttempt", "Audio failed to load"]) {
  assert(listeningPage.includes(token), `Listening student page is missing ${token}`);
}
const attemptService = readFileSync(join(peerRoot ?? repoRoot, "server/src/lib/exam/ExamAttemptService.ts"), "utf8");
assert(attemptService.includes("'listening'") && attemptService.includes("listeningSnapshot") && attemptService.includes("listeningAnswers"), "Exam attempt recovery is missing Listening phase data");
const startupPage = readFileSync(join(peerRoot ?? repoRoot, "apps/student-exam/src/pages/StartupCheckPage.vue"), "utf8");
assert(startupPage.includes("data.phase === 'listening'") && startupPage.includes("ListeningExam"), "Startup recovery does not route Listening phase");
const listeningProvider = readFileSync(join(peerRoot ?? repoRoot, "server/src/lib/library/listening/listening-v1-loader.ts"), "utf8");
assert(listeningProvider.includes("listening_asset_manifest_mismatch") && listeningProvider.includes("sourceAssetIds") && listeningProvider.includes("stagedAssetIds") && listeningProvider.includes("getAssetBytes"), "Listening provider is missing exact source/manifest asset closure");
const listeningFlag = readFileSync(join(peerRoot ?? repoRoot, "apps/student-exam/src/modules/listening-engine/featureFlag.ts"), "utf8");
assert(listeningFlag.includes("VITE_IELTS_LISTENING_V1") && listeningFlag.includes("=== '1'"), "Student Listening V1 feature flag must default to disabled");

function run(command, args) {
  const executable = process.platform === "win32" && command === "npm" ? "npm.cmd" : command;
  const result = spawnSync(executable, args, { cwd: repoRoot, stdio: "inherit", shell: process.platform === "win32" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run("npm", ["run", "check"]);
run("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "schema::listening_", "--", "--nocapture"]);
run("npm", ["run", "verify:phase7:listening-package"]);
const nasRoot = resolve(repoRoot, "../NAS");
const nasBuild = spawnSync(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "build:server"], { cwd: nasRoot, stdio: "inherit", shell: process.platform === "win32" });
if (nasBuild.status !== 0) process.exit(nasBuild.status ?? 1);
const studentBuild = spawnSync(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "build"], { cwd: join(nasRoot, "apps/student-exam"), stdio: "inherit", shell: process.platform === "win32" });
if (studentBuild.status !== 0) process.exit(studentBuild.status ?? 1);

console.log(JSON.stringify({
  schemaVersion: "Phase7ListeningContractVerificationReportV1",
  examId: source.examId,
  scope: source.meta.scope,
  partCount: source.parts.length,
  scoringSlotCount: Object.values(source.answerSlots).filter((slot) => slot.participation === "scoring").length,
  checks: [
    "authoring-listening-structure",
    "runtime-source-schema",
    "attempt-schema",
    "audio-asset-closure",
    "cue-boundary",
    "partial-vs-complete-scope",
    "playback-policy",
    "real-wav-decode-and-duration",
    "mp3-aac-decoder-registration",
    "hash-and-supported-mime-policy",
    "near-silence-and-clipping",
    "audio-probe-result-schema",
    "practice-playback-transitions",
    "one-part-form-provider-attempt-submit-score",
    "mock-no-pause-seek-replay",
    "refresh-crash-recovery-preserves-play-count",
    "restart-recovery-fails-closed-at-limit",
    "typescript",
    "rust-round-trip-and-semantics",
    "nas-listening-provider-service-and-student-page"
    ,"nas-listening-package-provider-and-corruption-probe"
  ],
  status: "passed"
}, null, 2));
