#!/usr/bin/env node
/**
 * **云端自主修复闭环**的真实产品链验收（WebView2 CDP）。
 *
 * 它回答的问题不是「有没有新代码」，而是三句话：
 *   1. 云端**自己**解决了什么？（不点任何「采用」按钮）
 *   2. 用户实际还需要做几次操作？每一次是什么？
 *   3. 剩下的为什么**不能**自动处理？
 *
 * 为什么必须走真实 Tauri：
 *   后端已有 `cloud_repair/tests.rs` 沿真实调度函数证明过「云端可以纠正本地结论」，
 *   但那是在**进程内**按生产顺序调函数。它证明不了：真实导入会发起完整候选请求、
 *   真实网关会把 5 个工具的往返跑完、真实权威稿会被改、真实画布会跟着刷新、
 *   真实导出/学生端会看到改后的内容。本脚本补的就是这一段。
 *
 * 链条（PDF 与 DOCX 同一条）：
 *   导入 → 本地初稿可见 → 云端自动修改 → 画布更新 → 编辑保存 → 重开 → 学生预览 → 导出 → 学生端加载
 *
 * 受控模型服务（`scripts/controlled-llm-service.mjs`）在这一轮被扩到支持
 * `generate_authoring_candidate` 与 `repair_authoring_step`：候选样本与修复剧本由
 * `lib/cloud-repair-scenario.mjs` **从真实本地稿派生**（见该文件的边界说明）。
 *
 * 两遍识别是**有意**的，不是绕路：
 *   候选样本必须与「这一份真实稿」逐字段可比，而稿子只有导入之后才存在。
 *   第 1 遍让本地稿落盘（此时云端候选被受控服务如实拒绝，正好顺带证明
 *   「本地稿不被云端拖慢」）；派生样本后重启受控服务，再**导入第二个条目**跑第 2 遍
 *   ——第 2 遍才是被断言的那一遍。
 *
 * 为什么不复用同一个条目重跑：`retry_processing` 走的本地闭包不带
 * `allowOverwrite`，而 `run_auto_pipeline_core` 在 `authoring-ir.json` 已存在时会以
 * `editable_draft_exists` 拒绝，因此对已有稿子的条目**无法重新识别**。
 * 这是本轮真实链路上发现的**产品缺陷**（见报告 `defect-retry-cannot-rerun`），
 * 不是本脚本的绕路：第二遍本来就是一次全新的完整导入。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs [--pdf <path>] [--port N] [--keep]
 *        [--no-diagnostic-args] [--skip-export]
 *
 * 退出码：0 通过 / 1 失败 / 2 部分无法执行 / 3 环境不满足 / 5 全部无法执行
 */

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  CDP_CHANNEL_LABEL,
  CDP_CHANNEL_NOTE,
  CannotRunError,
  assertBuildFresh,
  buildFreshReport,
  gitHead,
  gitWorktreeClean,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  sleep,
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";
import { computeScenarioVerdict, SCENARIO_STATUS } from "./lib/chain-verdict.mjs";
import { deriveRepairScenario, textOfNodes } from "./lib/cloud-repair-scenario.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const skipExport = process.argv.includes("--skip-export");
const pdfIdx = process.argv.indexOf("--pdf");
const fixturePath = path.resolve(
  pdfIdx >= 0 ? process.argv[pdfIdx + 1] : path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf"),
);
const portIdx = process.argv.indexOf("--port");
const servicePort = portIdx >= 0 ? Number(process.argv[portIdx + 1]) : 11455;
const isPdf = /\.pdf$/i.test(fixturePath);
const extraArgs = process.argv.includes("--no-diagnostic-args") ? "" : "--no-sandbox --disable-gpu";

const serviceScript = path.join(repoRoot, "scripts", "controlled-llm-service.mjs");
const PROFILE_ID = "controlled-repair-chain";
const PYTHON = process.env.PDF2TEST_PYTHON
  ?? "C:/Users/25788/.workbuddy-ai/binaries/python/versions/3.13.12/python.exe";

const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `run-cloud-repair-chain-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const appDataDir = path.join(runDir, "appdata", "data");
const dbPath = path.join(appDataDir, "authoring_hub.db");
const scenarioDir = path.join(runDir, "scenario");
const candidatePath = path.join(scenarioDir, "authoring-candidate.json");
const planPath = path.join(scenarioDir, "repair-plan.json");
const nasDir = path.join(runDir, "nas-library");

const report = {
  task: "cloud-repair-autonomous-loop-product-chain",
  scope: "真实 Tauri 导入 → 云端完整候选 → 真实修复循环 → 权威稿/画布/导出（不是进程内函数调用）",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  runProfile: extraArgs ? "cdp-with-stability-args" : "cdp-plain",
  stabilityArgs: extraArgs ? extraArgs.split(/\s+/).filter(Boolean) : [],
  startedAt: new Date().toISOString(),
  runDir,
  identity: {
    exePath,
    exeSha256: null,
    fixturePath,
    fixtureSha256: null,
    fixtureKind: isPdf ? "pdf" : "docx",
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: null,
    serviceScript,
    servicePort,
  },
  service: { started: false, health: null, modes: [], requestLines: [] },
  scenario: { derived: false, differences: [], fix: null, rule: null, unresolved: [] },
  observed: {
    localDraft: null,
    firstCycle: null,
    repairProgressSeen: [],
    repairFinal: null,
    exportAttempt: null,
  },
  scenarios: [],
  findings: [],
  verdict: "failed",
};

let session = null;
let itemId = null;
let serviceChild = null;

function record(name, status, detail, error) {
  const entry = { name, status, detail: detail ?? null };
  if (error) entry.error = String(error?.message ?? error);
  report.scenarios.push(entry);
  console.log(`[scenario] ${status.toUpperCase()} ${name}${entry.error ? ` :: ${entry.error}` : ""}`);
}

function notExecutable(name, reason) {
  record(name, SCENARIO_STATUS.NOT_EXECUTABLE, { reason });
}

async function call(command, args = {}) {
  const wrapped = command === "apply_recognition_decisions" || command === "apply_editor_commands";
  const r = await session.invoke(command, wrapped ? { input: args } : args);
  if (r && r.__noInvoke) throw new Error("invoke 不可用");
  return r;
}

async function readWorkspace() {
  const r = await call("get_workspace_item", { itemId });
  if (!r?.ok) throw new Error(`get_workspace_item 失败：${r?.error}`);
  return r.value;
}

async function readDecision() {
  const r = await call("get_recognition_decision", { itemId });
  if (!r?.ok) throw new Error(`get_recognition_decision 失败：${r?.error}`);
  return r.value ?? {};
}

/**
 * 把发布目标目录写进**产品自己的设置**（`localStorage`，见 `appSettings.ts`）。
 *
 * 为什么必须做：`publish()` 在 `nasDestination` 为空时会调 `chooseExportDirectory()`
 * 弹**原生目录选择框**。自动化里那个框没人点，点「发布」就会一直挂在那里——
 * 上一版正是如此，界面上留下的还是点发布之前的那条提示，于是把一条无关的
 * 「建议已过期」文案当成了发布结果。
 */
async function configureNasDestination() {
  await session.evaluate(
    `(() => {
       const key = "ielts-author-studio.app-settings.v1";
       let current = {};
       try { current = JSON.parse(window.localStorage.getItem(key) ?? "{}"); } catch { current = {}; }
       current.nasDestination = ${JSON.stringify(nasDir)};
       current.developerMode = true;
       window.localStorage.setItem(key, JSON.stringify(current));
       window.localStorage.setItem("ielts-author-studio.confirmed-nas-export-dir.v1", ${JSON.stringify(nasDir)});
       return current.nasDestination;
     })()`,
  );
  report.observed.nasDestination = nasDir;
}

async function openPanel() {
  const already = await session.evaluate(`!!document.querySelector('[data-testid="workspace-recognition"]')`);
  if (!already) await session.clickSelector('[data-testid="workspace-recognition-toggle"]');
  await session.waitFor(`!!document.querySelector('[data-testid="workspace-recognition"]')`, {
    timeoutMs: 20000,
    label: "recognition-panel",
  });
}

async function closePanel() {
  const open = await session.evaluate(`!!document.querySelector('[data-testid="workspace-recognition"]')`);
  if (!open) return;
  await session.clickSelector('[data-testid="workspace-recognition-toggle"]');
  await session.waitFor(`!document.querySelector('[data-testid="workspace-recognition"]')`, {
    timeoutMs: 20000,
    label: "recognition-panel-closed",
  });
}

async function reopenWorkspace() {
  await session.clickSelector('[data-testid="workspace-back"]');
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000, label: "library-after-back" });
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-reopen" });
  await openPanel();
}

/**
 * 走真实界面导入一份文件，返回新出现的题库条目 id。
 *
 * 为什么不用 IPC：`import-pick-folder` / `import-start` 是产品真正的导入入口，
 * 它们决定了云端 profile 是否被选中、`cloudEnabled` 是否打开。绕过去就等于
 * 验收了一条产品里并不存在的路径。
 */
async function importThroughUi() {
  const before = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
  await session.clickSelector(isPdf ? '[data-testid="import-pick-folder"]' : '[data-testid="import-pick-files"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked-files" });
  await session.clickSelector('[data-testid="import-start"]');
  const deadline = Date.now() + 120000;
  while (Date.now() < deadline) {
    const ids = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
    const found = (ids ?? []).find((id) => !(before ?? []).includes(id));
    if (found) return found;
    await sleep(1000);
  }
  throw new CannotRunError("导入后未出现新的题库行");
}

/** 等某个条目的权威稿落盘。返回 `{ds, editVersion, item}` 或 `null`。 */
async function waitForLocalDraft(targetItemId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const r = await call("get_workspace_item", { itemId: targetItemId });
    if (r?.ok && r.value?.ds) return r.value;
    await sleep(2000);
  }
  return null;
}

async function waitForService(timeoutMs = 20000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    try {
      const res = await fetch(`http://127.0.0.1:${servicePort}/health`);
      if (res.ok) return await res.json();
    } catch {
      // 还没起来
    }
    await sleep(250);
  }
  return null;
}

