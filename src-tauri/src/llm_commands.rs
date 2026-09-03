use crate::{
    authoring_review::refresh_authoring_review_state,
    auto_pipeline::append_authoring_audit_issue,
    job_store::{load_job, update_job},
    llm_gateway::run_llm_gateway,
    llm_profiles::{
        delete_profile_secret, file_secret_ref, find_profile, load_llm_api_key, load_profiles,
        os_secret_ref, redact_profile_for_ui, save_profile_secret, save_profiles,
    },
    llm_suggestions::{
        apply_suggestion_to_authoring, deterministic_llm_output, llm_group_context,
        llm_suggestion_auto_apply_issues, llm_suggestion_quote_mismatches, load_llm_suggestions,
        make_llm_input, save_llm_suggestion,
    },
    reading_source::{
        answer_key_from_authoring, display_map_from_authoring, question_order_from_authoring,
    },
    source_review::{source_review_issues, source_review_status_for_job},
    util::{job_dir, read_json, write_json},
    CommandResult, JobStatus, SaveLlmProfileInput, WorkflowStep,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::path::Path;
use uuid::Uuid;

/// Validate a profile base URL: must be http(s); plain http is only allowed
/// for loopback or private-network hosts so the API key cannot be sent in
/// cleartext to an arbitrary public host.
fn validate_llm_base_url(base_url: &str) -> CommandResult<()> {
    let trimmed = base_url.trim();
    if trimmed.contains(['@']) {
        return Err("llm_profile_base_url_credentials_in_url".to_string());
    }
    let (scheme, rest) = trimmed
        .split_once("://")
        .ok_or_else(|| "llm_profile_base_url_invalid:no_scheme".to_string())?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!(
            "llm_profile_base_url_unsupported_scheme:{}",
            scheme
        ));
    }
    if rest.is_empty() {
        return Err("llm_profile_base_url_invalid:empty_host".to_string());
    }
    if scheme == "https" {
        return Ok(());
    }
    let host_with_port = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host_with_port
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    // Plain http is only allowed for loopback/private endpoints identified
    // strictly: exact hostnames or true IP literals. Prefix-matching domain
    // names would let hosts like "10.evil.com" masquerade as private.
    let host_is_local = matches!(host.as_str(), "localhost" | "::1" | "[::1]");
    let host_is_private_literal = host
        .parse::<std::net::Ipv4Addr>()
        .ok()
        .map(|ip| ip.is_loopback() || ip.is_private() || ip.is_link_local())
        .unwrap_or(false);
    let host_is_trusted_name = host == "host.docker.internal" || host.ends_with(".local");
    if host_is_local || host_is_private_literal || host_is_trusted_name {
        Ok(())
    } else {
        Err(format!(
            "llm_profile_base_url_insecure_http:{}; use https for public endpoints",
            host
        ))
    }
}

const SUPPORTED_LLM_PROVIDERS: [&str; 4] = [
    "OpenAiCompatible",
    "AnthropicCompatible",
    "Ollama",
    "Custom",
];

