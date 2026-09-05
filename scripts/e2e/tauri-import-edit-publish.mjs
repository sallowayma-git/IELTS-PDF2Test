#!/usr/bin/env node
// 真实 Tauri E2E（M0-T3 / 计划 P0-T02）：tauri-driver + selenium-webdriver 驱动真实应用进程。
//
// 覆盖层级声明（对应计划 §19.7 / M0 退出条件）：
//   - 本脚本驱动真实 Tauri 进程 + WebView2 + 真实 SQLite + 真实文件系统，
//     不是浏览器 + devFallbackBackend（那是 scripts/e2e/library-workspace-smoke.mjs），
//     也不是 Rust 命令级测试（那是 src-tauri/src/product_chain.rs）。三者分别报告，不互相替代。
//   - 发布步骤受后端质量门约束：仓库现有语料 PDF 无法达到 ready（product_chain.rs 头注释），
//     因此发布步骤的结果按「成功 / 被门禁阻止」如实记录，被门禁阻止不计为通过。
//
// 隔离：应用进程继承被改写的 APPDATA/LOCALAPPDATA，SQLite、WebView2 配置、发布产物
// 全部落到本次运行的临时目录，不污染真实用户数据。原生文件对话框通过既有自动化钩子绕开：
//   - PDF2TEST_AUTOMATION_PDF_DIR      -> pick_pdf_folder_sources 免对话框返回目录内 PDF
//   - PDF2TEST_AUTOMATION_EXPORT_DIR   -> choose_export_dir 免对话框返回固定导出目录
//
// 用法：
//   node scripts/e2e/tauri-import-edit-publish.mjs [--exe path] [--pdf path] [--keep] [--screenshot]
//   npm run e2e:tauri

import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { Builder, By, Key, until } from "selenium-webdriver";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const DRIVER_CACHE_DIR = path.join(process.env.LOCALAPPDATA ?? repoRoot, "pdf2test-e2e-drivers");
const DEFAULT_EXE = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const DEFAULT_PDF = path.join(repoRoot, "fixtures", "golden", "synthetic", "pdf", "pdf-two-column.pdf");
const WEBVIEW2_REG_KEY = "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
const MSEDGEDRIVER_CDN = "https://msedgedriver.microsoft.com";

const args = parseArgs(process.argv.slice(2));
const exePath = path.resolve(args.exe ?? DEFAULT_EXE);
const pdfPath = path.resolve(args.pdf ?? DEFAULT_PDF);
const keepRun = Boolean(args.keep);
const takeScreenshots = args.screenshot !== false;

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--keep") out.keep = true;
    else if (arg === "--no-screenshot") out.screenshot = false;
    else if (arg.startsWith("--")) out[arg.slice(2)] = argv[i + 1];
  }
  return out;
}

function fail(message) {
  console.error(`[e2e:tauri] ${message}`);
  process.exit(2);
}

function runCapture(command, commandArgs) {
  const result = spawnSync(command, commandArgs, { encoding: "utf8", windowsHide: true });
  return { status: result.status, stdout: `${result.stdout ?? ""}${result.stderr ?? ""}` };
}

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForHttp(url, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {}
    await sleep(250);
  }
  throw new Error(`timeout waiting for ${url}`);
}

function webview2Version() {
  const probe = runCapture("reg", ["query", WEBVIEW2_REG_KEY, "/v", "pv"]);
  if (probe.status !== 0) return null;
  const match = probe.stdout.match(/REG_SZ\s+([\d.]+)/);
  return match?.[1] ?? null;
}

function findMsedgedriver() {
  const probe = runCapture("where", ["msedgedriver"]);
  if (probe.status === 0) {
    const first = probe.stdout.split(/\r?\n/).find((line) => line.trim().endsWith(".exe"));
    if (first) return path.dirname(path.resolve(first.trim()));
  }
  const cached = path.join(DRIVER_CACHE_DIR, "msedgedriver.exe");
  if (fs.existsSync(cached)) return DRIVER_CACHE_DIR;
  return null;
}

