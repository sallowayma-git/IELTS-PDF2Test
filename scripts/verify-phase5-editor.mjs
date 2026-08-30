import { existsSync, readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const requiredFiles = [
  "src/pages/StructuredAuthoringEditorV2.tsx",
  "src/editor/authoringTiptap.tsx",
  "src/types/runtime-view-model-v2.ts",
  "src/services/runtimeViewModelV2.ts",
  "src/services/authoringV2Patches.ts",
  "src/services/phase5Fixture.ts",
  "src/types/authoring-editor-v2.ts",
  "src-tauri/src/authoring_v2_commands.rs",
  "src-tauri/src/artifact_store.rs",
  "fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
];

const packageJson = JSON.parse(readFileSync("package.json", "utf8"));
for (const dependency of ["@tiptap/core", "@tiptap/react", "@tiptap/starter-kit", "@tiptap/extension-table", "@tiptap/extension-image"]) {
  if (!packageJson.dependencies?.[dependency]) throw new Error("Phase 5 Tiptap dependency is missing: " + dependency);
}

for (const file of requiredFiles) {
  if (!existsSync(file)) throw new Error("Phase 5 required file missing: " + file);
}

const flags = readFileSync("src/config/featureFlags.ts", "utf8");
if (!flags.includes("authoringEditorV2: true")) throw new Error("authoringEditorV2 must be the default authoring surface");
if (!flags.includes("pdfPerQuestionLlmRepair: false")) throw new Error("PDF per-question LLM repair safety flag is missing");
if (!flags.includes("return true")) throw new Error("Phase 5 editor must always use the structured authoring surface");

const editor = readFileSync("src/pages/StructuredAuthoringEditorV2.tsx", "utf8");
for (const token of [
  "ContentNodeV2",
  "responseGroups",
  "answerSlots",
  "optionBank",
  "sourceAnchorsFor",
  "AuthoringEditorRecoveryV2",
  "applyAuthoringV2Patches",
  "AuthoringTiptapEditor",
  "inverseAuthoringPatch",
  "cropAsset",
  "setHotspot",
  "insertNode",
  "deleteNode",
  "moveNode",
  "undo",
  "redo",
  "exportAuthoringV2",
  "setQuestionExpression",
  "setResponseCardinality",
  "setResponseGroup",
  "structured-expression-editor",
  "cardinality-editor",
  "optionBankRef",
  "scoringPolicy",
  "duplicatePolicy",
  "allowOptionReuse",
  "NAS_PACKAGE_V2_ENABLED = true"
]) {
  if (!editor.includes(token)) throw new Error("Phase 5 editor is missing " + token);
}
for (const token of [
  "activeTasks.map((task) => renderTaskEditor(task))",
  "task.responseGroups.length ? task.responseGroups.map((group) => renderResponseGroup(task, group))"
]) {
  if (!editor.includes(token)) throw new Error("Phase 6 editor coverage contract is missing " + token);
}
const exportPage = readFileSync("src/pages/ExportPage.tsx", "utf8");
if (!exportPage.includes("!NAS_PACKAGE_V2_ENABLED") || !exportPage.includes('data-testid="force-export"')) {
  throw new Error("V2 export must hide the unsupported force-publish action while preserving V1 force export");
}
const importWizard = readFileSync("src/pages/ImportWizard.tsx", "utf8");
for (const token of ["getAuthoringV2", 'destination = "authoring-v2"', "Existing jobs without a structured artifact remain readable"]) {
  if (!importWizard.includes(token)) throw new Error("Phase 5 import-to-editor routing is missing " + token);
}

const tiptap = readFileSync("src/editor/authoringTiptap.tsx", "utf8");
for (const token of ["textSegmentsFromTiptap", "headerScope", "tableHeader", "tableCell"]) {
  if (!tiptap.includes(token)) throw new Error("Phase 5 Tiptap roundtrip contract is missing " + token);
}

const patches = readFileSync("src/services/authoringV2Patches.ts", "utf8");
for (const token of ["ensureAnswerSlotsRemain", "AUTHORING_PATCH_ANSWER_SLOT_LOSS", "allowAnswerSlotRemoval", "restoreProvenanceStatus"]) {
  if (!patches.includes(token)) throw new Error("Phase 5 patch safety contract is missing " + token);
}

const runtimeModel = readFileSync("src/services/runtimeViewModelV2.ts", "utf8");
for (const token of ["RuntimeViewModelV2", "questionOrder", "answerSlots", "assets"]) {
  if (!runtimeModel.includes(token)) throw new Error("Phase 5 runtime projection is missing " + token);
}

for (const token of ["phase5-export-blockers", "exportBlocked", "recoveryCandidate.baseRevision", "anchorsOverride", "sourceAnchorStyle", "selectIssue", "ReadingInteractionModelV2", "RuntimeViewModelV2"]) {
  if (!editor.includes(token)) throw new Error("Phase 5 editor audit boundary is missing " + token);
}

const rustAuthoring = readFileSync("src-tauri/src/authoring_v2_commands.rs", "utf8");
for (const token of ["AUTHORING_PATCH_ANSWER_SLOT_LOSS", "phase5-export.lock", "AuthoringV2ExportJournalV1", "remove_dir_all", "preserve_provenance"]) {
  if (!rustAuthoring.includes(token)) throw new Error("Phase 5 backend safety contract is missing " + token);
}

const environment = readFileSync("src-tauri/src/environment.rs", "utf8");
if (!environment.includes("pub(crate) fn authoring_v2_shadow_enabled() -> bool {\n    true")) {
  throw new Error("Structured authoring must always be enabled");
}
const fallback = readFileSync("src/services/devFallbackBackend.ts", "utf8");
for (const token of ["staleDerivedQualityCodes", "RUNTIME_COMPILER_FAILED", "blockingIssues"]) {
  if (!fallback.includes(token)) throw new Error("Phase 5 fallback export gate is missing " + token);
}

const rust = readFileSync("src-tauri/src/lib.rs", "utf8");
for (const command of ["get_authoring_v2", "apply_authoring_v2_patches", "export_authoring_v2"]) {
  if (!rust.includes(command)) throw new Error("Tauri command is not registered: " + command);
}

const fixture = JSON.parse(readFileSync("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json", "utf8"));
const task = fixture.taskGroups?.[0];
if (task?.displayRange?.kind !== "set" || task.displayRange.values.join(",") !== "14,15") {
  throw new Error("Shared Questions 14 and 15 fixture changed unexpectedly");
}
if (task.responseGroups?.[0]?.slotIds?.join(",") !== "q14,q15") {
  throw new Error("Shared response group slot contract is missing");
}

const tsc = spawnSync(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "check"], {
  stdio: "inherit",
  shell: process.platform === "win32"
});
if (tsc.status !== 0) process.exit(tsc.status ?? 1);

console.log("Phase 5 structured editor verification passed: Tiptap schema, structural patches, source issue rail, shared slots, recovery/history contract, V2 export, and V1 safety boundary are present.");
