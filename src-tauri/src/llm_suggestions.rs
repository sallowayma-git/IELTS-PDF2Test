use crate::authoring_pipeline::{dynamic_interaction_for_kind, dynamic_template_for_kind};
use crate::util::{append_text, job_dir, read_json, read_json_opt, write_json};
use crate::{CommandResult, ImportJob};
use serde_json::{json, Value};
use std::{collections::HashSet, fs, path::Path};

fn sanitize_json_filename(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

pub(crate) fn llm_group_context(ir: &Value, group_id: &str) -> CommandResult<Value> {
    ir.get("groups")
        .and_then(Value::as_array)
        .and_then(|groups| {
            groups
                .iter()
                .find(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id))
        })
        .cloned()
        .ok_or_else(|| format!("group_not_found:{}", group_id))
}

fn normalize_llm_kind(text: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if lower.contains("true") && lower.contains("false") && lower.contains("not given") {
        "true_false_not_given"
    } else if lower.contains("yes") && lower.contains("no") && lower.contains("not given") {
        "yes_no_not_given"
    } else if lower.contains("complete the table")
        || lower.contains("table below")
        || lower.contains('|')
    {
        "table_completion"
    } else if lower.contains("choose") && lower.contains("letter") {
        "single_choice"
    } else if lower.contains("choose") && (lower.contains("two") || lower.contains("three")) {
        "multi_choice"
    } else if lower.contains("complete the summary") {
        "summary_completion"
    } else if lower.contains("complete the sentence") {
        "sentence_completion"
    } else {
        "short_answer"
    }
}

fn deterministic_llm_kind_for_group(group: &Value) -> &'static str {
    let mut text = String::new();
    for instruction in group
        .get("instruction")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        text.push_str(instruction);
        text.push(' ');
    }
    for prompt in group
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| question.get("prompt").and_then(Value::as_str))
    {
        text.push_str(prompt);
        text.push(' ');
    }
    normalize_llm_kind(&text)
}

pub(crate) fn deterministic_llm_output(group: &Value, mode: &str, warning: String) -> Value {
    let kind = deterministic_llm_kind_for_group(group);
    json!({
        "kind": kind,
        "confidence": 0.64,
        "patch": [
            {"op":"replace","path":"/kind","value":kind},
            {"op":"replace","path":"/layout/template","value": dynamic_template_for_kind(kind)}
        ],
        "questions": group.get("questions").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().map(|mut question| {
            if let Some(obj) = question.as_object_mut() {
                obj.insert("interaction".to_string(), dynamic_interaction_for_kind(kind));
            }
            question
        }).collect::<Vec<_>>(),
        "warnings": [warning, "low-confidence-review-required", "fallback-output-never-auto-applies"],
        "evidence": {"mode": mode, "source": "rust-local-fallback", "fallback": true}
    })
}

pub(crate) fn make_llm_input(
    profile: &Value,
    job: &ImportJob,
    group: &Value,
    profile_id: &str,
    mode: &str,
) -> Value {
    let repair_contract = json!({
        "schema": "Epic8LlmGroupRepairV1",
        "goal": "Classify or repair one IELTS Reading question group using only cited source evidence.",
        "allowedPatchOps": ["replace"],
        "allowedPatchPaths": ["/kind", "/layout/template"],
        "disallowedOutputs": ["html", "javascript", "readingExamSource", "finalExport"],
        "evidenceRequired": true,
        "mustUseOnlyGroupSourceBlocks": true,
        "highConfidenceAutoApplyThreshold": 0.85
    });
    let repair_context = json!({
        "sourceBlockIds": group.get("sourceBlockIds").cloned().unwrap_or_else(|| json!([])),
        "reviewWarnings": group.get("reviewWarnings").cloned().unwrap_or_else(|| json!([])),
        "classificationEvidence": group.get("classificationEvidence").cloned().unwrap_or_else(|| json!([])),
        "sectionEvidence": group.get("sectionEvidence").cloned().unwrap_or_else(|| json!([])),
        "continuationEdges": group.get("continuationEdges").cloned().unwrap_or_else(|| json!([])),
        "currentKind": group.get("kind").cloned().unwrap_or(Value::Null),
        "currentLayout": group.get("layout").cloned().unwrap_or(Value::Null),
        "allowOptionReuse": group.get("allowOptionReuse").cloned().unwrap_or(Value::Null),
        "requiresManualQuestionImport": group.get("requiresManualQuestionImport").cloned().unwrap_or(Value::Bool(false))
    });
    json!({
        "mode": mode,
        "job": {"jobId": job.job_id, "title": job.title, "category": job.category, "frequency": job.frequency, "tags": job.tags},
        "profile": {
            "profileId": profile_id,
            "provider": profile.get("provider").cloned().unwrap_or_else(|| json!("OpenAiCompatible")),
            "baseUrl": profile.get("baseUrl").cloned().unwrap_or_else(|| json!("")),
            "model": profile.get("model").cloned().unwrap_or_else(|| json!("")),
            "temperature": profile.get("temperature").cloned().unwrap_or_else(|| json!(0)),
            "timeoutMs": profile.get("timeoutMs").cloned().unwrap_or_else(|| json!(60000)),
            "forceJson": profile.get("forceJson").cloned().unwrap_or(Value::Bool(true))
        },
        "repairContract": repair_contract,
        "repairContext": repair_context,
        "group": group
    })
}

