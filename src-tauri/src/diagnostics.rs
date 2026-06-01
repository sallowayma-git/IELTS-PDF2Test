use crate::util::{read_json, write_json};
use crate::CommandResult;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct DiagnosticsSettings {
    #[serde(rename = "keepFullProcessArtifacts")]
    pub keep_full_process_artifacts: bool,
}

fn diagnostics_settings_path(root: &Path) -> PathBuf {
    root.join("config").join("diagnostics-settings.json")
}

pub(crate) fn load_diagnostics_settings(root: &Path) -> CommandResult<DiagnosticsSettings> {
    let path = diagnostics_settings_path(root);
    if !path.exists() {
        return Ok(DiagnosticsSettings::default());
    }
    read_json(&path)
}

pub(crate) fn write_diagnostics_settings(
    root: &Path,
    settings: &DiagnosticsSettings,
) -> CommandResult<DiagnosticsSettings> {
    write_json(&diagnostics_settings_path(root), settings)?;
    Ok(settings.clone())
}
