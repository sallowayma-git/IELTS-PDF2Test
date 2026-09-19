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

/// 遗留导出的校验选项。
///
/// 这个类型以前承载 `ValidationPolicy::{Strict, Force}`，而 `Force` 让调用方
/// **通过入参**关掉发布门禁（`should_block = policy == Strict && !passed`）。
/// 产品 IPC 面有三个入口能传它（`export_reading_assets`（`lib.rs:1407-1417`）、
/// `export_reading_js`、`export_nas_library`），属于「产品路径能绕过门禁」，
/// 因此该能力已整体删除（同时删掉的还有 `validation_overridden` / `ignored_issues`
/// 以及它们对 `severity == "error"` 的漏记、批量 `|=` 的批级污染）。
///
/// 现在它**不承载任何策略**：`from_policy` 只接受缺省与 `"strict"`，
/// 其它值一律**明确报错**。刻意不做「非 strict 就静默按 strict 处理」——
/// 静默降级会让调用方以为绕过生效了，比报错更坏。
///
/// 前端契约（`src/api/tauriCommands.ts` 的 `"strict" | "force"`）不在本轮范围内，
/// 由主线另行安排；**后端入口从这一版起已关闭**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExportValidationOptions;

impl ExportValidationOptions {
    pub(crate) fn strict() -> Self {
        Self
    }

    pub(crate) fn from_input(input: &Value) -> CommandResult<Self> {
        Self::from_policy(input.get("validationPolicy").and_then(Value::as_str))
    }

    pub(crate) fn from_policy(policy: Option<&str>) -> CommandResult<Self> {
        match policy.unwrap_or("strict") {
            "strict" => Ok(Self),
            other => Err(format!("invalid_validation_policy:{other}")),
        }
    }
}

/// 唯一判据入口，见 `authoring_validation::publish_verdict`。
///
/// 遗留导出的稿件来自 `authoring-ir.json`（派生文件），因此范围恒为 `FullDerived`；
/// 版本未知 ⇒ 传 `None`（`Some` 断言的是"这份稿确实取自 canonical 的第 v 版"，
/// 文件来源给不出这个断言，不能拿 canonical 的版本号冒充）。
///
/// 三条遗留导出入口（reading-assets / reading-js / nas-library）共用它，
/// 避免各自再写一遍"怎么算这份稿能不能发"。
pub(crate) fn legacy_export_verdict(
    root: &Path,
    job_id: &str,
    ir: &Value,
) -> crate::authoring_validation::PublishVerdict {
    crate::authoring_validation::publish_verdict(
        root,
        job_id,
        ir,
        None,
        crate::authoring_validation::PublishScope::FullDerived,
    )
}

/// 把结论转成调用方看得懂的错误串。
///
/// `Undetermined`（判据没能执行）与 `Blocked`（查过了，不能发）**用不同的错误码**：
/// 前者是"没查清楚"，后者是"确实不行"，混在一起会让人去改内容而其实该修环境。
pub(crate) fn legacy_gate_error(
    kind: &str,
    report: &Value,
    verdict: &crate::authoring_validation::PublishVerdict,
) -> String {
    if matches!(
        verdict,
        crate::authoring_validation::PublishVerdict::Undetermined { .. }
    ) {
        format!(
            "{kind}_undetermined:{}",
            serde_json::to_string(&verdict.to_value()).unwrap_or_default()
        )
    } else {
        format!(
            "{kind}:{}",
            serde_json::to_string(report).unwrap_or_default()
        )
    }
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
    _options: ExportValidationOptions,
) -> CommandResult<Value> {
    validate_path_segment("job_id", job_id)?;
    let ir: Value = read_json(&job_dir(root, job_id).join("authoring-ir.json"))?;
    let report = validate_for_runtime_gate(root, job_id, &ir, require_static_runtime_gate)?;
    let mut report = publish_readiness_gate(root, job_id, &ir, report)?;
    // 判据只来自 `publish_verdict`；`report.passed` 改为由结论派生，
    // 产物与结论因此不可能互相矛盾（修前 `report.passed` 自己是第 4 个判据来源）。
    let verdict = legacy_export_verdict(root, job_id, &ir);
    crate::authoring_validation::apply_publish_verdict(
        &mut report,
        &verdict,
        crate::authoring_validation::PublishScope::FullDerived,
    );
    // #14：本路径读的是 `authoring-ir.json` 文件，而预检读 canonical ⇒ 把
    // "两份稿是不是同一份"如实记进产物（只检出，不阻塞，见 `version_alignment`）。
    crate::authoring_validation::record_version_alignment(&mut report, root, job_id, &ir);
    persist_export_validation_report(root, job_id, &report)?;
    if !verdict.is_ready() {
        let _ =
            minimize_process_artifacts_after_authoring(root, job_id, "export_publish_gate_failed")?;
        return Err(legacy_gate_error(
            "export_validation_failed",
            &report,
            &verdict,
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
        // force 能力已删除。这三个字段保留是为不破坏导出产物的既有 schema，
        // 但它们只可能是「未绕过」；真正的判据在 `publishVerdict` 里。
        "validationPolicy": "strict",
        "validationOverridden": false,
        "ignoredIssueCount": 0u64,
        "ignoredIssues": Vec::<Value>::new(),
        "publishVerdict": verdict.to_value(),
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
        "validationPolicy": "strict",
        "validationOverridden": false,
        "ignoredIssueCount": 0u64,
        "ignoredIssues": Vec::<Value>::new(),
        "publishVerdict": verdict.to_value(),
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
    _options: ExportValidationOptions,
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
        let mut report = publish_readiness_gate(root, job_id, &ir, report)?;
        // 判据只来自 `publish_verdict`；修前这里用 `validation_overridden |= …` 做
        // 批级或运算，一卷被 override 就把整批 summary 标成 overridden。
        // 那个字段与它的批级污染随 force 能力一起消失。
        let verdict = legacy_export_verdict(root, job_id, &ir);
        crate::authoring_validation::apply_publish_verdict(
            &mut report,
            &verdict,
            crate::authoring_validation::PublishScope::FullDerived,
        );
        crate::authoring_validation::record_version_alignment(&mut report, root, job_id, &ir);
        persist_export_validation_report(root, job_id, &report)?;
        if !verdict.is_ready() {
            let _ = minimize_process_artifacts_after_authoring(
                root,
                job_id,
                "js_export_publish_gate_failed",
            )?;
            return Err(format!(
                "{}:{}",
                legacy_gate_error("js_export_validation_failed", &report, &verdict),
                job_id
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
        // force 能力已删除：这三个字段保留只为不破坏既有产物 schema，
        // 取值只可能是「未绕过」；判据在 `publishVerdict`（批内每卷都通过才会走到这里）。
        "validationPolicy": "strict",
        "validationOverridden": false,
        "ignoredIssueCount": 0u64,
        "ignoredIssues": Vec::<Value>::new(),
        "notice": "batch gate passed for every job in this batch; force policy is no longer accepted by any entry point",
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
        "validationPolicy": "strict",
        "validationOverridden": false,
        "ignoredIssueCount": 0u64,
        "ignoredIssues": Vec::<Value>::new(),
        "exportSummary": export_summary,
        "cleanup": cleanup
    }))
}
