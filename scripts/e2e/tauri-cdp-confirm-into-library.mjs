#!/usr/bin/env node
// 用户确认入库 + 编辑页选项删除入口的真实应用 e2e（合成语料，不依赖私有 PDF）。
//
// 第一阶段（启动 #1）：
//   预置一篇「识别完成、未通过质量门禁」的阅读卷（job.status=NeedsReview → 题库行 action_required）
//   → 导入页「待确认」里可见它（题库页看不到）→ 点「确认入库」→ 提示「已进入题库」→ 题库页出现它
// 第二阶段（关闭应用，启动 #2，同一个数据目录与 WebView 配置）：
//   → 题库页仍有它（确认状态持久化）→ 点「退回待确认」→ 题库页消失、导入页「待确认」重新出现它
// 第三阶段（同一次启动）：
//   预置一篇选项池型题目（拖放答案位，选项来自题组选项库）→ 打开编辑页 → 「＋ 添加选项」出现在池内部
//   → 新选项的 × 可见（不悬停、透明度足够、尺寸不小于 20×20）→ 点 × 删除 → 选项数回到原值
//
// 证据层级：real-app-e2e（CDP 驱动真实 Tauri/WebView2 界面 + 读取磁盘与后端只读命令的事实）。
// 注意：后端 set_library_item_confirmed 命令尚未上线（另有任务书），确认状态走前端本地兜底
// （appSettings.confirmedLibraryItemIds）；报告中会记录这一点。
// 缺少合成权威稿或 exe 不存在时以 CannotRunError 退出（退出码 3），不伪装成通过。
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import {
  CDP_CHANNEL_LABEL, CDP_CHANNEL_NOTE, CannotRunError,
  assertBuildFresh, buildFreshReport, gitHead, launchTauriAppCdp, repoRoot, sleep, writeReport,
} from "./lib/tauri-cdp-harness.mjs";
import { physicalShadowFor, waitForRows } from "./lib/paper-publish-steps.mjs";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `confirm-into-library-${stamp}`);
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const extraArgs = "--no-sandbox --disable-gpu";
const SYNTHETIC_AUTHORING = path.join(repoRoot, "fixtures", "golden", "synthetic", "ielts", "early-approaches-authoring-v2.json");
const NOW = "2026-11-07T00:00:00Z";


