use crate::{
    authoring_pipeline::{
        dynamic_block_text, dynamic_document_blocks, make_dynamic_authoring_ir,
        make_dynamic_split_candidates, merge_answer_source_candidates, parse_dynamic_answer_text,
    },
    authoring_review::refresh_authoring_review_state,
    cleanup::minimize_process_artifacts_after_authoring,
    job_store::{load_job, update_job},
    llm_gateway::run_llm_gateway,
    llm_profiles::{find_profile, load_llm_api_key, load_profiles},
    llm_suggestions::{
        apply_suggestion_to_authoring, deterministic_llm_output, llm_suggestion_auto_apply_issues,
        make_cloud_paper_generation_input, make_llm_input, make_vision_answer_extraction_input,
        make_vision_transcription_input, save_llm_suggestion,
    },
    main_source_file,
    parser::{
        extract_pdf_images_for_vision, image_count_from_extraction, missing_source_document_ir,
        parse_source_document, vision_transcription_document_ir,
    },
    reading_source::{
        answer_key_from_authoring, display_map_from_authoring, question_order_from_authoring,
    },
    runtime_validation::validate_for_runtime_gate,
    source_review::{
        low_confidence_block_ids, parser_warnings, source_review_issues, source_review_status,
        write_source_review_status,
    },
    util::{ensure_job_dirs, job_dir, read_json_opt, write_json, write_text},
    AutoPipelineInput, CommandResult, ImportJob, JobStatus, SourceFile, WorkflowStep,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn answer_key_sources(job: &ImportJob) -> Vec<&SourceFile> {
    job.source_files
        .iter()
        .filter(|source| source.role == "AnswerKey")
        .collect()
}

pub(crate) fn parse_answer_source_candidates(
    root: &Path,
    job: &ImportJob,
    mode: &str,
) -> CommandResult<Vec<Value>> {
    let mut candidates = Vec::new();
    for source in answer_key_sources(job) {
        let upload_path = job_dir(root, &job.job_id)
            .join("uploads")
            .join(&source.stored_name);
        if !upload_path.exists() {
            candidates.push(json!({
                "source": format!("answer-source-missing:{}", source.file_id),
                "sourceFileId": source.file_id,
                "sourceStoredName": source.stored_name,
                "answers": {},
                "warnings": [format!("Answer key source file missing: {}", source.original_name)]
            }));
            continue;
        }
        if !matches!(source.file_type.as_str(), "txt" | "md" | "pdf" | "docx") {
            candidates.push(json!({
                "source": format!("answer-source-unsupported:{}", source.file_id),
                "sourceFileId": source.file_id,
                "sourceStoredName": source.stored_name,
                "answers": {},
                "warnings": [format!("Unsupported answer key source type: {}", source.file_type)]
            }));
            continue;
        }
        let parser_output = root.join("cache").join("parser").join(format!(
            "{}-answer-{}-document-ir.json",
            job.job_id, source.file_id
        ));
        let answer_doc = parse_source_document(job, source, &upload_path, &parser_output, mode)?;
        let mut answers = serde_json::Map::new();
        for block in dynamic_document_blocks(Some(&answer_doc)) {
            for (key, value) in parse_dynamic_answer_text(&dynamic_block_text(&block)) {
                answers.insert(key, value);
            }
        }
        let warnings = parser_warnings(Some(&answer_doc));
        candidates.push(json!({
            "source": format!("answer-source:{}", source.file_id),
            "sourceFileId": source.file_id,
            "sourceStoredName": source.stored_name,
            "provider": answer_doc.pointer("/parser/provider").cloned().unwrap_or(Value::Null),
            "answers": answers,
            "warnings": warnings
        }));
    }
    Ok(candidates)
}

pub(crate) fn select_llm_profile(
    root: &Path,
    job: &ImportJob,
    requested_profile_id: Option<String>,
) -> Option<String> {
    let profiles = load_profiles(root).unwrap_or_default();
    requested_profile_id
        .or_else(|| job.active_llm_profile_id.clone())
        .or_else(|| {
            profiles.iter().find_map(|profile| {
                if profile
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
                {
                    profile
                        .get("profileId")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                } else {
                    None
                }
            })
        })
}

pub(crate) fn main_pdf_needs_vision_transcription(job: &ImportJob, doc: &Value) -> bool {
    let Some(source) = main_source_file(job) else {
        return false;
    };
    if source.file_type != "pdf" {
        return false;
    }
    let warnings = parser_warnings(Some(doc)).join("\n").to_lowercase();
    let provider = doc
        .pointer("/parser/provider")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let lacks_reliable_groups = !has_reliable_question_groups(&job.job_id, job, doc);
    provider != "vision-llm-transcription"
        && (warnings.contains("no extractable text")
            || warnings.contains("ocr/manual review required")
            || !low_confidence_block_ids(Some(doc), 0.5).is_empty()
            || lacks_reliable_groups)
}

fn has_reliable_question_groups(job_id: &str, job: &ImportJob, doc: &Value) -> bool {
    let split = make_dynamic_split_candidates(job_id, job, Some(doc));
    split
        .get("questionGroupCandidates")
        .and_then(Value::as_array)
        .map(|groups| {
            groups.iter().any(|group| {
                group
                    .get("requiresManualQuestionImport")
                    .and_then(Value::as_bool)
                    != Some(true)
                    && group
                        .get("questionRange")
                        .and_then(Value::as_array)
                        .map(|range| range.len() == 2)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn main_source_is_pdf(job: &ImportJob) -> bool {
    main_source_file(job)
        .map(|source| source.file_type.as_str() == "pdf")
        .unwrap_or(false)
}

fn main_pdf_upload_path(root: &Path, job: &ImportJob) -> CommandResult<(SourceFile, PathBuf)> {
    let source = main_source_file(job)
        .cloned()
        .ok_or_else(|| "no_main_source_file".to_string())?;
    if source.file_type != "pdf" {
        return Err(format!("main_source_is_not_pdf:{}", source.file_type));
    }
    let upload_path = job_dir(root, &job.job_id)
        .join("uploads")
        .join(&source.stored_name);
    if !upload_path.exists() {
        return Err(format!(
            "main_source_file_missing_for_vision:{}",
            upload_path.display()
        ));
    }
    Ok((source, upload_path))
}

fn main_pdf_vision_extraction(root: &Path, job: &ImportJob) -> CommandResult<(Value, PathBuf)> {
    let (_source, upload_path) = main_pdf_upload_path(root, job)?;
    let dir = job_dir(root, &job.job_id);
    let cache_dir = dir.join("cache").join("vision");
    let extraction_path = cache_dir.join("pdf-images.json");
    let asset_dir = cache_dir.join("assets");
    let extraction =
        extract_pdf_images_for_vision(&job.job_id, &upload_path, &extraction_path, &asset_dir)?;
    Ok((extraction, asset_dir))
}

pub(crate) fn vision_transcription_for_job(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    note: Option<&str>,
) -> CommandResult<(Value, Value)> {
    let (extraction, asset_dir) = main_pdf_vision_extraction(root, job)?;
    let profile = find_profile(root, profile_id)?;
    let image_count = image_count_from_extraction(&extraction);
    if image_count == 0 {
        return Err("vision_transcription_no_extractable_pdf_images".to_string());
    }

    let input = make_vision_transcription_input(&profile, job, profile_id, &extraction);
    let api_key = load_llm_api_key(root, profile_id);
    let output = run_llm_gateway(
        root,
        &job.job_id,
        "transcribe_pdf_images",
        &input,
        api_key.as_deref(),
    )?;
    let text = output
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(format!(
            "vision_transcription_empty:{}",
            output.get("warnings").cloned().unwrap_or_else(|| json!([]))
        ));
    }
    let confidence = output
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.6);
    let warnings = output
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let evidence = json!({
        "profileId": profile_id,
        "imageCount": image_count,
        "extraction": {
            "warnings": extraction.get("warnings").cloned().unwrap_or_else(|| json!([])),
            "assetDir": asset_dir
        },
        "model": output.get("evidence").cloned().unwrap_or_else(|| json!({}))
    });
    let ir = vision_transcription_document_ir(job, &text, confidence, warnings, evidence, note);
    Ok((ir, output))
}

fn vision_answer_candidate_for_job<F>(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    extraction: &Value,
    llm_gateway: &mut F,
) -> CommandResult<(Value, Value)>
where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value>,
{
    let profile = find_profile(root, profile_id)?;
    let image_count = image_count_from_extraction(extraction);
    if image_count == 0 {
        return Err("vision_answer_no_extractable_pdf_images".to_string());
    }
    let input = make_vision_answer_extraction_input(&profile, job, profile_id, extraction);
    let api_key = load_llm_api_key(root, profile_id);
    let output = llm_gateway(
        root,
        &job.job_id,
        "extract_pdf_image_answers",
        &input,
        api_key.as_deref(),
    )?;
    let answers = output.get("answers").cloned().unwrap_or_else(|| json!({}));
    let candidate = json!({
        "source": format!("vision-answer-images:{}", profile_id),
        "provider": "openai-compatible-vision",
        "profileId": profile_id,
        "answers": answers,
        "confidence": output.get("confidence").cloned().unwrap_or(Value::Null),
        "warnings": output.get("warnings").cloned().unwrap_or_else(|| json!([])),
        "evidence": output.get("evidence").cloned().unwrap_or_else(|| json!([])),
        "imageCount": image_count,
        "extractionWarnings": extraction.get("warnings").cloned().unwrap_or_else(|| json!([]))
    });
    Ok((candidate, output))
}

fn normalized_compare_text(value: &Value) -> String {
    match value {
        Value::Array(items) => items
            .iter()
            .map(normalized_compare_text)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join("|"),
        Value::String(text) => text
            .trim()
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase()
            .replace(['-', '_'], " "),
        Value::Number(_) | Value::Bool(_) => value.to_string().to_ascii_uppercase(),
        _ => String::new(),
    }
}

fn answer_key_by_display_from_authoring(ir: &Value) -> serde_json::Map<String, Value> {
    let mut answers = serde_json::Map::new();
    for group in ir
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for question in group
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let display = question
                .get("displayNumber")
                .and_then(Value::as_str)
                .or_else(|| question.get("id").and_then(Value::as_str))
                .unwrap_or_default()
                .trim_start_matches('q')
                .trim_start_matches('Q')
                .to_string();
            if display.is_empty() {
                continue;
            }
            answers.insert(
                display,
                question.get("answer").cloned().unwrap_or_else(|| json!("")),
            );
        }
    }
    answers
}

fn semantic_group_kind_family(kind: &str) -> &str {
    match kind {
        "summary_completion" | "sentence_completion" | "note_completion" | "notes_completion" => {
            "completion"
        }
        value => value,
    }
}

fn expected_question_ids(start: u64, end: u64) -> Vec<String> {
    (start..=end).map(|number| format!("q{}", number)).collect()
}

fn local_question_ids(group: &Value) -> Vec<String> {
    group
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| question.get("id").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect()
}

fn cloud_question_ids(group: &Value) -> Vec<String> {
    group
        .get("questionIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect()
}

fn text_has_inline_blank_marker(text: &str) -> bool {
    text.contains("___") || text.contains("……") || text.contains("...") || text.contains("_____")
}

fn text_has_notes_completion_signal(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("complete the notes")
        || lower.contains("note completion")
        || lower.contains("notes below")
        || (lower.contains("notes") && text_has_inline_blank_marker(text))
}

fn local_inline_notes_evidence(group: &Value) -> bool {
    let local_layout = group
        .pointer("/layout/layoutHint")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if local_layout != "inline_completion" {
        return false;
    }
    let notes = group
        .pointer("/layout/notes")
        .and_then(Value::as_str)
        .unwrap_or_default();
    text_has_notes_completion_signal(notes)
        || (text_has_inline_blank_marker(notes) && local_question_ids(group).len() > 1)
}

fn cloud_evidence_quote_count(group: &Value) -> usize {
    group
        .pointer("/evidence/quotes")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn cloud_outline_comparison(ir: &Value, output: &Value) -> Value {
    let local_groups = ir
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let cloud_groups = output
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut issues = Vec::<Value>::new();
    let mut observations = Vec::<Value>::new();
    if cloud_groups.is_empty() {
        issues.push(json!({
            "kind": "cloud_groups_missing",
            "message": "云端对照没有返回可核对的题组。"
        }));
    }

    for cloud_group in &cloud_groups {
        let range = cloud_group
            .get("range")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let start = range.first().and_then(Value::as_u64).unwrap_or(0);
        let end = range.get(1).and_then(Value::as_u64).unwrap_or(start);
        if start == 0 || end == 0 {
            issues.push(json!({
                "kind": "cloud_group_range_missing",
                "message": "云端对照返回了缺少题号范围的题组。"
            }));
            continue;
        }
        let local = local_groups.iter().find(|group| {
            let range = group
                .get("questionRange")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            range.first().and_then(Value::as_u64) == Some(start)
                && range.get(1).and_then(Value::as_u64) == Some(end)
        });
        let Some(local) = local else {
            issues.push(json!({
                "kind": "cloud_group_missing_locally",
                "range": [start, end],
                "message": format!("云端对照发现 Q{}-{} 题组，但本地题稿中没有对应题组。", start, end)
            }));
            continue;
        };
        let expected_qids = expected_question_ids(start, end);
        let local_qids = local_question_ids(local);
        if !local_qids.is_empty() && local_qids != expected_qids {
            issues.push(json!({
                "kind": "local_group_question_ids_do_not_match_range",
                "range": [start, end],
                "localQuestionIds": local_qids,
                "expectedQuestionIds": expected_qids,
                "message": format!("Q{}-{} 的本地题号归属不完整，请确认同一题组内是否包含所有题号。", start, end)
            }));
        }
        let cloud_qids = cloud_question_ids(cloud_group);
        if cloud_qids.is_empty() {
            observations.push(json!({
                "kind": "cloud_question_ids_missing",
                "range": [start, end],
                "message": "云端对照没有返回逐题 questionIds，本次只按题号范围和本地题稿证据核对。"
            }));
        } else if cloud_qids != expected_qids {
            issues.push(json!({
                "kind": "cloud_group_question_ids_do_not_match_range",
                "range": [start, end],
                "cloudQuestionIds": cloud_qids,
                "expectedQuestionIds": expected_qids,
                "message": format!("云端对照返回的 Q{}-{} 题号归属不完整，请确认是否被拆成了独立题。", start, end)
            }));
        }
        if cloud_evidence_quote_count(cloud_group) == 0 {
            observations.push(json!({
                "kind": "cloud_group_evidence_missing",
                "range": [start, end],
                "message": "云端对照没有提供可见原文证据，结构判断已降权。"
            }));
        }
        let cloud_kind = cloud_group
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("");
        let local_kind = local.get("kind").and_then(Value::as_str).unwrap_or("");
        let cloud_family = semantic_group_kind_family(cloud_kind);
        let local_family = semantic_group_kind_family(local_kind);
        if !cloud_kind.is_empty() && !local_kind.is_empty() && cloud_kind != local_kind {
            if cloud_family == local_family && local_family == "completion" {
                observations.push(json!({
                    "kind": "cloud_completion_kind_normalized",
                    "range": [start, end],
                    "localKind": local_kind,
                    "cloudKind": cloud_kind,
                    "message": "云端和本地同属填空题家族，题型命名差异不作为结构错误。"
                }));
            } else {
                issues.push(json!({
                    "kind": "cloud_group_kind_mismatch",
                    "range": [start, end],
                    "localKind": local_kind,
                    "cloudKind": cloud_kind,
                    "message": format!("Q{}-{} 的题型与云端对照不一致，请确认题型。", start, end)
                }));
            }
        }
        let cloud_layout = cloud_group
            .get("layoutHint")
            .and_then(Value::as_str)
            .unwrap_or("");
        let local_layout = local
            .pointer("/layout/layoutHint")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !cloud_layout.is_empty() && !local_layout.is_empty() && cloud_layout != local_layout {
            if local_layout == "inline_completion"
                && local_inline_notes_evidence(local)
                && cloud_family == "completion"
                && local_family == "completion"
            {
                observations.push(json!({
                    "kind": "cloud_layout_deprioritized_by_local_inline_notes",
                    "range": [start, end],
                    "localLayout": local_layout,
                    "cloudLayout": cloud_layout,
                    "message": "本地已识别到连续 notes 原文和内联空格，云端列表布局不覆盖本地结构。"
                }));
            } else {
                issues.push(json!({
                    "kind": "cloud_group_layout_mismatch",
                    "range": [start, end],
                    "localLayout": local_layout,
                    "cloudLayout": cloud_layout,
                    "message": format!("Q{}-{} 的填答布局与云端对照不一致，请确认是否应在原文空格内作答。", start, end)
                }));
            }
        }
    }

    let local_answers = answer_key_by_display_from_authoring(ir);
    if let Some(cloud_answers) = output.get("answerKey").and_then(Value::as_object) {
        for (number, cloud_value) in cloud_answers {
            let Some(local_value) = local_answers.get(number) else {
                issues.push(json!({
                    "kind": "cloud_answer_missing_locally",
                    "questionNumber": number,
                    "message": format!("云端对照发现第 {} 题答案，但本地题稿中没有对应答案。", number)
                }));
                continue;
            };
            let local_text = normalized_compare_text(local_value);
            let cloud_text = normalized_compare_text(cloud_value);
            if !cloud_text.is_empty() && local_text.is_empty() {
                observations.push(json!({
                    "kind": "cloud_answer_candidate_only",
                    "questionNumber": number,
                    "cloudAnswer": cloud_value,
                    "message": format!("云端对照提供了第 {} 题答案候选，本地题稿未覆盖。", number)
                }));
            } else if !cloud_text.is_empty() && local_text != cloud_text {
                issues.push(json!({
                    "kind": "cloud_answer_mismatch",
                    "questionNumber": number,
                    "localAnswer": local_value,
                    "cloudAnswer": cloud_value,
                    "message": format!("第 {} 题答案与云端对照不一致，请确认答案。", number)
                }));
            }
        }
    }

    let max_issues = 20usize;
    let truncated = issues.len().saturating_sub(max_issues);
    issues.truncate(max_issues);
    json!({
        "passed": issues.is_empty() && truncated == 0,
        "issueCount": issues.len() + truncated,
        "issues": issues,
        "observations": observations,
        "truncatedIssueCount": truncated
    })
}

fn append_authoring_audit_issue(ir: &mut Value, message: &str) {
    let Some(audit) = ir.get_mut("audit").and_then(Value::as_object_mut) else {
        return;
    };
    let issues = audit
        .entry("issues".to_string())
        .or_insert_with(|| json!([]));
    if !issues.is_array() {
        *issues = json!([]);
    }
    if let Some(items) = issues.as_array_mut() {
        if !items
            .iter()
            .any(|item| item.get("message").and_then(Value::as_str) == Some(message))
        {
            items.push(json!({
                "layer": "QualityCheck",
                "path": "$.audit.cloudComparison",
                "message": message
            }));
        }
    }
}

fn cloud_outline_check_for_job<F>(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    extraction: &Value,
    ir: &Value,
    llm_gateway: &mut F,
) -> Value
where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value>,
{
    let mut report = json!({
        "attempted": true,
        "passed": false,
        "profileId": profile_id,
        "failure": null,
        "warningCount": 0,
        "issues": [],
        "outputConfidence": null
    });
    let profile = match find_profile(root, profile_id) {
        Ok(profile) => profile,
        Err(error) => {
            report["failure"] = json!(error);
            return report;
        }
    };
    let (source, upload_path) = match main_pdf_upload_path(root, job) {
        Ok(value) => value,
        Err(error) => {
            report["failure"] = json!(error);
            return report;
        }
    };
    let input = make_cloud_paper_generation_input(
        &profile,
        job,
        profile_id,
        &source,
        &upload_path,
        extraction,
    );
    let api_key = load_llm_api_key(root, profile_id);
    match llm_gateway(
        root,
        &job.job_id,
        "generate_pdf_reading_outline",
        &input,
        api_key.as_deref(),
    ) {
        Ok(output) => {
            let comparison = cloud_outline_comparison(ir, &output);
            report["passed"] = comparison
                .get("passed")
                .cloned()
                .unwrap_or(Value::Bool(false));
            report["warningCount"] = comparison
                .get("issueCount")
                .cloned()
                .unwrap_or_else(|| json!(0));
            report["issues"] = comparison
                .get("issues")
                .cloned()
                .unwrap_or_else(|| json!([]));
            report["comparison"] = comparison;
            report["outputConfidence"] = output.get("confidence").cloned().unwrap_or(Value::Null);
            report["outputWarnings"] = output.get("warnings").cloned().unwrap_or_else(|| json!([]));
        }
        Err(error) => {
            report["failure"] = json!(error);
            report["issues"] = json!([{
                "kind": "cloud_check_failed",
                "message": "云端对照没有完成，请人工确认题组和答案。"
            }]);
            report["warningCount"] = json!(1);
        }
    }
    report
}

pub(crate) fn run_auto_pipeline_core(
    root: &Path,
    job_id: &str,
    input: Option<AutoPipelineInput>,
) -> CommandResult<Value> {
    run_auto_pipeline_core_with_gateway(root, job_id, input, run_llm_gateway)
}

fn record_group_llm_review(
    ir: &mut Value,
    group_id: &str,
    status: &str,
    confidence: f64,
    warning: String,
    suggestion: &Value,
) {
    let Some(group) = ir
        .get_mut("groups")
        .and_then(Value::as_array_mut)
        .and_then(|groups| {
            groups
                .iter_mut()
                .find(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id))
        })
    else {
        return;
    };
    let Some(obj) = group.as_object_mut() else {
        return;
    };
    let warnings = obj
        .entry("reviewWarnings".to_string())
        .or_insert_with(|| json!([]));
    if !warnings.is_array() {
        *warnings = json!([]);
    }
    if let Some(items) = warnings.as_array_mut() {
        if !items.iter().any(|item| item.as_str() == Some(&warning)) {
            items.push(json!(warning));
        }
    }
    obj.insert(
        "llmReview".to_string(),
        json!({
            "required": true,
            "status": status,
            "confidence": confidence,
            "suggestionId": suggestion.get("suggestionId").cloned().unwrap_or(Value::Null),
            "suggestedKind": suggestion.get("kind").cloned().unwrap_or(Value::Null),
            "warnings": suggestion.get("warnings").cloned().unwrap_or_else(|| json!([])),
            "evidence": suggestion.get("evidence").cloned().unwrap_or_else(|| json!({})),
            "recordedAt": Utc::now().to_rfc3339()
        }),
    );
}

pub(crate) fn run_auto_pipeline_core_with_gateway<F>(
    root: &Path,
    job_id: &str,
    input: Option<AutoPipelineInput>,
    mut llm_gateway: F,
) -> CommandResult<Value>
where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value>,
{
    let job_id = job_id.to_string();
    let options = input.unwrap_or_default();
    let parse_mode = options.parse_mode.as_deref().unwrap_or("auto");
    let confidence_threshold = options.confidence_threshold.unwrap_or(0.85).clamp(0.0, 1.0);
    let target = options.target.as_deref().unwrap_or("editableDraft");
    let allow_overwrite = options.allow_overwrite.unwrap_or(false);

    let mut job = load_job(root, &job_id)?;
    let dir = job_dir(root, &job_id);
    ensure_job_dirs(&dir)?;
    if dir.join("authoring-ir.json").exists() && !allow_overwrite {
        return Err(
            "editable_draft_exists; pass allowOverwrite=true before regenerating draft".to_string(),
        );
    }

    let has_doc = dir.join("document-ir.json").exists();
    if !has_doc {
        let ir = if let Some(source) = main_source_file(&job) {
            let upload_path = dir.join("uploads").join(&source.stored_name);
            if matches!(source.file_type.as_str(), "txt" | "md" | "pdf" | "docx")
                && upload_path.exists()
            {
                let parser_output = root
                    .join("cache")
                    .join("parser")
                    .join(format!("{}-document-ir.json", job_id));
                parse_source_document(&job, source, &upload_path, &parser_output, parse_mode)?
            } else {
                missing_source_document_ir(
                    &job,
                    parse_mode,
                    &format!(
                        "main source file missing or unsupported: type={}, path={}",
                        source.file_type,
                        upload_path.display()
                    ),
                )
            }
        } else {
            missing_source_document_ir(&job, parse_mode, "no MainQuestion source file")
        };
        write_json(&dir.join("document-ir.json"), &ir)?;
        let _ = write_source_review_status(root, &job_id, Some(&ir), false, None)?;
        job = update_job(root, &job_id, |item| {
            let review = source_review_status(root, &job_id, Some(&ir))
                .unwrap_or_else(|_| json!({"required": true, "resolved": false}));
            item.status = if review
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                JobStatus::NeedsReview
            } else {
                JobStatus::Working
            };
            item.current_step = WorkflowStep::DocumentReview;
            item.issue_counts.needs_review = source_review_issues(&review).len() as u32;
        })?;
    }

    let profile_id = select_llm_profile(root, &job, options.profile_id.clone());
    let selected_profile_id = profile_id.clone();
    let mut vision_transcription = json!({
        "attempted": false,
        "applied": false,
        "profileId": profile_id,
        "warnings": [],
        "failure": null
    });
    let mut vision_answer_extraction = json!({
        "attempted": false,
        "applied": false,
        "profileId": selected_profile_id,
        "answerCount": 0,
        "warnings": [],
        "failure": null
    });
    let mut pdf_vision_extraction: Option<Value> = None;
    let mut vision_answer_candidates = Vec::<Value>::new();

    let mut doc = read_json_opt(&dir.join("document-ir.json"))?;
    if let (Some(profile_id_for_vision), Some(current_doc)) = (profile_id.as_deref(), doc.as_ref())
    {
        if main_pdf_needs_vision_transcription(&job, current_doc) {
            if let Some(obj) = vision_transcription.as_object_mut() {
                obj.insert("attempted".to_string(), json!(true));
            }
            match vision_transcription_for_job(
                root,
                &job,
                profile_id_for_vision,
                Some("auto pipeline vision transcription"),
            ) {
                Ok((vision_ir, vision_output)) => {
                    write_text(
                        &dir.join("vision-transcription.txt"),
                        vision_output
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    )?;
                    write_json(
                        &dir.join("vision-transcription-output.json"),
                        &vision_output,
                    )?;
                    write_json(&dir.join("document-ir.json"), &vision_ir)?;
                    let _ =
                        write_source_review_status(root, &job_id, Some(&vision_ir), false, None)?;
                    if let Some(obj) = vision_transcription.as_object_mut() {
                        obj.insert("applied".to_string(), json!(true));
                        obj.insert(
                            "confidence".to_string(),
                            vision_output
                                .get("confidence")
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                        obj.insert(
                            "warnings".to_string(),
                            vision_output
                                .get("warnings")
                                .cloned()
                                .unwrap_or_else(|| json!([])),
                        );
                    }
                    doc = Some(vision_ir);
                    job = update_job(root, &job_id, |item| {
                        item.status = JobStatus::NeedsReview;
                        item.current_step = WorkflowStep::DocumentReview;
                    })?;
                }
                Err(error) => {
                    if let Some(obj) = vision_transcription.as_object_mut() {
                        obj.insert("failure".to_string(), json!(error));
                    }
                }
            }
        }
    }

    if let Some(profile_id_for_answers) = selected_profile_id.as_deref() {
        if main_source_is_pdf(&job) {
            if let Some(obj) = vision_answer_extraction.as_object_mut() {
                obj.insert("attempted".to_string(), json!(true));
            }
            match main_pdf_vision_extraction(root, &job) {
                Ok((extraction, _asset_dir)) => {
                    pdf_vision_extraction = Some(extraction.clone());
                    match vision_answer_candidate_for_job(
                        root,
                        &job,
                        profile_id_for_answers,
                        &extraction,
                        &mut llm_gateway,
                    ) {
                        Ok((candidate, output)) => {
                            let answer_count = candidate
                                .get("answers")
                                .and_then(Value::as_object)
                                .map(|answers| answers.len())
                                .unwrap_or(0);
                            write_json(&dir.join("vision-answer-output.json"), &output)?;
                            vision_answer_candidates.push(candidate);
                            if let Some(obj) = vision_answer_extraction.as_object_mut() {
                                obj.insert("applied".to_string(), json!(answer_count > 0));
                                obj.insert("answerCount".to_string(), json!(answer_count));
                                obj.insert(
                                    "confidence".to_string(),
                                    output.get("confidence").cloned().unwrap_or(Value::Null),
                                );
                                obj.insert(
                                    "warnings".to_string(),
                                    output.get("warnings").cloned().unwrap_or_else(|| json!([])),
                                );
                            }
                        }
                        Err(error) => {
                            if let Some(obj) = vision_answer_extraction.as_object_mut() {
                                obj.insert("failure".to_string(), json!(error));
                            }
                        }
                    }
                }
                Err(error) => {
                    if let Some(obj) = vision_answer_extraction.as_object_mut() {
                        obj.insert("failure".to_string(), json!(error));
                    }
                }
            }
        }
    }

    let source_review = source_review_status(root, &job_id, doc.as_ref())?;
    let parser_warnings = source_review
        .get("parserWarnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let low_confidence_blocks = source_review
        .get("lowConfidenceBlocks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut split = make_dynamic_split_candidates(&job_id, &job, doc.as_ref());
    let answer_candidates = parse_answer_source_candidates(root, &job, parse_mode)?;
    merge_answer_source_candidates(&mut split, answer_candidates);
    merge_answer_source_candidates(&mut split, vision_answer_candidates);
    write_json(&dir.join("split-candidates.json"), &split)?;
    job = update_job(root, &job_id, |item| {
        item.status = JobStatus::Working;
        item.current_step = WorkflowStep::Split;
    })?;

    let mut ir = make_dynamic_authoring_ir(&job, &split, doc.as_ref());
    write_json(&dir.join("authoring-ir.json"), &ir)?;
    job = update_job(root, &job_id, |item| {
        item.status = JobStatus::DraftSaved;
        item.current_step = WorkflowStep::Authoring;
    })?;

    let mut low_confidence_groups = Vec::<String>::new();
    let mut blocked_auto_apply_groups = Vec::<String>::new();
    let mut high_confidence_applied_groups = Vec::<String>::new();
    let mut llm_failures = Vec::<String>::new();
    let mut suggestion_count = 0u32;
    let mut applied_count = 0u32;

    let should_run_group_repair = !main_source_is_pdf(&job);
    if should_run_group_repair {
        if let Some(profile_id) = selected_profile_id.clone() {
            let profile = find_profile(root, &profile_id)?;
            if let Some(groups) = ir.get("groups").and_then(Value::as_array).cloned() {
                for group in groups {
                    let group_id = group
                        .get("groupId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if group_id.is_empty() {
                        continue;
                    }
                    let api_key = load_llm_api_key(root, &profile_id);
                    let llm_input =
                        make_llm_input(&profile, &job, &group, &profile_id, "extract_group");
                    let output = llm_gateway(
                        root,
                        &job_id,
                        "extract_group",
                        &llm_input,
                        api_key.as_deref(),
                    )
                    .unwrap_or_else(|error| {
                        llm_failures.push(format!("{}:{}", group_id, error));
                        deterministic_llm_output(
                            &group,
                            "extract_group",
                            format!("llm gateway fallback: {}", error),
                        )
                    });
                    let confidence = output
                        .get("confidence")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    suggestion_count += 1;

                    let suggestion = json!({
                        "suggestionId": format!("suggestion-{}", Uuid::new_v4().simple()),
                        "jobId": job_id,
                        "groupId": group_id,
                        "profileId": profile_id,
                        "kind": output.get("kind").cloned().unwrap_or_else(|| json!(group.get("kind").and_then(Value::as_str).unwrap_or("short_answer"))),
                        "confidence": confidence,
                        "patch": output.get("patch").cloned().unwrap_or_else(|| json!([])),
                        "questions": output.get("questions").cloned().unwrap_or_else(|| json!([])),
                        "warnings": output.get("warnings").cloned().unwrap_or_else(|| json!([])),
                        "evidence": output.get("evidence").cloned().unwrap_or_else(|| json!({})),
                        "createdAt": Utc::now().to_rfc3339()
                    });
                    let _ = save_llm_suggestion(root, &job_id, &suggestion);

                    if confidence >= confidence_threshold {
                        let selected = vec![
                            "kind".to_string(),
                            "layout".to_string(),
                            "questions".to_string(),
                        ];
                        let auto_apply_issues =
                            llm_suggestion_auto_apply_issues(&ir, &suggestion, &selected);
                        if auto_apply_issues.is_empty()
                            && apply_suggestion_to_authoring(&mut ir, &suggestion, &selected)
                                .is_ok()
                        {
                            if let Some(groups) = ir.get_mut("groups").and_then(Value::as_array_mut)
                            {
                                if let Some(group) = groups.iter_mut().find(|group| {
                                    group.get("groupId").and_then(Value::as_str)
                                        == Some(group_id.as_str())
                                }) {
                                    if let Some(obj) = group.as_object_mut() {
                                        obj.insert("autoApplied".to_string(), json!(true));
                                        obj.insert(
                                            "lastAutoAppliedSuggestionId".to_string(),
                                            suggestion
                                                .get("suggestionId")
                                                .cloned()
                                                .unwrap_or(Value::Null),
                                        );
                                    }
                                }
                            }
                            applied_count += 1;
                            high_confidence_applied_groups.push(
                                suggestion
                                    .get("groupId")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                            );
                        } else {
                            blocked_auto_apply_groups.push(group_id.clone());
                            let warning = format!(
                                "{}:auto_apply_blocked:{}",
                                group_id,
                                auto_apply_issues.join(",")
                            );
                            record_group_llm_review(
                                &mut ir,
                                &group_id,
                                "auto_apply_blocked",
                                confidence,
                                format!("识别结果需要确认：{}", auto_apply_issues.join(",")),
                                &suggestion,
                            );
                            llm_failures.push(warning);
                        }
                    } else {
                        let group_id = suggestion
                            .get("groupId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        record_group_llm_review(
                            &mut ir,
                            &group_id,
                            "low_confidence",
                            confidence,
                            format!(
                                "识别结果置信度 {:.2} 低于自动采用阈值 {:.2}，请确认该题组。",
                                confidence, confidence_threshold
                            ),
                            &suggestion,
                        );
                        low_confidence_groups.push(group_id);
                    }
                }
            }
        } else {
            llm_failures.push("no_enabled_llm_profile_available".to_string());
        }
    }

    if let Some(obj) = ir.as_object_mut() {
        obj.insert(
            "answerKey".to_string(),
            answer_key_from_authoring(&Value::Object(obj.clone())),
        );
        obj.insert(
            "questionOrder".to_string(),
            json!(question_order_from_authoring(&Value::Object(obj.clone()))),
        );
        obj.insert(
            "questionDisplayMap".to_string(),
            display_map_from_authoring(&Value::Object(obj.clone())),
        );
    }
    let remaining_authoring_review = refresh_authoring_review_state(&mut ir);
    if let Some(audit) = ir.get_mut("audit").and_then(Value::as_object_mut) {
        audit.insert("llmUsed".to_string(), json!(suggestion_count > 0));
        audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
        audit.insert(
            "revision".to_string(),
            json!(audit.get("revision").and_then(Value::as_u64).unwrap_or(0) + 1),
        );
    }

    let mut cloud_comparison = json!({
        "attempted": false,
        "passed": false,
        "profileId": selected_profile_id,
        "failure": null,
        "warningCount": 0,
        "issues": []
    });
    if let Some(profile_id_for_cloud) = selected_profile_id.as_deref() {
        if main_source_is_pdf(&job) {
            let extraction = if let Some(extraction) = pdf_vision_extraction.clone() {
                Some(extraction)
            } else {
                match main_pdf_vision_extraction(root, &job) {
                    Ok((extraction, _asset_dir)) => Some(extraction),
                    Err(error) => {
                        cloud_comparison["attempted"] = json!(true);
                        cloud_comparison["failure"] = json!(error);
                        None
                    }
                }
            };
            if let Some(extraction) = extraction {
                cloud_comparison = cloud_outline_check_for_job(
                    root,
                    &job,
                    profile_id_for_cloud,
                    &extraction,
                    &ir,
                    &mut llm_gateway,
                );
            }
        }
    }
    let cloud_warning_count = cloud_comparison
        .get("warningCount")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            if cloud_comparison
                .get("failure")
                .and_then(Value::as_str)
                .is_some()
            {
                1
            } else {
                0
            }
        }) as u32;
    let cloud_needs_confirmation = cloud_comparison
        .get("attempted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && (!cloud_comparison
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || cloud_warning_count > 0);
    if cloud_needs_confirmation {
        append_authoring_audit_issue(
            &mut ir,
            "云端对照没有通过，请确认题组、填空位置和答案；本地题稿未被云端结果覆盖。",
        );
    }
    write_json(&dir.join("authoring-ir.json"), &ir)?;

    let report = validate_for_runtime_gate(root, &job_id, &ir, false)?;
    let report_passed = report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let runtime_mode = report
        .get("runtime")
        .and_then(|runtime| runtime.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let static_runtime_passed = report_passed && runtime_mode == "static-rust";

    let requires_parser_review = !source_review_issues(&source_review).is_empty();
    let requires_authoring_review = remaining_authoring_review > 0;

    let has_review_blocks = !low_confidence_groups.is_empty()
        || !blocked_auto_apply_groups.is_empty()
        || requires_parser_review
        || cloud_needs_confirmation
        || requires_authoring_review;
    let next_status = if has_review_blocks {
        JobStatus::NeedsReview
    } else if target == "editableDraft" {
        JobStatus::DraftSaved
    } else if static_runtime_passed {
        JobStatus::ExportReady
    } else if report_passed {
        JobStatus::DraftSaved
    } else {
        JobStatus::NeedsReview
    };
    let next_step =
        if target == "editableDraft" || requires_parser_review || requires_authoring_review {
            WorkflowStep::Authoring
        } else if !low_confidence_groups.is_empty() || !blocked_auto_apply_groups.is_empty() {
            WorkflowStep::Authoring
        } else if static_runtime_passed {
            WorkflowStep::Export
        } else {
            WorkflowStep::Preview
        };
    let next_route = "groups";
    let user_status = if has_review_blocks {
        "needsConfirmation"
    } else {
        "draftReady"
    };
    let user_message = if cloud_needs_confirmation {
        "题稿已生成，但云端对照提示存在不一致，请在题稿编辑页确认题组、填空位置和答案。"
    } else if requires_parser_review {
        "题稿已生成，但源文件识别结果需要你确认。"
    } else if !low_confidence_groups.is_empty() || !blocked_auto_apply_groups.is_empty() {
        "题稿已生成，请在题稿编辑页确认部分识别结果。"
    } else if requires_authoring_review {
        "题稿已生成，还有题干、答案或题型需要你确认。"
    } else {
        "题稿已生成，可以开始检查和编辑。"
    };

    update_job(root, &job_id, |item| {
        item.status = next_status.clone();
        item.current_step = next_step.clone();
        item.issue_counts.needs_review = low_confidence_groups.len() as u32
            + blocked_auto_apply_groups.len() as u32
            + source_review_issues(&source_review).len() as u32
            + cloud_warning_count
            + remaining_authoring_review;
        let issues = report
            .get("issues")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        item.issue_counts.errors = issues
            .iter()
            .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("error"))
            .count() as u32;
        item.issue_counts.warnings = issues
            .iter()
            .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("warning"))
            .count() as u32;
    })?;

    let pipeline_report = json!({
        "jobId": job_id,
        "confidenceThreshold": confidence_threshold,
        "llm": {
            "suggestionCount": suggestion_count,
            "appliedCount": applied_count,
            "highConfidenceAppliedGroups": high_confidence_applied_groups,
            "lowConfidenceGroups": low_confidence_groups,
            "blockedAutoApplyGroups": blocked_auto_apply_groups,
            "failures": llm_failures
        },
        "validationPassed": report_passed,
        "staticRuntimePassed": static_runtime_passed,
        "realRuntimeDiagnosticPassed": false,
        "runtimeMode": runtime_mode,
        "parser": {
            "warnings": parser_warnings,
            "lowConfidenceBlocks": low_confidence_blocks,
            "visionTranscription": vision_transcription,
            "visionAnswerExtraction": vision_answer_extraction
        },
        "quality": {
            "cloudComparison": cloud_comparison
        },
        "authoring": {
            "remainingReviewItems": remaining_authoring_review
        },
        "status": format!("{:?}", next_status),
        "currentStep": format!("{:?}", next_step),
        "userStatus": user_status,
        "userMessage": user_message,
        "nextRoute": next_route,
        "generatedAt": Utc::now().to_rfc3339(),
        "validationReport": report
    });
    write_json(&dir.join("pipeline-report.json"), &pipeline_report)?;
    let _ = minimize_process_artifacts_after_authoring(root, &job_id, "run_auto_pipeline")?;
    Ok(pipeline_report)
}
