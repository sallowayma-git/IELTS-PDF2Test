import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const root = path.resolve(new URL("..", import.meta.url).pathname);
const tauriConfigPath = path.join(root, "src-tauri", "tauri.conf.json");
const tauriConfig = JSON.parse(fs.readFileSync(tauriConfigPath, "utf8"));
const productName = tauriConfig.productName;
const version = tauriConfig.version;
const bundleRoot = path.join(root, "src-tauri", "target", "release", "bundle");
const appPath = path.join(bundleRoot, "macos", `${productName}.app`);
const dmgDir = path.join(bundleRoot, "dmg");
const dmgScript = path.join(dmgDir, "bundle_dmg.sh");
const helperSource = path.join(root, "scripts", "macos-open-unsigned.command");
const helperName = "Open Unsigned App.command";

function fail(message) {
  console.error(`[repack-macos-dmg] ${message}`);
  process.exit(1);
}

function assertExists(target, label) {
  if (!fs.existsSync(target)) fail(`${label} not found: ${target}`);
}

function findDmgArtifacts() {
  if (!fs.existsSync(dmgDir)) return [];
  const prefix = `${productName}_${version}_`;
  return fs.readdirSync(dmgDir)
    .filter((name) => name.startsWith(prefix) && name.endsWith(".dmg"))
    .map((name) => path.join(dmgDir, name))
    .sort();
}

assertExists(appPath, "macOS app bundle");
assertExists(dmgScript, "Tauri DMG helper script");
assertExists(helperSource, "unsigned app helper script");

fs.chmodSync(helperSource, 0o755);

const dmgPaths = findDmgArtifacts();
if (dmgPaths.length === 0) {
  fail(`no DMG artifacts found in ${dmgDir}`);
}

for (const dmgPath of dmgPaths) {
  const tmpPath = dmgPath.replace(/\.dmg$/i, ".tmp.dmg");
  fs.rmSync(tmpPath, { force: true });

  const result = spawnSync(
    "bash",
    [
      dmgScript,
      "--volname",
      productName,
      "--window-size",
      "720",
      "420",
      "--icon",
      `${productName}.app`,
      "170",
      "170",
      "--app-drop-link",
      "500",
      "170",
      "--add-file",
      helperName,
      helperSource,
      "350",
      "320",
      "--hide-extension",
      helperName,
      tmpPath,
      path.dirname(appPath),
    ],
    { cwd: root, stdio: "inherit" },
  );

  if (result.status !== 0) {
    fs.rmSync(tmpPath, { force: true });
    fail(`failed to repack ${dmgPath}`);
  }

  fs.renameSync(tmpPath, dmgPath);
  console.log(`[repack-macos-dmg] added ${helperName} to ${dmgPath}`);
}
