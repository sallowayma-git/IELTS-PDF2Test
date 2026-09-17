#!/usr/bin/env node
/**
 * 无云导入的「四条链」证据：本地链必须真的跑过，云端必须如实标成 not_run。
 *
 * 背景（本轮修复的根因）：`scheduler.rs` 的 `!launch_cloud` 分支在发布「可编辑」之后
 * 直接 `return`，reconcile 从未被调用 —— 于是无云导入的 `batchId` 为 null、四条链
 * 全部 `not_run`，前端拿不到任何可解释的证据链（找不到批次、看不到原文核验、也没有
 * 裁决汇总）。后端现已修复：无云分支照常走完「本地候选 → 原文核验 → 裁决 → 落盘」，
 * 只把云端如实标成 `not_run`（`CLOUD_DISABLED`），**绝不**谎报成 `failed`。
 *
 * 本脚本在**真实 exe + 真实 IPC + 真实 SQLite**上验证这条修复：
 *   1. 导入后必须存在批次（`batchId` 非空）；
 *   2. 本地链状态必须**准确**——不是 `not_run`，且与权威稿是否真的产出内容一致
 *      （报 succeeded 却没有稿、或已有稿却报 not_run，都算谎报）；
 *   3. 原文核验与裁决也必须跑过（不能四条链全 not_run）；
 *   4. 云端必须是 `not_run` 且**带稳定原因码**（`not_run` 不携带原因是契约违规），
 *      而不是拿一个失败 profile 制造假失败。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-local-chain.mjs [--keep] [--fixtures a.pdf,b.docx]
 * 退出码：0 = 全部步骤通过；3 = CANNOT-RUN；1 = 步骤失败。
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  CDP_CHANNEL_LABEL,
  CDP_CHANNEL_NOTE,
  CannotRunError,
  assertBuildFresh,
  buildFreshReport,
  createStepRecorder,
  gitHead,
  gitWorktreeClean,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  sleep,
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const fixturesIdx = process.argv.indexOf("--fixtures");
const fixturePaths = (
  fixturesIdx >= 0
    ? process.argv[fixturesIdx + 1].split(",")
    : [
        path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf"),
        path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-1.docx"),
      ]
).map((p) => path.resolve(p.trim()));

// 与产品链一致：本沙箱下 renderer 需要这两个开关才能稳定，属诊断参数。
const extraArgs = "--no-sandbox --disable-gpu";
const runDir = path.join(
  repoRoot,
  "artifacts",
  "e2e-cdp",
  `run-local-chain-${new Date().toISOString().replace(/[:.]/g, "-")}`
);

/** `not_run` 的合法原因码：都表示「本次没有云端参与」，不是云端故障。 */
const NOT_RUN_REASONS = new Set(["CLOUD_DISABLED", "NO_PROFILE"]);

const report = {
  task: "no-cloud-local-chain",
  scope: "无云导入的四条链：有 batch、本地链真的跑过、云端如实 not_run（不谎报 failed）",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  startedAt: new Date().toISOString(),
  runDir,
  identity: {
    exePath,
    exeSha256: null,
    fixtures: fixturePaths.map((p) => ({ path: p, sha256: null })),
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: null,
  },
  steps: [],
  verdict: "failed",
};

let session = null;
let recorder = null;

async function call(command, args) {
  const wrapped = command === "apply_recognition_decisions" || command === "apply_editor_commands";
  const r = await session.invoke(command, wrapped ? { input: args } : args);
  if (r && r.__noInvoke) throw new Error("invoke 不可用");
  return r;
}

/** 归一化一条链：`{state, reasonCode, message}`。 */
function stage(v) {
  return {
    state: v?.state ?? null,
    reasonCode: v?.reasonCode ?? null,
    message: v?.message ?? null,
  };
}