pub(crate) fn profile_payload(profile: &Value, profile_id: &str) -> Value {
    json!({
        "profileId": profile_id,
        "provider": profile.get("provider").cloned().unwrap_or_else(|| json!("OpenAiCompatible")),
        "baseUrl": profile.get("baseUrl").cloned().unwrap_or_else(|| json!("")),
        "model": profile.get("model").cloned().unwrap_or_else(|| json!("")),
        "temperature": profile.get("temperature").cloned().unwrap_or_else(|| json!(0)),
        "timeoutMs": profile.get("timeoutMs").cloned().unwrap_or_else(|| json!(120000)),
        "forceJson": profile.get("forceJson").cloned().unwrap_or(Value::Bool(true))
    })
}

pub(crate) fn make_vision_transcription_input(
    profile: &Value,
    job: &ImportJob,
    profile_id: &str,
    extraction: &Value,
) -> Value {
    json!({
        "mode": "transcribe_pdf_images",
        "job": {"jobId": job.job_id, "title": job.title, "category": job.category, "frequency": job.frequency, "tags": job.tags},
        "profile": profile_payload(profile, profile_id),
        "pages": extraction.get("pages").cloned().unwrap_or_else(|| json!([])),
        "extractionWarnings": extraction.get("warnings").cloned().unwrap_or_else(|| json!([]))
    })
}

pub(crate) fn make_vision_answer_extraction_input(
    profile: &Value,
    job: &ImportJob,
    profile_id: &str,
    extraction: &Value,
) -> Value {
    json!({
        "mode": "extract_pdf_image_answers",
        "job": {"jobId": job.job_id, "title": job.title, "category": job.category, "frequency": job.frequency, "tags": job.tags},
        "profile": profile_payload(profile, profile_id),
        "pages": extraction.get("pages").cloned().unwrap_or_else(|| json!([])),
        "extractionWarnings": extraction.get("warnings").cloned().unwrap_or_else(|| json!([])),
        "outputContract": {
            "schema": "PdfImageAnswerKeyV1",
            "jsonOnly": true,
            "shape": {
                "answers": {"questionNumber": "answer text or array of answer texts"},
                "confidence": 0.0,
                "warnings": [],
                "evidence": [{"questionNumber": "8", "pageIndex": 5, "quote": "short visible text"}]
            },
            "rules": [
                "Only extract answer keys visible in the supplied PDF images.",
                "Use question number strings without q prefix, for example \"8\".",
                "Do not invent missing answers; omit uncertain question numbers.",
                "Normalize TRUE/FALSE/NOT GIVEN/YES/NO and single-letter options to uppercase."
            ]
        }
    })
}

