use crate::{
    util::{append_text, job_dir, write_json},
    validator::allowed_question_kind,
    CommandResult,
};
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const MAX_LLM_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_LLM_PDF_BYTES: u64 = 128 * 1024 * 1024;
const MAX_LLM_INLINE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_LLM_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LLM_ATTEMPTS: usize = 3;

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
    let started = std::time::Instant::now();
    let output = match command_name {
        "classify_group" | "extract_group" | "test_profile" => {
            run_openai_compatible_group_llm(command_name, input, api_key)
        }
        "transcribe_pdf_images" => run_openai_compatible_vision_llm(root, job_id, input, api_key),
        "extract_pdf_image_answers" => {
            run_openai_compatible_vision_answer_llm(root, job_id, input, api_key)
        }
        "generate_pdf_reading_outline" => {
            run_openai_compatible_cloud_outline_llm(root, job_id, input, api_key)
        }
        _ => Err(format!("unsupported_llm_gateway_command:{}", command_name)),
    };
    // Per-call observability record: every gateway invocation (success or
    // failure) lands in llm-calls.jsonl with its latency and error class so
    // failures stay diagnosable after the fact.
    let call_record = json!({
        "recordType": "llm_call",
        "commandName": command_name,
        "jobId": job_id,
        "model": input.get("profile").and_then(|profile| profile.get("model")).cloned().unwrap_or(Value::Null),
        "ok": output.is_ok(),
        "latencyMs": started.elapsed().as_millis() as u64,
        "errorClass": match &output {
            Ok(_) => Value::Null,
            Err(error) => json!(error.split(':').next().unwrap_or("unknown")),
        },
        "recordedAt": Utc::now().to_rfc3339()
    });
    let _ = append_llm_call_record(root, job_id, &call_record);
    let output = output?;
    write_json(&output_path, &output)?;
    Ok(output)
}

fn append_llm_call_record(root: &Path, job_id: &str, record: &Value) -> CommandResult<()> {
    let line = serde_json::to_string(record).map_err(|error| error.to_string())?;
    append_text(
        &job_dir(root, job_id).join("llm-calls.jsonl"),
        &format!(
            "{}
",
            line
        ),
    )
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

fn require_openai_compatible_provider(profile: &Value) -> CommandResult<()> {
    let provider = profile
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "llm_provider_missing".to_string())?;
    // Ollama exposes the OpenAI-compatible `/v1/chat/completions` contract.
    // Other configured provider labels do not have a protocol-specific route
    // here, so rejecting them is safer than silently sending the wrong wire
    // format to an endpoint that may accept it.
    if !matches!(provider, "OpenAiCompatible" | "Ollama") {
        return Err(format!(
            "llm_provider_unsupported_for_openai_route:{}",
            provider
        ));
    }
    Ok(())
}

fn openai_chat_completions_endpoint(profile: &Value) -> CommandResult<String> {
    require_openai_compatible_provider(profile)?;
    let base_url =
        llm_base_url(profile).ok_or_else(|| "llm_profile_base_url_missing".to_string())?;
    let parsed = reqwest::Url::parse(&base_url)
        .map_err(|error| format!("llm_profile_base_url_invalid:{}", error))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().filter(|host| !host.is_empty()).is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err("llm_profile_base_url_invalid:unsupported_url_shape".to_string());
    }
    if parsed.scheme() == "http" {
        let host = parsed.host_str().unwrap_or_default().trim_end_matches('.');
        let local_http = host == "localhost"
            || host == "host.docker.internal"
            || host.ends_with(".local")
            || host
                .parse::<std::net::IpAddr>()
                .ok()
                .map(|ip| {
                    ip.is_loopback()
                        || match ip {
                            std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
                            std::net::IpAddr::V6(ip) => ip.is_unicast_link_local(),
                        }
                })
                .unwrap_or(false);
        if !local_http {
            return Err(format!("llm_profile_base_url_insecure_http:{}", host));
        }
    }
    Ok(format!("{}/chat/completions", base_url))
}

