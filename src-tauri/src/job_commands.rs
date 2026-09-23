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

/// 永久删除一个 job 的全部落盘产物与题库行。
///
/// 从 `delete_job_core` 里抽出来只为了可测：删除的**完整性**（job 目录 + 受管音频 +
/// 题库行）是产品语义，不该只有拿得到 `AppHandle` 才能验。
///
/// 顺序与容错是刻意的：
/// - job 目录删失败 ⇒ 如实失败（用户要删的东西还在）；
/// - 受管音频 / 题库行删失败 ⇒ 只记日志。它们不在 job 目录里，用户看不到，
///   为了它们让「删除」整体失败只会让用户以为没删掉，从而反复点击。
pub(crate) fn delete_job_artifacts(root: &std::path::Path, job_id: &str) -> CommandResult<()> {
    let dir = job_dir(root, job_id);
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|error| error.to_string())?;
    }
    // 受管音频**不在** job 目录里（音频是最终版的一部分，job 目录只是过程产物），
    // 所以上面那句删不掉它。不在这里显式清理就是永久泄漏：文件躺在磁盘上、表里留着行，
    // 用户既看不到也删不掉。
    if let Err(error) = crate::listening_audio::store::purge_item_audio(root, job_id) {
        eprintln!("[listening_audio] purge failed for {}: {}", job_id, error);
    }
    // 同步删除题库 DB 中的记录（失败记日志但不阻断文件删除——文件已删，DB 孤儿可被迁移/重试清理）。
    if let Err(error) = crate::db::delete_exam_by_id(root, job_id) {
        eprintln!("[library] delete_exam_by_id failed for {}: {}", job_id, error);
    }
    Ok(())
}

