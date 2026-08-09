import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultManifestPath = path.join(repoRoot, "fixtures", "golden", "manifest.json");
const defaultReportPath = path.join(repoRoot, "tmp", "phase0-golden", "verification.json");

const args = parseArgs(process.argv.slice(2));
const command = args._[0] ?? "verify";
const manifestPath = path.resolve(args.manifest ?? defaultManifestPath);

if (args.help === true || args.h === true) {
  printUsageAndExit(0);
}

if (command === "verify") {
  verifyManifest();
} else if (command === "capture") {
  captureBaselines();
} else {
  console.error(`[phase0-golden] unknown command: ${command}`);
  printUsageAndExit(2);
}

function verifyManifest() {
  const manifest = readJson(manifestPath);
  const errors = [];
  const warnings = [];
  const fixtureIds = new Set();
  const fixturesById = new Map();
  const metrics = validateMetricsContract(manifest, errors);
  const legacyReference = validateLegacyReference(manifest, errors, warnings);

  if (manifest.schemaVersion !== "GoldenCorpusManifestV1") {
    errors.push(`unsupported manifest schema: ${manifest.schemaVersion ?? "missing"}`);
  }

  for (const [name, value] of Object.entries(manifest.featureFlags ?? {})) {
    if (value !== false) errors.push(`phase0 feature flag must be false: ${name}`);
  }

  for (const fixture of manifest.fixtures ?? []) {
    validateUniqueId(fixtureIds, fixture.fixtureId, errors);
    fixturesById.set(fixture.fixtureId, fixture);
    if (fixture.status !== "available") {
      errors.push(`workspace fixture must be available: ${fixture.fixtureId}`);
      continue;
    }

    const sourcePath = resolveRepoPath(fixture.sourcePath);
    const metadataPath = resolveRepoPath(fixture.metadataPath);
    const baselinePath = resolveRepoPath(fixture.baselinePath);
    if (!sourcePath || !fs.existsSync(sourcePath) || !fs.statSync(sourcePath).isFile()) {
      errors.push(`source missing: ${fixture.fixtureId} -> ${fixture.sourcePath}`);
      continue;
    }
    if (!metadataPath || !fs.existsSync(metadataPath)) {
      errors.push(`metadata missing: ${fixture.fixtureId} -> ${fixture.metadataPath}`);
    }
    if (!baselinePath || !fs.existsSync(baselinePath)) {
      errors.push(`V1 baseline missing: ${fixture.fixtureId} -> ${fixture.baselinePath}`);
    }

    const actualHash = sha256File(sourcePath);
    const actualSize = fs.statSync(sourcePath).size;
    if (fixture.sha256 !== actualHash) errors.push(`manifest source hash mismatch: ${fixture.fixtureId}`);
    if (fixture.sizeBytes !== actualSize) errors.push(`manifest source size mismatch: ${fixture.fixtureId}`);
    let metadata = null;
    let baseline = null;
    if (metadataPath && fs.existsSync(metadataPath)) {
      metadata = readJson(metadataPath);
      validateMetadata(metadata, fixture, actualHash, actualSize, errors);
    }
    if (baselinePath && fs.existsSync(baselinePath)) {
      baseline = readJson(baselinePath);
      validateBaseline(baseline, fixture, actualHash, actualSize, errors);
    }
    if (metadata?.baseline?.v1Path && metadata.baseline.v1Path !== fixture.baselinePath) {
      errors.push(`metadata baseline path mismatch: ${fixture.fixtureId}`);
    }
    if (metadata?.baseline?.observed && baseline?.observed) {
      for (const [key, expected] of Object.entries(metadata.baseline.observed)) {
        if (!sameJsonValue(baseline.observed[key], expected)) {
          errors.push(`baseline observed mismatch: ${fixture.fixtureId}.${key} expected ${expected} got ${baseline.observed[key]}`);
        }
      }
    }
  }

  for (const required of manifest.requiredPrivateCorpus ?? []) {
    if (required.status !== "available") continue;
    const fixture = fixturesById.get(required.fixtureId);
    if (!fixture) {
      errors.push(`required private fixture is not registered: ${required.fixtureId}`);
      continue;
    }
    if (fixture.sourcePath !== required.sourcePath) {
      errors.push(`required private source path mismatch: ${required.fixtureId}`);
    }
  }

  const privateMissing = (manifest.requiredPrivateCorpus ?? [])
    .filter((fixture) => fixture.status !== "available")
    .map((fixture) => fixture.fixtureId);
  const syntheticPending = (manifest.plannedSyntheticFixtures ?? [])
    .filter((fixture) => fixture.status !== "available")
    .map((fixture) => fixture.fixtureId);
  if (privateMissing.length) {
    warnings.push(`private corpus sources are not present: ${privateMissing.length}`);
  }
  if (syntheticPending.length) {
    warnings.push(`planned synthetic fixtures are not present: ${syntheticPending.length}`);
  }
  const repositories = inspectRepositoryBaselines(manifest, warnings);
  const currentRepoCommit = repositories.find((repository) => repository.repositoryId === "pdf2test")?.currentCommit ?? readCurrentRepoCommit();
  const baselineRepoCommit = manifest.baseline?.repoCommit ?? repositories.find((repository) => repository.repositoryId === "pdf2test")?.baselineCommit ?? null;

  const report = {
    schemaVersion: "Phase0GoldenVerificationReportV1",
    manifestPath: toRepoPath(manifestPath),
    repoCommit: baselineRepoCommit,
    currentRepoCommit,
    repoCommitMatches: Boolean(!baselineRepoCommit || !currentRepoCommit || baselineRepoCommit === currentRepoCommit),
    repositories,
    metrics,
    legacyReference,
    fixtureCount: (manifest.fixtures ?? []).length,
    privateMissingCount: privateMissing.length,
    syntheticPendingCount: syntheticPending.length,
    privateMissing,
    syntheticPending,
    errorCount: errors.length,
    warningCount: warnings.length,
    errors,
    warnings,
    readyForPhase1: errors.length === 0 && privateMissing.length === 0 && syntheticPending.length === 0
  };

  const reportPath = path.resolve(args.report ?? defaultReportPath);
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  console.log(JSON.stringify({ ...report, reportPath: toRepoPath(reportPath) }, null, 2));

  if (errors.length || (args.strict === true && !report.readyForPhase1)) {
    process.exitCode = 1;
  }
}

