import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import Ajv2020 from "ajv/dist/2020.js";
import ts from "typescript";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultManifestPath = path.join(repoRoot, "fixtures", "golden", "manifest.json");
const defaultReportPath = path.join(repoRoot, "tmp", "phase0-golden", "verification.json");
const metadataSchemaPath = path.join(repoRoot, "fixtures", "golden", "schema", "golden-fixture-metadata-v1.schema.json");
const metricsSchemaPath = path.join(repoRoot, "fixtures", "golden", "schema", "golden-metrics-v1.schema.json");
const featureFlagsPath = path.join(repoRoot, "src", "config", "featureFlags.ts");
const requiredPhase0Flags = [
  "documentIrV2",
  "authoringV2",
  "runtimeSourceV2",
  "nasPackageV2",
  "listeningV1",
  "pdfPerQuestionLlmRepair"
];
const requiredPhase1Flags = [
  ...requiredPhase0Flags,
  "documentIrV2Shadow",
  "authoringV2Shadow",
  "qualityGateV2"
];

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
} else if (command === "self-test-cli-freshness") {
  runV1CliFreshnessSelfTest();
} else if (command === "self-test-feature-flags") {
  runFeatureFlagSelfTest();
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
  let privateSourceSkippedCount = 0;
  const ajv = new Ajv2020({ allErrors: true, strict: true });
  const validateMetadataSchema = compileSchema(ajv, metadataSchemaPath, errors);
  const validateMetricsSchema = compileSchema(ajv, metricsSchemaPath, errors);
  const metrics = validateMetricsContract(manifest, errors, validateMetricsSchema);
  const legacyReference = validateLegacyReference(manifest, errors, warnings);
  const featureFlags = validateFeatureFlagDefaults(manifest, errors);
  const featureFlagSelfTest = runFeatureFlagSelfTest({ quiet: true });
  const v1ComparisonCli = args.strict === true
    ? inspectV1ComparisonCli(errors)
    : null;
  const v1CliFreshnessSelfTest = runV1CliFreshnessSelfTest({ quiet: true });

  if (manifest.schemaVersion !== "GoldenCorpusManifestV1") {
    errors.push(`unsupported manifest schema: ${manifest.schemaVersion ?? "missing"}`);
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
    const sourceAvailable = Boolean(sourcePath && fs.existsSync(sourcePath) && fs.statSync(sourcePath).isFile());
    const maySkipPrivateSource = args["ci-contract"] === true
      && String(fixture.sourcePath ?? "").startsWith("fixtures/golden/private-real/");
    if (!sourceAvailable && !maySkipPrivateSource) {
      errors.push(`source missing: ${fixture.fixtureId} -> ${fixture.sourcePath}`);
      continue;
    }
    if (!sourceAvailable) privateSourceSkippedCount += 1;
    if (!metadataPath || !fs.existsSync(metadataPath)) {
      errors.push(`metadata missing: ${fixture.fixtureId} -> ${fixture.metadataPath}`);
    }
    if (!baselinePath || !fs.existsSync(baselinePath)) {
      errors.push(`V1 baseline missing: ${fixture.fixtureId} -> ${fixture.baselinePath}`);
    }

    const actualHash = sourceAvailable ? sha256File(sourcePath) : fixture.sha256;
    const actualSize = sourceAvailable ? fs.statSync(sourcePath).size : fixture.sizeBytes;
    if (fixture.sha256 !== actualHash) errors.push(`manifest source hash mismatch: ${fixture.fixtureId}`);
    if (fixture.sizeBytes !== actualSize) errors.push(`manifest source size mismatch: ${fixture.fixtureId}`);
    let metadata = null;
    let baseline = null;
    if (metadataPath && fs.existsSync(metadataPath)) {
      metadata = readJson(metadataPath);
      validateAgainstSchema(validateMetadataSchema, metadata, `metadata:${fixture.fixtureId}`, errors);
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
    if (args.strict === true && baseline && sourceAvailable) {
      validateActualV1Baseline(fixture, sourcePath, baseline, errors, v1ComparisonCli);
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
    const metadataPath = resolveRepoPath(fixture.metadataPath);
    const metadata = metadataPath && fs.existsSync(metadataPath) ? readJson(metadataPath) : null;
    if (required.reviewRequired === true) {
      if (metadata?.review?.status !== "approved") {
        errors.push(`required private fixture review is not approved: ${required.fixtureId}`);
      }
      if (!metadata?.review?.reviewedBy || !metadata?.review?.reviewedAt || !metadata?.review?.method) {
        errors.push(`required private fixture review evidence incomplete: ${required.fixtureId}`);
      }
      if (String(metadata?.review?.method ?? "").includes("v1-derived")) {
        errors.push(`required private fixture review cannot be V1-derived: ${required.fixtureId}`);
      }
    }
    validateAuthoritativeMetadata(metadata, required.fixtureId, errors);
  }

  const privateCorpusSelection = inspectPrivateCorpusSelection(manifest, errors);

  const privateMissing = (manifest.requiredPrivateCorpus ?? [])
    .filter((fixture) => fixture.status !== "available")
    .map((fixture) => fixture.fixtureId);
  const syntheticPending = (manifest.plannedSyntheticFixtures ?? [])
    .filter((fixture) => fixture.status !== "available")
    .map((fixture) => fixture.fixtureId);
  const registeredSyntheticFixtures = (manifest.fixtures ?? []).filter((fixture) =>
    fixture.status === "available"
      && String(fixture.sourcePath ?? "").replaceAll("\\", "/").startsWith("fixtures/golden/synthetic/")
  );
  if (registeredSyntheticFixtures.length < 20) {
    errors.push(`registered synthetic corpus must contain at least 20 available fixtures: found ${registeredSyntheticFixtures.length}`);
  }
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
    featureFlags,
    featureFlagSelfTest,
    v1ComparisonCli,
    v1CliFreshnessSelfTest,
    privateCorpusSelection,
    privateSourceSkippedCount,
    fixtureCount: (manifest.fixtures ?? []).length,
    registeredSyntheticFixtureCount: registeredSyntheticFixtures.length,
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

function validateFeatureFlagDefaults(manifest, errors) {
  const manifestFlags = manifest.featureFlags ?? {};
  validateClosedFlagSet("manifest.featureFlags", manifestFlags, requiredPhase0Flags, errors);

  let defaults;
  try {
    defaults = readFeatureFlagDefaultObjects(featureFlagsPath);
  } catch (error) {
    errors.push(`feature flag defaults cannot be inspected: ${error.message}`);
    return null;
  }
  validateClosedFlagSet(
    "DEFAULT_PHASE0_FEATURE_FLAGS",
    defaults.DEFAULT_PHASE0_FEATURE_FLAGS,
    requiredPhase0Flags,
    errors
  );
  validateClosedFlagSet(
    "DEFAULT_PHASE1_FEATURE_FLAGS",
    defaults.DEFAULT_PHASE1_FEATURE_FLAGS,
    requiredPhase1Flags,
    errors
  );
  return {
    path: toRepoPath(featureFlagsPath),
    phase0DefaultCount: Object.keys(defaults.DEFAULT_PHASE0_FEATURE_FLAGS ?? {}).length,
    phase1DefaultCount: Object.keys(defaults.DEFAULT_PHASE1_FEATURE_FLAGS ?? {}).length,
    allRequiredDefaultsClosed: requiredPhase0Flags.every((name) => manifestFlags[name] === false)
      && requiredPhase0Flags.every((name) => defaults.DEFAULT_PHASE0_FEATURE_FLAGS?.[name] === false)
      && requiredPhase1Flags.every((name) => defaults.DEFAULT_PHASE1_FEATURE_FLAGS?.[name] === false)
  };
}

function validateClosedFlagSet(label, flags, requiredNames, errors) {
  if (!flags || typeof flags !== "object") {
    errors.push(`${label} is missing`);
    return;
  }
  for (const name of requiredNames) {
    if (!Object.hasOwn(flags, name)) errors.push(`${label} required flag is missing: ${name}`);
    else if (flags[name] !== false) errors.push(`${label} feature flag must be false: ${name}`);
  }
  for (const [name, value] of Object.entries(flags)) {
    if (value !== false && !requiredNames.includes(name)) {
      errors.push(`${label} feature flag must be false: ${name}`);
    }
  }
}

function runFeatureFlagSelfTest({ quiet = false } = {}) {
  const phase0 = requiredPhase0Flags.map((name) => `${name}: false`).join(",\n");
  const phase1Only = requiredPhase1Flags
    .filter((name) => !requiredPhase0Flags.includes(name))
    .map((name) => `${name}: false`)
    .join(",\n");
  const validSource = `
    export const DEFAULT_PHASE0_FEATURE_FLAGS = Object.freeze({${phase0}});
    export const DEFAULT_PHASE1_FEATURE_FLAGS = Object.freeze({
      ...DEFAULT_PHASE0_FEATURE_FLAGS,
      ${phase1Only}
    });
  `;
  const parsed = parseFeatureFlagDefaultObjects(validSource, "feature-flags-self-test.ts");
  const validationCases = [
    { id: "all-required-disabled", flags: parsed.DEFAULT_PHASE1_FEATURE_FLAGS, required: requiredPhase1Flags, expectedErrors: 0 },
    {
      id: "missing-required-flag",
      flags: Object.fromEntries(Object.entries(parsed.DEFAULT_PHASE0_FEATURE_FLAGS).filter(([name]) => name !== "authoringV2")),
      required: requiredPhase0Flags,
      expectedErrors: 1
    },
    {
      id: "required-flag-enabled",
      flags: { ...parsed.DEFAULT_PHASE1_FEATURE_FLAGS, qualityGateV2: true },
      required: requiredPhase1Flags,
      expectedErrors: 1
    },
    {
      id: "new-flag-enabled",
      flags: { ...parsed.DEFAULT_PHASE1_FEATURE_FLAGS, futureV2: true },
      required: requiredPhase1Flags,
      expectedErrors: 1
    }
  ];
  for (const testCase of validationCases) {
    const errors = [];
    validateClosedFlagSet(testCase.id, testCase.flags, testCase.required, errors);
    if (errors.length !== testCase.expectedErrors) {
      throw new Error(
        `feature flag self-test failed: ${testCase.id}: expectedErrors=${testCase.expectedErrors}:actual=${errors.length}`
      );
    }
  }
  const rejectedSources = [
    {
      id: "indirect-default",
      source: validSource.replace("documentIrV2: false", "documentIrV2: CLOSED")
    },
    {
      id: "duplicate-default",
      source: validSource.replace("documentIrV2: false", "documentIrV2: false, documentIrV2: false")
    },
    {
      id: "unknown-spread",
      source: validSource.replace("...DEFAULT_PHASE0_FEATURE_FLAGS", "...OTHER_FLAGS")
    },
    {
      id: "local-shadow",
      source: `${validSource}\nfunction shadow() { const DEFAULT_PHASE0_FEATURE_FLAGS = { documentIrV2: true }; return DEFAULT_PHASE0_FEATURE_FLAGS; }`
    },
    {
      id: "mutable-wrapper",
      source: validSource.replaceAll("Object.freeze", "mutate")
    }
  ];
  for (const testCase of rejectedSources) {
    let rejected = false;
    try {
      parseFeatureFlagDefaultObjects(testCase.source, `${testCase.id}.ts`);
    } catch {
      rejected = true;
    }
    if (!rejected) throw new Error(`feature flag self-test failed: ${testCase.id} was accepted`);
  }
  const report = {
    schemaVersion: "Phase0FeatureFlagSelfTestV1",
    status: "passed",
    caseCount: validationCases.length + rejectedSources.length
  };
  if (!quiet) console.log(JSON.stringify(report, null, 2));
  return report;
}

function readFeatureFlagDefaultObjects(filePath) {
  return parseFeatureFlagDefaultObjects(fs.readFileSync(filePath, "utf8"), filePath);
}

function parseFeatureFlagDefaultObjects(sourceText, filePath) {
  const source = ts.createSourceFile(
    filePath,
    sourceText,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS
  );
  const diagnostics = source.parseDiagnostics ?? [];
  if (diagnostics.length) {
    throw new Error(`${filePath} contains TypeScript parse diagnostics`);
  }
  const objects = {};
  const declarations = new Map();
  for (const statement of source.statements) {
    if (!ts.isVariableStatement(statement)) continue;
    const isExported = statement.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword);
    const isConst = (statement.declarationList.flags & ts.NodeFlags.Const) !== 0;
    for (const declaration of statement.declarationList.declarations) {
      if (!ts.isIdentifier(declaration.name)) continue;
      const name = declaration.name.text;
      if (name !== "DEFAULT_PHASE0_FEATURE_FLAGS" && name !== "DEFAULT_PHASE1_FEATURE_FLAGS") continue;
      if (!isExported || !isConst) throw new Error(`${name} must be a top-level exported const`);
      if (declarations.has(name)) throw new Error(`${name} declaration is duplicated`);
      declarations.set(name, declaration);
    }
  }
  const targetNames = new Set(["DEFAULT_PHASE0_FEATURE_FLAGS", "DEFAULT_PHASE1_FEATURE_FLAGS"]);
  const inspectShadowing = (node) => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && targetNames.has(node.name.text)) {
      const isTopLevel = node.parent?.parent?.parent === source;
      if (!isTopLevel) throw new Error(`${node.name.text} declaration is shadowed in a local scope`);
    }
    ts.forEachChild(node, inspectShadowing);
  };
  inspectShadowing(source);
  for (const name of ["DEFAULT_PHASE0_FEATURE_FLAGS", "DEFAULT_PHASE1_FEATURE_FLAGS"]) {
    const declaration = declarations.get(name);
    if (!declaration) throw new Error(`${name} declaration is missing`);
    const literal = unwrapObjectLiteral(declaration.initializer);
    if (!literal) throw new Error(`${name} must be initialized from an object literal`);
    objects[name] = evaluateBooleanObjectLiteral(literal, objects);
  }
  return objects;
}

