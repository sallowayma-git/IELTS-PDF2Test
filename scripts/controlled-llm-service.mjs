#!/usr/bin/env node
// 受控模型服务（controlled LLM service）
//
// 目的：让「导入 → 云端识别 → 候选与决策」这条链路能在**确定性输入**上跑起来，
// 供前端做按钮级验证，也供人复现问题，而不必依赖任何真实模型供应商。
//
// 它只做一件事：对 OpenAI 兼容的 chat-completions 请求，返回
// `fixtures/controlled-llm/reading-outline.json` 这份固定答案。
//
// 用法：
//   node scripts/controlled-llm-service.mjs                 # 默认 127.0.0.1:11435
//   node scripts/controlled-llm-service.mjs --port 18080
//   node scripts/controlled-llm-service.mjs --fixture fixtures/controlled-llm/reading-outline.json
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

function parseArgs(argv) {
  const options = {
    port: 11435,
    host: '127.0.0.1',
    fixture: path.join(repoRoot, 'fixtures', 'controlled-llm', 'reading-outline.json'),
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--port') options.port = Number(argv[++index]);
    else if (arg === '--host') options.host = argv[++index];
    else if (arg === '--fixture') options.fixture = path.resolve(argv[++index]);
    else if (arg === '--help' || arg === '-h') {
      console.log('usage: node scripts/controlled-llm-service.mjs [--port N] [--host H] [--fixture FILE]');
      process.exit(0);
    }
  }
  if (!Number.isInteger(options.port) || options.port <= 0) {
    throw new Error(`invalid --port: ${options.port}`);
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

function summarizeRequest(body) {
  let parsed;
  try {
    parsed = JSON.parse(body);
  } catch {
    return { model: '<unparsable>', attachedParts: [] };
  }
  const parts = (parsed?.messages ?? [])
    .flatMap((message) => (Array.isArray(message?.content) ? message.content : []))
    .map((part) => part?.type ?? 'unknown');
  return { model: parsed?.model ?? '<missing>', attachedParts: parts };
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
    reply(200, { ok: true, service: 'controlled-llm', fixture: options.fixture });
    return;
  }
  // 网关拼的是 `{baseUrl}/chat/completions`；baseUrl 自带 /v1 时即 /v1/chat/completions。
  // 两种都接，避免因为 baseUrl 写法不同而得到一个看不懂的 404。
  if (request.method !== 'POST' || !url.pathname.endsWith('/chat/completions')) {
    reply(404, { error: `unsupported path: ${url.pathname}` });
    return;
  }

  const raw = await readBody(request);
  const summary = summarizeRequest(raw);
  console.log(
    `[controlled-llm] ${new Date().toISOString()} POST ${url.pathname} model=${summary.model} parts=[${summary.attachedParts.join(',')}] content=${answerContent.length}B`,
  );

  reply(200, {
    id: 'controlled-llm-0001',
    object: 'chat.completion',
    created: Math.floor(Date.now() / 1000),
    model: summary.model,
    choices: [{ index: 0, finish_reason: 'stop', message: { role: 'assistant', content: answerContent } }],
    usage: { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
  });
});

server.listen(options.port, options.host, () => {
  const baseUrl = `http://${options.host}:${options.port}/v1`;
  console.log(`[controlled-llm] listening on http://${options.host}:${options.port}`);
  console.log(`[controlled-llm] serving fixture: ${options.fixture}`);
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
