import type {
  AnswerValue,
  AuthoringPatch,
  BuildPackInput,
  DocumentBlock,
  DocumentIr,
  ExportResult,
  GroupKind,
  ImportJob,
  JobFilter,
  JobMetaPatch,
  LlmProfilePublic,
  LlmSuggestion,
  LlmTestResult,
  EnvironmentPreflightReport,
  AutoPipelineReport,
  PackBuildResult,
  ParseOptions,
  PreviewAssets,
  ReadingAuthoringIr,
  SaveLlmProfileInput,
  SourceFile,
  SourceFileRole,
  SourceReview,
  SplitCandidates,
  ValidationIssue,
  ValidationReport
} from "../types";
import type { DiagnosticsSettings } from "../types/settings";
import { buildManifest, buildWrapper, escapeHtml, renderGroupBodyHtml, toReadingExamSource } from "./templateRenderer";

type Store = {
  jobs: ImportJob[];
  documents: Record<string, DocumentIr>;
  splits: Record<string, SplitCandidates>;
  authoring: Record<string, ReadingAuthoringIr>;
  validation: Record<string, ValidationReport>;
  previews: Record<string, PreviewAssets>;
  sourceReviews: Record<string, SourceReview>;
  profiles: LlmProfilePublic[];
  suggestions: Record<string, LlmSuggestion[]>;
  pipelineReports: Record<string, AutoPipelineReport>;
  packs: PackBuildResult[];
  diagnostics: DiagnosticsSettings;
};

export interface JobDetail {
  job: ImportJob;
  documentIr?: DocumentIr;
  sourceReview?: SourceReview;
  splitCandidates?: SplitCandidates;
  authoringIr?: ReadingAuthoringIr;
  validationReport?: ValidationReport;
  previewAssets?: PreviewAssets;
  pipelineReport?: AutoPipelineReport;
  llmSuggestions: LlmSuggestion[];
}

const STORE_KEY = "ielts-author-studio.dev-fallback-store.v1";

function now(): string {
  return new Date().toISOString();
}

function id(prefix: string): string {
  const stamp = new Date().toISOString().replace(/[-:.TZ]/g, "").slice(0, 14);
  return `${prefix}-${stamp}-${Math.random().toString(36).slice(2, 7)}`;
}

function initialStore(): Store {
  return {
    jobs: [],
    documents: {},
    splits: {},
    authoring: {},
    validation: {},
    previews: {},
    sourceReviews: {},
    profiles: [
      {
        profileId: "profile-local-placeholder",
        name: "Local Placeholder Gateway",
        provider: "OpenAiCompatible",
        baseUrl: "http://localhost:11434/v1",
        model: "local-placeholder-structurer",
        temperature: 0,
        timeoutMs: 60000,
        forceJson: true,
        enabled: true,
        hasApiKey: false
      }
    ],
    suggestions: {},
    pipelineReports: {},
    packs: [],
    diagnostics: { keepFullProcessArtifacts: false }
  };
}

function load(): Store {
  const raw = localStorage.getItem(STORE_KEY);
  if (!raw) return initialStore();
  try {
    return { ...initialStore(), ...JSON.parse(raw) };
  } catch {
    return initialStore();
  }
}

function save(store: Store): void {
  localStorage.setItem(STORE_KEY, JSON.stringify(store));
}

function updateJob(store: Store, jobId: string, patch: Partial<ImportJob>): ImportJob {
  const index = store.jobs.findIndex((job) => job.jobId === jobId);
  if (index < 0) throw new Error(`job_not_found:${jobId}`);
  store.jobs[index] = { ...store.jobs[index], ...patch, updatedAt: now() };
  return store.jobs[index];
}

function requireJob(store: Store, jobId: string): ImportJob {
  const job = store.jobs.find((item) => item.jobId === jobId);
  if (!job) throw new Error(`job_not_found:${jobId}`);
  return job;
}

function detectFileType(name: string): SourceFile["fileType"] {
  const ext = name.toLowerCase().split(".").pop();
  if (ext === "pdf") return "pdf";
  if (ext === "docx") return "docx";
  if (ext === "txt") return "txt";
  if (ext === "md") return "md";
  if (["png", "jpg", "jpeg", "webp"].includes(ext ?? "")) return "image";
  return "unknown";
}

function sampleBlocks(job: ImportJob): DocumentBlock[] {
  return [
    {
      blockId: "b001",
      blockType: "header",
      text: "READING PASSAGE 1",
      html: "<h2>READING PASSAGE 1</h2>",
      bbox: [72, 60, 460, 88],
      confidence: 0.99,
      roleHint: "passage"
    },
    {
      blockId: "b002",
      blockType: "paragraph",
      text: job.title || "The Rise and Fall of Detective Stories",
      html: `<h3>${escapeHtml(job.title || "The Rise and Fall of Detective Stories")}</h3>`,
      bbox: [72, 100, 520, 130],
      confidence: 0.97,
      roleHint: "passage"
    },
    {
      blockId: "b003",
      blockType: "paragraph",
      text: "Detective fiction developed from short literary experiments into a recognizable public genre. Early writers used clues, alibis, and narrators to teach readers how to reason through a mystery.",
      html: "<p>Detective fiction developed from short literary experiments into a recognizable public genre. Early writers used clues, alibis, and narrators to teach readers how to reason through a mystery.</p>",
      bbox: [72, 145, 520, 210],
      confidence: 0.96,
      roleHint: "passage"
    },
    {
      blockId: "b004",
      blockType: "paragraph",
      text: "Questions 1-5 Do the following statements agree with the information given in Reading Passage 1? TRUE if the statement agrees, FALSE if it contradicts, NOT GIVEN if there is no information.",
      html: "<h3>Questions 1-5</h3><p>Do the following statements agree with the information given in Reading Passage 1?</p>",
      bbox: [72, 250, 520, 320],
      confidence: 0.94,
      roleHint: "question"
    },
    {
      blockId: "b005",
      blockType: "list",
      text: "1 Detective fiction first appeared as a public genre before short literary experiments. 2 Early detective stories trained readers to interpret clues. 3 Every early detective writer used a police officer as narrator. 4 Alibis were one device used in the genre. 5 The passage says detective fiction disappeared in the twentieth century.",
      html: "<ol><li>Detective fiction first appeared as a public genre before short literary experiments.</li><li>Early detective stories trained readers to interpret clues.</li><li>Every early detective writer used a police officer as narrator.</li><li>Alibis were one device used in the genre.</li><li>The passage says detective fiction disappeared in the twentieth century.</li></ol>",
      bbox: [72, 330, 520, 520],
      confidence: 0.91,
      roleHint: "question"
    },
    {
      blockId: "b006",
      blockType: "paragraph",
      text: "Questions 6-8 Complete the table below. Choose ONE WORD ONLY from the passage for each answer.",
      html: "<h3>Questions 6-8</h3><p>Complete the table below. Choose ONE WORD ONLY from the passage for each answer.</p>",
      bbox: [72, 540, 520, 590],
      confidence: 0.95,
      roleHint: "question"
    },
    {
      blockId: "b007",
      blockType: "table",
      text: "Feature | Function | clues | help readers reason | alibis | complicate the mystery | narrators | guide interpretation",
      html: "<table><tr><th>Feature</th><th>Function</th></tr><tr><td>clues</td><td>help readers reason</td></tr><tr><td>alibis</td><td>complicate the mystery</td></tr><tr><td>narrators</td><td>guide interpretation</td></tr></table>",
      table: {
        rows: 4,
        cols: 2,
        cells: [
          { row: 0, col: 0, text: "Feature" },
          { row: 0, col: 1, text: "Function" },
          { row: 1, col: 0, text: "clues" },
          { row: 1, col: 1, text: "help readers reason" },
          { row: 2, col: 0, text: "alibis" },
          { row: 2, col: 1, text: "complicate the mystery" },
          { row: 3, col: 0, text: "narrators" },
          { row: 3, col: 1, text: "guide interpretation" }
        ]
      },
      bbox: [72, 600, 520, 760],
      confidence: 0.93,
      roleHint: "question"
    },
    {
      blockId: "b008",
      blockType: "paragraph",
      text: "Answers 1 FALSE 2 TRUE 3 NOT GIVEN 4 TRUE 5 FALSE 6 clues 7 alibis 8 narrators",
      html: "<p>Answers: 1 FALSE; 2 TRUE; 3 NOT GIVEN; 4 TRUE; 5 FALSE; 6 clues; 7 alibis; 8 narrators.</p>",
      bbox: [72, 780, 520, 820],
      confidence: 0.9,
      roleHint: "answer"
    }
  ];
}

function makeDocumentIr(job: ImportJob, options: ParseOptions): DocumentIr {
  return {
    schemaVersion: "DocumentIRV1",
    jobId: job.jobId,
    pages: [
      {
        pageIndex: 1,
        width: 595,
        height: 842,
        blocks: sampleBlocks(job)
      }
    ],
    assets: [],
    parser: {
      provider: "local-parser-placeholder",
      version: "0.1.0",
      mode: options.mode,
      warnings: options.mode === "ocr" ? ["OCR 识别结果需要人工确认。"] : []
    }
  };
}

function makeManualDocumentIr(job: ImportJob, text: string): DocumentIr {
  const blocks = text
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .split(/\n\s*\n/)
    .map((item) => item.trim())
    .filter(Boolean)
    .map((item, index): DocumentBlock => ({
      blockId: `b${String(index + 1).padStart(3, "0")}`,
      blockType: /^Questions?\s+\d/i.test(item) ? "header" : "paragraph",
      text: item,
      html: `<p>${escapeHtml(item)}</p>`,
      bbox: [72, 72 + index * 42, 520, 108 + index * 42],
      confidence: 1,
      roleHint: /^Questions?\s+\d/i.test(item) ? "question" : /^Answers?/i.test(item) || /answer key/i.test(item) ? "answer" : index === 0 ? "passage" : undefined
    }));
  return {
    schemaVersion: "DocumentIRV1",
    jobId: job.jobId,
    pages: [{ pageIndex: 1, width: 595, height: 842, blocks }],
    assets: [],
    parser: {
      provider: "manual-transcription",
      version: "0.3.0",
      mode: "manual",
      warnings: ["已使用人工转录内容；发布前请对照源文件确认。"]
    }
  };
}

function makeVisionDocumentIr(job: ImportJob): DocumentIr {
  const text = `READING PASSAGE 1
Vision model transcription placeholder for ${job.title}. Human review is required before publish.

Questions 1-2
1 The uploaded scanned PDF requires visual transcription.
2 The author must verify the generated text.

Answers
1 TRUE
2 TRUE`;
  const ir = makeManualDocumentIr(job, text);
  return {
    ...ir,
    parser: {
      provider: "vision-llm-transcription",
      version: "0.1.0",
      mode: "ocr",
      warnings: ["已使用视觉模型转录；发布前请对照源文件确认。", "开发预览占位结果"]
    }
  };
}

function parserWarnings(doc?: DocumentIr): string[] {
  return doc?.parser.warnings ?? [];
}

function lowConfidenceBlockIds(doc?: DocumentIr, threshold = 0.5): string[] {
  return flattenBlocks(doc)
    .filter((block) => block.confidence < threshold)
    .map((block) => block.blockId);
}

function sourceReviewFingerprint(doc?: DocumentIr): string {
  return JSON.stringify({ warnings: parserWarnings(doc), lowConfidenceBlocks: lowConfidenceBlockIds(doc) });
}

function sourceReviewStatus(store: Store, jobId: string): SourceReview {
  const doc = store.documents[jobId];
  const saved = store.sourceReviews[jobId];
  if (!doc && saved) return saved;
  const parserWarningsList = parserWarnings(doc);
  const lowConfidenceBlocks = lowConfidenceBlockIds(doc);
  const required = parserWarningsList.length > 0 || lowConfidenceBlocks.length > 0;
  const fingerprint = sourceReviewFingerprint(doc);
  const stale = required && Boolean(saved?.resolved) && saved?.fingerprint !== fingerprint;
  return {
    schemaVersion: "SourceReviewV1",
    jobId,
    required,
    resolved: !required || (Boolean(saved?.resolved) && !stale),
    stale,
    fingerprint,
    parserWarnings: parserWarningsList,
    lowConfidenceBlocks,
    resolvedAt: saved?.resolvedAt ?? null,
    note: saved?.note ?? null
  };
}

