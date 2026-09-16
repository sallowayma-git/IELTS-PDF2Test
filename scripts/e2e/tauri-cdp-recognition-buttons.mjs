#!/usr/bin/env node
/**
 * 真实候选的**按钮流程**验收（WebView2 CDP 通道）。
 *
 * 为什么单独一个脚本：`tauri-cdp-recognition-write-path.mjs` 的第 9 步用
 * `apply_editor_commands` 直接发 `setAnswer` 补丁，那只证明**编辑通道**可用，
 * **不替代**用户在面板上点「采用修正 / 保持现状 / 撤销」。任务书明确要求
 * 「必须通过真实界面点击」，本脚本就是那条通道。
 *
 * 六个场景（全部走真实 DOM 点击，内容断言读真实权威稿）：
 *   1. accept-suggestion          接受建议 → 内容确实改变、版本递增、重开保留
 *   2. reject-suggestion          拒绝建议 → 内容不变、决策持久化（重开后 status=rejected）
 *   3. undo-auto-fix              撤销自动修正 → 恢复正确原值、重开不再显示未撤销
 *   4. stale-suggestion-protected 用户先改目标再接受旧建议 → 用户的修改受保护
 *   5. idempotent-retry           重复点击不重复应用（版本只递增一次）
 *   6. （无候选时）所有场景记为 `not-executable` —— **不是 passed**
 *
 * 场景 4 的通过依据（任务书：**不能以按钮消失作为成功证据**）：
 *   ① 用户内容仍是用户写的（读权威稿比对，重开后再比一次）；
 *   ② 实际 outcome：后端 `stale` 为真 + 该决策有确定的 `resolution`（后端事实，不是按钮可见性）；
 *   ③ 重开持久化：① ② 与 resolution 重开后一致。
 * 「接受按钮不见了」只作为线索记录（`acceptAvailable`），不参与判定。
 *
 * 判定见 `lib/chain-verdict.mjs` 的 `computeScenarioVerdict`：
 * 有任何 `not-executable` → `not-executable`（退出码 5），绝不报 passed。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-recognition-buttons.mjs [--pdf <path>] [--diagnostic-args] [--keep]
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
import { computeScenarioVerdict, SCENARIO_STATUS } from "./lib/chain-verdict.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const fixtureIdx = process.argv.indexOf("--pdf");
const fixturePath = path.resolve(
  fixtureIdx >= 0 ? process.argv[fixtureIdx + 1] : path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf")
);
const isPdf = /\.pdf$/i.test(fixturePath);
const diagnosticArgsRequested = process.argv.includes("--diagnostic-args");
const extraArgs = diagnosticArgsRequested ? "--no-sandbox --disable-gpu" : "";
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `run-recog-buttons-${new Date().toISOString().replace(/[:.]/g, "-")}`);

const report = {
  task: "recognition-candidate-button-flows",
  scope: "真实界面点击验收（不是 IPC 探针）",
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
  scenarios: [],
  verdict: "failed",
};

let session = null;
let itemId = null;

/** 调真实 IPC，返回 {ok, value|error}。 */
async function call(command, args) {
  const wrapped = command === "apply_recognition_decisions" || command === "apply_editor_commands";
  const r = await session.invoke(command, wrapped ? { input: args } : args);
  if (r && r.__noInvoke) throw new Error("invoke 不可用");
  return r;
}

/** 读权威稿（只读观测，不驱动）。 */
async function readDraft() {
  const r = await call("get_workspace_item", { itemId });
  if (!r?.ok) throw new Error(`get_workspace_item 失败：${r?.error}`);
  return r.value;
}

/** 读识别决策视图（归一后：actionable / autoApplied）。 */
async function readDecision() {
  const r = await call("get_recognition_decision", { itemId });
  if (!r?.ok) throw new Error(`get_recognition_decision 失败：${r?.error}`);
  const v = r.value ?? {};
  return {
    batchId: v.batchId ?? null,
    editVersion: Number(v.editVersion ?? 0),
    // 过期判定是**后端**给的（`stale` 由 `baseEditVersion` 与当前版本比较得出），
    // 不是前端按钮的可见性推出来的 —— 所以它能当作「实际 outcome」的证据。
    stale: Boolean(v.stale),
    baseEditVersion: v.baseEditVersion ?? null,
    currentEditVersion: v.currentEditVersion ?? v.editVersion ?? null,
    actionable: Array.isArray(v.actionable) ? v.actionable : [],
    autoApplied: Array.isArray(v.autoApplied) ? v.autoApplied : [],
    chains: v.chains ?? null,
  };
}

