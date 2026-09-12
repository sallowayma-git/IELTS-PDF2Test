#!/usr/bin/env node
// 真实 Tauri 产品回归：工作区返回按钮（D0）。
//
// 覆盖（D0 复核后的强化版）：
//   1. aria/结构 + computed style 未被 header 通用选择器覆盖（40×40 / 8px / 非 56px）。
//   2. flush 链证明：待保存编辑 → 立即点击返回 → 保存完成后才导航 → 重开 marker 仍在。
//      使用 invoke 闸门把 apply_editor_commands 挂起，点击返回后断言"未导航"，
//      释放闸门后才导航——证明返回按钮真的等待 flush，而不是碰巧被防抖先保存。
//   3. 可控保存失败：拦截 apply_editor_commands 直接 reject → 点击返回 →
//      不导航、错误可见、无 unhandledrejection。
//   4. 键盘可达：Tab 聚焦、:focus-visible 轮廓可见、Enter 触发返回。
//   5. 双击防护：busy 窗口内第二次点击不产生第二次保存请求。
//
// 证据层级：product（真实进程 + WebView2 + SQLite + 文件系统）。

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { By, Key, until } from "selenium-webdriver";
import {
  DEFAULT_EXE, DEFAULT_PDF, CannotRunError, assertPrerequisites, assertFreshBuild, buildFreshness,
  buildIdentity, launchTauriApp, createStepRecorder, importPdfViaFolderHook, waitForRowStage,
  openWorkspaceForItem, writeReport, exitCodeForVerdict, logCannotRun, logHarnessError
} from "./lib/tauri-harness.mjs";

const args = Object.fromEntries(process.argv.slice(2).reduce((all, arg, index, list) => {
  if (arg.startsWith("--")) all.set(arg.slice(2), list[index + 1]?.startsWith("--") ? true : list[index + 1] ?? true);
  return all;
}, new Map()));
const exePath = path.resolve(args.exe ?? DEFAULT_EXE);
const pdfPath = path.resolve(args.pdf ?? DEFAULT_PDF);
const keepRun = Boolean(args.keep);
const takeScreenshots = args.screenshot !== false;

/** 安装 invoke 拦截器。mode: "hold"（挂起 apply_editor_commands，放行时真实重发）/
 * "reject"（直接拒绝）/ "passthrough"。记录每次拦截，供双击防护断言计数。 */
const INSTALL_INTERCEPTOR = `
  const mode = arguments[0];
  window.__e2eRejections = window.__e2eRejections || [];
  if (!window.__e2eRejectionHooked) {
    window.__e2eRejectionHooked = true;
    window.addEventListener("unhandledrejection", (event) => window.__e2eRejections.push(String(event.reason)));
  }
  if (!window.__e2eOriginalInvoke) window.__e2eOriginalInvoke = window.__TAURI_INTERNALS__.invoke.bind(window.__TAURI_INTERNALS__);
  window.__e2eHeld = [];
  window.__e2eSaveRequests = 0;
  window.__e2eInvokeMode = mode;
  window.__TAURI_INTERNALS__.invoke = (cmd, args, options) => {
    if (cmd === "apply_editor_commands" && window.__e2eInvokeMode !== "passthrough") {
      window.__e2eSaveRequests += 1;
      if (window.__e2eInvokeMode === "reject") {
        return Promise.reject(new Error("E2E_SAVE_FAILURE_INJECTED"));
      }
      const pending = { args, options, resolve: null, reject: null };
      const gate = new Promise((resolve, reject) => { pending.resolve = resolve; pending.reject = reject; });
      window.__e2eHeld.push(pending);
      return gate;
    }
    return window.__e2eOriginalInvoke(cmd, args, options);
  };
  return "installed:" + mode;
`;

/** 放行：把挂起的保存请求原样发给真实后端（保证持久化真的发生）。 */
const RELEASE_HELD = `
  const held = window.__e2eHeld || [];
  window.__e2eInvokeMode = "passthrough";
  for (const pending of held) {
    window.__e2eOriginalInvoke("apply_editor_commands", pending.args, pending.options).then(
      (result) => pending.resolve(result),
      (error) => pending.reject(error)
    );
  }
  window.__e2eHeld = [];
  return held.length;
`;

