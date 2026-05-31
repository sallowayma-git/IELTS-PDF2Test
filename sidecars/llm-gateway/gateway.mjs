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

function validateTranscription(value) {
  if (!value || typeof value !== "object") throw new Error("transcription_not_object");
  if (typeof value.text !== "string") value.text = "";
  if (typeof value.confidence !== "number") value.confidence = value.text.trim() ? 0.6 : 0;
  if (!Array.isArray(value.warnings)) value.warnings = [];
  if (!value.text.trim()) value.warnings.push("empty-vision-transcription");
  if (!value.evidence || typeof value.evidence !== "object") value.evidence = {};
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
  if (!["classify_group", "extract_group", "test_profile", "transcribe_pdf_images"].includes(command) || !inputPath || !outputPath) {
    console.error("usage: gateway.mjs <classify_group|extract_group|test_profile|transcribe_pdf_images> <input.json> <output.json>");
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
