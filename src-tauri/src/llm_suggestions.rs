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