/// A4 分歧裁决的输入构造。
///
/// **刻意只给「证据面 + 待裁定的分歧清单」**，不给整份稿子、不给指令文本、不给选项库：
/// A4 的任务是「在三条已有结论里挑一条」，上下文给得越多，模型越容易去编第四条。
/// 产出契约同样写进输入（`outputContract`），让「模型该返回什么」和「我们会校验什么」
/// 是同一份文字——两份文字迟早会漂移。
pub(crate) fn make_adjudication_input(
    profile: &Value,
    job: &ImportJob,
    profile_id: &str,
    source: &crate::SourceFile,
    pdf_path: &Path,
    divergences: &[Value],
    repair_note: Option<&str>,
) -> Value {
    json!({
        "mode": "adjudicate_divergence",
        "job": {"jobId": job.job_id, "title": job.title},
        "profile": profile_payload(profile, profile_id),
        "sourceFile": {
            "fileId": source.file_id,
            "originalName": source.original_name,
            "fileType": source.file_type,
            "sha256": source.sha256,
            "sizeBytes": source.size_bytes
        },
        "pdfPath": pdf_path.to_string_lossy(),
        "divergences": divergences,
        "repairNote": repair_note,
        "outputContract": {
            "schema": "AdjudicationRulingsV1",
            "jsonOnly": true,
            "shape": {
                "rulings": [{
                    "decisionId": "echo back a decisionId you were given",
                    "chosen": "local | cloud | source | unresolved",
                    "value": {"kind": "text", "values": ["the chosen value, repeated exactly"], "normalization": "ielts_default"},
                    "confidence": 0.8,
                    "rationale": "one short sentence citing what in the original file decided it"
                }]
            },
            "rules": [
                "Answer only for the decisionId values listed in `divergences`; never invent an id.",
                "`chosen` picks among the three values already given (local / cloud / source).",
                "`value` must repeat the value of the chosen source exactly, byte for byte. Never invent a fourth value.",
                "If the original file does not settle the question, return chosen = \"unresolved\".",
                "`rationale` must not be empty.",
                "Return JSON only."
            ]
        }
    })
}

pub(crate) fn make_cloud_paper_generation_input(
    profile: &Value,
    job: &ImportJob,
    profile_id: &str,
    source: &crate::SourceFile,
    pdf_path: &Path,
    extraction: &Value,
) -> Value {
    json!({
        "mode": "generate_pdf_reading_outline",
        "job": {"jobId": job.job_id, "title": job.title, "category": job.category, "frequency": job.frequency, "tags": job.tags},
        "profile": profile_payload(profile, profile_id),
        "sourceFile": {
            "fileId": source.file_id,
            "originalName": source.original_name,
            "fileType": source.file_type,
            "sha256": source.sha256,
            "sizeBytes": source.size_bytes
        },
        "pdfPath": pdf_path.to_string_lossy(),
        "pages": extraction.get("pages").cloned().unwrap_or_else(|| json!([])),
        "extractionWarnings": extraction.get("warnings").cloned().unwrap_or_else(|| json!([])),
        "outputContract": {
            "schema": "CloudReadingOutlineV1",
            "jsonOnly": true,
            "shape": {
                "title": "paper title",
                "groups": [{
                    "kind": "true_false_not_given",
                    "range": [1, 5],
                    "layoutHint": "list",
                    "questionIds": ["q1", "q2", "q3", "q4", "q5"],
                    "instructionsText": "Do the following statements agree with the claims of the writer?",
                    "stimulusText": "The passage text this group depends on (for completion groups: the notes/table/diagram text).",
                    "optionBank": {"options": [{"label": "A", "text": "TRUE"}, {"label": "B", "text": "FALSE"}, {"label": "C", "text": "NOT GIVEN"}], "allowReuse": true},
                    "notesText": "Optional notes for completion groups; empty for choice groups.",
                    "confidence": 0.9,
                    "evidence": {"quotes": [{"pageIndex": 1, "text": "short visible source excerpt"}]},
                    "slots": [
                        {"questionNumber": 1, "prompt": "Full transcribed question text for Q1.", "answer": "TRUE", "evidence": [{"pageIndex": 1, "quote": "visible excerpt supporting Q1"}]},
                        {"questionNumber": 2, "prompt": "Full transcribed question text for Q2.", "answer": "FALSE", "evidence": [{"pageIndex": 1, "quote": "visible excerpt supporting Q2"}]}
                    ]
                }],
                "answerKey": {"1": "TRUE", "2": "FALSE", "3": "NOT GIVEN", "4": "TRUE", "5": "FALSE"},
                "confidence": 0.9,
                "warnings": []
            },
            "rules": [
                "Return FULL recognition content for comparison, not an outline. An outline alone is not acceptable.",
                "Transcribe every question's FULL prompt text into slots[].prompt; do not abbreviate or summarize questions.",
                "Transcribe every option bank and every option label and its text into optionBank.options.",
                "Transcribe ALL passage / notes / table / diagram text the group depends on into stimulusText (and notesText for completion groups).",
                "Provide the answer for EVERY question: prefer slots[].answer per question; you may also repeat them in answerKey keyed by question number as a string or string array.",
                "Every group must include evidence.quotes with one quote copied from visible PDF text (pageIndex>0, non-empty text). If evidence is missing, lower group confidence below 0.75.",
                "Do not invent passage facts or answers; omit uncertain question numbers rather than guessing.",
                "Question kinds must use the local group-kind enum names: single_choice, multi_choice, true_false_not_given, yes_no_not_given, matching, heading_matching, matching_information, classification, summary_completion, table_completion, diagram_completion, short_answer, sentence_completion.",
                "range must be a 2-element array [start, end] with start>0 and end>=start; questionIds must be unique non-empty strings (length = end-start+1), one per question in the range.",
                "For notes completion groups (source says Complete the notes below, notes, or uses blank markers such as 8……… or 8 ______), keep the whole range as one completion group: set layoutHint to inline_completion, include qN ids for every blank, and copy the continuous notes text into notesText. Do not rewrite into a list of independent short-answer items."
            ]
        }
    })
}

