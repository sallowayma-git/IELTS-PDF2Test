use crate::authoring_review::authoring_review_issues;
use crate::authoring_validation::{
    merge_sidecar_validation, merge_validation_issues, validate_authoring,
};
use crate::environment::{
    command_failure, find_sidecar, node_validator_diagnostics_enabled, runtime_gate_strict_mode,
};
use crate::export_artifacts::build_reading_asset_bundle;
use crate::job_store::load_job;
use crate::reading_source::reading_source;
use crate::source_review::{source_review_issues, source_review_status};
use crate::util::{job_dir, read_json_opt, validate_path_segment, write_json, write_text};
use crate::validator::json_issue;
use crate::{CommandResult, JobStatus};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

pub(crate) fn validate_with_node_sidecar(
    root: &Path,
    job_id: &str,
    source: &Value,
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/node-validator/validate-reading-source.mjs")
        .ok_or_else(|| "node_validator_sidecar_missing".to_string())?;
    let input_path = job_dir(root, job_id)
        .join("cache")
        .join("reading-source-for-validation.json");
    write_json(&input_path, source)?;
    let output = Command::new("node")
        .arg(&script)
        .arg(&input_path)
        .output()
        .map_err(|error| format!("node_validator_spawn_failed:{}:{}", script.display(), error))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = serde_json::from_str::<Value>(&stdout)
        .map_err(|error| format!("node_validator_json_failed:{}:{}", error, stdout.trim()))?;
    if !output.status.success() && parsed.get("passed").and_then(Value::as_bool) != Some(false) {
        return Err(command_failure("node-validator", &output));
    }
    Ok(parsed)
}

pub(crate) fn validate_preview_with_node_sidecar(
    root: &Path,
    job_id: &str,
    preview_dir: &Path,
    exam_id: &str,
    unified_html_path: Option<&Path>,
    unified_python_path: Option<&Path>,
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/preview-e2e/preview-e2e.mjs")
        .ok_or_else(|| "preview_e2e_sidecar_missing".to_string())?;
    let mut command = Command::new("node");
    command
        .arg(&script)
        .arg("--preview-dir")
        .arg(preview_dir)
        .arg("--exam-id")
        .arg(exam_id)
        .arg("--job-id")
        .arg(job_id);
    if let Some(path) = unified_html_path {
        command.env("EPIC8_UNIFIED_HTML_PATH", path);
    }
    if let Some(path) = unified_python_path {
        command.env("EPIC8_UNIFIED_PYTHON", path);
    }
    let output = command
        .output()
        .map_err(|error| format!("preview_e2e_spawn_failed:{}:{}", script.display(), error))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = serde_json::from_str::<Value>(&stdout)
        .map_err(|error| format!("preview_e2e_json_failed:{}:{}", error, stdout.trim()))?;
    if !output.status.success() && parsed.get("passed").and_then(Value::as_bool) != Some(false) {
        return Err(command_failure("preview-e2e", &output));
    }
    let output_path = job_dir(root, job_id)
        .join("preview")
        .join("preview-e2e-report.json");
    write_json(&output_path, &parsed)?;
    Ok(parsed)
}

pub(crate) fn preview_assets_for_source(
    root: &Path,
    job_id: &str,
    source: &Value,
) -> CommandResult<(String, PathBuf, String, String, Value)> {
    validate_path_segment("job_id", job_id)?;
    let bundle = build_reading_asset_bundle(source)?;
    let preview_dir = job_dir(root, job_id).join("preview");
    write_text(
        &preview_dir.join(format!("{}.js", bundle.exam_id)),
        &bundle.wrapper_js,
    )?;
    write_text(&preview_dir.join("manifest.js"), &bundle.manifest_js)?;
    let assets = json!({"examId": bundle.exam_id, "manifestPath": preview_dir.join("manifest.js").to_string_lossy(), "scriptPath": preview_dir.join(format!("{}.js", bundle.exam_id)).to_string_lossy(), "previewUrl": format!("tauri-local://preview/{}", bundle.source.get("examId").and_then(Value::as_str).unwrap_or("local-authoring-exam")), "source": bundle.source, "wrapperJs": bundle.wrapper_js, "manifestJs": bundle.manifest_js});
    write_json(&preview_dir.join("preview-assets.json"), &assets)?;
    Ok((
        bundle.exam_id,
        preview_dir,
        bundle.wrapper_js,
        bundle.manifest_js,
        assets,
    ))
}

