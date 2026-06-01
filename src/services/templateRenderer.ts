import type { QuestionDraft, QuestionGroupDraft, ReadingAuthoringIr } from "../types/authoring-ir";
import type { ReadingExamSourceV1 } from "../types/reading-source";

export function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function renderRadioQuestion(question: QuestionDraft): string {
  const options = question.interaction.options ?? [];
  return `
    <li class="author-question" data-question-id="${question.id}">
      <div class="question-prompt"><strong>${escapeHtml(question.displayNumber)}</strong> ${escapeHtml(question.prompt)}</div>
      <div class="choice-row">
        ${options
          .map(
            (option) =>
              `<label><input name="${question.id}" type="radio" value="${escapeHtml(option)}"> ${escapeHtml(option)}</label>`
          )
          .join("")}
      </div>
    </li>`;
}

function renderCheckboxQuestion(question: QuestionDraft): string {
  const options = question.interaction.options ?? [];
  return `
    <li class="author-question" data-question-id="${question.id}">
      <div class="question-prompt"><strong>${escapeHtml(question.displayNumber)}</strong> ${escapeHtml(question.prompt)}</div>
      <div class="choice-row">
        ${options
          .map(
            (option) =>
              `<label><input name="${question.id}" type="checkbox" value="${escapeHtml(option)}"> ${escapeHtml(option)}</label>`
          )
          .join("")}
      </div>
    </li>`;
}

function renderTextQuestion(question: QuestionDraft): string {
  return `
    <li class="author-question inline-input" data-question-id="${question.id}">
      <label><strong>${escapeHtml(question.displayNumber)}</strong> ${escapeHtml(question.prompt)}
        <input type="text" id="${question.id}_input" name="${question.id}" placeholder="${escapeHtml(
          question.interaction.placeholder ?? "answer"
        )}">
      </label>
    </li>`;
}

function renderTableCompletion(group: QuestionGroupDraft): string {
  const headers = group.layout.tableHeaders?.length ? group.layout.tableHeaders : ["Question", "Prompt", "Answer"];
  const body = group.questions
    .map(
      (question) => `<tr>
        <td><strong>${escapeHtml(question.displayNumber)}</strong></td>
        <td>${escapeHtml(question.prompt)}</td>
        <td><input type="text" id="${question.id}_input" name="${question.id}" placeholder="answer"></td>
      </tr>`
    )
    .join("");

  return `<table class="completion-table">
    <thead><tr>${headers.map((header) => `<th>${escapeHtml(header)}</th>`).join("")}</tr></thead>
    <tbody>${body}</tbody>
  </table>`;
}

export function renderGroupBodyHtml(group: QuestionGroupDraft): string {
  const lead = group.instruction.map((item) => `<p>${escapeHtml(item)}</p>`).join("");
  let body = "";

  if (group.kind === "table_completion") {
    body = renderTableCompletion(group);
  } else if (group.kind === "multi_choice") {
    body = `<ol>${group.questions.map(renderCheckboxQuestion).join("")}</ol>`;
  } else if (group.kind === "true_false_not_given" || group.kind === "yes_no_not_given" || group.kind === "single_choice") {
    body = `<ol>${group.questions.map(renderRadioQuestion).join("")}</ol>`;
  } else {
    body = `<ol>${group.questions.map(renderTextQuestion).join("")}</ol>`;
  }

  return `<section class="reading-question-group" id="${group.groupId}">
    <div class="group-lead">${lead}</div>
    ${body}
  </section>`;
}

export function toReadingExamSource(ir: ReadingAuthoringIr): ReadingExamSourceV1 {
  const sourceFile = ir.exam.sourceFiles?.find((source) => source.role === "MainQuestion");
  const questionUmbrellaRanges = ir.passage.questionUmbrellaRanges ?? [];
  const questionIntroHtml = questionUmbrellaRanges.length
    ? `<h3>Questions</h3><ul class="question-umbrella-ranges">${questionUmbrellaRanges
        .map(
          (range) =>
            `<li><strong>${escapeHtml(range.heading)}</strong><span>Q${range.questionRange[0]}-${range.questionRange[1]}</span></li>`
        )
        .join("")}</ul>`
    : "<h3>Questions</h3>";
  return {
    schemaVersion: "ReadingExamSourceV1",
    examId: ir.exam.examId,
    meta: {
      title: ir.exam.title,
      category: ir.exam.category,
      frequency: ir.exam.frequency,
      pdfFilename: sourceFile?.originalName ?? "source.pdf",
      legacyPath: "",
      legacyFilename: "",
      questionIntroHtml,
      questionUmbrellaRanges
    },
    passage: {
      blocks: ir.passage.htmlBlocks.map((block) => ({ blockId: block.blockId, kind: "html", html: block.html }))
    },
    questionGroups: ir.groups.map((group) => ({
      groupId: group.groupId,
      kind: group.kind,
      questionIds: group.questions.map((question) => question.id),
      bodyHtml: renderGroupBodyHtml(group),
      leadHtml: group.instruction.map((item) => `<p>${escapeHtml(item)}</p>`).join(""),
      allowOptionReuse: group.allowOptionReuse
    })),
    answerKey: ir.answerKey,
    sourceRefs: {
      primaryHtml: `author-imports/${ir.jobId}/intermediate.html`,
      primaryProvider: "author_web",
      shuiHtml: null,
      shuiPdf: `uploads/${sourceFile?.storedName ?? "source.pdf"}`,
      ieltsHtml: null
    },
    audit: {
      matchStatus: ir.audit.humanVerified ? "author_verified" : "needs_review",
      matchConfidence: ir.audit.humanVerified ? 1 : 0,
      verifiedAt: ir.audit.humanVerified ? new Date().toISOString() : null,
      notes: `provider:author_tauri;sourceFileId:${sourceFile?.fileId ?? "unknown-source"};sourceSha256:${sourceFile?.sha256 ?? "unknown-sha256"};signature:radio,text,table`
    },
    questionOrder: ir.questionOrder,
    questionDisplayMap: ir.questionDisplayMap
  };
}

export function buildWrapper(source: ReadingExamSourceV1): string {
  return `(function registerReadingExamData(global) {\n  'use strict';\n  if (!global.__READING_EXAM_DATA__ || typeof global.__READING_EXAM_DATA__.register !== "function") {\n    throw new Error("reading_exam_registry_missing");\n  }\n  global.__READING_EXAM_DATA__.register(${JSON.stringify(source.examId)}, ${JSON.stringify(source, null, 2)});\n})(typeof window !== "undefined" ? window : globalThis);\n`;
}

export function buildManifest(sources: ReadingExamSourceV1[]): string {
  const manifest = Object.fromEntries(
    sources.map((source) => [
      source.examId,
      {
        examId: source.examId,
        dataKey: source.examId,
        script: `./${source.examId}.js`,
        title: source.meta.title,
        category: source.meta.category
      }
    ])
  );

  return `window.__READING_EXAM_MANIFEST__ = ${JSON.stringify(manifest, null, 2)};\n`;
}
