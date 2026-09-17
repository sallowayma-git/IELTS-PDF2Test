#!/usr/bin/env node
// 真实 Tauri 产品回归：**导出成功路径**（CDP 通道版）。
//
// 为什么另写一份而不是直接跑 `tauri-publish-ready.mjs`：
//   那份走 selenium + tauri-driver，本机 WebView2 下连续两次死在驱动握手
//   （`SessionNotCreatedError: session not created / chrome not reachable`、
//    `NoSuchSessionError: session deleted as the browser has closed the connection`），
//   连工作区都进不去。本仓更新的套件已经统一走 CDP，本脚本把「播种 ready 题稿 →
//   真实 UI 点发布 → 真实 NAS 包落盘」这条**成功路径**搬到 CDP 上，
//   使它与其余验收用同一条通道，结果可归因。
//
// 范围声明（与 `tauri-publish-ready.mjs` 一致，不得含糊）：
//   - 数据准备：仓库内 proven-ready authoring fixture + 派生物理阴影预置 job 目录，
//     应用启动迁移自动 seed canonical。**这不是真实 PDF/DOCX 自动识别的产物**。
//   - 因此本套件只证明「导出链（UI → 命令 → 质量门 → NAS 包）」，
//     不证明「自动识别能把真实 PDF 带到可导出」——后者由
//     `tauri-cdp-product-chain.mjs` 用真实夹具验证，本轮结论是**被门禁正确拦下**。
//   - 产物交给 `scripts/e2e/nas-student-contract.mjs` 做学生端可读性校验。
//
// 证据层级：product（真实进程 + WebView2 + SQLite + 文件系统）。

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
  sha256File,
  sleep,
  writeReport,
  repoRoot,
} from "./lib/tauri-cdp-harness.mjs";
import { describePreflightDisagreement, evaluatePublication } from "./lib/chain-verdict.mjs";
import { seedReadyJob } from "./lib/ready-fixture.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const diagnosticArgsRequested = process.argv.includes("--diagnostic-args");
const extraIdx = process.argv.indexOf("--extra-args");
const extraArgs = extraIdx >= 0
  ? (process.argv[extraIdx + 1] ?? "")
  : diagnosticArgsRequested ? "--no-sandbox --disable-gpu" : "";
const runDirIdx = process.argv.indexOf("--run-dir");
const runDir = runDirIdx >= 0
  ? path.resolve(process.argv[runDirIdx + 1])
  : path.join(repoRoot, "artifacts", "e2e-cdp", `run-publish-ready-${new Date().toISOString().replace(/[:.]/g, "-")}`);

const report = {
  task: "real-tauri-export-success-path-cdp",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  diagnosticRun: Boolean(extraArgs),
  runProfile: extraArgs ? "cdp-diagnostic" : "cdp-default",
  securityArgs: extraArgs ? extraArgs.split(/\s+/).filter(Boolean) : [],
  startedAt: new Date().toISOString(),
  runDir,
  identity: {
    exePath,
    exeSha256: null,
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: null,
    fixture: "fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json",
  },
  steps: [],
  verdict: "failed",
};

let session = null;
let recorder = null;

/**
 * 发布目录产物清点：只看磁盘，不相信步骤状态（步骤状态可能因为截图失败等原因偏乐观）。
 *
 * 运行时脚本按 **manifest 的 `entry.script`** 解析（当前布局是
 * `releases/<batchId>/<examId>.js`），不再用包根 `v2-p*.js` 通配 —— 后者是旧布局，
 * 会把**成功的**发布判成「没有题目 JS」（F-R14-6）。
 */
function collectPublicationFacts(nasDestination) {
  const facts = {
    nasDestination: nasDestination ?? null,
    runtimeScripts: [],
    missingRuntimeScripts: [],
    resourceManifestExists: false,
    resourceManifests: [],
    allFiles: [],
    manifestEntries: [],
  };
  if (!nasDestination || !fs.existsSync(nasDestination)) return facts;
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else facts.allFiles.push(path.relative(nasDestination, full));
    }
  };
  walk(nasDestination);

  const manifestPath = path.join(nasDestination, "manifest.js");
  if (fs.existsSync(manifestPath)) {
    try {
      const raw = fs.readFileSync(manifestPath, "utf8");
      const start = raw.indexOf("{");
      const end = raw.lastIndexOf("}");
      const library = JSON.parse(raw.slice(start, end + 1));
      facts.manifestSchemaVersion = library?._meta?.schemaVersion ?? null;
      for (const examId of Object.keys(library).filter((key) => key !== "_meta")) {
        const entry = library[examId] ?? {};
        const relative = String(entry.script ?? `${examId}.js`).replace(/^\.\//u, "");
        const resolved = path.join(nasDestination, relative);
        facts.manifestEntries.push({ examId, script: relative, schemaVersion: entry.schemaVersion ?? null });
        if (fs.existsSync(resolved)) facts.runtimeScripts.push(relative);
        else facts.missingRuntimeScripts.push(relative);
      }
    } catch (error) {
      facts.manifestParseError = String(error);
    }
  }

  const resourcesDir = path.join(nasDestination, "resources");
  if (fs.existsSync(resourcesDir)) {
    for (const examId of fs.readdirSync(resourcesDir)) {
      const manifest = path.join(resourcesDir, examId, "asset-manifest.json");
      if (fs.existsSync(manifest)) facts.resourceManifests.push(manifest);
    }
  }
  facts.resourceManifestExists = facts.resourceManifests.length > 0;
  return facts;
}

