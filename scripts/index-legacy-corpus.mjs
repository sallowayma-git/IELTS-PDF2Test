import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const legacyRoot = path.resolve(process.argv[2] ?? path.join(repoRoot, "..", "reading-exams"));
const outputPath = path.join(repoRoot, "fixtures", "golden", "private", "legacy-reference.json");

const targets = [
  ["fishbourne-roman-palace", "Fishbourne Roman Palace"],
  ["listening-to-the-ocean", "Listening to the Ocean"],
  ["chili-peppers", "Chili peppers"],
  ["petri-dish", "Petri dish"],
  ["organisational-design", "Organisational Design"],
  ["western-celebrity", "western celebrity"],
  ["conformity", "Conformity"],
  ["sleep-study", "Sleep Study"]
];

if (!fs.existsSync(legacyRoot) || !fs.statSync(legacyRoot).isDirectory()) {
  throw new Error(`legacy corpus directory not found: ${legacyRoot}`);
}

const registry = {
  items: {},
  register(id, data) {
    this.items[id] = data;
  }
};
globalThis.__READING_EXAM_DATA__ = registry;

for (const fileName of fs.readdirSync(legacyRoot).filter((name) => name.endsWith(".js") && name !== "manifest.js" && name !== "manifest.legacy.js")) {
  const filePath = path.join(legacyRoot, fileName);
  try {
    const source = fs.readFileSync(filePath, "utf8");
    new Function(source)();
  } catch {
    // A legacy file that is not a standalone exam is irrelevant to this index.
  }
}

const references = [];
const missing = [];
for (const [fixtureId, titleNeedle] of targets) {
  const matches = Object.entries(registry.items)
    .filter(([, data]) => String(data?.meta?.title ?? "").toLowerCase().includes(titleNeedle.toLowerCase()))
    .sort(([left], [right]) => left.localeCompare(right));
  if (!matches.length) {
    missing.push(fixtureId);
    continue;
  }
  const [examId, data] = matches[0];
  const jsFile = `${examId}.js`;
  const jsPath = path.join(legacyRoot, jsFile);
  const groups = (data.questionGroups ?? []).map((group) => ({
    groupId: group.groupId ?? null,
    kind: group.kind ?? null,
    questionIds: group.questionIds ?? [],
    questionNumbers: questionNumbers(group.questionIds ?? []),
    questionRange: questionRange(group.questionIds ?? [])
  }));
  const sourceFiles = data.sourceFiles ?? [];
  references.push({
    fixtureId,
    status: "reference-only",
    sourceType: "legacy-reading-exam-js",
    examId,
    title: data.meta?.title ?? null,
    category: data.meta?.category ?? null,
    pdfFilename: data.meta?.pdfFilename ?? null,
    legacyJsPath: toRepoRelative(jsPath),
    legacyJsSha256: sha256(jsPath),
    legacyJsSizeBytes: fs.statSync(jsPath).size,
    questionGroups: groups,
    questionCount: new Set(groups.flatMap((group) => group.questionIds)).size,
    answerCount: Object.keys(data.answerKey ?? {}).length,
    assetCount: Array.isArray(data.assets) ? data.assets.length : sourceFiles.filter((file) => file?.role === "Asset").length
  });
}

const result = {
  schemaVersion: "LegacyCorpusReferenceIndexV1",
  status: missing.length ? "incomplete" : "reference-only",
  sourceRoot: toRepoRelative(legacyRoot),
  note: "This index is evidence from the legacy JS corpus only; it is not a substitute for the required source PDFs.",
  referenceCount: references.length,
  missing,
  references
};
fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`, "utf8");
console.log(JSON.stringify({ outputPath: toRepoRelative(outputPath), referenceCount: references.length, missing }, null, 2));
if (missing.length) process.exitCode = 1;

function questionRange(ids) {
  const numbers = questionNumbers(ids);
  const contiguous = numbers.every((number, index) => index === 0 || number === numbers[index - 1] + 1);
  return numbers.length && contiguous ? [numbers[0], numbers.at(-1)] : null;
}

function questionNumbers(ids) {
  return ids.map((id) => Number(String(id).replace(/^q/i, ""))).filter(Number.isFinite).sort((left, right) => left - right);
}

function sha256(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function toRepoRelative(filePath) {
  const relative = path.relative(repoRoot, filePath);
  return relative.replaceAll("\\", "/") || ".";
}
