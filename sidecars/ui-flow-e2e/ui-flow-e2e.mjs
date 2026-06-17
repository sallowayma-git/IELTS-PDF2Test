#!/usr/bin/env node
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const DEFAULT_BASE_URL = "http://127.0.0.1:1420";
const CLEAR_TEXT_PDF = path.join(root, "fixtures", "parser", "complex-reading.pdf");
const SCANNED_PDF = path.join(root, "fixtures", "parser", "no-text.pdf");
const SCANNED_MANUAL_TRANSCRIPTION = `READING PASSAGE 1
Manual transcription passage for a scanned PDF. The author has checked the visual output against the source file.

Questions 1-3
1 The scanned PDF required manual review.
2 The author checked the transcription.
3 The final output can be validated after verification.

Answers
1 TRUE
2 TRUE
3 TRUE`;

function arg(name, fallback = undefined) {
  const index = process.argv.indexOf(name);
  if (index >= 0 && process.argv[index + 1]) return process.argv[index + 1];
  return fallback;
}

function hasFlag(name) {
  return process.argv.includes(name);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function removeDirBestEffort(dir) {
  try {
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 150 });
  } catch {
    // Chrome can release profile files slightly after process termination; this is not a test failure.
  }
}

async function waitFor(description, fn, { timeoutMs = 15000, intervalMs = 150 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const result = await fn();
      if (result) return result;
    } catch (error) {
      lastError = error;
    }
    await sleep(intervalMs);
  }
  throw new Error(`${description} timed out${lastError ? `: ${lastError.message}` : ""}`);
}

async function fetchJson(url, options) {
  const response = await fetch(url, options);
  if (!response.ok) throw new Error(`${options?.method ?? "GET"} ${url} -> ${response.status}`);
  return response.json();
}

function httpGet(url) {
  return new Promise((resolve, reject) => {
    const req = http.get(url, (res) => {
      res.resume();
      res.on("end", () => resolve(res.statusCode ?? 0));
    });
    req.on("error", reject);
    req.setTimeout(1500, () => {
      req.destroy(new Error(`timeout:${url}`));
    });
  });
}

async function ensureVite(baseUrl, noStartServer) {
  try {
    const status = await httpGet(baseUrl);
    if (status >= 200 && status < 500) return { process: null, started: false };
  } catch {
    // Start below unless explicitly disabled.
  }
  if (noStartServer) throw new Error(`Vite dev server is not reachable at ${baseUrl}`);

  const proc = spawn(npmCommand(), ["run", "dev", "--", "--host", "127.0.0.1"], {
    cwd: root,
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, BROWSER: "none" }
  });
  proc.stdout.on("data", (chunk) => process.stdout.write(`[vite] ${chunk}`));
  proc.stderr.on("data", (chunk) => process.stderr.write(`[vite] ${chunk}`));
  await waitFor("Vite dev server", async () => {
    try {
      const status = await httpGet(baseUrl);
      return status >= 200 && status < 500;
    } catch {
      return false;
    }
  }, { timeoutMs: 30000 });
  return { process: proc, started: true };
}

function npmCommand() {
  return process.platform === "win32" ? "npm.cmd" : "npm";
}

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : undefined;
      server.close(() => port ? resolve(port) : reject(new Error("failed_to_allocate_port")));
    });
    server.on("error", reject);
  });
}

function findChrome(chromePathArg) {
  const programFiles = [
    process.env.PROGRAMFILES,
    process.env["PROGRAMFILES(X86)"],
    process.env.LOCALAPPDATA
  ].filter(Boolean);
  const windowsCandidates = programFiles.flatMap((baseDir) => [
    path.join(baseDir, "Google", "Chrome", "Application", "chrome.exe"),
    path.join(baseDir, "Chromium", "Application", "chrome.exe"),
    path.join(baseDir, "Microsoft", "Edge", "Application", "msedge.exe")
  ]);
  const candidates = [
    chromePathArg,
    process.env.CHROME_PATH,
    process.env.EDGE_PATH,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    ...windowsCandidates
  ].filter(Boolean);
  const found = candidates.find((candidate) => fs.existsSync(candidate));
  if (!found) throw new Error("No Chrome/Chromium executable found. Set CHROME_PATH or pass --chrome-path.");
  return found;
}

