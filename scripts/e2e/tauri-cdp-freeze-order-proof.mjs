#!/usr/bin/env node
/**
 * 决定性实验：证明「无云导入无批次」的根因是**冻结快照早于权威稿播种**。
 *
 * 假设（H1）：
 *   调度器在 `set_item_status_ready`（它会 `migrate_single_item` 播种权威稿）**之前**
 *   调用 `freeze_local_candidate_snapshot`。冻结要求 `get_canonical_ds` 已存在，
 *   于是首次导入必然拿到 `canonical_not_seeded`；冻结失败 → 无云分支按设计拒绝裁决
 *   → 无批次 → 四条链全部 `not_run`。
 *
 * 检验方式（因果链，而不是相关性）：
 *   A. 导入（无云）→ 记录决策视图：期望 `batchId=null`、四链全 `not_run`；
 *   B. 调一次 `get_workspace_item`（其实现里会 `migrate_single_item`）→ 权威稿被播种；
 *   C. 调 `retry_processing` 重跑同一 job → 冻结此刻能成功 → 期望**批次出现**、四链有状态。
 *
 * 若 A 无批次、C 有批次，则 H1 成立（且修复方向明确：把播种提到冻结之前）。
 * 若 C 仍无批次，则要看**重试到底跑没跑起来**（只读宿主 SQLite 的 `processing_jobs_v2`）：
 *   - 重试被产品自己的草稿保护挡下（`editable_draft_exists`）→ 本实验**不确定**，
 *     C 的前提没成立，既不能证实也不能否证 H1；
 *   - 重试确实执行了却仍无批次 → H1 被否证，另有缺陷阻断 reconcile。
 *
 * 退出码：0 = H1 成立（实验成功复现因果）；1 = H1 被否证；3 = CANNOT-RUN；4 = 不确定（C 条件不成立）。
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { DatabaseSync } from "node:sqlite";
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
const fixtureIdx = process.argv.indexOf("--fixture");
const fixturePath = path.resolve(
  fixtureIdx >= 0
    ? process.argv[fixtureIdx + 1]
    : path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf")
);
const isPdf = /\.pdf$/i.test(fixturePath);
const extraArgs = "--no-sandbox --disable-gpu";
const runDir = path.join(
  repoRoot,
  "artifacts",
  "e2e-cdp",
  `run-freeze-order-${new Date().toISOString().replace(/[:.]/g, "-")}`
);

const report = {
  task: "freeze-before-seed-order-proof",
  scope: "证明「冻结快照早于权威稿播种」导致无云导入无批次（因果实验，非产品验收）",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  startedAt: new Date().toISOString(),
  runDir,
  identity: {
    exePath,
    exeSha256: null,
    fixturePath,
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: null,
  },
  steps: [],
  hypothesis: "H1: freeze_local_candidate_snapshot 早于 migrate_single_item（权威稿播种）执行",
  // C 阶段条件不成立时置 true，最终结论会写成「不确定」而不是「H1 不成立」。
  inconclusive: false,
  verdict: "failed",
};

let session = null;
let recorder = null;

async function call(command, args) {
  const wrapped = command === "apply_recognition_decisions" || command === "apply_editor_commands";
  const r = await session.invoke(command, wrapped ? { input: args } : args);
  if (r && r.__noInvoke) throw new Error("invoke 不可用");
  return r;
}

function stage(v) {
  return { state: v?.state ?? null, reasonCode: v?.reasonCode ?? null, message: v?.message ?? null };
}

function summarize(view) {
  const chains = view?.chains
    ? {
        local: stage(view.chains.local),
        cloud: stage(view.chains.cloud),
        source: stage(view.chains.source),
        adjudication: stage(view.chains.adjudication),
      }
    : null;
  const states = chains ? Object.values(chains).map((c) => c.state) : [];
  return {
    batchId: view?.batchId ?? null,
    chains,
    states,
    allNotRun: states.length > 0 && states.every((s) => s === "not_run"),
  };
}

/** 等本地链进入终态（用于 A 阶段：等识别跑完再断言「没有批次」）。 */
async function waitLocalTerminal(itemId, timeoutMs = 180000) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    const r = await call("get_recognition_decision", { itemId });
    if (r?.ok && r.value) {
      last = r.value;
      const localState = last.chains?.local?.state ?? null;
      if (["succeeded", "partial", "unusable", "failed", "canceled"].includes(localState)) return last;
      if (last.batchId) return last;
    }
    await sleep(2000);
  }
  return last;
}