pub(crate) fn save_llm_suggestion(
    root: &Path,
    job_id: &str,
    suggestion: &Value,
) -> CommandResult<()> {
    let dir = job_dir(root, job_id);
    let group_id = suggestion
        .get("groupId")
        .and_then(Value::as_str)
        .map(sanitize_json_filename)
        .unwrap_or_else(|| "unknown-group".to_string());
    let suggestion_id = suggestion
        .get("suggestionId")
        .and_then(Value::as_str)
        .map(sanitize_json_filename)
        .unwrap_or_else(|| "unknown-suggestion".to_string());
    write_json(
        &dir.join("llm-suggestions")
            .join(format!("{}--{}.json", group_id, suggestion_id)),
        suggestion,
    )?;
    write_json(&dir.join("llm-last-suggestion.json"), suggestion)?;
    append_text(
        &dir.join("llm-calls.jsonl"),
        &format!(
            "{}\n",
            serde_json::to_string(suggestion).map_err(|error| error.to_string())?
        ),
    )
}

pub(crate) fn load_llm_suggestions(root: &Path, job_id: &str) -> CommandResult<Vec<Value>> {
    let mut items = Vec::new();
    let job_path = job_dir(root, job_id);
    let dir = job_path.join("llm-suggestions");
    if dir.exists() {
        for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                items.push(read_json::<Value>(&path)?);
            }
        }
    }
    if items.is_empty() {
        if let Some(last) = read_json_opt(&job_path.join("llm-last-suggestion.json"))? {
            items.push(last);
        }
    }
    items.sort_by(|left, right| {
        right
            .get("createdAt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                left.get("createdAt")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
    });
    Ok(items)
}

fn is_allowed_llm_group_kind(kind: &str) -> bool {
    matches!(
        kind,
        "single_choice"
            | "multi_choice"
            | "true_false_not_given"
            | "yes_no_not_given"
            | "matching"
            | "heading_matching"
            | "matching_information"
            | "classification"
            | "summary_completion"
            | "table_completion"
            | "diagram_completion"
            | "short_answer"
            | "sentence_completion"
    )
}

fn is_allowed_llm_interaction_type(kind: &str) -> bool {
    matches!(
        kind,
        "radio"
            | "checkbox"
            | "text"
            | "textarea"
            | "select"
            | "dragdrop"
            | "table"
            | "diagram"
            | "matching"
    )
}

fn json_string_set(value: Option<&Value>) -> HashSet<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|item| !item.trim().is_empty())
        .map(ToString::to_string)
        .collect()
}

fn group_by_suggestion<'a>(ir: &'a Value, suggestion: &Value) -> Option<&'a Value> {
    let group_id = suggestion.get("groupId").and_then(Value::as_str)?;
    ir.get("groups")
        .and_then(Value::as_array)?
        .iter()
        .find(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id))
}

fn normalized_quote_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Hyphen-tolerant variant used as a fallback match: model quotes often
/// reproduce line-broken words as "stencil- ling" while the source block has
/// "stencilling". Removing hyphens on both sides tolerates that drift.
fn normalized_quote_text_without_hyphens(value: &str) -> String {
    normalized_quote_text(&value.replace('-', "")).replace(' ', "")
}

