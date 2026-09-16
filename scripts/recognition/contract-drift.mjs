#!/usr/bin/env node
// 识别闭环契约漂移检查（唯一真源：src-tauri/src/schema/recognition_v1.rs）。
//
// 为什么需要它：契约有三个消费面，历史上真实漂移过两次——
//   1) `DecisionFieldV1::as_str()` 返回 camelCase，而 serde 是 snake_case，
//      导致 decisionId 末段与线上 `field` 不一致（已修，并有 Rust 侧护栏）；
//   2) 前端 `src/api/recognitionClient.ts` 按 v1 设计文档写成，
//      读 `view.items` / `view.currentEditVersion`，而线上是
//      `actionable`+`autoApplied` / `editVersion`（面板因此永远显示「没有问题」）。
//
// 本脚本把三个面逐字段、逐枚举值对齐，任何不一致都以非零退出码报出。
//
// 用法：
//   node scripts/recognition/contract-drift.mjs          # 检查
//   node scripts/recognition/contract-drift.mjs --json   # 机器可读输出
//
// 退出码：0 = 一致；1 = 发现漂移。

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const RUST = join(ROOT, "src-tauri", "src", "schema", "recognition_v1.rs");
const TS = join(ROOT, "src", "api", "recognitionClient.ts");
const SCHEMA_VIEW = join(ROOT, "contracts", "recognition-decision-view-v1.schema.json");
const SCHEMA_APPLY = join(ROOT, "contracts", "recognition-apply-decisions-v1.schema.json");

const read = (path) => readFileSync(path, "utf8");

// ── 命名转换 ──────────────────────────────────────────────────────────
const toCamel = (s) => s.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());
const toSnake = (s) => s.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();

// ── Rust 解析 ─────────────────────────────────────────────────────────
/** 取某位置之前最近的一条 `#[serde(rename_all = "...")]`，缺省 camelCase。 */
function renameAllBefore(src, index) {
  const head = src.slice(0, index);
  const attributes = [...head.matchAll(/#\[serde\(([^)]*)\)\]/g)];
  for (let i = attributes.length - 1; i >= 0; i -= 1) {
    const match = /rename_all\s*=\s*"([^"]+)"/.exec(attributes[i][1]);
    if (match) return match[1];
  }
  return "camelCase";
}

