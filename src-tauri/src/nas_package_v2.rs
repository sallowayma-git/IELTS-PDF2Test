//! Phase 6 NAS V2 package builder and two-phase publisher.
//!
//! The V1 exporter remains untouched.  This module is an opt-in publisher for
//! a single `ReadingExamSourceV2` and deliberately commits the discovery
//! manifest last, after the staged package has passed the same probe used by
//! the runtime validator.

use crate::export_artifacts::{build_wrapper, safe_exam_id};
use crate::export_nas_library::{nas_reading_exams_dir, normalize_nas_library_root};
use crate::reading_runtime_v2::{
    run_student_loader_probe, safe_join_asset_path, ExamAssetManifestV2, StudentProbeReportV2,
};
use crate::reading_source_v2::{validate_reading_source_v2, ReadingExamSourceV2};
use crate::schema::common::canonical_json_bytes;
use crate::CommandResult;
use chrono::Utc;
use fs2::FileExt;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const ASSET_MANIFEST_FILE_NAME: &str = "asset-manifest.json";
const CONTROL_DIR_NAME: &str = ".publish-control";
const REPORT_DIR_NAME: &str = "publish/reports";
const BACKUP_DIR_NAME: &str = "backups";
const CURRENT_STUDENT_RUNTIME_VERSION: [u64; 3] = [0, 2, 0];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NasPackagePublishInput {
    #[serde(alias = "exportDir")]
    pub library_root: String,
    #[serde(alias = "runtimePath", alias = "sourceFile")]
    pub source_path: String,
    pub asset_root: Option<String>,
    pub exam_id: Option<String>,
    pub minimum_runtime_version: Option<String>,
    pub expected_manifest_sha256: Option<String>,
    pub fault: Option<String>,
}

#[derive(Debug, Clone)]
struct PackagePaths {
    library_root: PathBuf,
    reading_root: PathBuf,
    manifest_path: PathBuf,
    exam_path: PathBuf,
    resource_path: PathBuf,
    staging_root: PathBuf,
    staging_exam_path: PathBuf,
    staging_resource_path: PathBuf,
    staging_manifest_path: PathBuf,
    backup_root: PathBuf,
    journal_path: PathBuf,
    report_path: PathBuf,
    lock_path: PathBuf,
    lock_metadata_path: PathBuf,
    base_manifest_sha256: String,
}

#[derive(Debug, Clone)]
struct PackageReceipt {
    exam_id: String,
    runtime_sha256: String,
    asset_manifest_sha256: String,
    asset_count: usize,
    probe: StudentProbeReportV2,
    manifest_sha256: String,
}

pub(crate) fn publish_nas_package_v2_core(root: &Path, input: Value) -> CommandResult<Value> {
    let input: NasPackagePublishInput = serde_json::from_value(input)
        .map_err(|error| format!("nas_package_v2_invalid_request:{error}"))?;
    // The public command accepts either the NAS parent directory or its
    // `publish/` child (the legacy exporter accepts both).  Normalize before
    // deriving *any* transaction path so both spellings share one lock,
    // manifest/CAS target, journal and backup namespace.
    let library_root =
        normalize_nas_library_root(&absolute_path("library_root", &input.library_root)?);
    let source_path = absolute_path("source_path", &input.source_path)?;
    let source_bytes = fs::read(&source_path).map_err(|error| {
        format!(
            "nas_package_v2_source_read:{}:{error}",
            source_path.display()
        )
    })?;
    let source_value: Value = serde_json::from_slice(&source_bytes).map_err(|error| {
        format!(
            "nas_package_v2_source_json:{}:{error}",
            source_path.display()
        )
    })?;
    let source: ReadingExamSourceV2 = serde_json::from_value(source_value.clone())
        .map_err(|error| format!("nas_package_v2_source_contract:{error}"))?;
    let issues = validate_reading_source_v2(&source);
    if !issues.is_empty() {
        return Err(format!(
            "nas_package_v2_source_invalid:{}",
            serde_json::to_string(&issues).unwrap_or_default()
        ));
    }
    let safe_source_exam_id = safe_exam_id(&source_value)?;
    if input
        .exam_id
        .as_deref()
        .is_some_and(|exam_id| exam_id != safe_source_exam_id)
    {
        return Err("nas_package_v2_exam_id_mismatch".to_string());
    }

    let reading_root = nas_reading_exams_dir(&library_root);
    fs::create_dir_all(&reading_root).map_err(|error| error.to_string())?;
    let mut paths = make_paths(&library_root, &reading_root, &safe_source_exam_id)?;
    fs::create_dir_all(&paths.library_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&paths.reading_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(paths.lock_path.parent().unwrap_or(&paths.reading_root))
        .map_err(|error| error.to_string())?;

    let export_id = Uuid::new_v4().simple().to_string();
    paths.staging_root = paths
        .reading_root
        .join(format!(".phase6-staging-{export_id}"));
    paths.staging_exam_path = paths.staging_root.join(format!("{safe_source_exam_id}.js"));
    paths.staging_resource_path = paths
        .staging_root
        .join("resources")
        .join(&safe_source_exam_id);
    paths.staging_manifest_path = paths.staging_root.join("manifest.js");
    paths.backup_root = paths
        .library_root
        .join(CONTROL_DIR_NAME)
        .join(BACKUP_DIR_NAME)
        .join(format!("{safe_source_exam_id}-{export_id}"));
    paths.journal_path = paths
        .library_root
        .join(CONTROL_DIR_NAME)
        .join(format!("{safe_source_exam_id}-{export_id}.journal.json"));
    paths.report_path = paths
        .library_root
        .join(REPORT_DIR_NAME)
        .join(format!("{safe_source_exam_id}-{export_id}.json"));

    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&paths.lock_path)
        .map_err(|error| format!("nas_package_v2_lock_open:{error}"))?;
    lock.try_lock_exclusive()
        .map_err(|error| format!("nas_package_v2_lock_busy:{error}"))?;

    write_lock_metadata(&paths, export_id.as_str())?;
    recover_incomplete_transactions(&paths)?;
    paths.base_manifest_sha256 = manifest_sha256(&paths.manifest_path)?;
    if let Some(expected) = input.expected_manifest_sha256.as_deref() {
        verify_manifest_compare_and_swap(&paths.manifest_path, expected)?;
    }
    write_journal(&paths, &source, "staging", None, export_id.as_str())?;
    let result = stage_and_commit(
        root,
        &input,
        &source,
        &source_value,
        &source_path,
        &paths,
        export_id.as_str(),
    );
    if let Err(error) = &result {
        let _ = write_journal(&paths, &source, "failed", Some(error), export_id.as_str());
        let _ = fs::remove_dir_all(&paths.staging_root);
    }
    let _ = fs::remove_file(&paths.lock_metadata_path);
    lock.unlock().ok();
    result
}

