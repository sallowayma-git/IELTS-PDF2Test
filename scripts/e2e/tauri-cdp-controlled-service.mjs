#!/usr/bin/env node
/**
 * 受控模型服务 → 真实网关 → 真实候选 → **真实按钮** 的端到端验收（WebView2 CDP）。
 *
 * 为什么单独一个脚本：
 *   后端已交付「受控模型服务」这一层（`scripts/controlled-llm-service.mjs` +
 *   `fixtures/controlled-llm/*`），并在
 *   `Plan With Files/Dual_Recognition/CONTROLLED_LLM_SCENARIO_2026-09-16.md` 里明确写道：
 *   「真实 UI：否 …… 前端按第 3 节执行即可，但**结果要由前端侧回报**」。
 *   本脚本就是那次回报的执行体。它**不是**在验证网关（那是后端用例的职责），
 *   而是在验证「服务 → 网关 → 裁决 → 持久化 → 界面按钮」这整条产品链。
 *
 * 与前一层证据的分工（不可互相代替）：
 *   - 后端 `cargo test --lib controlled_model_service...`：网关解析/校验这一段；
 *   - 本脚本：真实进程 + 真实 IPC + 真实 DOM 点击，且断言读**真实权威稿**。
 *
 * 八个场景：
 *   1. controlled-service-drives-candidates       受控服务经真实网关产出预期候选
 *   2. expected-sample-reproducible               后端样本在本环境是否可复现（前提不成立时记 not-executable）
 *   3. accept-manual-candidate                    在界面上点「采用修正」→ 权威稿写入 + 版本推进 + 重开保留
 *   4. undo-manual-accept                         手工接受后**撤销入口是否出现**、点了是否真的回滚
 *   5. a3-a4-requests-reach-service               A3/A4 请求**确实**到达了受控服务（不是拿 outline 样本当通过）
 *   6. verification-status-matches-chains         界面那句话与后端四路链状态**逐字一致**（独立规则实现）
 *   7. late-model-result-does-not-overwrite-user-edit  用户先改过 → 迟到的模型结果不覆盖用户内容
 *   8. a3-partial-not-reported-as-complete        A3 部分返回 → 「部分内容尚未完成校验」，不是「校验完成」
 *   9. a3-model-failure-not-reported-as-complete  A3 调用失败 → 同样如实降级，且不泄露原因码、不拖住编辑
 *
 * 场景 4 是 R13 的重点，也是文档明确要求的：
 *   CONTROLLED_LLM_SCENARIO_2026-09-16.md §4「重要：撤销对**手工接受**的项也应按同一闭环退出」。
 * 场景 5–9 是 R14 新增：A3/A4 落地后，受控服务此前**只**覆盖 outline 候选，
 * 「旧样本通过」不能当作「A3/A4 已联通」。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-controlled-service.mjs [--pdf <path>] [--port N] [--service-mode normal|decline|partial|fail|garbage] [--service-fixture FILE] [--keep] [--tolerate-concurrent-edits]
 * 退出码：0 通过 / 1 失败 / 2 部分无法执行 / 3 环境不满足（不可执行）/ 5 全部无法执行。
 */

import { spawn } from "node:child_process";
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

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const fixtureIdx = process.argv.indexOf("--pdf");
const fixturePath = path.resolve(
  fixtureIdx >= 0 ? process.argv[fixtureIdx + 1] : path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf")
);
const portIdx = process.argv.indexOf("--port");
const servicePort = portIdx >= 0 ? Number(process.argv[portIdx + 1]) : 11435;
// A3/A4 的行为由受控服务的 `--mode` 决定（见 `scripts/controlled-llm-service.mjs`）。
// 默认 `normal`：A3 逐槽位 confirmed、A4 逐分歧选有值的那条链。
const modeIdx = process.argv.indexOf("--service-mode");
const initialServiceMode = modeIdx >= 0 ? String(process.argv[modeIdx + 1]) : "normal";
const isPdf = /\.pdf$/i.test(fixturePath);
// 与 `tauri-cdp-recognition-buttons.mjs` 同一开关：某些环境下 WebView2 的渲染进程会崩，
// 关掉 GPU/沙箱能让它稳定起来。诊断运行**不能**当作验收通过，报告里会标明。
const diagnosticArgsRequested = process.argv.includes("--diagnostic-args");
const extraArgs = diagnosticArgsRequested ? "--no-sandbox --disable-gpu" : "";

const serviceScript = path.join(repoRoot, "scripts", "controlled-llm-service.mjs");
const expectedPath = path.join(repoRoot, "fixtures", "controlled-llm", "expected-decisions.json");
const PROFILE_ID = "controlled-outline";

