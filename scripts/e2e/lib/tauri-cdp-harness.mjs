#!/usr/bin/env node
/**
 * 真实 Tauri 应用自动化通道（WebView2 CDP）。
 *
 * 背景：本环境下 `tauri-driver` + `msedgedriver` 无法为被测 exe 建立稳定会话
 * （`session not created / unable to connect to renderer`、`chrome not reachable`），
 * 而应用本体可正常启动并渲染。因此改为直接使用 WebView2 自带的 DevTools 协议：
 *
 *   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=<port> --remote-allow-origins=*"
 *
 * 这条通道驱动的是**真实 exe**：真实前端（embedded dist）、真实 Rust 后端、
 * 真实 SQLite 与真实文件系统。IPC 走的是 `window.__TAURI_INTERNALS__.invoke`，
 * 与产品运行时同一条路径，不是 mock、不是浏览器替身。
 *
 * ⚠️ 参数标注：`--remote-debugging-port` 会打开 WebView2 的调试端点（仅监听 127.0.0.1），
 * 属于**诊断/自动化参数**，不在产品默认启动配置内。所有以此通道产生的结论都必须
 * 标注为「CDP 自动化通道」，不得写成「默认产品路径通过」。
 * 该通道**不需要** `--no-sandbox` / `--disable-gpu` 等放宽安全或运行时行为的参数。
 */

