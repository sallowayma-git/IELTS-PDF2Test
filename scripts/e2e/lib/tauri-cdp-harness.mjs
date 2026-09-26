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

import {
  BACKEND_INPUT_DIRS,
  BACKEND_INPUT_FILES,
  compareManifest,
  DIST_DIR,
  FRONTEND_INPUT_DIRS,
  FRONTEND_INPUT_FILES,
  loadManifestForExe,
} from "./build-manifest.mjs";

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

/**
 * 构建链的三段输入定义。产物 exe **同时**内嵌两样东西：
 *   1) 前端产物 `dist/**`（由 `vite build` 从 `src/**` 等前端输入生成）；
 *   2) Rust 后端（由 `src-tauri/src/**` 等后端输入编译）。
 * 因此「exe 是否等于当前源码」是一条三段链，而不是「源码 vs exe」两两比较：
 *
 *     frontendInputs  --vite build-->  dist  --tauri build-->  exe  <--cargo build--  backendInputs
 *
 * ⚠️ 这正是「假 fresh」的成因：`tauri build --no-bundle` 配 `beforeBuildCommand:""`
 * **不会**重建前端。若只比较 `src` 与 `exe`，一次「旧 dist + 新 exe」会完全漏判——
 * 只要 src 的 mtime 早于 exe（例如先改 src、后 build dist、再 build exe，或 exe 由
 * 后端改动触发重建），旧的 dist 就会被新 exe 包进去而仍判 fresh。
 *
 * 判定的**首选**依据是内容哈希清单（`./build-manifest.mjs`，由
 * `node scripts/e2e/build-app.mjs` 产出）：只要清单里三段哈希与当前工作树一致，
 * exe 就是当前源码的产物，与 mtime 无关。找不到清单时才退回 mtime 三段链。
 */

function walkFiles(p, out) {
  if (!fs.existsSync(p)) return;
  const st = fs.statSync(p);
  if (st.isDirectory()) {
    for (const entry of fs.readdirSync(p)) walkFiles(path.join(p, entry), out);
  } else {
    out.push({ path: p, mtimeMs: st.mtimeMs });
  }
}

function newestOf(files) {
  return files.reduce((acc, f) => (f.mtimeMs > acc.mtimeMs ? f : acc), { path: null, mtimeMs: 0 });
}

function collect(root, dirs, files) {
  const out = [];
  for (const d of dirs) walkFiles(path.join(root, d), out);
  for (const f of files) walkFiles(path.join(root, f), out);
  return out;
}

/** 构建新鲜度检查（三段链：前端输入 → dist → exe，后端输入 → exe）。
 *
 * 任一段断裂即 `staleBuild`，判定为 CANNOT-RUN，不算通过：
 *   - `dist` 缺失或比前端输入旧  => exe 内嵌的是陈旧前端（**不可容忍**）；
 *   - `dist` 比 exe 新            => exe 早于前端产物，未包含最新前端（**不可容忍**）；
 *   - 后端输入比 exe 新           => exe 早于后端源码（见下）。
 *
 * `tolerateConcurrentEdits`：本仓库有两个 agent 并行写入（识别/云端后端 agent 独占
 * `src-tauri/src/{processing,recognition,reconcile,llm_*}/**`）。当对方在本次构建之后
 * 继续落盘时，exe 相对**最新后端源码**永远是「陈旧」的，但这不代表本次运行的二进制
 * 不是从被验收的源码构建出来的。开启该选项时，**只**豁免「后端输入比 exe 新」这一类，
 * 把它们原样列出，由调用方写进报告，标注为「并发外部改动，不在本次验收范围」。
 * 前端两段（dist 陈旧 / exe 早于 dist）**不豁免**：前端 `src/**` 由本 agent 独占，
 * 不存在并发写入，陈旧即是真实缺陷。
 */
