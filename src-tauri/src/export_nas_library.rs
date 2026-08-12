use crate::{
    cleanup::{cleanup_transient_job_artifacts, minimize_process_artifacts_after_authoring},
    export_artifacts::{build_manifest, build_wrapper, safe_exam_id},
    export_pack::ExportValidationOptions,
    job_store::update_job,
    reading_source::reading_source,
    runtime_validation::{publish_readiness_gate, validate_for_runtime_gate},
    util::{job_dir, read_json, validate_path_segment, write_bytes, write_json, write_text},
    CommandResult, JobStatus, WorkflowStep,
};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

const ALLOWED_CATEGORIES: &[&str] = &["P1", "P2", "P3"];
const ALLOWED_FREQUENCIES: &[&str] = &["low", "medium", "high"];
const LIBRARY_DB_FILE_NAME: &str = "library.db";
const LIBRARY_NEXT_DB_FILE_NAME: &str = "library.next.db";
const LIBRARY_VERSION_FILE_NAME: &str = "library.version.json";
const LIBRARY_SHA_FILE_NAME: &str = "library.db.sha256";
const REPORT_FILE_NAME: &str = "report.json";
const PUBLISH_DIR_NAME: &str = "publish";
const WRITING_EXAMS_DIR_NAME: &str = "writing-exams";

#[derive(Debug, Clone)]
struct SourceAssetRecord {
    exam_id: String,
    title: String,
    category: String,
    frequency: String,
    source_file: String,
    source_hash: String,
    source_size: i64,
    source_mtime: i64,
    payload_json: String,
    explanation_json: Option<String>,
    pdf_filename: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct WrittenSourceFile {
    job_id: String,
    exam_id: String,
    wrapper_js: String,
    source: Value,
}

#[derive(Debug, Clone)]
struct NasDirectWriteResult {
    files: Vec<WrittenSourceFile>,
    manifest_js: String,
    manifest_asset_count: usize,
    validation_overridden: bool,
    ignored_issues: Vec<Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct NasSourceWriteResult {
    pub exam_id: String,
    pub copied_pdf_relative: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct SourceCounts {
    html_count: u32,
    pdf_count: u32,
}

#[derive(Debug, Clone, Default)]
struct BuildState {
    assets: Vec<SourceAssetRecord>,
    counts: SourceCounts,
    errors: Vec<Value>,
}

#[derive(Debug, Clone, Default)]
struct DiffSummary {
    added: u32,
    modified: u32,
    removed: u32,
    unchanged: u32,
}

#[derive(Debug, Clone, Default)]
struct ExistingAssetSnapshot {
    hashes_by_id: HashMap<String, String>,
    asset_count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterCallPayload {
    schema_version: Option<String>,
    exam_id: Option<String>,
    meta: Option<RegisterMeta>,
    question_groups: Option<Vec<RegisterGroup>>,
    answer_key: Option<HashMap<String, Value>>,
    question_order: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterMeta {
    title: Option<String>,
    category: Option<String>,
    frequency: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterGroup {
    question_ids: Option<Vec<String>>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

fn normalize_slashes(value: &str) -> String {
    value.replace('\\', "/")
}

fn to_rel_string(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .map(|value| normalize_slashes(&value.to_string_lossy()))
        .unwrap_or_else(|| normalize_slashes(&path.to_string_lossy()))
}

pub(crate) fn normalize_nas_library_root(library_root: &Path) -> PathBuf {
    if library_root
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case(PUBLISH_DIR_NAME))
        .unwrap_or(false)
    {
        library_root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| library_root.to_path_buf())
    } else {
        library_root.to_path_buf()
    }
}

pub(crate) fn nas_publish_dir(library_root: &Path) -> PathBuf {
    if library_root
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case(PUBLISH_DIR_NAME))
        .unwrap_or(false)
    {
        library_root.to_path_buf()
    } else {
        library_root.join(PUBLISH_DIR_NAME)
    }
}

pub(crate) fn nas_reading_exams_dir(library_root: &Path) -> PathBuf {
    normalize_nas_library_root(library_root)
}

pub(crate) fn nas_writing_exams_dir(library_root: &Path) -> PathBuf {
    normalize_nas_library_root(library_root).join(WRITING_EXAMS_DIR_NAME)
}

fn list_source_files(source_dir: &Path) -> CommandResult<Vec<PathBuf>> {
    if !source_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = fs::read_dir(source_dir)
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok().map(|value| value.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case("js"))
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn skip_whitespace(input: &str, mut index: usize) -> usize {
    while let Some(ch) = input[index..].chars().next() {
        if ch.is_whitespace() {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    index
}

fn parse_json_value<T>(input: &str, start: usize) -> CommandResult<(T, usize)>
where
    T: for<'de> Deserialize<'de> + 'static,
{
    let slice = &input[start..];
    if std::any::TypeId::of::<T>() == std::any::TypeId::of::<String>() {
        let rest = &slice[1..];
        let mut escaped = false;
        for (offset, ch) in rest.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                let end = start + 1 + offset + ch.len_utf8();
                let value = serde_json::from_str::<T>(&input[start..end])
                    .map_err(|error| format!("register_payload_parse_failed:{error}"))?;
                return Ok((value, end));
            }
        }
        return Err("register_payload_parse_failed:unterminated_string".to_string());
    }

    let first = slice
        .chars()
        .next()
        .ok_or_else(|| "register_payload_parse_failed:empty_json_value".to_string())?;
    let end = match first {
        '{' | '[' => {
            let closing = if first == '{' { '}' } else { ']' };
            let mut depth = 0i32;
            let mut in_string = false;
            let mut escaped = false;
            let mut end = None;
            for (offset, ch) in slice.char_indices() {
                if in_string {
                    if escaped {
                        escaped = false;
                        continue;
                    }
                    if ch == '\\' {
                        escaped = true;
                    } else if ch == '"' {
                        in_string = false;
                    }
                    continue;
                }
                match ch {
                    '"' => in_string = true,
                    ch if ch == first => depth += 1,
                    ch if ch == closing => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(start + offset + ch.len_utf8());
                            break;
                        }
                    }
                    _ => {}
                }
            }
            end.ok_or_else(|| {
                "register_payload_parse_failed:unterminated_json_container".to_string()
            })?
        }
        _ => {
            let mut end = slice.len();
            for (offset, ch) in slice.char_indices() {
                if matches!(ch, ',' | ')' | ']' | '}') || ch.is_whitespace() {
                    end = offset;
                    break;
                }
            }
            start + end
        }
    };
    let value = serde_json::from_str::<T>(&input[start..end])
        .map_err(|error| format!("register_payload_parse_failed:{error}"))?;
    Ok((value, end))
}

fn extract_register_payload(source: &str) -> CommandResult<(String, Value)> {
    let marker = "__READING_EXAM_DATA__.register(";
    let start = source
        .find(marker)
        .ok_or_else(|| "register_call_not_found".to_string())?;
    let mut index = start + marker.len();
    index = skip_whitespace(source, index);
    let (exam_id, next) = parse_json_value::<String>(source, index)?;
    index = skip_whitespace(source, next);
    if !matches!(source.as_bytes().get(index), Some(b',')) {
        return Err("register_call_missing_payload_separator".to_string());
    }
    index += 1;
    index = skip_whitespace(source, index);
    let (payload, next) = parse_json_value::<Value>(source, index)?;
    index = skip_whitespace(source, next);
    if !matches!(source.as_bytes().get(index), Some(b')')) {
        return Err("register_call_not_closed".to_string());
    }
    Ok((exam_id, payload))
}

fn push_error(errors: &mut Vec<Value>, source_file: &str, code: &str, message: impl Into<String>) {
    errors.push(json!({
        "code": code,
        "message": message.into(),
        "sourceFile": source_file
    }));
}

fn validate_html_fields(
    value: &Value,
    field_path: &str,
    library_root: &Path,
    source_dir: &Path,
    source_file_dir: &Path,
    errors: &mut Vec<Value>,
    source_file: &str,
) {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                validate_html_fields(
                    item,
                    &format!("{field_path}[{index}]"),
                    library_root,
                    source_dir,
                    source_file_dir,
                    errors,
                    source_file,
                );
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                let next_path = format!("{field_path}.{key}");
                if matches!(
                    key.as_str(),
                    "html" | "bodyHtml" | "leadHtml" | "questionIntroHtml"
                ) {
                    if let Some(html) = item.as_str() {
                        if html.to_ascii_lowercase().contains("<script") {
                            push_error(
                                errors,
                                source_file,
                                "unsafe_html_script_tag",
                                format!("unsafe <script> tag found in {next_path}"),
                            );
                        }
                        for attr in extract_html_resource_refs(html) {
                            if let Some(missing) = missing_resource_path(
                                library_root,
                                source_dir,
                                source_file_dir,
                                &attr,
                            ) {
                                push_error(
                                    errors,
                                    source_file,
                                    "missing_resource_reference",
                                    format!("missing resource reference: {missing}"),
                                );
                            }
                        }
                        if html_attr_contains_inline_handler(html) {
                            push_error(
                                errors,
                                source_file,
                                "unsafe_html_inline_handler",
                                format!("unsafe inline handler found in {next_path}"),
                            );
                        }
                    }
                }
                validate_html_fields(
                    item,
                    &next_path,
                    library_root,
                    source_dir,
                    source_file_dir,
                    errors,
                    source_file,
                );
            }
        }
        Value::String(text) => {
            if looks_like_local_resource(text) {
                if let Some(missing) =
                    missing_resource_path(library_root, source_dir, source_file_dir, text)
                {
                    push_error(
                        errors,
                        source_file,
                        "missing_resource_reference",
                        format!("missing resource reference: {missing}"),
                    );
                }
            }
        }
        _ => {}
    }
}

