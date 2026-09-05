// Shared Chrome/CDP + Vite dev-server harness for UI verification scripts.
//
// Extracted so that `scripts/ui/layout-matrix.mjs` and `scripts/e2e/*.mjs` do not each
// re-implement browser launching. Deliberately dependency-free: it uses Node's global
// WebSocket and the same Chrome/Edge discovery order as `sidecars/ui-flow-e2e`.
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const DEFAULT_BASE_URL = "http://127.0.0.1:1420";
const VITE_BIN = path.join(repoRoot, "node_modules", "vite", "bin", "vite.js");

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function waitFor(description, fn, { timeoutMs = 15000, intervalMs = 150 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const result = await fn();
      if (result) return result;
    } catch (error) {
      lastError = error;
    }
    await sleep(intervalMs);
  }
  throw new Error(`${description} timed out${lastError ? `: ${lastError.message}` : ""}`);
}

function httpGet(url) {
  return new Promise((resolve, reject) => {
    const request = http.get(url, (response) => {
      response.resume();
      resolve(response.statusCode ?? 0);
    });
    request.on("error", reject);
    request.setTimeout(2500, () => request.destroy(new Error("timeout")));
  });
}

function fetchJson(url, options = {}) {
  return new Promise((resolve, reject) => {
    const request = http.request(url, { method: options.method ?? "GET" }, (response) => {
      const chunks = [];
      response.on("data", (chunk) => chunks.push(chunk));
      response.on("end", () => {
        try {
          resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")));
        } catch (error) {
          reject(error);
        }
      });
    });
    request.on("error", reject);
    request.setTimeout(5000, () => request.destroy(new Error("timeout")));
    request.end();
  });
}

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : undefined;
      server.close(() => (port ? resolve(port) : reject(new Error("failed_to_allocate_port"))));
    });
    server.on("error", reject);
  });
}

export function findChrome(chromePathArg) {
  const programFiles = [process.env.PROGRAMFILES, process.env["PROGRAMFILES(X86)"], process.env.LOCALAPPDATA].filter(Boolean);
  const windowsCandidates = programFiles.flatMap((baseDir) => [
    path.join(baseDir, "Google", "Chrome", "Application", "chrome.exe"),
    path.join(baseDir, "Chromium", "Application", "chrome.exe"),
    path.join(baseDir, "Microsoft", "Edge", "Application", "msedge.exe")
  ]);
  const candidates = [
    chromePathArg,
    process.env.CHROME_PATH,
    process.env.EDGE_PATH,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    ...windowsCandidates
  ].filter(Boolean);
  const found = candidates.find((candidate) => fs.existsSync(candidate));
  if (!found) throw new Error("No Chrome/Chromium/Edge executable found. Set CHROME_PATH or pass --chrome-path.");
  return found;
}

export async function launchChrome({ headful = false, chromePath, verbose = false, windowSize = "1440,1000" } = {}) {
  const port = await freePort();
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "pdf2test-ui-"));
  const args = [
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${userDataDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-extensions",
    "--disable-background-networking",
    "--disable-sync",
    "--disable-gpu",
    `--window-size=${windowSize}`,
    headful ? "" : "--headless=new",
    "about:blank"
  ].filter(Boolean);
  const proc = spawn(findChrome(chromePath), args, { stdio: ["ignore", "pipe", "pipe"] });
  proc.stderr.on("data", (chunk) => {
    const text = chunk.toString();
    if (verbose && !text.includes("DevTools listening")) process.stderr.write(`[chrome] ${text}`);
  });
  await waitFor("Chrome DevTools", async () => {
    try {
      return await fetchJson(`http://127.0.0.1:${port}/json/version`);
    } catch {
      return false;
    }
  }, { timeoutMs: 20000 });
  return {
    process: proc,
    port,
    userDataDir,
    close() {
      try {
        proc.kill();
      } catch {
        // Chrome may already be gone; not a failure.
      }
      try {
        fs.rmSync(userDataDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 150 });
      } catch {
        // Chrome can hold profile files briefly after exit; not a failure.
      }
    }
  };
}

