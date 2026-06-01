use crate::{
    authoring_review::authoring_review_issues,
    authoring_validation::{merge_sidecar_validation, validate_authoring},
    environment::{resolve_external_unified_html, resolve_external_unified_python},
    job_store::update_job,
    reading_source::reading_source,
    runtime_validation::{
        preview_assets_for_source, publish_readiness_gate, run_node_validator_diagnostic,
        validate_for_runtime_gate, validate_preview_with_node_sidecar,
    },
    source_review::{source_review_issues, source_review_status_for_job},
    util::{job_dir, read_json, read_json_opt, write_json},
    workflow_state::{apply_preview_e2e_job_state, update_validation_job_state},
    CommandResult, JobStatus, WorkflowStep,
};
use serde_json::{json, Value};
use std::path::Path;
use uuid::Uuid;

pub(crate) fn validate_authoring_ir_core(root: &Path, job_id: &str) -> CommandResult<Value> {
    let authoring = read_json_opt(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let mut report = validate_authoring(job_id, authoring.as_ref());
    if let Some(ir) = authoring.as_ref() {
        let source = reading_source(ir);
        run_node_validator_diagnostic(root, job_id, &mut report, &source);
    }
    let source_review = source_review_status_for_job(root, job_id)?;
    let source_review_issue_count = source_review_issues(&source_review).len() as u32;
    write_json(
        &job_dir(root, job_id).join("validation-report.json"),
        &report,
    )?;
    update_validation_job_state(root, job_id, &report, source_review_issue_count)?;
    Ok(report)
}

pub(crate) fn generate_preview_assets_core(root: &Path, job_id: &str) -> CommandResult<Value> {
    let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let report = validate_for_runtime_gate(root, job_id, &ir, false)?;
    if !report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        update_job(root, job_id, |job| {
            job.status = JobStatus::NeedsReview;
            job.current_step = WorkflowStep::Authoring;
        })?;
        return Err(format!(
            "preview_validation_failed:{}",
            serde_json::to_string(&report).unwrap_or_default()
        ));
    }
    let source = reading_source(&ir);
    let (_, _, _, _, assets) = preview_assets_for_source(root, job_id, &source)?;
    let human_verified = ir.pointer("/audit/humanVerified").and_then(Value::as_bool) == Some(true);
    let mut review_issues = authoring_review_issues(&ir);
    let source_review = source_review_status_for_job(root, job_id)?;
    review_issues.extend(source_review_issues(&source_review));
    update_job(root, job_id, |job| {
        job.status = if review_issues.is_empty() && human_verified {
            JobStatus::DraftSaved
        } else {
            JobStatus::NeedsReview
        };
        job.current_step = WorkflowStep::Preview;
        job.issue_counts.needs_review = review_issues.len() as u32;
    })?;
    Ok(assets)
}

pub(crate) fn run_preview_e2e_core(root: &Path, job_id: &str) -> CommandResult<Value> {
    let authoring = read_json_opt(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let (static_report, diagnostic_report) = if let Some(ir) = authoring.as_ref() {
        let source = reading_source(ir);
        let static_report = validate_for_runtime_gate(root, job_id, ir, false)?;
        let mut diagnostic_report = static_report.clone();
        if static_report
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let (exam_id, preview_dir, _, _, _) = preview_assets_for_source(root, job_id, &source)?;
            let unified_html_path = resolve_external_unified_html();
            let unified_python_path = resolve_external_unified_python();
            match validate_preview_with_node_sidecar(
                root,
                job_id,
                &preview_dir,
                &exam_id,
                unified_html_path.as_deref(),
                unified_python_path.as_deref(),
            ) {
                Ok(runtime_report) => {
                    merge_sidecar_validation(&mut diagnostic_report, runtime_report)
                }
                Err(error) => merge_sidecar_validation(
                    &mut diagnostic_report,
                    json!({
                        "layers": [{"layer":"RuntimePreview"}],
                        "issues": [{
                            "issueId": format!("issue-{}", Uuid::new_v4().simple()),
                            "severity": "error",
                            "layer": "RuntimePreview",
                            "path": "runtime.execution",
                            "message": format!("Preview E2E diagnostic unavailable: {}", error),
                            "fixHint": "Install Node.js and configure EPIC8_UNIFIED_HTML_PATH/EPIC8_UNIFIED_PYTHON only for explicit runtime diagnostics."
                        }]
                    }),
                ),
            }
        }
        (static_report, diagnostic_report)
    } else {
        let report = validate_authoring(job_id, None);
        write_json(
            &job_dir(root, job_id).join("validation-report.json"),
            &report,
        )?;
        (report.clone(), report)
    };
    let static_report_passed = static_report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let readiness_passed = if static_report_passed {
        if let Some(ir) = authoring.as_ref() {
            publish_readiness_gate(root, job_id, ir, static_report.clone())?
                .get("passed")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };
    write_json(
        &job_dir(root, job_id).join("validation-report.json"),
        &diagnostic_report,
    )?;
    apply_preview_e2e_job_state(root, job_id, &static_report, readiness_passed)?;
    Ok(diagnostic_report)
}
