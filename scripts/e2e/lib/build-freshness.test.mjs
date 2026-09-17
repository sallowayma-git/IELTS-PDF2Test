// 构建新鲜度护栏的自测（层 1：纯逻辑，不驱动真实应用）。
//
// 背景（用户本轮硬约束）：
//   「构建必须关联源码、dist 和 exe，避免旧前端被新 exe 包入后仍判 fresh。」
//
// 这是一个**真实的回归**：`tauri build --no-bundle` 配 `beforeBuildCommand:""` 不会
// 重建前端，所以「旧 dist + 新 exe」在只比较 `src` 与 `exe` 的旧护栏下会**完全漏判**
// （src 的 mtime 早于 exe，于是 newer 为空 => 判 fresh）。
//
// 本文件用合成目录把这条链固定下来：前端输入 → dist → exe，后端输入 → exe。
// 任何一段断裂都必须判 stale（CANNOT-RUN），而不是告警后继续给「通过」。

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { assertBuildFresh, CannotRunError } from "./tauri-cdp-harness.mjs";
import {
  buildManifest,
  saveManifest,
  sha256File,
} from "./build-manifest.mjs";
import {
  assertFreshBuild,
  buildFreshness,
  CannotRunError as LegacyCannotRunError,
} from "./tauri-harness.mjs";

const T0 = Date.UTC(2026, 8, 16, 10, 0, 0); // 基准时刻（固定，避免依赖真实时钟）
const at = (minutes) => new Date(T0 + minutes * 60_000);

let roots = [];
afterEach(() => {
  for (const r of roots) fs.rmSync(r, { recursive: true, force: true });
  roots = [];
});

/** 造一棵合成仓库：src/dist/src-tauri/src 三处输入 + 一个 exe 文件。 */
function makeRepo({ srcMin, distMin, exeMin, backendMin = 0, withDist = true }) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "pdf2test-fresh-"));
  roots.push(root);

  const write = (rel, minutes, content = "x") => {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content);
    if (minutes != null) fs.utimesSync(full, at(minutes), at(minutes));
    return full;
  };

  write("src/main.ts", srcMin);
  write("index.html", srcMin != null ? srcMin - 1 : null);
  write("package.json", srcMin != null ? srcMin - 1 : null);
  write("src-tauri/src/lib.rs", backendMin);
  write("src-tauri/Cargo.toml", backendMin);
  if (withDist) {
    write("dist/index.html", distMin);
    write("dist/assets/index.js", distMin);
  }
  const exePath = write("build/app.exe", exeMin);
  return { root, exePath };
}

const staleMessage = (fn) => {
  try {
    fn();
  } catch (e) {
    if (e instanceof CannotRunError) return e.message;
    throw e;
  }
  throw new Error("期望抛出 CannotRunError，但没有抛错（护栏失效）");
};