/** 导入一个夹具并返回其 itemId（等新行出现，最长 90s）。 */
async function importFixture(fixturePath, stagedPath) {
  const isPdf = /\.pdf$/i.test(fixturePath);
  const before = await session.evaluate(
    `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`
  );
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, {
    timeoutMs: 15000,
    label: "import-drawer",
  });
  await session.clickSelector(isPdf ? '[data-testid="import-pick-folder"]' : '[data-testid="import-pick-files"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, {
    timeoutMs: 20000,
    label: "picked-files",
  });
  await session.clickSelector('[data-testid="import-start"]');

  const deadline = Date.now() + 90000;
  let itemId = null;
  while (Date.now() < deadline && !itemId) {
    const ids = await session.evaluate(
      `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`
    );
    itemId = (ids ?? []).find((id) => !(before ?? []).includes(id)) ?? null;
    if (!itemId) await sleep(1000);
  }
  if (!itemId) throw new Error(`导入 ${path.basename(fixturePath)} 后未出现新的题库行`);
  return { itemId, entry: isPdf ? "pick-folder (pdf-only hook)" : "pick-files (source-files hook)", stagedPath };
}

/** 等识别落盘。
 *
 * ⚠️ 不能用 `view.batchId || view.chains` 当条件：`get_recognition_decision` 在**尚未**
 * 产生批次时也会返回一个视图，其 `chains` 是四条 `not_run`（见后端
 * `load_latest_batch` 为 None 的分支）。用 `|| chains` 会立刻退出，把「还没开始识别」
 * 误报成「四条链全 not_run」——这正是本轮先踩到的一个假失败。
 *
 * 正确的等待条件是**批次真的出现**（`batchId` 非空），或本地链进入终态。
 */
async function waitForDecision(itemId, timeoutMs = 180000) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  let lastBatchId = null;
  while (Date.now() < deadline) {
    const r = await call("get_recognition_decision", { itemId });
    if (r?.ok && r.value) {
      last = r.value;
      lastBatchId = last.batchId ?? null;
      const localState = last.chains?.local?.state ?? null;
      const terminalLocal = ["succeeded", "partial", "unusable", "failed", "canceled"].includes(localState);
      if (lastBatchId && terminalLocal) return last;
    }
    await sleep(2000);
  }
  return last;
}

async function readDraft(itemId) {
  const r = await call("get_workspace_item", { itemId });
  if (!r?.ok) throw new Error(`get_workspace_item 失败：${r?.error}`);
  return r.value;
}

/** 等权威稿落盘（本地识别是异步的，`ds` 要等本地识别落盘才非空）。 */
async function waitForDraft(itemId, timeoutMs = 120000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const d = await readDraft(itemId);
    if (d?.ds && typeof d.ds === "object" && Object.keys(d.ds).length) return d;
    await sleep(2000);
  }
  return null;
}

/** 权威稿里是否有真实内容（答案位非空 / 有题组）。 */
function draftHasContent(ds) {
  if (!ds || typeof ds !== "object") return false;
  const slots = ds.answerSlots && typeof ds.answerSlots === "object" ? Object.keys(ds.answerSlots) : [];
  const key = ds.answerKey && typeof ds.answerKey === "object" ? ds.answerKey : {};
  const resolved = slots.filter((id) => key[id] && key[id].kind && key[id].kind !== "unresolved");
  const groups = Array.isArray(ds.questionGroups) ? ds.questionGroups : [];
  return { slots: slots.length, resolvedSlots: resolved.length, groups: groups.length, hasContent: resolved.length > 0 || groups.length > 0 };
}

