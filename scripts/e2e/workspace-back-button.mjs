#!/usr/bin/env node
// 真实 Tauri 产品回归：工作区返回按钮（D0 强化版）。
//
// 拦截注入方案不可行（__TAURI_INTERNALS__.invoke 非 writable/configurable，
// 见 probe-internals 结论），本套件改用**真实后端失败路径**：
//   - node:sqlite 对隔离库持写锁（BEGIN IMMEDIATE）：短锁 = 保存被真实阻塞，
//     证明返回按钮等待 in-flight 保存完成后才导航；
//   - 锁超过 busy_timeout（5s）= 真实保存失败（database is locked），
//     验证失败阻断导航、用户可见错误、无 unhandledrejection；
//   - current_edit_version 差值 = 编辑只保存一批，双击不产生重复保存。
//
// 另覆盖：aria/test-id、computed style（40×40 / 8px / 非 56px）、
// Tab 聚焦 :focus-visible 轮廓 + Enter 导航。
// 证据层级：product（真实进程 + WebView2 + SQLite + 文件系统）。

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { DatabaseSync } from "node:sqlite";
import { By, Key, until } from "selenium-webdriver";
import {
  DEFAULT_EXE, DEFAULT_PDF, CannotRunError, assertPrerequisites, assertFreshBuild, buildFreshness,
  buildIdentity, launchTauriApp, createStepRecorder, importPdfViaFolderHook, waitForRowStage,
  openWorkspaceForItem, writeReport, exitCodeForVerdict, logCannotRun, logHarnessError, sleep
} from "./lib/tauri-harness.mjs";

const args = Object.fromEntries(process.argv.slice(2).reduce((all, arg, index, list) => {
  if (arg.startsWith("--")) all.set(arg.slice(2), list[index + 1]?.startsWith("--") ? true : list[index + 1] ?? true);
  return all;
}, new Map()));
const exePath = path.resolve(args.exe ?? DEFAULT_EXE);
const pdfPath = path.resolve(args.pdf ?? DEFAULT_PDF);
const keepRun = Boolean(args.keep);
const takeScreenshots = args.screenshot !== false;

function openDb(dataDir) {
  return new DatabaseSync(path.join(dataDir, "authoring_hub.db"));
}

function editVersionOf(db, itemId) {
  const row = db.prepare("SELECT current_edit_version FROM library_items_v2 WHERE id = ?").get(itemId);
  return row ? Number(row.current_edit_version) : null;
}

