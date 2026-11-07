#!/usr/bin/env node
// 组卷发布真实应用 e2e（题库页「组卷视图」）：
//   导入三份阅读卷 → 等识别结束 → 把三篇分别设为 P1/P2/P3 → 组卷视图里勾选三篇
//   → 发布为当前试卷 → 断言 NAS 上 current-paper.json 与清单条目一致（路径、examId、校验和）
//   → 页面顶部「当前试卷」显示新编号 → 名称单元格编辑（Enter 保存）经真实后端落库。
//
// 证据层级：real-app-e2e（CDP 自动化通道驱动真实 Tauri 界面 + 读取磁盘事实）。
// 依赖私有语料（fixtures/golden/private-real，不入库）与已构建的 debug exe；缺失时以
// CannotRunError 退出（退出码 3），不伪装成通过。
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
// 三份阅读卷（私有语料）。自动识别的 Part 不作为前提：不足的 Part 在界面里手动设置。
const fixtures = [
  path.join(repoRoot, "fixtures", "golden", "private-real", "chili-peppers.pdf"),
  path.join(repoRoot, "fixtures", "golden", "private-real", "organisational-design.pdf"),
  path.join(repoRoot, "fixtures", "golden", "private-real", "fishbourne-roman-palace.pdf"),
];

function recordAssertion(report, id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

/** 题库页行快照：id、Part、阶段、名称、是否勾选（组卷视图）。 */
async function listRows(session) {
  return await session.evaluate(`(() => {
    const out = [];
    for (const tr of document.querySelectorAll('[data-testid="paper-row"]')) {
      const check = tr.querySelector('input[type="checkbox"]');
      out.push({
        id: tr.getAttribute('data-item-id'),
        part: (tr.querySelector('.part-badge')?.textContent || '').trim() || null,
        title: (tr.querySelector('.paper-title-cell')?.textContent || '').trim(),
        checked: Boolean(check && check.checked),
        disabled: Boolean(check && check.disabled),
        group: tr.closest('[data-testid^="paper-group-"]')?.getAttribute('data-testid') ?? null,
      });
    }
    return out;
  })()`);
}

/** 用 React 能识别的方式点击复选框（原生 click 会触发 onChange）。 */
async function clickPaperCheck(session, itemId) {
  return await session.evaluate(`(() => {
    const tr = document.querySelector('tr[data-item-id="${itemId}"]');
    const check = tr && tr.querySelector('input[type="checkbox"]');
    if (!check || check.disabled) return false;
    check.click();
    return true;
  })()`);
}

async function readRowsFromInvoke(session) {
  const r = await session.invoke("list_library_items", { includeDeleted: false });
  return r?.ok && Array.isArray(r.value) ? r.value : [];
}

function manifestEntries(nasRoot) {
  const text = fs.readFileSync(path.join(nasRoot, "manifest.js"), "utf8");
  const json = text.trim().replace(/^window\.__READING_EXAM_MANIFEST__\s*=\s*/, "").replace(/;\s*$/, "");
  return JSON.parse(json);
}

function sha256Buffer(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

async function main() {
  const missing = fixtures.filter((f) => !fs.existsSync(f));
  if (missing.length) throw new CannotRunError(`私有语料缺失（需 fixtures/golden/private-real）：${missing.join(", ")}`);
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });

  const report = {
    task: "paper-publish",
    scope: "题库组卷视图：三篇阅读勾选 → 发布为当前试卷 → NAS current-paper.json 与清单一致；名称单元格编辑落库",
    channel: CDP_CHANNEL_LABEL, channelNote: CDP_CHANNEL_NOTE,
    evidenceLevel: "real-app-e2e (CDP automation channel + disk facts)",
    identity: { exePath, commit: gitHead(repoRoot), buildFresh: buildFreshReport(fresh) },
    assertions: [], consoleErrors: [], pageExceptions: [],
  };
  fs.mkdirSync(path.join(runDir, "pdfs"), { recursive: true });
  for (const f of fixtures) fs.copyFileSync(f, path.join(runDir, "pdfs", path.basename(f)));
  const nasRoot = path.join(runDir, "nas-library");

  const session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  let verdict = "failed";
  try {
    report.browserArgs = session.browserArgs;

    // 1. 发布目录写进设置（与 publish-ready 相同的做法：readAppSettings 每次读 localStorage）。
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library" });
    await session.evaluate(`(() => {
      const raw = window.localStorage.getItem(${JSON.stringify(APP_SETTINGS_KEY)});
      const settings = raw ? JSON.parse(raw) : {};
      settings.nasDestination = ${JSON.stringify(nasRoot)};
      window.localStorage.setItem(${JSON.stringify(APP_SETTINGS_KEY)}, JSON.stringify(settings));
      return true;
    })()`);
    // 重新挂载题库页，让「当前试卷」卡片读到目录。
    await session.evaluate(`(() => { window.location.hash = "#/settings"; return true; })()`);
    await sleep(400);
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="current-paper-card"]')`, { timeoutMs: 20000, label: "paper-card" });

    // 2. 导入三份阅读卷。
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 15000, label: "import-drawer" });
    await sleep(600);
    await session.clickSelector('[data-testid="import-pick-folder"]');
    await session.waitFor(`document.querySelectorAll('[data-testid="import-picked-files"] li').length >= ${fixtures.length}`, { timeoutMs: 30000, label: "picked" });
    await session.clickSelector('[data-testid="import-start"]');

    // 3. 等三条都识别结束（不再排队/识别中）。识别是长任务，给足时间。
    const deadline = Date.now() + 25 * 60 * 1000;
    while (Date.now() < deadline) {
      const listed = await readRowsFromInvoke(session);
      const reading = listed.filter((r) => r.modality === "reading");
      const busy = reading.some((r) => ["queued", "running", "local_recognition", "cloud_recognition", "reconciling"].includes(r.processing?.stage));
      if (reading.length >= fixtures.length && !busy) break;
      await sleep(3000);
    }
    const listed = await readRowsFromInvoke(session);
    const readingItems = listed.filter((r) => r.modality === "reading" && !r.deletedAt);
    recordAssertion(report, "P-1 three-reading-items-imported", readingItems.length >= fixtures.length,
      `阅读题库条目=${readingItems.length}（期望 ${fixtures.length}）`);
    if (readingItems.length < fixtures.length) throw new Error("导入的阅读卷不足三份，无法组卷");

    // 4. 设置 Part：自动判定已到位的保持不动，缺的用行内 Part 下拉手动补齐。
    await session.evaluate(`(() => { window.location.hash = "#/library"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 20000, label: "library-2" });
    const wanted = ["P1", "P2", "P3"];
    const current = new Map(readingItems.map((r) => [r.id, r.partLabel ?? null]));
    for (const part of wanted) {
      if ([...current.values()].includes(part)) continue;
      const candidate = [...current.entries()].find(([, label]) => !wanted.includes(label));
      if (!candidate) break;
      const [id] = candidate;
      await session.evaluate(`(() => {
        const el = document.querySelector('[data-item-id="${id}"] [data-testid="library-row-part-select"]');
        if (!el) return false;
        const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set;
        setter.call(el, '${part}'); el.dispatchEvent(new Event('change', { bubbles: true })); return true;
      })()`);
      current.set(id, part);
      const deadlinePart = Date.now() + 15000;
      while (Date.now() < deadlinePart) {
        const seen = (await readRowsFromInvoke(session)).find((r) => r.id === id);
        if (seen?.partLabel === part) break;
        await sleep(500);
      }
    }
    const ids = {};
    for (const part of wanted) ids[part] = [...current.entries()].find(([, label]) => label === part)?.[0];
    recordAssertion(report, "P-2 three-parts-assigned", wanted.every((p) => ids[p]),
      `Part 分配=${JSON.stringify(ids)}`);
    if (!wanted.every((p) => ids[p])) throw new Error("无法为三篇分别设置 P1/P2/P3");

    // 5. 组卷视图：勾选三篇，发布按钮只在三篇齐了之后可点。
    // 题库默认就是组卷表格（待检查题不在题库里，导入页负责它们）。
    await session.waitFor(`!!document.querySelector('[data-testid="paper-table"]')`, { timeoutMs: 15000, label: "paper-table" });
    await clickPaperCheck(session, ids.P1);
    await clickPaperCheck(session, ids.P2);
    await sleep(300);
    const beforeThird = await session.evaluate(`document.querySelector('[data-testid="paper-publish"]').disabled`);
    recordAssertion(report, "P-3 publish-disabled-until-three-parts", beforeThird === true, `两篇时发布按钮 disabled=${beforeThird}`);
    await clickPaperCheck(session, ids.P3);
    await sleep(300);
    const afterThird = await session.evaluate(`document.querySelector('[data-testid="paper-publish"]').disabled`);
    recordAssertion(report, "P-4 publish-enabled-after-three-parts", afterThird === false, `三篇齐后发布按钮 disabled=${afterThird}`);
    const checkedRows = (await listRows(session)).filter((r) => r.checked).map((r) => r.id).sort();
    recordAssertion(report, "P-5 three-rows-checked-in-table", checkedRows.join(",") === [ids.P1, ids.P2, ids.P3].sort().join(","),
      `勾选=${JSON.stringify(checkedRows)}`);

    // 6. 发布为当前试卷。
    await session.clickSelector('[data-testid="paper-publish"]');
    // 替换前必须经过确认框（列出将被替换的当前试卷）。
    await session.waitFor(`!!document.querySelector('[data-testid="paper-confirm"]')`, { timeoutMs: 10000, label: "paper-confirm" });
    await session.clickSelector('[data-testid="paper-confirm-ok"]');
    await session.waitFor(
      `(() => { const m = document.querySelector('[data-testid="paper-composer"] [role="status"]'); return !!m && /当前试卷已发布|没有|失败|未能|保持不变/.test(m.textContent); })()`,
      { timeoutMs: 15 * 60 * 1000, intervalMs: 1000, label: "paper-result" }
    );
    const resultText = await session.evaluate(`(document.querySelector('[data-testid="paper-composer"] [role="status"]')||{}).textContent || null`);
    report.paperResultText = resultText;
    const published = typeof resultText === "string" && resultText.includes("当前试卷已发布");
    recordAssertion(report, "P-6 paper-published-message", published, `界面提示=${JSON.stringify(resultText)}`);

    // 7. 磁盘事实：current-paper.json 与清单条目一致、脚本校验和正确。
    const pointerPath = path.join(nasRoot, "current-paper.json");
    const pointerExists = fs.existsSync(pointerPath);
    recordAssertion(report, "P-7 current-paper-file-written", pointerExists, `${pointerPath}`);
    if (pointerExists) {
      const pointer = JSON.parse(fs.readFileSync(pointerPath, "utf8"));
      report.pointer = { paperId: pointer.paperId, batchId: pointer.batchId, reading: pointer.reading.map((e) => ({ part: e.part, examId: e.examId, title: e.title })) };
      recordAssertion(report, "P-8 pointer-schema-and-parts", pointer.schemaVersion === "CurrentPaperV1"
        && pointer.reading.length === 3 && pointer.reading.map((e) => e.part).join(",") === "P1,P2,P3",
        `schema=${pointer.schemaVersion} parts=${pointer.reading.map((e) => e.part).join(",")}`);
      const manifest = manifestEntries(nasRoot);
      let allMatch = true;
      const details = [];
      for (const entry of pointer.reading) {
        const committed = manifest[entry.examId];
        const scriptFile = path.join(nasRoot, entry.script.replace(/^\.\//, ""));
        const scriptOk = Boolean(committed) && committed.script === entry.script && fs.existsSync(scriptFile)
          && sha256File(scriptFile) === entry.scriptSha256 && committed.checksums?.scriptSha256 === entry.scriptSha256
          && committed.checksums?.runtimeSha256 === entry.runtimeSha256;
        allMatch = allMatch && scriptOk;
        details.push(`${entry.part}:${scriptOk ? "match" : "MISMATCH"}`);
      }
      recordAssertion(report, "P-9 pointer-matches-manifest-and-script-bytes", allMatch, details.join(" "));
      const shownId = await session.evaluate(`(document.querySelector('[data-testid="current-paper-meta"]')||{}).textContent || ""`);
      recordAssertion(report, "P-10 card-shows-paper-id", String(shownId).includes(pointer.paperId.slice(0, 8)),
        `卡片=${JSON.stringify(shownId)}`);
    }

    // 8. 名称单元格：单击 → 输入新名 → Enter 保存 → 后端读回新名。
    const renameTarget = ids.P2;
    await session.evaluate(`(() => {
      const tr = document.querySelector('tr[data-item-id="${renameTarget}"]');
      tr.querySelector('.paper-title-cell').click(); return true;
    })()`);
    await session.waitFor(`!!document.querySelector('.paper-title-input')`, { timeoutMs: 5000, label: "title-input" });
    const newTitle = `组卷改名验证 ${stamp.slice(11, 19)}`;
    await session.evaluate(`(() => {
      const input = document.querySelector('.paper-title-input');
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(input, ${JSON.stringify(newTitle)});
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
      return true;
    })()`);
    let savedTitle = null;
    const renameDeadline = Date.now() + 15000;
    while (Date.now() < renameDeadline) {
      savedTitle = (await readRowsFromInvoke(session)).find((r) => r.id === renameTarget)?.title ?? null;
      if (savedTitle === newTitle) break;
      await sleep(500);
    }
    recordAssertion(report, "P-11 rename-cell-enter-persists", savedTitle === newTitle, `后端标题=${JSON.stringify(savedTitle)}`);

    report.consoleErrors = session.cdp.events
      .filter((e) => e.method === "Runtime.consoleAPICalled" && ["error", "assert"].includes(e.params?.type))
      .map((e) => (e.params.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 300));
    verdict = report.assertions.every((a) => a.ok) ? "passed" : "failed";
  } finally {
    report.verdict = verdict;
    report.failedAssertions = report.assertions.filter((a) => !a.ok).map((a) => a.id);
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
