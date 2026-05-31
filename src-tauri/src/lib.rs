use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

pub type CommandResult<T> = Result<T, String>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum JobStatus {
    Draft,
    Uploaded,
    Parsed,
    SplitReady,
    AuthoringReady,
    NeedsHumanReview,
    ValidationFailed,
    PreviewReady,
    ExportReady,
    Published,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum WorkflowStep {
    Upload,
    DocumentReview,
    Split,
    Authoring,
    LlmReview,
    Preview,
    Export,
    Pack,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct IssueCounts {
    pub errors: u32,
    pub warnings: u32,
    #[serde(rename = "needsReview")]
    pub needs_review: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImportJob {
    #[serde(rename = "jobId")]
    pub job_id: String,
    pub title: String,
    pub status: JobStatus,
    pub category: Option<String>,
    pub frequency: Option<String>,
    pub tags: Vec<String>,
    #[serde(rename = "sourceFiles")]
    pub source_files: Vec<SourceFile>,
    #[serde(rename = "activeLlmProfileId")]
    pub active_llm_profile_id: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt")]
    pub updated_at: DateTime<Utc>,
    #[serde(rename = "currentStep")]
    pub current_step: WorkflowStep,
    #[serde(rename = "issueCounts")]
    pub issue_counts: IssueCounts,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SourceFile {
    #[serde(rename = "fileId")]
    pub file_id: String,
    #[serde(rename = "originalName")]
    pub original_name: String,
    #[serde(rename = "storedName")]
    pub stored_name: String,
    #[serde(rename = "fileType")]
    pub file_type: String,
    #[serde(rename = "sha256")]
    pub sha256: String,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: u64,
    pub role: String,
    #[serde(rename = "importedAt")]
    pub imported_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CreateJobInput {
    pub title: Option<String>,
    pub category: Option<String>,
    pub frequency: Option<String>,
    pub tags: Option<Vec<String>>,
    #[serde(rename = "llmProfileId")]
    pub llm_profile_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct JobFilter {
    pub status: Option<JobStatus>,
    pub search: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct JobMetaPatch {
    pub title: Option<String>,
    pub category: Option<String>,
    pub frequency: Option<String>,
    pub tags: Option<Vec<String>>,
    #[serde(rename = "activeLlmProfileId")]
    pub active_llm_profile_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ParseOptions {
    pub mode: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ManualTranscriptionInput {
    pub text: String,
    pub note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct VisionTranscriptionInput {
    #[serde(rename = "profileId")]
    pub profile_id: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AutoPipelineInput {
    #[serde(rename = "profileId")]
    pub profile_id: Option<String>,
    #[serde(rename = "confidenceThreshold")]
    pub confidence_threshold: Option<f64>,
    #[serde(rename = "parseMode")]
    pub parse_mode: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JobDetail {
    pub job: ImportJob,
    #[serde(rename = "documentIr")]
    pub document_ir: Option<Value>,
    #[serde(rename = "sourceReview")]
    pub source_review: Option<Value>,
    #[serde(rename = "splitCandidates")]
    pub split_candidates: Option<Value>,
    #[serde(rename = "authoringIr")]
    pub authoring_ir: Option<Value>,
    #[serde(rename = "validationReport")]
    pub validation_report: Option<Value>,
    #[serde(rename = "previewAssets")]
    pub preview_assets: Option<Value>,
    #[serde(rename = "pipelineReport")]
    pub pipeline_report: Option<Value>,
    #[serde(rename = "llmSuggestions")]
    pub llm_suggestions: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SaveLlmProfileInput {
    #[serde(rename = "profileId")]
    pub profile_id: Option<String>,
    pub name: String,
    pub provider: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    pub model: String,
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    pub temperature: f64,
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: u64,
    #[serde(rename = "forceJson")]
    pub force_json: bool,
    pub enabled: bool,
}

#[derive(Debug, Default)]
pub struct AppState;

fn app_root(app: &AppHandle) -> CommandResult<PathBuf> {
    app.path().app_data_dir().map_err(|error| error.to_string())
}

fn ensure_app_dirs(root: &Path) -> CommandResult<()> {
    for relative in [
        "config",
        "config/secrets",
        "jobs",
        "packs",
        "logs",
        "cache",
        "cache/parser",
        "cache/thumbnails",
        "cache/preview-server",
    ] {
        fs::create_dir_all(root.join(relative)).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn job_dir(root: &Path, job_id: &str) -> PathBuf {
    root.join("jobs").join(job_id)
}

fn ensure_job_dirs(path: &Path) -> CommandResult<()> {
    for relative in ["uploads", "preview", "exports"] {
        fs::create_dir_all(path.join(relative)).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> CommandResult<T> {
    let data = fs::read_to_string(path)
        .map_err(|error| format!("read_json:{}:{}", path.display(), error))?;
    serde_json::from_str(&data).map_err(|error| format!("parse_json:{}:{}", path.display(), error))
}

fn read_json_opt(path: &Path) -> CommandResult<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    read_json(path).map(Some)
}

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, data).map_err(|error| format!("write_json:{}:{}", path.display(), error))
}

fn write_text(path: &Path, value: &str) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, value).map_err(|error| format!("write_text:{}:{}", path.display(), error))
}

fn write_bytes(path: &Path, value: &[u8]) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, value).map_err(|error| format!("write_bytes:{}:{}", path.display(), error))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn zip_safe_path(path: &str) -> CommandResult<String> {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains("../")
        || normalized == ".."
    {
        return Err(format!("unsafe_zip_entry_path:{}", path));
    }
    Ok(normalized)
}

fn write_u16_le(writer: &mut fs::File, value: u16) -> CommandResult<()> {
    use std::io::Write;
    writer
        .write_all(&value.to_le_bytes())
        .map_err(|error| error.to_string())
}

fn write_u32_le(writer: &mut fs::File, value: u32) -> CommandResult<()> {
    use std::io::Write;
    writer
        .write_all(&value.to_le_bytes())
        .map_err(|error| error.to_string())
}

fn write_zip(path: &Path, entries: &[(String, Vec<u8>)]) -> CommandResult<u64> {
    use std::io::{Seek, Write};

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let mut file = fs::File::create(path)
        .map_err(|error| format!("create_zip:{}:{}", path.display(), error))?;
    let mut central = Vec::new();

    for (entry_path, content) in entries {
        let safe_path = zip_safe_path(entry_path)?;
        let name = safe_path.as_bytes();
        let offset = file.stream_position().map_err(|error| error.to_string())? as u32;
        let crc = crc32(content);
        let size = content.len() as u32;

        write_u32_le(&mut file, 0x0403_4b50)?;
        write_u16_le(&mut file, 20)?;
        write_u16_le(&mut file, 0)?;
        write_u16_le(&mut file, 0)?;
        write_u16_le(&mut file, 0)?;
        write_u16_le(&mut file, 33)?;
        write_u32_le(&mut file, crc)?;
        write_u32_le(&mut file, size)?;
        write_u32_le(&mut file, size)?;
        write_u16_le(&mut file, name.len() as u16)?;
        write_u16_le(&mut file, 0)?;
        file.write_all(name).map_err(|error| error.to_string())?;
        file.write_all(content).map_err(|error| error.to_string())?;

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&33u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }

    let central_offset = file.stream_position().map_err(|error| error.to_string())? as u32;
    file.write_all(&central)
        .map_err(|error| error.to_string())?;
    write_u32_le(&mut file, 0x0605_4b50)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, entries.len() as u16)?;
    write_u16_le(&mut file, entries.len() as u16)?;
    write_u32_le(&mut file, central.len() as u32)?;
    write_u32_le(&mut file, central_offset)?;
    write_u16_le(&mut file, 0)?;
    file.flush().map_err(|error| error.to_string())?;
    file.metadata()
        .map(|metadata| metadata.len())
        .map_err(|error| error.to_string())
}

fn append_text(path: &Path, value: &str) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("append_text:{}:{}", path.display(), error))?;
    file.write_all(value.as_bytes())
        .map_err(|error| format!("append_text:{}:{}", path.display(), error))
}

fn sidecar_candidates(relative: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        candidates.push(cwd.join(relative));
        candidates.push(cwd.join("..").join(relative));
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(relative));
            candidates.push(parent.join("..").join(relative));
            candidates.push(parent.join("resources").join(relative));
            candidates.push(parent.join("..").join("Resources").join(relative));
            candidates.push(parent.join("..").join("resources").join(relative));
            if let Some(resource_name) = Path::new(relative).file_name() {
                candidates.push(
                    parent
                        .join("resources")
                        .join("sidecars")
                        .join(resource_name),
                );
                candidates.push(
                    parent
                        .join("..")
                        .join("Resources")
                        .join("sidecars")
                        .join(resource_name),
                );
            }
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(relative),
    );
    candidates
}

fn find_sidecar(relative: &str) -> Option<PathBuf> {
    sidecar_candidates(relative)
        .into_iter()
        .find(|path| path.exists())
}

fn command_failure(command_name: &str, output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!(
        "{} exited with {:?}; stdout={}; stderr={}",
        command_name,
        output.status.code(),
        stdout.trim(),
        stderr.trim()
    )
}

fn load_job(root: &Path, job_id: &str) -> CommandResult<ImportJob> {
    read_json(&job_dir(root, job_id).join("job.json"))
}

fn save_job(root: &Path, job: &ImportJob) -> CommandResult<()> {
    write_json(&job_dir(root, &job.job_id).join("job.json"), job)
}

fn update_job(
    root: &Path,
    job_id: &str,
    mutator: impl FnOnce(&mut ImportJob),
) -> CommandResult<ImportJob> {
    let mut job = load_job(root, job_id)?;
    mutator(&mut job);
    job.updated_at = Utc::now();
    save_job(root, &job)?;
    Ok(job)
}

fn make_job(input: CreateJobInput) -> ImportJob {
    let now = Utc::now();
    let suffix = Uuid::new_v4().simple().to_string()[..8].to_string();
    ImportJob {
        job_id: format!("import-{}-{}", now.format("%Y%m%d%H%M%S"), suffix),
        title: input
            .title
            .unwrap_or_else(|| "Untitled Reading".to_string()),
        status: JobStatus::Draft,
        category: input.category.or_else(|| Some("P1".to_string())),
        frequency: input.frequency.or_else(|| Some("medium".to_string())),
        tags: input.tags.unwrap_or_default(),
        source_files: vec![],
        active_llm_profile_id: input.llm_profile_id,
        created_at: now,
        updated_at: now,
        current_step: WorkflowStep::Upload,
        issue_counts: IssueCounts::default(),
    }
}

fn file_type_from_name(name: &str) -> &'static str {
    match Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "pdf" => "pdf",
        "docx" => "docx",
        "txt" => "txt",
        "md" => "md",
        "png" | "jpg" | "jpeg" | "webp" => "image",
        _ => "unknown",
    }
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn safe_json_filename(value: &str) -> String {
    let sanitized = sanitize_filename(value);
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    bytes_to_hex(&hasher.finalize())
}

fn hash_file_or_path(path: &Path) -> CommandResult<(String, u64, Option<Vec<u8>>)> {
    if path.exists() && path.is_file() {
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let size = bytes.len() as u64;
        Ok((hash_bytes(&bytes), size, Some(bytes)))
    } else {
        Err(format!("source_file_not_readable:{}", path.display()))
    }
}

fn main_source_file(job: &ImportJob) -> Option<&SourceFile> {
    job.source_files
        .iter()
        .find(|source| source.role == "MainQuestion")
}

fn answer_key_sources(job: &ImportJob) -> Vec<&SourceFile> {
    job.source_files
        .iter()
        .filter(|source| source.role == "AnswerKey")
        .collect()
}

fn role_hint_for_text(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    if lower.starts_with("answers") || lower.contains("answer key") || lower.contains("答案") {
        Some("answer")
    } else if lower.contains("questions ")
        || lower.starts_with("question ")
        || lower.contains("true") && lower.contains("false") && lower.contains("not given")
        || lower.contains("choose one")
        || lower.contains("complete the")
    {
        Some("question")
    } else if lower.contains("reading passage") || lower.starts_with("passage ") {
        Some("passage")
    } else {
        None
    }
}

fn block_type_for_text(text: &str) -> &'static str {
    let trimmed = text.trim();
    if trimmed.starts_with('#')
        || trimmed.to_uppercase().starts_with("READING PASSAGE")
        || trimmed.to_lowercase().starts_with("questions ")
    {
        "header"
    } else if trimmed.contains('|') && trimmed.matches('|').count() >= 2 {
        "table"
    } else if trimmed
        .lines()
        .any(|line| line.trim_start().starts_with("- ") || line.trim_start().starts_with("* "))
    {
        "list"
    } else {
        "paragraph"
    }
}

fn markdownish_to_html(text: &str, block_type: &str) -> String {
    let trimmed = text.trim();
    match block_type {
        "header" => format!(
            "<h3>{}</h3>",
            html_escape(trimmed.trim_start_matches('#').trim())
        ),
        "table" => {
            let rows = trimmed
                .lines()
                .filter(|line| {
                    line.contains('|')
                        && !line
                            .trim()
                            .chars()
                            .all(|ch| matches!(ch, '|' | '-' | ':' | ' '))
                })
                .map(|line| {
                    let cells = line
                        .trim_matches('|')
                        .split('|')
                        .map(|cell| format!("<td>{}</td>", html_escape(cell.trim())))
                        .collect::<String>();
                    format!("<tr>{}</tr>", cells)
                })
                .collect::<String>();
            format!("<table>{}</table>", rows)
        }
        "list" => {
            let items = trimmed
                .lines()
                .map(|line| {
                    line.trim_start()
                        .trim_start_matches("- ")
                        .trim_start_matches("* ")
                        .trim()
                })
                .filter(|line| !line.is_empty())
                .map(|line| format!("<li>{}</li>", html_escape(line)))
                .collect::<String>();
            format!("<ul>{}</ul>", items)
        }
        _ => format!("<p>{}</p>", html_escape(trimmed)),
    }
}

fn text_document_ir(job: &ImportJob, source: &SourceFile, content: &str, mode: &str) -> Value {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks = Vec::new();
    let mut current = Vec::new();

    for line in normalized.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line.to_string());
        }
    }
    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }
    if blocks.is_empty() {
        blocks.push(job.title.clone());
    }

    let document_blocks = blocks
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let block_type = block_type_for_text(text);
            let role_hint = role_hint_for_text(text);
            let mut block = json!({
                "blockId": format!("b{:03}", index + 1),
                "blockType": block_type,
                "text": text,
                "html": markdownish_to_html(text, block_type),
                "bbox": [72, 72 + (index as i32 * 42), 520, 108 + (index as i32 * 42)],
                "confidence": 1.0
            });
            if let Some(role) = role_hint {
                block["roleHint"] = json!(role);
            }
            block
        })
        .collect::<Vec<_>>();

    json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": [{
            "pageIndex": 1,
            "width": 595,
            "height": 842,
            "blocks": document_blocks
        }],
        "assets": [],
        "parser": {
            "provider": "local-text-parser",
            "version": "0.2.0",
            "mode": mode,
            "warnings": [],
            "sourceFileId": source.file_id,
            "sourceStoredName": source.stored_name
        }
    })
}

fn manual_transcription_document_ir(job: &ImportJob, content: &str, note: Option<&str>) -> Value {
    let source = main_source_file(job)
        .cloned()
        .unwrap_or_else(|| SourceFile {
            file_id: "manual-source".to_string(),
            original_name: "manual-transcription.txt".to_string(),
            stored_name: "manual-transcription.txt".to_string(),
            file_type: "txt".to_string(),
            sha256: hash_bytes(content.as_bytes()),
            size_bytes: content.len() as u64,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        });
    let mut ir = text_document_ir(job, &source, content, "manual");
    if let Some(parser) = ir.get_mut("parser").and_then(Value::as_object_mut) {
        parser.insert("provider".to_string(), json!("manual-transcription"));
        parser.insert("version".to_string(), json!("0.3.0"));
        parser.insert("mode".to_string(), json!("manual"));
        parser.insert(
            "warnings".to_string(),
            json!(["manual transcription supplied by operator; verify against source PDF before publish"]),
        );
        if let Some(note) = note.filter(|value| !value.trim().is_empty()) {
            parser.insert("note".to_string(), json!(note));
        }
    }
    ir
}

fn vision_transcription_document_ir(
    job: &ImportJob,
    content: &str,
    confidence: f64,
    warnings: Vec<String>,
    evidence: Value,
    note: Option<&str>,
) -> Value {
    let source = main_source_file(job)
        .cloned()
        .unwrap_or_else(|| SourceFile {
            file_id: "vision-source".to_string(),
            original_name: "vision-transcription.txt".to_string(),
            stored_name: "vision-transcription.txt".to_string(),
            file_type: "txt".to_string(),
            sha256: hash_bytes(content.as_bytes()),
            size_bytes: content.len() as u64,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        });
    let mut ir = text_document_ir(job, &source, content, "ocr");
    let normalized_confidence = confidence.clamp(0.0, 1.0);
    for block in dynamic_document_blocks_mut(&mut ir) {
        if let Some(obj) = block.as_object_mut() {
            obj.insert("confidence".to_string(), json!(normalized_confidence));
        }
    }
    if let Some(parser) = ir.get_mut("parser").and_then(Value::as_object_mut) {
        parser.insert("provider".to_string(), json!("vision-llm-transcription"));
        parser.insert("version".to_string(), json!("0.1.0"));
        parser.insert("mode".to_string(), json!("ocr"));
        let mut parser_warnings =
            vec!["vision LLM transcription; verify against source PDF before publish".to_string()];
        parser_warnings.extend(
            warnings
                .into_iter()
                .filter(|warning| !warning.trim().is_empty()),
        );
        parser.insert("warnings".to_string(), json!(parser_warnings));
        parser.insert("evidence".to_string(), evidence);
        if let Some(note) = note.filter(|value| !value.trim().is_empty()) {
            parser.insert("note".to_string(), json!(note));
        }
    }
    ir
}

fn append_parser_warning(ir: &mut Value, warning: String) {
    if let Some(parser) = ir.get_mut("parser").and_then(Value::as_object_mut) {
        let warnings = parser.entry("warnings").or_insert_with(|| json!([]));
        if let Some(items) = warnings.as_array_mut() {
            items.push(json!(warning));
        }
    }
}

fn parse_with_python_sidecar(
    job_id: &str,
    input_path: &Path,
    output_path: &Path,
    mode: &str,
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/python-parser/parser.py")
        .ok_or_else(|| "python_parser_sidecar_missing".to_string())?;
    let output = Command::new("python3")
        .arg(&script)
        .arg("parse")
        .arg("--input")
        .arg(input_path)
        .arg("--output")
        .arg(output_path)
        .arg("--job-id")
        .arg(job_id)
        .arg("--mode")
        .arg(mode)
        .output()
        .map_err(|error| format!("python_parser_spawn_failed:{}:{}", script.display(), error))?;
    if !output.status.success() {
        return Err(command_failure("python-parser", &output));
    }
    read_json(output_path)
}

fn extract_pdf_images_with_python_sidecar(
    job_id: &str,
    input_path: &Path,
    output_path: &Path,
    asset_dir: &Path,
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/python-parser/parser.py")
        .ok_or_else(|| "python_parser_sidecar_missing".to_string())?;
    let output = Command::new("python3")
        .arg(&script)
        .arg("extract_pdf_images")
        .arg("--input")
        .arg(input_path)
        .arg("--output")
        .arg(output_path)
        .arg("--job-id")
        .arg(job_id)
        .arg("--asset-dir")
        .arg(asset_dir)
        .output()
        .map_err(|error| format!("python_parser_spawn_failed:{}:{}", script.display(), error))?;
    if !output.status.success() {
        return Err(command_failure("python-parser:extract_pdf_images", &output));
    }
    read_json(output_path)
}