async function launchChrome({ headful = false, chromePath, verbose = false }) {
  const port = await freePort();
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "epic8-ui-e2e-chrome-"));
  const args = [
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${userDataDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-extensions",
    "--disable-background-networking",
    "--disable-sync",
    "--disable-gpu",
    "--window-size=1440,1000",
    headful ? "" : "--headless=new",
    "about:blank"
  ].filter(Boolean);
  const proc = spawn(findChrome(chromePath), args, { stdio: ["ignore", "pipe", "pipe"] });
  proc.stderr.on("data", (chunk) => {
    const text = chunk.toString();
    if (verbose && !text.includes("DevTools listening")) process.stderr.write(`[chrome] ${text}`);
  });
  await waitFor("Chrome DevTools", async () => {
    try {
      return await fetchJson(`http://127.0.0.1:${port}/json/version`);
    } catch {
      return false;
    }
  }, { timeoutMs: 20000 });
  return { process: proc, port, userDataDir };
}

class Cdp {
  constructor(wsUrl) {
    this.ws = new WebSocket(wsUrl);
    this.nextId = 1;
    this.pending = new Map();
    this.events = [];
    this.opened = new Promise((resolve, reject) => {
      this.ws.addEventListener("open", resolve, { once: true });
      this.ws.addEventListener("error", (event) => reject(event.error ?? new Error("websocket_error")), { once: true });
    });
    this.ws.addEventListener("message", (event) => {
      const message = JSON.parse(event.data.toString());
      if (message.id && this.pending.has(message.id)) {
        const { resolve, reject } = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (message.error) reject(new Error(message.error.message));
        else resolve(message.result);
        return;
      }
      this.events.push(message);
    });
  }

  async ready() {
    await this.opened;
  }

  async send(method, params = {}) {
    await this.ready();
    const id = this.nextId++;
    const payload = JSON.stringify({ id, method, params });
    const result = new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
    this.ws.send(payload);
    return result;
  }

  close() {
    this.ws.close();
  }
}

async function createPage(port) {
  let target;
  try {
    target = await fetchJson(`http://127.0.0.1:${port}/json/new?${encodeURIComponent("about:blank")}`, { method: "PUT" });
  } catch {
    target = await fetchJson(`http://127.0.0.1:${port}/json/new?${encodeURIComponent("about:blank")}`);
  }
  const cdp = new Cdp(target.webSocketDebuggerUrl);
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  return { cdp, targetId: target.id };
}

async function evaluate(cdp, expression, { awaitPromise = true } = {}) {
  const result = await cdp.send("Runtime.evaluate", {
    expression,
    awaitPromise,
    returnByValue: true,
    userGesture: true
  });
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.text || result.exceptionDetails.exception?.description || "runtime_exception");
  }
  return result.result.value;
}

function jsString(value) {
  return JSON.stringify(value);
}

async function navigate(cdp, url) {
  await cdp.send("Page.navigate", { url });
  await waitFor(`page load ${url}`, async () => evaluate(cdp, "document.readyState === 'complete'"));
}

async function waitSelector(cdp, selector, timeoutMs = 15000) {
  await waitFor(`selector ${selector}`, async () => evaluate(cdp, `Boolean(document.querySelector(${jsString(selector)}))`), { timeoutMs });
}

async function click(cdp, selector) {
  await waitSelector(cdp, selector);
  const ok = await evaluate(cdp, `(() => {
    const element = document.querySelector(${jsString(selector)});
    if (!element) return false;
    element.click();
    return true;
  })()`);
  if (!ok) throw new Error(`click_failed:${selector}`);
}

async function setValue(cdp, selector, value) {
  await waitSelector(cdp, selector);
  await evaluate(cdp, `(() => {
    const element = document.querySelector(${jsString(selector)});
    if (!element) return false;
    const previous = element.value;
    element.value = ${jsString(value)};
    const tracker = element._valueTracker;
    if (tracker) tracker.setValue(previous);
    element.dispatchEvent(new Event("input", { bubbles: true }));
    element.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  })()`);
}

async function getText(cdp, selector) {
  await waitSelector(cdp, selector);
  return evaluate(cdp, `document.querySelector(${jsString(selector)})?.textContent ?? ""`);
}

async function currentHash(cdp) {
  return evaluate(cdp, "window.location.hash");
}

async function getStoreSummary(cdp, requestedJobId = undefined) {
  return evaluate(cdp, `(() => {
    const raw = localStorage.getItem("ielts-author-studio.dev-fallback-store.v1");
    if (!raw) return null;
    const store = JSON.parse(raw);
    const explicitJobId = ${jsString(requestedJobId ?? null)};
    const routeJobId = window.location.hash.match(/#\\/?jobs\\/([^/]+)/)?.[1];
    const jobId = explicitJobId || routeJobId;
    const job = jobId
      ? store.jobs?.find((item) => item.jobId === jobId)
      : store.jobs?.[0];
    if (!job) return null;
    return {
      job,
      documentIr: store.documents?.[job.jobId],
      split: store.splits?.[job.jobId],
      authoringIr: store.authoring?.[job.jobId],
      validationReport: store.validation?.[job.jobId],
      previewAssets: store.previews?.[job.jobId],
      pipelineReport: store.pipelineReports?.[job.jobId],
      sourceReview: store.sourceReviews?.[job.jobId]
    };
  })()`);
}

