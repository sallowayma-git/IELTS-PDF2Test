//! Bind managed listening audio into the canonical draft's per-part `media`.
//!
//! The managed-audio table (`listening_audio_assets_v1`) is the binding authority;
//! the IR only mirrors it so preview, export and the student runtime can compile
//! from the canonical document alone. Two things are mirrored, and they must move
//! together: the part's `media` reference and the matching audio entry in the
//! draft's `assets` list — `validate_listening_structure_media_v2` closes one
//! against the other, so writing only one of them is not a valid draft.
//!
//! The mirror is written through a **real edit transaction** (base-version CAS,
//! editor journal, machine origin) rather than a raw `UPDATE`, because:
//! - a stale writer must not clobber a draft the user saved in the meantime;
//! - the write must respect `human_protected_targets`, so a part whose media the
//!   user edited by hand is never silently overwritten by a re-bind;
//! - the journal is what makes the change visible to the workspace's rebase path.

use std::path::Path;

use serde_json::{json, Map, Value};

use crate::authoring_v2_commands::{refresh_quality_report, validate_authoring};
use crate::library::repository::{
    apply_editor_commands_tx_with, get_canonical_ds, open_library_connection,
    ApplyEditorCommandsInput, EditOrigin,
};
use crate::CommandResult;

use super::store::{list_bindings, ListeningAudioAssetV1};

/// Editor patch op that mirrors managed audio onto `listening.parts[].media`
/// (and the matching `assets` entry).
pub(crate) const SET_LISTENING_PART_MEDIA_OP: &str = "setListeningPartMedia";

/// 镜像遇版本冲突时的重试次数。导入期的镜像与播种后的对账会并行提交，必然有输家；
/// 每次尝试都从当前稿重算，因此重试是有界且收敛的（不需要无限重试）。
const AUDIO_MEDIA_SYNC_ATTEMPTS: usize = 5;

/// Stable asset id for a managed audio file.
///
/// Derived from the content hash, so identical bytes always resolve to the same
/// asset id and the NAS package builder reproduces it without extra state. The
/// relative path inside the package is `audio/<sha256>.<ext>`.
pub(crate) fn audio_asset_id(sha256: &str) -> String {
    format!("audio-{sha256}")
}

/// `part-3` -> `3`. Anything else is not a part id this module wrote.
pub(crate) fn part_ordinal_from_id(part_id: &str) -> Option<i64> {
    part_id.strip_prefix("part-")?.parse::<i64>().ok()
}

fn extension_of(managed_path: &str) -> String {
    Path::new(managed_path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "bin".to_string())
}

/// Package-relative path of a managed audio file. Shared with the NAS package
/// builder so the manifest and the copied resource always agree.
pub(crate) fn audio_relative_path(sha256: &str, extension: &str) -> String {
    format!("audio/{sha256}.{extension}")
}

fn probe_field(binding: &ListeningAudioAssetV1, key: &str) -> Option<Value> {
    let value = binding.probe.get(key)?;
    if value.is_null() {
        None
    } else {
        Some(value.clone())
    }
}

/// `ListeningPartMediaV2` value for one binding.
///
/// A blocked probe is written **as it is** (status + issue codes), never dropped:
/// the quality check has to see that this part's audio is not usable, and the UI
/// has to be able to offer "replace audio" for that exact part.
pub(crate) fn media_value(binding: &ListeningAudioAssetV1) -> Value {
    let mut media = Map::new();
    media.insert(
        "assetId".to_string(),
        json!(audio_asset_id(&binding.sha256)),
    );
    media.insert(
        "mime".to_string(),
        json!(binding
            .mime
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_string())),
    );
    media.insert(
        "durationMs".to_string(),
        json!(binding.duration_ms.unwrap_or(0).max(0) as u64),
    );
    media.insert("sha256".to_string(), json!(binding.sha256.clone()));
    for key in ["channels", "sampleRateHz"] {
        if let Some(value) = probe_field(binding, key) {
            media.insert(key.to_string(), value);
        }
    }
    if let Some(probe) = probe_field(binding, "probe") {
        media.insert("probe".to_string(), probe);
    }
    Value::Object(media)
}

/// `AssetDescriptorV2` value for one binding, so the draft's asset list closes
/// against the part media.
pub(crate) fn asset_value(binding: &ListeningAudioAssetV1) -> Value {
    let extension = extension_of(&binding.managed_path);
    let mut asset = Map::new();
    asset.insert(
        "assetId".to_string(),
        json!(audio_asset_id(&binding.sha256)),
    );
    asset.insert("kind".to_string(), json!("audio"));
    asset.insert(
        "mime".to_string(),
        json!(binding
            .mime
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_string())),
    );
    asset.insert(
        "relativePath".to_string(),
        json!(audio_relative_path(&binding.sha256, &extension)),
    );
    asset.insert("sha256".to_string(), json!(binding.sha256.clone()));
    asset.insert(
        "byteLength".to_string(),
        json!(binding.size_bytes.max(0) as u64),
    );
    if let Some(duration) = binding.duration_ms.filter(|value| *value > 0) {
        asset.insert("durationMs".to_string(), json!(duration as u64));
    }
    asset.insert("extractionMode".to_string(), json!("user_upload"));
    asset.insert(
        "altText".to_string(),
        json!(format!("Listening audio: {}", binding.original_name)),
    );
    Value::Object(asset)
}

fn desired_media_by_part(bindings: &[ListeningAudioAssetV1]) -> Map<String, Value> {
    let mut desired = Map::new();
    for binding in bindings {
        desired.insert(
            format!("part-{}", binding.part_ordinal),
            media_value(binding),
        );
    }
    desired
}

