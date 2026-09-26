// 真实 Tauri E2E 共享驱动（scripts/e2e/tauri-*.mjs 共用）。
//
// 证据层级：**product** —— 驱动真实 Tauri 进程 + WebView2 + 真实 SQLite + 真实文件系统。
// 与 `library-workspace-smoke.mjs`（浏览器 + devFallback）和 Rust 命令级测试是三套不同证据，
// 分别报告，不互相替代（AGENTS.md / audit A11-F07）。
//
// 退出码约定（两个调用脚本一致）：
//   0 = passed（全部断言满足）
//   1 = failed（某一步断言失败）
//   2 = harness error（驱动/环境异常，非产品结论）
//   3 = cannot-run（缺少真实 Tauri 运行前提：非 Windows / exe 未构建 / 测试 PDF 缺失 / 无 WebView2 驱动）
//   4 = blocked（跑起来了，但产品门禁使目标断言无法执行——不计为通过，也不计为产品缺陷）
//
// 隔离：应用进程继承被改写的测试钩子环境变量，SQLite / WebView2 配置 / 发布产物
// 全部落到本次运行的临时目录，不污染真实用户数据。

import { spawn, spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { Builder, By, until } from "selenium-webdriver";

import { sanitizedAppEnv } from "./tauri-cdp-harness.mjs";

export const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
export const DEFAULT_EXE = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
export const DEFAULT_PDF = path.join(repoRoot, "fixtures", "golden", "synthetic", "pdf", "pdf-two-column.pdf");
export const DRIVER_CACHE_DIR = path.join(process.env.LOCALAPPDATA ?? repoRoot, "pdf2test-e2e-drivers");

const WEBVIEW2_REG_KEY = "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
const MSEDGEDRIVER_CDN = "https://msedgedriver.microsoft.com";

/** 缺少真实运行前提时抛出；调用脚本据此以退出码 3 报告 cannot-run，而不是静默通过。 */
export class CannotRunError extends Error {}

export { By, until };

export function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--keep") out.keep = true;
    else if (arg === "--no-screenshot") out.screenshot = false;
    else if (arg.startsWith("--")) out[arg.slice(2)] = argv[i + 1];
  }
  return out;
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
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
  return probe.stdout.match(/REG_SZ\s+([\d.]+)/)?.[1] ?? null;
}

function driverOnPath() {
  const probe = runCapture("where", ["msedgedriver"]);
  if (probe.status !== 0) return null;
  const first = probe.stdout.split(/\r?\n/).find((line) => line.trim().endsWith(".exe"));
  return first ? path.dirname(path.resolve(first.trim())) : null;
}

function msedgedriverVersion(dir) {
  const probe = runCapture(path.join(dir, "msedgedriver.exe"), ["--version"]);
  if (probe.status !== 0) return null;
  return probe.stdout.match(/(\d+\.\d+\.\d+\.\d+)/)?.[1] ?? null;
}