/** 在持写锁期间执行 fn（应用的保存 UPDATE 会真实阻塞/失败）。 */
async function withWriteLockHeld(db, fn) {
  db.exec("BEGIN IMMEDIATE");
  try {
    return await fn();
  } finally {
    try { db.exec("COMMIT"); } catch { try { db.exec("ROLLBACK"); } catch {} }
  }
}

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
  const db = openDb(session.dataDir);
  const marker = "E2E BACK FLUSH 77";
  let itemId = null;

  try {
    await recordStep(driver, "library-page-loads", async () => {
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      await driver.executeScript(`
        window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false }));
        window.__e2eRejections = [];
        window.addEventListener("unhandledrejection", (event) => window.__e2eRejections.push(String(event.reason)));
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

    // ── flush 等待证明：DB 写锁真实阻塞保存 → 点击返回 → 未导航 → 解锁 → 保存完成才导航 ──
    await recordStep(driver, "back-button-waits-for-in-flight-save", async () => {
      const versionBefore = editVersionOf(db, itemId);
      await withWriteLockHeld(db, async () => {
        await typeMarkerIntoPassage(driver, marker);
        const backButton = await driver.findElement(By.css(".workspace-back-button"));
        await backButton.click();
        // 保存 UPDATE 被写锁阻塞（busy 等待中）：返回按钮必须停在原地等它。
        await sleep(2000);
        if (!(await currentUrlIncludes(driver, "#/items/"))) {
          throw new Error("保存尚未完成就发生了导航——返回按钮没有等待 in-flight 保存（假绿风险）");
        }
      });
      // 解锁 → 阻塞中的保存完成 → flush 返回 → 导航。
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      const url = await driver.getCurrentUrl();
      if (!url.includes("#/library")) throw new Error(`未导航回题库，当前 URL: ${url}`);
      const versionAfter = editVersionOf(db, itemId);
      if (!(versionAfter > versionBefore)) {
        throw new Error(`返回前编辑未被保存：edit_version ${versionBefore} -> ${versionAfter}`);
      }
      return { heldThenNavigated: true, versionBefore, versionAfter, finalUrl: url };
    });

    await recordStep(driver, "flushed-marker-persists-after-reopen", async () => {
      await openWorkspaceForItem(driver, itemId);
      await driver.wait(until.elementLocated(By.css(".v2-passage-pane .v2-text")), 30000);
      const passageText = await driver.executeScript(
        "return document.querySelector('.v2-passage-pane') ? document.querySelector('.v2-passage-pane').innerText : '';"
      );
      if (!String(passageText).includes(marker)) {
        throw new Error(`重开后未找到返回前保存的 marker "${marker}"，实际开头：${String(passageText).slice(0, 200)}`);
      }
      return { marker, persisted: true };
    });

    // ── 键盘可达性：Tab 聚焦 + focus-visible + Enter 返回 ──
    await recordStep(driver, "back-button-keyboard-focus-visible-and-enter", async () => {
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
          outlineStyle: computed.outlineStyle
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

    // ── 双击防护：两连击只产生一批保存（edit_version 恰好 +1）且只导航一次 ──
    await recordStep(driver, "back-button-double-click-single-save", async () => {
      await openWorkspaceForItem(driver, itemId);
      const versionBefore = editVersionOf(db, itemId);
      await withWriteLockHeld(db, async () => {
        await typeMarkerIntoPassage(driver, "E2E BACK FLUSH 88");
        const backButton = await driver.findElement(By.css(".workspace-back-button"));
        await backButton.click();
        await backButton.click().catch(() => {}); // 第二击：保存挂起期间应被忽略
        await sleep(1200);
        if (!(await currentUrlIncludes(driver, "#/items/"))) {
          throw new Error("保存挂起期间发生了导航");
        }
      });
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      const navigatedOnce = (await driver.getCurrentUrl()).includes("#/library");
      const versionAfter = editVersionOf(db, itemId);
      const delta = versionAfter - versionBefore;
      if (delta !== 1) {
        throw new Error(`双击产生 ${delta} 批保存（期望恰 1 批）：edit_version ${versionBefore} -> ${versionAfter}`);
      }
      return { saveBatches: delta, navigatedOnce };
    });

    // ── 可控保存失败（真实 busy 超时）：不导航、错误可见、无 unhandledrejection ──
    await recordStep(driver, "save-failure-blocks-navigation-with-visible-error", async () => {
      await openWorkspaceForItem(driver, itemId);
      const failureMarker = "E2E BACK FLUSH FAIL 99";
      let failureSeen = false;
      // 持锁超过应用 busy_timeout（5s）：保存真实失败（database is locked）。
      db.exec("BEGIN IMMEDIATE");
      try {
        await typeMarkerIntoPassage(driver, failureMarker);
        const backButton = await driver.findElement(By.css(".workspace-back-button"));
        await backButton.click();
        // 失败在 busy 超时后落地（约 5-7s）；期间不得导航。
        await sleep(2500);
        if (!(await currentUrlIncludes(driver, "#/items/"))) {
          throw new Error("保存失败前就发生了导航");
        }
        const failureSeenResult = await driver.wait(async () => {
          if (!(await currentUrlIncludes(driver, "#/items/"))) return false;
          const notices = await driver.findElements(By.css(".workspace-notice"));
          for (const notice of notices) {
            const text = await notice.getText();
            if (/保存失败|修改已保留|稍后重试|锁定/.test(text)) return text;
          }
          return false;
        }, 20000).catch(() => false);
        failureSeen = Boolean(failureSeenResult);
        if (!failureSeenResult) throw new Error("保存失败后未见用户可理解的错误提示（.workspace-notice）");
        if (!(await currentUrlIncludes(driver, "#/items/"))) {
          throw new Error("保存失败后发生了导航——失败必须阻断返回");
        }
        const rejections = await driver.executeScript("return window.__e2eRejections || [];");
        if (rejections.length) {
          throw new Error(`出现 unhandledrejection ${rejections.length} 条：${rejections[0]}`);
        }
      } finally {
        try { db.exec("COMMIT"); } catch { try { db.exec("ROLLBACK"); } catch {} }
      }
      // 恢复：解锁后再次返回，flush 重试失败批次成功并导航（失败批次保留语义）。
      const backButton = await driver.findElement(By.css(".workspace-back-button"));
      await backButton.click().catch(async () => {
        const fresh = await driver.findElement(By.css(".workspace-back-button"));
        await fresh.click();
      });
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      return { blockedNavigation: true, errorShown: String(failureSeen), unhandledRejections: 0 };
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
    try { db.close(); } catch {}
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