fn openai_chat_content(payload: &Value) -> CommandResult<String> {
    payload
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| "llm_empty_content".to_string())
}

/// Fail-closed confidence normalization. A non-numeric or out-of-range
/// confidence collapses to 0.0 and gets a warning appended, so a pathological
/// model output can never satisfy an auto-apply threshold. Missing required
/// fields are rejected by the command-specific validators before this runs.
fn normalize_confidence_fail_closed(obj: &mut serde_json::Map<String, Value>) {
    let raw = obj.get("confidence").cloned();
    let in_range = raw
        .as_ref()
        .and_then(Value::as_f64)
        .map(|value| (0.0..=1.0).contains(&value))
        .unwrap_or(false);
    if !in_range {
        obj.insert("confidence".to_string(), json!(0.0));
        if let Some(warnings) = obj.get_mut("warnings").and_then(Value::as_array_mut) {
            warnings.push(json!(format!("confidence_out_of_range:{:?}", raw)));
        }
    }
}

pub(crate) fn parse_llm_json_content(content: &str) -> CommandResult<Value> {
    let first_error = match serde_json::from_str::<Value>(content) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let mut candidates = Vec::<(usize, usize, Value)>::new();
    for (start, byte) in content.bytes().enumerate() {
        if !matches!(byte, b'{' | b'[') {
            continue;
        }
        let Some(end) = balanced_json_end(content, start) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(&content[start..end]) {
            candidates.push((start, end, value));
        }
    }
    let outermost = candidates
        .iter()
        .filter(|(start, end, _)| {
            !candidates.iter().any(|(other_start, other_end, _)| {
                other_start <= start && other_end >= end && (other_start < start || other_end > end)
            })
        })
        .collect::<Vec<_>>();
    match outermost.as_slice() {
        [(_, _, value)] => Ok((*value).clone()),
        [] => Err(format!("llm_json_parse_failed:{}", first_error)),
        _ => Err("llm_json_parse_failed:ambiguous_wrapped_json".to_string()),
    }
}

fn balanced_json_end(content: &str, start: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut stack = Vec::<u8>::new();
    let mut in_string = false;
    let mut escaped = false;
    for index in start..bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => stack.push(byte),
            b'}' => {
                if stack.pop() != Some(b'{') {
                    return None;
                }
            }
            b']' => {
                if stack.pop() != Some(b'[') {
                    return None;
                }
            }
            _ => {}
        }
        if stack.is_empty() {
            return Some(index + 1);
        }
    }
    None
}

fn openai_post(profile: &Value, api_key: Option<&str>, body: Value) -> CommandResult<Value> {
    let mut last_error = String::new();
    let deadline = Instant::now() + llm_timeout(profile, 60_000);
    for attempt in 0..MAX_LLM_ATTEMPTS {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("llm_timeout_budget_exhausted".to_string());
        }
        match openai_post_once(profile, api_key, body.clone(), remaining) {
            Ok(payload) => return Ok(payload),
            Err(error) => {
                let retryable = is_retryable_llm_http_error(&error);
                last_error = error;
                if !retryable || attempt + 1 == MAX_LLM_ATTEMPTS {
                    break;
                }
                let retry_after_ms = retry_after_ms_from_error(&last_error);
                let backoff_ms = 400 * (attempt as u64 + 1);
                let delay = Duration::from_millis(retry_after_ms.max(backoff_ms));
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining <= delay {
                    return Err(format!("llm_timeout_budget_exhausted:{}", last_error));
                }
                thread::sleep(delay);
            }
        }
    }
    Err(last_error)
}

fn is_retryable_llm_http_error(error: &str) -> bool {
    if error.starts_with("llm_http_timeout:")
        || error.starts_with("llm_http_connect_failed:")
        || error.starts_with("llm_http_body_failed:")
    {
        return true;
    }
    let Some(status) = error
        .strip_prefix("llm_http_")
        .and_then(|value| value.split(':').next())
        .and_then(|value| value.parse::<u16>().ok())
    else {
        return false;
    };
    matches!(status, 408 | 425 | 429) || (500..=599).contains(&status)
}