/** 某条决策在后端视图里的 resolution（查不到返回 undefined）。 */
async function decisionResolution(decisionId) {
  const d = await readDecision();
  const found = [...d.actionable, ...d.autoApplied].find((x) => x.decisionId === decisionId);
  return found ? (found.resolution ?? null) : undefined;
}

/** 面板里某个 decisionId 当前显示的「当前值 → 建议值」文本。 */
async function panelValues(decisionId) {
  return session.evaluate(
    `(() => { const el = document.querySelector('[data-testid="workspace-recognition-values-${decisionId}"]'); return el ? el.innerText.replace(/\\s+/g,' ').trim() : null; })()`
  );
}

/** 面板里某个 decisionId 的卡片状态（data-status / data-resolution / 是否还有按钮）。 */
async function cardState(decisionId) {
  return session.evaluate(
    `(() => {
      const card = document.querySelector('[data-decision-id="${decisionId}"]');
      if (!card) return null;
      return {
        status: card.getAttribute('data-status'),
        resolution: card.getAttribute('data-resolution'),
        hasAccept: !!document.querySelector('[data-testid="workspace-recognition-accept-${decisionId}"]'),
        hasKeep: !!document.querySelector('[data-testid="workspace-recognition-keep-${decisionId}"]'),
        hasUndo: !!document.querySelector('[data-testid="workspace-recognition-undo-${decisionId}"]'),
        undone: !!document.querySelector('[data-testid="workspace-recognition-undone-${decisionId}"]'),
        text: card.innerText.replace(/\\s+/g,' ').trim()
      };
    })()`
  );
}

async function openPanel() {
  await session.clickSelector('[data-testid="workspace-recognition-toggle"]');
  await session.waitFor(`!!document.querySelector('[data-testid="workspace-recognition"]')`, { timeoutMs: 20000, label: "recognition-panel" });
}

/** 重新加载应用数据并重开面板：用于验证「重开后仍一致」。 */
async function reopenWorkspace() {
  await session.clickSelector(".workspace-back-button");
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000, label: "library-after-back" });
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-reopen" });
  await openPanel();
}