fn stage_and_commit(
    _app_root: &Path,
    input: &NasPackagePublishInput,
    source: &ReadingExamSourceV2,
    source_value: &Value,
    source_path: &Path,
    paths: &PackagePaths,
    export_id: &str,
) -> CommandResult<Value> {
    fs::create_dir_all(&paths.staging_resource_path)
        .map_err(|error| format!("nas_package_v2_staging_create:{error}"))?;
    let asset_root = input
        .asset_root
        .as_deref()
        .map(|value| absolute_path("asset_root", value))
        .transpose()?;
    let asset_root = asset_root.unwrap_or_else(|| {
        source_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });

    let mut manifest_assets = BTreeMap::new();
    let mut seen_destinations = BTreeSet::new();
    for descriptor in &source.assets.assets {
        validate_package_relative_path(&descriptor.relative_path)?;
        let source_asset =
            safe_join_asset_path(&asset_root, &descriptor.relative_path).map_err(|error| {
                format!(
                    "nas_package_v2_asset_source:{}:{error}",
                    descriptor.asset_id
                )
            })?;
        let destination_relative =
            format!("resources/{}/{}", source.exam_id, descriptor.relative_path);
        let destination = paths.staging_root.join(&destination_relative);
        let collision_key = destination_relative.to_ascii_lowercase();
        if !seen_destinations.insert(collision_key) {
            return Err(format!(
                "nas_package_v2_asset_path_collision:{}",
                descriptor.relative_path
            ));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::copy(&source_asset, &destination).map_err(|error| {
            format!("nas_package_v2_asset_copy:{}:{error}", descriptor.asset_id)
        })?;
        manifest_assets.insert(descriptor.asset_id.clone(), descriptor.clone());
    }

    let generated_at = Utc::now().to_rfc3339();
    let asset_manifest = ExamAssetManifestV2 {
        schema_version: "ExamAssetManifestV2".to_string(),
        exam_id: source.exam_id.clone(),
        generated_at,
        assets: manifest_assets,
    };
    let asset_manifest_value =
        serde_json::to_value(&asset_manifest).map_err(|error| error.to_string())?;
    let asset_manifest_bytes =
        canonical_json_bytes(&asset_manifest_value).map_err(|error| error.to_string())?;
    let asset_manifest_path = paths.staging_resource_path.join(ASSET_MANIFEST_FILE_NAME);
    write_synced_file(&asset_manifest_path, &asset_manifest_bytes)?;

    let runtime_value = serde_json::to_value(source).map_err(|error| error.to_string())?;
    let runtime_bytes = canonical_json_bytes(&runtime_value).map_err(|error| error.to_string())?;
    let wrapper = build_wrapper(source_value)?;
    write_synced_file(&paths.staging_exam_path, wrapper.as_bytes())?;
    if input.fault.as_deref() == Some("after_assets") {
        return Err("nas_package_v2_fault_after_assets".to_string());
    }
    if input.fault.as_deref() == Some("after_source") {
        return Err("nas_package_v2_fault_after_source".to_string());
    }

    update_lock_heartbeat(paths)?;
    let probe = run_student_loader_probe(source, &asset_manifest, &paths.staging_resource_path);
    if !probe.passed {
        return Err(format!(
            "nas_package_v2_probe_failed:{}",
            serde_json::to_string(&probe).unwrap_or_default()
        ));
    }

    let runtime_sha256 = sha256_hex(&runtime_bytes);
    let asset_manifest_sha256 = sha256_hex(&asset_manifest_bytes);
    let script_sha256 = sha256_hex(wrapper.as_bytes());
    verify_manifest_compare_and_swap(&paths.manifest_path, &paths.base_manifest_sha256)?;
    let mut manifest = load_existing_manifest(&paths.manifest_path)?;
    let minimum_runtime_version = input
        .minimum_runtime_version
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("0.2.0");
    validate_minimum_runtime_version(minimum_runtime_version)?;
    manifest.insert(
        source.exam_id.clone(),
        json!({
            "examId": source.exam_id,
            "dataKey": source.exam_id,
            "script": format!("./{}.js", source.exam_id),
            "title": source.meta.title,
            "category": source.meta.category,
            "schemaVersion": "ReadingExamSourceV2",
            "modality": "reading",
            "minimumRuntimeVersion": minimum_runtime_version,
            "resourcesBase": format!("./resources/{}/", source.exam_id),
            "assetManifest": format!("./resources/{}/{}", source.exam_id, ASSET_MANIFEST_FILE_NAME),
            "checksums": {
                "scriptSha256": script_sha256,
                "assetManifestSha256": asset_manifest_sha256,
                "runtimeSha256": runtime_sha256
            }
        }),
    );
    let mut metadata = manifest
        .remove("_meta")
        .unwrap_or_else(|| json!({"schemaVersion": "ReadingExamManifestV1"}));
    if let Some(meta) = metadata.as_object_mut() {
        meta.insert("schemaVersion".to_string(), json!("ReadingExamManifestV2"));
        meta.insert("assetCount".to_string(), json!(manifest.len()));
        meta.insert("generatedAt".to_string(), json!(Utc::now().to_rfc3339()));
    }
    manifest.insert("_meta".to_string(), metadata);
    let manifest_value = Value::Object(manifest);
    let candidate_manifest = format!(
        "window.__READING_EXAM_MANIFEST__ = {};\n",
        serde_json::to_string_pretty(&manifest_value).map_err(|error| error.to_string())?
    );
    write_synced_file(&paths.staging_manifest_path, candidate_manifest.as_bytes())?;
    if input.fault.as_deref() == Some("before_manifest") {
        return Err("nas_package_v2_fault_before_manifest".to_string());
    }

    write_journal(paths, source, "committing", None, export_id)?;
    let receipt = commit_package(
        paths,
        source,
        &probe,
        &runtime_sha256,
        export_id,
        input.fault.as_deref(),
    )?;
    let report = json!({
        "schemaVersion": "NasPackagePublishReportV2",
        "status": "committed",
        "examId": receipt.exam_id,
        "runtimeVersion": minimum_runtime_version,
        "runtimeSha256": receipt.runtime_sha256,
        "assetManifestSha256": receipt.asset_manifest_sha256,
        "manifestSha256": receipt.manifest_sha256,
        "assetCount": receipt.asset_count,
        "checkedAssetIds": receipt.probe.checked_asset_ids,
        "referencedAssetIds": receipt.probe.referenced_asset_ids,
        "probe": receipt.probe,
        "rollback": {"performed": false, "reason": null},
        "exportId": export_id,
        "writtenAt": Utc::now().to_rfc3339()
    });
    if input.fault.as_deref() == Some("report_write") {
        return Err(recover_post_commit_metadata_failure(
            paths,
            "nas_package_v2_fault_report_write",
        ));
    }
    if let Some(parent) = paths.report_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return Err(recover_post_commit_metadata_failure(
                paths,
                &format!("nas_package_v2_report_dir_create:{error}"),
            ));
        }
    }
    let report_bytes = match canonical_json_bytes(&report) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(recover_post_commit_metadata_failure(
                paths,
                &format!("nas_package_v2_report_encode:{error}"),
            ));
        }
    };
    if let Err(error) = write_synced_file(&paths.report_path, &report_bytes) {
        return Err(recover_post_commit_metadata_failure(
            paths,
            &format!("nas_package_v2_report_write:{error}"),
        ));
    }
    if input.fault.as_deref() == Some("committed_journal") {
        return Err(recover_post_commit_metadata_failure(
            paths,
            "nas_package_v2_fault_committed_journal",
        ));
    }
    if let Err(error) = write_journal(paths, source, "committed", None, export_id) {
        return Err(recover_post_commit_metadata_failure(
            paths,
            &format!("nas_package_v2_committed_journal:{error}"),
        ));
    }
    cleanup_transaction_artifacts(paths);
    Ok(json!({
        "schemaVersion": "NasPackagePublishReportV2",
        "status": "committed",
        "examId": source.exam_id,
        "manifestPath": paths.manifest_path,
        "reportPath": paths.report_path,
        "probe": receipt.probe,
        "assetCount": receipt.asset_count,
        "exportId": export_id
    }))
}