fn html_attr_contains_inline_handler(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            let attr_start = index + 1;
            if attr_start + 2 < bytes.len() && &bytes[attr_start..attr_start + 2] == b"on" {
                let mut cursor = attr_start + 2;
                while cursor < bytes.len() && bytes[cursor].is_ascii_alphabetic() {
                    cursor += 1;
                }
                let cursor = skip_whitespace(&lower, cursor);
                if matches!(bytes.get(cursor), Some(b'=')) {
                    return true;
                }
            }
        }
        index += 1;
    }
    false
}

fn extract_html_resource_refs(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut refs = Vec::new();
    for attr in ["src", "href"] {
        let mut search_from = 0usize;
        let needle = format!("{attr}=");
        while let Some(pos) = lower[search_from..].find(&needle) {
            let absolute = search_from + pos + needle.len();
            let Some(quote) = html[absolute..].chars().next() else {
                break;
            };
            if quote != '"' && quote != '\'' {
                search_from = absolute;
                continue;
            }
            let value_start = absolute + quote.len_utf8();
            let Some(end_rel) = html[value_start..].find(quote) else {
                break;
            };
            refs.push(html[value_start..value_start + end_rel].to_string());
            search_from = value_start + end_rel + quote.len_utf8();
        }
    }
    refs
}

