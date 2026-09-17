#!/usr/bin/env node
/**
 * 识别决策「写路径」对真实后端的接线验证。
 *
 * 背景：上一轮前端发的是 `{ itemId, decisions[] }`，而后端要的是
 * `{ requestId, batchId, baseEditVersion, accept[], reject[] }`，且后端没有
 * `deny_unknown_fields` —— 于是决策被静默丢弃，前端读 `result.accepted.length` 抛错，
 * 表现为「写路径无声失效 + 假失败」。契约层的类型已经对齐（漂移检查 0 处破坏性不一致），
 * 本脚本把「对齐」这件事放到**真实 IPC + 真实后端**上再验一次。
 *
 * 重要限制（不得含糊）：本脚本**不能**验证「用户采用/保留后权威稿真的变了」。
 * 那需要真实的识别候选项，而当前所有夹具的 `get_recognition_decision` 都返回
 * 0 条可核对项（面板显示「识别还没有产出可核对的结果」），没有对象可决策。
 * 因此这里验证的是**请求形状被真实后端接受、以及后端的结构校验确实可达**，
 * 属于接线验证，不是用户流程验收。
 *
 * 用法：node scripts/e2e/tauri-cdp-recognition-write-path.mjs [--keep] [--fixture <path>]
 * 退出码：0 = 全部步骤通过；3 = CANNOT-RUN；1 = 步骤失败。
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
const fixtureIdx = process.argv.indexOf("--fixture");
const fixturePath = path.resolve(
  fixtureIdx >= 0 ? process.argv[fixtureIdx + 1] : path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf")
);
const isPdf = /\.pdf$/i.test(fixturePath);
// 与产品链一致：本沙箱下 renderer 需要这两个开关才能稳定，属诊断参数。
const extraArgs = "--no-sandbox --disable-gpu";
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `run-recog-write-${new Date().toISOString().replace(/[:.]/g, "-")}`);

const report = {
  task: "recognition-write-path-wiring",
  scope: "接线验证（非用户流程验收）：请求形状被真实后端接受 + 结构校验可达 + 撤销通道确实写入权威稿",
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
  verdict: "failed",
};

let session = null;
let recorder = null;

/** 调真实 IPC，返回 {ok, value|error}，便于断言错误码。 */
async function call(command, args) {
  // 两个命令的签名都是 `(input: Value, app: AppHandle)`，IPC 参数必须整体包在 `input` 键里。
  // 这一层漂移契约检查器看不到（它只比对结构体字段名），是本轮真实 IPC 调用才暴露出来的：
  // 不包 input 时后端报 `invalid args 'input' ... missing required key input`。
  const wrapped = command === "apply_recognition_decisions" || command === "apply_editor_commands";
  const r = await session.invoke(command, wrapped ? { input: args } : args);
  if (r && r.__noInvoke) throw new Error("invoke 不可用");
  return r;
}

