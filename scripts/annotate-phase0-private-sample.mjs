import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = path.join(repoRoot, "fixtures", "golden", "manifest.json");
const manifest = readJson(manifestPath);
const fixtureFilter = process.argv.slice(2).find((value) => value.startsWith("--fixture-id="))?.split("=")[1] ?? null;
const fixtures = (manifest.fixtures ?? []).filter((fixture) =>
  String(fixture.fixtureId).startsWith("private-random-") && (!fixtureFilter || fixture.fixtureId === fixtureFilter)
);

if (fixtures.length === 0) throw new Error("no private-random fixtures found in the manifest");

const updated = [];
for (const fixture of fixtures) {
  const baselinePath = resolveRepoPath(fixture.baselinePath);
  const metadataPath = resolveRepoPath(fixture.metadataPath);
  if (!fs.existsSync(baselinePath)) throw new Error(`V1 baseline missing: ${fixture.fixtureId}`);
  const baseline = readJson(baselinePath);
  const metadata = readJson(metadataPath);
  const payload = baseline.payload ?? {};
  const observed = baseline.observed ?? {};
  const expected = buildExpected(payload);
  const knownIssues = new Set(metadata.knownIssues ?? []);
  knownIssues.delete("等待 V1 基线捕获后从实际输出初始化页面、题组、答案位和资源标注。");
  knownIssues.add("样本使用固定随机种子抽取；V1 派生标注是可复现的初始契约，正式评分前仍需人工确认。");
  if ((observed.warningCount ?? 0) > 0) {
    knownIssues.add(`V1 parser emitted ${observed.warningCount} warning(s); warning details remain part of the baseline.`);
  }
  if (expected.taskGroups.length === 0 || expected.slots.length === 0) {
    knownIssues.add("V1 未识别出完整题组或答案位；该样本必须进入人工复核队列。");
  }

  metadata.expected = expected;
  metadata.knownIssues = [...knownIssues];
  metadata.baseline = {
    ...(metadata.baseline ?? {}),
    v1Path: fixture.baselinePath,
    observed
  };
  writeJson(metadataPath, metadata);
  updated.push({ fixtureId: fixture.fixtureId, pageCount: expected.pageRoles.length, groupCount: expected.taskGroups.length, slotCount: expected.slots.length, assetCount: expected.assets.length });
}

console.log(JSON.stringify({ schemaVersion: "Phase0PrivateAnnotationV1", updated }, null, 2));

function buildExpected(payload) {
  const documentIr = payload.documentIr ?? {};
  const authoringIr = payload.authoringIr ?? {};
  const pages = Array.isArray(documentIr.pages) ? documentIr.pages : [];
  const pageRoles = pages.map((page, index) => {
    const roles = unique((page.blocks ?? []).map((block) => block.roleHint).filter(Boolean));
    return {
      pageIndex: Number(page.pageIndex ?? index + 1),
      roles: roles.length > 0 ? roles : ["needs_review"]
    };
  });

  const groups = Array.isArray(authoringIr.groups) ? authoringIr.groups : [];
  const taskGroups = [];
  const slots = [];
  for (let groupIndex = 0; groupIndex < groups.length; groupIndex += 1) {
    const group = groups[groupIndex] ?? {};
    const questions = Array.isArray(group.questions) ? group.questions : [];
    const groupSlots = [];
    const numbers = [];
    for (let questionIndex = 0; questionIndex < questions.length; questionIndex += 1) {
      const question = questions[questionIndex] ?? {};
      const id = String(question.id ?? question.questionId ?? `q${questionIndex + 1}`);
      const displayNumber = String(question.displayNumber ?? question.number ?? extractNumber(id) ?? questionIndex + 1);
      const number = Number.parseInt(displayNumber, 10);
      if (Number.isInteger(number)) numbers.push(number);
      const responseType = String(question.responseType ?? question.inputType ?? question.answerType ?? inferResponseType(group.kind));
      slots.push({ id, displayNumber, responseType });
      groupSlots.push(id);
    }
    if (groupSlots.length === 0) continue;
    const declaredRange = group.displayRange ?? group.questionRange ?? group.range;
    const displayRange = Array.isArray(declaredRange) && declaredRange.length === 2
      ? declaredRange.map((value) => Number.parseInt(value, 10))
      : [Math.min(...numbers), Math.max(...numbers)];
    taskGroups.push({
      id: String(group.id ?? group.groupId ?? `group-${groupIndex + 1}`),
      displayRange,
      kind: String(group.kind ?? group.type ?? "unclassified"),
      slotIds: groupSlots
    });
  }

  const assets = (Array.isArray(documentIr.assets) ? documentIr.assets : []).map((asset, index) => ({
    id: String(asset.id ?? asset.assetId ?? `asset-${index + 1}`),
    type: String(asset.type ?? asset.kind ?? asset.assetType ?? "asset"),
    required: true
  }));
  return { pageRoles, taskGroups, slots: dedupeSlots(slots), assets };
}

function inferResponseType(kind) {
  const normalized = String(kind ?? "").toLowerCase();
  if (normalized.includes("matching")) return "select";
  if (normalized.includes("choice") || normalized.includes("true_false") || normalized.includes("yes_no")) return "radio";
  return "text";
}

function extractNumber(value) {
  const match = String(value).match(/\d+/);
  return match ? match[0] : null;
}

function dedupeSlots(slots) {
  const seen = new Set();
  return slots.filter((slot) => !seen.has(slot.id) && seen.add(slot.id));
}

function unique(values) {
  return [...new Set(values.map((value) => String(value)))];
}

function resolveRepoPath(relativePath) {
  return path.isAbsolute(relativePath) ? relativePath : path.resolve(repoRoot, relativePath);
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function writeJson(filePath, value) {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}
