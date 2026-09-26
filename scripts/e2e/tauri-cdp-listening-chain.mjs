#!/usr/bin/env node
/**
 * T6 / R6：**听力真实 App 验收链**（真实 Tauri 应用 + WebView2 CDP 通道）。
 *
 * 这条链要证明的是一整条**用户能走完**的产品路径，不是「某段代码跑过」：
 *
 *   1. 题库页加载；
 *   2. 「选择文件」把听力卷放进已选清单；
 *   3. 开始导入 → 弹出听力确认弹窗（说明产品**认出了**这是听力卷）；
 *   4. 点「选择音频文件」→ 真实 picker 钩子交回 4 段音频 → 清单里恰好 4 个 Part，
 *      顺序与文件一致（音频顺序 = Part 顺序，这是产品语义）；
 *   5. 4 段的探针**全部通过**（失败态是 `audio-probe-blocked`，不许当通过）；
 *   6. 确认导入 → 题库里出现这一条；
 *   7. **等本地识别真正结束**，然后断言草稿结构：`modality=listening`、4 个 Part、
 *      题号 1–40 恰好各一次、每个 Part 都带自己的 `media` 且 sha256 就是绑定的那个文件；
 *   8. 工作区的 `ListeningHeader` 有 4 个 Part 页签，每个 Part 的播放器 `src` 指向
 *      **受管音频**（`<sha256>.<ext>`）且真的能加载（`readyState > 0`、`duration` 与
 *      绑定文件一致）；
 *   9. 像用户一样在界面上**填完 40 个答案**（选项类点选项，填空类打字，值都落在
 *      题型约束内）；
 *  10. 一键发布，期望**干净**结论 `data-publish-outcome="published"`（不是放行、
 *      不是「学生端打不开」）；
 *  11. 用**学生端真实代码**（R2 worktree `feat-listening-per-part-media` 的编译产物）
 *      加载刚发布出来的包：4 个 Part 各自解析到自己那段音频、字节 sha256 与发布包一致，
 *      且**阅读目录里没有这道听力卷**（R2 的端到端复证）。
 *
 * 为什么第 7–11 步必须存在（2026-09-23 第二轮复核 F1 / F2）：
 *   - 旧版第 7 步只断言「出现了某个机器可读结论」——任何结果都能过。识别没跑完就点发布
 *     会得到 `kind:"failed"`（「这道题还没有生成可编辑的题稿」），旧断言照样 PASS。
 *     那等于**什么都没证明**。现在每一步都断言确定的期望值。
 *   - 「这份卷子没有答案 key，所以末端不可达」不成立：产品流程本来就是**用户补原文缺失的
 *     答案**，这是用户唯一要做的决策。测试扮演用户填 40 个答案属于用户操作，不违反
 *     「机器不得编造答案」。所以学生端那一跳必须真跑到。
 *
 * 音频用**生成的 WAV**（440/523/659/784Hz 正弦，长度 6/7/8/9 秒）。探针会拒
 * `AudioNearSilent` / `AudioSevereClipping`，静音充数过不了。CDP 无法合成操作系统级
 * 文件拖放，所以上传继续走**真实按钮 → 真实 picker 钩子**这条产品路径（钩子只负责
 * 交回路径，点击与命中测试都是真的）。
 *
 * 用法：
 *   node scripts/e2e/tauri-cdp-listening-chain.mjs [--keep] [--run-dir <dir>]
 *        [--diagnostic-args] [--tolerate-concurrent-edits] [--accept-reattaches]
 *        [--student-repo <dir>]
 *
 * `--diagnostic-args`：加 `--no-sandbox --disable-gpu`。**执行方沙箱需要它**
 * （WebView2 renderer 会在启动约 7s 后崩掉、DevTools 端点消失，仓库自带的
 * `tauri-cdp-smoke.mjs` 早已写明）；质量方环境默认档即可通过。带它的运行记为
 * `runProfile=cdp-diagnostic`，**不得**当成默认产品路径通过。
 *
 * `--student-repo`：学生端 checkout，默认 R2 worktree
 * （`F:/workspace/IELTS-NASfor-WenDao-listening`，per-part media 与「听力不进阅读目录」
 * 两条都只在这里的编译产物里）。
 *
 * 退出码：0 = passed / passed_with_warnings；1 = failed；3 = CANNOT-RUN。
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  CDP_CHANNEL_LABEL,
  CDP_CHANNEL_NOTE,
  CannotRunError,
  applyReattachPolicy,
  assertBuildFresh,
  buildFreshReport,
  createStepRecorder,
  gitHead,
  gitWorktreeClean,
  isCleanPublishOutcome,
  launchTauriAppCdp,
  repoRoot,
  sha256File,
  sleep,
  summarizeReattaches,
  writeReport,
} from "./lib/tauri-cdp-harness.mjs";
import {
  DEFAULT_LISTENING_STUDENT_REPO,
  loadPublishedListeningPackageWithRealProviderAsync,
} from "./lib/student-listening-provider.mjs";

const exePath = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio.exe");
const keep = process.argv.includes("--keep");
const runDirIdx = process.argv.indexOf("--run-dir");
const runDir = runDirIdx >= 0
  ? path.resolve(process.argv[runDirIdx + 1])
  : path.join(repoRoot, "artifacts", "e2e-cdp", `run-listening-chain-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const diagnosticArgsRequested = process.argv.includes("--diagnostic-args");
const extraArgs = diagnosticArgsRequested ? "--no-sandbox --disable-gpu" : "";
const tolerateConcurrentEdits = process.argv.includes("--tolerate-concurrent-edits");
const acceptReattaches = process.argv.includes("--accept-reattaches");
const studentRepoIdx = process.argv.indexOf("--student-repo");
const studentRepo = studentRepoIdx >= 0
  ? path.resolve(process.argv[studentRepoIdx + 1])
  : DEFAULT_LISTENING_STUDENT_REPO;

// 真实听力卷（4 个 SECTION，Q1–10 / 11–20 / 21–30 / 31–40）。私有夹具，不进 Git 历史。
const PAPER_SOURCE = path.join(repoRoot, "fixtures", "golden", "private-real", "listening-vol7-t9.pdf");
const PAPER_NAME = "listening-vol7-t9.pdf";
const EXPECTED_PARTS = 4;
const EXPECTED_QUESTIONS = 40;
/** 识别是本地模型串，给足时间；超时即如实失败，并把当时界面提示与条目状态记下来。 */
const RECOGNITION_TIMEOUT_MS = 8 * 60 * 1000;

/**
 * 生成 4 段**能过探针**的 WAV。
 *
 * 探针会拒掉 `AudioNearSilent`（RMS 过低）与 `AudioSevereClipping`，所以不能拿静音充数：
 * 用 440Hz 基频 + 递增泛音的 16kHz 单声道 s16le 正弦，振幅 8000（与探针单测同一量级）。
 * 每段长度不同，顺便让「顺序 = Part 顺序」这件事在时长上也可区分。
 */
