/** One bounded real-provider import through the actual Tauri UI. Never print secrets. */
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { assertBuildFresh, launchTauriAppCdp, repoRoot, sleep, sha256File } from "./lib/tauri-cdp-harness.mjs";
const userRoot = path.join(process.env.APPDATA, "com.ielts.author.studio");
const fixture = process.argv[2] ?? "F:/workspace/IELTS Atlas/ReadingPractice/PDF/24. P1 - The Pearls 珍珠.pdf";
const exePath = path.join(repoRoot, "src-tauri/target/debug/ielts-author-studio.exe");
const runDir = path.join(repoRoot, "artifacts/e2e-cdp", `run-live-user-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const root = path.join(runDir, "appdata/data");
const originalProfiles = JSON.parse(fs.readFileSync(path.join(userRoot, "config/llm-profiles.json"), "utf8"));
const original = originalProfiles.find(profile => profile.enabled && profile.model === "deepseek-flash");
if (!original) throw new Error("No enabled deepseek-flash profile; no provider request made.");
const profile = { ...original, timeoutMs: 240000 };
const report = { fixture, fixtureSha256: sha256File(fixture), model: profile.model, timeoutMs: profile.timeoutMs, tokenBudget: 50000, evidence: "Real Tauri UI + existing DeepSeek provider", runDir, verdict: "failed" };
let session;
async function invoke(command, args = {}) {
  const result = await session.invoke(command, args);
  if (result?.ok === false) throw new Error(`${command}:${result.error}`);
  return result?.value ?? result;
}
try {
  report.freshness = assertBuildFresh({ exePath });
  fs.mkdirSync(path.join(root, "config"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  fs.writeFileSync(path.join(root, "config/llm-profiles.json"), JSON.stringify([profile]));
  if (profile.secretStorageBackend === "file") {
    fs.mkdirSync(path.join(root, "config/secrets"), { recursive: true });
    fs.copyFileSync(path.join(userRoot, "config/secrets", `${profile.profileId}.key`), path.join(root, "config/secrets", `${profile.profileId}.key`));
  }
  fs.copyFileSync(fixture, path.join(runDir, "pdfs/source.pdf"));
  session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: "", appEnv: {
    EPIC8_PDFIUM_LIB: path.join(repoRoot, "src-tauri/lib/pdfium-windows/pdfium.dll"),
    ...(profile.secretStorageBackend === "file" ? { EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK: "1" } : {})
  } });
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000 });
  await session.evaluate(`window.localStorage.setItem('ielts-author-studio.app-settings.v1',JSON.stringify({cloudTokenBudget:50000}))`);
  await session.cdp.send("Page.reload");
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000 });
  await session.clickSelector('[data-testid="library-import"]');
  await session.clickSelector('[data-testid="import-pick-folder"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000 });
  await session.clickSelector('[data-testid="import-start"]');
  const itemId = await session.waitFor(`document.querySelector('[data-testid="library-row"]')?.dataset.itemId`, { timeoutMs: 30000 });
  report.itemId = itemId;
  console.log(`[live-user] imported ${itemId}, model=${profile.model}, timeout=240s, budget=50000`);
  const deadline = Date.now() + 900000;
  while (Date.now() < deadline) {
    const db = new DatabaseSync(path.join(root, "authoring_hub.db"), { readOnly: true });
    db.exec("PRAGMA busy_timeout=5000");
    const state = db.prepare("SELECT stage,cloud_status,lease_owner FROM processing_jobs_v2 WHERE id=?").get(itemId);
    db.close();
    if (session.exitCode() !== null) throw new Error(`Application exited during checking: ${session.exitCode()}`);
    if (state && ["ready_for_review", "failed", "cancelled"].includes(state.stage) && !state.lease_owner) { report.processing = state; break; }
    await sleep(1000);
  }
  if (!report.processing) {
    await invoke("cancel_processing", { itemId }).catch(() => {});
    throw new Error("Checking exceeded 15-minute limit; cancelled isolated job.");
  }
  const item = await invoke("get_workspace_item", { itemId });
  const jobDir = path.join(root, "jobs", itemId);
  report.usage = JSON.parse(fs.readFileSync(path.join(jobDir, "llm-token-usage-total.json"), "utf8"));
  report.calls = fs.readFileSync(path.join(jobDir, "llm-calls.jsonl"), "utf8").trim().split(/\r?\n/).map(JSON.parse).map(call => ({ command: call.commandName, ok: call.ok, latencyMs: call.latencyMs, httpStatus: call.httpStatus, errorClass: call.errorClass, error: call.error?.slice(0, 800), usage: call.usage, finishReason: call.finishReason }));
  report.answers = JSON.parse(fs.readFileSync(path.join(jobDir, "pipeline-report.json"), "utf8")).parser.visionAnswerExtraction;
  report.slotCount = Object.keys(item.ds.answerSlots).length;
  report.resolvedAnswerCount = Object.values(item.ds.answerKey).filter(answer => answer.kind !== "unresolved").length;
  report.paragraphCount = item.ds.passage?.content?.length;
  report.issues = item.issues;
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('.v2-passage-content')`, { timeoutMs: 30000 });
  report.screenshot = await session.screenshot("live-provider-result");
  report.buildInfo = await invoke("get_app_build_info");
  report.crashed = false;
  const recognitionSucceeded = report.calls.some(call => call.command === "generate_authoring_candidate" && call.ok) && report.answers?.state === "succeeded";
  report.verdict = recognitionSucceeded
    ? report.processing.cloud_status === "failed" ? "candidate_and_answers_passed_review_incomplete" : "passed"
    : "completed_with_cloud_failure";
  console.log(`[live-user] ${report.verdict}: slots=${report.slotCount}, answers=${report.resolvedAnswerCount}, tokens=${report.usage.promptTokens+report.usage.completionTokens+(report.usage.unattributedTokens??0)}`);
} catch (error) {
  report.error = String(error?.stack ?? error);
  console.error(report.error);
  process.exitCode = 1;
} finally {
  if (session) report.app = await session.close();
  const temporarySecret = path.join(root, "config/secrets", `${profile.profileId}.key`);
  if (fs.existsSync(temporarySecret)) fs.unlinkSync(temporarySecret);
  fs.mkdirSync(runDir, { recursive: true });
  fs.writeFileSync(path.join(runDir, "report.json"), JSON.stringify(report, null, 2));
  console.log(`[live-user] report=${path.join(runDir, "report.json")}`);
}
