#![allow(dead_code)]

//! Phase 6 runtime/package primitives owned by the authoring repository.
//!
//! The student repository will eventually call the same checks from its NAS
//! loader. Keeping the path and asset closure policy here first gives the
//! authoring export and the future student probe one deterministic contract.

use crate::reading_source_v2::{validate_reading_source_v2, ReadingExamSourceV2};
use crate::schema::common::{AssetDescriptorV2, AssetKindV2};
use crate::schema::content_doc_v2::ContentNodeV2;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const EXAM_ASSET_MANIFEST_V2_SCHEMA_VERSION: &str = "ExamAssetManifestV2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExamAssetManifestV2 {
    pub schema_version: String,
    pub exam_id: String,
    pub generated_at: String,
    pub assets: BTreeMap<String, AssetDescriptorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeAssetIssueV2 {
    pub code: String,
    pub target_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StudentProbeReportV2 {
    pub schema_version: String,
    pub exam_id: String,
    pub passed: bool,
    pub checked_asset_ids: Vec<String>,
    pub referenced_asset_ids: Vec<String>,
    pub issues: Vec<RuntimeAssetIssueV2>,
}

pub(crate) fn run_student_loader_probe(
    source: &ReadingExamSourceV2,
    manifest: &ExamAssetManifestV2,
    resource_root: &Path,
) -> StudentProbeReportV2 {
    let mut issues = Vec::new();
    let referenced_asset_ids = referenced_asset_ids(source);
    let checked_asset_ids = manifest.assets.keys().cloned().collect::<Vec<_>>();
    if !validate_reading_source_v2(source).is_empty() {
        issues.push(asset_issue(
            "RUNTIME_SOURCE_INVALID",
            &source.exam_id,
            "ReadingExamSourceV2 failed the runtime semantic validator.",
        ));
    }
    if manifest.schema_version != EXAM_ASSET_MANIFEST_V2_SCHEMA_VERSION {
        issues.push(asset_issue(
            "ASSET_MANIFEST_SCHEMA_UNSUPPORTED",
            &manifest.exam_id,
            "Unsupported asset manifest schema version.",
        ));
    }
    if manifest.exam_id != source.exam_id || source.assets.exam_id != source.exam_id {
        issues.push(asset_issue(
            "ASSET_MANIFEST_EXAM_MISMATCH",
            &source.exam_id,
            "Runtime source, source asset reference and package manifest must use the same examId.",
        ));
    }

    let source_assets = unique_asset_map(&source.assets.assets, &mut issues);
    if source_assets.len() != manifest.assets.len() {
        issues.push(asset_issue(
            "ASSET_MANIFEST_SET_MISMATCH",
            &source.exam_id,
            "Package asset manifest must contain exactly the source asset set.",
        ));
    }
    for (asset_id, descriptor) in &source_assets {
        match manifest.assets.get(asset_id) {
            Some(package_descriptor) if package_descriptor == *descriptor => {}
            Some(_) => issues.push(asset_issue(
                "ASSET_DESCRIPTOR_MISMATCH",
                asset_id,
                "Package descriptor differs from the runtime source descriptor.",
            )),
            None => issues.push(asset_issue(
                "ASSET_MANIFEST_ENTRY_MISSING",
                asset_id,
                "Runtime source asset is missing from the package manifest.",
            )),
        }
    }
    for asset_id in manifest.assets.keys() {
        if !source_assets.contains_key(asset_id) {
            issues.push(asset_issue(
                "ASSET_MANIFEST_ENTRY_ORPHANED",
                asset_id,
                "Package manifest contains an asset not declared by the runtime source.",
            ));
        }
    }
    for (asset_id, descriptor) in &manifest.assets {
        if let Err(asset_issues) = verify_asset_descriptor(resource_root, descriptor) {
            issues.extend(asset_issues);
        } else if asset_id != &descriptor.asset_id {
            issues.push(asset_issue(
                "ASSET_ID_MISMATCH",
                asset_id,
                "Manifest map key must equal descriptor.assetId.",
            ));
        }
    }
    for asset_id in &referenced_asset_ids {
        if !manifest.assets.contains_key(asset_id) {
            issues.push(asset_issue(
                "ASSET_CLOSURE_MISSING",
                asset_id,
                "A content node references an asset absent from the package manifest.",
            ));
        }
    }

    StudentProbeReportV2 {
        schema_version: "StudentLoaderProbeV2".to_string(),
        exam_id: source.exam_id.clone(),
        passed: issues.is_empty(),
        checked_asset_ids,
        referenced_asset_ids,
        issues,
    }
}

pub(crate) fn safe_join_asset_path(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    reject_unsafe_relative_path(relative_path)?;
    let canonical_root =
        fs::canonicalize(root).map_err(|error| format!("asset_root_canonicalize:{error}"))?;
    let candidate = canonical_root.join(relative_path);
    let canonical_candidate = fs::canonicalize(&candidate)
        .map_err(|error| format!("asset_path_canonicalize:{}:{error}", candidate.display()))?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err("asset_path_outside_resource_root".to_string());
    }
    Ok(canonical_candidate)
}

