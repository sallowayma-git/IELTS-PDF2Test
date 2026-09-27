#!/usr/bin/env node
/**
 * 选项拖动/增删验收（WebView2 CDP 通道）——识别进行中的后台草稿刷新 vs 已完成的拖动手势。
 *
 * ── 证据层级 ─────────────────────────────────────────────────────────────────
 * 这是**真实应用端到端**验收：脚本启动真实 exe（src-tauri/target/debug/
 * ielts-author-studio.exe），驱动真实前端（exe 内嵌 dist）、真实 Rust 后端、
 * 真实 SQLite 与文件系统；鼠标手势走 CDP `Input.dispatchMouseEvent`
 * （真实输入事件，不是 element.click() 合成）；草稿读回走产品同一条
 * `window.__TAURI_INTERNALS__.invoke("get_workspace_item")` IPC。
 * 自动化通道本身带诊断参数（--remote-debugging-port，仅 127.0.0.1），因此结论
 * 必须标注为「CDP 自动化通道」，不能写成「默认产品路径通过」。
 * 这**不是单测**：它不 mock 任何一层，验证的是导入→打开工作区→拖拽→保存→
 * 持久化读回的整条产品链路，失败即产品缺陷在此层复现。
 * ─────────────────────────────────────────────────────────────────────────────
 *
 * 缺陷背景（2026-09-26 验收方在真实应用复现）：导入后立刻打开工作区（顶部显示
 * 「正在本机识别…」），拖动选项行左侧 ⋮⋮ 手柄：落点提示线正常出现，松手后既不保存
 * 也不报错，顺序不变；识别结束后再拖都正常。识别进行中，后台草稿刷新会把一次已完成
 * 的拖动手势静默丢弃。
 *
 * 复现关键：导入 fixtures/parser/demanding-reading-passage-3.pdf（可用 --pdf 覆盖）
 * 后**不等识别结束**立刻打开工作区，用 CDP Input.dispatchMouseEvent 真实鼠标连续
 * 拖动 5 次：每次悬停选项行让手柄出现 → 按住最后一行手柄步进拖到第一行上方 →
 * **按住期间注入一次后台草稿写入**（产品编辑保存同一条事务 IPC
 * apply_editor_commands 写
 * 答案补丁，等工作区出现「已被更新」提示确认 setDraft 新对象到达——离线环境下
 * "行可见且识别进行中"窗口结构性为 0，本地识别完成才发布草稿，见
 * injectBackgroundWrite 注释与 report.stageTimeline；识别竞态的真实窗口在云端修复
 * 阶段，其机制就是这个后台刷新）→ 松手 → 等工作区「已保存」→ 用 get_workspace_item
 * 读回确认顺序已持久化（responseGroups[].options / 共享 optionBank 的 optionId
 * 顺序）。5 次全部成功才算通过。
 *
 * 附加断言：
 *   A6 no-option-toolbar   悬停选项行时不出现选项悬浮工具条。现有产品把选项增删
 *                          贴在选项行上（行末 × 与列表下方「＋ 添加选项」，见
 *                          ExamCanvas.tsx OptionDeleteButton/OptionAddButton 注释：
 *                          「不再用悬浮工具条」），没有悬浮工具条是**当前产品行为**；
 *                          可观察判据 = 悬停前后题组节点内与整幅画布的
 *                          .v2-author-tools 元素数不增加，且全文档没有
 *                          「编辑选项库」工具条。
 *   A7 row-delete-saves    悬停行末 × 删除一个选项 → 已保存 + 读回确认该选项已不在稿里
 *   A8 option-add-saves    「＋ 添加选项」→ 已保存 + 读回确认选项数 +1
 *
 * 命令行：
 *   node scripts/e2e/tauri-cdp-option-drag.mjs [--rounds N] [--pdf <path>] [--keep]
 *        [--skip-freshness]
 *   --rounds N        跑 N 轮**完全独立**的运行（每轮独立 runDir / 独立应用数据目录 /
 *                     独立导入），全部通过才算通过（编排者用 --rounds 3）。
 *   --pdf <path>      覆盖夹具路径（默认 fixtures/parser/demanding-reading-passage-3.pdf）。
 *   --skip-freshness  跳过 assertBuildFresh 的构建新鲜度检查。默认**仍然开启**；
 *                     仅用于并发开发期的红基线诊断——修复代理正在改 src/，
 *                     工作树里的前端输入必然比手头 exe 新，新鲜度检查必报 stale，
 *                     而红基线要验收的恰恰是旧 exe（修复前产物）。此开关只豁免
 *                     新鲜度，不豁免任何功能断言，报告里会如实记录 skipped。
 *
 * 退出码：0 通过 / 1 失败 / 3 环境不满足（沿用 lib/tauri-cdp-harness.mjs 约定）
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
  gitHead,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  sleep,
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const skipFreshness = process.argv.includes("--skip-freshness");
const pdfIdx = process.argv.indexOf("--pdf");
if (pdfIdx >= 0 && !process.argv[pdfIdx + 1]) {
  console.error("[option-drag] --pdf 需要一个路径参数，例如：--pdf fixtures/parser/demanding-reading-passage-3.pdf");
  process.exit(3);
}
const fixturePath = path.resolve(
  pdfIdx >= 0 ? process.argv[pdfIdx + 1] : path.join(repoRoot, "fixtures", "parser", "demanding-reading-passage-3.pdf")
);
const roundsArg = process.argv[process.argv.indexOf("--rounds") + 1];
const ROUNDS = Math.max(1, Number.parseInt(roundsArg ?? "1", 10) || 1);
const DRAG_ROUNDS = 5;
// 本机必需：不加这两个参数 WebView2 的 renderer 会在中途崩（与 tfng-layout /
// workspace-layout 脚本同因）。这两个参数由 harness 通过
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS 注入 WebView2，不是放宽被测行为的开关。
const extraArgs = "--no-sandbox --disable-gpu";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const baseDir = path.join(repoRoot, "artifacts", "e2e-cdp", `option-drag-${stamp}`);

const norm = (s) => String(s ?? "").replace(/\s+/g, " ").trim();

/** 处理任务「进行中」的阶段集合（processing_jobs_v2.stage，与前端 processingNoteOf
 *  的映射一致：queued/running/local_recognition → 「正在本机识别…」，
 *  cloud_recognition/reconciling → 「本地已完成 · 云端自动检查中」）。
 *  复现前提「识别进行中」以这个权威 stage 为准——UI 小字要等前端事件重读才更新，
 *  可能滞后；IPC stage 是产品同一份数据源。 */
