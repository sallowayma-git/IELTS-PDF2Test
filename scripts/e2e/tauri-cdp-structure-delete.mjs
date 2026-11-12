#!/usr/bin/env node
// 编辑页结构删除 / 添加 / 撤销的真实应用 e2e（合成语料，不依赖私有 PDF）。
//
// 预置：parity 合成题稿（3 大题、6 题；大题 1 含两个作答组）。大题 1 改为「标题配对」（matching_features，拖放作答）。
// 步骤（全部经真实 Tauri 界面操作，状态以真实 IPC 与磁盘为准）：
//   1. 打开编辑页，初始 6 题；
//   2. 点第 2 题行末的 ×（删除第 2 题）→ 该题从界面消失；
//   3. Ctrl+Z（键盘）撤销 → 第 2 题回来；Ctrl+Shift+Z 重做 → 再次消失；
//   4. 作答组 group-2-0 末尾点「＋ 添加一题」→ 新增第 7 题；工具栏「撤销」→ 第 7 题消失；「重做」→ 回来；
//   5. 大题 task-3 的「⋯」→「删除整道大题」：先取消（确认框返回否）→ 题仍在；再确认 → 第 6 题消失；
//      确认框文字须写明题号范围与「可用撤销」；
//   6. 点「保存」→ 后端 get_workspace_item 的题稿与界面一致；
//   7. 关闭应用、重新启动同一数据目录 → 题稿结构仍一致（磁盘持久化）；
//   8. 切到学生预览 → 学生端渲染的题号集合与后端一致，且无编译失败面板。
//
// 证据层级：real-app-e2e（CDP 驱动真实 Tauri/WebView2 界面 + 真实 IPC 读取持久化结果）。
// 确认框由脚本替换为可控的 window.confirm（取消/确认两种分支都覆盖）；这是界面层唯一的桩。
import fs from "node:fs";
import path from "node:path";
import {
  CDP_CHANNEL_LABEL, CDP_CHANNEL_NOTE, CannotRunError,
  assertBuildFresh, buildFreshReport, gitHead, launchTauriAppCdp, repoRoot, sleep, writeReport,
} from "./lib/tauri-cdp-harness.mjs";
import { seedReadingJob, waitForRows } from "./lib/paper-publish-steps.mjs";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `structure-delete-${stamp}`);
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const extraArgs = "--no-sandbox --disable-gpu";
const PARITY = path.join(repoRoot, "contracts", "fixtures", "authoring-structure-parity.json");