async function markAuthoringVerified(cdp, jobId) {
  return evaluate(cdp, `(() => {
    const raw = localStorage.getItem("ielts-author-studio.dev-fallback-store.v1");
    if (!raw) throw new Error("dev_store_missing");
    const store = JSON.parse(raw);
    const ir = store.authoring?.[${jsString(jobId)}];
    if (!ir) throw new Error("authoring_ir_missing");
    const groups = ir.groups.map((group) => ({
      ...group,
      verified: true,
      questions: group.questions.map((question) => ({
        ...question,
        verified: true,
        requiresManualQuestionImport: false,
        prompt: question.prompt?.startsWith("Manual import required") ? \`Verified prompt for question \${question.displayNumber}\` : question.prompt,
        answer: question.answer || "TRUE"
      })),
      requiresManualQuestionImport: false
    }));
    const answerKey = Object.fromEntries(groups.flatMap((group) => group.questions.map((question) => [question.id, question.answer || "TRUE"])));
    const questionOrder = groups.flatMap((group) => group.questions.map((question) => question.id));
    const questionDisplayMap = Object.fromEntries(groups.flatMap((group) => group.questions.map((question) => [question.id, question.displayNumber])));
    const nextIr = {
      ...ir,
      groups,
      answerKey,
      questionOrder,
      questionDisplayMap,
      audit: {
        ...ir.audit,
        humanVerified: true,
        updatedAt: new Date().toISOString(),
        revision: (ir.audit?.revision || 1) + 1
      }
    };
    store.authoring[${jsString(jobId)}] = nextIr;
    const index = store.jobs.findIndex((job) => job.jobId === ${jsString(jobId)});
    if (index >= 0) {
      store.jobs[index] = {
        ...store.jobs[index],
        status: "DraftSaved",
        currentStep: "Authoring",
        issueCounts: { errors: 0, warnings: 0, needsReview: 0 },
        updatedAt: new Date().toISOString()
      };
    }
    localStorage.setItem("ielts-author-studio.dev-fallback-store.v1", JSON.stringify(store));
    return { groupCount: groups.length, questionCount: questionOrder.length };
  })()`);
}

function assert(condition, message, details) {
  if (!condition) {
    const suffix = details ? `\n${JSON.stringify(details, null, 2)}` : "";
    throw new Error(`${message}${suffix}`);
  }
}

async function resetDevStore(cdp) {
  await evaluate(cdp, `localStorage.removeItem("ielts-author-studio.dev-fallback-store.v1");
localStorage.removeItem("ielts-author-studio.dev-fallback-picked-paths.v1");`);
}

async function completeReviewPreviewExportPack(cdp, baseUrl, jobId) {
  const verified = await markAuthoringVerified(cdp, jobId);
  assert(verified.questionCount >= 1, "manual verification helper should verify at least one question", verified);
  await navigate(cdp, `${baseUrl}/#/jobs/${jobId}/groups`);
  await waitSelector(cdp, "[data-testid='group-editor']");
  await click(cdp, "[data-testid='validate-and-export']");
  await waitFor("route to export", async () => (await currentHash(cdp)).includes("/export"), { timeoutMs: 10000 });
  await waitSelector(cdp, "[data-testid='export-page']");
  await waitFor("export files rendered", async () => {
    const count = await evaluate(cdp, `document.querySelectorAll("[data-testid='export-file']").length`);
    return count >= 2;
  }, { timeoutMs: 10000 });
  const exportedFileCount = await evaluate(cdp, `document.querySelectorAll("[data-testid='export-file']").length`);
  const afterExport = await getStoreSummary(cdp, jobId);
  assert(["Exported", "Cleaned"].includes(afterExport.job.status), "export should advance job to exported/cleaned state", afterExport.job);
  await navigate(cdp, `${baseUrl}/#/packs`);
  await waitSelector(cdp, "[data-testid='pack-builder']");
  await waitSelector(cdp, "[data-testid='pack-job-checkbox']");
  await click(cdp, "[data-testid='pack-job-checkbox']");
  await click(cdp, "[data-testid='build-pack']");
  await waitFor("pack result rendered", async () => {
    const text = await getText(cdp, "[data-testid='pack-result']");
    const store = await evaluate(cdp, `JSON.parse(localStorage.getItem("ielts-author-studio.dev-fallback-store.v1") || "{}")`);
    return text.includes("输出路径") && Boolean(store.packs?.[0]?.packId);
  }, { timeoutMs: 10000 });
  const packResultText = await getText(cdp, "[data-testid='pack-result']");
  const packStore = await evaluate(cdp, `JSON.parse(localStorage.getItem("ielts-author-studio.dev-fallback-store.v1") || "{}").packs?.[0] ?? null`);
  return {
    finalStatus: afterExport.job.status,
    runtimeMode: afterExport.validationReport?.runtime?.mode ?? "unknown",
    exportedFileCount,
    packBuilt: packResultText.includes("输出路径") && Boolean(packStore?.packId)
  };
}