function unwrapObjectLiteral(expression) {
  if (!expression) return null;
  if (ts.isObjectLiteralExpression(expression)) return expression;
  if (
    ts.isCallExpression(expression)
    && expression.arguments.length === 1
    && ts.isPropertyAccessExpression(expression.expression)
    && ts.isIdentifier(expression.expression.expression)
    && expression.expression.expression.text === "Object"
    && expression.expression.name.text === "freeze"
  ) {
    return unwrapObjectLiteral(expression.arguments[0]);
  }
  if (ts.isAsExpression(expression) || ts.isSatisfiesExpression(expression) || ts.isParenthesizedExpression(expression)) {
    return unwrapObjectLiteral(expression.expression);
  }
  return null;
}

function evaluateBooleanObjectLiteral(literal, knownObjects) {
  const value = {};
  for (const property of literal.properties) {
    if (ts.isSpreadAssignment(property)) {
      if (!ts.isIdentifier(property.expression) || !knownObjects[property.expression.text]) {
        throw new Error("feature flag defaults contain an unsupported spread");
      }
      Object.assign(value, knownObjects[property.expression.text]);
      continue;
    }
    if (!ts.isPropertyAssignment(property)) throw new Error("feature flag defaults must use property assignments");
    const name = ts.isIdentifier(property.name) || ts.isStringLiteral(property.name)
      ? property.name.text
      : null;
    if (!name) throw new Error("feature flag defaults contain an unsupported property name");
    if (Object.hasOwn(value, name)) throw new Error(`feature flag default is duplicated: ${name}`);
    if (property.initializer.kind === ts.SyntaxKind.FalseKeyword) value[name] = false;
    else if (property.initializer.kind === ts.SyntaxKind.TrueKeyword) value[name] = true;
    else throw new Error(`feature flag default must be a boolean literal: ${name}`);
  }
  return value;
}

