import { existsSync, readFileSync, readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";

const requiredFiles = [
  "src-tauri/src/ielts_grammar/mod.rs",
  "src-tauri/src/ielts_grammar/issue_codes.rs",
  "src-tauri/src/ielts_grammar/question_number.rs",
  "src-tauri/src/ielts_grammar/instruction_zone.rs",
  "src-tauri/src/ielts_grammar/instruction_signature.rs",
  "src-tauri/src/ielts_grammar/anchors.rs",
  "src-tauri/src/ielts_grammar/prompt_assembler.rs",
  "src-tauri/src/ielts_grammar/option_run.rs",
  "src-tauri/src/ielts_grammar/option_bank.rs",
  "src-tauri/src/ielts_grammar/completion.rs",
  "src-tauri/src/ielts_grammar/diagram.rs",
  "src-tauri/src/ielts_grammar/reading.rs",
  "src-tauri/src/ielts_grammar/answer_key.rs",
  "src-tauri/src/ielts_grammar/evidence.rs",
  "src-tauri/src/ielts_grammar/quality.rs",
  "src-tauri/src/reading_source_v2.rs",
  "src-tauri/src/runtime_compiler.rs",
  "src-tauri/src/authoring_commands.rs",
  "src-tauri/src/auto_pipeline.rs",
  "src-tauri/src/environment.rs",
  "fixtures/golden/synthetic/ielts/phase4-grammar-fixtures.json",
  "fixtures/golden/phase4-eight-pdf-acceptance.json",
  "scripts/verify-phase4-eight-pdf-acceptance.mjs",
  "src-tauri/src/ielts_grammar/real_pdf_acceptance.rs",
  "Files/IELTS_Document_Recognition_Phase_4_Completion_CN.md",
];

for (const file of requiredFiles) {
  if (!existsSync(file)) {
    throw new Error("Phase 4 required file missing: " + file);
  }
}

const fixture = JSON.parse(
  readFileSync("fixtures/golden/synthetic/ielts/phase4-grammar-fixtures.json", "utf8"),
);
if (fixture.schemaVersion !== "Phase4GrammarFixtureMatrixV1") {
  throw new Error("Phase 4 fixture schema version changed unexpectedly");
}
if (
  !Array.isArray(fixture.questionExpressions) ||
  fixture.questionExpressions.length < 8 ||
  !Array.isArray(fixture.instructionSignatures) ||
  fixture.instructionSignatures.length < 7 ||
  !Array.isArray(fixture.semanticScenarios) ||
  fixture.semanticScenarios.length < 2
) {
  throw new Error("Phase 4 fixture matrix is incomplete");
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
const v1Write = authoring.indexOf(
  'write_json(&job_dir(root, job_id).join("authoring-ir.json"), &ir)?;',
);
const v2Call = authoring.indexOf("write_authoring_v2_shadow(");
if (
  v1Write < 0 ||
  v2Call < 0 ||
  v2Call < v1Write ||
  !authoring.includes("if authoring_v2_shadow_enabled()") ||
  !authoring.includes("document-ir.json")
) {
  throw new Error("V1 authoring must remain authoritative before the opt-in V2 shadow branch");
}

const autoPipeline = readFileSync("src-tauri/src/auto_pipeline.rs", "utf8");
if (
  !autoPipeline.includes("quality_gate_v2_enabled()") ||
  !autoPipeline.includes("legacy_has_reliable_question_groups") ||
  !autoPipeline.includes("build_authoring_v2_shadow") ||
  !autoPipeline.includes('pointer("/quality/state")')
) {
  throw new Error("Reliability gate must use the V2 quality state only in the opt-in shadow branch");
}
if (
  !autoPipeline.includes("cloud_diagnostics_opted_in = !local_only && options.profile_id.is_some()") ||
  !autoPipeline.includes("let should_run_group_repair = !local_only && !main_source_is_pdf(&job);") ||
  (autoPipeline.match(/"diagnosticOnly"\.to_string\(\), json!\(true\)/g) ?? []).length < 2 ||
  (autoPipeline.match(/write_json\(&dir\.join\("document-ir\.json"\)/g) ?? []).length !== 1
) {
  throw new Error(
    "PDF cloud vision must be explicit opt-in, diagnostic-only, and must never overwrite authoritative DocumentIRV1",
  );
}

const issueCodes = readFileSync("src-tauri/src/ielts_grammar/issue_codes.rs", "utf8");
for (const code of [
  "QUESTION_RANGE_UNPARSED",
  "INSTRUCTION_SIGNATURE_UNRESOLVED",
  "PROMPT_EMPTY",
  "PROMPT_BOUNDARY_AMBIGUOUS",
  "OPTION_RUN_INCOMPLETE",
  "OPTION_BANK_MISSING",
  "CARDINALITY_SLOT_MISMATCH",
  "SLOT_HOST_MISSING",
  "SLOT_OUTSIDE_FIGURE",
  "WORD_LIMIT_UNPARSED",
  "ANSWER_KEY_MISSING_SLOT",
  "ANSWER_WORD_LIMIT_VIOLATION",
  "ANSWER_OPTION_NOT_IN_BANK",
  "ASSET_REFERENCE_MISSING",
  "ASSET_HASH_MISMATCH",
  "OPTION_ALPHABET_MISMATCH",
  "RESPONSE_GROUP_POLICY_MISMATCH",
  "HOTSPOT_GEOMETRY_INVALID",
  "ASSET_PATH_UNSAFE",
  "EXAM_ID_INVALID",
  "PASSAGE_CONTENT_MISSING",
  "AUTHORING_SCHEMA_INVALID",
  "RUNTIME_COMPILER_FAILED",
  "V1_COMPATIBILITY_COMPILER_FAILED",
  "PHYSICAL_SHADOW_MISSING",
  "TASK_ID_DUPLICATE",
  "RESPONSE_GROUP_ID_DUPLICATE",
  "SLOT_REFERENCE_MISSING",
  "SLOT_GROUP_ASSIGNMENT_INVALID",
  "OPTION_BANK_REFERENCE_MISSING",
  "PROVENANCE_MISSING",
  "ASSET_ID_DUPLICATE",
  "SIGNIFICANT_REGION_UNASSIGNED",
]) {
  if (!issueCodes.includes(`pub const ${code}`)) {
    throw new Error(`Stable issue code missing: ${code}`);
  }
}

const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";
const rustfmt = process.platform === "win32" ? "rustfmt.exe" : "rustfmt";
const rustFiles = readdirSync("src-tauri/src/ielts_grammar")
  .filter((name) => name.endsWith(".rs"))
  .map((name) => `src-tauri/src/ielts_grammar/${name}`)
  .concat([
    "src-tauri/src/reading_source_v2.rs",
    "src-tauri/src/runtime_compiler.rs",
    "src-tauri/src/authoring_commands.rs",
    "src-tauri/src/auto_pipeline.rs",
    "src-tauri/src/environment.rs",
  ]);
const format = spawnSync(rustfmt, ["--edition", "2021", "--check", ...rustFiles], {
  stdio: "inherit",
});
if (format.status !== 0) {
  process.exit(format.status ?? 1);
}

for (const testName of [
  "ielts_grammar",
  "auto_pipeline::tests",
  "checked_in_complex_fixture_projects_question_groups_and_slots",
  "reading_source_v2",
]) {
  const tests = spawnSync(
    cargo,
    ["test", "--manifest-path", "src-tauri/Cargo.toml", testName, "--", "--nocapture"],
    { stdio: "inherit" },
  );
  if (tests.status !== 0) {
    process.exit(tests.status ?? 1);
  }
}

const realPdfAcceptance = spawnSync(
  process.execPath,
  ["scripts/verify-phase4-eight-pdf-acceptance.mjs"],
  { stdio: "inherit" },
);
if (realPdfAcceptance.status !== 0) {
  process.exit(realPdfAcceptance.status ?? 1);
}

const nasRepo = path.resolve(
  String(process.env.IELTS_NAS_REPO || "").trim()
    || (process.platform === "win32" ? "E:\\NAS" : "../NAS"),
);
const nasPackage = path.join(nasRepo, "package.json");
const nasParityScript = path.join(
  nasRepo,
  "developer",
  "tests",
  "cross-repo",
  "reading-v2-vertical-slice.cjs",
);
if (!existsSync(nasPackage) || !existsSync(nasParityScript)) {
  throw new Error(
    `PR-06 NAS peer proof is required; set IELTS_NAS_REPO to a repository containing ${path.basename(nasParityScript)}`,
  );
}
const nasManifest = JSON.parse(readFileSync(nasPackage, "utf8"));
if (nasManifest.scripts?.["verify:cross-repo-reading-v2"] == null) {
  throw new Error("NAS peer is missing verify:cross-repo-reading-v2");
}
const npm = process.platform === "win32" ? "npm.cmd" : "npm";
const nasParity = spawnSync(npm, ["run", "verify:cross-repo-reading-v2"], {
  cwd: nasRepo,
  env: { ...process.env, IELTS_PDF2TEST_REPO: process.cwd() },
  stdio: "inherit",
  shell: process.platform === "win32",
});
if (nasParity.status !== 0) {
  process.exit(nasParity.status ?? 1);
}

console.log(
  "Phase 4 grammar verification passed: expression/signature matrix, " +
    fixture.questionExpressions.length +
    " expression cases, " +
    fixture.instructionSignatures.length +
    " signature cases, V1/V2 boundary, quality-gate tests, and NAS PR-06 parity are green.",
);
