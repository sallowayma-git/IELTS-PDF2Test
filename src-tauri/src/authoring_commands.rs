use crate::{
    authoring_pipeline::{
        make_dynamic_authoring_ir, make_dynamic_split_candidates, merge_answer_source_candidates,
    },
    authoring_review::{authoring_review_issues, refresh_authoring_review_state},
    auto_pipeline::{
        parse_answer_source_candidates, select_llm_profile, vision_transcription_for_job,
    },
    cleanup::minimize_process_artifacts_after_authoring,
    job_store::{load_job, update_job},
    main_source_file,
    parser::{manual_transcription_document_ir, missing_source_document_ir, parse_source_document},
    reading_source::{
        answer_key_from_authoring, display_map_from_authoring, question_order_from_authoring,
        render_group_body_html,
    },
    source_review::{
        resolve_source_review_status, source_review_issues, source_review_status,
        source_review_status_for_job, write_source_review_status,
    },
    util::{ensure_job_dirs, job_dir, read_json, read_json_opt, write_json, write_text},
    CommandResult, IssueCounts, JobStatus, ManualTranscriptionInput, ParseOptions,
    RegenerateDraftInput, VisionTranscriptionInput, WorkflowStep,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::{fs, path::Path};

fn archive_current_draft_for_source_replacement(dir: &Path, reason: &str) -> CommandResult<()> {
    if !dir.join("authoring-ir.json").exists() && !dir.join("split-candidates.json").exists() {
        return Ok(());
    }
    let revisions_dir = dir.join("revisions");
    fs::create_dir_all(&revisions_dir).map_err(|error| error.to_string())?;
    let stamp = Utc::now().to_rfc3339().replace([':', '.', 'Z'], "-");
    let mut revision_dir = revisions_dir.join(format!("{}-source-replaced", stamp));
    let mut suffix = 1u32;
    while revision_dir.exists() {
        revision_dir = revisions_dir.join(format!("{}-source-replaced-{}", stamp, suffix));
        suffix += 1;
    }
    fs::create_dir_all(&revision_dir).map_err(|error| error.to_string())?;
    for file_name in [
        "authoring-ir.json",
        "split-candidates.json",
        "pipeline-report.json",
        "validation-report.json",
        "publish-readiness-report.json",
        "source-review.json",
    ] {
        let source = dir.join(file_name);
        if source.exists() {
            fs::rename(&source, revision_dir.join(file_name)).map_err(|error| {
                format!(
                    "archive_current_draft:{}->{}:{}",
                    source.display(),
                    revision_dir.join(file_name).display(),
                    error
                )
            })?;
        }
    }
    write_json(
        &revision_dir.join("revision-meta.json"),
        &json!({
            "archivedAt": Utc::now().to_rfc3339(),
            "reason": reason,
            "message": "Source document was replaced; previous editable draft was archived before regenerating."
        }),
    )?;
    Ok(())
}

pub(crate) fn parse_document_core(
    root: &Path,
    job_id: &str,
    options: ParseOptions,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let dir = job_dir(root, job_id);
    let mode = options.mode.as_deref().unwrap_or("auto");
    let ir = if let Some(source) = main_source_file(&job) {
        let upload_path = dir.join("uploads").join(&source.stored_name);
        if matches!(source.file_type.as_str(), "txt" | "md" | "pdf" | "docx")
            && upload_path.exists()
        {
            let parser_output = root
                .join("cache")
                .join("parser")
                .join(format!("{}-document-ir.json", job_id));
            parse_source_document(&job, source, &upload_path, &parser_output, mode)?
        } else {
            missing_source_document_ir(
                &job,
                mode,
                &format!(
                    "main source file missing or unsupported: type={}, path={}",
                    source.file_type,
                    upload_path.display()
                ),
            )
        }
    } else {
        missing_source_document_ir(&job, mode, "no MainQuestion source file")
    };
    archive_current_draft_for_source_replacement(&dir, "parse_document")?;
    write_json(&dir.join("document-ir.json"), &ir)?;
    let _ = write_source_review_status(root, job_id, Some(&ir), false, None)?;
    update_job(root, job_id, |job| {
        let review = source_review_status(root, job_id, Some(&ir))
            .unwrap_or_else(|_| json!({"required": true, "resolved": false}));
        job.status = if review
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            JobStatus::NeedsReview
        } else {
            JobStatus::Working
        };
        job.current_step = WorkflowStep::DocumentReview;
        job.issue_counts.needs_review = source_review_issues(&review).len() as u32;
    })?;
    Ok(ir)
}

