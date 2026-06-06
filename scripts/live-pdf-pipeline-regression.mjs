import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultOutDir = path.join(repoRoot, "tmp", "live-pdf-pipeline-regression");

const args = parseArgs(process.argv.slice(2));
if (args.help === true || args.h === true) printUsageAndExit(0);

const pdfDirArg = args.pdfDir ?? args["pdf-dir"];
const pdfListArg = args.pdfList ?? args["pdf-list"];
if (!pdfDirArg && !pdfListArg) {
  console.error("[live-pdf-regression] --pdf-dir or --pdf-list is required.");
  printUsageAndExit(2);
}

const pdfDir = pdfDirArg ? path.resolve(pdfDirArg) : "";
const pdfList = pdfListArg ? path.resolve(pdfListArg) : "";
const outDir = path.resolve(args.outDir ?? args["out-dir"] ?? defaultOutDir);
const sampleSize = Number(args.sample ?? 100);
const seed = String(args.seed ?? Date.now());
const baseUrl = String(args.baseUrl ?? args["base-url"] ?? process.env.EPIC8_LIVE_LLM_BASE_URL ?? "");
const model = String(args.model ?? process.env.EPIC8_LIVE_LLM_MODEL ?? "");
const apiKey = String(args.apiKey ?? args["api-key"] ?? process.env.EPIC8_LIVE_LLM_API_KEY ?? "");
const timeoutMs = Number(args.timeoutMs ?? args["timeout-ms"] ?? 600000);
const skipBuild = args.skipBuild === true || args["skip-build"] === true;
const strict = args.strict === true;
const cli = path.resolve(args.cli ?? path.join(repoRoot, "src-tauri", "target", "debug", cliBinaryName()));

if (pdfDir) assertReadableDirectory(pdfDir, "--pdf-dir");
if (pdfList && (!fs.existsSync(pdfList) || !fs.statSync(pdfList).isFile())) {
  console.error(`[live-pdf-regression] --pdf-list is not a readable file: ${pdfList}`);
  process.exit(2);
}
if (!baseUrl || !model || !apiKey) {
  console.error("[live-pdf-regression] missing live LLM config. Provide --base-url, --model, and --api-key or EPIC8_LIVE_LLM_* env vars.");
  process.exit(2);
}
fs.mkdirSync(outDir, { recursive: true });

if (!skipBuild || !fs.existsSync(cli)) {
  const build = spawnSync("cargo", ["build", "--manifest-path", path.join(repoRoot, "src-tauri", "Cargo.toml")], {
    cwd: repoRoot,
    stdio: "inherit"
  });
  if (build.status !== 0) process.exit(build.status ?? 1);
}

