// 学生端**真实代码**读取：核心实现。
//
// 被两处复用，避免出现「CLI 跑的是真的、链路里跑的是另一套」：
//   - `scripts/e2e/student-real-provider-load.mjs`（CLI，单独验收）
//   - `scripts/e2e/tauri-cdp-cloud-repair-chain.mjs`（真实链路里，作为导出后的最后一跳）
//
// 为什么不是镜像：`nas-student-contract.mjs` 按学生端规则**重新实现**了一遍校验，
// 只能证明「包符合规则」。这里直接 require 学生端仓库已编译的真实产物
// `<student-repo>/server/dist/lib/library/reading/NasJsDirectReadingAssetProvider.js`，
// 用真实 `ExamRuntimeConfig` 把发布包当 NAS 挂上去，跑真实入口。
//
// 覆盖边界：证明**服务端读取层**能加载发布包，且加载出的内容与云端修复后的 canonical
// 一致；**不**证明 Electron 学生端渲染/作答（属 M6）。

import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

export const DEFAULT_STUDENT_REPO =
  process.env.NAS_STUDENT_REPO ?? "F:/workspace/IELTS-NASfor-WenDao";

export function resolveProviderPath(studentRepo) {
  return path.join(
    studentRepo,
    "server",
    "dist",
    "lib",
    "library",
    "reading",
    "NasJsDirectReadingAssetProvider.js",
  );
}

/**
 * 构造真实 provider 需要的 `ExamRuntimeConfig` 形状。
 *
 * `readingExamsRelative` 用 `.`：这是学生端真实默认值
 * （`server/src/lib/library/exam-runtime.ts:6 DEFAULT_READING_EXAMS_RELATIVE = '.'`），
 * 含义是「papersRoot 自己就是 reading-exams 根」，于是 papersRoot 可以直接指向
 * 本仓发布出来的 `nas-library` 目录。
 */
export function buildRuntimeConfig(packageDir, appVersion) {
  return {
    mode: "practice",
    batchId: "student-real-provider-load",
    candidateId: null,
    candidateName: null,
    terminalId: "harness",
    appVersion,
    nas: {
      provider: "nas-js-direct",
      papersRoot: packageDir,
      submissionsRoot: packageDir,
      readingExamsRelative: ".",
      readingExplanationsRelative: ".",
      writingExamsRelative: ".",
      resourcesRelative: "resources",
      allowLocalCache: false,
    },
    storagePolicy: {},
  };
}

function parseManifest(source) {
  const match = source.match(/__READING_EXAM_MANIFEST__\s*=\s*([\s\S]*?);?\s*$/u);
  return JSON.parse(match ? match[1].trim().replace(/;\s*$/, "") : source);
}

/**
 * 异步版本——真实入口全是 async，链路里用的是这个。
 */