function writeSineWav(filePath, { seconds, baseHz }) {
  const sampleRate = 16_000;
  const frames = Math.round(seconds * sampleRate);
  const data = Buffer.alloc(frames * 2);
  for (let i = 0; i < frames; i += 1) {
    const t = i / sampleRate;
    const value = Math.sin(t * baseHz * Math.PI * 2) * 8000;
    data.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(value))), i * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0, "ascii");
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVE", 8, "ascii");
  header.write("fmt ", 12, "ascii");
  header.writeUInt32LE(16, 16);            // fmt chunk size
  header.writeUInt16LE(1, 20);             // PCM
  header.writeUInt16LE(1, 22);             // mono
  header.writeUInt32LE(sampleRate, 24);
  header.writeUInt32LE(sampleRate * 2, 28); // byte rate
  header.writeUInt16LE(2, 32);             // block align
  header.writeUInt16LE(16, 34);            // bits per sample
  header.write("data", 36, "ascii");
  header.writeUInt32LE(data.length, 40);
  fs.writeFileSync(filePath, Buffer.concat([header, data]));
}

const AUDIO_SPECS = [
  { name: "part-1.wav", seconds: 6, baseHz: 440 },
  { name: "part-2.wav", seconds: 7, baseHz: 523 },
  { name: "part-3.wav", seconds: 8, baseHz: 659 },
  { name: "part-4.wav", seconds: 9, baseHz: 784 },
];

const report = {
  task: "R6-listening-real-app-chain",
  scope: "听力卷：导入 → 识别完成 → 填 40 答案 → 干净发布 → 学生端逐 part 取音频",
  channel: CDP_CHANNEL_LABEL,
  channelNote: CDP_CHANNEL_NOTE,
  diagnosticRun: Boolean(extraArgs),
  runProfile: extraArgs ? "cdp-diagnostic" : "cdp-default",
  securityArgs: extraArgs ? extraArgs.split(/\s+/).filter(Boolean) : [],
  tolerateConcurrentEdits,
  acceptReattaches,
  startedAt: new Date().toISOString(),
  runDir,
  identity: {
    exePath,
    exeSha256: null,
    commit: gitHead(repoRoot),
    worktreeClean: gitWorktreeClean(repoRoot),
    buildFresh: null,
    paperSource: path.relative(repoRoot, PAPER_SOURCE).replace(/\\/g, "/"),
    studentRepo,
    parts: [],
  },
  steps: [],
  postChecks: [],
  verdict: "failed",
};

let session = null;
let recorder = null;
let itemId = null;

const nasDir = path.join(runDir, "nas-library");

const rowIdsExpr = `[...document.querySelectorAll('[data-testid="library-row"]')].map(r => r.getAttribute('data-item-id'))`;
const pickedNamesExpr =
  `[...document.querySelectorAll('[data-testid="import-picked-files"] li .file-name')].map(el => el.innerText.trim())`;
/** 弹窗里每个 Part：序号、文件名、探针状态类名。 */
const partRowsExpr = `[...document.querySelectorAll('[data-testid="listening-audio-parts"] li')].map(li => ({
  part: Number(li.getAttribute('data-part')),
  name: (li.querySelector('.file-name')?.innerText ?? '').trim(),
  probeClass: li.querySelector('.audio-probe')?.className ?? '',
  probeText: (li.querySelector('.audio-probe')?.innerText ?? '').trim(),
}))`;

/** 用真实 Tauri IPC 读工作区（与产品运行时同一条 invoke 通道）。 */
async function call(command, args = {}) {
  const wrapped = command === "apply_recognition_decisions" || command === "apply_editor_commands";
  const r = await session.invoke(command, wrapped ? { input: args } : args);
  if (r && r.__noInvoke) throw new Error("invoke 不可用");
  return r;
}

async function readWorkspace(target = itemId) {
  const r = await call("get_workspace_item", { itemId: target });
  if (!r?.ok) throw new Error(`get_workspace_item 失败：${r?.error}`);
  return r.value;
}

/** 当下界面上的处理副标题（没有则 null）。诊断超时时要用它说明卡在哪一步。 */
async function readProcessingNote() {
  return session.evaluate(
    `(() => {
       const el = document.querySelector('[data-testid="workspace-processing-note"]');
       return el ? el.innerText.replace(/\\s+/g, ' ').trim() : null;
     })()`,
  );
}

/**
 * 点开题库里那一条，进入工作区。
 *
 * 真实 App 里这一步带竞态（识别在后台跑，工作区可能先挂起来再重渲染），所以允许
 * **重试一次点开**，并把「重试过」如实写进证据，而不是把它藏成一次成功。
 */
async function openWorkspace() {
  const attempt = async () => {
    await session.clickSelectorWhenStable('[data-testid="library-row"]');
    return session
      .waitFor(`!!document.querySelector('[data-testid="exam-workspace"]')`, { timeoutMs: 60000, label: "exam-workspace" })
      .then(() => true)
      .catch(() => false);
  };
  const first = await attempt();
  const retried = !first;
  if (retried && !(await attempt())) throw new Error("点开条目后两次 60s 等待内仍未进入工作区");
  return retried;
}

/**
 * 配置发布目标目录到本轮的 `nas-library`。
 *
 * 走**产品自己的设置**（`localStorage`，见 `appSettings.ts`）：`publish()` 在
 * `nasDestination` 为空时会弹原生目录选择框，自动化里没人点，点「发布」就挂在那里。
 * 这里写的键与设置页写的是同一个，因此不是绕路。
 */
async function configureNasDestination() {
  await session.evaluate(
    `(() => {
       const key = "ielts-author-studio.app-settings.v1";
       let current = {};
       try { current = JSON.parse(window.localStorage.getItem(key) ?? "{}"); } catch { current = {}; }
       current.nasDestination = ${JSON.stringify(nasDir)};
       window.localStorage.setItem(key, JSON.stringify(current));
       window.localStorage.setItem("ielts-author-studio.confirmed-nas-export-dir.v1", ${JSON.stringify(nasDir)});
       return current.nasDestination;
     })()`,
  );
}

/** 草稿结构：把权威稿里有用的部分抠出来，供断言与报告使用。 */
function readDraftShape(ds) {
  const parts = ds?.listening?.parts ?? [];
  const answers = ds?.answerKey ?? {};
  const slots = Object.keys(ds?.answerSlots ?? {});
  const answered = slots.filter((slotId) => (answers[slotId]?.kind ?? "unresolved") !== "unresolved");
  return {
    modality: ds?.modality ?? null,
    scope: ds?.listening?.scope ?? null,
    partCount: parts.length,
    parts: parts.map((part) => ({
      partId: part.partId,
      displayLabel: part.displayLabel ?? null,
      expectedQuestionNumbers: part.expectedQuestionNumbers ?? [],
      taskIds: part.taskIds ?? [],
      media: part.media
        ? { assetId: part.media.assetId, sha256: part.media.sha256, mime: part.media.mime, probeStatus: part.media.probe?.status ?? null }
        : null,
    })),
    questionNumbers: parts.flatMap((part) => part.expectedQuestionNumbers ?? []),
    slotCount: slots.length,
    answeredCount: answered.length,
    unresolved: slots.filter((slotId) => (answers[slotId]?.kind ?? "unresolved") === "unresolved"),
    assetIds: (ds?.assets ?? []).map((asset) => asset.assetId),
  };
}