/** 在 passage 原位编辑器里写入 marker（与 tauri-workspace-edit 同一产品路径）。 */
async function typeMarkerIntoPassage(driver, marker) {
  let editor = null;
  for (let attempt = 0; attempt < 3 && !editor; attempt += 1) {
    const span = await driver.wait(until.elementLocated(By.css(".v2-passage-pane .v2-text")), 15000);
    await driver.executeScript("arguments[0].click()", span);
    try {
      editor = await driver.wait(until.elementLocated(By.css('textarea[aria-label="编辑题目文字"]')), 5000);
    } catch {
      editor = null;
    }
  }
  if (!editor) throw new Error("点击 passage 文本未进入原位编辑器");
  await driver.wait(async () => {
    try {
      await editor.sendKeys(Key.chord(Key.CONTROL, "a"));
      return true;
    } catch {
      editor = await driver.wait(until.elementLocated(By.css('textarea[aria-label="编辑题目文字"]')), 5000);
      return false;
    }
  }, 15000);
  await editor.sendKeys(marker);
  try {
    await editor.sendKeys(Key.ENTER);
  } catch { /* Enter 提交后组件随即卸载，最后一次按键可能打到过期引用。 */ }
  return marker;
}

async function currentUrlIncludes(driver, fragment) {
  try {
    return (await driver.getCurrentUrl()).includes(fragment);
  } catch {
    return false;
  }
}

