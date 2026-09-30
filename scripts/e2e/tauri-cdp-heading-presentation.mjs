#!/usr/bin/env node
/**
 * Controlled cloud candidate → real heading anchors → real Tauri canvas drag → persistence.
 * The controlled model's candidate uses @paragraph:A placeholders. The service resolves them
 * from each actual candidate request, so a second import can never pass by reusing stale IDs.
 */
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { DatabaseSync } from "node:sqlite";
import {
  assertBuildFresh,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  sleep,
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const pdfArg = process.argv.indexOf("--pdf");
const portArg = process.argv.indexOf("--port");
const fixturePath = path.resolve(pdfArg >= 0 ? process.argv[pdfArg + 1] : path.join(repoRoot, "fixtures", "golden", "private-real", "western-celebrity.pdf"));
const servicePort = portArg >= 0 ? Number(process.argv[portArg + 1]) : 11456;
const serviceScript = path.join(repoRoot, "scripts", "controlled-llm-service.mjs");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `run-heading-presentation-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const appDataDir = path.join(runDir, "appdata", "data");
const scenarioDir = path.join(runDir, "scenario");
const nasDir = path.join(runDir, "nas-library");
const candidatePath = path.join(scenarioDir, "authoring-candidate.json");
const PROFILE_ID = "controlled-heading-presentation";
// Transcribed from the fixture's image-only page 5 after visual review of all five pages.
const verifiedSourceAnswers = {
  q14: { kind: "option", labels: ["x"] },
  q15: { kind: "option", labels: ["ii"] },
  q16: { kind: "option", labels: ["v"] },
  q17: { kind: "option", labels: ["vii"] },
  q18: { kind: "option", labels: ["iv"] },
  q19: { kind: "option", labels: ["viii"] },
  q20: { kind: "option", labels: ["iii"] },
  q21: { kind: "option", labels: ["D"] },
  q22: { kind: "option", labels: ["A"] },
  q23: { kind: "option", labels: ["C"] },
  q24: { kind: "text", values: ["newspapers"] },
  q25: { kind: "text", values: ["appearance"] },
  q26: { kind: "text", values: ["audience"] },
};
const report = {
  task: "cloud-heading-presentation-product-chain",
  evidenceLevel: "real Tauri import + controlled cloud service + CDP canvas drag + persisted workspace readback",
  fixturePath,
  fixtureSha256: fs.existsSync(fixturePath) ? sha256File(fixturePath) : null,
  runDir,
  assertions: [],
  verdict: "failed",
};
let session;
let service;

function assert(id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[heading-presentation] ${ok ? "PASS" : "FAIL"} ${id}: ${detail}`);
  if (!ok) throw new Error(`${id}: ${detail}`);
}

function startService(candidate = null) {
  const args = [serviceScript, "--port", String(servicePort), "--mode", "normal"];
  if (candidate) args.push("--candidate", candidate);
  service = spawn(process.execPath, args, { cwd: repoRoot, stdio: "ignore", windowsHide: true });
}

async function waitForService() {
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`http://127.0.0.1:${servicePort}/health`);
      if (response.ok) return response.json();
    } catch { /* service is starting */ }
    await sleep(250);
  }
  throw new Error("controlled LLM service did not become ready");
}

async function stopService() {
  if (!service) return;
  service.kill();
  service = null;
  await sleep(500);
}

async function invoke(command, args) {
  const result = await session.invoke(command, args);
  if (result?.__noInvoke) throw new Error(`Tauri command unavailable: ${command}`);
  if (result?.ok === false) throw new Error(`${command} failed: ${result.error}`);
  return result?.value ?? result;
}

async function workspace(itemId) {
  return invoke("get_workspace_item", { itemId });
}