function validateAuthoritativeMetadata(metadata, fixtureId, errors) {
  const expected = metadata?.expected;
  if (!expected) {
    errors.push(`authoritative metadata expected contract missing: ${fixtureId}`);
    return;
  }
  for (const key of ["optionBanks", "responseGroups", "sourceEvidence"]) {
    if (!Array.isArray(expected[key])) errors.push(`authoritative metadata ${key} missing: ${fixtureId}`);
  }
  if (!expected.runtimeExpectations) errors.push(`authoritative metadata runtime expectations missing: ${fixtureId}`);
  if (!Array.isArray(expected.responseGroups) || !Array.isArray(expected.sourceEvidence) || !Array.isArray(expected.optionBanks)) return;

  const taskGroups = new Map((expected.taskGroups ?? []).map((group) => [group.id, group]));
  const slots = new Set((expected.slots ?? []).map((slot) => slot.id));
  const evidenceIds = new Set(expected.sourceEvidence.map((evidence) => evidence.id));
  const optionBanks = new Map(expected.optionBanks.map((bank) => [bank.id, bank]));
  const responseGroups = new Map();
  const slotAssignments = new Map();

  for (const evidence of expected.sourceEvidence) {
    if (!Array.isArray(evidence.pageIndexes) || evidence.pageIndexes.length === 0) {
      errors.push(`source evidence has no pages: ${fixtureId}.${evidence.id}`);
    }
    for (const pageIndex of evidence.pageIndexes ?? []) {
      if (!(expected.pageRoles ?? []).some((page) => page.pageIndex === pageIndex)) {
        errors.push(`source evidence references unknown page: ${fixtureId}.${evidence.id}.${pageIndex}`);
      }
    }
  }

  for (const bank of expected.optionBanks) {
    if (!taskGroups.has(bank.taskGroupId)) errors.push(`option bank references unknown task group: ${fixtureId}.${bank.id}`);
    for (const evidenceId of bank.sourceEvidenceIds ?? []) {
      if (!evidenceIds.has(evidenceId)) errors.push(`option bank references unknown evidence: ${fixtureId}.${bank.id}.${evidenceId}`);
    }
  }

  for (const group of expected.responseGroups) {
    if (responseGroups.has(group.id)) errors.push(`duplicate response group: ${fixtureId}.${group.id}`);
    responseGroups.set(group.id, group);
    const taskGroup = taskGroups.get(group.taskGroupId);
    if (!taskGroup) errors.push(`response group references unknown task group: ${fixtureId}.${group.id}`);
    if (group.cardinality?.min > group.cardinality?.max) errors.push(`invalid response cardinality: ${fixtureId}.${group.id}`);
    if (group.cardinality?.exact !== undefined && (group.cardinality.exact < group.cardinality.min || group.cardinality.exact > group.cardinality.max)) {
      errors.push(`exact response cardinality is outside min/max: ${fixtureId}.${group.id}`);
    }
    for (const slotId of group.slotIds ?? []) {
      if (!slots.has(slotId)) errors.push(`response group references unknown slot: ${fixtureId}.${group.id}.${slotId}`);
      if (taskGroup && !taskGroup.slotIds.includes(slotId)) errors.push(`response group slot escapes task group: ${fixtureId}.${group.id}.${slotId}`);
      slotAssignments.set(slotId, (slotAssignments.get(slotId) ?? 0) + 1);
    }
    for (const evidenceId of group.sourceEvidenceIds ?? []) {
      if (!evidenceIds.has(evidenceId)) errors.push(`response group references unknown evidence: ${fixtureId}.${group.id}.${evidenceId}`);
    }
    const binding = group.optionBinding ?? {};
    if (binding.mode === "option_bank") {
      const bank = optionBanks.get(binding.optionBankId);
      if (!bank) errors.push(`response group references unknown option bank: ${fixtureId}.${group.id}`);
      if (bank && bank.taskGroupId !== group.taskGroupId) errors.push(`response group option bank scope mismatch: ${fixtureId}.${group.id}`);
      if (bank && (group.reusePolicy === "allowed") !== bank.allowReuse) errors.push(`response group reuse policy mismatch: ${fixtureId}.${group.id}`);
    } else if (binding.optionBankId) {
      errors.push(`non-bank response group carries optionBankId: ${fixtureId}.${group.id}`);
    }
    if (binding.mode === "inline" && !Array.isArray(binding.inlineLabels)) errors.push(`inline response options missing: ${fixtureId}.${group.id}`);
    if (binding.mode === "none" && group.reusePolicy !== "not_applicable") errors.push(`optionless response group has reuse policy: ${fixtureId}.${group.id}`);
  }

  for (const [taskGroupId, taskGroup] of taskGroups) {
    const declared = new Set(taskGroup.responseGroupIds ?? []);
    const actual = new Set([...responseGroups.values()].filter((group) => group.taskGroupId === taskGroupId).map((group) => group.id));
    if (!sameStringSet(declared, actual)) errors.push(`task response group binding mismatch: ${fixtureId}.${taskGroupId}`);
    for (const evidenceId of taskGroup.sourceEvidenceIds ?? []) {
      if (!evidenceIds.has(evidenceId)) errors.push(`task group references unknown evidence: ${fixtureId}.${taskGroupId}.${evidenceId}`);
    }
  }
  for (const slotId of slots) {
    if (slotAssignments.get(slotId) !== 1) errors.push(`authoritative slot must belong to exactly one response group: ${fixtureId}.${slotId}`);
  }

  const runtime = expected.runtimeExpectations ?? {};
  if (runtime.authoritativeVersion !== "v1" || runtime.artifactMode !== "shadow_only" || runtime.productionExposure !== "disabled") {
    errors.push(`runtime authority contract mismatch: ${fixtureId}`);
  }
  for (const flag of ["documentIrV2Shadow", "authoringV2Shadow", "qualityGateV2", "runtimeSourceV2", "nasPackageV2"]) {
    if (runtime.requiredFeatureFlags?.[flag] !== false) errors.push(`runtime feature flag must remain false: ${fixtureId}.${flag}`);
  }
}

