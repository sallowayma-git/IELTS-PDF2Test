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
import { createHash } from "node:crypto";
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
import { deriveRepairScenario, loadRepairGolden, textOfNodes } from "./lib/cloud-repair-scenario.mjs";
import { loadPublishedPackageWithRealProviderAsync } from "./lib/student-real-provider.mjs";

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
  scenario: { derived: false, differences: [], fix: null, rule: null, unresolved: [], golden: null },
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

/**
 * **断言失败**（产品缺陷 / 证据不成立），与环境不满足（`CannotRunError`）区分开。
 *
 * 为什么要单独一个类型：链条里有些失败是**核心功能失败**（本地稿不落盘、修复循环
 * 不进入终态、用户补答案被拒），它们以前一律 `throw new CannotRunError`，被降级成
 * 「环境不满足」、退出码 3。那等于说「这次不算数」——而修复循环超时恰恰是本轮要验的
 * 核心功能。这里让它们走 FAILED（退出码 1）。
 */
class ChainFailure extends Error {
  constructor(message, detail) {
    super(message);
    this.name = "ChainFailure";
    this.detail = detail ?? null;
  }
}

/**
 * 报告收尾：把场景列表算成 verdict、写盘、设退出码。
 *
 * 抽出来是因为有三个出口（正常结束、前提不成立提前结束、核心失败提前结束），
 * 而它们以前各写一份——最后那个 `catch` 分支**根本没算 verdict**，于是报告里的
 * `verdict` 字段与 `process.exitCode` 是两套口径（报告说 failed、退出码说 3）。
 * 只留一份实现，这种分叉就不可能再出现。
 */
function writeFinalReport() {
  report.finishedAt = new Date().toISOString();
  try {
    fs.writeFileSync(path.join(runDir, "controlled-service.log"), report.service.log?.() ?? "");
    report.service.log = undefined;
  } catch {
    // 服务日志写不出来不影响判定
  }
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
  return verdict;
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
          // 引文：本轮的验收要拿它去原文里对。**不记下来就无法证伪**——
          // 「模型引用了原文」这句话必须能落到具体字符串上。
          evidence: [
            ...(call?.arguments?.evidence ?? []),
            ...(call?.arguments?.rulings ?? []).flatMap((ruling) => ruling?.evidence ?? []),
          ].map((item) => ({ pageIndex: item?.pageIndex ?? null, quote: item?.quote ?? null })),
        };
      } catch {
        return { stamp: entry.stamp, error: "unparsable" };
      }
    });
}

/**
 * 取一份权威稿快照。
 *
 * `required`（默认）为真时，取不到就**让场景失败**。这不是洁癖：旧写法失败返回 `null`，
 * 于是下游的 `versionAfter > versionBefore` 变成 `2 > null` → `2 > 0` → **恒真**，
 * 「dumpDb 失败」反而把断言染绿。诊断性探针（重试缺陷复现）显式传
 * `{ required: false }`，因为它失败与否不该改变链条判定。
 */
