#!/usr/bin/env node
// 跨仓库全链路真实应用 e2e（作者端 PDF2Test-data 编排；学生端 IELTS-wt-audit 真 Electron）。
//
//   1. 临时 NAS：<系统临时目录>/cross-repo-chain-<stamp>/{nas-papers,nas-submissions}
//   2. 作者端发布（真 Tauri 界面）：预置三篇合成权威稿 → 题库设 P1/P2/P3 → 组卷勾选 → 发布为当前试卷 → 发布目录 = nas-papers
//   3. 学生端（真 Electron，developer/tests/e2e/student_exam_altu_electron_flow.py 的发布态模式，环境变量指定目录）：
//      三名考生在同一终端依次 签到 → 锁卷 → 三篇作答（每篇两题，有对有错）→ 交卷 → 下一位考生；
//      第三名（在途考生）在锁卷之后、交卷之前，由钩子调用本脚本 --phase republish 重新发布同一套卷；
//      作答目录 = nas-submissions。
//   4. 断言学生端产物：snapshots/<terminal>/<attempt>.{sqlite,meta.json,ready}、status/<terminal>/<attempt>.json，
//      meta.paperId = 发布时的 paperId、installId 非 null、锁定的 examId = 当时的三篇、判分与计划一致。
//   5. 作者端数据模块（真 Tauri 界面，同一应用数据目录）：设作答目录 → 从 NAS 导入 → 断言批次卡片人数、
//      学生表各 Part 得分、学生页原始分与错题数、导出 xlsx。
//   6. 在途保护：第三名交卷成功且判分按锁定批次（paperId 仍为旧卷、旧批次文件仍在、指针已指向新批次）。
//
// 证据层级：real-app-e2e（作者端 CDP 自动化通道驱动真实 Tauri/WebView2；学生端 Playwright-over-CDP 驱动真实 Electron；
// 双方通过磁盘上的 NAS 目录交换数据）。自动化参数见 scripts/e2e/lib/tauri-cdp-harness.mjs 的说明。
// 缺少学生仓库、合成权威稿、exe 或过期构建时以 CannotRunError 退出（退出码 3），不伪装成通过。
//
// 用法：
//   node scripts/e2e/cross-repo-full-chain.mjs                                   完整链路
//   STUDENT_REPO=F:\workspace\IELTS-wt-audit  （默认值，学生端仓库根）
//   node scripts/e2e/cross-repo-full-chain.mjs --phase republish --run-dir <d> --result <f>   （在途钩子内部调用）
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { DatabaseSync } from "node:sqlite";
import {
  CDP_CHANNEL_LABEL, CDP_CHANNEL_NOTE, CannotRunError,
  assertBuildFresh, buildFreshReport, gitHead, launchTauriAppCdp, repoRoot, sha256File, sleep, writeReport,
} from "./lib/tauri-cdp-harness.mjs";
import {
  APP_SETTINGS_KEY, PARTS, checkPointerOnDisk, ensureAllCheckedStable, ensureChecked, libraryRows, publishCurrentSelection,
  readPointer, seedReadingJob, setRowPart, waitForRows,
} from "./lib/paper-publish-steps.mjs";

const scriptPath = fileURLToPath(import.meta.url);
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const extraArgs = "--no-sandbox --disable-gpu";
const SYNTHETIC_AUTHORING = path.join(repoRoot, "fixtures", "golden", "synthetic", "ielts", "early-approaches-authoring-v2.json");
const STUDENT_REPO = process.env.STUDENT_REPO || "F:\\workspace\\IELTS-wt-audit";
const STUDENT_FLOW = path.join(STUDENT_REPO, "developer", "tests", "e2e", "student_exam_altu_electron_flow.py");
const TERMINAL_ID = "TERM-ALTU-01";
const BATCH = "BATCH-ALTU-E2E";
const ADMIN_PASSWORD = "exam-admin-new-2026";

// 三名考生的作答计划：每篇两题（slot 顺序 = 页面出现顺序，正确答案取自合成权威稿的 answerKey）。
// 第三名为在途考生：锁卷后、交卷前触发作者端重新发布。
const CANDIDATES = [
  { candidateId: "C-001", candidateName: "张三", answers: { P1: ["B", "A"], P2: ["B", "D"], P3: ["C", "D"] } },
  { candidateId: "C-002", candidateName: "李四", answers: { P1: ["B", "D"], P2: ["A", "C"], P3: ["B", "A"] } },
  { candidateId: "C-003", candidateName: "王五", answers: { P1: ["B", "D"], P2: ["B", "A"], P3: ["A", "D"] }, inFlight: true },
];