/// A package commit has already moved the runtime files into their final
/// locations by the time report/journal metadata is written.  Never surface a
/// plain error here: immediately replay the durable journal/backup recovery so
/// the caller receives either a fully rolled-back failure or an explicit
/// rollback-incomplete error that will be retried on the next publish/startup.
fn recover_post_commit_metadata_failure(paths: &PackagePaths, error: &str) -> String {
    match recover_incomplete_transactions(paths) {
        Ok(()) => format!("{error};rollback=complete"),
        Err(recovery_error) => {
            format!("{error};rollback=incomplete:{recovery_error}")
        }
    }
}

fn commit_package(
    paths: &PackagePaths,
    source: &ReadingExamSourceV2,
    probe: &StudentProbeReportV2,
    runtime_sha256: &str,
    export_id: &str,
    fault: Option<&str>,
) -> CommandResult<PackageReceipt> {
    if paths.exam_path.exists() && !paths.exam_path.is_file() {
        return Err("nas_package_v2_existing_exam_not_file".to_string());
    }
    if paths.resource_path.exists() && !paths.resource_path.is_dir() {
        return Err("nas_package_v2_existing_resources_not_directory".to_string());
    }
    if paths.manifest_path.exists() && !paths.manifest_path.is_file() {
        return Err("nas_package_v2_existing_manifest_not_file".to_string());
    }
    let backup_root = paths.backup_root.clone();
    fs::create_dir_all(&backup_root).map_err(|error| error.to_string())?;
    let backup_exam = backup_root.join("exam.js");
    let backup_resources = backup_root.join("resources");
    let backup_manifest = backup_root.join("manifest.js");
    let backup_report = backup_root.join("report.json");
    let had_exam = paths.exam_path.is_file();
    let had_resources = paths.resource_path.exists();
    let had_manifest = paths.manifest_path.is_file();
    if paths.report_path.exists() && !paths.report_path.is_file() {
        return Err("nas_package_v2_existing_report_not_file".to_string());
    }
    let had_report = paths.report_path.is_file();
    let state = json!({
        "schemaVersion": "NasBackupStateV1",
        "hadExam": had_exam,
        "hadResources": had_resources,
        "hadManifest": had_manifest,
        "hadReport": had_report
    });
    write_synced_file(
        &backup_root.join("state.json"),
        &canonical_json_bytes(&state).map_err(|error| error.to_string())?,
    )?;

    // Keep the discovery manifest at its live path until the new staged
    // manifest is ready to replace it.  Moving the old manifest to the
    // backup directory here creates a window in which a student loader sees
    // no manifest at all.  A verified copy gives recovery a durable snapshot
    // without changing the old package's visible surface.
    if had_manifest {
        if let Err(error) = copy_file_verified(&paths.manifest_path, &backup_manifest) {
            cleanup_transaction_artifacts(paths);
            return Err(format!(
                "nas_package_v2_backup_manifest:{error};rollback=complete"
            ));
        }
    }
    if had_exam {
        if let Err(error) = fs::rename(&paths.exam_path, &backup_exam) {
            cleanup_transaction_artifacts(paths);
            return Err(format!(
                "nas_package_v2_backup_exam:{error};rollback=complete"
            ));
        }
    }
    // A failure while moving the old resource directory can happen after the
    // old exam has already been moved.  Restore that missing live entry before
    // returning so the still-live manifest never points at a half-backed-up
    // package.  The durable journal/backup remains available only when this
    // best-effort immediate restore itself fails.
    if fault == Some("backup_resources") {
        let rollback_errors = restore_partial_backup_before_commit(
            paths,
            &backup_exam,
            &backup_resources,
            source.exam_id.as_str(),
            had_exam,
            had_resources,
        );
        return Err(if rollback_errors.is_empty() {
            cleanup_transaction_artifacts(paths);
            "nas_package_v2_fault_backup_resources;rollback=complete".to_string()
        } else {
            format!(
                "nas_package_v2_fault_backup_resources;rollback={}",
                rollback_errors.join("|")
            )
        });
    }
    if had_resources {
        if let Err(error) = fs::create_dir_all(&backup_resources) {
            let rollback_errors = restore_partial_backup_before_commit(
                paths,
                &backup_exam,
                &backup_resources,
                source.exam_id.as_str(),
                had_exam,
                had_resources,
            );
            return Err(if rollback_errors.is_empty() {
                cleanup_transaction_artifacts(paths);
                format!("nas_package_v2_backup_resources:{error};rollback=complete")
            } else {
                format!(
                    "nas_package_v2_backup_resources:{error};rollback={}",
                    rollback_errors.join("|")
                )
            });
        }
        if let Err(error) = fs::rename(
            &paths.resource_path,
            backup_resources.join(source.exam_id.as_str()),
        ) {
            let rollback_errors = restore_partial_backup_before_commit(
                paths,
                &backup_exam,
                &backup_resources,
                source.exam_id.as_str(),
                had_exam,
                had_resources,
            );
            return Err(if rollback_errors.is_empty() {
                cleanup_transaction_artifacts(paths);
                format!("nas_package_v2_backup_resources:{error};rollback=complete")
            } else {
                format!(
                    "nas_package_v2_backup_resources:{error};rollback={}",
                    rollback_errors.join("|")
                )
            });
        }
    }
    let rollback = |manifest_committed: bool| -> Vec<String> {
        let mut errors = Vec::new();
        // Do not remove any currently visible package file until every old
        // item that was declared present has a durable backup.  A process or
        // NAS interruption can occur between the individual backup renames;
        // deleting the remaining visible file in that state would turn a
        // recoverable interruption into permanent data loss.
        if let Err(error) = validate_backup_contents(
            &backup_exam,
            &backup_resources,
            &backup_manifest,
            &backup_report,
            source.exam_id.as_str(),
            had_exam,
            had_resources,
            had_manifest,
            had_report && backup_report.exists(),
        ) {
            // If an earlier backup rename already removed a path, restoring
            // that missing path is safe and does not overwrite any visible
            // data.  Leave all other visible paths untouched and retain the
            // transaction artifacts for the next recovery attempt.
            if had_exam && !paths.exam_path.exists() && backup_exam.is_file() {
                if let Err(restore_error) = fs::rename(&backup_exam, &paths.exam_path) {
                    errors.push(format!("restore_exam_before_incomplete:{restore_error}"));
                }
            }
            if had_resources
                && !paths.resource_path.exists()
                && backup_resources.join(source.exam_id.as_str()).is_dir()
            {
                if let Err(restore_error) = fs::rename(
                    backup_resources.join(source.exam_id.as_str()),
                    &paths.resource_path,
                ) {
                    errors.push(format!(
                        "restore_resources_before_incomplete:{restore_error}"
                    ));
                }
            }
            errors.push(format!("backup_incomplete:{error}"));
            return errors;
        }
        if manifest_committed && paths.manifest_path.exists() {
            if let Err(error) = fs::remove_file(&paths.manifest_path) {
                errors.push(format!("remove_manifest:{error}"));
            }
        }
        if paths.exam_path.exists() {
            if let Err(error) = fs::remove_file(&paths.exam_path) {
                errors.push(format!("remove_exam:{error}"));
            }
        }
        if paths.resource_path.exists() {
            if let Err(error) = fs::remove_dir_all(&paths.resource_path) {
                errors.push(format!("remove_resources:{error}"));
            }
        }
        if had_exam {
            if let Err(error) = fs::rename(&backup_exam, &paths.exam_path) {
                errors.push(format!("restore_exam:{error}"));
            }
        }
        if had_resources {
            let backup_path = backup_resources.join(source.exam_id.as_str());
            if let Err(error) = fs::rename(backup_path, &paths.resource_path) {
                errors.push(format!("restore_resources:{error}"));
            }
        }
        if had_manifest && backup_manifest.exists() {
            // Before the final manifest replacement the old manifest is
            // still live, so there is nothing to restore.  Once replacement
            // has happened, use the same atomic replacement primitive used
            // for commit; this works on Unix and on Windows where a plain
            // rename cannot replace an existing file.
            if manifest_committed || !paths.manifest_path.exists() {
                if let Err(error) = atomic_replace_file(&backup_manifest, &paths.manifest_path) {
                    errors.push(format!("restore_manifest:{error}"));
                }
            }
        }
        if had_report && backup_report.exists() {
            if let Err(error) = fs::rename(&backup_report, &paths.report_path) {
                errors.push(format!("restore_report:{error}"));
            }
        }
        errors
    };

    if had_report {
        if let Err(error) = fs::rename(&paths.report_path, &backup_report) {
            let rollback_errors = rollback(false);
            return Err(if rollback_errors.is_empty() {
                cleanup_transaction_artifacts(paths);
                format!("nas_package_v2_backup_report:{error};rollback=complete")
            } else {
                format!(
                    "nas_package_v2_backup_report:{error};rollback={}",
                    rollback_errors.join("|")
                )
            });
        }
    }
    let mut manifest_committed = false;
    let move_result: CommandResult<()> = (|| {
        fs::rename(&paths.staging_exam_path, &paths.exam_path)
            .map_err(|error| format!("nas_package_v2_commit_exam:{error}"))?;
        if let Some(parent) = paths.resource_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::rename(&paths.staging_resource_path, &paths.resource_path)
            .map_err(|error| format!("nas_package_v2_commit_resources:{error}"))?;
        if fault == Some("manifest_rename") {
            return Err("nas_package_v2_fault_manifest_rename".to_string());
        }
        atomic_replace_file(&paths.staging_manifest_path, &paths.manifest_path)
            .map_err(|error| format!("nas_package_v2_commit_manifest:{error}"))?;
        manifest_committed = true;
        write_journal(paths, source, "manifest_committed", None, export_id)?;
        Ok(())
    })();
    if let Err(error) = move_result {
        let rollback_errors = rollback(manifest_committed);
        return Err(if rollback_errors.is_empty() {
            cleanup_transaction_artifacts(paths);
            format!("{error};rollback=complete")
        } else {
            format!("{error};rollback={}", rollback_errors.join("|"))
        });
    }

    let manifest_bytes = fs::read(&paths.manifest_path).map_err(|error| error.to_string())?;
    Ok(PackageReceipt {
        exam_id: source.exam_id.clone(),
        runtime_sha256: runtime_sha256.to_string(),
        asset_manifest_sha256: sha256_hex(
            &fs::read(paths.resource_path.join(ASSET_MANIFEST_FILE_NAME))
                .map_err(|error| error.to_string())?,
        ),
        asset_count: source.assets.assets.len(),
        probe: probe.clone(),
        manifest_sha256: sha256_hex(&manifest_bytes),
    })
}

