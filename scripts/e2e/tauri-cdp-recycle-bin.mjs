#!/usr/bin/env node
// C2 回收站真实应用 e2e：导入 → 删除进回收站 → 永久删除（二次确认）→ 断言列表消失、
// get_workspace_item 查不到；再导入两份 → 都进回收站 → 清空回收站 → 断言清空。
import fs from "node:fs";
import path from "node:path";
import {
  CDP_CHANNEL_LABEL, CDP_CHANNEL_NOTE, CannotRunError,
  assertBuildFresh, buildFreshReport, gitHead, launchTauriAppCdp, repoRoot, sha256File, sleep, writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `recycle-bin-${stamp}`);
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const fixture = path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf");
const extraArgs = "--no-sandbox --disable-gpu";
// 应用把数据根指向 <runDir>/appdata/data（见 harness 的 PDF2TEST_AUTOMATION_DATA_DIR）；
// 导入流程里 job_id 与库条目 id 同值，磁盘 job 目录即 <data>/jobs/<itemId>。
const jobDir = (itemId) => path.join(runDir, "appdata", "data", "jobs", itemId);

function recordAssertion(report, id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

async function libraryRowIds(session) {
  return (await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`)) ?? [];
}

const ACTIVE_STAGES = new Set(["queued", "running", "local_recognition", "cloud_recognition", "reconciling"]);
async function readStage(session, itemId) {
  const r = await session.invoke("list_library_items", { includeDeleted: true });
  if (!r?.ok || !Array.isArray(r.value)) return null;
  return r.value.find((i) => i?.id === itemId)?.processing?.stage ?? null;
}
// 活动阶段的条目仍被 worker 回写、删了会复活，故删前先等它落终态，避免与 worker 竞争。
async function waitUntilSettled(session, itemId, timeoutMs = 180000) {
  const deadline = Date.now() + timeoutMs;
  let stage = await readStage(session, itemId);
  while (Date.now() < deadline && (stage === null || ACTIVE_STAGES.has(stage))) {
    await sleep(1000);
    stage = await readStage(session, itemId);
  }
  return stage;
}

async function importOne(session, report) {
  await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library" });
  const before = await libraryRowIds(session);
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
  await sleep(600);
  await session.clickSelector('[data-testid="import-pick-folder"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked" });
  await session.clickSelector('[data-testid="import-start"]');
  let itemId = null;
  const deadline = Date.now() + 90000;
  while (Date.now() < deadline && !itemId) {
    const ids = await libraryRowIds(session);
    itemId = ids.find((id) => !before.includes(id)) ?? null;
    if (!itemId) await sleep(500);
  }
  if (!itemId) throw new Error("导入后未出现新的题库行");
  const settledStage = await waitUntilSettled(session, itemId);
  (report.settledStages ??= {})[itemId] = settledStage;
  if (settledStage === null || ACTIVE_STAGES.has(settledStage))
    throw new Error(`条目 ${itemId} 识别未在超时内落终态（stage=${settledStage}），无法可靠测试删除`);
  return itemId;
}

async function moveToTrash(session, itemId) {
  // 活动行里唯一的 .danger 就是「删除」（进回收站）。
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-actions .danger`);
  await sleep(500);
}

async function itemFound(session, itemId) {
  const r = await session.invoke("get_workspace_item", { itemId });
  return Boolean(r?.ok && r.value);
}
const report = {
  task: "recycle-bin-permanent-delete-and-empty",
  scope: "回收站永久删除 + 清空回收站（真实应用端到端）",
  channel: CDP_CHANNEL_LABEL, channelNote: CDP_CHANNEL_NOTE,
  evidenceLevel: "real-app-e2e (CDP automation channel)",
  fixturePath: fixture, fixtureSha256: sha256File(fixture),
  assertions: [], consoleErrors: [], pageExceptions: [],
};

async function main() {
  if (!fs.existsSync(fixture)) throw new CannotRunError(`fixture 不存在：${fixture}`);
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  report.identity = { exePath, exeSha256: sha256File(exePath), commit: gitHead(repoRoot), buildFresh: buildFreshReport(fresh) };
  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  fs.copyFileSync(fixture, path.join(runDir, "pdfs", path.basename(fixture)));

  const session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  let verdict = "failed";
  try {
    report.browserArgs = session.browserArgs;
    // 二次确认是 window.confirm（原生框，CDP 点不了）——覆盖成恒真。
    await session.evaluate(`(() => { window.confirm = () => true; return true; })()`);

    const id1 = await importOne(session, report);
    report.identity.itemId = id1;
    const jobDirExistedBefore = fs.existsSync(jobDir(id1));
    await moveToTrash(session, id1);
    await session.clickSelector('[data-testid="library-tab-trash"]');
    await session.waitFor(`!!document.querySelector('[data-item-id="${id1}"] [data-testid="library-row-permanent-delete"]')`, { timeoutMs: 15000, label: "in-trash" });
    await session.clickSelector(`[data-item-id="${id1}"] [data-testid="library-row-permanent-delete"]`);
    const goneDeadline = Date.now() + 20000;
    let goneFromList = false;
    while (Date.now() < goneDeadline && !goneFromList) {
      const ids = await libraryRowIds(session);
      goneFromList = !ids.includes(id1);
      if (!goneFromList) await sleep(400);
    }
    const stillFound = await itemFound(session, id1);
    // 删前必须已落盘 job 目录（否则判据落空），删后必须清掉，确保不留磁盘孤儿。
    const dirDeadline = Date.now() + 20000;
    while (Date.now() < dirDeadline && fs.existsSync(jobDir(id1))) await sleep(400);
    const jobDirGone = !fs.existsSync(jobDir(id1));
    recordAssertion(report, "C2-1 permanent-delete-removes-item",
      goneFromList && !stillFound && jobDirExistedBefore && jobDirGone,
      `列表已无该条=${goneFromList}；get_workspace_item 查不到=${!stillFound}；` +
      `job 目录删前存在=${jobDirExistedBefore}（必须为真）、删后已清=${jobDirGone}`);

    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 20000, label: "library-again" });
    const id2 = await importOne(session, report);
    const id3 = await importOne(session, report);
    await moveToTrash(session, id2);
    await moveToTrash(session, id3);
    await session.clickSelector('[data-testid="library-tab-trash"]');
    await session.waitFor(`!!document.querySelector('[data-testid="library-empty-trash"]')`, { timeoutMs: 15000, label: "empty-trash-btn" });
    const trashCountBefore = (await libraryRowIds(session)).filter((id) => id === id2 || id === id3).length;
    await session.clickSelector('[data-testid="library-empty-trash"]');
    const emptyDeadline = Date.now() + 20000;
    let cleared = false;
    while (Date.now() < emptyDeadline && !cleared) {
      const ids = await libraryRowIds(session);
      cleared = !ids.includes(id2) && !ids.includes(id3);
      if (!cleared) await sleep(400);
    }
    const found2 = await itemFound(session, id2);
    const found3 = await itemFound(session, id3);
    recordAssertion(report, "C2-2 empty-recycle-bin-clears-all", trashCountBefore === 2 && cleared && !found2 && !found3,
      `清空前回收站含两条=${trashCountBefore === 2}；清空后列表无二者=${cleared}；get_workspace_item 均查不到=${!found2 && !found3}`);

    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    report.pageExceptions = session.cdp.events
      .filter((e) => e.method === "Runtime.exceptionThrown")
      .map((e) => (e.params?.exceptionDetails?.exception?.description ?? e.params?.exceptionDetails?.text ?? "").slice(0, 400));
    verdict = report.assertions.every((a) => a.ok) ? "passed" : "failed";
  } finally {
    report.verdict = verdict;
    report.failedAssertions = report.assertions.filter((a) => !a.ok).map((a) => a.id);
    await session.close?.().catch?.(() => {});
  }
  const file = writeReport(runDir, report);
  console.log(`[recycle-bin] verdict=${report.verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
  console.log(`[recycle-bin] report=${file}`);
  process.exitCode = report.verdict === "passed" ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  report.verdict = cannotRun ? "cannot-run" : "failed";
  report.error = String(error?.message ?? error);
  try { writeReport(runDir, report); } catch {}
  console.error(`[recycle-bin] ${report.verdict}: ${report.error}`);
  process.exitCode = cannotRun ? 3 : 1;
});

