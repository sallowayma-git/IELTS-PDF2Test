#!/usr/bin/env node
import fs from "node:fs/promises";
import path from "node:path";

const allowedKinds = [
  "single_choice",
  "multi_choice",
  "true_false_not_given",
  "yes_no_not_given",
  "matching",
  "classification",
  "summary_completion",
  "table_completion",
  "diagram_completion",
  "short_answer",
  "sentence_completion"
];

function normalizeKind(text = "") {
  const lower = text.toLowerCase();
  if (lower.includes("true") && lower.includes("false") && lower.includes("not given")) return "true_false_not_given";
  if (lower.includes("yes") && lower.includes("no") && lower.includes("not given")) return "yes_no_not_given";
  if (lower.includes("complete the table") || lower.includes("table below") || lower.includes("|")) return "table_completion";
  if (lower.includes("choose") && lower.includes("letter")) return "single_choice";
  if (lower.includes("choose") && (lower.includes("two") || lower.includes("three"))) return "multi_choice";
  if (lower.includes("complete the summary")) return "summary_completion";
  if (lower.includes("complete the sentence")) return "sentence_completion";
  return "short_answer";
}

function deterministicSuggestion(input, mode) {
  const group = input.group ?? {};
  const sourceBlockIds = Array.isArray(group.sourceBlockIds) ? group.sourceBlockIds : [];
  const groupText = [
    ...(group.instruction ?? []),
    ...(group.questions ?? []).map((question) => question.prompt ?? "")
  ].join(" ");
  const kind = normalizeKind(groupText);
  const confidence = 0.64;
  return {
    kind,
    confidence,
    patch: [
      { op: "replace", path: "/kind", value: kind },
      { op: "replace", path: "/layout/template", value: templateForKind(kind) }
    ],
    questions: (group.questions ?? []).map((question) => ({
      id: question.id,
      prompt: question.prompt,
      interaction: interactionForKind(kind)
    })),
    warnings: [
      "deterministic-local-fallback",
      "low-confidence-review-required",
      "fallback-output-never-auto-applies"
    ],
    evidence: {
      mode,
      allowedKinds,
      source: "local-heuristic",
      sourceBlockIds,
      quotes: [],
      fallback: true
    }
  };
}

function templateForKind(kind) {
  const mapping = {
    true_false_not_given: "tfng_list",
    yes_no_not_given: "ynng_list",
    single_choice: "single_choice_list",
    multi_choice: "multi_choice_checkbox",
    table_completion: "table_completion",
    summary_completion: "summary_text_completion",
    sentence_completion: "inline_text_completion"
  };
  return mapping[kind] ?? "short_answer_list";
}

function interactionForKind(kind) {
  if (kind === "true_false_not_given") return { type: "radio", options: ["TRUE", "FALSE", "NOT GIVEN"] };
  if (kind === "yes_no_not_given") return { type: "radio", options: ["YES", "NO", "NOT GIVEN"] };
  if (kind === "single_choice") return { type: "radio", options: ["A", "B", "C", "D"] };
  if (kind === "multi_choice") return { type: "checkbox", options: ["A", "B", "C", "D", "E", "F"] };
  return { type: "text", placeholder: "answer" };
}

function buildPrompt(input, mode) {
  return [
    "You are an IELTS Reading authoring assistant.",
    "Return JSON only. Do not return Markdown, HTML, JavaScript, or explanations.",
    "Never invent passage facts or answers. Suggest structure only.",
    "Evidence is required: include evidence.sourceBlockIds copied from the input group.sourceBlockIds and evidence.quotes as [{blockId,text}] using short source excerpts that justify the suggestion.",
    "If you cannot cite the source blocks, return confidence below 0.85.",
    `Task: ${mode}.`,
    `Allowed group kinds: ${allowedKinds.join(", ")}.`,
    `Group JSON: ${JSON.stringify(input.group ?? {})}`
  ].join("\n");
}

