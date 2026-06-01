use crate::diagnostics::load_diagnostics_settings;
use crate::job_store::{load_job, update_job};
use crate::source_review::source_review_status_for_job;
use crate::util::{
    job_dir, read_json, read_json_opt, remove_dir_if_exists, remove_file_if_exists, write_json,
};
use crate::{CommandResult, JobStatus, WorkflowStep};
use chrono::Utc;
use serde_json::{json, Value};
use std::{fs, path::Path};

fn source_summary_from_job(job: &crate::ImportJob) -> Value {
    json!(job
        .source_files
        .iter()
        .map(|source| json!({
            "fileId": source.file_id,
            "originalName": source.original_name,
            "fileType": source.file_type,
            "sha256": source.sha256,
            "sizeBytes": source.size_bytes,
            "role": source.role,
            "importedAt": source.imported_at.to_rfc3339()
        }))
        .collect::<Vec<_>>())
}

pub(crate) fn validation_summary(report: &Value) -> Value {
    json!({
        "passed": report.get("passed").cloned().unwrap_or(Value::Bool(false)),
        "runtime": report.get("runtime").cloned().unwrap_or(Value::Null),
        "layers": report.get("layers").cloned().unwrap_or_else(|| json!([])),
        "issueCount": report.get("issues").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "generatedAt": report.get("generatedAt").cloned().unwrap_or(Value::Null)
    })
}

pub(crate) fn write_authoring_project(
    root: &Path,
    job_id: &str,
    export_summary: Option<Value>,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let dir = job_dir(root, job_id);
    let authoring: Value = read_json(&dir.join("authoring-ir.json"))?;
    let source_review = source_review_status_for_job(root, job_id)?;
    let validation_report = read_json_opt(&dir.join("validation-report.json"))?;
    let project = json!({
        "schemaVersion": "AuthoringProjectV1",
        "job": {
            "jobId": job.job_id,
            "title": job.title,
            "category": job.category,
            "frequency": job.frequency,
            "tags": job.tags,
            "status": job.status,
            "currentStep": job.current_step,
            "createdAt": job.created_at.to_rfc3339(),
            "updatedAt": job.updated_at.to_rfc3339()
        },
        "authoringIr": authoring,
        "sourceSummary": source_summary_from_job(&job),
        "reviewSummary": {
            "sourceReview": source_review,
            "humanVerified": authoring.pointer("/audit/humanVerified").and_then(Value::as_bool).unwrap_or(false),
            "audit": authoring.get("audit").cloned().unwrap_or_else(|| json!({}))
        },
        "validationSummary": validation_report.as_ref().map(validation_summary).unwrap_or(Value::Null),
        "exportSummary": export_summary.unwrap_or(Value::Null),
        "updatedAt": Utc::now().to_rfc3339()
    });
    write_json(&dir.join("authoring-project.json"), &project)?;
    Ok(project)
}

