#!/usr/bin/env node
/**
 * 编辑工作区的**布局**验收（WebView2 CDP 通道）。
 *
 * 为什么单独一个脚本：既有脚本只断言「元素存在」「没有横向滚动条」，那两条对本次缺陷
 * **全部为真**——识别建议面板确实存在，页面也确实没有横向滚动条，可工作区已经被劈成
 * 左右两半：建议面板占左半并拉满整列高度（一大片空白），题稿被挤进右半，题号被挤到
 * 逐字符换行。存在性断言看不见这类错误，所以这里断言的是**几何与网格归属**。
 *
 * 复现对象：用户截图（1080×617 视口，`demanding-reading-passage-3.pdf`，q27–q40）。
 *
 * 断言（任一不成立即 FAIL，逐条写进报告）：
 *   L1 body-not-pushed-right      题稿左边界贴合工作区左边界（没有被推到一侧）
 *   L2 body-full-width            题稿占满工作区宽度
 *   L3 recognition-spans-page     建议面板横跨工作区（不是占一列）
 *   L4 no-side-by-side            建议面板与题稿**上下**排列，不是左右并排
 *   L5 recognition-not-tall-blank 建议面板高度受内容约束，不占大块空白
 *   L6 question-number-intact     题号完整：没有被压缩到逐字符换行
 *   L7 panes-within-viewport      原文栏与题目栏都在视口内，且各有可用宽度
 *   L8 panes-independent-scroll   两栏可独立滚动
 *   L9 edit-save-reopen           编辑保存 → 返回题库 → 重新打开，值读得回来
 *   L10 header-spans-page         顶部工具栏与模式栏横跨工作区
 *
 * 另外记录（不作为硬断言，但必须出现在报告里）：
 *   - 控制台异常 / console.error（区分「运行时异常」与「布局错位」）
 *   - 工作区网格的 computed `grid-template-columns` 与各子项的 grid-row/column
 *   - 题面重复诊断：同一段文字在原文栏与题目栏是否都出现（对照 IR 判定哪一层产生）
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-workspace-layout.mjs [--pdf <path>] [--width N] [--height N]
 *        [--keep] [--no-diagnostic-args] [--tolerate-concurrent-edits]
 * 退出码：0 通过 / 1 失败 / 3 环境不满足
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
  fixtureIdx >= 0 ? process.argv[fixtureIdx + 1] : path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf")
);
// 截图对应的视口。`--width/--height` 用来在较窄窗口下复测（任务书第 5 条）。
const widthIdx = process.argv.indexOf("--width");
const heightIdx = process.argv.indexOf("--height");
const viewportWidth = widthIdx >= 0 ? Number(process.argv[widthIdx + 1]) || 1080 : 1080;
const viewportHeight = heightIdx >= 0 ? Number(process.argv[heightIdx + 1]) || 617 : 617;
// 本机必需：不加这两个参数 WebView2 的 renderer 会在中途崩（`CDP 连接已关闭`）。
// 默认**打开**——旧脚本把默认值写成空串、注释却写「环境必需」，无参运行必 CANNOT-RUN。
const extraArgs = process.argv.includes("--no-diagnostic-args") ? "" : "--no-sandbox --disable-gpu";
const runDir = path.join(
  repoRoot,
  "artifacts",
  "e2e-cdp",
  `run-workspace-layout-${new Date().toISOString().replace(/[:.]/g, "-")}`
);

const report = {
  task: "workspace-layout-geometry",
  scope: "真实界面几何与网格归属验收（不是元素存在性检查）",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  diagnosticRun: Boolean(extraArgs),
  runProfile: extraArgs ? "cdp-diagnostic" : "cdp-default",
  viewport: { width: viewportWidth, height: viewportHeight },
  probes: {},
  snapshots: [],
  assertions: [],
  consoleErrors: [],
  pageExceptions: [],
  duplicateText: null,
};

/** 被测元素：key → 选择器。选择器与组件里真实使用的类名一一对应。 */
const PROBES = [
  ["page", '[data-testid="exam-workspace"]'],
  ["header", ".workspace-header"],
  ["subHeader", ".workspace-sub-header"],
  ["recognition", ".workspace-recognition"],
  ["issues", ".workspace-issues"],
  ["paneTabs", ".workspace-pane-tabs"],
  ["body", ".workspace-body"],
  // 用后代选择器而不是 `>`：学生预览模式下两栏挂在
  // `.workspace-student-preview > .exam-canvas-v2` 下，同一时刻页面上只有一套题面。
  ["passagePane", ".workspace-body .v2-passage-pane"],
  ["questionPane", ".workspace-body .v2-question-pane"],
];