fn looks_like_local_resource(value: &str) -> bool {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    !trimmed.is_empty()
        && !lower.starts_with("http://")
        && !lower.starts_with("https://")
        && !lower.starts_with("data:")
        && !lower.starts_with("blob:")
        && !lower.starts_with("javascript:")
        && !lower.starts_with('#')
        && [".pdf", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg"]
            .iter()
            .any(|suffix| lower.ends_with(suffix))
}

fn missing_resource_path(
    library_root: &Path,
    source_dir: &Path,
    source_file_dir: &Path,
    value: &str,
) -> Option<String> {
    let trimmed = value.trim();
    if !looks_like_local_resource(trimmed) {
        return None;
    }
    let normalized = trimmed.replace('\\', "/");
    let candidates = if Path::new(&normalized).is_absolute() {
        vec![PathBuf::from(&normalized)]
    } else {
        vec![
            source_file_dir.join(&normalized),
            source_dir.join(&normalized),
            library_root.join(&normalized),
        ]
    };
    if candidates.iter().any(|path| path.exists()) {
        None
    } else {
        Some(normalized)
    }
}

fn validate_payload_contract(
    payload: &Value,
    source_file: &str,
    errors: &mut Vec<Value>,
) -> Option<RegisterCallPayload> {
    let parsed = serde_json::from_value::<RegisterCallPayload>(payload.clone()).map_err(|error| {
        push_error(
            errors,
            source_file,
            "payload_json_parse_failed",
            format!("payload is not a valid ReadingExamSourceV1 object: {error}"),
        );
    });
    let Ok(parsed) = parsed else {
        return None;
    };

    if parsed.schema_version.as_deref() != Some("ReadingExamSourceV1") {
        push_error(
            errors,
            source_file,
            "invalid_schema_version",
            "schemaVersion must be ReadingExamSourceV1",
        );
    }
    let exam_id = parsed.exam_id.as_deref().unwrap_or_default().trim();
    if exam_id.is_empty() {
        push_error(errors, source_file, "missing_exam_id", "examId is required");
    }
    let category = parsed
        .meta
        .as_ref()
        .and_then(|meta| meta.category.as_deref())
        .unwrap_or_default()
        .trim()
        .to_string();
    if !ALLOWED_CATEGORIES.iter().any(|value| *value == category) {
        push_error(
            errors,
            source_file,
            "invalid_category",
            format!(
                "unsupported category: {}",
                if category.is_empty() {
                    "empty"
                } else {
                    &category
                }
            ),
        );
    }
    let frequency = parsed
        .meta
        .as_ref()
        .and_then(|meta| meta.frequency.as_deref())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !ALLOWED_FREQUENCIES.iter().any(|value| *value == frequency) {
        push_error(
            errors,
            source_file,
            "invalid_frequency",
            format!(
                "unsupported frequency: {}",
                if frequency.is_empty() {
                    "empty"
                } else {
                    &frequency
                }
            ),
        );
    }

    let question_order = parsed.question_order.clone().unwrap_or_default();
    if question_order.is_empty() {
        push_error(
            errors,
            source_file,
            "empty_question_order",
            "questionOrder is required",
        );
    }
    let first_number = question_order
        .first()
        .and_then(|qid| qid.strip_prefix('q'))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    for (index, qid) in question_order.iter().enumerate() {
        let expected = format!("q{}", first_number + index);
        if qid != &expected {
            push_error(
                errors,
                source_file,
                "non_contiguous_question_order",
                format!("expected {expected}, received {qid}"),
            );
            break;
        }
    }

    let answer_key = parsed.answer_key.clone().unwrap_or_default();
    for qid in &question_order {
        if !answer_key.contains_key(qid) {
            push_error(
                errors,
                source_file,
                "answer_key_missing_question",
                format!("answerKey does not cover {qid}"),
            );
        }
    }

    let mut covered = HashSet::new();
    for group in parsed.question_groups.as_ref().into_iter().flatten() {
        for qid in group.question_ids.as_ref().into_iter().flatten() {
            covered.insert(qid.to_string());
        }
    }
    for qid in &question_order {
        if !covered.contains(qid) {
            push_error(
                errors,
                source_file,
                "question_group_missing_question",
                format!("questionGroups do not cover {qid}"),
            );
        }
    }
    Some(parsed)
}

fn load_existing_snapshot(library_db_path: &Path) -> CommandResult<ExistingAssetSnapshot> {
    if !library_db_path.exists() {
        return Ok(ExistingAssetSnapshot::default());
    }
    let connection = match Connection::open(library_db_path) {
        Ok(connection) => connection,
        Err(_) => return Ok(ExistingAssetSnapshot::default()),
    };
    let mut stmt = match connection
        .prepare("SELECT id, source_hash FROM reading_assets WHERE status = 'active'")
    {
        Ok(stmt) => stmt,
        Err(_) => return Ok(ExistingAssetSnapshot::default()),
    };
    let rows = match stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
        Ok(rows) => rows,
        Err(_) => return Ok(ExistingAssetSnapshot::default()),
    };
    let mut snapshot = ExistingAssetSnapshot::default();
    for row in rows {
        let (id, hash) = row.map_err(|error| error.to_string())?;
        snapshot.hashes_by_id.insert(id, hash);
    }
    snapshot.asset_count = snapshot.hashes_by_id.len() as u32;
    Ok(snapshot)
}

fn compute_diff(previous: &ExistingAssetSnapshot, next: &[SourceAssetRecord]) -> DiffSummary {
    let mut summary = DiffSummary::default();
    for asset in next {
        match previous.hashes_by_id.get(&asset.exam_id) {
            None => summary.added += 1,
            Some(previous_hash) if previous_hash != &asset.source_hash => summary.modified += 1,
            Some(_) => summary.unchanged += 1,
        }
    }
    let next_ids = next
        .iter()
        .map(|asset| asset.exam_id.as_str())
        .collect::<HashSet<_>>();
    for previous_id in previous.hashes_by_id.keys() {
        if !next_ids.contains(previous_id.as_str()) {
            summary.removed += 1;
        }
    }
    summary
}

fn create_schema(connection: &Connection) -> CommandResult<()> {
    connection
        .execute_batch(
            "
            CREATE TABLE library_meta (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );

            CREATE TABLE reading_assets (
              id TEXT PRIMARY KEY,
              exam_id TEXT NOT NULL,
              title TEXT NOT NULL,
              category TEXT,
              frequency TEXT,
              source_file TEXT NOT NULL,
              source_hash TEXT NOT NULL,
              source_size INTEGER NOT NULL,
              source_mtime INTEGER NOT NULL,
              payload_json TEXT NOT NULL,
              explanation_json TEXT,
              pdf_filename TEXT,
              status TEXT NOT NULL DEFAULT 'active',
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL
            );

            CREATE INDEX idx_reading_assets_status ON reading_assets(status);
            CREATE INDEX idx_reading_assets_category ON reading_assets(category);
            CREATE INDEX idx_reading_assets_hash ON reading_assets(source_hash);
            ",
        )
        .map_err(|error| error.to_string())
}