fn make_paths(
    library_root: &Path,
    reading_root: &Path,
    exam_id: &str,
) -> CommandResult<PackagePaths> {
    if exam_id.is_empty() || exam_id == "." || exam_id == ".." {
        return Err("nas_package_v2_exam_id_invalid".to_string());
    }
    Ok(PackagePaths {
        library_root: library_root.to_path_buf(),
        reading_root: reading_root.to_path_buf(),
        manifest_path: reading_root.join("manifest.js"),
        exam_path: reading_root.join(format!("{exam_id}.js")),
        resource_path: reading_root.join("resources").join(exam_id),
        staging_root: PathBuf::new(),
        staging_exam_path: PathBuf::new(),
        staging_resource_path: PathBuf::new(),
        staging_manifest_path: PathBuf::new(),
        backup_root: PathBuf::new(),
        journal_path: PathBuf::new(),
        report_path: PathBuf::new(),
        lock_path: library_root.join(CONTROL_DIR_NAME).join("export.lock"),
        lock_metadata_path: library_root
            .join(CONTROL_DIR_NAME)
            .join("export.lock.owner.json"),
        base_manifest_sha256: String::new(),
    })
}

fn absolute_path(label: &str, value: &str) -> CommandResult<PathBuf> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() {
        return Err(format!("nas_package_v2_{label}_must_be_absolute"));
    }
    Ok(path)
}

fn validate_package_relative_path(value: &str) -> CommandResult<()> {
    if value.is_empty()
        || value.contains('\0')
        || value.contains('\\')
        || value.contains("://")
        || value.contains(':')
        || value.starts_with('/')
        || value.starts_with("//")
        || value
            .chars()
            .any(|character| matches!(character, '⁄' | '∕' | '╱' | '⧸'))
    {
        return Err(format!("nas_package_v2_asset_path_unsafe:{value}"));
    }
    if value
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(format!("nas_package_v2_asset_path_unsafe:{value}"));
    }
    Ok(())
}

fn parse_runtime_version(value: &str) -> Option<[u64; 3]> {
    let core = value
        .trim()
        .strip_prefix('v')
        .or_else(|| value.trim().strip_prefix('V'))
        .unwrap_or(value.trim())
        .split(['-', '+'])
        .next()?;
    let parts = core.split('.').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let mut parsed = [0_u64; 3];
    for (index, part) in parts.iter().enumerate() {
        parsed[index] = part.parse().ok()?;
    }
    Some(parsed)
}

fn validate_minimum_runtime_version(value: &str) -> CommandResult<()> {
    let parsed = parse_runtime_version(value)
        .ok_or_else(|| format!("nas_package_v2_runtime_version_invalid:{value}"))?;
    if parsed > CURRENT_STUDENT_RUNTIME_VERSION {
        return Err(format!(
            "nas_package_v2_runtime_incompatible:minimum={value}:current=0.2.0"
        ));
    }
    Ok(())
}

fn load_existing_manifest(path: &Path) -> CommandResult<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let payload = extract_manifest_json_object(&source)?;
    serde_json::from_str::<Value>(&payload)
        .map_err(|error| format!("nas_package_v2_existing_manifest_invalid_json:{error}"))?
        .as_object()
        .cloned()
        .ok_or_else(|| "nas_package_v2_existing_manifest_not_object".to_string())
}

/// Extract the JSON object assigned to the manifest global from either the
/// direct V2 wrapper or the legacy closure-style V1 wrapper, e.g.
/// `global.__READING_EXAM_MANIFEST__ = { ... };`. The scanner only accepts a
/// balanced JSON object and respects quoted strings/escapes, so surrounding JS
/// statements are never passed to serde_json.
fn extract_manifest_json_object(source: &str) -> CommandResult<String> {
    let marker = "__READING_EXAM_MANIFEST__";
    let marker_index = source
        .find(marker)
        .ok_or_else(|| "nas_package_v2_existing_manifest_invalid_assignment".to_string())?;
    let assignment = &source[marker_index + marker.len()..];
    let object_start = assignment
        .find('{')
        .ok_or_else(|| "nas_package_v2_existing_manifest_invalid_assignment".to_string())?;
    let bytes = assignment.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for index in object_start..bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(assignment[object_start..=index].to_string());
                }
            }
            _ => {}
        }
    }
    Err("nas_package_v2_existing_manifest_invalid_assignment".to_string())
}

