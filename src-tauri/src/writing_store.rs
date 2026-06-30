//! 写作题库创作任务存储（镜像 job_store.rs，但独立模型，不污染阅读 ImportJob）。
//!
//! 存储：writing-jobs/<jobId>/writing-job.json
//! 复用 util::{safe_writing_job_dir, writing_job_dir, read_json, write_json, validate_path_segment}。

use crate::util::{safe_writing_job_dir, validate_path_segment, writing_job_dir, read_json, write_json};
use crate::CommandResult;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{cmp::Reverse, fs, path::Path};
use uuid::Uuid;

/// 写作任务状态。比阅读简单：无 PDF 解析中间态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub(crate) enum WritingJobStatus {
    Draft,
    ExportReady,
    Exported,
}

impl Default for WritingJobStatus {
    fn default() -> Self {
        WritingJobStatus::Draft
    }
}

/// 写作任务（手输 prompt，无 passage/questionGroups/answerKey）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WritingJob {
    pub job_id: String,
    pub title: String,
    pub task_type: String, // "task1" | "task2"
    pub exam_id: String,
    pub prompt_text: String,
    pub suggested_word_count: u32,
    #[serde(default)]
    pub status: WritingJobStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateWritingJobInput {
    pub title: Option<String>,
    pub task_type: Option<String>, // 默认 task1
    pub prompt_text: Option<String>,
    pub suggested_word_count: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WritingJobPatch {
    pub title: Option<String>,
    pub task_type: Option<String>,
    pub exam_id: Option<String>,
    pub prompt_text: Option<String>,
    pub suggested_word_count: Option<u32>,
    pub status: Option<WritingJobStatus>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WritingJobFilter {
    pub task_type: Option<String>,
    pub search: Option<String>,
}

fn normalize_task_type(value: Option<&str>) -> String {
    match value.map(|v| v.trim().to_lowercase()).as_deref() {
        Some("task2") => "task2".to_string(),
        _ => "task1".to_string(),
    }
}

fn default_suggested_word_count(task_type: &str) -> u32 {
    if task_type == "task2" {
        250
    } else {
        150
    }
}

fn default_exam_id(task_type: &str, now: &DateTime<Utc>) -> String {
    format!("wt-{}-{}", task_type, now.format("%Y%m%d%H%M%S"))
}

pub(crate) fn make_writing_job(input: CreateWritingJobInput) -> WritingJob {
    let now = Utc::now();
    let task_type = normalize_task_type(input.task_type.as_deref());
    let suggested_word_count = input
        .suggested_word_count
        .filter(|value| *value > 0)
        .unwrap_or_else(|| default_suggested_word_count(&task_type));
    let suffix = Uuid::new_v4().simple().to_string()[..8].to_string();
    WritingJob {
        job_id: format!("writing-{}-{}", now.format("%Y%m%d%H%M%S"), suffix),
        title: input
            .title
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("Untitled Writing {}", task_type)),
        exam_id: default_exam_id(&task_type, &now),
        task_type,
        prompt_text: input.prompt_text.unwrap_or_default(),
        suggested_word_count,
        status: WritingJobStatus::Draft,
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn load_writing_job(root: &Path, job_id: &str) -> CommandResult<WritingJob> {
    let dir = safe_writing_job_dir(root, job_id)?;
    read_json(&dir.join("writing-job.json"))
}

pub(crate) fn save_writing_job(root: &Path, job: &WritingJob) -> CommandResult<()> {
    validate_path_segment("writing_job_id", &job.job_id)?;
    write_json(&writing_job_dir(root, &job.job_id).join("writing-job.json"), job)
}

pub(crate) fn update_writing_job(
    root: &Path,
    job_id: &str,
    mutator: impl FnOnce(&mut WritingJob),
) -> CommandResult<WritingJob> {
    let mut job = load_writing_job(root, job_id)?;
    mutator(&mut job);
    job.updated_at = Utc::now();
    save_writing_job(root, &job)?;
    Ok(job)
}

pub(crate) fn list_writing_jobs(
    root: &Path,
    filter: Option<WritingJobFilter>,
) -> CommandResult<Vec<WritingJob>> {
    let jobs_root = root.join("writing-jobs");
    let mut jobs = Vec::new();
    if let Ok(entries) = fs::read_dir(&jobs_root) {
        for entry in entries.flatten() {
            let path = entry.path().join("writing-job.json");
            if path.exists() {
                if let Ok(job) = read_json::<WritingJob>(&path) {
                    jobs.push(job);
                }
            }
        }
    }
    let filter = filter.unwrap_or_default();
    if let Some(task_type) = filter.task_type {
        let normalized = task_type.trim().to_lowercase();
        jobs.retain(|job| job.task_type == normalized);
    }
    if let Some(search) = filter.search.filter(|value| !value.trim().is_empty()) {
        let search = search.to_lowercase();
        jobs.retain(|job| {
            job.title.to_lowercase().contains(&search)
                || job.job_id.to_lowercase().contains(&search)
                || job.exam_id.to_lowercase().contains(&search)
        });
    }
    jobs.sort_by_key(|job| Reverse(job.updated_at));
    Ok(jobs)
}

pub(crate) fn delete_writing_job(root: &Path, job_id: &str) -> CommandResult<()> {
    let dir = safe_writing_job_dir(root, job_id)?;
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|error| format!("remove_writing_job_dir:{}", error))?;
    }
    Ok(())
}

/// 将 WritingJob 转成导出用 source（WritingExamSourceV1 形状，作为 serde_json::Value）。
pub(crate) fn writing_source(job: &WritingJob) -> Value {
    serde_json::json!({
        "schemaVersion": "WritingExamSourceV1",
        "examId": job.exam_id,
        "taskType": job.task_type,
        "promptText": job.prompt_text,
        "suggestedWordCount": job.suggested_word_count,
        "meta": {
            "title": job.title,
            "taskType": job.task_type
        }
    })
}
