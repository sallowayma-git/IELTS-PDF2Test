#!/usr/bin/env node
// G2 direct canonical 真实产品 E2E。
// PDF 与 DOCX 均走真实 Tauri/WebView2/SQLite/文件系统产品路径，feature flags 显式开启。
// direct builder 发生 V1 fallback 时本脚本必须失败；质量门阻止发布则如实报告 blocked。

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { Key } from "selenium-webdriver";
import {
  DEFAULT_EXE, By, CannotRunError, assertFreshBuild, assertPrerequisites, buildFreshness,
  buildIdentity, createStepRecorder, exitCodeForVerdict, importSourceViaFileHook,
  launchTauriApp, logCannotRun, logHarnessError, openWorkspaceForItem, parseArgs,
  repoRoot, sleep, until, waitForRowStage, writeReport
} from "./lib/tauri-harness.mjs";

const DIRECT_AUDIT_NOTE = "Direct canonical from QuestionLayoutGraphV1 (G2-T04); no V1 authoring input.";
const DEFAULT_PDF = path.join(repoRoot, "fixtures", "golden", "synthetic", "pdf", "pdf-two-column.pdf");
const DEFAULT_DOCX = path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-1.docx");
const args = parseArgs(process.argv.slice(2));
const exePath = path.resolve(args.exe ?? DEFAULT_EXE);
const cases = [
  { sourceType: "pdf", sourcePath: path.resolve(args.pdf ?? DEFAULT_PDF) },
  { sourceType: "docx", sourcePath: path.resolve(args.docx ?? DEFAULT_DOCX) }
];
const keepRun = Boolean(args.keep);
const takeScreenshots = args.screenshot !== false;

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function walkFiles(root) {
  const files = [];
  const visit = (dir) => {
    if (!fs.existsSync(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) visit(full);
      else files.push(path.relative(root, full));
    }
  };
  visit(root);
  return files;
}

async function configureLocalOnly(driver) {
  await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
  await driver.executeScript(`
    window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false }));
    location.hash = "#/library";
  `);
  await driver.navigate().refresh();
  await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
}

async function editAndPersist(driver, itemId, marker) {
  let editor = null;
  for (let attempt = 0; attempt < 3 && !editor; attempt += 1) {
    const text = await driver.wait(until.elementLocated(By.css(".v2-passage-pane .v2-text")), 15000);
    await driver.executeScript("arguments[0].click()", text);
    try {
      editor = await driver.wait(until.elementLocated(By.css('textarea[aria-label="编辑题目文字"]')), 5000);
    } catch {
      editor = null;
    }
  }
  if (!editor) throw new Error("点击 passage 文本未进入原位编辑器");
  await editor.sendKeys(Key.chord(Key.CONTROL, "a"));
  await editor.sendKeys(marker);
  try { await editor.sendKeys(Key.ENTER); } catch {}
  const saveState = await driver.wait(
    until.elementTextContains(
      driver.wait(until.elementLocated(By.css('[data-testid="workspace-save-state"]')), 10000),
      "已保存"
    ),
    20000
  );
  await driver.executeScript("location.hash = '#/library';");
  await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 15000);
  await openWorkspaceForItem(driver, itemId);
  await driver.wait(until.elementLocated(By.css(".v2-passage-pane .v2-text")), 30000);
  const passage = await driver.executeScript(
    "return document.querySelector('.v2-passage-pane')?.innerText ?? '';"
  );
  if (!String(passage).includes(marker)) throw new Error(`重开后未找到编辑标记 ${marker}`);
  return { marker, saveState: await saveState.getText(), persistedAfterReopen: true };
}