fn write_library_db(
    next_db_path: &Path,
    assets: &[SourceAssetRecord],
    version_meta: &Value,
) -> CommandResult<()> {
    if next_db_path.exists() {
        fs::remove_file(next_db_path).map_err(|error| error.to_string())?;
    }
    let mut connection = Connection::open(next_db_path).map_err(|error| error.to_string())?;
    create_schema(&connection)?;
    let tx = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    {
        let mut insert_meta = tx
            .prepare("INSERT INTO library_meta (key, value) VALUES (?1, ?2)")
            .map_err(|error| error.to_string())?;
        for (key, value) in [
            (
                "version",
                version_meta.get("version").cloned().unwrap_or(Value::Null),
            ),
            (
                "libraryVersion",
                version_meta
                    .get("libraryVersion")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "buildId",
                version_meta.get("buildId").cloned().unwrap_or(Value::Null),
            ),
            (
                "generatedAt",
                version_meta
                    .get("generatedAt")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "assetCount",
                version_meta
                    .get("assetCount")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "htmlCount",
                version_meta
                    .get("htmlCount")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "pdfCount",
                version_meta.get("pdfCount").cloned().unwrap_or(Value::Null),
            ),
        ] {
            insert_meta
                .execute(params![key, value.to_string()])
                .map_err(|error| error.to_string())?;
        }
    }
    {
        let mut insert_asset = tx
            .prepare(
                "
                INSERT INTO reading_assets (
                  id, exam_id, title, category, frequency, source_file, source_hash,
                  source_size, source_mtime, payload_json, explanation_json,
                  pdf_filename, status, created_at, updated_at
                ) VALUES (
                  ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                  ?8, ?9, ?10, ?11,
                  ?12, 'active', ?13, ?14
                )
                ",
            )
            .map_err(|error| error.to_string())?;
        for asset in assets {
            insert_asset
                .execute(params![
                    asset.exam_id,
                    asset.exam_id,
                    asset.title,
                    asset.category,
                    asset.frequency,
                    asset.source_file,
                    asset.source_hash,
                    asset.source_size,
                    asset.source_mtime,
                    asset.payload_json,
                    asset.explanation_json,
                    asset.pdf_filename,
                    asset.created_at,
                    asset.updated_at,
                ])
                .map_err(|error| error.to_string())?;
        }
    }
    tx.commit().map_err(|error| error.to_string())
}