async function runClearTextFlow(cdp, baseUrl) {
  const url = `${baseUrl}/?epic8DevPickedPath=${encodeURIComponent(CLEAR_TEXT_PDF)}#/jobs/new`;
  await navigate(cdp, url);
  await resetDevStore(cdp);
  await navigate(cdp, url);
  await click(cdp, "[data-testid='pick-source-file']");
  await setValue(cdp, "[data-testid='job-title-input']", "UI E2E Clear Text");
  await click(cdp, "[data-testid='create-and-auto-process']");
  await waitFor("clear text route", async () => {
    const hash = await currentHash(cdp);
    return hash.includes("/groups");
  }, { timeoutMs: 20000 });

  const summary = await getStoreSummary(cdp);
  assert(summary?.job, "clear text flow did not create a job");
  assert(summary.sourceReview?.required === false, "clear text flow should not require source review", summary.sourceReview);
  assert(summary.job.currentStep === "Authoring", "clear text flow should route directly to editable draft", summary.job);
  assert(summary.authoringIr?.groups?.length >= 2, "clear text flow should produce editable question groups", summary.authoringIr);
  assert(!summary.documentIr, "clear text minimized state should not persist DocumentIR after AuthoringIR convergence", summary.documentIr);
  assert(!summary.split, "clear text minimized state should not persist split candidates after AuthoringIR convergence", summary.split);
  assert(!summary.validationReport, "clear text minimized state should not persist validation report before preview regeneration", summary.validationReport);
  assert(!summary.pipelineReport, "clear text minimized state should not persist pipeline report", summary.pipelineReport);
  await waitSelector(cdp, "[data-testid='group-editor']");
  const completion = await completeReviewPreviewExportPack(cdp, baseUrl, summary.job.jobId);
  return {
    name: "clear-text-review-preview-export-pack",
    jobId: summary.job.jobId,
    initialStatus: summary.job.status,
    initialStep: summary.job.currentStep,
    finalStatus: completion.finalStatus,
    groupCount: summary.authoringIr.groups.length,
    runtimeMode: completion.runtimeMode,
    exportedFileCount: completion.exportedFileCount,
    packBuilt: completion.packBuilt
  };
}