export function assertBuildFresh({ exePath, repoRoot: root = repoRoot, tolerateConcurrentEdits = false }) {
  const exeMs = fs.statSync(exePath).mtimeMs;
  const exeSha = sha256File(exePath);
  const manifest = loadManifestForExe(root, exeSha);
  if (manifest) return assertFreshViaManifest({ root, exePath, exeMs, exeSha, manifest, tolerateConcurrentEdits });

  const frontendInputs = collect(root, FRONTEND_INPUT_DIRS, FRONTEND_INPUT_FILES);
  const backendInputs = collect(root, BACKEND_INPUT_DIRS, BACKEND_INPUT_FILES);
  const distOutputs = collect(root, [DIST_DIR], []);
  const allInputs = [...frontendInputs, ...backendInputs];

  const frontendNewest = newestOf(frontendInputs);
  const backendNewest = newestOf(backendInputs);
  const srcNewest = newestOf(allInputs);
  const distNewest = newestOf(distOutputs);

  const iso = (ms) => new Date(ms).toISOString();
  const rel = (p) => (p ? path.relative(root, p).replace(/\\/g, "/") : "(none)");

  // 前端两段：硬失败，不因 tolerateConcurrentEdits 豁免。
  const hardViolations = [];
  if (distOutputs.length === 0) {
    hardViolations.push(
      `dist 不存在或为空（${path.join(root, DIST_DIR)}）：exe 内嵌的前端产物无法核对，先运行 \`npm run build\`。`
    );
  } else {
    if (frontendNewest.mtimeMs > distNewest.mtimeMs) {
      hardViolations.push(
        `dist 落后于前端源码（${rel(frontendNewest.path)} @ ${iso(frontendNewest.mtimeMs)} > dist @ ${iso(distNewest.mtimeMs)}）：dist 是陈旧构建，先运行 \`npm run build\` 再 \`tauri build\`。`
      );
    }
    if (distNewest.mtimeMs > exeMs) {
      hardViolations.push(
        `exe 早于 dist（dist @ ${iso(distNewest.mtimeMs)} > exe @ ${iso(exeMs)}）：exe 未包含最新前端产物，先运行 \`npx tauri build --debug --no-bundle\`。`
      );
    }
  }

  // 后端段：并发 agent 可能在本 agent 构建之后继续落盘 => 可豁免。
  const backendNewer = backendInputs
    .filter((f) => f.mtimeMs > exeMs)
    .sort((a, b) => b.mtimeMs - a.mtimeMs);

  if (hardViolations.length) {
    throw new CannotRunError(`staleBuild: ${hardViolations.join(" ")}`);
  }
  if (backendNewer.length && !tolerateConcurrentEdits) {
    throw new CannotRunError(
      `staleBuild: 后端源码/配置比 exe 新（${rel(backendNewer[0].path)} @ ${iso(backendNewer[0].mtimeMs)} > exe @ ${iso(exeMs)}）。` +
        "请先重新构建再验收；若确认是并发 agent 的构建后改动，加 --tolerate-concurrent-edits。"
    );
  }

  return {
    mode: "mtime",
    exeMs,
    srcNewestMs: srcNewest.mtimeMs,
    srcNewestPath: srcNewest.path,
    frontendNewestMs: frontendNewest.mtimeMs,
    frontendNewestPath: frontendNewest.path,
    distNewestMs: distNewest.mtimeMs,
    distNewestPath: distNewest.path,
    backendNewestMs: backendNewest.mtimeMs,
    backendNewestPath: backendNewest.path,
    chainOk: true,
    tolerated: tolerateConcurrentEdits
      ? backendNewer.map((f) => ({
          path: rel(f.path),
          mtime: iso(f.mtimeMs),
        }))
      : [],
  };
}

/**
 * 内容哈希判定：exe 的清单已找到，逐段比对当前工作树。
 *
 * 与 mtime 判定相比，它回答的是「内容对不对」而不是「谁更新」：
 *   - 后端 agent 在我构建之后碰一下 `scheduler.rs`（mtime 变新、内容也可能变）：
 *     若内容真的变了 => 后端段漂移；若只是 touch => 内容相同，不算漂移。
 *   - `npm run build` 重新产出内容相同的 dist：mtime 判定会误报「exe 早于 dist」，
 *     内容判定不会。
 * 前端两段（frontendInputs / dist）**不豁免**；后端段在 `tolerateConcurrentEdits` 下豁免，
 * 并把差异原样列出（「并发外部改动，不在本次验收范围」）。
 */