/**
 * 在页面里采集所有被测元素的几何 + 网格归属。
 *
 * 只读：不改样式、不点任何东西。`getBoundingClientRect` 取的是**实际**布局盒，
 * 这正是「被挤到一侧」能被看见的地方——元素存在性断言看不见它。
 */
const COLLECT_FN = `(() => {
  const probes = ${JSON.stringify(PROBES)};
  const out = {};
  for (const [key, sel] of probes) {
    const el = document.querySelector(sel);
    if (!el) { out[key] = { present: false, selector: sel }; continue; }
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    out[key] = {
      present: true,
      selector: sel,
      rect: { x: +r.x.toFixed(1), y: +r.y.toFixed(1), w: +r.width.toFixed(1), h: +r.height.toFixed(1),
              right: +(r.x + r.width).toFixed(1), bottom: +(r.y + r.height).toFixed(1) },
      display: cs.display,
      position: cs.position,
      gridRowStart: cs.gridRowStart, gridRowEnd: cs.gridRowEnd,
      gridColumnStart: cs.gridColumnStart, gridColumnEnd: cs.gridColumnEnd,
      gridTemplateColumns: cs.gridTemplateColumns,
      gridTemplateRows: cs.gridTemplateRows,
      overflowX: cs.overflowX, overflowY: cs.overflowY,
      visibility: cs.visibility, opacity: cs.opacity,
    };
  }
  // 题号完整性：逐字符换行时元素会变得又窄又高。
  // **必须同时查 .v2-slot-label**：行内填空（summary/sentence/note/form completion）的题号
  // 渲染在题干内部，用的是 .v2-slot-label 而不是 .v2-slot-number。只查后者时，
  // 用户截图里竖排的 27/28/29 一个都量不到（首版实测只查到 9 个，且全部判为「未换行」）。
  out.__questionNumbers = [...document.querySelectorAll('.v2-slot-number, .v2-slot-label')].slice(0, 120).map((el) => {
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    const lineHeight = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.2 || 16;
    return {
      cls: el.className,
      text: (el.textContent || '').trim(),
      w: +r.width.toFixed(1), h: +r.height.toFixed(1),
      fontSize: cs.fontSize, lineHeight: +lineHeight.toFixed(1),
      whiteSpace: cs.whiteSpace, flexGrow: cs.flexGrow, flexShrink: cs.flexShrink,
      // scrollWidth > clientWidth 说明内容被压过；h 超过 1.6 倍行高说明换了行。
      scrollWidth: el.scrollWidth, clientWidth: el.clientWidth,
      wrapped: r.height > lineHeight * 1.6 + 0.5,
    };
  });
  // 窄屏（≤980px）下只显示一栏是**设计如此**（切换栏接管），断言要据此区分，
  // 否则会把「按设计隐藏原文栏」误判成布局错位。
  const tabs = document.querySelector('.workspace-pane-tabs');
  out.__narrowTabs = { present: Boolean(tabs), visible: Boolean(tabs) && getComputedStyle(tabs).display !== 'none' };
  out.__viewport = { innerWidth: window.innerWidth, innerHeight: window.innerHeight,
                     docScrollWidth: document.documentElement.scrollWidth,
                     docClientWidth: document.documentElement.clientWidth };
  return out;
})()`;

/**
 * 题面重复诊断：统计「题目栏里出现的原文句子」与「原文栏里的同一句子」。
 *
 * 做法是**取证**而不是判定：取题目栏若干段文本，看它们在原文栏是否逐字存在。
 * 判定哪一层产生要对照 IR（下面 `readIrDuplication`），这里只负责指出「确实重了、重在哪几句」。
 */
