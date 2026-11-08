// Real Tauri UI/IPC/scheduler/SQLite against a REAL cloud model (spends tokens).
// Merges private single-passage PDFs into one multi-passage paper, imports it,
// then re-recognizes one item sequentially to observe prompt-cache hits.
// Requires: fixtures/golden/private-real/*.pdf and an enabled profile in the
// user's app config whose key lives in OS secure storage (never read here).
// Usage: node scripts/e2e/tauri-cdp-cloud-real-model.mjs [profileId] [existing multi-passage PDF]
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { DatabaseSync } from 'node:sqlite';
import { assertBuildFresh, launchTauriAppCdp, repoRoot, sha256File, sleep } from './lib/tauri-cdp-harness.mjs';

const passages = ['chili-peppers', 'conformity', 'sleep-study'];
const runDir = path.join(repoRoot, 'artifacts/e2e-cdp', `run-cloud-real-${new Date().toISOString().replace(/[:.]/g, '-')}`);
const data = path.join(runDir, 'appdata/data');
const exePath = path.join(repoRoot, 'src-tauri/target/debug/ielts-author-studio.exe');
fs.mkdirSync(path.join(data, 'config'), { recursive: true });

const userProfiles = JSON.parse(fs.readFileSync(path.join(process.env.APPDATA, 'com.ielts.author.studio/config/llm-profiles.json'), 'utf8'));
const wanted = process.argv[2];
const profile = userProfiles.find(p => p.enabled && p.secretStorageBackend === 'os' && (!wanted || p.profileId === wanted));
assert(profile, 'no enabled OS-secret profile found');
fs.writeFileSync(path.join(data, 'config/llm-profiles.json'), JSON.stringify([profile]));

const suppliedPdf = process.argv[3];
const merged = suppliedPdf ? path.join(runDir, 'supplied.pdf') : path.join(runDir, 'three-passages.pdf');
const inputs = passages.map(name => path.join(repoRoot, 'fixtures/golden/private-real', `${name}.pdf`));
if (suppliedPdf) fs.copyFileSync(suppliedPdf, merged);
else execFileSync('python', ['-I', '-c', `import sys,pypdf
w=pypdf.PdfWriter()
counts=[]
for f in sys.argv[2:]:
    r=pypdf.PdfReader(f); counts.append(len(r.pages))
    for p in r.pages: w.add_page(p)
w.write(sys.argv[1]); print(counts)`, merged, ...inputs], { stdio: ['ignore', 'pipe', 'ignore'] });
const pageCounts = passages.map(name => JSON.parse(execFileSync('python', ['-I', '-c',
  'import sys,pypdf,logging;logging.disable(50);print(len(pypdf.PdfReader(sys.argv[1]).pages))',
  path.join(repoRoot, 'fixtures/golden/private-real', `${name}.pdf`)]).toString()));
let start = 1;
const expectedRanges = suppliedPdf ? 'supplied PDF: judge by page text' : passages.map((name, i) => { const r = { name, from: start, to: start + pageCounts[i] - 1 }; start += pageCounts[i]; return r; });

const report = { channel: 'webview2-cdp', realModel: { provider: profile.baseUrl, model: profile.model, timeoutMs: profile.timeoutMs },
  exeSha256: sha256File(exePath), build: assertBuildFresh({ exePath }), expectedRanges, checks: [] };
