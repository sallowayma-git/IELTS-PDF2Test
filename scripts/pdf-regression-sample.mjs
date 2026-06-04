import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultCorpusRoot = "/Users/maziheng/Downloads/0.3.1 working";
const defaultPdfDir = path.join(defaultCorpusRoot, "ReadingPractice", "PDF");
const defaultLegacyDir = path.join(defaultCorpusRoot, "assets", "generated", "reading-exams");
const defaultOutDir = path.join(repoRoot, "tmp", "pdf-regression-sample");

const args = parseArgs(process.argv.slice(2));
const sampleSize = Number(args.sample ?? 30);
const pdfDir = path.resolve(args.pdfDir ?? defaultPdfDir);
const legacyDir = path.resolve(args.legacyDir ?? defaultLegacyDir);
const outDir = path.resolve(args.outDir ?? defaultOutDir);
const seed = args.seed ?? String(Date.now());
const strict = args.strict === "true" || args.strict === true;
const skipBuild = args.skipBuild === "true" || args.skipBuild === true;

fs.mkdirSync(outDir, { recursive: true });

const legacyItems = loadLegacyReadingExams(legacyDir);
const legacyByPdf = new Map();
for (const item of legacyItems) {
  const key = normalizeFileKey(item.data?.meta?.pdfFilename);
  if (key) legacyByPdf.set(key, item);
}

const allPdfs = fs.readdirSync(pdfDir)
  .filter((name) => name.toLowerCase().endsWith(".pdf"))
  .map((name) => path.join(pdfDir, name))
  .sort((a, b) => a.localeCompare(b));

const matched = [];
const unmatched = [];
for (const pdfPath of allPdfs) {
  const legacy = legacyByPdf.get(normalizeFileKey(path.basename(pdfPath)));
  if (legacy) matched.push({ pdfPath, legacy });
  else unmatched.push(pdfPath);
}

const selected = shuffle(matched, seed).slice(0, Math.min(sampleSize, matched.length));
const cli = path.join(repoRoot, "src-tauri", "target", "debug", "ielts-author-studio");
if (!skipBuild || !fs.existsSync(cli)) {
  const build = spawnSync("cargo", ["build", "--manifest-path", path.join(repoRoot, "src-tauri", "Cargo.toml")], {
    cwd: repoRoot,
    stdio: "inherit"
  });
  if (build.status !== 0) process.exit(build.status ?? 1);
}

const results = [];
for (const [index, item] of selected.entries()) {
  const outputPath = path.join(outDir, `${String(index + 1).padStart(2, "0")}-${safeName(path.basename(item.pdfPath))}.json`);
  const generated = spawnSync(cli, ["--generate-reading-source", item.pdfPath, "--out", outputPath], {
    cwd: repoRoot,
    encoding: "utf8"
  });
  if (generated.status !== 0) {
    results.push({
      pdf: item.pdfPath,
      legacyId: item.legacy.id,
      ok: false,
      error: generated.stderr.trim() || generated.stdout.trim() || `exit_${generated.status}`
    });
    continue;
  }
  const payload = JSON.parse(fs.readFileSync(outputPath, "utf8"));
  const actual = normalizeGenerated(payload.readingSource);
  const expected = normalizeLegacy(item.legacy.data);
  const comparison = compareNormalized(actual, expected);
  results.push({
    pdf: item.pdfPath,
    legacyId: item.legacy.id,
    ok: comparison.ok,
    structureOk: comparison.structureOk,
    answersOk: comparison.answersOk,
    limitation: classifyLimitation(payload, comparison),
    comparison,
    generatedPath: outputPath
  });
}

const failures = results.filter((result) => !result.ok);
const structureFailures = results.filter((result) => result.structureOk === false);
const answerFailures = results.filter((result) => result.answersOk === false);
const parserLimitations = results.filter((result) => result.limitation);
const report = {
  schemaVersion: "PdfRegressionSampleReportV1",
  generatedAt: new Date().toISOString(),
  seed,
  sampleSize,
  pdfDir,
  legacyDir,
  matchedCount: matched.length,
  unmatchedCount: unmatched.length,
  selected: selected.map((item) => ({ pdf: item.pdfPath, legacyId: item.legacy.id })),
  passCount: results.length - failures.length,
  failCount: failures.length,
  structurePassCount: results.length - structureFailures.length,
  structureFailCount: structureFailures.length,
  answerPassCount: results.length - answerFailures.length,
  answerFailCount: answerFailures.length,
  parserLimitationCount: parserLimitations.length,
  results
};
const reportPath = path.join(outDir, "report.json");
fs.writeFileSync(reportPath, JSON.stringify(report, null, 2));
console.log(JSON.stringify({
  seed,
  sampled: selected.length,
  matchedCount: matched.length,
  unmatchedCount: unmatched.length,
  passCount: report.passCount,
  failCount: report.failCount,
  structurePassCount: report.structurePassCount,
  structureFailCount: report.structureFailCount,
  answerPassCount: report.answerPassCount,
  answerFailCount: report.answerFailCount,
  parserLimitationCount: report.parserLimitationCount,
  reportPath
}, null, 2));