/**
 * 只读宿主 SQLite，取这个 job 在 `processing_jobs_v2` 里的真实失败码（诊断用，不改数据）。
 *
 * 为什么需要它：C 阶段的「重试后没有批次」有**两种**完全不同的原因——
 *   1. 重试真的跑了，但链路仍不产出批次（那才是「另有缺陷阻断 reconcile」）；
 *   2. 重试**压根没跑起来**，被产品自己的保护挡下（`editable_draft_exists`）。
 * 只看识别视图分不出这两种，会把第 2 种误读成第 1 种。
 * 本轮实测就撞上了：DB 里 `last_error_code = "editable_draft_exists; pass allowOverwrite=true
 * before regenerating draft"`，而脚本当时直接断言「H1 不成立：另有缺陷阻断 reconcile」。
 */
function readJobFailure(itemId) {
  const dbPath = path.join(runDir, "appdata", "data", "authoring_hub.db");
  if (!fs.existsSync(dbPath)) return null;
  try {
    const db = new DatabaseSync(dbPath, { readOnly: true });
    const row = db
      .prepare(
        "SELECT stage, local_status, cloud_status, reconcile_status, last_error_code, retry_count " +
          "FROM processing_jobs_v2 WHERE library_item_id = ? ORDER BY rowid DESC LIMIT 1"
      )
      .get(itemId);
    db.close();
    if (!row) return null;
    return {
      stage: row.stage,
      localStatus: row.local_status,
      cloudStatus: row.cloud_status,
      reconcileStatus: row.reconcile_status,
      lastErrorCode: row.last_error_code ?? null,
      retryCount: row.retry_count ?? null,
    };
  } catch (error) {
    return { readError: String(error?.message ?? error) };
  }
}

/** 等批次出现（用于 C 阶段）。 */
async function waitBatch(itemId, timeoutMs = 180000) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  while (Date.now() < deadline) {
    const r = await call("get_recognition_decision", { itemId });
    if (r?.ok && r.value) {
      last = r.value;
      if (last.batchId) return last;
    }
    await sleep(2000);
  }
  return last;
}

