use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}