if (strict && failures.length) process.exit(1);

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (!arg.startsWith("--")) continue;
    const key = arg.slice(2);
    const next = argv[index + 1];
    if (!next || next.startsWith("--")) parsed[key] = true;
    else {
      parsed[key] = next;
      index += 1;
    }
  }
  return parsed;
}

function loadLegacyReadingExams(dir) {
  const registry = {
    items: {},
    register(id, data) {
      this.items[id] = data;
    }
  };
  globalThis.__READING_EXAM_DATA__ = registry;
  for (const file of fs.readdirSync(dir).filter((name) => name.endsWith(".js") && name !== "manifest.js")) {
    const content = fs.readFileSync(path.join(dir, file), "utf8");
    try {
      new Function("window", "globalThis", content)(globalThis, globalThis);
    } catch (error) {
      console.warn(`[pdf-regression] skipped legacy file ${file}: ${error.message}`);
    }
  }
  return Object.entries(registry.items).map(([id, data]) => ({ id, data }));
}

function normalizeFileKey(value) {
  if (!value) return "";
  return path.basename(String(value))
    .normalize("NFKC")
    .replace(/【[^】]*】/g, "")
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
}

function safeName(value) {
  return value.replace(/[^a-z0-9._-]+/gi, "-").replace(/^-+|-+$/g, "").slice(0, 96);
}

