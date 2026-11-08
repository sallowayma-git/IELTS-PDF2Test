//! 流水线模式开关（cloud-direct 方案 §2）。
//!
//! `pipelineMode` 决定一份 PDF 导入后走哪条识别链：
//! - `direct`（默认）：在线且有可用云端配置时，云端一次直出整份题稿
//!   （[`crate::cloud_direct`]），不再等本地首稿、不再逐组合并；
//! - `legacy`：现有本地首稿 + 云端候选 + reconcile/adoption 链，行为不变。
//!
//! 优先级：job 入队时写入的 `progress.pipelineMode`（用户在导入当时的设置快照）
//! > 全局设置文件 `{root}/config/pipeline-settings.json` > 默认 `direct`。
//! 这样切换设置不会改变已在处理中的作业的链路，重跑旧作业也不会悄悄换链。

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{app_root, CommandResult};

pub const PIPELINE_SETTINGS_FILE: &str = "pipeline-settings.json";
pub const PIPELINE_SETTINGS_SCHEMA: &str = "PipelineSettingsV1";
pub const PIPELINE_MODE_DIRECT: &str = "direct";
pub const PIPELINE_MODE_LEGACY: &str = "legacy";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineMode {
    /// 云端直出主链（新）。
    Direct,
    /// 现有本地 + 云端候选链（保持行为不变）。
    Legacy,
}

impl Default for PipelineMode {
    fn default() -> Self {
        Self::Direct
    }
}

impl PipelineMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Direct => PIPELINE_MODE_DIRECT,
            Self::Legacy => PIPELINE_MODE_LEGACY,
        }
    }

    /// 解析job progress / 前端传来的字符串。无法识别时返回 `None`，
    /// 由调用方决定回退（不静默猜一个模式）。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            PIPELINE_MODE_DIRECT => Some(Self::Direct),
            PIPELINE_MODE_LEGACY => Some(Self::Legacy),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSettingsV1 {
    pub schema_version: String,
    pub pipeline_mode: PipelineMode,
}

impl Default for PipelineSettingsV1 {
    fn default() -> Self {
        Self {
            schema_version: PIPELINE_SETTINGS_SCHEMA.to_string(),
            pipeline_mode: PipelineMode::default(),
        }
    }
}

fn settings_path(root: &Path) -> std::path::PathBuf {
    root.join("config").join(PIPELINE_SETTINGS_FILE)
}

/// 读取全局流水线设置。文件不存在或不可解析时返回默认值（`direct`），
/// 语义与 `diagnostics settings` 一致：缺省不阻断处理。
pub(crate) fn load_pipeline_settings(root: &Path) -> CommandResult<PipelineSettingsV1> {
    let path = settings_path(root);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PipelineSettingsV1::default())
        }
        Err(error) => return Err(format!("pipeline_settings_read_failed:{error}")),
    };
    match serde_json::from_str::<PipelineSettingsV1>(&content) {
        // serde 只接受 direct / legacy 两个值；schemaVersion 不匹配时按新默认值修正。
        Ok(settings) => Ok(settings),
        // 旧文件/损坏文件不猜测模式：回落默认值并在调用方可见（写入时会被修正）。
        _ => Ok(PipelineSettingsV1::default()),
    }
}

pub(crate) fn save_pipeline_settings(
    root: &Path,
    settings: &PipelineSettingsV1,
) -> CommandResult<()> {
    let path = settings_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("pipeline_settings_dir_create:{error}"))?;
    }
    let mut normalized = settings.clone();
    normalized.schema_version = PIPELINE_SETTINGS_SCHEMA.to_string();
    let value = serde_json::to_value(&normalized)
        .map_err(|error| format!("pipeline_settings_encode_failed:{error}"))?;
    crate::artifact_store::write_canonical_json_atomic(&path, &value)
        .map(|_| ())
        .map_err(|error| format!("pipeline_settings_write_failed:{error}"))
}

/// 当前生效的模式：app root 由调用方解析后读全局设置。
pub(crate) fn resolve_pipeline_mode(app: &tauri::AppHandle) -> PipelineMode {
    let Ok(root) = app_root(app) else {
        return PipelineMode::default();
    };
    load_pipeline_settings(&root)
        .map(|settings| settings.pipeline_mode)
        .unwrap_or_default()
}

/// 从 job progress 读导入时确定的模式；没有或无法识别时用全局设置兜底。
pub(crate) fn pipeline_mode_for_job(
    root: &Path,
    job_progress: &serde_json::Value,
) -> PipelineMode {
    if let Some(mode) = job_progress
        .get("pipelineMode")
        .and_then(serde_json::Value::as_str)
        .and_then(PipelineMode::parse)
    {
        return mode;
    }
    load_pipeline_settings(root).map(|settings| settings.pipeline_mode).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "pdf2test-pipeline-settings-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(root.join("config")).unwrap();
        root
    }

    #[test]
    fn default_is_direct() {
        assert_eq!(PipelineMode::default(), PipelineMode::Direct);
        assert_eq!(PipelineSettingsV1::default().pipeline_mode, PipelineMode::Direct);
    }

    #[test]
    fn mode_round_trips_through_wire_names() {
        assert_eq!(PipelineMode::parse("direct"), Some(PipelineMode::Direct));
        assert_eq!(PipelineMode::parse("legacy"), Some(PipelineMode::Legacy));
        assert_eq!(PipelineMode::parse(""), None);
        assert_eq!(PipelineMode::parse("cloud"), None);
        assert_eq!(PipelineMode::Direct.as_str(), "direct");
        assert_eq!(PipelineMode::Legacy.as_str(), "legacy");
    }

    #[test]
    fn missing_file_falls_back_to_direct() {
        let root = temp_root("missing");
        let settings = load_pipeline_settings(&root).unwrap();
        assert_eq!(settings.pipeline_mode, PipelineMode::Direct);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn save_and_load_round_trip() {
        let root = temp_root("round-trip");
        save_pipeline_settings(
            &root,
            &PipelineSettingsV1 {
                schema_version: "Stale".to_string(),
                pipeline_mode: PipelineMode::Legacy,
            },
        )
        .unwrap();
        let loaded = load_pipeline_settings(&root).unwrap();
        assert_eq!(loaded.pipeline_mode, PipelineMode::Legacy);
        assert_eq!(loaded.schema_version, PIPELINE_SETTINGS_SCHEMA);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn corrupted_file_does_not_block_processing() {
        let root = temp_root("corrupt");
        std::fs::write(root.join("config/pipeline-settings.json"), "{not json").unwrap();
        let settings = load_pipeline_settings(&root).unwrap();
        assert_eq!(settings.pipeline_mode, PipelineMode::Direct);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn job_progress_wins_over_global_setting() {
        let root = temp_root("progress");
        save_pipeline_settings(&root, &PipelineSettingsV1::default()).unwrap();
        let progress = serde_json::json!({"pipelineMode": "legacy"});
        assert_eq!(
            pipeline_mode_for_job(&root, &progress),
            PipelineMode::Legacy
        );
        let empty = serde_json::json!({});
        assert_eq!(
            pipeline_mode_for_job(&root, &empty),
            PipelineMode::Direct
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
