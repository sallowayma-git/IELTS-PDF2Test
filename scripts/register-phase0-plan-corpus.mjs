import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = path.join(repoRoot, "fixtures", "golden", "manifest.json");
const defaultSourceRoot = "C:\\Users\\lenovo\\Desktop\\working space\\0.3.1 working\\ReadingPractice\\PDF";
const metadataOnly = process.argv.includes("--metadata-only");
const sourceRoot = path.resolve(argumentValue("--source-root") ?? defaultSourceRoot);
const privateRoot = path.join(repoRoot, "fixtures", "golden", "private-real");
const metadataRoot = path.join(repoRoot, "fixtures", "golden", "metadata");
const baselineRoot = path.join(repoRoot, "fixtures", "golden", "baseline", "v1");

const cases = [
  {
    fixtureId: "fishbourne-roman-palace",
    sourceName: "8. P1 - Fishbourne Roman Palace 罗马宫殿.pdf",
    pageRoles: [["passage"], ["passage"], ["question"], ["question"], ["answer"]],
    groups: [[1, 6, "true_false_not_given"], [7, 13, "note_completion"]],
    assets: [{ id: "answer-page-raster", type: "answer_page_raster", required: true }],
    knownIssues: ["V1 classifies Questions 7-13 as sentence_completion; golden evidence identifies notes completion."]
  },
  {
    fixtureId: "listening-to-the-ocean",
    sourceName: "9. P1 - Listening to the Ocean 海洋探测.pdf",
    pageRoles: [["passage"], ["passage"], ["question"], ["question"], ["answer"], ["explanation"]],
    groups: [[1, 4, "true_false_not_given"], [5, 8, "matching_information"], [9, 13, "single_choice"]],
    assets: [],
    knownIssues: ["V1 emits parser warnings for non-extractable answer or explanation pages; those pages must not enter passage text."]
  },
  {
    fixtureId: "chili-peppers",
    sourceName: "7. P1 - Chili peppers 辣椒的历史.pdf",
    pageRoles: [["passage"], ["passage"], ["question"], ["question"], ["answer"]],
    groups: [[1, 6, "true_false_not_given"], [7, 13, "note_completion"]],
    assets: [{ id: "passage-embedded-image", type: "passage_image", required: true }],
    knownIssues: ["V1 reports zero assets although the passage contains a significant embedded image."]
  },
  {
    fixtureId: "petri-dish",
    sourceName: "105. P2 - How the Petri dish supports scientific advances 培养皿.pdf",
    pageRoles: [["passage"], ["passage"], ["question"], ["question"], ["question"], ["answer"], ["explanation"]],
    groups: [[14, 19, "matching_information"], [20, 25, "matching_features"], [26, 29, "summary_completion"]],
    assets: [],
    knownIssues: ["Questions 20-25 use one shared List of People option bank; per-question option copies are incorrect."]
  },
  {
    fixtureId: "organisational-design",
    sourceName: "106. P2 - Early Approaches to Organisational Design 组织设计.pdf",
    pageRoles: [["passage"], ["passage"], ["passage"], ["question"], ["question"], ["question"], ["answer"], ["explanation"]],
    groups: [[14, 15, "multiple_choice"], [16, 17, "multiple_choice"], [18, 19, "multiple_choice"], [20, 21, "multiple_choice"], [22, 26, "matching_features"]],
    assets: [],
    knownIssues: ["Each Choose TWO group has one shared prompt and option set with two unordered scored slots."]
  },
  {
    fixtureId: "western-celebrity",
    sourceName: "107. P2 - A study of western celebrity 西方名人.pdf",
    pageRoles: [["question"], ["passage"], ["passage"], ["question"], ["answer"]],
    groups: [[14, 20, "matching_headings"], [21, 23, "matching_features"], [24, 26, "summary_completion"]],
    assets: [],
    knownIssues: ["Questions 14-20 precede the passage; physical page order must not pollute passage assembly."]
  },
  {
    fixtureId: "conformity",
    sourceName: "138. P3 - Conformity 从众心理.pdf",
    pageRoles: [["passage"], ["passage"], ["question"], ["question"], ["answer"]],
    groups: [[27, 30, "yes_no_not_given"], [31, 35, "summary_completion"], [36, 40, "note_completion"]],
    assets: [{ id: "answer-page-raster", type: "answer_page_raster", required: true }],
    knownIssues: ["Adjacent groups switch between YNNG, summary completion, and notes completion."]
  },
  {
    fixtureId: "sleep-study",
    sourceName: "139. P1 - Sleep Study on Modern-Day Hunter-Gatherers Dispels Popular Notions 部落睡眠研究.pdf",
    pageRoles: [["passage"], ["passage"], ["question"], ["question"], ["explanation"], ["answer"]],
    groups: [[1, 4, "true_false_not_given"], [5, 13, "note_completion"]],
    assets: [{ id: "answer-explanation-raster", type: "answer_explanation_page_raster", required: true }],
    knownIssues: ["Answer and disputed-answer explanation pages must remain separate from passage and question content."]
  }
];

