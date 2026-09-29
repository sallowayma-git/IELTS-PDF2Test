#!/usr/bin/env node
// Real Tauri UI audit: import one PDF, tour the reachable authoring surfaces,
// capture screenshots and interactive-control styles, and fail on bare controls.
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  CDP_CHANNEL_LABEL,
  CDP_CHANNEL_NOTE,
  assertBuildFresh,
  buildFreshReport,
  gitHead,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  sleep,
} from "./lib/tauri-cdp-harness.mjs";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const option = (name) => {
  const inline = process.argv.find((arg) => arg.startsWith(`${name}=`));
  if (inline) return inline.slice(name.length + 1);
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
};
const phase = option("--phase") ?? "audit";
const allowStaleExe = process.argv.includes("--allow-stale-exe");
const exePath = path.resolve(option("--exe") ?? path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe"));
const runDir = path.resolve(option("--run-dir") ?? path.join(repoRoot, "artifacts", "ui-audit", `${phase}-${stamp}`));
const privatePdf = path.join(repoRoot, "artifacts", "ui-audit-local", "private-sample.pdf");
const fallbackPdf = path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf");
const pdfPath = path.resolve(option("--pdf") ?? process.env.PDF2TEST_UI_AUDIT_PDF ?? (fs.existsSync(privatePdf) ? privatePdf : fallbackPdf));
const viewport = { width: 1100, height: 760, deviceScaleFactor: 1, mobile: false };
const screenshotsDir = runDir;
const jsonReportPath = path.join(runDir, "ui-audit-report.json");
const markdownReportPath = path.join(runDir, "ui-audit-report.md");

const report = {
  task: "tauri-cdp-ui-audit",
  phase,
  evidenceLevel: "real Tauri app end-to-end (WebView2 CDP automation channel)",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  viewport,
  pages: [],
  violations: [],
  assertions: [],
  errors: [],
};

const inventoryExpression = `(() => {
  const elements = [...document.querySelectorAll('button, a, [role="button"]')];
  const esc = (value) => CSS.escape(value);
  const selectorFor = (el) => {
    const testId = el.getAttribute('data-testid');
    if (testId) return '[data-testid="' + testId.replaceAll('"', '\\\\"') + '"]';
    if (el.id) return '#' + esc(el.id);
    const parts = [];
    for (let node = el; node && node !== document.documentElement; node = node.parentElement) {
      if (node.id) { parts.unshift('#' + esc(node.id)); break; }
      const tag = node.tagName.toLowerCase();
      const classes = [...node.classList].map((name) => '.' + esc(name)).join('');
      const siblings = node.parentElement ? [...node.parentElement.children].filter((sibling) => sibling.tagName === node.tagName) : [];
      const nth = siblings.length > 1 ? ':nth-of-type(' + (siblings.indexOf(node) + 1) + ')' : '';
      parts.unshift(tag + classes + nth);
      if (node.parentElement?.id === 'root') { parts.unshift('#root'); break; }
    }
    return parts.join(' > ');
  };
  const colorIsTransparent = (color) => !color || color === 'transparent' || color === 'rgba(0, 0, 0, 0)';
  const px = (value) => Number.parseFloat(value) || 0;
  return elements.map((el) => {
    const style = getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    const tag = el.tagName.toLowerCase();
    const classes = el.getAttribute('class') || '';
    const hasHiddenAncestor = Boolean(el.closest('[hidden], [aria-hidden="true"]'));
    const visible = !hasHiddenAncestor && style.display !== 'none' && style.visibility !== 'hidden'
      && px(style.opacity) !== 0 && rect.width > 0 && rect.height > 0;
    const borderWidth = Math.max(px(style.borderTopWidth), px(style.borderRightWidth), px(style.borderBottomWidth), px(style.borderLeftWidth));
    const paddingTotal = px(style.paddingTop) + px(style.paddingRight) + px(style.paddingBottom) + px(style.paddingLeft);
    const hasSurface = !colorIsTransparent(style.backgroundColor) || style.backgroundImage !== 'none';
    const hasBorder = style.borderStyle !== 'none' && borderWidth > 0;
    const hasSpacing = paddingTotal > 0;
    const defaultButtonChrome = tag === 'button' && (style.borderStyle === 'outset'
      || (style.borderRadius === '0px' && borderWidth >= 1 && paddingTotal <= 20 && !hasSurface));
    const defaultLinkChrome = tag === 'a' && style.color === 'rgb(0, 0, 238)'
      && style.textDecorationLine.includes('underline');
    const inlineEditorTarget = el.matches('.v2-author-editable[role="button"], .workspace-title-editable [role="button"]');
    const compositeControl = el.matches('.brand') || (el.matches('.library-row-main') && Boolean(el.closest('.library-row')));
    const canvasHoverControl = Boolean(el.closest('.exam-canvas-v2'))
      && el.matches('.v2-option-drag-handle, .v2-option-delete');
    const styleless = !hasSurface && !hasBorder && !hasSpacing && style.textDecorationLine === 'none';
    const likelyUnstyled = visible && !inlineEditorTarget && !compositeControl && !canvasHoverControl
      && (defaultButtonChrome || defaultLinkChrome || styleless);
    const text = (el.getAttribute('aria-label') || el.getAttribute('title') || el.innerText || el.textContent || '')
      .replace(/\\s+/g, ' ').trim().slice(0, 180);
    return {
      selector: selectorFor(el), tag, role: el.getAttribute('role'), text,
      testId: el.getAttribute('data-testid'), className: classes,
      visible, disabled: Boolean(el.disabled), likelyUnstyled,
      contextualStyle: inlineEditorTarget ? 'canvas text-edit affordance'
        : compositeControl ? 'logo or styled row surface'
          : canvasHoverControl ? 'canvas hover/focus icon control' : null,
      screenshotVisible: rect.bottom > 0 && rect.top < innerHeight && rect.right > 0 && rect.left < innerWidth,
      unstyledReason: !likelyUnstyled ? null : defaultButtonChrome ? '浏览器默认按钮外观'
        : defaultLinkChrome ? '浏览器默认链接外观' : '无边框、背景、内边距或文字装饰',
      computed: {
        padding: style.padding, border: style.border, borderRadius: style.borderRadius,
        backgroundColor: style.backgroundColor, color: style.color,
        textDecorationLine: style.textDecorationLine, outline: style.outline,
      },
      rect: { x: Math.round(rect.x * 10) / 10, y: Math.round(rect.y * 10) / 10,
        width: Math.round(rect.width * 10) / 10, height: Math.round(rect.height * 10) / 10,
        right: Math.round(rect.right * 10) / 10, bottom: Math.round(rect.bottom * 10) / 10 },
    };
  });
})()`;

function persistReports() {
  fs.mkdirSync(runDir, { recursive: true });
  fs.writeFileSync(jsonReportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  const rows = report.violations.length
    ? report.violations.map((item) => `| ${item.page} | \`${item.selector}\` | ${item.text || "（无文本）"} | (${item.rect.x}, ${item.rect.y}) ${item.rect.width}×${item.rect.height}; 截图内=${item.screenshotVisible ? "是" : "否"} | ${item.screenshot} |`).join("\n")
    : "| — | — | 未发现浏览器默认或无样式交互控件 | — | — |";
  const pageRows = report.pages.map((item) => `| ${item.id} | ${item.label} | ${item.controls.length} | ${item.violations.length} | ${item.screenshot} |`).join("\n");
  const markdown = [
    "# Tauri UI button audit",
    "",
    `- Phase: ${report.phase}`,
    `- Evidence: ${report.evidenceLevel}`,
    `- Viewport: ${viewport.width}×${viewport.height} CSS px` ,
    `- PDF: ${report.identity?.pdfName ?? "not staged"}`,
    `- Unstyled controls: ${report.violations.length}`,
    "",
    "## Screenshots and page inventories",
    "",
    "| Page | Surface | Controls | Unstyled | Screenshot |",
    "|---|---|---:|---:|---|",
    pageRows || "| — | No pages captured | 0 | 0 | — |",
    "",
    "## Unstyled controls (before fix)",
    "",
    "| Page | Selector | Text | Screenshot position | Screenshot |",
    "|---|---|---|---|---|",
    rows,
    "",
    report.errors.length ? `## Errors\n\n${report.errors.map((error) => `- ${error}`).join("\n")}` : "",
  ].filter(Boolean).join("\n");
  fs.writeFileSync(markdownReportPath, `${markdown}\n`, "utf8");
}

async function capturePage(session, id, label, rootSelector) {
  await session.waitFor(`!!document.querySelector(${JSON.stringify(rootSelector)})`, { timeoutMs: 30000, label: `${id}-root` });
  await sleep(180);
  const screenshot = await session.screenshot(id);
  if (!screenshot) throw new Error(`无法截图页面：${id}`);
  const controls = await session.evaluate(inventoryExpression);
  const layout = await session.evaluate(`(() => ({
    viewportWidth: document.documentElement.clientWidth,
    documentWidth: Math.max(document.documentElement.scrollWidth, document.body?.scrollWidth || 0),
    documentOverflowX: Math.max(document.documentElement.scrollWidth, document.body?.scrollWidth || 0) > document.documentElement.clientWidth + 1,
    offscreenControls: [...document.querySelectorAll('button, a, [role="button"]')].filter((el) => {
      const r = el.getBoundingClientRect();
      const s = getComputedStyle(el);
      return s.display !== 'none' && s.visibility !== 'hidden' && r.width > 0 && r.height > 0
        && (r.left < -1 || r.right > document.documentElement.clientWidth + 1);
    }).length,
  }))()`);
  const pageViolations = controls.filter((control) => control.likelyUnstyled);
  const relativeScreenshot = path.relative(repoRoot, screenshot).replace(/\\/g, "/");
  const page = { id, label, rootSelector, screenshot: relativeScreenshot, viewport, layout, controls, violations: pageViolations };
  report.pages.push(page);
  report.violations.push(...pageViolations.map((control) => ({ ...control, page: id, label, screenshot: relativeScreenshot })));
  const inventoryFile = path.join(screenshotsDir, `${id}-controls.json`);
  fs.writeFileSync(inventoryFile, `${JSON.stringify({ page: id, label, screenshot: relativeScreenshot, viewport, layout, controls }, null, 2)}\n`, "utf8");
  report.assertions.push({ id: `${id}-no-horizontal-overflow`, ok: !layout.documentOverflowX && layout.offscreenControls === 0, detail: layout });
  console.log(`[ui-audit] ${id}: ${controls.length} controls, ${pageViolations.length} unstyled, screenshot=${relativeScreenshot}`);
  persistReports();
  return page;
}

async function keyboardFocusProbe(session, pageId) {
  const key = { key: "Tab", code: "Tab", windowsVirtualKeyCode: 9, nativeVirtualKeyCode: 9 };
  await session.cdp.send("Input.dispatchKeyEvent", { ...key, type: "keyDown" });
  await session.cdp.send("Input.dispatchKeyEvent", { ...key, type: "keyUp" });
  const focus = await session.evaluate(`(() => {
    const el = document.activeElement;
    if (!el || el === document.body || el === document.documentElement) return null;
    const s = getComputedStyle(el);
    return { tag: el.tagName.toLowerCase(), text: (el.innerText || el.getAttribute('aria-label') || '').trim().slice(0, 100),
      outlineStyle: s.outlineStyle, outlineWidth: s.outlineWidth, outlineColor: s.outlineColor };
  })()`);
  const visible = Boolean(focus && focus.outlineStyle !== "none" && Number.parseFloat(focus.outlineWidth) > 0);
  report.assertions.push({ id: `${pageId}-keyboard-focus-visible`, ok: visible, detail: focus });
  console.log(`[ui-audit] ${pageId} keyboard focus: ${visible ? "visible" : "not observed"}`);
}

async function main() {
  fs.mkdirSync(runDir, { recursive: true });
  if (!fs.existsSync(pdfPath)) throw new Error(`巡检 PDF 不存在：${pdfPath}（可用 --pdf 指定）`);
  if (!fs.existsSync(exePath)) throw new Error(`Tauri exe 不存在：${exePath}（先运行 npm run build:app）`);
  const freshness = allowStaleExe ? null : assertBuildFresh({ exePath });
  report.identity = {
    exePath,
    exeSha256: sha256File(exePath),
    commit: gitHead(repoRoot),
    buildFresh: freshness ? buildFreshReport(freshness) : { status: "bypassed for pre-change capture" },
    pdfName: path.basename(pdfPath),
    pdfSha256: sha256File(pdfPath),
  };
  const stagedPdf = path.join(runDir, "pdfs", path.basename(pdfPath));
  fs.mkdirSync(path.dirname(stagedPdf), { recursive: true });
  fs.copyFileSync(pdfPath, stagedPdf);

  let session;
  let fatal = null;
  try {
    session = await launchTauriAppCdp({ exePath, runDir });
    report.browserArgs = session.browserArgs;
    await session.cdp.send("Emulation.setDeviceMetricsOverride", viewport);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });

    await session.evaluate(`(() => { location.hash = "#/library"; return true; })()`);
    await capturePage(session, "library", "题库", '[data-testid="library-page"]');
    await keyboardFocusProbe(session, "library");

    await session.clickSelectorWhenStable('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
    await capturePage(session, "import-drawer", "导入抽屉", '[data-testid="import-drawer"]');
    await session.clickSelectorWhenStable('[data-testid="import-pick-folder"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "import-picked-files" });
    await capturePage(session, "import-drawer-selected", "导入抽屉（已选择 PDF）", '[data-testid="import-drawer"]');

    const existingIds = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map((row) => row.getAttribute('data-item-id'))`);
    await session.clickSelectorWhenStable('[data-testid="import-start"]');
    const importDeadline = Date.now() + 150000;
    let itemId = null;
    while (Date.now() < importDeadline && !itemId) {
      const ids = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map((row) => row.getAttribute('data-item-id'))`);
      itemId = (ids ?? []).find((id) => !(existingIds ?? []).includes(id)) ?? null;
      if (!itemId) await sleep(900);
    }
    if (!itemId) throw new Error("真实 PDF 导入后没有出现新的题库条目");
    report.identity.itemId = itemId;
    await session.waitFor(`!!document.querySelector('[data-item-id="${itemId}"] .library-row-main')`, { timeoutMs: 30000, label: "imported-library-row" });
    const partSelect = await session.evaluate(`(() => {
      const el = document.querySelector('[data-testid="library-row-part-select"]');
      const value = el && [...el.options].find((option) => option.value)?.value;
      if (!el || !value) return null;
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set;
      setter.call(el, value);
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
      return value;
    })()`);
    if (partSelect) {
      await session.waitFor(`!!document.querySelector('[data-testid="library-part-chip-${partSelect}"]')`, { timeoutMs: 15000, label: "part-filter-chip" });
      report.identity.partFilterExercised = partSelect;
    }
    await capturePage(session, "library-imported", "题库（真实 PDF 已导入）", '[data-testid="library-page"]');

    let draftReady = false;
    const draftDeadline = Date.now() + 120000;
    while (Date.now() < draftDeadline && !draftReady) {
      const result = await session.invoke("get_workspace_item", { itemId });
      draftReady = Boolean(result?.ok && result.value?.ds);
      if (!draftReady) await sleep(1200);
    }
    if (!draftReady) throw new Error("真实 PDF 的题稿未在时限内进入工作区");
    await session.clickSelectorWhenStable(`[data-item-id="${itemId}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace-open" });
    await capturePage(session, "workspace-edit", "工作区（编辑）", '[data-testid="exam-workspace"]');

    await session.clickSelectorWhenStable('[data-testid="workspace-mode-student"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-student-preview"]')`, { timeoutMs: 20000, label: "workspace-preview" });
    await capturePage(session, "workspace-preview", "工作区（学生预览）", '[data-testid="workspace-student-preview"]');

    await session.clickSelectorWhenStable('[data-testid="workspace-mode-edit"]');
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 15000, label: "workspace-edit-return" });
    await session.clickSelectorWhenStable('[data-testid="workspace-issues"]');
    await session.waitFor(`(() => { const el = document.querySelector('[data-testid="workspace-issue-list"]'); return !!el && el.getAttribute('data-preflight-state') !== 'loading'; })()`, { timeoutMs: 50000, label: "publish-preflight-settled" });
    await capturePage(session, "review-tasks", "识别/校核待办面板", '[data-testid="workspace-issue-list"]');
    await keyboardFocusProbe(session, "review-tasks");
    // 当前产品的发布门槛与待办同在工作区侧栏；“发布”本身是一键写入动作，巡检不点击它。
    await capturePage(session, "publish-preflight", "发布预检（工作区门槛面板）", '[data-testid="workspace-issue-list"]');

    await session.clickSelectorWhenStable('[data-testid="workspace-recognition-toggle"]');
    await session.waitFor(`!!document.querySelector('[data-testid="workspace-recognition"]')`, { timeoutMs: 20000, label: "recognition-panel" });
    await capturePage(session, "recognition-panel", "识别详情面板", '[data-testid="workspace-recognition"]');

    await session.evaluate(`(() => { location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 20000, label: "library-after-workspace" });
    const trashSelector = `[data-item-id="${itemId}"] .library-row-actions button.danger`;
    await session.waitFor(`!!document.querySelector(${JSON.stringify(trashSelector)})`, { timeoutMs: 15000, label: "library-row-trash-action" });
    await session.clickSelectorWhenStable(trashSelector);
    await session.waitFor(`!document.querySelector(${JSON.stringify(`[data-item-id="${itemId}"]`)})`, { timeoutMs: 20000, label: "row-moved-to-trash" });
    await session.clickSelectorWhenStable('[data-testid="library-tab-trash"]');
    await capturePage(session, "recycle-bin", "回收站", '[data-testid="library-page"]');

    await session.evaluate(`(() => { location.hash = "#/library?modality=writing"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-writing-panel"]')`, { timeoutMs: 20000, label: "writing-panel" });
    // 新建一条只存在于本次隔离 appdata 的空白草稿，以便同时巡检编辑态操作按钮。
    await session.clickSelectorWhenStable('.writing-create-form button');
    await session.waitFor(`!!document.querySelector('.writing-edit-actions')`, { timeoutMs: 20000, label: "writing-editor-actions" });
    await session.evaluate(`document.querySelector('.writing-edit-actions')?.scrollIntoView({ block: 'center', inline: 'nearest' })`);
    await sleep(150);
    await capturePage(session, "writing-subtab", "写作子标签（草稿编辑态）", '[data-testid="library-writing-panel"]');

    await session.evaluate(`(() => { location.hash = "#/settings"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="settings-page"]')`, { timeoutMs: 20000, label: "settings-page" });
    await capturePage(session, "settings", "设置页", '[data-testid="settings-page"]');
    await keyboardFocusProbe(session, "settings");
    const advancedToggle = await session.evaluate(`!!document.querySelector('[data-testid="settings-advanced-toggle"]')`);
    if (advancedToggle) {
      await session.clickSelectorWhenStable('[data-testid="settings-advanced-toggle"]');
      await capturePage(session, "settings-advanced", "设置页（高级设置）", '[data-testid="settings-advanced"]');
    }
  } catch (error) {
    fatal = error;
    report.errors.push(String(error?.stack ?? error));
  } finally {
    if (session) {
      const closed = await session.close();
      report.runtime = { exitCode: closed.exitCode, screenshotErrors: session.screenshotErrors };
    }
    const requiredPages = ["library", "import-drawer", "workspace-edit", "workspace-preview", "review-tasks", "recognition-panel", "recycle-bin", "writing-subtab", "settings", "publish-preflight"];
    for (const required of requiredPages) report.assertions.push({
      id: `page-captured:${required}`,
      ok: report.pages.some((page) => page.id === required),
      detail: report.pages.find((page) => page.id === required)?.screenshot ?? null,
    });
    for (const page of report.pages) report.assertions.push({
      id: `${page.id}-no-unstyled-controls`, ok: page.violations.length === 0,
      detail: page.violations.map(({ selector, text, rect, unstyledReason }) => ({ selector, text, rect, unstyledReason })),
    });
    persistReports();
  }

  const failed = report.assertions.filter((assertion) => !assertion.ok);
  console.log(`[ui-audit] report=${path.relative(repoRoot, markdownReportPath).replace(/\\/g, "/")}`);
  console.log(`[ui-audit] screenshots=${report.pages.length}, violations=${report.violations.length}, failed assertions=${failed.length}`);
  if (fatal) {
    console.error(`[ui-audit] FAILED: ${fatal.message}`);
    process.exitCode = 1;
  } else if (failed.length) {
    for (const assertion of failed) console.error(`[ui-audit] FAIL ${assertion.id}: ${JSON.stringify(assertion.detail)}`);
    process.exitCode = 1;
  }
}

main().catch((error) => {
  console.error(`[ui-audit] FAILED before report finalization: ${error?.stack ?? error}`);
  process.exitCode = 1;
});