const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `run-controlled-service-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const appDataDir = path.join(runDir, "appdata", "data");

const report = {
  task: "controlled-model-service-ui-acceptance",
  scope: "受控服务经真实网关产出候选，并在真实界面上验收接受/撤销按钮（不是 IPC 探针）",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  runProfile: "cdp-default",
  startedAt: new Date().toISOString(),
  runDir,
  identity: {
    exePath,
    exeSha256: null,
    fixturePath,
    fixtureSha256: null,
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: null,
    serviceScript,
    expectedPath,
    servicePort,
  },
  service: { started: false, health: null, requestLines: [], modes: [], currentMode: null },
  scenarios: [],
  verdict: "failed",
};

let session = null;
let itemId = null;
let serviceChild = null;

/** 场景登记：三态，绝不使用「跳过」。 */
function record(name, status, detail, error) {
  const entry = { name, status, detail: detail ?? null };
  if (error) entry.error = String(error?.message ?? error);
  report.scenarios.push(entry);
  console.log(`[scenario] ${status.toUpperCase()} ${name}${entry.error ? ` :: ${entry.error}` : ""}`);
}

async function scenario(name, fn) {
  try {
    const detail = await fn();
    record(name, SCENARIO_STATUS.PASSED, detail);
  } catch (error) {
    record(name, SCENARIO_STATUS.FAILED, null, error);
  }
}

function notExecutable(name, reason) {
  record(name, SCENARIO_STATUS.NOT_EXECUTABLE, { reason });
}

/** 调真实 IPC，返回 {ok, value|error}。 */
async function call(command, args) {
  const wrapped = command === "apply_recognition_decisions" || command === "apply_editor_commands";
  const r = await session.invoke(command, wrapped ? { input: args } : args);
  if (r && r.__noInvoke) throw new Error("invoke 不可用");
  return r;
}

async function readDraft() {
  const r = await call("get_workspace_item", { itemId });
  if (!r?.ok) throw new Error(`get_workspace_item 失败：${r?.error}`);
  return r.value;
}

async function readDecision() {
  const r = await call("get_recognition_decision", { itemId });
  if (!r?.ok) throw new Error(`get_recognition_decision 失败：${r?.error}`);
  const v = r.value ?? {};
  return {
    batchId: v.batchId ?? null,
    editVersion: Number(v.editVersion ?? 0),
    stale: Boolean(v.stale),
    baseEditVersion: v.baseEditVersion ?? null,
    currentEditVersion: v.currentEditVersion ?? v.editVersion ?? null,
    actionable: Array.isArray(v.actionable) ? v.actionable : [],
    autoApplied: Array.isArray(v.autoApplied) ? v.autoApplied : [],
    chains: v.chains ?? null,
  };
}

/** 面板里某个 decisionId 的卡片状态（按钮可见性 + data 属性）。 */
async function cardState(decisionId) {
  return session.evaluate(
    `(() => {
      const card = document.querySelector('[data-decision-id="${decisionId}"]');
      const undoLi = document.querySelector('li[data-decision-id="${decisionId}"]');
      return {
        cardPresent: !!card,
        status: card ? card.getAttribute('data-status') : null,
        resolution: card ? card.getAttribute('data-resolution') : null,
        hasAccept: !!document.querySelector('[data-testid="workspace-recognition-accept-${decisionId}"]'),
        hasKeep: !!document.querySelector('[data-testid="workspace-recognition-keep-${decisionId}"]'),
        hasUndo: !!document.querySelector('[data-testid="workspace-recognition-undo-${decisionId}"]'),
        undoUnavailable: !!document.querySelector('[data-testid="workspace-recognition-undo-unavailable-${decisionId}"]'),
        undone: !!document.querySelector('[data-testid="workspace-recognition-undone-${decisionId}"]'),
        undoState: undoLi ? undoLi.getAttribute('data-undo-state') : null,
        text: card ? card.innerText.replace(/\\s+/g,' ').trim() : null
      };
    })()`
  );
}

async function openPanel() {
  // **幂等**：面板已经打开时不要再点一次切换按钮（那会把它关掉，后续断言读到 null）。
  const already = await session.evaluate(`!!document.querySelector('[data-testid="workspace-recognition"]')`);
  if (!already) {
    await session.clickSelector('[data-testid="workspace-recognition-toggle"]');
  }
  await session.waitFor(`!!document.querySelector('[data-testid="workspace-recognition"]')`, { timeoutMs: 20000, label: "recognition-panel" });
}

async function reopenWorkspace() {
  await session.clickSelector('[data-testid="workspace-back"]');
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000, label: "library-after-back" });
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-reopen" });
  await openPanel();
}

/** 等受控服务起来（GET /health）。 */
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

function startService(fixtureOverride, mode = initialServiceMode) {
  const args = [serviceScript, "--port", String(servicePort), "--mode", mode];
  // 允许指向一份**派生样本**：后端交付的样本目标是 q14，而本仓夹具没有 q14
  // （passage-3 是 q27–q40，passage-1.docx 是 q1–q13），于是云端对真实槽位一言不发，
  // 29 条候选里 **0 条**带可应用补丁 —— 接受/撤销按钮根本不会出现。
  // 派生样本只把目标槽位换成一个**真实存在且本地为空**的槽位，用于让按钮可达；
  // 它不修改后端样本，报告里用 `serviceFixture` 明确记录用了哪一份。
  if (fixtureOverride) args.push("--fixture", fixtureOverride);
  serviceChild = spawn(process.execPath, args, {
    cwd: repoRoot,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let out = "";
  serviceChild.stdout.on("data", (chunk) => {
    out += String(chunk);
    for (const line of String(chunk).split(/\r?\n/)) {
      if (line.includes("[controlled-llm]") && line.includes("POST")) report.service.requestLines.push(line.trim());
    }
  });
  serviceChild.stderr.on("data", (chunk) => { out += String(chunk); });
  report.service.stdout = () => out;
  report.service.currentMode = mode;
  report.service.modes.push(mode);
  return out;
}

/**
 * 换一个 `--mode` 重启受控服务。
 *
 * 为什么要重启而不是改文件：A3/A4 的三种非成功态（无法判断 / 部分返回 / 调用失败）
 * 只有在服务**真的这样回答**时才能被触发。用一个进程内的开关改行为会让「换模式」
 * 本身变成不可信的一步；重启子进程 + 重新 `/health` 是最难作弊的做法。
 */
async function restartService(mode, fixtureOverride = null) {
  if (serviceChild) {
    serviceChild.kill();
    serviceChild = null;
    await sleep(900);
  }
  startService(fixtureOverride, mode);
  const health = await waitForService();
  if (!health) throw new CannotRunError(`受控服务在 mode=${mode} 下没有就绪（/health 不可达）`);
  return health;
}

/** 从某个下标之后的请求行（用来把「这次重跑收到的请求」与上一轮分开）。 */
function requestsSince(index) {
  return report.service.requestLines.slice(index);
}

/**
 * 作业目录里网关调用的**痕迹**。
 *
 * 为什么必须读它：`run_llm_gateway` 对每次调用都会写 `<command>-input-<stamp>.json`、
 * 追加一行 `llm-calls.jsonl`、成功时再写 `<command>-output-<stamp>.json`（`llm_gateway.rs:30-79`）。
 * 三件套是否齐全，决定「受控服务没收到请求」到底是**应用没发起**还是**发起了但没落地**
 * ——这两件事的归属完全不同：前者是前提不成立（`not-executable`），后者是链路缺陷（`failed`）。
 * 把它们混成一个 `not-executable`，就是用「前提不成立」掩盖「东西坏了」。
 */
function llmGatewayTraces(jobId) {
  const jobDir = path.join(appDataDir, "jobs", String(jobId ?? ""));
  const dir = path.join(jobDir, "cache", "llm");
  const traces = { dir, exists: fs.existsSync(dir), files: [], byCommand: {}, callRecordExists: false, callRecords: [] };
  if (traces.exists) {
    traces.files = fs.readdirSync(dir);
    for (const file of traces.files) {
      const matched = /^(.+)-(input|output)-\d+\.json$/u.exec(file);
      if (!matched) continue;
      const command = matched[1];
      traces.byCommand[command] = traces.byCommand[command] ?? { input: 0, output: 0 };
      traces.byCommand[command][matched[2]] += 1;
    }
  }
  const recordPath = path.join(jobDir, "llm-calls.jsonl");
  if (fs.existsSync(recordPath)) {
    traces.callRecordExists = true;
    traces.callRecords = fs
      .readFileSync(recordPath, "utf8")
      .split(/\r?\n/)
      .filter(Boolean)
      .map((line) => {
        try {
          const parsed = JSON.parse(line);
          return {
            commandName: parsed.commandName,
            ok: parsed.ok,
            errorClass: parsed.errorClass ?? null,
            latencyMs: parsed.latencyMs ?? null,
          };
        } catch {
          return { raw: line.slice(0, 200) };
        }
      });
  }
  return traces;
}

/** 面板顶部那一行核验状态（用户实际看到的那句话）。 */
async function readStatusLine() {
  return session.evaluate(
    `(() => { const el = document.querySelector('[data-testid="workspace-recognition-cloud"]'); return el ? el.innerText.replace(/\\s+/g,' ').trim() : null; })()`
  );
}

/**
 * **独立实现**的核验状态行规则（与 `recognitionClient.describeVerificationStatus` 同规则、
 * 但这里不复用它的代码）。用来断言「界面那句话」与「后端四路链状态」一致。
 *
 * 最容易写错的一条：**空列表不等于核验成功**。只要还有一路没真正跑完，
 * 即使一条待处理项都没有，也不能说「没有发现需要处理的问题」。
 */
function expectedStatusText({ localStatus, cloudStatus, sourceStatus, adjudicationStatus, pendingCount }) {
  const PARTIAL = new Set(["partial"]);
  const UNAVAILABLE = new Set(["failed", "unavailable"]);
  const UNFINISHED = new Set(["partial", "failed", "unavailable", "not_started", "skipped"]);
  if (localStatus === "queued" || localStatus === "running") return "正在本机识别…";
  if (cloudStatus === "queued" || cloudStatus === "running") return "云端正在校验…";
  if (!cloudStatus || cloudStatus === "not_started") return "题稿已生成，可以开始编辑";
  if (UNAVAILABLE.has(cloudStatus)) return "云端校验暂时不可用，不影响继续编辑";
  const states = [cloudStatus, sourceStatus ?? "not_started", adjudicationStatus ?? "not_started"];
  const partial = states.some((s) => PARTIAL.has(s));
  const unfinished = states.some((s) => UNFINISHED.has(s));
  if (pendingCount > 0) return partial ? "部分内容尚未完成校验，请检查标出的题目" : `云端发现 ${pendingCount} 处建议`;
  return unfinished ? "部分内容尚未完成校验" : "云端校验完成，没有发现需要处理的问题";
}

/** 后端四路链状态 → 归一化状态（与 `CHAIN_STATE_TO_STATUS` 同规则，独立写出）。 */
const CHAIN_STATE_TO_STATUS = {
  queued: "queued", not_run: "not_started", not_started: "not_started", pending: "not_started",
  canceled: "not_started", cancelled: "not_started", running: "running", in_progress: "running",
  done: "succeeded", ok: "succeeded", succeeded: "succeeded", success: "succeeded",
  partial: "partial", failed: "failed", error: "failed", unusable: "unavailable",
  unavailable: "unavailable", skipped: "skipped",
};
function normalizeChainState(state) {
  if (typeof state !== "string" || !state.trim()) return "not_started";
  return CHAIN_STATE_TO_STATUS[state] ?? state;
}
function chainSnapshot(decision) {
  const chains = decision?.chains ?? {};
  return {
    local: normalizeChainState(chains.local?.state),
    cloud: normalizeChainState(chains.cloud?.state),
    source: normalizeChainState(chains.source?.state),
    adjudication: normalizeChainState(chains.adjudication?.state),
    sourceReasonCode: chains.source?.reasonCode ?? null,
    adjudicationReasonCode: chains.adjudication?.reasonCode ?? null,
    cloudReasonCode: chains.cloud?.reasonCode ?? null,
    raw: chains,
  };
}
/** 待用户处理的条数（`open` 的 needs_review / unverifiable / failed）。 */
function pendingCountOf(decision) {
  return [...(decision?.actionable ?? []), ...(decision?.autoApplied ?? [])]
    .filter((i) => i.status === "open" && (i.resolution === "needs_review" || i.resolution === "unverifiable" || i.status === "failed"))
    .length;
}

/** 触发一次重新识别，并等一个**新**批次落地。 */
async function rerunRecognition(previousBatchId, timeoutMs = 300000) {
  const retry = await call("retry_processing", { itemId });
  if (!retry?.ok) throw new CannotRunError(`retry_processing 失败：${retry?.error}`);
  const deadline = Date.now() + timeoutMs;
  let decision = null;
  while (Date.now() < deadline) {
    decision = await readDecision();
    const cloud = decision.chains?.cloud?.state ?? null;
    if (decision.batchId && decision.batchId !== previousBatchId && cloud && !["queued", "running"].includes(cloud)) return decision;
    await sleep(2000);
  }
  return decision;
}

/**
 * 从**真实草稿**派生一份 outline 样本，把受控服务的应答目标挪到一个真实存在的槽位上。
 *
 * 为什么必须派生：后端样本的目标是 `q14`，而本仓夹具里没有 q14
 * （`demanding-reading-passage-3.pdf` 是 q27–q40，`demanding-reading-passage-1.docx` 是 q1–q13），
 * 于是云端对真实槽位一言不发 → **没有分歧** → A4 根本没有可裁决的对象，
 * 「A4 裁定 → 采用 → 撤销」这条路在当前夹具上不可达。
 *
 * 派生只改「目标槽位」，其余（title / 证据 / 形状）照抄后端样本；
 * 它**不修改** `fixtures/controlled-llm/reading-outline.json`，也不改任何后端文件。
 * 目标优先选一个**本地为空**的槽位：这样云端给的值构成 `SUBSTANTIVE_DIVERGENCE`，
 * 采纳后是一份干净的新答案，便于断言。
 */
function deriveOutlineFixture(draft, outPath) {
  const candidates = [];
  for (const task of draft.taskGroups ?? []) {
    for (const group of task.responseGroups ?? []) {
      for (const slotId of group.slotIds ?? []) {
        const slot = (draft.answerSlots ?? {})[slotId];
        if (slot && slot.participation !== "scoring") continue;
        const number = Number(slot?.questionNumber ?? 0);
        if (!Number.isFinite(number) || number <= 0) continue;
        const answer = (draft.answerKey ?? {})[slotId];
        const empty = !answer || answer.kind === "unresolved"
          || (answer.kind === "text" && !(answer.values ?? []).some((v) => String(v).trim()))
          || (answer.kind === "option" && !(answer.labels ?? []).length);
        candidates.push({ slotId, number, empty });
      }
    }
  }
  const target = candidates.find((c) => c.empty) ?? candidates[0];
  if (!target) return null;
  const sample = JSON.parse(fs.readFileSync(path.join(repoRoot, "fixtures", "controlled-llm", "reading-outline.json"), "utf8"));
  delete sample._comment;
  const group = sample.groups[0];
  group.range = [target.number, target.number];
  group.questionIds = [target.slotId];
  group.slots[0].questionNumber = target.number;
  sample._derivedFrom = "fixtures/controlled-llm/reading-outline.json";
  sample._derivedNote = `受控服务应答目标由样本的 q14 改为本夹具真实存在的 ${target.slotId}（第 ${target.number} 题，本地${target.empty ? "为空" : "已有值"}）。派生不改后端样本。`;
  fs.writeFileSync(outPath, JSON.stringify(sample, null, 2));
  return { ...target, path: outPath, derivedFrom: "fixtures/controlled-llm/reading-outline.json" };
}


/**
 * 把受控 profile 写进应用数据目录。
 *
 * 关键机制（`src/features/import/useImportFiles.ts`）：导入时前端会自己 `listLlmProfiles()`，
 * 只要存在一个「enabled 且不是 `profile-local-placeholder`」的 profile，就自动
 * `cloudEnabled = true` 并把它的 id 当 `cloudProfileId`。所以**不需要**去点任何开关，
 * 也不需要伪造 localStorage —— 写文件即可，走的就是产品的正常判定。
 */
function writeProfile() {
  const configDir = path.join(appDataDir, "config");
  fs.mkdirSync(path.join(configDir, "secrets"), { recursive: true });
  const profile = {
    profileId: PROFILE_ID,
    name: "Controlled Outline Service",
    provider: "OpenAiCompatible",
    baseUrl: `http://127.0.0.1:${servicePort}/v1`,
    model: "controlled-outline-v1",
    temperature: 0,
    timeoutMs: 60000,
    forceJson: true,
    enabled: true,
  };
  fs.writeFileSync(path.join(configDir, "llm-profiles.json"), JSON.stringify([profile], null, 2));
  fs.writeFileSync(path.join(configDir, "secrets", `${PROFILE_ID}.key`), "controlled-service-token");
  return profile;
}