function transcriptionFallback(warning) {
  return {
    text: "",
    confidence: 0,
    warnings: [
      warning,
      "vision-transcription-failed",
      "manual-transcription-required"
    ],
    evidence: {
      mode: "transcribe_pdf_images",
      source: "local-fallback",
      fallback: true
    }
  };
}

function buildVisionPrompt(input) {
  return [
    "You are transcribing an IELTS Reading PDF page image for an authoring workflow.",
    "Return JSON only with shape {\"text\":\"...\",\"confidence\":0.0,\"warnings\":[]}.",
    "Transcribe all visible passage text, question headings, question prompts, options, tables, labels, and answer keys if present.",
    "Preserve useful structural headings such as READING PASSAGE, Questions 1-5, and Answers.",
    "Do not invent missing words or answers. If a region is unclear, write [unclear] and lower confidence.",
    `Job: ${JSON.stringify(input.job ?? {})}`
  ].join("\n");
}

function buildVisionAnswerPrompt(input) {
  return [
    "You are extracting the answer key from IELTS Reading PDF page images.",
    "Return JSON only with shape {\"answers\":{\"8\":\"answer text\"},\"confidence\":0.0,\"warnings\":[],\"evidence\":[{\"questionNumber\":\"8\",\"pageIndex\":1,\"quote\":\"short visible source text\"}]} .",
    "Use question number strings without q prefix. Normalize TRUE/FALSE/NOT GIVEN/YES/NO and single-letter options to uppercase.",
    "Multi-answer questions may use arrays. Do not invent answers; omit uncertain numbers and add a warning.",
    `Job: ${JSON.stringify(input.job ?? {})}`,
    `Output contract: ${JSON.stringify(input.outputContract ?? {})}`
  ].join("\n");
}

function buildCloudOutlinePrompt(input) {
  return [
    "You are creating a comparison-only outline from an IELTS Reading PDF.",
    "Return JSON only with shape {\"title\":\"paper title\",\"groups\":[{\"range\":[1,5],\"kind\":\"true_false_not_given\",\"layoutHint\":\"list\",\"questionIds\":[\"q1\"],\"notesText\":\"\",\"confidence\":0.0,\"evidence\":{\"quotes\":[{\"pageIndex\":1,\"text\":\"short visible source excerpt\"}]}}],\"answerKey\":{\"1\":\"TRUE\"},\"confidence\":0.0,\"warnings\":[]}.",
    "This output is used only to compare against a local deterministic draft; it must not overwrite the local draft.",
    "Use only visible PDF evidence. Do not invent missing groups or answers.",
    `Allowed group kinds: ${allowedKinds.join(", ")}.`,
    "If the PDF says Complete the notes below, note completion, notes, or contains numbered blank/ellipsis markers such as 8……… or 8 ______, keep the entire range as one group, set layoutHint=inline_completion, include every qN in questionIds, and copy the continuous notes text into notesText.",
    "Every group must include evidence.quotes. If evidence is missing, lower group confidence below 0.75.",
    `Job: ${JSON.stringify(input.job ?? {})}`,
    `Source file: ${JSON.stringify(input.sourceFile ?? {})}`,
    `Output contract: ${JSON.stringify(input.outputContract ?? {})}`
  ].join("\n");
}

async function imageToDataUrl(image) {
  if (!image?.path) throw new Error("vision_image_path_missing");
  const data = await fs.readFile(image.path);
  const mimeType = image.mimeType || "application/octet-stream";
  return `data:${mimeType};base64,${data.toString("base64")}`;
}

