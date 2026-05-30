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
  PackBuildResult,
  ParseOptions,
  PreviewAssets,
  ReadingAuthoringIr,
  SaveLlmProfileInput,
  SourceFile,
  SourceFileRole,
  SplitCandidates,
  ValidationIssue,
  ValidationReport
} from "../types";
import { buildManifest, buildWrapper, escapeHtml, renderGroupBodyHtml, toReadingExamSource } from "./templateRenderer";

type Store = {
  jobs: ImportJob[];
  documents: Record<string, DocumentIr>;
  splits: Record<string, SplitCandidates>;
  authoring: Record<string, ReadingAuthoringIr>;
  validation: Record<string, ValidationReport>;
  previews: Record<string, PreviewAssets>;
  profiles: LlmProfilePublic[];
  suggestions: Record<string, LlmSuggestion[]>;
  packs: PackBuildResult[];
};

export interface JobDetail {
  job: ImportJob;
  documentIr?: DocumentIr;
  splitCandidates?: SplitCandidates;
  authoringIr?: ReadingAuthoringIr;
  validationReport?: ValidationReport;
  previewAssets?: PreviewAssets;
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
    packs: []
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
      warnings: options.mode === "ocr" ? ["OCR confidence is simulated; human confirmation required."] : []
    }
  };
}