function recordAssertion(report, id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail: String(detail) });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

function argValue(name) {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : null;
}

function runCapture(command, args, options = {}) {
  return new Promise((resolve) => {
    const child = spawn(command, args, { ...options, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (c) => { stdout += String(c); });
    child.stderr.on("data", (c) => { stderr += String(c); });
    const timer = setTimeout(() => child.kill(), options.timeoutMs ?? 40 * 60 * 1000);
    child.on("error", (err) => { clearTimeout(timer); resolve({ code: -1, stdout, stderr: `${stderr}\n${err}` }); });
    child.on("close", (code) => { clearTimeout(timer); resolve({ code, stdout, stderr }); });
  });
}

function readState(runDir) {
  return JSON.parse(fs.readFileSync(path.join(runDir, "chain-state.json"), "utf8"));
}

function writeState(runDir, state) {
  fs.writeFileSync(path.join(runDir, "chain-state.json"), JSON.stringify(state, null, 2));
}

/** 作者端发布：真 Tauri 界面。seed=true 时先把三篇预置进应用数据目录（启动迁移种入题库）。 */
async function authorPublish({ runDir, nasRoot, state, seed, label }) {
  const template = JSON.parse(fs.readFileSync(SYNTHETIC_AUTHORING, "utf8"));
  const dataDir = path.join(runDir, "appdata", "data");
  if (seed) {
    state.seeded = [1, 2, 3].map((index) => seedReadingJob(dataDir, index, template, state.stamp));
  }
  const ids = state.seeded.map((job) => job.jobId);
  const out = { label, ok: false, assertions: [] };
  const session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs, quiet: true });
  try {
    const rows = await waitForRows(session, ids, 90000);
    const migrated = ids.every((id) => rows.some((row) => row.id === id));
    out.assertions.push(["migrated", migrated, `题库预置条目=${ids.filter((id) => rows.some((r) => r.id === id)).length}/3`]);
    if (!migrated) throw new Error("预置的三篇没有被启动迁移种进题库");

    await session.evaluate(`(() => {
      const raw = window.localStorage.getItem(${JSON.stringify(APP_SETTINGS_KEY)});
      const settings = raw ? JSON.parse(raw) : {};
      settings.nasDestination = ${JSON.stringify(nasRoot)};
      window.localStorage.setItem(${JSON.stringify(APP_SETTINGS_KEY)}, JSON.stringify(settings));
      window.location.hash = "#/settings"; return true;
    })()`);
    await sleep(400);
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="current-paper-card"]')`, { timeoutMs: 40000, label: `${label}-paper-card` });

    if (seed) {
      for (const [index, id] of ids.entries()) {
        await session.waitFor(`!!document.querySelector('[data-item-id="${id}"] [data-testid="library-row-part-select"]')`, { timeoutMs: 20000, label: `part-select-${index}` });
        await setRowPart(session, id, PARTS[index]);
        const deadline = Date.now() + 15000;
        while (Date.now() < deadline) {
          if ((await libraryRows(session)).find((r) => r.id === id)?.partLabel === PARTS[index]) break;
          await sleep(500);
        }
      }
    }
    const partRows = await libraryRows(session);
    const partOf = ids.map((id) => partRows.find((r) => r.id === id)?.partLabel ?? null);
    out.assertions.push(["parts", PARTS.every((p, i) => partOf[i] === p), `Part=${JSON.stringify(partOf)}`]);

    await session.waitFor(`!!document.querySelector('[data-testid="paper-table"]')`, { timeoutMs: 15000, label: `${label}-paper-table` });
    const stab = await ensureAllCheckedStable(session, ids);
    out.assertions.push(["checked", stab.stable, `勾选稳定=${stab.stable} 补点次数=${stab.rechecks} 状态=${JSON.stringify(stab.states)}`]);
    out.assertions.push(["checked-no-recheck", stab.rechecks === 0, `补点次数必须为 0（勾选重置根因已修）：rechecks=${stab.rechecks}`]);
    out.rechecks = stab.rechecks;
    const message = await publishCurrentSelection(session, label);
    out.message = message;
    out.assertions.push(["publish-message", String(message).includes("当前试卷已发布"), `界面提示=${JSON.stringify(message)}`]);
    await session.screenshot(`chain-${label}-publish-result`);

    const disk = checkPointerOnDisk(nasRoot);
    out.assertions.push(["pointer-on-disk", disk.ok, disk.ok ? "指针 entry = 清单，脚本字节一致，资源位于 releases/<batch>" : disk.problems.join("；")]);
    out.pointer = {
      paperId: disk.pointer.paperId,
      batchId: disk.pointer.batchId,
      publishedAt: disk.pointer.publishedAt,
      reading: disk.pointer.reading.map((item) => ({
        part: item.part, examId: item.examId, title: item.title, script: item.script,
        scriptSha256: item.scriptSha256, runtimeSha256: item.runtimeSha256, resourcesBase: item.entry?.resourcesBase,
      })),
    };
    out.ok = out.assertions.every(([, ok]) => ok);
  } catch (error) {
    out.error = String(error?.message ?? error);
    await session.screenshot(`chain-${label}-fail`).catch(() => {});
  } finally {
    await session.close?.().catch?.(() => {});
  }
  return out;
}