export class Cdp {
  constructor(wsUrl) {
    this.ws = new WebSocket(wsUrl);
    this.nextId = 1;
    this.pending = new Map();
    this.opened = new Promise((resolve, reject) => {
      this.ws.addEventListener("open", resolve, { once: true });
      this.ws.addEventListener("error", (event) => reject(event.error ?? new Error("websocket_error")), { once: true });
    });
    this.ws.addEventListener("message", (event) => {
      const message = JSON.parse(event.data.toString());
      if (message.id && this.pending.has(message.id)) {
        const { resolve, reject } = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (message.error) reject(new Error(message.error.message));
        else resolve(message.result);
      }
    });
  }

  async send(method, params = {}) {
    await this.opened;
    const id = this.nextId++;
    const result = new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
    this.ws.send(JSON.stringify({ id, method, params }));
    return result;
  }

  close() {
    this.ws.close();
  }
}

export async function createPage(port) {
  let target;
  const url = `http://127.0.0.1:${port}/json/new?${encodeURIComponent("about:blank")}`;
  try {
    target = await fetchJson(url, { method: "PUT" });
  } catch {
    target = await fetchJson(url);
  }
  const cdp = new Cdp(target.webSocketDebuggerUrl);
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  return { cdp, targetId: target.id };
}

export async function evaluate(cdp, expression, { awaitPromise = true } = {}) {
  const result = await cdp.send("Runtime.evaluate", { expression, awaitPromise, returnByValue: true, userGesture: true });
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.text || result.exceptionDetails.exception?.description || "runtime_exception");
  }
  return result.result.value;
}

/** Resize the emulated viewport. `deviceScaleFactor` models Windows display scaling
 *  (1.25 = 125%, 1.5 = 150%) so the layout matrix can reproduce scaling defects. */
export async function setViewport(cdp, { width, height, deviceScaleFactor = 1 }) {
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width,
    height,
    deviceScaleFactor,
    mobile: false,
    screenWidth: width,
    screenHeight: height
  });
}

export async function navigate(cdp, url, { readySelector = "#root", timeoutMs = 45000 } = {}) {
  await cdp.send("Page.navigate", { url });
  await waitFor(`page load ${url}`, async () =>
    evaluate(cdp, `document.readyState !== "loading" && Boolean(document.querySelector(${JSON.stringify(readySelector)}))`),
    { timeoutMs });
  await waitFor(`app content ${url}`, async () =>
    evaluate(cdp, `Boolean(document.getElementById("root")?.textContent?.trim())`), { timeoutMs });
}

/** Hash-route navigation without a full reload, so injected fixture state survives. */
export async function gotoHash(cdp, baseUrl, hash, { settleMs = 400 } = {}) {
  await evaluate(cdp, `(() => { window.location.hash = ${JSON.stringify(hash)}; return true; })()`);
  await sleep(settleMs);
}

export async function screenshot(cdp, filePath) {
  const result = await cdp.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, Buffer.from(result.data, "base64"));
  return filePath;
}

export async function ensureVite(baseUrl = DEFAULT_BASE_URL, { noStartServer = false, verbose = false } = {}) {
  try {
    const status = await httpGet(baseUrl);
    if (status && status < 500) return { started: false, close() {} };
  } catch {
    // Not running yet; fall through and start it.
  }
  if (noStartServer) throw new Error(`Vite dev server is not reachable at ${baseUrl}`);
  if (!fs.existsSync(VITE_BIN)) throw new Error(`Vite binary is missing at ${VITE_BIN}. Run npm install first.`);
  const proc = spawn(process.execPath, [VITE_BIN, "--host", "127.0.0.1"], {
    cwd: repoRoot,
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, BROWSER: "none" }
  });
  proc.stdout.on("data", (chunk) => {
    if (verbose) process.stdout.write(`[vite] ${chunk}`);
  });
  proc.stderr.on("data", (chunk) => process.stderr.write(`[vite] ${chunk}`));
  await waitFor("vite dev server", async () => {
    try {
      const status = await httpGet(baseUrl);
      return Boolean(status && status < 500);
    } catch {
      return false;
    }
  }, { timeoutMs: 90000, intervalMs: 300 });
  return {
    started: true,
    close() {
      try {
        proc.kill();
      } catch {
        // already exited
      }
    }
  };
}

export function argValue(name, fallback = undefined) {
  const index = process.argv.indexOf(name);
  if (index >= 0 && process.argv[index + 1]) return process.argv[index + 1];
  return fallback;
}

export function hasFlag(name) {
  return process.argv.includes(name);
}