/** 草稿结构必须自洽：4 个 Part、题号 1–40 各一次、每个 Part 有 media 且 hash 与绑定文件一致。 */
function checkDraftShape(shape, expectedShaByPart) {
  const problems = [];
  if (shape.modality !== "listening") problems.push(`modality=${shape.modality}（应 listening）`);
  if (shape.partCount !== EXPECTED_PARTS) problems.push(`parts=${shape.partCount}（应 ${EXPECTED_PARTS}）`);
  const numbers = [...shape.questionNumbers].sort((a, b) => a - b);
  if (numbers.length !== EXPECTED_QUESTIONS) {
    problems.push(`题号总数 ${numbers.length}（应 ${EXPECTED_QUESTIONS}）`);
  } else if (new Set(numbers).size !== EXPECTED_QUESTIONS) {
    problems.push(`题号有重复：${numbers.filter((n, i) => numbers.indexOf(n) !== i).join(",")}`);
  } else if (numbers[0] !== 1 || numbers[numbers.length - 1] !== EXPECTED_QUESTIONS) {
    problems.push(`题号范围 ${numbers[0]}–${numbers[numbers.length - 1]}（应 1–40）`);
  }
  for (const part of shape.parts) {
    if (!part.media) {
      problems.push(`${part.partId} 没有 media`);
      continue;
    }
    if (part.media.probeStatus !== "passed") {
      problems.push(`${part.partId} 的 media 探针状态 ${part.media.probeStatus}（应 passed）`);
    }
    const expected = expectedShaByPart[part.partId] ?? null;
    if (!expected) {
      problems.push(`${part.partId} 没有「应该绑哪一段音频」的期望值`);
    } else if (String(part.media.sha256).toLowerCase() !== expected.toLowerCase()) {
      problems.push(
        `${part.partId} 的音频不是绑定的那一段：media=${String(part.media.sha256).slice(0, 12)} 期望=${expected.slice(0, 12)}`,
      );
    }
    if (!shape.assetIds.includes(part.media.assetId)) {
      problems.push(`${part.partId} 的 media assetId ${part.media.assetId} 不在稿件 assets 里`);
    }
  }
  return problems;
}

/**
 * 把「没填上的槽」归因到题组 —— 让报告自己说清是不是脚本的问题。
 *
 * 已有的实测：识别把某一组产成 `multiple_choice` / `unordered_set` 却**没有选项集合**
 * （`options=0`），前端就不为这些槽渲染任何 checkbox；界面上根本没有可点的控件，
 * 这不是「脚本没找对控件类型」。把「哪个题组、什么题型、有多少个选项」写成题组级事实，
 * 后来的人就不会跑去改填充逻辑。
 */
function diagnoseUnfilledSlots(ds, slotIds) {
  const wanted = new Set(slotIds);
  const groups = [];
  for (const group of ds?.taskGroups ?? []) {
    const owned = [];
    let responseOptionCount = 0;
    let assignment = null;
    for (const responseGroup of group.responseGroups ?? []) {
      const mine = (responseGroup.slotIds ?? []).filter((slotId) => wanted.has(slotId));
      if (mine.length === 0) continue;
      owned.push(...mine);
      responseOptionCount += (responseGroup.options ?? []).length;
      assignment = responseGroup.assignment ?? assignment;
    }
    if (owned.length === 0) continue;
    groups.push({
      taskId: group.taskId ?? null,
      taskType: group.taskType ?? null,
      assignment,
      slotIds: owned,
      responseOptionCount,
      optionBankCount: (group.optionBank?.options ?? []).length,
      interactions: [...new Set(owned.map((slotId) => ds?.answerSlots?.[slotId]?.interaction ?? null))],
    });
  }
  return { unfilledCount: slotIds.length, unfilled: [...slotIds], groups };
}

/**
 * 门禁结论的码统计。`blockers` 会被截断（`BLOCKER_LIST_TRUNCATED`），
 * 所以按**权威结论** `publishVerdict.reasons` 归类，读报告的人一眼能看出「发不了」的构成。
 */
function publishReasonTally(preflight) {
  const value = preflight?.value ?? preflight ?? null;
  const reasons = value?.publishVerdict?.reasons ?? [];
  const tally = {};
  for (const reason of reasons) {
    const code = reason?.code ?? "unknown";
    // `ISSUE_UNRESOLVED` 的 target 每条都不同，按 message 归并才有统计意义。
    const key = code === "ISSUE_UNRESOLVED"
      ? `ISSUE_UNRESOLVED:${String(reason?.message ?? "").slice(0, 28)}`
      : code;
    tally[key] = (tally[key] ?? 0) + 1;
  }
  return {
    status: value?.publishVerdict?.status ?? null,
    ready: value?.publishVerdict?.ready ?? null,
    reasonCount: reasons.length,
    tally,
  };
}

/** 取一个元素的可点矩形；元素自身尺寸为 0 时回退到它的 `<label>`（选项行）。 */
function rectExpr(finder) {
  return `(() => {
    const el = ${finder};
    if (!el) return null;
    el.scrollIntoView({ block: 'center', inline: 'center' });
    const own = el.getBoundingClientRect();
    const target = own.width > 0 && own.height > 0 ? el : (el.closest('label') ?? el);
    const r = target.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return null;
    return {
      x: r.x + r.width / 2, y: r.y + r.height / 2, w: r.width, h: r.height,
      tag: target.tagName, value: el.value ?? null, checked: el.checked ?? null,
      text: (el.closest('label') ?? el).innerText?.replace(/\\s+/g, ' ').trim().slice(0, 40) ?? null,
    };
  })()`;
}

/**
 * 真实鼠标点击某个槽位的控件：连续两次读到同一坐标再点（布局可能还在动）。
 *
 * 刻意不退回 `element.click()`：那会绕过命中的是哪个元素、有没有被遮挡，等于把
 * 「用户点得到吗」这个问题换掉。
 */
