#!/usr/bin/env node
/**
 * 判断题（TFNG/YNNG）版式验收（WebView2 CDP 通道）：逐题题干 + 选项干净 + 指令区干净。
 *
 * 复现对象：fixtures/golden/private-real/chili-peppers.pdf（与 "7. P1 - Chili peppers" 字节一致）。
 * 缺陷背景（三个一起复现于这一份卷子）：
 *   1) 题干不落位：6 条陈述堆在顶部公共 prompt 块，下面再出 1、2、3… 每题一组 TRUE/FALSE/NOT GIVEN。
 *      官方版式：`1  陈述文字` 紧跟该题的 TRUE / FALSE / NOT GIVEN。
 *   2) 选项脏：说明区文字（"TRUE if the statement agrees…"）被塞进选项 content，或 content 与
 *      label 相同导致 "YES YES" 式重复。
 *   3) 指令区脏：group-2 笔记标题与小标题（"The role of capsaicin" 等）被吞进 instructions，
 *      同一内容在 stimulus 又出现一遍。
 *
 * 断言（编辑模式与学生预览模式各验一遍，任一不成立即 FAIL）：
 *   T1 per-slot-statement      q1..q6 每个 [data-question-id] 容器内同时有题号与对应陈述文字
 *                              （q1 含 "Archaeological evidence from pots"，q4 含 "Explorers from Portugal"）
 *   T2 no-shared-prompt-block  不存在包含 ≥2 条不同陈述的公共 prompt 块（.v2-response-prompt）
 *   T3 clean-option-labels     每题选项标签恰为 TRUE / FALSE / NOT GIVEN，选项行不含 "if the statement"，
 *                              且整行文本不出现 label 重复（如 "YES YES"）
 *   T4 clean-instructions      group-2 的 instructions（权威 IR 层）不含 "capsaicin"、
 *                              以 "Write your answers in boxes 7-13 on your answer sheet." 结尾；
 *   T5 notes-heading-once      界面上（题目栏）笔记标题 "The role of capsaicin" 只出现一次
 *
 * T4 属于权威稿（真实 IR）断言；其余是界面断言。IR 从 `get_workspace_item` 读取，
 * 与工作区渲染走的是同一份本地稿。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-tfng-layout.mjs [--pdf <path>] [--keep] [--no-diagnostic-args]
 *        [--tolerate-concurrent-edits]
 * 退出码：0 通过 / 1 失败 / 3 环境不满足（沿用 lib/tauri-cdp-harness.mjs 约定）
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

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const fixtureIdx = process.argv.indexOf("--pdf");
const fixturePath = path.resolve(
  fixtureIdx >= 0 ? process.argv[fixtureIdx + 1] : path.join(repoRoot, "fixtures", "golden", "private-real", "chili-peppers.pdf")
);
// 本机必需：不加这两个参数 WebView2 的 renderer 会在中途崩（与 workspace-layout 脚本同因）。
const extraArgs = process.argv.includes("--no-diagnostic-args") ? "" : "--no-sandbox --disable-gpu";
const runDir = path.join(
  repoRoot,
  "artifacts",
  "e2e-cdp",
  `run-tfng-layout-${new Date().toISOString().replace(/[:.]/g, "-")}`
);

// 每题陈述的取样文字（大小写不敏感、空白归一后做 includes）。q1/q4 是任务指定的锚点；
// 全部取自 Chili 真实识别产物 group-1-prompt-1..6（见 golden fixture 与既有 e2e 产物）。
const SLOT_STATEMENTS = {
  q1: "Archaeological evidence from pots",
  q2: "kept food from spoiling",
  q3: "Christopher Columbus",
  q4: "Explorers from Portugal",
  q5: "purely psychological effect",
  q6: "heat scale based on the collective judgement",
};
const SLOT_IDS = Object.keys(SLOT_STATEMENTS);
const TFNG_LABELS = ["TRUE", "FALSE", "NOT GIVEN"];

const report = {
  task: "tfng-per-slot-layout",
  scope: "判断题逐题题干/选项干净/指令区干净的界面与 IR 验收",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  diagnosticRun: Boolean(extraArgs),
  runProfile: extraArgs ? "cdp-diagnostic" : "cdp-default",
  fixturePath,
  fixtureSha256: null,
  probes: {},
  assertions: [],
  consoleErrors: [],
  pageExceptions: [],
};

const norm = (s) => String(s ?? "").replace(/\s+/g, " ").trim();

/** 编辑/学生两种模式下都用的界面采集脚本：返回每题容器、选项行、prompt 块的取证。 */
function collectUiSnapshotFn() {
  return `(() => {
    const norm = (s) => String(s ?? "").replace(/\\s+/g, " ").trim();
    const slotIds = ${JSON.stringify(SLOT_IDS)};
    const statements = ${JSON.stringify(SLOT_STATEMENTS)};
    const out = { slots: {}, promptBlocks: [], questionPaneText: null, capsaicinHeadingCount: 0 };

    const pane = document.querySelector('.workspace-body .v2-question-pane')
      || document.querySelector('.workspace-student-preview .v2-question-pane')
      || document.querySelector('.v2-question-pane');
    out.questionPaneText = pane ? norm(pane.innerText) : null;
    if (out.questionPaneText) {
      // 笔记标题的"渲染次数"按**精确等值节点**计：bullet 正文（"Tewksbury considers
      // the role of capsaicin …"）合法地含有这个短语，includes 计数会误报；这里只数
      // textContent 恰好等于标题、且不含同样命中的后代的元素（外层包装 div 也会
      // 精确等值，用无命中后代去重）。
      const TITLE = "The role of capsaicin";
      const exact = [...(pane ? pane.querySelectorAll("*") : [])].filter(
        (el) => norm(el.textContent) === TITLE
      );
      out.capsaicinHeadingCount = exact.filter(
        (el) => !exact.some((other) => other !== el && el.contains(other))
      ).length;
    }

    for (const slotId of slotIds) {
      const el = [...document.querySelectorAll('[data-question-id="' + slotId + '"]')]
        .find((n) => n.classList.contains("v2-slot-question")) ?? null;
      if (!el) { out.slots[slotId] = { present: false }; continue; }
      const number = norm(el.querySelector(".v2-slot-number")?.textContent ?? "");
      const options = [...el.querySelectorAll(".v2-choice-item")].map((item) => {
        const label = norm(item.querySelector("strong")?.textContent ?? "");
        const rowText = norm(item.textContent);
        // 整词出现次数（不用正则：本段整体在模板字符串里，字符类里的美元花括号会被插值）。
        const padded = " " + rowText.toUpperCase() + " ";
        const needle = " " + label.toUpperCase() + " ";
        const labelOccurrences = label ? padded.split(needle).length - 1 : 0;
        return { label, rowText, labelOccurrences };
      });
      out.slots[slotId] = {
        present: true,
        number,
        containerText: norm(el.textContent),
        options,
        optionRowContainsInstruction: options.some((o) => /if the statement/i.test(o.rowText)),
        // label 重复（"YES YES"）：整行里同一 label 以整词出现 ≥2 次。
        duplicatedLabel: options.some((o) => o.labelOccurrences >= 2),
      };
    }

    // 公共 prompt 块取证：每个 .v2-response-prompt 里包含几条不同的陈述。
    out.promptBlocks = [...document.querySelectorAll(".v2-response-prompt")].map((el) => {
      const text = norm(el.textContent);
      const hits = slotIds.filter((slotId) => text.toUpperCase().includes(String(statements[slotId] ?? "").toUpperCase()));
      return { textLength: text.length, statementHits: hits, sample: text.slice(0, 120) };
    });
    return out;
  })()`;
}