function seededRandom(seedValue) {
  let hash = 2166136261;
  for (const ch of String(seedValue)) {
    hash ^= ch.charCodeAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return () => {
    hash += 0x6d2b79f5;
    let value = hash;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

function shuffle(items, seedValue) {
  const random = seededRandom(seedValue);
  const copy = items.slice();
  for (let index = copy.length - 1; index > 0; index -= 1) {
    const swap = Math.floor(random() * (index + 1));
    [copy[index], copy[swap]] = [copy[swap], copy[index]];
  }
  return copy;
}

function normalizeQuestionId(value) {
  if (value == null) return "";
  const raw = String(value).trim();
  const qMatch = raw.match(/q(\d{1,4})/i);
  if (qMatch) return `q${Number(qMatch[1])}`;
  const digitMatch = raw.match(/\d{1,4}/);
  return digitMatch ? `q${Number(digitMatch[0])}` : raw.toLowerCase();
}

function normalizeAnswerValue(value) {
  const normalizeOne = (entry) => {
    const text = String(entry ?? "")
      .replace(/[“”]/g, "\"")
      .replace(/[‘’]/g, "'")
      .replace(/[‐‑‒–—]/g, "-")
      .replace(/\s+/g, " ")
      .trim()
      .replace(/^[\s"'`()[\]{}<>.,;:!?]+|[\s"'`()[\]{}<>.,;:!?]+$/g, "");
    if (!text) return "";
    const lower = text.toLowerCase();
    if (["true", "t", "yes", "y", "正确", "是"].includes(lower)) return "TRUE";
    if (["false", "f", "no", "n", "错误", "否"].includes(lower)) return "FALSE";
    if (["ng", "notgiven", "not-given", "not given"].includes(lower)) return "NOT GIVEN";
    if (/^[a-z]$/i.test(text)) return text.toUpperCase();
    return text;
  };
  if (Array.isArray(value)) return Array.from(new Set(value.map(normalizeOne).filter(Boolean)));
  return normalizeOne(value);
}

function canonicalKind(kind, bodyHtml = "") {
  const raw = String(kind ?? "").toLowerCase().replace(/[\s-]+/g, "_");
  const body = String(bodyHtml ?? "").toLowerCase();
  const semantic = semanticKindFromBody(body);
  if (semantic) return semantic;
  if (raw.includes("heading")) return "heading_matching";
  if (raw.includes("matching_information")) return "matching_information";
  if (raw.includes("matching") || raw.includes("classification")) return "matching";
  if (raw.includes("true_false") || body.includes("not given") && body.includes("true") && body.includes("false")) return "true_false_not_given";
  if (raw.includes("yes_no") || body.includes("not given") && body.includes("yes") && body.includes("no")) return "yes_no_not_given";
  if (raw.includes("multi")) return "multi_choice";
  if (raw.includes("single")) return "single_choice";
  if (raw.includes("table") || body.includes("<table")) return "table_completion";
  if (raw.includes("diagram") || raw.includes("flow")) return "sentence_completion";
  if (raw.includes("summary") || raw.includes("sentence") || body.includes("notes-completion") || body.includes("summary-text") || body.includes("summary-completion")) return "sentence_completion";
  if (body.includes("match-dropzone") || body.includes("match-question") || body.includes("matching-container")) return "matching";
  if (body.includes('type="radio"')) return "single_choice";
  if (raw.includes("short") || body.includes('type="text"')) return "short_answer";
  return raw || "unknown";
}

function semanticKindFromBody(body) {
  const text = stripHtml(body).toLowerCase().replace(/[‐‑‒–—]/g, "-").replace(/\s+/g, " ");
  if (text.includes("true") && text.includes("false") && text.includes("not given")) return "true_false_not_given";
  if (text.includes("yes") && text.includes("no") && text.includes("not given")) return "yes_no_not_given";
  if (text.includes("list of headings") || text.includes("correct heading for each")) return "heading_matching";
  if (text.includes("which paragraph contains") || text.includes("which section contains") || text.includes("which paragraph has")) return "matching_information";
  if (text.includes("match each statement") || text.includes("match each person") || text.includes("match each opinion") || text.includes("look at the following")) return "matching";
  if (text.includes("complete each sentence") && (text.includes("correct ending") || text.includes("list of endings"))) return "matching";
  if (text.includes("complete the table") || text.includes("table below") || body.includes("completion-table")) return "table_completion";
  if (text.includes("answer the questions below") || isShortAnswerQuestionText(text)) return "short_answer";
  if (text.includes("complete the summary") || text.includes("complete the notes") || text.includes("complete the sentences") || text.includes("complete the flow-chart") || body.includes("flowchart")) return "sentence_completion";
  if (text.includes("choose two letters") || text.includes("choose three letters")) return "multi_choice";
  if (text.includes("choose the correct letter") || text.includes("which of the following")) return "single_choice";
  return null;
}

function isShortAnswerQuestionText(text) {
  if (text.includes("complete the")) return false;
  const hasWordLimit = text.includes("choose no more than") || text.includes("write no more than");
  if (!hasWordLimit) return false;
  const questionMarks = (text.match(/\?/g) ?? []).length;
  const numberedQuestion = /\b\d{1,3}\s+(?:what|which|who|when|where|why|how|according to)\b/.test(text);
  return questionMarks >= 2 || numberedQuestion;
}

function stripHtml(html) {
  return String(html ?? "").replace(/<script[\s\S]*?<\/script>/gi, " ").replace(/<style[\s\S]*?<\/style>/gi, " ").replace(/<[^>]+>/g, " ");
}

function extractQuestionIdsFromHtml(html) {
  const ids = new Set();
  const pattern = /\b(?:name|data-question-id|data-question|id)=["']([^"']+)["']/gi;
  for (const match of String(html ?? "").matchAll(pattern)) {
    const qid = normalizeQuestionId(match[1]);
    if (/^q\d+$/.test(qid)) ids.add(qid);
  }
  return [...ids].sort((left, right) => Number(left.slice(1)) - Number(right.slice(1)));
}

function detectGroupQuestionRange(bodyHtml = "") {
  const text = stripHtml(bodyHtml).replace(/[‐‑‒–—]/g, "-").replace(/\s+/g, " ");
  const dash = text.match(/Questions?\s+(\d{1,3})\s*-\s*(\d{1,3})/i);
  if (dash) return [Number(dash[1]), Number(dash[2])];
  const paired = text.match(/Questions?\s+(\d{1,3})\s+and\s+(\d{1,3})/i);
  if (paired) return [Number(paired[1]), Number(paired[2])];
  const single = text.match(/Questions?\s+(\d{1,3})\b/i);
  if (single) return [Number(single[1]), Number(single[1])];
  return null;
}

function expandMultiChoiceIds(ids, bodyHtml) {
  const range = detectGroupQuestionRange(bodyHtml);
  if (!range || ids.length !== 1 || range[1] <= range[0]) return ids;
  const count = range[1] - range[0] + 1;
  const first = Number(String(ids[0]).replace(/^q/i, ""));
  if (!Number.isFinite(first) || count > 6) return ids;
  return Array.from({ length: count }, (_, index) => `q${first + index}`);
}

function normalizeGroups(source) {
  return (source?.questionGroups ?? []).map((group) => {
    const bodyHtml = String(group.bodyHtml ?? "");
    if (!bodyHtml.trim()) return null;
    const ids = (group.questionIds?.length ? group.questionIds : extractQuestionIdsFromHtml(group.bodyHtml))
      .map(normalizeQuestionId)
      .filter(Boolean);
    const kind = canonicalKind(group.kind, group.bodyHtml);
    const questionIds = normalizeGroupQuestionIds(kind === "multi_choice" ? expandMultiChoiceIds(ids, bodyHtml) : ids, bodyHtml);
    return {
      kind,
      questionIds,
      range: questionIds.length ? [Number(questionIds[0].slice(1)), Number(questionIds.at(-1).slice(1))] : [],
      layout: canonicalLayout(kind, group.bodyHtml)
    };
  }).filter(Boolean);
}

function normalizeGroupQuestionIds(ids, bodyHtml) {
  const range = detectGroupQuestionRange(bodyHtml);
  if (!range || !ids.length) return ids;
  const expectedCount = range[1] - range[0] + 1;
  if (expectedCount <= 0 || expectedCount > 20) return ids;
  const numbers = ids.map((qid) => Number(String(qid).replace(/^q/i, ""))).filter(Number.isFinite);
  const contiguous = numbers.length === expectedCount
    && numbers.every((value, index) => index === 0 || value === numbers[index - 1] + 1);
  if (contiguous) return ids;
  const first = numbers[0];
  if (!Number.isFinite(first)) return ids;
  return Array.from({ length: expectedCount }, (_, index) => `q${first + index}`);
}

function canonicalLayout(kind, bodyHtml = "") {
  const body = String(bodyHtml ?? "").toLowerCase();
  const text = stripHtml(body).toLowerCase().replace(/\s+/g, " ");
  if (kind === "table_completion") return "table";
  if (kind === "sentence_completion" && !text.includes("answer the questions below")) return "inline_completion";
  return "list";
}

function normalizeAnswerMap(answerKey) {
  return Object.fromEntries(Object.entries(answerKey ?? {})
    .map(([key, value]) => [normalizeQuestionId(key), normalizeAnswerValue(value)])
    .filter(([key]) => key));
}

function normalizeLegacy(source) {
  const groups = normalizeGroups(source);
  const baseOrder = (source?.questionOrder?.length ? source.questionOrder : Object.keys(source?.answerKey ?? {}))
    .map(normalizeQuestionId);
  const questionOrder = uniqueStable([...baseOrder, ...groups.flatMap((group) => group.questionIds)]);
  const answerKey = expandGroupedMultiChoiceAnswers(normalizeAnswerMap(source?.answerKey), groups);
  return rebaseQuestionNumbers({
    title: source?.meta?.title ?? "",
    groups,
    questionOrder,
    answerKey
  });
}

function normalizeGenerated(source) {
  return normalizeLegacy(source);
}

function uniqueStable(values) {
  const seen = new Set();
  return values.filter((value) => {
    if (!value || seen.has(value)) return false;
    seen.add(value);
    return true;
  });
}

function expandGroupedMultiChoiceAnswers(answerKey, groups) {
  const expanded = { ...answerKey };
  for (const group of groups) {
    if (group.kind !== "multi_choice" || group.questionIds.length <= 1) continue;
    const first = group.questionIds[0];
    const value = expanded[first];
    if (!Array.isArray(value) || value.length !== group.questionIds.length) continue;
    group.questionIds.forEach((qid, index) => {
      expanded[qid] = value[index];
    });
  }
  return expanded;
}

function rebaseQuestionNumbers(source) {
  const nums = [
    ...source.questionOrder,
    ...Object.keys(source.answerKey),
    ...source.groups.flatMap((group) => group.questionIds)
  ]
    .map((qid) => Number(String(qid).replace(/^q/i, "")))
    .filter(Number.isFinite);
  const base = nums.length ? Math.min(...nums) - 1 : 0;
  const rebase = (qid) => {
    const number = Number(String(qid).replace(/^q/i, ""));
    return Number.isFinite(number) ? `q${number - base}` : qid;
  };
  const groups = source.groups.map((group) => {
    const questionIds = group.questionIds.map(rebase);
    return {
      ...group,
      questionIds,
      range: questionIds.length ? [Number(questionIds[0].slice(1)), Number(questionIds.at(-1).slice(1))] : []
    };
  });
  return {
    ...source,
    questionOrder: source.questionOrder.map(rebase),
    answerKey: Object.fromEntries(Object.entries(source.answerKey).map(([qid, value]) => [rebase(qid), value])),
    groups
  };
}

function compareNormalized(actual, expected) {
  const expectedQuestionSet = new Set(expected.questionOrder);
  const actualQuestionSet = new Set(actual.questionOrder);
  const missingQuestions = [...expectedQuestionSet].filter((qid) => !actualQuestionSet.has(qid));
  const extraQuestions = [...actualQuestionSet].filter((qid) => !expectedQuestionSet.has(qid));
  const expectedAnswers = Object.keys(expected.answerKey).sort(questionSort);
  const actualAnswers = Object.keys(actual.answerKey).sort(questionSort);
  const missingAnswers = expectedAnswers.filter((qid) => !actual.answerKey[qid]);
  const groupShapeIssues = [];
  for (const expectedGroup of expected.groups) {
    const actualGroup = actual.groups.find((group) => sameRange(group.range, expectedGroup.range));
    if (!actualGroup) {
      groupShapeIssues.push({ range: expectedGroup.range, issue: "missing_group" });
      continue;
    }
    if (actualGroup.layout !== expectedGroup.layout) {
      groupShapeIssues.push({ range: expectedGroup.range, issue: "layout_mismatch", expected: expectedGroup.layout, actual: actualGroup.layout });
    }
    if (canonicalKind(actualGroup.kind) !== canonicalKind(expectedGroup.kind)) {
      groupShapeIssues.push({ range: expectedGroup.range, issue: "kind_mismatch", expected: expectedGroup.kind, actual: actualGroup.kind });
    }
    if (expectedGroup.layout === "inline_completion" && expectedGroup.questionIds.length > 1) {
      const actualIds = actualGroup.questionIds.join(",");
      const expectedIds = expectedGroup.questionIds.join(",");
      if (actualIds !== expectedIds) {
        groupShapeIssues.push({ range: expectedGroup.range, issue: "inline_question_ids_mismatch", expected: expectedIds, actual: actualIds });
      }
    }
  }
  const structureOk = missingQuestions.length === 0
    && extraQuestions.length === 0
    && groupShapeIssues.length === 0;
  const answersOk = missingAnswers.length === 0 && actualAnswers.length >= Math.min(1, expectedAnswers.length);
  return {
    ok: structureOk && answersOk,
    structureOk,
    answersOk,
    missingQuestions,
    extraQuestions,
    missingAnswers,
    expectedGroupCount: expected.groups.length,
    actualGroupCount: actual.groups.length,
    groupShapeIssues
  };
}

function classifyLimitation(payload, comparison) {
  if (comparison.answersOk) return null;
  const blocks = payload?.documentIr?.pages?.flatMap((page) => page.blocks ?? []) ?? [];
  const noExtractableAnswerPages = blocks.some((block) => {
    const text = String(block.text ?? "");
    return /^\[No extractable text on page \d+\]/.test(text) && (block.roleHint === "answer" || block.confidence < 0.5);
  });
  const parserWarnings = payload?.documentIr?.parser?.warnings ?? [];
  const noExtractableWarnings = Array.isArray(parserWarnings) && parserWarnings.some((warning) => String(warning).includes("No extractable text"));
  if (noExtractableAnswerPages || noExtractableWarnings) return "answer_pages_have_no_extractable_text";
  return null;
}

function questionSort(left, right) {
  return Number(left.replace(/^q/, "")) - Number(right.replace(/^q/, ""));
}

function sameRange(left, right) {
  return Array.isArray(left) && Array.isArray(right) && left[0] === right[0] && left[1] === right[1];
}
