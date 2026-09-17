#!/usr/bin/env node
/**
 * Task 1 最小冒烟：真实 Tauri 应用 + WebView2 CDP 通道。
 *
 * 顺序（与任务书一致）：启动 → 页面加载 → 一次 DOM 读取 → 一次真实 UI 点击 → 一次 Tauri 命令交互。
 *
 * 用法：node scripts/e2e/tauri-cdp-smoke.mjs [--keep] [--run-dir <dir>]
 * 退出码：0 = 全部步骤通过；3 = CANNOT-RUN（环境/构建问题）；1 = 步骤失败。
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
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const runDirIdx = process.argv.indexOf("--run-dir");
const extraIdx = process.argv.indexOf("--extra-args");
// 环境必需的诊断参数（见 CDP_CHANNEL_NOTE）：本沙箱环境下 WebView2 的 renderer
// 在不加这两个开关时会中途崩溃（`CDP 连接已关闭`）。它们放宽了渲染进程沙箱与 GPU
// 路径，因此所有以此运行得到的结论都必须标注「诊断参数运行」，不得写成默认产品路径通过。
const extraArgs = extraIdx >= 0 ? (process.argv[extraIdx + 1] ?? "") : "";
const runDir =
  runDirIdx >= 0
    ? path.resolve(process.argv[runDirIdx + 1])
    : path.join(repoRoot, "artifacts", "e2e-cdp", `run-smoke-${new Date().toISOString().replace(/[:.]/g, "-")}`);

const report = {
  task: "task1-minimal-smoke",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  startedAt: new Date().toISOString(),
  runDir,
  identity: {
    exePath,
    exeSha256: null,
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: null,
  },
  steps: [],
  verdict: "failed",
};

let session = null;
let recorder = null;

try {
  const fresh = assertBuildFresh({ exePath });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);

  session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  report.identity.browserArgs = session.browserArgs;
  report.diagnosticRun = Boolean(extraArgs);
  recorder = createStepRecorder({ session, artifactsDir: runDir });

  await recorder.run("app-launch-and-page-load", async () => {
    const text = await session.evaluateRetry("document.body.innerText");
    if (!String(text).trim()) throw new Error("页面 body 为空");
    return { excerpt: String(text).slice(0, 120) };
  });

  await recorder.run("dom-read-app-shell", async () => {
    const shell = await session.evaluateRetry(
      "({ title: document.title, href: location.href, hasInvoke: !!(window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke), navCount: document.querySelectorAll('nav a, nav button, aside button, [role=tab]').length })"
    );
    if (!shell.hasInvoke) throw new Error("页面没有 __TAURI_INTERNALS__.invoke，说明不是真实 Tauri 运行时");
    if (shell.navCount < 1) throw new Error("未找到任何导航元素，页面可能未挂载");
    return shell;
  });

  await recorder.run("tauri-command-roundtrip", async () => {
    const result = await session.invoke("list_llm_profiles");
    if (result && result.__noInvoke) throw new Error("invoke 不可用");
    if (!result || result.ok !== true) throw new Error(`命令调用失败：${JSON.stringify(result)}`);
    return { command: "list_llm_profiles", valueType: Array.isArray(result.value) ? `array(${result.value.length})` : typeof result.value };
  });

  await recorder.run("real-ui-click-settings-tab", async () => {
    const before = await session.evaluate("document.body.innerText.slice(0, 400)");
    const box = await session.clickByText("设置", { exact: true, timeoutMs: 15000 });
    await new Promise((r) => setTimeout(r, 1500));
    const after = await session.evaluate("document.body.innerText.slice(0, 400)");
    const changed = String(before) !== String(after);
    await session.screenshot("after-click-settings");
    return { clickedAt: { x: Math.round(box.x), y: Math.round(box.y) }, textChanged: changed, afterExcerpt: String(after).slice(0, 160) };
  });

  await recorder.run("page-survives-after-interaction", async () => {
    const info = await session.evaluateRetry("({ readyState: document.readyState, bodyLen: document.body.innerHTML.length })");
    if (info.readyState !== "complete") throw new Error(`readyState=${info.readyState}`);
    return info;
  });

  report.verdict = report.steps.every((s) => s.status === "passed") ? "passed" : "failed";
} catch (error) {
  const cannotRun = error instanceof CannotRunError;
  report.verdict = cannotRun ? "cannot-run" : "failed";
  report.fatal = { name: error.name, message: String(error.message), appOutput: error.appOutput ?? null };
  console.error(`[smoke] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${error.message}`);
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
    // 判定必须在步骤收集完成之后再算，否则空数组的 every() 会把失败写成通过。
    const failed = report.steps.filter((s) => s.status === "failed");
    if (report.verdict === "passed" && failed.length > 0) report.verdict = "failed";
    if (report.verdict === "passed" && report.steps.length === 0) report.verdict = "failed";
  }
  const file = writeReport(runDir, report);
  console.log(`[smoke] verdict=${report.verdict} report=${file}`);
  console.log(`[smoke] steps: ${report.steps.map((s) => `${s.name}:${s.status}`).join(" | ")}`);
  process.exit(report.verdict === "passed" ? 0 : report.verdict === "cannot-run" ? 3 : 1);
}
