//! Managed listening audio: copy, hash, probe and bind audio files to a library item.
//!
//! Audio is copied to `<appData>/audio/<itemId>/<sha256>.<ext>`, deliberately **outside**
//! the job directory: job directories hold process artifacts and are purged after publish,
//! while audio is part of the editable final version. After binding, the user's original
//! file is never read again.
//!
//! One row per `(item_id, part_ordinal)`: each listening part owns one audio file. The IR
//! contract does not yet carry per-part media, so this table is the binding authority until
//! the IR integration lands.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::library::repository::open_library_connection;
use crate::schema::listening_audio_probe_v1::{
    probe_listening_audio_v1, ListeningAudioProbePolicyV1, ListeningAudioProbeResultV1,
};
use crate::CommandResult;

pub(crate) const LISTENING_AUDIO_ASSETS_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS listening_audio_assets_v1 (
    item_id       TEXT NOT NULL REFERENCES library_items_v2(id),
    part_ordinal  INTEGER NOT NULL CHECK (part_ordinal >= 1),
    managed_path  TEXT NOT NULL,
    sha256        TEXT NOT NULL,
    size_bytes    INTEGER NOT NULL,
    mime          TEXT,
    duration_ms   INTEGER,
    probe_json    TEXT NOT NULL,
    original_name TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    PRIMARY KEY (item_id, part_ordinal)
);
"#;

/// Highest part ordinal accepted. IELTS Listening has four parts; a little headroom lets a
/// user bind an extra track (e.g. an example) without the backend guessing intent.
pub(crate) const MAX_PART_ORDINAL: i64 = 8;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListeningAudioAssetV1 {
    pub item_id: String,
    pub part_ordinal: i64,
    pub managed_path: String,
    pub sha256: String,
    pub size_bytes: i64,
    pub mime: Option<String>,
    pub duration_ms: Option<i64>,
    pub probe: Value,
    pub original_name: String,
    pub created_at: String,
    /// Probe passed with no issue codes.
    pub playable: bool,
    /// `AUDIO_DECODE_FAILED` etc., copied from the probe for direct UI use.
    pub issue_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListeningAudioStatusV1 {
    pub item_id: String,
    pub bindings: Vec<ListeningAudioAssetV1>,
    /// True only when at least one part is bound and every bound part probed clean.
    pub audio_ready: bool,
    /// Stable reason codes when not ready: `AUDIO_MISSING`, `AUDIO_PROBE_BLOCKED:<part>`.
    pub blockers: Vec<String>,
}

pub(crate) fn audio_root(root: &Path) -> PathBuf {
    root.join("audio")
}

pub(crate) fn item_audio_dir(root: &Path, item_id: &str) -> CommandResult<PathBuf> {
    crate::util::validate_path_segment("item_id", item_id)?;
    Ok(audio_root(root).join(item_id))
}

/// 受管音频文件的落盘路径：`<appData>/audio/<itemId>/<sha256>.<ext>`。
///
/// 导出与打包必须靠它取用户上传的 Section 音频。文件**不在** job 目录里（这是刻意的：
/// job 目录装的是过程产物、发布后会被清理，而音频是最终版的一部分），所以「按资源描述符
/// 的相对路径从 job 目录找」永远找不到它——那正是「导出一份带音频的听力卷」曾经失败的
/// 原因。
///
/// 扩展名来自用户上传的原始文件名，无法从 sha 推出来，所以按文件名前缀匹配。
pub(crate) fn managed_audio_path(
    root: &Path,
    item_id: &str,
    sha256: &str,
) -> CommandResult<Option<PathBuf>> {
    let dir = item_audio_dir(root, item_id)?;
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(None);
    };
    let needle = sha256.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Ok(None);
    }
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let stem = name.split('.').next().unwrap_or_default();
        if stem.eq_ignore_ascii_case(&needle) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn audio_extension(name: &str) -> String {
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let clean: String = extension.chars().filter(|ch| ch.is_ascii_alphanumeric()).take(8).collect();
    if clean.is_empty() { "bin".to_string() } else { clean }
}

