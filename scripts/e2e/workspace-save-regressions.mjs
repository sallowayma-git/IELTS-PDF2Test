import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import {
  createPage, ensureVite, evaluate, launchChrome, navigate,
  repoRoot, screenshot, setViewport, sleep, waitFor
} from "../lib/cdp.mjs";

// Real React workspace with a controlled IPC adapter. This does not run Rust or NAS.
const outDir = path.join(repoRoot, "artifacts", "workspace-save-regressions");
fs.mkdirSync(outDir, { recursive: true });
const baseUrl = "http://127.0.0.1:1420";
const vite = await ensureVite(baseUrl);
const chrome = await launchChrome();
const pages = [];
const results = [];

async function workspace({ hold = false, reject = false } = {}) {
  const { cdp } = await createPage(chrome.port);
  pages.push(cdp);
  await setViewport(cdp, { width: 1440, height: 960 });
  await navigate(cdp, `${baseUrl}/?devFallback=1#/settings`);
  await evaluate(cdp, `(async () => {
    const backend = await import('/src/services/devFallbackBackend.ts');
    const patches = await import('/src/services/authoringV2Patches.ts');
    const fixture = await backend.devFallbackInvoke('get_authoring_v2', { jobId: 'phase5-editor-fixture' });
    localStorage.removeItem('ielts-author-studio.workspace-recovery.v1:audit-item');
    const audit = window.__audit = {
      ds: structuredClone(fixture.authoring), version: 1, saves: [], exports: [],
      hold: ${hold}, reject: ${reject}, release: []
    };
    window.localStorage.setItem('ielts-author-studio.app-settings.v1', JSON.stringify({
      cloudEnabled: false, nasDestination: 'AUDIT_NO_FILESYSTEM'
    }));
    window.__TAURI_INTERNALS__ = {
      invoke: async (name, args) => {
        if (name === 'get_workspace_item') return {
          schemaVersion: 'WorkspaceItemV1', ds: structuredClone(audit.ds), editVersion: audit.version,
          item: { itemId: 'audit-item', title: 'Audit Paper', editVersion: audit.version,
            modality: 'reading', status: 'ready', hasCanonicalDs: true }, issues: []
        };
        if (name === 'apply_editor_commands') {
          const input = structuredClone(args.input);
          audit.saves.push(input);
          if (audit.reject) throw new Error('AUDIT_SAVE_FAILED');
          if (audit.hold) await new Promise(resolve => audit.release.push(resolve));
          audit.ds = patches.applyAuthoringV2Patches(audit.ds, input.commands);
          if (input.title !== undefined) audit.ds.exam.title = input.title;
          audit.version += 1;
          return { editVersion: audit.version, appliedCount: input.commands.length, replayed: false };
        }
        if (name === 'publish_items') {
          audit.exports.push(structuredClone(args));
          throw new Error('AUDIT_STOP_BEFORE_FILESYSTEM');
        }
        return backend.devFallbackInvoke(name, args);
      }
    };
    location.hash = '#/items/audit-item';
    return true;
  })()`);
  await waitFor("audit workspace", () => evaluate(cdp,
    `Boolean(document.querySelector('.v2-passage-pane .v2-text.v2-author-editable'))`));
  return cdp;
}

async function edit(cdp, marker) {
  await evaluate(cdp, `(() => {
    document.querySelector('.v2-passage-pane .v2-text.v2-author-editable').click();
    return true;
  })()`);
  await waitFor("inline editor", () => evaluate(cdp,
    `Boolean(document.querySelector('.inline-text-editor'))`));
  await evaluate(cdp, `(() => {
    const editor = document.querySelector('.inline-text-editor');
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(editor, ${JSON.stringify(marker)});
    editor.dispatchEvent(new Event('input', { bubbles: true }));
    return true;
  })()`);
  await sleep(60);
  await evaluate(cdp, `(() => {
    document.querySelector('.inline-text-editor').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    return true;
  })()`);
}