pub(crate) fn save_llm_profile_core(
    root: &Path,
    input: SaveLlmProfileInput,
) -> CommandResult<Value> {
    if !SUPPORTED_LLM_PROVIDERS.contains(&input.provider.as_str()) {
        return Err(format!("unsupported_llm_provider:{}", input.provider));
    }
    validate_llm_base_url(&input.base_url)?;
    let profile_id = input
        .profile_id
        .unwrap_or_else(|| format!("profile-{}", Uuid::new_v4().simple()));
    let (has_api_key, secret_backend, secret_message, api_key_secret_ref) =
        if input.api_key.is_some() {
            let (has_api_key, secret_backend, secret_message) =
                save_profile_secret(root, &profile_id, input.api_key.as_deref())?;
            let api_key_secret_ref = if secret_backend == "os" {
                os_secret_ref(&profile_id)
            } else if secret_backend == "file" {
                file_secret_ref(&profile_id)
            } else {
                String::new()
            };
            (
                has_api_key,
                secret_backend,
                secret_message,
                api_key_secret_ref,
            )
        } else {
            let secret_meta = redact_profile_for_ui(root, json!({ "profileId": profile_id }));
            (
                secret_meta
                    .get("hasApiKey")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                secret_meta
                    .get("secretStorageBackend")
                    .and_then(Value::as_str)
                    .unwrap_or("none")
                    .to_string(),
                secret_meta
                    .get("secretStorageMessage")
                    .and_then(Value::as_str)
                    .unwrap_or("No API key is stored.")
                    .to_string(),
                secret_meta
                    .get("apiKeySecretRef")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        };
    let profile = json!({
        "profileId": profile_id,
        "name": input.name,
        "provider": input.provider,
        "baseUrl": input.base_url,
        "model": input.model,
        "temperature": input.temperature,
        "timeoutMs": input.timeout_ms,
        "forceJson": input.force_json,
        "enabled": input.enabled,
        "hasApiKey": has_api_key,
        "apiKeySecretRef": api_key_secret_ref,
        "secretStorageBackend": secret_backend,
        "secretStorageMessage": secret_message
    });
    let mut profiles = load_profiles(root)?;
    let profile_id = profile
        .get("profileId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    profiles
        .retain(|item| item.get("profileId").and_then(Value::as_str) != Some(profile_id.as_str()));
    profiles.insert(0, profile.clone());
    save_profiles(root, &profiles)?;
    Ok(profile)
}

pub(crate) fn delete_llm_profile_core(root: &Path, profile_id: &str) -> CommandResult<Vec<Value>> {
    let mut profiles = load_profiles(root)?;
    profiles.retain(|item| item.get("profileId").and_then(Value::as_str) != Some(profile_id));
    save_profiles(root, &profiles)?;
    delete_profile_secret(root, profile_id)?;
    Ok(load_profiles(root)?)
}

pub(crate) fn test_llm_profile_core(root: &Path, profile_id: &str) -> CommandResult<Value> {
    let started = Utc::now();
    let profile_enabled = load_profiles(root)?
        .iter()
        .find(|item| item.get("profileId").and_then(Value::as_str) == Some(profile_id))
        .and_then(|item| item.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !profile_enabled {
        return Err(format!("profile_disabled:{}", profile_id));
    }
    let profile = find_profile(root, profile_id)?;
    let input = json!({
        "profile": {
            "profileId": profile_id,
            "provider": profile.get("provider").cloned().unwrap_or_else(|| json!("OpenAiCompatible")),
            "baseUrl": profile.get("baseUrl").cloned().unwrap_or_else(|| json!("")),
            "model": profile.get("model").cloned().unwrap_or_else(|| json!("")),
            "temperature": profile.get("temperature").cloned().unwrap_or_else(|| json!(0)),
            "timeoutMs": profile.get("timeoutMs").cloned().unwrap_or_else(|| json!(60000)),
            "forceJson": profile.get("forceJson").cloned().unwrap_or(Value::Bool(true))
        },
        "group": {"groupId": "test", "kind": "short_answer", "instruction": ["Return JSON only."], "questions": []}
    });
    let api_key = load_llm_api_key(root, profile_id);
    let result = run_llm_gateway(
        root,
        "profile-test",
        "test_profile",
        &input,
        api_key.as_deref(),
    );
    let latency = Utc::now()
        .signed_duration_since(started)
        .num_milliseconds()
        .max(0) as u64;
    Ok(
        json!({"ok": result.is_ok(), "message": match result { Ok(_) => "LLM gateway returned valid JSON.".to_string(), Err(error) => format!("LLM gateway failed: {}", error) }, "latencyMs": latency}),
    )
}

pub(crate) fn llm_run_group_core(
    root: &Path,
    job_id: &str,
    group_id: &str,
    profile_id: &str,
    mode: &str,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let profile = find_profile(root, profile_id)?;
    if profile.get("enabled").and_then(Value::as_bool) == Some(false) {
        return Err(format!("profile_disabled:{}", profile_id));
    }
    let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let group = llm_group_context(&ir, group_id)?;
    let input = make_llm_input(&profile, &job, &group, profile_id, mode);
    let api_key = load_llm_api_key(root, profile_id);
    let output =
        run_llm_gateway(root, job_id, mode, &input, api_key.as_deref()).unwrap_or_else(|error| {
            deterministic_llm_output(&group, mode, format!("llm gateway fallback: {}", error))
        });
    let confidence = output
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.65);
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
    save_llm_suggestion(root, job_id, &suggestion)?;
    update_job(root, job_id, |job| {
        job.status = if confidence < 0.85 {
            JobStatus::NeedsReview
        } else {
            JobStatus::DraftSaved
        };
        job.current_step = WorkflowStep::LlmReview;
    })?;
    Ok(suggestion)
}

pub(crate) fn apply_llm_suggestion_core(
    root: &Path,
    job_id: &str,
    suggestion_id: &str,
    selected_paths: Vec<String>,
    question_ids: Option<Vec<String>>,
    user_confirmed: bool,
) -> CommandResult<Value> {
    let mut ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let suggestion = load_llm_suggestions(root, job_id)?
        .into_iter()
        .find(|item| item.get("suggestionId").and_then(Value::as_str) == Some(suggestion_id))
        .ok_or_else(|| format!("suggestion_not_found:{}", suggestion_id))?;
    // The confidence hard gate exists to stop *automatic* trust in low
    // confidence output. A human who just reviewed the diff preview may
    // explicitly adopt it: `user_confirmed` skips only this gate, while every
    // evidence/quote/patch/interaction check below still applies.
    if !user_confirmed
        && suggestion
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            < 0.85
    {
        return Err("low_confidence_suggestion_requires_manual_review".to_string());
    }
    // Per-question partial adoption: restrict the suggestion to the reviewed
    // question ids before validation and application, so an adopted subset
    // cannot be blocked by unselected question defects.
    let suggestion = if let Some(selected_ids) = question_ids.as_ref() {
        let suggested_ids: Vec<String> = suggestion
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|question| {
                question
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect();
        let unknown: Vec<&String> = selected_ids
            .iter()
            .filter(|id| {
                !suggested_ids
                    .iter()
                    .any(|suggested| suggested == id.as_str())
            })
            .collect();
        if !unknown.is_empty() {
            return Err(format!(
                "llm_suggestion_question_not_in_suggestion:{}",
                unknown
                    .iter()
                    .map(|id| id.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        let mut filtered = suggestion.clone();
        if let Some(questions) = filtered.get_mut("questions").and_then(Value::as_array_mut) {
            questions.retain(|question| {
                let id = question
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                selected_ids.iter().any(|selected| selected == id)
            });
        }
        filtered
    } else {
        suggestion
    };
    let mut auto_apply_issues = llm_suggestion_auto_apply_issues(&ir, &suggestion, &selected_paths);
    if user_confirmed {
        // The human reviewed the diff and explicitly accepted: drop only the
        // automatic confidence gate, keep every evidence/quote/patch check.
        auto_apply_issues.retain(|issue| issue != "confidence_below_auto_apply_threshold");
    }
    // Verify evidence quotes against the real source content when the
    // document IR is still available; after artifact minimization the source
    // text is gone and manual adoption remains a human decision.
    if let Ok(document) = read_json(&job_dir(root, job_id).join("document-ir.json")) {
        let block_texts: std::collections::BTreeMap<String, String> =
            crate::authoring_pipeline::dynamic_document_blocks(Some(&document))
                .into_iter()
                .filter_map(|block| {
                    let id = block.get("blockId").and_then(Value::as_str)?.to_string();
                    Some((id, crate::authoring_pipeline::dynamic_block_text(&block)))
                })
                .collect();
        auto_apply_issues.extend(llm_suggestion_quote_mismatches(&suggestion, &block_texts));
    }
    if !auto_apply_issues.is_empty() {
        return Err(format!(
            "llm_suggestion_auto_apply_blocked:{}",
            auto_apply_issues.join(",")
        ));
    }
    let suggestion_group_id = suggestion
        .get("groupId")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    apply_suggestion_to_authoring(&mut ir, &suggestion, &selected_paths)?;
    if suggestion
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        >= 0.85
    {
        if let (Some(group_id), Some(groups)) = (
            suggestion_group_id.as_deref(),
            ir.get_mut("groups").and_then(Value::as_array_mut),
        ) {
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id))
            {
                if let Some(obj) = group.as_object_mut() {
                    obj.insert("autoApplied".to_string(), json!(true));
                    obj.insert(
                        "lastAutoAppliedSuggestionId".to_string(),
                        json!(suggestion_id),
                    );
                }
            }
        }
    }
    let needs_review = refresh_authoring_review_state(&mut ir);
    let source_review = source_review_status_for_job(root, job_id)?;
    let source_review_issue_count = source_review_issues(&source_review).len() as u32;
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
    if let Some(audit) = ir.get_mut("audit").and_then(Value::as_object_mut) {
        audit.insert("llmUsed".to_string(), json!(true));
        audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
        audit.insert("lastSuggestionId".to_string(), json!(suggestion_id));
        if user_confirmed {
            audit.insert("lastSuggestionManuallyConfirmed".to_string(), json!(true));
        }
        audit.insert(
            "revision".to_string(),
            json!(audit.get("revision").and_then(Value::as_u64).unwrap_or(0) + 1),
        );
    }
    if user_confirmed {
        if let Some(group) = ir
            .get_mut("groups")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .find(|group| {
                group.get("groupId").and_then(Value::as_str)
                    == suggestion.get("groupId").and_then(Value::as_str)
            })
        {
            if let Some(obj) = group.as_object_mut() {
                obj.insert(
                    "manuallyConfirmedSuggestion".to_string(),
                    json!(suggestion_id),
                );
            }
        }
    }
    write_json(&job_dir(root, job_id).join("authoring-ir.json"), &ir)?;
    update_job(root, job_id, |job| {
        job.status = if needs_review > 0 || source_review_issue_count > 0 {
            JobStatus::NeedsReview
        } else {
            JobStatus::DraftSaved
        };
        job.current_step = WorkflowStep::Authoring;
        job.issue_counts.needs_review = needs_review + source_review_issue_count;
    })?;
    Ok(ir)
}

pub(crate) fn apply_vision_answer_candidates_core(
    root: &Path,
    job_id: &str,
    decisions: Value,
) -> CommandResult<Value> {
    let dir = job_dir(root, job_id);
    let mut candidates_doc: Value = read_json(&dir.join("vision-answer-candidates.json"))
        .map_err(|_| "vision_answer_candidates_missing".to_string())?;
    let mut ir: Value = read_json(&dir.join("authoring-ir.json"))?;
    let decision_list = decisions.as_array().cloned().unwrap_or_default();
    if decision_list.is_empty() {
        return Err("vision_answer_decisions_empty".to_string());
    }

    // Reject-on-sight when the caller reviewed a stale candidate document:
    // a background cloud review may have regenerated the candidates in the
    // meantime, and silently adopting unseen answers would be wrong.
    if let Some(expected_generated_at) = decisions.get("generatedAt").and_then(Value::as_str) {
        let current_generated_at = candidates_doc
            .get("generatedAt")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !current_generated_at.is_empty() && current_generated_at != expected_generated_at {
            return Err(
                "vision_answer_candidates_stale:regenerated_by_background_review".to_string(),
            );
        }
    }
    let decision_list = decisions
        .get("decisions")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| decisions.as_array().cloned())
        .unwrap_or_default();
    if decision_list.is_empty() {
        return Err("vision_answer_decisions_empty".to_string());
    }

    let mut accepted = Vec::<String>::new();
    let mut rejected = Vec::<String>::new();
    let mut unmatched = Vec::<String>::new();
    let mut already_answered = Vec::<String>::new();
    let mut dismissed_numbers = Vec::<String>::new();
    for decision in &decision_list {
        let number = decision
            .get("questionNumber")
            .and_then(Value::as_str)
            .map(|value| value.trim().trim_start_matches(['q', 'Q']).to_string())
            .or_else(|| {
                decision
                    .get("questionNumber")
                    .and_then(Value::as_u64)
                    .map(|value| value.to_string())
            })
            .unwrap_or_default();
        if number.is_empty() {
            continue;
        }
        let accept = decision
            .get("accept")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !accept {
            rejected.push(format!("q{}", number));
            dismissed_numbers.push(number.clone());
            continue;
        }
        let Some(candidate) = candidates_doc
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("questionNumber").and_then(Value::as_str) == Some(number.as_str())
                })
            })
        else {
            unmatched.push(number.clone());
            continue;
        };
        // Answer overrides are only accepted as plain strings or string
        // arrays; anything else cannot be a legitimate answer value.
        let override_answer = decision
            .get("answer")
            .cloned()
            .filter(|value| !value.is_null());
        if let Some(value) = &override_answer {
            let type_ok = value.is_string()
                || value
                    .as_array()
                    .map(|items| items.iter().all(Value::is_string))
                    .unwrap_or(false);
            if !type_ok {
                unmatched.push(number.clone());
                continue;
            }
        }
        let answer = override_answer
            .unwrap_or_else(|| candidate.get("answer").cloned().unwrap_or(Value::Null));
        let answer_is_empty = answer
            .as_str()
            .map(|text| text.trim().is_empty())
            .unwrap_or(false);
        if answer.is_null() || answer_is_empty {
            unmatched.push(number.clone());
            continue;
        }
        let question_id = format!("q{}", number);
        let mut target = None::<String>;
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
                let id = question
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let display_number = question
                    .get("displayNumber")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if id == question_id || display_number == number {
                    target = Some(id.to_string());
                    break;
                }
            }
            if target.is_some() {
                break;
            }
        }
        let Some(target_id) = target else {
            unmatched.push(number.clone());
            continue;
        };
        // Backend guard: adoption must never overwrite an existing or
        // already-confirmed answer. The UI only offers candidates for empty
        // answers; this keeps the IPC surface safe on its own.
        let existing = ir
            .pointer_mut("/groups")
            .and_then(Value::as_array_mut)
            .and_then(|groups| {
                groups.iter_mut().find_map(|group| {
                    group
                        .get_mut("questions")
                        .and_then(Value::as_array_mut)
                        .and_then(|questions| {
                            questions.iter_mut().find(|question| {
                                question.get("id").and_then(Value::as_str)
                                    == Some(target_id.as_str())
                            })
                        })
                })
            });
        let Some(question) = existing else {
            unmatched.push(number.clone());
            continue;
        };
        let current_answer_filled = question
            .get("answer")
            .map(|value| {
                value
                    .as_str()
                    .map(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| !value.is_null())
            })
            .unwrap_or(false);
        let is_verified = question
            .get("verified")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if current_answer_filled || is_verified {
            already_answered.push(target_id.clone());
            continue;
        }
        if let Some(obj) = question.as_object_mut() {
            obj.insert("answer".to_string(), answer.clone());
            // Vision candidates stay non-verified: adoption only fills the
            // answer; human confirmation still goes through the existing
            // per-group verification gate.
            if let Some(confidence) = candidate.get("confidence").and_then(Value::as_f64) {
                obj.insert("confidence".to_string(), json!(confidence));
            }
        }
        accepted.push(target_id.clone());
    }

    // Persist dismissals so ignored candidates stay ignored across restarts.
    if !dismissed_numbers.is_empty() {
        if let Some(candidates) = candidates_doc
            .get_mut("candidates")
            .and_then(Value::as_array_mut)
        {
            for candidate in candidates.iter_mut() {
                let number = candidate
                    .get("questionNumber")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if dismissed_numbers.iter().any(|item| item == number) {
                    if let Some(obj) = candidate.as_object_mut() {
                        obj.insert("dismissedAt".to_string(), json!(Utc::now().to_rfc3339()));
                    }
                }
            }
        }
        let _ = write_json(&dir.join("vision-answer-candidates.json"), &candidates_doc);
    }

    if accepted.is_empty() && rejected.is_empty() && already_answered.is_empty() {
        return Err(format!(
            "vision_answer_no_candidates_applied:accepted=0,rejected={},unmatched={}",
            rejected.len(),
            unmatched.len()
        ));
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
    let needs_review = refresh_authoring_review_state(&mut ir);
    let source_review = source_review_status_for_job(root, job_id)?;
    let source_review_issue_count = source_review_issues(&source_review).len() as u32;
    append_authoring_audit_issue(
        &mut ir,
        json!({
            "layer": "Authoring",
            "path": "$.audit.visionAnswerAdoption",
            "kind": "vision_answer_adoption",
            "message": format!(
                "视觉答案候选处理完成：采用 {} 题，忽略 {} 题，未匹配 {} 题；采用的答案仍需逐题人工确认。",
                accepted.len(),
                rejected.len(),
                unmatched.len()
            ),
            "acceptedQuestionIds": accepted,
            "rejectedQuestionIds": rejected,
            "unmatchedQuestionNumbers": unmatched,
            "recordedAt": Utc::now().to_rfc3339()
        }),
    );
    if let Some(audit) = ir.get_mut("audit").and_then(Value::as_object_mut) {
        audit.insert("llmUsed".to_string(), json!(true));
        audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
        audit.insert(
            "revision".to_string(),
            json!(audit.get("revision").and_then(Value::as_u64).unwrap_or(0) + 1),
        );
    }
    write_json(&dir.join("authoring-ir.json"), &ir)?;
    update_job(root, job_id, |job| {
        job.status = if needs_review > 0 || source_review_issue_count > 0 {
            JobStatus::NeedsReview
        } else {
            JobStatus::DraftSaved
        };
        job.current_step = WorkflowStep::Authoring;
        job.issue_counts.needs_review = needs_review + source_review_issue_count;
    })?;
    Ok(json!({
        "authoringIr": ir,
        "acceptedQuestionIds": accepted,
        "rejectedQuestionIds": rejected,
        "alreadyAnsweredQuestionIds": already_answered,
        "unmatchedQuestionNumbers": unmatched
    }))
}
