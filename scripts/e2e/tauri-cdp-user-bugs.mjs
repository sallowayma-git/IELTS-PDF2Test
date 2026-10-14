/** Real Tauri UI import, controlled provider, heading drag and persisted answer regression. */
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import { DatabaseSync } from "node:sqlite";
import { assertBuildFresh, launchTauriAppCdp, repoRoot, sleep, sha256File } from "./lib/tauri-cdp-harness.mjs";

const release = process.argv.includes("--release");
const exePath = path.join(repoRoot, `src-tauri/target/${release ? "release" : "debug"}/ielts-author-studio.exe`);
const fixture = path.join(repoRoot, "tmp/pdfs/user-bugs-headings.pdf");
const runDir = path.join(repoRoot, "artifacts/e2e-cdp", `run-user-bugs-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const appRoot = path.join(runDir, "appdata/data");
const report = { fixture, fixtureSha256: sha256File(fixture), exePath, buildMode: release ? "release" : "debug", runDir, channel: "WebView2 CDP; sandbox and GPU unchanged", assertions: [], calls: [], verdict: "failed" };
let session, candidate, malformedSent = false;
function check(name, ok, detail) {
  report.assertions.push({ name, ok: Boolean(ok), detail });
  console.log(`[user-bugs] ${ok ? "PASS" : "FAIL"} ${name}`);
  if (!ok) throw new Error(`${name}: ${JSON.stringify(detail)}`);
}
const server = http.createServer(async (request, response) => {
  try {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks).toString());
    const text = body.messages.flatMap(message => typeof message.content === "string" ? [message.content] : message.content.map(part => part.text ?? "")).join("\n");
    let output, kind;
    if (text.includes("Response mode: missing-field patch")) {
      kind = "candidate-patch";
      output = { missingFields: { "/taskGroups/0/taskId": candidate.taskGroups[0].taskId } };
    } else if (text.includes("extracting the answer key")) {
      kind = "answer-page";
      output = { answers: { "1": "i", "2": "ii" }, confidence: 1, warnings: [], evidence: [
        { questionNumber: "1", pageIndex: 2, quote: "1 i" }, { questionNumber: "2", pageIndex: 2, quote: "2 ii" }
      ] };
    } else if (text.includes("submit_batch_decisions")) {
      kind = "source-review";
      const data = JSON.parse(text.slice(text.lastIndexOf("Input JSON:") + "Input JSON:".length));
      output = { callId: "controlled-review", tool: "submit_batch_decisions", arguments: { decisions: (data.context.differences ?? []).map(difference => ({ decisionId: difference.decisionId, choice: "Unknown", reason: "Controlled test does not decide ambiguous source differences." })) } };
    } else {
      kind = "candidate";
      output = structuredClone(candidate);
      if (!malformedSent) { delete output.taskGroups[0].taskId; malformedSent = true; }
    }
    report.calls.push({ kind, requestBytes: chunks.reduce((sum, chunk) => sum + chunk.length, 0) });
    response.writeHead(200, { "Content-Type": "application/json" });
    // No input/output split: the next request must still run and be budgeted.
    response.end(JSON.stringify({ choices: [{ message: { content: JSON.stringify(output) }, finish_reason: "stop" }], usage: { total_tokens: 100 } }));
  } catch (error) {
    response.writeHead(500, { "Content-Type": "application/json" });
    response.end(JSON.stringify({ error: String(error) }));
  }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const port = server.address().port;
async function invoke(command, args = {}) {
  const result = await session.invoke(command, args);
  if (result?.ok === false) throw new Error(`${command}:${result.error}`);
  return result?.value ?? result;
}
const workspace = itemId => invoke("get_workspace_item", { itemId });
async function waitTerminal(itemId) {
  const deadline = Date.now() + 240000;
  while (Date.now() < deadline) {
    const file = path.join(appRoot, "authoring_hub.db");
    const db = new DatabaseSync(file, { readOnly: true });
    db.exec("PRAGMA busy_timeout=5000");
    const state = db.prepare("SELECT stage,cloud_status,lease_owner FROM processing_jobs_v2 WHERE id=?").get(itemId);
    db.close();
    if (state && ["ready_for_review", "failed", "cancelled"].includes(state.stage) && !state.lease_owner) return state;
    if (session.exitCode() !== null) throw new Error(`app exited while checking file: ${session.exitCode()}`);
    await sleep(500);
  }
  throw new Error("file checking did not finish");
}
async function importUi() {
  const before = await session.evaluate(`Array.from(document.querySelectorAll('[data-testid="library-row"]'),row=>row.dataset.itemId)`);
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000 });
  await sleep(500); // Wait for the drawer's slide animation before the pointer click.
  await session.clickSelector('[data-testid="import-pick-folder"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000 });
  await session.clickSelector('[data-testid="import-start"]');
  const ids = await session.waitFor(`(() => { const ids=Array.from(document.querySelectorAll('[data-testid="library-row"]'),row=>row.dataset.itemId); return ids.find(id=>!${JSON.stringify(before)}.includes(id)); })()`, { timeoutMs: 30000 });
  return ids;
}
async function openWorkspace(itemId) {
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('.v2-passage-content')`, { timeoutMs: 30000 });
}
try {
  report.freshness = assertBuildFresh({ exePath });
  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  fs.copyFileSync(fixture, path.join(runDir, "pdfs/headings.pdf"));
  session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: "", appEnv: {
    ...(release ? {} : { EPIC8_PDFIUM_LIB: path.join(repoRoot, "src-tauri/lib/pdfium-windows/pdfium.dll") }),
    EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK: "1"
  } });
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000 });
  const localId = await importUi();
  report.localId = localId;
  await waitTerminal(localId);
  const local = (await workspace(localId)).ds;
  const group = local.taskGroups.find(group => group.taskType === "matching_headings");
  check("local-recognition-keeps-heading-targets", group && local.passage.paragraphMap.A && local.passage.paragraphMap.B, local.taskGroups.map(group => group.taskType));
  candidate = { taskGroups: structuredClone(local.taskGroups), answerSlots: structuredClone(local.answerSlots), answerKey: {}, passage: structuredClone(local.passage), warnings: [], unresolvedRegions: [], sourceCoverageNotes: [] };
  const map = candidate.passage.paragraphMap;
  const replaced = new Map();
  for (const [label, nodeId] of Object.entries(map)) {
    const freshId = `cloud-recovered-${label}`;
    replaced.set(nodeId, freshId); map[label] = freshId;
  }
  for (const node of candidate.passage.content) if (replaced.has(node.id)) node.id = replaced.get(node.id);
  for (const [slotId, slot] of Object.entries(candidate.answerSlots)) {
    if (replaced.has(slot.hostNodeId)) slot.hostNodeId = replaced.get(slot.hostNodeId);
    candidate.answerKey[slotId] = { kind: "unresolved" };
  }
  for (const response of candidate.taskGroups[0].responseGroups) delete response.cardinality;
  const profile = { profileId: "controlled-user-bugs", name: "Controlled", provider: "OpenAiCompatible", baseUrl: `http://127.0.0.1:${port}/v1`, model: "controlled-flash", apiKey: "controlled-test-key", temperature: 0, timeoutMs: 10000, forceJson: true, enabled: true };
  await invoke("save_llm_profile", { input: profile });
  await session.cdp.send("Page.reload");
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000 });
  const itemId = await importUi(); report.itemId = itemId;
  report.processing = await waitTerminal(itemId);
  const item = await workspace(itemId);
  const callRecords = fs.readFileSync(path.join(appRoot, "jobs", itemId, "llm-calls.jsonl"), "utf8").trim().split(/\r?\n/).map(JSON.parse);
  report.gatewayCalls = callRecords.map(call => ({ command: call.commandName, ok: call.ok, errorClass: call.errorClass, usage: call.usage }));
  check("missing-field-repair-works-without-full-diagnostics", report.calls.some(call => call.kind === "candidate-patch") && callRecords.some(call => call.commandName === "generate_authoring_candidate" && call.ok), report.gatewayCalls);
  const answerReport = JSON.parse(fs.readFileSync(path.join(appRoot, "jobs", itemId, "pipeline-report.json"), "utf8")).parser.visionAnswerExtraction;
  report.answerReport = answerReport;
  check("scan-answer-page-writes-both-source-answers-after-total-only-usage", answerReport.state === "succeeded" && answerReport.answerCount === 2 && item.ds.answerKey.q1.labels?.[0]?.toLowerCase() === "i" && item.ds.answerKey.q2.labels?.[0]?.toLowerCase() === "ii", { answerReport, key: item.ds.answerKey });
  await openWorkspace(itemId);
  const usageText = await session.evaluate(`document.querySelector('[data-testid="workspace-llm-usage"]')?.innerText`);
  check("total-only-usage-is-visible-in-workspace", usageText?.includes("服务商未提供完整输入/输出明细") && !usageText.includes("0 输入 + 0 输出"), usageText);
  const placement = await session.evaluate(`(() => { const passage=document.querySelector('.v2-passage-content'); return ['q1','q2'].map(id => { const target=passage.querySelector('[data-answer-drop-slot="'+id+'"]'); const next=target?.closest('.v2-answer-dropzone-wrap')?.nextElementSibling; return {id,count:document.querySelectorAll('[data-answer-drop-slot="'+id+'"]').length,beforeParagraph:next?.classList.contains('v2-paragraph')}; }); })()`);
  check("one-heading-dropzone-before-each-passage-paragraph", placement.every(entry => entry.count === 1 && entry.beforeParagraph), placement);
  const text = await session.evaluate(`document.querySelector('.v2-passage-content').innerText`);
  check("paragraph-ending-text-remains-visible", text.includes("final sentence of paragraph A") && text.includes("final sentence of paragraph B"), { textLength: text.length });
  report.authorScreenshot = await session.screenshot("heading-author");
  // Clicking a filled answer clears it; dragging from the pool must write it back.
  await session.clickSelector('[data-answer-drop-slot="q1"]');
  const optionLabel = item.ds.answerKey.q1.labels[0];
  const points = await session.evaluate(`(() => { const a=document.querySelector(${JSON.stringify(`[data-option-label="${optionLabel}"]`)}).getBoundingClientRect(); const b=document.querySelector('[data-answer-drop-slot="q1"]').getBoundingClientRect(); return {a:{x:a.x+a.width/2,y:a.y+a.height/2},b:{x:b.x+b.width/2,y:b.y+b.height/2}}; })()`);
  await session.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...points.a });
  await session.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", ...points.a, button: "left", buttons: 1, clickCount: 1 });
  for (let n = 1; n <= 8; n++) {
    await session.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: points.a.x+(points.b.x-points.a.x)*n/8, y: points.a.y+(points.b.y-points.a.y)*n/8, button: "left", buttons: 1 });
    await sleep(60);
  }
  await session.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...points.b, button: "left", buttons: 0, clickCount: 1 });
  await session.clickSelector('[data-testid="workspace-save"]');
  await sleep(1200);
  const saved = await workspace(itemId);
  check("real-drag-saves-the-heading-answer", saved.ds.answerKey.q1.labels?.[0]?.toLowerCase() === "i", saved.ds.answerKey.q1);
  await session.evaluate(`location.hash='#/library'`); await sleep(500); await openWorkspace(itemId);
  check("answer-survives-workspace-reopen", (await workspace(itemId)).ds.answerKey.q1.labels?.[0]?.toLowerCase() === "i");
  await session.clickSelector('[data-testid="workspace-mode-student"]');
  await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-student"]')`, { timeoutMs: 20000 });
  const preview = await session.evaluate(`Array.from(document.querySelectorAll('[data-testid="exam-canvas-v2-student"] [data-answer-drop-slot]'),target=>target.innerText)`);
  check("student-preview-starts-with-empty-heading-targets", preview.length === 2 && preview.every(text=>text.includes('Drop heading here')), preview);
  report.studentScreenshot = await session.screenshot("heading-student");
  await session.evaluate(`location.hash='#/settings'`);
  const visibleBuild = await session.waitFor(`document.querySelector('[data-testid="app-build-info"]')?.innerText`, { timeoutMs: 15000 });
  report.buildInfo = await invoke("get_app_build_info");
  check("visible-version-identifies-the-new-build", visibleBuild.includes(report.buildInfo.revision), visibleBuild);
  check("application-survives-import-and-review", session.exitCode() === null, session.exitCode());
  report.verdict = "passed";
} catch (error) {
  report.error = String(error?.stack ?? error);
  report.failureScreenshot = await session?.screenshot("failure").catch(() => null);
  console.error(report.error);
  process.exitCode = 1;
} finally {
  if (session) report.app = await session.close();
  await new Promise(resolve => server.close(resolve));
  fs.mkdirSync(runDir, { recursive: true });
  fs.writeFileSync(path.join(runDir, "report.json"), JSON.stringify(report, null, 2));
  console.log(`[user-bugs] ${report.verdict} ${path.join(runDir, "report.json")}`);
}
