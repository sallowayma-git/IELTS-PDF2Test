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
  const groupText = [
    ...(group.instruction ?? []),
    ...(group.questions ?? []).map((question) => question.prompt ?? "")
  ].join(" ");
  const kind = normalizeKind(groupText);
  const confidence = kind === group.kind ? 0.9 : 0.72;
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
      ...(confidence < 0.85 ? ["low-confidence-review-required"] : [])
    ],
    evidence: {
      mode,
      allowedKinds,
      source: "local-heuristic"
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
    `Task: ${mode}.`,
    `Allowed group kinds: ${allowedKinds.join(", ")}.`,
    `Group JSON: ${JSON.stringify(input.group ?? {})}`
  ].join("\n");
}

async function callOpenAiCompatible(input, mode) {
  const profile = input.profile ?? {};
  const apiKey = input.apiKey ?? "";
  if (!apiKey || !profile.baseUrl || !profile.model) return null;
  const endpoint = new URL(profile.baseUrl.replace(/\/$/, "") + "/chat/completions");
  const response = await fetch(endpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${apiKey}`
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
  return value;
}

async function main() {
  const [command, inputPath, outputPath] = process.argv.slice(2);
  if (!["classify_group", "extract_group", "test_profile"].includes(command) || !inputPath || !outputPath) {
    console.error("usage: gateway.mjs <classify_group|extract_group|test_profile> <input.json> <output.json>");
    process.exit(2);
  }
  const input = JSON.parse(await fs.readFile(inputPath, "utf8"));
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