fn verify_manifest_compare_and_swap(path: &Path, expected: &str) -> CommandResult<()> {
    let actual = manifest_sha256(path)?;
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(format!(
            "nas_package_v2_manifest_conflict:expected={}:actual={actual}",
            expected.trim()
        ));
    }
    Ok(())
}

fn manifest_sha256(path: &Path) -> CommandResult<String> {
    if !path.exists() {
        return Ok(String::new());
    }
    if !path.is_file() {
        return Err("nas_package_v2_manifest_not_file".to_string());
    }
    Ok(sha256_hex(
        &fs::read(path).map_err(|error| error.to_string())?,
    ))
}

fn safe_transaction_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        && !value.starts_with('.')
}

fn write_lock_metadata(paths: &PackagePaths, export_id: &str) -> CommandResult<()> {
    let now = Utc::now().to_rfc3339();
    let metadata = json!({
        "schemaVersion": "NasPublishLockV1",
        "exportId": export_id,
        "pid": std::process::id(),
        "createdAt": now,
        "heartbeatAt": now,
        "lockPath": paths.lock_path,
        "manifestPath": paths.manifest_path
    });
    write_synced_file(
        &paths.lock_metadata_path,
        &canonical_json_bytes(&metadata).map_err(|error| error.to_string())?,
    )
}

fn update_lock_heartbeat(paths: &PackagePaths) -> CommandResult<()> {
    let raw = fs::read_to_string(&paths.lock_metadata_path)
        .map_err(|error| format!("nas_package_v2_lock_metadata_read:{error}"))?;
    let mut metadata: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("nas_package_v2_lock_metadata_invalid:{error}"))?;
    let object = metadata
        .as_object_mut()
        .ok_or_else(|| "nas_package_v2_lock_metadata_not_object".to_string())?;
    object.insert("heartbeatAt".to_string(), json!(Utc::now().to_rfc3339()));
    write_synced_file(
        &paths.lock_metadata_path,
        &canonical_json_bytes(&metadata).map_err(|error| error.to_string())?,
    )
}

fn cleanup_transaction_artifacts(paths: &PackagePaths) {
    let _ = fs::remove_dir_all(&paths.staging_root);
    let _ = fs::remove_dir_all(&paths.backup_root);
}

fn remove_path_if_present(path: &Path) -> CommandResult<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|error| error.to_string())
    } else {
        fs::remove_file(path).map_err(|error| error.to_string())
    }
}

/// Copy a file into the durable transaction backup and verify that the copy
/// contains exactly the same bytes before the source is allowed to remain in
/// service.  The manifest uses copy (rather than rename) so the previous
/// discovery point remains visible while the package is being committed.
fn copy_file_verified(source: &Path, destination: &Path) -> CommandResult<()> {
    if !source.is_file() {
        return Err(format!("backup_source_not_file:{}", source.display()));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = destination.with_extension(format!("copy-{}", Uuid::new_v4().simple()));
    fs::copy(source, &temporary).map_err(|error| error.to_string())?;
    let source_bytes = fs::read(source).map_err(|error| error.to_string())?;
    let destination_bytes = fs::read(&temporary).map_err(|error| error.to_string())?;
    if source_bytes != destination_bytes {
        let _ = fs::remove_file(&temporary);
        return Err("backup_copy_verification_failed".to_string());
    }
    fs::rename(&temporary, destination).map_err(|error| error.to_string())?;
    Ok(())
}

/// Replace a destination file with a staged file in the same directory.
/// `rename` is an atomic replacement on Unix.  Windows refuses to rename
/// over an existing file, so use the platform-compatible remove/rename
/// fallback; the old file is always present in the durable backup before this
/// helper is called, allowing recovery if the second operation is interrupted.
fn atomic_replace_file(source: &Path, destination: &Path) -> CommandResult<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(rename_error) if destination.exists() => {
            fs::remove_file(destination).map_err(|error| {
                format!("replace_remove_destination:{error};rename={rename_error}")
            })?;
            fs::rename(source, destination)
                .map_err(|error| format!("replace_rename_source:{error};initial={rename_error}"))
        }
        Err(error) => Err(error.to_string()),
    }
}

fn validate_backup_contents(
    backup_exam: &Path,
    backup_resources: &Path,
    backup_manifest: &Path,
    backup_report: &Path,
    exam_id: &str,
    had_exam: bool,
    had_resources: bool,
    had_manifest: bool,
    require_report_backup: bool,
) -> CommandResult<()> {
    let mut missing = Vec::new();
    if had_exam && !backup_exam.is_file() {
        missing.push("exam.js");
    }
    if had_resources && !backup_resources.join(exam_id).is_dir() {
        missing.push("resources");
    }
    if had_manifest && !backup_manifest.is_file() {
        missing.push("manifest.js");
    }
    if require_report_backup && !backup_report.is_file() {
        missing.push("report.json");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("missing={}", missing.join(",")))
    }
}

