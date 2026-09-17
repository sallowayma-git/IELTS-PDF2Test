#!/usr/bin/env node
/**
 * 观测（不断言）：无云导入后，批次究竟会不会出现、多久出现、有没有冻结失败日志。
 *
 * 为什么需要它：`tauri-cdp-local-chain.mjs` 在 14:13 的一次运行里两个夹具都拿不到批次
 * （`batchId=null`、四链全 `not_run`），而 14:18 的一次实验里同一个 exe 却在导入后
 * 约 35 秒出现了批次。同一个二进制、同样步骤、结果不同 ⇒ 必须先把它**观测清楚**，
 * 再决定怎么报告，不能凭一次运行下结论。
 *
 * 本脚本刻意**不调用** `get_workspace_item` / `get_publish_preflight`——它们会
 * `migrate_single_item` 播种权威稿，而播种正是被怀疑的竞态变量。只读决策视图。
 *
 * 两种场景由 `--open-workspace` 选择，**必须分开跑**（不能在同一进程里连着做）：
 *   A) 默认：导入后**不打开工作区**，只看批次自己会不会出现；
 *   B) `--open-workspace`：导入后**立即**导航到 `#/items/<id>`，其余观测方式与 A 完全一致。
 * 若把 B 塞进 A 的同一次运行，就分不清批次是因为开了工作区才出现、还是本来就会出现——
 * 两次观测的差异必须是**唯一变量**（开不开工作区）造成的。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-import-batch-timeline.mjs [--open-workspace] [--window-ms N] [--fixture PATH] [--keep]
 *
 * 退出码：0 = 观测完成（无论是否出现批次）；3 = CANNOT-RUN。
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
const windowIdx = process.argv.indexOf("--window-ms");
const windowMs = windowIdx >= 0 ? Number(process.argv[windowIdx + 1]) : 300000;
const openWorkspace = process.argv.includes("--open-workspace");
const fixtureIdx = process.argv.indexOf("--fixture");
const fixturePath = path.resolve(
  fixtureIdx >= 0
    ? process.argv[fixtureIdx + 1]
    : path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf")
);
const isPdf = /\.pdf$/i.test(fixturePath);
const runDir = path.join(
  repoRoot,
  "artifacts",
  "e2e-cdp",
  `run-import-timeline-${new Date().toISOString().replace(/[:.]/g, "-")}`
);

const report = {
  task: "no-cloud-import-batch-timeline",
  scope: "观测：无云导入后批次是否/何时出现（不断言，不调用会播种权威稿的命令）",
  scenario: openWorkspace ? "import-then-open-workspace" : "import-without-opening-workspace",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  startedAt: new Date().toISOString(),
  runDir,
  windowMs,
  identity: {
    exePath,
    exeSha256: null,
    fixturePath,
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: null,
  },
  timeline: [],
  verdict: "failed",
};

let session = null;

async function call(command, args) {
  const r = await session.invoke(command, args);
  if (r && r.__noInvoke) throw new Error("invoke 不可用");
  return r;
}

const stateOf = (view) => ({
  batchId: view?.batchId ?? null,
  local: view?.chains?.local?.state ?? null,
  cloud: view?.chains?.cloud?.state ?? null,
  cloudReason: view?.chains?.cloud?.reasonCode ?? null,
  source: view?.chains?.source?.state ?? null,
  sourceReason: view?.chains?.source?.reasonCode ?? null,
  adjudication: view?.chains?.adjudication?.state ?? null,
});

try {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);
  if (!fs.existsSync(fixturePath)) throw new CannotRunError(`夹具不存在：${fixturePath}`);
  report.identity.fixtureSha256 = sha256File(fixturePath);

  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  const staged = path.join(runDir, "pdfs", path.basename(fixturePath));
  fs.copyFileSync(fixturePath, staged);

  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: "--no-sandbox --disable-gpu",
    appEnv: isPdf ? {} : { PDF2TEST_AUTOMATION_SOURCE_FILES: staged },
  });
  report.identity.browserArgs = session.browserArgs;

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

  const before = await session.evaluate(
    `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`
  );
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
  await session.clickSelector(isPdf ? '[data-testid="import-pick-folder"]' : '[data-testid="import-pick-files"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked-files" });
  const importedAt = Date.now();
  await session.clickSelector('[data-testid="import-start"]');

  let itemId = null;
  const findDeadline = Date.now() + 90000;
  while (Date.now() < findDeadline && !itemId) {
    const ids = await session.evaluate(
      `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`
    );
    itemId = (ids ?? []).find((id) => !(before ?? []).includes(id)) ?? null;
    if (!itemId) await sleep(1000);
  }
  if (!itemId) throw new CannotRunError("导入后未出现新的题库行");
  report.itemId = itemId;
  report.importedAt = new Date(importedAt).toISOString();

  // 场景 B：导入后**立即打开工作区**。
  //
  // 走**真实导航**（把 hash 改到 `#/items/<id>`）触发产品自己的打开路径，
  // 而不是脚本代劳去调 `get_workspace_item`——后者测的是脚本、不是产品，
  // 也绕开了「打开工作区会播种权威稿」这个真正想观测的变量。
  if (openWorkspace) {
    report.workspaceNavigatedMs = Date.now() - importedAt;
    await session.evaluate(
      `(() => { window.location.hash = "#/items/" + ${JSON.stringify(itemId)}; return true; })()`
    );
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, {
      timeoutMs: 40000,
      label: "exam-workspace",
    });
    report.workspaceVisibleMs = Date.now() - importedAt;
  }

  // 只读轮询：不调用任何会播种权威稿的命令。
  const deadline = Date.now() + windowMs;
  let firstBatchAtMs = null;
  let last = null;
  while (Date.now() < deadline) {
    const r = await call("get_recognition_decision", { itemId });
    const elapsed = Date.now() - importedAt;
    if (r?.ok && r.value) {
      last = stateOf(r.value);
      if (last.batchId && firstBatchAtMs === null) firstBatchAtMs = elapsed;
      report.timeline.push({ elapsedMs: elapsed, ...last });
      // 批次出现且本地链已终态即可停；否则一直观察到窗口结束。
      const terminal = ["succeeded", "partial", "unusable", "failed", "canceled"].includes(last.local);
      if (last.batchId && terminal) break;
    } else {
      report.timeline.push({ elapsedMs: elapsed, error: r?.error ?? "no value" });
    }
    await sleep(3000);
  }

  report.firstBatchObservedMs = firstBatchAtMs;
  report.finalState = last;
  report.batchAppeared = Boolean(last?.batchId);
  report.observation = report.batchAppeared
    ? `批次在导入后约 ${Math.round(firstBatchAtMs / 1000)} 秒出现（窗口 ${Math.round(windowMs / 1000)} 秒）。`
    : `窗口 ${Math.round(windowMs / 1000)} 秒内**始终没有**批次（最终 ${JSON.stringify(last)}）。`;
  report.verdict = "passed";
} catch (error) {
  const cannotRun = error instanceof CannotRunError;
  report.verdict = cannotRun ? "cannot-run" : "failed";
  report.fatal = { name: error.name, message: String(error.message) };
  console.error(`[timeline] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${error.message}`);
} finally {
  if (session) {
    if (!keep) await session.screenshot("final").catch(() => {});
    const closed = await session.close({ keep });
    // 完整应用输出落盘（不截断）：冻结失败等关键行可能出现在很靠前的位置。
    const logFile = path.join(runDir, "app-output.log");
    fs.writeFileSync(logFile, closed.appOutput ?? "");
    report.appOutputFile = logFile;
    report.freezeFailures = String(closed.appOutput ?? "")
      .split(/\r?\n/)
      .filter((line) => /freeze local candidate snapshot failed|base edit version unreadable|canonical_not_seeded/.test(line));
    report.exitCode = closed.exitCode;
  }
  report.finishedAt = new Date().toISOString();
  const file = writeReport(runDir, report);
  console.log(`[timeline] verdict=${report.verdict} report=${file}`);
  console.log(`[timeline] batchAppeared=${report.batchAppeared} firstBatchObservedMs=${report.firstBatchObservedMs}`);
  console.log(`[timeline] final=${JSON.stringify(report.finalState)}`);
  console.log(`[timeline] freezeFailures=${JSON.stringify(report.freezeFailures ?? [])}`);
  console.log(`[timeline] observation=${report.observation ?? report.fatal?.message}`);
  process.exit(report.verdict === "passed" ? 0 : report.verdict === "cannot-run" ? 3 : 1);
}