/** 在途钩子：同一套卷再发布一次（不重新预置；应用数据目录沿用，题库行与 Part 已在库中）。 */
async function republishPhase({ runDir, nasRoot, resultFile }) {
  const state = readState(runDir);
  const ids = state.seeded.map((job) => job.jobId);
  const before = readPointer(nasRoot);
  const session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs, quiet: true });
  const out = { label: "republish-in-flight", ok: false, before: { paperId: before.paperId, batchId: before.batchId }, assertions: [] };
  try {
    await waitForRows(session, ids, 90000);
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="paper-table"]')`, { timeoutMs: 40000, label: "republish-paper-table" });
    const rows = await libraryRows(session);
    const partsOk = PARTS.every((p, i) => rows.find((r) => r.id === ids[i])?.partLabel === p);
    out.assertions.push(["parts-persisted", partsOk, `Part 仍为 P1/P2/P3=${partsOk}`]);
    const stab = await ensureAllCheckedStable(session, ids);
    out.assertions.push(["checked", stab.stable, `勾选稳定=${stab.stable} 补点次数=${stab.rechecks} 状态=${JSON.stringify(stab.states)}`]);
    out.assertions.push(["checked-no-recheck", stab.rechecks === 0, `补点次数必须为 0（勾选重置根因已修）：rechecks=${stab.rechecks}`]);
    out.message = await publishCurrentSelection(session, "republish-in-flight");
    out.assertions.push(["publish-message", String(out.message).includes("当前试卷已发布"), `界面提示=${JSON.stringify(out.message)}`]);
    const disk = checkPointerOnDisk(nasRoot);
    out.assertions.push(["pointer-on-disk", disk.ok, disk.ok ? "指针与磁盘一致" : disk.problems.join("；")]);
    out.after = { paperId: disk.pointer.paperId, batchId: disk.pointer.batchId };
    out.assertions.push(["paperId-changed", disk.pointer.paperId !== before.paperId, `旧=${before.paperId} 新=${disk.pointer.paperId}`]);
    out.assertions.push(["batch-changed", disk.pointer.batchId !== before.batchId, `旧批次=${before.batchId} 新批次=${disk.pointer.batchId}`]);
    out.ok = out.assertions.every(([, ok]) => ok);
  } catch (error) {
    out.error = String(error?.message ?? error);
    await session.screenshot("chain-republish-fail").catch(() => {});
  } finally {
    await session.close?.().catch?.(() => {});
  }
  fs.mkdirSync(path.dirname(resultFile), { recursive: true });
  fs.writeFileSync(resultFile, JSON.stringify(out, null, 2));
  console.log(`[republish] ok=${out.ok} ${JSON.stringify(out.after ?? out.error)}`);
  process.exitCode = out.ok ? 0 : 1;
}

