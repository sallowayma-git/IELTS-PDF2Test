use crate::util::{job_dir, read_json, safe_job_dir, validate_path_segment, write_json};
use crate::{CommandResult, CreateJobInput, ImportJob, JobFilter, JobStatus, WorkflowStep};
use chrono::Utc;
use std::{cmp::Reverse, fs, path::Path};
use uuid::Uuid;

pub(crate) fn make_job(input: CreateJobInput) -> ImportJob {
    let now = Utc::now();
    let suffix = Uuid::new_v4().simple().to_string()[..8].to_string();
    ImportJob {
        job_id: format!("import-{}-{}", now.format("%Y%m%d%H%M%S"), suffix),
        title: input
            .title
            .unwrap_or_else(|| "Untitled Reading".to_string()),
        status: JobStatus::Working,
        category: input.category.or_else(|| Some("P1".to_string())),
        frequency: input.frequency.or_else(|| Some("medium".to_string())),
        tags: input.tags.unwrap_or_default(),
        source_files: vec![],
        active_llm_profile_id: input.llm_profile_id,
        created_at: now,
        updated_at: now,
        current_step: WorkflowStep::Upload,
        issue_counts: Default::default(),
    }
}

pub(crate) fn load_job(root: &Path, job_id: &str) -> CommandResult<ImportJob> {
    let dir = safe_job_dir(root, job_id)?;
    read_json(&dir.join("job.json"))
}

pub(crate) fn save_job(root: &Path, job: &ImportJob) -> CommandResult<()> {
    validate_path_segment("job_id", &job.job_id)?;
    write_json(&job_dir(root, &job.job_id).join("job.json"), job)
}

pub(crate) fn update_job(
    root: &Path,
    job_id: &str,
    mutator: impl FnOnce(&mut ImportJob),
) -> CommandResult<ImportJob> {
    let mut job = load_job(root, job_id)?;
    mutator(&mut job);
    job.updated_at = Utc::now();
    save_job(root, &job)?;
    Ok(job)
}

pub(crate) fn list_saved_jobs(
    root: &Path,
    filter: Option<JobFilter>,
) -> CommandResult<Vec<ImportJob>> {
    let jobs_root = root.join("jobs");
    let mut jobs = Vec::new();
    for entry in fs::read_dir(jobs_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path().join("job.json");
        if path.exists() {
            jobs.push(read_json::<ImportJob>(&path)?);
        }
    }
    let filter = filter.unwrap_or_default();
    if let Some(status) = filter.status {
        jobs.retain(|job| job.status == status);
    }
    if let Some(search) = filter.search.filter(|value| !value.trim().is_empty()) {
        let search = search.to_lowercase();
        jobs.retain(|job| {
            job.title.to_lowercase().contains(&search)
                || job.job_id.to_lowercase().contains(&search)
        });
    }
    jobs.sort_by_key(|job| Reverse(job.updated_at));
    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs};

    #[test]
    fn legacy_job_load_does_not_require_phase1_directories() {
        let root = env::temp_dir().join(format!(
            "phase1-job-store-{}",
            Uuid::new_v4().simple()
        ));
        let job = make_job(CreateJobInput {
            title: Some("Legacy job".to_string()),
            ..Default::default()
        });
        save_job(&root, &job).unwrap();
        let loaded = load_job(&root, &job.job_id).unwrap();
        assert_eq!(loaded.job_id, job.job_id);
        assert_eq!(loaded.title, "Legacy job");
        assert!(!job_dir(&root, &job.job_id).join("authoring").exists());
        let _ = fs::remove_dir_all(root);
    }
}