import { execFileSync, spawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import WebSocket from "ws";

export const CDP_CHANNEL_LABEL = "webview2-cdp";
export const CDP_CHANNEL_NOTE =
  "自动化参数 --remote-debugging-port（仅 127.0.0.1），属诊断参数，非产品默认启动配置";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

export class CannotRunError extends Error {
  constructor(message) {
    super(message);
    this.name = "CannotRunError";
  }
}

export function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

export function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** 构建新鲜度检查：源码比 exe 新 => staleBuild，判定为 CANNOT-RUN，不算通过。
 *
 * `tolerateConcurrentEdits`：本仓库有两个 agent 并行写入（识别/云端后端 agent 独占
 * `src-tauri/src/{processing,recognition,reconcile,llm_*}/**`）。当对方在本次构建之后
 * 继续落盘时，exe 相对**最新**源码永远是「陈旧」的，但这不代表本次运行的二进制不是
 * 从被验收的源码构建出来的。开启该选项时，函数不抛错，而是把这些**构建之后**才出现的
 * 文件原样列出来，由调用方写进报告，明确标注为「并发外部改动，不在本次验收范围」。
 */
export function assertBuildFresh({ exePath, repoRoot: root = repoRoot, tolerateConcurrentEdits = false }) {
  const walk = (p, out) => {
    if (!fs.existsSync(p)) return;
    const st = fs.statSync(p);
    if (st.isDirectory()) {
      for (const entry of fs.readdirSync(p)) walk(path.join(p, entry), out);
    } else {
      out.push({ path: p, mtimeMs: st.mtimeMs });
    }
  };
  const files = [];
  for (const d of ["src", "src-tauri/src"]) walk(path.join(root, d), files);
  for (const f of [
    "src-tauri/Cargo.toml",
    "src-tauri/Cargo.lock",
    "src-tauri/tauri.conf.json",
    "package.json",
    "package-lock.json",
  ]) {
    walk(path.join(root, f), files);
  }
  const exeMs = fs.statSync(exePath).mtimeMs;
  const newer = files.filter((f) => f.mtimeMs > exeMs).sort((a, b) => b.mtimeMs - a.mtimeMs);
  if (newer.length && !tolerateConcurrentEdits) {
    throw new CannotRunError(
      `staleBuild: 源码比 exe 新（${newer[0].path} @ ${new Date(newer[0].mtimeMs).toISOString()} > exe @ ${new Date(exeMs).toISOString()}）。请先重新构建再验收。`
    );
  }
  const newest = files.reduce((acc, f) => (f.mtimeMs > acc.mtimeMs ? f : acc), { path: null, mtimeMs: 0 });
  return {
    exeMs,
    srcNewestMs: newest.mtimeMs,
    srcNewestPath: newest.path,
    tolerated: tolerateConcurrentEdits
      ? newer.map((f) => ({
          path: path.relative(root, f.path).replace(/\\/g, "/"),
          mtime: new Date(f.mtimeMs).toISOString(),
        }))
      : [],
  };
}

class CdpConnection {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 0;
    this.pending = new Map();
    this.events = [];
    this.closed = false;
    ws.on("message", (raw) => {
      let msg;
      try {
        msg = JSON.parse(String(raw));
      } catch {
        return;
      }
      if (msg.id && this.pending.has(msg.id)) {
        const entry = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        clearTimeout(entry.timer);
        if (msg.error) entry.reject(new Error(`${msg.error.message} :: ${JSON.stringify(msg.error)}`));
        else entry.resolve(msg.result);
      } else if (msg.method) {
        this.events.push(msg);
        if (this.events.length > 500) this.events.shift();
      }
    });
    ws.on("close", () => {
      this.closed = true;
      for (const [, entry] of this.pending) {
        clearTimeout(entry.timer);
        entry.reject(new Error("CDP 连接已关闭（renderer 或应用退出）"));
      }
      this.pending.clear();
    });
  }

  send(method, params = {}, timeoutMs = 60000) {
    if (this.closed) return Promise.reject(new Error(`CDP 连接已关闭，无法发送 ${method}`));
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP 超时：${method}（${timeoutMs}ms）`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  close() {
    try {
      this.ws.close();
    } catch {}
  }
}

/**
 * 启动真实 exe 并建立 CDP 会话。
 * @param {object} opts
 * @param {string} opts.exePath
 * @param {string} opts.runDir  运行目录（appdata / pdfs / exports / nas-library 都放这里）
 * @param {string} [opts.extraBrowserArgs] 额外的浏览器参数（会被记录进报告）
 * @param {object} [opts.appEnv]
 * @param {number} [opts.pageReadyTimeoutMs]
 */
export async function launchTauriAppCdp({
  exePath,
  runDir,
  extraBrowserArgs = "",
  appEnv = {},
  pageReadyTimeoutMs = 90000,
  quiet = false,
}) {
  const log = (...args) => {
    if (!quiet) console.log("[cdp]", ...args);
  };
  if (!fs.existsSync(exePath)) throw new CannotRunError(`exe 不存在：${exePath}`);

  for (const sub of ["appdata/data", "appdata/webview", "pdfs", "exports", "nas-library"]) {
    fs.mkdirSync(path.join(runDir, sub), { recursive: true });
  }

  const devtoolsPort = await freePort();
  const browserArgs = [
    `--remote-debugging-port=${devtoolsPort}`,
    "--remote-allow-origins=*",
    extraBrowserArgs,
  ]
    .filter(Boolean)
    .join(" ");

  const env = {
    ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: browserArgs,
    PDF2TEST_AUTOMATION_DATA_DIR: path.join(runDir, "appdata", "data"),
    WEBVIEW2_USER_DATA_FOLDER: path.join(runDir, "appdata", "webview"),
    PDF2TEST_AUTOMATION_PDF_DIR: path.join(runDir, "pdfs"),
    PDF2TEST_AUTOMATION_EXPORT_DIR: path.join(runDir, "exports"),
    ...appEnv,
  };

  log(`run dir: ${runDir}`);
  log(`devtools port: ${devtoolsPort}`);

  const child = spawn(exePath, [], { stdio: ["ignore", "pipe", "pipe"], env, windowsHide: true });
  let appOutput = "";
  child.stdout.on("data", (c) => { appOutput += String(c); });
  child.stderr.on("data", (c) => { appOutput += String(c); });
  let exitCode = null;
  child.on("exit", (code) => { exitCode = code; });

  const startedAt = Date.now();
  let ws = null;
  let connection = null;
  try {
    // 1) 等 DevTools HTTP 端点
    let endpointReady = false;
    while (Date.now() - startedAt < pageReadyTimeoutMs) {
      if (exitCode !== null) throw new CannotRunError(`应用进程在 CDP 就绪前退出（code=${exitCode}）`);
      try {
        const res = await fetch(`http://127.0.0.1:${devtoolsPort}/json/version`);
        if (res.ok) { endpointReady = true; break; }
      } catch {}
      await sleep(250);
    }
    if (!endpointReady) throw new CannotRunError(`WebView2 DevTools 端点未在 ${pageReadyTimeoutMs}ms 内就绪`);

    // 2) 等 page target
    let target = null;
    while (Date.now() - startedAt < pageReadyTimeoutMs) {
      if (exitCode !== null) throw new CannotRunError(`应用进程在 page target 出现前退出（code=${exitCode}）`);
      try {
        const res = await fetch(`http://127.0.0.1:${devtoolsPort}/json/list`);
        if (res.ok) {
          const list = await res.json();
          target = list.find((t) => t.type === "page" && t.webSocketDebuggerUrl) ?? null;
          if (target) break;
        }
      } catch {}
      await sleep(250);
    }
    if (!target) throw new CannotRunError(`未在 ${pageReadyTimeoutMs}ms 内发现 page target`);

    // 3) 建 WS
    ws = new WebSocket(target.webSocketDebuggerUrl, {
      perMessageDeflate: false,
      maxPayload: 256 * 1024 * 1024,
    });
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new CannotRunError("CDP WebSocket 连接超时")), 20000);
      ws.once("open", () => { clearTimeout(timer); resolve(); });
      ws.once("error", (e) => { clearTimeout(timer); reject(e); });
    });
    connection = new CdpConnection(ws);
    await connection.send("Runtime.enable", {}, 30000);
    await connection.send("Page.enable", {}, 30000);

    const session = new TauriCdpSession({
      connection,
      child,
      devtoolsPort,
      runDir,
      browserArgs,
      appOutput: () => appOutput,
      exitCode: () => exitCode,
      log,
    });

    // 4) 等真实前端挂载（body 有可见文本）
    await session.waitForPageReady(pageReadyTimeoutMs);
    return session;
  } catch (error) {
    try { connection?.close(); } catch {}
    try { ws?.close(); } catch {}
    try { child.kill(); } catch {}
    await sleep(500);
    const err = error instanceof CannotRunError ? error : new CannotRunError(String(error?.message ?? error));
    err.appOutput = appOutput;
    err.runDir = runDir;
    throw err;
  }
}

