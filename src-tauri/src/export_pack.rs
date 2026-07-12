use crate::{
    cleanup::{
        cleanup_transient_job_artifacts, minimize_process_artifacts_after_authoring,
        validation_summary,
    },
    export_artifacts::{build_manifest, build_reading_asset_bundle, safe_exam_id},
    job_store::update_job,
    reading_source::reading_source,
    runtime_validation::{publish_readiness_gate, validate_for_runtime_gate},
    util::{job_dir, read_json, validate_path_segment, write_json, write_text},
    CommandResult, JobStatus, WorkflowStep,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationPolicy {
    Strict,
    Force,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExportValidationOptions {
    policy: ValidationPolicy,
}

impl ExportValidationOptions {
    pub(crate) fn strict() -> Self {
        Self {
            policy: ValidationPolicy::Strict,
        }
    }

    pub(crate) fn from_input(input: &Value) -> CommandResult<Self> {
        Self::from_policy(input.get("validationPolicy").and_then(Value::as_str))
    }

    pub(crate) fn from_policy(policy: Option<&str>) -> CommandResult<Self> {
        match policy.unwrap_or("strict") {
            "strict" => Ok(Self::strict()),
            "force" => Ok(Self {
                policy: ValidationPolicy::Force,
            }),
            other => Err(format!("invalid_validation_policy:{other}")),
        }
    }

    pub(crate) fn policy_name(self) -> &'static str {
        match self.policy {
            ValidationPolicy::Strict => "strict",
            ValidationPolicy::Force => "force",
        }
    }

    pub(crate) fn should_block(self, report: &Value) -> bool {
        self.policy == ValidationPolicy::Strict && !validation_report_passed(report)
    }

    pub(crate) fn validation_overridden(self, report: &Value) -> bool {
        self.policy == ValidationPolicy::Force && !validation_report_passed(report)
    }

    pub(crate) fn ignored_issues(self, job_id: &str, report: &Value) -> Vec<Value> {
        if !self.validation_overridden(report) {
            return Vec::new();
        }
        report
            .get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|issue| {
                issue
                    .get("severity")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    == "error"
            })
            .map(|issue| {
                json!({
                    "jobId": job_id,
                    "issueId": issue.get("issueId").cloned().unwrap_or(Value::Null),
                    "severity": issue.get("severity").cloned().unwrap_or_else(|| json!("error")),
                    "layer": issue.get("layer").cloned().unwrap_or(Value::Null),
                    "path": issue.get("path").cloned().unwrap_or(Value::Null),
                    "message": issue.get("message").cloned().unwrap_or(Value::Null),
                    "fixHint": issue.get("fixHint").cloned().unwrap_or(Value::Null)
                })
            })
            .collect()
    }
}