/** 学生端快照与信标的磁盘断言（不依赖 UI）。 */
function assertStudentArtifacts(report, state, submissionsRoot, paperA) {
  const snapDir = path.join(submissionsRoot, "snapshots", TERMINAL_ID);
  const statusDir = path.join(submissionsRoot, "status", TERMINAL_ID);
  const metas = fs.existsSync(snapDir) ? fs.readdirSync(snapDir).filter((f) => f.endsWith(".meta.json")) : [];
  recordAssertion(report, "S-1 snapshot-count", metas.length === CANDIDATES.length, `快照 meta 数=${metas.length}（期望 ${CANDIDATES.length}）`);

  const attemptsByCandidate = {};
  for (const meta of metas) {
    const m = JSON.parse(fs.readFileSync(path.join(snapDir, meta), "utf8"));
    const base = meta.replace(/\.meta\.json$/, "");
    const files = ["sqlite", "ready"].map((ext) => fs.existsSync(path.join(snapDir, `${base}.${ext}`)));
    const sqlite = path.join(snapDir, `${base}.sqlite`);
    const shaOk = files[0] && sha256File(sqlite) === m.sha256;
    recordAssertion(report, `S-2 ${base}-files-and-sha`, files.every(Boolean) && shaOk,
      `ready=${files[1]} sqlite=${files[0]} sha256=${shaOk}`);
    recordAssertion(report, `S-3 ${base}-paperId`, m.paperId === paperA.paperId,
      `meta.paperId=${m.paperId} 发布时=${paperA.paperId}`);
    recordAssertion(report, `S-4 ${base}-installId`, typeof m.installId === "string" && m.installId.length > 0,
      `installId=${m.installId}`);
    recordAssertion(report, `S-5 ${base}-schema`, m.schemaVersion === "SubmissionSnapshotV1" && m.dbSchemaVersion === 1 && m.scorerVersion === "reading-scorer/1",
      `schema=${m.schemaVersion} db=${m.dbSchemaVersion} scorer=${m.scorerVersion}`);
    const statusPath = path.join(statusDir, `${m.attemptId}.json`);
    const status = fs.existsSync(statusPath) ? JSON.parse(fs.readFileSync(statusPath, "utf8")) : null;
    recordAssertion(report, `S-6 ${base}-status-beacon`, status?.status === "submitted" && status?.paperId === paperA.paperId,
      `status=${status?.status} paperId=${status?.paperId}`);
    attemptsByCandidate[m.candidateId] = { meta: m, sqlite, attemptId: m.attemptId };
  }
  return attemptsByCandidate;
}

/** 从快照 sqlite 读取锁定的 examId 与判分（只读）。 */
function readLockedScores(sqlitePath, attemptId) {
  const db = new DatabaseSync(sqlitePath);
  try {
    const suite = db.prepare("SELECT suite_attempt_id AS id, aggregate_json AS agg FROM reading_suite_attempts WHERE attempt_id = ?").get(attemptId);
    if (!suite) return null;
    const passages = db.prepare(
      "SELECT passage_index AS idx, exam_id AS examId, score_info_json AS score FROM reading_passages WHERE suite_attempt_id = ? ORDER BY passage_index"
    ).all(suite.id).map((row) => ({ idx: row.idx, examId: row.examId, correct: JSON.parse(row.score).correct, total: JSON.parse(row.score).total }));
    const answers = db.prepare(
      "SELECT passage_index AS idx, question_id AS q, is_correct AS ok, user_answer_json AS user FROM reading_answers WHERE suite_attempt_id = ? ORDER BY passage_index, question_id"
    ).all(suite.id);
    return { aggregate: JSON.parse(suite.agg ?? "{}"), passages, answers };
  } finally {
    db.close();
  }
}

/** unordered_set 判分：所选字母与答案集合的交集大小（合成权威稿 selectionCardinality=2，answerAssignment=unordered_set）。 */
function expectedCorrect(answers, key) {
  return answers.filter((letter) => key.includes(letter)).length;
}

function fixtureKeyLabels() {
  const authoring = JSON.parse(fs.readFileSync(SYNTHETIC_AUTHORING, "utf8"));
  const slots = Object.keys(authoring.answerKey).sort((a, b) => Number(a.replace(/\D/g, "")) - Number(b.replace(/\D/g, "")));
  return slots.map((slot) => authoring.answerKey[slot].labels[0]);
}