if (!metadataOnly && (!fs.existsSync(sourceRoot) || !fs.statSync(sourceRoot).isDirectory())) {
  throw new Error(`source root is not a directory: ${sourceRoot}`);
}
fs.mkdirSync(privateRoot, { recursive: true });
fs.mkdirSync(metadataRoot, { recursive: true });
fs.mkdirSync(baselineRoot, { recursive: true });

const manifest = readJson(manifestPath);
const fixtureById = new Map((manifest.fixtures ?? []).map((fixture) => [fixture.fixtureId, fixture]));
const registered = [];

for (const spec of cases) {
  const sourcePath = path.join(sourceRoot, spec.sourceName);
  if (!metadataOnly && (!fs.existsSync(sourcePath) || !fs.statSync(sourcePath).isFile())) {
    throw new Error(`required plan source missing: ${sourcePath}`);
  }
  const relativeSourcePath = `fixtures/golden/private-real/${spec.fixtureId}.pdf`;
  const targetPath = path.join(repoRoot, relativeSourcePath);
  if (metadataOnly) {
    if (!fs.existsSync(targetPath) || !fs.statSync(targetPath).isFile()) {
      throw new Error(`registered plan source missing: ${targetPath}`);
    }
  } else {
    fs.copyFileSync(sourcePath, targetPath);
  }
  const sha256 = sha256File(targetPath);
  const sizeBytes = fs.statSync(targetPath).size;
  const metadataPath = `fixtures/golden/metadata/${spec.fixtureId}.json`;
  const baselinePath = `fixtures/golden/baseline/v1/${spec.fixtureId}.json`;
  const fixture = {
    fixtureId: spec.fixtureId,
    status: "available",
    sourcePath: relativeSourcePath,
    originalName: spec.sourceName,
    sha256,
    sizeBytes,
    metadataPath,
    baselinePath
  };
  fixtureById.set(spec.fixtureId, fixture);

  const baseTaskGroups = spec.groups.map(([start, end, kind], index) => ({
    id: `group-${index + 1}`,
    displayRange: [start, end],
    kind,
    slotIds: range(start, end).map((number) => `q${number}`)
  }));
  const semantic = buildSemanticExpectations(spec.fixtureId, relativeSourcePath, baseTaskGroups);
  const taskGroups = baseTaskGroups.map((group) => ({
    ...group,
    responseGroupIds: semantic.responseGroups
      .filter((responseGroup) => responseGroup.taskGroupId === group.id)
      .map((responseGroup) => responseGroup.id),
    sourceEvidenceIds: semantic.sourceEvidence
      .filter((evidence) => evidence.taskGroupId === group.id)
      .map((evidence) => evidence.id)
  }));
  const slots = taskGroups.flatMap((group) => group.slotIds.map((id) => ({
    id,
    displayNumber: id.slice(1),
    responseType: responseTypeFor(group.kind)
  })));
  const existingMetadata = fs.existsSync(path.join(repoRoot, metadataPath))
    ? readJson(path.join(repoRoot, metadataPath))
    : null;
  const metadata = {
    schemaVersion: "GoldenFixtureMetadataV1",
    fixtureId: spec.fixtureId,
    source: {
      path: relativeSourcePath,
      originalName: spec.sourceName,
      sha256,
      sizeBytes,
      format: "pdf"
    },
    expected: {
      pageRoles: spec.pageRoles.map((roles, index) => ({ pageIndex: index + 1, roles })),
      taskGroups,
      slots,
      assets: spec.assets,
      optionBanks: semantic.optionBanks,
      responseGroups: semantic.responseGroups,
      sourceEvidence: semantic.sourceEvidence.map(({ taskGroupId: _taskGroupId, ...evidence }) => evidence),
      runtimeExpectations: semantic.runtimeExpectations
    },
    knownIssues: spec.knownIssues,
    review: {
      status: "approved",
      reviewedBy: "phase0-source-audit",
      reviewedAt: "2026-08-10",
      method: "source-text-and-overhaul-plan-evidence",
      evidence: [
        "Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md#2.1",
        "Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md#23.2",
        "fixtures/golden/private/legacy-reference.json"
      ]
    },
    baseline: existingMetadata?.baseline ?? {
      v1Path: baselinePath,
      observed: {}
    }
  };
  writeJson(path.join(repoRoot, metadataPath), metadata);
  registered.push({ fixtureId: spec.fixtureId, sourcePath: relativeSourcePath, sha256, sizeBytes });
}