async function clickSlotControl(finder, { timeoutMs = 10000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let previous = null;
  let last = null;
  while (Date.now() < deadline) {
    const box = await session.evaluate(rectExpr(finder)).catch(() => null);
    if (box) {
      last = box;
      if (previous && previous.x === box.x && previous.y === box.y && previous.w === box.w && previous.h === box.h) {
        await session.clickAt(box.x, box.y);
        return box;
      }
      previous = box;
    }
    await sleep(120);
  }
  throw new Error(`等待控件位置稳定超时（${timeoutMs}ms）：${finder}，最后一次=${JSON.stringify(last)}`);
}

/**
 * 槽位的填空输入框。
 *
 * 同一个「填空槽」在产品里有**两种**渲染路径，类名不同：
 *   - 独立作答组：`<input class="v2-text-answer" type="text" name={slotId}>`（ExamCanvas:509）
 *   - 题干内联槽：`<input type="text" name={slot.slotId}>`，**没有** `v2-text-answer` 类（ExamCanvas:352）
 * 只按类名找会漏掉内联那一半，于是「填不满 40 个」被误读成产品缺陷。按 name 找才两种都覆盖。
 */
function textInputExpr(slotId) {
  return `[...document.querySelectorAll('input[name="' + ${JSON.stringify(slotId)} + '"]')].find((el) => el.type === 'text') ?? null`;
}

/** 一个槽位在**界面上**长什么样：选项类给可选 label，填空类给当前值。 */
function slotControlExpr(slotId) {
  return `(() => {
    const id = ${JSON.stringify(slotId)};
    const inputs = [...document.querySelectorAll('input[name="' + id + '"]')];
    const options = inputs.filter((el) => el.type === 'radio' || el.type === 'checkbox');
    if (options.length) {
      return {
        kind: 'option',
        labels: options.map((el) => el.value),
        checked: options.filter((el) => el.checked).map((el) => el.value),
      };
    }
    const text = inputs.find((el) => el.type === 'text');
    if (text) return { kind: 'text', value: text.value ?? '' };
    return null;
  })()`;
}

/** 按题型约束挑一个「用户会填」的值。 */
function plannedTextValue(slot) {
  const limit = slot?.constraints ?? {};
  const maxCharacters = Number.isFinite(limit.maxCharacters) ? limit.maxCharacters : null;
  // 一个词、零个纯数字 token：对 maxWords / maxNumbers / maxCharacters 都安全。
  if (maxCharacters !== null && maxCharacters > 0 && maxCharacters < 3) return "a";
  return "one";
}

/**
 * 共享选项库（`unordered_set`）作答面。
 *
 * `<fieldset class="v2-shared-selection">` 里的 checkbox **没有 `name` 属性**
 * （ExamCanvas 只给 `value={option.label}`），所以按 `input[name="qN"]` 找槽位的写法
 * 永远看不到它——9 个槽会被误读成「产品渲染不出控件」。
 *
 * 产品把这一组的勾选按顺序摊到各槽上（`onChange` 里 `response.slotIds.forEach(...)`），
 * 所以「勾 N 个选项」就等于「一轮填满 N 个槽」，真实用户也是这样一次勾完的。
 * 断言只用产品自己的 `v2-slot-chip[data-question-id]`，不读脚本内部的账。
 */
function sharedSelectionExpr(fieldsetIndex) {
  return `(() => {
    const fieldset = document.querySelectorAll('.v2-shared-selection')[${fieldsetIndex}] ?? null;
    if (!fieldset) return null;
    return {
      slots: [...fieldset.querySelectorAll('.v2-slot-chip')].map((chip) => {
        const text = chip.textContent || '';
        const separator = text.indexOf(':');
        return {
          slotId: chip.getAttribute('data-question-id'),
          value: separator >= 0 ? text.slice(separator + 1).trim() : '',
        };
      }),
      options: [...fieldset.querySelectorAll('input[type="checkbox"]')].map((el, index) => ({
        index,
        value: el.value,
        checked: el.checked,
        disabled: el.disabled,
      })),
    };
  })()`;
}

function sharedSelectionOptionExpr(fieldsetIndex, optionIndex) {
  return `(() => {
    const fieldset = document.querySelectorAll('.v2-shared-selection')[${fieldsetIndex}] ?? null;
    if (!fieldset) return null;
    return [...fieldset.querySelectorAll('input[type="checkbox"]')][${optionIndex}] ?? null;
  })()`;
}

/** 把当前页上每一组共享选项库都勾满；填不满就抛错，绝不静默放过。 */
async function fillSharedSelections(actions) {
  const groupCount = await session
    .evaluate(`document.querySelectorAll('.v2-shared-selection').length`)
    .catch(() => 0);
  for (let fieldsetIndex = 0; fieldsetIndex < groupCount; fieldsetIndex += 1) {
    const slotIds = [];
    for (let attempt = 0; attempt < 16; attempt += 1) {
      const state = await session.evaluate(sharedSelectionExpr(fieldsetIndex)).catch(() => null);
      if (!state) break;
      if (attempt === 0) slotIds.push(...state.slots.map((slot) => slot.slotId));
      const unfilled = state.slots.filter((slot) => !slot.value || slot.value === "—");
      if (unfilled.length === 0) break;
      const next = state.options.find((option) => !option.checked && !option.disabled);
      if (!next) break;
      const box = await clickSlotControl(sharedSelectionOptionExpr(fieldsetIndex, next.index));
      actions.push({
        slotId: state.slots.map((slot) => slot.slotId).join("+"),
        kind: "shared-option",
        label: next.value,
        text: box.text,
      });
      await sleep(150);
    }
    const settled = await session.evaluate(sharedSelectionExpr(fieldsetIndex)).catch(() => null);
    if (!settled) continue;
    const unfilled = settled.slots.filter((slot) => !slot.value || slot.value === "—");
    if (unfilled.length) {
      throw new Error(
        `共享选项库第 ${fieldsetIndex + 1} 组（${slotIds.join(", ")}）仍有 ${unfilled.length} 个槽没填满：`
        + unfilled.map((slot) => slot.slotId).join(", "),
      );
    }
    actions.push({
      slotId: slotIds.join("+"),
      kind: "shared-filled",
      value: settled.slots.map((slot) => `${slot.slotId}=${slot.value}`).join(","),
    });
  }
}

/**
 * 用户遇到「保存失败 / 冲突」时会点的那个按钮。
 *
 * 实测（2026-09-24，11:25 那次运行）：识别收尾阶段后端仍会改写权威稿
 * （`editVersion` 7→8→9），用户答题期间的保存会撞上版本冲突，界面如实报
 * 「保存失败，请重试」并给出 `workspace-save-retry`。此时**本地修改仍在编辑器里**
 * （截图 08b 可见 38/39/40 的输入框里已有值），但权威稿还没有它们——
 * 所以脚本必须像用户那样**重试保存**，而不是重新打字（重打同一个值不会让
 * 受控输入变脏，反而永远不会触发保存）。
 */
async function retrySaveIfOffered() {
  const offered = await session
    .evaluate(`!!document.querySelector('[data-testid="workspace-save-retry"]')`)
    .catch(() => false);
  if (!offered) return false;
  await session.clickSelectorWhenStable('[data-testid="workspace-save-retry"]', { timeoutMs: 15000 });
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    const stillThere = await session
      .evaluate(`!!document.querySelector('[data-testid="workspace-save-retry"]')`)
      .catch(() => true);
    if (!stillThere) return true;
    await sleep(1000);
  }
  return false;
}