async function runStudent({ runDir, nasRoot, submissionsRoot, paperA, key, hookCommand }) {
  const plan = {
    candidates: CANDIDATES.map((candidate) => ({
      candidateId: candidate.candidateId,
      candidateName: candidate.candidateName,
      parts: PARTS.map((part, index) => ({
        part,
        title: paperA.reading[index].title,
        answers: candidate.answers[part],
      })),
    })),
    hook: { candidateId: CANDIDATES.find((c) => c.inFlight).candidateId, command: hookCommand, logPath: path.join(runDir, "hook-republish.log") },
  };
  const planFile = path.join(runDir, "student-plan.json");
  fs.writeFileSync(planFile, JSON.stringify(plan, null, 2));
  // 可选测试补丁（默认关闭）。学生端「选择题库目录」已按 current-paper.json 校验 V1.1 发布目录，无需此标记。
  // 仅当显式设置 CHAIN_MARKER_WORKAROUND=1 时，在发布目录根放一个不被指针引用的标记脚本，用于复现旧版目录校验缺陷。
  // 启用时 workarounds 列表会记录它，报告中据此可见；默认运行 workarounds 应为空。
  const workarounds = [];
  if (process.env.CHAIN_MARKER_WORKAROUND === "1") {
    fs.writeFileSync(path.join(nasRoot, "zz-e2e-validation-marker.js"), "// e2e-only marker (test workaround for student directory validator, not referenced by current-paper.json). __READING_EXAM_DATA__\n");
    workarounds.push("root-exam-js-marker (zz-e2e-validation-marker.js)");
  }
  const env = {
    ...process.env,
    PYTHONIOENCODING: "utf-8",
    PYTHONUTF8: "1",
    STUDENT_E2E_PAPERS_ROOT: nasRoot,
    STUDENT_E2E_SUBMISSIONS_ROOT: submissionsRoot,
    STUDENT_E2E_PLAN_FILE: planFile,
  };
  const started = Date.now();
  const result = await runCapture("python", ["-X", "utf8", STUDENT_FLOW], { cwd: STUDENT_REPO, env });
  fs.writeFileSync(path.join(runDir, "student-run.stdout.log"), result.stdout);
  fs.writeFileSync(path.join(runDir, "student-run.stderr.log"), result.stderr);
  const lastLine = result.stdout.trim().split(/\r?\n/).filter(Boolean).pop() ?? "";
  let parsed = null;
  try { parsed = JSON.parse(lastLine); } catch { parsed = null; }
  return { workarounds, code: result.code, seconds: Math.round((Date.now() - started) / 1000), parsed, plan, key };
}