fn issue_codes(probe: &ListeningAudioProbeResultV1) -> Vec<String> {
    serde_json::to_value(&probe.probe.issue_codes)
        .ok()
        .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn row_to_asset(row: &rusqlite::Row<'_>) -> rusqlite::Result<ListeningAudioAssetV1> {
    let probe_json: String = row.get("probe_json")?;
    let probe: Value = serde_json::from_str(&probe_json).unwrap_or(Value::Null);
    let parsed: Option<ListeningAudioProbeResultV1> = serde_json::from_value(probe.clone()).ok();
    let (playable, issue_codes) = match &parsed {
        Some(result) => (result.is_passed(), issue_codes(result)),
        None => (false, vec!["AUDIO_PROBE_UNREADABLE".to_string()]),
    };
    Ok(ListeningAudioAssetV1 {
        item_id: row.get("item_id")?,
        part_ordinal: row.get("part_ordinal")?,
        managed_path: row.get("managed_path")?,
        sha256: row.get("sha256")?,
        size_bytes: row.get("size_bytes")?,
        mime: row.get("mime")?,
        duration_ms: row.get("duration_ms")?,
        probe,
        original_name: row.get("original_name")?,
        created_at: row.get("created_at")?,
        playable,
        issue_codes,
    })
}

fn get_binding(conn: &Connection, item_id: &str, part_ordinal: i64) -> CommandResult<Option<ListeningAudioAssetV1>> {
    conn.query_row(
        "SELECT * FROM listening_audio_assets_v1 WHERE item_id = ?1 AND part_ordinal = ?2",
        params![item_id, part_ordinal],
        row_to_asset,
    )
    .optional()
    .map_err(|error| format!("listening_audio_get:{error}"))
}

pub(crate) fn list_bindings(root: &Path, item_id: &str) -> CommandResult<Vec<ListeningAudioAssetV1>> {
    let conn = open_library_connection(root)?;
    list_bindings_conn(&conn, item_id)
}

fn list_bindings_conn(conn: &Connection, item_id: &str) -> CommandResult<Vec<ListeningAudioAssetV1>> {
    let mut statement = conn
        .prepare("SELECT * FROM listening_audio_assets_v1 WHERE item_id = ?1 ORDER BY part_ordinal")
        .map_err(|error| format!("listening_audio_list:{error}"))?;
    let rows = statement
        .query_map([item_id], row_to_asset)
        .map_err(|error| format!("listening_audio_list:{error}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| format!("listening_audio_list:{error}"))
}

pub(crate) fn audio_status(root: &Path, item_id: &str) -> CommandResult<ListeningAudioStatusV1> {
    let bindings = list_bindings(root, item_id)?;
    let mut blockers = Vec::new();
    if bindings.is_empty() {
        blockers.push("AUDIO_MISSING".to_string());
    }
    for binding in &bindings {
        if !binding.playable {
            blockers.push(format!("AUDIO_PROBE_BLOCKED:{}", binding.part_ordinal));
        }
    }
    Ok(ListeningAudioStatusV1 {
        item_id: item_id.to_string(),
        audio_ready: blockers.is_empty(),
        bindings,
        blockers,
    })
}

/// Deletes a managed file unless another binding still references it (two parts may
/// legitimately bind byte-identical audio, which share one `<sha256>.<ext>` file).
fn remove_if_unreferenced(conn: &Connection, root: &Path, managed_path: &str) {
    let still_used: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM listening_audio_assets_v1 WHERE managed_path = ?1",
            [managed_path],
            |row| row.get(0),
        )
        .unwrap_or(1);
    if still_used > 0 {
        return;
    }
    let path = PathBuf::from(managed_path);
    // Never delete anything outside the managed audio root.
    if path.starts_with(audio_root(root)) {
        if let Err(error) = fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("[listening_audio] remove {managed_path} failed: {error}");
            }
        }
    }
}