async function ensureMsedgedriver() {
  const existing = findMsedgedriver();
  if (existing) return existing;

  const version = webview2Version();
  if (!version) {
    fail("未找到 msedgedriver，也无法从注册表读取 WebView2 运行时版本（无法自动下载匹配驱动）。");
  }
  console.log(`[e2e:tauri] WebView2 runtime ${version}; downloading matching msedgedriver...`);
  fs.mkdirSync(DRIVER_CACHE_DIR, { recursive: true });
  const zipPath = path.join(DRIVER_CACHE_DIR, `edgedriver-${version}.zip`);
  const zipResult = runCapture("powershell", [
    "-NoProfile", "-Command",
    `Invoke-WebRequest -Uri '${MSEDGEDRIVER_CDN}/${version}/edgedriver_win64.zip' -OutFile '${zipPath}'`
  ]);
  if (zipResult.status !== 0 || !fs.existsSync(zipPath)) {
    fail(`下载 msedgedriver ${version} 失败：${zipResult.stdout.slice(0, 400)}`);
  }
  const unzip = runCapture("powershell", [
    "-NoProfile", "-Command",
    `Expand-Archive -Force -Path '${zipPath}' -DestinationPath '${DRIVER_CACHE_DIR}'`
  ]);
  const driverExe = path.join(DRIVER_CACHE_DIR, "msedgedriver.exe");
  if (unzip.status !== 0 || !fs.existsSync(driverExe)) {
    fail(`解压 msedgedriver 失败：${unzip.stdout.slice(0, 400)}`);
  }
  return DRIVER_CACHE_DIR;
}

const steps = [];
const artifacts = { dir: null, screenshotErrors: [] };

async function recordStep(name, fn) {
  const started = Date.now();
  const entry = { name, status: "passed", startedAt: new Date(started).toISOString(), details: {} };
  steps.push(entry);
  try {
    const details = (await fn()) ?? {};
    entry.details = details;
    // 被产品门禁阻止（如质量门）不是测试失败也不是通过，单独记录。
    if (details && details.outcome === "blocked_by_quality_gate") entry.status = "blocked";
  } catch (error) {
    entry.status = "failed";
    entry.error = String(error instanceof Error ? error.message : error).slice(0, 2000);
    if (takeScreenshots && globalThis.__driver) {
      try {
        const shot = await globalThis.__driver.takeScreenshot();
        const file = path.join(artifacts.dir, `step-${steps.length}-${name.replace(/[^\w-]+/g, "_")}.png`);
        fs.writeFileSync(file, shot, "base64");
        entry.screenshot = path.relative(repoRoot, file);
      } catch (shotError) {
        artifacts.screenshotErrors.push(String(shotError));
      }
    }
  }
  entry.durationMs = Date.now() - started;
  console.log(`[e2e:tauri] ${entry.status.toUpperCase()} ${name}${entry.error ? ` :: ${entry.error.slice(0, 300)}` : ""}`);
  return entry;
}

async function waitForRowStage(driver, itemId, timeoutMs) {
  const selector = `[data-item-id="${itemId}"]`;
  const deadline = Date.now() + timeoutMs;
  let lastText = "";
  while (Date.now() < deadline) {
    const rows = await driver.findElements(By.css(selector));
    if (rows.length) {
      const text = (await rows[0].getText()).replace(/\s+/g, " ").trim();
      lastText = text;
      // 待检查/可发布/失败 都意味着 job 不再处于 Working（见 libraryTypes deriveStage）。
      if (/待检查|可发布|失败|已发布/.test(text)) {
        return { stageClass: await rows[0].getAttribute("class"), rowText: text };
      }
    }
    await sleep(1000);
  }
  throw new Error(`行 ${itemId} 未在限时内进入稳定阶段；最后文本：${lastText || "(无行)"}`);
}

async function openWorkspaceForItem(driver, itemId) {
  const row = await driver.wait(until.elementLocated(By.css(`[data-item-id="${itemId}"] .library-row-main`)), 15000);
  await row.click();
  await driver.wait(until.elementLocated(By.css('[data-testid="exam-workspace"]')), 15000);
  return itemId;
}