fn replace_atomically(next_path: &Path, target_path: &Path) -> CommandResult<()> {
    let backup_path = target_path.with_extension("db.bak");
    if backup_path.exists() {
        fs::remove_file(&backup_path).map_err(|error| error.to_string())?;
    }
    if target_path.exists() {
        fs::rename(target_path, &backup_path).map_err(|error| error.to_string())?;
    }
    match fs::rename(next_path, target_path) {
        Ok(()) => {
            if backup_path.exists() {
                fs::remove_file(backup_path).map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        Err(error) => {
            if backup_path.exists() && !target_path.exists() {
                let _ = fs::rename(&backup_path, target_path);
            }
            Err(error.to_string())
        }
    }
}

const READING_MANIFEST_GLOBAL: &str = "window.__READING_EXAM_MANIFEST__";

fn parse_reading_manifest_js(source: &str) -> CommandResult<Map<String, Value>> {
    let source = source.trim_start_matches('\u{feff}').trim();
    let payload = source
        .strip_prefix(READING_MANIFEST_GLOBAL)
        .ok_or_else(|| "nas_manifest_parse_failed:missing_assignment".to_string())?
        .trim_start()
        .strip_prefix('=')
        .ok_or_else(|| "nas_manifest_parse_failed:missing_assignment_operator".to_string())?
        .trim();
    let payload = payload.strip_suffix(';').unwrap_or(payload).trim();
    let manifest: Value = serde_json::from_str(payload)
        .map_err(|error| format!("nas_manifest_parse_failed:invalid_json:{error}"))?;
    manifest
        .as_object()
        .cloned()
        .ok_or_else(|| "nas_manifest_parse_failed:root_must_be_object".to_string())
}

fn serialize_reading_manifest(manifest: Map<String, Value>) -> CommandResult<String> {
    Ok(format!(
        "{READING_MANIFEST_GLOBAL} = {};\n",
        serde_json::to_string_pretty(&Value::Object(manifest)).map_err(|error| error.to_string())?
    ))
}

fn merge_reading_manifest_js(
    existing_manifest_js: Option<&str>,
    selected_manifest_js: &str,
) -> CommandResult<(String, usize)> {
    let mut selected = parse_reading_manifest_js(selected_manifest_js)?;
    let selected_meta = selected.remove("_meta");
    let selected_exam_ids = selected.keys().cloned().collect::<HashSet<_>>();

    let mut merged = match existing_manifest_js {
        Some(source) => parse_reading_manifest_js(source)?,
        None => Map::new(),
    };
    let existing_meta = merged.remove("_meta");

    // examId is the stable identity. Remove both the canonical key and any
    // legacy alias carrying the same examId before inserting this batch.
    merged.retain(|key, value| {
        let entry_exam_id = value.get("examId").and_then(Value::as_str);
        !selected_exam_ids.contains(key)
            && !entry_exam_id.is_some_and(|exam_id| selected_exam_ids.contains(exam_id))
    });
    merged.extend(selected);

    let asset_count = merged.len();
    let mut metadata = existing_meta
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    if let Some(selected_metadata) = selected_meta.and_then(|value| value.as_object().cloned()) {
        metadata.extend(selected_metadata);
    }
    metadata.insert("assetCount".to_string(), json!(asset_count));
    merged.insert("_meta".to_string(), Value::Object(metadata));

    Ok((serialize_reading_manifest(merged)?, asset_count))
}

#[derive(Debug)]
struct CommittedNasFile {
    target: PathBuf,
    backup: Option<PathBuf>,
}

fn rollback_committed_nas_files(committed: &mut Vec<CommittedNasFile>) -> Vec<String> {
    let mut errors = Vec::new();
    while let Some(file) = committed.pop() {
        if file.target.exists() {
            if let Err(error) = fs::remove_file(&file.target) {
                errors.push(format!("remove {}: {error}", file.target.display()));
                continue;
            }
        }
        if let Some(backup) = file.backup {
            if let Err(error) = fs::rename(&backup, &file.target) {
                errors.push(format!(
                    "restore {} from {}: {error}",
                    file.target.display(),
                    backup.display()
                ));
            }
        }
    }
    errors
}

fn commit_staged_nas_files(
    reading_exams_dir: &Path,
    staging_dir: &Path,
    file_names: &[String],
) -> CommandResult<()> {
    let backup_dir = staging_dir.join("backups");
    fs::create_dir_all(&backup_dir).map_err(|error| error.to_string())?;
    let mut committed = Vec::with_capacity(file_names.len());

    for (index, file_name) in file_names.iter().enumerate() {
        let staged = staging_dir.join(file_name);
        let target = reading_exams_dir.join(file_name);
        let backup = if target.exists() {
            if !target.is_file() {
                let rollback_errors = rollback_committed_nas_files(&mut committed);
                return Err(format!(
                    "nas_direct_publish_target_not_file:{}{}",
                    target.display(),
                    if rollback_errors.is_empty() {
                        String::new()
                    } else {
                        format!(":rollback_failed:{}", rollback_errors.join(" | "))
                    }
                ));
            }
            let backup = backup_dir.join(format!("{index}.bak"));
            if let Err(error) = fs::rename(&target, &backup) {
                let rollback_errors = rollback_committed_nas_files(&mut committed);
                return Err(format!(
                    "nas_direct_publish_backup_failed:{}:{error}{}",
                    target.display(),
                    if rollback_errors.is_empty() {
                        String::new()
                    } else {
                        format!(":rollback_failed:{}", rollback_errors.join(" | "))
                    }
                ));
            }
            Some(backup)
        } else {
            None
        };

        if let Err(error) = fs::rename(&staged, &target) {
            let mut current = vec![CommittedNasFile { target, backup }];
            let mut rollback_errors = rollback_committed_nas_files(&mut current);
            rollback_errors.extend(rollback_committed_nas_files(&mut committed));
            return Err(format!(
                "nas_direct_publish_commit_failed:{file_name}:{error}{}",
                if rollback_errors.is_empty() {
                    String::new()
                } else {
                    format!(":rollback_failed:{}", rollback_errors.join(" | "))
                }
            ));
        }
        committed.push(CommittedNasFile { target, backup });
    }

    Ok(())
}

fn write_nas_direct_artifacts(
    reading_exams_dir: &Path,
    files: &[WrittenSourceFile],
) -> CommandResult<(String, usize)> {
    let sources = files
        .iter()
        .map(|written| written.source.clone())
        .collect::<Vec<_>>();
    let selected_manifest_js = build_manifest(&sources)?;
    let manifest_path = reading_exams_dir.join("manifest.js");
    let existing_manifest_js = if manifest_path.exists() {
        Some(fs::read_to_string(&manifest_path).map_err(|error| {
            format!(
                "nas_manifest_read_failed:{}:{error}",
                manifest_path.display()
            )
        })?)
    } else {
        None
    };
    let (manifest_js, manifest_asset_count) =
        merge_reading_manifest_js(existing_manifest_js.as_deref(), &selected_manifest_js)?;

    fs::create_dir_all(reading_exams_dir).map_err(|error| error.to_string())?;
    let staging_dir = reading_exams_dir.join(format!(
        ".nas-publish-staging-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir(&staging_dir).map_err(|error| error.to_string())?;

    let result = (|| {
        let mut file_names = Vec::with_capacity(files.len() + 1);
        for file in files {
            let file_name = format!("{}.js", file.exam_id);
            write_text(&staging_dir.join(&file_name), &file.wrapper_js)?;
            file_names.push(file_name);
        }
        write_text(&staging_dir.join("manifest.js"), &manifest_js)?;
        // The manifest is the discovery/commit point, so it must be last.
        file_names.push("manifest.js".to_string());
        commit_staged_nas_files(reading_exams_dir, &staging_dir, &file_names)
    })();

    let _ = fs::remove_dir_all(&staging_dir);
    result?;
    Ok((manifest_js, manifest_asset_count))
}

pub(crate) fn write_nas_reading_sources(
    reading_exams_dir: &Path,
    sources: &[Value],
) -> CommandResult<(String, usize)> {
    let mut seen_exam_ids = HashSet::new();
    let mut files = Vec::with_capacity(sources.len());
    for source in sources {
        let exam_id = safe_exam_id(source)?;
        if !seen_exam_ids.insert(exam_id.clone()) {
            return Err(format!("duplicate_exam_id:{exam_id}"));
        }
        files.push(WrittenSourceFile {
            job_id: String::new(),
            exam_id,
            wrapper_js: build_wrapper(source)?,
            source: source.clone(),
        });
    }
    write_nas_direct_artifacts(reading_exams_dir, &files)
}

pub(crate) fn resolve_real_nas_library_root(export_dir: &str) -> CommandResult<PathBuf> {
    let export_dir = export_dir.trim();
    if export_dir.is_empty() {
        return Err("nas_export_requires_library_root".to_string());
    }
    if export_dir
        .get(.."local://".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("local://"))
    {
        return Err(
            "nas_export_requires_real_library_root:local_placeholder_not_allowed".to_string(),
        );
    }
    Ok(normalize_nas_library_root(&PathBuf::from(export_dir)))
}

fn write_selected_nas_direct_files(
    root: &Path,
    job_ids: &[String],
    reading_exams_dir: &Path,
    require_static_runtime_gate: bool,
    options: ExportValidationOptions,
) -> CommandResult<NasDirectWriteResult> {
    let mut files = Vec::with_capacity(job_ids.len());
    let mut seen_exam_ids = HashSet::new();
    let mut validation_overridden = false;
    let mut ignored_issues = Vec::new();

    for job_id in job_ids {
        validate_path_segment("job_id", job_id)?;
        let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
        let report = validate_for_runtime_gate(root, job_id, &ir, require_static_runtime_gate)?;
        let report = publish_readiness_gate(root, job_id, &ir, report)?;
        write_json(
            &job_dir(root, job_id).join("validation-report.json"),
            &report,
        )?;
        validation_overridden |= options.validation_overridden(&report);
        ignored_issues.extend(options.ignored_issues(job_id, &report));
        if options.should_block(&report) {
            let _ = minimize_process_artifacts_after_authoring(
                root,
                job_id,
                "nas_js_direct_export_publish_gate_failed",
            )?;
            return Err(format!(
                "nas_export_validation_failed:{}:{}",
                job_id,
                serde_json::to_string(&report).unwrap_or_default()
            ));
        }

        let source = reading_source(&ir);
        let exam_id = source
            .get("examId")
            .and_then(Value::as_str)
            .unwrap_or("local-authoring-exam")
            .to_string();
        if !seen_exam_ids.insert(exam_id.clone()) {
            return Err(format!("duplicate_exam_id:{exam_id}"));
        }
        let wrapper = build_wrapper(&source)?;
        files.push(WrittenSourceFile {
            job_id: job_id.clone(),
            exam_id,
            wrapper_js: wrapper,
            source,
        });
    }

    let (manifest_js, manifest_asset_count) =
        write_nas_direct_artifacts(reading_exams_dir, &files)?;
    Ok(NasDirectWriteResult {
        files,
        manifest_js,
        manifest_asset_count,
        validation_overridden,
        ignored_issues,
    })
}

pub(crate) fn copy_pdf_into_source_tree(
    pdf_path: &Path,
    source_dir: &Path,
    source: &mut Value,
) -> CommandResult<Option<String>> {
    if !pdf_path.exists() || !pdf_path.is_file() {
        return Err(format!("source_file_not_readable:{}", pdf_path.display()));
    }
    let exam_id = source
        .get("examId")
        .and_then(Value::as_str)
        .unwrap_or("local-authoring-exam");
    let assets_dir = source_dir.join("assets").join(exam_id);
    fs::create_dir_all(&assets_dir).map_err(|error| error.to_string())?;
    let original_name = pdf_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("source.pdf");
    let safe_name = crate::util::sanitize_filename(original_name);
    let dest_path = assets_dir.join(&safe_name);
    let bytes = fs::read(pdf_path).map_err(|error| error.to_string())?;
    write_bytes(&dest_path, &bytes)?;
    let relative = normalize_slashes(
        &dest_path
            .strip_prefix(source_dir)
            .ok()
            .unwrap_or(dest_path.as_path())
            .to_string_lossy(),
    );

    if let Some(meta) = source.get_mut("meta").and_then(Value::as_object_mut) {
        meta.insert("pdfFilename".to_string(), Value::String(relative.clone()));
    }
    if let Some(source_refs) = source.get_mut("sourceRefs").and_then(Value::as_object_mut) {
        source_refs.insert("shuiPdf".to_string(), Value::String(relative.clone()));
    }
    Ok(Some(relative))
}

pub(crate) fn write_source_payload_file(
    source_dir: &Path,
    source: &mut Value,
    pdf_path: Option<&Path>,
) -> CommandResult<NasSourceWriteResult> {
    let copied_pdf_relative = if let Some(path) = pdf_path {
        copy_pdf_into_source_tree(path, source_dir, source)?
    } else {
        None
    };
    let exam_id = source
        .get("examId")
        .and_then(Value::as_str)
        .unwrap_or("local-authoring-exam")
        .to_string();
    let wrapper_js = build_wrapper(source)?;
    write_text(&source_dir.join(format!("{exam_id}.js")), &wrapper_js)?;
    Ok(NasSourceWriteResult {
        exam_id,
        copied_pdf_relative,
    })
}

fn build_library_from_source_tree(
    library_root: &Path,
    source_dir: &Path,
    version: &str,
    previous: &ExistingAssetSnapshot,
) -> CommandResult<(BuildState, Value, DiffSummary)> {
    let mut state = BuildState::default();
    let source_files = list_source_files(source_dir)?;
    let generated_at = Utc::now().to_rfc3339();
    let generated_at_ts = Utc::now().timestamp_millis();
    let mut exam_to_file = HashMap::<String, String>::new();
    let mut hash_to_file = HashMap::<String, String>::new();

    for source_file_path in source_files {
        let source_text =
            fs::read_to_string(&source_file_path).map_err(|error| error.to_string())?;
        let source_hash = sha256_hex(source_text.as_bytes());
        let source_file_rel = to_rel_string(library_root, &source_file_path);
        let (captured_exam_id, payload) = match extract_register_payload(&source_text) {
            Ok(parsed) => parsed,
            Err(error) => {
                push_error(
                    &mut state.errors,
                    &source_file_rel,
                    "reading_asset_parse_failed",
                    error,
                );
                continue;
            }
        };

        validate_html_fields(
            &payload,
            "$",
            library_root,
            source_dir,
            source_file_path.parent().unwrap_or(source_dir),
            &mut state.errors,
            &source_file_rel,
        );
        let parsed = validate_payload_contract(&payload, &source_file_rel, &mut state.errors);
        let Some(parsed) = parsed else {
            continue;
        };
        let exam_id = parsed
            .exam_id
            .clone()
            .unwrap_or_else(|| captured_exam_id.clone());
        if !captured_exam_id.trim().is_empty() && captured_exam_id != exam_id {
            push_error(
                &mut state.errors,
                &source_file_rel,
                "reading_asset_key_mismatch",
                format!("register key does not match examId: {captured_exam_id} != {exam_id}"),
            );
        }
        if let Some(previous_file) = exam_to_file.insert(exam_id.clone(), source_file_rel.clone()) {
            push_error(
                &mut state.errors,
                &source_file_rel,
                "duplicate_exam_id",
                format!(
                    "duplicate examId detected: {exam_id}; previous source file: {previous_file}"
                ),
            );
        }
        if let Some(previous_file) =
            hash_to_file.insert(source_hash.clone(), source_file_rel.clone())
        {
            push_error(
                &mut state.errors,
                &source_file_rel,
                "duplicate_source_hash",
                format!(
                    "duplicate source hash detected: {source_hash}; previous source file: {previous_file}"
                ),
            );
        }

        let stats = fs::metadata(&source_file_path).map_err(|error| error.to_string())?;
        let title = parsed
            .meta
            .as_ref()
            .and_then(|meta| meta.title.clone())
            .unwrap_or_else(|| exam_id.clone());
        let category = parsed
            .meta
            .as_ref()
            .and_then(|meta| meta.category.clone())
            .unwrap_or_else(|| "P1".to_string());
        let frequency = parsed
            .meta
            .as_ref()
            .and_then(|meta| meta.frequency.clone())
            .unwrap_or_else(|| "medium".to_string());
        let pdf_filename = payload
            .pointer("/meta/pdfFilename")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        state.counts.html_count += 1;
        if pdf_filename.is_some() {
            state.counts.pdf_count += 1;
        }
        state.assets.push(SourceAssetRecord {
            exam_id,
            title,
            category,
            frequency,
            source_file: source_file_rel,
            source_hash,
            source_size: stats.len() as i64,
            source_mtime: stats
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i64)
                .unwrap_or(generated_at_ts),
            payload_json: serde_json::to_string_pretty(&payload)
                .map_err(|error| error.to_string())?,
            explanation_json: None,
            pdf_filename,
            created_at: generated_at_ts,
            updated_at: generated_at_ts,
        });
    }

    state
        .assets
        .sort_by(|left, right| left.exam_id.cmp(&right.exam_id));
    let version_meta = json!({
        "version": version,
        "libraryVersion": version,
        "buildId": version,
        "generatedAt": generated_at,
        "assetCount": state.assets.len(),
        "htmlCount": state.counts.html_count,
        "pdfCount": state.counts.pdf_count
    });
    let diff = compute_diff(previous, &state.assets);
    Ok((state, version_meta, diff))
}

pub(crate) fn publish_nas_library_from_source_tree(
    library_root: &Path,
    version: Option<&str>,
) -> CommandResult<Value> {
    let library_root = normalize_nas_library_root(library_root);
    fs::create_dir_all(&library_root).map_err(|error| error.to_string())?;
    let source_dir = library_root.join("source");
    let publish_dir = nas_publish_dir(&library_root);
    fs::create_dir_all(&source_dir).map_err(|error| error.to_string())?;
    fs::create_dir_all(&publish_dir).map_err(|error| error.to_string())?;

    let requested_version = version.map(str::trim);
    let version = requested_version
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| Utc::now().format("%Y.%m.%d-%H%M%S").to_string());
    let library_db_path = publish_dir.join(LIBRARY_DB_FILE_NAME);
    let library_next_db_path = publish_dir.join(LIBRARY_NEXT_DB_FILE_NAME);
    let version_path = publish_dir.join(LIBRARY_VERSION_FILE_NAME);
    let sha_path = publish_dir.join(LIBRARY_SHA_FILE_NAME);
    let report_path = publish_dir.join(REPORT_FILE_NAME);

    let previous = load_existing_snapshot(&library_db_path)?;
    let (state, version_meta, diff) =
        build_library_from_source_tree(&library_root, &source_dir, &version, &previous)?;
    let source_file_count = list_source_files(&source_dir)?.len();
    let has_errors = !state.errors.is_empty();

    let status = if state.errors.is_empty() {
        "ok"
    } else {
        "failed"
    };
    let report = json!({
        "status": status,
        "version": version,
        "generatedAt": version_meta.get("generatedAt").cloned().unwrap_or(Value::Null),
        "summary": {
            "sourceFileCount": source_file_count,
            "assetCountBefore": previous.asset_count,
            "assetCountAfter": state.assets.len(),
            "added": diff.added,
            "modified": diff.modified,
            "removed": diff.removed,
            "unchanged": diff.unchanged,
            "failed": state.errors.len(),
            "htmlCount": state.counts.html_count,
            "pdfCount": state.counts.pdf_count
        },
        "errors": state.errors.clone(),
    });
    write_json(&report_path, &report)?;
    if has_errors {
        return Err(format!(
            "nas_publish_failed:{}",
            serde_json::to_string(&report).unwrap_or_default()
        ));
    }

    write_library_db(&library_next_db_path, &state.assets, &version_meta)?;
    replace_atomically(&library_next_db_path, &library_db_path)?;
    write_json(&version_path, &version_meta)?;
    let db_bytes = fs::read(&library_db_path).map_err(|error| error.to_string())?;
    let sha = sha256_hex(&db_bytes);
    write_text(&sha_path, &format!("{sha}  {LIBRARY_DB_FILE_NAME}\n"))?;

    Ok(json!({
        "mode": "nas-library",
        "assetCount": state.assets.len(),
        "libraryRoot": library_root.to_string_lossy(),
        "sourceDir": source_dir.to_string_lossy(),
        "publishDir": publish_dir.to_string_lossy(),
        "version": version,
        "report": report,
        "files": [
            json!({"name": "publish/library.db", "content": format!("sqlite:{} bytes", db_bytes.len())}),
            json!({"name": "publish/library.version.json", "content": serde_json::to_string_pretty(&version_meta).unwrap_or_default()}),
            json!({"name": "publish/library.db.sha256", "content": format!("{sha}  {LIBRARY_DB_FILE_NAME}\n")}),
            json!({"name": "publish/report.json", "content": serde_json::to_string_pretty(&report).unwrap_or_default()}),
        ]
    }))
}

pub(crate) fn export_nas_library_core(
    root: &Path,
    input: &Value,
    require_static_runtime_gate: bool,
) -> CommandResult<Value> {
    let options = ExportValidationOptions::from_input(input)?;
    let job_ids = input
        .get("jobIds")
        .and_then(Value::as_array)
        .ok_or_else(|| "nas_export_requires_job_ids".to_string())?
        .iter()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if job_ids.is_empty() {
        return Err("nas_export_requires_at_least_one_job".to_string());
    }
    let export_dir = input
        .get("exportDir")
        .and_then(Value::as_str)
        .ok_or_else(|| "nas_export_requires_library_root".to_string())?;
    let library_root = resolve_real_nas_library_root(export_dir)?;
    fs::create_dir_all(&library_root).map_err(|error| error.to_string())?;
    let reading_exams_dir = nas_reading_exams_dir(&library_root);

    let requested_version = input.get("version").and_then(Value::as_str).map(str::trim);
    let version = requested_version
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| Utc::now().format("%Y.%m.%d-%H%M%S").to_string());

    let write_result = write_selected_nas_direct_files(
        root,
        &job_ids,
        &reading_exams_dir,
        require_static_runtime_gate,
        options,
    )?;
    let NasDirectWriteResult {
        files: written_sources,
        manifest_js,
        manifest_asset_count,
        validation_overridden,
        ignored_issues,
    } = write_result;
    let asset_count = written_sources.len();
    let ignored_issue_count = ignored_issues.len() as u64;
    let report = json!({
        "status": "ok",
        "version": version.clone(),
        "generatedAt": Utc::now().to_rfc3339(),
        "summary": {
            "runtime": "nas-js-direct",
            "readingExamFileCount": asset_count,
            "manifestFileCount": 1,
            "manifestAssetCount": manifest_asset_count,
            "assetCount": asset_count,
            "validationPolicy": options.policy_name(),
            "validationOverridden": validation_overridden,
            "ignoredIssueCount": ignored_issue_count
        },
        "errors": []
    });

    let mut cleanup = Vec::with_capacity(written_sources.len());
    for written in &written_sources {
        update_job(root, &written.job_id, |job| {
            job.status = JobStatus::Exported;
            job.current_step = WorkflowStep::Export;
        })?;
        cleanup.push(cleanup_transient_job_artifacts(
            root,
            &written.job_id,
            json!({
                "type": "nas-library",
                "runtime": "nas-js-direct",
                "version": version,
                "outputDir": library_root.to_string_lossy(),
                "validationPolicy": options.policy_name(),
                "validationOverridden": validation_overridden,
                "ignoredIssueCount": ignored_issue_count,
                "ignoredIssues": ignored_issues.clone(),
                "exportedAt": Utc::now().to_rfc3339()
            }),
        )?);
    }

    let mut files = written_sources
        .iter()
        .map(|written| json!({"name": format!("{}.js", written.exam_id), "content": written.wrapper_js}))
        .collect::<Vec<_>>();
    files.push(json!({
        "name": "manifest.js",
        "content": manifest_js
    }));

    Ok(json!({
        "mode": "nas-library",
        "jobIds": written_sources.iter().map(|written| written.job_id.clone()).collect::<Vec<_>>(),
        "examIds": written_sources.iter().map(|written| written.exam_id.clone()).collect::<Vec<_>>(),
        "assetCount": asset_count,
        "manifestAssetCount": manifest_asset_count,
        "libraryRoot": library_root.to_string_lossy(),
        "readingExamsDir": reading_exams_dir.to_string_lossy(),
        "version": version.clone(),
        "files": files,
        "report": report,
        "validationPolicy": options.policy_name(),
        "validationOverridden": validation_overridden,
        "ignoredIssueCount": ignored_issue_count,
        "ignoredIssues": ignored_issues.clone(),
        "exportSummary": {
            "type": "nas-library",
            "runtime": "nas-js-direct",
            "jobIds": job_ids,
            "version": Value::String(version.clone()),
            "outputDir": library_root.to_string_lossy(),
            "readingExamsDir": reading_exams_dir.to_string_lossy(),
            "assetCount": asset_count,
            "manifestAssetCount": manifest_asset_count,
            "validationPolicy": options.policy_name(),
            "validationOverridden": validation_overridden,
            "ignoredIssueCount": ignored_issue_count,
            "ignoredIssues": ignored_issues,
            "exportedAt": Utc::now().to_rfc3339()
        },
        "cleanup": cleanup
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "nas-export-safety-test-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    fn source(exam_id: &str, title: &str) -> Value {
        json!({
            "examId": exam_id,
            "meta": {
                "title": title,
                "category": "P1"
            }
        })
    }

    fn written_source(exam_id: &str, title: &str) -> WrittenSourceFile {
        let source = source(exam_id, title);
        WrittenSourceFile {
            job_id: format!("job-{exam_id}"),
            exam_id: exam_id.to_string(),
            wrapper_js: build_wrapper(&source).unwrap(),
            source,
        }
    }

    #[test]
    fn nas_direct_subset_publish_preserves_unselected_manifest_entries() {
        let root = temp_root();
        fs::create_dir_all(&root).unwrap();
        let old_a = source("exam-a", "A unchanged");
        let old_b = source("exam-b", "B old");
        write_text(&root.join("exam-a.js"), "legacy-a-wrapper").unwrap();
        write_text(&root.join("exam-b.js"), "legacy-b-wrapper").unwrap();
        write_text(
            &root.join("manifest.js"),
            &build_manifest(&[old_a.clone(), old_b]).unwrap(),
        )
        .unwrap();

        let selected = vec![
            written_source("exam-b", "B updated"),
            written_source("exam-c", "C added"),
        ];
        let (manifest_js, manifest_asset_count) =
            write_nas_direct_artifacts(&root, &selected).unwrap();
        let manifest = Value::Object(parse_reading_manifest_js(&manifest_js).unwrap());

        assert_eq!(manifest_asset_count, 3);
        assert_eq!(
            manifest.pointer("/exam-a/title").and_then(Value::as_str),
            Some("A unchanged")
        );
        assert_eq!(
            manifest.pointer("/exam-b/title").and_then(Value::as_str),
            Some("B updated")
        );
        assert_eq!(
            manifest.pointer("/exam-c/title").and_then(Value::as_str),
            Some("C added")
        );
        assert_eq!(
            manifest
                .pointer("/_meta/assetCount")
                .and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            fs::read_to_string(root.join("exam-a.js")).unwrap(),
            "legacy-a-wrapper"
        );
        assert_eq!(
            fs::read_to_string(root.join("manifest.js")).unwrap(),
            manifest_js
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_existing_manifest_blocks_publish_before_writing_assets() {
        let root = temp_root();
        fs::create_dir_all(&root).unwrap();
        let malformed = "window.__READING_EXAM_MANIFEST__ = { broken };\n";
        write_text(&root.join("manifest.js"), malformed).unwrap();

        let error =
            write_nas_direct_artifacts(&root, &[written_source("exam-new", "Must not be written")])
                .unwrap_err();

        assert!(error.starts_with("nas_manifest_parse_failed:invalid_json:"));
        assert!(!root.join("exam-new.js").exists());
        assert_eq!(
            fs::read_to_string(root.join("manifest.js")).unwrap(),
            malformed
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn staged_nas_commit_rolls_back_files_when_manifest_commit_cannot_start() {
        let root = temp_root();
        let staging = root.join("staging");
        fs::create_dir_all(&staging).unwrap();
        write_text(&root.join("exam-a.js"), "old-wrapper").unwrap();
        fs::create_dir(root.join("manifest.js")).unwrap();
        write_text(&staging.join("exam-a.js"), "new-wrapper").unwrap();
        write_text(&staging.join("manifest.js"), "new-manifest").unwrap();

        let error = commit_staged_nas_files(
            &root,
            &staging,
            &["exam-a.js".to_string(), "manifest.js".to_string()],
        )
        .unwrap_err();

        assert!(error.starts_with("nas_direct_publish_target_not_file:"));
        assert_eq!(
            fs::read_to_string(root.join("exam-a.js")).unwrap(),
            "old-wrapper"
        );
        assert!(root.join("manifest.js").is_dir());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nas_export_rejects_local_placeholder_before_creating_output() {
        let root = temp_root();
        let error = export_nas_library_core(
            &root,
            &json!({
                "jobIds": ["unused-job"],
                "exportDir": "LOCAL://exports/nas-library"
            }),
            false,
        )
        .unwrap_err();

        assert_eq!(
            error,
            "nas_export_requires_real_library_root:local_placeholder_not_allowed"
        );
        assert!(!root.exists());
    }
}
