#!/usr/bin/env node
// cloud-direct P1/P2 真实产品验收（CDP 通道）。
//
// 证据层级：product（真实 exe + WebView2 + SQLite + 文件系统 + 真实 DeepSeek）。
// 范围：
//   在线轮（pipelineMode=direct，默认）：导入 5 份真实 PDF → 学生预览无阻断 →
//     发布 NAS 包 → 学生端加载器契约探针；记录正文覆盖率、题数、与 IELTS Atlas V1
//     answerKey 的答案一致率、云端调用次数、耗时、token。
//   离线轮（关闭云端配置）：同样 5 份 → 产出可编辑题稿（质量不作硬性要求）。
//
// 与用户环境隔离：独立 PDF2TEST_AUTOMATION_DATA_DIR；DeepSeek profile 从用户真实
// 配置复制（同一 profileId，密钥仍在 OS keyring，本脚本不读取、不打印 key），
// 仅把超时调为直出默认 240s。

import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { spawn } from "node:child_process";
import {
  assertBuildFresh,
  buildFreshReport,
  launchTauriAppCdp,
  repoRoot,
  sleep
} from "./lib/tauri-cdp-harness.mjs";

const release = process.argv.includes("--release");
const exePath = path.join(repoRoot, `src-tauri/target/${release ? "release" : "debug"}/ielts-author-studio.exe`);
const userConfig = path.join("C:", "Users", "25788", "AppData", "Roaming", "com.ielts.author.studio", "config", "llm-profiles.json");
const atlasDir = path.join("F:", "workspace", "IELTS Atlas", "assets", "generated", "reading-exams");
const pdfDir = path.join("F:", "workspace", "IELTS Atlas", "ReadingPractice", "PDF");
const PDFS = [
  "24. P1 - The Pearls 珍珠.pdf",
  "107. P2 - A study of western celebrity 西方名人.pdf",
  "100. P3 - What makes a musical expert_ 音乐天赋.pdf",
  "10. P1 - Maori Fish Hooks 毛利鱼钩.pdf",
  "1. P1 - A Brief History of Tea 茶叶简史.pdf"
];
// Default acceptance uses the product sandbox. Restricted-session diagnostics
// must be explicitly requested and are supplemental evidence only.
const sessionNoSandbox = process.argv.includes("--session-no-sandbox")
  || process.env.PDF2TEST_SESSION_NO_SANDBOX === "1";
const SESSION_BROWSER_ARGS = sessionNoSandbox ? "--no-sandbox --disable-gpu-sandbox" : "";

const runStamp = new Date().toISOString().replace(/[:.]/g, "-");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `run-cloud-direct-${runStamp}`);
const report = {
  task: "cloud-direct-p1-p2-acceptance",
  channel: "WebView2 CDP（自动化通道）",
  sessionBrowserArgs: SESSION_BROWSER_ARGS,
  diagnosticRun: sessionNoSandbox,
  sessionBrowserArgsReason: sessionNoSandbox
    ? "显式开启受限会话诊断参数；仅作补充证据，非默认产品沙箱"
    : "默认产品沙箱；未启用 no-sandbox",
  exePath, buildMode: release ? "release" : "debug", runDir,
  pdfs: PDFS, rounds: [], verdict: "failed"
};
// exe 构建标识：报告读者必须能看出「跑的是哪一版」。过期/漂移时如实记录错误，
// 不因此中止验收（标识是证据的一部分，不是前置条件）。
try {
  report.identity = { buildFresh: buildFreshReport(assertBuildFresh({ exePath })) };
} catch (error) {
  report.identity = { buildFresh: { ok: false, error: String(error) } };
}

