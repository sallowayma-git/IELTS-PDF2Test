//! 写作题库导出管线（镜像 export_nas_library.rs，但极简）。
//!
//! 产出 NAS 端 NasJsDirectWritingAssetProvider 能识别的：
//!   writing-exams/manifest.js   (window.__WRITING_EXAM_MANIFEST__ = { task1:{...}, task2:{...} })
//!   writing-exams/task1.js       (__WRITING_EXAM_DATA__.register("task1", {...}))
//!   writing-exams/task2.js       (__WRITING_EXAM_DATA__.register("task2", {...}))
//!
//! 关键差异 vs 阅读：
//!   - 单包双题（task1+task2 固定两题），非多 job 组套
//!   - manifest key 是 taskType（"task1"/"task2"），非 examId（NAS provider 按 taskType 查）
//!   - wrapper register key 也是 taskType
//!   - 无 runtime gate（写作无 passage/answerKey，仅校验 promptText 非空 + taskType 合法）

use crate::export_nas_library::{nas_writing_exams_dir, normalize_nas_library_root};
use crate::util::{read_json, safe_writing_job_dir, validate_path_segment, write_text};
use crate::writing_store::{writing_source, WritingJob, WritingJobStatus};
use crate::CommandResult;
use chrono::Utc;
use serde_json::{json, Value};
use std::{collections::HashSet, fs, path::Path};

const WRITING_TASK_TYPES: [&str; 2] = ["task1", "task2"];

/// 生成单题 wrapper JS（IIFE 调 __WRITING_EXAM_DATA__.register(taskType, payload)）。
fn build_writing_wrapper(source: &Value) -> CommandResult<String> {
    let task_type = source
        .get("taskType")
        .and_then(Value::as_str)
        .unwrap_or("task1");
    let key_json = serde_json::to_string(task_type).map_err(|error| error.to_string())?;
    let source_json = serde_json::to_string_pretty(source).map_err(|error| error.to_string())?;
    Ok(format!(
        "(function registerWritingExamData(global) {{\n  'use strict';\n  if (!global.__WRITING_EXAM_DATA__ || typeof global.__WRITING_EXAM_DATA__.register !== \"function\") {{\n    throw new Error(\"writing_exam_registry_missing\");\n  }}\n  global.__WRITING_EXAM_DATA__.register({}, {});\n}})(typeof window !== \"undefined\" ? window : globalThis);\n",
        key_json, source_json
    ))
}

/// 生成 manifest JS（window.__WRITING_EXAM_MANIFEST__ = { task1:{...}, task2:{...} }）。
/// key 用 taskType（NAS provider 按 taskType 查 entries）。
fn build_writing_manifest(sources: &[Value]) -> CommandResult<String> {
    let mut manifest = serde_json::Map::new();
    for source in sources {
        let task_type = source
            .get("taskType")
            .and_then(Value::as_str)
            .unwrap_or("task1");
        let exam_id = source.get("examId").and_then(Value::as_str).unwrap_or("");
        manifest.insert(
            task_type.to_string(),
            json!({
                "taskType": task_type,
                "examId": exam_id,
                "dataKey": task_type,
                "script": format!("./{}.js", task_type),
                "title": source.pointer("/meta/title").and_then(Value::as_str).unwrap_or("Untitled Writing")
            }),
        );
    }
    Ok(format!(
        "window.__WRITING_EXAM_MANIFEST__ = {};\n",
        serde_json::to_string_pretty(&Value::Object(manifest)).map_err(|error| error.to_string())?
    ))
}