/** 场景登记：状态只有三态，绝不使用「跳过」。 */
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

  // PDF 必须先进 runDir/pdfs：harness 把 PDF2TEST_AUTOMATION_PDF_DIR 指向那里，
  // 「选择文件夹」hook 只列该目录下的 PDF。
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

  // ---- 准备：题库 → 导入 → 工作区 → 面板 ----
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
  const deadline = Date.now() + 90000;
  while (Date.now() < deadline && !itemId) {
    const ids = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
    itemId = (ids ?? []).find((id) => !(before ?? []).includes(id)) ?? null;
    if (!itemId) await sleep(1000);
  }
  if (!itemId) throw new CannotRunError("导入后未出现新的题库行");
  report.identity.itemId = itemId;

  // 等本地稿落盘（ds 非空）再进工作区。
  const draftDeadline = Date.now() + 120000;
  while (Date.now() < draftDeadline) {
    const w = await readDraft();
    if (w?.ds) break;
    await sleep(2000);
  }
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-open" });
  await openPanel();

  // ---- 候选可用性：这是所有场景的前提 ----
  const decision = await readDecision();
  const reviewItems = decision.actionable.filter((i) => i.resolution === "needs_review");
  const autoFixedItems = decision.autoApplied.filter((i) => i.resolution === "auto_fixed");
  report.candidates = {
    batchId: decision.batchId,
    chainStates: decision.chains,
    actionableCount: decision.actionable.length,
    needsReviewCount: reviewItems.length,
    autoFixedCount: autoFixedItems.length,
  };

  const NO_CANDIDATES =
    "本仓当前没有真实候选项（actionable/autoApplied 为空）：按钮流程的前提不成立。"
    + "任务书要求此时记为无法执行，不得跳过或判 passed。";

  // ---- 场景 1：接受建议 ----
  const acceptTarget = reviewItems[0];
  if (!acceptTarget) notExecutable("accept-suggestion", NO_CANDIDATES);
  else await scenario("accept-suggestion", async () => {
    const beforeDraft = await readDraft();
    const beforeVersion = Number(beforeDraft.editVersion ?? 0);
    const panelText = await panelValues(acceptTarget.decisionId);
    await session.clickSelector(`[data-testid="workspace-recognition-accept-${acceptTarget.decisionId}"]`);
    await session.waitFor(`!document.querySelector('[data-testid="workspace-recognition-accept-${acceptTarget.decisionId}"]')`, { timeoutMs: 30000, label: "accept-done" });
    // 保存是异步的：等版本推进。
    const vDeadline = Date.now() + 30000;
    let afterDraft = null;
    while (Date.now() < vDeadline) {
      afterDraft = await readDraft();
      if (Number(afterDraft.editVersion ?? 0) > beforeVersion) break;
      await sleep(500);
    }
    const afterVersion = Number(afterDraft?.editVersion ?? 0);
    if (!(afterVersion > beforeVersion)) throw new Error(`接受后版本没有推进：${beforeVersion} → ${afterVersion}`);
    const state = await cardState(acceptTarget.decisionId);
    if (state?.status !== "accepted") throw new Error(`卡片状态不是 accepted：${JSON.stringify(state)}`);
    // 重开保留
    await reopenWorkspace();
    const reopened = await readDraft();
    if (Number(reopened.editVersion ?? 0) < afterVersion) throw new Error(`重开后版本回退：${afterVersion} → ${reopened.editVersion}`);
    const reopenedState = await cardState(acceptTarget.decisionId);
    if (reopenedState && reopenedState.status === "open") throw new Error("重开后该建议又变回待处理（决策未持久化）");
    return { decisionId: acceptTarget.decisionId, targetId: acceptTarget.target?.targetId ?? null, panelText, versionBefore: beforeVersion, versionAfter: afterVersion, reopenedVersion: reopened.editVersion };
  });

  // ---- 场景 2：拒绝建议（内容不变 + 决策持久化） ----
  const rejectTarget = reviewItems[1];
  if (!rejectTarget) notExecutable("reject-suggestion", NO_CANDIDATES);
  else await scenario("reject-suggestion", async () => {
    const beforeDraft = await readDraft();
    const canonicalBefore = JSON.stringify(beforeDraft.ds?.answerKey ?? {});
    await session.clickSelector(`[data-testid="workspace-recognition-keep-${rejectTarget.decisionId}"]`);
    await session.waitFor(`!document.querySelector('[data-testid="workspace-recognition-keep-${rejectTarget.decisionId}"]')`, { timeoutMs: 30000, label: "keep-done" });
    await sleep(1500);
    const afterDraft = await readDraft();
    const canonicalAfter = JSON.stringify(afterDraft.ds?.answerKey ?? {});
    if (canonicalBefore !== canonicalAfter) throw new Error("拒绝建议竟然改动了权威稿内容");
    const state = await cardState(rejectTarget.decisionId);
    if (state?.status !== "rejected") throw new Error(`卡片状态不是 rejected：${JSON.stringify(state)}`);
    await reopenWorkspace();
    const reopenedState = await cardState(rejectTarget.decisionId);
    if (reopenedState && reopenedState.status === "open") throw new Error("重开后被拒绝的建议又变回待处理（决策未持久化）");
    return { decisionId: rejectTarget.decisionId, canonicalUnchanged: true, statusAfterReopen: reopenedState?.status ?? null };
  });

  // ---- 场景 3：撤销自动修正（恢复正确原值） ----
  const undoTarget = autoFixedItems[0];
  if (!undoTarget) notExecutable("undo-auto-fix", NO_CANDIDATES);
  else await scenario("undo-auto-fix", async () => {
    const beforeDraft = await readDraft();
    const beforeVersion = Number(beforeDraft.editVersion ?? 0);
    // 后端 undo 补丁指向的槽位与目标值。
    const undo = undoTarget.undo ?? null;
    if (!undo || undo.op !== "setAnswer") throw new Error(`auto_fixed 项没有可用的 undo 补丁：${JSON.stringify(undo)}`);
    const slotId = undo.slotId;
    const valueBeforeUndo = beforeDraft.ds?.answerKey?.[slotId] ?? null;
    if (JSON.stringify(valueBeforeUndo) === JSON.stringify(undo.value)) {
      throw new Error("撤销前后的值本来就相同，断言会退化成空断言，拒绝执行");
    }
    await session.clickSelector('[data-testid="workspace-recognition-autofixed"] button');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-recognition-undo-${undoTarget.decisionId}"]')`, { timeoutMs: 15000, label: "autofixed-expanded" });
    await session.clickSelector(`[data-testid="workspace-recognition-undo-${undoTarget.decisionId}"]`);
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
    if (JSON.stringify(valueAfterUndo) !== JSON.stringify(undo.value)) {
      throw new Error(`撤销没有把值改回原值：期望 ${JSON.stringify(undo.value)}，实际 ${JSON.stringify(valueAfterUndo)}`);
    }
    await reopenWorkspace();
    const reopenedState = await cardState(undoTarget.decisionId);
    if (reopenedState && reopenedState.hasUndo && !reopenedState.undone) {
      throw new Error("重开后仍提供「撤销」，说明撤销没有被一致记录");
    }
    return { decisionId: undoTarget.decisionId, slotId, valueBeforeUndo, valueAfterUndo, versionBefore: beforeVersion, versionAfter: afterVersion };
  });

  // ---- 场景 4：用户先改目标，再接受旧建议 → 用户的修改受保护 ----
  //
  // 任务书：「过期候选测试必须验证实际 outcome、用户内容及重开持久化，
  // **不能以按钮消失作为成功证据**。」
  //
  // 所以这里三条都必须成立，缺一条判失败：
  //   ① **用户内容**：草稿里那个值仍然是用户写的（被旧建议覆盖 = 失败）；
  //   ② **实际 outcome**：后端的 `stale` 必须为真（过期判定由 `baseEditVersion`
  //      与当前版本比较得出，是后端事实），且该决策在后端视图里有一个**确定的**
  //      resolution（查不到 = 失败）；
  //   ③ **重开持久化**：重开后 ① ② 以及 resolution 都仍然一致。
  // 「接受按钮不见了」**只作为线索记录**（`acceptAvailable`），不作为通过依据 ——
  // 旧版就是只要按钮不在就 return 成功，既没看值、也没看状态、更没重开。
  const staleTarget = reviewItems[2];
  if (!staleTarget) notExecutable("stale-suggestion-protected", NO_CANDIDATES);
  else await scenario("stale-suggestion-protected", async () => {
    const slotId = staleTarget.target?.targetId ?? null;
    if (!slotId) throw new Error("候选项没有可定位的 targetId，无法先做用户编辑");
    const beforeDraft = await readDraft();
    const beforeVersion = Number(beforeDraft.editVersion ?? 0);
    // 走真实编辑器事务写入一个「用户自己的值」，作为随后接受旧建议时的保护对象。
    const userValue = { kind: "text", values: ["E2E-USER-EDIT-PROTECT"] };
    const applied = await call("apply_editor_commands", {
      itemId, baseVersion: beforeVersion, requestId: `e2e-user-edit-${Date.now()}`,
      commands: [{ op: "setAnswer", slotId, value: userValue }],
    });
    if (!applied?.ok) throw new Error(`用户编辑未被接受：${applied?.error}`);
    const midDraft = await readDraft();
    const midVersion = Number(midDraft.editVersion ?? 0);
    const readUserValue = async () => (await readDraft())?.ds?.answerKey?.[slotId] ?? null;
    const assertUserValueIntact = (label, actual) => {
      if (JSON.stringify(actual) !== JSON.stringify(userValue)) {
        throw new Error(`${label}：用户的修改被旧建议覆盖了。期望 ${JSON.stringify(userValue)}，实际 ${JSON.stringify(actual)}`);
      }
    };

    // ②-a 用户改过之后，后端必须认为这批建议过期了。
    const decisionAfterEdit = await readDecision();
    if (decisionAfterEdit.stale !== true) {
      throw new Error(
        `用户已经改到 v${midVersion}，但后端仍认为这批建议不过期（stale=${decisionAfterEdit.stale}，`
        + `baseEditVersion=${String(decisionAfterEdit.baseEditVersion)}）：过期判定没有生效，用户修改没有受到保护`
      );
    }

    // 现在尝试接受那条基于旧值生成的建议（按钮在就真点，不在就如实记录）。
    const acceptAvailable = await session.evaluate(`!!document.querySelector('[data-testid="workspace-recognition-accept-${staleTarget.decisionId}"]')`);
    if (acceptAvailable) {
      await session.clickSelector(`[data-testid="workspace-recognition-accept-${staleTarget.decisionId}"]`);
      await sleep(2000);
    }

    // ① 用户内容：无论按钮在不在，都必须核对值。
    const valueAfter = await readUserValue();
    assertUserValueIntact("接受旧建议之后", valueAfter);

    // ②-b 实际 outcome：该决策在后端视图里必须有确定的 resolution。
    const resolutionAfter = await decisionResolution(staleTarget.decisionId);
    if (resolutionAfter === undefined) {
      throw new Error("这条决策在后端识别视图里查不到，无法确认它到底被怎么处理了");
    }

    // ③ 重开持久化：值、过期判定、resolution 三者都要一致。
    await reopenWorkspace();
    const valueAfterReopen = await readUserValue();
    assertUserValueIntact("重开之后", valueAfterReopen);
    const decisionAfterReopen = await readDecision();
    if (decisionAfterReopen.stale !== true) {
      throw new Error(`重开后过期判定丢失（stale=${decisionAfterReopen.stale}），用户修改不再受保护`);
    }
    const resolutionAfterReopen = await decisionResolution(staleTarget.decisionId);
    if (resolutionAfterReopen !== resolutionAfter) {
      throw new Error(`重开后该决策的 resolution 变了：重开前 ${String(resolutionAfter)}，重开后 ${String(resolutionAfterReopen)}`);
    }

    return {
      decisionId: staleTarget.decisionId,
      slotId,
      userValue,
      userEditVersion: midVersion,
      // 通过依据是这三条，不是「按钮不见了」。
      protectedEvidence: "user-value-intact + backend-stale-flag + survives-reopen",
      acceptAvailable,
      acceptGone: !acceptAvailable,
      staleAfterEdit: decisionAfterEdit.stale,
      baseEditVersion: decisionAfterEdit.baseEditVersion,
      resolutionAfter: resolutionAfter ?? null,
      valueAfter,
      valueAfterReopen,
      resolutionAfterReopen: resolutionAfterReopen ?? null
    };
  });

  // ---- 场景 5：重复点击不重复应用 ----
  const idempotentTarget = reviewItems[3];
  if (!idempotentTarget) notExecutable("idempotent-retry", NO_CANDIDATES);
  else await scenario("idempotent-retry", async () => {
    const beforeDraft = await readDraft();
    const beforeVersion = Number(beforeDraft.editVersion ?? 0);
    // 同一次事件循环里点两次：第二次应被 busy 保护挡掉（按钮已 disabled）。
    await session.evaluate(
      `(() => { const b = document.querySelector('[data-testid="workspace-recognition-accept-${idempotentTarget.decisionId}"]'); if (!b) return false; b.click(); b.click(); return true; })()`
    );
    const vDeadline = Date.now() + 30000;
    let afterDraft = null;
    while (Date.now() < vDeadline) {
      afterDraft = await readDraft();
      if (Number(afterDraft.editVersion ?? 0) > beforeVersion) break;
      await sleep(500);
    }
    await sleep(2500);
    const finalDraft = await readDraft();
    const bumps = Number(finalDraft.editVersion ?? 0) - beforeVersion;
    if (bumps !== 1) throw new Error(`重复点击导致版本推进了 ${bumps} 次（应为 1 次）`);
    return { decisionId: idempotentTarget.decisionId, versionBefore: beforeVersion, versionAfter: finalDraft.editVersion, versionBumps: bumps };
  });

  report.verdict = "pending";
}

