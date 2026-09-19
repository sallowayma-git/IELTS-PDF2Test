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
    // 诊断是否**真的跑过**。静态契约没过时根本不跑侧车，此时两份 report 是同一个，
    // 也就不该凭空多出一个"诊断结论"字段（否则会把"没查"读成"查了且失败"）。
    let mut diagnostic_ran = false;
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
                    // 显式声明"这是合并"，不依赖 `merge_sidecar_validation` 的缺省。
                    //
                    // 这里直传侧车**原始**输出，它可能既没有 `layers` 也没有
                    // `replaceExistingLayers`；过去"缺省即替换"的语义会让这样一份
                    // 空输出把 Rust 自查在 ReadingExamSourceV1/DomProtocol 两层的 error
                    // 整层删掉（`runtime_validation.rs:340` 那条路径显式置了 false，
                    // 只有这里没置）。侧车自己显式给出的值仍然优先。
                    diagnostic_ran = true;
                    let mut runtime_report = runtime_report;
                    if let Some(object) = runtime_report.as_object_mut() {
                        object
                            .entry("replaceExistingLayers".to_string())
                            .or_insert(json!(false));
                    }
                    merge_sidecar_validation(&mut diagnostic_report, runtime_report)
                }
                Err(error) => {
                    diagnostic_ran = true;
                    merge_sidecar_validation(
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
                    )
                }
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
    // ── 一份产物、两个**已命名**的问题（NOTES §3.4）────────────────────────
    //
    // 修前这里有两份 report，而且各用各的：`readiness_passed` 与 job 状态用
    // `static_report`（侧车合并**之前**的那份），落盘并返回的却是
    // `diagnostic_report`（合并**之后**的那份）。于是"能不能发"这件事在产物里
    // 和在 job 状态里可以是两个答案。
    //
    // 产品决策 E8-26（`Plan With Files/task_plan.md:506`、`findings.md:615`）：
    // 预览 E2E 是开发 / CI **诊断**行为，**诊断失败可见，但不把 static 已就绪的
    // job 降级**。所以本轮不动判据归属，只让同一份产物把两件事分别说清楚：
    //
    //   * `passed`                     = 发布判据（静态契约 + 就绪门）—— 与 job 状态同源；
    //   * `previewDiagnostics.passed`  = 预览运行时诊断结论 —— 显式标为 `binding:false`。
    //
    // `issues` / `layers` / `runtime` 的形状刻意保持原样（既有 UI 与脚本在读），
    // `previewDiagnostics` 是纯新增字段；这样不会有第三个消费者被改坏。
    let static_report_passed = static_report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let diagnostic_passed = diagnostic_report
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
    let mut report = diagnostic_report;
    // #14：本路径读的是 `authoring-ir.json` 文件，而预检读 canonical ⇒ 记下两者是否
    // 同一份稿（只检出，不阻塞，见 `authoring_validation::version_alignment`）。
    if let Some(ir) = authoring.as_ref() {
        crate::authoring_validation::record_version_alignment(&mut report, root, job_id, ir);
    }
    if let Some(object) = report.as_object_mut() {
        // 覆盖合并时按 issues 重算出来的 `passed`：那个值把**诊断**的 error 也算了进去，
        // 而 `passed` 回答的是发布问题（E8-26）。覆盖成 `static_report_passed`
        // —— 即 job 状态用的那个值 —— 产物与状态从此同源。
        object.insert("passed".to_string(), json!(static_report_passed));
        // 把 `passed` 由哪两项构成写清楚，读者不必去猜"诊断算不算"。
        object.insert(
            "publishBasis".to_string(),
            json!({
                "staticContractPassed": static_report_passed,
                "readinessPassed": readiness_passed,
                "previewDiagnosticsBinding": false,
            }),
        );
        if diagnostic_ran {
            object.insert(
                "previewDiagnostics".to_string(),
                json!({
                    "binding": false,
                    "passed": diagnostic_passed,
                    "note": "Preview E2E is an explicit development/CI diagnostic: its outcome is recorded here and does not change `passed` or the job's publish status.",
                }),
            );
        }
    }
    write_json(
        &job_dir(root, job_id).join("validation-report.json"),
        &report,
    )?;
    // job 状态与落盘产物同源：传的就是上面写下去的那一份（不再传一个旧副本）。
    apply_preview_e2e_job_state(root, job_id, &report, readiness_passed)?;
    Ok(report)
}