// msedgedriver 必须与被测 exe 实际加载的 WebView2 运行时同版本。版本不一致时驱动能启动 exe，
// 却连不上 WebView2 的调试端点，会话以 "DevToolsActivePort file doesn't exist" 失败。
// PATH 上的驱动（例如 GitHub windows 镜像预装的、跟随 Edge 浏览器版本的那个）与旧缓存都可能过期，
// 所以只接受版本一致的驱动，否则按运行时版本下载到独立的版本目录。
async function ensureMsedgedriver() {
  const runtime = webview2Version();
  if (!runtime) {
    const fallback = driverOnPath()
      ?? (fs.existsSync(path.join(DRIVER_CACHE_DIR, "msedgedriver.exe")) ? DRIVER_CACHE_DIR : null);
    if (fallback) {
      console.log(`[e2e:tauri] WebView2 runtime version unknown; using msedgedriver ${msedgedriverVersion(fallback)} from ${fallback}`);
      return fallback;
    }
    throw new CannotRunError("未找到 msedgedriver，也无法从注册表读取 WebView2 运行时版本（无法自动下载匹配驱动）。");
  }

  const versionDir = path.join(DRIVER_CACHE_DIR, runtime);
  for (const candidate of [versionDir, driverOnPath(), DRIVER_CACHE_DIR]) {
    if (!candidate || !fs.existsSync(path.join(candidate, "msedgedriver.exe"))) continue;
    const driverVersion = msedgedriverVersion(candidate);
    if (driverVersion === runtime) {
      console.log(`[e2e:tauri] msedgedriver ${driverVersion} matches WebView2 runtime (${candidate})`);
      return candidate;
    }
    console.log(`[e2e:tauri] skipping msedgedriver ${driverVersion ?? "(unknown)"} at ${candidate}: WebView2 runtime is ${runtime}`);
  }

  console.log(`[e2e:tauri] WebView2 runtime ${runtime}; downloading matching msedgedriver...`);
  fs.mkdirSync(versionDir, { recursive: true });
  const zipPath = path.join(versionDir, `edgedriver-${runtime}.zip`);
  const zipResult = runCapture("powershell", [
    "-NoProfile", "-Command",
    `Invoke-WebRequest -Uri '${MSEDGEDRIVER_CDN}/${runtime}/edgedriver_win64.zip' -OutFile '${zipPath}'`
  ]);
  if (zipResult.status !== 0 || !fs.existsSync(zipPath)) {
    throw new CannotRunError(`下载 msedgedriver ${runtime} 失败：${zipResult.stdout.slice(0, 400)}`);
  }
  const unzip = runCapture("powershell", [
    "-NoProfile", "-Command",
    `Expand-Archive -Force -Path '${zipPath}' -DestinationPath '${versionDir}'`
  ]);
  if (unzip.status !== 0 || !fs.existsSync(path.join(versionDir, "msedgedriver.exe"))) {
    throw new CannotRunError(`解压 msedgedriver 失败：${unzip.stdout.slice(0, 400)}`);
  }
  return versionDir;
}

/**
 * 会话建不起来时，把能解释「DevToolsActivePort file doesn't exist」的证据打出来：
 * 启动前被清掉的环境残留，以及 msedgedriver 给 WebView2 分配的 %TEMP%\scoped_dir*
 * （它会覆盖 WEBVIEW2_USER_DATA_FOLDER）里有没有生成 EBWebView 配置、chrome_debug.log 写了什么。
 */
function logLaunchDiagnostics(sinceMs, runDir) {
  try {
    // WebView2 的配置目录可能落在：我们指定的 WEBVIEW2_USER_DATA_FOLDER、msedgedriver 的
    // scoped_dir、或应用默认的 %LOCALAPPDATA%\<identifier>\EBWebView。哪个都没有 = WebView2 根本没起来。
    const localAppData = process.env.LOCALAPPDATA;
    for (const dir of [
      path.join(runDir, "appdata", "webview"),
      localAppData ? path.join(localAppData, "com.ielts.author.studio") : null,
    ].filter(Boolean)) {
      const profile = path.join(dir, "EBWebView");
      console.log(`[e2e:tauri] ${dir}: EBWebView=${fs.existsSync(profile)}${fs.existsSync(profile) ? ` DevToolsActivePort=${fs.existsSync(path.join(profile, "DevToolsActivePort"))}` : ""}`);
    }
    const env = process.env;
    const proxies = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"]
      .filter((key) => env[key]);
    const badPath = (env.PATH ?? "").split(path.delimiter).filter((entry) => !entry || !fs.existsSync(entry)).length;
    console.log(`[e2e:tauri] launch env: proxies=[${proxies.join(",")}] __COMPAT_LAYER=${env.__COMPAT_LAYER ?? "(unset)"} badPathEntries=${badPath} (all removed before launch)`);
    const temp = env.TEMP ?? env.TMP;
    if (!temp || !fs.existsSync(temp)) return;
    const scoped = fs.readdirSync(temp)
      .filter((name) => name.startsWith("scoped_dir"))
      .map((name) => path.join(temp, name))
      .filter((dir) => fs.statSync(dir).mtimeMs >= sinceMs - 5000);
    if (!scoped.length) {
      console.log(`[e2e:tauri] no msedgedriver scoped_dir created under ${temp} since launch`);
      return;
    }
    for (const dir of scoped) {
      const profile = path.join(dir, "EBWebView");
      const debugLog = path.join(profile, "chrome_debug.log");
      console.log(`[e2e:tauri] ${dir}: EBWebView=${fs.existsSync(profile)} DevToolsActivePort=${fs.existsSync(path.join(profile, "DevToolsActivePort"))}`);
      if (fs.existsSync(debugLog)) {
        console.log(`[e2e:tauri] chrome_debug.log (tail):\n${fs.readFileSync(debugLog, "utf8").slice(-3000)}`);
      }
    }
  } catch (error) {
    console.log(`[e2e:tauri] launch diagnostics failed: ${error.message}`);
  }
}

