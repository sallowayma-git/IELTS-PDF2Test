#!/usr/bin/env node
// 真实 Tauri 产品回归：发布成功路径（G0-T03 的发布缺口收敛）。
//
// 范围声明（重要）：
//   - 仓库现有语料 PDF 经自动识别达不到 ready 质量门（product_chain.rs 头注释），
//     因此本套件不用自动识别准备数据。
//   - 数据准备：启动前用仓库内「proven-ready authoring fixture + 派生物理阴影」
//     预置一个 job 目录（与 product_chain_ready_authoring_exports_and_publishes_to_nas
//     同一构造），应用启动迁移自动 seed canonical。
//   - 发布本身走真实产品路径：真实 UI 题库行 → 打开工作区 → 点击「发布」→
//     真实 publish_items 命令 → 质量门重算 → NAS 包落盘。
//   - 因此本套件只证明「发布链（UI→命令→质量门→NAS 包）」，不证明
//     「自动识别完整链能把真实 PDF 带到 ready」——后者保持 D1 blocked。
//
// 证据层级：product（真实进程 + WebView2 + SQLite + 文件系统）。

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import crypto from "node:crypto";
import { By, until } from "selenium-webdriver";
import {
  DEFAULT_EXE, DEFAULT_PDF, repoRoot, CannotRunError, assertPrerequisites, assertFreshBuild,
  buildFreshness, buildIdentity, launchTauriApp, createStepRecorder, openWorkspaceForItem,
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
const READY_AUTHORING = path.join(repoRoot, "fixtures", "golden", "synthetic", "ielts", "early-approaches-authoring-v2.json");

/** 复现 product_chain.rs::physical_shadow_for（导出会重算质量，错了会被门禁拦截——
 * 这正是本套件的验证点之一，无需信任复制本身）。 */
function physicalShadowFor(authoring, jobId) {
  const nodeIds = new Set();
  const walk = (value) => {
    if (Array.isArray(value)) {
      for (const item of value) walk(item);
      return;
    }
    if (value && typeof value === "object") {
      if (Array.isArray(value.sourceAnchors)) {
        for (const anchor of value.sourceAnchors) {
          for (const id of anchor?.nodeIds ?? []) nodeIds.add(id);
        }
      }
      for (const child of Object.values(value)) walk(child);
    }
  };
  walk(authoring);
  const sourceHash = "a".repeat(64);
  const sourceFileId = authoring?.exam?.sourceFiles?.[0]?.sourceFileId ?? "source-pdf-1";
  return {
    schemaVersion: "DocumentIRV2",
    documentId: authoring.sourceDocumentId ?? "document-1",
    jobId,
    sourceFiles: [{
      sourceFileId,
      originalName: "early-approaches.pdf",
      mediaType: "application/pdf",
      sha256: sourceHash,
      byteLength: 1,
      role: "question_paper"
    }],
    pages: [{
      pageIndex: 0,
      widthPt: 612.0,
      heightPt: 792.0,
      rotation: 0,
      glyphs: [],
      spans: [],
      lines: [],
      regions: [{
        id: "region-question-surface",
        kind: "text",
        bbox: { x: 10.0, y: 10.0, width: 500.0, height: 200.0, unit: "pt", origin: "top-left", pageRotation: 0 },
        childLineIds: [...nodeIds].sort(),
        childObjectIds: [],
        confidence: 1.0,
        sourceAnchors: [{
          sourceFileId,
          pageIndex: 0,
          nodeIds: ["region-question-surface"],
          extractionMode: "pdf_native",
          sourceHash
        }]
      }],
      vectorPaths: [],
      tables: [],
      assetIds: [],
      readingOrder: ["region-question-surface"],
      quality: {
        classification: "born_digital",
        nativeCharacterCount: 100,
        unicodeErrorRatio: 0.0,
        duplicateTextRatio: 0.0,
        imageCoverageRatio: 0.0,
        textCoverageRatio: 1.0,
        rotationConfidence: 1.0,
        requiresOcrRegions: []
      }
    }],
    assets: [],
    extraction: {
      engine: "e2e-publish-ready-fixture",
      engineVersion: "1.0.0",
      extractedAt: "2026-01-01T00:00:00Z",
      warnings: []
    }
  };
}

/** 启动前预置 job 目录：product_chain 同款 ready 数据，应用迁移会自动 seed canonical。 */
function seedReadyJob(dataDir) {
  const jobId = `e2e-publish-ready-${crypto.randomBytes(4).toString("hex")}`;
  const jobDir = path.join(dataDir, "jobs", jobId);
  fs.mkdirSync(jobDir, { recursive: true });
  const authoring = JSON.parse(fs.readFileSync(READY_AUTHORING, "utf8"));
  authoring.jobId = jobId;
  const now = "2026-09-12T00:00:00Z";
  const job = {
    jobId,
    title: "E2E Publish Ready",
    status: "Working",
    category: "P1",
    frequency: "medium",
    tags: ["e2e-publish-ready"],
    sourceFiles: [{
      fileId: "file-e2e-publish-ready",
      originalName: "early-approaches.pdf",
      storedName: "e2e-publish-ready.pdf",
      fileType: "pdf",
      sha256: "a".repeat(64),
      sizeBytes: 1,
      role: "MainQuestion",
      importedAt: now
    }],
    activeLlmProfileId: null,
    createdAt: now,
    updatedAt: now,
    currentStep: "Preview",
    issueCounts: { errors: 0, warnings: 0, needsReview: 0 }
  };
  fs.writeFileSync(path.join(jobDir, "job.json"), JSON.stringify(job, null, 2));
  fs.writeFileSync(path.join(jobDir, "authoring-ir-v2.shadow.json"), JSON.stringify(authoring, null, 2));
  fs.writeFileSync(path.join(jobDir, "document-ir-v2.shadow.json"), JSON.stringify(physicalShadowFor(authoring, jobId), null, 2));
  return { jobId, title: job.title };
}

async function main() {
  assertPrerequisites({ exePath, pdfPath });
  const freshness = buildFreshness(exePath);
  assertFreshBuild(freshness);
  const identity = buildIdentity(exePath);

  const session = await launchTauriApp({ exePath, pdfPath, keep: keepRun, runPrefix: "publish-ready" });
  const seeded = seedReadyJob(session.dataDir);
  console.log(`[e2e:tauri] seeded ready job ${seeded.jobId}（重启迁移会在启动时 seed canonical）`);
  // 预置发生在首次启动之后，会错过启动迁移窗口：保留数据目录，重启让产品的
  // 启动迁移（migrate_existing_into_library）自己 seed canonical。
  await session.cleanup({ removeRunDir: false });
  const relaunch = await launchTauriApp({
    exePath, pdfPath, keep: keepRun, runPrefix: "publish-ready", runDirOverride: session.runDir
  });

  const artifacts = { dir: relaunch.runDir, screenshotErrors: [] };
  const { steps, recordStep } = createStepRecorder({ artifacts, takeScreenshots });
  const driver = relaunch.driver;

  try {
    await recordStep(driver, "library-page-loads", async () => {
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      await driver.executeScript(`
        window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false, nasDestination: ${JSON.stringify(relaunch.publishDir)} }));
        location.hash = "#/library";
      `);
      await driver.navigate().refresh();
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      return { url: await driver.getCurrentUrl() };
    });

    await recordStep(driver, "ready-item-visible-in-library", async () => {
      const row = await driver.wait(
        until.elementLocated(By.css(`[data-item-id="${seeded.jobId}"]`)),
        30000
      );
      const text = (await row.getText()).replace(/\s+/g, " ").trim();
      return { jobId: seeded.jobId, rowText: text };
    });

    await recordStep(driver, "workspace-opens-for-ready-item", async () => {
      await openWorkspaceForItem(driver, seeded.jobId);
      const loadErrors = await driver.findElements(By.css(".workspace-load-error"));
      if (loadErrors.length) {
        throw new Error(`工作区加载失败：${(await loadErrors[0].getText()).slice(0, 300)}`);
      }
      return { itemId: seeded.jobId };
    });

    await recordStep(driver, "publish-via-workspace-button", async () => {
      await driver.findElement(By.css('[data-testid="workspace-publish"]')).click();
      const notice = await driver.wait(until.elementLocated(By.css(".workspace-notice")), 60000);
      await driver.wait(async () => {
        const text = await notice.getText();
        return /发布完成|失败|未完成|补齐|还没有|请先|待确认|需要确认/.test(text);
      }, 120000);
      const noticeText = (await notice.getText()).replace(/\s+/g, " ").trim();
      if (!noticeText.includes("发布完成")) {
        throw new Error(`发布未成功（质量门或其他产品门拦截）：${noticeText}`);
      }
      // 发布产物递归枚举：manifest / releases / exams 应出现在目标题库根。
      const files = [];
      const walk = (dir) => {
        if (!fs.existsSync(dir)) return;
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
          const full = path.join(dir, entry.name);
          if (entry.isDirectory()) walk(full);
          else files.push(path.relative(relaunch.publishDir, full));
        }
      };
      walk(relaunch.publishDir);
      if (!files.length) throw new Error(`发布显示成功但导出目录为空：${relaunch.publishDir}`);
      const manifest = files.find((file) => /manifest\.(js|json)$/i.test(file));
      if (!manifest) throw new Error(`发布产物中未见 manifest：${files.slice(0, 20).join(", ")}`);
      return { outcome: "published", notice: noticeText, publishedFiles: files.slice(0, 30), manifest };
    });

    const failed = steps.filter((step) => step.status === "failed");
    const report = {
      runId: path.basename(relaunch.runDir),
      scenario: "publish-ready-path",
      coverage: "real-tauri-process+webview2+sqlite+filesystem",
      evidenceLevel: "product",
      scope: "发布链（UI→publish_items→质量门重算→NAS 包）；不验证自动识别链达到 ready（D1 保持 blocked）",
      seededJobId: seeded.jobId,
      exe: exePath,
      ...identity,
      ...freshness,
      pdf: pdfPath,
      steps,
      verdict: failed.length ? "failed" : "passed",
      driverStderr: relaunch.driverStderr().slice(0, 4000)
    };
    writeReport(relaunch.runDir, report);
    process.exitCode = exitCodeForVerdict(report.verdict);
  } catch (error) {
    logHarnessError(error);
    fs.writeFileSync(path.join(relaunch.runDir, "harness-error.json"), JSON.stringify({
      error: String(error),
      driverStderr: relaunch.driverStderr().slice(0, 8000)
    }, null, 2));
    process.exitCode = 2;
  } finally {
    await relaunch.cleanup();
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