function controlledExportAnswer(ds, group, response, slotId) {
  const interaction = ds.answerSlots?.[slotId]?.interaction ?? "text";
  const answer = verifiedSourceAnswers[slotId];
  if (!answer) return null;
  if (answer.kind === "text") {
    if (interaction !== "text") throw new Error(`source answer for ${slotId} is text but interaction is ${interaction}`);
    return { ...answer, normalization: "ielts_default" };
  }
  if (interaction === "text") throw new Error(`source answer for ${slotId} is option but interaction is text`);
  const options = group.optionBank?.options ?? response.options ?? [];
  const labels = options
    .map((option) => option?.label)
    .filter((label) => typeof label === "string" && label.trim().length > 0);
  if (!labels.some((label) => label.toLowerCase() === answer.labels[0].toLowerCase())) {
    throw new Error(`source answer ${answer.labels[0]} for ${slotId} is absent from the option list`);
  }
  return {
    kind: "option",
    labels: answer.labels,
    assignment: response.assignment ?? "per_slot",
  };
}

function findFilesNamed(root, fileName, files = []) {
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const target = path.join(root, entry.name);
    if (entry.isDirectory()) findFilesNamed(target, fileName, files);
    else if (entry.isFile() && entry.name === fileName) files.push(target);
  }
  return files;
}

async function importThroughUi() {
  const before = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(row => row.getAttribute('data-item-id'))`);
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
  await sleep(500);
  await session.clickSelector('[data-testid="import-pick-folder"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked-pdf" });
  await session.clickSelector('[data-testid="import-start"]');
  const deadline = Date.now() + 120000;
  while (Date.now() < deadline) {
    const ids = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(row => row.getAttribute('data-item-id'))`);
    const id = (ids ?? []).find((value) => !(before ?? []).includes(value));
    if (id) return id;
    await sleep(500);
  }
  throw new Error("PDF import did not create a library row");
}

async function waitForDraft(itemId, timeoutMs = 240000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const item = await workspace(itemId).catch(() => null);
    if (item?.ds?.taskGroups?.length && item?.ds?.answerSlots) return item;
    await sleep(1000);
  }
  throw new Error(`local first draft did not appear for ${itemId}`);
}

async function waitForCandidateSettled(itemId, requestLog, timeoutMs = 240000) {
  const deadline = Date.now() + timeoutMs;
  let seenCandidateRequest = false;
  while (Date.now() < deadline) {
    const log = fs.existsSync(requestLog) ? fs.readFileSync(requestLog, "utf8") : "";
    seenCandidateRequest ||= log.includes("generate_authoring_candidate");
    const decision = await invoke("get_recognition_decision", { itemId }).catch(() => null);
    const cloud = decision?.chains?.cloud?.state;
    if (seenCandidateRequest && cloud && !["queued", "running"].includes(cloud)) return { cloud, decision };
    await sleep(750);
  }
  throw new Error(`prepass candidate did not settle (requestSeen=${seenCandidateRequest})`);
}

function clone(value) { return JSON.parse(JSON.stringify(value)); }

function headingSlots(ds) {
  const group = (ds.taskGroups ?? []).find((task) => /heading/iu.test(String(task.taskType ?? "")));
  if (!group) throw new Error("local first draft has no heading task group");
  const slots = (group.responseGroups ?? []).flatMap((response) => response.slotIds ?? []);
  if (!slots.length) throw new Error("heading task group has no answer slots");
  return { group, slots };
}

function deriveCandidate(ds) {
  const map = ds.passage?.paragraphMap ?? {};
  if (!Object.keys(map).length) throw new Error("local first draft has no passage.paragraphMap");
  const reverse = new Map(Object.entries(map).map(([label, nodeId]) => [nodeId, label]));
  const candidate = {
    taskGroups: clone(ds.taskGroups),
    answerSlots: clone(ds.answerSlots),
    answerKey: clone(ds.answerKey ?? {}),
    answerPageEvidence: clone(ds.answerPageEvidence ?? []),
    unresolvedRegions: clone(ds.unresolvedRegions ?? []),
    sourceCoverageNotes: clone(ds.sourceCoverageNotes ?? []),
    warnings: clone(ds.warnings ?? []),
  };
  const { group, slots } = headingSlots(candidate);
  for (const slotId of slots) {
    const slot = candidate.answerSlots[slotId];
    const label = reverse.get(slot?.hostNodeId);
    if (!label) throw new Error(`heading slot ${slotId} is not anchored to a mapped source paragraph`);
    slot.hostType = "passage_paragraph";
    slot.interaction = "dragdrop";
    slot.hostNodeId = `@paragraph:${label}`;
  }
  report.headingCandidate = {
    taskType: group.taskType,
    slotIds: slots,
    placeholderTargets: Object.fromEntries(slots.map((slotId) => [slotId, candidate.answerSlots[slotId].hostNodeId])),
    sourceParagraphLabels: Object.keys(map),
  };
  return candidate;
}

