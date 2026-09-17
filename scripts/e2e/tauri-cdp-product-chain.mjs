#!/usr/bin/env node
/**
 * 真实 Tauri 产品链路（WebView2 CDP 通道）：
 *   真实 PDF 导入 → 本地识别 → 打开工作区 → 编辑正文 → 保存 → 返回重开验证 →
 *   切到学生预览 → 在预览里作答 → 回到编辑 → 再保存 → 重开验证 → 发布到 NAS 目录。
 *
 * 全部步骤都发生在**真实 exe** 里：真实 Rust 后端、真实 SQLite、真实文件系统、
 * 真实内嵌前端。没有浏览器替身、没有 dev fallback、没有 mock 后端。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-product-chain.mjs [--keep] [--pdf <path>] [--run-dir <dir>]
 *        [--scope=full|edit-preview] [--expect-blocked] [--diagnostic-args] [--tolerate-concurrent-edits]
 *
 * 判定（见 `lib/chain-verdict.mjs`，纯函数 + 回归测试）：
 *   - 必需步骤**缺失/未执行** → incomplete（2）；**failed** → failed（1）；
 *     **blocked**（含发布被质量门禁拦下）→ blocked（4）。以上都**不得**报 passed。
 *   - 步骤全过但产物不完整（manifest / 题目 JS / 资源清单缺失）→ failed。
 *   - `--expect-blocked`：门禁正确拦下坏题时判 `passed-negative-case`（0），
 *     这是**独立负例通过，不代表发布成功**。
 *   - `--scope=edit-preview`：只验编辑/预览，判 `passed-specialty`（0），
 *     名字与完整链区分开，避免「专项绿」被当成「发布链绿」。
 *   - CANNOT-RUN → 3。
 *
 * 运行档案：默认**不带**测试专用安全参数（`runProfile=cdp-default`）；
 * 传 `--diagnostic-args` 才加 `--no-sandbox --disable-gpu`（`runProfile=cdp-diagnostic`）。
 * 两类证据必须分开记录，诊断运行不得当作默认产品路径通过。
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
  createStepRecorder,
  gitHead,
  gitWorktreeClean,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  sleep,
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";
import {
  computeChainVerdict,
  EDIT_PREVIEW_REQUIRED_STEPS,
  evaluatePublication,
  FULL_CHAIN_REQUIRED_STEPS,
  PUBLISH_STEP,
} from "./lib/chain-verdict.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const pdfIdx = process.argv.indexOf("--pdf");
const pdfPath = pdfIdx >= 0
  ? path.resolve(process.argv[pdfIdx + 1])
  : path.join(repoRoot, "fixtures", "parser", "complex-reading.pdf");
const runDirIdx = process.argv.indexOf("--run-dir");
const extraIdx = process.argv.indexOf("--extra-args");
// 测试专用安全参数（放宽沙箱 / 关 GPU）默认**不开**：默认运行必须贴近真实产品路径。
// 需要它们时显式传 `--diagnostic-args`，报告里会记成 `runProfile=cdp-diagnostic`。
const diagnosticArgsRequested = process.argv.includes("--diagnostic-args");
const extraArgs = extraIdx >= 0
  ? (process.argv[extraIdx + 1] ?? "")
  : diagnosticArgsRequested ? "--no-sandbox --disable-gpu" : "";
const scopeIdx = process.argv.findIndex((a) => a.startsWith("--scope="));
const scope = scopeIdx >= 0 ? process.argv[scopeIdx].slice("--scope=".length) : "full-chain";
const expectBlocked = process.argv.includes("--expect-blocked");
const runDir = runDirIdx >= 0
  ? path.resolve(process.argv[runDirIdx + 1])
  : path.join(repoRoot, "artifacts", "e2e-cdp", `run-chain-${new Date().toISOString().replace(/[:.]/g, "-")}`);

if (scope !== "full-chain" && scope !== "edit-preview-specialty") {
  console.error(`[chain] 未知 --scope：${scope}（允许 full-chain | edit-preview-specialty）`);
  process.exit(2);
}

const EDITED_MARKER = "E2E EDIT CHECK 42";

const report = {
  task: "task1+task2-real-tauri-product-chain",
  scope,
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  // 诊断运行标记保留：带测试专用安全参数的运行不得当作默认产品路径通过。
  diagnosticRun: Boolean(extraArgs),
  runProfile: extraArgs ? "cdp-diagnostic" : "cdp-default",
  securityArgs: extraArgs ? extraArgs.split(/\s+/).filter(Boolean) : [],
  expectBlocked,
  requiredSteps: scope === "edit-preview-specialty" ? EDIT_PREVIEW_REQUIRED_STEPS : FULL_CHAIN_REQUIRED_STEPS,
  startedAt: new Date().toISOString(),
  runDir,
  identity: {
    exePath,
    exeSha256: null,
    pdfPath,
    pdfSha256: null,
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: null,
  },
  steps: [],
  verdict: "failed",
};

let session = null;
let recorder = null;

async function main() {
  const tolerateConcurrentEdits = process.argv.includes("--tolerate-concurrent-edits");
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);
  if (!fs.existsSync(pdfPath)) throw new CannotRunError(`验收 PDF 不存在：${pdfPath}`);
  report.identity.pdfSha256 = sha256File(pdfPath);

  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  const stagedPdf = path.join(runDir, "pdfs", path.basename(pdfPath));
  fs.copyFileSync(pdfPath, stagedPdf);

  // 真实导入有两条产品入口，自动化钩子也分两条：
  //   - 「选择文件夹」→ `pick_pdf_folder_sources_core`，只列目录里的 **PDF**（list_pdf_files_in_dir）；
  //   - 「选择文件」  → `automation_source_files_from_env`，读 PDF2TEST_AUTOMATION_SOURCE_FILES，任意扩展名。
  // DOCX 只能走第二条，否则会被 PDF-only 的目录过滤静默丢弃（本轮踩到过）。
  const isPdf = /\.pdf$/i.test(pdfPath);
  report.identity.importEntry = isPdf ? "pick-folder (pdf-only hook)" : "pick-files (source-files hook)";

  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    appEnv: isPdf ? {} : { PDF2TEST_AUTOMATION_SOURCE_FILES: stagedPdf }
  });
  report.identity.browserArgs = session.browserArgs;
  recorder = createStepRecorder({ session, artifactsDir: runDir });

  const bodyText = () => session.evaluate("document.body ? document.body.innerText : ''");

  // ---- 1. 题库页加载 ----
  await recorder.run("library-page-loads", async () => {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });
    // 云端识别保持关闭：本链路验证本地识别 + 编辑 + 预览 + 发布。
    await session.evaluate(`(() => { window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false })); location.hash = "#/library"; return true; })()`);
    await session.cdp.send("Page.reload", {}, 30000).catch(() => {});
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page-after-reload" });
    await session.screenshot("01-library");
    return { url: await session.evaluate("location.href") };
  });

  // ---- 2. 真实 PDF 导入 ----
  let importedItemId = null;
  await recorder.run("import-pdf-via-folder-hook", async () => {
    const before = await session.evaluate(
      `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`
    );
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
    await session.clickSelector(isPdf ? '[data-testid="import-pick-folder"]' : '[data-testid="import-pick-files"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked-files" });
    const picked = await session.evaluate(`[...document.querySelectorAll('[data-testid="import-picked-files"] li')].map(li => li.innerText)`);
    await session.screenshot("02-import-drawer");
    await session.clickSelector('[data-testid="import-start"]');

    const deadline = Date.now() + 60000;
    while (Date.now() < deadline && !importedItemId) {
      const ids = await session.evaluate(
        `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`
      );
      importedItemId = (ids ?? []).find((id) => !(before ?? []).includes(id)) ?? null;
      if (!importedItemId) await sleep(1000);
    }
    if (!importedItemId) throw new Error(`导入后未出现新的题库行（导入前 ${(before ?? []).length} 行）`);
    return { itemId: importedItemId, pickedFiles: picked, priorRowCount: (before ?? []).length };
  });

  // ---- 3. 本地识别跑到稳定阶段 ----
  await recorder.run("background-pipeline-reaches-stable-stage", async () => {
    const selector = `[data-item-id="${importedItemId}"]`;
    const deadline = Date.now() + 300000;
    let lastText = "";
    while (Date.now() < deadline) {
      lastText = String(await session.evaluate(
        `(() => { const r = document.querySelector(${JSON.stringify(selector)}); return r ? r.innerText.replace(/\\s+/g, ' ').trim() : ''; })()`
      ) ?? "");
      if (/待检查|可发布|失败|已发布/.test(lastText)) return { rowText: lastText };
      await sleep(1500);
    }
    throw new Error(`行未在限时内进入稳定阶段；最后文本：${lastText || "(无行)"}`);
  });

  // ---- 4. 打开工作区 ----
  await recorder.run("workspace-opens", async () => {
    await session.clickSelector(`[data-item-id="${importedItemId}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "exam-workspace" });
    await session.waitFor(`!!document.querySelector('.v2-passage-pane .v2-text')`, { timeoutMs: 40000, label: "passage-text" });
    const loadError = await session.evaluate(`(() => { const e = document.querySelector('.workspace-load-error'); return e ? e.innerText.replace(/\\s+/g,' ').trim() : null; })()`);
    if (loadError) throw new Error(`工作区打开失败：${loadError}`);
    await session.screenshot("03-workspace-edit");
    return { itemId: importedItemId, title: await session.evaluate(`(() => { const t = document.querySelector('[data-testid="workspace-title"]'); return t ? t.innerText.trim() : null; })()`) };
  });

  // ---- 5. 编辑正文并保存 ----
  await recorder.run("edit-body-text-and-save", async () => {
    let opened = false;
    for (let attempt = 0; attempt < 4 && !opened; attempt += 1) {
      await session.clickSelector(".v2-passage-pane .v2-text");
      opened = Boolean(await session.waitFor(
        `!!document.querySelector('textarea[aria-label="编辑题目文字"]')`,
        { timeoutMs: 8000, label: "inline-editor" }
      ).catch(() => false));
    }
    if (!opened) throw new Error("点击 passage 文本未进入原位编辑器");
    await session.evaluate(`(() => { const t = document.querySelector('textarea[aria-label="编辑题目文字"]'); t.select(); return true; })()`);
    await session.cdp.send("Input.insertText", { text: EDITED_MARKER });
    await session.pressKey("Enter", { code: "Enter", windowsVirtualKeyCode: 13 });
    // 保存有 450ms 防抖；等保存状态机回到「已保存」。
    await session.waitFor(
      `(() => { const s = document.querySelector('[data-testid="workspace-save-state"]'); return !s || /已保存/.test(s.innerText); })()`,
      { timeoutMs: 40000, label: "saved" }
    );
    const text = await session.evaluate(`document.querySelector('.v2-passage-pane .v2-text').innerText`);
    if (!String(text).includes(EDITED_MARKER)) throw new Error(`保存后题面里没有出现编辑标记：${text}`);
    await session.screenshot("04-edited-and-saved");
    return { text };
  });

  // ---- 6. 返回题库并重开，验证编辑持久化 ----
  await recorder.run("edit-survives-reopen", async () => {
    await session.clickSelector(".workspace-back-button");
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000, label: "back-to-library" });
    await session.clickSelector(`[data-item-id="${importedItemId}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-again" });
    const text = await session.waitFor(
      `(() => { const el = document.querySelector('.v2-passage-pane .v2-text'); return el && el.innerText.includes(${JSON.stringify(EDITED_MARKER)}) ? el.innerText : null; })()`,
      { timeoutMs: 40000, label: "marker-persisted" }
    );
    await session.screenshot("05-reopened-persisted");
    return { text };
  });

  // ---- 7. 学生预览：渲染 ----
  await recorder.run("student-preview-renders", async () => {
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-student-preview"]')`, { timeoutMs: 20000, label: "student-preview" });
    const info = await session.waitFor(
      `(() => {
        const root = document.querySelector('.workspace-student-preview');
        if (!root) return null;
        const canvas = root.querySelector('[data-testid="exam-canvas-v2-student"]');
        if (!canvas) return null;
        const revision = document.querySelector('[data-testid="workspace-preview-revision"]');
        const compileError = document.querySelector('[data-testid="workspace-preview-error"]');
        return {
          isStudentMode: canvas.classList.contains('is-student'),
          hasAuthorTextarea: root.querySelectorAll('textarea').length,
          hasAuthorTools: root.querySelectorAll('.v2-author-tools').length,
          revisionText: revision ? revision.innerText : null,
          compileError: compileError ? compileError.innerText.replace(/\\s+/g,' ').trim() : null,
          passageText: (root.querySelector('.v2-passage-pane') || {}).innerText || null,
          radioCount: root.querySelectorAll('input[type=radio]').length,
          checkboxCount: root.querySelectorAll('input[type=checkbox]').length,
          textInputs: root.querySelectorAll('input[type=text]').length,
          hotspotCount: root.querySelectorAll('.v2-canvas-hotspot').length,
          slotCount: root.querySelectorAll('[data-question-id]').length,
          // 预览必须把「能渲染」和「学生端能提交」分开说：答案键类型不匹配时题面照常画出，
          // 但学生端提交会被拒。这里记录预览是否如实报出了这些答案位。
          runtimeIssueCount: root.querySelectorAll('[data-testid="workspace-preview-runtime-issues"] li').length,
          runtimeIssueCodes: [...root.querySelectorAll('[data-testid="workspace-preview-runtime-issues"] li')]
            .map((li) => li.getAttribute('data-preview-runtime-code'))
            .filter(Boolean),
          // 学生答案必须从空开始：预览里不得预填任何作者答案。
          checkedAtStart: [...root.querySelectorAll('input[type=radio], input[type=checkbox]')].filter(i => i.checked).length,
          prefilledTextAtStart: [...root.querySelectorAll('input[type=text]')].map(i => i.value).filter(Boolean).length
        };
      })()`,
      { timeoutMs: 25000, label: "preview-dom" }
    );
    if (info.compileError) throw new Error(`预览显示编译错误：${info.compileError}`);
    // 本轮任务书第一节：普通界面不得出现 `v1/v2/v3`、批次基线、editVersion 这类内部版本信息。
    if (/\bv\d+\b/.test(String(info.revisionText ?? ""))) {
      throw new Error(`预览文案里出现了内部版本号：${JSON.stringify(info.revisionText)}`);
    }
    if (!info.isStudentMode) throw new Error("预览没有使用 student 模式渲染");
    if (info.hasAuthorTextarea > 0) throw new Error("预览里出现了作者态可编辑 textarea");
    if (info.hasAuthorTools > 0) throw new Error("预览里出现了作者态结构工具");
    if (!String(info.passageText ?? "").includes(EDITED_MARKER)) throw new Error(`预览没有反映刚保存的正文修改：${info.passageText}`);
    if (info.checkedAtStart > 0) throw new Error(`预览预填了 ${info.checkedAtStart} 个作答，学生答案必须从空开始（不能泄露答案）`);
    if (info.prefilledTextAtStart > 0) throw new Error(`预览预填了 ${info.prefilledTextAtStart} 个文本作答，学生答案必须从空开始`);
    // 可作答性：真实学生端把 text/radio/checkbox/select/hotspot 渲染成可交互控件，
    // 其余 interaction 只渲染成不可作答的 badge。若一个可作答控件都没有，这份题稿
    // 学生根本无法作答，后续「加载→作答→提交→计分」的验收无从谈起，必须直接失败。
    const answerableCount = info.radioCount + info.checkboxCount + info.textInputs + info.hotspotCount;
    if (answerableCount === 0) {
      throw new Error(`预览里没有任何可作答控件（slot=${info.slotCount}），这份题稿学生无法作答`);
    }
    await session.screenshot("06-student-preview");
    return info;
  });

  // ---- 8. 预览作答不污染作者答案 ----
  // 覆盖真实学生端的全部可作答交互（radio / checkbox / text / hotspot），
  // 并且**刻意给出与作者答案不同的作答**，否则「隔离」只是巧合。
  await recorder.run("student-preview-answering-isolated", async () => {
    const PREVIEW_SENTINEL = "E2E-PV-99";
    const snapshotExpr = (rootTestId) => `(() => {
        const root = document.querySelector('[data-testid="${rootTestId}"]');
        const choices = [...root.querySelectorAll('input[type=radio], input[type=checkbox]')]
          .filter(i => i.checked).map(i => i.name + '=' + i.value).sort();
        const texts = [...root.querySelectorAll('input[type=text]')]
          .map(i => i.name + '=' + i.value).filter(s => s.charAt(s.length - 1) !== '=').sort();
        return { choices, texts };
      })()`;

    // 作者态当前答案快照（预览里的作答不得改动它）。选项类与文本框都要采集。
    await session.clickSelector('[data-testid="workspace-mode-edit"]');
    await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-author"]')`, { timeoutMs: 20000, label: "back-to-author" });
    const authorSnapshotBefore = await session.evaluate(snapshotExpr("exam-canvas-v2-author"));

    // 切到预览，记录修订号，然后刻意给出与作者答案不同的作答。
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-student-preview"]')`, { timeoutMs: 20000, label: "preview-again" });
    // 修订号取自 `data-edit-version`（机器可读），**不是**预览那行文案：
    // 本轮把 `v1/v2/v3` 从普通界面移除了，读文案会退化成「两次读到同一段静态文字」，
    // 断言永远通过。同时顺带断言那行文案里确实没有版本号。
    const revisionInPreviewBefore = String(await session.evaluate(
      `(() => { const el = document.querySelector('[data-testid="exam-workspace"]'); return el ? el.getAttribute('data-edit-version') : ''; })()`
    ) ?? "");
    const revisionTextInPreview = String(await session.evaluate(
      `(() => { const el = document.querySelector('[data-testid="workspace-preview-revision"]'); return el ? el.innerText : ''; })()`
    ) ?? "");
    if (/\bv\d+\b/.test(revisionTextInPreview)) {
      throw new Error(`预览文案里出现了内部版本号：${JSON.stringify(revisionTextInPreview)}`);
    }

    // 在预览里挑一个「与作者答案不同」的可作答控件：优先选项类，其次文本框。
    // 关键：作者画布在 student 模式下已卸载，不能在预览 DOM 里再查作者画布
    //（那样永远查不到，会退化成随便点第一个选项，也就证明不了隔离），
    // 必须拿上面采集好的快照来比较。
    const previewTarget = await session.evaluate(
      `(() => {
        const root = document.querySelector('.workspace-student-preview');
        const authorChoices = ${JSON.stringify(authorSnapshotBefore.choices)};
        const authorTexts = ${JSON.stringify(authorSnapshotBefore.texts)};
        const sentinel = ${JSON.stringify(PREVIEW_SENTINEL)};
        const lookup = (arr, slotId) => {
          const hit = arr.find(s => s.slice(0, slotId.length + 1) === slotId + '=');
          return hit === undefined ? null : hit.slice(slotId.length + 1);
        };
        const choices = [...root.querySelectorAll('input[type=radio], input[type=checkbox]')];
        const texts = [...root.querySelectorAll('input[type=text]')];
        const distinctChoice = choices.find(c => { const av = lookup(authorChoices, c.name); return av !== null && av !== c.value; });
        const fallbackChoice = choices.find(c => !c.checked);
        const textTarget = texts.find(t => lookup(authorTexts, t.name) !== sentinel);
        let el = null; let kind = null; let value = null; let differs = false;
        if (distinctChoice) { el = distinctChoice; kind = 'choice'; value = distinctChoice.value; differs = true; }
        else if (textTarget) { el = textTarget; kind = 'text'; value = sentinel; differs = lookup(authorTexts, textTarget.name) !== sentinel; }
        else if (fallbackChoice) { el = fallbackChoice; kind = 'choice'; value = fallbackChoice.value; differs = false; }
        if (!el) return { kind: null, reason: 'no-answerable-control', choices: choices.length, texts: texts.length };
        el.setAttribute('data-e2e-preview-target', '1');
        return { kind, value, slotId: el.name, deliberatelyDiffersFromAuthor: differs };
      })()`
    );
    if (!previewTarget?.kind) throw new Error(`预览里没有可作答的控件：${JSON.stringify(previewTarget)}`);

    // 真实输入：选项类走真实鼠标点击，文本框走真实键盘输入（CDP Input.insertText）。
    let inputMethod = previewTarget.kind === "text" ? "cdp-Input.insertText" : "cdp-mouse-click";
    if (previewTarget.kind === "text") {
      await session.typeInto('[data-e2e-preview-target="1"]', String(previewTarget.value));
    } else {
      await session.clickSelector('[data-e2e-preview-target="1"]');
    }
    await sleep(1200);

    let previewSnapshot = await session.evaluate(snapshotExpr("workspace-student-preview"));
    const previewAnswered = previewSnapshot.choices.length + previewSnapshot.texts.length > 0;
    // 文本框：若真实键盘输入没有被受控组件登记，退回到原生 setter + input 事件
    //（仍是真实 DOM 事件，但不是真实键盘），并如实记录所用方式。
    if (!previewAnswered && previewTarget.kind === "text") {
      inputMethod = "native-setter+input-event-fallback";
      await session.evaluate(
        `(() => {
          const el = document.querySelector('[data-e2e-preview-target="1"]');
          if (!el) return false;
          const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
          setter.call(el, ${JSON.stringify(String(previewTarget.value))});
          el.dispatchEvent(new Event('input', { bubbles: true }));
          return true;
        })()`
      );
      await sleep(800);
      previewSnapshot = await session.evaluate(snapshotExpr("workspace-student-preview"));
    }

    const revisionInPreviewAfter = String(await session.evaluate(
      `(() => { const el = document.querySelector('[data-testid="exam-workspace"]'); return el ? el.getAttribute('data-edit-version') : ''; })()`
    ) ?? "");
    await session.screenshot("07-preview-answered");

    // 回到编辑：作者答案必须原样。
    await session.clickSelector('[data-testid="workspace-mode-edit"]');
    await session.waitFor(`!!document.querySelector('[data-testid="exam-canvas-v2-author"]')`, { timeoutMs: 20000, label: "author-after-preview" });
    await sleep(1500);
    const authorSnapshotAfter = await session.evaluate(snapshotExpr("exam-canvas-v2-author"));
    const saveState = await session.evaluate(
      `(() => { const el = document.querySelector('[data-testid="workspace-save-state"]'); return el ? el.innerText : null; })()`
    );
    const pendingNotice = await session.evaluate(
      `(() => { const el = document.querySelector('[data-testid="workspace-save-notice"]'); return el ? el.innerText : null; })()`
    );

    // 预览里必须真的记录下这次作答，否则「隔离」是空断言。
    if (!previewAnswered) throw new Error(`预览没有记录下这次作答：${JSON.stringify({ previewTarget, previewSnapshot })}`);
    if (JSON.stringify(authorSnapshotBefore) !== JSON.stringify(authorSnapshotAfter)) {
      throw new Error(`预览作答污染了作者答案：before=${JSON.stringify(authorSnapshotBefore)} after=${JSON.stringify(authorSnapshotAfter)}`);
    }
    if (revisionInPreviewBefore !== revisionInPreviewAfter) {
      throw new Error(`预览作答产生了新的编辑修订：before=${JSON.stringify(revisionInPreviewBefore)} after=${JSON.stringify(revisionInPreviewAfter)}`);
    }
    return {
      previewAnswer: previewTarget,
      inputMethod,
      previewSnapshot,
      authorSnapshotBefore,
      authorSnapshotAfter,
      revisionInPreviewBefore,
      revisionInPreviewAfter,
      saveStateAfterPreview: saveState,
      saveNoticeAfterPreview: pendingNotice,
      previewWroteNothing: true
    };
  });

  // ---- 9. 预览后仍可编辑并保存，且持久化 ----
  await recorder.run("edit-after-preview-survives-reopen", async () => {
    await session.clickSelector(".v2-passage-pane .v2-text");
    const opened = await session.waitFor(
      `!!document.querySelector('textarea[aria-label="编辑题目文字"]')`,
      { timeoutMs: 10000, label: "inline-editor-again" }
    ).catch(() => false);
    if (!opened) throw new Error("预览返回后无法再次进入原位编辑");
    const second = "E2E EDIT CHECK 43";
    await session.evaluate(`(() => { const t = document.querySelector('textarea[aria-label="编辑题目文字"]'); t.select(); return true; })()`);
    await session.cdp.send("Input.insertText", { text: second });
    await session.pressKey("Enter", { code: "Enter", windowsVirtualKeyCode: 13 });
    await session.waitFor(
      `(() => { const s = document.querySelector('[data-testid="workspace-save-state"]'); return !s || /已保存/.test(s.innerText); })()`,
      { timeoutMs: 40000, label: "saved-again" }
    );
    await session.clickSelector(".workspace-back-button");
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000, label: "library-again" });
    await session.clickSelector(`[data-item-id="${importedItemId}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-third" });
    const text = await session.waitFor(
      `(() => { const el = document.querySelector('.v2-passage-pane .v2-text'); return el && el.innerText.includes(${JSON.stringify(second)}) ? el.innerText : null; })()`,
      { timeoutMs: 40000, label: "second-marker-persisted" }
    );
    await session.screenshot("08-second-edit-persisted");
    return { text };
  });

  // ---- 10. 从工作区发布到 NAS 目录 ----
  // 专项范围（编辑/预览）**根本不注册这一步**：专项绿不许被当成发布链绿。
  // 注意不能用「提前 return」来跳过 —— recorder 会把提前返回记成 passed，
  // 那就是「没跑也写通过」，正是本轮要消灭的那类假绿。
  if (scope === "full-chain") {
  await recorder.run("publish-via-workspace-button", async () => {
    const nasDestination = path.join(runDir, "nas-library");
    await session.evaluate(
      `(() => {
        const raw = window.localStorage.getItem("ielts-author-studio.app-settings.v1");
        const settings = raw ? JSON.parse(raw) : {};
        settings.nasDestination = ${JSON.stringify(nasDestination)};
        settings.cloudEnabled = false;
        window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify(settings));
        return true;
      })()`
    );
    await session.clickSelector('[data-testid="workspace-publish"]');
    const outcome = await session.waitFor(
      `(() => {
        const notices = [...document.querySelectorAll('.workspace-notice')].map(n => n.innerText.replace(/\\s+/g,' ').trim());
        const joined = notices.join(' || ');
        if (/发布完成|发布失败|问题|不能发布|拦/.test(joined)) return { notices };
        return null;
      })()`,
      { timeoutMs: 120000, label: "publish-outcome" }
    );
    await session.screenshot("09-publish-outcome");
    const joined = (outcome.notices ?? []).join(" || ");
    const blocked = /问题|不能发布|拦|阻断/.test(joined) && !/发布完成/.test(joined);
    // 结构化门禁详情：走真实 IPC 读 get_publish_preflight。
    // 报告里要留下具体 blocker（code / targetId / internal），而不是一句提示语 ——
    // 否则「为什么发不出去」无法交接，也无法判断是题稿问题还是门禁问题。
    let preflight = null;
    try {
      const r = await session.invoke("get_publish_preflight", { jobId: importedItemId });
      if (r?.ok && r.value) {
        preflight = {
          passed: r.value.passed,
          editVersion: r.value.editVersion ?? null,
          blockers: (r.value.blockers ?? []).map((b) => ({
            code: b.code,
            targetId: b.targetId ?? null,
            internal: b.internal ?? null
          })),
          warnings: (r.value.warnings ?? []).map((w) => w.code ?? w.message ?? null)
        };
      } else {
        preflight = { error: r?.error ?? "no-invoke" };
      }
    } catch (error) {
      preflight = { error: String(error) };
    }
    // 门禁只说 RUNTIME_COMPILER_FAILED，不说为什么。同一份产物里存着编译器探针的明细，
    // 读出来才能判断「预览该不该已经报出这个问题」，也是交接给识别侧的关键证据。
    let runtimeProbeDetails = null;
    try {
      const shadowPath = path.join(runDir, "appdata", "data", "jobs", importedItemId, "authoring-ir-v2.shadow.json");
      if (fs.existsSync(shadowPath)) {
        const shadow = JSON.parse(fs.readFileSync(shadowPath, "utf8"));
        const probe = shadow?.quality?.compilerProbes?.v2Runtime ?? null;
        runtimeProbeDetails = probe
          ? { status: probe.status, issueCodes: probe.issueCodes ?? [], details: (probe.details ?? []).slice(0, 12) }
          : null;
      }
    } catch (error) {
      runtimeProbeDetails = { error: String(error) };
    }
    return {
      outcome: blocked ? "blocked_by_quality_gate" : (outcome.notices ?? []),
      notices: outcome.notices ?? [],
      nasDestination,
      manifestExists: fs.existsSync(path.join(nasDestination, "manifest.js")),
      preflight,
      runtimeProbeDetails
    };
  });
  }

  // ---- 11. 识别建议面板（对真实后端命令的只读路径） ----
  await recorder.run("recognition-panel-reads-real-backend", async () => {
    await session.clickSelector('[data-testid="workspace-recognition-toggle"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-recognition"]')`, { timeoutMs: 20000, label: "recognition-panel" });
    const state = await session.waitFor(
      `(() => {
        const root = document.querySelector('[data-testid="workspace-recognition"]');
        if (!root) return null;
        const cloud = root.querySelector('[data-testid="workspace-recognition-cloud"]');
        if (!cloud) return null;
        return {
          cloudStatus: cloud.innerText.replace(/\\s+/g,' ').trim(),
          loadError: (() => { const e = root.querySelector('[data-testid="workspace-recognition-error"]'); return e ? e.innerText.replace(/\\s+/g,' ').trim() : null; })(),
          summary: [...root.querySelectorAll('[data-testid="workspace-recognition-summary"] li')].map(li => li.innerText.replace(/\\s+/g,' ').trim()),
          cardCount: root.querySelectorAll('[data-testid="workspace-recognition-card"]').length,
          empty: (() => { const e = root.querySelector('[data-testid="workspace-recognition-empty"]'); return e ? e.innerText.replace(/\\s+/g,' ').trim() : null; })()
        };
      })()`,
      { timeoutMs: 25000, label: "recognition-state" }
    );
    // 面板必须给出**可读**状态，绝不能把缺失字段渲染成 undefined。
    if (/undefined/.test(JSON.stringify(state))) throw new Error(`识别面板出现了 undefined：${JSON.stringify(state)}`);
    await session.screenshot("10-recognition-panel");
    return state;
  });

  // ---- 12. 一致性：预览不得对「学生端会拒绝的题稿」显示假完成 ----
  // 断言必须精确到「门禁给出的具体原因码」，而不是笼统的 RUNTIME_COMPILER_FAILED：
  // 该码有多个来源（答案键类型不匹配、答案键缺槽位……），只有前者是预览侧已经能独立判定的。
  // 否则会把「预览没覆盖的原因」误判成「预览在骗人」。
  await recorder.run("preview-and-gate-agree", async () => {
    const previewStep = recorder.steps.find((s) => s.name === "student-preview-renders");
    const publishStep = recorder.steps.find((s) => s.name === "publish-via-workspace-button");
    const previewRuntimeIssueCount = previewStep?.detail?.runtimeIssueCount ?? null;
    const probe = publishStep?.detail?.runtimeProbeDetails ?? null;
    const probeCodes = probe?.issueCodes ?? [];
    const gateRuntimeFailure = Boolean(
      (publishStep?.detail?.preflight?.blockers ?? []).some(
        (b) => b.code === "QUALITY_HARD_FAILURE" && b.internal === "RUNTIME_COMPILER_FAILED"
      )
    );
    // 预览侧目前能独立判定的运行时原因码（见 src/services/readingRuntimeV2.ts）。
    const PREVIEW_COVERED = ["RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION", "RUNTIME_TEXT_SLOT_ANSWER_NOT_TEXT"];
    const covered = probeCodes.filter((code) => PREVIEW_COVERED.includes(code));
    const unmapped = probeCodes.filter((code) => !PREVIEW_COVERED.includes(code));
    const detail = {
      previewRuntimeIssueCount,
      gateRuntimeFailure,
      // 专项范围没有发布步骤 ⇒ 没有门禁结论可比。如实记下来，
      // 免得把「没得比」当成「比过且一致」。
      gateVerdictAvailable: Boolean(publishStep),
      runtimeProbeStatus: probe?.status ?? null,
      runtimeProbeCodes: probeCodes,
      previewCoveredCodes: covered,
      // 门禁拦下、但预览侧没有**专门的**运行时呈现区的原因码。这是交接用的覆盖映射记录，
      // 不等于「预览在骗人」：这些原因仍通过别的前端呈现面暴露（问题列表的 ANSWER_MISSING、
      // 预览的 answeredSlots 计数、以及预览里的门禁阻断数）。真正算缺陷的是下面的
      // previewFalseCompletion。
      previewUnmappedProbeCodes: unmapped,
      hasUnmappedProbeCodes: unmapped.length > 0,
      // 只有当「门禁原因确实属于预览已覆盖的那类」而预览却没报出来时，才算假完成。
      previewFalseCompletion: covered.length > 0 && !(previewRuntimeIssueCount > 0)
    };
    if (detail.previewFalseCompletion) {
      throw new Error(
        `预览与门禁不一致：门禁的编译器探针报出 ${covered.join(",")}，但预览没有报出任何答案键类型问题（预览显示假完成）`
      );
    }
    return detail;
  });

  // 判定不在 try 里算：此时 report.steps 还是空数组，空数组的 filter 会把失败写成通过。
  // 真正的判定在 finally 里、recorder.steps 赋给 report.steps 之后执行。
  report.pendingVerdict = { failed: report.steps.filter((s) => s.status === "failed").map((s) => s.name) };
}

/**
 * 发布产物的落盘事实。
 *
 * 「发布步骤 passed」只说明 UI 没报错；任务书要求 manifest / 题目 JS / 资源三者都核对，
 * 所以这里直接看发布目录，而不是相信步骤状态。
 */