try {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);
  if (!fs.existsSync(fixturePath)) throw new CannotRunError(`夹具不存在：${fixturePath}`);
  report.identity.fixtureSha256 = sha256File(fixturePath);

  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  const staged = path.join(runDir, "pdfs", path.basename(fixturePath));
  fs.copyFileSync(fixturePath, staged);

  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    appEnv: isPdf ? {} : { PDF2TEST_AUTOMATION_SOURCE_FILES: staged },
  });
  report.identity.browserArgs = session.browserArgs;
  report.diagnosticRun = true;
  recorder = createStepRecorder({ session, artifactsDir: runDir });

  await recorder.run("library-page-loads-cloud-off", async () => {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, {
      timeoutMs: 40000,
      label: "library-page",
    });
    await session.evaluate(
      `(() => { window.localStorage.setItem("ielts-author-studio.app-settings.v1", JSON.stringify({ cloudEnabled: false })); location.hash = "#/library"; return true; })()`
    );
    await session.cdp.send("Page.reload", {}, 30000).catch(() => {});
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, {
      timeoutMs: 40000,
      label: "library-after-reload",
    });
    return { cloudEnabled: false };
  });

  let itemId = null;
  await recorder.run("A-import-without-cloud", async () => {
    const before = await session.evaluate(
      `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`
    );
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
    await session.clickSelector(isPdf ? '[data-testid="import-pick-folder"]' : '[data-testid="import-pick-files"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked-files" });
    await session.clickSelector('[data-testid="import-start"]');
    const deadline = Date.now() + 90000;
    while (Date.now() < deadline && !itemId) {
      const ids = await session.evaluate(
        `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`
      );
      itemId = (ids ?? []).find((id) => !(before ?? []).includes(id)) ?? null;
      if (!itemId) await sleep(1000);
    }
    if (!itemId) throw new Error("导入后未出现新的题库行");
    return { itemId, entry: isPdf ? "pick-folder" : "pick-files" };
  });

  let afterImport = null;
  await recorder.run("A-observe-no-batch-before-seed", async () => {
    const view = await waitLocalTerminal(itemId);
    if (!view) throw new Error("识别视图在超时前没有返回");
    afterImport = summarize(view);
    // 本步不断言「必须有批次」——恰恰相反，它记录**没有**批次，作为 H1 的前半段证据。
    if (afterImport.batchId) {
      throw new Error(
        `导入后已经出现批次（batchId=${afterImport.batchId}），H1 的前提不成立：` +
          "要么冻结与播种的顺序已被修好，要么本场景不需要修复。请复核后再解释本实验。"
      );
    }
    return { ...afterImport, expectedUnderH1: "batchId=null 且四链全 not_run" };
  });

  let canonicalAfterOpen = null;
  await recorder.run("B-open-workspace-seeds-canonical", async () => {
    const r = await call("get_workspace_item", { itemId });
    if (!r?.ok) throw new Error(`get_workspace_item 失败：${r?.error}`);
    const item = r.value?.item ?? {};
    const ds = r.value?.ds ?? null;
    canonicalAfterOpen = {
      hasCanonicalDs: Boolean(item.hasCanonicalDs),
      editVersion: item.editVersion ?? null,
      dsPresent: Boolean(ds && typeof ds === "object"),
    };
    // 打开工作区必须真的播种了权威稿，否则 C 阶段的前提不成立。
    if (!canonicalAfterOpen.hasCanonicalDs) {
      throw new Error(
        "打开工作区后权威稿仍未播种（hasCanonicalDs=false）——C 阶段前提不成立，无法检验 H1"
      );
    }
    return canonicalAfterOpen;
  });

  let afterRetry = null;
  await recorder.run("C-retry-after-seed-should-produce-batch", async () => {
    const retry = await call("retry_processing", { itemId });
    if (retry && retry.ok === false) throw new Error(`retry_processing 失败：${retry.error}`);
    const view = await waitBatch(itemId);
    if (!view) throw new Error("重试后识别视图在超时前没有返回");
    afterRetry = summarize(view);
    if (!afterRetry.batchId) {
      // **不能**把「重试后没有批次」直接读成「H1 不成立」。先看重试到底跑没跑起来。
      const jobFailure = readJobFailure(itemId);
      report.jobFailure = jobFailure;
      if (/editable_draft_exists/.test(jobFailure?.lastErrorCode ?? "")) {
        // 重试被产品自己的草稿保护挡下：C 阶段的前提根本没成立，
        // 本实验既不证实也不否证 H1 —— 如实报 inconclusive，别编一个「另有缺陷」的结论。
        report.inconclusive = true;
        throw new Error(
          `重试被产品自身的草稿保护挡下（last_error_code=${jobFailure.lastErrorCode}）：` +
            "C 阶段条件不成立，本实验**不确定**（既不证实也不否证 H1）。" +
            "要检验 H1，需让重试真正执行（例如允许覆盖已有草稿后重跑）。"
        );
      }
      throw new Error(
        `重试后仍无批次（chains=${JSON.stringify(afterRetry.states)}，` +
          `last_error_code=${jobFailure?.lastErrorCode ?? "unknown"}，` +
          `stage=${jobFailure?.stage ?? "unknown"}）。` +
          "H1 不成立：冻结失败并非唯一原因，另有缺陷阻断 reconcile。"
      );
    }
    if (afterRetry.allNotRun) {
      throw new Error("重试后出现批次但四链仍全 not_run，与 H1 预期不符。");
    }
    return afterRetry;
  });

  report.afterImport = afterImport;
  report.canonicalAfterOpen = canonicalAfterOpen;
  report.afterRetry = afterRetry;
  report.causalityEstablished =
    !afterImport.batchId && Boolean(afterRetry.batchId) && !afterRetry.allNotRun;
  // `causality` / `conclusion` **不在这里算**：结论要引用应用日志里的冻结失败行，
  // 而日志只有在 `finally` 里关掉应用之后才拿得到。算早了就会得出「日志里没有证据」
  // 这种自己造出来的结论。放到 finally 里、提取日志之后再算。
  report.verdict = report.steps.every((s) => s.status === "passed") ? "passed" : "failed";
} catch (error) {
  const cannotRun = error instanceof CannotRunError;
  report.verdict = cannotRun ? "cannot-run" : "failed";
  report.fatal = { name: error.name, message: String(error.message), appOutput: error.appOutput ?? null };
  console.error(`[freeze-order] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${error.message}`);
} finally {
  if (session) {
    if (!keep) await session.screenshot("final").catch(() => {});
    const closed = await session.close({ keep });
    report.appOutput = closed.appOutput?.slice(-4000) ?? null;
    // 冻结失败行要**单独提取**：它是 A 阶段「无批次」的机制证据。
    // 只看识别视图是看不到它的——四链全 `not_run` 既可能是「压根没跑」，也可能是
    // 「跑了但冻结失败被吞掉」，而这两件事对定位根因的意义完全不同。
    report.freezeFailures = String(closed.appOutput ?? "")
      .split(/\r?\n/)
      .filter((line) => /freeze local candidate snapshot failed|canonical_not_seeded/.test(line));
    report.freezeFailureObserved = report.freezeFailures.length > 0;
    report.exitCode = closed.exitCode;
  }
  report.finishedAt = new Date().toISOString();

  // ---- 结论（必须放在日志提取之后，见 try 块里的说明）----
  // 三态，而不是两态：**「没证实」与「被否证」是两件不同的事**。
  // C 阶段条件不成立时只能说「不确定」，不能顺势写成「H1 不成立」——那等于用一次
  // 没跑成的实验去否证一个假设。
  report.causality = report.causalityEstablished
    ? "established"
    : report.inconclusive
      ? "inconclusive"
      : "refuted";
  report.conclusion =
    report.causality === "established"
      ? "H1 成立：播种权威稿之前冻结快照必然失败，从而阻断无云路径的 reconcile。" +
        "修复方向：在 freeze_local_candidate_snapshot 之前调用 migrate_single_item（或等价的播种）。"
      : report.causality === "inconclusive"
        ? "本实验**不确定**：C 阶段的重试被产品自身的草稿保护挡下" +
          `（last_error_code=${report.jobFailure?.lastErrorCode ?? "unknown"}），条件不成立，` +
          "H1 既未被证实也未被否证。" +
          (report.freezeFailureObserved
            ? "不过 H1 的**机制前提有日志证据**：首次导入时冻结确实因权威稿未播种而失败" +
              `（${String(report.freezeFailures?.[0] ?? "").slice(0, 180)}）。` +
              "要让实验给出完整结论，需先让重试真正执行（否则无法验证「播种后即可恢复」这一半）。"
            : "本轮日志里**也没有**观察到冻结失败行，H1 的机制前提同样未获支持。")
        : "H1 被否证：权威稿已播种之后重试仍不产出批次，说明还有别的缺陷阻断 reconcile" +
          `（stage=${report.jobFailure?.stage ?? "unknown"}，` +
          `last_error_code=${report.jobFailure?.lastErrorCode ?? "unknown"}）。`;
  if (recorder) {
    report.steps = recorder.steps;
    const failed = report.steps.filter((s) => s.status === "failed");
    if (report.verdict !== "cannot-run") {
      report.verdict = failed.length || report.steps.length === 0 ? "failed" : "passed";
    }
    report.summary = { failed: failed.map((s) => s.name) };
  }
  const file = writeReport(runDir, report);
  console.log(`[freeze-order] verdict=${report.verdict} report=${file}`);
  console.log(`[freeze-order] steps: ${report.steps.map((s) => `${s.name}:${s.status}`).join(" | ")}`);
  console.log(
    `[freeze-order] afterImport.batchId=${report.afterImport?.batchId} allNotRun=${report.afterImport?.allNotRun}`
  );
  console.log(
    `[freeze-order] afterRetry.batchId=${report.afterRetry?.batchId} states=${JSON.stringify(report.afterRetry?.states)}`
  );
  console.log(`[freeze-order] causalityEstablished=${report.causalityEstablished}`);
  console.log(`[freeze-order] causality=${report.causality}`);
  console.log(
    `[freeze-order] freezeFailureObserved=${report.freezeFailureObserved} lines=${JSON.stringify(report.freezeFailures ?? [])}`
  );
  console.log(`[freeze-order] conclusion=${report.conclusion}`);
  process.exit(
    report.verdict === "passed"
      ? 0
      : report.verdict === "cannot-run"
        ? 3
        : report.causality === "inconclusive"
          ? 4
          : 1
  );
}
