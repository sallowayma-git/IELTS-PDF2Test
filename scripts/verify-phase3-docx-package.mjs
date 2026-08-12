import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const requiredFiles = [
  "src-tauri/src/docx_ingest/mod.rs",
  "src-tauri/src/docx_ingest/package.rs",
  "Files/IELTS_Document_Recognition_Phase_3_C001_Completion_CN.md",
];

for (const file of requiredFiles) {
  try {
    readFileSync(file);
  } catch (error) {
    console.error(`Phase 3 C-001 file missing: ${file}`);
    console.error(error.message);
    process.exit(1);
  }
}

const flags = readFileSync("src/config/featureFlags.ts", "utf8");
for (const flag of [
  "documentIrV2",
  "authoringV2",
  "runtimeSourceV2",
  "nasPackageV2",
  "listeningV1",
  "pdfPerQuestionLlmRepair",
  "documentIrV2Shadow",
]) {
  if (!new RegExp(`${flag}\\s*:\\s*false`).test(flags)) {
    throw new Error(`${flag} must remain disabled by default`);
  }
}

const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
const rustfmt = process.platform === "win32" ? "rustfmt.exe" : "rustfmt";
const rustFiles = [
  "src-tauri/src/docx_ingest/mod.rs",
  "src-tauri/src/docx_ingest/package.rs",
  "src-tauri/src/parser.rs",
];

const format = spawnSync(rustfmt, ["--edition", "2021", "--check", ...rustFiles], {
  stdio: "inherit",
});
if (format.error) throw format.error;
if (format.status !== 0) {
  process.exit(format.status ?? 1);
}

const rustTestBatches = [
  { filter: "docx_ingest", minimumCount: 20, requiredPrefix: "docx_ingest::" },
  { filter: "docx_ooxml", minimumCount: 5, requiredPrefix: "tests::docx_ooxml_" },
  {
    filter: "complex_docx_fixture_reaches_authoring_ir",
    minimumCount: 1,
    requiredPrefix: "tests::complex_docx_fixture_reaches_authoring_ir",
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
      `Phase 3 Rust filter ${batch.filter} matched ${testNames.length} tests; expected at least ${batch.minimumCount}`,
    );
  }
  if (!testNames.some((name) => name.startsWith(batch.requiredPrefix))) {
    throw new Error(
      `Phase 3 Rust filter ${batch.filter} did not enumerate ${batch.requiredPrefix} tests`,
    );
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
  `Phase 3 C-001 verification passed: ${executedTestCount} Rust tests executed, 0 failed; ` +
    "bounded DOCX package/relationship reading and existing DOCX regressions are green.",
);
