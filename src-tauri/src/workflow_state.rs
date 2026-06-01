use crate::job_store::update_job;
use crate::{CommandResult, JobStatus, WorkflowStep};
use serde_json::Value;
use std::path::Path;

pub(crate) fn apply_preview_e2e_job_state(
    root: &Path,
    job_id: &str,
    report: &Value,
    readiness_passed: bool,
) -> CommandResult<()> {
    let report_passed = report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let issues = report
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    update_job(root, job_id, |job| {
        job.status = if report_passed {
            if readiness_passed {
                JobStatus::ExportReady
            } else {
                JobStatus::DraftSaved
            }
        } else {
            JobStatus::NeedsReview
        };
        job.current_step = if report_passed {
            if readiness_passed {
                WorkflowStep::Export
            } else {
                WorkflowStep::Preview
            }
        } else {
            WorkflowStep::Preview
        };
        job.issue_counts.errors = count_issues(&issues, "error");
        job.issue_counts.warnings = count_issues(&issues, "warning");
    })?;
    Ok(())
}

pub(crate) fn validation_job_state(
    report_passed: bool,
    source_review_issue_count: u32,
) -> (JobStatus, WorkflowStep) {
    if source_review_issue_count > 0 {
        (JobStatus::NeedsReview, WorkflowStep::DocumentReview)
    } else if report_passed {
        (JobStatus::DraftSaved, WorkflowStep::Authoring)
    } else {
        (JobStatus::NeedsReview, WorkflowStep::Authoring)
    }
}

pub(crate) fn update_validation_job_state(
    root: &Path,
    job_id: &str,
    report: &Value,
    source_review_issue_count: u32,
) -> CommandResult<()> {
    let report_passed = report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (next_status, next_step) = validation_job_state(report_passed, source_review_issue_count);
    let issues = report
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    update_job(root, job_id, |job| {
        job.status = next_status.clone();
        job.current_step = next_step.clone();
        job.issue_counts.errors = count_issues(&issues, "error");
        job.issue_counts.warnings = count_issues(&issues, "warning");
        job.issue_counts.needs_review = source_review_issue_count;
    })?;
    Ok(())
}

fn count_issues(issues: &[Value], severity: &str) -> u32 {
    issues
        .iter()
        .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some(severity))
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_store::{load_job, make_job, save_job};
    use crate::{CreateJobInput, ImportJob, IssueCounts};
    use chrono::Utc;
    use serde_json::json;
    use std::{env, fs, path::PathBuf};
    use uuid::Uuid;

    fn temp_test_root() -> PathBuf {
        env::temp_dir().join(format!("epic8-workflow-test-{}", Uuid::new_v4().simple()))
    }

    fn test_job() -> ImportJob {
        make_job(CreateJobInput {
            title: Some("Workflow Fixture".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["workflow".to_string()]),
            llm_profile_id: None,
        })
    }

    fn save_stale_export_job(root: &Path) -> ImportJob {
        fs::create_dir_all(root.join("jobs")).unwrap();
        let mut job = test_job();
        job.status = JobStatus::ExportReady;
        job.current_step = WorkflowStep::Export;
        job.issue_counts = IssueCounts {
            errors: 0,
            warnings: 0,
            needs_review: 0,
        };
        job.updated_at = Utc::now();
        save_job(root, &job).unwrap();
        job
    }

    #[test]
    fn validation_job_state_routes_review_and_authoring_steps() {
        assert_eq!(
            validation_job_state(true, 1),
            (JobStatus::NeedsReview, WorkflowStep::DocumentReview)
        );
        assert_eq!(
            validation_job_state(false, 0),
            (JobStatus::NeedsReview, WorkflowStep::Authoring)
        );
        assert_eq!(
            validation_job_state(true, 0),
            (JobStatus::DraftSaved, WorkflowStep::Authoring)
        );
    }

    #[test]
    fn validate_authoring_state_update_overwrites_stale_current_step() {
        let root = temp_test_root();
        let job = save_stale_export_job(&root);

        let failed_report = json!({
            "passed": false,
            "issues": [{
                "severity": "error",
                "layer": "AuthoringIR",
                "path": "$.groups",
                "message": "Missing groups"
            }]
        });
        update_validation_job_state(&root, &job.job_id, &failed_report, 0).unwrap();
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert_eq!(saved.current_step, WorkflowStep::Authoring);
        assert_eq!(saved.issue_counts.errors, 1);

        let passed_report = json!({"passed": true, "issues": []});
        update_validation_job_state(&root, &job.job_id, &passed_report, 0).unwrap();
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::DraftSaved);
        assert_eq!(saved.current_step, WorkflowStep::Authoring);

        update_validation_job_state(&root, &job.job_id, &passed_report, 2).unwrap();
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert_eq!(saved.current_step, WorkflowStep::DocumentReview);
        assert_eq!(saved.issue_counts.needs_review, 2);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_e2e_state_update_overwrites_stale_export_ready_status() {
        let root = temp_test_root();
        let job = save_stale_export_job(&root);
        let failed_report = json!({
            "passed": false,
            "issues": [{
                "severity": "error",
                "layer": "RuntimePreview",
                "path": "runtime.execution",
                "message": "Preview failed"
            }]
        });

        apply_preview_e2e_job_state(&root, &job.job_id, &failed_report, false).unwrap();

        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::NeedsReview);
        assert_eq!(saved.current_step, WorkflowStep::Preview);
        assert_eq!(saved.issue_counts.errors, 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_e2e_state_update_routes_export_ready_only_after_readiness_passes() {
        let root = temp_test_root();
        let job = save_stale_export_job(&root);
        let passed_report = json!({"passed": true, "issues": []});

        apply_preview_e2e_job_state(&root, &job.job_id, &passed_report, false).unwrap();
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::DraftSaved);
        assert_eq!(saved.current_step, WorkflowStep::Preview);

        apply_preview_e2e_job_state(&root, &job.job_id, &passed_report, true).unwrap();
        let saved = load_job(&root, &job.job_id).unwrap();
        assert_eq!(saved.status, JobStatus::ExportReady);
        assert_eq!(saved.current_step, WorkflowStep::Export);

        let _ = fs::remove_dir_all(root);
    }
}
