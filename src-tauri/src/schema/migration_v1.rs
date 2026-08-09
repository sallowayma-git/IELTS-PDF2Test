use crate::artifact_store::canonical_json_hash;
use crate::util::{read_json_opt, safe_job_dir};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub const DOCUMENT_IR_V1_SCHEMA_VERSION: &str = "DocumentIRV1";
pub const AUTHORING_IR_V1_SCHEMA_VERSION: &str = "ReadingAuthoringIRV1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationDispositionV2 {
    ReadOnly,
    NeedsReview,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationDirectionV2 {
    V1ToV2,
    V2ToV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlanV2 {
    pub direction: MigrationDirectionV2,
    pub from_schema_version: String,
    pub to_schema_version: String,
    pub disposition: MigrationDispositionV2,
    pub review_required: bool,
    pub preserved_artifact: bool,
    pub blocking_reasons: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LegacyArtifactKindV2 {
    DocumentIr,
    AuthoringIr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyArtifactSnapshotV2 {
    pub kind: LegacyArtifactKindV2,
    pub schema_version: String,
    pub canonical_sha256: String,
    pub artifact: Value,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPreviewV2 {
    pub snapshot: LegacyArtifactSnapshotV2,
    pub plan: MigrationPlanV2,
    pub v2_artifact: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyJobCompatibilityV2 {
    pub schema_version: String,
    pub job_id: String,
    pub opened_read_only: bool,
    pub document: Option<MigrationPreviewV2>,
    pub authoring: Option<MigrationPreviewV2>,
}

pub const LEGACY_JOB_COMPATIBILITY_SCHEMA_VERSION: &str = "LegacyJobCompatibilityV2";

pub fn inspect_document_v1(value: &Value) -> MigrationPlanV2 {
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if version != DOCUMENT_IR_V1_SCHEMA_VERSION {
        return blocked_plan(
            MigrationDirectionV2::V1ToV2,
            version,
            "DocumentIRV2",
            "input is not DocumentIRV1",
        );
    }

    MigrationPlanV2 {
        direction: MigrationDirectionV2::V1ToV2,
        from_schema_version: version.to_string(),
        to_schema_version: "DocumentIRV2".to_string(),
        disposition: MigrationDispositionV2::NeedsReview,
        review_required: true,
        preserved_artifact: true,
        blocking_reasons: Vec::new(),
        warnings: vec![
            "V1 remains the read-only compatibility oracle; no V2 fields are inferred here."
                .to_string(),
            "Glyph, source-anchor, and coverage facts require a later extractor before promotion."
                .to_string(),
        ],
    }
}

pub fn inspect_authoring_v1(value: &Value) -> MigrationPlanV2 {
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if version != AUTHORING_IR_V1_SCHEMA_VERSION {
        return blocked_plan(
            MigrationDirectionV2::V1ToV2,
            version,
            "IeltsAuthoringIRV2",
            "input is not ReadingAuthoringIRV1",
        );
    }

    MigrationPlanV2 {
        direction: MigrationDirectionV2::V1ToV2,
        from_schema_version: version.to_string(),
        to_schema_version: "IeltsAuthoringIRV2".to_string(),
        disposition: MigrationDispositionV2::NeedsReview,
        review_required: true,
        preserved_artifact: true,
        blocking_reasons: Vec::new(),
        warnings: vec![
            "Question ranges and HTML are preserved as V1 evidence, not promoted to verified V2 semantics."
                .to_string(),
            "Answers, shared stems, option reuse, and slots require explicit source evidence or user confirmation."
                .to_string(),
        ],
    }
}

pub fn inspect_v2_to_v1(value: &Value, target_schema_version: &str) -> MigrationPlanV2 {
    let source = value
        .get("schemaVersion")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    blocked_plan(
        MigrationDirectionV2::V2ToV1,
        source,
        target_schema_version,
        "V2-to-V1 compatibility compiler is not enabled in Schema-only PR-01",
    )
}

/// Capture a V1 artifact as an immutable compatibility snapshot.
///
/// The snapshot deliberately keeps the original JSON value. It is the
/// migration boundary's evidence, not a partially inferred V2 document.
pub fn capture_v1_artifact(
    value: &Value,
    kind: LegacyArtifactKindV2,
) -> Result<LegacyArtifactSnapshotV2, String> {
    let expected = expected_v1_schema(&kind);
    assert_schema_version(value, expected)?;
    let schema_version = value
        .get("schemaVersion")
        .and_then(Value::as_str)
        .unwrap_or(expected)
        .to_string();
    Ok(LegacyArtifactSnapshotV2 {
        kind,
        schema_version,
        canonical_sha256: canonical_json_hash(value)?,
        artifact: value.clone(),
        read_only: true,
    })
}

/// Preview the one-way, review-required V1 to V2 migration boundary.
///
/// Phase 1 has no lossless semantic converter. Returning an absent V2
/// artifact is intentional: callers can inspect and preserve V1 evidence
/// without mistaking a placeholder for verified semantics.
pub fn preview_v1_to_v2(
    value: &Value,
    kind: LegacyArtifactKindV2,
) -> Result<MigrationPreviewV2, String> {
    let snapshot = capture_v1_artifact(value, kind.clone())?;
    let plan = match kind {
        LegacyArtifactKindV2::DocumentIr => inspect_document_v1(value),
        LegacyArtifactKindV2::AuthoringIr => inspect_authoring_v1(value),
    };
    Ok(MigrationPreviewV2 {
        snapshot,
        plan,
        v2_artifact: None,
    })
}

/// Inspect old job artifacts without routing the old UI through V2.
///
/// Missing V2 directories or metadata are not errors. This is what lets an
/// existing V1 job open unchanged while exposing an explicit read-only
/// migration boundary for a future UI.
pub fn inspect_legacy_job_artifacts(
    root: &Path,
    job_id: &str,
) -> Result<LegacyJobCompatibilityV2, String> {
    let dir = safe_job_dir(root, job_id)?;
    let document = read_json_opt(&dir.join("document-ir.json"))?
        .map(|value| preview_v1_to_v2(&value, LegacyArtifactKindV2::DocumentIr))
        .transpose()?;
    let authoring = read_json_opt(&dir.join("authoring-ir.json"))?
        .map(|value| preview_v1_to_v2(&value, LegacyArtifactKindV2::AuthoringIr))
        .transpose()?;
    Ok(LegacyJobCompatibilityV2 {
        schema_version: LEGACY_JOB_COMPATIBILITY_SCHEMA_VERSION.to_string(),
        job_id: job_id.to_string(),
        opened_read_only: true,
        document,
        authoring,
    })
}

fn expected_v1_schema(kind: &LegacyArtifactKindV2) -> &'static str {
    match kind {
        LegacyArtifactKindV2::DocumentIr => DOCUMENT_IR_V1_SCHEMA_VERSION,
        LegacyArtifactKindV2::AuthoringIr => AUTHORING_IR_V1_SCHEMA_VERSION,
    }
}

pub fn assert_schema_version(value: &Value, expected: &str) -> Result<(), String> {
    let actual = value
        .get("schemaVersion")
        .and_then(Value::as_str)
        .unwrap_or("missing");
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "schema_version_mismatch:expected={expected}:actual={actual}"
        ))
    }
}

fn blocked_plan(
    direction: MigrationDirectionV2,
    from_schema_version: &str,
    to_schema_version: &str,
    reason: &str,
) -> MigrationPlanV2 {
    MigrationPlanV2 {
        direction,
        from_schema_version: from_schema_version.to_string(),
        to_schema_version: to_schema_version.to_string(),
        disposition: MigrationDispositionV2::Blocked,
        review_required: true,
        preserved_artifact: true,
        blocking_reasons: vec![reason.to_string()],
        warnings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn v1_is_read_only_and_requires_review() {
        let plan = inspect_document_v1(&json!({"schemaVersion":"DocumentIRV1"}));
        assert_eq!(plan.disposition, MigrationDispositionV2::NeedsReview);
        assert!(plan.review_required);
        assert!(plan.preserved_artifact);
    }

    #[test]
    fn unknown_version_is_blocked() {
        let plan = inspect_authoring_v1(&json!({"schemaVersion":"Unknown"}));
        assert_eq!(plan.disposition, MigrationDispositionV2::Blocked);
        assert!(!plan.blocking_reasons.is_empty());
    }

    #[test]
    fn v2_to_v1_is_explicitly_outside_pr01() {
        let plan = inspect_v2_to_v1(
            &json!({"schemaVersion":"IeltsAuthoringIRV2"}),
            "ReadingAuthoringIRV1",
        );
        assert_eq!(plan.disposition, MigrationDispositionV2::Blocked);
        assert!(plan.preserved_artifact);
    }

    #[test]
    fn v1_preview_keeps_exact_artifact_and_marks_v2_unavailable() {
        let value = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "blocks": [{"text": "legacy"}]}]
        });
        let preview = preview_v1_to_v2(&value, LegacyArtifactKindV2::DocumentIr).unwrap();
        assert_eq!(preview.snapshot.artifact, value);
        assert!(preview.snapshot.read_only);
        assert_eq!(preview.snapshot.canonical_sha256.len(), 64);
        assert_eq!(
            preview.plan.disposition,
            MigrationDispositionV2::NeedsReview
        );
        assert!(preview.v2_artifact.is_none());
    }

    #[test]
    fn old_job_artifacts_are_readable_without_v2_directory_or_mutation() {
        let root = std::env::temp_dir().join(format!(
            "phase1-legacy-job-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let dir = root.join("jobs").join("legacy-job");
        fs::create_dir_all(&dir).unwrap();
        let document = json!({"schemaVersion":"DocumentIRV1","pages":[]});
        let authoring = json!({"schemaVersion":"ReadingAuthoringIRV1","groups":[]});
        fs::write(
            dir.join("document-ir.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("authoring-ir.json"),
            serde_json::to_vec(&authoring).unwrap(),
        )
        .unwrap();
        let inspected = inspect_legacy_job_artifacts(&root, "legacy-job").unwrap();
        assert!(inspected.opened_read_only);
        assert!(inspected.document.is_some());
        assert!(inspected.authoring.is_some());
        assert!(!dir.join("authoring").exists());
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(dir.join("document-ir.json")).unwrap())
                .unwrap(),
            document
        );
        let _ = fs::remove_dir_all(root);
    }
}
