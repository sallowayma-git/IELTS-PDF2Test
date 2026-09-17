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
 *   L8 panes-independent-scroll   两栏可独立滚动——**实际滚动一栏，另一栏位置与 scrollTop 不变**
 *   L9 edit-save-reopen           编辑保存 → 返回题库 → 重新打开，值读得回来
 *   L10 header-spans-page         顶部工具栏与模式栏横跨工作区
 *   L11 passage-usable-height     题稿可用高度 ≥ 下限（面板开/关都不例外）
 *   L12 no-overlay-on-passage     没有任何辅助带压在题稿区域上
 *   L13 aside-shared-height-cap   两个辅助面板共用一个有总高度上限的区域（上限本身也要被量到）
 *   L14 aside-panels-exclusive    两个辅助面板互斥展开（两个方向都验）
 *
 * L8 的判据在第二轮被**替换**过：旧版只读 `overflow-y` 是不是 `auto`，那对缺陷恒为真
 * ——「声明了 overflow-y: auto」与「真的能独立滚动」是两件事。现在在页面里真的设
 * `scrollTop`，再量另一栏的矩形与 `scrollTop` 有没有被带动。
 * L11–L14 是第二轮新增：验收重点从「布局结构对不对」改成「用户有没有足够空间读题稿」。
 *
 * 另外记录（不作为硬断言，但必须出现在报告里）：
 *   - 控制台异常 / console.error（区分「运行时异常」与「布局错位」）
 *   - 工作区网格的 computed `grid-template-columns` 与各子项的 grid-row/column
 *   - 题面重复诊断：同一段文字在原文栏与题目栏是否都出现（对照 IR 判定哪一层产生）
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-workspace-layout.mjs [--pdf <path>] [--width N] [--height N]
 *        [--low-height N] [--keep] [--no-diagnostic-args] [--tolerate-concurrent-edits]
 *   `--width/--height` 是主视口（默认 1080×617，即用户截图的尺寸）；
 *   `--low-height` 是「低高度窗口」那一档的高度（默认 520）；窄屏那一档固定 900 宽。
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
// 低高度窗口（本轮任务书第 5 条）。617 是用户截图的尺寸，520 用来验证「窗口变矮时
// 辅助面板的上限会不会把题稿压没」——上限用的是 vh，这一档正是它该起作用的地方。
const lowHeightIdx = process.argv.indexOf("--low-height");
const lowHeight = lowHeightIdx >= 0 ? Number(process.argv[lowHeightIdx + 1]) || 520 : 520;
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
  lowHeightViewport: { width: viewportWidth, height: lowHeight },
  probes: {},
  snapshots: [],
  assertions: [],
  consoleErrors: [],
  pageExceptions: [],
  duplicateText: null,
  // 题目栏内部三个文本块的逐块取证（本轮任务书第 4 条：右侧摘要重复）。
  questionPaneLayers: null,
  // 上面那份取证汇总出来的重复对（诊断项，不是硬断言——成因在后端）。
  questionPaneDuplication: [],
  // 互斥展开与独立滚动的原始证据（L8 / L14 用）。
  exclusivityChecks: [],
  scrollProbes: [],
};