fn reject_unsafe_relative_path(relative_path: &str) -> Result<(), String> {
    if relative_path.is_empty()
        || relative_path.contains('\0')
        || relative_path.contains('\\')
        || relative_path.contains("://")
        || relative_path.contains(':')
        || relative_path.starts_with('/')
        || relative_path.starts_with("//")
        || relative_path.contains('⁄')
        || relative_path.contains('∕')
        || relative_path.contains('╱')
        || relative_path.contains('⧸')
        || relative_path.to_ascii_lowercase().contains("%2f")
        || relative_path.to_ascii_lowercase().contains("%5c")
    {
        return Err("asset_path_absolute_or_escaped".to_string());
    }
    for component in relative_path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err("asset_path_parent_or_empty_component".to_string());
        }
    }
    if Path::new(relative_path).is_absolute() {
        return Err("asset_path_absolute".to_string());
    }
    Ok(())
}

fn verify_asset_descriptor(
    resource_root: &Path,
    descriptor: &AssetDescriptorV2,
) -> Result<(), Vec<RuntimeAssetIssueV2>> {
    let path = safe_join_asset_path(resource_root, &descriptor.relative_path).map_err(|error| {
        vec![asset_issue(
            "ASSET_PATH_UNSAFE",
            &descriptor.asset_id,
            &error,
        )]
    })?;
    let metadata = fs::metadata(&path).map_err(|error| {
        vec![asset_issue(
            "ASSET_FILE_MISSING",
            &descriptor.asset_id,
            &format!("{}:{error}", path.display()),
        )]
    })?;
    if !metadata.is_file() {
        return Err(vec![asset_issue(
            "ASSET_FILE_NOT_REGULAR",
            &descriptor.asset_id,
            "Asset path must resolve to a regular file.",
        )]);
    }
    let bytes = fs::read(&path).map_err(|error| {
        vec![asset_issue(
            "ASSET_FILE_READ_FAILED",
            &descriptor.asset_id,
            &error.to_string(),
        )]
    })?;
    let mut issues = Vec::new();
    if bytes.len() as u64 != descriptor.byte_length {
        issues.push(asset_issue(
            "ASSET_SIZE_MISMATCH",
            &descriptor.asset_id,
            "Asset byteLength does not match the package file.",
        ));
    }
    if sha256_hex(&bytes) != descriptor.sha256.to_ascii_lowercase() {
        issues.push(asset_issue(
            "ASSET_HASH_MISMATCH",
            &descriptor.asset_id,
            "Asset sha256 does not match the package file.",
        ));
    }
    if !allowed_asset_mime(&descriptor.mime) {
        issues.push(asset_issue(
            "ASSET_MIME_UNSUPPORTED",
            &descriptor.asset_id,
            "Asset MIME is not in the offline runtime allowlist.",
        ));
    }
    if matches!(descriptor.kind, AssetKindV2::Audio) && !descriptor.mime.starts_with("audio/") {
        issues.push(asset_issue(
            "ASSET_MIME_KIND_MISMATCH",
            &descriptor.asset_id,
            "Audio assets must use an audio MIME type.",
        ));
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

fn allowed_asset_mime(mime: &str) -> bool {
    mime.starts_with("image/") || mime.starts_with("audio/") || mime == "application/octet-stream"
}

fn unique_asset_map<'a>(
    assets: &'a [AssetDescriptorV2],
    issues: &mut Vec<RuntimeAssetIssueV2>,
) -> BTreeMap<String, &'a AssetDescriptorV2> {
    let mut result = BTreeMap::new();
    for asset in assets {
        if result.insert(asset.asset_id.clone(), asset).is_some() {
            issues.push(asset_issue(
                "ASSET_ID_DUPLICATE",
                &asset.asset_id,
                "Runtime source contains duplicate assetId entries.",
            ));
        }
    }
    result
}

fn referenced_asset_ids(source: &ReadingExamSourceV2) -> Vec<String> {
    let mut ids = BTreeSet::new();
    collect_node_asset_ids(&source.passage.content, &mut ids);
    for task in &source.task_groups {
        collect_node_asset_ids(&task.instructions, &mut ids);
        if let Some(stimulus) = &task.stimulus {
            collect_node_asset_ids(stimulus, &mut ids);
        }
        if let Some(bank) = &task.option_bank {
            if let Some(title) = &bank.title {
                collect_node_asset_ids(title, &mut ids);
            }
            for option in &bank.options {
                collect_node_asset_ids(&option.content, &mut ids);
            }
        }
        for response in &task.response_groups {
            if let Some(prompt) = &response.prompt {
                collect_node_asset_ids(prompt, &mut ids);
            }
            if let Some(options) = &response.options {
                for option in options {
                    collect_node_asset_ids(&option.content, &mut ids);
                }
            }
        }
    }
    ids.into_iter().collect()
}

