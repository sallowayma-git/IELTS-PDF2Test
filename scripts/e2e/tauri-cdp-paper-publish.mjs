#!/usr/bin/env node
// 组卷发布真实应用 e2e（题库页「组卷视图」，合成语料，不依赖私有 PDF）：
//   启动前预置三份阅读卷（fixtures/golden/synthetic/ielts 的可发布权威稿，各自不同的 jobId 与 examId）
//   → 启动真 Tauri（启动迁移把它们种进题库）→ 题库页用行内 Part 下拉设 P1/P2/P3
//   → 组卷视图勾选三篇 → 确认替换 → 发布为当前试卷
//   → 断言 NAS：current-paper.json 每条含 entry（= 清单条目原样）、资源位于 releases/<batchId>/resources/<examId>/、
//     根级 resources/ 未被写入
//   → 再次发布同三篇：第一批次目录仍在；两批次 asset-manifest.json 字节相同；第一批次文件字节不变
//   → 名称单元格改名（Enter 保存）→ 重新发布 → 清单与当前试卷的 title 均为新名。
//
// 证据层级：real-app-e2e（CDP 自动化通道驱动真实 Tauri/WebView2 界面 + 读取磁盘事实 + 真实 publish_items 命令）。
// 缺少合成权威稿、或 exe 不存在时以 CannotRunError 退出（退出码 3），不伪装成通过。
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import {
  CDP_CHANNEL_LABEL, CDP_CHANNEL_NOTE, CannotRunError,
  assertBuildFresh, buildFreshReport, gitHead, launchTauriAppCdp, repoRoot, sha256File, sleep, writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `paper-publish-${stamp}`);
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const extraArgs = "--no-sandbox --disable-gpu";
const APP_SETTINGS_KEY = "ielts-author-studio.app-settings.v1";
const SYNTHETIC_AUTHORING = path.join(repoRoot, "fixtures", "golden", "synthetic", "ielts", "early-approaches-authoring-v2.json");
const PARTS = ["P1", "P2", "P3"];
const RENAME_TO = `组卷改名验证 ${stamp.slice(11, 19)}`;