const DUPLICATE_FN = `(() => {
  const norm = (s) => (s || '').replace(/\\s+/g, ' ').trim();
  const passage = document.querySelector('.workspace-body .v2-passage-pane');
  const questions = document.querySelector('.workspace-body .v2-question-pane');
  if (!passage || !questions) return null;
  const passageText = norm(passage.innerText);
  // 题目栏里的段落：按长度筛掉标题/指令这类短行，只看真正的正文句子。
  const sentences = norm(questions.innerText)
    .split(/(?<=[.?!])\\s+/)
    .map(norm)
    .filter((s) => s.length >= 60);
  const echoed = sentences.filter((s) => passageText.includes(s));
  return {
    passageChars: passageText.length,
    questionSentenceCount: sentences.length,
    echoedCount: echoed.length,
    echoedSamples: echoed.slice(0, 3).map((s) => s.slice(0, 120)),
  };
})()`;

/** 从权威稿（真实 IR）里查同一段文字出现在哪些节点——用来判定重复产生在前端还是源数据。 */
function readIrDuplication(ds) {
  if (!ds) return null;
  const norm = (s) => (s || "").replace(/\s+/g, " ").trim();
  // ContentNodeV2 是**树**（`paragraph.children[].text`、`table.rows[].cells[].children`、
  // `bullet_list.items[].children`…），不是自带 `.text` 的扁平数组。
  // 第一版按 `nodes.map(n => n.text)` 平铺取值 → 取到空串，且 `(nodes ?? []).map`
  // 在非数组（`passage` 是对象、不是数组）上直接抛 `map is not a function`。
  const textOf = (nodes) => {
    const out = [];
    const seen = new Set();
    const walk = (n) => {
      if (!n || typeof n !== "object" || seen.has(n)) return;
      seen.add(n);
      if (Array.isArray(n)) { for (const child of n) walk(child); return; }
      if (typeof n.text === "string") out.push(n.text);
      for (const key of ["children", "items", "rows", "cells", "steps", "caption", "options"]) {
        if (n[key]) walk(n[key]);
      }
    };
    walk(nodes);
    return norm(out.join(" "));
  };
  // 一层一层的文本清单：任务书第 4 条要求逐层对照 instructions / stimulus / response prompt。
  const layers = [];
  const passageText = textOf(ds.passage?.content);
  layers.push({ where: "passage.content", role: "原文", chars: passageText.length, text: passageText });
  for (const task of ds.taskGroups ?? []) {
    const instructions = textOf(task.instructions);
    layers.push({ where: `taskGroup ${task.taskId} instructions`, role: "指令", chars: instructions.length, text: instructions });
    const taskStimulus = textOf(task.stimulus);
    if (taskStimulus) layers.push({ where: `taskGroup ${task.taskId} stimulus`, role: "题组题干", chars: taskStimulus.length, text: taskStimulus });
    for (const rg of task.responseGroups ?? []) {
      const prompt = textOf(rg.prompt);
      if (prompt) layers.push({ where: `taskGroup ${task.taskId} / responseGroup ${rg.responseGroupId ?? "?"} prompt`, role: "作答提示", chars: prompt.length, text: prompt });
    }
  }
  // 「同一段文字同时出现在两个不同的层」才是重复。判据取较长的公共前缀做包含测试，
  // 避免短指令（"Choose the correct letter"）在原文里偶然命中就误报。
  const MIN = 80;
  const contains = (haystack, needle) => needle.length >= MIN && haystack.includes(needle.slice(0, MIN));
  const duplicated = [];
  const passageLayer = layers.find((l) => l.role === "原文");
  for (const layer of layers) {
    if (layer.role === "原文" || !layer.text) continue;
    if (passageLayer && contains(passageLayer.text, layer.text)) {
      duplicated.push({ layer: layer.where, role: layer.role, chars: layer.chars, alsoIn: "passage.content", sample: layer.text.slice(0, 160) });
    } else if (passageLayer && contains(layer.text, passageLayer.text)) {
      duplicated.push({ layer: layer.where, role: layer.role, chars: layer.chars, alsoIn: "passage.content(被包含)", sample: layer.text.slice(0, 160) });
    }
  }
  return {
    passageChars: passageText.length,
    layerInventory: layers.map(({ where, role, chars }) => ({ where, role, chars })),
    // 非空即说明**源数据层面**原文被搬进了题面层 → 属后端复现材料，不是前端重复渲染。
    duplicatedLayers: duplicated,
    passageSample: passageText.slice(0, 160),
  };
}