function candidateTrace(jobId) {
  const dir = path.join(appDataDir, "jobs", String(jobId), "cache", "llm");
  if (!fs.existsSync(dir)) throw new Error(`LLM trace directory missing: ${dir}`);
  const inputs = fs.readdirSync(dir)
    .map((file) => /^generate_authoring_candidate-input-(\d+(?:-\d+)?)\.json$/u.exec(file))
    .filter(Boolean)
    .map((match) => ({ stamp: match[1], file: `generate_authoring_candidate-input-${match[1]}.json` }));
  if (!inputs.length) throw new Error("no generate_authoring_candidate request trace was saved");
  return inputs.map(({ stamp, file }) => {
    const input = JSON.parse(fs.readFileSync(path.join(dir, file), "utf8"));
    const outputPath = path.join(dir, `generate_authoring_candidate-output-${stamp}.json`);
    const output = fs.existsSync(outputPath) ? JSON.parse(fs.readFileSync(outputPath, "utf8")) : null;
    return { input, output };
  });
}

async function waitForCloudTerminal(itemId, timeoutMs = 480000) {
  const deadline = Date.now() + timeoutMs;
  const requestLog = path.join(appDataDir, "jobs", String(itemId), "llm-calls.jsonl");
  const traceDir = path.join(appDataDir, "jobs", String(itemId), "cache", "llm");
  const databasePath = path.join(appDataDir, "authoring_hub.db");
  let seenCandidateRequest = false;
  let lastJobState = null;
  while (Date.now() < deadline) {
    const log = fs.existsSync(requestLog) ? fs.readFileSync(requestLog, "utf8") : "";
    seenCandidateRequest ||= log.includes("generate_authoring_candidate");
    let candidateResponseSaved = false;
    if (fs.existsSync(traceDir)) {
      candidateResponseSaved = fs.readdirSync(traceDir).some((file) =>
        /^generate_authoring_candidate-(?:output|rejected)-\d+(?:-\d+)?\.json$/u.test(file),
      );
    }
    if (fs.existsSync(databasePath)) {
      const db = new DatabaseSync(databasePath, { readOnly: true });
      try {
        db.exec("PRAGMA busy_timeout = 5000");
        db.exec("PRAGMA query_only = ON");
        lastJobState = db.prepare(
          "SELECT stage, local_status, cloud_status, lease_owner FROM processing_jobs_v2 WHERE id = ?",
        ).get(itemId) ?? null;
      } finally {
        db.close();
      }
    }
    const decision = await invoke("get_recognition_decision", { itemId }).catch(() => null);
    const cloudStatus = lastJobState?.cloud_status;
    const decisionCloudStatus = decision?.chains?.cloud?.state;
    if (
      seenCandidateRequest
      && candidateResponseSaved
      && cloudStatus
      && !["queued", "running"].includes(cloudStatus)
      && ["ready_for_review", "failed", "cancelled"].includes(lastJobState?.stage)
      && lastJobState?.lease_owner == null
      && decision?.batchId
      && decisionCloudStatus
      && !["queued", "running", "not_run"].includes(decisionCloudStatus)
    ) {
      report.processingJobState = lastJobState;
      return decision;
    }
    await sleep(1000);
  }
  throw new Error(
    `cloud processing did not reach a terminal state (candidateRequestSeen=${seenCandidateRequest}, job=${JSON.stringify(lastJobState)})`,
  );
}

