#!/usr/bin/env node
// 受控模型服务（controlled LLM service）
//
// 目的：让「导入 → 云端识别 → 候选与决策」这条链路能在**确定性输入**上跑起来，
// 供前端做按钮级验证，也供人复现问题，而不必依赖任何真实模型供应商。
//
// ── 它要回答三类请求，不是一个 ──────────────────────────────────────────────
//
// A3/A4 落地后，网关会向同一个 profile 发**五种**语义完全不同的请求：
//   1. `generate_pdf_reading_outline`（旧云端大纲识别）→ 固定返回 `reading-outline.json`；
//   2. `verify_source_answers`（A3 原文件核验）→ 必须回 `{findings:[…]}`，
//      且每条 `slotId` 都得是**本次请求里给过的**，`confirmed`/`contradicted`
//      还必须带 `quote` + 1-based `pageIndex`；
//   3. `adjudicate_divergence`（A4 分歧裁决）→ 必须回 `{rulings:[…]}`，
//      且 `decisionId` 同上，`value` 要与所选链的值逐字一致；
//   4. `generate_authoring_candidate`（**完整候选识别**，新主链的第一步）→
//      必须回一份**整卷 authoring 草稿**（passage / taskGroups / answerSlots /
//      answerKey），由 `--candidate` 指定的样本提供；
//   5. `repair_authoring_step`（**修复回合**，新主链的第二步）→ 必须回
//      `{callId,tool,arguments}`，tool 只能是 read_draft / read_source /
//      apply_edits / record_ruling / finish，由 `--plan` 指定的剧本驱动。
//
// 上一版**只**会返回第 1 种。于是 A3/A4 请求拿到的是一份 outline，被网关校验器
// 整份拒绝（`MODEL_INVALID_OUTPUT`），链状态退化成 `partial`/`unusable`——
// 「受控服务能跑通」这句话当时**并不覆盖** A3/A4，把它当成 A3/A4 已联通的证据是错的。
// 现在按请求里的标记分派，并从请求里**回指 id 与值**——这是唯一能满足校验器的做法：
// 静态样本无法预知本次请求的 slotId / decisionId / editVersion。
//
// ── 4/5 的剧本纪律（为什么不是「固定回一段 JSON」）───────────────────────────
//
// 修复回合的每一步都必须**从上一轮的真实 observation 里取值**：
//   · `editVersion` 只能从 `read_draft` 的真实返回里读；
//   · 结构改写要带回的来源依据（`sourceAnchors`）也只能从 `read_draft` 的真实返回里读；
//   · `record_ruling` 只能针对**本次上下文里确实列出的差异**。
// 所以剧本是「状态机 + 从 observation 取值」，不是静态应答。这样一来，
// 「受控服务只是把常量回显了一遍」这种质疑就不成立——它回不出它没读到的东西。
//
// 用法：
//   node scripts/controlled-llm-service.mjs                 # 默认 127.0.0.1:11435，mode=normal
//   node scripts/controlled-llm-service.mjs --port 18080
//   node scripts/controlled-llm-service.mjs --fixture fixtures/controlled-llm/reading-outline.json
//   node scripts/controlled-llm-service.mjs --mode partial   # 见下
//   node scripts/controlled-llm-service.mjs --candidate artifacts/.../authoring-candidate.json --plan artifacts/.../repair-plan.json
//
// `--mode` 控制 A3/A4 的行为，用来覆盖「无法判断 / 部分返回 / 调用失败」三种非成功态：
//   normal  （默认）A3 逐槽位 confirmed；A4 逐分歧选云端值（有值就选，否则 local，再否则 unresolved）
//   decline          A3 逐槽位 not_verifiable；A4 逐分歧 unresolved
//                    → 后端应报 `ADJUDICATION_DECLINED` + `chains.adjudication = partial`
//   partial          只回答第一项，其余**不回答**
//                    → 后端应报部分覆盖（source `partial` / adjudication `partial`）
//   fail             HTTP 500（网关应归类为模型调用失败，而不是「模型说没问题」）
//   garbage          返回合法 HTTP 200 但内容不是约定 JSON（校验器必须整份拒绝）
//
// 然后在应用的「LLM 配置」里新建一个 profile：
//   provider = OpenAiCompatible
//   baseUrl  = http://127.0.0.1:11435/v1
//   model    = controlled-outline-v1
//   apiKey   = 任意非空字符串（网关只做 bearer 透传，不校验）
//   forceJson = true
// 服务启动时会把可直接粘贴的这段配置打印出来。
//
// 注意：网关对明文 http 有白名单（`llm_gateway.rs::openai_chat_completions_endpoint`），
// 只允许 localhost / *.local / 回环与私有地址，所以这里绑定 127.0.0.1。

