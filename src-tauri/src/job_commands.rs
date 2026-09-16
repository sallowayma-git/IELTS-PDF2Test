use crate::artifact_store::ensure_job_artifact_layout;
use crate::diagnostics::DiagnosticsSettings;
use crate::job_store::{list_saved_jobs, load_job, make_job, save_job, update_job};
use crate::llm_profiles::load_profiles;
use crate::llm_suggestions::load_llm_suggestions;
use crate::source_review::source_review_status_for_job;
use crate::util::{
    ensure_app_dirs, ensure_job_dirs, file_type_from_name, job_dir, read_json_opt,
    sanitize_filename, stage_file_with_hash,
};
use crate::{
    app_root, CommandResult, CreateJobInput, ImportJob, JobDetail, JobFilter, JobMetaPatch,
    JobStatus, SourceFile, WorkflowStep,
};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use std::{env, fs, path::PathBuf, time::SystemTime};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

/// Cleanup orphaned staged files that have no corresponding job reference.
/// Files older than 24 hours in uploads directories matching the staging pattern
/// are considered orphaned and removed.
pub(crate) fn cleanup_orphaned_staged_files(root: &std::path::Path) -> CommandResult<u32> {
    let jobs_root = root.join("jobs");
    if !jobs_root.exists() {
        return Ok(0);
    }

    // G1 对抗审计 P2：系统时钟早于 epoch+24h 时 checked_sub 防下溢回绕
    // （回绕会把 24h 窗口变成巨大正值，误删活跃 staging 文件）。
    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let cutoff = now_secs
        .checked_sub(24 * 60 * 60)
        .ok_or("orphan_cleanup_clock_before_window")?;

    let mut cleaned = 0;

    for entry in fs::read_dir(jobs_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let uploads_dir = entry.path().join("uploads");
        if !uploads_dir.exists() {
            continue;
        }

        for file_entry in fs::read_dir(&uploads_dir).map_err(|error| error.to_string())? {
            let file_entry = file_entry.map_err(|error| error.to_string())?;
            let file_path = file_entry.path();

            // Only process files matching the staging pattern
            if let Some(name) = file_path.file_name().and_then(|n| n.to_str()) {
                if !name.starts_with(".staging-") {
                    continue;
                }

                if let Ok(metadata) = fs::metadata(&file_path) {
                    if let Ok(modified) = metadata.modified() {
                        if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                            if duration.as_secs() < cutoff {
                                if fs::remove_file(&file_path).is_ok() {
                                    cleaned += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(cleaned)
}

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
    let stem = name
        .rsplit_once('.')
        .map(|(value, _)| value)
        .unwrap_or(name);
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

fn picked_source_path(path: PathBuf) -> CommandResult<PickedSourcePath> {
    if !path.is_file() {
        return Err(format!("automation_source_file_missing:{}", path.display()));
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("automation_source_file_name_invalid:{}", path.display()))?
        .to_string();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "pdf" | "docx" | "txt" | "md") {
        return Err(format!("automation_source_file_type_unsupported:{extension}"));
    }
    let size_bytes = fs::metadata(&path)
        .map_err(|error| error.to_string())?
        .len();
    Ok(PickedSourcePath {
        path: path.to_string_lossy().to_string(),
        title_hint: clean_file_stem(&name),
        name,
        size_bytes,
        requires_desktop_parser: false,
    })
}

/// Real-Tauri E2E hook for the normal "choose files" path. Absent the env var,
/// the frontend keeps using the native dialog unchanged.
pub(crate) fn automation_source_files_from_env() -> CommandResult<Option<Vec<PickedSourcePath>>> {
    let Ok(raw) = env::var("PDF2TEST_AUTOMATION_SOURCE_FILES") else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let mut files = Vec::new();
    for path in env::split_paths(&raw) {
        files.push(picked_source_path(path)?);
    }
    if files.is_empty() {
        Ok(None)
    } else {
        Ok(Some(files))
    }
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
        vision_answer_candidates: read_json_opt(&dir.join("vision-answer-candidates.json"))?,
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
    // 同步删除题库 DB 中的记录（失败记日志但不阻断文件删除——文件已删，DB 孤儿可被迁移/重试清理）。
    if let Err(error) = crate::db::delete_exam_by_id(&root, &job_id) {
        eprintln!(
            "[library] delete_exam_by_id failed for {}: {}",
            job_id, error
        );
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
    tauri::async_runtime::spawn_blocking(move || {
        stage_source_file(&root, &job_id, &file_path, &role)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn stage_source_file(
    root: &std::path::Path,
    job_id: &str,
    file_path: &str,
    role: &str,
) -> CommandResult<SourceFile> {
    let dir = job_dir(root, job_id);
    ensure_job_dirs(&dir)?;
    let input = PathBuf::from(&file_path);
    let original_name = input
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("source.pdf")
        .to_string();
    let uploads_dir = dir.join("uploads");
    let staging_name = format!(
        ".staging-{}-{}",
        Uuid::new_v4().simple(),
        sanitize_filename(&original_name)
    );
    let staging_path = uploads_dir.join(&staging_name);
    let (hash, size) = stage_file_with_hash(&input, &staging_path)?;
    let stored_name = format!("{}-{}", &hash[..8], sanitize_filename(&original_name));
    let final_path = uploads_dir.join(&stored_name);
    // G1 对抗审计 P1-2：final_path 已存在（同 hash 重导复用既有文件）时，
    // 该文件属于此前导入的 job——补偿删除只能针对本次新建的文件。
    let created_final = !final_path.exists();
    if !created_final {
        let _ = fs::remove_file(&staging_path);
    } else if let Err(error) = fs::rename(&staging_path, &final_path) {
        let _ = fs::remove_file(&staging_path);
        return Err(format!(
            "stage_source_file:{}:{}",
            final_path.display(),
            error
        ));
    }
    let source = SourceFile {
        file_id: format!("file-{}", Uuid::new_v4().simple()),
        original_name,
        stored_name,
        file_type: file_type_from_name(&file_path).to_string(),
        sha256: hash,
        size_bytes: size,
        role: role.to_string(),
        imported_at: Utc::now(),
    };

    // Fix A4-F01: If update_job fails, delete the staged file to prevent orphans
    if let Err(error) = update_job(&root, &job_id, |job| {
        job.source_files.push(source.clone());
        job.status = JobStatus::Working;
        job.current_step = WorkflowStep::DocumentReview;
    }) {
        if created_final {
            let _ = fs::remove_file(&final_path);
        }
        return Err(error);
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use uuid::Uuid;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("orphan-cleanup-{}", Uuid::new_v4().simple()))
    }

    /// G1/A4-F01：启动期孤儿清理只删超过 24h 的 `.staging-` 遗留文件，
    /// 活跃导入（新 mtime）与非 staging 产物不受影响。
    #[test]
    fn cleanup_removes_only_stale_staging_files() {
        let root = temp_root();
        let uploads = root.join("jobs").join("job-1").join("uploads");
        fs::create_dir_all(&uploads).unwrap();

        let stale = uploads.join(".staging-old-deadbeef.pdf");
        fs::write(&stale, b"stale").unwrap();
        let fresh = uploads.join(".staging-fresh-cafebabe.pdf");
        fs::write(&fresh, b"fresh").unwrap();
        let kept = uploads.join("abcd1234-final.pdf");
        fs::write(&kept, b"final").unwrap();
        // 非 uploads 目录下的同名文件不受影响。
        let stray = root.join("jobs").join("job-1").join(".staging-elsewhere.pdf");
        fs::write(&stray, b"stray").unwrap();

        // 把 stale 的 mtime 拨回 2 天前（std FileTimes，无需额外依赖）。
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let stale_time = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(now_secs - 2 * 24 * 60 * 60);
        let file = fs::OpenOptions::new().write(true).open(&stale).unwrap();
        file.set_times(fs::FileTimes::new().set_modified(stale_time)).unwrap();

        let removed = cleanup_orphaned_staged_files(&root).unwrap();
        assert_eq!(removed, 1, "只清理超过 24h 的 .staging- 文件");
        assert!(!stale.exists());
        assert!(fresh.exists(), "新 staged 文件不得误删");
        assert!(kept.exists(), "非 staging 产物不得误删");
        assert!(stray.exists(), "uploads 之外的文件不在清理范围");
        let _ = fs::remove_dir_all(&root);
    }
}
