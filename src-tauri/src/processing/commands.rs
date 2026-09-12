//! M2（原 P6-T01 前半 + 计划 §12.2）：`import_files` —— 后端接管批量导入调度。
//!
//! 每份文件独立处理：建 job → 文件落地（hash 在事务外）→ library 外壳 → 入队。
//! 单文件失败只影响该文件（rejected），其余照常建行并入队；批量建行在秒级完成，
//! 解析由调度器在后台按并发上限执行。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::job_store::{make_job, save_job};
use crate::library::repository::{open_library_connection, set_item_status, upsert_item_shell, UpsertItemInput};
use crate::{app_root, CreateJobInput, CommandResult};

const MAX_IMPORT_FILE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportFileInput {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub title_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportFilesInput {
    pub files: Vec<ImportFileInput>,
    #[serde(default)]
    pub cloud_enabled: Option<bool>,
    #[serde(default)]
    pub cloud_profile_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportCreatedItem {
    pub item_id: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportRejectedFile {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportFilesResult {
    pub created: Vec<ImportCreatedItem>,
    pub rejected: Vec<ImportRejectedFile>,
}

pub(crate) async fn import_files_core(app: &AppHandle, input: ImportFilesInput) -> CommandResult<ImportFilesResult> {
    let root = app_root(app)?;
    tauri::async_runtime::spawn_blocking(move || import_files_at_root(&root, input))
        .await
        .map_err(|error| error.to_string())?
}

pub(crate) fn import_files_at_root(root: &std::path::Path, input: ImportFilesInput) -> CommandResult<ImportFilesResult> {
    let cloud_enabled = input.cloud_enabled.unwrap_or(false);
    let mut created = Vec::new();
    let mut rejected = Vec::new();

    for file in input.files {
        let title = file
            .title_hint
            .clone()
            .filter(|hint| !hint.trim().is_empty())
            .unwrap_or_else(|| file.name.trim_end_matches(['.', ' ']).to_string());
        let title = {
            let stem = file.name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(&file.name);
            if title.trim().is_empty() { stem.to_string() } else { title }
        };

        if file.size_bytes > MAX_IMPORT_FILE_BYTES {
            rejected.push(ImportRejectedFile {
                name: file.name.clone(),
                reason: format!("超过导入上限 {}", format_bytes(MAX_IMPORT_FILE_BYTES)),
            });
            continue;
        }
        // 1. 建 job（元数据落 job.json，与既有链共用）。
        let job = make_job(CreateJobInput {
            title: Some(title.clone()),
            category: None,
            frequency: None,
            tags: None,
            llm_profile_id: None,
        });
        let job_id = job.job_id.clone();
        let staged = (|| -> CommandResult<()> {
            save_job(&root, &job)?;
            // 2. 文件落地：staging + hash 全部同步完成（数据库事务外，计划 §12.2）。
            crate::job_commands::stage_source_file(root, &job_id, &file.path, "MainQuestion")?;
            Ok(())
        })();
        if let Err(error) = staged {
            // G1 边界：save_job/staging 失败同样会留下 job 壳（目录 + job.json），
            // 与 queue 失败同口径补偿清理，不留磁盘孤儿。
            rejected.push(ImportRejectedFile { name: file.name.clone(), reason: error });
            compensate_failed_import(root, &job_id);
            continue;
        }

        // 3. 数据库：library 外壳 + 处理任务入队（两个独立短写，文件操作已在外完成）。
        let queue_result: CommandResult<()> = queue_import(
            &root,
            &job_id,
            &title,
            &file.name,
            cloud_enabled,
            input.cloud_profile_id.as_deref(),
        );
        if let Err(error) = queue_result {
            // G1/A4-F01：queue 失败必须补偿——磁盘 job 目录（含 staged 文件）
            // 与可能残留的 DB 行一并清除，不留「磁盘有 job、DB 无可见行」的孤儿；
            // 以 rejected 明确告知用户（原始文件未被移动或修改）。
            rejected.push(ImportRejectedFile { name: file.name.clone(), reason: error });
            compensate_failed_import(root, &job_id);
            continue;
        }
        created.push(ImportCreatedItem { item_id: job_id, title });
    }

    Ok(ImportFilesResult { created, rejected })
}

/// G1/A4-F01 补偿删除：queue_import 失败后清空本次导入的痕迹。
/// queue_import 是单事务，正常失败时 DB 无行；commit 歧义时可能有行残留，
/// 因此 DB 清理尽力而为。job 目录（job.json + uploads staged 文件）一并删除。
fn compensate_failed_import(root: &std::path::Path, job_id: &str) {
    let dir = crate::util::job_dir(root, job_id);
    if dir.exists() {
        if let Err(error) = std::fs::remove_dir_all(&dir) {
            eprintln!("[import] compensate: remove job dir {job_id} failed: {error}");
        }
    }
    if let Ok(conn) = open_library_connection(root) {
        if let Err(error) = conn.execute(
            "DELETE FROM processing_jobs_v2 WHERE id = ?1",
            [job_id],
        ) {
            eprintln!("[import] compensate: delete queue row {job_id} failed: {error}");
        }
        if let Err(error) = conn.execute(
            "DELETE FROM library_items_v2 WHERE id = ?1",
            [job_id],
        ) {
            eprintln!("[import] compensate: delete item shell {job_id} failed: {error}");
        }
    }
}

/// 数据库收尾：library 外壳 + 处理任务入队（独立短写）。
fn queue_import(
    root: &std::path::Path,
    job_id: &str,
    title: &str,
    file_name: &str,
    cloud_enabled: bool,
    cloud_profile_id: Option<&str>,
) -> CommandResult<()> {
    let conn = open_library_connection(root)?;
    let transaction = rusqlite::Transaction::new_unchecked(&conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    upsert_item_shell(
        &transaction,
        &UpsertItemInput {
            id: job_id,
            modality: "reading",
            title,
            status: "processing",
            source_asset_id: None,
        },
    )?;
    set_item_status(&transaction, job_id, "processing")?;
    super::queue::enqueue(
        &transaction,
        job_id,
        job_id,
        job_id,
        &json!({
            "cloudEnabled": cloud_enabled,
            "fileName": file_name,
            "cloudProfileId": cloud_profile_id
        }),
    )?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

/// import_files 的返回转 Value 供 Tauri 命令层使用。
pub(crate) fn import_files_result_to_value(result: ImportFilesResult) -> Value {
    serde_json::to_value(result).unwrap_or(json!({"created": [], "rejected": []}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("import-comp-{}", Uuid::new_v4().simple()))
    }

    /// G1/A4-F01：queue 失败（DB 打不开）时补偿删除 job 目录与 DB 残留，
    /// 不留「磁盘有 job、DB 无可见行」的孤儿；用户拿到明确的 rejected 结果。
    #[test]
    fn queue_failure_compensates_and_rejects() {
        let root = temp_root();
        fs::create_dir_all(&root).unwrap();
        // 失败注入：把 authoring_hub.db 占位成目录 → Connection::open 必然失败。
        fs::create_dir_all(root.join("authoring_hub.db")).unwrap();
        let source = root.join("sample.pdf");
        fs::write(&source, b"%PDF-1.4 fake").unwrap();

        let input = ImportFilesInput {
            files: vec![ImportFileInput {
                path: source.to_string_lossy().to_string(),
                name: "sample.pdf".to_string(),
                size_bytes: 13,
                title_hint: None,
            }],
            cloud_enabled: Some(false),
            cloud_profile_id: None,
        };
        let result = import_files_at_root(&root, input).unwrap();
        assert!(result.created.is_empty(), "queue 失败的文件不得计入 created");
        assert_eq!(result.rejected.len(), 1, "失败必须以 rejected 明确告知");
        let jobs_dir = root.join("jobs");
        let leftover = fs::read_dir(&jobs_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftover, 0, "queue 失败后不得留下 job 目录孤儿");
        let _ = fs::remove_dir_all(&root);
    }

    /// G1 边界：staging 本身失败（源文件不存在）时，save_job 已写下的
    /// job 壳同样要被补偿清理——与 queue 失败同口径，不留磁盘孤儿。
    #[test]
    fn staging_failure_compensates_job_shell() {
        let root = temp_root();
        fs::create_dir_all(&root).unwrap();

        let input = ImportFilesInput {
            files: vec![ImportFileInput {
                path: root.join("missing-source.pdf").to_string_lossy().to_string(),
                name: "missing-source.pdf".to_string(),
                size_bytes: 0,
                title_hint: None,
            }],
            cloud_enabled: Some(false),
            cloud_profile_id: None,
        };
        let result = import_files_at_root(&root, input).unwrap();
        assert!(result.created.is_empty());
        assert_eq!(result.rejected.len(), 1, "staging 失败必须以 rejected 明确告知");
        let jobs_dir = root.join("jobs");
        let leftover = fs::read_dir(&jobs_dir)
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(leftover, 0, "staging 失败后不得留下 job 壳目录");
        let _ = fs::remove_dir_all(&root);
    }
}