function captureBaselines() {
  const manifest = readJson(manifestPath);
  const cli = path.resolve(args.cli ?? path.join(repoRoot, "src-tauri", "target", "debug", cliBinaryName()));
  if (!fs.existsSync(cli)) {
    console.error(`[phase0-golden] CLI not found: ${cli}`);
    console.error("Build it first with: cargo build --manifest-path src-tauri/Cargo.toml --jobs 2");
    process.exitCode = 2;
    return;
  }

  const rawDir = path.join(repoRoot, "tmp", "phase0-golden", "raw-v1");
  fs.mkdirSync(rawDir, { recursive: true });
  const summaries = [];

  for (const fixture of manifest.fixtures ?? []) {
    if (fixture.status !== "available") continue;
    const sourcePath = resolveRepoPath(fixture.sourcePath);
    if (!sourcePath || !fs.existsSync(sourcePath)) {
      throw new Error(`source missing: ${fixture.fixtureId}`);
    }
    const rawPath = path.join(rawDir, `${fixture.fixtureId}.json`);
    const generated = spawnSync(cli, ["--generate-reading-source", sourcePath, "--out", rawPath], {
      cwd: repoRoot,
      encoding: "utf8",
      maxBuffer: 1024 * 1024 * 20
    });
    if (generated.status !== 0 || generated.error) {
      throw new Error(`V1 capture failed for ${fixture.fixtureId}: ${generated.error?.message ?? generated.stderr?.trim() ?? `exit_${generated.status}`}`);
    }
    const payload = readJson(rawPath);
    const hash = sha256File(sourcePath);
    const sizeBytes = fs.statSync(sourcePath).size;
    const snapshot = {
      schemaVersion: "V1BaselineSnapshotV1",
      fixtureId: fixture.fixtureId,
      source: { path: fixture.sourcePath, sha256: hash, sizeBytes },
      capturedAt: manifest.baseline?.capturedAt ?? null,
      observed: summarizePayload(payload),
      payload: normalizePayload(payload)
    };
    const baselinePath = resolveRepoPath(fixture.baselinePath);
    fs.mkdirSync(path.dirname(baselinePath), { recursive: true });
    fs.writeFileSync(baselinePath, `${JSON.stringify(snapshot, null, 2)}\n`, "utf8");
    const metadataPath = resolveRepoPath(fixture.metadataPath);
    if (metadataPath && fs.existsSync(metadataPath)) {
      const metadata = readJson(metadataPath);
      metadata.baseline = {
        ...(metadata.baseline ?? {}),
        v1Path: fixture.baselinePath,
        observed: snapshot.observed
      };
      fs.writeFileSync(metadataPath, `${JSON.stringify(metadata, null, 2)}\n`, "utf8");
    }
    summaries.push({ fixtureId: fixture.fixtureId, baselinePath: fixture.baselinePath, observed: snapshot.observed });
  }

  console.log(JSON.stringify({
    schemaVersion: "Phase0GoldenCaptureReportV1",
    cli: toRepoPath(cli),
    capturedCount: summaries.length,
    summaries
  }, null, 2));
}

