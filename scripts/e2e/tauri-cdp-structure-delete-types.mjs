#!/usr/bin/env node
// 编辑页结构删除 / 添加：矩阵（配对表）逐行删除、判断题（TFNG）逐题删除、矩阵底部「＋ 添加一题」的真实应用 e2e。
//
// 预置（由 contracts/fixtures/authoring-structure-parity.json 改造，合成语料）：
//   task-1 「配对」（matching_features）：q1–q4 各自一个单题作答组，共享选项库 A–E、逐选框作答 → 渲染为 MatchingMatrix；
//   task-2 「判断」（true_false_not_given）：q5、q6 同一作答组，选项库 TRUE / FALSE / NOT GIVEN → 逐题列表。
// 步骤（全部经真实 Tauri 界面操作，状态以真实 IPC 与磁盘为准）：
//   1. 初始 6 题，矩阵可见；
//   2. 矩阵第 2 行的 × 删除 → 该行消失、矩阵仍在；Ctrl+Z 恢复，Ctrl+Shift+Z 重做；
//   3. 判断题第 6 题的 × 删除 → 该题消失；工具栏「撤销」恢复，「重做」再删；
//   4. 矩阵底部「＋ 添加一题」→ 新题出现（题号 = 全卷最大 + 1）；工具栏「撤销」移除；
//      观察项（非断言）：追加到矩阵末行所在的组后，该组不再是单题组，矩阵退回逐题列表；
//   5. 保存 → 后端题稿题号与界面一致；判断题选项库仍是 TRUE / FALSE / NOT GIVEN；
//   6. 关闭并重启同一数据目录 → 结构仍一致；
//   7. 学生预览：题号与后端一致，无编译失败。
//
// 证据层级：real-app-e2e（CDP 驱动真实 Tauri/WebView2 界面 + 真实 IPC 读取持久化结果；学生预览为作者端的学生画布）。
// 逐题删除不弹确认框，本脚本不替换 window.confirm。
import fs from "node:fs";
import path from "node:path";
import {
  CDP_CHANNEL_LABEL, CDP_CHANNEL_NOTE, CannotRunError,
  assertBuildFresh, buildFreshReport, gitHead, launchTauriAppCdp, repoRoot, sleep, writeReport,
} from "./lib/tauri-cdp-harness.mjs";
import { seedReadingJob, waitForRows } from "./lib/paper-publish-steps.mjs";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `structure-delete-types-${stamp}`);
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const extraArgs = "--no-sandbox --disable-gpu";
const PARITY = path.join(repoRoot, "contracts", "fixtures", "authoring-structure-parity.json");

/** 由 parity 初稿改造出「矩阵 + 判断题」两道大题（共 6 题）。 */
export function typesTemplate(parity) {
  const doc = structuredClone(parity.initial);
  doc.passage.content = doc.passage.content.filter((node) => node.id !== "diagram"); // 合成图示热点 ID 重复，与本次结构编辑无关
  const [matchTask, tfngTask] = doc.taskGroups;
  const promptOf = (group, slotId) => (group.prompt ?? []).filter((node) => node.id === doc.answerSlots[slotId].hostNodeId);

  // task-1：每题一个单题组，matching + per_slot + radio，共享选项库 → 矩阵。
  const matchGroups = ["q1", "q2", "q3", "q4"].map((slotId, index) => {
    const source = matchTask.responseGroups.find((group) => group.slotIds.includes(slotId));
    return {
      ...structuredClone(source),
      responseGroupId: `group-1-${index}`, kind: "matching", assignment: "per_slot",
      slotIds: [slotId], cardinality: { min: 1, max: 1, exact: 1 },
      prompt: promptOf(source, slotId),
    };
  });
  matchTask.taskType = "matching_features";
  matchTask.instructionSignature = { ...matchTask.instructionSignature, taskType: "matching_features", answerAssignment: "per_slot", selectionCardinality: { min: 1, max: 1, exact: 1 }, expectedSlotCount: 4 };
  matchTask.responseGroups = matchGroups;
  for (const slotId of ["q1", "q2", "q3", "q4"]) doc.answerSlots[slotId].interaction = "radio";

  // task-2：判断题，q5 与 q6 同组，选项库改为 TRUE / FALSE / NOT GIVEN。
  const tfngGroupSource = tfngTask.responseGroups[0];
  const tfngPrompt = [...(tfngGroupSource.prompt ?? []), ...(doc.taskGroups[2].responseGroups[0].prompt ?? [])];
  tfngTask.taskType = "true_false_not_given";
  tfngTask.instructionSignature = { ...tfngTask.instructionSignature, taskType: "true_false_not_given", expectedQuestionNumbers: [5, 6], expectedSlotCount: 2, answerAssignment: "per_slot", selectionCardinality: { min: 1, max: 1, exact: 1 } };
  tfngTask.displayRange = { kind: "set", values: [5, 6] };
  tfngTask.optionBank = {
    ...tfngTask.optionBank,
    options: ["TRUE", "FALSE", "NOT GIVEN"].map((label, index) => ({ ...tfngTask.optionBank.options[index], optionId: `option-2-${"ABC"[index]}`, label, content: [] })),
  };
  tfngTask.responseGroups = [{ ...tfngGroupSource, slotIds: ["q5", "q6"], prompt: tfngPrompt, cardinality: { min: 1, max: 1, exact: 1 } }];
  doc.answerKey.q5 = { kind: "option", labels: ["TRUE"], assignment: "per_slot" };
  doc.answerKey.q6 = { kind: "option", labels: ["FALSE"], assignment: "per_slot" };
  doc.answerSlots.q5.interaction = "radio";
  doc.answerSlots.q6.interaction = "radio";
  doc.taskGroups = [matchTask, tfngTask];
  return doc;
}

