import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const requiredFiles = [
  "src-tauri/src/pdf_ingest/mod.rs",
  "src-tauri/src/pdf_ingest/coordinates.rs",
  "src-tauri/src/pdf_ingest/line_builder.rs",
  "src-tauri/src/pdf_ingest/region_builder.rs",
  "src-tauri/src/pdf_ingest/reading_order.rs",
  "src-tauri/src/pdf_ingest/table_detector.rs",
  "src-tauri/src/pdf_ingest/ocr_router.rs",
  "src-tauri/src/pdf_ingest/ocr_merge.rs",
  "src-tauri/src/pdf_ingest/compare_report.rs",
  "Files/IELTS_Document_Recognition_Phase_2_Completion_CN.md",
];

for (const file of requiredFiles) {
  try {
    readFileSync(file);
  } catch (error) {
    console.error(`Phase 2 shadow file missing: ${file}`);
    console.error(error.message);
    process.exit(1);
  }
}

const flags = readFileSync("src/config/featureFlags.ts", "utf8");
if (!/documentIrV2Shadow\s*:\s*false/.test(flags)) {
  throw new Error("documentIrV2Shadow must remain disabled by default");
}
if (!/pdfPerQuestionLlmRepair\s*:\s*false/.test(flags)) {
  throw new Error("pdfPerQuestionLlmRepair must remain disabled");
}

const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
const phase2RustFiles = [
  "src-tauri/src/authoring_commands.rs",
  "src-tauri/src/pdf_facts_shadow.rs",
  "src-tauri/src/pdf_ingest/mod.rs",
  "src-tauri/src/pdf_ingest/coordinates.rs",
  "src-tauri/src/pdf_ingest/line_builder.rs",
  "src-tauri/src/pdf_ingest/region_builder.rs",
  "src-tauri/src/pdf_ingest/reading_order.rs",
  "src-tauri/src/pdf_ingest/table_detector.rs",
  "src-tauri/src/pdf_ingest/ocr_router.rs",
  "src-tauri/src/pdf_ingest/ocr_merge.rs",
  "src-tauri/src/pdf_ingest/compare_report.rs",
];
const rustfmt = process.platform === "win32" ? "rustfmt.exe" : "rustfmt";
const format = spawnSync(rustfmt, ["--edition", "2021", "--check", ...phase2RustFiles], {
  stdio: "inherit",
});
if (format.error) throw format.error;
if (format.status !== 0) {
  process.exit(format.status ?? 1);
}

const rustTestBatches = [
  {
    filter: "pdf_facts_shadow",
    minimumCount: 19,
    requiredPrefixes: ["pdf_facts_shadow::tests::"],
    requiredNames: [
      "pdf_facts_shadow::tests::shadow_bundle_mid_commit_failure_restores_previous_bytes",
      "pdf_facts_shadow::tests::shadow_bundle_rollback_failure_is_fail_closed_and_preserves_backup_root",
    ],
  },
  {
    filter: "pdf_ingest",
    minimumCount: 8,
    requiredPrefixes: [
      "pdf_ingest::coordinates::tests::",
      "pdf_ingest::line_builder::tests::",
      "pdf_ingest::ocr_merge::tests::",
      "pdf_ingest::reading_order::tests::",
      "pdf_ingest::table_detector::tests::",
    ],
    requiredNames: [],
  },
];

let executedTestCount = 0;
for (const batch of rustTestBatches) {
  const listed = spawnSync(
    cargo,
    ["test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", batch.filter, "--", "--list"],
    { encoding: "utf8" },
  );
  if (listed.error) throw listed.error;
  if (listed.status !== 0) {
    process.stdout.write(listed.stdout ?? "");
    process.stderr.write(listed.stderr ?? "");
    process.exit(listed.status ?? 1);
  }
  const testNames = String(listed.stdout ?? "")
    .split(/\r?\n/)
    .filter((line) => line.endsWith(": test"))
    .map((line) => line.slice(0, -": test".length));
  if (testNames.length < batch.minimumCount) {
    throw new Error(
      `Phase 2 Rust filter ${batch.filter} matched ${testNames.length} tests; expected at least ${batch.minimumCount}`,
    );
  }
  for (const prefix of batch.requiredPrefixes) {
    if (!testNames.some((name) => name.startsWith(prefix))) {
      throw new Error(`Phase 2 Rust filter ${batch.filter} did not enumerate ${prefix} tests`);
    }
  }
  for (const name of batch.requiredNames) {
    if (!testNames.includes(name)) {
      throw new Error(`Phase 2 Rust filter ${batch.filter} did not enumerate required test ${name}`);
    }
  }

  const tests = spawnSync(
    cargo,
    [
      "test",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--lib",
      batch.filter,
      "--",
      "--nocapture",
    ],
    { stdio: "inherit" },
  );
  if (tests.error) throw tests.error;
  if (tests.status !== 0) {
    process.exit(tests.status ?? 1);
  }
  executedTestCount += testNames.length;
}

console.log(
  `Phase 2 shadow verification passed: ${executedTestCount} Rust tests executed, 0 failed; ` +
    "physical layers, assets, OCR routing, compare report, and overlay tests are green.",
);
