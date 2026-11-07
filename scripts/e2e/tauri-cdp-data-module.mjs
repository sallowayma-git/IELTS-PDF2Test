#!/usr/bin/env node
// 「数据」模块真实应用 e2e（教师端第一阶段）：
//   生成学生端形态的作答快照目录（两个终端库 + 一个坏快照 + 状态信标）→ 设置里记住作答数据目录
//   → 数据页「从 NAS 导入」→ 批次卡片 N=4 → 考试页（分布 / 矩阵 / 学生表）→ 学生详情（用时重叠提示）
//   → 题目详情（答案分布）→ 导出 Excel（保存路径由自动化钩子指定）→ 设置页显示作答数据目录。
//
// 证据层级：real-app-e2e（CDP 自动化通道驱动真实 Tauri 界面，读取磁盘上的导出文件）。
// 快照是本脚本用 node:sqlite 按学生端 ensureSchema() 的建表语句生成的夹具；
// 它不依赖学生仓库代码，也不替代与真实学生端终端产生的快照的对接验证（那一项在报告里单列为未覆盖）。
// 依赖已构建的 debug exe（npm run build:app）；exe 缺失或过期时以 CannotRunError 退出（退出码 3）。
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { DatabaseSync } from "node:sqlite";
import {
  CDP_CHANNEL_LABEL, CDP_CHANNEL_NOTE, CannotRunError,
  assertBuildFresh, buildFreshReport, gitHead, launchTauriAppCdp, repoRoot, sleep, writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `data-module-${stamp}`);
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const extraArgs = "--no-sandbox --disable-gpu";
const APP_SETTINGS_KEY = "ielts-author-studio.app-settings.v1";
const BATCH = "B-2026-11";
const PAPER = "paper-A";

// 学生端 MachineSubmissionStore.ensureSchema() 的建表语句（逐字抄录）。
const STUDENT_SCHEMA_SQL = `
  CREATE TABLE IF NOT EXISTS exam_attempts (
    attempt_id TEXT PRIMARY KEY, batch_id TEXT NOT NULL, terminal_id TEXT NOT NULL,
    candidate_id TEXT NOT NULL, candidate_name TEXT NOT NULL, checkin_at TEXT NOT NULL,
    started_at TEXT, submitted_at TEXT,
    status TEXT NOT NULL CHECK(status IN ('checked_in', 'in_progress', 'submitted', 'failed')),
    app_version TEXT, runtime_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
  );
  CREATE TABLE IF NOT EXISTS reading_suite_attempts (
    suite_attempt_id TEXT PRIMARY KEY, attempt_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'completed', 'submitted')),
    started_at TEXT, completed_at TEXT, submitted_at TEXT,
    total_passages INTEGER NOT NULL DEFAULT 3, aggregate_json TEXT,
    FOREIGN KEY(attempt_id) REFERENCES exam_attempts(attempt_id) ON DELETE CASCADE
  );
  CREATE TABLE IF NOT EXISTS reading_passages (
    id INTEGER PRIMARY KEY AUTOINCREMENT, suite_attempt_id TEXT NOT NULL, passage_index INTEGER NOT NULL,
    asset_id TEXT NOT NULL, exam_id TEXT NOT NULL, title TEXT, category TEXT, source_file TEXT, source_hash TEXT,
    started_at TEXT, ended_at TEXT, duration_sec INTEGER, score_info_json TEXT NOT NULL, raw_submission_json TEXT NOT NULL,
    UNIQUE(suite_attempt_id, passage_index),
    FOREIGN KEY(suite_attempt_id) REFERENCES reading_suite_attempts(suite_attempt_id) ON DELETE CASCADE
  );
  CREATE TABLE IF NOT EXISTS reading_answers (
    id INTEGER PRIMARY KEY AUTOINCREMENT, suite_attempt_id TEXT NOT NULL, passage_index INTEGER NOT NULL,
    question_id TEXT NOT NULL, display_label TEXT, question_kind TEXT, control TEXT, source TEXT, match_mode TEXT,
    user_answer_json TEXT NOT NULL, correct_answer_json TEXT NOT NULL,
    normalized_user_answer_json TEXT, normalized_correct_answer_json TEXT,
    is_correct INTEGER, weight REAL NOT NULL DEFAULT 1, first_answered_at TEXT, last_answered_at TEXT,
    change_count INTEGER NOT NULL DEFAULT 0, visit_count INTEGER NOT NULL DEFAULT 0,
    elapsed_ms INTEGER NOT NULL DEFAULT 0, duration_ms INTEGER NOT NULL DEFAULT 0,
    UNIQUE(suite_attempt_id, passage_index, question_id)
  );
  CREATE TABLE IF NOT EXISTS writing_submissions (
    writing_submission_id TEXT PRIMARY KEY, attempt_id TEXT NOT NULL,
    task_type TEXT NOT NULL CHECK(task_type IN ('task1', 'task2')), prompt_text TEXT, content TEXT NOT NULL,
    word_count INTEGER NOT NULL, started_at TEXT, submitted_at TEXT NOT NULL, duration_sec INTEGER,
    metadata_json TEXT NOT NULL,
    FOREIGN KEY(attempt_id) REFERENCES exam_attempts(attempt_id) ON DELETE CASCADE
  );
  CREATE TABLE IF NOT EXISTS submission_receipts (
    receipt_id TEXT PRIMARY KEY, attempt_id TEXT NOT NULL, sink TEXT NOT NULL, database_file_name TEXT NOT NULL,
    relative_path TEXT NOT NULL, written_at TEXT NOT NULL, payload_hash TEXT,
    FOREIGN KEY(attempt_id) REFERENCES exam_attempts(attempt_id) ON DELETE CASCADE
  );
  CREATE TABLE IF NOT EXISTS exam_kv (
    key TEXT PRIMARY KEY, value_json TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
  );
`;