fn parse_source_document(
    job: &ImportJob,
    source: &SourceFile,
    upload_path: &Path,
    output_path: &Path,
    mode: &str,
) -> CommandResult<Value> {
    match parse_with_python_sidecar(&job.job_id, upload_path, output_path, mode) {
        Ok(ir) => Ok(ir),
        Err(error) => {
            if matches!(source.file_type.as_str(), "txt" | "md") {
                match fs::read_to_string(upload_path) {
                    Ok(content) => {
                        let mut ir = text_document_ir(job, source, &content, mode);
                        append_parser_warning(
                            &mut ir,
                            format!(
                                "python parser sidecar fallback for {}: {}",
                                source.file_type, error
                            ),
                        );
                        Ok(ir)
                    }
                    Err(read_error) => Ok(parser_failure_document_ir(
                        job,
                        source,
                        mode,
                        &format!(
                            "python parser failed and text fallback could not read source: {}; {}",
                            error, read_error
                        ),
                    )),
                }
            } else {
                Ok(parser_failure_document_ir(job, source, mode, &error))
            }
        }
    }
}

fn parser_failure_document_ir(
    job: &ImportJob,
    source: &SourceFile,
    mode: &str,
    error: &str,
) -> Value {
    let warning = format!(
        "parser failed for {} source {}; manual review required: {}",
        source.file_type, source.original_name, error
    );
    json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": [{
            "pageIndex": 1,
            "width": 595,
            "height": 842,
            "blocks": [{
                "blockId": "parser-failure",
                "blockType": "paragraph",
                "text": format!("[Parser failed for {}. Manual review required.]", source.original_name),
                "html": format!("<p>{}</p>", html_escape(&format!("[Parser failed for {}. Manual review required.]", source.original_name))),
                "bbox": [72, 72, 520, 120],
                "confidence": 0.0,
                "roleHint": "ignore"
            }]
        }],
        "assets": [],
        "parser": {
            "provider": "python-parser-sidecar:failure",
            "version": "0.3.0",
            "mode": mode,
            "warnings": [warning, "no-sample-content-generated"],
            "sourceFileId": source.file_id,
            "sourceStoredName": source.stored_name
        }
    })
}

fn missing_source_document_ir(job: &ImportJob, mode: &str, reason: &str) -> Value {
    json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": [{
            "pageIndex": 1,
            "width": 595,
            "height": 842,
            "blocks": [{
                "blockId": "source-missing",
                "blockType": "paragraph",
                "text": "[No main source file is available. Manual import/review required.]",
                "html": "<p>[No main source file is available. Manual import/review required.]</p>",
                "bbox": [72, 72, 520, 120],
                "confidence": 0.0,
                "roleHint": "ignore"
            }]
        }],
        "assets": [],
        "parser": {
            "provider": "local-parser:source-missing",
            "version": "0.3.0",
            "mode": mode,
            "warnings": [format!("main source unavailable; manual review required: {}", reason), "no-sample-content-generated"],
            "sourceFileId": null,
            "sourceStoredName": null
        }
    })
}

#[cfg(test)]
fn sample_document_ir(job: &ImportJob, mode: &str) -> Value {
    json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": [{
            "pageIndex": 1,
            "width": 595,
            "height": 842,
            "blocks": [
                {"blockId":"b001","blockType":"header","text":"READING PASSAGE 1","html":"<h2>READING PASSAGE 1</h2>","bbox":[72,60,460,88],"confidence":0.99,"roleHint":"passage"},
                {"blockId":"b002","blockType":"paragraph","text":job.title,"html":format!("<h3>{}</h3>", html_escape(&job.title)),"bbox":[72,100,520,130],"confidence":0.97,"roleHint":"passage"},
                {"blockId":"b003","blockType":"paragraph","text":"Detective fiction developed from short literary experiments into a recognizable public genre. Early writers used clues, alibis, and narrators to teach readers how to reason through a mystery.","html":"<p>Detective fiction developed from short literary experiments into a recognizable public genre. Early writers used clues, alibis, and narrators to teach readers how to reason through a mystery.</p>","bbox":[72,145,520,210],"confidence":0.96,"roleHint":"passage"},
                {"blockId":"b004","blockType":"paragraph","text":"Questions 1-5 Do the following statements agree with the information given in Reading Passage 1? TRUE if the statement agrees, FALSE if it contradicts, NOT GIVEN if there is no information.","html":"<h3>Questions 1-5</h3><p>Do the following statements agree with the information given in Reading Passage 1?</p>","bbox":[72,250,520,320],"confidence":0.94,"roleHint":"question"},
                {"blockId":"b005","blockType":"list","text":"1 Detective fiction first appeared as a public genre before short literary experiments. 2 Early detective stories trained readers to interpret clues. 3 Every early detective writer used a police officer as narrator. 4 Alibis were one device used in the genre. 5 The passage says detective fiction disappeared in the twentieth century.","html":"<ol><li>Detective fiction first appeared as a public genre before short literary experiments.</li><li>Early detective stories trained readers to interpret clues.</li><li>Every early detective writer used a police officer as narrator.</li><li>Alibis were one device used in the genre.</li><li>The passage says detective fiction disappeared in the twentieth century.</li></ol>","bbox":[72,330,520,520],"confidence":0.91,"roleHint":"question"},
                {"blockId":"b006","blockType":"paragraph","text":"Questions 6-8 Complete the table below. Choose ONE WORD ONLY from the passage for each answer.","html":"<h3>Questions 6-8</h3><p>Complete the table below. Choose ONE WORD ONLY from the passage for each answer.</p>","bbox":[72,540,520,590],"confidence":0.95,"roleHint":"question"},
                {"blockId":"b007","blockType":"table","text":"Feature | Function | clues | help readers reason | alibis | complicate the mystery | narrators | guide interpretation","html":"<table><tr><th>Feature</th><th>Function</th></tr><tr><td>clues</td><td>help readers reason</td></tr><tr><td>alibis</td><td>complicate the mystery</td></tr><tr><td>narrators</td><td>guide interpretation</td></tr></table>","bbox":[72,600,520,760],"confidence":0.93,"roleHint":"question"},
                {"blockId":"b008","blockType":"paragraph","text":"Answers 1 FALSE 2 TRUE 3 NOT GIVEN 4 TRUE 5 FALSE 6 clues 7 alibis 8 narrators","html":"<p>Answers: 1 FALSE; 2 TRUE; 3 NOT GIVEN; 4 TRUE; 5 FALSE; 6 clues; 7 alibis; 8 narrators.</p>","bbox":[72,780,520,820],"confidence":0.9,"roleHint":"answer"}
            ]
        }],
        "assets": [],
        "parser": {"provider":"local-parser-placeholder","version":"0.1.0","mode":mode,"warnings": if mode == "ocr" { json!(["OCR confidence is simulated; human confirmation required."]) } else { json!([]) }}
    })
}

fn split_candidates(job_id: &str) -> Value {
    json!({
        "jobId": job_id,
        "passageCandidates": [{"range":["b001","b002","b003"],"title":"The Rise and Fall of Detective Stories","categoryHint":"P1"}],
        "questionGroupCandidates": [
            {"groupId":"group-1","heading":"Questions 1-5","questionRange":[1,5],"instructionText":"Do the following statements agree with the information given in Reading Passage 1?","blockIds":["b004","b005"],"kindHint":"true_false_not_given","confidence":0.88},
            {"groupId":"group-2","heading":"Questions 6-8","questionRange":[6,8],"instructionText":"Complete the table below. Choose ONE WORD ONLY from the passage for each answer.","blockIds":["b006","b007"],"kindHint":"table_completion","confidence":0.84}
        ],
        "answerKeyCandidates": [{"source":"answer-block:b008","answers":{"1":"FALSE","2":"TRUE","3":"NOT GIVEN","4":"TRUE","5":"FALSE","6":"clues","7":"alibis","8":"narrators"}}],
        "issues": []
    })
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_html(input: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                output.push(' ');
            }
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    collapse_whitespace(&output)
}

