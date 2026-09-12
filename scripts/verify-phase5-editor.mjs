import { existsSync, readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

// 2026-09-12：本脚本原先对 `src/pages/StructuredAuthoringEditorV2.tsx`、`src/editor/authoringTiptap.tsx`、
// `src/pages/ExportPage.tsx`、`src/pages/ImportWizard.tsx` 做 token 断言。这四个文件属旧世代页面，
// 已按当前计划（§16 逐文件改造清单 / §20 删除清单）退休并删除，针对它们的断言随之退休。
// 保留的断言全部指向**仍然存在**的文件：共享 ExamCanvas、原位文本编辑器、EditorCommandV1、
// authoringV2Patches、runtimeViewModelV2 与 Rust 侧安全契约。

const requiredFiles = [
  "src/exam-canvas/ExamCanvas.tsx",
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

// ExamCanvas 在产品收敛阶段从 src/components/ExamCanvasV2.tsx 迁到 src/exam-canvas/ExamCanvas.tsx。
const examCanvas = readFileSync("src/exam-canvas/ExamCanvas.tsx", "utf8");
for (const token of ["buildReadingInteractionModelV2", "buildRuntimeViewModelV2", "exam-canvas-v2", "v2-passage-pane", "v2-question-pane", "v2-response-group", "v2-slot-question", "InlineTextEditor", "onTextCommand", "expectedText", "onTextChange", "onAnswerChange", "onStructureAction", "resolveAuthoringAssetPreview", "table.row.add", "option.add", "answer-slot.insert"]) {
  if (!examCanvas.includes(token)) throw new Error("Phase 5 shared ExamCanvas contract is missing " + token);
}

const patches = readFileSync("src/services/authoringV2Patches.ts", "utf8");
for (const token of ["ensureAnswerSlotsRemain", "AUTHORING_PATCH_ANSWER_SLOT_LOSS", "allowAnswerSlotRemoval", "restoreProvenanceStatus"]) {
  if (!patches.includes(token)) throw new Error("Phase 5 patch safety contract is missing " + token);
}

const runtimeModel = readFileSync("src/services/runtimeViewModelV2.ts", "utf8");
for (const token of ["RuntimeViewModelV2", "questionOrder", "answerSlots", "assets"]) {
  if (!runtimeModel.includes(token)) throw new Error("Phase 5 runtime projection is missing " + token);
}

// 产品要求变更（简化/双路识别/WYSIWYG 计划 §9.3）：原位文本编辑不再使用裸 contentEditable +
// document.execCommand。原因是中文输入法 composition 与 rerender 冲突、光标跳动、粘贴富文本污染、
// React 状态与 DOM 分叉、浏览器 undo 与应用 undo 不一致、blur 前崩溃丢内容。
// 现在的契约是继承字体的 auto-size textarea（InlineTextEditor）+ onTextCommand/expectedText 乐观并发。
// 这两条负向断言防止旧实现被无意恢复。
for (const banned of ["contentEditable", "document.execCommand"]) {
  if (examCanvas.includes(banned)) {
    throw new Error("ExamCanvas must not reintroduce " + banned + " (see plan section 9.3)");
  }
}
const inlineEditor = readFileSync("src/exam-canvas/editors/InlineTextEditor.tsx", "utf8");
for (const token of ["onCompositionStart", "onCompositionEnd", "clipboardData.getData(\"text/plain\")", "aria-label"]) {
  if (!inlineEditor.includes(token)) throw new Error("Inline text editor IME/paste contract is missing " + token);
}
const editorCommands = readFileSync("src/exam-canvas/editorCommands.ts", "utf8");
for (const token of ["set_text", "expectedText", "codePointLength", "EditorCommandConflictError", "Array.from(text).length"]) {
  if (!editorCommands.includes(token)) throw new Error("EditorCommandV1 contract is missing " + token);
}

const rustAuthoring = readFileSync("src-tauri/src/authoring_v2_commands.rs", "utf8");
for (const token of ["AUTHORING_PATCH_ANSWER_SLOT_LOSS", "phase5-export.lock", "AuthoringV2ExportJournalV1", "remove_dir_all", "preserve_provenance", "setOptionBank", "insertAnswerSlot", "deleteAnswerSlot", "resolve_authoring_asset_preview_core"]) {
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

console.log("Phase 5 structured editor verification passed: shared ExamCanvas, structural patches, source issue rail, shared slots, recovery/history contract, V2 export, and V1 safety boundary are present. (Page-level assertions on the retired StructuredAuthoringEditorV2/ExportPage/ImportWizard/authoringTiptap files were retired with those files on 2026-09-12.)");