/// Honor a server-provided Retry-After (seconds), capped so a hostile or
/// misconfigured endpoint cannot stall the pipeline for minutes per retry.
fn retry_after_ms_from_error(error: &str) -> u64 {
    if let Some(milliseconds) = error
        .rsplit(";retry_after_ms=")
        .next()
        .and_then(|suffix| suffix.split(';').next())
        .and_then(|value| value.trim().parse::<u64>().ok())
    {
        return milliseconds.min(5_000);
    }
    error
        .rsplit(";retry_after=")
        .next()
        .and_then(|suffix| suffix.split(';').next())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1000).min(5_000))
        .unwrap_or(0)
}

fn retry_after_ms_from_header(value: &str) -> u64 {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return seconds.saturating_mul(1000).min(5_000);
    }
    let Ok(date) = chrono::DateTime::parse_from_rfc2822(value.trim()) else {
        return 0;
    };
    let delay = date
        .with_timezone(&Utc)
        .signed_duration_since(Utc::now())
        .num_milliseconds()
        .max(0) as u64;
    delay.min(5_000)
}

fn openai_post_once(
    profile: &Value,
    api_key: Option<&str>,
    body: Value,
    timeout: Duration,
) -> CommandResult<Value> {
    let endpoint = openai_chat_completions_endpoint(profile)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("llm_http_client_failed:{}", error))?;
    let mut request = client
        .post(endpoint)
        .header("content-type", "application/json")
        .json(&body);
    if let Some(secret) = api_key.filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(secret);
    }
    let response = request.send().map_err(|error| {
        if error.is_timeout() {
            format!("llm_http_timeout:{}", error)
        } else if error.is_connect() {
            format!("llm_http_connect_failed:{}", error)
        } else {
            format!("llm_http_transport_failed:{}", error)
        }
    })?;
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .map(retry_after_ms_from_header)
        .unwrap_or(0);
    if response.content_length().unwrap_or(0) > MAX_LLM_RESPONSE_BYTES {
        return Err("llm_http_response_too_large".to_string());
    }
    let mut body_reader = response.take(MAX_LLM_RESPONSE_BYTES.saturating_add(1));
    let mut body_bytes = Vec::new();
    body_reader
        .read_to_end(&mut body_bytes)
        .map_err(|error| format!("llm_http_body_failed:{}", error))?;
    if body_bytes.len() as u64 > MAX_LLM_RESPONSE_BYTES {
        return Err("llm_http_response_too_large".to_string());
    }
    let text = String::from_utf8(body_bytes)
        .map_err(|error| format!("llm_http_body_invalid_utf8:{}", error))?;
    if !status.is_success() {
        let retry_suffix = if retry_after > 0 {
            format!(";retry_after_ms={retry_after}")
        } else {
            String::new()
        };
        let payload = serde_json::from_str::<Value>(&text)
            .unwrap_or_else(|_| json!({"raw": text.chars().take(300).collect::<String>()}));
        return Err(format!(
            "llm_http_{}:{}{}",
            status.as_u16(),
            payload,
            retry_suffix
        ));
    }
    let payload = serde_json::from_str::<Value>(&text)
        .map_err(|error| format!("llm_http_json_failed:{}:{}", error, text))?;
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