function flattenBlocks(doc?: DocumentIr): DocumentBlock[] {
  return doc?.pages.flatMap((page) => page.blocks) ?? [];
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

function detectGroupKind(text: string): GroupKind {
  const lower = text.toLowerCase();
  if (lower.includes("true") && lower.includes("false") && lower.includes("not given")) return "true_false_not_given";
  if (lower.includes("yes") && lower.includes("no") && lower.includes("not given")) return "yes_no_not_given";
  if (lower.includes("complete the table") || lower.includes("table below") || lower.includes("|") && lower.includes("complete")) return "table_completion";
  if (lower.includes("choose") && lower.includes("letter") && /\b[A-D]\b/.test(text)) return "single_choice";
  if (lower.includes("choose") && (lower.includes("two") || lower.includes("three"))) return "multi_choice";
  if (lower.includes("complete the summary")) return "summary_completion";
  if (lower.includes("complete the sentence")) return "sentence_completion";
  if (lower.includes("short answer")) return "short_answer";
  return "short_answer";
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

function inferPassageTitle(job: ImportJob, passageBlocks: DocumentBlock[]): string {
  const title = passageBlocks
    .map(blockText)
    .find((text) => text && !/^READING PASSAGE/i.test(text));
  return title ?? job.title;
}

function makeSplit(jobId: string, doc?: DocumentIr, job?: ImportJob): SplitCandidates {
  const blocks = flattenBlocks(doc);
  if (!blocks.length) return makeStaticSplit(jobId);

  const firstQuestionIndex = blocks.findIndex((block) => block.roleHint === "question" || /Questions?\s+\d/i.test(blockText(block)));
  const firstAnswerIndex = blocks.findIndex((block) => block.roleHint === "answer" || /^Answers?/i.test(blockText(block)) || /answer key/i.test(blockText(block)));
  const passageBlocks = blocks.filter((block, index) => block.roleHint === "passage" || (firstQuestionIndex >= 0 && index < firstQuestionIndex && block.roleHint !== "ignore"));
  const questionEndIndex = firstAnswerIndex > firstQuestionIndex ? firstAnswerIndex : undefined;
  const questionBlocks =
    firstQuestionIndex >= 0
      ? blocks
          .slice(firstQuestionIndex, questionEndIndex)
          .filter((block) => block.roleHint !== "answer" && block.roleHint !== "ignore")
      : blocks.filter((block) => block.roleHint === "question" || /Questions?\s+\d/i.test(blockText(block)));
  const answerBlocks = blocks.filter((block) => block.roleHint === "answer" || /^Answers?/i.test(blockText(block)) || /answer key/i.test(blockText(block)));

  const answerMap = answerBlocks.reduce<Record<string, AnswerValue>>((acc, block) => ({ ...acc, ...parseAnswerText(blockText(block)) }), {});
  const answerNumbers = Object.keys(answerMap).map(Number).filter(Number.isFinite).sort((a, b) => a - b);

  const questionGroupCandidates = questionBlocks
    .map((block, index) => {
      const text = blockText(block);
      const range = detectQuestionRange(text);
      if (!range) return null;
      const nextHeadingIndex = questionBlocks.findIndex((candidate, candidateIndex) => candidateIndex > index && detectQuestionRange(blockText(candidate)));
      const included = nextHeadingIndex > -1 ? questionBlocks.slice(index, nextHeadingIndex) : questionBlocks.slice(index);
      return {
        groupId: `group-${index + 1}`,
        heading: text.match(/Questions?\s+\d{1,3}(?:\s*[-–—]\s*\d{1,3})?/i)?.[0] ?? `Questions ${range[0]}-${range[1]}`,
        questionRange: range,
        instructionText: text,
        blockIds: included.map((item) => item.blockId),
        kindHint: detectGroupKind(included.map(blockText).join(" ")),
        confidence: 0.72
      };
    })
    .filter((candidate): candidate is NonNullable<typeof candidate> => Boolean(candidate));
  questionGroupCandidates.forEach((candidate, index) => {
    candidate.groupId = `group-${index + 1}`;
  });

  if (!questionGroupCandidates.length && questionBlocks.length) {
    const start = answerNumbers[0] ?? 1;
    const end = answerNumbers.at(-1) ?? start;
    questionGroupCandidates.push({
      groupId: "group-1",
      heading: `Questions ${start}-${end}`,
      questionRange: [start, end],
      instructionText: questionBlocks.map(blockText).join("\n"),
      blockIds: questionBlocks.map((block) => block.blockId),
      kindHint: detectGroupKind(questionBlocks.map(blockText).join(" ")),
      confidence: 0.58
    });
  }

  const fallbackPassageRange = firstQuestionIndex > 0 ? blocks.slice(0, firstQuestionIndex).map((block) => block.blockId) : blocks.slice(0, Math.max(1, Math.min(3, blocks.length))).map((block) => block.blockId);
  const passageRange = passageBlocks.length ? passageBlocks.map((block) => block.blockId) : fallbackPassageRange;
  const issues = [
    ...(questionGroupCandidates.length ? [] : ["No question range heading detected; manual split required."]),
    ...(Object.keys(answerMap).length ? [] : ["No answer key detected; answers must be entered manually."]),
    ...(firstAnswerIndex >= 0 && firstQuestionIndex >= 0 && firstAnswerIndex < firstQuestionIndex ? ["Answer block appears before question block; verify split order."] : [])
  ];

  return {
    jobId,
    passageCandidates: [{ range: passageRange, title: inferPassageTitle(job ?? { title: "Untitled Reading" } as ImportJob, passageBlocks), categoryHint: job?.category ?? "P1" }],
    questionGroupCandidates,
    answerKeyCandidates: Object.keys(answerMap).length ? [{ source: answerBlocks.map((block) => block.blockId).join(",") || "manual", answers: answerMap }] : [],
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
  if (kind === "multi_choice") return { type: "checkbox" as const, options: ["A", "B", "C", "D", "E", "F"] };
  return { type: "text" as const, placeholder: "answer" };
}

function templateForKind(kind: GroupKind): string {
  const mapping: Partial<Record<GroupKind, string>> = {
    true_false_not_given: "tfng_list",
    yes_no_not_given: "ynng_list",
    single_choice: "single_choice_list",
    multi_choice: "multi_choice_checkbox",
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
    const groupBlocks = candidate.blockIds.map((blockId) => blocksById.get(blockId)).filter(Boolean) as DocumentBlock[];
    const groupText = groupBlocks.map(blockText).join(" ") || candidate.instructionText;
    const [start, end] = candidate.questionRange;
    const questions = Array.from({ length: Math.max(0, end - start + 1) }, (_, offset) => start + offset).map((number) => {
      const displayNumber = String(number);
      const idValue = `q${displayNumber}`;
      return {
        id: idValue,
        displayNumber,
        prompt: promptForQuestion(groupText, number, candidate.heading, end),
        interaction: interactionForKind(kind),
        answer: answerByDisplay[displayNumber],
        sourceBlockIds: candidate.blockIds,
        confidence: candidate.confidence,
        verified: false
      };
    });
    return {
      groupId: candidate.groupId || `group-${index + 1}`,
      kind,
      questionRange: candidate.questionRange,
      instruction: [candidate.instructionText],
      questions,
      layout: { template: templateForKind(kind), ...(kind === "table_completion" ? { tableHeaders: ["Question", "Prompt", "Answer"] } : {}) },
      sourceBlockIds: candidate.blockIds,
      confidence: candidate.confidence,
      verified: false
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
      tags: job.tags
    },
    passage: {
      title: split.passageCandidates[0]?.title ?? job.title,
      htmlBlocks: [{ blockId: "passage-main", html: passageHtml }],
      sourceBlockIds: split.passageCandidates[0]?.range ?? []
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
    add("AuthoringIR", "$", "Authoring IR is missing.", "Build Authoring IR from split candidates first.");
  } else {
    if (!ir.exam.examId) add("AuthoringIR", "$.exam.examId", "examId is required.");
    if (!ir.passage.htmlBlocks.length) add("AuthoringIR", "$.passage.htmlBlocks", "Passage HTML cannot be empty.");
    if (!ir.groups.length) add("AuthoringIR", "$.groups", "At least one question group is required.");
    for (const group of ir.groups) {
      for (const question of group.questions) {
        if (!question.interaction?.type) add("AuthoringIR", `$.groups.${group.groupId}.${question.id}`, "Question interaction is required.");
        if (!question.answer || (Array.isArray(question.answer) && !question.answer.length)) {
          add("AuthoringIR", `$.answerKey.${question.id}`, "Every question must have an answer before export.");
        }
      }
    }

    const source = toReadingExamSource(ir);
    if (source.schemaVersion !== "ReadingExamSourceV1") add("ReadingExamSourceV1", "$.schemaVersion", "Invalid schemaVersion.");
    if (!source.answerKey || !Object.keys(source.answerKey).length) add("ReadingExamSourceV1", "$.answerKey", "answerKey cannot be empty.");

    for (const group of source.questionGroups) {
      for (const qid of group.questionIds) {
        const hasNamedControl = new RegExp(`name=["']${qid}["']|data-question=["']${qid}["']|data-question-id=["']${qid}["']`).test(group.bodyHtml);
        if (!hasNamedControl) {
          add("DomProtocol", `$.questionGroups.${group.groupId}.bodyHtml`, `No collectible control found for ${qid}.`);
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
    body{font-family:Georgia,serif;margin:0;padding:24px;color:#15211f;background:#f5f1e8;line-height:1.6}.layout{display:grid;grid-template-columns:minmax(0,1fr) minmax(360px,.8fr);gap:32px}.passage,.questions{background:#fffaf0;border:1px solid #d8cfbf;padding:22px}.choice-row{display:flex;gap:10px;flex-wrap:wrap}.completion-table{width:100%;border-collapse:collapse}.completion-table th,.completion-table td{border:1px solid #c8beaa;padding:8px}input{font:inherit;padding:6px;border:1px solid #9aa391}
  </style></head><body><div class="layout"><article class="passage">${source.passage.blocks
    .map((block) => block.html)
    .join("")}</article><section class="questions">${source.questionGroups.map((group) => group.bodyHtml).join("")}</section></div></body></html>`;
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
    add("preview-assets", "Preview assets must be generated before RuntimePreview E2E.");
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
      add("runtime.execution", `Generated manifest/wrapper failed to execute: ${error instanceof Error ? error.message : String(error)}`);
    }
    if (!registry.has(source.examId)) add(`${source.examId}.js`, `Generated wrapper did not register ${source.examId}.`);
    const manifest = runtime.__READING_EXAM_MANIFEST__ as Record<string, unknown> | undefined;
    if (!manifest?.[source.examId]) add("manifest.js", `Manifest does not contain ${source.examId}.`);
  }

  const collected: Record<string, AnswerValue> = {};
  for (const group of source.questionGroups) {
    for (const qid of group.questionIds) {
      const controls = controlsFor(group.bodyHtml, qid);
      if (!controls.length) {
        add(`$.questionGroups.${group.groupId}.bodyHtml`, `No runtime-collectible control or dropzone found for ${qid}.`);
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
            add(`$.questionGroups.${group.groupId}.bodyHtml`, `Answer for ${qid} is not present in option values.`);
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
  if (scoreInfo.total > 0 && scoreInfo.percent !== 100) add("runtime.scoreInfo", `Correct-answer E2E expected 100%, got ${scoreInfo.percent}%.`);
  if (scoreInfo.total > 0 && wrongScoreInfo.percent >= scoreInfo.percent) add("runtime.scoreInfo", "Wrong-answer sample did not reduce the runtime score.");

  return {
    jobId,
    passed: issues.length === 0,
    layers: [{ layer: "RuntimePreview", passed: issues.length === 0, issueCount: issues.length }],
    issues,
    runtime: {
      adapter: "dev-fallback-unified-runtime-contract-simulator",
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

export async function devFallbackInvoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  const store = load();

  switch (command) {
    case "create_import_job": {
      const input = (args.input ?? {}) as { title?: string; category?: ImportJob["category"]; frequency?: ImportJob["frequency"]; tags?: string[]; llmProfileId?: string };
      const job: ImportJob = {
        jobId: id("import"),
        title: input.title?.trim() || "Untitled Reading",
        status: "Draft",
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
        splitCandidates: store.splits[jobId],
        authoringIr: store.authoring[jobId],
        validationReport: store.validation[jobId],
        previewAssets: store.previews[jobId]
      };
      return detail as T;
    }

    case "update_job_meta": {
      const job = updateJob(store, args.jobId as string, args.patch as JobMetaPatch);
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
      updateJob(store, jobId, { sourceFiles: [...job.sourceFiles, source], status: "Uploaded", currentStep: "DocumentReview" });
      save(store);
      return source as T;
    }

    case "parse_document": {
      const jobId = args.jobId as string;
      const job = requireJob(store, jobId);
      const ir = makeDocumentIr(job, (args.options ?? { mode: "auto" }) as ParseOptions);
      store.documents[jobId] = ir;
      updateJob(store, jobId, { status: "Parsed", currentStep: "DocumentReview" });
      save(store);
      return ir as T;
    }

    case "rerun_ocr": {
      const jobId = args.jobId as string;
      const job = requireJob(store, jobId);
      const ir = makeDocumentIr(job, { mode: "ocr" });
      store.documents[jobId] = ir;
      save(store);
      return ir as T;
    }

    case "run_rule_split": {
      const jobId = args.jobId as string;
      requireJob(store, jobId);
      const split = makeSplit(jobId, store.documents[jobId], requireJob(store, jobId));
      store.splits[jobId] = split;
      updateJob(store, jobId, { status: "SplitReady", currentStep: "Split" });
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
      updateJob(store, jobId, { status: "AuthoringReady", currentStep: "Authoring", issueCounts: { errors: 0, warnings: 1, needsReview: 8 } });
      save(store);
      return ir as T;
    }

    case "update_authoring_ir": {
      const jobId = args.jobId as string;
      const patch = args.patch as AuthoringPatch;
      store.authoring[jobId] = {
        ...patch.ir,
        answerKey: Object.fromEntries(patch.ir.groups.flatMap((group) => group.questions.map((question) => [question.id, question.answer ?? ""]))),
        questionOrder: patch.ir.groups.flatMap((group) => group.questions.map((question) => question.id)),
        questionDisplayMap: Object.fromEntries(patch.ir.groups.flatMap((group) => group.questions.map((question) => [question.id, question.displayNumber]))),
        audit: { ...patch.ir.audit, revision: patch.ir.audit.revision + 1, updatedAt: now() }
      };
      updateJob(store, jobId, { issueCounts: { errors: 0, warnings: 0, needsReview: store.authoring[jobId].groups.flatMap((g) => g.questions).filter((q) => !q.verified).length } });
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
        confidence: group.kind === "table_completion" ? 0.78 : 0.91,
        patch: [
          { op: "replace", path: "/kind", value: group.kind },
          { op: "replace", path: "/layout/template", value: group.layout.template }
        ],
        questions: group.questions.map((question) => ({ id: question.id, prompt: question.prompt, interaction: question.interaction })),
        evidence: { source: "dev-fallback-local-heuristic", directJsGeneration: false },
        warnings: group.kind === "table_completion" ? ["Table layout should be reviewed by a human.", "low-confidence-review-required"] : [],
        createdAt: now()
      };
      store.suggestions[jobId] = [suggestion, ...(store.suggestions[jobId] ?? [])];
      updateJob(store, jobId, { status: suggestion.confidence < 0.85 ? "NeedsHumanReview" : "AuthoringReady" });
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
      const patched = refreshAuthoringDerivedFields(applySuggestionPatch(ir, suggestion, selectedPaths));
      store.authoring[jobId] = { ...patched, audit: { ...patched.audit, llmUsed: true, updatedAt: now(), revision: patched.audit.revision + 1 } };
      updateJob(store, jobId, { status: "AuthoringReady" });
      save(store);
      return store.authoring[jobId] as T;
    }

    case "validate_authoring_ir": {
      const jobId = args.jobId as string;
      const report = validateIr(jobId, store.authoring[jobId]);
      store.validation[jobId] = report;
      updateJob(store, jobId, {
        status: report.passed ? "PreviewReady" : "ValidationFailed",
        issueCounts: {
          errors: report.issues.filter((issue) => issue.severity === "error").length,
          warnings: report.issues.filter((issue) => issue.severity === "warning").length,
          needsReview: 0
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
      store.previews[jobId] = assets;
      updateJob(store, jobId, { status: "PreviewReady", currentStep: "Preview" });
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
        updateJob(store, jobId, { status: "ExportReady", currentStep: "Export" });
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
      store.validation[jobId] = report;
      if (!report.passed) {
        save(store);
        throw new Error(`export_validation_failed:${report.issues.map((issue) => issue.message).join(";")}`);
      }
      const result: ExportResult = {
        examId: source.examId,
        files: [
          { name: `${source.examId}.json`, content: JSON.stringify(source, null, 2) },
          { name: `${source.examId}.js`, content: buildWrapper(source) },
          { name: "manifest.js", content: buildManifest([source]) },
          { name: "preview.html", content: previewHtml(source) }
        ]
      };
      updateJob(store, jobId, { status: "ExportReady", currentStep: "Export" });
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
        store.validation[jobId] = report;
        if (!report.passed) throw new Error(`pack_validation_failed:${jobId}:${report.issues.map((issue) => issue.message).join(";")}`);
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
        createdAt: manifest.generatedAt
      };
      store.packs.unshift(result);
      input.jobIds.forEach((jobId) => updateJob(store, jobId, { status: "Published", currentStep: "Pack" }));
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