async function callVisionOpenAiCompatible(input) {
  const profile = input.profile ?? {};
  const apiKey = process.env.EPIC8_LLM_API_KEY ?? input.apiKey ?? "";
  if (!profile.baseUrl || !profile.model) return null;
  const pages = input.pages ?? [];
  const content = [{ type: "text", text: buildVisionPrompt(input) }];
  for (const page of pages) {
    for (const image of page.images ?? []) {
      content.push({ type: "text", text: `Page ${page.pageIndex}, image ${image.assetId || image.fileName || ""}` });
      content.push({ type: "image_url", image_url: { url: await imageToDataUrl(image) } });
    }
  }
  if (!content.some((item) => item.type === "image_url")) return null;

  const endpoint = new URL(profile.baseUrl.replace(/\/$/, "") + "/chat/completions");
  const response = await fetch(endpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(apiKey ? { authorization: `Bearer ${apiKey}` } : {})
    },
    body: JSON.stringify({
      model: profile.model,
      temperature: profile.temperature ?? 0,
      response_format: profile.forceJson === false ? undefined : { type: "json_object" },
      messages: [
        { role: "system", content: "Return valid JSON only." },
        { role: "user", content }
      ]
    }),
    signal: AbortSignal.timeout(profile.timeoutMs ?? 120000)
  });
  if (!response.ok) throw new Error(`llm_http_${response.status}:${await response.text()}`);
  const payload = await response.json();
  const message = payload?.choices?.[0]?.message?.content;
  if (!message) throw new Error("vision_llm_empty_content");
  const parsed = JSON.parse(message);
  return {
    text: typeof parsed.text === "string" ? parsed.text : "",
    confidence: typeof parsed.confidence === "number" ? parsed.confidence : 0.6,
    warnings: Array.isArray(parsed.warnings) ? parsed.warnings : [],
    evidence: {
      ...(parsed.evidence ?? {}),
      mode: "transcribe_pdf_images",
      source: "openai-compatible-vision",
      model: profile.model,
      usage: payload.usage ?? null
    }
  };
}

async function callImageJsonOpenAiCompatible(input, promptBuilder, mode) {
  const profile = input.profile ?? {};
  const apiKey = process.env.EPIC8_LLM_API_KEY ?? input.apiKey ?? "";
  if (!profile.baseUrl || !profile.model) return null;
  const pages = input.pages ?? [];
  const content = [{ type: "text", text: promptBuilder(input) }];
  for (const page of pages) {
    for (const image of page.images ?? []) {
      content.push({ type: "text", text: `Page ${page.pageIndex}, image ${image.assetId || image.fileName || ""}` });
      content.push({ type: "image_url", image_url: { url: await imageToDataUrl(image) } });
    }
  }
  if (!content.some((item) => item.type === "image_url")) return null;

  const endpoint = new URL(profile.baseUrl.replace(/\/$/, "") + "/chat/completions");
  const response = await fetch(endpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(apiKey ? { authorization: `Bearer ${apiKey}` } : {})
    },
    body: JSON.stringify({
      model: profile.model,
      temperature: profile.temperature ?? 0,
      response_format: profile.forceJson === false ? undefined : { type: "json_object" },
      messages: [
        { role: "system", content: "Return valid JSON only." },
        { role: "user", content }
      ]
    }),
    signal: AbortSignal.timeout(profile.timeoutMs ?? 120000)
  });
  if (!response.ok) throw new Error(`llm_http_${response.status}:${await response.text()}`);
  const payload = await response.json();
  const message = payload?.choices?.[0]?.message?.content;
  if (!message) throw new Error(`${mode}_empty_content`);
  const parsed = JSON.parse(message);
  return {
    ...parsed,
    metadata: {
      ...(parsed.metadata ?? {}),
      mode,
      source: "openai-compatible-vision",
      model: profile.model,
      usage: payload.usage ?? null
    }
  };
}

function validateTranscription(value) {
  if (!value || typeof value !== "object") throw new Error("transcription_not_object");
  if (typeof value.text !== "string") value.text = "";
  if (typeof value.confidence !== "number") value.confidence = value.text.trim() ? 0.6 : 0;
  if (!Array.isArray(value.warnings)) value.warnings = [];
  if (!value.text.trim()) value.warnings.push("empty-vision-transcription");
  if (!value.evidence || typeof value.evidence !== "object") value.evidence = {};
  return value;
}

function normalizeQuestionKey(key) {
  const number = Number(String(key ?? "").trim().replace(/^q/i, ""));
  if (!Number.isInteger(number) || number <= 0 || number > 200) return undefined;
  return String(number);
}

