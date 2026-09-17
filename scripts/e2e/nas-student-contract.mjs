#!/usr/bin/env node
// 跨仓 NAS 学生端契约入口（M0-T5 / 计划 §26.4 攻击问题 A 的第一层落地）。
//
// 校验对象：本仓 publisher（nas_package_v2）真实产出的发布包。默认目录
//   artifacts/nas-contract-fixture
// （由 `cargo test … dump_published_package_for_nas_contract -- --ignored` 落盘），
// 也可用 `--package <dir>` 直接指向一次真实 UI 发布的产物（例如
// `artifacts/e2e-cdp/run-publish-ready-*/nas-library`）。
//
// 校验规则：**忠实镜像**学生端 `NasJsDirectReadingAssetProvider`
// （F:/workspace/IELTS-NASfor-WenDao/server/src/lib/library/reading/NasJsDirectReadingAssetProvider.ts）
// 与 `nas-path-policy.ts` / `generated-json.ts` 的既有规则。
//
// R14 修正（重要）：上一版只检查「包根目录存在 `<examId>.js`」，那是**旧布局**。
// 真实学生端从 `manifest.entry.script` 解析运行时脚本（`…Provider.ts:114` 与 `:236`），
// 当前 publisher 把它放在 `./releases/<batchId>/<examId>.js`。于是旧版会对**正确的**
// 发布包报 `runtime-js-present: FAIL` —— 一个假阴性，等于这套契约根本回答不了
// 「学生端能不能读我们的导出」。本版按真实规则解析 `entry.script`，并补上真实学生端
// 会做的两道完整性绑定：`checksums.scriptSha256`（整份脚本文本）与
// `checksums.runtimeSha256`（脚本内 `__READING_EXAM_DATA__.register(key, payload)` 的
// payload 的 canonical JSON）。根目录 `<examId>.js` 不再作为通过条件，只作提示。
//
// 覆盖层级声明：本脚本只证明「发布包能通过学生端读取规则」；
// Electron 学生端真实加载与作答一致性属于 M6 的 NAS 实测，当前如实标记 pending。
// 本仓另有学生端仓库自带的真实代码验收 `npm run verify:cross-repo-reading-v2`
// （用学生端的 test-only loader 真跑一遍），两者互补。
//
//   node scripts/e2e/nas-student-contract.mjs [--package dir] [--student-repo dir] [--out report.json]

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const args = process.argv.slice(2);
function argOf(name, fallback) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : fallback;
}
const packageDir = path.resolve(argOf("--package", path.join(repoRoot, "artifacts", "nas-contract-fixture")));
const studentRepo = path.resolve(
  argOf("--student-repo", process.env.NAS_STUDENT_REPO ?? "F:/workspace/IELTS-NASfor-WenDao")
);
const outPath = argOf("--out", null);

