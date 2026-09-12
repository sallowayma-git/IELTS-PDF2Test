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
        let result: CommandResult<String> = (|| {
            // 1. 建 job（元数据落 job.json，与既有链共用）。
            let job = make_job(CreateJobInput {
                title: Some(title.clone()),
                category: None,
                frequency: None,
                tags: None,
                llm_profile_id: None,
            });
            save_job(&root, &job)?;
            // 2. 文件落地：staging + hash 全部同步完成（数据库事务外，计划 §12.2）。
            crate::job_commands::stage_source_file(root, &job.job_id, &file.path, "MainQuestion")?;
            Ok(job.job_id)
        })();
        let job_id = match result {
            Ok(job_id) => job_id,
            Err(error) => {
                rejected.push(ImportRejectedFile { name: file.name.clone(), reason: error });
                continue;
            }
        };

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
            // 文件已落地但队列失败：明确失败状态，不留「永远 Processing」的悬挂行（计划 §12.2）。
            rejected.push(ImportRejectedFile { name: file.name.clone(), reason: error });
            if let Ok(conn) = open_library_connection(&root) {
                let _ = set_item_status(&conn, &job_id, "failed");
            }
            continue;
        }
        created.push(ImportCreatedItem { item_id: job_id, title });
    }

    Ok(ImportFilesResult { created, rejected })
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
