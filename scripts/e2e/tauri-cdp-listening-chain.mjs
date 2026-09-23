#!/usr/bin/env node
/**
 * T6 / R6：**听力真实 App 验收链**（真实 Tauri 应用 + WebView2 CDP 通道）。
 *
 * 为什么单列一条：听力这条链上有一段**在真实 App 里从未跑过**的路——
 * 「识别到听力 → 弹 `ListeningAudioDialog` → 用户挑 4 段音频 → 每段过探针 →
 * 确认导入」。此前它跑不起来有两个叠加原因：
 *   1. 音频选择走的是**前端直接调 dialog 插件**，没有自动化钩子（已修，见
 *      `automation_audio_selection_from_env`）；
 *   2. CDP 通道在受污染的执行环境里起不来（已修，见 findings.md F-WEBVIEW2-…）。
 *
 * 本脚本断言的就是**这一段产品行为**，全部走真实界面点击：
 *   1. 题库页加载；
 *   2. 「选择文件」把听力卷放进已选清单；
 *   3. 开始导入 → 弹出听力确认弹窗（说明产品**认出了**这是听力卷）；
 *   4. 点「选择音频文件」→ 真实 picker 钩子交回 4 段音频 → 清单里恰好 4 个 Part，
 *      顺序与文件一致（音频顺序 = Part 顺序，这是产品语义）；
 *   5. 4 段的探针**全部通过**（失败态是 `audio-probe-blocked`，不许当通过）；
 *   6. 确认导入 → 题库里出现这一条。
 *
 * 之后**只做记录、不做断言**：打开该条目，如实记下发布按钮是否可用与原因。
 * 听力卷 `fixtures/golden/private-real/listening-vol7-t9.pdf` 本身**没有答案 key**，
 * 发布能否成立取决于上游识别/教材校验，不属于本条要证明的范围；把它写成断言
 * 只会把「数据不全」伪装成「App 链路失败」。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-listening-chain.mjs [--keep] [--run-dir <dir>]
 *        [--diagnostic-args] [--tolerate-concurrent-edits]
 *
 * ⚠️ 本沙箱下**必须**带 `--diagnostic-args`：不加 `--no-sandbox --disable-gpu` 时
 * WebView2 renderer 会在启动约 7s 后崩掉（DevTools 端点消失、进程仍在），
 * 这一点仓库自带的 `tauri-cdp-smoke.mjs` 早已写明。带诊断参数的运行会记为
 * `runProfile=cdp-diagnostic`，**不得**当成默认产品路径通过。
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
  isCleanPublishOutcome,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const runDirIdx = process.argv.indexOf("--run-dir");
const runDir = runDirIdx >= 0
  ? path.resolve(process.argv[runDirIdx + 1])
  : path.join(repoRoot, "artifacts", "e2e-cdp", `run-listening-chain-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const diagnosticArgsRequested = process.argv.includes("--diagnostic-args");
const extraArgs = diagnosticArgsRequested ? "--no-sandbox --disable-gpu" : "";
const tolerateConcurrentEdits = process.argv.includes("--tolerate-concurrent-edits");

// 真实听力卷（4 个 SECTION，Q1–10 / 11–20 / 21–30 / 31–40）。私有夹具，不进 Git 历史。
const PAPER_SOURCE = path.join(repoRoot, "fixtures", "golden", "private-real", "listening-vol7-t9.pdf");
const PAPER_NAME = "listening-vol7-t9.pdf";
const EXPECTED_PARTS = 4;

/**
 * 生成 4 段**能过探针**的 WAV。
 *
 * 探针会拒掉 `AudioNearSilent`（RMS 过低）与 `AudioSevereClipping`，所以不能拿静音充数：
 * 用 440Hz 基频 + 递增泛音的 16kHz 单声道 s16le 正弦，振幅 8000（与探针单测同一量级）。
 * 每段长度不同，顺便让「顺序 = Part 顺序」这件事在时长上也可区分。
 */