function recordAssertion(id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

/** 取某个探针的矩形（缺失返回 null）。 */
const rectOf = (snap, key) => snap?.probes?.[key]?.rect ?? null;

function evaluateLayout(snap) {
  const page = rectOf(snap, "page");
  const body = rectOf(snap, "body");
  const rec = snap.probes?.recognition?.present ? rectOf(snap, "recognition") : null;
  const passage = rectOf(snap, "passagePane");
  const question = rectOf(snap, "questionPane");
  const tag = snap.label;
  const results = [];

  if (!page || !body) {
    results.push(["L1 body-not-pushed-right", false, `${tag}: 工作区或题稿缺失`]);
    return results;
  }

  // L1 / L2：题稿必须贴合工作区左边界并占满宽度。
  const bodyLeftGap = body.x - page.x;
  results.push(["L1 body-not-pushed-right", bodyLeftGap <= 4,
    `${tag}: 题稿左边界距工作区左边界 ${bodyLeftGap.toFixed(1)}px（要求 ≤4）`]);
  const bodyWidthGap = page.w - body.w;
  results.push(["L2 body-full-width", bodyWidthGap <= 4,
    `${tag}: 题稿宽 ${body.w.toFixed(1)} / 工作区宽 ${page.w.toFixed(1)}（差 ${bodyWidthGap.toFixed(1)}px，要求 ≤4）`]);

  // L3 / L4 / L5：建议面板必须横跨工作区、与题稿上下排列、且不占大块空白。
  if (rec) {
    results.push(["L3 recognition-spans-page", rec.w >= page.w - 4,
      `${tag}: 建议面板宽 ${rec.w.toFixed(1)} / 工作区宽 ${page.w.toFixed(1)}`]);
    const overlapX = Math.min(rec.right, body.right) - Math.max(rec.x, body.x);
    results.push(["L4 no-side-by-side", overlapX > 0,
      `${tag}: 建议面板与题稿水平重叠 ${overlapX.toFixed(1)}px（要求 >0，即上下排列而非左右并排）`]);
    results.push(["L5 recognition-not-tall-blank", rec.h <= Math.max(220, page.h * 0.5),
      `${tag}: 建议面板高 ${rec.h.toFixed(1)}px（上限 ${Math.max(220, page.h * 0.5).toFixed(1)}px）`]);
  }

  // L6：题号不得被压到逐字符换行。
  const nums = snap.probes?.__questionNumbers ?? [];
  const wrapped = nums.filter((n) => n.wrapped);
  results.push(["L6 question-number-intact", nums.length > 0 && wrapped.length === 0,
    `${tag}: 题号 ${nums.length} 个，逐字符换行的 ${wrapped.length} 个` +
    (wrapped.length ? `（例：${wrapped.slice(0, 3).map((n) => `「${n.text}」${n.w}×${n.h} ${n.cls}`).join("、")}）` : "")]);

  // L7 / L8：两栏都在视口内且可独立滚动。
  if (passage && question) {
    // 窄屏（≤980px）由 `.workspace-pane-tabs` 接管，**按设计只显示一栏**。
    // 断言要区分「设计如此」与「被挤没了」，否则会把前者误判成布局错位。
    const narrow = Boolean(snap.probes?.__narrowTabs?.visible);
    const pageRight = page.right + 1;
    if (narrow) {
      const visible = [["原文栏", passage], ["题目栏", question]].filter(([, r]) => r.w > 0);
      const hidden = [["原文栏", passage], ["题目栏", question]].filter(([, r]) => r.w <= 0);
      const hiddenByDesign = hidden.every(([name]) =>
        snap.probes?.[name === "原文栏" ? "passagePane" : "questionPane"]?.display === "none");
      const ok = visible.length === 1 && hiddenByDesign
        && visible[0][1].right <= pageRight && visible[0][1].w >= 200;
      results.push(["L7 panes-within-viewport", ok,
        `${tag}: 窄屏单栏模式，可见 ${visible.map(([n, r]) => `${n} ${r.w.toFixed(1)}px(right ${r.right.toFixed(1)})`).join("、") || "无"}` +
        `，另一栏 display:none=${hiddenByDesign}，工作区 right ${page.right.toFixed(1)}`]);
    } else {
      const inView = passage.right <= pageRight && question.right <= pageRight
        && passage.w >= 120 && question.w >= 120;
      results.push(["L7 panes-within-viewport", inView,
        `${tag}: 原文栏 ${passage.w.toFixed(1)}px（right ${passage.right.toFixed(1)}）、题目栏 ${question.w.toFixed(1)}px（right ${question.right.toFixed(1)}），工作区 right ${page.right.toFixed(1)}`]);
    }
    const oy = snap.probes?.passagePane?.overflowY;
    const qy = snap.probes?.questionPane?.overflowY;
    const scrollable = ["auto", "scroll"].includes(oy) && ["auto", "scroll"].includes(qy);
    results.push(["L8 panes-independent-scroll", scrollable,
      `${tag}: 原文栏 overflow-y=${oy}，题目栏 overflow-y=${qy}`]);
  }

  // L10：顶部工具栏与模式栏必须横跨工作区（任务书第 2 条）。
  // 它们和题稿一样是页面骨架的直接子项，上一版只断言了题稿与建议面板，漏了这两条。
  const header = snap.probes?.header?.present ? rectOf(snap, "header") : null;
  const subHeader = snap.probes?.subHeader?.present ? rectOf(snap, "subHeader") : null;
  if (header && subHeader) {
    results.push(["L10 header-spans-page", header.w >= page.w - 4 && subHeader.w >= page.w - 4,
      `${tag}: 顶部栏宽 ${header.w.toFixed(1)}、模式栏宽 ${subHeader.w.toFixed(1)} / 工作区宽 ${page.w.toFixed(1)}`]);
  }

  return results;
}

async function main() {
  const tolerateConcurrentEdits = process.argv.includes("--tolerate-concurrent-edits");
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits });
  report.identity = {
    exePath,
    exeSha256: sha256File(exePath),
    fixturePath,
    fixtureSha256: sha256File(fixturePath),
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: buildFreshReport(fresh),
  };
  if (!fs.existsSync(fixturePath)) throw new CannotRunError(`夹具不存在：${fixturePath}`);

  // PDF 走「选择文件夹」而不是「选择文件」：harness 把 `PDF2TEST_AUTOMATION_PDF_DIR`
  // 指向 `<runDir>/pdfs`，免对话框的 hook 只列该目录下的 PDF。
  // 第一版脚本点的是 `import-pick-files`（DOCX 通道，读 `PDF2TEST_AUTOMATION_SOURCE_FILES`），
  // 那条路径对 PDF 不生效 → 原生对话框弹出、`import-picked-files` 永远空 → 20s 超时。
  const isPdf = fixturePath.toLowerCase().endsWith(".pdf");
  let stagedFixture = fixturePath;
  if (isPdf) {
    fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
    stagedFixture = path.join(runDir, "pdfs", path.basename(fixturePath));
    fs.copyFileSync(fixturePath, stagedFixture);
  }

  const session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    appEnv: isPdf ? {} : { PDF2TEST_AUTOMATION_SOURCE_FILES: stagedFixture },
  });
  let verdict = "failed";
  try {
    report.browserArgs = session.browserArgs;

    // 0) 视口对齐截图尺寸。桌面 WebView2 也认 Emulation 覆盖，改完必须回读确认。
    await session.cdp.send("Emulation.setDeviceMetricsOverride", {
      width: viewportWidth, height: viewportHeight, deviceScaleFactor: 1, mobile: false,
    });
    await sleep(600);
    report.viewportActual = await session.evaluate("({ w: window.innerWidth, h: window.innerHeight })");

    // 1) 导入夹具 → 打开工作区（与真人一致：题库页 → 导入 → 点开）。
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library" });
    const before = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
    await session.clickSelector(isPdf ? '[data-testid="import-pick-folder"]' : '[data-testid="import-pick-files"]');
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

    // 等本地稿落盘（ds 非空）再进工作区；草稿未就绪时题号/答案位都还没渲染，量不到东西。
    const draftDeadline = Date.now() + 120000;
    while (Date.now() < draftDeadline) {
      const r = await session.invoke("get_workspace_item", { itemId });
      if (r?.ok && r.value?.ds) break;
      await sleep(2000);
    }
    await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace" });

    // 2) 权威稿（真实 IR）留档：题面重复的判定要用它，而不是靠文本相似度猜。
    const ir = await session.invoke("get_workspace_item", { itemId });
    report.irPresent = Boolean(ir?.ok && ir.value?.ds);
    report.irDuplication = readIrDuplication(ir?.value?.ds);
    report.irTaskGroups = (ir?.value?.ds?.taskGroups ?? []).map((t) => ({
      taskId: t.taskId,
      taskType: t.taskType ?? null,
      instructionNodes: (t.instructions ?? []).length,
      stimulusNodes: (t.stimulus ?? []).length,
      responseGroups: (t.responseGroups ?? []).map((g) => ({
        responseGroupId: g.responseGroupId ?? null,
        kind: g.kind ?? null,
        // ResponseGroupV2 上没有 `stimulus`，题干在 `prompt`；第一版读错字段。
        promptNodes: (g.prompt ?? []).length,
        slotIds: (g.slotIds ?? []).length,
      })),
    }));

    // 3) 快照 A：建议面板**关闭**（默认）——题稿应当占满整个工作区。
    const snapClosed = { label: "recognition-closed", probes: await session.evaluate(COLLECT_FN) };
    report.snapshots.push(snapClosed);
    await session.screenshot("01-recognition-closed");
    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    report.pageExceptions = session.cdp.events
      .filter((e) => e.method === "Runtime.exceptionThrown")
      .map((e) => (e.params?.exceptionDetails?.exception?.description ?? e.params?.exceptionDetails?.text ?? "").slice(0, 400));

    // 4) 快照 B：**打开**建议面板——缺陷就在这一步暴露（面板挤进同一行、题稿被推到右半）。
    await session.clickSelector('[data-testid="workspace-recognition-toggle"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-recognition"]')`, { timeoutMs: 20000, label: "recognition-panel" });
    await sleep(500);
    const snapOpen = { label: "recognition-open", probes: await session.evaluate(COLLECT_FN) };
    report.snapshots.push(snapOpen);
    await session.screenshot("02-recognition-open");
    report.duplicateText = await session.evaluate(DUPLICATE_FN);

    // 5) 快照 C：**问题列表**打开（另一个动态带，不得再次破坏网格）。
    await session.clickSelector('[data-testid="workspace-issues"]');
    await sleep(400);
    const snapIssues = { label: "issues-open", probes: await session.evaluate(COLLECT_FN) };
    report.snapshots.push(snapIssues);
    await session.screenshot("03-issues-open");

    // 6) 快照 D：窄屏（980px 断点附近）——不得靠固定宽度或隐藏溢出来掩盖错位。
    await session.cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 900, height: viewportHeight, deviceScaleFactor: 1, mobile: false,
    });
    await sleep(600);
    const snapNarrow = { label: "narrow-900", probes: await session.evaluate(COLLECT_FN) };
    report.snapshots.push(snapNarrow);
    await session.screenshot("04-narrow-900");

    // 7) 回到原视口，把两个面板**关掉**：题稿必须回到占满整个工作区。
    //    「打开时错位、关掉后仍留一列空白」也是一种缺陷，只看打开态会漏掉。
    await session.cdp.send("Emulation.setDeviceMetricsOverride", {
      width: viewportWidth, height: viewportHeight, deviceScaleFactor: 1, mobile: false,
    });
    await sleep(500);
    await session.clickSelector('[data-testid="workspace-recognition-toggle"]');
    await sleep(400);
    report.snapshots.push({ label: "recognition-closed-again", probes: await session.evaluate(COLLECT_FN) });
    await session.screenshot("05-recognition-closed-again");
    await session.clickSelector('[data-testid="workspace-issues"]');
    await sleep(400);
    report.snapshots.push({ label: "issues-closed", probes: await session.evaluate(COLLECT_FN) });
    await session.screenshot("06-issues-closed");

    // 8) 编辑 / 学生预览 切换：两条路径都要占满工作区，双栏都要在视口内。
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-student-preview"]')`, { timeoutMs: 20000, label: "student-preview" });
    await sleep(500);
    report.snapshots.push({ label: "student-preview", probes: await session.evaluate(COLLECT_FN) });
    await session.screenshot("07-student-preview");
    await session.clickSelector('[data-testid="workspace-mode-edit"]');
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"] .exam-canvas-v2')`, { timeoutMs: 20000, label: "back-to-edit" });
    await sleep(400);
    report.snapshots.push({ label: "edit-again", probes: await session.evaluate(COLLECT_FN) });
    await session.screenshot("08-edit-again");

    // 9) 编辑保存与重新打开：改一个答案 → 等保存态落定 → 回题库 → 重新打开 → 值还在。
    //    这条同时验证「保存链没被本轮 CSS 改动碰坏」——改了页面骨架却把编辑器弄丢，
    //    是这类改动的典型副作用。
    const answerSel = '.workspace-body .v2-answer-slot-text > input';
    const hasAnswerInput = await session.evaluate(`!!document.querySelector(${JSON.stringify(answerSel)})`);
    report.saveCycle = { hasAnswerInput: Boolean(hasAnswerInput) };
    if (hasAnswerInput) {
      const marker = "qa-layout";
      const before = await session.evaluate(`document.querySelector(${JSON.stringify(answerSel)}).value`);
      await session.typeInto(answerSel, marker);
      await session.waitFor(
        `(() => { const el = document.querySelector('[data-testid="workspace-save-state"]'); return !!el && /已保存/.test(el.textContent || ''); })()`,
        { timeoutMs: 40000, label: "save-state-saved" }
      );
      report.saveCycle.before = before;
      report.saveCycle.typed = marker;
      await session.screenshot("09-answer-edited");
      await session.clickSelector('[data-testid="workspace-back"]');
      await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000, label: "library-after-back" });
      await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
      await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-reopen" });
      await sleep(800);
      const reopened = await session.evaluate(`document.querySelector(${JSON.stringify(answerSel)})?.value ?? null`);
      report.saveCycle.reopened = reopened;
      report.saveCycle.roundTripped = reopened === marker;
      report.snapshots.push({ label: "reopened", probes: await session.evaluate(COLLECT_FN) });
      await session.screenshot("10-reopened");
    }

    // 10) 判定：每个快照都要满足 L1–L8 与 L10（L9 是单列断言，在下面单独追加）。
    const all = [];
    for (const snap of report.snapshots) all.push(...evaluateLayout(snap));
    // 同一断言在多个快照下都过才算过；把结果按 id 聚合，便于一眼看出是哪一步坏的。
    const byId = new Map();
    for (const [id, ok, detail] of all) {
      const cur = byId.get(id) ?? { id, ok: true, details: [] };
      cur.ok = cur.ok && ok;
      cur.details.push(detail);
      byId.set(id, cur);
    }
    report.assertionSummary = [...byId.values()].map((v) => ({ id: v.id, ok: v.ok, details: v.details }));
    for (const v of report.assertionSummary) {
      recordAssertion(v.id, v.ok, v.details.join(" | "));
    }

    // L9：编辑保存与重新打开（单列，不属于任何快照）。
    // 「有输入框」是这条断言的前提——没有输入框时它**没跑过**，不能算通过。
    const sc = report.saveCycle ?? {};
    if (!sc.hasAnswerInput) {
      report.assertionSummary.push({ id: "L9 edit-save-reopen", ok: false, details: ["题面上找不到可编辑的行内答案输入框，这条断言没有执行"] });
    } else {
      report.assertionSummary.push({
        id: "L9 edit-save-reopen",
        ok: sc.roundTripped === true,
        details: [`写入「${sc.typed}」→ 保存态落定 → 返回题库 → 重新打开 → 读回「${sc.reopened}」（原值「${sc.before}」）`],
      });
    }
    const l9 = report.assertionSummary.find((a) => a.id === "L9 edit-save-reopen");
    recordAssertion(l9.id, l9.ok, l9.details.join(" | "));

    verdict = report.assertionSummary.every((a) => a.ok) ? "passed" : "failed";
    report.verdict = verdict;
    report.failedAssertions = report.assertionSummary.filter((a) => !a.ok).map((a) => a.id);
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
    console.log(`[layout] verdict=${verdict} assertions=${report.assertionSummary?.length ?? 0} failed=${JSON.stringify(report.failedAssertions ?? [])}`);
    console.log(`[layout] report=${file}`);
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
  console.error(`[layout] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${report.error}`);
  process.exitCode = cannotRun ? 3 : 1;
});
