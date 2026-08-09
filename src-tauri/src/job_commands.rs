use crate::artifact_store::ensure_job_artifact_layout;
use crate::diagnostics::DiagnosticsSettings;
use crate::job_store::{list_saved_jobs, load_job, make_job, save_job, update_job};
use crate::llm_profiles::load_profiles;
use crate::llm_suggestions::load_llm_suggestions;
use crate::source_review::source_review_status_for_job;
use crate::util::{
    ensure_app_dirs, ensure_job_dirs, file_type_from_name, hash_file_or_path, job_dir,
    read_json_opt, sanitize_filename,
};
use crate::{
    app_root, CommandResult, CreateJobInput, ImportJob, JobDetail, JobFilter, JobMetaPatch,
    JobStatus, SourceFile, WorkflowStep,
};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use std::{env, fs, path::PathBuf};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickedSourcePath {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub title_hint: String,
    pub requires_desktop_parser: bool,
}

fn clean_file_stem(name: &str) -> String {
    let stem = name.rsplit_once('.').map(|(value, _)| value).unwrap_or(name);
    stem.replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn list_pdf_files_in_dir(dir: PathBuf) -> CommandResult<Vec<PickedSourcePath>> {
    let mut files = fs::read_dir(&dir)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false)
        })
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("source.pdf")
                .to_string();
            let size_bytes = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            PickedSourcePath {
                path: path.to_string_lossy().to_string(),
                title_hint: clean_file_stem(&name),
                name,
                size_bytes,
                requires_desktop_parser: false,
            }
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(files)
}

pub(crate) async fn create_import_job_core(
    input: CreateJobInput,
    app: AppHandle,
) -> CommandResult<ImportJob> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    let job = make_job(input);
    let dir = job_dir(&root, &job.job_id);
    ensure_job_dirs(&dir)?;
    ensure_job_artifact_layout(&root, &job.job_id)?;
    save_job(&root, &job)?;
    Ok(job)
}

pub(crate) async fn list_jobs_core(
    filter: Option<JobFilter>,
    app: AppHandle,
) -> CommandResult<Vec<ImportJob>> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    list_saved_jobs(&root, filter)
}

pub(crate) async fn get_job_core(job_id: String, app: AppHandle) -> CommandResult<JobDetail> {
    let root = app_root(&app)?;
    let dir = job_dir(&root, &job_id);
    Ok(JobDetail {
        job: load_job(&root, &job_id)?,
        document_ir: read_json_opt(&dir.join("document-ir.json"))?,
        source_review: Some(source_review_status_for_job(&root, &job_id)?),
        split_candidates: read_json_opt(&dir.join("split-candidates.json"))?,
        authoring_ir: read_json_opt(&dir.join("authoring-ir.json"))?,
        validation_report: read_json_opt(&dir.join("validation-report.json"))?,
        preview_assets: read_json_opt(&dir.join("preview").join("preview-assets.json"))?,
        pipeline_report: read_json_opt(&dir.join("pipeline-report.json"))?,
        llm_suggestions: load_llm_suggestions(&root, &job_id)?,
    })
}

pub(crate) async fn update_job_meta_core(
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

pub(crate) async fn delete_job_core(job_id: String, app: AppHandle) -> CommandResult<()> {
    let root = app_root(&app)?;
    let dir = job_dir(&root, &job_id);
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) async fn import_source_file_core(
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
        job.status = JobStatus::Working;
        job.current_step = WorkflowStep::DocumentReview;
    })?;
    Ok(source)
}

pub(crate) async fn reveal_job_folder_core(job_id: String, app: AppHandle) -> CommandResult<()> {
    let root = app_root(&app)?;
    let dir = job_dir(&root, &job_id);
    if !dir.exists() {
        return Err("job_folder_missing".to_string());
    }
    tauri_plugin_opener::open_path(dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|error| error.to_string())
}

pub(crate) async fn choose_export_dir_core(app: AppHandle) -> CommandResult<Option<String>> {
    if let Ok(path) = env::var("PDF2TEST_AUTOMATION_EXPORT_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_string()));
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose export directory")
            .set_can_create_directories(true)
            .blocking_pick_folder()
            .map(|path| {
                path.into_path()
                    .map(|value| value.to_string_lossy().to_string())
                    .map_err(|error| error.to_string())
            })
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) async fn pick_pdf_folder_sources_core(
    app: AppHandle,
) -> CommandResult<Vec<PickedSourcePath>> {
    if let Ok(path) = env::var("PDF2TEST_AUTOMATION_PDF_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return list_pdf_files_in_dir(PathBuf::from(trimmed));
        }
    }
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose PDF folder")
            .set_can_create_directories(false)
            .blocking_pick_folder()
            .map(|path| {
                path.into_path()
                    .map_err(|error| error.to_string())
                    .and_then(list_pdf_files_in_dir)
            })
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(selected.unwrap_or_default())
}

pub(crate) async fn list_llm_profiles_core(app: AppHandle) -> CommandResult<Vec<Value>> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    load_profiles(&root)
}

pub(crate) async fn run_environment_preflight_core() -> CommandResult<Value> {
    Ok(crate::environment::environment_preflight_report())
}

pub(crate) async fn get_diagnostics_settings_core(
    app: AppHandle,
) -> CommandResult<DiagnosticsSettings> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    crate::diagnostics::load_diagnostics_settings(&root)
}

pub(crate) async fn save_diagnostics_settings_core(
    settings: DiagnosticsSettings,
    app: AppHandle,
) -> CommandResult<DiagnosticsSettings> {
    let root = app_root(&app)?;
    ensure_app_dirs(&root)?;
    crate::diagnostics::write_diagnostics_settings(&root, &settings)
}