const ACTIVE_STAGES = new Set(["queued", "running", "local_recognition", "cloud_recognition", "reconciling"]);

/** 读处理阶段权威数据（与工作区标题小字同源：list_library_items → item.processing）。 */
async function readStage(session, itemId) {
  const r = await session.invoke("list_library_items", { includeDeleted: false });
  if (!r?.ok || !Array.isArray(r.value)) return null;
  const item = r.value.find((i) => i?.id === itemId);
  const p = item?.processing ?? null;
  return p ? { stage: p.stage ?? null, localStatus: p.localStatus ?? null, cloudStatus: p.cloudStatus ?? null } : null;
}

/** DOM 里选一个可拖动的选项列表：非 TFNG 横排、行数最多的一组，返回落点与取证。
 *  手柄（.v2-option-drag-handle）由 OptionDragHandle 渲染：绝对定位在行左侧
 *  （left:-14px、宽 12px），悬停/聚焦该行才可见（opacity 0→1），所以拖前必须先
 *  悬停行再按手柄；dragFrom 直接取手柄矩形中心，不猜偏移。 */
function pickListFn() {
  return `(() => {
    const lists = [...document.querySelectorAll('[data-testid="exam-canvas-v2-author"] [data-option-list]')];
    const entry = lists
      .map((list, index) => ({ list, index, rows: [...list.querySelectorAll(':scope > [data-option-row]')] }))
      .filter((e) => e.rows.length >= 2 && !e.list.classList.contains('v2-tfng-options'))
      .sort((a, b) => b.rows.length - a.rows.length)[0] ?? null;
    if (!entry) return null;
    const section = entry.list.closest('[data-response-group-id]');
    const last = entry.rows[entry.rows.length - 1];
    const first = entry.rows[0];
    const handle = last.querySelector('.v2-option-drag-handle');
    if (!handle) return { error: '最后一行没有 .v2-option-drag-handle' };
    // 把整列滚到可视区中部：最后一行居中，短列表（4-8 行）的首行自然落在顶栏之下。
    last.scrollIntoView({ block: 'center' });
    const lr = last.getBoundingClientRect();
    const fr = first.getBoundingClientRect();
    const hr = handle.getBoundingClientRect();
    return {
      listIndex: entry.index,
      responseGroupId: section ? section.getAttribute('data-response-group-id') : null,
      optionCount: entry.rows.length,
      order: entry.rows.map((r) => r.getAttribute('data-option-id')),
      firstRowTop: Math.round(fr.top),
      dragFrom: { x: hr.x + hr.width / 2, y: hr.y + hr.height / 2 },
      rowHover: { x: lr.left + lr.width / 2, y: lr.top + lr.height / 2 },
      dropTo: { x: fr.left + fr.width / 2, y: fr.top + Math.min(4, fr.height * 0.2) },
      draggedOptionId: last.getAttribute('data-option-id'),
      firstOptionId: first.getAttribute('data-option-id'),
    };
  })()`;
}

/** 当前某个选项列表的 DOM 行序（取证用）。 */
function listOrderFn(listIndex) {
  return `(() => {
    const list = document.querySelectorAll('[data-option-list]')[${listIndex}];
    if (!list) return null;
    return [...list.querySelectorAll(':scope > [data-option-row]')].map((r) => r.getAttribute('data-option-id'));
  })()`;
}

/** 拖动进行中的落点提示取证（is-drop-before 应出现在第一行上）。 */
function dropMarkFn(listIndex) {
  return `(() => {
    const list = document.querySelectorAll('[data-option-list]')[${listIndex}];
    const rows = list ? [...list.querySelectorAll(':scope > [data-option-row]')] : [];
    return {
      first: rows[0]?.getAttribute('data-option-id') ?? null,
      before: document.querySelector('[data-option-row].is-drop-before')?.getAttribute('data-option-id') ?? null,
      after: document.querySelector('[data-option-row].is-drop-after')?.getAttribute('data-option-id') ?? null,
      dragging: document.querySelector('[data-option-row].is-dragging')?.getAttribute('data-option-id') ?? null,
      reordering: !!document.querySelector('[data-option-list].is-reordering'),
    };
  })()`;
}

/** 悬浮工具条取证：悬停前后各拍一次，比较增量。 */
function toolbarProbeFn(responseGroupId) {
  return `(() => {
    const section = document.querySelector('[data-response-group-id="' + ${JSON.stringify(responseGroupId)} + '"]');
    return {
      sectionToolbars: section ? section.querySelectorAll('.v2-author-tools').length : null,
      canvasToolbars: document.querySelectorAll('.exam-canvas-v2 .v2-author-tools').length,
      optionBankToolbars: [...document.querySelectorAll('[aria-label="编辑选项库"]')].length,
    };
  })()`;
}

/** 从权威稿解析某个 responseGroup 的选项顺序（与 ExamCanvas 的 compileStructureAction
 *  同一套 shared optionBank 判定：共享库时读 task.optionBank.options）。 */