pub(crate) fn apply_manual_transcription_core(
    root: &Path,
    job_id: &str,
    input: ManualTranscriptionInput,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let text = input.text.trim();
    if text.is_empty() {
        return Err("manual_transcription_text_required".to_string());
    }
    let dir = job_dir(root, job_id);
    ensure_job_dirs(&dir)?;
    archive_current_draft_for_source_replacement(&dir, "manual_transcription")?;
    write_text(&dir.join("manual-transcription.txt"), text)?;
    let ir = manual_transcription_document_ir(&job, text, input.note.as_deref());
    write_json(&dir.join("document-ir.json"), &ir)?;
    write_source_review_status(
        root,
        job_id,
        Some(&ir),
        true,
        Some(
            "manual transcription applied; operator must verify content before publish".to_string(),
        ),
    )?;
    update_job(root, job_id, |job| {
        job.status = JobStatus::Working;
        job.current_step = WorkflowStep::DocumentReview;
        job.issue_counts.needs_review = 0;
    })?;
    Ok(ir)
}

pub(crate) fn apply_vision_transcription_core(
    root: &Path,
    job_id: &str,
    input: Option<VisionTranscriptionInput>,
) -> CommandResult<Value> {
    let options = input.unwrap_or_default();
    let job = load_job(root, job_id)?;
    let profile_id = select_llm_profile(root, &job, options.profile_id)
        .ok_or_else(|| "no_enabled_llm_profile_available_for_vision_transcription".to_string())?;
    let dir = job_dir(root, job_id);
    ensure_job_dirs(&dir)?;
    archive_current_draft_for_source_replacement(&dir, "vision_transcription")?;
    let (ir, output) =
        vision_transcription_for_job(root, &job, &profile_id, options.note.as_deref())?;
    write_text(
        &dir.join("vision-transcription.txt"),
        output
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )?;
    write_json(&dir.join("vision-transcription-output.json"), &output)?;
    write_json(&dir.join("document-ir.json"), &ir)?;
    let review = write_source_review_status(root, job_id, Some(&ir), false, None)?;
    update_job(root, job_id, |job| {
        job.status = JobStatus::NeedsReview;
        job.current_step = WorkflowStep::DocumentReview;
        job.issue_counts.needs_review = source_review_issues(&review).len() as u32;
    })?;
    Ok(ir)
}

pub(crate) fn resolve_source_review_core(
    root: &Path,
    job_id: &str,
    note: Option<String>,
) -> CommandResult<Value> {
    let review = resolve_source_review_status(root, job_id, note)?;
    let dir = job_dir(root, job_id);
    let authoring = read_json_opt(&dir.join("authoring-ir.json"))?;
    let authoring_review_count = authoring
        .as_ref()
        .map(|ir| authoring_review_issues(ir).len() as u32)
        .unwrap_or(0);
    update_job(root, job_id, |job| {
        job.status = if authoring_review_count > 0 {
            JobStatus::NeedsReview
        } else if authoring.is_some() {
            JobStatus::DraftSaved
        } else {
            JobStatus::Working
        };
        job.current_step = if authoring_review_count > 0 {
            WorkflowStep::Authoring
        } else {
            WorkflowStep::DocumentReview
        };
        job.issue_counts.needs_review = authoring_review_count;
    })?;
    Ok(review)
}

fn allow_regenerate(input: Option<&RegenerateDraftInput>) -> bool {
    input
        .and_then(|value| value.allow_overwrite)
        .unwrap_or(false)
}

fn protect_existing_authoring(
    dir: &Path,
    input: Option<&RegenerateDraftInput>,
) -> CommandResult<()> {
    if dir.join("authoring-ir.json").exists() && !allow_regenerate(input) {
        return Err(
            "editable_draft_exists; pass allowOverwrite=true before regenerating split or draft"
                .to_string(),
        );
    }
    Ok(())
}

pub(crate) fn run_rule_split_core(
    root: &Path,
    job_id: &str,
    input: Option<RegenerateDraftInput>,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let dir = job_dir(root, job_id);
    protect_existing_authoring(&dir, input.as_ref())?;
    let doc = read_json_opt(&dir.join("document-ir.json"))?;
    let mut split = make_dynamic_split_candidates(job_id, &job, doc.as_ref());
    let answer_candidates = parse_answer_source_candidates(root, &job, "auto")?;
    merge_answer_source_candidates(&mut split, answer_candidates);
    write_json(&dir.join("split-candidates.json"), &split)?;
    update_job(root, job_id, |job| {
        job.status = JobStatus::Working;
        job.current_step = WorkflowStep::Split;
    })?;
    Ok(split)
}