try {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);

  // PDF 必须先进 runDir/pdfs：harness 把 PDF2TEST_AUTOMATION_PDF_DIR 指向那里，
  // 「选择文件夹」hook 只列该目录下的 PDF。不 staging 就会卡在 picked-files 永不出现。
  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  const stagedFixture = path.join(runDir, "pdfs", path.basename(fixturePath));
  fs.copyFileSync(fixturePath, stagedFixture);
  report.identity.stagedFixture = stagedFixture;

  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    appEnv: isPdf ? {} : { PDF2TEST_AUTOMATION_SOURCE_FILES: stagedFixture },
  });
  report.identity.browserArgs = session.browserArgs;
  report.diagnosticRun = true;
  recorder = createStepRecorder({ session, artifactsDir: runDir });

  // ---- 1. 题库页 ----
  await recorder.run("library-page-loads", async () => {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });
    // 这里**不**写 `cloudEnabled`，也**不**重载页面。
    // `AppSettingsV1` 根本没有 `cloudEnabled` 字段，写进去也读不到——那是一句会骗人的死代码；
    // 而 `Page.reload` 唯一的旧理由就是「让刚写的那行设置生效」，设置删了它只剩副作用：
    // 打断 CDP 会话（`tauri-cdp-issue-list.mjs` 上一轮就是被它打断的）。
    // 本脚本的「无云」由**数据目录机制**保证：harness 每次用全新的
    // `PDF2TEST_AUTOMATION_DATA_DIR`，里面没有 `config/llm-profiles.json`，
    // `listLlmProfiles` 因此只回一个 `profile-local-placeholder`。
    await session.evaluate(`(() => { location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-after-hash" });
    return { url: await session.evaluate("location.href") };
  });

  // ---- 2. 导入夹具，拿到真实 itemId ----
  let itemId = null;
  await recorder.run("import-fixture", async () => {
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
    return { itemId, fixture: path.basename(fixturePath) };
  });

  // ---- 3. 读路径：真实后端返回的决策视图 ----
  let batchId = null;
  let editVersion = 0;
  await recorder.run("read-recognition-decision", async () => {
    // ⚠️ 不能把 `view.chains` 当「识别已落盘」的判据：`get_recognition_decision` 在**尚未**
    // 产生批次时也会返回一个视图，其 `chains` 是四条 `not_run`（后端 `load_latest_batch`
    // 为 None 的分支）。用 `|| chains` 会让循环立刻退出，把「还没开始」当成「已落盘」。
    // 正确判据是**批次出现**或**本地链进入终态**。
    const deadline = Date.now() + 120000;
    let view = null;
    while (Date.now() < deadline) {
      const r = await call("get_recognition_decision", { itemId });
      if (r?.ok && r.value) {
        view = r.value;
        const localState = view.chains?.local?.state ?? null;
        const terminalLocal = ["succeeded", "partial", "unusable", "failed", "canceled"].includes(localState);
        if (view.batchId || terminalLocal) break;
      }
      await sleep(2000);
    }
    if (!view) throw new Error("get_recognition_decision 没有返回视图");
    batchId = view.batchId ?? null;
    editVersion = Number(view.editVersion ?? 0);
    if (/undefined/.test(JSON.stringify(view))) throw new Error(`决策视图出现 undefined：${JSON.stringify(view).slice(0, 400)}`);
    const actionable = Array.isArray(view.actionable) ? view.actionable : [];
    const autoApplied = Array.isArray(view.autoApplied) ? view.autoApplied : [];
    return {
      batchId,
      editVersion,
      baseEditVersion: view.baseEditVersion ?? null,
      chainStates: view.chains ? Object.fromEntries(Object.entries(view.chains).map(([k, v]) => [k, v?.state ?? null])) : null,
      actionableCount: actionable.length,
      autoAppliedCount: autoApplied.length,
      // 如实记录：0 条候选项意味着「采用/保留」没有对象可决策，写路径的用户流程无法验收。
      hasRealCandidates: actionable.length > 0,
      note: actionable.length === 0
        ? "无真实候选项：写路径只能验证接线（形状被接受 + 结构校验可达），不能验证权威稿写入"
        : "存在真实候选项"
    };
  });

  // ---- 4. 旧格式（漂移格式）必须被显式拒绝，不能再静默失效 ----
  await recorder.run("write-path-rejects-legacy-shape", async () => {
    // 这正是上一轮前端实际发出的形状。后端 request_id/batch_id/base_edit_version 无默认值，
    // 因此必须在反序列化阶段就被拒绝；只要它还能「成功返回」，静默失效就回来了。
    const r = await call("apply_recognition_decisions", { itemId, decisions: [{ decisionId: "d-1", action: "accept" }] });
    const error = String(r?.error ?? "");
    if (r?.ok) throw new Error(`旧格式竟然被接受：${JSON.stringify(r.value).slice(0, 300)}`);
    if (!/recognition_invalid_input/.test(error)) throw new Error(`旧格式未按预期被拒：${error}`);
    return { rejectedWith: error.slice(0, 200) };
  });

  // ---- 5. 空决策必须被拒（后端注释明确：空请求与发错字段名都会落到这里）----
  await recorder.run("write-path-rejects-empty-decisions", async () => {
    const r = await call("apply_recognition_decisions", {
      requestId: `e2e-empty-${Date.now()}`,
      batchId: batchId ?? "batch-missing",
      baseEditVersion: editVersion,
      accept: [],
      reject: []
    });
    const error = String(r?.error ?? "");
    if (r?.ok) throw new Error("空决策被接受");
    if (!/RECOGNITION_NO_DECISIONS/.test(error)) throw new Error(`未返回 RECOGNITION_NO_DECISIONS：${error}`);
    return { rejectedWith: error };
  });

  // ---- 6. 同一 id 既接受又拒绝必须整批拒绝 ----
  await recorder.run("write-path-rejects-conflict", async () => {
    const r = await call("apply_recognition_decisions", {
      requestId: `e2e-conflict-${Date.now()}`,
      batchId: batchId ?? "batch-missing",
      baseEditVersion: editVersion,
      accept: ["d-same"],
      reject: ["d-same"]
    });
    const error = String(r?.error ?? "");
    if (r?.ok) throw new Error("冲突决策被接受");
    if (!/RECOGNITION_DECISION_CONFLICT/.test(error)) throw new Error(`未返回 RECOGNITION_DECISION_CONFLICT：${error}`);
    return { rejectedWith: error };
  });

  // ---- 7. 形状合法 + 批次不存在 → 证明形状已通过反序列化与结构校验 ----
  await recorder.run("write-path-shape-accepted", async () => {
    const r = await call("apply_recognition_decisions", {
      requestId: `e2e-shape-${Date.now()}`,
      batchId: `nonexistent-batch-${Date.now()}`,
      baseEditVersion: editVersion,
      accept: ["d-not-real"],
      reject: []
    });
    const error = String(r?.error ?? "");
    if (r?.ok) throw new Error("不存在的批次竟然成功");
    // 能走到批次查询，说明请求已经通过 deserialize + validate：
    // 这是在没有真实候选项的前提下，对「前端形状与后端一致」最强的证明。
    if (!/RECOGNITION_BATCH_NOT_FOUND/.test(error)) {
      throw new Error(`期望 RECOGNITION_BATCH_NOT_FOUND（说明形状已通过校验），实际：${error}`);
    }
    return { reachedBatchLookup: true, rejectedWith: error };
  });

  // ---- 8. 空 requestId / 空 batchId / 负版本 必须被拒 ----
  await recorder.run("write-path-rejects-invalid-identity", async () => {
    const cases = [
      { name: "empty-request-id", input: { requestId: "  ", batchId: "b", baseEditVersion: 0, accept: ["d"], reject: [] }, expect: "RECOGNITION_REQUEST_ID_EMPTY" },
      { name: "empty-batch-id", input: { requestId: "r", batchId: "", baseEditVersion: 0, accept: ["d"], reject: [] }, expect: "RECOGNITION_BATCH_ID_EMPTY" },
      { name: "negative-version", input: { requestId: "r", batchId: "b", baseEditVersion: -1, accept: ["d"], reject: [] }, expect: "RECOGNITION_BASE_VERSION_INVALID" }
    ];
    const results = [];
    for (const c of cases) {
      const r = await call("apply_recognition_decisions", c.input);
      const error = String(r?.error ?? "");
      if (r?.ok) throw new Error(`${c.name} 被接受`);
      if (!error.includes(c.expect)) throw new Error(`${c.name} 期望 ${c.expect}，实际 ${error}`);
      results.push({ case: c.name, rejectedWith: error });
    }
    return { results };
  });

  // ---- 9. 撤销通道：setAnswer 补丁走编辑器事务，真的改权威稿并递增版本 ----
  // 「撤销自动修正」此前被实现成 reject 决策，而后端拒绝分支只改状态、不碰权威稿
  // （`reconcile/commands.rs` 的拒绝分支消息就是「已拒绝该建议，权威稿未改动。」），
  // 于是界面说「已保持现状」而自动修正仍留在稿里 —— 假完成。
  // 现在撤销改为把 `undo` 当编辑器命令提交，本步验证这条通道在真实后端上确实写入。
  await recorder.run("undo-channel-writes-canonical", async () => {
    const readWorkspace = async () => {
      const r = await call("get_workspace_item", { itemId });
      if (!r?.ok) throw new Error(`get_workspace_item 失败：${r?.error}`);
      return r.value;
    };

    // 本地稿是**异步**生成的：`ds` 要等本地识别落盘才非空（契约 §6.1 第 1 条）。
    // 这里必须等，不能拿一个刚导入的空 `ds` 去断言——那会把「还没生成」误报成缺陷。
    const deadline = Date.now() + 120000;
    let before = null;
    while (Date.now() < deadline) {
      const candidate = await readWorkspace();
      if (candidate?.ds && typeof candidate.ds === "object") { before = candidate; break; }
      await sleep(2000);
    }
    if (!before) throw new Error("本地稿迟迟没有生成（get_workspace_item 的 ds 一直为空），无法验证撤销通道");

    const ds = before.ds;
    const slots = Object.keys(ds.answerSlots ?? {});
    if (!slots.length) throw new Error("题稿里没有答案位，无法验证撤销通道");
    // 优先挑「已有答案」的槽位：真实撤销就是把自动修正写进去的值改回修正前的值
    // （`undo_patch_for` 回退到 local_value，通常是 unresolved）。
    const slotId = slots.find((id) => {
      const kind = ds.answerKey?.[id]?.kind;
      return kind === "text" || kind === "option";
    }) ?? slots[0];
    const original = ds.answerKey?.[slotId] ?? { kind: "unresolved" };
    const beforeVersion = Number(before.editVersion ?? 0);
    // 写入值必须**与当前值不同**，否则「改到了权威稿」会退化成空断言：
    // 原值本来就等于目标值时，读回来相等什么也证明不了。
    const undoValue = original?.kind === "unresolved"
      ? { kind: "text", values: ["E2E-UNDO-PROBE"] }
      : { kind: "unresolved" };
    if (JSON.stringify(undoValue) === JSON.stringify(original)) {
      throw new Error("探针值与原值相同，断言会变成空断言，拒绝执行");
    }

    // 1) 写入「撤销值」：与后端 undo_patch_for 产出的形状一致。
    const undoPatch = { op: "setAnswer", slotId, value: undoValue };
    const applied = await call("apply_editor_commands", {
      itemId,
      baseVersion: beforeVersion,
      requestId: `e2e-undo-${Date.now()}`,
      commands: [undoPatch],
    });
    if (!applied?.ok) throw new Error(`撤销补丁未被接受：${applied?.error}`);
    const afterUndo = await readWorkspace();
    if (afterUndo.editVersion <= beforeVersion) {
      throw new Error(`撤销补丁没有递增版本：${beforeVersion} → ${afterUndo.editVersion}`);
    }
    const valueAfterUndo = afterUndo.ds?.answerKey?.[slotId];
    if (JSON.stringify(valueAfterUndo) !== JSON.stringify(undoValue)) {
      throw new Error(`撤销补丁没有把值写进权威稿：期望 ${JSON.stringify(undoValue)}，实际 ${JSON.stringify(valueAfterUndo)}`);
    }
    if (JSON.stringify(valueAfterUndo) === JSON.stringify(original)) {
      throw new Error("撤销后读回的值与原值相同，无法证明写入生效");
    }

    // 2) 还原原值，让本次运行不在题稿里留残留（同时证明还原方向也走得通）。
    const restore = await call("apply_editor_commands", {
      itemId,
      baseVersion: afterUndo.editVersion,
      requestId: `e2e-undo-restore-${Date.now()}`,
      commands: [{ op: "setAnswer", slotId, value: original }],
    });
    if (!restore?.ok) throw new Error(`还原补丁未被接受：${restore?.error}`);
    const afterRestore = await readWorkspace();
    if (JSON.stringify(afterRestore.ds?.answerKey?.[slotId]) !== JSON.stringify(original)) {
      throw new Error(`还原后答案位与原始值不一致：${JSON.stringify(afterRestore.ds?.answerKey?.[slotId])} ≠ ${JSON.stringify(original)}`);
    }

    return {
      slotId,
      originalValue: original,
      probeValue: undoValue,
      versionBefore: beforeVersion,
      versionAfterUndo: afterUndo.editVersion,
      versionAfterRestore: afterRestore.editVersion,
      canonicalValueAfterUndo: valueAfterUndo,
      valueActuallyChanged: JSON.stringify(valueAfterUndo) !== JSON.stringify(original),
      restored: true,
      note: "证明撤销通道（setAnswer 补丁经 apply_editor_commands 事务）在真实后端上会写入权威稿并递增版本；但本仓没有真实 auto_fixed 候选项，面板上「撤销」按钮的端到端点击仍未验收",
    };
  });

  report.verdict = report.steps.every((s) => s.status === "passed") ? "passed" : "failed";} catch (error) {
  const cannotRun = error instanceof CannotRunError;
  report.verdict = cannotRun ? "cannot-run" : "failed";
  report.fatal = { name: error.name, message: String(error.message), appOutput: error.appOutput ?? null };
  console.error(`[recog-write] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${error.message}`);
} finally {
  if (session) {
    if (!keep) await session.screenshot("final").catch(() => {});
    const closed = await session.close({ keep });
    report.appOutput = closed.appOutput?.slice(-4000) ?? null;
    report.exitCode = closed.exitCode;
    report.screenshotErrors = session.screenshotErrors;
  }
  report.finishedAt = new Date().toISOString();
  if (recorder) {
    report.steps = recorder.steps;
    const failed = report.steps.filter((s) => s.status === "failed");
    if (report.verdict !== "cannot-run") {
      report.verdict = failed.length || report.steps.length === 0 ? "failed" : "passed";
    }
    report.summary = { failed: failed.map((s) => s.name) };
  }
  const file = writeReport(runDir, report);
  console.log(`[recog-write] verdict=${report.verdict} report=${file}`);
  console.log(`[recog-write] steps: ${report.steps.map((s) => `${s.name}:${s.status}`).join(" | ")}`);
  process.exit(report.verdict === "passed" ? 0 : report.verdict === "cannot-run" ? 3 : 1);
}