function sourceReviewIssues(review: SourceReview): ValidationIssue[] {
  if (!review.required || review.resolved) return [];
  const issues: ValidationIssue[] = [];
  for (const warning of review.parserWarnings) {
    issues.push({ issueId: id("issue"), severity: "error", layer: "AuthoringIR", path: "$.sourceReview.parserWarnings", message: `解析提醒需人工确认后才能发布：${warning}` });
  }
  for (const blockId of review.lowConfidenceBlocks) {
    issues.push({ issueId: id("issue"), severity: "error", layer: "AuthoringIR", path: `$.sourceReview.lowConfidenceBlocks.${blockId}`, message: "低置信解析内容需人工确认后才能发布。" });
  }
  if (!issues.length) {
    issues.push({ issueId: id("issue"), severity: "error", layer: "AuthoringIR", path: "$.sourceReview.resolved", message: "源文档审核完成后才能发布。" });
  }
  return issues;
}

function flattenBlocks(doc?: DocumentIr): DocumentBlock[] {
  type OrderedBlock = DocumentBlock & { pageIndex?: number; __pageWidth: number; __pageHeight: number; __pageRotation: number; __originalOrder: number };
  return (doc?.pages.flatMap((page, pagePosition) => {
    const pageIndex = page.pageIndex ?? pagePosition + 1;
    const pageWidth = page.width ?? 595;
    const pageHeight = page.height ?? 842;
    const pageRotation = page.rotation ?? (typeof page.layoutHints?.rotation === "number" ? page.layoutHints.rotation : 0);
    return page.blocks.map((block, blockPosition) => ({
      ...block,
      pageIndex,
      __pageWidth: pageWidth,
      __pageHeight: pageHeight,
      __pageRotation: pageRotation,
      __originalOrder: blockPosition
    } as OrderedBlock));
  }) ?? []).sort((left, right) => {
    const leftBox = normalizedBlockBbox(left) ?? [0, 0, 0, 0];
    const rightBox = normalizedBlockBbox(right) ?? [0, 0, 0, 0];
    const leftRole = left.roleHint === "answer" ? 3 : left.roleHint === "ignore" ? 4 : 0;
    const rightRole = right.roleHint === "answer" ? 3 : right.roleHint === "ignore" ? 4 : 0;
    const leftColumn = blockColumn(left);
    const rightColumn = blockColumn(right);
    return ((left as OrderedBlock).pageIndex ?? 1) - ((right as OrderedBlock).pageIndex ?? 1)
      || leftRole - rightRole
      || leftColumn - rightColumn
      || leftBox[1] - rightBox[1]
      || leftBox[0] - rightBox[0]
      || ((left as typeof left & { __originalOrder?: number }).__originalOrder ?? 0) - ((right as typeof right & { __originalOrder?: number }).__originalOrder ?? 0);
  }).map(({ __pageWidth: _pageWidth, __pageHeight: _pageHeight, __pageRotation: _pageRotation, __originalOrder: _originalOrder, ...block }) => block as DocumentBlock);
}

function blockText(block: DocumentBlock): string {
  const text = block.text?.trim();
  if (text) return text.replace(/\s+/g, " ").trim();
  return (block.html ?? "").replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
}

function detectQuestionRange(text: string): [number, number] | undefined {
  const range = text.match(/Questions?\s+(\d{1,3})\s*[-–—]\s*(\d{1,3})/i);
  if (range) return [Number(range[1]), Number(range[2])];
  const single = text.match(/Questions?\s+(\d{1,3})\b/i);
  if (single) return [Number(single[1]), Number(single[1])];
  return undefined;
}

function isUmbrellaQuestionRange(text: string): boolean {
  const range = detectQuestionRange(text);
  return Boolean(range && range[1] > range[0] && hasUmbrellaQuestionContext(text));
}