// 答对模式：T=对，F=错。P1/P2 有重叠用时（用于验证「用时可能重叠」）。
const ATTEMPTS = [
  { id: "A1", terminal: "T1", cand: ["C-001", "张三"], submitted: "2026-11-07T11:10:00+08:00", passages: [["P1", "TTFT", "10:00", "10:20"], ["P2", "TTTT", "10:15", "10:40"], ["P3", "FFTF", "10:50", "11:05"]] },
  { id: "A2", terminal: "T1", cand: ["C-002", "李四"], submitted: "2026-11-07T11:20:00+08:00", passages: [["P1", "TFFF", "11:25", "11:40"], ["P2", "TTFF", "11:42", "11:55"], ["P3", "TTTT", "12:00", "12:15"]] },
  { id: "A3", terminal: "T2", cand: ["C-002", "李四四"], submitted: "2026-11-07T11:30:00+08:00", passages: [["P1", "TTTT", "10:00", "10:20"], ["P2", "TTTT", "10:30", "10:50"], ["P3", "TTTT", "11:00", "11:20"]] },
  { id: "A4", terminal: "T2", cand: ["C-004", "张三"], submitted: "2026-11-07T11:40:00+08:00", passages: [["P1", "FFFF", "10:00", "10:20"], ["P2", "TFFF", "10:30", "10:50"], ["P3", "FFFF", "11:00", "11:20"]] },
];
const KIND = { P1: "true_false_not_given", P2: "matching_headings", P3: "sentence_completion" };
const EXPECTED_RAW = { A1: 8, A2: 7, A3: 12, A4: 1 };