export async function loadPublishedPackageWithRealProviderAsync({
  packageDir,
  studentRepo = DEFAULT_STUDENT_REPO,
  examId: requestedExam = null,
  appVersion = "0.2.0",
}) {
  const results = [];
  const check = (name, ok, detail) => {
    results.push({ name, ok: Boolean(ok), detail: detail ?? null });
    return Boolean(ok);
  };
  const info = (name, detail) => {
    results.push({ name, ok: true, info: true, detail: detail ?? null });
  };

  const resolvedPackage = packageDir ? path.resolve(packageDir) : null;
  const providerPath = resolveProviderPath(studentRepo);

  if (!fs.existsSync(providerPath)) {
    return {
      ok: false,
      cannotRun: true,
      reason: `学生端编译产物不存在：${providerPath}（先在该仓 npm run build:server，或设置 NAS_STUDENT_REPO）`,
      results,
      failures: [],
    };
  }
  if (!resolvedPackage || !fs.existsSync(resolvedPackage)) {
    return {
      ok: false,
      cannotRun: true,
      reason: `发布包目录不存在：${resolvedPackage ?? "(未提供)"}`,
      results,
      failures: [],
    };
  }
  const manifestPath = path.join(resolvedPackage, "manifest.js");
  if (!fs.existsSync(manifestPath)) {
    return {
      ok: false,
      cannotRun: true,
      reason: `发布包里没有 manifest.js：${manifestPath}`,
      results,
      failures: [],
    };
  }

  let manifest;
  try {
    manifest = parseManifest(fs.readFileSync(manifestPath, "utf8"));
  } catch (error) {
    return {
      ok: false,
      cannotRun: true,
      reason: `manifest.js 解析失败：${error.message}`,
      results,
      failures: [],
    };
  }
  const entries = Object.entries(manifest).filter(([key]) => key !== "_meta");
  if (entries.length === 0) {
    return {
      ok: false,
      cannotRun: true,
      reason: "manifest.js 里没有任何资产条目",
      results,
      failures: [],
    };
  }
  const examId = requestedExam || entries[0][1].examId || entries[0][0];
  info("package-dir", resolvedPackage);
  info("exam-id", examId);
  info("manifest-entry-script", entries[0][1].script ?? null);

  let NasJsDirectReadingAssetProvider;
  try {
    ({ NasJsDirectReadingAssetProvider } = require(providerPath));
  } catch (error) {
    return {
      ok: false,
      cannotRun: true,
      reason: `学生端 provider 加载失败：${error.message}`,
      results,
      failures: [],
    };
  }
  info("provider-module", providerPath);

  const provider = new NasJsDirectReadingAssetProvider(buildRuntimeConfig(resolvedPackage, appVersion));

  let status = null;
  try {
    status = await provider.getStatus();
  } catch (error) {
    check("getStatus-does-not-throw", false, error.message);
  }
  if (status) {
    check("getStatus-ready", status.ready === true, `ready=${status.ready} errorCode=${status.errorCode}`);
    check("getStatus-source", status.source === "nas-js-direct", `source=${status.source}`);
    check("getStatus-assetCount>=1", status.assetCount >= 1, `assetCount=${status.assetCount}`);
  }

  let assets = [];
  try {
    assets = await provider.listAssets();
  } catch (error) {
    check("listAssets-does-not-throw", false, error.message);
  }
  check(
    "listAssets-finds-exam",
    assets.some((asset) => asset.id === examId),
    `ids=${assets.map((asset) => asset.id).join(",")}`,
  );

  const summary = assets.find((asset) => asset.id === examId) ?? null;
  if (summary) {
    check(
      "listAssets-reports-v2-schema",
      summary.metadata?.runtimeSchemaVersion === "ReadingExamSourceV2",
      `runtimeSchemaVersion=${summary.metadata?.runtimeSchemaVersion}`,
    );
    check("listAssets-title", Boolean(summary.title), `title=${summary.title}`);
  }

  let detail = null;
  try {
    detail = await provider.getAsset(examId);
  } catch (error) {
    check("getAsset-does-not-throw", false, error.message);
  }

  const observed = {};
  if (detail) {
    const payload = detail.payload ?? {};
    check(
      "getAsset-schema-version",
      payload.schemaVersion === "ReadingExamSourceV2",
      `schemaVersion=${payload.schemaVersion}`,
    );
    check("getAsset-exam-id", payload.examId === examId, `examId=${payload.examId}`);
    check("getAsset-has-runtime-source", Boolean(payload.runtimeSourceV2), "runtimeSourceV2 present");

    const source = payload.runtimeSourceV2 ?? null;
    const questionCount = payload.questionCount ?? 0;
    check("getAsset-question-count>0", questionCount > 0, `questionCount=${questionCount}`);
    observed.questionCount = questionCount;

    if (source) {
      const taskGroups = Array.isArray(source.taskGroups) ? source.taskGroups : [];
      check("getAsset-task-groups>0", taskGroups.length > 0, `taskGroups=${taskGroups.length}`);
      observed.taskGroups = taskGroups.length;

      // 云端自行修掉的那处：题面尾部的分页残留 `… must 14 BLANK PAGE` 必须已经不在。
      const prompts = taskGroups.flatMap((group) =>
        (group.responseGroups ?? []).map((response) => String(response.prompt ?? "")),
      );
      const footerResidue = prompts.filter((prompt) => /\d+\s+BLANK\s+PAGE\s*$/iu.test(prompt));
      check(
        "cloud-fix-survives-into-student-payload",
        footerResidue.length === 0,
        footerResidue.length
          ? `仍有分页残留：${footerResidue[0].slice(-40)}`
          : `${prompts.length} 条题面，无分页残留`,
      );
      observed.promptCount = prompts.length;
      observed.footerResidueCount = footerResidue.length;

      const answerKey = payload.answerKey ?? {};
      const slotIds = source.questionOrder ?? [];
      const missing = slotIds.filter((slotId) => !(slotId in answerKey));
      check(
        "getAsset-answer-key-covers-all-slots",
        slotIds.length > 0 && missing.length === 0,
        `slots=${slotIds.length} missing=${missing.join(",") || "none"}`,
      );
      observed.slotCount = slotIds.length;

      const instructionTexts = taskGroups.map((group) => String(group.instructions ?? ""));
      check(
        "getAsset-instructions-present",
        instructionTexts.some((text) => text.trim().length > 0),
        `${instructionTexts.filter((text) => text.trim()).length}/${instructionTexts.length} 组有 instructions`,
      );
    }
  }

  // 反向对照：真实 provider 会按 `minimumRuntimeVersion` 过滤掉版本不达标的资产。
  // 这一条用来证明上面跑的是**真实代码**，而不是被 stub 掉的成功路径。
  const lowProvider = new NasJsDirectReadingAssetProvider(buildRuntimeConfig(resolvedPackage, "0.1.0"));
  const lowAssets = await lowProvider.listAssets().catch(() => []);
  check(
    "negative-control-low-runtime-version-filters-asset",
    lowAssets.length === 0,
    `assets=${lowAssets.length}`,
  );

  const failures = results.filter((entry) => !entry.ok);
  return {
    ok: failures.length === 0,
    cannotRun: false,
    reason: null,
    examId,
    providerPath,
    packageDir: resolvedPackage,
    appVersion,
    results,
    failures,
    observed,
  };
}