/** 把大题 1（q1–q4）改为标题配对：拖放作答、作答组类型为 matching。 */
function headingMatchingTemplate(doc) {
  // 与后端交付的 .planning 验证脚本一致：去掉合成图示（其热点 ID 重复，会让学生端编译失败，与本次结构编辑无关）。
  doc.passage.content = doc.passage.content.filter((node) => node.id !== "diagram");
  const task = doc.taskGroups[0];
  task.taskType = "matching_features";
  if (task.instructionSignature) task.instructionSignature.taskType = "matching_features";
  for (const group of task.responseGroups) {
    group.kind = "matching";
    for (const slotId of group.slotIds) {
      const slot = doc.answerSlots[slotId];
      if (slot) slot.interaction = "dragdrop";
    }
  }
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

/** 作者画布上某题的 × 是否存在（列表行与矩阵行共用 aria-label）。 */
const deleteControl = (n) => `!!document.querySelector('[data-testid="exam-canvas-v2-author"] button[aria-label="删除第 ${n} 题"]')`;
const deleteControlGone = (n) => `!document.querySelector('[data-testid="exam-canvas-v2-author"] button[aria-label="删除第 ${n} 题"]')`;

/** 编辑画布上现有的题号（取自每题的 × aria-label）。 */
async function authorSlotNumbers(session) {
  return session.evaluate(`[...new Set([...document.querySelectorAll('[data-testid="exam-canvas-v2-author"] button[aria-label^="删除第 "]')].map((b) => Number(b.getAttribute('aria-label').match(/\\d+/)[0])))].sort((a, b) => a - b)`);
}

async function backendSlots(session, itemId) {
  const item = await invokeValue(session, "get_workspace_item", { itemId });
  return {
    numbers: Object.values(item.ds.answerSlots).map((slot) => slot.questionNumber).sort((a, b) => a - b),
    ds: item.ds,
    editVersion: item.editVersion,
  };
}

async function main() {
  if (!fs.existsSync(PARITY)) throw new CannotRunError(`合成题稿缺失：${PARITY}`);
  if (!fs.existsSync(exePath)) throw new CannotRunError(`exe 不存在（先运行 node scripts/e2e/build-app.mjs）：${exePath}`);
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  const parity = JSON.parse(fs.readFileSync(PARITY, "utf8"));
  const template = headingMatchingTemplate(structuredClone(parity.initial));

  const report = {
    task: "structure-delete",
    scope: "编辑页 × 删除一题、撤销/重做（键盘与按钮）、添加一题、删除整道大题（确认框）、保存、重启、学生预览一致",
    channel: CDP_CHANNEL_LABEL, channelNote: CDP_CHANNEL_NOTE,
    evidenceLevel: "real-app-e2e (CDP automation channel; UI driven; real IPC and reopen; window.confirm stubbed for cancel and accept)",
    identity: { exePath, commit: gitHead(repoRoot), buildFresh: buildFreshReport(fresh) },
    assertions: [], screenshots: [],
  };
  const dataDir = path.join(runDir, "appdata", "data");
  const target = seedReadingJob(dataDir, "structure", template, stamp);
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
    await session.evaluate(`(() => {
      window.__confirmLog = [];
      window.__confirmAnswer = true;
      window.confirm = (message) => { window.__confirmLog.push(String(message)); return window.__confirmAnswer; };
      return true;
    })()`);
  };
  /** 通过「⋯」菜单点「删除整道大题」（菜单用 DOM 点击触发同一个 React 处理函数）。 */
  const clickTaskDelete = () => session.evaluate(`(() => {
    const menu = document.querySelector('[data-group-id="task-3"] .v2-structure-menu');
    menu.open = true;
    [...menu.querySelectorAll('button')].find((button) => button.textContent === '删除整道大题').click();
    return true;
  })()`);
  try {
    report.browserArgs = session.browserArgs;
    const rows = await waitForRows(session, [itemId], 90000);
    assertTrue(report, "S-0 seeded-item-in-backend", rows.some((row) => row.id === itemId), `item=${itemId}`);
    const initial = await backendSlots(session, itemId);
    assertEqual(report, "S-0b initial-backend-questions", initial.numbers, [1, 2, 3, 4, 5, 6]);

    await openEditor();
    assertEqual(report, "S-1 initial-ui-questions", await authorSlotNumbers(session), [1, 2, 3, 4, 5, 6]);
    await shot("01-editor-initial");

    // 2. 点第 2 题的 ×。
    await session.clickSelector('[data-testid="exam-canvas-v2-author"] button[aria-label="删除第 2 题"]');
    await session.waitFor(deleteControlGone(2), { timeoutMs: 20000, label: "q2-gone" });
    assertEqual(report, "S-2 delete-one-question-via-x", await authorSlotNumbers(session), [1, 3, 4, 5, 6]);
    await shot("02-after-delete-q2");

    // 3a. Ctrl+Z 撤销（键盘）。
    await pressKey(session, "z", "KeyZ", 90, 2);
    await session.waitFor(deleteControl(2), { timeoutMs: 20000, label: "q2-back-after-ctrl-z" });
    assertEqual(report, "S-3 ctrl-z-restores-question", await authorSlotNumbers(session), [1, 2, 3, 4, 5, 6]);
    // 3b. Ctrl+Shift+Z 重做（键盘）。
    await pressKey(session, "Z", "KeyZ", 90, 2 | 8);
    await session.waitFor(deleteControlGone(2), { timeoutMs: 20000, label: "q2-gone-after-redo" });
    assertEqual(report, "S-3b ctrl-shift-z-redoes-deletion", await authorSlotNumbers(session), [1, 3, 4, 5, 6]);
    await shot("03-after-undo-redo-keyboard");

    // 4. 作答组 group-2-0（只有 q5）末尾添加一题；工具栏撤销/重做。
    await session.clickSelector('[data-response-group-id="group-2-0"] .v2-question-append');
    await session.waitFor(deleteControl(7), { timeoutMs: 20000, label: "q7-added" });
    assertEqual(report, "S-4 append-adds-next-number", await authorSlotNumbers(session), [1, 3, 4, 5, 6, 7]);
    await session.clickSelector('button[aria-label="撤销"]');
    await session.waitFor(deleteControlGone(7), { timeoutMs: 20000, label: "q7-undone" });
    assertEqual(report, "S-4b toolbar-undo-removes-added", await authorSlotNumbers(session), [1, 3, 4, 5, 6]);
    await session.clickSelector('button[aria-label="重做"]');
    await session.waitFor(deleteControl(7), { timeoutMs: 20000, label: "q7-redone" });
    assertEqual(report, "S-4c toolbar-redo-restores-added", await authorSlotNumbers(session), [1, 3, 4, 5, 6, 7]);
    await shot("04-after-append-and-redo");

    // 5a. 整道大题（task-3，只有 q6）：先取消确认。
    await session.evaluate(`(() => { window.__confirmAnswer = false; window.__confirmLog = []; return true; })()`);
    await clickTaskDelete();
    await sleep(600);
    const cancelLog = await session.evaluate(`window.__confirmLog.slice()`);
    assertTrue(report, "S-5a confirm-dialog-states-range-and-undo",
      cancelLog.length === 1 && /第 6 题/.test(cancelLog[0]) && /可用撤销/.test(cancelLog[0]), JSON.stringify(cancelLog));
    assertEqual(report, "S-5b cancel-keeps-task", await authorSlotNumbers(session), [1, 3, 4, 5, 6, 7]);

    // 5b. 再次确认。
    await session.evaluate(`(() => { window.__confirmAnswer = true; window.__confirmLog = []; return true; })()`);
    await clickTaskDelete();
    await session.waitFor(deleteControlGone(6), { timeoutMs: 20000, label: "task-3-gone" });
    assertEqual(report, "S-5c accept-deletes-whole-task", await authorSlotNumbers(session), [1, 3, 4, 5, 7]);
    await shot("05-after-delete-task");

    // 6. 保存，并核对后端题稿。
    await session.clickSelector('[data-testid="workspace-save"]');
    await session.waitFor(`document.querySelector('[data-testid="workspace-save-state"]')?.textContent === '已保存'`, { timeoutMs: 40000, label: "saved" });
    const saved = await backendSlots(session, itemId);
    assertEqual(report, "S-6 backend-matches-ui-after-save", saved.numbers, [1, 3, 4, 5, 7]);
    assertEqual(report, "S-6b task-3-removed-in-backend", Object.keys(saved.ds.answerSlots).includes("q6"), false);
    const group2 = saved.ds.taskGroups.find((task) => task.taskId === "task-2")?.responseGroups[0];
    assertEqual(report, "S-6c appended-slot-in-group-2", group2?.slotIds.length, 2);
    report.savedSlotIds = Object.keys(saved.ds.answerSlots).sort();

    // 7. 关闭并重新启动同一数据目录，核对持久化。
    await session.close();
    await sleep(1500);
    session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
    await waitForRows(session, [itemId], 90000);
    const reopened = await backendSlots(session, itemId);
    assertEqual(report, "S-7 restart-keeps-structure", reopened.numbers, [1, 3, 4, 5, 7]);
    assertEqual(report, "S-7b restart-keeps-task-ids", reopened.ds.taskGroups.map((task) => task.taskId), ["task-1", "task-2"]);
    await openEditor();
    assertEqual(report, "S-7c reopened-ui-matches", await authorSlotNumbers(session), [1, 3, 4, 5, 7]);
    await shot("06-reopened-editor");

    // 8. 学生预览：题号集合与后端一致，且没有编译失败面板。
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-student"]')`, { timeoutMs: 60000, label: "student-preview" });
    // 学生端按题号显示（data-question-id 是稳定的 slotId，新题的 slotId 带后缀，不能用它比较题号）。
    const preview = await session.evaluate(`({
      error: document.querySelector('[data-testid="workspace-preview-error"]')?.textContent ?? null,
      ids: [...new Set([...document.querySelectorAll('[data-testid="exam-canvas-v2-student"] .v2-slot-question .v2-slot-number, [data-testid="exam-canvas-v2-student"] .v2-matching-item .v2-slot-number')].map((n) => Number(n.textContent.trim())).filter((n) => Number.isInteger(n) && n > 0))].sort((a, b) => a - b)
    })`);
    assertTrue(report, "S-8a student-preview-compiles", preview.error === null, JSON.stringify(preview.error));
    assertEqual(report, "S-8b student-preview-matches-backend", preview.ids, [1, 3, 4, 5, 7]);
    await shot("07-student-preview-after-delete");
    report.student = preview;

    // 9. 制造乱序并全卷重新编号。往大题 1 的第一个作答组末尾追加一题（题号取全卷最大 + 1 = 8），
    //    它在界面上排在大题 2 的题之前，于是显示顺序变成乱序（如 1 3 4 8 5 7）。
    //    「全卷按顺序重新编号」→ 二次确认（stub 记录确认文字）→ 题号按显示顺序连续；撤销恢复乱序；重做回到连续；
    //    保存、重启后保持连续。
    const inDocumentOrder = `(() => { const seen = []; for (const b of document.querySelectorAll('[data-testid="exam-canvas-v2-author"] button[aria-label^="删除第 "]')) { const n = Number(b.getAttribute('aria-label').match(/\\d+/)[0]); if (!seen.includes(n)) seen.push(n); } return seen; })()`;
    await session.clickSelector('[data-testid="workspace-mode-edit"]');
    await session.waitFor(deleteControl(1), { timeoutMs: 60000, label: "back-to-author-canvas" });
    await session.clickSelector('[data-response-group-id="group-1-0"] .v2-question-append');
    await session.waitFor(deleteControl(8), { timeoutMs: 20000, label: "q8-appended-to-task-1" });
    const disordered = await session.evaluate(inDocumentOrder);
    assertTrue(report, "R-9a disordered-display-order", JSON.stringify(disordered) !== JSON.stringify([...disordered].sort((a, b) => a - b)), `display=${JSON.stringify(disordered)}`);
    const beforeRenumber = disordered;
    report.observations = [{
      id: "R-O1", note: "质量提示是否出现「题号不连续」快捷入口（观察项，取决于后端是否给出该警告）",
      hintVisibleAtDisorder: await session.evaluate(`!!document.querySelector('[data-testid="workspace-renumber-hint"]')`),
    }];

    await session.evaluate(`(() => { window.__confirmAnswer = true; window.__confirmLog = []; return true; })()`);
    await session.clickSelector('button[aria-label="更多操作"]');
    await session.clickSelector('[data-testid="workspace-renumber-exam"]');
    await session.waitFor(`window.__confirmLog.length === 1`, { timeoutMs: 20000, label: "renumber-confirm" });
    const renumberLog = await session.evaluate(`window.__confirmLog.slice()`);
    assertTrue(report, "R-9b confirm-states-change-and-undo",
      /题号会改变，答案与统计不受影响，可撤销/.test(renumberLog[0] ?? "") && /原题号/.test(renumberLog[0] ?? ""), JSON.stringify(renumberLog[0]?.slice(0, 120)));
    await session.waitFor(`(${inDocumentOrder}).join(',') === '1,2,3,4,5,6'`, { timeoutMs: 20000, label: "renumbered-six" });
    const renumbered = await session.evaluate(inDocumentOrder);
    assertEqual(report, "R-9c renumbered-display-continuous", renumbered, [1, 2, 3, 4, 5, 6]);
    assertEqual(report, "R-9d renumbered-ui-sorted", await authorSlotNumbers(session), [1, 2, 3, 4, 5, 6]);
    await shot("08-after-exam-renumber");

    // 9e. 撤销恢复乱序原题号；重做回到连续。
    await session.clickSelector('button[aria-label="撤销"]');
    await session.waitFor(`(${inDocumentOrder}).join(',') === '${beforeRenumber.join(",")}'`, { timeoutMs: 20000, label: "renumber-undone" });
    assertEqual(report, "R-9e toolbar-undo-restores-numbers", await session.evaluate(inDocumentOrder), beforeRenumber);
    await session.clickSelector('button[aria-label="重做"]');
    await session.waitFor(`(${inDocumentOrder}).join(',') === '1,2,3,4,5,6'`, { timeoutMs: 20000, label: "renumber-redone" });
    assertEqual(report, "R-9f redo-renumber", await session.evaluate(inDocumentOrder), [1, 2, 3, 4, 5, 6]);

    // 9g. 学生预览的题号连续且与后端一致。
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-student"]')`, { timeoutMs: 60000, label: "student-preview-renumbered" });
    const renumberedPreview = await session.evaluate(`[...new Set([...document.querySelectorAll('[data-testid="exam-canvas-v2-student"] .v2-slot-question .v2-slot-number, [data-testid="exam-canvas-v2-student"] .v2-matching-item .v2-slot-number')].map((n) => Number(n.textContent.trim())).filter((n) => Number.isInteger(n) && n > 0))]`);
    assertEqual(report, "R-9g student-preview-renumbered-continuous", renumberedPreview, [1, 2, 3, 4, 5, 6]);
    await shot("09-student-preview-renumbered");

    // 9h. 保存 → 后端连续；重启后仍连续。
    await session.clickSelector('[data-testid="workspace-mode-edit"]');
    await session.waitFor(deleteControl(1), { timeoutMs: 60000, label: "edit-before-save" });
    await session.clickSelector('[data-testid="workspace-save"]');
    await session.waitFor(`document.querySelector('[data-testid="workspace-save-state"]')?.textContent === '已保存'`, { timeoutMs: 40000, label: "renumber-saved" });
    const renumberedSaved = await backendSlots(session, itemId);
    assertEqual(report, "R-9h backend-renumbered-after-save", renumberedSaved.numbers, [1, 2, 3, 4, 5, 6]);
    await session.close();
    await sleep(1500);
    session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
    await waitForRows(session, [itemId], 90000);
    const renumberedReopened = await backendSlots(session, itemId);
    assertEqual(report, "R-9i restart-keeps-renumbered", renumberedReopened.numbers, [1, 2, 3, 4, 5, 6]);
    await openEditor();
    assertEqual(report, "R-9j reopened-ui-renumbered", await authorSlotNumbers(session), [1, 2, 3, 4, 5, 6]);
    await shot("10-reopened-renumbered");

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
  console.log(`[structure-delete] verdict=${report.verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
  console.log(`[structure-delete] report=${file}`);
  process.exitCode = report.verdict === "passed" ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  console.error(cannotRun ? `[structure-delete] CANNOT RUN: ${error.message}` : error);
  process.exitCode = cannotRun ? 3 : 1;
});
