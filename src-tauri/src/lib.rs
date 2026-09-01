use artifact_store::inspect_job_artifacts;
use authoring_commands::{
    apply_manual_transcription_core, apply_vision_transcription_core, build_authoring_ir_core,
    parse_document_core, render_group_html_core, resolve_source_review_core, run_rule_split_core,
    save_split_adjustments_core, update_authoring_ir_core,
};
use authoring_v2_commands::{
    apply_authoring_v2_patches_core, export_authoring_v2_core, get_authoring_v2_core,
    resolve_authoring_asset_preview_core,
};
use auto_pipeline::{run_auto_pipeline_core, run_cloud_review_core};
use chrono::{DateTime, Utc};
use diagnostics::DiagnosticsSettings;
use export_artifacts::{build_wrapper, safe_exam_id};
use export_nas_library::{
    export_nas_library_core, nas_reading_exams_dir, resolve_real_nas_library_root,
    write_nas_reading_sources,
};
#[cfg(test)]
use export_pack::export_reading_assets_core;
use export_pack::{
    export_reading_assets_with_options_core, export_reading_js_core, ExportValidationOptions,
};
use export_writing_library::export_writing_library_core;
use llm_commands::{
    apply_llm_suggestion_core, delete_llm_profile_core, llm_run_group_core, save_llm_profile_core,
    test_llm_profile_core,
};
use pdf_facts_shadow::debug_document_ir_v2_overlay_core;
use preview_commands::{
    generate_preview_assets_core, run_preview_e2e_core, validate_authoring_ir_core,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};
use util::ensure_app_dirs;
mod artifact_store;
mod authoring_commands;
mod authoring_pipeline;
mod authoring_review;
mod authoring_v2_commands;
mod authoring_validation;
mod auto_pipeline;
mod cleanup;
#[cfg(test)]
mod cross_repo_contract_fixture;
mod db;
mod diagnostics;
mod docx_facts_shadow;
mod docx_ingest;
mod environment;
mod export_artifacts;
mod export_nas_library;
mod export_pack;
mod export_writing_library;
mod ielts_grammar;
mod job_commands;
mod job_store;
mod library_commands;
mod llm_commands;
mod llm_gateway;
mod llm_profiles;
mod llm_suggestions;
mod nas_package_v2;
mod parser;
mod pdf_facts_shadow;
mod pdf_geometry;
mod pdf_ingest;
mod preview_commands;
mod reading_runtime_v2;
mod reading_source;
mod reading_source_v2;
mod runtime_compiler;
mod runtime_validation;
pub mod schema;
mod source_review;
mod util;
mod validator;
mod workflow_state;
mod writing_store;
use tauri::{AppHandle, Manager};

pub type CommandResult<T> = Result<T, String>;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum JobStatus {
    #[serde(
        alias = "Draft",
        alias = "Uploaded",
        alias = "Parsed",
        alias = "SplitReady"
    )]
    Working,
    #[serde(alias = "NeedsHumanReview", alias = "ValidationFailed")]
    NeedsReview,
    #[serde(alias = "AuthoringReady")]
    DraftSaved,
    #[serde(alias = "PreviewReady")]
    ExportReady,
    #[serde(alias = "Published")]
    Exported,
    Cleaned,
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

// ── 题库管理（library）类型 ───────────────────────────────────────────────
// 统一收录阅读 + 写作题目，subject 区分；status 用统一枚举避免两套模型混淆。

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LibraryFilter {
    pub subject: Option<String>,  // "reading" | "writing"
    pub status: Option<String>,   // draft | needs_review | ready | exported
    pub category: Option<String>, // P1|P2|P3 (阅读) | task1|task2 (写作)
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LibraryExamSummary {
    pub id: String,
    #[serde(rename = "examId")]
    pub exam_id: Option<String>,
    pub title: String,
    pub subject: String,
    pub category: Option<String>,
    pub frequency: Option<String>,
    pub status: String,
    #[serde(rename = "taskType")]
    pub task_type: Option<String>,
    pub tags: Vec<String>,
    #[serde(rename = "sourceHash")]
    pub source_hash: Option<String>,
    #[serde(rename = "issueErrors")]
    pub issue_errors: u32,
    #[serde(rename = "issueWarnings")]
    pub issue_warnings: u32,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LibraryExamDetail {
    pub summary: LibraryExamSummary,
    pub payload: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LibraryMetaPatch {
    pub title: Option<String>,
    pub category: Option<String>,
    pub frequency: Option<String>,
    pub status: Option<String>,
    #[serde(rename = "taskType")]
    pub task_type: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LibraryStats {
    pub total: u32,
    #[serde(rename = "bySubject")]
    pub by_subject: std::collections::BTreeMap<String, u32>,
    #[serde(rename = "byStatus")]
    pub by_status: std::collections::BTreeMap<String, u32>,
    #[serde(rename = "byCategory")]
    pub by_category: std::collections::BTreeMap<String, u32>,
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
    #[serde(rename = "executionMode")]
    pub execution_mode: Option<String>,
    pub target: Option<String>,
    #[serde(rename = "allowOverwrite")]
    pub allow_overwrite: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RunCloudReviewInput {
    #[serde(rename = "profileId")]
    pub profile_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RegenerateDraftInput {
    #[serde(rename = "allowOverwrite")]
    pub allow_overwrite: Option<bool>,
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

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    bytes_to_hex(&hasher.finalize())
}

pub(crate) fn main_source_file(job: &ImportJob) -> Option<&SourceFile> {
    job.source_files
        .iter()
        .find(|source| source.role == "MainQuestion")
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

pub(crate) fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#039;")
}

fn inferred_category_from_name(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("p3") {
        "P3".to_string()
    } else if lower.contains("p2") {
        "P2".to_string()
    } else {
        "P1".to_string()
    }
}

fn title_from_source_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled Reading");
    let without_prefix = stem
        .split_once(" - ")
        .map(|(_, rest)| rest)
        .unwrap_or(stem)
        .trim();
    without_prefix
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn generate_reading_source_from_path(path: &Path) -> CommandResult<Value> {
    if !path.exists() || !path.is_file() {
        return Err(format!("source_file_not_readable:{}", path.display()));
    }
    let original_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("source.pdf")
        .to_string();
    let (sha256, size_bytes, _) = util::hash_file_or_path(path)?;
    let source = SourceFile {
        file_id: "file-cli-main".to_string(),
        original_name: original_name.clone(),
        stored_name: original_name.clone(),
        file_type: util::file_type_from_name(&original_name).to_string(),
        sha256,
        size_bytes,
        role: "MainQuestion".to_string(),
        imported_at: Utc::now(),
    };
    let mut job = job_store::make_job(CreateJobInput {
        title: Some(title_from_source_path(path)),
        category: Some(inferred_category_from_name(&original_name)),
        frequency: Some("medium".to_string()),
        tags: Some(vec!["regression-cli".to_string()]),
        llm_profile_id: None,
    });
    job.source_files = vec![source.clone()];

    let scratch = env::temp_dir().join(format!(
        "ielts-author-studio-cli-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
    let parser_output = scratch.join("document-ir.json");
    let document_ir = parser::parse_source_document(&job, &source, path, &parser_output, "auto")?;
    let split_candidates =
        authoring_pipeline::make_dynamic_split_candidates(&job.job_id, &job, Some(&document_ir));
    let authoring_ir =
        authoring_pipeline::make_dynamic_authoring_ir(&job, &split_candidates, Some(&document_ir));
    let reading_source = reading_source::reading_source(&authoring_ir);
    let _ = fs::remove_dir_all(&scratch);

    Ok(json!({
        "schemaVersion": "ReadingSourceGenerationFixtureV1",
        "sourcePath": path.to_string_lossy(),
        "documentIr": document_ir,
        "splitCandidates": split_candidates,
        "authoringIr": authoring_ir,
        "readingSource": reading_source
    }))
}

fn cli_option_value(args: &[String], key: &str) -> Option<String> {
    args.windows(2).find_map(|pair| {
        if pair.first().map(String::as_str) == Some(key) {
            pair.get(1).cloned()
        } else {
            None
        }
    })
}

fn run_auto_pipeline_from_path(path: &Path, args: &[String]) -> CommandResult<Value> {
    if !path.exists() || !path.is_file() {
        return Err(format!("source_file_not_readable:{}", path.display()));
    }
    let base_url = cli_option_value(args, "--llm-base-url")
        .or_else(|| env::var("EPIC8_LIVE_LLM_BASE_URL").ok())
        .ok_or_else(|| "missing_llm_base_url".to_string())?;
    let model = cli_option_value(args, "--llm-model")
        .or_else(|| env::var("EPIC8_LIVE_LLM_MODEL").ok())
        .ok_or_else(|| "missing_llm_model".to_string())?;
    let api_key = cli_option_value(args, "--llm-api-key")
        .or_else(|| env::var("EPIC8_LIVE_LLM_API_KEY").ok())
        .ok_or_else(|| "missing_llm_api_key".to_string())?;
    let profile_id = cli_option_value(args, "--llm-profile-id")
        .unwrap_or_else(|| "profile-cli-live".to_string());
    let root = cli_option_value(args, "--app-root")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::temp_dir().join(format!(
                "ielts-author-studio-live-{}",
                uuid::Uuid::new_v4().simple()
            ))
        });

    util::ensure_app_dirs(&root)?;
    diagnostics::write_diagnostics_settings(
        &root,
        &DiagnosticsSettings {
            keep_full_process_artifacts: true,
        },
    )?;
    let profile = json!({
        "profileId": profile_id,
        "name": "CLI Live API",
        "provider": "OpenAiCompatible",
        "baseUrl": base_url,
        "model": model,
        "temperature": 0,
        "timeoutMs": 240000u64,
        "forceJson": true,
        "enabled": true
    });
    llm_profiles::save_profiles(&root, &[profile])?;
    env::set_var("EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK", "1");
    llm_profiles::file_save_secret(&root, &profile_id, &api_key)?;

    let original_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("source.pdf")
        .to_string();
    let (sha256, size_bytes, bytes) = util::hash_file_or_path(path)?;
    let stored_name = format!(
        "{}-{}",
        &sha256[..8],
        util::sanitize_filename(&original_name)
    );
    let source = SourceFile {
        file_id: "file-cli-main".to_string(),
        original_name: original_name.clone(),
        stored_name: stored_name.clone(),
        file_type: util::file_type_from_name(&original_name).to_string(),
        sha256,
        size_bytes,
        role: "MainQuestion".to_string(),
        imported_at: Utc::now(),
    };
    let mut job = job_store::make_job(CreateJobInput {
        title: Some(title_from_source_path(path)),
        category: Some(inferred_category_from_name(&original_name)),
        frequency: Some("medium".to_string()),
        tags: Some(vec!["cli-live-api".to_string()]),
        llm_profile_id: Some(profile_id.clone()),
    });
    job.source_files = vec![source.clone()];
    let dir = util::job_dir(&root, &job.job_id);
    util::ensure_job_dirs(&dir)?;
    util::write_bytes(
        &dir.join("uploads").join(&stored_name),
        &bytes.ok_or_else(|| format!("source_file_not_readable:{}", path.display()))?,
    )?;
    job_store::save_job(&root, &job)?;

    let report = auto_pipeline::run_auto_pipeline_core(
        &root,
        &job.job_id,
        Some(AutoPipelineInput {
            profile_id: Some(profile_id),
            confidence_threshold: Some(0.85),
            parse_mode: Some("auto".to_string()),
            execution_mode: None,
            target: Some("editableDraft".to_string()),
            allow_overwrite: Some(false),
        }),
    )?;
    let authoring_ir = util::read_json_opt(&dir.join("authoring-ir.json"))?;
    let project = util::read_json_opt(&dir.join("authoring-project.json"))?;
    Ok(json!({
        "schemaVersion": "LiveAutoPipelineCliResultV1",
        "appRoot": root,
        "sourcePath": path.to_string_lossy(),
        "job": job_store::load_job(&root, &job.job_id)?,
        "report": report,
        "authoringIr": authoring_ir,
        "authoringProject": project,
        "generatedAt": Utc::now().to_rfc3339()
    }))
}

fn export_nas_library_from_pdf_paths(paths: &[PathBuf], args: &[String]) -> CommandResult<Value> {
    if paths.is_empty() {
        return Err("missing_source_path".to_string());
    }
    for path in paths {
        if !path.exists() || !path.is_file() {
            return Err(format!("source_file_not_readable:{}", path.display()));
        }
    }
    let export_dir =
        cli_option_value(args, "--export-dir").ok_or_else(|| "missing_export_dir".to_string())?;
    let version = cli_option_value(args, "--version")
        .unwrap_or_else(|| Utc::now().format("%Y.%m.%d-%H%M%S").to_string());
    let library_root = resolve_real_nas_library_root(&export_dir)?;
    fs::create_dir_all(&library_root).map_err(|error| error.to_string())?;
    let reading_exams_dir = nas_reading_exams_dir(&library_root);

    let mut seen_exam_ids = HashSet::new();
    let mut source_entries = Vec::with_capacity(paths.len());
    for path in paths {
        let generation = generate_reading_source_from_path(path)?;
        let source = generation
            .get("readingSource")
            .cloned()
            .ok_or_else(|| "generated_reading_source_missing".to_string())?;
        let exam_id = safe_exam_id(&source)?;
        if !seen_exam_ids.insert(exam_id.clone()) {
            return Err(format!("duplicate_exam_id:{exam_id}"));
        }
        let wrapper_js = build_wrapper(&source)?;
        source_entries.push((path.clone(), exam_id, source, wrapper_js));
    }

    let sources = source_entries
        .iter()
        .map(|(_, _, source, _)| source.clone())
        .collect::<Vec<_>>();
    let (manifest_js, manifest_asset_count) =
        write_nas_reading_sources(&reading_exams_dir, &sources)?;
    let exam_ids = source_entries
        .iter()
        .map(|(_, exam_id, _, _)| exam_id.clone())
        .collect::<Vec<_>>();
    let mut files = source_entries
        .iter()
        .map(|(_, exam_id, _, wrapper_js)| {
            json!({"name": format!("{exam_id}.js"), "content": wrapper_js.clone()})
        })
        .collect::<Vec<_>>();
    files.push(json!({
        "name": "manifest.js",
        "content": manifest_js.clone()
    }));

    Ok(json!({
        "schemaVersion": "NasLibraryCliExportResultV1",
        "mode": "nas-library-cli",
        "runtime": "nas-js-direct",
        "sourcePath": source_entries.first().map(|(path, _, _, _)| path.to_string_lossy().to_string()).unwrap_or_default(),
        "sourcePaths": source_entries.iter().map(|(path, _, _, _)| path.to_string_lossy().to_string()).collect::<Vec<_>>(),
        "libraryRoot": library_root.to_string_lossy(),
        "readingExamsDir": reading_exams_dir.to_string_lossy(),
        "version": version,
        "examId": exam_ids.first().cloned().unwrap_or_default(),
        "examIds": exam_ids,
        "assetCount": source_entries.len(),
        "manifestAssetCount": manifest_asset_count,
        "files": files,
        "readingSource": sources.first().cloned().unwrap_or(Value::Null),
        "readingSources": sources,
        "generatedAt": Utc::now().to_rfc3339()
    }))
}

fn run_cli(args: &[String]) -> CommandResult<bool> {
    let command = args.first().map(String::as_str);
    if !matches!(
        command,
        Some("--generate-reading-source" | "--run-auto-pipeline" | "--export-nas-library")
    ) {
        return Ok(false);
    }
    let mut out_path: Option<PathBuf> = None;
    let mut source_paths = Vec::new();
    let mut index = 1usize;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                out_path = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            "--export-dir" | "--version" => {
                if args.get(index + 1).is_none() {
                    return Err(format!("missing_value_for_cli_arg:{}", args[index]));
                }
                index += 2;
            }
            "--app-root" | "--llm-base-url" | "--llm-model" | "--llm-api-key"
            | "--llm-profile-id" => {
                if args.get(index + 1).is_none() {
                    return Err(format!("missing_value_for_cli_arg:{}", args[index]));
                }
                index += 2;
            }
            other if other.starts_with("--") => return Err(format!("unknown_cli_arg:{}", other)),
            source => {
                source_paths.push(PathBuf::from(source));
                index += 1;
            }
        }
    }
    if source_paths.is_empty() {
        return Err("missing_source_path".to_string());
    }
    let result = if command == Some("--generate-reading-source") {
        if source_paths.len() != 1 {
            return Err("generate_reading_source_requires_single_source".to_string());
        }
        generate_reading_source_from_path(&source_paths[0])?
    } else if command == Some("--export-nas-library") {
        export_nas_library_from_pdf_paths(&source_paths, args)?
    } else {
        if source_paths.len() != 1 {
            return Err("run_auto_pipeline_requires_single_source".to_string());
        }
        run_auto_pipeline_from_path(&source_paths[0], args)?
    };
    if let Some(path) = out_path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        util::write_json(&path, &result)?;
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
        );
    }
    Ok(true)
}

pub fn run_cli_or_app() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let is_cli = run_cli_is_active(&args);
    // In a release build the binary uses the Windows GUI subsystem (see
    // main.rs `windows_subsystem = "windows"`), so no console window is
    // allocated on launch. When invoked as a CLI (e.g.
    // `--generate-reading-source`), re-attach to the parent process's console
    // and reopen the std streams so `println!`/`eprintln!` still reach the
    // terminal. GUI launches skip this and run windowed with no console.
    if is_cli {
        attach_parent_console();
    }
    match run_cli(&args) {
        Ok(true) => {}
        Ok(false) => run(),
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(1);
        }
    }
}

/// Returns true when the given args select the CLI path (a known headless
/// command) rather than the GUI. Mirrors the dispatch in `run_cli`.
fn run_cli_is_active(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some("--generate-reading-source")
            | Some("--run-auto-pipeline")
            | Some("--export-nas-library")
            | Some("--help")
            | Some("-h")
            | Some("--version")
            | Some("-V")
    )
}