pub(crate) async fn delete_job_core(job_id: String, app: AppHandle) -> CommandResult<()> {
    let root = app_root(&app)?;
    delete_job_artifacts(&root, &job_id)
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

    /// 永久删除一个条目 = job 目录 + **受管音频** + 题库行，一样都不能留。
    ///
    /// 受管音频刻意不在 job 目录里（job 目录装过程产物、发布后会被清理；音频是最终版
    /// 的一部分），所以只删 job 目录就是永久泄漏：文件躺在磁盘上、表里留着行，
    /// 用户既看不到也删不掉。而「顺手多删一点」的代价更大——删掉别人的音频是不可逆的。
    #[test]
    fn permanent_delete_takes_the_managed_audio_with_it_and_nothing_else() {
        use crate::library::repository::{open_library_connection, upsert_item_shell, UpsertItemInput};
        use crate::listening_audio::store::{audio_status, bind_audio};

        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        let conn = open_library_connection(&root).unwrap();
        for id in ["job-doomed", "job-kept"] {
            upsert_item_shell(
                &conn,
                &UpsertItemInput {
                    id,
                    modality: "listening",
                    title: "L",
                    status: "ready",
                    source_asset_id: None,
                },
            )
            .unwrap();
        }
        drop(conn);

        let outside = root.join("user-audio");
        fs::create_dir_all(&outside).unwrap();
        let doomed = outside.join("Section 1.wav");
        crate::test_support::write_audio_fixture(&doomed, 440.0);
        let kept = outside.join("Section 2.wav");
        crate::test_support::write_audio_fixture(&kept, 880.0);
        bind_audio(&root, "job-doomed", 1, &doomed).unwrap();
        bind_audio(&root, "job-kept", 1, &kept).unwrap();

        let doomed_job_dir = crate::util::job_dir(&root, "job-doomed");
        fs::create_dir_all(&doomed_job_dir).unwrap();
        fs::write(doomed_job_dir.join("job.json"), b"{}").unwrap();
        let doomed_audio_dir = root.join("audio").join("job-doomed");
        let kept_audio_dir = root.join("audio").join("job-kept");
        assert!(doomed_audio_dir.is_dir() && kept_audio_dir.is_dir());

        delete_job_artifacts(&root, "job-doomed").unwrap();

        assert!(!doomed_job_dir.exists(), "job 目录必须删掉");
        assert!(
            !doomed_audio_dir.exists(),
            "被永久删除的条目不该留下受管音频——那是永久泄漏"
        );
        let conn = open_library_connection(&root).unwrap();
        let doomed_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM listening_audio_assets_v1 WHERE item_id = 'job-doomed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(doomed_rows, 0, "表行必须跟着删");
        drop(conn);

        // 另一个条目一个字都不许动。
        assert!(kept_audio_dir.is_dir(), "永久删除一个条目不能碰别的条目");
        assert!(
            audio_status(&root, "job-kept").unwrap().audio_ready,
            "另一个条目的音频必须仍然可用"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// T5-e：同目录放 3 份 PDF，只用「选择文件」选 1 份 ⇒ 恰好 1 个条目、只处理这一份。
    ///
    /// 为什么必须把两条入口放在**同一个目录**上对照：「恰好 1 份」本身无法排除
    /// 「目录里本来就只有 1 份」这种平凡解释。而两条入口的选择语义本来就不同：
    ///   - 「选择文件」  → `automation_source_files_from_env`：**只**返回清单里那几份，
    ///     目录里多出来的文件与它无关（用户在系统对话框里逐份挑过）；
    ///   - 「选择 PDF 文件夹」→ `list_pdf_files_in_dir`：把目录里的 PDF **全列出来**，
    ///     用户没有逐份挑过。
    /// 所以这里同时断言「文件入口 1 份」与「目录入口 3 份」，两者在同一次运行里互证。
    ///
    /// 而且不止断言钩子返回了什么：把钩子的返回值直接喂给真实导入命令
    /// `import_files_at_root`（UI 点「开始导入」走的就是它），再断言题库只多出 1 行、
    /// job 目录只多出 1 个、落地文件只有被选中的那一份 ——「只处理这一份」说的是
    /// **真的没有去落地/解析另外两份**，而不是「钩子少返回了两份」。
    #[test]
    fn choosing_one_file_imports_only_that_file_even_though_the_directory_holds_three() {
        use crate::processing::commands::{import_files_at_root, ImportFileInput, ImportFilesInput};

        // 环境变量是进程级的，而测试默认并行。本文件里目前只有这一条测试碰这对钩子变量，
        // 但仍加锁：将来有人给「目录入口」也写测试时会用同一个进程的同一片环境。
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        let source_dir = root.join("incoming");
        fs::create_dir_all(&source_dir).unwrap();

        // 三份 PDF 放同一个目录。选**中间**那一份：选第一份时「只导入 1 份」与
        // 「只导入了排在最前的」无法区分，中间那份才能排除「按顺序只取第一个」。
        let names = ["alpha.pdf", "bravo.pdf", "charlie.pdf"];
        for name in names {
            fs::write(source_dir.join(name), format!("%PDF-1.4 {name}")).unwrap();
        }
        let chosen = source_dir.join("bravo.pdf");

        let hook_key = "PDF2TEST_AUTOMATION_SOURCE_FILES";
        let previous = env::var_os(hook_key);
        // 「选择文件」钩子只拿到被选中的那一份。
        env::set_var(hook_key, env::join_paths([&chosen]).unwrap());

        let picked = automation_source_files_from_env()
            .unwrap()
            .expect("设置了钩子就必须返回清单，而不是回落到原生对话框");
        assert_eq!(
            picked.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(),
            vec!["bravo.pdf"],
            "「选择文件」只能带进被选中的那一份"
        );
        assert_eq!(Path::new(&picked[0].path), chosen.as_path());

        // 对照：同一个目录走「选择 PDF 文件夹」，三份都会被列出来。
        // 这一条不是「顺手多验一个功能」——它是上面「恰好 1 份」的反平凡证据。
        let via_folder = list_pdf_files_in_dir(source_dir.clone()).unwrap();
        assert_eq!(
            via_folder.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(),
            names.to_vec(),
            "目录入口本来就该把三份都列出来；列不出来说明这次对照不成立"
        );

        // 把钩子的返回值原样喂给真实导入命令（UI「开始导入」走的就是它）。
        let input = ImportFilesInput {
            files: picked
                .iter()
                .map(|file| ImportFileInput {
                    path: file.path.clone(),
                    name: file.name.clone(),
                    size_bytes: file.size_bytes,
                    title_hint: Some(file.title_hint.clone()),
                })
                .collect(),
            cloud_enabled: Some(false),
            cloud_profile_id: None,
            modality: None,
        };
        let result = import_files_at_root(&root, input).unwrap();
        // 立刻还原环境变量：后面的断言失败也不该把钩子留给别的测试。
        match previous {
            Some(value) => env::set_var(hook_key, value),
            None => env::remove_var(hook_key),
        }

        assert!(
            result.rejected.is_empty(),
            "不该有被拒的文件：{:?}",
            result.rejected.iter().map(|r| &r.reason).collect::<Vec<_>>()
        );
        assert_eq!(result.created.len(), 1, "选 1 份就只能建立 1 个条目");
        assert_eq!(result.created[0].title, "bravo");

        // 磁盘事实：恰好 1 个 job 目录，且只落地了被选中的那一份。
        let job_dirs: Vec<PathBuf> = fs::read_dir(root.join("jobs"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect();
        assert_eq!(job_dirs.len(), 1, "只处理 1 份文件就只能有 1 个 job 目录");
        let uploads: Vec<String> = fs::read_dir(job_dirs[0].join("uploads"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(uploads.len(), 1, "只该落地 1 份文件，实际 {uploads:?}");
        assert!(
            uploads[0].ends_with("bravo.pdf"),
            "落地的必须是被选中的那一份，实际 {uploads:?}"
        );
        for other in ["alpha", "charlie"] {
            assert!(
                !uploads.iter().any(|name| name.contains(other)),
                "没被选中的 {other} 不该出现在任何 job 目录里：{uploads:?}"
            );
        }

        let _ = fs::remove_dir_all(&root);
    }
}
