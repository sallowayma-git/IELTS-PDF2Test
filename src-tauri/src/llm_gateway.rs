use crate::{
    util::{job_dir, write_json},
    validator::allowed_question_kind,
    CommandResult,
};
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use serde_json::{json, Value};
use std::{fs, path::Path, time::Duration};

pub(crate) fn run_llm_gateway(
    root: &Path,
    job_id: &str,
    command_name: &str,
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let cache_dir = job_dir(root, job_id).join("cache").join("llm");
    let stamp = Utc::now().timestamp_millis();
    let input_path = cache_dir.join(format!("{}-input-{}.json", command_name, stamp));
    let output_path = cache_dir.join(format!("{}-output-{}.json", command_name, stamp));
    write_json(&input_path, &redact_llm_input_for_cache(input))?;
    let output = match command_name {
        "classify_group" | "extract_group" | "test_profile" => {
            run_openai_compatible_group_llm(command_name, input, api_key)
        }
        "transcribe_pdf_images" => run_openai_compatible_vision_llm(input, api_key),
        _ => Err(format!("unsupported_llm_gateway_command:{}", command_name)),
    }?;
    write_json(&output_path, &output)?;
    Ok(output)
}

pub(crate) fn redact_llm_input_for_cache(input: &Value) -> Value {
    let mut redacted = input.clone();
    if let Some(obj) = redacted.as_object_mut() {
        obj.remove("apiKey");
        obj.insert("apiKeySource".to_string(), json!("process-env"));
    }
    redacted
}

fn llm_profile(input: &Value) -> &Value {
    input.get("profile").unwrap_or(&Value::Null)
}

fn llm_base_url(profile: &Value) -> Option<String> {
    profile
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string())
}

fn llm_model(profile: &Value) -> Option<String> {
    profile
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn llm_temperature(profile: &Value) -> f64 {
    profile
        .get("temperature")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

fn llm_timeout(profile: &Value, default_ms: u64) -> Duration {
    Duration::from_millis(
        profile
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .unwrap_or(default_ms)
            .clamp(1_000, 300_000),
    )
}

fn llm_force_json(profile: &Value) -> bool {
    profile.get("forceJson").and_then(Value::as_bool) != Some(false)
}

fn openai_chat_completions_endpoint(profile: &Value) -> CommandResult<String> {
    let base_url =
        llm_base_url(profile).ok_or_else(|| "llm_profile_base_url_missing".to_string())?;
    Ok(format!("{}/chat/completions", base_url))
}

fn openai_chat_content(payload: &Value) -> CommandResult<String> {
    payload
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| "llm_empty_content".to_string())
}

fn parse_llm_json_content(content: &str) -> CommandResult<Value> {
    serde_json::from_str::<Value>(content).or_else(|first_error| {
        let trimmed = content.trim();
        let start = trimmed.find('{');
        let end = trimmed.rfind('}');
        if let (Some(start), Some(end)) = (start, end) {
            if start <= end {
                return serde_json::from_str::<Value>(&trimmed[start..=end])
                    .map_err(|error| format!("llm_json_parse_failed:{};{}", error, first_error));
            }
        }
        Err(format!("llm_json_parse_failed:{}", first_error))
    })
}

fn openai_post(profile: &Value, api_key: Option<&str>, body: Value) -> CommandResult<Value> {
    let endpoint = openai_chat_completions_endpoint(profile)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(llm_timeout(profile, 60_000))
        .build()
        .map_err(|error| format!("llm_http_client_failed:{}", error))?;
    let mut request = client
        .post(endpoint)
        .header("content-type", "application/json")
        .json(&body);
    if let Some(secret) = api_key.filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(secret);
    }
    let response = request
        .send()
        .map_err(|error| format!("llm_http_failed:{}", error))?;
    let status = response.status();
    let text = response
        .text()
        .map_err(|error| format!("llm_http_body_failed:{}", error))?;
    let payload = serde_json::from_str::<Value>(&text)
        .map_err(|error| format!("llm_http_json_failed:{}:{}", error, text))?;
    if !status.is_success() {
        return Err(format!("llm_http_{}:{}", status.as_u16(), payload));
    }
    Ok(payload)
}