function assertFreshViaManifest({ root, exePath, exeMs, exeSha, manifest, tolerateConcurrentEdits }) {
  const hard = compareManifest(root, manifest, ["frontendInputs", "dist"]);
  const backend = compareManifest(root, manifest, ["backendInputs"]);
  const segLabel = { frontendInputs: "前端输入", dist: "dist", backendInputs: "后端输入" };
  const fmt = (d) =>
    `${segLabel[d.segment] ?? d.segment}（清单 ${String(d.expected).slice(0, 12)} → 当前 ${String(d.actual).slice(0, 12)}` +
    `${d.newestPath ? `，最新 ${d.newestPath}` : ""}，文件数 ${d.expectedFiles}→${d.actualFiles}）`;

  const violations = [];
  if (manifest.inputsDriftedDuringBuild) {
    violations.push(
      `清单自陈构建期间输入漂移（${(manifest.driftedSegments ?? []).join(", ")}）：该 exe 不可归因`
    );
  }
  for (const d of hard.diffs) violations.push(fmt(d));
  if (backend.diffs.length && !tolerateConcurrentEdits) {
    for (const d of backend.diffs) violations.push(fmt(d));
  }

  if (violations.length) {
    throw new CannotRunError(
      `staleBuild（内容比对，清单 ${manifest.createdAt}）：exe 不是当前源码/前端的产物 —— ${violations.join("；")}。` +
        "请运行 `node scripts/e2e/build-app.mjs` 重新构建。"
    );
  }

  return {
    mode: "manifest",
    exeMs,
    exeSha256: exeSha,
    manifestCreatedAt: manifest.createdAt,
    srcNewestMs: exeMs,
    srcNewestPath: manifest.exePath,
    frontendNewestMs: exeMs,
    frontendNewestPath: `${manifest.frontendInputs.fileCount} 个前端输入（内容哈希一致）`,
    distNewestMs: exeMs,
    distNewestPath: `${manifest.dist.fileCount} 个 dist 产物（内容哈希一致）`,
    backendNewestMs: exeMs,
    backendNewestPath: `${manifest.backendInputs.fileCount} 个后端输入（内容哈希一致）`,
    hashes: {
      frontendInputs: manifest.frontendInputs.hash,
      dist: manifest.dist.hash,
      backendInputs: manifest.backendInputs.hash,
    },
    chainOk: true,
    tolerated: tolerateConcurrentEdits
      ? backend.diffs.map((d) => ({ path: d.newestPath ?? "(backend inputs)", reason: "内容已漂移" }))
      : [],
  };
}

/** 把 `assertBuildFresh` 的返回值规范成报告字段（含 dist 段），供各 E2E 统一写入
 * `identity.buildFresh`。写入 dist 段是必需的：只记 `srcNewest` 时，报告读者无法看出
 * exe 内嵌的到底是哪一版前端。 */
