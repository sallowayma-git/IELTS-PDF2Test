import assert from "node:assert/strict";
import { existsSync, readFileSync, rmSync, statSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";

const reportPath = "tmp/phase5-real-pdf-acceptance/report.json";
const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
const runToken = randomUUID();
const startedAtMs = Date.now();
if (existsSync(reportPath)) rmSync(reportPath);
const test = spawnSync(cargo, [
  "test", "--manifest-path", "src-tauri/Cargo.toml", "--lib",
  "ielts_grammar::real_pdf_acceptance::phase5_real_pdf_edit_and_v2_export_round_trip",
  "--", "--exact", "--nocapture", "--test-threads=1"
], { env: { ...process.env, PHASE5_ACCEPTANCE_RUN_TOKEN: runToken }, stdio: "inherit" });

assert.ok(existsSync(reportPath), `Phase 5 real-PDF report missing: ${reportPath}`);
const report = JSON.parse(readFileSync(reportPath, "utf8"));
assert.equal(report.schemaVersion, "Phase5RealPdfEditorExportReportV1");
assert.equal(report.runToken, runToken);
assert.equal(report.passed, true);
assert.equal(report.fixtureId, "chili-peppers");
assert.ok(statSync(reportPath).mtimeMs + 1_000 >= startedAtMs, "Phase 5 real-PDF report is stale");
assert.equal(test.status, 0, "Phase 5 real-PDF cargo acceptance failed");

const phase5 = report.phase5;
assert.ok(Number(phase5.answerPatchCount) > 0, "Phase 5 acceptance must apply answer edits");
assert.ok(Number(phase5.revision) >= 2, "Phase 5 acceptance must create an edited revision and export it");
assert.equal(phase5.manifest?.schemaVersion, "AuthoringV2ExportReceiptV1");
assert.equal(phase5.authoringSchemaVersion, "IeltsAuthoringIRV2");
assert.equal(phase5.runtimeSchemaVersion, "ReadingExamSourceV2");
console.log(JSON.stringify({
  schemaVersion: report.schemaVersion,
  fixtureId: report.fixtureId,
  answerPatchCount: phase5.answerPatchCount,
  revision: phase5.revision,
  outputDir: phase5.outputDir,
  reportPath
}, null, 2));
console.log("Phase 5 real-PDF edit → immutable revision → ReadingExamSourceV2 export acceptance passed.");
