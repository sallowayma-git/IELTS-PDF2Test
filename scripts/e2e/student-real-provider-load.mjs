#!/usr/bin/env node
// 学生端**真实代码**读取验收（补 `nas-student-contract.mjs` 的最后一层）。
//
// 为什么还要这个脚本：`nas-student-contract.mjs` 是**忠实镜像**——它按学生端的规则
// 重新实现了一遍校验逻辑。镜像能证明「我们的包符合规则」，但证明不了「学生端那份
// 代码真的能读」。本脚本换一条路：直接 require 学生端仓库**已编译的真实产物**
//   <student-repo>/server/dist/lib/library/reading/NasJsDirectReadingAssetProvider.js
// 用真实 `ExamRuntimeConfig` 构造 provider，把本仓真实发布出来的 NAS 包当 NAS 挂上去，
// 跑 `getStatus()` / `listAssets()` / `getAsset()` 三个真实入口。
//
// 核心实现放在 `lib/student-real-provider.mjs`，真实链路脚本（
// `tauri-cdp-cloud-repair-chain.mjs`）用的是同一个函数——避免出现「CLI 跑真的、
// 链路里跑另一套」。
//
// 覆盖边界（务必如实理解）：
//   本脚本证明的是 **服务端读取层**（`NasJsDirectReadingAssetProvider` + `reading-v2-loader`
//   + `reading-asset-resolver`）能加载本仓的发布包，并且加载出来的内容与云端修复后的
//   canonical 一致。
//   它**不**证明 Electron 学生端渲染与作答一致——那需要真正启动学生端（M6）。
//
// 用法：
//   node scripts/e2e/student-real-provider-load.mjs --package <nas-library 目录>
//        [--exam <examId>] [--student-repo <dir>] [--out report.json]
// 退出码：0 全部通过；1 有失败；3 无法执行（缺学生端产物 / 缺包）。

import fs from "node:fs";
import path from "node:path";
import {
  DEFAULT_STUDENT_REPO,
  loadPublishedPackageWithRealProviderAsync,
} from "./lib/student-real-provider.mjs";

const args = process.argv.slice(2);
function argOf(name, fallback) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : fallback;
}

const packageDir = argOf("--package", "");
const studentRepo = argOf("--student-repo", DEFAULT_STUDENT_REPO);
const requestedExam = argOf("--exam", null);
const outPath = argOf("--out", null);

const outcome = await loadPublishedPackageWithRealProviderAsync({
  packageDir,
  studentRepo,
  examId: requestedExam,
});

for (const entry of outcome.results) {
  const tag = entry.info ? "INFO" : entry.ok ? "PASS" : "FAIL";
  console.log(`${tag} ${entry.name}${entry.detail ? ` :: ${entry.detail}` : ""}`);
}

if (outcome.cannotRun) {
  console.error(`\nstudent-real-provider-load: cannot-run :: ${outcome.reason}`);
  if (outPath) {
    const resolved = path.resolve(outPath);
    fs.mkdirSync(path.dirname(resolved), { recursive: true });
    fs.writeFileSync(
      resolved,
      JSON.stringify({ status: "cannot-run", reason: outcome.reason, results: outcome.results }, null, 2),
    );
  }
  process.exit(3);
}

const total = outcome.results.length;
const passed = total - outcome.failures.length;
console.log(`\nstudent-real-provider-load: ${outcome.ok ? "PASS" : "FAIL"} (${passed}/${total})`);

if (outPath) {
  const resolved = path.resolve(outPath);
  fs.mkdirSync(path.dirname(resolved), { recursive: true });
  fs.writeFileSync(
    resolved,
    JSON.stringify(
      {
        status: outcome.ok ? "passed" : "failed",
        studentRepo,
        providerPath: outcome.providerPath,
        packageDir: outcome.packageDir,
        examId: outcome.examId,
        appVersion: outcome.appVersion,
        observed: outcome.observed,
        results: outcome.results,
      },
      null,
      2,
    ),
  );
  console.log(`report: ${resolved}`);
}

process.exit(outcome.ok ? 0 : 1);
