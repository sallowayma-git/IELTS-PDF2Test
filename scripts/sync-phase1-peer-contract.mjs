import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const contractRoot = path.join(repoRoot, "contracts");
const manifestPath = path.join(contractRoot, "contract-manifest.json");
const manifest = readJson(manifestPath);
const configuredRelativeRoot = manifest.peerRepositories?.nasForWendao?.expectedRelativeContractRoot;
const defaultPeerRepositoryRoot = path.resolve(repoRoot, "..", "NAS");
const peerRootArgument = argumentValue("--peer-root");
const peerRoot = path.resolve(peerRootArgument ?? path.join(defaultPeerRepositoryRoot, configuredRelativeRoot ?? ""));

if (!configuredRelativeRoot) throw new Error("NAS peer contract root is not configured in contract-manifest.json");
if (!peerRootArgument && !fs.existsSync(path.join(defaultPeerRepositoryRoot, "package.json"))) {
  throw new Error(`NAS peer repository is unavailable: ${defaultPeerRepositoryRoot}`);
}

fs.mkdirSync(peerRoot, { recursive: true });
const files = ["contract-manifest.json", ...Object.values(manifest.schemas).map((entry) => entry.path)];
const synced = [];
for (const relativePath of files) {
  const sourcePath = path.join(contractRoot, relativePath);
  const targetPath = path.join(peerRoot, relativePath);
  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  fs.copyFileSync(sourcePath, targetPath);
  synced.push({ path: relativePath, sha256: sha256File(targetPath) });
}

console.log(JSON.stringify({
  schemaVersion: "Phase1PeerContractSyncReportV1",
  peerRoot,
  synced
}, null, 2));

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function sha256File(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}