/** 打印当前 msedgewebview2.exe 浏览器进程的命令行，以及可能覆盖 WebView2 参数的策略注册表项。 */
function logWebViewCommandLines() {
  const processes = runCapture("powershell", [
    "-NoProfile", "-Command",
    "Get-CimInstance Win32_Process -Filter \"name='msedgewebview2.exe'\" | "
      + "Where-Object { $_.CommandLine -notmatch '--type=' -and $_.CommandLine -match 'ielts-author-studio' } | ForEach-Object { $_.CommandLine }",
  ]);
  console.log(`[e2e:tauri] msedgewebview2 browser process command lines:\n${processes.stdout.trim().slice(0, 4000) || "(none)"}`);
  for (const key of [
    "HKLM\\SOFTWARE\\Policies\\Microsoft\\Edge",
    "HKCU\\SOFTWARE\\Policies\\Microsoft\\Edge",
    "HKLM\\SOFTWARE\\WOW6432Node\\Policies\\Microsoft\\Edge",
  ]) {
    const query = runCapture("reg", ["query", key, "/s"]);
    console.log(`[e2e:tauri] ${key}: ${query.status === 0 ? `\n${query.stdout.trim().slice(0, 2000)}` : "(absent)"}`);
  }
}

/**
 * 绕开 tauri-driver / msedgedriver，直接用 WebView2 官方变量打开调试端点启动被测 exe，
 * 判断「这台机器上 WebView2 调试端点能不能起来」：能起来 → 问题在驱动链；起不来 → 问题在
 * WebView2 本身（打印 chrome_debug.log）。只在会话建立失败时运行，只做诊断。
 */
async function probeWebView2Directly(exePath, runDir) {
  let child = null;
  try {
    const port = await freePort();
    const profileRoot = path.join(runDir, "appdata", "probe-webview");
    fs.mkdirSync(profileRoot, { recursive: true });
    const env = {
      ...sanitizedAppEnv(process.env),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-allow-origins=* --enable-logging --v=0`,
      WEBVIEW2_USER_DATA_FOLDER: profileRoot,
      PDF2TEST_AUTOMATION_DATA_DIR: path.join(runDir, "appdata", "probe-data"),
    };
    let output = "";
    let exitCode = null;
    child = spawn(exePath, [], { stdio: ["ignore", "pipe", "pipe"], env, windowsHide: true });
    child.stdout.on("data", (chunk) => { output += String(chunk); });
    child.stderr.on("data", (chunk) => { output += String(chunk); });
    child.on("exit", (code) => { exitCode = code; });
    const probeStartedAt = Date.now();
    const deadline = probeStartedAt + 90000;
    const profile = path.join(profileRoot, "EBWebView");
    let version = null;
    let profileAppearedMs = null;
    let commandLinesLogged = false;
    while (Date.now() < deadline && exitCode === null && !version) {
      try {
        const response = await fetch(`http://127.0.0.1:${port}/json/version`);
        if (response.ok) version = await response.json();
      } catch {}
      if (profileAppearedMs === null && fs.existsSync(profile)) profileAppearedMs = Date.now() - probeStartedAt;
      // 浏览器进程起来后读一次它的真实命令行：参数有没有真正传到 msedgewebview2.exe，一眼可见。
      if (!commandLinesLogged && profileAppearedMs !== null && Date.now() - probeStartedAt - profileAppearedMs > 5000) {
        commandLinesLogged = true;
        logWebViewCommandLines();
      }
      if (!version) await sleep(500);
    }
    if (!commandLinesLogged) logWebViewCommandLines();
    console.log(`[e2e:tauri] direct probe: EBWebView appeared after ${profileAppearedMs ?? "never"} ms`);
    if (fs.existsSync(profile)) {
      console.log(`[e2e:tauri] direct probe EBWebView entries: ${fs.readdirSync(profile).join(", ")}`);
    }
    console.log(
      `[e2e:tauri] direct WebView2 probe: endpoint=${version ? `up (${version.Browser ?? "?"})` : "down"} ` +
      `exit=${exitCode ?? "running"} EBWebView=${fs.existsSync(profile)} ` +
      `DevToolsActivePort=${fs.existsSync(path.join(profile, "DevToolsActivePort"))}`
    );
    if (output.trim()) console.log(`[e2e:tauri] direct probe app output (tail):\n${output.slice(-2000)}`);
    const debugLog = path.join(profile, "chrome_debug.log");
    if (fs.existsSync(debugLog)) {
      console.log(`[e2e:tauri] direct probe chrome_debug.log (tail):\n${fs.readFileSync(debugLog, "utf8").slice(-3000)}`);
    }
  } catch (error) {
    console.log(`[e2e:tauri] direct WebView2 probe failed: ${error.message}`);
  } finally {
    try { child?.kill(); } catch {}
    await sleep(1000);
  }
}