fn read_llm_file(
    root: &Path,
    job_id: &str,
    raw_path: &str,
    max_bytes: u64,
    kind: &str,
) -> CommandResult<(PathBuf, Vec<u8>)> {
    let path = PathBuf::from(raw_path.trim());
    if !path.is_absolute() {
        return Err(format!("llm_{}_path_must_be_absolute", kind));
    }
    let root = fs::canonicalize(root)
        .map_err(|error| format!("llm_{}_root_unavailable:{}", kind, error))?;
    let path = fs::canonicalize(&path)
        .map_err(|error| format!("llm_{}_path_unavailable:{}:{}", kind, path.display(), error))?;
    if !path.starts_with(&root) {
        return Err(format!(
            "llm_{}_path_outside_app_root:{}:{}",
            kind,
            job_id,
            path.display()
        ));
    }
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("llm_{}_metadata_failed:{}:{}", kind, path.display(), error))?;
    if !metadata.is_file() {
        return Err(format!("llm_{}_path_not_file:{}", kind, path.display()));
    }
    if metadata.len() > max_bytes {
        return Err(format!(
            "llm_{}_too_large:max_bytes={}:size_bytes={}",
            kind,
            max_bytes,
            metadata.len()
        ));
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("llm_{}_read_failed:{}:{}", kind, path.display(), error))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!(
            "llm_{}_too_large:max_bytes={}:size_bytes={}",
            kind,
            max_bytes,
            bytes.len()
        ));
    }
    Ok((path, bytes))
}

fn data_url_for_image(root: &Path, job_id: &str, image: &Value) -> CommandResult<String> {
    let raw_path = image
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "vision_image_path_missing".to_string())?;
    let (_path, bytes) = read_llm_file(root, job_id, raw_path, MAX_LLM_IMAGE_BYTES, "image")?;
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