async function main() {
  const tolerateConcurrentEdits = process.argv.includes("--tolerate-concurrent-edits");
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);

  // 预置必须在**启动之前**：应用启动迁移会把 ready 题稿 seed 成 canonical。
  fs.mkdirSync(path.join(runDir, "appdata", "data"), { recursive: true });
  const seeded = seedReadyJob(path.join(runDir, "appdata", "data"));
  report.identity.seededJobId = seeded.jobId;
  console.log(`[publish-ready-cdp] seeded ready job ${seeded.jobId}`);

  const nasDestination = path.join(runDir, "nas-library");
  session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  report.identity.browserArgs = session.browserArgs;
  recorder = createStepRecorder({ session, artifactsDir: runDir });

  await recorder.run("library-page-loads", async () => {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 60000, label: "library-page" });
    await session.screenshot("01-library");
    return { url: await session.evaluate("location.href") };
  });

  await recorder.run("ready-item-visible-in-library", async () => {
    await session.waitFor(
      `!!document.querySelector('[data-item-id="${seeded.jobId}"]')`,
      { timeoutMs: 30000, label: "ready-row" }
    );
    const rowText = await session.evaluate(
      `(() => { const row = document.querySelector('[data-item-id="${seeded.jobId}"]'); return row ? row.innerText.replace(/\\s+/g,' ').trim() : null; })()`
    );
    return { jobId: seeded.jobId, rowText };
  });

  await recorder.run("workspace-opens-for-ready-item", async () => {
    // 发布目标目录：`readAppSettings()` 每次调用都重读 localStorage，所以这里写进去
    // 立即生效，**不需要**刷新页面（刷新会打断 CDP 会话，见 F-R14-3）。
    await session.evaluate(
      `(() => {
        const key = "ielts-author-studio.app-settings.v1";
        const raw = window.localStorage.getItem(key);
        const settings = raw ? JSON.parse(raw) : {};
        settings.nasDestination = ${JSON.stringify(nasDestination)};
        window.localStorage.setItem(key, JSON.stringify(settings));
        return settings.nasDestination;
      })()`
    );
    await session.clickSelector(`[data-item-id="${seeded.jobId}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 60000, label: "workspace" });
    const loadError = await session.evaluate(
      `(() => { const el = document.querySelector('.workspace-load-error'); return el ? el.innerText.replace(/\\s+/g,' ').trim() : null; })()`
    );
    if (loadError) throw new Error(`工作区加载失败：${loadError}`);
    await session.screenshot("02-workspace");
    return { itemId: seeded.jobId, nasDestination };
  });

  const readPreflight = async () => {
    const r = await session.invoke("get_publish_preflight", { jobId: seeded.jobId });
    if (!r?.ok || !r.value) return { error: r?.error ?? "no-invoke" };
    return {
      passed: r.value.passed === true,
      editVersion: r.value.editVersion ?? null,
      blockers: (r.value.blockers ?? []).map((b) => ({ code: b.code, targetId: b.targetId ?? null, internal: b.internal ?? null })),
    };
  };

  await recorder.run("publish-via-workspace-button", async () => {
    // 发布**前**读一次门禁：它是「为什么发得出去 / 发不出去」的权威依据。
    const gateBefore = await readPreflight();

    await session.clickSelector('[data-testid="workspace-publish"]');
    const outcome = await session.waitFor(
      `(() => {
        const notices = [...document.querySelectorAll('.workspace-notice')].map(n => n.innerText.replace(/\\s+/g,' ').trim());
        const joined = notices.join(' || ');
        if (/发布完成|发布失败|不能发布|问题|拦/.test(joined)) return { notices };
        return null;
      })()`,
      { timeoutMs: 180000, label: "publish-outcome" }
    );
    await session.screenshot("03-publish-outcome");
    const joined = (outcome.notices ?? []).join(" || ");
    const blocked = !/发布完成/.test(joined);
    // 发布**后**再读一次门禁。
    //
    // 为什么必须两次：实测出现过「发布前 `passed=false`（`QUALITY_NOT_READY /
    // quality_state=review_required`）而发布**成功**」。这有两种解释，处置完全不同：
    //   (a) 预检的 `passed` 比「可发布」更严 → 产品自相矛盾，要报缺陷；
    //   (b) 发布前读得太早（canonical 迁移/质量重算还没落定）→ 只是读取时机问题。
    // 只读一次就下结论，必然把 (b) 误报成 (a)。两次一起看才能区分。
    const gateAfter = await readPreflight();
    return {
      outcome: blocked ? "blocked_by_quality_gate" : "published",
      notices: outcome.notices ?? [],
      nasDestination,
      manifestExists: fs.existsSync(path.join(nasDestination, "manifest.js")),
      preflightBefore: gateBefore,
      preflightAfter: gateAfter,
      // 保留 `preflight` 字段名兼容既有判定代码：用**发布后**那次作为权威读数。
      preflight: gateAfter,
      preflightContradiction:
        gateBefore?.passed === false && !blocked && gateAfter?.passed === true
          ? "发布前 passed=false、发布后 passed=true —— 属于读取时机（canonical/质量尚未落定），不是产品矛盾"
          : gateBefore?.passed === false && !blocked && gateAfter?.passed === false
            ? "发布前 passed=false、发布后仍 passed=false，而发布已成功 —— 预检与可发布性口径不一致，需后端判定"
            : null,
    };
  });

  report.verdict = "pending";
}

