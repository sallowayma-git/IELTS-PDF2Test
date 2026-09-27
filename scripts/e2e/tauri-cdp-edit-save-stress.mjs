#!/usr/bin/env node
// C1 编辑保存压力：识别可编辑窗口里，连续 30 次前台编辑，每次同时经
// apply_authoring_v2_patches 注入一条与本次编辑无关的后台写入（照搬 option-drag 的做法，
// 制造「前台保存 + 后台写稿」并发）。断言：没有一次进入 failed/conflict；每次都读回持久化；
// 最后重开题库逐条核对。前台编辑走真实 UI 标题编辑（editor.setTitle → 版本化保存链，
// 即 C1 修的 apply_editor_commands 路径），后台写入是 setAnswer 补丁（与标题正交）。
import fs from "node:fs";
import path from "node:path";
import {
  CDP_CHANNEL_LABEL,
  CDP_CHANNEL_NOTE,
  CannotRunError,
  assertBuildFresh,
  buildFreshReport,
  gitHead,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  sleep,
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `edit-save-stress-${stamp}`);
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const pdfArg = process.argv.indexOf("--pdf");
const fixturePath = pdfArg >= 0
  ? path.resolve(process.argv[pdfArg + 1])
  : path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf");
const editsArg = process.argv.indexOf("--edits");
const EDITS = editsArg >= 0 ? Number(process.argv[editsArg + 1]) : 30;
// 默认不做人工后台注入：识别进行中真实的机器写入（本地/云端识别）本身就是并发写入者，
// 且 apply_authoring_v2_patches 不推进 editVersion。--bg 仅用于诊断。
const INJECT_BG = process.argv.includes("--bg");
const extraArgs = "--no-sandbox --disable-gpu";

async function readWorkspaceItem(session, itemId) {
  const r = await session.invoke("get_workspace_item", { itemId });
  if (r?.ok && r.value) return { editVersion: r.value.editVersion ?? null, ds: r.value.ds ?? null };
  return null;
}

// 等保存排空（UI 证据）：pendingCount=0 且状态「已保存」；进入 failed/conflict/recovery
// 即返回 saveFailed，并带上开发者模式下的 saveErrorDetail 供报告归因。
async function waitSaveSettled(session, { timeoutMs = 90000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    last = await session.evaluate(`(() => ({
      pending: document.querySelector('[data-testid="exam-workspace"]')?.getAttribute('data-pending-count') ?? null,
      save: document.querySelector('[data-testid="workspace-save-state"]')?.textContent?.trim() ?? null,
      saveClass: document.querySelector('[data-testid="workspace-save-state"]')?.className ?? null,
      recovery: !!document.querySelector('[data-testid="workspace-save-recovery"]'),
      errorDetail: document.querySelector('[data-testid="workspace-save-error-detail"]')?.textContent?.trim() ?? null,
    }))()`).catch(() => null);
    if (last) {
      if (last.pending === "0" && (last.saveClass?.includes("saved") || last.save === "已保存")) {
        return { settled: true, ...last };
      }
      if (last.saveClass?.includes("failed") || last.saveClass?.includes("conflict") || last.recovery) {
        return { settled: false, saveFailed: true, ...last };
      }
    }
    await sleep(250);
  }
  return { settled: false, timeout: true, ...(last ?? {}) };
}

async function waitPersisted(session, itemId, predicate, { timeoutMs = 30000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let lastItem = null;
  while (Date.now() < deadline) {
    const item = await readWorkspaceItem(session, itemId);
    if (item) { lastItem = item; if (item.ds && predicate(item)) return { ok: true, item }; }
    await sleep(500);
  }
  return { ok: false, item: lastItem };
}

// 后台写入：从权威稿取第一个 responseGroup 的第一个 slot + 选项，发 setAnswer 补丁
// （与标题正交）。baseRevision 从冲突错误 current=N 解析重试。
function firstAnswerTarget(ds) {
  for (const task of ds?.taskGroups ?? []) {
    for (const group of task.responseGroups ?? []) {
      const slotId = (group.slotIds ?? [])[0];
      const shared = Boolean(task.optionBank) && (!(group.options?.length) || group.optionBankRef === task.optionBank.optionBankId);
      const options = shared ? task.optionBank.options : (group.options ?? []);
      if (slotId && options?.length) return { slotId, labels: options.map((o) => o.label).filter(Boolean) };
    }
  }
  return null;
}

async function injectBackgroundWrite(session, itemId, ds, roundIndex) {
  const info = { attempted: true, applied: false };
  const jobId = ds?.jobId ?? null;
  const target = firstAnswerTarget(ds);
  if (!jobId || !target || !target.labels.length) {
    info.reason = `注入前提不足 jobId=${jobId ? "ok" : "missing"} target=${target ? "ok" : "missing"}`;
    return info;
  }
  const label = target.labels[roundIndex % target.labels.length];
  const patch = { op: "setAnswer", slotId: target.slotId, value: { kind: "option", labels: [label], assignment: "per_slot" } };
  let base = 0;
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const r = await session.invoke("apply_authoring_v2_patches", { input: { jobId, baseRevision: base, patches: [patch] } });
    if (r?.ok) { info.applied = true; info.label = label; return info; }
    const match = String(r?.error ?? "").match(/current=(\d+)/);
    if (!match) { info.reason = `apply_authoring_v2_patches 失败：${String(r?.error ?? "").slice(0, 160)}`; return info; }
    base = Number(match[1]);
  }
  info.reason = "baseRevision 重试用尽";
  return info;
}

