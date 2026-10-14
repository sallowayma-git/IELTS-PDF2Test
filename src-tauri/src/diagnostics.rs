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

/// Keep crash location/backtrace without recording document text or credentials.
pub(crate) fn install_worker_panic_log(root: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let record = serde_json::json!({
            "occurredAt": chrono::Utc::now().to_rfc3339(),
            "thread": thread.name().unwrap_or("unnamed"),
            "location": info.location().map(|location| format!("{}:{}", location.file(), location.line())),
            "backtrace": std::backtrace::Backtrace::force_capture().to_string(),
        });
        if let Ok(line) = serde_json::to_string(&record) {
            let _ = crate::util::append_text(&root.join("logs").join("worker-panics.jsonl"), &format!("{line}\n"));
        }
        previous(info);
    }));
}
