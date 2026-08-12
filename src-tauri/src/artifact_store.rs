use crate::schema::common::canonical_json_bytes;
use crate::util::{safe_job_dir, validate_path_segment};
use crate::{hash_bytes, CommandResult};
use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

/// Version of the on-disk V2 job layout. This is intentionally independent of
/// the schema versions of the JSON documents stored inside it.
pub(crate) const JOB_ARTIFACT_LAYOUT_VERSION: &str = "JobArtifactLayoutV1";
pub(crate) const CURRENT_REVISION_SCHEMA_VERSION: &str = "JobCurrentRevisionV1";
pub(crate) const REVISION_RECORD_SCHEMA_VERSION: &str = "AuthoringRevisionRecordV1";

#[derive(Debug, Clone)]
pub(crate) struct JobArtifactPaths {
    pub(crate) job_dir: PathBuf,
    pub(crate) sources_dir: PathBuf,
    pub(crate) extraction_dir: PathBuf,
    pub(crate) authoring_dir: PathBuf,
    pub(crate) revisions_dir: PathBuf,
    pub(crate) patches_dir: PathBuf,
    pub(crate) assets_dir: PathBuf,
    pub(crate) asset_blobs_dir: PathBuf,
    pub(crate) asset_metadata_dir: PathBuf,
    pub(crate) asset_previews_dir: PathBuf,
    pub(crate) preview_dir: PathBuf,
    pub(crate) preview_runtime_dir: PathBuf,
    pub(crate) export_history_dir: PathBuf,
    pub(crate) legacy_dir: PathBuf,
}

impl JobArtifactPaths {
    pub(crate) fn for_job(root: &Path, job_id: &str) -> CommandResult<Self> {
        let job_dir = safe_job_dir(root, job_id)?;
        let sources_dir = job_dir.join("sources");
        let extraction_dir = job_dir.join("extraction");
        let authoring_dir = job_dir.join("authoring");
        let revisions_dir = authoring_dir.join("revisions");
        let patches_dir = authoring_dir.join("patches");
        let assets_dir = job_dir.join("assets");
        let asset_blobs_dir = assets_dir.join("blobs");
        let asset_metadata_dir = assets_dir.join("metadata");
        let asset_previews_dir = assets_dir.join("previews");
        let preview_dir = job_dir.join("preview");
        let preview_runtime_dir = preview_dir.join("runtime");
        let export_history_dir = job_dir.join("export-history");
        let legacy_dir = job_dir.join("legacy");
        Ok(Self {
            job_dir,
            sources_dir,
            extraction_dir,
            authoring_dir,
            revisions_dir,
            patches_dir,
            assets_dir,
            asset_blobs_dir,
            asset_metadata_dir,
            asset_previews_dir,
            preview_dir,
            preview_runtime_dir,
            export_history_dir,
            legacy_dir,
        })
    }

    pub(crate) fn current_revision_path(&self) -> PathBuf {
        self.authoring_dir.join("current-revision.json")
    }

    pub(crate) fn revision_path(&self, revision: u64) -> PathBuf {
        self.revisions_dir.join(format!("{revision}.json"))
    }

    pub(crate) fn revision_meta_path(&self, revision: u64) -> PathBuf {
        self.revisions_dir.join(format!("{revision}.meta.json"))
    }