function dumpDb(outPath, label, { required = true } = {}) {
  const result = spawnSync(PYTHON, [path.join(repoRoot, "scripts", "e2e", "lib", "dump-authoring-db.py"), dbPath, outPath, itemId ?? ""], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  report[`db${label}`] = { path: outPath, status: result.status, stdout: String(result.stdout ?? "").trim(), stderr: String(result.stderr ?? "").trim() };
  if (result.status !== 0) {
    if (required) {
      throw new ChainFailure(
        `取权威稿快照失败（${label}）：dump-authoring-db.py 退出码 ${result.status}`,
        report[`db${label}`],
      );
    }
    return null;
  }
  try {
    return JSON.parse(fs.readFileSync(outPath, "utf8"));
  } catch (error) {
    if (required) throw new ChainFailure(`权威稿快照不是合法 JSON（${label}）：${error.message}`);
    return null;
  }
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

/** 文件 sha256。golden fixture 绑定的原文件必须与本次输入是同一份，否则标注不成立。 */
function sha256OfFile(filePath) {
  try {
    return createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
  } catch {
    return null;
  }
}

/**
 * 读作业目录里**解析器层**抽取出来的逐页原文文本。
 *
 * 与后端 `source_page_texts` 读同一批产物、同样的页号归一（DocumentIR 是 0-based，
 * 归一成 1-based）。这里是**独立**读一遍：模型说它引用了原文，验收侧就自己去看
 * 原文里到底有没有这句话——两边都读同一份 artifact，但走的是两条代码路径。
 */
function sourcePageTextsFromJob(jobId) {
  const dir = path.join(appDataDir, "jobs", String(jobId ?? ""));
  const out = new Map();
  const readPage = (page, pick) => {
    const index = Number(page?.pageIndex);
    if (!Number.isInteger(index) || index < 0) return;
    const text = pick(page);
    if (typeof text === "string" && text.trim()) out.set(index + 1, text.trim());
  };
  const documentIr = path.join(dir, "document-ir.json");
  if (fs.existsSync(documentIr)) {
    const parsed = JSON.parse(fs.readFileSync(documentIr, "utf8"));
    for (const page of parsed?.pages ?? []) {
      readPage(page, (entry) => {
        if (Array.isArray(entry?.lines)) {
          return entry.lines.map((line) => line?.text ?? "").filter((line) => line.trim()).join("\n");
        }
        if (Array.isArray(entry?.spans)) return entry.spans.map((span) => span?.text ?? "").join("");
        return "";
      });
    }
  }
  if (out.size === 0) {
    const compare = path.join(dir, "document-ir-v2.shadow.compare.json");
    if (fs.existsSync(compare)) {
      const parsed = JSON.parse(fs.readFileSync(compare, "utf8"));
      for (const page of parsed?.pages ?? []) {
        readPage(page, (entry) => entry?.v1Text ?? entry?.v2Text ?? "");
      }
    }
  }
  return out;
}

/**
 * 本地稿的**真实形状**：题组/题面数量、空题面与占位题面各多少、blocking 代码。
 *
 * 单独抽出来是因为它要在**多条**退出路径上写进报告。以前它只写在「派生失败」那一个
 * 分支里，于是 DOCX 那一遍（golden 哈希对不上，更早退出）的报告里看不到稿子形状，
 * 读者只看到「哈希不一致」，很容易误以为是脚本配置问题——而真正的原因是本地识别
 * 一个题面都没产出。诊断信息只挂在一条路径上，就等于其他路径上没有诊断。
 */
function draftShapeOf(ds) {
  const groups = ds?.taskGroups ?? [];
  const placeholder = /^\s*\[[^\]]*pending review[^\]]*\]\s*$/iu;
  const promptTexts = [];
  for (const group of groups) {
    for (const response of group.responseGroups ?? []) {
      promptTexts.push(textOfNodes(response.prompt ?? []));
    }
  }
  return {
    taskGroups: groups.length,
    responseGroups: promptTexts.length,
    placeholderPrompts: promptTexts.filter((text) => placeholder.test(text)).length,
    emptyPrompts: promptTexts.filter((text) => !text).length,
    qualityState: ds?.quality?.state ?? null,
    blockingCodes: [
      ...new Set((ds?.quality?.issues ?? []).filter((issue) => issue?.severity === "blocking").map((issue) => issue.code)),
    ],
    samplePrompt: promptTexts[0] ?? null,
  };
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
  if (!prepassDraft) {
    // 本地稿不落盘是**产品失败**（本地识别是这条链的第一环），不是「环境不满足」。
    // 以前这里 throw CannotRunError，退出码 3，报告读起来像「这次没跑成」。
    record("prepass-import-for-scenario-derivation", SCENARIO_STATUS.FAILED, {
      problems: ["预跑一遍的本地稿在 180 秒内没有落盘"],
    });
    writeFinalReport();
    return;
  }
  const prepassGroups = prepassDraft.ds.taskGroups ?? [];
  const prepassSlots = Object.keys(prepassDraft.ds.answerSlots ?? {}).length;
  // 这一条以前只是「记了个事实」：把稿子形状写进报告，两个分支都是 PASSED。
  // 它的名字声明的是一个**前提**，那就必须有前提的断言：本地识别至少要产出
  // 一个题组和一个答案槽，否则「从真实稿派生场景」这件事根本无从谈起。
  const prepassProblems = [];
  if (prepassGroups.length === 0) prepassProblems.push("本地稿没有任何题组");
  if (prepassSlots === 0) prepassProblems.push("本地稿没有任何答案槽");
  if (prepassProblems.length === 0) {
    record("prepass-import-for-scenario-derivation", SCENARIO_STATUS.PASSED, {
      itemId,
      taskGroups: prepassGroups.length,
      answerSlots: prepassSlots,
      qualityState: prepassDraft.ds.quality?.state ?? null,
    });
  } else {
    record("prepass-import-for-scenario-derivation", SCENARIO_STATUS.FAILED, {
      problems: prepassProblems,
      itemId,
      qualityState: prepassDraft.ds.quality?.state ?? null,
    });
    writeFinalReport();
    return;
  }

  // 稿子的真实形状**无条件**写进报告：它是后面多条退出路径共用的诊断依据。
  report.observed.prepassDraftShape = {
    ...draftShapeOf(prepassDraft.ds),
    // 说明这份形状是从**哪一层**读到的。实测 DOCX 这一遍：产品读取路径
    // （`get_workspace_item`）读到的是 `[prompt pending review]` 占位符，而库里
    // canonical 的同一位置存的是**空串**——两者都表示「题面没有内容」，结论不变，
    // 但读者必须知道读的是哪一层，否则会把「占位符 13」和「空串 13」当成两个事实。
    readPath: "get_workspace_item.ds",
  };

  // ---- 2. 期望值来自人工标注的 golden fixture；真实稿只用来**核对** ----
  //
  // 这一步以前是「脚本从本地稿派生期望值」：脚本自己剥掉页脚残留，把结果同时当作
  // 候选内容、剧本里的 `fixedPromptText`、以及断言时比的字符串。三者是同一个值，
  // 于是「云端能依据原文件修正识别错误」无法被证伪——脚本把答案递给假模型，假模型照抄。
  const golden = loadRepairGolden(repoRoot);
  report.scenario.golden = {
    path: path.relative(repoRoot, golden.path),
    fixtureId: golden.fixtureId,
    sourceSha256: golden.source.sha256,
    errorId: golden.recognitionErrors[0].id,
    errorClass: golden.recognitionErrors[0].errorClass,
  };
  // fixture 绑定的是**具体一份原文件**：哈希对不上就说明标注与文件不是同一份，
  // 这时任何结论都不成立。
  const actualFixtureSha = sha256OfFile(fixturePath);
  if (actualFixtureSha !== golden.source.sha256) {
    const shape = report.observed.prepassDraftShape;
    const problems = [
      `golden fixture 绑定的原文件哈希与本次输入不一致：fixture=${golden.source.sha256} 实际=${actualFixtureSha}`,
    ];
    // 哈希对不上只是「这份输入没有标注」。若这份输入的初稿本身也不可用，必须一并说出来：
    // 否则读者会以为「补一份标注就能验收」，而实际上补了也跑不动。
    if (shape.placeholderPrompts > 0 || shape.emptyPrompts > 0) {
      problems.push(
        `而且这份输入的本地初稿本身不可用：${shape.responseGroups} 个题面里 ${shape.placeholderPrompts} 个是占位符、`
          + `${shape.emptyPrompts} 个是空的（blocking ${JSON.stringify(shape.blockingCodes)}）。`
          + "这是导入阶段的产品退化，先把初稿修好，才谈得上这份输入能不能验收。",
      );
    }
    record("derive-scenario-from-real-draft", SCENARIO_STATUS.FAILED, {
      problems,
      fixture: fixturePath,
      prepassDraftShape: shape,
    });
    writeFinalReport();
    return;
  }

  const derived = deriveRepairScenario(prepassDraft.ds, golden);
  if (!derived?.ok) {
    // 前提不成立时，必须把**这份稿子的真实形状**写进报告：只说「没有页脚残留」
    // 会让读者以为是脚本挑剔，而实测 DOCX 那一遍是 **13 个题面全是空的**——
    // 那是本地识别本身的问题，不是脚本的前提问题。
    // （形状在第 1 步之后就已无条件算好，这里直接取用，不再重算一份。）
    const shape = report.observed.prepassDraftShape;
    const placeholderCount = shape.placeholderPrompts;
    const emptyCount = shape.emptyPrompts;
    report.observed.goldenMismatch = {
      reason: derived?.reason ?? "派生函数没有给出理由",
      observed: derived?.observed ?? null,
      expected: derived?.expected ?? null,
      target: derived?.target ?? null,
    };
    // 分类必须诚实，这是本轮改掉的一处「用 not-executable 掩盖产品退化」：
    //   · 题面是**占位符 / 空**（本地识别产出的题面是退化的）→ FAILED。
    //     那是产品缺陷，不是「本次没法验」。以前记 not-executable（退出码 5），
    //     报告读起来像环境问题，真正的退化被藏起来了。
    //   · 题面都是真实文字、只是**没有出现 golden 标注的那个错误** → 这份样本确实
    //     不覆盖本场景 → 这才是 not-executable（前提不成立）。
    //     注意：这时**不能**自己造一个错误出来。错误必须由本地识别真实产生；
    //     造一个错再「修好」，证明的只是脚本会写字。
    if (placeholderCount > 0 || emptyCount > 0) {
      record("derive-scenario-from-real-draft", SCENARIO_STATUS.FAILED, {
        problems: [
          `本地识别产出的题面是退化的：${shape.responseGroups} 个题面里 ${placeholderCount} 个是占位符、`
            + `${emptyCount} 个是空的（例如 ${JSON.stringify(shape.samplePrompt)}）`,
          `blocking 代码 ${JSON.stringify(shape.blockingCodes)}`,
        ],
        prepassDraftShape: report.observed.prepassDraftShape,
      });
    } else {
      notExecutable(
        "derive-scenario-from-real-draft",
        `本地识别没有产出 golden fixture 标注的那个错误，场景前提不成立：${derived?.reason ?? ""}`
          + `（标注期望 ${JSON.stringify(derived?.expected ?? null)}，实测 ${JSON.stringify(derived?.observed ?? null)}）`,
      );
    }
    writeFinalReport();
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
  // 这一条以前只记「派生成功了、文件写哪儿了」。它真正的断言是：派生出来的修复
  // **必须是一处真实的内容差异**（改前 ≠ 改后），否则后面的「云端改对了」就没有靶子。
  const deriveProblems = [];
  if (!derived.fix?.responseGroupId) deriveProblems.push("派生结果里没有作答组 id");
  if (!derived.rule?.taskId) deriveProblems.push("派生结果里没有题组 id");
  if (typeof derived.fix?.before !== "string" || derived.fix.before === derived.fix.after) {
    deriveProblems.push(`派生出的题面修改不是一处真实差异：before=${JSON.stringify(derived.fix?.before)} after=${JSON.stringify(derived.fix?.after)}`);
  }
  // 剧本里**不能**出现期望值：一旦出现，受控服务就不必读原文件，链条立刻退回自证。
  // 这条断言是「拆掉自证结构」最直接的守卫——它防的是以后有人图省事把答案塞回剧本。
  if (JSON.stringify(derived.plan).includes(derived.golden.originalFileSays)) {
    deriveProblems.push("剧本里出现了期望值（正确题面）：受控服务就不再需要读原文件，链条退回自证");
  }
  if (deriveProblems.length === 0) {
    record("derive-scenario-from-real-draft", SCENARIO_STATUS.PASSED, {
      candidate: candidatePath,
      plan: planPath,
      differences: report.scenario.differences,
      fix: { responseGroupId: derived.fix.responseGroupId, before: derived.fix.before, after: derived.fix.after },
      // 期望值的来源写清楚：断言时比的字符串来自 fixture，而不是脚本自己算的。
      golden: derived.golden,
      // 本地识别**真实产生**了这个错误（不是脚本注入的）。
      localErrorIsReal: derived.fix.before === derived.golden.localDraftContains,
      planCarriesNoExpectedText: !JSON.stringify(derived.plan).includes(derived.golden.originalFileSays),
    });
  } else {
    record("derive-scenario-from-real-draft", SCENARIO_STATUS.FAILED, { problems: deriveProblems });
    writeFinalReport();
    return;
  }

  // ---- 3. 重启受控服务（这次带样本与剧本）----
  await restartService({ candidate: candidatePath, plan: planPath });
  // 「重启成功」的实质断言：服务真的活着，**而且**样本与剧本真的被它读进去了。
  // 以前只记了端口号，两个分支都 PASSED——重启失败也照样绿。
  const health1 = await waitForService();
  const scenarioProblems = [];
  if (!health1) scenarioProblems.push("重启后的受控服务 /health 不可达");
  // `/health` 回的是 `{ candidate, plan }` —— 载入的样本 / 剧本路径（未载入为 null）。
  if (health1 && !health1.candidate) scenarioProblems.push("受控服务没有载入候选样本");
  if (health1 && !health1.plan) scenarioProblems.push("受控服务没有载入修复剧本");
  if (scenarioProblems.length === 0) {
    record("controlled-service-restarted-with-scenario", SCENARIO_STATUS.PASSED, {
      port: servicePort,
      mode: health1.mode,
      candidate: health1.candidate,
      plan: health1.plan,
    });
  } else {
    record("controlled-service-restarted-with-scenario", SCENARIO_STATUS.FAILED, { problems: scenarioProblems, health: health1 });
    writeFinalReport();
    return;
  }

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
        // 这一条以前只是「记了个事实」（两个分支都 PASSED）。它的名字声明的是一个
        // **时序主张**：本地稿在云端写入之前就可见。那就必须有真的断言。
        const localProblems = [];
        if (versionBefore == null) localProblems.push("基线快照里读不到 editVersion");
        if (baselineTooLate) {
          localProblems.push("基线取晚了：dump 到的题面已经是修复后的内容，无法证明本地稿先于云端写入可见");
        } else if (derived.fix.before != null && promptBefore !== derived.fix.before) {
          localProblems.push(
            `基线题面既不是改前的 ${JSON.stringify(derived.fix.before)}，也不是改后的 ${JSON.stringify(derived.fix.after)}：${JSON.stringify(promptBefore)}`,
          );
        }
        if (localProblems.length === 0) {
          record("local-draft-visible-before-cloud", SCENARIO_STATUS.PASSED, report.observed.localDraft);
        } else {
          record("local-draft-visible-before-cloud", SCENARIO_STATUS.FAILED, {
            problems: localProblems,
            ...report.observed.localDraft,
          });
        }
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
  if (!draft) {
    // 本地稿不落盘 = 产品失败（本地识别是这条链的第一环）。以前这里 throw
    // CannotRunError，退出码 3，报告读起来像「环境不满足」。
    record("local-draft-visible-before-cloud", SCENARIO_STATUS.FAILED, {
      problems: ["900 秒内没有读到本地初稿（taskGroups > 0）"],
    });
    record("cloud-fixed-content-on-its-own", SCENARIO_STATUS.FAILED, {
      problems: ["没有本地稿，无法比较云端写入前后的权威稿"],
    });
    writeFinalReport();
    return;
  }
  if (!finalRepair) {
    // 修复循环在超时前没有进入终态 = **核心功能失败**，不是「环境不满足」。
    // 修复循环进入终态正是本轮要验的东西，把它降级成退出码 3 等于说「这次不算数」。
    record("cloud-fixed-content-on-its-own", SCENARIO_STATUS.FAILED, {
      problems: ["修复循环在 900 秒内没有进入终态（repair.status 一直是 running 或从未出现）"],
      repairProgressSeen: report.observed.repairProgressSeen,
    });
    writeFinalReport();
    return;
  }
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
  // 版本必须**两个都读到了**才谈得上「推进」。显式拒绝 null：`2 > null` 在 JS 里是 true
  // （null 被转成 0），把「快照缺失」读成「版本推进了」。
  if (versionBefore == null || versionAfter == null) {
    problems.push(`编辑版本读不到（before=${versionBefore} after=${versionAfter}），无法判定是否推进`);
  } else if (!(versionAfter > versionBefore)) {
    problems.push(`编辑版本应推进，实际 ${versionBefore} -> ${versionAfter}`);
  }
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

  // ---- 10b. 断言：这次「云端改对了」不是自证 ----
  //
  // 这是本轮拆掉自证结构之后新增的一条，也是整条链最要紧的一条。
  //
  // 旧链条：脚本派生差异 → 写进剧本 → 假模型照剧本改 → 断言等于剧本里的字符串。
  // 三者是同一个值，于是「云端能依据原文件修正识别错误」**无法被证伪**；剧本里一次
  // `read_source` 都没有，而网关 prompt 却写着 "so it matches the ORIGINAL FILE"。
  //
  // 现在要证明的是：受控服务的**修改内容与证据引文**都只能从 `read_source` 的返回里
  // 得到（剧本里没有这些值），而且引文能在原文里逐字找到。
  {
    const annotated = golden.recognitionErrors[0];
    const toolCalls = report.modelTraces.toolCalls ?? [];
    const sourceRounds = toolCalls.filter((call) => call.tool === "read_source");
    const quotes = toolCalls.flatMap((call) => call.evidence ?? []);
    const pageTexts = sourcePageTextsFromJob(itemId);
    const expectedPageText = pageTexts.get(Number(annotated.sourcePage.oneBased)) ?? null;
    const problems = [];

    // (0) 回合本身必须存在：没有 read_source，后面两条都无从谈起。
    if (sourceRounds.length === 0) {
      problems.push("整条修复回合里没有一次 read_source：改对了也只是照剧本抄的，证明不了「依据原文件」");
    }
    if (!expectedPageText) {
      problems.push(`作业目录里读不到第 ${annotated.sourcePage.oneBased} 页的原文文本，无法核对引文`);
    }
    // (1) 期望值来自人工标注的 fixture —— 与脚本派生彻底脱钩。
    if (promptAfter !== annotated.originalFileSays) {
      problems.push(
        `改后题面与 golden 标注的原文件真值不一致：期望 ${JSON.stringify(annotated.originalFileSays)}，实际 ${JSON.stringify(promptAfter)}`,
      );
    }
    // (2) fixture 的标注必须与**真实原文件**对得上：原文里那一行确实是「题号 + 真值」。
    //     对不上说明标注写错了（或文件换了），这时上面的比较没有意义。
    const expectedLine = `${annotated.target.questionNumber} ${annotated.originalFileSays}`;
    if (expectedPageText && !expectedPageText.includes(expectedLine)) {
      problems.push(`原文第 ${annotated.sourcePage.oneBased} 页里找不到「${expectedLine}」，golden 标注与真实原文件不一致`);
    }
    if (expectedPageText && !expectedPageText.includes(annotated.originalFileQuote)) {
      problems.push(`golden 自带的引文在原文里找不到：${JSON.stringify(annotated.originalFileQuote)}`);
    }
    // (3) 引文必须能被证伪：模型给出的每一条引文都要在原文里逐字找到。
    if (quotes.length === 0) {
      problems.push("模型一条引文都没给：修改没有任何出处");
    }
    for (const quote of quotes) {
      const text = String(quote?.quote ?? "").trim();
      if (!text) {
        problems.push("模型给出了一条空引文");
        continue;
      }
      // 引文要落在**它自己声明的那一页**上，而不是脚本挑定的那一页。
      // 旧写法把每条引文都拿去和第 4 页比：模型一条来自第 3 页（说明文字）的合法引文
      // 会被判成「找不到」，反过来把第 3 页的引文谎报成第 4 页也能过。按声明页码逐条
      // 核对，两个方向都堵住。
      const pageIndex = Number(quote?.pageIndex);
      const pageText = Number.isInteger(pageIndex) ? pageTexts.get(pageIndex) ?? null : null;
      if (!pageText) {
        problems.push(`模型引文声明了第 ${quote?.pageIndex} 页，但作业目录里读不到该页原文`);
        continue;
      }
      if (!pageText.includes(text)) {
        problems.push(`模型的引文在第 ${pageIndex} 页原文里找不到：${JSON.stringify(text)}`);
      }
    }
    // 反向对照：检查本身必须能说「不」。若连一句显然不存在的话都能在原文里"找到"，
    // 上面那圈检查就是恒真，等于没查。
    const fabricated = "this sentence was never printed in the original file";
    for (const [pageIndex, pageText] of pageTexts) {
      if (pageText.includes(fabricated)) {
        problems.push(`反向对照失效：虚构引文竟然能在第 ${pageIndex} 页原文里找到，说明引文检查是恒真的`);
      }
    }

    report.observed.sourceGroundedCorrection = {
      readSourceRounds: sourceRounds.length,
      quotes: quotes.map((quote) => ({ pageIndex: quote?.pageIndex ?? null, quote: quote.quote })),
      sourcePageOneBased: annotated.sourcePage.oneBased,
      sourcePageTextLength: expectedPageText?.length ?? 0,
      pagesAvailable: [...pageTexts.keys()].sort((a, b) => a - b),
      expectedPrompt: annotated.originalFileSays,
      actualPrompt: promptAfter,
    };
    if (problems.length === 0) {
      record("correction-and-evidence-come-from-the-original-file", SCENARIO_STATUS.PASSED, {
        readSourceRounds: sourceRounds.length,
        quotes: report.observed.sourceGroundedCorrection.quotes,
        promptAfter,
      });
    } else {
      record("correction-and-evidence-come-from-the-original-file", SCENARIO_STATUS.FAILED, {
        problems,
        ...report.observed.sourceGroundedCorrection,
      });
    }
  }

  // ---- 11. 断言：模型是**照着真实反馈**改的 ----
  const rounds = report.modelTraces.toolCalls;
  const feedbackProblems = [];
  if (rounds.length < 5) feedbackProblems.push(`至少应有 5 轮工具调用，实际 ${rounds.length}`);
  if (rounds[0]?.tool !== "read_draft") feedbackProblems.push(`第 1 轮应为 read_draft，实际 ${rounds[0]?.tool}`);
  // 第 2 轮必须是 read_source：少了它，「照真实反馈改」就退化成照剧本改。
  // （read_source 是后加的，下面所有轮次序号都跟着后移一位。）
  if (rounds[1]?.tool !== "read_source") feedbackProblems.push(`第 2 轮应为 read_source，实际 ${rounds[1]?.tool}`);
  if (rounds[2]?.tool !== "apply_edits" || rounds[2]?.baseVersion != null) {
    feedbackProblems.push(`第 3 轮应为不带 baseVersion 的 apply_edits，实际 ${rounds[2]?.tool}/${rounds[2]?.baseVersion}`);
  }
  if (rounds[3]?.tool !== "apply_edits" || rounds[3]?.baseVersion == null) {
    feedbackProblems.push(`第 4 轮应带 baseVersion 重交，实际 ${rounds[3]?.tool}/${rounds[3]?.baseVersion}`);
  }
  const readDraftRound = report.modelTraces.llm.repairRounds[0];
  if (readDraftRound && rounds[3]?.baseVersion !== readDraftRound.editVersion) {
    feedbackProblems.push(`第 4 轮的 baseVersion(${rounds[3]?.baseVersion}) 必须等于第 1 轮真实读到的 editVersion(${readDraftRound.editVersion})`);
  }
  if (rounds[4]?.tool !== "record_ruling") feedbackProblems.push(`第 5 轮应为 record_ruling，实际 ${rounds[4]?.tool}`);
  if ((rounds.at(-1)?.tool ?? null) !== "finish") feedbackProblems.push(`最后一轮应为 finish，实际 ${rounds.at(-1)?.tool}`);
  if ((rounds.at(-1)?.unresolved ?? 0) < 1) feedbackProblems.push("finish 必须留下至少一条未解疑问");
  if (feedbackProblems.length === 0) {
    record("model-corrected-itself-from-real-feedback", SCENARIO_STATUS.PASSED, {
      rounds: rounds.map((round) => round.tool),
      baseVersion: rounds[3]?.baseVersion,
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
  // 2026-09-21 起界面只有**一份**编辑辅助清单（工作区「待补充」列表，`[data-task-id]`）：
  // 本地检查、发布前检查与云端修复剩下的任务合并成一份，每个题位只出一条。修复面板收成
  // 折叠的「详情」，不再单独列任务。所以这里不再数面板里的旧任务行，而是核对不变量：
  //   - 后端每条剩余任务的目标，都被清单里某一条接住（按 data-action-target / data-task-id）；
  //   - 后端有剩余任务时清单不能为空；
  //   - 新链路上不得渲染旧建议卡（`[data-decision-id]`）。
  await session
    .waitFor(
      `(() => Boolean(document.querySelector('[data-testid="workspace-tasks-headline"], [data-testid="workspace-tasks-clear"]')))()`,
      { timeoutMs: 25000, intervalMs: 500, label: "editing-aid-list-rendered" },
    )
    .catch(() => null);
  // 列表折叠时展开，保证读到全部条目。
  await session.evaluate(`(() => { const more = document.querySelector('[data-testid="workspace-tasks-more"]'); if (more) more.click(); return true; })()`);
  const panel = await session.evaluate(
    `(() => {
      const entries = [...document.querySelectorAll('[data-task-id]')].map((el) => ({
        taskId: el.getAttribute('data-task-id'),
        kind: el.getAttribute('data-task-kind'),
        targets: [...el.querySelectorAll('[data-action-target]')].map((b) => b.getAttribute('data-action-target')),
        text: el.innerText.replace(/\s+/g,' ').trim().slice(0, 160)
      }));
      const legacy = [...document.querySelectorAll('[data-decision-id]')];
      const clear = document.querySelector('[data-testid="workspace-tasks-clear"]');
      return {
        entryCount: entries.length,
        entries,
        clearText: clear ? clear.innerText.replace(/\s+/g,' ').trim() : null,
        legacyCardCount: legacy.length
      };
    })()`,
  );
  report.observed.panel = panel;
  const remaining = finalRepair.remainingTasks ?? [];
  const covered = new Set();
  for (const entry of panel.entries ?? []) {
    for (const target of entry.targets ?? []) if (target) covered.add(target);
    for (const part of String(entry.taskId ?? "").split(/[:+]/)) if (part) covered.add(part);
  }
  const uncovered = remaining.filter((task) => {
    const targets = (task.targetIds ?? []).filter(Boolean);
    if (targets.length === 0) return (panel.entryCount ?? 0) === 0;
    return !targets.some((id) => covered.has(id) || covered.has(String(id).replace(/^answerKey:/, "")));
  });
  const panelProblems = [];
  if (remaining.length > 0 && (panel.entryCount ?? 0) === 0) panelProblems.push("后端有剩余任务，清单里一条都没有");
  if (uncovered.length > 0) panelProblems.push(`${uncovered.length} 条后端剩余任务没有被清单接住：${uncovered.map((task) => task.userTaskId).slice(0, 5).join(", ")}`);
  if (panel.legacyCardCount > 0) panelProblems.push(`新链路上仍然渲染了 ${panel.legacyCardCount} 张旧建议卡`);
  if (panelProblems.length === 0) {
    record("remaining-tasks-match-backend-and-are-actionable", SCENARIO_STATUS.PASSED, {
      remaining: remaining.length,
      entryCount: panel.entryCount,
      clearText: panel.clearText,
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
  // 这一条以前两个分支都 PASSED，等于什么都没断言。它的名字声明的是「剩下的每一件
  // 事都有一个**真实**原因」，那就必须逐条去核那个原因是不是真的：
  //   · 每条阻断任务都要有非空 action（零动作的阻断任务用户没法处理）；
  //   · 每条「缺答案」都要对应一个**当前稿里确实是 unresolved** 的答案槽
  //     （证明原因不是标签造出来的，而是稿子的真实状态）。
  const reasonProblems = [];
  const unactionable = blocking.filter((task) => !(typeof task.action === "string" && task.action.length > 0));
  if (unactionable.length > 0) {
    reasonProblems.push(`${unactionable.length} 条阻断任务没有可执行动作：${JSON.stringify(unactionable.map((t) => t.userTaskId))}`);
  }
  const canonicalAfter = dbAfter?.item?.canonical ?? null;
  const unexplainedMissing = answersMissing.filter((task) => {
    const slotId = (task.targetIds ?? [])[0] ?? null;
    const answer = slotId ? answerOf(canonicalAfter, slotId) : null;
    return answer?.kind !== "unresolved";
  });
  if (unexplainedMissing.length > 0) {
    reasonProblems.push(
      `${unexplainedMissing.length} 条「缺答案」任务对应的槽在权威稿里并不是 unresolved：`
        + JSON.stringify(unexplainedMissing.map((t) => ({ id: t.userTaskId, slot: (t.targetIds ?? [])[0] ?? null }))),
    );
  }
  if (reasonProblems.length === 0) {
    record("remaining-work-has-a-real-reason", SCENARIO_STATUS.PASSED, {
      answerKeyMissing: answersMissing.length,
      blocking: blocking.length,
      total: remaining.length,
      note: answersMissing.length > 0
        ? "每条缺答案都对应权威稿里真实的 unresolved 槽；原文件没有答案页，云端不得编造答案"
        : "本次没有「缺答案」类剩余任务",
    });
  } else {
    record("remaining-work-has-a-real-reason", SCENARIO_STATUS.FAILED, { problems: reasonProblems, breakdown: report.remainingBreakdown });
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
      // 但它是**产品失败**（用户点得动的操作被拒绝），不是「环境不满足」——
      // 以前 throw CannotRunError 会把它降级成退出码 3。
      if (!filled?.ok) {
        record("export-and-student-runtime", SCENARIO_STATUS.FAILED, {
          problems: [`用户补答案被拒绝：${filled?.error}`],
          userAnswerFill: report.observed.userAnswerFill,
        });
        writeFinalReport();
        return;
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
    // 只认**干净**发布：放行发布（`published_forced`）在界面上同样显示「已发布」，
    // 但对这条验收链不算通过。判据是库里的条目状态，不是提示文案。
    report.observed.publish.outcome = await session.evaluate(
      `(() => { const el = document.querySelector('.workspace-notice'); return el ? el.getAttribute('data-publish-outcome') : null; })()`,
    );
    if (afterExport?.item?.status === "published") {
      // 发布成功**还不够**。链条的最后一跳是「学生端加载」，而这一跳此前只有
      // `nas-student-contract.mjs` 那层**镜像**证据（按学生端规则重新实现了一遍校验）。
      // 镜像证明「包符合规则」，证明不了「学生端那份代码真的能读」。这里补上真实一跳：
      // 直接 require 学生端仓库**已编译的真实 provider**，把这个发布包当 NAS 挂上去，
      // 跑 `getStatus()` / `listAssets()` / `getAsset()`，并核对加载出来的内容
      // 与云端修复后的 canonical 一致（分页残留已消失、答案键齐全）。
      const studentLoad = await loadPublishedPackageWithRealProviderAsync({
        packageDir: nasDir,
        examId: null,
      }).catch((error) => {
        // 抛错要**分类**，不能一律当「环境不满足」：
        //   · 学生端模块根本加载不了（仓库不在 / 没编译）→ 环境不满足，not-executable；
        //   · 模块加载了、读这个包时崩了 → 那是「读不了这个包」，是**失败**。
        // 以前 `cannotRun: true` 一刀切，把「学生端读不了我们发布的包」这个真实产品
        // 缺陷伪装成「这台机器上没有学生端」。
        const message = String(error?.message ?? error);
        const environment = /Cannot find module|ERR_MODULE_NOT_FOUND|MODULE_NOT_FOUND|ENOENT/u.test(message);
        return {
          ok: false,
          cannotRun: environment,
          reason: environment
            ? `学生端真实代码不可用：${message}`
            : `学生端真实代码读不了这个发布包：${message}`,
          results: [],
          failures: [],
        };
      });
      report.observed.studentRealProviderLoad = {
        ok: studentLoad.ok,
        cannotRun: Boolean(studentLoad.cannotRun),
        reason: studentLoad.reason ?? null,
        examId: studentLoad.examId ?? null,
        providerPath: studentLoad.providerPath ?? null,
        observed: studentLoad.observed ?? null,
        passed: studentLoad.results.filter((entry) => entry.ok).length,
        total: studentLoad.results.length,
        failures: studentLoad.failures.map((entry) => ({ name: entry.name, detail: entry.detail })),
      };
      fs.writeFileSync(
        path.join(runDir, "student-real-provider-load.json"),
        JSON.stringify(report.observed.studentRealProviderLoad, null, 2),
      );

      if (studentLoad.cannotRun) {
        // 学生端仓库不在本环境里 → 如实记 not-executable，而不是把发布成功当成人端也过了。
        notExecutable(
          "export-and-student-runtime",
          `发布成功，但学生端真实代码不可用：${studentLoad.reason}`,
        );
      } else if (!studentLoad.ok) {
        record("export-and-student-runtime", SCENARIO_STATUS.FAILED, {
          ...report.observed.exportAttempt.after,
          note: "发布成功，但学生端真实代码读不了这个包",
          studentRealProviderLoad: report.observed.studentRealProviderLoad,
        });
      } else {
        record("export-and-student-runtime", SCENARIO_STATUS.PASSED, {
          ...report.observed.exportAttempt.after,
          studentRealProviderLoad: report.observed.studentRealProviderLoad,
        });
      }
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
      // 探针用快照：取不到就如实记「没跑到」，**不**让链条失败（它只是诊断）。
      const immediate = dumpDb(path.join(runDir, "db-retry-probe-immediate.json"), "RetryProbeImmediate", { required: false });
      const before = (immediate?.processingJobs ?? []).find((job) => job.library_item_id === target) ?? null;
      // 给工作线程一点时间真的去跑，再看它落到哪里——只看 retry 的返回值会误判成成功。
      await sleep(12000);
      const settled = dumpDb(path.join(runDir, "db-retry-probe.json"), "RetryProbe", { required: false });
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
  report.modelTracesAfter = { toolCalls: repairToolCalls(itemId), llm: llmTraces(itemId) };
  writeFinalReport();
}

/** `get_workspace_item` 顶层就带 `editVersion`（`library_items_v2.current_edit_version`）。 */
function workspaceVersion(workspace) {
  return workspace?.editVersion ?? workspace?.item?.editVersion ?? null;
}

main()
  .catch(async (error) => {
    // 三类失败必须区分，而且**报告与退出码要用同一份判定**：
    //   · ChainFailure    → 产品缺陷 / 证据不成立 → failed（1）
    //   · CannotRunError  → 环境不满足           → not-executable（5）
    //   · 其它             → 意外异常             → failed（1）
    // 以前这里只写 report 不重算 verdict，于是报告里的 `verdict` 还是初始的
    // "failed"、而退出码是 3 —— 两套口径。
    const cannotRun = error instanceof CannotRunError;
    record("harness", cannotRun ? SCENARIO_STATUS.NOT_EXECUTABLE : SCENARIO_STATUS.FAILED, {
      message: String(error?.message ?? error),
      kind: error?.name ?? "Error",
      detail: error instanceof ChainFailure ? error.detail : undefined,
    }, error);
    try {
      report.appOutput = session?.appOutput?.() ?? null;
    } catch {
      // 忽略
    }
    try {
      fs.mkdirSync(runDir, { recursive: true });
    } catch {
      // 忽略
    }
    writeFinalReport();
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