/** 被测 exe 内嵌构建时的前端产物；若 src 比 exe 新，本次结果不能证明当前源码（A11-F01 根因之一）。 */
export function newestMtimeMs(dir) {
  let newest = 0;
  const stack = [dir];
  while (stack.length) {
    const current = stack.pop();
    let entries;
    try {
      entries = fs.readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) stack.push(full);
      else if (entry.isFile()) {
        try {
          const mtime = fs.statSync(full).mtimeMs;
          if (mtime > newest) newest = mtime;
        } catch {}
      }
    }
  }
  return newest;
}

export function assertPrerequisites({ exePath, pdfPath }) {
  const missing = [];
  if (process.platform !== "win32") missing.push("真实 Tauri E2E 目前仅在 Windows（WebView2）上运行。");
  if (!fs.existsSync(exePath)) missing.push(`被测应用不存在：${exePath}（先运行 npx tauri build --debug --no-bundle）`);
  if (!fs.existsSync(pdfPath)) missing.push(`测试 PDF 不存在：${pdfPath}`);
  if (missing.length) throw new CannotRunError(missing.join("\n"));
}

export function buildFreshness(exePath, repoRootOverride = repoRoot) {
  const repoRoot = repoRootOverride;
  const exeMtimeMs = fs.statSync(exePath).mtimeMs;
  // D0 复核（缺口 2）：exe 内嵌前端产物 + Rust 静态链接，仅看 src/ 会漏掉
  // src-tauri 源码、构建配置与锁文件的漂移。全部纳入后再判定。
  const sourceScopes = [
    ["src", path.join(repoRoot, "src")],
    ["src-tauri/src", path.join(repoRoot, "src-tauri", "src")]
  ];
  const buildFiles = [
    "src-tauri/Cargo.toml",
    "src-tauri/Cargo.lock",
    "src-tauri/tauri.conf.json",
    "src-tauri/build.rs",
    "index.html",
    "vite.config.ts",
    "package.json",
    "package-lock.json"
  ];
  let newestSourceMtimeMs = 0;
  let newestSource = "(none)";
  for (const [label, dir] of sourceScopes) {
    const mtime = newestMtimeMs(dir);
    if (mtime > newestSourceMtimeMs) {
      newestSourceMtimeMs = mtime;
      newestSource = label;
    }
  }
  // capabilities 决定运行时权限面，递归纳入（目录 mtime 不反映内部文件修改）。
  const capsDir = path.join(repoRoot, "src-tauri", "capabilities");
  if (fs.existsSync(capsDir)) {
    const capsMtime = newestMtimeMs(capsDir);
    if (capsMtime > newestSourceMtimeMs) {
      newestSourceMtimeMs = capsMtime;
      newestSource = "src-tauri/capabilities";
    }
  }
  for (const file of buildFiles) {
    const full = path.join(repoRoot, file);
    if (!fs.existsSync(full)) continue;
    const mtime = fs.statSync(full).mtimeMs;
    if (mtime > newestSourceMtimeMs) {
      newestSourceMtimeMs = mtime;
      newestSource = file;
    }
  }
  // 构建链是「前端输入 → dist → exe」+「后端输入 → exe」。exe 内嵌的是 `dist/**`，
  // 而 `tauri build --no-bundle`（配 beforeBuildCommand:""）**不会**重建前端：
  // 只比 src 与 exe 时，「旧 dist + 新 exe」会被漏判为 fresh。
  const frontendInputDirs = ["src"];
  const frontendInputFiles = ["index.html", "vite.config.ts", "vitest.config.ts", "tsconfig.json", "tsconfig.node.json", "package.json", "package-lock.json"];
  let newestFrontendMtimeMs = 0;
  let newestFrontend = "(none)";
  for (const dir of frontendInputDirs) {
    const mtime = newestMtimeMs(path.join(repoRoot, dir));
    if (mtime > newestFrontendMtimeMs) {
      newestFrontendMtimeMs = mtime;
      newestFrontend = dir;
    }
  }
  for (const file of frontendInputFiles) {
    const full = path.join(repoRoot, file);
    if (!fs.existsSync(full)) continue;
    const mtime = fs.statSync(full).mtimeMs;
    if (mtime > newestFrontendMtimeMs) {
      newestFrontendMtimeMs = mtime;
      newestFrontend = file;
    }
  }
  const distDir = path.join(repoRoot, "dist");
  const distNewestMtimeMs = newestMtimeMs(distDir);
  const distMissing = distNewestMtimeMs === 0;

  const reasons = [];
  if (newestSourceMtimeMs > exeMtimeMs) {
    reasons.push(`exe 早于源码/构建配置（newest=${new Date(newestSourceMtimeMs).toISOString()} @${newestSource}）`);
  }
  if (distMissing) {
    reasons.push("dist 不存在或为空，exe 内嵌前端产物无法核对");
  } else {
    if (newestFrontendMtimeMs > distNewestMtimeMs) {
      reasons.push(`dist 落后于前端源码（${newestFrontend} @ ${new Date(newestFrontendMtimeMs).toISOString()} > dist @ ${new Date(distNewestMtimeMs).toISOString()}）`);
    }
    if (distNewestMtimeMs > exeMtimeMs) {
      reasons.push(`exe 早于 dist（dist @ ${new Date(distNewestMtimeMs).toISOString()} > exe @ ${new Date(exeMtimeMs).toISOString()}）`);
    }
  }
  const staleBuild = reasons.length > 0;
  if (staleBuild) {
    console.warn(
      `[e2e:tauri] WARNING 被测 exe 不是当前源码/前端的构建产物：${reasons.join("；")}——` +
      "本次结果不能证明当前源码，请先重新构建再作为验收证据。"
    );
  }
  return {
    exeMtime: new Date(exeMtimeMs).toISOString(),
    newestSourceMtime: new Date(newestSourceMtimeMs).toISOString(),
    newestSource,
    newestFrontendMtime: new Date(newestFrontendMtimeMs).toISOString(),
    newestFrontend,
    distNewestMtime: distMissing ? null : new Date(distNewestMtimeMs).toISOString(),
    staleBuild,
    staleReasons: reasons
  };
}