function recordAssertion(report, id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

/**
 * 复现 product_chain.rs::physical_shadow_for：导出会重算质量，文档阴影必须与权威稿的 sourceAnchors 一致，
 * 否则门禁会拦下。这是本套件要验证的发布链的一部分，而不是测试替身。
 */
function physicalShadowFor(authoring, jobId) {
  const nodeIds = new Set();
  const walk = (value) => {
    if (Array.isArray(value)) { for (const item of value) walk(item); return; }
    if (value && typeof value === "object") {
      if (Array.isArray(value.sourceAnchors)) {
        for (const anchor of value.sourceAnchors) for (const id of anchor?.nodeIds ?? []) nodeIds.add(id);
      }
      for (const child of Object.values(value)) walk(child);
    }
  };
  walk(authoring);
  const sourceHash = "a".repeat(64);
  const sourceFileId = authoring?.exam?.sourceFiles?.[0]?.sourceFileId ?? "source-pdf-1";
  return {
    schemaVersion: "DocumentIRV2",
    documentId: authoring.sourceDocumentId ?? "document-1",
    jobId,
    sourceFiles: [{ sourceFileId, originalName: "synthetic-paper.pdf", mediaType: "application/pdf", sha256: sourceHash, byteLength: 1, role: "question_paper" }],
    pages: [{
      pageIndex: 0, widthPt: 612.0, heightPt: 792.0, rotation: 0,
      glyphs: [], spans: [], lines: [],
      regions: [{
        id: "region-question-surface", kind: "text",
        bbox: { x: 10.0, y: 10.0, width: 500.0, height: 200.0, unit: "pt", origin: "top-left", pageRotation: 0 },
        childLineIds: [...nodeIds].sort(), childObjectIds: [], confidence: 1.0,
        sourceAnchors: [{ sourceFileId, pageIndex: 0, nodeIds: ["region-question-surface"], extractionMode: "pdf_native", sourceHash }],
      }],
      vectorPaths: [], tables: [], assetIds: [], readingOrder: ["region-question-surface"],
      quality: { classification: "born_digital", nativeCharacterCount: 100, unicodeErrorRatio: 0.0, duplicateTextRatio: 0.0, imageCoverageRatio: 0.0, textCoverageRatio: 1.0, rotationConfidence: 1.0, requiresOcrRegions: [] },
    }],
    assets: [],
    extraction: { engine: "e2e-paper-publish-fixture", engineVersion: "1.0.0", extractedAt: "2026-01-01T00:00:00Z", warnings: [] },
  };
}

/** 预置一份可发布的阅读卷（启动前写入数据目录，启动迁移把它种进题库）。 */
function seedReadingJob(dataDir, index, authoringTemplate) {
  const token = crypto.randomBytes(4).toString("hex");
  const jobId = `e2e-paper-${index}-${token}`;
  const examId = `e2epaper${index}${token}`;
  const title = `E2E 组卷阅读 ${index} ${stamp.slice(11, 19)}`;
  const jobDir = path.join(dataDir, "jobs", jobId);
  fs.mkdirSync(jobDir, { recursive: true });
  const authoring = JSON.parse(JSON.stringify(authoringTemplate));
  authoring.jobId = jobId;
  authoring.exam.examId = examId;
  authoring.exam.title = title;
  // 与 nas_package_v2.rs 的 test_export_bundle 同款：权威稿的质量块标为 ready、无遗留 issues，
  // 题库才把它列为「可发布」。导出时门禁仍会重算质量（这才是要被验证的部分）。
  authoring.quality = { ...(authoring.quality ?? {}), state: "ready", issues: [] };
  const now = "2026-11-07T00:00:00Z";
  const job = {
    // 产品的 job 状态映射（library/migration.rs::status_of）：DraftSaved → ready（可发布），Working → action_required。
    jobId, title, status: "DraftSaved", category: "P1", frequency: "medium", tags: ["e2e-paper-publish"],
    sourceFiles: [{
      fileId: `file-e2e-paper-${index}-${token}`, originalName: `synthetic-paper-${index}.pdf`, storedName: `e2e-paper-${index}-${token}.pdf`,
      fileType: "pdf", sha256: "a".repeat(64), sizeBytes: 1, role: "MainQuestion", importedAt: now,
    }],
    activeLlmProfileId: null, createdAt: now, updatedAt: now, currentStep: "Preview",
    issueCounts: { errors: 0, warnings: 0, needsReview: 0 },
  };
  fs.writeFileSync(path.join(jobDir, "job.json"), JSON.stringify(job, null, 2));
  fs.writeFileSync(path.join(jobDir, "authoring-ir-v2.shadow.json"), JSON.stringify(authoring, null, 2));
  fs.writeFileSync(path.join(jobDir, "document-ir-v2.shadow.json"), JSON.stringify(physicalShadowFor(authoring, jobId), null, 2));
  return { jobId, examId, title };
}

/** 题库行快照（启动迁移之后由后端 list_library_items 读回）。 */
async function libraryRows(session) {
  const r = await session.invoke("list_library_items", { includeDeleted: false });
  return r?.ok && Array.isArray(r.value) ? r.value : [];
}

async function waitForRows(session, ids, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let rows = [];
  while (Date.now() < deadline) {
    rows = await libraryRows(session);
    if (ids.every((id) => rows.some((row) => row.id === id))) return rows;
    await sleep(1000);
  }
  return rows;
}

/** 行内 Part 下拉（与题库表格同一个 testid）。 */
async function setRowPart(session, itemId, part) {
  return await session.evaluate(`(() => {
    const el = document.querySelector('[data-item-id="${itemId}"] [data-testid="library-row-part-select"]');
    if (!el) return false;
    const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set;
    setter.call(el, '${part}'); el.dispatchEvent(new Event('change', { bubbles: true })); return true;
  })()`);
}

/** 组卷视图的勾选框：只在未勾选时点击，避免重复发布时把已勾选的又取消。 */
async function ensureChecked(session, itemId) {
  return await session.evaluate(`(() => {
    const tr = document.querySelector('tr[data-item-id="${itemId}"]');
    const check = tr && tr.querySelector('input[type="checkbox"]');
    if (!check || check.disabled) return 'missing';
    if (!check.checked) check.click();
    return check.checked ? 'checked' : 'unchecked';
  })()`);
}

async function publishCurrentSelection(session, report, label) {
  await session.waitFor(`document.querySelector('[data-testid="paper-publish"]') && !document.querySelector('[data-testid="paper-publish"]').disabled`,
    { timeoutMs: 15000, label: `${label}-publish-enabled` });
  await session.clickSelector('[data-testid="paper-publish"]');
  await session.waitFor(`!!document.querySelector('[data-testid="paper-confirm"]')`, { timeoutMs: 10000, label: `${label}-confirm` });
  await session.clickSelector('[data-testid="paper-confirm-ok"]');
  await session.waitFor(
    `(() => { const m = document.querySelector('[data-testid="paper-composer"] [role="status"]'); return !!m && /当前试卷已发布|未能发布|保持不变|旧试卷|没有更新|另一次发布/.test(m.textContent); })()`,
    { timeoutMs: 10 * 60 * 1000, intervalMs: 1000, label: `${label}-result` },
  );
  const text = await session.evaluate(`(document.querySelector('[data-testid="paper-composer"] [role="status"]')||{}).textContent || null`);
  report[`${label}ResultText`] = text;
  return text;
}

function readPointer(nasRoot) {
  return JSON.parse(fs.readFileSync(path.join(nasRoot, "current-paper.json"), "utf8"));
}

function readManifest(nasRoot) {
  const text = fs.readFileSync(path.join(nasRoot, "manifest.js"), "utf8");
  return JSON.parse(text.trim().replace(/^window\.__READING_EXAM_MANIFEST__\s*=\s*/, "").replace(/;\s*$/, ""));
}

/** 指针 ↔ 清单 ↔ 磁盘：返回 { ok, details }。 */
function checkPointerOnDisk(nasRoot) {
  const pointer = readPointer(nasRoot);
  const manifest = readManifest(nasRoot);
  const problems = [];
  if (pointer.schemaVersion !== "CurrentPaperV1") problems.push(`schema=${pointer.schemaVersion}`);
  if (pointer.reading.length !== 3 || pointer.reading.map((e) => e.part).join(",") !== PARTS.join(",")) {
    problems.push(`reading parts=${pointer.reading.map((e) => e.part).join(",")}`);
  }
  for (const item of pointer.reading) {
    const entry = item.entry;
    if (!entry) { problems.push(`${item.part} 缺 entry`); continue; }
    if (JSON.stringify(entry) !== JSON.stringify(manifest[item.examId])) problems.push(`${item.part} entry 与清单不一致`);
    if (item.script !== entry.script || item.scriptSha256 !== entry.checksums?.scriptSha256 || item.runtimeSha256 !== entry.checksums?.runtimeSha256) {
      problems.push(`${item.part} 顶层与 entry 不一致`);
    }
    const scriptFile = path.join(nasRoot, entry.script.replace(/^\.\//, ""));
    if (!fs.existsSync(scriptFile) || sha256File(scriptFile) !== item.scriptSha256) problems.push(`${item.part} 脚本字节校验失败`);
    const base = `${pointer.batchId}`;
    const prefix = `./releases/${base}/resources/${item.examId}`;
    if (entry.resourcesBase !== prefix || entry.assetManifest !== `${prefix}/asset-manifest.json`) {
      problems.push(`${item.part} 资源未在批次目录：${entry.resourcesBase}`);
    }
    if (!fs.existsSync(path.join(nasRoot, entry.assetManifest.replace(/^\.\//, "")))) problems.push(`${item.part} asset-manifest 缺失`);
    if (fs.existsSync(path.join(nasRoot, "resources", item.examId))) problems.push(`${item.part} 根级 resources 被写入`);
  }
  return { ok: problems.length === 0, problems, pointer, manifest };
}

async function main() {
  if (!fs.existsSync(SYNTHETIC_AUTHORING)) throw new CannotRunError(`合成权威稿缺失：${SYNTHETIC_AUTHORING}`);
  if (!fs.existsSync(exePath)) throw new CannotRunError(`exe 不存在（先运行 node scripts/e2e/build-app.mjs）：${exePath}`);
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  const template = JSON.parse(fs.readFileSync(SYNTHETIC_AUTHORING, "utf8"));

  const report = {
    task: "paper-publish",
    scope: "题库组卷视图：三篇阅读预置→设 Part→勾选→发布为当前试卷；指针自包含、批次目录不可变、重发布字节稳定、改名后重发布",
    channel: CDP_CHANNEL_LABEL, channelNote: CDP_CHANNEL_NOTE,
    evidenceLevel: "real-app-e2e (CDP automation channel + real publish_items + disk facts)",
    identity: { exePath, commit: gitHead(repoRoot), buildFresh: buildFreshReport(fresh) },
    assertions: [], consoleErrors: [], pageExceptions: [],
  };
  const nasRoot = path.join(runDir, "nas-library");
  fs.mkdirSync(nasRoot, { recursive: true });
  const dataDir = path.join(runDir, "appdata", "data");
  const seeded = [1, 2, 3].map((index) => seedReadingJob(dataDir, index, template));
  const ids = seeded.map((job) => job.jobId);
  report.seeded = seeded;

  const session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  let verdict = "failed";
  try {
    report.browserArgs = session.browserArgs;
    const migrated = await waitForRows(session, ids, 90000);
    recordAssertion(report, "P-0 seeded-jobs-migrated-into-library",
      ids.every((id) => migrated.some((row) => row.id === id && row.modality === "reading")),
      `题库里的预置条目=${ids.filter((id) => migrated.some((row) => row.id === id)).length}/3`);
    if (!ids.every((id) => migrated.some((row) => row.id === id))) throw new Error("预置的三篇没有被启动迁移种进题库");

    // 预置条目是「识别完成、未确认」：经真实后端命令确认入库（与界面「确认入库」同一条后端路径），才会进题库。
    const confirmed = await session.invoke("set_library_items_confirmed", { itemIds: ids, confirmed: true });
    const confirmRows = confirmed?.ok ? confirmed.value?.results ?? [] : [];
    recordAssertion(report, "P-0b backend-confirm-seeded-items",
      confirmed?.ok && confirmRows.length === ids.length && confirmRows.every((row) => row.ok),
      `后端确认=${confirmRows.filter((row) => row.ok).length}/${ids.length}${confirmed?.ok ? "" : `；${confirmed?.error ?? ""}`}`);
    if (!confirmed?.ok || confirmRows.some((row) => !row.ok)) throw new Error("预置条目没有通过后端确认入库");

    // 1. 发布目录写进设置，并重新挂载题库页让「当前试卷」卡片读到目录。
    await session.evaluate(`(() => {
      const raw = window.localStorage.getItem(${JSON.stringify(APP_SETTINGS_KEY)});
      const settings = raw ? JSON.parse(raw) : {};
      settings.nasDestination = ${JSON.stringify(nasRoot)};
      window.localStorage.setItem(${JSON.stringify(APP_SETTINGS_KEY)}, JSON.stringify(settings));
      window.location.hash = "#/settings"; return true;
    })()`);
    await sleep(400);
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="current-paper-card"]')`, { timeoutMs: 40000, label: "paper-card" });
    await session.screenshot("01-library-seeded");

    // 2. 行内 Part 下拉设 P1/P2/P3（三篇各一个 Part）。
    for (const [index, id] of ids.entries()) {
      await session.waitFor(`!!document.querySelector('[data-item-id="${id}"] [data-testid="library-row-part-select"]')`, { timeoutMs: 20000, label: `part-select-${index}` });
      await setRowPart(session, id, PARTS[index]);
      const deadline = Date.now() + 15000;
      while (Date.now() < deadline) {
        const row = (await libraryRows(session)).find((r) => r.id === id);
        if (row?.partLabel === PARTS[index]) break;
        await sleep(500);
      }
    }
    const rowsAfterParts = await libraryRows(session);
    const partOf = Object.fromEntries(ids.map((id) => [id, rowsAfterParts.find((r) => r.id === id)?.partLabel ?? null]));
    recordAssertion(report, "P-1 three-parts-assigned", PARTS.every((p, i) => partOf[ids[i]] === p), `Part=${JSON.stringify(partOf)}`);
    if (!PARTS.every((p, i) => partOf[ids[i]] === p)) throw new Error("无法为三篇分别设置 P1/P2/P3");

    // 3. 组卷视图勾选三篇 → 发布为当前试卷（确认替换）。
    await session.waitFor(`!!document.querySelector('[data-testid="paper-table"]')`, { timeoutMs: 15000, label: "paper-table" });
    const checked1 = [];
    for (const id of ids) checked1.push(await ensureChecked(session, id));
    recordAssertion(report, "P-2 three-rows-checked", checked1.every((s) => s === "checked"), `勾选=${JSON.stringify(checked1)}`);
    await session.screenshot("02-paper-composer-three-checked");
    const firstText = await publishCurrentSelection(session, report, "first");
    recordAssertion(report, "P-3 first-publish-message", String(firstText).includes("当前试卷已发布"), `界面提示=${JSON.stringify(firstText)}`);
    await session.screenshot("03-first-publish-result");

    const first = checkPointerOnDisk(nasRoot);
    recordAssertion(report, "P-4 pointer-entries-self-contained-and-on-disk", first.ok, first.ok ? "指针 entry = 清单条目，脚本字节一致，资源在 releases/<batchId>/resources" : first.problems.join("；"));
    const firstBatch = first.pointer.batchId;
    report.firstBatch = firstBatch;
    const firstPinned = first.pointer.reading.flatMap((item) => [item.entry.script, item.entry.assetManifest]).map((rel) => {
      const abs = path.join(nasRoot, rel.replace(/^\.\//, ""));
      return { abs, bytes: fs.readFileSync(abs) };
    });

    // 4. 同三篇再次发布：第一批次仍在、字节不变；两批次 asset-manifest 字节相同。
    const checked2 = [];
    for (const id of ids) checked2.push(await ensureChecked(session, id));
    recordAssertion(report, "P-5 rechecked-for-second-publish", checked2.every((s) => s === "checked"), `勾选=${JSON.stringify(checked2)}`);
    const secondText = await publishCurrentSelection(session, report, "second");
    recordAssertion(report, "P-6 second-publish-message", String(secondText).includes("当前试卷已发布"), `界面提示=${JSON.stringify(secondText)}`);
    const second = checkPointerOnDisk(nasRoot);
    recordAssertion(report, "P-7 second-pointer-on-disk", second.ok, second.problems.join("；") || "指针与磁盘一致");
    const secondBatch = second.pointer.batchId;
    report.secondBatch = secondBatch;
    recordAssertion(report, "P-8 old-batch-directory-kept", fs.existsSync(path.join(nasRoot, "releases", firstBatch)),
      `releases/${firstBatch} 存在=${fs.existsSync(path.join(nasRoot, "releases", firstBatch))}`);
    const pinnedUnchanged = firstPinned.every((item) => fs.existsSync(item.abs) && fs.readFileSync(item.abs).equals(item.bytes));
    recordAssertion(report, "P-9 first-batch-files-byte-identical", pinnedUnchanged, `旧批次 ${firstPinned.length} 个文件字节不变=${pinnedUnchanged}`);
    const assetSame = second.pointer.reading.every((item) => {
      const a = fs.readFileSync(path.join(nasRoot, "releases", firstBatch, "resources", item.examId, "asset-manifest.json"));
      const b = fs.readFileSync(path.join(nasRoot, "releases", secondBatch, "resources", item.examId, "asset-manifest.json"));
      return a.equals(b);
    });
    recordAssertion(report, "P-10 asset-manifest-bytes-identical-across-batches", assetSame, `两批次 asset-manifest 字节相同=${assetSame}`);
    await session.screenshot("04-second-publish-result");

    // 5. 名称单元格改名（Enter 保存）→ 后端读回新名 → 重新发布 → 清单与当前试卷的 title 为新名。
    const renameTarget = ids[1];
    await session.clickSelector(`tr[data-item-id="${renameTarget}"] .paper-title-cell`);
    await session.waitFor(`!!document.querySelector('.paper-title-input')`, { timeoutMs: 5000, label: "title-input" });
    await session.evaluate(`(() => {
      const input = document.querySelector('.paper-title-input');
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(input, ${JSON.stringify(RENAME_TO)});
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
      return true;
    })()`);
    let savedTitle = null;
    const renameDeadline = Date.now() + 15000;
    while (Date.now() < renameDeadline) {
      savedTitle = (await libraryRows(session)).find((r) => r.id === renameTarget)?.title ?? null;
      if (savedTitle === RENAME_TO) break;
      await sleep(500);
    }
    recordAssertion(report, "P-11 rename-persists-in-library", savedTitle === RENAME_TO, `后端标题=${JSON.stringify(savedTitle)}`);

    const checked3 = [];
    for (const id of ids) checked3.push(await ensureChecked(session, id));
    recordAssertion(report, "P-12 rechecked-after-rename", checked3.every((s) => s === "checked"), `勾选=${JSON.stringify(checked3)}`);
    const thirdText = await publishCurrentSelection(session, report, "third");
    const third = checkPointerOnDisk(nasRoot);
    const renamedExam = third.pointer.reading.find((item) => item.part === "P2");
    const manifestTitle = renamedExam ? third.manifest[renamedExam.examId]?.title : null;
    recordAssertion(report, "P-13 republish-after-rename-carries-new-title",
      third.ok && renamedExam?.title === RENAME_TO && renamedExam?.entry?.title === RENAME_TO && manifestTitle === RENAME_TO,
      `界面=${JSON.stringify(thirdText)} 指针title=${JSON.stringify(renamedExam?.title)} 清单title=${JSON.stringify(manifestTitle)}`);
    await session.screenshot("05-rename-then-republish");

    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    verdict = report.assertions.every((a) => a.ok) ? "passed" : "failed";
  } catch (error) {
    report.error = String(error?.message ?? error);
    await session.screenshot("fail-final").catch(() => {});
  } finally {
    report.verdict = verdict;
    report.failedAssertions = report.assertions.filter((a) => !a.ok).map((a) => a.id);
    report.screenshotDir = runDir;
    await session.close?.().catch?.(() => {});
  }
  const file = writeReport(runDir, report);
  console.log(`[paper-publish] verdict=${report.verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
  console.log(`[paper-publish] report=${file}`);
  process.exitCode = report.verdict === "passed" ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  console.error(cannotRun ? `[paper-publish] CANNOT RUN: ${error.message}` : error);
  process.exitCode = cannotRun ? 3 : 1;
});
