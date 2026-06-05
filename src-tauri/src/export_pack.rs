use crate::{
    cleanup::{
        cleanup_transient_job_artifacts, minimize_process_artifacts_after_authoring,
        validation_summary,
    },
    export_artifacts::{
        build_manifest, build_pack_entry_bundle, build_reading_asset_bundle, safe_exam_id,
        PackSource,
    },
    job_store::update_job,
    reading_source::reading_source,
    runtime_validation::{publish_readiness_gate, validate_for_runtime_gate},
    util::{
        job_dir, read_json, validate_path_segment, write_bytes, write_json, write_text, write_zip,
    },
    CommandResult, JobStatus, WorkflowStep,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn export_reading_assets_core(
    root: &Path,
    job_id: &str,
    export_dir: &str,
    require_static_runtime_gate: bool,
) -> CommandResult<Value> {
    validate_path_segment("job_id", job_id)?;
    let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let report = validate_for_runtime_gate(root, job_id, &ir, require_static_runtime_gate)?;
    let report = publish_readiness_gate(root, job_id, &ir, report)?;
    if !report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
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
        "exportSummary": export_summary,
        "cleanup": cleanup
    }))
}

pub(crate) fn export_reading_js_core(
    root: &Path,
    input: &Value,
    require_static_runtime_gate: bool,
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
        if !report
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
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
    let export_summary = json!({
        "type": "reading-js",
        "mode": mode,
        "jobIds": job_ids,
        "examIds": exam_ids,
        "outputDir": out_dir.to_string_lossy(),
        "files": exam_ids.iter().map(|exam_id| format!("{}.js", exam_id)).chain(std::iter::once("manifest.js".to_string())).collect::<Vec<_>>(),
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
        "exportSummary": export_summary,
        "cleanup": cleanup
    }))
}

pub(crate) fn build_pack_core(
    root: &Path,
    input: &Value,
    require_static_runtime_gate: bool,
) -> CommandResult<Value> {
    let pack_id = input
        .get("packId")
        .and_then(Value::as_str)
        .unwrap_or("pack-local")
        .to_string();
    validate_path_segment("pack_id", &pack_id)?;
    if input
        .get("jobIds")
        .and_then(Value::as_array)
        .map(|items| items.is_empty())
        .unwrap_or(true)
    {
        return Err("pack_requires_at_least_one_job".to_string());
    }
    let pack_dir = root.join("packs").join(&pack_id);
    let exams_dir = pack_dir.join("reading-exams");
    let mut pack_sources = Vec::new();
    let mut job_ids = Vec::new();
    for job_id in input
        .get("jobIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        validate_path_segment("job_id", job_id)?;
        let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
        let source = reading_source(&ir);
        let report = validate_for_runtime_gate(root, job_id, &ir, require_static_runtime_gate)?;
        let report = publish_readiness_gate(root, job_id, &ir, report)?;
        if !report
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let _ = minimize_process_artifacts_after_authoring(
                root,
                job_id,
                "pack_publish_gate_failed",
            )?;
            return Err(format!(
                "pack_validation_failed:{}:{}",
                job_id,
                serde_json::to_string(&report).unwrap_or_default()
            ));
        }
        job_ids.push(job_id.to_string());
        pack_sources.push(PackSource {
            fallback_exam_id: job_id.to_string(),
            source,
        });
    }
    let pack_bundle = build_pack_entry_bundle(input, &pack_sources)?;
    let zip_path = root.join("packs").join(format!("{}.zip", pack_id));
    let zip_size = write_zip(&zip_path, &pack_bundle.entries)?;
    fs::create_dir_all(&exams_dir).map_err(|error| error.to_string())?;
    for (entry_path, content) in &pack_bundle.entries {
        if entry_path == "pack.json" {
            write_bytes(&pack_dir.join("pack.json"), content)?;
        } else if let Some(file_name) = entry_path.strip_prefix("reading-exams/") {
            write_bytes(&exams_dir.join(file_name), content)?;
        }
    }
    for job_id in &job_ids {
        update_job(root, job_id, |job| {
            job.status = JobStatus::Exported;
            job.current_step = WorkflowStep::Pack;
        })?;
    }
    let export_summary = json!({
        "type": "pack",
        "packId": pack_id,
        "outputPath": zip_path.to_string_lossy(),
        "files": pack_bundle.entries.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>(),
        "zipSizeBytes": zip_size,
        "entryCount": pack_bundle.entries.len(),
        "exportedAt": Utc::now().to_rfc3339()
    });
    let mut cleanup_summaries = Vec::new();
    for job_id in &job_ids {
        cleanup_summaries.push(cleanup_transient_job_artifacts(
            root,
            job_id,
            export_summary.clone(),
        )?);
    }
    Ok(json!({
        "packId": pack_id,
        "outputPath": zip_path.to_string_lossy(),
        "files": pack_bundle.entries.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>(),
        "zipSizeBytes": zip_size,
        "entryCount": pack_bundle.entries.len(),
        "manifest": pack_bundle.pack_manifest,
        "exportSummary": export_summary,
        "cleanup": cleanup_summaries,
        "createdAt": Utc::now().to_rfc3339()
    }))
}