async function fillAnswersThroughUi({ plannedBySlot, ordinals, timeoutMs = 90000 }) {
  const deadline = Date.now() + timeoutMs;
  const actions = [];
  for (const ordinal of ordinals) {
    await session.clickSelectorWhenStable(`.listening-part-nav > button:nth-of-type(${ordinal})`);
    await sleep(250);
    // 共享选项库先勾满：它的 checkbox 不带 name，下面的逐槽循环找不到它。
    await fillSharedSelections(actions);
    for (const [slotId, plan] of plannedBySlot) {
      if (Date.now() > deadline) break;
      const state = await session.evaluate(slotControlExpr(slotId)).catch(() => null);
      if (!state) continue;
      if (state.kind === "option") {
        if (state.checked.length) continue;
        const label = state.labels[0];
        if (!label) continue;
        const box = await clickSlotControl(
          `[...document.querySelectorAll('input[name="' + ${JSON.stringify(slotId)} + '"]')].find((el) => el.value === ${JSON.stringify(label)})`,
        );
        actions.push({ slotId, kind: "option", label, text: box.text });
      } else if (state.kind === "text") {
        if (String(state.value ?? "").trim()) continue;
        const value = plan.textValue;
        const box = await clickSlotControl(textInputExpr(slotId));
        await session.evaluate(
          `(() => { const el = ${textInputExpr(slotId)}; if (el && el.select) el.select(); return true; })()`,
        );
        await session.cdp.send("Input.insertText", { text: value });
        actions.push({ slotId, kind: "text", value, text: box.text });
      }
    }
  }
  return actions;
}