function validateMetadata(metadata, fixture, actualHash, actualSize, errors) {
  if (metadata.schemaVersion !== "GoldenFixtureMetadataV1") {
    errors.push(`unsupported metadata schema: ${fixture.fixtureId}`);
  }
  if (metadata.fixtureId !== fixture.fixtureId) {
    errors.push(`metadata fixture id mismatch: ${fixture.fixtureId}`);
  }
  if (metadata.source?.path !== fixture.sourcePath) {
    errors.push(`metadata source path mismatch: ${fixture.fixtureId}`);
  }
  if (metadata.source?.sha256 !== actualHash) {
    errors.push(`source hash mismatch: ${fixture.fixtureId}`);
  }
  if (metadata.source?.sizeBytes !== actualSize) {
    errors.push(`source size mismatch: ${fixture.fixtureId}`);
  }
  const expected = metadata.expected ?? {};
  for (const field of ["pageRoles", "taskGroups", "slots", "assets"]) {
    if (!Array.isArray(expected[field])) errors.push(`metadata expected.${field} must be an array: ${fixture.fixtureId}`);
  }
  if (!Array.isArray(metadata.knownIssues)) errors.push(`metadata knownIssues must be an array: ${fixture.fixtureId}`);

  const slotIds = new Set();
  for (const slot of expected.slots ?? []) {
    if (!slot.id || slotIds.has(slot.id)) errors.push(`duplicate/missing slot id: ${fixture.fixtureId}`);
    slotIds.add(slot.id);
  }
  const groupIds = new Set();
  for (const group of expected.taskGroups ?? []) {
    if (!group.id || groupIds.has(group.id)) errors.push(`duplicate/missing task group id: ${fixture.fixtureId}`);
    groupIds.add(group.id);
    for (const slotId of group.slotIds ?? []) {
      if (!slotIds.has(slotId)) errors.push(`task group references unknown slot ${slotId}: ${fixture.fixtureId}`);
    }
  }
}

function validateBaseline(baseline, fixture, actualHash, actualSize, errors) {
  if (baseline.schemaVersion !== "V1BaselineSnapshotV1") {
    errors.push(`unsupported baseline schema: ${fixture.fixtureId}`);
  }
  if (baseline.fixtureId !== fixture.fixtureId) {
    errors.push(`baseline fixture id mismatch: ${fixture.fixtureId}`);
  }
  if (baseline.source?.path !== fixture.sourcePath) {
    errors.push(`baseline source path mismatch: ${fixture.fixtureId}`);
  }
  if (baseline.source?.sha256 !== actualHash) {
    errors.push(`baseline source hash mismatch: ${fixture.fixtureId}`);
  }
  if (baseline.source?.sizeBytes !== actualSize) {
    errors.push(`baseline source size mismatch: ${fixture.fixtureId}`);
  }
  if (!baseline.payload || typeof baseline.payload !== "object") {
    errors.push(`baseline payload missing: ${fixture.fixtureId}`);
  }
}