function recordAssertion(report, id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

function sha256(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function isoAt(hhmm) {
  return `2026-11-07T${hhmm}:00+08:00`;
}

/** 写一个学生端形态的终端库（整库：含该终端全部已交卷 attempt）。 */
function writeTerminalDb(file, attempts) {
  fs.rmSync(file, { force: true });
  const db = new DatabaseSync(file);
  db.exec(STUDENT_SCHEMA_SQL);
  for (const a of attempts) {
    db.prepare(`INSERT INTO exam_attempts (attempt_id, batch_id, terminal_id, candidate_id, candidate_name, checkin_at,
        started_at, submitted_at, status, app_version, runtime_json)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'submitted', '1.0.0', '{}')`)
      .run(a.id, BATCH, a.terminal, a.cand[0], a.cand[1], isoAt("09:50"), isoAt("10:00"), a.submitted);
    const suite = `suite-${a.id}`;
    db.prepare(`INSERT INTO reading_suite_attempts (suite_attempt_id, attempt_id, status, started_at, completed_at, submitted_at, total_passages, aggregate_json)
      VALUES (?, ?, 'submitted', ?, ?, ?, 3, ?)`)
      .run(suite, a.id, isoAt("10:00"), a.submitted, a.submitted, JSON.stringify({ passageCount: 3, paperId: PAPER }));
    db.prepare("INSERT INTO exam_kv (key, value_json) VALUES (?, ?)").run(`scorer_version:${a.id}`, JSON.stringify("reading-scorer/1"));
    a.passages.forEach(([category, pattern, start, end], passageIndex) => {
      const correct = [...pattern].filter((c) => c === "T").length;
      const score = { gradable: true, correct, total: pattern.length, totalQuestions: pattern.length, percentage: Math.round((correct / pattern.length) * 100), source: "practice_reading_session" };
      db.prepare(`INSERT INTO reading_passages (suite_attempt_id, passage_index, asset_id, exam_id, title, category, started_at, ended_at, duration_sec, score_info_json, raw_submission_json)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1200, ?, '{}')`)
        .run(suite, passageIndex, `asset-${a.id}-${passageIndex}`, `exam-${passageIndex}`, `Passage ${passageIndex + 1}`, category, isoAt(start), isoAt(end), JSON.stringify(score));
      [...pattern].forEach((ch, n) => {
        const ok = ch === "T";
        const user = ok ? "TRUE" : "FALSE";
        db.prepare(`INSERT INTO reading_answers (suite_attempt_id, passage_index, question_id, display_label, question_kind, control, source, match_mode,
            user_answer_json, correct_answer_json, normalized_user_answer_json, normalized_correct_answer_json, is_correct, weight,
            change_count, visit_count, elapsed_ms, duration_ms)
          VALUES (?, ?, ?, ?, ?, 'radio', 'answerKey', 'exact', ?, ?, ?, ?, ?, 1, ?, ?, ?, ?)`)
          .run(suite, passageIndex, `p${passageIndex}q${n + 1}`, `Q${passageIndex * 4 + n + 1}`, KIND[category],
            JSON.stringify(user), JSON.stringify("TRUE"), JSON.stringify([user]), JSON.stringify(["TRUE"]), ok ? 1 : 0,
            n % 3, ((n + 2) % 4), (n + 1) * 1000, (n + 2) * 100);
      });
    });
  }
  db.close();
}

/** 发布一份快照：sqlite + meta.json + .ready（ready 最后写）。 */
function publishSnapshot(submissionsRoot, terminal, attemptId, dbBytes, exportedAt, { tamperSha = false } = {}) {
  const dir = path.join(submissionsRoot, "snapshots", terminal);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, `${attemptId}.sqlite`), dbBytes);
  const meta = {
    schemaVersion: "SubmissionSnapshotV1", terminalId: terminal, installId: null, attemptId, batchId: BATCH, paperId: PAPER,
    candidateId: "C-x", candidateName: "x", submittedAt: exportedAt, exportedAt, appVersion: "1.0.0",
    dbSchemaVersion: 1, scorerVersion: "reading-scorer/1", sizeBytes: dbBytes.length,
    sha256: tamperSha ? "0".repeat(64) : sha256(dbBytes),
  };
  fs.writeFileSync(path.join(dir, `${attemptId}.meta.json`), JSON.stringify(meta, null, 2));
  fs.writeFileSync(path.join(dir, `${attemptId}.ready`), "");
}

function writeStatus(submissionsRoot, terminal, attemptId, status, cand, updatedAt) {
  const dir = path.join(submissionsRoot, "status", terminal);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, `${attemptId}.json`), JSON.stringify({
    schemaVersion: "AttemptStatusV1", attemptId, terminalId: terminal, batchId: BATCH, paperId: PAPER,
    candidateId: cand[0], candidateName: cand[1], status, updatedAt, appVersion: "1.0.0",
  }));
}

