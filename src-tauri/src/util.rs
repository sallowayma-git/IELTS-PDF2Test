use crate::CommandResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(crate) const MAX_SOURCE_FILE_BYTES: u64 = 128 * 1024 * 1024;
const FILE_IO_BUFFER_BYTES: usize = 64 * 1024;

pub(crate) fn is_safe_path_segment(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

pub(crate) fn validate_path_segment(kind: &str, value: &str) -> CommandResult<()> {
    if is_safe_path_segment(value) {
        Ok(())
    } else {
        Err(format!("invalid_{}_path_segment:{}", kind, value))
    }
}

pub(crate) fn job_dir(root: &Path, job_id: &str) -> PathBuf {
    // Defense-in-depth: callers should validate first, but this prevents a missed
    // validation from turning an external id into path traversal.
    let segment = if is_safe_path_segment(job_id) {
        job_id
    } else {
        "__invalid_job_id__"
    };
    root.join("jobs").join(segment)
}

pub(crate) fn safe_job_dir(root: &Path, job_id: &str) -> CommandResult<PathBuf> {
    validate_path_segment("job_id", job_id)?;
    Ok(job_dir(root, job_id))
}

pub(crate) fn writing_job_dir(root: &Path, job_id: &str) -> PathBuf {
    let segment = if is_safe_path_segment(job_id) {
        job_id
    } else {
        "__invalid_writing_job_id__"
    };
    root.join("writing-jobs").join(segment)
}

pub(crate) fn safe_writing_job_dir(root: &Path, job_id: &str) -> CommandResult<PathBuf> {
    validate_path_segment("writing_job_id", job_id)?;
    Ok(writing_job_dir(root, job_id))
}

pub(crate) fn ensure_app_dirs(root: &Path) -> CommandResult<()> {
    for relative in [
        "config",
        "config/secrets",
        "jobs",
        "writing-jobs",
        "logs",
        "cache",
        "cache/parser",
        "cache/thumbnails",
        "cache/preview-server",
    ] {
        fs::create_dir_all(root.join(relative)).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) fn file_type_from_name(name: &str) -> &'static str {
    match Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "pdf" => "pdf",
        "docx" => "docx",
        "txt" => "txt",
        "md" => "md",
        "png" | "jpg" | "jpeg" | "webp" => "image",
        _ => "unknown",
    }
}

pub(crate) fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn source_file_too_large_error(path: &Path, size_bytes: u64, max_bytes: u64) -> String {
    format!(
        "source_file_too_large:max_bytes={max_bytes}:size_bytes={size_bytes}:path={}",
        path.display()
    )
}

fn validate_source_file_size(path: &Path, size_bytes: u64, max_bytes: u64) -> CommandResult<()> {
    if size_bytes > max_bytes {
        Err(source_file_too_large_error(path, size_bytes, max_bytes))
    } else {
        Ok(())
    }
}

fn read_file_with_hash_and_limit(
    path: &Path,
    max_bytes: u64,
) -> CommandResult<(String, u64, Option<Vec<u8>>)> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(format!("source_file_not_readable:{}", path.display()));
    }
    validate_source_file_size(path, metadata.len(), max_bytes)?;

    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let capacity = usize::try_from(metadata.len()).unwrap_or(FILE_IO_BUFFER_BYTES);
    let mut bytes = Vec::with_capacity(capacity);
    let mut hasher = Sha256::new();
    let mut total_read = 0u64;
    let mut buffer = [0u8; FILE_IO_BUFFER_BYTES];

    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        total_read += read as u64;
        if total_read > max_bytes {
            return Err(source_file_too_large_error(path, total_read, max_bytes));
        }
        hasher.update(&buffer[..read]);
        bytes.extend_from_slice(&buffer[..read]);
    }

    Ok((format!("{:x}", hasher.finalize()), total_read, Some(bytes)))
}

pub(crate) fn hash_file_or_path(path: &Path) -> CommandResult<(String, u64, Option<Vec<u8>>)> {
    if path.exists() && path.is_file() {
        read_file_with_hash_and_limit(path, MAX_SOURCE_FILE_BYTES)
    } else {
        Err(format!("source_file_not_readable:{}", path.display()))
    }
}