fn cleanup_parser_cache_for_job(root: &Path, job_id: &str) -> CommandResult<()> {
    let parser_cache = root.join("cache").join("parser");
    if !parser_cache.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&parser_cache)
        .map_err(|error| format!("read_parser_cache:{}:{}", parser_cache.display(), error))?
    {
        let entry =
            entry.map_err(|error| format!("read_parser_cache_entry:{}:{}", job_id, error))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(job_id) {
            let path = entry.path();
            if path.is_dir() {
                remove_dir_if_exists(&path)?;
            } else {
                remove_file_if_exists(&path)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn cleanup_transient_job_artifacts(
    root: &Path,
    job_id: &str,
    export_summary: Value,
) -> CommandResult<Value> {
    let dir = job_dir(root, job_id);
    let diagnostics = load_diagnostics_settings(root)?;
    let project = write_authoring_project(root, job_id, Some(export_summary.clone()))?;
    if diagnostics.keep_full_process_artifacts {
        let summary = json!({
            "schemaVersion": "CleanupSummaryV1",
            "jobId": job_id,
            "cleaned": false,
            "retainedFullProcessArtifacts": true,
            "message": "Developer diagnostics retention is enabled; full process artifacts were kept.",
            "exportSummary": export_summary,
            "generatedAt": Utc::now().to_rfc3339()
        });
        write_json(&dir.join("cleanup-summary.json"), &summary)?;
        return Ok(summary);
    }

    for relative in ["cache", "preview", "llm-suggestions"] {
        remove_dir_if_exists(&dir.join(relative))?;
    }
    for relative in [
        "document-ir.json",
        "split-candidates.json",
        "pipeline-report.json",
        "pipeline-report-summary.json",
        "llm-last-suggestion.json",
        "llm-calls.jsonl",
        "vision-transcription-output.json",
        "vision-transcription.txt",
        "manual-transcription.txt",
        "validation-report.json",
        "publish-readiness-report.json",
    ] {
        remove_file_if_exists(&dir.join(relative))?;
    }
    cleanup_parser_cache_for_job(root, job_id)?;

    let summary = json!({
        "schemaVersion": "CleanupSummaryV1",
        "jobId": job_id,
        "cleaned": true,
        "retainedFullProcessArtifacts": false,
        "message": "中间文件已自动清理，已保留可编辑题目稿。",
        "kept": ["job.json", "authoring-ir.json", "authoring-project.json", "source-review.json", "uploads/", "exports/"],
        "removed": ["cache/", "preview/", "document-ir.json", "split-candidates.json", "pipeline-report.json", "llm-suggestions/", "llm-calls.jsonl", "vision/manual transcription temp files", "validation/runtime intermediate reports"],
        "exportSummary": export_summary,
        "projectSchemaVersion": project.get("schemaVersion").cloned().unwrap_or(Value::Null),
        "generatedAt": Utc::now().to_rfc3339()
    });
    update_job(root, job_id, |job| {
        job.status = JobStatus::Cleaned;
        job.current_step = WorkflowStep::Export;
    })?;
    Ok(summary)
}

pub(crate) fn minimize_process_artifacts_after_authoring(
    root: &Path,
    job_id: &str,
    reason: &str,
) -> CommandResult<Value> {
    let dir = job_dir(root, job_id);
    let diagnostics = load_diagnostics_settings(root)?;
    let project = write_authoring_project(root, job_id, None)?;
    if diagnostics.keep_full_process_artifacts {
        return Ok(json!({
            "schemaVersion": "ArtifactMinimizationV1",
            "jobId": job_id,
            "minimized": false,
            "retainedFullProcessArtifacts": true,
            "reason": reason,
            "message": "Developer diagnostics retention is enabled; full process artifacts were kept.",
            "generatedAt": Utc::now().to_rfc3339()
        }));
    }

    for relative in ["cache", "preview", "llm-suggestions"] {
        remove_dir_if_exists(&dir.join(relative))?;
    }
    for relative in [
        "document-ir.json",
        "split-candidates.json",
        "pipeline-report.json",
        "validation-report.json",
        "publish-readiness-report.json",
        "llm-last-suggestion.json",
        "llm-calls.jsonl",
        "vision-transcription-output.json",
        "vision-transcription.txt",
        "manual-transcription.txt",
    ] {
        remove_file_if_exists(&dir.join(relative))?;
    }
    cleanup_parser_cache_for_job(root, job_id)?;

    Ok(json!({
        "schemaVersion": "ArtifactMinimizationV1",
        "jobId": job_id,
        "minimized": true,
        "retainedFullProcessArtifacts": false,
        "reason": reason,
        "message": "已压缩为最小可编辑态，仅保留 authoring-ir、authoring-project、source-review 与作业元数据。",
        "kept": ["job.json", "authoring-ir.json", "authoring-project.json", "source-review.json", "uploads/"],
        "removed": ["document-ir.json", "split-candidates.json", "pipeline-report*.json", "cache/", "preview/", "llm-suggestions/", "llm-calls.jsonl", "vision/manual transcription temp files"],
        "projectSchemaVersion": project.get("schemaVersion").cloned().unwrap_or(Value::Null),
        "generatedAt": Utc::now().to_rfc3339()
    }))
}
