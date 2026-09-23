#!/usr/bin/env node
/**
 * T5-e：**单文件导入端到端确认**（真实 Tauri 应用 + WebView2 CDP 通道）。
 *
 * 要证明的产品行为：
 *   同一个目录里放 3 份 PDF，用户**只用「选择文件」选 1 份**，
 *   产品必须**恰好建立 1 个条目**，并且**只处理被选中的那一份**。
 *
 * 为什么这条值得单独验：
 *   导入有两条产品入口，自动化钩子也分两条，它们的选择语义完全不同：
 *     - 「选择 PDF 文件夹」→ `pick_pdf_folder_sources` → `PDF2TEST_AUTOMATION_PDF_DIR`
 *       → `list_pdf_files_in_dir`：**把目录里的 PDF 全列出来**（用户没逐份挑过）；
 *     - 「选择文件」      → `automation_source_files` → `PDF2TEST_AUTOMATION_SOURCE_FILES`
 *       → 只返回清单里那几份（用户在系统对话框里挑过）。
 *   一旦「选择文件」这条路径被目录扫描污染，用户挑 1 份就会莫名多出条目，
 *   而且多出来的条目会**真的被解析、真的占资源**。所以本脚本对同一个目录
 *   同时跑两条入口，把「1」和「3」放在同一次运行里对照 —— 否则「恰好 1 个」
 *   无法排除「目录里本来就只有 1 份」这种平凡解释。
 *
 * 判定（全部是产品链断言，不看日志、不看内部状态）：
 *   1. 点「选择文件」后，已选清单里**恰好 1 项**，且就是被指定的那一份；
 *   2. 开始导入后，题库里**恰好新增 1 行**，标题对应那一份；
 *   3. 磁盘上**恰好 1 个 job 目录**，其 `uploads/` 里恰好 1 个落地文件，
 *      文件名属于被选中的那一份；另外两份的名字**在任何 job 目录里都不出现**；
 *   4. 同一个目录走「选择 PDF 文件夹」时，已选清单是**3 项**（对照，证明目录里真有 3 份）；
 *   5. 取消这次文件夹选择后，题库行数与 job 目录数**都没有变化**（没被顺手导入）。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-single-file-import.mjs [--keep] [--run-dir <dir>]
 *        [--diagnostic-args] [--tolerate-concurrent-edits]
 *
 * 运行档案：默认**不带**测试专用安全参数（`runProfile=cdp-default`）；
 * 传 `--diagnostic-args` 才加 `--no-sandbox --disable-gpu`（`runProfile=cdp-diagnostic`）。
 * 两类证据必须分开记录，诊断运行不得当作默认产品路径通过。
 *
 * 退出码：0 = passed；1 = failed；3 = CANNOT-RUN。
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  CDP_CHANNEL_LABEL,
  CDP_CHANNEL_NOTE,
  CannotRunError,
  assertBuildFresh,
  buildFreshReport,
  createStepRecorder,
  gitHead,
  gitWorktreeClean,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  sleep,
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const runDirIdx = process.argv.indexOf("--run-dir");
const runDir = runDirIdx >= 0
  ? path.resolve(process.argv[runDirIdx + 1])
  : path.join(repoRoot, "artifacts", "e2e-cdp", `run-single-file-import-${new Date().toISOString().replace(/[:.]/g, "-")}`);
// 测试专用安全参数默认**不开**：默认运行必须贴近真实产品路径。
const diagnosticArgsRequested = process.argv.includes("--diagnostic-args");
const extraArgs = diagnosticArgsRequested ? "--no-sandbox --disable-gpu" : "";
const tolerateConcurrentEdits = process.argv.includes("--tolerate-concurrent-edits");

// 三份同目录 PDF，用**受控文件名**（不用夹具原名）：这样「只处理被选中的那一份」
// 可以按文件名精确断言，也不会跟别的运行目录里的同名文件混淆。
// 选中间那一份（bravo）而不是第一份：选第一份时「只导入 1 份」跟「只导入了排在最前的」
// 无法区分，中间那份才能排除「按顺序只取第一个」这类平凡实现。
const STAGED_FILES = [
  { name: "alpha-reading.pdf", source: path.join(repoRoot, "fixtures", "parser", "complex-reading.pdf") },
  { name: "bravo-reading.pdf", source: path.join(repoRoot, "fixtures", "golden", "synthetic", "pdf", "pdf-two-column.pdf") },
  { name: "charlie-reading.pdf", source: path.join(repoRoot, "fixtures", "golden", "synthetic", "pdf", "pdf-three-column.pdf") },
];
const CHOSEN = "bravo-reading.pdf";
const CHOSEN_STEM_WORDS = "bravo reading";
const NOT_CHOSEN = STAGED_FILES.map((f) => f.name).filter((name) => name !== CHOSEN);

const report = {
  task: "T5-e-single-file-import-end-to-end",
  scope: "同目录 3 份 PDF，只用「选择文件」选 1 份 ⇒ 恰好 1 个条目、只处理这一份",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  diagnosticRun: Boolean(extraArgs),
  runProfile: extraArgs ? "cdp-diagnostic" : "cdp-default",
  securityArgs: extraArgs ? extraArgs.split(/\s+/).filter(Boolean) : [],
  tolerateConcurrentEdits,
  startedAt: new Date().toISOString(),
  runDir,
  identity: {
    exePath,
    exeSha256: null,
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: null,
    stagedDir: path.join(runDir, "pdfs"),
    stagedFiles: [],
    chosenFile: CHOSEN,
    importEntry: "pick-files (PDF2TEST_AUTOMATION_SOURCE_FILES)",
  },
  steps: [],
  verdict: "failed",
};

let session = null;
let recorder = null;

/** 题库页当前所有行的 item id。 */
const rowIdsExpr = `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`;

