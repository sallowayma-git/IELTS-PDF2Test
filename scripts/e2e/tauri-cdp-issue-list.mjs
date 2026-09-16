#!/usr/bin/env node
/**
 * 问题列表的真实界面校验（WebView2 CDP 通道）。
 *
 * 为什么单独一个脚本：R9 改了两处**用户可见**的行为，但当时只有单测 + 真实产物回放，
 * 没有在真实应用里看过一眼。任务书要求「用户能完成修复，而不只是看到错误」，
 * 那就必须在真实 DOM 上验证，而不是只在纯函数上验证。
 *
 * 校验四件事：
 *   1. **同一根因的泛化重复不再显示** —— 门禁对同一个根因会给出 `QUALITY_HARD_FAILURE`
 *      （targetId 为空、文案泛化）+ `ISSUE_UNRESOLVED`（带具体题位）两条记录。
 *      界面上不应再出现那条泛化的「这道题必须修复的内容缺陷。」。
 *   2. **逐「根因 + 目标」的渲染行数 = 独立算法期望** —— 一条断言同时管两件事：
 *      行数多了是「同一问题重复显示」，少了是「不同问题被隐藏」。
 *      任务书明确要求两者都成立，**不能只追求列表条数变少**。
 *   3. **两半都没被多删** —— 单独比「本地来源的行数」，防止用「多删」满足第 2 条那种假绿。
 *   4. **文档级问题点了要如实说明** —— `document` 级目标在题面上没有对应元素，
 *      以前点了静默无反应；现在必须出现 `workspace-locate-miss` 说明。
 *
 * 期望值用**独立算法**算出来（不复用被测实现），避免自己测自己。
 * 界面行来自 `mergePublishGateIssues(本地闭包, 门禁)`，**两半都要算**：
 * 第一版只算门禁那半，于是把本地那 14 行误判成「产品多渲染」。
 * 第二版按「根因 + 目标」当唯一身份，又把 group-2 上两条**不同事实**合成一行 ——
 * 那是**隐藏**问题，不是去重。现在按「同来源同文案才算同一条」判（见 `expectedFactCounts`）。
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

/**
 * 一个门禁 blocker 携带的**稳定事实 id**（独立实现，与产品 `factIdOf` 对应）。
 *
 * 后端把 `issueId` 从 `phase4-{code}-{target}` 改成 `phase4-{code}-{target}-{slug}`
 * （`slug` 是判别性载荷的确定性哈希），preflight 把该 id 放进 `ISSUE_UNRESOLVED` 的
 * `internal`。**只有 `ISSUE_UNRESOLVED` 的 `internal` 才是 id** ——
 * `QUALITY_HARD_FAILURE` 的 `internal` 放的是质量码本身，取它就把质量码当成了 id。
 */
function gateFactId(blocker) {
  if (blocker.code !== "ISSUE_UNRESOLVED") return "";
  return String(blocker.internal ?? "").trim();
}

/**
 * **独立算法：界面上每个「根因 + 目标」应该有几行。**
 *
 * 故意不复用 `actionableIssues.ts` 的实现——复用就等于自己测自己。
 * 这条断言**同时**管两件事（任务书要求两者都成立）：
 *   - 同一问题不重复显示（行数不能多）；
 *   - 不同问题不被隐藏（行数不能少）。
 * 所以比的是**逐「根因 + 目标」的计数**，不是总行数 —— 总数对得上也可能是
 * 「吞掉一条、同时多算一条」。
 *
 * 规则（与产品一致但独立写出）：
 *  - `QUALITY_HARD_FAILURE` 是泛化行：若它的 `internal`（质量码）已经出现在某条
 *    **带 targetId** 的记录里，这条泛化行应被去掉（它只是「有硬失败」的汇总）；
 *  - `ISSUE_UNRESOLVED` 的 `internal` 是 `phase4-<质量码>-<目标>[-<slug>]`，根因取开头大写段；
 *  - **同来源**（都是门禁）两条算同一事实的条件：根因相同 + 目标相同 + 文案相同，
 *    且**没有**两个不同的稳定事实 id。id 不同 ⇒ 一定是两条事实，**哪怕文案逐字相同**
 *    （这是本轮新加的保护：文案相同不再等于同一事实）；
 *  - **跨来源**（本地闭包 vs 门禁）同根因同目标算同一件事：两个子系统各写一句文案
 *    是设计如此，不能因此让同一道题占两行（实测会白多 14 行）；
 *  - **warnings 也要计入**（它们同样渲染成行）——第一版漏了这一点，
 *    于是把「32 行 vs 我算的 31」误判成产品多渲染了一行，其实多出来的是那条
 *    `BLOCKER_LIST_TRUNCATED` 警告。断言算错和产品出错必须分得清；
 *  - **本地已有的「根因 + 目标」会吸收门禁那一族**（级别提升、文案保留本地的）。
 */