manifest.fixtures = [...fixtureById.values()].sort((left, right) => left.fixtureId.localeCompare(right.fixtureId));
manifest.requiredPrivateCorpus = registered.map((entry) => ({
  fixtureId: entry.fixtureId,
  status: "available",
  sourcePath: entry.sourcePath,
  originalName: cases.find((item) => item.fixtureId === entry.fixtureId).sourceName,
  reviewRequired: true
}));
manifest.planCorpus = {
  sourcePlanSections: ["2.1", "16.1", "23.2"],
  requiredFixtureIds: cases.map((item) => item.fixtureId),
  registrationMethod: "exact-filename-authorized-local-source",
  registeredAt: "2026-08-10"
};
writeJson(manifestPath, manifest);
console.log(JSON.stringify({ schemaVersion: "Phase0PlanCorpusRegistrationV1", mode: metadataOnly ? "metadata-only" : "copy-and-register", sourceRoot, registered }, null, 2));

function buildSemanticExpectations(fixtureId, sourcePath, taskGroups) {
  const fixtureSpecs = {
    "fishbourne-roman-palace": [
      { pages: [3], optionLabels: ["TRUE", "FALSE", "NOT GIVEN"], allowReuse: true },
      { pages: [4] }
    ],
    "listening-to-the-ocean": [
      { pages: [3], optionLabels: ["TRUE", "FALSE", "NOT GIVEN"], allowReuse: true },
      { pages: [3], optionLabels: rangeLabels("A", "G"), allowReuse: true },
      { pages: [4], inlineLabels: rangeLabels("A", "D") }
    ],
    "chili-peppers": [
      { pages: [3], optionLabels: ["TRUE", "FALSE", "NOT GIVEN"], allowReuse: true },
      { pages: [4] }
    ],
    "petri-dish": [
      { pages: [3], optionLabels: rangeLabels("A", "F"), allowReuse: true },
      { pages: [4], optionLabels: rangeLabels("A", "D"), allowReuse: true },
      { pages: [5] }
    ],
    "organisational-design": [
      { pages: [4], promptMode: "shared", assignment: "unordered_set", optionLabels: rangeLabels("A", "E"), allowReuse: false },
      { pages: [4], promptMode: "shared", assignment: "unordered_set", optionLabels: rangeLabels("A", "E"), allowReuse: false },
      { pages: [5], promptMode: "shared", assignment: "unordered_set", optionLabels: rangeLabels("A", "E"), allowReuse: false },
      { pages: [5], promptMode: "shared", assignment: "unordered_set", optionLabels: rangeLabels("A", "E"), allowReuse: false },
      { pages: [6], optionLabels: rangeLabels("A", "D"), allowReuse: true }
    ],
    "western-celebrity": [
      { pages: [1], optionLabels: ["i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x"], allowReuse: false },
      { pages: [4], optionLabels: rangeLabels("A", "D"), allowReuse: false },
      { pages: [4] }
    ],
    conformity: [
      { pages: [3], optionLabels: ["YES", "NO", "NOT GIVEN"], allowReuse: true },
      { pages: [4] },
      { pages: [4] }
    ],
    "sleep-study": [
      { pages: [3], optionLabels: ["TRUE", "FALSE", "NOT GIVEN"], allowReuse: true },
      { pages: [4] }
    ]
  };
  const groupSpecs = fixtureSpecs[fixtureId];
  if (!groupSpecs || groupSpecs.length !== taskGroups.length) {
    throw new Error(`semantic fixture specification mismatch: ${fixtureId}`);
  }

  const optionBanks = [];
  const responseGroups = [];
  const sourceEvidence = [];
  for (const [index, taskGroup] of taskGroups.entries()) {
    const spec = groupSpecs[index];
    const evidenceId = `${taskGroup.id}-source`;
    const responseGroupId = `${taskGroup.id}-responses`;
    const isText = /completion|short_answer/.test(taskGroup.kind);
    const assignment = spec.assignment ?? "per_slot";
    const exact = assignment === "unordered_set" ? taskGroup.slotIds.length : 1;
    const optionBankId = spec.optionLabels ? `${taskGroup.id}-option-bank` : null;
    sourceEvidence.push({
      id: evidenceId,
      taskGroupId: taskGroup.id,
      evidenceType: "source_text",
      pageIndexes: spec.pages,
      references: spec.pages.map((pageIndex) => `${sourcePath}#page=${pageIndex}`)
    });
    if (optionBankId) {
      optionBanks.push({
        id: optionBankId,
        taskGroupId: taskGroup.id,
        scope: "task_group",
        labels: spec.optionLabels,
        allowReuse: spec.allowReuse,
        sourceEvidenceIds: [evidenceId]
      });
    }
    responseGroups.push({
      id: responseGroupId,
      taskGroupId: taskGroup.id,
      slotIds: taskGroup.slotIds,
      promptMode: spec.promptMode ?? (isText ? "embedded" : "per_slot"),
      cardinality: { min: exact, max: exact, exact },
      assignment,
      optionBinding: optionBankId
        ? { mode: "option_bank", optionBankId }
        : spec.inlineLabels
          ? { mode: "inline", inlineLabels: spec.inlineLabels }
          : { mode: "none" },
      reusePolicy: optionBankId || spec.inlineLabels
        ? (spec.allowReuse ? "allowed" : "disallowed")
        : "not_applicable",
      scoringPolicy: isText ? "per_slot_ielts_normalized" : "per_slot_binary",
      sourceEvidenceIds: [evidenceId]
    });
  }
  return {
    optionBanks,
    responseGroups,
    sourceEvidence,
    runtimeExpectations: {
      authoritativeVersion: "v1",
      artifactMode: "shadow_only",
      compileExpectation: "structure_must_compile",
      loaderExpectation: "test_only",
      rendererExpectation: "test_only",
      scoringExpectation: "per_slot",
      productionExposure: "disabled",
      requiredFeatureFlags: {
        documentIrV2Shadow: false,
        authoringV2Shadow: false,
        qualityGateV2: false,
        runtimeSourceV2: false,
        nasPackageV2: false
      }
    }
  };
}

function rangeLabels(start, end) {
  const first = start.codePointAt(0);
  const last = end.codePointAt(0);
  return Array.from({ length: last - first + 1 }, (_, index) => String.fromCodePoint(first + index));
}

function responseTypeFor(kind) {
  if (kind === "multiple_choice") return "checkbox";
  if (["true_false_not_given", "yes_no_not_given", "single_choice"].includes(kind)) return "radio";
  if (kind.startsWith("matching")) return "select";
  return "text";
}

function range(start, end) {
  return Array.from({ length: end - start + 1 }, (_, index) => start + index);
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function sha256File(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function writeJson(filePath, value) {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}