function generateFixtures(submissionsRoot) {
  const work = path.join(runDir, "fixture-work");
  fs.mkdirSync(work, { recursive: true });
  const t1 = path.join(work, "t1.sqlite");
  writeTerminalDb(t1, ATTEMPTS.filter((a) => a.terminal === "T1"));
  const t1Bytes = fs.readFileSync(t1);
  publishSnapshot(submissionsRoot, "T1", "A1", t1Bytes, "2026-11-07T11:10:05+08:00");
  publishSnapshot(submissionsRoot, "T1", "A2", t1Bytes, "2026-11-07T11:10:05+08:00");
  const t2 = path.join(work, "t2.sqlite");
  writeTerminalDb(t2, ATTEMPTS.filter((a) => a.terminal === "T2"));
  publishSnapshot(submissionsRoot, "T2", "A3", fs.readFileSync(t2), "2026-11-07T11:40:05+08:00");
  // 坏快照：meta 的 sha256 与文件不符 -> 进入异常队列，不影响其他。
  publishSnapshot(submissionsRoot, "T3", "A5", t1Bytes, "2026-11-07T12:00:00+08:00", { tamperSha: true });
  for (const a of ATTEMPTS) writeStatus(submissionsRoot, a.terminal, a.id, "submitted", a.cand, a.submitted);
  writeStatus(submissionsRoot, "T1", "A8", "in_progress", ["C-005", "王五"], "2026-11-07T11:50:00+08:00");
  writeStatus(submissionsRoot, "T1", "A9", "checked_in", ["C-006", "赵六"], "2026-11-07T11:55:00+08:00");
  writeStatus(submissionsRoot, "T2", "A10", "failed", ["C-007", "钱七"], "2026-11-07T11:45:00+08:00");
  fs.rmSync(work, { recursive: true, force: true });
}

