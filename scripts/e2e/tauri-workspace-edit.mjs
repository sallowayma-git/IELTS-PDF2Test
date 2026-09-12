#!/usr/bin/env node
// 真实 Tauri E2E —— 工作区编辑链（计划 §18 P0-T02 / §19.7，audit A11-F02 缺失脚本之一）。
//
// 覆盖层级：**product**。驱动真实 Tauri 进程 + WebView2 + 真实 SQLite + 真实文件系统。
// 与 `library-workspace-smoke.mjs`（浏览器 + devFallback）和 Rust 命令级测试是不同层级的证据，
// 分别报告，不互相替代。
//
// 场景：题库导入 PDF -> 后台识别到稳定阶段 -> 打开工作区 -> 原位改一个字符并保存
//       -> 回题库再重开 -> 断言修改仍在（走完整读盘 + revision 解析链）。
//
// 用法：
//   node scripts/e2e/tauri-workspace-edit.mjs [--exe path] [--pdf path] [--keep] [--no-screenshot]
//   npm run e2e:tauri:workspace-edit
//
// 退出码：0 passed / 1 failed / 2 harness error / 3 cannot-run / 4 blocked（见 lib/tauri-harness.mjs）。

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { Key } from "selenium-webdriver";
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

  const session = await launchTauriApp({ exePath, pdfPath, keep: keepRun, runPrefix: "workspace-edit" });
  const artifacts = { dir: session.runDir, screenshotErrors: [] };
  const { steps, recordStep } = createStepRecorder({ artifacts, takeScreenshots });
  const driver = session.driver;
  const editedMarker = "E2E WORKSPACE EDIT 42";
  let itemId = null;
  let loadErrorText = null;

  try {
    await recordStep(driver, "library-page-loads", async () => {
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      // 云端识别保持关闭：本轮验证本地链（云端依赖外部模型配置）。
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
        const jobDir = path.join(session.dataDir, "jobs", itemId);
        const errorFiles = [];
        if (fs.existsSync(jobDir)) {
          for (const name of fs.readdirSync(jobDir)) {
            if (name.endsWith(".error.json")) {
              errorFiles.push({ file: name, content: fs.readFileSync(path.join(jobDir, name), "utf8").slice(0, 4000) });
            }
          }
        }
        throw new Error(`REPRODUCED workspace load blocker :: ${loadErrorText} :: jobDir=${jobDir} :: errorFiles=${JSON.stringify(errorFiles)}`);
      }
      return { itemId };
    });

    if (!loadErrorText) {
      await recordStep(driver, "edit-one-character-and-save", async () => {
        // 点击会触发 React 重渲染（选中态/编辑态），元素引用极易过期；每一步都重新定位。
        let editor = null;
        for (let attempt = 0; attempt < 3 && !editor; attempt += 1) {
          const span = await driver.wait(until.elementLocated(By.css(".v2-passage-pane .v2-text")), 15000);
          await driver.executeScript("arguments[0].click()", span);
          try {
            editor = await driver.wait(until.elementLocated(By.css('textarea[aria-label="编辑题目文字"]')), 5000);
          } catch {
            editor = null;
          }
        }
        if (!editor) throw new Error("点击 passage 文本未进入原位编辑器");
        await driver.wait(async () => {
          try {
            await editor.sendKeys(Key.chord(Key.CONTROL, "a"));
            return true;
          } catch {
            editor = await driver.wait(until.elementLocated(By.css('textarea[aria-label="编辑题目文字"]')), 5000);
            return false;
          }
        }, 15000);
        await editor.sendKeys(editedMarker);
        // Enter 提交后组件随即卸载，最后一次按键可能打到过期引用——提交本身已发生。
        try {
          await editor.sendKeys(Key.ENTER);
        } catch {}
        const saveState = await driver.wait(
          until.elementTextContains(
            driver.wait(until.elementLocated(By.css('[data-testid="workspace-save-state"]')), 10000),
            "已保存"
          ),
          20000
        );
        return { saveState: await saveState.getText(), marker: editedMarker };
      });

      await recordStep(driver, "edit-survives-reopen", async () => {
        await driver.executeScript("location.hash = '#/library';");
        await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 15000);
        await openWorkspaceForItem(driver, itemId);
        // 重开走完整加载链（读盘 + revision 解析），等 Canvas 真正渲染出文本再断言。
        await driver.wait(until.elementLocated(By.css(".v2-passage-pane .v2-text")), 30000);
        const loadErrors = await driver.findElements(By.css(".workspace-load-error"));
        if (loadErrors.length) {
          throw new Error(`重开后出现加载错误：${(await loadErrors[0].getText()).slice(0, 300)}`);
        }
        const passageText = await driver.executeScript(
          "return document.querySelector('.v2-passage-pane') ? document.querySelector('.v2-passage-pane').innerText : '';"
        );
        if (!String(passageText).includes(editedMarker)) {
          throw new Error(`重开后未找到编辑文本 "${editedMarker}"，实际开头：${String(passageText).slice(0, 200)}`);
        }
        return { marker: editedMarker, found: true };
      });
    }

    const failed = steps.filter((step) => step.status === "failed");
    const report = {
      runId: path.basename(session.runDir),
      scenario: "workspace-edit",
      coverage: "real-tauri-process+webview2+sqlite+filesystem",
      evidenceLevel: "product",
      exe: exePath,
      ...freshness,
      pdf: pdfPath,
      workspaceLoadBlocker: loadErrorText,
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
