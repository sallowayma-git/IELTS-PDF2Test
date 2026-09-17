// 「proven-ready 题稿」夹具的共享构造。
//
// 从 `tauri-publish-ready.mjs` 原样抽出（不修改那份脚本的行为），供 CDP 版发布验收复用。
// 内容一字未改：数据构造与 `product_chain.rs::physical_shadow_for` 一致，
// 应用启动迁移会自动 seed canonical。
//
// 为什么必须共享而不是各写一份：这份阴影是「导出会重算质量」的输入，
// 两份实现一旦漂移，就会出现「A 脚本能发布、B 脚本发布不了」这种查不出原因的差异。

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

export const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
export const READY_AUTHORING = path.join(
  repoRoot,
  "fixtures",
  "golden",
  "synthetic",
  "ielts",
  "early-approaches-authoring-v2.json"
);

/** 复现 product_chain.rs::physical_shadow_for（导出会重算质量，错了会被门禁拦截——
 * 这正是本套件的验证点之一，无需信任复制本身）。 */
export function physicalShadowFor(authoring, jobId) {
  const nodeIds = new Set();
  const walk = (value) => {
    if (Array.isArray(value)) {
      for (const item of value) walk(item);
      return;
    }
    if (value && typeof value === "object") {
      if (Array.isArray(value.sourceAnchors)) {
        for (const anchor of value.sourceAnchors) {
          for (const id of anchor?.nodeIds ?? []) nodeIds.add(id);
        }
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
    sourceFiles: [{
      sourceFileId,
      originalName: "early-approaches.pdf",
      mediaType: "application/pdf",
      sha256: sourceHash,
      byteLength: 1,
      role: "question_paper"
    }],
    pages: [{
      pageIndex: 0,
      widthPt: 612.0,
      heightPt: 792.0,
      rotation: 0,
      glyphs: [],
      spans: [],
      lines: [],
      regions: [{
        id: "region-question-surface",
        kind: "text",
        bbox: { x: 10.0, y: 10.0, width: 500.0, height: 200.0, unit: "pt", origin: "top-left", pageRotation: 0 },
        childLineIds: [...nodeIds].sort(),
        childObjectIds: [],
        confidence: 1.0,
        sourceAnchors: [{
          sourceFileId,
          pageIndex: 0,
          nodeIds: ["region-question-surface"],
          extractionMode: "pdf_native",
          sourceHash
        }]
      }],
      vectorPaths: [],
      tables: [],
      assetIds: [],
      readingOrder: ["region-question-surface"],
      quality: {
        classification: "born_digital",
        nativeCharacterCount: 100,
        unicodeErrorRatio: 0.0,
        duplicateTextRatio: 0.0,
        imageCoverageRatio: 0.0,
        textCoverageRatio: 1.0,
        rotationConfidence: 1.0,
        requiresOcrRegions: []
      }
    }],
    assets: [],
    extraction: {
      engine: "e2e-publish-ready-fixture",
      engineVersion: "1.0.0",
      extractedAt: "2026-01-01T00:00:00Z",
      warnings: []
    }
  };
}

/** 启动前预置 job 目录：product_chain 同款 ready 数据，应用迁移会自动 seed canonical。 */
export function seedReadyJob(dataDir) {
  const jobId = `e2e-publish-ready-${crypto.randomBytes(4).toString("hex")}`;
  const jobDir = path.join(dataDir, "jobs", jobId);
  fs.mkdirSync(jobDir, { recursive: true });
  const authoring = JSON.parse(fs.readFileSync(READY_AUTHORING, "utf8"));
  authoring.jobId = jobId;
  const now = "2026-09-12T00:00:00Z";
  const job = {
    jobId,
    title: "E2E Publish Ready",
    status: "Working",
    category: "P1",
    frequency: "medium",
    tags: ["e2e-publish-ready"],
    sourceFiles: [{
      fileId: "file-e2e-publish-ready",
      originalName: "early-approaches.pdf",
      storedName: "e2e-publish-ready.pdf",
      fileType: "pdf",
      sha256: "a".repeat(64),
      sizeBytes: 1,
      role: "MainQuestion",
      importedAt: now
    }],
    activeLlmProfileId: null,
    createdAt: now,
    updatedAt: now,
    currentStep: "Preview",
    issueCounts: { errors: 0, warnings: 0, needsReview: 0 }
  };
  fs.writeFileSync(path.join(jobDir, "job.json"), JSON.stringify(job, null, 2));
  fs.writeFileSync(path.join(jobDir, "authoring-ir-v2.shadow.json"), JSON.stringify(authoring, null, 2));
  fs.writeFileSync(path.join(jobDir, "document-ir-v2.shadow.json"), JSON.stringify(physicalShadowFor(authoring, jobId), null, 2));
  return { jobId, title: job.title };
}