function expectedFactCounts(blockers, warnings, localIssues) {
  const specificRootCauses = new Set();
  for (const b of blockers) {
    if (b.code === "QUALITY_HARD_FAILURE") continue;
    if (!(b.targetId ?? "")) continue;
    specificRootCauses.add(gateRootCause(b));
  }
  const rowsByPair = new Map(); // 根因:目标 -> [{ message, factId }]
  for (const b of blockers) {
    if (b.code === "QUALITY_HARD_FAILURE" && specificRootCauses.has(b.internal ?? "")) continue;
    const targetId = b.targetId ?? "";
    const pair = gateRootCause(b) + ":" + (targetId || b.code);
    if (!rowsByPair.has(pair)) rowsByPair.set(pair, []);
    rowsByPair.get(pair).push({ message: b.userMessage ?? "", factId: gateFactId(b) });
  }
  // 逐对判「同一事实」并归并成等价类；等价类的个数就是这一对应有的行数。
  const counts = new Map();
  for (const [pair, rows] of rowsByPair) {
    const open = [...rows];
    let n = 0;
    while (open.length) {
      const seed = open.shift();
      n += 1;
      for (let i = open.length - 1; i >= 0; i -= 1) {
        const other = open[i];
        // 与产品 `sameFact` 同规则：id 不同 ⇒ 不是同一事实；
        // 只有一侧有 id（或都没 id）⇒ **不作结论**，继续比文案。
        const distinctFact = Boolean(seed.factId) && Boolean(other.factId) && seed.factId !== other.factId;
        if (!distinctFact && seed.message === other.message) open.splice(i, 1);
      }
    }
    counts.set(pair, n);
  }
  for (const w of warnings ?? []) counts.set(w.code + ":" + w.code, 1);

  // 跨来源同事实：本地行会把门禁那一族吸收掉，只留本地那一行。
  const localCounts = new Map();
  for (const i of localIssues ?? []) {
    const pair = aliasRootCause(i.code) + ":" + i.targetId;
    localCounts.set(pair, (localCounts.get(pair) ?? 0) + 1);
  }
  for (const [pair, n] of localCounts) counts.set(pair, n);
  return counts;
}

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