function isBareQuestionRangeHeading(text: string): boolean {
  const range = detectQuestionRange(text);
  if (!range || range[1] <= range[0] || !isQuestionHeadingText(text)) return false;
  const heading = questionHeading(range[0], range[1]).toLowerCase().replace(/\s+/g, "");
  const normalized = text.trimStart().replace(/^#+\s*/, "").toLowerCase().replace(/[–—]/g, "-").replace(/\s+/g, "");
  const suffix = normalized.slice(heading.length).replace(/[.:;\-\s]+$/g, "");
  return normalized.startsWith(heading) && !suffix;
}

function normalizedQuestionContext(text: string): string {
  return text.toLowerCase().split(/\s+/).filter(Boolean).join(" ");
}

function hasUmbrellaQuestionContext(text: string): boolean {
  const lower = normalizedQuestionContext(text);
  if (!lower.includes("reading passage")) return false;
  const basedOnPassage = lower.includes("based on reading passage") || lower.includes("based on the reading passage");
  const passageReference = lower.includes("refer to reading passage")
    || lower.includes("refer to the reading passage")
    || lower.includes("relate to reading passage")
    || lower.includes("relate to the reading passage");

  return lower.includes("which are based on reading passage")
    || lower.includes("which are based on the reading passage")
    || lower.includes("which is based on reading passage")
    || lower.includes("which is based on the reading passage")
    || (basedOnPassage && (lower.includes("below") || lower.includes("you should spend")))
    || (passageReference && lower.includes("below"))
    || (lower.includes("you should spend") && lower.includes("about"));
}

function nearbyQuestionContext(blocks: DocumentBlock[], index: number): string {
  return normalizedQuestionContext(blocks.slice(Math.max(0, index - 3), Math.min(blocks.length, index + 4)).map(blockText).join(" "));
}

function isReadingPassageHeading(text: string): boolean {
  return text.trimStart().toUpperCase().startsWith("READING PASSAGE");
}

function isSubstantivePassageBlock(block: DocumentBlock): boolean {
  const text = blockText(block);
  return text.length >= 48
    && (block.roleHint === "passage" || (!isQuestionBlock(block) && !isAnswerBlock(block) && !isReadingPassageHeading(text)));
}

function hasOpeningQuestionRangePosition(blocks: DocumentBlock[], index: number): boolean {
  let headerIndex = -1;
  for (let candidateIndex = index; candidateIndex >= 0; candidateIndex -= 1) {
    if (isReadingPassageHeading(blockText(blocks[candidateIndex]))) {
      headerIndex = candidateIndex;
      break;
    }
  }
  if (headerIndex < 0 || index - headerIndex > 4) return false;
  return !blocks.slice(headerIndex + 1, index).some(isSubstantivePassageBlock);
}

function hasLaterConcreteSubgroup(blocks: DocumentBlock[], index: number, start: number, end: number): boolean {
  return blocks.slice(index + 1).some((candidate) => {
    const text = blockText(candidate);
    const range = detectQuestionHeadingRange(text);
    return Boolean(range
      && range[1] > range[0]
      && range[0] >= start
      && range[1] <= end
      && !(range[0] === start && range[1] === end)
      && !isUmbrellaQuestionRange(text));
  });
}

function isUmbrellaQuestionBlock(blocks: DocumentBlock[], index: number): boolean {
  const text = blockText(blocks[index]);
  if (isUmbrellaQuestionRange(text)) return true;
  if (!isBareQuestionRangeHeading(text)) return false;
  const range = detectQuestionRange(text);
  if (!range) return false;
  const isFullPassageSpan = range[1] - range[0] >= 9;
  const nearby = nearbyQuestionContext(blocks, index);
  const hasOpeningContext = isFullPassageSpan
    && hasOpeningQuestionRangePosition(blocks, index)
    && (nearby.includes("reading passage") || (nearby.includes("you should spend") && nearby.includes("about")));
  return hasOpeningContext || hasLaterConcreteSubgroup(blocks, index, range[0], range[1]);
}

function isKnownUmbrellaBlock(block: DocumentBlock, umbrellaBlocks: DocumentBlock[]): boolean {
  return umbrellaBlocks.some((candidate) => candidate.blockId === block.blockId);
}

function isQuestionHeadingText(text: string): boolean {
  return text.trimStart().replace(/^#+\s*/, "").toLowerCase().startsWith("questions ")
    || text.trimStart().replace(/^#+\s*/, "").toLowerCase().startsWith("question ");
}

function detectQuestionHeadingRange(text: string): [number, number] | undefined {
  return isQuestionHeadingText(text) ? detectQuestionRange(text) : undefined;
}

function isQuestionBlock(block: DocumentBlock): boolean {
  return block.roleHint === "question" || Boolean(detectQuestionRange(blockText(block)));
}

function isAnswerBlock(block: DocumentBlock): boolean {
  const text = blockText(block);
  return block.roleHint === "answer" || /^Answers?/i.test(text) || /answer key/i.test(text);
}

function questionHeading(start: number, end: number): string {
  return start === end ? `Questions ${start}` : `Questions ${start}-${end}`;
}

function detectGroupKind(text: string): GroupKind {
  const lower = text.toLowerCase();
  if (lower.includes("true") && lower.includes("false") && lower.includes("not given")) return "true_false_not_given";
  if (lower.includes("yes") && lower.includes("no") && lower.includes("not given")) return "yes_no_not_given";
  if (lower.includes("choose") && lower.includes("letter") && (lower.includes("two") || lower.includes("three"))) return "multi_choice";
  if (lower.includes("complete the table") || lower.includes("table below") || lower.includes("|") && lower.includes("complete")) return "table_completion";
  if (lower.includes("list of headings") || lower.includes("matching headings")) return "heading_matching";
  if (lower.includes("which paragraph contains") || lower.includes("matching information") || lower.includes("match each statement")) return "matching_information";
  if (lower.includes("classify") || lower.includes("classification") || lower.includes("according to")) return "classification";
  if (lower.includes("match") && lower.includes("letter")) return "matching";
  if (lower.includes("choose") && lower.includes("letter") && /\b[A-D]\b/.test(text)) return "single_choice";
  if (lower.includes("complete the summary")) return "summary_completion";
  if (lower.includes("complete the sentence") || lower.includes("complete the sentences")) return "sentence_completion";
  if (lower.includes("short answer")) return "short_answer";
  return "short_answer";
}

function letterOptionsForText(text: string): string[] {
  const lower = text.toLowerCase();
  if (lower.includes("a-g") || lower.includes("list of headings")) return ["A", "B", "C", "D", "E", "F", "G"];
  if (lower.includes("a-f")) return ["A", "B", "C", "D", "E", "F"];
  if (lower.includes("a-e")) return ["A", "B", "C", "D", "E"];
  return ["A", "B", "C", "D"];
}

function selectionCount(text: string): number | undefined {
  const lower = text.toLowerCase();
  if (lower.includes("choose three") || lower.includes("three letters")) return 3;
  if (lower.includes("choose two") || lower.includes("two letters")) return 2;
  return undefined;
}

function optionReuseRule(kind: GroupKind, text: string): { allowOptionReuse: boolean; warning?: string } {
  const lower = text.toLowerCase();
  if (lower.includes("may use any letter more than once") || lower.includes("may be used more than once") || lower.includes("use any letter more than once")) return { allowOptionReuse: true };
  if (lower.includes("each option may be used once only") || lower.includes("use each letter once only") || lower.includes("each letter may be used once only")) return { allowOptionReuse: false };
  if (kind === "classification" || kind === "matching_information") return { allowOptionReuse: true, warning: "Option reuse was inferred from question type; source wording did not state it explicitly." };
  if (kind === "heading_matching" || kind === "matching" || kind === "single_choice" || kind === "multi_choice") return { allowOptionReuse: false, warning: "Option reuse was inferred from question type; source wording did not state it explicitly." };
  return { allowOptionReuse: false };
}

function classifyGroup(text: string, blockIds: string[]): NonNullable<SplitCandidates["questionGroupCandidates"][number]["classification"]> {
  const kind = detectGroupKind(text);
  const reuse = optionReuseRule(kind, text);
  const warnings = reuse.warning ? [reuse.warning] : [];
  const interaction =
    kind === "true_false_not_given" ? { type: "radio" as const, options: ["TRUE", "FALSE", "NOT GIVEN"], allowOptionReuse: reuse.allowOptionReuse }
    : kind === "yes_no_not_given" ? { type: "radio" as const, options: ["YES", "NO", "NOT GIVEN"], allowOptionReuse: reuse.allowOptionReuse }
    : kind === "single_choice" ? { type: "radio" as const, options: letterOptionsForText(text), allowOptionReuse: reuse.allowOptionReuse }
    : kind === "multi_choice" ? { type: "checkbox" as const, options: letterOptionsForText(text), allowOptionReuse: reuse.allowOptionReuse, minSelections: selectionCount(text) ?? 2, maxSelections: selectionCount(text) ?? 2 }
    : kind === "heading_matching" || kind === "matching" || kind === "matching_information" || kind === "classification" ? { type: "matching" as const, options: letterOptionsForText(text), allowOptionReuse: reuse.allowOptionReuse }
    : { type: "text" as const, placeholder: "answer" };
  return { kind, interaction, confidence: warnings.length ? 0.68 : 0.82, warnings, evidence: blockIds };
}

function blockColumn(block: DocumentBlock): number {
  const box = normalizedBlockBbox(block);
  const ordered = block as DocumentBlock & { __pageWidth?: number; __pageHeight?: number; __pageRotation?: number };
  const pageWidth = [90, 270].includes(normalizeRotation(ordered.__pageRotation ?? 0)) ? ordered.__pageHeight ?? 842 : ordered.__pageWidth ?? 595;
  return (box?.[0] ?? 0) >= pageWidth * 0.45 ? 1 : 0;
}

function normalizeRotation(value: number): number {
  return ((value % 360) + 360) % 360;
}

function rotatePointToUpright(x: number, y: number, width: number, height: number, rotation: number): [number, number] {
  switch (normalizeRotation(rotation)) {
    case 90: return [y, width - x];
    case 180: return [width - x, height - y];
    case 270: return [height - y, x];
    default: return [x, y];
  }
}

function normalizedBlockBbox(block: DocumentBlock): [number, number, number, number] | undefined {
  if (!block.bbox) return undefined;
  const ordered = block as DocumentBlock & { __pageWidth?: number; __pageHeight?: number; __pageRotation?: number };
  const rotation = normalizeRotation(ordered.__pageRotation ?? 0);
  if (rotation === 0) return block.bbox;
  const width = ordered.__pageWidth ?? 595;
  const height = ordered.__pageHeight ?? 842;
  const [x0, y0, x1, y1] = block.bbox;
  const points = [
    rotatePointToUpright(x0, y0, width, height, rotation),
    rotatePointToUpright(x1, y0, width, height, rotation),
    rotatePointToUpright(x1, y1, width, height, rotation),
    rotatePointToUpright(x0, y1, width, height, rotation)
  ];
  return [
    Math.min(...points.map(([x]) => x)),
    Math.min(...points.map(([, y]) => y)),
    Math.max(...points.map(([x]) => x)),
    Math.max(...points.map(([, y]) => y))
  ];
}

function sectionEvidenceForBlocks(blocks: DocumentBlock[]): NonNullable<SplitCandidates["questionGroupCandidates"][number]["sectionEvidence"]> {
  const hintNumber = (block: DocumentBlock, path: string[]): number | undefined => {
    let value: unknown = block.layoutHints;
    for (const key of path) value = typeof value === "object" && value !== null ? (value as Record<string, unknown>)[key] : undefined;
    return typeof value === "number" ? value : undefined;
  };
  const hintString = (block: DocumentBlock, path: string[]): string | undefined => {
    let value: unknown = block.layoutHints;
    for (const key of path) value = typeof value === "object" && value !== null ? (value as Record<string, unknown>)[key] : undefined;
    return typeof value === "string" ? value : undefined;
  };
  return blocks.map((block) => ({
    blockId: block.blockId,
    pageIndex: block.pageIndex ?? 1,
    column: blockColumn(block),
    role: block.roleHint ?? "",
    textPreview: blockText(block).slice(0, 120),
    bbox: block.bbox,
    normalizedBbox: normalizedBlockBbox(block),
    pageRotation: normalizeRotation((block as DocumentBlock & { __pageRotation?: number }).__pageRotation ?? 0),
    tableRows: block.table?.rows,
    tableCols: block.table?.cols,
    tableHasColSpans: block.table ? block.table.cells.some((cell) => (cell.colSpan ?? 1) > 1) : undefined,
    tableHasVerticalMerges: block.table ? block.table.cells.some((cell) => Boolean(cell.verticalMerge)) : undefined,
    tableMergedCellCount: block.table ? block.table.cells.filter((cell) => (cell.colSpan ?? 1) > 1 || Boolean(cell.verticalMerge)).length : undefined,
    headingLevel: hintNumber(block, ["headingLevel"]),
    numberingLevel: hintNumber(block, ["numbering", "level"]),
    numberingId: hintString(block, ["numbering", "id"])
  }));
}

function continuationEdgesForBlocks(blocks: DocumentBlock[]): NonNullable<SplitCandidates["questionGroupCandidates"][number]["continuationEdges"]> {
  return blocks.slice(1).map((block, index) => {
    const previous = blocks[index];
    const reason = (previous.pageIndex ?? 1) !== (block.pageIndex ?? 1)
      ? "cross-page-continuation"
      : blockColumn(previous) !== blockColumn(block)
        ? "cross-column-continuation"
        : "same-section-continuation";
    return {
      fromBlockId: previous.blockId,
      toBlockId: block.blockId,
      reason,
      confidence: 0.72
    };
  });
}

function parseAnswerText(text: string): Record<string, AnswerValue> {
  const answers: Record<string, AnswerValue> = {};
  const normalized = text.replace(/[;,\n]+/g, " ").replace(/\s+/g, " ").trim();
  const pattern = /(?:^|\s)(\d{1,3})\s*[).:-]?\s+((?:NOT\s+GIVEN)|TRUE|FALSE|YES|NO|[A-D]|[A-Za-z][A-Za-z-]*)(?=\s+\d{1,3}\s*[).:-]?\s+|$)/gi;
  for (const match of normalized.matchAll(pattern)) {
    const raw = match[2].trim();
    const upper = raw.toUpperCase();
    answers[match[1]] = ["TRUE", "FALSE", "YES", "NO", "NOT GIVEN", "A", "B", "C", "D"].includes(upper) ? upper : raw;
  }
  return answers;
}

function answerSourceCandidates(job?: ImportJob): SplitCandidates["answerKeyCandidates"] {
  return (job?.sourceFiles ?? [])
    .filter((source) => source.role === "AnswerKey")
    .map((source) => ({
      source: `answer-source:${source.fileId}`,
      answers: parseAnswerText(source.originalName.replace(/\.[^.]+$/, " "))
    }))
    .filter((candidate) => Object.keys(candidate.answers).length > 0);
}

function inferPassageTitle(job: ImportJob, passageBlocks: DocumentBlock[]): string {
  const title = passageBlocks
    .map(blockText)
    .find((text) => text && !/^READING PASSAGE/i.test(text));
  return title ?? job.title;
}

function makeSplit(jobId: string, doc?: DocumentIr, job?: ImportJob): SplitCandidates {
  const blocks = flattenBlocks(doc);
  if (!blocks.length) return makeStaticSplit(jobId);

  const firstQuestionIndex = blocks.findIndex(isQuestionBlock);
  const firstConcreteQuestionIndex = blocks.findIndex((block, index) => {
    const text = blockText(block);
    return Boolean(detectQuestionHeadingRange(text)) && !isUmbrellaQuestionBlock(blocks, index);
  });
  const firstAnswerIndex = blocks.findIndex(isAnswerBlock);
  const passageBlocks =
    firstConcreteQuestionIndex >= 0
      ? blocks.filter((block, index) => index < firstConcreteQuestionIndex && !isQuestionBlock(block) && !isAnswerBlock(block) && block.roleHint !== "ignore")
      : blocks.filter((block) => !isQuestionBlock(block) && !isAnswerBlock(block) && block.roleHint !== "ignore");
  const allUmbrellaBlocks = blocks.filter((_, index) => isUmbrellaQuestionBlock(blocks, index));
  const questionBlocks =
    firstConcreteQuestionIndex >= 0
      ? blocks
          .slice(firstConcreteQuestionIndex)
          .filter((block) => !isAnswerBlock(block) && block.roleHint !== "ignore")
      : allUmbrellaBlocks.length
        ? allUmbrellaBlocks
        : firstQuestionIndex >= 0
          ? blocks
              .slice(firstQuestionIndex)
              .filter((block) => !isAnswerBlock(block) && block.roleHint !== "ignore")
          : blocks.filter(isQuestionBlock);
  const answerBlocks = blocks.filter(isAnswerBlock);

  const answerMap = answerBlocks.reduce<Record<string, AnswerValue>>((acc, block) => ({ ...acc, ...parseAnswerText(blockText(block)) }), {});
  const externalAnswerCandidates = answerSourceCandidates(job);
  for (const candidate of externalAnswerCandidates) {
    Object.assign(answerMap, candidate.answers);
  }
  const answerNumbers = Object.keys(answerMap).map(Number).filter(Number.isFinite).sort((a, b) => a - b);
  const umbrellaQuestionRanges = allUmbrellaBlocks
    .map((block) => {
      const text = blockText(block);
      const range = detectQuestionRange(text);
      if (!range) return null;
      return {
        heading: questionHeading(range[0], range[1]),
        questionRange: range,
        blockId: block.blockId,
        text
      };
    })
    .filter((range): range is NonNullable<typeof range> => Boolean(range));

  const questionGroupCandidates = questionBlocks
    .map((block, index) => {
      const text = blockText(block);
      if (isKnownUmbrellaBlock(block, allUmbrellaBlocks)) return null;
      const range = detectQuestionHeadingRange(text);
      if (!range) return null;
      const nextHeadingIndex = questionBlocks.findIndex((candidate, candidateIndex) => {
        const candidateText = blockText(candidate);
        return candidateIndex > index && Boolean(detectQuestionHeadingRange(candidateText)) && !isKnownUmbrellaBlock(candidate, allUmbrellaBlocks);
      });
      const included = nextHeadingIndex > -1 ? questionBlocks.slice(index, nextHeadingIndex) : questionBlocks.slice(index);
      const blockIds = included.map((item) => item.blockId);
      const classification = classifyGroup(included.map(blockText).join(" "), blockIds);
      return {
        groupId: `group-${index + 1}`,
        heading: text.match(/Questions?\s+\d{1,3}(?:\s*[-–—]\s*\d{1,3})?/i)?.[0] ?? questionHeading(range[0], range[1]),
        questionRange: range,
        instructionText: text,
        blockIds,
        kindHint: classification.kind,
        confidence: classification.confidence,
        classification,
        sectionEvidence: sectionEvidenceForBlocks(included),
        continuationEdges: continuationEdgesForBlocks(included)
      };
    })
    .filter(Boolean) as SplitCandidates["questionGroupCandidates"];
  questionGroupCandidates.forEach((candidate, index) => {
    candidate.groupId = `group-${index + 1}`;
  });

  if (!questionGroupCandidates.length && umbrellaQuestionRanges.length) {
    for (const umbrella of umbrellaQuestionRanges) {
      questionGroupCandidates.push({
        groupId: `group-${questionGroupCandidates.length + 1}`,
        heading: umbrella.heading,
        questionRange: umbrella.questionRange,
        instructionText: umbrella.text,
        blockIds: umbrella.blockId ? [umbrella.blockId] : [],
        kindHint: "short_answer",
        confidence: 0.35,
        isUmbrellaRange: true,
        requiresManualQuestionImport: true
      });
    }
  } else if (!questionGroupCandidates.length && questionBlocks.length) {
    const start = answerNumbers[0] ?? 1;
    const end = answerNumbers.at(-1) ?? start;
    questionGroupCandidates.push({
      groupId: "group-1",
      heading: questionHeading(start, end),
      questionRange: [start, end],
      instructionText: questionBlocks.map(blockText).join("\n"),
      blockIds: questionBlocks.map((block) => block.blockId),
      kindHint: classifyGroup(questionBlocks.map(blockText).join(" "), questionBlocks.map((block) => block.blockId)).kind,
      confidence: 0.58,
      classification: classifyGroup(questionBlocks.map(blockText).join(" "), questionBlocks.map((block) => block.blockId)),
      sectionEvidence: sectionEvidenceForBlocks(questionBlocks),
      continuationEdges: continuationEdgesForBlocks(questionBlocks)
    });
  }

  const fallbackPassageRange = firstQuestionIndex > 0 ? blocks.slice(0, firstQuestionIndex).map((block) => block.blockId) : blocks.slice(0, Math.max(1, Math.min(3, blocks.length))).map((block) => block.blockId);
  const passageRange = passageBlocks.length ? passageBlocks.map((block) => block.blockId) : fallbackPassageRange;
  const issues = [
    ...(questionGroupCandidates.length ? [] : ["未识别到题号范围，请手动切分。"]),
    ...(questionGroupCandidates.some((candidate) => candidate.requiresManualQuestionImport) ? ["仅识别到总题号范围，请导入或手动填写每道题题干。"] : []),
    ...(Object.keys(answerMap).length ? [] : ["未识别到答案，请手动填写。"]),
    ...(firstAnswerIndex >= 0 && firstQuestionIndex >= 0 && firstAnswerIndex < firstQuestionIndex ? ["答案内容出现在题目前，请确认切分顺序。"] : [])
  ];

  return {
    jobId,
    passageCandidates: [{ range: passageRange, title: inferPassageTitle(job ?? { title: "Untitled Reading" } as ImportJob, passageBlocks), categoryHint: job?.category ?? "P1" }],
    questionGroupCandidates,
    umbrellaQuestionRanges,
    answerKeyCandidates: [
      ...(Object.keys(answerMap).length ? [{ source: answerBlocks.map((block) => block.blockId).join(",") || "manual", answers: answerMap }] : []),
      ...externalAnswerCandidates
    ],
    issues
  };
}

function makeStaticSplit(jobId: string): SplitCandidates {
  return {
    jobId,
    passageCandidates: [{ range: ["b001", "b002", "b003"], title: "The Rise and Fall of Detective Stories", categoryHint: "P1" }],
    questionGroupCandidates: [
      {
        groupId: "group-1",
        heading: "Questions 1-5",
        questionRange: [1, 5],
        instructionText: "Do the following statements agree with the information given in Reading Passage 1?",
        blockIds: ["b004", "b005"],
        kindHint: "true_false_not_given",
        confidence: 0.88
      },
      {
        groupId: "group-2",
        heading: "Questions 6-8",
        questionRange: [6, 8],
        instructionText: "Complete the table below. Choose ONE WORD ONLY from the passage for each answer.",
        blockIds: ["b006", "b007"],
        kindHint: "table_completion",
        confidence: 0.84
      }
    ],
    answerKeyCandidates: [
      {
        source: "local-answer-block:b008",
        answers: {
          "1": "FALSE",
          "2": "TRUE",
          "3": "NOT GIVEN",
          "4": "TRUE",
          "5": "FALSE",
          "6": "clues",
          "7": "alibis",
          "8": "narrators"
        }
      }
    ],
    issues: []
  };
}

function interactionForKind(kind: GroupKind) {
  if (kind === "true_false_not_given") return { type: "radio" as const, options: ["TRUE", "FALSE", "NOT GIVEN"] };
  if (kind === "yes_no_not_given") return { type: "radio" as const, options: ["YES", "NO", "NOT GIVEN"] };
  if (kind === "single_choice") return { type: "radio" as const, options: ["A", "B", "C", "D"] };
  if (kind === "multi_choice") return { type: "checkbox" as const, options: ["A", "B", "C", "D", "E", "F"], minSelections: 2, maxSelections: 2 };
  if (kind === "heading_matching" || kind === "matching" || kind === "matching_information" || kind === "classification") return { type: "matching" as const, options: ["A", "B", "C", "D"], allowOptionReuse: kind === "classification" || kind === "matching_information" };
  return { type: "text" as const, placeholder: "answer" };
}

function templateForKind(kind: GroupKind): string {
  const mapping: Partial<Record<GroupKind, string>> = {
    true_false_not_given: "tfng_list",
    yes_no_not_given: "ynng_list",
    single_choice: "single_choice_list",
    multi_choice: "multi_choice_checkbox",
    heading_matching: "heading_matching",
    matching: "matching_list",
    matching_information: "matching_information",
    classification: "classification",
    table_completion: "table_completion",
    summary_completion: "summary_text_completion",
    sentence_completion: "inline_text_completion",
    short_answer: "short_answer_list"
  };
  return mapping[kind] ?? "short_answer_list";
}

function promptForQuestion(groupText: string, number: number, fallbackHeading: string, rangeEnd: number): string {
  const normalized = groupText.replace(/\s+/g, " ").trim();
  const escaped = String(number).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const next = String(number + 1).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const nextBoundary = number < rangeEnd ? `\\s+${next}[).]?\\s+` : "\\s+(?:Questions?\\s+\\d|Answers?|Answer\\s+Key)\\b|$";
  const match = normalized.match(new RegExp(`(?:^|\\s)${escaped}[).]?\\s+(.+?)(?=${nextBoundary})`, "i"));
  if (match?.[1]) return match[1].replace(/^Questions?\s+\d+(?:\s*[-–—]\s*\d+)?\s*/i, "").trim();
  return `${fallbackHeading} item ${number}`;
}

function makeAuthoring(job: ImportJob, split: SplitCandidates, doc?: DocumentIr): ReadingAuthoringIr {
  const examId = `${(job.category ?? "P1").toLowerCase()}-${job.frequency ?? "medium"}-${job.jobId.split("-").at(-1) ?? "001"}`;
  const blocksById = new Map(flattenBlocks(doc).map((block) => [block.blockId, block]));
  const answerByDisplay = Object.assign({}, ...split.answerKeyCandidates.map((candidate) => candidate.answers));
  const passageBlocks = split.passageCandidates[0]?.range.map((blockId) => blocksById.get(blockId)).filter(Boolean) as DocumentBlock[];
  const passageHtml = passageBlocks.length ? passageBlocks.map((block) => block.html ?? `<p>${escapeHtml(block.text ?? "")}</p>`).join("\n") : `<h2>${escapeHtml(split.passageCandidates[0]?.title ?? job.title)}</h2>`;
  const groups: ReadingAuthoringIr["groups"] = split.questionGroupCandidates.map((candidate, index) => {
    const kind = candidate.kindHint ?? "short_answer";
    const requiresManualQuestionImport = candidate.requiresManualQuestionImport === true;
    const groupBlocks = candidate.blockIds.map((blockId) => blocksById.get(blockId)).filter(Boolean) as DocumentBlock[];
    const groupText = groupBlocks.map(blockText).join(" ") || candidate.instructionText;
    const [start, end] = candidate.questionRange;
    const questions = Array.from({ length: Math.max(0, end - start + 1) }, (_, offset) => start + offset).map((number) => {
      const displayNumber = String(number);
      const idValue = `q${displayNumber}`;
      return {
        id: idValue,
        displayNumber,
        prompt: requiresManualQuestionImport ? `Manual import required for question ${number}` : promptForQuestion(groupText, number, candidate.heading, end),
        interaction: candidate.classification?.interaction ?? interactionForKind(kind),
        answer: answerByDisplay[displayNumber],
        sourceBlockIds: candidate.blockIds,
        confidence: candidate.confidence,
        verified: false,
        requiresManualQuestionImport
      };
    });
    return {
      groupId: candidate.groupId || `group-${index + 1}`,
      kind,
      questionRange: candidate.questionRange,
      instruction: [candidate.instructionText],
      questions,
      layout: { template: templateForKind(kind), ...(kind === "table_completion" ? { tableHeaders: ["Question", "Prompt", "Answer"] } : {}) },
      reviewWarnings: candidate.classification?.warnings ?? [],
      classificationEvidence: candidate.classification?.evidence ?? candidate.blockIds,
      sectionEvidence: candidate.sectionEvidence ?? [],
      continuationEdges: candidate.continuationEdges ?? [],
      allowOptionReuse: candidate.classification?.interaction.allowOptionReuse,
      sourceBlockIds: candidate.blockIds,
      confidence: candidate.confidence,
      verified: false,
      isUmbrellaRange: candidate.isUmbrellaRange,
      requiresManualQuestionImport
    };
  });

  const answerKey = Object.fromEntries(groups.flatMap((group) => group.questions.map((question) => [question.id, question.answer ?? ""])));
  const questionOrder = groups.flatMap((group) => group.questions.map((question) => question.id));
  const questionDisplayMap = Object.fromEntries(groups.flatMap((group) => group.questions.map((question) => [question.id, question.displayNumber])));

  return {
    schemaVersion: "ReadingAuthoringIRV1",
    jobId: job.jobId,
    exam: {
      examId,
      title: job.title,
      category: job.category ?? "P1",
      frequency: job.frequency ?? "medium",
      tags: job.tags,
      sourceFiles: job.sourceFiles
    },
    passage: {
      title: split.passageCandidates[0]?.title ?? job.title,
      htmlBlocks: [{ blockId: "passage-main", html: passageHtml }],
      sourceBlockIds: split.passageCandidates[0]?.range ?? [],
      questionUmbrellaRanges: split.umbrellaQuestionRanges ?? []
    },
    groups,
    answerKey,
    questionOrder,
    questionDisplayMap,
    audit: {
      llmUsed: false,
      humanVerified: false,
      issues: split.issues,
      revision: 1,
      updatedAt: now()
    }
  };
}

function validateIr(jobId: string, ir: ReadingAuthoringIr | undefined): ValidationReport {
  const issues: ValidationIssue[] = [];
  const add = (layer: ValidationIssue["layer"], path: string, message: string, fixHint?: string) => {
    issues.push({ issueId: id("issue"), severity: "error", layer, path, message, fixHint });
  };

  if (!ir) {
    add("AuthoringIR", "$", "可编辑题稿尚未生成。", "请先完成粗切并生成可编辑题稿。");
  } else {
    if (!ir.exam.examId) add("AuthoringIR", "$.exam.examId", "缺少题目编号。");
    if (!ir.passage.htmlBlocks.length) add("AuthoringIR", "$.passage.htmlBlocks", "文章正文不能为空。");
    if (!ir.groups.length) add("AuthoringIR", "$.groups", "至少需要一个题组。");
    for (const group of ir.groups) {
      if (group.requiresManualQuestionImport && !group.verified) {
        add("AuthoringIR", `$.groups.${group.groupId}.questions`, "仅有总题号范围，发布前需要手动补齐具体题干。");
      }
      for (const question of group.questions) {
        if (!question.interaction?.type) add("AuthoringIR", `$.groups.${group.groupId}.${question.id}`, "题目缺少作答方式。");
        if (!question.answer || (Array.isArray(question.answer) && !question.answer.length)) {
          add("AuthoringIR", `$.answerKey.${question.id}`, "每道题导出前都需要答案。");
        }
        if (question.requiresManualQuestionImport && !question.verified) {
          add("AuthoringIR", `$.groups.${group.groupId}.${question.id}.prompt`, "发布前需要从源文档补齐题干并确认。");
        }
      }
    }

    const source = toReadingExamSource(ir);
    if (source.schemaVersion !== "ReadingExamSourceV1") add("ReadingExamSourceV1", "$.schemaVersion", "导出数据版本不正确。");
    if (!source.answerKey || !Object.keys(source.answerKey).length) add("ReadingExamSourceV1", "$.answerKey", "答案不能为空。");

    for (const group of source.questionGroups) {
      for (const qid of group.questionIds) {
        const hasNamedControl = new RegExp(`name=["']${qid}["']|data-question=["']${qid}["']|data-question-id=["']${qid}["']`).test(group.bodyHtml);
        if (!hasNamedControl) {
          add("DomProtocol", `$.questionGroups.${group.groupId}.bodyHtml`, `题目 ${qid} 缺少可填写或可选择的答题控件。`);
        }
      }
    }
  }

  const layerNames: ValidationIssue["layer"][] = ["AuthoringIR", "ReadingExamSourceV1", "DomProtocol", "RuntimePreview"];
  const layers = layerNames.map((layer) => ({
    layer,
    issueCount: issues.filter((issue) => issue.layer === layer).length,
    passed: issues.every((issue) => issue.layer !== layer)
  }));

  return { jobId, passed: issues.length === 0, layers, issues, generatedAt: now() };
}

function answerIsEmpty(answer: AnswerValue | undefined): boolean {
  return answer == null || (Array.isArray(answer) ? answer.length === 0 || answer.every((item) => !String(item).trim()) : !String(answer).trim());
}

function refreshReviewState(ir: ReadingAuthoringIr): { ir: ReadingAuthoringIr; needsReview: number } {
  let needsReview = 0;
  let total = 0;
  let verified = 0;
  let emptyAnswers = 0;
  const groups = ir.groups.map((group) => {
    let groupTotal = 0;
    let groupVerified = 0;
    const questions = group.questions.map((question) => {
      total += 1;
      groupTotal += 1;
      if (question.verified) {
        verified += 1;
        groupVerified += 1;
      }
      if (answerIsEmpty(question.answer)) {
        emptyAnswers += 1;
        needsReview += 1;
      }
      if (question.confidence < 0.85 && !question.verified) needsReview += 1;
      return question;
    });
    const nextVerified = groupTotal > 0 && groupTotal === groupVerified;
    if (group.confidence < 0.85 && !nextVerified) needsReview += 1;
    if (group.reviewWarnings?.length && !nextVerified) needsReview += 1;
    if (group.requiresManualQuestionImport && !nextVerified) needsReview += 1;
    return { ...group, questions, verified: nextVerified };
  });

  return {
    ir: {
      ...refreshAuthoringDerivedFields({ ...ir, groups }),
      audit: {
        ...ir.audit,
        humanVerified: total > 0 && total === verified && emptyAnswers === 0,
        updatedAt: now()
      }
    },
    needsReview
  };
}

function publishReadinessReport(store: Store, jobId: string, ir: ReadingAuthoringIr, report: ValidationReport): ValidationReport {
  const job = requireJob(store, jobId);
  const issues: ValidationIssue[] = [...report.issues];
  const add = (path: string, message: string) => issues.push({ issueId: id("issue"), severity: "error", layer: "AuthoringIR", path, message });
  const humanVerified = ir.audit.humanVerified === true;
  if (job.status === "NeedsReview") add("$.job.status", "任务仍需审核；请完成所有人工确认后再发布。");
  issues.push(...sourceReviewIssues(sourceReviewStatus(store, jobId)));
  if (!humanVerified) {
    add("$.audit.humanVerified", "所有题目和答案都需要人工确认后才能发布。");
  }
  for (const group of ir.groups) {
    if (group.confidence < 0.85 && !group.verified) add(`$.groups.${group.groupId}.verified`, "低置信题组需要人工确认后才能发布。");
    for (const warning of group.reviewWarnings ?? []) {
      if (!group.verified) add(`$.groups.${group.groupId}.reviewWarnings`, `题型或选项规则需要人工确认：${warning}`);
    }
    if (group.requiresManualQuestionImport && !group.verified) add(`$.groups.${group.groupId}.questions`, "仅有总题号范围，发布前需要手动补齐具体题干。");
    for (const question of group.questions) {
      if (answerIsEmpty(question.answer)) add(`$.answerKey.${question.id}`, "题目答案为空；请填写或确认答案后再发布。");
      if (question.confidence < 0.85 && !question.verified) add(`$.groups.${group.groupId}.questions.${question.id}.verified`, "低置信题目需要人工确认后才能发布。");
      if (question.requiresManualQuestionImport && !question.verified) add(`$.groups.${group.groupId}.questions.${question.id}.prompt`, "发布前需要从源文档补齐题干并确认。");
    }
  }
  const layerNames: ValidationIssue["layer"][] = ["AuthoringIR", "ReadingExamSourceV1", "DomProtocol", "RuntimePreview"];
  return {
    ...report,
    passed: issues.length === 0,
    issues,
    layers: layerNames.map((layer) => ({
      layer,
      issueCount: issues.filter((issue) => issue.layer === layer).length,
      passed: issues.every((issue) => issue.layer !== layer)
    })),
    generatedAt: now()
  };
}

function applySuggestionPatch(ir: ReadingAuthoringIr, suggestion: LlmSuggestion, selectedPaths: string[]): ReadingAuthoringIr {
  const selected = new Set(selectedPaths);
  const patches = Array.isArray(suggestion.patch) ? suggestion.patch as Array<{ op?: string; path?: string; value?: unknown }> : [];
  return {
    ...ir,
    groups: ir.groups.map((group) => {
      if (group.groupId !== suggestion.groupId) return group;
      let next = { ...group };
      for (const patch of patches) {
        if (patch.op !== "replace") continue;
        if (patch.path === "/kind" && selected.has("kind")) next = { ...next, kind: patch.value as GroupKind };
        if (patch.path === "/layout/template" && (selected.has("layout") || selected.has("kind"))) next = { ...next, layout: { ...next.layout, template: String(patch.value) } };
      }
      if (selected.has("questions") && Array.isArray(suggestion.questions)) {
        const suggestionsById = new Map(suggestion.questions.map((item) => {
          const question = item as { id?: string; prompt?: string; interaction?: ReadingAuthoringIr["groups"][number]["questions"][number]["interaction"] };
          return [question.id, question] as const;
        }));
        next = {
          ...next,
          questions: next.questions.map((question) => {
            const patch = suggestionsById.get(question.id);
            if (!patch) return question;
            return { ...question, prompt: patch.prompt ?? question.prompt, interaction: patch.interaction ?? question.interaction };
          })
        };
      }
      return next;
    })
  };
}

function suggestionAutoApplyIssues(ir: ReadingAuthoringIr, suggestion: LlmSuggestion, selectedPaths: string[]): string[] {
  const issues: string[] = [];
  if (suggestion.confidence < 0.85) issues.push("置信度低于自动应用阈值。");
  const group = ir.groups.find((item) => item.groupId === suggestion.groupId);
  if (!group) return [...issues, "未找到对应题组。"];
  const allowed = new Set(["kind", "layout", "questions"]);
  for (const path of selectedPaths) if (!allowed.has(path)) issues.push(`不支持自动修改：${path}`);
  const evidence = (suggestion.evidence ?? {}) as { fallback?: boolean; source?: string; sourceBlockIds?: string[]; blockIds?: string[]; quotes?: Array<{ blockId?: string; text?: string }> };
  if (evidence.fallback) issues.push("本地兜底建议不能自动应用。");
  if ((evidence.source ?? "").includes("fallback") || (evidence.source ?? "").includes("heuristic")) issues.push(`建议来源不符合自动应用要求：${evidence.source}`);
  const groupBlockIds = new Set(group.sourceBlockIds);
  const evidenceBlockIds = evidence.sourceBlockIds ?? evidence.blockIds ?? [];
  if (!evidenceBlockIds.length) issues.push("缺少可追溯的来源段落。");
  for (const blockId of evidenceBlockIds) if (!groupBlockIds.has(blockId)) issues.push(`引用的来源段落不属于当前题组：${blockId}`);
  if (!evidence.quotes?.length) issues.push("缺少来源摘录。");
  for (const quote of evidence.quotes ?? []) {
    if (!quote.blockId || !groupBlockIds.has(quote.blockId)) issues.push(`来源摘录不属于当前题组：${quote.blockId ?? ""}`);
    if (!quote.text?.trim()) issues.push(`来源摘录缺少文字：${quote.blockId ?? ""}`);
  }
  return [...new Set(issues)].sort();
}

function refreshAuthoringDerivedFields(ir: ReadingAuthoringIr): ReadingAuthoringIr {
  return {
    ...ir,
    answerKey: Object.fromEntries(ir.groups.flatMap((group) => group.questions.map((question) => [question.id, question.answer ?? ""]))),
    questionOrder: ir.groups.flatMap((group) => group.questions.map((question) => question.id)),
    questionDisplayMap: Object.fromEntries(ir.groups.flatMap((group) => group.questions.map((question) => [question.id, question.displayNumber])))
  };
}

function previewHtml(source: ReturnType<typeof toReadingExamSource>): string {
  return `<!doctype html><html><head><meta charset="utf-8"><style>
    body{font-family:Georgia,serif;margin:0;padding:24px;color:#15211f;background:#f5f1e8;line-height:1.6}.layout{display:grid;grid-template-columns:minmax(0,1fr) minmax(360px,.8fr);gap:32px}.passage,.questions{background:#fffaf0;border:1px solid #d8cfbf;padding:22px}.choice-row{display:flex;gap:10px;flex-wrap:wrap}.completion-table{width:100%;border-collapse:collapse}.completion-table th,.completion-table td{border:1px solid #c8beaa;padding:8px}.question-umbrella-ranges{padding-left:18px;color:#5d4630}input{font:inherit;padding:6px;border:1px solid #9aa391}
  </style></head><body><div class="layout"><article class="passage">${source.passage.blocks
    .map((block) => block.html)
    .join("")}</article><section class="questions">${source.meta.questionIntroHtml}${source.questionGroups.map((group) => group.bodyHtml).join("")}</section></div></body></html>`;
}

function normalizeAnswer(value: AnswerValue | undefined): string {
  if (Array.isArray(value)) return [...value].map((item) => normalizeAnswer(item)).sort().join("|");
  return String(value ?? "").trim().toLowerCase().replace(/\s+/g, " ");
}

function attrs(tag: string): Record<string, string> {
  const result: Record<string, string> = {};
  const pattern = /([:\w-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(tag))) {
    const [, key, doubleQuoted, singleQuoted, bare] = match;
    if (!key || key === tag.split(/\s+/)[0].replace("<", "")) continue;
    result[key.toLowerCase()] = doubleQuoted ?? singleQuoted ?? bare ?? "";
  }
  return result;
}

function tags(html: string, name: string): string[] {
  return [...html.matchAll(new RegExp(`<${name}\\b[^>]*>`, "gi"))].map((match) => match[0]);
}

function controlQuestionId(attributes: Record<string, string>): string | undefined {
  return (
    attributes.name ||
    attributes["data-question"] ||
    attributes["data-question-id"] ||
    attributes["data-target"] ||
    (attributes.id?.endsWith("_input") ? attributes.id.slice(0, -6) : attributes.id)
  );
}

function controlsFor(html: string, qid: string): Array<Record<string, string>> {
  const controlTags = [
    ...tags(html, "input"),
    ...tags(html, "select"),
    ...tags(html, "textarea"),
    ...[...html.matchAll(/<[^>]*\b(?:paragraph-dropzone|match-dropzone|drop-target-summary)\b[^>]*>/gi)].map((match) => match[0])
  ];
  return controlTags.map(attrs).filter((attributes) => controlQuestionId(attributes) === qid);
}

function score(source: ReturnType<typeof toReadingExamSource>, collected: Record<string, AnswerValue>): { total: number; correct: number; percent: number } {
  const total = source.questionOrder.length;
  const correct = source.questionOrder.filter((qid) => normalizeAnswer(collected[qid]) === normalizeAnswer(source.answerKey[qid])).length;
  return { total, correct, percent: total ? Math.round((correct / total) * 10000) / 100 : 0 };
}

function runtimePreviewReport(jobId: string, assets: PreviewAssets | undefined, source: ReturnType<typeof toReadingExamSource>): ValidationReport {
  const issues: ValidationIssue[] = [];
  const add = (path: string, message: string, fixHint?: string) => {
    issues.push({ issueId: id("issue"), severity: "error", layer: "RuntimePreview", path, message, fixHint });
  };

  if (!assets) {
    add("preview-assets", "请先生成预览，再运行预览检查。");
  } else {
    const registry = new Map<string, unknown>();
    const runtime = {
      __READING_EXAM_DATA__: {
        register(examId: string, registeredSource: unknown) {
          registry.set(examId, registeredSource);
        }
      },
      __READING_EXAM_MANIFEST__: undefined as unknown
    };
    try {
      new Function("window", "globalThis", assets.manifestJs)(runtime, runtime);
      new Function("window", "globalThis", assets.wrapperJs)(runtime, runtime);
    } catch (error) {
      add("runtime.execution", `预览脚本运行失败：${error instanceof Error ? error.message : String(error)}`);
    }
    if (!registry.has(source.examId)) add(`${source.examId}.js`, `导出脚本未注册题目 ${source.examId}。`);
    const manifest = runtime.__READING_EXAM_MANIFEST__ as Record<string, unknown> | undefined;
    if (!manifest?.[source.examId]) add("manifest.js", `清单中缺少题目 ${source.examId}。`);
  }

  const collected: Record<string, AnswerValue> = {};
  for (const group of source.questionGroups) {
    for (const qid of group.questionIds) {
      const controls = controlsFor(group.bodyHtml, qid);
      if (!controls.length) {
        add(`$.questionGroups.${group.groupId}.bodyHtml`, `题目 ${qid} 缺少可填写或可拖放的答题区域。`);
        continue;
      }
      const answer = source.answerKey[qid];
      const first = controls[0];
      const type = (first.type || "text").toLowerCase();
      if (type === "radio" || type === "checkbox") {
        const values = controls.map((attributes) => normalizeAnswer(attributes.value));
        const expected = Array.isArray(answer) ? answer : [answer];
        for (const item of expected) {
          if (!values.includes(normalizeAnswer(item))) {
            add(`$.questionGroups.${group.groupId}.bodyHtml`, `题目 ${qid} 的答案不在选项中。`);
          }
        }
      }
      collected[qid] = answer;
    }
  }

  const scoreInfo = score(source, collected);
  const wrongAnswers = { ...source.answerKey };
  const firstQid = source.questionOrder[0];
  if (firstQid) wrongAnswers[firstQid] = Array.isArray(wrongAnswers[firstQid]) ? ["__wrong__"] : `${wrongAnswers[firstQid] ?? ""}__wrong__`;
  const wrongScoreInfo = score(source, wrongAnswers);
  if (scoreInfo.total > 0 && scoreInfo.percent !== 100) add("runtime.scoreInfo", `正确答案检查应为 100%，当前为 ${scoreInfo.percent}%。`);
  if (scoreInfo.total > 0 && wrongScoreInfo.percent >= scoreInfo.percent) add("runtime.scoreInfo", "错误答案样本没有降低得分。");

  return {
    jobId,
    passed: issues.length === 0,
    layers: [{ layer: "RuntimePreview", passed: issues.length === 0, issueCount: issues.length }],
    issues,
    runtime: {
      adapter: "本地基础校验",
      mode: "static-rust",
      fallbackReason: "当前为开发预览环境，已执行基础校验；真实运行预览仅作为诊断项。",
      examId: source.examId,
      jobId,
      registeredIds: assets ? [source.examId] : [],
      navButtonCount: source.questionOrder.length,
      questionCount: source.questionOrder.length,
      collectedAnswers: collected,
      scoreInfo,
      wrongScoreInfo,
      consoleErrors: []
    },
    generatedAt: now()
  };
}

function mergeValidationReports(base: ValidationReport, sidecar: ValidationReport): ValidationReport {
  const replaceLayers = new Set(sidecar.layers.map((layer) => layer.layer));
  const issues = [...base.issues.filter((issue) => !replaceLayers.has(issue.layer)), ...sidecar.issues];
  const layerNames: ValidationIssue["layer"][] = ["AuthoringIR", "ReadingExamSourceV1", "DomProtocol", "RuntimePreview"];
  return {
    ...base,
    runtime: sidecar.runtime ?? base.runtime,
    passed: issues.length === 0,
    issues,
    layers: layerNames.map((layer) => ({
      layer,
      issueCount: issues.filter((issue) => issue.layer === layer).length,
      passed: issues.every((issue) => issue.layer !== layer)
    })),
    generatedAt: now()
  };
}

function packManifest(input: BuildPackInput, sources: ReturnType<typeof toReadingExamSource>[]) {
  return {
    schemaVersion: "ReadingExamPackV1",
    packId: input.packId,
    version: input.version,
    institution: input.institution,
    description: input.description,
    validFrom: input.validFrom ?? null,
    validTo: input.validTo ?? null,
    generatedAt: now(),
    assetsRoot: "reading-exams",
    exams: sources.map((source, index) => ({
      order: index + 1,
      examId: source.examId,
      title: source.meta.title,
      category: source.meta.category,
      frequency: source.meta.frequency,
      script: `reading-exams/${source.examId}.js`
    }))
  };
}

function estimateStoredZipSize(entries: Array<{ path: string; content: string }>): number {
  // Dev fallback does not write files; this mirrors the Rust stored-zip envelope closely enough for UI smoke tests.
  return entries.reduce((total, entry) => total + entry.path.length * 2 + entry.content.length + 128, 22);
}

function cleanupDevArtifacts(store: Store, jobId: string, exportSummary: unknown): Record<string, unknown> {
  if (store.diagnostics.keepFullProcessArtifacts) {
    return {
      schemaVersion: "CleanupSummaryV1",
      jobId,
      cleaned: false,
      retainedFullProcessArtifacts: true,
      message: "Developer diagnostics retention is enabled; full process artifacts were kept.",
      exportSummary,
      generatedAt: now()
    };
  }
  delete store.documents[jobId];
  delete store.splits[jobId];
  delete store.validation[jobId];
  delete store.previews[jobId];
  delete store.suggestions[jobId];
  delete store.pipelineReports[jobId];
  updateJob(store, jobId, { status: "Cleaned", currentStep: "Export" });
  return {
    schemaVersion: "CleanupSummaryV1",
    jobId,
    cleaned: true,
    retainedFullProcessArtifacts: false,
    message: "中间文件已自动清理，已保留可编辑题目稿。",
    exportSummary,
    generatedAt: now()
  };
}

function minimizeDevProcessArtifacts(store: Store, jobId: string): void {
  if (store.diagnostics.keepFullProcessArtifacts) return;
  delete store.documents[jobId];
  delete store.splits[jobId];
  delete store.validation[jobId];
  delete store.previews[jobId];
  delete store.pipelineReports[jobId];
}

function recordGroupLlmReview(
  ir: ReadingAuthoringIr,
  groupId: string,
  status: "low_confidence" | "auto_apply_blocked",
  confidence: number,
  warning: string,
  suggestion: LlmSuggestion
): ReadingAuthoringIr {
  return {
    ...ir,
    groups: ir.groups.map((group) => group.groupId === groupId ? {
      ...group,
      reviewWarnings: Array.from(new Set([...(group.reviewWarnings ?? []), warning])),
      llmReview: {
        required: true,
        status,
        confidence,
        suggestionId: suggestion.suggestionId,
        suggestedKind: suggestion.kind ?? null,
        warnings: suggestion.warnings,
        evidence: suggestion.evidence,
        recordedAt: now()
      }
    } : group)
  };
}

export async function devFallbackInvoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  const store = load();

  switch (command) {
    case "create_import_job": {
      const input = (args.input ?? {}) as { title?: string; category?: ImportJob["category"]; frequency?: ImportJob["frequency"]; tags?: string[]; llmProfileId?: string };
      const job: ImportJob = {
        jobId: id("import"),
        title: input.title?.trim() || "Untitled Reading",
        status: "Working",
        category: input.category ?? "P1",
        frequency: input.frequency ?? "medium",
        tags: input.tags ?? [],
        sourceFiles: [],
        activeLlmProfileId: input.llmProfileId,
        createdAt: now(),
        updatedAt: now(),
        currentStep: "Upload",
        issueCounts: { errors: 0, warnings: 0, needsReview: 0 }
      };
      store.jobs.unshift(job);
      save(store);
      return job as T;
    }

    case "list_jobs": {
      const filter = (args.filter ?? {}) as JobFilter;
      let jobs = store.jobs;
      if (filter.status) jobs = jobs.filter((job) => job.status === filter.status);
      if (filter.search) jobs = jobs.filter((job) => job.title.toLowerCase().includes(filter.search!.toLowerCase()));
      return jobs as T;
    }

	    case "get_job": {
	      const jobId = args.jobId as string;
	      const detail: JobDetail = {
	        job: requireJob(store, jobId),
	        documentIr: store.documents[jobId],
	        sourceReview: sourceReviewStatus(store, jobId),
	        splitCandidates: store.splits[jobId],
        authoringIr: store.authoring[jobId],
        validationReport: store.validation[jobId],
        previewAssets: store.previews[jobId],
        pipelineReport: store.pipelineReports[jobId],
        llmSuggestions: store.suggestions[jobId] ?? []
      };
      return detail as T;
    }

    case "update_job_meta": {
      const { status: _status, currentStep: _currentStep, ...patch } = args.patch as JobMetaPatch & Partial<ImportJob>;
      const job = updateJob(store, args.jobId as string, patch);
      save(store);
      return job as T;
    }

    case "delete_job": {
      const jobId = args.jobId as string;
      store.jobs = store.jobs.filter((job) => job.jobId !== jobId);
      delete store.documents[jobId];
      delete store.splits[jobId];
      delete store.authoring[jobId];
      delete store.validation[jobId];
      delete store.previews[jobId];
      save(store);
      return undefined as T;
    }

    case "import_source_file": {
      const jobId = args.jobId as string;
      const filePath = (args.filePath as string) || "source.pdf";
      const source: SourceFile = {
        fileId: id("file"),
        originalName: filePath.split(/[\\/]/).pop() || filePath,
        storedName: `${Math.random().toString(36).slice(2, 8)}-${filePath.split(/[\\/]/).pop() || "source.pdf"}`,
        fileType: detectFileType(filePath),
        sha256: Math.random().toString(16).slice(2).padEnd(64, "0"),
        sizeBytes: Number(args.sizeBytes ?? 0),
        role: (args.role as SourceFileRole) ?? "MainQuestion",
        importedAt: now()
      };
      const job = requireJob(store, jobId);
      updateJob(store, jobId, { sourceFiles: [...job.sourceFiles, source], status: "Working", currentStep: "DocumentReview" });
      save(store);
      return source as T;
    }

    case "parse_document": {
      const jobId = args.jobId as string;
      const job = requireJob(store, jobId);
      const ir = makeDocumentIr(job, (args.options ?? { mode: "auto" }) as ParseOptions);
      store.documents[jobId] = ir;
      delete store.sourceReviews[jobId];
      const review = sourceReviewStatus(store, jobId);
      updateJob(store, jobId, {
        status: review.required ? "NeedsReview" : "Working",
        currentStep: "DocumentReview",
        issueCounts: { ...requireJob(store, jobId).issueCounts, needsReview: sourceReviewIssues(review).length }
      });
      save(store);
      return ir as T;
    }

	    case "rerun_ocr": {
	      const jobId = args.jobId as string;
	      const job = requireJob(store, jobId);
	      const ir = makeDocumentIr(job, { mode: "ocr" });
	      store.documents[jobId] = ir;
	      delete store.sourceReviews[jobId];
	      save(store);
	      return ir as T;
	    }

    case "apply_manual_transcription": {
      const jobId = args.jobId as string;
      const job = requireJob(store, jobId);
      const input = args.input as { text?: string; note?: string };
      const text = input?.text?.trim() ?? "";
      if (!text) throw new Error("manual_transcription_text_required");
      const ir = makeManualDocumentIr(job, text);
      store.documents[jobId] = ir;
      const review = sourceReviewStatus(store, jobId);
      store.sourceReviews[jobId] = { ...review, resolved: true, stale: false, resolvedAt: now(), note: input.note ?? "manual transcription applied" };
      updateJob(store, jobId, {
        status: "Working",
        currentStep: "DocumentReview",
        issueCounts: { ...requireJob(store, jobId).issueCounts, needsReview: 0 }
      });
      save(store);
      return ir as T;
    }

    case "apply_vision_transcription": {
      const jobId = args.jobId as string;
      const job = requireJob(store, jobId);
      const ir = makeVisionDocumentIr(job);
      store.documents[jobId] = ir;
      const review = sourceReviewStatus(store, jobId);
      store.sourceReviews[jobId] = { ...review, resolved: false, stale: false, resolvedAt: null, note: null };
      updateJob(store, jobId, {
        status: "NeedsReview",
        currentStep: "DocumentReview",
        issueCounts: { ...requireJob(store, jobId).issueCounts, needsReview: sourceReviewIssues(review).length }
      });
      save(store);
      return ir as T;
    }

	    case "resolve_source_review": {
	      const jobId = args.jobId as string;
	      const review = sourceReviewStatus(store, jobId);
	      const resolved: SourceReview = { ...review, resolved: true, stale: false, resolvedAt: now(), note: (args.note as string | undefined) ?? null };
	      store.sourceReviews[jobId] = resolved;
	      const authoringReviewCount = store.authoring[jobId] ? refreshReviewState(store.authoring[jobId]).needsReview : 0;
	      updateJob(store, jobId, {
	        status: authoringReviewCount ? "NeedsReview" : store.authoring[jobId] ? "DraftSaved" : "Working",
	        currentStep: authoringReviewCount ? "Authoring" : "DocumentReview",
	        issueCounts: { ...requireJob(store, jobId).issueCounts, needsReview: authoringReviewCount }
	      });
	      save(store);
	      return resolved as T;
	    }

    case "run_rule_split": {
      const jobId = args.jobId as string;
      requireJob(store, jobId);
      const split = makeSplit(jobId, store.documents[jobId], requireJob(store, jobId));
      store.splits[jobId] = split;
      updateJob(store, jobId, { status: "Working", currentStep: "Split" });
      save(store);
      return split as T;
    }

    case "save_split_adjustments": {
      const jobId = args.jobId as string;
      const split = args.patch as SplitCandidates;
      store.splits[jobId] = split;
      save(store);
      return split as T;
    }

    case "build_authoring_ir": {
      const jobId = args.jobId as string;
      const job = requireJob(store, jobId);
      const split = store.splits[jobId] ?? makeSplit(jobId, store.documents[jobId], job);
      const ir = makeAuthoring(job, split, store.documents[jobId]);
      store.splits[jobId] = split;
      store.authoring[jobId] = ir;
      const authoringReview = refreshReviewState(ir);
      const sourceReviewIssueCount = sourceReviewIssues(sourceReviewStatus(store, jobId)).length;
      updateJob(store, jobId, {
        status: authoringReview.needsReview || sourceReviewIssueCount ? "NeedsReview" : "DraftSaved",
        currentStep: "Authoring",
        issueCounts: { errors: 0, warnings: 1, needsReview: authoringReview.needsReview + sourceReviewIssueCount }
      });
      minimizeDevProcessArtifacts(store, jobId);
      save(store);
      return ir as T;
    }

    case "run_auto_pipeline": {
      const jobId = args.jobId as string;
      const input = (args.input ?? {}) as { profileId?: string; confidenceThreshold?: number; parseMode?: ParseOptions["mode"] };
      const threshold = Math.min(1, Math.max(0, input.confidenceThreshold ?? 0.85));
      let job = requireJob(store, jobId);

      let documentIr = store.documents[jobId] ?? makeDocumentIr(job, { mode: input.parseMode ?? "auto" });
      const visionTranscription = { attempted: false, applied: false, profileId: input.profileId ?? store.profiles.find((profile) => profile.enabled)?.profileId ?? "profile-local-placeholder", warnings: [] as string[], failure: null as string | null, confidence: undefined as number | undefined };
      if (
        documentIr.parser.mode === "ocr" &&
        documentIr.parser.provider !== "vision-llm-transcription" &&
        documentIr.parser.warnings.length
      ) {
        visionTranscription.attempted = true;
        documentIr = makeVisionDocumentIr(job);
        visionTranscription.applied = true;
        visionTranscription.confidence = 0.72;
        visionTranscription.warnings = documentIr.parser.warnings;
      }
      store.documents[jobId] = documentIr;
      job = updateJob(store, jobId, { status: "Working", currentStep: "DocumentReview" });

      const split = makeSplit(jobId, documentIr, job);
      store.splits[jobId] = split;
      job = updateJob(store, jobId, { status: "Working", currentStep: "Split" });

      let ir = makeAuthoring(job, split, documentIr);
      store.authoring[jobId] = ir;
      updateJob(store, jobId, { status: "DraftSaved", currentStep: "Authoring" });

      const lowConfidenceGroups: string[] = [];
      const blockedAutoApplyGroups: string[] = [];
      const highConfidenceAppliedGroups: string[] = [];
      const failures: string[] = [];
      let suggestionCount = 0;
      let appliedCount = 0;
      const profileId = input.profileId ?? store.profiles.find((profile) => profile.enabled)?.profileId ?? "profile-local-placeholder";

      for (const group of ir.groups) {
        const suggestion: LlmSuggestion = {
          suggestionId: id("suggestion"),
          jobId,
          groupId: group.groupId,
          kind: group.kind,
          confidence: 0.64,
          patch: [
            { op: "replace", path: "/kind", value: group.kind },
            { op: "replace", path: "/layout/template", value: group.layout.template }
          ],
          questions: group.questions.map((question) => ({ id: question.id, prompt: question.prompt, interaction: question.interaction })),
          evidence: { source: "dev-fallback-auto-pipeline", directJsGeneration: false, fallback: true },
          warnings: ["deterministic-local-fallback", "low-confidence-review-required", "fallback-output-never-auto-applies"],
          createdAt: now()
        };
        suggestionCount += 1;
        store.suggestions[jobId] = [suggestion, ...(store.suggestions[jobId] ?? [])];

        if (suggestion.confidence >= threshold) {
          const autoApplyIssues = suggestionAutoApplyIssues(ir, suggestion, ["kind", "layout", "questions"]);
          if (autoApplyIssues.length) {
            blockedAutoApplyGroups.push(group.groupId);
            const warning = `LLM suggestion reached confidence threshold but was not safe to auto-apply: ${autoApplyIssues.join(",")}`;
            ir = recordGroupLlmReview(ir, group.groupId, "auto_apply_blocked", suggestion.confidence, warning, suggestion);
            failures.push(`${group.groupId}:auto_apply_blocked:${autoApplyIssues.join(",")}`);
            continue;
          }
          ir = refreshAuthoringDerivedFields(applySuggestionPatch(ir, suggestion, ["kind", "layout", "questions"]));
          appliedCount += 1;
          highConfidenceAppliedGroups.push(group.groupId);
        } else {
          const warning = `LLM suggestion confidence ${suggestion.confidence.toFixed(2)} is below auto-apply threshold ${threshold.toFixed(2)}; manual review is required.`;
          ir = recordGroupLlmReview(ir, group.groupId, "low_confidence", suggestion.confidence, warning, suggestion);
          lowConfidenceGroups.push(group.groupId);
        }
      }

      ir = {
        ...refreshAuthoringDerivedFields(ir),
        audit: { ...ir.audit, llmUsed: suggestionCount > 0, updatedAt: now(), revision: ir.audit.revision + 1 }
      };
      const authoringReview = refreshReviewState(ir);
      ir = authoringReview.ir;
      store.authoring[jobId] = ir;

      const source = toReadingExamSource(ir);
      const assets: PreviewAssets = {
        examId: source.examId,
        manifestPath: `local://${jobId}/preview/manifest.js`,
        scriptPath: `local://${jobId}/preview/${source.examId}.js`,
        previewUrl: `local-preview://${source.examId}`,
        source,
        wrapperJs: buildWrapper(source),
        manifestJs: buildManifest([source])
      };
      store.previews[jobId] = assets;

      const validationReport = mergeValidationReports(validateIr(jobId, ir), runtimePreviewReport(jobId, assets, source));
      store.validation[jobId] = validationReport;

      const review = sourceReviewStatus(store, jobId);
      store.sourceReviews[jobId] = review;
      const warnings = review.parserWarnings;
      const lowConfidenceBlocks = review.lowConfidenceBlocks;
      const sourceReviewIssueCount = sourceReviewIssues(review).length;
      const requiresParserReview = sourceReviewIssueCount > 0;
      const requiresAuthoringReview = authoringReview.needsReview > 0;
      const staticRuntimePassed = validationReport.passed && validationReport.runtime?.mode === "static-rust";
      const status = lowConfidenceGroups.length || blockedAutoApplyGroups.length || requiresParserReview || requiresAuthoringReview ? "NeedsReview" : staticRuntimePassed ? "ExportReady" : validationReport.passed ? "DraftSaved" : "NeedsReview";
      const currentStep = requiresParserReview ? "DocumentReview" : lowConfidenceGroups.length || blockedAutoApplyGroups.length ? "LlmReview" : requiresAuthoringReview ? "Authoring" : staticRuntimePassed ? "Export" : "Preview";
      updateJob(store, jobId, {
        status,
        currentStep,
        issueCounts: {
          errors: validationReport.issues.filter((issue) => issue.severity === "error").length,
          warnings: validationReport.issues.filter((issue) => issue.severity === "warning").length,
          needsReview: lowConfidenceGroups.length + blockedAutoApplyGroups.length + sourceReviewIssueCount + authoringReview.needsReview
        }
      });

      const pipelineReport: AutoPipelineReport = {
        jobId,
        confidenceThreshold: threshold,
        llm: {
          profileId,
          suggestionCount,
          appliedCount,
          highConfidenceAppliedGroups,
          lowConfidenceGroups,
          blockedAutoApplyGroups,
          failures
        },
        parser: { warnings, lowConfidenceBlocks, visionTranscription },
        validationPassed: validationReport.passed,
        staticRuntimePassed,
        realRuntimePassed: false,
        runtimeMode: validationReport.runtime?.mode ?? "unknown",
        authoring: {
          remainingReviewItems: authoringReview.needsReview
        },
        status,
        currentStep,
        generatedAt: now(),
        validationReport
      };
      store.pipelineReports[jobId] = pipelineReport;
      minimizeDevProcessArtifacts(store, jobId);
      save(store);
      return pipelineReport as T;
    }

    case "update_authoring_ir": {
      const jobId = args.jobId as string;
      const patch = args.patch as AuthoringPatch;
      const next = refreshReviewState({
        ...patch.ir,
        answerKey: Object.fromEntries(patch.ir.groups.flatMap((group) => group.questions.map((question) => [question.id, question.answer ?? ""]))),
        questionOrder: patch.ir.groups.flatMap((group) => group.questions.map((question) => question.id)),
        questionDisplayMap: Object.fromEntries(patch.ir.groups.flatMap((group) => group.questions.map((question) => [question.id, question.displayNumber]))),
        audit: { ...patch.ir.audit, revision: patch.ir.audit.revision + 1, updatedAt: now() }
      });
      store.authoring[jobId] = next.ir;
      updateJob(store, jobId, {
        status: next.needsReview ? "NeedsReview" : "DraftSaved",
        currentStep: "Authoring",
        issueCounts: { errors: 0, warnings: 0, needsReview: next.needsReview }
      });
      save(store);
      return store.authoring[jobId] as T;
    }

    case "render_group_html": {
      const jobId = args.jobId as string;
      const groupId = args.groupId as string;
      const group = store.authoring[jobId]?.groups.find((item) => item.groupId === groupId);
      if (!group) throw new Error(`group_not_found:${groupId}`);
      return { groupId, bodyHtml: renderGroupBodyHtml(group) } as T;
    }

    case "list_llm_profiles": {
      return store.profiles as T;
    }

    case "run_environment_preflight": {
      const report: EnvironmentPreflightReport = {
        schemaVersion: "EnvironmentPreflightV1",
        ok: true,
        errors: 0,
        warnings: 2,
        generatedAt: now(),
        checks: [
          { name: "node", ok: true, severity: "info", message: "Browser dev fallback cannot inspect host Node; real Tauri production does not require Node. Node is only for optional validator/runtime diagnostics." },
          { name: "rust:text-parser", ok: true, severity: "info", message: "Built-in Rust TXT/MD parsing is available in real Tauri." },
          { name: "rust:pdf-extract", ok: true, severity: "info", message: "Built-in Rust PDF text-layer extraction is available in real Tauri." },
          { name: "rust:docx-ooxml", ok: true, severity: "info", message: "Built-in Rust DOCX OOXML extraction is available in real Tauri." },
          { name: "python3", ok: true, severity: "warning", message: "Python is optional for TXT/MD and clear text PDF/DOCX parsing, but real Tauri still uses it for embedded image extraction and legacy fallback." },
          { name: "sidecar:node-validator", ok: true, severity: "warning", message: "Node validator is a supplementary parity check; Rust built-in validation is authoritative in real Tauri." },
          { name: "runtime:unified-html", ok: false, severity: "warning", message: "Browser dev fallback uses simulator; configure real runtime only for explicit E2E diagnostics." },
          { name: "runtime:unified-python", ok: false, severity: "warning", message: "Browser dev fallback uses simulator; configure real runtime only for explicit E2E diagnostics." }
        ]
      };
      return report as T;
    }

    case "get_diagnostics_settings": {
      return store.diagnostics as T;
    }

    case "save_diagnostics_settings": {
      store.diagnostics = args.settings as DiagnosticsSettings;
      save(store);
      return store.diagnostics as T;
    }

    case "save_llm_profile": {
      const input = args.input as SaveLlmProfileInput;
      const profile: LlmProfilePublic = {
        profileId: input.profileId ?? id("profile"),
        name: input.name,
        provider: input.provider,
        baseUrl: input.baseUrl,
        model: input.model,
        temperature: input.temperature,
        timeoutMs: input.timeoutMs,
        forceJson: input.forceJson,
        enabled: input.enabled,
        hasApiKey: Boolean(input.apiKey),
        apiKeySecretRef: `dev-fallback-secret:${input.profileId ?? "new"}`,
        secretStorageBackend: input.apiKey ? "file" : "none",
        secretStorageMessage: input.apiKey ? "Browser dev fallback keeps only key presence metadata in localStorage." : "No API key saved."
      };
      store.profiles = [profile, ...store.profiles.filter((item) => item.profileId !== profile.profileId)];
      save(store);
      return profile as T;
    }

    case "test_llm_profile": {
      const result: LlmTestResult = { ok: true, message: "Local placeholder gateway returned strict JSON.", latencyMs: 38 };
      return result as T;
    }

    case "llm_classify_group":
    case "llm_extract_group": {
      const jobId = args.jobId as string;
      const groupId = args.groupId as string;
      const ir = store.authoring[jobId];
      const group = ir?.groups.find((item) => item.groupId === groupId);
      if (!group) throw new Error(`group_not_found:${groupId}`);
      const suggestion: LlmSuggestion = {
        suggestionId: id("suggestion"),
        jobId,
        groupId,
        kind: group.kind,
        confidence: 0.64,
        patch: [
          { op: "replace", path: "/kind", value: group.kind },
          { op: "replace", path: "/layout/template", value: group.layout.template }
        ],
        questions: group.questions.map((question) => ({ id: question.id, prompt: question.prompt, interaction: question.interaction })),
        evidence: { source: "dev-fallback-local-heuristic", directJsGeneration: false, fallback: true },
        warnings: ["deterministic-local-fallback", "low-confidence-review-required", "fallback-output-never-auto-applies"],
        createdAt: now()
      };
      store.suggestions[jobId] = [suggestion, ...(store.suggestions[jobId] ?? [])];
      updateJob(store, jobId, { status: suggestion.confidence < 0.85 ? "NeedsReview" : "DraftSaved" });
      save(store);
      return suggestion as T;
    }

    case "apply_llm_suggestion": {
      const jobId = args.jobId as string;
      const ir = store.authoring[jobId];
      if (!ir) throw new Error("authoring_ir_missing");
      const suggestionId = args.suggestionId as string;
      const suggestion = (store.suggestions[jobId] ?? []).find((item) => item.suggestionId === suggestionId);
      if (!suggestion) throw new Error(`suggestion_not_found:${suggestionId}`);
      if (suggestion.confidence < 0.85) throw new Error("low_confidence_suggestion_requires_manual_review");
      const selectedPaths = (args.selectedPaths ?? []) as string[];
      const autoApplyIssues = suggestionAutoApplyIssues(ir, suggestion, selectedPaths);
      if (autoApplyIssues.length) throw new Error(`llm_suggestion_auto_apply_blocked:${autoApplyIssues.join(",")}`);
      const patched = refreshAuthoringDerivedFields(applySuggestionPatch(ir, suggestion, selectedPaths));
      const withSuggestionAudit: ReadingAuthoringIr = {
        ...patched,
        audit: { ...patched.audit, llmUsed: true, updatedAt: now(), revision: patched.audit.revision + 1 }
      };
      const next = refreshReviewState(withSuggestionAudit);
      store.authoring[jobId] = next.ir;
      const sourceReviewIssueCount = sourceReviewIssues(sourceReviewStatus(store, jobId)).length;
      updateJob(store, jobId, { status: next.needsReview || sourceReviewIssueCount ? "NeedsReview" : "DraftSaved", currentStep: "Authoring", issueCounts: { ...requireJob(store, jobId).issueCounts, needsReview: next.needsReview + sourceReviewIssueCount } });
      save(store);
      return store.authoring[jobId] as T;
    }

    case "validate_authoring_ir": {
      const jobId = args.jobId as string;
      const report = validateIr(jobId, store.authoring[jobId]);
      store.validation[jobId] = report;
      const sourceReviewIssueCount = sourceReviewIssues(sourceReviewStatus(store, jobId)).length;
      updateJob(store, jobId, {
        status: sourceReviewIssueCount ? "NeedsReview" : report.passed ? "DraftSaved" : "NeedsReview",
        currentStep: sourceReviewIssueCount ? "DocumentReview" : "Authoring",
        issueCounts: {
          errors: report.issues.filter((issue) => issue.severity === "error").length,
          warnings: report.issues.filter((issue) => issue.severity === "warning").length,
          needsReview: sourceReviewIssueCount
        }
      });
      save(store);
      return report as T;
    }

    case "generate_preview_assets": {
      const jobId = args.jobId as string;
      const ir = store.authoring[jobId];
      if (!ir) throw new Error("authoring_ir_missing");
      const source = toReadingExamSource(ir);
      const assets: PreviewAssets = {
        examId: source.examId,
        manifestPath: `local://${jobId}/preview/manifest.js`,
        scriptPath: `local://${jobId}/preview/${source.examId}.js`,
        previewUrl: `local-preview://${source.examId}`,
        source,
        wrapperJs: buildWrapper(source),
        manifestJs: buildManifest([source])
      };
      const validationReport = mergeValidationReports(validateIr(jobId, ir), runtimePreviewReport(jobId, assets, source));
      if (!validationReport.passed) {
        store.validation[jobId] = validationReport;
        updateJob(store, jobId, { status: "NeedsReview", currentStep: "Authoring" });
        save(store);
        throw new Error(`preview_validation_failed:${validationReport.issues.map((issue) => issue.message).join(";")}`);
      }
      store.previews[jobId] = assets;
      store.validation[jobId] = validationReport;
      const readiness = publishReadinessReport(store, jobId, ir, validationReport);
      updateJob(store, jobId, {
        status: readiness.issues.some((issue) => issue.layer === "AuthoringIR") ? "NeedsReview" : "DraftSaved",
        currentStep: "Preview",
        issueCounts: {
          errors: validationReport.issues.filter((issue) => issue.severity === "error").length,
          warnings: validationReport.issues.filter((issue) => issue.severity === "warning").length,
          needsReview: readiness.issues.filter((issue) => issue.layer === "AuthoringIR").length
        }
      });
      save(store);
      return assets as T;
    }

    case "run_preview_e2e": {
      const jobId = args.jobId as string;
      const ir = store.authoring[jobId];
      if (!ir) throw new Error("authoring_ir_missing");
      const source = toReadingExamSource(ir);
      const assets = store.previews[jobId] ?? {
        examId: source.examId,
        manifestPath: `local://${jobId}/preview/manifest.js`,
        scriptPath: `local://${jobId}/preview/${source.examId}.js`,
        previewUrl: `local-preview://${source.examId}`,
        source,
        wrapperJs: buildWrapper(source),
        manifestJs: buildManifest([source])
      };
      store.previews[jobId] = assets;
      const report = mergeValidationReports(validateIr(jobId, ir), runtimePreviewReport(jobId, assets, source));
      if (report.passed) {
        const staticRuntimePassed = report.runtime?.mode === "static-rust";
        updateJob(store, jobId, {
          status: staticRuntimePassed ? "ExportReady" : "DraftSaved",
          currentStep: staticRuntimePassed ? "Export" : "Preview"
        });
      }
      store.validation[jobId] = report;
      save(store);
      return report as T;
    }

    case "export_reading_assets": {
      const jobId = args.jobId as string;
      const ir = store.authoring[jobId];
      if (!ir) throw new Error("authoring_ir_missing");
      const source = toReadingExamSource(ir);
      const assets = store.previews[jobId] ?? {
        examId: source.examId,
        manifestPath: `local://${jobId}/preview/manifest.js`,
        scriptPath: `local://${jobId}/preview/${source.examId}.js`,
        previewUrl: `local-preview://${source.examId}`,
        source,
        wrapperJs: buildWrapper(source),
        manifestJs: buildManifest([source])
      };
      store.previews[jobId] = assets;
      const report = mergeValidationReports(validateIr(jobId, ir), runtimePreviewReport(jobId, assets, source));
      const readiness = publishReadinessReport(store, jobId, ir, report);
      store.validation[jobId] = readiness;
      if (!report.passed) {
        save(store);
        throw new Error(`export_validation_failed:${report.issues.map((issue) => issue.message).join(";")}`);
      }
      if (!readiness.passed) {
        save(store);
        throw new Error(`export_validation_failed:${readiness.issues.map((issue) => issue.message).join(";")}`);
      }
      const result: ExportResult = {
        examId: source.examId,
        files: [
          { name: `${source.examId}.json`, content: JSON.stringify(source, null, 2) },
          { name: `${source.examId}.js`, content: buildWrapper(source) },
          { name: "manifest.js", content: buildManifest([source]) },
          { name: "preview.html", content: previewHtml(source) }
        ],
        outputDir: "local://exports",
        exportSummary: { type: "reading-assets", examId: source.examId, exportedAt: now() }
      };
      updateJob(store, jobId, { status: "Exported", currentStep: "Export" });
      result.cleanup = cleanupDevArtifacts(store, jobId, result.exportSummary);
      save(store);
      return result as T;
    }

    case "build_pack": {
      const input = args.input as BuildPackInput;
      const sources = input.jobIds.map((jobId) => {
        const ir = store.authoring[jobId];
        if (!ir) throw new Error(`authoring_ir_missing:${jobId}`);
        const source = toReadingExamSource(ir);
        const assets = store.previews[jobId] ?? {
          examId: source.examId,
          manifestPath: `local://${jobId}/preview/manifest.js`,
          scriptPath: `local://${jobId}/preview/${source.examId}.js`,
          previewUrl: `local-preview://${source.examId}`,
          source,
          wrapperJs: buildWrapper(source),
          manifestJs: buildManifest([source])
        };
        store.previews[jobId] = assets;
        const report = mergeValidationReports(validateIr(jobId, ir), runtimePreviewReport(jobId, assets, source));
        const readiness = publishReadinessReport(store, jobId, ir, report);
        store.validation[jobId] = readiness;
        if (!report.passed) throw new Error(`pack_validation_failed:${jobId}:${report.issues.map((issue) => issue.message).join(";")}`);
        if (!readiness.passed) throw new Error(`pack_validation_failed:${jobId}:${readiness.issues.map((issue) => issue.message).join(";")}`);
        return source;
      });
      const manifest = packManifest(input, sources);
      const entries = [
        { path: "pack.json", content: JSON.stringify(manifest, null, 2) },
        { path: "reading-exams/manifest.js", content: buildManifest(sources) },
        ...sources.map((source) => ({ path: `reading-exams/${source.examId}.js`, content: buildWrapper(source) }))
      ];
      const result: PackBuildResult = {
        packId: input.packId,
        outputPath: `local://packs/${input.packId}.zip`,
        files: entries.map((entry) => entry.path),
        zipSizeBytes: estimateStoredZipSize(entries),
        entryCount: entries.length,
        manifest,
        exportSummary: { type: "pack", packId: input.packId, outputPath: `local://packs/${input.packId}.zip`, exportedAt: now() },
        createdAt: manifest.generatedAt
      };
      store.packs.unshift(result);
      input.jobIds.forEach((jobId) => updateJob(store, jobId, { status: "Exported", currentStep: "Pack" }));
      result.cleanup = input.jobIds.map((jobId) => cleanupDevArtifacts(store, jobId, result.exportSummary));
      save(store);
      return result as T;
    }

    case "reveal_job_folder":
    case "choose_export_dir": {
      return "local://exports" as T;
    }

    default:
      throw new Error(`dev_fallback_command_not_implemented:${command}`);
  }
}
