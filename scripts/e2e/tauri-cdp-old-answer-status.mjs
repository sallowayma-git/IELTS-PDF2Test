/** Read a copy of the user's old failure record through the new real UI. */
import fs from "node:fs";
import path from "node:path";
import { assertBuildFresh, launchTauriAppCdp, repoRoot } from "./lib/tauri-cdp-harness.mjs";
const userRoot = path.join(process.env.APPDATA, "com.ielts.author.studio");
const itemId = "import-20261014085721-e83e8fb2";
const exePath = path.join(repoRoot, "src-tauri/target/release/ielts-author-studio.exe");
const runDir = path.join(repoRoot, "artifacts/e2e-cdp", `run-old-answer-status-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const root = path.join(runDir, "appdata/data");
const report = { itemId, runDir, evidence: "Copied historical user job; real release UI; cloud profiles disabled", verdict: "failed" };
let session;
try {
  report.freshness = assertBuildFresh({ exePath });
  fs.mkdirSync(path.join(root, "config"), { recursive: true });
  fs.copyFileSync(path.join(userRoot, "authoring_hub.db"), path.join(root, "authoring_hub.db"));
  fs.cpSync(path.join(userRoot, "jobs", itemId), path.join(root, "jobs", itemId), { recursive: true });
  fs.writeFileSync(path.join(root, "config/llm-profiles.json"), "[]");
  session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: "" });
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000 });
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-answer-page-reason]')`, { timeoutMs: 30000 });
  report.notice = await session.evaluate(`(() => {const el=document.querySelector('[data-answer-page-reason]');return {state:el.dataset.answerPageState,reason:el.dataset.answerPageReason,text:el.innerText,retry:!!el.querySelector('button')};})()`);
  if (report.notice.state !== "not_executed" || report.notice.reason !== "usage_unavailable" || report.notice.text.includes("无法核验") || report.notice.retry) throw new Error(`Old failure was misrepresented: ${JSON.stringify(report.notice)}`);
  report.screenshot = await session.screenshot("old-answer-failure");
  report.verdict = "passed";
  console.log("[old-answer-status] PASS old budget stop is not shown as invalid answer content");
} catch (error) {
  report.error = String(error?.stack ?? error);
  console.error(report.error);
  process.exitCode = 1;
} finally {
  if (session) report.app = await session.close();
  fs.mkdirSync(runDir, { recursive: true });
  fs.writeFileSync(path.join(runDir, "report.json"), JSON.stringify(report, null, 2));
  console.log(`[old-answer-status] report=${path.join(runDir, "report.json")}`);
}
