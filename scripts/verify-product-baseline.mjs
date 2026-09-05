#!/usr/bin/env node
// Product surface baseline (plan section 18, P0-T01).
//
// Snapshots the things the convergence work is allowed to change on purpose and nothing else:
// frontend routes, page components wired into App, registered Tauri commands, SQLite tables,
// and feature-flag defaults. Any PR can print the drift instead of arguing about it.
//
//   node scripts/verify-product-baseline.mjs           # verify against the recorded baseline
//   node scripts/verify-product-baseline.mjs --update  # accept the current surface as baseline
//   node scripts/verify-product-baseline.mjs --print   # just print the current surface
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require_ = createRequire(import.meta.url);
const { execFileSync } = require_("node:child_process");

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE = path.join(repoRoot, "fixtures", "product-baseline.json");

function read(relPath) {
  return fs.readFileSync(path.join(repoRoot, relPath), "utf8");
}

function unique(values) {
  return [...new Set(values)].sort();
}

function routeNames() {
  const source = read("src/app/router.ts");
  const block = /export type RouteName\s*=([\s\S]*?);/.exec(source);
  if (!block) throw new Error("RouteName union not found in src/app/router.ts");
  return unique([...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]));
}

/** 仍被应用装配的 `src/pages/*` 组件。收敛期这些页面从 App.tsx 移到 legacyRoutes.tsx，
 *  两个文件都要扫，否则「页面数」会假性归零。 */
function appPages() {
  const files = [
    "src/app/App.tsx",
    "src/app/legacyRoutes.tsx",
    "src/features/settings/SettingsPage.tsx",
    "src/features/library/LibraryPage.tsx",
    "src/features/editor/ExamWorkspacePage.tsx"
  ];
  const names = [];
  for (const file of files) {
    if (!fs.existsSync(path.join(repoRoot, file))) continue;
    const source = read(file);
    for (const match of source.matchAll(/^import \{ ([A-Z][A-Za-z0-9]*) \} from "(?:\.\.\/)+pages\/[^"]+";$/gm)) {
      names.push(match[1]);
    }
  }
  return unique(names);
}

/** 新产品表面的模块数：feature 目录 + exam-canvas + styles 分层。 */
function productModules() {
  const roots = ["src/features", "src/exam-canvas", "src/styles"];
  const out = {};
  for (const root of roots) {
    const full = path.join(repoRoot, root);
    if (!fs.existsSync(full)) {
      out[root] = 0;
      continue;
    }
    let count = 0;
    const walk = (dir) => {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        if (entry.isDirectory()) walk(path.join(dir, entry.name));
        else count += 1;
      }
    };
    walk(full);
    out[root] = count;
  }
  return out;
}

function tauriCommands() {
  const source = read("src-tauri/src/lib.rs");
  const block = /generate_handler!\s*\[([\s\S]*?)\]/.exec(source);
  if (!block) throw new Error("generate_handler! block not found in src-tauri/src/lib.rs");
  return unique(
    block[1]
      .split(",")
      .map((entry) => entry.replace(/\/\/.*$/gm, "").trim())
      .filter(Boolean)
      .map((entry) => entry.split("::").pop())
  );
}

