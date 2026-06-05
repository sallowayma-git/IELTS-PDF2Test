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
        "extract_pdf_image_answers" => run_openai_compatible_vision_answer_llm(input, api_key),
        "generate_pdf_reading_outline" => run_openai_compatible_cloud_outline_llm(input, api_key),
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

fn vision_answer_prompt(input: &Value) -> String {
    format!(
        "You are extracting the answer key from IELTS Reading PDF page images.\nReturn JSON only. Do not return Markdown, explanations, HTML, JavaScript, or prose outside JSON.\nReturn exactly one JSON object with this shape: {{\"answers\":{{\"8\":\"answer text\",\"9\":\"answer text\"}},\"confidence\":0.0,\"warnings\":[],\"evidence\":[{{\"questionNumber\":\"8\",\"pageIndex\":1,\"quote\":\"short visible source text\"}}]}}.\nUse question number strings without q prefix. Normalize TRUE/FALSE/NOT GIVEN/YES/NO and single-letter options to uppercase. Multi-answer questions may use arrays. Do not invent answers; omit uncertain numbers and add a warning. The supplied images may contain answer pages or answer areas.\nJob JSON: {}\nOutput contract JSON: {}",
        serde_json::to_string(input.get("job").unwrap_or(&Value::Null)).unwrap_or_default(),
        serde_json::to_string(input.get("outputContract").unwrap_or(&Value::Null)).unwrap_or_default()
    )
}

