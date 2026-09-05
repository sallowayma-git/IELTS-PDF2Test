#!/usr/bin/env node
// 新三表面主链 UI 冒烟：题库导入 -> 打开工作区 -> 改一个字符 -> 保存。
//
// 覆盖层级（按 AGENTS.md 的要求明确声明）：
//   这是 **浏览器 + devFallbackBackend** 级别的验证，不是真实 Tauri/SQLite/文件系统链路。
//   它能证明 Phase 1-3 的路由收敛、ImportDrawer、题库行、工作区与原位编辑保存链是通的，
//   不能替代真实 Tauri E2E（计划 P0-T02，见 Dual_Recognition/task_plan.md 的 S0.6）。
import fs from "node:fs";
import path from "node:path";
import {
  DEFAULT_BASE_URL, argValue, createPage, ensureVite, evaluate, hasFlag,
  launchChrome, navigate, repoRoot, screenshot, setViewport, sleep, waitFor
} from "../lib/cdp.mjs";

const DEV_PICKED_PATHS_KEY = "ielts-author-studio.dev-fallback-picked-paths.v1";
const FIXTURE = path.join(repoRoot, "fixtures", "parser", "complex-reading.txt");
const OUT_DIR = path.join(repoRoot, "tmp", "library-workspace-smoke");

function js(value) {
  return JSON.stringify(value);
}

async function waitSelector(cdp, selector, timeoutMs = 30000) {
  await waitFor(`selector ${selector}`, async () =>
    evaluate(cdp, `Boolean(document.querySelector(${js(selector)}))`), { timeoutMs });
}

async function click(cdp, selector, timeoutMs = 30000) {
  await waitSelector(cdp, selector, timeoutMs);
  const clicked = await evaluate(cdp, `(() => {
    const el = document.querySelector(${js(selector)});
    if (!el || el.disabled) return false;
    el.click();
    return true;
  })()`);
  if (!clicked) throw new Error(`click failed or disabled: ${selector}`);
}

async function text(cdp, selector) {
  return evaluate(cdp, `document.querySelector(${js(selector)})?.textContent?.trim() ?? ""`);
}

const steps = [];
function record(name, detail) {
  steps.push({ name, detail });
  console.log(`  ok  ${name}${detail ? ` — ${detail}` : ""}`);
}