try {
  await main();
} catch (error) {
  report.cannotRun = error instanceof CannotRunError;
  report.fatal = { name: error.name, message: String(error.message), appOutput: error.appOutput ?? null };
  console.error(`[recog-buttons] ${report.cannotRun ? "CANNOT-RUN" : "FAIL"} ${error.message}`);
} finally {
  if (session) {
    if (!keep) await session.screenshot("final").catch(() => {});
    const closed = await session.close({ keep });
    report.appOutput = closed.appOutput?.slice(-4000) ?? null;
    report.appProcessExitCode = closed.exitCode;
  }
  report.finishedAt = new Date().toISOString();
  // 判定无条件执行：CANNOT-RUN 时 scenarios 可能为空，也必须走判定而不是留一个初值。
  const verdict = report.cannotRun
    ? { verdict: "cannot-run", exitCode: 3, reason: "环境或构建不满足运行条件，见 fatal", passed: [], failed: [], notExecutable: [] }
    : computeScenarioVerdict({ scenarios: report.scenarios });
  report.verdict = verdict.verdict;
  report.exitCode = verdict.exitCode;
  report.verdictReason = verdict.reason;
  report.summary = { passed: verdict.passed, failed: verdict.failed, notExecutable: verdict.notExecutable };
  const file = writeReport(runDir, report);
  console.log(`[recog-buttons] verdict=${report.verdict} exit=${report.exitCode} reason=${report.verdictReason}`);
  console.log(`[recog-buttons] report=${file}`);
  console.log(`[recog-buttons] scenarios: ${report.scenarios.map((s) => `${s.name}:${s.status}`).join(" | ") || "(无)"}`);
  process.exit(report.exitCode ?? 1);
}