// ── 启动前断言：exe 必须是**产品构建**（内嵌 dist），而不是去加载 devUrl ──────
//
// 成因（review-2 §2）：`Cargo.toml` 的 tauri features 没有 `custom-protocol`，
// 只有 `tauri build` 会自动加 `--features tauri/custom-protocol` 并嵌入 dist；
// `cargo build --release` 出来的 exe 会去加载 `http://localhost:1420/`（devUrl），
// 端口上没有任何服务 ⇒ 页面永远起不来。修前那次「环境阻断」就是这个原因。
//
// 探针：临时起一次应用 + CDP，读 page target 的 URL，然后关掉。
async function probePageUrl(exe) {
  const port = 51000 + Math.floor(Math.random() * 2000);
  const dir = path.join(runDir, "preflight");
  fs.mkdirSync(path.join(dir, "webview"), { recursive: true });
  const child = spawn(exe, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-allow-origins=* ${SESSION_BROWSER_ARGS}`,
      WEBVIEW2_USER_DATA_FOLDER: path.join(dir, "webview"),
      PDF2TEST_AUTOMATION_DATA_DIR: path.join(dir, "data")
    },
    stdio: "ignore"
  });
  let url = null;
  let targets = [];
  const deadline = Date.now() + 40000;
  try {
    while (Date.now() < deadline) {
      try {
        const response = await fetch(`http://127.0.0.1:${port}/json/list`);
        if (response.ok) {
          targets = await response.json();
          // `about:blank` 是导航前的初始页：它既不是 devUrl 也不是产品页，
          // 不能拿它当结论，继续等到真实 URL 出现。
          const page = targets.find(
            target => target.type === "page" && target.url && target.url !== "about:blank"
          );
          if (page) { url = page.url; break; }
        }
      } catch { /* 端点还没起来 */ }
      await sleep(500);
    }
  } finally {
    try { child.kill(); } catch { /* 已退出 */ }
  }
  return { url, targetCount: targets.length };
}

function check(name, ok, detail) {
  report.rounds[report.rounds.length - 1].assertions.push({ name, ok: Boolean(ok), detail });
  console.log(`[${report.rounds.length === 1 ? "online" : "offline"}] ${ok ? "PASS" : "FAIL"} ${name}${ok ? "" : " :: " + JSON.stringify(detail)}`);
  return Boolean(ok);
}

// ── DeepSeek profile：从用户真实配置复制，仅调超时为直出默认值 ──────────────
function seedProfiles(targetDir, { offline }) {
  fs.mkdirSync(path.join(targetDir, "config"), { recursive: true });
  const profiles = JSON.parse(fs.readFileSync(userConfig, "utf8"));
  for (const profile of profiles) {
    if (profile.baseUrl?.includes("deepseek.com")) {
      profile.timeoutMs = 240000;
      if (offline) profile.enabled = false;
    }
    if (offline) profile.enabled = false;
  }
  fs.writeFileSync(path.join(targetDir, "config", "llm-profiles.json"), JSON.stringify(profiles, null, 2));
}

// ── IELTS Atlas V1 answerKey（用 manifest 的 filename 对应 PDF）────────────
function atlasAnswerKeys() {
  const manifest = fs.readFileSync(path.join(atlasDir, "manifest.js"), "utf8");
  const entries = [...manifest.matchAll(/"([^"]+)":\s*\{\s*"examId":\s*"([^"]+)"[\s\S]*?"filename":\s*"([^"]+)"/g)]
    .map(match => ({ dataKey: match[1], examId: match[2], filename: match[3] }));
  const normalize = value => value.replace(/【[^】]*】/g, "").replace(/\s+/g, " ").trim();
  const keys = new Map();
  for (const pdf of PDFS) {
    const stem = normalize(path.basename(pdf, ".pdf"));
    const entry = entries.find(candidate => normalize(path.basename(candidate.filename, ".pdf")).startsWith(stem)
      || stem.startsWith(normalize(path.basename(candidate.filename, ".pdf"))));
    if (!entry) { keys.set(pdf, null); continue; }
    const text = fs.readFileSync(path.join(atlasDir, `${entry.dataKey}.js`), "utf8");
    const match = /"answerKey":\s*(\{[\s\S]*?\n\s*\})/.exec(text);
    keys.set(pdf, match ? { examId: entry.examId, answerKey: JSON.parse(match[1]) } : null);
  }
  return keys;
}