try {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);

  for (const f of fixturePaths) {
    if (!fs.existsSync(f)) throw new CannotRunError(`夹具不存在：${f}`);
  }
  report.identity.fixtures = fixturePaths.map((p) => ({ path: p, sha256: sha256File(p) }));

  // 两类导入入口同时可用：PDF 走「选择文件夹」（只列 runDir/pdfs 下的 PDF），
  // DOCX 走「选择文件」（读 PDF2TEST_AUTOMATION_SOURCE_FILES，任意扩展名）。
  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "docs"), { recursive: true });
  const staged = new Map();
  for (const f of fixturePaths) {
    const dest = path.join(runDir, /\.pdf$/i.test(f) ? "pdfs" : "docs", path.basename(f));
    fs.copyFileSync(f, dest);
    staged.set(f, dest);
  }
  const docxStaged = fixturePaths.filter((f) => !/\.pdf$/i.test(f)).map((f) => staged.get(f));
  report.identity.staged = Object.fromEntries([...staged].map(([k, v]) => [path.basename(k), v]));

  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    appEnv: docxStaged.length ? { PDF2TEST_AUTOMATION_SOURCE_FILES: docxStaged.join(path.delimiter) } : {},
  });
  report.identity.browserArgs = session.browserArgs;
  report.diagnosticRun = true;
  recorder = createStepRecorder({ session, artifactsDir: runDir });

  // ---- 1. 题库页 + 显式关闭云端（无 profile 之外再加一层保证）----
  await recorder.run("library-page-loads-cloud-off", async () => {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, {
      timeoutMs: 40000,
      label: "library-page",
    });
    await session.evaluate(
      `(() => { window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false })); location.hash = "#/library"; return true; })()`
    );
    await session.cdp.send("Page.reload", {}, 30000).catch(() => {});
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, {
      timeoutMs: 40000,
      label: "library-after-reload",
    });
    return { url: await session.evaluate("location.href"), cloudEnabled: false };
  });

  const perFixture = [];

  for (const fixturePath of fixturePaths) {
    const name = path.basename(fixturePath);
    const isPdf = /\.pdf$/i.test(fixturePath);
    const tag = isPdf ? "pdf" : "docx";

    // ---- 导入 ----
    let itemId = null;
    await recorder.run(`import-${tag}`, async () => {
      const imported = await importFixture(fixturePath, staged.get(fixturePath));
      itemId = imported.itemId;
      return imported;
    });

    // ---- 四条链 ----
    let chainEvidence = null;
    await recorder.run(`chains-${tag}`, async () => {
      const view = await waitForDecision(itemId);
      if (!view) throw new Error(`get_recognition_decision 在超时前没有返回视图（${name}）`);
      if (!view.chains) throw new Error(`识别视图缺少 chains 字段（${name}）`);

      const chains = {
        local: stage(view.chains.local),
        cloud: stage(view.chains.cloud),
        source: stage(view.chains.source),
        adjudication: stage(view.chains.adjudication),
      };
      const batchId = view.batchId ?? null;
      const states = Object.values(chains).map((c) => c.state);
      const notRunCount = states.filter((s) => s === "not_run").length;

      // (1) 有 batch：修复前这里是 null。
      if (!batchId) {
        throw new Error(
          `无云导入没有产生批次（batchId=null，chains=${JSON.stringify(states)}）——` +
            "正是「无云分支跳过 reconcile」的症状；本地证据链整体缺失。"
        );
      }
      // (2) 不能四条链全部 not_run。
      if (notRunCount === states.length) {
        throw new Error(`四条链全部 not_run（${JSON.stringify(chains)}）：reconcile 根本没有跑。`);
      }
      // (3) 本地链必须真的跑过。
      if (chains.local.state === "not_run") {
        throw new Error("本地链为 not_run：本地候选没有落盘（无云路径不应跳过本地链）。");
      }
      // (4) 原文核验必须真的跑过。
      if (chains.source.state === "not_run") {
        throw new Error("原文核验链为 not_run：核验没有跑（无云路径不应跳过核验）。");
      }
      // (5) 裁决必须真的跑过。
      if (chains.adjudication.state === "not_run") {
        throw new Error("裁决链为 not_run：裁决没有跑（无云路径不应跳过裁决）。");
      }
      // (6) 云端必须如实 not_run，且**带原因码**；绝不接受把无云谎报成 failed。
      if (chains.cloud.state !== "not_run") {
        throw new Error(
          `云端链应为 not_run（本次没有云端参与），实际 ${chains.cloud.state}` +
            (chains.cloud.reasonCode ? `（${chains.cloud.reasonCode}）` : "") +
            "——无云被折叠成失败会让用户去排查不存在的云端故障。"
        );
      }
      if (!chains.cloud.reasonCode) {
        throw new Error("云端 not_run 未携带原因码（契约要求 not_run 必须给出稳定原因）。");
      }
      if (!NOT_RUN_REASONS.has(chains.cloud.reasonCode)) {
        throw new Error(
          `云端 not_run 的原因码不是「没有云端参与」类（期望 ${[...NOT_RUN_REASONS].join("/")}），实际 ${chains.cloud.reasonCode}`
        );
      }

      // (7) 本地链状态必须**准确**：与权威稿是否真的产出内容一致。
      const draft = await waitForDraft(itemId);
      if (!draft) throw new Error(`权威稿迟迟没有落盘（ds 为空），无法核对本地链状态是否准确（${name}）`);
      const content = draftHasContent(draft.ds);
      if (chains.local.state === "succeeded" && !content.hasContent) {
        throw new Error(
          `本地链报 succeeded 但权威稿没有任何内容（${JSON.stringify(content)}）——状态与事实不符。`
        );
      }
      if (chains.local.state !== "succeeded" && content.hasContent) {
        throw new Error(
          `权威稿已有内容（${JSON.stringify(content)}）但本地链报 ${chains.local.state}——状态与事实不符。`
        );
      }

      chainEvidence = {
        fixture: name,
        itemId,
        batchId,
        editVersion: view.editVersion ?? null,
        baseEditVersion: view.baseEditVersion ?? null,
        stale: Boolean(view.stale),
        chains,
        notRunCount,
        allNotRun: notRunCount === states.length,
        summary: view.summary ?? null,
        actionableCount: Array.isArray(view.actionable) ? view.actionable.length : null,
        autoAppliedCount: Array.isArray(view.autoApplied) ? view.autoApplied.length : null,
        draftContent: content,
        localStateMatchesDraft: true,
      };
      return chainEvidence;
    });

    perFixture.push(chainEvidence);
  }

  // 夹具在 `chains-*` 步骤失败时，`chainEvidence` 还没被赋值就 push 进来的是 `null`。
  // 必须过滤掉，否则报告里出现 `[null, null]`，读起来像「夹具没跑」——
  // 真实情况是「跑了但失败了」，细节在 `steps` 里。
  const measured = perFixture.filter(Boolean);
  report.fixtures = measured;
  report.fixturesMeasured = measured.length;
  report.fixturesFailed = report.steps.filter((s) => s.name.startsWith("chains-") && s.status === "failed").length;
  // 注意：空数组上 `.every()` 恒为 true。若一个夹具都没测成，
  // `allFourChainsNotRun` 会被**空洞地**判成 true，读起来像「确认了四条链全没跑」，
  // 实际是「什么都没测到」。所以先要求 measured 非空。
  report.allFourChainsNotRun = measured.length > 0 && measured.every((f) => f.allNotRun);
  report.everyFixtureHasBatch = measured.length > 0 && measured.every((f) => Boolean(f.batchId));
  report.cloudAlwaysNotRun = measured.length > 0 && measured.every((f) => f.chains?.cloud?.state === "not_run");
  report.verdict = report.steps.every((s) => s.status === "passed") ? "passed" : "failed";
} catch (error) {
  const cannotRun = error instanceof CannotRunError;
  report.verdict = cannotRun ? "cannot-run" : "failed";
  report.fatal = { name: error.name, message: String(error.message), appOutput: error.appOutput ?? null };
  console.error(`[local-chain] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${error.message}`);
} finally {
  if (session) {
    if (!keep) await session.screenshot("final").catch(() => {});
    const closed = await session.close({ keep });
    report.appOutput = closed.appOutput?.slice(-4000) ?? null;
    report.exitCode = closed.exitCode;
    report.screenshotErrors = session.screenshotErrors;
  }
  report.finishedAt = new Date().toISOString();
  if (recorder) {
    report.steps = recorder.steps;
    const failed = report.steps.filter((s) => s.status === "failed");
    if (report.verdict !== "cannot-run") {
      report.verdict = failed.length || report.steps.length === 0 ? "failed" : "passed";
    }
    report.summary = { failed: failed.map((s) => s.name) };
  }
  const file = writeReport(runDir, report);
  console.log(`[local-chain] verdict=${report.verdict} report=${file}`);
  console.log(`[local-chain] steps: ${report.steps.map((s) => `${s.name}:${s.status}`).join(" | ")}`);
  for (const f of report.fixtures ?? []) {
    if (!f) continue;
    console.log(
      `[local-chain] ${f.fixture}: batch=${f.batchId} local=${f.chains.local.state} cloud=${f.chains.cloud.state}(${f.chains.cloud.reasonCode}) source=${f.chains.source.state} adjudication=${f.chains.adjudication.state}`
    );
  }
  process.exit(report.verdict === "passed" ? 0 : report.verdict === "cannot-run" ? 3 : 1);
}