/** D0 复核（缺口 2）：陈旧构建不得只告警后仍得出"当前源码通过"。
 * 各 E2E 在 buildFreshness 之后必须调用本函数，stale 即 CANNOT-RUN。 */
export function assertFreshBuild(freshness) {
  if (freshness?.staleBuild) {
    const detail = Array.isArray(freshness.staleReasons) && freshness.staleReasons.length
      ? freshness.staleReasons.join("；")
      : `exe=${freshness.exeMtime} < 最新源码/配置 ${freshness.newestSourceMtime} @${freshness.newestSource}`;
    throw new CannotRunError(
      `被测 exe 是陈旧构建（${detail}）。` +
      "拒绝以陈旧构建冒充当前源码验收；先运行 npm run build 再 npx tauri build --debug --no-bundle 后重跑本套件。"
    );
  }
}

/** 构建身份：把每份报告钉到确切产物上（HEAD、工作树、exe 哈希）。 */
export function buildIdentity(exePath) {
  const sha256 = (file) => {
    const hash = crypto.createHash("sha256");
    hash.update(fs.readFileSync(file));
    return hash.digest("hex");
  };
  const git = (args) => {
    const result = spawnSync("git", args, { cwd: repoRoot, encoding: "utf8" });
    return result.status === 0 ? String(result.stdout).trim() : "(git unavailable)";
  };
  const status = git(["status", "--porcelain"]);
  return {
    headSha: git(["rev-parse", "HEAD"]),
    headSubject: git(["log", "-1", "--format=%s"]),
    worktreeStatus: status === "" ? "clean" : status,
    exeSha256: sha256(exePath)
  };
}

/** 断电式重启模拟：强杀被测应用（套件顺序执行，同时只有一个实例）。 */
export function killAppProcess() {
  const result = spawnSync("taskkill", ["/F", "/IM", "ielts-author-studio.exe"], { encoding: "utf8" });
  return result.status === 0;
}