async function main() {
  const baseUrl = argValue("--base-url", DEFAULT_BASE_URL);
  const vite = await ensureVite(baseUrl, { noStartServer: hasFlag("--no-start-server"), verbose: hasFlag("--verbose") });
  const chrome = await launchChrome({ headful: hasFlag("--headful"), chromePath: argValue("--chrome-path") });
  let cdp;
  try {
    ({ cdp } = await createPage(chrome.port));
    await setViewport(cdp, { width: 1440, height: 960, deviceScaleFactor: 1 });
    await navigate(cdp, `${baseUrl}/#/library`);

    // 默认路由必须是题库（计划 §16.1 验收：默认 hash 为 /library）。
    const defaultHash = await evaluate(cdp, "window.location.hash");
    if (!defaultHash.startsWith("#/library")) throw new Error(`default route is ${defaultHash}, expected #/library`);
    record("默认进入题库", defaultHash);

    // 旧链接必须重定向到新路由，而不是渲染旧页面。
    await evaluate(cdp, `(() => { window.location.hash = "/dashboard"; return true; })()`);
    await sleep(500);
    const redirected = await evaluate(cdp, "window.location.hash");
    if (!redirected.startsWith("#/library")) throw new Error(`/dashboard did not redirect, got ${redirected}`);
    record("旧链接 /dashboard 重定向", redirected);

    // 预置 dev 文件选择：带上真实内容，浏览器开发预览才会走真实解析而不是报错。
    const fixtureText = fs.readFileSync(FIXTURE, "utf8");
    const preset = [{
      path: FIXTURE,
      name: path.basename(FIXTURE),
      sizeBytes: Buffer.byteLength(fixtureText, "utf8"),
      titleHint: "Complex Reading Smoke",
      textContent: fixtureText,
      requiresDesktopParser: false
    }];
    await evaluate(cdp, `(() => {
      window.localStorage.setItem(${js(DEV_PICKED_PATHS_KEY)}, ${js(JSON.stringify(preset))});
      return true;
    })()`);

    await click(cdp, '[data-testid="library-import"]');
    await waitSelector(cdp, '[data-testid="import-drawer"]');
    record("题库内打开导入抽屉");

    await click(cdp, '[data-testid="import-pick-files"]');
    await waitSelector(cdp, '[data-testid="import-picked-files"] li');
    record("选择文件", await text(cdp, '[data-testid="import-picked-files"] li .file-name'));

    await click(cdp, '[data-testid="import-start"]');
    // 抽屉必须在建行后立刻关闭，不等识别完成（计划 §12.2）。
    await waitFor("import drawer closes", async () =>
      evaluate(cdp, `!document.querySelector('[data-testid="import-drawer"]')`), { timeoutMs: 60000 });
    await waitSelector(cdp, '[data-testid="library-row"]', 60000);
    record("题库立即出现条目", await text(cdp, '[data-testid="library-row"] .file-name'));

    // 后台识别完成后行会离开处理中阶段。
    await waitFor("local recognition finishes", async () => evaluate(cdp, `(() => {
      const row = document.querySelector('[data-testid="library-row"]');
      if (!row) return false;
      return !/stage-(queued|local|cloud|reconciling)/.test(row.className);
    })()`), { timeoutMs: 180000, intervalMs: 1000 });
    record("本地识别完成", await text(cdp, '[data-testid="library-row"] .stage-pill'));

    await screenshot(cdp, path.join(OUT_DIR, "01-library.png"));

    const itemId = await evaluate(cdp, `document.querySelector('[data-testid="library-row"]')?.dataset.itemId ?? ""`);
    if (!itemId) throw new Error("library row has no data-item-id");
    await evaluate(cdp, `(() => { window.location.hash = "/items/" + ${js(itemId)}; return true; })()`);
    await waitSelector(cdp, '[data-testid="exam-workspace"]', 60000);
    record("从题库打开工作区", itemId);

    // 浏览器 + devFallbackBackend 不会为真实导入的 job 生成 V2 题稿
    // （它只为 phase5 fixture 和 apply patches 建过 authoringV2），
    // 所以这一段只断言「没有可编辑题稿时的降级是可用的」：
    // 必须给出人话说明 + 可执行按钮，而不是白屏或抛错。
    const hasCanvas = await evaluate(cdp, `Boolean(document.querySelector('[data-testid="exam-canvas-v2-author"]'))`);
    if (hasCanvas) {
      record("工作区直接渲染最终题面");
    } else {
      const message = await text(cdp, ".workspace-load-error .error-text");
      const recoverable = await evaluate(cdp, `Boolean(document.querySelector(".workspace-load-error .button-row button"))`);
      if (!message || !recoverable) throw new Error("workspace has neither a canvas nor a recoverable load state");
      record("无题稿时降级可用", message.slice(0, 60));
    }
    await screenshot(cdp, path.join(OUT_DIR, "02-workspace-from-library.png"));

    // ---- 工作区编辑链：用 phase5 fixture 题稿验证「改一个字符 -> 保存 -> 刷新仍在」 ----
    const editItemId = "phase5-editor-fixture";
    await navigate(cdp, `${baseUrl}/#/items/${editItemId}`);
    await waitSelector(cdp, '[data-testid="exam-canvas-v2-author"]', 60000);
    record("工作区打开即最终题面", editItemId);
    await screenshot(cdp, path.join(OUT_DIR, "03-workspace-canvas.png"));

    const hasModeSwitch = await evaluate(cdp, `Array.from(document.querySelectorAll("button"))
      .some((button) => /学生端预览|返回编辑/.test(button.textContent ?? ""))`);
    if (hasModeSwitch) throw new Error("workspace still exposes an edit/preview switch");
    record("没有编辑/预览切换");

    const before = await evaluate(cdp, `(() => {
      const el = document.querySelector('.exam-canvas-v2.is-author .v2-text.v2-author-editable');
      if (!el) return null;
      el.click();
      return el.textContent ?? "";
    })()`);
    if (before === null) throw new Error("no editable text node in author canvas");
    await waitSelector(cdp, ".inline-text-editor", 15000);
    record("进入原位编辑", JSON.stringify(before.slice(0, 32)));

    // 删掉最后一个字符，用 Enter 提交。
    // 不用 element.blur()：headless Chrome 下窗口未聚焦时它不会派发 focusout，
    // React 的 onBlur 收不到，会把环境限制误报成产品缺陷。
    const expected = await evaluate(cdp, `(() => {
      const editor = document.querySelector(".inline-text-editor");
      const next = editor.value.slice(0, -1);
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value").set;
      setter.call(editor, next);
      editor.dispatchEvent(new Event("input", { bubbles: true }));
      return next;
    })()`);
    await sleep(150);
    await evaluate(cdp, `(() => {
      const editor = document.querySelector(".inline-text-editor");
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      return true;
    })()`);
    if (expected === before) throw new Error("edit did not change the text");

    await waitFor("save confirmed", async () =>
      evaluate(cdp, `document.querySelector('[data-testid="workspace-save-state"]')?.textContent?.includes("已保存") ?? false`),
      { timeoutMs: 30000 });
    record("删一个字符后保存", `剩 ${expected.length} 字符`);

    await navigate(cdp, `${baseUrl}/#/items/${editItemId}`);
    await waitSelector(cdp, '[data-testid="exam-canvas-v2-author"]', 60000);
    const after = await evaluate(cdp, `(() => {
      const el = document.querySelector('.exam-canvas-v2.is-author .v2-text.v2-author-editable');
      return el ? el.textContent ?? "" : "";
    })()`);
    if (after !== expected) {
      throw new Error(`edit did not persist: expected ${JSON.stringify(expected.slice(-24))}, got ${JSON.stringify(after.slice(-24))}`);
    }
    record("刷新后修改仍在");
    await screenshot(cdp, path.join(OUT_DIR, "04-after-reload.png"));

    console.log(`\nlibrary-workspace-smoke: ${steps.length} steps passed`);
    console.log(`screenshots: ${path.relative(repoRoot, OUT_DIR)}`);
    return 0;
  } finally {
    cdp?.close();
    chrome.close();
    vite.close();
  }
}

main().then((code) => process.exit(code)).catch((error) => {
  console.error(`\nlibrary-workspace-smoke FAILED after ${steps.length} step(s): ${error.message}`);
  process.exit(1);
});