/// 校验单个写作 job 可导出：taskType 合法 + promptText 非空。
fn validate_writing_job_for_export(job: &WritingJob) -> CommandResult<()> {
    if !WRITING_TASK_TYPES.contains(&job.task_type.as_str()) {
        return Err(format!(
            "writing_export_invalid_task_type:{}:{}",
            job.job_id, job.task_type
        ));
    }
    if job.prompt_text.trim().is_empty() {
        return Err(format!("writing_export_prompt_empty:{}", job.job_id));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct WrittenWritingSource {
    job_id: String,
    task_type: String,
    wrapper_js: String,
    source: Value,
}

#[derive(Debug, Clone)]
struct WritingExportWriteResult {
    files: Vec<WrittenWritingSource>,
    manifest_js: String,
}

/// 读指定 writing jobs → 校验 → 生成 task1.js/task2.js + manifest.js。
/// job_ids 应含两个（task1 + task2），按 taskType 去重。
fn write_writing_library_files(
    root: &Path,
    job_ids: &[String],
    writing_exams_dir: &Path,
) -> CommandResult<WritingExportWriteResult> {
    let mut files = Vec::with_capacity(job_ids.len());
    let mut seen_task_types = HashSet::new();

    for job_id in job_ids {
        validate_path_segment("writing_job_id", job_id)?;
        let job: WritingJob =
            read_json(&safe_writing_job_dir(root, job_id)?.join("writing-job.json"))?;
        validate_writing_job_for_export(&job)?;
        if !seen_task_types.insert(job.task_type.clone()) {
            return Err(format!(
                "writing_export_duplicate_task_type:{}:{}",
                job.job_id, job.task_type
            ));
        }
        let source = writing_source(&job);
        let wrapper = build_writing_wrapper(&source)?;
        files.push(WrittenWritingSource {
            job_id: job.job_id.clone(),
            task_type: job.task_type.clone(),
            wrapper_js: wrapper,
            source,
        });
    }

    // 必须两题齐全（task1 + task2）
    if !seen_task_types.contains("task1") || !seen_task_types.contains("task2") {
        return Err(format!(
            "writing_export_requires_both_tasks:missing={:?}",
            WRITING_TASK_TYPES
                .iter()
                .filter(|t| !seen_task_types.contains(**t))
                .collect::<Vec<_>>()
        ));
    }

    let sources: Vec<Value> = files.iter().map(|f| f.source.clone()).collect();
    let manifest_js = build_writing_manifest(&sources)?;

    fs::create_dir_all(writing_exams_dir).map_err(|error| error.to_string())?;
    for file in &files {
        write_text(
            &writing_exams_dir.join(format!("{}.js", file.task_type)),
            &file.wrapper_js,
        )?;
    }
    write_text(&writing_exams_dir.join("manifest.js"), &manifest_js)?;

    Ok(WritingExportWriteResult { files, manifest_js })
}

/// 导出核心：input = { jobIds: [task1JobId, task2JobId], exportDir: string }
pub(crate) fn export_writing_library_core(root: &Path, input: &Value) -> CommandResult<Value> {
    let job_ids: Vec<String> = input
        .get("jobIds")
        .and_then(Value::as_array)
        .ok_or_else(|| "writing_export_requires_job_ids".to_string())?
        .iter()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect();
    if job_ids.len() != 2 {
        return Err("writing_export_requires_two_jobs:task1+task2".to_string());
    }

    let export_dir = input
        .get("exportDir")
        .and_then(Value::as_str)
        .ok_or_else(|| "writing_export_requires_export_dir".to_string())?;
    let library_root = if export_dir.starts_with("local://") {
        root.join("exports").join("nas-library")
    } else {
        std::path::PathBuf::from(export_dir)
    };
    let library_root = normalize_nas_library_root(&library_root);
    let writing_exams_dir = nas_writing_exams_dir(&library_root);

    let write_result = write_writing_library_files(root, &job_ids, &writing_exams_dir)?;
    let asset_count = write_result.files.len();
    let version = Utc::now().format("%Y.%m.%d-%H%M%S").to_string();

    // 标记 job 已导出
    let mut cleanup = Vec::with_capacity(write_result.files.len());
    for written in &write_result.files {
        let mut job = crate::writing_store::load_writing_job(root, &written.job_id)?;
        job.status = WritingJobStatus::Exported;
        job.updated_at = Utc::now();
        crate::writing_store::save_writing_job(root, &job)?;
        cleanup.push(json!({
            "jobId": written.job_id,
            "taskType": written.task_type,
            "status": "Exported"
        }));
    }

    let files_array = write_result
        .files
        .iter()
        .map(|written| {
            json!({
                "name": format!("{}.js", written.task_type),
                "content": written.wrapper_js
            })
        })
        .chain(std::iter::once(json!({
            "name": "manifest.js",
            "content": write_result.manifest_js
        })))
        .collect::<Vec<_>>();

    Ok(json!({
        "mode": "writing-library",
        "jobIds": write_result.files.iter().map(|f| f.job_id.clone()).collect::<Vec<_>>(),
        "taskTypes": write_result.files.iter().map(|f| f.task_type.clone()).collect::<Vec<_>>(),
        "assetCount": asset_count,
        "libraryRoot": library_root.to_string_lossy(),
        "writingExamsDir": writing_exams_dir.to_string_lossy(),
        "version": version,
        "files": files_array,
        "report": {
            "status": "ok",
            "version": version,
            "generatedAt": Utc::now().to_rfc3339(),
            "summary": {
                "runtime": "nas-js-direct",
                "writingTaskCount": asset_count,
                "manifestFileCount": 1
            },
            "errors": []
        },
        "exportSummary": {
            "type": "writing-library",
            "runtime": "nas-js-direct",
            "jobIds": job_ids,
            "version": version,
            "outputDir": library_root.to_string_lossy(),
            "writingExamsDir": writing_exams_dir.to_string_lossy(),
            "assetCount": asset_count,
            "exportedAt": Utc::now().to_rfc3339()
        },
        "cleanup": cleanup
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writing_store::{save_writing_job, WritingJob};
    use chrono::Utc;
    use std::{fs, path::PathBuf};
    use uuid::Uuid;

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "pdf2test-writing-export-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn make_job(task_type: &str, exam_id: &str, prompt_text: &str) -> WritingJob {
        let now = Utc::now();
        WritingJob {
            job_id: format!("writing-{task_type}-fixture"),
            title: format!("Fixture {task_type}"),
            task_type: task_type.to_string(),
            exam_id: exam_id.to_string(),
            prompt_text: prompt_text.to_string(),
            suggested_word_count: if task_type == "task2" { 250 } else { 150 },
            status: WritingJobStatus::ExportReady,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn export_writing_library_core_writes_writing_exams_subdir() {
        let root = temp_root();
        let task1 = make_job("task1", "wt-task1-fixture", "Task 1 prompt");
        let task2 = make_job("task2", "wt-task2-fixture", "Task 2 prompt");
        save_writing_job(&root, &task1).unwrap();
        save_writing_job(&root, &task2).unwrap();

        let library_root = root.join("nas-library");
        let writing_exams_dir = nas_writing_exams_dir(&library_root);
        let input = json!({
            "jobIds": [task1.job_id, task2.job_id],
            "exportDir": library_root.to_string_lossy()
        });

        let result = export_writing_library_core(&root, &input).unwrap();

        assert!(writing_exams_dir.join("manifest.js").exists());
        assert!(writing_exams_dir.join("task1.js").exists());
        assert!(writing_exams_dir.join("task2.js").exists());
        assert_eq!(
            result.pointer("/writingExamsDir").and_then(Value::as_str),
            Some(writing_exams_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            result
                .pointer("/exportSummary/writingExamsDir")
                .and_then(Value::as_str),
            Some(writing_exams_dir.to_string_lossy().as_ref())
        );

        let _ = fs::remove_dir_all(root);
    }
}
