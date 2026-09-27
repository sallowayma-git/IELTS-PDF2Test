#!/usr/bin/env node
// C3 写作题库真实应用 e2e：#/legacy/writing 重定向到题库写作子标签 → 新建 Task 1 + Task 2、
// 填题目、标记可导出 → 导出（走 PDF2TEST_AUTOMATION_EXPORT_DIR 钩子，不弹原生框）→
// 检查产物文件存在且含刚录入的题目文本 → 删除一题后列表同步。
import fs from "node:fs";
import path from "node:path";
import {
  CDP_CHANNEL_LABEL, CDP_CHANNEL_NOTE, CannotRunError,
  assertBuildFresh, buildFreshReport, gitHead, launchTauriAppCdp, repoRoot, sha256File, sleep, writeReport,
} from "./lib/tauri-cdp-harness.mjs";

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const runDir = path.join(repoRoot, "artifacts", "e2e-cdp", `writing-bank-${stamp}`);
const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const extraArgs = "--no-sandbox --disable-gpu";

function recordAssertion(report, id, ok, detail) {
  report.assertions.push({ id, ok: Boolean(ok), detail });
  console.log(`[assert] ${ok ? "PASS" : "FAIL"} ${id} — ${detail}`);
}

// 给受控输入（React value+onChange）赋值：必须走原生 setter + 派发 input/change，React 才认。
async function setControlled(session, selector, value) {
  await session.evaluate(`(() => {
    const el = document.querySelector(${JSON.stringify(selector)});
    if (!el) return false;
    const proto = el.tagName === 'TEXTAREA' ? window.HTMLTextAreaElement.prototype
      : el.tagName === 'SELECT' ? window.HTMLSelectElement.prototype : window.HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
    setter.call(el, ${JSON.stringify(value)});
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return true;
  })()`);
}

async function writingJobCount(session) {
  return await session.evaluate(`document.querySelectorAll('[data-testid="writing-job-table"] .job-row').length`);
}

// 新建一道 taskType 的写作题并填题目、标记可导出。返回新建后的 job 数。
async function createAndReadyTask(session, taskType, promptText) {
  await setControlled(session, '.writing-create-form select', taskType);
  await session.clickByText('新建写作任务');
  await sleep(800);
  // 选中最新一行（新建后默认选中它），填题目。
  await session.waitFor(`!!document.querySelector('.writing-prompt-textarea')`, { timeoutMs: 8000, label: "editor" });
  await setControlled(session, '.writing-prompt-textarea', promptText);
  await session.clickByText('保存');
  await sleep(600);
  await session.clickByText('标记可导出');
  await sleep(600);
}
const T1 = `图表描述题正文 T1 ${stamp}`;
const T2 = `议论文正文 T2 ${stamp}`;
const report = {
  task: "writing-bank-subtab-export",
  scope: "写作题库并入题库写作子标签 + 重定向 + 就地导出（真实应用端到端）",
  channel: CDP_CHANNEL_LABEL, channelNote: CDP_CHANNEL_NOTE,
  evidenceLevel: "real-app-e2e (CDP automation channel)",
  assertions: [], consoleErrors: [], pageExceptions: [],
};

async function main() {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits: true });
  report.identity = { exePath, exeSha256: sha256File(exePath), commit: gitHead(repoRoot), buildFresh: buildFreshReport(fresh) };
  const exportDir = path.join(runDir, "exports");
  const session = await launchTauriAppCdp({ exePath, runDir, extraBrowserArgs: extraArgs });
  let verdict = "failed";
  try {
    report.browserArgs = session.browserArgs;
    await session.evaluate(`(() => { window.confirm = () => true; return true; })()`);

    // 1) #/legacy/writing 重定向到题库写作子标签。
    await session.evaluate(`(() => { window.location.hash = "#/legacy/writing"; return true; })()`);
    await session.waitFor(`!!document.querySelector('[data-testid="library-writing-panel"]')`, { timeoutMs: 20000, label: "writing-panel" });
    const hash = await session.evaluate(`window.location.hash`);
    recordAssertion(report, "C3-1 legacy-writing-redirects-to-subtab", String(hash).includes("modality=writing"),
      `#/legacy/writing 落到 ${JSON.stringify(hash)}，写作面板已渲染`);

    // 2) 新建 Task 1 + Task 2 并标记可导出。
    await createAndReadyTask(session, "task1", T1);
    await createAndReadyTask(session, "task2", T2);
    const count = await writingJobCount(session);
    recordAssertion(report, "C3-2 create-two-tasks", count === 2, `写作任务数=${count}（期望 2）`);

    // 3) 就地导出（走自动化目录钩子，不弹原生框）。
    await session.clickSelector('[data-testid="writing-export"]');
    await session.waitFor(`(document.querySelector('[data-testid="writing-notice"]')?.textContent ?? '').includes('已导出')`, { timeoutMs: 30000, label: "export-done" });

    // 4) 检查产物文件存在且含刚录入的题目文本。
    const artifactsFound = [];
    const roots = [path.join(exportDir, "writing-exams"), exportDir];
    let combined = "";
    for (const root of roots) {
      if (!fs.existsSync(root)) continue;
      for (const f of fs.readdirSync(root)) {
        if (f.endsWith(".js") || f.endsWith(".json")) { artifactsFound.push(path.join(root, f)); combined += fs.readFileSync(path.join(root, f), "utf8"); }
      }
    }
    report.artifactsFound = artifactsFound;
    recordAssertion(report, "C3-3 export-artifacts-contain-prompts",
      artifactsFound.length > 0 && combined.includes(T1) && combined.includes(T2),
      `产物文件=${JSON.stringify(artifactsFound)}；含 T1=${combined.includes(T1)}、含 T2=${combined.includes(T2)}`);

    // 5) 删除一题后列表同步。
    await session.clickSelector('[data-testid="writing-job-table"] .job-row');
    await sleep(400);
    await session.clickByText('删除');
    await sleep(800);
    const after = await writingJobCount(session);
    recordAssertion(report, "C3-4 delete-updates-list", after === 1, `删除后写作任务数=${after}（期望 1）`);

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
  console.log(`[writing-bank] verdict=${report.verdict} assertions=${report.assertions.length} failed=${JSON.stringify(report.failedAssertions)}`);
  console.log(`[writing-bank] report=${file}`);
  process.exitCode = report.verdict === "passed" ? 0 : 1;
}

main().catch((error) => {
  const cannotRun = error instanceof CannotRunError;
  report.verdict = cannotRun ? "cannot-run" : "failed";
  report.error = String(error?.message ?? error);
  try { writeReport(runDir, report); } catch {}
  console.error(`[writing-bank] ${report.verdict}: ${report.error}`);
  process.exitCode = cannotRun ? 3 : 1;
});