describe("assertBuildFresh：三段链（前端输入 → dist → exe，后端输入 → exe）", () => {
  it("旧 dist 被新 exe 包入 => 判 stale（用户点名的回归）", () => {
    // 复现真实时间线：src 11:12、dist 11:05（比 src 旧）、exe 11:15（比两者都新）。
    const { root, exePath } = makeRepo({ srcMin: 12, distMin: 5, exeMin: 15 });
    const msg = staleMessage(() => assertBuildFresh({ exePath, repoRoot: root }));
    expect(msg).toContain("staleBuild");
    expect(msg).toContain("dist 落后于前端源码");
  });

  it("旧 dist + 新 exe：即使 src 早于 exe 也不得判 fresh（旧护栏会漏判的那一条）", () => {
    const { root, exePath } = makeRepo({ srcMin: 12, distMin: 5, exeMin: 15 });
    // 旧护栏的判据：src(12) > exe(15) 为假 => newer 为空 => 判 fresh。证明它确实漏判。
    const legacy = buildFreshness(exePath, root);
    expect(legacy.staleBuild).toBe(true);
    expect(legacy.staleReasons.join(" ")).toContain("dist 落后于前端源码");
    // 两个 harness 各自定义 CannotRunError（模块实例不同），故用各自导出的类断言。
    let thrown = null;
    try {
      assertFreshBuild(legacy);
    } catch (e) {
      thrown = e;
    }
    expect(thrown).toBeInstanceOf(LegacyCannotRunError);
    expect(thrown.message).toContain("陈旧构建");
    expect(thrown.message).toContain("dist 落后于前端源码");
  });

  it("exe 早于 dist => 判 stale（exe 未包含最新前端）", () => {
    const { root, exePath } = makeRepo({ srcMin: 1, distMin: 20, exeMin: 10 });
    const msg = staleMessage(() => assertBuildFresh({ exePath, repoRoot: root }));
    expect(msg).toContain("exe 早于 dist");
  });

  it("dist 缺失 => 判 stale（无法核对 exe 内嵌的前端产物）", () => {
    const { root, exePath } = makeRepo({ srcMin: 1, distMin: null, exeMin: 10, withDist: false });
    const msg = staleMessage(() => assertBuildFresh({ exePath, repoRoot: root }));
    expect(msg).toContain("dist 不存在或为空");
  });

  it("完整链新鲜 => 通过，并回传三段各自的最新时间戳", () => {
    const { root, exePath } = makeRepo({ srcMin: 1, distMin: 5, exeMin: 10, backendMin: 3 });
    const fresh = assertBuildFresh({ exePath, repoRoot: root });
    expect(fresh.chainOk).toBe(true);
    expect(fresh.frontendNewestMs).toBe(at(1).getTime());
    expect(fresh.distNewestMs).toBe(at(5).getTime());
    expect(fresh.backendNewestMs).toBe(at(3).getTime());
    expect(fresh.tolerated).toEqual([]);
  });

  it("前端陈旧**不**因 tolerateConcurrentEdits 被豁免（src/ 无并发写入者）", () => {
    const { root, exePath } = makeRepo({ srcMin: 12, distMin: 5, exeMin: 15 });
    const msg = staleMessage(() =>
      assertBuildFresh({ exePath, repoRoot: root, tolerateConcurrentEdits: true })
    );
    expect(msg).toContain("dist 落后于前端源码");
  });

  it("后端输入比 exe 新：默认判 stale；带 tolerate 时豁免但原样列出", () => {
    const { root, exePath } = makeRepo({ srcMin: 1, distMin: 5, exeMin: 10, backendMin: 30 });
    expect(() => assertBuildFresh({ exePath, repoRoot: root })).toThrow(CannotRunError);
    const fresh = assertBuildFresh({ exePath, repoRoot: root, tolerateConcurrentEdits: true });
    expect(fresh.chainOk).toBe(true);
    expect(fresh.tolerated.map((t) => t.path).sort()).toEqual([
      "src-tauri/Cargo.toml",
      "src-tauri/src/lib.rs",
    ]);
  });
});

