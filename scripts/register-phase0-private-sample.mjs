import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = path.join(repoRoot, "fixtures", "golden", "manifest.json");
const privateRoot = path.join(repoRoot, "fixtures", "golden", "private-real");
const defaultSourceRoot = "C:/Users/lenovo/Desktop/working space/0.3.1 working/ReadingPractice/PDF";
const args = parseArgs(process.argv.slice(2));
const sourceRoot = path.resolve(args["source-root"] ?? defaultSourceRoot);
const seed = Number(args.seed ?? 20260809);
const sampleCount = Number(args.count ?? 8);

if (!Number.isInteger(seed) || !Number.isInteger(sampleCount) || sampleCount <= 0) {
  throw new Error("--seed must be an integer and --count must be a positive integer");
}
if (!fs.existsSync(sourceRoot) || !fs.statSync(sourceRoot).isDirectory()) {
  throw new Error(`source directory does not exist: ${sourceRoot}`);
}

const candidates = fs.readdirSync(sourceRoot, { withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.toLowerCase().endsWith(".pdf"))
  .map((entry) => entry.name)
  .sort(compareCodePoint);
if (candidates.length < sampleCount) {
  throw new Error(`source directory has only ${candidates.length} PDFs; need ${sampleCount}`);
}

const shuffled = [...candidates];
const random = mulberry32(seed >>> 0);
for (let index = shuffled.length - 1; index > 0; index -= 1) {
  const swapIndex = Math.floor(random() * (index + 1));
  [shuffled[index], shuffled[swapIndex]] = [shuffled[swapIndex], shuffled[index]];
}

const selected = shuffled.slice(0, sampleCount);
fs.mkdirSync(privateRoot, { recursive: true });
const manifest = readJson(manifestPath);
const legacyReferenceFixtureIds = (manifest.legacyReferenceFixtureIds ?? [])
  .length > 0
  ? manifest.legacyReferenceFixtureIds
  : readJson(path.join(repoRoot, manifest.legacyReferencePath)).references.map((entry) => entry.fixtureId);
const existingFixtures = (manifest.fixtures ?? [])
  .filter((fixture) => !String(fixture.fixtureId).startsWith("private-random-"));
const selectedFixtures = [];
const selection = [];

for (let index = 0; index < selected.length; index += 1) {
  const originalName = selected[index];
  const fixtureId = `private-random-${String(index + 1).padStart(2, "0")}`;
  const destinationName = `${fixtureId}.pdf`;
  const destinationPath = path.join(privateRoot, destinationName);
  const sourcePath = path.join(sourceRoot, originalName);
  fs.copyFileSync(sourcePath, destinationPath);
  const stat = fs.statSync(destinationPath);
  const hash = sha256File(destinationPath);
  const relativeSourcePath = toRepoPath(destinationPath);
  const metadataPath = `fixtures/golden/metadata/${fixtureId}.json`;
  const baselinePath = `fixtures/golden/baseline/v1/${fixtureId}.json`;

  selectedFixtures.push({
    fixtureId,
    status: "available",
    sourcePath: relativeSourcePath,
    originalName,
    selectionSeed: seed,
    sampleRank: index + 1,
    sha256: hash,
    sizeBytes: stat.size,
    metadataPath,
    baselinePath
  });
  selection.push({ fixtureId, sampleRank: index + 1, originalName, sourcePath: relativeSourcePath, sha256: hash, sizeBytes: stat.size });

  const metadataFilePath = path.join(repoRoot, metadataPath);
  const existingMetadata = fs.existsSync(metadataFilePath) ? readJson(metadataFilePath) : null;
  const metadata = existingMetadata?.source?.sha256 === hash
    ? {
      ...existingMetadata,
      fixtureId,
      source: {
        ...(existingMetadata.source ?? {}),
        path: relativeSourcePath,
        originalName,
        sha256: hash,
        sizeBytes: stat.size,
        format: "pdf"
      },
      baseline: {
        ...(existingMetadata.baseline ?? {}),
        v1Path: baselinePath
      }
    }
    : {
    schemaVersion: "GoldenFixtureMetadataV1",
    fixtureId,
    source: {
      path: relativeSourcePath,
      originalName,
      sha256: hash,
      sizeBytes: stat.size,
      format: "pdf"
    },
    expected: { pageRoles: [], taskGroups: [], slots: [], assets: [] },
    knownIssues: ["等待 V1 基线捕获后从实际输出初始化页面、题组、答案位和资源标注。"],
    baseline: { v1Path: baselinePath, observed: {} }
    };
  writeJson(metadataFilePath, metadata);
}

manifest.fixtures = [...existingFixtures, ...selectedFixtures]
  .sort((left, right) => left.fixtureId.localeCompare(right.fixtureId));
manifest.requiredPrivateCorpus = selectedFixtures.map(({ fixtureId, status, sourcePath, originalName, selectionSeed, sampleRank }) => ({
  fixtureId,
  status,
  sourcePath,
  originalName,
  selectionSeed,
  sampleRank
}));
manifest.legacyReferenceFixtureIds = legacyReferenceFixtureIds;
manifest.privateSourceSelection = {
  method: "seeded-fisher-yates",
  sourceDirectory: sourceRoot,
  populationCount: candidates.length,
  sampleCount,
  seed,
  selected
};
writeJson(manifestPath, manifest);

console.log(JSON.stringify({
  schemaVersion: "Phase0PrivateSampleRegistrationV1",
  sourceDirectory: sourceRoot,
  populationCount: candidates.length,
  sampleCount,
  seed,
  selected: selection
}, null, 2));

function sha256File(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function mulberry32(value) {
  return () => {
    let state = value += 0x6D2B79F5;
    state = Math.imul(state ^ state >>> 15, state | 1);
    state ^= state + Math.imul(state ^ state >>> 7, state | 61);
    return ((state ^ state >>> 14) >>> 0) / 4294967296;
  };
}

function compareCodePoint(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith("--")) continue;
    const key = token.slice(2);
    parsed[key] = argv[index + 1] && !argv[index + 1].startsWith("--") ? argv[++index] : true;
  }
  return parsed;
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function writeJson(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function toRepoPath(filePath) {
  return path.relative(repoRoot, filePath).replaceAll("\\", "/");
}