/// Verify that suggestion evidence quotes actually appear in the referenced
/// source blocks. `block_texts` maps blockId -> block text. Blocks missing
/// from the map are skipped (the source may already be minimized); a quote
/// whose normalized text is not contained in its block is reported so the
/// suggestion cannot auto-apply on fabricated evidence.
pub(crate) fn llm_suggestion_quote_mismatches(
    suggestion: &Value,
    block_texts: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let mut issues = Vec::<String>::new();
    let quotes = suggestion
        .get("evidence")
        .unwrap_or(&Value::Null)
        .get("quotes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for quote in quotes {
        let block_id = quote
            .get("blockId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = quote
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if text.trim().is_empty() || block_id.is_empty() {
            continue;
        }
        let Some(source_text) = block_texts.get(block_id) else {
            // With a non-empty source map a missing referenced block is a
            // suspicious signal (wrong/case-mangled blockId), not a reason to
            // silently skip verification.
            if !block_texts.is_empty() {
                issues.push(format!("evidence_quote_block_missing:{}", block_id));
            }
            continue;
        };
        let needle = normalized_quote_text(text);
        let haystack = normalized_quote_text(source_text);
        let matched = !needle.is_empty() && haystack.contains(&needle) || {
            let loose_needle = normalized_quote_text_without_hyphens(text);
            let loose_haystack = normalized_quote_text_without_hyphens(source_text);
            !loose_needle.is_empty() && loose_haystack.contains(&loose_needle)
        };
        if !matched {
            issues.push(format!("evidence_quote_not_in_source:{}", block_id));
        }
    }
    issues.sort();
    issues.dedup();
    issues
}

pub(crate) fn llm_suggestion_auto_apply_issues(
    ir: &Value,
    suggestion: &Value,
    selected_paths: &[String],
) -> Vec<String> {
    let mut issues = Vec::<String>::new();
    let confidence = suggestion
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if confidence < 0.85 {
        issues.push("confidence_below_auto_apply_threshold".to_string());
    }

    let selected = selected_paths
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for path in &selected {
        if !matches!(*path, "kind" | "layout" | "questions") {
            issues.push(format!("unsupported_selected_path:{}", path));
        }
    }

    let Some(group) = group_by_suggestion(ir, suggestion) else {
        issues.push("suggestion_group_not_found".to_string());
        return issues;
    };

    let group_source_ids = json_string_set(group.get("sourceBlockIds"));
    if group_source_ids.is_empty() {
        issues.push("group_source_blocks_missing".to_string());
    }
    let question_ids = group
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| question.get("id").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<HashSet<_>>();

    let suggested_kind = suggestion.get("kind").and_then(Value::as_str);
    if let Some(kind) = suggested_kind {
        if !is_allowed_llm_group_kind(kind) {
            issues.push(format!("invalid_kind:{}", kind));
        }
    }

    let Some(patches) = suggestion.get("patch").and_then(Value::as_array) else {
        issues.push("patch_array_missing".to_string());
        return issues;
    };
    for patch in patches {
        let op = patch.get("op").and_then(Value::as_str).unwrap_or_default();
        let path = patch
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if op != "replace" {
            issues.push(format!("unsupported_patch_op:{}", op));
            continue;
        }
        match path {
            "/kind" => {
                if !selected.contains("kind") {
                    continue;
                }
                let kind = patch
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !is_allowed_llm_group_kind(kind) {
                    issues.push(format!("invalid_patch_kind:{}", kind));
                }
            }
            "/layout/template" => {
                if !(selected.contains("layout") || selected.contains("kind")) {
                    continue;
                }
                if patch
                    .get("value")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    issues.push("invalid_layout_template".to_string());
                }
            }
            other if other.starts_with("/questions/") => {
                issues.push(format!("question_patch_must_use_questions_array:{}", other));
            }
            other => issues.push(format!("unsupported_patch_path:{}", other)),
        }
    }

    if selected.contains("questions") {
        let Some(questions) = suggestion.get("questions").and_then(Value::as_array) else {
            issues.push("questions_array_missing".to_string());
            return issues;
        };
        for question in questions {
            let qid = question
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !question_ids.contains(qid) {
                issues.push(format!("unknown_question_id:{}", qid));
            }
            if let Some(prompt) = question.get("prompt").and_then(Value::as_str) {
                if prompt.trim().is_empty() {
                    issues.push(format!("empty_question_prompt:{}", qid));
                }
            }
            if let Some(interaction) = question.get("interaction") {
                let interaction_type = interaction
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !is_allowed_llm_interaction_type(interaction_type) {
                    issues.push(format!(
                        "invalid_interaction_type:{}:{}",
                        qid, interaction_type
                    ));
                }
                if matches!(interaction_type, "radio" | "checkbox" | "select") {
                    let options = interaction
                        .get("options")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|option| !option.trim().is_empty())
                        .count();
                    if options == 0 {
                        issues.push(format!("interaction_options_missing:{}", qid));
                    }
                }
            }
        }
    }

    let evidence = suggestion.get("evidence").unwrap_or(&Value::Null);
    if evidence.get("fallback").and_then(Value::as_bool) == Some(true) {
        issues.push("fallback_evidence_never_auto_applies".to_string());
    }
    let evidence_source = evidence
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if evidence_source.contains("fallback") || evidence_source.contains("heuristic") {
        issues.push(format!("non_provider_evidence_source:{}", evidence_source));
    }

    let evidence_block_ids = json_string_set(
        evidence
            .get("sourceBlockIds")
            .or_else(|| evidence.get("blockIds")),
    );
    if evidence_block_ids.is_empty() {
        issues.push("evidence_source_block_ids_missing".to_string());
    }
    for block_id in &evidence_block_ids {
        if !group_source_ids.contains(block_id) {
            issues.push(format!("evidence_block_not_in_group:{}", block_id));
        }
    }

    let quotes = evidence
        .get("quotes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if quotes.is_empty() {
        issues.push("evidence_quotes_missing".to_string());
    }
    for quote in quotes {
        let block_id = quote
            .get("blockId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = quote
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !group_source_ids.contains(block_id) {
            issues.push(format!("evidence_quote_block_not_in_group:{}", block_id));
        }
        if text.trim().is_empty() {
            issues.push(format!("evidence_quote_text_missing:{}", block_id));
        }
    }

    for warning in suggestion
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if warning.contains("fallback-output-never-auto-applies")
            || warning.contains("deterministic-local-fallback")
        {
            issues.push(format!("blocking_warning:{}", warning));
        }
    }

    issues.sort();
    issues.dedup();
    issues
}

