//! Tauri command surface for listening import. Kept here (not in lib.rs) so the only
//! lib.rs change is the handler registration.

use serde_json::Value;

use crate::CommandResult;

#[tauri::command]
pub(crate) async fn detect_import_modality(paths: Vec<String>) -> CommandResult<Value> {
    let detections = tauri::async_runtime::spawn_blocking(move || super::detect::detect_import_modality(&paths))
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_value(detections).map_err(|error| error.to_string())
}