function writeSineWav(filePath, { seconds, baseHz }) {
  const sampleRate = 16_000;
  const frames = Math.round(seconds * sampleRate);
  const data = Buffer.alloc(frames * 2);
  for (let i = 0; i < frames; i += 1) {
    const t = i / sampleRate;
    const value = Math.sin(t * baseHz * Math.PI * 2) * 8000;
    data.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(value))), i * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0, "ascii");
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVE", 8, "ascii");
  header.write("fmt ", 12, "ascii");
  header.writeUInt32LE(16, 16);            // fmt chunk size
  header.writeUInt16LE(1, 20);             // PCM
  header.writeUInt16LE(1, 22);             // mono
  header.writeUInt32LE(sampleRate, 24);
  header.writeUInt32LE(sampleRate * 2, 28); // byte rate
  header.writeUInt16LE(2, 32);             // block align
  header.writeUInt16LE(16, 34);            // bits per sample
  header.write("data", 36, "ascii");
  header.writeUInt32LE(data.length, 40);
  fs.writeFileSync(filePath, Buffer.concat([header, data]));
}

const AUDIO_SPECS = [
  { name: "part-1.wav", seconds: 6, baseHz: 440 },
  { name: "part-2.wav", seconds: 7, baseHz: 523 },
  { name: "part-3.wav", seconds: 8, baseHz: 659 },
  { name: "part-4.wav", seconds: 9, baseHz: 784 },
];

const report = {
  task: "R6-listening-real-app-chain",
  scope: "听力卷：选择文件 → 听力弹窗 → 真实 picker 绑 4 段音频（探针全过）→ 确认导入",
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
    paperSource: path.relative(repoRoot, PAPER_SOURCE).replace(/\\/g, "/"),
    parts: [],
  },
  steps: [],
  postChecks: [],
  verdict: "failed",
};

let session = null;
let recorder = null;

const rowIdsExpr = `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`;
const pickedNamesExpr =
  `[...document.querySelectorAll('[data-testid="import-picked-files"] li .file-name')].map(el => el.innerText.trim())`;
/** 弹窗里每个 Part：序号、文件名、探针状态类名。 */
const partRowsExpr = `[...document.querySelectorAll('[data-testid="listening-audio-parts"] li')].map(li => ({
  part: Number(li.getAttribute('data-part')),
  name: (li.querySelector('.file-name')?.innerText ?? '').trim(),
  probeClass: li.querySelector('.audio-probe')?.className ?? '',
  probeText: (li.querySelector('.audio-probe')?.innerText ?? '').trim(),
}))`;