pub(crate) fn apply_suggestion_to_authoring(
    ir: &mut Value,
    suggestion: &Value,
    selected_paths: &[String],
) -> CommandResult<()> {
    let group_id = suggestion
        .get("groupId")
        .and_then(Value::as_str)
        .ok_or_else(|| "suggestion_group_missing".to_string())?;
    let selected = selected_paths
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let Some(groups) = ir.get_mut("groups").and_then(Value::as_array_mut) else {
        return Err("authoring_groups_missing".to_string());
    };
    let group = groups
        .iter_mut()
        .find(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id))
        .ok_or_else(|| format!("group_not_found:{}", group_id))?;

    if let Some(patches) = suggestion.get("patch").and_then(Value::as_array) {
        for patch in patches {
            let op = patch.get("op").and_then(Value::as_str).unwrap_or_default();
            let path = patch
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let value = patch.get("value").cloned().unwrap_or(Value::Null);
            if op != "replace" {
                continue;
            }
            match path {
                "/kind" if selected.contains("kind") => {
                    if let Some(obj) = group.as_object_mut() {
                        obj.insert("kind".to_string(), value);
                    }
                }
                "/layout/template" if selected.contains("layout") || selected.contains("kind") => {
                    if let Some(layout) = group.get_mut("layout").and_then(Value::as_object_mut) {
                        layout.insert("template".to_string(), value);
                    }
                }
                _ => {}
            }
        }
    }

    if selected.contains("questions") {
        if let (Some(suggested), Some(existing)) = (
            suggestion.get("questions").and_then(Value::as_array),
            group.get_mut("questions").and_then(Value::as_array_mut),
        ) {
            for suggested_question in suggested {
                if let Some(qid) = suggested_question.get("id").and_then(Value::as_str) {
                    if let Some(current) = existing
                        .iter_mut()
                        .find(|question| question.get("id").and_then(Value::as_str) == Some(qid))
                    {
                        if let Some(prompt) =
                            suggested_question.get("prompt").and_then(Value::as_str)
                        {
                            current["prompt"] = json!(prompt);
                        }
                        if let Some(interaction) = suggested_question.get("interaction") {
                            current["interaction"] = interaction.clone();
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