function normalizeAnswerValue(value) {
  if (Array.isArray(value)) {
    const items = value.map(normalizeAnswerValue).filter((item) => item !== undefined && String(item).trim());
    return items.length ? items : undefined;
  }
  const text = String(value ?? "").trim().replace(/^[."'`;,:[\]{}()\s]+|[."'`;,:[\]{}()\s]+$/g, "");
  if (!text) return undefined;
  const compact = text.toUpperCase().replace(/[\s_-]/g, "");
  if (compact === "NOTGIVEN") return "NOT GIVEN";
  if (["TRUE", "FALSE", "YES", "NO"].includes(text.toUpperCase())) return text.toUpperCase();
  if (/^[a-z]$/i.test(text)) return text.toUpperCase();
  return text;
}

function validateVisionAnswers(value) {
  if (!value || typeof value !== "object") throw new Error("vision_answer_not_object");
  const answers = {};
  for (const [key, answer] of Object.entries(value.answers ?? {})) {
    const number = normalizeQuestionKey(key);
    const normalized = normalizeAnswerValue(answer);
    if (number && normalized !== undefined) answers[number] = normalized;
  }
  if (!Object.keys(answers).length) throw new Error("vision_answer_answers_empty");
  value.answers = answers;
  if (typeof value.confidence !== "number") value.confidence = 0.6;
  if (!Array.isArray(value.warnings)) value.warnings = [];
  if (!Array.isArray(value.evidence)) value.evidence = [];
  return value;
}

function normalizeCloudKind(kind = "") {
  const normalized = String(kind).trim().toLowerCase().replace(/[-\s]/g, "_");
  if (["note_completion", "notes_completion", "note_completion_questions"].includes(normalized)) return "summary_completion";
  return allowedKinds.includes(normalized) ? normalized : "short_answer";
}

function validateCloudOutline(value) {
  if (!value || typeof value !== "object") throw new Error("cloud_outline_not_object");
  value.groups = Array.isArray(value.groups) ? value.groups.map((group) => {
    const next = group && typeof group === "object" ? { ...group } : {};
    next.kind = normalizeCloudKind(next.kind);
    if (!Array.isArray(next.range)) next.range = [];
    if (!Array.isArray(next.questionIds)) next.questionIds = [];
    if (typeof next.layoutHint !== "string") next.layoutHint = "";
    if (typeof next.notesText !== "string") next.notesText = "";
    if (typeof next.confidence !== "number") next.confidence = 0.5;
    if (!next.evidence || typeof next.evidence !== "object") next.evidence = {};
    if (!Array.isArray(next.evidence.quotes)) next.evidence.quotes = [];
    if (!Array.isArray(next.warnings)) next.warnings = [];
    return next;
  }) : [];
  const answerKey = {};
  for (const [key, answer] of Object.entries(value.answerKey ?? {})) {
    const number = normalizeQuestionKey(key);
    const normalized = normalizeAnswerValue(answer);
    if (number && normalized !== undefined) answerKey[number] = normalized;
  }
  value.answerKey = answerKey;
  if (typeof value.confidence !== "number") value.confidence = 0.5;
  if (!Array.isArray(value.warnings)) value.warnings = [];
  return value;
}

async function callOpenAiCompatible(input, mode) {
  const profile = input.profile ?? {};
  const apiKey = process.env.EPIC8_LLM_API_KEY ?? input.apiKey ?? "";
  if (!profile.baseUrl || !profile.model) return null;
  const endpoint = new URL(profile.baseUrl.replace(/\/$/, "") + "/chat/completions");
  const response = await fetch(endpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(apiKey ? { authorization: `Bearer ${apiKey}` } : {})
    },
    body: JSON.stringify({
      model: profile.model,
      temperature: profile.temperature ?? 0,
      response_format: profile.forceJson === false ? undefined : { type: "json_object" },
      messages: [
        { role: "system", content: "Return valid JSON only." },
        { role: "user", content: buildPrompt(input, mode) }
      ]
    }),
    signal: AbortSignal.timeout(profile.timeoutMs ?? 60000)
  });
  if (!response.ok) throw new Error(`llm_http_${response.status}:${await response.text()}`);
  const payload = await response.json();
  const content = payload?.choices?.[0]?.message?.content;
  if (!content) throw new Error("llm_empty_content");
  const parsed = JSON.parse(content);
  return {
    ...parsed,
    evidence: {
      ...(parsed.evidence ?? {}),
      mode,
      source: "openai-compatible",
      model: profile.model,
      usage: payload.usage ?? null
    }
  };
}

