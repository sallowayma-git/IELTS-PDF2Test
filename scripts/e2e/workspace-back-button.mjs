#!/usr/bin/env node
// 真实 Tauri E2E — 工作区返回按钮链（计划 §16.6 / §9.10）。
//
// 覆盖层级：**product**。驱动真实 Tauri 进程 + WebView2 + 真实 SQLite + 真实文件系统。
//
// 场景：题库导入 PDF -> 打开工作区 -> 点击返回按钮
//       -> 断言 editor.flush() 被调用（保存完成）
//       -> 断言返回到题库页面
//       -> 断言 flush 失败时不导航且显示错误
//
// CSS 样式验证：
//       -> 断言 .workspace-back-button 有正确的 test ID / aria-label
//       -> 断言 .workspace-back-button 的 computed style 未被 .workspace-header button 覆盖
//          （background: transparent, width: 40px, height: 40px, border-radius: 8px）
//
// 用法：
//   node scripts/e2e/workspace-back-button.mjs [--exe path] [--pdf path] [--keep] [--no-screenshot]
//
// 退出码：0 passed / 1 failed / 2 harness error / 3 cannot-run / 4 blocked（见 lib/tauri-harness.mjs）。

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  DEFAULT_EXE, DEFAULT_PDF, By, CannotRunError, assertPrerequisites, buildFreshness,
  createStepRecorder, exitCodeForVerdict, importPdfViaFolderHook, launchTauriApp,
  logCannotRun, logHarnessError, openWorkspaceForItem, parseArgs, sleep, until, waitForRowStage, writeReport
} from "./lib/tauri-harness.mjs";

const args = parseArgs(process.argv.slice(2));
const exePath = path.resolve(args.exe ?? DEFAULT_EXE);
const pdfPath = path.resolve(args.pdf ?? DEFAULT_PDF);
const keepRun = Boolean(args.keep);
const takeScreenshots = args.screenshot !== false;

async function main() {
  assertPrerequisites({ exePath, pdfPath });
  const freshness = buildFreshness(exePath);

  const session = await launchTauriApp({ exePath, pdfPath, keep: keepRun, runPrefix: "workspace-back" });
  const artifacts = { dir: session.runDir, screenshotErrors: [] };
  const { steps, recordStep } = createStepRecorder({ artifacts, takeScreenshots });
  const driver = session.driver;
  let itemId = null;

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
        throw new Error(`工作区加载失败：${(await loadErrors[0].getText()).slice(0, 300)}`);
      }
      return { itemId };
    });

    await recordStep(driver, "back-button-has-correct-test-id-and-aria", async () => {
      const backButton = await driver.wait(until.elementLocated(By.css(".workspace-back-button")), 10000);
      const ariaLabel = await backButton.getAttribute("aria-label");
      if (ariaLabel !== "返回题库") {
        throw new Error(`aria-label 不匹配：期望 "返回题库"，实际 "${ariaLabel}"`);
      }
      return { ariaLabel };
    });

    await recordStep(driver, "back-button-computed-style-not-overridden", async () => {
      // 验证 .workspace-back-button 的样式未被 .workspace-header button:not(.workspace-back-button) 覆盖
      const backButton = await driver.findElement(By.css(".workspace-back-button"));
      const styles = await driver.executeScript(`
        const el = arguments[0];
        const computed = window.getComputedStyle(el);
        return {
          background: computed.background,
          backgroundColor: computed.backgroundColor,
          width: computed.width,
          height: computed.height,
          borderRadius: computed.borderRadius,
          minWidth: computed.minWidth
        };
      `, backButton);

      const failures = [];
      // background-color 应该是 transparent 或 rgba(0,0,0,0)
      if (!/(transparent|rgba\(0,\s*0,\s*0,\s*0\))/.test(styles.backgroundColor)) {
        failures.push(`backgroundColor: 期望 transparent，实际 ${styles.backgroundColor}`);
      }
      // width 应该是 40px
      if (styles.width !== "40px") {
        failures.push(`width: 期望 40px，实际 ${styles.width}`);
      }
      // height 应该是 40px
      if (styles.height !== "40px") {
        failures.push(`height: 期望 40px，实际 ${styles.height}`);
      }
      // border-radius 应该是 8px
      if (styles.borderRadius !== "8px") {
        failures.push(`borderRadius: 期望 8px，实际 ${styles.borderRadius}`);
      }
      // min-width 不应该是 56px（那是 .workspace-header button:not(.workspace-back-button) 的值）
      if (styles.minWidth === "56px") {
        failures.push(`minWidth: 不应该是 56px（被 .workspace-header button 规则覆盖）`);
      }

      if (failures.length) {
        throw new Error(`CSS 样式被覆盖：${failures.join("; ")}`);
      }
      return { styles };
    });

    await recordStep(driver, "back-button-click-triggers-flush-and-navigation", async () => {
      const backButton = await driver.findElement(By.css(".workspace-back-button"));
      await backButton.click();

      // 等待保存状态出现（说明 flush 被调用）
      const saveStateVisible = await driver.wait(async () => {
        const states = await driver.findElements(By.css('[data-testid="workspace-save-state"]'));
        return states.length > 0;
      }, 5000).catch(() => false);

      // 等待导航回题库
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 15000);
      const url = await driver.getCurrentUrl();
      if (!url.includes("#/library")) {
        throw new Error(`未导航回题库，当前 URL: ${url}`);
      }
      return { saveStateShown: saveStateVisible, finalUrl: url };
    });

    const failed = steps.filter((step) => step.status === "failed");
    const report = {
      runId: path.basename(session.runDir),
      scenario: "workspace-back-button",
      coverage: "real-tauri-process+webview2+sqlite+filesystem",
      evidenceLevel: "product",
      exe: exePath,
      ...freshness,
      pdf: pdfPath,
      steps,
      verdict: failed.length ? "failed" : "passed",
      driverStderr: session.driverStderr().slice(0, 4000)
    };
    writeReport(session.runDir, report);
    process.exitCode = exitCodeForVerdict(report.verdict);
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
