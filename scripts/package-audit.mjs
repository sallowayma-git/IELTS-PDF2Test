import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultTauriConfigPath = path.join(root, "src-tauri", "tauri.conf.json");
const releaseRoot = path.join(root, "src-tauri", "target", "release");
const bundleRoot = path.join(releaseRoot, "bundle");

const usage = `Usage: node scripts/package-audit.mjs [--platform macos|windows] [--config <path>]

Audits release package artifacts for forbidden bundled runtimes and emits a JSON manifest.

Options:
  --platform <name>  Audit target platform. Defaults to the host platform.
  --config <path>    Tauri config or config override to merge over src-tauri/tauri.conf.json.
  -h, --help         Show this help.
`;

function fail(message, details) {
  console.error(`[package-audit] ${message}`);
  if (details) console.error(details);
  process.exit(1);
}

function parseArgs(argv) {
  const out = {
    platform: process.platform === "win32" ? "windows" : "macos",
    configPath: defaultTauriConfigPath,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "-h" || arg === "--help") {
      console.log(usage);
      process.exit(0);
    }
    if (arg === "--platform") {
      out.platform = argv[++i];
      continue;
    }
    if (arg.startsWith("--platform=")) {
      out.platform = arg.slice("--platform=".length);
      continue;
    }
    if (arg === "--config") {
      out.configPath = resolveCliPath(argv[++i]);
      continue;
    }
    if (arg.startsWith("--config=")) {
      out.configPath = resolveCliPath(arg.slice("--config=".length));
      continue;
    }
    fail(`unknown argument: ${arg}`, usage);
  }

  if (!["macos", "windows"].includes(out.platform)) {
    fail(`unsupported platform: ${out.platform}`, "Expected macos or windows.");
  }
  return out;
}

function resolveCliPath(value) {
  if (!value) fail("--config requires a path");
  return path.resolve(root, value);
}

function readJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`failed to read JSON: ${file}`, error.message);
  }
}

function deepMerge(base, override) {
  if (!override || typeof override !== "object" || Array.isArray(override)) return override;
  const merged = { ...base };
  for (const [key, value] of Object.entries(override)) {
    if (
      value
      && typeof value === "object"
      && !Array.isArray(value)
      && base?.[key]
      && typeof base[key] === "object"
      && !Array.isArray(base[key])
    ) {
      merged[key] = deepMerge(base[key], value);
    } else {
      merged[key] = value;
    }
  }
  return merged;
}

function loadTauriConfig(configPath) {
  const baseConfig = readJson(defaultTauriConfigPath);
  if (path.resolve(configPath) === path.resolve(defaultTauriConfigPath)) {
    return {
      config: baseConfig,
      configFiles: [defaultTauriConfigPath],
    };
  }
  return {
    config: deepMerge(baseConfig, readJson(configPath)),
    configFiles: [defaultTauriConfigPath, configPath],
  };
}

function assertExists(target, label) {
  if (!fs.existsSync(target)) fail(`${label} is missing`, target);
}

function walk(target) {
  const out = [];
  if (!fs.existsSync(target)) return out;
  const stat = fs.statSync(target);
  if (stat.isFile()) return [target];
  for (const entry of fs.readdirSync(target, { withFileTypes: true })) {
    const full = path.join(target, entry.name);
    out.push(full);
    if (entry.isDirectory()) out.push(...walk(full));
  }
  return out;
}

function existingTargets(targets) {
  return targets.filter((target) => fs.existsSync(target));
}

function filesOnly(targets) {
  return targets.flatMap((target) => walk(target)).filter((file) => fs.statSync(file).isFile());
}

function fileSize(file) {
  return fs.statSync(file).size;
}

function directoryPayloadSize(dir) {
  return walk(dir).reduce((sum, file) => sum + (fs.statSync(file).isFile() ? fileSize(file) : 0), 0);
}

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function hashFileIfExists(file) {
  if (!fs.existsSync(file)) return null;
  return {
    path: file,
    sizeBytes: fileSize(file),
    sha256: sha256File(file),
  };
}

function artifactManifest(file) {
  return {
    path: file,
    sizeBytes: fileSize(file),
    sha256: sha256File(file),
  };
}

function listArtifacts(dir, predicate) {
  if (!fs.existsSync(dir)) return [];
  return filesOnly([dir]).filter((file) => predicate(path.basename(file), file)).sort();
}

function findDmgArtifacts(productName, version) {
  const dmgDir = path.join(bundleRoot, "dmg");
  const prefix = `${productName}_${version}_`;
  return listArtifacts(dmgDir, (name) => name.startsWith(prefix) && name.endsWith(".dmg"));
}

function findWindowsArtifacts() {
  const nsisDir = path.join(bundleRoot, "nsis");
  const msiDir = path.join(bundleRoot, "msi");
  const nsisSetupArtifacts = listArtifacts(
    nsisDir,
    (name) => name.toLowerCase().endsWith(".exe") && /setup|installer/.test(name.toLowerCase()),
  );
  const msiArtifacts = listArtifacts(msiDir, (name) => name.toLowerCase().endsWith(".msi"));

  return {
    nsisSetupArtifacts,
    msiArtifacts,
    all: [...nsisSetupArtifacts, ...msiArtifacts].sort(),
  };
}