function assertEqual(report, id, actual, expected, detail = "") {
  const ok = JSON.stringify(actual) === JSON.stringify(expected);
  report.assertions.push({ id, ok, actual, expected, detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} actual=${JSON.stringify(actual)}${ok ? "" : ` expected=${JSON.stringify(expected)}`} ${detail}`);
  return ok;
}

function assertTrue(report, id, value, detail = "") {
  return assertEqual(report, id, Boolean(value), true, detail);
}

async function invokeValue(session, command, args = {}) {
  const result = await session.invoke(command, args);
  if (!result?.ok) throw new Error(`${command}: ${result?.error ?? JSON.stringify(result)}`);
  return result.value;
}

/** 真实 CDP 键盘事件。modifiers：Ctrl=2，Shift=8。 */
async function pressKey(session, key, code, keyCode, modifiers) {
  const base = { key, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode, modifiers };
  await session.cdp.send("Input.dispatchKeyEvent", { type: "keyDown", ...base }, 5000);
  await session.cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...base }, 5000);
}

const deleteControl = (n) => `!!document.querySelector('[data-testid="exam-canvas-v2-author"] button[aria-label="删除第 ${n} 题"]')`;
const deleteControlGone = (n) => `!document.querySelector('[data-testid="exam-canvas-v2-author"] button[aria-label="删除第 ${n} 题"]')`;
const matrixRendered = `!!document.querySelector('[data-testid="exam-canvas-v2-author"] [data-testid="matching-matrix"]')`;

/** 编辑画布上现有的题号（取自每题/每行的 × aria-label）。 */
async function authorSlotNumbers(session) {
  return session.evaluate(`[...new Set([...document.querySelectorAll('[data-testid="exam-canvas-v2-author"] button[aria-label^="删除第 "]')].map((b) => Number(b.getAttribute('aria-label').match(/\\d+/)[0])))].sort((a, b) => a - b)`);
}

async function backendState(session, itemId) {
  const item = await invokeValue(session, "get_workspace_item", { itemId });
  return {
    numbers: Object.values(item.ds.answerSlots).map((slot) => slot.questionNumber).sort((a, b) => a - b),
    ds: item.ds,
  };
}

async function main() {
  if (!fs.existsSync(PARITY)) throw new CannotRunError(`合成题稿缺失：${PARITY}`);
  if (!fs.existsSync(exePath)) throw new CannotRunError(`exe 不存在（先运行 node scripts/e2e/build-app.mjs）：${exePath}`);
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  const template = typesTemplate(JSON.parse(fs.readFileSync(PARITY, "utf8")));

  const report = {
    task: "structure-delete-types",
    scope: "编辑页：矩阵行删除（撤销/重做）、判断题逐题删除、矩阵底部添加一题、保存、重启、学生预览",
    channel: CDP_CHANNEL_LABEL, channelNote: CDP_CHANNEL_NOTE,
    evidenceLevel: "real-app-e2e (CDP automation channel; UI driven; real IPC and reopen)",
    identity: { exePath, commit: gitHead(repoRoot), buildFresh: buildFreshReport(fresh) },
    assertions: [], screenshots: [], observations: [],
  };
  const dataDir = path.join(runDir, "appdata", "data");
  const target = seedReadingJob(dataDir, "types", template, stamp);
  const itemId = target.jobId;
  report.seeded = target;

  let verdict = "failed";
  let session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  const shot = async (name) => {
    const file = await session.screenshot(name);
    if (file) report.screenshots.push(file);
    return file;
  };
  const openEditor = async () => {
    await session.evaluate(`(() => { window.location.hash = ${JSON.stringify(`#/items/${itemId}`)}; return true; })()`);
    await session.waitFor(deleteControl(1), { timeoutMs: 60000, label: "author-canvas-q1" });
  };
  try {
    report.browserArgs = session.browserArgs;
    const rows = await waitForRows(session, [itemId], 90000);
    assertTrue(report, "T-0 seeded-item-in-backend", rows.some((row) => row.id === itemId), `item=${itemId}`);
    const initial = await backendState(session, itemId);
    assertEqual(report, "T-0b initial-backend-questions", initial.numbers, [1, 2, 3, 4, 5, 6]);
    assertEqual(report, "T-0c initial-task-types", initial.ds.taskGroups.map((task) => task.taskType), ["matching_features", "true_false_not_given"]);

    await openEditor();
    assertEqual(report, "T-1 initial-ui-questions", await authorSlotNumbers(session), [1, 2, 3, 4, 5, 6]);
    assertTrue(report, "T-1b matrix-rendered", await session.evaluate(matrixRendered), "配对题以矩阵呈现");
    await shot("types-01-editor-initial");

    // 2. 矩阵第 2 行的 × → 删除这一行（即删除该单题组），矩阵仍在。
    await session.clickSelector('[data-testid="exam-canvas-v2-author"] button[aria-label="删除第 2 题"]');
    await session.waitFor(deleteControlGone(2), { timeoutMs: 20000, label: "matrix-row-2-gone" });
    assertEqual(report, "T-2 matrix-row-delete", await authorSlotNumbers(session), [1, 3, 4, 5, 6]);
    assertTrue(report, "T-2b matrix-still-rendered-after-row-delete", await session.evaluate(matrixRendered));
    await shot("types-02-after-matrix-row-delete");
    await pressKey(session, "z", "KeyZ", 90, 2);
    await session.waitFor(deleteControl(2), { timeoutMs: 20000, label: "matrix-row-back" });
    assertEqual(report, "T-2c ctrl-z-restores-matrix-row", await authorSlotNumbers(session), [1, 2, 3, 4, 5, 6]);
    await pressKey(session, "Z", "KeyZ", 90, 2 | 8);
    await session.waitFor(deleteControlGone(2), { timeoutMs: 20000, label: "matrix-row-redone" });
    assertEqual(report, "T-2d ctrl-shift-z-redoes-matrix-row", await authorSlotNumbers(session), [1, 3, 4, 5, 6]);

    // 3. 判断题 q6 的 ×（同组还有 q5，允许删除）→ 工具栏撤销 → 重做。
    await session.clickSelector('[data-testid="exam-canvas-v2-author"] button[aria-label="删除第 6 题"]');
    await session.waitFor(deleteControlGone(6), { timeoutMs: 20000, label: "tfng-q6-gone" });
    assertEqual(report, "T-3 tfng-question-delete", await authorSlotNumbers(session), [1, 3, 4, 5]);
    await shot("types-03-after-tfng-delete");
    await session.clickSelector('button[aria-label="撤销"]');
    await session.waitFor(deleteControl(6), { timeoutMs: 20000, label: "tfng-q6-back" });
    assertEqual(report, "T-3b toolbar-undo-restores-tfng-question", await authorSlotNumbers(session), [1, 3, 4, 5, 6]);
    await session.clickSelector('button[aria-label="重做"]');
    await session.waitFor(deleteControlGone(6), { timeoutMs: 20000, label: "tfng-q6-redone" });
    assertEqual(report, "T-3c toolbar-redo-removes-tfng-question", await authorSlotNumbers(session), [1, 3, 4, 5]);

    // 4. 矩阵底部「＋ 添加一题」→ 新题（题号 = 最大 + 1）；工具栏撤销移除，重做恢复。
    await session.clickSelector('[data-testid="exam-canvas-v2-author"] .v2-matching-footer > .v2-question-append');
    await session.waitFor(deleteControl(6), { timeoutMs: 20000, label: "matrix-append-added" });
    assertEqual(report, "T-4 matrix-append-adds-next-number", await authorSlotNumbers(session), [1, 3, 4, 5, 6]);
    report.observations.push({
      id: "O-1",
      note: "追加到矩阵末行所在的组后，该组不再是单题组，矩阵前提不满足，任务退回逐题列表",
      matrixRenderedAfterAppend: await session.evaluate(matrixRendered),
    });
    await shot("types-04-after-matrix-append");
    await session.clickSelector('button[aria-label="撤销"]');
    await session.waitFor(deleteControlGone(6), { timeoutMs: 20000, label: "matrix-append-undone" });
    assertEqual(report, "T-4b toolbar-undo-removes-appended", await authorSlotNumbers(session), [1, 3, 4, 5]);
    await session.clickSelector('button[aria-label="重做"]');
    await session.waitFor(deleteControl(6), { timeoutMs: 20000, label: "matrix-append-redone" });
    assertEqual(report, "T-4c toolbar-redo-restores-appended", await authorSlotNumbers(session), [1, 3, 4, 5, 6]);

    // 5. 保存 → 后端核对。
    await session.clickSelector('[data-testid="workspace-save"]');
    await session.waitFor(`document.querySelector('[data-testid="workspace-save-state"]')?.textContent === '已保存'`, { timeoutMs: 40000, label: "saved" });
    const saved = await backendState(session, itemId);
    assertEqual(report, "T-5 backend-matches-ui-after-save", saved.numbers, [1, 3, 4, 5, 6]);
    const tfngOptions = saved.ds.taskGroups.find((task) => task.taskId === "task-2")?.optionBank?.options.map((option) => option.label);
    assertEqual(report, "T-5b tfng-option-bank-kept", tfngOptions, ["TRUE", "FALSE", "NOT GIVEN"]);
    report.savedSlotIds = Object.keys(saved.ds.answerSlots).sort();

    // 6. 关闭并重启同一数据目录。
    await session.close();
    await sleep(1500);
    session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
    await waitForRows(session, [itemId], 90000);
    const reopened = await backendState(session, itemId);
    assertEqual(report, "T-6 restart-keeps-structure", reopened.numbers, [1, 3, 4, 5, 6]);
    assertEqual(report, "T-6b restart-keeps-task-types", reopened.ds.taskGroups.map((task) => task.taskType), ["matching_features", "true_false_not_given"]);
    await openEditor();
    assertEqual(report, "T-6c reopened-ui-matches", await authorSlotNumbers(session), [1, 3, 4, 5, 6]);
    await shot("types-05-reopened-editor");

    // 7. 学生预览：题号与后端一致，无编译失败面板，判断题选项正常渲染。
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-student"]')`, { timeoutMs: 60000, label: "student-preview" });
    const preview = await session.evaluate(`({
      error: document.querySelector('[data-testid="workspace-preview-error"]')?.textContent ?? null,
      ids: [...new Set([...document.querySelectorAll('[data-testid="exam-canvas-v2-student"] .v2-slot-question .v2-slot-number, [data-testid="exam-canvas-v2-student"] .v2-matching-item .v2-slot-number')].map((n) => Number(n.textContent.trim())).filter((n) => Number.isInteger(n) && n > 0))].sort((a, b) => a - b),
      tfngOptionsRendered: [...document.querySelectorAll('[data-testid="exam-canvas-v2-student"] .v2-slot-question')].some((q) => /TRUE/.test(q.textContent) && /NOT GIVEN/.test(q.textContent)),
    })`);
    assertTrue(report, "T-7a student-preview-compiles", preview.error === null, JSON.stringify(preview.error));
    assertEqual(report, "T-7b student-preview-matches-backend", preview.ids, [1, 3, 4, 5, 6]);
    assertTrue(report, "T-7c tfng-options-rendered-in-preview", preview.tfngOptionsRendered);
    await shot("types-06-student-preview");
    report.student = preview;

    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    verdict = report.assertions.every((a) => a.ok) ? "passed" : "failed";
  } catch (error) {
    report.error = String(error?.stack ?? error?.message ?? error);
    await shot("fail-final").catch(() => {});
  } finally {
    report.verdict = verdict;
    report.failedAssertions = report.assertions.filter((a) => !a.ok).map((a) => a.id);
    report.screenshotDir = runDir;
    await session?.close?.().catch?.(() => {});
  }
  const file = writeReport(runDir, report);
  console.log(`[structure-delete-types] verdict=${report.verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
  console.log(`[structure-delete-types] report=${file}`);
  process.exitCode = report.verdict === "passed" ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  console.error(cannotRun ? `[structure-delete-types] CANNOT RUN: ${error.message}` : error);
  process.exitCode = cannotRun ? 3 : 1;
});