async function dragOptionToTarget(optionLabel, slotId) {
  const points = await session.evaluate(`(() => {
    const option = [...document.querySelectorAll('[data-option-label]')].find(el => el.getAttribute('data-option-label') === ${JSON.stringify(optionLabel)});
    const target = document.querySelector('[data-answer-drop-slot="' + ${JSON.stringify(slotId)} + '"]');
    if (!option || !target) return null;
    const a = option.getBoundingClientRect(), b = target.getBoundingClientRect();
    return { from: { x: a.x + a.width / 2, y: a.y + a.height / 2 }, to: { x: b.x + b.width / 2, y: b.y + b.height / 2 } };
  })()`);
  if (!points) throw new Error("option or answer target is not present in the real canvas");
  const send = (type, p, buttons, button) => session.cdp.send("Input.dispatchMouseEvent", {
    type, x: Math.round(p.x), y: Math.round(p.y), button, buttons: buttons ? 1 : 0, clickCount: type.includes("Pressed") || type.includes("Released") ? 1 : 0,
  });
  await send("mouseMoved", points.from, false, "none");
  await send("mousePressed", points.from, true, "left");
  for (let step = 1; step <= 8; step += 1) {
    await send("mouseMoved", {
      x: points.from.x + (points.to.x - points.from.x) * step / 8,
      y: points.from.y + (points.to.y - points.from.y) * step / 8,
    }, true, "left");
    await sleep(60);
  }
  await send("mouseReleased", points.to, false, "left");
}