function validateMetricsContract(manifest, errors) {
  const metricsPath = resolveRepoPath(manifest.metricsPath);
  if (!metricsPath || !fs.existsSync(metricsPath)) {
    errors.push(`metrics contract missing: ${manifest.metricsPath ?? "<missing>"}`);
    return null;
  }
  const metrics = readJson(metricsPath);
  if (metrics.schemaVersion !== "GoldenMetricsV1") {
    errors.push(`unsupported metrics schema: ${metrics.schemaVersion ?? "missing"}`);
  }
  const metricIds = new Set();
  let metricCount = 0;
  for (const group of metrics.metricGroups ?? []) {
    for (const metric of group.metrics ?? []) {
      if (!metric.id || metricIds.has(metric.id)) errors.push(`duplicate/missing metric id: ${metric.id ?? "<missing>"}`);
      metricIds.add(metric.id);
      metricCount += 1;
    }
  }
  const gateIds = new Set();
  for (const gate of metrics.hardGates ?? []) {
    if (!gate.id || gateIds.has(gate.id)) errors.push(`duplicate/missing hard gate id: ${gate.id ?? "<missing>"}`);
    gateIds.add(gate.id);
    if (!metricIds.has(gate.metricId)) errors.push(`hard gate references unknown metric: ${gate.id ?? "<missing>"}`);
  }
  if (!Array.isArray(metrics.metricGroups) || metrics.metricGroups.length === 0) errors.push("metrics contract has no metric groups");
  if (!Array.isArray(metrics.hardGates) || metrics.hardGates.length === 0) errors.push("metrics contract has no hard gates");
  return {
    path: manifest.metricsPath,
    groupCount: Array.isArray(metrics.metricGroups) ? metrics.metricGroups.length : 0,
    metricCount,
    hardGateCount: Array.isArray(metrics.hardGates) ? metrics.hardGates.length : 0,
    sourcePlanSection: metrics.sourcePlanSection ?? null
  };
}

function validateLegacyReference(manifest, errors, warnings) {
  const referencePath = resolveRepoPath(manifest.legacyReferencePath);
  if (!referencePath || !fs.existsSync(referencePath)) {
    errors.push(`legacy reference index missing: ${manifest.legacyReferencePath ?? "<missing>"}`);
    return null;
  }
  const index = readJson(referencePath);
  if (index.schemaVersion !== "LegacyCorpusReferenceIndexV1") {
    errors.push(`unsupported legacy reference schema: ${index.schemaVersion ?? "missing"}`);
  }
  const ids = new Set((index.references ?? []).map((reference) => reference.fixtureId));
  const requiredIds = new Set(
    manifest.legacyReferenceFixtureIds
      ?? (manifest.requiredPrivateCorpus ?? []).map((fixture) => fixture.fixtureId)
  );
  for (const fixtureId of requiredIds) {
    if (!ids.has(fixtureId)) warnings.push(`legacy reference missing for private fixture: ${fixtureId}`);
  }
  for (const reference of index.references ?? []) {
    if (reference.status !== "reference-only") errors.push(`legacy reference must remain reference-only: ${reference.fixtureId ?? "<missing>"}`);
    if (!/^[a-f0-9]{64}$/.test(String(reference.legacyJsSha256 ?? ""))) errors.push(`legacy reference hash invalid: ${reference.fixtureId ?? "<missing>"}`);
  }
  return {
    path: manifest.legacyReferencePath,
    status: index.status ?? null,
    referenceCount: Array.isArray(index.references) ? index.references.length : 0,
    missingCount: Array.isArray(index.missing) ? index.missing.length : 0
  };
}

function summarizePayload(payload) {
  const pages = payload.documentIr?.pages ?? [];
  const blocks = pages.flatMap((page) => page.blocks ?? []);
  const groups = payload.authoringIr?.groups ?? [];
  const questions = groups.flatMap((group) => group.questions ?? []);
  const answerKey = payload.authoringIr?.answerKey ?? {};
  return {
    pageCount: pages.length,
    blockCount: blocks.length,
    groupCount: groups.length,
    slotCount: questions.length,
    assetCount: (payload.documentIr?.assets ?? []).length,
    answerCount: Object.keys(answerKey).length,
    warningCount: (payload.documentIr?.parser?.warnings ?? []).length,
    groupKinds: groups.map((group) => group.kind),
    questionIds: questions.map((question) => question.id),
    roles: [...new Set(blocks.map((block) => block.roleHint).filter(Boolean))].sort()
  };
}