async function main() {
  assertPrerequisites({ exePath, pdfPath });
  const freshness = buildFreshness(exePath);
  assertFreshBuild(freshness);
  const identity = buildIdentity(exePath);

  const session = await launchTauriApp({ exePath, pdfPath, keep: keepRun, runPrefix: "workspace-back" });
  const artifacts = { dir: session.runDir, screenshotErrors: [] };
  const { steps, recordStep } = createStepRecorder({ artifacts, takeScreenshots });
  const driver = session.driver;
  const marker = "E2E BACK FLUSH 77";
  let itemId = null;

  try {
    await recordStep(driver, "library-page-loads", async () => {
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      await driver.executeScript(`
        window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false }));
        location.hash = "#/library";
      `);
      await driver.navigate().refresh();
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      return { url: await driver.getCurrentUrl() };
    });

    await recordStep(driver, "import-pdf-via-folder-hook", async () => {
      const result = await importPdfViaFolderHook(driver);
      itemId = result.itemId;
      return result;
    });

    await recordStep(driver, "background-pipeline-reaches-stable-stage", async () => {
      if (!itemId) throw new Error("缺少导入 item id");
      return waitForRowStage(driver, itemId, 240000);
    });

    await recordStep(driver, "workspace-opens", async () => {
      await openWorkspaceForItem(driver, itemId);
      const loadErrors = await driver.findElements(By.css(".workspace-load-error"));
      if (loadErrors.length) {
        throw new Error(`工作区加载失败：${(await loadErrors[0].getText()).slice(0, 300)}`);
      }
      return { itemId };
    });

    await recordStep(driver, "back-button-has-correct-test-id-and-aria", async () => {
      const backButton = await driver.wait(until.elementLocated(By.css(".workspace-back-button")), 10000);
      const ariaLabel = await backButton.getAttribute("aria-label");
      if (ariaLabel !== "返回题库") {
        throw new Error(`aria-label 不匹配：期望 "返回题库"，实际 "${ariaLabel}"`);
      }
      return { ariaLabel };
    });

    await recordStep(driver, "back-button-computed-style-not-overridden", async () => {
      const backButton = await driver.findElement(By.css(".workspace-back-button"));
      const styles = await driver.executeScript(`
        const el = arguments[0];
        const computed = window.getComputedStyle(el);
        return {
          backgroundColor: computed.backgroundColor,
          width: computed.width,
          height: computed.height,
          borderRadius: computed.borderRadius,
          minWidth: computed.minWidth
        };
      `, backButton);

      const failures = [];
      if (!/(transparent|rgba\(0,\s*0,\s*0,\s*0\))/.test(styles.backgroundColor)) {
        failures.push(`backgroundColor: 期望 transparent，实际 ${styles.backgroundColor}`);
      }
      if (styles.width !== "40px") failures.push(`width: 期望 40px，实际 ${styles.width}`);
      if (styles.height !== "40px") failures.push(`height: 期望 40px，实际 ${styles.height}`);
      if (styles.borderRadius !== "8px") failures.push(`borderRadius: 期望 8px，实际 ${styles.borderRadius}`);
      if (styles.minWidth === "56px") failures.push("minWidth 不应是 56px（被 .workspace-header button 规则覆盖）");
      if (failures.length) throw new Error(`CSS 样式被覆盖：${failures.join("; ")}`);
      return { styles };
    });

    // ── flush 链证明：挂起保存 → 待保存编辑 → 点击返回 → 未导航 → 放行 → 导航 ──
    await recordStep(driver, "back-button-holds-until-flush-completes", async () => {
      await driver.executeScript(INSTALL_INTERCEPTOR, "hold");
      await typeMarkerIntoPassage(driver, marker);
      // 等防抖（450ms）触发 persist，请求被闸门挂起。
      await driver.sleep(900);
      const backButton = await driver.findElement(By.css(".workspace-back-button"));
      await backButton.click();
      // flush 被挂起：断言短时间内不导航（证明返回按钮等待保存完成）。
      await driver.sleep(2500);
      const stillInWorkspace = await currentUrlIncludes(driver, "#/items/");
      if (!stillInWorkspace) {
        throw new Error("flush 未完成就发生了导航——返回按钮没有等待 flush（假绿风险）");
      }
      const heldCount = await driver.executeScript("return window.__e2eHeld.length;");
      if (heldCount < 1) {
        throw new Error("闸门上没有挂起的 apply_editor_commands——待保存编辑没有在保存链路上");
      }
      // 放行（真实重发到后端）→ flush 完成 → 导航到题库。
      await driver.executeScript(RELEASE_HELD);
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 20000);
      const url = await driver.getCurrentUrl();
      if (!url.includes("#/library")) throw new Error(`未导航回题库，当前 URL: ${url}`);
      return { saveRequestsHeld: heldCount, finalUrl: url, proof: "back-button awaited in-flight flush before navigating" };
    });

    await recordStep(driver, "flushed-marker-persists-after-reopen", async () => {
      await openWorkspaceForItem(driver, itemId);
      await driver.wait(until.elementLocated(By.css(".v2-passage-pane .v2-text")), 30000);
      const passageText = await driver.executeScript(
        "return document.querySelector('.v2-passage-pane') ? document.querySelector('.v2-passage-pane').innerText : '';"
      );
      if (!String(passageText).includes(marker)) {
        throw new Error(`重开后未找到 flush 保存的 marker "${marker}"，实际开头：${String(passageText).slice(0, 200)}`);
      }
      return { marker, persisted: true };
    });

    // ── 键盘可达性：Tab 聚焦 + focus-visible + Enter 返回 ──
    await recordStep(driver, "back-button-keyboard-focus-visible-and-enter", async () => {
      await driver.executeScript(INSTALL_INTERCEPTOR, "passthrough");
      // 焦点回到文档起点后按 Tab：back button 是工作区第一个可聚焦元素。
      await driver.executeScript("if (document.activeElement) document.activeElement.blur();");
      await driver.actions().sendKeys(Key.TAB).perform();
      const state = await driver.executeScript(`
        const el = document.activeElement;
        if (!el || !el.classList.contains("workspace-back-button")) {
          return { focused: false, tag: el ? el.tagName : null };
        }
        const computed = window.getComputedStyle(el);
        return {
          focused: true,
          focusVisible: el.matches(":focus-visible"),
          outlineWidth: computed.outlineWidth,
          outlineStyle: computed.outlineStyle,
          outlineColor: computed.outlineColor
        };
      `);
      if (!state.focused) {
        throw new Error(`Tab 后焦点不在返回按钮上（实际：${state.tag}）`);
      }
      if (!state.focusVisible) throw new Error("键盘聚焦后 :focus-visible 未生效");
      if (state.outlineStyle === "none" || state.outlineWidth === "0px") {
        throw new Error(`focus-visible 轮廓不可见：outline=${state.outlineWidth} ${state.outlineStyle}`);
      }
      // Enter 触发按钮：导航回题库（无待保存编辑，flush 立即完成）。
      await driver.actions().sendKeys(Key.ENTER).perform();
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 20000);
      return { ...state, enterNavigates: true };
    });

    // ── 双击防护：busy 窗口内第二次点击不得产生第二次保存请求 ──
    await recordStep(driver, "back-button-double-click-single-save", async () => {
      await openWorkspaceForItem(driver, itemId);
      await driver.executeScript(INSTALL_INTERCEPTOR, "hold");
      await typeMarkerIntoPassage(driver, "E2E BACK FLUSH 88");
      await driver.sleep(900);
      const backButton = await driver.findElement(By.css(".workspace-back-button"));
      await backButton.click();
      await backButton.click().catch(() => {}); // 第二次点击：busy 期间应被忽略
      await driver.sleep(800);
      // 期望恰一次挂起的保存请求（防抖批次）；busy 锁必须吞掉第二次点击，
      // 不得再产生新的保存调用。
      const saveRequests = await driver.executeScript("return window.__e2eSaveRequests || 0;");
      const stillHeld = await driver.executeScript("return window.__e2eHeld.length;");
      if (Number(saveRequests) !== 1 || Number(stillHeld) !== 1) {
        throw new Error(`双击产生了 ${saveRequests} 次保存请求（挂起 ${stillHeld}）——busy 锁未生效`);
      }
      await driver.executeScript(RELEASE_HELD);
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 20000);
      return { saveRequests, singleNavigation: true };
    });

    // ── 可控保存失败：不导航、错误可见、无 unhandledrejection ──
    await recordStep(driver, "save-failure-blocks-navigation-with-visible-error", async () => {
      await openWorkspaceForItem(driver, itemId);
      await driver.executeScript(INSTALL_INTERCEPTOR, "reject");
      const failureMarker = "E2E BACK FLUSH FAIL 99";
      await typeMarkerIntoPassage(driver, failureMarker);
      await driver.sleep(900);
      const backButton = await driver.findElement(By.css(".workspace-back-button"));
      await backButton.click();
      // flush reject：必须留在工作区，并给出用户可理解的错误。
      await driver.sleep(2500);
      if (!(await currentUrlIncludes(driver, "#/items/"))) {
        throw new Error("保存失败后发生了导航——失败必须阻断返回");
      }
      const errorVisible = await driver.wait(async () => {
        const notices = await driver.findElements(By.css(".workspace-notice"));
        for (const notice of notices) {
          const text = await notice.getText();
          if (/保存失败|失败/.test(text)) return text;
        }
        return false;
      }, 10000).catch(() => false);
      if (!errorVisible) throw new Error("保存失败后未见用户可理解的错误提示（.workspace-notice）");
      const rejections = await driver.executeScript("return window.__e2eRejections || [];");
      if (rejections.length) {
        throw new Error(`出现 unhandledrejection ${rejections.length} 条：${rejections[0]}`);
      }
      // 恢复：解除拦截后再次点击返回，flush 重试成功并导航（失败批次保留语义）。
      await driver.executeScript(INSTALL_INTERCEPTOR, "passthrough");
      await backButton.click().catch(async () => {
        const fresh = await driver.findElement(By.css(".workspace-back-button"));
        await fresh.click();
      });
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 20000);
      return { blockedNavigation: true, errorShown: String(errorVisible), unhandledRejections: 0 };
    });

    await recordStep(driver, "no-unhandledrejections-collected", async () => {
      const rejections = await driver.executeScript("return window.__e2eRejections || [];");
      if (rejections.length) throw new Error(`全程收集到 unhandledrejection：${JSON.stringify(rejections.slice(0, 3))}`);
      return { unhandledRejections: 0 };
    });

    const failed = steps.filter((step) => step.status === "failed");
    const report = {
      runId: path.basename(session.runDir),
      scenario: "workspace-back-button",
      coverage: "real-tauri-process+webview2+sqlite+filesystem",
      evidenceLevel: "product",
      exe: exePath,
      ...identity,
      ...freshness,
      pdf: pdfPath,
      steps,
      verdict: failed.length ? "failed" : "passed",
      driverStderr: session.driverStderr().slice(0, 4000)
    };
    writeReport(session.runDir, report);
    process.exitCode = exitCodeForVerdict(report.verdict);
  } catch (error) {
    logHarnessError(error);
    fs.writeFileSync(path.join(session.runDir, "harness-error.json"), JSON.stringify({
      error: String(error),
      driverStderr: session.driverStderr().slice(0, 8000)
    }, null, 2));
    process.exitCode = 2;
  } finally {
    await session.cleanup();
  }
}

try {
  await main();
} catch (error) {
  if (error instanceof CannotRunError) {
    logCannotRun(error);
    process.exit(3);
  }
  logHarnessError(error);
  process.exit(2);
}