async function dataPhase({ runDir, submissionsRoot, paperA, report, expectations }) {
  // 学生端快照的 batchId = 选卷时 manifest._meta.batchId（作者端发布批次），不是 exam-runtime 里配置的会话号。
  const batch = paperA.batchId;
  const exportPath = path.join(runDir, "exports", "cross-repo-chain-exam.xlsx");
  fs.mkdirSync(path.dirname(exportPath), { recursive: true });
  const session = await launchTauriAppCdp({
    exePath, runDir, extraBrowserArgs: extraArgs, quiet: true,
    appEnv: { PDF2TEST_AUTOMATION_EXPORT_XLSX: exportPath },
  });
  const shot = async (name) => { await session.screenshot(name).catch(() => null); };
  try {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 60000, label: "library" });
    await session.evaluate(`(() => {
      const raw = window.localStorage.getItem(${JSON.stringify(APP_SETTINGS_KEY)});
      const settings = raw ? JSON.parse(raw) : {};
      settings.submissionsRoot = ${JSON.stringify(submissionsRoot)};
      window.localStorage.setItem(${JSON.stringify(APP_SETTINGS_KEY)}, JSON.stringify(settings));
      window.location.hash = "#/data"; return true;
    })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="data-import"]')`, { timeoutMs: 20000, label: "data-import-button" });
    await shot("chain-data-01-before-import");
    await session.clickSelector('[data-testid="data-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="data-import-result"]')`, { timeoutMs: 90000, label: "import-result" });
    const importText = await session.evaluate(`document.querySelector('[data-testid="data-import-result"]').textContent`);
    report.importResultText = importText;
    recordAssertion(report, "D-1 import-no-failure", /新导入 \d+/.test(importText) && /失败 0/.test(importText), `导入结果=${JSON.stringify(importText)}`);
    await session.waitFor(`!!document.querySelector('[data-testid="data-batch-card"]')`, { timeoutMs: 20000, label: "batch-card" });
    const cards = await session.evaluate(`[...document.querySelectorAll('[data-testid="data-batch-card"]')].map((c) => c.textContent)`);
    report.batchCards = cards;
    const card = cards.find((text) => text.includes(batch.slice(0, 8)) && text.includes(paperA.paperId.slice(0, 8)));
    const n = expectations.candidates.length;
    recordAssertion(report, "D-2 batch-card-counts", Boolean(card) && card.includes(`已导入 ${n} 人`) && card.includes(`有效计分 N=${n}`) && card.includes(`已交卷 ${n}`),
      `批次卡片=${JSON.stringify(card ?? cards.join(" | ")).slice(0, 220)}`);
    await shot("chain-data-02-batches");

    await session.evaluate(`(() => { window.location.hash = ${JSON.stringify("#/data/exam?batch=" + encodeURIComponent(batch) + "&paper=" + encodeURIComponent(paperA.paperId))}; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="data-students"]')`, { timeoutMs: 30000, label: "data-students" });
    const studentRows = await session.evaluate(`[...document.querySelectorAll('[data-testid="data-students"] tbody tr')].map((tr) => [...tr.querySelectorAll('td')].map((td) => td.textContent.trim()))`);
    report.studentTable = studentRows;
    for (const cand of expectations.candidates) {
      const row = studentRows.find((cells) => cells.includes(cand.candidateName));
      const cellsOk = row && cand.partCorrect.every((value, i) => row.includes(String(value)));
      recordAssertion(report, `D-3 ${cand.candidateId}-part-scores`, Boolean(cellsOk),
        `学生表行=${JSON.stringify(row ?? null)} 期望各 Part 得分点=${JSON.stringify(cand.partCorrect)} 原始分=${cand.raw}`);
    }
    await shot("chain-data-03-exam");

    const examText = await session.evaluate(`document.querySelector('[data-testid="data-exam"]')?.textContent || ""`);
    recordAssertion(report, "D-4 exam-page-n", examText.includes(`N=${n}`), `考试页含 N=${n}`);

    // 学生详情：第三名（在途）
    const inFlight = expectations.candidates.find((c) => c.inFlight);
    await session.evaluate(`(() => {
      const btn = [...document.querySelectorAll('[data-testid="data-students"] button')].find((b) => b.textContent.trim() === ${JSON.stringify(inFlight.candidateName)});
      if (!btn) return false; btn.click(); return true;
    })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="data-student"]')`, { timeoutMs: 20000, label: "student-page" });
    const studentText = await session.evaluate(`document.querySelector('[data-testid="data-student"]').textContent`);
    recordAssertion(report, "D-5 student-raw-score", studentText.includes(`${inFlight.raw} / ${inFlight.total}`),
      `在途考生学生页含 ${inFlight.raw} / ${inFlight.total}`);
    const wrongButtons = await session.evaluate(`[...document.querySelectorAll('[data-testid="data-student"] button')].filter((b) => b.textContent.trim() === '看此题').length`);
    recordAssertion(report, "D-6 student-wrong-list", wrongButtons === inFlight.wrong, `错题条目=${wrongButtons}（期望 ${inFlight.wrong}）`);
    await shot("chain-data-04-student");

    await session.evaluate(`(() => { window.location.hash = ${JSON.stringify("#/data/exam?batch=" + encodeURIComponent(batch) + "&paper=" + encodeURIComponent(paperA.paperId))}; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="data-export"]')`, { timeoutMs: 20000, label: "export-button" });
    await session.clickSelector('[data-testid="data-export"]');
    await session.waitFor(`!!document.querySelector('[data-testid="data-export-notice"]')`, { timeoutMs: 60000, label: "export-notice" });
    const exists = fs.existsSync(exportPath);
    const magic = exists ? fs.readFileSync(exportPath).subarray(0, 2).toString("latin1") : "";
    recordAssertion(report, "D-7 xlsx-written", exists && magic === "PK", `导出=${exportPath} exists=${exists} magic=${JSON.stringify(magic)}`);

    const settingsOk = await session.evaluate(`(async () => {
      window.location.hash = "#/settings";
      for (let i = 0; i < 40; i++) {
        const el = document.querySelector('[data-testid="settings-submissions"]');
        if (el) return el.textContent.includes(${JSON.stringify(submissionsRoot)});
        await new Promise((r) => setTimeout(r, 250));
      }
      return false;
    })()`);
    recordAssertion(report, "D-8 settings-shows-submissions-root", settingsOk, `设置页含作答数据目录=${settingsOk}`);
    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type)).length;
    report.dataExportPath = exportPath;
  } catch (error) {
    recordAssertion(report, "D-x data-phase-exception", false, String(error?.message ?? error));
    await shot("chain-data-fail");
  } finally {
    await session.close?.().catch?.(() => {});
  }
}

