use crate::{
    util::{append_text, job_dir, write_json},
    validator::allowed_question_kind,
    CommandResult,
};
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use serde_json::{json, Value};
use std::{
    cell::RefCell,
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
/// Upper bound of the per-call timeout. It matches the Settings page maximum
/// (600000 ms): a value the user can type must be the value that takes effect,
/// never a silent clamp.
const LLM_TIMEOUT_MAX_MS: u64 = 600_000;
/// Default output-token cap sent as `max_tokens` when the profile does not set
/// `maxOutputTokens`. An explicit cap makes `finish_reason = length`
/// interpretable: a truncated reply is reported as `llm_output_truncated`
/// instead of masquerading as `llm_json_parse_failed`.
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 16_384;
/// Longest error string kept verbatim in a call record.
const RECORD_ERROR_MAX_CHARS: usize = 8_000;

/// Per-call transport trace. Every gateway call runs synchronously on one
/// thread, so a thread-local collects what the HTTP layer saw (attempts,
/// statuses, request size, usage, finish reason, raw reply) without threading
/// a context argument through every command body. `run_llm_gateway` resets it
/// before dispatch and drains it into the `llm-calls.jsonl` record afterwards.
#[derive(Default)]
struct LlmCallTrace {
    attempts: Vec<Value>,
    request_bytes: u64,
    max_tokens: Option<u64>,
    pdf_bytes: Option<u64>,
    image_count: Option<usize>,
    image_fallback: bool,
    direct_pdf_error: Option<String>,
    http_status: Option<u16>,
    usage: Option<Value>,
    finish_reason: Option<String>,
    raw_content: Option<String>,
}

thread_local! {
    static LLM_CALL_TRACE: RefCell<LlmCallTrace> = RefCell::new(LlmCallTrace::default());
}

fn with_trace<F: FnOnce(&mut LlmCallTrace)>(update: F) {
    LLM_CALL_TRACE.with(|trace| update(&mut trace.borrow_mut()));
}

fn take_trace() -> LlmCallTrace {
    LLM_CALL_TRACE.with(|trace| std::mem::take(&mut *trace.borrow_mut()))
}

fn truncate_for_record(text: &str) -> String {
    if text.chars().count() <= RECORD_ERROR_MAX_CHARS {
        text.to_string()
    } else {
        let mut kept = text.chars().take(RECORD_ERROR_MAX_CHARS).collect::<String>();
        kept.push_str("...[truncated]");
        kept
    }
}

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
    let _stale = take_trace();
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
        // 云端**完整候选**识别：产出可直接渲染的完整稿件（正文 / 题组 / 富内容题干 /
        // 选项库 / 作答位置 / 答案）。它是候选，不写权威稿；身份与质量由后端生成。
        "generate_authoring_candidate" => {
            run_openai_compatible_authoring_candidate_llm(root, job_id, input, api_key)
        }
        // 修复回合：模型输出一个**应用层 JSON 工具消息**，由 Rust 真实执行并把真实结果
        // 回传下一轮。不是建议卡，也不改造供应商 native tools 协议。
        "repair_authoring_step" => {
            run_openai_compatible_repair_step_llm(root, job_id, input, api_key)
        }
        // A4：分歧裁决。与云端识别共用证据面（PDF 附原文件 / 非 PDF 附原文文本），
        // 但输出契约与校验完全不同——它必须回指本次提交的 decisionId 集合。
        "adjudicate_divergence" => {
            run_openai_compatible_adjudication_llm(root, job_id, input, api_key)
        }
        // A3：原文件核验。任务与裁决相反：不是「在三条已有结论里挑一条」，而是
        // 「回原文件查这个值对不对」。输出必须回指本次提交的 slotId 集合，
        // 且任何断言都要带原文引用（quote + pageIndex）。
        "verify_source_answers" => {
            run_openai_compatible_source_verification_llm(root, job_id, input, api_key)
        }
        _ => Err(format!("unsupported_llm_gateway_command:{}", command_name)),
    };
    // Per-call observability record: every gateway invocation (success or
    // failure) lands in llm-calls.jsonl with its latency, transport attempts,
    // request size, usage, finish reason and the FULL error string, so a
    // failure stays diagnosable after the fact. A reply that was received but
    // rejected (unparseable, truncated, or refused by a validator) is also
    // persisted verbatim as `<command>-rejected-<stamp>.json`.
    let trace = take_trace();
    let rejected_path = match (&output, &trace.raw_content) {
        (Err(error), Some(raw)) => {
            let path = cache_dir.join(format!("{}-rejected-{}.json", command_name, stamp));
            let saved = json!({
                "commandName": command_name,
                "error": error,
                "rawContent": raw,
                "usage": trace.usage.clone().unwrap_or(Value::Null),
                "finishReason": trace.finish_reason.clone(),
                "httpStatus": trace.http_status,
                "recordedAt": Utc::now().to_rfc3339()
            });
            write_json(&path, &saved)
                .ok()
                .map(|_| path.to_string_lossy().to_string())
        }
        _ => None,
    };
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
        "error": match &output {
            Ok(_) => Value::Null,
            Err(error) => json!(truncate_for_record(error)),
        },
        "attempts": trace.attempts,
        "requestBytes": trace.request_bytes,
        "maxTokens": trace.max_tokens,
        "pdfBytes": trace.pdf_bytes,
        "imageCount": trace.image_count,
        "imageFallback": trace.image_fallback,
        "directPdfError": trace.direct_pdf_error.as_deref().map(truncate_for_record),
        "httpStatus": trace.http_status,
        "usage": trace.usage.unwrap_or(Value::Null),
        "finishReason": trace.finish_reason,
        "rejectedPath": rejected_path,
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
            .clamp(1_000, LLM_TIMEOUT_MAX_MS),
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

/// Output-token cap for a request: the profile's `maxOutputTokens` when set,
/// otherwise [`DEFAULT_MAX_OUTPUT_TOKENS`].
fn llm_max_output_tokens(profile: &Value) -> u64 {
    profile
        .get("maxOutputTokens")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
}