pub(crate) fn stage_file_with_hash(path: &Path, target: &Path) -> CommandResult<(String, u64)> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(format!("source_file_not_readable:{}", path.display()));
    }
    validate_source_file_size(path, metadata.len(), MAX_SOURCE_FILE_BYTES)?;

    if target.exists() {
        return Err(format!("staged_source_target_exists:{}", target.display()));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let result = (|| {
        let mut reader = fs::File::open(path).map_err(|error| error.to_string())?;
        let mut writer = fs::File::create(target)
            .map_err(|error| format!("stage_source_file:{}:{}", target.display(), error))?;
        let mut hasher = Sha256::new();
        let mut total_read = 0u64;
        let mut buffer = [0u8; FILE_IO_BUFFER_BYTES];

        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("read_source_file:{}:{}", path.display(), error))?;
            if read == 0 {
                break;
            }
            total_read += read as u64;
            if total_read > MAX_SOURCE_FILE_BYTES {
                return Err(source_file_too_large_error(
                    path,
                    total_read,
                    MAX_SOURCE_FILE_BYTES,
                ));
            }
            hasher.update(&buffer[..read]);
            writer
                .write_all(&buffer[..read])
                .map_err(|error| format!("stage_source_file:{}:{}", target.display(), error))?;
        }

        writer
            .flush()
            .map_err(|error| format!("stage_source_file:{}:{}", target.display(), error))?;
        Ok((format!("{:x}", hasher.finalize()), total_read))
    })();

    if result.is_err() {
        let _ = fs::remove_file(target);
    }

    result
}

pub(crate) fn ensure_job_dirs(path: &Path) -> CommandResult<()> {
    for relative in [
        "uploads",
        "sources",
        "preview",
        "preview/runtime",
        "exports",
        "extraction",
        "authoring",
        "authoring/revisions",
        "authoring/patches",
        "assets",
        "assets/blobs",
        "assets/metadata",
        "assets/previews",
        "export-history",
        "legacy",
    ] {
        fs::create_dir_all(path.join(relative)).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> CommandResult<T> {
    let data = fs::read_to_string(path)
        .map_err(|error| format!("read_json:{}:{}", path.display(), error))?;
    serde_json::from_str(&data).map_err(|error| format!("parse_json:{}:{}", path.display(), error))
}

pub(crate) fn read_json_opt(path: &Path) -> CommandResult<Option<Value>> {
    if !path.exists() {
        return Ok(None);
    }
    read_json(path).map(Some)
}

pub(crate) fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, data).map_err(|error| format!("write_json:{}:{}", path.display(), error))
}

pub(crate) fn write_text(path: &Path, value: &str) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, value).map_err(|error| format!("write_text:{}:{}", path.display(), error))
}

pub(crate) fn write_bytes(path: &Path, value: &[u8]) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, value).map_err(|error| format!("write_bytes:{}:{}", path.display(), error))
}

pub(crate) fn remove_file_if_exists(path: &Path) -> CommandResult<()> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("remove_file:{}:{}", path.display(), error))?;
    }
    Ok(())
}

pub(crate) fn remove_dir_if_exists(path: &Path) -> CommandResult<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("remove_dir:{}:{}", path.display(), error))?;
    }
    Ok(())
}

pub(crate) fn append_text(path: &Path, value: &str) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("append_text:{}:{}", path.display(), error))?;
    file.write_all(value.as_bytes())
        .map_err(|error| format!("append_text:{}:{}", path.display(), error))
}

#[cfg(test)]
mod tests {
    use super::{hash_file_or_path, read_file_with_hash_and_limit, stage_file_with_hash};
    use std::{
        env, fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "ielts-author-studio-util-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn hash_file_or_path_returns_hash_and_bytes_for_small_file() {
        let dir = temp_dir("hash-small");
        let path = dir.join("sample.txt");
        let expected = b"hello world";
        fs::write(&path, expected).unwrap();

        let (hash, size, bytes) = hash_file_or_path(&path).unwrap();

        assert_eq!(hash, crate::hash_bytes(expected));
        assert_eq!(size, expected.len() as u64);
        assert_eq!(bytes.unwrap(), expected);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn hash_file_or_path_rejects_oversized_file() {
        let dir = temp_dir("hash-large");
        let path = dir.join("sample.txt");
        fs::write(&path, b"hello world").unwrap();

        let error = read_file_with_hash_and_limit(&path, 4).unwrap_err();

        assert!(error.contains("source_file_too_large"));
        assert!(error.contains("max_bytes=4"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn stage_file_with_hash_streams_and_cleans_up_partial_file() {
        let dir = temp_dir("stage");
        let source = dir.join("source.txt");
        let staged = dir.join("staged.txt");
        let expected = b"hello world";
        fs::write(&source, expected).unwrap();

        let (hash, size) = stage_file_with_hash(&source, &staged).unwrap();
        assert_eq!(hash, crate::hash_bytes(expected));
        assert_eq!(size, expected.len() as u64);
        assert_eq!(fs::read(&staged).unwrap(), expected);

        let oversized = dir.join("oversized.bin");
        let partial = dir.join("partial.bin");
        fs::write(
            &oversized,
            vec![b'x'; (super::MAX_SOURCE_FILE_BYTES + 1) as usize],
        )
        .unwrap();

        let error = stage_file_with_hash(&oversized, &partial).unwrap_err();
        assert!(error.contains("source_file_too_large"));
        assert!(!partial.exists());

        let _ = fs::remove_dir_all(dir);
    }
}