function recordAssertion(report, id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

/**
 * 预置一份阅读卷。`status` 决定题库行阶段：NeedsReview → action_required（识别完成、待确认）。
 * `authoringTransform` 可改写权威稿（用于造出选项池型题目）。
 */
function seedJob(dataDir, label, template, { status, authoringTransform } = {}) {
  const token = crypto.randomBytes(4).toString("hex");
  const jobId = `e2e-confirm-${label}-${token}`;
  const examId = `e2econfirm${label}${token}`;
  const title = `E2E 确认入库 ${label} ${stamp.slice(11, 19)}`;
  const jobDir = path.join(dataDir, "jobs", jobId);
  fs.mkdirSync(jobDir, { recursive: true });
  let authoring = JSON.parse(JSON.stringify(template));
  authoring.jobId = jobId;
  authoring.exam.examId = examId;
  authoring.exam.title = title;
  // 未达到 ready：门禁不通过的草稿（review_required，无 ready 标记）。
  authoring.quality = { ...(authoring.quality ?? {}), state: "review_required" };
  if (authoringTransform) authoring = authoringTransform(authoring);
  const job = {
    jobId, title, status: status ?? "NeedsReview", category: "P1", frequency: "medium", tags: ["e2e-confirm-into-library"],
    sourceFiles: [{
      fileId: `file-e2e-confirm-${label}-${token}`, originalName: `synthetic-confirm-${label}.pdf`, storedName: `e2e-confirm-${label}-${token}.pdf`,
      fileType: "pdf", sha256: "a".repeat(64), sizeBytes: 1, role: "MainQuestion", importedAt: NOW,
    }],
    activeLlmProfileId: null, createdAt: NOW, updatedAt: NOW, currentStep: "Preview",
    issueCounts: { errors: 0, warnings: 0, needsReview: 1 },
  };
  fs.writeFileSync(path.join(jobDir, "job.json"), JSON.stringify(job, null, 2));
  fs.writeFileSync(path.join(jobDir, "authoring-ir-v2.shadow.json"), JSON.stringify(authoring, null, 2));
  fs.writeFileSync(path.join(jobDir, "document-ir-v2.shadow.json"), JSON.stringify(physicalShadowFor(authoring, jobId), null, 2));
  return { jobId, examId, title };
}

/** 选项池型题目：两个拖放答案位挂在题目本身（不在正文段落里），选项库共享。 */
function poolAuthoring(authoring) {
  const task = authoring.taskGroups[0];
  task.taskType = "matching_features";
  if (task.instructionSignature) task.instructionSignature.taskType = "matching_features";
  for (const group of task.responseGroups) group.kind = "matching";
  for (const slotId of task.responseGroups[0].slotIds) {
    const slot = authoring.answerSlots[slotId];
    if (!slot) continue;
    slot.interaction = "dragdrop";
    slot.hostType = "prompt";
    delete slot.hostNodeId;
  }
  return authoring;
}

async function importReviewHas(session, id) {
  return await session.evaluate(`!!document.querySelector('[data-testid="import-group-review"] [data-item-id="${id}"]')`);
}

async function libraryHas(session, id) {
  return await session.evaluate(`!!document.querySelector('[data-testid="library-page"] tr[data-item-id="${id}"]')`);
}

async function gotoRoute(session, hash) {
  await session.evaluate(`(() => { window.location.hash = ${JSON.stringify(hash)}; return true; })()`);
}

async function main() {
  if (!fs.existsSync(SYNTHETIC_AUTHORING)) throw new CannotRunError(`合成权威稿缺失：${SYNTHETIC_AUTHORING}`);
  if (!fs.existsSync(exePath)) throw new CannotRunError(`exe 不存在（先运行 node scripts/e2e/build-app.mjs）：${exePath}`);
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  const template = JSON.parse(fs.readFileSync(SYNTHETIC_AUTHORING, "utf8"));

  const report = {
    task: "confirm-into-library",
    scope: "导入页待确认→确认入库→题库出现→重启后仍在→退回待确认；编辑页选项池的「添加选项」与 × 删除",
    channel: CDP_CHANNEL_LABEL, channelNote: CDP_CHANNEL_NOTE,
    evidenceLevel: "real-app-e2e (CDP automation channel; UI driven; disk + read-only command facts)",
    backendConfirmCommand: "not-yet-implemented (local fallback in appSettings.confirmedLibraryItemIds)",
    identity: { exePath, commit: gitHead(repoRoot), buildFresh: buildFreshReport(fresh) },
    assertions: [], consoleErrors: [], pageExceptions: [], screenshots: [],
  };
  const nasRoot = path.join(runDir, "nas-library");
  fs.mkdirSync(nasRoot, { recursive: true });
  const dataDir = path.join(runDir, "appdata", "data");
  const confirmTarget = seedJob(dataDir, "review", template);
  const poolTarget = seedJob(dataDir, "pool", template, { status: "NeedsReview", authoringTransform: poolAuthoring });
  const ids = [confirmTarget.jobId, poolTarget.jobId];
  report.seeded = { confirmTarget, poolTarget };

  let verdict = "failed";
  let session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  const shot = async (name) => {
    const file = await session.screenshot(name);
    if (file) report.screenshots.push(file);
    return file;
  };
  try {
    report.browserArgs = session.browserArgs;
    const migrated = await waitForRows(session, ids, 90000);
    const rowOf = (id) => migrated.find((row) => row.id === id);
    recordAssertion(report, "C-0 seeded-items-migrated",
      ids.every((id) => rowOf(id)), `后端条目=${ids.filter((id) => rowOf(id)).length}/2`);
    if (!ids.every((id) => rowOf(id))) throw new Error("预置条目没有被启动迁移读入");
    recordAssertion(report, "C-0b seeded-item-is-action-required",
      rowOf(confirmTarget.jobId)?.status === "action_required", `后端状态=${rowOf(confirmTarget.jobId)?.status}`);

    // ── 第一阶段：导入页待确认 → 确认入库 → 题库可见 ──
    await gotoRoute(session, "#/library");
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });
    recordAssertion(report, "C-1 not-in-library-before-confirm", !(await libraryHas(session, confirmTarget.jobId)),
      "未确认的识别完成题不在题库");

    await gotoRoute(session, "#/import");
    await session.waitFor(`!!document.querySelector('[data-testid="import-group-review"]')`, { timeoutMs: 40000, label: "import-page" });
    await session.waitFor(`!!document.querySelector('[data-testid="import-group-review"] [data-item-id="${confirmTarget.jobId}"]')`,
      { timeoutMs: 30000, label: "review-has-target" });
    recordAssertion(report, "C-2 import-review-lists-target", await importReviewHas(session, confirmTarget.jobId), "导入页「待确认」里有它");
    await shot("01-import-review-awaiting-confirm");

    await session.clickSelector(`[data-testid="import-group-review"] [data-item-id="${confirmTarget.jobId}"] [data-testid="import-row-confirm"]`);
    await session.waitFor(`/已进入题库/.test(document.querySelector('[data-testid="import-notice"]')?.textContent ?? '')`,
      { timeoutMs: 20000, label: "confirm-notice" });
    const noticeText = await session.evaluate(`document.querySelector('[data-testid="import-notice"]')?.textContent ?? null`);
    await session.waitFor(`!document.querySelector('[data-testid="import-group-review"] [data-item-id="${confirmTarget.jobId}"]')`,
      { timeoutMs: 20000, label: "left-review" });
    recordAssertion(report, "C-3 confirm-moves-row-out-of-review", !(await importReviewHas(session, confirmTarget.jobId)),
      `确认提示=${JSON.stringify(noticeText)}`);
    await shot("02-import-after-confirm");

    await gotoRoute(session, "#/library");
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"] tr[data-item-id="${confirmTarget.jobId}"]')`,
      { timeoutMs: 30000, label: "library-has-target" });
    recordAssertion(report, "C-4 confirmed-item-in-library", await libraryHas(session, confirmTarget.jobId), "题库里出现了它");
    await shot("03-library-after-confirm");

    // ── 第二阶段：重启后仍在题库 → 退回待确认 ──
    await session.close();
    await sleep(1500);
    session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
    await waitForRows(session, ids, 90000);
    await gotoRoute(session, "#/library");
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"] tr[data-item-id="${confirmTarget.jobId}"]')`,
      { timeoutMs: 60000, label: "library-after-restart" });
    recordAssertion(report, "C-5 confirmation-survives-restart", await libraryHas(session, confirmTarget.jobId), "重启后题库仍有它（确认状态持久化）");
    await shot("04-library-after-restart");

    await session.clickSelector(`tr[data-item-id="${confirmTarget.jobId}"] [data-testid="library-row-unconfirm"]`);
    await session.waitFor(`!document.querySelector('[data-testid="library-page"] tr[data-item-id="${confirmTarget.jobId}"]')`,
      { timeoutMs: 20000, label: "left-library" });
    recordAssertion(report, "C-6 unconfirm-removes-from-library", !(await libraryHas(session, confirmTarget.jobId)), "退回后题库里不再有它");

    await gotoRoute(session, "#/import");
    await session.waitFor(`!!document.querySelector('[data-testid="import-group-review"] [data-item-id="${confirmTarget.jobId}"]')`,
      { timeoutMs: 30000, label: "review-after-unconfirm" });
    recordAssertion(report, "C-7 unconfirmed-item-back-in-review", await importReviewHas(session, confirmTarget.jobId), "导入页「待确认」重新出现它");
    await shot("05-import-after-unconfirm");

    // ── 第三阶段：编辑页选项池的添加 / 删除 与 × 可见性 ──
    await gotoRoute(session, `#/items/${poolTarget.jobId}`);
    await session.waitFor(`!!document.querySelector('[data-option-pool-task] .v2-answer-option')`, { timeoutMs: 60000, label: "option-pool" });
    await session.evaluate(`document.querySelector('[data-option-pool-task]').scrollIntoView({ block: 'center' }) || true`);
    const before = await session.evaluate(`document.querySelectorAll('[data-option-pool-task] .v2-answer-option').length`);
    await shot("06-editor-option-pool-before");

    const poolAddInside = await session.evaluate(`(() => {
      const pool = document.querySelector('[data-option-pool-task]');
      const add = pool?.querySelector('.v2-option-add');
      return { present: !!add, lastChild: !!add && pool.lastElementChild === add };
    })()`);
    recordAssertion(report, "C-8 add-button-inside-pool", poolAddInside.present && poolAddInside.lastChild,
      `池内「添加选项」=${JSON.stringify(poolAddInside)}`);

    await session.clickSelector(`[data-option-pool-task] .v2-option-add`);
    await session.waitFor(`document.querySelectorAll('[data-option-pool-task] .v2-answer-option').length === ${before + 1}`,
      { timeoutMs: 30000, label: "option-added" });
    const addedCount = await session.evaluate(`document.querySelectorAll('[data-option-pool-task] .v2-answer-option').length`);
    recordAssertion(report, "C-9 add-option-appends", addedCount === before + 1, `选项数 ${before} → ${addedCount}`);

    // 新选项的 × 不悬停时也可见：透明度足够、尺寸达到 20×20、落在选项 chip 内。
    const visibility = await session.evaluate(`(() => {
      const rows = document.querySelectorAll('[data-option-pool-task] .v2-answer-option');
      const row = rows[rows.length - 1];
      const btn = row.querySelector('.v2-option-delete');
      if (!btn) return { present: false };
      const rect = btn.getBoundingClientRect();
      const chip = row.getBoundingClientRect();
      const token = row.querySelector('.v2-answer-option-token').getBoundingClientRect();
      const style = getComputedStyle(btn);
      return {
        present: true, label: btn.getAttribute('aria-label'), opacity: Number(style.opacity), disabled: btn.disabled,
        width: rect.width, height: rect.height,
        insideChip: rect.left >= chip.left && rect.right <= chip.right && rect.top >= chip.top && rect.bottom <= chip.bottom,
        clearOfText: rect.left >= token.right - 1 || rect.left >= chip.right - 32,
      };
    })()`);
    recordAssertion(report, "C-10 delete-x-visible-without-hover",
      visibility.present && visibility.opacity >= 0.5 && visibility.width >= 20 && visibility.height >= 20 && !visibility.disabled && visibility.insideChip,
      `× 可见性=${JSON.stringify(visibility)}`);
    await shot("07-editor-option-added-x-visible");

    // 悬停到新选项上，× 加深（仅作为截图证据，不作为断言的前提）。
    const hoverAt = await session.evaluate(`(() => {
      const rows = document.querySelectorAll('[data-option-pool-task] .v2-answer-option');
      const r = rows[rows.length - 1].getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    })()`);
    await session.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: hoverAt.x, y: hoverAt.y }, 5000);
    await sleep(300);
    await shot("08-editor-option-hover");

    await session.clickSelector(`[data-option-pool-task] .v2-answer-option:last-child .v2-option-delete`);
    await session.waitFor(`document.querySelectorAll('[data-option-pool-task] .v2-answer-option').length === ${before}`,
      { timeoutMs: 30000, label: "option-deleted" });
    const afterDelete = await session.evaluate(`document.querySelectorAll('[data-option-pool-task] .v2-answer-option').length`);
    recordAssertion(report, "C-11 delete-x-removes-option", afterDelete === before, `选项数 ${addedCount} → ${afterDelete}`);
    await shot("09-editor-option-deleted");

    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    verdict = report.assertions.every((a) => a.ok) ? "passed" : "failed";
  } catch (error) {
    report.error = String(error?.message ?? error);
    await shot("fail-final").catch(() => {});
  } finally {
    report.verdict = verdict;
    report.failedAssertions = report.assertions.filter((a) => !a.ok).map((a) => a.id);
    report.screenshotDir = runDir;
    await session.close?.().catch?.(() => {});
  }
  const file = writeReport(runDir, report);
  console.log(`[confirm-into-library] verdict=${report.verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
  console.log(`[confirm-into-library] report=${file}`);
  process.exitCode = report.verdict === "passed" ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  console.error(cannotRun ? `[confirm-into-library] CANNOT RUN: ${error.message}` : error);
  process.exitCode = cannotRun ? 3 : 1;
});