function normalizePayload(value, key = "") {
  if (Array.isArray(value)) return value.map((entry) => normalizePayload(entry, key));
  if (!value || typeof value !== "object") {
    if ((key === "sourcePath" || key === "path") && typeof value === "string") return normalizePathValue(value);
    return value;
  }
  const dynamicKeys = new Set(["jobId", "examId", "generatedAt", "createdAt", "updatedAt", "importedAt", "resolvedAt", "recordedAt"]);
  const normalized = {};
  for (const [childKey, childValue] of Object.entries(value)) {
    if (dynamicKeys.has(childKey)) continue;
    normalized[childKey] = normalizePayload(childValue, childKey);
  }
  return normalized;
}

function normalizePathValue(value) {
  const absolute = path.resolve(value);
  if (absolute === repoRoot || absolute.startsWith(`${repoRoot}${path.sep}`)) return toRepoPath(absolute);
  return value.replaceAll("\\", "/");
}

function readJson(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    throw new Error(`invalid JSON ${filePath}: ${error.message}`);
  }
}

function sameJsonValue(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function sha256File(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function readCurrentRepoCommit() {
  return readGitValue(repoRoot, ["rev-parse", "HEAD"]);
}

function inspectRepositoryBaselines(manifest, warnings) {
  const configured = manifest.baseline?.repositories ?? [{
    repositoryId: "pdf2test",
    path: ".",
    branch: manifest.baseline?.repoBranch,
    commit: manifest.baseline?.repoCommit
  }];
  return configured.map((entry) => {
    const repositoryPath = resolveRepoPath(entry.path);
    if (!repositoryPath || !fs.existsSync(repositoryPath)) {
      warnings.push(`repository path missing: ${entry.repositoryId} -> ${entry.path}`);
      return {
        repositoryId: entry.repositoryId,
        path: entry.path,
        baselineBranch: entry.branch ?? null,
        baselineCommit: entry.commit ?? null,
        currentBranch: null,
        currentCommit: null,
        exists: false,
        matches: false
      };
    }
    const currentBranch = readGitValue(repositoryPath, ["branch", "--show-current"]);
    const currentCommit = readGitValue(repositoryPath, ["rev-parse", "HEAD"]);
    const matches = Boolean(
      (!entry.branch || !currentBranch || entry.branch === currentBranch)
      && (!entry.commit || !currentCommit || entry.commit === currentCommit)
    );
    if (!matches) {
      warnings.push(`repository baseline drift: ${entry.repositoryId}`);
    }
    return {
      repositoryId: entry.repositoryId,
      path: entry.path,
      baselineBranch: entry.branch ?? null,
      baselineCommit: entry.commit ?? null,
      currentBranch,
      currentCommit,
      exists: true,
      matches
    };
  });
}

function readGitValue(cwd, args) {
  const result = spawnSync("git", args, { cwd, encoding: "utf8" });
  return result.status === 0 ? result.stdout.trim() : null;
}

function resolveRepoPath(relativePath) {
  if (!relativePath || path.isAbsolute(relativePath)) return relativePath ? path.resolve(relativePath) : null;
  return path.resolve(repoRoot, relativePath);
}

function toRepoPath(filePath) {
  const relative = path.relative(repoRoot, filePath);
  return relative ? relative.replaceAll("\\", "/") : ".";
}

function validateUniqueId(ids, id, errors) {
  if (!id || ids.has(id)) errors.push(`duplicate/missing fixture id: ${id ?? "<missing>"}`);
  ids.add(id);
}

function cliBinaryName() {
  return process.platform === "win32" ? "ielts-author-studio.exe" : "ielts-author-studio";
}

function parseArgs(argv) {
  const parsed = { _: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (!arg.startsWith("--")) {
      parsed._.push(arg);
      continue;
    }
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
  console.log([
    "usage: node scripts/phase0-golden.mjs <verify|capture> [options]",
    "",
    "verify options:",
    "  --manifest <path>  manifest path (default: fixtures/golden/manifest.json)",
    "  --report <path>    report path (default: tmp/phase0-golden/verification.json)",
    "  --strict           fail when required private/synthetic corpus is incomplete",
    "",
    "capture options:",
    "  --cli <path>       existing CLI binary; defaults to src-tauri/target/debug",
    "  --manifest <path>  manifest path (default: fixtures/golden/manifest.json)"
  ].join("\n"));
  process.exit(code);
}
