#!/usr/bin/env node
/**
 * Removes generated build outputs, temp directories, and local logs that can
 * be recreated from source. Use --deep to also remove node_modules.
 *
 * Usage:
 *   node scripts/clean-generated.mjs
 *   node scripts/clean-generated.mjs --deep
 *   node scripts/clean-generated.mjs --dry-run
 */
import { existsSync, lstatSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";

const DEEP = process.argv.includes("--deep");
const DRY_RUN = process.argv.includes("--dry-run");

const targets = [
  "dist",
  "output",
  "tmp",
  ".playwright-mcp",
  "output-tauri-dev.log",
  "output-tauri-dev.err.log",
  join("src-tauri", "target"),
];

if (DEEP) {
  targets.push("node_modules");
}

function sizeOf(path) {
  const stat = lstatSync(path);
  if (!stat.isDirectory()) {
    return stat.size;
  }

  let total = 0;
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    total += sizeOf(join(path, entry.name));
  }
  return total;
}

function formatBytes(bytes) {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(2)} MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(2)} KB`;
  return `${bytes} B`;
}

let reclaimed = 0;
let removed = 0;

for (const target of targets) {
  if (!existsSync(target)) {
    console.log(`[clean] skip ${target} (not found)`);
    continue;
  }

  const bytes = sizeOf(target);
  reclaimed += bytes;
  removed += 1;
  console.log(`[clean] ${DRY_RUN ? "would remove" : "remove"} ${target} (${formatBytes(bytes)})`);

  if (!DRY_RUN) {
    rmSync(target, { recursive: true, force: true });
  }
}

console.log(
  `[clean] ${DRY_RUN ? "would reclaim" : "reclaimed"} ${formatBytes(reclaimed)} across ${removed} path(s).`,
);