function recordAssertion(id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

/** 从权威稿（真实 IR）拍平一段 ContentNodes 文本（与 workspace-layout 的 textOf 同思路）。 */
function irTextOf(nodes) {
  const out = [];
  const seen = new Set();
  const walk = (n) => {
    if (!n || typeof n !== "object" || seen.has(n)) return;
    seen.add(n);
    if (Array.isArray(n)) { for (const child of n) walk(child); return; }
    if (typeof n.text === "string") out.push(n.text);
    for (const key of ["children", "items", "rows", "cells", "steps", "caption", "options", "prompt", "instructions", "stimulus"]) {
      if (n[key]) walk(n[key]);
    }
  };
  walk(nodes);
  return norm(out.join(" "));
}

/** T4：权威 IR 层断言（instructions 干净）。 */
function evaluateIr(ir) {
  const results = [];
  const ds = ir?.value?.ds;
  if (!ds) {
    results.push(["T4 clean-instructions", false, "权威稿 ds 为空，IR 断言没有执行"]);
    return results;
  }
  const groups = (ds.taskGroups ?? []).map((t) => ({
    taskId: t.taskId,
    taskType: t.taskType ?? null,
    instructions: irTextOf(t.instructions),
    stimulus: irTextOf(t.stimulus),
  }));
  report.irGroups = groups.map(({ taskId, taskType, instructions, stimulus }) => ({
    taskId, taskType,
    instructionChars: instructions.length,
    instructionTail: instructions.slice(-140),
    stimulusChars: stimulus.length,
  }));
  const note = groups.find((g) => g.taskType === "note_completion") ?? groups[1] ?? null;
  if (!note) {
    results.push(["T4 clean-instructions", false, "权威稿里找不到笔记填空题组"]);
    return results;
  }
  const noCapsaicin = !/capsaicin/i.test(note.instructions);
  const closes = note.instructions.toLowerCase().endsWith("write your answers in boxes 7-13 on your answer sheet.");
  results.push(["T4 clean-instructions", noCapsaicin && closes,
    `group「${note.taskId}」instructions ${note.instructions.length} 字符；不含 capsaicin=${noCapsaicin}；` +
    `结尾正确=${closes}（末尾 80 字符：「${note.instructions.slice(-80)}」）`]);
  return results;
}

/** T1–T3 / T5：界面断言（一个模式一份快照）。 */
function evaluateUi(snapshot, tag) {
  const results = [];
  const ui = snapshot.ui;
  if (!ui) {
    results.push([`${tag} T1 per-slot-statement`, false, "界面快照缺失"]);
    return results;
  }

  // T1：每个题号容器里同时有题号与对应陈述。
  const t1 = SLOT_IDS.map((slotId) => {
    const slot = ui.slots[slotId];
    if (!slot?.present) return [slotId, false, `容器缺失`];
    const hasNumber = /^\d+$/.test(slot.number);
    const hasStatement = slot.containerText.toUpperCase().includes(SLOT_STATEMENTS[slotId].toUpperCase());
    return [slotId, hasNumber && hasStatement,
      `题号「${slot.number}」含题号=${hasNumber}，含陈述「${SLOT_STATEMENTS[slotId]}」=${hasStatement}`];
  });
  results.push([`${tag} T1 per-slot-statement`, t1.every(([, ok]) => ok),
    t1.map(([slotId, ok, d]) => `${slotId}: ${ok ? "OK" : `FAIL(${d})`}`).join("；")]);

  // T2：公共 prompt 块不得包含 ≥2 条不同陈述。
  const badBlocks = ui.promptBlocks.filter((b) => b.statementHits.length >= 2);
  results.push([`${tag} T2 no-shared-prompt-block`, badBlocks.length === 0,
    `页面共 ${ui.promptBlocks.length} 个 .v2-response-prompt，` +
    (badBlocks.length ? `其中 ${badBlocks.length} 个含多条陈述（${badBlocks.map((b) => `hits=[${b.statementHits.join(",")}] sample="${b.sample}"`).join("；")}）` : "没有块吞下多条陈述")]);

  // T3：选项标签干净。
  const optionIssues = [];
  for (const slotId of SLOT_IDS) {
    const slot = ui.slots[slotId];
    if (!slot?.present) continue;
    for (const option of slot.options) {
      if (!TFNG_LABELS.includes(option.label.toUpperCase())) {
        optionIssues.push(`${slotId}: 非法标签「${option.label}」`);
      }
      if (option.rowText && /if the statement/i.test(option.rowText)) {
        optionIssues.push(`${slotId}: 选项行含说明文字（${option.rowText.slice(0, 80)}）`);
      }
      if (slot.duplicatedLabel) {
        optionIssues.push(`${slotId}: label 重复渲染`);
      }
    }
  }
  const optionCount = SLOT_IDS.reduce((acc, slotId) => acc + (ui.slots[slotId]?.options.length ?? 0), 0);
  results.push([`${tag} T3 clean-option-labels`, optionIssues.length === 0 && optionCount >= SLOT_IDS.length * 3,
    `${optionCount} 个选项行；${optionIssues.length ? "问题：" + optionIssues.slice(0, 6).join("；") : "标签全部为 TRUE/FALSE/NOT GIVEN，无说明文字、无重复"}`]);

  // T5：笔记标题只出现一次。
  results.push([`${tag} T5 notes-heading-once`, ui.capsaicinHeadingCount === 1,
    `题目栏内 "The role of capsaicin" 出现 ${ui.capsaicinHeadingCount} 次（要求恰好 1 次）`]);
  return results;
}

async function main() {
  const tolerateConcurrentEdits = process.argv.includes("--tolerate-concurrent-edits");
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits });
  report.identity = {
    exePath,
    exeSha256: sha256File(exePath),
    fixturePath,
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: buildFreshReport(fresh),
  };
  report.fixtureSha256 = sha256File(fixturePath);
  if (!fs.existsSync(fixturePath)) throw new CannotRunError(`夹具不存在：${fixturePath}`);

  // PDF 走「选择文件夹」通道：harness 把 PDF2TEST_AUTOMATION_PDF_DIR 指向 <runDir>/pdfs。
  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  const stagedFixture = path.join(runDir, "pdfs", path.basename(fixturePath));
  fs.copyFileSync(fixturePath, stagedFixture);

  const session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
  });
  let verdict = "failed";
  try {
    report.browserArgs = session.browserArgs;

    // 1) 导入夹具 → 等草稿就绪 → 打开工作区（与真人一致：题库页 → 导入 → 点开）。
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library" });
    const before = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
    // 抽屉挂载到点击处理器生效之间有一帧间隙；workspace-layout 实测稳定需要这口喘息。
    await sleep(600);
    await session.clickSelector('[data-testid="import-pick-folder"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked" });
    await session.clickSelector('[data-testid="import-start"]');
    let itemId = null;
    const deadline = Date.now() + 90000;
    while (Date.now() < deadline && !itemId) {
      const ids = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
      itemId = (ids ?? []).find((id) => !(before ?? []).includes(id)) ?? null;
      if (!itemId) await sleep(1000);
    }
    if (!itemId) throw new CannotRunError("导入后未出现新的题库行");
    report.identity.itemId = itemId;

    // 等本地稿落盘（ds 非空）——草稿未就绪时题面没有渲染，断言无从谈起。
    const draftDeadline = Date.now() + 120000;
    while (Date.now() < draftDeadline) {
      const r = await session.invoke("get_workspace_item", { itemId });
      if (r?.ok && r.value?.ds) break;
      await sleep(2000);
    }
    await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace" });
    await sleep(1200);

    // 2) 权威稿（真实 IR）留档 + T4。
    const ir = await session.invoke("get_workspace_item", { itemId });
    report.irPresent = Boolean(ir?.ok && ir.value?.ds);
    for (const [id, ok, detail] of evaluateIr(ir)) recordAssertion(id, ok, detail);

    // 3) 编辑模式界面快照。
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"] .exam-canvas-v2')`, { timeoutMs: 30000, label: "edit-canvas" });
    await sleep(600);
    await session.screenshot("01-edit-mode");
    const editUi = await session.evaluate(collectUiSnapshotFn());
    report.probes.edit = editUi;
    for (const [id, ok, detail] of evaluateUi({ ui: editUi }, "edit")) recordAssertion(id, ok, detail);

    // 4) 学生预览模式：同三条界面断言。
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-student-preview"]')`, { timeoutMs: 20000, label: "student-preview" });
    await sleep(800);
    await session.screenshot("02-student-preview");
    const studentUi = await session.evaluate(collectUiSnapshotFn());
    report.probes.student = studentUi;
    for (const [id, ok, detail] of evaluateUi({ ui: studentUi }, "student")) recordAssertion(id, ok, detail);

    // 5) 控制台异常留档（不作为硬断言，但写进报告）。
    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    report.pageExceptions = session.cdp.events
      .filter((e) => e.method === "Runtime.exceptionThrown")
      .map((e) => (e.params?.exceptionDetails?.exception?.description ?? e.params?.exceptionDetails?.text ?? "").slice(0, 400));

    verdict = report.assertions.every((a) => a.ok) ? "passed" : "failed";
    report.verdict = verdict;
    report.failedAssertions = report.assertions.filter((a) => !a.ok).map((a) => a.id);
  } finally {
    if (session) {
      if (!keep) await session.screenshot("99-final").catch(() => {});
      const closed = await session.close({ keep });
      report.appOutput = closed.appOutput ?? null;
      report.appProcessExitCode = closed.exitCode;
      if (closed.appOutput) fs.writeFileSync(path.join(runDir, "app-output.log"), closed.appOutput);
    }
    report.verdict = verdict;
    const file = writeReport(runDir, report);
    console.log(`[tfng-layout] verdict=${verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions ?? [])}`);
    console.log(`[tfng-layout] report=${file}`);
  }
  process.exitCode = verdict === "passed" ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  report.cannotRun = cannotRun;
  report.verdict = cannotRun ? "cannot-run" : "failed";
  report.error = String(error?.message ?? error);
  try {
    writeReport(runDir, report);
  } catch {}
  console.error(`[tfng-layout] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${report.error}`);
  process.exitCode = cannotRun ? 3 : 1;
});