function parseRustStructs(src) {
  const structs = new Map();
  for (const match of src.matchAll(/pub struct (\w+)\s*\{/g)) {
    const name = match[1];
    const start = match.index + match[0].length;
    const end = src.indexOf("\n}", start);
    const body = src.slice(start, end === -1 ? undefined : end);
    const style = renameAllBefore(src, match.index);
    const fields = [];
    for (const line of body.split("\n")) {
      const field = /^\s*pub ([a-z0-9_]+)\s*:/.exec(line);
      if (!field) continue;
      fields.push(style === "snake_case" ? field[1] : toCamel(field[1]));
    }
    structs.set(name, fields);
  }
  return structs;
}

function parseRustEnums(src) {
  const enums = new Map();
  for (const match of src.matchAll(/pub enum (\w+)\s*\{/g)) {
    const name = match[1];
    const start = match.index + match[0].length;
    const end = src.indexOf("\n}", start);
    const body = src.slice(start, end === -1 ? undefined : end);
    const style = renameAllBefore(src, match.index);
    const variants = [];
    for (const line of body.split("\n")) {
      const variant = /^\s*([A-Z]\w*)\s*,/.exec(line);
      if (!variant) continue;
      variants.push(style === "snake_case" ? toSnake(variant[1]) : variant[1][0].toLowerCase() + variant[1].slice(1));
    }
    enums.set(name, variants);
  }
  return enums;
}

/** 按大括号深度，只取一层字段（跳过嵌套对象字面量的键）。 */
function parseTsInterfaces(src) {
  const interfaces = new Map();
  for (const match of src.matchAll(/export interface (\w+)\s*\{/g)) {
    const name = match[1];
    let depth = 1;
    let cursor = match.index + match[0].length;
    const fields = [];
    while (cursor < src.length && depth > 0) {
      const character = src[cursor];
      if (character === "{") depth += 1;
      if (character === "}") depth -= 1;
      if (depth === 1) {
        const rest = src.slice(cursor);
        const field = /^([A-Za-z_]\w*)(\?)?\s*:\s*([^;\n]+)/.exec(rest);
        if (field) {
          fields.push(field[1]);
          // 值里可能自带花括号（如 `decisions: Array<{ id: string; action: ... }>`），
          // 跳过去时必须同步修正深度，否则嵌套对象的键会被误当成字段。
          const opened = (field[0].match(/\{/g) ?? []).length;
          const closed = (field[0].match(/\}/g) ?? []).length;
          depth += opened - closed;
          cursor += field[0].length;
          continue;
        }
      }
      cursor += 1;
    }
    interfaces.set(name, fields);
  }
  return interfaces;
}

// ── 比较工具 ──────────────────────────────────────────────────────────
const findings = [];

function compareExact(kind, label, expected, actual) {
  const expectedSet = new Set(expected);
  const actualSet = new Set(actual);
  const missing = expected.filter((item) => !actualSet.has(item));
  const extra = actual.filter((item) => !expectedSet.has(item));
  if (!missing.length && !extra.length) return;
  findings.push({ level: "error", kind, label, missing, extra });
}

/** 消费方独有 = 读一个后端永不发送的字段（运行时 undefined），属破坏性。
 *  `allow` 用于「双形状容错」这类**有意**的兼容字段（前端声明了 v1 契约名并做归一化回退）。 */
function compareConsumer(kind, label, produced, consumed, allow = []) {
  const producedSet = new Set(produced);
  const allowedSet = new Set(allow);
  const consumedSet = new Set(consumed);
  const consumedOnly = consumed.filter((item) => !producedSet.has(item) && !allowedSet.has(item));
  const producedOnly = produced.filter((item) => !consumedSet.has(item));
  if (consumedOnly.length) {
    findings.push({ level: "error", kind, label, missing: consumedOnly, extra: [] });
  }
  if (producedOnly.length) {
    findings.push({ level: "warn", kind, label, missing: [], extra: producedOnly });
  }
}

// ── 主流程 ────────────────────────────────────────────────────────────
const rustSrc = read(RUST);
const structs = parseRustStructs(rustSrc);
const enums = parseRustEnums(rustSrc);
const tsInterfaces = parseTsInterfaces(read(TS));
const viewSchema = JSON.parse(read(SCHEMA_VIEW));
const applySchema = JSON.parse(read(SCHEMA_APPLY));

// 0) Schema 自身的结构合法性（元 schema 校验）。
//    字段比对只能发现「内容不一致」，发现不了「这份 schema 本身不是合法 JSON Schema」。
//    ajv 缺失时降级为提示，不阻断离线环境。
try {
  const { default: Ajv2020 } = await import("ajv/dist/2020.js");
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  for (const [name, schema] of [
    ["recognition-decision-view-v1.schema.json", viewSchema],
    ["recognition-apply-decisions-v1.schema.json", applySchema]
  ]) {
    if (!ajv.validateSchema(schema)) {
      findings.push({
        level: "error",
        kind: "schema-invalid",
        label: name,
        missing: (ajv.errors ?? []).map((error) => `${error.instancePath || "/"} ${error.message}`),
        extra: []
      });
    }
  }
} catch (error) {
  findings.push({
    level: "warn",
    kind: "ajv-unavailable",
    label: "跳过 JSON Schema 元校验",
    missing: [],
    extra: [String(error?.message ?? error)]
  });
}

const schemaFields = (pointer) => {
  let node = pointer.schema === "view" ? viewSchema : applySchema;
  for (const segment of pointer.path) node = node[segment];
  return Object.keys(node.properties ?? {});
};

// 1) Rust ↔ JSON Schema：字段名（双向必须精确一致）
const FIELD_MAP = [
  { rust: "RecognitionDecisionViewV1", schema: { schema: "view", path: [] } },
  { rust: "DecisionItemV1", schema: { schema: "view", path: ["$defs", "decisionItem"] } },
  { rust: "DecisionTargetV1", schema: { schema: "view", path: ["$defs", "decisionItem", "properties", "target"] } },
  { rust: "DecisionEvidenceV1", schema: { schema: "view", path: ["$defs", "decisionItem", "properties", "evidence", "items"] } },
  { rust: "DecisionSummaryV1", schema: { schema: "view", path: ["$defs", "summary"] } },
  { rust: "StageStatusV1", schema: { schema: "view", path: ["$defs", "stageStatus"] } },
  { rust: "RecognitionChainStateV1", schema: { schema: "view", path: ["$defs", "chains"] } },
  { rust: "ApplyRecognitionDecisionsRequestV1", schema: { schema: "apply", path: ["$defs", "request"] } },
  { rust: "ApplyRecognitionDecisionsResultV1", schema: { schema: "apply", path: ["$defs", "result"] } },
  { rust: "DecisionOutcomeV1", schema: { schema: "apply", path: ["$defs", "result", "properties", "outcomes", "items"] } }
];

for (const entry of FIELD_MAP) {
  const rustFields = structs.get(entry.rust);
  if (!rustFields) {
    findings.push({ level: "error", kind: "rust-missing", label: entry.rust, missing: [], extra: [] });
    continue;
  }
  compareExact("schema-field", `${entry.rust} ↔ schema`, rustFields, schemaFields(entry.schema));
}

// 2) Rust ↔ JSON Schema：枚举值域（双向必须精确一致）
const ENUM_MAP = [
  { rust: "DecisionResolutionV1", schema: { schema: "view", path: ["$defs", "decisionItem", "properties", "resolution"] } },
  { rust: "DecisionStatusV1", schema: { schema: "view", path: ["$defs", "decisionItem", "properties", "status"] } },
  { rust: "DecisionSeverityV1", schema: { schema: "view", path: ["$defs", "decisionItem", "properties", "severity"] } },
  { rust: "DecisionFieldV1", schema: { schema: "view", path: ["$defs", "decisionItem", "properties", "field"] } },
  { rust: "DecisionTargetTypeV1", schema: { schema: "view", path: ["$defs", "decisionItem", "properties", "target", "properties", "targetType"] } },
  { rust: "ChainKindV1", schema: { schema: "view", path: ["$defs", "decisionItem", "properties", "evidence", "items", "properties", "chain"] } },
  { rust: "StageStateV1", schema: { schema: "view", path: ["$defs", "stageState"] } },
  { rust: "DecisionOutcomeKindV1", schema: { schema: "apply", path: ["$defs", "result", "properties", "outcomes", "items", "properties", "kind"] } }
];

const schemaEnum = (pointer) => {
  let node = pointer.schema === "view" ? viewSchema : applySchema;
  for (const segment of pointer.path) node = node[segment];
  return node.enum ?? [];
};

for (const entry of ENUM_MAP) {
  const variants = enums.get(entry.rust);
  if (!variants) {
    findings.push({ level: "error", kind: "rust-missing", label: entry.rust, missing: [], extra: [] });
    continue;
  }
  compareExact("schema-enum", `${entry.rust} ↔ schema`, variants, schemaEnum(entry.schema));
}

// 3) Rust ↔ 前端 TypeScript：字段名（前端多读是破坏，后端多给只是信息）
//
// 读取面用 `RecognitionDecisionRawV1`：前端有意做「双形状容错」——
// 优先读契约名，缺了回退到实现名（`editVersion` / `chains` / `actionable` 等），
// 因此那些 v1 契约名出现在 `allow` 里，不算漂移。这是已确认的设计选择（见交接单）。
// apply 的请求与返回目前**没有**归一化，故不加 allow —— 那正是尚未修复的写入路径。
const TS_MAP = [
  {
    rust: "RecognitionDecisionViewV1",
    ts: "RecognitionDecisionRawV1",
    allow: ["currentEditVersion", "localStatus", "cloudStatus", "cloudReasonCode", "items"]
  },
  { rust: "DecisionItemV1", ts: "RecognitionDecisionItemV1" },
  { rust: "DecisionTargetV1", ts: "RecognitionDecisionTargetV1" },
  { rust: "DecisionEvidenceV1", ts: "RecognitionEvidenceV1" },
  { rust: "DecisionSummaryV1", ts: "RecognitionDecisionSummaryV1" },
  { rust: "ApplyRecognitionDecisionsRequestV1", ts: "ApplyRecognitionDecisionsInputV1" },
  { rust: "ApplyRecognitionDecisionsResultV1", ts: "ApplyRecognitionDecisionsResultV1" }
];

for (const entry of TS_MAP) {
  const rustFields = structs.get(entry.rust);
  const tsFields = tsInterfaces.get(entry.ts);
  if (!rustFields) {
    findings.push({ level: "error", kind: "rust-missing", label: entry.rust, missing: [], extra: [] });
    continue;
  }
  if (!tsFields) {
    findings.push({ level: "error", kind: "ts-missing", label: entry.ts, missing: [], extra: [] });
    continue;
  }
  compareConsumer("ts-field", `${entry.rust} ↔ ${entry.ts}`, rustFields, tsFields, entry.allow ?? []);
}

// ── 报告 ──────────────────────────────────────────────────────────────
if (process.argv.includes("--json")) {
  console.log(JSON.stringify({ ok: findings.length === 0, findings }, null, 2));
} else {
  const errors = findings.filter((item) => item.level === "error");
  const warns = findings.filter((item) => item.level === "warn");
  for (const item of findings) {
    const mark = item.level === "error" ? "✗" : "!";
    console.log(`${mark} [${item.kind}] ${item.label}`);
    if (item.missing.length) console.log(`    消费方缺少/读不到： ${item.missing.join(", ")}`);
    if (item.extra.length) console.log(`    多余（未被消费或不存在于真源）： ${item.extra.join(", ")}`);
  }
  console.log("");
  console.log(`契约漂移检查：${errors.length} 处破坏性不一致，${warns.length} 处需要留意。`);
  console.log(`真源：${RUST.replace(ROOT, ".")}`);
}

process.exit(findings.some((item) => item.level === "error") ? 1 : 0);