fn collect_node_asset_ids(nodes: &[ContentNodeV2], ids: &mut BTreeSet<String>) {
    let value = match serde_json::to_value(nodes) {
        Ok(value) => value,
        Err(_) => return,
    };
    collect_asset_ids_from_value(&value, ids);
}

fn collect_asset_ids_from_value(value: &serde_json::Value, ids: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_asset_ids_from_value(item, ids);
            }
        }
        serde_json::Value::Object(object) => {
            if let Some(asset_id) = object.get("assetId").and_then(serde_json::Value::as_str) {
                ids.insert(asset_id.to_string());
            }
            for child in object.values() {
                collect_asset_ids_from_value(child, ids);
            }
        }
        _ => {}
    }
}

fn asset_issue(code: &str, target_id: &str, message: &str) -> RuntimeAssetIssueV2 {
    RuntimeAssetIssueV2 {
        code: code.to_string(),
        target_id: target_id.to_string(),
        message: message.to_string(),
    }
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
    use crate::schema::common::{AssetExtractionModeV2, AssetKindV2};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ielts-phase6-assets-{suffix}"));
        fs::create_dir_all(root.join("images")).unwrap();
        root
    }

    fn descriptor(bytes: &[u8]) -> AssetDescriptorV2 {
        AssetDescriptorV2 {
            asset_id: "asset-1".to_string(),
            kind: AssetKindV2::RasterImage,
            mime: "image/png".to_string(),
            relative_path: "images/asset-1.png".to_string(),
            sha256: sha256_hex(bytes),
            byte_length: bytes.len() as u64,
            width_px: Some(1),
            height_px: Some(1),
            duration_ms: None,
            extraction_mode: AssetExtractionModeV2::Embedded,
            alt_text: Some("fixture".to_string()),
            decorative: Some(false),
            source_anchor: None,
            diagram_question_region: None,
        }
    }

    #[test]
    fn safe_join_rejects_traversal_absolute_and_lookalike_paths() {
        let root = temp_root();
        for path in [
            "../escape.png",
            "/absolute.png",
            "C:/drive.png",
            "\\\\server\\share\\file.png",
            "https://example.invalid/file.png",
            "images/.. /file.png",
            "images/⁄escape.png",
        ] {
            assert!(
                safe_join_asset_path(&root, path).is_err(),
                "path should be rejected: {path}"
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn asset_probe_checks_size_hash_mime_and_accepts_valid_blob() {
        let root = temp_root();
        let bytes = b"phase6-asset";
        let path = root.join("images/asset-1.png");
        fs::write(&path, bytes).unwrap();
        let valid = descriptor(bytes);
        assert!(verify_asset_descriptor(&root, &valid).is_ok());

        let mut wrong_hash = valid.clone();
        wrong_hash.sha256 = "0".repeat(64);
        let errors = verify_asset_descriptor(&root, &wrong_hash).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "ASSET_HASH_MISMATCH"));

        let mut wrong_mime = valid;
        wrong_mime.mime = "text/html".to_string();
        let errors = verify_asset_descriptor(&root, &wrong_mime).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.code == "ASSET_MIME_UNSUPPORTED"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn asset_id_map_preserves_duplicate_diagnostic() {
        let bytes = b"phase6-asset";
        let first = descriptor(bytes);
        let second = descriptor(bytes);
        let mut issues = Vec::new();
        let assets = [first, second];
        let map = unique_asset_map(&assets, &mut issues);
        assert_eq!(map.len(), 1);
        assert_eq!(issues[0].code, "ASSET_ID_DUPLICATE");
    }

    #[test]
    fn student_probe_fails_closed_when_manifest_asset_set_is_incomplete() {
        let authoring: crate::schema::IeltsAuthoringIRV2 = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        let source = crate::reading_source_v2::compile_reading_source_v2(&authoring).unwrap();
        let mut assets = BTreeMap::new();
        assets.insert("asset-1".to_string(), descriptor(b"phase6-asset"));
        let manifest = ExamAssetManifestV2 {
            schema_version: EXAM_ASSET_MANIFEST_V2_SCHEMA_VERSION.to_string(),
            exam_id: source.exam_id.clone(),
            generated_at: "2026-08-12T00:00:00Z".to_string(),
            assets,
        };
        let root = temp_root();
        let report = run_student_loader_probe(&source, &manifest, &root);
        assert!(!report.passed);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "ASSET_MANIFEST_SET_MISMATCH"));
        let _ = fs::remove_dir_all(root);
    }
}