import http from 'node:http';
import { readFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const MODES = ['normal', 'decline', 'partial', 'fail', 'garbage'];

function parseArgs(argv) {
  const options = {
    port: 11435,
    host: '127.0.0.1',
    mode: 'normal',
    fixture: path.join(repoRoot, 'fixtures', 'controlled-llm', 'reading-outline.json'),
    candidate: null,
    plan: null,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--port') options.port = Number(argv[++index]);
    else if (arg === '--host') options.host = argv[++index];
    else if (arg === '--mode') options.mode = String(argv[++index]);
    else if (arg === '--fixture') options.fixture = path.resolve(argv[++index]);
    else if (arg === '--candidate') options.candidate = path.resolve(argv[++index]);
    else if (arg === '--plan') options.plan = path.resolve(argv[++index]);
    else if (arg === '--help' || arg === '-h') {
      console.log('usage: node scripts/controlled-llm-service.mjs [--port N] [--host H] [--mode normal|decline|partial|fail|garbage] [--fixture FILE] [--candidate FILE] [--plan FILE]');
      process.exit(0);
    }
  }
  if (!Number.isInteger(options.port) || options.port <= 0) {
    throw new Error(`invalid --port: ${options.port}`);
  }
  if (!MODES.includes(options.mode)) {
    throw new Error(`invalid --mode: ${options.mode}（可选：${MODES.join(' | ')}）`);
  }
  return options;
}

const options = parseArgs(process.argv.slice(2));

if (!existsSync(options.fixture)) {
  console.error(`fixture not found: ${options.fixture}`);
  process.exit(1);
}
const outline = JSON.parse(readFileSync(options.fixture, 'utf8'));
delete outline._comment;

// 归一后的回答文本：网关会先取 choices[0].message.content，再从中解析 JSON。
const answerContent = JSON.stringify(outline);

/** 完整候选样本（新主链第 1 步）。缺样本时如实报错，而不是回一份能被猜出来的空壳。 */
const authoringCandidate = (() => {
  if (!options.candidate) return null;
  if (!existsSync(options.candidate)) {
    console.error(`candidate not found: ${options.candidate}`);
    process.exit(1);
  }
  return JSON.parse(readFileSync(options.candidate, 'utf8'));
})();

/** 修复剧本（新主链第 2 步）。 */
const repairPlan = (() => {
  if (!options.plan) return null;
  if (!existsSync(options.plan)) {
    console.error(`plan not found: ${options.plan}`);
    process.exit(1);
  }
  return JSON.parse(readFileSync(options.plan, 'utf8'));
})();

function readBody(request) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    request.on('data', (chunk) => chunks.push(chunk));
    request.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    request.on('error', reject);
  });
}

/** 请求里的全部文本片段（system + user，含附件 part 之外的文字）。 */
function textOf(body) {
  let parsed;
  try {
    parsed = JSON.parse(body);
  } catch {
    return { model: '<unparsable>', text: '', attachedParts: [] };
  }
  const messages = parsed?.messages ?? [];
  const parts = messages.flatMap((message) => (Array.isArray(message?.content) ? message.content : []));
  const attachedParts = parts.map((part) => part?.type ?? 'unknown');
  const text = parts
    .filter((part) => part?.type === 'text')
    .map((part) => String(part?.text ?? ''))
    .join('\n');
  return { model: parsed?.model ?? '<missing>', text, attachedParts };
}

/** 任务种类由 prompt 里的标记判定（与 `llm_gateway.rs` 的 prompt 构造器一一对应）。 */
function detectTask(text) {
  if (text.includes('--- SLOTS BEGIN ---')) return 'verify_source_answers';
  if (text.includes('--- DIVERGENCES BEGIN ---')) return 'adjudicate_divergence';
  // 这两句是 `llm_gateway.rs::repair_step_prompt` / `authoring_candidate_prompt` 的
  // 首句，逐字取自源码——不靠「大概像不像」猜，避免 prompt 改了之后静默串到别的分支。
  if (text.includes('You are repairing an IELTS Reading authoring draft so it matches the ORIGINAL FILE.')) {
    return 'repair_authoring_step';
  }
  if (text.includes('You are recognising an IELTS Reading paper from its ORIGINAL FILE into a COMPLETE authoring draft.')) {
    return 'generate_authoring_candidate';
  }
  return 'generate_pdf_reading_outline';
}