/// Restore only entries that were already moved out of the live package while
/// building the durable backup.  This helper is intentionally conservative:
/// it never removes an existing live path, so a partial backup failure cannot
/// turn into a manifest-visible data loss.  The full rollback closure below is
/// still responsible for failures after the new package starts committing.
fn restore_partial_backup_before_commit(
    paths: &PackagePaths,
    backup_exam: &Path,
    backup_resources: &Path,
    exam_id: &str,
    had_exam: bool,
    had_resources: bool,
) -> Vec<String> {
    let mut errors = Vec::new();
    if had_exam && !paths.exam_path.exists() && backup_exam.is_file() {
        if let Err(error) = fs::rename(backup_exam, &paths.exam_path) {
            errors.push(format!("restore_exam_before_backup_failure:{error}"));
        }
    }
    let backup_resource_path = backup_resources.join(exam_id);
    if had_resources && !paths.resource_path.exists() && backup_resource_path.is_dir() {
        if let Err(error) = fs::rename(&backup_resource_path, &paths.resource_path) {
            errors.push(format!("restore_resources_before_backup_failure:{error}"));
        }
    }
    errors
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> CommandResult<()> {
    if !source.is_dir() {
        return Err(format!("backup_source_not_directory:{}", source.display()));
    }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if source_path.is_file() {
            copy_file_verified(&source_path, &destination_path)?;
        } else {
            return Err(format!(
                "backup_source_not_regular:{}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn restore_backup_file_or_keep(
    backup: &Path,
    live: &Path,
    required: bool,
    label: &str,
    atomic_replace: bool,
) -> CommandResult<()> {
    if backup.exists() {
        if atomic_replace {
            atomic_replace_file(backup, live)
                .map_err(|error| format!("nas_package_v2_recovery_{label}:{error}"))?;
        } else {
            remove_path_if_present(live)?;
            if let Some(parent) = live.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::rename(backup, live)
                .map_err(|error| format!("nas_package_v2_recovery_{label}:{error}"))?;
        }
    } else if required && !live.exists() {
        return Err(format!(
            "nas_package_v2_recovery_backup_incomplete:{label}_missing"
        ));
    }
    Ok(())
}

fn restore_backup_dir_or_keep(
    backup: &Path,
    live: &Path,
    required: bool,
    label: &str,
) -> CommandResult<()> {
    if backup.exists() {
        remove_path_if_present(live)?;
        if let Some(parent) = live.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::rename(backup, live)
            .map_err(|error| format!("nas_package_v2_recovery_{label}:{error}"))?;
    } else if required && !live.exists() {
        return Err(format!(
            "nas_package_v2_recovery_backup_incomplete:{label}_missing"
        ));
    }
    Ok(())
}

fn recover_incomplete_transactions(paths: &PackagePaths) -> CommandResult<()> {
    let control_root = paths.library_root.join(CONTROL_DIR_NAME);
    if !control_root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&control_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let journal_path = entry.path();
        if journal_path.extension().and_then(|value| value.to_str()) != Some("json")
            || !journal_path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.ends_with(".journal.json"))
        {
            continue;
        }
        let raw = fs::read_to_string(&journal_path).map_err(|error| error.to_string())?;
        let mut journal: Value = serde_json::from_str(&raw)
            .map_err(|error| format!("nas_package_v2_recovery_journal_invalid:{error}"))?;
        let status = journal
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if matches!(status, "committed" | "recovered") {
            continue;
        }
        let exam_id = journal
            .get("examId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let export_id = journal
            .get("exportId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !safe_transaction_component(exam_id) || !safe_transaction_component(export_id) {
            return Err("nas_package_v2_recovery_journal_identity_invalid".to_string());
        }
        let staging_root = paths
            .reading_root
            .join(format!(".phase6-staging-{export_id}"));
        let backup_root = control_root
            .join(BACKUP_DIR_NAME)
            .join(format!("{exam_id}-{export_id}"));
        let exam_path = paths.reading_root.join(format!("{exam_id}.js"));
        let resource_path = paths.reading_root.join("resources").join(exam_id);
        let manifest_path = paths.reading_root.join("manifest.js");
        let report_path = paths
            .library_root
            .join(REPORT_DIR_NAME)
            .join(format!("{exam_id}-{export_id}.json"));
        let commit_started = matches!(status, "committing" | "manifest_committed" | "failed")
            || backup_root.exists();
        if commit_started && backup_root.exists() {
            let state_path = backup_root.join("state.json");
            let state: Value = serde_json::from_str(
                &fs::read_to_string(&state_path)
                    .map_err(|error| format!("nas_package_v2_recovery_state_missing:{error}"))?,
            )
            .map_err(|error| format!("nas_package_v2_recovery_state_invalid:{error}"))?;
            let had_exam = state
                .get("hadExam")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let had_resources = state
                .get("hadResources")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let had_manifest = state
                .get("hadManifest")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let had_report = state
                .get("hadReport")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let backup_exam = backup_root.join("exam.js");
            let backup_resources = backup_root.join("resources");
            let backup_manifest = backup_root.join("manifest.js");
            let backup_report = backup_root.join("report.json");
            // A crash may occur between individual backup renames. Restore
            // each available old item independently; if its backup is absent,
            // preserve the live item and only fail when that required item is
            // absent too. This is idempotent and keeps the old package
            // loadable across repeated recovery attempts.
            restore_backup_file_or_keep(&backup_exam, &exam_path, had_exam, "exam", false)?;
            restore_backup_dir_or_keep(
                &backup_resources.join(exam_id),
                &resource_path,
                had_resources,
                "resources",
            )?;
            restore_backup_file_or_keep(
                &backup_manifest,
                &manifest_path,
                had_manifest,
                "manifest",
                true,
            )?;
            restore_backup_file_or_keep(&backup_report, &report_path, had_report, "report", false)?;
        }
        remove_path_if_present(&staging_root)?;
        remove_path_if_present(&backup_root)?;
        if let Some(object) = journal.as_object_mut() {
            object.insert("status".to_string(), json!("recovered"));
            object.insert("recoveredAt".to_string(), json!(Utc::now().to_rfc3339()));
        }
        write_synced_file(
            &journal_path,
            &canonical_json_bytes(&journal).map_err(|error| error.to_string())?,
        )?;
    }
    Ok(())
}

fn write_synced_file(path: &Path, bytes: &[u8]) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let tmp = path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()));
    let mut file = File::create(&tmp).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(&tmp, path).map_err(|error| error.to_string())?;
    Ok(())
}