/// Part ids whose `media` this call would change, given the current draft.
pub(crate) fn changed_parts(authoring: &Value, bindings: &[ListeningAudioAssetV1]) -> Vec<String> {
    let desired = desired_media_by_part(bindings);
    let parts = authoring
        .pointer("/listening/parts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut changed = Vec::new();
    for part in parts {
        let Some(part_id) = part.get("partId").and_then(Value::as_str) else {
            continue;
        };
        let current = part.get("media").filter(|value| !value.is_null()).cloned();
        let next = desired.get(part_id).cloned();
        if current != next {
            changed.push(part_id.to_string());
        }
    }
    changed
}

/// True when the draft's `assets` list does not yet describe the bindings.
pub(crate) fn assets_need_update(authoring: &Value, bindings: &[ListeningAudioAssetV1]) -> bool {
    let current = authoring
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    bindings.iter().any(|binding| {
        let asset_id = audio_asset_id(&binding.sha256);
        current
            .iter()
            .find(|asset| asset.get("assetId").and_then(Value::as_str) == Some(asset_id.as_str()))
            != Some(&asset_value(binding))
    })
}

/// Merge the audio asset descriptors into `document["assets"]` (replace by id).
fn merge_audio_assets(document: &mut Value, assets: &[Value]) -> CommandResult<()> {
    let list = document
        .as_object_mut()
        .ok_or_else(|| "AUTHORING_PATCH_DOCUMENT_NOT_OBJECT".to_string())?
        .entry("assets".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let list = list
        .as_array_mut()
        .ok_or_else(|| "AUTHORING_PATCH_ASSETS_NOT_ARRAY".to_string())?;
    for asset in assets {
        let asset_id = asset.get("assetId").and_then(Value::as_str);
        match asset_id.and_then(|asset_id| {
            list.iter().position(|existing| {
                existing.get("assetId").and_then(Value::as_str) == Some(asset_id)
            })
        }) {
            Some(position) => list[position] = asset.clone(),
            None => list.push(asset.clone()),
        }
    }
    Ok(())
}

/// Apply the `setListeningPartMedia` patch. `media: null` clears the field.
pub(crate) fn apply_set_listening_part_media(
    document: &mut Value,
    patch: &Map<String, Value>,
) -> CommandResult<()> {
    let entries = patch
        .get("parts")
        .and_then(Value::as_array)
        .ok_or_else(|| "AUTHORING_PATCH_PARTS_REQUIRED:setListeningPartMedia".to_string())?;
    {
        let listening = document
            .get_mut("listening")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "AUTHORING_PATCH_LISTENING_MISSING".to_string())?;
        let parts = listening
            .get_mut("parts")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "AUTHORING_PATCH_LISTENING_PARTS_MISSING".to_string())?;
        for entry in entries {
            let part_id = entry
                .get("partId")
                .and_then(Value::as_str)
                .ok_or_else(|| "AUTHORING_PATCH_PART_ID_REQUIRED".to_string())?;
            let part = parts
                .iter_mut()
                .find(|part| part.get("partId").and_then(Value::as_str) == Some(part_id))
                .ok_or_else(|| format!("AUTHORING_PATCH_PART_NOT_FOUND:{part_id}"))?;
            let part = part
                .as_object_mut()
                .ok_or_else(|| format!("AUTHORING_PATCH_PART_NOT_OBJECT:{part_id}"))?;
            match entry.get("media") {
                None | Some(Value::Null) => {
                    part.remove("media");
                }
                Some(media) => {
                    part.insert("media".to_string(), media.clone());
                }
            }
        }
    }
    if let Some(assets) = patch.get("assets").and_then(Value::as_array) {
        merge_audio_assets(document, assets)?;
    }
    Ok(())
}

/// Result of mirroring managed audio onto the canonical draft.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AudioMediaSyncV1 {
    /// Part ids whose `media` changed. Empty means the draft was already in sync.
    pub updated_parts: Vec<String>,
    /// Edit version after the write; `None` when nothing had to change.
    pub edit_version: Option<i64>,
    /// Parts left untouched because a human edited their media by hand. The audio
    /// file is still bound in the managed table; only the mirror is refused, so
    /// the user keeps the value they set and the quality report says what is
    /// missing instead of the write silently losing.
    pub protected_parts: Vec<String>,
}

/// Mirror the managed bindings onto the canonical draft.
///
/// No-op (and no version bump) when the draft already matches. Otherwise one
/// machine-origin edit transaction, so a concurrent human save wins the CAS
/// instead of being overwritten.
///
/// **版本冲突要重试，不能当成失败**：镜像不是「把旧值写回去」，而是「让稿收敛到台账」——
/// 它每次都从**当前**稿重算该改哪几个 part，所以重试是安全且必然收敛的。而它天然会撞版本：
/// 导入期「绑一段就镜像一次」与「播种后的对账」并行，两条链各自读一个 base_version，
/// 提交时必有一条输掉 CAS。此前输了就返回 Err——调用方（命令）把它整个丢掉，于是那一段的
/// media 永久缺失：台账有行、稿里没有。实测在并发扫掠里稳定复现为「丢后缀」
/// （只镜像上 part-1/part-2，台账却是 4 行）。
///
/// 人工保护不受影响：每次尝试都在事务内重新计算 `protected_edits_json`，被人手工改过的
/// part 永远走 `protected_parts` 分支，不会被重试绕过。
pub(crate) fn sync_item_audio_media(root: &Path, item_id: &str) -> CommandResult<AudioMediaSyncV1> {
    let mut last_conflict = None;
    for _ in 0..AUDIO_MEDIA_SYNC_ATTEMPTS {
        match sync_item_audio_media_once(root, item_id) {
            Err(error) if error.starts_with("EDIT_VERSION_CONFLICT:") => {
                last_conflict = Some(error)
            }
            other => return other,
        }
    }
    // 重试到上限仍冲突：如实上抛最后一次的具体版本号，不悄悄当成功。
    Err(last_conflict.expect("循环内至少发生过一次版本冲突"))
}