/** 取出 `--- XXX BEGIN ---\n{json}\n--- XXX END ---` 里的 JSON。 */
function embeddedJson(text, begin, end) {
  const start = text.indexOf(begin);
  const stop = text.indexOf(end);
  if (start < 0 || stop < 0 || stop <= start) return null;
  const raw = text.slice(start + begin.length, stop).trim();
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

/** 一个答案值「有没有实质内容」——决定 A4 能不能选它。 */
function valuePresent(value) {
  if (value === null || value === undefined) return false;
  if (typeof value === 'object') {
    if (value.kind === 'unresolved') return false;
    if (Array.isArray(value.values)) return value.values.some((entry) => String(entry).trim().length > 0);
    if (Array.isArray(value.labels)) return value.labels.length > 0;
    return Object.keys(value).length > 0;
  }
  return String(value).trim().length > 0;
}

/**
 * A3 的回答。**必须回指请求里的 slotId**，否则网关整份拒绝
 * （`source_verification_finding_unknown_slot`）。
 */
function sourceVerificationReply(slots) {
  const list = Array.isArray(slots) ? slots : [];
  const answered = options.mode === 'partial' ? list.slice(0, 1) : list;
  return {
    findings: answered.map((slot) => {
      const slotId = slot?.slotId;
      const questionNumber = Number(slot?.questionNumber ?? 0);
      if (options.mode === 'decline') {
        // 无法判断：如实说读不出来。**不**给 quote——没有出处的「确认」等于编造。
        return { slotId, questionNumber, verdict: 'not_verifiable', confidence: null };
      }
      const localValue = slot?.localValue;
      const shown = Array.isArray(localValue?.values) ? localValue.values[0] : '';
      return {
        slotId,
        questionNumber,
        verdict: 'confirmed',
        // 出处的形态与真实模型一致：一段原文 + 1-based 页码。
        // 受控服务固定应答，所以这段引文是**合成**的；它的作用是把「有出处的核验结果」
        // 这条链路走通，不代表任何真实文件里真的有这句话。
        quote: `Controlled evidence for question ${questionNumber}: ${String(shown)}`,
        pageIndex: 1,
        confidence: 0.9,
      };
    }),
  };
}

/**
 * A4 的回答。`value` 必须与所选链的值**逐字一致**，否则后端会以
 * `ADJUDICATION_VALUE_NOT_CORROBORATED` 拒绝这条裁定——这正是「模型只能选择、不能发明」。
 */
function adjudicationReply(divergences) {
  const list = Array.isArray(divergences) ? divergences : [];
  const answered = options.mode === 'partial' ? list.slice(0, 1) : list;
  return {
    rulings: answered.map((item) => {
      const decisionId = item?.decisionId;
      if (options.mode === 'decline') {
        return {
          decisionId,
          chosen: 'unresolved',
          confidence: null,
          rationale: '受控服务在 decline 模式下不裁定（模拟「原文不足以定论」）。',
        };
      }
      // 选一条**确实有值**的链；三条都没有值就只能 unresolved。
      const candidates = [['cloud', item?.cloud], ['local', item?.local], ['source', item?.source]];
      const picked = candidates.find(([, value]) => valuePresent(value));
      if (!picked) {
        return {
          decisionId,
          chosen: 'unresolved',
          confidence: null,
          rationale: '三条链都没有给出值，受控服务不发明第四个值。',
        };
      }
      const [chain, value] = picked;
      return {
        decisionId,
        chosen: chain,
        // 逐字回传所选链的值——后端会拿它和那条链比对，不一致就拒。
        value,
        confidence: 0.8,
        rationale: `受控服务选择 ${chain} 链：它是本次分歧中唯一给出值的来源。`,
      };
    }),
  };
}

/**
 * 从修复回合的 prompt 里取回网关嵌进去的输入信封（`Input JSON: {...}` 之后的全部内容）。
 *
 * 必须先按 JSON 解析信封再取文本：prompt 是 `messages[1].content` 里的一个 text part，
 * 直接从原始字节里找 `Input JSON: ` 会拿到一层 `\"` 转义，解析必然失败——失败的表现是
 * 「剧本读不到版本号 → 编辑被 CAS 拒」，看起来像产品没接通，其实是受控服务没读懂请求。
 */
function repairInput(text) {
  const marker = 'Input JSON: ';
  const at = text.lastIndexOf(marker);
  if (at < 0) return null;
  try {
    return JSON.parse(text.slice(at + marker.length).trim());
  } catch {
    return null;
  }
}

/** 观察结果里最后一次 `read_draft` 的真实返回（含 editVersion 与整卷结构）。 */
function lastDraftObservation(observations) {
  for (let index = (observations ?? []).length - 1; index >= 0; index -= 1) {
    const result = observations[index]?.result;
    if (result && typeof result === 'object' && result.editVersion !== undefined && Array.isArray(result.taskGroups)) {
      return result;
    }
  }
  return null;
}

/** 某个观察结果里的错误文本（用来证明「下一轮是照着真实反馈改的」）。 */
function observationErrors(observation) {
  const errors = observation?.errors;
  if (Array.isArray(errors)) return errors.map(String);
  return [];
}

/** 把一段节点树里第一个 text 节点的文字换成新值；找不到就返回原样。 */
function replaceFirstText(nodes, nextText) {
  let done = false;
  const walk = (node) => {
    if (done || !node || typeof node !== 'object') return node;
    if (Array.isArray(node)) return node.map(walk);
    const copy = { ...node };
    if (!done && copy.type === 'text' && typeof copy.text === 'string') {
      copy.text = nextText;
      done = true;
      return copy;
    }
    if (Array.isArray(copy.children)) copy.children = copy.children.map(walk);
    if (Array.isArray(copy.content)) copy.content = copy.content.map(walk);
    return copy;
  };
  const out = walk(nodes);
  return { nodes: out, done };
}

/** 从锚点里取一个可用的 `{sourceFileId, pageIndex}`——证据必须落在真实来源上。 */
function anchorLocation(target) {
  const anchors = target?.sourceAnchors;
  if (Array.isArray(anchors)) {
    for (const anchor of anchors) {
      const pageIndex = Number(anchor?.pageIndex ?? 0);
      const sourceFileId = anchor?.sourceFileId;
      if (sourceFileId && pageIndex >= 1) return { sourceFileId, pageIndex };
    }
  }
  return null;
}

/**
 * 修复回合的剧本。
 *
 * 五轮，每一步都只用**上一轮真实 observation 里读到的东西**：
 *   1. `read_draft`      —— 拿到 editVersion 与整卷结构（含来源依据）
 *   2. `apply_edits`     —— **故意漏掉 baseVersion**：真实模型最常见的第一次失手。
 *                           后端回 `CLOUD_EDIT_BASE_VERSION_MISSING`，这一轮什么也没写。
 *   3. `apply_edits`     —— 用第 1 轮真实读到的 editVersion 重交，并把来源依据原样带回。
 *                           这一批**真的落库**，是本场景「云端自动修改」的唯一来源。
 *   4. `record_ruling`   —— 对剩余差异里「当前稿对、候选错」的那些作出裁定，不写内容。
 *   5. `finish`          —— 把**确实无法定论**的疑问交出去（它们会变成用户可见的剩余任务）。
 *
 * 剧本拿不到目标（样本/结构对不上）时**不编造**：直接 `finish` 并如实说明，让这一轮
 * 以「什么都没改」结束。宁可报告为空，也不要制造一条假的成功。
 */
function repairStepReply(text) {
  const input = repairInput(text);
  if (!input) {
    return { callId: 'c1', tool: 'finish', arguments: { note: '受控服务无法解析请求信封' } };
  }
  const observations = Array.isArray(input.observations) ? input.observations : [];
  const round = observations.length + 1;
  const context = input.context ?? {};
  const draft = lastDraftObservation(observations);

  const plan = repairPlan ?? {};
  const giveUp = (note) => ({ callId: `c${round}`, tool: 'finish', arguments: { note } });

  if (round === 1) {
    return { callId: 'c1', tool: 'read_draft', arguments: {} };
  }

  // ── 目标定位（只用真实读到的结构）──
  const fixTaskId = plan.fixTaskId;
  const fixResponseGroupId = plan.fixResponseGroupId;
  const fixGroup = (draft?.taskGroups ?? []).find((group) => group?.taskId === fixTaskId);
  const fixResponseGroup = (fixGroup?.responseGroups ?? []).find(
    (group) => group?.responseGroupId === fixResponseGroupId,
  );
  const fixLocation = anchorLocation(fixResponseGroup);
  const rewritten = fixResponseGroup
    ? replaceFirstText(fixResponseGroup.prompt, plan.fixedPromptText)
    : { nodes: null, done: false };

  // 结构改写是**整块替换**：来源依据必须原样带回，否则「这段文字出自哪一页」就没了，
  // 质量门禁会逐个点名拒绝（`PROVENANCE_MISSING`）。依据只能从第 1 轮的真实返回里取。
  const fixCommands = () => [
    {
      op: 'setResponseGroup',
      taskId: fixTaskId,
      responseGroup: { ...fixResponseGroup, prompt: rewritten.nodes },
    },
  ];
  const fixEvidence = () => [
    {
      sourceFileId: fixLocation?.sourceFileId ?? context.sourceFileId,
      pageIndex: fixLocation?.pageIndex ?? 1,
      quote: plan.evidenceQuote ?? 'BLANK PAGE',
    },
  ];

  if (round === 2) {
    if (!fixResponseGroup || !rewritten.done || !fixLocation) {
      return giveUp('受控服务在真实稿里找不到剧本指定的作答结构，本轮不做任何修改');
    }
    // 有意**不带** `baseVersion`：这一轮一定被拒，用来证明「被拒之后是照着真实错误改的」。
    return {
      callId: 'c2',
      tool: 'apply_edits',
      arguments: { commands: fixCommands() },
    };
  }

  if (round === 3) {
    if (!fixResponseGroup || !rewritten.done || !fixLocation) {
      return giveUp('受控服务在真实稿里找不到剧本指定的作答结构，本轮不做任何修改');
    }
    // `baseVersion` 只能来自第 1 轮 `read_draft` 的真实返回；第 2 轮被拒的错误文本也
    // 在这里被读到（下一轮报告据此断言「它是照着真实反馈改的」）。
    const version = draft?.editVersion;
    if (typeof version !== 'number') {
      return giveUp('受控服务没有读到真实 editVersion，不能提交编辑');
    }
    const rejected = observationErrors(observations[1]);
    if (!rejected.some((line) => line.includes('CLOUD_EDIT_BASE_VERSION_MISSING'))) {
      return giveUp('第 2 轮没有被 baseVersion 规则拒绝，剧本前提不成立');
    }
    return {
      callId: 'c3',
      tool: 'apply_edits',
      arguments: { baseVersion: version, commands: fixCommands(), evidence: fixEvidence() },
    };
  }

  if (round === 4) {
    const differences = Array.isArray(context.differences) ? context.differences : [];
    const wanted = Array.isArray(plan.rulings) ? plan.rulings : [];
    const rulings = [];
    for (const entry of wanted) {
      const listed = differences.find(
        (difference) =>
          difference?.targetType === entry.targetType
          && difference?.targetId === entry.targetId
          && difference?.field === entry.field,
      );
      if (!listed) continue;
      const target =
        entry.targetType === 'task_group'
          ? (draft?.taskGroups ?? []).find((group) => group?.taskId === entry.targetId)
          : null;
      const location = anchorLocation(target) ?? fixLocation;
      rulings.push({
        targetType: entry.targetType,
        targetId: entry.targetId,
        field: entry.field,
        ruling: entry.ruling ?? 'current_is_correct',
        reason: entry.reason ?? '原文件与当前稿一致，候选读错了。',
        evidence: [
          {
            sourceFileId: location?.sourceFileId ?? context.sourceFileId,
            pageIndex: location?.pageIndex ?? 1,
            quote: entry.quote ?? 'Questions',
          },
        ],
      });
    }
    if (rulings.length === 0) {
      // 差异已经被改掉或本来就不存在：不硬造裁定（后端也会拒），直接进入收尾。
      return {
        callId: 'c4',
        tool: 'finish',
        arguments: { note: '没有需要裁定的差异', unresolved: unresolvedFrom(plan, context) },
      };
    }
    return { callId: 'c4', tool: 'record_ruling', arguments: { rulings } };
  }

  return {
    callId: `c${round}`,
    tool: 'finish',
    arguments: {
      note: plan.finishNote ?? '受控服务：已按原文件修正作答结构，并留下无法定论的疑问',
      unresolved: unresolvedFrom(plan, context),
    },
  };
}

/** `finish.unresolved`：模型**确实无法定论**的疑问，会变成用户可见的剩余任务。 */
function unresolvedFrom(plan, context) {
  const sourceFileId = context?.sourceFileId;
  return (Array.isArray(plan?.unresolved) ? plan.unresolved : []).map((entry) => ({
    ...(entry.targetId ? { targetId: entry.targetId } : {}),
    message: entry.message,
    evidence: [
      {
        sourceFileId,
        pageIndex: Number(entry.pageIndex ?? 1),
        quote: entry.quote ?? entry.message,
      },
    ],
  }));
}

/** 完整候选识别：样本里就是整卷草稿，原样返回。 */
function authoringCandidateReply() {
  if (!authoringCandidate) {
    return {
      taskGroups: [],
      answerSlots: {},
      note: '受控服务没有拿到 --candidate 样本',
    };
  }
  return authoringCandidate;
}

function replyFor(task, text) {
  if (task === 'verify_source_answers') {
    return sourceVerificationReply(embeddedJson(text, '--- SLOTS BEGIN ---', '--- SLOTS END ---'));
  }
  if (task === 'adjudicate_divergence') {
    return adjudicationReply(embeddedJson(text, '--- DIVERGENCES BEGIN ---', '--- DIVERGENCES END ---'));
  }
  if (task === 'generate_authoring_candidate') return authoringCandidateReply();
  if (task === 'repair_authoring_step') return repairStepReply(text);
  return outline;
}

const server = http.createServer(async (request, response) => {
  const url = new URL(request.url ?? '/', `http://${options.host}:${options.port}`);
  const reply = (status, payload) => {
    const body = JSON.stringify(payload);
    response.writeHead(status, {
      'content-type': 'application/json; charset=utf-8',
      'content-length': Buffer.byteLength(body),
    });
    response.end(body);
  };

  if (request.method === 'GET' && (url.pathname === '/health' || url.pathname === '/')) {
    reply(200, {
      ok: true,
      service: 'controlled-llm',
      mode: options.mode,
      fixture: options.fixture,
      candidate: options.candidate,
      plan: options.plan,
    });
    return;
  }
  // 网关拼的是 `{baseUrl}/chat/completions`；baseUrl 自带 /v1 时即 /v1/chat/completions。
  // 两种都接，避免因为 baseUrl 写法不同而得到一个看不懂的 404。
  if (request.method !== 'POST' || !url.pathname.endsWith('/chat/completions')) {
    reply(404, { error: `unsupported path: ${url.pathname}` });
    return;
  }

  const raw = await readBody(request);
  const { model, text, attachedParts } = textOf(raw);
  const task = detectTask(text);

  // `fail` 只影响 A3/A4：outline 仍然照常返回，否则整个导入链路会因为拿不到云端结果
  // 而提前失败，测不到「A3 调用失败」这一条。
  const failThisTask = options.mode === 'fail' && task !== 'generate_pdf_reading_outline';
  const garbageThisTask = options.mode === 'garbage' && task !== 'generate_pdf_reading_outline';

  const content = garbageThisTask
    ? JSON.stringify({ note: '受控服务在 garbage 模式下刻意返回不符合约定的 JSON。' })
    : JSON.stringify(replyFor(task, text));

  console.log(
    `[controlled-llm] ${new Date().toISOString()} POST ${url.pathname} mode=${options.mode} task=${task} `
      + `model=${model} parts=[${attachedParts.join(',')}] content=${content.length}B`
      + `${failThisTask ? ' -> 500' : ''}${garbageThisTask ? ' -> 非约定 JSON' : ''}`,
  );

  if (failThisTask) {
    reply(500, { error: { message: 'controlled-llm: injected failure', type: 'controlled_failure' } });
    return;
  }

  reply(200, {
    id: 'controlled-llm-0001',
    object: 'chat.completion',
    created: Math.floor(Date.now() / 1000),
    model,
    choices: [{ index: 0, finish_reason: 'stop', message: { role: 'assistant', content } }],
    usage: { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
  });
});

server.listen(options.port, options.host, () => {
  const baseUrl = `http://${options.host}:${options.port}/v1`;
  console.log(`[controlled-llm] listening on http://${options.host}:${options.port}`);
  console.log(`[controlled-llm] mode=${options.mode} fixture: ${options.fixture}`);
  if (options.candidate) console.log(`[controlled-llm] candidate: ${options.candidate}`);
  if (options.plan) console.log(`[controlled-llm] repair plan: ${options.plan}`);
  console.log('[controlled-llm] profile to paste into the app (LLM 配置):');
  console.log(
    JSON.stringify(
      {
        profileId: 'controlled-outline',
        name: 'Controlled Outline Service',
        provider: 'OpenAiCompatible',
        baseUrl,
        model: 'controlled-outline-v1',
        temperature: 0,
        timeoutMs: 60000,
        forceJson: true,
        enabled: true,
      },
      null,
      2,
    ),
  );
});