const allPdfs = pdfList
  ? fs.readFileSync(pdfList, "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => path.resolve(line))
  : fs.readdirSync(pdfDir)
    .filter((name) => name.toLowerCase().endsWith(".pdf"))
    .map((name) => path.join(pdfDir, name))
    .sort((left, right) => left.localeCompare(right));
for (const pdf of allPdfs) {
  if (!fs.existsSync(pdf) || !fs.statSync(pdf).isFile()) {
    console.error(`[live-pdf-regression] PDF is not readable: ${pdf}`);
    process.exit(2);
  }
}
const selected = shuffle(allPdfs, seed).slice(0, Math.min(sampleSize, allPdfs.length));

const results = [];
for (const [index, pdfPath] of selected.entries()) {
  const id = String(index + 1).padStart(3, "0");
  const outputPath = path.join(outDir, `${id}-${safeName(path.basename(pdfPath))}.json`);
  const appRoot = path.join(outDir, "app-roots", id);
  console.log(`[live-pdf-regression] ${index + 1}/${selected.length} ${path.basename(pdfPath)}`);
  const startedAt = Date.now();
  const generated = spawnSync(cli, [
    "--run-auto-pipeline",
    pdfPath,
    "--out",
    outputPath,
    "--app-root",
    appRoot,
    "--llm-base-url",
    baseUrl,
    "--llm-model",
    model,
    "--llm-api-key",
    apiKey
  ], {
    cwd: repoRoot,
    encoding: "utf8",
    timeout: timeoutMs,
    maxBuffer: 1024 * 1024 * 20,
    env: {
      ...process.env,
      EPIC8_LIVE_LLM_BASE_URL: baseUrl,
      EPIC8_LIVE_LLM_MODEL: model,
      EPIC8_LIVE_LLM_API_KEY: apiKey,
      EPIC8_ENABLE_CLOUD_PDF_VISION: process.env.EPIC8_ENABLE_CLOUD_PDF_VISION ?? "1"
    }
  });
  const elapsedMs = Date.now() - startedAt;
  if (generated.status !== 0 || generated.error) {
    scrubSecrets(appRoot);
    results.push({
      pdf: pdfPath,
      ok: false,
      elapsedMs,
      category: generated.error?.code === "ETIMEDOUT" ? "timeout" : "cli_failed",
      error: generated.error?.message ?? (generated.stderr.trim() || generated.stdout.trim() || `exit_${generated.status}`)
    });
    continue;
  }
  let payload;
  try {
    payload = JSON.parse(fs.readFileSync(outputPath, "utf8"));
  } catch (error) {
    scrubSecrets(appRoot);
    results.push({
      pdf: pdfPath,
      ok: false,
      elapsedMs,
      category: "invalid_output",
      error: error.message,
      outputPath
    });
    continue;
  }
  scrubSecrets(appRoot);
  results.push(summarizePayload(pdfPath, outputPath, elapsedMs, payload));
}

const report = buildReport({
  seed,
  sampleSize,
  pdfDir,
  pdfList,
  outDir,
  model,
  baseUrl: redactBaseUrl(baseUrl),
  selected,
  results
});
const reportPath = path.join(outDir, "report.json");
fs.writeFileSync(reportPath, JSON.stringify(report, null, 2));
console.log(JSON.stringify({
  seed,
  sampled: selected.length,
  okCount: report.okCount,
  failCount: report.failCount,
  timeoutCount: report.categoryCounts.timeout ?? 0,
  emptyAnswerJobs: report.emptyAnswerJobCount,
  visionAttempted: report.visionAttemptedCount,
  visionApplied: report.visionAppliedCount,
  cloudAttempted: report.cloudAttemptedCount,
  cloudPassed: report.cloudPassedCount,
  reportPath
}, null, 2));

if (strict && report.failCount) process.exit(1);

function summarizePayload(pdfPath, outputPath, elapsedMs, payload) {
  const report = payload.report ?? {};
  const authoring = payload.authoringIr ?? {};
  const groups = Array.isArray(authoring.groups) ? authoring.groups : [];
  const questions = groups.flatMap((group) => Array.isArray(group.questions) ? group.questions : []);
  const emptyAnswerQuestionIds = questions
    .filter((question) => answerIsEmpty(question.answer))
    .map((question) => question.id)
    .filter(Boolean);
  const auditIssues = Array.isArray(authoring.audit?.issues) ? authoring.audit.issues : [];
  const parserWarnings = Array.isArray(report.parser?.warnings) ? report.parser.warnings : [];
  const vision = report.parser?.visionAnswerExtraction ?? {};
  const cloud = report.quality?.cloudComparison ?? {};
  const status = payload.job?.status ?? report.status ?? "unknown";
  const currentStep = payload.job?.currentStep ?? report.currentStep ?? "unknown";
  const hasAuthoringOutput = Boolean(payload.job?.jobId && authoring.schemaVersion);
  return {
    pdf: pdfPath,
    ok: hasAuthoringOutput,
    elapsedMs,
    category: hasAuthoringOutput ? "pipeline_completed" : "missing_authoring_output",
    outputPath,
    jobId: payload.job?.jobId,
    status,
    currentStep,
    groupCount: groups.length,
    questionCount: questions.length,
    zeroQuestionOutput: hasAuthoringOutput && questions.length === 0,
    emptyAnswerCount: emptyAnswerQuestionIds.length,
    emptyAnswerQuestionIds: emptyAnswerQuestionIds.slice(0, 30),
    remainingReviewItems: report.authoring?.remainingReviewItems ?? null,
    validationPassed: report.validationPassed ?? false,
    userStatus: report.userStatus ?? null,
    nextRoute: report.nextRoute ?? null,
    parserWarningCount: parserWarnings.length,
    parserWarnings: parserWarnings.slice(0, 8),
    visionAnswerExtraction: {
      attempted: Boolean(vision.attempted),
      applied: Boolean(vision.applied),
      answerCount: Number(vision.answerCount ?? 0),
      missingQuestionCount: Array.isArray(vision.missingQuestionIds) ? vision.missingQuestionIds.length : null,
      failure: vision.failure ?? null
    },
    cloudComparison: {
      attempted: Boolean(cloud.attempted),
      passed: Boolean(cloud.passed),
      warningCount: Number(cloud.warningCount ?? 0),
      failure: cloud.failure ?? null,
      issueCount: Array.isArray(cloud.issues) ? cloud.issues.length : 0
    },
    auditSummaryKinds: auditIssues
      .map((issue) => typeof issue === "object" && issue ? issue.kind : null)
      .filter(Boolean)
  };
}

function buildReport({ seed, sampleSize, pdfDir, pdfList, outDir, model, baseUrl, selected, results }) {
  const categoryCounts = countBy(results, (result) => result.category ?? "unknown");
  return {
    schemaVersion: "LivePdfPipelineRegressionReportV1",
    generatedAt: new Date().toISOString(),
    seed,
    requestedSampleSize: sampleSize,
    sampledCount: selected.length,
    pdfDir,
    pdfList,
    outDir,
    liveConfig: {
      baseUrl,
      model,
      apiKey: "redacted"
    },
    selected,
    okCount: results.filter((result) => result.ok).length,
    failCount: results.filter((result) => !result.ok).length,
    emptyAnswerJobCount: results.filter((result) => Number(result.emptyAnswerCount ?? 0) > 0).length,
    zeroQuestionJobCount: results.filter((result) => result.zeroQuestionOutput).length,
    totalEmptyAnswers: results.reduce((sum, result) => sum + Number(result.emptyAnswerCount ?? 0), 0),
    visionAttemptedCount: results.filter((result) => result.visionAnswerExtraction?.attempted).length,
    visionAppliedCount: results.filter((result) => result.visionAnswerExtraction?.applied).length,
    cloudAttemptedCount: results.filter((result) => result.cloudComparison?.attempted).length,
    cloudPassedCount: results.filter((result) => result.cloudComparison?.passed).length,
    categoryCounts,
    statusCounts: countBy(results, (result) => result.status ?? "unknown"),
    currentStepCounts: countBy(results, (result) => result.currentStep ?? "unknown"),
    results
  };
}

function answerIsEmpty(value) {
  if (Array.isArray(value)) return value.length === 0 || value.every((item) => !String(item ?? "").trim());
  return !String(value ?? "").trim();
}

function countBy(items, keyFn) {
  const counts = {};
  for (const item of items) {
    const key = String(keyFn(item));
    counts[key] = (counts[key] ?? 0) + 1;
  }
  return counts;
}

function redactBaseUrl(value) {
  try {
    const parsed = new URL(value);
    return `${parsed.origin}${parsed.pathname.replace(/\/$/, "") || ""}`;
  } catch {
    return "configured";
  }
}

function scrubSecrets(appRoot) {
  const secretDir = path.join(appRoot, "config", "secrets");
  fs.rmSync(secretDir, { recursive: true, force: true });
  const profilePath = path.join(appRoot, "config", "llm-profiles.json");
  if (!fs.existsSync(profilePath)) return;
  try {
    const profiles = JSON.parse(fs.readFileSync(profilePath, "utf8"));
    const scrubbed = Array.isArray(profiles) ? profiles.map((profile) => ({
      ...profile,
      apiKeySecretRef: profile.apiKeySecretRef ? "redacted" : profile.apiKeySecretRef
    })) : profiles;
    fs.writeFileSync(profilePath, JSON.stringify(scrubbed, null, 2));
  } catch {
    fs.rmSync(profilePath, { force: true });
  }
}

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (!arg.startsWith("--")) continue;
    const key = arg.slice(2);
    const next = argv[index + 1];
    if (!next || next.startsWith("--")) parsed[key] = true;
    else {
      parsed[key] = next;
      index += 1;
    }
  }
  return parsed;
}