async function main() {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: process.argv.includes("--tolerate-concurrent-edits") });
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
    exePath, runDir, extraBrowserArgs: extraArgs,
    appEnv: isPdf ? {} : { PDF2TEST_AUTOMATION_SOURCE_FILES: staged },
  });
  report.identity.browserArgs = session.browserArgs;

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
  const expectedCounts = expectedFactCounts(blockers, gateWarnings, localExpected);
  const expectedLocal = localExpected.length;
  const expectedTotal = [...expectedCounts.values()].reduce((a, b) => a + b, 0);
  report.gate = {
    passed: pf.value?.passed ?? null,
    rawBlockerCount: blockers.length,
    rawByCode: blockers.reduce((acc, b) => ({ ...acc, [b.code]: (acc[b.code] ?? 0) + 1 }), {}),
    genericRows: blockers.filter((b) => b.code === "QUALITY_HARD_FAILURE").length,
    warningCount: gateWarnings.length,
    warningCodes: gateWarnings.map((w) => w.code),
    expectedFactCounts: [...expectedCounts.entries()].sort(),
    expectedLocalRows: expectedLocal,
    expectedRenderedRows: expectedTotal,
  };
  console.log(`[issue-list] gate raw=${blockers.length} warnings=${gateWarnings.length} expectedLocal=${expectedLocal} expectedTotal=${expectedTotal} pairs=${expectedCounts.size}`);

  // ---- 打开问题面板，读真实渲染的行 ----
  await session.clickSelector('[data-testid="workspace-issues"]');
  await session.waitFor(`!!document.querySelector('[data-testid="workspace-issue-list"]')`, { timeoutMs: 15000, label: "issue-list-open" });
  // 页面**自己**也会去取一次 preflight（`ExamWorkspacePage` 的 useEffect），而这一步是异步的。
  // 我在上面用 IPC 直接读门禁要快得多，于是会出现「面板已挂载、但 issues 还是空」的竞态：
  // 实测有一次读到 0 行。必须等页面自己的门禁结论到位再断言，否则会得出
  // 「渲染 0 行」这种既假又**空**的结论（0 行会让「没有泛化行」之类的断言自动通过）。
  await session.waitFor(
    `document.querySelectorAll('[data-testid="workspace-issue-list"] li').length > 0`,
    { timeoutMs: 30000, label: "issue-rows-populated" }
  );
  const rows = await session.evaluate(`(() => {
    return [...document.querySelectorAll('[data-testid="workspace-issue-list"] li')].map((li) => {
      const btn = li.querySelector('button');
      return {
        severity: li.getAttribute('data-severity'),
        targetId: btn ? btn.getAttribute('data-issue-target-id') : null,
        code: btn ? btn.getAttribute('data-issue-code') : null,
        source: btn ? btn.getAttribute('data-issue-source') : null,
        rootCause: btn ? btn.getAttribute('data-issue-root-cause') : null,
        factId: btn ? btn.getAttribute('data-issue-fact-id') : null,
        text: li.innerText.replace(/\\s+/g, ' ').trim(),
      };
    });
  })()`);
  report.rendered = { count: (rows ?? []).length, rows };
  console.log(`[issue-list] rendered=${(rows ?? []).length}`);

  const gateRows = (rows ?? []).filter((r) => r.source === "gate");
  const localRows = (rows ?? []).filter((r) => r.source === "local");
  report.rendered.bySource = { gate: gateRows.length, local: localRows.length };

  // ---- 断言 0：列表非空（否则下面几条断言会退化成空断言）----
  assert("问题列表确实渲染出了行（非空断言前置）", (rows ?? []).length > 0, { rendered: (rows ?? []).length });

  // ---- 断言 0b：后端**稳定事实 id** 确实到达了界面，且逐行唯一 ----
  // 这是任务书第 3 条「等后端稳定事实 id」的落地验收：不再靠文案猜身份。
  // 两个方向一起查：
  //   - 少了（missing）⇒ 某条事实被隐藏；
  //   - 重了（duplicated）⇒ 同一事实重复显示。
  // 被本地行吸收的门禁行不会渲染成门禁行，所以先从期望里剔除（同根因 + 同目标）。
  const localKeys = new Set((localExpected ?? []).map((i) => aliasRootCause(i.code) + ":" + i.targetId));
  const expectedFactIds = new Set();
  for (const b of blockers) {
    const id = gateFactId(b);
    if (!id) continue;
    if (localKeys.has(gateRootCause(b) + ":" + (b.targetId ?? ""))) continue;
    expectedFactIds.add(id);
  }
  const renderedFactIds = gateRows.map((r) => r.factId).filter(Boolean);
  const renderedFactIdSet = new Set(renderedFactIds);
  const missingFactIds = [...expectedFactIds].filter((id) => !renderedFactIdSet.has(id));
  const duplicatedFactIds = renderedFactIds.filter((id, index) => renderedFactIds.indexOf(id) !== index);
  report.factIds = {
    expected: expectedFactIds.size,
    renderedDistinct: renderedFactIdSet.size,
    missing: missingFactIds,
    duplicated: duplicatedFactIds,
  };
  assert(
    "后端稳定事实 id 全部到达界面且逐行唯一（不隐藏、不重复）",
    expectedFactIds.size > 0 && missingFactIds.length === 0 && duplicatedFactIds.length === 0,
    report.factIds
  );

  // ---- 断言 1：泛化重复不再显示 ----
  // 用 `data-issue-code` 判，不再靠文案匹配：文案是后端的，改一个字断言就假绿。
  const genericRendered = (rows ?? []).filter((r) => r.code === "QUALITY_HARD_FAILURE");
  assert(
    "泛化 QUALITY_HARD_FAILURE 行不再出现",
    blockers.some((b) => b.code === "QUALITY_HARD_FAILURE") && genericRendered.length === 0,
    { gateHasGeneric: blockers.filter((b) => b.code === "QUALITY_HARD_FAILURE").length, renderedGeneric: genericRendered.length }
  );

  // ---- 断言 2（核心）：逐「根因 + 目标」的渲染行数 = 独立算法期望 ----
  // 一条断言同时管两件事（任务书要求两者都成立）：
  //   - 行数**多**了 = 同一问题重复显示；
  //   - 行数**少**了 = 不同问题被隐藏。
  // 比的是逐键计数而不是总数：总数对得上也可能是「吞掉一条、同时多算一条」。
  // 键用界面输出的 `data-issue-root-cause`（不是 `data-issue-code`）：
  // 门禁把**所有**质量码都写成 `ISSUE_UNRESOLVED`，只看 code 会把两个不同根因看成同一条。
  const actualCounts = new Map();
  for (const r of rows ?? []) {
    const pair = `${r.rootCause}:${r.targetId}`;
    actualCounts.set(pair, (actualCounts.get(pair) ?? 0) + 1);
  }
  const allPairs = new Set([...expectedCounts.keys(), ...actualCounts.keys()]);
  const mismatches = [];
  for (const pair of allPairs) {
    const want = expectedCounts.get(pair) ?? 0;
    const got = actualCounts.get(pair) ?? 0;
    if (want !== got) mismatches.push({ pair, want, got, kind: got > want ? "重复显示" : "被隐藏" });
  }
  report.factCounts = { expected: [...expectedCounts.entries()].sort(), actual: [...actualCounts.entries()].sort() };
  assert(
    "每个 (根因, 目标) 的渲染行数 = 独立算法期望（同时抓重复与隐藏）",
    mismatches.length === 0,
    { mismatches: mismatches.slice(0, 8), pairs: allPairs.size, rendered: (rows ?? []).length }
  );

  // ---- 断言 3：本地那半没有被合并吃掉 ----
  // 断言 2 也可能靠**多删**满足；这条防止那种假绿。
  const localCodes = [...new Set(localRows.map((r) => r.code))];
  assert(
    "本地来源的行数 = 独立算法算出的期望（没有被多删）",
    localRows.length === expectedLocal && localCodes.every((c) => ["ANSWER_MISSING", "ANSWER_UNRESOLVED"].includes(c)),
    { rendered: localRows.length, expected: expectedLocal, localCodes }
  );

  // ---- 断言 4：确实发生了去重（否则上面几条可能是空断言）----
  assert("确实去重了（渲染总行数 < 门禁原始条数）", (rows ?? []).length < blockers.length, { rendered: (rows ?? []).length, raw: blockers.length });

  // ---- 断言 4：文档级问题点了要如实说明 ----
  const docRow = (rows ?? []).find((r) => r.targetId === "document");
  if (docRow) {
    await session.evaluate(`(() => {
      const li = [...document.querySelectorAll('[data-testid="workspace-issue-list"] li')]
        .find((el) => el.querySelector('button') && el.querySelector('button').getAttribute('data-issue-target-id') === 'document');
      li.querySelector('button').click();
      return true;
    })()`);
    const shown = await session.waitFor(`!!document.querySelector('[data-testid="workspace-locate-miss"]')`, { timeoutMs: 8000, label: "locate-miss" }).catch(() => false);
    const notice = shown ? await session.evaluate(`document.querySelector('[data-testid="workspace-locate-miss"]').innerText.replace(/\\s+/g,' ').trim()`) : null;
    assert("文档级问题点击后如实说明（不再静默无反应）", shown, { notice });
    report.locateMissNotice = notice;
  } else {
    assert("夹具里存在 document 级问题（本断言的先决条件）", false, { targets: [...new Set((rows ?? []).map((r) => r.targetId))] });
  }

  // ---- 断言 5：可定位的问题仍然能定位（改动没有把正常定位弄坏）----
  // 必须读**提示文本**，不能只判元素是否存在：提示是粘性的，只判存在的话
  // 「上一条 document 的提示还挂着」和「这一条真的定位失败」看起来一模一样。
  const locatableRow = (rows ?? []).find((r) => r.targetId && r.targetId !== "document" && /^q\d+$/.test(r.targetId));
  if (locatableRow) {
    await session.evaluate(`(() => {
      const li = [...document.querySelectorAll('[data-testid="workspace-issue-list"] li')]
        .find((el) => el.querySelector('button') && el.querySelector('button').getAttribute('data-issue-target-id') === ${JSON.stringify(locatableRow.targetId)});
      li.querySelector('button').click();
      return true;
    })()`);
    await sleep(800);
    const noticeText = await session.evaluate(`(() => { const el = document.querySelector('[data-testid="workspace-locate-miss"]'); return el ? el.innerText.replace(/\\s+/g,' ').trim() : null; })()`);
    assert("可定位的题位问题不显示「不在题面上」提示", noticeText === null, { targetId: locatableRow.targetId, noticeText });
    report.locateAfterSlotClick = { targetId: locatableRow.targetId, noticeText };
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
