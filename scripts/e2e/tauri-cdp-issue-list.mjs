#!/usr/bin/env node
/**
 * 问题列表的真实界面校验（WebView2 CDP 通道）——**任务卡版本**。
 *
 * 为什么单独一个脚本：R9 改了两处**用户可见**的行为，但当时只有单测 + 真实产物回放，
 * 没有在真实应用里看过一眼。任务书要求「用户能完成修复，而不只是看到错误」，
 * 那就必须在真实 DOM 上验证，而不是只在纯函数上验证。
 *
 * ── 本轮（R14）改了什么，以及本脚本为什么跟着重写 ────────────────────────────
 *
 * 上一版脚本断言的是「逐条原始问题行的渲染行数 = 独立算法期望」。本轮问题列表改成
 * **用户任务卡**：连续缺答并成一个题号区间、同一题组的内部问题并成一条、泛化行在
 * 原因被完整表达后隐藏。行数对不上是**设计如此**，旧断言全部失效——但失效不等于
 * 要求放宽：原来那两条互相牵制的硬要求（**同一问题不重复显示** + **不同问题不被隐藏**）
 * 必须在新形状下继续被证明，所以断言换成：
 *
 *   1. **合并真的发生了** —— 任务数 < 门禁原始阻断条数，且 `data-merged-rows` > 0。
 *      这条挡的是「其实没合并、只是行数恰好少」的假绿。
 *   2. **一个根因都没被吞** —— 门禁里每个**带具体目标**的根因码，以及本地闭包报出的
 *      每个根因码，都必须出现在某张任务卡的 `data-task-covers` 里。
 *      这是「不同问题不被隐藏」在新形状下的等价命题（行不再逐条渲染，但归属可查）。
 *   3. **泛化行不冒充任务** —— `QUALITY_NOT_READY` / `QUALITY_HARD_FAILURE` 是汇总，
 *      不得出现在任何任务的 `data-task-covers` 里，也不得单独渲染成一张卡。
 *   4. **每条任务都有真按钮** —— 每张卡至少一个 `button[data-action-id]`，动作只允许是
 *      `fill-answer` / `view-source` / `retry-recognition`；不存在「确认」「忽略」这类
 *      点了不改变门禁结果的按钮（本轮任务书第三节明确禁止）。
 *   5. **按钮真的有作用** —— 三类动作各点一次，各断言一个**可观察的真实后果**：
 *      `fill-answer` → 题面滚动到目标，或如实给出「不在题面上」（不允许静默无反应）；
 *      `view-source` → 原文件抽屉真的打开；
 *      `retry-recognition` → 真的把这道题重新加入识别队列（界面出现「已重新加入识别队列」）。
 *      这一条放在最后执行，因为它会重启识别，会污染后续断言。
 *   6. **界面里没有内部术语** —— 问题码、`v1/v2/v3`、`slot`、`schema`、`reasonCode`、
 *      `batchId`、`editVersion` 都不得出现在任务卡的可见文本里（本轮任务书第一节）。
 *
 * 期望值仍用**独立算法**算出（不复用被测实现），避免自己测自己。
 *
 * 判定：各条都成立 → `passed`(0)；任一条不成立 → `failed`(1)；
 * 环境不满足 → `cannot-run`(3)。**没有** not-executable 这一档：
 * 本脚本的前提（有一份被门禁拦下的题稿）在任何真实夹具下都成立。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-issue-list.mjs [--pdf <path>] [--diagnostic-args] [--keep]
 */

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
import { EXIT_CODES } from "./lib/chain-verdict.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const fixtureIdx = process.argv.indexOf("--pdf");
const fixturePath = path.resolve(
  fixtureIdx >= 0 ? process.argv[fixtureIdx + 1] : path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf")
);
const isPdf = /\.pdf$/i.test(fixturePath);
const extraArgs = process.argv.includes("--diagnostic-args") ? "--no-sandbox --disable-gpu" : "";
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `run-issue-list-${new Date().toISOString().replace(/[:.]/g, "-")}`);