async function runOcrSourceReviewFlow(cdp, baseUrl) {
  const url = `${baseUrl}/?epic8DevPickedPath=${encodeURIComponent(SCANNED_PDF)}#/jobs/new`;
  await navigate(cdp, url);
  await resetDevStore(cdp);
  await navigate(cdp, url);
  await click(cdp, "[data-testid='pick-source-file']");
  await setValue(cdp, "[data-testid='job-title-input']", "UI E2E Scanned PDF");
  await setValue(cdp, "[data-testid='parse-mode']", "ocr");
  await click(cdp, "[data-testid='create-and-auto-process']");
  await waitFor("ocr editable draft route", async () => {
    const hash = await currentHash(cdp);
    return hash.includes("/groups");
  }, { timeoutMs: 20000 });
  await waitSelector(cdp, "[data-testid='group-editor']");
  const summary = await getStoreSummary(cdp);

  assert(summary.sourceReview?.required === true, "ocr flow should keep required source review evidence", { sourceReview: summary.sourceReview });
  assert(summary.job.currentStep === "Authoring", "ocr flow should route directly to editable draft", summary.job);
  assert(summary.authoringIr?.groups?.length >= 1, "ocr flow should still produce an editable draft shell", summary.authoringIr);
  assert(!summary.documentIr, "ocr minimized state should not persist vision placeholder DocumentIR before manual transcription", summary.documentIr);
  assert(!summary.split, "ocr minimized state should not persist split candidates before manual transcription", summary.split);
  assert(!summary.pipelineReport, "ocr minimized state should not persist pipeline report", summary.pipelineReport);
  assert(summary.sourceReview?.required === true, "ocr flow should keep source review evidence", summary.sourceReview);
  assert(summary.sourceReview?.resolved === false, "ocr flow should remain unresolved before manual transcription", summary.sourceReview);
  assert(summary.job.status === "NeedsReview", "ocr flow must remain NeedsReview", summary.job);

  await navigate(cdp, `${baseUrl}/#/jobs/${summary.job.jobId}/document`);
  await waitSelector(cdp, "[data-testid='source-review-status']");
  const sourceReviewText = await getText(cdp, "[data-testid='source-review-status']");
  const sourceReviewJson = await getText(cdp, "[data-testid='source-review-json']");
  assert(
    sourceReviewJson.includes("解析提醒")
      || sourceReviewJson.includes("低置信内容")
      || summary.sourceReview?.parserWarnings?.length
      || summary.sourceReview?.lowConfidenceBlocks?.length,
    "DocumentReview should expose persisted SourceReview after minimization",
    { sourceReviewJson, sourceReview: summary.sourceReview }
  );
  await setValue(cdp, "[data-testid='manual-transcription-text']", SCANNED_MANUAL_TRANSCRIPTION);
  await click(cdp, "[data-testid='apply-manual-transcription']");
  await waitFor("manual transcription resolves source review", async () => {
    const next = await getStoreSummary(cdp, summary.job.jobId);
    return next?.sourceReview?.resolved === true;
  }, { timeoutMs: 10000 });
  const afterManual = await getStoreSummary(cdp, summary.job.jobId);
  assert(afterManual.documentIr?.parser?.provider === "manual-transcription", "manual transcription should replace vision placeholder DocumentIR", afterManual.documentIr?.parser);
  await click(cdp, "[data-testid='go-split']");
  await waitFor("route to split after manual transcription", async () => (await currentHash(cdp)).includes("/split"), { timeoutMs: 10000 });
  await click(cdp, "[data-testid='build-authoring-ir']");
  await waitFor("route to groups after build authoring", async () => (await currentHash(cdp)).includes("/groups"), { timeoutMs: 10000 });
  const afterBuild = await getStoreSummary(cdp, summary.job.jobId);
  assert(afterBuild.authoringIr?.groups?.length >= 1, "manual transcription flow should produce AuthoringIR groups", afterBuild.authoringIr);
  assert(!afterBuild.documentIr, "manual transcription flow should minimize DocumentIR after AuthoringIR is built", afterBuild.documentIr);
  assert(!afterBuild.split, "manual transcription flow should minimize split candidates after AuthoringIR is built", afterBuild.split);
  const completion = await completeReviewPreviewExportPack(cdp, baseUrl, afterBuild.job.jobId);
  return {
    name: "ocr-manual-transcription-review-preview-export-pack",
    jobId: summary.job.jobId,
    initialStatus: summary.job.status,
    initialStep: summary.job.currentStep,
    initialSourceReview: sourceReviewText.trim(),
    visionApplied: true,
    manualProvider: afterManual.documentIr.parser.provider,
    groupCount: afterBuild.authoringIr.groups.length,
    finalStatus: completion.finalStatus,
    runtimeMode: completion.runtimeMode,
    exportedFileCount: completion.exportedFileCount,
    packBuilt: completion.packBuilt
  };
}

async function main() {
  const baseUrl = arg("--base-url", DEFAULT_BASE_URL).replace(/\/$/, "");
  const noStartServer = hasFlag("--no-start-server");
  const headful = hasFlag("--headful");
  const keepOpen = hasFlag("--keep-open");
  const verbose = hasFlag("--verbose");
  const chromePath = arg("--chrome-path");
  const vite = await ensureVite(baseUrl, noStartServer);
  const chrome = await launchChrome({ headful, chromePath, verbose });
  const page = await createPage(chrome.port);
  const results = [];
  try {
    results.push(await runClearTextFlow(page.cdp, baseUrl));
    results.push(await runOcrSourceReviewFlow(page.cdp, baseUrl));
    const report = {
      schemaVersion: "Epic8UiFlowE2eReportV1",
      passed: true,
      baseUrl,
      generatedAt: new Date().toISOString(),
      results
    };
    console.log(JSON.stringify(report, null, 2));
  } finally {
    if (!keepOpen) page.cdp.close();
    if (!keepOpen) chrome.process.kill();
    if (!keepOpen) removeDirBestEffort(chrome.userDataDir);
    if (vite.started && vite.process) vite.process.kill();
  }
}

main().catch((error) => {
  console.error(JSON.stringify({
    schemaVersion: "Epic8UiFlowE2eReportV1",
    passed: false,
    error: error?.stack ?? String(error)
  }, null, 2));
  process.exit(1);
});
