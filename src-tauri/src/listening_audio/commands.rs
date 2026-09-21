//! Tauri command surface for listening import. Kept here (not in lib.rs) so the only
//! lib.rs change is the handler registration.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::store;
use crate::CommandResult;

fn to_value<T: serde::Serialize>(value: T) -> CommandResult<Value> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> CommandResult<T> + Send + 'static,
) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|error| error.to_string())?
}

/// The webview plays managed audio through the asset protocol. The static scope covers
/// `$APPDATA/audio/**`; the data root can be relocated (automation override), so the
/// actual managed directory is allowed at runtime as well. Nothing outside it is exposed.
fn allow_managed_audio(app: &AppHandle, root: &Path) {
    let dir = store::audio_root(root);
    let _ = std::fs::create_dir_all(&dir);
    if let Err(error) = app.asset_protocol_scope().allow_directory(&dir, true) {
        eprintln!("[listening_audio] asset scope: {error}");
    }
}

#[tauri::command]
pub(crate) async fn detect_import_modality(paths: Vec<String>) -> CommandResult<Value> {
    let detections = blocking(move || Ok(super::detect::detect_import_modality(&paths))).await?;
    to_value(detections)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BindListeningAudioInput {
    pub item_id: String,
    pub part_ordinal: i64,
    pub path: String,
}

#[tauri::command]
pub(crate) async fn bind_listening_audio(input: BindListeningAudioInput, app: AppHandle) -> CommandResult<Value> {
    let root = crate::app_root(&app)?;
    allow_managed_audio(&app, &root);
    let bound = blocking(move || store::bind_audio(&root, &input.item_id, input.part_ordinal, Path::new(&input.path))).await?;
    to_value(bound)
}

#[tauri::command]
pub(crate) async fn bind_listening_audio_folder(item_id: String, folder: String, app: AppHandle) -> CommandResult<Value> {
    let root = crate::app_root(&app)?;
    allow_managed_audio(&app, &root);
    let bound = blocking(move || store::bind_folder(&root, &item_id, Path::new(&folder))).await?;
    to_value(bound)
}

#[tauri::command]
pub(crate) async fn unbind_listening_audio(item_id: String, part_ordinal: i64, app: AppHandle) -> CommandResult<Value> {
    let root = crate::app_root(&app)?;
    let removed = blocking(move || store::unbind_audio(&root, &item_id, part_ordinal)).await?;
    to_value(removed)
}

/// Bindings + readiness. `verify: true` re-probes each managed file against its hash.
#[tauri::command]
pub(crate) async fn get_listening_audio(item_id: String, verify: Option<bool>, app: AppHandle) -> CommandResult<Value> {
    let root = crate::app_root(&app)?;
    allow_managed_audio(&app, &root);
    let status = blocking(move || {
        if verify.unwrap_or(false) {
            store::verify_bindings(&root, &item_id)
        } else {
            store::audio_status(&root, &item_id)
        }
    })
    .await?;
    to_value(status)
}

/// `.mp3` files of a picked folder in natural order (dialog preview; nothing is copied).
#[tauri::command]
pub(crate) async fn list_listening_audio_folder(folder: String) -> CommandResult<Value> {
    let files: Vec<PathBuf> = blocking(move || store::list_folder_mp3(Path::new(&folder))).await?;
    to_value(
        files
            .iter()
            .map(|path| {
                serde_json::json!({
                    "path": path.to_string_lossy(),
                    "name": path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default(),
                    "sizeBytes": std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
                })
            })
            .collect::<Vec<_>>(),
    )
}

/// Probe user-selected files before import (duration + issue codes per file).
#[tauri::command]
pub(crate) async fn probe_listening_audio_files(paths: Vec<String>) -> CommandResult<Value> {
    let results = blocking(move || Ok(store::probe_files(&paths))).await?;
    to_value(results)
}
