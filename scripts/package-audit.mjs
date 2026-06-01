import fs from "node:fs";
import path from "node:path";

const root = path.resolve(new URL("..", import.meta.url).pathname);
const tauriConfigPath = path.join(root, "src-tauri", "tauri.conf.json");
const tauriConfig = JSON.parse(fs.readFileSync(tauriConfigPath, "utf8"));
const bundleRoot = path.join(root, "src-tauri", "target", "release", "bundle");
const appPath = path.join(bundleRoot, "macos", `${tauriConfig.productName}.app`);
const dmgDir = path.join(bundleRoot, "dmg");

function fail(message, details) {
  console.error(`[package-audit] ${message}`);
  if (details) console.error(details);
  process.exit(1);
}

function assertExists(target, label) {
  if (!fs.existsSync(target)) fail(`${label} is missing`, target);
}

function walk(dir) {
  const out = [];
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    out.push(full);
    if (entry.isDirectory()) out.push(...walk(full));
  }
  return out;
}

function fileSize(file) {
  return fs.statSync(file).size;
}

function directoryPayloadSize(dir) {
  return walk(dir).reduce((sum, file) => sum + (fs.statSync(file).isFile() ? fileSize(file) : 0), 0);
}

function findDmgArtifacts() {
  if (!fs.existsSync(dmgDir)) return [];
  const prefix = `${tauriConfig.productName}_${tauriConfig.version}_`;
  return fs.readdirSync(dmgDir)
    .filter((name) => name.startsWith(prefix) && name.endsWith(".dmg"))
    .map((name) => path.join(dmgDir, name))
    .sort();
}

if (Array.isArray(tauriConfig.bundle?.externalBin) && tauriConfig.bundle.externalBin.length > 0) {
  fail("production package must not declare externalBin runtime dependencies", JSON.stringify(tauriConfig.bundle.externalBin));
}

assertExists(appPath, "macOS app bundle");
const dmgPaths = findDmgArtifacts();
if (dmgPaths.length === 0) {
  fail("macOS DMG bundle is missing", `Expected ${tauriConfig.productName}_${tauriConfig.version}_*.dmg in ${dmgDir}`);
}

const files = walk(appPath);
const forbidden = files.filter((file) => {
  const normalized = file.split(path.sep).join("/").toLowerCase();
  const base = path.basename(normalized);
  return normalized.includes("/node_modules/")
    || normalized.includes("python.framework")
    || normalized.includes("/.venv/")
    || normalized.includes("/venv/")
    || ["node", "node.exe", "python", "python3", "python.exe", "tesseract", "tesseract.exe", "pdfium", "pdfium.dll"].includes(base)
    || /(^|\/)ocr(engine|_engine)?(\.exe)?$/.test(normalized);
});
if (forbidden.length) {
  fail("package contains forbidden bundled runtime/OCR dependencies", forbidden.join("\n"));
}

const junk = files.filter((file) => [".ds_store", "thumbs.db"].includes(path.basename(file).toLowerCase()));
if (junk.length) {
  fail("package contains junk metadata files", junk.join("\n"));
}

const resources = walk(path.join(appPath, "Contents", "Resources"));
const sidecarScripts = resources.filter((file) => /\/sidecars\/.*\.(mjs|py|md)$/.test(file.split(path.sep).join("/")));

const report = {
  schemaVersion: "Epic8PackageAuditV1",
  passed: true,
  productName: tauriConfig.productName,
  version: tauriConfig.version,
  externalBinCount: tauriConfig.bundle?.externalBin?.length ?? 0,
  appPath,
  dmgPaths,
  appSizeBytes: directoryPayloadSize(appPath),
  dmgArtifacts: dmgPaths.map((file) => ({ path: file, sizeBytes: fileSize(file) })),
  sidecarScripts: sidecarScripts.map((file) => path.relative(appPath, file)).sort(),
  checkedForbiddenPatterns: ["node runtime", "python runtime", "node_modules", "venv", "tesseract", "ocr engine", "pdfium"]
};
console.log(JSON.stringify(report, null, 2));