try {
  const slow = await workspace({ hold: true });
  await edit(slow, "AUDIT_FIRST_SAVE");
  await waitFor("first save starts", () => evaluate(slow, `window.__audit.saves.length === 1`));
  await edit(slow, "AUDIT_SECOND_SAVE");
  await sleep(750);
  await evaluate(slow, `(() => {
    window.__audit.hold = false;
    for (const release of window.__audit.release.splice(0)) release();
    return true;
  })()`);
  await waitFor("first save settles", () => evaluate(slow, `Boolean(document.querySelector('.save-state.saved'))`));
  await sleep(750);
  const stalled = await evaluate(slow, `({
    saveCalls: window.__audit.saves.length,
    transmittedText: window.__audit.saves.flatMap(input => input.commands.map(command => command.text)),
    visibleText: document.querySelector('.v2-passage-pane .v2-text.v2-author-editable').textContent,
    saveLabel: document.querySelector('[data-testid="workspace-save-state"]').textContent
  })`);
  assert.equal(stalled.saveCalls, 2);
  assert.equal(stalled.visibleText, "AUDIT_SECOND_SAVE");
  assert.deepEqual(stalled.transmittedText, ["AUDIT_FIRST_SAVE", "AUDIT_SECOND_SAVE"]);
  results.push({ check: "edits-queued-during-an-inflight-save-are-drained", passed: true, ...stalled });
  await screenshot(slow, path.join(outDir, "queued-edit-drained-after-inflight-save.png"));

  const failed = await workspace({ reject: true });
  await edit(failed, "AUDIT_UNSAVED_EDIT");
  await waitFor("save failure", () => evaluate(failed, `Boolean(document.querySelector('.save-state.failed'))`));
  await evaluate(failed, `(() => {
    document.querySelector('[data-testid="workspace-publish"]').click();
    return true;
  })()`);
  await waitFor("flush retried the failed batch", () => evaluate(failed, `window.__audit.saves.length === 2`));
  await sleep(600);
  const bypassed = await evaluate(failed, `({
    saveCalls: window.__audit.saves.length, exportCalls: window.__audit.exports.length,
    failedSaveVisible: Boolean(document.querySelector('.save-state.failed')),
    currentVersion: window.__audit.version
  })`);
  assert.equal(bypassed.failedSaveVisible, true);
  assert.equal(bypassed.currentVersion, 1);
  assert.equal(bypassed.exportCalls, 0);
  const retries = await evaluate(failed, `window.__audit.saves.map(input => input.requestId)`);
  assert.equal(retries[0], retries[1]);
  results.push({ check: "publish-stops-when-the-flush-fails-and-retries-the-same-request", passed: true, ...bypassed });
  await screenshot(failed, path.join(outDir, "publish-blocked-after-failed-flush.png"));

  const firstWindow = await workspace();
  const secondWindow = await workspace();
  await edit(firstWindow, "AUDIT_WINDOW_ONE");
  await waitFor("first window save", () => evaluate(firstWindow, `Boolean(document.querySelector('.save-state.saved'))`));
  await edit(secondWindow, "AUDIT_WINDOW_TWO");
  await waitFor("second window save", () => evaluate(secondWindow, `Boolean(document.querySelector('.save-state.saved'))`));
  const one = await evaluate(firstWindow, `window.__audit.saves[0]`);
  const two = await evaluate(secondWindow, `window.__audit.saves[0]`);
  assert.notEqual(one.requestId, two.requestId);
  assert.notDeepEqual(one.commands, two.commands);
  results.push({ check: "two-windows-never-share-a-request-id", passed: true,
    firstRequestId: one.requestId, secondRequestId: two.requestId,
    firstText: one.commands[0].text, secondText: two.commands[0].text,
    note: "每批命令使用 UUID；重试沿用同一编号。后端对编号复用还会比对稿件与载荷。" });

  fs.mkdirSync(outDir, { recursive: true });
  const report = { coverage: "current-react-ui-with-controlled-ipc-adapter", results };
  fs.writeFileSync(path.join(outDir, "editor-repro.json"), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
} finally {
  for (const cdp of pages) cdp.close();
  chrome.close();
  vite.close();
}
