#!/usr/bin/env node
// Real Tauri UI + Rust + persisted SQLite + NAS export. Existing real import is
// copied into an isolated profile; no recognition service or user DB is mutated.
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { DatabaseSync } from "node:sqlite";
import { assertBuildFresh, buildFreshReport, launchTauriAppCdp, repoRoot, writeReport } from "./lib/tauri-cdp-harness.mjs";

const runDir = path.join(repoRoot, "artifacts/e2e-cdp", `part-assignment-sync-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const exePath = path.join(repoRoot, "src-tauri/target/debug/ielts-author-studio.exe");
const storeArg = process.argv.indexOf("--store");
const store = storeArg >= 0 ? path.resolve(process.argv[storeArg + 1]) : path.join(repoRoot, "artifacts/e2e-cdp/run-cloud-direct-2026-10-14T22-55-23-591Z/online-direct/appdata/data");
const itemArg = process.argv.indexOf("--item");
const itemId = itemArg >= 0 ? process.argv[itemArg + 1] : "import-20261014225531-d35bb0b9";
const report = { task: "part-assignment-sync", evidenceLevel: "real Tauri UI on isolated copy of a real imported reading exam", diagnosticRun: true, browserArgs: "--no-sandbox --disable-gpu", sourceStore: store, itemId, assertions: [], screenshots: [], verdict: "failed" };
let session;
const assert = (id, ok, detail) => {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[part-sync] ${ok ? "PASS" : "FAIL"} ${id}: ${JSON.stringify(detail)}`);
  if (!ok) throw new Error(id);
};
async function invoke(name, args) {
  const result = await session.invoke(name, args);
  if (!result?.ok) throw new Error(`${name}: ${JSON.stringify(result)}`);
  return result.value;
}
async function select(selector, value) {
  assert("select-control-present", await session.evaluate(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)}); if (!el || el.disabled) return false;
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event('change', { bubbles:true })); return true;
  })()`), { selector, value });
}
function filesNamed(dir, name) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => entry.isDirectory()
    ? filesNamed(path.join(dir, entry.name), name) : entry.name === name ? [path.join(dir, entry.name)] : []);
}
async function main() {
  report.build = buildFreshReport(assertBuildFresh({ exePath }));
  const data = path.join(runDir, "appdata/data");
  fs.mkdirSync(data, { recursive: true });
  fs.cpSync(store, data, { recursive: true });
  const db = new DatabaseSync(path.join(data, "authoring_hub.db"));
  // Quiesce historical processing jobs in the test copy; this scenario edits
  // completed documents and must never resume an old network recognition run.
  db.exec("UPDATE processing_jobs_v2 SET stage='cancelled',cloud_status='cancelled',cancel_requested_at='2026-10-16T00:00:00Z'");
  // Exercise the historical no-job notification path, rather than relying on
  // a durable processing row that may already have been pruned in real profiles.
  db.prepare("DELETE FROM processing_jobs_v2 WHERE library_item_id=?").run(itemId);
  db.close();
  session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: report.browserArgs });
  await session.evaluate(`window.location.hash='#/library'`);
  await session.waitFor(`!!document.querySelector('[data-item-id="${itemId}"]')`, { timeoutMs: 30000 });
  let original = await invoke("get_workspace_item", { itemId });
  report.original = { version: original.editVersion, category: original.ds.exam.category, examId: original.ds.exam.examId, title: original.ds.exam.title };
  await select(`[data-item-id="${itemId}"] [data-testid="library-row-part-select"]`, "P2");
  await session.waitFor(`document.querySelector('[data-item-id="${itemId}"] [data-testid="library-row-part"]')?.textContent==='P2'`, { timeoutMs: 15000 });
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.value==='P2'`, { timeoutMs: 30000 });
  let saved = await invoke("get_workspace_item", { itemId });
  assert("library-change-reaches-canonical-with-version", saved.ds.exam.category === "P2" && saved.editVersion > original.editVersion, { version: saved.editVersion, category: saved.ds.exam.category });
  await invoke("set_library_item_part", { itemId, label: "P1" });
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.value==='P1'`, { timeoutMs: 10000 });
  await invoke("set_library_item_part", { itemId, label: "P2" });
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.value==='P2'`, { timeoutMs: 10000 });
  assert("open-editor-observes-remote-part-version-notifications", true, "clean editor followed library command P1 → P2");
  const editablePart = await session.evaluate(`({count:document.querySelectorAll('.workspace-part-control select').length,badges:document.querySelectorAll('[data-testid="workspace-part-badge"]').length,label:document.querySelector('[data-testid="workspace-part-select"]')?.getAttribute('aria-label'),disabled:document.querySelector('[data-testid="workspace-part-select"]')?.disabled})`);
  assert("editable-header-has-one-accessible-Part-control", editablePart.count === 1 && editablePart.badges === 0 && editablePart.label === "Part 归属" && editablePart.disabled === false, editablePart);
  report.screenshots.push(await session.screenshot("editor-P2"));
  // Controlled cloud-lock state in this isolated DB (no recognition/network run).
  // Reload through real navigation so the component reads the actual persisted
  // processing state, rather than simulating a disabled control in the DOM.
  const lockDb = new DatabaseSync(path.join(data,"authoring_hub.db"));
  lockDb.prepare(`INSERT INTO processing_jobs_v2
    (id,library_item_id,source_asset_id,stage,local_status,cloud_status,reconcile_status,
     progress_json,actionable_count,retry_count,created_at,updated_at)
    VALUES (?,?,?,'cloud_recognition','succeeded','running','not_started','{}',0,0,?,?)`)
    .run("part-sync-controlled-lock",itemId,"part-sync-controlled-source",new Date().toISOString(),new Date().toISOString());
  lockDb.close();
  await session.clickSelector('[data-testid="workspace-back"]');
  await session.waitFor(`!!document.querySelector('[data-item-id="${itemId}"] .library-row-main')`);
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.disabled===true`,{timeoutMs:15000});
  const lockedPart = await session.evaluate(`({value:document.querySelector('[data-testid="workspace-part-select"]')?.value,count:document.querySelectorAll('.workspace-part-control select').length,badges:document.querySelectorAll('[data-testid="workspace-part-badge"]').length,banner:!!document.querySelector('[data-testid="workspace-cloud-review-banner"]')})`);
  assert("locked-header-has-one-disabled-Part-control", lockedPart.value === "P2" && lockedPart.count === 1 && lockedPart.badges === 0 && lockedPart.banner, lockedPart);
  report.screenshots.push(await session.screenshot("editor-P2-locked-controlled-state"));
  const unlockDb = new DatabaseSync(path.join(data,"authoring_hub.db"));
  unlockDb.prepare("DELETE FROM processing_jobs_v2 WHERE id=?").run("part-sync-controlled-lock");
  unlockDb.close();
  await session.clickSelector('[data-testid="workspace-back"]');
  await session.waitFor(`!!document.querySelector('[data-item-id="${itemId}"] .library-row-main')`);
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.disabled===false`,{timeoutMs:15000});
  await session.clickSelector('[data-testid="workspace-mode-student"]');
  await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-student"]') || !!document.querySelector('[data-testid="workspace-preview-error"]')`, { timeoutMs: 30000 });
  const preview = await session.evaluate(`({error:document.querySelector('[data-testid="workspace-preview-error"]')?.innerText ?? null, nav:document.querySelector('[data-testid="question-nav"] .part-nav-name')?.textContent ?? null, subheader:document.querySelector('.workspace-sub-header-label')?.textContent})`);
  assert("student-preview-renders-Part-2", !preview.error && preview.subheader === "READING · Part 2" && preview.nav === "Part 2", preview);
  report.screenshots.push(await session.screenshot("student-preview-P2"));
  const nas = path.join(runDir, "nas-library");
  await session.evaluate(`(() => {
    const key='ielts-author-studio.app-settings.v1'; let settings={};
    try { settings=JSON.parse(localStorage.getItem(key)??'{}'); } catch {}
    settings.nasDestination=${JSON.stringify(nas)}; localStorage.setItem(key,JSON.stringify(settings));
    localStorage.setItem('ielts-author-studio.confirmed-nas-export-dir.v1',${JSON.stringify(nas)});
    return true;
  })()`);
  await session.clickSelector('[data-testid="workspace-publish"]');
  await session.waitFor(`!!document.querySelector('.workspace-notice[data-publish-outcome]')`, { timeoutMs: 120000 });
  const publish = await session.evaluate(`({outcome:document.querySelector('.workspace-notice')?.getAttribute('data-publish-outcome'),text:document.querySelector('.workspace-notice')?.innerText})`);
  assert("NAS-export-published", publish.outcome === "published", publish);
  const runtimes = filesNamed(nas, "reading-source-v2.json");
  const runtime = runtimes.map((file) => ({ file, value: JSON.parse(fs.readFileSync(file, "utf8")) })).find((entry) => entry.value.id === original.ds.exam.examId || entry.value.examId === original.ds.exam.examId);
  assert("runtime-meta-category-P2-and-stable-id", runtime?.value.meta.category === "P2", { file: runtime?.file, id: runtime?.value.id ?? runtime?.value.examId, category: runtime?.value.meta.category });
  const manifests = filesNamed(nas, "manifest.js");
  let matchingManifest;
  for (const file of manifests) {
    const sandbox = { window: {}, globalThis: {} };
    try { vm.runInNewContext(fs.readFileSync(file,"utf8"), sandbox); } catch { continue; }
    const collections = [...Object.values(sandbox), ...Object.values(sandbox.window), ...Object.values(sandbox.globalThis)];
    for (const collection of collections.filter((v) => v && typeof v === "object")) {
      const list = Array.isArray(collection) ? collection : Object.values(collection);
      const entry = list.find((entry) => entry?.id === original.ds.exam.examId || entry?.examId === original.ds.exam.examId);
      if (entry) matchingManifest = { file, entry };
    }
  }
  assert("manifest-category-P2", matchingManifest?.entry.category === "P2", matchingManifest);
  await session.clickSelector('[data-testid="workspace-mode-edit"]');
  await select('[data-testid="workspace-part-select"]', "P3");
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.value==='P3'`);
  await session.clickSelector('[aria-label="撤销"]');
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.value==='P2'`);
  await session.clickSelector('[aria-label="重做"]');
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.value==='P3'`);
  await session.clickSelector('[data-testid="workspace-back"]');
  await session.waitFor(`document.querySelector('[data-item-id="${itemId}"] [data-testid="library-row-part"]')?.textContent==='P3'`, { timeoutMs: 20000 });
  saved = await invoke("get_workspace_item", { itemId });
  assert("editor-change-undo-redo-persists-in-library", saved.ds.exam.category === "P3" && saved.ds.exam.examId === original.ds.exam.examId && saved.ds.exam.title === original.ds.exam.title, { category: saved.ds.exam.category, version: saved.editVersion, examId: saved.ds.exam.examId });
  report.screenshots.push(await session.screenshot("library-P3"));
  await session.close();
  session = undefined;
  // The user's exact historical mismatch, from an online SQLite backup, is
  // checked separately without altering the user's profile or its source files.
  const backup = path.join(repoRoot, "artifacts/part-sync-user-db-backup.db");
  if (!fs.existsSync(backup)) throw new Error(`Historical acceptance backup missing: ${backup}`);
  const pearlsDir = path.join(runDir, "pearls");
  const pearlsData = path.join(pearlsDir, "appdata/data");
  fs.mkdirSync(pearlsData, { recursive: true });
  fs.copyFileSync(backup, path.join(pearlsData,"authoring_hub.db"));
  const pearlsId = "import-20261014104038-86fcf23e";
  const copyDb = new DatabaseSync(path.join(pearlsData,"authoring_hub.db"));
  const before = copyDb.prepare("SELECT current_edit_version AS version, canonical_ds_json AS ds FROM library_items_v2 WHERE id=?").get(pearlsId);
  copyDb.exec("UPDATE processing_jobs_v2 SET stage='cancelled',cloud_status='cancelled',cancel_requested_at='2026-10-16T00:00:00Z'");
  copyDb.close();
  session = await launchTauriAppCdp({ exePath, runDir: pearlsDir, extraBrowserArgs: report.browserArgs });
  await session.evaluate(`window.location.hash='#/library'`);
  await session.waitFor(`!!document.querySelector('[data-item-id="${pearlsId}"]')`, { timeoutMs: 30000 });
  const aligned = await invoke("get_workspace_item", { itemId: pearlsId });
  await invoke("list_library_items", { includeDeleted: false });
  const again = await invoke("get_workspace_item", { itemId: pearlsId });
  const beforeDs = JSON.parse(before.ds);
  assert("actual-Pearls-manual-P3-repair-is-versioned-and-idempotent", beforeDs.exam.category === "P1" && aligned.ds.exam.category === "P3" && aligned.editVersion === before.version + 1 && again.editVersion === aligned.editVersion && again.ds.exam.examId === beforeDs.exam.examId && again.ds.exam.title === beforeDs.exam.title,
    { beforeVersion: before.version, afterVersion: again.editVersion, beforeCategory: beforeDs.exam.category, afterCategory: again.ds.exam.category, examId: again.ds.exam.examId });
  await session.clickSelector(`[data-item-id="${pearlsId}"] .library-row-main`);
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.value==='P3'`, { timeoutMs: 30000 });
  report.screenshots.push(await session.screenshot("Pearls-P3-aligned"));
  report.verdict = "passed";
}
try { await main(); } catch (error) { report.error = String(error?.stack ?? error); console.error(report.error); process.exitCode = 1; }
finally {
  await session?.close().catch(() => {});
  console.log(`[part-sync] report=${writeReport(runDir, report)}`);
}