function validateSuggestion(value) {
  if (!value || typeof value !== "object") throw new Error("suggestion_not_object");
  if (!allowedKinds.includes(value.kind)) throw new Error(`invalid_kind:${value.kind}`);
  if (typeof value.confidence !== "number") value.confidence = 0.65;
  if (!Array.isArray(value.patch)) value.patch = [];
  if (!Array.isArray(value.warnings)) value.warnings = [];
  if (!Array.isArray(value.questions)) value.questions = [];
  if (!value.evidence || typeof value.evidence !== "object") value.evidence = {};
  if (!Array.isArray(value.evidence.sourceBlockIds)) value.evidence.sourceBlockIds = [];
  if (!Array.isArray(value.evidence.quotes)) value.evidence.quotes = [];
  return value;
}

async function main() {
  const [command, inputPath, outputPath] = process.argv.slice(2);
  if (!["classify_group", "extract_group", "test_profile", "transcribe_pdf_images", "extract_pdf_image_answers", "generate_pdf_reading_outline"].includes(command) || !inputPath || !outputPath) {
    console.error("usage: gateway.mjs <classify_group|extract_group|test_profile|transcribe_pdf_images|extract_pdf_image_answers|generate_pdf_reading_outline> <input.json> <output.json>");
    process.exit(2);
  }
  const input = JSON.parse(await fs.readFile(inputPath, "utf8"));
  if (command === "transcribe_pdf_images") {
    let transcription;
    try {
      transcription = await callVisionOpenAiCompatible(input);
    } catch (error) {
      transcription = transcriptionFallback(`provider-call-failed:${String(error.message ?? error)}`);
    }
    if (!transcription) transcription = transcriptionFallback("no-enabled-vision-provider-or-images");
    const output = validateTranscription(transcription);
    await fs.mkdir(path.dirname(outputPath), { recursive: true });
    await fs.writeFile(outputPath, JSON.stringify(output, null, 2));
    return;
  }
  if (command === "extract_pdf_image_answers") {
    const output = validateVisionAnswers(await callImageJsonOpenAiCompatible(input, buildVisionAnswerPrompt, command));
    await fs.mkdir(path.dirname(outputPath), { recursive: true });
    await fs.writeFile(outputPath, JSON.stringify(output, null, 2));
    return;
  }
  if (command === "generate_pdf_reading_outline") {
    const output = validateCloudOutline(await callImageJsonOpenAiCompatible(input, buildCloudOutlinePrompt, command));
    await fs.mkdir(path.dirname(outputPath), { recursive: true });
    await fs.writeFile(outputPath, JSON.stringify(output, null, 2));
    return;
  }
  const mode = command === "test_profile" ? "test_profile" : command;
  let suggestion;
  try {
    suggestion = await callOpenAiCompatible(input, mode);
  } catch (error) {
    suggestion = deterministicSuggestion(input, mode);
    suggestion.warnings.push(`provider-call-failed:${String(error.message ?? error)}`);
  }
  if (!suggestion) suggestion = deterministicSuggestion(input, mode);
  const output = validateSuggestion(suggestion);
  await fs.mkdir(path.dirname(outputPath), { recursive: true });
  await fs.writeFile(outputPath, JSON.stringify(output, null, 2));
}

main().catch((error) => {
  console.error(error?.stack ?? String(error));
  process.exit(1);
});