function recordAssertion(report, id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

// 识别「进行中」的权威阶段（与 option-drag 同源，list_library_items → item.processing）。
const ACTIVE_STAGES = new Set(["queued", "running", "local_recognition", "cloud_recognition", "reconciling"]);
async function readStage(session, itemId) {
  const r = await session.invoke("list_library_items", { includeDeleted: false });
  if (!r?.ok || !Array.isArray(r.value)) return null;
  const p = r.value.find((i) => i?.id === itemId)?.processing ?? null;
  return p?.stage ?? null;
}
const report = {
  task: "edit-save-stress-under-concurrent-writes",
  scope: "识别可编辑窗口内连续 30 次前台编辑 + 每次并发后台写入，断言无 failed/conflict、无丢写（真实应用端到端）",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  evidenceLevel: "real-app-e2e (CDP automation channel)",
  fixturePath,
  fixtureSha256: sha256File(fixturePath),
  edits: [],
  assertions: [],
  consoleErrors: [],
  pageExceptions: [],
};

async function commitTitle(session, newTitle) {
  // 标题显示态是 <strong data-testid="workspace-title"><span role="button" onClick=begin>；
  // 必须点内层 span（strong 可能宽于文字，点中心会落空）。
  await session.clickSelector('[data-testid="workspace-title"] [role="button"]');
  await session.waitFor(`!!document.querySelector('[data-testid="workspace-title-input"]')`, { timeoutMs: 8000, label: "title-input" });
  await session.evaluate(`(() => {
    const input = document.querySelector('[data-testid="workspace-title-input"]');
    if (!input) return false;
    input.value = ${JSON.stringify(newTitle)};
    input.blur();
    return true;
  })()`);
}

async function main() {
  if (!fs.existsSync(fixturePath)) throw new CannotRunError(`fixture 不存在：${fixturePath}`);
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  report.identity = {
    exePath, exeSha256: sha256File(exePath), commit: gitHead(repoRoot),
    buildFresh: buildFreshReport(fresh), toleratedConcurrentBackendEdits: fresh.tolerated ?? [],
  };

  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  fs.copyFileSync(fixturePath, path.join(runDir, "pdfs", path.basename(fixturePath)));

  const session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  let verdict = "failed";
  try {
    report.browserArgs = session.browserArgs;
    // 开发者模式：让保存失败时的 saveErrorDetail 渲染出来，供归因。
    await session.evaluate(`(() => {
      const key = "ielts-author-studio.app-settings.v1";
      let cur = {}; try { cur = JSON.parse(window.localStorage.getItem(key) ?? "{}"); } catch {}
      cur.developerMode = true; window.localStorage.setItem(key, JSON.stringify(cur)); return true;
    })()`);

    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library" });
    const before = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
    await sleep(600);
    await session.clickSelector('[data-testid="import-pick-folder"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked" });
    await session.clickSelector('[data-testid="import-start"]');
    let itemId = null;
    const rowDeadline = Date.now() + 90000;
    while (Date.now() < rowDeadline && !itemId) {
      const ids = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
      itemId = (ids ?? []).find((id) => !(before ?? []).includes(id)) ?? null;
      if (!itemId) await sleep(500);
    }
    if (!itemId) throw new Error("导入后未出现新的题库行");
    report.identity.itemId = itemId;

    await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace" });
    // 关键：**不等 seed 完成**就开始编辑——识别进行中打开工作区正是变红的复现前提。
    await sleep(800); // 等工作区外壳与标题渲染

    const stagesSeen = new Set();
    let sawActiveStage = false;
    let seedAtEdit = null; // 第一次读到权威稿（ds 出现）的编辑序号 = seed 时点
    for (let i = 0; i < EDITS; i += 1) {
      const edit = { round: i + 1 };
      const stage = await readStage(session, itemId);
      edit.stage = stage;
      if (stage) { stagesSeen.add(stage); if (ACTIVE_STAGES.has(stage)) sawActiveStage = true; }
      const pre = await readWorkspaceItem(session, itemId);
      edit.baseVersionSeen = pre?.editVersion ?? null;
      edit.dsPresent = Boolean(pre?.ds); // false = 本次编辑发生在 seed 之前
      if (edit.dsPresent && seedAtEdit === null) seedAtEdit = i + 1;
      if (INJECT_BG) edit.backgroundWrite = await injectBackgroundWrite(session, itemId, pre?.ds, i);
      const newTitle = `压测标题 ${i + 1} · ${Date.now()}`;
      edit.title = newTitle;
      await commitTitle(session, newTitle);
      // 给足到本地识别完成的时间：seed 前暂存的标题会在 seed→重载后补发。
      const save = await waitSaveSettled(session, { timeoutMs: 120000 });
      edit.save = save;
      if (!save.settled) {
        edit.ok = false;
        edit.saveErrorDetail = save.errorDetail ?? null; // 失败时记录后端原始错误码
        report.edits.push(edit);
        await session.screenshot(`fail-edit-${i + 1}`).catch(() => {});
        continue;
      }
      const persisted = await waitPersisted(session, itemId,
        (item) => item.ds?.exam?.title === newTitle, { timeoutMs: 30000 });
      edit.ok = persisted.ok;
      edit.editVersion = persisted.item?.editVersion ?? null;
      if (!persisted.ok) edit.reason = `读回标题未持久化，当前=${JSON.stringify(persisted.item?.ds?.exam?.title ?? null)}`;
      report.edits.push(edit);
    }
    report.stagesSeen = [...stagesSeen];
    report.seedAtEdit = seedAtEdit; // 第几次编辑时权威稿才出现（其之前的编辑发生在 seed 之前）
    const first = report.edits[0] ?? {};
    // 证明至少第一次编辑确实落在 seed 之前 / 识别进行中——否则「base=0 修复在真实应用生效」
    // 没被真正覆盖，如实记进 detail（离线时"识别中还能编辑"的窗口可能很窄）。
    const firstEditDuringRecognition = first.dsPresent === false || ACTIVE_STAGES.has(first.stage);

    const okEdits = report.edits.filter((e) => e.ok).length;
    const failedSaves = report.edits.filter((e) => e.save && !e.save.settled);
    recordAssertion(report, "C1-1 edits-during-recognition-persist-no-red", okEdits === EDITS,
      `${okEdits}/${EDITS} 次编辑（识别进行中打开工作区）保存排空并读回持久化；失败详情=${JSON.stringify(failedSaves.map((e) => ({ round: e.round, stage: e.stage, dsPresent: e.dsPresent, save: e.save, saveErrorDetail: e.saveErrorDetail })))}`);
    recordAssertion(report, "C1-2 first-edit-before-seed", firstEditDuringRecognition,
      `第 1 次编辑：stage=${JSON.stringify(first.stage)}、dsPresent=${first.dsPresent}（false=seed 前）；seed 出现在第 ${seedAtEdit} 次编辑；全程阶段=${JSON.stringify([...stagesSeen])}。为真才证明 base=0 修复覆盖了「识别进行中编辑」窗口。`);

    // 重开题库再进工作区，核对最后一次标题持久化。
    const lastTitle = report.edits.length ? report.edits[report.edits.length - 1].title : null;
    // 先等保存彻底排空（pending=0）再离开，避免返回时布局因保存条/提示变动导致点击落空。
    await waitSaveSettled(session, { timeoutMs: 30000 }).catch(() => {});
    await sleep(400);
    await session.clickSelectorWhenStable('[data-testid="workspace-back"]', { timeoutMs: 15000 })
      .catch(() => session.clickSelector('[data-testid="workspace-back"]'));
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-back" });
    await session.waitFor(`!!document.querySelector('[data-item-id="${itemId}"] .library-row-main')`, { timeoutMs: 20000, label: "row-back" });
    await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-reopen" });
    const reopened = await readWorkspaceItem(session, itemId);
    recordAssertion(report, "C1-3 reopen-consistent", reopened?.ds?.exam?.title === lastTitle,
      `重开后标题=${JSON.stringify(reopened?.ds?.exam?.title ?? null)}，期望=${JSON.stringify(lastTitle)}`);

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
  console.log(`[edit-save-stress] verdict=${report.verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
  console.log(`[edit-save-stress] report=${file}`);
  process.exitCode = report.verdict === "passed" ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  report.verdict = cannotRun ? "cannot-run" : "failed";
  report.error = String(error?.message ?? error);
  try { writeReport(runDir, report); } catch {}
  console.error(`[edit-save-stress] ${report.verdict}: ${report.error}`);
  process.exitCode = cannotRun ? 3 : 1;
});