/// 单次镜像尝试（见 [`sync_item_audio_media`]：冲突由调用方重试）。
fn sync_item_audio_media_once(root: &Path, item_id: &str) -> CommandResult<AudioMediaSyncV1> {
    let mut conn = open_library_connection(root)?;
    let Some((canonical, base_version)) = get_canonical_ds(&conn, item_id)? else {
        // No draft yet (import still running). The seed path applies bindings once
        // the draft exists; there is nothing to mirror onto here.
        return Ok(AudioMediaSyncV1 {
            updated_parts: Vec::new(),
            edit_version: None,
            protected_parts: Vec::new(),
        });
    };
    // **顺序不能反：先读稿、后读台账。**
    //
    // `changed_parts` 把「台账里没有这个 part」解读成「稿里也不该有 media」，于是补丁会带
    // `media: null` 去**删除**它。这把「台账快照」当成了「权威的全集」，而快照是会过期的：
    // 台账行由 `bind_audio` 先落、镜像后跑，两条链并发时，镜像完全可能读到一个**早于当前稿**
    // 的台账快照，于是它看到「part-3/part-4 还没绑」，转手就把**另一个绑定刚镜像进去的
    // media 删掉**。实测（30 次并发扫掠）：播种读到 1 行、写好 v1（只有 part-1），随后绑定链
    // 依次把 part-2/3/4 镜像到 v4，最后对账用一份**只有 1 行**的台账快照提交 v5——
    // 读回是 `["part-1"]`，part-2/3/4 的 media 被那次写入抹掉；台账 4 行、稿里 1 个。
    //
    // 为什么反过来就安全：稿里的 media **只可能由镜像写入**，而镜像必须先有台账行，
    // 因此「稿的 media 集合 ⊆ 台账行集合」始终成立（台账行只增不减，显式解绑才会删行）。
    // 于是「先读稿、再读台账」保证后读到的台账至少覆盖前一刻稿的 media 集合，
    // 不会再出现「因为快照旧了而误删」。真正被解绑的 part（台账行确实没了）仍然会被清掉。
    let bindings = list_bindings(root, item_id)?;
    let changed = changed_parts(&canonical, &bindings);
    // A part whose media a human edited is dropped from the batch *before* it is
    // built. Leaving it in would make the all-or-nothing transaction reject the
    // whole batch, so one hand-edited part would freeze every other part's audio.
    let protected =
        crate::library::repository::human_protected_targets(&conn, item_id, &canonical)?;
    let (protected_parts, changed): (Vec<String>, Vec<String>) = changed
        .into_iter()
        .partition(|part_id| protected.contains(part_id));
    let assets_changed = assets_need_update(&canonical, &bindings);
    if changed.is_empty() && !assets_changed {
        return Ok(AudioMediaSyncV1 {
            updated_parts: Vec::new(),
            edit_version: None,
            protected_parts,
        });
    }
    let desired = desired_media_by_part(&bindings);
    let entries = changed
        .iter()
        .map(|part_id| {
            json!({
                "partId": part_id,
                "media": desired.get(part_id).cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();
    let protected_parts_for_result = protected_parts.clone();
    let command = json!({
        "op": SET_LISTENING_PART_MEDIA_OP,
        "parts": entries,
        "assets": bindings.iter().map(asset_value).collect::<Vec<_>>(),
    });
    let result = apply_editor_commands_tx_with(
        &mut conn,
        &ApplyEditorCommandsInput {
            item_id: item_id.to_string(),
            base_version,
            request_id: None,
            commands: vec![command],
            title: None,
        },
        EditOrigin::ListeningAudio,
        None,
        &|document, patch| {
            let object = patch
                .as_object()
                .ok_or_else(|| "authoring_v2_patch_must_be_object".to_string())?;
            match object.get("op").and_then(Value::as_str) {
                Some(SET_LISTENING_PART_MEDIA_OP) => {
                    apply_set_listening_part_media(document, object)
                }
                other => Err(format!("AUTHORING_PATCH_UNSUPPORTED:{other:?}")),
            }
        },
        &|document| {
            refresh_quality_report(root, item_id, document)?;
            validate_authoring(document)
        },
        &|_, _| Ok(()),
    );
    let result = match result {
        Ok(result) => result,
        // The user edited this part's media by hand. Binding the file still
        // succeeded in the managed table; refusing the mirror keeps their value.
        Err(error) if error.starts_with("EDIT_PROTECTED_TARGET:") => {
            return Ok(AudioMediaSyncV1 {
                updated_parts: Vec::new(),
                edit_version: None,
                protected_parts: changed,
            });
        }
        Err(error) => return Err(error),
    };
    Ok(AudioMediaSyncV1 {
        updated_parts: changed,
        edit_version: Some(result.edit_version),
        protected_parts: protected_parts_for_result,
    })
}

/// 读回确认：权威稿**已经存在**时，这一 part 的 media 必须是刚刚绑定的那一段。
///
/// 台账（`listening_audio_assets_v1`）是绑定的权威，但预览 / 导出 / 学生端读的是权威稿里的
/// `listening.parts[].media`。两者可以不一致，而且不一致时**没有任何界面会说话**：
/// - 镜像被人工保护挡住（`protected_parts`）——这是刻意拒绝，但调用方此前把结果整个丢掉；
/// - 镜像因为版本冲突/校验失败没能落盘。
///
/// 所以绑定命令必须自己读回确认，而不是把「台账写成功」当成「绑定成功」。
///
/// 稿**还不存在**是合法的：导入期的识别可能还没产出首稿，此时镜像是**故意**空转的
/// （`sync_item_audio_media` 的 no-draft 分支），播种路径会补齐——所以这里返回 `Ok`。
pub(crate) fn ensure_part_media_matches(
    root: &Path,
    item_id: &str,
    part_ordinal: i64,
    bound: &ListeningAudioAssetV1,
) -> CommandResult<()> {
    let conn = open_library_connection(root)?;
    let Some((canonical, _)) = get_canonical_ds(&conn, item_id)? else {
        return Ok(());
    };
    let part_id = format!("part-{part_ordinal}");
    let media = canonical
        .pointer("/listening/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|part| part.get("partId").and_then(Value::as_str) == Some(part_id.as_str()))
        .and_then(|part| part.get("media"))
        .filter(|media| !media.is_null())
        .cloned();
    let Some(media) = media else {
        return Err(format!("LISTENING_AUDIO_MEDIA_NOT_MIRRORED:{part_id}"));
    };
    if !media
        .get("sha256")
        .and_then(Value::as_str)
        .is_some_and(|mirrored| mirrored.eq_ignore_ascii_case(&bound.sha256))
    {
        return Err(format!("LISTENING_AUDIO_MEDIA_NOT_MIRRORED:{part_id}"));
    }
    Ok(())
}

/// Pure mirror used by the seed path, where the draft is not in SQLite yet.
pub(crate) fn apply_bindings_to_authoring(
    authoring: &mut Value,
    bindings: &[ListeningAudioAssetV1],
) -> Vec<String> {
    let desired = desired_media_by_part(bindings);
    let mut changed = Vec::new();
    if let Some(parts) = authoring
        .pointer_mut("/listening/parts")
        .and_then(Value::as_array_mut)
    {
        for part in parts.iter_mut() {
            let Some(part_id) = part
                .get("partId")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            let Some(object) = part.as_object_mut() else {
                continue;
            };
            match desired.get(&part_id) {
                Some(media) => {
                    if object.get("media") != Some(media) {
                        object.insert("media".to_string(), media.clone());
                        changed.push(part_id);
                    }
                }
                None => {
                    if object.remove("media").is_some() {
                        changed.push(part_id);
                    }
                }
            }
        }
    }
    if assets_need_update(authoring, bindings) {
        let assets = bindings.iter().map(asset_value).collect::<Vec<_>>();
        let _ = merge_audio_assets(authoring, &assets);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(ordinal: i64, sha: &str, playable: bool) -> ListeningAudioAssetV1 {
        ListeningAudioAssetV1 {
            item_id: "item-1".to_string(),
            part_ordinal: ordinal,
            managed_path: format!("/tmp/{sha}.wav"),
            sha256: sha.to_string(),
            size_bytes: 1024,
            mime: Some("audio/wav".to_string()),
            duration_ms: Some(1000),
            probe: json!({
                "sha256": sha,
                "mime": "audio/wav",
                "durationMs": 1000,
                "channels": 1,
                "sampleRateHz": 16000,
                "probe": {
                    "status": if playable { "passed" } else { "blocked" },
                    "provider": "symphonia",
                    "providerVersion": "0.6.0",
                    "probedAt": "2026-09-22T00:00:00Z",
                    "issueCodes": if playable { json!([]) } else { json!(["AUDIO_NEAR_SILENT"]) }
                }
            }),
            original_name: "section.wav".to_string(),
            created_at: "2026-09-22T00:00:00Z".to_string(),
            playable,
            issue_codes: if playable {
                Vec::new()
            } else {
                vec!["AUDIO_NEAR_SILENT".to_string()]
            },
        }
    }

    fn draft() -> Value {
        json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "modality": "listening",
            "assets": [],
            "listening": {
                "scope": "complete_exam",
                "parts": [
                    {"partId": "part-1", "displayLabel": "SECTION 1", "expectedQuestionNumbers": [1], "taskIds": [], "sourceAnchors": []},
                    {"partId": "part-2", "displayLabel": "SECTION 2", "expectedQuestionNumbers": [11], "taskIds": [], "sourceAnchors": []}
                ],
                "playbackPolicy": {"mode": "practice"}
            }
        })
    }

    #[test]
    fn binding_audio_writes_the_part_media_and_unbinding_clears_it() {
        let mut value = draft();
        let changed = apply_bindings_to_authoring(&mut value, &[binding(1, &"a".repeat(64), true)]);
        assert_eq!(changed, vec!["part-1".to_string()]);
        let media = &value["listening"]["parts"][0]["media"];
        assert_eq!(media["assetId"], json!(audio_asset_id(&"a".repeat(64))));
        assert_eq!(media["sha256"], json!("a".repeat(64)));
        assert_eq!(media["probe"]["status"], json!("passed"));
        assert!(value["listening"]["parts"][1]["media"].is_null());

        let changed = apply_bindings_to_authoring(&mut value, &[]);
        assert_eq!(changed, vec!["part-1".to_string()]);
        assert!(value["listening"]["parts"][0].get("media").is_none());
    }

    #[test]
    fn the_asset_list_closes_against_the_part_media() {
        let mut value = draft();
        apply_bindings_to_authoring(&mut value, &[binding(1, &"a".repeat(64), true)]);
        let assets = value["assets"].as_array().unwrap();
        assert_eq!(assets.len(), 1);
        let asset = &assets[0];
        assert_eq!(asset["assetId"], json!(audio_asset_id(&"a".repeat(64))));
        assert_eq!(asset["kind"], json!("audio"));
        assert_eq!(
            asset["relativePath"],
            json!(format!("audio/{}.wav", "a".repeat(64)))
        );
        assert_eq!(asset["extractionMode"], json!("user_upload"));
        assert!(!assets_need_update(
            &value,
            &[binding(1, &"a".repeat(64), true)]
        ));
    }

    #[test]
    fn a_blocked_probe_is_written_as_is_never_dropped() {
        let mut value = draft();
        apply_bindings_to_authoring(&mut value, &[binding(2, &"b".repeat(64), false)]);
        let media = &value["listening"]["parts"][1]["media"];
        assert_eq!(media["probe"]["status"], json!("blocked"));
        assert_eq!(media["probe"]["issueCodes"], json!(["AUDIO_NEAR_SILENT"]));
    }

    #[test]
    fn media_is_only_rewritten_when_it_actually_differs() {
        let bindings = vec![binding(1, &"a".repeat(64), true)];
        let mut value = draft();
        assert_eq!(changed_parts(&value, &bindings).len(), 1);
        apply_bindings_to_authoring(&mut value, &bindings);
        assert!(changed_parts(&value, &bindings).is_empty());
        assert!(!assets_need_update(&value, &bindings));
    }

    #[test]
    fn patch_rejects_an_unknown_part_instead_of_silently_doing_nothing() {
        let mut value = draft();
        let patch = json!({"op": SET_LISTENING_PART_MEDIA_OP, "parts": [{"partId": "part-9", "media": null}]});
        let error =
            apply_set_listening_part_media(&mut value, patch.as_object().unwrap()).unwrap_err();
        assert!(
            error.starts_with("AUTHORING_PATCH_PART_NOT_FOUND"),
            "{error}"
        );
    }

    #[test]
    fn part_ordinal_parsing_is_strict() {
        assert_eq!(part_ordinal_from_id("part-4"), Some(4));
        assert_eq!(part_ordinal_from_id("part-x"), None);
        assert_eq!(part_ordinal_from_id("4"), None);
    }
}

#[cfg(test)]
mod media_sync_tests {
    use super::*;
    use crate::library::repository::{
        get_canonical_ds, open_library_connection, seed_canonical_ds, upsert_item_shell,
        UpsertItemInput,
    };
    use crate::listening_audio::store::{bind_audio, unbind_audio};
    use std::io::Write;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("audio-media-{}", Uuid::new_v4().simple()));
        crate::util::ensure_app_dirs(&root).unwrap();
        root
    }

    fn listening_draft() -> Value {
        let parts = (1..=4)
            .map(|ordinal| {
                json!({
                    "partId": format!("part-{ordinal}"),
                    "displayLabel": format!("SECTION {ordinal}"),
                    "expectedQuestionNumbers": [ordinal],
                    "taskIds": [],
                    "sourceAnchors": []
                })
            })
            .collect::<Vec<_>>();
        json!({
            "scope": "complete_exam",
            "parts": parts,
            "playbackPolicy": {
                "mode": "practice",
                "autoplay": false,
                "allowPause": true,
                "allowSeek": true,
                "allowReplay": true,
                "refreshBehavior": "resume_from_snapshot",
                "crashRecoveryBehavior": "resume_from_snapshot",
                "showCurrentTime": true,
                "showDuration": true
            }
        })
    }

    /// A schema-valid listening canonical draft, derived from the committed
    /// authoring fixture so `validate_authoring` (run inside the edit
    /// transaction) exercises the real contract.
    fn seed_listening_item(root: &Path, item_id: &str) {
        let conn = open_library_connection(root).unwrap();
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: item_id,
                modality: "listening",
                title: "Listening",
                status: "processing",
                source_asset_id: None,
            },
        )
        .unwrap();
        let mut ds: Value = serde_json::from_str(include_str!(
            "../../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        ds["modality"] = json!("listening");
        ds.as_object_mut().unwrap().remove("passage");
        ds["assets"] = json!([]);
        ds["listening"] = listening_draft();
        seed_canonical_ds(&conn, item_id, &ds.to_string(), "processing").unwrap();
        std::fs::create_dir_all(crate::util::job_dir(root, item_id)).unwrap();
    }

    fn tone(path: &Path, hz: f64) {
        let rate = 16_000_u32;
        let samples = (0..16_000)
            .map(|index| ((index as f64 / 16_000.0) * hz * std::f64::consts::TAU).sin() * 8_000.0)
            .map(|value| value as i16)
            .collect::<Vec<_>>();
        let data = (samples.len() * 2) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::File::create(path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }

    fn canonical(root: &Path, item_id: &str) -> Value {
        let conn = open_library_connection(root).unwrap();
        get_canonical_ds(&conn, item_id).unwrap().unwrap().0
    }

    fn hard_failures(ds: &Value) -> Vec<String> {
        ds.pointer("/quality/hardFailures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn managed_audio_reason(ds: &Value, asset_id: &str) -> Option<String> {
        ds.pointer("/quality/issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|issue| issue.get("targetId").and_then(Value::as_str) == Some(asset_id))
            .and_then(|issue| {
                issue
                    .pointer("/details/managedAudioReason")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
    }

    /// 真的绑好音频的听力条目：质量重算**不得**再报音频资产不存在。
    ///
    /// 这条是端到端的那一条：走真实台账（`bind_audio` 落盘 + 建行）、真实编辑事务里那次
    /// `refresh_quality_report`，再看落库的质量块。纯门禁单测证明不了「台账真的被读到了」。
    ///
    /// 反过来，受管文件被删掉之后必须如实失败——这才说明它不是「看见 user_upload 就放行」。
    #[test]
    fn a_bound_part_clears_the_asset_gate_and_a_deleted_file_blocks_it_again() {
        let root = temp_root();
        seed_listening_item(&root, "item-gate");
        // 四个 part 全绑（真实听力卷的样子），这样逐 part 的 LISTENING_AUDIO_MISSING 不会
        // 混进来干扰判断，剩下的差异只可能来自资产核对本身。
        for ordinal in 1..=4 {
            let source = root.join(format!("section-{ordinal}.wav"));
            tone(&source, 300.0 + ordinal as f64 * 60.0);
            bind_audio(&root, "item-gate", ordinal, &source).unwrap();
        }
        let bound = crate::listening_audio::store::list_bindings(&root, "item-gate")
            .unwrap()
            .into_iter()
            .find(|binding| binding.part_ordinal == 1)
            .expect("part 1 is bound");
        let asset_id = audio_asset_id(&bound.sha256);

        let sync = sync_item_audio_media(&root, "item-gate").unwrap();
        assert!(sync.updated_parts.contains(&"part-1".to_string()));

        let ds = canonical(&root, "item-gate");
        assert!(
            ds["assets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|asset| asset["assetId"] == json!(asset_id)),
            "绑定后音频必须进资产列表：{ds}"
        );
        let hard = hard_failures(&ds);
        assert!(
            !hard.contains(&"ASSET_REFERENCE_MISSING".to_string()),
            "已经绑好并核对通过的音频不该报资产不存在（reason={:?}）：{hard:?}",
            managed_audio_reason(&ds, &asset_id)
        );
        assert!(
            !hard.contains(&"ASSET_HASH_MISMATCH".to_string()),
            "{hard:?}"
        );
        assert!(
            !hard.contains(&"LISTENING_AUDIO_PROBE_BLOCKED".to_string()),
            "{hard:?}"
        );
        assert!(
            !hard.contains(&"LISTENING_AUDIO_MISSING".to_string()),
            "四个 part 都绑了，不该还有 part 缺音频：{hard:?}"
        );

        // 磁盘上的受管文件被删掉：核对要如实失败，并指出是「受管文件不在」。
        std::fs::remove_file(&bound.managed_path).unwrap();
        let mut ds = canonical(&root, "item-gate");
        refresh_quality_report(&root, "item-gate", &mut ds).unwrap();
        assert!(
            hard_failures(&ds).contains(&"ASSET_REFERENCE_MISSING".to_string()),
            "受管文件没了还报通过就是假绿：{ds}"
        );
        assert_eq!(
            managed_audio_reason(&ds, &asset_id).as_deref(),
            Some("managed_file_missing"),
            "{ds}"
        );

        // 文件放回去（内容一字不差）：又回到通过——不是「一旦红就永远红」。
        let source = root.join("section-1.wav");
        tone(&source, 360.0);
        let again = bind_audio(&root, "item-gate", 1, &source).unwrap();
        assert_eq!(again.sha256, bound.sha256, "同一段音频必须算出同一个 sha");
        let mut ds = canonical(&root, "item-gate");
        refresh_quality_report(&root, "item-gate", &mut ds).unwrap();
        assert!(
            !hard_failures(&ds).contains(&"ASSET_REFERENCE_MISSING".to_string()),
            "reason={:?} : {ds}",
            managed_audio_reason(&ds, &asset_id)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 受管音频被人改坏（内容变了）之后，凭台账记录的那个 sha 必须报哈希不一致。
    ///
    /// 这条盯的是「表里存的探测结论是上次绑定时写的」：不现场重跑探针，改坏的文件照样显示
    /// `passed`。
    #[test]
    fn a_tampered_managed_file_is_reported_as_a_hash_mismatch() {
        let root = temp_root();
        seed_listening_item(&root, "item-tamper");
        let source = root.join("section-1.wav");
        tone(&source, 440.0);
        let bound = bind_audio(&root, "item-tamper", 1, &source).unwrap();
        sync_item_audio_media(&root, "item-tamper").unwrap();

        std::fs::write(&bound.managed_path, b"not the audio you bound").unwrap();

        let mut ds = canonical(&root, "item-tamper");
        refresh_quality_report(&root, "item-tamper", &mut ds).unwrap();
        let hard = hard_failures(&ds);
        assert!(
            hard.contains(&"ASSET_HASH_MISMATCH".to_string()),
            "{hard:?} / {ds}"
        );
        assert_eq!(
            managed_audio_reason(&ds, &audio_asset_id(&bound.sha256)).as_deref(),
            Some("hash_mismatch"),
            "{ds}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn binding_audio_mirrors_onto_the_draft_through_the_edit_transaction() {
        let root = temp_root();
        seed_listening_item(&root, "item-media");
        let source = root.join("section-1.wav");
        tone(&source, 440.0);

        let bound = bind_audio(&root, "item-media", 1, &source).unwrap();
        let sync = sync_item_audio_media(&root, "item-media").unwrap();
        assert_eq!(sync.updated_parts, vec!["part-1".to_string()]);
        assert_eq!(sync.edit_version, Some(2), "one edit version for one bind");

        let ds = canonical(&root, "item-media");
        let media = &ds["listening"]["parts"][0]["media"];
        assert_eq!(media["sha256"], json!(bound.sha256));
        assert_eq!(media["assetId"], json!(audio_asset_id(&bound.sha256)));
        assert_eq!(media["probe"]["status"], json!("passed"));
        // The draft's asset list closes against the part media.
        let asset = ds["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["assetId"] == media["assetId"])
            .expect("audio asset descriptor");
        assert_eq!(asset["sha256"], json!(bound.sha256));
        assert_eq!(asset["kind"], json!("audio"));

        // Re-running is a no-op: no version churn, no journal entry.
        let again = sync_item_audio_media(&root, "item-media").unwrap();
        assert!(again.updated_parts.is_empty());
        assert_eq!(again.edit_version, None);

        // Unbinding clears the field through the same transaction.
        assert!(unbind_audio(&root, "item-media", 1).unwrap());
        let cleared = sync_item_audio_media(&root, "item-media").unwrap();
        assert_eq!(cleared.updated_parts, vec!["part-1".to_string()]);
        let ds = canonical(&root, "item-media");
        assert!(ds["listening"]["parts"][0].get("media").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_stale_base_version_loses_the_race_instead_of_overwriting_a_newer_save() {
        let root = temp_root();
        seed_listening_item(&root, "item-stale");
        let source = root.join("section-1.wav");
        tone(&source, 440.0);
        bind_audio(&root, "item-stale", 1, &source).unwrap();
        // Someone else saves first, bumping the version the sync was going to use.
        let conn = open_library_connection(&root).unwrap();
        conn.execute(
            "UPDATE library_items_v2 SET current_edit_version = 7 WHERE id = 'item-stale'",
            [],
        )
        .unwrap();
        let ds = canonical(&root, "item-stale");
        assert!(
            changed_parts(
                &ds,
                &crate::listening_audio::store::list_bindings(&root, "item-stale").unwrap()
            )
            .len()
                == 1
        );
        // The sync reads the current version itself, so it must succeed against 7
        // and land on 8 — never on a stale 2.
        let sync = sync_item_audio_media(&root, "item-stale").unwrap();
        assert_eq!(sync.edit_version, Some(8));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn four_bound_parts_close_the_media_contract() {
        use crate::schema::common::AssetDescriptorV2;
        use crate::schema::ielts_authoring_v2::ListeningStructureV2;
        use crate::schema::listening_runtime_v1::validate_listening_structure_media_v2;

        let root = temp_root();
        seed_listening_item(&root, "item-four");
        for ordinal in 1..=4 {
            let source = root.join(format!("section-{ordinal}.wav"));
            tone(&source, 300.0 + ordinal as f64 * 60.0);
            bind_audio(&root, "item-four", ordinal, &source).unwrap();
        }
        let sync = sync_item_audio_media(&root, "item-four").unwrap();
        assert_eq!(
            sync.updated_parts,
            vec!["part-1", "part-2", "part-3", "part-4"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );

        let ds = canonical(&root, "item-four");
        let structure: ListeningStructureV2 =
            serde_json::from_value(ds["listening"].clone()).expect("listening structure");
        let assets: Vec<AssetDescriptorV2> =
            serde_json::from_value(ds["assets"].clone()).expect("asset list");
        assert_eq!(structure.parts.len(), 4);
        assert!(
            structure.media.is_none(),
            "audio is per part, not exam-level"
        );
        assert_eq!(assets.len(), 4);
        assert_eq!(
            validate_listening_structure_media_v2(&structure, &assets),
            Vec::new(),
            "four bound parts must close the contract"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_human_edited_part_media_is_never_overwritten_by_a_rebind() {
        let root = temp_root();
        seed_listening_item(&root, "item-protected");
        {
            let conn = open_library_connection(&root).unwrap();
            conn.execute(
                "UPDATE library_items_v2 SET protected_edits_json = ?2 WHERE id = ?1",
                rusqlite::params!["item-protected", r#"{"targets":["part-1"]}"#],
            )
            .unwrap();
        }
        let source = root.join("section-1.wav");
        tone(&source, 440.0);
        let bound = bind_audio(&root, "item-protected", 1, &source).unwrap();
        assert!(bound.playable);

        let sync = sync_item_audio_media(&root, "item-protected").unwrap();
        assert!(sync.updated_parts.is_empty());
        assert_eq!(sync.protected_parts, vec!["part-1".to_string()]);
        let ds = canonical(&root, "item-protected");
        assert!(
            ds["listening"]["parts"][0].get("media").is_none(),
            "the protected part must keep the value the human set"
        );
        // A part nobody edited is still mirrored.
        let other = root.join("section-2.wav");
        tone(&other, 660.0);
        bind_audio(&root, "item-protected", 2, &other).unwrap();
        let sync = sync_item_audio_media(&root, "item-protected").unwrap();
        assert_eq!(sync.updated_parts, vec!["part-2".to_string()]);
        // part-1 is still not mirrored, so it is still reported as protected — the
        // UI has to keep saying so instead of the refusal disappearing silently.
        assert_eq!(sync.protected_parts, vec!["part-1".to_string()]);
        let ds = canonical(&root, "item-protected");
        assert!(ds["listening"]["parts"][1]["media"]["assetId"].is_string());
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── 导入期并发：边导边绑 vs 识别播种 ────────────────────────────────────

    /// 只建壳 + 写 job 目录里的 shadow 候选稿，**不**播种权威稿。
    ///
    /// 这正是「识别还在跑、用户已经在校验音频」那一刻的库状态：条目在、稿还没落。
    fn seed_item_awaiting_seed(root: &Path, item_id: &str) {
        let conn = open_library_connection(root).unwrap();
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: item_id,
                modality: "listening",
                title: "Listening",
                status: "processing",
                source_asset_id: None,
            },
        )
        .unwrap();
        let dir = crate::util::job_dir(root, item_id);
        std::fs::create_dir_all(&dir).unwrap();
        let mut ds: Value = serde_json::from_str(include_str!(
            "../../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        ds["modality"] = json!("listening");
        ds.as_object_mut().unwrap().remove("passage");
        ds["assets"] = json!([]);
        ds["listening"] = listening_draft();
        std::fs::write(
            dir.join(crate::authoring_v2_commands::AUTHORING_V2_SHADOW_FILE),
            ds.to_string(),
        )
        .unwrap();
    }

    /// 稿里**已经镜像上 media** 的 part id。
    fn mirrored_part_ids(ds: &Value) -> Vec<String> {
        ds.pointer("/listening/parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|part| {
                part.get("media")
                    .map(|media| !media.is_null())
                    .unwrap_or(false)
            })
            .filter_map(|part| {
                part.get("partId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect()
    }

    /// 导入期的真实并发：用户边导边绑 4 段音频，识别完成的**播种**同时写入。
    ///
    /// 这条盯的是一个真实发生过的缺陷：`ensure_initial_canonical`（播种路径）**只读一次**
    /// 音频台账，然后整体写稿；而 `sync_item_audio_media` 在「稿还不存在」时会**静默 no-op**
    /// （它假定「种子路径会补上」）。于是任何一段的 `bind`+镜像若恰好落在
    /// 「播种读台账 → 播种提交」这个窗口里，它的镜像就没做、之后也无人补做——台账里那一行
    /// 在（`listening_audio_assets_v1` 有 4 行），但**权威稿里那个 part 没有 media**，
    /// 预览 / 导出 / 学生端都读不到它的音频。
    ///
    /// 窗口很窄 ⇒ 每次只丢一个 part；窗口落点随机 ⇒ 丢的 part 会变（真实链路实测：
    /// 一次 part-4、一次 part-2）。所以这里按评审要求重复 30 次，并把播种时机的**偏移逐次
    /// 扫掠**过绑定序列，让窗口有机会落在某一次绑定行提交的前一刻；同时统计两件事：
    /// 台账行数 与 稿里镜像上的 part 数。
    #[test]
    fn binding_audio_while_the_seed_runs_never_loses_a_parts_media() {
        let mut ledger_losses: Vec<String> = Vec::new();
        let mut mirror_losses: Vec<String> = Vec::new();
        // 反空转：这条用例的价值全在「绑定链真的与播种交错了」上。若哪天改动让播种
        // 总是最后一个跑（或绑定总是先跑完），扫掠就会退化成一条什么都不测的绿用例——
        // 所以这里显式要求「至少有若干次，绑定线程的镜像真的往稿里写过东西」。
        let mut iterations_where_a_mirror_wrote = 0usize;

        for iteration in 0..30usize {
            let root = temp_root();
            let item_id = "item-race";
            seed_item_awaiting_seed(&root, item_id);

            // 绑定线程：严格照命令的次序——先落台账行，再把 media 镜像进稿。
            let binder_root = root.clone();
            let binder = std::thread::spawn(move || {
                let mut log: Vec<String> = Vec::new();
                for ordinal in 1..=4 {
                    let source = binder_root.join(format!("section-{ordinal}.wav"));
                    tone(&source, 300.0 + ordinal as f64 * 60.0);
                    let bound = crate::listening_audio::store::bind_audio(
                        &binder_root,
                        item_id,
                        ordinal,
                        &source,
                    )
                    .unwrap();
                    assert!(bound.playable, "part {ordinal}: {:?}", bound.issue_codes);
                    // 命令里紧接着就是这次镜像；它可能在「稿还不存在」时静默 no-op。
                    // 这里**不吞掉**结果：丢行时要能从失败信息里看出是哪一次、哪种结局。
                    match sync_item_audio_media(&binder_root, item_id) {
                        Ok(sync) => log.push(format!(
                            "part-{ordinal}:ok updated={:?} protected={:?}",
                            sync.updated_parts, sync.protected_parts
                        )),
                        Err(error) => log.push(format!("part-{ordinal}:ERR {error}")),
                    }
                }
                log
            });

            // 播种线程（主线程）：把启动时机逐次后移，扫掠过整个绑定序列。
            std::thread::sleep(std::time::Duration::from_micros(iteration as u64 * 1_200));
            let seeded = crate::library::migration::ensure_initial_canonical(&root, item_id)
                .unwrap_or_else(|error| panic!("迭代 {iteration}：播种不得失败：{error}"));
            let binder_log = binder.join().unwrap();
            if binder_log.iter().any(|entry| !entry.contains("updated=[]")) {
                iterations_where_a_mirror_wrote += 1;
            }

            let rows = crate::listening_audio::store::list_bindings(&root, item_id)
                .unwrap()
                .len();
            if rows != 4 {
                ledger_losses.push(format!("#{iteration} 台账只有 {rows} 行"));
            }
            if !seeded {
                ledger_losses.push(format!("#{iteration} 播种没有产生权威稿"));
                continue;
            }
            let ds = canonical(&root, item_id);
            let mirrored = mirrored_part_ids(&ds);
            if mirrored.len() != 4 {
                mirror_losses.push(format!(
                    "#{iteration} 稿里只镜像了 {mirrored:?}（台账 {rows} 行）绑定线程镜像日志 {binder_log:?}"
                ));
            }
            let _ = std::fs::remove_dir_all(&root);
        }

        assert!(
            ledger_losses.is_empty(),
            "台账（listening_audio_assets_v1）丢行：{ledger_losses:#?}"
        );
        assert!(
            mirror_losses.is_empty(),
            "台账有行、权威稿却少了 media 的 part：{mirror_losses:#?}"
        );
        assert!(
            iterations_where_a_mirror_wrote > 0,
            "扫掠空转：30 次里没有任何一次「绑定线程的镜像真的写了稿」，说明播种与绑定的交错已经不存在，这条用例失效了"
        );
    }

    /// 评审指定的反例必须在**命令处理器层**复现：导入听力卷后立即连续绑定 4 段音频，
    /// 同时让识别侧的写入并发进行，重复 30 次，统计 `listening_audio_assets_v1` 的行数。
    ///
    /// 上面那条扫掠直接拼 `bind_audio` + `sync_item_audio_media`；这条走
    /// `bind_listening_audio` 命令的**同一条写路径**（[`crate::listening_audio::commands::bind_audio_command_path`]：
    /// bind_audio → 镜像 → 读回确认），并把识别侧的写入做全：播种（`ensure_initial_canonical`）
    /// 之外，还有调度器在识别前后对同一 DB 的条目状态写。绑定线程的任何一段失败都
    /// **收集后整条判红**——正如界面对绑定失败的义务：不得吞掉、不得显示成功。
    #[test]
    fn the_bind_command_path_never_loses_a_row_while_recognition_writes_run() {
        let mut ledger_losses: Vec<String> = Vec::new();
        let mut bind_failures: Vec<String> = Vec::new();
        let mut mirror_losses: Vec<String> = Vec::new();
        // 反空转：绑定线程的镜像必须真的往稿里写过东西，扫掠才有效（同上条）。
        let mut iterations_where_a_mirror_wrote = 0usize;

        for iteration in 0..30usize {
            let root = temp_root();
            let item_id = "item-cmd-race";
            seed_item_awaiting_seed(&root, item_id);

            // 绑定线程：逐字跑命令的写路径，四段连续绑定。
            let binder_root = root.clone();
            let binder = std::thread::spawn(move || {
                let mut log: Vec<String> = Vec::new();
                for ordinal in 1..=4 {
                    let source = binder_root.join(format!("section-{ordinal}.wav"));
                    tone(&source, 300.0 + ordinal as f64 * 60.0);
                    match crate::listening_audio::commands::bind_audio_command_path(
                        &binder_root,
                        item_id,
                        ordinal,
                        &source,
                    ) {
                        Ok((_, sync)) => log.push(format!(
                            "part-{ordinal}:ok updated={:?} protected={:?}",
                            sync.updated_parts, sync.protected_parts
                        )),
                        Err(error) => log.push(format!("part-{ordinal}:ERR {error}")),
                    }
                }
                log
            });

            // 识别线程：播种 + 条目状态写，与调度器在识别完成前后的写入同构；
            // 启动时机逐次后移，扫掠过整个绑定序列。
            std::thread::sleep(std::time::Duration::from_micros(iteration as u64 * 1_200));
            let recognizer_root = root.clone();
            let recognizer = std::thread::spawn(move || {
                let mut notes = Vec::new();
                match crate::library::migration::ensure_initial_canonical(&recognizer_root, item_id)
                {
                    Ok(seeded) => notes.push(format!("seeded={seeded}")),
                    Err(error) => notes.push(format!("seed ERR {error}")),
                }
                if let Ok(conn) = open_library_connection(&recognizer_root) {
                    for status in ["ready_for_review", "processing", "ready_for_review"] {
                        if crate::library::repository::set_item_status(&conn, item_id, status)
                            .is_err()
                        {
                            notes.push(format!("status {status} ERR"));
                        }
                    }
                }
                notes
            });

            let binder_log = binder.join().unwrap();
            let recognizer_notes = recognizer.join().unwrap();
            if binder_log.iter().any(|entry| !entry.contains("updated=[]")) {
                iterations_where_a_mirror_wrote += 1;
            }
            for entry in &binder_log {
                if entry.contains(":ERR") {
                    bind_failures.push(format!("#{iteration} {entry}"));
                }
            }
            if recognizer_notes
                .iter()
                .any(|note| note.contains(" ERR") || note.contains("ERR "))
            {
                mirror_losses.push(format!("#{iteration} 识别侧写入失败 {recognizer_notes:?}"));
            }

            let rows = crate::listening_audio::store::list_bindings(&root, item_id)
                .unwrap()
                .len();
            if rows != 4 {
                ledger_losses.push(format!("#{iteration} 台账只有 {rows} 行"));
            }
            let seeded = recognizer_notes
                .iter()
                .any(|note| note.starts_with("seeded=true"));
            if seeded {
                let ds = canonical(&root, item_id);
                let mirrored = mirrored_part_ids(&ds);
                if mirrored.len() != 4 {
                    mirror_losses.push(format!(
                        "#{iteration} 稿里只镜像了 {mirrored:?}（台账 {rows} 行）绑定日志 {binder_log:?}"
                    ));
                }
            }
            let _ = std::fs::remove_dir_all(&root);
        }

        assert!(
            bind_failures.is_empty(),
            "绑定命令路径有失败被吞掉（界面不得显示成功）：{bind_failures:#?}"
        );
        assert!(
            ledger_losses.is_empty(),
            "命令层并发下台账（listening_audio_assets_v1）丢行：{ledger_losses:#?}"
        );
        assert!(
            mirror_losses.is_empty(),
            "台账有行、权威稿却少了 media 的 part（或识别侧写入失败）：{mirror_losses:#?}"
        );
        assert!(
            iterations_where_a_mirror_wrote > 0,
            "扫掠空转：30 次里没有任何一次「绑定线程的镜像真的写了稿」，说明播种与绑定的交错已经不存在，这条用例失效了"
        );
    }

    /// 播种之后必须**对账**：台账里已绑的 part，稿里就得有 media。
    ///
    /// 上面那条是并发扫掠（概率命中）；这条是把同一个契约**确定性地**钉死：
    /// 只要稿是「播种时才出现」的，播种这一步就必须自己把此前落地的绑定补齐——
    /// 那些绑定的镜像当时看到「稿不存在」而 no-op，之后不会有人再替它们跑一次。
    #[test]
    fn seeding_after_the_bindings_mirrors_every_bound_part() {
        let root = temp_root();
        let item_id = "item-seed-after";
        seed_item_awaiting_seed(&root, item_id);

        // 稿还不存在：这四次镜像全都只能 no-op——播种必须替它们收尾。
        for ordinal in 1..=4 {
            let source = root.join(format!("section-{ordinal}.wav"));
            tone(&source, 300.0 + ordinal as f64 * 60.0);
            crate::listening_audio::store::bind_audio(&root, item_id, ordinal, &source).unwrap();
            let sync = sync_item_audio_media(&root, item_id).unwrap();
            assert!(
                sync.updated_parts.is_empty(),
                "稿还不存在时镜像无处可写，只能空转：{sync:?}"
            );
        }

        assert!(
            crate::library::migration::ensure_initial_canonical(&root, item_id).unwrap(),
            "播种必须产出权威稿"
        );

        let ds = canonical(&root, item_id);
        assert_eq!(
            mirrored_part_ids(&ds),
            vec!["part-1", "part-2", "part-3", "part-4"],
            "播种之后，台账里已绑的 4 个 part 在稿里都必须有 media：{ds}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 台账写成功 ≠ 绑定成功：镜像没落盘时，绑定命令必须**读回发现**，而不是返回 Ok。
    ///
    /// 这里用**人工保护**制造「镜像被刻意拒绝」这一真实分支（用户在编辑器里手工改过这个
    /// part 的 media）。此前 `bind_listening_audio` 把 `AudioMediaSyncV1` 整个丢掉，
    /// 于是界面只会说「已添加」，而稿里那一段音频根本读不到。
    #[test]
    fn a_refused_mirror_is_reported_instead_of_looking_like_a_successful_bind() {
        let root = temp_root();
        seed_listening_item(&root, "item-refused");
        {
            let conn = open_library_connection(&root).unwrap();
            conn.execute(
                "UPDATE library_items_v2 SET protected_edits_json = ?2 WHERE id = ?1",
                rusqlite::params!["item-refused", r#"{"targets":["part-1"]}"#],
            )
            .unwrap();
        }
        let source = root.join("section-1.wav");
        tone(&source, 440.0);
        let refused = bind_audio(&root, "item-refused", 1, &source).unwrap();
        let sync = sync_item_audio_media(&root, "item-refused").unwrap();
        assert_eq!(sync.protected_parts, vec!["part-1".to_string()]);
        let error = ensure_part_media_matches(&root, "item-refused", 1, &refused).unwrap_err();
        assert!(
            error.starts_with("LISTENING_AUDIO_MEDIA_NOT_MIRRORED:part-1"),
            "{error}"
        );

        // 反向：没人保护的那个 part 镜像得上，读回必须是 Ok——否则这条判据会把正常绑定也判红。
        let other = root.join("section-2.wav");
        tone(&other, 660.0);
        let mirrored = bind_audio(&root, "item-refused", 2, &other).unwrap();
        sync_item_audio_media(&root, "item-refused").unwrap();
        ensure_part_media_matches(&root, "item-refused", 2, &mirrored).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        // 稿还不存在时**不能**报错：识别还没产出首稿，播种（含播种后的对账）负责补镜像。
        let no_draft = temp_root();
        seed_item_awaiting_seed(&no_draft, "item-nodraft");
        let source = no_draft.join("section-1.wav");
        tone(&source, 440.0);
        let bound =
            crate::listening_audio::store::bind_audio(&no_draft, "item-nodraft", 1, &source)
                .unwrap();
        ensure_part_media_matches(&no_draft, "item-nodraft", 1, &bound).unwrap();
        let _ = std::fs::remove_dir_all(&no_draft);
    }
}