/**
 * 从应用完整输出里挑出 panic / 致命错误行，落进报告。
 *
 * 为什么必须做：A3 请求「输入缓存有、调用记录与输出都没有、服务端零 POST」这种情形，
 * 光看网关痕迹只能说「既没有结果也无法诊断」—— 那是一个**没有根因的结论**。
 * 而应用日志里往往**已经有**根因。实测到的就是一例：
 *
 *   thread 'tokio-rt-worker' panicked at tokio-1.52.3/src/runtime/blocking/shutdown.rs:51:21:
 *   Cannot drop a runtime in a context where blocking is not allowed.
 *
 * 不把它提取出来，读者就得自己翻 `app-output.log`；上一轮把「没有 trace」当成结论，
 * 正是因为少了这一步。提取出来之后，「A3 为什么没到服务」在报告里就是可读的。
 */
function extractAppPanics(appOutput) {
  const lines = String(appOutput ?? "").split(/\r?\n/);
  const hits = [];
  let covered = -1;
  for (let i = 0; i < lines.length; i += 1) {
    // panic 的消息体常常跨 2–3 行（`panicked at …` / 原因 / `note:`），
    // 只按起点匹配会产出互相重叠的重复条目。已并入上一条的行不再单独成条。
    if (i <= covered) continue;
    if (!/panicked at|Cannot drop a runtime|Cannot start a runtime/i.test(lines[i])) continue;
    const end = Math.min(lines.length, i + 3);
    hits.push(lines.slice(i, end).join(" ").replace(/\s+/g, " ").trim());
    covered = end - 1;
  }
  return hits;
}