async function main() {
  const fresh = assertBuildFresh({ exePath, tolerateConcurrentEdits });
  report.identity.buildFresh = buildFreshReport(fresh);
  report.identity.exeSha256 = sha256File(exePath);

  if (!fs.existsSync(PAPER_SOURCE)) throw new CannotRunError(`听力夹具不存在：${PAPER_SOURCE}`);

  // ── 布景：试卷放进 harness 给「选择 PDF 文件夹」用的目录；4 段音频单独放 ──
  const pdfDir = path.join(runDir, "pdfs");
  const audioDir = path.join(runDir, "audio");
  fs.mkdirSync(pdfDir, { recursive: true });
  fs.mkdirSync(audioDir, { recursive: true });

  const paperPath = path.join(pdfDir, PAPER_NAME);
  fs.copyFileSync(PAPER_SOURCE, paperPath);

  const audioPaths = [];
  const expectedShaByPart = {};
  for (const [index, spec] of AUDIO_SPECS.entries()) {
    const target = path.join(audioDir, spec.name);
    writeSineWav(target, spec);
    audioPaths.push(target);
    const sha = sha256File(target);
    expectedShaByPart[`part-${index + 1}`] = sha;
    report.identity.parts.push({
      partId: `part-${index + 1}`,
      name: spec.name,
      seconds: spec.seconds,
      baseHz: spec.baseHz,
      sha256: sha,
      sizeBytes: fs.statSync(target).size,
    });
  }

  session = await launchTauriAppCdp({
    exePath,
    runDir,
    extraBrowserArgs: extraArgs,
    appEnv: {
      PDF2TEST_AUTOMATION_SOURCE_FILES: paperPath,
      PDF2TEST_AUTOMATION_AUDIO_FILES: audioPaths.join(path.delimiter),
    },
  });
  report.identity.browserArgs = session.browserArgs;
  recorder = createStepRecorder({ session, artifactsDir: runDir });

  await recorder.run("library-page-loads", async () => {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000, label: "library-page" });
    await session.screenshot("01-library");
    const rows = await session.evaluate(rowIdsExpr);
    if (rows.length !== 0) throw new Error(`预期题库页开始时为空，实际 ${rows.length} 行`);
    return { rowsAtStart: rows };
  });

  await recorder.run("import-drawer-takes-the-listening-paper", async () => {
    await session.clickSelectorWhenStable('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-drawer"]')`, { timeoutMs: 20000, label: "import-drawer" });
    // 抽屉里「未连接云端」那一行是**异步插入**的，会把按钮下推约 49px；
    // 先等它出现再点，否则会点在位移之前的空白处（真实用户手快也点空）。
    await session.waitFor(
      `!!document.querySelector('[data-testid="import-cloud-offline"]')`,
      { timeoutMs: 20000, label: "import-cloud-offline-row" },
    ).catch(() => null);
    await session.clickSelectorWhenStable('[data-testid="import-pick-files"]');
    const names = await session.waitFor(
      `(${pickedNamesExpr}).length === 1`,
      { timeoutMs: 20000, label: "picked-one-paper" }
    ).then(() => session.evaluate(pickedNamesExpr));
    if (names[0] !== PAPER_NAME) throw new Error(`已选清单应为 ${PAPER_NAME}，实际 ${JSON.stringify(names)}`);
    return { picked: names };
  });

  await recorder.run("listening-dialog-asks-for-part-audio", async () => {
    await session.clickSelectorWhenStable('[data-testid="import-start"]');
    await session.waitFor(
      `!!document.querySelector('[data-testid="listening-audio-dialog"]')`,
      { timeoutMs: 60000, label: "listening-audio-dialog" }
    );
    await session.screenshot("02-listening-dialog");
    return { dialog: "listening-audio-dialog" };
  });

  await recorder.run("real-picker-binds-four-parts-in-order", async () => {
    await session.clickSelectorWhenStable('[data-testid="listening-audio-pick-files"]');
    await session.waitFor(
      `(${partRowsExpr}).length === ${EXPECTED_PARTS}`,
      { timeoutMs: 30000, label: "four-parts" }
    );
    const rows = await session.evaluate(partRowsExpr);
    const names = rows.map((row) => row.name);
    const expected = AUDIO_SPECS.map((spec) => spec.name);
    // 顺序即 Part 顺序：这是产品语义，不是巧合。
    if (JSON.stringify(names) !== JSON.stringify(expected)) {
      throw new Error(`Part 顺序应等于文件顺序 ${JSON.stringify(expected)}，实际 ${JSON.stringify(names)}`);
    }
    const ordinals = rows.map((row) => row.part);
    if (JSON.stringify(ordinals) !== JSON.stringify([1, 2, 3, 4])) {
      throw new Error(`Part 序号应为 1..4，实际 ${JSON.stringify(ordinals)}`);
    }
    await session.screenshot("03-four-parts");
    return { parts: rows.map((row) => ({ part: row.part, name: row.name })) };
  });

  await recorder.run("every-part-probe-passes", async () => {
    // 探针是异步的：等每一行的状态类离开 pending。
    await session.waitFor(
      `(${partRowsExpr}).every(r => !r.probeClass.includes('audio-probe-pending'))`,
      { timeoutMs: 30000, label: "probes-settled" }
    );
    const rows = await session.evaluate(partRowsExpr);
    const blocked = rows.filter((row) => !row.probeClass.includes("audio-probe-passed"));
    if (blocked.length) {
      throw new Error(`这 4 段音频是探针单测同款的 440Hz 正弦，不该被拒；被拒：${JSON.stringify(blocked)}`);
    }
    await session.screenshot("04-probes-passed");
    return { probes: rows.map((row) => ({ part: row.part, text: row.probeText })) };
  });

  await recorder.run("confirm-imports-the-listening-item", async () => {
    await session.clickSelectorWhenStable('[data-testid="listening-confirm"]');
    await session.waitFor(`(${rowIdsExpr}).length === 1`, { timeoutMs: 60000, label: "one-item" });
    const rows = await session.evaluate(rowIdsExpr);
    await session.screenshot("05-imported");
    itemId = rows[0];
    report.identity.itemId = itemId;
    return { itemIds: rows };
  });

  // ── 第 6b 步：等 4 个 Part 的音频**全部**落库 ────────────────────────────
  //
  // 「条目建好了」不等于「音频绑好了」：`importFiles` 是「先建条目、再逐个 Part
  // 调 bind_listening_audio」。它把每个 Part 的失败收进 `rejected`，而 `rejected`
  // 只显示在**导入抽屉自己的 state** 里——抽屉在 `onImport` 时就被卸载了，于是
  // 绑定失败在界面上**静默消失**，用户只看到「已建立 1 个题目」。
  // 实测（2026-09-24）两次运行各丢 1 个 Part（一次 part-4、一次 part-2），
  // 界面与抽屉都没有报错。这里把「4 个都绑上」变成一条**确定的期望值**断言，
  // 失败时把应用输出与绑定实况一起写进报告，不留给下一跳去猜。
  await recorder.run("all-four-part-audio-bindings-are-persisted", async () => {
    const deadline = Date.now() + 30000;
    let status = null;
    let failure = null;
    // 第一次往返的**原始信封**留证：这个命令读不到东西时，「库里没有」与「读错了层级」
    // 是两种完全不同的结论，报告里必须能分辨。
    let firstReply = null;
    while (Date.now() < deadline) {
      const reply = await call("get_listening_audio", { itemId, verify: false }).catch((error) => ({
        ok: false,
        error: String(error?.message ?? error),
      }));
      if (firstReply === null) firstReply = JSON.stringify(reply)?.slice(0, 1200) ?? "(undefined)";
      if (!reply?.ok) {
        failure = reply?.error ?? "(no reply)";
      } else {
        failure = null;
        status = reply.value;
        if (Array.isArray(status?.bindings) && status.bindings.length === EXPECTED_PARTS) break;
      }
      await sleep(1000);
    }
    const bindings = (status?.bindings ?? []).map((binding) => ({
      partOrdinal: binding.partOrdinal,
      sha256: binding.sha256,
      originalName: binding.originalName,
      playable: binding.playable,
    }));
    const ordinals = bindings.map((binding) => binding.partOrdinal).sort((a, b) => a - b);
    const expectedOrdinals = Array.from({ length: EXPECTED_PARTS }, (_, index) => index + 1);
    const problems = [];
    if (failure) problems.push(`get_listening_audio 调用失败：${failure}`);
    if (JSON.stringify(ordinals) !== JSON.stringify(expectedOrdinals)) {
      problems.push(`已绑定的 Part 序号应为 ${JSON.stringify(expectedOrdinals)}，实际 ${JSON.stringify(ordinals)}`);
    }
    for (const binding of bindings) {
      const want = expectedShaByPart[`part-${binding.partOrdinal}`];
      if (want && binding.sha256 !== want) {
        problems.push(`part-${binding.partOrdinal} 的 sha256 应为 ${want}，实际 ${binding.sha256}`);
      }
      if (!binding.playable) problems.push(`part-${binding.partOrdinal} 的探针没通过`);
    }
    report.postChecks.push({
      name: "audio-bindings",
      problems,
      audioReady: status?.audioReady ?? null,
      blockers: status?.blockers ?? null,
      firstReply,
      bindings,
      appOutputTail: String(session.appOutput?.() ?? "").slice(-4000) || null,
    });
    if (problems.length) {
      throw new Error(
        `导入后 4 个 Part 的音频没有全部落库：${problems.join("；")}；`
        + `audioReady=${JSON.stringify(status?.audioReady)} blockers=${JSON.stringify(status?.blockers)}`,
      );
    }
    return { bindings, audioReady: status.audioReady };
  });

  // ── 第 7 步：等识别真的结束，再断言草稿结构 ─────────────────────────────
  //
  // 这一步是 F1 的核心。旧版在这里就去点发布，于是「识别还没跑完」被当成
  // 「发布路径给了一个机器可读结论」——任何结论都算过。判据必须是**确定的期望值**。
  await recorder.run("listening-draft-is-ready-and-structured", async () => {
    const workspaceOpenRetried = await openWorkspace();
    if (!workspaceOpenRetried) await sleep(1000);

    const deadline = Date.now() + RECOGNITION_TIMEOUT_MS;
    let shape = null;
    let note = null;
    let item = null;
    while (Date.now() < deadline) {
      // 两个信号都要看：界面副标题（用户看到什么）与权威稿是否落盘（后端事实）。
      note = await readProcessingNote().catch(() => null);
      const workspace = await readWorkspace().catch(() => null);
      item = workspace?.item ?? null;
      shape = workspace?.ds ? readDraftShape(workspace.ds) : null;
      if (shape && !note) break;
      await sleep(2000);
    }

    if (!shape) {
      throw new Error(
        `本地识别在 ${Math.round(RECOGNITION_TIMEOUT_MS / 1000)}s 内没有产出可编辑的题稿：`
        + `状态=${item?.status ?? "(读不到)"} hasCanonicalDs=${item?.hasCanonicalDs ?? "(读不到)"}`
        + ` 界面提示=${JSON.stringify(note)}`,
      );
    }
    if (note) {
      throw new Error(
        `题稿已存在但界面仍在处理中（${RECOGNITION_TIMEOUT_MS / 1000}s 超时）：提示=${JSON.stringify(note)}`,
      );
    }

    const problems = checkDraftShape(shape, expectedShaByPart);
    report.postChecks.push({ name: "draft-shape", problems, shape });
    if (problems.length) throw new Error(`草稿结构不符合预期：${problems.join("；")}`);

    await session.screenshot("06-draft-ready");
    return {
      workspaceOpenRetried,
      modality: shape.modality,
      parts: shape.parts.map((part) => ({ partId: part.partId, label: part.displayLabel, sha256: part.media?.sha256 })),
      questionNumbers: shape.questionNumbers.length,
      slotCount: shape.slotCount,
    };
  });

  // ── 第 8 步：ListeningHeader 的 Part 页签与「各放各的」播放器 ─────────────
  await recorder.run("listening-header-plays-each-part-from-its-own-file", async () => {
    await session.waitFor(`!!document.querySelector('[data-testid="listening-header"]')`, { timeoutMs: 30000, label: "listening-header" });
    const tabCount = await session.evaluate(
      `document.querySelectorAll('[data-testid="listening-header"] .listening-part-tab').length`,
    );
    if (tabCount !== EXPECTED_PARTS) throw new Error(`ListeningHeader 的 Part 页签应 ${EXPECTED_PARTS} 个，实际 ${tabCount}`);

    const workspace = await readWorkspace();
    const shape = readDraftShape(workspace.ds);
    const observed = [];
    for (const [index, spec] of AUDIO_SPECS.entries()) {
      const ordinal = index + 1;
      await session.clickSelectorWhenStable(`.listening-part-nav > button:nth-of-type(${ordinal})`);
      await session.waitFor(
        `!!document.querySelector('audio[data-testid="listening-audio-${ordinal}"]')`,
        { timeoutMs: 30000, label: `player-${ordinal}` },
      );
      // 播放器是异步取 src 的（`managedAudioUrl`），所以等它真的能播再断言。
      // 「可播」必须同时要求 src 已指向**本 part** 的音频：AudioPlayer 切换 Part 时
      // 组件状态会残留一个「prop 已换、effect 尚未清 src」的单帧窗口——testid 已经是
      // 新 part 的、src 还是上一段的（能播、时长也对得上另一段）。不排除这个瞬态，
      // 断言就会按机器负载间歇性误报（2026-09-25 run2 的 part-4）。
      const expectedSha = expectedShaByPart[`part-${ordinal}`];
      let playable = null;
      try {
        playable = await session.waitFor(
          `(() => {
             const a = document.querySelector('audio[data-testid="listening-audio-${ordinal}"]');
             if (!a) return null;
             const src = a.currentSrc || a.src || '';
             if (!src) return null;
             if (!decodeURIComponent(src).toLowerCase().includes(${JSON.stringify((expectedSha ?? "").toLowerCase())})) return null;
             if (!(a.readyState > 0)) return null;
             if (!Number.isFinite(a.duration) || a.duration <= 0) return null;
             return { src, readyState: a.readyState, duration: a.duration };
           })()`,
          { timeoutMs: 40000, label: `player-${ordinal}-loadable` },
        );
      } catch (error) {
        // 超时也要带着最后观测到的状态报错，否则只知道「没等到」不知道「等的时候是什么」。
        const snapshot = await session.evaluate(
          `(() => {
             const a = document.querySelector('audio[data-testid="listening-audio-${ordinal}"]');
             if (!a) return null;
             return { src: decodeURIComponent(a.currentSrc || a.src || ''), readyState: a.readyState, duration: a.duration };
           })()`,
        );
        throw new Error(`part-${ordinal} 播放器 40 秒内没有加载到本段音频（期望 sha ${expectedSha}）；最后观测：${JSON.stringify(snapshot)}；原始等待错误：${error.message}`);
      }
      const decoded = decodeURIComponent(playable.src);
      const problems = [];
      if (!/^(?:asset:|https?:\/\/asset\.localhost\/)/u.test(playable.src)) {
        problems.push(`src 协议不是受管资源（asset 协议）：${playable.src.slice(0, 80)}`);
      }
      // 受管文件的文件名就是内容哈希：src 必须指向**这一段**的音频，不是随便一段。
      if (!decoded.toLowerCase().includes(expectedSha.toLowerCase())) {
        problems.push(`src 指向的不是本 part 绑定的那一段音频：${decoded.slice(-80)}`);
      }
      // 时长与生成的 WAV 一致（±0.6s）：这是「播放器真的读到了这一段」的旁证。
      if (Math.abs(playable.duration - spec.seconds) > 0.6) {
        problems.push(`时长 ${playable.duration}s 与绑定的 ${spec.seconds}s 对不上`);
      }
      const declared = shape.parts.find((part) => part.partId === `part-${ordinal}`)?.media?.sha256 ?? null;
      if (declared && declared.toLowerCase() !== expectedSha.toLowerCase()) {
        problems.push(`草稿里 part-${ordinal} 的 media 哈希与绑定文件不一致`);
      }
      observed.push({ part: ordinal, src: decoded, readyState: playable.readyState, duration: playable.duration, problems });
    }
    const problems = observed.flatMap((row) => row.problems.map((p) => `part-${row.part}: ${p}`));
    report.postChecks.push({ name: "listening-players", observed: observed.map(({ problems: _p, ...rest }) => rest), problems });
    if (problems.length) throw new Error(`播放器没有各放各的音频：${problems.join("；")}`);
    await session.screenshot("07-part-players");
    return { tabCount, players: observed.map(({ problems: _p, ...rest }) => rest) };
  });

  // ── 第 9 步：像用户一样填完 40 个答案 ───────────────────────────────────
  //
  // 「这份卷子没有答案 key」不是「末端不可达」的理由：产品流程本来就是**用户补答案**。
  // 测试扮演用户，值都落在题型约束内（选项类点选项、填空类打字），不编造答案内容。
  await recorder.run("user-fills-forty-answers", async () => {
    const before = readDraftShape((await readWorkspace()).ds);
    if (before.unresolved.length === 0) throw new Error("导入后就已经没有待补答案，说明该断言的前提不成立");

    const plannedBySlot = new Map();
    for (const slotId of before.unresolved) {
      const slot = (await readWorkspace()).ds?.answerSlots?.[slotId] ?? null;
      plannedBySlot.set(slotId, { textValue: plannedTextValue(slot) });
    }

    const actions = [];
    const saveRetries = [];
    // 逐 Part 视图填；共享选择（unordered_set）一轮只落一个槽，所以最多跑 4 轮。
    for (let pass = 0; pass < 4; pass += 1) {
      const remaining = readDraftShape((await readWorkspace()).ds).unresolved;
      if (remaining.length === 0) break;
      const planned = new Map([...plannedBySlot].filter(([slotId]) => remaining.includes(slotId)));
      actions.push(...await fillAnswersThroughUi({ plannedBySlot: planned, ordinals: [1, 2, 3, 4] }));
      // 编辑器是防抖保存的：等这一轮的写入落库再决定要不要再来一轮。
      const settleDeadline = Date.now() + 30000;
      let progressed = false;
      while (Date.now() < settleDeadline) {
        const now = readDraftShape((await readWorkspace()).ds);
        if (now.unresolved.length === 0 || now.answeredCount > before.answeredCount) { progressed = true; break; }
        await sleep(1000);
      }
      // 这一轮的修改没有落库：先看是不是保存失败/冲突（用户会点「重试保存」）。
      if (!progressed) {
        const retried = await retrySaveIfOffered();
        saveRetries.push({ pass, retried });
        if (retried) {
          const afterRetry = Date.now() + 30000;
          while (Date.now() < afterRetry) {
            const now = readDraftShape((await readWorkspace()).ds);
            if (now.unresolved.length === 0 || now.answeredCount > before.answeredCount) break;
            await sleep(1000);
          }
        }
      }
    }

    // 最终一致性：等所有写入落库（防抖 + 队列）。
    const deadline = Date.now() + 60000;
    let after = null;
    while (Date.now() < deadline) {
      after = readDraftShape((await readWorkspace()).ds);
      if (after.unresolved.length === 0) break;
      await sleep(1500);
    }
    report.postChecks.push({
      name: "answer-fill",
      before: { slots: before.slotCount, unanswered: before.unresolved.length },
      after: { slots: after.slotCount, answered: after.answeredCount, unanswered: after.unresolved.length },
      saveRetries,
      actions,
    });
    if (after.unresolved.length !== 0) {
      // 填不满时先归因到题组：界面上没有控件，还是脚本没找对控件？这两种结论的后续动作完全不同。
      const diagnosis = diagnoseUnfilledSlots((await readWorkspace()).ds, after.unresolved);
      report.postChecks.push({ name: "unfilled-slots", problems: [], diagnosis });
      await session.screenshot("08b-unfilled-answers");
      throw new Error(
        `界面填了 ${actions.length} 次，仍有 ${after.unresolved.length} 个槽没有答案：${after.unresolved.join(", ")}；`
        + `归因=${JSON.stringify(diagnosis.groups)}`,
      );
    }
    if (after.answeredCount !== EXPECTED_QUESTIONS) {
      throw new Error(`已答槽位数 ${after.answeredCount}（应 ${EXPECTED_QUESTIONS}）——题号结构在填答案过程中变了`);
    }
    await session.screenshot("08-answers-filled");
    return {
      filled: actions.length,
      slots: after.slotCount,
      answered: after.answeredCount,
      byKind: actions.reduce((acc, action) => { acc[action.kind] = (acc[action.kind] ?? 0) + 1; return acc; }, {}),
    };
  });

  // ── 第 10 步：一键发布，断言**干净**结论 ────────────────────────────────
  await recorder.run("publish-reports-published", async () => {
    await configureNasDestination();
    const outcome = await session.publishAndReadOutcome({ timeoutMs: 240000 });
    report.postChecks.push({
      name: "publish-outcome",
      kind: outcome.kind,
      text: outcome.text,
      clean: isCleanPublishOutcome(outcome.kind),
      nasDestination: nasDir,
    });
    await session.screenshot("09-published");
    if (!isCleanPublishOutcome(outcome.kind)) {
      // 不干净就**报出门禁原因**，绝不放宽门禁：判据是「产品说可以发了」，
      // 不是「产品给了个结论」。原始 blocker 列表很长，另给一份码统计便于阅读。
      const preflight = await call("get_publish_preflight", { jobId: itemId }).catch(() => null);
      const tally = publishReasonTally(preflight);
      report.postChecks.push({ name: "publish-blockers", problems: [], ...tally });
      throw new Error(
        `期望 data-publish-outcome=published，实际 ${JSON.stringify(outcome.kind)}（提示：${JSON.stringify(outcome.text)}）；`
        + `门禁结论=${JSON.stringify(tally)}；`
        + `门禁明细=${JSON.stringify(preflight?.value ?? preflight ?? null)}`,
      );
    }
    if (!fs.existsSync(path.join(nasDir, "manifest.js"))) {
      throw new Error(`发布结论是 published 但 ${nasDir} 里没有 manifest.js`);
    }
    const workspace = await readWorkspace();
    const item = workspace?.item ?? {};
    if (item.status !== "published") {
      throw new Error(`发布结论是 published 但条目状态是 ${JSON.stringify(item.status)}`);
    }
    return { kind: outcome.kind, itemStatus: item.status, nasDestination: nasDir };
  });

  // ── 第 11 步：学生端真实代码逐 part 取音频 + 阅读目录不含这道听力卷 ──────
  await recorder.run("student-app-loads-each-part-audio", async () => {
    const outcome = await loadPublishedListeningPackageWithRealProviderAsync({
      packageDir: nasDir,
      studentRepo,
      expectedPartSha256: expectedShaByPart,
    });
    report.postChecks.push({
      name: "student-listening-provider",
      ok: outcome.ok,
      cannotRun: Boolean(outcome.cannotRun),
      studentRepo,
      providerPath: outcome.listeningProviderPath,
      examId: outcome.examId ?? null,
      observed: outcome.observed ?? null,
      results: outcome.results,
    });
    if (outcome.cannotRun) {
      // 「清单里没有听力条目」不是环境缺件，而是**上游发布没产出**：不能报成环境问题，
      // 否则「这一轮发布没成功」会被读成「学生端没构建」。
      const upstreamMissingPackage = outcome.cause === "no-listening-package";
      const message = upstreamMissingPackage
        ? `发布没有产出可加载的听力运行时，学生端没有包可读（本步以第 10 步的干净发布为前提）：${outcome.reason}`
        : `学生端真实 provider 无法运行（环境问题，先构建学生端）：${outcome.reason}`;
      if (upstreamMissingPackage) throw new Error(message);
      throw new CannotRunError(message);
    }
    if (!outcome.ok) {
      throw new Error(`学生端真实 provider 断言未通过：${outcome.failures.map((f) => `${f.name}(${f.detail})`).join("；")}`);
    }
    await session.screenshot("10-student-provider");
    return {
      examId: outcome.examId,
      parts: outcome.observed.parts,
      readingAssetIds: outcome.observed.readingAssetIds,
      providerPath: outcome.listeningProviderPath,
    };
  });
}

try {
  await main();
} catch (error) {
  const cannotRun = error instanceof CannotRunError;
  report.cannotRun = cannotRun;
  report.fatal = {
    name: error?.name ?? "Error",
    message: String(error?.message ?? error),
    appOutput: error?.appOutput ?? null,
  };
  console.error(`[listening-chain] ${cannotRun ? "CANNOT-RUN" : "FAIL"} ${error?.message ?? error}`);
} finally {
  if (session) {
    try { await session.close({ keep }); } catch {}
  }
  // `createStepRecorder` 把结果攒在自己的 `steps` 里，必须回填，否则报告里 steps 永远是空的。
  if (recorder) report.steps = recorder.steps;
  report.finishedAt = new Date().toISOString();
  const failedSteps = report.steps.filter((step) => step.status === "failed").map((step) => step.name);
  const blockedSteps = report.steps.filter((step) => step.status === "blocked").map((step) => step.name);
  report.summary = {
    passed: report.steps.filter((step) => step.status === "passed").length,
    failed: failedSteps.length,
    failedSteps,
    blocked: blockedSteps.length,
    blockedSteps,
  };
  // 判定**由步骤派生**，不另算一个答案：`recorder.run` 自己吞掉异常并记 failed，
  // 所以 main() 可能正常返回而某一步其实是红的 —— 只有从 steps 派生才不会写出
  // 「verdict=passed 却带着一条 failed」的自相矛盾报告。
  const baseVerdict = report.cannotRun ? "cannot-run" : failedSteps.length ? "failed" : "passed";
  // 应用自身输出（stdout+stderr）是后端失败唯一的第一手证据，无条件落盘。
  // 只在 cannot-run 分支写会丢掉「跑完了但后端报错」的那一半。
  report.appOutput = session?.appOutput?.() ?? null;
  // 断线重连必须可见（F3）：默认「发生过重连就不算干净通过」，要接受得显式声明。
  report.cdpReattaches = summarizeReattaches(session, { acceptReattaches });
  const reattach = applyReattachPolicy(baseVerdict, report.cdpReattaches.entries, { acceptReattaches });
  report.verdict = reattach.verdict;
  report.reattachWarning = reattach.warning;
  writeReport(runDir, report);
  const line = report.steps.map((step) => `${step.name}:${step.status}`).join(" | ");
  console.log(`[listening-chain] verdict=${report.verdict} report=${path.join(runDir, "report.json")}`);
  console.log(`[listening-chain] steps: ${line}`);
  console.log(
    `[listening-chain] cdpReattaches=${report.cdpReattaches.count}（策略 ${report.cdpReattaches.policy}）`
    + `${report.reattachWarning ? ` — ${report.reattachWarning}` : ""}`,
  );
  if (report.fatal) console.log(`[listening-chain] fatal=${report.fatal.message}`);
  for (const check of report.postChecks) console.log(`[listening-chain] postCheck ${JSON.stringify(check).slice(0, 600)}`);
  process.exit(report.verdict === "passed" || report.verdict === "passed_with_warnings"
    ? 0
    : report.verdict === "cannot-run" ? 3 : 1);
}
