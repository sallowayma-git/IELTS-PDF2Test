use authoring_commands::{
    apply_manual_transcription_core, apply_vision_transcription_core, build_authoring_ir_core,
    parse_document_core, render_group_html_core, resolve_source_review_core, run_rule_split_core,
    save_split_adjustments_core, update_authoring_ir_core,
};
use auto_pipeline::run_auto_pipeline_core;
use chrono::{DateTime, Utc};
use diagnostics::DiagnosticsSettings;
use export_pack::{build_pack_core, export_reading_assets_core};
use llm_commands::{
    apply_llm_suggestion_core, llm_run_group_core, save_llm_profile_core, test_llm_profile_core,
};
use preview_commands::{
    generate_preview_assets_core, run_preview_e2e_core, validate_authoring_ir_core,
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{env, path::PathBuf};
use util::ensure_app_dirs;
mod authoring_commands;
mod authoring_pipeline;
mod authoring_review;
mod authoring_validation;
mod auto_pipeline;
mod cleanup;
mod diagnostics;
mod environment;
mod export_artifacts;
mod export_pack;
mod job_commands;
mod job_store;
mod llm_commands;
mod llm_gateway;
mod llm_profiles;
mod llm_suggestions;
mod parser;
mod preview_commands;
mod reading_source;
mod runtime_validation;
mod source_review;
mod util;
mod validator;
mod workflow_state;
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
async fn parse_document(
    job_id: String,
    options: ParseOptions,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    parse_document_core(&root, &job_id, options)
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
    apply_vision_transcription_core(&root, &job_id, input)
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
async fn run_rule_split(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    run_rule_split_core(&root, &job_id)
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
async fn build_authoring_ir(job_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    build_authoring_ir_core(&root, &job_id)
}

#[tauri::command]
async fn update_authoring_ir(job_id: String, patch: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    update_authoring_ir_core(&root, &job_id, patch)
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
async fn save_llm_profile(input: SaveLlmProfileInput, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    save_llm_profile_core(&root, input)
}

#[tauri::command]
async fn test_llm_profile(profile_id: String, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    test_llm_profile_core(&root, &profile_id)
}

#[tauri::command]
async fn llm_classify_group(
    job_id: String,
    group_id: String,
    profile_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    llm_run_group_core(&root, &job_id, &group_id, &profile_id, "classify_group")
}

#[tauri::command]
async fn llm_extract_group(
    job_id: String,
    group_id: String,
    profile_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    llm_run_group_core(&root, &job_id, &group_id, &profile_id, "extract_group")
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
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    export_reading_assets_core(&root, &job_id, &export_dir, true)
}

#[tauri::command]
async fn build_pack(input: Value, app: AppHandle) -> CommandResult<Value> {
    let root = app_root(&app)?;
    build_pack_core(&root, &input, true)
}

#[tauri::command]
async fn run_auto_pipeline(
    job_id: String,
    input: Option<AutoPipelineInput>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    run_auto_pipeline_core(&root, &job_id, input)
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
            run_environment_preflight,
            get_diagnostics_settings,
            save_diagnostics_settings,
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
        parse_source_document, parser_failure_document_ir, render_pdf_page_with_sips_fallback,
        vision_transcription_document_ir,
    };
    use crate::reading_source::reading_source;
    use crate::runtime_validation::{publish_readiness_gate, validate_for_runtime_gate};
    use crate::source_review::{
        low_confidence_block_ids, parser_warnings, source_review_fingerprint, source_review_issues,
        source_review_status, write_source_review_status,
    };
    use crate::util::{
        ensure_job_dirs, file_type_from_name, hash_file_or_path, is_safe_path_segment, job_dir,
        read_json, read_json_opt, safe_job_dir, sanitize_filename, validate_path_segment,
        write_bytes, write_json, write_text,
    };
    use crate::workflow_state::apply_preview_e2e_job_state;
    use std::{
        fs,
        path::Path,
        sync::{Mutex, OnceLock},
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
    fn unsafe_path_segments_are_rejected() {
        assert!(is_safe_path_segment("import-20260531120000-abcdef12"));
        assert!(is_safe_path_segment("pack.fixture-01"));

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
    fn build_pack_core_rejects_unsafe_pack_and_job_ids_before_paths() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();

        let unsafe_pack = json!({
            "packId": "../pack",
            "jobIds": ["missing-job"]
        });
        let pack_error = build_pack_core(&root, &unsafe_pack, false).unwrap_err();
        assert!(pack_error.contains("invalid_pack_id_path_segment"));

        let unsafe_job = json!({
            "packId": "pack-safe",
            "jobIds": ["../job"]
        });
        let job_error = build_pack_core(&root, &unsafe_job, false).unwrap_err();
        assert!(job_error.contains("invalid_job_id_path_segment"));

        let _ = fs::remove_dir_all(root);
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
            "python3",
            "python:pypdf",
            "renderer:macos-sips",
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
        let image_count = image_count_from_extraction(&extraction);

        assert!(image_count > 0);
        assert!(extraction
            .get("warnings")
            .and_then(Value::as_array)
            .map(|warnings| warnings.is_empty())
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
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].get("mimeType").and_then(Value::as_str),
            Some("image/png")
        );
        assert!(images[0]
            .get("renderedFallback")
            .and_then(Value::as_bool)
            .unwrap_or(false));
        assert!(PathBuf::from(images[0].get("path").and_then(Value::as_str).unwrap()).exists());
        assert!(extraction
            .get("warnings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|warning| warning
                .as_str()
                .unwrap_or_default()
                .contains("sips rendered-page fallback")));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rust_sips_fallback_renders_pdf_without_python_extraction() {
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

        let extraction = render_pdf_page_with_sips_fallback(
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
    fn rust_contract_validator_rejects_unsupported_group_kind() {
        let mut source = contract_fixture_source();
        source["questionGroups"][0]["kind"] = json!("made_up_kind");

        let messages = contract_messages(&source);

        assert!(messages.contains("made_up_kind is not an allowed group kind"));
    }

    #[test]
    fn rust_contract_validator_rejects_missing_answer_key_coverage() {
        let mut source = contract_fixture_source();
        source["answerKey"].as_object_mut().unwrap().remove("q2");

        let messages = contract_messages(&source);

        assert!(messages.contains("q2 is missing from answerKey"));
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
            .expect("no-text PDF fixture should parse through Rust PDF extractor");

        assert_eq!(
            ir.pointer("/parser/provider").and_then(Value::as_str),
            Some("rust-parser:pdf:pdf-extract")
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
            Some("LlmReview")
        );
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert_eq!(saved.current_step, WorkflowStep::LlmReview);
        assert!(saved.issue_counts.needs_review > 0);
        assert!(job_dir(&root, &job.job_id)
            .join("document-ir.json")
            .exists());
        assert!(job_dir(&root, &job.job_id)
            .join("split-candidates.json")
            .exists());
        assert!(job_dir(&root, &job.job_id)
            .join("authoring-ir.json")
            .exists());
        assert!(job_dir(&root, &job.job_id)
            .join("pipeline-report.json")
            .exists());
        assert!(!job_dir(&root, &job.job_id)
            .join("cleanup-summary.json")
            .exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn auto_pipeline_high_confidence_llm_auto_applies_without_human_verification() {
        let root = temp_test_root();
        ensure_app_dirs(&root).unwrap();
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
                profile_id: None,
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
    fn auto_pipeline_keeps_no_text_pdf_blocked_for_source_review() {
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
            Some(true)
        );
        assert!(
            report
                .pointer("/parser/visionTranscription/applied")
                .and_then(Value::as_bool)
                == Some(true)
                || report
                    .pointer("/parser/visionTranscription/failure")
                    .and_then(Value::as_str)
                    .is_some()
        );
        assert_eq!(
            report.get("status").and_then(Value::as_str),
            Some("NeedsReview")
        );
        assert_eq!(
            report.get("currentStep").and_then(Value::as_str),
            Some("DocumentReview")
        );
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert_eq!(saved.current_step, WorkflowStep::DocumentReview);
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
    fn auto_pipeline_routes_source_review_before_llm_review_when_both_block() {
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
            Some("DocumentReview")
        );
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert_eq!(saved.current_step, WorkflowStep::DocumentReview);

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
        let split: Value =
            read_json(&job_dir(&root, &job.job_id).join("split-candidates.json")).unwrap();
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/requiresManualQuestionImport")
                .and_then(Value::as_bool),
            Some(true)
        );
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
            Some("Manual import required for question 14")
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
        assert!(job_path.join("cleanup-summary.json").exists());
        assert!(!job_path.join("document-ir.json").exists());
        assert!(!job_path.join("split-candidates.json").exists());
        assert!(!job_path.join("preview").exists());

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
        assert!(!job_path.join("authoring-project.json").exists());
        assert!(job_path.join("document-ir.json").exists());
        assert!(job_path.join("split-candidates.json").exists());
        assert!(job_path.join("publish-readiness-report.json").exists());
        assert_eq!(
            load_job(&root, &job.job_id).unwrap().status,
            JobStatus::Working
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn build_pack_core_writes_zip_after_static_runtime_gate() {
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
            JobStatus::Cleaned
        );
        assert_eq!(
            result
                .pointer("/cleanup/0/cleaned")
                .and_then(Value::as_bool),
            Some(true)
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

    #[test]
    fn build_pack_publish_gate_failure_writes_no_pack_or_cleanup() {
        let root = temp_test_root();
        let (job, mut ir) = make_publishable_fixture(&root);
        if let Some(audit) = ir.get_mut("audit").and_then(Value::as_object_mut) {
            audit.insert("humanVerified".to_string(), json!(false));
        }
        write_json(&job_dir(&root, &job.job_id).join("authoring-ir.json"), &ir).unwrap();
        let input = json!({
            "packId": "blocked-pack",
            "version": "0.1.0",
            "institution": "internal",
            "description": "blocked",
            "jobIds": [job.job_id]
        });

        let error = build_pack_core(&root, &input, true).unwrap_err();

        assert!(error.contains("pack_validation_failed"));
        assert!(error.contains("$.audit.humanVerified"));
        assert!(!root.join("packs").join("blocked-pack.zip").exists());
        assert!(!root
            .join("packs")
            .join("blocked-pack")
            .join("pack.json")
            .exists());
        let job_path = job_dir(&root, &job.job_id);
        assert!(!job_path.join("cleanup-summary.json").exists());
        assert!(!job_path.join("authoring-project.json").exists());
        assert!(job_path.join("document-ir.json").exists());
        assert!(job_path.join("split-candidates.json").exists());
        assert!(job_path.join("publish-readiness-report.json").exists());
        assert_eq!(
            load_job(&root, &job.job_id).unwrap().status,
            JobStatus::Working
        );

        let _ = fs::remove_dir_all(root);
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

    #[test]
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
            assert_eq!(
                doc.pointer("/parser/provider").and_then(Value::as_str),
                Some("rust-parser:pdf:pdf-extract"),
                "{} should use Rust PDF text-layer parser",
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
                assert!(
                    !needs_vision,
                    "{} is fully text-layer readable and should not enter the vision path",
                    sample.display()
                );
                assert!(
                    !review_required,
                    "{} is fully text-layer readable and should not require source review",
                    sample.display()
                );
            }

            let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
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
}
