#!/usr/bin/env node
/**
 * 发布阻塞**归因探针**（诊断，不是验收）。
 *
 * 要回答的唯一问题：`get_publish_preflight` 把一份题稿拦下，到底是
 *   (A) **数据问题** —— 把数据按产品自己的编辑通道改对之后，门禁会自动放行；还是
 *   (B) **门禁代码问题** —— 无论数据怎么改都不放行（resolution 盲区 / 结构性判定）。
 *
 * 为什么必须先问这个：任务书的本轮第一优先级是「先解除真实发布阻塞」。
 * 若答案是 (A)，那么「导入 → 人工修正 → 发布 → 学生端」这条链**不需要**等门禁修复就能跑通，
 * 后端要修的只是「确认/忽略」那条支路；若答案是 (B)，则发布在门禁修好之前根本不可能发生，
 * 任务四必须如实记为未完成。这个判断决定了后续所有工作，因此值得一次专门的运行。
 *
 * 本探针**只做诊断**，不做验收，因此：
 *   - 不产生 verdict passed/failed；输出的是 `attribution` 归因结论；
 *   - 明确标注 `isAcceptanceEvidence: false`；
 *   - 修改数据时记录「改了什么」（`changes` 数组），并区分「有 UI 入口」与「没有 UI 入口」的槽位。
 *
 * 做法（全部走真实通道）：
 *   1. 真实导入夹具 → 等权威稿落盘 → 进工作区；
 *   2. 读基线：`ds.quality`（state / hardFailures / issues 含 actions）、`ds.answerSlots`、`ds.answerKey`、
 *      `get_publish_preflight` 的 blockers；
 *   3. **UI 可用性普查**：对每个答案位查 DOM 里有没有可交互的输入控件。
 *      这一项直接对应任务二「必须修改内容的问题提供定位和对应编辑入口」——
 *      没有控件就是「用户看得到问题、却无处可改」；
 *   4. 用**真实 DOM 交互**（点选/键入）把答案改成「与 interaction 种类相符」的值；
 *   5. 重读 quality + preflight，逐条比对哪些 blocker 消失了、哪些还在。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-publish-unblock-probe.mjs [--pdf <path>] [--diagnostic-args] [--tolerate-concurrent-edits]
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  CDP_CHANNEL_LABEL,
  CDP_CHANNEL_NOTE,
  CannotRunError,
  assertBuildFresh,
  gitHead,
  gitWorktreeClean,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  sleep,
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const fixtureIdx = process.argv.indexOf("--pdf");
const fixturePath = path.resolve(
  fixtureIdx >= 0 ? process.argv[fixtureIdx + 1] : path.join(repoRoot, "fixtures", "parser", "complex-reading.pdf")
);
const isPdf = /\.pdf$/i.test(fixturePath);
const diagnosticArgsRequested = process.argv.includes("--diagnostic-args");
const extraArgs = diagnosticArgsRequested ? "--no-sandbox --disable-gpu" : "";
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `run-publish-attribution-${new Date().toISOString().replace(/[:.]/g, "-")}`);

/**
 * 独立人工答案表（按题号）。
 *
 * 「独立」的含义：这张表是本探针作者**从 passage 正文独立推导**出来的，不是从夹具自带的
 * `## Answers` 段读出来的，也不是从识别结果里抄的。夹具自带的答案段与本题表一致，但那只构成
 * 一次交叉核对，不构成来源。
 *
 * 推导（passage 正文）：
 *   "A short passage about tidal diaries used by harbour researchers.
 *    The diaries record daily observations and help compare storms."
 *   1 The diaries record daily observations.      → 正文逐字一致            → TRUE
 *   2 The passage says storms are never compared. → 正文是 "help compare storms" → FALSE
 *   3 The records are used by harbour researchers.→ 正文 "used by harbour researchers" → TRUE
 *   4 The passage is about tidal _____.           → "tidal diaries"          → diaries
 *   5 The records are called _____.               → "diaries"                → diaries
 */
