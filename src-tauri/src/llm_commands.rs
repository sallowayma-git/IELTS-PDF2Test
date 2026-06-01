use crate::{
    authoring_review::refresh_authoring_review_state,
    job_store::{load_job, update_job},
    llm_gateway::run_llm_gateway,
    llm_profiles::{
        file_secret_ref, find_profile, load_llm_api_key, load_profiles, os_secret_ref,
        save_profile_secret, save_profiles,
    },
    llm_suggestions::{
        apply_suggestion_to_authoring, deterministic_llm_output, llm_group_context,
        llm_suggestion_auto_apply_issues, load_llm_suggestions, make_llm_input,
        save_llm_suggestion,
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

pub(crate) fn save_llm_profile_core(
    root: &Path,
    input: SaveLlmProfileInput,
) -> CommandResult<Value> {
    let profile_id = input
        .profile_id
        .unwrap_or_else(|| format!("profile-{}", Uuid::new_v4().simple()));
    let (has_api_key, secret_backend, secret_message) =
        save_profile_secret(root, &profile_id, input.api_key.as_deref())?;
    let api_key_secret_ref = if secret_backend == "os" {
        os_secret_ref(&profile_id)
    } else if secret_backend == "file" {
        file_secret_ref(&profile_id)
    } else {
        String::new()
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

pub(crate) fn test_llm_profile_core(root: &Path, profile_id: &str) -> CommandResult<Value> {
    let started = Utc::now();
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
) -> CommandResult<Value> {
    let mut ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let suggestion = load_llm_suggestions(root, job_id)?
        .into_iter()
        .find(|item| item.get("suggestionId").and_then(Value::as_str) == Some(suggestion_id))
        .ok_or_else(|| format!("suggestion_not_found:{}", suggestion_id))?;
    if suggestion
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        < 0.85
    {
        return Err("low_confidence_suggestion_requires_manual_review".to_string());
    }
    let auto_apply_issues = llm_suggestion_auto_apply_issues(&ir, &suggestion, &selected_paths);
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
        audit.insert(
            "revision".to_string(),
            json!(audit.get("revision").and_then(Value::as_u64).unwrap_or(0) + 1),
        );
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
