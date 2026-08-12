import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { cp, mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const nasRoot = resolve(repoRoot, "..", "NAS");
const examId = "phase7-listening-package";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function hash(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function wavFixture() {
  const sampleRate = 16000;
  const samples = sampleRate;
  const data = Buffer.alloc(samples * 2);
  for (let index = 0; index < samples; index += 1) {
    const value = Math.round(Math.sin((index / sampleRate) * Math.PI * 2 * 440) * 0.25 * 32767);
    data.writeInt16LE(value, index * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0, "ascii");
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVE", 8, "ascii");
  header.write("fmt ", 12, "ascii");
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(sampleRate, 24);
  header.writeUInt32LE(sampleRate * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36, "ascii");
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}

function run(command, args, cwd) {
  const executable = process.platform === "win32" && command === "npm" ? "npm.cmd" : command;
  const result = spawnSync(executable, args, { cwd, stdio: "inherit", shell: process.platform === "win32" });
  assert(result.status === 0, `${command} ${args.join(" ")} failed with ${result.status}`);
}

const source = JSON.parse(readFileSync(join(repoRoot, "fixtures/golden/synthetic/ielts/phase7-listening-part1-source-v1.json"), "utf8"));
const audio = wavFixture();
const audioSha = hash(audio);
const audioRelativePath = `audio/${audioSha}.wav`;
source.examId = examId;
source.assets.examId = examId;
source.assets.assets[0] = {
  ...source.assets.assets[0],
  assetId: "audio-part1",
  relativePath: audioRelativePath,
  sha256: audioSha,
  byteLength: audio.byteLength,
  durationMs: 1000
};
source.media.assetId = "audio-part1";
source.media.sha256 = audioSha;
source.media.durationMs = 1000;
source.parts[0].cue.endMs = 1000;
source.audit.sourceDocumentId = "phase7-listening-package-document";

const work = mkdtempSync(join(tmpdir(), "phase7-listening-package-"));
const readingRoot = join(work, "reading-exams");
const stagingRoot = join(readingRoot, `.phase6-staging-${examId}`);
const resourceRoot = join(stagingRoot, "resources", examId);
mkdirSync(resourceRoot, { recursive: true });
const descriptor = source.assets.assets[0];
const assetPath = join(resourceRoot, descriptor.relativePath.replaceAll("/", "\\"));
mkdirSync(dirname(assetPath), { recursive: true });
writeFileSync(assetPath, audio);

const assetManifest = {
  schemaVersion: "ExamAssetManifestV2",
  examId,
  generatedAt: "2026-08-13T00:00:00.000Z",
  assets: { [descriptor.assetId]: descriptor }
};
const assetManifestBytes = Buffer.from(JSON.stringify(assetManifest, null, 2) + "\n", "utf8");
writeFileSync(join(resourceRoot, "asset-manifest.json"), assetManifestBytes);

const runtimeBytes = Buffer.from(canonicalJson(source), "utf8");
const sourceScript = `(function registerListeningExamData(global) {\n  if (!global.__READING_EXAM_DATA__ || typeof global.__READING_EXAM_DATA__.register !== "function") throw new Error("reading_exam_registry_missing");\n  global.__READING_EXAM_DATA__.register(${JSON.stringify(examId)}, ${runtimeBytes.toString("utf8")});\n})(typeof window !== "undefined" ? window : globalThis);\n`;
writeFileSync(join(stagingRoot, `${examId}.js`), sourceScript, "utf8");

const manifestValue = {
  [examId]: {
    examId,
    dataKey: examId,
    script: `./${examId}.js`,
    title: source.meta.title,
    schemaVersion: "ListeningExamSourceV1",
    modality: "listening",
    minimumRuntimeVersion: "0.2.0",
    resourcesBase: `./resources/${examId}/`,
    assetManifest: `./resources/${examId}/asset-manifest.json`,
    checksums: {
      scriptSha256: hash(Buffer.from(sourceScript, "utf8")),
      assetManifestSha256: hash(assetManifestBytes),
      runtimeSha256: hash(runtimeBytes)
    }
  },
  _meta: { schemaVersion: "ReadingExamManifestV2", modality: "listening", generatedAt: "2026-08-13T00:00:00.000Z" }
};
function writeManifestScript() {
  writeFileSync(join(readingRoot, "manifest.js"), `window.__READING_EXAM_MANIFEST__ = ${JSON.stringify(manifestValue, null, 2)};\n`, "utf8");
}

// Manifest-last: the staging tree is complete and probeable before discovery
// manifest publication. The directory names deliberately match Phase 6.
assert(!existsSync(join(readingRoot, "manifest.js")), "manifest became visible before staged probe");
mkdirSync(join(readingRoot, "resources"), { recursive: true });
renameSyncSafe(join(stagingRoot, `${examId}.js`), join(readingRoot, `${examId}.js`));
renameSyncSafe(join(stagingRoot, "resources", examId), join(readingRoot, "resources", examId));
writeManifestScript();
assert(existsSync(join(readingRoot, "manifest.js")), "manifest-last publication did not complete");

run("npm", ["run", "build:server"], nasRoot);
const providerModule = await import(`file://${join(nasRoot, "server/dist/lib/library/listening/listening-v1-loader.js").replaceAll("\\", "/")}`);
const config = {
  mode: "exam",
  appVersion: "0.2.0",
  nas: {
    provider: "nas-js-direct",
    papersRoot: work,
    readingExamsRelative: "reading-exams",
    readingExplanationsRelative: "reading-explanations",
    writingExamsRelative: "writing-exams",
    submissionsRoot: work,
    resourcesRelative: null,
    allowLocalCache: false
  },
  storagePolicy: {
    persistQuestionPayload: false,
    persistAnswerDraft: false,
    persistSubmissionLocalMirror: false,
    allowSessionStorageSnapshot: false,
    allowLocalHistory: false
  }
};
const loaded = await new providerModule.NasJsDirectListeningAssetProvider(config).getAsset(examId);
assert(loaded.source.schemaVersion === "ListeningExamSourceV1", "Listening provider did not load the structured source");
assert(loaded.payload.modality === "listening", "Listening provider returned the wrong modality");
assert(loaded.payload.interactionModel.slots.q1, "Listening provider did not build slotId interaction model");
assert(loaded.audio.bytes.equals(audio), "Listening provider returned audio bytes different from staged asset");

const orphanManifest = {
  ...assetManifest,
  assets: {
    ...assetManifest.assets,
    orphan: { ...descriptor, assetId: "orphan", relativePath: `audio/${audioSha}.orphan.wav` }
  }
};
const orphanManifestBytes = Buffer.from(JSON.stringify(orphanManifest, null, 2) + "\n", "utf8");
writeFileSync(join(readingRoot, "resources", examId, "asset-manifest.json"), orphanManifestBytes);
manifestValue[examId].checksums.assetManifestSha256 = hash(orphanManifestBytes);
writeManifestScript();
let closureBlocked = false;
try {
  await new providerModule.NasJsDirectListeningAssetProvider(config).getAsset(examId);
} catch (error) {
  closureBlocked = String(error?.code || error?.message || error).includes("listening_asset_manifest_mismatch");
}
assert(closureBlocked, "Listening provider accepted an orphan staged asset descriptor");
writeFileSync(join(readingRoot, "resources", examId, "asset-manifest.json"), assetManifestBytes);
manifestValue[examId].checksums.assetManifestSha256 = hash(assetManifestBytes);
writeManifestScript();

const corrupted = Buffer.from(audio);
corrupted[corrupted.length - 1] ^= 0xff;
writeFileSync(join(readingRoot, "resources", examId, descriptor.relativePath.replaceAll("/", "\\")), corrupted);
let integrityBlocked = false;
try {
  await new providerModule.NasJsDirectListeningAssetProvider(config).getAsset(examId);
} catch (error) {
  integrityBlocked = String(error?.code || error?.message || error).includes("integrity");
}
assert(integrityBlocked, "Listening provider accepted corrupted audio");

console.log(JSON.stringify({
  schemaVersion: "Phase7ListeningPackageVerificationReportV1",
  examId,
  packageProtocol: "Phase6-NAS-V2-lock-CAS-journal-manifest-last",
  checks: ["staging", "manifest-last", "source-checksum", "asset-manifest-checksum", "asset-closure-no-orphan", "audio-realpath", "audio-size", "audio-sha256", "student-provider", "slotId-interaction-model", "corruption-fail-closed"],
  status: "passed"
}, null, 2));

rmSync(work, { recursive: true, force: true });

function renameSyncSafe(from, to) {
  mkdirSync(dirname(to), { recursive: true });
  renameSync(from, to);
}