async function main() {
  const tolerateConcurrentEdits = process.argv.includes("--tolerate-concurrent-edits");
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);

  if (!fs.existsSync(fixturePath)) throw new CannotRunError(`夹具不存在：${fixturePath}`);
  report.identity.fixtureSha256 = sha256File(fixturePath);
  if (!fs.existsSync(expectedPath)) throw new CannotRunError(`预期样本不存在：${expectedPath}`);
  const expected = JSON.parse(fs.readFileSync(expectedPath, "utf8"));
  report.expected = expected;

  // ---- 0. 起受控服务 ----
  // `--skip-service` 是**诊断开关**：用来隔离「起这个子进程是否影响应用渲染」。
  // 带它跑时没有云端可用，只用于定位，结果不得当作验收通过。
  const skipService = process.argv.includes("--skip-service");
  report.serviceSkipped = skipService;
  /** 服务当前实际在用的样本（显式 `--service-fixture` 优先；否则可能在派生后改写）。 */
  let effectiveFixture = null;
  const fixtureIdx2 = process.argv.indexOf("--service-fixture");
  const explicitServiceFixture = fixtureIdx2 >= 0 ? path.resolve(process.argv[fixtureIdx2 + 1]) : null;
  if (!skipService) {
    report.serviceFixture = explicitServiceFixture;
    effectiveFixture = explicitServiceFixture;
    if (explicitServiceFixture && !fs.existsSync(explicitServiceFixture)) {
      throw new CannotRunError(`--service-fixture 指向的文件不存在：${explicitServiceFixture}`);
    }
    startService(effectiveFixture);
    const health = await waitForService();
    if (!health) throw new CannotRunError(`受控服务在 127.0.0.1:${servicePort} 上没有就绪（/health 不可达）`);
    report.service.started = true;
    report.service.health = health;
    console.log(`[controlled] service up: ${JSON.stringify(health)}`);
  }

  // ---- 1. 应用数据目录 + profile ----
  fs.mkdirSync(appDataDir, { recursive: true });
  // `--skip-profile` 是**诊断开关**：用来隔离「写 profile 是否导致启动失败」这一个变量。
  // 带它跑时云端必然 not_run(NO_PROFILE)，只用于定位，结果不得当作验收通过。
  const skipProfile = process.argv.includes("--skip-profile");
  report.profileSkipped = skipProfile;
  report.profile = skipProfile ? null : writeProfile();

  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  const staged = path.join(runDir, "pdfs", path.basename(fixturePath));
  fs.copyFileSync(fixturePath, staged);

  // ---- 2. 启动应用 ----
  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    appEnv: {
      // 明文密钥回退：profile 的密钥写在 config/secrets/<id>.key 里。
      EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK: "1",
      ...(isPdf ? {} : { PDF2TEST_AUTOMATION_SOURCE_FILES: staged }),
    },
  });
  report.identity.browserArgs = session.browserArgs;
  report.diagnosticRun = Boolean(extraArgs);
  report.runProfile = extraArgs ? "cdp-diagnostic" : "cdp-default";
  report.securityArgs = extraArgs ? extraArgs.split(/\s+/).filter(Boolean) : [];

  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });

  // 先确认应用真的看到了我们写的 profile —— 这是「云端会被启用」的前提，也是本层证据的一部分。
  const profiles = await call("list_llm_profiles", {});
  report.profilesSeenByApp = profiles?.ok
    ? (profiles.value ?? []).map((p) => ({ profileId: p.profileId, enabled: p.enabled, hasApiKey: p.hasApiKey, model: p.model }))
    : { error: profiles?.error };

  // ---- 3. 导入（走真实 UI）----
  const before = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
  await session.clickSelector(isPdf ? '[data-testid="import-pick-folder"]' : '[data-testid="import-pick-files"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked-files" });
  await session.clickSelector('[data-testid="import-start"]');
  const deadline = Date.now() + 90000;
  while (Date.now() < deadline && !itemId) {
    const ids = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
    itemId = (ids ?? []).find((id) => !(before ?? []).includes(id)) ?? null;
    if (!itemId) await sleep(1000);
  }
  if (!itemId) throw new CannotRunError("导入后未出现新的题库行");
  report.identity.itemId = itemId;

  // ---- 4. 等本地稿落盘 ----
  const draftDeadline = Date.now() + 120000;
  let draft = null;
  while (Date.now() < draftDeadline) {
    const w = await readDraft();
    if (w?.ds) { draft = w.ds; break; }
    await sleep(2000);
  }
  if (!draft) throw new CannotRunError("本地稿在超时前没有落盘");

  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-open" });
  await openPanel();

  // ---- 5. 等云端链终态（受控服务应当成功）----
  // 判据：批次出现 **且** 云端链离开 queued/running。不能用「有 chains 就算好」——
  // 无批次时后端也会返回四条 not_run，那会让等待立刻通过，把「还没开始」误判成结果。
  const cloudDeadline = Date.now() + 180000;
  let decision = null;
  let cloudState = null;
  while (Date.now() < cloudDeadline) {
    decision = await readDecision();
    cloudState = decision.chains?.cloud?.state ?? null;
    if (decision.batchId && cloudState && !["queued", "running"].includes(cloudState)) break;
    await sleep(2000);
  }
  report.observed = {
    batchId: decision?.batchId ?? null,
    cloudState,
    cloudReasonCode: decision?.chains?.cloud?.reasonCode ?? null,
    chains: decision?.chains ?? null,
  };

  // ---- 5b. 默认导入路径没产出批次时，用「播种 + 重试」把它推到可裁决 ----
  //
  // 这是对**未修复的后端顺序缺陷**的显式绕行，**不是产品默认路径**，结果里用
  // `usedSeedRetryWorkaround` 标记，绝不把绕行当默认路径通过。
  //
  // 缺陷（F-R12-2）：`scheduler.rs` 在 `set_item_status_ready`（它内部才
  // `migrate_single_item` 播种权威稿）**之前**冻结快照，首次导入必然
  // `canonical_not_seeded` → 按新逻辑冻结失败即拒绝裁决 → 没有批次。
  // 后果：即使受控服务完全正常，云端结果也会被丢弃、裁决被跳过，候选恒为 0 ——
  // 「候选接受/撤销按钮」这一层根本不可达。
  // 见 `Plan With Files/Dual_Recognition/HANDOFF_2026-09-16_r12-2_freeze_before_seed.md`。
  //
  // 绕行机制：上面第 4 步的 `readDraft()` 已经调过 `get_workspace_item`（会播种），
  // 因此此刻重试即可冻结成功、走到裁决。
  if (!decision?.batchId) {
    report.usedSeedRetryWorkaround = true;
    console.log("[controlled] 默认导入未产出批次（后端冻结顺序缺陷 F-R12-2）；执行「播种 + 重试」绕行以触达按钮层");
    const retry = await call("retry_processing", { itemId });
    if (!retry?.ok) throw new CannotRunError(`retry_processing 失败：${retry?.error}`);
    const retryDeadline = Date.now() + 240000;
    while (Date.now() < retryDeadline) {
      decision = await readDecision();
      cloudState = decision.chains?.cloud?.state ?? null;
      if (decision.batchId && cloudState && !["queued", "running"].includes(cloudState)) break;
      await sleep(2000);
    }
    report.observed.afterRetry = {
      batchId: decision?.batchId ?? null,
      cloudState,
      cloudReasonCode: decision?.chains?.cloud?.reasonCode ?? null,
      chains: decision?.chains ?? null,
    };
    console.log(`[controlled] 重试后 batchId=${String(report.observed.afterRetry.batchId)} cloud=${String(cloudState)}`);
  }

  // ---- 5c. 让 A4 有可裁决的对象：派生样本 → 重启服务 → 重跑一次 ----
  //
  // 不派生时（后端样本目标 q14 在本仓夹具里不存在），云端对真实槽位一言不发 →
  // 没有分歧 → A4 走「没有分歧项 → 完全不碰 → Succeeded」这条**正确但测不到模型**的分支，
  // 「A4 裁定 → 采用 → 撤销」整条路不可达。派生只改应答目标，不改后端样本。
  // 显式给了 `--service-fixture` 就尊重它，不派生（保证可复现他人给的样本）。
  if (!skipService && !explicitServiceFixture) {
    const derived = deriveOutlineFixture(draft, path.join(runDir, "derived-outline.json"));
    report.derivedFixture = derived;
    if (derived) {
      console.log(`[controlled] 派生受控样本：应答目标 → ${derived.slotId}（第 ${derived.number} 题）`);
      effectiveFixture = derived.path;
      await restartService(initialServiceMode, effectiveFixture);
      report.serviceFixture = effectiveFixture;
      const beforeBatch = decision?.batchId ?? null;
      decision = await rerunRecognition(beforeBatch);
      cloudState = decision?.chains?.cloud?.state ?? null;
      report.observed.afterDerivedFixture = {
        batchId: decision?.batchId ?? null,
        cloudState,
        chains: decision?.chains ?? null,
      };
      console.log(`[controlled] 派生样本重跑后 batchId=${String(decision?.batchId)} cloud=${String(cloudState)}`);
    } else {
      console.log("[controlled] 草稿里没有可用的答案槽位，跳过派生（A4 的模型通道本次不可达）");
    }
  }

  // ---- 场景 1：受控服务经真实网关产出真实候选（本层的核心断言）----
  await scenario("controlled-service-drives-candidates", async () => {
    if (!decision?.batchId) throw new Error(`没有产出批次：chains=${JSON.stringify(decision?.chains)}`);
    if (cloudState !== "succeeded") {
      throw new Error(`云端链不是 succeeded（实际 ${String(cloudState)}，原因码 ${String(decision.chains?.cloud?.reasonCode)}）——受控服务没有被真实网关消费`);
    }
    // 服务确实被调用过（不是从缓存/替身来的）——取服务自身打印的 POST 行做旁证。
    // `parts=[text,file]` 还顺带证明请求是**真实 prompt 构造器**产出的（附了原文与文件）。
    if (!report.service.requestLines.length) {
      throw new Error("受控服务没有收到任何 POST /chat/completions —— 云端链不是经由该服务完成的");
    }
    const all = [...decision.actionable, ...decision.autoApplied];
    if (!all.length) throw new Error("云端 succeeded 但没有产出任何候选 —— 裁决没有跑");
    return {
      batchId: decision.batchId,
      cloudState,
      candidates: all.length,
      decisionIds: all.map((i) => i.decisionId),
      serviceCalls: report.service.requestLines.length,
      serviceRequestLines: report.service.requestLines,
      chains: decision.chains,
    };
  });

  const want = expected.items?.[0];
  const allItems = [...(decision?.actionable ?? []), ...(decision?.autoApplied ?? [])];

  // ---- 场景 2：`expected-decisions.json` 在本环境里是否可复现 ----
  //
  // 那份样本断言的目标是 `d:slot:slot-14:answer`，前提是**本地稿里存在一个空的 slot-14**。
  // 但本仓 `fixtures/parser/` 下的真实夹具没有 q14：`demanding-reading-passage-3.pdf`
  // 的槽位是 q27–q40，`demanding-reading-passage-1.docx` 是 q1–q13。
  // 后端的 Rust 用例（`controlled_model_service_drives_candidates_through_the_real_gateway`）
  // 用的是**测试内构造的合成稿**，所以那份「预期」只在合成稿上成立。
  //
  // 因此这里不把它算作产品失败：能对上就断言形状；对不上就如实记为 `not-executable`
  // 并说清前提为何不成立。**不修改样本、不替换夹具**去凑一个通过。
  const expectedFound = want ? allItems.find((i) => i.decisionId === want.decisionId) : undefined;
  if (!want) {
    notExecutable("expected-sample-reproducible", "expected-decisions.json 里没有 items[0]");
  } else if (!expectedFound) {
    notExecutable(
      "expected-sample-reproducible",
      `样本目标 ${want.decisionId} 的前提（本地稿存在空槽位）在本仓夹具里不成立：`
        + "passage-3.pdf 的槽位是 q27–q40，passage-1.docx 是 q1–q13，都没有 q14。"
        + `本次实际产出 ${allItems.length} 条候选，q14 相关项：`
        + `${allItems.map((i) => i.decisionId).filter((id) => id.includes("q14")).join(", ") || "无"}。`
        + "该样本由后端在测试内合成稿上标定；要经真实导入复现，需要一个含空 q14 槽位的夹具。"
    );
  } else
    await scenario("expected-sample-reproducible", async () => {
      const mismatches = [];
      if (expectedFound.resolution !== want.resolution) mismatches.push(`resolution ${expectedFound.resolution} != ${want.resolution}`);
      if ((expectedFound.reasonCode ?? null) !== want.reasonCode) mismatches.push(`reasonCode ${expectedFound.reasonCode} != ${want.reasonCode}`);
      if (JSON.stringify(expectedFound.localValue ?? null) !== JSON.stringify(want.localValue)) mismatches.push(`localValue ${JSON.stringify(expectedFound.localValue)} != ${JSON.stringify(want.localValue)}`);
      if (JSON.stringify(expectedFound.cloudValue ?? null) !== JSON.stringify(want.cloudValue)) mismatches.push(`cloudValue ${JSON.stringify(expectedFound.cloudValue)} != ${JSON.stringify(want.cloudValue)}`);
      if (mismatches.length) throw new Error(`候选与 expected-decisions.json 不一致：${mismatches.join("；")}`);
      return {
        decisionId: expectedFound.decisionId,
        resolution: expectedFound.resolution,
        reasonCode: expectedFound.reasonCode ?? null,
        localValue: expectedFound.localValue ?? null,
        cloudValue: expectedFound.cloudValue ?? null,
      };
    });

  // 按钮场景的目标从**实际**视图里挑，不绑定样本里的 decisionId —— 否则样本与夹具一旦
  // 不匹配，接受/撤销按钮就永远测不到（那会把「样本没标定对」误报成「按钮坏了」）。
  const target = allItems.find(
    (i) => i.resolution === "needs_review" && i.proposedPatch && (i.target?.targetId ?? null)
  );
  if (!target) {
    notExecutable("accept-manual-candidate", "视图里没有「待确认且带可应用补丁」的候选，按钮流程的前提不成立");
    notExecutable("undo-manual-accept", "同上");
  } else {
    const slotId = target.target?.targetId ?? null;
    const proposedValue = target.proposedPatch?.value ?? null;

    // ---- 场景 2：点「采用修正」 ----
    await scenario("accept-manual-candidate", async () => {
      if (!slotId) throw new Error("候选项没有 targetId，无法核对写入位置");
      const beforeDraft = await readDraft();
      const beforeVersion = Number(beforeDraft.editVersion ?? 0);
      const valueBefore = beforeDraft.ds?.answerKey?.[slotId] ?? null;
      await session.clickSelector(`[data-testid="workspace-recognition-accept-${target.decisionId}"]`);
      await session.waitFor(`!document.querySelector('[data-testid="workspace-recognition-accept-${target.decisionId}"]')`, { timeoutMs: 30000, label: "accept-done" });
      const vDeadline = Date.now() + 30000;
      let afterDraft = null;
      while (Date.now() < vDeadline) {
        afterDraft = await readDraft();
        if (Number(afterDraft.editVersion ?? 0) > beforeVersion) break;
        await sleep(500);
      }
      const afterVersion = Number(afterDraft?.editVersion ?? 0);
      if (!(afterVersion > beforeVersion)) throw new Error(`接受后版本没有推进：${beforeVersion} → ${afterVersion}`);
      const valueAfter = afterDraft?.ds?.answerKey?.[slotId] ?? null;
      if (proposedValue && JSON.stringify(valueAfter) !== JSON.stringify(proposedValue)) {
        throw new Error(`接受后 ${slotId} 的值不是建议值：期望 ${JSON.stringify(proposedValue)}，实际 ${JSON.stringify(valueAfter)}`);
      }
      await reopenWorkspace();
      const reopened = await readDraft();
      const valueAfterReopen = reopened?.ds?.answerKey?.[slotId] ?? null;
      if (JSON.stringify(valueAfterReopen) !== JSON.stringify(valueAfter)) {
        throw new Error(`重开后 ${slotId} 的值变了：${JSON.stringify(valueAfter)} → ${JSON.stringify(valueAfterReopen)}`);
      }
      return { decisionId: target.decisionId, slotId, valueBefore, valueAfter, versionBefore: beforeVersion, versionAfter, valueAfterReopen };
    });

    // ---- 场景 3：手工接受后，撤销入口是否出现 / 点了是否回滚 ----
    await scenario("undo-manual-accept", async () => {
      const state = await cardState(target.decisionId);
      const view = await readDecision();
      const inActionable = view.actionable.some((i) => i.decisionId === target.decisionId);
      const inAutoApplied = view.autoApplied.some((i) => i.decisionId === target.decisionId);
      const diagnostics = {
        decisionId: target.decisionId,
        panel: state,
        backendView: { inActionable, inAutoApplied, batchId: view.batchId },
        // 后端视图里这条决策的实际状态（若还在任一份列表里）。
        status: [...view.actionable, ...view.autoApplied].find((i) => i.decisionId === target.decisionId)?.status ?? null,
      };
      if (!state?.hasUndo && !state?.undone) {
        // 如实失败：文档要求「撤销对**手工接受**的项也应按同一闭环退出」。
        const error = new Error(
          `手工接受后没有出现撤销入口（hasUndo=false, undone=false, cardPresent=${String(state?.cardPresent)}, `
          + `后端视图 inActionable=${String(inActionable)} inAutoApplied=${String(inAutoApplied)}）——`
          + "撤销闭环对手工接受的项不成立"
        );
        error.diagnostics = diagnostics;
        throw error;
      }
      // 有入口就真点，并核对回滚。
      const beforeDraft = await readDraft();
      const beforeVersion = Number(beforeDraft.editVersion ?? 0);
      const slotId = target.target?.targetId ?? null;
      const valueBeforeUndo = beforeDraft.ds?.answerKey?.[slotId] ?? null;
      if (state.hasUndo) {
        await session.clickSelector('[data-testid="workspace-recognition-autofixed"] button');
        await session.waitFor(`!!document.querySelector('[data-testid="workspace-recognition-undo-${target.decisionId}"]')`, { timeoutMs: 15000, label: "autofixed-expanded" });
        await session.clickSelector(`[data-testid="workspace-recognition-undo-${target.decisionId}"]`);
      }
      const vDeadline = Date.now() + 30000;
      let afterDraft = null;
      while (Date.now() < vDeadline) {
        afterDraft = await readDraft();
        if (Number(afterDraft.editVersion ?? 0) > beforeVersion) break;
        await sleep(500);
      }
      const afterVersion = Number(afterDraft?.editVersion ?? 0);
      if (!(afterVersion > beforeVersion)) throw new Error(`撤销后版本没有推进：${beforeVersion} → ${afterVersion}`);
      const valueAfterUndo = afterDraft?.ds?.answerKey?.[slotId] ?? null;
      if (JSON.stringify(valueAfterUndo) !== JSON.stringify(valueBeforeUndo) && JSON.stringify(valueAfterUndo) !== JSON.stringify({ kind: "unresolved" })) {
        throw new Error(`撤销后 ${slotId} 没有回到修正前的值：修正前 ${JSON.stringify(valueBeforeUndo)}，撤销后 ${JSON.stringify(valueAfterUndo)}`);
      }
      await reopenWorkspace();
      const reopenedState = await cardState(target.decisionId);
      const reopenedView = await readDecision();
      return {
        ...diagnostics,
        valueBeforeUndo,
        valueAfterUndo,
        versionBefore: beforeVersion,
        versionAfter: afterVersion,
        afterReopen: {
          panel: reopenedState,
          status: [...reopenedView.actionable, ...reopenedView.autoApplied].find((i) => i.decisionId === target.decisionId)?.status ?? null,
        },
      };
    });
  }

  // ═══════════════ A3/A4 联调段（本轮任务书第 6 条）═══════════════════════
  //
  // 为什么必须单独一段：受控服务此前**只**会返回 outline 样本。A3（原文件核验）与
  // A4（分歧裁决）请求拿到的是同一份 outline，被网关校验器整份拒绝
  // （`MODEL_INVALID_OUTPUT`），链状态退化成 `partial`/`unusable`。
  // 所以「受控服务跑通」这句话当时**不覆盖** A3/A4 —— 把它当作 A3/A4 已联通的证据是错的。
  // 现在服务按 prompt 标记分派（`task=verify_source_answers` / `task=adjudicate_divergence`），
  // 本段验证的是前端侧要负责的那一半：
  //   1. A3/A4 请求**确实到达了服务**（服务自己的 POST 日志是旁证，不是我们的推断）；
  //   2. 四路链状态与界面那句话**一致**，尤其「部分完成」不能显示成「校验完成」；
  //   3. 用户先改过之后，迟到的模型结果**不覆盖**用户内容；
  //   4. A3 部分返回 / 调用失败时，界面如实说「部分内容尚未完成校验」，且不泄露原因码。

  const requestsForThisBatch = () => report.service.requestLines;
  const taskCounts = () => {
    const lines = requestsForThisBatch();
    return {
      total: lines.length,
      outline: lines.filter((l) => l.includes("task=generate_pdf_reading_outline")).length,
      a3: lines.filter((l) => l.includes("task=verify_source_answers")).length,
      a4: lines.filter((l) => l.includes("task=adjudicate_divergence")).length,
    };
  };

  // ---- 场景 4：A3/A4 请求真的到达了受控服务 ----
  //
  // 三种结果必须分开，绝不能合成一个 `not-executable`：
  //   (a) 应用**根本没发起** A3/A4（作业目录里连输入缓存都没有）→ 前提不成立，记 not-executable；
  //   (b) 应用**发起了**（有输入缓存）但服务一个 POST 都没收到 → **链路缺陷**，必须记 FAILED；
  //   (c) 服务收到了 → 断言形状。
  // (b) 是真实发生过的情形：A3 的输入缓存落在 17:13:20，而受控服务全程零 POST、
  // 作业目录里也没有 `llm-calls.jsonl`、没有 `-output-`。旧版脚本会把它记成
  // 「本夹具里没有可核验的槽位」——那是**假解释**（请求里明明带了 14 个槽位）。
  const gatewayTraces = llmGatewayTraces(itemId);
  report.service.gatewayTraces = gatewayTraces;
  const afterFirstBatch = taskCounts();
  report.service.taskCounts = afterFirstBatch;
  const attemptedA3 = gatewayTraces.byCommand.verify_source_answers?.input ?? 0;
  const attemptedA4 = gatewayTraces.byCommand.adjudicate_divergence?.input ?? 0;
  if (afterFirstBatch.a3 === 0 && afterFirstBatch.a4 === 0 && attemptedA3 + attemptedA4 === 0) {
    notExecutable(
      "a3-a4-requests-reach-service",
      `受控服务只收到 outline 请求（total=${afterFirstBatch.total}），且作业目录里没有 A3/A4 的输入缓存`
        + `（cache/llm 存在=${gatewayTraces.exists}，内容=${JSON.stringify(gatewayTraces.files)}）`
        + "——即应用**没有发起**这两条模型通道，属于前提不成立，不是链路坏了。"
        + `本次链状态：${JSON.stringify(chainSnapshot(decision))}`
    );
  } else {
    await scenario("a3-a4-requests-reach-service", async () => {
      if (attemptedA3 > 0 && afterFirstBatch.a3 === 0) {
        throw new Error(
          `A3（原文件核验）在应用侧已经发起（输入缓存 ${attemptedA3} 份），却从未到达受控服务：`
            + `服务收到的请求=${JSON.stringify(afterFirstBatch)}；`
            + `作业目录 llm-calls.jsonl 存在=${gatewayTraces.callRecordExists}，`
            + `网关痕迹=${JSON.stringify(gatewayTraces.byCommand)}。`
            + "输入缓存有、调用记录与输出都没有、服务端零 POST，说明这次调用既没有结果也无法诊断。"
            + "根因看 `report.appPanics`（应用日志里的 panic；本仓实测为 tokio 运行时"
            + "「Cannot drop a runtime in a context where blocking is not allowed」）。"
        );
      }
      if (afterFirstBatch.a3 === 0) {
        throw new Error(`A3（原文件核验）没有向受控服务发出任何请求：counts=${JSON.stringify(afterFirstBatch)}`);
      }
      return { counts: afterFirstBatch, sample: report.service.requestLines.slice(-4), gatewayTraces };
    });
  }

  // ---- 场景 5：界面那句话必须与后端四路链状态一致 ----
  // 这是本轮前端契约适配的**核心断言**。用独立实现的规则算出「应该显示什么」，
  // 再和真实 DOM 里的那句话逐字比对；同时断言原因码没有泄漏进用户文案。
  await scenario("verification-status-matches-chains", async () => {
    const view = await readDecision();
    const snap = chainSnapshot(view);
    const pending = pendingCountOf(view);
    const expected = expectedStatusText({ ...snap, pendingCount: pending });
    const actual = await readStatusLine();
    if (actual !== expected) {
      throw new Error(`核验状态行与链状态不一致：期望 ${JSON.stringify(expected)}，实际 ${JSON.stringify(actual)}（chains=${JSON.stringify(snap)}，pending=${pending}）`);
    }
    // 「空列表 ≠ 核验成功」：有链没跑完时不得出现「没有发现需要处理的问题」。
    const unfinished = [snap.cloud, snap.source, snap.adjudication].some((s) => ["partial", "failed", "unavailable", "not_started", "skipped"].includes(s));
    if (unfinished && String(actual).includes("没有发现需要处理的问题")) {
      throw new Error(`有链路未完成却显示「没有发现需要处理的问题」：chains=${JSON.stringify(snap)}`);
    }
    // 原因码只作内部分类，绝不进用户文案。
    for (const code of [snap.sourceReasonCode, snap.adjudicationReasonCode, snap.cloudReasonCode]) {
      if (code && String(actual).includes(code)) {
        throw new Error(`核验状态行里出现了内部原因码 ${code}：${JSON.stringify(actual)}`);
      }
    }
    return { chains: snap, pendingCount: pending, statusLine: actual };
  });

  // ---- 场景 6：用户编辑后，迟到的模型结果不覆盖用户内容 ----
  // 机制：批次带 `baseEditVersion`，用户编辑会把版本推上去。之后点「采用修正」时，
  // 后端必须按「用户已改过」处理（superseded），**不能**把建议值写回去。
  // 这一条与「撤销」互补：撤销保护的是「已经写入之后」，这条保护的是「写入之前」。
  await scenario("late-model-result-does-not-overwrite-user-edit", async () => {
    // 重跑一次拿一个干净的批次（前面的接受/撤销已经把目标项处理掉了）。
    await restartService(initialServiceMode, effectiveFixture);
    const clean = await rerunRecognition(decision?.batchId ?? null);
    if (!clean?.batchId) throw new Error("重跑后没有产出批次，无法验证迟到结果");
    const items = [...(clean.actionable ?? []), ...(clean.autoApplied ?? [])];
    const candidate = items.find((i) => i.status === "open" && i.proposedPatch && (i.target?.targetId ?? null));
    if (!candidate) {
      throw new Error(`新批次里没有「待确认且带可应用补丁」的候选，本场景前提不成立：${JSON.stringify(items.map((i) => ({ id: i.decisionId, status: i.status, hasPatch: Boolean(i.proposedPatch) })))}`);
    }
    const slotId = candidate.target.targetId;
    const before = await readDraft();
    const originalValue = before.ds?.answerKey?.[slotId] ?? null;
    // 造一个与建议值不同的用户值：形状跟随本地已有值，避免被当成「形状不合法」拒掉。
    const sentinel = originalValue?.kind === "option"
      ? { kind: "option", labels: ["A"], normalization: "ielts_default" }
      : { kind: "text", values: ["e2e-user-edit"], normalization: "ielts_default" };
    if (JSON.stringify(sentinel) === JSON.stringify(candidate.proposedPatch?.value)) {
      throw new Error("哨兵值与建议值相同，断言会变成空断言，拒绝执行");
    }
    const edit = await call("apply_editor_commands", {
      itemId,
      baseVersion: Number(before.editVersion ?? 0),
      requestId: `e2e-late-${Date.now()}`,
      commands: [{ op: "setAnswer", slotId, value: sentinel }],
    });
    if (!edit?.ok) throw new Error(`用户编辑没有被接受：${edit?.error}`);
    const afterEdit = await readDraft();
    const versionAfterEdit = Number(afterEdit.editVersion ?? 0);
    if (JSON.stringify(afterEdit.ds?.answerKey?.[slotId] ?? null) !== JSON.stringify(sentinel)) {
      throw new Error(`用户编辑没有写进权威稿：${JSON.stringify(afterEdit.ds?.answerKey?.[slotId] ?? null)}`);
    }

    // 现在点「采用修正」。这是**迟到的**模型结果（批次基线早于用户修改）。
    await openPanel();
    const acceptExists = await session.evaluate(`!!document.querySelector('[data-testid="workspace-recognition-accept-${candidate.decisionId}"]')`);
    if (acceptExists) {
      await session.clickSelector(`[data-testid="workspace-recognition-accept-${candidate.decisionId}"]`);
      await sleep(2500);
    }
    const afterAccept = await readDraft();
    const valueAfterAccept = afterAccept.ds?.answerKey?.[slotId] ?? null;
    const noticeText = await session.evaluate(
      `(() => { const el = document.querySelector('[data-testid="workspace-recognition-notice"]'); return el ? el.innerText.replace(/\\s+/g,' ').trim() : null; })()`
    );
    if (JSON.stringify(valueAfterAccept) !== JSON.stringify(sentinel)) {
      throw new Error(
        `迟到的模型结果覆盖了用户内容：用户写入 ${JSON.stringify(sentinel)}，`
        + `点「采用修正」后变成 ${JSON.stringify(valueAfterAccept)}（建议值 ${JSON.stringify(candidate.proposedPatch?.value)}）`
      );
    }
    return {
      decisionId: candidate.decisionId,
      slotId,
      sentinel,
      proposedValue: candidate.proposedPatch?.value ?? null,
      acceptButtonWasPresent: Boolean(acceptExists),
      valueAfterAccept,
      versionAfterEdit,
      versionAfterAccept: Number(afterAccept.editVersion ?? 0),
      noticeText,
    };
  });

  // ---- 场景 7：A3「部分返回」不能被显示成「校验完成」----
  await scenario("a3-partial-not-reported-as-complete", async () => {
    const health = await restartService("partial", effectiveFixture);
    const partialDecision = await rerunRecognition(decision?.batchId ?? null);
    const snap = chainSnapshot(partialDecision);
    const pending = pendingCountOf(partialDecision);
    await openPanel();
    await sleep(1200);
    const actual = await readStatusLine();
    const expected = expectedStatusText({ ...snap, pendingCount: pending });
    report.partialMode = { health, chains: snap, pendingCount: pending, statusLine: actual, expected };
    if (snap.source !== "partial") {
      throw new Error(`mode=partial 下 A3 只回答了第一项，chains.source 应当是 partial，实际 ${snap.source}（原因码 ${String(snap.sourceReasonCode)}）`);
    }
    if (String(actual).includes("没有发现需要处理的问题")) {
      throw new Error(`部分返回被显示成核验完成：${JSON.stringify(actual)}`);
    }
    if (!String(actual).includes("部分内容尚未完成校验")) {
      throw new Error(`部分返回没有如实说明「部分内容尚未完成校验」：${JSON.stringify(actual)}`);
    }
    return { chains: snap, statusLine: actual, expected, pendingCount: pending };
  });

  // ---- 场景 8：A3「调用失败」同样不能被显示成「校验完成」----
  await scenario("a3-model-failure-not-reported-as-complete", async () => {
    const health = await restartService("fail", effectiveFixture);
    const failedDecision = await rerunRecognition(decision?.batchId ?? null);
    const snap = chainSnapshot(failedDecision);
    const pending = pendingCountOf(failedDecision);
    await openPanel();
    await sleep(1200);
    const actual = await readStatusLine();
    const expected = expectedStatusText({ ...snap, pendingCount: pending });
    report.failureMode = { health, chains: snap, pendingCount: pending, statusLine: actual, expected };
    if (snap.source !== "partial") {
      throw new Error(`模型调用失败时 chains.source 应当是 partial（模型通道失败），实际 ${snap.source}`);
    }
    const reason = String(snap.sourceReasonCode ?? "");
    if (!reason.startsWith("MODEL_")) {
      throw new Error(`模型调用失败的链原因码应当是 MODEL_*，实际 ${JSON.stringify(snap.sourceReasonCode)}`);
    }
    if (String(actual).includes("没有发现需要处理的问题")) {
      throw new Error(`模型调用失败被显示成核验完成：${JSON.stringify(actual)}`);
    }
    // 原因码只作内部分类：它出现在链状态里，但**不得**出现在用户文案里。
    if (String(actual).includes(reason)) {
      throw new Error(`用户文案里泄露了内部原因码 ${reason}：${JSON.stringify(actual)}`);
    }
    // 云端不可用时编辑必须照常可用（降级不能拖住编辑）。
    const stillEditable = await session.evaluate(`!!document.querySelector('[data-testid="exam-canvas-v2-author"]')`);
    if (!stillEditable) throw new Error("A3 失败后编辑区消失了 —— 降级没有保住编辑能力");
    return { chains: snap, statusLine: actual, expected, pendingCount: pending, stillEditable };
  });

  report.verdict = "pending";
}