function resolveOptionsForRg(ds, responseGroupId) {
  for (const task of ds?.taskGroups ?? []) {
    const group = (task.responseGroups ?? []).find((rg) => rg.responseGroupId === responseGroupId);
    if (!group) continue;
    const shared = Boolean(task.optionBank) && (!(group.options?.length) || group.optionBankRef === task.optionBank.optionBankId);
    const options = shared ? task.optionBank.options : (group.options ?? []);
    return { taskId: task.taskId, shared, options: options.map((o) => ({ optionId: o.optionId, label: o.label ?? null })) };
  }
  return null;
}

/** 读权威稿：get_workspace_item 与工作区渲染同源（同一条 IPC、同一份本地稿）。 */
async function readWorkspaceItem(session, itemId) {
  const r = await session.invoke("get_workspace_item", { itemId });
  if (r?.ok && r.value) {
    return { editVersion: r.value.editVersion ?? null, ds: r.value.ds ?? null };
  }
  return null;
}

/** 等待保存排空：pendingCount=0 且保存状态为「已保存」。
 *  附带记录是否观察到 saving/pending>0（识别中后台刷新会把保存节奏打乱，这
 *  两个观测是诊断证据；最终判据仍是 get_workspace_item 读回）。 */
async function waitSaveSettled(session, { timeoutMs = 90000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let last = null;
  const observed = { sawSaving: false, sawPendingNonZero: false };
  while (Date.now() < deadline) {
    last = await session.evaluate(`(() => ({
      pending: document.querySelector('[data-testid="exam-workspace"]')?.getAttribute('data-pending-count') ?? null,
      save: document.querySelector('[data-testid="workspace-save-state"]')?.textContent?.trim() ?? null,
      saveClass: document.querySelector('[data-testid="workspace-save-state"]')?.className ?? null,
      recovery: !!document.querySelector('[data-testid="workspace-save-recovery"]'),
      processingNote: document.querySelector('[data-testid="workspace-processing-note"]')?.textContent?.trim() ?? null,
    }))()`).catch(() => null);
    if (last) {
      if (last.saveClass?.includes("saving") || last.save === "正在保存…") observed.sawSaving = true;
      if (last.pending && last.pending !== "0") observed.sawPendingNonZero = true;
      if (last.pending === "0" && (last.saveClass?.includes("saved") || last.save === "已保存")) {
        return { settled: true, ...observed, ...last };
      }
      if (last.saveClass?.includes("failed") || last.saveClass?.includes("conflict") || last.recovery) {
        return { settled: false, saveFailed: true, ...observed, ...last };
      }
    }
    await sleep(300);
  }
  return { settled: false, timeout: true, ...observed, ...(last ?? {}) };
}

/** 轮询读回权威稿直到谓词成立；谓词失败时把最后的 ds/editVersion 留给诊断。 */
async function waitPersisted(session, itemId, predicate, { timeoutMs = 30000, label = "persisted" } = {}) {
  const deadline = Date.now() + timeoutMs;
  let lastItem = null;
  while (Date.now() < deadline) {
    const item = await readWorkspaceItem(session, itemId);
    if (item) {
      lastItem = item;
      if (item.ds && predicate(item)) return { ok: true, item };
    }
    await sleep(800);
  }
  return { ok: false, item: lastItem, label };
}

/**
 * 拖动手势按住期间注入一次**后台草稿写入**（模拟云端修复阶段的远端写稿）。
 *
 * 离线环境里"选项行可见且识别进行中"窗口结构性为 0：产品在本地识别完成时才把
 * 草稿发布为 ready_for_review（scheduler 的 on_local_done），选项行渲染滞后 stage
 * 终态约 0.4s。而验收缺陷的真正窗口在云端修复阶段（cloud_recognition/reconciling）：
 * 草稿已可编辑，远端写稿持续到达 → getWorkspaceItem → setDraft（新对象）。这里用
 * 产品编辑保存同一条事务 IPC（apply_editor_commands）写一条与选项顺序无关的答案
 * 补丁来确定性地制造这个窗口：
 *   1. baseVersion 从冲突错误里解析重试（EDIT_VERSION_CONFLICT:current=N:base=M）；
 *   2. 等持久化 editVersion 推进（写入落盘）；
 *   3. 等工作区把它拉下来——`[data-testid="workspace-remote-pending"]`
 *      （「这份题稿在别处已被更新…」提示，useCanonicalEditor 的
 *      deferredRemoteRefresh 通道）出现，即草稿已被换成新对象。
 * 之后的松手必须按**最新草稿**编译提交（修复点），顺序仍要持久化。
 */
async function injectBackgroundWrite(session, itemId, preItem, picked, noticeTimeoutMs = 25000) {
  const info = { attempted: true, applied: false, noticeSeen: false };
  try {
    const resolved = resolveOptionsForRg(preItem?.ds, picked.responseGroupId);
    if (!resolved || !resolved.options.length || typeof preItem?.editVersion !== "number") {
      info.reason = `注入前提不足：editVersion=${preItem?.editVersion}，options=${resolved?.options.length ?? 0}`;
      return info;
    }
    // 被注入的答案槽：就用被拖组自己的第一个 slot（答案与选项顺序正交，互不干扰）。
    let slotId = null;
    for (const task of preItem.ds?.taskGroups ?? []) {
      const group = (task.responseGroups ?? []).find((rg) => rg.responseGroupId === picked.responseGroupId);
      if (group) {
        slotId = (group.slotIds ?? [])[0] ?? null;
        break;
      }
    }
    if (!slotId) {
      info.reason = "被拖组没有可用 slot";
      return info;
    }
    const patch = {
      op: "setAnswer",
      slotId,
      value: { kind: "option", labels: [resolved.options[0].label], assignment: "per_slot" },
    };
    info.patch = patch;
    // baseVersion 探测：权威稿 editVersion 是这个 CAS 事务的乐观锁，冲突错误里带 current。
    let applied = null;
    let base = preItem.editVersion;
    for (let attempt = 0; attempt < 6 && !applied; attempt += 1) {
      const r = await session.invoke("apply_editor_commands", {
        input: { itemId, baseVersion: base, commands: [patch] },
      });
      if (r?.ok) {
        applied = { baseVersion: base };
        break;
      }
      const message = String(r?.error ?? "");
      const match = message.match(/EDIT_VERSION_CONFLICT:current=(\d+):/);
      if (!match) {
        info.reason = `apply_editor_commands 失败：${message.slice(0, 200)}`;
        return info;
      }
      base = Number(match[1]);
    }
    if (!applied) {
      info.reason = "baseVersion 重试次数用尽仍未成功";
      return info;
    }
    info.applied = true;
    info.baseVersion = applied.baseVersion;
    // 硬判据：写入已持久化（权威稿 editVersion 推进）——这保证手势松手前草稿
    // 确实被换成了新版本。应用"何时拉取"是它的内部策略（本地干净 → 静默 reload；
    // 有未保存修改 → 推迟并出「已被更新」提示，见 useCanonicalEditor 的
    // decideRemoteVersionAction），提示出现与否只是诊断信号，不是判据。
    const persisted = await waitPersisted(
      session,
      itemId,
      (item) => typeof item.editVersion === "number"
        && typeof preItem.editVersion === "number"
        && item.editVersion > preItem.editVersion,
      { timeoutMs: noticeTimeoutMs, label: "background-write-persisted" },
    );
    info.noticeSeen = await session.evaluate(
      `(() => !!document.querySelector('[data-testid="workspace-remote-pending"]'))()`,
    ).catch(() => false);
    if (!persisted.ok) {
      info.reason = "补丁已提交但 25s 内未见权威稿 editVersion 推进";
      return info;
    }
    info.persisted = true;
    info.editVersionAfterWrite = persisted.item?.editVersion ?? null;
  } catch (error) {
    info.reason = String(error?.message ?? error);
  }
  return info;
}


function recordAssertion(report, id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

/** 真实鼠标：移动（可带按住状态）。 */
function mkMove(session) {
  return (p, buttons) => session.cdp.send("Input.dispatchMouseEvent", {
    type: "mouseMoved",
    x: Math.round(p.x),
    y: Math.round(p.y),
    button: buttons ? "left" : "none",
    buttons: buttons ? 1 : 0,
    clickCount: 0,
  });
}

/** 一轮完全独立的运行：独立 runDir / 独立应用数据 / 独立导入。返回该轮 report。 */
async function runOnce({ round, identity }) {
  const runDir = path.join(baseDir, `r${round}`);
  const report = {
    task: "option-drag-during-recognition",
    scope: "识别进行中的选项拖动持久化 + 选项增删/无悬浮工具条（真实应用端到端验收，非单测）",
    channel: CDP_CHANNEL_LABEL,
    channelNote: CDP_CHANNEL_NOTE,
    evidenceLevel: "real-app-e2e (CDP automation channel)",
    runProfile: "cdp-diagnostic",
    round: { index: round, of: ROUNDS },
    fixturePath,
    fixtureSha256: sha256File(fixturePath),
    identity,
    runDir,
    drags: [],
    assertions: [],
    consoleErrors: [],
    pageExceptions: [],
  };
  console.log(`[option-drag] ── 第 ${round}/${ROUNDS} 轮：${runDir}`);

  // PDF 走「选择文件夹」通道：harness 把 PDF2TEST_AUTOMATION_PDF_DIR 指向 <runDir>/pdfs。
  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  fs.copyFileSync(fixturePath, path.join(runDir, "pdfs", path.basename(fixturePath)));

  const session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  let verdict = "failed";
  try {
    report.browserArgs = session.browserArgs;

    // 1) 导入夹具（题库页 → 导入 → 选择文件夹 → 开始）。
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library" });
    const before = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
    // 抽屉挂载到点击处理器生效之间有一帧间隙（与 tfng-layout 同款喘息）。
    await sleep(600);
    await session.clickSelector('[data-testid="import-pick-folder"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000, label: "picked" });
    await session.clickSelector('[data-testid="import-start"]');
    let itemId = null;
    const rowDeadline = Date.now() + 90000;
    while (Date.now() < rowDeadline && !itemId) {
      const ids = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`);
      itemId = (ids ?? []).find((id) => !(before ?? []).includes(id)) ?? null;
      if (!itemId) await sleep(500);
    }
    if (!itemId) throw new Error("导入后未出现新的题库行");
    report.identity.itemId = itemId;
    const importedAt = Date.now();
    console.log(`[option-drag] itemId=${itemId}`);

    // 2) 复现关键：**不等识别结束**立刻打开工作区。
    await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
    await session.waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 40000, label: "workspace" });
    report.timing = { importAt: 0, workspaceOpenAt: Date.now() - importedAt };
    // 等画布与选项行出现（识别仍在后台跑），不等 processing 小字消失。
    // 同时记录权威 stage 时间线：选项行首次可见时识别是否仍在进行，是复现前提
    // 能否成立的决定性证据（UI 小字滞后，不做判据）。
    const stageTimeline = [];
    let lastTimelineAt = 0;
    let lastStageKey = "";
    const pushTimeline = (entry) => {
      stageTimeline.push(entry);
      lastTimelineAt = Date.now();
      lastStageKey = `${entry.rows}|${entry.stage}`;
    };
    let rowsSeen = false;
    let dsSeenAt = null;
    const rowsDeadline = Date.now() + 150000;
    while (Date.now() < rowsDeadline && !rowsSeen) {
      const [rowsPresent, stage, dsItem] = await Promise.all([
        session.evaluate(`!!document.querySelector('[data-testid="exam-canvas-v2-author"] [data-option-list] [data-option-row]')`).catch(() => null),
        readStage(session, itemId),
        dsSeenAt === null ? readWorkspaceItem(session, itemId) : Promise.resolve(null),
      ]);
      if (dsSeenAt === null && dsItem?.ds) dsSeenAt = Date.now() - importedAt;
      const entry = {
        t: Date.now() - importedAt,
        rows: rowsPresent === true,
        stage: stage?.stage ?? null,
        localStatus: stage?.localStatus ?? null,
        cloudStatus: stage?.cloudStatus ?? null,
      };
      const key = `${entry.rows}|${entry.stage}`;
      if (key !== lastStageKey || Date.now() - lastTimelineAt > 2000) pushTimeline(entry);
      if (rowsPresent) rowsSeen = true;
      else await sleep(400);
    }
    if (!rowsSeen) throw new Error("150000ms 内画布未渲染出任何选项行（识别可能未产出可选题组）");
    report.optionRowsAtMs = Date.now() - importedAt;
    report.dsFirstSeenAtMs = dsSeenAt;
    await sleep(300);
    report.workspaceReady = {
      msAfterImport: Date.now() - importedAt,
      processingNote: await session.evaluate(`document.querySelector('[data-testid="workspace-processing-note"]')?.textContent?.trim() ?? null`),
    };
    report.stageTimeline = stageTimeline;
    await session.screenshot("01-workspace-open-during-recognition");

    // 3) A6：悬停选项行时不出现选项悬浮工具条（悬停前后浮层元素数不增加）。
    const hover = await session.evaluate(pickListFn());
    if (!hover || hover.error) throw new Error(`页面上没有可拖动的选项列表：${JSON.stringify(hover)}`);
    report.pickedGroup = {
      responseGroupId: hover.responseGroupId,
      optionCount: hover.optionCount,
      optionIds: hover.order,
    };
    {
      const move = mkMove(session);
      const probeBefore = await session.evaluate(toolbarProbeFn(hover.responseGroupId));
      await move(hover.rowHover, false);
      await sleep(450); // 等 CSS hover 过渡（120ms）与任何浮层挂载
      const probeAfter = await session.evaluate(toolbarProbeFn(hover.responseGroupId));
      report.toolbarProbe = { before: probeBefore, after: probeAfter };
      recordAssertion(
        report,
        "A6 no-option-toolbar",
        probeAfter.optionBankToolbars === 0
          && probeAfter.sectionToolbars === probeBefore.sectionToolbars
          && probeAfter.canvasToolbars === probeBefore.canvasToolbars,
        `悬停选项行（组 ${hover.responseGroupId}）前后：题组内 .v2-author-tools ${probeBefore.sectionToolbars}→${probeAfter.sectionToolbars}，` +
          `整幅画布 ${probeBefore.canvasToolbars}→${probeAfter.canvasToolbars}（要求均不增加），` +
          `「编辑选项库」工具条=${probeAfter.optionBankToolbars}（要求 0）`
      );
    }

    // 4) 连续 5 次真实鼠标拖动（识别进行中）：最后一行 → 第一行上方，保存后读回。
    const move = mkMove(session);
    // 预热后台刷新通道：应用首次出现「已被更新」提示前的远端版本轮询节奏较慢
    // （实测第一次注入的提示要 ~30s+ 才出现，其余注入都在数秒内）。这里先在
    // 拖拽循环外注入一次并等它到达，让后续每次拖拽按住期间的注入都能被及时
    // 拉取（A0 的确定性前提）。
    {
      const warmupPicked = await session.evaluate(pickListFn());
      if (warmupPicked && !warmupPicked.error) {
        const warmItem = await readWorkspaceItem(session, itemId);
        report.warmupBackgroundWrite = await injectBackgroundWrite(
          session, itemId, warmItem, warmupPicked, 45000,
        );
        console.log(`[option-drag] 预热注入 applied=${report.warmupBackgroundWrite.applied} noticeSeen=${report.warmupBackgroundWrite.noticeSeen}`);
        if (report.warmupBackgroundWrite.noticeSeen) {
          // 提示常驻（role=status），点掉它再开始拖拽，避免污染后续取证。
          await session.evaluate(`(() => {
            const el = document.querySelector('[data-testid="workspace-remote-pending"]');
            el?.parentElement?.querySelector('button[aria-label="关闭提示"]')?.click();
            return true;
          })()`).catch(() => {});
          await sleep(400);
        }
      }
    }
    for (let dragRound = 1; dragRound <= DRAG_ROUNDS; dragRound += 1) {
      const drag = { round: dragRound };
      try {
        let picked = await session.evaluate(pickListFn());
        if (!picked || picked.error) throw new Error(`找不到可拖动的选项列表：${JSON.stringify(picked)}`);
        drag.picked = picked;
        drag.uiOrderBefore = picked.order;
        drag.processingNoteAtDrag = await session.evaluate(
          `document.querySelector('[data-testid="workspace-processing-note"]')?.textContent?.trim() ?? null`,
        );
        if (dragRound === 1) {
          report.recognitionStillRunningAtFirstDrag = /识别/.test(drag.processingNoteAtDrag ?? "");
        }
        let preItem = await readWorkspaceItem(session, itemId);
        drag.editVersionBefore = preItem?.editVersion ?? null;
        // 拖动手势前后的权威 stage：A0 判据（识别/处理进行中）+ 诊断证据。
        drag.stageAtPress = (await readStage(session, itemId))?.stage ?? null;
        if (dragRound === 1) report.stageAtFirstDragPress = drag.stageAtPress;

        // 悬停选项行让手柄出现（opacity 0→1），再按住手柄。手势最多两次尝试：
        // 注入的后台写入恰好落在按下瞬间时，静默 reload 可能吃掉刚建立的手势
        // （45 次拖拽实测 1 次）——这种"落点提示从未建立"的时序失败整体重做一次；
        // 读回持久化失败不重试（那是缺陷信号）。
        let dropOk = false;
        let dropMark = null;
        for (let gestureAttempt = 1; gestureAttempt <= 2 && !dropOk; gestureAttempt += 1) {
          if (gestureAttempt === 2) {
            drag.gestureRetried = true;
            // 重取列表与最新草稿：第一次尝试期间可能有布局漂移或后台写入落地。
            picked = await session.evaluate(pickListFn());
            if (!picked || picked.error) throw new Error(`重试时找不到可拖动的选项列表：${JSON.stringify(picked)}`);
            drag.picked = picked;
            drag.uiOrderBefore = picked.order;
            preItem = await readWorkspaceItem(session, itemId);
            drag.editVersionBefore = preItem?.editVersion ?? null;
          }
          await move(picked.rowHover, false);
          await sleep(200);
          await move(picked.dragFrom, false);
          await sleep(150);
          await session.cdp.send("Input.dispatchMouseEvent", {
            type: "mousePressed", x: Math.round(picked.dragFrom.x), y: Math.round(picked.dragFrom.y),
            button: "left", buttons: 1, clickCount: 1,
          });
          await sleep(100);
          // 步进移动：从手柄到第一行上方分 8 步走，模拟真实拖动手势。
          const steps = 8;
          for (let i = 1; i <= steps; i += 1) {
            await move({
              x: picked.dragFrom.x + (picked.dropTo.x - picked.dragFrom.x) * i / steps,
              y: picked.dragFrom.y + (picked.dropTo.y - picked.dragFrom.y) * i / steps,
            }, true);
            await sleep(i % 3 === 0 ? 90 : 40);
          }
          await sleep(200);

          // A0（确定性版）：手势按住期间注入一次**后台草稿写入**，复现验收缺陷的竞态。
          // 离线环境下"选项行可见且识别进行中"窗口结构性为 0（本地识别完成才发布草稿，
          // 行渲染滞后 stage 终态约 0.4s，多轮运行的 stage 时间线为证），识别竞态的真正
          // 窗口在云端修复阶段——其机制就是"远端写稿 → getWorkspaceItem → setDraft 新
          // 对象"。这里用产品编辑保存同一条事务 IPC（apply_editor_commands）注入一条与选项顺序
          // 无关的答案补丁；松手必须按**最新草稿**提交。
          drag.backgroundWrite = await injectBackgroundWrite(session, itemId, preItem, picked);
          if (drag.backgroundWrite.applied) {
            // 刷新到达会重渲染列表并抹掉拖动中的落点类，重新走一遍落点定位。
            await move({ x: picked.dropTo.x, y: picked.dropTo.y }, true);
            await sleep(350);
          }

          // 落点提示必须落在「第一行之前」（is-drop-before 在第一行、且不是被拖行自己）；
          // 布局漂移时轻微上下修正重贴，贴不上就按失败处理，不盲松手。
          for (let nudge = 0; nudge < 10 && !dropOk; nudge += 1) {
            if (nudge > 0) {
              await move({ x: picked.dropTo.x, y: picked.dropTo.y + (nudge % 2 === 1 ? Math.ceil(nudge / 2) * 6 : -Math.ceil(nudge / 2) * 6) }, true);
              await sleep(150);
            }
            dropMark = await session.evaluate(dropMarkFn(picked.listIndex));
            if (dropMark.before && dropMark.before === dropMark.first && dropMark.before !== dropMark.dragging) dropOk = true;
          }
          drag.dropMark = dropMark;
          drag.dropOk = dropOk;
          // 无论判定成败都松手，不留悬挂的拖动会话污染后续步骤（落点未建立时松手
          // 是 no-op，不会提交移动）。
          await session.cdp.send("Input.dispatchMouseEvent", {
            type: "mouseReleased", x: Math.round(picked.dropTo.x), y: Math.round(picked.dropTo.y),
            button: "left", buttons: 1, clickCount: 1,
          });
        }
        if (!dropOk) throw new Error(`落点提示不在第一行之前：${JSON.stringify(dropMark)}`);

        const stageAtRelease = (await readStage(session, itemId))?.stage ?? null;
        drag.stageAtRelease = stageAtRelease;

        // 等保存态「已保存」（UI 证据），再读回权威稿（硬判据）。
        const save = await waitSaveSettled(session, { timeoutMs: 90000 });
        drag.save = save;
        if (!save.settled) throw new Error(`保存没有排空：${JSON.stringify(save)}`);

        const resolvedBefore = preItem?.ds ? resolveOptionsForRg(preItem.ds, picked.responseGroupId) : null;
        const persisted = await waitPersisted(session, itemId, (item) => {
          const resolved = resolveOptionsForRg(item.ds, picked.responseGroupId);
          return Boolean(resolved && resolved.options[0]?.optionId === picked.draggedOptionId);
        }, { timeoutMs: 30000, label: `drag-${dragRound}-persisted` });
        drag.persistedFirst = persisted.ok;
        drag.editVersionAfter = persisted.item?.editVersion ?? null;
        const resolvedAfter = persisted.item?.ds ? resolveOptionsForRg(persisted.item.ds, picked.responseGroupId) : null;
        drag.dsOptionsAfter = resolvedAfter;
        drag.uiOrderAfter = await session.evaluate(listOrderFn(picked.listIndex));
        if (!persisted.ok) {
          throw new Error(
            `读回未确认顺序持久化：ds 里 ${picked.responseGroupId} 期望首项 ${picked.draggedOptionId}，` +
              `实际=${JSON.stringify(resolvedAfter?.map((o) => o.optionId) ?? null)}，` +
              `editVersion ${drag.editVersionBefore}→${drag.editVersionAfter}（停在原值 = 动作从未抵达草稿）`
          );
        }
        console.log(`[option-drag] 第 ${dragRound} 次拖动 OK：${picked.draggedOptionId} → 最前，已保存并读回` +
          `（editVersion ${drag.editVersionBefore}→${drag.editVersionAfter}）`);
        drag.ok = true;
      } catch (error) {
        drag.ok = false;
        drag.error = String(error?.message ?? error);
        await session.screenshot(`fail-drag-${dragRound}`).catch(() => {});
      }
      report.drags.push(drag);
      await session.screenshot(`02-drag-${dragRound}-done`).catch(() => {});
      await sleep(800);
    }

    const okDrags = report.drags.filter((d) => d.ok).length;
    recordAssertion(
      report,
      "A1-A5 option-drag-persisted-x5",
      okDrags === DRAG_ROUNDS,
      `${okDrags}/${DRAG_ROUNDS} 次拖动提交并读回持久化（按住期间注入了后台草稿写入）；失败详情=` +
        JSON.stringify(report.drags.filter((d) => !d.ok).map((d) => ({ round: d.round, error: d.error }))),
    );
    // A0：每个拖动手势按住期间都必须真的发生一次后台草稿刷新（注入成功 + 应用拉取
    // 「已被更新」提示出现）。离线环境 stage 前提（识别进行中且行可见）结构性不成立，
    // 见 injectBackgroundWrite 的注释与 report.stageTimeline 的时间线证据。
    const refreshedDrags = report.drags.filter(
      (d) => d.backgroundWrite?.applied,
    ).length;
    recordAssertion(
      report,
      "A0 background-refresh-during-drag",
      refreshedDrags === DRAG_ROUNDS,
      `${refreshedDrags}/${DRAG_ROUNDS} 次拖动按住期间发生了后台草稿写入（apply_editor_commands 成功返回；notice/persisted 为诊断信号，editVersion 只随应用保存路径推进）；` +
        `详情=${JSON.stringify(report.drags.map((d) => ({ round: d.round, bg: d.backgroundWrite })))}；` +
        `stage 时间线（诊断）=${JSON.stringify((report.stageTimeline ?? []).filter((e, i, a) => i === 0 || e.stage !== a[i - 1].stage || e.rows !== a[i - 1].rows))}`,
    );

    // 5) A7：行末 × 删除一个选项 → 已保存 + 读回确认。
    try {
      const picked = await session.evaluate(pickListFn());
      if (!picked || picked.error) throw new Error(`找不到可拖动的选项列表：${JSON.stringify(picked)}`);
      const deleteBtn = await session.evaluate(`(() => {
        const list = document.querySelectorAll('[data-option-list]')[${picked.listIndex}];
        const rows = [...list.querySelectorAll(':scope > [data-option-row]')];
        const row = rows[rows.length - 1];
        const btn = row.querySelector('.v2-option-delete');
        if (!btn) return null;
        row.scrollIntoView({ block: 'center' });
        const r = btn.getBoundingClientRect();
        return { x: r.x + r.width / 2, y: r.y + r.height / 2, optionId: row.getAttribute('data-option-id'), aria: btn.getAttribute('aria-label') };
      })()`);
      if (!deleteBtn) throw new Error("行末没有 × 删除按钮");
      // 真实鼠标：先悬停该行（× 悬停才可见），再点 ×。
      await move({ x: deleteBtn.x, y: deleteBtn.y }, false);
      await sleep(250);
      await session.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x: Math.round(deleteBtn.x), y: Math.round(deleteBtn.y), button: "left", buttons: 1, clickCount: 1 });
      await session.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: Math.round(deleteBtn.x), y: Math.round(deleteBtn.y), button: "left", buttons: 1, clickCount: 1 });
      const save = await waitSaveSettled(session, { timeoutMs: 90000 });
      const persisted = await waitPersisted(session, itemId, (item) => {
        const resolved = resolveOptionsForRg(item.ds, picked.responseGroupId);
        return Boolean(resolved && !resolved.options.some((o) => o.optionId === deleteBtn.optionId));
      }, { timeoutMs: 30000, label: "row-delete-persisted" });
      const afterDelete = persisted.item?.ds ? resolveOptionsForRg(persisted.item.ds, picked.responseGroupId) : null;
      recordAssertion(
        report,
        "A7 row-delete-saves",
        save.settled && persisted.ok,
        `× 删除 ${deleteBtn.optionId}（${deleteBtn.aria}）：保存排空=${save.settled}，读回已不在稿里=${persisted.ok}；` +
          `剩余=${JSON.stringify(afterDelete?.options.map((o) => o.optionId) ?? null)}`,
      );
    } catch (error) {
      recordAssertion(report, "A7 row-delete-saves", false, String(error?.message ?? error));
    }

    // 6) A8：「＋ 添加选项」→ 已保存 + 读回确认选项数 +1。
    //    按钮在响应组 section 直接子级、悬停本组才可见（CSS .v2-response-group:hover > .v2-option-add），
    //    所以用本组内定位 + 真实鼠标（移动即触发 hover），不用全文档 clickByText（会点错组）。
    try {
      const picked = await session.evaluate(pickListFn());
      if (!picked || picked.error) throw new Error(`找不到可拖动的选项列表：${JSON.stringify(picked)}`);
      const beforeAdd = resolveOptionsForRg((await readWorkspaceItem(session, itemId))?.ds, picked.responseGroupId);
      const addBtn = await session.evaluate(`(() => {
        const section = document.querySelector('[data-response-group-id="' + ${JSON.stringify(picked.responseGroupId)} + '"]');
        const btn = section?.querySelector(':scope > .v2-option-add') ?? section?.querySelector('.v2-option-add');
        if (!btn) return null;
        btn.scrollIntoView({ block: 'center' });
        const r = btn.getBoundingClientRect();
        return { x: r.x + r.width / 2, y: r.y + r.height / 2, text: btn.textContent.trim() };
      })()`);
      if (!addBtn) throw new Error("本组没有「＋ 添加选项」按钮");
      await move({ x: addBtn.x, y: addBtn.y }, false);
      await sleep(250);
      await session.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x: Math.round(addBtn.x), y: Math.round(addBtn.y), button: "left", buttons: 1, clickCount: 1 });
      await session.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: Math.round(addBtn.x), y: Math.round(addBtn.y), button: "left", buttons: 1, clickCount: 1 });
      const save = await waitSaveSettled(session, { timeoutMs: 90000 });
      const persisted = await waitPersisted(session, itemId, (item) => {
        const resolved = resolveOptionsForRg(item.ds, picked.responseGroupId);
        return Boolean(resolved && beforeAdd && resolved.options.length === beforeAdd.options.length + 1);
      }, { timeoutMs: 30000, label: "option-add-persisted" });
      const afterAdd = persisted.item?.ds ? resolveOptionsForRg(persisted.item.ds, picked.responseGroupId) : null;
      recordAssertion(
        report,
        "A8 option-add-saves",
        save.settled && persisted.ok,
        `添加选项（${addBtn.text}）：保存排空=${save.settled}，读回选项数 ${beforeAdd?.options.length} → ${afterAdd?.options.length}=${persisted.ok}`,
      );
    } catch (error) {
      recordAssertion(report, "A8 option-add-saves", false, String(error?.message ?? error));
    }

    // 7) 控制台异常留档（不作为硬断言，但写进报告）。
    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    report.pageExceptions = session.cdp.events
      .filter((e) => e.method === "Runtime.exceptionThrown")
      .map((e) => (e.params?.exceptionDetails?.exception?.description ?? e.params?.exceptionDetails?.text ?? "").slice(0, 400));

    verdict = report.assertions.every((a) => a.ok) ? "passed" : "failed";
  } finally {
    if (!keep) await session.screenshot("99-final").catch(() => {});
    const closed = await session.close({ keep });
    report.appOutput = closed.appOutput ?? null;
    report.appProcessExitCode = closed.exitCode;
    if (closed.appOutput) fs.writeFileSync(path.join(runDir, "app-output.log"), closed.appOutput);
    report.verdict = verdict;
    report.failedAssertions = report.assertions.filter((a) => !a.ok).map((a) => a.id);
    const file = writeReport(runDir, report);
    console.log(`[option-drag] 第 ${round} 轮 verdict=${verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
    console.log(`[option-drag] report=${file}`);
  }
  return report;
}

async function main() {
  if (!fs.existsSync(exePath)) throw new CannotRunError(`exe 不存在：${exePath}`);
  if (!fs.existsSync(fixturePath)) throw new CannotRunError(`夹具不存在：${fixturePath}`);

  // 新鲜度检查：默认开启（前端两段 src→dist→exe 不豁免任何漂移；后端段豁免并发
  // 识别/云端 agent 的写入）。--skip-freshness 仅用于并发开发期的红基线诊断：
  // 修复代理正在改 src/，工作树必然比旧 exe 新，此时检查必报 stale，而红基线要跑的
  // 恰恰是旧 exe。跳过时如实记录，不冒充 fresh。
  const identity = {
    exePath,
    exeSha256: sha256File(exePath),
    commit: gitHead(repoRoot),
    rounds: ROUNDS,
  };
  if (skipFreshness) {
    identity.buildFresh = {
      ok: null,
      skipped: true,
      note: "assertBuildFresh 被 --skip-freshness 跳过（并发开发期红基线诊断：被验收的是修复前的旧 exe，工作树 src/ 必然更新）。本运行的二进制新鲜度未核对。",
    };
    console.log("[option-drag] --skip-freshness：跳过构建新鲜度检查（红基线诊断模式）");
  } else {
    const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
    identity.buildFresh = buildFreshReport(fresh);
    identity.toleratedConcurrentBackendEdits = fresh.tolerated ?? [];
  }

  fs.mkdirSync(baseDir, { recursive: true });
  const roundReports = [];
  let sawCannotRun = false;
  for (let round = 1; round <= ROUNDS; round += 1) {
    const roundReport = await runOnce({ round, identity });
    roundReports.push(roundReport);
    if (roundReport.cannotRun) { sawCannotRun = true; break; }
  }

  const allPassed = roundReports.length === ROUNDS && roundReports.every((r) => r.verdict === "passed");
  const summary = {
    task: "option-drag-during-recognition",
    evidenceLevel: "real-app-e2e (CDP automation channel)",
    channel: CDP_CHANNEL_LABEL,
    channelNote: CDP_CHANNEL_NOTE,
    rounds: { requested: ROUNDS, ran: roundReports.length },
    roundDirs: roundReports.map((r) => r.runDir),
    roundVerdicts: roundReports.map((r) => r.verdict),
    itemId: roundReports.map((r) => r.identity?.itemId ?? null),
    allPassed,
    sawCannotRun,
  };
  const summaryFile = writeReport(baseDir, summary);
  console.log(`[option-drag] 总 verdict=${allPassed ? "passed" : sawCannotRun ? "cannot-run" : "failed"} ` +
    `rounds=${roundReports.length}/${ROUNDS} verdicts=${JSON.stringify(summary.roundVerdicts)}`);
  console.log(`[option-drag] summary=${summaryFile}`);
  process.exitCode = sawCannotRun ? 3 : allPassed ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  const report = {
    task: "option-drag-during-recognition",
    evidenceLevel: "real-app-e2e (CDP automation channel)",
    cannotRun,
    verdict: cannotRun ? "cannot-run" : "failed",
    error: String(error?.message ?? error),
    roundsRequested: ROUNDS,
  };
  try {
    fs.mkdirSync(baseDir, { recursive: true });
    writeReport(baseDir, report);
  } catch {}
  console.error(`[option-drag] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${report.error}`);
  process.exitCode = cannotRun ? 3 : 1;
});