fn llm_prompt(input: &Value, mode: &str) -> String {
    let allowed = [
        "single_choice",
        "multi_choice",
        "true_false_not_given",
        "yes_no_not_given",
        "matching",
        "heading_matching",
        "matching_information",
        "classification",
        "summary_completion",
        "table_completion",
        "diagram_completion",
        "short_answer",
        "sentence_completion",
    ]
    .join(", ");
    format!(
        "You are an IELTS Reading authoring assistant.\nReturn JSON only. Do not return Markdown, HTML, JavaScript, ReadingExamSource, final export files, or explanations.\nReturn exactly one JSON object with this shape: {{\"kind\":\"short_answer\",\"confidence\":0.0,\"patch\":[],\"questions\":[],\"warnings\":[],\"evidence\":{{\"sourceBlockIds\":[],\"quotes\":[]}}}}.\nThe kind value MUST be one of the allowed group kinds. patch, questions, warnings, evidence.sourceBlockIds, and evidence.quotes MUST be arrays.\nOnly emit JSON Patch-like objects with op=replace and path in repairContract.allowedPatchPaths. Do not create new paths.\nNever invent passage facts or answers. Suggest structure only.\nUse repairContext.sectionEvidence, continuationEdges, table dimensions, heading/numbering metadata, normalized bbox/page rotation, and reviewWarnings to decide whether the current group kind/layout should be repaired.\nEvidence is required: include evidence.sourceBlockIds copied from the input group.sourceBlockIds and evidence.quotes as [{{\"blockId\":\"...\",\"text\":\"...\"}}] using short source excerpts that justify the suggestion.\nEvery evidence.sourceBlockIds entry and evidence.quotes[].blockId MUST be present in group.sourceBlockIds. If you cannot cite the source blocks, return confidence below 0.85.\nTask: {}.\nAllowed group kinds: {}.\nRepair contract JSON: {}.\nRepair context JSON: {}.\nGroup JSON: {}",
        mode,
        allowed,
        serde_json::to_string(input.get("repairContract").unwrap_or(&Value::Null))
            .unwrap_or_default(),
        serde_json::to_string(input.get("repairContext").unwrap_or(&Value::Null))
            .unwrap_or_default(),
        serde_json::to_string(input.get("group").unwrap_or(&Value::Null)).unwrap_or_default()
    )
}

fn vision_prompt(input: &Value) -> String {
    format!(
        "You are transcribing an IELTS Reading PDF page image for an authoring workflow.\nReturn JSON only with shape {{\"text\":\"...\",\"confidence\":0.0,\"warnings\":[]}}.\nTranscribe all visible passage text, question headings, question prompts, options, tables, labels, and answer keys if present.\nPreserve useful structural headings such as READING PASSAGE, Questions 1-5, and Answers.\nDo not invent missing words or answers. If a region is unclear, write [unclear] and lower confidence.\nJob: {}",
        serde_json::to_string(input.get("job").unwrap_or(&Value::Null)).unwrap_or_default()
    )
}

fn data_url_for_image(image: &Value) -> CommandResult<String> {
    let path = image
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "vision_image_path_missing".to_string())?;
    let bytes =
        fs::read(path).map_err(|error| format!("vision_image_read_failed:{}:{}", path, error))?;
    let mime_type = image
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream");
    Ok(format!(
        "data:{};base64,{}",
        mime_type,
        general_purpose::STANDARD.encode(bytes)
    ))
}

fn run_openai_compatible_group_llm(
    command_name: &str,
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    let mut body = json!({
        "model": model,
        "temperature": llm_temperature(profile),
        "messages": [
            {"role": "system", "content": "Return valid JSON only."},
            {"role": "user", "content": llm_prompt(input, command_name)}
        ]
    });
    if llm_force_json(profile) {
        body["response_format"] = json!({"type": "json_object"});
    }
    let payload = openai_post(profile, api_key, body)?;
    let content = openai_chat_content(&payload)?;
    let mut parsed = parse_llm_json_content(&content)?;
    validate_llm_suggestion_output(&mut parsed, command_name, profile, &payload)?;
    Ok(parsed)
}

fn run_openai_compatible_vision_llm(input: &Value, api_key: Option<&str>) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    let mut content = vec![json!({"type": "text", "text": vision_prompt(input)})];
    for page in input
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let page_index = page.get("pageIndex").and_then(Value::as_u64).unwrap_or(0);
        for image in page
            .get("images")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let label = image
                .get("assetId")
                .or_else(|| image.get("fileName"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            content.push(
                json!({"type": "text", "text": format!("Page {}, image {}", page_index, label)}),
            );
            content.push(
                json!({"type": "image_url", "image_url": {"url": data_url_for_image(image)?}}),
            );
        }
    }
    if content
        .iter()
        .all(|item| item.get("type").and_then(Value::as_str) != Some("image_url"))
    {
        return Err("vision_transcription_no_images".to_string());
    }
    let mut body = json!({
        "model": model,
        "temperature": llm_temperature(profile),
        "messages": [
            {"role": "system", "content": "Return valid JSON only."},
            {"role": "user", "content": content}
        ]
    });
    if llm_force_json(profile) {
        body["response_format"] = json!({"type": "json_object"});
    }
    let payload = openai_post(profile, api_key, body)?;
    let content = openai_chat_content(&payload)?;
    let mut parsed = parse_llm_json_content(&content)?;
    validate_vision_transcription_output(&mut parsed, profile, &payload)?;
    Ok(parsed)
}

