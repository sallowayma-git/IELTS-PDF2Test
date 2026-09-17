#!/usr/bin/env node
/**
 * 可归因构建：前端 → dist → exe，并产出一份**内容哈希清单**把三者钉在一起。
 *
 * 为什么不用「一条 tauri build」了事：
 *   `tauri build --no-bundle` 的 `beforeBuildCommand` 会自己跑一次 `npm run build`，
 *   但那条路径**不产出任何可核对的凭证**——事后没人能证明 exe 内嵌的 dist 到底是哪一版。
 *   本脚本显式分两步构建、逐步取内容哈希，最后写 `artifacts/build-manifests/<exeSha>.json`。
 *
 * 构建期间若前端/后端输入被外部改动，清单会标记 `inputsDriftedDuringBuild`，
 * 并以退出码 4 退出——这种二进制不可归因，不能拿去验收。
 *
 * 用法：
 *   node scripts/e2e/build-app.mjs [--no-frontend]
 * 退出码：0 = 构建成功且清单有效；1 = 构建失败；4 = 构建期间输入漂移（不可归因）。
 */

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import {
  backendInputsHash,
  buildManifest,
  frontendInputsHash,
  saveManifest,
} from "./lib/build-manifest.mjs";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const skipFrontend = process.argv.includes("--no-frontend");

const nodeBinDir = path.dirname(process.execPath);
const env = { ...process.env, PATH: `${nodeBinDir}${path.delimiter}${process.env.PATH ?? ""}` };

/** 跑一条命令，日志落盘，**显式检查退出码**（不接管道，避免管道吞掉退出码）。 */
function run(label, command, args, logFile) {
  const started = Date.now();
  console.log(`[build] ${label}: ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    env,
    encoding: "utf8",
    shell: true,
    maxBuffer: 64 * 1024 * 1024,
  });
  const out = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  fs.mkdirSync(path.dirname(logFile), { recursive: true });
  fs.writeFileSync(logFile, out);
  const ms = Date.now() - started;
  if (result.status !== 0) {
    console.error(`[build] ${label} FAILED exit=${result.status} (${ms}ms) log=${logFile}`);
    console.error(out.slice(-4000));
    return { ok: false, status: result.status, ms, logFile };
  }
  console.log(`[build] ${label} ok (${ms}ms) log=${logFile}`);
  return { ok: true, status: 0, ms, logFile };
}

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const logDir = path.join(repoRoot, "artifacts", "build-logs", stamp);

// ── 0. 构建前的输入指纹（用于检测构建期间的并发写入）──
const inputsAtBuildStart = {
  frontend: frontendInputsHash(repoRoot),
  backend: backendInputsHash(repoRoot),
};
console.log(
  `[build] inputs@start frontend=${inputsAtBuildStart.frontend.hash.slice(0, 12)}(${inputsAtBuildStart.frontend.fileCount}) ` +
    `backend=${inputsAtBuildStart.backend.hash.slice(0, 12)}(${inputsAtBuildStart.backend.fileCount})`
);

// ── 0.5 让 vite 的 `emptyDir` 无事可做 ──
// 本机安全删除护栏对**单回合的批量删除**有配额（约 50 个文件）。一次完整 vite 构建会产出
// 50+ 个 assets，`emptyDir(dist/assets)` 就会撞配额并让构建失败；更糟的是失败前它已经
// 删掉一部分，dist 停在半删状态（实测：54 个文件删到剩 6 个才抛）。
//
// 解法不是去关护栏（那是保护用户文件的机制，不该为构建让路），而是**让这次构建根本
// 不需要批量删除**：把旧 dist 整体 `rename` 到 tmp/ 下 —— 重命名是一次目录项操作，
// 不是删除 —— vite 便在一个全新的空目录上构建，`emptyDir` 面对空目录无事可做。
//
// 副产品是好事：dist 内容 100% 来自本次构建，清单里的 `dist` 哈希因此更干净，
// 不会混进上一版残留的 asset。
const distDir = path.join(repoRoot, "dist");
if (!skipFrontend && fs.existsSync(distDir)) {
  const parked = path.join(repoRoot, "tmp", `dist-prev-${stamp}`);
  try {
    fs.mkdirSync(path.dirname(parked), { recursive: true });
    fs.renameSync(distDir, parked);
    console.log(`[build] 旧 dist 已挪到 ${path.relative(repoRoot, parked)}（避开 emptyDir 的批量删除配额）`);
  } catch (error) {
    // 挪不动（被占用等）时不要硬来：交给 vite 自己处理，失败原因如实报出来。
    console.warn(`[build] 旧 dist 挪不动（${error instanceof Error ? error.message : String(error)}），交给 vite 自行处理`);
  }
}

// ── 1. 前端：tsc --noEmit && vite build ──
if (!skipFrontend) {
  const fe = run("frontend", "npm", ["run", "build"], path.join(logDir, "frontend.log"));
  if (!fe.ok) process.exit(1);
}

// ── 2. exe：不覆盖 beforeBuildCommand，但 dist 已由上一步产出 ──
// 说明：这里**故意**把 beforeBuildCommand 置空，避免它再跑一次 npm build 并把 dist
// 的 mtime 推新（那会让 mtime 判定与内容判定打架）。dist 的新鲜度由本脚本第 1 步与
// 清单哈希保证，不由 tauri 的重跑保证。
//
// `--config` 用**文件路径**而不是内联 JSON 字符串：经 shell 传内联 JSON 时引号会被
// 剥掉（`{build:{...}}` 无法解析）。文件路径没有引号，任何 shell 下都稳定。
const overrideConfig = path.join(logDir, "tauri-build-config.json");
fs.mkdirSync(logDir, { recursive: true });
fs.writeFileSync(overrideConfig, JSON.stringify({ build: { beforeBuildCommand: "" } }, null, 2));
const exe = run(
  "tauri",
  "npx",
  ["tauri", "build", "--debug", "--no-bundle", "--config", overrideConfig],
  path.join(logDir, "tauri.log")
);
if (!exe.ok) process.exit(1);

if (!fs.existsSync(exePath)) {
  console.error(`[build] exe 未产出：${exePath}`);
  process.exit(1);
}

// ── 3. 清单 ──
const manifest = buildManifest({
  root: repoRoot,
  exePath,
  inputsAtBuildStart,
  extra: { buildLogs: path.relative(repoRoot, logDir).replace(/\\/g, "/") },
});
const manifestFile = saveManifest(repoRoot, manifest);

console.log(`[build] exe=${manifest.exePath} sha256=${manifest.exeSha256}`);
console.log(
  `[build] manifest frontendInputs=${manifest.frontendInputs.hash.slice(0, 12)} ` +
    `dist=${manifest.dist.hash.slice(0, 12)} backendInputs=${manifest.backendInputs.hash.slice(0, 12)}`
);
console.log(`[build] manifest=${manifestFile}`);

if (manifest.inputsDriftedDuringBuild) {
  console.error(
    `[build] 构建期间输入漂移（${manifest.driftedSegments.join(", ")}）：该 exe 不可归因，` +
      "请等写入方静止后重跑本脚本。"
  );
  process.exit(4);
}
process.exit(0);