pub(crate) fn save_split_adjustments_core(
    root: &Path,
    job_id: &str,
    patch: Value,
) -> CommandResult<Value> {
    write_json(&job_dir(root, job_id).join("split-candidates.json"), &patch)?;
    Ok(patch)
}

pub(crate) fn build_authoring_ir_core(
    root: &Path,
    job_id: &str,
    input: Option<RegenerateDraftInput>,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let dir = job_dir(root, job_id);
    protect_existing_authoring(&dir, input.as_ref())?;
    let doc = read_json_opt(&dir.join("document-ir.json"))?;
    let split = match read_json_opt(&dir.join("split-candidates.json"))? {
        Some(value) => value,
        None => {
            let mut value = make_dynamic_split_candidates(job_id, &job, doc.as_ref());
            let answer_candidates = parse_answer_source_candidates(root, &job, "auto")?;
            merge_answer_source_candidates(&mut value, answer_candidates);
            value
        }
    };
    let mut ir = make_dynamic_authoring_ir(&job, &split, doc.as_ref());
    let needs_review = refresh_authoring_review_state(&mut ir);
    let source_review = source_review_status(root, job_id, doc.as_ref())?;
    let source_review_issue_count = source_review_issues(&source_review).len() as u32;
    write_json(&job_dir(root, job_id).join("authoring-ir.json"), &ir)?;
    let _ = minimize_process_artifacts_after_authoring(root, job_id, "build_authoring_ir")?;
    update_job(root, job_id, |job| {
        job.status = if needs_review > 0 || source_review_issue_count > 0 {
            JobStatus::NeedsReview
        } else {
            JobStatus::DraftSaved
        };
        job.current_step = WorkflowStep::Authoring;
        job.issue_counts = IssueCounts {
            errors: 0,
            warnings: 1,
            needs_review: needs_review + source_review_issue_count,
        };
    })?;
    Ok(ir)
}

pub(crate) fn update_authoring_ir_core(
    root: &Path,
    job_id: &str,
    patch: Value,
) -> CommandResult<Value> {
    let mut ir = patch.get("ir").cloned().unwrap_or(patch);
    let needs_review = refresh_authoring_review_state(&mut ir);
    let source_review = source_review_status_for_job(root, job_id)?;
    let source_review_issue_count = source_review_issues(&source_review).len() as u32;
    if let Some(obj) = ir.as_object_mut() {
        obj.insert(
            "answerKey".to_string(),
            answer_key_from_authoring(&Value::Object(obj.clone())),
        );
        obj.insert(
            "questionOrder".to_string(),
            json!(question_order_from_authoring(&Value::Object(obj.clone()))),
        );
        obj.insert(
            "questionDisplayMap".to_string(),
            display_map_from_authoring(&Value::Object(obj.clone())),
        );
        if let Some(audit) = obj.get_mut("audit").and_then(Value::as_object_mut) {
            let revision = audit.get("revision").and_then(Value::as_u64).unwrap_or(0) + 1;
            audit.insert("revision".to_string(), json!(revision));
            audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
        }
    }
    write_json(&job_dir(root, job_id).join("authoring-ir.json"), &ir)?;
    update_job(root, job_id, |job| {
        job.status = if needs_review > 0 || source_review_issue_count > 0 {
            JobStatus::NeedsReview
        } else {
            JobStatus::DraftSaved
        };
        job.current_step = WorkflowStep::Authoring;
        job.issue_counts.needs_review = needs_review;
        job.issue_counts.needs_review += source_review_issue_count;
    })?;
    Ok(ir)
}

pub(crate) fn render_group_html_core(
    root: &Path,
    job_id: &str,
    group_id: &str,
) -> CommandResult<Value> {
    let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let group = ir
        .get("groups")
        .and_then(Value::as_array)
        .and_then(|groups| {
            groups
                .iter()
                .find(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id))
        })
        .ok_or_else(|| "group_not_found".to_string())?;
    Ok(json!({"groupId": group_id, "bodyHtml": render_group_body_html(group)}))
}