/// Attach to the parent process's console (Windows release builds only) and
/// rebind the standard handles so CLI output is visible. No-op on non-Windows,
/// when no parent console exists (e.g. launched from Explorer), or when
/// stdout/stderr are already redirected to a file/pipe (so
/// `app.exe --generate-reading-source > out.json` still works).
#[cfg(all(target_os = "windows", not(debug_assertions)))]
fn attach_parent_console() {
    // Raw FFI to kernel32 — avoids pulling in the windows-sys/winapi crate.
    // Rust's `println!`/`eprintln!` write via io::stdout()/io::stderr(), which
    // read the standard handle through GetStdHandle on each write, so calling
    // SetStdHandle with a new console output handle is enough to redirect
    // Rust's std streams (no C-runtime freopen needed).
    extern "system" {
        fn AttachConsole(pid: u32) -> i32;
        fn GetStdHandle(nstd: u32) -> *mut std::ffi::c_void;
        fn SetStdHandle(nstd: u32, handle: *mut std::ffi::c_void) -> i32;
        fn GetConsoleMode(handle: *mut std::ffi::c_void, mode: *mut u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFFFFFF;
    const STD_INPUT_HANDLE: u32 = 0xFFFFFFF6; // (u32)-10
    const STD_OUTPUT_HANDLE: u32 = 0xFFFFFFF5; // (u32)-11
    const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4; // (u32)-12

    unsafe {
        let out_handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let err_handle = GetStdHandle(STD_ERROR_HANDLE);
        let mut mode: u32 = 0;
        let out_is_console = !out_handle.is_null() && GetConsoleMode(out_handle, &mut mode) != 0;
        let err_is_console = !err_handle.is_null() && GetConsoleMode(err_handle, &mut mode) != 0;
        // Already wired to a console (launched from an existing console) —
        // nothing to do, and redirecting would break shell pipes.
        if out_is_console && err_is_console {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return; // no parent console to attach to (e.g. launched from Explorer)
        }
        // After AttachConsole, GetStdHandle returns the parent console's
        // handles. Rebind only the streams that were not already redirected
        // to a file/pipe, preserving `> file` and `|` behaviour.
        let conout = GetStdHandle(STD_OUTPUT_HANDLE);
        let conerr = GetStdHandle(STD_ERROR_HANDLE);
        let conin = GetStdHandle(STD_INPUT_HANDLE);
        if !out_is_console && !conout.is_null() {
            SetStdHandle(STD_OUTPUT_HANDLE, conout);
        }
        if !err_is_console && !conerr.is_null() {
            SetStdHandle(STD_ERROR_HANDLE, conerr);
        }
        if !conin.is_null() {
            SetStdHandle(STD_INPUT_HANDLE, conin);
        }
    }
}

#[cfg(not(all(target_os = "windows", not(debug_assertions))))]
fn attach_parent_console() {
    // Debug builds and non-Windows platforms always have a console / are
    // already wired to std streams.
}

#[tauri::command]
async fn create_import_job(input: CreateJobInput, app: AppHandle) -> CommandResult<ImportJob> {
    job_commands::create_import_job_core(input, app).await
}

#[tauri::command]
async fn list_jobs(filter: Option<JobFilter>, app: AppHandle) -> CommandResult<Vec<ImportJob>> {
    job_commands::list_jobs_core(filter, app).await
}

#[tauri::command]
async fn get_job(job_id: String, app: AppHandle) -> CommandResult<JobDetail> {
    job_commands::get_job_core(job_id, app).await
}

#[tauri::command]
async fn update_job_meta(
    job_id: String,
    patch: JobMetaPatch,
    app: AppHandle,
) -> CommandResult<ImportJob> {
    job_commands::update_job_meta_core(job_id, patch, app).await
}

#[tauri::command]
async fn delete_job(job_id: String, app: AppHandle) -> CommandResult<()> {
    job_commands::delete_job_core(job_id, app).await
}

#[tauri::command]
async fn import_source_file(
    job_id: String,
    file_path: String,
    role: String,
    app: AppHandle,
) -> CommandResult<SourceFile> {
    job_commands::import_source_file_core(job_id, file_path, role, app).await
}

#[tauri::command]
async fn reveal_job_folder(job_id: String, app: AppHandle) -> CommandResult<()> {
    job_commands::reveal_job_folder_core(job_id, app).await
}

#[tauri::command]
async fn choose_export_dir(app: AppHandle) -> CommandResult<Option<String>> {
    job_commands::choose_export_dir_core(app).await
}

#[tauri::command]
async fn pick_pdf_folder_sources(
    app: AppHandle,
) -> CommandResult<Vec<job_commands::PickedSourcePath>> {
    job_commands::pick_pdf_folder_sources_core(app).await
}

#[tauri::command]
async fn parse_document(
    job_id: String,
    options: ParseOptions,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    parse_document_core(&root, &job_id, options)
}

#[tauri::command]
async fn debug_document_ir_v2_overlay(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    debug_document_ir_v2_overlay_core(&root, &job_id)
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
    apply_manual_transcription_core(&root, &job_id, input)
}

#[tauri::command]
async fn apply_vision_transcription(
    job_id: String,
    input: Option<VisionTranscriptionInput>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        apply_vision_transcription_core(&root, &job_id, input)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn resolve_source_review(
    job_id: String,
    note: Option<String>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    resolve_source_review_core(&root, &job_id, note)
}

#[tauri::command]
async fn run_rule_split(
    job_id: String,
    input: Option<RegenerateDraftInput>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    run_rule_split_core(&root, &job_id, input)
}

#[tauri::command]
async fn save_split_adjustments(
    job_id: String,
    patch: Value,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    save_split_adjustments_core(&root, &job_id, patch)
}

#[tauri::command]
async fn build_authoring_ir(
    job_id: String,
    input: Option<RegenerateDraftInput>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    build_authoring_ir_core(&root, &job_id, input)
}

#[tauri::command]
async fn update_authoring_ir(job_id: String, patch: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    update_authoring_ir_core(&root, &job_id, patch)
}

#[tauri::command]
async fn get_authoring_v2(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    get_authoring_v2_core(&root, &job_id)
}

#[tauri::command]
async fn resolve_authoring_asset_preview(
    job_id: String,
    asset_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    resolve_authoring_asset_preview_core(&root, &job_id, &asset_id)
}

#[tauri::command]
async fn apply_authoring_v2_patches(input: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    apply_authoring_v2_patches_core(&root, input)
}

#[tauri::command]
async fn export_authoring_v2(input: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    export_authoring_v2_core(&root, input)
}

#[tauri::command]
async fn render_group_html(
    job_id: String,
    group_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    render_group_html_core(&root, &job_id, &group_id)
}

#[tauri::command]
async fn list_llm_profiles(app: AppHandle) -> CommandResult<Vec<Value>> {
    job_commands::list_llm_profiles_core(app).await
}

#[tauri::command]
async fn run_environment_preflight(_app: AppHandle) -> CommandResult<Value> {
    job_commands::run_environment_preflight_core().await
}

#[tauri::command]
async fn get_diagnostics_settings(app: AppHandle) -> CommandResult<DiagnosticsSettings> {
    job_commands::get_diagnostics_settings_core(app).await
}

#[tauri::command]
async fn save_diagnostics_settings(
    settings: DiagnosticsSettings,
    app: AppHandle,
) -> CommandResult<DiagnosticsSettings> {
    job_commands::save_diagnostics_settings_core(settings, app).await
}

#[tauri::command]
async fn get_job_artifact_status(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    inspect_job_artifacts(&root, &job_id)
}

#[tauri::command]
async fn save_llm_profile(input: SaveLlmProfileInput, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    save_llm_profile_core(&root, input)
}

#[tauri::command]
async fn delete_llm_profile(profile_id: String, app: AppHandle) -> CommandResult<Vec<Value>> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    delete_llm_profile_core(&root, &profile_id)
}

#[tauri::command]
async fn test_llm_profile(profile_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    tauri::async_runtime::spawn_blocking(move || test_llm_profile_core(&root, &profile_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn llm_classify_group(
    job_id: String,
    group_id: String,
    profile_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        llm_run_group_core(&root, &job_id, &group_id, &profile_id, "classify_group")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn llm_extract_group(
    job_id: String,
    group_id: String,
    profile_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        llm_run_group_core(&root, &job_id, &group_id, &profile_id, "extract_group")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn apply_llm_suggestion(
    job_id: String,
    suggestion_id: String,
    selected_paths: Vec<String>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    apply_llm_suggestion_core(&root, &job_id, &suggestion_id, selected_paths)
}

#[tauri::command]
async fn validate_authoring_ir(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    validate_authoring_ir_core(&root, &job_id)
}

#[tauri::command]
async fn generate_preview_assets(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    generate_preview_assets_core(&root, &job_id)
}

#[tauri::command]
async fn run_preview_e2e(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    run_preview_e2e_core(&root, &job_id)
}

#[tauri::command]
async fn export_reading_assets(
    job_id: String,
    export_dir: String,
    validation_policy: Option<String>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let options = ExportValidationOptions::from_policy(validation_policy.as_deref())?;
    export_reading_assets_with_options_core(&root, &job_id, &export_dir, true, options)
}

#[tauri::command]
async fn export_reading_js(input: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    export_reading_js_core(&root, &input, true)
}

#[tauri::command]
async fn export_nas_library(input: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    export_nas_library_core(&root, &input, true)
}

/// Opt-in Reading V2 package publisher.  The command is intentionally kept
/// separate from the legacy export command so the V1 manifest path remains
/// untouched while the lock/CAS/journal/manifest-last protocol is callable by
/// the real authoring application (not only by unit tests).
#[tauri::command]
async fn publish_nas_package_v2(input: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    nas_package_v2::publish_nas_package_v2_core(&root, input)
}

// ---------- 写作题库命令（独立模型，不污染阅读 ImportJob）----------
#[tauri::command]
async fn create_writing_job(input: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    util::ensure_app_dirs(&root)?;
    let parsed: writing_store::CreateWritingJobInput = serde_json::from_value(input)
        .map_err(|error| format!("create_writing_job_invalid_input:{}", error))?;
    let job = writing_store::make_writing_job(parsed);
    writing_store::save_writing_job(&root, &job)?;
    Ok(serde_json::to_value(&job).map_err(|error| error.to_string())?)
}

#[tauri::command]
async fn list_writing_jobs(filter: Option<Value>, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let parsed_filter = match filter {
        Some(value) => Some(
            serde_json::from_value::<writing_store::WritingJobFilter>(value)
                .map_err(|error| format!("list_writing_jobs_invalid_filter:{}", error))?,
        ),
        None => None,
    };
    let jobs = writing_store::list_writing_jobs(&root, parsed_filter)?;
    Ok(serde_json::to_value(&jobs).map_err(|error| error.to_string())?)
}

#[tauri::command]
async fn get_writing_job(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let job = writing_store::load_writing_job(&root, &job_id)?;
    Ok(serde_json::to_value(&job).map_err(|error| error.to_string())?)
}

#[tauri::command]
async fn update_writing_job(job_id: String, patch: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let parsed: writing_store::WritingJobPatch = serde_json::from_value(patch)
        .map_err(|error| format!("update_writing_job_invalid_patch:{}", error))?;
    let job = writing_store::update_writing_job(&root, &job_id, |job| {
        if let Some(title) = parsed.title {
            job.title = title;
        }
        if let Some(task_type) = parsed.task_type {
            let normalized = task_type.trim().to_lowercase();
            if normalized == "task1" || normalized == "task2" {
                job.task_type = normalized;
            }
        }
        if let Some(exam_id) = parsed.exam_id {
            job.exam_id = exam_id;
        }
        if let Some(prompt_text) = parsed.prompt_text {
            job.prompt_text = prompt_text;
        }
        if let Some(suggested_word_count) = parsed.suggested_word_count {
            job.suggested_word_count = suggested_word_count;
        }
        if let Some(status) = parsed.status {
            job.status = status;
        }
    })?;
    Ok(serde_json::to_value(&job).map_err(|error| error.to_string())?)
}

#[tauri::command]
async fn delete_writing_job(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    writing_store::delete_writing_job(&root, &job_id)?;
    Ok(json!({ "deleted": true, "jobId": job_id }))
}

#[tauri::command]
async fn export_writing_library(input: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    export_writing_library_core(&root, &input)
}

#[tauri::command]
async fn run_auto_pipeline(
    job_id: String,
    input: Option<AutoPipelineInput>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || run_auto_pipeline_core(&root, &job_id, input))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn run_cloud_review(
    job_id: String,
    input: Option<RunCloudReviewInput>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || run_cloud_review_core(&root, &job_id, input))
        .await
        .map_err(|error| error.to_string())?
}

// ── 题库管理命令（library）──────────────────────────────────────────────────

#[tauri::command]
async fn list_library_exams(
    filter: Option<LibraryFilter>,
    app: AppHandle,
) -> CommandResult<Vec<LibraryExamSummary>> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        library_commands::list_library_exams_core(&root, filter)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_library_exam(id: String, app: AppHandle) -> CommandResult<Option<LibraryExamDetail>> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        library_commands::get_library_exam_core(&root, &id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn update_library_exam_meta(
    id: String,
    patch: LibraryMetaPatch,
    app: AppHandle,
) -> CommandResult<Option<LibraryExamSummary>> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        library_commands::update_library_exam_meta_core(&root, &id, patch)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn delete_library_exam(id: String, app: AppHandle) -> CommandResult<bool> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        library_commands::delete_library_exam_core(&root, &id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn search_library_exams(
    query: String,
    app: AppHandle,
) -> CommandResult<Vec<LibraryExamSummary>> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        library_commands::search_library_exams_core(&root, &query)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn get_library_stats(app: AppHandle) -> CommandResult<LibraryStats> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || library_commands::get_library_stats_core(&root))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn restore_library_exam(id: String, app: AppHandle) -> CommandResult<bool> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        library_commands::restore_library_exam_core(&root, &id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn list_trashed_exams(app: AppHandle) -> CommandResult<Vec<LibraryExamSummary>> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || library_commands::list_trashed_exams_core(&root))
        .await
        .map_err(|error| error.to_string())?
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState)
        .setup(|app| {
            let root = app_root(app.handle()).map_err(Box::<dyn std::error::Error>::from)?;
            ensure_app_dirs(&root).map_err(Box::<dyn std::error::Error>::from)?;
            // 初始化题库 DB schema + 首次启动迁移既有 job 数据（幂等，失败不阻断启动）。
            let _ = library_commands::migrate_existing_into_library(&root);
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
            pick_pdf_folder_sources,
            parse_document,
            debug_document_ir_v2_overlay,
            rerun_ocr,
            apply_manual_transcription,
            apply_vision_transcription,
            resolve_source_review,
            run_rule_split,
            save_split_adjustments,
            build_authoring_ir,
            update_authoring_ir,
            get_authoring_v2,
            resolve_authoring_asset_preview,
            apply_authoring_v2_patches,
            export_authoring_v2,
            render_group_html,
            list_llm_profiles,
            run_environment_preflight,
            get_diagnostics_settings,
            save_diagnostics_settings,
            get_job_artifact_status,
            save_llm_profile,
            delete_llm_profile,
            test_llm_profile,
            llm_classify_group,
            llm_extract_group,
            apply_llm_suggestion,
            validate_authoring_ir,
            generate_preview_assets,
            run_preview_e2e,
            run_auto_pipeline,
            run_cloud_review,
            export_reading_assets,
            export_reading_js,
            export_nas_library,
            publish_nas_package_v2,
            export_writing_library,
            create_writing_job,
            list_writing_jobs,
            get_writing_job,
            update_writing_job,
            delete_writing_job,
            list_library_exams,
            get_library_exam,
            update_library_exam_meta,
            delete_library_exam,
            search_library_exams,
            get_library_stats,
            restore_library_exam,
            list_trashed_exams
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring_pipeline::{
        dynamic_block_role, dynamic_block_text, dynamic_document_blocks,
        is_dynamic_umbrella_question_range, make_dynamic_authoring_ir,
        make_dynamic_split_candidates, merge_answer_source_candidates,
    };
    use crate::authoring_review::{authoring_review_issues, refresh_authoring_review_state};
    use crate::authoring_validation::{merge_validation_issues, validate_authoring};
    use crate::auto_pipeline::{
        main_pdf_needs_vision_transcription, parse_answer_source_candidates,
        run_auto_pipeline_core_with_gateway,
    };
    use crate::cleanup::cleanup_transient_job_artifacts;
    use crate::diagnostics::write_diagnostics_settings;
    use crate::environment::{command_probe, environment_preflight_report};
    use crate::export_artifacts::{build_manifest, build_wrapper, safe_exam_id};
    use crate::export_nas_library::{
        nas_reading_exams_dir, publish_nas_library_from_source_tree, write_source_payload_file,
    };
    use crate::job_store::{load_job, make_job, save_job, update_job};
    use crate::llm_profiles::{
        file_load_secret, file_save_secret, load_profile_secret, plaintext_secret_fallback_allowed,
        redact_profile_for_ui,
    };
    use crate::llm_suggestions::{
        apply_suggestion_to_authoring, deterministic_llm_output, llm_suggestion_auto_apply_issues,
        make_llm_input,
    };
    use crate::parser::{
        extract_pdf_images_for_vision, extract_pdf_images_with_python_sidecar,
        image_count_from_extraction, manual_transcription_document_ir, missing_source_document_ir,
        parse_source_document, parser_failure_document_ir, render_pdf_pages_with_adapter,
        vision_transcription_document_ir,
    };
    use crate::reading_source::reading_source;
    use crate::runtime_validation::{publish_readiness_gate, validate_for_runtime_gate};
    use crate::source_review::{
        low_confidence_block_ids, parser_warnings, resolve_source_review_status,
        source_review_fingerprint, source_review_issues, source_review_status,
        source_review_status_for_job, write_source_review_status,
    };
    use crate::util::{
        ensure_job_dirs, file_type_from_name, hash_file_or_path, is_safe_path_segment, job_dir,
        read_json, read_json_opt, safe_job_dir, sanitize_filename, validate_path_segment,
        write_bytes, write_json, write_text,
    };
    use crate::validator::allowed_question_kind;
    use crate::workflow_state::apply_preview_e2e_job_state;
    use std::{
        fs,
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        path::Path,
        sync::{mpsc, Mutex, OnceLock},
        thread,
    };
    use uuid::Uuid;

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

    fn parser_fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures")
            .join("parser")
            .join(name)
    }

    fn phase4_pdf_fixtures() -> Vec<(String, PathBuf, Value)> {
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let acceptance: Value =
            read_json(&repository_root.join("fixtures/golden/phase4-eight-pdf-acceptance.json"))
                .expect("Phase 4 acceptance manifest must be readable");
        let fixture_ids = acceptance
            .get("fixtureIds")
            .and_then(Value::as_array)
            .expect("Phase 4 acceptance manifest must declare fixtureIds");
        assert_eq!(
            fixture_ids.len(),
            8,
            "Phase 4 acceptance must remain pinned to exactly eight named fixtures"
        );
        let unique_ids = fixture_ids
            .iter()
            .filter_map(Value::as_str)
            .collect::<HashSet<_>>();
        assert_eq!(
            unique_ids.len(),
            fixture_ids.len(),
            "Phase 4 acceptance fixtureIds must be unique strings"
        );

        let golden_manifest: Value =
            read_json(&repository_root.join("fixtures/golden/manifest.json"))
                .expect("golden manifest must be readable");
        let manifest_fixtures = golden_manifest
            .get("fixtures")
            .and_then(Value::as_array)
            .expect("golden manifest must declare fixtures");

        fixture_ids
            .iter()
            .map(|fixture_id| {
                let fixture_id = fixture_id
                    .as_str()
                    .expect("Phase 4 fixtureId must be a string");
                let entry = manifest_fixtures
                    .iter()
                    .find(|entry| {
                        entry.get("fixtureId").and_then(Value::as_str) == Some(fixture_id)
                    })
                    .unwrap_or_else(|| {
                        panic!("Phase 4 fixture {fixture_id} is missing from golden manifest")
                    });
                assert_eq!(
                    entry.get("status").and_then(Value::as_str),
                    Some("available"),
                    "Phase 4 fixture {fixture_id} must be available"
                );
                let source_relative = entry
                    .get("sourcePath")
                    .and_then(Value::as_str)
                    .expect("available fixture must declare sourcePath");
                let metadata_relative = entry
                    .get("metadataPath")
                    .and_then(Value::as_str)
                    .expect("available fixture must declare metadataPath");
                let source_path = repository_root.join(source_relative);
                let metadata: Value = read_json(&repository_root.join(metadata_relative))
                    .unwrap_or_else(|error| {
                        panic!("Phase 4 fixture {fixture_id} metadata failed: {error}")
                    });
                assert_eq!(
                    metadata.get("fixtureId").and_then(Value::as_str),
                    Some(fixture_id),
                    "Phase 4 fixture metadata identity must match the acceptance manifest"
                );
                assert_eq!(
                    metadata.pointer("/source/path").and_then(Value::as_str),
                    Some(source_relative),
                    "Phase 4 fixture metadata source path must match the golden manifest"
                );
                let (actual_hash, actual_size, _) = hash_file_or_path(&source_path)
                    .unwrap_or_else(|error| panic!("Phase 4 fixture {fixture_id} failed: {error}"));
                assert_eq!(
                    entry.get("sha256").and_then(Value::as_str),
                    Some(actual_hash.as_str()),
                    "Phase 4 fixture {fixture_id} source hash drifted"
                );
                assert_eq!(
                    entry.get("sizeBytes").and_then(Value::as_u64),
                    Some(actual_size),
                    "Phase 4 fixture {fixture_id} source size drifted"
                );
                (fixture_id.to_string(), source_path, metadata)
            })
            .collect()
    }

    fn assert_pdf_parser_provenance(document: &Value, fixture: &Path) {
        let provider = document
            .pointer("/parser/provider")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let warnings = parser_warnings(Some(document));
        let fallback_warning = warnings.iter().find(|warning| {
            warning.contains("rust pdf-extract failed; used Python parser fallback:")
        });
        match provider {
            "rust-parser:pdf:pdfium" => {
                let fallback_warning = warnings.iter().find(|warning| {
                    warning.contains("rust pdf-extract failed; used Python parser fallback:")
                });
                assert!(
                    fallback_warning.is_none(),
                    "{} reports native pdfium parsing and must not claim sidecar fallback",
                    fixture.display()
                );
            }
            "rust-parser:pdf:pdf-extract" => assert!(
                fallback_warning.is_none(),
                "{} reports native Rust parsing and must not claim sidecar fallback",
                fixture.display()
            ),
            "python-parser-sidecar:pdf:pypdf" => assert!(
                fallback_warning.is_some_and(|warning| warning.contains("rust_pdf_extract_failed:")),
                "{} may use the Python sidecar only with the original Rust parser failure preserved",
                fixture.display()
            ),
            other => panic!(
                "{} used unsupported PDF parser provider {other:?}",
                fixture.display()
            ),
        }
    }

    fn write_minimal_docx(path: &Path, document_xml: &str) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("[Content_Types].xml", options).unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
        zip.add_directory("word/", options).unwrap();
        zip.start_file("word/document.xml", options).unwrap();
        zip.write_all(document_xml.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    fn read_mock_http_request(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        let mut content_length = None::<usize>;
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if content_length.is_none() {
                if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let header = String::from_utf8_lossy(&bytes[..header_end]).to_string();
                    content_length = header.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    });
                }
            }
            if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let body_start = header_end + 4;
                let expected = body_start + content_length.unwrap_or(0);
                if bytes.len() >= expected {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&bytes).to_string()
    }

    fn mock_openai_server(
        content: Value,
    ) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_mock_http_request(&mut stream);
            sender.send(request).unwrap();
            let response_body = json!({
                "choices": [{"message": {"content": serde_json::to_string(&content).unwrap()}}],
                "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });
        (format!("http://{}/v1", address), receiver, handle)
    }

    fn tiny_png_bytes() -> Vec<u8> {
        vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31,
            0, 3, 3, 2, 0, 239, 191, 167, 219, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ]
    }

    fn cached_llm_inputs(root: &Path, job_id: &str) -> Vec<String> {
        let dir = job_dir(root, job_id).join("cache").join("llm");
        if !dir.exists() {
            return Vec::new();
        }
        fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("-input-"))
            .map(|entry| fs::read_to_string(entry.path()).unwrap())
            .collect::<Vec<_>>()
    }

    fn live_llm_profile_from_env() -> Option<(Value, String)> {
        let base_url = env::var("EPIC8_LIVE_LLM_BASE_URL").ok()?;
        let api_key = env::var("EPIC8_LIVE_LLM_API_KEY").ok()?;
        let model = env::var("EPIC8_LIVE_LLM_MODEL").ok()?;
        Some((
            json!({
                "profileId": "profile-live-provider",
                "provider": "OpenAiCompatible",
                "baseUrl": base_url,
                "model": model,
                "temperature": 0,
                "timeoutMs": 120000,
                "forceJson": true
            }),
            api_key,
        ))
    }

    fn attach_fixture_source(root: &Path, job: &mut ImportJob, fixture_name: &str, role: &str) {
        let fixture = parser_fixture(fixture_name);
        let original_name = fixture
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap()
            .to_string();
        let (hash, size, bytes) = hash_file_or_path(&fixture).unwrap();
        let stored_name = format!("{}-{}", &hash[..8], sanitize_filename(&original_name));
        ensure_job_dirs(&job_dir(root, &job.job_id)).unwrap();
        write_bytes(
            &job_dir(root, &job.job_id)
                .join("uploads")
                .join(&stored_name),
            &bytes.unwrap(),
        )
        .unwrap();
        job.source_files.push(SourceFile {
            file_id: format!("file-{}", Uuid::new_v4().simple()),
            original_name,
            stored_name,
            file_type: file_type_from_name(fixture_name).to_string(),
            sha256: hash,
            size_bytes: size,
            role: role.to_string(),
            imported_at: Utc::now(),
        });
    }

    fn env_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
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
        write_text(
            &job_dir(root, &job.job_id)
                .join("uploads")
                .join("publishable.pdf"),
            "original source bytes",
        )
        .unwrap();

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

    fn write_nas_source_fixture(source_dir: &Path, file_name: &str, source: &Value) {
        fs::create_dir_all(source_dir).unwrap();
        let wrapper = build_wrapper(source).unwrap();
        write_text(&source_dir.join(file_name), &wrapper).unwrap();
    }

    fn write_publishable_nas_source(root: &Path, job: &ImportJob, ir: &Value, source_dir: &Path) {
        fs::create_dir_all(source_dir).unwrap();
        let mut source = reading_source(ir);
        let pdf_path = job_dir(root, &job.job_id)
            .join("uploads")
            .join("publishable.pdf");
        write_source_payload_file(source_dir, &mut source, Some(&pdf_path)).unwrap();
    }

    fn report_has_error_code(report: &Value, code: &str) -> bool {
        report
            .get("errors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|item| item.get("code").and_then(Value::as_str) == Some(code))
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

    fn clear_all_authoring_answers(ir: &mut Value) -> Vec<String> {
        let mut question_ids = Vec::new();
        if let Some(groups) = ir.get_mut("groups").and_then(Value::as_array_mut) {
            for question in groups.iter_mut().flat_map(|group| {
                group
                    .get_mut("questions")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
            }) {
                if let Some(qid) = question.get("id").and_then(Value::as_str) {
                    question_ids.push(qid.to_string());
                }
                if let Some(obj) = question.as_object_mut() {
                    obj.insert("answer".to_string(), Value::String(String::new()));
                }
            }
        }
        let needs_review = refresh_authoring_review_state(ir);
        assert_eq!(needs_review, 0, "empty answers must not require review");
        let answer_key = crate::reading_source::answer_key_from_authoring(ir);
        ir.as_object_mut()
            .unwrap()
            .insert("answerKey".to_string(), answer_key);
        question_ids
    }

    #[test]
    fn unsafe_path_segments_are_rejected() {
        assert!(is_safe_path_segment("import-20260531120000-abcdef12"));
        assert!(is_safe_path_segment("exam.fixture-01"));

        for value in [
            "",
            ".",
            "..",
            "../evil",
            "nested/path",
            "evil\\path",
            "bad id",
        ] {
            assert!(
                !is_safe_path_segment(value),
                "unsafe segment unexpectedly accepted: {value}"
            );
            assert!(validate_path_segment("test", value).is_err());
        }
    }

    #[test]
    fn job_path_helpers_reject_traversal_ids() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();

        assert!(load_job(&root, "../outside").is_err());
        assert!(safe_job_dir(&root, "../outside").is_err());
        assert_eq!(
            job_dir(&root, "../outside"),
            root.join("jobs").join("__invalid_job_id__")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn secret_file_helpers_reject_unsafe_profile_ids() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();

        assert!(file_save_secret(&root, "../profile", "sk-nope").is_err());
        assert_eq!(file_load_secret(&root, "../profile"), None);
        assert!(!root.join("config").join("profile.key").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn wrapper_and_manifest_reject_unsafe_exam_ids() {
        let source = json!({
            "schemaVersion": "ReadingExamSourceV1",
            "examId": "../evil",
            "meta": {"title": "Unsafe", "category": "P1"},
            "questionGroups": [],
            "answerKey": {},
            "questionOrder": [],
            "questionDisplayMap": {}
        });

        assert!(safe_exam_id(&source).is_err());
        assert!(build_wrapper(&source).is_err());
        assert!(build_manifest(&[source]).is_err());
    }

    #[test]
    fn llm_cache_input_redacts_api_key() {
        let input = json!({
            "apiKey": "sk-secret-value",
            "profile": {"profileId": "profile-test", "model": "gpt-test"},
            "group": {"groupId": "group-test"}
        });

        let redacted = llm_gateway::redact_llm_input_for_cache(&input);
        let serialized = serde_json::to_string(&redacted).unwrap();

        assert!(!serialized.contains("sk-secret-value"));
        assert!(redacted.get("apiKey").is_none());
        assert_eq!(
            redacted.get("apiKeySource").and_then(Value::as_str),
            Some("process-env")
        );
    }

    #[test]
    fn environment_preflight_reports_required_dependency_names() {
        let report = environment_preflight_report();
        assert_eq!(
            report.get("schemaVersion").and_then(Value::as_str),
            Some("EnvironmentPreflightV1")
        );
        let checks = report
            .get("checks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let names = checks
            .iter()
            .filter_map(|check| check.get("name").and_then(Value::as_str))
            .collect::<std::collections::HashSet<_>>();

        for required in [
            "node",
            "rust:text-parser",
            "rust:pdf-extract",
            "rust:docx-ooxml",
            "python",
            "python:pypdf",
            "renderer:pdf-page-renderer",
            "ocr:local",
            "vision:cloud",
            "sidecar:python-parser",
            "sidecar:llm-gateway",
            "sidecar:node-validator",
            "sidecar:preview-e2e",
            "runtime:unified-html",
            "runtime:unified-python",
            "runtime:strict-gate",
            "security:plaintext-secret-fallback",
        ] {
            assert!(
                names.contains(required),
                "missing preflight check: {}",
                required
            );
        }
        if cfg!(target_os = "macos") {
            assert!(
                names.contains("renderer:macos-sips"),
                "missing preflight check: renderer:macos-sips"
            );
        }
        if cfg!(target_os = "windows") {
            assert!(
                names.contains("renderer:windows-pdfium"),
                "missing preflight check: renderer:windows-pdfium"
            );
        }
    }

    #[test]
    fn plaintext_secret_fallback_is_disabled_by_default() {
        let _guard = env_test_lock().lock().unwrap();
        env::remove_var("EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK");
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        file_save_secret(&root, "profile-plain", "sk-plain").unwrap();

        assert!(!plaintext_secret_fallback_allowed());
        assert_eq!(load_profile_secret(&root, "profile-plain"), None);
        let redacted = redact_profile_for_ui(
            &root,
            json!({"profileId":"profile-plain","name":"Plain","provider":"OpenAiCompatible"}),
        );
        assert_eq!(
            redacted.get("secretStorageBackend").and_then(Value::as_str),
            Some("none")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn plaintext_secret_fallback_requires_explicit_opt_in() {
        let _guard = env_test_lock().lock().unwrap();
        env::set_var("EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK", "1");
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        file_save_secret(&root, "profile-plain", "sk-plain").unwrap();

        assert!(plaintext_secret_fallback_allowed());
        assert_eq!(
            load_profile_secret(&root, "profile-plain"),
            Some("sk-plain".to_string())
        );
        let redacted = redact_profile_for_ui(
            &root,
            json!({"profileId":"profile-plain","name":"Plain","provider":"OpenAiCompatible"}),
        );
        assert_eq!(
            redacted.get("secretStorageBackend").and_then(Value::as_str),
            Some("file")
        );

        env::remove_var("EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK");
        let _ = fs::remove_dir_all(root);
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
    fn make_llm_input_carries_structured_repair_context_and_evidence() {
        let job = test_job();
        let profile = json!({
            "profileId": "profile-test",
            "provider": "OpenAiCompatible",
            "baseUrl": "https://example.invalid/v1",
            "model": "gpt-test"
        });
        let group = json!({
            "groupId": "group-test",
            "kind": "matching",
            "layout": {"template": "matching_list"},
            "sourceBlockIds": ["b-heading", "b-options"],
            "reviewWarnings": ["Option reuse was inferred from question type; source wording did not state it explicitly."],
            "classificationEvidence": ["b-heading"],
            "sectionEvidence": [{
                "blockId": "b-options",
                "pageIndex": 2,
                "column": 1,
                "textPreview": "A option",
                "tableRows": 3,
                "tableCols": 2,
                "headingLevel": 2,
                "numberingLevel": 0,
                "normalizedBbox": [10, 20, 120, 240],
                "pageRotation": 90
            }],
            "continuationEdges": [{"fromBlockId": "b-heading", "toBlockId": "b-options", "reason": "cross-page-continuation", "confidence": 0.72}],
            "questions": []
        });

        let input = make_llm_input(&profile, &job, &group, "profile-test", "extract_group");

        assert_eq!(
            input
                .pointer("/repairContract/schema")
                .and_then(Value::as_str),
            Some("Epic8LlmGroupRepairV1")
        );
        assert_eq!(
            input
                .pointer("/repairContext/sectionEvidence/0/tableRows")
                .and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            input
                .pointer("/repairContext/sectionEvidence/0/pageRotation")
                .and_then(Value::as_u64),
            Some(90)
        );
        assert_eq!(
            input
                .pointer("/repairContext/continuationEdges/0/reason")
                .and_then(Value::as_str),
            Some("cross-page-continuation")
        );
        assert!(serde_json::to_string(&input)
            .unwrap()
            .contains("allowedPatchPaths"));
    }

    #[test]
    fn rust_llm_gateway_hits_mock_openai_compatible_chat_completion() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let (base_url, request_receiver, handle) = mock_openai_server(json!({
            "kind": "short_answer",
            "confidence": 0.91,
            "patch": [],
            "questions": [],
            "warnings": [],
            "evidence": {
                "sourceBlockIds": ["b-question"],
                "quotes": [{"blockId": "b-question", "text": "Questions 1-2"}]
            }
        }));
        let input = json!({
            "profile": {
                "profileId": "profile-mock",
                "provider": "OpenAiCompatible",
                "baseUrl": base_url,
                "model": "mock-json-model",
                "temperature": 0,
                "timeoutMs": 10000,
                "forceJson": true
            },
            "group": {
                "groupId": "group-1",
                "kind": "short_answer",
                "sourceBlockIds": ["b-question"],
                "instruction": ["Questions 1-2"],
                "questions": []
            }
        });

        let output = llm_gateway::run_llm_gateway(
            &root,
            "job-mock-llm",
            "extract_group",
            &input,
            Some("sk-mock-secret"),
        )
        .unwrap();
        let request = request_receiver.recv().unwrap();
        handle.join().unwrap();

        assert!(request.starts_with("POST /v1/chat/completions "));
        assert!(request.contains("authorization: Bearer sk-mock-secret"));
        assert!(request.contains("\"response_format\":{\"type\":\"json_object\"}"));
        assert_eq!(
            output.get("kind").and_then(Value::as_str),
            Some("short_answer")
        );
        assert_eq!(
            output.pointer("/evidence/source").and_then(Value::as_str),
            Some("openai-compatible-rust")
        );
        assert_eq!(
            output.pointer("/evidence/model").and_then(Value::as_str),
            Some("mock-json-model")
        );

        let cached_inputs = cached_llm_inputs(&root, "job-mock-llm");
        assert!(!cached_inputs.is_empty());
        assert!(!cached_inputs.join("\n").contains("sk-mock-secret"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "requires EPIC8_LIVE_LLM_BASE_URL, EPIC8_LIVE_LLM_API_KEY, and EPIC8_LIVE_LLM_MODEL"]
    fn live_rust_llm_gateway_provider_smoke_text_and_vision() {
        let Some((profile, api_key)) = live_llm_profile_from_env() else {
            eprintln!(
                "skipping: set EPIC8_LIVE_LLM_BASE_URL, EPIC8_LIVE_LLM_API_KEY, and EPIC8_LIVE_LLM_MODEL"
            );
            return;
        };
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();

        let text_input = json!({
            "profile": profile,
            "group": {
                "groupId": "group-live",
                "kind": "short_answer",
                "sourceBlockIds": ["b-live"],
                "instruction": ["Return JSON only."],
                "questions": []
            }
        });
        let text_output = llm_gateway::run_llm_gateway(
            &root,
            "job-live-provider-text",
            "extract_group",
            &text_input,
            Some(&api_key),
        )
        .unwrap();
        assert!(allowed_question_kind(
            text_output
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ));
        assert_eq!(
            text_output
                .pointer("/evidence/source")
                .and_then(Value::as_str),
            Some("openai-compatible-rust")
        );

        let image_path = root.join("live-vision-fixture.png");
        write_bytes(&image_path, &tiny_png_bytes()).unwrap();
        let vision_input = json!({
            "profile": text_input.get("profile").cloned().unwrap(),
            "job": {"jobId": "job-live-provider-vision", "title": "Live Vision Smoke"},
            "pages": [{
                "pageIndex": 1,
                "images": [{
                    "assetId": "live-page-1",
                    "path": image_path.to_string_lossy(),
                    "mimeType": "image/png"
                }]
            }]
        });
        let vision_output = llm_gateway::run_llm_gateway(
            &root,
            "job-live-provider-vision",
            "transcribe_pdf_images",
            &vision_input,
            Some(&api_key),
        )
        .unwrap();
        assert!(vision_output
            .get("text")
            .map(Value::is_string)
            .unwrap_or(false));
        assert_eq!(
            vision_output
                .pointer("/evidence/source")
                .and_then(Value::as_str),
            Some("openai-compatible-vision-rust")
        );

        let cached = [
            cached_llm_inputs(&root, "job-live-provider-text").join("\n"),
            cached_llm_inputs(&root, "job-live-provider-vision").join("\n"),
        ]
        .join("\n");
        assert!(!cached.contains(&api_key));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "requires EPIC8_LIVE_LLM_BASE_URL, EPIC8_LIVE_LLM_API_KEY, and EPIC8_LIVE_LLM_MODEL; calls live provider for Files/*.pdf groups"]
    fn live_llm_repair_contract_on_files_pdf_samples() {
        let Some((profile, api_key)) = live_llm_profile_from_env() else {
            eprintln!(
                "skipping: set EPIC8_LIVE_LLM_BASE_URL, EPIC8_LIVE_LLM_API_KEY, and EPIC8_LIVE_LLM_MODEL"
            );
            return;
        };
        let sample_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("Files");
        let mut samples = fs::read_dir(&sample_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        samples.sort();
        if samples.is_empty() {
            eprintln!(
                "skipping files_pdf_samples_auto_pipeline_minimizes_artifacts_and_preserves_review_gate: Files/ contains no PDF samples"
            );
            return;
        }
        assert_eq!(samples.len(), 4);

        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let mut diagnostics = Vec::<Value>::new();
        let mut checked_groups = 0usize;
        let mut high_confidence_count = 0usize;
        let mut auto_applicable_count = 0usize;
        let mut low_confidence_count = 0usize;
        let mut blocked_high_confidence = Vec::<Value>::new();
        let mut manual_scaffold_samples = Vec::<String>::new();

        for (sample_index, sample) in samples.iter().enumerate() {
            let original_name = sample
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap()
                .to_string();
            let mut job = make_job(CreateJobInput {
                title: Some(
                    sample
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("PDF sample")
                        .to_string(),
                ),
                category: Some("P2".to_string()),
                frequency: Some("medium".to_string()),
                tags: Some(vec!["live-llm-files-pdf".to_string()]),
                llm_profile_id: None,
            });
            let source = SourceFile {
                file_id: format!("file-live-sample-{}", sample_index + 1),
                original_name: original_name.clone(),
                stored_name: original_name.clone(),
                file_type: "pdf".to_string(),
                sha256: "3".repeat(64),
                size_bytes: sample.metadata().unwrap().len(),
                role: "MainQuestion".to_string(),
                imported_at: Utc::now(),
            };
            job.source_files = vec![source.clone()];
            save_job(&root, &job).unwrap();
            ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();

            let parser_output = root
                .join("cache")
                .join("parser")
                .join(format!("{}-live-document-ir.json", job.job_id));
            let doc = parse_source_document(&job, &source, sample, &parser_output, "auto")
                .unwrap_or_else(|error| panic!("{} parse failed: {}", original_name, error));
            let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
            let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
            let groups = ir
                .get("groups")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let selected_groups = groups
                .iter()
                .filter(|group| {
                    group
                        .get("requiresManualQuestionImport")
                        .and_then(Value::as_bool)
                        != Some(true)
                })
                .take(2)
                .cloned()
                .collect::<Vec<_>>();
            if selected_groups.is_empty() {
                manual_scaffold_samples.push(original_name);
                continue;
            }

            for group in selected_groups {
                checked_groups += 1;
                let group_id = group
                    .get("groupId")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let input = make_llm_input(
                    &profile,
                    &job,
                    &group,
                    "profile-live-provider",
                    "extract_group",
                );
                let output = llm_gateway::run_llm_gateway(
                    &root,
                    &job.job_id,
                    "extract_group",
                    &input,
                    Some(&api_key),
                )
                .unwrap_or_else(|error| {
                    panic!("{} {} live LLM failed: {}", original_name, group_id, error)
                });
                assert!(allowed_question_kind(
                    output
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                ));
                assert!(output.get("patch").map(Value::is_array).unwrap_or(false));
                assert!(output
                    .get("questions")
                    .map(Value::is_array)
                    .unwrap_or(false));
                assert!(output.get("warnings").map(Value::is_array).unwrap_or(false));
                assert_eq!(
                    output.pointer("/evidence/source").and_then(Value::as_str),
                    Some("openai-compatible-rust")
                );

                let suggestion = json!({
                    "suggestionId": format!("live-suggestion-{}", Uuid::new_v4().simple()),
                    "jobId": job.job_id,
                    "groupId": group_id,
                    "profileId": "profile-live-provider",
                    "kind": output.get("kind").cloned().unwrap_or_else(|| json!("short_answer")),
                    "confidence": output.get("confidence").cloned().unwrap_or_else(|| json!(0.0)),
                    "patch": output.get("patch").cloned().unwrap_or_else(|| json!([])),
                    "questions": output.get("questions").cloned().unwrap_or_else(|| json!([])),
                    "warnings": output.get("warnings").cloned().unwrap_or_else(|| json!([])),
                    "evidence": output.get("evidence").cloned().unwrap_or_else(|| json!({})),
                    "createdAt": Utc::now().to_rfc3339()
                });
                let issues = llm_suggestion_auto_apply_issues(
                    &ir,
                    &suggestion,
                    &[
                        "kind".to_string(),
                        "layout".to_string(),
                        "questions".to_string(),
                    ],
                );
                let confidence = suggestion
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                if confidence >= 0.85 {
                    high_confidence_count += 1;
                    if issues.is_empty() {
                        auto_applicable_count += 1;
                    } else {
                        blocked_high_confidence.push(json!({
                            "sample": original_name,
                            "groupId": suggestion.get("groupId").cloned().unwrap_or(Value::Null),
                            "kind": suggestion.get("kind").cloned().unwrap_or(Value::Null),
                            "confidence": confidence,
                            "issues": issues
                        }));
                    }
                } else {
                    low_confidence_count += 1;
                }
                diagnostics.push(json!({
                    "sample": original_name,
                    "groupId": suggestion.get("groupId").cloned().unwrap_or(Value::Null),
                    "kind": suggestion.get("kind").cloned().unwrap_or(Value::Null),
                    "confidence": confidence,
                    "autoApplyIssueCount": issues.len(),
                    "issues": issues,
                    "evidenceBlockCount": suggestion.pointer("/evidence/sourceBlockIds").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
                    "quoteCount": suggestion.pointer("/evidence/quotes").and_then(Value::as_array).map(Vec::len).unwrap_or(0)
                }));
            }
        }

        assert!(checked_groups >= 4);
        assert!(
            high_confidence_count + low_confidence_count == checked_groups,
            "all checked groups should be counted"
        );
        assert!(
            auto_applicable_count > 0
                || !blocked_high_confidence.is_empty()
                || low_confidence_count > 0,
            "live provider diagnostic should classify at least one output path"
        );
        if !blocked_high_confidence.is_empty() {
            eprintln!(
                "blocked high-confidence live outputs: {}",
                serde_json::to_string_pretty(&blocked_high_confidence).unwrap()
            );
        }
        eprintln!(
            "live Files PDF LLM diagnostics: {}",
            serde_json::to_string_pretty(&json!({
                "checkedGroups": checked_groups,
                "highConfidence": high_confidence_count,
                "autoApplicable": auto_applicable_count,
                "lowConfidence": low_confidence_count,
                "diagnostics": diagnostics,
                "manualScaffoldSamples": manual_scaffold_samples
            }))
            .unwrap()
        );

        let cached = fs::read_dir(root.join("jobs"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| {
                cached_llm_inputs(&root, entry.file_name().to_string_lossy().as_ref()).join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!cached.contains(&api_key));

        let _ = fs::remove_dir_all(root);
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
    fn llm_auto_apply_accepts_matching_interaction_type() {
        let job = test_job();
        let group = json!({
            "groupId": "group-matching",
            "kind": "matching_information",
            "sourceBlockIds": ["b1"],
            "questions": [{
                "id": "q14",
                "displayNumber": "14",
                "prompt": "Which paragraph contains information about diet?",
                "interaction": {"type": "matching", "options": ["A", "B"], "allowOptionReuse": true},
                "sourceBlockIds": ["b1"],
                "confidence": 0.8,
                "verified": false
            }]
        });
        let ir = json!({
            "schemaVersion": "ReadingAuthoringIRV1",
            "jobId": job.job_id,
            "groups": [group],
            "audit": {"humanVerified": false}
        });
        let suggestion = json!({
            "suggestionId": "suggestion-matching",
            "jobId": job.job_id,
            "groupId": "group-matching",
            "profileId": "profile-test",
            "kind": "matching_information",
            "confidence": 0.95,
            "patch": [
                {"op":"replace","path":"/kind","value":"matching_information"},
                {"op":"replace","path":"/layout/template","value":"matching_information"}
            ],
            "questions": [{
                "id": "q14",
                "prompt": "Which paragraph contains information about diet?",
                "interaction": {"type": "matching", "options": ["A", "B"], "allowOptionReuse": true}
            }],
            "warnings": [],
            "evidence": {
                "source": "openai-compatible-rust",
                "sourceBlockIds": ["b1"],
                "quotes": [{"blockId": "b1", "text": "diet"}]
            }
        });
        let issues = llm_suggestion_auto_apply_issues(
            &ir,
            &suggestion,
            &[
                "kind".to_string(),
                "layout".to_string(),
                "questions".to_string(),
            ],
        );
        assert!(issues.is_empty(), "unexpected issues: {:?}", issues);
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
                // Confidence verification alone is not enough: empty prompts and
                // incomplete choice sets remain hard review debt on the default path.
                if question
                    .get("prompt")
                    .and_then(Value::as_str)
                    .map(|prompt| prompt.trim().is_empty())
                    .unwrap_or(true)
                {
                    if let Some(obj) = question.as_object_mut() {
                        obj.insert("prompt".to_string(), json!("Author-completed prompt"));
                    }
                }
                let interaction_type = question
                    .pointer("/interaction/type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if matches!(
                    interaction_type.as_str(),
                    "radio" | "checkbox" | "single_choice" | "multi_choice"
                ) {
                    let option_count = question
                        .pointer("/interaction/options")
                        .and_then(Value::as_array)
                        .map(|items| items.len())
                        .unwrap_or(0);
                    if option_count < 2 {
                        if let Some(interaction) = question
                            .get_mut("interaction")
                            .and_then(Value::as_object_mut)
                        {
                            interaction.insert("options".to_string(), json!(["A", "B", "C", "D"]));
                            interaction.insert(
                                "optionTexts".to_string(),
                                json!({
                                    "A": "Option A",
                                    "B": "Option B",
                                    "C": "Option C",
                                    "D": "Option D"
                                }),
                            );
                        }
                    }
                }
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
        assert!(
            authoring_review_issues(&ir).is_empty(),
            "completed + verified IR should clear hard review gates: {:?}",
            authoring_review_issues(&ir)
        );
    }

    #[test]
    fn publish_review_issues_require_low_confidence_verification() {
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
    fn authoring_review_issues_block_empty_prompt_even_when_verified() {
        let ir = json!({
            "schemaVersion": "ReadingAuthoringIRV1",
            "jobId": "job-empty-prompt",
            "groups": [{
                "groupId": "group-1",
                "kind": "short_answer",
                "confidence": 0.95,
                "verified": true,
                "questions": [{
                    "id": "q1",
                    "displayNumber": "1",
                    "prompt": "   ",
                    "interaction": {"type": "short_answer"},
                    "confidence": 0.95,
                    "verified": true
                }]
            }],
            "audit": {"humanVerified": true}
        });

        let issues = authoring_review_issues(&ir);
        assert!(
            issues.iter().any(|issue| issue
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("Question prompt is empty and must be completed before publish")),
            "expected empty-prompt hard gate, got {:?}",
            issues
        );
    }

    #[test]
    fn authoring_review_issues_block_incomplete_choice_options() {
        let ir = json!({
            "schemaVersion": "ReadingAuthoringIRV1",
            "jobId": "job-incomplete-choice",
            "groups": [{
                "groupId": "group-sc",
                "kind": "single_choice",
                "confidence": 0.95,
                "verified": true,
                "questions": [{
                    "id": "q5",
                    "displayNumber": "5",
                    "prompt": "Choose the best answer.",
                    "interaction": {
                        "type": "radio",
                        "options": ["A"],
                        "optionTexts": {"A": "Only one recovered option"}
                    },
                    "confidence": 0.95,
                    "verified": true
                }]
            }],
            "audit": {"humanVerified": true}
        });

        let issues = authoring_review_issues(&ir);
        assert!(
            issues.iter().any(|issue| issue
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("at least two complete options")),
            "expected incomplete-option hard gate, got {:?}",
            issues
        );
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
    fn source_review_status_preserves_v1_json_contract() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let job = test_job();
        save_job(&root, &job).unwrap();
        let source = test_source("pdf");
        let doc = parser_failure_document_ir(&job, &source, "auto", "boom");

        let review = source_review_status(&root, &job.job_id, Some(&doc)).unwrap();
        assert_eq!(
            review.get("schemaVersion").and_then(Value::as_str),
            Some("SourceReviewV1")
        );
        assert_eq!(
            review.get("jobId").and_then(Value::as_str),
            Some(job.job_id.as_str())
        );
        assert_eq!(review.get("required").and_then(Value::as_bool), Some(true));
        assert_eq!(review.get("resolved").and_then(Value::as_bool), Some(false));
        assert_eq!(review.get("stale").and_then(Value::as_bool), Some(false));
        assert!(review.get("fingerprint").and_then(Value::as_str).is_some());
        assert!(review
            .get("parserWarnings")
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));
        assert!(review
            .get("lowConfidenceBlocks")
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));
        assert_eq!(review.get("resolvedAt"), Some(&Value::Null));
        assert_eq!(review.get("note"), Some(&Value::Null));

        let resolved = write_source_review_status(
            &root,
            &job.job_id,
            Some(&doc),
            true,
            Some("checked".into()),
        )
        .unwrap();
        assert_eq!(
            resolved.get("resolved").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            resolved.get("note").and_then(Value::as_str),
            Some("checked")
        );
        assert!(resolved.get("resolvedAt").and_then(Value::as_str).is_some());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_review_resolution_survives_minimal_state_without_document_ir() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let job = test_job();
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        let source = test_source("pdf");
        let doc = parser_failure_document_ir(&job, &source, "auto", "boom");
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();
        let unresolved =
            write_source_review_status(&root, &job.job_id, Some(&doc), false, None).unwrap();
        assert!(!source_review_issues(&unresolved).is_empty());
        fs::remove_file(job_dir(&root, &job.job_id).join("document-ir.json")).unwrap();

        let saved = source_review_status_for_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.get("required").and_then(Value::as_bool), Some(true));
        assert!(saved
            .get("parserWarnings")
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));
        let resolved =
            resolve_source_review_status(&root, &job.job_id, Some("checked minimal state".into()))
                .unwrap();
        assert_eq!(
            resolved.get("required").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            resolved.get("resolved").and_then(Value::as_bool),
            Some(true)
        );
        assert!(source_review_issues(&resolved).is_empty());
        assert!(resolved
            .get("parserWarnings")
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));

        let _ = fs::remove_dir_all(root);
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
    fn rust_vision_gateway_sends_image_url_and_parses_transcription() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let image_path = root.join("vision-fixture.png");
        write_bytes(&image_path, &tiny_png_bytes()).unwrap();
        let (base_url, request_receiver, handle) = mock_openai_server(json!({
            "text": "READING PASSAGE 1\nQuestions 1-1\n1 Mock vision text.\nAnswers\n1 TRUE",
            "confidence": 0.87,
            "warnings": []
        }));
        let input = json!({
            "profile": {
                "profileId": "profile-vision-mock",
                "provider": "OpenAiCompatible",
                "baseUrl": base_url,
                "model": "mock-vision-model",
                "temperature": 0,
                "timeoutMs": 10000,
                "forceJson": true
            },
            "job": {"jobId": "job-mock-vision", "title": "Vision Mock"},
            "pages": [{
                "pageIndex": 1,
                "images": [{
                    "assetId": "page-1",
                    "path": image_path.to_string_lossy(),
                    "mimeType": "image/png"
                }]
            }]
        });

        let output = llm_gateway::run_llm_gateway(
            &root,
            "job-mock-vision",
            "transcribe_pdf_images",
            &input,
            Some("sk-vision-secret"),
        )
        .unwrap();
        let request = request_receiver.recv().unwrap();
        handle.join().unwrap();

        assert!(request.starts_with("POST /v1/chat/completions "));
        assert!(request.contains("\"type\":\"image_url\""));
        assert!(request.contains("data:image/png;base64,"));
        assert!(!request.contains(&image_path.to_string_lossy().to_string()));
        assert!(request.contains("authorization: Bearer sk-vision-secret"));
        assert_eq!(
            output.get("text").and_then(Value::as_str),
            Some("READING PASSAGE 1\nQuestions 1-1\n1 Mock vision text.\nAnswers\n1 TRUE")
        );
        assert_eq!(
            output.pointer("/evidence/source").and_then(Value::as_str),
            Some("openai-compatible-vision-rust")
        );
        assert_eq!(
            output.pointer("/evidence/model").and_then(Value::as_str),
            Some("mock-vision-model")
        );

        let cached_inputs = cached_llm_inputs(&root, "job-mock-vision");
        assert!(!cached_inputs.is_empty());
        assert!(!cached_inputs.join("\n").contains("sk-vision-secret"));

        let _ = fs::remove_dir_all(root);
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
        let image_count = image_count_from_extraction(&extraction);

        assert!(image_count > 0);
        assert!(extraction
            .get("warnings")
            .and_then(Value::as_array)
            .map(|warnings| warnings.iter().all(|warning| {
                let text = warning.as_str().unwrap_or_default();
                !text.contains("PDF contains no extractable embedded page images")
                    && !text.contains("manual transcription required")
            }))
            .unwrap_or(false));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn no_text_pdf_fixture_renders_page_fallback_for_vision() {
        let sips = command_probe("sips", &["--version"]);
        if !sips.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            eprintln!("skipping: macOS sips renderer is unavailable");
            return;
        }
        let job = test_job();
        let fixture = parser_fixture("no-text.pdf");
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let output = root
            .join("cache")
            .join("parser")
            .join("rendered-extraction.json");
        let asset_dir = root.join("cache").join("parser").join("rendered-assets");

        let extraction = extract_pdf_images_for_vision(&job.job_id, &fixture, &output, &asset_dir)
            .expect("no-text PDF fixture should render a page image fallback");
        let images = extraction
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
            .collect::<Vec<_>>();

        assert_eq!(
            extraction.get("renderedFallback").and_then(Value::as_bool),
            Some(true)
        );
        assert!(
            !images.is_empty(),
            "no-text PDF fixture should expose at least one rendered page image"
        );
        assert_eq!(
            images[0].get("mimeType").and_then(Value::as_str),
            Some("image/png")
        );
        for image in images {
            assert!(image
                .get("renderedFallback")
                .and_then(Value::as_bool)
                .unwrap_or(false));
            assert!(PathBuf::from(image.get("path").and_then(Value::as_str).unwrap()).exists());
        }
        assert!(extraction
            .get("warnings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|warning| warning
                .as_str()
                .unwrap_or_default()
                .contains("rendered-page fallback")));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pdf_render_adapter_renders_with_macos_sips_without_ocr() {
        let sips = command_probe("sips", &["--version"]);
        if !sips.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            eprintln!("skipping: macOS sips renderer is unavailable");
            return;
        }
        let job = test_job();
        let fixture = parser_fixture("no-text.pdf");
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let output = root
            .join("cache")
            .join("parser")
            .join("rust-sips-extraction.json");
        let asset_dir = root.join("cache").join("parser").join("rust-sips-assets");

        let extraction = render_pdf_pages_with_adapter(
            &job.job_id,
            &fixture,
            &output,
            &asset_dir,
            vec![
                "python PDF image extraction failed; used Rust sips fallback: fixture".to_string(),
            ],
        )
        .expect("Rust sips fallback should render a page image");

        assert_eq!(image_count_from_extraction(&extraction), 1);
        assert_eq!(
            extraction.get("renderedFallback").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            extraction.get("rendererAdapter").and_then(Value::as_str),
            Some("macos-sips")
        );
        assert_eq!(
            extraction.get("ocrPerformed").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            extraction.get("futureAdapter").and_then(Value::as_str),
            Some("pdfium-render-page-renderer")
        );
        assert_eq!(
            extraction
                .pointer("/pages/0/images/0/renderSource")
                .and_then(Value::as_str),
            Some("rust-macos-sips")
        );
        assert!(output.exists());
        assert!(PathBuf::from(
            extraction
                .pointer("/pages/0/images/0/path")
                .and_then(Value::as_str)
                .unwrap()
        )
        .exists());
        assert!(extraction
            .get("warnings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|warning| warning
                .as_str()
                .unwrap_or_default()
                .contains("Python image extraction failed")
                || warning
                    .as_str()
                    .unwrap_or_default()
                    .contains("python PDF image extraction failed")));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pdf_render_adapter_reports_manual_review_when_renderer_disabled() {
        let _guard = env_test_lock().lock().unwrap();
        env::set_var("EPIC8_PDF_RENDERER", "none");
        env::remove_var("EPIC8_ENABLE_LOCAL_OCR");
        env::remove_var("EPIC8_ENABLE_CLOUD_PDF_VISION");
        let job = test_job();
        let fixture = parser_fixture("no-text.pdf");
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let output = root
            .join("cache")
            .join("parser")
            .join("disabled-renderer-extraction.json");
        let asset_dir = root
            .join("cache")
            .join("parser")
            .join("disabled-renderer-assets");

        let extraction = render_pdf_pages_with_adapter(
            &job.job_id,
            &fixture,
            &output,
            &asset_dir,
            vec!["prior extraction warning".to_string()],
        )
        .expect("disabled renderer should return structured manual-review extraction");

        assert_eq!(image_count_from_extraction(&extraction), 0);
        assert_eq!(
            extraction.get("schemaVersion").and_then(Value::as_str),
            Some("PdfImageExtractionV1")
        );
        assert_eq!(
            extraction.get("failureReason").and_then(Value::as_str),
            Some("renderer_disabled")
        );
        assert_eq!(
            extraction
                .get("requiresManualReview")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            extraction.get("ocrPerformed").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            extraction.get("renderedPageCount").and_then(Value::as_u64),
            Some(0)
        );
        assert!(output.exists());

        env::remove_var("EPIC8_PDF_RENDERER");
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

    fn contract_fixture_source() -> Value {
        json!({
            "schemaVersion": "ReadingExamSourceV1",
            "examId": "contract-fixture",
            "meta": {
                "title": "Contract Fixture",
                "category": "P1",
                "frequency": "medium",
                "pdfFilename": "fixture.pdf",
                "legacyPath": "",
                "legacyFilename": "",
                "questionIntroHtml": "<h3>Questions</h3>"
            },
            "passage": {
                "blocks": [{"blockId": "passage-main", "kind": "html", "html": "<p>Passage</p>"}]
            },
            "questionGroups": [{
                "groupId": "group-1",
                "kind": "short_answer",
                "questionIds": ["q1", "q2"],
                "bodyHtml": "<input id=\"q1_input\"><input name=\"q2\">",
                "leadHtml": "",
                "allowOptionReuse": null
            }],
            "answerKey": {"q1": "A", "q2": "B"},
            "sourceRefs": {
                "primaryHtml": "author-imports/job/intermediate.html",
                "primaryProvider": "author_web",
                "shuiHtml": null,
                "shuiPdf": "uploads/fixture.pdf",
                "ieltsHtml": null
            },
            "audit": {
                "matchStatus": "needs_review",
                "matchConfidence": 0.0,
                "verifiedAt": null,
                "notes": "fixture"
            },
            "questionOrder": ["q1", "q2"],
            "questionDisplayMap": {"q1": "1", "q2": "2"}
        })
    }

    fn contract_messages(source: &Value) -> String {
        validator::validate_reading_source_contract(source)
            .iter()
            .filter_map(|issue| issue.get("message").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn rust_contract_validator_accepts_valid_source_fixture() {
        let source = contract_fixture_source();
        let issues = validator::validate_reading_source_contract(&source);

        assert!(issues.is_empty(), "unexpected issues: {:?}", issues);
    }

    #[test]
    fn rust_contract_validator_accepts_specialized_matching_group_kinds() {
        let mut source = contract_fixture_source();
        for kind in ["matching", "heading_matching", "matching_information"] {
            source["questionGroups"][0]["kind"] = json!(kind);
            source["questionGroups"][0]["allowOptionReuse"] = json!(kind == "matching_information");
            let issues = validator::validate_reading_source_contract(&source);
            assert!(
                issues.is_empty(),
                "{} should be accepted: {:?}",
                kind,
                issues
            );
        }
    }

    #[test]
    fn rust_contract_validator_rejects_unsupported_group_kind() {
        let mut source = contract_fixture_source();
        source["questionGroups"][0]["kind"] = json!("made_up_kind");

        let messages = contract_messages(&source);

        assert!(messages.contains("made_up_kind is not an allowed group kind"));
    }

    #[test]
    fn rust_contract_validator_warns_for_missing_answer_key_coverage() {
        let mut source = contract_fixture_source();
        source["answerKey"].as_object_mut().unwrap().remove("q2");

        let issues = validator::validate_reading_source_contract(&source);
        let warning = issues
            .iter()
            .find(|issue| issue.get("path").and_then(Value::as_str) == Some("$.answerKey.q2"))
            .expect("missing answer warning");

        assert_eq!(
            warning.get("severity").and_then(Value::as_str),
            Some("warning")
        );
        assert!(!validator::has_error_issues(&issues));
    }

    #[test]
    fn rust_contract_validator_rejects_uncovered_answer_key_qid() {
        let mut source = contract_fixture_source();
        source["answerKey"]["q99"] = json!("C");

        let messages = contract_messages(&source);

        assert!(messages.contains("q99 from answerKey is not covered by any question group"));
    }

    #[test]
    fn rust_contract_validator_rejects_missing_question_order_qid() {
        let mut source = contract_fixture_source();
        source["questionOrder"] = json!(["q1", "q99"]);

        let messages = contract_messages(&source);

        assert!(messages.contains("q2 is missing from questionOrder"));
        assert!(messages.contains("q99 is not covered by any question group"));
    }

    #[test]
    fn rust_contract_validator_rejects_question_order_length_mismatch() {
        let mut source = contract_fixture_source();
        source["questionOrder"] = json!(["q1"]);

        let messages = contract_messages(&source);

        assert!(messages.contains("questionOrder length must equal covered question count"));
        assert!(messages.contains("q2 is missing from questionOrder"));
    }

    #[test]
    fn rust_contract_validator_rejects_missing_display_map_entry() {
        let mut source = contract_fixture_source();
        source["questionDisplayMap"]
            .as_object_mut()
            .unwrap()
            .remove("q2");

        let messages = contract_messages(&source);

        assert!(messages.contains("q2 is missing original display number"));
    }

    #[test]
    fn reading_source_v1_preserves_export_contract() {
        let mut job = test_job();
        job.source_files = vec![test_source("pdf")];
        let doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let mut ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        verify_all_authoring_items(&mut ir);

        let source = reading_source(&ir);

        assert_eq!(
            source.get("schemaVersion").and_then(Value::as_str),
            Some("ReadingExamSourceV1")
        );
        assert_eq!(
            source.get("examId").and_then(Value::as_str),
            Some(ir.pointer("/exam/examId").and_then(Value::as_str).unwrap())
        );
        assert_eq!(
            source
                .pointer("/meta/questionIntroHtml")
                .and_then(Value::as_str),
            Some("<h3>Questions</h3>")
        );
        assert_eq!(
            source
                .pointer("/passage/blocks/0/kind")
                .and_then(Value::as_str),
            Some("html")
        );
        assert!(source
            .pointer("/passage/blocks/0/html")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("READING PASSAGE"));
        assert!(source
            .pointer("/questionGroups/0/questionIds")
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));
        assert_eq!(
            source
                .pointer("/sourceRefs/primaryProvider")
                .and_then(Value::as_str),
            Some("author_web")
        );
        assert_eq!(
            source
                .pointer("/sourceRefs/shuiPdf")
                .and_then(Value::as_str),
            Some("uploads/stored.pdf")
        );
        assert_eq!(
            source.pointer("/audit/matchStatus").and_then(Value::as_str),
            Some("author_verified")
        );
        assert_eq!(
            source
                .pointer("/audit/matchConfidence")
                .and_then(Value::as_f64),
            Some(1.0)
        );
        assert!(source
            .pointer("/audit/verifiedAt")
            .and_then(Value::as_str)
            .is_some());
        assert_eq!(
            source.pointer("/questionOrder/0").and_then(Value::as_str),
            Some("q1")
        );
        assert_eq!(
            source
                .pointer("/questionDisplayMap/q1")
                .and_then(Value::as_str),
            Some("1")
        );

        let messages = contract_messages(&source);
        assert!(
            messages.is_empty(),
            "typed export contract should satisfy validation, got: {}",
            messages
        );
    }

    #[test]
    fn rust_contract_validator_rejects_missing_dom_collectible_control() {
        let mut source = contract_fixture_source();
        source["questionGroups"][0]["bodyHtml"] =
            json!("<input id=\"q1_input\"><p>q2 text only</p>");

        let messages = contract_messages(&source);

        assert!(messages.contains("No collectible control found for q2"));
    }

    #[test]
    fn rust_contract_validator_rejects_invalid_dropzone_without_question_id() {
        let mut source = contract_fixture_source();
        source["questionGroups"][0]["kind"] = json!("summary_completion");
        source["questionGroups"][0]["bodyHtml"] = json!(
            "<span class=\"paragraph-dropzone\"></span><input id=\"q1_input\"><input name=\"q2\">"
        );

        let messages = contract_messages(&source);

        assert!(messages.contains(
            "Dropzone is missing data-question/data-question-id/data-target or id fallback"
        ));
    }

    #[test]
    fn rust_contract_validator_requires_matching_allow_option_reuse() {
        let mut source = contract_fixture_source();
        source["questionGroups"][0]["kind"] = json!("matching");
        source["questionGroups"][0]["allowOptionReuse"] = Value::Null;

        let messages = contract_messages(&source);

        assert!(messages
            .contains("matching/classification groups must explicitly set allowOptionReuse"));
    }

    #[test]
    fn validation_warning_does_not_block_runtime_gate_progress() {
        let job = test_job();
        let doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let mut report = validate_authoring(&job.job_id, Some(&ir));
        assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));

        merge_validation_issues(
            &mut report,
            vec![json!({
                "issueId": "issue-warning-only",
                "severity": "warning",
                "layer": "ReadingExamSourceV1",
                "path": "$",
                "message": "Node validator sidecar unavailable; Rust built-in validation was used.",
                "fixHint": null
            })],
        );

        assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
        let layer = report
            .get("layers")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|layer| layer.get("layer").and_then(Value::as_str) == Some("ReadingExamSourceV1"))
            .unwrap();
        assert_eq!(layer.get("passed").and_then(Value::as_bool), Some(true));
        assert_eq!(layer.get("issueCount").and_then(Value::as_u64), Some(1));
        assert_eq!(layer.get("warningCount").and_then(Value::as_u64), Some(1));
        assert_eq!(layer.get("errorCount").and_then(Value::as_u64), Some(0));
    }

    #[test]
    fn validation_report_v1_preserves_static_runtime_contract() {
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);

        let report = validate_for_runtime_gate(&root, &job.job_id, &ir, true).unwrap();
        let mut top_level_keys = report
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        top_level_keys.sort_unstable();
        assert_eq!(
            top_level_keys,
            vec![
                "generatedAt",
                "issues",
                "jobId",
                "layers",
                "passed",
                "runtime"
            ]
        );
        assert_eq!(
            report.get("jobId").and_then(Value::as_str),
            Some(job.job_id.as_str())
        );
        assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
        assert!(report.get("generatedAt").and_then(Value::as_str).is_some());
        assert!(report
            .get("issues")
            .and_then(Value::as_array)
            .map(Vec::is_empty)
            .unwrap_or(false));
        assert_eq!(
            report.pointer("/runtime/mode").and_then(Value::as_str),
            Some("static-rust")
        );
        assert_eq!(
            report.pointer("/runtime/adapter").and_then(Value::as_str),
            Some("rust-static-contract")
        );
        let layers = report
            .get("layers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(layers.len(), 4);
        for expected_layer in [
            "AuthoringIR",
            "ReadingExamSourceV1",
            "DomProtocol",
            "RuntimePreview",
        ] {
            let layer = layers
                .iter()
                .find(|layer| layer.get("layer").and_then(Value::as_str) == Some(expected_layer))
                .unwrap_or_else(|| panic!("missing layer {}", expected_layer));
            assert_eq!(layer.get("passed").and_then(Value::as_bool), Some(true));
            assert_eq!(layer.get("issueCount").and_then(Value::as_u64), Some(0));
            assert_eq!(layer.get("errorCount").and_then(Value::as_u64), Some(0));
            assert_eq!(layer.get("warningCount").and_then(Value::as_u64), Some(0));
        }

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
    fn llm_question_field_patches_are_rejected_in_favor_of_questions_array() {
        let job = test_job();
        let doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
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
            "suggestionId": "suggestion-question-patch",
            "groupId": first_group_id,
            "kind": "short_answer",
            "confidence": 0.99,
            "patch": [{"op":"replace","path":"/questions/0/prompt","value":"Unsafe direct prompt patch"}],
            "questions": [],
            "warnings": [],
            "evidence": {
                "source": "openai-compatible",
                "sourceBlockIds": [first_block_id],
                "quotes": [{"blockId": first_block_id, "text": "Questions 1-5"}]
            }
        });

        let issues = llm_suggestion_auto_apply_issues(&ir, &suggestion, &["questions".to_string()]);

        assert!(issues.iter().any(|issue| issue
            .starts_with("question_patch_must_use_questions_array:/questions/0/prompt")));
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
    fn rust_llm_fallback_remains_low_confidence_and_non_auto_applicable() {
        let job = test_job();
        let doc = sample_document_ir(&job, "auto");
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let group = ir
            .pointer("/groups/0")
            .cloned()
            .expect("fixture should have a group");
        let suggestion = deterministic_llm_output(
            &group,
            "extract_group",
            "llm gateway fallback: unit-test".to_string(),
        );

        assert!(
            suggestion
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap()
                < 0.85
        );
        assert_eq!(
            suggestion
                .pointer("/evidence/fallback")
                .and_then(Value::as_bool),
            Some(true)
        );
        let issues = llm_suggestion_auto_apply_issues(
            &ir,
            &json!({
                "suggestionId": "suggestion-fallback",
                "groupId": group.get("groupId").and_then(Value::as_str).unwrap(),
                "kind": suggestion.get("kind").cloned().unwrap(),
                "confidence": suggestion.get("confidence").cloned().unwrap(),
                "patch": suggestion.get("patch").cloned().unwrap(),
                "questions": suggestion.get("questions").cloned().unwrap(),
                "warnings": suggestion.get("warnings").cloned().unwrap(),
                "evidence": suggestion.get("evidence").cloned().unwrap()
            }),
            &["kind".to_string()],
        );

        assert!(issues.contains(&"confidence_below_auto_apply_threshold".to_string()));
        assert!(issues.contains(&"fallback_evidence_never_auto_applies".to_string()));
    }

    #[test]
    fn rust_openai_compatible_output_validation_adds_evidence_metadata() {
        let mut output = json!({
            "kind": "short_answer",
            "confidence": 0.91,
            "patch": [{"op": "replace", "path": "/kind", "value": "short_answer"}],
            "questions": [],
            "evidence": {
                "sourceBlockIds": ["b001"],
                "quotes": [{"blockId": "b001", "text": "Questions 1-2"}]
            }
        });
        let profile = json!({"model": "gpt-test"});
        let payload = json!({"usage": {"total_tokens": 42}});

        llm_gateway::validate_llm_suggestion_output(
            &mut output,
            "extract_group",
            &profile,
            &payload,
        )
        .unwrap();

        assert_eq!(
            output.pointer("/evidence/source").and_then(Value::as_str),
            Some("openai-compatible-rust")
        );
        assert_eq!(
            output.pointer("/evidence/model").and_then(Value::as_str),
            Some("gpt-test")
        );
        assert_eq!(
            output
                .pointer("/evidence/usage/total_tokens")
                .and_then(Value::as_u64),
            Some(42)
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
            .expect("no-text PDF fixture should parse through the supported parser chain");

        assert_pdf_parser_provenance(&ir, &fixture);
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
    fn auto_pipeline_llm_failure_keeps_text_import_in_llm_review() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let mut job = test_job();
        attach_fixture_source(&root, &mut job, "complex-reading.txt", "MainQuestion");
        save_job(&root, &job).unwrap();

        let report = run_auto_pipeline_core(&root, &job.job_id, None).unwrap();

        assert!(report
            .pointer("/llm/failures")
            .and_then(Value::as_array)
            .map(|failures| !failures.is_empty())
            .unwrap_or(false));
        assert_eq!(
            report.get("validationPassed").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            report.get("staticRuntimePassed").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            report.get("status").and_then(Value::as_str),
            Some("NeedsReview")
        );
        assert_eq!(
            report.get("currentStep").and_then(Value::as_str),
            Some("Authoring")
        );
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert_eq!(saved.current_step, WorkflowStep::Authoring);
        assert!(saved.issue_counts.needs_review > 0);
        assert!(!job_dir(&root, &job.job_id)
            .join("document-ir.json")
            .exists());
        assert!(!job_dir(&root, &job.job_id)
            .join("split-candidates.json")
            .exists());
        assert!(job_dir(&root, &job.job_id)
            .join("authoring-ir.json")
            .exists());
        assert!(job_dir(&root, &job.job_id)
            .join("authoring-project.json")
            .exists());
        assert!(!job_dir(&root, &job.job_id).join("cache").exists());
        assert!(!job_dir(&root, &job.job_id)
            .join("pipeline-report.json")
            .exists());
        assert!(!job_dir(&root, &job.job_id)
            .join("pipeline-report-summary.json")
            .exists());
        assert!(!job_dir(&root, &job.job_id)
            .join("cleanup-summary.json")
            .exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_pipeline_persists_llm_review_in_authoring_ir_after_minimization() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        crate::llm_profiles::save_profiles(
            &root,
            &[json!({
                "profileId": "profile-group-review",
                "name": "Group Review Test",
                "provider": "OpenAiCompatible",
                "baseUrl": "http://unit.test/v1",
                "model": "unit-test",
                "temperature": 0,
                "timeoutMs": 60000,
                "forceJson": true,
                "enabled": true
            })],
        )
        .unwrap();
        let mut job = test_job();
        attach_fixture_source(&root, &mut job, "complex-reading.txt", "MainQuestion");
        save_job(&root, &job).unwrap();

        let report = run_auto_pipeline_core_with_gateway(
            &root,
            &job.job_id,
            Some(AutoPipelineInput {
                profile_id: Some("profile-group-review".to_string()),
                ..AutoPipelineInput::default()
            }),
            |_root, _job_id, command_name, _input, _api_key| {
                Err(format!("intentional_group_review_failure:{command_name}"))
            },
        )
        .unwrap();
        assert_eq!(
            report.get("currentStep").and_then(Value::as_str),
            Some("Authoring")
        );
        let ir: Value = read_json(&job_dir(&root, &job.job_id).join("authoring-ir.json")).unwrap();
        let groups = ir.get("groups").and_then(Value::as_array).unwrap();
        assert!(groups.iter().any(|group| group
            .get("llmReview")
            .and_then(Value::as_object)
            .map(|review| review.get("required").and_then(Value::as_bool) == Some(true))
            .unwrap_or(false)));
        assert!(!job_dir(&root, &job.job_id).join("llm-suggestions").exists());
        assert!(!job_dir(&root, &job.job_id)
            .join("pipeline-report.json")
            .exists());
        assert!(job_dir(&root, &job.job_id)
            .join("authoring-ir.json")
            .exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_pipeline_retains_process_artifacts_only_when_diagnostics_enabled() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        write_diagnostics_settings(
            &root,
            &DiagnosticsSettings {
                keep_full_process_artifacts: true,
            },
        )
        .unwrap();
        let mut job = test_job();
        attach_fixture_source(&root, &mut job, "complex-reading.txt", "MainQuestion");
        save_job(&root, &job).unwrap();
        let job_path = job_dir(&root, &job.job_id);
        fs::create_dir_all(job_path.join("cache").join("fixture-stage")).unwrap();
        fs::create_dir_all(job_path.join("preview")).unwrap();
        fs::write(
            job_path
                .join("cache")
                .join("fixture-stage")
                .join("facts.json"),
            b"{}",
        )
        .unwrap();
        fs::write(job_path.join("preview").join("index.html"), b"diagnostic").unwrap();

        let report = run_auto_pipeline_core(&root, &job.job_id, None).unwrap();

        assert_eq!(
            report.get("status").and_then(Value::as_str),
            Some("NeedsReview")
        );
        assert!(job_path.join("document-ir.json").exists());
        assert!(job_path.join("split-candidates.json").exists());
        assert!(job_path.join("pipeline-report.json").exists());
        assert!(job_path
            .join("cache")
            .join("fixture-stage")
            .join("facts.json")
            .exists());
        assert!(job_path.join("preview").join("index.html").exists());
        let parser_cache = root.join("cache").join("parser");
        assert!(parser_cache.exists());
        let retained_parser_outputs = fs::read_dir(&parser_cache)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(&job.job_id))
            .collect::<Vec<_>>();
        assert!(
            !retained_parser_outputs.is_empty(),
            "diagnostics mode should retain root parser cache outputs"
        );
        assert!(job_path.join("authoring-ir.json").exists());
        assert!(job_path.join("authoring-project.json").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_pipeline_high_confidence_llm_auto_applies_without_human_verification() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        crate::llm_profiles::save_profiles(
            &root,
            &[json!({
                "profileId": "profile-high-confidence",
                "name": "High Confidence Group Test",
                "provider": "OpenAiCompatible",
                "baseUrl": "http://unit.test/v1",
                "model": "unit-test",
                "temperature": 0,
                "timeoutMs": 60000,
                "forceJson": true,
                "enabled": true
            })],
        )
        .unwrap();
        let mut job = test_job();
        attach_fixture_source(&root, &mut job, "complex-reading.txt", "MainQuestion");
        save_job(&root, &job).unwrap();

        let mut gateway_calls = 0usize;
        let report = run_auto_pipeline_core_with_gateway(
            &root,
            &job.job_id,
            Some(AutoPipelineInput {
                parse_mode: None,
                confidence_threshold: Some(0.85),
                profile_id: Some("profile-high-confidence".to_string()),
                execution_mode: None,
                target: None,
                allow_overwrite: None,
            }),
            |_root, _job_id, command_name, input, _api_key| {
                gateway_calls += 1;
                if command_name != "extract_group" {
                    return Err(format!("unexpected_command:{}", command_name));
                }
                let group = input.get("group").cloned().unwrap_or_else(|| json!({}));
                let source_block_ids = group
                    .get("sourceBlockIds")
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                let first_block_id = source_block_ids
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(Value::as_str)
                    .unwrap_or("unknown-block")
                    .to_string();
                let questions = group
                    .get("questions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|mut question| {
                        let display = question
                            .get("displayNumber")
                            .and_then(Value::as_str)
                            .unwrap_or("?")
                            .to_string();
                        if let Some(obj) = question.as_object_mut() {
                            obj.insert(
                                "prompt".to_string(),
                                json!(format!("LLM refined prompt {}", display)),
                            );
                            obj.insert(
                                "interaction".to_string(),
                                json!({"type": "text", "placeholder": "answer"}),
                            );
                            obj.remove("verified");
                            obj.remove("answer");
                        }
                        question
                    })
                    .collect::<Vec<_>>();

                Ok(json!({
                    "kind": "short_answer",
                    "confidence": 0.96,
                    "patch": [
                        {"op": "replace", "path": "/kind", "value": "short_answer"},
                        {"op": "replace", "path": "/layout/template", "value": "short_answer_list"}
                    ],
                    "questions": questions,
                    "warnings": [],
                    "evidence": {
                        "source": "openai-compatible-rust-mock",
                        "sourceBlockIds": source_block_ids,
                        "quotes": [{"blockId": first_block_id, "text": "Questions"}]
                    }
                }))
            },
        )
        .unwrap();

        assert!(gateway_calls > 0);
        assert_eq!(
            report.pointer("/llm/appliedCount").and_then(Value::as_u64),
            Some(gateway_calls as u64)
        );
        assert_eq!(
            report
                .pointer("/llm/blockedAutoApplyGroups")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            report
                .pointer("/llm/lowConfidenceGroups")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            report.get("status").and_then(Value::as_str),
            Some("NeedsReview")
        );
        assert_eq!(
            report.get("currentStep").and_then(Value::as_str),
            Some("Authoring")
        );
        assert!(
            report
                .pointer("/authoring/remainingReviewItems")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0
        );

        let ir: Value = read_json(&job_dir(&root, &job.job_id).join("authoring-ir.json")).unwrap();
        assert_eq!(
            ir.pointer("/audit/humanVerified").and_then(Value::as_bool),
            Some(false)
        );
        let groups = ir.get("groups").and_then(Value::as_array).unwrap();
        assert_eq!(
            groups
                .iter()
                .filter(|group| group.get("autoApplied").and_then(Value::as_bool) == Some(true))
                .count(),
            gateway_calls
        );
        let first_question = groups
            .first()
            .and_then(|group| group.get("questions"))
            .and_then(Value::as_array)
            .and_then(|questions| questions.first())
            .unwrap();
        assert!(first_question
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .starts_with("LLM refined prompt "));
        assert_eq!(
            first_question.get("verified").and_then(Value::as_bool),
            Some(false)
        );
        assert!(
            first_question
                .get("answer")
                .and_then(Value::as_str)
                .map(|answer| !answer.trim().is_empty())
                .unwrap_or(false),
            "auto-applied LLM structure must not erase parsed answers"
        );

        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert_eq!(saved.current_step, WorkflowStep::Authoring);
        assert!(saved.issue_counts.needs_review > 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_pipeline_keeps_no_text_pdf_review_visible_in_authoring() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let mut job = test_job();
        attach_fixture_source(&root, &mut job, "no-text.pdf", "MainQuestion");
        save_job(&root, &job).unwrap();

        let report = run_auto_pipeline_core(&root, &job.job_id, None).unwrap();

        assert!(report
            .pointer("/parser/warnings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|warning| warning
                .as_str()
                .unwrap_or_default()
                .contains("no extractable text")));
        assert_eq!(
            report
                .pointer("/parser/visionTranscription/attempted")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            report
                .pointer("/parser/visionTranscription/failure")
                .and_then(Value::as_str),
            Some("no_enabled_llm_profile_available_for_pdf_vision_transcription")
        );
        assert_eq!(
            report.get("status").and_then(Value::as_str),
            Some("NeedsReview")
        );
        assert_eq!(
            report.get("currentStep").and_then(Value::as_str),
            Some("Authoring")
        );
        assert_eq!(
            report.get("nextRoute").and_then(Value::as_str),
            Some("groups")
        );
        assert_eq!(
            report.get("userStatus").and_then(Value::as_str),
            Some("needsConfirmation")
        );
        assert!(report
            .get("userMessage")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("题稿已生成"));
        assert_eq!(
            report
                .pointer("/authoring/remainingReviewItems")
                .and_then(Value::as_u64),
            Some(0)
        );
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert_eq!(saved.current_step, WorkflowStep::Authoring);
        assert!(saved.issue_counts.needs_review > 0);
        let review = source_review_status(
            &root,
            &job.job_id,
            read_json_opt(&job_dir(&root, &job.job_id).join("document-ir.json"))
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        assert!(!source_review_issues(&review).is_empty());
        assert!(job_dir(&root, &job.job_id)
            .join("authoring-ir.json")
            .exists());
        assert!(!job_dir(&root, &job.job_id)
            .join("cleanup-summary.json")
            .exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_pipeline_reports_source_review_before_llm_review_in_authoring() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let mut job = test_job();
        job.title = "Source Review Priority".to_string();
        job.source_files = vec![test_source("txt")];
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();

        let mut doc = sample_document_ir(&job, "auto");
        doc["parser"]["warnings"] =
            json!(["source text requires human review before low-confidence LLM review"]);
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();
        write_source_review_status(&root, &job.job_id, Some(&doc), false, None).unwrap();

        let report = run_auto_pipeline_core_with_gateway(
            &root,
            &job.job_id,
            Some(AutoPipelineInput {
                parse_mode: Some("auto".to_string()),
                confidence_threshold: Some(0.85),
                profile_id: Some("profile-local-placeholder".to_string()),
                execution_mode: None,
                target: None,
                allow_overwrite: None,
            }),
            |_root, _job_id, command_name, _input, _api_key| {
                if command_name == "extract_group" {
                    return Ok(json!({
                        "kind": "true_false_not_given",
                        "confidence": 0.42,
                        "patch": [],
                        "questions": [],
                        "warnings": ["low confidence by design"],
                        "evidence": {"source": "unit-test-low-confidence"}
                    }));
                }
                Err(format!("unexpected_command:{}", command_name))
            },
        )
        .unwrap();

        assert!(report
            .pointer("/parser/warnings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|warning| warning
                .as_str()
                .unwrap_or_default()
                .contains("source text requires human review")));
        assert!(report
            .pointer("/llm/lowConfidenceGroups")
            .and_then(Value::as_array)
            .map(|groups| !groups.is_empty())
            .unwrap_or(false));
        assert_eq!(
            report.get("status").and_then(Value::as_str),
            Some("NeedsReview")
        );
        assert_eq!(
            report.get("currentStep").and_then(Value::as_str),
            Some("Authoring")
        );
        assert_eq!(
            report.get("nextRoute").and_then(Value::as_str),
            Some("groups")
        );
        assert_eq!(
            report.get("userStatus").and_then(Value::as_str),
            Some("needsConfirmation")
        );
        assert!(report
            .get("userMessage")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("源文件识别结果"));
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert_eq!(saved.current_step, WorkflowStep::Authoring);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_pipeline_blocks_umbrella_only_manual_import_from_export_ready() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let mut job = test_job();
        job.title = "P2 Umbrella Auto Pipeline".to_string();
        job.category = Some("P2".to_string());
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"p2-header","blockType":"header","text":"READING PASSAGE 2","html":"<h2>READING PASSAGE 2</h2>","bbox":[72,60,460,88],"confidence":0.99,"roleHint":"passage"},
                    {"blockId":"p2-umbrella","blockType":"paragraph","text":"Questions 14\u{2013}26","html":"<p>Questions 14\u{2013}26</p>","bbox":[72,92,520,120],"confidence":0.98,"roleHint":"question"},
                    {"blockId":"p2-passage","blockType":"paragraph","text":"The passage text is readable, but only the opening total question range was extracted. Concrete question prompts must be imported by the author.","html":"<p>The passage text is readable, but only the opening total question range was extracted. Concrete question prompts must be imported by the author.</p>","bbox":[72,130,520,210],"confidence":0.97,"roleHint":"passage"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 14 TRUE 15 FALSE 16 TRUE 17 NOT GIVEN 18 TRUE 19 FALSE 20 A 21 B 22 C 23 D 24 chemicals 25 threats 26 information","html":"<p>Answers 14 TRUE 15 FALSE 16 TRUE 17 NOT GIVEN 18 TRUE 19 FALSE 20 A 21 B 22 C 23 D 24 chemicals 25 threats 26 information</p>","bbox":[72,600,520,650],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();

        let report = run_auto_pipeline_core(&root, &job.job_id, None).unwrap();

        assert_eq!(
            report.get("validationPassed").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            report.get("staticRuntimePassed").and_then(Value::as_bool),
            Some(true)
        );
        assert!(
            report
                .pointer("/authoring/remainingReviewItems")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0
        );
        assert_eq!(
            report.get("status").and_then(Value::as_str),
            Some("NeedsReview")
        );
        assert!(matches!(
            report.get("currentStep").and_then(Value::as_str),
            Some("LlmReview" | "Authoring")
        ));
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert!(matches!(
            saved.current_step,
            WorkflowStep::LlmReview | WorkflowStep::Authoring
        ));
        assert!(saved.issue_counts.needs_review > 0);
        assert!(!job_dir(&root, &job.job_id)
            .join("split-candidates.json")
            .exists());
        assert!(!job_dir(&root, &job.job_id)
            .join("document-ir.json")
            .exists());
        let ir: Value = read_json(&job_dir(&root, &job.job_id).join("authoring-ir.json")).unwrap();
        assert_eq!(
            ir.pointer("/groups/0/requiresManualQuestionImport")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            ir.pointer("/audit/humanVerified").and_then(Value::as_bool),
            Some(false)
        );
        assert!(!job_dir(&root, &job.job_id)
            .join("cleanup-summary.json")
            .exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rust_backend_fixture_flow_exports_from_minimal_editable_state() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let mut job = test_job();
        attach_fixture_source(&root, &mut job, "complex-reading.txt", "MainQuestion");
        save_job(&root, &job).unwrap();

        let report = run_auto_pipeline_core(&root, &job.job_id, None).unwrap();

        assert_eq!(
            report.get("status").and_then(Value::as_str),
            Some("NeedsReview")
        );
        assert_eq!(
            report.get("currentStep").and_then(Value::as_str),
            Some("Authoring")
        );
        let job_path = job_dir(&root, &job.job_id);
        assert!(job_path.join("authoring-ir.json").exists());
        assert!(job_path.join("authoring-project.json").exists());
        assert!(job_path.join("source-review.json").exists());
        assert!(job_path.join("uploads").exists());
        for transient in [
            "document-ir.json",
            "split-candidates.json",
            "pipeline-report.json",
            "validation-report.json",
            "publish-readiness-report.json",
            "cleanup-summary.json",
        ] {
            assert!(
                !job_path.join(transient).exists(),
                "auto-pipeline should not persist {} in ordinary mode",
                transient
            );
        }
        assert!(!job_path.join("llm-suggestions").exists());
        assert!(!job_path.join("cache").exists());

        let mut ir: Value = read_json(&job_path.join("authoring-ir.json")).unwrap();
        assert!(ir
            .get("groups")
            .and_then(Value::as_array)
            .map(|groups| !groups.is_empty())
            .unwrap_or(false));
        let questions = ir
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
            .collect::<Vec<_>>();
        assert!(!questions.is_empty());
        assert!(questions
            .iter()
            .all(|question| question.get("verified").and_then(Value::as_bool) == Some(false)));
        verify_all_authoring_items(&mut ir);
        assert_eq!(
            ir.pointer("/audit/humanVerified").and_then(Value::as_bool),
            Some(true)
        );
        write_json(&job_path.join("authoring-ir.json"), &ir).unwrap();
        update_job(&root, &job.job_id, |job| {
            job.status = JobStatus::DraftSaved;
            job.current_step = WorkflowStep::Authoring;
            job.issue_counts.needs_review = 0;
        })
        .unwrap();

        let export_dir = root.join("fixture-export");
        let export = export_reading_assets_core(
            &root,
            &job.job_id,
            export_dir.to_string_lossy().as_ref(),
            true,
        )
        .unwrap();

        assert_eq!(
            export.pointer("/cleanup/cleaned").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            load_job(&root, &job.job_id).unwrap().status,
            JobStatus::Cleaned
        );
        let exam_id = export.get("examId").and_then(Value::as_str).unwrap();
        assert!(export_dir.join(format!("{}.json", exam_id)).exists());
        assert!(export_dir.join(format!("{}.js", exam_id)).exists());
        assert!(export_dir.join("manifest.js").exists());
        assert!(export_dir.join("validation-report.json").exists());

        let final_project: Value = read_json(&job_path.join("authoring-project.json")).unwrap();
        assert_eq!(
            final_project
                .pointer("/exportSummary/type")
                .and_then(Value::as_str),
            Some("reading-assets")
        );
        assert!(job_path.join("authoring-ir.json").exists());
        assert!(job_path.join("authoring-project.json").exists());
        assert!(job_path.join("source-review.json").exists());
        assert!(job_path.join("uploads").exists());
        assert!(job_path
            .join("uploads")
            .read_dir()
            .unwrap()
            .next()
            .is_some());
        let parser_cache = root.join("cache").join("parser");
        if parser_cache.exists() {
            let retained_job_parser_outputs = fs::read_dir(&parser_cache)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| name.starts_with(&job.job_id))
                .collect::<Vec<_>>();
            assert!(
                retained_job_parser_outputs.is_empty(),
                "export should remove job-scoped parser cache outputs: {:?}",
                retained_job_parser_outputs
            );
        }
        for transient in [
            "document-ir.json",
            "split-candidates.json",
            "pipeline-report.json",
            "validation-report.json",
            "publish-readiness-report.json",
            "cleanup-summary.json",
            "llm-last-suggestion.json",
            "llm-calls.jsonl",
        ] {
            assert!(
                !job_path.join(transient).exists(),
                "export should leave only minimal editable state, found {}",
                transient
            );
        }
        for transient_dir in ["cache", "preview", "llm-suggestions"] {
            assert!(
                !job_path.join(transient_dir).exists(),
                "export should remove transient dir {}",
                transient_dir
            );
        }

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
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert!(saved.issue_counts.errors > 0);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_e2e_diagnostic_failure_does_not_block_static_export_ready() {
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let static_report = validate_for_runtime_gate(&root, &job.job_id, &ir, false).unwrap();
        let diagnostic_report = json!({
            "jobId": job.job_id,
            "passed": false,
            "layers": [{"layer": "RuntimePreview", "passed": false, "issueCount": 1, "errorCount": 1, "warningCount": 0}],
            "issues": [{
                "issueId": "issue-diagnostic",
                "severity": "error",
                "layer": "RuntimePreview",
                "path": "runtime.execution",
                "message": "Preview E2E diagnostic unavailable"
            }],
            "runtime": {"mode": "fallback", "fallbackReason": "real_runtime_unavailable"}
        });
        write_json(
            &job_dir(&root, &job.job_id).join("validation-report.json"),
            &diagnostic_report,
        )
        .unwrap();
        let readiness_passed =
            publish_readiness_gate(&root, &job.job_id, &ir, static_report.clone())
                .unwrap()
                .get("passed")
                .and_then(Value::as_bool)
                .unwrap_or(false);

        apply_preview_e2e_job_state(&root, &job.job_id, &static_report, readiness_passed).unwrap();

        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::ExportReady);
        let saved_report: Value =
            read_json(&job_dir(&root, &job.job_id).join("validation-report.json")).unwrap();
        assert_eq!(
            saved_report
                .pointer("/runtime/mode")
                .and_then(Value::as_str),
            Some("fallback")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn umbrella_question_range_detection_keeps_opening_instructions_distinct() {
        assert!(is_dynamic_umbrella_question_range(
            "Questions 14-26 are based on Reading Passage 2 below."
        ));
        assert!(is_dynamic_umbrella_question_range(
            "Questions 14\u{2013}26 are based on Reading Passage 2 below."
        ));
        assert!(is_dynamic_umbrella_question_range(
            "You should spend about 20 minutes on Questions 14-26, which are based on Reading Passage 2 below."
        ));
        assert!(!is_dynamic_umbrella_question_range(
            "Questions 14-19 Do the following statements agree with the information given in Reading Passage 2?"
        ));
        assert!(!is_dynamic_umbrella_question_range(
            "Questions 20-23 Choose the correct letter, A, B, C or D."
        ));
    }

    #[test]
    fn layout_aware_split_reorders_two_column_blocks_and_preserves_continuations() {
        let job = make_job(CreateJobInput {
            title: Some("Two Column Reading Order".to_string()),
            category: Some("P2".to_string()),
            frequency: Some("hard".to_string()),
            tags: Some(vec!["layout-aware".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 600,
                "height": 842,
                "blocks": [
                    {"blockId":"right-heading","blockType":"paragraph","text":"Questions 6-8 Classify the following statements according to the groups A-D.","html":"<p>Questions 6-8 Classify the following statements according to the groups A-D.</p>","bbox":[330,120,560,150],"confidence":0.94,"roleHint":"question"},
                    {"blockId":"left-heading","blockType":"paragraph","text":"Questions 1-5 Choose TWO letters, A-E. Which TWO features are mentioned?","html":"<p>Questions 1-5 Choose TWO letters, A-E.</p>","bbox":[60,120,290,150],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"right-options","blockType":"table","text":"A marine animals B insects C birds D plants You may use any letter more than once.","html":"<table><tr><td>A marine animals</td></tr></table>","bbox":[330,160,560,250],"confidence":0.92,"roleHint":"question"},
                    {"blockId":"left-options","blockType":"paragraph","text":"A faster growth B lower cost C stronger roots D brighter leaves E longer stems 1 ___ 2 ___ 3 ___ 4 ___ 5 ___","html":"<p>A faster growth B lower cost C stronger roots D brighter leaves E longer stems</p>","bbox":[60,160,290,250],"confidence":0.93,"roleHint":"question"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 1 A 2 C 3 D 4 E 5 B 6 A 7 C 8 D","html":"<p>Answers 1 A 2 C 3 D 4 E 5 B 6 A 7 C 8 D</p>","bbox":[60,700,560,760],"confidence":0.91,"roleHint":"answer"}
                ]
            }, {
                "pageIndex": 2,
                "width": 600,
                "height": 842,
                "blocks": [
                    {"blockId":"right-more","blockType":"paragraph","text":"6 ___ 7 ___ 8 ___","html":"<p>6 ___ 7 ___ 8 ___</p>","bbox":[330,80,560,120],"confidence":0.92,"roleHint":"question"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let blocks = dynamic_document_blocks(Some(&doc));
        assert_eq!(
            blocks
                .iter()
                .take(4)
                .map(|block| block.get("blockId").and_then(Value::as_str).unwrap())
                .collect::<Vec<_>>(),
            vec![
                "left-heading",
                "left-options",
                "right-heading",
                "right-options"
            ]
        );

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let groups = split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0].get("kindHint").and_then(Value::as_str),
            Some("multi_choice")
        );
        assert_eq!(
            groups[0]
                .pointer("/classification/interaction/type")
                .and_then(Value::as_str),
            Some("checkbox")
        );
        assert_eq!(
            groups[0]
                .pointer("/classification/interaction/maxSelections")
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            groups[0]
                .get("blockIds")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec!["left-heading", "left-options"]
        );
        assert_eq!(
            groups[1].get("kindHint").and_then(Value::as_str),
            Some("classification")
        );
        assert_eq!(
            groups[1]
                .pointer("/classification/interaction/allowOptionReuse")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            groups[1]
                .get("blockIds")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec!["right-heading", "right-options", "right-more"]
        );
        assert!(groups[1]
            .get("sectionEvidence")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|item| {
                item.get("blockId").and_then(Value::as_str) == Some("right-more")
                    && item.get("pageIndex").and_then(Value::as_u64) == Some(2)
            }));
        assert_eq!(
            groups[1]
                .pointer("/sectionEvidence/0/pageIndex")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert!(groups[1]
            .get("continuationEdges")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|edge| edge.get("reason").and_then(Value::as_str)
                == Some("cross-page-continuation")));

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/groups/0/questions/0/interaction/maxSelections")
                .and_then(Value::as_u64),
            Some(2)
        );
        assert!(ir
            .pointer("/groups/0/reviewWarnings")
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));
        assert_eq!(
            ir.pointer("/groups/1/allowOptionReuse")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            ir.pointer("/groups/1/classificationEvidence/0")
                .and_then(Value::as_str),
            Some("right-heading")
        );
        assert_eq!(
            ir.pointer("/groups/1/sourceBlockIds")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec!["right-heading", "right-options", "right-more"]
        );
        assert!(ir
            .pointer("/groups/1/questions/0/sourceBlockIds")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|item| item.as_str() == Some("right-more")));
        assert!(ir
            .pointer("/groups/1/sectionEvidence")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|item| {
                item.get("blockId").and_then(Value::as_str) == Some("right-more")
                    && item.get("pageIndex").and_then(Value::as_u64) == Some(2)
            }));
        assert!(ir
            .pointer("/groups/1/continuationEdges")
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));
    }

    #[test]
    fn heading_matching_passage_after_heading_list_returns_to_passage_range() {
        let job = make_job(CreateJobInput {
            title: Some("Paternity Leave".to_string()),
            category: Some("P2".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["split-regression".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 600,
                "height": 842,
                "blocks": [
                    {"blockId":"b001","blockType":"paragraph","text":"READING PASSAGE 2","html":"<p>READING PASSAGE 2</p>","bbox":[72,60,520,90],"confidence":0.99},
                    {"blockId":"b002","blockType":"paragraph","text":"You should spend about 20 minutes on Questions 14-26, which are based on Reading Passage 2 on the following pages.","html":"<p>You should spend about 20 minutes on Questions 14-26, which are based on Reading Passage 2 on the following pages.</p>","bbox":[72,100,520,140],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"b003","blockType":"header","text":"Questions 14-19","html":"<h3>Questions 14-19</h3>","bbox":[72,150,520,180],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"b004","blockType":"paragraph","text":"Reading Passage 2 has six sections, A-F.","html":"<p>Reading Passage 2 has six sections, A-F.</p>","bbox":[72,185,520,210],"confidence":0.95},
                    {"blockId":"b005","blockType":"paragraph","text":"Choose the correct heading for each section from the list of headings below.","html":"<p>Choose the correct heading for each section from the list of headings below.</p>","bbox":[72,215,520,245],"confidence":0.95},
                    {"blockId":"b006","blockType":"paragraph","text":"14 Section A 15 Section B 16 Section C 17 Section D 18 Section E 19 Section F","html":"<p>14 Section A 15 Section B 16 Section C 17 Section D 18 Section E 19 Section F</p>","bbox":[72,250,520,330],"confidence":0.94},
                    {"blockId":"b007","blockType":"paragraph","text":"List of Headings","html":"<p>List of Headings</p>","bbox":[72,340,520,360],"confidence":0.95},
                    {"blockId":"b008","blockType":"paragraph","text":"i Opposition by employers to parental leave","html":"<p>i Opposition by employers to parental leave</p>","bbox":[72,365,520,385],"confidence":0.94},
                    {"blockId":"b009","blockType":"paragraph","text":"ii An illustration of a trend in one country","html":"<p>ii An illustration of a trend in one country</p>","bbox":[72,390,520,410],"confidence":0.94}
                ]
            }, {
                "pageIndex": 2,
                "width": 600,
                "height": 842,
                "blocks": [
                    {"blockId":"b010","blockType":"paragraph","text":"Paternity Leave","html":"<h2>Paternity Leave</h2>","bbox":[72,60,520,90],"confidence":0.97},
                    {"blockId":"b011","blockType":"paragraph","text":"A At a course for fathers-to-be in New York, participants are introduced to baby maintenance for beginners and practical childcare routines.","html":"<p>A At a course for fathers-to-be in New York, participants are introduced to baby maintenance for beginners and practical childcare routines.</p>","bbox":[72,100,520,170],"confidence":0.96},
                    {"blockId":"b012","blockType":"paragraph","text":"B In general, legal and financial support for new parents is better than it has ever been in many developed countries.","html":"<p>B In general, legal and financial support for new parents is better than it has ever been in many developed countries.</p>","bbox":[72,180,520,250],"confidence":0.95}
                ]
            }, {
                "pageIndex": 3,
                "width": 600,
                "height": 842,
                "blocks": [
                    {"blockId":"b013","blockType":"header","text":"Questions 20 and 21","html":"<h3>Questions 20 and 21</h3>","bbox":[72,60,520,90],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"b014","blockType":"paragraph","text":"Choose TWO letters, A-E. Which TWO problems may be caused by maternity leave?","html":"<p>Choose TWO letters, A-E. Which TWO problems may be caused by maternity leave?</p>","bbox":[72,100,520,140],"confidence":0.94}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let passage_range = split
            .pointer("/passageCandidates/0/range")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| item.as_str().map(ToString::to_string))
            .collect::<Vec<_>>();
        assert!(passage_range.contains(&"b010".to_string()));
        assert!(passage_range.contains(&"b011".to_string()));
        assert!(passage_range.contains(&"b012".to_string()));
        assert!(!passage_range.contains(&"b006".to_string()));
        assert!(!passage_range.contains(&"b007".to_string()));

        let first_group_ids = split
            .pointer("/questionGroupCandidates/0/blockIds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| item.as_str().map(ToString::to_string))
            .collect::<Vec<_>>();
        assert!(first_group_ids.contains(&"b006".to_string()));
        assert!(first_group_ids.contains(&"b007".to_string()));
        assert!(first_group_ids.contains(&"b008".to_string()));
        assert!(!first_group_ids.contains(&"b010".to_string()));
        assert!(!first_group_ids.contains(&"b011".to_string()));
        assert!(!first_group_ids.contains(&"b012".to_string()));

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let passage_html = ir
            .pointer("/passage/htmlBlocks/0/html")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(passage_html.contains("Paternity Leave"));
        assert!(passage_html.contains("fathers-to-be"));
        assert!(!ir
            .pointer("/groups/0/sourceBlockIds")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|item| item.as_str() == Some("b011")));
    }

    #[test]
    fn passage_after_front_loaded_questions_returns_to_passage_range() {
        let job = make_job(CreateJobInput {
            title: Some("The fashion industry".to_string()),
            category: Some("P2".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["split-regression".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 600,
                "height": 842,
                "blocks": [
                    {"blockId":"b001","blockType":"paragraph","text":"READING PASSAGE 2","html":"<p>READING PASSAGE 2</p>","bbox":[72,60,520,90],"confidence":0.99},
                    {"blockId":"b002","blockType":"paragraph","text":"You should spend about 20 minutes on Questions 14-26, which are based on Reading Passage 2 on the following pages.","html":"<p>You should spend about 20 minutes on Questions 14-26, which are based on Reading Passage 2 on the following pages.</p>","bbox":[72,100,520,140],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"b003","blockType":"header","text":"Questions 14-20","html":"<h3>Questions 14-20</h3>","bbox":[72,150,520,180],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"b004","blockType":"paragraph","text":"Choose the correct heading for each section from the list of headings below.","html":"<p>Choose the correct heading for each section from the list of headings below.</p>","bbox":[72,185,520,215],"confidence":0.95},
                    {"blockId":"b005","blockType":"paragraph","text":"14 Section A 15 Section B 16 Section C 17 Section D 18 Section E 19 Section F 20 Section G","html":"<p>14 Section A 15 Section B 16 Section C 17 Section D 18 Section E 19 Section F 20 Section G</p>","bbox":[72,220,520,280],"confidence":0.94},
                    {"blockId":"b006","blockType":"paragraph","text":"List of Headings","html":"<p>List of Headings</p>","bbox":[72,285,520,310],"confidence":0.95},
                    {"blockId":"b007","blockType":"paragraph","text":"i How new clothing styles are created","html":"<p>i How new clothing styles are created</p>","bbox":[72,315,520,335],"confidence":0.94}
                ]
            }, {
                "pageIndex": 2,
                "width": 600,
                "height": 842,
                "blocks": [
                    {"blockId":"b008","blockType":"header","text":"Questions 21-24","html":"<h3>Questions 21-24</h3>","bbox":[72,60,520,90],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"b009","blockType":"paragraph","text":"Complete the summary below.","html":"<p>Complete the summary below.</p>","bbox":[72,100,520,130],"confidence":0.94},
                    {"blockId":"b010","blockType":"paragraph","text":"Choose NO MORE THAN TWO WORDS from the passage for each answer.","html":"<p>Choose NO MORE THAN TWO WORDS from the passage for each answer.</p>","bbox":[72,140,520,170],"confidence":0.94},
                    {"blockId":"b011","blockType":"header","text":"Questions 25 and 26","html":"<h3>Questions 25 and 26</h3>","bbox":[72,180,520,210],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"b012","blockType":"paragraph","text":"Choose TWO letters, A-E.","html":"<p>Choose TWO letters, A-E.</p>","bbox":[72,220,520,250],"confidence":0.94},
                    {"blockId":"b013","blockType":"paragraph","text":"Which TWO of the following statements does the writer make about garment assembly?","html":"<p>Which TWO of the following statements does the writer make about garment assembly?</p>","bbox":[72,260,520,290],"confidence":0.94},
                    {"blockId":"b014","blockType":"paragraph","text":"A The majority of sewing is done by computer-operated machines. B Highly skilled workers are the most important requirement. C Most businesses use other companies to manufacture their products. D Fasteners and labels are attached after the clothes have been made up. E Manufacturers usually produce one range of women’s clothing annually.","html":"<p>A The majority of sewing is done by computer-operated machines. B Highly skilled workers are the most important requirement. C Most businesses use other companies to manufacture their products. D Fasteners and labels are attached after the clothes have been made up. E Manufacturers usually produce one range of women’s clothing annually.</p>","bbox":[72,300,520,360],"confidence":0.94}
                ]
            }, {
                "pageIndex": 3,
                "width": 600,
                "height": 842,
                "blocks": [
                    {"blockId":"b015","blockType":"paragraph","text":"The fashion industry","html":"<h2>The fashion industry</h2>","bbox":[72,60,520,90],"confidence":0.97},
                    {"blockId":"b016","blockType":"paragraph","text":"A The fashion industry is a multibillion-dollar global enterprise devoted to the business of making and selling clothes. It encompasses all types of garments, from designer fashions to ordinary everyday clothing, and accounts for a significant share of world economic output.","html":"<p>A The fashion industry is a multibillion-dollar global enterprise devoted to the business of making and selling clothes. It encompasses all types of garments, from designer fashions to ordinary everyday clothing, and accounts for a significant share of world economic output.</p>","bbox":[72,100,520,180],"confidence":0.96},
                    {"blockId":"b017","blockType":"paragraph","text":"B The fashion industry is a product of the modern age. Prior to the mid-19th century, virtually all clothing was handmade for individuals, but by the beginning of the 20th century clothing had increasingly come to be mass-produced in standard sizes.","html":"<p>B The fashion industry is a product of the modern age. Prior to the mid-19th century, virtually all clothing was handmade for individuals, but by the beginning of the 20th century clothing had increasingly come to be mass-produced in standard sizes.</p>","bbox":[72,190,520,270],"confidence":0.95}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let passage_range = split
            .pointer("/passageCandidates/0/range")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| item.as_str().map(ToString::to_string))
            .collect::<Vec<_>>();
        assert!(passage_range.contains(&"b015".to_string()));
        assert!(passage_range.contains(&"b016".to_string()));
        assert!(passage_range.contains(&"b017".to_string()));
        assert!(!passage_range.contains(&"b014".to_string()));

        let last_group_ids = split
            .pointer("/questionGroupCandidates/2/blockIds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| item.as_str().map(ToString::to_string))
            .collect::<Vec<_>>();
        assert!(last_group_ids.contains(&"b014".to_string()));
        assert!(!last_group_ids.contains(&"b015".to_string()));
        assert!(!last_group_ids.contains(&"b016".to_string()));

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let passage_html = ir
            .pointer("/passage/htmlBlocks/0/html")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(passage_html.contains("The fashion industry"));
        assert!(passage_html.contains("multibillion-dollar global enterprise"));
        assert!(!ir
            .pointer("/groups/2/sourceBlockIds")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|item| item.as_str() == Some("b016")));
    }

    #[test]
    fn no_text_placeholder_blocks_are_excluded_from_question_groups() {
        let job = make_job(CreateJobInput {
            title: Some("On art and artists".to_string()),
            category: Some("P3".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["split-regression".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 600,
                "height": 842,
                "blocks": [
                    {"blockId":"b001","blockType":"paragraph","text":"READING PASSAGE 3","html":"<p>READING PASSAGE 3</p>","bbox":[72,60,520,90],"confidence":0.99},
                    {"blockId":"b002","blockType":"paragraph","text":"Art and the people who make it have long shaped public taste and social debate in different historical periods.","html":"<p>Art and the people who make it have long shaped public taste and social debate in different historical periods.</p>","bbox":[72,100,520,170],"confidence":0.96},
                    {"blockId":"b003","blockType":"header","text":"Questions 37-40","html":"<h3>Questions 37-40</h3>","bbox":[72,180,520,210],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"b004","blockType":"paragraph","text":"Complete the summary below. Choose ONE WORD ONLY from the passage for each answer.","html":"<p>Complete the summary below. Choose ONE WORD ONLY from the passage for each answer.</p>","bbox":[72,220,520,260],"confidence":0.95},
                    {"blockId":"b005","blockType":"paragraph","text":"The writer argues that art depends on 37 ______ and public institutions, while artists also need 38 ______ from critics. Museums can provide 39 ______, but markets often reward 40 ______.","html":"<p>The writer argues that art depends on 37 ______ and public institutions, while artists also need 38 ______ from critics. Museums can provide 39 ______, but markets often reward 40 ______.</p>","bbox":[72,270,520,340],"confidence":0.94}
                ]
            }, {
                "pageIndex": 2,
                "width": 600,
                "height": 842,
                "blocks": [
                    {"blockId":"b006","blockType":"paragraph","text":"[No extractable text on page 2]","html":"<p>[No extractable text on page 2]</p>","bbox":[72,60,520,90],"confidence":0.2}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let first_group_ids = split
            .pointer("/questionGroupCandidates/0/blockIds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| item.as_str().map(ToString::to_string))
            .collect::<Vec<_>>();
        assert!(first_group_ids.contains(&"b005".to_string()));
        assert!(!first_group_ids.contains(&"b006".to_string()));

        let passage_range = split
            .pointer("/passageCandidates/0/range")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| item.as_str().map(ToString::to_string))
            .collect::<Vec<_>>();
        assert!(!passage_range.contains(&"b006".to_string()));

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let group_body = ir
            .pointer("/groups/0/questions/0/prompt")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let ir_text = serde_json::to_string(&ir).unwrap();
        assert!(!group_body.contains("No extractable text"));
        assert!(!ir_text.contains("No extractable text"));
    }

    #[test]
    fn rotated_page_bbox_is_normalized_before_split_ordering() {
        let job = make_job(CreateJobInput {
            title: Some("Rotated PDF Reading Order".to_string()),
            category: Some("P2".to_string()),
            frequency: Some("hard".to_string()),
            tags: Some(vec!["rotation".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 600,
                "height": 800,
                "rotation": 90,
                "blocks": [
                    {"blockId":"later-question","blockType":"paragraph","text":"Questions 3-4 Complete the sentences below. Choose ONE WORD ONLY from the passage.","html":"<p>Questions 3-4 Complete the sentences below.</p>","bbox":[470,120,520,360],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"earlier-question","blockType":"paragraph","text":"Questions 1-2 Choose the correct letter, A, B or C.","html":"<p>Questions 1-2 Choose the correct letter.</p>","bbox":[520,120,570,360],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 1 A 2 B 3 archive 4 maps","html":"<p>Answers 1 A 2 B 3 archive 4 maps</p>","bbox":[40,120,90,500],"confidence":0.90,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let blocks = dynamic_document_blocks(Some(&doc));
        assert_eq!(
            blocks
                .iter()
                .take(2)
                .map(|block| block.get("blockId").and_then(Value::as_str).unwrap())
                .collect::<Vec<_>>(),
            vec!["earlier-question", "later-question"]
        );

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/blockIds/0")
                .and_then(Value::as_str),
            Some("earlier-question")
        );
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/sectionEvidence/0/pageRotation")
                .and_then(Value::as_i64),
            Some(90)
        );
        assert!(split
            .pointer("/questionGroupCandidates/0/sectionEvidence/0/normalizedBbox")
            .and_then(Value::as_array)
            .map(|bbox| bbox.len() == 4)
            .unwrap_or(false));
    }

    #[test]
    fn same_column_blocks_keep_original_order_when_bbox_y_wraps() {
        let job = make_job(CreateJobInput {
            title: Some("Wrapped Y Reading Order".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["reading-order".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"q1-5-heading","blockType":"paragraph","text":"Questions 1-5","html":"<p>Questions 1-5</p>","bbox":[72,72,520,108],"confidence":0.98,"roleHint":"question"},
                    {"blockId":"q1-5-instruction","blockType":"paragraph","text":"Which paragraph contains the following information?","html":"<p>Which paragraph contains the following information?</p>","bbox":[72,110,520,150],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"q1","blockType":"paragraph","text":"1 Humans imagine yawning.","html":"<p>1 Humans imagine yawning.</p>","bbox":[72,160,520,200],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"q2","blockType":"paragraph","text":"2 Occupations are linked to yawning.","html":"<p>2 Occupations are linked to yawning.</p>","bbox":[72,202,520,242],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"q3","blockType":"paragraph","text":"3 A research overview on yawning.","html":"<p>3 A research overview on yawning.</p>","bbox":[72,244,520,284],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"q4","blockType":"paragraph","text":"4 Brain temperature is regulated.","html":"<p>4 Brain temperature is regulated.</p>","bbox":[72,286,520,326],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"q5","blockType":"paragraph","text":"5 Earlier theories were disproved.","html":"<p>5 Earlier theories were disproved.</p>","bbox":[72,328,520,368],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"q6-9-heading","blockType":"paragraph","text":"Questions 6-9","html":"<p>Questions 6-9</p>","bbox":[72,370,520,410],"confidence":0.98,"roleHint":"question"},
                    {"blockId":"q6-9-instruction","blockType":"paragraph","text":"Match each with the correct university. Write the correct letter, A, B or C.","html":"<p>Match each with the correct university.</p>","bbox":[72,412,520,452],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"q6","blockType":"paragraph","text":"6 There is no gender difference.","html":"<p>6 There is no gender difference.</p>","bbox":[72,454,520,494],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"q7","blockType":"paragraph","text":"7 Certain disorders reduce contagious yawning.","html":"<p>7 Certain disorders reduce contagious yawning.</p>","bbox":[72,72,520,108],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"q8","blockType":"paragraph","text":"8 Yawning is linked to breathing.","html":"<p>8 Yawning is linked to breathing.</p>","bbox":[72,110,520,150],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"q9","blockType":"paragraph","text":"9 Empathy training increases yawning.","html":"<p>9 Empathy training increases yawning.</p>","bbox":[72,152,520,192],"confidence":0.97,"roleHint":"question"},
                    {"blockId":"list","blockType":"paragraph","text":"A University at Albany B University of Leeds C University of London","html":"<p>A University at Albany B University of Leeds C University of London</p>","bbox":[72,194,520,234],"confidence":0.96,"roleHint":"question"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let blocks = dynamic_document_blocks(Some(&doc));
        assert_eq!(
            blocks
                .iter()
                .map(|block| block.get("blockId").and_then(Value::as_str).unwrap())
                .collect::<Vec<_>>(),
            vec![
                "q1-5-heading",
                "q1-5-instruction",
                "q1",
                "q2",
                "q3",
                "q4",
                "q5",
                "q6-9-heading",
                "q6-9-instruction",
                "q6",
                "q7",
                "q8",
                "q9",
                "list"
            ]
        );

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/blockIds")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec![
                "q1-5-heading",
                "q1-5-instruction",
                "q1",
                "q2",
                "q3",
                "q4",
                "q5"
            ]
        );
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/1/blockIds")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec![
                "q6-9-heading",
                "q6-9-instruction",
                "q6",
                "q7",
                "q8",
                "q9",
                "list"
            ]
        );

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/groups/0/questions/0/prompt")
                .and_then(Value::as_str),
            Some("Humans imagine yawning.")
        );
        assert_eq!(
            ir.pointer("/groups/0/questions/4/prompt")
                .and_then(Value::as_str),
            Some("Earlier theories were disproved.")
        );
        assert_eq!(
            ir.pointer("/groups/1/questions/3/prompt")
                .and_then(Value::as_str),
            Some("Empathy training increases yawning.")
        );
    }

    #[test]
    fn lucy_notes_completion_keeps_questions_6_to_13_in_one_inline_group() {
        let mut job = make_job(CreateJobInput {
            title: Some("What Lucy Taught Us".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["lucy-regression".to_string()]),
            llm_profile_id: None,
        });
        job.source_files = vec![test_source("pdf")];
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"passage-title","blockType":"header","text":"READING PASSAGE 1 What Lucy Taught Us","html":"<h2>What Lucy Taught Us</h2>","bbox":[72,60,520,100],"confidence":0.99,"roleHint":"passage"},
                    {"blockId":"passage-body","blockType":"paragraph","text":"The discovery of Lucy helped scientists understand how early humans moved, ate and survived in woodland environments.","html":"<p>The discovery of Lucy helped scientists understand early humans.</p>","bbox":[72,110,520,260],"confidence":0.97,"roleHint":"passage"},
                    {"blockId":"q1-5","blockType":"paragraph","text":"Questions 1-5 Do the following statements agree with the information given in Reading Passage 1? TRUE if the statement agrees with the information FALSE if the statement contradicts the information NOT GIVEN if there is no information on this","html":"<p>Questions 1-5 Do the following statements agree with the information?</p>","bbox":[72,280,520,340],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"q1-5-items","blockType":"paragraph","text":"1 Lucy was found in Africa. 2 Lucy's skeleton was complete. 3 Scientists disagreed about Lucy's age. 4 Lucy could walk upright. 5 Lucy lived in trees only.","html":"<p>1 Lucy was found in Africa. 2 Lucy's skeleton was complete.</p>","bbox":[72,345,520,430],"confidence":0.94,"roleHint":"question"},
                    {"blockId":"q6-13","blockType":"paragraph","text":"Questions 6-13 Complete the notes below. Choose ONE WORD ONLY from the passage for each answer. What Lucy taught us Lucy's environment included 6 _______ of trees. Scientists studied her 7 _______ and teeth. The shape of her 8 _______ showed she could walk upright. Her arms suggest she still climbed 9 _______. The discovery changed ideas about the evolution of 10 _______. It showed that walking came before larger 11 _______. Researchers compared Lucy with modern 12 _______. The fossil remains important evidence for human 13 _______.","html":"<p>Questions 6-13 Complete the notes below.</p>","bbox":[72,450,520,620],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 1 TRUE 2 FALSE 3 NOT GIVEN 4 TRUE 5 FALSE 6 branches 7 bones 8 pelvis 9 trees 10 humans 11 brains 12 apes 13 evolution","html":"<p>Answers 1 TRUE 2 FALSE 3 NOT GIVEN 4 TRUE 5 FALSE 6 branches 7 bones 8 pelvis 9 trees 10 humans 11 brains 12 apes 13 evolution</p>","bbox":[72,700,520,760],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let groups = split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let ranges = groups
            .iter()
            .map(|group| {
                let range = group
                    .get("questionRange")
                    .and_then(Value::as_array)
                    .unwrap();
                (
                    range.first().and_then(Value::as_u64).unwrap(),
                    range.get(1).and_then(Value::as_u64).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(ranges, vec![(1, 5), (6, 13)]);
        assert_eq!(
            groups[1].get("kindHint").and_then(Value::as_str),
            Some("sentence_completion")
        );
        assert_eq!(
            groups[1].get("layoutHint").and_then(Value::as_str),
            Some("inline_completion")
        );

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/groups/1/questionRange")
                .and_then(Value::as_array)
                .map(|range| (
                    range.first().and_then(Value::as_u64).unwrap(),
                    range.get(1).and_then(Value::as_u64).unwrap()
                )),
            Some((6, 13))
        );
        assert_eq!(
            ir.pointer("/groups/1/layout/layoutHint")
                .and_then(Value::as_str),
            Some("inline_completion")
        );
        assert!(ir
            .pointer("/groups/1/layout/notes")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("6 _______ of trees"));
        assert_eq!(
            ir.pointer("/groups/1/questions")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(|question| question.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["q6", "q7", "q8", "q9", "q10", "q11", "q12", "q13"]
        );
        assert_eq!(
            ir.pointer("/answerKey/q6").and_then(Value::as_str),
            Some("branches")
        );
        assert_eq!(
            ir.pointer("/answerKey/q13").and_then(Value::as_str),
            Some("evolution")
        );

        let source = reading_source(&ir);
        let body_html = source
            .pointer("/questionGroups/1/bodyHtml")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(body_html.contains("notes-completion"));
        assert!(body_html.contains("name=\"q6\""));
        assert!(body_html.contains("name=\"q13\""));
        assert_eq!(
            source
                .pointer("/questionGroups/1/questionIds")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec!["q6", "q7", "q8", "q9", "q10", "q11", "q12", "q13"]
        );
    }

    #[test]
    fn numbered_ellipsis_blanks_render_as_one_inline_completion_group() {
        let mut job = make_job(CreateJobInput {
            title: Some("Artwork Notes Regression".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["inline-ellipsis-regression".to_string()]),
            llm_profile_id: None,
        });
        job.source_files = vec![test_source("pdf")];
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"passage","blockType":"paragraph","text":"The artist combined several techniques across a long career.","html":"<p>The artist combined several techniques.</p>","bbox":[72,80,520,180],"confidence":0.98,"roleHint":"passage"},
                    {"blockId":"q8-13","blockType":"paragraph","text":"Questions 8-13 Write ONE WORD ONLY from the passage for each answer. Early work: 8……… first appeared in local shows. 9……… she gave her artworks 1953 exhibition: a very old method called 10……… was used for some prints was inspired by 11……… about Chinese art that she had started collecting in 1915 Old age: still interested in art and 12………. worked for nearly six decades, making more than 13………. artworks","html":"<p>Questions 8-13 Write ONE WORD ONLY.</p>","bbox":[72,220,520,430],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 8 patterns 9 galleries 10 woodcut 11 books 12 travel 13 2000","html":"<p>Answers 8 patterns 9 galleries 10 woodcut 11 books 12 travel 13 2000</p>","bbox":[72,700,520,760],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let groups = split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0]
                .get("questionRange")
                .and_then(Value::as_array)
                .map(|range| (
                    range.first().and_then(Value::as_u64).unwrap(),
                    range.get(1).and_then(Value::as_u64).unwrap()
                )),
            Some((8, 13))
        );
        assert_eq!(
            groups[0].get("kindHint").and_then(Value::as_str),
            Some("sentence_completion")
        );
        assert_eq!(
            groups[0].get("layoutHint").and_then(Value::as_str),
            Some("inline_completion")
        );

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/groups/0/questions")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(|question| question.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["q8", "q9", "q10", "q11", "q12", "q13"]
        );
        assert_eq!(
            ir.pointer("/answerKey/q8").and_then(Value::as_str),
            Some("patterns")
        );
        assert_eq!(
            ir.pointer("/answerKey/q13").and_then(Value::as_str),
            Some("2000")
        );

        let source = reading_source(&ir);
        let body_html = source
            .pointer("/questionGroups/0/bodyHtml")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(body_html.contains("notes-completion"));
        for qid in ["q8", "q9", "q10", "q11", "q12", "q13"] {
            assert!(
                body_html.contains(&format!("name=\"{}\"", qid)),
                "missing {}",
                qid
            );
        }
        assert!(
            !body_html.contains("Questions 8-13 item 11")
                && !body_html.contains("Questions 8-13 第 11 题")
        );
    }

    #[test]
    fn cloud_outline_completion_family_diff_does_not_override_local_inline_notes() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        crate::llm_profiles::save_profiles(
            &root,
            &[json!({
                "profileId": "profile-cloud-compare",
                "name": "Cloud Compare Test",
                "provider": "OpenAiCompatible",
                "baseUrl": "http://unit.test/v1",
                "model": "unit-test",
                "temperature": 0,
                "timeoutMs": 60000,
                "forceJson": true,
                "enabled": true
            })],
        )
        .unwrap();
        let mut job = make_job(CreateJobInput {
            title: Some("Cloud Notes Compare".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["cloud-compare-regression".to_string()]),
            llm_profile_id: Some("profile-cloud-compare".to_string()),
        });
        attach_fixture_source(&root, &mut job, "no-text.pdf", "MainQuestion");
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"passage","blockType":"paragraph","text":"The artist combined several techniques across a long career.","html":"<p>The artist combined several techniques.</p>","bbox":[72,80,520,180],"confidence":0.98,"roleHint":"passage"},
                    {"blockId":"q8-13","blockType":"paragraph","text":"Questions 8-13 Complete the notes below. Write ONE WORD ONLY from the passage for each answer. Early work: 8……… first appeared in local shows. 9……… she gave her artworks 1953 exhibition: a very old method called 10……… was used for some prints was inspired by 11……… about Chinese art Old age: still interested in art and 12………. worked for nearly six decades, making more than 13………. artworks","html":"<p>Questions 8-13 Complete the notes below.</p>","bbox":[72,220,520,430],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 8 symbols 9 titles 10 stencilling 11 books 12 travel 13 400","html":"<p>Answers 8 symbols 9 titles 10 stencilling 11 books 12 travel 13 400</p>","bbox":[72,700,520,760],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();
        write_source_review_status(&root, &job.job_id, Some(&doc), true, None).unwrap();

        let report = run_auto_pipeline_core_with_gateway(
            &root,
            &job.job_id,
            Some(AutoPipelineInput {
                parse_mode: Some("auto".to_string()),
                confidence_threshold: Some(0.85),
                profile_id: Some("profile-cloud-compare".to_string()),
                execution_mode: None,
                target: Some("editableDraft".to_string()),
                allow_overwrite: None,
            }),
            |_root, _job_id, command_name, _input, _api_key| match command_name {
                "extract_pdf_image_answers" => Ok(json!({
                    "answers": {
                        "8": "symbols",
                        "9": "titles",
                        "10": "stencilling",
                        "11": "books",
                        "12": "travel",
                        "13": "400"
                    },
                    "confidence": 0.99,
                    "warnings": [],
                    "evidence": [{"questionNumber": "8", "pageIndex": 1, "quote": "8 symbols"}]
                })),
                "generate_pdf_reading_outline" => Ok(json!({
                    "title": "Cloud Notes Compare",
                    "groups": [{
                        "range": [8, 13],
                        "kind": "summary_completion",
                        "layoutHint": "list",
                        "questionIds": ["q8", "q9", "q10", "q11", "q12", "q13"],
                        "notesText": "",
                        "confidence": 0.9,
                        "evidence": {
                            "quotes": [{"pageIndex": 1, "text": "Questions 8-13 Complete the notes below"}]
                        }
                    }],
                    "answerKey": {
                        "8": "symbols",
                        "9": "titles",
                        "10": "stencilling",
                        "11": "books",
                        "12": "travel",
                        "13": "400"
                    },
                    "confidence": 0.9,
                    "warnings": []
                })),
                other => Err(format!("unexpected_command:{}", other)),
            },
        )
        .unwrap();

        let cloud = report.pointer("/quality/cloudComparison").unwrap();
        assert_eq!(cloud.get("attempted").and_then(Value::as_bool), Some(true));
        assert_eq!(cloud.get("passed").and_then(Value::as_bool), Some(true));
        assert_eq!(cloud.get("warningCount").and_then(Value::as_u64), Some(0));
        assert_eq!(
            report
                .pointer("/parser/visionAnswerExtraction/filledQuestionIds")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec!["q8", "q9", "q10", "q11", "q12", "q13"]
        );
        assert_eq!(
            report
                .pointer("/parser/visionAnswerExtraction/missingQuestionIds")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            cloud
                .pointer("/localSummary/0/questionIds")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec!["q8", "q9", "q10", "q11", "q12", "q13"]
        );
        assert_eq!(
            cloud
                .pointer("/cloudSummary/0/layoutHint")
                .and_then(Value::as_str),
            Some("list")
        );
        let issues = cloud
            .pointer("/comparison/issues")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            issues.is_empty(),
            "completion-family naming/layout differences should not become issues: {}",
            serde_json::to_string_pretty(&issues).unwrap()
        );
        let observations = cloud
            .pointer("/comparison/observations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(observations.iter().any(|item| {
            item.get("kind").and_then(Value::as_str) == Some("cloud_completion_kind_normalized")
        }));
        assert!(observations.iter().any(|item| {
            item.get("kind").and_then(Value::as_str)
                == Some("cloud_layout_deprioritized_by_local_inline_notes")
        }));
        let ir: Value = read_json(&job_dir(&root, &job.job_id).join("authoring-ir.json")).unwrap();
        assert_eq!(
            ir.pointer("/groups/0/layout/layoutHint")
                .and_then(Value::as_str),
            Some("inline_completion")
        );
        assert_eq!(
            ir.pointer("/groups/0/questions")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(|question| question.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["q8", "q9", "q10", "q11", "q12", "q13"]
        );
        assert_eq!(
            ir.pointer("/answerKey/q13").and_then(Value::as_str),
            Some("400")
        );
        let audit_issues = ir
            .pointer("/audit/issues")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(audit_issues.iter().any(|issue| {
            issue.get("kind").and_then(Value::as_str) == Some("vision_answer_extraction_summary")
                && issue
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .contains("视觉模型已从 PDF 图片页补全答案")
        }));
        let cloud_audit = audit_issues
            .iter()
            .find(|issue| {
                issue.get("kind").and_then(Value::as_str) == Some("cloud_comparison_summary")
            })
            .expect("cloud pass summary should survive artifact minimization");
        assert_eq!(
            cloud_audit.get("status").and_then(Value::as_str),
            Some("passed")
        );
        assert_eq!(
            cloud_audit.get("passed").and_then(Value::as_bool),
            Some(true)
        );
        assert!(cloud_audit
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("云端对照通过"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cloud_outline_mismatch_is_persisted_as_authoring_audit_summary() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        crate::llm_profiles::save_profiles(
            &root,
            &[json!({
                "profileId": "profile-cloud-mismatch",
                "name": "Cloud Mismatch Test",
                "provider": "OpenAiCompatible",
                "baseUrl": "http://unit.test/v1",
                "model": "unit-test",
                "temperature": 0,
                "timeoutMs": 60000,
                "forceJson": true,
                "enabled": true
            })],
        )
        .unwrap();
        let mut job = make_job(CreateJobInput {
            title: Some("Cloud Notes Mismatch".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["cloud-mismatch-regression".to_string()]),
            llm_profile_id: Some("profile-cloud-mismatch".to_string()),
        });
        attach_fixture_source(&root, &mut job, "no-text.pdf", "MainQuestion");
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"passage","blockType":"paragraph","text":"The artist combined several techniques across a long career.","html":"<p>The artist combined several techniques.</p>","bbox":[72,80,520,180],"confidence":0.98,"roleHint":"passage"},
                    {"blockId":"q8-13","blockType":"paragraph","text":"Questions 8-13 Complete the notes below. Write ONE WORD ONLY from the passage for each answer. Early work: 8……… first appeared in local shows. 9……… she gave her artworks 1953 exhibition: a very old method called 10……… was used for some prints was inspired by 11……… about Chinese art Old age: still interested in art and 12………. worked for nearly six decades, making more than 13………. artworks","html":"<p>Questions 8-13 Complete the notes below.</p>","bbox":[72,220,520,430],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 8 symbols 9 titles 10 stencilling 11 books 12 travel 13 400","html":"<p>Answers 8 symbols 9 titles 10 stencilling 11 books 12 travel 13 400</p>","bbox":[72,700,520,760],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();
        write_source_review_status(&root, &job.job_id, Some(&doc), true, None).unwrap();

        let report = run_auto_pipeline_core_with_gateway(
            &root,
            &job.job_id,
            Some(AutoPipelineInput {
                parse_mode: Some("auto".to_string()),
                confidence_threshold: Some(0.85),
                profile_id: Some("profile-cloud-mismatch".to_string()),
                execution_mode: None,
                target: Some("editableDraft".to_string()),
                allow_overwrite: None,
            }),
            |_root, _job_id, command_name, _input, _api_key| match command_name {
                "extract_pdf_image_answers" => Ok(json!({
                    "answers": {
                        "8": "symbols",
                        "9": "titles",
                        "10": "stencilling",
                        "11": "books",
                        "12": "travel",
                        "13": "400"
                    },
                    "confidence": 0.99,
                    "warnings": [],
                    "evidence": [{"questionNumber": "8", "pageIndex": 1, "quote": "8 symbols"}]
                })),
                "generate_pdf_reading_outline" => Ok(json!({
                    "title": "Cloud Notes Mismatch",
                    "groups": [{
                        "range": [8, 13],
                        "kind": "summary_completion",
                        "layoutHint": "inline_completion",
                        "questionIds": ["q8", "q9", "q10", "q11", "q12", "q13"],
                        "notesText": "Questions 8-13 Complete the notes below.",
                        "confidence": 0.9,
                        "evidence": {
                            "quotes": [{"pageIndex": 1, "text": "Questions 8-13 Complete the notes below"}]
                        }
                    }],
                    "answerKey": {
                        "8": "symbols",
                        "9": "titles",
                        "10": "stencilling",
                        "11": "books",
                        "12": "travel",
                        "13": "500"
                    },
                    "confidence": 0.9,
                    "warnings": []
                })),
                other => Err(format!("unexpected_command:{}", other)),
            },
        )
        .unwrap();

        let cloud = report.pointer("/quality/cloudComparison").unwrap();
        assert_eq!(cloud.get("attempted").and_then(Value::as_bool), Some(true));
        assert_eq!(cloud.get("passed").and_then(Value::as_bool), Some(false));
        assert_eq!(cloud.get("warningCount").and_then(Value::as_u64), Some(1));
        assert!(cloud
            .pointer("/issues/0/message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("第 13 题答案与云端对照不一致"));
        let ir: Value = read_json(&job_dir(&root, &job.job_id).join("authoring-ir.json")).unwrap();
        assert_eq!(
            ir.pointer("/answerKey/q13").and_then(Value::as_str),
            Some("400"),
            "cloud comparison must not overwrite the local authoritative draft"
        );
        let cloud_issue = ir
            .pointer("/audit/issues")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|issue| {
                issue.get("kind").and_then(Value::as_str) == Some("cloud_comparison_summary")
            })
            .expect("cloud comparison summary should survive artifact minimization");
        assert_eq!(
            cloud_issue.get("status").and_then(Value::as_str),
            Some("needs_review")
        );
        assert_eq!(
            cloud_issue
                .pointer("/cloudSummary/0/questionIds")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec!["q8", "q9", "q10", "q11", "q12", "q13"]
        );
        assert!(cloud_issue
            .pointer("/issues/0/message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("第 13 题答案与云端对照不一致"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn split_and_authoring_regeneration_refuse_to_overwrite_existing_draft_by_default() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let mut job = test_job();
        job.source_files = vec![test_source("txt")];
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();

        let doc = sample_document_ir(&job, "auto");
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        write_json(
            &job_dir(&root, &job.job_id).join("split-candidates.json"),
            &split,
        )
        .unwrap();
        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        write_json(&job_dir(&root, &job.job_id).join("authoring-ir.json"), &ir).unwrap();

        let split_error = run_rule_split_core(&root, &job.job_id, None).unwrap_err();
        assert!(
            split_error.contains("editable_draft_exists"),
            "unexpected split error: {}",
            split_error
        );
        let build_error = build_authoring_ir_core(&root, &job.job_id, None).unwrap_err();
        assert!(
            build_error.contains("editable_draft_exists"),
            "unexpected build error: {}",
            build_error
        );

        run_rule_split_core(
            &root,
            &job.job_id,
            Some(RegenerateDraftInput {
                allow_overwrite: Some(true),
            }),
        )
        .unwrap();
        build_authoring_ir_core(
            &root,
            &job.job_id,
            Some(RegenerateDraftInput {
                allow_overwrite: Some(true),
            }),
        )
        .unwrap();

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn manual_transcription_replaces_source_and_archives_previous_draft_revision() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        let mut job = test_job();
        job.source_files = vec![test_source("txt")];
        save_job(&root, &job).unwrap();
        let job_path = job_dir(&root, &job.job_id);
        ensure_job_dirs(&job_path).unwrap();

        let doc = sample_document_ir(&job, "auto");
        write_json(&job_path.join("document-ir.json"), &doc).unwrap();
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        write_json(&job_path.join("split-candidates.json"), &split).unwrap();
        let old_ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        write_json(&job_path.join("authoring-ir.json"), &old_ir).unwrap();

        apply_manual_transcription_core(
            &root,
            &job.job_id,
            ManualTranscriptionInput {
                text: "READING PASSAGE 1\nReplacement passage.\n\nQuestions 1-1\n1 Replacement text is present.\n\nAnswers\n1 TRUE".to_string(),
                note: Some("operator replacement".to_string()),
            },
        )
        .unwrap();

        assert!(
            !job_path.join("authoring-ir.json").exists(),
            "current authoring draft should be archived when the source text is replaced"
        );
        assert!(
            job_path
                .join("revisions")
                .read_dir()
                .unwrap()
                .any(|entry| entry.unwrap().path().join("authoring-ir.json").exists()),
            "previous draft should be retained as a revision"
        );

        run_rule_split_core(&root, &job.job_id, None).unwrap();
        build_authoring_ir_core(&root, &job.job_id, None).unwrap();
        let new_ir: Value = read_json(&job_path.join("authoring-ir.json")).unwrap();
        assert_eq!(
            new_ir
                .pointer("/groups/0/questions/0/answer")
                .and_then(Value::as_str),
            Some("TRUE")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn enhanced_classifier_distinguishes_matching_table_and_completion_types() {
        let job = make_job(CreateJobInput {
            title: Some("Enhanced Classifier Fixture".to_string()),
            category: Some("P3".to_string()),
            frequency: Some("hard".to_string()),
            tags: Some(vec!["classifier".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"q27-30","blockType":"paragraph","text":"Questions 27-30 The reading passage has four sections, A-D. Choose the correct heading for each section from the list of headings below. Each heading may be used once only.","html":"<p>Questions 27-30 Choose the correct heading.</p>","bbox":[72,90,520,140],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"headings","blockType":"paragraph","text":"List of Headings i Early experiments ii Commercial growth iii Public criticism iv A later decline","html":"<p>List of Headings</p>","bbox":[72,145,520,220],"confidence":0.94,"roleHint":"question"},
                    {"blockId":"q31-34","blockType":"table","text":"Questions 31-34 Complete the table below. Choose ONE WORD ONLY from the passage for each answer. Year | Event | 31 ___","html":"<table><tr><td>Year</td><td>Event</td></tr></table>","bbox":[72,240,520,330],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"q35-38","blockType":"paragraph","text":"Questions 35-38 Complete the summary below. Choose NO MORE THAN TWO WORDS from the passage for each answer. Researchers first noticed 35 ___ before recording 36 ___.","html":"<p>Questions 35-38 Complete the summary below.</p>","bbox":[72,350,520,430],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 27 i 28 ii 29 iii 30 iv 31 trial 32 growth 33 decline 34 criticism 35 movement 36 signals 37 roots 38 leaves","html":"<p>Answers</p>","bbox":[72,700,520,760],"confidence":0.90,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let kinds = split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|group| group.get("kindHint").and_then(Value::as_str).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec!["heading_matching", "table_completion", "summary_completion"]
        );
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/classification/interaction/allowOptionReuse")
                .and_then(Value::as_bool),
            Some(false)
        );
        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/groups/0/allowOptionReuse")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            ir.pointer("/groups/0/classificationEvidence/0")
                .and_then(Value::as_str),
            Some("q27-30")
        );
        let issues = authoring_review_issues(&ir);
        assert!(issues.iter().any(|issue| {
            issue
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("Question-group classification warning")
        }));
    }

    #[test]
    fn classifier_keeps_single_choice_multi_choice_and_and_ranges_distinct() {
        let job = make_job(CreateJobInput {
            title: Some("Classifier Range Regression".to_string()),
            category: Some("P3".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["classifier-regression".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"passage","blockType":"paragraph","text":"The passage describes several research findings.","html":"<p>The passage describes several research findings.</p>","bbox":[72,80,520,170],"confidence":0.98,"roleHint":"passage"},
                    {"blockId":"q27-30","blockType":"paragraph","text":"Questions 27-30 Choose the correct letter, A, B, C or D. 27 According to the writer, what is unclear about the findings? A cause B effect C cost D timing. 28 Which of the following is mentioned by the writer? A archive B practice C climate D funding. 29 What is the writer's main purpose? A explain B compare C reject D predict. 30 Which title is most suitable? A Growth B Decline C Evidence D Debate","html":"<p>Questions 27-30 Choose the correct letter.</p>","bbox":[72,190,520,300],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"q31-32","blockType":"paragraph","text":"Questions 31 and 32 Choose TWO letters, A-E. Which TWO points are made about the study? A It was repeated. B It used children. C It was expensive. D It was short. E It changed policy.","html":"<p>Questions 31 and 32 Choose TWO letters.</p>","bbox":[72,320,520,390],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"q33-36","blockType":"paragraph","text":"Questions 33-36 Look at the following opinions and the list of companies below. Match each opinion with the correct company, A, B or C. 33 Disposable waste 34 Buying policies 35 Training 36 Public information List of Companies A Alpha B Beta C Gamma","html":"<p>Questions 33-36 Match each opinion.</p>","bbox":[72,410,520,500],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 27 A 28 B 29 C 30 D 31 A 32 E 33 A 34 B 35 C 36 A","html":"<p>Answers</p>","bbox":[72,700,520,760],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let groups = split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .unwrap();
        let observed = groups
            .iter()
            .map(|group| {
                let range = group
                    .get("questionRange")
                    .and_then(Value::as_array)
                    .unwrap();
                (
                    range.first().and_then(Value::as_u64).unwrap(),
                    range.get(1).and_then(Value::as_u64).unwrap(),
                    group.get("kindHint").and_then(Value::as_str).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            observed,
            vec![
                (27, 30, "single_choice"),
                (31, 32, "multi_choice"),
                (33, 36, "matching"),
            ]
        );

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/groups/1/questions")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(|question| question.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["q31", "q32"]
        );
        assert_eq!(
            ir.pointer("/groups/0/questions/0/interaction/type")
                .and_then(Value::as_str),
            Some("radio")
        );
        assert_eq!(
            ir.pointer("/groups/1/questions/0/interaction/type")
                .and_then(Value::as_str),
            Some("checkbox")
        );
    }

    #[test]
    fn classifier_prefers_short_answer_word_limit_before_stray_option_bank() {
        let job = make_job(CreateJobInput {
            title: Some("Short Answer Stray Option Regression".to_string()),
            category: Some("P3".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["classifier-regression".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"passage","blockType":"paragraph","text":"The passage describes language strategy in multinational companies.","html":"<p>The passage describes language strategy in multinational companies.</p>","bbox":[72,80,520,170],"confidence":0.98,"roleHint":"passage"},
                    {"blockId":"q33-39","blockType":"paragraph","text":"Questions 33-39 Answer the questions below. Choose NO MORE THAN THREE WORDS AND/OR A NUMBER from the passage for each answer. 33 Which policy was introduced first? 34 What did staff receive every week? 35 Which region reported the lowest usage? 36 What did managers review monthly? 37 Which language was used in training? 38 How many offices joined the trial? 39 What was the final recommendation? A Asia B Budget C Compliance D Denmark E English F Feedback G Germany H Hiring I Induction J Japan K Knowledge-sharing L Leadership","html":"<p>Questions 33-39 Answer the questions below.</p>","bbox":[72,190,520,420],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 33 policy handbook 34 a checklist 35 Germany 36 complaints 37 English 38 12 39 shared templates","html":"<p>Answers</p>","bbox":[72,700,520,760],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/kindHint")
                .and_then(Value::as_str),
            Some("short_answer")
        );

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/groups/0/kind").and_then(Value::as_str),
            Some("short_answer")
        );
        assert_eq!(
            ir.pointer("/groups/0/layout/template")
                .and_then(Value::as_str),
            Some("short_answer_list")
        );
    }

    #[test]
    fn classifier_keeps_sentence_endings_matching_before_single_choice() {
        let job = make_job(CreateJobInput {
            title: Some("Sentence Ending Regression".to_string()),
            category: Some("P3".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["classifier-regression".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"passage","blockType":"paragraph","text":"The passage discusses literary prizes.","html":"<p>The passage discusses literary prizes.</p>","bbox":[72,80,520,170],"confidence":0.98,"roleHint":"passage"},
                    {"blockId":"q36-40-head","blockType":"paragraph","text":"Questions 36-40","html":"<p>Questions 36-40</p>","bbox":[72,190,520,210],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"q36-40-instruction","blockType":"paragraph","text":"Complete each sentence with the correct ending, A-G, below. Write the correct letter, A-G, in boxes 36-40 on your answer sheet.","html":"<p>Complete each sentence with the correct ending, A-G, below.</p>","bbox":[72,215,520,260],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"q36-40-items","blockType":"paragraph","text":"36 In ancient Greece, prizes 37 In the post-classical period, literary prizes 38 In medieval Europe, talented writers 39 The first results issued by the Nobel foundation 40 After the establishment of the Nobel prizes, other awards","html":"<p>36 In ancient Greece, prizes</p>","bbox":[72,265,520,350],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"q36-40-options","blockType":"paragraph","text":"A were supported by wealthy people. B were covered by the international press. C were intended to boost education. D were considered less significant. E were out of touch with their time. F were for different categories. G were for oral performance.","html":"<p>A were supported by wealthy people.</p>","bbox":[72,355,520,470],"confidence":0.95,"roleHint":"question"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let group = split.pointer("/questionGroupCandidates/0").unwrap();
        assert_eq!(
            group.get("kindHint").and_then(Value::as_str),
            Some("matching")
        );
        assert_eq!(
            group
                .pointer("/classification/interaction/type")
                .and_then(Value::as_str),
            Some("matching")
        );
    }

    #[test]
    fn split_normalizes_overlapping_question_ranges() {
        let job = make_job(CreateJobInput {
            title: Some("Overlapping Range Regression".to_string()),
            category: Some("P3".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["range-regression".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"passage","blockType":"paragraph","text":"The passage describes salinity research.","html":"<p>The passage describes salinity research.</p>","bbox":[72,80,520,170],"confidence":0.98,"roleHint":"passage"},
                    {"blockId":"q34-36-head","blockType":"paragraph","text":"Questions 34-36","html":"<p>Questions 34-36</p>","bbox":[72,190,520,210],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"q34-36-instruction","blockType":"paragraph","text":"Look at the list of techniques and the list of uses which follows it. Match each technique with the correct use, A, B, C or D.","html":"<p>Match each technique with the correct use.</p>","bbox":[72,215,520,260],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"q34-36-items","blockType":"paragraph","text":"34 Electromagnetic surveys 35 Radiometric analysis 36 Airborne electromagnetics List of uses A can help farmers choose the best location for plants. B can show the composition of the top layer of the ground. C can detect how far below ground the salt is. D can determine how old the salt is.","html":"<p>34 Electromagnetic surveys</p>","bbox":[72,265,520,350],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"q36-40-head","blockType":"paragraph","text":"Questions 36-40","html":"<p>Questions 36-40</p>","bbox":[72,360,520,380],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"q36-40-instruction","blockType":"paragraph","text":"Choose the correct letter, A, B, C or D. Write the correct letter in boxes 36-40 on your answer sheet.","html":"<p>Choose the correct letter.</p>","bbox":[72,385,520,430],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"q37-40-items","blockType":"paragraph","text":"37 What link does the writer make between salt and gold? A same locations B same impact C same techniques D neither is present 38 What is the process referred to? A vegetation B salt travel C trees D tracing minerals 39 According to the writer, one problem is that A concern B ignored C income D support 40 Which view is best? A hidden enemy B contained C success D groups","html":"<p>37 What link does the writer make between salt and gold?</p>","bbox":[72,435,520,560],"confidence":0.95,"roleHint":"question"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let ranges = split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|group| {
                let range = group
                    .get("questionRange")
                    .and_then(Value::as_array)
                    .unwrap();
                (
                    range.first().and_then(Value::as_u64).unwrap(),
                    range.get(1).and_then(Value::as_u64).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(ranges, vec![(34, 36), (37, 40)]);

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/groups/1/questions")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .filter_map(|question| question.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["q37", "q38", "q39", "q40"]
        );
    }

    #[test]
    fn opening_umbrella_range_is_included_without_replacing_concrete_groups() {
        let job = make_job(CreateJobInput {
            title: Some("P2 Umbrella Fixture".to_string()),
            category: Some("P2".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["umbrella-range".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"p2-header","blockType":"header","text":"READING PASSAGE 2","html":"<h2>READING PASSAGE 2</h2>","bbox":[72,60,460,88],"confidence":0.99,"roleHint":"passage"},
                    {"blockId":"p2-umbrella","blockType":"paragraph","text":"Questions 14\u{2013}26 are based on Reading Passage 2 below.","html":"<p>Questions 14\u{2013}26 are based on Reading Passage 2 below.</p>","bbox":[72,92,520,120],"confidence":0.98,"roleHint":"question"},
                    {"blockId":"p2-passage","blockType":"paragraph","text":"The passage describes how plants exchange chemical signals and adapt to nearby threats.","html":"<p>The passage describes how plants exchange chemical signals and adapt to nearby threats.</p>","bbox":[72,130,520,210],"confidence":0.97,"roleHint":"passage"},
                    {"blockId":"q14-19","blockType":"paragraph","text":"Questions 14-19 Do the following statements agree with the information given in Reading Passage 2? TRUE if the statement agrees, FALSE if it contradicts, NOT GIVEN if there is no information. 14 Plants can exchange signals. 15 All signals are visible to humans. 16 Some plants respond to nearby threats. 17 The passage says every plant is identical. 18 Chemical signals can travel between plants. 19 The writer focuses only on animals.","html":"<p>Questions 14-19 Do the following statements agree with the information given in Reading Passage 2?</p>","bbox":[72,250,520,360],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"q20-23","blockType":"paragraph","text":"Questions 20-23 Choose the correct letter, A, B, C or D. 20 Which signal is mentioned first? 21 What does the writer say about roots? 22 Which finding surprised researchers? 23 What is the main purpose of the passage?","html":"<p>Questions 20-23 Choose the correct letter, A, B, C or D.</p>","bbox":[72,370,520,480],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"q24-26","blockType":"paragraph","text":"Questions 24-26 Complete the sentences below. Choose ONE WORD ONLY from the passage for each answer. 24 Plants may release ______. 25 Nearby plants can prepare for ______. 26 Roots may share ______.","html":"<p>Questions 24-26 Complete the sentences below.</p>","bbox":[72,490,520,580],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 14 TRUE 15 FALSE 16 TRUE 17 NOT GIVEN 18 TRUE 19 FALSE 20 A 21 B 22 C 23 D 24 chemicals 25 threats 26 information","html":"<p>Answers 14 TRUE 15 FALSE 16 TRUE 17 NOT GIVEN 18 TRUE 19 FALSE 20 A 21 B 22 C 23 D 24 chemicals 25 threats 26 information</p>","bbox":[72,600,520,650],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let umbrella_ranges = split
            .get("umbrellaQuestionRanges")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(umbrella_ranges.iter().any(|item| {
            item.get("questionRange")
                .and_then(Value::as_array)
                .map(|range| {
                    range.first().and_then(Value::as_u64) == Some(14)
                        && range.get(1).and_then(Value::as_u64) == Some(26)
                })
                .unwrap_or(false)
        }));

        let groups = split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let ranges = groups
            .iter()
            .filter_map(|candidate| {
                candidate
                    .get("questionRange")
                    .and_then(Value::as_array)
                    .map(|range| {
                        (
                            range.first().and_then(Value::as_u64).unwrap_or_default(),
                            range.get(1).and_then(Value::as_u64).unwrap_or_default(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(ranges, vec![(14, 19), (20, 23), (24, 26)]);
        assert!(!groups.iter().any(|candidate| {
            candidate
                .get("requiresManualQuestionImport")
                .and_then(Value::as_bool)
                == Some(true)
        }));
        assert!(!ranges.contains(&(14, 26)));

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/passage/questionUmbrellaRanges/0/questionRange")
                .and_then(Value::as_array)
                .map(|range| {
                    (
                        range.first().and_then(Value::as_u64).unwrap_or_default(),
                        range.get(1).and_then(Value::as_u64).unwrap_or_default(),
                    )
                }),
            Some((14, 26))
        );
        assert_eq!(
            ir.pointer("/passage/questionUmbrellaRanges/0/text")
                .and_then(Value::as_str),
            Some("Questions 14\u{2013}26 are based on Reading Passage 2 below.")
        );

        let source = reading_source(&ir);
        assert_eq!(
            source
                .pointer("/meta/questionUmbrellaRanges/0/questionRange")
                .and_then(Value::as_array)
                .map(|range| {
                    (
                        range.first().and_then(Value::as_u64).unwrap_or_default(),
                        range.get(1).and_then(Value::as_u64).unwrap_or_default(),
                    )
                }),
            Some((14, 26))
        );
        assert!(source
            .pointer("/meta/questionIntroHtml")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("Questions 14-26"));
        assert_eq!(
            source
                .get("questionGroups")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(3)
        );
    }

    #[test]
    fn split_opening_bare_umbrella_heading_is_included_without_duplication() {
        let job = make_job(CreateJobInput {
            title: Some("P2 Split Umbrella Heading".to_string()),
            category: Some("P2".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["umbrella-range".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"p2-header","blockType":"header","text":"READING PASSAGE 2","html":"<h2>READING PASSAGE 2</h2>","bbox":[72,60,460,88],"confidence":0.99,"roleHint":"passage"},
                    {"blockId":"p2-umbrella","blockType":"paragraph","text":"Questions 14\u{2013}26","html":"<p>Questions 14\u{2013}26</p>","bbox":[72,92,520,120],"confidence":0.98,"roleHint":"question"},
                    {"blockId":"p2-passage","blockType":"paragraph","text":"A passage about plant communication follows this opening instruction and should remain passage content rather than question content.","html":"<p>A passage about plant communication follows this opening instruction and should remain passage content rather than question content.</p>","bbox":[72,130,520,210],"confidence":0.97,"roleHint":"passage"},
                    {"blockId":"q14-19","blockType":"paragraph","text":"Questions 14-19 Do the following statements agree with the information given in Reading Passage 2? TRUE if the statement agrees, FALSE if it contradicts, NOT GIVEN if there is no information. 14 Plants can exchange signals. 15 All signals are visible to humans. 16 Some plants respond to nearby threats. 17 The passage says every plant is identical. 18 Chemical signals can travel between plants. 19 The writer focuses only on animals.","html":"<p>Questions 14-19 Do the following statements agree with the information given in Reading Passage 2?</p>","bbox":[72,250,520,360],"confidence":0.96,"roleHint":"question"},
                    {"blockId":"q20-23","blockType":"paragraph","text":"Questions 20-23 Choose the correct letter, A, B, C or D. 20 Which signal is mentioned first? 21 What does the writer say about roots? 22 Which finding surprised researchers? 23 What is the main purpose of the passage?","html":"<p>Questions 20-23 Choose the correct letter, A, B, C or D.</p>","bbox":[72,370,520,480],"confidence":0.95,"roleHint":"question"},
                    {"blockId":"q24-26","blockType":"paragraph","text":"Questions 24-26 Complete the sentences below. Choose ONE WORD ONLY from the passage for each answer. 24 Plants may release ______. 25 Nearby plants can prepare for ______. 26 Roots may share ______.","html":"<p>Questions 24-26 Complete the sentences below.</p>","bbox":[72,490,520,580],"confidence":0.95,"roleHint":"question"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let umbrella_ranges = split
            .get("umbrellaQuestionRanges")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(umbrella_ranges.iter().any(|item| {
            item.get("questionRange")
                .and_then(Value::as_array)
                .map(|range| {
                    range.first().and_then(Value::as_u64) == Some(14)
                        && range.get(1).and_then(Value::as_u64) == Some(26)
                })
                .unwrap_or(false)
        }));

        let groups = split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let ranges = groups
            .iter()
            .filter_map(|candidate| {
                candidate
                    .get("questionRange")
                    .and_then(Value::as_array)
                    .map(|range| {
                        (
                            range.first().and_then(Value::as_u64).unwrap_or_default(),
                            range.get(1).and_then(Value::as_u64).unwrap_or_default(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(ranges, vec![(14, 19), (20, 23), (24, 26)]);
        assert!(!groups.iter().any(|candidate| {
            candidate
                .get("requiresManualQuestionImport")
                .and_then(Value::as_bool)
                == Some(true)
        }));
        assert!(!ranges.contains(&(14, 26)));
    }

    #[test]
    fn split_candidates_v1_preserves_umbrella_contract_and_manual_scaffold() {
        let job = make_job(CreateJobInput {
            title: Some("P2 Umbrella Only".to_string()),
            category: Some("P2".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["umbrella-range".to_string()]),
            llm_profile_id: None,
        });
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"p2-header","blockType":"header","text":"READING PASSAGE 2","html":"<h2>READING PASSAGE 2</h2>","bbox":[72,60,460,88],"confidence":0.99,"roleHint":"passage"},
                    {"blockId":"p2-umbrella","blockType":"paragraph","text":"Questions 14\u{2013}26","html":"<p>Questions 14\u{2013}26</p>","bbox":[72,92,520,120],"confidence":0.98,"roleHint":"question"},
                    {"blockId":"p2-passage","blockType":"paragraph","text":"This passage has only the opening total question range after extraction, so concrete prompts must be reviewed manually.","html":"<p>This passage has only the opening total question range after extraction, so concrete prompts must be reviewed manually.</p>","bbox":[72,130,520,210],"confidence":0.97,"roleHint":"passage"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 14 TRUE 15 FALSE 16 TRUE 17 NOT GIVEN 18 TRUE 19 FALSE 20 A 21 B 22 C 23 D 24 chemicals 25 threats 26 information","html":"<p>Answers 14 TRUE 15 FALSE 16 TRUE 17 NOT GIVEN 18 TRUE 19 FALSE 20 A 21 B 22 C 23 D 24 chemicals 25 threats 26 information</p>","bbox":[72,600,520,650],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let mut top_level_keys = split
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        top_level_keys.sort_unstable();
        assert_eq!(
            top_level_keys,
            vec![
                "answerKeyCandidates",
                "issues",
                "jobId",
                "passageCandidates",
                "questionGroupCandidates",
                "umbrellaQuestionRanges",
            ]
        );

        let umbrella = split
            .get("umbrellaQuestionRanges")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .unwrap();
        assert_eq!(
            umbrella.get("heading").and_then(Value::as_str),
            Some("Questions 14-26")
        );
        assert_eq!(
            umbrella.get("blockId").and_then(Value::as_str),
            Some("p2-umbrella")
        );
        assert_eq!(
            umbrella
                .get("questionRange")
                .and_then(Value::as_array)
                .map(|range| {
                    (
                        range.first().and_then(Value::as_u64).unwrap_or_default(),
                        range.get(1).and_then(Value::as_u64).unwrap_or_default(),
                    )
                }),
            Some((14, 26))
        );
        assert_eq!(
            umbrella.get("text").and_then(Value::as_str),
            Some("Questions 14\u{2013}26")
        );

        let group = split
            .get("questionGroupCandidates")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .unwrap();
        assert_eq!(
            group.get("heading").and_then(Value::as_str),
            Some("Questions 14-26")
        );
        assert_eq!(
            group.get("isUmbrellaRange").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            group
                .get("requiresManualQuestionImport")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            group.get("kindHint").and_then(Value::as_str),
            Some("short_answer")
        );
        assert!(
            (group
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or_default()
                - 0.35)
                .abs()
                < f64::EPSILON
        );

        let answers = split
            .pointer("/answerKeyCandidates/0/answers")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(answers.get("14").and_then(Value::as_str), Some("TRUE"));
        assert_eq!(answers.get("17").and_then(Value::as_str), Some("NOT GIVEN"));
        assert_eq!(
            answers.get("26").and_then(Value::as_str),
            Some("information")
        );
        assert!(split
            .get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|issue| issue
                .as_str()
                .unwrap_or_default()
                .contains("Only umbrella question range detected")));
    }

    #[test]
    fn reading_authoring_ir_v1_preserves_manual_import_contract() {
        let mut job = make_job(CreateJobInput {
            title: Some("P2 Authoring IR Contract".to_string()),
            category: Some("P2".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["typed-ir".to_string()]),
            llm_profile_id: None,
        });
        job.source_files = vec![test_source("pdf")];
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"p2-header","blockType":"header","text":"READING PASSAGE 2","html":"<h2>READING PASSAGE 2</h2>","bbox":[72,60,460,88],"confidence":0.99,"roleHint":"passage"},
                    {"blockId":"p2-umbrella","blockType":"paragraph","text":"Questions 14\u{2013}26","html":"<p>Questions 14\u{2013}26</p>","bbox":[72,92,520,120],"confidence":0.98,"roleHint":"question"},
                    {"blockId":"p2-passage","blockType":"paragraph","text":"The passage content is available but the concrete question prompts were not extracted.","html":"<p>The passage content is available but the concrete question prompts were not extracted.</p>","bbox":[72,130,520,210],"confidence":0.97,"roleHint":"passage"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 14 TRUE 15 FALSE 16 TRUE 17 NOT GIVEN 18 TRUE 19 FALSE 20 A 21 B 22 C 23 D 24 chemicals 25 threats 26 information","html":"<p>Answers 14 TRUE 15 FALSE 16 TRUE 17 NOT GIVEN 18 TRUE 19 FALSE 20 A 21 B 22 C 23 D 24 chemicals 25 threats 26 information</p>","bbox":[72,600,520,650],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let mut ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let mut top_level_keys = ir
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        top_level_keys.sort_unstable();
        assert_eq!(
            top_level_keys,
            vec![
                "answerKey",
                "audit",
                "exam",
                "groups",
                "jobId",
                "passage",
                "questionDisplayMap",
                "questionOrder",
                "schemaVersion",
            ]
        );
        assert_eq!(
            ir.get("schemaVersion").and_then(Value::as_str),
            Some("ReadingAuthoringIRV1")
        );
        assert_eq!(
            ir.pointer("/exam/category").and_then(Value::as_str),
            Some("P2")
        );
        assert_eq!(
            ir.pointer("/exam/sourceFiles/0/fileId")
                .and_then(Value::as_str),
            Some("file-test")
        );
        assert_eq!(
            ir.pointer("/passage/htmlBlocks/0/blockId")
                .and_then(Value::as_str),
            Some("passage-main")
        );
        assert!(ir
            .pointer("/passage/sourceBlockIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|block_id| block_id.as_str() == Some("p2-passage")));

        let group = ir
            .get("groups")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .unwrap();
        assert_eq!(
            group
                .get("questionRange")
                .and_then(Value::as_array)
                .map(|range| {
                    (
                        range.first().and_then(Value::as_u64).unwrap_or_default(),
                        range.get(1).and_then(Value::as_u64).unwrap_or_default(),
                    )
                }),
            Some((14, 26))
        );
        assert_eq!(
            group.get("isUmbrellaRange").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            group
                .get("requiresManualQuestionImport")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(group.get("verified").and_then(Value::as_bool), Some(false));

        let first_question = group
            .get("questions")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .unwrap();
        assert_eq!(
            first_question.get("id").and_then(Value::as_str),
            Some("q14")
        );
        assert_eq!(
            first_question.get("displayNumber").and_then(Value::as_str),
            Some("14")
        );
        assert_eq!(
            first_question.get("prompt").and_then(Value::as_str),
            Some("")
        );
        assert_eq!(
            first_question
                .get("requiresManualQuestionImport")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            ir.pointer("/answerKey/q14").and_then(Value::as_str),
            Some("TRUE")
        );
        assert_eq!(
            ir.pointer("/answerKey/q26").and_then(Value::as_str),
            Some("information")
        );
        assert_eq!(
            ir.pointer("/questionDisplayMap/q14")
                .and_then(Value::as_str),
            Some("14")
        );
        assert_eq!(
            ir.pointer("/questionDisplayMap/q26")
                .and_then(Value::as_str),
            Some("26")
        );
        assert_eq!(
            ir.get("questionOrder")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(13)
        );
        assert_eq!(
            ir.pointer("/audit/humanVerified").and_then(Value::as_bool),
            Some(false)
        );
        assert!(ir
            .pointer("/audit/issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|issue| issue
                .as_str()
                .unwrap_or_default()
                .contains("Only umbrella question range detected")));

        let needs_review = refresh_authoring_review_state(&mut ir);
        assert!(needs_review > 0);
        assert!(authoring_review_issues(&ir).iter().any(|issue| {
            issue
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("Umbrella question range requires manually imported concrete prompts")
        }));
        assert_eq!(
            validate_authoring(&job.job_id, Some(&ir))
                .get("passed")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn export_core_writes_assets_after_static_runtime_gate() {
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
        assert_eq!(
            result.get("validationPolicy").and_then(Value::as_str),
            Some("strict")
        );
        assert_eq!(
            result.get("validationOverridden").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            result.get("ignoredIssueCount").and_then(Value::as_u64),
            Some(0)
        );
        assert!(out_dir.join(format!("{}.json", exam_id)).exists());
        assert!(out_dir.join(format!("{}.js", exam_id)).exists());
        assert!(out_dir.join("manifest.js").exists());
        let report: Value = read_json(&out_dir.join("validation-report.json")).unwrap();
        assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
        assert_eq!(
            report.pointer("/runtime/mode").and_then(Value::as_str),
            Some("static-rust")
        );
        assert_eq!(
            load_job(&root, &job.job_id).unwrap().status,
            JobStatus::Cleaned
        );
        assert_eq!(
            result.pointer("/cleanup/cleaned").and_then(Value::as_bool),
            Some(true)
        );
        let job_path = job_dir(&root, &job.job_id);
        assert!(job_path.join("authoring-ir.json").exists());
        assert!(job_path.join("authoring-project.json").exists());
        assert!(job_path.join("uploads").join("publishable.pdf").exists());
        assert!(!job_path.join("cleanup-summary.json").exists());
        assert!(!job_path.join("document-ir.json").exists());
        assert!(!job_path.join("split-candidates.json").exists());
        assert!(!job_path.join("validation-report.json").exists());
        assert!(!job_path.join("publish-readiness-report.json").exists());
        assert!(!job_path.join("preview").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn strict_export_allows_missing_answers_and_ignores_historical_needs_review_status() {
        let root = temp_test_root();
        let (job, mut ir) = make_publishable_fixture(&root);
        let missing_question_ids = clear_all_authoring_answers(&mut ir);
        assert!(!missing_question_ids.is_empty());
        assert_eq!(
            ir.pointer("/audit/humanVerified").and_then(Value::as_bool),
            Some(true)
        );
        assert!(ir
            .get("answerKey")
            .and_then(Value::as_object)
            .map(serde_json::Map::is_empty)
            .unwrap_or(false));
        assert!(!authoring_review_issues(&ir).iter().any(|issue| {
            issue
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.starts_with("$.answerKey"))
        }));
        write_json(&job_dir(&root, &job.job_id).join("authoring-ir.json"), &ir).unwrap();
        update_job(&root, &job.job_id, |saved| {
            saved.status = JobStatus::NeedsReview;
        })
        .unwrap();

        let static_report = validate_authoring(&job.job_id, Some(&ir));
        assert_eq!(
            static_report.get("passed").and_then(Value::as_bool),
            Some(true)
        );
        assert!(static_report
            .get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|issue| {
                issue.get("severity").and_then(Value::as_str) == Some("warning")
                    && issue
                        .get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| path.starts_with("$.answerKey"))
            }));

        let out_dir = root.join("missing-answer-export");
        let result = export_reading_assets_core(
            &root,
            &job.job_id,
            out_dir.to_string_lossy().as_ref(),
            true,
        )
        .unwrap();

        assert_eq!(
            result.get("validationPolicy").and_then(Value::as_str),
            Some("strict")
        );
        assert_eq!(
            result.get("validationOverridden").and_then(Value::as_bool),
            Some(false)
        );
        let report: Value = read_json(&out_dir.join("validation-report.json")).unwrap();
        assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
        assert!(report
            .get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .all(|issue| issue.get("severity").and_then(Value::as_str) != Some("error")));
        let exam_id = result.get("examId").and_then(Value::as_str).unwrap();
        let source: Value = read_json(&out_dir.join(format!("{}.json", exam_id))).unwrap();
        assert!(source
            .get("answerKey")
            .and_then(Value::as_object)
            .map(serde_json::Map::is_empty)
            .unwrap_or(false));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn force_export_bypasses_publish_validation_and_records_override() {
        let root = temp_test_root();
        let (job, mut ir) = make_publishable_fixture(&root);
        ir.pointer_mut("/audit/humanVerified")
            .map(|value| *value = json!(false));
        write_json(&job_dir(&root, &job.job_id).join("authoring-ir.json"), &ir).unwrap();
        let out_dir = root.join("forced-export");
        let options = ExportValidationOptions::from_policy(Some("force")).unwrap();

        let result = export_reading_assets_with_options_core(
            &root,
            &job.job_id,
            out_dir.to_string_lossy().as_ref(),
            true,
            options,
        )
        .unwrap();

        assert_eq!(
            result.get("validationPolicy").and_then(Value::as_str),
            Some("force")
        );
        assert_eq!(
            result.get("validationOverridden").and_then(Value::as_bool),
            Some(true)
        );
        assert!(result
            .get("ignoredIssueCount")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0));
        let ignored_issues = result
            .get("ignoredIssues")
            .and_then(Value::as_array)
            .expect("force export ignored issues");
        assert!(ignored_issues.iter().any(|issue| {
            issue.get("jobId").and_then(Value::as_str) == Some(job.job_id.as_str())
                && issue.get("severity").and_then(Value::as_str) == Some("error")
                && issue.get("path").and_then(Value::as_str) == Some("$.audit.humanVerified")
                && issue.get("issueId").and_then(Value::as_str).is_some()
                && issue.get("message").and_then(Value::as_str).is_some()
        }));
        assert_eq!(
            result
                .pointer("/exportSummary/validationOverridden")
                .and_then(Value::as_bool),
            Some(true)
        );
        let report: Value = read_json(&out_dir.join("validation-report.json")).unwrap();
        assert_eq!(report.get("passed").and_then(Value::as_bool), Some(false));
        assert!(report
            .get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(
                |issue| issue.get("path").and_then(Value::as_str) == Some("$.audit.humanVerified")
            ));
        let project: Value =
            read_json(&job_dir(&root, &job.job_id).join("authoring-project.json")).unwrap();
        assert!(project
            .pointer("/exportSummary/ignoredIssues")
            .and_then(Value::as_array)
            .is_some_and(|issues| !issues.is_empty()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn force_policy_is_honored_by_js_and_nas_exports() {
        let root = temp_test_root();
        let (js_job, mut js_ir) = make_publishable_fixture(&root);
        let (nas_job, mut nas_ir) = make_publishable_fixture(&root);
        for (job_id, ir) in [
            (js_job.job_id.as_str(), &mut js_ir),
            (nas_job.job_id.as_str(), &mut nas_ir),
        ] {
            ir["audit"]["humanVerified"] = json!(false);
            write_json(&job_dir(&root, job_id).join("authoring-ir.json"), ir).unwrap();
        }

        let js_result = export_reading_js_core(
            &root,
            &json!({
                "jobIds": [js_job.job_id],
                "exportDir": root.join("forced-js").to_string_lossy(),
                "validationPolicy": "force"
            }),
            true,
        )
        .unwrap();
        let nas_result = export_nas_library_core(
            &root,
            &json!({
                "jobIds": [nas_job.job_id],
                "exportDir": root.join("forced-nas").to_string_lossy(),
                "validationPolicy": "force"
            }),
            true,
        )
        .unwrap();
        for result in [&js_result, &nas_result] {
            assert_eq!(
                result.get("validationPolicy").and_then(Value::as_str),
                Some("force")
            );
            assert_eq!(
                result.get("validationOverridden").and_then(Value::as_bool),
                Some(true)
            );
            assert!(result
                .get("ignoredIssues")
                .and_then(Value::as_array)
                .is_some_and(|issues| !issues.is_empty()));
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn force_export_does_not_bypass_unsafe_exam_id() {
        let root = temp_test_root();
        let (job, mut ir) = make_publishable_fixture(&root);
        ir["exam"]["examId"] = json!("../unsafe-exam");
        ir["audit"]["humanVerified"] = json!(false);
        write_json(&job_dir(&root, &job.job_id).join("authoring-ir.json"), &ir).unwrap();
        let out_dir = root.join("unsafe-forced-export");
        let options = ExportValidationOptions::from_policy(Some("force")).unwrap();

        let error = export_reading_assets_with_options_core(
            &root,
            &job.job_id,
            out_dir.to_string_lossy().as_ref(),
            true,
            options,
        )
        .unwrap_err();

        assert!(error.contains("invalid_exam_id_path_segment"));
        assert!(!out_dir.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_core_publish_gate_failure_writes_no_export_or_cleanup() {
        let root = temp_test_root();
        let (job, mut ir) = make_publishable_fixture(&root);
        if let Some(audit) = ir.get_mut("audit").and_then(Value::as_object_mut) {
            audit.insert("humanVerified".to_string(), json!(false));
        }
        write_json(&job_dir(&root, &job.job_id).join("authoring-ir.json"), &ir).unwrap();
        let out_dir = root.join("blocked-export");

        let error = export_reading_assets_core(
            &root,
            &job.job_id,
            out_dir.to_string_lossy().as_ref(),
            true,
        )
        .unwrap_err();

        assert!(error.contains("export_validation_failed"));
        assert!(error.contains("$.audit.humanVerified"));
        assert!(!out_dir.exists());
        let job_path = job_dir(&root, &job.job_id);
        assert!(!job_path.join("cleanup-summary.json").exists());
        assert!(job_path.join("authoring-project.json").exists());
        assert!(!job_path.join("document-ir.json").exists());
        assert!(!job_path.join("split-candidates.json").exists());
        assert!(!job_path.join("publish-readiness-report.json").exists());
        assert_eq!(
            load_job(&root, &job.job_id).unwrap().status,
            JobStatus::Working
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_reading_js_core_writes_single_js_and_manifest() {
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let expected_exam_id = ir
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let out_dir = root.join("manual-js-export");
        let input = json!({
            "jobIds": [job.job_id],
            "exportDir": out_dir.to_string_lossy()
        });

        let result = export_reading_js_core(&root, &input, true).unwrap();

        assert_eq!(result.get("mode").and_then(Value::as_str), Some("single"));
        assert_eq!(
            result.get("validationPolicy").and_then(Value::as_str),
            Some("strict")
        );
        assert_eq!(
            result
                .get("examIds")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(vec![expected_exam_id.as_str()])
        );
        assert!(out_dir.join(format!("{}.js", expected_exam_id)).exists());
        assert!(out_dir.join("manifest.js").exists());
        assert_eq!(
            load_job(&root, &job.job_id).unwrap().status,
            JobStatus::Cleaned
        );
        assert_eq!(
            result
                .pointer("/cleanup/0/cleaned")
                .and_then(Value::as_bool),
            Some(true)
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_reading_js_core_writes_batch_js_files() {
        let root = temp_test_root();
        let (job_a, ir_a) = make_publishable_fixture(&root);
        let exam_a = ir_a
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();

        let (job_b, ir_b) = make_publishable_fixture(&root);
        let exam_b = ir_b
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let out_dir = root.join("batch-js-export");
        let input = json!({
            "jobIds": [job_a.job_id, job_b.job_id],
            "exportDir": out_dir.to_string_lossy()
        });

        let result = export_reading_js_core(&root, &input, true).unwrap();

        assert_eq!(result.get("mode").and_then(Value::as_str), Some("batch"));
        assert!(out_dir.join(format!("{}.js", exam_a)).exists());
        assert!(out_dir.join(format!("{}.js", exam_b)).exists());
        assert!(out_dir.join("manifest.js").exists());
        assert_eq!(
            result.get("files").and_then(Value::as_array).map(Vec::len),
            Some(3)
        );
        assert_eq!(
            load_job(&root, &job_a.job_id).unwrap().status,
            JobStatus::Cleaned
        );
        assert_eq!(
            load_job(&root, &job_b.job_id).unwrap().status,
            JobStatus::Cleaned
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_nas_library_core_writes_nas_js_direct_artifacts() {
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let expected_exam_id = ir
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let library_root = root.join("nas-library");
        let input = json!({
            "jobIds": [job.job_id],
            "exportDir": library_root.to_string_lossy(),
            "version": "2026.06.05-1"
        });

        let result = export_nas_library_core(&root, &input, true).unwrap();

        assert_eq!(
            result.get("mode").and_then(Value::as_str),
            Some("nas-library")
        );
        assert_eq!(result.get("assetCount").and_then(Value::as_u64), Some(1));
        let reading_exams_dir = library_root.clone();
        assert_eq!(
            result.get("validationPolicy").and_then(Value::as_str),
            Some("strict")
        );
        let reading_exams_dir = nas_reading_exams_dir(&library_root);
        assert!(reading_exams_dir
            .join(format!("{}.js", expected_exam_id))
            .exists());
        assert!(reading_exams_dir.join("manifest.js").exists());
        assert!(!library_root.join("publish").join("library.db").exists());
        assert!(!library_root.join("source").exists());
        let manifest = fs::read_to_string(reading_exams_dir.join("manifest.js")).unwrap();
        assert!(manifest.contains("__READING_EXAM_MANIFEST__"));
        assert!(manifest.contains(&expected_exam_id));
        let wrapper =
            fs::read_to_string(reading_exams_dir.join(format!("{}.js", expected_exam_id))).unwrap();
        assert!(wrapper.contains("__READING_EXAM_DATA__.register"));
        assert_eq!(
            result.get("files").and_then(Value::as_array).map(Vec::len),
            Some(2)
        );
        assert_eq!(
            result
                .pointer("/report/summary/runtime")
                .and_then(Value::as_str),
            Some("nas-js-direct")
        );
        assert_eq!(
            result
                .pointer("/exportSummary/readingExamsDir")
                .and_then(Value::as_str),
            Some(reading_exams_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            load_job(&root, &job.job_id).unwrap().status,
            JobStatus::Cleaned
        );
        assert_eq!(
            result
                .pointer("/cleanup/0/cleaned")
                .and_then(Value::as_bool),
            Some(true)
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_nas_library_from_source_tree_writes_sqlite_artifacts() {
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let expected_exam_id = ir
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let library_root = root.join("nas-library");
        let source_dir = library_root.join("source");
        write_publishable_nas_source(&root, &job, &ir, &source_dir);

        let result =
            publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-1")).unwrap();

        assert_eq!(
            result.get("mode").and_then(Value::as_str),
            Some("nas-library")
        );
        assert_eq!(result.get("assetCount").and_then(Value::as_u64), Some(1));
        assert!(library_root
            .join("source")
            .join("assets")
            .join(&expected_exam_id)
            .join("publishable.pdf")
            .exists());
        assert!(library_root.join("publish").join("library.db").exists());
        assert!(library_root
            .join("publish")
            .join("library.version.json")
            .exists());
        assert!(library_root
            .join("publish")
            .join("library.db.sha256")
            .exists());
        assert!(library_root.join("publish").join("report.json").exists());
        let report: Value = read_json(&library_root.join("publish").join("report.json")).unwrap();
        assert_eq!(report.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(
            report
                .pointer("/summary/assetCountAfter")
                .and_then(Value::as_u64),
            Some(1)
        );
        let version_payload: Value =
            read_json(&library_root.join("publish").join("library.version.json")).unwrap();
        assert_eq!(
            version_payload.get("version").and_then(Value::as_str),
            Some("2026.06.05-1")
        );
        let connection =
            rusqlite::Connection::open(library_root.join("publish").join("library.db")).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM reading_assets", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let stored_exam_id: String = connection
            .query_row("SELECT exam_id FROM reading_assets LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(stored_exam_id, expected_exam_id);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_nas_library_from_source_tree_preserves_last_good_db_on_failure() {
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let library_root = root.join("nas-library");
        let source_dir = library_root.join("source");
        write_publishable_nas_source(&root, &job, &ir, &source_dir);

        publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-1")).unwrap();
        let db_path = library_root.join("publish").join("library.db");
        let before = fs::read(&db_path).unwrap();
        write_text(
            &library_root.join("source").join("broken.js"),
            "not a valid register payload",
        )
        .unwrap();

        let error =
            publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-2")).unwrap_err();

        assert!(error.contains("nas_publish_failed"));
        assert!(error.contains("reading_asset_parse_failed"));
        let after = fs::read(&db_path).unwrap();
        assert_eq!(before, after);
        let report: Value = read_json(&library_root.join("publish").join("report.json")).unwrap();
        assert_eq!(report.get("status").and_then(Value::as_str), Some("failed"));
        assert!(
            report
                .pointer("/summary/failed")
                .and_then(Value::as_u64)
                .unwrap_or_default()
                >= 1
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_nas_library_from_source_tree_rejects_duplicate_exam_id() {
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let library_root = root.join("nas-library");
        let source_dir = library_root.join("source");
        write_publishable_nas_source(&root, &job, &ir, &source_dir);
        publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-dup")).unwrap();
        let duplicate = reading_source(&ir);
        write_nas_source_fixture(&source_dir, "duplicate-exam.js", &duplicate);

        let error = publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-dup2"))
            .unwrap_err();

        assert!(error.contains("nas_publish_failed"));
        assert!(error.contains("duplicate_exam_id"));
        let report: Value = read_json(&library_root.join("publish").join("report.json")).unwrap();
        assert_eq!(report.get("status").and_then(Value::as_str), Some("failed"));
        assert!(report_has_error_code(&report, "duplicate_exam_id"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_nas_library_from_source_tree_rejects_missing_answer_key_coverage() {
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let library_root = root.join("nas-library");
        let source_dir = library_root.join("source");
        write_publishable_nas_source(&root, &job, &ir, &source_dir);
        publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-answer")).unwrap();
        let mut invalid = reading_source(&ir);
        invalid
            .as_object_mut()
            .unwrap()
            .insert("examId".to_string(), json!("missing-answer"));
        let first_question = invalid
            .get("questionOrder")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        invalid
            .get_mut("answerKey")
            .and_then(Value::as_object_mut)
            .unwrap()
            .remove(&first_question);
        write_nas_source_fixture(&source_dir, "missing-answer.js", &invalid);

        let error = publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-answer2"))
            .unwrap_err();

        assert!(error.contains("nas_publish_failed"));
        assert!(error.contains("answer_key_missing_question"));
        let report: Value = read_json(&library_root.join("publish").join("report.json")).unwrap();
        assert_eq!(report.get("status").and_then(Value::as_str), Some("failed"));
        assert!(report_has_error_code(
            &report,
            "answer_key_missing_question"
        ));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_nas_library_from_source_tree_rejects_unsafe_html() {
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let library_root = root.join("nas-library");
        let source_dir = library_root.join("source");
        write_publishable_nas_source(&root, &job, &ir, &source_dir);
        publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-html")).unwrap();
        let mut invalid = reading_source(&ir);
        invalid
            .as_object_mut()
            .unwrap()
            .insert("examId".to_string(), json!("unsafe-html"));
        *invalid
            .pointer_mut("/questionGroups/0/bodyHtml")
            .expect("question group bodyHtml should exist") =
            Value::String("<div onclick=\"alert('x')\">bad</div>".to_string());
        write_nas_source_fixture(&source_dir, "unsafe-html.js", &invalid);

        let error = publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-html2"))
            .unwrap_err();

        assert!(error.contains("nas_publish_failed"));
        assert!(error.contains("unsafe_html_inline_handler"));
        let report: Value = read_json(&library_root.join("publish").join("report.json")).unwrap();
        assert_eq!(report.get("status").and_then(Value::as_str), Some("failed"));
        assert!(report_has_error_code(&report, "unsafe_html_inline_handler"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_nas_library_from_source_tree_rejects_invalid_meta_and_missing_resource() {
        let root = temp_test_root();
        let (job, ir) = make_publishable_fixture(&root);
        let library_root = root.join("nas-library");
        let source_dir = library_root.join("source");
        write_publishable_nas_source(&root, &job, &ir, &source_dir);
        publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-meta")).unwrap();
        let mut invalid = reading_source(&ir);
        invalid
            .as_object_mut()
            .unwrap()
            .insert("examId".to_string(), json!("invalid-meta"));
        let meta = invalid
            .get_mut("meta")
            .and_then(Value::as_object_mut)
            .unwrap();
        meta.insert("category".to_string(), json!("PX"));
        meta.insert("frequency".to_string(), json!("rare"));
        meta.insert("pdfFilename".to_string(), json!("assets/not-found.pdf"));
        write_nas_source_fixture(&source_dir, "invalid-meta.js", &invalid);

        let error = publish_nas_library_from_source_tree(&library_root, Some("2026.06.05-meta2"))
            .unwrap_err();

        assert!(error.contains("nas_publish_failed"));
        assert!(error.contains("invalid_category"));
        assert!(error.contains("invalid_frequency"));
        assert!(error.contains("missing_resource_reference"));
        let report: Value = read_json(&library_root.join("publish").join("report.json")).unwrap();
        assert_eq!(report.get("status").and_then(Value::as_str), Some("failed"));
        assert!(report_has_error_code(&report, "invalid_category"));
        assert!(report_has_error_code(&report, "invalid_frequency"));
        assert!(report_has_error_code(&report, "missing_resource_reference"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_nas_library_cli_generates_nas_js_direct_files_from_pdf() {
        let library_root = temp_test_root().join("nas-cli-library");
        let fixture = parser_fixture("complex-reading.pdf");
        let args = vec![
            "--export-nas-library".to_string(),
            fixture.to_string_lossy().to_string(),
            "--export-dir".to_string(),
            library_root.to_string_lossy().to_string(),
            "--version".to_string(),
            "2026.06.05-cli-test".to_string(),
        ];

        let handled = run_cli(&args).unwrap();

        assert!(handled);
        assert!(!library_root.join("source").exists());
        assert!(!library_root.join("publish").join("library.db").exists());
        let reading_exams_dir = nas_reading_exams_dir(&library_root);
        assert!(reading_exams_dir.join("manifest.js").exists());
        let exam_files = fs::read_dir(&reading_exams_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("js")
                    && path.file_name().and_then(|value| value.to_str()) != Some("manifest.js")
            })
            .collect::<Vec<_>>();
        assert_eq!(exam_files.len(), 1);
        let manifest = fs::read_to_string(reading_exams_dir.join("manifest.js")).unwrap();
        assert!(manifest.contains("__READING_EXAM_MANIFEST__"));

        let _ = fs::remove_dir_all(library_root.parent().unwrap_or_else(|| Path::new("/tmp")));
    }

    #[test]
    fn consecutive_nas_cli_subset_exports_merge_existing_manifest() {
        let test_root = temp_test_root();
        let library_root = test_root.join("nas-cli-library");
        let export_args = vec![
            "--export-dir".to_string(),
            library_root.to_string_lossy().to_string(),
            "--version".to_string(),
            "2026.07.16-cli-merge".to_string(),
        ];
        let first_fixture = parser_fixture("complex-reading.pdf");
        let second_fixture = parser_fixture("demanding-reading-passage-3.pdf");

        let first = export_nas_library_from_pdf_paths(&[first_fixture], &export_args).unwrap();
        let first_exam_id = first
            .get("examId")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        assert_eq!(
            first.get("manifestAssetCount").and_then(Value::as_u64),
            Some(1)
        );

        let second = export_nas_library_from_pdf_paths(&[second_fixture], &export_args).unwrap();
        let second_exam_id = second
            .get("examId")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        assert_ne!(first_exam_id, second_exam_id);
        assert_eq!(
            second.get("manifestAssetCount").and_then(Value::as_u64),
            Some(2)
        );

        let reading_exams_dir = nas_reading_exams_dir(&library_root);
        let manifest = fs::read_to_string(reading_exams_dir.join("manifest.js")).unwrap();
        assert!(manifest.contains(&first_exam_id));
        assert!(manifest.contains(&second_exam_id));
        assert!(reading_exams_dir
            .join(format!("{first_exam_id}.js"))
            .exists());
        assert!(reading_exams_dir
            .join(format!("{second_exam_id}.js"))
            .exists());

        let _ = fs::remove_dir_all(test_root);
    }

    #[test]
    fn cleanup_respects_diagnostics_artifact_retention() {
        let root = temp_test_root();
        let (job, _) = make_publishable_fixture(&root);
        write_diagnostics_settings(
            &root,
            &DiagnosticsSettings {
                keep_full_process_artifacts: true,
            },
        )
        .unwrap();
        let job_path = job_dir(&root, &job.job_id);
        write_json(
            &job_path.join("validation-report.json"),
            &json!({"passed": true}),
        )
        .unwrap();
        let summary = cleanup_transient_job_artifacts(
            &root,
            &job.job_id,
            json!({"type": "test-export", "exportedAt": Utc::now().to_rfc3339()}),
        )
        .unwrap();

        assert_eq!(summary.get("cleaned").and_then(Value::as_bool), Some(false));
        assert!(job_path.join("cleanup-summary.json").exists());
        assert!(job_path.join("document-ir.json").exists());
        assert!(job_path.join("split-candidates.json").exists());
        assert!(job_path.join("validation-report.json").exists());
        assert_eq!(
            load_job(&root, &job.job_id).unwrap().status,
            JobStatus::Working
        );

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
        let fixture = parser_fixture(file_name);
        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &fixture,
            &output,
            "auto",
        )
        .unwrap();

        if file_type == "pdf" {
            assert_pdf_parser_provenance(&doc, &fixture);
        } else {
            assert_eq!(
                doc.pointer("/parser/provider").and_then(Value::as_str),
                Some(provider)
            );
            assert!(parser_warnings(Some(&doc)).is_empty());
        }
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
        assert_complex_fixture_pipeline("complex-reading.pdf", "rust-parser:pdf:pdf-extract");
    }

    #[test]
    fn complex_txt_fixture_reaches_authoring_ir() {
        assert_complex_fixture_pipeline("complex-reading.txt", "rust-parser:text:plain");
    }

    #[test]
    fn complex_markdown_fixture_reaches_authoring_ir() {
        assert_complex_fixture_pipeline("complex-reading.md", "rust-parser:text:markdown");
    }

    #[test]
    fn complex_docx_fixture_reaches_authoring_ir() {
        assert_complex_fixture_pipeline("complex-reading.docx", "rust-parser:docx:ooxml");
    }

    /// Regression test for the demanding DOCX fixture that marks structure ONLY
    /// via run-level formatting (bold/centered/font-size) — no Heading styles,
    /// no numbering, no tables. Verifies the parser now captures `runFormat`
    /// and infers synthetic heading levels so passage sub-headings and the
    /// centered passage title are recognized.
    #[test]
    fn demanding_docx_run_format_detects_bold_subheadings_and_title() {
        let mut job = test_job();
        job.source_files = vec![test_source("docx")];
        let output = env::temp_dir().join(format!(
            "epic8-demanding-docx-{}.json",
            Uuid::new_v4().simple()
        ));
        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &parser_fixture("demanding-reading-passage-1.docx"),
            &output,
            "auto",
        )
        .unwrap();
        let _ = fs::remove_file(&output);

        // Collect every block's text + runFormat + headingLevel.
        let mut blocks: Vec<(String, Option<Value>, Option<u64>)> = Vec::new();
        for page in doc
            .get("pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for block in page
                .get("blocks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let run_format = block.pointer("/layoutHints/runFormat").cloned();
                let heading_level = block
                    .pointer("/layoutHints/headingLevel")
                    .and_then(Value::as_u64);
                blocks.push((text, run_format, heading_level));
            }
        }

        let opening_instruction = blocks
            .iter()
            .find(|(text, _, _)| text.starts_with("You should spend"))
            .map(|(text, _, _)| text.as_str())
            .expect("opening IELTS instruction should be extracted");
        assert!(opening_instruction.contains("spend about 20 minutes"));
        assert!(opening_instruction.contains("minutes on Questions 1-13"));
        assert!(!opening_instruction.contains("spendabout"));

        // The centered, bold, largest-font passage title must be a heading.
        let title = blocks
            .iter()
            .find(|(text, _, _)| text == "The history of lighting");
        assert!(title.is_some(), "passage title block should be present");
        let (_, title_fmt, title_hl) = title.unwrap();
        let title_fmt = title_fmt.as_ref().expect("title must have runFormat");
        assert_eq!(
            title_fmt.get("bold").and_then(Value::as_bool),
            Some(true),
            "title must be bold"
        );
        assert_eq!(
            title_fmt.get("centered").and_then(Value::as_bool),
            Some(true),
            "title must be centered"
        );
        assert_eq!(
            *title_hl,
            Some(2),
            "centered title should be heading level 2"
        );

        // Each bold sub-heading should be detected as a heading (level 3).
        for subheading in ["Candlelight", "Oil lamps", "Gas lighting", "Electricity"] {
            let found = blocks.iter().find(|(text, _, _)| text == subheading);
            assert!(
                found.is_some(),
                "sub-heading {} should be present",
                subheading
            );
            let (_, fmt, hl) = found.unwrap();
            let fmt = fmt.as_ref().expect("sub-heading must have runFormat");
            assert_eq!(
                fmt.get("bold").and_then(Value::as_bool),
                Some(true),
                "sub-heading {} must be bold",
                subheading
            );
            assert_eq!(
                *hl,
                Some(3),
                "sub-heading {} should be heading level 3",
                subheading
            );
        }

        // Body paragraphs must NOT be flagged as headings.
        let body = blocks
            .iter()
            .find(|(text, _, _)| text.starts_with("We forget how painfully dark"));
        assert!(body.is_some(), "a body paragraph should be present");
        let (_, body_fmt, body_hl) = body.unwrap();
        if let Some(fmt) = body_fmt.as_ref() {
            assert_ne!(
                fmt.get("bold").and_then(Value::as_bool),
                Some(true),
                "body paragraph must not be bold"
            );
        }
        assert_eq!(*body_hl, None, "body paragraph must not be a heading");
    }

    /// Regression test for the demanding PDF fixture (2-column passage, A-H
    /// option grid, mixed blank markers, no answer key). When the pdfium
    /// library is available, verifies the parser yields REAL coordinates (not
    /// the fabricated [72, *, 520, *] envelope) and that the 2-column passage
    /// page is correctly split into left/right columns. Skips gracefully when
    /// pdfium is not bundled (e.g. CI without the binary).
    #[test]
    fn demanding_pdf_pdfium_yields_real_coordinates_and_column_split() {
        if crate::pdf_geometry::pdfium_library_path().is_none() {
            // pdfium binary not available in this environment — skip.
            return;
        }
        let mut job = test_job();
        job.source_files = vec![test_source("pdf")];
        let output = env::temp_dir().join(format!(
            "epic8-demanding-pdf-{}.json",
            Uuid::new_v4().simple()
        ));
        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &parser_fixture("demanding-reading-passage-3.pdf"),
            &output,
            "auto",
        )
        .unwrap();
        let _ = fs::remove_file(&output);

        // Provider must be pdfium (real coordinates), not the text-layer fallback.
        let provider = doc
            .pointer("/parser/provider")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert_eq!(
            provider, "rust-parser:pdf:pdfium",
            "pdfium backend should be used when the library is available"
        );

        // At least one block must escape the fabricated bbox envelope
        // [72, *, 520, *] — i.e. have a real x1 > 520.5 or real x0 < 71.5.
        let mut has_real_coord = false;
        let mut page0_has_right_column = false;
        for page in doc
            .get("pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let page_index = page.get("pageIndex").and_then(Value::as_u64).unwrap_or(0);
            for block in page
                .get("blocks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(bbox) = block.get("bbox").and_then(Value::as_array) {
                    let x0 = bbox.first().and_then(Value::as_f64).unwrap_or(0.0);
                    let x1 = bbox.get(2).and_then(Value::as_f64).unwrap_or(0.0);
                    if x1 > 520.5 || x0 < 71.5 {
                        has_real_coord = true;
                    }
                    // On the 2-column passage page, a right-column block has
                    // x0 clearly past the gutter (> 300 on a 612pt page).
                    if page_index == 1 && x0 > 300.0 {
                        page0_has_right_column = true;
                    }
                }
            }
        }
        assert!(
            has_real_coord,
            "pdfium must produce real coordinates, not the fabricated envelope"
        );
        assert!(
            page0_has_right_column,
            "the 2-column passage page must contain right-column blocks (column split working)"
        );
    }

    #[test]
    fn demanding_pdf_split_keeps_cross_page_passage_before_question_pages() {
        if crate::pdf_geometry::pdfium_library_path().is_none() {
            return;
        }
        let mut job = test_job();
        job.source_files = vec![test_source("pdf")];
        let output = env::temp_dir().join(format!(
            "epic8-demanding-pdf-split-{}.json",
            Uuid::new_v4().simple()
        ));
        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &parser_fixture("demanding-reading-passage-3.pdf"),
            &output,
            "auto",
        )
        .unwrap();
        let _ = fs::remove_file(&output);

        assert_eq!(
            doc.pointer("/parser/provider").and_then(Value::as_str),
            Some("rust-parser:pdf:pdfium"),
            "cross-page regression should run against the pdfium path"
        );

        let blocks = dynamic_document_blocks(Some(&doc));
        let lookup_block = |block_id: &str| {
            blocks
                .iter()
                .find(|block| block.get("blockId").and_then(Value::as_str) == Some(block_id))
        };
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));

        let passage_ids = split
            .pointer("/passageCandidates/0/range")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let passage_pages = passage_ids
            .iter()
            .filter_map(|block_id| {
                lookup_block(block_id)
                    .and_then(|block| block.get("pageIndex").and_then(Value::as_u64))
            })
            .collect::<Vec<_>>();
        assert!(
            passage_pages.contains(&2),
            "passage range should retain at least one continuation block from page 2"
        );
        assert!(
            passage_pages.iter().copied().max().unwrap_or(0) <= 3,
            "passage recovery should stop before the question-only page 3 (pdfium may keep a page-3 header/prose block)"
        );

        let passage_text = passage_ids
            .iter()
            .filter_map(|block_id| lookup_block(block_id))
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            passage_text.to_lowercase().contains("household artefacts")
                || passage_text.to_lowercase().contains("critical awareness"),
            "recovered passage text should include page 2 continuation prose"
        );

        let first_group_ids = split
            .pointer("/questionGroupCandidates/0/blockIds")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let first_group_pages = first_group_ids
            .iter()
            .filter_map(|block_id| {
                lookup_block(block_id)
                    .and_then(|block| block.get("pageIndex").and_then(Value::as_u64))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            first_group_pages.iter().copied().min(),
            Some(3),
            "first concrete question group should begin on page 3 after the cross-page passage"
        );
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/questionRange/0")
                .and_then(Value::as_u64),
            Some(27)
        );
    }

    #[test]
    fn docx_ooxml_parser_preserves_table_ir_for_split_evidence() {
        let mut job = test_job();
        job.source_files = vec![test_source("docx")];
        let output = env::temp_dir().join(format!(
            "epic8-docx-table-ir-{}.json",
            Uuid::new_v4().simple()
        ));
        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &parser_fixture("complex-reading.docx"),
            &output,
            "auto",
        )
        .unwrap();

        let table_block = doc
            .pointer("/pages/0/blocks")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|block| block.get("blockType").and_then(Value::as_str) == Some("table"))
            .expect("complex DOCX fixture should contain a table block");
        assert!(table_block
            .pointer("/table/cells")
            .and_then(Value::as_array)
            .map(|cells| !cells.is_empty())
            .unwrap_or(false));
        assert!(
            table_block
                .pointer("/table/rows")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                >= 2
        );
        assert!(
            table_block
                .pointer("/table/cols")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                >= 2
        );

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert!(split
            .pointer("/questionGroupCandidates")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .flat_map(|group| {
                group
                    .get("sectionEvidence")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .any(|evidence| evidence
                .get("tableRows")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                >= 2));

        let _ = fs::remove_file(output);
    }

    #[test]
    fn docx_ooxml_parser_preserves_table_cell_span_metadata() {
        let mut job = test_job();
        job.source_files = vec![test_source("docx")];
        let docx_path = env::temp_dir().join(format!(
            "epic8-docx-table-span-{}.docx",
            Uuid::new_v4().simple()
        ));
        let output = env::temp_dir().join(format!(
            "epic8-docx-table-span-{}.json",
            Uuid::new_v4().simple()
        ));
        write_minimal_docx(
            &docx_path,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>READING PASSAGE 1</w:t></w:r></w:p>
    <w:p><w:r><w:t>Questions 1-2 Complete the table below.</w:t></w:r></w:p>
    <w:tbl>
      <w:tr>
        <w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>Feature group</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>Answer</w:t></w:r></w:p></w:tc>
      </w:tr>
      <w:tr>
        <w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>Growth</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>1 ____</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>rapid</w:t></w:r></w:p></w:tc>
      </w:tr>
      <w:tr>
        <w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p><w:r><w:t></w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>2 ____</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>slow</w:t></w:r></w:p></w:tc>
      </w:tr>
    </w:tbl>
    <w:p><w:r><w:t>Answers 1 rapid 2 slow</w:t></w:r></w:p>
  </w:body>
</w:document>"#,
        );

        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &docx_path,
            &output,
            "auto",
        )
        .unwrap();

        let table_block = doc
            .pointer("/pages/0/blocks")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|block| block.get("blockType").and_then(Value::as_str) == Some("table"))
            .expect("generated DOCX should contain a table block");
        assert_eq!(
            table_block.pointer("/table/cols").and_then(Value::as_u64),
            Some(3)
        );
        let cells = table_block
            .pointer("/table/cells")
            .and_then(Value::as_array)
            .unwrap();
        assert!(
            cells.iter().any(|cell| {
                cell.get("text").and_then(Value::as_str) == Some("Feature group")
                    && cell.get("colSpan").and_then(Value::as_u64) == Some(2)
            }),
            "table cells should preserve gridSpan metadata: {}",
            serde_json::to_string_pretty(cells).unwrap()
        );
        assert!(cells.iter().any(|cell| {
            cell.get("text").and_then(Value::as_str) == Some("Growth")
                && cell.get("verticalMerge").and_then(Value::as_str) == Some("restart")
        }));
        assert!(cells
            .iter()
            .any(|cell| { cell.get("verticalMerge").and_then(Value::as_str) == Some("continue") }));

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert!(split
            .pointer("/questionGroupCandidates")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .flat_map(|group| {
                group
                    .get("sectionEvidence")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .any(|evidence| {
                evidence.get("tableCols").and_then(Value::as_u64) == Some(3)
                    && evidence.get("tableRows").and_then(Value::as_u64) == Some(3)
                    && evidence.get("tableHasColSpans").and_then(Value::as_bool) == Some(true)
                    && evidence
                        .get("tableHasVerticalMerges")
                        .and_then(Value::as_bool)
                        == Some(true)
                    && evidence.get("tableMergedCellCount").and_then(Value::as_u64) == Some(3)
            }));

        let _ = fs::remove_file(docx_path);
        let _ = fs::remove_file(output);
    }

    #[test]
    fn docx_ooxml_parser_preserves_paragraph_style_and_numbering_metadata() {
        let mut job = test_job();
        job.source_files = vec![test_source("docx")];
        let docx_path = env::temp_dir().join(format!(
            "epic8-docx-style-numbering-{}.docx",
            Uuid::new_v4().simple()
        ));
        let output = env::temp_dir().join(format!(
            "epic8-docx-style-numbering-{}.json",
            Uuid::new_v4().simple()
        ));
        write_minimal_docx(
            &docx_path,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>READING PASSAGE 1</w:t></w:r></w:p>
    <w:p><w:r><w:t>Styled passage text for the parser.</w:t></w:r></w:p>
    <w:p><w:r><w:t>Questions 1-2 Choose TWO letters, A-C.</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr></w:pPr><w:r><w:t>A faster growth</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr></w:pPr><w:r><w:t>B lower cost</w:t></w:r></w:p>
    <w:p><w:r><w:t>Answers 1 A 2 B</w:t></w:r></w:p>
  </w:body>
</w:document>"#,
        );

        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &docx_path,
            &output,
            "auto",
        )
        .unwrap();

        assert_eq!(
            doc.pointer("/pages/0/blocks/0/blockType")
                .and_then(Value::as_str),
            Some("header")
        );
        assert_eq!(
            doc.pointer("/pages/0/blocks/0/layoutHints/headingLevel")
                .and_then(Value::as_u64),
            Some(1)
        );
        let list_block = doc
            .pointer("/pages/0/blocks")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|block| block.get("blockType").and_then(Value::as_str) == Some("list"))
            .expect("styled DOCX should preserve numbered list blocks");
        assert_eq!(
            list_block
                .pointer("/layoutHints/numbering/id")
                .and_then(Value::as_str),
            Some("7")
        );
        assert_eq!(
            list_block
                .pointer("/layoutHints/numbering/level")
                .and_then(Value::as_u64),
            Some(0)
        );

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let evidence = split
            .pointer("/questionGroupCandidates/0/sectionEvidence")
            .and_then(Value::as_array)
            .unwrap();
        assert!(evidence
            .iter()
            .any(|item| item.get("numberingId").and_then(Value::as_str) == Some("7")));

        let _ = fs::remove_file(docx_path);
        let _ = fs::remove_file(output);
    }

    #[test]
    fn docx_ooxml_parser_resolves_styles_and_numbering_definitions() {
        let mut job = test_job();
        job.source_files = vec![test_source("docx")];
        let docx_path = env::temp_dir().join(format!(
            "epic8-docx-resolved-style-numbering-{}.docx",
            Uuid::new_v4().simple()
        ));
        let output = env::temp_dir().join(format!(
            "epic8-docx-resolved-style-numbering-{}.json",
            Uuid::new_v4().simple()
        ));
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:pPr><w:pStyle w:val="IELTSHeading"/></w:pPr><w:r><w:t>READING PASSAGE 2</w:t></w:r></w:p>
    <w:p><w:r><w:t>Questions 14-16 Match each statement with the correct paragraph.</w:t></w:r></w:p>
    <w:p><w:pPr><w:pStyle w:val="IELTSOption"/></w:pPr><w:r><w:t>A early research</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="42"/></w:numPr></w:pPr><w:r><w:t>B later criticism</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        write_minimal_docx(&docx_path, document_xml);
        {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&docx_path)
                .unwrap();
            let mut zip = zip::ZipWriter::new_append(file).unwrap();
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("word/styles.xml", options).unwrap();
            zip.write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="BaseHeading"><w:name w:val="Heading 2"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style>
  <w:style w:type="paragraph" w:styleId="IELTSHeading"><w:name w:val="IELTS Passage Heading"/><w:basedOn w:val="BaseHeading"/></w:style>
  <w:style w:type="paragraph" w:styleId="IELTSOption"><w:name w:val="IELTS Option"/><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="42"/></w:numPr></w:pPr></w:style>
</w:styles>"#,
            )
            .unwrap();
            zip.start_file("word/numbering.xml", options).unwrap();
            zip.write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:abstractNum w:abstractNumId="9">
    <w:lvl w:ilvl="1"><w:numFmt w:val="upperLetter"/><w:lvlText w:val="%2."/></w:lvl>
  </w:abstractNum>
  <w:num w:numId="42"><w:abstractNumId w:val="9"/></w:num>
</w:numbering>"#,
            )
            .unwrap();
            zip.finish().unwrap();
        }

        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &docx_path,
            &output,
            "auto",
        )
        .unwrap();

        assert_eq!(
            doc.pointer("/pages/0/blocks/0/blockType")
                .and_then(Value::as_str),
            Some("header")
        );
        assert_eq!(
            doc.pointer("/pages/0/blocks/0/layoutHints/styleId")
                .and_then(Value::as_str),
            Some("IELTSHeading")
        );
        assert_eq!(
            doc.pointer("/pages/0/blocks/0/layoutHints/styleName")
                .and_then(Value::as_str),
            Some("IELTS Passage Heading")
        );
        assert_eq!(
            doc.pointer("/pages/0/blocks/0/layoutHints/basedOnStyleId")
                .and_then(Value::as_str),
            Some("BaseHeading")
        );
        assert_eq!(
            doc.pointer("/pages/0/blocks/0/layoutHints/headingLevel")
                .and_then(Value::as_u64),
            Some(2)
        );
        let list_block = doc
            .pointer("/pages/0/blocks")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|block| block.get("blockType").and_then(Value::as_str) == Some("list"))
            .expect("numbering definitions should produce list block metadata");
        assert_eq!(
            list_block
                .pointer("/layoutHints/numbering/id")
                .and_then(Value::as_str),
            Some("42")
        );
        assert_eq!(
            list_block
                .pointer("/layoutHints/numbering/abstractId")
                .and_then(Value::as_str),
            Some("9")
        );
        assert_eq!(
            list_block
                .pointer("/layoutHints/numbering/format")
                .and_then(Value::as_str),
            Some("upperLetter")
        );
        assert_eq!(
            list_block
                .pointer("/layoutHints/numbering/text")
                .and_then(Value::as_str),
            Some("%2.")
        );
        assert_eq!(
            list_block
                .pointer("/layoutHints/numbering/renderedLabel")
                .and_then(Value::as_str),
            Some("A.")
        );

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let evidence = split
            .pointer("/questionGroupCandidates/0/sectionEvidence")
            .and_then(Value::as_array)
            .unwrap();
        assert!(evidence
            .iter()
            .any(|item| item.get("numberingId").and_then(Value::as_str) == Some("42")));

        let _ = fs::remove_file(docx_path);
        let _ = fs::remove_file(output);
    }

    #[test]
    fn docx_auto_numbering_recovers_questions_options_and_student_html() {
        let mut job = test_job();
        job.source_files = vec![test_source("docx")];
        let docx_path = env::temp_dir().join(format!(
            "epic8-docx-auto-numbering-{}.docx",
            Uuid::new_v4().simple()
        ));
        let output = env::temp_dir().join(format!(
            "epic8-docx-auto-numbering-{}.json",
            Uuid::new_v4().simple()
        ));
        write_minimal_docx(
            &docx_path,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>READING PASSAGE 1</w:t></w:r></w:p>
    <w:p><w:r><w:t>Archive access</w:t></w:r></w:p>
    <w:p><w:r><w:t>The archive moved so visitors could inspect maps in a larger room.</w:t></w:r></w:p>
    <w:p><w:r><w:t>Choose the correct letter, A, B, C or D.</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="10"/></w:numPr></w:pPr><w:r><w:t>Why did the archive move?</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="20"/></w:numPr></w:pPr><w:r><w:t>To reduce staffing</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="20"/></w:numPr></w:pPr><w:r><w:t>To provide more space</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="20"/></w:numPr></w:pPr><w:r><w:t>To close the map room</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="20"/></w:numPr></w:pPr><w:r><w:t>To sell the collection</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="10"/></w:numPr></w:pPr><w:r><w:t>What can visitors inspect?</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="21"/></w:numPr></w:pPr><w:r><w:t>Maps</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="21"/></w:numPr></w:pPr><w:r><w:t>Tickets</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="21"/></w:numPr></w:pPr><w:r><w:t>Furniture</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="21"/></w:numPr></w:pPr><w:r><w:t>Photographs only</w:t></w:r></w:p>
    <w:p><w:r><w:t>Choose the correct heading for each paragraph from the list of headings.</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="30"/></w:numPr></w:pPr><w:r><w:t>Early access problems</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="30"/></w:numPr></w:pPr><w:r><w:t>A larger public archive</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="30"/></w:numPr></w:pPr><w:r><w:t>Future collection plans</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="11"/></w:numPr></w:pPr><w:r><w:t>Paragraph A</w:t></w:r></w:p>
    <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="11"/></w:numPr></w:pPr><w:r><w:t>Paragraph B</w:t></w:r></w:p>
    <w:p><w:r><w:t>Answers 14 B 15 A 16 ii 17 i</w:t></w:r></w:p>
  </w:body>
</w:document>"#,
        );
        {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&docx_path)
                .unwrap();
            let mut zip = zip::ZipWriter::new_append(file).unwrap();
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("word/numbering.xml", options).unwrap();
            zip.write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum>
  <w:abstractNum w:abstractNumId="2"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="upperLetter"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum>
  <w:abstractNum w:abstractNumId="3"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="lowerRoman"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum>
  <w:num w:numId="10"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="14"/></w:lvlOverride></w:num>
  <w:num w:numId="11"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="16"/></w:lvlOverride></w:num>
  <w:num w:numId="20"><w:abstractNumId w:val="2"/></w:num>
  <w:num w:numId="21"><w:abstractNumId w:val="2"/></w:num>
  <w:num w:numId="30"><w:abstractNumId w:val="3"/></w:num>
</w:numbering>"#,
            )
            .unwrap();
            zip.finish().unwrap();
        }

        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &docx_path,
            &output,
            "auto",
        )
        .unwrap();
        let blocks = doc
            .pointer("/pages/0/blocks")
            .and_then(Value::as_array)
            .unwrap();
        assert!(blocks.iter().any(|block| {
            block.get("text").and_then(Value::as_str) == Some("14. Why did the archive move?")
                && block
                    .pointer("/layoutHints/numbering/renderedLabel")
                    .and_then(Value::as_str)
                    == Some("14.")
        }));
        assert!(blocks.iter().any(|block| {
            block.get("text").and_then(Value::as_str) == Some("A. To reduce staffing")
        }));

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split.pointer("/questionGroupCandidates/0/questionRange"),
            Some(&json!([14, 15]))
        );
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/kindHint")
                .and_then(Value::as_str),
            Some("single_choice")
        );
        assert_eq!(
            split.pointer("/questionGroupCandidates/1/questionRange"),
            Some(&json!([16, 17]))
        );
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/1/kindHint")
                .and_then(Value::as_str),
            Some("heading_matching")
        );

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/groups/0/questions/0/prompt")
                .and_then(Value::as_str),
            Some("Why did the archive move?")
        );
        assert_eq!(
            ir.pointer("/groups/0/questions/1/prompt")
                .and_then(Value::as_str),
            Some("What can visitors inspect?")
        );
        assert_eq!(
            ir.pointer("/groups/0/questions/0/interaction/options"),
            Some(&json!(["A", "B", "C", "D"]))
        );
        assert_eq!(
            ir.pointer("/groups/0/questions/0/interaction/optionTexts/B")
                .and_then(Value::as_str),
            Some("To provide more space")
        );
        assert_eq!(
            ir.pointer("/groups/0/questions/1/interaction/optionTexts/A")
                .and_then(Value::as_str),
            Some("Maps")
        );
        assert_eq!(
            ir.pointer("/groups/1/questions/0/prompt")
                .and_then(Value::as_str),
            Some("Paragraph A")
        );
        assert_eq!(
            ir.pointer("/groups/1/questions/0/interaction/options"),
            Some(&json!(["i", "ii", "iii"]))
        );
        assert_eq!(
            ir.pointer("/groups/1/questions/0/interaction/optionTexts/ii")
                .and_then(Value::as_str),
            Some("A larger public archive")
        );

        let source = reading_source(&ir);
        let body_html = source
            .pointer("/questionGroups/0/bodyHtml")
            .and_then(Value::as_str)
            .unwrap();
        assert!(body_html.contains("value=\"B\""));
        assert!(body_html.contains("To provide more space"));

        let _ = fs::remove_file(docx_path);
        let _ = fs::remove_file(output);
    }

    #[test]
    fn docx_embedded_drawing_requires_source_review_warning() {
        let mut job = test_job();
        job.source_files = vec![test_source("docx")];
        let docx_path = env::temp_dir().join(format!(
            "epic8-docx-drawing-warning-{}.docx",
            Uuid::new_v4().simple()
        ));
        let output = env::temp_dir().join(format!(
            "epic8-docx-drawing-warning-{}.json",
            Uuid::new_v4().simple()
        ));
        write_minimal_docx(
            &docx_path,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Questions 1-2 Label the map below.</w:t></w:r></w:p>
    <w:p><w:r><w:drawing/></w:r></w:p>
  </w:body>
</w:document>"#,
        );

        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &docx_path,
            &output,
            "auto",
        )
        .unwrap();
        let warnings = doc
            .pointer("/parser/warnings")
            .and_then(Value::as_array)
            .unwrap();
        assert!(warnings.iter().any(|warning| {
            warning
                .as_str()
                .unwrap_or_default()
                .contains("embedded drawings or images")
        }));

        let _ = fs::remove_file(docx_path);
        let _ = fs::remove_file(output);
    }

    #[test]
    fn docx_ooxml_parser_preserves_section_column_metadata() {
        let mut job = test_job();
        job.source_files = vec![test_source("docx")];
        let docx_path = env::temp_dir().join(format!(
            "epic8-docx-section-columns-{}.docx",
            Uuid::new_v4().simple()
        ));
        let output = env::temp_dir().join(format!(
            "epic8-docx-section-columns-{}.json",
            Uuid::new_v4().simple()
        ));
        let document_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:pPr><w:sectPr><w:cols w:num="2" w:space="720" w:equalWidth="1"/></w:sectPr></w:pPr><w:r><w:t>READING PASSAGE 3</w:t></w:r></w:p>
    <w:p><w:r><w:t>The first paragraph is laid out in two newspaper-style columns.</w:t></w:r></w:p>
    <w:p><w:r><w:t>Questions 27-28 Choose TWO letters, A-C.</w:t></w:r></w:p>
    <w:p><w:r><w:t>A Column-aware extraction</w:t></w:r></w:p>
    <w:p><w:r><w:t>B Ignored page structure</w:t></w:r></w:p>
    <w:p><w:r><w:t>Answers 27 A 28 C</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        write_minimal_docx(&docx_path, document_xml);

        let doc = parse_source_document(
            &job,
            job.source_files.first().unwrap(),
            &docx_path,
            &output,
            "auto",
        )
        .unwrap();

        assert_eq!(
            doc.pointer("/pages/0/blocks/0/layoutHints/section/columns/count")
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            doc.pointer("/pages/0/blocks/0/layoutHints/section/columns/spaceTwips")
                .and_then(Value::as_u64),
            Some(720)
        );
        assert_eq!(
            doc.pointer("/pages/0/blocks/0/layoutHints/section/columns/equalWidth")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            doc.pointer("/pages/0/blocks/1/layoutHints/section/columns/count")
                .and_then(Value::as_u64),
            Some(2)
        );

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let evidence = split
            .pointer("/questionGroupCandidates/0/sectionEvidence")
            .and_then(Value::as_array)
            .unwrap();
        assert!(evidence
            .iter()
            .any(|item| { item.get("sectionColumnCount").and_then(Value::as_u64) == Some(2) }));

        let _ = fs::remove_file(docx_path);
        let _ = fs::remove_file(output);
    }

    #[test]
    fn registered_phase4_pdf_corpus_reaches_frozen_v1_parse_split_and_authoring() {
        for (fixture_id, sample, metadata) in phase4_pdf_fixtures() {
            let mut job = make_job(CreateJobInput {
                title: Some(fixture_id.clone()),
                category: None,
                frequency: None,
                tags: Some(vec!["phase4-eight-pdf-corpus".to_string()]),
                llm_profile_id: None,
            });
            let source = SourceFile {
                file_id: format!("file-{fixture_id}"),
                original_name: metadata
                    .pointer("/source/originalName")
                    .and_then(Value::as_str)
                    .expect("Phase 4 metadata must declare source.originalName")
                    .to_string(),
                stored_name: sample
                    .file_name()
                    .and_then(|name| name.to_str())
                    .expect("Phase 4 source filename must be UTF-8")
                    .to_string(),
                file_type: "pdf".to_string(),
                sha256: metadata
                    .pointer("/source/sha256")
                    .and_then(Value::as_str)
                    .expect("Phase 4 metadata must declare source.sha256")
                    .to_string(),
                size_bytes: metadata
                    .pointer("/source/sizeBytes")
                    .and_then(Value::as_u64)
                    .expect("Phase 4 metadata must declare source.sizeBytes"),
                role: "MainQuestion".to_string(),
                imported_at: Utc::now(),
            };
            job.source_files = vec![source.clone()];
            let output = env::temp_dir().join(format!(
                "phase4-v1-{fixture_id}-{}-document-ir.json",
                Uuid::new_v4().simple()
            ));
            let document = parse_source_document(&job, &source, &sample, &output, "auto")
                .unwrap_or_else(|error| panic!("{fixture_id} V1 parse failed: {error}"));
            assert_pdf_parser_provenance(&document, &sample);

            let baseline = metadata
                .pointer("/baseline/observed")
                .expect("Phase 4 metadata must declare frozen V1 observations");
            assert_eq!(
                document
                    .get("pages")
                    .and_then(Value::as_array)
                    .map(Vec::len),
                baseline
                    .get("pageCount")
                    .and_then(Value::as_u64)
                    .map(|count| count as usize),
                "{fixture_id} V1 page count drifted"
            );
            assert_eq!(
                parser_warnings(Some(&document)).len(),
                baseline
                    .get("warningCount")
                    .and_then(Value::as_u64)
                    .expect("frozen V1 warning count must be numeric") as usize,
                "{fixture_id} V1 parser warning count drifted"
            );

            let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&document));
            assert_eq!(
                split
                    .get("questionGroupCandidates")
                    .and_then(Value::as_array)
                    .map(Vec::len),
                baseline
                    .get("groupCount")
                    .and_then(Value::as_u64)
                    .map(|count| count as usize),
                "{fixture_id} V1 split group count drifted"
            );
            let authoring = make_dynamic_authoring_ir(&job, &split, Some(&document));
            assert_eq!(
                authoring
                    .get("questionOrder")
                    .and_then(Value::as_array)
                    .map(Vec::len),
                baseline
                    .get("slotCount")
                    .and_then(Value::as_u64)
                    .map(|count| count as usize),
                "{fixture_id} V1 authoring slot count drifted"
            );
            let _ = fs::remove_file(output);
        }
    }

    #[test]
    #[ignore = "superseded by the registered Phase 4 eight-PDF corpus; requires removed legacy Files/*.pdf inputs"]
    fn files_pdf_samples_reach_expected_review_paths() {
        let sample_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("Files");
        let mut samples = fs::read_dir(&sample_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        samples.sort();
        if samples.is_empty() {
            eprintln!(
                "skipping files_pdf_samples_reach_expected_review_paths: Files/ contains no PDF samples"
            );
            return;
        }
        assert_eq!(
            samples.len(),
            4,
            "Files/ should contain exactly the four user-provided PDF samples"
        );

        let mut concrete_question_samples = 0usize;
        let mut umbrella_scaffold_samples = 0usize;
        let mut mixed_image_page_samples = 0usize;
        let mut full_text_layer_samples = 0usize;
        for (index, sample) in samples.iter().enumerate() {
            let root = temp_test_root();
            ensure_app_dirs(&root).unwrap();
            let original_name = sample
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap()
                .to_string();
            let mut job = make_job(CreateJobInput {
                title: Some(
                    sample
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("PDF sample")
                        .to_string(),
                ),
                category: Some("P2".to_string()),
                frequency: Some("medium".to_string()),
                tags: Some(vec!["files-pdf-sample".to_string()]),
                llm_profile_id: None,
            });
            let source = SourceFile {
                file_id: format!("file-sample-{}", index + 1),
                original_name: original_name.clone(),
                stored_name: original_name,
                file_type: "pdf".to_string(),
                sha256: "2".repeat(64),
                size_bytes: sample.metadata().unwrap().len(),
                role: "MainQuestion".to_string(),
                imported_at: Utc::now(),
            };
            job.source_files = vec![source.clone()];
            save_job(&root, &job).unwrap();
            ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();

            let parser_output = root
                .join("cache")
                .join("parser")
                .join(format!("{}-files-sample-document-ir.json", job.job_id));
            let doc = parse_source_document(&job, &source, sample, &parser_output, "auto")
                .unwrap_or_else(|error| panic!("{} parse failed: {}", sample.display(), error));
            assert!(
                matches!(
                    doc.pointer("/parser/provider").and_then(Value::as_str),
                    Some("rust-parser:pdf:pdf-extract") | Some("python-parser-sidecar:pdf:pypdf")
                ),
                "{} should parse through either the Rust PDF path or the Python PDF fallback",
                sample.display()
            );
            let extracted_text = dynamic_document_blocks(Some(&doc))
                .iter()
                .map(dynamic_block_text)
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                extracted_text
                    .chars()
                    .filter(|ch| ch.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_uppercase()
                    .contains("READINGPASSAGE2")
                    || extracted_text
                        .to_lowercase()
                        .contains("which are based on reading"),
                "{} should expose passage text",
                sample.display()
            );

            write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();
            write_source_review_status(&root, &job.job_id, Some(&doc), false, None).unwrap();
            let review = source_review_status(&root, &job.job_id, Some(&doc)).unwrap();
            let parser_warning_text = parser_warnings(Some(&doc)).join("\n").to_lowercase();
            let has_image_only_pages = parser_warning_text.contains("no extractable text");
            let needs_vision = main_pdf_needs_vision_transcription(&job, &doc);
            let review_required = review.get("required").and_then(Value::as_bool) == Some(true);
            let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
            let has_reliable_groups = split
                .get("questionGroupCandidates")
                .and_then(Value::as_array)
                .map(|groups| {
                    groups.iter().any(|candidate| {
                        candidate
                            .get("requiresManualQuestionImport")
                            .and_then(Value::as_bool)
                            != Some(true)
                    })
                })
                .unwrap_or(false);
            if has_image_only_pages {
                mixed_image_page_samples += 1;
                assert!(
                    needs_vision,
                    "{} has image-only/blank pages and should be eligible for vision transcription",
                    sample.display()
                );
                assert!(
                    review_required,
                    "{} has image-only/blank pages and should require source review before publish",
                    sample.display()
                );
            } else {
                full_text_layer_samples += 1;
                if has_reliable_groups {
                    assert!(
                        !needs_vision,
                        "{} has reliable text-layer question groups and should not enter the vision path",
                        sample.display()
                    );
                    assert!(
                        !review_required,
                        "{} has reliable text-layer question groups and should not require source review",
                        sample.display()
                    );
                } else {
                    assert!(
                        needs_vision,
                        "{} is text-layer readable but lacks reliable question groups, so it should enter the vision path",
                        sample.display()
                    );
                }
            }
            assert!(
                split
                    .get("umbrellaQuestionRanges")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items.iter().any(|item| {
                            item.get("questionRange")
                                .and_then(Value::as_array)
                                .map(|range| {
                                    range.first().and_then(Value::as_u64) == Some(14)
                                        && range.get(1).and_then(Value::as_u64) == Some(26)
                                })
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false),
                "{} should preserve the P2 umbrella Questions 14-26 range",
                sample.display()
            );

            let group_count = split
                .get("questionGroupCandidates")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let issue_text = split
                .get("issues")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" | ");
            assert!(
                group_count > 0,
                "{} should produce at least an umbrella scaffold",
                sample.display()
            );
            let groups = split
                .get("questionGroupCandidates")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let has_manual_umbrella_scaffold = groups.iter().any(|candidate| {
                candidate.get("isUmbrellaRange").and_then(Value::as_bool) == Some(true)
                    && candidate
                        .get("requiresManualQuestionImport")
                        .and_then(Value::as_bool)
                        == Some(true)
                    && candidate
                        .get("questionRange")
                        .and_then(Value::as_array)
                        .map(|range| {
                            range.first().and_then(Value::as_u64) == Some(14)
                                && range.get(1).and_then(Value::as_u64) == Some(26)
                        })
                        .unwrap_or(false)
            });
            let concrete_groups = groups
                .iter()
                .filter(|candidate| {
                    candidate
                        .get("requiresManualQuestionImport")
                        .and_then(Value::as_bool)
                        != Some(true)
                })
                .collect::<Vec<_>>();
            if has_manual_umbrella_scaffold {
                umbrella_scaffold_samples += 1;
                assert!(
                    issue_text.contains("Only umbrella question range detected"),
                    "{} should explicitly ask for manual concrete question import",
                    sample.display()
                );
            } else {
                assert!(
                    concrete_groups.iter().all(|candidate| {
                        candidate
                            .get("questionRange")
                            .and_then(Value::as_array)
                            .map(|range| {
                                !(range.first().and_then(Value::as_u64) == Some(14)
                                    && range.get(1).and_then(Value::as_u64) == Some(26))
                            })
                            .unwrap_or(true)
                    }),
                    "{} should not turn the umbrella range into a duplicate concrete question group",
                    sample.display()
                );
                concrete_question_samples += 1;
                if sample
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.starts_with("120."))
                {
                    let ranges = concrete_groups
                        .iter()
                        .filter_map(|candidate| {
                            candidate
                                .get("questionRange")
                                .and_then(Value::as_array)
                                .map(|range| {
                                    (
                                        range.first().and_then(Value::as_u64).unwrap_or_default(),
                                        range.get(1).and_then(Value::as_u64).unwrap_or_default(),
                                    )
                                })
                        })
                        .collect::<Vec<_>>();
                    assert!(
                        ranges.contains(&(14, 19))
                            && ranges.contains(&(20, 23))
                            && ranges.contains(&(24, 26)),
                        "{} should keep later concrete question groups after interleaved answer letters; got {:?}",
                        sample.display(),
                        ranges
                    );
                }
            }
            let mut ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
            let review_count = refresh_authoring_review_state(&mut ir);
            assert!(
                ir.get("questionOrder")
                    .and_then(Value::as_array)
                    .map(|items| !items.is_empty())
                    .unwrap_or(false),
                "{} should reach non-empty AuthoringIR questions",
                sample.display()
            );
            if has_manual_umbrella_scaffold {
                assert!(
                    review_count > 0
                        && ir
                            .pointer("/groups/0/requiresManualQuestionImport")
                            .and_then(Value::as_bool)
                            == Some(true),
                    "{} umbrella scaffold must stay in AuthoringReview until manually completed",
                    sample.display()
                );
            }

            let _ = fs::remove_dir_all(root);
        }

        assert!(
            concrete_question_samples >= 3,
            "at least three samples should include concrete question groups"
        );
        assert!(
            umbrella_scaffold_samples >= 1,
            "at least one sample should exercise the umbrella-only manual question import path"
        );
        assert!(
            mixed_image_page_samples >= 3,
            "at least three samples should exercise mixed text/image PDF review routing"
        );
        assert!(
            full_text_layer_samples >= 1,
            "at least one sample should exercise the fully text-layer readable path"
        );
    }

    #[test]
    fn text_layer_pdf_without_reliable_question_groups_still_enters_vision_path() {
        let root = temp_test_root();
        let mut job = make_job(CreateJobInput {
            title: Some("Sunlight Soap Missing Questions".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["sunlight-soap".to_string()]),
            llm_profile_id: None,
        });
        job.source_files = vec![test_source("pdf")];
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [
                {
                    "pageIndex": 1,
                    "width": 595,
                    "height": 842,
                    "blocks": [
                        {
                            "blockId":"passage-1",
                            "blockType":"paragraph",
                            "text":"READING PASSAGE 1 Lever Brothers' Sunlight Soap A Revolution in Hygiene and Industry.",
                            "html":"<p>READING PASSAGE 1 Lever Brothers' Sunlight Soap A Revolution in Hygiene and Industry.</p>",
                            "bbox":[72,72,520,120],
                            "confidence":0.98,
                            "roleHint":"passage"
                        }
                    ]
                },
                {
                    "pageIndex": 2,
                    "width": 595,
                    "height": 842,
                    "blocks": [
                        {
                            "blockId":"passage-2",
                            "blockType":"paragraph",
                            "text":"Marketing and Branding. Impact on Hygiene and Public Health.",
                            "html":"<p>Marketing and Branding. Impact on Hygiene and Public Health.</p>",
                            "bbox":[72,72,520,120],
                            "confidence":0.98,
                            "roleHint":"passage"
                        }
                    ]
                }
            ],
            "assets": [],
            "parser": {
                "provider":"rust-parser:pdf:pdf-extract",
                "version":"0.3.0",
                "mode":"auto",
                "warnings":[]
            }
        });

        assert!(
            main_pdf_needs_vision_transcription(&job, &doc),
            "text-layer PDFs that contain passage text but no reliable question groups must enter the vision path"
        );
        assert!(
            !make_dynamic_split_candidates(&job.job_id, &job, Some(&doc))
                .get("questionGroupCandidates")
                .and_then(Value::as_array)
                .map(|groups| !groups.is_empty())
                .unwrap_or(false)
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn vision_transcription_without_question_groups_does_not_overwrite_text_layer_parse() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        write_diagnostics_settings(
            &root,
            &crate::diagnostics::DiagnosticsSettings {
                keep_full_process_artifacts: true,
            },
        )
        .unwrap();
        crate::llm_profiles::save_profiles(
            &root,
            &[json!({
                "profileId": "profile-vision-transcription",
                "name": "Vision Transcription Test",
                "provider": "OpenAiCompatible",
                "baseUrl": "http://unit.test/v1",
                "model": "unit-test",
                "temperature": 0,
                "timeoutMs": 60000,
                "forceJson": true,
                "enabled": true
            })],
        )
        .unwrap();
        let mut job = make_job(CreateJobInput {
            title: Some("Origin of Paper".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["vision-no-overwrite".to_string()]),
            llm_profile_id: Some("profile-vision-transcription".to_string()),
        });
        attach_fixture_source(&root, &mut job, "no-text.pdf", "MainQuestion");
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [{
                    "blockId": "passage-only",
                    "blockType": "paragraph",
                    "text": "READING PASSAGE 1 The Origin of Paper. The earliest paper was made from plant fibres and changed communication.",
                    "html": "<p>READING PASSAGE 1 The Origin of Paper.</p>",
                    "bbox": [72, 72, 520, 140],
                    "confidence": 0.98,
                    "roleHint": "passage"
                }]
            }],
            "assets": [],
            "parser": {
                "provider": "rust-parser:pdf:pdf-extract",
                "version": "0.3.0",
                "mode": "auto",
                "warnings": []
            }
        });
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();
        write_source_review_status(&root, &job.job_id, Some(&doc), false, None).unwrap();

        let report = run_auto_pipeline_core_with_gateway(
            &root,
            &job.job_id,
            Some(AutoPipelineInput {
                parse_mode: Some("auto".to_string()),
                confidence_threshold: Some(0.85),
                profile_id: Some("profile-vision-transcription".to_string()),
                execution_mode: None,
                target: Some("editableDraft".to_string()),
                allow_overwrite: None,
            }),
            |_root, _job_id, command_name, _input, _api_key| match command_name {
                "transcribe_pdf_images" => Ok(json!({
                    "text": "题号 答案 原文定位\n1 TRUE 段 4\n2 FALSE 段 5",
                    "confidence": 0.94,
                    "warnings": ["Only the answer page was visible in the supplied image set."]
                })),
                "extract_pdf_image_answers" => Ok(json!({
                    "answers": {
                        "1": "TRUE",
                        "2": "FALSE"
                    },
                    "confidence": 0.99,
                    "warnings": [],
                    "evidence": [{"questionNumber": "1", "pageIndex": 1, "quote": "1 TRUE"}]
                })),
                "generate_pdf_reading_outline" => Ok(json!({
                    "title": "Origin of Paper",
                    "groups": [],
                    "answerKey": {},
                    "confidence": 0.0,
                    "warnings": ["No question groups visible."]
                })),
                other => Err(format!("unexpected_command:{}", other)),
            },
        )
        .unwrap();

        assert_eq!(
            report
                .pointer("/parser/visionTranscription/attempted")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            report
                .pointer("/parser/visionTranscription/applied")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert!(report
            .pointer("/parser/visionTranscription/failure")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("authoritative local parse was unchanged"));
        assert_eq!(
            report
                .pointer("/parser/visionTranscription/diagnosticOnly")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            report
                .pointer("/parser/visionTranscription/wouldPassQualityGate")
                .and_then(Value::as_bool),
            Some(false)
        );
        let saved_doc: Value =
            read_json(&job_dir(&root, &job.job_id).join("document-ir.json")).unwrap();
        let saved_text = dynamic_document_blocks(Some(&saved_doc))
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(saved_text.contains("The Origin of Paper"));
        assert!(!saved_text.contains("题号 答案"));
        assert!(parser_warnings(Some(&saved_doc)).is_empty());
        let ir: Value = read_json(&job_dir(&root, &job.job_id).join("authoring-ir.json")).unwrap();
        let ir_text = serde_json::to_string(&ir).unwrap();
        assert!(ir_text.contains("The Origin of Paper"));
        assert!(ir
            .pointer("/audit/issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|issue| {
                issue.get("kind").and_then(Value::as_str) == Some("vision_transcription_summary")
            }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_pipeline_records_vision_unavailable_for_umbrella_only_pdf_without_profile() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        crate::llm_profiles::save_profiles(&root, &[]).unwrap();
        let mut job = make_job(CreateJobInput {
            title: Some("P2 Umbrella Only".to_string()),
            category: Some("P2".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["umbrella-no-profile".to_string()]),
            llm_profile_id: None,
        });
        attach_fixture_source(&root, &mut job, "no-text.pdf", "MainQuestion");
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [
                    {"blockId":"p2-header","blockType":"header","text":"READING PASSAGE 2","html":"<h2>READING PASSAGE 2</h2>","bbox":[72,60,460,88],"confidence":0.99,"roleHint":"passage"},
                    {"blockId":"p2-umbrella","blockType":"paragraph","text":"Questions 14\u{2013}26","html":"<p>Questions 14\u{2013}26</p>","bbox":[72,92,520,120],"confidence":0.98,"roleHint":"question"},
                    {"blockId":"p2-passage","blockType":"paragraph","text":"The passage content is available but the concrete question prompts were not extracted.","html":"<p>The passage content is available but the concrete question prompts were not extracted.</p>","bbox":[72,130,520,210],"confidence":0.97,"roleHint":"passage"},
                    {"blockId":"answers","blockType":"paragraph","text":"Answers 14 TRUE 15 FALSE 16 TRUE 17 NOT GIVEN 18 TRUE 19 FALSE 20 A 21 B 22 C 23 D 24 chemicals 25 threats 26 information","html":"<p>Answers 14 TRUE 15 FALSE 16 TRUE 17 NOT GIVEN 18 TRUE 19 FALSE 20 A 21 B 22 C 23 D 24 chemicals 25 threats 26 information</p>","bbox":[72,600,520,650],"confidence":0.92,"roleHint":"answer"}
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();
        write_source_review_status(&root, &job.job_id, Some(&doc), false, None).unwrap();

        let report = run_auto_pipeline_core(&root, &job.job_id, None).unwrap();

        assert_eq!(
            report
                .pointer("/parser/visionTranscription/attempted")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            report
                .pointer("/parser/visionTranscription/failure")
                .and_then(Value::as_str),
            Some("no_enabled_llm_profile_available_for_pdf_vision_transcription")
        );

        let ir: Value = read_json(&job_dir(&root, &job.job_id).join("authoring-ir.json")).unwrap();
        assert_eq!(
            ir.pointer("/groups/0/questions/0/prompt")
                .and_then(Value::as_str),
            Some("")
        );
        let vision_issue = ir
            .pointer("/audit/issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|issue| {
                issue.get("kind").and_then(Value::as_str) == Some("vision_transcription_summary")
            })
            .cloned()
            .unwrap();
        assert!(vision_issue
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("未配置可用云端模型"));
        assert!(vision_issue
            .get("missingPromptQuestionIds")
            .and_then(Value::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_pipeline_local_only_skips_all_pdf_cloud_calls() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
        write_diagnostics_settings(
            &root,
            &crate::diagnostics::DiagnosticsSettings {
                keep_full_process_artifacts: true,
            },
        )
        .unwrap();
        crate::llm_profiles::save_profiles(
            &root,
            &[json!({
                "profileId": "profile-vision-transcription",
                "name": "Vision Transcription Test",
                "provider": "OpenAiCompatible",
                "baseUrl": "http://unit.test/v1",
                "model": "unit-test",
                "temperature": 0,
                "timeoutMs": 60000,
                "forceJson": true,
                "enabled": true
            })],
        )
        .unwrap();
        let mut job = make_job(CreateJobInput {
            title: Some("Origin of Paper".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["vision-local-only".to_string()]),
            llm_profile_id: Some("profile-vision-transcription".to_string()),
        });
        attach_fixture_source(&root, &mut job, "no-text.pdf", "MainQuestion");
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595,
                "height": 842,
                "blocks": [{
                    "blockId": "passage-only",
                    "blockType": "paragraph",
                    "text": "READING PASSAGE 1 The Origin of Paper. The earliest paper was made from plant fibres and changed communication.",
                    "html": "<p>READING PASSAGE 1 The Origin of Paper.</p>",
                    "bbox": [72, 72, 520, 140],
                    "confidence": 0.98,
                    "roleHint": "passage"
                }]
            }],
            "assets": [],
            "parser": {
                "provider": "rust-parser:pdf:pdf-extract",
                "version": "0.3.0",
                "mode": "auto",
                "warnings": []
            }
        });
        write_json(&job_dir(&root, &job.job_id).join("document-ir.json"), &doc).unwrap();
        write_source_review_status(&root, &job.job_id, Some(&doc), false, None).unwrap();
        let mut gateway_calls = 0_usize;

        let report = run_auto_pipeline_core_with_gateway(
            &root,
            &job.job_id,
            Some(AutoPipelineInput {
                parse_mode: Some("auto".to_string()),
                confidence_threshold: Some(0.85),
                profile_id: Some("profile-vision-transcription".to_string()),
                execution_mode: Some("localOnly".to_string()),
                target: Some("editableDraft".to_string()),
                allow_overwrite: None,
            }),
            |_root, _job_id, command_name, _input, _api_key| {
                gateway_calls += 1;
                Err(format!("local_only_must_not_call:{command_name}"))
            },
        )
        .unwrap();

        assert_eq!(gateway_calls, 0);
        assert_eq!(
            report
                .pointer("/parser/visionTranscription/attempted")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            report
                .pointer("/parser/visionTranscription/applied")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            report
                .pointer("/parser/visionAnswerExtraction/attempted")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            report
                .pointer("/parser/visionAnswerExtraction/applied")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            report
                .pointer("/parser/visionAnswerExtraction/answerCount")
                .and_then(Value::as_u64),
            Some(0)
        );
        assert_eq!(
            report
                .pointer("/quality/cloudComparison/attempted")
                .and_then(Value::as_bool),
            Some(false)
        );

        let saved_doc: Value =
            read_json(&job_dir(&root, &job.job_id).join("document-ir.json")).unwrap();
        let physical: Value =
            read_json(&job_dir(&root, &job.job_id).join("document-ir-v2.shadow.json")).unwrap();
        assert_eq!(physical["schemaVersion"], "DocumentIRV2");
        assert_eq!(physical["jobId"], job.job_id);
        assert_eq!(
            saved_doc
                .pointer("/parser/provider")
                .and_then(Value::as_str),
            Some("rust-parser:pdf:pdf-extract")
        );
        let ir: Value = read_json(&job_dir(&root, &job.job_id).join("authoring-ir.json")).unwrap();
        assert_eq!(
            ir.get("questionOrder")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert!(!ir
            .get("groups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|group| group.get("autoApplied").and_then(Value::as_bool) == Some(true)));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "superseded by the registered Phase 4 eight-PDF corpus; requires removed legacy Files/*.pdf inputs"]
    fn files_pdf_samples_auto_pipeline_minimizes_artifacts_and_preserves_review_gate() {
        let sample_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("Files");
        let mut samples = fs::read_dir(&sample_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        samples.sort();
        if samples.is_empty() {
            eprintln!(
                "skipping files_pdf_samples_auto_pipeline_minimizes_artifacts_and_preserves_review_gate: Files/ contains no PDF samples"
            );
            return;
        }
        assert_eq!(
            samples.len(),
            4,
            "Files/ should contain exactly the four user-provided PDF samples"
        );

        let mut source_review_required = 0usize;
        let mut authoring_or_llm_required = 0usize;
        for sample in samples {
            let root = temp_test_root();
            ensure_app_dirs(&root).unwrap();
            let mut job = make_job(CreateJobInput {
                title: Some(
                    sample
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("PDF sample")
                        .to_string(),
                ),
                category: Some("P2".to_string()),
                frequency: Some("medium".to_string()),
                tags: Some(vec!["files-pdf-pipeline-sample".to_string()]),
                llm_profile_id: None,
            });
            let original_name = sample
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap()
                .to_string();
            let (hash, size, bytes) = hash_file_or_path(&sample).unwrap();
            let stored_name = format!("{}-{}", &hash[..8], sanitize_filename(&original_name));
            ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
            write_bytes(
                &job_dir(&root, &job.job_id)
                    .join("uploads")
                    .join(&stored_name),
                &bytes.unwrap(),
            )
            .unwrap();
            job.source_files.push(SourceFile {
                file_id: format!("file-{}", Uuid::new_v4().simple()),
                original_name: original_name.clone(),
                stored_name,
                file_type: "pdf".to_string(),
                sha256: hash,
                size_bytes: size,
                role: "MainQuestion".to_string(),
                imported_at: Utc::now(),
            });
            save_job(&root, &job).unwrap();

            let report = run_auto_pipeline_core_with_gateway(
                &root,
                &job.job_id,
                None,
                |_root, _job_id, _mode, _input, _api_key| {
                    Err("mock gateway should not be called without an enabled profile".to_string())
                },
            )
            .unwrap_or_else(|error| panic!("{} auto pipeline failed: {}", original_name, error));
            let job_path = job_dir(&root, &job.job_id);
            assert!(
                job_path.join("authoring-ir.json").exists(),
                "{} should keep the editable AuthoringIR",
                original_name
            );
            assert!(
                job_path.join("authoring-project.json").exists(),
                "{} should keep the editable project manifest",
                original_name
            );
            assert!(
                job_path.join("source-review.json").exists(),
                "{} should persist source review independently of DocumentIR",
                original_name
            );
            for relative in [
                "document-ir.json",
                "split-candidates.json",
                "pipeline-report.json",
                "pipeline-report-summary.json",
                "llm-last-suggestion.json",
                "llm-calls.jsonl",
                "vision-transcription-output.json",
                "vision-transcription.txt",
            ] {
                assert!(
                    !job_path.join(relative).exists(),
                    "{} should not persist transient {} after AuthoringIR",
                    original_name,
                    relative
                );
            }
            assert!(!job_path.join("cache").exists());
            assert!(!job_path.join("preview").exists());
            assert!(!job_path.join("llm-suggestions").exists());
            let parser_cache = root.join("cache").join("parser");
            if parser_cache.exists() {
                let leaked = fs::read_dir(&parser_cache)
                    .unwrap()
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .filter(|name| name.starts_with(&job.job_id))
                    .collect::<Vec<_>>();
                assert!(
                    leaked.is_empty(),
                    "{} should remove root parser cache entries, leaked {:?}",
                    original_name,
                    leaked
                );
            }

            let source_review = source_review_status(&root, &job.job_id, None).unwrap();
            let saved = load_job(&root, &job.job_id).unwrap();
            let review_required =
                source_review.get("required").and_then(Value::as_bool) == Some(true);
            let review_issues = source_review_issues(&source_review);
            if review_required {
                source_review_required += 1;
                assert!(!review_issues.is_empty());
                assert_eq!(saved.status, JobStatus::NeedsReview);
                assert_eq!(saved.current_step, WorkflowStep::Authoring);
                assert_eq!(
                    report.get("currentStep").and_then(Value::as_str),
                    Some("Authoring")
                );
                assert_eq!(
                    report.get("nextRoute").and_then(Value::as_str),
                    Some("groups")
                );
                assert_eq!(
                    report.get("userStatus").and_then(Value::as_str),
                    Some("needsConfirmation")
                );
                assert!(
                    report
                        .get("userMessage")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .contains("题稿已生成"),
                    "{} should keep a user-facing confirmation message visible on the editable draft route",
                    original_name
                );
                let vision_attempted = report
                    .pointer("/parser/visionTranscription/attempted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if vision_attempted {
                    assert_eq!(
                        report
                            .pointer("/parser/visionTranscription/applied")
                            .and_then(Value::as_bool),
                        Some(false),
                        "{} should not apply failed/unavailable vision transcription",
                        original_name
                    );
                    assert!(
                        report
                            .pointer("/parser/visionTranscription/failure")
                            .is_some_and(|failure| !failure.is_null()),
                        "{} should report the unavailable vision gateway while preserving source review",
                        original_name
                    );
                } else {
                    let ir: Value = read_json(&job_path.join("authoring-ir.json")).unwrap();
                    assert!(
                        ir.get("groups")
                            .and_then(Value::as_array)
                            .map(|groups| !groups.is_empty())
                            .unwrap_or(false),
                        "{} may skip full-page vision transcription only after local text produced editable groups",
                        original_name
                    );
                }
            } else {
                authoring_or_llm_required += 1;
                assert_eq!(saved.status, JobStatus::NeedsReview);
                assert_eq!(saved.current_step, WorkflowStep::Authoring);
                assert_eq!(
                    report.get("currentStep").and_then(Value::as_str),
                    Some("Authoring")
                );
                let llm_failed = report
                    .pointer("/llm/failures")
                    .and_then(Value::as_array)
                    .map(|failures| !failures.is_empty())
                    .unwrap_or(false);
                let remaining_review_items = report
                    .pointer("/authoring/remainingReviewItems")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                assert!(
                    llm_failed || remaining_review_items > 0,
                    "{} should explain why the editable draft still needs confirmation",
                    original_name
                );
            }

            let _ = fs::remove_dir_all(root);
        }

        assert!(
            source_review_required >= 3,
            "mixed image/text samples should still require source review"
        );
        assert!(
            authoring_or_llm_required >= 1,
            "at least one fully text-layer sample should require authoring or LLM review"
        );
    }
}
