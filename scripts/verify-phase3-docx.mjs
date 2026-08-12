import { existsSync, readFileSync, readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";

const requiredFiles = [
  "src-tauri/src/docx_ingest/mod.rs",
  "src-tauri/src/docx_ingest/package.rs",
  "src-tauri/src/docx_ingest/xml.rs",
  "src-tauri/src/docx_ingest/model.rs",
  "src-tauri/src/docx_ingest/styles.rs",
  "src-tauri/src/docx_ingest/numbering.rs",
  "src-tauri/src/docx_ingest/paragraphs.rs",
  "src-tauri/src/docx_ingest/tables.rs",
  "src-tauri/src/docx_ingest/drawings.rs",
  "src-tauri/src/docx_ingest/text_boxes.rs",
  "src-tauri/src/docx_ingest/smartart.rs",
  "src-tauri/src/docx_ingest/sections.rs",
  "src-tauri/src/docx_ingest/render_fallback.rs",
  "src-tauri/src/docx_facts_shadow.rs",
  "scripts/generate-phase3-docx-fixtures.py",
  "fixtures/golden/synthetic/docx/phase3-bad-word-fixtures.json",
  "fixtures/golden/synthetic/docx/render-assisted-two-column-options.docx",
  "fixtures/golden/synthetic/docx/render-assisted-two-column-options.provider-output.pdf",
  "Files/IELTS_Document_Recognition_Phase_3_C001_Completion_CN.md",
  "Files/IELTS_Document_Recognition_Phase_3_Completion_CN.md",
];

for (const file of requiredFiles) {
  if (!existsSync(file)) {
    throw new Error("Phase 3 required file missing: " + file);
  }
}

if (
  !readFileSync(
    "fixtures/golden/synthetic/docx/render-assisted-two-column-options.provider-output.pdf",
  )
    .subarray(0, 5)
    .equals(Buffer.from("%PDF-"))
) {
  throw new Error("Phase 3 render-assist provider output must be a real PDF fixture");
}

const fixtureMatrix = JSON.parse(
  readFileSync("fixtures/golden/synthetic/docx/phase3-bad-word-fixtures.json", "utf8"),
);
if (
  !Array.isArray(fixtureMatrix.fixtures) ||
  fixtureMatrix.fixtures.length < 10 ||
  fixtureMatrix.fixtures.length > 20
) {
  throw new Error("Phase 3 DOCX bad-fixture matrix must contain 10-20 entries");
}
const actualFixtures = readdirSync("fixtures/golden/synthetic/docx").filter((name) =>
  name.endsWith(".docx"),
);
const matrixSources = fixtureMatrix.fixtures.map((fixture) => fixture.source);
if (new Set(matrixSources).size !== fixtureMatrix.fixtures.length) {
  throw new Error("Every Phase 3 adversarial case must use a distinct DOCX package");
}
for (const fixture of fixtureMatrix.fixtures) {
  if (
    typeof fixture.source !== "string" ||
    !fixture.source.endsWith(".docx") ||
    !existsSync(`fixtures/golden/synthetic/docx/${fixture.source}`)
  ) {
    throw new Error(`Phase 3 fixture ${fixture.id} has no checked-in DOCX source`);
  }
}
if (actualFixtures.length < fixtureMatrix.fixtures.length) {
  throw new Error("Phase 3 DOCX directory must retain every adversarial package fixture");
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
  "authoringV2Shadow",
  "qualityGateV2",
]) {
  if (!new RegExp(flag + "\\s*:\\s*false").test(flags)) {
    throw new Error(flag + " must remain disabled by default");
  }
}

const authoring = readFileSync("src-tauri/src/authoring_commands.rs", "utf8");
if (
  !authoring.includes("document_ir_v2_shadow_enabled") ||
  !authoring.includes("write_docx_facts_shadow_with_v1")
) {
  throw new Error("DOCX shadow must remain an opt-in authoring-side branch");
}
if (!authoring.includes("document-ir.json")) {
  throw new Error("V1 document-ir.json path disappeared");
}
if (
  !authoring.includes("physical_shadow_matches_source") ||
  !authoring.includes("clear_document_shadow_success")
) {
  throw new Error("Phase 3 shadow lifecycle must reject stale source artifacts");
}

const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
const rustfmt = process.platform === "win32" ? "rustfmt.exe" : "rustfmt";
const rustFiles = requiredFiles
  .filter((file) => file.endsWith(".rs"))
  .filter((file) => file.startsWith("src-tauri/src/"));
const format = spawnSync(rustfmt, ["--edition", "2021", "--check", ...rustFiles], {
  stdio: "inherit",
});
if (format.error) throw format.error;
if (format.status !== 0) {
  process.exit(format.status ?? 1);
}

const rustTestBatches = [
  {
    filter: "docx_ingest",
    minimumCount: 20,
    requiredPrefix: "docx_ingest::",
    requiredNames: [],
  },
  {
    filter: "docx_facts_shadow",
    minimumCount: 9,
    requiredPrefix: "docx_facts_shadow::tests::",
    requiredNames: [
      "docx_facts_shadow::tests::docx_shadow_mid_commit_failure_restores_previous_bytes",
      "docx_facts_shadow::tests::docx_shadow_rollback_failure_is_fail_closed_and_preserves_backup_root",
    ],
  },
  {
    filter: "docx_ooxml",
    minimumCount: 5,
    requiredPrefix: "tests::docx_ooxml_",
    requiredNames: [],
  },
  {
    filter: "complex_docx_fixture_reaches_authoring_ir",
    minimumCount: 1,
    requiredPrefix: "tests::complex_docx_fixture_reaches_authoring_ir",
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
      `Phase 3 Rust filter ${batch.filter} matched ${testNames.length} tests; expected at least ${batch.minimumCount}`,
    );
  }
  if (!testNames.some((name) => name.startsWith(batch.requiredPrefix))) {
    throw new Error(
      `Phase 3 Rust filter ${batch.filter} did not enumerate ${batch.requiredPrefix} tests`,
    );
  }
  for (const name of batch.requiredNames) {
    if (!testNames.includes(name)) {
      throw new Error(`Phase 3 Rust filter ${batch.filter} did not enumerate required test ${name}`);
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
  `Phase 3 DOCX verification passed: ${executedTestCount} Rust tests executed, 0 failed; ` +
    "C-002-C-009 rich-structure shadow, " +
    fixtureMatrix.fixtures.length +
    " bad-Word cases, and V1/V2 boundary checks are green.",
);