export class TauriCdpSession {
  constructor({ connection, child, devtoolsPort, runDir, browserArgs, appOutput, exitCode, log }) {
    this.cdp = connection;
    this.child = child;
    this.devtoolsPort = devtoolsPort;
    this.runDir = runDir;
    this.browserArgs = browserArgs;
    this.appOutput = appOutput;
    this.exitCode = exitCode;
    this.log = log;
    this.screenshotErrors = [];
    this.pageErrors = [];
    this.consoleErrors = [];
  }

  /** 底层求值：异常会抛出，返回 by-value。 */
  async evaluate(expression, { timeoutMs = 60000, awaitPromise = true } = {}) {
    const r = await this.cdp.send(
      "Runtime.evaluate",
      { expression, returnByValue: true, awaitPromise },
      timeoutMs
    );
    if (r.exceptionDetails) {
      const d = r.exceptionDetails.exception?.description ?? r.exceptionDetails.text ?? "unknown";
      throw new Error(`页面脚本异常：${String(d).slice(0, 500)}`);
    }
    return r.result?.value;
  }

  /** 带重试的求值（renderer 冷启动阶段偶发不响应）。 */
  async evaluateRetry(expression, { attempts = 6, delayMs = 1200, timeoutMs = 30000 } = {}) {
    let last;
    for (let i = 0; i < attempts; i += 1) {
      try {
        return await this.evaluate(expression, { timeoutMs });
      } catch (error) {
        last = error;
        await sleep(delayMs);
      }
    }
    throw last;
  }

  async waitForPageReady(timeoutMs = 90000) {
    const deadline = Date.now() + timeoutMs;
    let lastText = "";
    while (Date.now() < deadline) {
      if (this.exitCode() !== null) throw new CannotRunError(`应用在页面就绪前退出（code=${this.exitCode()}）`);
      try {
        const text = await this.evaluate("document.body ? document.body.innerText : ''", { timeoutMs: 15000 });
        lastText = String(text ?? "");
        if (lastText.trim().length > 0) return lastText;
      } catch {}
      await sleep(500);
    }
    throw new CannotRunError(`页面在 ${timeoutMs}ms 内未渲染出可见文本（最后文本=${JSON.stringify(lastText.slice(0, 120))}）`);
  }

  /** 轮询直到表达式返回真值。 */
  async waitFor(expression, { timeoutMs = 30000, intervalMs = 300, label = expression } = {}) {
    const deadline = Date.now() + timeoutMs;
    let last;
    while (Date.now() < deadline) {
      try {
        last = await this.evaluate(expression, { timeoutMs: 15000 });
        if (last) return last;
      } catch (error) {
        last = `ERR:${error.message}`;
      }
      await sleep(intervalMs);
    }
    throw new Error(`等待条件超时（${label}），最后一次结果=${JSON.stringify(last)}`);
  }

  /** 通过 DOM 查询拿到元素中心点，再用真实鼠标事件点击。 */
  async clickByText(text, { exact = true, nth = 0, timeoutMs = 20000 } = {}) {
    const expr = `(() => {
      const t = ${JSON.stringify(text)};
      const all = [...document.querySelectorAll('*')].filter(n => {
        if (n.children.length !== 0) return false;
        const s = (n.textContent || '').trim();
        return ${exact ? "s === t" : "s.includes(t)"};
      });
      const el = all[${nth}];
      if (!el) return null;
      el.scrollIntoView({ block: 'center', inline: 'center' });
      const r = el.getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2, w: r.width, h: r.height, tag: el.tagName };
    })()`;
    const box = await this.waitFor(expr, { timeoutMs, label: `clickByText(${text})` });
    if (box.w === 0 || box.h === 0) throw new Error(`目标元素尺寸为 0，无法点击：${text}`);
    await this.clickAt(box.x, box.y);
    return box;
  }