fn data_url_for_pdf(root: &Path, job_id: &str, input: &Value) -> CommandResult<Option<Value>> {
    let Some(raw_path) = input.get("pdfPath").and_then(Value::as_str) else {
        return Ok(None);
    };
    let (_path, bytes) = read_llm_file(root, job_id, raw_path, MAX_LLM_PDF_BYTES, "pdf")?;
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

fn append_pdf_images_to_content(
    root: &Path,
    job_id: &str,
    content: &mut Vec<Value>,
    input: &Value,
) -> CommandResult<usize> {
    let mut image_count = 0usize;
    let mut inline_bytes = 0u64;
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
            let image_data_url = data_url_for_image(root, job_id, image)?;
            inline_bytes = inline_bytes.saturating_add(image_data_url.len() as u64);
            if inline_bytes > MAX_LLM_INLINE_BYTES {
                return Err("vision_inline_payload_too_large".to_string());
            }
            content.push(json!({"type": "image_url", "image_url": {"url": image_data_url}}));
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

fn run_openai_compatible_vision_llm(
    root: &Path,
    job_id: &str,
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    let mut content = vec![json!({"type": "text", "text": vision_prompt(input)})];
    let image_count = append_pdf_images_to_content(root, job_id, &mut content, input)?;
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
    root: &Path,
    job_id: &str,
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    let mut content = vec![json!({"type": "text", "text": vision_answer_prompt(input)})];
    let image_count = append_pdf_images_to_content(root, job_id, &mut content, input)?;
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
    root: &Path,
    job_id: &str,
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    let mut warnings = Vec::<String>::new();
    let mut content = vec![json!({"type": "text", "text": cloud_outline_prompt(input)})];
    if let Some(pdf_part) = data_url_for_pdf(root, job_id, input)? {
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
            let image_count =
                append_pdf_images_to_content(root, job_id, &mut image_content, input)?;
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
        return Err("suggestion_confidence_missing_or_invalid".to_string());
    }
    if !obj.get("patch").map(Value::is_array).unwrap_or(false) {
        return Err("suggestion_patch_missing_or_invalid".to_string());
    }
    if !obj.get("questions").map(Value::is_array).unwrap_or(false) {
        return Err("suggestion_questions_missing_or_invalid".to_string());
    }
    if let Some(warnings) = obj.get("warnings") {
        if !warnings.is_array() {
            return Err("suggestion_warnings_invalid".to_string());
        }
    } else {
        obj.insert("warnings".to_string(), json!([]));
    }

    let evidence = obj
        .get("evidence")
        .ok_or_else(|| "suggestion_evidence_missing".to_string())?;
    let evidence_obj = evidence
        .as_object()
        .ok_or_else(|| "suggestion_evidence_invalid".to_string())?;
    let source_block_ids = evidence_obj
        .get("sourceBlockIds")
        .and_then(Value::as_array)
        .ok_or_else(|| "suggestion_evidence_source_block_ids_missing".to_string())?;
    for block_id in source_block_ids {
        if block_id
            .as_str()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err("suggestion_evidence_source_block_id_invalid".to_string());
        }
    }
    let quotes = evidence_obj
        .get("quotes")
        .and_then(Value::as_array)
        .ok_or_else(|| "suggestion_evidence_quotes_missing".to_string())?;
    for quote in quotes {
        let quote_obj = quote
            .as_object()
            .ok_or_else(|| "suggestion_evidence_quote_invalid".to_string())?;
        if quote_obj
            .get("blockId")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
            || quote_obj
                .get("text")
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err("suggestion_evidence_quote_invalid".to_string());
        }
    }

    for patch in obj
        .get("patch")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let patch_obj = patch
            .as_object()
            .ok_or_else(|| "suggestion_patch_item_invalid".to_string())?;
        if patch_obj.get("op").and_then(Value::as_str) != Some("replace") {
            return Err("suggestion_patch_op_invalid".to_string());
        }
        let path = patch_obj
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "suggestion_patch_path_missing".to_string())?;
        let value = patch_obj
            .get("value")
            .ok_or_else(|| "suggestion_patch_value_missing".to_string())?;
        match path {
            "/kind" => {
                let value = value
                    .as_str()
                    .ok_or_else(|| "suggestion_patch_kind_invalid".to_string())?;
                if !allowed_question_kind(value) {
                    return Err(format!("invalid_patch_kind:{}", value));
                }
            }
            "/layout/template" => {
                if value.as_str().is_none_or(|value| value.trim().is_empty()) {
                    return Err("suggestion_patch_layout_template_invalid".to_string());
                }
            }
            other => return Err(format!("suggestion_patch_path_invalid:{}", other)),
        }
    }

    for question in obj
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let question_obj = question
            .as_object()
            .ok_or_else(|| "suggestion_question_invalid".to_string())?;
        if question_obj
            .get("id")
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err("suggestion_question_id_invalid".to_string());
        }
        if let Some(prompt) = question_obj.get("prompt") {
            if !prompt.is_string() {
                return Err("suggestion_question_prompt_invalid".to_string());
            }
        }
        if let Some(interaction) = question_obj.get("interaction") {
            let interaction_obj = interaction
                .as_object()
                .ok_or_else(|| "suggestion_question_interaction_invalid".to_string())?;
            if interaction_obj
                .get("type")
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err("suggestion_question_interaction_type_invalid".to_string());
            }
        }
    }

    // Warnings are optional metadata, but every field that can influence
    // adoption is required and structurally checked. A malformed model
    // response therefore becomes a gateway error and is handled by the
    // caller's explicit low-confidence fallback path.
    normalize_confidence_fail_closed(obj);
    if let Some(evidence_obj) = obj.get_mut("evidence").and_then(Value::as_object_mut) {
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
        return Err("transcription_text_missing_or_invalid".to_string());
    }
    let has_text = obj
        .get("text")
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !obj.get("confidence").map(Value::is_number).unwrap_or(false) {
        return Err("transcription_confidence_missing_or_invalid".to_string());
    }
    if let Some(warnings) = obj.get("warnings") {
        if !warnings.is_array() {
            return Err("transcription_warnings_invalid".to_string());
        }
    } else {
        obj.insert("warnings".to_string(), json!([]));
    }
    if !has_text {
        if let Some(warnings) = obj.get_mut("warnings").and_then(Value::as_array_mut) {
            warnings.push(json!("empty-vision-transcription"));
        }
    }
    if let Some(evidence) = obj.get("evidence") {
        if !evidence.is_object() {
            return Err("transcription_evidence_invalid".to_string());
        }
    }
    normalize_confidence_fail_closed(obj);
    let evidence = obj
        .entry("evidence".to_string())
        .or_insert_with(|| json!({}));
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

