#!/usr/bin/env node
/**
 * Fetches the pdfium native library for the current platform and extracts it
 * into src-tauri/lib/pdfium-<platform>/ so it is bundled as a Tauri resource.
 *
 * The pdfium-render Rust crate binds to this library at runtime (see
 * src-tauri/src/pdf_geometry.rs -> pdfium_library_path), giving the app a
 * real-coordinate PDF backend that works on machines with NO Python.
 *
 * Sources binaries from bblanchon/pdfium-binaries (official chromium builds).
 * Idempotent: skips download when the library is already present and the
 * stamped version matches.
 *
 * Usage: node scripts/fetch-pdfium.mjs [--force] [--platform <win|mac|linux>]
 */
import { createWriteStream, existsSync, readFileSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import https from "node:https";

// Pin a known-good chromium release. Update deliberately — each bump should
// be validated against the sample PDFs in fixtures/parser.
const PDFIUM_RELEASE = "chromium/7920";

const PLATFORM_OVERRIDE = process.argv.includes("--platform")
  ? process.argv[process.argv.indexOf("--platform") + 1]
  : null;
const FORCE = process.argv.includes("--force");

function detectPlatform() {
  if (PLATFORM_OVERRIDE) return PLATFORM_OVERRIDE;
  if (process.platform === "win32") return "win";
  if (process.platform === "darwin") return "mac";
  return "linux";
}

function platformAsset(platform) {
  // bblanchon/pdfium-binaries asset names (x64). arm64 variants exist but are
  // not fetched here; add them if you ship arm64 builds.
  return `pdfium-${platform}-x64.tgz`;
}

function libraryFileName(platform) {
  if (platform === "win") return "pdfium.dll";
  if (platform === "mac") return "libpdfium.dylib";
  return "libpdfium.so";
}

function platformFolder(platform) {
  // MUST match the folder names checked by pdf_geometry.rs::platform_pdfium_folder.
  return `pdfium-${platform === "win" ? "windows" : platform === "mac" ? "macos" : "linux"}`;
}

function download(url, dest) {
  return new Promise((resolve, reject) => {
    const file = createWriteStream(dest);
    const req = https.get(url, (response) => {
      // Follow redirects (GitHub releases 302 to a CDN).
      if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
        file.close();
        return resolve(download(response.headers.location, dest));
      }
      if (response.statusCode !== 200) {
        file.close();
        return reject(new Error(`download failed: HTTP ${response.statusCode} for ${url}`));
      }
      response.pipe(file);
      file.on("finish", () => file.close(() => resolve(dest)));
    });
    req.on("error", (error) => {
      file.close();
      reject(error);
    });
  });
}

function extractTgz(tgzPath, destDir) {
  mkdirSync(destDir, { recursive: true });
  // Use the system tar (available in git-bash on Windows, and on mac/linux).
  const result = spawnSync("tar", ["-xzf", tgzPath, "-C", destDir], {
    stdio: "inherit",
  });
  if (result.status !== 0) {
    throw new Error(`tar extraction failed (exit ${result.status}). Ensure 'tar' is on PATH.`);
  }
}

async function main() {
  const platform = detectPlatform();
  const asset = platformAsset(platform);
  const url = `https://github.com/bblanchon/pdfium-binaries/releases/download/${PDFIUM_RELEASE}/${asset}`;
  const libDir = join("src-tauri", "lib", platformFolder(platform));
  const libFile = join(libDir, libraryFileName(platform));
  const versionStamp = join(libDir, ".version");

  if (existsSync(libFile) && existsSync(versionStamp) && !FORCE) {
    const stamped = readFileSync(versionStamp, "utf8").trim();
    if (stamped === PDFIUM_RELEASE) {
      console.log(`[fetch-pdfium] ${platform}: already present (${PDFIUM_RELEASE}), skipping.`);
      return;
    }
  }

  mkdirSync(libDir, { recursive: true });
  // Keep the temp tgz under the project (relative path) so Windows `tar`
  // doesn't misparse a `C:\Users\...` tmpdir as a remote host.
  const tgzPath = join(libDir, asset);
  console.log(`[fetch-pdfium] ${platform}: downloading ${url}`);
  await download(url, tgzPath);
  console.log(`[fetch-pdfium] ${platform}: extracting to ${libDir}`);
  // Clean stale extractions first.
  rmSync(join(libDir, "bin"), { recursive: true, force: true });
  rmSync(join(libDir, "lib"), { recursive: true, force: true });
  extractTgz(tgzPath, libDir);

  // The tgz lays out bin/pdfium.dll (win) or lib/libpdfium.dylib/so (mac/linux).
  // Flatten so the library sits directly under src-tauri/lib/pdfium-<platform>/,
  // which is where pdf_geometry.rs and the tauri resource config look for it.
  const nested = {
    win: join(libDir, "bin", "pdfium.dll"),
    mac: join(libDir, "lib", "libpdfium.dylib"),
    linux: join(libDir, "lib", "libpdfium.so"),
  }[platform];
  if (existsSync(nested) && nested !== libFile) {
    rmSync(libFile, { force: true });
    // Rename across dirs.
    const content = readFileSync(nested);
    writeFileSync(libFile, content);
    rmSync(join(libDir, "bin"), { recursive: true, force: true });
    rmSync(join(libDir, "lib"), { recursive: true, force: true });
  }

  if (!existsSync(libFile)) {
    throw new Error(`expected library not found at ${libFile} after extraction`);
  }
  rmSync(tgzPath, { force: true });
  writeFileSync(versionStamp, PDFIUM_RELEASE);
  const stats = readFileSync(libFile);
  console.log(
    `[fetch-pdfium] ${platform}: installed ${libFile} (${(stats.length / 1024 / 1024).toFixed(1)} MB), version ${PDFIUM_RELEASE}.`
  );
}

main().catch((error) => {
  console.error(`[fetch-pdfium] FAILED: ${error.message}`);
  process.exit(1);
});