// 答案一致率：**分母是 Atlas 印出的全部题号**（不是稿子里有的题号）——
// 模型漏掉一个题号必须拉低一致率，不能因为「稿里没这道题」就不计分。
function answerConsistency(document, atlas) {
  if (!atlas?.answerKey) return { total: 0, matched: 0, missing: 0, extra: 0, rate: null, details: [] };
  const norm = text => String(text).trim().toLowerCase().replace(/\s+/g, " ");
  const documentKey = document.answerKey ?? {};
  const actualIds = Object.keys(documentKey);
  const details = [];
  let matched = 0;
  let missing = 0;
  // 题号口径对齐：Atlas V1 的 answerKey 用**本卷相对题号**（q1..qN），而稿件用
  // **卷面题号**（P2 是 14..26、P3 是 27..40）。不做对齐会把 P2/P3 全部误报成 0%。
  const docNumbers = actualIds
    .map(id => Number.parseInt(String(id).replace(/^q/i, ""), 10))
    .filter(number => Number.isFinite(number))
    .sort((a, b) => a - b);
  const offset = docNumbers.length ? docNumbers[0] - 1 : 0;
  const expectedIds = Object.keys(atlas.answerKey);
  for (const slotId of expectedIds) {
    const index = Number.parseInt(String(slotId).replace(/^q/i, ""), 10);
    const documentSlotId = Number.isFinite(index) ? `q${index + offset}` : slotId;
    const expected = atlas.answerKey[slotId];
    const value = documentKey[documentSlotId];
    const actual = value?.kind === "option" ? (value.labels?.[0] ?? "")
      : value?.kind === "text" ? (value.values?.[0] ?? "") : "";
    const absent = value === undefined;
    if (absent) missing += 1;
    const ok = !absent && norm(actual) === norm(expected);
    if (ok) matched += 1;
    details.push({ slotId: documentSlotId, atlasSlotId: slotId, expected, actual: absent ? null : actual, absent, ok });
  }
  const extraIds = actualIds.filter(slotId => !details.some(detail => detail.slotId === slotId));
  const total = expectedIds.length;
  return {
    total,
    matched,
    missing,
    extra: extraIds.length,
    extraIds,
    questionNumberOffset: offset,
    documentSlots: actualIds.length,
    rate: total ? matched / total : null,
    details
  };
}

function readJobDb(appRoot) {
  const db = new DatabaseSync(path.join(appRoot, "authoring_hub.db"), { readOnly: true });
  // 短超时：应用若长时间持有写锁，读取最多等 2 秒就抛错——由调用方按「这一拍读不到」
  // 处理并继续轮询，绝不让整轮验收挂死（修前实测：作业卡住时脚本自己也挂住，
  // 报告一个字都写不出来）。
  db.exec("PRAGMA busy_timeout=2000");
  return db;
}

/** 读取作业行；读不到（锁/表缺失）返回 null，不抛。 */
function readJobRow(appRoot, itemId) {
  let db = null;
  try {
    db = readJobDb(appRoot);
    return db.prepare("SELECT stage,cloud_status,local_status,lease_owner,created_at,updated_at FROM processing_jobs_v2 WHERE id=?").get(itemId) ?? null;
  } catch {
    return null;
  } finally {
    try { db?.close(); } catch { /* 已关闭 */ }
  }
}

async function waitTerminal(session, appRoot, itemIds) {
  const deadline = Date.now() + 600000;
  const pending = new Set(itemIds);
  const states = new Map();
  while (Date.now() < deadline && pending.size) {
    for (const itemId of [...pending]) {
      const row = readJobRow(appRoot, itemId);
      if (row && ["ready_for_review", "failed", "cancelled"].includes(row.stage) && !row.lease_owner) {
        states.set(itemId, row);
        pending.delete(itemId);
      }
    }
    if (session.exitCode() !== null) throw new Error(`app exited while waiting: ${session.exitCode()}`);
    await sleep(500);
  }
  if (pending.size) {
    // **不中止整轮**：把卡住的作业连同冻结现场（阶段/状态/租约/最后更新）记下来，
    // 其余作业继续走完预览/发布/离线轮——一份作业卡住不该让整轮证据全部作废。
    const stuck = [...pending].map(itemId => ({ itemId, row: readJobRow(appRoot, itemId) }));
    return { states, stuck };
  }
  return { states, stuck: [] };
}