async function verifyIssueTarget(driver) {
  await driver.findElement(By.css('[data-testid="workspace-issues"]')).click();
  await driver.wait(until.elementLocated(By.css('[data-testid="workspace-issue-list"]')), 10000);
  // 问题列表现在渲染的是**任务卡**（每条任务带一个或多个真实动作按钮），不再是逐条原始问题行。
  // 因此这里按动作找按钮：只挑「去填写」（`data-action-id="fill-answer"`）——
  // 它的目标必然是题面上的答案控件，可定位；「查看原文」会顺带打开原文件抽屉，不适合当定位样本。
  const buttons = await driver.findElements(By.css('[data-testid="workspace-issue-list"] button[data-action-id="fill-answer"]'));
  if (!buttons.length) throw new Error("当前 direct canonical 题稿没有可用于验证定位的问题项");
  // 一条任务可能覆盖一个题号区间，取其中**目标确实在题面上**的那一个。
  let button = null;
  let targetId = null;
  for (const candidate of buttons) {
    const id = await candidate.getAttribute("data-action-target");
    const exists = await driver.executeScript(`
      const id = arguments[0];
      return Array.from(document.querySelectorAll('[data-editor-id], [data-question-id], [data-response-group-id]'))
        .some((node) => [node.dataset.editorId, node.dataset.questionId, node.dataset.responseGroupId].includes(id));
    `, id);
    if (exists) { button = candidate; targetId = id; break; }
  }
  if (!button) throw new Error("问题列表里没有可定位到题面的「去填写」动作");
  const targetExists = true;
  await driver.executeScript(`
    window.__pdf2testIssueScrolled = null;
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (...args) {
      window.__pdf2testIssueScrolled = this.dataset.editorId || this.dataset.questionId || this.dataset.responseGroupId || null;
      if (original) return original.apply(this, args);
    };
  `);
  await button.click();
  await driver.wait(async () => (
    await driver.executeScript("return window.__pdf2testIssueScrolled;")
  ) === targetId, 10000);
  return { issueCount: buttons.length, targetId, targetExists, scrollResolved: true };
}

async function publishOrBlock(driver, publishDir) {
  await driver.findElement(By.css('[data-testid="workspace-publish"]')).click();
  let noticeText = "";
  await driver.wait(async () => {
    const notices = await driver.findElements(By.css(".workspace-notice"));
    noticeText = notices.length ? (await notices[notices.length - 1].getText()).replace(/\s+/g, " ").trim() : "";
    return /发布完成|失败|未完成|补齐|还没有|请先|待确认|需要确认/.test(noticeText);
  }, 60000);
  if (!noticeText.includes("发布完成")) {
    return { outcome: "blocked_by_quality_gate", notice: noticeText, countedAsPass: false };
  }
  const publishedFiles = walkFiles(publishDir);
  if (!publishedFiles.length) throw new Error(`发布显示成功但 NAS 目录为空：${publishDir}`);
  return { outcome: "published", notice: noticeText, publishedFiles: publishedFiles.slice(0, 30) };
}