try {
  await main();
} catch (error) {
  report.cannotRun = error instanceof CannotRunError;
  report.fatal = { name: error.name, message: String(error.message), appOutput: error.appOutput ?? null };
  console.error(`[controlled] ${report.cannotRun ? "CANNOT-RUN" : "FAIL"} ${error.message}`);
} finally {
  if (session) {
    if (!keep) await session.screenshot("final").catch(() => {});
    const closed = await session.close({ keep });
    // 完整应用日志落盘（不截断）：截断会把 freeze/cloud 失败行切掉。
    report.appOutput = closed.appOutput ?? null;
    // panic 单独提到顶层：它是「A3 为什么没到达服务」的可读根因，
    // 埋在 app-output.log 里等于没有（见 `extractAppPanics` 的注释）。
    report.appPanics = extractAppPanics(closed.appOutput);
    report.appProcessExitCode = closed.exitCode;
    if (closed.appOutput) fs.writeFileSync(path.join(runDir, "app-output.log"), closed.appOutput);
  }
  if (serviceChild) {
    const out = report.service.stdout ? report.service.stdout() : "";
    delete report.service.stdout;
    fs.writeFileSync(path.join(runDir, "controlled-service.log"), out ?? "");
    serviceChild.kill();
    report.service.stopped = true;
  }
  report.finishedAt = new Date().toISOString();
  const verdict = report.cannotRun
    ? { verdict: "cannot-run", exitCode: 3, reason: "环境或构建不满足运行条件，见 fatal", passed: [], failed: [], notExecutable: [] }
    : computeScenarioVerdict({ scenarios: report.scenarios });
  report.verdict = verdict.verdict;
  report.exitCode = verdict.exitCode;
  report.verdictReason = verdict.reason;
  report.summary = { passed: verdict.passed, failed: verdict.failed, notExecutable: verdict.notExecutable };
  const file = writeReport(runDir, report);
  console.log(`[controlled] verdict=${report.verdict} exit=${report.exitCode} reason=${report.verdictReason}`);
  console.log(`[controlled] report=${file}`);
  console.log(`[controlled] scenarios: ${report.scenarios.map((s) => `${s.name}:${s.status}`).join(" | ") || "(无)"}`);
  process.exit(report.exitCode ?? 1);
}