fn dynamic_document_blocks(doc: Option<&Value>) -> Vec<Value> {
    doc.and_then(|value| value.get("pages"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|page| {
            page.get("blocks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .cloned()
        .collect()
}

fn dynamic_document_blocks_mut(doc: &mut Value) -> Vec<&mut Value> {
    doc.get_mut("pages")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .flat_map(|page| {
            page.get_mut("blocks")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
        })
        .collect()
}

fn dynamic_block_text(block: &Value) -> String {
    let text = block
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if !text.is_empty() {
        return collapse_whitespace(text);
    }
    block
        .get("html")
        .and_then(Value::as_str)
        .map(strip_html)
        .unwrap_or_default()
}

fn dynamic_block_id(block: &Value) -> String {
    block
        .get("blockId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn dynamic_block_role(block: &Value) -> &str {
    block
        .get("roleHint")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn find_question_word(text: &str) -> Option<(usize, usize)> {
    let lower = text.to_lowercase();
    if let Some(index) = lower.find("questions") {
        Some((index, "questions".len()))
    } else {
        lower
            .find("question")
            .map(|index| (index, "question".len()))
    }
}

fn parse_number_after(text: &str, start: usize) -> Option<(u32, usize)> {
    let mut index = start.min(text.len());
    while let Some(ch) = text[index..].chars().next() {
        if ch.is_whitespace() || matches!(ch, ':' | '#' | '.' | ')') {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    let number_start = index;
    while let Some(ch) = text[index..].chars().next() {
        if ch.is_ascii_digit() {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    if index == number_start {
        return None;
    }
    text[number_start..index]
        .parse::<u32>()
        .ok()
        .map(|value| (value, index))
}

fn detect_dynamic_question_range(text: &str) -> Option<(u32, u32)> {
    let (word_index, word_len) = find_question_word(text)?;
    let (start, mut index) = parse_number_after(text, word_index + word_len)?;
    while let Some(ch) = text[index..].chars().next() {
        if ch.is_whitespace() {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    if let Some(ch) = text[index..].chars().next() {
        if matches!(ch, '-' | '\u{2013}' | '\u{2014}') {
            if let Some((end, _)) = parse_number_after(text, index + ch.len_utf8()) {
                return Some((start, end));
            }
        }
    }
    Some((start, start))
}

fn is_dynamic_question_block(block: &Value) -> bool {
    dynamic_block_role(block) == "question"
        || detect_dynamic_question_range(&dynamic_block_text(block)).is_some()
}

fn is_dynamic_answer_block(block: &Value) -> bool {
    let lower = dynamic_block_text(block).to_lowercase();
    dynamic_block_role(block) == "answer"
        || lower.starts_with("answers")
        || lower.contains("answer key")
}

fn detect_dynamic_group_kind(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if lower.contains("true") && lower.contains("false") && lower.contains("not given") {
        "true_false_not_given"
    } else if lower.contains("yes") && lower.contains("no") && lower.contains("not given") {
        "yes_no_not_given"
    } else if lower.contains("complete the table")
        || lower.contains("table below")
        || (lower.contains('|') && lower.contains("complete"))
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

fn dynamic_question_heading(start: u32, end: u32) -> String {
    if start == end {
        format!("Questions {}", start)
    } else {
        format!("Questions {}-{}", start, end)
    }
}

fn normalized_answer_value(raw: &str) -> Value {
    let upper = raw.trim().to_uppercase();
    if matches!(
        upper.as_str(),
        "TRUE" | "FALSE" | "YES" | "NO" | "NOT GIVEN" | "A" | "B" | "C" | "D"
    ) {
        json!(upper)
    } else {
        json!(raw.trim())
    }
}

fn clean_answer_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| matches!(ch, '.' | ')' | '(' | ':' | ';' | ','))
        .to_string()
}

fn parse_dynamic_answer_text(text: &str) -> serde_json::Map<String, Value> {
    let normalized = text
        .chars()
        .map(|ch| {
            if matches!(ch, '\n' | '\r' | ';' | ',') {
                ' '
            } else {
                ch
            }
        })
        .collect::<String>();
    let tokens = normalized
        .split_whitespace()
        .map(clean_answer_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut answers = serde_json::Map::new();
    let mut index = 0;
    while index < tokens.len() {
        if let Ok(number) = tokens[index].parse::<u32>() {
            index += 1;
            let mut value_tokens = Vec::new();
            while index < tokens.len() && tokens[index].parse::<u32>().is_err() {
                value_tokens.push(tokens[index].clone());
                index += 1;
            }
            if !value_tokens.is_empty() {
                answers.insert(
                    number.to_string(),
                    normalized_answer_value(&value_tokens.join(" ")),
                );
            }
        } else {
            index += 1;
        }
    }
    answers
}

fn infer_dynamic_passage_title(job: &ImportJob, passage_blocks: &[Value]) -> String {
    passage_blocks
        .iter()
        .map(dynamic_block_text)
        .find(|text| !text.is_empty() && !text.to_uppercase().starts_with("READING PASSAGE"))
        .unwrap_or_else(|| job.title.clone())
}

fn make_dynamic_split_candidates(job_id: &str, job: &ImportJob, doc: Option<&Value>) -> Value {
    let blocks = dynamic_document_blocks(doc);
    if blocks.is_empty() {
        return split_candidates(job_id);
    }

    let first_question_index = blocks.iter().position(is_dynamic_question_block);
    let first_answer_index = blocks.iter().position(is_dynamic_answer_block);
    let passage_blocks = blocks
        .iter()
        .enumerate()
        .filter(|(index, block)| {
            dynamic_block_role(block) == "passage"
                || first_question_index
                    .map(|first| *index < first && dynamic_block_role(block) != "ignore")
                    .unwrap_or(false)
        })
        .map(|(_, block)| block.clone())
        .collect::<Vec<_>>();
    let question_end = match (first_question_index, first_answer_index) {
        (Some(first_question), Some(first_answer)) if first_answer > first_question => {
            Some(first_answer)
        }
        _ => None,
    };
    let question_blocks = if let Some(first_question) = first_question_index {
        blocks[first_question..question_end.unwrap_or(blocks.len())]
            .iter()
            .filter(|block| {
                dynamic_block_role(block) != "answer" && dynamic_block_role(block) != "ignore"
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        blocks
            .iter()
            .filter(|block| is_dynamic_question_block(block))
            .cloned()
            .collect::<Vec<_>>()
    };
    let answer_blocks = blocks
        .iter()
        .filter(|block| is_dynamic_answer_block(block))
        .cloned()
        .collect::<Vec<_>>();

    let mut answer_map = serde_json::Map::new();
    for block in &answer_blocks {
        for (key, value) in parse_dynamic_answer_text(&dynamic_block_text(block)) {
            answer_map.insert(key, value);
        }
    }
    let mut answer_numbers = answer_map
        .keys()
        .filter_map(|key| key.parse::<u32>().ok())
        .collect::<Vec<_>>();
    answer_numbers.sort_unstable();

    let mut group_candidates = Vec::new();
    for (index, block) in question_blocks.iter().enumerate() {
        let text = dynamic_block_text(block);
        let Some((start, end)) = detect_dynamic_question_range(&text) else {
            continue;
        };
        let next_heading = question_blocks
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, candidate)| {
                detect_dynamic_question_range(&dynamic_block_text(candidate)).is_some()
            })
            .map(|(candidate_index, _)| candidate_index)
            .unwrap_or(question_blocks.len());
        let included = &question_blocks[index..next_heading];
        let combined = included
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join(" ");
        group_candidates.push(json!({
            "groupId": format!("group-{}", group_candidates.len() + 1),
            "heading": dynamic_question_heading(start, end),
            "questionRange": [start, end],
            "instructionText": text,
            "blockIds": included.iter().map(dynamic_block_id).collect::<Vec<_>>(),
            "kindHint": detect_dynamic_group_kind(&combined),
            "confidence": 0.72
        }));
    }

    if group_candidates.is_empty() && !question_blocks.is_empty() {
        let start = answer_numbers.first().copied().unwrap_or(1);
        let end = answer_numbers.last().copied().unwrap_or(start);
        let combined = question_blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");
        group_candidates.push(json!({
            "groupId": "group-1",
            "heading": dynamic_question_heading(start, end),
            "questionRange": [start, end],
            "instructionText": combined,
            "blockIds": question_blocks.iter().map(dynamic_block_id).collect::<Vec<_>>(),
            "kindHint": detect_dynamic_group_kind(&question_blocks.iter().map(dynamic_block_text).collect::<Vec<_>>().join(" ")),
            "confidence": 0.58
        }));
    }

    let fallback_passage_range = if let Some(first_question) = first_question_index {
        blocks[..first_question]
            .iter()
            .map(dynamic_block_id)
            .collect::<Vec<_>>()
    } else {
        blocks
            .iter()
            .take(3)
            .map(dynamic_block_id)
            .collect::<Vec<_>>()
    };
    let passage_range = if passage_blocks.is_empty() {
        fallback_passage_range
    } else {
        passage_blocks
            .iter()
            .map(dynamic_block_id)
            .collect::<Vec<_>>()
    };
    let mut issues = Vec::new();
    if group_candidates.is_empty() {
        issues.push("No question range heading detected; manual split required.");
    }
    if answer_map.is_empty() {
        issues.push("No answer key detected; answers must be entered manually.");
    }
    if let (Some(first_answer), Some(first_question)) = (first_answer_index, first_question_index) {
        if first_answer < first_question {
            issues.push("Answer block appears before question block; verify split order.");
        }
    }

    json!({
        "jobId": job_id,
        "passageCandidates": [{
            "range": passage_range,
            "title": infer_dynamic_passage_title(job, &passage_blocks),
            "categoryHint": job.category.clone().unwrap_or_else(|| "P1".to_string())
        }],
        "questionGroupCandidates": group_candidates,
        "answerKeyCandidates": if answer_map.is_empty() {
            json!([])
        } else {
            json!([{"source": answer_blocks.iter().map(dynamic_block_id).collect::<Vec<_>>().join(","), "answers": answer_map}])
        },
        "issues": issues
    })
}

fn dynamic_interaction_for_kind(kind: &str) -> Value {
    match kind {
        "true_false_not_given" => {
            json!({"type": "radio", "options": ["TRUE", "FALSE", "NOT GIVEN"]})
        }
        "yes_no_not_given" => json!({"type": "radio", "options": ["YES", "NO", "NOT GIVEN"]}),
        "single_choice" => json!({"type": "radio", "options": ["A", "B", "C", "D"]}),
        "multi_choice" => json!({"type": "checkbox", "options": ["A", "B", "C", "D", "E", "F"]}),
        _ => json!({"type": "text", "placeholder": "answer"}),
    }
}

fn dynamic_template_for_kind(kind: &str) -> &'static str {
    match kind {
        "true_false_not_given" => "tfng_list",
        "yes_no_not_given" => "ynng_list",
        "single_choice" => "single_choice_list",
        "multi_choice" => "multi_choice_checkbox",
        "table_completion" => "table_completion",
        "summary_completion" => "summary_text_completion",
        "sentence_completion" => "inline_text_completion",
        _ => "short_answer_list",
    }
}

fn find_dynamic_number_marker(text: &str, number: u32, from: usize) -> Option<(usize, usize)> {
    let needle = number.to_string();
    let mut search = from.min(text.len());
    while let Some(relative) = text[search..].find(&needle) {
        let start = search + relative;
        let after_digits = start + needle.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .map(|ch| ch.is_whitespace() || matches!(ch, '(' | '['))
            .unwrap_or(true);
        if !before_ok {
            search = after_digits;
            continue;
        }
        if let Some(next) = text[after_digits..].chars().next() {
            if matches!(next, '-' | '\u{2013}' | '\u{2014}') {
                search = after_digits;
                continue;
            }
            if !(next.is_whitespace() || matches!(next, '.' | ')' | ':' | '、')) {
                search = after_digits;
                continue;
            }
        }
        let mut content_start = after_digits;
        if let Some(next) = text[content_start..].chars().next() {
            if matches!(next, '.' | ')' | ':' | '、') {
                content_start += next.len_utf8();
            }
        }
        while let Some(next) = text[content_start..].chars().next() {
            if next.is_whitespace() {
                content_start += next.len_utf8();
            } else {
                break;
            }
        }
        return Some((start, content_start));
    }
    None
}

fn find_dynamic_final_prompt_boundary(text: &str, from: usize) -> usize {
    let lower = text.to_lowercase();
    [" questions ", " answers", " answer key"]
        .iter()
        .filter_map(|marker| lower[from..].find(marker).map(|relative| from + relative))
        .min()
        .unwrap_or(text.len())
}

fn dynamic_prompt_for_question(
    group_text: &str,
    number: u32,
    fallback_heading: &str,
    range_end: u32,
) -> String {
    let normalized = collapse_whitespace(group_text);
    if let Some((_, content_start)) = find_dynamic_number_marker(&normalized, number, 0) {
        let boundary = if number < range_end {
            find_dynamic_number_marker(&normalized, number + 1, content_start)
                .map(|(next_start, _)| next_start)
                .unwrap_or(normalized.len())
        } else {
            find_dynamic_final_prompt_boundary(&normalized, content_start)
        };
        let prompt = normalized[content_start..boundary]
            .trim()
            .trim_end_matches([';', ','])
            .trim();
        if !prompt.is_empty() {
            return prompt.to_string();
        }
    }
    format!("{} item {}", fallback_heading, number)
}

fn dynamic_block_html(block: &Value) -> String {
    block
        .get("html")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            format!(
                "<p>{}</p>",
                html_escape(
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                )
            )
        })
}

fn dynamic_answer_map_from_split(split: &Value) -> serde_json::Map<String, Value> {
    let mut answers = serde_json::Map::new();
    for candidate in split
        .get("answerKeyCandidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(map) = candidate.get("answers").and_then(Value::as_object) {
            for (key, value) in map {
                answers.insert(key.clone(), value.clone());
            }
        }
    }
    answers
}

fn parse_answer_source_candidates(
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

fn merge_answer_source_candidates(split: &mut Value, answer_candidates: Vec<Value>) {
    if answer_candidates.is_empty() {
        return;
    }
    if let Some(obj) = split.as_object_mut() {
        let has_any_answers = answer_candidates.iter().any(|candidate| {
            candidate
                .get("answers")
                .and_then(Value::as_object)
                .map(|answers| !answers.is_empty())
                .unwrap_or(false)
        });
        let candidates = obj
            .entry("answerKeyCandidates".to_string())
            .or_insert_with(|| json!([]));
        if !candidates.is_array() {
            *candidates = json!([]);
        }
        if let Some(items) = candidates.as_array_mut() {
            for candidate in answer_candidates {
                items.push(candidate);
            }
        }
        if has_any_answers {
            let issues = obj
                .get("issues")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|issue| {
                    issue.as_str()
                        != Some("No answer key detected; answers must be entered manually.")
                })
                .collect::<Vec<_>>();
            obj.insert("issues".to_string(), Value::Array(issues));
        }
    }
}

fn parser_warnings(doc: Option<&Value>) -> Vec<String> {
    doc.and_then(|value| value.pointer("/parser/warnings"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect()
}

fn low_confidence_block_ids(doc: Option<&Value>, threshold: f64) -> Vec<String> {
    dynamic_document_blocks(doc)
        .iter()
        .filter(|block| {
            block
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(1.0)
                < threshold
        })
        .map(dynamic_block_id)
        .collect()
}

fn source_review_path(root: &Path, job_id: &str) -> PathBuf {
    job_dir(root, job_id).join("source-review.json")
}

fn source_review_fingerprint(doc: Option<&Value>) -> String {
    let payload = json!({
        "sourceFileId": doc.and_then(|value| value.pointer("/parser/sourceFileId")).cloned().unwrap_or(Value::Null),
        "sourceStoredName": doc.and_then(|value| value.pointer("/parser/sourceStoredName")).cloned().unwrap_or(Value::Null),
        "provider": doc.and_then(|value| value.pointer("/parser/provider")).cloned().unwrap_or(Value::Null),
        "mode": doc.and_then(|value| value.pointer("/parser/mode")).cloned().unwrap_or(Value::Null),
        "parserWarnings": parser_warnings(doc),
        "lowConfidenceBlocks": dynamic_document_blocks(doc)
            .iter()
            .filter(|block| block.get("confidence").and_then(Value::as_f64).unwrap_or(1.0) < 0.5)
            .map(|block| json!({
                "blockId": dynamic_block_id(block),
                "confidence": block.get("confidence").cloned().unwrap_or(Value::Null),
                "roleHint": block.get("roleHint").cloned().unwrap_or(Value::Null),
                "textHash": hash_bytes(dynamic_block_text(block).as_bytes())
            }))
            .collect::<Vec<_>>()
    });
    serde_json::to_vec(&payload)
        .map(|bytes| hash_bytes(&bytes))
        .unwrap_or_else(|_| hash_bytes(b"source-review-fingerprint-error"))
}

fn source_review_status(root: &Path, job_id: &str, doc: Option<&Value>) -> CommandResult<Value> {
    let parser_warnings = parser_warnings(doc);
    let low_confidence_blocks = low_confidence_block_ids(doc, 0.5);
    let required = !parser_warnings.is_empty() || !low_confidence_blocks.is_empty();
    let fingerprint = source_review_fingerprint(doc);
    let saved = read_json_opt(&source_review_path(root, job_id))?;
    let saved_fingerprint = saved
        .as_ref()
        .and_then(|value| value.get("fingerprint"))
        .and_then(Value::as_str);
    let saved_resolved = saved
        .as_ref()
        .and_then(|value| value.get("resolved"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stale = required && saved_resolved && saved_fingerprint != Some(fingerprint.as_str());
    let resolved = !required || (saved_resolved && !stale);
    Ok(json!({
        "schemaVersion": "SourceReviewV1",
        "jobId": job_id,
        "required": required,
        "resolved": resolved,
        "stale": stale,
        "fingerprint": fingerprint,
        "parserWarnings": parser_warnings,
        "lowConfidenceBlocks": low_confidence_blocks,
        "resolvedAt": saved.as_ref().and_then(|value| value.get("resolvedAt")).cloned().unwrap_or(Value::Null),
        "note": saved.as_ref().and_then(|value| value.get("note")).cloned().unwrap_or(Value::Null)
    }))
}

fn write_source_review_status(
    root: &Path,
    job_id: &str,
    doc: Option<&Value>,
    resolved: bool,
    note: Option<String>,
) -> CommandResult<Value> {
    let mut review = source_review_status(root, job_id, doc)?;
    if let Some(obj) = review.as_object_mut() {
        obj.insert(
            "resolved".to_string(),
            json!(
                resolved
                    || !obj
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
            ),
        );
        obj.insert("stale".to_string(), json!(false));
        obj.insert(
            "resolvedAt".to_string(),
            if resolved {
                json!(Utc::now().to_rfc3339())
            } else {
                Value::Null
            },
        );
        obj.insert(
            "note".to_string(),
            note.map(Value::String).unwrap_or(Value::Null),
        );
    }
    write_json(&source_review_path(root, job_id), &review)?;
    Ok(review)
}

fn source_review_issues(review: &Value) -> Vec<Value> {
    let required = review
        .get("required")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let resolved = review
        .get("resolved")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !required || resolved {
        return Vec::new();
    }

    let mut issues = Vec::new();
    for warning in review
        .get("parserWarnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        issues.push(json_issue(
            "AuthoringIR",
            "$.sourceReview.parserWarnings",
            &format!(
                "Parser warning must be manually resolved before publish: {}",
                warning
            ),
        ));
    }
    for block_id in review
        .get("lowConfidenceBlocks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        issues.push(json_issue(
            "AuthoringIR",
            &format!("$.sourceReview.lowConfidenceBlocks[{}]", block_id),
            "Low-confidence parsed block requires source review before publish",
        ));
    }
    if issues.is_empty() {
        issues.push(json_issue(
            "AuthoringIR",
            "$.sourceReview.resolved",
            "Source document review must be resolved before publish",
        ));
    }
    issues
}

fn answer_is_empty(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(text)) => text.trim().is_empty(),
        Some(Value::Array(items)) => {
            items.is_empty() || items.iter().all(|item| answer_is_empty(Some(item)))
        }
        Some(Value::Object(items)) => items.is_empty(),
        Some(_) => false,
    }
}

fn value_confidence(value: &Value) -> f64 {
    value
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

fn value_verified(value: &Value) -> bool {
    value
        .get("verified")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn refresh_authoring_review_state(ir: &mut Value) -> u32 {
    let mut needs_review = 0u32;
    let mut total_questions = 0u32;
    let mut verified_questions = 0u32;
    let mut empty_answers = 0u32;

    if let Some(groups) = ir.get_mut("groups").and_then(Value::as_array_mut) {
        for group in groups {
            let mut group_question_count = 0u32;
            let mut group_verified_questions = 0u32;
            if let Some(questions) = group.get_mut("questions").and_then(Value::as_array_mut) {
                for question in questions {
                    total_questions += 1;
                    group_question_count += 1;
                    if value_verified(question) {
                        verified_questions += 1;
                        group_verified_questions += 1;
                    }
                    if answer_is_empty(question.get("answer")) {
                        empty_answers += 1;
                        needs_review += 1;
                    }
                    if value_confidence(question) < 0.85 && !value_verified(question) {
                        needs_review += 1;
                    }
                }
            }
            let all_group_questions_verified =
                group_question_count > 0 && group_question_count == group_verified_questions;
            if let Some(obj) = group.as_object_mut() {
                obj.insert("verified".to_string(), json!(all_group_questions_verified));
            }
            if value_confidence(group) < 0.85 && !all_group_questions_verified {
                needs_review += 1;
            }
        }
    }

    if let Some(audit) = ir.get_mut("audit").and_then(Value::as_object_mut) {
        audit.insert(
            "humanVerified".to_string(),
            json!(
                total_questions > 0 && total_questions == verified_questions && empty_answers == 0
            ),
        );
        audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
    }

    needs_review
}

fn authoring_review_issues(ir: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    for group in ir
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let group_id = group
            .get("groupId")
            .and_then(Value::as_str)
            .unwrap_or("unknown-group");
        if value_confidence(group) < 0.85 && !value_verified(group) {
            issues.push(json_issue(
                "AuthoringIR",
                &format!("$.groups[{}].verified", group_id),
                "Low-confidence group requires human verification before publish",
            ));
        }
        for question in group
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let qid = question
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("unknown-question");
            if answer_is_empty(question.get("answer")) {
                issues.push(json_issue(
                    "AuthoringIR",
                    &format!("$.answerKey.{}", qid),
                    "Question answer is empty; fill or verify the answer before publish",
                ));
            }
            if value_confidence(question) < 0.85 && !value_verified(question) {
                issues.push(json_issue(
                    "AuthoringIR",
                    &format!("$.groups[{}].questions[{}].verified", group_id, qid),
                    "Low-confidence question requires human verification before publish",
                ));
            }
        }
    }
    issues
}

fn dynamic_range_from_candidate(candidate: &Value) -> (u32, u32) {
    let values = candidate
        .get("questionRange")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let start = values.first().and_then(Value::as_u64).unwrap_or(1) as u32;
    let end = values
        .get(1)
        .and_then(Value::as_u64)
        .unwrap_or(start as u64) as u32;
    (start, end)
}

fn make_dynamic_authoring_ir(job: &ImportJob, split: &Value, doc: Option<&Value>) -> Value {
    let exam_id = format!(
        "{}-{}-{}",
        job.category
            .clone()
            .unwrap_or_else(|| "P1".to_string())
            .to_lowercase(),
        job.frequency
            .clone()
            .unwrap_or_else(|| "medium".to_string()),
        &job.job_id[job.job_id.len().saturating_sub(8)..]
    );
    let blocks = dynamic_document_blocks(doc);
    let blocks_by_id = blocks
        .iter()
        .map(|block| (dynamic_block_id(block), block.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let answer_by_display = dynamic_answer_map_from_split(split);
    let first_passage = split
        .get("passageCandidates")
        .and_then(Value::as_array)
        .and_then(|items| items.first());
    let passage_source_ids = first_passage
        .and_then(|candidate| candidate.get("range"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let passage_html = passage_source_ids
        .iter()
        .filter_map(|block_id| blocks_by_id.get(block_id))
        .map(dynamic_block_html)
        .collect::<Vec<_>>()
        .join("\n");
    let passage_title = first_passage
        .and_then(|candidate| candidate.get("title"))
        .and_then(Value::as_str)
        .unwrap_or(&job.title)
        .to_string();

    let groups = split
        .get("questionGroupCandidates")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let kind = candidate.get("kindHint").and_then(Value::as_str).unwrap_or("short_answer");
            let heading = candidate.get("heading").and_then(Value::as_str).unwrap_or("Questions");
            let instruction_text = candidate.get("instructionText").and_then(Value::as_str).unwrap_or(heading);
            let block_ids = candidate
                .get("blockIds")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).map(ToString::to_string).collect::<Vec<_>>())
                .unwrap_or_default();
            let group_text = {
                let text = block_ids
                    .iter()
                    .filter_map(|block_id| blocks_by_id.get(block_id))
                    .map(dynamic_block_text)
                    .collect::<Vec<_>>()
                    .join(" ");
                if text.trim().is_empty() { instruction_text.to_string() } else { text }
            };
            let (start, end) = dynamic_range_from_candidate(candidate);
            let questions = (start..=end)
                .map(|number| {
                    let display = number.to_string();
                    let qid = format!("q{}", display);
                    json!({
                        "id": qid,
                        "displayNumber": display,
                        "prompt": dynamic_prompt_for_question(&group_text, number, heading, end),
                        "interaction": dynamic_interaction_for_kind(kind),
                        "answer": answer_by_display.get(&number.to_string()).cloned().unwrap_or_else(|| json!("")),
                        "sourceBlockIds": block_ids,
                        "confidence": candidate.get("confidence").and_then(Value::as_f64).unwrap_or(0.72),
                        "verified": false
                    })
                })
                .collect::<Vec<_>>();
            let layout = if kind == "table_completion" {
                json!({"template": dynamic_template_for_kind(kind), "tableHeaders": ["Question", "Prompt", "Answer"]})
            } else {
                json!({"template": dynamic_template_for_kind(kind)})
            };
            json!({
                "groupId": candidate.get("groupId").and_then(Value::as_str).map(ToString::to_string).unwrap_or_else(|| format!("group-{}", index + 1)),
                "kind": kind,
                "questionRange": [start, end],
                "instruction": [instruction_text],
                "questions": questions,
                "layout": layout,
                "sourceBlockIds": block_ids,
                "confidence": candidate.get("confidence").and_then(Value::as_f64).unwrap_or(0.72),
                "verified": false
            })
        })
        .collect::<Vec<_>>();

    let answer_key = {
        let mut map = serde_json::Map::new();
        for group in &groups {
            for question in group
                .get("questions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(qid) = question.get("id").and_then(Value::as_str) {
                    map.insert(
                        qid.to_string(),
                        question.get("answer").cloned().unwrap_or_else(|| json!("")),
                    );
                }
            }
        }
        Value::Object(map)
    };
    let question_order = groups
        .iter()
        .flat_map(|group| {
            group
                .get("questions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|question| {
            question
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    let mut display_map = serde_json::Map::new();
    for group in &groups {
        for question in group
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(qid), Some(display)) = (
                question.get("id").and_then(Value::as_str),
                question.get("displayNumber").and_then(Value::as_str),
            ) {
                display_map.insert(qid.to_string(), json!(display));
            }
        }
    }
    let split_issues = split
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    json!({
        "schemaVersion": "ReadingAuthoringIRV1",
        "jobId": job.job_id,
        "exam": {
            "examId": exam_id,
            "title": job.title,
            "category": job.category.clone().unwrap_or_else(|| "P1".to_string()),
            "frequency": job.frequency.clone().unwrap_or_else(|| "medium".to_string()),
            "tags": job.tags,
            "sourceFiles": job.source_files.iter().map(|source| json!({
                "fileId": source.file_id,
                "originalName": source.original_name,
                "storedName": source.stored_name,
                "fileType": source.file_type,
                "sha256": source.sha256,
                "sizeBytes": source.size_bytes,
                "role": source.role,
                "importedAt": source.imported_at.to_rfc3339()
            })).collect::<Vec<_>>()
        },
        "passage": {
            "title": passage_title,
            "htmlBlocks": [{"blockId": "passage-main", "html": if passage_html.trim().is_empty() { format!("<h2>{}</h2>", html_escape(&job.title)) } else { passage_html }}],
            "sourceBlockIds": passage_source_ids
        },
        "groups": groups,
        "answerKey": answer_key,
        "questionOrder": question_order,
        "questionDisplayMap": Value::Object(display_map),
        "audit": {
            "llmUsed": false,
            "humanVerified": false,
            "issues": split_issues,
            "revision": 1,
            "updatedAt": Utc::now().to_rfc3339()
        }
    })
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#039;")
}

fn string_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn render_group_body_html(group: &Value) -> String {
    let group_id = string_at(group, "groupId");
    let kind = string_at(group, "kind");
    let lead = group
        .get("instruction")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|item| format!("<p>{}</p>", html_escape(item)))
                .collect::<String>()
        })
        .unwrap_or_default();
    let questions = group
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let body = if kind == "table_completion" {
        let rows = questions.iter().map(|q| {
            let qid = string_at(q, "id");
            format!("<tr><td><strong>{}</strong></td><td>{}</td><td><input type=\"text\" id=\"{}_input\" name=\"{}\" placeholder=\"answer\"></td></tr>", html_escape(string_at(q, "displayNumber")), html_escape(string_at(q, "prompt")), html_escape(qid), html_escape(qid))
        }).collect::<String>();
        format!("<table class=\"completion-table\"><thead><tr><th>Question</th><th>Prompt</th><th>Answer</th></tr></thead><tbody>{}</tbody></table>", rows)
    } else {
        let rows = questions.iter().map(|q| {
            let qid = string_at(q, "id");
            let options = q.pointer("/interaction/options").and_then(Value::as_array).cloned().unwrap_or_default();
            if kind == "multi_choice" {
                let controls = options.iter().filter_map(Value::as_str).map(|option| format!("<label><input name=\"{}\" type=\"checkbox\" value=\"{}\"> {}</label>", html_escape(qid), html_escape(option), html_escape(option))).collect::<String>();
                format!("<li><div><strong>{}</strong> {}</div><div class=\"choice-row\">{}</div></li>", html_escape(string_at(q, "displayNumber")), html_escape(string_at(q, "prompt")), controls)
            } else if !options.is_empty() {
                let controls = options.iter().filter_map(Value::as_str).map(|option| format!("<label><input name=\"{}\" type=\"radio\" value=\"{}\"> {}</label>", html_escape(qid), html_escape(option), html_escape(option))).collect::<String>();
                format!("<li><div><strong>{}</strong> {}</div><div class=\"choice-row\">{}</div></li>", html_escape(string_at(q, "displayNumber")), html_escape(string_at(q, "prompt")), controls)
            } else {
                format!("<li><label><strong>{}</strong> {} <input type=\"text\" id=\"{}_input\" name=\"{}\"></label></li>", html_escape(string_at(q, "displayNumber")), html_escape(string_at(q, "prompt")), html_escape(qid), html_escape(qid))
            }
        }).collect::<String>();
        format!("<ol>{}</ol>", rows)
    };
    format!("<section class=\"reading-question-group\" id=\"{}\"><div class=\"group-lead\">{}</div>{}</section>", html_escape(group_id), lead, body)
}

fn answer_key_from_authoring(authoring: &Value) -> Value {
    let mut map = serde_json::Map::new();
    if let Some(groups) = authoring.get("groups").and_then(Value::as_array) {
        for group in groups {
            if let Some(questions) = group.get("questions").and_then(Value::as_array) {
                for question in questions {
                    if let Some(qid) = question.get("id").and_then(Value::as_str) {
                        map.insert(
                            qid.to_string(),
                            question
                                .get("answer")
                                .cloned()
                                .unwrap_or(Value::String(String::new())),
                        );
                    }
                }
            }
        }
    }
    Value::Object(map)
}

fn question_order_from_authoring(authoring: &Value) -> Vec<String> {
    authoring
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| {
            group
                .get("questions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|question| {
            question
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect()
}

fn display_map_from_authoring(authoring: &Value) -> Value {
    let mut map = serde_json::Map::new();
    for group in authoring
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
            if let (Some(qid), Some(display)) = (
                question.get("id").and_then(Value::as_str),
                question.get("displayNumber").and_then(Value::as_str),
            ) {
                map.insert(qid.to_string(), Value::String(display.to_string()));
            }
        }
    }
    Value::Object(map)
}

fn authoring_source_file(authoring: &Value) -> Option<Value> {
    authoring
        .pointer("/exam/sourceFiles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|source| source.get("role").and_then(Value::as_str) == Some("MainQuestion"))
        .cloned()
}

fn reading_source(authoring: &Value) -> Value {
    let groups = authoring
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|group| {
            let question_ids = group
                .get("questions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|q| q.get("id").and_then(Value::as_str).map(|id| Value::String(id.to_string())))
                .collect::<Vec<_>>();
            json!({
                "groupId": group.get("groupId").cloned().unwrap_or(Value::String("group".to_string())),
                "kind": group.get("kind").cloned().unwrap_or(Value::String("short_answer".to_string())),
                "questionIds": question_ids,
                "bodyHtml": render_group_body_html(&group),
                "leadHtml": group.get("instruction").and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_str).map(|item| format!("<p>{}</p>", html_escape(item))).collect::<String>()).unwrap_or_default(),
                "allowOptionReuse": group.get("allowOptionReuse").cloned().unwrap_or(Value::Null)
            })
        })
        .collect::<Vec<_>>();

    let source_file = authoring_source_file(authoring);
    let pdf_filename = source_file
        .as_ref()
        .and_then(|source| source.get("originalName"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("source.pdf");
    let stored_name = source_file
        .as_ref()
        .and_then(|source| source.get("storedName"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("source.pdf");
    let source_file_id = source_file
        .as_ref()
        .and_then(|source| source.get("fileId"))
        .and_then(Value::as_str)
        .unwrap_or("unknown-source");
    let source_sha256 = source_file
        .as_ref()
        .and_then(|source| source.get("sha256"))
        .and_then(Value::as_str)
        .unwrap_or("unknown-sha256");
    let human_verified = authoring
        .pointer("/audit/humanVerified")
        .and_then(Value::as_bool)
        == Some(true);

    json!({
        "schemaVersion":"ReadingExamSourceV1",
        "examId": authoring.pointer("/exam/examId").and_then(Value::as_str).unwrap_or("local-authoring-exam"),
        "meta": {
            "title": authoring.pointer("/exam/title").and_then(Value::as_str).unwrap_or("Untitled Reading"),
            "category": authoring.pointer("/exam/category").and_then(Value::as_str).unwrap_or("P1"),
            "frequency": authoring.pointer("/exam/frequency").and_then(Value::as_str).unwrap_or("medium"),
            "pdfFilename": pdf_filename,
            "legacyPath": "",
            "legacyFilename": "",
            "questionIntroHtml": "<h3>Questions</h3>"
        },
        "passage": {"blocks": authoring.pointer("/passage/htmlBlocks").cloned().unwrap_or(json!([{"blockId":"passage-main","kind":"html","html":""}]))},
        "questionGroups": groups,
        "answerKey": answer_key_from_authoring(authoring),
        "sourceRefs": {"primaryHtml": format!("author-imports/{}/intermediate.html", authoring.get("jobId").and_then(Value::as_str).unwrap_or("job")), "primaryProvider":"author_web", "shuiHtml": null, "shuiPdf": format!("uploads/{}", stored_name), "ieltsHtml": null},
        "audit": {"matchStatus": if human_verified { "author_verified" } else { "needs_review" }, "matchConfidence": if human_verified { 1.0 } else { 0.0 }, "verifiedAt": if human_verified { json!(Utc::now().to_rfc3339()) } else { Value::Null }, "notes": format!("provider:author_tauri;sourceFileId:{};sourceSha256:{};signature:radio,text,table", source_file_id, source_sha256)},
        "questionOrder": question_order_from_authoring(authoring),
        "questionDisplayMap": display_map_from_authoring(authoring)
    })
}

fn build_wrapper(source: &Value) -> CommandResult<String> {
    let exam_id = source
        .get("examId")
        .and_then(Value::as_str)
        .unwrap_or("local-authoring-exam");
    let source_json = serde_json::to_string_pretty(source).map_err(|error| error.to_string())?;
    Ok(format!("(function registerReadingExamData(global) {{\n  'use strict';\n  if (!global.__READING_EXAM_DATA__ || typeof global.__READING_EXAM_DATA__.register !== \"function\") {{\n    throw new Error(\"reading_exam_registry_missing\");\n  }}\n  global.__READING_EXAM_DATA__.register(\"{}\", {});\n}})(typeof window !== \"undefined\" ? window : globalThis);\n", exam_id, source_json))
}

fn build_manifest(sources: &[Value]) -> CommandResult<String> {
    let mut manifest = serde_json::Map::new();
    for source in sources {
        let exam_id = source
            .get("examId")
            .and_then(Value::as_str)
            .unwrap_or("local-authoring-exam");
        manifest.insert(exam_id.to_string(), json!({
            "examId": exam_id,
            "dataKey": exam_id,
            "script": format!("./{}.js", exam_id),
            "title": source.pointer("/meta/title").and_then(Value::as_str).unwrap_or("Untitled Reading"),
            "category": source.pointer("/meta/category").and_then(Value::as_str).unwrap_or("P1")
        }));
    }
    Ok(format!(
        "window.__READING_EXAM_MANIFEST__ = {};\n",
        serde_json::to_string_pretty(&Value::Object(manifest)).map_err(|error| error.to_string())?
    ))
}

fn build_pack_manifest(input: &Value, sources: &[Value]) -> Value {
    let exams = sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let exam_id = source.get("examId").and_then(Value::as_str).unwrap_or("local-authoring-exam");
            json!({
                "order": index + 1,
                "examId": exam_id,
                "title": source.pointer("/meta/title").and_then(Value::as_str).unwrap_or("Untitled Reading"),
                "category": source.pointer("/meta/category").and_then(Value::as_str).unwrap_or("P1"),
                "frequency": source.pointer("/meta/frequency").and_then(Value::as_str).unwrap_or("medium"),
                "script": format!("reading-exams/{}.js", exam_id)
            })
        })
        .collect::<Vec<_>>();

    json!({
        "schemaVersion": "ReadingExamPackV1",
        "packId": input.get("packId").and_then(Value::as_str).unwrap_or("pack-local"),
        "version": input.get("version").and_then(Value::as_str).unwrap_or("0.1.0"),
        "institution": input.get("institution").and_then(Value::as_str).unwrap_or("internal"),
        "description": input.get("description").and_then(Value::as_str).unwrap_or(""),
        "validFrom": input.get("validFrom").cloned().unwrap_or(Value::Null),
        "validTo": input.get("validTo").cloned().unwrap_or(Value::Null),
        "generatedAt": Utc::now().to_rfc3339(),
        "assetsRoot": "reading-exams",
        "exams": exams
    })
}

fn qid_sort_key(qid: &str) -> Option<u32> {
    qid.strip_prefix('q')?.parse::<u32>().ok()
}

fn validate_authoring(job_id: &str, authoring: Option<&Value>) -> Value {
    let mut issues = Vec::new();
    if let Some(ir) = authoring {
        if ir
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .is_empty()
        {
            issues.push(json_issue(
                "AuthoringIR",
                "$.exam.examId",
                "examId is required",
            ));
        }
        if ir
            .get("groups")
            .and_then(Value::as_array)
            .map(|items| items.is_empty())
            .unwrap_or(true)
        {
            issues.push(json_issue(
                "AuthoringIR",
                "$.groups",
                "At least one question group is required",
            ));
        }
        let question_order = question_order_from_authoring(ir);
        let mut seen_qids = HashSet::new();
        let mut duplicate_qids = HashSet::new();
        for qid in &question_order {
            if !seen_qids.insert(qid.clone()) {
                duplicate_qids.insert(qid.clone());
            }
        }
        for qid in duplicate_qids {
            issues.push(json_issue(
                "AuthoringIR",
                "$.questionOrder",
                &format!("Duplicate question id in questionOrder: {}", qid),
            ));
        }

        let mut numeric_order = question_order
            .iter()
            .filter_map(|qid| qid_sort_key(qid))
            .collect::<Vec<_>>();
        numeric_order.sort_unstable();
        numeric_order.dedup();
        if let (Some(first), Some(last)) = (
            numeric_order.first().copied(),
            numeric_order.last().copied(),
        ) {
            let expected_len = (last - first + 1) as usize;
            if expected_len != numeric_order.len() {
                issues.push(json_issue(
                    "ReadingExamSourceV1",
                    "$.questionOrder",
                    &format!(
                        "questionOrder must be numerically continuous from q{} to q{}",
                        first, last
                    ),
                ));
            }
        }

        let mut display_seen: HashMap<String, String> = HashMap::new();
        for question in ir
            .get("groups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|group| {
                group
                    .get("questions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
        {
            if let Some(qid) = question.get("id").and_then(Value::as_str) {
                let display = question
                    .get("displayNumber")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if display.is_empty() {
                    issues.push(json_issue(
                        "ReadingExamSourceV1",
                        &format!("$.questionDisplayMap.{}", qid),
                        "questionDisplayMap display number cannot be empty",
                    ));
                } else if let Some(existing_qid) =
                    display_seen.insert(display.clone(), qid.to_string())
                {
                    issues.push(json_issue(
                        "ReadingExamSourceV1",
                        "$.questionDisplayMap",
                        &format!(
                            "Duplicate display number {} for {} and {}",
                            display, existing_qid, qid
                        ),
                    ));
                }
            }
        }

        for qid in question_order {
            let source = reading_source(ir);
            let found = source
                .get("questionGroups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|group| {
                    group
                        .get("bodyHtml")
                        .and_then(Value::as_str)
                        .map(|html| {
                            html.contains(&format!("name=\"{}\"", qid))
                                || html.contains(&format!("data-question=\"{}\"", qid))
                        })
                        .unwrap_or(false)
                });
            if !found {
                issues.push(json_issue(
                    "DomProtocol",
                    "$.questionGroups[].bodyHtml",
                    &format!("No collectible control found for {}", qid),
                ));
            }
        }
    } else {
        issues.push(json_issue("AuthoringIR", "$", "Authoring IR is missing"));
    }

    let layers = [
        "AuthoringIR",
        "ReadingExamSourceV1",
        "DomProtocol",
        "RuntimePreview",
    ]
    .iter()
    .map(|layer| {
        let count = issues
            .iter()
            .filter(|issue| issue.get("layer").and_then(Value::as_str) == Some(*layer))
            .count();
        json!({"layer": layer, "passed": count == 0, "issueCount": count})
    })
    .collect::<Vec<_>>();

    json!({"jobId": job_id, "passed": issues.is_empty(), "layers": layers, "issues": issues, "generatedAt": Utc::now().to_rfc3339()})
}

fn json_issue(layer: &str, path: &str, message: &str) -> Value {
    json!({"issueId": format!("issue-{}", Uuid::new_v4().simple()), "severity":"error", "layer":layer, "path":path, "message":message, "fixHint": null})
}

fn validate_with_node_sidecar(root: &Path, job_id: &str, source: &Value) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/node-validator/validate-reading-source.mjs")
        .ok_or_else(|| "node_validator_sidecar_missing".to_string())?;
    let input_path = job_dir(root, job_id)
        .join("cache")
        .join("reading-source-for-validation.json");
    write_json(&input_path, source)?;
    let output = Command::new("node")
        .arg(&script)
        .arg(&input_path)
        .output()
        .map_err(|error| format!("node_validator_spawn_failed:{}:{}", script.display(), error))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = serde_json::from_str::<Value>(&stdout)
        .map_err(|error| format!("node_validator_json_failed:{}:{}", error, stdout.trim()))?;
    if !output.status.success() && parsed.get("passed").and_then(Value::as_bool) != Some(false) {
        return Err(command_failure("node-validator", &output));
    }
    Ok(parsed)
}

fn validate_preview_with_node_sidecar(
    root: &Path,
    job_id: &str,
    preview_dir: &Path,
    exam_id: &str,
    unified_html_path: Option<&Path>,
    unified_python_path: Option<&Path>,
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/preview-e2e/preview-e2e.mjs")
        .ok_or_else(|| "preview_e2e_sidecar_missing".to_string())?;
    let mut command = Command::new("node");
    command
        .arg(&script)
        .arg("--preview-dir")
        .arg(preview_dir)
        .arg("--exam-id")
        .arg(exam_id)
        .arg("--job-id")
        .arg(job_id);
    if let Some(path) = unified_html_path {
        command.env("EPIC8_UNIFIED_HTML_PATH", path);
    }
    if let Some(path) = unified_python_path {
        command.env("EPIC8_UNIFIED_PYTHON", path);
    }
    let output = command
        .output()
        .map_err(|error| format!("preview_e2e_spawn_failed:{}:{}", script.display(), error))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = serde_json::from_str::<Value>(&stdout)
        .map_err(|error| format!("preview_e2e_json_failed:{}:{}", error, stdout.trim()))?;
    if !output.status.success() && parsed.get("passed").and_then(Value::as_bool) != Some(false) {
        return Err(command_failure("preview-e2e", &output));
    }
    let output_path = job_dir(root, job_id)
        .join("preview")
        .join("preview-e2e-report.json");
    write_json(&output_path, &parsed)?;
    Ok(parsed)
}

fn merge_sidecar_validation(base: &mut Value, sidecar: Value) {
    let Some(base_obj) = base.as_object_mut() else {
        return;
    };
    let sidecar_issues = sidecar
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let sidecar_layers = sidecar
        .get("layers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut layers_to_replace = sidecar_layers
        .iter()
        .filter_map(|layer| {
            layer
                .get("layer")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    if layers_to_replace.is_empty() {
        layers_to_replace.extend(["ReadingExamSourceV1".to_string(), "DomProtocol".to_string()]);
    }
    let mut merged_issues = base_obj
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|issue| {
            issue
                .get("layer")
                .and_then(Value::as_str)
                .map(|layer| !layers_to_replace.iter().any(|item| item == layer))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    merged_issues.extend(sidecar_issues);

    let layers = [
        "AuthoringIR",
        "ReadingExamSourceV1",
        "DomProtocol",
        "RuntimePreview",
    ]
    .iter()
    .map(|layer| {
        let count = merged_issues
            .iter()
            .filter(|issue| issue.get("layer").and_then(Value::as_str) == Some(*layer))
            .count();
        json!({"layer": layer, "passed": count == 0, "issueCount": count})
    })
    .collect::<Vec<_>>();
    base_obj.insert("passed".to_string(), json!(merged_issues.is_empty()));
    base_obj.insert("layers".to_string(), json!(layers));
    base_obj.insert("issues".to_string(), json!(merged_issues));
    if let Some(runtime) = sidecar.get("runtime") {
        base_obj.insert("runtime".to_string(), runtime.clone());
    }
}

fn preview_assets_for_source(
    root: &Path,
    job_id: &str,
    source: &Value,
) -> CommandResult<(String, PathBuf, String, String, Value)> {
    let exam_id = source
        .get("examId")
        .and_then(Value::as_str)
        .unwrap_or("local-authoring-exam")
        .to_string();
    let preview_dir = job_dir(root, job_id).join("preview");
    let wrapper_js = build_wrapper(source)?;
    let manifest_js = build_manifest(std::slice::from_ref(source))?;
    write_text(&preview_dir.join(format!("{}.js", exam_id)), &wrapper_js)?;
    write_text(&preview_dir.join("manifest.js"), &manifest_js)?;
    let assets = json!({"examId": exam_id, "manifestPath": preview_dir.join("manifest.js").to_string_lossy(), "scriptPath": preview_dir.join(format!("{}.js", exam_id)).to_string_lossy(), "previewUrl": format!("tauri-local://preview/{}", source.get("examId").and_then(Value::as_str).unwrap_or("local-authoring-exam")), "source": source, "wrapperJs": wrapper_js, "manifestJs": manifest_js});
    write_json(&preview_dir.join("preview-assets.json"), &assets)?;
    Ok((exam_id, preview_dir, wrapper_js, manifest_js, assets))
}

fn resolve_external_unified_html() -> Option<PathBuf> {
    if let Ok(value) = env::var("EPIC8_UNIFIED_HTML_PATH") {
        let path = PathBuf::from(value);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn resolve_external_unified_python() -> Option<PathBuf> {
    if let Ok(value) = env::var("EPIC8_UNIFIED_PYTHON") {
        let path = PathBuf::from(value);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn runtime_gate_strict_mode() -> bool {
    env::var("EPIC8_RUNTIME_GATE_STRICT")
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
        })
        .unwrap_or(true)
}

fn validate_for_runtime_gate(
    root: &Path,
    job_id: &str,
    ir: &Value,
    require_real_runtime: bool,
) -> CommandResult<Value> {
    let source = reading_source(ir);
    let mut report = validate_authoring(job_id, Some(ir));
    match validate_with_node_sidecar(root, job_id, &source) {
        Ok(sidecar_report) => merge_sidecar_validation(&mut report, sidecar_report),
        Err(error) => {
            merge_sidecar_validation(
                &mut report,
                json!({
                    "layers": [{"layer":"ReadingExamSourceV1"},{"layer":"DomProtocol"}],
                    "issues": [{
                        "issueId": format!("issue-{}", Uuid::new_v4().simple()),
                        "severity": "error",
                        "layer": "ReadingExamSourceV1",
                        "path": "$",
                        "message": format!("Node validator sidecar unavailable before runtime gate: {}", error),
                        "fixHint": "Runtime gate requires the ReadingExamSourceV1 and DOM sidecar validator."
                    }]
                }),
            );
        }
    }
    if report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let (exam_id, preview_dir, _, _, _) = preview_assets_for_source(root, job_id, &source)?;
        let unified_html_path = resolve_external_unified_html();
        let unified_python_path = resolve_external_unified_python();
        match validate_preview_with_node_sidecar(
            root,
            job_id,
            &preview_dir,
            &exam_id,
            unified_html_path.as_deref(),
            unified_python_path.as_deref(),
        ) {
            Ok(runtime_report) => merge_sidecar_validation(&mut report, runtime_report),
            Err(error) => {
                merge_sidecar_validation(
                    &mut report,
                    json!({
                        "layers": [{"layer":"RuntimePreview"}],
                        "issues": [{
                            "issueId": format!("issue-{}", Uuid::new_v4().simple()),
                            "severity": "error",
                            "layer": "RuntimePreview",
                            "path": "runtime.execution",
                            "message": format!("Preview E2E sidecar unavailable: {}", error),
                            "fixHint": "Verify node is installed and sidecars/preview-e2e/preview-e2e.mjs is bundled."
                        }]
                    }),
                );
            }
        }
        if require_real_runtime {
            let runtime = report.get("runtime").cloned().unwrap_or(Value::Null);
            let mode = runtime
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if mode != "real" {
                let reason = runtime
                    .get("fallbackReason")
                    .and_then(Value::as_str)
                    .unwrap_or("real runtime did not pass");
                merge_sidecar_validation(
                    &mut report,
                    json!({
                        "layers": [{"layer":"RuntimePreview"}],
                        "issues": [{
                            "issueId": format!("issue-{}", Uuid::new_v4().simple()),
                            "severity": "error",
                            "layer": "RuntimePreview",
                            "path": "runtime.mode",
                            "message": format!("Strict runtime gate requires real unified runtime mode; got '{}': {}", mode, reason),
                            "fixHint": "Set EPIC8_UNIFIED_HTML_PATH and EPIC8_UNIFIED_PYTHON to a valid unified runtime environment and re-run E2E."
                        }]
                    }),
                );
            }
        }
    }
    write_json(
        &job_dir(root, job_id).join("validation-report.json"),
        &report,
    )?;
    Ok(report)
}

fn merge_validation_issues(report: &mut Value, extra_issues: Vec<Value>) {
    if extra_issues.is_empty() {
        return;
    }
    let Some(obj) = report.as_object_mut() else {
        return;
    };
    let mut issues = obj
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    issues.extend(extra_issues);
    let layers = [
        "AuthoringIR",
        "ReadingExamSourceV1",
        "DomProtocol",
        "RuntimePreview",
    ]
    .iter()
    .map(|layer| {
        let count = issues
            .iter()
            .filter(|issue| issue.get("layer").and_then(Value::as_str) == Some(*layer))
            .count();
        json!({"layer": layer, "passed": count == 0, "issueCount": count})
    })
    .collect::<Vec<_>>();
    obj.insert("passed".to_string(), json!(issues.is_empty()));
    obj.insert("layers".to_string(), json!(layers));
    obj.insert("issues".to_string(), json!(issues));
}

fn publish_readiness_gate(
    root: &Path,
    job_id: &str,
    ir: &Value,
    mut runtime_report: Value,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let dir = job_dir(root, job_id);
    let document_ir = read_json_opt(&dir.join("document-ir.json"))?;
    let source_review = source_review_status(root, job_id, document_ir.as_ref())?;
    let human_verified = ir.pointer("/audit/humanVerified").and_then(Value::as_bool) == Some(true);
    let mut issues = Vec::new();

    if job.status == JobStatus::NeedsHumanReview {
        issues.push(json_issue(
            "AuthoringIR",
            "$.job.status",
            "Job is still marked NeedsHumanReview; complete manual review before publish",
        ));
    }
    issues.extend(source_review_issues(&source_review));
    if !human_verified {
        issues.push(json_issue(
            "AuthoringIR",
            "$.audit.humanVerified",
            "All questions and answers must be human verified before publish",
        ));
    }
    issues.extend(authoring_review_issues(ir));

    merge_validation_issues(&mut runtime_report, issues);
    write_json(&dir.join("publish-readiness-report.json"), &runtime_report)?;
    Ok(runtime_report)
}

fn apply_preview_e2e_job_state(
    root: &Path,
    job_id: &str,
    report: &Value,
    readiness_passed: bool,
) -> CommandResult<()> {
    let report_passed = report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let issues = report
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    update_job(root, job_id, |job| {
        job.status = if report_passed {
            if readiness_passed {
                JobStatus::ExportReady
            } else {
                JobStatus::PreviewReady
            }
        } else {
            JobStatus::ValidationFailed
        };
        job.current_step = if report_passed {
            if readiness_passed {
                WorkflowStep::Export
            } else {
                WorkflowStep::Preview
            }
        } else {
            WorkflowStep::Preview
        };
        job.issue_counts.errors = issues
            .iter()
            .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("error"))
            .count() as u32;
        job.issue_counts.warnings = issues
            .iter()
            .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("warning"))
            .count() as u32;
    })?;
    Ok(())
}

fn profiles_path(root: &Path) -> PathBuf {
    root.join("config").join("llm-profiles.json")
}

fn secret_path(root: &Path, profile_id: &str) -> PathBuf {
    root.join("config")
        .join("secrets")
        .join(format!("{}.key", profile_id))
}

fn keychain_service() -> &'static str {
    "com.ielts.author.studio.llm"
}

fn keychain_ref(profile_id: &str) -> String {
    format!("keychain:{}:{}", keychain_service(), profile_id)
}

fn file_secret_ref(profile_id: &str) -> String {
    format!("profile-secret:{}", profile_id)
}

#[cfg(target_os = "macos")]
fn keychain_save_secret(profile_id: &str, api_key: &str) -> CommandResult<()> {
    let output = Command::new("/usr/bin/security")
        .arg("add-generic-password")
        .arg("-a")
        .arg(profile_id)
        .arg("-s")
        .arg(keychain_service())
        .arg("-w")
        .arg(api_key)
        .arg("-U")
        .output()
        .map_err(|error| format!("keychain_save_spawn_failed:{}", error))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure("security add-generic-password", &output))
    }
}

#[cfg(not(target_os = "macos"))]
fn keychain_save_secret(_profile_id: &str, _api_key: &str) -> CommandResult<()> {
    Err("keychain_unavailable_on_this_platform".to_string())
}

#[cfg(target_os = "macos")]
fn keychain_load_secret(profile_id: &str) -> CommandResult<Option<String>> {
    let output = Command::new("/usr/bin/security")
        .arg("find-generic-password")
        .arg("-a")
        .arg(profile_id)
        .arg("-s")
        .arg(keychain_service())
        .arg("-w")
        .output()
        .map_err(|error| format!("keychain_load_spawn_failed:{}", error))?;
    if output.status.success() {
        let secret = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok((!secret.is_empty()).then_some(secret));
    }
    if output.status.code() == Some(44) {
        return Ok(None);
    }
    Err(command_failure("security find-generic-password", &output))
}

#[cfg(not(target_os = "macos"))]
fn keychain_load_secret(_profile_id: &str) -> CommandResult<Option<String>> {
    Ok(None)
}

#[cfg(target_os = "macos")]
fn keychain_delete_secret(profile_id: &str) -> CommandResult<()> {
    let output = Command::new("/usr/bin/security")
        .arg("delete-generic-password")
        .arg("-a")
        .arg(profile_id)
        .arg("-s")
        .arg(keychain_service())
        .output()
        .map_err(|error| format!("keychain_delete_spawn_failed:{}", error))?;
    if output.status.success() || output.status.code() == Some(44) {
        Ok(())
    } else {
        Err(command_failure("security delete-generic-password", &output))
    }
}

#[cfg(not(target_os = "macos"))]
fn keychain_delete_secret(_profile_id: &str) -> CommandResult<()> {
    Ok(())
}

fn file_save_secret(root: &Path, profile_id: &str, api_key: &str) -> CommandResult<()> {
    write_text(&secret_path(root, profile_id), api_key)
}

fn file_load_secret(root: &Path, profile_id: &str) -> Option<String> {
    fs::read_to_string(secret_path(root, profile_id))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn file_delete_secret(root: &Path, profile_id: &str) -> CommandResult<()> {
    let path = secret_path(root, profile_id);
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|error| format!("delete_secret_file:{}:{}", path.display(), error))?;
    }
    Ok(())
}

fn redact_profile_for_ui(root: &Path, mut profile: Value) -> Value {
    let Some(profile_id) = profile
        .get("profileId")
        .and_then(Value::as_str)
        .map(ToString::to_string)
    else {
        return profile;
    };
    let keychain_has_secret = matches!(keychain_load_secret(&profile_id), Ok(Some(_)));
    let file_has_secret = file_load_secret(root, &profile_id).is_some();
    let (backend, secret_ref, message) = if keychain_has_secret {
        (
            "keychain",
            keychain_ref(&profile_id),
            "API Key is stored in macOS Keychain.",
        )
    } else if file_has_secret {
        (
            "file",
            file_secret_ref(&profile_id),
            "API Key is stored in app data file fallback.",
        )
    } else {
        ("none", String::new(), "No API key is stored.")
    };
    if let Some(obj) = profile.as_object_mut() {
        obj.remove("apiKey");
        obj.insert("hasApiKey".to_string(), json!(backend != "none"));
        if backend == "none" {
            obj.remove("apiKeySecretRef");
        } else {
            obj.insert("apiKeySecretRef".to_string(), json!(secret_ref));
        }
        obj.insert("secretStorageBackend".to_string(), json!(backend));
        obj.insert("secretStorageMessage".to_string(), json!(message));
    }
    profile
}

fn save_profile_secret(
    root: &Path,
    profile_id: &str,
    api_key: Option<&str>,
) -> CommandResult<(bool, String, String)> {
    let Some(api_key) = api_key.map(str::trim).filter(|value| !value.is_empty()) else {
        let _ = keychain_delete_secret(profile_id);
        file_delete_secret(root, profile_id)?;
        return Ok((
            false,
            "none".to_string(),
            "No API key is stored.".to_string(),
        ));
    };
    match keychain_save_secret(profile_id, api_key) {
        Ok(()) => {
            let _ = file_delete_secret(root, profile_id);
            Ok((
                true,
                "keychain".to_string(),
                "API Key saved to macOS Keychain.".to_string(),
            ))
        }
        Err(error) => {
            file_save_secret(root, profile_id, api_key)?;
            Ok((
                true,
                "file".to_string(),
                format!(
                    "Keychain unavailable; API Key saved to app data file fallback: {}",
                    error
                ),
            ))
        }
    }
}

fn load_profile_secret(root: &Path, profile_id: &str) -> Option<String> {
    match keychain_load_secret(profile_id) {
        Ok(Some(secret)) => Some(secret),
        _ => file_load_secret(root, profile_id),
    }
}

fn load_profiles(root: &Path) -> CommandResult<Vec<Value>> {
    let path = profiles_path(root);
    if !path.exists() {
        return Ok(vec![redact_profile_for_ui(
            root,
            json!({"profileId":"profile-local-placeholder","name":"Local JSON Gateway","provider":"OpenAiCompatible","baseUrl":"http://localhost:11434/v1","model":"local-structurer","temperature":0,"timeoutMs":60000,"forceJson":true,"enabled":true}),
        )]);
    }
    Ok(read_json::<Vec<Value>>(&path)?
        .into_iter()
        .map(|profile| redact_profile_for_ui(root, profile))
        .collect())
}

fn save_profiles(root: &Path, profiles: &[Value]) -> CommandResult<()> {
    write_json(&profiles_path(root), profiles)
}

fn find_profile(root: &Path, profile_id: &str) -> CommandResult<Value> {
    load_profiles(root)?
        .into_iter()
        .find(|profile| profile.get("profileId").and_then(Value::as_str) == Some(profile_id))
        .ok_or_else(|| format!("profile_not_found:{}", profile_id))
}

fn llm_group_context(ir: &Value, group_id: &str) -> CommandResult<Value> {
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

fn run_llm_gateway(
    root: &Path,
    job_id: &str,
    command_name: &str,
    input: &Value,
    api_key: Option<&str>,
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/llm-gateway/gateway.mjs")
        .ok_or_else(|| "llm_gateway_sidecar_missing".to_string())?;
    let cache_dir = job_dir(root, job_id).join("cache").join("llm");
    let stamp = Utc::now().timestamp_millis();
    let input_path = cache_dir.join(format!("{}-input-{}.json", command_name, stamp));
    let output_path = cache_dir.join(format!("{}-output-{}.json", command_name, stamp));
    write_json(&input_path, &redact_llm_input_for_cache(input))?;
    let mut command = Command::new("node");
    command
        .arg(&script)
        .arg(command_name)
        .arg(&input_path)
        .arg(&output_path);
    if let Some(secret) = api_key.filter(|value| !value.trim().is_empty()) {
        command.env("EPIC8_LLM_API_KEY", secret);
    }
    let output = command
        .output()
        .map_err(|error| format!("llm_gateway_spawn_failed:{}:{}", script.display(), error))?;
    if !output.status.success() {
        return Err(command_failure("llm-gateway", &output));
    }
    read_json(&output_path)
}

fn redact_llm_input_for_cache(input: &Value) -> Value {
    let mut redacted = input.clone();
    if let Some(obj) = redacted.as_object_mut() {
        obj.remove("apiKey");
        obj.insert("apiKeySource".to_string(), json!("process-env"));
    }
    redacted
}

fn deterministic_llm_output(group: &Value, mode: &str, warning: String) -> Value {
    let kind = group
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("short_answer");
    json!({
        "kind": kind,
        "confidence": 0.64,
        "patch": [
            {"op":"replace","path":"/kind","value":kind}
        ],
        "questions": group.get("questions").cloned().unwrap_or_else(|| json!([])),
        "warnings": [warning, "low-confidence-review-required", "fallback-output-never-auto-applies"],
        "evidence": {"mode": mode, "source": "rust-local-fallback", "fallback": true}
    })
}

fn make_llm_input(
    profile: &Value,
    job: &ImportJob,
    group: &Value,
    profile_id: &str,
    mode: &str,
) -> Value {
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
        "group": group
    })
}

fn profile_payload(profile: &Value, profile_id: &str) -> Value {
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

fn select_llm_profile(
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

fn make_vision_transcription_input(
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

fn main_pdf_needs_vision_transcription(job: &ImportJob, doc: &Value) -> bool {
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
    provider != "vision-llm-transcription"
        && (warnings.contains("no extractable text")
            || warnings.contains("ocr/manual review required")
            || !low_confidence_block_ids(Some(doc), 0.5).is_empty())
}

fn vision_transcription_for_job(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    note: Option<&str>,
) -> CommandResult<(Value, Value)> {
    let source = main_source_file(job).ok_or_else(|| "no_main_source_file".to_string())?;
    if source.file_type != "pdf" {
        return Err(format!(
            "vision_transcription_requires_pdf:{}",
            source.file_type
        ));
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
    let profile = find_profile(root, profile_id)?;
    let dir = job_dir(root, &job.job_id);
    let cache_dir = dir.join("cache").join("vision");
    let extraction_path = cache_dir.join("pdf-images.json");
    let asset_dir = cache_dir.join("assets");
    let extraction = extract_pdf_images_with_python_sidecar(
        &job.job_id,
        &upload_path,
        &extraction_path,
        &asset_dir,
    )?;
    let image_count = extraction
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|page| {
            page.get("images")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .count();
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

fn load_llm_api_key(root: &Path, profile_id: &str) -> Option<String> {
    load_profile_secret(root, profile_id).filter(|value| !value.trim().is_empty())
}

fn save_llm_suggestion(root: &Path, job_id: &str, suggestion: &Value) -> CommandResult<()> {
    let dir = job_dir(root, job_id);
    let group_id = suggestion
        .get("groupId")
        .and_then(Value::as_str)
        .map(safe_json_filename)
        .unwrap_or_else(|| "unknown-group".to_string());
    let suggestion_id = suggestion
        .get("suggestionId")
        .and_then(Value::as_str)
        .map(safe_json_filename)
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

fn load_llm_suggestions(root: &Path, job_id: &str) -> CommandResult<Vec<Value>> {
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
        "radio" | "checkbox" | "text" | "textarea" | "select" | "dragdrop" | "table" | "diagram"
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

fn llm_suggestion_auto_apply_issues(
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

fn apply_suggestion_to_authoring(
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
        .collect::<std::collections::HashSet<_>>();
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

#[tauri::command]
async fn create_import_job(input: CreateJobInput, app: AppHandle) -> CommandResult<ImportJob> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    let job = make_job(input);
    let dir = job_dir(&root, &job.job_id);
    ensure_job_dirs(&dir)?;
    save_job(&root, &job)?;
    Ok(job)
}

#[tauri::command]
async fn list_jobs(filter: Option<JobFilter>, app: AppHandle) -> CommandResult<Vec<ImportJob>> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    let jobs_root = root.join("jobs");
    let mut jobs = Vec::new();
    for entry in fs::read_dir(jobs_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path().join("job.json");
        if path.exists() {
            jobs.push(read_json::<ImportJob>(&path)?);
        }
    }
    let filter = filter.unwrap_or_default();
    if let Some(status) = filter.status {
        jobs.retain(|job| job.status == status);
    }
    if let Some(search) = filter.search.filter(|value| !value.trim().is_empty()) {
        let search = search.to_lowercase();
        jobs.retain(|job| {
            job.title.to_lowercase().contains(&search)
                || job.job_id.to_lowercase().contains(&search)
        });
    }
    jobs.sort_by_key(|job| std::cmp::Reverse(job.updated_at));
    Ok(jobs)
}

#[tauri::command]
async fn get_job(job_id: String, app: AppHandle) -> CommandResult<JobDetail> {
    let root = app_root(&app)?;
    let dir = job_dir(&root, &job_id);
    Ok(JobDetail {
        job: load_job(&root, &job_id)?,
        document_ir: read_json_opt(&dir.join("document-ir.json"))?,
        source_review: {
            let document_ir = read_json_opt(&dir.join("document-ir.json"))?;
            Some(source_review_status(&root, &job_id, document_ir.as_ref())?)
        },
        split_candidates: read_json_opt(&dir.join("split-candidates.json"))?,
        authoring_ir: read_json_opt(&dir.join("authoring-ir.json"))?,
        validation_report: read_json_opt(&dir.join("validation-report.json"))?,
        preview_assets: read_json_opt(&dir.join("preview").join("preview-assets.json"))?,
        pipeline_report: read_json_opt(&dir.join("pipeline-report.json"))?,
        llm_suggestions: load_llm_suggestions(&root, &job_id)?,
    })
}

#[tauri::command]
async fn update_job_meta(
    job_id: String,
    patch: JobMetaPatch,
    app: AppHandle,
) -> CommandResult<ImportJob> {
    let root = app_root(&app)?;
    update_job(&root, &job_id, |job| {
        if let Some(title) = patch.title {
            job.title = title;
        }
        if let Some(category) = patch.category {
            job.category = Some(category);
        }
        if let Some(frequency) = patch.frequency {
            job.frequency = Some(frequency);
        }
        if let Some(tags) = patch.tags {
            job.tags = tags;
        }
        if let Some(profile_id) = patch.active_llm_profile_id {
            job.active_llm_profile_id = Some(profile_id);
        }
    })
}

#[tauri::command]
async fn delete_job(job_id: String, app: AppHandle) -> CommandResult<()> {
    let root = app_root(&app)?;
    let dir = job_dir(&root, &job_id);
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn import_source_file(
    job_id: String,
    file_path: String,
    role: String,
    app: AppHandle,
) -> CommandResult<SourceFile> {
    let root = app_root(&app)?;
    let dir = job_dir(&root, &job_id);
    ensure_job_dirs(&dir)?;
    let input = PathBuf::from(&file_path);
    let original_name = input
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("source.pdf")
        .to_string();
    let (hash, size, bytes) = hash_file_or_path(&input)?;
    let stored_name = format!("{}-{}", &hash[..8], sanitize_filename(&original_name));
    let bytes = bytes.ok_or_else(|| format!("source_file_not_readable:{}", input.display()))?;
    fs::write(dir.join("uploads").join(&stored_name), bytes).map_err(|error| error.to_string())?;
    let source = SourceFile {
        file_id: format!("file-{}", Uuid::new_v4().simple()),
        original_name,
        stored_name,
        file_type: file_type_from_name(&file_path).to_string(),
        sha256: hash,
        size_bytes: size,
        role,
        imported_at: Utc::now(),
    };
    update_job(&root, &job_id, |job| {
        job.source_files.push(source.clone());
        job.status = JobStatus::Uploaded;
        job.current_step = WorkflowStep::DocumentReview;
    })?;
    Ok(source)
}

#[tauri::command]
async fn reveal_job_folder(job_id: String, app: AppHandle) -> CommandResult<()> {
    let root = app_root(&app)?;
    let dir = job_dir(&root, &job_id);
    if !dir.exists() {
        return Err("job_folder_missing".to_string());
    }
    tauri_plugin_opener::open_path(dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn choose_export_dir() -> CommandResult<Option<String>> {
    Ok(None)
}

#[tauri::command]
async fn parse_document(
    job_id: String,
    options: ParseOptions,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let job = load_job(&root, &job_id)?;
    let mode = options.mode.as_deref().unwrap_or("auto");
    let ir = if let Some(source) = main_source_file(&job) {
        let upload_path = job_dir(&root, &job_id)
            .join("uploads")
            .join(&source.stored_name);
        if matches!(source.file_type.as_str(), "txt" | "md" | "pdf" | "docx")
            && upload_path.exists()
        {
            let parser_output = root
                .join("cache")
                .join("parser")
                .join(format!("{}-document-ir.json", job_id));
            parse_source_document(&job, source, &upload_path, &parser_output, mode)?
        } else {
            missing_source_document_ir(
                &job,
                mode,
                &format!(
                    "main source file missing or unsupported: type={}, path={}",
                    source.file_type,
                    upload_path.display()
                ),
            )
        }
    } else {
        missing_source_document_ir(&job, mode, "no MainQuestion source file")
    };
    write_json(&job_dir(&root, &job_id).join("document-ir.json"), &ir)?;
    let _ = write_source_review_status(&root, &job_id, Some(&ir), false, None)?;
    update_job(&root, &job_id, |job| {
        let review = source_review_status(&root, &job_id, Some(&ir))
            .unwrap_or_else(|_| json!({"required": true, "resolved": false}));
        job.status = if review
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            JobStatus::NeedsHumanReview
        } else {
            JobStatus::Parsed
        };
        job.current_step = WorkflowStep::DocumentReview;
        job.issue_counts.needs_review = source_review_issues(&review).len() as u32;
    })?;
    Ok(ir)
}

#[tauri::command]
async fn rerun_ocr(
    job_id: String,
    _page_indices: Vec<u32>,
    app: AppHandle,
) -> CommandResult<Value> {
    parse_document(
        job_id,
        ParseOptions {
            mode: Some("ocr".to_string()),
        },
        app,
    )
    .await
}

#[tauri::command]
async fn apply_manual_transcription(
    job_id: String,
    input: ManualTranscriptionInput,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let job = load_job(&root, &job_id)?;
    let text = input.text.trim();
    if text.is_empty() {
        return Err("manual_transcription_text_required".to_string());
    }
    let dir = job_dir(&root, &job_id);
    ensure_job_dirs(&dir)?;
    write_text(&dir.join("manual-transcription.txt"), text)?;
    let ir = manual_transcription_document_ir(&job, text, input.note.as_deref());
    write_json(&dir.join("document-ir.json"), &ir)?;
    write_source_review_status(
        &root,
        &job_id,
        Some(&ir),
        true,
        Some(
            "manual transcription applied; operator must verify content before publish".to_string(),
        ),
    )?;
    update_job(&root, &job_id, |job| {
        job.status = JobStatus::Parsed;
        job.current_step = WorkflowStep::DocumentReview;
        job.issue_counts.needs_review = 0;
    })?;
    Ok(ir)
}

#[tauri::command]
async fn apply_vision_transcription(
    job_id: String,
    input: Option<VisionTranscriptionInput>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let options = input.unwrap_or_default();
    let job = load_job(&root, &job_id)?;
    let profile_id = select_llm_profile(&root, &job, options.profile_id)
        .ok_or_else(|| "no_enabled_llm_profile_available_for_vision_transcription".to_string())?;
    let dir = job_dir(&root, &job_id);
    ensure_job_dirs(&dir)?;
    let (ir, output) =
        vision_transcription_for_job(&root, &job, &profile_id, options.note.as_deref())?;
    write_text(
        &dir.join("vision-transcription.txt"),
        output
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )?;
    write_json(&dir.join("vision-transcription-output.json"), &output)?;
    write_json(&dir.join("document-ir.json"), &ir)?;
    let review = write_source_review_status(&root, &job_id, Some(&ir), false, None)?;
    update_job(&root, &job_id, |job| {
        job.status = JobStatus::NeedsHumanReview;
        job.current_step = WorkflowStep::DocumentReview;
        job.issue_counts.needs_review = source_review_issues(&review).len() as u32;
    })?;
    Ok(ir)
}

#[tauri::command]
async fn resolve_source_review(
    job_id: String,
    note: Option<String>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let dir = job_dir(&root, &job_id);
    let document_ir = read_json_opt(&dir.join("document-ir.json"))?;
    let review = write_source_review_status(&root, &job_id, document_ir.as_ref(), true, note)?;
    let authoring = read_json_opt(&dir.join("authoring-ir.json"))?;
    let authoring_review_count = authoring
        .as_ref()
        .map(|ir| authoring_review_issues(ir).len() as u32)
        .unwrap_or(0);
    update_job(&root, &job_id, |job| {
        job.status = if authoring_review_count > 0 {
            JobStatus::NeedsHumanReview
        } else if authoring.is_some() {
            JobStatus::AuthoringReady
        } else {
            JobStatus::Parsed
        };
        job.current_step = if authoring_review_count > 0 {
            WorkflowStep::Authoring
        } else {
            WorkflowStep::DocumentReview
        };
        job.issue_counts.needs_review = authoring_review_count;
    })?;
    Ok(review)
}

#[tauri::command]
async fn run_rule_split(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let job = load_job(&root, &job_id)?;
    let doc = read_json_opt(&job_dir(&root, &job_id).join("document-ir.json"))?;
    let mut split = make_dynamic_split_candidates(&job_id, &job, doc.as_ref());
    let answer_candidates = parse_answer_source_candidates(&root, &job, "auto")?;
    merge_answer_source_candidates(&mut split, answer_candidates);
    write_json(
        &job_dir(&root, &job_id).join("split-candidates.json"),
        &split,
    )?;
    update_job(&root, &job_id, |job| {
        job.status = JobStatus::SplitReady;
        job.current_step = WorkflowStep::Split;
    })?;
    Ok(split)
}

#[tauri::command]
async fn save_split_adjustments(
    job_id: String,
    patch: Value,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    write_json(
        &job_dir(&root, &job_id).join("split-candidates.json"),
        &patch,
    )?;
    Ok(patch)
}

#[tauri::command]
async fn build_authoring_ir(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let job = load_job(&root, &job_id)?;
    let dir = job_dir(&root, &job_id);
    let doc = read_json_opt(&dir.join("document-ir.json"))?;
    let split = match read_json_opt(&dir.join("split-candidates.json"))? {
        Some(value) => value,
        None => {
            let mut value = make_dynamic_split_candidates(&job_id, &job, doc.as_ref());
            let answer_candidates = parse_answer_source_candidates(&root, &job, "auto")?;
            merge_answer_source_candidates(&mut value, answer_candidates);
            value
        }
    };
    write_json(&dir.join("split-candidates.json"), &split)?;
    let mut ir = make_dynamic_authoring_ir(&job, &split, doc.as_ref());
    let needs_review = refresh_authoring_review_state(&mut ir);
    let source_review = source_review_status(&root, &job_id, doc.as_ref())?;
    let source_review_issue_count = source_review_issues(&source_review).len() as u32;
    write_json(&job_dir(&root, &job_id).join("authoring-ir.json"), &ir)?;
    update_job(&root, &job_id, |job| {
        job.status = if needs_review > 0 || source_review_issue_count > 0 {
            JobStatus::NeedsHumanReview
        } else {
            JobStatus::AuthoringReady
        };
        job.current_step = WorkflowStep::Authoring;
        job.issue_counts = IssueCounts {
            errors: 0,
            warnings: 1,
            needs_review: needs_review + source_review_issue_count,
        };
    })?;
    Ok(ir)
}

#[tauri::command]
async fn update_authoring_ir(job_id: String, patch: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let mut ir = patch.get("ir").cloned().unwrap_or(patch);
    let needs_review = refresh_authoring_review_state(&mut ir);
    let document_ir = read_json_opt(&job_dir(&root, &job_id).join("document-ir.json"))?;
    let source_review = source_review_status(&root, &job_id, document_ir.as_ref())?;
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
        if let Some(audit) = obj.get_mut("audit").and_then(Value::as_object_mut) {
            let revision = audit.get("revision").and_then(Value::as_u64).unwrap_or(0) + 1;
            audit.insert("revision".to_string(), json!(revision));
            audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
        }
    }
    write_json(&job_dir(&root, &job_id).join("authoring-ir.json"), &ir)?;
    update_job(&root, &job_id, |job| {
        job.status = if needs_review > 0 || source_review_issue_count > 0 {
            JobStatus::NeedsHumanReview
        } else {
            JobStatus::AuthoringReady
        };
        job.current_step = WorkflowStep::Authoring;
        job.issue_counts.needs_review = needs_review;
        job.issue_counts.needs_review += source_review_issue_count;
    })?;
    Ok(ir)
}

#[tauri::command]
async fn render_group_html(
    job_id: String,
    group_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let ir: Value = read_json(&job_dir(&root, &job_id).join("authoring-ir.json"))?;
    let group = ir
        .get("groups")
        .and_then(Value::as_array)
        .and_then(|groups| {
            groups.iter().find(|group| {
                group.get("groupId").and_then(Value::as_str) == Some(group_id.as_str())
            })
        })
        .ok_or_else(|| "group_not_found".to_string())?;
    Ok(json!({"groupId": group_id, "bodyHtml": render_group_body_html(group)}))
}

#[tauri::command]
async fn list_llm_profiles(app: AppHandle) -> CommandResult<Vec<Value>> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    load_profiles(&root)
}

#[tauri::command]
async fn save_llm_profile(input: SaveLlmProfileInput, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    let profile_id = input
        .profile_id
        .unwrap_or_else(|| format!("profile-{}", Uuid::new_v4().simple()));
    let (has_api_key, secret_backend, secret_message) =
        save_profile_secret(&root, &profile_id, input.api_key.as_deref())?;
    let api_key_secret_ref = if secret_backend == "keychain" {
        keychain_ref(&profile_id)
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
    let mut profiles = load_profiles(&root)?;
    let profile_id = profile
        .get("profileId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    profiles
        .retain(|item| item.get("profileId").and_then(Value::as_str) != Some(profile_id.as_str()));
    profiles.insert(0, profile.clone());
    save_profiles(&root, &profiles)?;
    Ok(profile)
}

#[tauri::command]
async fn test_llm_profile(profile_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let started = Utc::now();
    let profile = find_profile(&root, &profile_id)?;
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
    let api_key = load_llm_api_key(&root, &profile_id);
    let result = run_llm_gateway(
        &root,
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

#[tauri::command]
async fn llm_classify_group(
    job_id: String,
    group_id: String,
    profile_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let job = load_job(&root, &job_id)?;
    let profile = find_profile(&root, &profile_id)?;
    let ir: Value = read_json(&job_dir(&root, &job_id).join("authoring-ir.json"))?;
    let group = llm_group_context(&ir, &group_id)?;
    let input = make_llm_input(&profile, &job, &group, &profile_id, "classify_group");
    let api_key = load_llm_api_key(&root, &profile_id);
    let output = run_llm_gateway(&root, &job_id, "classify_group", &input, api_key.as_deref())
        .unwrap_or_else(|error| {
            deterministic_llm_output(
                &group,
                "classify_group",
                format!("llm gateway fallback: {}", error),
            )
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
    save_llm_suggestion(
        &root,
        suggestion
            .get("jobId")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        &suggestion,
    )?;
    update_job(
        &root,
        suggestion
            .get("jobId")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        |job| {
            job.status = if confidence < 0.85 {
                JobStatus::NeedsHumanReview
            } else {
                JobStatus::AuthoringReady
            };
            job.current_step = WorkflowStep::LlmReview;
        },
    )?;
    Ok(suggestion)
}

#[tauri::command]
async fn llm_extract_group(
    job_id: String,
    group_id: String,
    profile_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let job = load_job(&root, &job_id)?;
    let profile = find_profile(&root, &profile_id)?;
    let ir: Value = read_json(&job_dir(&root, &job_id).join("authoring-ir.json"))?;
    let group = llm_group_context(&ir, &group_id)?;
    let input = make_llm_input(&profile, &job, &group, &profile_id, "extract_group");
    let api_key = load_llm_api_key(&root, &profile_id);
    let output = run_llm_gateway(&root, &job_id, "extract_group", &input, api_key.as_deref())
        .unwrap_or_else(|error| {
            deterministic_llm_output(
                &group,
                "extract_group",
                format!("llm gateway fallback: {}", error),
            )
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
    save_llm_suggestion(
        &root,
        suggestion
            .get("jobId")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        &suggestion,
    )?;
    update_job(
        &root,
        suggestion
            .get("jobId")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        |job| {
            job.status = if confidence < 0.85 {
                JobStatus::NeedsHumanReview
            } else {
                JobStatus::AuthoringReady
            };
            job.current_step = WorkflowStep::LlmReview;
        },
    )?;
    Ok(suggestion)
}

#[tauri::command]
async fn apply_llm_suggestion(
    job_id: String,
    suggestion_id: String,
    selected_paths: Vec<String>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let mut ir: Value = read_json(&job_dir(&root, &job_id).join("authoring-ir.json"))?;
    let suggestion = load_llm_suggestions(&root, &job_id)?
        .into_iter()
        .find(|item| {
            item.get("suggestionId").and_then(Value::as_str) == Some(suggestion_id.as_str())
        })
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
    let document_ir = read_json_opt(&job_dir(&root, &job_id).join("document-ir.json"))?;
    let source_review = source_review_status(&root, &job_id, document_ir.as_ref())?;
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
    write_json(&job_dir(&root, &job_id).join("authoring-ir.json"), &ir)?;
    update_job(&root, &job_id, |job| {
        job.status = if needs_review > 0 || source_review_issue_count > 0 {
            JobStatus::NeedsHumanReview
        } else {
            JobStatus::AuthoringReady
        };
        job.current_step = WorkflowStep::Authoring;
        job.issue_counts.needs_review = needs_review + source_review_issue_count;
    })?;
    Ok(ir)
}

#[tauri::command]
async fn validate_authoring_ir(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let authoring = read_json_opt(&job_dir(&root, &job_id).join("authoring-ir.json"))?;
    let mut report = validate_authoring(&job_id, authoring.as_ref());
    if let Some(ir) = authoring.as_ref() {
        let source = reading_source(ir);
        match validate_with_node_sidecar(&root, &job_id, &source) {
            Ok(sidecar_report) => merge_sidecar_validation(&mut report, sidecar_report),
            Err(error) => {
                merge_validation_issues(
                    &mut report,
                    vec![json!({
                        "issueId": format!("issue-{}", Uuid::new_v4().simple()),
                        "severity": "warning",
                        "layer": "ReadingExamSourceV1",
                        "path": "$",
                        "message": format!("Node validator sidecar unavailable; used built-in validation only: {}", error),
                        "fixHint": "Verify node is installed and sidecars/node-validator/validate-reading-source.mjs is bundled."
                    })],
                );
            }
        }
    }
    let document_ir = read_json_opt(&job_dir(&root, &job_id).join("document-ir.json"))?;
    let source_review = source_review_status(&root, &job_id, document_ir.as_ref())?;
    let source_review_issue_count = source_review_issues(&source_review).len() as u32;
    write_json(
        &job_dir(&root, &job_id).join("validation-report.json"),
        &report,
    )?;
    update_job(&root, &job_id, |job| {
        job.status = if source_review_issue_count > 0 {
            JobStatus::NeedsHumanReview
        } else if report
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            JobStatus::PreviewReady
        } else {
            JobStatus::ValidationFailed
        };
        let issues = report
            .get("issues")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        job.issue_counts.errors = issues
            .iter()
            .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("error"))
            .count() as u32;
        job.issue_counts.warnings = issues
            .iter()
            .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("warning"))
            .count() as u32;
        job.issue_counts.needs_review = source_review_issue_count;
    })?;
    Ok(report)
}

#[tauri::command]
async fn generate_preview_assets(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let ir: Value = read_json(&job_dir(&root, &job_id).join("authoring-ir.json"))?;
    let report = validate_for_runtime_gate(&root, &job_id, &ir, false)?;
    if !report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        update_job(&root, &job_id, |job| {
            job.status = JobStatus::ValidationFailed;
            job.current_step = WorkflowStep::Authoring;
        })?;
        return Err(format!(
            "preview_validation_failed:{}",
            serde_json::to_string(&report).unwrap_or_default()
        ));
    }
    let source = reading_source(&ir);
    let (_, _, _, _, assets) = preview_assets_for_source(&root, &job_id, &source)?;
    let human_verified = ir.pointer("/audit/humanVerified").and_then(Value::as_bool) == Some(true);
    let mut review_issues = authoring_review_issues(&ir);
    let document_ir = read_json_opt(&job_dir(&root, &job_id).join("document-ir.json"))?;
    let source_review = source_review_status(&root, &job_id, document_ir.as_ref())?;
    review_issues.extend(source_review_issues(&source_review));
    update_job(&root, &job_id, |job| {
        job.status = if review_issues.is_empty() && human_verified {
            JobStatus::PreviewReady
        } else {
            JobStatus::NeedsHumanReview
        };
        job.current_step = WorkflowStep::Preview;
        job.issue_counts.needs_review = review_issues.len() as u32;
    })?;
    Ok(assets)
}

#[tauri::command]
async fn run_preview_e2e(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let authoring = read_json_opt(&job_dir(&root, &job_id).join("authoring-ir.json"))?;
    let report = if let Some(ir) = authoring.as_ref() {
        validate_for_runtime_gate(&root, &job_id, ir, false)?
    } else {
        let report = validate_authoring(&job_id, None);
        write_json(
            &job_dir(&root, &job_id).join("validation-report.json"),
            &report,
        )?;
        report
    };
    let report_passed = report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let real_runtime_passed = report
        .get("runtime")
        .and_then(|runtime| runtime.get("mode"))
        .and_then(Value::as_str)
        == Some("real");
    let readiness_passed = if report_passed && real_runtime_passed {
        if let Some(ir) = authoring.as_ref() {
            publish_readiness_gate(&root, &job_id, ir, report.clone())?
                .get("passed")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };
    apply_preview_e2e_job_state(&root, &job_id, &report, readiness_passed)?;
    Ok(report)
}

#[tauri::command]
async fn export_reading_assets(
    job_id: String,
    export_dir: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    export_reading_assets_core(&root, &job_id, &export_dir, runtime_gate_strict_mode())
}

fn export_reading_assets_core(
    root: &Path,
    job_id: &str,
    export_dir: &str,
    require_real_runtime: bool,
) -> CommandResult<Value> {
    let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let report = validate_for_runtime_gate(root, job_id, &ir, require_real_runtime)?;
    let report = publish_readiness_gate(root, job_id, &ir, report)?;
    if !report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(format!(
            "export_validation_failed:{}",
            serde_json::to_string(&report).unwrap_or_default()
        ));
    }
    let source = reading_source(&ir);
    let exam_id = source
        .get("examId")
        .and_then(Value::as_str)
        .unwrap_or("local-authoring-exam")
        .to_string();
    let wrapper_js = build_wrapper(&source)?;
    let manifest_js = build_manifest(std::slice::from_ref(&source))?;
    let out_dir = if export_dir.starts_with("local://") {
        job_dir(root, job_id).join("exports")
    } else {
        PathBuf::from(export_dir)
    };
    fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;
    write_json(&out_dir.join(format!("{}.json", exam_id)), &source)?;
    write_text(&out_dir.join(format!("{}.js", exam_id)), &wrapper_js)?;
    write_text(&out_dir.join("manifest.js"), &manifest_js)?;
    write_json(&out_dir.join("validation-report.json"), &report)?;
    update_job(root, job_id, |job| {
        job.status = JobStatus::ExportReady;
        job.current_step = WorkflowStep::Export;
    })?;
    Ok(
        json!({"examId": exam_id, "files":[{"name":format!("{}.json", exam_id),"content":serde_json::to_string_pretty(&source).unwrap_or_default()},{"name":format!("{}.js", exam_id),"content":wrapper_js},{"name":"manifest.js","content":manifest_js}], "outputDir": out_dir.to_string_lossy()}),
    )
}

#[tauri::command]
async fn build_pack(input: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    build_pack_core(&root, &input, runtime_gate_strict_mode())
}

fn build_pack_core(root: &Path, input: &Value, require_real_runtime: bool) -> CommandResult<Value> {
    let pack_id = input
        .get("packId")
        .and_then(Value::as_str)
        .unwrap_or("pack-local")
        .to_string();
    if input
        .get("jobIds")
        .and_then(Value::as_array)
        .map(|items| items.is_empty())
        .unwrap_or(true)
    {
        return Err("pack_requires_at_least_one_job".to_string());
    }
    let pack_dir = root.join("packs").join(&pack_id);
    let exams_dir = pack_dir.join("reading-exams");
    let mut sources = Vec::new();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut job_ids = Vec::new();
    for job_id in input
        .get("jobIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
        let source = reading_source(&ir);
        let report = validate_for_runtime_gate(root, job_id, &ir, require_real_runtime)?;
        let report = publish_readiness_gate(root, job_id, &ir, report)?;
        if !report
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(format!(
                "pack_validation_failed:{}:{}",
                job_id,
                serde_json::to_string(&report).unwrap_or_default()
            ));
        }
        let exam_id = source
            .get("examId")
            .and_then(Value::as_str)
            .unwrap_or(job_id)
            .to_string();
        let wrapper = build_wrapper(&source)?;
        let script_name = format!("reading-exams/{}.js", exam_id);
        entries.push((script_name, wrapper.into_bytes()));
        job_ids.push(job_id.to_string());
        sources.push(source);
    }
    let manifest_js = build_manifest(&sources)?;
    let pack_manifest = build_pack_manifest(input, &sources);
    let pack_json =
        serde_json::to_string_pretty(&pack_manifest).map_err(|error| error.to_string())?;
    entries.insert(
        0,
        (
            "reading-exams/manifest.js".to_string(),
            manifest_js.into_bytes(),
        ),
    );
    entries.insert(0, ("pack.json".to_string(), pack_json.into_bytes()));
    let zip_path = root.join("packs").join(format!("{}.zip", pack_id));
    let zip_size = write_zip(&zip_path, &entries)?;
    fs::create_dir_all(&exams_dir).map_err(|error| error.to_string())?;
    for (entry_path, content) in &entries {
        if entry_path == "pack.json" {
            write_bytes(&pack_dir.join("pack.json"), content)?;
        } else if let Some(file_name) = entry_path.strip_prefix("reading-exams/") {
            write_bytes(&exams_dir.join(file_name), content)?;
        }
    }
    for job_id in &job_ids {
        update_job(root, job_id, |job| {
            job.status = JobStatus::Published;
            job.current_step = WorkflowStep::Pack;
        })?;
    }
    Ok(json!({
        "packId": pack_id,
        "outputPath": zip_path.to_string_lossy(),
        "files": entries.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>(),
        "zipSizeBytes": zip_size,
        "entryCount": entries.len(),
        "manifest": pack_manifest,
        "createdAt": Utc::now().to_rfc3339()
    }))
}

#[tauri::command]
async fn run_auto_pipeline(
    job_id: String,
    input: Option<AutoPipelineInput>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let options = input.unwrap_or_default();
    let parse_mode = options.parse_mode.as_deref().unwrap_or("auto");
    let confidence_threshold = options.confidence_threshold.unwrap_or(0.85).clamp(0.0, 1.0);

    let mut job = load_job(&root, &job_id)?;
    let dir = job_dir(&root, &job_id);
    ensure_job_dirs(&dir)?;

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
        let _ = write_source_review_status(&root, &job_id, Some(&ir), false, None)?;
        job = update_job(&root, &job_id, |item| {
            let review = source_review_status(&root, &job_id, Some(&ir))
                .unwrap_or_else(|_| json!({"required": true, "resolved": false}));
            item.status = if review
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                JobStatus::NeedsHumanReview
            } else {
                JobStatus::Parsed
            };
            item.current_step = WorkflowStep::DocumentReview;
            item.issue_counts.needs_review = source_review_issues(&review).len() as u32;
        })?;
    }

    let profile_id = select_llm_profile(&root, &job, options.profile_id.clone());
    let mut vision_transcription = json!({
        "attempted": false,
        "applied": false,
        "profileId": profile_id,
        "warnings": [],
        "failure": null
    });

    let mut doc = read_json_opt(&dir.join("document-ir.json"))?;
    if let (Some(profile_id_for_vision), Some(current_doc)) = (profile_id.as_deref(), doc.as_ref())
    {
        if main_pdf_needs_vision_transcription(&job, current_doc) {
            if let Some(obj) = vision_transcription.as_object_mut() {
                obj.insert("attempted".to_string(), json!(true));
            }
            match vision_transcription_for_job(
                &root,
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
                        write_source_review_status(&root, &job_id, Some(&vision_ir), false, None)?;
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
                    job = update_job(&root, &job_id, |item| {
                        item.status = JobStatus::NeedsHumanReview;
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

    let source_review = source_review_status(&root, &job_id, doc.as_ref())?;
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
    let answer_candidates = parse_answer_source_candidates(&root, &job, parse_mode)?;
    merge_answer_source_candidates(&mut split, answer_candidates);
    write_json(&dir.join("split-candidates.json"), &split)?;
    job = update_job(&root, &job_id, |item| {
        item.status = JobStatus::SplitReady;
        item.current_step = WorkflowStep::Split;
    })?;

    let mut ir = make_dynamic_authoring_ir(&job, &split, doc.as_ref());
    write_json(&dir.join("authoring-ir.json"), &ir)?;
    job = update_job(&root, &job_id, |item| {
        item.status = JobStatus::AuthoringReady;
        item.current_step = WorkflowStep::Authoring;
    })?;

    let mut low_confidence_groups = Vec::<String>::new();
    let mut blocked_auto_apply_groups = Vec::<String>::new();
    let mut high_confidence_applied_groups = Vec::<String>::new();
    let mut llm_failures = Vec::<String>::new();
    let mut suggestion_count = 0u32;
    let mut applied_count = 0u32;

    if let Some(profile_id) = profile_id {
        let profile = find_profile(&root, &profile_id)?;
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
                let api_key = load_llm_api_key(&root, &profile_id);
                let llm_input =
                    make_llm_input(&profile, &job, &group, &profile_id, "extract_group");
                let output = run_llm_gateway(
                    &root,
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
                let _ = save_llm_suggestion(&root, &job_id, &suggestion);

                if confidence >= confidence_threshold {
                    let selected = vec![
                        "kind".to_string(),
                        "layout".to_string(),
                        "questions".to_string(),
                    ];
                    let auto_apply_issues =
                        llm_suggestion_auto_apply_issues(&ir, &suggestion, &selected);
                    if auto_apply_issues.is_empty()
                        && apply_suggestion_to_authoring(&mut ir, &suggestion, &selected).is_ok()
                    {
                        if let Some(groups) = ir.get_mut("groups").and_then(Value::as_array_mut) {
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
                        llm_failures.push(format!(
                            "{}:auto_apply_blocked:{}",
                            group_id,
                            auto_apply_issues.join(",")
                        ));
                    }
                } else {
                    low_confidence_groups.push(
                        suggestion
                            .get("groupId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    );
                }
            }
        }
    } else {
        llm_failures.push("no_enabled_llm_profile_available".to_string());
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
    write_json(&dir.join("authoring-ir.json"), &ir)?;

    let report = validate_for_runtime_gate(&root, &job_id, &ir, false)?;
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
    let real_runtime_passed = report_passed && runtime_mode == "real";

    let requires_parser_review = !source_review_issues(&source_review).is_empty();
    let requires_authoring_review = remaining_authoring_review > 0;

    let next_status = if !low_confidence_groups.is_empty()
        || !blocked_auto_apply_groups.is_empty()
        || requires_parser_review
        || requires_authoring_review
    {
        JobStatus::NeedsHumanReview
    } else if real_runtime_passed {
        JobStatus::ExportReady
    } else if report_passed {
        JobStatus::PreviewReady
    } else {
        JobStatus::ValidationFailed
    };
    let next_step = if !low_confidence_groups.is_empty() || !blocked_auto_apply_groups.is_empty() {
        WorkflowStep::LlmReview
    } else if requires_parser_review {
        WorkflowStep::DocumentReview
    } else if requires_authoring_review {
        WorkflowStep::Authoring
    } else if real_runtime_passed {
        WorkflowStep::Export
    } else {
        WorkflowStep::Preview
    };

    update_job(&root, &job_id, |item| {
        item.status = next_status.clone();
        item.current_step = next_step.clone();
        item.issue_counts.needs_review = low_confidence_groups.len() as u32
            + blocked_auto_apply_groups.len() as u32
            + source_review_issues(&source_review).len() as u32
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
        "realRuntimePassed": real_runtime_passed,
        "runtimeMode": runtime_mode,
        "parser": {
            "warnings": parser_warnings,
            "lowConfidenceBlocks": low_confidence_blocks,
            "visionTranscription": vision_transcription
        },
        "authoring": {
            "remainingReviewItems": remaining_authoring_review
        },
        "status": format!("{:?}", next_status),
        "currentStep": format!("{:?}", next_step),
        "generatedAt": Utc::now().to_rfc3339(),
        "validationReport": report
    });
    write_json(&dir.join("pipeline-report.json"), &pipeline_report)?;
    Ok(pipeline_report)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState)
        .setup(|app| {
            let root = app_root(app.handle()).map_err(Box::<dyn std::error::Error>::from)?;
            ensure_app_dirs(&root).map_err(Box::<dyn std::error::Error>::from)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_import_job,
            list_jobs,
            get_job,
            update_job_meta,
            delete_job,
            import_source_file,
            reveal_job_folder,
            choose_export_dir,
            parse_document,
            rerun_ocr,
            apply_manual_transcription,
            apply_vision_transcription,
            resolve_source_review,
            run_rule_split,
            save_split_adjustments,
            build_authoring_ir,
            update_authoring_ir,
            render_group_html,
            list_llm_profiles,
            save_llm_profile,
            test_llm_profile,
            llm_classify_group,
            llm_extract_group,
            apply_llm_suggestion,
            validate_authoring_ir,
            generate_preview_assets,
            run_preview_e2e,
            run_auto_pipeline,
            export_reading_assets,
            build_pack
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_job() -> ImportJob {
        make_job(CreateJobInput {
            title: Some("Audit Fixture".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["test".to_string()]),
            llm_profile_id: None,
        })
    }

    fn test_source(file_type: &str) -> SourceFile {
        SourceFile {
            file_id: "file-test".to_string(),
            original_name: format!("source.{}", file_type),
            stored_name: format!("stored.{}", file_type),
            file_type: file_type.to_string(),
            sha256: "0".repeat(64),
            size_bytes: 1,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        }
    }

    fn test_answer_source(file_type: &str) -> SourceFile {
        SourceFile {
            file_id: "file-answer".to_string(),
            original_name: format!("answers.{}", file_type),
            stored_name: format!("answers.{}", file_type),
            file_type: file_type.to_string(),
            sha256: "1".repeat(64),
            size_bytes: 1,
            role: "AnswerKey".to_string(),
            imported_at: Utc::now(),
        }
    }

    fn temp_test_root() -> PathBuf {
        env::temp_dir().join(format!("epic8-test-{}", Uuid::new_v4().simple()))
    }

    fn external_runtime_available() -> bool {
        resolve_external_unified_html().is_some() && resolve_external_unified_python().is_some()
    }

    fn parser_fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures")
            .join("parser")
            .join(name)
    }

    fn make_publishable_fixture(root: &Path) -> (ImportJob, Value) {
        ensure_app_dirs(root).unwrap();
        let mut job = test_job();
        job.source_files = vec![SourceFile {
            file_id: "file-publishable".to_string(),
            original_name: "publishable.pdf".to_string(),
            stored_name: "publishable.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: "b".repeat(64),
            size_bytes: 431,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        }];
        save_job(root, &job).unwrap();
        ensure_job_dirs(&job_dir(root, &job.job_id)).unwrap();

        let doc = sample_document_ir(&job, "auto");
        write_json(&job_dir(root, &job.job_id).join("document-ir.json"), &doc).unwrap();
        write_source_review_status(root, &job.job_id, Some(&doc), true, None).unwrap();
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        write_json(
            &job_dir(root, &job.job_id).join("split-candidates.json"),
            &split,
        )
        .unwrap();
        let mut ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        verify_all_authoring_items(&mut ir);
        write_json(&job_dir(root, &job.job_id).join("authoring-ir.json"), &ir).unwrap();
        (job, ir)
    }

    fn verify_all_authoring_items(ir: &mut Value) {
        if let Some(groups) = ir.get_mut("groups").and_then(Value::as_array_mut) {
            for group in groups {
                if let Some(obj) = group.as_object_mut() {
                    obj.insert("verified".to_string(), json!(true));
                }
                for question in group
                    .get_mut("questions")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                {
                    if let Some(obj) = question.as_object_mut() {
                        obj.insert("verified".to_string(), json!(true));
                    }
                }
            }
        }
        refresh_authoring_review_state(ir);
    }

    #[test]
    fn llm_cache_input_redacts_api_key() {
        let input = json!({
            "apiKey": "sk-secret-value",
            "profile": {"profileId": "profile-test", "model": "gpt-test"},
            "group": {"groupId": "group-test"}
        });

        let redacted = redact_llm_input_for_cache(&input);
        let serialized = serde_json::to_string(&redacted).unwrap();

        assert!(!serialized.contains("sk-secret-value"));
        assert!(redacted.get("apiKey").is_none());
        assert_eq!(
            redacted.get("apiKeySource").and_then(Value::as_str),
            Some("process-env")
        );
    }

    #[test]
    fn make_llm_input_never_contains_api_key() {
        let job = test_job();
        let profile = json!({
            "profileId": "profile-test",
            "provider": "OpenAiCompatible",
            "baseUrl": "https://example.invalid/v1",
            "model": "gpt-test",
            "apiKey": "sk-profile-secret"
        });
        let group = json!({"groupId": "group-test", "questions": []});

        let input = make_llm_input(&profile, &job, &group, "profile-test", "extract_group");
        let serialized = serde_json::to_string(&input).unwrap();

        assert!(!serialized.contains("sk-profile-secret"));
        assert!(input.get("apiKey").is_none());
        assert!(input.pointer("/profile/apiKey").is_none());
    }

    #[test]
    fn parser_failure_document_ir_never_uses_sample_content() {
        let job = test_job();
        let source = test_source("pdf");
        let ir = parser_failure_document_ir(&job, &source, "auto", "boom");

        assert_eq!(
            ir.pointer("/parser/provider").and_then(Value::as_str),
            Some("python-parser-sidecar:failure")
        );
        assert_eq!(
            ir.pointer("/pages/0/blocks/0/confidence")
                .and_then(Value::as_f64),
            Some(0.0)
        );
        let warnings = ir
            .pointer("/parser/warnings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(warnings
            .iter()
            .any(|item| item.as_str() == Some("no-sample-content-generated")));
        let text = ir
            .pointer("/pages/0/blocks/0/text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(!text.contains("Detective fiction"));
    }

    #[test]
    fn refresh_authoring_review_state_requires_low_confidence_verification() {
        let job = test_job();
        let mut ir = sample_document_ir(&job, "auto");
        ir = make_dynamic_authoring_ir(
            &job,
            &make_dynamic_split_candidates(&job.job_id, &job, Some(&ir)),
            Some(&ir),
        );

        let needs_review = refresh_authoring_review_state(&mut ir);

        assert!(needs_review > 0);
        assert_eq!(
            ir.pointer("/audit/humanVerified").and_then(Value::as_bool),
            Some(false)
        );

        for group in ir
            .get_mut("groups")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(obj) = group.as_object_mut() {
                obj.insert("verified".to_string(), json!(true));
            }
            for question in group
                .get_mut("questions")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                if let Some(obj) = question.as_object_mut() {
                    obj.insert("verified".to_string(), json!(true));
                }
            }
        }

        let needs_review = refresh_authoring_review_state(&mut ir);

        assert_eq!(needs_review, 0);
        assert_eq!(
            ir.pointer("/audit/humanVerified").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn publish_review_issues_block_empty_answers() {
        let job = test_job();
        let doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));

        let issues = authoring_review_issues(&ir);

        assert!(issues.iter().any(|issue| issue
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("requires human verification")));
    }

    #[test]
    fn source_review_issues_block_even_when_authoring_is_human_verified() {
        let job = test_job();
        let source = test_source("pdf");
        let doc = parser_failure_document_ir(&job, &source, "auto", "boom");
        let review = json!({
            "schemaVersion": "SourceReviewV1",
            "jobId": job.job_id,
            "required": true,
            "resolved": false,
            "stale": false,
            "fingerprint": "fixture",
            "parserWarnings": parser_warnings(Some(&doc)),
            "lowConfidenceBlocks": low_confidence_block_ids(Some(&doc), 0.5),
            "resolvedAt": null,
            "note": null
        });

        let issues = source_review_issues(&review);

        assert!(issues.iter().any(|issue| issue
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("Parser warning")));
        assert!(issues.iter().any(|issue| issue
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("Low-confidence")));
    }

    #[test]
    fn source_review_fingerprint_changes_when_low_confidence_text_changes() {
        let job = test_job();
        let source = test_source("pdf");
        let mut doc = parser_failure_document_ir(&job, &source, "auto", "boom");
        let before = source_review_fingerprint(Some(&doc));
        doc["pages"][0]["blocks"][0]["text"] = json!("[Different low-confidence content]");
        let after = source_review_fingerprint(Some(&doc));

        assert_ne!(before, after);
    }

    #[test]
    fn missing_source_document_ir_never_uses_sample_content() {
        let job = test_job();
        let ir = missing_source_document_ir(&job, "auto", "no source");

        assert_eq!(
            ir.pointer("/parser/provider").and_then(Value::as_str),
            Some("local-parser:source-missing")
        );
        assert_eq!(
            ir.pointer("/pages/0/blocks/0/confidence")
                .and_then(Value::as_f64),
            Some(0.0)
        );
        let text = ir
            .pointer("/pages/0/blocks/0/text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(!text.contains("Detective fiction"));
    }

    #[test]
    fn manual_transcription_document_ir_reaches_split_answers() {
        let job = test_job();
        let transcript = "\
READING PASSAGE 1
Manual passage text about tides.

Questions 1-3
1 The tide rises daily.
2 The passage mentions storms.
3 The passage discusses satellites.

Answers
1 TRUE
2 FALSE
3 NOT GIVEN";

        let doc = manual_transcription_document_ir(&job, transcript, Some("operator transcript"));
        assert_eq!(
            doc.pointer("/parser/provider").and_then(Value::as_str),
            Some("manual-transcription")
        );
        assert!(parser_warnings(Some(&doc))
            .iter()
            .any(|warning| warning.contains("manual transcription")));
        assert!(dynamic_document_blocks(Some(&doc))
            .iter()
            .any(|block| dynamic_block_role(block) == "question"));
        assert!(dynamic_document_blocks(Some(&doc))
            .iter()
            .any(|block| dynamic_block_role(block) == "answer"));

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .pointer("/answerKeyCandidates/0/answers/1")
                .and_then(Value::as_str),
            Some("TRUE")
        );
        assert_eq!(
            split
                .pointer("/answerKeyCandidates/0/answers/3")
                .and_then(Value::as_str),
            Some("NOT GIVEN")
        );
    }

    #[test]
    fn vision_transcription_document_ir_requires_source_review_and_reaches_split() {
        let job = test_job();
        let transcript = "\
READING PASSAGE 1
Vision transcript text about tides.

Questions 1-2
1 The page was transcribed by a vision model.
2 The author must verify the transcript.

Answers
1 TRUE
2 TRUE";

        let doc = vision_transcription_document_ir(
            &job,
            transcript,
            0.91,
            vec![],
            json!({"source": "unit-test"}),
            Some("vision fixture"),
        );

        assert_eq!(
            doc.pointer("/parser/provider").and_then(Value::as_str),
            Some("vision-llm-transcription")
        );
        assert!(parser_warnings(Some(&doc))
            .iter()
            .any(|warning| warning.contains("vision LLM transcription")));
        let review = json!({
            "schemaVersion": "SourceReviewV1",
            "jobId": job.job_id,
            "required": true,
            "resolved": false,
            "stale": false,
            "fingerprint": "fixture",
            "parserWarnings": parser_warnings(Some(&doc)),
            "lowConfidenceBlocks": low_confidence_block_ids(Some(&doc), 0.5),
            "resolvedAt": null,
            "note": null
        });
        assert!(!source_review_issues(&review).is_empty());

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .pointer("/answerKeyCandidates/0/answers/1")
                .and_then(Value::as_str),
            Some("TRUE")
        );
        assert_eq!(
            split
                .pointer("/answerKeyCandidates/0/answers/2")
                .and_then(Value::as_str),
            Some("TRUE")
        );
    }

    #[test]
    fn image_only_pdf_fixture_exposes_embedded_images_for_vision() {
        let job = test_job();
        let fixture = parser_fixture("image-only-reading.pdf");
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let output = root
            .join("cache")
            .join("parser")
            .join("image-extraction.json");
        let asset_dir = root.join("cache").join("parser").join("image-assets");

        let extraction =
            extract_pdf_images_with_python_sidecar(&job.job_id, &fixture, &output, &asset_dir)
                .expect("image-only PDF fixture should expose an embedded image");
        let image_count = extraction
            .get("pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|page| {
                page.get("images")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .count();

        assert!(image_count > 0);
        assert!(extraction
            .get("warnings")
            .and_then(Value::as_array)
            .map(|warnings| warnings.is_empty())
            .unwrap_or(false));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn answer_key_source_candidates_merge_into_split_answers() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let mut job = test_job();
        let main_source = test_source("txt");
        let answer_source = test_answer_source("txt");
        job.source_files = vec![main_source, answer_source.clone()];
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        write_text(
            &job_dir(&root, &job.job_id)
                .join("uploads")
                .join(&answer_source.stored_name),
            "Answers\n1 TRUE\n2 FALSE\n3 NOT GIVEN\n",
        )
        .unwrap();

        let mut split = json!({
            "jobId": job.job_id,
            "passageCandidates": [],
            "questionGroupCandidates": [],
            "answerKeyCandidates": [],
            "issues": ["No answer key detected; answers must be entered manually."]
        });
        let candidates = parse_answer_source_candidates(&root, &job, "auto").unwrap();
        merge_answer_source_candidates(&mut split, candidates);

        assert_eq!(
            split
                .pointer("/answerKeyCandidates/0/source")
                .and_then(Value::as_str),
            Some("answer-source:file-answer")
        );
        assert_eq!(
            split
                .pointer("/answerKeyCandidates/0/answers/1")
                .and_then(Value::as_str),
            Some("TRUE")
        );
        assert_eq!(
            split
                .pointer("/answerKeyCandidates/0/answers/3")
                .and_then(Value::as_str),
            Some("NOT GIVEN")
        );
        assert!(split
            .get("issues")
            .and_then(Value::as_array)
            .map(|issues| issues.is_empty())
            .unwrap_or(false));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validate_authoring_blocks_duplicate_display_numbers_and_gaps() {
        let job = test_job();
        let doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let mut ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        if let Some(question) = ir
            .get_mut("groups")
            .and_then(Value::as_array_mut)
            .and_then(|groups| groups.first_mut())
            .and_then(|group| group.get_mut("questions"))
            .and_then(Value::as_array_mut)
            .and_then(|questions| questions.get_mut(1))
        {
            question["id"] = json!("q4");
            question["displayNumber"] = json!("1");
        }

        let report = validate_authoring(&job.job_id, Some(&ir));
        let messages = report
            .get("issues")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|issue| issue.get("message").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(messages.contains("numerically continuous"));
        assert!(messages.contains("Duplicate display number"));
    }

    #[test]
    fn auto_applied_llm_patch_does_not_create_human_verification() {
        let job = test_job();
        let doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let mut ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let first_group_id = ir
            .pointer("/groups/0/groupId")
            .and_then(Value::as_str)
            .unwrap_or("group-1")
            .to_string();
        let suggestion = json!({
            "suggestionId": "suggestion-test",
            "groupId": first_group_id,
            "confidence": 0.99,
            "patch": [{"op":"replace","path":"/kind","value":"short_answer"}],
            "questions": []
        });

        apply_suggestion_to_authoring(&mut ir, &suggestion, &["kind".to_string()]).unwrap();
        if let Some(group) = ir
            .get_mut("groups")
            .and_then(Value::as_array_mut)
            .and_then(|groups| groups.first_mut())
        {
            group["autoApplied"] = json!(true);
        }
        let needs_review = refresh_authoring_review_state(&mut ir);

        assert!(needs_review > 0);
        assert_eq!(
            ir.pointer("/audit/humanVerified").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn high_confidence_llm_without_evidence_cannot_auto_apply() {
        let job = test_job();
        let doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let first_group_id = ir
            .pointer("/groups/0/groupId")
            .and_then(Value::as_str)
            .unwrap_or("group-1");
        let suggestion = json!({
            "suggestionId": "suggestion-no-evidence",
            "groupId": first_group_id,
            "kind": "short_answer",
            "confidence": 0.99,
            "patch": [{"op":"replace","path":"/kind","value":"short_answer"}],
            "questions": [],
            "evidence": {"source":"openai-compatible"}
        });

        let issues = llm_suggestion_auto_apply_issues(&ir, &suggestion, &["kind".to_string()]);

        assert!(issues.contains(&"evidence_source_block_ids_missing".to_string()));
        assert!(issues.contains(&"evidence_quotes_missing".to_string()));
    }

    #[test]
    fn high_confidence_llm_with_source_block_evidence_can_auto_apply() {
        let job = test_job();
        let doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let mut ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let first_group = ir
            .pointer("/groups/0")
            .cloned()
            .expect("fixture should have a group");
        let first_group_id = first_group
            .get("groupId")
            .and_then(Value::as_str)
            .unwrap_or("group-1")
            .to_string();
        let first_block_id = first_group
            .get("sourceBlockIds")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str)
            .unwrap_or("b004")
            .to_string();
        let suggestion = json!({
            "suggestionId": "suggestion-with-evidence",
            "groupId": first_group_id,
            "kind": "short_answer",
            "confidence": 0.99,
            "patch": [{"op":"replace","path":"/kind","value":"short_answer"}],
            "questions": [],
            "warnings": [],
            "evidence": {
                "source": "openai-compatible",
                "sourceBlockIds": [first_block_id],
                "quotes": [{"blockId": first_block_id, "text": "Questions 1-5"}]
            }
        });

        let issues = llm_suggestion_auto_apply_issues(&ir, &suggestion, &["kind".to_string()]);
        assert!(issues.is_empty(), "unexpected issues: {:?}", issues);

        apply_suggestion_to_authoring(&mut ir, &suggestion, &["kind".to_string()]).unwrap();
        if let Some(group) = ir
            .get_mut("groups")
            .and_then(Value::as_array_mut)
            .and_then(|groups| groups.first_mut())
        {
            group["autoApplied"] = json!(true);
        }
        let needs_review = refresh_authoring_review_state(&mut ir);

        assert!(needs_review > 0);
        assert_eq!(
            ir.pointer("/audit/humanVerified").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn no_text_pdf_fixture_requires_source_review() {
        let job = test_job();
        let source = test_source("pdf");
        let fixture = parser_fixture("no-text.pdf");
        let output = env::temp_dir().join(format!(
            "epic8-no-text-pdf-{}-document-ir.json",
            Uuid::new_v4().simple()
        ));

        let ir = parse_source_document(&job, &source, &fixture, &output, "auto")
            .expect("no-text PDF fixture should parse through pypdf");

        assert_eq!(
            ir.pointer("/parser/provider").and_then(Value::as_str),
            Some("python-parser-sidecar:pdf:pypdf")
        );
        assert!(parser_warnings(Some(&ir))
            .iter()
            .any(|warning| warning.contains("no extractable text")));
        assert_eq!(low_confidence_block_ids(Some(&ir), 0.5), vec!["b001"]);

        let review = json!({
            "schemaVersion": "SourceReviewV1",
            "jobId": job.job_id,
            "required": true,
            "resolved": false,
            "stale": false,
            "fingerprint": "fixture",
            "parserWarnings": parser_warnings(Some(&ir)),
            "lowConfidenceBlocks": low_confidence_block_ids(Some(&ir), 0.5),
            "resolvedAt": null,
            "note": null
        });
        assert!(!source_review_issues(&review).is_empty());
        let _ = fs::remove_file(output);
    }

    #[test]
    fn reading_source_uses_real_source_metadata_and_review_status() {
        let mut job = test_job();
        job.source_files = vec![SourceFile {
            file_id: "file-real".to_string(),
            original_name: "Cambridge 18 Test 1.pdf".to_string(),
            stored_name: "abc12345-Cambridge-18-Test-1.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: "a".repeat(64),
            size_bytes: 431,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        }];
        let doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let mut ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        verify_all_authoring_items(&mut ir);

        let source = reading_source(&ir);

        assert_eq!(
            source.pointer("/meta/pdfFilename").and_then(Value::as_str),
            Some("Cambridge 18 Test 1.pdf")
        );
        assert_eq!(
            source
                .pointer("/sourceRefs/shuiPdf")
                .and_then(Value::as_str),
            Some("uploads/abc12345-Cambridge-18-Test-1.pdf")
        );
        assert_eq!(
            source.pointer("/audit/matchStatus").and_then(Value::as_str),
            Some("author_verified")
        );
        assert!(source
            .pointer("/audit/notes")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("sourceFileId:file-real"));
    }

    #[test]
    fn publish_gate_blocks_no_text_pdf_until_source_review_resolved() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let mut job = test_job();
        let source = test_source("pdf");
        job.source_files = vec![source.clone()];
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();

        let fixture = parser_fixture("no-text.pdf");
        let parser_output = root
            .join("cache")
            .join("parser")
            .join("no-text-document-ir.json");
        let doc = parse_source_document(&job, &source, &fixture, &parser_output, "auto").unwrap();
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();
        write_source_review_status(&root, &job.job_id, Some(&doc), false, None).unwrap();

        let authoring_doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&authoring_doc));
        let mut ir = make_dynamic_authoring_ir(&job, &split, Some(&authoring_doc));
        verify_all_authoring_items(&mut ir);
        write_json(&job_dir(&root, &job.job_id).join("authoring-ir.json"), &ir).unwrap();

        let report = json!({
            "jobId": job.job_id,
            "passed": true,
            "layers": [{"layer": "RuntimePreview", "passed": true, "issueCount": 0}],
            "issues": [],
            "runtime": {"mode": "real"}
        });

        let gated = publish_readiness_gate(&root, &job.job_id, &ir, report).unwrap();
        assert_eq!(gated.get("passed").and_then(Value::as_bool), Some(false));
        assert!(gated
            .get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|issue| issue
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("Parser warning")));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_runtime_validation_downgrades_stale_export_ready_status() {
        let root = temp_test_root();
        let (job, mut ir) = make_publishable_fixture(&root);
        update_job(&root, &job.job_id, |item| {
            item.status = JobStatus::ExportReady;
            item.current_step = WorkflowStep::Export;
        })
        .unwrap();
        ir["groups"] = json!([]);
        write_json(&job_dir(&root, &job.job_id).join("authoring-ir.json"), &ir).unwrap();

        let report = validate_for_runtime_gate(&root, &job.job_id, &ir, false).unwrap();
        apply_preview_e2e_job_state(&root, &job.job_id, &report, false).unwrap();

        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::ValidationFailed);
        assert!(saved.issue_counts.errors > 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_core_writes_assets_after_real_runtime_gate() {
        if !external_runtime_available() {
            eprintln!("skipping: external unified runtime env vars are not configured");
            return;
        }
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let expected_exam_id = ir
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let out_dir = root.join("manual-export");

        let result = export_reading_assets_core(
            &root,
            &job.job_id,
            out_dir.to_string_lossy().as_ref(),
            true,
        )
        .unwrap();

        let exam_id = result.get("examId").and_then(Value::as_str).unwrap();
        assert_eq!(exam_id, expected_exam_id);
        assert!(out_dir.join(format!("{}.json", exam_id)).exists());
        assert!(out_dir.join(format!("{}.js", exam_id)).exists());
        assert!(out_dir.join("manifest.js").exists());
        let report: Value = read_json(&out_dir.join("validation-report.json")).unwrap();
        assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
        assert_eq!(
            report.pointer("/runtime/mode").and_then(Value::as_str),
            Some("real")
        );
        assert_eq!(
            load_job(&root, &job.job_id).unwrap().status,
            JobStatus::ExportReady
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn build_pack_core_writes_zip_after_real_runtime_gate() {
        if !external_runtime_available() {
            eprintln!("skipping: external unified runtime env vars are not configured");
            return;
        }
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let expected_exam_id = ir
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let input = json!({
            "packId": "pack-fixture",
            "version": "0.1.0",
            "institution": "internal",
            "description": "fixture",
            "jobIds": [job.job_id]
        });

        let result = build_pack_core(&root, &input, true).unwrap();

        let output_path = PathBuf::from(result.get("outputPath").and_then(Value::as_str).unwrap());
        assert!(output_path.exists());
        assert!(output_path.metadata().unwrap().len() > 0);
        assert_eq!(result.get("entryCount").and_then(Value::as_u64), Some(3));
        assert_eq!(
            result
                .pointer("/manifest/exams/0/examId")
                .and_then(Value::as_str),
            Some(expected_exam_id.as_str())
        );
        assert_eq!(
            load_job(&root, &job.job_id).unwrap().status,
            JobStatus::Published
        );
        assert!(root
            .join("packs")
            .join("pack-fixture")
            .join("pack.json")
            .exists());
        assert!(root
            .join("packs")
            .join("pack-fixture")
            .join("reading-exams")
            .join("manifest.js")
            .exists());
        assert!(root
            .join("packs")
            .join("pack-fixture")
            .join("reading-exams")
            .join(format!("{}.js", expected_exam_id))
            .exists());

        let _ = fs::remove_dir_all(root);
    }

    fn assert_complex_fixture_pipeline(file_name: &str, provider: &str) {
        let mut job = test_job();
        let file_type = file_name.rsplit('.').next().unwrap();
        job.source_files = vec![test_source(file_type)];
        let output = env::temp_dir().join(format!(
            "epic8-complex-{}-{}-document-ir.json",
            file_type,
            Uuid::new_v4().simple()
        ));
        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &parser_fixture(file_name),
            &output,
            "auto",
        )
        .unwrap();

        assert_eq!(
            doc.pointer("/parser/provider").and_then(Value::as_str),
            Some(provider)
        );
        assert!(parser_warnings(Some(&doc)).is_empty());
        assert!(low_confidence_block_ids(Some(&doc), 0.5).is_empty());
        let blocks = dynamic_document_blocks(Some(&doc));
        assert!(blocks
            .iter()
            .any(|block| dynamic_block_role(block) == "passage"));
        assert!(blocks
            .iter()
            .any(|block| dynamic_block_role(block) == "question"));
        assert!(blocks
            .iter()
            .any(|block| dynamic_block_role(block) == "answer"));

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert!(split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .map(|items| items.len() >= 2)
            .unwrap_or(false));
        assert_eq!(
            split
                .pointer("/answerKeyCandidates/0/answers/1")
                .and_then(Value::as_str),
            Some("TRUE")
        );
        assert_eq!(
            split
                .pointer("/answerKeyCandidates/0/answers/5")
                .and_then(Value::as_str),
            Some("diaries")
        );

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.get("questionOrder")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(5)
        );
        assert_eq!(
            ir.pointer("/answerKey/q1").and_then(Value::as_str),
            Some("TRUE")
        );
        assert_eq!(
            ir.pointer("/answerKey/q5").and_then(Value::as_str),
            Some("diaries")
        );
        let _ = fs::remove_file(output);
    }

    #[test]
    fn complex_text_pdf_fixture_reaches_authoring_ir() {
        assert_complex_fixture_pipeline("complex-reading.pdf", "python-parser-sidecar:pdf:pypdf");
    }

    #[test]
    fn complex_docx_fixture_reaches_authoring_ir() {
        assert_complex_fixture_pipeline("complex-reading.docx", "python-parser-sidecar:docx:ooxml");
    }
}