const forbiddenRuntimePatterns = [
  "node runtime",
  "python runtime",
  "node_modules",
  "venv",
  "tesseract",
  "tessdata",
  "ocr engine",
];

// pdfium (pdfium.dll / libpdfium.dylib / libpdfium.so) is the BUNDLED native
// PDF backend — intentionally shipped as a Tauri resource so all PDF features
// work on machines with no Python. It is allow-listed ONLY under the
// lib/pdfium-<platform>/ resources path. Python/Node/tesseract stay forbidden.
const PDFIUM_ALLOW_PATH = /(^|\/)lib\/pdfium-(windows|macos|linux)\/(pdfium\.dll|libpdfium\.dylib|libpdfium\.so)$/i;
const PDFIUM_LIB_BASENAMES = new Set(["pdfium.dll", "libpdfium.dylib", "libpdfium.so"]);

function forbiddenRuntimeReason(file) {
  const normalized = file.split(path.sep).join("/").toLowerCase();
  const base = path.basename(normalized);

  // Allow-list the bundled pdfium binary in its resources folder.
  if (PDFIUM_ALLOW_PATH.test(normalized) && PDFIUM_LIB_BASENAMES.has(base)) {
    return null;
  }
  if (normalized.includes("/node_modules/")) return "node_modules";
  if (normalized.includes("python.framework")) return "python framework";
  if (/(^|\/)(\.venv|venv)(\/|$)/.test(normalized)) return "python virtualenv";
  if (/(^|\/)tessdata(\/|$)/.test(normalized)) return "tesseract language data";
  // Any pdfium binary OUTSIDE the allow-listed resources path is still forbidden.
  if (/(^|\/)pdfium(\/|$)/.test(normalized) || /^(lib)?pdfium(\.|$)/.test(base)) return "pdfium (outside bundled resources path)";
  if (/(^|\/)ocr(engine|_engine)?(\.exe)?$/.test(normalized)) return "ocr engine";
  if (/^lib?tesseract(\.|$)/.test(base)) return "tesseract";
  if (["node", "node.exe", "python", "python3", "python.exe", "py.exe", "tesseract", "tesseract.exe"].includes(base)) {
    return base;
  }
  return null;
}

function assertNoForbiddenRuntimes(files) {
  const forbidden = [];
  for (const file of files) {
    const reason = forbiddenRuntimeReason(file);
    if (reason) forbidden.push(`${file} (${reason})`);
  }
  if (forbidden.length) {
    fail("package contains forbidden bundled runtime/OCR dependencies", forbidden.join("\n"));
  }
}

function assertNoJunkMetadata(files) {
  const junk = files.filter((file) => [".ds_store", "thumbs.db"].includes(path.basename(file).toLowerCase()));
  if (junk.length) {
    fail("package contains junk metadata files", junk.join("\n"));
  }
}

function gitValue(args) {
  try {
    return execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    return null;
  }
}

function gitMetadata() {
  const status = gitValue(["status", "--short"]);
  return {
    commit: gitValue(["rev-parse", "HEAD"]),
    dirty: status === null ? null : status.length > 0,
    statusShort: status,
  };
}

function packageLockSummary() {
  const file = path.join(root, "package-lock.json");
  if (!fs.existsSync(file)) return null;
  const lock = readJson(file);
  const rootPackage = lock.packages?.[""] ?? {};
  return {
    ...hashFileIfExists(file),
    lockfileVersion: lock.lockfileVersion,
    rootPackageName: rootPackage.name,
    rootPackageVersion: rootPackage.version,
    dependencyCount: Object.keys(rootPackage.dependencies ?? {}).length,
    devDependencyCount: Object.keys(rootPackage.devDependencies ?? {}).length,
  };
}

function cargoLockSummary() {
  const file = path.join(root, "src-tauri", "Cargo.lock");
  if (!fs.existsSync(file)) return null;
  const text = fs.readFileSync(file, "utf8");
  return {
    ...hashFileIfExists(file),
    lockfileVersion: text.match(/^version = (\d+)/m)?.[1] ?? null,
    packageCount: (text.match(/^\[\[package\]\]/gm) ?? []).length,
  };
}

function cargoPackageSummary() {
  const file = path.join(root, "src-tauri", "Cargo.toml");
  if (!fs.existsSync(file)) return null;
  const text = fs.readFileSync(file, "utf8");
  return {
    path: file,
    name: text.match(/^\[package\][\s\S]*?^name = "([^"]+)"/m)?.[1] ?? null,
    version: text.match(/^\[package\][\s\S]*?^version = "([^"]+)"/m)?.[1] ?? null,
  };
}

function lockFileSummary() {
  return {
    npm: packageLockSummary(),
    cargo: cargoLockSummary(),
  };
}