try {
  await main();
} catch (error) {
  const cannotRun = error instanceof CannotRunError;
  report.cannotRun = cannotRun;
  report.fatal = { name: error.name, message: String(error.message), appOutput: error.appOutput ?? null };
  console.error(`[publish-ready-cdp] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${error.message}`);
} finally {
  if (session) {
    if (!keep) await session.screenshot("final").catch(() => {});
    const closed = await session.close({ keep });
    report.appOutput = closed.appOutput?.slice(-6000) ?? null;
    report.appProcessExitCode = closed.exitCode;
    report.screenshotErrors = session.screenshotErrors;
  }
  report.finishedAt = new Date().toISOString();
  if (recorder) report.steps = recorder.steps;

  // 判定**无条件**执行（含 CANNOT-RUN 分支）：否则「判定没跑」会退化成初始的 failed
  // 或未定义的退出码，看起来像跑过。
  const publishStep = report.steps.find((s) => s.name === "publish-via-workspace-button");
  const publicationFacts = collectPublicationFacts(publishStep?.detail?.nasDestination);
  const publicationFailures = report.cannotRun ? ["CANNOT-RUN：链路没跑起来，不判产物"] : evaluatePublication({
    publishDetail: publishStep?.detail,
    publicationFacts,
  });
  report.publication = {
    ...publicationFacts,
    failures: publicationFailures,
    preflightDisagreement: describePreflightDisagreement(publishStep?.detail),
  };

  const failed = report.steps.filter((s) => s.status === "failed").map((s) => s.name);
  const blocked = report.steps.filter((s) => s.status === "blocked").map((s) => s.name);
  const required = ["library-page-loads", "ready-item-visible-in-library", "workspace-opens-for-ready-item", "publish-via-workspace-button"];
  const missing = required.filter((name) => !report.steps.some((s) => s.name === name));

  if (report.cannotRun) {
    report.verdict = "cannot-run";
  } else if (failed.length || missing.length) {
    report.verdict = "failed";
    report.reason = `步骤失败或缺失：${[...failed, ...missing].join(", ")}`;
  } else if (blocked.length) {
    report.verdict = "blocked";
    report.reason = `必需步骤被质量门禁阻断：${blocked.join(", ")}。发布未发生，不得计为通过。`;
  } else if (publicationFailures.length) {
    report.verdict = "failed";
    report.reason = `发布显示成功但产物不完整：${publicationFailures.join("；")}`;
  } else {
    report.verdict = "passed";
  }

  const exitCode = { passed: 0, failed: 2, "cannot-run": 3, blocked: 4 }[report.verdict];
  const file = writeReport(runDir, report);
  console.log(`[publish-ready-cdp] verdict=${report.verdict} exit=${exitCode}${report.reason ? ` reason=${report.reason}` : ""}`);
  console.log(`[publish-ready-cdp] report=${file}`);
  console.log(`[publish-ready-cdp] steps: ${report.steps.map((s) => `${s.name}:${s.status}`).join(" | ") || "(无)"}`);
  process.exit(exitCode);
}