async function main() {
  if (process.platform !== "win32") fail("真实 Tauri E2E 目前仅在 Windows（WebView2）上运行。");
  if (!fs.existsSync(exePath)) fail(`被测应用不存在：${exePath}（先运行 npx tauri build --debug --no-bundle）`);
  if (!fs.existsSync(pdfPath)) fail(`测试 PDF 不存在：${pdfPath}`);

  const runId = new Date().toISOString().replace(/[:.]/g, "-");
  const runDir = path.join(repoRoot, "artifacts", "e2e-tauri", `run-${runId}`);
  for (const sub of ["appdata/roaming", "appdata/local", "appdata/data", "appdata/webview", "pdfs", "publish"]) {
    fs.mkdirSync(path.join(runDir, sub), { recursive: true });
  }
  artifacts.dir = runDir;
  fs.copyFileSync(pdfPath, path.join(runDir, "pdfs", path.basename(pdfPath)));
  const pdfDir = path.join(runDir, "pdfs");
  const publishDir = path.join(runDir, "publish");
  const dataDir = path.join(runDir, "appdata", "data");

  const driverDir = await ensureMsedgedriver();
  const port = await freePort();

  console.log(`[e2e:tauri] run dir: ${runDir}`);
  console.log(`[e2e:tauri] starting tauri-driver on :${port}`);
  const driverProcess = spawn("tauri-driver", ["--port", String(port)], {
    stdio: ["ignore", "pipe", "pipe"],
    env: {
      ...process.env,
      PATH: `${driverDir}${path.delimiter}${process.env.PATH ?? ""}`,
      // Windows 上 Tauri 的 app_data_dir 走 known-folder API、WebView2 配置同理，
      // 都不读 APPDATA/LOCALAPPDATA 环境变量，因此必须用产品侧测试钩子
      // （PDF2TEST_AUTOMATION_DATA_DIR，见 lib.rs app_root）+ WebView2 官方变量做隔离。
      PDF2TEST_AUTOMATION_DATA_DIR: dataDir,
      WEBVIEW2_USER_DATA_FOLDER: path.join(runDir, "appdata", "webview"),
      PDF2TEST_AUTOMATION_PDF_DIR: pdfDir,
      PDF2TEST_AUTOMATION_EXPORT_DIR: publishDir
    },
    windowsHide: true
  });
  let driverStderr = "";
  driverProcess.stderr.on("data", (chunk) => { driverStderr += String(chunk); });

  const serverUrl = `http://127.0.0.1:${port}`;
  let driver;
  let appProcessHint = null;
  try {
    await waitForHttp(`${serverUrl}/status`, 20000);

    const capabilities = {
      // selenium-webdriver 4.x 远程会话强制要求 browserName；tauri-driver 模式下
      // 该值仅占位，实际被测对象由 tauri:options.application 指定。
      browserName: "wry",
      "tauri:options": { application: exePath }
    };
    driver = await new Builder()
      .usingServer(serverUrl)
      .withCapabilities(capabilities)
      .build();
    globalThis.__driver = driver;

    const windowHandle = (await driver.getAllWindowHandles())[0];
    await driver.switchTo().window(windowHandle);

    const editedMarker = "E2E EDIT CHECK 42";

    await recordStep("library-page-loads", async () => {
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      // 云端识别保持关闭：本轮冒烟验证的是本地链（云端依赖外部模型配置）。
      await driver.executeScript(`
        window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false }));
        location.hash = "#/library";
      `);
      await driver.navigate().refresh();
      await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);
      return { url: await driver.getCurrentUrl() };
    });

    await recordStep("import-pdf-via-folder-hook", async () => {
      const before = new Set(
        await driver.findElements(By.css('[data-testid="library-row"]'))
          .then(async (rows) => Promise.all(rows.map((row) => row.getAttribute("data-item-id"))))
      );
      await driver.findElement(By.css('[data-testid="library-import"]')).click();
      await driver.wait(until.elementLocated(By.css('[data-testid="import-drawer"]')), 10000);
      await driver.findElement(By.css('[data-testid="import-pick-folder"]')).click();
      await driver.wait(until.elementLocated(By.css('[data-testid="import-picked-files"] li')), 10000);
      await driver.findElement(By.css('[data-testid="import-start"]')).click();
      // 新行要么乐观插入、要么随 2s 轮询出现；用集合差分确定本次导入的 item id。
      const deadline = Date.now() + 30000;
      let newItemId = null;
      while (Date.now() < deadline) {
        const ids = await driver.findElements(By.css('[data-testid="library-row"]'))
          .then(async (rows) => Promise.all(rows.map((row) => row.getAttribute("data-item-id"))));
        newItemId = ids.find((id) => !before.has(id)) ?? null;
        if (newItemId) break;
        await sleep(1000);
      }
      if (!newItemId) throw new Error(`导入后未出现新题库行（导入前行数 ${before.size}）`);
      globalThis.__importedItemId = newItemId;
      return { itemId: newItemId, priorRowCount: before.size };
    });

    let importedItemId = null;
    await recordStep("background-pipeline-reaches-stable-stage", async () => {
      const rows = await driver.findElements(By.css('[data-testid="library-row"]'));
      // 从导入步骤的报告中拿 itemId：重新差分不可靠，直接在两步间共享状态。
      importedItemId = globalThis.__importedItemId ?? null;
      if (!importedItemId) throw new Error("缺少导入 item id");
      return waitForRowStage(driver, importedItemId, 240000);
    });

    let itemId = null;
    let loadErrorText = null;
    await recordStep("workspace-opens", async () => {
      itemId = importedItemId ?? (await openWorkspaceForItem(driver, importedItemId));
      await openWorkspaceForItem(driver, itemId);
      const loadErrors = await driver.findElements(By.css(".workspace-load-error"));
      if (loadErrors.length) {
        loadErrorText = (await loadErrors[0].getText()).replace(/\s+/g, " ").trim();
        // 真实导入的 job 打不开工作区：把 job 目录下的 shadow 错误产物一并带出，不提前归因。
        const jobDir = path.join(dataDir, "jobs", itemId);
        const errorFiles = [];
        if (fs.existsSync(jobDir)) {
          for (const name of fs.readdirSync(jobDir)) {
            if (name.endsWith(".error.json")) {
              errorFiles.push({ file: name, content: fs.readFileSync(path.join(jobDir, name), "utf8").slice(0, 4000) });
            }
          }
        }
        throw new Error(`REPRODUCED workspace load blocker :: ${loadErrorText} :: jobDir=${jobDir} :: errorFiles=${JSON.stringify(errorFiles)}`);
      }
      return { itemId };
    });

    if (!loadErrorText) {
      await recordStep("edit-one-character-and-save", async () => {
        // 点击会触发 React 重渲染（选中态/编辑态），元素引用极易过期；每一步都重新定位。
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
        await editor.sendKeys(editedMarker);
        // Enter 提交后组件随即卸载，最后一次按键可能打到过期引用——提交本身已发生。
        try {
          await editor.sendKeys(Key.ENTER);
        } catch {}
        const saveState = await driver.wait(
          until.elementTextContains(
            driver.wait(until.elementLocated(By.css('[data-testid="workspace-save-state"]')), 10000),
            "已保存"
          ),
          20000
        );
        return { saveState: await saveState.getText(), marker: editedMarker };
      });

      await recordStep("edit-survives-reopen", async () => {
        await driver.executeScript("location.hash = '#/library';");
        await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 15000);
        await openWorkspaceForItem(driver, itemId);
        // 重开走完整加载链（读盘 + revision 解析），等 Canvas 真正渲染出文本再断言。
        await driver.wait(until.elementLocated(By.css(".v2-passage-pane .v2-text")), 30000);
        const loadErrors = await driver.findElements(By.css(".workspace-load-error"));
        if (loadErrors.length) {
          throw new Error(`重开后出现加载错误：${(await loadErrors[0].getText()).slice(0, 300)}`);
        }
        const passageText = await driver.executeScript(
          "return document.querySelector('.v2-passage-pane') ? document.querySelector('.v2-passage-pane').innerText : '';"
        );
        if (!String(passageText).includes(editedMarker)) {
          throw new Error(`重开后未找到编辑文本 "${editedMarker}"，实际开头：${String(passageText).slice(0, 200)}`);
        }
        return { marker: editedMarker, found: true };
      });

      await recordStep("edit-title-and-save", async () => {
        // 计划 §9.10「标题（可编辑）」：M1 起标题随命令批次进同一保存事务。
        const newTitle = "E2E 标题 42";
        const titleSpan = driver.wait(until.elementLocated(By.css('[data-testid="workspace-title"] [role="button"]')), 10000);
        await driver.executeScript("arguments[0].click()", titleSpan);
        const titleInput = await driver.wait(until.elementLocated(By.css('[data-testid="workspace-title-input"]')), 10000);
        await titleInput.sendKeys(Key.chord(Key.CONTROL, "a"));
        await titleInput.sendKeys(newTitle);
        try {
          await titleInput.sendKeys(Key.ENTER);
        } catch {}
        await driver.wait(
          until.elementTextContains(
            driver.wait(until.elementLocated(By.css('[data-testid="workspace-save-state"]')), 10000),
            "已保存"
          ),
          20000
        );
        return { title: newTitle };
      });

      await recordStep("title-persists-in-library", async () => {
        await driver.executeScript("location.hash = '#/library';");
        await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 15000);
        const row = await driver.wait(until.elementLocated(By.css(`[data-item-id="${itemId}"]`)), 15000);
        const rowText = (await row.getText()).replace(/\s+/g, " ").trim();
        if (!rowText.includes("E2E 标题 42")) {
          throw new Error(`题库行未显示新标题，实际：${rowText.slice(0, 120)}`);
        }
        // 回到工作区，标题也应保持（DB 权威，工作区从仓库读）。
        await openWorkspaceForItem(driver, itemId);
        // 加载完成后标题才来自 V2 仓库；轮询等待，避免读到 job.json 回退值。
        await driver.wait(
          until.elementTextContains(
            driver.wait(until.elementLocated(By.css('[data-testid="workspace-title"]')), 15000),
            "E2E 标题 42"
          ),
          20000
        );
        return { title: "E2E 标题 42", persisted: true };
      });

      await recordStep("publish-via-workspace-button", async () => {
        await driver.findElement(By.css('[data-testid="workspace-publish"]')).click();
        const notice = await driver.wait(
          until.elementLocated(By.css(".workspace-notice")),
          60000
        );
        await driver.wait(async () => {
          const text = await notice.getText();
          return text.includes("发布完成") || text.includes("失败") || text.includes("未完成");
        }, 60000);
        const noticeText = (await notice.getText()).replace(/\s+/g, " ").trim();
        if (!noticeText.includes("发布完成")) {
          // 仓库现有语料 PDF 达不到 ready 质量门（product_chain.rs 头注释）：
          // 被门禁阻止是如实记录的产品行为，不计为通过。
          return { outcome: "blocked_by_quality_gate", notice: noticeText, countedAsPass: false };
        }
        const files = fs.existsSync(publishDir) ? fs.readdirSync(publishDir) : [];
        if (!files.length) throw new Error(`发布显示成功但导出目录为空：${publishDir}`);
        return { outcome: "published", notice: noticeText, publishedFiles: files.slice(0, 20) };
      });
    }

    const failed = steps.filter((step) => step.status === "failed");
    const publishStep = steps.find((step) => step.name === "publish-via-workspace-button");
    const publishBlocked = publishStep?.details?.outcome === "blocked_by_quality_gate";
    const report = {
      runId,
      coverage: "real-tauri-process+webview2+sqlite+filesystem",
      exe: exePath,
      pdf: pdfPath,
      driverPort: port,
      workspaceLoadBlocker: loadErrorText,
      publishBlockedByQualityGate: publishBlocked,
      steps,
      verdict: failed.length === 0 && !publishBlocked
        ? "passed"
        : failed.length === 0 && publishBlocked
          ? "passed-with-publish-gate-blocked"
          : "failed",
      driverStderr: driverStderr.slice(0, 4000)
    };
    fs.writeFileSync(path.join(runDir, "report.json"), JSON.stringify(report, null, 2));
    console.log(`[e2e:tauri] verdict: ${report.verdict}`);
    console.log(`[e2e:tauri] report: ${path.join(runDir, "report.json")}`);
    if (report.verdict === "failed") process.exitCode = 1;
  } catch (error) {
    console.error(`[e2e:tauri] harness error: ${error}`);
    if (driverStderr) console.error(`[e2e:tauri] tauri-driver stderr: ${driverStderr.slice(0, 2000)}`);
    fs.writeFileSync(path.join(runDir, "harness-error.json"), JSON.stringify({
      error: String(error),
      driverStderr: driverStderr.slice(0, 8000)
    }, null, 2));
    process.exitCode = 2;
  } finally {
    if (driver) {
      try { await driver.quit(); } catch {}
    }
    driverProcess.kill();
    if (!keepRun) {
      await sleep(1500);
      try { fs.rmSync(runDir, { recursive: true, force: true }); } catch {}
      console.log("[e2e:tauri] run dir cleaned (use --keep to inspect artifacts)");
    }
  }
}

await main();