  async clickSelector(selector, { timeoutMs = 20000 } = {}) {
    const expr = `(() => {
      const el = document.querySelector(${JSON.stringify(selector)});
      if (!el) return null;
      el.scrollIntoView({ block: 'center', inline: 'center' });
      const r = el.getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2, w: r.width, h: r.height, tag: el.tagName };
    })()`;
    const box = await this.waitFor(expr, { timeoutMs, label: `clickSelector(${selector})` });
    if (box.w === 0 || box.h === 0) throw new Error(`目标元素尺寸为 0，无法点击：${selector}`);
    await this.clickAt(box.x, box.y);
    return box;
  }

  async clickAt(x, y) {
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "none", clickCount: 0 });
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
  }

  /** 真实键盘输入：先聚焦元素，再用 Input.insertText。 */
  async typeInto(selector, text) {
    await this.clickSelector(selector);
    await this.evaluate(`(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (el && el.select) el.select(); return true; })()`);
    await this.cdp.send("Input.insertText", { text });
    return true;
  }

  async pressKey(key, { code, windowsVirtualKeyCode, text } = {}) {
    const base = { key, code: code ?? key, windowsVirtualKeyCode: windowsVirtualKeyCode ?? 0 };
    await this.cdp.send("Input.dispatchKeyEvent", { type: "keyDown", ...base, ...(text ? { text } : {}) });
    await this.cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
  }

  /** 真实 Tauri IPC 往返（与产品运行时同一条 invoke 通道）。 */
  async invoke(command, args = {}) {
    return this.evaluate(
      `(async () => {
        const inv = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
        if (!inv) return { __noInvoke: true };
        try { return { ok: true, value: await inv(${JSON.stringify(command)}, ${JSON.stringify(args)}) }; }
        catch (e) { return { ok: false, error: String(e) }; }
      })()`
    );
  }

  async screenshot(name) {
    const variants = [
      { label: "plain", params: { format: "png" } },
      { label: "fromSurface-false", params: { format: "png", fromSurface: false } },
      { label: "beyondViewport-false", params: { format: "png", captureBeyondViewport: false } },
    ];
    for (const v of variants) {
      try {
        await this.cdp.send("Page.bringToFront", {}, 5000).catch(() => {});
        const shot = await this.cdp.send("Page.captureScreenshot", v.params, 15000);
        const file = path.join(this.runDir, `${name.replace(/[^\w.-]+/g, "_")}.png`);
        fs.writeFileSync(file, Buffer.from(shot.data, "base64"));
        return file;
      } catch (error) {
        this.screenshotErrors.push(`${v.label}: ${error.message}`);
      }
    }
    return null;
  }

  async close({ keep = true } = {}) {
    try { this.cdp?.close(); } catch {}
    try { this.child.kill(); } catch {}
    await sleep(400);
    return { appOutput: this.appOutput(), exitCode: this.exitCode(), runDir: this.runDir };
  }
}

/** 步骤记录器：blocked_by_quality_gate 记为 blocked，而不是 pass/fail。 */
export function createStepRecorder({ session, artifactsDir }) {
  const steps = [];
  return {
    steps,
    async run(name, fn) {
      const started = Date.now();
      try {
        const value = await fn();
        if (value && value.outcome === "blocked_by_quality_gate") {
          steps.push({ name, status: "blocked", detail: value, ms: Date.now() - started });
        } else {
          steps.push({ name, status: "passed", detail: value ?? null, ms: Date.now() - started });
        }
      } catch (error) {
        const shot = session ? await session.screenshot(`fail-${steps.length}-${name}`).catch(() => null) : null;
        steps.push({
          name,
          status: "failed",
          error: String(error?.message ?? error),
          screenshot: shot,
          ms: Date.now() - started,
        });
      }
      const last = steps[steps.length - 1];
      console.log(`[step] ${last.status.toUpperCase()} ${name}${last.error ? ` :: ${last.error}` : ""}`);
      return last;
    },
  };
}

export function writeReport(runDir, report) {
  const file = path.join(runDir, "report.json");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(report, null, 2));
  return file;
}

export function gitHead(root) {
  try {
    return execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
  } catch {
    return null;
  }
}

export function gitWorktreeClean(root) {
  try {
    return execFileSync("git", ["status", "--porcelain"], { cwd: root, encoding: "utf8" }).trim() === "";
  } catch {
    return null;
  }
}

export { repoRoot };
