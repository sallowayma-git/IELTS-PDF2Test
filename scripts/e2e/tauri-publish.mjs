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
  DEFAULT_EXE, DEFAULT_PDF, By, CannotRunError, assertPrerequisites, buildFreshness, assertFreshBuild, buildIdentity,
  createStepRecorder, exitCodeForVerdict, importPdfViaFolderHook, isCleanPublishOutcome, launchTauriApp,
  logCannotRun, logHarnessError, openWorkspaceForItem, parseArgs, publishAndReadOutcome, until, waitForRowStage, writeReport
} from "./lib/tauri-harness.mjs";

const args = parseArgs(process.argv.slice(2));
const exePath = path.resolve(args.exe ?? DEFAULT_EXE);
const pdfPath = path.resolve(args.pdf ?? DEFAULT_PDF);
const keepRun = Boolean(args.keep);
const takeScreenshots = args.screenshot !== false;

async function main() {
  assertPrerequisites({ exePath, pdfPath });
  const freshness = buildFreshness(exePath);
  assertFreshBuild(freshness);
  const identity = buildIdentity(exePath);

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
        // 判据是**机器可读**的发布结论（`.workspace-notice[data-publish-outcome]`），
        // 不是提示文案：放行发布与干净发布显示的是同一句「已发布」，匹配文案既认不出
        // 干净发布、也认不出学生端打不开的那一条。只有 `published` 算干净通过。
        const published = await publishAndReadOutcome(driver);
        const noticeText = published.text ?? published.noticeBefore ?? "";
        if (published.timedOut) {
          throw new Error(`点发布后 ${120000}ms 内没有出现发布结论（提示=${JSON.stringify(noticeText)}）`);
        }
        if (isCleanPublishOutcome(published.kind)) {
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
          return { outcome: "published", publishOutcome: published.kind, notice: noticeText, publishedFiles: files.slice(0, 20), countedAsPass: true };
        }
        if (published.kind === "published_forced" || published.kind === "published_forced_not_loadable") {
          // 产品上这是「已发布」，但对这条验收链不算通过：要证明的是**干净**发布。
          // 与既有的门禁阻止口径一致（verdict=blocked、退出码 4），不写成 passed。
          return {
            outcome: "blocked_by_quality_gate",
            publishOutcome: published.kind,
            notice: noticeText,
            countedAsPass: false,
            note: "发布被用户放行（或学生端暂时打不开这道题）：发布链未被端到端干净验证，因此本脚本不判定为 passed。"
          };
        }
        // `failed`：既没成功、也不是放行——按真实失败处理，不掩盖。
        throw new Error(`发布失败：${noticeText.slice(0, 300)}`);
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
      ...identity,
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