async function runCase(testCase, identity, freshness) {
  const session = await launchTauriApp({
    exePath,
    pdfPath: testCase.sourcePath,
    keep: keepRun,
    runPrefix: `direct-canonical-${testCase.sourceType}`,
    appEnv: {
      QLG_DIRECT_CANONICAL: "1",
      LOCAL_RECOGNITION_BLOCKERS_GATE: "1",
      PDF2TEST_AUTOMATION_SOURCE_FILES: testCase.sourcePath
    }
  });
  const artifacts = { dir: session.runDir, screenshotErrors: [] };
  const { steps, recordStep } = createStepRecorder({ artifacts, takeScreenshots });
  const driver = session.driver;
  let itemId = null;
  let directEvidence = null;

  try {
    await recordStep(driver, "library-page-loads", () => configureLocalOnly(driver));
    await recordStep(driver, `import-${testCase.sourceType}-via-file-ui`, async () => {
      const imported = await importSourceViaFileHook(driver);
      itemId = imported.itemId;
      return { ...imported, originalName: path.basename(testCase.sourcePath) };
    });
    await recordStep(driver, "background-pipeline-reaches-stable-stage", async () => {
      if (!itemId) throw new Error("缺少导入 item id");
      return waitForRowStage(driver, itemId, 300000);
    });
    await recordStep(driver, "direct-canonical-artifact-proven", async () => {
      if (!itemId) throw new Error("缺少导入 item id");
      const jobDir = path.join(session.dataDir, "jobs", itemId);
      const authoringPath = path.join(jobDir, "authoring-ir-v2.shadow.json");
      const graphPath = path.join(jobDir, "question-layout-graph.json");
      if (!fs.existsSync(authoringPath)) throw new Error(`缺少 canonical shadow：${authoringPath}`);
      if (!fs.existsSync(graphPath)) throw new Error(`缺少 QLG：${graphPath}`);
      const authoring = readJson(authoringPath);
      const graph = readJson(graphPath);
      const notes = Array.isArray(authoring.audit?.notes) ? authoring.audit.notes : [];
      if (!notes.includes(DIRECT_AUDIT_NOTE)) {
        throw new Error(`检测到 direct canonical 未生效或已回退 V1；audit.notes=${JSON.stringify(notes)}`);
      }
      directEvidence = {
        auditNote: DIRECT_AUDIT_NOTE,
        taskGroupCount: authoring.taskGroups?.length ?? 0,
        answerSlotCount: Object.keys(authoring.answerSlots ?? {}).length,
        recognitionBlockers: authoring.recognitionBlockers ?? [],
        qualityState: authoring.quality?.state,
        graphQuestionBlockCount: graph.questionBlocks?.length ?? 0,
        graphVisualCount: graph.visualStimuli?.length ?? 0
      };
      return directEvidence;
    });
    await recordStep(driver, "workspace-opens-and-renders-canonical", async () => {
      if (!itemId) throw new Error("缺少导入 item id");
      await openWorkspaceForItem(driver, itemId);
      const loadErrors = await driver.findElements(By.css(".workspace-load-error"));
      if (loadErrors.length) throw new Error(`工作区加载失败：${await loadErrors[0].getText()}`);
      await driver.wait(until.elementLocated(By.css('[data-testid="exam-canvas-v2-author"]')), 30000);
      return { itemId, canonicalCanvasRendered: true };
    });
    await recordStep(driver, "issue-click-resolves-to-canvas-target", () => verifyIssueTarget(driver));
    await recordStep(driver, "source-drawer-shows-original-file", async () => {
      await driver.findElement(By.css('[data-testid="workspace-source"]')).click();
      const drawer = await driver.wait(until.elementLocated(By.css('[role="dialog"][aria-label="原文件"]')), 10000);
      const text = (await drawer.getText()).replace(/\s+/g, " ").trim();
      const expected = path.basename(testCase.sourcePath);
      if (!text.includes(expected)) throw new Error(`原文件抽屉未显示 ${expected}；实际=${text.slice(0, 300)}`);
      await drawer.findElement(By.css('button[aria-label="关闭"]')).click();
      return { originalName: expected };
    });
    await recordStep(driver, "canonical-edit-saves-and-survives-reopen", () =>
      editAndPersist(driver, itemId, `DIRECT ${testCase.sourceType.toUpperCase()} EDIT 42`)
    );
    await recordStep(driver, "publish-via-real-workspace-gate", () => publishOrBlock(driver, session.publishDir));

    const failed = steps.some((step) => step.status === "failed");
    const blocked = steps.some((step) => step.status === "blocked");
    const verdict = failed ? "failed" : blocked ? "blocked" : "passed";
    const report = {
      runId: path.basename(session.runDir),
      scenario: "direct-canonical-product-chain",
      coverage: "real-tauri-process+webview2+sqlite+filesystem",
      evidenceLevel: "product",
      featureFlags: { QLG_DIRECT_CANONICAL: true, LOCAL_RECOGNITION_BLOCKERS_GATE: true },
      sourceType: testCase.sourceType,
      sourcePath: testCase.sourcePath,
      sourceSha256: sha256(testCase.sourcePath),
      itemId,
      directEvidence,
      exe: exePath,
      ...identity,
      ...freshness,
      steps,
      verdict,
      driverStderr: session.driverStderr().slice(0, 6000)
    };
    const reportPath = writeReport(session.runDir, report);
    return { ...report, reportPath };
  } finally {
    await session.cleanup();
  }
}

async function main() {
  for (const testCase of cases) {
    assertPrerequisites({ exePath, pdfPath: testCase.sourcePath });
  }
  const freshness = buildFreshness(exePath);
  assertFreshBuild(freshness);
  const identity = buildIdentity(exePath);
  const reports = [];
  for (const testCase of cases) {
    reports.push(await runCase(testCase, identity, freshness));
  }
  const verdict = reports.some((report) => report.verdict === "failed")
    ? "failed"
    : reports.some((report) => report.verdict === "blocked") ? "blocked" : "passed";
  const summary = {
    generatedAt: new Date().toISOString(),
    scenario: "direct-canonical-pdf-docx",
    evidenceLevel: "product",
    ...identity,
    ...freshness,
    verdict,
    cases: reports.map((report) => ({
      sourceType: report.sourceType,
      verdict: report.verdict,
      itemId: report.itemId,
      directEvidence: report.directEvidence,
      reportPath: report.reportPath
    }))
  };
  const summaryPath = path.join(repoRoot, "artifacts", "e2e-tauri", `DIRECT-CANONICAL-${Date.now()}.json`);
  fs.mkdirSync(path.dirname(summaryPath), { recursive: true });
  fs.writeFileSync(summaryPath, JSON.stringify(summary, null, 2));
  console.log(`[e2e:tauri] direct canonical summary: ${summaryPath}`);
  process.exitCode = exitCodeForVerdict(verdict);
}

try {
  await main();
} catch (error) {
  if (error instanceof CannotRunError) {
    logCannotRun(error, "e2e:direct-canonical");
    process.exit(3);
  }
  logHarnessError(error, "e2e:direct-canonical");
  process.exit(2);
}