async function main() {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);

  if (!fs.existsSync(PAPER_SOURCE)) throw new CannotRunError(`听力夹具不存在：${PAPER_SOURCE}`);

  // ── 布景：试卷放进 harness 给「选择 PDF 文件夹」用的目录；4 段音频单独放 ──
  const pdfDir = path.join(runDir, "pdfs");
  const audioDir = path.join(runDir, "audio");
  fs.mkdirSync(pdfDir, { recursive: true });
  fs.mkdirSync(audioDir, { recursive: true });

  const paperPath = path.join(pdfDir, PAPER_NAME);
  fs.copyFileSync(PAPER_SOURCE, paperPath);

  const audioPaths = [];
  for (const spec of AUDIO_SPECS) {
    const target = path.join(audioDir, spec.name);
    writeSineWav(target, spec);
    audioPaths.push(target);
    report.identity.parts.push({
      name: spec.name,
      seconds: spec.seconds,
      baseHz: spec.baseHz,
      sha256: sha256File(target),
      sizeBytes: fs.statSync(target).size,
    });
  }

  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    appEnv: {
      PDF2TEST_AUTOMATION_SOURCE_FILES: paperPath,
      PDF2TEST_AUTOMATION_AUDIO_FILES: audioPaths.join(path.delimiter),
    },
  });
  report.identity.browserArgs = session.browserArgs;
  recorder = createStepRecorder({ session, artifactsDir: runDir });

  await recorder.run("library-page-loads", async () => {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });
    await session.screenshot("01-library");
    const rows = await session.evaluate(rowIdsExpr);
    if (rows.length !== 0) throw new Error(`预期题库页开始时为空，实际 ${rows.length} 行`);
    return { rowsAtStart: rows };
  });

  await recorder.run("import-drawer-takes-the-listening-paper", async () => {
    await session.clickSelectorWhenStable('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 20000, label: "import-drawer" });
    await session.clickSelectorWhenStable('[data-testid="import-pick-files"]');
    const names = await session.waitFor(
      `(${pickedNamesExpr}).length === 1`,
      { timeoutMs: 20000, label: "picked-one-paper" }
    ).then(() => session.evaluate(pickedNamesExpr));
    if (names[0] !== PAPER_NAME) throw new Error(`已选清单应为 ${PAPER_NAME}，实际 ${JSON.stringify(names)}`);
    return { picked: names };
  });

  await recorder.run("listening-dialog-asks-for-part-audio", async () => {
    await session.clickSelectorWhenStable('[data-testid="import-start"]');
    await session.waitFor(
      `!!document.querySelector('[data-testid="listening-audio-dialog"]')`,
      { timeoutMs: 60000, label: "listening-audio-dialog" }
    );
    await session.screenshot("02-listening-dialog");
    return { dialog: "listening-audio-dialog" };
  });

  await recorder.run("real-picker-binds-four-parts-in-order", async () => {
    await session.clickSelectorWhenStable('[data-testid="listening-audio-pick-files"]');
    await session.waitFor(
      `(${partRowsExpr}).length === ${EXPECTED_PARTS}`,
      { timeoutMs: 30000, label: "four-parts" }
    );
    const rows = await session.evaluate(partRowsExpr);
    const names = rows.map((row) => row.name);
    const expected = AUDIO_SPECS.map((spec) => spec.name);
    // 顺序即 Part 顺序：这是产品语义，不是巧合。
    if (JSON.stringify(names) !== JSON.stringify(expected)) {
      throw new Error(`Part 顺序应等于文件顺序 ${JSON.stringify(expected)}，实际 ${JSON.stringify(names)}`);
    }
    const ordinals = rows.map((row) => row.part);
    if (JSON.stringify(ordinals) !== JSON.stringify([1, 2, 3, 4])) {
      throw new Error(`Part 序号应为 1..4，实际 ${JSON.stringify(ordinals)}`);
    }
    await session.screenshot("03-four-parts");
    return { parts: rows.map((row) => ({ part: row.part, name: row.name })) };
  });

  await recorder.run("every-part-probe-passes", async () => {
    // 探针是异步的：等每一行的状态类离开 pending。
    await session.waitFor(
      `(${partRowsExpr}).every(r => !r.probeClass.includes('audio-probe-pending'))`,
      { timeoutMs: 30000, label: "probes-settled" }
    );
    const rows = await session.evaluate(partRowsExpr);
    const blocked = rows.filter((row) => !row.probeClass.includes("audio-probe-passed"));
    if (blocked.length) {
      throw new Error(`这 4 段音频是探针单测同款的 440Hz 正弦，不该被拒；被拒：${JSON.stringify(blocked)}`);
    }
    await session.screenshot("04-probes-passed");
    return { probes: rows.map((row) => ({ part: row.part, text: row.probeText })) };
  });

  await recorder.run("confirm-imports-the-listening-item", async () => {
    await session.clickSelectorWhenStable('[data-testid="listening-confirm"]');
    await session.waitFor(`(${rowIdsExpr}).length === 1`, { timeoutMs: 60000, label: "one-item" });
    const rows = await session.evaluate(rowIdsExpr);
    await session.screenshot("05-imported");
    report.identity.itemId = rows[0];
    return { itemIds: rows };
  });

  // ── 发布：断言「App 必须给出机器可读结论」，但**不**断言它一定是干净发布 ──
  // 这条听力卷没有答案 key，能否干净发布取决于上游识别/教材校验。把「必须干净」写成断言，
  // 就会把「数据不全」伪装成「App 链路失败」。这里要证明的是：发布路径真的走通了、
  // 并且如实给出了 `data-publish-outcome`（干净 / 放行 / 放行且学生打不开 / 失败，四选一）。
  await recorder.run("publish-reports-a-machine-readable-outcome", async () => {
    // 真实 App 里「点条目 → 工作区就绪」这一步本身带竞态（识别在后台跑，工作区可能先挂起来
    // 再重渲染）。允许**重试一次点开**，并把「重试过」如实写进证据，而不是把它藏成一次成功。
    const openWorkspace = async () => {
      await session.clickSelectorWhenStable('[data-testid="library-row"]');
      return session
        .waitFor(
          `!!document.querySelector('[data-testid="workspace-publish"]')`,
          { timeoutMs: 120000, label: "workspace-publish" }
        )
        .then(() => true)
        .catch(() => false);
    };
    const firstAttempt = await openWorkspace();
    const retried = !firstAttempt;
    if (retried && !(await openWorkspace())) {
      throw new Error("点开条目后两次 120s 等待内仍未出现发布按钮");
    }
    await session.screenshot("06-workspace");
    const outcome = await session.publishAndReadOutcome({ timeoutMs: 180000 });
    if (!outcome.kind) {
      throw new Error(
        `发布未给出机器可读结论：timedOut=${outcome.timedOut} text=${JSON.stringify(outcome.text)}`
      );
    }
    await session.screenshot("07-published");
    report.postChecks.push({
      name: "publish-outcome",
      kind: outcome.kind,
      text: outcome.text,
      clean: isCleanPublishOutcome(outcome.kind),
      workspaceOpenRetried: retried,
    });
    return { ...outcome, workspaceOpenRetried: retried };
  });
}

