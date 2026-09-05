#!/usr/bin/env node
// 一次性清理：把 E2E 调试期间写进真实用户 AppData 的测试 job 走产品路径移入回收站，
// 并在校验 job.json 标题属于本仓测试语料后删除对应 job 目录（含 DB 行的软删除已由产品完成）。
// 用法：node scripts/e2e/cleanup-tauri-test-jobs.mjs <itemId>[,<itemId>...]

import fs from "node:fs";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import process from "node:process";
import { Builder, By, until } from "selenium-webdriver";

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\/(?=[A-Za-z]:)/, "")), "..", "..");
const ids = (process.argv[2] ?? "").split(",").map((id) => id.trim()).filter(Boolean);
if (!ids.length) {
  console.error("usage: node cleanup-tauri-test-jobs.mjs <itemId>[,<itemId>...]");
  process.exit(2);
}
const dataRoot = path.join(process.env.APPDATA ?? "", "com.ielts.author.studio");
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const driverDir = path.join(process.env.LOCALAPPDATA ?? "", "pdf2test-e2e-drivers");

// 双保险：目录必须存在且 job.json 标题/来源是本仓合成语料，否则跳过目录删除。
const TEST_MARKERS = ["two column", "two-column", "synthetic"];

function looksLikeTestJob(jobDir) {
  try {
    const job = JSON.parse(fs.readFileSync(path.join(jobDir, "job.json"), "utf8"));
    const haystack = `${job.title ?? ""} ${job.originalName ?? ""} ${JSON.stringify(job.source_files?.map((s) => s.original_name ?? "") ?? [])}`.toLowerCase();
    return TEST_MARKERS.some((marker) => haystack.includes(marker));
  } catch {
    return false;
  }
}

const port = 51700 + Math.floor(Math.random() * 800);
const driverProcess = spawn("tauri-driver", ["--port", String(port)], {
  stdio: ["ignore", "pipe", "pipe"],
  env: { ...process.env, PATH: `${driverDir}${path.delimiter}${process.env.PATH ?? ""}` },
  windowsHide: true
});
let stderr = "";
driverProcess.stderr.on("data", (chunk) => { stderr += String(chunk); });

async function waitForHttp(url, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`timeout waiting for ${url}`);
}

let driver;
try {
  await waitForHttp(`http://127.0.0.1:${port}/status`, 20000);
  driver = await new Builder()
    .usingServer(`http://127.0.0.1:${port}`)
    .withCapabilities({ browserName: "wry", "tauri:options": { application: exePath } })
    .build();
  await driver.switchTo().window((await driver.getAllWindowHandles())[0]);
  await driver.wait(until.elementLocated(By.css('[data-testid="library-page"]')), 30000);

  for (const id of ids) {
    const rows = await driver.findElements(By.css(`[data-item-id="${id}"]`));
    if (!rows.length) {
      console.log(`cleanup: row ${id} 不在题库列表（可能已删除）`);
    } else {
      const buttons = await rows[0].findElements(By.css("button.danger"));
      if (buttons.length) {
        await buttons[0].click();
        await driver.sleep(1200);
        console.log(`cleanup: row ${id} 已走产品路径移入回收站`);
      }
    }
    const jobDir = path.join(dataRoot, "jobs", id);
    if (fs.existsSync(jobDir)) {
      if (looksLikeTestJob(jobDir)) {
        fs.rmSync(jobDir, { recursive: true, force: true });
        console.log(`cleanup: job dir ${jobDir} 已删除（标题校验通过）`);
      } else {
        console.log(`cleanup: 跳过 ${jobDir} —— 标题不像本仓测试语料，请人工确认`);
      }
    } else {
      console.log(`cleanup: job dir 不存在：${jobDir}`);
    }
  }
} finally {
  if (driver) { try { await driver.quit(); } catch {} }
  driverProcess.kill();
}