const report = {
  task: "issue-list-real-ui-verification",
  shape: "user-task-cards",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  diagnosticRun: Boolean(extraArgs),
  runProfile: extraArgs ? "cdp-diagnostic" : "cdp-default",
  securityArgs: extraArgs ? extraArgs.split(/\s+/).filter(Boolean) : [],
  startedAt: new Date().toISOString(),
  runDir,
  identity: { exePath, exeSha256: null, fixturePath, fixtureSha256: null, commit: gitHead(repoRoot), worktreeClean: gitWorktreeClean(repoRoot), buildFresh: null },
  gate: null,
  rendered: null,
  assertions: [],
  verdict: "failed",
  exitCode: 1,
};

let session = null;
let itemId = null;

async function call(command, args) {
  const wrapped = command === "apply_recognition_decisions" || command === "apply_editor_commands";
  const r = await session.invoke(command, wrapped ? { input: args } : args);
  if (r && r.__noInvoke) throw new Error("invoke 不可用");
  return r;
}

function assert(name, ok, detail) {
  report.assertions.push({ name, ok: Boolean(ok), detail: detail ?? null });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${name}${detail ? ` :: ${JSON.stringify(detail).slice(0, 300)}` : ""}`);
  return Boolean(ok);
}

/**
 * 同一根因在不同来源被写成不同 code —— 与产品里的 `ROOT_CAUSE_ALIASES` 对应，
 * 但这里是**独立写出**的：本地闭包按 `answerKey[slot].kind === "unresolved"` 记
 * `ANSWER_UNRESOLVED`，门禁一律记 `ANSWER_MISSING`，不去重就会一道题两行。
 */
const ROOT_CAUSE_ALIASES = { ANSWER_UNRESOLVED: "ANSWER_MISSING" };

/** 本地 code 归一到根因码。 */
function aliasRootCause(code) {
  return ROOT_CAUSE_ALIASES[code] ?? code;
}

/**
 * 一个门禁 blocker 的**根因码**（独立实现，与产品 `rootCauseOf` 对应）。
 *
 * 门禁把**所有**质量码都塞进 `ISSUE_UNRESOLVED` 这一个 code，根因在 `internal`
 * （`phase4-<质量码>-<目标>`）里。`phase4-SLOT_HOST_MISSING-group-2` 这种目标自带 '-'
 * 的情况，**不能**按最后一个 '-' 切（会切出 `SLOT_HOST_MISSING-group`），
 * 要按「开头连续的大写/数字/下划线」取码。
 */
function gateRootCause(blocker) {
  const internal = blocker.internal ?? "";
  if (blocker.code === "QUALITY_HARD_FAILURE") return internal || blocker.code;
  const phase4 = /^phase4-([A-Z0-9_]+)-/.exec(internal);
  if (blocker.code === "ISSUE_UNRESOLVED" && phase4) return phase4[1];
  return blocker.code;
}

/** 泛化汇总行：它们只说明「有硬失败」，不指向任何具体目标，不得冒充任务。 */
const GENERIC_GATE_CODES = new Set(["QUALITY_HARD_FAILURE", "QUALITY_NOT_READY"]);

/**
 * **独立算法：门禁里哪些根因是「必须被某张任务卡接住」的。**
 *
 * 规则（与产品一致但独立写出）：
 *  - 泛化汇总行（`QUALITY_HARD_FAILURE`）不算——它只是「有硬失败」的汇总；
 *  - 其余每一条 blocker 的根因码都要有归属，**不论有没有 targetId**：
 *    `RUNTIME_COMPILER_FAILED` / `SIGNIFICANT_REGION_UNASSIGNED` 的 targetId 就是
 *    `document`，它们同样是用户必须处理的事，不能因为「没有具体题号」被吞掉。
 */
function expectedGateRootCauses(blockers) {
  const out = new Set();
  for (const b of blockers) {
    if (GENERIC_GATE_CODES.has(b.code)) continue;
    out.add(gateRootCause(b));
  }
  return out;
}

/** 本地闭包会报出哪些根因码（独立实现，与 `deriveActionableIssues` 对应）。 */
function expectedLocalIssues(ds) {
  const out = [];
  for (const task of ds.taskGroups ?? []) {
    for (const group of task.responseGroups ?? []) {
      for (const slotId of group.slotIds ?? []) {
        const slot = (ds.answerSlots ?? {})[slotId];
        if (slot && slot.participation !== "scoring") continue;
        const answer = (ds.answerKey ?? {})[slotId];
        if (!answer || answer.kind === "unresolved") {
          out.push({ code: answer?.kind === "unresolved" ? "ANSWER_UNRESOLVED" : "ANSWER_MISSING", targetId: slotId });
          continue;
        }
        const empty = answer.kind === "text"
          ? !(answer.values ?? []).some((value) => String(value).trim())
          : answer.kind === "option" && !(answer.labels ?? []).length;
        if (empty) out.push({ code: "ANSWER_MISSING", targetId: slotId });
      }
    }
  }
  return out;
}

/** 从真实 DOM 读任务卡。 */
const READ_TASKS = `(() => {
  const root = document.querySelector('[data-testid="workspace-issue-list"]');
  if (!root) return null;
  const cards = [...root.querySelectorAll('li')].map((li) => {
    const titleEl = li.querySelector('[data-testid^="workspace-task-title-"]');
    const detailEl = li.querySelector('.workspace-task-detail');
    const buttons = [...li.querySelectorAll('button[data-action-id]')].map((b) => ({
      actionId: b.getAttribute('data-action-id'),
      target: b.getAttribute('data-action-target'),
      testid: b.getAttribute('data-testid'),
      label: b.innerText.replace(/\\s+/g, ' ').trim(),
    }));
    return {
      taskId: li.getAttribute('data-task-id'),
      kind: li.getAttribute('data-task-kind'),
      severity: li.getAttribute('data-severity'),
      covers: (li.getAttribute('data-task-covers') || '').split(',').filter(Boolean),
      title: titleEl ? titleEl.innerText.replace(/\\s+/g, ' ').trim() : '',
      detail: detailEl ? detailEl.innerText.replace(/\\s+/g, ' ').trim() : '',
      text: li.innerText.replace(/\\s+/g, ' ').trim(),
      buttons,
    };
  });
  const more = root.querySelector('[data-testid="workspace-tasks-more"]');
  const clear = root.querySelector('[data-testid="workspace-tasks-clear"]');
  return {
    taskCount: Number(root.getAttribute('data-task-count') || 0),
    mergedRows: Number(root.getAttribute('data-merged-rows') || 0),
    canExport: root.getAttribute('data-can-export'),
    preflightState: root.getAttribute('data-preflight-state'),
    cards,
    hasMore: Boolean(more),
    moreText: more ? more.innerText.replace(/\\s+/g, ' ').trim() : null,
    clearText: clear ? clear.innerText.replace(/\\s+/g, ' ').trim() : null,
    headerText: (() => {
      const el = document.querySelector('[data-testid="workspace-issues"]');
      return el ? el.innerText.replace(/\\s+/g, ' ').trim() : null;
    })(),
  };
})()`;

/** 内部术语不得出现在普通界面的可见文本里。 */
const INTERNAL_TERM_PATTERNS = [
  { name: "问题码/枚举名", re: /\b[A-Z][A-Z0-9_]{3,}\b/ },
  { name: "内部版本号", re: /\bv\d+\b/ },
  { name: "slot", re: /slot/i },
  { name: "schema", re: /schema/i },
  { name: "reasonCode", re: /reason[\s_-]?code/i },
  { name: "batchId/editVersion", re: /batch[\s_-]?id|edit[\s_-]?version/i },
  { name: "批次基线", re: /批次基线/ },
];

async function main() {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: process.argv.includes("--tolerate-concurrent-edits") });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);
  if (!fs.existsSync(fixturePath)) throw new CannotRunError(`夹具不存在：${fixturePath}`);
  report.identity.fixtureSha256 = sha256File(fixturePath);

  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  const staged = path.join(runDir, "pdfs", path.basename(fixturePath));
  fs.copyFileSync(fixturePath, staged);

  session = await launchTauriAppCdp({
    exePath, runDir, extraBrowserArgs: extraArgs,
    appEnv: isPdf ? {} : { PDF2TEST_AUTOMATION_SOURCE_FILES: staged },
  });
  report.identity.browserArgs = session.browserArgs;

  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });
  // **不做 `Page.reload`**（本轮实测它会把 CDP 会话打断：重载后 WebView2 的 page target
  // 重建，`Runtime.evaluate` 直接报「CDP 连接已关闭」，脚本在等 `library-after-reload`
  // 时超时，一次断言都跑不到）。上一版重载的理由是「让应用重新读 localStorage 里的
  // `cloudEnabled:false`」，但这个理由不成立：
  //   - `AppSettingsV1` 里**没有** `cloudEnabled` 这个字段（`appSettings.ts`），
  //     它从来不会被 `readAppSettings()` 读到；
  //   - `readAppSettings()` 每次调用都现读 localStorage，本来就不需要重载。
  // 于是重载只带来风险、不带来任何前置条件。这里只把路由指回题库，应用启动时本来就在题库页。
  await session.evaluate(`(() => { location.hash = "#/library"; return true; })()`);

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
    const r = await call("get_workspace_item", { itemId });
    if (r?.ok && r.value?.ds) { draft = r.value.ds; break; }
    await sleep(2000);
  }
  if (!draft) throw new CannotRunError("题稿在超时前没有加载完成");
  report.identity.draftVersion = (await call("get_workspace_item", { itemId }))?.value?.version ?? null;
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-open" });

  // ---- 门禁原始 blockers（期望值的来源）----
  const pf = await call("get_publish_preflight", { jobId: itemId });
  if (!pf?.ok) throw new CannotRunError(`get_publish_preflight 失败：${pf?.error}`);
  const blockers = (pf.value?.blockers ?? []).map((b) => ({ code: b.code, targetId: b.targetId ?? null, internal: b.internal ?? "", userMessage: b.userMessage ?? "" }));
  const gateWarnings = (pf.value?.warnings ?? []).map((w) => ({ code: w.code, message: w.message ?? "" }));
  const localExpected = expectedLocalIssues(draft);
  const expectedGateCauses = expectedGateRootCauses(blockers);
  const expectedLocalCauses = new Set(localExpected.map((i) => aliasRootCause(i.code)));
  report.gate = {
    passed: pf.value?.passed ?? null,
    rawBlockerCount: blockers.length,
    rawByCode: blockers.reduce((acc, b) => ({ ...acc, [b.code]: (acc[b.code] ?? 0) + 1 }), {}),
    genericRows: blockers.filter((b) => b.code === "QUALITY_HARD_FAILURE").length,
    warningCount: gateWarnings.length,
    warningCodes: gateWarnings.map((w) => w.code),
    expectedGateRootCauses: [...expectedGateCauses].sort(),
    expectedLocalRootCauses: [...expectedLocalCauses].sort(),
    expectedLocalRows: localExpected.length,
  };
  console.log(`[issue-list] gate raw=${blockers.length} warnings=${gateWarnings.length} expectedGateCauses=${expectedGateCauses.size} expectedLocalCauses=${expectedLocalCauses.size}`);

  // ---- 打开问题面板，读真实渲染的任务卡 ----
  await session.clickSelector('[data-testid="workspace-issues"]');
  await session.waitFor(`!!document.querySelector('[data-testid="workspace-issue-list"]')`, { timeoutMs: 15000, label: "issue-list-open" });
  // 页面**自己**也会去取一次 preflight（`ExamWorkspacePage` 的 useEffect），而这一步是异步的。
  // 我在上面用 IPC 直接读门禁要快得多，于是会出现「面板已挂载、但任务还是空」的竞态。
  //
  // 必须等**门禁结论到位**再断言，判据是 `data-preflight-state`（`loading|loaded|error`）——
  // 上一版等的条件是「有 li 或出现空态提示」，而空态提示在门禁回来**之前**就已经渲染，
  // 于是等待立刻通过、读到 0 张卡，把「还没查完」误判成「产品没渲染」。
  // 这正是本轮修掉的那个产品缺陷：门禁没回来时界面绝不能显示「可以导出」。
  await session.waitFor(
    `(() => {
      const el = document.querySelector('[data-testid="workspace-issue-list"]');
      return !!el && el.getAttribute('data-preflight-state') !== 'loading';
    })()`,
    { timeoutMs: 40000, label: "issue-preflight-settled" }
  );
  let snapshot = await session.evaluate(READ_TASKS);
  // 折叠时看不到的卡，其 `data-task-covers` 也不可见 —— 先展开，否则「不隐藏」的断言会假失败。
  if (snapshot?.hasMore) {
    await session.clickSelector('[data-testid="workspace-tasks-more"]');
    await sleep(300);
    const expanded = await session.evaluate(READ_TASKS);
    report.expandedFrom = { before: snapshot.cards.length, moreText: snapshot.moreText, after: expanded.cards.length };
    snapshot = expanded;
  }
  report.rendered = snapshot;
  console.log(`[issue-list] rendered cards=${(snapshot?.cards ?? []).length} mergedRows=${snapshot?.mergedRows} canExport=${snapshot?.canExport}`);

  const cards = snapshot?.cards ?? [];
  const coveredCauses = new Set(cards.flatMap((card) => card.covers));

  // ---- 断言 0：列表非空（否则下面几条断言会退化成空断言）----
  assert("任务列表确实渲染出了任务卡（非空断言前置）", cards.length > 0, { rendered: cards.length, clearText: snapshot?.clearText ?? null });

  // ---- 断言 0b：门禁还有阻断时，界面绝不说「可以导出」----
  // 「可以导出」以**当前题稿的后端发布检查**为准（本轮任务书第 5 条）。
  // 实测撞到过一个假完成：面板挂载即显示「可以导出」，而同一时刻后端门禁报 34 条阻断——
  // 因为门禁结论还没回来时列表是空的。这条把它钉住。
  assert(
    "门禁还有阻断时界面不说「可以导出」",
    !(blockers.length > 0 && snapshot?.canExport === "true"),
    { canExport: snapshot?.canExport, rawBlockers: blockers.length, clearText: snapshot?.clearText ?? null, preflightState: snapshot?.preflightState ?? null }
  );

  // ---- 断言 1：合并真的发生了 ----
  // 「任务数 < 门禁原始阻断条数」+「界面自报合并掉的原始行数 > 0」两条一起看：
  // 前者可能因为「本来就没几条」而偶然成立，后者由构建任务的算法直接给出。
  assert(
    "合并真的发生了（任务数 < 门禁原始条数，且界面自报 mergedRows > 0）",
    cards.length < blockers.length && Number(snapshot?.mergedRows ?? 0) > 0,
    { cards: cards.length, rawBlockers: blockers.length, mergedRows: snapshot?.mergedRows }
  );

  // ---- 断言 2（核心）：一个根因都没被吞 ----
  // 这是「不同问题不被隐藏」在新形状下的等价命题：行不再逐条渲染，但每个根因都必须
  // 出现在某张任务卡的 `data-task-covers` 里。**同时**查门禁与本地两半，
  // 因为上一版脚本只算门禁那半，曾把本地那 14 行误判成「产品多渲染」。
  const missingGateCauses = [...expectedGateCauses].filter((code) => !coveredCauses.has(code));
  const missingLocalCauses = [...expectedLocalCauses].filter((code) => !coveredCauses.has(code));
  report.coverage = {
    expectedGate: [...expectedGateCauses].sort(),
    expectedLocal: [...expectedLocalCauses].sort(),
    rendered: [...coveredCauses].sort(),
    missingGate: missingGateCauses,
    missingLocal: missingLocalCauses,
  };
  assert(
    "门禁里每个具体根因都被某张任务卡接住（不隐藏）",
    expectedGateCauses.size > 0 && missingGateCauses.length === 0,
    { expected: expectedGateCauses.size, missing: missingGateCauses }
  );
  assert(
    "本地闭包报出的根因也被接住（本地那半没被合并吃掉）",
    expectedLocalCauses.size > 0 && missingLocalCauses.length === 0,
    { expected: expectedLocalCauses.size, missing: missingLocalCauses }
  );

  // ---- 断言 3：泛化汇总行不冒充任务 ----
  // 泛化行只在「原因没有被具体任务完整表达」时才该显示；本夹具里原因是被表达了的，
  // 所以它不该出现在任何 covers 里，也不该单独成卡。
  const genericClaimed = cards.filter((card) => card.covers.some((code) => GENERIC_GATE_CODES.has(code)));
  const genericCards = cards.filter((card) => /还有未确认的内容|必须修复的内容缺陷/.test(card.text));
  assert(
    "泛化汇总行没有冒充成任务（原因已被具体任务表达）",
    blockers.some((b) => b.code === "QUALITY_HARD_FAILURE") && genericClaimed.length === 0 && genericCards.length === 0,
    {
      gateHasGeneric: blockers.filter((b) => b.code === "QUALITY_HARD_FAILURE").length,
      claimedBy: genericClaimed.map((c) => c.taskId),
      genericCards: genericCards.map((c) => c.text),
    }
  );

  // ---- 断言 4：每条任务都有真按钮，且动作种类合法 ----
  const ALLOWED_ACTIONS = new Set(["fill-answer", "view-source", "retry-recognition"]);
  const withoutAction = cards.filter((card) => card.buttons.length === 0);
  const illegalActions = cards.flatMap((card) =>
    card.buttons.filter((b) => !ALLOWED_ACTIONS.has(b.actionId)).map((b) => ({ taskId: card.taskId, actionId: b.actionId, label: b.label }))
  );
  // 「确认」「忽略」这类点了不改变门禁结果的按钮，本轮明确禁止。
  const fakeButtons = cards.flatMap((card) =>
    card.buttons.filter((b) => /确认|忽略|知道了|知道了，跳过/.test(b.label)).map((b) => ({ taskId: card.taskId, label: b.label }))
  );
  report.actions = cards.map((c) => ({ taskId: c.taskId, kind: c.kind, actions: c.buttons.map((b) => b.actionId) }));
  assert(
    "每条任务至少有一个真实动作按钮，且动作种类合法（无「确认」「忽略」这类假按钮）",
    withoutAction.length === 0 && illegalActions.length === 0 && fakeButtons.length === 0,
    { withoutAction: withoutAction.map((c) => c.taskId), illegalActions, fakeButtons }
  );

  // ---- 断言 5：任务卡文本里没有内部术语 ----
  const leaks = [];
  for (const card of cards) {
    for (const pattern of INTERNAL_TERM_PATTERNS) {
      const hit = pattern.re.exec(card.text);
      if (hit) leaks.push({ taskId: card.taskId, term: pattern.name, hit: hit[0], text: card.text });
    }
  }
  report.leaks = leaks;
  assert("任务卡可见文本里没有内部术语（问题码 / v1 / slot / schema / batchId …）", leaks.length === 0, { leaks: leaks.slice(0, 8) });

  // ---- 断言 6：顶部入口与列表说的是同一件事 ----
  // 顶部此前显示原始问题条数（`问题 46 · 阻断 29`），用户点开却只看到 3 条任务——
  // 「顶部计数」与「点开后的列表」必须收敛到同一个数字。
  const headerMatch = /问题\s*(\d+)/.exec(String(snapshot?.headerText ?? ""));
  assert(
    "顶部入口显示的是任务数，与列表一致",
    Boolean(headerMatch) && Number(headerMatch[1]) === Number(snapshot?.taskCount ?? -1),
    { headerText: snapshot?.headerText, taskCount: snapshot?.taskCount }
  );

  // ---- 断言 7：`fill-answer` 真的有作用（定位到答案控件）----
  // 逐条点**每一个**「去填写」：
  //   - 不允许**静默无反应**（点了既不滚动、也不给说明）；
  //   - 至少有一条必须**真的滚动到题面元素** —— 否则「定位答案控件」这句话就没被证明，
  //     只是「点了之后有话说」。实测这一条抓到过一个真缺陷：内联填空的答案输入框
  //     渲染在 stimulus 内部，宿主元素带的是内容节点 id（`slot-node-q27`），
  //     而定位只按 slotId 与 `hostNodeId`（那是 stimulus 节点 id）找，两跳全落空，
  //     于是每一张「去填写」卡都只给出「找不到」。
  const fillButtons = cards.flatMap((card) =>
    card.buttons.filter((b) => b.actionId === "fill-answer").map((b) => ({ card, button: b }))
  );
  if (fillButtons.length) {
    await session.evaluate(`(() => {
      window.__issueScrolled = [];
      const original = Element.prototype.scrollIntoView;
      Element.prototype.scrollIntoView = function (...args) {
        const id = this.dataset.editorId || this.dataset.questionId || this.dataset.responseGroupId || null;
        if (id) window.__issueScrolled.push(id);
        if (original) return original.apply(this, args);
      };
      return true;
    })()`);
    const perButton = [];
    for (const { card, button } of fillButtons) {
      await session.evaluate(`(() => { window.__issueScrolled = []; return true; })()`);
      await session.clickSelector(`[data-testid="${button.testid}"]`);
      await sleep(600);
      const scrolled = await session.evaluate(`window.__issueScrolled`);
      const missNotice = await session.evaluate(
        `(() => { const el = document.querySelector('[data-testid="workspace-locate-miss"]'); return el ? el.innerText.replace(/\\s+/g,' ').trim() : null; })()`
      );
      perButton.push({ taskId: card.taskId, target: button.target, scrolled: scrolled ?? [], missNotice });
    }
    report.fillAnswer = perButton;
    const silent = perButton.filter((entry) => !entry.scrolled.length && !entry.missNotice);
    const located = perButton.filter((entry) => entry.scrolled.length > 0);
    assert(
      "每个「去填写」都有作用：滚动到目标或如实说明找不到（不静默无反应）",
      silent.length === 0,
      { silent }
    );
    assert(
      "至少有一条「去填写」真的定位到了题面上的答案控件",
      located.length > 0,
      { located: located.map((entry) => ({ taskId: entry.taskId, target: entry.target, scrolled: entry.scrolled })), perButton }
    );
  } else {
    assert("夹具里存在「去填写」任务（本断言的先决条件）", false, { kinds: cards.map((c) => c.kind) });
  }

  // ---- 断言 8：`view-source` 真的打开原文件 ----
  const sourceCard = cards.find((card) => card.buttons.some((b) => b.actionId === "view-source"));
  if (sourceCard) {
    const button = sourceCard.buttons.find((b) => b.actionId === "view-source");
    await session.clickSelector(`[data-testid="${button.testid}"]`);
    await sleep(900);
    const drawer = await session.evaluate(
      `(() => { const el = document.querySelector('[aria-label="原文件"]'); return el ? el.innerText.replace(/\\s+/g,' ').trim().slice(0, 200) : null; })()`
    );
    report.viewSource = { taskId: sourceCard.taskId, drawer };
    assert("「查看原文」真的打开了原文件", Boolean(drawer), report.viewSource);
    // 关掉抽屉，避免影响后续步骤。
    await session.evaluate(`(() => { const btn = document.querySelector('[aria-label="原文件"] [aria-label="关闭"]'); if (btn) btn.click(); return true; })()`);
    await sleep(400);
  } else {
    assert("夹具里存在「查看原文」任务（本断言的先决条件）", false, { kinds: cards.map((c) => c.kind) });
  }

  // ---- 断言 9（最后执行）：`retry-recognition` 真的重新入队 ----
  // 这条会重启识别，会污染后续断言，所以放在最后。但它必须真的点一次 ——
  // 「按钮存在」不等于「按钮有用」，本轮任务书第 5 条要的是后者。
  const retryCard = cards.find((card) => card.buttons.some((b) => b.actionId === "retry-recognition"));
  if (retryCard) {
    const button = retryCard.buttons.find((b) => b.actionId === "retry-recognition");
    await session.clickSelector(`[data-testid="${button.testid}"]`);
    const shown = await session.waitFor(
      `(() => { const el = document.querySelector('.workspace-notice'); return !!el && /重新加入识别队列/.test(el.innerText); })()`,
      { timeoutMs: 20000, label: "retry-requeued" }
    ).catch(() => false);
    const noticeText = await session.evaluate(
      `(() => { const el = document.querySelector('.workspace-notice'); return el ? el.innerText.replace(/\\s+/g,' ').trim() : null; })()`
    );
    report.retryRecognition = { taskId: retryCard.taskId, target: button.target, requeued: Boolean(shown), noticeText };
    assert("「重新识别」真的把这道题重新加入识别队列", Boolean(shown), report.retryRecognition);
  } else {
    assert("夹具里存在「重新识别」任务（本断言的先决条件）", false, { kinds: cards.map((c) => c.kind) });
  }

  const failed = report.assertions.filter((a) => !a.ok);
  report.verdict = failed.length ? "failed" : "passed";
  report.exitCode = failed.length ? EXIT_CODES.failed : EXIT_CODES.passed;
  report.failedAssertions = failed.map((a) => a.name);
  await session.screenshot("issue-list");
}

let fatal = null;
try {
  await main();
} catch (error) {
  fatal = { kind: error instanceof CannotRunError ? "cannot-run" : "error", message: String(error?.message ?? error) };
  report.fatal = fatal;
  report.verdict = fatal.kind === "cannot-run" ? "cannot-run" : "failed";
  report.exitCode = fatal.kind === "cannot-run" ? EXIT_CODES["cannot-run"] : EXIT_CODES.failed;
  console.error(`[issue-list] ${fatal.kind}: ${fatal.message}`);
}

try {
  if (session) {
    report.screenshots = { final: await session.screenshot("final") };
    const closed = await session.close({ keep });
    report.appProcessExitCode = closed.exitCode;
    report.appOutputTail = closed.appOutput.slice(-3000);
  }
} catch (error) {
  report.closeError = String(error?.message ?? error);
}

report.finishedAt = new Date().toISOString();
writeReport(runDir, report);
console.log(`[issue-list] verdict=${report.verdict} exit=${report.exitCode}`);
if (report.failedAssertions?.length) console.log(`[issue-list] failed=${JSON.stringify(report.failedAssertions)}`);
console.log(`[issue-list] report=${path.join(runDir, "report.json")}`);
process.exit(report.exitCode ?? 1);
