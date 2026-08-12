import { existsSync, readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const requiredFiles = [
  "src/pages/StructuredAuthoringEditorV2.tsx",
  "src/services/authoringV2Patches.ts",
  "src/services/phase5Fixture.ts",
  "src/types/authoring-editor-v2.ts",
  "src-tauri/src/authoring_v2_commands.rs",
  "src-tauri/src/artifact_store.rs",
  "fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
];

for (const file of requiredFiles) {
  if (!existsSync(file)) throw new Error("Phase 5 required file missing: " + file);
}

const flags = readFileSync("src/config/featureFlags.ts", "utf8");
if (!flags.includes("authoringEditorV2: false")) throw new Error("authoringEditorV2 must remain disabled by default");
if (!flags.includes("pdfPerQuestionLlmRepair: false")) throw new Error("PDF per-question LLM repair safety flag is missing");

const editor = readFileSync("src/pages/StructuredAuthoringEditorV2.tsx", "utf8");
for (const token of [
  "ContentNodeV2",
  "responseGroups",
  "answerSlots",
  "optionBank",
  "sourceAnchorsFor",
  "AuthoringEditorRecoveryV2",
  "applyAuthoringV2Patches"
]) {
  if (!editor.includes(token)) throw new Error("Phase 5 editor is missing " + token);
}

const rust = readFileSync("src-tauri/src/lib.rs", "utf8");
for (const command of ["get_authoring_v2", "apply_authoring_v2_patches"]) {
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

console.log("Phase 5 structured editor verification passed: schema-driven editor, source issue rail, shared slots, recovery contract, and V1 safety boundary are present.");