pub(crate) fn validate_llm_suggestion_output(
    output: &mut Value,
    mode: &str,
    profile: &Value,
    payload: &Value,
) -> CommandResult<()> {
    if !output.is_object() {
        return Err("suggestion_not_object".to_string());
    }
    let kind = output
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "suggestion_kind_missing".to_string())?;
    if !allowed_question_kind(kind) {
        return Err(format!("invalid_kind:{}", kind));
    }
    let Some(obj) = output.as_object_mut() else {
        return Err("suggestion_not_object".to_string());
    };
    if !obj.get("confidence").map(Value::is_number).unwrap_or(false) {
        obj.insert("confidence".to_string(), json!(0.65));
    }
    if !obj.get("patch").map(Value::is_array).unwrap_or(false) {
        obj.insert("patch".to_string(), json!([]));
    }
    if !obj.get("warnings").map(Value::is_array).unwrap_or(false) {
        obj.insert("warnings".to_string(), json!([]));
    }
    if !obj.get("questions").map(Value::is_array).unwrap_or(false) {
        obj.insert("questions".to_string(), json!([]));
    }
    let evidence = obj
        .entry("evidence".to_string())
        .or_insert_with(|| json!({}));
    if !evidence.is_object() {
        *evidence = json!({});
    }
    if let Some(evidence_obj) = evidence.as_object_mut() {
        evidence_obj.insert("mode".to_string(), json!(mode));
        evidence_obj.insert("source".to_string(), json!("openai-compatible-rust"));
        evidence_obj.insert(
            "model".to_string(),
            profile.get("model").cloned().unwrap_or(Value::Null),
        );
        evidence_obj.insert(
            "usage".to_string(),
            payload.get("usage").cloned().unwrap_or(Value::Null),
        );
        if !evidence_obj
            .get("sourceBlockIds")
            .map(Value::is_array)
            .unwrap_or(false)
        {
            evidence_obj.insert("sourceBlockIds".to_string(), json!([]));
        }
        if !evidence_obj
            .get("quotes")
            .map(Value::is_array)
            .unwrap_or(false)
        {
            evidence_obj.insert("quotes".to_string(), json!([]));
        }
    }
    Ok(())
}

fn validate_vision_transcription_output(
    output: &mut Value,
    profile: &Value,
    payload: &Value,
) -> CommandResult<()> {
    if !output.is_object() {
        return Err("transcription_not_object".to_string());
    }
    let Some(obj) = output.as_object_mut() else {
        return Err("transcription_not_object".to_string());
    };
    if !obj.get("text").map(Value::is_string).unwrap_or(false) {
        obj.insert("text".to_string(), json!(""));
    }
    let has_text = obj
        .get("text")
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !obj.get("confidence").map(Value::is_number).unwrap_or(false) {
        obj.insert(
            "confidence".to_string(),
            json!(if has_text { 0.6 } else { 0.0 }),
        );
    }
    if !obj.get("warnings").map(Value::is_array).unwrap_or(false) {
        obj.insert("warnings".to_string(), json!([]));
    }
    if !has_text {
        if let Some(warnings) = obj.get_mut("warnings").and_then(Value::as_array_mut) {
            warnings.push(json!("empty-vision-transcription"));
        }
    }
    let evidence = obj
        .entry("evidence".to_string())
        .or_insert_with(|| json!({}));
    if !evidence.is_object() {
        *evidence = json!({});
    }
    if let Some(evidence_obj) = evidence.as_object_mut() {
        evidence_obj.insert("mode".to_string(), json!("transcribe_pdf_images"));
        evidence_obj.insert("source".to_string(), json!("openai-compatible-vision-rust"));
        evidence_obj.insert(
            "model".to_string(),
            profile.get("model").cloned().unwrap_or(Value::Null),
        );
        evidence_obj.insert(
            "usage".to_string(),
            payload.get("usage").cloned().unwrap_or(Value::Null),
        );
    }
    Ok(())
}