/// Extract the assistant content and record usage / finish reason / the raw
/// reply on the call trace. `finish_reason = length` means the reply was cut
/// at the output-token cap; it is reported as `llm_output_truncated` because a
/// truncated JSON object would otherwise surface as `llm_json_parse_failed`
/// and send diagnosis down the wrong path.
fn openai_chat_content(payload: &Value) -> CommandResult<String> {
    let usage = payload.get("usage").cloned();
    let finish_reason = payload
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    let content = payload
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let mut max_tokens = None;
    with_trace(|trace| {
        trace.usage = usage.clone();
        trace.finish_reason = finish_reason.clone();
        trace.raw_content = content.clone();
        max_tokens = trace.max_tokens;
    });
    if finish_reason.as_deref() == Some("length") {
        return Err(format!(
            "llm_output_truncated:finish_reason=length:max_tokens={}:completion_tokens={}:content_chars={}",
            max_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unset".to_string()),
            usage
                .as_ref()
                .and_then(|usage| usage.get("completion_tokens"))
                .and_then(Value::as_u64)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            content.as_deref().map(|text| text.chars().count()).unwrap_or(0)
        ));
    }
    content.ok_or_else(|| "llm_empty_content".to_string())
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

fn openai_post(profile: &Value, api_key: Option<&str>, mut body: Value) -> CommandResult<Value> {
    if body.get("max_tokens").is_none() {
        body["max_tokens"] = json!(llm_max_output_tokens(profile));
    }
    let max_tokens = body.get("max_tokens").and_then(Value::as_u64);
    let body_bytes =
        serde_json::to_vec(&body).map_err(|error| format!("llm_request_encode_failed:{error}"))?;
    let request_bytes = body_bytes.len() as u64;
    with_trace(|trace| {
        trace.request_bytes = trace.request_bytes.saturating_add(request_bytes);
        trace.max_tokens = max_tokens;
    });
    let mut last_error = String::new();
    let deadline = Instant::now() + llm_timeout(profile, 60_000);
    for attempt in 0..MAX_LLM_ATTEMPTS {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("llm_timeout_budget_exhausted".to_string());
        }
        let attempt_started = Instant::now();
        with_trace(|trace| trace.http_status = None);
        let result = openai_post_once(profile, api_key, &body_bytes, remaining);
        with_trace(|trace| {
            trace.attempts.push(json!({
                "attempt": trace.attempts.len() + 1,
                "requestBytes": request_bytes,
                "httpStatus": trace.http_status,
                "latencyMs": attempt_started.elapsed().as_millis() as u64,
                "error": result.as_ref().err().map(|error| truncate_for_record(error)),
            }));
        });
        match result {
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
    body: &[u8],
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
        .body(body.to_vec());
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
    with_trace(|trace| trace.http_status = Some(status.as_u16()));
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
    let payload = match serde_json::from_str::<Value>(&text) {
        Ok(payload) => payload,
        Err(error) => {
            let message = format!("llm_http_json_failed:{}:{}", error, text);
            with_trace(|trace| trace.raw_content = Some(text));
            return Err(message);
        }
    };
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
        "You are an IELTS Reading authoring assistant.\nReturn JSON only. Do not return Markdown, HTML, JavaScript, ReadingExamSource, final export files, or explanations.\nReturn exactly one JSON object with this shape: {{\"kind\":\"short_answer\",\"confidence\":0.0,\"patch\":[],\"questions\":[],\"warnings\":[],\"evidence\":{{\"sourceBlockIds\":[],\"quotes\":[]}}}}.\nThe kind value MUST be one of the allowed group kinds. patch, questions, warnings, evidence.sourceBlockIds, and evidence.quotes MUST be arrays.\nEach questions[] item has this shape: {{\"id\":\"q1\",\"prompt\":\"question text\",\"interaction\":{{\"type\":\"text\"}}}}; id is required and must be an id of a question in the input group; prompt (string) and interaction (object with a non-empty type) are optional.\nEach patch[] item has this shape: {{\"op\":\"replace\",\"path\":\"/kind\",\"value\":\"short_answer\"}}.\nOnly emit JSON Patch-like objects with op=replace and path in repairContract.allowedPatchPaths. Do not create new paths.\nNever invent passage facts or answers. Suggest structure only.\nUse repairContext.sectionEvidence, continuationEdges, table dimensions, heading/numbering metadata, normalized bbox/page rotation, and reviewWarnings to decide whether the current group kind/layout should be repaired.\nEvidence is required: include evidence.sourceBlockIds copied from the input group.sourceBlockIds and evidence.quotes as [{{\"blockId\":\"...\",\"text\":\"...\"}}] using short source excerpts that justify the suggestion.\nEvery evidence.sourceBlockIds entry and evidence.quotes[].blockId MUST be present in group.sourceBlockIds. If you cannot cite the source blocks, return confidence below 0.85.\nTask: {}.\nAllowed group kinds: {}.\nRepair contract JSON: {}.\nRepair context JSON: {}.\nGroup JSON: {}",
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
        "You are extracting the answer key from scanned/image-only IELTS Reading answer-page images.\nReturn JSON only. Do not return Markdown, explanations, HTML, JavaScript, or prose outside JSON.\nReturn exactly one JSON object with this shape: {{\"answers\":{{\"8\":\"answer text\",\"9\":\"answer text\"}},\"confidence\":0.0,\"warnings\":[],\"evidence\":[{{\"questionNumber\":\"8\",\"pageIndex\":1,\"quote\":\"short visible source text\"}}]}}.\nUse question number strings without q prefix. Normalize TRUE/FALSE/NOT GIVEN/YES/NO and single-letter options to uppercase. Multi-answer questions may use arrays. Do not invent answers; omit uncertain numbers and add a warning. Only use answers visibly printed on the supplied answer pages. answers must contain at least one entry and evidence at least one item: a reply without any answer is rejected and recorded as \"no answer key found\" — that is the honest outcome when the pages show no readable answer key, so never fill answers from anywhere else. Every emitted answer must have a non-empty visible quote and the one-based rendered image pageIndex where it appears.\nJob JSON: {}\nOutput contract JSON: {}",
        serde_json::to_string(input.get("job").unwrap_or(&Value::Null)).unwrap_or_default(),
        serde_json::to_string(input.get("outputContract").unwrap_or(&Value::Null)).unwrap_or_default()
    )
}

fn cloud_outline_prompt(input: &Value) -> String {
    format!(
        "You are creating a comparison-only outline from an IELTS Reading PDF.\nReturn JSON only. Do not return JavaScript, HTML, Markdown, or final export files.\nReturn exactly one JSON object with this shape: {{\"title\":\"paper title\",\"groups\":[{{\"range\":[1,2],\"kind\":\"true_false_not_given\",\"layoutHint\":\"list\",\"questionIds\":[\"q1\",\"q2\"],\"notesText\":\"\",\"confidence\":0.0,\"evidence\":{{\"quotes\":[{{\"pageIndex\":1,\"text\":\"short visible source excerpt\"}}]}}}}],\"answerKey\":{{\"1\":\"TRUE\"}},\"confidence\":0.0,\"warnings\":[]}}.\nThis output is used only to compare against a local deterministic draft; it must not overwrite the local draft. Use only visible PDF evidence. Do not invent missing groups or answers. Allowed kind values are single_choice, multi_choice, true_false_not_given, yes_no_not_given, matching, heading_matching, matching_information, classification, summary_completion, table_completion, diagram_completion, short_answer, sentence_completion. If your internal label is note_completion, output summary_completion or sentence_completion and preserve layoutHint/notesText. layoutHint is required on every group: inline_completion, table, or list (use list when neither of the others applies). notesText is required on every group: the continuous notes text for completion groups, otherwise an empty string \"\".\nCritical notes-completion rule: if the PDF says Complete the notes below, note completion, notes, or contains numbered blank/ellipsis markers such as 8……… or 8 ______, keep the entire range as one group, set layoutHint=inline_completion, include every qN in questionIds, and copy the continuous notes text into notesText. Never rewrite this structure into independent list items.\nEvidence rule: every group must include at least one evidence.quotes item (pageIndex >= 1, non-empty text) with a short visible PDF excerpt supporting the range, instructions, layout, and blank markers. A group without a quote is rejected: if you cannot quote a group, leave that group out and say so in warnings.\nJob JSON: {}\nSource file JSON: {}\nOutput contract JSON: {}",
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
    let pdf_bytes = bytes.len() as u64;
    with_trace(|trace| trace.pdf_bytes = Some(pdf_bytes));
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
    with_trace(|trace| trace.image_count = Some(image_count));
    Ok(image_count)
}

/// Whether a failed direct-PDF request is worth a second request that carries
/// the rendered page images instead.
///
/// Only a client-side rejection of the request itself qualifies: a 4xx other
/// than auth / timeout / rate limit, typically "file parts unsupported" or
/// "payload too large". A timeout means the server was computing: an image
/// request would compute again from a fresh budget and time out again — that
/// is how a 120 s profile produced 134 s / 144 s whole-paper failures.
/// Connect failures, 5xx and exhausted budgets would equally fail again
/// against the same server.
fn direct_pdf_error_permits_image_fallback(error: &str) -> bool {
    let Some(status) = error
        .strip_prefix("llm_http_")
        .and_then(|value| value.split(':').next())
        .and_then(|value| value.parse::<u16>().ok())
    else {
        return false;
    };
    (400..500).contains(&status) && !matches!(status, 401 | 403 | 408 | 425 | 429)
}

/// Send `body`; when it carried the original PDF and the provider rejected the
/// request itself, retry ONCE with the rendered page images as the evidence.
///
/// The direct-PDF error is never swallowed: it is kept on the call trace, as a
/// warning on success, and appended to the error when the fallback also fails.
#[allow(clippy::too_many_arguments)]
fn post_with_pdf_image_fallback(
    root: &Path,
    job_id: &str,
    profile: &Value,
    api_key: Option<&str>,
    body: Value,
    had_pdf: bool,
    input: &Value,
    fallback_prompt: String,
    no_images_error: &str,
    warnings: &mut Vec<String>,
) -> CommandResult<Value> {
    let pdf_error = match openai_post(profile, api_key, body.clone()) {
        Ok(payload) => return Ok(payload),
        Err(error) => error,
    };
    if !had_pdf || !direct_pdf_error_permits_image_fallback(&pdf_error) {
        return Err(pdf_error);
    }
    with_trace(|trace| trace.direct_pdf_error = Some(pdf_error.clone()));
    let mut image_content = vec![json!({"type": "text", "text": fallback_prompt})];
    let image_count = append_pdf_images_to_content(root, job_id, &mut image_content, input)
        .map_err(|error| format!("{error};direct_pdf_request_failed={pdf_error}"))?;
    if image_count == 0 {
        return Err(format!("{no_images_error}:{pdf_error}"));
    }
    with_trace(|trace| trace.image_fallback = true);
    let mut fallback_body = body;
    fallback_body["messages"][1]["content"] = Value::Array(image_content);
    let payload = openai_post(profile, api_key, fallback_body).map_err(|fallback_error| {
        format!("{fallback_error};direct_pdf_request_failed={pdf_error}")
    })?;
    warnings.push(format!("direct_pdf_request_failed:{pdf_error}"));
    Ok(payload)
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
    let pdf_part = data_url_for_pdf(root, job_id, input)?;
    let had_pdf = pdf_part.is_some();
    if let Some(pdf_part) = pdf_part {
        content.push(pdf_part);
    } else if let Some(source_text) = input
        .get("sourceText")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
    {
        // DOCX 等非 PDF 来源没有页图可附：用本地抽取的原文文本作为唯一证据面。
        // 这样 DOCX 也走完整云端链路，而不是被静默跳过（任务书第二/九项）。
        content.push(json!({
            "type": "text",
            "text": format!(
                "The original file is not a PDF, so no page image is attached. \
The extracted source text below is the ONLY evidence you may use; do not invent content.\n\
--- SOURCE TEXT BEGIN ---\n{source_text}\n--- SOURCE TEXT END ---"
            )
        }));
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

    let payload = post_with_pdf_image_fallback(
        root,
        job_id,
        profile,
        api_key,
        body,
        had_pdf,
        input,
        format!("{}\nThe direct PDF file request failed, so compare using the supplied rendered/extracted page images.", cloud_outline_prompt(input)),
        "cloud_outline_direct_pdf_failed_and_no_images",
        &mut warnings,
    )?;
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

/// 云端完整候选识别的 prompt。
///
/// 契约文字与 `make_cloud_authoring_candidate_input` 的 `outputContract` 是**同一套规则**：
/// 「模型该返回什么」与「我们会校验什么」各写一份，两者迟早漂移，而漂移的代价是模型
/// 产出被静默拒绝、用户看到「识别失败」却无从解释。
///
/// `repairNote`：上一次回复被校验器拒了，把**被拒原因原样**回给模型再问一次。
/// 不带原因地重试同一句话，只会再拿到同一种错误——那不是修复，只是多烧一次配额。
fn authoring_candidate_prompt(input: &Value) -> String {
    let modality = crate::llm_suggestions::candidate_modality(
        input.get("modality").and_then(Value::as_str).unwrap_or("reading"),
    );
    let paper = ielts_paper_label(modality);
    let repair = input
        .get("repairNote")
        .and_then(Value::as_str)
        .filter(|note| !note.trim().is_empty())
        .map(|note| {
            format!(
                "\nYour previous reply was REJECTED by the backend validator. Fix exactly this and return the whole JSON object again.\nRejection reason: {note}\n"
            )
        })
        .unwrap_or_default();
    let (envelope_extra, modality_rules) = if modality == "listening" {
        (
            ", \"listeningParts\"",
            "- This is a Listening question paper: you see the printed questions, not the audio. There is no reading passage.\n\
- Organise the task groups by Part (Part 1-4, also called Sections) and list every Part in listeningParts as {\"displayLabel\":\"Part 1\",\"expectedQuestionNumbers\":[1,2,3],\"taskIds\":[\"cloud-tg-1\"]}; every taskIds entry MUST be a taskId you defined.\n",
        )
    } else {
        ("", "")
    };
    format!(
        "You are recognising an {paper} paper from its ORIGINAL FILE into a COMPLETE authoring draft.\n\
Return JSON only. Do not return Markdown, HTML, JavaScript, explanations, or final export files.\n\
Return exactly one JSON object with the top-level keys \"taskGroups\", \"answerSlots\", \"answerKey\", \"unresolvedRegions\", \"sourceCoverageNotes\", \"warnings\"{envelope_extra}; the exact shape is outputContract.shape.\n\
This is NOT an outline and NOT a comparison summary: transcribe the FULL question content so it can be rendered.\n\
{repair}\n\
Rules that matter most:\n\
{modality_rules}\
- Transcribe every question's FULL prompt text; never abbreviate or summarise a question.\n\
- Transcribe every option label and its FULL text; keep one option bank per task group.\n\
- Do NOT transcribe the passage or script body. Transcribe the instructions and the notes / tables / diagrams / form text a task group depends on (into stimulus).\n\
- Give EVERY question an answerKey entry; use {{\"kind\":\"unresolved\"}} when the file gives no answer. Never invent answers.\n\
- Use TEMPORARY ids only (cloud-tg-1, cloud-q14, cloud-opt-a ...). Never copy a real database id.\n\
- Every responseGroups[].slotIds entry MUST be a key of answerSlots; every hostNodeId MUST be an id you defined here.\n\
- Every responseGroup needs kind, cardinality, assignment, scoringPolicy, duplicatePolicy and allowOptionReuse; every answerSlot needs slotId, questionNumber, displayLabel, hostType, interaction, participation and confidence; every content node needs type and id.\n\
- NEVER output jobId, schemaVersion, exam, quality, audit, reviewState, sourceDocumentId, provenanceStatus or any publish/verification flag — the backend owns those.\n\
- Report unreadable areas in unresolvedRegions (sourceFileId, 1-based pageIndex, reason, detail) and unverified coverage in sourceCoverageNotes.\n\
- Use only the enum values listed in outputContract.enums.\n\
Job JSON: {}\nSource file JSON: {}\nOutput contract JSON: {}",
        serde_json::to_string(input.get("job").unwrap_or(&Value::Null)).unwrap_or_default(),
        serde_json::to_string(input.get("sourceFile").unwrap_or(&Value::Null)).unwrap_or_default(),
        serde_json::to_string(input.get("outputContract").unwrap_or(&Value::Null)).unwrap_or_default()
    )
}

/// "IELTS Reading" / "IELTS Listening" for the candidate and repair prompts.
fn ielts_paper_label(modality: &str) -> &'static str {
    if crate::llm_suggestions::candidate_modality(modality) == "listening" {
        "IELTS Listening"
    } else {
        "IELTS Reading"
    }
}

/// 云端完整候选识别的执行体。
///
/// 证据面规则与云端大纲识别一致（模型看到的必须是**原文件**）：
/// - PDF：附原文件本身，失败回退到渲染页图；
/// - 非 PDF：附 `DocumentIRV2` 独立抽出的原文文本（`sourceText`），绝不读本地识别产物。
fn run_openai_compatible_authoring_candidate_llm(
    root: &Path,
    job_id: &str,
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    let mut warnings = Vec::<String>::new();
    let mut content = vec![json!({"type": "text", "text": authoring_candidate_prompt(input)})];
    let pdf_part = data_url_for_pdf(root, job_id, input)?;
    let had_pdf = pdf_part.is_some();
    if let Some(pdf_part) = pdf_part {
        content.push(pdf_part);
    } else if let Some(source_text) = input
        .get("sourceText")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
    {
        content.push(json!({
            "type": "text",
            "text": format!(
                "The original file is not a PDF, so no page image is attached. \
The extracted source text below is the ONLY evidence you may use; do not invent content.\n\
--- SOURCE TEXT BEGIN ---\n{source_text}\n--- SOURCE TEXT END ---"
            )
        }));
    } else {
        warnings.push("cloud_authoring_candidate_source_unavailable".to_string());
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

    let payload = post_with_pdf_image_fallback(
        root,
        job_id,
        profile,
        api_key,
        body,
        had_pdf,
        input,
        format!("{}\nThe direct PDF file request failed, so use the supplied rendered page images as the only evidence.", authoring_candidate_prompt(input)),
        "cloud_authoring_candidate_direct_pdf_failed_and_no_images",
        &mut warnings,
    )?;
    let content = openai_chat_content(&payload)?;
    let mut parsed = parse_llm_json_content(&content)?;
    validate_authoring_candidate_output(
        &mut parsed,
        input.get("modality").and_then(Value::as_str).unwrap_or("reading"),
    )?;
    if !warnings.is_empty() {
        if let Some(items) = parsed.get_mut("warnings").and_then(Value::as_array_mut) {
            for warning in warnings {
                items.push(json!(warning));
            }
        }
    }
    Ok(parsed)
}

/// 云端完整候选输出的**结构**校验。
///
/// 只校验「形状是否可用」：内容对不对是模型结合原文的语义判断，程序替代不了。
/// 但形状不对必须**具体**报错——原因会原样回给模型，让它定向修好再交一次。
/// 这里刻意**不**校验 quality / audit / 身份字段：那些由后端生成，模型写什么都不采信。
fn validate_authoring_candidate_output(output: &mut Value, modality: &str) -> CommandResult<()> {
    let modality = crate::llm_suggestions::candidate_modality(modality);
    let Some(object) = output.as_object() else {
        return Err("cloud_authoring_output_not_object".to_string());
    };
    let Some(groups) = object.get("taskGroups").and_then(Value::as_array) else {
        return Err("cloud_authoring_output_task_groups_missing".to_string());
    };
    if groups.is_empty() {
        return Err("cloud_authoring_output_task_groups_empty".to_string());
    }
    let slot_keys: std::collections::BTreeSet<String> = object
        .get("answerSlots")
        .and_then(Value::as_object)
        .map(|slots| slots.keys().cloned().collect())
        .unwrap_or_default();
    if slot_keys.is_empty() {
        return Err("cloud_authoring_output_answer_slots_empty".to_string());
    }

    let mut task_ids = std::collections::BTreeSet::<String>::new();
    for (index, group) in groups.iter().enumerate() {
        let Some(group_object) = group.as_object() else {
            return Err(format!("cloud_authoring_output_group_not_object:{index}"));
        };
        let task_id = non_empty_str(group_object.get("taskId"))
            .ok_or_else(|| format!("cloud_authoring_output_group_task_id_missing:{index}"))?;
        task_ids.insert(task_id.to_string());
        let Some(range) = group_object.get("displayRange").and_then(Value::as_object) else {
            return Err(format!("cloud_authoring_output_group_range_missing:{index}"));
        };
        validate_candidate_display_range(range)
            .map_err(|problem| format!("cloud_authoring_output_group_range_invalid:{index}:{problem}"))?;
        if non_empty_str(group_object.get("taskType")).is_none() {
            return Err(format!("cloud_authoring_output_group_task_type_missing:{index}"));
        }
        let Some(instructions) = group_object.get("instructions").and_then(Value::as_array) else {
            return Err(format!("cloud_authoring_output_group_instructions_missing:{index}"));
        };
        validate_candidate_nodes(instructions, &format!("{index}:instructions"))?;
        if let Some(stimulus) = group_object.get("stimulus") {
            let Some(stimulus) = stimulus.as_array() else {
                return Err(format!("cloud_authoring_output_group_stimulus_invalid:{index}"));
            };
            validate_candidate_nodes(stimulus, &format!("{index}:stimulus"))?;
        }
        if let Some(bank) = group_object.get("optionBank").filter(|bank| !bank.is_null()) {
            validate_candidate_option_bank(bank, index)?;
        }
        let Some(response_groups) = group_object.get("responseGroups").and_then(Value::as_array)
        else {
            return Err(format!(
                "cloud_authoring_output_group_response_groups_missing:{index}"
            ));
        };
        for (position, response_group) in response_groups.iter().enumerate() {
            let Some(response_object) = response_group.as_object() else {
                return Err(format!(
                    "cloud_authoring_output_response_group_not_object:{index}:{position}"
                ));
            };
            if non_empty_str(response_object.get("responseGroupId")).is_none() {
                return Err(format!(
                    "cloud_authoring_output_response_group_id_missing:{index}:{position}"
                ));
            }
            for field in ["kind", "assignment", "scoringPolicy", "duplicatePolicy"] {
                if non_empty_str(response_object.get(field)).is_none() {
                    return Err(format!(
                        "cloud_authoring_output_response_group_field_missing:{index}:{position}:{field}"
                    ));
                }
            }
            if !response_object
                .get("allowOptionReuse")
                .map(Value::is_boolean)
                .unwrap_or(false)
            {
                return Err(format!(
                    "cloud_authoring_output_response_group_field_missing:{index}:{position}:allowOptionReuse"
                ));
            }
            let cardinality_ok = response_object
                .get("cardinality")
                .and_then(Value::as_object)
                .map(|cardinality| {
                    cardinality.get("min").map(Value::is_u64).unwrap_or(false)
                        && cardinality.get("max").map(Value::is_u64).unwrap_or(false)
                })
                .unwrap_or(false);
            if !cardinality_ok {
                return Err(format!(
                    "cloud_authoring_output_response_group_cardinality_invalid:{index}:{position}:expected {{\"min\":1,\"max\":1}}"
                ));
            }
            if let Some(prompt) = response_object.get("prompt").filter(|prompt| !prompt.is_null()) {
                let Some(prompt) = prompt.as_array() else {
                    return Err(format!(
                        "cloud_authoring_output_response_group_prompt_invalid:{index}:{position}"
                    ));
                };
                validate_candidate_nodes(prompt, &format!("{index}:{position}:prompt"))?;
            }
            let Some(slot_ids) = response_object.get("slotIds").and_then(Value::as_array) else {
                return Err(format!(
                    "cloud_authoring_output_response_group_slot_ids_missing:{index}:{position}"
                ));
            };
            if slot_ids.is_empty() {
                return Err(format!(
                    "cloud_authoring_output_response_group_slot_ids_empty:{index}:{position}"
                ));
            }
            for slot_id in slot_ids {
                let Some(slot_id) = slot_id.as_str().filter(|value| !value.trim().is_empty()) else {
                    return Err(format!(
                        "cloud_authoring_output_response_group_slot_id_invalid:{index}:{position}"
                    ));
                };
                if !slot_keys.contains(slot_id) {
                    return Err(format!(
                        "cloud_authoring_output_slot_reference_dangling:{index}:{position}:{slot_id}"
                    ));
                }
            }
        }
    }

    if let Some(slots) = object.get("answerSlots").and_then(Value::as_object) {
        for (key, slot) in slots {
            let Some(slot_object) = slot.as_object() else {
                return Err(format!("cloud_authoring_output_slot_not_object:{key}"));
            };
            if slot_object.get("questionNumber").and_then(Value::as_u64).is_none() {
                return Err(format!(
                    "cloud_authoring_output_slot_question_number_missing:{key}"
                ));
            }
            for field in ["slotId", "displayLabel", "hostType", "interaction", "participation"] {
                if non_empty_str(slot_object.get(field)).is_none() {
                    return Err(format!(
                        "cloud_authoring_output_slot_field_missing:{key}:{field}"
                    ));
                }
            }
            if !slot_object
                .get("confidence")
                .and_then(Value::as_f64)
                .map(|value| (0.0..=1.0).contains(&value))
                .unwrap_or(false)
            {
                return Err(format!(
                    "cloud_authoring_output_slot_field_missing:{key}:confidence (a number in [0,1])"
                ));
            }
        }
    }
    if let Some(keys) = object.get("answerKey").and_then(Value::as_object) {
        for (key, value) in keys {
            if !slot_keys.contains(key) {
                return Err(format!("cloud_authoring_output_answer_key_dangling:{key}"));
            }
            validate_candidate_answer_value(value)
                .map_err(|problem| format!("cloud_authoring_output_answer_key_invalid:{key}:{problem}"))?;
        }
    }
    if let Some(regions) = object.get("unresolvedRegions") {
        let Some(regions) = regions.as_array() else {
            return Err("cloud_authoring_output_unresolved_regions_invalid".to_string());
        };
        for (index, region) in regions.iter().enumerate() {
            let Some(region_object) = region.as_object() else {
                return Err(format!("cloud_authoring_output_unresolved_region_invalid:{index}"));
            };
            if non_empty_str(region_object.get("sourceFileId")).is_none() {
                return Err(format!(
                    "cloud_authoring_output_unresolved_region_source_missing:{index}"
                ));
            }
            // 页索引 0 在本产品里是无效来源定位（见 `cloud_outline_group_quote_invalid`）。
            match region_object.get("pageIndex").and_then(Value::as_i64) {
                Some(page) if page >= 1 => {}
                _ => {
                    return Err(format!(
                        "cloud_authoring_output_unresolved_region_page_invalid:{index}"
                    ))
                }
            }
            for field in ["reason", "detail"] {
                if non_empty_str(region_object.get(field)).is_none() {
                    return Err(format!(
                        "cloud_authoring_output_unresolved_region_field_missing:{index}:{field}"
                    ));
                }
            }
        }
    }
    if modality == "listening" {
        if let Some(parts) = object.get("listeningParts").filter(|parts| !parts.is_null()) {
            let Some(parts) = parts.as_array() else {
                return Err("cloud_authoring_output_listening_parts_invalid".to_string());
            };
            for (index, part) in parts.iter().enumerate() {
                if non_empty_str(part.get("displayLabel")).is_none() {
                    return Err(format!(
                        "cloud_authoring_output_listening_part_label_missing:{index}"
                    ));
                }
                let Some(part_task_ids) = part.get("taskIds").and_then(Value::as_array) else {
                    return Err(format!(
                        "cloud_authoring_output_listening_part_task_ids_missing:{index}"
                    ));
                };
                for task_id in part_task_ids {
                    let task_id = task_id.as_str().unwrap_or_default();
                    if !task_ids.contains(task_id) {
                        return Err(format!(
                            "cloud_authoring_output_listening_part_task_dangling:{index}:{task_id}"
                        ));
                    }
                }
            }
        }
    }

    // 最后一道闸：用占位身份把回复**真的**标准化 + 反序列化一遍。上面逐字段的检查给模型
    // 具体的原因；这一步保证「网关放行 ⇒ finalize 必然成功」——任何 serde 在 finalize
    // 时才会发现的问题（未知枚举值、节点缺必填字段……）都在这里、在还能受约束重试的
    // 时候暴露，而不是在 finalize 里变成一次没有重试机会的整份失败。
    dry_run_candidate_finalize(output, modality)
        .map_err(|error| format!("cloud_authoring_output_schema_invalid:{error}"))
}

fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn validate_candidate_display_range(range: &serde_json::Map<String, Value>) -> Result<(), String> {
    match range.get("kind").and_then(Value::as_str) {
        Some("range") => {
            let start = range.get("start").and_then(Value::as_u64);
            let end = range.get("end").and_then(Value::as_u64);
            match (start, end) {
                (Some(start), Some(end)) if start >= 1 && end >= start => Ok(()),
                _ => Err("range needs start >= 1 and end >= start".to_string()),
            }
        }
        Some("set") => {
            let ok = range
                .get("values")
                .and_then(Value::as_array)
                .map(|values| !values.is_empty() && values.iter().all(Value::is_u64))
                .unwrap_or(false);
            if ok {
                Ok(())
            } else {
                Err("set needs a non-empty values array of question numbers".to_string())
            }
        }
        Some("mixed") => {
            if range.get("values").map(Value::is_array).unwrap_or(false) {
                Ok(())
            } else {
                Err("mixed needs a values array".to_string())
            }
        }
        _ => Err("kind must be range or set".to_string()),
    }
}

/// 内容节点的最小形状：每个节点都要有 `type` 与 `id`，子节点递归检查。
fn validate_candidate_nodes(nodes: &[Value], location: &str) -> CommandResult<()> {
    for (position, node) in nodes.iter().enumerate() {
        let Some(node_object) = node.as_object() else {
            return Err(format!(
                "cloud_authoring_output_content_node_invalid:{location}:{position}"
            ));
        };
        for field in ["type", "id"] {
            if non_empty_str(node_object.get(field)).is_none() {
                return Err(format!(
                    "cloud_authoring_output_content_node_field_missing:{location}:{position}:{field}"
                ));
            }
        }
        for child_key in ["children", "items", "rows", "cells", "caption"] {
            if let Some(children) = node_object.get(child_key).and_then(Value::as_array) {
                validate_candidate_nodes(children, &format!("{location}:{position}:{child_key}"))?;
            }
        }
    }
    Ok(())
}

fn validate_candidate_option_bank(bank: &Value, index: usize) -> CommandResult<()> {
    let Some(bank) = bank.as_object() else {
        return Err(format!("cloud_authoring_output_option_bank_invalid:{index}"));
    };
    for field in ["optionBankId", "scope"] {
        if non_empty_str(bank.get(field)).is_none() {
            return Err(format!(
                "cloud_authoring_output_option_bank_field_missing:{index}:{field}"
            ));
        }
    }
    if !bank.get("allowReuse").map(Value::is_boolean).unwrap_or(false) {
        return Err(format!(
            "cloud_authoring_output_option_bank_field_missing:{index}:allowReuse"
        ));
    }
    let Some(options) = bank.get("options").and_then(Value::as_array) else {
        return Err(format!(
            "cloud_authoring_output_option_bank_field_missing:{index}:options"
        ));
    };
    for (position, option) in options.iter().enumerate() {
        for field in ["optionId", "label"] {
            if non_empty_str(option.get(field)).is_none() {
                return Err(format!(
                    "cloud_authoring_output_option_field_missing:{index}:{position}:{field}"
                ));
            }
        }
        let Some(content) = option.get("content").and_then(Value::as_array) else {
            return Err(format!(
                "cloud_authoring_output_option_field_missing:{index}:{position}:content"
            ));
        };
        validate_candidate_nodes(content, &format!("{index}:option:{position}"))?;
    }
    Ok(())
}

fn validate_candidate_answer_value(value: &Value) -> Result<(), String> {
    validate_answer_value_shape(value)?;
    if value.get("kind").and_then(Value::as_str) == Some("option")
        && non_empty_str(value.get("assignment")).is_none()
    {
        return Err("option_assignment_missing".to_string());
    }
    Ok(())
}

/// 占位身份下跑一遍真实的标准化 + 反序列化（不写盘、不读库）。
fn dry_run_candidate_finalize(output: &Value, modality: &'static str) -> CommandResult<()> {
    let identity = crate::reconcile::candidate::CloudAuthoringIdentity {
        job_id: "gateway-dry-run",
        item_id: "gateway-dry-run",
        batch_id: "gateway-dry-run",
        source_file_id: "gateway-dry-run-source",
        source_sha256: "gateway-dry-run",
        base_edit_version: 0,
        generated_at: "1970-01-01T00:00:00Z",
        exam: json!({
            "examId": "gateway-dry-run",
            "title": "gateway-dry-run",
            "language": "en",
            "tags": [],
            "sourceFiles": [{"sourceFileId": "gateway-dry-run-source", "role": "question_paper"}]
        }),
        modality,
        source_document_id: "gateway-dry-run-document",
        extraction_mode: "pdf_native",
    };
    let normalized = crate::reconcile::candidate::normalize_cloud_authoring(&identity, None, output)?;
    crate::reconcile::candidate::cloud_authoring_candidate_from_normalized(&identity, normalized)
        .map(|_| ())
}

/// 修复回合的 prompt。
///
/// 工具清单来自 [`crate::schema::cloud_repair_v1::CLOUD_REPAIR_TOOLS`]——**唯一真源**。
/// 提示词里写一个、分发器不认，是这类循环最典型的漂移；这里刻意引用同一份常量。
fn repair_step_prompt(input: &Value) -> String {
    let tools = crate::schema::cloud_repair_v1::CLOUD_REPAIR_TOOLS.join(", ");
    let paper = ielts_paper_label(input.get("modality").and_then(Value::as_str).unwrap_or("reading"));
    // 只给模型它需要的：profile（baseUrl / model / timeout）、本机绝对路径、以及已经作为
    // 独立文本块附上的 DOCX 原文都不进 prompt。前两者对修复毫无用处还泄露本机信息，
    // 后者会让同一份原文在请求里出现两次。
    let mut prompt_input = input.clone();
    if let Some(object) = prompt_input.as_object_mut() {
        for key in ["profile", "pdfPath", "sourceText", "apiKey", "apiKeySource", "pages", "repairNote"] {
            object.remove(key);
        }
    }
    let repair = input
        .get("repairNote")
        .and_then(Value::as_str)
        .filter(|note| !note.trim().is_empty())
        .map(|note| {
            format!(
                "\nYour previous reply was REJECTED by the backend. Fix exactly this and reply with one JSON object again.\nRejection reason: {note}\n"
            )
        })
        .unwrap_or_default();
    format!(
        "You are repairing an {paper} authoring draft so it matches the ORIGINAL FILE.\n\
Return JSON only: exactly one object {{\"callId\":\"call-1\",\"tool\":\"read_draft\",\"arguments\":{{}}}} (tool is one of the allowed tools; arguments follow the tools table in the input).\n\
Do not return Markdown, prose, or several objects.\n\
Allowed tools (and nothing else): {tools}.\n\
{repair}\n\
Work like an editor: read what you need, then submit ONE batch of domain commands per turn, then read the result.\n\
- apply_edits requires baseVersion: pass the editVersion you actually saw from read_draft.\n\
- Use only the stable ids you were given. Never invent ids.\n\
- Attach evidence copied from the original file to content changes (sourceFileId, 1-based pageIndex, exact quote). A malformed evidence entry rejects the whole batch.\n\
- Never invent an answer the file does not give.\n\
- If a batch is rejected because a target is protected by a human edit, narrow the batch — do not retry the same commands.\n\
- The context lists the whole document. Do not claim the paper is verified because you handled the listed differences.\n\
\n\
DIFFERENCES ARE NOT AUTOMATICALLY THE USER'S PROBLEM.\n\
The first-pass cloud candidate is only an input and it can be wrong. For every difference listed in the context, the user should NOT have to decide it unless you genuinely cannot:\n\
- If the ORIGINAL FILE shows the current draft is right and the candidate is wrong: call record_ruling with ruling \"current_is_correct\" and the evidence you used. Do NOT edit anything.\n\
- If the original file does not settle it (unreadable, ambiguous, missing): call record_ruling with ruling \"cannot_resolve\" and say why.\n\
- If neither side is right: apply_edits to the correct content, then record_ruling \"current_is_correct\" for that difference (the candidate stays wrong).\n\
- A recorded ruling removes that difference from the user's list. Only rule on differences you actually checked against the file.\n\
- record_ruling cannot remove structural problems found by the backend validator. Fix those with apply_edits or leave them.\n\
\n\
When you are done, call finish. Put every question you could NOT settle in \"unresolved\": \
those become user-visible items, so leaving them out hides real uncertainty.\n\
Input JSON: {}",
        serde_json::to_string(&prompt_input).unwrap_or_default()
    )
}

/// 修复回合的执行体。证据面规则与完整候选识别一致（模型看到的必须是原文件）。
fn run_openai_compatible_repair_step_llm(
    root: &Path,
    job_id: &str,
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    let mut warnings = Vec::<String>::new();
    let mut content = vec![json!({"type": "text", "text": repair_step_prompt(input)})];
    let pdf_part = data_url_for_pdf(root, job_id, input)?;
    let had_pdf = pdf_part.is_some();
    if let Some(pdf_part) = pdf_part {
        content.push(pdf_part);
    } else if let Some(source_text) = input
        .get("sourceText")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
    {
        content.push(json!({
            "type": "text",
            "text": format!(
                "The original file is not a PDF, so no page image is attached. \
The extracted source text below is the ONLY evidence you may use; do not invent content.\n\
--- SOURCE TEXT BEGIN ---\n{source_text}\n--- SOURCE TEXT END ---"
            )
        }));
    } else {
        warnings.push("cloud_repair_source_unavailable".to_string());
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

    let payload = post_with_pdf_image_fallback(
        root,
        job_id,
        profile,
        api_key,
        body,
        had_pdf,
        input,
        format!("{}\nThe direct PDF file request failed, so use the supplied rendered page images as the only evidence.", repair_step_prompt(input)),
        "cloud_repair_step_direct_pdf_failed_and_no_images",
        &mut warnings,
    )?;
    let content = openai_chat_content(&payload)?;
    let mut parsed = parse_llm_json_content(&content)?;
    validate_repair_step_output(&mut parsed)?;
    if !warnings.is_empty() {
        if let Some(object) = parsed.as_object_mut() {
            object.insert("warnings".to_string(), json!(warnings));
        }
    }
    Ok(parsed)
}

/// 修复回合输出的**结构**校验：形状不对就给出具体原因，让模型定向改好。
///
/// 注意：这里**不**执行工具。执行发生在 `cloud_repair` 的分发器里，只有那里才知道
/// 运行归属、取消状态与真实稿件。
fn validate_repair_step_output(output: &mut Value) -> CommandResult<()> {
    let Some(object) = output.as_object() else {
        return Err("cloud_repair_step_not_object".to_string());
    };
    if object
        .get("callId")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        return Err("cloud_repair_step_call_id_missing".to_string());
    }
    let Some(tool) = object.get("tool").and_then(Value::as_str).map(str::trim) else {
        return Err("cloud_repair_step_tool_missing".to_string());
    };
    if !crate::schema::cloud_repair_v1::CLOUD_REPAIR_TOOLS.contains(&tool) {
        return Err(format!("cloud_repair_step_tool_unknown:{tool}"));
    }
    if let Some(arguments) = object.get("arguments") {
        if !arguments.is_object() && !arguments.is_null() {
            return Err("cloud_repair_step_arguments_not_object".to_string());
        }
    }
    Ok(())
}

/// A4：分歧裁决的 prompt。
///
/// 契约文字与 `make_adjudication_input` 的 `outputContract` 是**同一套规则**：
/// 「模型该返回什么」与「我们会校验什么」若各写一份，两者迟早漂移，
/// 而漂移的代价是模型产出被静默拒绝、用户看到「裁决失败」却无从解释。
/// A4 裁决的 prompt。
///
/// 与 A3 同一处缺陷、同一天由真实网关暴露：`validate_adjudication_output` 要求顶层
/// `rulings` 数组，而这条 prompt 原本从不声明它，真实模型返回被整份拒绝
/// （`adjudication_rulings_missing_or_invalid`）。同样只补信封声明，不动校验规则。
fn adjudication_prompt(input: &Value) -> String {
    let divergences = serde_json::to_string(input.get("divergences").unwrap_or(&Value::Null))
        .unwrap_or_else(|_| "[]".to_string());
    // 受约束修复：上一次回复被校验器拒了，把**被拒的原因原样**回给模型。
    // 不带原因地重试同一句话，只会再拿到同一种错误——那不是修复，只是多花一次配额。
    let repair = input
        .get("repairNote")
        .and_then(Value::as_str)
        .filter(|note| !note.trim().is_empty())
        .map(|note| {
            format!(
                "\n7. Your previous reply was REJECTED by our validator: {note}\n\
Fix exactly that and return JSON only."
            )
        })
        .unwrap_or_default();
    format!(
        "You are reconciling answers for one reading exam. The SAME question was answered by \
three independent sources: `local` (on-device recognition of the authoring draft), `cloud` \
(automated full-paper recognition) and `source` (an answer extracted from the original file).\n\
For EACH item in DIVERGENCES below, decide which of the given answers the ORIGINAL FILE supports.\n\
Return exactly one JSON object with this shape: {{\"rulings\":[{{\"decisionId\":\"a decisionId from \
DIVERGENCES\",\"chosen\":\"cloud\",\"value\":{{\"kind\":\"text\",\"values\":[\"TRUE\"]}},\
\"rationale\":\"what in the original file decided it\",\"confidence\":0.9}}]}}\n\
One ruling per decisionId, never two rulings for the same decisionId. `value` is required unless \
`chosen` is \"unresolved\", and uses one of exactly three shapes: \
{{\"kind\":\"text\",\"values\":[\"...\"]}}, {{\"kind\":\"option\",\"labels\":[\"A\"]}}, or \
{{\"kind\":\"unresolved\"}}. `confidence` is optional and must be a number in [0,1] when present.\n\
Hard rules:\n\
1. Answer only for the decisionId values listed; never invent an id.\n\
2. `chosen` must be exactly one of: local, cloud, source, unresolved.\n\
3. You may only SELECT one of the values already given. `value` must repeat that value exactly, \
byte for byte. Never invent a fourth value.\n\
4. If the original file does not settle the question, return chosen = \"unresolved\" and say why.\n\
5. `rationale` must not be empty; cite what in the original file decided it.\n\
6. Return JSON only.{repair}\n\
--- DIVERGENCES BEGIN ---\n{divergences}\n--- DIVERGENCES END ---"
    )
}

/// A4：分歧裁决的网关实现。
///
/// 证据面与云端识别一致：PDF 走 base64 原文件附件，非 PDF 走 `sourceText`。
/// 差别是**拿不到证据面时直接失败**，不像云端识别那样降级成一条警告后继续——
/// 裁决的全部意义就是读原文，在空证据上「裁定」只会产生一段看似可信的编造。
fn run_openai_compatible_adjudication_llm(
    root: &Path,
    job_id: &str,
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    if input
        .get("divergences")
        .and_then(Value::as_array)
        .map(|items| items.is_empty())
        .unwrap_or(true)
    {
        return Err("adjudication_no_divergences".to_string());
    }
    let mut content = vec![json!({"type": "text", "text": adjudication_prompt(input)})];
    if let Some(pdf_part) = data_url_for_pdf(root, job_id, input)? {
        content.push(pdf_part);
    } else if let Some(source_text) = input
        .get("sourceText")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
    {
        content.push(json!({
            "type": "text",
            "text": format!(
                "The original file is not a PDF, so no page image is attached. \
The extracted source text below is the ONLY evidence you may use; do not invent content.\n\
--- SOURCE TEXT BEGIN ---\n{source_text}\n--- SOURCE TEXT END ---"
            )
        }));
    } else {
        return Err("adjudication_no_evidence_surface".to_string());
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
    validate_adjudication_output(&mut parsed, input)?;
    Ok(parsed)
}

/// 裁决值的形状校验。
///
/// 刻意**不做**宽松转换（与 `to_answer_value` 的「裸字符串当成 text」相反）：
/// 一个形状不对的裁决值如果被静默改写成别的答案，模型的意思就被我们改掉了，
/// 而这种改动在结果里看不出来——正是最该 fail-closed 的地方。
///
/// A3 与 A4 共用：两个通道都要「模型返回的是一个合法 `AnswerValueV2`」这一条闸。
fn validate_answer_value_shape(value: &Value) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Err("not_object".to_string());
    };
    match object.get("kind").and_then(Value::as_str) {
        Some("text") => {
            let Some(values) = object.get("values").and_then(Value::as_array) else {
                return Err("text_values_missing".to_string());
            };
            if values.is_empty() || !values.iter().all(Value::is_string) {
                return Err("text_values_invalid".to_string());
            }
            Ok(())
        }
        Some("option") => {
            let Some(labels) = object.get("labels").and_then(Value::as_array) else {
                return Err("option_labels_missing".to_string());
            };
            if labels.is_empty() || !labels.iter().all(Value::is_string) {
                return Err("option_labels_invalid".to_string());
            }
            Ok(())
        }
        Some("unresolved") => Ok(()),
        _ => Err("kind_invalid".to_string()),
    }
}

/// A3：原文件核验的 prompt。
///
/// **信封声明不是可选的装饰**：`validate_source_verification_output` 对缺少顶层
/// `findings` 数组的回复整份拒绝。2026-09-20 用真实网关（grok-4.5）实测时，这条
/// prompt 因为只讲字段规则、从不声明外层结构，返回被校验器整份拒绝
/// （`source_verification_findings_missing_or_invalid`）。受控假模型是照着校验器写的，
/// 所以它永远"记得"这个键，这处 prompt 与校验器的不一致在假模型下不可观测。
/// 修的是 prompt，不是校验器——下面每条 hard rule 一条都没放宽。
///
/// 与裁决的关键差别写在正文里：**裁决是「三选一」，核验是「回原文查」**。
/// 把这两个任务说混，模型就会去挑一条链交差，而不是真的读原文——
/// 那样得到的「确认」没有任何证据含量。
fn source_verification_prompt(input: &Value) -> String {
    let items = serde_json::to_string(input.get("slots").unwrap_or(&Value::Null))
        .unwrap_or_else(|_| "[]".to_string());
    let repair = input
        .get("repairNote")
        .and_then(Value::as_str)
        .filter(|note| !note.trim().is_empty())
        .map(|note| {
            format!(
                "\n7. Your previous reply was REJECTED by our validator: {note}\n\
Fix exactly that and return JSON only."
            )
        })
        .unwrap_or_default();
    format!(
        "You are verifying answers for one reading exam against the ORIGINAL FILE attached below \
(this is the file the exam was imported from; read it yourself, including any answer key printed \
in it).\n\
For EACH item in SLOTS below, the on-device draft already holds `localValue`. Decide whether the \
ORIGINAL FILE supports that value.\n\
Return exactly one JSON object with this shape: {{\"findings\":[{{\"slotId\":\"a slotId from SLOTS\",\
\"questionNumber\":1,\"verdict\":\"confirmed\",\"quote\":\"short excerpt copied from the file\",\
\"pageIndex\":1,\"observedValue\":{{\"kind\":\"text\",\"values\":[\"FALSE\"]}},\"confidence\":0.9}}]}}\n\
One finding per slotId, never two findings for the same slotId. `quote` and `pageIndex` are required \
unless `verdict` is \"not_verifiable\"; `observedValue` is required only when `verdict` is \
\"contradicted\". Every answer value uses one of exactly three shapes: \
{{\"kind\":\"text\",\"values\":[\"...\"]}}, {{\"kind\":\"option\",\"labels\":[\"A\"]}}, or \
{{\"kind\":\"unresolved\"}}. `confidence` is optional and must be a number in [0,1] when present.\n\
Hard rules:\n\
1. Answer only for the slotId values listed; never invent an id.\n\
2. `verdict` must be exactly one of: confirmed, contradicted, not_verifiable.\n\
3. Use \"confirmed\" only when the file explicitly supports `localValue`; use \"contradicted\" \
only when the file explicitly gives a DIFFERENT value (then `observedValue` is required); \
otherwise use \"not_verifiable\".\n\
4. Both \"confirmed\" and \"contradicted\" are claims about the file and MUST carry a non-empty \
`quote` copied from the file plus the 1-based `pageIndex` it appears on. Never guess.\n\
5. Never invent a value that is not in the file. If you cannot read the file, say \
\"not_verifiable\" — an honest gap is far better than a plausible guess.\n\
6. Return JSON only.{repair}\n\
--- SLOTS BEGIN ---\n{items}\n--- SLOTS END ---"
    )
}

/// A3：原文件核验的网关实现。
///
/// 证据面与云端识别/裁决**完全一致**（PDF 附原文件；非 PDF 附独立抽取的原文文本），
/// 同样**不把「没有证据面」降级成警告**：核验的意义就是读原文，空证据上「核验」出来
/// 的结论是编造。
fn run_openai_compatible_source_verification_llm(
    root: &Path,
    job_id: &str,
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let profile = llm_profile(input);
    let model = llm_model(profile).ok_or_else(|| "llm_profile_model_missing".to_string())?;
    if input
        .get("slots")
        .and_then(Value::as_array)
        .map(|items| items.is_empty())
        .unwrap_or(true)
    {
        return Err("source_verification_no_slots".to_string());
    }
    let mut content = vec![json!({"type": "text", "text": source_verification_prompt(input)})];
    if let Some(pdf_part) = data_url_for_pdf(root, job_id, input)? {
        content.push(pdf_part);
    } else if let Some(source_text) = input
        .get("sourceText")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
    {
        content.push(json!({
            "type": "text",
            "text": format!(
                "The original file is not a PDF, so no page image is attached. \
The extracted source text below is the ONLY evidence you may use; do not invent content.\n\
--- SOURCE TEXT BEGIN ---\n{source_text}\n--- SOURCE TEXT END ---"
            )
        }));
    } else {
        return Err("source_verification_no_evidence_surface".to_string());
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
    validate_source_verification_output(&mut parsed, input)?;
    Ok(parsed)
}

/// A3 核验输出的契约校验。
///
/// 四条规则，每条都封死一类「把没核验写成已核验」：
/// 1. `slotId` 必须属于本次提交的集合，且不得重复——模型幻觉出的 id 会被下游按 id 匹配的
///    循环静默跳过，「模型编了一条」在结果里完全看不出来，因此整份拒绝；
/// 2. `questionNumber` 必须是整数（它是与本地槽位对齐的第二个键）；
/// 3. `verdict` 必须落在三值枚举内（与 prompt 的规则 2 同一份枚举）；
/// 4. `confirmed` / `contradicted` 必须同时具备**非空 `quote` 与 1-based `pageIndex`**，
///    且 `contradicted` 必须带形状合法的 `observedValue`。
///    没有出处的「确认」无法复核，等价于编造。
fn validate_source_verification_output(output: &mut Value, input: &Value) -> CommandResult<()> {
    let allowed: std::collections::BTreeSet<String> = input
        .get("slots")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("slotId").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if allowed.is_empty() {
        return Err("source_verification_request_missing_slot_ids".to_string());
    }
    let Some(object) = output.as_object_mut() else {
        return Err("source_verification_not_object".to_string());
    };
    let Some(findings) = object.get("findings").and_then(Value::as_array).cloned() else {
        return Err("source_verification_findings_missing_or_invalid".to_string());
    };
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (index, finding) in findings.iter().enumerate() {
        let Some(finding_object) = finding.as_object() else {
            return Err(format!("source_verification_finding_not_object:{index}"));
        };
        let Some(slot_id) = finding_object.get("slotId").and_then(Value::as_str) else {
            return Err(format!("source_verification_finding_slot_missing:{index}"));
        };
        if !allowed.contains(slot_id) {
            return Err(format!(
                "source_verification_finding_unknown_slot:{index}:{slot_id}"
            ));
        }
        if !seen.insert(slot_id.to_string()) {
            return Err(format!(
                "source_verification_finding_duplicate_slot:{index}:{slot_id}"
            ));
        }
        if !finding_object
            .get("questionNumber")
            .map(Value::is_u64)
            .unwrap_or(false)
        {
            return Err(format!(
                "source_verification_finding_question_number_invalid:{index}"
            ));
        }
        let Some(verdict) = finding_object.get("verdict").and_then(Value::as_str) else {
            return Err(format!("source_verification_finding_verdict_missing:{index}"));
        };
        if !matches!(verdict, "confirmed" | "contradicted" | "not_verifiable") {
            return Err(format!(
                "source_verification_finding_verdict_invalid:{index}:{verdict}"
            ));
        }
        if let Some(confidence) = finding_object.get("confidence") {
            let valid = confidence.is_null()
                || (confidence.is_number()
                    && confidence
                        .as_f64()
                        .map(|value| (0.0..=1.0).contains(&value))
                        .unwrap_or(false));
            if !valid {
                return Err(format!(
                    "source_verification_finding_confidence_invalid:{index}"
                ));
            }
        }
        if verdict == "not_verifiable" {
            continue;
        }
        // 「确认」与「有分歧」都是对原文件的断言：没有出处一律拒绝。
        let quote_present = finding_object
            .get("quote")
            .and_then(Value::as_str)
            .map(|quote| !quote.trim().is_empty())
            .unwrap_or(false);
        if !quote_present {
            return Err(format!(
                "source_verification_finding_quote_missing:{index}"
            ));
        }
        // 页码必须 ≥ 1：本仓库的约定里 0 表示「没有页码」。
        let page_ok = finding_object
            .get("pageIndex")
            .and_then(Value::as_u64)
            .map(|page| page >= 1)
            .unwrap_or(false);
        if !page_ok {
            return Err(format!(
                "source_verification_finding_page_index_invalid:{index}"
            ));
        }
        if verdict == "contradicted" {
            let Some(observed) = finding_object.get("observedValue") else {
                return Err(format!(
                    "source_verification_finding_observed_value_missing:{index}"
                ));
            };
            validate_answer_value_shape(observed)
                .map_err(|error| format!("source_verification_finding_observed_value_invalid:{index}:{error}"))?;
        }
    }
    Ok(())
}

/// A4 裁决输出的契约校验。三条规则都对应「不把模型说的话当成事实」：
///
/// 1. **`decisionId` 必须属于本次提交的 id 集合**——模型幻觉出的 id 会去改一个
///    根本不存在的决策项，而按 id 匹配的落实循环恰好会静默跳过它，于是「模型编了一条」
///    在结果里完全看不出来。这里必须整份拒绝。
/// 2. `chosen` 必须落在四值枚举内（与 prompt 的规则 2 同一份枚举）。
/// 3. `chosen != unresolved` 时 `value` 必填且形状合法、`rationale` 非空——
///    没有理由的「裁定」无法复核，等于没裁。
fn validate_adjudication_output(output: &mut Value, input: &Value) -> CommandResult<()> {
    let allowed: std::collections::BTreeSet<String> = input
        .get("divergences")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("decisionId").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if allowed.is_empty() {
        return Err("adjudication_request_missing_decision_ids".to_string());
    }
    let Some(object) = output.as_object_mut() else {
        return Err("adjudication_not_object".to_string());
    };
    let Some(rulings) = object.get("rulings").and_then(Value::as_array).cloned() else {
        return Err("adjudication_rulings_missing_or_invalid".to_string());
    };
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (index, ruling) in rulings.iter().enumerate() {
        let Some(ruling_object) = ruling.as_object() else {
            return Err(format!("adjudication_ruling_not_object:{index}"));
        };
        let Some(decision_id) = ruling_object.get("decisionId").and_then(Value::as_str) else {
            return Err(format!("adjudication_ruling_id_missing:{index}"));
        };
        if !allowed.contains(decision_id) {
            return Err(format!(
                "adjudication_ruling_unknown_decision_id:{index}:{decision_id}"
            ));
        }
        if !seen.insert(decision_id.to_string()) {
            return Err(format!(
                "adjudication_ruling_duplicate_decision_id:{index}:{decision_id}"
            ));
        }
        let Some(chosen) = ruling_object.get("chosen").and_then(Value::as_str) else {
            return Err(format!("adjudication_ruling_chosen_missing:{index}"));
        };
        if !matches!(chosen, "local" | "cloud" | "source" | "unresolved") {
            return Err(format!("adjudication_ruling_chosen_invalid:{index}:{chosen}"));
        }
        let rationale_present = ruling_object
            .get("rationale")
            .and_then(Value::as_str)
            .map(|text| !text.trim().is_empty())
            .unwrap_or(false);
        if !rationale_present {
            return Err(format!("adjudication_ruling_rationale_missing:{index}"));
        }
        if let Some(confidence) = ruling_object.get("confidence") {
            let valid = confidence.is_null()
                || (confidence.is_number()
                    && confidence
                        .as_f64()
                        .map(|value| (0.0..=1.0).contains(&value))
                        .unwrap_or(false));
            if !valid {
                return Err(format!(
                    "adjudication_ruling_confidence_invalid:{index}"
                ));
            }
        }
        if chosen == "unresolved" {
            continue;
        }
        let Some(value) = ruling_object.get("value") else {
            return Err(format!("adjudication_ruling_value_missing:{index}"));
        };
        validate_answer_value_shape(value)
            .map_err(|error| format!("adjudication_ruling_value_invalid:{index}:{error}"))?;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn request(ids: &[&str]) -> Value {
        json!({
            "divergences": ids
                .iter()
                .map(|id| json!({"decisionId": id}))
                .collect::<Vec<_>>()
        })
    }

    /// 幻觉 `decisionId` 必须**整份**拒绝。
    ///
    /// 若只按 id 匹配地落实裁定，模型编出来的 id 会被静默跳过——「模型编了一条」
    /// 在结果里完全看不出来。校验层必须提前把这个可能性掐掉。
    #[test]
    fn adjudication_output_rejects_a_hallucinated_decision_id() {
        let input = request(&["d:slot:slot-14:answer"]);
        let mut output = json!({"rulings":[{
            "decisionId": "d:slot:slot-99:answer",
            "chosen": "cloud",
            "value": {"kind":"text","values":["painting"]},
            "rationale": "made up"
        }]});
        let error =
            validate_adjudication_output(&mut output, &input).expect_err("必须拒绝幻觉 id");
        assert!(error.contains("unknown_decision_id"), "实际错误：{error}");
    }

    /// `chosen != unresolved` 时 `value` 必填，且形状必须是合法答案值。
    #[test]
    fn adjudication_output_requires_a_well_formed_value() {
        let input = request(&["d:slot:slot-14:answer"]);
        let mut missing = json!({"rulings":[{
            "decisionId": "d:slot:slot-14:answer",
            "chosen": "cloud",
            "rationale": "reason"
        }]});
        assert!(validate_adjudication_output(&mut missing, &input).is_err());

        let mut empty_values = json!({"rulings":[{
            "decisionId": "d:slot:slot-14:answer",
            "chosen": "cloud",
            "value": {"kind":"text","values":[]},
            "rationale": "reason"
        }]});
        let error = validate_adjudication_output(&mut empty_values, &input)
            .expect_err("空 values 不是合法答案值");
        assert!(error.contains("text_values_invalid"), "实际错误：{error}");
    }

    /// 没有理由的「裁定」无法复核，等于没裁。
    #[test]
    fn adjudication_output_requires_a_rationale() {
        let input = request(&["d:slot:slot-14:answer"]);
        let mut output = json!({"rulings":[{
            "decisionId": "d:slot:slot-14:answer",
            "chosen": "cloud",
            "value": {"kind":"text","values":["painting"]},
            "rationale": "   "
        }]});
        let error =
            validate_adjudication_output(&mut output, &input).expect_err("空 rationale 必须被拒绝");
        assert!(error.contains("rationale_missing"), "实际错误：{error}");
    }

    /// 合法裁定（含 `unresolved`）必须通过；`unresolved` 不要求 `value`。
    #[test]
    fn adjudication_output_accepts_well_formed_rulings() {
        let input = request(&["d:slot:slot-14:answer", "d:slot:slot-15:answer"]);
        let mut output = json!({"rulings":[
            {
                "decisionId": "d:slot:slot-14:answer",
                "chosen": "cloud",
                "value": {"kind":"text","values":["painting"]},
                "confidence": 0.8,
                "rationale": "page 2 lists painting"
            },
            {
                "decisionId": "d:slot:slot-15:answer",
                "chosen": "unresolved",
                "rationale": "the page does not settle question 15"
            }
        ]});
        assert!(validate_adjudication_output(&mut output, &input).is_ok());

        // 重复的 decisionId 会让落实循环写两次同一项，必须拒绝。
        let mut duplicated = json!({"rulings":[
            {"decisionId":"d:slot:slot-14:answer","chosen":"unresolved","rationale":"a"},
            {"decisionId":"d:slot:slot-14:answer","chosen":"unresolved","rationale":"b"}
        ]});
        let error = validate_adjudication_output(&mut duplicated, &input)
            .expect_err("重复 id 必须被拒绝");
        assert!(error.contains("duplicate_decision_id"), "实际错误：{error}");
    }

    // ── A3：原文件核验输出校验 ──────────────────────────────────────────

    fn source_request(slot_ids: &[&str]) -> Value {
        json!({
            "slots": slot_ids
                .iter()
                .map(|slot_id| json!({"slotId": slot_id}))
                .collect::<Vec<_>>()
        })
    }

    /// 幻觉 `slotId` 必须**整份**拒绝：按 id 匹配的合并循环会静默跳过未知 id，
    /// 于是「模型编了一条」在结果里完全看不出来。
    #[test]
    fn source_verification_output_rejects_a_hallucinated_slot_id() {
        let input = source_request(&["slot-14"]);
        let mut output = json!({"findings":[{
            "slotId": "slot-99",
            "questionNumber": 99,
            "verdict": "confirmed",
            "quote": "somewhere",
            "pageIndex": 1
        }]});
        let error = validate_source_verification_output(&mut output, &input)
            .expect_err("必须拒绝幻觉 slotId");
        assert!(error.contains("unknown_slot"), "实际错误：{error}");
    }

    /// 「确认」与「有分歧」都是对原文件的断言：没有 `quote` 或没有 1-based `pageIndex`
    /// 一律拒绝——没有出处的断言无法复核，等价于编造。
    #[test]
    fn source_verification_output_requires_a_locator_for_every_claim() {
        let input = source_request(&["slot-14"]);
        let cases = [
            json!({"slotId":"slot-14","questionNumber":14,"verdict":"confirmed","pageIndex":2}),
            json!({"slotId":"slot-14","questionNumber":14,"verdict":"confirmed","quote":"   ","pageIndex":2}),
            json!({"slotId":"slot-14","questionNumber":14,"verdict":"confirmed","quote":"text"}),
            // 0 表示「没有页码」，不是「第 0 页」。
            json!({"slotId":"slot-14","questionNumber":14,"verdict":"confirmed","quote":"text","pageIndex":0}),
        ];
        for case in cases {
            let mut output = json!({"findings": [case.clone()]});
            let error = validate_source_verification_output(&mut output, &input)
                .expect_err(&format!("必须拒绝无出处的断言：{case}"));
            assert!(
                error.contains("quote_missing") || error.contains("page_index_invalid"),
                "实际错误：{error}（case {case}）"
            );
        }
    }

    /// `contradicted` 必须带形状合法的 `observedValue`：模型说「原文是别的值」，
    /// 却给不出那个值，或给的形状无法当答案用，都不能采纳。
    #[test]
    fn source_verification_output_requires_an_observed_value_when_contradicted() {
        let input = source_request(&["slot-14"]);
        let mut missing = json!({"findings":[{
            "slotId": "slot-14",
            "questionNumber": 14,
            "verdict": "contradicted",
            "quote": "stencilling",
            "pageIndex": 1
        }]});
        let error = validate_source_verification_output(&mut missing, &input)
            .expect_err("contradicted 必须有 observedValue");
        assert!(error.contains("observed_value_missing"), "实际错误：{error}");

        let mut malformed = json!({"findings":[{
            "slotId": "slot-14",
            "questionNumber": 14,
            "verdict": "contradicted",
            "quote": "stencilling",
            "pageIndex": 1,
            "observedValue": {"kind": "text", "values": []}
        }]});
        let error = validate_source_verification_output(&mut malformed, &input)
            .expect_err("空 values 不是合法答案值");
        assert!(error.contains("observed_value_invalid"), "实际错误：{error}");
    }

    /// 合法输出必须通过；`not_verifiable` 不要求出处（诚实说明读不出来是被允许的）。
    #[test]
    fn source_verification_output_accepts_well_formed_findings() {
        let input = source_request(&["slot-14", "slot-15"]);
        let mut output = json!({"findings":[
            {
                "slotId": "slot-14",
                "questionNumber": 14,
                "verdict": "confirmed",
                "quote": "the artist was stencilling",
                "pageIndex": 2,
                "confidence": 0.9
            },
            {
                "slotId": "slot-15",
                "questionNumber": 15,
                "verdict": "not_verifiable"
            }
        ]});
        assert!(validate_source_verification_output(&mut output, &input).is_ok());

        // questionNumber 必须是整数：它是与本地槽位对齐的第二个键。
        let mut bad_number = json!({"findings":[{
            "slotId": "slot-14",
            "questionNumber": "14",
            "verdict": "not_verifiable"
        }]});
        let error = validate_source_verification_output(&mut bad_number, &input)
            .expect_err("questionNumber 必须是整数");
        assert!(error.contains("question_number_invalid"), "实际错误：{error}");

        // 重复 slotId 会让合并写两次同一项，必须拒绝。
        let mut duplicated = json!({"findings":[
            {"slotId":"slot-14","questionNumber":14,"verdict":"not_verifiable"},
            {"slotId":"slot-14","questionNumber":14,"verdict":"not_verifiable"}
        ]});
        let error = validate_source_verification_output(&mut duplicated, &input)
            .expect_err("重复 slotId 必须被拒绝");
        assert!(error.contains("duplicate_slot"), "实际错误：{error}");
    }

    /// 校验器要求的**顶层信封键**必须写在 prompt 里。
    ///
    /// 2026-09-20 真实网关实测暴露的缺陷：A3/A4 的 prompt 逐条写了字段规则，却从未
    /// 声明 `{"findings":[…]}` / `{"rulings":[…]}` 这个外层结构，而校验器对缺少该键的
    /// 回复整份拒绝。受控假模型是照着校验器写的，所以它永远"记得"这个键，这个
    /// prompt 与校验器的不一致在假模型下完全不可见。真实模型两条请求均被拒。
    #[test]
    fn every_prompt_declares_the_envelope_key_its_validator_requires() {
        let verification_input = json!({
            "slots": [{"slotId": "slot-1", "questionNumber": 1,
                "localValue": {"kind": "text", "values": ["TRUE"]}}]
        });
        let verification = source_verification_prompt(&verification_input);
        assert!(
            verification.contains("\"findings\""),
            "A3 prompt 必须声明顶层 findings 信封，否则模型无从得知校验器要什么：{verification}"
        );

        let adjudication_input = json!({
            "divergences": [{"decisionId": "decision-1", "questionNumber": 1}]
        });
        let adjudication = adjudication_prompt(&adjudication_input);
        assert!(
            adjudication.contains("\"rulings\""),
            "A4 prompt 必须声明顶层 rulings 信封：{adjudication}"
        );

        // 另外三类请求在真实网关上通过，正是因为它们已经声明了信封；
        // 一并钉住，避免以后有人把这几行删掉。
        let outline = cloud_outline_prompt(&json!({}));
        assert!(outline.contains("\"groups\""), "outline prompt 丢了信封声明");
        let repair = repair_step_prompt(&json!({"draft": {}, "observations": []}));
        assert!(
            repair.contains("\"callId\"") && repair.contains("\"tool\""),
            "repair prompt 丢了信封声明"
        );
    }

    // ── S1：传输与可观测性（本地假服务，无真实模型）──────────────────────────

    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn fake_read_request(stream: &mut std::net::TcpStream) -> String {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 8192];
        let mut expected: Option<usize> = None;
        loop {
            let read = match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            bytes.extend_from_slice(&buffer[..read]);
            if expected.is_none() {
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    expected = Some(end + 4 + length);
                }
            }
            if let Some(expected) = expected {
                if bytes.len() >= expected {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&bytes).to_string()
    }

    pub(crate) enum FakeReply {
        /// 读完请求后一直不回，模拟服务端生成超时。
        Stall(Duration),
        /// 按状态码回一段原样 body。
        Respond(u16, String),
    }

    /// 每个连接按顺序消费一个 `FakeReply`；收到的请求逐个送进 channel，用来数 POST 次数。
    /// 线程不 join：停住的连接要比网关超时活得久。
    pub(crate) fn fake_llm_server(replies: Vec<FakeReply>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake llm server");
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let request = fake_read_request(&mut stream);
                let _ = sender.send(request);
                match reply {
                    FakeReply::Stall(duration) => thread::sleep(duration),
                    FakeReply::Respond(status, body) => {
                        let response = format!(
                            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                }
            }
        });
        (format!("http://{address}/v1"), receiver)
    }

    pub(crate) fn chat_body(content: &str, finish_reason: &str) -> String {
        json!({
            "choices": [{"message": {"content": content}, "finish_reason": finish_reason}],
            "usage": {"prompt_tokens": 1200, "completion_tokens": 34, "total_tokens": 1234}
        })
        .to_string()
    }

    struct FakeJob {
        root: PathBuf,
        job_id: &'static str,
        input: Value,
    }

    impl Drop for FakeJob {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// 一个 PDF 候选请求：原文件 + 一张页图（页图存在，因此「回退到页图」在物理上可行——
    /// 断言的是它**不该**因为超时而发生）。
    fn fake_candidate_job(base_url: &str, job_id: &'static str) -> FakeJob {
        let root = std::env::temp_dir().join(format!(
            "llm-gateway-s1-{}-{}",
            job_id,
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let source_dir = job_dir(&root, job_id).join("source");
        fs::create_dir_all(&source_dir).unwrap();
        let pdf_path = source_dir.join("paper.pdf");
        fs::write(&pdf_path, b"%PDF-1.4 fake paper bytes").unwrap();
        let image_path = source_dir.join("page-1.png");
        fs::write(&image_path, [137u8, 80, 78, 71, 13, 10, 26, 10]).unwrap();
        let input = json!({
            "profile": {
                "profileId": "fake",
                "provider": "OpenAiCompatible",
                "baseUrl": base_url,
                "model": "fake-model",
                "temperature": 0,
                "timeoutMs": 1000,
                "forceJson": true
            },
            "sourceFile": {"fileId": "src-1", "originalName": "paper.pdf", "fileType": "pdf"},
            "pdfPath": pdf_path.to_string_lossy(),
            "pages": [{"pageIndex": 1, "images": [{"path": image_path.to_string_lossy(), "mimeType": "image/png", "assetId": "page-1"}]}],
            "outputContract": {}
        });
        FakeJob { root, job_id, input }
    }

    fn call_records(job: &FakeJob) -> Vec<Value> {
        fs::read_to_string(job_dir(&job.root, job.job_id).join("llm-calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// 真实事故：212 KB PDF 的整卷候选在 134 s / 144 s 以 `llm_timeout_budget_exhausted`
    /// 失败，而 profile 超时是 120 s。唯一能超出一个预算的路径是「直传 PDF 失败 →
    /// 页图回退拿一个全新预算」。超时说明服务端在算，换成页图只会再算一遍、再超一次。
    #[test]
    fn a_timeout_on_the_direct_pdf_request_never_triggers_the_image_fallback() {
        let (base_url, requests) = fake_llm_server(vec![
            FakeReply::Stall(Duration::from_secs(4)),
            FakeReply::Stall(Duration::from_secs(4)),
        ]);
        let job = fake_candidate_job(&base_url, "job-s1-stall");
        let started = Instant::now();
        let error = run_llm_gateway(
            &job.root,
            job.job_id,
            "generate_authoring_candidate",
            &job.input,
            None,
        )
        .expect_err("停住的服务必须让调用失败");
        let elapsed = started.elapsed();
        thread::sleep(Duration::from_millis(600));
        let posts = requests.try_iter().count();
        assert_eq!(posts, 1, "超时后不得再发页图回退请求（实际 {posts} 次 POST）；错误：{error}");
        assert!(
            elapsed < Duration::from_millis(2500),
            "一次调用不得超过一个超时预算太多：{elapsed:?}"
        );
        assert!(error.contains("timeout"), "错误必须如实说明是超时：{error}");

        let records = call_records(&job);
        let record = records.last().expect("必须留下调用记录");
        assert_eq!(record["ok"], json!(false));
        assert_eq!(record["error"].as_str(), Some(error.as_str()), "记录必须保留完整错误串：{record}");
        assert!(record["requestBytes"].as_u64().unwrap_or(0) > 0, "记录必须有请求体字节数：{record}");
        assert!(record["pdfBytes"].as_u64().unwrap_or(0) > 0, "记录必须有 PDF 字节数：{record}");
        let attempts = record["attempts"].as_array().expect("记录必须列出每次 HTTP 尝试");
        assert_eq!(attempts.len(), 1, "{record}");
        assert!(
            attempts[0]["error"].as_str().unwrap_or("").contains("llm_http_timeout"),
            "每次尝试必须带自己的错误：{record}"
        );
        assert_eq!(record["imageFallback"], json!(false), "{record}");
    }

    /// 回复解析不了时，原始回复必须落盘——否则「模型到底回了什么」事后永远无从得知。
    #[test]
    fn an_unparseable_reply_is_persisted_with_its_raw_content_and_usage() {
        let (base_url, _requests) = fake_llm_server(vec![FakeReply::Respond(
            200,
            chat_body("Sure! Here is the draft: {not json", "stop"),
        )]);
        let job = fake_candidate_job(&base_url, "job-s1-garbage");
        let error = run_llm_gateway(
            &job.root,
            job.job_id,
            "generate_authoring_candidate",
            &job.input,
            None,
        )
        .expect_err("无法解析的回复必须失败");
        assert!(error.starts_with("llm_json_parse_failed"), "{error}");

        let cache = job_dir(&job.root, job.job_id).join("cache").join("llm");
        let rejected = fs::read_dir(&cache)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.starts_with("generate_authoring_candidate-rejected-"))
                    .unwrap_or(false)
            })
            .expect("被拒回复必须落盘为 <command>-rejected-<stamp>.json");
        let saved: Value = serde_json::from_str(&fs::read_to_string(rejected).unwrap()).unwrap();
        assert_eq!(saved["rawContent"].as_str(), Some("Sure! Here is the draft: {not json"));
        assert_eq!(saved["error"].as_str(), Some(error.as_str()));
        assert_eq!(saved["usage"]["total_tokens"], json!(1234));

        let record = call_records(&job).pop().unwrap();
        assert_eq!(record["usage"]["total_tokens"], json!(1234), "{record}");
        assert_eq!(record["finishReason"].as_str(), Some("stop"), "{record}");
        assert_eq!(record["httpStatus"], json!(200), "{record}");
        assert!(record["rejectedPath"].as_str().is_some(), "{record}");
    }

    /// `finish_reason = length` 是截断，不是「模型写了坏 JSON」。两者的处理完全不同。
    #[test]
    fn a_length_finish_reason_is_reported_as_truncation_not_a_parse_failure() {
        let (base_url, _requests) = fake_llm_server(vec![FakeReply::Respond(
            200,
            chat_body("{\"taskGroups\":[{\"taskId\":\"cloud-tg-1\"", "length"),
        )]);
        let job = fake_candidate_job(&base_url, "job-s1-truncated");
        let error = run_llm_gateway(
            &job.root,
            job.job_id,
            "generate_authoring_candidate",
            &job.input,
            None,
        )
        .expect_err("被截断的回复必须失败");
        assert!(error.starts_with("llm_output_truncated"), "{error}");
        let record = call_records(&job).pop().unwrap();
        assert_eq!(record["errorClass"].as_str(), Some("llm_output_truncated"));
    }

    /// 请求体必须带输出上限：否则截断时既不知道上限是多少，也无法与 finish_reason 对账。
    #[test]
    fn every_request_carries_an_output_token_budget() {
        let (base_url, requests) =
            fake_llm_server(vec![FakeReply::Respond(200, chat_body("{}", "stop"))]);
        let job = fake_candidate_job(&base_url, "job-s1-max-tokens");
        let _ = run_llm_gateway(
            &job.root,
            job.job_id,
            "generate_authoring_candidate",
            &job.input,
            None,
        );
        let request = requests.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(request.contains("\"max_tokens\":"), "请求体缺少 max_tokens");
    }

    /// 非超时的直传失败（供应商不收 PDF 附件）仍然回退到页图；回退也失败时，
    /// 直传的错误不得被吞掉。
    #[test]
    fn a_failed_image_fallback_keeps_the_direct_pdf_error() {
        let (base_url, requests) = fake_llm_server(vec![
            FakeReply::Respond(400, "{\"error\":\"file parts unsupported\"}".to_string()),
            FakeReply::Respond(400, "{\"error\":\"image too small\"}".to_string()),
        ]);
        let job = fake_candidate_job(&base_url, "job-s1-fallback");
        let error = run_llm_gateway(
            &job.root,
            job.job_id,
            "generate_authoring_candidate",
            &job.input,
            None,
        )
        .expect_err("两条都失败");
        thread::sleep(Duration::from_millis(200));
        assert_eq!(requests.try_iter().count(), 2, "400 应当触发且只触发一次页图回退");
        assert!(error.contains("image too small"), "{error}");
        assert!(error.contains("file parts unsupported"), "直传 PDF 的错误被吞掉了：{error}");
        let record = call_records(&job).pop().unwrap();
        assert_eq!(record["imageFallback"], json!(true), "{record}");
        assert_eq!(record["imageCount"], json!(1), "{record}");
        assert_eq!(record["attempts"].as_array().map(Vec::len), Some(2), "{record}");
    }

    /// Settings 允许 600000 ms，网关却静默夹到 300 s：用户设的值必须真的生效。
    #[test]
    fn the_timeout_clamp_matches_the_settings_maximum() {
        assert_eq!(
            llm_timeout(&json!({"timeoutMs": 600_000}), 60_000),
            Duration::from_millis(600_000)
        );
        assert_eq!(
            llm_timeout(&json!({"timeoutMs": 900_000}), 60_000),
            Duration::from_millis(600_000)
        );
    }

    // ── S2：prompt 与校验器对齐 + 模态钩子 ────────────────────────────────

    /// 取出 prompt 正文里声明的形状示例（`marker` 之后的第一个 JSON 对象）。
    /// 测的是**真正发给模型的文字**，不是另一份手抄。
    fn declared_shape(prompt: &str, marker: &str) -> Value {
        let start = prompt
            .find(marker)
            .unwrap_or_else(|| panic!("prompt 缺少形状声明 `{marker}`：{prompt}"))
            + marker.len();
        let offset = start + prompt[start..].find('{').expect("形状声明后必须跟一个 JSON 对象");
        let end = balanced_json_end(prompt, offset).expect("形状示例必须是闭合的 JSON");
        serde_json::from_str(&prompt[offset..end])
            .unwrap_or_else(|error| panic!("形状示例必须是合法 JSON：{error}：{}", &prompt[offset..end]))
    }

    fn candidate_identity(modality: &'static str) -> crate::reconcile::candidate::CloudAuthoringIdentity<'static> {
        crate::reconcile::candidate::CloudAuthoringIdentity {
            job_id: "job-shape",
            item_id: "job-shape",
            batch_id: "batch-shape",
            source_file_id: "the source fileId you were given",
            source_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            base_edit_version: 1,
            generated_at: "2026-09-21T00:00:00Z",
            exam: json!({
                "examId": "job-shape",
                "title": "Shape",
                "language": "en",
                "tags": [],
                "sourceFiles": [{"sourceFileId": "src-1", "role": "question_paper"}]
            }),
            modality,
            source_document_id: "job-shape-document",
            extraction_mode: "pdf_native",
        }
    }

    fn finalize_candidate(output: &Value, modality: &'static str) -> CommandResult<()> {
        let identity = candidate_identity(modality);
        let normalized =
            crate::reconcile::candidate::normalize_cloud_authoring(&identity, None, output)?;
        crate::reconcile::candidate::cloud_authoring_candidate_from_normalized(&identity, normalized)
            .map(|_| ())
    }

    /// 每一个写给模型看的形状示例，都必须能通过**它自己的**校验器。
    ///
    /// 形状示例就是模型的范本：范本本身过不了校验，照做的模型必然被拒。候选还要再过
    /// 一道 finalize（标准化 + serde），因为那才是它真正落地的地方。
    #[test]
    fn every_declared_output_shape_passes_its_own_validator() {
        let profile = json!({"model": "fake"});
        let payload = json!({});

        let mut group = declared_shape(&llm_prompt(&json!({}), "extract_group"), "with this shape:");
        validate_llm_suggestion_output(&mut group, "extract_group", &profile, &payload)
            .expect("group prompt 的形状示例必须通过校验");

        let mut vision = declared_shape(&vision_prompt(&json!({})), "with shape");
        validate_vision_transcription_output(&mut vision, &profile, &payload)
            .expect("vision prompt 的形状示例必须通过校验");

        let mut vision_answer =
            declared_shape(&vision_answer_prompt(&json!({})), "with this shape:");
        validate_vision_answer_output(&mut vision_answer, &profile, &payload)
            .expect("vision answer prompt 的形状示例必须通过校验");
        let mut vision_answer_contract =
            crate::llm_suggestions::vision_answer_output_contract()["shape"].clone();
        validate_vision_answer_output(&mut vision_answer_contract, &profile, &payload)
            .expect("vision answer outputContract.shape 必须通过校验");

        let mut outline = declared_shape(&cloud_outline_prompt(&json!({})), "with this shape:");
        validate_cloud_outline_output(&mut outline, &profile, &payload)
            .expect("outline prompt 的形状示例必须通过校验");
        let mut outline_contract =
            crate::llm_suggestions::cloud_outline_output_contract()["shape"].clone();
        validate_cloud_outline_output(&mut outline_contract, &profile, &payload)
            .expect("outline outputContract.shape 必须通过校验");

        let mut repair = declared_shape(&repair_step_prompt(&json!({})), "exactly one object");
        validate_repair_step_output(&mut repair).expect("repair prompt 的形状示例必须通过校验");

        for modality in ["reading", "listening"] {
            let shape =
                crate::llm_suggestions::authoring_candidate_output_contract(modality)["shape"].clone();
            let mut validated = shape.clone();
            validate_authoring_candidate_output(&mut validated, modality)
                .unwrap_or_else(|error| panic!("{modality} 候选形状示例必须通过校验：{error}"));
            finalize_candidate(&shape, if modality == "reading" { "reading" } else { "listening" })
                .unwrap_or_else(|error| panic!("{modality} 候选形状示例必须能 finalize：{error}"));
        }
    }

    /// 正文（passage）是候选里最大的一块输出，却从来没有人比较或读取它：
    /// 输出契约不再要求它，finalize 也不依赖它。
    #[test]
    fn the_candidate_contract_no_longer_asks_for_the_passage() {
        for modality in ["reading", "listening"] {
            let contract = crate::llm_suggestions::authoring_candidate_output_contract(modality);
            assert!(contract["shape"].get("passage").is_none(), "{modality}: {contract}");
            let text = contract.to_string();
            assert!(
                !text.contains("passage.content"),
                "{modality} 契约规则仍在要求转写正文：{text}"
            );
        }
    }

    /// 候选校验器必须覆盖 serde 在 finalize 时要求的每一个字段：
    /// 否则缺字段的回复通过网关、在 finalize 才失败，而那里没有受约束重试。
    #[test]
    fn candidate_validator_rejects_everything_finalize_would_reject() {
        let base = crate::llm_suggestions::authoring_candidate_output_contract("reading")["shape"].clone();
        let slot_key = base["answerSlots"]
            .as_object()
            .and_then(|slots| slots.keys().next().cloned())
            .expect("形状示例必须有答案槽");
        let slot = format!("/answerSlots/{slot_key}");
        let key = format!("/answerKey/{slot_key}");
        let removals = [
            "/taskGroups/0/responseGroups/0/kind".to_string(),
            "/taskGroups/0/responseGroups/0/cardinality".to_string(),
            "/taskGroups/0/responseGroups/0/assignment".to_string(),
            "/taskGroups/0/responseGroups/0/scoringPolicy".to_string(),
            "/taskGroups/0/responseGroups/0/duplicatePolicy".to_string(),
            "/taskGroups/0/responseGroups/0/allowOptionReuse".to_string(),
            "/taskGroups/0/displayRange/end".to_string(),
            "/taskGroups/0/optionBank/allowReuse".to_string(),
            "/taskGroups/0/optionBank/options/0/label".to_string(),
            "/taskGroups/0/instructions/0/type".to_string(),
            "/taskGroups/0/instructions/0/id".to_string(),
            format!("{slot}/displayLabel"),
            format!("{slot}/hostType"),
            format!("{slot}/interaction"),
            format!("{slot}/participation"),
            format!("{slot}/confidence"),
            format!("{key}/assignment"),
            "/unresolvedRegions/0/reason".to_string(),
            "/unresolvedRegions/0/detail".to_string(),
        ];
        for pointer in removals {
            let mut output = base.clone();
            let (parent, field) = pointer.rsplit_once('/').unwrap();
            let removed = output
                .pointer_mut(parent)
                .and_then(Value::as_object_mut)
                .and_then(|object| object.remove(field));
            assert!(removed.is_some(), "测试前提：形状示例里应有 {pointer}");
            assert!(
                finalize_candidate(&output, "reading").is_err(),
                "测试前提：缺 {pointer} 时 finalize 应当失败"
            );
            let error = validate_authoring_candidate_output(&mut output, "reading")
                .expect_err(&format!("缺 {pointer} 必须在网关校验时就被拒"));
            assert!(error.starts_with("cloud_authoring_output_"), "{pointer}: {error}");
        }

        let replacements = [
            ("/taskGroups/0/taskType", json!("note_completion_questions")),
            ("/taskGroups/0/displayRange", json!({"kind": "set"})),
            ("/taskGroups/0/responseGroups/0/kind", json!("radio")),
            ("/taskGroups/0/responseGroups/0/cardinality", json!({"min": 1})),
        ];
        for (pointer, value) in replacements {
            let mut output = base.clone();
            *output.pointer_mut(pointer).unwrap() = value.clone();
            let error = validate_authoring_candidate_output(&mut output, "reading")
                .expect_err(&format!("{pointer}={value} 必须被拒"));
            assert!(error.starts_with("cloud_authoring_output_"), "{pointer}: {error}");
        }
        let mut output = base.clone();
        output["answerSlots"][&slot_key]["interaction"] = json!("button");
        assert!(validate_authoring_candidate_output(&mut output, "reading").is_err());
        let mut output = base.clone();
        output["answerKey"][&slot_key] = json!({"kind": "text", "values": []});
        assert!(validate_authoring_candidate_output(&mut output, "reading").is_err());
    }

    /// prompt 里写「可选」「已知时」「引用不到就降低置信度」，校验器却把它们当必填——
    /// 照 prompt 做的模型会被拒。prompt 必须说出校验器真正的要求。
    #[test]
    fn prompts_state_what_their_validators_require() {
        let outline = cloud_outline_prompt(&json!({}));
        assert!(!outline.contains("when known"), "layoutHint 是必填，不是「已知时」：{outline}");
        assert!(
            !outline.contains("lower that group confidence"),
            "引用是必填，不能用降低置信度代替：{outline}"
        );
        assert!(outline.contains("notesText"), "{outline}");
        let contract = crate::llm_suggestions::cloud_outline_output_contract().to_string();
        assert!(!contract.contains("Optional notes"), "notesText 是必填：{contract}");
        assert!(!contract.contains("lower group confidence"), "{contract}");

        let vision_answer = vision_answer_prompt(&json!({}));
        assert!(
            !vision_answer.contains("return no answers and explain it in warnings"),
            "空答案会被校验器拒绝，prompt 不能把它当成合法输出：{vision_answer}"
        );

        let group = llm_prompt(&json!({}), "extract_group");
        let item = declared_shape(&group, "Each questions[] item has this shape:");
        assert!(item.get("id").is_some(), "{group}");
    }

    /// 校验器要求的**顶层信封键**必须写在 prompt 正文里（不只在 outputContract 里）。
    #[test]
    fn candidate_vision_and_group_prompts_declare_their_envelope_keys() {
        for modality in ["reading", "listening"] {
            let prompt = authoring_candidate_prompt(&json!({"modality": modality}));
            for key in ["\"taskGroups\"", "\"answerSlots\"", "\"answerKey\""] {
                assert!(prompt.contains(key), "{modality} 候选 prompt 缺少 {key}：{prompt}");
            }
        }
        assert!(vision_prompt(&json!({})).contains("\"text\""));
        let vision_answer = vision_answer_prompt(&json!({}));
        assert!(vision_answer.contains("\"answers\"") && vision_answer.contains("\"evidence\""));
        let group = llm_prompt(&json!({}), "extract_group");
        for key in ["\"kind\"", "\"questions\"", "\"evidence\""] {
            assert!(group.contains(key), "group prompt 缺少 {key}");
        }
    }

    /// 模态钩子：listening 候选 / 修复用 Listening 的措辞与部分结构，reading 保持原样。
    #[test]
    fn candidate_and_repair_prompts_follow_the_modality() {
        let reading = authoring_candidate_prompt(&json!({"modality": "reading"}));
        assert!(reading.contains("IELTS Reading"), "{reading}");
        let default = authoring_candidate_prompt(&json!({}));
        assert!(default.contains("IELTS Reading"), "缺省模态必须是 reading");
        let listening = authoring_candidate_prompt(&json!({"modality": "listening"}));
        assert!(listening.contains("IELTS Listening"), "{listening}");
        assert!(!listening.contains("IELTS Reading"), "{listening}");
        assert!(listening.contains("Part"), "listening 候选必须按 Part 组织：{listening}");

        let repair = repair_step_prompt(&json!({"modality": "listening"}));
        assert!(repair.contains("IELTS Listening") && !repair.contains("IELTS Reading"));
        assert!(repair_step_prompt(&json!({})).contains("IELTS Reading"));

        let contract = crate::llm_suggestions::authoring_candidate_output_contract("listening");
        assert!(contract["shape"].get("listeningParts").is_some(), "{contract}");
        let mut dangling = contract["shape"].clone();
        dangling["listeningParts"][0]["taskIds"] = json!(["cloud-tg-404"]);
        let error = validate_authoring_candidate_output(&mut dangling, "listening")
            .expect_err("listeningParts 引用不存在的题组必须被拒");
        assert!(error.contains("listening_part"), "{error}");
    }

    /// 修复 prompt 只给模型它需要的东西：不带 profile（baseUrl/model/timeout）、
    /// 不带本机绝对路径、不把 DOCX 原文在 JSON 里再塞一遍。
    #[test]
    fn the_repair_prompt_carries_no_profile_path_or_duplicated_source_text() {
        let input = json!({
            "mode": "repair_authoring_step",
            "profile": {"baseUrl": "https://gateway.example/v1", "model": "secret-model-name", "timeoutMs": 120000},
            "apiKey": "sk-should-never-appear",
            "pdfPath": "C:\\Users\\someone\\AppData\\paper.pdf",
            "sourceText": "UNIQUE-SOURCE-TEXT-MARKER",
            "context": {"differences": [{"targetId": "q1"}]},
            "observations": []
        });
        let prompt = repair_step_prompt(&input);
        for forbidden in [
            "gateway.example",
            "secret-model-name",
            "sk-should-never-appear",
            "AppData",
            "UNIQUE-SOURCE-TEXT-MARKER",
            "timeoutMs",
        ] {
            assert!(!prompt.contains(forbidden), "修复 prompt 泄露了 {forbidden}：{prompt}");
        }
        assert!(prompt.contains("\"differences\""), "上下文必须保留：{prompt}");
        assert!(
            !prompt.contains("Content changes need evidence"),
            "校验器不强制证据，prompt 不能谎称必填"
        );
    }

    // ── S3：分块请求 ──────────────────────────────────────────────────────

    /// 分块请求的 prompt 必须把范围说清楚，校验器必须拒绝范围外的题号——
    /// 否则两块各自「顺手」识别了对方的题，合并时同一道题出现两次。
    #[test]
    fn a_chunk_request_is_scoped_to_its_questions_and_validated_against_them() {
        let input = json!({"modality": "reading", "chunk": {"label": "Questions 14-26", "questionNumbers": (14..=26).collect::<Vec<u32>>()}});
        let prompt = authoring_candidate_prompt(&input);
        assert!(prompt.contains("ONLY") && prompt.contains("Questions 14-26"), "{prompt}");

        let shape = crate::llm_suggestions::authoring_candidate_output_contract("reading")["shape"].clone();
        // 形状示例是第 1 题：对 14-26 这一块来说在范围外。
        let mut outside = shape.clone();
        let error = validate_authoring_candidate_output_for_chunk(&mut outside, "reading", Some(&input["chunk"]))
            .expect_err("范围外的题号必须被拒");
        assert!(error.starts_with("cloud_authoring_output_slot_outside_chunk"), "{error}");
        let mut unscoped = shape;
        validate_authoring_candidate_output_for_chunk(&mut unscoped, "reading", None)
            .expect("不分块时不做范围限制");
    }
}