function printUsageAndExit(code) {
  const usage = [
    "usage: node scripts/live-pdf-pipeline-regression.mjs (--pdf-dir <dir>|--pdf-list <file>) --base-url <url> --model <model> --api-key <key> [options]",
    "",
    "Options:",
    "  --sample <n>         Number of PDFs to sample. Default: 100.",
    "  --out-dir <dir>      Report/output directory. Default: tmp/live-pdf-pipeline-regression.",
    "  --seed <value>       Deterministic sample seed. Default: current timestamp.",
    "  --timeout-ms <n>     Per-PDF timeout. Default: 600000.",
    "  --cli <path>         Existing ielts-author-studio CLI binary to run.",
    "  --skip-build         Skip cargo build when --cli/default binary already exists.",
    "  --strict             Exit non-zero when any sampled pipeline invocation fails."
  ].join("\n");
  (code === 0 ? console.log : console.error)(usage);
  process.exit(code);
}

function assertReadableDirectory(dir, label) {
  if (!fs.existsSync(dir) || !fs.statSync(dir).isDirectory()) {
    console.error(`[live-pdf-regression] ${label} is not a readable directory: ${dir}`);
    process.exit(2);
  }
}

function cliBinaryName() {
  return process.platform === "win32" ? "ielts-author-studio.exe" : "ielts-author-studio";
}

function safeName(value) {
  return value.replace(/[^a-z0-9._-]+/gi, "-").replace(/^-+|-+$/g, "").slice(0, 96);
}

function seededRandom(seedValue) {
  let hash = 2166136261;
  for (const ch of String(seedValue)) {
    hash ^= ch.charCodeAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return () => {
    hash += 0x6d2b79f5;
    let value = hash;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

function shuffle(items, seedValue) {
  const random = seededRandom(seedValue);
  const copy = items.slice();
  for (let index = copy.length - 1; index > 0; index -= 1) {
    const swap = Math.floor(random() * (index + 1));
    [copy[index], copy[swap]] = [copy[swap], copy[index]];
  }
  return copy;
}