async function main() {
  if (!fs.existsSync(fixturePath)) throw new Error(`PDF fixture missing: ${fixturePath}`);
  const freshness = assertBuildFresh({ exePath, tolerateConcurrentEdits: false });
  report.buildFresh = freshness;
  report.exeSha256 = sha256File(exePath);
  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  fs.mkdirSync(path.join(appDataDir, "config", "secrets"), { recursive: true });
  fs.mkdirSync(scenarioDir, { recursive: true });
  fs.copyFileSync(fixturePath, path.join(runDir, "pdfs", path.basename(fixturePath)));
  const profile = {
    profileId: PROFILE_ID,
    name: "Controlled Heading Presentation",
    provider: "OpenAiCompatible",
    baseUrl: `http://127.0.0.1:${servicePort}/v1`,
    model: "controlled-outline-v1",
    temperature: 0,
    timeoutMs: 120000,
    forceJson: true,
    enabled: true,
  };
  fs.writeFileSync(path.join(appDataDir, "config", "llm-profiles.json"), JSON.stringify([profile], null, 2));
  fs.writeFileSync(path.join(appDataDir, "config", "secrets", `${PROFILE_ID}.key`), "controlled-service-token");

  startService();
  await waitForService();
  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: "--no-sandbox --disable-gpu",
    appEnv: { EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK: "1", IELTS_LLM_DIAGNOSTICS: "1" },
  });
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library" });
  const prepassId = await importThroughUi();
  report.prepassItemId = prepassId;
  const prepass = await waitForDraft(prepassId);
  const sourceParagraphMap = clone(prepass.ds.passage?.paragraphMap ?? {});
  const requestLogFallback = path.join(appDataDir, "jobs", String(prepassId), "llm-calls.jsonl");
  await waitForCandidateSettled(prepassId, requestLogFallback);
  const candidate = deriveCandidate(prepass.ds);
  fs.writeFileSync(candidatePath, JSON.stringify(candidate, null, 2));

  await stopService();
  startService(candidatePath);
  const serviceHealth = await waitForService();
  assert("controlled-service-loaded-candidate", Boolean(serviceHealth.candidate), JSON.stringify(serviceHealth));

  const itemId = await importThroughUi();
  report.itemId = itemId;
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace" });
  const beforeCandidate = await waitForDraft(itemId);
  const preVersion = Number(beforeCandidate.editVersion ?? beforeCandidate.ds?.editVersion ?? 0);
  const decision = await waitForCloudTerminal(itemId);
  report.cloudDecision = decision;

  const finalItem = await workspace(itemId);
  const finalDs = finalItem.ds;
  const { group, slots } = headingSlots(finalDs);
  const finalMap = finalDs.passage?.paragraphMap ?? {};
  const targetProblems = slots.flatMap((slotId) => {
    const slot = finalDs.answerSlots[slotId];
    return slot.hostType === "passage_paragraph"
      && slot.interaction === "dragdrop"
      && Object.values(finalMap).includes(slot.hostNodeId)
      ? []
      : [`${slotId} => ${slot.hostType}/${slot.interaction}/${slot.hostNodeId}`];
  });
  const traces = candidateTrace(itemId);
  const candidateOutputSlots = traces.flatMap(({ input, output }) => {
    const map = input?.sourceParagraphs?.paragraphMap ?? {};
    return Object.entries(output?.answerSlots ?? {})
      .filter(([, slot]) => slot?.hostType === "passage_paragraph")
      .map(([slotId, slot]) => ({ slotId, hostNodeId: slot.hostNodeId, requestParagraphMap: map }));
  });
  const returnedAgainstRequest = candidateOutputSlots.length > 0 && candidateOutputSlots.every(({ hostNodeId, requestParagraphMap }) => Object.values(requestParagraphMap).includes(hostNodeId));
  assert("candidate-output-uses-this-job-paragraph-map", returnedAgainstRequest, JSON.stringify(candidateOutputSlots));
  assert("candidate-adopted-and-authoritative-anchors-valid", decision.repair?.candidateAdoption?.adopted === true && targetProblems.length === 0, JSON.stringify({ adoption: decision.repair?.candidateAdoption, targetProblems }));

  const uiTargets = await session.evaluate(`(() => {
    const slots = ${JSON.stringify(slots)};
    return slots.map(slotId => {
      const target = document.querySelector('[data-answer-drop-slot="' + slotId + '"]');
      const wrapper = target?.closest('.v2-answer-dropzone-wrap');
      const paragraph = wrapper?.nextElementSibling;
      return { slotId, targetBeforeParagraph: Boolean(target && paragraph?.matches('.v2-paragraph')), hostNodeId: paragraph?.getAttribute('data-editor-id') ?? null, value: target?.querySelector('.v2-answer-dropzone-value')?.textContent?.trim() ?? null };
    });
  })()`);
  const uiProblems = uiTargets.filter(({ slotId, targetBeforeParagraph, hostNodeId }) => {
    const slot = finalDs.answerSlots[slotId];
    return !targetBeforeParagraph || hostNodeId !== slot.hostNodeId;
  });
  assert("canvas-renders-each-target-before-authoritative-paragraph", uiProblems.length === 0, JSON.stringify({ taskType: group.taskType, targets: uiTargets, problems: uiProblems }));

  const slotId = slots[0];
  const task = finalDs.taskGroups.find((entry) => entry.taskId === group.taskId);
  const response = task.responseGroups.find((entry) => entry.slotIds.includes(slotId));
  const options = response.options?.length ? response.options : task.optionBank?.options ?? [];
  if (!options.length) throw new Error("heading task has no response option bank");
  const optionLabel = verifiedSourceAnswers[slotId]?.labels?.[0];
  if (!optionLabel || !options.some((option) => option.label.toLowerCase() === optionLabel.toLowerCase())) {
    throw new Error(`verified source answer for ${slotId} is missing from the real heading option pool`);
  }
  await dragOptionToTarget(optionLabel, slotId);
  const persistedDeadline = Date.now() + 90000;
  let persisted = null;
  while (Date.now() < persistedDeadline) {
    const current = await workspace(itemId).catch(() => null);
    const answer = current?.ds?.answerKey?.[slotId];
    if (current?.editVersion > preVersion && answer?.kind === "option" && answer.labels?.includes(optionLabel)) {
      persisted = current;
      break;
    }
    await sleep(500);
  }
  assert("pointer-drop-saved-to-authoritative-draft", Boolean(persisted), JSON.stringify({ slotId, optionLabel, preVersion, observed: persisted?.editVersion ?? null }));

  await session.clickSelector('[data-testid="workspace-back"]');
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000, label: "library-after-back" });
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-reopen" });
  const reopened = await session.evaluate(`(() => {
    const target = document.querySelector('[data-answer-drop-slot="' + ${JSON.stringify(slotId)} + '"]');
    return target?.querySelector('.v2-answer-dropzone-value')?.textContent?.trim() ?? null;
  })()`);
  const reopenedItem = await workspace(itemId);
  const reopenedAnswer = reopenedItem.ds?.answerKey?.[slotId];
  assert("answer-and-target-survive-workspace-reopen", reopenedAnswer?.labels?.includes(optionLabel) && reopened !== "Drop heading here", JSON.stringify({ slotId, optionLabel, reopened, reopenedAnswer, editVersion: reopenedItem.editVersion }));

  const beforeExport = reopenedItem.ds;
  const sourceReview = await invoke("resolve_source_review", {
    jobId: itemId,
    note: "已逐页视觉复核原始 PDF 第 1–5 页：确认 A–G 正文、题目选项与第 5 页扫描答案表；答案 14–26 逐项转录，b105 已人工回看。",
  });
  assert("source-review-completed-from-original-pages", sourceReview?.resolved === true && sourceReview?.stale !== true, JSON.stringify(sourceReview));
  const exportPreflightBefore = await invoke("get_publish_preflight", { jobId: itemId });
  const answerCommands = [];
  for (const exportGroup of beforeExport.taskGroups ?? []) {
    for (const exportResponse of exportGroup.responseGroups ?? []) {
      for (const exportSlotId of exportResponse.slotIds ?? []) {
        if (beforeExport.answerKey?.[exportSlotId]?.kind !== "unresolved") continue;
        const value = controlledExportAnswer(
          beforeExport,
          exportGroup,
          exportResponse,
          exportSlotId,
        );
        if (value) answerCommands.push({ op: "setAnswer", slotId: exportSlotId, value });
      }
    }
  }
  if (answerCommands.length) {
    // The source PDF has no answer key; placeholders live only in this isolated product E2E package.
    await invoke("apply_editor_commands", {
      input: { itemId, commands: answerCommands, baseVersion: reopenedItem.editVersion },
    });
  }
  const readyForExport = await workspace(itemId);
  const exportPreflightAfter = await invoke("get_publish_preflight", { jobId: itemId });
  const publishReady = exportPreflightAfter?.publishVerdict?.ready === true
    || exportPreflightAfter?.passed === true;
  assert(
    "heading-package-preflight-ready",
    publishReady,
    JSON.stringify({ before: exportPreflightBefore, after: exportPreflightAfter, answerCommands: answerCommands.length }),
  );

  fs.mkdirSync(nasDir, { recursive: true });
  await session.evaluate(`(() => {
    const key = "ielts-author-studio.app-settings.v1";
    let current = {};
    try { current = JSON.parse(window.localStorage.getItem(key) ?? "{}"); } catch {}
    current.nasDestination = ${JSON.stringify(nasDir)};
    current.developerMode = true;
    window.localStorage.setItem(key, JSON.stringify(current));
    window.localStorage.setItem("ielts-author-studio.confirmed-nas-export-dir.v1", ${JSON.stringify(nasDir)});
    return current.nasDestination;
  })()`);
  const noticeBeforePublish = await session.evaluate(
    `(() => { const el = document.querySelector('.workspace-notice'); return el ? el.innerText.replace(/\\s+/g,' ').trim() : null; })()`,
  );
  await session.clickSelector('[data-testid="workspace-publish"]');
  const publishNotice = await session.waitFor(
    `(() => { const el = document.querySelector('.workspace-notice'); const text = el ? el.innerText.replace(/\\s+/g,' ').trim() : null; return text && text !== ${JSON.stringify(noticeBeforePublish)} ? text : null; })()`,
    { timeoutMs: 120000, intervalMs: 1000, label: "heading-package-publish" },
  );
  const publishOutcome = await session.evaluate(
    `(() => document.querySelector('.workspace-notice')?.getAttribute('data-publish-outcome') ?? null)()`,
  );
  assert("heading-package-published", publishOutcome === "published", JSON.stringify({ publishNotice, publishOutcome, status: readyForExport.status }));

  const runtimeFiles = findFilesNamed(nasDir, "reading-source-v2.json");
  assert("heading-runtime-package-written", runtimeFiles.length > 0, JSON.stringify({ nasDir, runtimeFiles }));
  const runtime = JSON.parse(fs.readFileSync(runtimeFiles[0], "utf8"));
  const runtimeHeadingGroup = (runtime.taskGroups ?? []).find((entry) => entry.taskType === "matching_headings");
  const runtimeTargets = slots.map((targetSlotId) => {
    const target = runtime.answerSlots?.[targetSlotId];
    return {
      slotId: targetSlotId,
      hostType: target?.hostType,
      interaction: target?.interaction,
      hostNodeId: target?.hostNodeId,
      mapped: Object.values(runtime.passage?.paragraphMap ?? {}).includes(target?.hostNodeId),
      answer: runtime.answerKey?.[targetSlotId],
    };
  });
  const runtimeOptions = runtimeHeadingGroup?.optionBank?.options
    ?? runtimeHeadingGroup?.responseGroups?.flatMap((entry) => entry.options ?? [])[0]
    ?? [];
  const runtimeProblems = runtimeTargets.filter((target) =>
    target.hostType !== "passage_paragraph"
    || target.interaction !== "dragdrop"
    || !target.mapped
    || target.answer?.kind !== "option",
  );
  const answerProblems = Object.entries(verifiedSourceAnswers).flatMap(([answerSlotId, expected]) => {
    const actual = runtime.answerKey?.[answerSlotId];
    const matches = expected.kind === "option"
      ? actual?.kind === "option" && JSON.stringify(actual.labels) === JSON.stringify(expected.labels)
      : actual?.kind === "text" && JSON.stringify(actual.values) === JSON.stringify(expected.values);
    return matches ? [] : [{ slotId: answerSlotId, expected, actual }];
  });
  assert(
    "heading-runtime-preserves-anchors-and-option-pool",
    Boolean(runtimeHeadingGroup) && runtimeProblems.length === 0 && answerProblems.length === 0 && runtimeOptions.length >= slots.length,
    JSON.stringify({ runtimeProblems, answerProblems, optionLabels: runtimeOptions.map((entry) => entry.label), paragraphMap: runtime.passage?.paragraphMap }),
  );

  report.result = {
    prepassParagraphMap: sourceParagraphMap,
    finalParagraphMap: finalMap,
    taskType: group.taskType,
    headingSlots: slots,
    candidateTraceCount: traces.length,
    candidateOutputSlots,
    dragged: { slotId, optionLabel },
    savedEditVersion: persisted.editVersion,
    reopenedEditVersion: reopenedItem.editVersion,
    export: {
      packageDir: nasDir,
      runtimeSourcePath: runtimeFiles[0],
      answerCommands: answerCommands.length,
      publishOutcome,
      headingTaskType: runtimeHeadingGroup?.taskType ?? null,
      optionLabels: runtimeOptions.map((entry) => entry.label),
      targets: runtimeTargets,
    },
  };
  report.verdict = "passed";
}

try {
  await main();
} catch (error) {
  report.error = String(error?.stack ?? error);
  console.error(`[heading-presentation] FAIL ${report.error}`);
  process.exitCode = 1;
} finally {
  await stopService().catch(() => {});
  if (session) await session.close().catch(() => {});
  report.finishedAt = new Date().toISOString();
  if (fs.existsSync(runDir)) writeReport(runDir, report);
}
