use crate::CommandResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::{Seek, Write},
    path::{Path, PathBuf},
};

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

pub(crate) fn safe_path_segment<'a>(kind: &str, value: &'a str) -> CommandResult<&'a str> {
    validate_path_segment(kind, value)?;
    Ok(value)
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

pub(crate) fn ensure_app_dirs(root: &Path) -> CommandResult<()> {
    for relative in [
        "config",
        "config/secrets",
        "jobs",
        "packs",
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

pub(crate) fn hash_file_or_path(path: &Path) -> CommandResult<(String, u64, Option<Vec<u8>>)> {
    if path.exists() && path.is_file() {
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let size = bytes.len() as u64;
        Ok((crate::hash_bytes(&bytes), size, Some(bytes)))
    } else {
        Err(format!("source_file_not_readable:{}", path.display()))
    }
}

pub(crate) fn ensure_job_dirs(path: &Path) -> CommandResult<()> {
    for relative in ["uploads", "preview", "exports"] {
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

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn zip_safe_path(path: &str) -> CommandResult<String> {
    let normalized = path.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains("../")
        || normalized == ".."
    {
        return Err(format!("unsafe_zip_entry_path:{}", path));
    }
    Ok(normalized)
}

fn write_u16_le(writer: &mut fs::File, value: u16) -> CommandResult<()> {
    writer
        .write_all(&value.to_le_bytes())
        .map_err(|error| error.to_string())
}

fn write_u32_le(writer: &mut fs::File, value: u32) -> CommandResult<()> {
    writer
        .write_all(&value.to_le_bytes())
        .map_err(|error| error.to_string())
}

pub(crate) fn write_zip(path: &Path, entries: &[(String, Vec<u8>)]) -> CommandResult<u64> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let mut file = fs::File::create(path)
        .map_err(|error| format!("create_zip:{}:{}", path.display(), error))?;
    let mut central = Vec::new();

    for (entry_path, content) in entries {
        let safe_path = zip_safe_path(entry_path)?;
        let name = safe_path.as_bytes();
        let offset = file.stream_position().map_err(|error| error.to_string())? as u32;
        let crc = crc32(content);
        let size = content.len() as u32;

        write_u32_le(&mut file, 0x0403_4b50)?;
        write_u16_le(&mut file, 20)?;
        write_u16_le(&mut file, 0)?;
        write_u16_le(&mut file, 0)?;
        write_u16_le(&mut file, 0)?;
        write_u16_le(&mut file, 33)?;
        write_u32_le(&mut file, crc)?;
        write_u32_le(&mut file, size)?;
        write_u32_le(&mut file, size)?;
        write_u16_le(&mut file, name.len() as u16)?;
        write_u16_le(&mut file, 0)?;
        file.write_all(name).map_err(|error| error.to_string())?;
        file.write_all(content).map_err(|error| error.to_string())?;

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&33u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }

    let central_offset = file.stream_position().map_err(|error| error.to_string())? as u32;
    file.write_all(&central)
        .map_err(|error| error.to_string())?;
    write_u32_le(&mut file, 0x0605_4b50)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, 0)?;
    write_u16_le(&mut file, entries.len() as u16)?;
    write_u16_le(&mut file, entries.len() as u16)?;
    write_u32_le(&mut file, central.len() as u32)?;
    write_u32_le(&mut file, central_offset)?;
    write_u16_le(&mut file, 0)?;
    file.flush().map_err(|error| error.to_string())?;
    file.metadata()
        .map(|metadata| metadata.len())
        .map_err(|error| error.to_string())
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
