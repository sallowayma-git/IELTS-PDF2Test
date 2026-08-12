import { createHash } from "node:crypto";
import { access, readFile, readdir } from "node:fs/promises";
import { dirname, isAbsolute, join, normalize, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import Ajv2020 from "ajv/dist/2020.js";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "..");
const contractRoot = join(repoRoot, "contracts");
const manifestPath = join(contractRoot, "contract-manifest.json");

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function collectRefs(value, refs = []) {
  if (!value || typeof value !== "object") return refs;
  if (typeof value.$ref === "string") refs.push(value.$ref);
  for (const child of Object.values(value)) collectRefs(child, refs);
  return refs;
}

function localRefPath(ref) {
  const [pathPart] = ref.split("#", 1);
  return pathPart || null;
}

async function readJson(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

async function pathExists(path) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

function compileContractSchemas(schemas, compiled, verificationErrors, verificationResults) {
  const ajv = new Ajv2020({
    allErrors: true,
    strict: true,
    // Conditional branches refer to properties declared on their parent schema.
    strictRequired: false,
    // Format keywords remain annotations until a shared format policy is versioned.
    validateFormats: false
  });
  try {
    for (const schema of schemas.values()) ajv.addSchema(schema);
  } catch (error) {
    verificationErrors.push(`schema_registration_failed:${error.message}`);
    return;
  }

  for (const [schemaName, schema] of schemas) {
    try {
      const validate = ajv.getSchema(schema.$id);
      if (!validate) {
        verificationErrors.push(`schema_compile_missing_validator:${schemaName}`);
        continue;
      }
      compiled.set(schemaName, validate);
      verificationResults.push({ check: "schema-compile", schemaName, status: "passed" });
    } catch (error) {
      verificationErrors.push(`schema_compile_failed:${schemaName}:${error.message}`);
    }
  }
}

function valueAtPointer(value, pointer) {
  if (!pointer) return value;
  return pointer
    .split("/")
    .slice(1)
    .map((token) => token.replaceAll("~1", "/").replaceAll("~0", "~"))
    .reduce((current, token) => current?.[token], value);
}

function ajvErrorSummary(issues) {
  return (issues ?? []).slice(0, 25).map((issue) => {
    const detail = issue.keyword === "additionalProperties"
      ? `:${issue.params.additionalProperty}`
      : issue.keyword === "required"
        ? `:${issue.params.missingProperty}`
        : "";
    return `${issue.instancePath || "/"}:${issue.keyword}${detail}:${issue.message}`;
  });
}

function validateFixtureValue(compiled, schemaName, value, label, verificationErrors, verificationResults) {
  const validate = compiled.get(schemaName);
  if (!validate) {
    verificationErrors.push(`fixture_validator_missing:${label}:${schemaName}`);
    return false;
  }
  const valid = validate(value);
  if (!valid) {
    verificationErrors.push(
      `schema_fixture_invalid:${label}:${schemaName}:count=${validate.errors?.length ?? 0}:${ajvErrorSummary(validate.errors).join("|")}`
    );
    return false;
  }
  verificationResults.push({ check: "schema-fixture", schemaName, fixture: label, status: "passed" });
  return true;
}

async function validateStableContractFixtures(compiled, verificationErrors, verificationResults) {
  const fixtureSpecs = [
    {
      schemaName: "DocumentIRV2",
      path: join(repoRoot, "fixtures", "golden", "synthetic", "ielts", "phase1-document-ir-v2-contract.json")
    },
    {
      schemaName: "ContentDocV2",
      path: join(repoRoot, "fixtures", "golden", "synthetic", "ielts", "phase1-content-doc-v2-contract.json")
    },
    {
      schemaName: "IeltsAuthoringIRV2",
      path: join(repoRoot, "fixtures", "golden", "synthetic", "ielts", "early-approaches-authoring-v2.json")
    },
    {
      schemaName: "QualityReportV2",
      path: join(repoRoot, "fixtures", "golden", "synthetic", "ielts", "early-approaches-authoring-v2.json"),
      pointer: "/quality"
    }
  ];
  const values = new Map();
  for (const spec of fixtureSpecs) {
    const label = `${relative(repoRoot, spec.path).replaceAll("\\", "/")}${spec.pointer ?? ""}`;
    try {
      const document = await readJson(spec.path);
      const value = valueAtPointer(document, spec.pointer);
      if (value === undefined) {
        verificationErrors.push(`stable_fixture_pointer_missing:${label}`);
        continue;
      }
      if (validateFixtureValue(compiled, spec.schemaName, value, label, verificationErrors, verificationResults)) {
        values.set(spec.schemaName, structuredClone(value));
      }
    } catch (error) {
      verificationErrors.push(`stable_fixture_read_failed:${label}:${error.message}`);
    }
  }
  return values;
}

async function validateTrackedShadowPairs(compiled, verificationErrors, verificationResults) {
  const registryPath = join(repoRoot, "fixtures", "golden", "synthetic", "ielts", "phase1-tracked-pair-registry.json");
  let registry;
  try {
    registry = await readJson(registryPath);
  } catch (error) {
    verificationErrors.push(`tracked_pair_registry_read_failed:${error.message}`);
    return;
  }
  const pairs = registry?.pairs;
  if (registry?.schemaVersion !== "Phase1TrackedShadowPairRegistryV1" || registry?.evidenceKind !== "contract-slice") {
    verificationErrors.push("tracked_pair_registry_schema_invalid");
    return;
  }
  if (!Array.isArray(pairs) || registry.pairCount !== pairs.length || pairs.length !== 1) {
    verificationErrors.push(`tracked_pair_count_mismatch:expected=1:actual=${Array.isArray(pairs) ? pairs.length : "missing"}`);
    return;
  }
  const seen = new Set();
  for (const pair of pairs) {
    if (!pair?.pairId || seen.has(pair.pairId)) {
      verificationErrors.push(`tracked_pair_id_invalid:${pair?.pairId ?? "missing"}`);
      continue;
    }
    seen.add(pair.pairId);
    const documentPath = resolve(repoRoot, pair.documentPath ?? "");
    const authoringPath = resolve(repoRoot, pair.authoringPath ?? "");
    try {
      const document = await readJson(documentPath);
      const authoring = await readJson(authoringPath);
      const documentValid = validateFixtureValue(compiled, "DocumentIRV2", document, `tracked:${pair.pairId}:document`, verificationErrors, verificationResults);
      const authoringValid = validateFixtureValue(compiled, "IeltsAuthoringIRV2", authoring, `tracked:${pair.pairId}:authoring`, verificationErrors, verificationResults);
      const qualityValid = authoringValid && validateFixtureValue(compiled, "QualityReportV2", authoring.quality, `tracked:${pair.pairId}:quality`, verificationErrors, verificationResults);
      if (!documentValid || !authoringValid || !qualityValid) continue;
      if (document.documentId !== pair.documentId) verificationErrors.push(`tracked_pair_document_id_mismatch:${pair.pairId}`);
      if (authoring.sourceDocumentId !== pair.authoringSourceDocumentId) verificationErrors.push(`tracked_pair_authoring_document_id_mismatch:${pair.pairId}`);
      const documentSources = new Map((document.sourceFiles ?? []).map((source) => [source.sourceFileId, source.sha256]));
      const authoringHashes = new Set();
      const authoringSourceIds = new Set();
      const visit = (value) => {
        if (!value || typeof value !== "object") return;
        if (Array.isArray(value)) { for (const entry of value) visit(entry); return; }
        if (typeof value.sourceFileId === "string") authoringSourceIds.add(value.sourceFileId);
        if (typeof value.sourceHash === "string") authoringHashes.add(value.sourceHash);
        for (const child of Object.values(value)) visit(child);
      };
      visit(authoring);
      if (JSON.stringify([...documentSources.keys()].sort()) !== JSON.stringify([...new Set(pair.documentSourceFileIds ?? [])].sort())) verificationErrors.push(`tracked_pair_document_source_ids_mismatch:${pair.pairId}`);
      if (JSON.stringify([...authoringSourceIds].sort()) !== JSON.stringify([...new Set(pair.authoringSourceFileIds ?? [])].sort())) verificationErrors.push(`tracked_pair_authoring_source_ids_mismatch:${pair.pairId}`);
      if (documentSources.get(pair.documentSourceFileIds?.[0]) !== pair.sourceIdentity?.documentSourceHash) verificationErrors.push(`tracked_pair_document_source_hash_mismatch:${pair.pairId}`);
      if (!authoringHashes.has(pair.sourceIdentity?.authoringSourceHash)) verificationErrors.push(`tracked_pair_authoring_source_hash_mismatch:${pair.pairId}`);
    } catch (error) {
      verificationErrors.push(`tracked_pair_read_failed:${pair.pairId}:${error.message}`);
    }
  }
  verificationResults.push({ check: "tracked-shadow-pairs", status: verificationErrors.some((error) => error.startsWith("tracked_pair_")) ? "failed" : "passed", pairCount: pairs.length, evidenceKind: registry.evidenceKind });
}

function runNegativeContractProbes(compiled, stableValues, verificationErrors, verificationResults) {
  const probes = [
    {
      id: "document-rejects-unknown-page-transform-field",
      schemaName: "DocumentIRV2",
      mutate(value) {
        value.pages[0].pageTransform.unexpected = true;
      }
    },
    {
      id: "document-rejects-negative-inline-gap",
      schemaName: "DocumentIRV2",
      mutate(value) {
        value.pages[0].lines[0].inlineGapsPt = [-1];
      }
    },
    {
      id: "content-rejects-unknown-node-type",
      schemaName: "ContentDocV2",
      mutate(value) {
        value.root[0].type = "unknown_node";
      }
    },
    {
      id: "authoring-rejects-null-option-bank-title",
      schemaName: "IeltsAuthoringIRV2",
      mutate(value) {
        const group = value.taskGroups.find((candidate) => candidate.optionBank);
        if (!group) throw new Error("stable authoring fixture has no option bank");
        group.optionBank.title = null;
      }
    },
    {
      id: "quality-rejects-unknown-state",
      schemaName: "QualityReportV2",
      mutate(value) {
        value.state = "green";
      }
    }
  ];

  for (const probe of probes) {
    const validate = compiled.get(probe.schemaName);
    const source = stableValues.get(probe.schemaName);
    if (!validate || !source) {
      verificationErrors.push(`negative_probe_prerequisite_missing:${probe.id}`);
      continue;
    }
    const invalid = structuredClone(source);
    try {
      probe.mutate(invalid);
    } catch (error) {
      verificationErrors.push(`negative_probe_setup_failed:${probe.id}:${error.message}`);
      continue;
    }
    if (validate(invalid)) {
      verificationErrors.push(`negative_probe_unexpectedly_accepted:${probe.id}`);
      continue;
    }
    verificationResults.push({
      check: "negative-schema-probe",
      schemaName: probe.schemaName,
      probe: probe.id,
      rejectedBy: [...new Set((validate.errors ?? []).map((issue) => issue.keyword))],
      status: "passed"
    });
  }
}

async function validateAvailableRealShadows(compiled, verificationErrors, verificationResults) {
  const acceptanceRoot = join(repoRoot, "tmp", "phase4-real-pdf-acceptance");
  if (!(await pathExists(acceptanceRoot))) {
    verificationResults.push({
      check: "real-phase4-shadow-fixtures",
      status: "stable-fixture-fallback",
      reason: "optional tmp/phase4-real-pdf-acceptance is absent; tracked contract-slice pairs are the hard requirement"
    });
    return;
  }

  const directories = (await readdir(acceptanceRoot, { withFileTypes: true }))
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
  let discoveredPairs = 0;
  let validatedPairs = 0;
  for (const directory of directories) {
    const documentPath = join(acceptanceRoot, directory, "document-ir-v2.physical.json");
    const authoringPath = join(acceptanceRoot, directory, "authoring-ir-v2.shadow.json");
    const documentExists = await pathExists(documentPath);
    const authoringExists = await pathExists(authoringPath);
    if (!documentExists && !authoringExists) continue;
    if (!documentExists || !authoringExists) {
      verificationResults.push({ check: "real-phase4-shadow-fixtures", status: "optional-pair-incomplete", directory, documentExists, authoringExists });
      continue;
    }
    discoveredPairs += 1;

    try {
      const document = await readJson(documentPath);
      const authoring = await readJson(authoringPath);
      const documentValid = validateFixtureValue(compiled, "DocumentIRV2", document, `real:${directory}:document`, verificationErrors, verificationResults);
      const authoringValid = validateFixtureValue(compiled, "IeltsAuthoringIRV2", authoring, `real:${directory}:authoring`, verificationErrors, verificationResults);
      let qualityValid = false;
      if (authoring.quality === undefined) {
        verificationResults.push({ check: "real-phase4-shadow-fixtures", status: "optional-quality-missing", directory });
      } else {
        qualityValid = validateFixtureValue(compiled, "QualityReportV2", authoring.quality, `real:${directory}:quality`, verificationErrors, verificationResults);
      }
      if (documentValid && authoringValid && qualityValid) validatedPairs += 1;
    } catch (error) {
      verificationResults.push({ check: "real-phase4-shadow-fixtures", status: "optional-pair-read-failed", directory, reason: error.message });
    }
  }
  verificationResults.push({
    check: "real-phase4-shadow-fixtures",
    status: discoveredPairs === 0
      ? "stable-fixture-fallback"
      : validatedPairs === discoveredPairs
        ? "passed"
        : "failed",
    discoveredPairs,
    validatedPairs,
    reason: discoveredPairs > 0 ? "optional real acceptance diagnostics" : "no optional real shadow pairs were available"
  });
}

const manifest = await readJson(manifestPath);
const errors = [];
const results = [];
const loadedSchemas = new Map();
const validators = new Map();
const expectedRootVersions = {
  DocumentIRV2: "DocumentIRV2",
  ContentDocV2: "ContentDocV2",
  IeltsAuthoringIRV2: "IeltsAuthoringIRV2",
  QualityReportV2: "QualityReportV2"
};
const expectedSchemaNames = ["CommonV2", ...Object.keys(expectedRootVersions)];
if (manifest.schemaBundleVersion !== "2026.08.0") {
  errors.push(`unexpected_schema_bundle_version:${manifest.schemaBundleVersion}`);
}
const manifestSchemaNames = Object.keys(manifest.schemas ?? {});
for (const schemaName of expectedSchemaNames) {
  if (!manifestSchemaNames.includes(schemaName)) errors.push(`manifest_schema_missing:${schemaName}`);
}
for (const schemaName of manifestSchemaNames) {
  if (!expectedSchemaNames.includes(schemaName)) errors.push(`manifest_schema_unexpected:${schemaName}`);
}
const compatibility = manifest.compatibility ?? {};
for (const [key, expected] of Object.entries({
  v1ArtifactsMutable: false,
  v1ToV2Disposition: "needs_review",
  v2ToV1Disposition: "blocked_until_lossless_compiler",
  artifactStoreLayoutVersion: "JobArtifactLayoutV1",
  currentRevisionSchemaVersion: "JobCurrentRevisionV1",
  revisionRecordSchemaVersion: "AuthoringRevisionRecordV1",
  legacyJobOpenMode: "read_only_compatible"
})) {
  if (compatibility[key] !== expected) {
    errors.push(`compatibility_contract_mismatch:${key}:expected=${expected}:actual=${compatibility[key] ?? "missing"}`);
  }
}

for (const [schemaName, entry] of Object.entries(manifest.schemas ?? {})) {
  const relativePath = String(entry.path ?? "");
  const schemaPath = join(contractRoot, relativePath);
  try {
    const bytes = await readFile(schemaPath);
    const actualHash = sha256(bytes);
    if (entry.sha256 !== actualHash) {
      errors.push(`hash_mismatch:${schemaName}:expected=${entry.sha256}:actual=${actualHash}`);
    }
    const schema = JSON.parse(bytes.toString("utf8"));
    if (schema.$schema !== manifest.schemaFormat) {
      errors.push(`schema_format_mismatch:${schemaName}`);
    }
    if (!schema.$id || typeof schema.$id !== "string") {
      errors.push(`missing_schema_id:${schemaName}`);
    }
    loadedSchemas.set(schemaName, schema);
    const expectedRootVersion = expectedRootVersions[schemaName];
    const actualRootVersion = schema.properties?.schemaVersion?.const;
    if (expectedRootVersion && actualRootVersion !== expectedRootVersion) {
      errors.push(`schema_version_contract_mismatch:${schemaName}:expected=${expectedRootVersion}:actual=${actualRootVersion ?? "missing"}`);
    }
    for (const ref of collectRefs(schema)) {
      const refPath = localRefPath(ref);
      if (!refPath) continue;
      const resolvedRef = normalize(join(contractRoot, refPath));
      const relativeRef = relative(contractRoot, resolvedRef);
      if (relativeRef === ".." || relativeRef.startsWith(`..\\`) || relativeRef.startsWith("../") || isAbsolute(relativeRef)) {
        errors.push(`ref_escapes_contract_root:${schemaName}:${ref}`);
      } else {
        try {
          await readFile(resolvedRef);
        } catch {
          errors.push(`missing_local_ref:${schemaName}:${ref}`);
        }
      }
    }
    results.push({ schemaName, path: relativePath, sha256: actualHash });
  } catch (error) {
    errors.push(`schema_read_failed:${schemaName}:${error.message}`);
  }
}

compileContractSchemas(loadedSchemas, validators, errors, results);
const stableFixtureValues = await validateStableContractFixtures(validators, errors, results);
runNegativeContractProbes(validators, stableFixtureValues, errors, results);
await validateTrackedShadowPairs(validators, errors, results);
await validateAvailableRealShadows(validators, errors, results);

const peerRootArgument = argumentValue("--peer-root");
const localOnly = process.argv.includes("--local-only");
const configuredPeerContractRoot = manifest.peerRepositories?.nasForWendao?.expectedRelativeContractRoot;
const defaultPeerRoot = configuredPeerContractRoot
  ? resolve(repoRoot, "..", "NAS", configuredPeerContractRoot)
  : null;
const peerRoot = peerRootArgument
  ? (isAbsolute(peerRootArgument) ? peerRootArgument : resolve(repoRoot, peerRootArgument))
  : defaultPeerRoot;
if (!localOnly) {
  if (!peerRoot) {
    errors.push("peer_contract_root_not_configured:nasForWendao");
  }
  let peerManifest = null;
  try {
    peerManifest = await readJson(join(peerRoot, "contract-manifest.json"));
    if (JSON.stringify(peerManifest) !== JSON.stringify(manifest)) {
      errors.push("peer_contract_manifest_mismatch:nasForWendao");
    }
  } catch (error) {
    errors.push(`peer_manifest_read_failed:${error.message}`);
  }
  for (const [schemaName, entry] of Object.entries(manifest.schemas ?? {})) {
    const peerPath = join(peerRoot, String(entry.path ?? ""));
    try {
      const peerHash = sha256(await readFile(peerPath));
      const local = results.find((result) => result.schemaName === schemaName);
      if (!local || local.sha256 !== peerHash) {
        errors.push(`peer_hash_mismatch:${schemaName}:local=${local?.sha256 ?? "missing"}:peer=${peerHash}`);
      }
    } catch (error) {
      errors.push(`peer_schema_read_failed:${schemaName}:${error.message}`);
    }
  }
} else {
  results.push({ peerRepository: "not_checked", reason: "explicit --local-only" });
}

const report = {
  schemaVersion: "Phase1SchemaContractVerificationReportV1",
  manifestPath: manifestPath,
  schemaBundleVersion: manifest.schemaBundleVersion,
  peerChecked: !localOnly,
  peerRoot: peerRoot ?? null,
  schemas: results.filter((result) => result.schemaName && result.sha256),
  checks: results,
  errorCount: errors.length,
  errors
};
console.log(JSON.stringify(report, null, 2));
if (errors.length > 0) process.exitCode = 1;
