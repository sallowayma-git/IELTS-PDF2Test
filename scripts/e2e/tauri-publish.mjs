#!/usr/bin/env node
// 真实 Tauri E2E —— 发布链（计划 §18 P0-T02 / §19.7，audit A11-F02 缺失脚本之一）。
//
// 覆盖层级：**product**。驱动真实 Tauri 进程 + WebView2 + 真实 SQLite + 真实文件系统，
// 发布产物写入本次运行的隔离目录（PDF2TEST_AUTOMATION_EXPORT_DIR）。
//
// 场景：题库导入 PDF -> 后台识别到稳定阶段 -> 打开工作区 -> 点「发布」-> 断言发布结果。
//
// 重要口径（AGENTS.md / audit A11-F07）：仓库现有语料 PDF 达不到 ready 质量门，
// 因此「被质量门阻止」是**如实记录的产品行为，不计为通过**：本脚本以 verdict=blocked、
// 退出码 4 报告，绝不把「门禁阻止」写成 passed。只有真正写出发布产物才算 passed。
//
// 用法：
//   node scripts/e2e/tauri-publish.mjs [--exe path] [--pdf path] [--keep] [--no-screenshot]
//   npm run e2e:tauri:publish
//
// 退出码：0 passed / 1 failed / 2 harness error / 3 cannot-run / 4 blocked（见 lib/tauri-harness.mjs）。

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  DEFAULT_EXE, DEFAULT_PDF, By, CannotRunError, assertPrerequisites, buildFreshness,
  createStepRecorder, exitCodeForVerdict, importPdfViaFolderHook, launchTauriApp,
  logCannotRun, logHarnessError, openWorkspaceForItem, parseArgs, until, waitForRowStage, writeReport
} from "./lib/tauri-harness.mjs";

const args = parseArgs(process.argv.slice(2));
const exePath = path.resolve(args.exe ?? DEFAULT_EXE);
const pdfPath = path.resolve(args.pdf ?? DEFAULT_PDF);
const keepRun = Boolean(args.keep);
const takeScreenshots = args.screenshot !== false;

/** 质量门阻止的文案（经 userFacingError 收敛后的人话）——不计为通过，也不计为缺陷。 */
const GATE_BLOCK_PATTERN = /补齐|未完成|还没有|请先|待确认|需要确认/;

async function main() {
  assertPrerequisites({ exePath, pdfPath });
  const freshness = buildFreshness(exePath);

  const session = await launchTauriApp({ exePath, pdfPath, keep: keepRun, runPrefix: "publish" });
  const artifacts = { dir: session.runDir, screenshotErrors: [] };
  const { steps, recordStep } = createStepRecorder({ artifacts, takeScreenshots });
  const driver = session.driver;
  let itemId = null;
  let loadErrorText = null;

  try {
    await recordStep(driver, "library-page-loads", async () => {
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      await driver.executeScript(`
        window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false }));
        location.hash = "#/library";
      `);
      await driver.navigate().refresh();
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      return { url: await driver.getCurrentUrl() };
    });

    await recordStep(driver, "import-pdf-via-folder-hook", async () => {
      const result = await importPdfViaFolderHook(driver);
      itemId = result.itemId;
      return result;
    });

    await recordStep(driver, "background-pipeline-reaches-stable-stage", async () => {
      if (!itemId) throw new Error("缺少导入 item id");
      return waitForRowStage(driver, itemId, 240000);
    });

    await recordStep(driver, "workspace-opens", async () => {
      await openWorkspaceForItem(driver, itemId);
      const loadErrors = await driver.findElements(By.css(".workspace-load-error"));
      if (loadErrors.length) {
        loadErrorText = (await loadErrors[0].getText()).replace(/\s+/g, " ").trim();
        throw new Error(`REPRODUCED workspace load blocker :: ${loadErrorText}`);
      }
      return { itemId };
    });

    if (!loadErrorText) {
      await recordStep(driver, "publish-via-workspace-button", async () => {
        await driver.findElement(By.css('[data-testid="workspace-publish"]')).click();
        const notice = await driver.wait(until.elementLocated(By.css(".workspace-notice")), 60000);
        await driver.wait(async () => {
          const text = await notice.getText();
          return text.includes("发布完成") || text.includes("失败") || text.includes("未完成") || GATE_BLOCK_PATTERN.test(text);
        }, 60000);
        const noticeText = (await notice.getText()).replace(/\s+/g, " ").trim();

        if (noticeText.includes("发布完成")) {
          // 产品把 destination 当题库根，产物落在其 reading-exams 子树；
          // 递归枚举而不是只看一层。
          const files = [];
          const walk = (dir) => {
            if (!fs.existsSync(dir)) return;
            for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
              const full = path.join(dir, entry.name);
              if (entry.isDirectory()) walk(full);
              else files.push(path.relative(session.publishDir, full));
            }
          };
          walk(session.publishDir);
          if (!files.length) throw new Error(`发布显示成功但导出目录为空：${session.publishDir}`);
          return { outcome: "published", notice: noticeText, publishedFiles: files.slice(0, 20), countedAsPass: true };
        }
        if (GATE_BLOCK_PATTERN.test(noticeText)) {
          return {
            outcome: "blocked_by_quality_gate",
            notice: noticeText,
            countedAsPass: false,
            note: "产品质量门阻止了发布；发布链未被端到端验证，因此本脚本不判定为 passed。"
          };
        }
        // 既没成功、也不是门禁阻止——按真实失败处理，不掩盖。
        throw new Error(`发布既未成功也未被门禁阻止：${noticeText.slice(0, 300)}`);
      });
    }

    const failed = steps.filter((step) => step.status === "failed");
    const publishStep = steps.find((step) => step.name === "publish-via-workspace-button");
    const publishBlocked = publishStep?.status === "blocked";
    const verdict = failed.length ? "failed" : publishBlocked ? "blocked" : "passed";
    const report = {
      runId: path.basename(session.runDir),
      scenario: "publish",
      coverage: "real-tauri-process+webview2+sqlite+filesystem",
      evidenceLevel: "product",
      exe: exePath,
      ...freshness,
      pdf: pdfPath,
      publishDir: session.publishDir,
      workspaceLoadBlocker: loadErrorText,
      publishBlockedByQualityGate: publishBlocked,
      steps,
      verdict,
      driverStderr: session.driverStderr().slice(0, 4000)
    };
    writeReport(session.runDir, report);
    if (verdict === "blocked") {
      console.log("[e2e:tauri] 注意：发布被产品门禁阻止，本次运行不计为通过（verdict=blocked）。");
    }
    process.exitCode = exitCodeForVerdict(verdict);
  } catch (error) {
    logHarnessError(error);
    fs.writeFileSync(path.join(session.runDir, "harness-error.json"), JSON.stringify({
      error: String(error),
      driverStderr: session.driverStderr().slice(0, 8000)
    }, null, 2));
    process.exitCode = 2;
  } finally {
    await session.cleanup();
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