const INDEPENDENT_ANSWER_TABLE = {
  "complex-reading": { q1: "TRUE", q2: "FALSE", q3: "TRUE", q4: "diaries", q5: "diaries" },
};

const report = {
  task: "publish-blocker-attribution",
  isAcceptanceEvidence: false,
  purpose:
    "判定发布阻塞属于数据问题（改对数据即放行）还是门禁代码问题（改数据也不放行）。这是诊断，不是验收。",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  diagnosticRun: Boolean(extraArgs),
  runProfile: extraArgs ? "cdp-diagnostic" : "cdp-default",
  securityArgs: extraArgs ? extraArgs.split(/\s+/).filter(Boolean) : [],
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
  },
  baseline: null,
  uiAvailability: null,
  changes: [],
  after: null,
  attribution: null,
};

let session = null;
let itemId = null;

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

async function readPreflight() {
  const r = await call("get_publish_preflight", { jobId: itemId });
  if (!r?.ok) throw new Error(`get_publish_preflight 失败：${r?.error}`);
  return r.value;
}

/**
 * 把 quality 摘要成可比较的形状。
 *
 * 注意 `suggestedActions` 才是后端 `issue()` 真正写出的字段名（quality.rs:3668）。
 * 早先这里误读成 `actions`，于是所有问题都显示成「没有建议动作」——那是探针自己的假阴性，
 * 不是产品的缺陷。这里两个名字都读，并且保留原始对象，避免再次因为字段名猜错而得出反向结论。
 */
function summarizeQuality(ds) {
  const quality = ds?.quality ?? {};
  const issues = Array.isArray(quality.issues) ? quality.issues : [];
  return {
    state: quality.state ?? null,
    hardFailures: Array.isArray(quality.hardFailures) ? [...quality.hardFailures] : [],
    documentScore: quality.documentScore ?? null,
    sourceCoverage: quality.sourceCoverage ?? null,
    compilerProbes: quality.compilerProbes ?? null,
    recognitionBlockers: ds?.recognitionBlockers ?? null,
    issues: issues.map((issue) => ({
      issueId: issue.issueId ?? null,
      code: issue.code ?? null,
      severity: issue.severity ?? null,
      targetId: issue.targetId ?? null,
      targetType: issue.targetType ?? null,
      resolution: issue?.details?.resolution ?? null,
      suggestedActions: Array.isArray(issue.suggestedActions) ? issue.suggestedActions : [],
      actions: Array.isArray(issue.actions) ? issue.actions : [],
      message: typeof issue.message === "string" ? issue.message.slice(0, 200) : null,
      raw: issue,
    })),
  };
}

function summarizePreflight(preflight) {
  const blockers = Array.isArray(preflight?.blockers) ? preflight.blockers : [];
  return {
    passed: preflight?.passed ?? null,
    blockerCodes: [...new Set(blockers.map((b) => b.code))],
    blockers: blockers.map((b) => ({
      code: b.code ?? null,
      targetId: b.targetId ?? null,
      action: b.action ?? null,
      internal: b.internal ?? null,
      userMessage: typeof b.userMessage === "string" ? b.userMessage.slice(0, 120) : null,
    })),
    warnings: Array.isArray(preflight?.warnings) ? preflight.warnings.slice(0, 10) : [],
  };
}

/** 递归找出内容节点里的第一个 text 节点 id（说明文字通常就是它）。 */
function firstTextNodeId(nodes) {
  let found = null;
  const walk = (value) => {
    if (found || !value || typeof value !== "object") return;
    if (Array.isArray(value)) { for (const entry of value) walk(entry); return; }
    if (value.type === "text" && typeof value.id === "string") { found = value.id; return; }
    for (const entry of Object.values(value)) walk(entry);
  };
  walk(nodes);
  return found;
}