/**
 * 启动一次隔离的真实 Tauri 会话。
 * `runDirOverride`：重启场景（取消跨重启/中断恢复）复用上一次运行的目录
 * （同一 dataDir/publishDir），产品以相同数据重新启动。
 * @returns {Promise<{driver, runDir, dataDir, publishDir, pdfDir, driverStderr:()=>string, cleanup:()=>Promise<void>}>}
 */
export async function launchTauriApp({ exePath, pdfPath, keep = false, runPrefix = "tauri", runDirOverride = null, appEnv = {} }) {
  let runDir;
  if (runDirOverride) {
    runDir = runDirOverride;
    console.log(`[e2e:tauri] reusing run dir: ${runDir}`);
  } else {
    const runId = new Date().toISOString().replace(/[:.]/g, "-");
    runDir = path.join(repoRoot, "artifacts", "e2e-tauri", `run-${runPrefix}-${runId}`);
    for (const sub of ["appdata/roaming", "appdata/local", "appdata/data", "appdata/webview", "pdfs", "nas-library"]) {
      fs.mkdirSync(path.join(runDir, sub), { recursive: true });
    }
    fs.copyFileSync(pdfPath, path.join(runDir, "pdfs", path.basename(pdfPath)));
  }
  const pdfDir = path.join(runDir, "pdfs");
  // 目标目录名不能叫 "publish"：产品约定 destination 是题库根，名为 publish 的
  // 目录会被 normalize_nas_library_root 改写到父目录，破坏隔离断言。
  const publishDir = path.join(runDir, "nas-library");
  const dataDir = path.join(runDir, "appdata", "data");

  const driverDir = await ensureMsedgedriver();
  const port = await freePort();

  console.log(`[e2e:tauri] run dir: ${runDir}`);
  console.log(`[e2e:tauri] starting tauri-driver on :${port}`);
  const launchEnv = sanitizedAppEnv(process.env);
  const launchStartedAt = Date.now();
  const driverProcess = spawn("tauri-driver", ["--port", String(port)], {
    stdio: ["ignore", "pipe", "pipe"],
    env: {
      // tauri-driver 的环境会一路传给 msedgedriver 和被测 exe：先清掉已知会让 WebView2
      // 调试端点起不来的宿主残留（代理 / __COMPAT_LAYER / 坏掉的 PATH 项，见
      // findings.md F-WEBVIEW2-CDP-UNAVAILABLE-2026-09-22），与 CDP 通道一致。
      ...launchEnv,
      ...appEnv,
      PATH: `${driverDir}${path.delimiter}${launchEnv.PATH ?? ""}`,
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
  // spawn 失败（如 tauri-driver 未安装）必须以 cannot-run 报告，而不是挂起或当成产品失败。
  const spawnFailure = new Promise((_, reject) => {
    driverProcess.once("error", (error) => reject(new CannotRunError(`无法启动 tauri-driver：${error.message}`)));
  });

  const serverUrl = `http://127.0.0.1:${port}`;
  let driver;
  try {
    await Promise.race([waitForHttp(`${serverUrl}/status`, 20000), spawnFailure]);
    const capabilities = {
      // selenium-webdriver 4.x 远程会话强制要求 browserName；tauri-driver 模式下
      // 该值仅占位，实际被测对象由 tauri:options.application 指定。
      browserName: "wry",
      "tauri:options": { application: exePath }
    };
    driver = await new Builder().usingServer(serverUrl).withCapabilities(capabilities).build();
    const windowHandle = (await driver.getAllWindowHandles())[0];
    await driver.switchTo().window(windowHandle);
    // 会话健康检查：WebView2 若在建立后立即断开（例如被测 exe 是过期/不可用构建），
    // 任何后续步骤都会以 "invalid session id" 失败——那是环境问题，不能当成产品断言失败。
    try {
      await driver.getCurrentUrl();
    } catch (error) {
      throw new CannotRunError(
        `WebView2 会话建立后立即断开，无法驱动产品链路（应用可能未成功启动或 exe 为过期构建）：${error.message}`
      );
    }
  } catch (error) {
    try { await driver?.quit(); } catch {}
    try { driverProcess.kill(); } catch {}
    if (driverStderr.trim()) console.log(`[e2e:tauri] tauri-driver stderr (tail):\n${driverStderr.slice(-3000)}`);
    logLaunchDiagnostics(launchStartedAt, runDir);
    await probeWebView2Directly(exePath, runDir);
    if (!keep) {
      await sleep(1500);
      try { fs.rmSync(runDir, { recursive: true, force: true }); } catch {}
      console.log("[e2e:tauri] run dir cleaned after failed launch (use --keep to inspect artifacts)");
    }
    throw error;
  }

  return {
    driver,
    runDir,
    dataDir,
    publishDir,
    pdfDir,
    driverStderr: () => driverStderr,
    async cleanup({ removeRunDir = !keep } = {}) {
      if (driver) {
        try { await driver.quit(); } catch {}
      }
      driverProcess.kill();
      if (removeRunDir) {
        await sleep(1500);
        try { fs.rmSync(runDir, { recursive: true, force: true }); } catch {}
        console.log("[e2e:tauri] run dir cleaned (use --keep to inspect artifacts)");
      }
    }
  };
}

/** 步骤记录器：单步失败不中断后续步骤，但计入 verdict。 */
export function createStepRecorder({ artifacts, takeScreenshots = true }) {
  const steps = [];
  async function recordStep(driver, name, fn) {
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
      if (takeScreenshots && driver) {
        try {
          const shot = await driver.takeScreenshot();
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
  return { steps, recordStep };
}

/** 等待题库行进入稳定阶段（不再处于 Working）。 */
export async function waitForRowStage(driver, itemId, timeoutMs) {
  const selector = `[data-item-id="${itemId}"]`;
  const deadline = Date.now() + timeoutMs;
  let lastText = "";
  while (Date.now() < deadline) {
    const rows = await driver.findElements(By.css(selector));
    if (rows.length) {
      const text = (await rows[0].getText()).replace(/\s+/g, " ").trim();
      const stageClass = await rows[0].getAttribute("class");
      lastText = text;
      // 按行的 stage-* class 判断，不按文案：action_required/ready 现在都显示「识别完成」，
      // 文案会随产品措辞变化（见 libraryTypes STAGE_LABEL / LibraryItemRow）。
      if (/\bstage-(action_required|ready|published|failed)\b/.test(stageClass ?? "")) {
        return { stageClass, rowText: text };
      }
    }
    await sleep(1000);
  }
  throw new Error(`行 ${itemId} 未在限时内进入稳定阶段；最后文本：${lastText || "(无行)"}`);
}

/** 轮询题库行文本直到满足判据；题库列表是异步重载的，单次即时读取会与刷新竞态。 */
export async function waitForRowText(driver, itemId, predicate, timeoutMs) {
  const selector = `[data-item-id="${itemId}"]`;
  const deadline = Date.now() + timeoutMs;
  let lastText = "";
  while (Date.now() < deadline) {
    const rows = await driver.findElements(By.css(selector));
    if (rows.length) {
      lastText = (await rows[0].getText()).replace(/\s+/g, " ").trim();
      if (predicate(lastText)) return lastText;
    }
    await sleep(500);
  }
  throw new Error(`行 ${itemId} 未在限时内满足判据；最后文本：${lastText || "(无行)"}`);
}

/** 打开指定条目的工作区并等待工作区外壳出现。
 * 行点击与 processing 事件刷新竞态会产生 stale element（行重渲染），
 * 最多重试 3 次，每次重新定位——产品断言本身不变。 */
export async function openWorkspaceForItem(driver, itemId) {
  let lastError = null;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      const row = await driver.wait(
        until.elementLocated(By.css(`[data-item-id="${itemId}"] .library-row-main`)),
        15000
      );
      await row.click();
      await driver.wait(until.elementLocated(By.css('[data-testid="exam-workspace"]')), 15000);
      return itemId;
    } catch (error) {
      lastError = error;
      if (!/stale element/i.test(String(error?.message ?? error))) throw error;
      await sleep(1000);
    }
  }
  throw lastError;
}

/** 读当前工作区提示元素上的发布结论（`{ text, kind }`，没有则 `kind: null`）。 */
export async function readPublishNotice(driver) {
  const found = await driver.findElements(By.css(".workspace-notice"));
  if (!found.length) return { text: null, kind: null };
  const text = (await found[0].getText()).replace(/\s+/g, " ").trim() || null;
  return { text, kind: await found[0].getAttribute("data-publish-outcome") };
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
 *
 * 每次轮询都重新 `findElements`：React 会用新节点替换这条提示，缓存下来的元素引用会 stale。
 */
export async function publishAndReadOutcome(driver, { timeoutMs = 120000 } = {}) {
  const before = await readPublishNotice(driver);
  await driver.findElement(By.css('[data-testid="workspace-publish"]')).click();
  let settled = null;
  try {
    await driver.wait(async () => {
      const now = await readPublishNotice(driver);
      if (now.kind && now.text && now.text !== before.text) {
        settled = now;
        return true;
      }
      return false;
    }, timeoutMs);
  } catch {
    settled = null;
  }
  return {
    noticeBefore: before.text,
    text: settled?.text ?? null,
    kind: settled?.kind ?? null,
    timedOut: settled === null,
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

/** 导入目录内 PDF 并返回新行 id（集合差分，兼容乐观插入与事件刷新两种时序）。 */
export async function importPdfViaFolderHook(driver, timeoutMs = 30000) {
  const before = new Set(
    await driver.findElements(By.css('[data-testid="library-row"]'))
      .then(async (rows) => Promise.all(rows.map((row) => row.getAttribute("data-item-id"))))
  );
  await driver.findElement(By.css('[data-testid="library-import"]')).click();
  await driver.wait(until.elementLocated(By.css('[data-testid="import-drawer"]')), 10000);
  await driver.findElement(By.css('[data-testid="import-pick-folder"]')).click();
  await driver.wait(until.elementLocated(By.css('[data-testid="import-picked-files"] li')), 10000);
  await driver.findElement(By.css('[data-testid="import-start"]')).click();
  const deadline = Date.now() + timeoutMs;
  let newItemId = null;
  while (Date.now() < deadline) {
    const ids = await driver.findElements(By.css('[data-testid="library-row"]'))
      .then(async (rows) => Promise.all(rows.map((row) => row.getAttribute("data-item-id"))));
    newItemId = ids.find((id) => !before.has(id)) ?? null;
    if (newItemId) break;
    await sleep(1000);
  }
  if (!newItemId) throw new Error(`导入后未出现新题库行（导入前行数 ${before.size}）`);
  return { itemId: newItemId, priorRowCount: before.size };
}

/** 通过“选择文件”走真实导入 UI。文件本身由 PDF2TEST_AUTOMATION_SOURCE_FILES 提供，
 * 因此可覆盖 DOCX/TXT/MD，而不需要驱动系统文件对话框。 */
export async function importSourceViaFileHook(driver, timeoutMs = 30000) {
  const before = new Set(
    await driver.findElements(By.css('[data-testid="library-row"]'))
      .then(async (rows) => Promise.all(rows.map((row) => row.getAttribute("data-item-id"))))
  );
  await driver.findElement(By.css('[data-testid="library-import"]')).click();
  await driver.wait(until.elementLocated(By.css('[data-testid="import-drawer"]')), 10000);
  await driver.findElement(By.css('[data-testid="import-pick-files"]')).click();
  await driver.wait(until.elementLocated(By.css('[data-testid="import-picked-files"] li')), 10000);
  await driver.findElement(By.css('[data-testid="import-start"]')).click();
  const deadline = Date.now() + timeoutMs;
  let newItemId = null;
  while (Date.now() < deadline) {
    const ids = await driver.findElements(By.css('[data-testid="library-row"]'))
      .then(async (rows) => Promise.all(rows.map((row) => row.getAttribute("data-item-id"))));
    newItemId = ids.find((id) => !before.has(id)) ?? null;
    if (newItemId) break;
    await sleep(1000);
  }
  if (!newItemId) throw new Error(`导入后未出现新题库行（导入前行数 ${before.size}）`);
  return { itemId: newItemId, priorRowCount: before.size };
}

export function writeReport(runDir, report) {
  const file = path.join(runDir, "report.json");
  fs.writeFileSync(file, JSON.stringify(report, null, 2));
  console.log(`[e2e:tauri] verdict: ${report.verdict}`);
  console.log(`[e2e:tauri] report: ${file}`);
  return file;
}

/** 把 verdict 映射为退出码（见文件头约定）。 */
export function exitCodeForVerdict(verdict) {
  switch (verdict) {
    case "passed": return 0;
    case "failed": return 1;
    case "cannot-run": return 3;
    case "blocked": return 4;
    default: return 2;
  }
}

export function logCannotRun(error, prefix = "e2e:tauri") {
  console.error(`[${prefix}] CANNOT-RUN（未执行真实产品链路，不计为通过）: ${error.message}`);
}

export function logHarnessError(error, prefix = "e2e:tauri") {
  console.error(`[${prefix}] harness error: ${error}`);
}
