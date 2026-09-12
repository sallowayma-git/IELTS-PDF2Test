#!/usr/bin/env node
// 真实 Tauri 产品回归：取消与断电式重启恢复（G1/A4-F02 + P0-2，不需要高质量识别语料）。
//
// 场景：
//   A. 批量导入 6 份 PDF（并发上限 3，后段任务必然排队）→ 打开靠后条目的工作区
//      → 菜单「停止识别」→ 该行必须显示「已取消」（诚实状态，不是"识别失败"）。
//   B. 断电式重启（taskkill 强杀 + 同一 dataDir 重启）：
//      - 已取消行必须仍是「已取消」（durable 取消标记跨重启兑现，不得复活）；
//      - 其余运行中/排队中任务经启动恢复（interrupted → requeue）后在限时内
//        达到稳定诚实状态（待检查/可发布/失败），不得永久停在"排队中"。
//
// 证据层级：product（真实进程 + WebView2 + SQLite + 文件系统 + 进程强杀）。

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { By, until } from "selenium-webdriver";
import {
  DEFAULT_EXE, DEFAULT_PDF, CannotRunError, assertPrerequisites, assertFreshBuild, buildFreshness,
  buildIdentity, launchTauriApp, killAppProcess, createStepRecorder, importPdfViaFolderHook,
  writeReport, exitCodeForVerdict, logCannotRun, logHarnessError, sleep
} from "./lib/tauri-harness.mjs";

const args = Object.fromEntries(process.argv.slice(2).reduce((all, arg, index, list) => {
  if (arg.startsWith("--")) all.set(arg.slice(2), list[index + 1]?.startsWith("--") ? true : list[index + 1] ?? true);
  return all;
}, new Map()));
const exePath = path.resolve(args.exe ?? DEFAULT_EXE);
const pdfPath = path.resolve(args.pdf ?? DEFAULT_PDF);
const keepRun = Boolean(args.keep);
const takeScreenshots = args.screenshot !== false;
const IMPORT_COUNT = 6;

async function rowText(driver, itemId) {
  const rows = await driver.findElements(By.css(`[data-item-id="${itemId}"]`));
  if (!rows.length) return null;
  return (await rows[0].getText()).replace(/\s+/g, " ").trim();
}

async function pollRowText(driver, itemId, predicate, timeoutMs, intervalMs = 500) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    last = await rowText(driver, itemId);
    if (last !== null && predicate(last)) return { text: last, timedOut: false };
    await sleep(intervalMs);
  }
  return { text: last, timedOut: true };
}

/** 打开条目工作区并通过菜单「停止识别」请求取消。 */
async function cancelViaWorkspaceMenu(driver, itemId) {
  const row = await driver.wait(
    until.elementLocated(By.css(`[data-item-id="${itemId}"] .library-row-main`)),
    15000
  );
  await row.click();
  await driver.wait(until.elementLocated(By.css('[data-testid="exam-workspace"]')), 15000);
  await driver.findElement(By.css('button[aria-label="更多操作"]')).click();
  const stopButton = await driver.wait(async () => {
    const items = await driver.findElements(By.css('button[role="menuitem"]'));
    for (const item of items) {
      if ((await item.getText()).includes("停止识别")) return item;
    }
    return null;
  }, 5000, "菜单中未找到「停止识别」");
  await stopButton.click();
}

/** 选一个尚未完成的条目（优先靠后入队的，必然还在排队/运行）。 */
async function pickUnfinishedItem(driver, ids) {
  for (const id of [...ids].reverse()) {
    const text = await rowText(driver, id);
    if (text !== null && !/待检查|可发布|失败|已发布/.test(text)) return id;
  }
  return ids[ids.length - 1];
}

