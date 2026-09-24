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

/// `bind_listening_audio` 命令的完整写路径：台账行 → 权威稿镜像 → 读回确认。
///
/// 抽成自由函数是为了让并发回归测试逐字跑**同一条命令路径**——测试若自己重新拼装
/// 「bind_audio + 同步」，就会跟真实命令的次序与内容脱钩（评审指定反例必须在
/// 命令处理器层复现，见 `canonical_media.rs` 的三十次扫掠测试）。
pub(crate) fn bind_audio_command_path(
    root: &Path,
    item_id: &str,
    part_ordinal: i64,
    path: &Path,
) -> CommandResult<(store::ListeningAudioAssetV1, super::canonical_media::AudioMediaSyncV1)> {
    let bound = store::bind_audio(root, item_id, part_ordinal, path)?;
    // Mirror onto the canonical draft's part `media` through a real edit
    // transaction, so preview/export/student runtime read one document.
    let sync = super::canonical_media::sync_item_audio_media(root, item_id)?;
    // 台账写成功 ≠ 绑定成功：预览/导出/学生端只读权威稿里的 part media。
    // 稿已存在而这一 part 的 media 没跟上（镜像被人工保护挡住、或写入没落盘）时
    // 必须如实报错——返回 Ok 会让界面说「已添加」，而音频根本读不到。
    super::canonical_media::ensure_part_media_matches(root, item_id, part_ordinal, &bound)?;
    Ok((bound, sync))
}

#[tauri::command]
pub(crate) async fn bind_listening_audio(input: BindListeningAudioInput, app: AppHandle) -> CommandResult<Value> {
    let root = crate::app_root(&app)?;
    allow_managed_audio(&app, &root);
    let item_id = input.item_id.clone();
    let part_ordinal = input.part_ordinal;
    let path = input.path.clone();
    let bound = blocking(move || bind_audio_command_path(&root, &item_id, part_ordinal, Path::new(&path)))
        .await?;
    to_value(bound.0)
}

#[tauri::command]
pub(crate) async fn bind_listening_audio_folder(item_id: String, folder: String, app: AppHandle) -> CommandResult<Value> {
    let root = crate::app_root(&app)?;
    allow_managed_audio(&app, &root);
    let bound = blocking(move || {
        let bound = store::bind_folder(&root, &item_id, Path::new(&folder))?;
        super::canonical_media::sync_item_audio_media(&root, &item_id)?;
        Ok(bound)
    })
    .await?;
    to_value(bound)
}

#[tauri::command]
pub(crate) async fn unbind_listening_audio(item_id: String, part_ordinal: i64, app: AppHandle) -> CommandResult<Value> {
    let root = crate::app_root(&app)?;
    let removed = blocking(move || {
        let removed = store::unbind_audio(&root, &item_id, part_ordinal)?;
        super::canonical_media::sync_item_audio_media(&root, &item_id)?;
        Ok(removed)
    })
    .await?;
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

/// 真实 Tauri E2E 用的「音频选择」钩子。
///
/// 与 `job_commands::automation_source_files_from_env` 同一套路：钩子走的仍然是**产品命令**，
/// 只是把系统对话框换成环境变量给的清单。没装钩子时返回 `None`，前端照旧弹真实对话框——
/// 产品路径与自动化路径**共用同一个调用点**，不会出现「只有测试能过」的第二份实现。
#[tauri::command]
pub(crate) async fn automation_audio_selection_from_env() -> CommandResult<Value> {
    let selection = automation_audio_selection(
        std::env::var("PDF2TEST_AUTOMATION_AUDIO_FILES").ok(),
        std::env::var("PDF2TEST_AUTOMATION_AUDIO_DIR").ok(),
    );
    // 钩子必须指向**真实存在**的路径：让脚本立刻失败在钩子上，
    // 而不是伪装成下游的「音频解码失败 / 该文件夹里没有 MP3」。
    if let Some(files) = &selection.files {
        for file in files {
            if !Path::new(file).is_file() {
                return Err(format!(
                    "PDF2TEST_AUTOMATION_AUDIO_FILES 指向不存在的文件：{file}"
                ));
            }
        }
    }
    if let Some(folder) = &selection.folder {
        if !Path::new(folder).is_dir() {
            return Err(format!(
                "PDF2TEST_AUTOMATION_AUDIO_DIR 指向不存在的目录：{folder}"
            ));
        }
    }
    to_value(selection)
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AutomationAudioSelection {
    /// `None` = 没装「选文件」钩子 ⇒ 前端必须弹真实对话框。
    pub files: Option<Vec<String>>,
    /// `None` = 没装「选文件夹」钩子。
    pub folder: Option<String>,
}

/// 纯函数，便于直接断言：不碰进程环境，测试之间不会互相干扰。
pub(crate) fn automation_audio_selection(
    files_raw: Option<String>,
    folder_raw: Option<String>,
) -> AutomationAudioSelection {
    AutomationAudioSelection {
        files: automation_paths_from_raw(files_raw),
        folder: automation_path_from_raw(folder_raw),
    }
}

/// 空串 / 全空白 = **没装钩子**（而不是「装了但给空清单」）。
/// 这个区分很重要：否则前端会拿到空清单、跳过对话框，却又一份音频都没有。
fn automation_paths_from_raw(raw: Option<String>) -> Option<Vec<String>> {
    let raw = raw?;
    if raw.trim().is_empty() {
        return None;
    }
    let paths: Vec<String> = std::env::split_paths(&raw)
        .map(|path| path.to_string_lossy().to_string())
        .filter(|path| !path.trim().is_empty())
        .collect();
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

fn automation_path_from_raw(raw: Option<String>) -> Option<String> {
    let raw = raw?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_selection_hook_is_absent_until_it_is_installed() {
        let absent = automation_audio_selection(None, None);
        assert_eq!(absent.files, None);
        assert_eq!(absent.folder, None);

        // 空串是「没装」，不是「装了但什么都没有」——否则前端会跳过真实对话框。
        let blank = automation_audio_selection(Some("   ".to_string()), Some(String::new()));
        assert_eq!(blank.files, None);
        assert_eq!(blank.folder, None);
    }

    #[test]
    fn audio_selection_hook_splits_the_list_and_trims_the_folder() {
        let joined = std::env::join_paths([
            Path::new("/tmp/listening-part1.wav"),
            Path::new("/tmp/listening-part2.wav"),
        ])
        .expect("join_paths")
        .to_string_lossy()
        .to_string();

        let selection =
            automation_audio_selection(Some(joined), Some("  /tmp/listening-audio  ".to_string()));

        assert_eq!(
            selection.files,
            Some(vec![
                "/tmp/listening-part1.wav".to_string(),
                "/tmp/listening-part2.wav".to_string()
            ])
        );
        // 两侧空白必须吃掉：否则拿去查目录会查不到，钩子会被误判成「装错了」。
        assert_eq!(selection.folder, Some("/tmp/listening-audio".to_string()));
    }
}