async function main() {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  const report = {
    task: "data-module",
    scope: "数据模块：作答快照导入 → 批次总览 → 考试页统计 → 学生详情 → 题目分布 → Excel 导出 → 设置页作答数据目录",
    channel: CDP_CHANNEL_LABEL, channelNote: CDP_CHANNEL_NOTE,
    evidenceLevel: "real-app-e2e (CDP automation channel + disk facts); submissions are node:sqlite fixtures of the student V1 schema",
    identity: { exePath, commit: gitHead(repoRoot), buildFresh: buildFreshReport(fresh) },
    assertions: [], consoleErrors: [], pageExceptions: [], screenshots: [],
  };
  fs.mkdirSync(runDir, { recursive: true });
  const submissionsRoot = path.join(runDir, "submissions");
  generateFixtures(submissionsRoot);
  const exportPath = path.join(runDir, "exports", "data-module-exam.xlsx");
  fs.mkdirSync(path.dirname(exportPath), { recursive: true });
  report.submissionsRoot = submissionsRoot;

  const session = await launchTauriAppCdp({
    exePath, runDir, extraBrowserArgs: extraArgs,
    appEnv: { PDF2TEST_AUTOMATION_EXPORT_XLSX: exportPath },
  });
  let verdict = "failed";
  const shot = async (name) => {
    const file = await session.screenshot(name);
    if (file) report.screenshots.push(file);
    return file;
  };
  try {
    report.browserArgs = session.browserArgs;
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 60000, label: "library" });

    // 1. 把作答数据目录写进设置（与其他 e2e 相同：readAppSettings 每次读 localStorage）。
    await session.evaluate(`(() => {
      const raw = window.localStorage.getItem(${JSON.stringify(APP_SETTINGS_KEY)});
      const settings = raw ? JSON.parse(raw) : {};
      settings.submissionsRoot = ${JSON.stringify(submissionsRoot)};
      window.localStorage.setItem(${JSON.stringify(APP_SETTINGS_KEY)}, JSON.stringify(settings));
      return true;
    })()`);
    await session.evaluate(`(() => { window.location.hash = "#/data"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="data-page"]')`, { timeoutMs: 20000, label: "data-page" });
    await session.waitFor(`!!document.querySelector('[data-testid="data-import"]')`, { timeoutMs: 20000, label: "import-button" });
    const navLabels = await session.evaluate(`[...document.querySelectorAll('.primary-nav .nav-label')].map((n) => n.textContent.trim())`);
    recordAssertion(report, "D-1 nav-has-data-entry", JSON.stringify(navLabels) === JSON.stringify(["导入", "题库", "数据", "设置"]),
      `一级入口=${JSON.stringify(navLabels)}`);
    // 导入前：状态信标已能给出批次（实时签到/交卷计数），但快照侧「已导入」为 0，有效计分未出现。
    await session.waitFor(`!!document.querySelector('[data-testid="data-batch-card"]') || !!document.querySelector('[data-testid="data-no-batches"]')`, { timeoutMs: 20000, label: "batches-loaded" });
    const beforeText = await session.evaluate(`document.querySelector('[data-testid="data-batch-card"]')?.textContent || ""`);
    const beforeOk = beforeText.includes("已导入 0 人") && !beforeText.includes("有效计分 N=4");
    recordAssertion(report, "D-2 before-import-beacon-only", beforeOk, `导入前批次卡片=${JSON.stringify(beforeText.slice(0, 120))}`);
    await shot("data-01-before-import");

    // 2. 从作答目录导入。
    await session.clickSelector('[data-testid="data-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="data-import-result"]')`, { timeoutMs: 60000, label: "import-result" });
    const importText = await session.evaluate(`document.querySelector('[data-testid="data-import-result"]').textContent`);
    report.importResultText = importText;
    recordAssertion(report, "D-3 import-counts", /新导入 2，跳过 1，失败 1/.test(importText),
      `导入结果=${JSON.stringify(importText)}`);

    // 3. 批次总览：有效计分 N=4、异常队列、身份待核对。
    await session.waitFor(`!!document.querySelector('[data-testid="data-batch-card"]')`, { timeoutMs: 20000, label: "batch-card" });
    const cardText = await session.evaluate(`document.querySelector('[data-testid="data-batch-card"]').textContent`);
    recordAssertion(report, "D-4 batch-card-N4", cardText.includes("有效计分 N=4") && cardText.includes("已导入 4 人"),
      `批次卡片=${JSON.stringify(cardText.slice(0, 160))}`);
    const exceptionText = await session.evaluate(`document.querySelector('[data-testid="data-exceptions"]')?.textContent || ""`);
    recordAssertion(report, "D-5 exception-queue-shows-bad-snapshot", exceptionText.includes("sha256 不符"), `异常队列=${JSON.stringify(exceptionText.slice(0, 120))}`);
    const identityText = await session.evaluate(`document.querySelector('[data-testid="data-identity"]')?.textContent || ""`);
    recordAssertion(report, "D-6 identity-review-listed", identityText.includes("C-002") && identityText.includes("张三"), `待核对=${JSON.stringify(identityText.slice(0, 160))}`);
    await shot("data-02-batches");

    // 4. 单次考试：统计、分布、矩阵、学生表（全部标 N）。
    await session.clickByText("查看考试");
    await session.waitFor(`!!document.querySelector('[data-testid="data-exam"]')`, { timeoutMs: 20000, label: "exam" });
    const examText = await session.evaluate(`document.querySelector('[data-testid="data-exam"]').textContent`);
    recordAssertion(report, "D-7 exam-stats-with-n", examText.includes("N=4") && !/Band/.test(examText), `考试页含 N=4 且无 Band`);
    const histogramBars = await session.evaluate(`document.querySelectorAll('[data-testid="data-histogram"] .data-bar').length`);
    recordAssertion(report, "D-8 histogram-rendered", histogramBars >= 13, `分布柱数=${histogramBars}（满分 12 → 13 箱）`);
    const matrixRows = await session.evaluate(`document.querySelectorAll('[data-testid="data-matrix"] tbody tr').length`);
    recordAssertion(report, "D-9 matrix-rows-equal-n", matrixRows === 4, `矩阵行数=${matrixRows}（N=4）`);
    const hardText = await session.evaluate(`document.querySelector('[data-testid="data-hard"]')?.textContent || ""`);
    recordAssertion(report, "D-10 small-sample-no-hard-verdict", !hardText.includes("难题\n") && hardText.includes("样本不足"), `难题区=${JSON.stringify(hardText.slice(0, 140))}`);
    await shot("data-03-exam");

    // 5. 学生详情。
    await session.evaluate(`(() => {
      const btn = [...document.querySelectorAll('[data-testid="data-students"] button')].find((b) => b.textContent.trim() === '张三');
      if (!btn) return false; btn.click(); return true;
    })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="data-student"]')`, { timeoutMs: 20000, label: "student" });
    const studentText = await session.evaluate(`document.querySelector('[data-testid="data-student"]').textContent`);
    recordAssertion(report, "D-11 student-raw-score", studentText.includes("8 / 12"), `学生原始分文本含 8 / 12`);
    recordAssertion(report, "D-12 student-overlap-flag", studentText.includes("用时可能重叠"), `学生页含用时重叠提示`);
    await shot("data-04-student");

    // 6. 题目详情（从错题进入）。
    await session.evaluate(`(() => {
      const btn = [...document.querySelectorAll('[data-testid="data-student"] button')].find((b) => b.textContent.trim() === '看此题');
      if (!btn) return false; btn.click(); return true;
    })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="data-question"]')`, { timeoutMs: 20000, label: "question" });
    const questionText = await session.evaluate(`document.querySelector('[data-testid="data-question"]').textContent`);
    recordAssertion(report, "D-13 question-n-and-distribution", /N=\d/.test(questionText) && questionText.includes("答案分布"), `题目页=${JSON.stringify(questionText.slice(0, 140))}`);
    await shot("data-05-question");

    // 7. 导出 Excel：保存路径由自动化钩子指定，断言磁盘上真的生成了 xlsx。
    await session.evaluate(`(() => { window.location.hash = ${JSON.stringify("#/data/exam?batch=" + encodeURIComponent(BATCH) + "&paper=" + encodeURIComponent(PAPER))}; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="data-export"]')`, { timeoutMs: 20000, label: "export-button" });
    await session.clickSelector('[data-testid="data-export"]');
    await session.waitFor(`!!document.querySelector('[data-testid="data-export-notice"]')`, { timeoutMs: 30000, label: "export-notice" });
    const xlsxExists = fs.existsSync(exportPath);
    const head = xlsxExists ? fs.readFileSync(exportPath).subarray(0, 2).toString("latin1") : "";
    recordAssertion(report, "D-14 xlsx-written", xlsxExists && head === "PK", `导出文件=${exportPath} exists=${xlsxExists} magic=${JSON.stringify(head)}`);

    // 8. 设置页显示作答数据目录（与 NAS 发布目录分开）。
    await session.evaluate(`(() => { window.location.hash = "#/settings"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="settings-submissions"]')`, { timeoutMs: 20000, label: "settings" });
    const settingsText = await session.evaluate(`document.querySelector('[data-testid="settings-submissions"]').textContent`);
    recordAssertion(report, "D-15 settings-shows-submissions-root", settingsText.includes(submissionsRoot), `设置页含作答数据目录`);
    await shot("data-06-settings");

    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    verdict = report.assertions.every((a) => a.ok) ? "passed" : "failed";
  } finally {
    report.verdict = verdict;
    report.failedAssertions = report.assertions.filter((a) => !a.ok).map((a) => a.id);
    await session.close?.().catch?.(() => {});
  }
  const file = writeReport(runDir, report);
  console.log(`[data-module] verdict=${report.verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
  console.log(`[data-module] report=${file}`);
  process.exitCode = report.verdict === "passed" ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  console.error(cannotRun ? `[data-module] CANNOT RUN: ${error.message}` : error);
  process.exitCode = cannotRun ? 3 : 1;
});