function frontendCommandCalls() {
  const source = read("src/api/tauriCommands.ts");
  return unique([...source.matchAll(/command<[^>]*>\(\s*"([a-z0-9_]+)"|command\(\s*"([a-z0-9_]+)"/g)].map((m) => m[1] ?? m[2]));
}

function sqliteTables() {
  const source = read("src-tauri/src/db.rs");
  return unique([...source.matchAll(/CREATE TABLE (?:IF NOT EXISTS )?([A-Za-z0-9_]+)/g)].map((m) => m[1]));
}

function featureFlagDefaults() {
  const source = read("src/config/featureFlags.ts");
  const out = {};
  for (const match of source.matchAll(/DEFAULT_(PHASE\d)_FEATURE_FLAGS[^{]*\{([\s\S]*?)\n\}\);/g)) {
    const flags = {};
    for (const pair of match[2].matchAll(/^\s*([A-Za-z0-9]+):\s*(true|false)/gm)) flags[pair[1]] = pair[2] === "true";
    out[match[1]] = flags;
  }
  return out;
}

function fileSizes() {
  const targets = [
    "src/styles.css",
    "src/app/App.tsx",
    "src/app/router.ts",
    "src/components/AppShell.tsx",
    "src/exam-canvas/ExamCanvas.tsx",
    "src/pages/UnifiedPreview.tsx",
    "src/pages/StructuredAuthoringEditorV2.tsx",
    "src/services/devFallbackBackend.ts",
    "src-tauri/src/lib.rs",
    "src-tauri/src/authoring_pipeline.rs",
    "src-tauri/src/auto_pipeline.rs",
    "src-tauri/src/ielts_grammar/mod.rs"
  ];
  const out = {};
  for (const target of targets) {
    const full = path.join(repoRoot, target);
    out[target] = fs.existsSync(full) ? fs.statSync(full).size : null;
  }
  return out;
}

/** 本轮起始 commit（M0-T2 / 计划 P0-T01）：基线必须能回答「从哪个提交开始」。 */
function gitCommitSha() {
  try {
    return execFileSync("git", ["rev-parse", "HEAD"], { cwd: repoRoot, encoding: "utf8" }).trim();
  } catch {
    return null;
  }
}

function sha256(content) {
  return crypto.createHash("sha256").update(content).digest("hex");
}

/** 合同 schema 的内容寻址 hash：路径 + 内容一起进 hash，任何一端漂移都会改变它。 */
function schemaHash() {
  const roots = ["contracts", path.join("src-tauri", "src", "schema"), "src/types"];
  const files = [];
  const walk = (dir) => {
    const full = path.join(repoRoot, dir);
    if (!fs.existsSync(full)) return;
    for (const entry of fs.readdirSync(full, { withFileTypes: true })) {
      const rel = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(rel);
      else if (/\.(json|rs|ts)$/.test(entry.name)) files.push(rel);
    }
  };
  for (const root of roots) walk(root);
  files.sort();
  const combined = files
    .map((rel) => `${rel}:${sha256(fs.readFileSync(path.join(repoRoot, rel)))}`)
    .join("\n");
  return { files: files.length, sha256: sha256(combined) };
}

/** 公开语料清单（M0-T2）：E2E 与回归实际使用的 PDF fixture，逐个记 sha256。
 *  私有 8 份 Reading 语料不入库（git-ignored），只记录是否就绪（test_support.rs 的判定）。 */
function corpusManifest() {
  const dir = path.join(repoRoot, "fixtures", "golden", "synthetic", "pdf");
  const files = fs.existsSync(dir)
    ? fs.readdirSync(dir).filter((name) => name.endsWith(".pdf")).sort()
        .map((name) => ({ name, sha256: sha256(fs.readFileSync(path.join(dir, name))) }))
    : [];
  const privateCorpus = path.join(repoRoot, "fixtures", "private");
  const privateCorpusReady = fs.existsSync(privateCorpus)
    ? fs.readdirSync(privateCorpus, { recursive: true }).some((name) => String(name).toLowerCase().endsWith(".pdf"))
    : false;
  return {
    publicSyntheticPdf: files,
    e2ePrimaryFixture: "fixtures/golden/synthetic/pdf/pdf-two-column.pdf",
    privateCorpusReady
  };
}

function collect() {
  return {
    commitSha: gitCommitSha(),
    schemaHash: schemaHash(),
    corpus: corpusManifest(),
    routeNames: routeNames(),
    appPages: appPages(),
    tauriCommands: tauriCommands(),
    frontendCommandCalls: frontendCommandCalls(),
    sqliteTables: sqliteTables(),
    featureFlagDefaults: featureFlagDefaults(),
    productModules: productModules(),
    fileSizes: fileSizes()
  };
}

function diffLists(name, before, after) {
  const removed = before.filter((v) => !after.includes(v));
  const added = after.filter((v) => !before.includes(v));
  const lines = [];
  if (removed.length) lines.push(`  ${name}: removed ${removed.join(", ")}`);
  if (added.length) lines.push(`  ${name}: added ${added.join(", ")}`);
  return lines;
}

function diffFlags(before, after) {
  const lines = [];
  for (const phase of unique([...Object.keys(before), ...Object.keys(after)])) {
    const a = before[phase] ?? {};
    const b = after[phase] ?? {};
    for (const flag of unique([...Object.keys(a), ...Object.keys(b)])) {
      if (a[flag] !== b[flag]) lines.push(`  featureFlagDefaults.${phase}.${flag}: ${a[flag]} -> ${b[flag]}`);
    }
  }
  return lines;
}

function diffSizes(before, after) {
  const lines = [];
  for (const file of unique([...Object.keys(before), ...Object.keys(after)])) {
    const a = before[file];
    const b = after[file];
    if (a === b) continue;
    if (a === null || a === undefined) lines.push(`  fileSizes.${file}: created (${b} B)`);
    else if (b === null || b === undefined) lines.push(`  fileSizes.${file}: deleted (was ${a} B)`);
    else {
      const delta = b - a;
      const pct = a ? ((delta / a) * 100).toFixed(1) : "n/a";
      lines.push(`  fileSizes.${file}: ${a} -> ${b} B (${delta > 0 ? "+" : ""}${delta}, ${pct}%)`);
    }
  }
  return lines;
}

function diffSurface(baseline, current) {
  return [
    ...diffLists("routeNames", baseline.routeNames ?? [], current.routeNames),
    ...diffLists("appPages", baseline.appPages ?? [], current.appPages),
    ...diffLists("tauriCommands", baseline.tauriCommands ?? [], current.tauriCommands),
    ...diffLists("frontendCommandCalls", baseline.frontendCommandCalls ?? [], current.frontendCommandCalls),
    ...diffLists("sqliteTables", baseline.sqliteTables ?? [], current.sqliteTables),
    ...diffFlags(baseline.featureFlagDefaults ?? {}, current.featureFlagDefaults ?? {}),
    ...diffSizes(baseline.fileSizes ?? {}, current.fileSizes ?? {}),
    ...Object.keys({ ...(baseline.productModules ?? {}), ...current.productModules })
      .filter((root) => (baseline.productModules ?? {})[root] !== current.productModules[root])
      .map((root) => `  productModules.${root}: ${(baseline.productModules ?? {})[root] ?? 0} -> ${current.productModules[root]} files`),
    ...(baseline.commitSha !== undefined && baseline.commitSha !== current.commitSha
      ? [`  commitSha: ${baseline.commitSha} -> ${current.commitSha}`]
      : []),
    ...(baseline.schemaHash && baseline.schemaHash.sha256 !== current.schemaHash.sha256
      ? [`  schemaHash: ${baseline.schemaHash.sha256.slice(0, 12)} (${baseline.schemaHash.files} files) -> ${current.schemaHash.sha256.slice(0, 12)} (${current.schemaHash.files} files)`]
      : []),
    ...(baseline.corpus && JSON.stringify(baseline.corpus) !== JSON.stringify(current.corpus)
      ? ["  corpus: fixture manifest changed"]
      : [])
  ];
}

function main() {
  const current = collect();
  if (process.argv.includes("--print")) {
    console.log(JSON.stringify(current, null, 2));
    return 0;
  }
  if (process.argv.includes("--update") || !fs.existsSync(BASELINE)) {
    const existed = fs.existsSync(BASELINE);
    // M0-T2：重录基线必须说明产品行为变化，不能以重录快照代替验收。
    // 第一次创建不要求 reason；对已存在基线的重录，若无 --reason 且上一次校验存在漂移则拒绝。
    const reasonIndex = process.argv.indexOf("--reason");
    const reason = reasonIndex >= 0 ? process.argv[reasonIndex + 1] : null;
    let driftExisted = false;
    if (existed) {
      try {
        const previous = JSON.parse(fs.readFileSync(BASELINE, "utf8")).surface;
        driftExisted = diffSurface(previous, collect()).length > 0;
      } catch {}
      if (driftExisted && !reason) {
        console.error("refusing to re-record: the product surface drifted and no --reason was given.");
        console.error("基线更新必须同时说明产品行为变化（M0-T2）。用法：--update --reason \"<行为变化说明>\"");
        return 2;
      }
    }
    const previousDoc = existed ? JSON.parse(fs.readFileSync(BASELINE, "utf8")) : null;
    const changeLog = previousDoc?.changeLog ?? [];
    if (existed) {
      changeLog.push({
        at: new Date().toISOString(),
        commit: current.commitSha,
        reason: reason ?? "initial re-record (no drift)",
        driftSummary: driftExisted ? diffSurface(JSON.parse(fs.readFileSync(BASELINE, "utf8")).surface, current).slice(0, 20) : []
      });
    }
    fs.mkdirSync(path.dirname(BASELINE), { recursive: true });
    fs.writeFileSync(BASELINE, JSON.stringify({ recordedAt: new Date().toISOString(), surface: current, changeLog }, null, 2) + "\n");
    console.log(`${existed ? "updated" : "created"} ${path.relative(repoRoot, BASELINE)}${reason ? ` (reason: ${reason})` : ""}`);
    if (!existed && !process.argv.includes("--update")) console.log("(no baseline existed; recorded the current surface)");
    return 0;
  }
  const baseline = JSON.parse(fs.readFileSync(BASELINE, "utf8")).surface;
  const lines = diffSurface(baseline, current);
  const changeLog = JSON.parse(fs.readFileSync(BASELINE, "utf8")).changeLog ?? [];
  const lastChange = changeLog[changeLog.length - 1];
  console.log(`product-baseline: commit ${String(current.commitSha ?? "(unknown)").slice(0, 12)}, schema ${current.schemaHash.sha256.slice(0, 12)} (${current.schemaHash.files} files), corpus ${current.corpus.publicSyntheticPdf.length} pdf, routes ${current.routeNames.length}, pages ${current.appPages.length}, commands ${current.tauriCommands.length}, tables ${current.sqliteTables.length}`);
  if (lastChange) console.log(`last baseline update: ${lastChange.at} @ ${String(lastChange.commit ?? "").slice(0, 12)} :: ${lastChange.reason}`);
  if (!lines.length) {
    console.log("no drift from the recorded product surface.");
    return 0;
  }
  console.log("\nproduct surface drift (expected during convergence; review each line):");
  for (const line of lines) console.log(line);
  console.log("\nIf every line is intended, re-record with: node scripts/verify-product-baseline.mjs --update --reason \"<行为变化说明>\"");
  return process.argv.includes("--strict") ? 1 : 0;
}

process.exit(main());
