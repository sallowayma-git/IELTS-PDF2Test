#!/usr/bin/env node
// Real Tauri UI/IPC/SQLite/export, isolated profile. The local HTTP service
// replays an archived real cloud-direct output for The Pearls (A–G, Q1–13).
// This deliberately tests the real scheduler and write path without a paid
// model call. It is not evidence of a new live model recognition run.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { DatabaseSync } from 'node:sqlite';
import { assertBuildFresh, buildFreshReport, launchTauriAppCdp, repoRoot, sleep, writeReport } from './lib/tauri-cdp-harness.mjs';

const before = process.argv.includes('--before');
const runDir = path.join(repoRoot, 'artifacts/e2e-cdp', `student-feedback-${before ? 'before-' : ''}${new Date().toISOString().replace(/[:.]/g, '-')}`);
const data = path.join(runDir, 'appdata/data');
const exePath = path.join(repoRoot, 'src-tauri/target/debug/ielts-author-studio.exe');
const historical = path.join(repoRoot, 'artifacts/e2e-cdp/run-cloud-direct-2026-10-14T22-55-23-591Z/online-direct/appdata/data/authoring_hub.db');
const report = { task: 'student-feedback', before, channel: 'WebView2 CDP', cloudService: 'local HTTP replay of archived real cloud-direct output', diagnosticRun: false, assertions: [], snapshots: [], screenshots: [], requests: 0, verdict: 'failed' };
let session, itemId;
let delayResponse = false;
const assert = (id, ok, detail) => { report.assertions.push({ id, ok: !!ok, detail }); console.log(`${ok ? 'PASS' : 'FAIL'} ${id}: ${JSON.stringify(detail)}`); };
const dbRead = (fn) => { const db = new DatabaseSync(path.join(data, 'authoring_hub.db'), { readOnly: true }); try { return fn(db); } finally { db.close(); } };
async function invoke(command, args) { const r = await session.invoke(command, args); if (!r?.ok) throw new Error(`${command}: ${JSON.stringify(r)}`); return r.value; }
async function selectPart(value) {
  await session.evaluate(`(() => {const el=document.querySelector('[data-testid="workspace-part-select"]'); if(!el||el.disabled) throw Error('Part is disabled'); Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(el,${JSON.stringify(value)}); el.dispatchEvent(new Event('change',{bubbles:true}));})()`);
}
async function snapshot(label) {
  const persisted = dbRead(db => db.prepare('SELECT current_edit_version,part_label,part_source,canonical_ds_json,protected_edits_json FROM library_items_v2 WHERE id=?').get(itemId));
  const journal = dbRead(db => db.prepare('SELECT * FROM editor_journal_v1 WHERE library_item_id=? ORDER BY rowid DESC LIMIT 6').all(itemId));
  const ds = JSON.parse(persisted.canonical_ds_json);
  const ui = await session.evaluate(`({part:document.querySelector('[data-testid="workspace-part-select"]')?.value,disabled:document.querySelector('[data-testid="workspace-part-select"]')?.disabled,save:document.querySelector('[data-testid="workspace-save-state"]')?.textContent,remote:document.querySelector('[data-testid="workspace-remote-pending"]')?.textContent,error:document.querySelector('[data-testid="workspace-save-message"]')?.textContent,subtitle:document.querySelector('.workspace-sub-header-label')?.textContent})`);
  const result = { label, ...ui, category: ds.exam.category, partLabel: persisted.part_label, partSource: persisted.part_source, version: persisted.current_edit_version, protectedEdits: persisted.protected_edits_json, journal };
  report.snapshots.push(result); return result;
}
async function settlePart(value, label) {
  await session.waitFor(`document.querySelector('[data-testid="workspace-save-state"]')?.textContent==='已保存'`, { timeoutMs: 20000 });
  await sleep(350);
  const s = await snapshot(label);
  assert(`${label}-persisted`, s.part === value && s.category === value && s.partLabel === value && s.partSource === 'manual', s);
  assert(`${label}-no-own-save-warning`, !s.remote, s.remote ?? null);
}
async function open() { await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`); await session.waitFor(`!!document.querySelector('[data-testid="workspace-part-select"]')`, { timeoutMs: 30000 }); }
async function back() { await session.clickSelector('[data-testid="workspace-back"]'); await session.waitFor(`!!document.querySelector('[data-item-id="${itemId}"]')`, { timeoutMs: 30000 }); }

const archive = new DatabaseSync(historical, { readOnly: true });
const document = JSON.parse(archive.prepare('SELECT canonical_ds_json FROM library_items_v2 WHERE id=?').get('import-20261014225530-860b418d').canonical_ds_json);
archive.close();
document.exam.title = 'The Pearls';
const service = http.createServer(async (req, res) => {
  for await (const _ of req) { /* consume request */ }
  report.requests++;
  if (delayResponse) await sleep(4500);
  res.writeHead(200, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ choices: [{ message: { content: JSON.stringify(document) }, finish_reason: 'stop' }], usage: { prompt_tokens: 100, completion_tokens: 100, total_tokens: 200 } }));
});
await new Promise(resolve => service.listen(0, '127.0.0.1', resolve));
try {
  report.build = buildFreshReport(assertBuildFresh({ exePath }));
  fs.mkdirSync(path.join(data, 'config/secrets'), { recursive: true });
  fs.writeFileSync(path.join(data, 'config/llm-profiles.json'), JSON.stringify([{ profileId: 'feedback-replay', name: 'Feedback replay', provider: 'OpenAiCompatible', model: 'feedback-replay', baseUrl: `http://127.0.0.1:${service.address().port}/v1`, enabled: true, forceJson: true, temperature: 0, timeoutMs: 30000 }]));
  fs.writeFileSync(path.join(data, 'config/secrets/feedback-replay.key'), 'local-test-token');
  const pdf = path.join(runDir, 'The Pearls.pdf');
  fs.copyFileSync('F:/workspace/IELTS Atlas/ReadingPractice/PDF/24. P1 - The Pearls 珍珠.pdf', pdf);
  session = await launchTauriAppCdp({ exePath, runDir, appEnv: { PDF2TEST_AUTOMATION_SOURCE_FILES: pdf, EPIC8_PDFIUM_LIB: path.join(repoRoot, 'src-tauri/lib/pdfium-windows/pdfium.dll') } });
  await session.waitFor(`!!document.querySelector('[data-testid="library-import"]')`, { timeoutMs: 40000 });
  await session.evaluate(`localStorage.setItem('ielts-author-studio.app-settings.v1',JSON.stringify({nasDestination:${JSON.stringify(path.join(runDir,'nas'))},cloudEnabled:true,cloudProfileId:'feedback-replay',cloudTokenBudget:100000,pipelineMode:'direct'}))`);
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-pick-files"]')`);
  await session.clickSelector('[data-testid="import-pick-files"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`);
  await session.clickSelector('[data-testid="import-start"]');
  await session.waitFor(`!!document.querySelector('[data-testid="library-row"]')`, { timeoutMs: 40000 });
  itemId = await session.evaluate(`document.querySelector('[data-testid="library-row"]').dataset.itemId`);
  report.itemId = itemId;
  for (let i = 0; i < 120; i++) {
    const state = dbRead(db => db.prepare('SELECT stage FROM processing_jobs_v2 WHERE library_item_id=?').get(itemId));
    if (state?.stage === 'ready_for_review') break;
    if (state?.stage === 'failed') throw new Error('direct import failed');
    await sleep(500);
  }
  const w = await invoke('get_workspace_item', { itemId });
  assert('cloud-direct-structural-fixture', w.ds.audit.source === 'cloud_direct' && w.ds.exam.category === 'P1' && Object.keys(w.ds.passage.paragraphMap).join('') === 'ABCDEFG' && Object.keys(w.ds.answerSlots).length === 13 && !w.ds.taskGroups[0].optionBank, { source: w.ds.audit.source, category: w.ds.exam.category, title: w.ds.exam.title, filename: path.basename(pdf), paragraphs: Object.keys(w.ds.passage.paragraphMap), questions: Object.values(w.ds.answerSlots).map(s => s.questionNumber), taskType: w.ds.taskGroups[0].taskType });
  await open();
  for (const mode of ['edit', 'student']) {
    await session.clickSelector(`[data-testid="workspace-mode-${mode}"]`);
    for (const part of ['P1', 'P3', 'P2']) { await selectPart(part); await settlePart(part, `${mode}-${part}`); }
    report.screenshots.push(await session.screenshot(`${mode}-P2`));
    await back();
    assert(`${mode}-library-P2`, await session.evaluate(`document.querySelector('[data-item-id="${itemId}"] [data-testid="library-row-part"]')?.textContent==='P2'`));
    await open();
    assert(`${mode}-reopen-P2`, (await snapshot(`${mode}-reopen`)).part === 'P2');
  }
  // Exercise real active scheduler, direct normalization and replace transaction.
  await selectPart('P3'); await settlePart('P3', 'before-cloud-rerun');
  delayResponse = true;
  await invoke('retry_processing', { itemId });
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.disabled===true`, { timeoutMs: 20000 });
  const active = await snapshot('cloud-running');
  assert('active-cloud-shows-lock-and-retains-P3', active.disabled && active.part === 'P3');
  await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.disabled===false`, { timeoutMs: 40000 });
  await sleep(500);
  const rerun = await snapshot('after-cloud-rerun');
  assert('machine-replacement-preserves-human-Part', rerun.part === 'P3' && rerun.category === 'P3' && rerun.partLabel === 'P3' && rerun.partSource === 'manual', rerun);
  report.screenshots.push(await session.screenshot('after-cloud-rerun'));
  if (!before) {
    for (const mode of ['edit', 'student']) {
      await session.clickSelector(`[data-testid="workspace-mode-${mode}"]`);
      await selectPart('P1'); await settlePart('P1', `${mode}-race-base`);
      // Controlled persisted lock between selection and autosave. No response
      // is mocked: the actual Rust save must reject and retain the selection.
      // A real scheduler rerun was exercised separately above.
      await selectPart('P3');
      const lockDb = new DatabaseSync(path.join(data, 'authoring_hub.db'));
      lockDb.prepare("UPDATE processing_jobs_v2 SET stage='cloud_recognition',cloud_status='running',cancel_requested_at=NULL,lease_owner='feedback-controlled-lock',lease_expires_at=?,event_seq=event_seq+1 WHERE library_item_id=?").run(new Date(Date.now()+3600000).toISOString(), itemId);
      const lockState = lockDb.prepare('SELECT event_seq FROM processing_jobs_v2 WHERE library_item_id=?').get(itemId);
      lockDb.close();
      await session.waitFor(`document.querySelector('[data-testid="workspace-save-state"]')?.textContent?.includes('暂缓保存')`, { timeoutMs: 20000 });
      await invoke('plugin:event|emit', { event: 'processing://item-updated', payload: { libraryItemId: itemId, stateVersion: lockState.event_seq, editVersion: dbRead(db => db.prepare('SELECT current_edit_version FROM library_items_v2 WHERE id=?').get(itemId)).current_edit_version } });
      await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.disabled===true`, { timeoutMs: 10000 });
      const locked = await snapshot(`${mode}-save-locked`);
      assert(`${mode}-lock-failure-explicit-and-selection-retained`, locked.part === 'P3' && locked.error?.includes('选择已保留') && locked.category === 'P1', locked);
      report.screenshots.push(await session.screenshot(`${mode}-save-locked`));
      await session.clickSelector('[data-testid="workspace-cloud-review-stop"]');
      await session.waitFor(`document.querySelector('[data-testid="workspace-part-select"]')?.disabled===false`, { timeoutMs: 40000 });
      await settlePart('P3', `${mode}-lock-retry`);
      // A controlled lease has no worker to finalize cancellation. Set its
      // terminal state exactly as a worker would, before navigating away.
      const terminalDb = new DatabaseSync(path.join(data, 'authoring_hub.db'));
      terminalDb.prepare("UPDATE processing_jobs_v2 SET stage='cancelled',cloud_status='cancelled',lease_owner=NULL,lease_expires_at=NULL,event_seq=event_seq+1 WHERE library_item_id=?").run(itemId);
      const terminal = terminalDb.prepare('SELECT event_seq FROM processing_jobs_v2 WHERE library_item_id=?').get(itemId);
      terminalDb.close();
      await invoke('plugin:event|emit', { event: 'processing://item-updated', payload: { libraryItemId: itemId, stateVersion: terminal.event_seq, editVersion: dbRead(db => db.prepare('SELECT current_edit_version FROM library_items_v2 WHERE id=?').get(itemId)).current_edit_version } });
      await selectPart('P2'); await settlePart('P2', `${mode}-active-P2`);
      await back(); await open();
      assert(`${mode}-active-reopen-P2`, (await snapshot(`${mode}-active-reopen`)).part === 'P2');
    }
    await selectPart('P3'); await settlePart('P3', 'published-P3');
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="matching-matrix"]')`, { timeoutMs: 20000 });
    const previewBaseline = await invoke('get_workspace_item', { itemId });
    await session.clickSelector('[data-testid="matching-matrix"] input[name="q1"][value="A"]');
    assert('matrix-answer-selected', await session.evaluate(`document.querySelector('[data-testid="matching-matrix"] input[name="q1"][value="A"]').checked`));
    await session.clickSelector('.student-reading-toolbar button[aria-label="Note"]');
    assert('topbar-Notes-panel', await session.evaluate(`!!document.querySelector('[aria-label="Notes"] textarea')`));
    await session.clickSelector('.student-notes-panel button');
    await session.evaluate(`(() => {const el=document.querySelector('.v2-passage-pane .v2-text');const r=document.createRange();r.selectNodeContents(el);const s=window.getSelection();s.removeAllRanges();s.addRange(r);document.dispatchEvent(new Event('selectionchange'));})()`);
    await session.waitFor(`!!document.querySelector('.student-highlight-toolbar')`);
    await session.clickSelector('.student-highlight-toolbar button[data-action="note"]');
    assert('selection-Note-captures-quote', await session.evaluate(`!!document.querySelector('.hl[data-hl-type="note"]') && document.querySelector('.student-notes-panel textarea').value.startsWith('> ')`));
    report.screenshots.push(await session.screenshot('student-Notes-and-matrix'));
    await session.clickSelector('.student-notes-panel button');
    report.screenshots.push(await session.screenshot('student-matrix-selected'));
    await session.clickSelector('[aria-label="Text size xlarge"]');
    assert('author-preview-matrix-font-scales', await session.evaluate(`getComputedStyle(document.querySelector('[data-testid="matching-matrix"] table')).fontSize==='20px'`));
    await session.cdp.send('Emulation.setDeviceMetricsOverride', { width: 520, height: 850, deviceScaleFactor: 1, mobile: false });
    assert('author-preview-narrow-matrix-scrolls', await session.evaluate(`(() => {const el=document.querySelector('[data-testid="matching-matrix"]');return getComputedStyle(el).overflowX==='auto' && el.scrollWidth>el.clientWidth;})()`));
    report.screenshots.push(await session.screenshot('student-matrix-narrow'));
    await session.cdp.send('Emulation.clearDeviceMetricsOverride');
    const saved = await invoke('get_workspace_item', { itemId });
    assert('preview-Notes-and-answer-not-persisted', JSON.stringify(saved.ds) === JSON.stringify(previewBaseline.ds) && saved.editVersion === previewBaseline.editVersion);
    await session.evaluate(`localStorage.setItem('ielts-author-studio.confirmed-nas-export-dir.v1',${JSON.stringify(path.join(runDir,'nas'))})`);
    await session.clickSelector('[data-testid="workspace-publish"]');
    await session.waitFor(`!!document.querySelector('[data-publish-outcome]')`, { timeoutMs: 60000 });
    const publish = await session.evaluate(`document.querySelector('[data-publish-outcome]').getAttribute('data-publish-outcome')`);
    assert('published-NAS-package', publish === 'published', publish);
    report.nasDir = path.join(runDir, 'nas');
  }
  report.verdict = report.assertions.every(a => a.ok) ? 'passed' : 'failed';
} catch (error) {
  report.error = String(error.stack ?? error); console.error(report.error);
  if (session && itemId) {
    try { report.failureSnapshot = await snapshot('failure'); report.screenshots.push(await session.screenshot('failure')); } catch {}
  }
}
finally { report.reattaches = session?.reattaches; await session?.close(); service.close(); console.log(`report=${writeReport(runDir, report)}`); if (report.verdict !== 'passed') process.exitCode = 1; }