pub(crate) fn validate_for_runtime_gate(
    root: &Path,
    job_id: &str,
    ir: &Value,
    require_static_runtime_gate: bool,
) -> CommandResult<Value> {
    let source = reading_source(ir);
    let mut report = validate_authoring(job_id, Some(ir));
    if report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let _ = preview_assets_for_source(root, job_id, &source)?;
        if require_static_runtime_gate && !runtime_gate_strict_mode() {
            merge_validation_issues(
                &mut report,
                vec![json!({
                    "issueId": format!("issue-{}", Uuid::new_v4().simple()),
                    "severity": "warning",
                    "layer": "RuntimePreview",
                    "path": "runtime.staticGate",
                    "message": "Production static runtime gate was explicitly disabled by EPIC8_RUNTIME_GATE_STRICT.",
                    "fixHint": "Enable EPIC8_RUNTIME_GATE_STRICT for production exports."
                })],
            );
        }
        if let Some(obj) = report.as_object_mut() {
            obj.insert(
                "runtime".to_string(),
                json!({
                    "mode": "static-rust",
                    "adapter": "rust-static-contract",
                    "diagnosticE2e": "not-run",
                    "fallbackReason": null
                }),
            );
        }
    }
    write_json(
        &job_dir(root, job_id).join("validation-report.json"),
        &report,
    )?;
    Ok(report)
}

pub(crate) fn run_node_validator_diagnostic(
    root: &Path,
    job_id: &str,
    report: &mut Value,
    source: &Value,
) {
    if !node_validator_diagnostics_enabled() {
        return;
    }
    match validate_with_node_sidecar(root, job_id, source) {
        Ok(mut sidecar_report) => {
            if let Some(obj) = sidecar_report.as_object_mut() {
                obj.insert("replaceExistingLayers".to_string(), json!(false));
            }
            merge_sidecar_validation(report, sidecar_report);
        }
        Err(error) => {
            merge_validation_issues(
                report,
                vec![json!({
                    "issueId": format!("issue-{}", Uuid::new_v4().simple()),
                    "severity": "warning",
                    "layer": "ReadingExamSourceV1",
                    "path": "$",
                    "message": format!("Node validator diagnostic unavailable; Rust built-in ReadingExamSourceV1/DOM validation was used: {}", error),
                    "fixHint": "Set up Node.js only if development parity diagnostics are needed."
                })],
            );
        }
    }
}

pub(crate) fn publish_readiness_gate(
    root: &Path,
    job_id: &str,
    ir: &Value,
    mut runtime_report: Value,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let dir = job_dir(root, job_id);
    let document_ir = read_json_opt(&dir.join("document-ir.json"))?;
    let source_review = source_review_status(root, job_id, document_ir.as_ref())?;
    let human_verified = ir.pointer("/audit/humanVerified").and_then(Value::as_bool) == Some(true);
    let mut issues = Vec::new();

    if job.status == JobStatus::NeedsReview {
        issues.push(json_issue(
            "AuthoringIR",
            "$.job.status",
            "Job is still marked NeedsReview; complete manual review before publish",
        ));
    }
    issues.extend(source_review_issues(&source_review));
    if !human_verified {
        issues.push(json_issue(
            "AuthoringIR",
            "$.audit.humanVerified",
            "All questions and answers must be human verified before publish",
        ));
    }
    issues.extend(authoring_review_issues(ir));

    merge_validation_issues(&mut runtime_report, issues);
    write_json(&dir.join("publish-readiness-report.json"), &runtime_report)?;
    Ok(runtime_report)
}
