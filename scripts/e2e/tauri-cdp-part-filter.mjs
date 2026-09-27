#!/usr/bin/env node
// C4b Part 筛选真实应用 e2e：导入两份不同 Passage 的卷子 → 卡片显示对应标签（P1/P2）→
// 点 P1 筛选只剩 P1 那份 → 手动把其中一份改成别的 Part，筛选结果随之变化、且持久化。
import fs from "node:fs";
import path from "node:path";
import {
  CDP_CHANNEL_LABEL, CDP_CHANNEL_NOTE, CannotRunError,
  assertBuildFresh, buildFreshReport, gitHead, launchTauriAppCdp, repoRoot, sha256File, sleep, writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `part-filter-${stamp}`);
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const extraArgs = "--no-sandbox --disable-gpu";
// 两份判为不同 Passage 的真实卷（content 层：chili=P1、organisational-design=P2）。
const fixtures = [
  path.join(repoRoot, "fixtures", "golden", "private-real", "chili-peppers.pdf"),
  path.join(repoRoot, "fixtures", "golden", "private-real", "organisational-design.pdf"),
];

function recordAssertion(report, id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

// 读题库行的 {id -> part 徽标文本}（卡片上的 library-row-part）。
async function rowParts(session) {
  return await session.evaluate(`(() => {
    const out = {};
    for (const row of document.querySelectorAll('[data-testid="library-row"]')) {
      const id = row.getAttribute('data-item-id');
      const badge = row.querySelector('[data-testid="library-row-part"]');
      out[id] = badge ? badge.textContent.trim() : null;
    }
    return out;
  })()`);
}
async function partViaList(session, itemId) {
  const r = await session.invoke("list_library_items", { includeDeleted: false });
  if (!r?.ok || !Array.isArray(r.value)) return null;
  return r.value.find((i) => i?.id === itemId)?.partLabel ?? null;
}
const report = {
  task: "part-label-and-filter",
  scope: "卡片显示 Part 标签 + 按 Part 筛选 + 手动改 Part 持久化（真实应用端到端）",
  channel: CDP_CHANNEL_LABEL, channelNote: CDP_CHANNEL_NOTE,
  evidenceLevel: "real-app-e2e (CDP automation channel)",
  assertions: [], consoleErrors: [], pageExceptions: [],
};

async function main() {
  for (const f of fixtures) if (!fs.existsSync(f)) throw new CannotRunError(`fixture 不存在（需私有 corpus）：${f}`);
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  report.identity = { exePath, exeSha256: sha256File(exePath), commit: gitHead(repoRoot), buildFresh: buildFreshReport(fresh) };
  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  for (const f of fixtures) fs.copyFileSync(f, path.join(runDir, "pdfs", path.basename(f)));

  const session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  let verdict = "failed";
  try {
    report.browserArgs = session.browserArgs;
    // 一次导入文件夹（含两份 PDF）。
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library" });
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
    await sleep(600);
    await session.clickSelector('[data-testid="import-pick-folder"]');
    await session.waitFor(`document.querySelectorAll('[data-testid="import-picked-files"] li').length >= 2`, { timeoutMs: 20000, label: "picked-2" });
    await session.clickSelector('[data-testid="import-start"]');
    // 等两行出现且各自 part 徽标被回填（识别 seed 后 list 惰性回填 part）。
    let parts = {};
    const deadline = Date.now() + 240000;
    while (Date.now() < deadline) {
      parts = await rowParts(session);
      const labels = Object.values(parts).filter(Boolean);
      if (Object.keys(parts).length >= 2 && labels.includes("P1") && labels.includes("P2")) break;
      await sleep(1500);
    }
    report.rowParts = parts;
    const p1Id = Object.keys(parts).find((id) => parts[id] === "P1");
    const p2Id = Object.keys(parts).find((id) => parts[id] === "P2");
    recordAssertion(report, "C4b-1 cards-show-part-labels", Boolean(p1Id && p2Id),
      `卡片 Part 徽标=${JSON.stringify(parts)}（需含一张 P1、一张 P2）`);
    if (!p1Id || !p2Id) throw new Error("两份卷未分别判为 P1/P2，无法继续筛选场景");

    // 点 P1 筛选 → 只剩 P1 那份。
    await session.clickSelector('[data-testid="library-part-chip-P1"]');
    await sleep(600);
    const afterFilter = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
    recordAssertion(report, "C4b-2 filter-P1-keeps-only-P1", afterFilter.length === 1 && afterFilter[0] === p1Id,
      `P1 筛选后可见行=${JSON.stringify(afterFilter)}（期望只剩 ${p1Id}）`);

    // 取消筛选，手动把 P2 那份改成 P3。
    await session.clickByText('全部').catch(() => {});
    await sleep(400);
    await session.evaluate(`(() => {
      const el = document.querySelector('[data-item-id="${p2Id}"] [data-testid="library-row-part-select"]');
      if (!el) return false;
      const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set;
      setter.call(el, 'P3'); el.dispatchEvent(new Event('change', { bubbles: true })); return true;
    })()`);
    // 等回写生效（list 刷新后该项 partLabel=P3）。
    let manual = null;
    const md = Date.now() + 15000;
    while (Date.now() < md) { manual = await partViaList(session, p2Id); if (manual === "P3") break; await sleep(500); }
    recordAssertion(report, "C4b-3 manual-set-part", manual === "P3", `手动改后 partLabel=${JSON.stringify(manual)}（期望 P3）`);

    // 筛选随手改的 Part 变化：这份从 P2 改成 P3 后，P3 筛选应只剩它。
    await session.clickByText('全部').catch(() => {});
    await sleep(300);
    await session.waitFor(`!!document.querySelector('[data-testid="library-part-chip-P3"]')`, { timeoutMs: 8000, label: "p3-chip" });
    await session.clickSelector('[data-testid="library-part-chip-P3"]');
    await sleep(600);
    const afterP3 = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
    recordAssertion(report, "C4b-3b filter-follows-manual-change", afterP3.length === 1 && afterP3[0] === p2Id,
      `改成 P3 后按 P3 筛选可见行=${JSON.stringify(afterP3)}（期望只剩 ${p2Id}）`);
    await session.clickByText('全部').catch(() => {});
    await sleep(300);

    // 持久化：重开题库（导航离开再回来），该项仍是 P3。
    await session.evaluate(`(() => { window.location.hash = "#/settings"; return true; })()`);
    await sleep(400);
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 20000, label: "library-reopen" });
    const persisted = await partViaList(session, p2Id);
    recordAssertion(report, "C4b-4 manual-part-persists", persisted === "P3", `重开后 partLabel=${JSON.stringify(persisted)}（期望 P3）`);

    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    verdict = report.assertions.every((a) => a.ok) ? "passed" : "failed";
  } finally {
    report.verdict = verdict;
    report.failedAssertions = report.assertions.filter((a) => !a.ok).map((a) => a.id);
    await session.close?.().catch?.(() => {});
  }
  const file = writeReport(runDir, report);
  console.log(`[part-filter] verdict=${report.verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
  console.log(`[part-filter] report=${file}`);
  process.exitCode = report.verdict === "passed" ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  report.verdict = cannotRun ? "cannot-run" : "failed";
  report.error = String(error?.message ?? error);
  try { writeReport(runDir, report); } catch {}
  console.error(`[part-filter] ${report.verdict}: ${report.error}`);
  process.exitCode = cannotRun ? 3 : 1;
});