describe("assertBuildFresh：内容哈希清单（源码 ↔ dist ↔ exe 的真实关联）", () => {
  /** 造一棵合成仓库并写入一份与当前内容对应的清单。 */
  function makeRepoWithManifest(opts) {
    const { root, exePath } = makeRepo(opts);
    const manifest = buildManifest({ root, exePath });
    saveManifest(root, manifest);
    return { root, exePath, manifest };
  }

  it("清单哈希一致 => 判 fresh，且 mode=manifest", () => {
    const { root, exePath } = makeRepoWithManifest({ srcMin: 1, distMin: 5, exeMin: 10, backendMin: 3 });
    const fresh = assertBuildFresh({ exePath, repoRoot: root });
    expect(fresh.mode).toBe("manifest");
    expect(fresh.chainOk).toBe(true);
    expect(fresh.hashes.frontendInputs).toMatch(/^[0-9a-f]{64}$/);
  });

  it("内容一致但 mtime 被推新：mtime 判定误报，内容判定正确放行", () => {
    const { root, exePath } = makeRepoWithManifest({ srcMin: 1, distMin: 5, exeMin: 10 });
    // 模拟「npm run build 重跑一次，产出内容完全相同的 dist」：只动 mtime，不动内容。
    const distFile = path.join(root, "dist", "index.html");
    fs.utimesSync(distFile, at(60), at(60));
    // mtime 判定会说「exe 早于 dist」=> stale；这正是并发/重跑带来的噪声。
    expect(buildFreshness(exePath, root).staleBuild).toBe(true);
    // 内容判定：哈希一致 => fresh。
    const fresh = assertBuildFresh({ exePath, repoRoot: root });
    expect(fresh.mode).toBe("manifest");
    expect(fresh.chainOk).toBe(true);
  });

  it("前端源码内容变了（即使 mtime 未变）=> 判 stale，并指出 frontendInputs", () => {
    const { root, exePath } = makeRepoWithManifest({ srcMin: 1, distMin: 5, exeMin: 10 });
    // 内容改变但把 mtime 压回旧值：mtime 判定完全看不见，内容判定必须抓住。
    const srcFile = path.join(root, "src", "main.ts");
    fs.writeFileSync(srcFile, "CHANGED-CONTENT");
    fs.utimesSync(srcFile, at(1), at(1));
    const msg = staleMessage(() => assertBuildFresh({ exePath, repoRoot: root }));
    expect(msg).toContain("内容比对");
    expect(msg).toContain("前端输入");
  });

  it("dist 内容变了 => 判 stale，并指出 dist", () => {
    const { root, exePath } = makeRepoWithManifest({ srcMin: 1, distMin: 5, exeMin: 10 });
    const distFile = path.join(root, "dist", "assets", "index.js");
    fs.writeFileSync(distFile, "CHANGED-BUNDLE");
    fs.utimesSync(distFile, at(5), at(5));
    const msg = staleMessage(() => assertBuildFresh({ exePath, repoRoot: root }));
    expect(msg).toContain("dist");
  });

  it("后端内容变了：默认 stale；tolerate 时豁免（前端两段仍硬失败）", () => {
    const { root, exePath } = makeRepoWithManifest({ srcMin: 1, distMin: 5, exeMin: 10, backendMin: 3 });
    const backendFile = path.join(root, "src-tauri", "src", "lib.rs");
    fs.writeFileSync(backendFile, "CHANGED-BACKEND");
    fs.utimesSync(backendFile, at(3), at(3));
    expect(() => assertBuildFresh({ exePath, repoRoot: root })).toThrow(CannotRunError);
    const fresh = assertBuildFresh({ exePath, repoRoot: root, tolerateConcurrentEdits: true });
    expect(fresh.mode).toBe("manifest");
    expect(fresh.chainOk).toBe(true);
    expect(fresh.tolerated.length).toBe(1);
  });

  it("清单自陈构建期间输入漂移 => 一律 stale（不可归因的二进制不得验收）", () => {
    const { root, exePath } = makeRepo({ srcMin: 1, distMin: 5, exeMin: 10, backendMin: 3 });
    const start = { frontend: { hash: "deadbeef" }, backend: { hash: "cafebabe" } };
    const manifest = buildManifest({ root, exePath, inputsAtBuildStart: start });
    saveManifest(root, manifest);
    expect(manifest.inputsDriftedDuringBuild).toBe(true);
    const msg = staleMessage(() =>
      assertBuildFresh({ exePath, repoRoot: root, tolerateConcurrentEdits: true })
    );
    expect(msg).toContain("构建期间输入漂移");
  });

  it("exe 换了内容 => 找不到对应清单，退回 mtime 判定（不会误用别人的清单）", () => {
    const { root, exePath, manifest } = makeRepoWithManifest({ srcMin: 1, distMin: 5, exeMin: 10 });
    const oldSha = manifest.exeSha256;
    fs.writeFileSync(exePath, "A-DIFFERENT-BINARY");
    fs.utimesSync(exePath, at(10), at(10));
    expect(sha256File(exePath)).not.toBe(oldSha);
    const fresh = assertBuildFresh({ exePath, repoRoot: root });
    // 清单是按 exe 内容命名的：换了 exe 就命中不到旧清单，必须退回 mtime 判定。
    expect(fresh.mode).toBe("mtime");
  });
});
