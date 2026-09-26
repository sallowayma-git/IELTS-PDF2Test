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

/// 一个解析缓存条目是否属于这个条目。
///
/// 命名约定是 `<id>-<rest>`（`<jobId>-document-ir.json`、
/// `<jobId>-answer-<fileId>-document-ir.json`），也可能按**源文件 sha** 内容寻址。
/// 所以判归属必须**精确**：名字等于身份本身，或以 `<身份>-` 开头。
///
/// 绝不能只做 `starts_with(身份)`：迁移把 `jobs/` 下的**目录名**当 item id
/// （见 `library::migration::migrate_existing_items`），而历史目录名长度不一，
/// 于是 `job-1` 会匹配上 `job-10-document-ir.json` —— 清理一个条目会删掉别人的解析产物。
fn cache_entry_belongs_to(name: &str, identities: &[String]) -> bool {
    identities.iter().any(|identity| {
        !identity.is_empty()
            && (name == identity.as_str() || name.starts_with(&format!("{identity}-")))
    })
}

/// 删掉 `<appData>/cache/parser/` 里属于这个条目的解析产物（原文抽取结果）。
///
/// 调用方有两处：过程产物清理（`cleanup_transient_job_artifacts`）与**发布后**的
/// 源产物清理（`library::final_version::purge_source_artifacts`）。后者此前漏了，
/// 于是发布后这道题的解析缓存仍留在磁盘上，与「发布后只保留可编辑最终版」相违。
pub(crate) fn cleanup_parser_cache_for_job(root: &Path, job_id: &str) -> CommandResult<()> {
    let parser_cache = root.join("cache").join("parser");
    if !parser_cache.exists() {
        return Ok(());
    }
    // 归属身份：这个条目自己的 id，加上它**自己**那些源文件的内容 sha。
    // 别的条目的源 sha 是别人的内容，不在这里面。
    let mut identities: Vec<String> = vec![job_id.to_string()];
    if let Ok(job) = load_job(root, job_id) {
        for source in &job.source_files {
            let sha = source.sha256.trim().to_ascii_lowercase();
            if !sha.is_empty() && !identities.contains(&sha) {
                identities.push(sha);
            }
        }
    }
    for entry in fs::read_dir(&parser_cache)
        .map_err(|error| format!("read_parser_cache:{}:{}", parser_cache.display(), error))?
    {
        let entry =
            entry.map_err(|error| format!("read_parser_cache_entry:{}:{}", job_id, error))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if cache_entry_belongs_to(&name, &identities) {
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
        "vision-answer-output.json",
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
        "vision-answer-output.json",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_root() -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("cleanup-parser-{}", uuid::Uuid::new_v4().simple()));
        crate::util::ensure_app_dirs(&root).unwrap();
        root
    }

    /// 解析缓存按**精确**条目归属清理：一个 id 是另一个的前缀时，绝不能顺手删掉别人的。
    ///
    /// 旧实现用 `name.starts_with(job_id)` 判归属。迁移把 `jobs/` 下的**目录名**当 item id
    /// （见 `library::migration::migrate_existing_items`），而历史目录名长度不一，于是
    /// `job-1` 会匹配上 `job-10-document-ir.json` —— 清理一个条目会把另一个条目的解析产物
    /// 一起删掉，而对方可能正要导出。共享资产目录不属于任何单个条目，更不能碰。
    #[test]
    fn parser_cache_cleanup_matches_exact_job_identity_and_never_a_prefix() {
        let root = temp_root();
        let cache = root.join("cache").join("parser");
        fs::create_dir_all(&cache).unwrap();
        // 本条目自己的产物：`<jobId>-…` 文件与同名目录。
        fs::write(cache.join("job-1-document-ir.json"), b"{}").unwrap();
        fs::write(cache.join("job-1-answer-src-2-document-ir.json"), b"{}").unwrap();
        fs::create_dir_all(cache.join("job-1")).unwrap();
        // 别人的产物：名字以 `job-1` 开头，但**不是**同一个条目。
        fs::write(cache.join("job-10-document-ir.json"), b"{}").unwrap();
        fs::write(cache.join("job-11-document-ir.json"), b"{}").unwrap();
        fs::create_dir_all(cache.join("job-10")).unwrap();
        // 共享资产目录：不属于任何单个条目。
        fs::create_dir_all(cache.join("image-assets")).unwrap();

        cleanup_parser_cache_for_job(&root, "job-1").unwrap();

        assert!(
            !cache.join("job-1-document-ir.json").exists(),
            "自己的产物必须删掉"
        );
        assert!(!cache.join("job-1-answer-src-2-document-ir.json").exists());
        assert!(!cache.join("job-1").exists());
        assert!(
            cache.join("job-10-document-ir.json").exists(),
            "job-10 不是 job-1：前缀匹配会删掉别人的解析产物"
        );
        assert!(cache.join("job-10").is_dir(), "job-10 不是 job-1");
        assert!(
            cache.join("job-11-document-ir.json").exists(),
            "job-11 不是 job-1"
        );
        assert!(
            cache.join("image-assets").is_dir(),
            "共享资产目录不属于任何单个条目"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// 缓存条目也可能按**源文件 sha** 命名（内容寻址）。那一类同样按精确 sha 归属，
    /// 而且只清这个条目自己的源文件——别的 sha 是别人的内容。
    #[test]
    fn parser_cache_cleanup_matches_the_jobs_own_source_sha_exactly() {
        let root = temp_root();
        let cache = root.join("cache").join("parser");
        fs::create_dir_all(&cache).unwrap();
        let mut job = crate::job_store::make_job(crate::CreateJobInput::default());
        let mine = "a".repeat(64);
        let theirs = "b".repeat(64);
        job.source_files = vec![crate::SourceFile {
            file_id: "src-1".to_string(),
            original_name: "paper.pdf".to_string(),
            stored_name: "stored-paper.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: mine.clone(),
            size_bytes: 1,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        }];
        crate::job_store::save_job(&root, &job).unwrap();

        fs::write(cache.join(format!("{mine}-document-ir.json")), b"{}").unwrap();
        fs::write(cache.join(format!("{mine}00-document-ir.json")), b"{}").unwrap();
        fs::write(cache.join(format!("{theirs}-document-ir.json")), b"{}").unwrap();

        cleanup_parser_cache_for_job(&root, &job.job_id).unwrap();

        assert!(
            !cache.join(format!("{mine}-document-ir.json")).exists(),
            "本条目源文件的内容寻址缓存必须清掉"
        );
        assert!(
            cache.join(format!("{mine}00-document-ir.json")).exists(),
            "sha 前缀不是同一个内容"
        );
        assert!(
            cache.join(format!("{theirs}-document-ir.json")).exists(),
            "别的源文件（别人的内容）不能删"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