fn write_journal(
    paths: &PackagePaths,
    source: &ReadingExamSourceV2,
    status: &str,
    error: Option<&String>,
    export_id: &str,
) -> CommandResult<()> {
    let journal = json!({
        "schemaVersion": "NasCommitJournalV1",
        "exportId": export_id,
        "examId": source.exam_id,
        "status": status,
        "manifestPath": paths.manifest_path,
        "stagingRoot": paths.staging_root,
        "backupRoot": paths.backup_root,
        "manifestCommitted": status == "manifest_committed" || status == "committed",
        "baseManifestSha256": paths.base_manifest_sha256,
        "error": error,
        "updatedAt": Utc::now().to_rfc3339()
    });
    if let Some(parent) = paths.journal_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    write_synced_file(
        &paths.journal_path,
        &canonical_json_bytes(&journal).map_err(|error| error.to_string())?,
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading_source_v2::compile_reading_source_v2;
    use crate::schema::IeltsAuthoringIRV2;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ielts-phase6-package-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn package_path_policy_rejects_unsafe_relative_paths() {
        for path in [
            "../x.png",
            "/x.png",
            "C:/x.png",
            "images/⁄x.png",
            "images//x.png",
        ] {
            assert!(validate_package_relative_path(path).is_err(), "{path}");
        }
        assert!(validate_package_relative_path("images/x.png").is_ok());
    }

    #[test]
    fn manifest_cas_rejects_stale_writer() {
        let root = temp_root();
        let manifest = root.join("manifest.js");
        fs::write(&manifest, "window.__READING_EXAM_MANIFEST__ = {};\n").unwrap();
        assert!(verify_manifest_compare_and_swap(&manifest, &"0".repeat(64)).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn existing_manifest_parser_accepts_legacy_closure_wrapper() {
        let root = temp_root();
        let manifest = root.join("manifest.js");
        fs::write(
            &manifest,
            "(function register(global) { global.__READING_EXAM_MANIFEST__ = {\"legacy\": {\"schemaVersion\": \"ReadingExamManifestV1\"}}; })(window);\n",
        )
        .unwrap();
        let parsed = load_existing_manifest(&manifest).unwrap();
        assert_eq!(
            parsed
                .get("legacy")
                .and_then(Value::as_object)
                .and_then(|value| value.get("schemaVersion")),
            Some(&Value::String("ReadingExamManifestV1".to_string()))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_normalizes_publish_child_to_one_transaction_root() {
        let root = temp_root();
        let source_path = root.join("reading-source-v2.json");
        let authoring: IeltsAuthoringIRV2 = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        let source = compile_reading_source_v2(&authoring).unwrap();
        fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();

        let nas_parent = root.join("nas");
        let publish_child = nas_parent.join("publish");
        let result = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": publish_child,
                "sourcePath": source_path
            }),
        )
        .unwrap();
        assert_eq!(result["status"], "committed");
        assert!(nas_parent.join("manifest.js").is_file());
        assert!(nas_parent.join("early-approaches.js").is_file());
        assert!(nas_parent
            .join(CONTROL_DIR_NAME)
            .join("export.lock")
            .is_file());
        assert!(!publish_child.join("manifest.js").exists());
        assert!(!publish_child.join(CONTROL_DIR_NAME).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_version_policy_blocks_invalid_or_future_student_runtime() {
        assert!(validate_minimum_runtime_version("0.2.0").is_ok());
        assert!(validate_minimum_runtime_version("0.1.9").is_ok());
        assert!(validate_minimum_runtime_version("future").is_err());
        assert!(validate_minimum_runtime_version("0.3.0").is_err());
    }

    #[test]
    fn package_publish_probes_before_commit_and_fault_rolls_back_old_manifest() {
        let root = temp_root();
        let source_path = root.join("reading-source-v2.json");
        let authoring: IeltsAuthoringIRV2 = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        let source = compile_reading_source_v2(&authoring).unwrap();
        fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();
        let library_root = root.join("library");
        let first = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path,
                "minimumRuntimeVersion": "0.2.0"
            }),
        )
        .unwrap();
        assert_eq!(first["status"], "committed");
        let manifest_path = library_root.join("manifest.js");
        let old_manifest = fs::read(&manifest_path).unwrap();
        let old_source = fs::read(library_root.join("early-approaches.js")).unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path,
                "fault": "manifest_rename"
            }),
        )
        .unwrap_err();
        assert!(error.contains("rollback=complete"), "{error}");
        assert_eq!(fs::read(&manifest_path).unwrap(), old_manifest);
        assert_eq!(
            fs::read(library_root.join("early-approaches.js")).unwrap(),
            old_source
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn partial_resource_backup_fault_restores_old_visible_package() {
        let root = temp_root();
        let source_path = root.join("reading-source-v2.json");
        let authoring: IeltsAuthoringIRV2 = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        let source = compile_reading_source_v2(&authoring).unwrap();
        fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();
        let library_root = root.join("library");
        publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path
            }),
        )
        .unwrap();

        let manifest_path = library_root.join("manifest.js");
        let exam_path = library_root.join("early-approaches.js");
        let resource_path = library_root
            .join("resources")
            .join("early-approaches")
            .join(ASSET_MANIFEST_FILE_NAME);
        let old_manifest = fs::read(&manifest_path).unwrap();
        let old_exam = fs::read(&exam_path).unwrap();
        let old_asset_manifest = fs::read(&resource_path).unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path,
                "fault": "backup_resources"
            }),
        )
        .unwrap_err();

        assert!(
            error.contains("nas_package_v2_fault_backup_resources"),
            "{error}"
        );
        assert!(error.contains("rollback=complete"), "{error}");
        assert_eq!(fs::read(&manifest_path).unwrap(), old_manifest);
        assert_eq!(fs::read(&exam_path).unwrap(), old_exam);
        assert_eq!(fs::read(&resource_path).unwrap(), old_asset_manifest);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn post_commit_report_failure_rolls_back_package_and_report() {
        let root = temp_root();
        let source_path = root.join("reading-source-v2.json");
        let authoring: IeltsAuthoringIRV2 = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        let source = compile_reading_source_v2(&authoring).unwrap();
        fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();
        let library_root = root.join("library");
        let first = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path
            }),
        )
        .unwrap();
        let old_manifest = fs::read(library_root.join("manifest.js")).unwrap();
        let old_exam = fs::read(library_root.join("early-approaches.js")).unwrap();
        let report_path = PathBuf::from(first["reportPath"].as_str().unwrap());
        let old_report = fs::read(&report_path).unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path,
                "fault": "report_write"
            }),
        )
        .unwrap_err();
        assert!(
            error.contains("nas_package_v2_fault_report_write"),
            "{error}"
        );
        assert!(error.contains("rollback=complete"), "{error}");
        assert_eq!(
            fs::read(library_root.join("manifest.js")).unwrap(),
            old_manifest
        );
        assert_eq!(
            fs::read(library_root.join("early-approaches.js")).unwrap(),
            old_exam
        );
        assert_eq!(fs::read(&report_path).unwrap(), old_report);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn v2_commit_fault_preserves_loadable_legacy_v1_manifest() {
        let root = temp_root();
        let source_path = root.join("reading-source-v2.json");
        let authoring: IeltsAuthoringIRV2 = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        let source = compile_reading_source_v2(&authoring).unwrap();
        fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();

        let library_root = root.join("library");
        fs::create_dir_all(&library_root).unwrap();
        let legacy_manifest = json!({
            "legacy-v1": {
                "examId": "legacy-v1",
                "dataKey": "legacy-v1",
                "script": "./legacy-v1.js",
                "title": "Legacy V1",
                "category": "P1"
            },
            "_meta": {
                "schemaVersion": "ReadingExamManifestV1",
                "assetCount": 1
            }
        });
        let legacy_manifest_js = format!(
            "window.__READING_EXAM_MANIFEST__ = {};\n",
            serde_json::to_string_pretty(&legacy_manifest).unwrap()
        );
        let manifest_path = library_root.join("manifest.js");
        fs::write(&manifest_path, &legacy_manifest_js).unwrap();
        let legacy_exam_path = library_root.join("legacy-v1.js");
        fs::write(&legacy_exam_path, "legacy-v1-wrapper").unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path,
                "fault": "manifest_rename"
            }),
        )
        .unwrap_err();

        assert!(error.contains("rollback=complete"), "{error}");
        assert_eq!(
            fs::read(&manifest_path).unwrap(),
            legacy_manifest_js.as_bytes()
        );
        assert_eq!(
            load_existing_manifest(&manifest_path).unwrap(),
            legacy_manifest.as_object().unwrap().clone()
        );
        assert_eq!(fs::read(&legacy_exam_path).unwrap(), b"legacy-v1-wrapper");
        assert!(!library_root.join("early-approaches.js").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_publish_rejects_busy_lock_before_commit() {
        let root = temp_root();
        let source_path = root.join("reading-source-v2.json");
        let authoring: IeltsAuthoringIRV2 = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        let source = compile_reading_source_v2(&authoring).unwrap();
        fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();

        let library_root = root.join("library");
        let lock_path = make_paths(&library_root, &library_root, &source.exam_id)
            .unwrap()
            .lock_path;
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        lock.try_lock_exclusive().unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path
            }),
        )
        .unwrap_err();

        assert!(error.starts_with("nas_package_v2_lock_busy:"), "{error}");
        assert!(!library_root.join("manifest.js").exists());
        assert!(!library_root.join("early-approaches.js").exists());

        lock.unlock().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn commit_cas_rejects_manifest_changed_after_staging_baseline() {
        let root = temp_root();
        let manifest = root.join("manifest.js");
        fs::write(&manifest, "old").unwrap();
        let baseline = manifest_sha256(&manifest).unwrap();
        fs::write(&manifest, "changed").unwrap();
        let error = verify_manifest_compare_and_swap(&manifest, &baseline).unwrap_err();
        assert!(
            error.starts_with("nas_package_v2_manifest_conflict:"),
            "{error}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_restores_persistent_backup_after_manifest_commit_interruption() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = library_root.clone();
        let exam_id = "recovery-exam";
        let export_id = "recovery-export";
        let mut paths = make_paths(&library_root, &reading_root, exam_id).unwrap();
        paths.staging_root = reading_root.join(format!(".phase6-staging-{export_id}"));
        paths.backup_root = library_root
            .join(CONTROL_DIR_NAME)
            .join(BACKUP_DIR_NAME)
            .join(format!("{exam_id}-{export_id}"));
        paths.journal_path = library_root
            .join(CONTROL_DIR_NAME)
            .join(format!("{exam_id}-{export_id}.journal.json"));
        fs::create_dir_all(&paths.backup_root).unwrap();
        fs::create_dir_all(paths.resource_path.parent().unwrap()).unwrap();
        fs::create_dir_all(&paths.resource_path).unwrap();
        fs::write(&paths.exam_path, "old-exam").unwrap();
        fs::write(&paths.resource_path.join("asset.bin"), "old-asset").unwrap();
        fs::write(&paths.manifest_path, "old-manifest").unwrap();
        let state = json!({
            "schemaVersion": "NasBackupStateV1",
            "hadExam": true,
            "hadResources": true,
            "hadManifest": true
        });
        write_synced_file(
            &paths.backup_root.join("state.json"),
            &canonical_json_bytes(&state).unwrap(),
        )
        .unwrap();
        fs::rename(&paths.exam_path, paths.backup_root.join("exam.js")).unwrap();
        fs::create_dir_all(paths.backup_root.join("resources")).unwrap();
        fs::rename(
            &paths.resource_path,
            paths.backup_root.join("resources").join(exam_id),
        )
        .unwrap();
        fs::rename(&paths.manifest_path, paths.backup_root.join("manifest.js")).unwrap();
        fs::write(&paths.exam_path, "new-exam").unwrap();
        fs::create_dir_all(&paths.resource_path).unwrap();
        fs::write(&paths.resource_path.join("asset.bin"), "new-asset").unwrap();
        fs::write(&paths.manifest_path, "new-manifest").unwrap();
        fs::create_dir_all(&paths.staging_root).unwrap();
        let journal = json!({
            "schemaVersion": "NasCommitJournalV1",
            "exportId": export_id,
            "examId": exam_id,
            "status": "manifest_committed"
        });
        write_synced_file(
            &paths.journal_path,
            &canonical_json_bytes(&journal).unwrap(),
        )
        .unwrap();

        recover_incomplete_transactions(&paths).unwrap();

        assert_eq!(fs::read_to_string(&paths.exam_path).unwrap(), "old-exam");
        assert_eq!(
            fs::read_to_string(paths.resource_path.join("asset.bin")).unwrap(),
            "old-asset"
        );
        assert_eq!(
            fs::read_to_string(&paths.manifest_path).unwrap(),
            "old-manifest"
        );
        assert!(!paths.backup_root.exists());
        assert!(!paths.staging_root.exists());
        let recovered: Value =
            serde_json::from_str(&fs::read_to_string(&paths.journal_path).unwrap()).unwrap();
        assert_eq!(recovered["status"], "recovered");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_keeps_visible_package_when_no_backup_moves_completed() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = library_root.clone();
        let exam_id = "incomplete-backup-exam";
        let export_id = "incomplete-backup-export";
        let mut paths = make_paths(&library_root, &reading_root, exam_id).unwrap();
        paths.staging_root = reading_root.join(format!(".phase6-staging-{export_id}"));
        paths.backup_root = library_root
            .join(CONTROL_DIR_NAME)
            .join(BACKUP_DIR_NAME)
            .join(format!("{exam_id}-{export_id}"));
        paths.journal_path = library_root
            .join(CONTROL_DIR_NAME)
            .join(format!("{exam_id}-{export_id}.journal.json"));

        fs::create_dir_all(paths.resource_path.parent().unwrap()).unwrap();
        fs::create_dir_all(&paths.resource_path).unwrap();
        fs::write(&paths.exam_path, "old-exam").unwrap();
        fs::write(paths.resource_path.join("asset.bin"), "old-asset").unwrap();
        fs::write(&paths.manifest_path, "old-manifest").unwrap();

        // Simulate a crash immediately after state.json was durable, before
        // any of the old files reached the backup directory.
        fs::create_dir_all(&paths.backup_root).unwrap();
        let state = json!({
            "schemaVersion": "NasBackupStateV1",
            "hadExam": true,
            "hadResources": true,
            "hadManifest": true,
            "hadReport": false
        });
        write_synced_file(
            &paths.backup_root.join("state.json"),
            &canonical_json_bytes(&state).unwrap(),
        )
        .unwrap();
        let journal = json!({
            "schemaVersion": "NasCommitJournalV1",
            "exportId": export_id,
            "examId": exam_id,
            "status": "committing"
        });
        write_synced_file(
            &paths.journal_path,
            &canonical_json_bytes(&journal).unwrap(),
        )
        .unwrap();

        recover_incomplete_transactions(&paths).unwrap();
        assert_eq!(fs::read_to_string(&paths.exam_path).unwrap(), "old-exam");
        assert_eq!(
            fs::read_to_string(paths.resource_path.join("asset.bin")).unwrap(),
            "old-asset"
        );
        assert_eq!(
            fs::read_to_string(&paths.manifest_path).unwrap(),
            "old-manifest"
        );
        assert!(!paths.backup_root.exists());
        let recovered: Value =
            serde_json::from_str(&fs::read_to_string(&paths.journal_path).unwrap()).unwrap();
        assert_eq!(recovered["status"], "recovered");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_restores_partial_exam_backup_and_keeps_other_live_items() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = library_root.clone();
        let exam_id = "partial-backup-exam";
        let export_id = "partial-backup-export";
        let mut paths = make_paths(&library_root, &reading_root, exam_id).unwrap();
        paths.staging_root = reading_root.join(format!(".phase6-staging-{export_id}"));
        paths.backup_root = library_root
            .join(CONTROL_DIR_NAME)
            .join(BACKUP_DIR_NAME)
            .join(format!("{exam_id}-{export_id}"));
        paths.journal_path = library_root
            .join(CONTROL_DIR_NAME)
            .join(format!("{exam_id}-{export_id}.journal.json"));

        fs::create_dir_all(paths.resource_path.parent().unwrap()).unwrap();
        fs::create_dir_all(&paths.resource_path).unwrap();
        fs::write(paths.resource_path.join("asset.bin"), "old-asset").unwrap();
        fs::write(&paths.manifest_path, "old-manifest").unwrap();
        fs::create_dir_all(&paths.backup_root).unwrap();
        fs::write(paths.backup_root.join("exam.js"), "old-exam").unwrap();
        let state = json!({
            "schemaVersion": "NasBackupStateV1",
            "hadExam": true,
            "hadResources": true,
            "hadManifest": true,
            "hadReport": false
        });
        write_synced_file(
            &paths.backup_root.join("state.json"),
            &canonical_json_bytes(&state).unwrap(),
        )
        .unwrap();
        let journal = json!({
            "schemaVersion": "NasCommitJournalV1",
            "exportId": export_id,
            "examId": exam_id,
            "status": "committing"
        });
        write_synced_file(
            &paths.journal_path,
            &canonical_json_bytes(&journal).unwrap(),
        )
        .unwrap();

        recover_incomplete_transactions(&paths).unwrap();
        assert_eq!(fs::read_to_string(&paths.exam_path).unwrap(), "old-exam");
        assert_eq!(
            fs::read_to_string(paths.resource_path.join("asset.bin")).unwrap(),
            "old-asset"
        );
        assert_eq!(
            fs::read_to_string(&paths.manifest_path).unwrap(),
            "old-manifest"
        );
        assert!(!paths.backup_root.exists());

        let _ = fs::remove_dir_all(root);
    }
}