/** 每个答案位在 DOM 里到底有没有可交互控件；没有就是「用户无处可改」。 */
async function surveyUiAvailability(ds) {
  const slotIds = Object.keys(ds?.answerSlots ?? {});
  return session.evaluate(`(() => {
    const slotIds = ${JSON.stringify(slotIds)};
    return slotIds.map((slotId) => {
      const all = [...document.querySelectorAll('input,select,textarea')]
        .filter((el) => el.getAttribute('name') === slotId || (el.getAttribute('aria-label') || '').length > 0);
      const named = all.filter((el) => el.getAttribute('name') === slotId);
      const visible = named.filter((el) => {
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      });
      return {
        slotId,
        controlCount: named.length,
        visibleControlCount: visible.length,
        kinds: [...new Set(named.map((el) => el.tagName.toLowerCase() + ':' + (el.getAttribute('type') || 'text')))],
        labels: [...new Set(named.map((el) => el.getAttribute('value') || ''))].filter(Boolean).slice(0, 12),
      };
    });
  })()`);
}

async function waitForVersionAbove(previous) {
  const deadline = Date.now() + 40000;
  let current = null;
  while (Date.now() < deadline) {
    current = await readDraft();
    if (Number(current?.editVersion ?? 0) > previous) return current;
    await sleep(600);
  }
  return current;
}

