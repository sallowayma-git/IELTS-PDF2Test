/** Recheck an isolated saved real-provider draft; never call the provider. */
import fs from "node:fs";
import path from "node:path";
import { assertBuildFresh, launchTauriAppCdp, repoRoot } from "./lib/tauri-cdp-harness.mjs";

const originalRun = path.join(repoRoot, "artifacts/e2e-cdp/run-live-user-2026-10-14T10-06-18-886Z");
const itemId = "import-20261014100624-6c5c3ec7";
const exePath = path.join(repoRoot, "src-tauri/target/release/ielts-author-studio.exe");
const runDir = path.join(repoRoot, "artifacts/e2e-cdp", `run-saved-review-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const root = path.join(runDir, "appdata/data");
const report = { originalRun, itemId, runDir, evidence: "Saved real DeepSeek draft; release Tauri IPC + UI; no provider calls", assertions: [], verdict: "failed" };
let session;
function check(name, ok, detail) {
  report.assertions.push({ name, ok: Boolean(ok), detail });
  console.log(`[saved-review] ${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) throw new Error(`${name}: ${JSON.stringify(detail)}`);
}
async function invoke(command, args = {}) {
  const result = await session.invoke(command, args);
  if (result?.ok === false) throw new Error(`${command}:${result.error}`);
  return result?.value ?? result;
}
try {
  report.freshness = assertBuildFresh({ exePath });
  fs.cpSync(path.join(originalRun, "appdata/data"), root, { recursive: true, filter: file => !file.includes(`${path.sep}secrets${path.sep}`) });
  // Disable all cloud profiles in this copy before starting the application.
  fs.writeFileSync(path.join(root, "config/llm-profiles.json"), "[]");
  session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: "" });
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000 });
  const before = await invoke("get_workspace_item", { itemId });
  report.beforeIssues = before.ds.quality.issues.map(({ code, targetId }) => ({ code, targetId }));
  const instruction = before.ds.taskGroups.find(group => group.taskId === "group-2").instructions[0].children.find(node => node.type === "text");
  // Save the exact existing instruction through the editor command handler,
  // causing the same signature/quality refresh as a user instruction edit.
  await invoke("apply_editor_commands", { input: { itemId, baseVersion: before.editVersion, commands: [{
    op: "replaceText", nodeId: instruction.id, from: 0, to: [...instruction.text].length, text: instruction.text
  }] } });
  const after = await invoke("get_workspace_item", { itemId });
  report.afterIssues = after.ds.quality.issues.map(({ code, targetId }) => ({ code, targetId }));
  report.compilerProbe = after.ds.quality.compilerProbes.v2Runtime;
  check("A-K-selection-completion-does-not-require-word-limit", !report.afterIssues.some(issue => issue.code === "WORD_LIMIT_UNPARSED"));
  check("paragraph-information-runtime-compiles-from-source-map", report.compilerProbe.status === "passed", report.compilerProbe);
  check("existing-thirteen-answers-preserved", Object.values(after.ds.answerKey).filter(answer => answer.kind !== "unresolved").length === 13);
  report.preflight = await invoke("get_publish_preflight", { jobId: itemId });
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('.v2-passage-content')`, { timeoutMs: 30000 });
  await session.clickSelector('[data-testid="workspace-mode-student"]');
  await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-student"]') || !!document.querySelector('[data-testid="workspace-preview-error"]')`, { timeoutMs: 20000 });
  const preview = await session.evaluate(`({error:document.querySelector('[data-testid="workspace-preview-error"]')?.innerText ?? null, choices:['q1','q2','q3','q4'].map(id=>Array.from(document.querySelectorAll('[data-testid="exam-canvas-v2-student"] input[type="radio"][name="'+id+'"]'),input=>input.value))})`);
  check("real-Pearls-student-preview-renders-paragraph-choices", !preview.error && preview.choices.every(labels=>labels.join(',')==='A,B,C,D,E,F,G'), preview);
  const unanswered = await session.evaluate(`Array.from(document.querySelectorAll('[data-testid="exam-canvas-v2-student"] input'),input=>input.type==='radio' ? input.checked : input.value).filter(Boolean).length`);
  check("real-Pearls-student-preview-does-not-leak-source-answers", unanswered === 0, unanswered);
  await session.clickSelector('[data-testid="exam-canvas-v2-student"] input[type="radio"][name="q1"][value="A"]');
  check("real-Pearls-student-preview-can-answer-without-changing-saved-key", await session.evaluate(`document.querySelector('[data-testid="exam-canvas-v2-student"] input[type="radio"][name="q1"][value="A"]').checked`) && JSON.stringify((await invoke("get_workspace_item", { itemId })).ds.answerKey) === JSON.stringify(after.ds.answerKey));
  report.screenshot = await session.screenshot("saved-real-draft");
  report.buildInfo = await invoke("get_app_build_info");
  check("release-application-survives-draft-check", session.exitCode() === null);
  report.verdict = "passed";
} catch (error) {
  report.error = String(error?.stack ?? error);
  console.error(report.error);
  process.exitCode = 1;
} finally {
  if (session) report.app = await session.close();
  fs.mkdirSync(runDir, { recursive: true });
  fs.writeFileSync(path.join(runDir, "report.json"), JSON.stringify(report, null, 2));
  console.log(`[saved-review] report=${path.join(runDir, "report.json")}`);
}