async function main() {
  const phase = argValue("--phase");
  if (phase === "republish") {
    const runDir = argValue("--run-dir");
    const resultFile = argValue("--result");
    const nasRoot = readState(runDir).nasRoot;
    await republishPhase({ runDir, nasRoot, resultFile });
    return;
  }

  if (!fs.existsSync(SYNTHETIC_AUTHORING)) throw new CannotRunError(`合成权威稿缺失：${SYNTHETIC_AUTHORING}`);
  if (!fs.existsSync(exePath)) throw new CannotRunError(`exe 不存在（先运行 node scripts/e2e/build-app.mjs）：${exePath}`);
  if (!fs.existsSync(STUDENT_FLOW)) throw new CannotRunError(`学生端仓库缺失（STUDENT_REPO=${STUDENT_REPO}）：${STUDENT_FLOW}`);
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });

  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `cross-repo-chain-${stamp}`);
  const tmpRoot = path.join(os.tmpdir(), `cross-repo-chain-${stamp}`);
  const nasRoot = path.join(tmpRoot, "nas-papers");
  const submissionsRoot = path.join(tmpRoot, "nas-submissions");
  fs.mkdirSync(nasRoot, { recursive: true });
  fs.mkdirSync(submissionsRoot, { recursive: true });
  fs.mkdirSync(runDir, { recursive: true });
  const state = { stamp, runDir, nasRoot, submissionsRoot, seeded: [] };
  writeState(runDir, state);

  const report = {
    task: "cross-repo-full-chain",
    scope: "作者端组卷发布 → 学生端真 Electron 三人考试（含在途重新发布）→ 作者端数据模块导入与统计",
    channel: CDP_CHANNEL_LABEL, channelNote: CDP_CHANNEL_NOTE,
    evidenceLevel: "real-app-e2e: author Tauri (CDP automation channel) + student Electron (Playwright over CDP); exchange via NAS directories on disk",
    identity: { exePath, commit: gitHead(repoRoot), studentRepo: STUDENT_REPO, studentCommit: gitHead(STUDENT_REPO), buildFresh: buildFreshReport(fresh) },
    paths: { runDir, nasRoot, submissionsRoot },
    assertions: [], steps: {},
  };
  const finish = (verdict) => {
    report.verdict = verdict;
    report.failedAssertions = report.assertions.filter((a) => !a.ok).map((a) => a.id);
    const file = writeReport(runDir, report);
    console.log(`[cross-repo] verdict=${verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
    console.log(`[cross-repo] report=${file}`);
    process.exitCode = verdict === "passed" ? 0 : 1;
  };

  // 步骤 2：作者端发布
  const pub = await authorPublish({ runDir, nasRoot, state, seed: true, label: "publish-A" });
  report.steps.authorPublishA = pub;
  for (const [id, ok, detail] of pub.assertions) recordAssertion(report, `P-${id}`, ok, detail);
  if (!pub.ok) { recordAssertion(report, "P-x publish-aborted", false, pub.error ?? "publish failed"); return finish("failed"); }
  writeState(runDir, state);
  const paperA = { paperId: pub.pointer.paperId, batchId: pub.pointer.batchId, reading: pub.pointer.reading };
  state.paperA = paperA;
  writeState(runDir, state);

  // 步骤 3：学生端三人考试（在途钩子 = 作者端重新发布）
  const hookCommand = `"${process.execPath}" "${scriptPath}" --phase republish --run-dir "${runDir}" --result "${path.join(runDir, "republish-result.json")}"`;
  const key = fixtureKeyLabels();
  const student = await runStudent({ runDir, nasRoot, submissionsRoot, paperA, key, hookCommand });
  report.workarounds = student.workarounds;
  report.steps.student = { code: student.code, seconds: student.seconds, status: student.parsed?.status ?? null, detail: student.parsed?.detail ?? null };
  recordAssertion(report, "S-0 student-flow-exit", student.code === 0 && student.parsed?.status === "pass",
    `退出码=${student.code} status=${student.parsed?.status ?? "n/a"} 耗时=${student.seconds}s${student.parsed?.status === "pass" ? "" : ` detail=${student.parsed?.detail ?? "见 student-run.stdout.log"}`}`);

  // 在途钩子结果
  const republishFile = path.join(runDir, "republish-result.json");
  const republish = fs.existsSync(republishFile) ? JSON.parse(fs.readFileSync(republishFile, "utf8")) : null;
  report.steps.republishInFlight = republish;
  recordAssertion(report, "R-1 in-flight-republish", Boolean(republish?.ok), republish ? JSON.stringify(republish.assertions.map((a) => [a[0], a[1]])) : "钩子未执行（缺 republish-result.json）");
  recordAssertion(report, "R-2 in-flight-republish-paperId", Boolean(republish?.after && republish.after.paperId !== paperA.paperId),
    `旧 paperId=${paperA.paperId} 新=${republish?.after?.paperId ?? "n/a"}`);

  // 步骤 4：学生端产物（磁盘）
  if (fs.existsSync(path.join(submissionsRoot, "snapshots"))) {
    const attempts = assertStudentArtifacts(report, state, submissionsRoot, paperA);
    const expectations = { candidates: [] };
    for (const cand of CANDIDATES) {
      const found = attempts[cand.candidateId];
      const partCorrect = PARTS.map((part) => expectedCorrect(cand.answers[part], key));
      const raw = partCorrect.reduce((s, v) => s + v, 0);
      const total = PARTS.length * key.length;
      const wrong = total - raw;
      const exp = { ...cand, partCorrect, raw, total, wrong };
      expectations.candidates.push(exp);
      if (!found) { recordAssertion(report, `S-7 ${cand.candidateId}-snapshot-present`, false, "无快照"); continue; }
      const scores = readLockedScores(found.sqlite, found.attemptId);
      const lockedIds = scores?.passages.map((p) => p.examId) ?? [];
      recordAssertion(report, `S-7 ${cand.candidateId}-locked-examIds`,
        JSON.stringify(lockedIds) === JSON.stringify(paperA.reading.map((r) => r.examId)),
        `锁定 examId=${JSON.stringify(lockedIds)} 发布时=${JSON.stringify(paperA.reading.map((r) => r.examId))}`);
      const gotCorrect = (scores?.passages ?? []).map((p) => p.correct);
      recordAssertion(report, `S-8 ${cand.candidateId}-scores-vs-plan`,
        JSON.stringify(gotCorrect) === JSON.stringify(partCorrect),
        `学生端判分=${JSON.stringify(gotCorrect)} 计划期望=${JSON.stringify(partCorrect)}（每篇 ${key.length} 题）`);
      if (cand.inFlight) {
        const passageOk = scores?.passages.every((p) => p.examId && paperA.reading.some((r) => r.examId === p.examId));
        recordAssertion(report, "R-3 in-flight-submitted-under-locked-batch",
          found.meta.paperId === paperA.paperId && Boolean(passageOk) && gotCorrect.length === 3,
          `交卷快照 paperId=${found.meta.paperId}（旧卷）；锁定 examId 均属旧卷=${passageOk}`);
        const oldScript = path.join(nasRoot, paperA.reading[0].script.replace(/^\.\//, ""));
        const oldBatchAlive = fs.existsSync(oldScript) && sha256File(oldScript) === paperA.reading[0].scriptSha256;
        recordAssertion(report, "R-4 old-batch-script-kept-and-intact", oldBatchAlive,
          `旧批次脚本 ${paperA.reading[0].script} 存在且 sha256 与锁定值一致=${oldBatchAlive}`);
        const pointerNow = readPointer(nasRoot);
        recordAssertion(report, "R-5 pointer-moved-to-new-batch", pointerNow.paperId !== paperA.paperId,
          `当前指针 paperId=${pointerNow.paperId} batch=${pointerNow.batchId}`);
      }
    }
    state.expectations = expectations;
    writeState(runDir, state);

    // 步骤 5：作者端数据模块（同一应用数据目录）
    await dataPhase({ runDir, submissionsRoot, paperA, report, expectations });
  } else {
    recordAssertion(report, "S-x snapshots-missing", false, `未找到 ${path.join(submissionsRoot, "snapshots")}`);
  }

  const verdict = report.assertions.every((a) => a.ok) ? "passed" : "failed";
  finish(verdict);
  console.log(`[cross-repo] temp NAS kept at ${tmpRoot}`);
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  console.error(cannotRun ? `[cross-repo] CANNOT RUN: ${error.message}` : error);
  process.exitCode = cannotRun ? 3 : 1;
});