async function main() {
  const tolerateConcurrentEdits = process.argv.includes("--tolerate-concurrent-edits");
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits });
  report.identity.buildFresh = {
    ok: true,
    exeMtime: new Date(fresh.exeMs).toISOString(),
    srcNewest: new Date(fresh.srcNewestMs).toISOString(),
    srcNewestPath: fresh.srcNewestPath,
    toleratedConcurrentEdits: fresh.tolerated ?? [],
  };
  report.identity.exeSha256 = sha256File(exePath);
  if (!fs.existsSync(fixturePath)) throw new CannotRunError(`夹具不存在：${fixturePath}`);
  report.identity.fixtureSha256 = sha256File(fixturePath);

  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  const staged = path.join(runDir, "pdfs", path.basename(fixturePath));
  fs.copyFileSync(fixturePath, staged);

  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    appEnv: isPdf ? {} : { PDF2TEST_AUTOMATION_SOURCE_FILES: staged },
  });
  report.identity.browserArgs = session.browserArgs;

  // ---- 准备：题库 → 导入 → 工作区 ----
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });
  await session.evaluate(`(() => { window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false })); location.hash = "#/library"; return true; })()`);
  await session.cdp.send("Page.reload", {}, 30000).catch(() => {});
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-after-reload" });

  const before = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
  await session.clickSelector(isPdf ? '[data-testid="import-pick-folder"]' : '[data-testid="import-pick-files"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked-files" });
  await session.clickSelector('[data-testid="import-start"]');
  const importDeadline = Date.now() + 90000;
  while (Date.now() < importDeadline && !itemId) {
    const ids = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
    itemId = (ids ?? []).find((id) => !(before ?? []).includes(id)) ?? null;
    if (!itemId) await sleep(1000);
  }
  if (!itemId) throw new CannotRunError("导入后未出现新的题库行");
  report.identity.itemId = itemId;

  const draftDeadline = Date.now() + 120000;
  let draft = null;
  while (Date.now() < draftDeadline) {
    draft = await readDraft();
    if (draft?.ds) break;
    await sleep(2000);
  }
  if (!draft?.ds) throw new CannotRunError("本地稿在 120s 内没有落盘（ds 为空）");

  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-open" });
  await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-author"]')`, { timeoutMs: 40000, label: "author-canvas" });

  // ---- 基线 ----
  const preflightBefore = await readPreflight();
  report.baseline = {
    editVersion: Number(draft.editVersion ?? 0),
    quality: summarizeQuality(draft.ds),
    preflight: summarizePreflight(preflightBefore),
    answerSlots: Object.fromEntries(
      Object.entries(draft.ds.answerSlots ?? {}).map(([slotId, slot]) => [slotId, {
        interaction: slot?.interaction ?? null,
        hostNodeId: slot?.hostNodeId ?? null,
        hostType: slot?.hostType ?? null,
        displayLabel: slot?.displayLabel ?? null,
      }])
    ),
    answerKey: draft.ds.answerKey ?? {},
    instructionSignatures: Object.fromEntries(
      (draft.ds.taskGroups ?? []).map((task) => [task.taskId, task.instructionSignature ?? null])
    ),
    optionBanks: Object.fromEntries(
      (draft.ds.taskGroups ?? []).map((task) => [task.taskId, (task.optionBank?.options ?? []).map((o) => o.label)])
    ),
  };
  console.log("[probe] baseline quality.state =", report.baseline.quality.state);
  console.log("[probe] baseline hardFailures =", JSON.stringify(report.baseline.quality.hardFailures));
  console.log("[probe] baseline preflight.passed =", report.baseline.preflight.passed);
  console.log("[probe] baseline blockerCodes =", JSON.stringify(report.baseline.preflight.blockerCodes));

  // ---- UI 可用性普查 ----
  report.uiAvailability = await surveyUiAvailability(draft.ds);
  for (const row of report.uiAvailability ?? []) {
    console.log(`[probe] ui ${row.slotId}: controls=${row.controlCount} visible=${row.visibleControlCount} kinds=${JSON.stringify(row.kinds)}`);
  }

  // ---- 真实 DOM 交互：把答案改成与 interaction 种类相符的值 ----
  const tableKey = path.basename(fixturePath).replace(/\.(pdf|docx)$/i, "");
  const answerTable = INDEPENDENT_ANSWER_TABLE[tableKey] ?? {};
  const slots = Object.entries(draft.ds.answerSlots ?? {});
  let version = Number(draft.editVersion ?? 0);

  for (const [slotId, slot] of slots) {
    const desired = answerTable[slotId] ?? null;
    const interaction = slot?.interaction ?? null;
    const isText = interaction === "text";
    const selector = `input[name="${slotId}"]`;
    const hasControl = await session.evaluate(`(() => {
      const el = document.querySelector(${JSON.stringify(selector)});
      if (!el) return false;
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    })()`);
    if (!hasControl) {
      report.changes.push({ slotId, interaction, action: "no-ui-entry", note: "DOM 里没有该答案位的可交互控件，用户无法从界面修改" });
      console.log(`[probe] ${slotId}: 无 UI 入口，跳过`);
      continue;
    }
    if (isText) {
      const value = desired ?? "probe";
      await session.typeInto(selector, value);
      const next = await waitForVersionAbove(version);
      const applied = next?.ds?.answerKey?.[slotId] ?? null;
      report.changes.push({
        slotId, interaction, action: "type-text", value,
        selector, versionBefore: version, versionAfter: Number(next?.editVersion ?? 0),
        answerAfter: applied, saved: JSON.stringify(applied?.values ?? null) === JSON.stringify([value]),
      });
      version = Number(next?.editVersion ?? version);
    } else {
      // 选项位：优先用独立答案表里的标签，否则**从已渲染的控件本身**取可用标签。
      //
      // 早先这里回退到「第一个 optionBank 的标签」，而 `optionBanks` 里 group-1/group-3 是空数组，
      // 于是 9 个槽位被本探针自己跳过，却被记成了「没有 UI 入口」——那是探针的假阴性。
      // 控件已经渲染出来了，它自己就说明了合法标签是什么，直接问 DOM 最可靠。
      const domLabels = await session.evaluate(
        `[...document.querySelectorAll('input[name="${slotId}"]')].map(el => el.getAttribute('value')).filter(Boolean)`
      );
      const label = desired ?? (Array.isArray(domLabels) ? domLabels[0] : null) ?? null;
      if (!label) {
        report.changes.push({ slotId, interaction, action: "no-option-label", note: `DOM 里也没有可用标签：${JSON.stringify(domLabels)}` });
        continue;
      }
      const radio = `input[name="${slotId}"][value="${label}"]`;
      const hasRadio = await session.evaluate(`!!document.querySelector(${JSON.stringify(radio)})`);
      if (!hasRadio) {
        report.changes.push({ slotId, interaction, action: "no-option-control", label, note: `没有 value=${label} 的单选项控件` });
        continue;
      }
      await session.clickSelector(radio);
      const next = await waitForVersionAbove(version);
      const applied = next?.ds?.answerKey?.[slotId] ?? null;
      report.changes.push({
        slotId, interaction, action: "click-option", label,
        selector: radio, versionBefore: version, versionAfter: Number(next?.editVersion ?? 0),
        answerAfter: applied, saved: JSON.stringify(applied?.labels ?? null) === JSON.stringify([label]),
      });
      version = Number(next?.editVersion ?? version);
    }
  }

  // ---- 改完之后重读 ----
  await sleep(1500);
  const afterDraft = await readDraft();
  const preflightAfter = await readPreflight();
  report.after = {
    editVersion: Number(afterDraft.editVersion ?? 0),
    quality: summarizeQuality(afterDraft.ds),
    preflight: summarizePreflight(preflightAfter),
    answerKey: afterDraft.ds?.answerKey ?? {},
  };
  console.log("[probe] after quality.state =", report.after.quality.state);
  console.log("[probe] after hardFailures =", JSON.stringify(report.after.quality.hardFailures));
  console.log("[probe] after preflight.passed =", report.after.preflight.passed);
  console.log("[probe] after blockerCodes =", JSON.stringify(report.after.preflight.blockerCodes));

  // ---- 阶段 B：resolution 支路（把所有 blocking 问题标成 ignored）----
  //
  // 这一阶段直接检验任务书交接点一的关切：`resolution` 到底能不能让门禁放行？
  // 注意：这是**诊断**，不是「把门禁改松」的提议。它只回答「现状是什么」。
  // 若标成 ignored 之后门禁仍然拦下，说明 resolution 在决定性路径上被忽略（与静态审读一致）；
  // 若放行了，说明用户确实可以靠「确认」解决问题，问题处理能力已经有了落点。
  const blockingIssues = (afterDraft.ds?.quality?.issues ?? []).filter((issue) => issue.severity === "blocking");
  const resolutionPhase = { attempted: false, blockingIssueCount: blockingIssues.length, issueIds: [], after: null };
  if (preflightAfter?.passed !== true && blockingIssues.length > 0) {
    resolutionPhase.attempted = true;
    resolutionPhase.issueIds = blockingIssues.map((issue) => issue.issueId);
    const baseVersion = Number(afterDraft.editVersion ?? 0);
    const applied = await call("apply_editor_commands", {
      itemId,
      baseVersion,
      requestId: `e2e-probe-resolve-${Date.now()}`,
      commands: blockingIssues.map((issue) => ({
        op: "resolveIssue", issueId: issue.issueId, resolution: "ignored", note: "attribution probe: diagnostic only",
      })),
    });
    resolutionPhase.applyResult = applied?.ok ? "ok" : `error:${applied?.error}`;
    await sleep(2500);
    const resolvedDraft = await readDraft();
    const resolvedPreflight = await readPreflight();
    resolutionPhase.after = {
      editVersion: Number(resolvedDraft.editVersion ?? 0),
      quality: summarizeQuality(resolvedDraft.ds),
      preflight: summarizePreflight(resolvedPreflight),
    };
    console.log("[probe] resolution-phase applyResult =", resolutionPhase.applyResult);
    console.log("[probe] resolution-phase quality.state =", resolutionPhase.after.quality.state);
    console.log("[probe] resolution-phase hardFailures =", JSON.stringify(resolutionPhase.after.quality.hardFailures));
    console.log("[probe] resolution-phase preflight.passed =", resolutionPhase.after.preflight.passed);
    console.log("[probe] resolution-phase blockerCodes =", JSON.stringify(resolutionPhase.after.preflight.blockerCodes));
  }
  report.resolutionPhase = resolutionPhase;

  // ---- 阶段 C：按问题**自己给的建议动作**去改，问题会不会消失？ ----
  //
  // `WORD_LIMIT_UNPARSED` 的 suggestedActions 是 `["edit_text","confirm_table"]`。
  // 但它判定的是 `taskGroups[].instructionSignature.wordLimit`，而 `instructionSignature`
  // 只被 `setTaskType` / `setQuestionExpression` 改写，`replaceText` 不会重算它。
  // 于是「按建议去改说明文字」很可能是一条**死路**。本阶段就是实测这一点：
  // 若改完文字后 wordLimit 仍然是 null、问题仍然在，那么问题给出的补救动作是无效的 ——
  // 用户「看得到错误、却无法完成修复」，正是任务二要求验证的那件事。
  const wordLimitIssues = (afterDraft.ds?.quality?.issues ?? [])
    .filter((issue) => issue.severity === "blocking" && issue.code === "WORD_LIMIT_UNPARSED");
  const textEditPhase = { attempted: false, targetIds: wordLimitIssues.map((i) => i.targetId), edits: [], notes: [], after: null };
  if (wordLimitIssues.length > 0) {
    textEditPhase.attempted = true;
    for (const issue of wordLimitIssues) {
      const groupId = issue.targetId;
      const task = (afterDraft.ds?.taskGroups ?? []).find((t) => t.taskId === groupId);
      // 取该题组说明文字里的第一个 text 节点；`replaceText` 就是按 nodeId 改它。
      const firstTextId = firstTextNodeId(task?.instructions);
      const selector = firstTextId ? `[data-editor-id="${firstTextId}"]` : `[data-group-id="${groupId}"] .v2-instruction .v2-text`;
      const present = await session.waitFor(`!!document.querySelector(${JSON.stringify(selector)})`, { timeoutMs: 15000, label: `instruction-${groupId}` }).catch(() => false);
      if (!present) { textEditPhase.notes.push(`${groupId}: 找不到说明文字元素（${selector}）`); continue; }
      const original = await session.evaluate(`document.querySelector(${JSON.stringify(selector)}).innerText`);
      const next = `${original} Write ONE WORD ONLY.`;
      const versionBefore = Number((await readDraft()).editVersion ?? 0);

      // 路径 1：真实 UI 原位编辑。链脚本对 passage 文本要重试 4 次才稳，这里同样重试。
      //
      // 说明文字是很长的**行内** span（本夹具 670 字、跨多行）。`clickSelector` 点的是
      // 联合包围盒中心，对跨行行内元素来说那个点可能落在**空白区域**（某行较短时），
      // 于是点击落在父元素上、进不了编辑。所以这里交替尝试「首行靠左边缘」与「包围盒中心」，
      // 以免把「探针点偏了」误报成「产品不可编辑」。
      let uiEdited = false;
      const clickEdge = async () => {
        const box = await session.evaluate(`(() => {
          const el = document.querySelector(${JSON.stringify(selector)});
          if (!el) return null;
          el.scrollIntoView({ block: 'center', inline: 'center' });
          const r = el.getBoundingClientRect();
          return { x: r.left + 6, y: r.top + 8, w: r.width, h: r.height };
        })()`);
        if (!box || box.w === 0 || box.h === 0) return false;
        await session.clickAt(box.x, box.y);
        return true;
      };
      for (let attempt = 0; attempt < 4 && !uiEdited; attempt += 1) {
        const clicked = attempt % 2 === 0 ? await clickEdge() : Boolean(await session.clickSelector(selector).catch(() => false));
        if (!clicked) continue;
        const opened = await session.waitFor(
          `!!document.querySelector('textarea[aria-label="编辑题目文字"]')`,
          { timeoutMs: 6000, label: "inline-editor" }
        ).catch(() => false);
        if (!opened) continue;
        await session.evaluate(`(() => { const t = document.querySelector('textarea[aria-label="编辑题目文字"]'); t.select(); return true; })()`);
        await session.cdp.send("Input.insertText", { text: next });
        await session.pressKey("Enter", { code: "Enter", windowsVirtualKeyCode: 13 });
        uiEdited = true;
        textEditPhase.clickStrategy = attempt % 2 === 0 ? "first-line-edge" : "bbox-center";
      }

      // 路径 2（仅在 UI 打不开时）：走 `replaceText` 补丁 —— 这正是 UI 提交时产生的那个 patch，
      // 同一条 `apply_patch` 代码路径，所以对「保存路径是否重算 signature」这个命题等价。
      let fallbackApplied = null;
      if (!uiEdited) {
        const applied = await call("apply_editor_commands", {
          itemId,
          baseVersion: versionBefore,
          requestId: `e2e-probe-text-${Date.now()}`,
          commands: [{ op: "replaceText", nodeId: firstTextId, from: 0, to: Array.from(original).length, text: next }],
        });
        fallbackApplied = applied?.ok ? "ok" : `error:${applied?.error}`;
      }
      await session.waitFor(
        `(() => { const s = document.querySelector('[data-testid="workspace-save-state"]'); return !s || /已保存/.test(s.innerText); })()`,
        { timeoutMs: 40000, label: "saved-after-text-edit" }
      ).catch(() => {});
      const afterEdit = await readDraft();
      textEditPhase.edits.push({
        groupId, nodeId: firstTextId, original, next,
        path: uiEdited ? "real-ui-inline-editor" : "replaceText-patch",
        fallbackApplied,
        versionBefore,
        versionAfter: Number(afterEdit.editVersion ?? 0),
        signatureWordLimitAfter: afterEdit.ds?.taskGroups?.find((t) => t.taskId === groupId)?.instructionSignature?.wordLimit ?? null,
      });
    }
    await sleep(2000);
    const afterTextDraft = await readDraft();
    const afterTextPreflight = await readPreflight();
    textEditPhase.after = {
      editVersion: Number(afterTextDraft.editVersion ?? 0),
      quality: summarizeQuality(afterTextDraft.ds),
      preflight: summarizePreflight(afterTextPreflight),
      instructionSignatures: Object.fromEntries(
        (afterTextDraft.ds?.taskGroups ?? []).map((task) => [task.taskId, task.instructionSignature ?? null])
      ),
    };
    textEditPhase.wordLimitCleared = !(textEditPhase.after.quality.hardFailures ?? []).includes("WORD_LIMIT_UNPARSED");
    console.log("[probe] text-edit-phase edits =", JSON.stringify(textEditPhase.edits.map((e) => ({ g: e.groupId, path: e.path, v: `${e.versionBefore}->${e.versionAfter}`, wl: e.signatureWordLimitAfter }))));
    console.log("[probe] text-edit-phase notes =", JSON.stringify(textEditPhase.notes));
    console.log("[probe] text-edit-phase hardFailures =", JSON.stringify(textEditPhase.after.quality.hardFailures));
    console.log("[probe] text-edit-phase wordLimitCleared =", textEditPhase.wordLimitCleared);
  }
  report.textEditPhase = textEditPhase;

  // ---- 归因 ----
  const beforeCodes = new Set(report.baseline.preflight.blockerCodes);
  const afterCodes = new Set(report.after.preflight.blockerCodes);
  const cleared = [...beforeCodes].filter((code) => !afterCodes.has(code));
  const persisted = [...afterCodes];
  const resolvedHardFailures = report.baseline.quality.hardFailures.filter((c) => !report.after.quality.hardFailures.includes(c));
  const persistedHardFailures = report.after.quality.hardFailures;
  // 只有「控件本身不存在」才算没有 UI 入口；`no-option-label` 是探针没推出标签，不能算到产品头上。
  const noUiEntry = (report.changes ?? []).filter((c) => c.action === "no-ui-entry").map((c) => c.slotId);
  const probeSkipped = (report.changes ?? []).filter((c) => c.action === "no-option-label" || c.action === "no-option-control").map((c) => c.slotId);
  const notSaved = (report.changes ?? []).filter((c) => c.saved === false).map((c) => c.slotId);
  const ignoredClearedGate = report.resolutionPhase?.attempted === true
    && report.resolutionPhase?.after?.preflight?.passed === true;
  const resolutionBlind = report.resolutionPhase?.attempted === true
    && report.resolutionPhase?.after?.preflight?.passed !== true;

  report.attribution = {
    preflightPassedBefore: report.baseline.preflight.passed,
    preflightPassedAfterDataFix: report.after.preflight.passed,
    preflightPassedAfterIgnore: report.resolutionPhase?.after?.preflight?.passed ?? null,
    clearedBlockerCodes: cleared,
    persistedBlockerCodes: persisted,
    clearedHardFailures: resolvedHardFailures,
    persistedHardFailures,
    slotsWithoutUiEntry: noUiEntry,
    probeSkippedSlots: probeSkipped,
    notSavedSlots: notSaved,
    ignoredClearedGate,
    resolutionBlind,
    verdict: probeSkipped.length > 0
      // 探针自己没能把「能改的都改了」，那么「改完还是拦」不构成结论 —— 如实记为不确定。
      ? "inconclusive-probe-incomplete"
      : (report.after.preflight.passed === true
        ? "data-fix-unblocks-publish"
        : (ignoredClearedGate
          ? "confirmation-unblocks-publish"
          : (noUiEntry.length > 0 ? "blocked-without-ui-entry" : "data-fix-insufficient"))),
    note: probeSkipped.length > 0
      ? `本次探针没能覆盖全部槽位（自己跳过了 ${probeSkipped.join(", ")}），因此**不能**据此断言「改完数据也放行不了」。需要修正探针后重跑。`
      : (report.after.preflight.passed === true
        ? "按产品自己的编辑通道把数据改对之后，门禁放行 —— 发布阻塞属于**数据问题**，不必等门禁修复。"
        : (ignoredClearedGate
          ? "数据改对不够，但把 blocking 问题标成 ignored 之后门禁放行 —— 用户的「确认」动作确实能让发布继续，问题处理能力有落点。"
          : (noUiEntry.length > 0
            ? `即使用户把能改的都改了，仍有 blocker 残留，且这些题位在界面上没有可交互控件（${noUiEntry.join(", ")}）—— 用户看得到问题但无处可改。`
            : "即使用户把能改的都改了、并把所有 blocking 问题标成 ignored，门禁仍然拦下 —— 需要判定是门禁 resolution 盲区还是结构性判定，属后端门禁职责。"))),
  };
  console.log("[probe] attribution =", report.attribution.verdict);
  console.log("[probe] ignoredClearedGate =", ignoredClearedGate, " resolutionBlind =", resolutionBlind);
  console.log("[probe] cleared =", JSON.stringify(cleared));
  console.log("[probe] persisted =", JSON.stringify(persisted));
  console.log("[probe] slotsWithoutUiEntry =", JSON.stringify(noUiEntry));
  console.log("[probe] probeSkippedSlots =", JSON.stringify(probeSkipped));
  console.log("[probe] notSavedSlots =", JSON.stringify(notSaved));
}

let fatal = null;
try {
  await main();
} catch (error) {
  fatal = {
    kind: error instanceof CannotRunError ? "cannot-run" : "error",
    message: String(error?.message ?? error),
    appOutputTail: typeof error?.appOutput === "string" ? error.appOutput.slice(-3000) : undefined,
  };
  report.fatal = fatal;
  console.error(`[probe] ${fatal.kind}: ${fatal.message}`);
}

try {
  if (session) {
    report.identity.screenshots = {
      final: await session.screenshot("final"),
    };
    const closed = await session.close({ keep });
    report.appProcessExitCode = closed.exitCode;
    report.appOutputTail = closed.appOutput.slice(-4000);
  }
} catch (error) {
  report.closeError = String(error?.message ?? error);
}

report.finishedAt = new Date().toISOString();
writeReport(runDir, report);
console.log(`[probe] report: ${path.join(runDir, "report.json")}`);
process.exit(fatal ? (fatal.kind === "cannot-run" ? 3 : 1) : 0);
