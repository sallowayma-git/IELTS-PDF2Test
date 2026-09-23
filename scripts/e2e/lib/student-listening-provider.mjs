// 学生端**真实代码**读取听力发布包：核心实现。
//
// 与阅读版（`lib/student-real-provider.mjs`）同源同理，区别有两点：
//
//  1. 走的入口是**听力**的：`NasJsDirectListeningAssetProvider.getAsset(examId)` →
//     `{ source, payload, audio }`，其中 `payload.parts[].media` 是**每个 part 自己的**
//     音频段，再逐个用 `getAssetBytes(examId, assetId)` 取字节。这正是 T6 的核心
//     ——「学生端逐 part 取到音频」——必须逐段核对 sha256，而不是只看「有一份音频」。
//
//  2. 默认读的是 **R2 worktree**（`feat-listening-per-part-media`），不是学生端主检出。
//     per-part media 与「听力卷不进阅读目录」这两条都只在这个 worktree 的编译产物里。
//     用主检出会得到一个「本来就没有这条链」的假失败。
//
// 与阅读版一样，这里直接 `require` 学生端仓库**已编译的真实产物**，不做镜像重写：
// 镜像只能证明「包符合规则」，证明不了「学生端那份代码真的能读」。

import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);

/** R2 worktree（per-part media 那条链在这里）。可用 `NAS_LISTENING_STUDENT_REPO` 覆盖。 */
export const DEFAULT_LISTENING_STUDENT_REPO =
  process.env.NAS_LISTENING_STUDENT_REPO ?? "F:/workspace/IELTS-NASfor-WenDao-listening";

/** 听力发布包的 `minimumRuntimeVersion` 是 1.0.0（`listening_source_v1.rs`）。 */
export const LISTENING_STUDENT_APP_VERSION = "1.0.0";

export function resolveListeningProviderPath(studentRepo) {
  return path.join(
    studentRepo,
    "server",
    "dist",
    "lib",
    "library",
    "listening",
    "NasJsDirectListeningAssetProvider.js",
  );
}