async function importAll(session) {
  await session.clickSelector('[data-testid="library-import"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000 });
  await sleep(500);
  await session.clickSelector('[data-testid="import-pick-files"]');
  await session.waitFor(`!!document.querySelector('[data-testid="import-picked-files"] li')`, { timeoutMs: 20000 });
  await session.clickSelector('[data-testid="import-start"]');
  await session.waitFor(`(() => { const rows = document.querySelectorAll('[data-testid="library-row"]'); return rows.length >= ${PDFS.length}; })()`, { timeoutMs: 60000 });
  await sleep(1000);
  // 库页会按状态分区渲染，同一个 item 可能出现多次：**去重**后再交给验收。
  const ids = await session.evaluate(`Array.from(new Set(Array.from(document.querySelectorAll('[data-testid="library-row"]'),row=>row.dataset.itemId).filter(Boolean)))`);
  console.log(`[import] unique item ids: ${ids.length}`);
  return ids;
}

async function workspaceOf(session, itemId) {
  const result = await session.invoke("get_workspace_item", { itemId });
  if (result?.ok === false) throw new Error(`get_workspace_item:${result.error}`);
  return result?.value ?? result;
}

async function openWorkspace(session, itemId) {
  // 先确保停在库页：处理过程中应用可能已经跳走（或停在工作区页），此时行选择器
  // 在 DOM 里根本不存在——直接点会超时（实测两次真实运行都死在这一步）。
  await session.evaluate(`(() => { if (location.hash !== "#/library") location.hash = "#/library"; return location.hash; })()`).catch(() => {});
  await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 20000 }).catch(() => {});
  await sleep(400);
  await session.clickSelector(`[data-item-id="${itemId}"] .library-row-main`);
  await session.waitFor(`!!document.querySelector('.v2-passage-content, .workspace-preview-error')`, { timeoutMs: 30000 });
  await sleep(800);
}

/** 收集单份作业的全部证据（产物文件 + 工作区草稿 + 学生预览 + 发布）。
 *
 * 单独成函数是为了让**单份失败只影响这一份**：调用方 try/catch 后继续下一份，
 * 一份作业卡住或某个 UI 选择器失配不该让整轮的证据作废。
 */
async function collectItem(session, appRoot, itemId, item, pdf, states, atlas, offline) {
  const jobDir = path.join(appRoot, "jobs", itemId);
  // 单份耗时：作业行的 created_at → updated_at（终态时刻）。
  const timing = readJobRow(appRoot, itemId);
  if (timing?.created_at && timing?.updated_at) {
    item.startedAt = timing.created_at;
    item.finishedAt = timing.updated_at;
    item.durationMs = Date.parse(timing.updated_at) - Date.parse(timing.created_at);
  }
  const readJson = file => { try { return JSON.parse(fs.readFileSync(path.join(jobDir, file), "utf8")); } catch { return null; } };
  item.directResult = readJson("cloud-direct-result.json");
  item.fidelity = readJson("cloud-direct-fidelity.json");
  try {
    item.calls = fs.readFileSync(path.join(jobDir, "llm-calls.jsonl"), "utf8").trim().split(/\r?\n/).filter(Boolean).map(JSON.parse);
  } catch { item.calls = []; }
  item.llmUsage = readJson("llm-token-usage-total.json");
  const workspace = await workspaceOf(session, itemId);
  item.taskGroupCount = workspace?.ds?.taskGroups?.length ?? 0;
  item.slotCount = Object.keys(workspace?.ds?.answerSlots ?? {}).length;
  item.auditSource = workspace?.ds?.audit?.source ?? null;
  item.qualityIssueCodes = (workspace?.ds?.quality?.issues ?? []).reduce((counts, issue) => {
    counts[issue.code] = (counts[issue.code] ?? 0) + 1;
    return counts;
  }, {});
  const preflight = await session.invoke("get_publish_preflight", { jobId: itemId });
  item.publishPreflight = preflight?.value ?? preflight;
  item.paragraphCount = workspace?.ds?.passage?.content?.filter(node => node.type === "paragraph").length ?? 0;
  const paragraphMap = workspace?.ds?.passage?.paragraphMap ?? {};
  item.paragraphMapCount = Object.keys(paragraphMap).length;
  // 段落图里**不能有悬挂条目**（指向不存在的节点）；而条目数为 0 本身是合法的——
  // P3 这类原文没有 A/B/C 字母段，契约就不要求段落图。
  const nodeIds = new Set();
  const walkNodes = nodes => {
    for (const node of nodes ?? []) {
      if (node?.id) nodeIds.add(node.id);
      walkNodes(node?.children);
    }
  };
  walkNodes(workspace?.ds?.passage?.content);
  item.danglingParagraphMap = Object.entries(paragraphMap)
    .filter(([, nodeId]) => !nodeIds.has(nodeId))
    .map(([label, nodeId]) => ({ label, nodeId }));
  item.consistency = answerConsistency(workspace?.ds ?? {}, atlas.get(pdf));
  // 学生预览：真实 UI 打开工作区并切到学生模式。
  await openWorkspace(session, itemId);
  await session.clickSelector('[data-testid="workspace-mode-student"]').catch(() => {});
  // 预览编译走 Rust 命令（异步）：等错误块或学生画布出现，再读结论。
  await session.waitFor(
    `!!document.querySelector('[data-testid="workspace-preview-error"], [data-testid="exam-canvas-v2-student"]')`,
    { timeoutMs: 30000 }
  ).catch(() => {});
  await sleep(1200);
  const previewState = await session.evaluate(`(() => {
    const error = document.querySelector('[data-testid="workspace-preview-error"]');
    const summary = document.querySelector('[data-testid="workspace-preview-summary"]');
    const canvas = document.querySelector('[data-testid="exam-canvas-v2-student"]');
    return { error: error ? error.textContent : null, summary: summary ? summary.textContent : null, canvas: Boolean(canvas) };
  })()`);
  item.preview = previewState;
  if (!offline) {
    // 发布 NAS 包（真实 UI）。
    await session.clickSelector('[data-testid="workspace-publish"]');
    await session.waitFor(`!!document.querySelector('[data-publish-outcome]')`, { timeoutMs: 120000 }).catch(() => {});
    await sleep(1500);
    item.publishNotice = await session.evaluate(`document.querySelector('.workspace-notice')?.textContent ?? null`);
    const outcomeAttr = await session.evaluate(`document.querySelector('[data-publish-outcome]')?.getAttribute('data-publish-outcome') ?? null`);
    item.publishOutcome = outcomeAttr;
  }
}