/// Copies `source_path` into managed storage, probes the managed copy and binds it to
/// `(item_id, part_ordinal)`, replacing any previous binding for that part. A failed probe
/// is still stored (blocked, with issue codes) so the UI can show what is wrong and offer
/// "replace audio"; it never counts as audio-ready.
pub(crate) fn bind_audio(
    root: &Path,
    item_id: &str,
    part_ordinal: i64,
    source_path: &Path,
) -> CommandResult<ListeningAudioAssetV1> {
    if !(1..=MAX_PART_ORDINAL).contains(&part_ordinal) {
        return Err(format!("listening_audio_part_out_of_range:{part_ordinal}"));
    }
    let dir = item_audio_dir(root, item_id)?;
    {
        let conn = open_library_connection(root)?;
        let exists: Option<String> = conn
            .query_row("SELECT id FROM library_items_v2 WHERE id = ?1", [item_id], |row| row.get(0))
            .optional()
            .map_err(|error| format!("listening_audio_item:{error}"))?;
        if exists.is_none() {
            return Err(format!("ITEM_NOT_FOUND:{item_id}"));
        }
    }
    let original_name = source_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("audio")
        .to_string();
    let extension = audio_extension(&original_name);
    fs::create_dir_all(&dir).map_err(|error| format!("listening_audio_dir:{error}"))?;
    let staging = dir.join(format!(".staging-{}.{extension}", Uuid::new_v4().simple()));
    let (sha256, size) = crate::util::stage_file_with_hash(source_path, &staging)?;
    let final_path = dir.join(format!("{sha256}.{extension}"));
    if final_path.exists() {
        let _ = fs::remove_file(&staging);
    } else if let Err(error) = fs::rename(&staging, &final_path) {
        let _ = fs::remove_file(&staging);
        return Err(format!("listening_audio_store:{}:{error}", final_path.display()));
    }

    // Probe the managed copy (never the user's original) and pin it to the hash we copied.
    let probe = probe_listening_audio_v1(&final_path, Some(&sha256), &ListeningAudioProbePolicyV1::default());
    let probe_json = serde_json::to_string(&probe).map_err(|error| error.to_string())?;
    let managed_path = final_path.to_string_lossy().to_string();

    let mut conn = open_library_connection(root)?;
    let transaction = conn.transaction().map_err(|error| format!("listening_audio_tx:{error}"))?;
    let previous = get_binding(&transaction, item_id, part_ordinal)?;
    transaction
        .execute(
            "INSERT INTO listening_audio_assets_v1
             (item_id, part_ordinal, managed_path, sha256, size_bytes, mime, duration_ms, probe_json, original_name, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(item_id, part_ordinal) DO UPDATE SET
               managed_path = excluded.managed_path, sha256 = excluded.sha256,
               size_bytes = excluded.size_bytes, mime = excluded.mime,
               duration_ms = excluded.duration_ms, probe_json = excluded.probe_json,
               original_name = excluded.original_name, created_at = excluded.created_at",
            params![
                item_id,
                part_ordinal,
                managed_path,
                sha256,
                size as i64,
                probe.mime,
                probe.duration_ms.map(|value| value as i64),
                probe_json,
                original_name,
                Utc::now().to_rfc3339(),
            ],
        )
        .map_err(|error| format!("listening_audio_bind:{error}"))?;
    transaction.commit().map_err(|error| format!("listening_audio_commit:{error}"))?;
    if let Some(previous) = previous {
        if previous.managed_path != managed_path {
            remove_if_unreferenced(&conn, root, &previous.managed_path);
        }
    }
    get_binding(&conn, item_id, part_ordinal)?.ok_or_else(|| "listening_audio_bind_lost".to_string())
}

pub(crate) fn unbind_audio(root: &Path, item_id: &str, part_ordinal: i64) -> CommandResult<bool> {
    let conn = open_library_connection(root)?;
    let Some(previous) = get_binding(&conn, item_id, part_ordinal)? else {
        return Ok(false);
    };
    conn.execute(
        "DELETE FROM listening_audio_assets_v1 WHERE item_id = ?1 AND part_ordinal = ?2",
        params![item_id, part_ordinal],
    )
    .map_err(|error| format!("listening_audio_unbind:{error}"))?;
    remove_if_unreferenced(&conn, root, &previous.managed_path);
    Ok(true)
}

/// 永久删除一个条目时，连它自己的受管音频一起清掉。
///
/// 音频刻意不在 job 目录里（job 目录装过程产物、发布后会被清理；音频是最终版的一部分），
/// 所以「删掉 job 目录」永远带不走它。不显式清理就是**永久泄漏**：文件躺在磁盘上、
/// 表里留着行，用户既看不到也删不掉。
///
/// 边界只有一个条目，两层都按它收窄：
/// - 表：事务内只删 `item_id = ?` 的行；
/// - 文件：只删 `<appData>/audio/<itemId>/` 这一层目录，且先确认它**确实**落在音频根之下、
///   是一个**真实目录**（不是指向别处的联接）——否则一个被换掉的目录会让
///   「删除这一个条目」删掉别的条目。
///
/// 不返回 `Err` 除非 `item_id` 本身不安全：用户要求的是删除，清理音频失败不该让删除失败，
/// 那些失败如实进 `issues`。
pub(crate) fn purge_item_audio(root: &Path, item_id: &str) -> CommandResult<Value> {
    let dir = item_audio_dir(root, item_id)?;
    let mut issues: Vec<String> = Vec::new();

    let mut conn = open_library_connection(root)?;
    let rows_removed = {
        let transaction = conn
            .transaction()
            .map_err(|error| format!("listening_audio_purge_tx:{error}"))?;
        let affected = transaction
            .execute(
                "DELETE FROM listening_audio_assets_v1 WHERE item_id = ?1",
                params![item_id],
            )
            .map_err(|error| format!("listening_audio_purge_rows:{error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("listening_audio_purge_commit:{error}"))?;
        affected
    };
    drop(conn);

    let mut directory_removed = false;
    match fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            // 一个指向别处的联接：删它可能连带删掉别人。只摘掉链接本身，绝不下钻。
            match fs::remove_file(&dir) {
                Ok(()) => directory_removed = true,
                Err(error) => issues.push(format!("listening_audio_purge_link:{}:{error}", dir.display())),
            }
        }
        Ok(metadata) if metadata.is_dir() => {
            if !dir.starts_with(audio_root(root)) {
                issues.push(format!("listening_audio_purge_outside_root:{}", dir.display()));
            } else {
                match fs::remove_dir_all(&dir) {
                    Ok(()) => directory_removed = true,
                    Err(error) => {
                        issues.push(format!("listening_audio_purge_dir:{}:{error}", dir.display()))
                    }
                }
            }
        }
        // 没有目录：没有音频，空操作。
        Ok(_) => issues.push(format!("listening_audio_purge_not_a_directory:{}", dir.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => issues.push(format!("listening_audio_purge_stat:{}:{error}", dir.display())),
    }

    Ok(serde_json::json!({
        "itemId": item_id,
        "rowsRemoved": rows_removed,
        "directoryRemoved": directory_removed,
        "issues": issues,
    }))
}

/// Re-probes every managed file against its recorded hash and stores the fresh result.
/// Used when a workspace opens so a managed file that went missing or was altered shows up
/// as blocked instead of silently failing at playback.
pub(crate) fn verify_bindings(root: &Path, item_id: &str) -> CommandResult<ListeningAudioStatusV1> {
    let conn = open_library_connection(root)?;
    for binding in list_bindings_conn(&conn, item_id)? {
        let probe = probe_listening_audio_v1(
            Path::new(&binding.managed_path),
            Some(&binding.sha256),
            &ListeningAudioProbePolicyV1::default(),
        );
        let probe_json = serde_json::to_string(&probe).map_err(|error| error.to_string())?;
        conn.execute(
            "UPDATE listening_audio_assets_v1 SET probe_json = ?3 WHERE item_id = ?1 AND part_ordinal = ?2",
            params![item_id, binding.part_ordinal, probe_json],
        )
        .map_err(|error| format!("listening_audio_verify:{error}"))?;
    }
    drop(conn);
    audio_status(root, item_id)
}

/// Splits a name into text and number runs so `Part 10` sorts after `Part 2`.
fn natural_key(name: &str) -> Vec<(u8, String, u128)> {
    let mut key = Vec::new();
    let mut text = String::new();
    let mut digits = String::new();
    let flush_text = |text: &mut String, key: &mut Vec<(u8, String, u128)>| {
        if !text.is_empty() {
            key.push((1, std::mem::take(text), 0));
        }
    };
    for ch in name.chars() {
        if ch.is_ascii_digit() {
            flush_text(&mut text, &mut key);
            digits.push(ch);
        } else {
            if !digits.is_empty() {
                key.push((0, String::new(), digits.parse().unwrap_or(u128::MAX)));
                digits.clear();
            }
            text.extend(ch.to_lowercase());
        }
    }
    flush_text(&mut text, &mut key);
    if !digits.is_empty() {
        key.push((0, String::new(), digits.parse().unwrap_or(u128::MAX)));
    }
    key
}

pub(crate) fn natural_sort_names(names: &mut [String]) {
    names.sort_by(|left, right| natural_key(left).cmp(&natural_key(right)).then_with(|| left.cmp(right)));
}

/// `.mp3` files directly inside `folder`, naturally sorted by file name. Ordinal `n` is the
/// n-th entry; the dialog lets the user reorder before binding.
pub(crate) fn list_folder_mp3(folder: &Path) -> CommandResult<Vec<PathBuf>> {
    let entries = fs::read_dir(folder).map_err(|error| format!("listening_audio_folder:{error}"))?;
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| !name.starts_with('.') && audio_extension(name) == "mp3")
        .collect();
    natural_sort_names(&mut names);
    Ok(names.into_iter().map(|name| folder.join(name)).collect())
}

/// Binds every `.mp3` in `folder` to parts `1..n` in natural name order.
pub(crate) fn bind_folder(root: &Path, item_id: &str, folder: &Path) -> CommandResult<Vec<ListeningAudioAssetV1>> {
    let files = list_folder_mp3(folder)?;
    if files.is_empty() {
        return Err("listening_audio_folder_empty".to_string());
    }
    if files.len() as i64 > MAX_PART_ORDINAL {
        return Err(format!("listening_audio_folder_too_many:{}", files.len()));
    }
    files
        .iter()
        .enumerate()
        .map(|(index, path)| bind_audio(root, item_id, index as i64 + 1, path))
        .collect()
}

/// Probe files before an item exists (import dialog preview). Nothing is copied.
pub(crate) fn probe_files(paths: &[String]) -> Vec<ListeningAudioProbeResultV1> {
    paths
        .iter()
        .map(|path| probe_listening_audio_v1(Path::new(path), None, &ListeningAudioProbePolicyV1::default()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::repository::{upsert_item_shell, UpsertItemInput};
    use std::io::Write;

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("listening-audio-{}", Uuid::new_v4().simple()));
        crate::util::ensure_app_dirs(&root).unwrap();
        root
    }

    fn seed_item(root: &Path, item_id: &str) {
        let conn = open_library_connection(root).unwrap();
        upsert_item_shell(
            &conn,
            &UpsertItemInput { id: item_id, modality: "listening", title: "L", status: "processing", source_asset_id: None },
        )
        .unwrap();
        // A job directory exists for real imports; audio must never land inside it.
        fs::create_dir_all(crate::util::job_dir(root, item_id)).unwrap();
    }

    fn write_wav(path: &Path, samples: &[i16]) {
        let rate = 16_000_u32;
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
        fs::File::create(path).unwrap().write_all(&bytes).unwrap();
    }

    fn tone(path: &Path, hz: f64) {
        let samples = (0..16_000)
            .map(|index| ((index as f64 / 16_000.0) * hz * std::f64::consts::TAU).sin() * 8_000.0)
            .map(|value| value as i16)
            .collect::<Vec<_>>();
        write_wav(path, &samples);
    }

    #[test]
    fn bound_audio_survives_deleting_the_original_and_lives_outside_the_job_dir() {
        let root = temp_root();
        seed_item(&root, "item-l");
        let outside = std::env::temp_dir().join(format!("user-audio-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&outside).unwrap();
        let original = outside.join("Section 1.wav");
        tone(&original, 440.0);

        let bound = bind_audio(&root, "item-l", 1, &original).unwrap();
        assert!(bound.playable, "{:?}", bound.issue_codes);
        assert_eq!(bound.duration_ms, Some(1000));
        assert_eq!(bound.mime.as_deref(), Some("audio/wav"));
        assert_eq!(bound.original_name, "Section 1.wav");
        assert_eq!(bound.sha256.len(), 64);
        let managed = PathBuf::from(&bound.managed_path);
        assert_eq!(managed, root.join("audio").join("item-l").join(format!("{}.wav", bound.sha256)));
        assert!(!managed.starts_with(root.join("jobs")), "audio must not live inside a job directory");

        fs::remove_dir_all(&outside).unwrap();
        let status = verify_bindings(&root, "item-l").unwrap();
        assert!(status.audio_ready, "{:?}", status.blockers);
        assert_eq!(status.bindings.len(), 1);
        assert!(status.bindings[0].playable);
        // Purging the job directory (post-publish cleanup) leaves the audio intact.
        fs::remove_dir_all(crate::util::job_dir(&root, "item-l")).unwrap();
        assert!(verify_bindings(&root, "item-l").unwrap().audio_ready);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_silent_and_unsupported_audio_is_stored_blocked_and_not_ready() {
        let root = temp_root();
        seed_item(&root, "item-bad");
        let dir = root.join("user");
        fs::create_dir_all(&dir).unwrap();
        let corrupt = dir.join("broken.mp3");
        fs::write(&corrupt, b"not audio at all").unwrap();
        let silent = dir.join("silent.wav");
        write_wav(&silent, &vec![0; 16_000]);
        let unsupported = dir.join("notes.ogg");
        write_wav(&unsupported, &vec![1_000; 1600]);

        let cases = [(1, &corrupt, "AUDIO_DECODE_FAILED"), (2, &silent, "AUDIO_NEAR_SILENT"), (3, &unsupported, "AUDIO_CODEC_UNSUPPORTED")];
        for (part, path, code) in cases {
            let bound = bind_audio(&root, "item-bad", part, path).unwrap();
            assert!(!bound.playable, "part {part} must be blocked");
            assert_eq!(bound.probe["probe"]["status"], "blocked");
            assert!(bound.issue_codes.iter().any(|issue| issue == code), "part {part}: {:?}", bound.issue_codes);
        }
        let status = audio_status(&root, "item-bad").unwrap();
        assert!(!status.audio_ready);
        assert_eq!(status.bindings.len(), 3);
        assert_eq!(
            status.blockers,
            vec!["AUDIO_PROBE_BLOCKED:1", "AUDIO_PROBE_BLOCKED:2", "AUDIO_PROBE_BLOCKED:3"]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_item_without_audio_is_not_audio_ready() {
        let root = temp_root();
        seed_item(&root, "item-empty");
        let status = audio_status(&root, "item-empty").unwrap();
        assert!(!status.audio_ready);
        assert_eq!(status.blockers, vec!["AUDIO_MISSING"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rebinding_a_part_replaces_the_managed_file_and_unbind_removes_it() {
        let root = temp_root();
        seed_item(&root, "item-r");
        let dir = root.join("user");
        fs::create_dir_all(&dir).unwrap();
        let first = dir.join("a.wav");
        let second = dir.join("b.wav");
        tone(&first, 440.0);
        tone(&second, 660.0);

        let old = bind_audio(&root, "item-r", 2, &first).unwrap();
        let new = bind_audio(&root, "item-r", 2, &second).unwrap();
        assert_ne!(old.sha256, new.sha256);
        assert!(!Path::new(&old.managed_path).exists(), "the replaced managed file must be removed");
        assert!(Path::new(&new.managed_path).exists());
        let bindings = list_bindings(&root, "item-r").unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].original_name, "b.wav");

        assert!(unbind_audio(&root, "item-r", 2).unwrap());
        assert!(!Path::new(&new.managed_path).exists());
        assert!(list_bindings(&root, "item-r").unwrap().is_empty());
        assert!(!unbind_audio(&root, "item-r", 2).unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn shared_audio_file_is_kept_while_another_part_uses_it() {
        let root = temp_root();
        seed_item(&root, "item-s");
        let source = root.join("same.wav");
        tone(&source, 440.0);
        let one = bind_audio(&root, "item-s", 1, &source).unwrap();
        let two = bind_audio(&root, "item-s", 2, &source).unwrap();
        assert_eq!(one.managed_path, two.managed_path);
        assert!(unbind_audio(&root, "item-s", 1).unwrap());
        assert!(Path::new(&two.managed_path).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn binding_requires_an_existing_item_and_a_valid_part() {
        let root = temp_root();
        let source = root.join("x.wav");
        tone(&source, 440.0);
        assert!(bind_audio(&root, "missing", 1, &source).unwrap_err().starts_with("ITEM_NOT_FOUND"));
        seed_item(&root, "item-p");
        assert!(bind_audio(&root, "item-p", 0, &source).is_err());
        assert!(bind_audio(&root, "item-p", MAX_PART_ORDINAL + 1, &source).is_err());
        assert!(bind_audio(&root, "../escape", 1, &source).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn folder_binding_naturally_sorts_mp3_files_into_parts() {
        let root = temp_root();
        seed_item(&root, "item-f");
        let folder = root.join("cd");
        fs::create_dir_all(&folder).unwrap();
        for name in ["Part 10.mp3", "Part 2.mp3", "part 1.MP3", "cover.jpg", "Part 3.wav"] {
            fs::write(folder.join(name), b"x").unwrap();
        }
        let listed: Vec<String> = list_folder_mp3(&folder)
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(listed, vec!["part 1.MP3", "Part 2.mp3", "Part 10.mp3"]);

        let bound = bind_folder(&root, "item-f", &folder).unwrap();
        assert_eq!(bound.iter().map(|b| b.part_ordinal).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(bound[2].original_name, "Part 10.mp3");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn migration_creates_the_audio_table() {
        let conn = Connection::open_in_memory().unwrap();
        crate::library::schema::ensure_v2_schema(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'listening_audio_assets_v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    /// 永久删除一个条目时必须连它自己的受管音频一起清掉，且**只**清它自己的。
    ///
    /// 音频刻意不在 job 目录里（job 目录装过程产物、发布后会被清理；音频是最终版的一部分），
    /// 所以「删掉 job 目录」永远带不走它。不显式清理就是永久泄漏：文件躺在磁盘上、
    /// 表里留着行，用户既看不到也删不掉——一个被删掉的条目会永久占着磁盘。
    #[test]
    fn permanent_delete_purges_only_this_items_audio() {
        let root = temp_root();
        seed_item(&root, "item-keep");
        seed_item(&root, "item-gone");
        let outside = std::env::temp_dir().join(format!("user-audio-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&outside).unwrap();
        let kept = outside.join("keep.wav");
        tone(&kept, 440.0);
        let doomed = outside.join("doomed.wav");
        tone(&doomed, 880.0);
        bind_audio(&root, "item-keep", 1, &kept).unwrap();
        // 同一个文件被两个 part 共用：两行，一份文件。
        bind_audio(&root, "item-gone", 1, &doomed).unwrap();
        bind_audio(&root, "item-gone", 2, &doomed).unwrap();

        let gone_dir = root.join("audio").join("item-gone");
        let keep_dir = root.join("audio").join("item-keep");
        assert!(gone_dir.is_dir() && keep_dir.is_dir(), "夹具必须真的落了两份受管音频");

        let report = purge_item_audio(&root, "item-gone").unwrap();
        assert_eq!(report["rowsRemoved"], serde_json::json!(2), "{report}");
        assert_eq!(report["directoryRemoved"], serde_json::json!(true), "{report}");
        assert!(report["issues"].as_array().is_some_and(Vec::is_empty), "{report}");
        assert!(!gone_dir.exists(), "被永久删除的条目不该留下音频目录");

        // 另一个条目一个字都不许动：目录、文件、表行、可用性全部照旧。
        assert!(keep_dir.is_dir(), "永久删除一个条目不能碰别的条目");
        let status = audio_status(&root, "item-keep").unwrap();
        assert!(status.audio_ready, "{:?}", status.blockers);
        assert_eq!(status.bindings.len(), 1);
        let conn = open_library_connection(&root).unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM listening_audio_assets_v1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 1, "表里只该剩下另一个条目的那一行");
        drop(conn);

        // 幂等：重复删除不报错，也不会顺手带走别的条目。
        let again = purge_item_audio(&root, "item-gone").unwrap();
        assert_eq!(again["rowsRemoved"], serde_json::json!(0), "{again}");
        assert_eq!(again["directoryRemoved"], serde_json::json!(false), "{again}");
        assert!(keep_dir.is_dir());

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    /// 没有音频的条目：清理是空操作，不是错误；不安全的 id 直接拒绝。
    #[test]
    fn purging_an_item_with_no_audio_is_a_no_op_and_rejects_unsafe_ids() {
        let root = temp_root();
        seed_item(&root, "item-none");
        let report = purge_item_audio(&root, "item-none").unwrap();
        assert_eq!(report["rowsRemoved"], serde_json::json!(0), "{report}");
        assert_eq!(report["directoryRemoved"], serde_json::json!(false), "{report}");
        assert!(
            purge_item_audio(&root, "../escape").is_err(),
            "路径穿越必须被拒绝，而不是删掉音频根之外的东西"
        );
        assert!(
            !root.join("audio").join("item-none").exists(),
            "空操作不该凭空造出一个音频目录"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
