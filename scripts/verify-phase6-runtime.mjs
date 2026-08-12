import { createHash } from "node:crypto";
import { readFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import Ajv2020 from "ajv/dist/2020.js";
import ts from "typescript";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));

function readJson(relativePath) {
  return JSON.parse(readFileSync(join(repoRoot, relativePath), "utf8"));
}

function sha256(relativePath) {
  return createHash("sha256").update(readFileSync(join(repoRoot, relativePath))).digest("hex");
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function buildRuntimeSource(authoring) {
  const slots = Object.values(authoring.answerSlots ?? {});
  const questionOrder = [...slots]
    .sort((left, right) => left.questionNumber - right.questionNumber || left.slotId.localeCompare(right.slotId))
    .map((slot) => slot.slotId);
  return {
    schemaVersion: "ReadingExamSourceV2",
    examId: authoring.exam.examId,
    meta: {
      title: authoring.exam.title,
      language: authoring.exam.language,
      ...(authoring.exam.category ? { category: authoring.exam.category } : {})
    },
    assets: { examId: authoring.exam.examId, assets: authoring.assets ?? [] },
    passage: {
      content: authoring.passage?.content ?? [],
      ...(authoring.passage?.paragraphMap ? { paragraphMap: authoring.passage.paragraphMap } : {})
    },
    taskGroups: authoring.taskGroups ?? [],
    answerSlots: authoring.answerSlots ?? {},
    answerKey: authoring.answerKey ?? {},
    questionOrder,
    questionDisplayMap: Object.fromEntries(slots.map((slot) => [slot.slotId, slot.displayLabel])),
    audit: {
      sourceSchemaVersion: authoring.schemaVersion,
      sourceDocumentId: authoring.sourceDocumentId,
      sourceRevision: authoring.audit?.revision ?? 0,
      sourceRevisionKind: authoring.audit?.source ?? "auto_extract"
    }
  };
}

function semanticIssues(source) {
  const issues = [];
  const slotIds = new Set(Object.keys(source.answerSlots ?? {}));
  const order = source.questionOrder ?? [];
  if (order.length !== slotIds.size || new Set(order).size !== order.length || order.some((id) => !slotIds.has(id))) {
    issues.push("question_order_invalid");
  }
  const answerIds = new Set(Object.keys(source.answerKey ?? {}));
  if (answerIds.size !== slotIds.size || [...answerIds].some((id) => !slotIds.has(id))) issues.push("answer_key_mismatch");
  const assigned = new Set();
  const responseIds = new Set();
  for (const task of source.taskGroups ?? []) {
    for (const response of task.responseGroups ?? []) {
      if (responseIds.has(response.responseGroupId)) issues.push(`duplicate_response:${response.responseGroupId}`);
      responseIds.add(response.responseGroupId);
      for (const slotId of response.slotIds ?? []) {
        if (!slotIds.has(slotId)) issues.push(`missing_slot:${slotId}`);
        if (assigned.has(slotId)) issues.push(`duplicate_slot:${slotId}`);
        assigned.add(slotId);
      }
    }
  }
  for (const slotId of slotIds) if (!assigned.has(slotId)) issues.push(`unassigned_slot:${slotId}`);
  return issues;
}

function safeAssetRelativePath(relativePath) {
  if (!relativePath || relativePath.includes("\\") || relativePath.includes("://") || relativePath.includes(":")) return false;
  if (relativePath.startsWith("/") || relativePath.startsWith("//") || ["⁄", "∕", "╱", "⧸"].some((slash) => relativePath.includes(slash))) return false;
  return relativePath.split("/").every((segment) => segment && segment !== "." && segment !== "..");
}

const requiredFiles = [
  "contracts/reading-exam-source-v2.schema.json",
  "src/types/reading-runtime-v2.ts",
  "src/services/readingRuntimeV2.ts",
  "src-tauri/src/reading_runtime_v2.rs",
  "src-tauri/src/nas_package_v2.rs",
  "src-tauri/src/lib.rs",
  "src/api/tauriCommands.ts",
  "src/pages/ExportPage.tsx",
  "src/pages/StructuredAuthoringEditorV2.tsx",
  "Files/IELTS_Document_Recognition_Phase_6_Progress_CN.md"
];
for (const file of requiredFiles) assert(existsSync(join(repoRoot, file)), `Phase 6 required file missing: ${file}`);

const manifest = readJson("contracts/contract-manifest.json");
const runtimeEntry = manifest.schemas?.ReadingExamSourceV2;
assert(runtimeEntry?.path === "reading-exam-source-v2.schema.json", "ReadingExamSourceV2 is missing from contract manifest");
assert(runtimeEntry.sha256 === sha256(`contracts/${runtimeEntry.path}`), "ReadingExamSourceV2 contract hash is stale");

const schemas = [
  "common-v2.schema.json",
  "content-doc-v2.schema.json",
  "ielts-authoring-ir-v2.schema.json",
  "quality-report-v2.schema.json",
  "reading-exam-source-v2.schema.json"
].map((file) => readJson(`contracts/${file}`));
const ajv = new Ajv2020({ allErrors: true, strict: true, strictRequired: false, validateFormats: false });
for (const schema of schemas) ajv.addSchema(schema);
const validateRuntime = ajv.getSchema("https://contracts.ielts-author-studio.dev/phase1/reading-exam-source-v2.schema.json");
assert(validateRuntime, "ReadingExamSourceV2 validator was not compiled");

const authoring = readJson("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json");
const runtime = buildRuntimeSource(authoring);
assert(validateRuntime(runtime), `ReadingExamSourceV2 fixture failed schema validation: ${JSON.stringify(validateRuntime.errors)}`);
assert(semanticIssues(runtime).length === 0, `ReadingExamSourceV2 fixture failed semantic validation: ${semanticIssues(runtime).join(",")}`);

const invalidRuntime = structuredClone(runtime);
invalidRuntime.answerKey = {};
assert(semanticIssues(invalidRuntime).includes("answer_key_mismatch"), "Runtime semantic probe accepted a missing answer key");
const invalidSchemaRuntime = structuredClone(runtime);
invalidSchemaRuntime.schemaVersion = "ReadingExamSourceV99";
assert(!validateRuntime(invalidSchemaRuntime), "Runtime schema probe accepted an unsupported version");

for (const path of ["../escape.png", "/absolute.png", "C:/drive.png", "\\\\server\\share\\x.png", "https://example.invalid/x.png", "images/⁄escape.png", "images/../x.png"]) {
  assert(!safeAssetRelativePath(path), `unsafe asset path accepted: ${path}`);
}
assert(safeAssetRelativePath("diagrams/abc123.png"), "valid content-addressed asset path was rejected");

function transpileRuntimeModule(relativePath) {
  const result = ts.transpileModule(readFileSync(join(repoRoot, relativePath), "utf8"), {
    compilerOptions: {
      module: ts.ModuleKind.ESNext,
      target: ts.ScriptTarget.ES2022,
      verbatimModuleSyntax: false
    },
    fileName: relativePath,
    reportDiagnostics: true
  });
  const errors = (result.diagnostics ?? []).filter((diagnostic) => diagnostic.category === ts.DiagnosticCategory.Error);
  assert(errors.length === 0, `Failed to transpile ${relativePath}: ${errors.map((entry) => entry.messageText).join(", ")}`);
  return result.outputText;
}

const runtimeTypesModuleUrl = `data:text/javascript;base64,${Buffer.from(transpileRuntimeModule("src/types/reading-runtime-v2.ts")).toString("base64")}`;
const runtimeServiceJs = transpileRuntimeModule("src/services/readingRuntimeV2.ts")
  .replace(/from\s+["']\.\.\/types\/reading-runtime-v2["']/u, `from ${JSON.stringify(runtimeTypesModuleUrl)}`);
const runtimeApi = await import(`data:text/javascript;base64,${Buffer.from(runtimeServiceJs).toString("base64")}`);

function assertRuntimeErrorCode(operation, expectedCode) {
  try {
    operation();
  } catch (error) {
    assert(error?.code === expectedCode, `Expected ${expectedCode}, received ${error?.code ?? error}`);
    return;
  }
  throw new Error(`Expected runtime operation to fail with ${expectedCode}`);
}

const initialAttempt = runtimeApi.createReadingAttempt(runtime, new Date("2026-08-12T00:00:00.000Z"));
assertRuntimeErrorCode(
  () => runtimeApi.setReadingSlotAnswer(runtime, { ...initialAttempt, examId: "other-exam" }, "q14", { kind: "option", labels: ["B"], assignment: "unordered_set" }),
  "RUNTIME_ATTEMPT_EXAM_MISMATCH"
);
assertRuntimeErrorCode(
  () => runtimeApi.clearReadingSlotAnswer(runtime, { ...initialAttempt, sourceRevision: initialAttempt.sourceRevision + 1 }, "q14"),
  "RUNTIME_ATTEMPT_REVISION_MISMATCH"
);

const textRuntime = structuredClone(runtime);
const textTask = textRuntime.taskGroups[0];
textTask.responseGroups = [{
  ...textTask.responseGroups[0],
  kind: "completion",
  slotIds: ["q14"],
  optionBankRef: undefined,
  cardinality: { min: 1, max: 1, exact: 1 },
  assignment: "per_slot"
}];
textRuntime.answerSlots = {
  q14: { ...textRuntime.answerSlots.q14, interaction: "text" }
};
textRuntime.answerKey = {
  q14: { kind: "text", values: ["answer"], normalization: "case_insensitive" }
};
textRuntime.questionOrder = ["q14"];
textRuntime.questionDisplayMap = { q14: textRuntime.answerSlots.q14.displayLabel };
const emptyTextAttempt = {
  ...runtimeApi.createReadingAttempt(textRuntime),
  answers: { q14: { kind: "text", values: [] } }
};
assertRuntimeErrorCode(
  () => runtimeApi.submitReadingAttempt(textRuntime, emptyTextAttempt),
  "RUNTIME_ANSWER_REQUIRED"
);

const runtimeTypes = readFileSync(join(repoRoot, "src/types/reading-runtime-v2.ts"), "utf8");
for (const token of ["ReadingExamSourceV2", "ReadingAttemptV2", "ReadingInteractionModelV2", "ExamAssetManifestV2", "slotId"]) {
  assert(runtimeTypes.includes(token), `Phase 6 runtime type contract is missing ${token}`);
}
const runtimeService = readFileSync(join(repoRoot, "src/services/readingRuntimeV2.ts"), "utf8");
for (const token of ["normalizeReadingSource", "buildReadingRuntimeInteractionModel", "createReadingAttempt", "setReadingSlotAnswer", "submitReadingAttempt", "scoreReadingAttempt", "safeAssetRelativePath", "resolveAsset"]) {
  assert(runtimeService.includes(token), `Phase 6 runtime service is missing ${token}`);
}
const flags = readFileSync(join(repoRoot, "src/config/featureFlags.ts"), "utf8");
assert(flags.includes("runtimeSourceV2: false") && flags.includes("nasPackageV2: false"), "Phase 6 rollout flags must remain disabled by default");
const tauriLib = readFileSync(join(repoRoot, "src-tauri/src/lib.rs"), "utf8");
assert(tauriLib.includes("async fn publish_nas_package_v2") && tauriLib.includes("publish_nas_package_v2,"), "Phase 6 Tauri NAS V2 command is not wired into generate_handler");
const tauriApi = readFileSync(join(repoRoot, "src/api/tauriCommands.ts"), "utf8");
assert(tauriApi.includes("export async function exportNasPackageV2") && tauriApi.includes('command("publish_nas_package_v2"'), "Phase 6 authoring API is missing exportNasPackageV2");
const authoringExport = readFileSync(join(repoRoot, "src-tauri/src/authoring_v2_commands.rs"), "utf8");
assert(authoringExport.includes("materialize_authoring_assets") && authoringExport.includes("stage_file_with_hash") && authoringExport.includes("authoring_v2_asset_hash_mismatch"), "Phase 6 authoring export must materialize and verify runtime assets");
const exportPage = readFileSync(join(repoRoot, "src/pages/ExportPage.tsx"), "utf8");
assert(exportPage.includes("NAS_PACKAGE_V2_ENABLED") && exportPage.includes("exportNasPackageV2") && exportPage.includes("nas-package-v2-export-result"), "Phase 6 ExportPage V2 opt-in/probe result is not wired");
const nasRoot = resolve(repoRoot, "../NAS");
const readingServicePath = join(nasRoot, "server/src/lib/exam/ExamReadingService.ts");
if (existsSync(readingServicePath)) {
  const readingService = readFileSync(readingServicePath, "utf8");
  assert(readingService.includes("runtimeSourceV2") && readingService.includes("answerKey: {}"), "Reading V2 HTTP facade must redact answerKey from student payloads");
}

function run(command, args) {
  const executable = process.platform === "win32" && command === "npm" ? "npm.cmd" : command;
  const result = spawnSync(executable, args, { cwd: repoRoot, stdio: "inherit", shell: process.platform === "win32" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run("npm", ["run", "check"]);
run("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "reading_runtime_v2", "--", "--nocapture"]);
run("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "nas_package_v2", "--", "--nocapture"]);

console.log(JSON.stringify({
  schemaVersion: "Phase6RuntimeVerificationReportV1",
  schema: "ReadingExamSourceV2",
  examId: runtime.examId,
  taskGroupCount: runtime.taskGroups.length,
  answerSlotCount: Object.keys(runtime.answerSlots).length,
  checks: ["schema", "semantic-cross-fields", "negative-schema", "negative-slot-map", "attempt-resume-boundary", "empty-text-submit", "asset-path-policy", "typescript", "rust-asset-probe", "rust-nas-package-staging", "rust-manifest-last-cas-lock-journal-rollback"],
  status: "passed"
}, null, 2));
