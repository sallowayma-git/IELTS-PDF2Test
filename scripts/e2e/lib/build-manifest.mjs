/**
 * 构建清单（build manifest）：把 exe 与「生成它的源码与前端产物」用**内容哈希**钉在一起。
 *
 * 为什么需要它：mtime 只能证明「顺序」，不能证明「内容」。
 *   - 后端 agent 在本次构建之后碰一下 `scheduler.rs`，exe 相对最新源码就永远「陈旧」，
 *     于是并发写入把真正的陈旧问题淹没在噪声里；
 *   - 反过来，`npm run build` 重新产出一份**内容完全相同**的 dist（只有 mtime 变新），
 *     会让 mtime 判定报「exe 早于 dist」，而其实 exe 内嵌的前端一点没变。
 *
 * 清单记录三段的内容哈希：
 *   frontendInputs（src/** + 前端构建配置）→ dist（vite 产物）→ exe（tauri 产物）
 *   backendInputs（src-tauri/src/** + Cargo/tauri 配置）→ exe（cargo 产物）
 *
 * 判定规则（内容优先，mtime 兜底）：
 *   1. 若能按 exe 的 sha256 找到清单 => 逐段比对当前哈希。
 *      全部相等 ⇒ fresh（与 mtime 无关）；任一不等 ⇒ stale，并指出是哪一段漂移。
 *   2. 找不到清单（例如别人手动 build）=> 退回 mtime 三段链判定。
 *
 * 清单落盘在 `artifacts/build-manifests/<exeSha256>.json`（gitignored，本地产物）。
 */

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export const MANIFEST_DIR = path.join("artifacts", "build-manifests");

/** 前端输入：决定 `dist/**` 的内容。 */
export const FRONTEND_INPUT_DIRS = ["src"];
export const FRONTEND_INPUT_FILES = [
  "index.html",
  "vite.config.ts",
  "vitest.config.ts",
  "tsconfig.json",
  "tsconfig.node.json",
  "package.json",
  "package-lock.json",
];

/** 前端产物目录。 */
export const DIST_DIR = "dist";

/** 后端输入：决定 exe 里 Rust 那一半的内容。 */
export const BACKEND_INPUT_DIRS = ["src-tauri/src", "src-tauri/capabilities"];
export const BACKEND_INPUT_FILES = [
  "src-tauri/Cargo.toml",
  "src-tauri/Cargo.lock",
  "src-tauri/tauri.conf.json",
  "src-tauri/build.rs",
];

export function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

/** 收集目录/文件下的所有文件（递归）。 */
export function collectFiles(root, dirs, files) {
  const out = [];
  const walk = (p) => {
    if (!fs.existsSync(p)) return;
    const st = fs.statSync(p);
    if (st.isDirectory()) {
      for (const entry of fs.readdirSync(p)) walk(path.join(p, entry));
    } else {
      out.push(p);
    }
  };
  for (const d of dirs) walk(path.join(root, d));
  for (const f of files) walk(path.join(root, f));
  return out;
}

/**
 * 对一组输入求**内容哈希**：`sha256("relpath\0fileSha256\n" ...)`（按 relpath 排序）。
 * 返回 `{hash, fileCount, newestMtimeMs, newestPath}`。
 *
 * 为什么带上 relpath：否则「把 a.ts 删掉、把 b.ts 改名成 a.ts」这类替换会算出同一个哈希。
 */
export function hashTree(root, dirs, files) {
  const abs = collectFiles(root, dirs, files);
  const entries = abs.map((p) => {
    const st = fs.statSync(p);
    return {
      rel: path.relative(root, p).replace(/\\/g, "/"),
      sha: sha256File(p),
      mtimeMs: st.mtimeMs,
    };
  });
  entries.sort((a, b) => (a.rel < b.rel ? -1 : a.rel > b.rel ? 1 : 0));
  const h = crypto.createHash("sha256");
  for (const e of entries) h.update(`${e.rel}\0${e.sha}\n`);
  const newest = entries.reduce((acc, e) => (e.mtimeMs > acc.mtimeMs ? e : acc), { mtimeMs: 0, rel: null });
  return {
    hash: h.digest("hex"),
    fileCount: entries.length,
    newestMtimeMs: newest.mtimeMs,
    newestPath: newest.rel,
  };
}

