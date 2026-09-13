#!/usr/bin/env node
// G2-T02：Phase 4 指标 runner 包装（report-only）。
// 职责：写入运行身份（runToken/HEAD/source tree），驱动 Rust 指标测试，
// 把报告从 tmp/ 归档到 artifacts/phase4-metrics/ 并附上构建身份。
// 不设置验收门槛：阈值在报告中"记录不强制"（recognition blocker gate 不开启）。
// synthetic 语料上的结果只能作开发证据，不能替代真实语料验收。

import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const runToken = crypto.randomUUID();
const git = (args) => {
  const result = spawnSync("git", args, { cwd: repoRoot, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${result.stderr}`);
  return result.stdout.trim();
};
const sha256 = (file) =>
  crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");

const headSha = git(["rev-parse", "HEAD"]);
const sourceTree = git(["write-tree"]);
const headTree = git(["rev-parse", "HEAD^{tree}"]);
const status = git(["status", "--porcelain"]);
if (headSha === "(git unavailable)") throw new Error("git unavailable");

console.log(`[phase4-metrics] runToken=${runToken}`);
console.log(`[phase4-metrics] HEAD=${headSha} tree=${sourceTree} tree==headTree=${sourceTree === headTree}`);

const manifestPath = path.join(repoRoot, "fixtures", "golden", "manifest.json");
const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
const manifestSha256 = sha256(manifestPath);

const testResult = spawnSync(
  "cargo",
  ["test", "--lib", "phase4_metrics_report_only", "--", "--nocapture"],
  { cwd: path.join(repoRoot, "src-tauri"), encoding: "utf8", env: {
    ...process.env,
    PHASE4_METRICS_RUN_TOKEN: runToken,
    PHASE4_METRICS_GIT_HEAD: headSha
  }, maxBuffer: 64 * 1024 * 1024 }
);
if (testResult.status !== 0) {
  console.error(testResult.stdout?.slice(-4000));
  console.error(testResult.stderr?.slice(-4000));
  throw new Error("phase4 metrics cargo test failed");
}

const tmpReportPath = path.join(repoRoot, "tmp", "phase4-metrics", "phase4-metrics-report.json");
const tmpReport = JSON.parse(fs.readFileSync(tmpReportPath, "utf8"));
const report = {
  ...tmpReport,
  identity: {
    headSha,
    headSubject: git(["log", "-1", "--format=%s"]),
    sourceTreeIdentity: { gitWriteTree: sourceTree, equalsHeadTree: sourceTree === headTree },
    worktreeStatus: status || "clean",
    goldenManifestSha256: manifestSha256
  },
  archivedAt: new Date().toISOString()
};

const outDir = path.join(repoRoot, "artifacts", "phase4-metrics");
fs.mkdirSync(outDir, { recursive: true });
const outPath = path.join(outDir, `report-${runToken}.json`);
fs.writeFileSync(outPath, JSON.stringify(report, null, 2));
fs.writeFileSync(path.join(outDir, "latest.json"), JSON.stringify({
  runToken, headSha, sourceTree, reportPath: path.relative(repoRoot, outPath),
  aggregate: report.aggregate, corpus: report.corpus, policy: report.policy
}, null, 2));

console.log(`[phase4-metrics] report: ${path.relative(repoRoot, outPath)}`);
console.log(`[phase4-metrics] corpus: ${JSON.stringify(report.corpus)}`);
console.log(`[phase4-metrics] aggregate: ${JSON.stringify(report.aggregate)}`);
console.log("[phase4-metrics] policy=report-only（阈值记录不强制；synthetic 结果仅开发证据）");