try {
  await main();
} catch (error) {
  const cannotRun = error instanceof CannotRunError;
  report.cannotRun = cannotRun;
  report.fatal = {
    name: error?.name ?? "Error",
    message: String(error?.message ?? error),
    appOutput: error?.appOutput ?? null,
  };
  console.error(`[listening-chain] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${error?.message ?? error}`);
} finally {
  if (session) {
    try { await session.close({ keep }); } catch {}
  }
  // `createStepRecorder` 把结果攒在自己的 `steps` 里，必须回填，否则报告里 steps 永远是空的。
  if (recorder) report.steps = recorder.steps;
  report.finishedAt = new Date().toISOString();
  const failedSteps = report.steps.filter((step) => step.status === "failed").map((step) => step.name);
  report.summary = {
    passed: report.steps.filter((step) => step.status === "passed").length,
    failed: failedSteps.length,
    failedSteps,
    blocked: report.steps.filter((step) => step.status === "blocked").length,
  };
  // 判定**由步骤派生**，不另算一个答案：`recorder.run` 自己吞掉异常并记 failed，
  // 所以 main() 可能正常返回而某一步其实是红的 —— 只有从 steps 派生才不会写出
  // 「verdict=passed 却带着一条 failed」的自相矛盾报告。
  report.verdict = report.cannotRun ? "cannot-run" : failedSteps.length ? "failed" : "passed";
  writeReport(runDir, report);
  const line = report.steps.map((step) => `${step.name}:${step.status}`).join(" | ");
  console.log(`[listening-chain] verdict=${report.verdict} report=${path.join(runDir, "report.json")}`);
  console.log(`[listening-chain] steps: ${line}`);
  if (report.fatal) console.log(`[listening-chain] fatal=${report.fatal.message}`);
  for (const check of report.postChecks) console.log(`[listening-chain] postCheck ${JSON.stringify(check)}`);
  process.exit(report.verdict === "passed" ? 0 : report.verdict === "cannot-run" ? 3 : 1);
}