fn validation_report_passed(report: &Value) -> bool {
    report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn persist_export_validation_report(
    root: &Path,
    job_id: &str,
    report: &Value,
) -> CommandResult<()> {
    write_json(
        &job_dir(root, job_id).join("validation-report.json"),
        report,
    )
}

pub(crate) fn export_reading_assets_core(
    root: &Path,
    job_id: &str,
    export_dir: &str,
    require_static_runtime_gate: bool,
) -> CommandResult<Value> {
    export_reading_assets_with_options_core(
        root,
        job_id,
        export_dir,
        require_static_runtime_gate,
        ExportValidationOptions::strict(),
    )
}

pub(crate) fn export_reading_assets_with_options_core(
    root: &Path,
    job_id: &str,
    export_dir: &str,
    require_static_runtime_gate: bool,
    options: ExportValidationOptions,
) -> CommandResult<Value> {
    validate_path_segment("job_id", job_id)?;
    let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let report = validate_for_runtime_gate(root, job_id, &ir, require_static_runtime_gate)?;
    let report = publish_readiness_gate(root, job_id, &ir, report)?;
    persist_export_validation_report(root, job_id, &report)?;
    let validation_overridden = options.validation_overridden(&report);
    let ignored_issues = options.ignored_issues(job_id, &report);
    let ignored_issue_count = ignored_issues.len() as u64;
    if options.should_block(&report) {
        let _ =
            minimize_process_artifacts_after_authoring(root, job_id, "export_publish_gate_failed")?;
        return Err(format!(
            "export_validation_failed:{}",
            serde_json::to_string(&report).unwrap_or_default()
        ));
    }
    let source = reading_source(&ir);
    let bundle = build_reading_asset_bundle(&source)?;
    let out_dir = if export_dir.starts_with("local://") {
        job_dir(root, job_id).join("exports")
    } else {
        PathBuf::from(export_dir)
    };
    fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;
    write_json(&out_dir.join(format!("{}.json", bundle.exam_id)), &source)?;
    write_text(
        &out_dir.join(format!("{}.js", bundle.exam_id)),
        &bundle.wrapper_js,
    )?;
    write_text(&out_dir.join("manifest.js"), &bundle.manifest_js)?;
    write_json(&out_dir.join("validation-report.json"), &report)?;
    let export_summary = json!({
        "type": "reading-assets",
        "examId": bundle.exam_id,
        "outputDir": out_dir.to_string_lossy(),
        "files": [format!("{}.json", bundle.exam_id), format!("{}.js", bundle.exam_id), "manifest.js".to_string(), "validation-report.json".to_string()],
        "validationSummary": validation_summary(&report),
        "validationPolicy": options.policy_name(),
        "validationOverridden": validation_overridden,
        "ignoredIssueCount": ignored_issue_count,
        "ignoredIssues": ignored_issues.clone(),
        "exportedAt": Utc::now().to_rfc3339()
    });
    update_job(root, job_id, |job| {
        job.status = JobStatus::Exported;
        job.current_step = WorkflowStep::Export;
    })?;
    let cleanup = cleanup_transient_job_artifacts(root, job_id, export_summary.clone())?;
    Ok(json!({
        "examId": bundle.exam_id,
        "files":[{"name":format!("{}.json", bundle.exam_id),"content":serde_json::to_string_pretty(&source).unwrap_or_default()},{"name":format!("{}.js", bundle.exam_id),"content":bundle.wrapper_js},{"name":"manifest.js","content":bundle.manifest_js}],
        "outputDir": out_dir.to_string_lossy(),
        "validationPolicy": options.policy_name(),
        "validationOverridden": validation_overridden,
        "ignoredIssueCount": ignored_issue_count,
        "ignoredIssues": ignored_issues,
        "exportSummary": export_summary,
        "cleanup": cleanup
    }))
}
pub(crate) fn export_reading_js_core(
    root: &Path,
    input: &Value,
    require_static_runtime_gate: bool,
) -> CommandResult<Value> {
    let options = ExportValidationOptions::from_input(input)?;
    export_reading_js_with_options_core(root, input, require_static_runtime_gate, options)
}

fn export_reading_js_with_options_core(
    root: &Path,
    input: &Value,
    require_static_runtime_gate: bool,
    options: ExportValidationOptions,
) -> CommandResult<Value> {
    let job_ids = input
        .get("jobIds")
        .and_then(Value::as_array)
        .ok_or_else(|| "js_export_requires_job_ids".to_string())?
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if job_ids.is_empty() {
        return Err("js_export_requires_at_least_one_job".to_string());
    }

    let export_dir = input
        .get("exportDir")
        .and_then(Value::as_str)
        .unwrap_or("local://exports");
    let mut sources = Vec::with_capacity(job_ids.len());
    let mut wrappers = Vec::with_capacity(job_ids.len());
    let mut exam_ids = Vec::with_capacity(job_ids.len());
    let mut cleanup = Vec::with_capacity(job_ids.len());
    let mut validation_overridden = false;
    let mut ignored_issues = Vec::new();

    let out_dir = if export_dir.starts_with("local://") {
        if job_ids.len() == 1 {
            job_dir(root, job_ids[0]).join("exports").join("js")
        } else {
            root.join("exports").join("reading-exams")
        }
    } else {
        PathBuf::from(export_dir)
    };
    fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;

    for job_id in &job_ids {
        validate_path_segment("job_id", job_id)?;
        let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
        let report = validate_for_runtime_gate(root, job_id, &ir, require_static_runtime_gate)?;
        let report = publish_readiness_gate(root, job_id, &ir, report)?;
        persist_export_validation_report(root, job_id, &report)?;
        validation_overridden |= options.validation_overridden(&report);
        ignored_issues.extend(options.ignored_issues(job_id, &report));
        if options.should_block(&report) {
            let _ = minimize_process_artifacts_after_authoring(
                root,
                job_id,
                "js_export_publish_gate_failed",
            )?;
            return Err(format!(
                "js_export_validation_failed:{}:{}",
                job_id,
                serde_json::to_string(&report).unwrap_or_default()
            ));
        }

        let source = reading_source(&ir);
        let exam_id = safe_exam_id(&source)?;
        let wrapper_js = build_reading_asset_bundle(&source)?.wrapper_js;
        write_text(&out_dir.join(format!("{}.js", exam_id)), &wrapper_js)?;
        exam_ids.push(exam_id.clone());
        wrappers.push(json!({
            "name": format!("{}.js", exam_id),
            "content": wrapper_js
        }));
        sources.push(source);

        update_job(root, job_id, |job| {
            job.status = JobStatus::Exported;
            job.current_step = WorkflowStep::Export;
        })?;
    }

    let manifest_js = build_manifest(&sources)?;
    write_text(&out_dir.join("manifest.js"), &manifest_js)?;

    let mode = if job_ids.len() > 1 { "batch" } else { "single" };
    let ignored_issue_count = ignored_issues.len() as u64;
    let export_summary = json!({
        "type": "reading-js",
        "mode": mode,
        "jobIds": job_ids,
        "examIds": exam_ids,
        "outputDir": out_dir.to_string_lossy(),
        "files": exam_ids.iter().map(|exam_id| format!("{}.js", exam_id)).chain(std::iter::once("manifest.js".to_string())).collect::<Vec<_>>(),
        "validationPolicy": options.policy_name(),
        "validationOverridden": validation_overridden,
        "ignoredIssueCount": ignored_issue_count,
        "ignoredIssues": ignored_issues.clone(),
        "exportedAt": Utc::now().to_rfc3339()
    });

    for job_id in &job_ids {
        cleanup.push(cleanup_transient_job_artifacts(
            root,
            job_id,
            export_summary.clone(),
        )?);
    }

    wrappers.push(json!({
        "name": "manifest.js",
        "content": manifest_js
    }));

    Ok(json!({
        "mode": mode,
        "jobIds": job_ids,
        "examIds": exam_ids,
        "files": wrappers,
        "outputDir": out_dir.to_string_lossy(),
        "validationPolicy": options.policy_name(),
        "validationOverridden": validation_overridden,
        "ignoredIssueCount": ignored_issue_count,
        "ignoredIssues": ignored_issues,
        "exportSummary": export_summary,
        "cleanup": cleanup
    }))
}