async function runRound(label, { offline }) {
  const round = { label, offline, items: [], assertions: [] };
  report.rounds.push(round);
  const roundDir = path.join(runDir, label);
  const appRoot = path.join(roundDir, "appdata", "data");
  seedProfiles(appRoot, { offline });
  const sources = PDFS.map(pdf => path.join(pdfDir, pdf));
  for (const source of sources) {
    if (!fs.existsSync(source)) throw new Error(`样本 PDF 不存在：${source}`);
  }
  const session = await launchTauriAppCdp({
    exePath,
    runDir: roundDir,
    extraBrowserArgs: SESSION_BROWSER_ARGS,
    appEnv: {
      ...(release ? {} : { EPIC8_PDFIUM_LIB: path.join(repoRoot, "src-tauri/lib/pdfium-windows/pdfium.dll") }),
      PDF2TEST_AUTOMATION_SOURCE_FILES: sources.join(";")
    }
  });
  try {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 60000 });
    // 发布目录（设置页同一把 localStorage 键）。
    const nasDir = path.join(roundDir, "nas");
    await session.evaluate(`window.localStorage.setItem("ielts-author-studio.app-settings.v1", ${JSON.stringify(JSON.stringify({ nasDestination: nasDir, developerMode: false, cloudTokenBudget: 300000 }))})`);
    const ids = await importAll(session);
    console.log(`[${label}] imported ${ids.length} items`);
    const started = Date.now();
    const { states, stuck } = await waitTerminal(session, appRoot, ids);
    round.elapsedMs = Date.now() - started;
    if (stuck.length) {
      console.log(`[${label}] ${stuck.length} 份作业在期限内未到终态：${JSON.stringify(stuck)}`);
      round.stuckJobs = stuck;
    }
    const atlas = atlasAnswerKeys();
    // 文件名匹配（不再假设库行序 = PDFS 顺序）：从 job.json 的 source_files 取原文件名。
    const pdfOfItem = itemId => {
      try {
        const job = JSON.parse(fs.readFileSync(path.join(appRoot, "jobs", itemId, "job.json"), "utf8"));
        const name = job.source_files?.[0]?.original_name ?? job.sourceFiles?.[0]?.originalName ?? null;
        if (!name) return null;
        const normalize = value => value.replace(/【[^】]*】/g, "").replace(/\s+/g, " ").trim();
        const stem = normalize(path.basename(name, ".pdf"));
        return PDFS.find(candidate => {
          const target = normalize(path.basename(candidate, ".pdf"));
          return target === stem || target.startsWith(stem) || stem.startsWith(target);
        }) ?? null;
      } catch {
        return null;
      }
    };
    for (const itemId of ids) {
      const pdf = pdfOfItem(itemId);
      const item = { itemId, pdf, stage: states.get(itemId) };
      try {
        await collectItem(session, appRoot, itemId, item, pdf, states, atlas, offline);
      } catch (error) {
        // 单份的 UI 步骤失败（点击/预览/发布）只记这一份失败，其余继续——
        // 一份失败不该让整轮证据作废。
        item.error = String(error);
        console.log(`[${label}] ${pdf ?? itemId} UI 步骤失败：${error}`);
      }
      round.items.push(item);
    }
    // 汇总断言
    const matched = round.items.filter(item => Boolean(item.pdf));
    const extras = round.items.filter(item => !item.pdf);
    round.extraItems = extras.map(item => item.itemId);
    if (extras.length) console.log(`[${label}] 非样本作业（不参与逐份断言）：${extras.map(item => item.itemId).join(",")}`);
    check("5 份样本 PDF 都匹配到作业", matched.length === PDFS.length, {
      matched: matched.map(item => item.pdf),
      extraItems: round.extraItems
    });
    check(
      "五份全部到达终态且无 failed",
      matched.every(item => item.stage?.stage === "ready_for_review") && !(round.stuckJobs?.length),
      { stages: matched.map(item => ({ pdf: item.pdf, stage: item.stage })), stuck: round.stuckJobs ?? [] }
    );
    check("五份都产出题稿（题组与答案位非空）", matched.every(item => item.taskGroupCount > 0 && item.slotCount > 0), matched.map(item => ({ pdf: item.pdf, groups: item.taskGroupCount, slots: item.slotCount })));
    check("五份学生预览无阻断错误且画布渲染", matched.every(item => !item.preview?.error && item.preview?.canvas), matched.map(item => ({ pdf: item.pdf, error: item.preview?.error ?? null, canvas: item.preview?.canvas ?? false })));
    if (!offline) {
      check("正确直出稿发布提示为个位数且没有旧本地复核噪音", matched.every(item =>
        Array.isArray(item.publishPreflight?.warnings) && item.publishPreflight.warnings.length < 10
        && !item.publishPreflight.warnings.some(w => /^(SOURCE_REVIEW|RECOGNITION_CHOICE)/.test(w.code))
      ), matched.map(item => ({ pdf: item.pdf, warnings: item.publishPreflight?.warnings ?? null, qualityIssueCodes: item.qualityIssueCodes })));
      check("直出稿 revision source = cloud_direct", matched.every(item => item.auditSource === "cloud_direct"), matched.map(item => ({ pdf: item.pdf, source: item.auditSource })));
      check("每份云端调用 ≤3 次", matched.every(item => (item.directResult?.calls ?? 99) <= 3), matched.map(item => ({ pdf: item.pdf, calls: item.directResult?.calls })));
      check("每份编译问题为空", matched.every(item => (item.directResult?.compileIssues ?? []).length === 0), matched.map(item => ({ pdf: item.pdf, issues: item.directResult?.compileIssues })));
      // 答案一致率：分母 = Atlas 全部题号（缺题按不一致计）。
      check("每份答案全部一致（分母=Atlas 全部题号）", matched.every(item => item.consistency?.matched === item.consistency?.total && item.consistency?.total > 0), matched.map(item => ({ pdf: item.pdf, rate: item.consistency?.rate, matched: item.consistency?.matched, total: item.consistency?.total, missing: item.consistency?.missing })));
      check("每份题数与 Atlas 印出的题号数一致", matched.every(item => item.consistency?.documentSlots === item.consistency?.total && item.consistency?.total > 0), matched.map(item => ({ pdf: item.pdf, documentSlots: item.consistency?.documentSlots, atlas: item.consistency?.total })));
      check("五份答案66/66", matched.reduce((n,i)=>n+(i.consistency?.matched??0),0)===66 && matched.reduce((n,i)=>n+(i.consistency?.total??0),0)===66, matched.map(i=>i.consistency));
      const directCalls = item => (item.calls ?? []).filter(c => c.commandName === "generate_exam_direct" && c.providerCall === true);
      check("每次直出请求体≤350000B", matched.every(item => directCalls(item).length > 0 && directCalls(item).every(c => c.requestBytes > 0 && c.requestBytes <= 350000)), matched.map(item => ({pdf:item.pdf, calls:directCalls(item).map(c=>({requestBytes:c.requestBytes,imageCount:c.imageCount,segments:c.segments}))})));
      // 正文覆盖率：正文与 PDF 文字层的词级覆盖（保真度报告），≥0.95。
      check("每份正文覆盖率 ≥0.95（与 PDF 文字层对照）", matched.every(item => (item.fidelity?.overallCoverage ?? 0) >= 0.95), matched.map(item => ({ pdf: item.pdf, coverage: item.fidelity?.overallCoverage ?? null, flagged: item.fidelity?.flaggedParagraphIds ?? null })));
      check("每份段落图无悬挂条目（段落图可空，但不能指向不存在的节点）", matched.every(item => (item.danglingParagraphMap ?? []).length === 0), matched.map(item => ({ pdf: item.pdf, paragraphs: item.paragraphCount, mapped: item.paragraphMapCount, dangling: item.danglingParagraphMap ?? [] })));
      // 单份耗时 ≤90s（created_at → updated_at）。
      check("每份处理耗时 ≤90s", matched.every(item => typeof item.durationMs === "number" && item.durationMs >= 0 && item.durationMs <= 90000), matched.map(item => ({ pdf: item.pdf, durationMs: item.durationMs ?? null })));
      // token ≤60k/份（估算值也计入，见软上限记账）。
      // 用量总计文件是 camelCase，且**没有** total 字段：按 prompt + completion 求和。
      const usageTotal = item => {
        const usage = item.llmUsage;
        if (!usage) return null;
        const prompt = usage.promptTokens;
        const completion = usage.completionTokens;
        if (typeof prompt !== "number" || typeof completion !== "number") return null;
        return prompt + completion;
      };
      check("每份 token ≤60k", matched.every(item => {
        const total = usageTotal(item);
        return typeof total === "number" && total <= 60000;
      }), matched.map(item => ({ pdf: item.pdf, totalTokens: usageTotal(item), usage: item.llmUsage ?? null })));
      check("五份都发布且学生端可加载（published）", matched.every(item => item.publishOutcome === "published"), matched.map(item => ({ pdf: item.pdf, outcome: item.publishOutcome, notice: item.publishNotice })));
    } else {
      check("离线稿 revision source = auto_extract（本地链）", matched.every(item => item.auditSource === "auto_extract"), matched.map(item => ({ pdf: item.pdf, source: item.auditSource })));
      check("离线无云端调用", matched.every(item => item.calls.length === 0), matched.map(item => ({ pdf: item.pdf, calls: item.calls.length })));
    }
  } catch (error) {
    // 失败也要把应用的 stderr/stdout 带进报告：作业卡住这类问题，原因只可能在
    // 应用自己的输出里（调度器的 eprintln）。
    round.error = String(error);
    round.appOutput = String(session.appOutput?.() ?? "").slice(0, 4000);
    throw error;
  } finally {
    await session.close?.();
  }
  return round;
}

