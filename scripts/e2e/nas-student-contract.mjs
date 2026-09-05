#!/usr/bin/env node
// 跨仓 NAS 学生端契约入口（M0-T5 / 计划 §26.4 攻击问题 A 的第一层落地）。
//
// 校验对象：本仓 publisher（nas_package_v2）真实产出的发布包，固定目录来自
//   cargo test --manifest-path src-tauri/Cargo.toml dump_published_package_for_nas_contract -- --ignored --nocapture
// 校验规则：镜像学生端 `validateReadingV2AssetManifest`（F:/workspace/IELTS-NASfor-WenDao/
//   server/src/lib/library/reading/reading-asset-resolver.ts:73）的既有规则——schemaVersion/examId/
//   asset sha256/byteLength/mime 白名单/路径安全。镜像与权威的收敛在 M6 做真实 loader 接入。
//
// 覆盖层级声明：本脚本只证明「发布包能通过学生端 manifest 契约」；
// Electron 学生端真实加载与作答一致性属于 M6 的 NAS 实测，当前如实标记 pending。
//
//   node scripts/e2e/nas-student-contract.mjs [--package dir] [--student-repo dir]

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

const results = [];
function check(name, ok, detail) {
  results.push({ name, ok: Boolean(ok), detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` :: ${detail}` : ""}`);
  return Boolean(ok);
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

function main() {
  let ok = true;
  ok = check("student-repo-found", fs.existsSync(studentRepo), studentRepo) && ok;
  const studentHasNodeModules = fs.existsSync(path.join(studentRepo, "node_modules"));
  console.log(`INFO student repo node_modules: ${studentHasNodeModules ? "installed" : "absent"}；Electron 真实加载实测安排在 M6（需 npm install + 桌面运行），当前不算验收通过`);

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
    ok = check(
      `exam[${examId}].id-shape`,
      /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/u.test(examId)
    ) && ok;

    const runtimeFile = path.join(packageDir, `${examId}.js`);
    ok = check(`exam[${examId}].runtime-js-present`, fs.existsSync(runtimeFile)) && ok;

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

    // checksums.assetManifestSha256 是学生端防篡改绑定；同规则校验。
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
  console.log(`\nnas-student-contract: ${ok ? "PASS" : "FAIL"} (${results.filter((r) => r.ok).length}/${results.length})`);
  fs.mkdirSync(path.join(repoRoot, "artifacts"), { recursive: true });
  process.exit(ok ? 0 : 1);
}

main();
