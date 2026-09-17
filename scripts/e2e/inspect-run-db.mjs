#!/usr/bin/env node
/**
 * 只读诊断：直接看某个 E2E 运行目录里的 SQLite，确认「批次/决策项/链状态」到底有没有落盘。
 * 不修改任何数据（readOnly 打开）。
 *
 * 用法：node scripts/e2e/inspect-run-db.mjs <runDir>
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { DatabaseSync } from "node:sqlite";

const runDir = process.argv[2];
if (!runDir) {
  console.error("用法：node scripts/e2e/inspect-run-db.mjs <runDir>");
  process.exit(2);
}
const dbPath = path.join(runDir, "appdata", "data", "authoring_hub.db");
if (!fs.existsSync(dbPath)) {
  console.error(`数据库不存在：${dbPath}`);
  process.exit(2);
}

const db = new DatabaseSync(dbPath, { readOnly: true });
const tables = db
  .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
  .all()
  .map((r) => r.name);
console.log("tables:", tables.join(", "));

const interesting = tables.filter((t) => /batch|decision|recognition|job|authoring|canonical/i.test(t));
for (const t of interesting) {
  let count = null;
  try {
    count = db.prepare(`SELECT COUNT(*) AS c FROM "${t}"`).get().c;
  } catch (error) {
    console.log(`\n== ${t}: count failed: ${error.message}`);
    continue;
  }
  console.log(`\n== ${t} (${count} rows)`);
  if (count === 0) continue;
  try {
    const rows = db.prepare(`SELECT * FROM "${t}" ORDER BY rowid DESC LIMIT 3`).all();
    for (const row of rows) {
      const trimmed = {};
      for (const [k, v] of Object.entries(row)) {
        const s = typeof v === "string" ? v : JSON.stringify(v);
        trimmed[k] = s && s.length > 300 ? `${s.slice(0, 300)}…` : s;
      }
      console.log(JSON.stringify(trimmed));
    }
  } catch (error) {
    console.log(`  select failed: ${error.message}`);
  }
}
db.close();
