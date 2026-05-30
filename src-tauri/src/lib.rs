use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
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
    pub status: Option<JobStatus>,
    #[serde(rename = "currentStep")]
    pub current_step: Option<WorkflowStep>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ParseOptions {
    pub mode: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JobDetail {
    pub job: ImportJob,
    #[serde(rename = "documentIr")]
    pub document_ir: Option<Value>,
    #[serde(rename = "splitCandidates")]
    pub split_candidates: Option<Value>,
    #[serde(rename = "authoringIr")]
    pub authoring_ir: Option<Value>,
    #[serde(rename = "validationReport")]
    pub validation_report: Option<Value>,
    #[serde(rename = "previewAssets")]
    pub preview_assets: Option<Value>,
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
        let fallback = path.to_string_lossy().as_bytes().to_vec();
        Ok((hash_bytes(&fallback), 0, None))
    }
}

fn main_source_file(job: &ImportJob) -> Option<&SourceFile> {
    job.source_files
        .iter()
        .find(|source| source.role == "MainQuestion")
        .or_else(|| job.source_files.first())
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
            let mut ir = if matches!(source.file_type.as_str(), "txt" | "md") {
                let content = fs::read_to_string(upload_path).map_err(|read_error| {
                    format!("read_source_text:{}:{}", upload_path.display(), read_error)
                })?;
                text_document_ir(job, source, &content, mode)
            } else {
                sample_document_ir(job, mode)
            };
            append_parser_warning(
                &mut ir,
                format!(
                    "python parser sidecar fallback for {}: {}",
                    source.file_type, error
                ),
            );
            Ok(ir)
        }
    }
}

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
            "tags": job.tags
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

    json!({
        "schemaVersion":"ReadingExamSourceV1",
        "examId": authoring.pointer("/exam/examId").and_then(Value::as_str).unwrap_or("local-authoring-exam"),
        "meta": {
            "title": authoring.pointer("/exam/title").and_then(Value::as_str).unwrap_or("Untitled Reading"),
            "category": authoring.pointer("/exam/category").and_then(Value::as_str).unwrap_or("P1"),
            "frequency": authoring.pointer("/exam/frequency").and_then(Value::as_str).unwrap_or("medium"),
            "pdfFilename": "source.pdf",
            "legacyPath": "",
            "legacyFilename": "",
            "questionIntroHtml": "<h3>Questions</h3>"
        },
        "passage": {"blocks": authoring.pointer("/passage/htmlBlocks").cloned().unwrap_or(json!([{"blockId":"passage-main","kind":"html","html":""}]))},
        "questionGroups": groups,
        "answerKey": answer_key_from_authoring(authoring),
        "sourceRefs": {"primaryHtml": format!("author-imports/{}/intermediate.html", authoring.get("jobId").and_then(Value::as_str).unwrap_or("job")), "primaryProvider":"author_web", "shuiHtml": null, "shuiPdf":"uploads/source.pdf", "ieltsHtml": null},
        "audit": {"matchStatus":"author_verified", "matchConfidence":1, "verifiedAt":Utc::now().to_rfc3339(), "notes":"provider:author_tauri;signature:radio,text,table"},
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
        for qid in question_order_from_authoring(ir) {
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
    let default = PathBuf::from("/Users/maziheng/Downloads/0.3.1 working/assets/generated/reading-exams/reading-practice-unified.html");
    default.exists().then_some(default)
}

fn resolve_external_unified_python() -> Option<PathBuf> {
    if let Ok(value) = env::var("EPIC8_UNIFIED_PYTHON") {
        let path = PathBuf::from(value);
        if path.exists() {
            return Some(path);
        }
    }
    let default = PathBuf::from("/Users/maziheng/Downloads/0.3.1 working/.venv/bin/python");
    default.exists().then_some(default)
}

fn validate_for_runtime_gate(root: &Path, job_id: &str, ir: &Value) -> CommandResult<Value> {
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
    }
    write_json(
        &job_dir(root, job_id).join("validation-report.json"),
        &report,
    )?;
    Ok(report)
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

fn profile_has_secret(root: &Path, profile_id: &str) -> bool {
    matches!(keychain_load_secret(profile_id), Ok(Some(_)))
        || file_load_secret(root, profile_id).is_some()
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
        let profile = redact_profile_for_ui(root, json!({"profileId": profile_id}));
        return Ok((
            profile
                .get("hasApiKey")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            profile
                .get("secretStorageBackend")
                .and_then(Value::as_str)
                .unwrap_or("none")
                .to_string(),
            profile
                .get("secretStorageMessage")
                .and_then(Value::as_str)
                .unwrap_or("No API key is stored.")
                .to_string(),
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
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/llm-gateway/gateway.mjs")
        .ok_or_else(|| "llm_gateway_sidecar_missing".to_string())?;
    let cache_dir = job_dir(root, job_id).join("cache").join("llm");
    let stamp = Utc::now().timestamp_millis();
    let input_path = cache_dir.join(format!("{}-input-{}.json", command_name, stamp));
    let output_path = cache_dir.join(format!("{}-output-{}.json", command_name, stamp));
    write_json(&input_path, input)?;
    let output = Command::new("node")
        .arg(&script)
        .arg(command_name)
        .arg(&input_path)
        .arg(&output_path)
        .output()
        .map_err(|error| format!("llm_gateway_spawn_failed:{}:{}", script.display(), error))?;
    if !output.status.success() {
        return Err(command_failure("llm-gateway", &output));
    }
    read_json(&output_path)
}

fn deterministic_llm_output(group: &Value, mode: &str, warning: String) -> Value {
    let kind = group
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("short_answer");
    json!({
        "kind": kind,
        "confidence": if kind == "table_completion" { 0.78 } else { 0.88 },
        "patch": [
            {"op":"replace","path":"/kind","value":kind}
        ],
        "questions": group.get("questions").cloned().unwrap_or_else(|| json!([])),
        "warnings": [warning],
        "evidence": {"mode": mode, "source": "rust-local-fallback"}
    })
}

fn make_llm_input(
    root: &Path,
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
        "apiKey": load_profile_secret(root, profile_id).unwrap_or_default(),
        "group": group
    })
}

fn save_llm_suggestion(root: &Path, job_id: &str, suggestion: &Value) -> CommandResult<()> {
    let dir = job_dir(root, job_id);
    write_json(&dir.join("llm-last-suggestion.json"), suggestion)?;
    append_text(
        &dir.join("llm-calls.jsonl"),
        &format!(
            "{}\n",
            serde_json::to_string(suggestion).map_err(|error| error.to_string())?
        ),
    )
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
        split_candidates: read_json_opt(&dir.join("split-candidates.json"))?,
        authoring_ir: read_json_opt(&dir.join("authoring-ir.json"))?,
        validation_report: read_json_opt(&dir.join("validation-report.json"))?,
        preview_assets: read_json_opt(&dir.join("preview").join("preview-assets.json"))?,
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
        if let Some(status) = patch.status {
            job.status = status;
        }
        if let Some(step) = patch.current_step {
            job.current_step = step;
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
    if let Some(bytes) = bytes {
        fs::write(dir.join("uploads").join(&stored_name), bytes)
            .map_err(|error| error.to_string())?;
    } else {
        write_text(&dir.join("uploads").join(format!("{}.missing.txt", stored_name)), "Original file path was not readable from the current embedded UI context. Use the Tauri file dialog command in production flow.\n")?;
    }
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
            sample_document_ir(&job, mode)
        }
    } else {
        sample_document_ir(&job, mode)
    };
    write_json(&job_dir(&root, &job_id).join("document-ir.json"), &ir)?;
    update_job(&root, &job_id, |job| {
        job.status = JobStatus::Parsed;
        job.current_step = WorkflowStep::DocumentReview;
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
async fn run_rule_split(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let job = load_job(&root, &job_id)?;
    let doc = read_json_opt(&job_dir(&root, &job_id).join("document-ir.json"))?;
    let split = make_dynamic_split_candidates(&job_id, &job, doc.as_ref());
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
        None => make_dynamic_split_candidates(&job_id, &job, doc.as_ref()),
    };
    write_json(&dir.join("split-candidates.json"), &split)?;
    let ir = make_dynamic_authoring_ir(&job, &split, doc.as_ref());
    write_json(&job_dir(&root, &job_id).join("authoring-ir.json"), &ir)?;
    update_job(&root, &job_id, |job| {
        job.status = JobStatus::AuthoringReady;
        job.current_step = WorkflowStep::Authoring;
        job.issue_counts = IssueCounts {
            errors: 0,
            warnings: 1,
            needs_review: 8,
        };
    })?;
    Ok(ir)
}

#[tauri::command]
async fn update_authoring_ir(job_id: String, patch: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let mut ir = patch.get("ir").cloned().unwrap_or(patch);
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
        job.status = JobStatus::AuthoringReady;
        job.current_step = WorkflowStep::Authoring;
        job.issue_counts.needs_review = 0;
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
        "apiKey": load_profile_secret(&root, &profile_id).unwrap_or_default(),
        "group": {"groupId": "test", "kind": "short_answer", "instruction": ["Return JSON only."], "questions": []}
    });
    let result = run_llm_gateway(&root, "profile-test", "test_profile", &input);
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
    let input = make_llm_input(&root, &profile, &job, &group, &profile_id, "classify_group");
    let output =
        run_llm_gateway(&root, &job_id, "classify_group", &input).unwrap_or_else(|error| {
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
    let input = make_llm_input(&root, &profile, &job, &group, &profile_id, "extract_group");
    let output = run_llm_gateway(&root, &job_id, "extract_group", &input).unwrap_or_else(|error| {
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
    let suggestion: Value = read_json(&job_dir(&root, &job_id).join("llm-last-suggestion.json"))?;
    if suggestion.get("suggestionId").and_then(Value::as_str) != Some(suggestion_id.as_str()) {
        return Err(format!("suggestion_not_found:{}", suggestion_id));
    }
    if suggestion
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        < 0.85
    {
        return Err("low_confidence_suggestion_requires_manual_review".to_string());
    }
    apply_suggestion_to_authoring(&mut ir, &suggestion, &selected_paths)?;
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
        job.status = JobStatus::AuthoringReady;
        job.current_step = WorkflowStep::Authoring;
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
                if let Some(issues) = report.get_mut("issues").and_then(Value::as_array_mut) {
                    issues.push(json!({
                        "issueId": format!("issue-{}", Uuid::new_v4().simple()),
                        "severity": "warning",
                        "layer": "ReadingExamSourceV1",
                        "path": "$",
                        "message": format!("Node validator sidecar unavailable; used built-in validation only: {}", error),
                        "fixHint": "Verify node is installed and sidecars/node-validator/validate-reading-source.mjs is bundled."
                    }));
                }
            }
        }
    }
    write_json(
        &job_dir(&root, &job_id).join("validation-report.json"),
        &report,
    )?;
    update_job(&root, &job_id, |job| {
        job.status = if report
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
    })?;
    Ok(report)
}

#[tauri::command]
async fn generate_preview_assets(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let ir: Value = read_json(&job_dir(&root, &job_id).join("authoring-ir.json"))?;
    let source = reading_source(&ir);
    let (_, _, _, _, assets) = preview_assets_for_source(&root, &job_id, &source)?;
    update_job(&root, &job_id, |job| {
        job.status = JobStatus::PreviewReady;
        job.current_step = WorkflowStep::Preview;
    })?;
    Ok(assets)
}

#[tauri::command]
async fn run_preview_e2e(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let authoring = read_json_opt(&job_dir(&root, &job_id).join("authoring-ir.json"))?;
    let report = if let Some(ir) = authoring.as_ref() {
        validate_for_runtime_gate(&root, &job_id, ir)?
    } else {
        let report = validate_authoring(&job_id, None);
        write_json(
            &job_dir(&root, &job_id).join("validation-report.json"),
            &report,
        )?;
        report
    };
    if report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        update_job(&root, &job_id, |job| {
            job.status = JobStatus::ExportReady;
            job.current_step = WorkflowStep::Export;
        })?;
    }
    Ok(report)
}

#[tauri::command]
async fn export_reading_assets(
    job_id: String,
    export_dir: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let ir: Value = read_json(&job_dir(&root, &job_id).join("authoring-ir.json"))?;
    let report = validate_for_runtime_gate(&root, &job_id, &ir)?;
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
        job_dir(&root, &job_id).join("exports")
    } else {
        PathBuf::from(export_dir)
    };
    fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;
    write_json(&out_dir.join(format!("{}.json", exam_id)), &source)?;
    write_text(&out_dir.join(format!("{}.js", exam_id)), &wrapper_js)?;
    write_text(&out_dir.join("manifest.js"), &manifest_js)?;
    write_json(&out_dir.join("validation-report.json"), &report)?;
    update_job(&root, &job_id, |job| {
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
    fs::create_dir_all(&exams_dir).map_err(|error| error.to_string())?;
    let mut sources = Vec::new();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for job_id in input
        .get("jobIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let ir: Value = read_json(&job_dir(&root, job_id).join("authoring-ir.json"))?;
        let source = reading_source(&ir);
        let report = validate_for_runtime_gate(&root, job_id, &ir)?;
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
        write_text(&exams_dir.join(format!("{}.js", exam_id)), &wrapper)?;
        entries.push((script_name, wrapper.into_bytes()));
        update_job(&root, job_id, |job| {
            job.status = JobStatus::Published;
            job.current_step = WorkflowStep::Pack;
        })?;
        sources.push(source);
    }
    let manifest_js = build_manifest(&sources)?;
    let pack_manifest = build_pack_manifest(&input, &sources);
    let pack_json =
        serde_json::to_string_pretty(&pack_manifest).map_err(|error| error.to_string())?;
    write_text(&exams_dir.join("manifest.js"), &manifest_js)?;
    write_text(&pack_dir.join("pack.json"), &pack_json)?;
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
            export_reading_assets,
            build_pack
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
