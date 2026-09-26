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
        "forceJson": profile.get("forceJson").cloned().unwrap_or(Value::Bool(true)),
        // Optional output-token cap; the gateway falls back to its default when null.
        "maxOutputTokens": profile.get("maxOutputTokens").cloned().unwrap_or(Value::Null)
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

/// Output contract of `extract_pdf_image_answers`. The shape example must pass
/// `validate_vision_answer_output` (pinned by a test in `llm_gateway`).
pub(crate) fn vision_answer_output_contract() -> Value {
    json!({
        "schema": "PdfImageAnswerKeyV1",
        "jsonOnly": true,
        "shape": {
            "answers": {"8": "answer text", "9": ["answer", "alternative"]},
            "confidence": 0.0,
            "warnings": [],
            "evidence": [{"questionNumber": "8", "pageIndex": 5, "quote": "short visible text"}]
        },
        "rules": [
            "The supplied pages were selected from scanned/image-only answer-page candidates. Extract only answer keys visibly printed on those pages; do not infer answers from the question paper.",
            "answers keys are question number strings without q prefix, for example \"8\"; values are an answer string or an array of accepted answer strings.",
            "answers must contain at least one entry and evidence must contain at least one item {questionNumber, pageIndex >= 1, non-empty quote}. A reply without any answer is rejected and recorded as \"no answer key found\".",
            "Do not invent missing answers; omit uncertain question numbers.",
            "Normalize TRUE/FALSE/NOT GIVEN/YES/NO and single-letter options to uppercase."
        ]
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
        "outputContract": vision_answer_output_contract()
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

/// A3 原文件核验的输入构造。
///
/// 与 [`make_adjudication_input`] 的差别是任务本身：裁决是「在三条已有结论里挑一条」，
/// 核验是「回原文件查这个值对不对」。所以这里给的是**待核验槽位 + 本地已有值**，
/// 并明确要求每条断言带原文引用。
///
/// **不给整份题稿**：核验的判定对象就是「原文支持不支持这个值」，
/// 把题干、选项库、其他槽位一并塞进去只会让模型跑去做别的判断。
pub(crate) fn make_source_verification_input(
    profile: &Value,
    job: &ImportJob,
    profile_id: &str,
    source: &crate::SourceFile,
    pdf_path: &Path,
    slots: &[Value],
    repair_note: Option<&str>,
) -> Value {
    json!({
        "mode": "verify_source_answers",
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
        "slots": slots,
        "repairNote": repair_note,
        "outputContract": {
            "schema": "SourceVerificationV1",
            "jsonOnly": true,
            "shape": {
                "findings": [{
                    "slotId": "echo back a slotId you were given",
                    "questionNumber": 14,
                    "verdict": "confirmed | contradicted | not_verifiable",
                    "quote": "exact text copied from the original file that decides it (required unless not_verifiable)",
                    "pageIndex": 3,
                    "observedValue": {"kind": "text", "values": ["the value the file actually gives"], "normalization": "ielts_default"},
                    "confidence": 0.9
                }]
            },
            "rules": [
                "Answer only for the slotId values listed in `slots`; never invent an id.",
                "`verdict` must be exactly one of: confirmed, contradicted, not_verifiable.",
                "Use \"confirmed\" only when the file explicitly supports `localValue`.",
                "Use \"contradicted\" only when the file explicitly gives a different value, and then `observedValue` is required.",
                "Both \"confirmed\" and \"contradicted\" must carry a non-empty `quote` and the 1-based `pageIndex` it appears on.",
                "Never invent a value that is not in the file. If you cannot read it, return \"not_verifiable\".",
                "Return JSON only."
            ]
        }
    })
}

/// Output contract of `generate_pdf_reading_outline`. The shape example must
/// pass `validate_cloud_outline_output` (pinned by a test in `llm_gateway`).
pub(crate) fn cloud_outline_output_contract() -> Value {
    json!({
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
                "notesText": "",
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
            "Required on every group: kind, range, layoutHint (inline_completion, table or list; use list when neither of the others applies), questionIds, notesText (the notes text for completion groups, otherwise an empty string \"\"), confidence, and evidence.quotes.",
            "Every group must include at least one evidence.quotes item copied from visible PDF text (pageIndex >= 1, non-empty text). A group without a quote is rejected: if you cannot quote a group, leave that group out and say so in warnings.",
            "Do not invent passage facts or answers; omit uncertain question numbers from answerKey rather than guessing.",
            "Question kinds must use the local group-kind enum names: single_choice, multi_choice, true_false_not_given, yes_no_not_given, matching, heading_matching, matching_information, classification, summary_completion, table_completion, diagram_completion, short_answer, sentence_completion.",
            "range must be a 2-element array [start, end] with start>0 and end>=start; questionIds must be unique non-empty strings (length = end-start+1), one per question in the range.",
            "For notes completion groups (source says Complete the notes below, notes, or uses blank markers such as 8……… or 8 ______), keep the whole range as one completion group: set layoutHint to inline_completion, include qN ids for every blank, and copy the continuous notes text into notesText. Do not rewrite into a list of independent short-answer items."
        ]
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
        "outputContract": cloud_outline_output_contract()
    })
}

/// Normalise a modality label to the two values the candidate/repair prompts
/// know. Anything that is not `listening` is treated as `reading` (the default).
pub(crate) fn candidate_modality(modality: &str) -> &'static str {
    if modality.trim().eq_ignore_ascii_case("listening") {
        "listening"
    } else {
        "reading"
    }
}

/// Output contract of `generate_authoring_candidate`.
///
/// The shape example is the model's template: a test in `llm_gateway` pins that
/// it passes `validate_authoring_candidate_output` AND `normalize_cloud_authoring`
/// + serde, so a model that copies it faithfully is never rejected.
///
/// There is deliberately no `passage` block: it was the largest part of the
/// output and no stage reads or compares it (`candidate_differences` compares
/// task groups only). Listening papers declare their parts in `listeningParts`.
pub(crate) fn authoring_candidate_output_contract(modality: &str) -> Value {
    let modality = candidate_modality(modality);
    let mut shape = json!({
        "taskGroups": [{
            "taskId": "cloud-tg-1",
            "displayRange": {"kind": "range", "start": 1, "end": 5},
            "taskType": "true_false_not_given",
            "instructions": [{"type": "paragraph", "id": "cloud-tg-1-instr", "children": [{"type": "text", "id": "cloud-tg-1-instr-text", "text": "full instruction text"}]}],
            "stimulus": [{"type": "paragraph", "id": "cloud-tg-1-stim", "children": [{"type": "text", "id": "cloud-tg-1-stim-text", "text": "full notes / table / diagram / form text"}]}],
            "optionBank": {
                "optionBankId": "cloud-tg-1-bank",
                "scope": "task_group",
                "options": [{"optionId": "cloud-opt-a", "label": "A", "content": [{"type": "text", "id": "cloud-opt-a-text", "text": "full option text"}]}],
                "allowReuse": false
            },
            "responseGroups": [{
                "responseGroupId": "cloud-rg-1",
                "kind": "choice",
                "prompt": [{"type": "paragraph", "id": "cloud-q1-prompt", "children": [{"type": "text", "id": "cloud-q1-prompt-text", "text": "full question prompt text"}]}],
                "slotIds": ["cloud-q1"],
                "optionBankRef": "cloud-tg-1-bank",
                "cardinality": {"min": 1, "max": 1, "exact": 1},
                "assignment": "per_slot",
                "scoringPolicy": "per_slot_ielts_normalized",
                "duplicatePolicy": "reject_submission",
                "allowOptionReuse": false
            }]
        }],
        "answerSlots": {
            "cloud-q1": {
                "slotId": "cloud-q1",
                "questionNumber": 1,
                "displayLabel": "1",
                "hostNodeId": "cloud-q1-prompt",
                "hostType": "prompt",
                "interaction": "radio",
                "participation": "scoring",
                "constraints": {"acceptedOptionLabels": ["A", "B", "C"]},
                "confidence": 0.9
            }
        },
        "answerKey": {
            "cloud-q1": {"kind": "option", "labels": ["B"], "assignment": "per_slot"}
        },
        "unresolvedRegions": [{
            "sourceFileId": "the source fileId you were given",
            "pageIndex": 3,
            "reason": "page_image_unavailable",
            "detail": "what could not be read"
        }],
        "sourceCoverageNotes": ["anything about source coverage you could not verify"],
        "warnings": []
    });
    let mut rules = vec![
        "Return FULL recognition content, not an outline and not a summary. This is used as a complete candidate draft.".to_string(),
        "Top-level keys: taskGroups, answerSlots, answerKey, unresolvedRegions, sourceCoverageNotes, warnings. Nothing else.".to_string(),
        "Transcribe every question's FULL prompt text. Do not abbreviate, summarise or paraphrase any question.".to_string(),
        "Transcribe every option's label and FULL option text. Keep the option bank per task group.".to_string(),
        "Do NOT transcribe the reading passage / audio script body. Transcribe only what the task groups show: instructions, and the notes / table / diagram / flow-chart / form / summary text a group depends on (into stimulus).".to_string(),
        "Give every question an answerKey entry. If the original file does not provide an answer, use {\"kind\": \"unresolved\"} — never invent an answer.".to_string(),
        "answerKey values: {\"kind\":\"text\",\"values\":[\"...\"]} with at least one value, {\"kind\":\"option\",\"labels\":[\"A\"],\"assignment\":\"per_slot\"} with at least one label, or {\"kind\":\"unresolved\"}.".to_string(),
        "Use temporary ids only (for example cloud-tg-1, cloud-q14, cloud-opt-a). NEVER copy ids from any other document and never use a real database id.".to_string(),
        "Required on every taskGroup: taskId, displayRange, taskType, instructions (array), responseGroups (array). displayRange is {\"kind\":\"range\",\"start\":1,\"end\":5} or {\"kind\":\"set\",\"values\":[1,3,5]}.".to_string(),
        "Required on every optionBank: optionBankId, scope, options, allowReuse; every option needs optionId, label and content (array).".to_string(),
        "Required on every responseGroup: responseGroupId, kind, slotIds (non-empty), cardinality {min, max}, assignment, scoringPolicy, duplicatePolicy, allowOptionReuse (true/false).".to_string(),
        "Required on every answerSlot: slotId, questionNumber, displayLabel, hostType, interaction, participation, confidence (0..1). Every slotIds entry in a responseGroup MUST appear as a key in answerSlots. Every hostNodeId MUST be an id you defined in this same output.".to_string(),
        "Every content node needs type and id; text nodes need text; paragraph / heading / list_item / table_cell nodes need children.".to_string(),
        "Do NOT output jobId, schemaVersion, exam, quality, audit, reviewState, sourceDocumentId, provenanceStatus, pageIndex hashes or any publish/verification flag. The backend fills all of those.".to_string(),
        "sourceAnchors are optional. If you provide them, use only {\"sourceFileId\": \"...\", \"pageIndex\": 1, \"nodeIds\": []}; pageIndex is 1-based. Never invent hashes or file paths.".to_string(),
        "Report anything you could not read in unresolvedRegions (sourceFileId, 1-based pageIndex, reason, detail — all required) and anything you could not verify in sourceCoverageNotes. Do not hide gaps with empty arrays.".to_string(),
        "Use only the enum values listed in outputContract.enums.".to_string(),
        "Return JSON only. No Markdown, no explanations, no code fences.".to_string(),
    ];
    if modality == "listening" {
        shape["listeningParts"] = json!([{
            "displayLabel": "Part 1",
            "expectedQuestionNumbers": [1, 2, 3, 4, 5],
            "taskIds": ["cloud-tg-1"]
        }]);
        rules.insert(
            2,
            "This is an IELTS Listening question paper. It is organised in Parts (sections) 1-4; list them in listeningParts with displayLabel, expectedQuestionNumbers and the taskIds of the task groups printed under that Part. Every taskIds entry MUST be a taskId you defined.".to_string(),
        );
    }
    json!({
        "schema": "CloudAuthoringCandidateV1",
        "jsonOnly": true,
        "modality": modality,
        "shape": shape,
        "enums": {
            "taskType": ["single_choice", "multiple_choice", "true_false_not_given", "yes_no_not_given", "matching_information", "matching_headings", "matching_features", "matching_sentence_endings", "classification", "sentence_completion", "summary_completion", "note_completion", "table_completion", "form_completion", "flowchart_completion", "diagram_label_completion", "plan_map_label_completion", "short_answer"],
            "displayRange.kind": ["range", "set"],
            "optionBank.scope": ["task_group", "response_group"],
            "responseGroup.kind": ["choice", "text_entry", "matching", "diagram_hotspot", "composite"],
            "responseGroup.assignment": ["per_slot", "unordered_set", "ordered_slots"],
            "responseGroup.scoringPolicy": ["per_slot_binary", "per_slot_ielts_normalized", "exact_set", "all_or_nothing"],
            "responseGroup.duplicatePolicy": ["reject_submission", "ignore_duplicates"],
            "answerSlot.hostType": ["prompt", "paragraph", "table_cell", "figure_hotspot", "flow_step"],
            "answerSlot.interaction": ["radio", "checkbox", "text", "select", "dragdrop", "hotspot"],
            "answerSlot.participation": ["scoring", "example", "non_scoring"],
            "answerKey.kind": ["text", "option", "unresolved"],
            "answerKey.assignment": ["per_slot", "unordered_set", "ordered"],
            "answerKey.normalization": ["ielts_default", "exact"]
        },
        "rules": rules
    })
}

/// 云端**完整候选**识别的输入。
///
/// 与 [`make_cloud_paper_generation_input`] 的关键区别：后者要的是「比对用大纲」，
/// 本函数要的是**可直接渲染的完整稿件**（题组、富内容题干、选项库、作答位置、答案）。
/// 模型只负责内容与**临时引用**；job/source 身份、质量、审计、稳定 ID 一律由后端生成。
///
/// `modality` 是模态钩子（`reading` 缺省；`listening` 让契约与 prompt 按 Listening 措辞，
/// 并要求 `listeningParts`）。
pub(crate) fn make_cloud_authoring_candidate_input(
    profile: &Value,
    job: &ImportJob,
    profile_id: &str,
    source: &crate::SourceFile,
    pdf_path: &Path,
    extraction: &Value,
    modality: &str,
) -> Value {
    json!({
        "mode": "generate_authoring_candidate",
        "modality": candidate_modality(modality),
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
        "outputContract": authoring_candidate_output_contract(modality)
    })
}

/// 修复输入里最多保留的观察条数（最近的优先）。
pub(crate) const MAX_REPAIR_OBSERVATIONS: usize = 12;

/// 修复回合的输入：上下文 + 最近的 observation（有界，见 [`MAX_REPAIR_OBSERVATIONS`]）。
///
/// 采用**应用层 JSON 工具消息**（模型输出 JSON → Rust 分发 → 结果回传），而不是供应商
/// native tools：现有网关只读 `message.content`，本轮不改造 SDK，也不实现第二套协议。
/// `modality` 是模态钩子（`reading` 缺省）。
pub(crate) fn make_repair_authoring_step_input(
    profile: &Value,
    job: &ImportJob,
    profile_id: &str,
    source: &crate::SourceFile,
    pdf_path: &Path,
    context: &Value,
    observations: &[Value],
    modality: &str,
) -> Value {
    // 观察历史有界：只带最近的若干条，省略多少如实写明。多轮修复不该让 prompt 无限增长，
    // 而最近的观察（上一批被拒的具体原因、刚写入的新版本）才是模型下一步需要的。
    let omitted = observations.len().saturating_sub(MAX_REPAIR_OBSERVATIONS);
    let recent = &observations[omitted..];
    let mut input = json!({
        "mode": "repair_authoring_step",
        "modality": candidate_modality(modality),
        "job": {"jobId": job.job_id, "title": job.title},
        "profile": profile_payload(profile, profile_id),
        "sourceFile": {
            "fileId": source.file_id,
            "originalName": source.original_name,
            "fileType": source.file_type,
            "sha256": source.sha256
        },
        "context": context,
        "observations": recent,
        "omittedObservationCount": omitted,
        "tools": repair_tools_table(&source.file_id),
        // 唯一真源：分发器真正放行的 op 清单。手抄一份迟早漂移。
        "allowedOps": crate::cloud_repair::tools::MODEL_ALLOWED_OPS,
        "rules": repair_tool_rules(context)
    });
    let packet_mode = context.get("contextMode").and_then(Value::as_str) == Some("packets");
    let attach_full_source = context.get("attachFullSource").and_then(Value::as_bool) == Some(true);
    // L0-L2 只传包证据，连本机 PDF 路径也不进入 repair input；只有 L3 后端确实要附整份
    // PDF 时才把路径交给网关。legacy 则保留原有路径，作为 L3 与回归对照。
    if !packet_mode || attach_full_source {
        input["pdfPath"] = json!(pdf_path.to_string_lossy().to_string());
    }
    input
}

/// 修复工具表（**给模型看的信封形状**，唯一真源）。
///
/// 键集合必须与 [`crate::schema::cloud_repair_v1::CLOUD_REPAIR_TOOLS`] 一致：提示词里
/// 写了一个工具、分发器不认（或反过来），是这类循环最典型的漂移，而且后果是「模型
/// 永远改不对」——它照着表交，每次都被拒。有测试逐项核对。
///
/// 抓取类工具（`search_source` / `read_page_region` / `read_passage` / `read_candidate`）
/// 与 `report_insufficient_context` 是「上下文不够时」的正式出口：包里的范围是**故意**
/// 收窄的，模型必须能要回它真正需要的那一块，而不是凭印象下结论。
///
/// `main_source_file_id`：示例里的 evidence sourceFileId **必须**注入当前作业真实的
/// 主试卷 id（P13-Q）。模型最先看到、也最常照抄的就是示例；示例里写死一个占位 id
/// （如 `answer-source`——既不是主试卷、也不是作业里任何真实文件），照抄的引文就会
/// 被当「编造来源」整批拒绝，或在「未知 id 放行」的旧语义下整批绕过核验。两种结果
/// 都不可接受：示例是 prompt 的一部分，属于「修 prompt 不放宽校验器」的范畴。
pub(crate) fn repair_tools_table(main_source_file_id: &str) -> Value {
    json!({
        "read_draft": {
            "purpose": "Read the CURRENT draft (authoritative canonical) for specific task groups. In packet mode, taskGroupIds or questionNumbers from this packet are REQUIRED; an empty or out-of-packet selector is rejected.",
            "arguments": {"taskGroupIds": ["task id from this packet"], "questionNumbers": [1, 2]}
        },
        "read_source": {
            "purpose": "Read the ORIGINAL FILE evidence. You cannot choose a path. \
    In packet mode a page range or a quote is REQUIRED and one call returns at most 3 pages.",
            "arguments": {"pageIndex": 1, "pageTo": 2, "quote": "optional exact quote to locate"}
        },
        "search_source": {
            "purpose": "Search the extracted text layer of the ORIGINAL FILE. Returns matching line ids with their page and a couple of context lines.",
            "arguments": {"query": "a phrase you remember from the file"}
        },
        "read_page_region": {
            "purpose": "Get the rendered image of one page (optionally one region of it) as picture evidence.",
            "arguments": {"pageIndex": 1, "bbox": {"x": 0, "y": 0, "width": 100, "height": 20}}
        },
        "read_passage": {
            "purpose": "Read the passage text of the paper by paragraph label or by question number.",
            "arguments": {"paragraphLabels": ["C", "D"], "questionNumbers": [14, 15]}
        },
        "read_candidate": {
            "purpose": "Read the first-pass CLOUD CANDIDATE slice for specific targets. The candidate is an input, not the truth.",
            "arguments": {"taskIds": ["optional task id list"], "questionNumbers": [14, 15]}
        },
        "apply_edits": {
            "purpose": "Submit a batch of domain commands. This really writes to the authoritative draft.",
            "arguments": {
                "baseVersion": 7,
                "commands": [{"op": "setAnswer", "slotId": "slot-27", "value": {"kind": "text", "values": ["example"]}}],
                "evidence": [{"sourceFileId": main_source_file_id, "pageIndex": 1, "quote": "27 example"}]
            }
        },
        "record_ruling": {
            "purpose": "Adjudicate a difference that is listed in the context, WITHOUT editing anything. \
    Use it when you have checked the original file and the difference does not need the user.",
            "arguments": {
                "rulings": [{
                    "targetType": "slot | task_group | response_group | part",
                    "targetId": "the targetId exactly as listed in differences",
                    "field": "the field exactly as listed in differences",
                    "ruling": "current_is_correct | cannot_resolve",
                    "reason": "why, in one sentence",
                    "evidence": [{"sourceFileId": main_source_file_id, "pageIndex": 1, "quote": "the exact text you relied on"}]
                }]
            }
        },
        "report_insufficient_context": {
            "purpose": "Say that the evidence you were given is NOT enough to judge, and ask for exactly what you need. \
    Use this instead of guessing: a guess that cannot be checked against the file is worse than saying you do not know. packetId and a non-empty reason are required.",
            "arguments": {
                "packetId": "the packetId you were given",
                "reason": "why the current evidence is not enough, in one sentence",
                "needs": [
                    {"kind": "pages", "from": 7, "to": 7},
                    {"kind": "search", "quote": "Questions 14-20"},
                    {"kind": "page_region", "pageIndex": 3, "bbox": {"x": 0, "y": 0, "width": 100, "height": 20}},
                    {"kind": "passage", "paragraphLabels": ["C", "D"]},
                    {"kind": "candidate", "taskIds": ["task-id"]},
                    {"kind": "draft", "questionNumbers": [14, 15]}
                ]
            }
        },
        "finish_packet": {
            "purpose": "Declare that you are done with THIS packet. The backend still recomputes what is left.",
            "arguments": {
                "note": "short explanation",
                "unresolved": [{
                    "targetId": "optional stable id when the doubt is about one target",
                    "message": "what you could not settle, in the user's language",
                    "evidence": [{"sourceFileId": main_source_file_id, "pageIndex": 1, "quote": "what you saw"}]
                }]
            }
        },
        "finish": {
            "purpose": "Declare that you have done what you can. The backend still recomputes what is left.",
            "arguments": {
                "note": "short explanation",
                "unresolved": [{
                    "targetId": "optional stable id when the doubt is about one target",
                    "message": "what you could not settle, in the user's language",
                    "evidence": [{"sourceFileId": main_source_file_id, "pageIndex": 1, "quote": "what you saw"}]
                }]
            }
        }
    })
}

/// 修复规则（给模型看的文字约束）。
///
/// `context.contextMode == "packets"` 时，上下文**不是**整卷：它是本地预切的一个校核包。
/// 同一句话在两种模式下含义相反（「你看到的是整份文档」vs「你看到的是一个包」），
/// 所以规则必须按模式生成，而不是两套手抄的文案。
pub(crate) fn repair_tool_rules(context: &Value) -> Value {
    let packet_mode = context.get("contextMode").and_then(Value::as_str) == Some("packets");
    let tools = crate::schema::cloud_repair_v1::CLOUD_REPAIR_TOOLS.join(", ");
    let mut rules: Vec<String> = vec![
        "Return JSON only: exactly one object with top-level keys callId (non-empty string), tool (one allowed name), and arguments (an object matching that tool's entry in the tools table).".to_string(),
        format!("tool MUST be one of {tools}. There is no other tool."),
        "You may only use the ops listed in allowedOps. resolveIssue and any quality/audit/provenance flag are NOT available.".to_string(),
        "Target ids MUST be the stable ids you got from read_draft or the context. Never invent an id.".to_string(),
        "Attach evidence from the original file to content changes: evidence entries are {sourceFileId, 1-based pageIndex, exact non-empty quote}. A malformed entry rejects the whole batch. An empty evidence list is accepted, but the change then carries no source support for the reviewer.".to_string(),
        "Never invent an answer the original file does not provide. Leave it unresolved instead.".to_string(),
        "Some targets are protected because a human edited them. If a batch is rejected for that reason, narrow the batch instead of retrying the same commands.".to_string(),
    ];
    rules.push(if packet_mode {
        "apply_edits REQUIRES numeric baseVersion. Use draftSlice.editVersion from this packet or editVersion from a scoped read_draft result; never invent a version.".to_string()
    } else {
        "apply_edits REQUIRES numeric baseVersion. Call read_draft first and pass back the editVersion you actually saw.".to_string()
    });
    rules.push(if packet_mode {
        "The context is ONE repair packet, not the whole paper. It lists what was included, what was omitted and which tool fetches it. Do not claim anything outside the packet is verified.".to_string()
    } else {
        "The context lists the WHOLE document. Do not claim the paper is verified just because you handled the listed differences.".to_string()
    });
    if packet_mode {
        rules.push("In packet mode, if you call read_draft, include taskGroupIds and/or questionNumbers copied from this packet; an empty or out-of-packet selector is rejected.".to_string());
    }
    rules.extend([
        "When a batch is rejected you get the specific error in the next observation. Fix exactly that and try again.".to_string(),
        "A difference is NOT automatically the user's problem. The first-pass candidate can be wrong. If the file shows the current draft is right, record_ruling \"current_is_correct\" instead of leaving the difference for the user.".to_string(),
        "record_ruling only accepts differences that are actually listed in the context, and only for a pair of contents you have checked. It cannot remove structural problems found by the backend validator.".to_string(),
        "If neither side is right, apply_edits to the correct content and then rule the difference \"current_is_correct\" (the candidate stays wrong).".to_string(),
        "Put every doubt you could NOT settle into finish.unresolved. Those become user-visible items, so omitting them hides real uncertainty.".to_string(),
    ]);
    rules.push(if packet_mode {
        "If the packet does not contain what you need, call report_insufficient_context with the exact pages/quotes you need, or fetch it yourself with the read-only tools. NEVER conclude from an impression of a file you were not shown.".to_string()
    } else {
        "If the evidence is not enough to judge, call report_insufficient_context with the exact pages/quotes you need. NEVER conclude from an impression.".to_string()
    });
    rules.push(if packet_mode {
        "Call finish_packet when this packet is done. Use finish only to end the whole run early.".to_string()
    } else {
        "Call finish when you are done; the backend recomputes the remaining work from the current draft.".to_string()
    });
    json!(rules)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_job() -> ImportJob {
        crate::job_store::make_job(
            serde_json::from_value(json!({"title": "Fixture"})).expect("CreateJobInput"),
        )
    }

    fn fixture_source() -> crate::SourceFile {
        serde_json::from_value(json!({
            "fileId": "src-1",
            "originalName": "paper.pdf",
            "storedName": "paper.pdf",
            "fileType": "pdf",
            "sha256": "abc",
            "sizeBytes": 10,
            "role": "question_paper",
            "importedAt": "2026-09-21T00:00:00Z"
        }))
        .expect("SourceFile")
    }

    fn repair_input(observations: &[Value], modality: &str) -> Value {
        make_repair_authoring_step_input(
            &json!({"model": "m"}),
            &fixture_job(),
            "profile-1",
            &fixture_source(),
            Path::new("C:/tmp/paper.pdf"),
            &json!({"differences": []}),
            observations,
            modality,
        )
    }

    /// 允许的 op 清单只有一份真源（分发器的 `MODEL_ALLOWED_OPS`）；手抄一份迟早漂移。
    #[test]
    fn repair_allowed_ops_come_from_the_dispatcher_allow_list() {
        let input = repair_input(&[], "reading");
        let declared: Vec<&str> = input["allowedOps"]
            .as_array()
            .expect("allowedOps")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            declared,
            crate::cloud_repair::tools::MODEL_ALLOWED_OPS.to_vec()
        );
    }

    /// 观察历史有界：多轮修复不该让 prompt 无限增长。最近的保留，省略的如实计数。
    #[test]
    fn repair_observation_history_is_bounded_and_keeps_the_latest() {
        let observations: Vec<Value> = (0..40)
            .map(|index| json!({"callId": format!("c{index}")}))
            .collect();
        let input = repair_input(&observations, "reading");
        let kept = input["observations"].as_array().expect("observations");
        assert_eq!(kept.len(), MAX_REPAIR_OBSERVATIONS);
        assert_eq!(
            kept.last().unwrap()["callId"],
            json!("c39"),
            "必须保留最近的观察"
        );
        assert_eq!(
            input["omittedObservationCount"],
            json!(40 - MAX_REPAIR_OBSERVATIONS),
            "省略了多少必须如实写明"
        );
    }

    /// 模态钩子：输入带上模态，后续 prompt 按模态措辞。
    #[test]
    fn candidate_and_repair_inputs_carry_the_modality() {
        assert_eq!(
            repair_input(&[], "listening")["modality"],
            json!("listening")
        );
        let candidate = make_cloud_authoring_candidate_input(
            &json!({"model": "m"}),
            &fixture_job(),
            "profile-1",
            &fixture_source(),
            Path::new("C:/tmp/paper.pdf"),
            &json!({}),
            "listening",
        );
        assert_eq!(candidate["modality"], json!("listening"));
        assert!(candidate["outputContract"]["shape"]
            .get("listeningParts")
            .is_some());
    }

    #[test]
    fn packet_repair_rules_explain_the_read_draft_scope_selector() {
        let rules = repair_tool_rules(&json!({"contextMode": "packets"}));
        let rules = rules.as_array().expect("rules array");
        assert!(
            rules.iter().filter_map(Value::as_str).any(|rule| {
                rule.contains("read_draft")
                    && rule.contains("taskGroupIds")
                    && rule.contains("questionNumbers")
                    && rule.contains("packet")
            }),
            "包模式必须说明 read_draft 需要本包内的 taskGroupIds 或 questionNumbers：{rules:#?}"
        );
        assert!(
            rules.iter().filter_map(Value::as_str).any(|rule| {
                rule.contains("apply_edits")
                    && rule.contains("draftSlice")
                    && rule.contains("editVersion")
                    && rule.contains("read_draft")
            }),
            "包模式应指导使用 draftSlice.editVersion 或 read_draft 返回的版本：{rules:#?}"
        );
        assert!(
            !rules.iter().filter_map(Value::as_str).any(|rule| {
                rule.contains("apply_edits") && rule.contains("Call read_draft first")
            }),
            "包模式已有 draftSlice.editVersion，不应强制多打一轮 read_draft：{rules:#?}"
        );
    }

    #[test]
    fn packet_repair_input_carries_pdf_path_only_for_the_l3_fallback() {
        let profile = json!({"model": "m"});
        let job = fixture_job();
        let source = fixture_source();
        let path = Path::new("C:/tmp/paper.pdf");
        let build = |context: &Value| {
            make_repair_authoring_step_input(
                &profile,
                &job,
                "profile-1",
                &source,
                path,
                context,
                &[],
                "reading",
            )
        };

        let packet = build(&json!({"contextMode": "packets"}));
        assert!(
            packet.get("pdfPath").is_none(),
            "L0-L2 packet input must not carry the full-source path: {packet:#?}"
        );

        let l3 = build(&json!({"contextMode": "packets", "attachFullSource": true}));
        assert_eq!(
            l3["pdfPath"],
            json!(path.to_string_lossy().to_string()),
            "L3 still needs the backend-only path to attach the full source"
        );

        assert!(
            build(&json!({"contextMode": "legacy"}))
                .get("pdfPath")
                .is_some(),
            "legacy regression path must keep the PDF source"
        );
    }

    #[test]
    fn repair_tools_table_names_and_argument_keys_match_the_dispatch_contract() {
        let table = repair_tools_table("example-main-source");
        let mut table_names: Vec<&str> = table
            .as_object()
            .expect("tools table object")
            .keys()
            .map(String::as_str)
            .collect();
        let mut allowed_names = crate::schema::cloud_repair_v1::CLOUD_REPAIR_TOOLS.to_vec();
        table_names.sort_unstable();
        allowed_names.sort_unstable();
        assert_eq!(
            table_names, allowed_names,
            "tool table and parser allow-list must match"
        );

        for (tool, expected) in [
            ("read_draft", vec!["questionNumbers", "taskGroupIds"]),
            ("read_source", vec!["pageIndex", "pageTo", "quote"]),
            ("search_source", vec!["query"]),
            ("read_page_region", vec!["bbox", "pageIndex"]),
            ("read_passage", vec!["paragraphLabels", "questionNumbers"]),
            ("read_candidate", vec!["questionNumbers", "taskIds"]),
            ("apply_edits", vec!["baseVersion", "commands", "evidence"]),
            ("record_ruling", vec!["rulings"]),
            (
                "report_insufficient_context",
                vec!["needs", "packetId", "reason"],
            ),
            ("finish_packet", vec!["note", "unresolved"]),
            ("finish", vec!["note", "unresolved"]),
        ] {
            let mut actual: Vec<&str> = table[tool]["arguments"]
                .as_object()
                .unwrap_or_else(|| panic!("{tool} arguments must be an object"))
                .keys()
                .map(String::as_str)
                .collect();
            actual.sort_unstable();
            assert_eq!(actual, expected, "{tool} tool-table keys drifted");
        }
        assert!(
            table["apply_edits"]["arguments"]["baseVersion"].is_number(),
            "baseVersion must be shown with its numeric type"
        );
    }
}