fn cloud_outline_prompt(input: &Value) -> String {
    format!(
        "You are creating a comparison-only outline from an IELTS Reading PDF.\nReturn JSON only. Do not return JavaScript, HTML, Markdown, or final export files.\nReturn exactly one JSON object with this shape: {{\"title\":\"paper title\",\"groups\":[{{\"range\":[1,5],\"kind\":\"true_false_not_given\",\"layoutHint\":\"list\",\"questionIds\":[\"q1\",\"q2\"],\"notesText\":\"\",\"confidence\":0.0,\"evidence\":{{\"quotes\":[{{\"pageIndex\":1,\"text\":\"short visible source excerpt\"}}]}}}}],\"answerKey\":{{\"1\":\"TRUE\"}},\"confidence\":0.0,\"warnings\":[]}}.\nThis output is used only to compare against a local deterministic draft; it must not overwrite the local draft. Use only visible PDF evidence. Do not invent missing groups or answers. Allowed kind values are single_choice, multi_choice, true_false_not_given, yes_no_not_given, matching, heading_matching, matching_information, classification, summary_completion, table_completion, diagram_completion, short_answer, sentence_completion. If your internal label is note_completion, output summary_completion or sentence_completion and preserve layoutHint/notesText. layoutHint must be inline_completion, table, or list when known.\nCritical notes-completion rule: if the PDF says Complete the notes below, note completion, notes, or contains numbered blank/ellipsis markers such as 8……… or 8 ______, keep the entire range as one group, set layoutHint=inline_completion, include every qN in questionIds, and copy the continuous notes text into notesText. Never rewrite this structure into independent list items.\nEvidence rule: every group must include evidence.quotes with short visible PDF excerpts supporting the range, instructions, layout, and blank markers; if you cannot cite evidence, lower that group confidence below 0.75.\nJob JSON: {}\nSource file JSON: {}\nOutput contract JSON: {}",
        serde_json::to_string(input.get("job").unwrap_or(&Value::Null)).unwrap_or_default(),
        serde_json::to_string(input.get("sourceFile").unwrap_or(&Value::Null)).unwrap_or_default(),
        serde_json::to_string(input.get("outputContract").unwrap_or(&Value::Null)).unwrap_or_default()
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

fn data_url_for_pdf(input: &Value) -> CommandResult<Option<Value>> {
    let Some(path) = input.get("pdfPath").and_then(Value::as_str) else {
        return Ok(None);
    };
    let bytes =
        fs::read(path).map_err(|error| format!("cloud_pdf_read_failed:{}:{}", path, error))?;
    let filename = input
        .pointer("/sourceFile/originalName")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("source.pdf");
    Ok(Some(json!({
        "type": "file",
        "file": {
            "filename": filename,
            "file_data": format!("data:application/pdf;base64,{}", general_purpose::STANDARD.encode(bytes))
        }
    })))
}

fn append_pdf_images_to_content(content: &mut Vec<Value>, input: &Value) -> CommandResult<usize> {
    let mut image_count = 0usize;
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
            image_count += 1;
        }
    }
    Ok(image_count)
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
    let image_count = append_pdf_images_to_content(&mut content, input)?;
    if image_count == 0 {
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

fn run_openai_compatible_vision_answer_llm(
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    let mut content = vec![json!({"type": "text", "text": vision_answer_prompt(input)})];
    let image_count = append_pdf_images_to_content(&mut content, input)?;
    if image_count == 0 {
        return Err("vision_answer_extraction_no_images".to_string());
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
    validate_vision_answer_output(&mut parsed, profile, &payload)?;
    Ok(parsed)
}

fn run_openai_compatible_cloud_outline_llm(
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    let mut warnings = Vec::<String>::new();
    let mut content = vec![json!({"type": "text", "text": cloud_outline_prompt(input)})];
    if let Some(pdf_part) = data_url_for_pdf(input)? {
        content.push(pdf_part);
    } else {
        warnings.push("cloud_outline_pdf_file_unavailable".to_string());
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

    let payload = match openai_post(profile, api_key, body) {
        Ok(payload) => payload,
        Err(pdf_error) => {
            warnings.push(format!("direct_pdf_request_failed:{}", pdf_error));
            let mut image_content = vec![
                json!({"type": "text", "text": format!("{}\nThe direct PDF file request failed, so compare using the supplied rendered/extracted page images.", cloud_outline_prompt(input))}),
            ];
            let image_count = append_pdf_images_to_content(&mut image_content, input)?;
            if image_count == 0 {
                return Err(format!(
                    "cloud_outline_direct_pdf_failed_and_no_images:{}",
                    pdf_error
                ));
            }
            let mut fallback_body = json!({
                "model": llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?,
                "temperature": llm_temperature(profile),
                "messages": [
                    {"role": "system", "content": "Return valid JSON only."},
                    {"role": "user", "content": image_content}
                ]
            });
            if llm_force_json(profile) {
                fallback_body["response_format"] = json!({"type": "json_object"});
            }
            openai_post(profile, api_key, fallback_body)?
        }
    };
    let content = openai_chat_content(&payload)?;
    let mut parsed = parse_llm_json_content(&content)?;
    validate_cloud_outline_output(&mut parsed, profile, &payload)?;
    if !warnings.is_empty() {
        if let Some(items) = parsed.get_mut("warnings").and_then(Value::as_array_mut) {
            for warning in warnings {
                items.push(json!(warning));
            }
        }
    }
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

fn normalize_question_number_key(key: &str) -> Option<String> {
    let trimmed = key.trim().trim_start_matches('q').trim_start_matches('Q');
    trimmed
        .parse::<u32>()
        .ok()
        .filter(|number| *number > 0 && *number <= 200)
        .map(|number| number.to_string())
}

fn normalize_answer_text(value: &str) -> String {
    let trimmed = value
        .trim()
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
            )
        })
        .trim();
    let upper = trimmed.to_ascii_uppercase();
    let compact = upper.replace([' ', '-', '_'], "");
    if compact == "NOTGIVEN" {
        "NOT GIVEN".to_string()
    } else if matches!(upper.as_str(), "TRUE" | "FALSE" | "YES" | "NO") {
        upper
    } else if trimmed.len() == 1 && trimmed.chars().all(|ch| ch.is_ascii_alphabetic()) {
        upper
    } else {
        trimmed.to_string()
    }
}

fn normalize_answer_value(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) => {
            let normalized = normalize_answer_text(text);
            (!normalized.is_empty()).then_some(json!(normalized))
        }
        Value::Array(items) => {
            let normalized = items
                .iter()
                .filter_map(normalize_answer_value)
                .filter(|item| match item {
                    Value::String(text) => !text.trim().is_empty(),
                    _ => true,
                })
                .collect::<Vec<_>>();
            (!normalized.is_empty()).then_some(Value::Array(normalized))
        }
        Value::Number(_) | Value::Bool(_) => normalize_answer_value(&json!(value.to_string())),
        _ => None,
    }
}

fn normalize_answer_map_value(raw: Option<Value>) -> serde_json::Map<String, Value> {
    let mut normalized = serde_json::Map::new();
    if let Some(Value::Object(map)) = raw {
        for (key, value) in map {
            let Some(number) = normalize_question_number_key(&key) else {
                continue;
            };
            if let Some(answer) = normalize_answer_value(&value) {
                normalized.insert(number, answer);
            }
        }
    }
    normalized
}

fn ensure_json_array_field(obj: &mut serde_json::Map<String, Value>, key: &str) {
    if !obj.get(key).map(Value::is_array).unwrap_or(false) {
        obj.insert(key.to_string(), json!([]));
    }
}

