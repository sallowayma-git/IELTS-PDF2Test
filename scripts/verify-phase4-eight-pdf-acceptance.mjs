import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { existsSync, readFileSync, rmSync, statSync } from "node:fs";
import { spawnSync } from "node:child_process";

const specPath = "fixtures/golden/phase4-eight-pdf-acceptance.json";
const manifestPath = "fixtures/golden/manifest.json";
const reportPath = "tmp/phase4-real-pdf-acceptance/report.json";
const spec = JSON.parse(readFileSync(specPath, "utf8"));
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

function isFreshAcceptanceReport(report, reportStat, runToken, startedAtMs) {
  return report?.schemaVersion === "Phase4EightPdfAcceptanceReportV1"
    && report?.runToken === runToken
    && reportStat.mtimeMs + 1_000 >= startedAtMs;
}

assert.equal(
  isFreshAcceptanceReport(
    { schemaVersion: "Phase4EightPdfAcceptanceReportV1", runToken: "old-run" },
    { mtimeMs: 0 },
    "current-run",
    Date.now(),
  ),
  false,
  "a stale acceptance report must never satisfy this run",
);

if (spec.schemaVersion !== "Phase4EightPdfAcceptanceV1") {
  throw new Error(`Unexpected Phase 4 acceptance spec: ${spec.schemaVersion}`);
}
if (!Array.isArray(spec.fixtureIds) || spec.fixtureIds.length !== 8) {
  throw new Error("Phase 4 acceptance spec must name exactly eight PDFs");
}

for (const fixtureId of spec.fixtureIds) {
  const fixture = manifest.fixtures?.find((candidate) => candidate.fixtureId === fixtureId);
  if (!fixture) throw new Error(`${fixtureId}: missing from golden manifest`);
  if (!String(fixture.sourcePath).toLowerCase().endsWith(".pdf")) {
    throw new Error(`${fixtureId}: acceptance source must be a PDF`);
  }
  for (const path of [fixture.sourcePath, fixture.metadataPath, fixture.baselinePath]) {
    if (!existsSync(path)) throw new Error(`${fixtureId}: required evidence missing: ${path}`);
  }
  const bytes = readFileSync(fixture.sourcePath);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  if (sha256 !== fixture.sha256 || statSync(fixture.sourcePath).size !== fixture.sizeBytes) {
    throw new Error(`${fixtureId}: real PDF identity does not match the frozen manifest`);
  }
  const metadata = JSON.parse(readFileSync(fixture.metadataPath, "utf8"));
  const baseline = JSON.parse(readFileSync(fixture.baselinePath, "utf8"));
  if (
    metadata.fixtureId !== fixtureId ||
    baseline.fixtureId !== fixtureId ||
    metadata.source?.sha256 !== fixture.sha256 ||
    baseline.source?.sha256 !== fixture.sha256 ||
    JSON.stringify(metadata.baseline?.observed) !== JSON.stringify(baseline.observed)
  ) {
    throw new Error(`${fixtureId}: manifest, metadata, and V1 baseline are not coherent`);
  }
}

const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
const runToken = randomUUID();
const startedAtMs = Date.now();
if (existsSync(reportPath)) {
  rmSync(reportPath);
}
const test = spawnSync(
  cargo,
  [
    "test",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--lib",
    "ielts_grammar::real_pdf_acceptance::phase4_eight_real_pdfs_reach_physical_authoring_quality_truth",
    "--",
    "--exact",
    "--nocapture",
    "--test-threads=1",
  ],
  {
    env: { ...process.env, PHASE4_ACCEPTANCE_RUN_TOKEN: runToken },
    stdio: "inherit",
  },
);

if (!existsSync(reportPath)) throw new Error(`Acceptance report missing: ${reportPath}`);
const report = JSON.parse(readFileSync(reportPath, "utf8"));
if (!isFreshAcceptanceReport(report, statSync(reportPath), runToken, startedAtMs)) {
  throw new Error(`Acceptance report is stale or belongs to another run: ${reportPath}`);
}
const failures = (report.fixtures ?? []).flatMap((fixture) => [
  ...(fixture.fatalError ? [`${fixture.fixtureId}:FATAL:${fixture.fatalError}`] : []),
  ...(fixture.checks ?? [])
    .filter((check) => check.passed !== true)
    .map((check) => `${fixture.fixtureId}:${check.code}`),
]);
console.log(
  JSON.stringify(
    {
      schemaVersion: report.schemaVersion,
      runToken: report.runToken,
      passed: report.passed,
      fixtureCount: report.fixtureCount,
      failureCount: failures.length,
      failures,
      reportPath,
    },
    null,
    2,
  ),
);

if (test.status !== 0) process.exit(test.status ?? 1);
if (report.passed !== true || report.fixtureCount !== 8) {
  throw new Error(`Phase 4 8-PDF acceptance did not pass; inspect ${reportPath}`);
}

console.log("Phase 4 8-PDF executable acceptance passed with real physical/V1/V2 evidence.");