export function resolveReadingProviderPath(studentRepo) {
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
 * 真实 `ExamRuntimeConfig` 形状（与阅读版同一份约定）。
 *
 * `readingExamsRelative` 用 `.`：学生端真实默认值，含义是「papersRoot 自己就是
 * reading-exams 根」，于是 papersRoot 直接指向本仓发布的 `nas-library` 目录。
 * 听力 provider 用的也是同一对根，所以一份配置能同时喂给两个 provider。
 */
export function buildListeningRuntimeConfig(packageDir, appVersion) {
  return {
    mode: "practice",
    batchId: "student-listening-provider-load",
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
 * 用学生端真实 provider 加载发布出来的听力包。
 *
 * @param {object} opts
 * @param {string} opts.packageDir `nas-library` 目录
 * @param {string} [opts.studentRepo] 学生端 checkout（默认 R2 worktree）
 * @param {string} [opts.examId] 指定 examId；不传就用清单里第一条 `modality=listening`
 * @param {string} [opts.appVersion]
 * @param {Record<string,string>} [opts.expectedPartSha256] partId → 期望的音频 sha256。
 *        传了就必须逐条相等；不传只记录。
 *
 * 返回值的 `cause` 区分两种「跑不了」，调用方**必须**分开报：
 *   - `"no-listening-package"`：包/清单里没有听力条目 —— 上游发布没产出，是链条的失败。
 *   - 缺省（环境缺件）：学生端 provider 构建产物或包目录不存在 —— 真正的 cannot-run。
 */
export async function loadPublishedListeningPackageWithRealProviderAsync({
  packageDir,
  studentRepo = DEFAULT_LISTENING_STUDENT_REPO,
  examId: requestedExam = null,
  appVersion = LISTENING_STUDENT_APP_VERSION,
  expectedPartSha256 = null,
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
  const listeningProviderPath = resolveListeningProviderPath(studentRepo);
  const readingProviderPath = resolveReadingProviderPath(studentRepo);
  const base = {
    studentRepo,
    listeningProviderPath,
    readingProviderPath,
    packageDir: resolvedPackage,
    appVersion,
  };

  if (!fs.existsSync(listeningProviderPath)) {
    return {
      ...base,
      ok: false,
      cannotRun: true,
      reason:
        `学生端听力 provider 编译产物不存在：${listeningProviderPath}`
        + "（在 R2 worktree 里 npm run build:server，或用 NAS_LISTENING_STUDENT_REPO 指到已构建的 checkout）",
      results,
      failures: [],
    };
  }
  if (!fs.existsSync(readingProviderPath)) {
    return {
      ...base,
      ok: false,
      cannotRun: true,
      reason: `学生端阅读 provider 编译产物不存在：${readingProviderPath}`,
      results,
      failures: [],
    };
  }
  if (!resolvedPackage || !fs.existsSync(resolvedPackage)) {
    return {
      ...base,
      ok: false,
      cannotRun: true,
      cause: "no-listening-package",
      reason: `发布包目录不存在：${resolvedPackage ?? "(未提供)"}`,
      results,
      failures: [],
    };
  }
  const manifestPath = path.join(resolvedPackage, "manifest.js");
  if (!fs.existsSync(manifestPath)) {
    return {
      ...base,
      ok: false,
      cannotRun: true,
      cause: "no-listening-package",
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
      ...base,
      ok: false,
      cannotRun: true,
      reason: `manifest.js 解析失败：${error.message}`,
      results,
      failures: [],
    };
  }

  const entries = Object.entries(manifest).filter(([key]) => key !== "_meta");
  const listeningEntries = entries.filter(([, value]) => value?.modality === "listening");
  info("manifest-entries", `total=${entries.length} listening=${listeningEntries.length}`);
  if (listeningEntries.length === 0) {
    return {
      ...base,
      ok: false,
      cannotRun: true,
      // 包在、provider 在，只是清单里没有听力条目 —— 这是**上游发布没产出**，
      // 不是环境缺件。调用方必须据此报「发布没成功」，不能报成「学生端没构建」。
      cause: "no-listening-package",
      reason: `发布清单里没有任何 modality=listening 的条目：${entries.map(([key]) => key).join(", ")}`,
      results,
      failures: [],
    };
  }
  const chosen = requestedExam
    ? listeningEntries.find(([key, value]) => key === requestedExam || value.examId === requestedExam)
    : listeningEntries[0];
  if (!chosen) {
    return {
      ...base,
      ok: false,
      cannotRun: true,
      reason: `发布清单里没有 examId=${requestedExam} 的听力条目`,
      results,
      failures: [],
    };
  }
  const [manifestKey, manifestEntry] = chosen;
  const examId = manifestEntry.examId ?? manifestKey;
  info("exam-id", examId);
  info("manifest-entry", JSON.stringify({
    schemaVersion: manifestEntry.schemaVersion,
    modality: manifestEntry.modality,
    script: manifestEntry.script,
    assetManifest: manifestEntry.assetManifest,
  }));

  let NasJsDirectListeningAssetProvider;
  let NasJsDirectReadingAssetProvider;
  try {
    ({ NasJsDirectListeningAssetProvider } = require(listeningProviderPath));
  } catch (error) {
    return {
      ...base,
      ok: false,
      cannotRun: true,
      reason: `学生端听力 provider 加载失败：${error.message}`,
      results,
      failures: [],
    };
  }
  try {
    ({ NasJsDirectReadingAssetProvider } = require(readingProviderPath));
  } catch (error) {
    return {
      ...base,
      ok: false,
      cannotRun: true,
      reason: `学生端阅读 provider 加载失败：${error.message}`,
      results,
      failures: [],
    };
  }
  info("provider-modules", `${listeningProviderPath} + ${readingProviderPath}`);

  const observed = { examId, parts: [], readingAssetIds: [] };
  const config = buildListeningRuntimeConfig(resolvedPackage, appVersion);

  let listeningProvider;
  try {
    listeningProvider = new NasJsDirectListeningAssetProvider(config);
  } catch (error) {
    check("listening-provider-constructs", false, error.message);
  }

  let loaded = null;
  if (listeningProvider) {
    check("listening-provider-constructs", true, "constructed");
    try {
      loaded = await listeningProvider.getAsset(examId);
      check("listening-getAsset-does-not-throw", true, "ok");
    } catch (error) {
      check("listening-getAsset-does-not-throw", false, error.message);
    }
  }

  if (loaded) {
    const source = loaded.source ?? {};
    const payload = loaded.payload ?? {};
    check("payload-modality-is-listening", payload.modality === "listening", `modality=${payload.modality}`);
    check(
      "source-schema-version",
      source.schemaVersion === "ListeningExamSourceV1",
      `schemaVersion=${source.schemaVersion}`,
    );
    const parts = Array.isArray(payload.parts) ? payload.parts : [];
    check("payload-has-four-parts", parts.length === 4, `parts=${parts.length}`);
    check("exam-level-media-is-absent", !payload.media, `media=${payload.media ? "present" : "absent"}`);
    const numbers = parts.flatMap((part) => part.expectedQuestionNumbers ?? []);
    check("parts-cover-questions-one-to-forty", parts.length === 4
      && numbers.length === 40
      && new Set(numbers).size === 40
      && Math.min(...numbers) === 1
      && Math.max(...numbers) === 40, `questions=${numbers.join(",")}`);
    observed.partCount = parts.length;
    observed.questionNumbers = numbers.length;

    const assetManifest = payload.assetManifest ?? { assets: {} };
    for (const part of parts) {
      const media = part.media ?? null;
      const row = {
        partId: part.partId,
        assetId: media?.assetId ?? null,
        sha256: media?.sha256 ?? null,
        mime: media?.mime ?? null,
        durationMs: media?.durationMs ?? null,
        probeStatus: media?.probe?.status ?? null,
        bytesSha256: null,
        bytes: null,
      };
      check(
        `part-${part.partId}-carries-its-own-media`,
        Boolean(media?.assetId && media?.sha256),
        `media=${media ? JSON.stringify({ assetId: media.assetId, sha256: String(media.sha256).slice(0, 12) }) : "missing"}`,
      );
      check(
        `part-${part.partId}-probe-passed`,
        media?.probe?.status === "passed" && (media?.probe?.issueCodes ?? []).length === 0,
        `probe=${JSON.stringify(media?.probe ?? null)}`,
      );
      check(
        `part-${part.partId}-declared-in-asset-manifest`,
        Boolean(assetManifest.assets?.[media?.assetId]),
        `assetId=${media?.assetId}`,
      );
      // 逐 part 取字节：这才是「学生端逐 part 取到音频」的真正证据。
      if (media?.assetId) {
        try {
          const bytes = await listeningProvider.getAssetBytes(examId, media.assetId);
          const { createHash } = await import("node:crypto");
          const actual = createHash("sha256").update(bytes.bytes).digest("hex");
          row.bytesSha256 = actual;
          row.bytes = bytes.bytes.length;
          check(
            `part-${part.partId}-bytes-match-declared-hash`,
            actual.toLowerCase() === String(media.sha256).toLowerCase(),
            `bytes=${actual.slice(0, 12)} declared=${String(media.sha256).slice(0, 12)}`,
          );
        } catch (error) {
          check(`part-${part.partId}-bytes-match-declared-hash`, false, error.message);
        }
      }
      observed.parts.push(row);
    }

    const distinctHashes = new Set(observed.parts.map((row) => row.sha256));
    check(
      "four-parts-are-four-distinct-files",
      distinctHashes.size === 4,
      `distinct=${distinctHashes.size}`,
    );

    if (expectedPartSha256) {
      for (const row of observed.parts) {
        const expected = expectedPartSha256[row.partId] ?? null;
        check(
          `part-${row.partId}-sha256-is-the-bound-file`,
          Boolean(expected) && expected.toLowerCase() === String(row.sha256).toLowerCase(),
          expected ? `expected=${expected.slice(0, 12)} actual=${String(row.sha256).slice(0, 12)}` : "没有为这个 part 记录期望哈希",
        );
      }
    }
  }

  // R2 复证：听力卷不得出现在学生端「阅读」练习目录里。
  let readingProvider = null;
  try {
    readingProvider = new NasJsDirectReadingAssetProvider(config);
    const assets = await readingProvider.listAssets();
    const status = await readingProvider.getStatus();
    observed.readingAssetIds = assets.map((asset) => asset.id);
    check(
      "reading-library-excludes-the-listening-exam",
      !observed.readingAssetIds.includes(examId),
      `ids=${observed.readingAssetIds.join(",") || "(empty)"}`,
    );
    check(
      "reading-count-matches-reading-list",
      status.assetCount === assets.length,
      `assetCount=${status.assetCount} listAssets=${assets.length}`,
    );
  } catch (error) {
    check("reading-library-excludes-the-listening-exam", false, `阅读 provider 读取失败：${error.message}`);
  }

  const failures = results.filter((entry) => !entry.ok);
  return {
    ...base,
    ok: failures.length === 0,
    cannotRun: false,
    reason: null,
    examId,
    results,
    failures,
    observed,
  };
}