export function buildFreshReport(fresh) {
  const iso = (ms) => (ms ? new Date(ms).toISOString() : null);
  return {
    ok: true,
    // `mode` 决定读者该怎么解释下面的字段：
    //   "manifest" => 内容哈希比对通过，mtime 字段无意义（统一填 exe 时刻）；
    //   "mtime"    => 没找到清单，退回时间戳三段链。
    mode: fresh.mode ?? "mtime",
    chain: "frontend-inputs -> dist -> exe; backend-inputs -> exe",
    exeMtime: iso(fresh.exeMs),
    exeSha256: fresh.exeSha256 ?? null,
    manifestCreatedAt: fresh.manifestCreatedAt ?? null,
    hashes: fresh.hashes ?? null,
    srcNewest: iso(fresh.srcNewestMs),
    srcNewestPath: fresh.srcNewestPath,
    frontendNewest: iso(fresh.frontendNewestMs),
    frontendNewestPath: fresh.frontendNewestPath,
    distNewest: iso(fresh.distNewestMs),
    distNewestPath: fresh.distNewestPath,
    backendNewest: iso(fresh.backendNewestMs),
    backendNewestPath: fresh.backendNewestPath,
    toleratedConcurrentEdits: fresh.tolerated ?? [],
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
 * 把宿主进程环境里**已知会毒化 App 启动**的项清掉，再交给真实 exe。
 *
 * 为什么需要：2026-09-22 那次「WebView2 DevTools 端点完全起不来」的根因就是**执行环境**
 * 而不是机器（结论已更正，见 `findings.md` F-WEBVIEW2-CDP-UNAVAILABLE-2026-09-22`）：
 * 同一台机器、同一分支的新构建上，清掉这些项之后冒烟 5/5、阅读链 13/13 都过。
 *
 * 清三类：
 *  - `PATH` 里空项 / 不存在的目录（被写坏的首项会让子进程的查找行为变得不可预测）；
 *  - `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`（含小写）：本机回环请求会被宿主代理劫持——
 *    连 `curl` 都会给出**假阳性**（代理错误页被当成响应体却 exit 0），受控模型服务的
 *    loopback stub 也会被绕过；
 *  - `__COMPAT_LAYER=Installer`：兼容性垫片会改变 WebView2 的进程启动行为。
 *
 * 只做减法，其余变量原样保留：产品测试环境要尽量贴近真实使用环境。
 */
export function sanitizedAppEnv(base = process.env) {
  const env = { ...base };
  for (const key of [
    "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY",
    "http_proxy", "https_proxy", "all_proxy",
  ]) {
    delete env[key];
  }
  delete env.__COMPAT_LAYER;
  // Windows 的环境变量名不区分大小写，但展开成普通对象后是区分的：PowerShell 下键名是
  // `Path`，只看 `env.PATH` 会整段漏掉清理，调用方再写 `PATH` 还会与 `Path` 并存。
  // 统一收成一个 `PATH` 键。
  const pathKeys = Object.keys(env).filter((key) => key.toUpperCase() === "PATH");
  const rawPath = pathKeys.map((key) => env[key]).find((value) => typeof value === "string");
  for (const key of pathKeys) delete env[key];
  if (typeof rawPath === "string") {
    // 保留原顺序（不重排），只丢掉空项与不存在的目录。
    const kept = rawPath.split(path.delimiter).filter((entry) => entry && fs.existsSync(entry));
    env.PATH = kept.length ? kept.join(path.delimiter) : rawPath;
  }
  return env;
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
    // 先清掉会毒化 App 启动的宿主环境残留（代理 / __COMPAT_LAYER / 坏掉的 PATH 项），
    // 再叠加本条的自动化变量。
    ...sanitizedAppEnv(process.env),
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
    /**
     * 断线重连的**记录**（本轮 F3 修复点）。
     *
     * 重连本身是对的（WebView2 会重建 page target，不重连就只能把一次瞬时重建报成
     * 「页面没渲染」），但**静默**重连等于把 renderer 崩溃洗成正常：脚本接到一个刚
     * 重载的页面上继续跑，断言照样全过。所以每次重连都往这里记一条，由脚本写进报告，
     * 并让 verdict 至少降成「通过但有警告」。
     */
    this.reattaches = [];
    /** 当前正在跑的步骤名，由 `createStepRecorder` 维护；步骤之外为 `null`。 */
    this.currentStep = null;
  }

  /** 记一次重连。`reason` 原样保留，不截断成「连接断了」这种看不出原因的话。 */
  _recordReattach(reason) {
    const entry = {
      at: new Date().toISOString(),
      step: this.currentStep,
      reason: String(reason ?? "").slice(0, 300),
    };
    this.reattaches.push(entry);
    this.log(`CDP 重连 #${this.reattaches.length}（step=${entry.step ?? "(steps 之外)"}）：${entry.reason.slice(0, 96)}`);
    return entry;
  }

  /**
   * 连到当前 App 的 page target（可重复调用：会用新连接替换旧连接）。
   *
   * WebView2 在启动阶段会**重建 page target**，旧 WebSocket 随之关闭。仓库里早有记录：
   * 重建之后 `Runtime.evaluate` 一律报「CDP 连接已关闭」。所以「发现 target → 建 WS」
   * 不能只做一次 —— 必须能在会话中途重新附着，否则一次瞬时的 target 重建会被报成
   * 「页面 90000ms 内未渲染出可见文本」，把人往「前端没渲染」的方向带偏。
   */
  async attachToPageTarget({ timeoutMs = 20000, settleMs = 400 } = {}) {
    const deadline = Date.now() + timeoutMs;
    let lastError = null;
    while (Date.now() < deadline) {
      if (this.exitCode() !== null) {
        throw new CannotRunError(`应用进程已退出（code=${this.exitCode()}），无法重新附着 page target`);
      }
      try {
        const res = await fetch(`http://127.0.0.1:${this.devtoolsPort}/json/list`);
        if (res.ok) {
          const list = await res.json();
          const target = list.find((t) => t.type === "page" && t.webSocketDebuggerUrl);
          if (target) {
            // 刚重建出来的 target 立刻连往往连到一个马上又被换掉的文档上，
            // 稍等一拍再连，避免刚连上就再断一次。
            await sleep(settleMs);
            return await this._connectPageTarget(target);
          }
        }
      } catch (error) {
        lastError = error;
      }
      await sleep(250);
    }
    throw new CannotRunError(
      `重新附着 page target 失败（${timeoutMs}ms）：${lastError?.message ?? "未发现 page target"}`
    );
  }

  async _connectPageTarget(target) {
    const ws = new WebSocket(target.webSocketDebuggerUrl, {
      perMessageDeflate: false,
      maxPayload: 256 * 1024 * 1024,
    });
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new CannotRunError("CDP WebSocket 重连超时")), 20000);
      ws.once("open", () => { clearTimeout(timer); resolve(); });
      ws.once("error", (e) => { clearTimeout(timer); reject(e); });
    });
    const connection = new CdpConnection(ws);
    await connection.send("Runtime.enable", {}, 30000);
    await connection.send("Page.enable", {}, 30000);
    const previous = this.cdp;
    this.cdp = connection;
    this.ws = ws;
    try { previous?.close(); } catch {}
    return connection;
  }

  /** 底层求值：异常会抛出，返回 by-value；连接断了会自动重新附着再重试一次。 */
  async evaluate(expression, { timeoutMs = 60000, awaitPromise = true, reattach = true } = {}) {
    try {
      return await this._evaluateOnce(expression, { timeoutMs, awaitPromise });
    } catch (error) {
      const message = String(error?.message ?? error);
      const disconnected =
        this.cdp?.closed === true ||
        message.includes("CDP 连接已关闭") ||
        message.includes("WebSocket is not open");
      if (!reattach || !disconnected) throw error;
      this._recordReattach(message);
      await this.attachToPageTarget();
      return await this._evaluateOnce(expression, { timeoutMs, awaitPromise });
    }
  }

  async _evaluateOnce(expression, { timeoutMs, awaitPromise }) {
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
    // 这个 `catch {}` 曾经把两类完全不同的故障混成同一句话「未渲染出可见文本」：
    //   (a) 页面真的没有可见文本；(b) `evaluate` 每次都抛（连求值都做不到）。
    // 两者的排查方向完全相反，所以这里把最后一次求值错误留下来。
    let lastEvalError = null;
    let lastUrl = null;
    while (Date.now() < deadline) {
      if (this.exitCode() !== null) throw new CannotRunError(`应用在页面就绪前退出（code=${this.exitCode()}）`);
      try {
        const text = await this.evaluate("document.body ? document.body.innerText : ''", { timeoutMs: 15000 });
        lastText = String(text ?? "");
        lastEvalError = null;
        if (lastText.trim().length > 0) return lastText;
        // 页面有 body 但没文本：把当前 URL 记下来，区分「没导航」与「导航了但白屏」。
        lastUrl = await this.evaluate("location.href", { timeoutMs: 15000 }).catch(() => null);
      } catch (error) {
        lastEvalError = String(error?.message ?? error);
      }
      await sleep(500);
    }
    const detail = lastEvalError
      ? `求值持续失败（最后一次：${lastEvalError}）`
      : `最后文本=${JSON.stringify(lastText.slice(0, 120))}，url=${String(lastUrl)}`;
    throw new CannotRunError(`页面在 ${timeoutMs}ms 内未渲染出可见文本（${detail}）`);
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

  /**
   * 点一个**布局可能还在动**的元素：先等它连续若干次读到同一个中心点，再用真实鼠标事件点击。
   *
   * 为什么需要单独一个方法：`clickSelector` 只量**一次**坐标。抽屉打开后有一行提示
   * （「未连接云端，仅本地识别 · 去连接」）要等 profile 列表异步返回才插入，插入后把
   * 「选择文件」按钮**下推约 49px**。量在位移之前、点在后头，就点在空白处——
   * 真机上表现为「点了没反应」，脚本里表现为这一步超时、后续级联失败。
   * 真实用户手快抢在异步返回之前点，也会点空，所以这不是脚本独有的问题。
   *
   * **刻意不退回 `element.click()`**：那会绕过真实鼠标事件（命中测试、遮挡、`disabled`
   * 全都验不到），等于把被测行为换掉。这里等的是坐标稳定，而不是换一种点击方式。
   *
   * 一直等不到稳定就**如实抛错**（带上最后一次坐标），不猜一个位置点下去。
   */
  async clickSelectorWhenStable(selector, { timeoutMs = 20000, settleReads = 3, intervalMs = 120 } = {}) {
    const expr = `(() => {
      const el = document.querySelector(${JSON.stringify(selector)});
      if (!el) return null;
      el.scrollIntoView({ block: 'center', inline: 'center' });
      const r = el.getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2, w: r.width, h: r.height, tag: el.tagName };
    })()`;
    const deadline = Date.now() + timeoutMs;
    let previous = null;
    let stable = 0;
    let last = null;
    while (Date.now() < deadline) {
      const box = await this.evaluate(expr, { timeoutMs: 15000 }).catch(() => null);
      if (box) {
        last = box;
        stable = previous && previous.x === box.x && previous.y === box.y
          && previous.w === box.w && previous.h === box.h
          ? stable + 1
          : 1;
        previous = box;
        if (stable >= settleReads) {
          if (box.w === 0 || box.h === 0) throw new Error(`目标元素尺寸为 0，无法点击：${selector}`);
          await this.clickAt(box.x, box.y);
          return { ...box, settleReads: stable };
        }
      }
      await sleep(intervalMs);
    }
    throw new Error(
      `等待「${selector}」位置稳定超时（${timeoutMs}ms，需要连续 ${settleReads} 次读到同一坐标），最后一次坐标=${JSON.stringify(last)}`
    );
  }

  /**
   * 打开顶栏「待补充」侧栏，等处理真正结束，读回全部编辑辅助条目。
   *
   * 三件事都必须做，少一件就会读到**假清单**：
   * - **先展开**：清单是顶栏 `[data-testid="workspace-issues"]` 开合的侧栏，默认收起，
   *   收起时条目根本不在 DOM 里。而且它只在编辑模式渲染（`mode === "edit"`）——
   *   学生预览里没有这份清单。
   * - **等处理结束**：处理没跑完之前，云端剩余条目还没并进清单，此时数出来的是
   *   一份不完整的清单。判据是标题下的处理副标题
   *   `[data-testid="workspace-processing-note"]` 消失（一直不消失本身就是要报出来的问题，
   *   所以如实返回 `settled: false` 而不是抛错）。
   * - **再展开分组**：条目多时清单会折叠，不点开只能读到前几组。
   *
   * 条目钩子：`[data-task-id]` / `[data-task-kind]` / `[data-severity]` /
   * `[data-task-covers]`（合并后仍保留的底层问题关联）/ `[data-action-id]` / `[data-action-target]`。
   */
  async readTaskList({ timeoutMs = 30000, settleTimeoutMs = 90000 } = {}) {
    await this.evaluate(`(() => {
      const toggle = document.querySelector('[data-testid="workspace-issues"]');
      if (toggle && toggle.getAttribute('aria-expanded') !== 'true') toggle.click();
      return true;
    })()`);
    await this.waitFor(
      `(() => Boolean(document.querySelector('[data-testid="workspace-tasks-headline"], [data-testid="workspace-tasks-clear"]')))()`,
      { timeoutMs, intervalMs: 500, label: "task-list-rendered" },
    ).catch(() => null);
    const settled = await this.waitFor(
      `(() => !document.querySelector('[data-testid="workspace-processing-note"]'))()`,
      { timeoutMs: settleTimeoutMs, intervalMs: 1000, label: "processing-settled" },
    )
      .then(() => true)
      .catch(() => false);
    await this.evaluate(
      `(() => { const more = document.querySelector('[data-testid="workspace-tasks-more"]'); if (more) more.click(); return true; })()`,
    );
    const panel = await this.evaluate(`(() => {
      const entries = [...document.querySelectorAll('[data-task-id]')].map((el) => ({
        taskId: el.getAttribute('data-task-id'),
        kind: el.getAttribute('data-task-kind'),
        severity: el.getAttribute('data-severity'),
        covers: (el.getAttribute('data-task-covers') ?? '').split(',').filter(Boolean),
        targets: [...el.querySelectorAll('[data-action-target]')].map((b) => b.getAttribute('data-action-target')),
        actions: [...el.querySelectorAll('[data-action-id]')].map((b) => b.getAttribute('data-action-id')),
        text: el.innerText.replace(/\\s+/g,' ').trim().slice(0, 160)
      }));
      const clear = document.querySelector('[data-testid="workspace-tasks-clear"]');
      const root = document.querySelector('[data-testid="workspace-issue-list"]');
      return {
        entryCount: entries.length,
        entries,
        clearText: clear ? clear.innerText.replace(/\\s+/g,' ').trim() : null,
        taskCount: root ? Number(root.getAttribute('data-task-count') ?? 0) : null,
        preflightState: root ? root.getAttribute('data-preflight-state') : null,
        tasksReady: root ? root.getAttribute('data-tasks-ready') : null,
        legacyCardCount: document.querySelectorAll('[data-decision-id]').length
      };
    })()`);
    return { ...panel, settled };
  }

  /** 读当前工作区提示元素上的发布结论（`{ text, kind }`，没有则 `kind: null`）。 */
  async readPublishNotice() {
    return this.evaluate(`(() => {
      const el = document.querySelector('.workspace-notice[data-publish-outcome]')
        ?? document.querySelector('.workspace-notice');
      if (!el) return { text: null, kind: null };
      return {
        text: el.innerText.replace(/\\s+/g,' ').trim() || null,
        kind: el.getAttribute('data-publish-outcome')
      };
    })()`);
  }

  /**
   * 点「发布」并读回**机器可读**的发布结论。
   *
   * 为什么不能匹配提示文案：产品决策是「不向用户展示放行与否」，放行发布与干净发布
   * 显示的是同一句「已发布」。所以「文案里有『已发布』」既认不出干净发布、也认不出
   * 学生端打不开的那一条。唯一的机器可读出口是 `.workspace-notice[data-publish-outcome]`，
   * 取值 `published | published_forced | published_forced_not_loadable | failed`——
   * **只有 `published` 算干净通过**（见 `isCleanPublishOutcome`）。
   *
   * 点之前先读一次提示：界面上可能已经挂着一条**无关**的提示（例如「识别建议已过期」），
   * 直接读 `.workspace-notice` 会把它当成这次点击的结论。要等它**变成别的文字**。
   */
  async publishAndReadOutcome({ timeoutMs = 120000 } = {}) {
    const before = await this.readPublishNotice();
    await this.clickSelector('[data-testid="workspace-publish"]');
    const settled = await this.waitFor(
      `(() => {
         const el = document.querySelector('.workspace-notice[data-publish-outcome]');
         if (!el) return null;
         const text = el.innerText.replace(/\\s+/g,' ').trim() || null;
         const kind = el.getAttribute('data-publish-outcome');
         return kind && text && text !== ${JSON.stringify(before.text)} ? { text, kind } : null;
       })()`,
      { timeoutMs, intervalMs: 1000, label: "publish-outcome" },
    ).catch(() => null);
    return {
      noticeBefore: before.text,
      text: settled?.text ?? null,
      kind: settled?.kind ?? null,
      timedOut: settled === null,
    };
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

/**
 * 步骤记录器：blocked_by_quality_gate 记为 blocked，而不是 pass/fail。
 *
 * 同时把当前步骤名写给 `session.currentStep`——重连记录要靠它回答「这次断线发生在
 * 哪一步」（见 `_recordReattach`）。步骤之外发生的断线记为 `step: null`，不丢。
 */
export function createStepRecorder({ session, artifactsDir }) {
  const steps = [];
  return {
    steps,
    async run(name, fn) {
      const started = Date.now();
      if (session) session.currentStep = name;
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
      } finally {
        if (session) session.currentStep = null;
      }
      const last = steps[steps.length - 1];
      console.log(`[step] ${last.status.toUpperCase()} ${name}${last.error ? ` :: ${last.error}` : ""}`);
      return last;
    },
  };
}

/** 重连策略取值。只有显式声明 `acceptReattaches` 的脚本才配得上干净 `passed`。 */
export const REATTACH_POLICY = Object.freeze({
  /** 默认：重连要报出来，verdict 至少降为 `passed_with_warnings`。 */
  WARN: "warn-on-reattach",
  /** 显式声明：这份脚本接受重连（例如它单独断言了页面重载后的行为）。 */
  ACCEPTED: "accepted",
});

/**
 * 把会话上的重连记录规范成报告字段（写在每个脚本的 `report.cdpReattaches` 上）。
 *
 * `count: 0` 也必须写出来——「这次一个重连都没有」跟「这个字段根本不存在」是两件事，
 * 报告读者要能区分。
 */
export function summarizeReattaches(session, { acceptReattaches = false } = {}) {
  const entries = (session?.reattaches ?? []).map((entry) => ({ ...entry }));
  return {
    policy: acceptReattaches ? REATTACH_POLICY.ACCEPTED : REATTACH_POLICY.WARN,
    acceptReattaches: Boolean(acceptReattaches),
    count: entries.length,
    steps: [...new Set(entries.map((entry) => entry.step).filter(Boolean))],
    entries,
  };
}

/**
 * 按重连记录决定最终 verdict。
 *
 * 规则：**发生重连的运行 verdict 至少降为 `passed_with_warnings`**——重连过的运行
 * 不是「同样的运行」，读者必须看见这一点。要把它当成可接受的脚本得显式声明
 * `acceptReattaches`（`--accept-reattaches`）。
 *
 * 反向不成立：已经 failed / cannot-run 的运行不会因为「有重连」被降级成警告。
 * 重连只影响「本来是 passed」的那一档，绝不把失败洗成通过。
 */
export function applyReattachPolicy(verdict, reattaches, { acceptReattaches = false } = {}) {
  const count = (reattaches ?? []).length;
  if (verdict !== "passed" || count === 0 || acceptReattaches) {
    return { verdict, downgraded: false, warning: null };
  }
  const steps = [...new Set((reattaches ?? []).map((entry) => entry.step).filter(Boolean))];
  return {
    verdict: "passed_with_warnings",
    downgraded: true,
    warning:
      `本次运行发生 ${count} 次 CDP 断线重连（步骤：${steps.join(", ") || "步骤之外"}）：`
      + "重连可能掩盖 renderer 崩溃重建，按默认策略降级为「通过但有警告」。"
      + "若该脚本确实接受重连，请显式声明 --accept-reattaches。",
  };
}

/**
 * 发布结论是否算**干净**通过。
 *
 * 只有 `published` 是。`published_forced`（用户放行了门禁）与
 * `published_forced_not_loadable`（学生端暂时打不开这道题）都不算——产品上它们是
 * 「已发布」，但验收链要证明的是**干净**发布这一跳真的走通了，所以一律按 blocked 报。
 */
export function isCleanPublishOutcome(kind) {
  return kind === "published";
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
