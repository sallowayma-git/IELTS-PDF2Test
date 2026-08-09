import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { dirname, isAbsolute, join, normalize, resolve } from "node:path";
import { fileURLToPath } from "node:url";

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

const manifest = await readJson(manifestPath);
const errors = [];
const results = [];
const expectedRootVersions = {
  DocumentIRV2: "DocumentIRV2",
  ContentDocV2: "ContentDocV2",
  IeltsAuthoringIRV2: "IeltsAuthoringIRV2",
  QualityReportV2: "QualityReportV2"
};
if (manifest.schemaBundleVersion !== "2026.08.0") {
  errors.push(`unexpected_schema_bundle_version:${manifest.schemaBundleVersion}`);
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
    const expectedRootVersion = expectedRootVersions[schemaName];
    const actualRootVersion = schema.properties?.schemaVersion?.const;
    if (expectedRootVersion && actualRootVersion !== expectedRootVersion) {
      errors.push(`schema_version_contract_mismatch:${schemaName}:expected=${expectedRootVersion}:actual=${actualRootVersion ?? "missing"}`);
    }
    for (const ref of collectRefs(schema)) {
      const refPath = localRefPath(ref);
      if (!refPath) continue;
      const resolvedRef = normalize(join(contractRoot, refPath));
      if (!resolvedRef.startsWith(normalize(contractRoot))) {
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

const peerRootArgument = argumentValue("--peer-root");
if (peerRootArgument) {
  const peerRoot = isAbsolute(peerRootArgument) ? peerRootArgument : resolve(repoRoot, peerRootArgument);
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
  results.push({ peerRepository: "not_checked", reason: "no --peer-root supplied" });
}

const report = {
  schemaVersion: "Phase1SchemaContractVerificationReportV1",
  manifestPath: manifestPath,
  schemaBundleVersion: manifest.schemaBundleVersion,
  peerChecked: Boolean(peerRootArgument),
  schemas: results,
  errorCount: errors.length,
  errors
};
console.log(JSON.stringify(report, null, 2));
if (errors.length > 0) process.exitCode = 1;