function webviewInstallMode(config) {
  return config.bundle?.windows?.webviewInstallMode ?? {
    type: "downloadBootstrapper",
    silent: true,
    source: "tauri default",
  };
}

function baseReport(platform, config, configFiles) {
  return {
    schemaVersion: "Epic8PackageAuditV2",
    passed: true,
    platform,
    productName: config.productName,
    version: config.version,
    configFiles,
    externalBinCount: config.bundle?.externalBin?.length ?? 0,
    bundleTargets: config.bundle?.targets ?? null,
    checkedForbiddenPatterns: forbiddenRuntimePatterns,
    git: gitMetadata(),
    lockFiles: lockFileSummary(),
  };
}

function assertNoExternalBin(config) {
  if (Array.isArray(config.bundle?.externalBin) && config.bundle.externalBin.length > 0) {
    fail("production package must not declare externalBin runtime dependencies", JSON.stringify(config.bundle.externalBin));
  }
}

function auditMacos(config, configFiles) {
  const appPath = path.join(bundleRoot, "macos", `${config.productName}.app`);
  const dmgDir = path.join(bundleRoot, "dmg");

  assertNoExternalBin(config);
  assertExists(appPath, "macOS app bundle");

  const dmgPaths = findDmgArtifacts(config.productName, config.version);
  if (dmgPaths.length === 0) {
    fail("macOS DMG bundle is missing", `Expected ${config.productName}_${config.version}_*.dmg in ${dmgDir}`);
  }

  const files = walk(appPath);
  assertNoForbiddenRuntimes(files);
  assertNoJunkMetadata(files);

  const resources = walk(path.join(appPath, "Contents", "Resources"));
  const sidecarScripts = resources.filter((file) => /\/sidecars\/.*\.(mjs|py|md)$/.test(file.split(path.sep).join("/")));
  const dmgArtifacts = dmgPaths.map((file) => artifactManifest(file));

  return {
    ...baseReport("macos", config, configFiles),
    appPath,
    dmgPaths,
    appSizeBytes: directoryPayloadSize(appPath),
    artifactSizeBytesTotal: dmgArtifacts.reduce((sum, artifact) => sum + artifact.sizeBytes, 0),
    dmgArtifacts,
    sidecarScripts: sidecarScripts.map((file) => path.relative(appPath, file)).sort(),
  };
}

function auditWindows(config, configFiles) {
  assertNoExternalBin(config);

  const windowsArtifacts = findWindowsArtifacts();
  if (windowsArtifacts.all.length === 0) {
    fail(
      "Windows installer bundle is missing",
      `Expected NSIS *setup*.exe in ${path.join(bundleRoot, "nsis")} or MSI *.msi in ${path.join(bundleRoot, "msi")}`,
    );
  }

  const payloadRoots = existingTargets([
    path.join(bundleRoot, "nsis"),
    path.join(bundleRoot, "msi"),
    path.join(releaseRoot, "resources"),
    path.join(releaseRoot, "sidecars"),
  ]);
  const files = filesOnly(payloadRoots);
  assertNoForbiddenRuntimes(files);
  assertNoJunkMetadata(files);

  const artifacts = windowsArtifacts.all.map((file) => ({
    ...artifactManifest(file),
    type: file.toLowerCase().endsWith(".msi") ? "msi" : "nsis",
  }));
  const sidecarRoot = path.join(releaseRoot, "resources", "sidecars");
  const sidecarFiles = fs.existsSync(sidecarRoot)
    ? filesOnly([sidecarRoot]).map((file) => path.relative(sidecarRoot, file)).sort()
    : [];
  const sourceSidecarRoot = path.join(root, "sidecars");

  return {
    ...baseReport("windows", config, configFiles),
    webview2: {
      installMode: webviewInstallMode(config),
      offlineInstaller: webviewInstallMode(config)?.type === "offlineInstaller",
      fixedRuntime: webviewInstallMode(config)?.type === "fixedRuntime",
    },
    installerArtifacts: artifacts,
    nsisSetupArtifacts: windowsArtifacts.nsisSetupArtifacts,
    msiArtifacts: windowsArtifacts.msiArtifacts,
    artifactSizeBytesTotal: artifacts.reduce((sum, artifact) => sum + artifact.sizeBytes, 0),
    checkedPayloadRoots: payloadRoots,
    checkedPayloadSizeBytes: payloadRoots.reduce((sum, target) => sum + (fs.statSync(target).isDirectory() ? directoryPayloadSize(target) : fileSize(target)), 0),
    packagedSidecarFiles: sidecarFiles,
    sourceSidecarSizeBytes: fs.existsSync(sourceSidecarRoot) ? directoryPayloadSize(sourceSidecarRoot) : null,
    cargoPackage: cargoPackageSummary(),
  };
}

const args = parseArgs(process.argv.slice(2));
const { config: tauriConfig, configFiles } = loadTauriConfig(args.configPath);
const report = args.platform === "windows"
  ? auditWindows(tauriConfig, configFiles)
  : auditMacos(tauriConfig, configFiles);

console.log(JSON.stringify(report, null, 2));
