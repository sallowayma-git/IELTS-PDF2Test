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
    media.insert("assetId".to_string(), json!(audio_asset_id(&binding.sha256)));
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
    asset.insert("assetId".to_string(), json!(audio_asset_id(&binding.sha256)));
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
pub(crate) fn sync_item_audio_media(root: &Path, item_id: &str) -> CommandResult<AudioMediaSyncV1> {
    let bindings = list_bindings(root, item_id)?;
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
    let changed = changed_parts(&canonical, &bindings);
    // A part whose media a human edited is dropped from the batch *before* it is
    // built. Leaving it in would make the all-or-nothing transaction reject the
    // whole batch, so one hand-edited part would freeze every other part's audio.
    let protected = crate::library::repository::human_protected_targets(&conn, item_id, &canonical)?;
    let (protected_parts, changed): (Vec<String>, Vec<String>) =
        changed.into_iter().partition(|part_id| protected.contains(part_id));
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
        assert!(!assets_need_update(&value, &[binding(1, &"a".repeat(64), true)]));
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
        assert!(error.starts_with("AUTHORING_PATCH_PART_NOT_FOUND"), "{error}");
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
        std::fs::File::create(path).unwrap().write_all(&bytes).unwrap();
    }

    fn canonical(root: &Path, item_id: &str) -> Value {
        let conn = open_library_connection(root).unwrap();
        get_canonical_ds(&conn, item_id).unwrap().unwrap().0
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
        assert!(changed_parts(&ds, &crate::listening_audio::store::list_bindings(&root, "item-stale").unwrap()).len() == 1);
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
        assert!(structure.media.is_none(), "audio is per part, not exam-level");
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
}