const READING_V2_SCHEMA_VERSION = "ReadingExamSourceV2";
const results = [];
function check(name, ok, detail) {
  results.push({ name, ok: Boolean(ok), detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` :: ${detail}` : ""}`);
  return Boolean(ok);
}
function info(name, detail) {
  results.push({ name, ok: true, info: true, detail });
  console.log(`INFO ${name}${detail ? ` :: ${detail}` : ""}`);
}

function sha256(content) {
  return crypto.createHash("sha256").update(content).digest("hex");
}

/** manifest.js 是 `window.__READING_EXAM_MANIFEST__ = {...};` 形态的 JSONP。 */
function readLibraryManifest(file) {
  const raw = fs.readFileSync(file, "utf8");
  const start = raw.indexOf("{");
  const end = raw.lastIndexOf("}");
  if (start < 0 || end <= start) throw new Error("manifest.js 不含 JSON 对象");
  return JSON.parse(raw.slice(start, end + 1));
}

/** 镜像 `normalizeNasRelativePath`（nas-path-policy.ts:19）：只接受相对、无 `..`、无盘符的路径。 */
function isSafeRelativeNasPath(value) {
  if (typeof value !== "string" || !value.trim()) return false;
  const normalized = value.trim().replace(/\\/g, "/");
  if (normalized.includes("\0") || normalized.includes(":")) return false;
  if (path.win32.isAbsolute(normalized) || path.posix.isAbsolute(normalized)) return false;
  const posix = path.posix.normalize(normalized);
  if (!posix || posix === ".." || posix.startsWith("../") || posix.split("/").includes("..")) return false;
  return true;
}

/** 镜像 `canonicalJson`（NasJsDirectReadingAssetProvider.ts:159）：对象键排序后紧凑序列化。 */
function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

/**
 * 镜像 `parseGeneratedRegisterSource`（generated-json.ts:218）：
 * 生成脚本是 `__READING_EXAM_DATA__.register("<key>", {…payload…})`。
 * 只解析这一个 register 调用的 key 与 payload。
 */
function parseGeneratedRegisterSource(source, registryName) {
  const registerPattern = new RegExp(`${registryName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\.register\\s*\\(`);
  const registerCall = source.match(registerPattern);
  if (!registerCall || registerCall.index == null) return null;
  let cursor = registerCall.index + registerCall[0].length;
  const skipWs = () => { while (cursor < source.length && /\s/u.test(source[cursor])) cursor += 1; };
  skipWs();
  if (source[cursor] !== '"') return null;
  let end = cursor + 1;
  while (end < source.length && source[end] !== '"') {
    if (source[end] === "\\") end += 1;
    end += 1;
  }
  let key;
  try {
    key = JSON.parse(source.slice(cursor, end + 1));
  } catch {
    return null;
  }
  cursor = end + 1;
  skipWs();
  if (source[cursor] !== ",") return null;
  cursor += 1;
  skipWs();
  if (source[cursor] !== "{") return null;
  // 从 payload 起点做括号配对，取出整段 JSON 文本再解析（比手写 tokenizer 稳）。
  let depth = 0;
  let inString = false;
  let escaped = false;
  const start = cursor;
  for (; cursor < source.length; cursor += 1) {
    const ch = source[cursor];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') { inString = true; continue; }
    if (ch === "{") depth += 1;
    else if (ch === "}") {
      depth -= 1;
      if (depth === 0) { cursor += 1; break; }
    }
  }
  let payload;
  try {
    payload = JSON.parse(source.slice(start, cursor));
  } catch {
    return null;
  }
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) return null;
  return { key, payload };
}

function main() {
  let ok = true;
  ok = check("student-repo-found", fs.existsSync(studentRepo), studentRepo) && ok;
  const studentHasNodeModules = fs.existsSync(path.join(studentRepo, "node_modules"));
  info(
    "student-repo-node-modules",
    `${studentHasNodeModules ? "installed" : "absent"}；Electron 真实加载实测安排在 M6（需桌面运行），当前不算验收通过；`
      + "本仓另有学生端仓库自带的 `npm run verify:cross-repo-reading-v2`（真实 loader）"
  );

  const manifestPath = path.join(packageDir, "manifest.js");
  if (!fs.existsSync(manifestPath)) {
    check("manifest.js-exists", false, `${manifestPath}；先运行 dump_published_package_for_nas_contract (--ignored)`);
    return finish(false);
  }
  ok = check("manifest.js-exists", true, packageDir) && ok;

  let library;
  try {
    library = readLibraryManifest(manifestPath);
    check("manifest.js-parses", true);
  } catch (error) {
    check("manifest.js-parses", false, String(error));
    return finish(false);
  }

  ok = check(
    "library-schemaVersion-is-ReadingExamManifestV2",
    library._meta?.schemaVersion === "ReadingExamManifestV2",
    String(library._meta?.schemaVersion)
  ) && ok;

  const examIds = Object.keys(library).filter((key) => key !== "_meta");
  ok = check("at-least-one-exam", examIds.length > 0, examIds.join(", ")) && ok;

  for (const examId of examIds) {
    const entry = library[examId];
    ok = check(`exam[${examId}].id-shape`, /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/u.test(examId)) && ok;

    const isV2 = entry.schemaVersion === READING_V2_SCHEMA_VERSION;
    // V2 条目：学生端要求 resourcesBase + assetManifest 同时存在，否则直接抛错。
    if (isV2) {
      ok = check(`exam[${examId}].v2-requires-resourcesBase`, isSafeRelativeNasPath(entry.resourcesBase), String(entry.resourcesBase)) && ok;
      ok = check(`exam[${examId}].v2-requires-assetManifest`, isSafeRelativeNasPath(entry.assetManifest), String(entry.assetManifest)) && ok;
    }

    // ---- 运行时脚本：按 `entry.script` 解析（真实学生端 :114 / :236）----
    const scriptRel = typeof entry.script === "string" && entry.script.trim()
      ? entry.script.trim().replace(/^\.\//u, "")
      : `${examId}.js`;
    const scriptSafe = isSafeRelativeNasPath(scriptRel);
    ok = check(`exam[${examId}].script-path-safe`, scriptSafe, scriptRel) && ok;
    if (scriptSafe) {
      const scriptPath = path.join(packageDir, scriptRel);
      const scriptExists = fs.existsSync(scriptPath);
      ok = check(`exam[${examId}].script-exists`, scriptExists, scriptRel) && ok;
      if (scriptExists) {
        const scriptSource = fs.readFileSync(scriptPath, "utf8");
        const expectedScriptSha = entry.checksums?.scriptSha256;
        if (isV2) {
          ok = check(
            `exam[${examId}].script-sha256-shape`,
            /^[a-f0-9]{64}$/iu.test(String(expectedScriptSha ?? ""))
          ) && ok;
        }
        if (expectedScriptSha) {
          ok = check(
            `exam[${examId}].script-sha256`,
            sha256(Buffer.from(scriptSource, "utf8")) === String(expectedScriptSha).toLowerCase(),
            scriptRel
          ) && ok;
        }
        // 脚本里的 data key 必须与 manifest 的 assetId/examId/dataKey 之一一致（学生端 :243）。
        const parsed = parseGeneratedRegisterSource(scriptSource, "__READING_EXAM_DATA__");
        ok = check(`exam[${examId}].script-register-parses`, Boolean(parsed), scriptRel) && ok;
        if (parsed) {
          const accepted = [entry.assetId, entry.examId, entry.dataKey].filter(Boolean).map(String);
          ok = check(
            `exam[${examId}].script-key-matches`,
            accepted.includes(String(parsed.key)),
            `脚本 key=${JSON.stringify(parsed.key)}，manifest 接受=${JSON.stringify(accepted)}`
          ) && ok;
          if (isV2) {
            const expectedRuntimeSha = entry.checksums?.runtimeSha256;
            ok = check(
              `exam[${examId}].runtime-sha256-shape`,
              /^[a-f0-9]{64}$/iu.test(String(expectedRuntimeSha ?? ""))
            ) && ok;
            if (expectedRuntimeSha) {
              ok = check(
                `exam[${examId}].runtime-sha256`,
                sha256(Buffer.from(canonicalJson(parsed.payload), "utf8")) === String(expectedRuntimeSha).toLowerCase(),
                "canonicalJson(payload)"
              ) && ok;
            }
          }
        }
      }
    }
    // 旧布局（包根 `<examId>.js`）不再是通过条件：真实学生端不读它。只作提示，
    // 免得下次有人看到它消失就以为是回归。
    const legacyRootScript = path.join(packageDir, `${examId}.js`);
    info(`exam[${examId}].legacy-root-script`, fs.existsSync(legacyRootScript) ? "存在（旧布局，学生端不依赖）" : "不存在（当前布局，学生端不依赖）");

    // ---- 资源清单 ----
    const assetManifestRel = entry.assetManifest ?? "";
    const assetManifestPath = path.resolve(packageDir, assetManifestRel);
    ok = check(
      `exam[${examId}].asset-manifest-exists`,
      assetManifestRel.startsWith("./resources/")
        && !assetManifestRel.includes("..")
        && fs.existsSync(assetManifestPath),
      assetManifestRel
    ) && ok;
    if (!fs.existsSync(assetManifestPath)) continue;

    const expectedSha = entry.checksums?.assetManifestSha256;
    if (expectedSha) {
      ok = check(
        `exam[${examId}].asset-manifest-sha256`,
        sha256(fs.readFileSync(assetManifestPath)) === expectedSha
      ) && ok;
    }

    let assetManifest;
    try {
      assetManifest = JSON.parse(fs.readFileSync(assetManifestPath, "utf8"));
      check(`exam[${examId}].asset-manifest-parses`, true);
    } catch (error) {
      check(`exam[${examId}].asset-manifest-parses`, false, String(error));
      ok = false;
      continue;
    }

    // ---- 镜像 validateReadingV2AssetManifest（reading-asset-resolver.ts:73）----
    ok = check(
      `exam[${examId}].schemaVersion-is-ExamAssetManifestV2`,
      assetManifest.schemaVersion === "ExamAssetManifestV2",
      String(assetManifest.schemaVersion)
    ) && ok;
    ok = check(
      `exam[${examId}].manifest-examId-matches`,
      assetManifest.examId === examId,
      String(assetManifest.examId)
    ) && ok;

    const assets = assetManifest.assets ?? null;
    ok = check(`exam[${examId}].assets-map-present`, Boolean(assets) && typeof assets === "object") && ok;
    if (!assets) continue;

    for (const [assetId, descriptor] of Object.entries(assets)) {
      ok = check(`asset[${assetId}].id-consistent`, descriptor.assetId === assetId) && ok;
      ok = check(
        `asset[${assetId}].sha256-shape`,
        /^[a-f0-9]{64}$/iu.test(descriptor.sha256 ?? "")
      ) && ok;
      ok = check(
        `asset[${assetId}].byteLength-shape`,
        Number.isInteger(descriptor.byteLength) && descriptor.byteLength >= 0
      ) && ok;
      const relative = descriptor.relativePath ?? "";
      ok = check(
        `asset[${assetId}].path-safe`,
        typeof relative === "string"
          && relative.length > 0
          && !path.isAbsolute(relative)
          && !relative.split(/[\\/]/).includes(".."),
        relative
      ) && ok;
      ok = check(
        `asset[${assetId}].mime-whitelisted`,
        /^(?:image\/|audio\/|application\/octet-stream$)/iu.test(descriptor.mime ?? ""),
        descriptor.mime
      ) && ok;

      // 学生端 loader 按 sha256/byteLength 校验磁盘内容；这里提前做同样的事。
      const absolute = path.join(packageDir, relative);
      if (fs.existsSync(absolute)) {
        ok = check(`asset[${assetId}].file-sha256`, sha256(fs.readFileSync(absolute)) === descriptor.sha256) && ok;
        ok = check(
          `asset[${assetId}].file-byteLength`,
          fs.statSync(absolute).size === descriptor.byteLength
        ) && ok;
      } else {
        ok = check(`asset[${assetId}].file-exists`, false, absolute) && ok;
      }
    }
  }

  return finish(ok);
}

function finish(ok) {
  const graded = results.filter((r) => !r.info);
  console.log(`\nnas-student-contract: ${ok ? "PASS" : "FAIL"} (${graded.filter((r) => r.ok).length}/${graded.length})`);
  if (outPath) {
    fs.mkdirSync(path.dirname(path.resolve(outPath)), { recursive: true });
    fs.writeFileSync(
      path.resolve(outPath),
      JSON.stringify(
        { packageDir, studentRepo, verdict: ok ? "passed" : "failed", checkedAt: new Date().toISOString(), results },
        null,
        2
      )
    );
  }
  process.exit(ok ? 0 : 1);
}

main();