async function main() {
  assertPrerequisites({ exePath, pdfPath });
  const freshness = buildFreshness(exePath);
  assertFreshBuild(freshness);
  const identity = buildIdentity(exePath);

  const session = await launchTauriApp({ exePath, pdfPath, keep: keepRun, runPrefix: "cancel-recovery" });
  const artifacts = { dir: session.runDir, screenshotErrors: [] };
  const { steps, recordStep } = createStepRecorder({ artifacts, takeScreenshots });
  let driver = session.driver;
  let cancelledId = null;
  let workerIds = [];

  try {
    await recordStep(driver, "library-page-loads", async () => {
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      await driver.executeScript(`
        window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false }));
        location.hash = "#/library";
      `);
      await driver.navigate().refresh();
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      // 批量导入素材：复制 6 份不同文件名的 PDF 到导入目录（点击导入时读取）。
      const base = fs.readFileSync(pdfPath);
      for (let index = 2; index <= IMPORT_COUNT; index += 1) {
        fs.copyFileSync(pdfPath, path.join(session.pdfDir, `cancel-recovery-${String(index).padStart(2, "0")}.pdf`));
      }
      return { url: await driver.getCurrentUrl(), preparedFiles: IMPORT_COUNT };
    });

    await recordStep(driver, "import-batch-via-folder-hook", async () => {
      // 一次性选中目录：6 份文件全部入队。
      const result = await importPdfViaFolderHook(driver);
      // folder hook 只回报一个新 id；从题库抓出本次全部新行（按数量差分）。
      const rows = await driver.findElements(By.css('[data-testid="library-row"]'));
      const ids = [];
      for (const row of rows) ids.push(await row.getAttribute("data-item-id"));
      if (ids.length < IMPORT_COUNT) throw new Error(`导入后行数不足：${ids.length} < ${IMPORT_COUNT}`);
      workerIds = ids;
      cancelledId = result.itemId;
      return { imported: ids.length, sampleId: cancelledId };
    });

    await recordStep(driver, "cancel-latest-item-via-workspace-menu", async () => {
      // 打开尚未完成的条目（并发 3，靠后条目必然仍在排队）→ 菜单「停止识别」。
      const target = await pickUnfinishedItem(driver, workerIds);
      await cancelViaWorkspaceMenu(driver, target);
      cancelledId = target;
      // 断言发生在题库页：工作区里没有题库行可轮询。
      await driver.executeScript("location.hash = '#/library';");
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 15000);
      // 行必须显示「已取消」（诚实状态文案，见 libraryTypes detailFor）。
      const outcome = await pollRowText(driver, target, (text) => text.includes("已取消"), 30000);
      if (outcome.timedOut) {
        throw new Error(`取消后 30s 行未显示「已取消」；最后文本：${outcome.text}`);
      }
      if (/识别失败/.test(outcome.text)) {
        throw new Error(`取消被显示成失败：${outcome.text}`);
      }
      return { cancelledId: target, rowText: outcome.text };
    });

    // ── 断电式重启：强杀 + 同一 dataDir 重启 ──
    await recordStep(driver, "hard-kill-app", async () => {
      const beforeKill = {};
      for (const id of workerIds) beforeKill[id] = await rowText(driver, id);
      const killed = killAppProcess();
      if (!killed) throw new Error("taskkill 未能终止被测应用");
      await sleep(2000);
      return { killed: true, rowTextsAtKill: beforeKill };
    });

    session.cleanup({ removeRunDir: false }).catch(() => {});
    driver = null;
    const relaunch = await launchTauriApp({
      exePath, pdfPath, keep: keepRun, runPrefix: "cancel-recovery", runDirOverride: session.runDir
    });
    session.__relaunched = relaunch;
    driver = relaunch.driver;

    await recordStep(driver, "relaunched-same-data-dir", async () => {
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      return { dataDir: session.dataDir, reused: true };
    });

    await recordStep(driver, "durable-cancel-survives-restart", async () => {
      // 已取消行不得复活为处理中，也不得显示排队/识别文案。
      const outcome = await pollRowText(driver, cancelledId, (text) => text !== null && text.includes("已取消"), 60000);
      if (outcome.timedOut) {
        throw new Error(`重启后取消状态丢失；最后文本：${outcome.text}`);
      }
      if (/排队中|正在读取|识别中/.test(outcome.text)) {
        throw new Error(`重启后已取消任务被复活：${outcome.text}`);
      }
      return { cancelledId, rowTextAfterRestart: outcome.text };
    });

    await recordStep(driver, "interrupted-workers-recover-to-honest-stable-state", async () => {
      // 运行中/排队中任务经启动恢复后，限时内达到稳定状态（不再有 Working 悬挂）。
      const pending = workerIds.filter((id) => id !== cancelledId);
      const deadline = Date.now() + 300000;
      const finalTexts = {};
      const stillUnstable = new Set(pending);
      while (Date.now() < deadline && stillUnstable.size) {
        for (const id of [...stillUnstable]) {
          const text = await rowText(driver, id);
          if (text === null) continue; // 行暂未渲染，下一轮再看
          if (/待检查|可发布|失败|已取消|已发布/.test(text)) {
            finalTexts[id] = text;
            stillUnstable.delete(id);
          }
        }
        await sleep(1500);
      }
      if (stillUnstable.size) {
        const stuck = {};
        for (const id of stillUnstable) stuck[id] = await rowText(driver, id);
        throw new Error(`重启恢复后仍有任务未达稳定状态：${JSON.stringify(stuck)}`);
      }
      const recovered = Object.entries(finalTexts).filter(([, text]) => /待检查|可发布/.test(text)).length;
      if (!recovered) throw new Error(`没有任务恢复到可检查状态：${JSON.stringify(finalTexts)}`);
      return { total: pending.length, stabilized: Object.keys(finalTexts).length, finalTexts };
    });

    const failed = steps.filter((step) => step.status === "failed");
    const report = {
      runId: path.basename(session.runDir),
      scenario: "cancel-and-restart-recovery",
      coverage: "real-tauri-process+webview2+sqlite+filesystem+hard-kill",
      evidenceLevel: "product",
      exe: exePath,
      ...identity,
      ...freshness,
      pdf: pdfPath,
      steps,
      verdict: failed.length ? "failed" : "passed",
      driverStderr: (session.__relaunched?.driverStderr ?? session.driverStderr)().slice(0, 4000)
    };
    writeReport(session.runDir, report);
    process.exitCode = exitCodeForVerdict(report.verdict);
  } catch (error) {
    logHarnessError(error);
    fs.writeFileSync(path.join(session.runDir, "harness-error.json"), JSON.stringify({
      error: String(error),
      driverStderr: (session.__relaunched?.driverStderr ?? session.driverStderr)().slice(0, 8000)
    }, null, 2));
    process.exitCode = 2;
  } finally {
    const finalSession = session.__relaunched ?? session;
    try { await finalSession.cleanup(); } catch {}
    try { fs.rmSync(session.runDir, { recursive: true, force: true }); } catch {}
  }
}

try {
  await main();
} catch (error) {
  if (error instanceof CannotRunError) {
    logCannotRun(error);
    process.exit(3);
  }
  logHarnessError(error);
  process.exit(2);
}