/** 被测元素：key → 选择器。选择器与组件里真实使用的类名一一对应。 */
const PROBES = [
  ["page", '[data-testid="exam-workspace"]'],
  ["header", ".workspace-header"],
  ["subHeader", ".workspace-sub-header"],
  // 两个辅助面板的**共用容器**（互斥展开 + 统一高度上限都在它身上）。
  ["aside", '[data-testid="workspace-aside"]'],
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
  // 工作区页面的**直接子项**（按 DOM 顺序）。用于遮挡检查：题稿之前的每一带都必须落在
  // 题稿上方，谁都不许压到题稿区域上。这一项是第二轮新增的——旧版只看建议面板与题稿的
  // 水平重叠，漏掉了通知带 / 保存失败提示 / 问题列表等其它动态带。
  const pageEl = document.querySelector('[data-testid="exam-workspace"]');
  out.__pageChildren = pageEl ? [...pageEl.children].map((el, index) => {
    const r = el.getBoundingClientRect();
    return {
      index,
      cls: (typeof el.className === 'string' && el.className) || el.tagName,
      isBody: el.classList.contains('workspace-body'),
      isAside: el.classList.contains('workspace-aside'),
      y: +r.y.toFixed(1), bottom: +r.bottom.toFixed(1),
      w: +r.width.toFixed(1), h: +r.height.toFixed(1),
    };
  }) : [];
  // 辅助面板容器的实际高度上限。**上限本身也要被量到**：只断言「面板没有变高」看不出
  // 「上限被收到容器上了」，下次有人再给单个面板加一条 vh 上限，这条断言不会响。
  const asideEl = document.querySelector('[data-testid="workspace-aside"]');
  out.__aside = asideEl ? (() => {
    const r = asideEl.getBoundingClientRect();
    const cs = getComputedStyle(asideEl);
    return {
      h: +r.height.toFixed(1),
      maxHeight: cs.maxHeight,
      overflowY: cs.overflowY,
      flex: cs.flex,
      // 同一时刻容器里有几个面板——互斥展开的**直接证据**（应为 1）。
      childCount: asideEl.children.length,
      children: [...asideEl.children].map((c) => ({
        cls: (typeof c.className === 'string' && c.className) || c.tagName,
        h: +c.getBoundingClientRect().height.toFixed(1),
        // 面板**自己**有没有 vh 上限。L13 要求这里是 none：上限必须只在容器上有一处。
        maxHeight: getComputedStyle(c).maxHeight,
        overflowY: getComputedStyle(c).overflowY,
      })),
      scrollHeight: asideEl.scrollHeight,
      clientHeight: asideEl.clientHeight,
    };
  })() : null;
  // 两个面板各自的**存在性**（互斥展开的反向证据：一个开时另一个必须不在 DOM 里）。
  out.__panelsPresent = {
    issues: Boolean(document.querySelector('[data-testid="workspace-issue-list"]')),
    recognition: Boolean(document.querySelector('[data-testid="workspace-recognition"]')),
  };
  out.__viewport = { innerWidth: window.innerWidth, innerHeight: window.innerHeight,
                     docScrollWidth: document.documentElement.scrollWidth,
                     docClientWidth: document.documentElement.clientWidth };
  return out;
})()`;

/**
 * 独立滚动的**实测**（替换旧版「读 overflow-y 是不是 auto」的弱判据）。
 *
 * 做法：记下两栏矩形与 scrollTop → 真的把原文栏 `scrollTop` 设成 140 → 再量两栏矩形与
 * scrollTop → 再滚题目栏、再量一次。判据是「滚一栏时另一栏的**矩形**与 **scrollTop**
 * 都不动」，而不是「声明了 overflow-y」。
 *
 * 同时如实报告两栏各自**能滚多少**（`scrollHeight - clientHeight`）。若某栏内容没有溢出，
 * 这条断言就没有真正被行使，报告里必须能看出来（`passageScrollable` / `questionScrollable`），
 * 免得把「没得滚」当成「滚过了，独立」。
 */
const SCROLL_FN = `(() => {
  const p = document.querySelector('.workspace-body .v2-passage-pane');
  const q = document.querySelector('.workspace-body .v2-question-pane');
  if (!p || !q) return null;
  const rect = (el) => { const r = el.getBoundingClientRect();
    return { x: +r.x.toFixed(1), y: +r.y.toFixed(1), w: +r.width.toFixed(1), h: +r.height.toFixed(1) }; };
  // 从零开始，避免上一次探针的残留影响判断。
  p.scrollTop = 0; q.scrollTop = 0;
  const before = { passage: rect(p), question: rect(q), pScrollTop: p.scrollTop, qScrollTop: q.scrollTop };
  const passageScrollable = Math.max(0, p.scrollHeight - p.clientHeight);
  const questionScrollable = Math.max(0, q.scrollHeight - q.clientHeight);
  p.scrollTop = 140;
  const afterPassageScroll = { passage: rect(p), question: rect(q), pScrollTop: p.scrollTop, qScrollTop: q.scrollTop };
  q.scrollTop = 140;
  const afterQuestionScroll = { passage: rect(p), question: rect(q), pScrollTop: p.scrollTop, qScrollTop: q.scrollTop };
  // 复原，免得影响后面的截图与其它探针。
  p.scrollTop = 0; q.scrollTop = 0;
  return { before, passageScrollable, questionScrollable, afterPassageScroll, afterQuestionScroll };
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

/**
 * 「题目栏内部重复」的取证（本轮任务书第 4 条）。
 *
 * 用户报的是**右侧**摘要重复：同一段摘要在题目栏里出现了两遍。
 * 旧探针（`DUPLICATE_FN`）比的是「题目栏 ↔ 原文栏」，轴不对——那个方向上实测
 * `echoedCount = 0`，看起来「没有重复」，而用户看到的重复一直都在。
 *
 * 这里按题面真实的三个文本块逐块取文本（与 `ExamCanvas` 的渲染结构一一对应）：
 *   - `.v2-instruction`     ← `taskGroup.instructions`
 *   - `.v2-stimulus`        ← `taskGroup.stimulus`
 *   - `.v2-response-prompt` ← `responseGroup.prompt`
 * 然后两两做包含比对，并分别报告每块里有没有源文的空位点线（`............`）与答案位数量
 * ——点线是「题干原文被搬进这一块」的指纹，真输入框才是答案位。
 *
 * 两个**必须**的细节，第一版都踩过：
 *   1. 空位在块与块之间写法不同：`instructions` 里是源文残留的点线，`stimulus` 里是真答案位
 *      （输入框 + 作者态的插入/删除工具字形 `＋ ×`）。直接逐字比对，同一段摘要会因为
 *      「空位写法不同」被判成不重复 —— 实测第一版 `duplicatedPairs` 为空就是这么漏的。
 *      所以比对前先做 `skeleton()` 归一化，把各种空位写法统一成一个记号。
 *   2. 判据必须是**最长公共子串**，不是「谁包含谁」。实测 group-1 的 `instructions` 是
 *      「指令句 + 摘要前半段」，`stimulus` 是「完整摘要」：两者互相都**不**完整包含对方，
 *      包含测试照样漏报。只有取最长公共子串才能量到「真正重复的那 517 个字」。
 *      （第三版把文本 `slice(0, 500)` 之后再比，670 字的指令被截到 500 字——同样漏报。
 *      现在全文只用于比较，报告里只存样本。）
 *
 * 只取证、不判定：判定哪一层该负责要对照 IR（`readIrDuplication`）。
 */
const QUESTION_DUPLICATION_FN = `(() => {
  const norm = (s) => (s || '').replace(/\\s+/g, ' ').trim();
  const skeleton = (s) => norm(s)
    .replace(/(?:[.．]\\s*){4,}/g, ' [B] ')
    .replace(/[＋+][\\s]*[×x]/g, ' [B] ')
    .replace(/(?:[…]\\s*){2,}/g, ' [B] ')
    .replace(/\\s+/g, ' ')
    .trim();
  // 最长公共子串（滚动数组 DP）。~700 字的块，几百次比较也就几十万次操作。
  const lcs = (a, b) => {
    const m = b.length;
    let prev = new Array(m + 1).fill(0);
    let cur = new Array(m + 1).fill(0);
    let best = 0; let end = 0;
    for (let i = 1; i <= a.length; i++) {
      for (let j = 1; j <= m; j++) {
        if (a[i - 1] === b[j - 1]) {
          cur[j] = prev[j - 1] + 1;
          if (cur[j] > best) { best = cur[j]; end = i; }
        } else { cur[j] = 0; }
      }
      const swap = prev; prev = cur; cur = swap;
    }
    return { chars: best, text: a.slice(end - best, end) };
  };
  const groups = [...document.querySelectorAll('.workspace-body .v2-task-group')];
  return groups.map((g) => {
    const els = [
      ['instructions', g.querySelector('.v2-instruction')],
      ['stimulus', g.querySelector('.v2-stimulus')],
      ['responsePrompt', g.querySelector('.v2-response-prompt')],
    ];
    const full = els.map(([role, el]) => {
      const raw = norm(el && el.innerText);
      return { role, el, raw, skel: skeleton(el && el.innerText) };
    });
    const pairs = [];
    for (let i = 0; i < full.length; i++) {
      for (let j = i + 1; j < full.length; j++) {
        const a = full[i]; const b = full[j];
        if (a.skel.length < 60 || b.skel.length < 60) continue;
        const shorter = a.skel.length <= b.skel.length ? a : b;
        const longer = a.skel.length <= b.skel.length ? b : a;
        // 整段包含是最强的情形（一条完全重复了另一条）；否则退到最长公共子串。
        const contained = longer.skel.includes(shorter.skel);
        const overlap = contained
          ? { chars: shorter.skel.length, text: shorter.skel }
          : lcs(a.skel, b.skel);
        const ratio = shorter.skel.length ? overlap.chars / shorter.skel.length : 0;
        // 阈值：重复段 ≥120 字**且**占较短那一块的 ≥40%。实测真重复是 517/635 = 81%，
        // 另外两组题型的偶然重合只有 3% / 10%，区分度足够。
        if (overlap.chars >= 120 && ratio >= 0.4) {
          pairs.push({
            a: a.role, b: b.role, contained,
            overlapChars: overlap.chars,
            shorterChars: shorter.skel.length,
            ratio: +ratio.toFixed(3),
            sample: overlap.text.slice(0, 240),
          });
        }
      }
    }
    const heading = g.querySelector('.v2-task-header h2');
    return {
      taskId: g.getAttribute('data-group-id'),
      heading: norm(heading && heading.innerText),
      blocks: full.map((b) => {
        const dotRuns = b.raw.match(/[.]{6,}/g) || [];
        return {
          role: b.role,
          present: Boolean(b.el),
          chars: b.raw.length,
          skeletonChars: b.skel.length,
          text: b.raw.slice(0, 400),
          skeleton: b.skel.slice(0, 400),
          slotCount: b.el ? b.el.querySelectorAll('.v2-answer-slot').length : 0,
          // 源文的空位是点线；答案位是真输入框。点线出现在哪一块，就说明那一块里装的是题干原文。
          dottedBlanks: dotRuns.length,
          longestDottedRun: Math.max(0, ...dotRuns.map((m) => m.length)),
        };
      }),
      duplicatedPairs: pairs,
    };
  });
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

  // L7：两栏都在视口内。
  // L8（独立滚动）**不在这里**——它需要真的滚一下再量，见 `evaluateScroll`。
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
  }

  // L10：顶部工具栏与模式栏必须横跨工作区（任务书第 2 条）。
  // 它们和题稿一样是页面骨架的直接子项，上一版只断言了题稿与建议面板，漏了这两条。
  const header = snap.probes?.header?.present ? rectOf(snap, "header") : null;
  const subHeader = snap.probes?.subHeader?.present ? rectOf(snap, "subHeader") : null;
  if (header && subHeader) {
    results.push(["L10 header-spans-page", header.w >= page.w - 4 && subHeader.w >= page.w - 4,
      `${tag}: 顶部栏宽 ${header.w.toFixed(1)}、模式栏宽 ${subHeader.w.toFixed(1)} / 工作区宽 ${page.w.toFixed(1)}`]);
  }

  // ── 第二轮新增：验收重点改成「用户有没有足够空间阅读和编辑题稿」 ──────────────

  // L11：题稿可用高度下限。
  // 这是本轮的核心指标。上一轮「同时展开两个辅助面板」时实测题稿只剩 153.4px（视口 617），
  // 用户报的就是这个。下限取 `max(200, 视口高 × 32%)`：用比例而不是固定 px，是因为
  // 缺陷的本质是「辅助带按比例把空间吃光」，固定 px 在高窗口下判不出来。
  const floor = Math.max(200, page.h * 0.32);
  const visiblePanes = [
    ["原文栏", passage, snap.probes?.passagePane?.display],
    ["题目栏", question, snap.probes?.questionPane?.display],
  ].filter(([, r, display]) => r && r.w > 0 && display !== "none");
  if (body && visiblePanes.length) {
    const minPaneH = Math.min(...visiblePanes.map(([, r]) => r.h));
    results.push(["L11 passage-usable-height", minPaneH >= floor,
      `${tag}: 题稿高 ${body.h.toFixed(1)}px，可见栏最小高 ${minPaneH.toFixed(1)}px（下限 ${floor.toFixed(1)}px = max(200, 视口高 ${page.h.toFixed(1)}×0.32)）`]);
  }

  // L12：遮挡检查。题稿之前的**每一带**（顶栏、模式栏、通知、保存失败提示、辅助面板容器、
  // 窄屏切换栏…）都必须落在题稿上方，谁都不许压到题稿区域上。
  // 旧版只比了「建议面板与题稿的水平重叠」，漏掉其余动态带。
  const children = snap.probes?.__pageChildren ?? [];
  const bodyIdx = children.findIndex((c) => c.isBody);
  if (body && bodyIdx >= 0) {
    const offenders = children.filter((c) => c.index < bodyIdx && c.h > 0.5 && c.bottom > body.y + 1);
    results.push(["L12 no-overlay-on-passage", offenders.length === 0,
      `${tag}: 题稿之前有 ${bodyIdx} 带，压到题稿区域（bottom > ${body.y.toFixed(1)}）的有 ${offenders.length} 个` +
      (offenders.length ? `（${offenders.map((c) => `${c.cls} bottom=${c.bottom}`).join("、")}）` : "")]);
  }

  // L13：两个辅助面板**共用一个有总高度上限的区域**。
  // 三条一起才说明「上限被收到了容器上」：
  //   a) 容器本身有有限 `max-height`（不是 `none`）；
  //   b) 容器里**同时只有一个**面板（互斥展开在 DOM 层面的结果）；
  //   c) 面板自己**没有** vh 上限（`max-height: none`）——否则就是把旧写法换个地方重来。
  // 另加一条与设计无关的信封：容器高度不得超过 `min(视口高×40%, 320px)`。
  // 旧行为（34vh + 46vh = 60% 视口）会突破这个信封，所以它能拦住回归。
  const aside = snap.probes?.__aside;
  if (aside) {
    const envelope = Math.min(page.h * 0.4, 320) + 4;
    const childCap = aside.children?.[0]?.maxHeight ?? null;
    const finiteCap = aside.maxHeight && aside.maxHeight !== "none";
    const ok = Boolean(finiteCap) && aside.h <= envelope && childCap === "none" && aside.childCount === 1;
    results.push(["L13 aside-shared-height-cap", ok,
      `${tag}: 容器 max-height=${aside.maxHeight}、实际高 ${aside.h.toFixed(1)}px（信封 ${envelope.toFixed(1)}px）、` +
      `容器内面板数 ${aside.childCount}、面板自身 max-height=${childCap}（要求 none）` +
      `（面板：${(aside.children ?? []).map((c) => `${c.cls} ${c.h}px`).join("、") || "无"}）`]);
  }

  return results;
}

/**
 * L8：独立滚动的**实测**。
 * 判据（全部成立才算过）：
 *   1. 两栏**都真的能滚**（`scrollHeight - clientHeight > 0`）——否则这条断言没被行使，
 *      不能算通过（用户明确要求「不能只根据 overflow-y:auto 就认定通过」）；
 *   2. 把原文栏滚 140px 后，它自己的 `scrollTop` 确实变了（证明它是真的滚动容器）；
 *   3. 此时题目栏的**矩形**与 **scrollTop** 都没动；
 *   4. 反向再滚题目栏，原文栏的矩形与 scrollTop 也没动。
 */
function evaluateScroll(scroll, tag) {
  if (!scroll) {
    return [["L8 panes-independent-scroll", false, `${tag}: 滚动探针没有拿到两栏，这条断言没有执行`]];
  }
  const { before, passageScrollable, questionScrollable, afterPassageScroll, afterQuestionScroll } = scroll;
  const sameRect = (a, b) =>
    Math.abs(a.x - b.x) <= 0.5 && Math.abs(a.y - b.y) <= 0.5
    && Math.abs(a.w - b.w) <= 0.5 && Math.abs(a.h - b.h) <= 0.5;
  const bothScrollable = passageScrollable > 0 && questionScrollable > 0;
  const passageScrolled = afterPassageScroll.pScrollTop > before.pScrollTop;
  const questionStayed = sameRect(before.question, afterPassageScroll.question)
    && afterPassageScroll.qScrollTop === before.qScrollTop;
  const passageStayed = sameRect(before.passage, afterQuestionScroll.passage)
    && afterQuestionScroll.pScrollTop === afterPassageScroll.pScrollTop;
  const ok = bothScrollable && passageScrolled && questionStayed && passageStayed;
  return [["L8 panes-independent-scroll", ok,
    `${tag}: 原文栏可滚 ${passageScrollable}px / 题目栏可滚 ${questionScrollable}px；` +
    `滚原文栏 scrollTop ${before.pScrollTop}→${afterPassageScroll.pScrollTop}` +
    `（题目栏矩形${questionStayed ? "不变" : "被带动"}、scrollTop ${afterPassageScroll.qScrollTop}）；` +
    `滚题目栏 scrollTop ${afterPassageScroll.qScrollTop}→${afterQuestionScroll.qScrollTop}` +
    `（原文栏矩形${passageStayed ? "不变" : "被带动"}、scrollTop ${afterQuestionScroll.pScrollTop}）`]];
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

    // 3) 快照 A：两个辅助面板都**关闭**（默认）——题稿应当占满整个工作区。
    const snapshot = async (label, shotName) => {
      const probes = await session.evaluate(COLLECT_FN);
      report.snapshots.push({ label, probes });
      if (shotName) await session.screenshot(shotName);
      return probes;
    };
    // 互斥展开的证据：点开某一个之后，**另一个必须不在 DOM 里**。两个方向都要验。
    const recordExclusivity = (label, expected) => {
      const present = report.snapshots[report.snapshots.length - 1]?.probes?.__panelsPresent;
      if (!present) return;
      const other = expected === "issues" ? "recognition" : "issues";
      report.exclusivityChecks.push({
        label, expected,
        issues: present.issues, recognition: present.recognition,
        ok: present[expected] === true && present[other] === false,
      });
    };
    // 独立滚动实测（在几个有代表性的状态下各做一次；滚动完自己复原）。
    const measureScroll = async (label) => {
      const value = await session.evaluate(SCROLL_FN);
      report.scrollProbes.push({ label, value });
      return value;
    };

    await snapshot("panels-closed", "01-panels-closed");
    // 题目栏内部重复的逐块取证（题目栏 ↔ 原文栏那一路单独记在 `duplicateText`）。
    report.questionPaneLayers = await session.evaluate(QUESTION_DUPLICATION_FN);
    // 汇总成一句可读的结论。**记为诊断项而不是硬断言**：重复的成因在源数据层
    // （`instructions` 里装了整段摘要，而同一段摘要又作为 `stimulus` 独立存在，见 findings），
    // 修法在后端。把它做成硬断言会让「布局验收」因为一个无关缺陷长期变红，
    // 反而盖住布局本身的回归信号。
    report.questionPaneDuplication = (report.questionPaneLayers ?? []).flatMap((g) =>
      (g.duplicatedPairs ?? []).map((p) => ({ taskId: g.taskId, heading: g.heading, ...p })));
    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    report.pageExceptions = session.cdp.events
      .filter((e) => e.method === "Runtime.exceptionThrown")
      .map((e) => (e.params?.exceptionDetails?.exception?.description ?? e.params?.exceptionDetails?.text ?? "").slice(0, 400));

    // 4) 快照 B：**打开**识别建议面板。
    await session.clickSelector('[data-testid="workspace-recognition-toggle"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-recognition"]')`, { timeoutMs: 20000, label: "recognition-panel" });
    await sleep(500);
    await snapshot("recognition-open", "02-recognition-open");
    recordExclusivity("open-recognition", "recognition");
    report.duplicateText = await session.evaluate(DUPLICATE_FN);

    // 5) 快照 C：再打开**问题列表** —— 两个辅助面板**互斥**，识别建议必须自动收起。
    //    互斥 + 共用上限一起保证「题稿不会被两个面板同时挤扁」（本轮任务书第 1 条）。
    await session.clickSelector('[data-testid="workspace-issues"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-issue-list"]')`, { timeoutMs: 20000, label: "issue-list" });
    await sleep(500);
    await snapshot("issues-open", "03-issues-open");
    recordExclusivity("open-issues", "issues");
    await measureScroll("issues-open");

    // 5b) 反向再验一次互斥：点识别建议 → 问题列表必须自动收起。
    await session.clickSelector('[data-testid="workspace-recognition-toggle"]');
    await sleep(500);
    await snapshot("recognition-open-again", "04-recognition-open-again");
    recordExclusivity("open-recognition-again", "recognition");

    // 5c) 关掉识别建议 → 两个面板都关闭（打开时占空间、关掉后必须一点都不留）。
    await session.clickSelector('[data-testid="workspace-recognition-toggle"]');
    await sleep(400);
    await snapshot("panels-closed-again", "05-panels-closed-again");
    await measureScroll("panels-closed");

    // 6) 低高度窗口（本轮任务书第 5 条）：题稿仍要有可用高度，面板上限也要跟着收缩。
    await session.cdp.send("Emulation.setDeviceMetricsOverride", {
      width: viewportWidth, height: lowHeight, deviceScaleFactor: 1, mobile: false,
    });
    await sleep(600);
    await session.clickSelector('[data-testid="workspace-issues"]');
    await sleep(500);
    await snapshot("low-height-issues-open", "06-low-height-issues-open");
    await measureScroll("low-height-issues-open");
    await session.clickSelector('[data-testid="workspace-issues"]');
    await sleep(400);
    await snapshot("low-height-panels-closed", "07-low-height-panels-closed");

    // 7) 窄屏（980px 断点附近）+ 面板打开：不得靠固定宽度或隐藏溢出来掩盖错位。
    await session.cdp.send("Emulation.setDeviceMetricsOverride", {
      width: 900, height: viewportHeight, deviceScaleFactor: 1, mobile: false,
    });
    await sleep(600);
    await session.clickSelector('[data-testid="workspace-issues"]');
    await sleep(500);
    await snapshot("narrow-900-issues-open", "08-narrow-900-issues-open");
    await session.clickSelector('[data-testid="workspace-issues"]');
    await sleep(400);
    await snapshot("narrow-900-panels-closed", "09-narrow-900-panels-closed");

    // 7b) 回到原视口，继续验编辑/预览与保存链。
    await session.cdp.send("Emulation.setDeviceMetricsOverride", {
      width: viewportWidth, height: viewportHeight, deviceScaleFactor: 1, mobile: false,
    });
    await sleep(500);

    // 8) 编辑 / 学生预览 切换：两条路径都要占满工作区，双栏都要在视口内。
    await session.clickSelector('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-student-preview"]')`, { timeoutMs: 20000, label: "student-preview" });
    await sleep(500);
    await snapshot("student-preview", "10-student-preview");
    await session.clickSelector('[data-testid="workspace-mode-edit"]');
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"] .exam-canvas-v2')`, { timeoutMs: 20000, label: "back-to-edit" });
    await sleep(400);
    await snapshot("edit-again", "11-edit-again");

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
      await session.screenshot("12-answer-edited");
      await session.clickSelector('[data-testid="workspace-back"]');
      await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 30000, label: "library-after-back" });
      await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
      await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-reopen" });
      await sleep(800);
      const reopened = await session.evaluate(`document.querySelector(${JSON.stringify(answerSel)})?.value ?? null`);
      report.saveCycle.reopened = reopened;
      report.saveCycle.roundTripped = reopened === marker;
      await snapshot("reopened", "13-reopened");
    }

    // 10) 判定：每个快照都要满足 L1–L7、L10–L13（L8 / L9 / L14 需要真的动一下，单独追加）。
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

    // L8：独立滚动实测（在几个代表性状态各做一次；全部通过才算过）。
    // 这里**不**接受「没得滚所以跳过」：内容没溢出就说明这条断言没被行使，
    // 如实判 FAIL 并写清原因，免得把「没跑」当成「跑过了」。
    const scrollResults = report.scrollProbes.flatMap(({ label, value }) => evaluateScroll(value, label));
    if (!scrollResults.length) {
      scrollResults.push(["L8 panes-independent-scroll", false, "滚动探针一次都没有执行"]);
    }
    const l8ok = scrollResults.every(([, ok]) => ok);
    report.assertionSummary.push({
      id: "L8 panes-independent-scroll",
      ok: l8ok,
      details: scrollResults.map(([, , detail]) => detail),
    });
    recordAssertion("L8 panes-independent-scroll", l8ok, scrollResults.map(([, , d]) => d).join(" | "));

    // L14：两个辅助面板互斥展开（两个方向都要验到）。
    const checks = report.exclusivityChecks ?? [];
    const hasBothDirections =
      checks.some((c) => c.expected === "issues" && c.ok)
      && checks.some((c) => c.expected === "recognition" && c.ok);
    const l14ok = checks.length > 0 && checks.every((c) => c.ok) && hasBothDirections;
    report.assertionSummary.push({
      id: "L14 aside-panels-exclusive",
      ok: l14ok,
      details: checks.map((c) => `${c.label}: 问题=${c.issues} 识别建议=${c.recognition}（期望 ${c.expected}）`),
    });
    recordAssertion("L14 aside-panels-exclusive", l14ok,
      `两个方向都验到=${hasBothDirections}；` + checks.map((c) => `${c.label}:问题=${c.issues}/识别=${c.recognition}`).join("、"));

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