const dbRead = fn => { const db = new DatabaseSync(path.join(data, 'authoring_hub.db'), { readOnly: true }); try { db.exec('PRAGMA busy_timeout=5000'); return fn(db); } finally { db.close(); } };
function usage() {
  const out = [];
  for (const dir of fs.readdirSync(path.join(data, 'jobs'))) {
    const file = path.join(data, 'jobs', dir, 'llm-usage.jsonl');
    if (!fs.existsSync(file)) continue;
    for (const line of fs.readFileSync(file, 'utf8').split('\n').filter(Boolean)) {
      const r = JSON.parse(line); const u = r.usage ?? r;
      out.push({ job: dir, command: r.commandName ?? r.command, prompt: u.prompt_tokens, cacheHit: u.prompt_cache_hit_tokens, cacheMiss: u.prompt_cache_miss_tokens, completion: u.completion_tokens, at: r.recordedAt });
    }
  }
  return out.sort((a, b) => String(a.at).localeCompare(String(b.at)));
}
async function settled(timeoutMs = 600000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const jobs = dbRead(db => db.prepare('SELECT id,stage,cloud_status,last_error_code FROM processing_jobs_v2').all());
    if (jobs.length && jobs.every(j => ['ready_for_review', 'failed', 'cancelled'].includes(j.stage))) return jobs;
    await sleep(1000);
  }
  throw new Error('real-model import did not settle');
}
let session;
try {
  session = await launchTauriAppCdp({ exePath, runDir, appEnv: { PDF2TEST_AUTOMATION_SOURCE_FILES: merged } });
  await session.waitFor(`!!document.querySelector('[data-testid="library-import"]')`, { timeoutMs: 40000 });
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-cloud-multi"]')`);
  await session.clickSelectorWhenStable('[data-testid="import-pick-files"]');
  await session.waitFor(`document.querySelectorAll('[data-testid="import-picked-files"] li').length===1`);
  await session.clickSelector('[data-testid="import-start"]');
  const started = Date.now();
  report.jobs = await settled();
  report.firstImportSeconds = Math.round((Date.now() - started) / 1000);
  const rows = dbRead(db => db.prepare('SELECT id,title,status,canonical_ds_json FROM library_items_v2').all());
  report.items = rows.map(row => {
    const ds = row.canonical_ds_json ? JSON.parse(row.canonical_ds_json) : null;
    const direct = path.join(data, 'jobs', row.id, 'cloud-direct-result.json');
    const result = fs.existsSync(direct) ? JSON.parse(fs.readFileSync(direct, 'utf8')) : null;
    return { id: row.id, title: row.title, status: row.status, sourcePages: result?.sourcePages,
      slots: ds ? Object.keys(ds.answerSlots ?? {}).length : 0, warnings: result?.warnings };
  });
  report.usageAfterImport = usage();
  report.libraryScreenshot = await session.screenshot('real-model-library');
  for (const row of rows.filter(r => r.canonical_ds_json)) {
    await session.waitFor(`!!document.querySelector('[data-item-id="${row.id}"] .library-row-main')`, { timeoutMs: 15000 });
    await session.clickSelector(`[data-item-id="${row.id}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-title"]')`, { timeoutMs: 15000 });
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('.v2-passage-pane')`, { timeoutMs: 15000 });
    const text = await session.evaluate(`document.querySelector('.v2-passage-pane').innerText.length`);
    report.checks.push(`student preview renders "${row.title}" (${text} chars of passage text)`);
    await session.clickSelector('[data-testid="workspace-back"]');
    await session.waitFor(`!!document.querySelector('[data-testid="library-row"]')`);
  }
  // Sequential scoped re-recognition: same system prompt + examples prefix.
  const target = rows.find(r => r.canonical_ds_json);
  if (target) {
    const before = usage().length;
    const result = await session.invoke('retry_processing', { itemId: target.id });
    assert(result.ok, String(result.error));
    await sleep(1500);
    await settled();
    report.rerecognition = { itemId: target.id, calls: usage().slice(before) };
    report.itemCountAfterRetry = dbRead(db => db.prepare('SELECT count(*) AS n FROM library_items_v2').get()).n;
  }
  report.verdict = 'completed';
} catch (error) {
  report.verdict = 'failed'; report.error = String(error); process.exitCode = 1;
  report.failureScreenshot = await session?.screenshot('failure').catch(() => null);
} finally {
  if (session) report.appOutput = (await session.close()).appOutput;
  try { report.usageFinal = usage(); } catch {}
  fs.writeFileSync(path.join(runDir, 'report.json'), JSON.stringify(report, null, 2));
  console.log(path.join(runDir, 'report.json'));
}