export function frontendInputsHash(root) {
  return hashTree(root, FRONTEND_INPUT_DIRS, FRONTEND_INPUT_FILES);
}

export function backendInputsHash(root) {
  return hashTree(root, BACKEND_INPUT_DIRS, BACKEND_INPUT_FILES);
}

export function distHash(root) {
  return hashTree(root, [DIST_DIR], []);
}

export function manifestPathFor(root, exeSha256) {
  return path.join(root, MANIFEST_DIR, `${exeSha256}.json`);
}

/**
 * 生成一份清单。`exePath` 必须已存在。
 * `inputsAtBuildStart` 用于检测「构建期间输入被外部改动」——若与构建结束时不一致，
 * 清单会带 `inputsDriftedDuringBuild: true`，调用方应拒绝把它当成有效凭证。
 */
export function buildManifest({ root, exePath, inputsAtBuildStart = null, extra = {} }) {
  const exeSha256 = sha256File(exePath);
  const frontend = frontendInputsHash(root);
  const backend = backendInputsHash(root);
  const dist = distHash(root);
  const exeStat = fs.statSync(exePath);

  const drift = [];
  if (inputsAtBuildStart) {
    if (inputsAtBuildStart.frontend?.hash && inputsAtBuildStart.frontend.hash !== frontend.hash) {
      drift.push("frontendInputs");
    }
    if (inputsAtBuildStart.backend?.hash && inputsAtBuildStart.backend.hash !== backend.hash) {
      drift.push("backendInputs");
    }
  }

  return {
    schemaVersion: "BuildManifestV1",
    createdAt: new Date().toISOString(),
    exePath: path.relative(root, exePath).replace(/\\/g, "/"),
    exeSha256,
    exeMtime: new Date(exeStat.mtimeMs).toISOString(),
    frontendInputs: { hash: frontend.hash, fileCount: frontend.fileCount },
    dist: { hash: dist.hash, fileCount: dist.fileCount },
    backendInputs: { hash: backend.hash, fileCount: backend.fileCount },
    inputsDriftedDuringBuild: drift.length > 0,
    driftedSegments: drift,
    ...extra,
  };
}

export function saveManifest(root, manifest) {
  const file = manifestPathFor(root, manifest.exeSha256);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(manifest, null, 2));
  return file;
}

/** 读与某 exe 内容对应的清单；找不到返回 null。 */
export function loadManifestForExe(root, exeSha256) {
  const file = manifestPathFor(root, exeSha256);
  if (!fs.existsSync(file)) return null;
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return null;
  }
}

/**
 * 用清单核对当前工作树。返回 `{matched, diffs: [{segment, expected, actual}], manifest}`。
 * `segments` 可限定只核对某些段（用于「只豁免后端并发改动」）。
 */
export function compareManifest(root, manifest, segments = ["frontendInputs", "dist", "backendInputs"]) {
  const current = {
    frontendInputs: frontendInputsHash(root),
    dist: distHash(root),
    backendInputs: backendInputsHash(root),
  };
  const diffs = [];
  for (const seg of segments) {
    const expected = manifest[seg]?.hash ?? null;
    const actual = current[seg].hash;
    if (expected !== actual) {
      diffs.push({
        segment: seg,
        expected,
        actual,
        expectedFiles: manifest[seg]?.fileCount ?? null,
        actualFiles: current[seg].fileCount,
        newestPath: current[seg].newestPath,
      });
    }
  }
  return { matched: diffs.length === 0, diffs, current, manifest };
}
