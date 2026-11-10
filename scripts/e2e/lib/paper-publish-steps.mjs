// 组卷发布 CDP 步骤的可复用函数（抽自 tauri-cdp-paper-publish.mjs 的做法，供跨仓库全链路编排脚本使用）。
// 只提供「预置阅读卷 / 题库行 / Part / 组卷勾选 / 发布 / 指针与磁盘校验」，不含断言框架之外的业务判断。
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { sha256File, sleep } from "./tauri-cdp-harness.mjs";

export const APP_SETTINGS_KEY = "ielts-author-studio.app-settings.v1";
export const PARTS = ["P1", "P2", "P3"];

/** 复现 product_chain.rs::physical_shadow_for（与 tauri-cdp-paper-publish.mjs 同款）。 */
export function physicalShadowFor(authoring, jobId) {
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

/** 预置一份可发布的阅读卷（启动前写入数据目录；启动迁移把它种进题库）。 */
export function seedReadingJob(dataDir, index, authoringTemplate, stamp) {
  const token = crypto.randomBytes(4).toString("hex");
  const jobId = `e2e-chain-${index}-${token}`;
  const examId = `e2echain${index}${token}`;
  const title = `全链路组卷 ${index} ${stamp.slice(11, 19)}`;
  const jobDir = path.join(dataDir, "jobs", jobId);
  fs.mkdirSync(jobDir, { recursive: true });
  const authoring = JSON.parse(JSON.stringify(authoringTemplate));
  authoring.jobId = jobId;
  authoring.exam.examId = examId;
  authoring.exam.title = title;
  authoring.quality = { ...(authoring.quality ?? {}), state: "ready", issues: [] };
  const now = "2026-11-07T00:00:00Z";
  const job = {
    jobId, title, status: "DraftSaved", category: "P1", frequency: "medium", tags: ["e2e-cross-repo-chain"],
    sourceFiles: [{
      fileId: `file-e2e-chain-${index}-${token}`, originalName: `synthetic-paper-${index}.pdf`, storedName: `e2e-chain-${index}-${token}.pdf`,
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

export async function libraryRows(session) {
  const r = await session.invoke("list_library_items", { includeDeleted: false });
  return r?.ok && Array.isArray(r.value) ? r.value : [];
}

export async function waitForRows(session, ids, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let rows = [];
  while (Date.now() < deadline) {
    rows = await libraryRows(session);
    if (ids.every((id) => rows.some((row) => row.id === id))) return rows;
    await sleep(1000);
  }
  return rows;
}

export async function setRowPart(session, itemId, part) {
  return await session.evaluate(`(() => {
    const el = document.querySelector('[data-item-id="${itemId}"] [data-testid="library-row-part-select"]');
    if (!el) return false;
    const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set;
    setter.call(el, '${part}'); el.dispatchEvent(new Event('change', { bubbles: true })); return true;
  })()`);
}

export async function ensureChecked(session, itemId) {
  return await session.evaluate(`(() => {
    const tr = document.querySelector('tr[data-item-id="${itemId}"]');
    const check = tr && tr.querySelector('input[type="checkbox"]');
    if (!check || check.disabled) return 'missing';
    if (!check.checked) check.click();
    return check.checked ? 'checked' : 'unchecked';
  })()`);
}

/** 组卷视图点发布 → 确认替换 → 等待结果文案。返回界面提示文本。 */
export async function publishCurrentSelection(session, label) {
  await session.waitFor(`document.querySelector('[data-testid="paper-publish"]') && !document.querySelector('[data-testid="paper-publish"]').disabled`,
    { timeoutMs: 15000, label: `${label}-publish-enabled` });
  await session.clickSelector('[data-testid="paper-publish"]');
  await session.waitFor(`!!document.querySelector('[data-testid="paper-confirm"]')`, { timeoutMs: 10000, label: `${label}-confirm` });
  await session.clickSelector('[data-testid="paper-confirm-ok"]');
  await session.waitFor(
    `(() => { const m = document.querySelector('[data-testid="paper-composer"] [role="status"]'); return !!m && /当前试卷已发布|未能发布|保持不变|旧试卷|没有更新|另一次发布/.test(m.textContent); })()`,
    { timeoutMs: 10 * 60 * 1000, intervalMs: 1000, label: `${label}-result` },
  );
  return await session.evaluate(`(document.querySelector('[data-testid="paper-composer"] [role="status"]')||{}).textContent || null`);
}

export function readPointer(nasRoot) {
  return JSON.parse(fs.readFileSync(path.join(nasRoot, "current-paper.json"), "utf8"));
}

export function readManifest(nasRoot) {
  const text = fs.readFileSync(path.join(nasRoot, "manifest.js"), "utf8");
  return JSON.parse(text.trim().replace(/^window\.__READING_EXAM_MANIFEST__\s*=\s*/, "").replace(/;\s*$/, ""));
}

/** 指针 ↔ 清单 ↔ 磁盘（与 tauri-cdp-paper-publish.mjs 同款校验）。 */
export function checkPointerOnDisk(nasRoot) {
  const pointer = readPointer(nasRoot);
  const manifest = readManifest(nasRoot);
  const problems = [];
  if (pointer.schemaVersion !== "CurrentPaperV1") problems.push(`schema=${pointer.schemaVersion}`);
  if (pointer.reading.length !== 3 || pointer.reading.map((e) => e.part).join(",") !== PARTS.join(",")) {
    problems.push(`reading parts=${pointer.reading.map((e) => e.part).join(",")}`);
  }
  for (const item of [...pointer.reading,...pointer.listening]) {
    const entry = item.entry;
    if (!entry) { problems.push(`${item.part} 缺 entry`); continue; }
    if (JSON.stringify(entry) !== JSON.stringify(manifest[item.examId])) problems.push(`${item.part} entry 与清单不一致`);
    if (item.script !== entry.script || item.scriptSha256 !== entry.checksums?.scriptSha256 || item.runtimeSha256 !== entry.checksums?.runtimeSha256) {
      problems.push(`${item.part} 顶层与 entry 不一致`);
    }
    const scriptFile = path.join(nasRoot, entry.script.replace(/^\.\//, ""));
    if (!fs.existsSync(scriptFile) || sha256File(scriptFile) !== item.scriptSha256) problems.push(`${item.part} 脚本字节校验失败`);
    const prefix = `./releases/${pointer.batchId}/resources/${item.examId}`;
    if (entry.resourcesBase !== prefix || entry.assetManifest !== `${prefix}/asset-manifest.json`) {
      problems.push(`${item.part} 资源未在批次目录：${entry.resourcesBase}`);
    }
    if (!fs.existsSync(path.join(nasRoot, entry.assetManifest.replace(/^\.\//, "")))) problems.push(`${item.part} asset-manifest 缺失`);
    if (fs.existsSync(path.join(nasRoot, "resources", item.examId))) problems.push(`${item.part} 根级 resources 被写入`);
  }
  return { ok: problems.length === 0, problems, pointer, manifest };
}

/**
 * 发布前稳定化：反复确认三篇仍处于勾选态（界面在 Part 变更 / 列表刷新后可能重置勾选）。
 * 返回 { stable, rechecks, states }；rechecks 为本次为恢复勾选而补点的次数，写入报告作为证据。
 */
export async function ensureAllCheckedStable(session, ids, { rounds = 6, settleMs = 1500 } = {}) {
  let rechecks = 0;
  let states = [];
  for (let round = 0; round < rounds; round++) {
    states = [];
    for (const id of ids) states.push(await ensureChecked(session, id));
    if (states.every((s) => s === "checked")) {
      await sleep(settleMs);
      const again = [];
      for (const id of ids) again.push(await session.evaluate(`(() => { const tr = document.querySelector('tr[data-item-id="${id}"]'); const c = tr && tr.querySelector('input[type="checkbox"]'); return !!c && c.checked; })()`));
      if (again.every(Boolean)) return { stable: true, rechecks, states: again };
      rechecks += again.filter((v) => !v).length;
      states = again.map((v) => (v ? "checked" : "unchecked"));
    } else {
      rechecks += states.filter((s) => s !== "checked").length;
    }
  }
  return { stable: false, rechecks, states };
}