/** 已选文件清单里的文件名（只取 `.file-name`，不要把「移除」按钮文案算进去）。 */
const pickedNamesExpr =
  `[...document.querySelectorAll('[data-testid="import-picked-files"] li .file-name')].map(el => el.innerText.trim())`;

/** 读磁盘事实：job 目录数与每个 job 的 uploads 落地文件。 */
function readJobFacts(dataDir) {
  const jobsDir = path.join(dataDir, "jobs");
  const facts = { jobsDir, jobDirs: [], uploads: [], uploadsMissing: [] };
  if (!fs.existsSync(jobsDir)) return facts;
  for (const entry of fs.readdirSync(jobsDir)) {
    const dir = path.join(jobsDir, entry);
    if (!fs.statSync(dir).isDirectory()) continue;
    facts.jobDirs.push(entry);
    const uploadsDir = path.join(dir, "uploads");
    if (!fs.existsSync(uploadsDir)) {
      facts.uploadsMissing.push(entry);
      continue;
    }
    for (const file of fs.readdirSync(uploadsDir)) {
      facts.uploads.push({ jobId: entry, file });
    }
  }
  facts.jobDirs.sort();
  facts.uploads.sort((a, b) => a.file.localeCompare(b.file));
  return facts;
}

async function main() {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);

  // ── 布景：3 份 PDF 放进**同一个目录**，也就是 harness 给「选择 PDF 文件夹」用的那个目录 ──
  const stagedDir = path.join(runDir, "pdfs");
  fs.mkdirSync(stagedDir, { recursive: true });
  for (const staged of STAGED_FILES) {
    if (!fs.existsSync(staged.source)) throw new CannotRunError(`夹具不存在：${staged.source}`);
    const target = path.join(stagedDir, staged.name);
    fs.copyFileSync(staged.source, target);
    report.identity.stagedFiles.push({
      name: staged.name,
      source: path.relative(repoRoot, staged.source).replace(/\\/g, "/"),
      sha256: sha256File(target),
      sizeBytes: fs.statSync(target).size,
    });
  }

  const chosenPath = path.join(stagedDir, CHOSEN);
  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    // 只把**被选中的那一份**交给「选择文件」钩子。另外两份仍然躺在同一个目录里，
    // 「选择 PDF 文件夹」会看到它们 —— 这正是对照组的来源。
    appEnv: { PDF2TEST_AUTOMATION_SOURCE_FILES: chosenPath },
  });
  report.identity.browserArgs = session.browserArgs;
  recorder = createStepRecorder({ session, artifactsDir: runDir });

  const dataDir = path.join(runDir, "appdata", "data");

  // ---- 1. 题库页加载（本条只验导入的选择语义，不需要云端）----
  // **不做 `Page.reload`**：本仓库已有记录，重载会打断 CDP 会话（WebView2 重建 page
  // target，随后 `Runtime.evaluate` 一律报「CDP 连接已关闭」，一次断言都跑不到）。
  // 上一版重载的理由是「让应用重新读 localStorage 里的 cloudEnabled:false」，
  // 但这个理由不成立：`AppSettingsV1` 里根本没有 `cloudEnabled` 字段，写了也读不到。
  // 本条的运行目录是全新的，没有任何 LLM profile ⇒ `listLlmProfiles()` 为空 ⇒ 云端关闭。
  await recorder.run("library-page-loads", async () => {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });
    await session.screenshot("01-library");
    return {
      url: await session.evaluate("location.href"),
      rowsAtStart: await session.evaluate(rowIdsExpr),
    };
  });

  // ---- 2. 「选择文件」只带进被选中的那一份 ----
  let beforeRows = null;
  await recorder.run("pick-files-brings-exactly-the-chosen-file", async () => {
    beforeRows = await session.evaluate(rowIdsExpr);
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
    await session.clickSelector('[data-testid="import-pick-files"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked-files" });
    const picked = (await session.evaluate(pickedNamesExpr)) ?? [];
    await session.screenshot("02-picked-files");
    if (picked.length !== 1) {
      throw new Error(`「选择文件」应恰好带进 1 份，实际 ${picked.length} 份：${JSON.stringify(picked)}`);
    }
    if (picked[0] !== CHOSEN) {
      throw new Error(`「选择文件」带进的是 ${JSON.stringify(picked[0])}，期望 ${JSON.stringify(CHOSEN)}`);
    }
    return {
      picked,
      beforeRows: (beforeRows ?? []).length,
      // 如实记录云端状态：全新运行目录里没有 LLM profile，导入抽屉应显示「未连接云端」。
      cloudOfflineHint: Boolean(await session.evaluate(
        `!!document.querySelector('[data-testid="import-cloud-offline"]')`
      )),
    };
  });

  // ---- 3. 导入后题库恰好新增 1 行，且标题对应被选中的那一份 ----
  let importedItemId = null;
  await recorder.run("import-creates-exactly-one-item", async () => {
    await session.clickSelector('[data-testid="import-start"]');
    // 阅读 PDF 不该弹出听力音频对话框；若弹了，说明这份稿被识别成听力，
    // 本条链就验不到「阅读单文件导入」了，必须如实失败而不是继续往下走。
    const listeningDialog = await session
      .waitFor(`!!document.querySelector('[data-testid="listening-audio-dialog"]')`, { timeoutMs: 6000, label: "listening-dialog-probe" })
      .catch(() => false);
    if (listeningDialog) throw new Error("导入阅读 PDF 时弹出了听力音频对话框，本条链的前提不成立");

    const deadline = Date.now() + 60000;
    let newIds = [];
    while (Date.now() < deadline) {
      const ids = (await session.evaluate(rowIdsExpr)) ?? [];
      newIds = ids.filter((id) => !(beforeRows ?? []).includes(id));
      if (newIds.length) break;
      await sleep(1000);
    }
    if (!newIds.length) throw new Error(`导入后题库未出现新行（导入前 ${(beforeRows ?? []).length} 行）`);
    // 等界面把这一批刷完：乐观行会先出现，随后 store.refresh() 再落一次。
    // 不等就数，会把「还没刷完」误读成「只有 1 行」。
    await sleep(4000);
    const idsAfter = (await session.evaluate(rowIdsExpr)) ?? [];
    const newAfter = idsAfter.filter((id) => !(beforeRows ?? []).includes(id));
    if (newAfter.length !== 1) {
      throw new Error(`导入 1 份应恰好新增 1 行，实际新增 ${newAfter.length} 行：${JSON.stringify(newAfter)}`);
    }
    importedItemId = newAfter[0];
    const rowText = String(await session.evaluate(
      `(() => { const r = document.querySelector('[data-item-id="${importedItemId}"]'); return r ? r.innerText.replace(/\\s+/g, ' ').trim() : ''; })()`
    ) ?? "");
    await session.screenshot("03-imported-row");
    if (!rowText.includes(CHOSEN_STEM_WORDS)) {
      throw new Error(`新行的标题没有对应被选中的那一份（期望含 ${JSON.stringify(CHOSEN_STEM_WORDS)}）：${JSON.stringify(rowText)}`);
    }
    for (const other of NOT_CHOSEN) {
      const otherStem = other.replace(/\.[^.]+$/, "").replace(/[_-]+/g, " ");
      if (rowText.includes(otherStem)) {
        throw new Error(`新行的标题里出现了未被选中的文件（${JSON.stringify(otherStem)}）：${JSON.stringify(rowText)}`);
      }
    }
    return { itemId: importedItemId, rowText, rowsBefore: (beforeRows ?? []).length, rowsAfter: idsAfter.length, newIds: newAfter };
  });

  // ---- 4. 磁盘事实：恰好 1 个 job 目录，且只落地了被选中的那一份 ----
  await recorder.run("only-the-chosen-file-reached-the-jobs-directory", async () => {
    const facts = readJobFacts(dataDir);
    if (facts.jobDirs.length !== 1) {
      throw new Error(`应恰好有 1 个 job 目录，实际 ${facts.jobDirs.length} 个：${JSON.stringify(facts.jobDirs)}`);
    }
    if (facts.jobDirs[0] !== importedItemId) {
      throw new Error(`job 目录 ${JSON.stringify(facts.jobDirs[0])} 与题库新行 ${JSON.stringify(importedItemId)} 不是同一条`);
    }
    if (facts.uploads.length !== 1) {
      throw new Error(`应恰好有 1 个落地文件，实际 ${facts.uploads.length} 个：${JSON.stringify(facts.uploads)}`);
    }
    const landed = facts.uploads[0].file;
    if (!landed.endsWith(CHOSEN)) {
      throw new Error(`落地文件 ${JSON.stringify(landed)} 不是被选中的那一份（${CHOSEN}）`);
    }
    const leaked = facts.uploads.filter((u) => NOT_CHOSEN.some((name) => u.file.includes(name)));
    if (leaked.length) {
      throw new Error(`未被选中的文件也落了盘：${JSON.stringify(leaked)}`);
    }
    return { dataDir, ...facts };
  });

  // ---- 5. 对照：同一个目录走「选择 PDF 文件夹」会带回全部 3 份 ----
  // 这一条不是「顺手多验一个功能」，而是第 2/3/4 步「恰好 1」的**反平凡证据**：
  // 只有证明这个目录里真的有 3 份、且另一条入口确实会取走 3 份，
  // 「选择文件只取 1 份」才是一条真实的选择语义，而不是「目录里只有 1 份」。
  await recorder.run("folder-entry-in-the-same-directory-takes-all-three", async () => {
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer-again" });
    await session.clickSelector('[data-testid="import-pick-folder"]');
    await session.waitFor(
      `(() => { const items = document.querySelectorAll('[data-testid="import-picked-files"] li'); return items.length >= ${STAGED_FILES.length}; })()`,
      { timeoutMs: 20000, label: "folder-picked-files" }
    );
    const picked = (await session.evaluate(pickedNamesExpr)) ?? [];
    await session.screenshot("04-folder-picked-files");
    const expected = STAGED_FILES.map((f) => f.name).sort();
    const actual = [...picked].sort();
    if (actual.length !== expected.length || actual.some((name, index) => name !== expected[index])) {
      throw new Error(`「选择 PDF 文件夹」应带回 ${JSON.stringify(expected)}，实际 ${JSON.stringify(actual)}`);
    }
    return { picked, expected };
  });

  // ---- 6. 取消文件夹选择：什么都没被导入 ----
  await recorder.run("cancelling-the-folder-pick-imports-nothing", async () => {
    // 「取消」按钮没有 testid，用抽屉页脚里那个非 `import-start` 的按钮定位，
    // 不要用全页文本匹配：别处也可能出现「取消」两个字，点错就会关掉别的东西。
    await session.clickSelector('[data-testid="import-drawer"] .drawer-foot button.ghost');
    await session.waitFor(`!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "drawer-closed" });
    await sleep(3000);
    const idsAfter = (await session.evaluate(rowIdsExpr)) ?? [];
    const newAfter = idsAfter.filter((id) => !(beforeRows ?? []).includes(id));
    const facts = readJobFacts(dataDir);
    if (newAfter.length !== 1) {
      throw new Error(`取消文件夹选择后题库行数变了：新增 ${newAfter.length} 行（应仍为 1）`);
    }
    if (facts.jobDirs.length !== 1) {
      throw new Error(`取消文件夹选择后 job 目录数变了：${facts.jobDirs.length} 个（应仍为 1）`);
    }
    if (facts.uploads.length !== 1) {
      throw new Error(`取消文件夹选择后落地文件数变了：${facts.uploads.length} 个（应仍为 1）`);
    }
    await session.screenshot("05-after-cancel");
    return { rowsAfter: idsAfter.length, newRows: newAfter, jobDirs: facts.jobDirs, uploads: facts.uploads };
  });
}

try {
  await main();
} catch (error) {
  const cannotRun = error instanceof CannotRunError;
  report.cannotRun = cannotRun;
  report.fatal = { name: error.name, message: String(error.message), appOutput: error.appOutput ?? null };
  console.error(`[single-file-import] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${error.message}`);
} finally {
  if (session) {
    if (!keep) await session.screenshot("final").catch(() => {});
    const closed = await session.close({ keep });
    report.appOutput = closed.appOutput?.slice(-6000) ?? null;
    report.appProcessExitCode = closed.exitCode;
    report.screenshotErrors = session.screenshotErrors;
  }
  report.finishedAt = new Date().toISOString();
  if (recorder) report.steps = recorder.steps;
  const failed = report.steps.filter((s) => s.status === "failed").map((s) => s.name);
  report.summary = {
    passed: report.steps.filter((s) => s.status === "passed").length,
    failed: failed.length,
    failedSteps: failed,
    blocked: report.steps.filter((s) => s.status === "blocked").length,
  };
  // 判定无条件执行：CANNOT-RUN 时 recorder 为 null，若把判定关在 `if (recorder)` 里，
  // 这种运行会退回初始的 `verdict="failed"`，看起来像跑过且失败。
  report.verdict = report.cannotRun ? "cannot-run" : failed.length ? "failed" : "passed";
  const file = writeReport(runDir, report);
  console.log(`[single-file-import] verdict=${report.verdict} report=${file}`);
  console.log(`[single-file-import] steps: ${report.steps.map((s) => `${s.name}:${s.status}`).join(" | ")}`);
  if (report.fatal) console.log(`[single-file-import] fatal=${report.fatal.message}`);
  process.exit(report.verdict === "passed" ? 0 : report.verdict === "cannot-run" ? 3 : 1);
}