// 启动前探针（在跑任何一轮之前）：页面 URL 决定 exe 是不是产品构建。
const devUrlPattern = /localhost:1420/;
try {
  const probe = await probePageUrl(exePath);
  report.preflight = {
    exePath,
    pageUrl: probe.url,
    targetCount: probe.targetCount,
    devUrl: probe.url ? devUrlPattern.test(probe.url) : null,
    ok: Boolean(probe.url) && !devUrlPattern.test(probe.url)
  };
  console.log(`[preflight] page url = ${probe.url ?? "(未取到 page target)"}`);
} catch (error) {
  report.preflight = { exePath, pageUrl: null, ok: false, error: String(error) };
  console.log(`[preflight] FAILED :: ${error}`);
}

// 看门狗：任何一段无界等待都不该让报告写不出来。
const WATCHDOG_MS = 45 * 60 * 1000;
const watchdog = setTimeout(() => {
  console.log("[watchdog] 超过 45 分钟，强制收尾并写报告");
  report.watchdogFired = true;
  try {
    report.verdict = "failed";
    fs.writeFileSync(path.join(runDir, "report.json"), JSON.stringify(report, null, 2));
    console.log(`report: ${path.join(runDir, "report.json")} verdict=failed`);
  } catch (error) {
    console.log(`[watchdog] 写报告失败：${error}`);
  }
  process.exit(2);
}, WATCHDOG_MS);
watchdog.unref?.();