fn normalize_cloud_outline_kind(kind: &str) -> String {
    let normalized = kind.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    match normalized.as_str() {
        "note_completion" | "notes_completion" | "note_completion_questions" => {
            "summary_completion".to_string()
        }
        value if allowed_question_kind(value) => value.to_string(),
        _ => "short_answer".to_string(),
    }
}

fn validate_vision_answer_output(
    output: &mut Value,
    profile: &Value,
    payload: &Value,
) -> CommandResult<()> {
    if !output.is_object() {
        return Err("vision_answer_not_object".to_string());
    }
    let Some(obj) = output.as_object_mut() else {
        return Err("vision_answer_not_object".to_string());
    };
    let answers = normalize_answer_map_value(obj.remove("answers"));
    obj.insert("answers".to_string(), Value::Object(answers.clone()));
    if answers.is_empty() {
        return Err("vision_answer_answers_empty".to_string());
    }
    if !obj.get("confidence").map(Value::is_number).unwrap_or(false) {
        obj.insert("confidence".to_string(), json!(0.6));
    }
    ensure_json_array_field(obj, "warnings");
    ensure_json_array_field(obj, "evidence");
    let evidence = obj
        .entry("metadata".to_string())
        .or_insert_with(|| json!({}));
    if !evidence.is_object() {
        *evidence = json!({});
    }
    if let Some(evidence_obj) = evidence.as_object_mut() {
        evidence_obj.insert("mode".to_string(), json!("extract_pdf_image_answers"));
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

fn validate_cloud_outline_output(
    output: &mut Value,
    profile: &Value,
    payload: &Value,
) -> CommandResult<()> {
    if !output.is_object() {
        return Err("cloud_outline_not_object".to_string());
    }
    let Some(obj) = output.as_object_mut() else {
        return Err("cloud_outline_not_object".to_string());
    };
    if !obj.get("groups").map(Value::is_array).unwrap_or(false) {
        obj.insert("groups".to_string(), json!([]));
    }
    if let Some(groups) = obj.get_mut("groups").and_then(Value::as_array_mut) {
        for group in groups {
            let Some(group_obj) = group.as_object_mut() else {
                continue;
            };
            let kind = group_obj
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("short_answer")
                .to_string();
            let normalized_kind = normalize_cloud_outline_kind(&kind);
            if normalized_kind != kind {
                group_obj.insert("kind".to_string(), json!(normalized_kind));
                group_obj.insert("rawKind".to_string(), json!(kind));
                let warnings = group_obj
                    .entry("warnings".to_string())
                    .or_insert_with(|| json!([]));
                if let Some(items) = warnings.as_array_mut() {
                    items.push(json!(format!("kind normalized from {}", kind)));
                }
            }
            if !group_obj
                .get("layoutHint")
                .map(Value::is_string)
                .unwrap_or(false)
            {
                group_obj.insert("layoutHint".to_string(), json!(""));
            }
            if !group_obj
                .get("notesText")
                .map(Value::is_string)
                .unwrap_or(false)
            {
                group_obj.insert("notesText".to_string(), json!(""));
            }
            if !group_obj.get("range").map(Value::is_array).unwrap_or(false) {
                group_obj.insert("range".to_string(), json!([]));
            }
            if !group_obj
                .get("questionIds")
                .map(Value::is_array)
                .unwrap_or(false)
            {
                group_obj.insert("questionIds".to_string(), json!([]));
            }
            if !group_obj
                .get("confidence")
                .map(Value::is_number)
                .unwrap_or(false)
            {
                group_obj.insert("confidence".to_string(), json!(0.5));
            }
            let evidence = group_obj
                .entry("evidence".to_string())
                .or_insert_with(|| json!({}));
            if !evidence.is_object() {
                *evidence = json!({});
            }
            if let Some(evidence_obj) = evidence.as_object_mut() {
                ensure_json_array_field(evidence_obj, "quotes");
            }
            ensure_json_array_field(group_obj, "warnings");
        }
    }
    let answer_key = normalize_answer_map_value(obj.remove("answerKey"));
    obj.insert("answerKey".to_string(), Value::Object(answer_key));
    if !obj.get("confidence").map(Value::is_number).unwrap_or(false) {
        obj.insert("confidence".to_string(), json!(0.5));
    }
    ensure_json_array_field(obj, "warnings");
    let evidence = obj
        .entry("metadata".to_string())
        .or_insert_with(|| json!({}));
    if !evidence.is_object() {
        *evidence = json!({});
    }
    if let Some(evidence_obj) = evidence.as_object_mut() {
        evidence_obj.insert("mode".to_string(), json!("generate_pdf_reading_outline"));
        evidence_obj.insert(
            "source".to_string(),
            json!("openai-compatible-cloud-outline-rust"),
        );
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
