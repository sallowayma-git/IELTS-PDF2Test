#!/usr/bin/env node
// 受控模型服务（controlled LLM service）
//
// 目的：让「导入 → 云端识别 → 候选与决策」这条链路能在**确定性输入**上跑起来，
// 供前端做按钮级验证，也供人复现问题，而不必依赖任何真实模型供应商。
//
// ── 它要回答三类请求，不是一个 ──────────────────────────────────────────────
//
// A3/A4 落地后，网关会向同一个 profile 发**三种**语义完全不同的请求：
//   1. `generate_pdf_reading_outline`（云端识别）→ 固定返回 `reading-outline.json`；
//   2. `verify_source_answers`（A3 原文件核验）→ 必须回 `{findings:[…]}`，
//      且每条 `slotId` 都得是**本次请求里给过的**，`confirmed`/`contradicted`
//      还必须带 `quote` + 1-based `pageIndex`；
//   3. `adjudicate_divergence`（A4 分歧裁决）→ 必须回 `{rulings:[…]}`，
//      且 `decisionId` 同上，`value` 要与所选链的值逐字一致。
//
// 上一版**只**会返回第 1 种。于是 A3/A4 请求拿到的是一份 outline，被网关校验器
// 整份拒绝（`MODEL_INVALID_OUTPUT`），链状态退化成 `partial`/`unusable`——
// 「受控服务能跑通」这句话当时**并不覆盖** A3/A4，把它当成 A3/A4 已联通的证据是错的。
// 现在按请求里的标记分派（A3 的 prompt 有 `--- SLOTS BEGIN ---`，A4 有
// `--- DIVERGENCES BEGIN ---`），并从请求里**回指 id 与值**——这是唯一能满足
// 校验器的做法：静态样本无法预知本次请求的 slotId / decisionId。
//
// 用法：
//   node scripts/controlled-llm-service.mjs                 # 默认 127.0.0.1:11435，mode=normal
//   node scripts/controlled-llm-service.mjs --port 18080
//   node scripts/controlled-llm-service.mjs --fixture fixtures/controlled-llm/reading-outline.json
//   node scripts/controlled-llm-service.mjs --mode partial   # 见下
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
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--port') options.port = Number(argv[++index]);
    else if (arg === '--host') options.host = argv[++index];
    else if (arg === '--mode') options.mode = String(argv[++index]);
    else if (arg === '--fixture') options.fixture = path.resolve(argv[++index]);
    else if (arg === '--help' || arg === '-h') {
      console.log('usage: node scripts/controlled-llm-service.mjs [--port N] [--host H] [--mode normal|decline|partial|fail|garbage] [--fixture FILE]');
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

/** 任务种类由 prompt 里的标记判定（与 `llm_gateway.rs` 的三个 prompt 构造器一一对应）。 */
function detectTask(text) {
  if (text.includes('--- SLOTS BEGIN ---')) return 'verify_source_answers';
  if (text.includes('--- DIVERGENCES BEGIN ---')) return 'adjudicate_divergence';
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

function replyFor(task, text) {
  if (task === 'verify_source_answers') {
    return sourceVerificationReply(embeddedJson(text, '--- SLOTS BEGIN ---', '--- SLOTS END ---'));
  }
  if (task === 'adjudicate_divergence') {
    return adjudicationReply(embeddedJson(text, '--- DIVERGENCES BEGIN ---', '--- DIVERGENCES END ---'));
  }
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
    reply(200, { ok: true, service: 'controlled-llm', mode: options.mode, fixture: options.fixture });
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