function startService({ candidate = null, plan = null } = {}) {
  const args = [serviceScript, "--port", String(servicePort), "--mode", "normal"];
  if (candidate) args.push("--candidate", candidate);
  if (plan) args.push("--plan", plan);
  serviceChild = spawn(process.execPath, args, {
    cwd: repoRoot,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let out = "";
  const collect = (chunk) => {
    out += String(chunk);
    for (const line of String(chunk).split(/\r?\n/)) {
      if (line.includes("[controlled-llm]") && line.includes("POST")) report.service.requestLines.push(line.trim());
    }
  };
  serviceChild.stdout.on("data", collect);
  serviceChild.stderr.on("data", collect);
  report.service.log = () => out;
  report.service.modes.push(candidate ? "candidate+plan" : "skeleton");
  return out;
}

async function restartService(options) {
  if (serviceChild) {
    serviceChild.kill();
    serviceChild = null;
    await sleep(900);
  }
  startService(options);
  const health = await waitForService();
  if (!health) throw new CannotRunError("受控服务没有就绪（/health 不可达）");
  report.service.health = health;
  return health;
}

function writeProfile() {
  const configDir = path.join(appDataDir, "config");
  fs.mkdirSync(path.join(configDir, "secrets"), { recursive: true });
  const profile = {
    profileId: PROFILE_ID,
    name: "Controlled Repair Chain Service",
    provider: "OpenAiCompatible",
    baseUrl: `http://127.0.0.1:${servicePort}/v1`,
    model: "controlled-outline-v1",
    temperature: 0,
    timeoutMs: 120000,
    forceJson: true,
    enabled: true,
  };
  fs.writeFileSync(path.join(configDir, "llm-profiles.json"), JSON.stringify([profile], null, 2));
  fs.writeFileSync(path.join(configDir, "secrets", `${PROFILE_ID}.key`), "controlled-service-token");
  return profile;
}

/** 作业目录里网关调用的痕迹与**模型工具往返记录**。 */
function llmTraces(jobId) {
  const jobDir = path.join(appDataDir, "jobs", String(jobId ?? ""));
  const dir = path.join(jobDir, "cache", "llm");
  const traces = { dir, exists: fs.existsSync(dir), byCommand: {}, repairRounds: [], callRecords: [] };
  if (traces.exists) {
    const files = fs.readdirSync(dir);
    for (const file of files) {
      const matched = /^(.+)-(input|output)-(\d+)\.json$/u.exec(file);
      if (!matched) continue;
      const command = matched[1];
      traces.byCommand[command] = traces.byCommand[command] ?? { input: 0, output: 0 };
      traces.byCommand[command][matched[2]] += 1;
    }
    const inputs = files
      .map((file) => /^repair_authoring_step-input-(\d+)\.json$/u.exec(file))
      .filter(Boolean)
      .map((matched) => ({ stamp: Number(matched[1]), file: matched[0] }))
      .sort((a, b) => a.stamp - b.stamp);
    for (const entry of inputs) {
      try {
        const input = JSON.parse(fs.readFileSync(path.join(dir, entry.file), "utf8"));
        traces.repairRounds.push({
          stamp: entry.stamp,
          observations: Array.isArray(input.observations) ? input.observations.length : 0,
          differences: (input.context?.differences ?? []).length,
          editVersion: input.context?.editVersion ?? null,
          protectedTargets: input.context?.protectedTargets ?? [],
        });
      } catch {
        traces.repairRounds.push({ stamp: entry.stamp, error: "unparsable" });
      }
    }
  }
  const recordPath = path.join(jobDir, "llm-calls.jsonl");
  if (fs.existsSync(recordPath)) {
    traces.callRecords = fs
      .readFileSync(recordPath, "utf8")
      .split(/\r?\n/)
      .filter(Boolean)
      .map((line) => {
        try {
          const parsed = JSON.parse(line);
          return { commandName: parsed.commandName, ok: parsed.ok, errorClass: parsed.errorClass ?? null, latencyMs: parsed.latencyMs ?? null };
        } catch {
          return { raw: line.slice(0, 200) };
        }
      });
  }
  return traces;
}

/** 从修复回合的**输出**文件里取每一轮模型的工具调用（这是「模型工具记录」原件）。 */
function repairToolCalls(jobId) {
  const dir = path.join(appDataDir, "jobs", String(jobId ?? ""), "cache", "llm");
  if (!fs.existsSync(dir)) return [];
  return fs
    .readdirSync(dir)
    .map((file) => /^repair_authoring_step-output-(\d+)\.json$/u.exec(file))
    .filter(Boolean)
    .map((matched) => ({ stamp: Number(matched[1]), file: matched[0] }))
    .sort((a, b) => a.stamp - b.stamp)
    .map((entry) => {
      try {
        const raw = JSON.parse(fs.readFileSync(path.join(dir, entry.file), "utf8"));
        // 落盘的是**已解析的工具调用本身**（`{callId, tool, arguments}`），不是 OpenAI 那种
        // `choices[0].message.content` 信封。上一版按信封解，于是每一轮的 tool 都是 null，
        // 断言把「解析形状写错了」误报成「模型没按反馈修正」。两种形状都认。
        const envelope = raw?.choices?.[0]?.message?.content;
        const call =
          raw && typeof raw.tool === "string"
            ? raw
            : typeof envelope === "string"
              ? JSON.parse(envelope)
              : null;
        return {
          stamp: entry.stamp,
          tool: call?.tool ?? null,
          baseVersion: call?.arguments?.baseVersion ?? null,
          hasCommands: Array.isArray(call?.arguments?.commands),
          commands: (call?.arguments?.commands ?? []).map((command) => command?.op ?? null),
          rulings: (call?.arguments?.rulings ?? []).map((ruling) => `${ruling?.targetType}:${ruling?.targetId}:${ruling?.field}=${ruling?.ruling}`),
          unresolved: (call?.arguments?.unresolved ?? []).length,
        };
      } catch {
        return { stamp: entry.stamp, error: "unparsable" };
      }
    });
}

function dumpDb(outPath, label) {
  const result = spawnSync(PYTHON, [path.join(repoRoot, "scripts", "e2e", "lib", "dump-authoring-db.py"), dbPath, outPath, itemId ?? ""], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  report[`db${label}`] = { path: outPath, status: result.status, stdout: String(result.stdout ?? "").trim(), stderr: String(result.stderr ?? "").trim() };
  if (result.status !== 0) return null;
  return JSON.parse(fs.readFileSync(outPath, "utf8"));
}

/** 权威稿里某个作答组的题面文字（从数据库里读，不是从界面读）。 */
function promptTextOf(ds, responseGroupId) {
  for (const group of ds?.taskGroups ?? []) {
    for (const response of group.responseGroups ?? []) {
      if (response.responseGroupId === responseGroupId) return textOfNodes(response.prompt ?? []);
    }
  }
  return null;
}

/** 权威稿里的答案键（用于「用户补答案」这一段的断言）。 */
function answerOf(ds, slotId) {
  return (ds?.answerKey ?? {})[slotId] ?? null;
}

function diffCanonical(before, after) {
  const changes = [];
  const beforeGroups = new Map((before?.taskGroups ?? []).map((group) => [group.taskId, group]));
  for (const group of after?.taskGroups ?? []) {
    const previous = beforeGroups.get(group.taskId);
    if (!previous) {
      changes.push({ kind: "task_group_added", targetId: group.taskId });
      continue;
    }
    for (const [field, pointer] of [["instructions", "instructions"], ["stimulus", "stimulus"]]) {
      const a = textOfNodes(previous[field] ?? []);
      const b = textOfNodes(group[field] ?? []);
      if (a !== b) changes.push({ kind: `task_group.${pointer}`, targetId: group.taskId, before: a, after: b });
    }
    const beforeResponses = new Map((previous.responseGroups ?? []).map((r) => [r.responseGroupId, r]));
    for (const response of group.responseGroups ?? []) {
      const prior = beforeResponses.get(response.responseGroupId);
      const a = textOfNodes(prior?.prompt ?? []);
      const b = textOfNodes(response.prompt ?? []);
      if (a !== b) changes.push({ kind: "response_group.prompt", targetId: response.responseGroupId, before: a, after: b });
    }
  }
  for (const [slotId, value] of Object.entries(after?.answerKey ?? {})) {
    const prior = (before?.answerKey ?? {})[slotId] ?? null;
    if (JSON.stringify(prior) !== JSON.stringify(value)) {
      changes.push({ kind: "answer", targetId: slotId, before: prior, after: value });
    }
  }
  return changes;
}

async function main() {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: false });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);
  if (!fs.existsSync(fixturePath)) throw new CannotRunError(`夹具不存在：${fixturePath}`);
  report.identity.fixtureSha256 = sha256File(fixturePath);

  fs.mkdirSync(scenarioDir, { recursive: true });
  fs.mkdirSync(appDataDir, { recursive: true });
  fs.mkdirSync(path.join(runDir, "source-file"), { recursive: true });
  fs.copyFileSync(fixturePath, path.join(runDir, "source-file", path.basename(fixturePath)));

  // ---- 0. 起受控服务（**没有**候选样本：第 1 遍的云端候选会被如实拒绝）----
  startService({});
  const health0 = await waitForService();
  if (!health0) throw new CannotRunError("受控服务没有就绪");
  report.service.started = true;
  report.service.health = health0;

  report.profile = writeProfile();
  const staged = path.join(runDir, "pdfs", path.basename(fixturePath));
  fs.mkdirSync(path.dirname(staged), { recursive: true });
  fs.copyFileSync(fixturePath, staged);

  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    appEnv: {
      EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK: "1",
      ...(isPdf ? {} : { PDF2TEST_AUTOMATION_SOURCE_FILES: staged }),
    },
  });
  report.identity.browserArgs = session.browserArgs;
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });
  // 发布目标目录必须在**点发布之前**就写好，否则发布按钮会弹出原生目录选择框。
  await configureNasDestination();

  // ---- 1. 预跑一遍：只为**读到这份文件被本地识别成什么样** ----
  //
  // 为什么必须有这一遍：候选样本必须与「这一份真实稿」逐字段可比，而稿子只有导入
  // 之后才存在。这一遍的云端候选会被受控服务如实拒绝（它还没有样本），
  // 于是顺带证明了「本地稿不被云端拖慢」——本地稿照样落盘、照样可编辑。
  //
  // **被断言的链条是第二遍**（第 4 步），它是完整的一遍：导入 → 本地初稿 → 云端候选
  // → 修复循环。第一遍只用来派生样本，不对它做任何产品结论。
  itemId = await importThroughUi();
  report.identity.prepassItemId = itemId;
  const prepassDraft = await waitForLocalDraft(itemId, 180000);
  if (!prepassDraft) throw new CannotRunError("预跑一遍的本地稿在超时前没有落盘");
  record("prepass-import-for-scenario-derivation", SCENARIO_STATUS.PASSED, {
    itemId,
    taskGroups: (prepassDraft.ds.taskGroups ?? []).length,
    answerSlots: Object.keys(prepassDraft.ds.answerSlots ?? {}).length,
    qualityState: prepassDraft.ds.quality?.state ?? null,
  });

  // ---- 2. 从**真实稿**派生候选样本与修复剧本 ----
  const derived = deriveRepairScenario(prepassDraft.ds);
  if (!derived) {
    // 前提不成立时，必须把**这份稿子的真实形状**写进报告：只说「没有页脚残留」
    // 会让读者以为是脚本挑剔，而实测 DOCX 那一遍是 13 个题面全是占位符
    // `[prompt pending review]`——那是本地识别本身的问题，不是脚本的前提问题。
    const groups = prepassDraft.ds.taskGroups ?? [];
    const placeholder = /^\s*\[[^\]]*pending review[^\]]*\]\s*$/iu;
    const promptTexts = [];
    for (const group of groups) {
      for (const response of group.responseGroups ?? []) {
        promptTexts.push(textOfNodes(response.prompt ?? []));
      }
    }
    const placeholderCount = promptTexts.filter((text) => placeholder.test(text)).length;
    const blockingCodes = [
      ...new Set((prepassDraft.ds.quality?.issues ?? []).filter((issue) => issue?.severity === "blocking").map((issue) => issue.code)),
    ];
    report.observed.prepassDraftShape = {
      taskGroups: groups.length,
      responseGroups: promptTexts.length,
      placeholderPrompts: placeholderCount,
      emptyPrompts: promptTexts.filter((text) => !text).length,
      qualityState: prepassDraft.ds.quality?.state ?? null,
      blockingCodes,
      samplePrompt: promptTexts[0] ?? null,
    };
    notExecutable(
      "derive-scenario-from-real-draft",
      placeholderCount > 0
        ? `这份稿子的本地初稿是退化的：${promptTexts.length} 个题面里有 ${placeholderCount} 个是占位符 `
          + `（例如 ${JSON.stringify(promptTexts[0] ?? null)}），blocking 代码 ${JSON.stringify(blockingCodes)}。`
          + "没有任何可枚举的真实内容差异可供云端修复，硬造一份候选只会得到假结论。"
        : "这份稿子里没有可辨认的题面页脚残留，场景前提不成立。",
    );
    report.finishedAt = new Date().toISOString();
    fs.writeFileSync(path.join(runDir, "controlled-service.log"), report.service.log?.() ?? "");
    report.service.log = undefined;
    const verdict = computeScenarioVerdict({ scenarios: report.scenarios });
    report.verdict = verdict.verdict;
    report.exitCode = verdict.exitCode;
    report.scenarioFacts = {
      passed: verdict.passed,
      failed: verdict.failed,
      notExecutable: verdict.notExecutable,
      unknown: verdict.unknown,
      total: verdict.total,
      reason: verdict.reason,
    };
    writeReport(runDir, report);
    console.log(`[cloud-repair-chain] verdict: ${verdict.verdict} (${verdict.reason})`);
    process.exitCode = verdict.exitCode;
    return;
  }
  fs.writeFileSync(candidatePath, JSON.stringify(derived.candidate, null, 2));
  fs.writeFileSync(planPath, JSON.stringify(derived.plan, null, 2));
  report.scenario.derived = true;
  report.scenario.fix = derived.fix;
  report.scenario.rule = derived.rule;
  report.scenario.unresolved = derived.plan.unresolved;
  report.scenario.differences = [
    `task_group:${derived.rule.taskId}:instructions`,
    `response_group:${derived.fix.responseGroupId}:prompt`,
  ];
  record("derive-scenario-from-real-draft", SCENARIO_STATUS.PASSED, {
    candidate: candidatePath,
    plan: planPath,
    differences: report.scenario.differences,
  });

  // ---- 3. 重启受控服务（这次带样本与剧本）----
  await restartService({ candidate: candidatePath, plan: planPath });
  record("controlled-service-restarted-with-scenario", SCENARIO_STATUS.PASSED, { port: servicePort });

  // ---- 4. 被断言的那一遍：完整导入链 ----
  //
  // 基线（`db-before`）与「运行中进度」必须在**同一个**轮询里取，因为两个时间窗都很窄：
  // 实测云端修复在本地稿出现后约 20 秒就跑完了。上一版先开工作区、再取基线，
  // 拿到的是**已经改完**的稿子（editVersion 2 -> 2），断言于是把「取数晚了」
  // 误报成「云端没改」——那是取数时机的问题，不是产品的问题。这里改成：
  // 工作区先挂上（画布要在云端写入前就存在），随后一个循环同时负责
  // 「初稿一出现立刻取基线」与「从那一刻起持续采样修复进度」。
  itemId = await importThroughUi();
  report.identity.itemId = itemId;

  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-open" });
  await openPanel();

  const repairDeadline = Date.now() + 900000;
  let draft = null;
  let draftWorkspace = null;
  let dbBefore = null;
  let promptBefore = null;
  let versionBefore = null;
  let finalRepair = null;
  let sawRunning = false;
  let baselineTooLate = false;
  while (Date.now() < repairDeadline) {
    // (a) 本地初稿一出现就**立刻**取基线——它必须落在云端写入之前。
    if (!draft) {
      let workspace = null;
      try {
        workspace = await readWorkspace();
      } catch {
        workspace = null; // 任务行还没建好，下一轮再看
      }
      if (workspace?.ds && (workspace.ds.taskGroups ?? []).length > 0) {
        draft = workspace.ds;
        draftWorkspace = workspace;
        dbBefore = dumpDb(path.join(runDir, "db-before.json"), "Before");
        promptBefore = promptTextOf(dbBefore?.item?.canonical, derived.fix.responseGroupId);
        versionBefore = dbBefore?.item?.editVersion ?? null;
        // 基线取晚了的判据：题面已经不含页脚残留，说明云端已经写过了。
        // 记录下来，断言里据此区分「云端没改」与「我们没赶上」。
        baselineTooLate = promptBefore === derived.fix.after;
        report.observed.localDraft = {
          at: new Date().toISOString(),
          editVersion: workspaceVersion(draftWorkspace),
          baselineEditVersion: versionBefore,
          baselineTooLate,
          taskGroups: (draft.taskGroups ?? []).length,
          answerSlots: Object.keys(draft.answerSlots ?? {}).length,
          unresolvedAnswers: Object.values(draft.answerKey ?? {}).filter((value) => value?.kind === "unresolved").length,
          qualityState: draft.quality?.state ?? null,
          blockingIssues: (draft.quality?.issues ?? []).filter((issue) => issue?.severity === "blocking").length,
        };
        record("local-draft-visible-before-cloud", SCENARIO_STATUS.PASSED, report.observed.localDraft);
      }
    }
    // (b) 进度采样：**从本地稿出现那一刻就开始**。等界面开完再采样，
    //     20 秒的循环会在两次轮询之间跑完，「运行中就能看到进度」永远抓不到。
    const decision = await readDecision();
    const repair = decision?.repair ?? null;
    const cloudState = decision?.chains?.cloud?.state ?? null;
    if (repair?.status) {
      const seen = report.observed.repairProgressSeen;
      const last = seen[seen.length - 1];
      if (!last || last.status !== repair.status || last.appliedCount !== (repair.appliedCount ?? null)) {
        seen.push({
          at: new Date().toISOString(),
          status: repair.status,
          appliedCount: repair.appliedCount ?? null,
          editVersion: repair.editVersion ?? null,
          rounds: repair.rounds ?? null,
        });
      }
      if (repair.status === "running") sawRunning = true;
    }
    if (repair && repair.status !== "running" && cloudState && !["queued", "running"].includes(cloudState)) {
      finalRepair = repair;
      break;
    }
    await sleep(600);
  }
  if (!draft) throw new CannotRunError("本地稿在超时前没有落盘");
  if (!finalRepair) throw new CannotRunError("修复循环在超时前没有进入终态");
  report.observed.repairFinal = finalRepair;

  // ---- 6. 数据库 after + 差异 ----
  const dbAfter = dumpDb(path.join(runDir, "db-after.json"), "After");
  const promptAfter = promptTextOf(dbAfter?.item?.canonical, derived.fix.responseGroupId);
  const versionAfter = dbAfter?.item?.editVersion ?? null;
  const changes = diffCanonical(dbBefore?.item?.canonical, dbAfter?.item?.canonical);
  report.observed.canonicalChanges = changes;
  report.observed.editVersion = { before: versionBefore, after: versionAfter };
  report.observed.promptText = { before: promptBefore, after: promptAfter };
  report.modelTraces = { llm: llmTraces(itemId), toolCalls: repairToolCalls(itemId) };

  // ---- 10. 断言：云端**自己**改了什么 ----
  //
  // 前提是基线真的落在云端写入之前。若没赶上（`baselineTooLate`），这份证据无法区分
  // 「云端没改」与「我们没拍到改之前」——那就如实记为无法执行，不能算作失败，
  // 也不能算作通过。
  if (baselineTooLate) {
    notExecutable(
      "cloud-fixed-content-on-its-own",
      "基线取晚了：dump 到的题面已经是修复后的内容，本次无法比较云端写入前后的权威稿",
    );
  } else {
  const problems = [];
  if (!(finalRepair.appliedCount >= 1)) problems.push(`appliedCount 应 >= 1，实际 ${finalRepair.appliedCount}`);
  if (!(versionAfter > versionBefore)) problems.push(`编辑版本应推进，实际 ${versionBefore} -> ${versionAfter}`);
  if (promptBefore === promptAfter) problems.push("题面文字没有被改动");
  if (promptAfter !== derived.fix.after) problems.push(`题面文字应为 ${JSON.stringify(derived.fix.after)}，实际 ${JSON.stringify(promptAfter)}`);
  if (/BLANK PAGE/iu.test(promptAfter ?? "")) problems.push("改后的题面里仍有页脚残留");
  const applied = changes.find((change) => change.kind === "response_group.prompt" && change.targetId === derived.fix.responseGroupId);
  if (!applied) problems.push("权威稿差异里没有这条题面修改");
  if (!(finalRepair.adjudicatedCount >= 1)) problems.push(`adjudicatedCount 应 >= 1，实际 ${finalRepair.adjudicatedCount}`);
  const remainingIds = (finalRepair.remainingTasks ?? []).map((task) => task.userTaskId ?? "");
  if (remainingIds.some((id) => id.includes(`cloud-diff:task_group:${derived.rule.taskId}:instructions`))) {
    problems.push("已裁定的差异仍然出现在用户剩余任务里");
  }
  if (!remainingIds.some((id) => id.startsWith("cloud-question:"))) {
    problems.push("模型明确留下的疑问没有出现在剩余任务里");
  }
  const everyTaskActionable = (finalRepair.remainingTasks ?? []).every(
    (task) => typeof task.action === "string" && task.action.length > 0,
  );
  if (!everyTaskActionable) problems.push("有剩余任务没有可执行的动作");
  if (problems.length === 0) {
    record("cloud-fixed-content-on-its-own", SCENARIO_STATUS.PASSED, {
      appliedCount: finalRepair.appliedCount,
      adjudicatedCount: finalRepair.adjudicatedCount,
      editVersion: report.observed.editVersion,
      promptAfter,
    });
  } else {
    record("cloud-fixed-content-on-its-own", SCENARIO_STATUS.FAILED, { problems });
  }
  }

  // ---- 11. 断言：模型是**照着真实反馈**改的 ----
  const rounds = report.modelTraces.toolCalls;
  const feedbackProblems = [];
  if (rounds.length < 4) feedbackProblems.push(`至少应有 4 轮工具调用，实际 ${rounds.length}`);
  if (rounds[0]?.tool !== "read_draft") feedbackProblems.push(`第 1 轮应为 read_draft，实际 ${rounds[0]?.tool}`);
  if (rounds[1]?.tool !== "apply_edits" || rounds[1]?.baseVersion != null) {
    feedbackProblems.push(`第 2 轮应为不带 baseVersion 的 apply_edits，实际 ${rounds[1]?.tool}/${rounds[1]?.baseVersion}`);
  }
  if (rounds[2]?.tool !== "apply_edits" || rounds[2]?.baseVersion == null) {
    feedbackProblems.push(`第 3 轮应带 baseVersion 重交，实际 ${rounds[2]?.tool}/${rounds[2]?.baseVersion}`);
  }
  const readDraftRound = report.modelTraces.llm.repairRounds[0];
  if (readDraftRound && rounds[2]?.baseVersion !== readDraftRound.editVersion) {
    feedbackProblems.push(`第 3 轮的 baseVersion(${rounds[2]?.baseVersion}) 必须等于第 1 轮真实读到的 editVersion(${readDraftRound.editVersion})`);
  }
  if (rounds[3]?.tool !== "record_ruling") feedbackProblems.push(`第 4 轮应为 record_ruling，实际 ${rounds[3]?.tool}`);
  if ((rounds.at(-1)?.tool ?? null) !== "finish") feedbackProblems.push(`最后一轮应为 finish，实际 ${rounds.at(-1)?.tool}`);
  if ((rounds.at(-1)?.unresolved ?? 0) < 1) feedbackProblems.push("finish 必须留下至少一条未解疑问");
  if (feedbackProblems.length === 0) {
    record("model-corrected-itself-from-real-feedback", SCENARIO_STATUS.PASSED, {
      rounds: rounds.map((round) => round.tool),
      baseVersion: rounds[2]?.baseVersion,
      readDraftEditVersion: readDraftRound?.editVersion ?? null,
    });
  } else {
    record("model-corrected-itself-from-real-feedback", SCENARIO_STATUS.FAILED, { problems: feedbackProblems });
  }

  // ---- 12. 断言：进度在循环结束前就可读（不是十分钟后才出现）----
  if (sawRunning) {
    record("repair-progress-visible-while-running", SCENARIO_STATUS.PASSED, {
      samples: report.observed.repairProgressSeen.length,
      first: report.observed.repairProgressSeen[0],
    });
  } else {
    notExecutable("repair-progress-visible-while-running", "轮询没有抓到 running 状态（循环可能在两次轮询之间就跑完了）");
  }

  // ---- 13. 断言：画布跟着刷新 ----
  //
  // 截图前先把识别面板收起来：面板展开时会盖住画布，上一版两张 PNG 逐字节相同
  // （`canvas-after-cloud-repair.png` 与 `recognition-remaining-tasks.png` 同哈希），
  // 于是「画布更新」根本没有**看得见**的证据。DOM 断言本身是过的，但截图得说真话。
  await closePanel();
  const canvasText = await session.evaluate(
    `(() => { const el = document.querySelector('[data-testid="exam-canvas-v2-author"]'); return el ? el.innerText.replace(/\\s+/g,' ').trim() : null; })()`,
  );
  const canvasProblems = [];
  if (!canvasText) canvasProblems.push("作者画布不在 DOM 里");
  else {
    if (derived.fix.before && canvasText.includes(derived.fix.before)) canvasProblems.push("画布上仍是改前的题面");
    if (/BLANK PAGE/u.test(canvasText)) canvasProblems.push("画布上仍能看到页脚残留");
  }
  if (canvasProblems.length === 0) record("canvas-refreshed-after-cloud-write", SCENARIO_STATUS.PASSED, { sample: (canvasText ?? "").slice(0, 200) });
  else record("canvas-refreshed-after-cloud-write", SCENARIO_STATUS.FAILED, { problems: canvasProblems });
  await session.screenshot("canvas-after-cloud-repair");

  // ---- 14. 剩余任务：界面与后端一致，且每条都有真实动作 ----
  // 第 13 步为了拍到画布把面板收起来了，这里重新展开再读界面。
  await openPanel();
  // 面板重新展开后，修复摘要与剩余任务是**异步**取回来的：挂上元素就立刻读会读到空壳
  // （实测只读到「识别建议 刷新」，于是把「后端有 17 条、界面 0 条」误报成界面缺陷）。
  // 等它真的渲染出结论再读；等不到也照读，让断言如实失败。
  await session
    .waitFor(
      `(() => {
         const root = document.querySelector('[data-testid="workspace-recognition"]');
         if (!root) return false;
         return Boolean(root.querySelector('[data-testid="workspace-recognition-repair-headline"]'))
           || root.querySelectorAll('[data-testid="workspace-recognition-repair-task"]').length > 0;
       })()`,
      { timeoutMs: 25000, intervalMs: 500, label: "recognition-repair-rendered" },
    )
    .catch(() => null);
  const panel = await session.evaluate(
    `(() => {
      const root = document.querySelector('[data-testid="workspace-recognition"]');
      const headline = document.querySelector('[data-testid="workspace-recognition-repair-headline"]');
      const tasks = [...document.querySelectorAll('[data-testid="workspace-recognition-repair-task"]')];
      const legacy = [...document.querySelectorAll('[data-decision-id]')];
      return {
        panelPresent: !!root,
        text: root ? root.innerText.replace(/\\s+/g,' ').trim().slice(0, 1200) : null,
        headline: headline ? headline.innerText.replace(/\\s+/g,' ').trim() : null,
        repairTaskCount: tasks.length,
        legacyCardCount: legacy.length
      };
    })()`,
  );
  report.observed.panel = panel;
  const remaining = finalRepair.remainingTasks ?? [];
  const panelProblems = [];
  if (remaining.length > 0 && panel.repairTaskCount === 0) panelProblems.push("后端有剩余任务，界面上一条都没有");
  if (remaining.length === 0 && panel.repairTaskCount > 0) panelProblems.push("后端没有剩余任务，界面却列了任务");
  if (panel.legacyCardCount > 0) panelProblems.push(`新链路上仍然渲染了 ${panel.legacyCardCount} 张旧建议卡`);
  if (panelProblems.length === 0) {
    record("remaining-tasks-match-backend-and-are-actionable", SCENARIO_STATUS.PASSED, {
      remaining: remaining.length,
      repairTaskCount: panel.repairTaskCount,
      headline: panel.headline,
    });
  } else {
    record("remaining-tasks-match-backend-and-are-actionable", SCENARIO_STATUS.FAILED, { problems: panelProblems, remaining: remaining.length });
  }
  await session.screenshot("recognition-remaining-tasks");

  // ---- 15. 为什么剩下的不能自动处理：逐条给出真实原因 ----
  const blocking = remaining.filter((task) => task.blocking);
  const answersMissing = blocking.filter((task) => String(task.userTaskId).includes("ANSWER_KEY_MISSING_SLOT"));
  report.remainingBreakdown = {
    total: remaining.length,
    blocking: blocking.length,
    answerKeyMissing: answersMissing.length,
    byAction: remaining.reduce((acc, task) => {
      acc[task.action] = (acc[task.action] ?? 0) + 1;
      return acc;
    }, {}),
    items: remaining.map((task) => ({
      userTaskId: task.userTaskId,
      action: task.action,
      blocking: Boolean(task.blocking),
      targetIds: task.targetIds ?? [],
      message: String(task.message ?? "").slice(0, 200),
    })),
  };
  if (answersMissing.length > 0) {
    record("remaining-work-has-a-real-reason", SCENARIO_STATUS.PASSED, {
      reason: "原文件没有答案页；云端不得编造答案，所以这些题只能由用户提供答案",
      answerKeyMissing: answersMissing.length,
      total: remaining.length,
    });
  } else {
    record("remaining-work-has-a-real-reason", SCENARIO_STATUS.PASSED, { note: "本次没有「缺答案」类剩余任务", total: remaining.length });
  }

  // ---- 16. 编辑保存 → 重开 ----
  const editable = (draft.taskGroups ?? []).find((group) => (group.responseGroups ?? []).length > 0);
  const editTarget = editable?.responseGroups?.[0];
  const humanProblems = [];
  let humanSlot = null;
  for (const group of draft.taskGroups ?? []) {
    for (const response of group.responseGroups ?? []) {
      for (const slotId of response.slotIds ?? []) {
        const answer = answerOf(draft, slotId);
        if (answer?.kind === "unresolved") {
          humanSlot = { slotId, interaction: draft.answerSlots?.[slotId]?.interaction ?? "text" };
          break;
        }
      }
      if (humanSlot) break;
    }
    if (humanSlot) break;
  }
  if (humanSlot) {
    const value = humanSlot.interaction === "radio" || humanSlot.interaction === "checkbox"
      ? { kind: "option", labels: ["YES"] }
      : { kind: "text", values: ["controlled-user-answer"] };
    const appliedHuman = await call("apply_editor_commands", {
      itemId,
      commands: [{ op: "setAnswer", slotId: humanSlot.slotId, value }],
      baseVersion: versionAfter,
    });
    if (!appliedHuman?.ok) humanProblems.push(`用户补答案失败：${appliedHuman?.error}`);
    await sleep(1200);
    const afterHuman = await readWorkspace();
    const stored = answerOf(afterHuman?.ds, humanSlot.slotId);
    if (JSON.stringify(stored) !== JSON.stringify(value)) {
      humanProblems.push(`用户补的答案没有落库：${JSON.stringify(stored)}`);
    }
    await reopenWorkspace();
    const reopened = await readWorkspace();
    const afterReopen = answerOf(reopened?.ds, humanSlot.slotId);
    if (JSON.stringify(afterReopen) !== JSON.stringify(value)) {
      humanProblems.push("重开之后用户的答案丢了");
    }
    const reopenedPrompt = promptTextOf(reopened?.ds, derived.fix.responseGroupId);
    if (reopenedPrompt !== derived.fix.after) humanProblems.push("重开之后云端改过的题面丢了");
  } else {
    humanProblems.push("找不到可编辑的空答案槽");
  }
  if (humanProblems.length === 0) {
    record("human-edit-saves-and-survives-reopen", SCENARIO_STATUS.PASSED, { slot: humanSlot?.slotId ?? null });
  } else {
    record("human-edit-saves-and-survives-reopen", SCENARIO_STATUS.FAILED, { problems: humanProblems });
  }

  // ---- 17. 导出：先看真实门禁怎么说，再由**用户**补齐答案 ----
  if (!skipExport) {
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-student-preview"]')`, { timeoutMs: 20000, label: "student-preview" });
    const previewText = await session.evaluate(
      `(() => { const el = document.querySelector('[data-testid="workspace-student-preview"]'); return el ? el.innerText.replace(/\\s+/g,' ').trim().slice(0, 1500) : null; })()`,
    );
    report.observed.studentPreview = previewText;
    await session.screenshot("student-preview");
    await session.clickSelector('[data-testid="workspace-mode-edit"]');
    await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-author"]')`, { timeoutMs: 20000, label: "back-to-author" });

    const preflight = await call("get_publish_preflight", { jobId: itemId });
    report.observed.exportAttempt = { stage: "before-user-answers", preflight: preflight?.value ?? null };

    // 用户把原文件里没有的答案补上——这是**用户的操作**，不是云端的。
    //
    // 选项型答案必须带 `assignment`：`AnswerValueV2::Option` 里它是**必填**字段
    // （`schema/ielts_authoring_v2.rs`），漏掉会让整批命令以
    // `AUTHORING_SCHEMA_INVALID:missing field assignment` 原子失败——上一版正是如此，
    // 于是「用户补答案」这一步其实一次都没成功，却被当成了门禁的问题。
    const workspaceNow = await readWorkspace();
    const currentVersion = workspaceVersion(workspaceNow);
    const commands = [];
    for (const group of workspaceNow?.ds?.taskGroups ?? []) {
      for (const response of group.responseGroups ?? []) {
        for (const slotId of response.slotIds ?? []) {
          const answer = answerOf(workspaceNow?.ds, slotId);
          if (answer?.kind !== "unresolved") continue;
          const interaction = workspaceNow?.ds?.answerSlots?.[slotId]?.interaction ?? "text";
          if (interaction === "radio" || interaction === "checkbox") {
            const bank = group.optionBank?.options ?? response.options ?? [];
            const label = bank[0]?.label ?? "A";
            commands.push({
              op: "setAnswer",
              slotId,
              value: { kind: "option", labels: [label], assignment: response.assignment ?? "per_slot" },
            });
          } else {
            commands.push({ op: "setAnswer", slotId, value: { kind: "text", values: ["answer"] } });
          }
        }
      }
    }
    if (commands.length > 0) {
      const filled = await call("apply_editor_commands", { itemId, commands, baseVersion: currentVersion });
      report.observed.userAnswerFill = {
        commands: commands.length,
        ok: Boolean(filled?.ok),
        error: filled?.error ?? null,
        // 记下每条命令的类型，便于事后核对「选项型有没有带 assignment」。
        kinds: commands.reduce((acc, command) => {
          const kind = command.value.kind;
          acc[kind] = (acc[kind] ?? 0) + 1;
          return acc;
        }, {}),
      };
      await sleep(1500);
      // 补答案这一步必须真的成功，否则后面的发布结论会被一个输入错误污染。
      if (!filled?.ok) {
        throw new CannotRunError(`用户补答案被拒绝：${filled?.error}`);
      }
    }

    // 点发布**之前**先读一次提示文字：发布结果写进的是通用提示元素
    // （`ExamWorkspacePage.tsx` 里 `setNotice(...)` → `.workspace-notice`），
    // 而界面上在点之前可能已经挂着一条**无关**的提示（例如「识别建议已过期」）。
    // 上一版直接读 `.workspace-notice`，把那条无关文案当成了发布结果。
    // 正确做法：等它**变成别的文字**，那才是这次点击产生的结论。
    const noticeBefore = await session.evaluate(
      `(() => { const el = document.querySelector('.workspace-notice'); return el ? el.innerText.replace(/\\s+/g,' ').trim() : null; })()`,
    );
    await session.clickSelector('[data-testid="workspace-publish"]');
    const published = await session.waitFor(
      `(() => {
         const el = document.querySelector('.workspace-notice');
         const text = el ? el.innerText.replace(/\\s+/g,' ').trim() : null;
         return text && text !== ${JSON.stringify(noticeBefore)} ? text : null;
       })()`,
      { timeoutMs: 120000, intervalMs: 1000, label: "publish-result" },
    ).catch(() => null);
    report.observed.publish = { noticeBefore, text: published };
    const afterExport = dumpDb(path.join(runDir, "db-after-export.json"), "AfterExport");
    report.observed.exportAttempt.after = {
      status: afterExport?.item?.status ?? null,
      editVersion: afterExport?.item?.editVersion ?? null,
      qualityState: afterExport?.item?.canonical?.quality?.state ?? null,
      remainingBlocking: (afterExport?.item?.canonical?.quality?.issues ?? []).filter((issue) => issue?.severity === "blocking").length,
    };
    await session.screenshot("after-export-attempt");
    if (afterExport?.item?.status === "published" || (published ?? "").includes("发布完成")) {
      record("export-and-student-runtime", SCENARIO_STATUS.PASSED, report.observed.exportAttempt.after);
    } else {
      // 发布没成。这**不一定是脚本的问题**：门禁按设计拦下不完整的卷子是正确行为。
      // 所以这里把「还剩哪些 blocking 问题」逐条取出来，写进 findings，
      // 让「为什么不能自动处理」有据可查，而不是一句「发布失败」。
      const remainingBlocking = (afterExport?.item?.canonical?.quality?.issues ?? [])
        .filter((issue) => issue?.severity === "blocking")
        .map((issue) => ({ code: issue.code, targetId: issue.targetId ?? null, message: issue.message ?? null }));
      report.observed.exportAttempt.remainingBlocking = remainingBlocking;
      report.findings.push({
        id: "export-blocked-by-quality-gate",
        kind: "gate-blocked",
        title: "用户补完全部答案后，发布仍被质量门禁拦下",
        detail: { notice: published, remainingBlocking, status: afterExport?.item?.status ?? null },
        explanation:
          "门禁拦下不完整的卷子是产品设计行为，不是脚本失败；但它同时说明"
          + "「用户把能做的都做了之后仍然发不出去」。剩余 blocking 问题的代码即原因。",
      });

      // 逐条判定「剩下的这条为什么用户也处理不了」。
      //
      // `WORD_LIMIT_UNPARSED` 的判定条件是
      //   `!selection_type && signature.wordLimit.is_none()`
      // 而 `selection_type` 只看 `instructionSignature.optionAlphabet`，
      // `optionAlphabet` 又只由题干文字经 `infer_option_alphabet` 推出。
      // 那张表里只有 a-d / a-e / a-i / a-g —— **没有 a-h**。
      // 于是「list of words and phrases, A-H」这种标准写法既拿不到 optionAlphabet，
      // 题干里又不可能有 word limit，用户在界面上也没有任何手段能设置这两个字段：
      // 这道题永远发布不出去。这里用**真实稿里的文字**去核对，而不是靠猜。
      for (const issue of remainingBlocking) {
        if (issue.code !== "WORD_LIMIT_UNPARSED") continue;
        const group = (afterExport?.item?.canonical?.taskGroups ?? []).find((candidate) => candidate.taskId === issue.targetId);
        const normalized = String(group?.instructionSignature?.normalizedText ?? "");
        const match = /a\s*[-–]\s*([d-z])/iu.exec(normalized) ?? /a\s+to\s+([d-z])/iu.exec(normalized);
        report.findings.push({
          id: "defect-option-alphabet-missing-range",
          kind: match ? "defect-detected" : "undetermined",
          title: "题干的字母区间不在选项字母表推导表里，导致整卷无法发布",
          detail: {
            groupId: issue.targetId,
            matchedRange: match ? match[0] : null,
            supportedRanges: ["a-d", "a-e", "a-i", "a-g"],
            signatureKeys: Object.keys(group?.instructionSignature ?? {}),
            codeRef: "src-tauri/src/ielts_grammar/instruction_signature.rs:286 infer_option_alphabet",
          },
          explanation: match
            ? `题干写的是「${match[0].trim()}」，` +
              "`infer_option_alphabet` 的区间表（a-d / a-e / a-i / a-g）里没有这一项，"
              + "所以 `optionAlphabet` 为 None → 该题组被判成非选择型 → 因为题干里没有 word limit "
              + "而报 `WORD_LIMIT_UNPARSED`（blocking）。用户界面上无法设置这两个字段，"
              + "所以这份卷子**无论用户怎么操作都发布不出去**。"
            : "未能从题干文字里判定字母区间，本条原因待定。",
        });
      }
      record("export-and-student-runtime", SCENARIO_STATUS.FAILED, {
        note: "补齐答案后仍然没有发布成功",
        notice: published,
        remainingBlocking,
        detail: report.observed.exportAttempt,
      });
    }
  } else {
    notExecutable("export-and-student-runtime", "本次带 --skip-export");
  }

  // ---- 17b. 记录一个真实产品缺陷：对**已有稿子**的条目「重新识别」永远失败 ----
  //
  // 放在链条末尾、且只碰预跑那一条条目，是为了让这条侧探针**不可能**影响被断言的
  // 那条链。它记录的不是脚本行为，而是产品行为：
  //   `retry_processing` → `retry_job` 只做 failed→queued 的队列转换，
  //   工作线程随后调用的本地闭包不带 `allowOverwrite`，而 `run_auto_pipeline_core`
  //   在 `authoring-ir.json` 已存在时以 `editable_draft_exists` 拒绝。
  //   于是用户点「重新识别」只会消耗重试次数，永远拿不到新的识别结果。
  const retryProbe = await (async () => {
    const target = report.identity.prepassItemId;
    if (!target) return { ran: false, reason: "没有预跑条目" };
    try {
      const retry = await call("retry_processing", { itemId: target });
      const immediate = dumpDb(path.join(runDir, "db-retry-probe-immediate.json"), "RetryProbeImmediate");
      const before = (immediate?.processingJobs ?? []).find((job) => job.library_item_id === target) ?? null;
      // 给工作线程一点时间真的去跑，再看它落到哪里——只看 retry 的返回值会误判成成功。
      await sleep(12000);
      const settled = dumpDb(path.join(runDir, "db-retry-probe.json"), "RetryProbe");
      const after = (settled?.processingJobs ?? []).find((job) => job.library_item_id === target) ?? null;
      return {
        ran: true,
        retryAccepted: Boolean(retry?.ok),
        retryError: retry?.error ?? null,
        before,
        after,
        reproduced:
          after?.stage === "failed" &&
          typeof after?.last_error_code === "string" &&
          after.last_error_code.includes("editable_draft_exists"),
      };
    } catch (error) {
      return { ran: true, retryAccepted: false, retryError: String(error?.message ?? error), reproduced: null };
    }
  })();
  report.observed.retryProbe = retryProbe;
  // 这是**发现**，不是验收场景：缺陷复现与否都不该改变「这条产品链有没有跑通」的判定，
  // 所以写进 `findings` 而不进 `scenarios`（进 scenarios 会让一个诊断项决定整体退出码）。
  report.findings.push({
    id: "defect-retry-cannot-rerun",
    kind: retryProbe.reproduced ? "defect-reproduced" : "not-reproduced",
    title: "对已有稿子的条目点「重新识别」不会重新识别",
    detail: retryProbe,
    explanation: retryProbe.reproduced
      ? "`retry_processing` 只把 stage 从 failed 改回 queued；工作线程随后调用的本地闭包不带 "
        + "`allowOverwrite`，而 `run_auto_pipeline_core` 在 `authoring-ir.json` 已存在时以 "
        + "`editable_draft_exists` 拒绝。用户因此只会消耗重试次数，拿不到新的识别结果。"
      : "本次未复现 `editable_draft_exists`，重试路径的实际行为需重新判定。",
  });
  console.log(`[cloud-repair-chain] finding defect-retry-cannot-rerun: ${report.findings.at(-1).kind}`);

  // ---- 18. 报告 ----
  report.finishedAt = new Date().toISOString();
  report.modelTracesAfter = { toolCalls: repairToolCalls(itemId), llm: llmTraces(itemId) };
  fs.writeFileSync(path.join(runDir, "controlled-service.log"), report.service.log?.() ?? "");
  report.service.log = undefined;
  const verdict = computeScenarioVerdict({ scenarios: report.scenarios });
  report.verdict = verdict.verdict;
  report.exitCode = verdict.exitCode;
  report.scenarioFacts = {
    passed: verdict.passed,
    failed: verdict.failed,
    notExecutable: verdict.notExecutable,
    unknown: verdict.unknown,
    total: verdict.total,
    reason: verdict.reason,
  };
  writeReport(runDir, report);
  console.log(`[cloud-repair-chain] scenarios: ${report.scenarios.map((s) => `${s.name}:${s.status}`).join(" | ")}`);
  console.log(`[cloud-repair-chain] verdict: ${verdict.verdict} (${verdict.reason})`);
  process.exitCode = verdict.exitCode;
}

/** `get_workspace_item` 顶层就带 `editVersion`（`library_items_v2.current_edit_version`）。 */
function workspaceVersion(workspace) {
  return workspace?.editVersion ?? workspace?.item?.editVersion ?? null;
}

main()
  .catch(async (error) => {
    const cannotRun = error instanceof CannotRunError;
    record("harness", cannotRun ? SCENARIO_STATUS.NOT_EXECUTABLE : SCENARIO_STATUS.FAILED, { message: String(error?.message ?? error) }, error);
    report.finishedAt = new Date().toISOString();
    try {
      report.appOutput = session?.appOutput?.() ?? null;
    } catch {
      // 忽略
    }
    try {
      fs.mkdirSync(runDir, { recursive: true });
      writeReport(runDir, report);
    } catch {
      // 忽略
    }
    process.exitCode = cannotRun ? 3 : 1;
  })
  .finally(async () => {
    try {
      if (serviceChild) serviceChild.kill();
    } catch {
      // 忽略
    }
    try {
      await session?.close({ keep });
    } catch {
      // 忽略
    }
  });