    pub(crate) fn patch_path(&self, revision: u64) -> PathBuf {
        self.patches_dir.join(format!("{revision}.jsonl"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RevisionSourceV2 {
    AutoExtract,
    User,
    Migration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtifactReceiptV2 {
    pub relative_path: String,
    pub byte_length: u64,
    pub sha256: String,
    pub canonical_json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CurrentRevisionV2 {
    pub schema_version: String,
    pub layout_version: String,
    pub job_id: String,
    pub revision: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RevisionRecordV2 {
    pub schema_version: String,
    pub layout_version: String,
    pub job_id: String,
    pub revision: u64,
    pub parent_revision: u64,
    pub source: RevisionSourceV2,
    pub created_at: String,
    pub artifact_path: String,
    pub artifact_sha256: String,
    pub patch_path: Option<String>,
    pub patch_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RevisionWriteResultV2 {
    pub current: CurrentRevisionV2,
    pub record: RevisionRecordV2,
    pub artifact: ArtifactReceiptV2,
    pub patch: Option<ArtifactReceiptV2>,
}

pub(crate) fn ensure_job_artifact_layout(
    root: &Path,
    job_id: &str,
) -> CommandResult<JobArtifactPaths> {
    let paths = JobArtifactPaths::for_job(root, job_id)?;
    for path in [
        &paths.sources_dir,
        &paths.extraction_dir,
        &paths.authoring_dir,
        &paths.revisions_dir,
        &paths.patches_dir,
        &paths.assets_dir,
        &paths.asset_blobs_dir,
        &paths.asset_metadata_dir,
        &paths.asset_previews_dir,
        &paths.preview_dir,
        &paths.preview_runtime_dir,
        &paths.export_history_dir,
        &paths.legacy_dir,
    ] {
        fs::create_dir_all(path).map_err(|error| error.to_string())?;
    }
    Ok(paths)
}

pub(crate) fn canonical_json_hash(value: &Value) -> CommandResult<String> {
    let bytes = canonical_json_bytes(value).map_err(|error| error.to_string())?;
    Ok(hash_bytes(&bytes))
}

pub(crate) fn write_canonical_json_atomic(
    path: &Path,
    value: &Value,
) -> CommandResult<ArtifactReceiptV2> {
    let bytes = canonical_json_bytes(value).map_err(|error| error.to_string())?;
    write_bytes_atomic(path, &bytes, true)
}

pub(crate) fn write_artifact_json(
    root: &Path,
    job_id: &str,
    relative_path: &str,
    value: &Value,
) -> CommandResult<ArtifactReceiptV2> {
    let paths = ensure_job_artifact_layout(root, job_id)?;
    let relative = safe_relative_path(relative_path)?;
    validate_v2_namespace(&relative)?;
    let path = paths.job_dir.join(&relative);
    let receipt = write_canonical_json_atomic(&path, value)?;
    Ok(ArtifactReceiptV2 {
        relative_path: relative.to_string_lossy().replace('\\', "/"),
        ..receipt
    })
}

pub(crate) fn read_current_revision(root: &Path, job_id: &str) -> CommandResult<CurrentRevisionV2> {
    let paths = JobArtifactPaths::for_job(root, job_id)?;
    let path = paths.current_revision_path();
    if !path.exists() {
        return Ok(empty_current_revision(job_id));
    }
    let value: Value = crate::util::read_json(&path)?;
    let current: CurrentRevisionV2 = serde_json::from_value(value)
        .map_err(|error| format!("parse_current_revision:{}:{}", path.display(), error))?;
    validate_current_revision(&current, job_id)?;
    Ok(current)
}

pub(crate) fn recover_current_revision(
    root: &Path,
    job_id: &str,
) -> CommandResult<CurrentRevisionV2> {
    let current = read_current_revision(root, job_id)?;
    let records = list_revision_records(root, job_id)?;
    let latest_revision = records.last().map(|record| record.revision).unwrap_or(0);
    if current.revision == latest_revision {
        return Ok(current);
    }
    if current.revision > latest_revision {
        return Err(format!(
            "current_revision_missing_record:current={}:latest={}",
            current.revision, latest_revision
        ));
    }
    let recovered = CurrentRevisionV2 {
        schema_version: CURRENT_REVISION_SCHEMA_VERSION.to_string(),
        layout_version: JOB_ARTIFACT_LAYOUT_VERSION.to_string(),
        job_id: job_id.to_string(),
        revision: latest_revision,
        updated_at: Utc::now().to_rfc3339(),
    };
    write_canonical_json_atomic(
        &JobArtifactPaths::for_job(root, job_id)?.current_revision_path(),
        &serde_json::to_value(&recovered).map_err(|error| error.to_string())?,
    )?;
    Ok(recovered)
}

pub(crate) fn append_revision(
    root: &Path,
    job_id: &str,
    base_revision: u64,
    source: RevisionSourceV2,
    artifact: &Value,
    patches: &[Value],
) -> CommandResult<RevisionWriteResultV2> {
    let paths = ensure_job_artifact_layout(root, job_id)?;
    let lock_path = paths.authoring_dir.join("revision.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("open_revision_lock:{}:{}", lock_path.display(), error))?;
    lock_file
        .lock_exclusive()
        .map_err(|error| format!("lock_revision_store:{}:{}", lock_path.display(), error))?;

    let current = recover_current_revision(root, job_id)?;
    if current.revision != base_revision {
        return Err(format!(
            "revision_conflict:current={}:base={}",
            current.revision, base_revision
        ));
    }

    let revision = max_revision_number(&paths.revisions_dir)?
        .max(current.revision)
        .saturating_add(1);
    let artifact_receipt =
        write_canonical_json_create_new(&paths.revision_path(revision), artifact)?;
    let patch_receipt = if patches.is_empty() {
        None
    } else {
        let mut bytes = Vec::new();
        for patch in patches {
            bytes.extend(canonical_json_bytes(patch).map_err(|error| error.to_string())?);
            bytes.push(b'\n');
        }
        Some(write_bytes_create_new(
            &paths.patch_path(revision),
            &bytes,
            true,
        )?)
    };

    let artifact_path = relative_to_job(&paths.job_dir, &paths.revision_path(revision))?;
    let patch_path = patch_receipt
        .as_ref()
        .map(|_| relative_to_job(&paths.job_dir, &paths.patch_path(revision)))
        .transpose()?;
    let record = RevisionRecordV2 {
        schema_version: REVISION_RECORD_SCHEMA_VERSION.to_string(),
        layout_version: JOB_ARTIFACT_LAYOUT_VERSION.to_string(),
        job_id: job_id.to_string(),
        revision,
        parent_revision: current.revision,
        source,
        created_at: Utc::now().to_rfc3339(),
        artifact_path: artifact_path.clone(),
        artifact_sha256: artifact_receipt.sha256.clone(),
        patch_path: patch_path.clone(),
        patch_sha256: patch_receipt.as_ref().map(|receipt| receipt.sha256.clone()),
    };
    write_canonical_json_create_new(
        &paths.revision_meta_path(revision),
        &serde_json::to_value(&record).map_err(|error| error.to_string())?,
    )?;

    let next_current = CurrentRevisionV2 {
        schema_version: CURRENT_REVISION_SCHEMA_VERSION.to_string(),
        layout_version: JOB_ARTIFACT_LAYOUT_VERSION.to_string(),
        job_id: job_id.to_string(),
        revision,
        updated_at: Utc::now().to_rfc3339(),
    };
    write_canonical_json_atomic(
        &paths.current_revision_path(),
        &serde_json::to_value(&next_current).map_err(|error| error.to_string())?,
    )?;

    Ok(RevisionWriteResultV2 {
        current: next_current,
        record,
        artifact: ArtifactReceiptV2 {
            relative_path: artifact_path,
            ..artifact_receipt
        },
        patch: patch_receipt.map(|receipt| ArtifactReceiptV2 {
            relative_path: patch_path.unwrap_or_default(),
            ..receipt
        }),
    })
}

pub(crate) fn read_revision(root: &Path, job_id: &str, revision: u64) -> CommandResult<Value> {
    if revision == 0 {
        return Err("revision_zero_has_no_artifact".to_string());
    }
    let paths = JobArtifactPaths::for_job(root, job_id)?;
    crate::util::read_json(&paths.revision_path(revision))
}

pub(crate) fn list_revision_records(
    root: &Path,
    job_id: &str,
) -> CommandResult<Vec<RevisionRecordV2>> {
    let paths = JobArtifactPaths::for_job(root, job_id)?;
    if !paths.revisions_dir.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(&paths.revisions_dir).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if path.extension().and_then(|value| value.to_str()) != Some("json")
            || file_name.contains(".meta.")
        {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let meta_path = path.with_file_name(format!("{stem}.meta.json"));
        if !meta_path.exists() {
            continue;
        }
        let value: Value = crate::util::read_json(&meta_path)?;
        let record: RevisionRecordV2 = serde_json::from_value(value)
            .map_err(|error| format!("parse_revision_record:{}:{}", meta_path.display(), error))?;
        validate_revision_record(&record, job_id)?;
        records.push(record);
    }
    records.sort_by_key(|record| record.revision);
    Ok(records)
}

/// Read-only diagnostic view of the V2 layout. It never creates directories
/// and therefore is safe to call while opening a legacy V1 job.
pub(crate) fn inspect_job_artifacts(root: &Path, job_id: &str) -> CommandResult<Value> {
    let paths = JobArtifactPaths::for_job(root, job_id)?;
    let current = read_current_revision(root, job_id)?;
    let revisions = list_revision_records(root, job_id)?;
    Ok(serde_json::json!({
        "schemaVersion": "JobArtifactStatusV1",
        "layoutVersion": JOB_ARTIFACT_LAYOUT_VERSION,
        "jobId": job_id,
        "current": current,
        "revisions": revisions,
        "paths": {
            "sources": relative_to_job(&paths.job_dir, &paths.sources_dir)?,
            "extraction": relative_to_job(&paths.job_dir, &paths.extraction_dir)?,
            "authoring": relative_to_job(&paths.job_dir, &paths.authoring_dir)?,
            "assets": relative_to_job(&paths.job_dir, &paths.assets_dir)?,
            "preview": relative_to_job(&paths.job_dir, &paths.preview_dir)?,
            "exportHistory": relative_to_job(&paths.job_dir, &paths.export_history_dir)?,
            "legacy": relative_to_job(&paths.job_dir, &paths.legacy_dir)?
        },
        "v1FilesRemainReadable": true
    }))
}

fn empty_current_revision(job_id: &str) -> CurrentRevisionV2 {
    CurrentRevisionV2 {
        schema_version: CURRENT_REVISION_SCHEMA_VERSION.to_string(),
        layout_version: JOB_ARTIFACT_LAYOUT_VERSION.to_string(),
        job_id: job_id.to_string(),
        revision: 0,
        updated_at: Utc::now().to_rfc3339(),
    }
}

fn validate_current_revision(current: &CurrentRevisionV2, job_id: &str) -> CommandResult<()> {
    if current.schema_version != CURRENT_REVISION_SCHEMA_VERSION
        || current.layout_version != JOB_ARTIFACT_LAYOUT_VERSION
        || current.job_id != job_id
    {
        return Err(format!(
            "unsupported_current_revision_metadata:job={job_id}:schema={}:layout={}:recordJob={}",
            current.schema_version, current.layout_version, current.job_id
        ));
    }
    Ok(())
}

fn validate_revision_record(record: &RevisionRecordV2, job_id: &str) -> CommandResult<()> {
    if record.schema_version != REVISION_RECORD_SCHEMA_VERSION
        || record.layout_version != JOB_ARTIFACT_LAYOUT_VERSION
        || record.job_id != job_id
        || record.revision == 0
        || record.parent_revision >= record.revision
    {
        return Err(format!(
            "unsupported_revision_record:job={job_id}:revision={}",
            record.revision
        ));
    }
    Ok(())
}

fn safe_relative_path(relative_path: &str) -> CommandResult<PathBuf> {
    if relative_path.trim().is_empty() {
        return Err("artifact_relative_path_required".to_string());
    }
    let path = PathBuf::from(relative_path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe_artifact_relative_path:{relative_path}"));
    }
    for component in path.components() {
        if let Component::Normal(value) = component {
            let value = value.to_string_lossy();
            validate_path_segment("artifact_path", &value)?;
        }
    }
    Ok(path)
}

fn validate_v2_namespace(path: &Path) -> CommandResult<()> {
    let namespace = path.components().next().and_then(|component| {
        if let Component::Normal(value) = component {
            Some(value.to_string_lossy())
        } else {
            None
        }
    });
    if matches!(
        namespace.as_deref(),
        Some(
            "sources"
                | "extraction"
                | "authoring"
                | "assets"
                | "preview"
                | "export-history"
                | "legacy"
        )
    ) {
        Ok(())
    } else {
        Err(format!(
            "artifact_path_not_in_v2_namespace:{}",
            path.display()
        ))
    }
}

fn relative_to_job(job_dir: &Path, path: &Path) -> CommandResult<String> {
    path.strip_prefix(job_dir)
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .map_err(|_| format!("artifact_path_outside_job_dir:{}", path.display()))
}

fn write_bytes_atomic(
    path: &Path,
    bytes: &[u8],
    canonical_json: bool,
) -> CommandResult<ArtifactReceiptV2> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("artifact_parent_missing:{}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("artifact_file_name_missing:{}", path.display()))?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", Uuid::new_v4().simple()));
    let result = (|| -> CommandResult<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("create_artifact_temp:{}:{}", temporary.display(), error))?;
        file.write_all(bytes)
            .map_err(|error| format!("write_artifact_temp:{}:{}", temporary.display(), error))?;
        file.flush()
            .map_err(|error| format!("flush_artifact_temp:{}:{}", temporary.display(), error))?;
        file.sync_all()
            .map_err(|error| format!("sync_artifact_temp:{}:{}", temporary.display(), error))?;
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(ArtifactReceiptV2 {
        relative_path: path.to_string_lossy().replace('\\', "/"),
        byte_length: bytes.len() as u64,
        sha256: hash_bytes(bytes),
        canonical_json,
    })
}

fn write_canonical_json_create_new(path: &Path, value: &Value) -> CommandResult<ArtifactReceiptV2> {
    let bytes = canonical_json_bytes(value).map_err(|error| error.to_string())?;
    write_bytes_create_new(path, &bytes, true)
}

fn write_bytes_create_new(
    path: &Path,
    bytes: &[u8],
    canonical_json: bool,
) -> CommandResult<ArtifactReceiptV2> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("artifact_parent_missing:{}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("create_immutable_artifact:{}:{}", path.display(), error))?;
    file.write_all(bytes)
        .map_err(|error| format!("write_immutable_artifact:{}:{}", path.display(), error))?;
    file.flush()
        .map_err(|error| format!("flush_immutable_artifact:{}:{}", path.display(), error))?;
    file.sync_all()
        .map_err(|error| format!("sync_immutable_artifact:{}:{}", path.display(), error))?;
    Ok(ArtifactReceiptV2 {
        relative_path: path.to_string_lossy().replace('\\', "/"),
        byte_length: bytes.len() as u64,
        sha256: hash_bytes(bytes),
        canonical_json,
    })
}

fn max_revision_number(revisions_dir: &Path) -> CommandResult<u64> {
    if !revisions_dir.exists() {
        return Ok(0);
    }
    let mut maximum = 0;
    for entry in fs::read_dir(revisions_dir).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let revision_text = file_name
            .strip_suffix(".meta.json")
            .or_else(|| file_name.strip_suffix(".json"));
        if let Some(revision) = revision_text.and_then(|value| value.parse::<u64>().ok()) {
            maximum = maximum.max(revision);
        }
    }
    Ok(maximum)
}

fn replace_file(temporary: &Path, target: &Path) -> CommandResult<()> {
    if !target.exists() {
        return fs::rename(temporary, target).map_err(|error| {
            format!(
                "rename_artifact_temp:{}->{}:{}",
                temporary.display(),
                target.display(),
                error
            )
        });
    }

    let backup = target.with_file_name(format!(
        ".{}.bak-{}",
        target
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("artifact"),
        Uuid::new_v4().simple()
    ));
    fs::rename(target, &backup).map_err(|error| {
        format!(
            "backup_artifact:{}->{}:{}",
            target.display(),
            backup.display(),
            error
        )
    })?;
    match fs::rename(temporary, target) {
        Ok(()) => {
            let _ = fs::remove_file(backup);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup, target);
            Err(format!(
                "replace_artifact:{}->{}:{}",
                temporary.display(),
                target.display(),
                error
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        env, fs,
        sync::{Arc, Barrier},
        thread,
    };

    fn temp_root() -> PathBuf {
        env::temp_dir().join(format!("phase1-artifact-{}", Uuid::new_v4().simple()))
    }

    #[test]
    fn layout_is_explicit_and_does_not_replace_v1_files() {
        let root = temp_root();
        let job_id = "job-layout";
        let paths = ensure_job_artifact_layout(&root, job_id).unwrap();
        fs::create_dir_all(&paths.job_dir).unwrap();
        fs::write(
            paths.job_dir.join("document-ir.json"),
            br#"{"schemaVersion":"DocumentIRV1"}"#,
        )
        .unwrap();

        assert!(paths.extraction_dir.is_dir());
        assert!(paths.sources_dir.is_dir());
        assert!(paths.revisions_dir.is_dir());
        assert!(paths.asset_blobs_dir.is_dir());
        assert_eq!(
            fs::read_to_string(paths.job_dir.join("document-ir.json")).unwrap(),
            r#"{"schemaVersion":"DocumentIRV1"}"#
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn canonical_artifact_write_is_stable_and_hashable() {
        let root = temp_root();
        let value = json!({"z":1,"a":{"y":2,"b":3},"items":[2,1]});
        let first =
            write_artifact_json(&root, "job-canonical", "extraction/sample.json", &value).unwrap();
        let second = write_artifact_json(
            &root,
            "job-canonical",
            "extraction/sample.json",
            &json!({"items":[2,1],"a":{"b":3,"y":2},"z":1}),
        )
        .unwrap();
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(first.byte_length, second.byte_length);
        assert_eq!(
            fs::read_to_string(root.join("jobs/job-canonical/extraction/sample.json")).unwrap(),
            r#"{"a":{"b":3,"y":2},"items":[2,1],"z":1}"#
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn revisions_are_optimistic_and_recoverable() {
        let root = temp_root();
        let first = append_revision(
            &root,
            "job-revision",
            0,
            RevisionSourceV2::AutoExtract,
            &json!({"schemaVersion":"IeltsAuthoringIRV2","value":"first"}),
            &[json!({"op":"add","path":"/value","value":"first"})],
        )
        .unwrap();
        assert_eq!(first.current.revision, 1);
        assert!(append_revision(
            &root,
            "job-revision",
            0,
            RevisionSourceV2::User,
            &json!({"schemaVersion":"IeltsAuthoringIRV2","value":"stale"}),
            &[],
        )
        .is_err());
        let second = append_revision(
            &root,
            "job-revision",
            1,
            RevisionSourceV2::User,
            &json!({"schemaVersion":"IeltsAuthoringIRV2","value":"second"}),
            &[],
        )
        .unwrap();
        assert_eq!(second.record.parent_revision, 1);
        assert_eq!(
            read_revision(&root, "job-revision", 2).unwrap()["value"],
            "second"
        );
        assert_eq!(
            list_revision_records(&root, "job-revision").unwrap().len(),
            2
        );

        let pointer = JobArtifactPaths::for_job(&root, "job-revision")
            .unwrap()
            .current_revision_path();
        fs::remove_file(pointer).unwrap();
        assert_eq!(
            recover_current_revision(&root, "job-revision")
                .unwrap()
                .revision,
            2
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn append_with_missing_pointer_never_overwrites_revision_one() {
        let root = temp_root();
        append_revision(
            &root,
            "job-missing-pointer",
            0,
            RevisionSourceV2::AutoExtract,
            &json!({"value":"original"}),
            &[],
        )
        .unwrap();
        let paths = JobArtifactPaths::for_job(&root, "job-missing-pointer").unwrap();
        fs::remove_file(paths.current_revision_path()).unwrap();

        let error = append_revision(
            &root,
            "job-missing-pointer",
            0,
            RevisionSourceV2::User,
            &json!({"value":"replacement"}),
            &[],
        )
        .unwrap_err();
        assert_eq!(error, "revision_conflict:current=1:base=0");
        assert_eq!(
            read_revision(&root, "job-missing-pointer", 1).unwrap()["value"],
            "original"
        );
        assert_eq!(
            read_current_revision(&root, "job-missing-pointer")
                .unwrap()
                .revision,
            1
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_base_zero_writers_cannot_replace_each_other() {
        let root = Arc::new(temp_root());
        let barrier = Arc::new(Barrier::new(2));
        let handles = ["first", "second"].map(|value| {
            let root = Arc::clone(&root);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                append_revision(
                    root.as_ref(),
                    "job-concurrent",
                    0,
                    RevisionSourceV2::User,
                    &json!({"value":value}),
                    &[],
                )
            })
        });
        let results = handles.map(|handle| handle.join().unwrap());
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        assert_eq!(
            list_revision_records(root.as_ref(), "job-concurrent")
                .unwrap()
                .len(),
            1
        );
        assert!(matches!(
            read_revision(root.as_ref(), "job-concurrent", 1).unwrap()["value"].as_str(),
            Some("first" | "second")
        ));
        let _ = fs::remove_dir_all(root.as_ref());
    }

    #[test]
    fn incomplete_revision_number_is_quarantined_by_monotonic_allocation() {
        let root = temp_root();
        let paths = ensure_job_artifact_layout(&root, "job-half-write").unwrap();
        fs::write(paths.revision_path(1), br#"{"value":"partial"}"#).unwrap();
        let result = append_revision(
            &root,
            "job-half-write",
            0,
            RevisionSourceV2::AutoExtract,
            &json!({"value":"complete"}),
            &[],
        )
        .unwrap();
        assert_eq!(result.current.revision, 2);
        assert_eq!(
            read_revision(&root, "job-half-write", 1).unwrap()["value"],
            "partial"
        );
        assert_eq!(
            read_revision(&root, "job-half-write", 2).unwrap()["value"],
            "complete"
        );
        assert_eq!(
            list_revision_records(&root, "job-half-write")
                .unwrap()
                .len(),
            1
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn artifact_relative_paths_cannot_escape_job() {
        let root = temp_root();
        assert!(write_artifact_json(&root, "job-safe", "../outside.json", &json!({})).is_err());
        assert!(write_artifact_json(&root, "job-safe", "C:/outside.json", &json!({})).is_err());
        assert!(write_artifact_json(&root, "job-safe", "document-ir.json", &json!({})).is_err());
        assert!(write_artifact_json(&root, "job-safe", "uploads/source.pdf", &json!({})).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn status_is_read_only_for_a_legacy_job() {
        let root = temp_root();
        let dir = root.join("jobs").join("legacy-status");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("document-ir.json"),
            br#"{"schemaVersion":"DocumentIRV1"}"#,
        )
        .unwrap();
        let status = inspect_job_artifacts(&root, "legacy-status").unwrap();
        assert_eq!(status["schemaVersion"], "JobArtifactStatusV1");
        assert_eq!(status["current"]["revision"], 0);
        assert!(status["v1FilesRemainReadable"].as_bool().unwrap());
        assert!(!dir.join("authoring").exists());
        let _ = fs::remove_dir_all(root);
    }
}