// 两轮都跑；**任何一轮抛错也必须写出 report**——修前失败只留在终端里，
// 报告目录里没有 report.json，读者无从判断跑到哪一步、错在哪。
try {
  for (const [label, offline] of (process.argv.includes("--online-only") ? [["online-direct", false]] : [["online-direct", false], ["offline-local", true]])) {
    try {
      await runRound(label, { offline });
    } catch (error) {
      console.log(`[${label}] ROUND FAILED :: ${error}`);
      const last = report.rounds[report.rounds.length - 1];
      const assertion = { name: "整轮未抛错", ok: false, detail: String(error) };
      if (last?.label === label) {
        last.aborted = true;
        last.error = String(error);
        last.assertions.push(assertion);
      } else {
        report.rounds.push({ label, offline, aborted: true, error: String(error), items: [], assertions: [assertion] });
      }
    }
  }

  // ── 学生端加载器**契约**探针（对在线轮 NAS 包）──────────────────────────
  // 证据层级：contract（加载器契约 + 静态产物检查），**不是**「学生端渲染通过」。
  {
    const onlineRound = report.rounds.find(round => round.label === "online-direct");
    const nasDir = path.join(runDir, "online-direct", "nas");
    const probeOut = path.join(runDir, "nas-student-contract.json");
    const probeAssertion = { name: "NAS 加载器契约探针（契约级，非学生端渲染）", ok: false, detail: null };
    try {
      if (!onlineRound) throw new Error("在线轮未运行，跳过 NAS 契约探针");
      if (!fs.existsSync(nasDir)) {
        // 没有包可探：这不是探针失败，而是「未执行」。如实标注，不假装通过。
        probeAssertion.name = "NAS 加载器契约探针（未执行：在线轮未产出 NAS 包）";
        throw new Error(`在线轮未产出 NAS 包目录：${nasDir}`);
      }
      // 用**异步** spawn 调探针：本机 Node 的同步 spawn（execFileSync/spawnSync）
      // 一律 EBUSY，异步 spawn 正常（harness 启应用用的就是它）。
      await new Promise((resolve, reject) => {
        const child = spawn(process.execPath, [
          path.join(repoRoot, "scripts/e2e/nas-student-contract.mjs"),
          "--package", nasDir,
          "--out", probeOut
        ], { stdio: ["ignore", "pipe", "pipe"] });
        let output = "";
        child.stdout.on("data", chunk => { output += chunk; });
        child.stderr.on("data", chunk => { output += chunk; });
        child.on("error", error => reject(error));
        child.on("close", code => {
          if (code === 0) resolve();
          else reject(new Error(`nas-student-contract exit=${code}: ${output.slice(-400)}`));
        });
      });
      const probe = JSON.parse(fs.readFileSync(probeOut, "utf8"));
      report.nasStudentContract = probe;
      const failed = (probe.results ?? []).filter(result => !result.ok);
      probeAssertion.ok = failed.length === 0;
      probeAssertion.detail = { failed, total: (probe.results ?? []).length };
      console.log(`[probe] NAS contract ${failed.length === 0 ? "PASS" : "FAIL"} (${(probe.results ?? []).length - failed.length}/${(probe.results ?? []).length})`);
    } catch (error) {
      report.nasStudentContractError = String(error);
      probeAssertion.detail = String(error);
      console.log(`[probe] NAS contract FAIL :: ${error}`);
    }
    if (onlineRound) onlineRound.assertions.push(probeAssertion);
    else report.rounds.push({ label: "nas-contract-probe", items: [], assertions: [probeAssertion] });
  }
} finally {
  // 启动前断言：页面不是 devUrl（exe 必须是 `tauri build` 产物）。
  const preflightAssertion = {
    name: "exe 是产品构建（页面 URL 不是 devUrl http://localhost:1420）",
    ok: Boolean(report.preflight?.ok),
    detail: report.preflight ?? null
  };
  const freshnessAssertion = {
    name: "exe 与当前源码及内嵌前端哈希一致",
    ok: report.identity?.buildFresh?.ok === true,
    detail: report.identity?.buildFresh ?? null
  };
  if (report.rounds[0]) report.rounds[0].assertions.unshift(freshnessAssertion);
  if (report.rounds[0]) report.rounds[0].assertions.unshift(preflightAssertion);
  else report.rounds.push({ label: "preflight", items: [], assertions: [preflightAssertion] });
  report.verdict = report.rounds.length > 0
    && report.rounds.every(round => round.assertions.every(assertion => assertion.ok))
    ? "passed" : "failed";
  fs.writeFileSync(path.join(runDir, "report.json"), JSON.stringify(report, null, 2));
  console.log(`report: ${path.join(runDir, "report.json")} verdict=${report.verdict}`);
  process.exitCode = report.verdict === "passed" ? 0 : 1;
}