function collectPublicationFacts(nasDestination) {
  const facts = { nasDestination: nasDestination ?? null, scriptFiles: [], resourceManifestExists: false, resourceManifests: [] };
  if (!nasDestination || !fs.existsSync(nasDestination)) return facts;
  let entries = [];
  try {
    entries = fs.readdirSync(nasDestination);
  } catch {
    return facts;
  }
  facts.scriptFiles = entries.filter((name) => /^v2-p.*\.js$/i.test(name));
  const resourcesDir = path.join(nasDestination, "resources");
  if (fs.existsSync(resourcesDir)) {
    for (const examId of fs.readdirSync(resourcesDir)) {
      const manifest = path.join(resourcesDir, examId, "asset-manifest.json");
      if (fs.existsSync(manifest)) facts.resourceManifests.push(manifest);
    }
  }
  facts.resourceManifestExists = facts.resourceManifests.length > 0;
  return facts;
}

try {
  await main();
} catch (error) {
  const cannotRun = error instanceof CannotRunError;
  report.cannotRun = cannotRun;
  report.fatal = { name: error.name, message: String(error.message), appOutput: error.appOutput ?? null };
  console.error(`[chain] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${error.message}`);
} finally {
  if (session) {
    if (!keep) await session.screenshot("final").catch(() => {});
    const closed = await session.close({ keep });
    report.appOutput = closed.appOutput?.slice(-6000) ?? null;
    report.appProcessExitCode = closed.exitCode;
    report.screenshotErrors = session.screenshotErrors;
  }
  report.finishedAt = new Date().toISOString();
  // 判定**无条件**执行：CANNOT-RUN 时 recorder 还是 null，若把判定关在 `if (recorder)` 里，
  // 这种运行会退回初始的 `verdict="failed"`、退出码 undefined —— 又是一个「判定没跑却看起来跑过」。
  if (recorder) report.steps = recorder.steps;
  const publishStep = report.steps.find((s) => s.name === PUBLISH_STEP);
  const publicationFacts = collectPublicationFacts(publishStep?.detail?.nasDestination);
  report.publication = {
    ...publicationFacts,
    manifestExists: publishStep?.detail?.manifestExists ?? null,
    preflightPassed: publishStep?.detail?.preflight?.passed ?? null
  };
  // 只有完整链才把「产物完整性」当判定条件：专项范围根本不该有发布步骤。
  const publicationFailures = scope === "full-chain" && !report.cannotRun
    ? evaluatePublication({ publishDetail: publishStep?.detail, publicationFacts })
    : [];
  const verdict = computeChainVerdict({
    steps: report.steps,
    scope,
    expectBlocked,
    cannotRun: Boolean(report.cannotRun),
    publicationFailures
  });
  report.verdict = verdict.verdict;
  report.exitCode = verdict.exitCode;
  report.verdictReason = verdict.reason;
  report.summary = {
    passed: verdict.passed,
    failed: verdict.failed,
    blocked: verdict.blocked,
    missing: verdict.missing,
    publicationFailures: verdict.publicationFailures
  };
  delete report.pendingVerdict;
  const file = writeReport(runDir, report);
  console.log(`[chain] verdict=${report.verdict} exit=${report.exitCode} reason=${report.verdictReason}`);
  console.log(`[chain] report=${file}`);
  console.log(`[chain] steps: ${report.steps.map((s) => `${s.name}:${s.status}`).join(" | ")}`);
  process.exit(report.exitCode ?? 1);
}