fn normalize_answer_map_value_checked(
    raw: Option<&Value>,
    field: &str,
) -> CommandResult<serde_json::Map<String, Value>> {
    let value = raw.ok_or_else(|| format!("{field}_missing"))?;
    let map = value
        .as_object()
        .ok_or_else(|| format!("{field}_not_object"))?;
    let mut normalized = serde_json::Map::new();
    for (key, value) in map {
        let number = normalize_question_number_key(key)
            .ok_or_else(|| format!("{field}_question_number_invalid:{key}"))?;
        if normalized.contains_key(&number) {
            return Err(format!("{field}_duplicate_question_number:{number}"));
        }
        let answer =
            normalize_answer_value(value).ok_or_else(|| format!("{field}_answer_invalid:{key}"))?;
        normalized.insert(number, answer);
    }
    Ok(normalized)
}

fn normalize_cloud_outline_kind(kind: &str) -> Option<String> {
    let normalized = kind.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    match normalized.as_str() {
        "note_completion" | "notes_completion" | "note_completion_questions" => {
            Some("summary_completion".to_string())
        }
        value if allowed_question_kind(value) => Some(value.to_string()),
        _ => None,
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
    let answers = normalize_answer_map_value_checked(obj.get("answers"), "vision_answer_answers")?;
    obj.insert("answers".to_string(), Value::Object(answers.clone()));
    if answers.is_empty() {
        return Err("vision_answer_answers_empty".to_string());
    }
    if !obj.get("confidence").map(Value::is_number).unwrap_or(false) {
        return Err("vision_answer_confidence_missing_or_invalid".to_string());
    }
    if let Some(warnings) = obj.get("warnings") {
        if !warnings.is_array() {
            return Err("vision_answer_warnings_invalid".to_string());
        }
    } else {
        obj.insert("warnings".to_string(), json!([]));
    }
    if let Some(evidence) = obj.get("evidence") {
        if !evidence.is_array() {
            return Err("vision_answer_evidence_invalid".to_string());
        }
    } else {
        return Err("vision_answer_evidence_missing".to_string());
    }
    let evidence_items = obj
        .get("evidence")
        .and_then(Value::as_array)
        .ok_or_else(|| "vision_answer_evidence_invalid".to_string())?;
    if evidence_items.is_empty() {
        return Err("vision_answer_evidence_empty".to_string());
    }
    for evidence in evidence_items {
        let evidence_obj = evidence
            .as_object()
            .ok_or_else(|| "vision_answer_evidence_item_invalid".to_string())?;
        let question_number = evidence_obj.get("questionNumber").and_then(|value| {
            value
                .as_str()
                .and_then(normalize_question_number_key)
                .or_else(|| {
                    value
                        .as_u64()
                        .and_then(|number| normalize_question_number_key(&number.to_string()))
                })
        });
        if question_number.is_none()
            || evidence_obj
                .get("pageIndex")
                .and_then(Value::as_u64)
                .is_none_or(|value| value == 0)
            || evidence_obj
                .get("quote")
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err("vision_answer_evidence_item_invalid".to_string());
        }
    }
    normalize_confidence_fail_closed(obj);
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
    if !obj.get("title").map(Value::is_string).unwrap_or(false) {
        return Err("cloud_outline_title_missing_or_invalid".to_string());
    }
    if !obj.get("groups").map(Value::is_array).unwrap_or(false) {
        return Err("cloud_outline_groups_missing_or_invalid".to_string());
    }
    if !obj.get("confidence").map(Value::is_number).unwrap_or(false) {
        return Err("cloud_outline_confidence_missing_or_invalid".to_string());
    }
    if let Some(warnings) = obj.get("warnings") {
        if !warnings.is_array() {
            return Err("cloud_outline_warnings_invalid".to_string());
        }
    } else {
        obj.insert("warnings".to_string(), json!([]));
    }

    let answer_key =
        normalize_answer_map_value_checked(obj.get("answerKey"), "cloud_outline_answer_key")?;
    obj.insert("answerKey".to_string(), Value::Object(answer_key));

    let groups = obj
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "cloud_outline_groups_missing_or_invalid".to_string())?;
    for (index, group) in groups.iter().enumerate() {
        let group_obj = group
            .as_object()
            .ok_or_else(|| format!("cloud_outline_group_invalid:{index}"))?;
        let kind = group_obj
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("cloud_outline_group_kind_missing:{index}"))?;
        let _normalized_kind = normalize_cloud_outline_kind(kind)
            .ok_or_else(|| format!("cloud_outline_group_kind_invalid:{index}:{kind}"))?;
        let _layout_hint = group_obj
            .get("layoutHint")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("cloud_outline_group_layout_missing:{index}"))?;
        if !group_obj
            .get("notesText")
            .map(Value::is_string)
            .unwrap_or(false)
        {
            return Err(format!("cloud_outline_group_notes_missing:{index}"));
        }
        let range = group_obj
            .get("range")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("cloud_outline_group_range_missing:{index}"))?;
        if range.len() != 2 {
            return Err(format!("cloud_outline_group_range_invalid:{index}"));
        }
        let start = range[0]
            .as_u64()
            .filter(|value| *value > 0)
            .ok_or_else(|| format!("cloud_outline_group_range_invalid:{index}"))?;
        let end = range[1]
            .as_u64()
            .filter(|value| *value >= start)
            .ok_or_else(|| format!("cloud_outline_group_range_invalid:{index}"))?;
        let question_ids = group_obj
            .get("questionIds")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("cloud_outline_group_question_ids_missing:{index}"))?;
        let expected_count = end.saturating_sub(start).saturating_add(1) as usize;
        if question_ids.len() != expected_count {
            return Err(format!(
                "cloud_outline_group_question_ids_count_invalid:{index}"
            ));
        }
        let mut seen_question_ids = std::collections::BTreeSet::new();
        for question_id in question_ids {
            let question_id = question_id
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("cloud_outline_group_question_id_invalid:{index}"))?;
            if !seen_question_ids.insert(question_id) {
                return Err(format!(
                    "cloud_outline_group_question_id_duplicate:{index}:{question_id}"
                ));
            }
        }
        if !group_obj
            .get("confidence")
            .map(Value::is_number)
            .unwrap_or(false)
        {
            return Err(format!(
                "cloud_outline_group_confidence_missing_or_invalid:{index}"
            ));
        }
        if let Some(warnings) = group_obj.get("warnings") {
            if !warnings.is_array() {
                return Err(format!("cloud_outline_group_warnings_invalid:{index}"));
            }
        }
        let evidence = group_obj
            .get("evidence")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("cloud_outline_group_evidence_missing:{index}"))?;
        let quotes = evidence
            .get("quotes")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("cloud_outline_group_quotes_missing:{index}"))?;
        if quotes.is_empty() {
            return Err(format!("cloud_outline_group_quotes_empty:{index}"));
        }
        for quote in quotes {
            let quote_obj = quote
                .as_object()
                .ok_or_else(|| format!("cloud_outline_group_quote_invalid:{index}"))?;
            if quote_obj
                .get("pageIndex")
                .and_then(Value::as_u64)
                .is_none_or(|value| value == 0)
                || quote_obj
                    .get("text")
                    .and_then(Value::as_str)
                    .is_none_or(|value| value.trim().is_empty())
            {
                return Err(format!("cloud_outline_group_quote_invalid:{index}"));
            }
        }
    }

    normalize_confidence_fail_closed(obj);
    if let Some(groups) = obj.get_mut("groups").and_then(Value::as_array_mut) {
        for (index, group) in groups.iter_mut().enumerate() {
            let Some(group_obj) = group.as_object_mut() else {
                return Err(format!("cloud_outline_group_invalid:{index}"));
            };
            let kind = group_obj
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let normalized_kind = normalize_cloud_outline_kind(&kind)
                .ok_or_else(|| format!("cloud_outline_group_kind_invalid:{index}:{kind}"))?;
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
            group_obj
                .entry("warnings".to_string())
                .or_insert_with(|| json!([]));
            normalize_confidence_fail_closed(group_obj);
        }
    }
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