function inspectPrivateCorpusSelection(manifest, errors) {
  const selected = manifest.requiredPrivateCorpus ?? [];
  const selectedIds = selected.map((fixture) => fixture.fixtureId);
  if (selectedIds.length !== 8 || new Set(selectedIds).size !== 8) {
    errors.push(`authoritative private corpus must select exactly eight unique fixtures: found ${selectedIds.length}`);
  }
  const privateRoot = path.join(repoRoot, "fixtures", "golden", "private-real");
  const availablePdfCount = fs.existsSync(privateRoot)
    ? fs.readdirSync(privateRoot).filter((name) => name.toLowerCase().endsWith(".pdf")).length
    : 0;
  return {
    contract: "manifest_selection",
    authoritativeCount: selectedIds.length,
    availablePdfCount,
    fixtureIds: selectedIds
  };
}

function sameStringSet(left, right) {
  return left.size === right.size && [...left].every((value) => right.has(value));
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
  const refreshSourceContract = args["refresh-source-contract"] === true;
  const revisionReason = String(args.reason ?? "authoritative fixture regenerated after an intentional fixture revision");
  const revisionGenerator = String(args.generator ?? "phase0-golden capture");
  let manifestChanged = false;

  const requestedIds = new Set(String(args["fixture-id"] ?? "").split(",").map((value) => value.trim()).filter(Boolean));
  for (const fixture of manifest.fixtures ?? []) {
    if (fixture.status !== "available") continue;
    if (requestedIds.size > 0 && !requestedIds.has(fixture.fixtureId)) continue;
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
    const previousHash = fixture.sha256;
    if (refreshSourceContract && (previousHash !== hash || fixture.sizeBytes !== sizeBytes)) {
      fixture.sha256 = hash;
      fixture.sizeBytes = sizeBytes;
      manifestChanged = true;
    }
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
      if (refreshSourceContract && previousHash && previousHash !== hash) {
        metadata.sourceRevision = {
          updatedAt: new Date().toISOString().slice(0, 10),
          reason: revisionReason,
          generator: revisionGenerator,
          supersedesSha256: previousHash
        };
        metadata.source = {
          ...(metadata.source ?? {}),
          sha256: hash,
          sizeBytes
        };
      }
      metadata.baseline = {
        ...(metadata.baseline ?? {}),
        v1Path: fixture.baselinePath,
        observed: snapshot.observed
      };
      fs.writeFileSync(metadataPath, `${JSON.stringify(metadata, null, 2)}\n`, "utf8");
    }
    summaries.push({ fixtureId: fixture.fixtureId, baselinePath: fixture.baselinePath, observed: snapshot.observed });
  }

  if (manifestChanged) {
    fs.writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
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

function validateMetricsContract(manifest, errors, validateMetricsSchema) {
  const metricsPath = resolveRepoPath(manifest.metricsPath);
  if (!metricsPath || !fs.existsSync(metricsPath)) {
    errors.push(`metrics contract missing: ${manifest.metricsPath ?? "<missing>"}`);
    return null;
  }
  const metrics = readJson(metricsPath);
  validateAgainstSchema(validateMetricsSchema, metrics, "metrics", errors);
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

function compileSchema(ajv, schemaPath, errors) {
  try {
    return ajv.compile(readJson(schemaPath));
  } catch (error) {
    errors.push(`JSON Schema compile failed: ${toRepoPath(schemaPath)}: ${error.message}`);
    return null;
  }
}

function validateAgainstSchema(validate, value, label, errors) {
  if (!validate || validate(value)) return;
  for (const issue of validate.errors ?? []) {
    errors.push(`${label} schema violation ${issue.instancePath || "/"}: ${issue.message}`);
  }
}

function validateActualV1Baseline(fixture, sourcePath, baseline, errors, cliInspection) {
  const cli = resolveV1ComparisonCli();
  if (!cliInspection?.exists || !cliInspection.fresh) return;
  const actualDir = path.join(repoRoot, "tmp", "phase0-golden", "current-v1");
  fs.mkdirSync(actualDir, { recursive: true });
  const outputPath = path.join(actualDir, `${fixture.fixtureId}.json`);
  const generated = spawnSync(cli, ["--generate-reading-source", sourcePath, "--out", outputPath], {
    cwd: repoRoot,
    encoding: "utf8",
    maxBuffer: 1024 * 1024 * 20
  });
  if (generated.status !== 0 || generated.error) {
    errors.push(`V1 current-output comparison failed: ${fixture.fixtureId}: ${generated.error?.message ?? generated.stderr?.trim() ?? `exit_${generated.status}`}`);
    return;
  }
  const actual = normalizePayload(readJson(outputPath));
  if (!sameJsonValue(canonicalize(actual), canonicalize(normalizePayload(baseline.payload)))) {
    errors.push(`V1 baseline drift: ${fixture.fixtureId}`);
  }
}

function inspectV1ComparisonCli(errors) {
  const cli = resolveV1ComparisonCli();
  const newestInput = newestV1CliInput();
  if (!fs.existsSync(cli) || !fs.statSync(cli).isFile()) {
    errors.push(`V1 comparison CLI missing: ${toRepoPath(cli)}; build it before strict verification`);
    return {
      path: toRepoPath(cli),
      exists: false,
      fresh: false,
      newestInput: newestInput ? toRepoPath(newestInput.path) : null
    };
  }

  const cliMtimeMs = fs.statSync(cli).mtimeMs;
  const fresh = evaluateV1CliFreshness(true, cliMtimeMs, newestInput?.mtimeMs);
  if (!fresh) {
    errors.push(
      `V1 comparison CLI is stale: ${toRepoPath(cli)} is older than ${toRepoPath(newestInput.path)}; rebuild before strict verification`
    );
  }
  return {
    path: toRepoPath(cli),
    exists: true,
    fresh,
    modifiedAt: new Date(cliMtimeMs).toISOString(),
    newestInput: newestInput ? toRepoPath(newestInput.path) : null,
    newestInputModifiedAt: newestInput ? new Date(newestInput.mtimeMs).toISOString() : null
  };
}

function resolveV1ComparisonCli() {
  return path.resolve(args.cli ?? path.join(repoRoot, "src-tauri", "target", "debug", cliBinaryName()));
}

function evaluateV1CliFreshness(exists, cliMtimeMs, newestInputMtimeMs) {
  if (!exists || !Number.isFinite(cliMtimeMs)) return false;
  return !Number.isFinite(newestInputMtimeMs) || cliMtimeMs > newestInputMtimeMs;
}

function runV1CliFreshnessSelfTest({ quiet = false } = {}) {
  const cases = [
    { id: "missing-cli", exists: false, cliMtimeMs: Number.NaN, inputMtimeMs: 1000, expected: false },
    { id: "stale-by-one-millisecond", exists: true, cliMtimeMs: 999, inputMtimeMs: 1000, expected: false },
    { id: "same-timestamp", exists: true, cliMtimeMs: 1000, inputMtimeMs: 1000, expected: false },
    { id: "newer-cli", exists: true, cliMtimeMs: 1001, inputMtimeMs: 1000, expected: true }
  ];
  for (const testCase of cases) {
    const actual = evaluateV1CliFreshness(
      testCase.exists,
      testCase.cliMtimeMs,
      testCase.inputMtimeMs
    );
    if (actual !== testCase.expected) {
      throw new Error(
        `V1 CLI freshness self-test failed: ${testCase.id}: expected=${testCase.expected}:actual=${actual}`
      );
    }
  }
  const report = {
    schemaVersion: "Phase0V1CliFreshnessSelfTestV1",
    status: "passed",
    caseCount: cases.length,
    staleToleranceMs: 0
  };
  if (!quiet) console.log(JSON.stringify(report, null, 2));
  return report;
}

function newestV1CliInput() {
  const rustRoot = path.join(repoRoot, "src-tauri");
  const candidates = [
    path.join(rustRoot, "Cargo.toml"),
    path.join(rustRoot, "Cargo.lock"),
    path.join(rustRoot, "build.rs"),
    path.join(rustRoot, "tauri.conf.json"),
    path.join(rustRoot, "tauri.windows.offline.conf.json"),
    ...collectFiles(path.join(rustRoot, "capabilities"), () => true),
    ...collectFiles(path.join(repoRoot, ".cargo"), () => true),
    ...collectFiles(path.join(rustRoot, ".cargo"), () => true),
    ...collectFiles(repoRoot, (filePath) => path.basename(filePath).startsWith("rust-toolchain")),
    ...collectFiles(path.join(rustRoot, "src"), (filePath) => filePath.endsWith(".rs"))
  ].filter((filePath) => fs.existsSync(filePath) && fs.statSync(filePath).isFile());
  return candidates.reduce((newest, filePath) => {
    const mtimeMs = fs.statSync(filePath).mtimeMs;
    return !newest || mtimeMs > newest.mtimeMs ? { path: filePath, mtimeMs } : newest;
  }, null);
}

function collectFiles(root, include) {
  if (!fs.existsSync(root)) return [];
  const files = [];
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const entryPath = path.join(root, entry.name);
    if (entry.isDirectory()) files.push(...collectFiles(entryPath, include));
    else if (entry.isFile() && include(entryPath)) files.push(entryPath);
  }
  return files;
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
    return typeof value === "string" ? normalizeDynamicString(value) : value;
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

function normalizeDynamicString(value) {
  return value
    .replace(/author-imports[\\/]import-\d{14}-[a-f0-9]+/gi, "author-imports/<import>")
    .replace(/phase0-[a-z0-9-]+-[0-9a-f]{16,}/gi, "<phase0-run>");
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

function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (!value || typeof value !== "object") return value;
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonicalize(value[key])]));
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
    "usage: node scripts/phase0-golden.mjs <verify|capture|self-test-cli-freshness|self-test-feature-flags> [options]",
    "",
    "verify options:",
    "  --manifest <path>  manifest path (default: fixtures/golden/manifest.json)",
    "  --report <path>    report path (default: tmp/phase0-golden/verification.json)",
    "  --strict           fail when required private/synthetic corpus is incomplete",
    "  --ci-contract      allow absent ignored private PDFs while checking tracked corpus contracts",
    "  --cli <path>       compare current V1 output with every stored baseline in strict mode",
    "",
    "capture options:",
    "  --cli <path>       existing CLI binary; defaults to src-tauri/target/debug",
    "  --manifest <path>  manifest path (default: fixtures/golden/manifest.json)",
    "  --fixture-id <ids> capture only comma-separated fixture ids",
    "  --refresh-source-contract update manifest/metadata hashes for intentional fixture revisions",
    "  --reason <text>    source revision reason (used with --refresh-source-contract)",
    "  --generator <text> source revision generator (used with --refresh-source-contract)"
  ].join("\n"));
  process.exit(code);
}
