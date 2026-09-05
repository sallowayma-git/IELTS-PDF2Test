//! Phase 5 structured-authoring persistence.
//!
//! This module is deliberately separate from the V1 authoring commands.  A
//! V2 edit starts from the Phase 4 shadow artifact (or from an existing V2
//! revision), applies a small, explicit patch vocabulary, validates the full
//! typed authoring document, and appends an immutable revision.  The legacy
//! `authoring-ir.json` file is never rewritten here.

use crate::artifact_store::{
    append_revision, ensure_job_artifact_layout, list_revision_records, read_revision,
    recover_current_revision, write_artifact_json, write_canonical_json_atomic, RevisionSourceV2,
};
use crate::ielts_grammar::evaluate_quality;
use crate::reading_source_v2::compile_reading_source_v2;
use crate::schema::common::AssetDescriptorV2;
use crate::schema::IeltsAuthoringIRV2;
use crate::source_review::{
    source_review_issues, source_review_status, source_review_status_for_job,
};
use crate::util::{
    is_safe_path_segment, job_dir, read_json_opt, safe_job_dir, stage_file_with_hash,
};
use crate::CommandResult;
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use fs2::FileExt;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use std::{fs, fs::OpenOptions, path::PathBuf};
use uuid::Uuid;

pub(crate) const AUTHORING_V2_SHADOW_FILE: &str = "authoring-ir-v2.shadow.json";
const DOCUMENT_V2_SHADOW_FILE: &str = "document-ir-v2.shadow.json";
const SESSION_SCHEMA_VERSION: &str = "AuthoringEditorSessionV1";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyAuthoringV2PatchesInput {
    pub job_id: String,
    pub base_revision: u64,
    pub patches: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportAuthoringV2Input {
    pub job_id: String,
    pub export_dir: String,
    pub revision: Option<u64>,
    /// M1 typed-preflight 直通：调用方（题库/工作区发布链）显式传入 canonical DS
    /// （来自 library_items_v2，编辑版本 `editVersion`）。提供时：
    /// - 不再读文件会话/校验文件 revision（DB 是权威，文件只是派生缓存）；
    /// - 发布门禁换成 `check_publish_preflight`（当前稿检查，无历史痕迹扫描）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authoring: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_version: Option<u64>,
}

pub(crate) fn get_authoring_v2_core(root: &Path, job_id: &str) -> CommandResult<Value> {
    safe_job_dir(root, job_id)?;
    build_editor_session(root, job_id)
}

pub(crate) fn resolve_authoring_asset_preview_core(
    root: &Path,
    job_id: &str,
    asset_id: &str,
) -> CommandResult<Value> {
    let job_root = fs::canonicalize(safe_job_dir(root, job_id)?)
        .map_err(|error| format!("authoring_asset_root_unavailable:{error}"))?;
    let (authoring_value, _) = load_current_authoring(root, job_id)?;
    let authoring: IeltsAuthoringIRV2 = serde_json::from_value(authoring_value)
        .map_err(|error| format!("AUTHORING_SCHEMA_INVALID:{error}"))?;
    let descriptor = authoring
        .assets
        .iter()
        .find(|asset| asset.asset_id == asset_id)
        .ok_or_else(|| format!("authoring_asset_missing:{asset_id}"))?;
    if !descriptor.mime.starts_with("image/") {
        return Err(format!(
            "authoring_asset_preview_mime_unsupported:{}:{}",
            descriptor.asset_id, descriptor.mime
        ));
    }
    let relative = safe_asset_relative_path(&descriptor.relative_path)?;
    let asset_path = fs::canonicalize(job_root.join(relative)).map_err(|error| {
        format!(
            "authoring_asset_source_missing:{}:{error}",
            descriptor.asset_id
        )
    })?;
    if !asset_path.starts_with(&job_root) {
        return Err(format!(
            "authoring_asset_source_escape:{}",
            descriptor.asset_id
        ));
    }
    let bytes = fs::read(&asset_path)
        .map_err(|error| format!("authoring_asset_read:{}:{error}", descriptor.asset_id))?;
    if bytes.len() as u64 != descriptor.byte_length {
        return Err(format!(
            "authoring_asset_size_mismatch:{}:expected={}:actual={}",
            descriptor.asset_id,
            descriptor.byte_length,
            bytes.len()
        ));
    }
    let actual_hash = crate::hash_bytes(&bytes);
    if !actual_hash.eq_ignore_ascii_case(&descriptor.sha256) {
        return Err(format!(
            "authoring_asset_hash_mismatch:{}:expected={}:actual={actual_hash}",
            descriptor.asset_id, descriptor.sha256
        ));
    }
    Ok(json!({
        "assetId": descriptor.asset_id,
        "mime": descriptor.mime,
        "widthPx": descriptor.width_px,
        "heightPx": descriptor.height_px,
        "resourceUri": format!(
            "data:{};base64,{}",
            descriptor.mime,
            general_purpose::STANDARD.encode(bytes)
        )
    }))
}

pub(crate) fn apply_authoring_v2_patches_core(root: &Path, input: Value) -> CommandResult<Value> {
    let input: ApplyAuthoringV2PatchesInput = serde_json::from_value(input)
        .map_err(|error| format!("authoring_v2_invalid_patch_request:{error}"))?;
    safe_job_dir(root, &input.job_id)?;
    if input.patches.is_empty() {
        return Err("authoring_v2_patches_required".to_string());
    }
    if input.patches.len() > 200 {
        return Err("authoring_v2_patch_batch_too_large:max=200".to_string());
    }

    let (mut authoring, current_revision) = load_current_authoring(root, &input.job_id)?;
    if current_revision != input.base_revision {
        return Err(format!(
            "revision_conflict:current={current_revision}:base={}",
            input.base_revision
        ));
    }

    for patch in &input.patches {
        apply_patch(&mut authoring, patch)?;
    }
    mark_user_audit(&mut authoring, input.base_revision.saturating_add(1));
    refresh_quality_report(root, &input.job_id, &mut authoring)?;
    validate_authoring(&authoring)?;

    let result = append_revision(
        root,
        &input.job_id,
        input.base_revision,
        RevisionSourceV2::User,
        &authoring,
        &input.patches,
    )?;

    Ok(json!({
        "schemaVersion": SESSION_SCHEMA_VERSION,
        "jobId": input.job_id,
        "authoring": authoring,
        "revision": result.current.revision,
        "source": "revision",
        "revisions": list_revision_records(root, &input.job_id)?,
        "v1FilesRemainReadable": true,
        "savedPatchCount": input.patches.len(),
    }))
}

/// Return the durable proof required before a V2 export can be published.
///
/// This is shared with the NAS publisher so a source file cannot be published
/// merely because it happens to deserialize as `ReadingExamSourceV2`.
pub(crate) fn validate_authoring_v2_publish_readiness(
    root: &Path,
    job_id: &str,
    revision: u64,
    authoring_value: &Value,
) -> CommandResult<Value> {
    safe_job_dir(root, job_id)?;
    validate_authoring(authoring_value)?;
    if authoring_value.get("jobId").and_then(Value::as_str) != Some(job_id) {
        return Err("authoring_v2_export_blocked:job_id_mismatch".to_string());
    }
    let current = recover_current_revision(root, job_id)?;
    if current.revision != revision {
        return Err(format!(
            "authoring_v2_export_blocked:revision_not_current:current={}:requested={revision}",
            current.revision
        ));
    }

    let quality: crate::schema::quality_report_v2::QualityReportV2 = serde_json::from_value(
        authoring_value
            .get("quality")
            .cloned()
            .ok_or_else(|| "AUTHORING_SCHEMA_INVALID:quality_missing".to_string())?,
    )
    .map_err(|error| format!("AUTHORING_SCHEMA_INVALID:quality:{error}"))?;
    if !matches!(
        &quality.state,
        crate::schema::quality_report_v2::ReadinessStateV2::Ready
    ) {
        let state = match &quality.state {
            crate::schema::quality_report_v2::ReadinessStateV2::Ready => "ready",
            crate::schema::quality_report_v2::ReadinessStateV2::ReviewRequired => "review_required",
            crate::schema::quality_report_v2::ReadinessStateV2::Blocked => "blocked",
        };
        return Err(format!("authoring_v2_export_blocked:quality_state={state}"));
    }
    let unresolved_answers = authoring_value
        .get("answerKey")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|answers| answers.iter())
        .filter_map(|(slot_id, value)| {
            (value.get("kind").and_then(Value::as_str) == Some("unresolved"))
                .then_some(slot_id.clone())
        })
        .collect::<Vec<_>>();
    if !unresolved_answers.is_empty() {
        return Err(format!(
            "authoring_v2_export_blocked:unresolved_answers={}",
            unresolved_answers.join(",")
        ));
    }
    if !quality.hard_failures.is_empty() {
        return Err(format!(
            "authoring_v2_export_blocked:hard_failures={}",
            quality.hard_failures.join(",")
        ));
    }
    let unresolved_blockers = quality
        .issues
        .iter()
        .filter(|issue| {
            matches!(
                issue.severity,
                crate::schema::quality_report_v2::ReviewSeverityV2::Blocking
            ) && issue
                .details
                .as_ref()
                .and_then(|details| details.get("resolution"))
                .and_then(Value::as_str)
                .is_none_or(|resolution| !matches!(resolution, "resolved" | "ignored"))
        })
        .map(|issue| issue.issue_id.clone())
        .collect::<Vec<_>>();
    if !unresolved_blockers.is_empty() {
        return Err(format!(
            "authoring_v2_export_blocked:issues={}",
            unresolved_blockers.join(",")
        ));
    }
    if authoring_value
        .pointer("/audit/humanVerified")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("authoring_v2_export_blocked:human_verification_required".to_string());
    }

    let document_ir = read_json_opt(&job_dir(root, job_id).join("document-ir.json"))?;
    let source_review = match document_ir.as_ref() {
        Some(document) => source_review_status(root, job_id, Some(document))?,
        None => source_review_status_for_job(root, job_id)?,
    };
    if source_review.get("schemaVersion").and_then(Value::as_str) != Some("SourceReviewV1")
        || source_review.get("jobId").and_then(Value::as_str) != Some(job_id)
    {
        return Err("authoring_v2_export_blocked:source_review_invalid".to_string());
    }
    if source_review.get("stale").and_then(Value::as_bool) == Some(true) {
        return Err("authoring_v2_export_blocked:source_review_stale".to_string());
    }
    if source_review.get("resolved").and_then(Value::as_bool) != Some(true) {
        return Err("authoring_v2_export_blocked:source_review_unresolved".to_string());
    }
    let source_review_blockers = source_review_issues(&source_review);
    if !source_review_blockers.is_empty() {
        return Err(format!(
            "authoring_v2_export_blocked:source_review_issues={}",
            serde_json::to_string(&source_review_blockers).unwrap_or_default()
        ));
    }

    let mut ai_fallbacks = Vec::new();
    let mut partial_failures = Vec::new();
    collect_publish_gate_markers(
        authoring_value,
        "authoring",
        &mut ai_fallbacks,
        &mut partial_failures,
    );
    for relative in ["authoring-ir.json", "pipeline-report.json"] {
        let path = job_dir(root, job_id).join(relative);
        if let Some(value) = read_json_opt(&path)? {
            collect_publish_gate_markers(
                &value,
                relative,
                &mut ai_fallbacks,
                &mut partial_failures,
            );
        }
    }
    if !ai_fallbacks.is_empty() {
        return Err(format!(
            "authoring_v2_export_blocked:ai_fallback={}",
            ai_fallbacks.join(",")
        ));
    }
    if !partial_failures.is_empty() {
        return Err(format!(
            "authoring_v2_export_blocked:partial_failures={}",
            partial_failures.join(",")
        ));
    }

    Ok(json!({
        "schemaVersion": "AuthoringV2PublishProofV1",
        "jobId": job_id,
        "revision": revision,
        "qualityState": "ready",
        "humanVerified": true,
        "sourceReview": source_review,
        "unresolvedAnswers": [],
        "unresolvedBlockingIssues": [],
        "aiFallbacks": [],
        "partialFailures": []
    }))
}

/// M1 typed preflight（计划 §13.3/§13.4）：只检查**当前** canonical DS 与当前 blocker。
/// 与 [`validate_authoring_v2_publish_readiness`] 的差别（均为有意移除）：
/// - 不扫描历史 authoring/pipeline JSON 的 fallback/partial 字符串；
/// - 不要求全局 `audit.humanVerified`；
/// - 不依赖 SourceReviewV1 文件状态。
/// 资源闭包由 publisher 的 staging/probe 与资产 hash 绑定继续保证（§13.5 原子发布保留）。
pub(crate) fn check_publish_preflight(
    job_id: &str,
    edit_version: u64,
    authoring_value: &Value,
) -> Value {
    let mut blockers: Vec<Value> = Vec::new();
    let mut warnings: Vec<Value> = Vec::new();

    if let Err(error) = validate_authoring(authoring_value) {
        blockers.push(json!({
            "code": "SCHEMA_INVALID",
            "targetId": null,
            "userMessage": "这道题的数据结构不完整，无法编译成学生端试卷。",
            "action": "open_workspace",
            "internal": error
        }));
    }

    let quality = authoring_value
        .get("quality")
        .cloned()
        .unwrap_or(Value::Null);
    let state = quality
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if state != "ready" {
        blockers.push(json!({
            "code": "QUALITY_NOT_READY",
            "targetId": null,
            "userMessage": "这道题还有未确认的内容，处理完界面里列出的问题后可以发布。",
            "action": "open_workspace",
            "internal": format!("quality_state={state}")
        }));
    }
    let hard_failures = quality
        .get("hardFailures")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for failure in hard_failures.iter().take(20) {
        blockers.push(json!({
            "code": "QUALITY_HARD_FAILURE",
            "targetId": null,
            "userMessage": "这道题存在必须修复的内容缺陷。",
            "action": "open_workspace",
            "internal": failure
        }));
    }
    let unresolved_answers = authoring_value
        .get("answerKey")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|answers| answers.iter())
        .filter(|(_, value)| value.get("kind").and_then(Value::as_str) == Some("unresolved"))
        .map(|(slot_id, _)| slot_id.clone())
        .collect::<Vec<_>>();
    for slot_id in unresolved_answers.iter().take(20) {
        blockers.push(json!({
            "code": "ANSWER_MISSING",
            "targetId": slot_id,
            "userMessage": "这道题还有答案没有填写。",
            "action": "open_workspace"
        }));
    }
    let unresolved_blockers = quality
        .get("issues")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|issue| {
            issue.get("severity").and_then(Value::as_str) == Some("blocking")
                && !matches!(
                    issue.pointer("/details/resolution").and_then(Value::as_str),
                    Some("resolved") | Some("ignored")
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    for issue in unresolved_blockers.iter().take(20) {
        blockers.push(json!({
            "code": "ISSUE_UNRESOLVED",
            "targetId": issue.get("targetId").cloned().unwrap_or(Value::Null),
            "userMessage": issue.get("message").cloned().unwrap_or_else(|| Value::String(
                "这道题有需要处理的问题。".to_string()
            )),
            "action": "open_workspace",
            "internal": issue.get("issueId").cloned().unwrap_or(Value::Null)
        }));
    }
    if blockers.len() >= 20 {
        warnings.push(
            json!({ "code": "BLOCKER_LIST_TRUNCATED", "message": "问题较多，仅显示前 20 条。" }),
        );
    }

    json!({
        "schemaVersion": "PublishCheckResultV1",
        "jobId": job_id,
        "editVersion": edit_version,
        "passed": blockers.is_empty(),
        "blockers": blockers,
        "warnings": warnings
    })
}

fn collect_publish_gate_markers(
    value: &Value,
    path: &str,
    ai_fallbacks: &mut Vec<String>,
    partial_failures: &mut Vec<String>,
) {
    let lower_path = path.to_ascii_lowercase();
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{path}.{key}");
                let lower_key = key.to_ascii_lowercase();
                if child.as_bool() == Some(true)
                    && (matches!(
                        lower_key.as_str(),
                        "aifallback"
                            | "llmfallback"
                            | "fallbackused"
                            | "partialfailure"
                            | "aipartialfailure"
                            | "requiresmanualquestionimport"
                    ) || (lower_key == "fallback"
                        && (lower_path.contains("audit")
                            || lower_path.contains("llm")
                            || lower_path.contains("ai")
                            || lower_path.contains("suggestion")
                            || lower_path.contains("evidence"))))
                {
                    if lower_key.contains("fallback") {
                        ai_fallbacks.push(child_path.clone());
                    } else {
                        partial_failures.push(child_path.clone());
                    }
                }
                if child.as_array().is_some_and(|items| !items.is_empty())
                    && ((lower_key == "failures"
                        && (lower_path.contains("llm") || lower_path.contains("ai")))
                        || matches!(
                            lower_key.as_str(),
                            "blockedautoapplygroups" | "lowconfidencegroups"
                        ))
                {
                    partial_failures.push(child_path.clone());
                }
                if lower_key == "status"
                    && child.as_str().is_some_and(|status| {
                        matches!(
                            status.to_ascii_lowercase().as_str(),
                            "partial" | "partial_failure" | "needs_review" | "auto_apply_blocked"
                        )
                    })
                    && (lower_path.contains("llm")
                        || lower_path.contains("ai")
                        || lower_path.contains("vision")
                        || lower_path.contains("cloud")
                        || lower_path.contains("quality")
                        || lower_path.contains("audit"))
                {
                    partial_failures.push(child_path.clone());
                }
                if (lower_key == "code"
                    && child
                        .as_str()
                        .is_some_and(|code| code.eq_ignore_ascii_case("PARTIAL_RECOVERY_FAILURE")))
                    || (lower_key == "failure"
                        && json_value_is_nonempty(child)
                        && (lower_path.contains("llm")
                            || lower_path.contains("ai")
                            || lower_path.contains("vision")
                            || lower_path.contains("cloud")
                            || lower_path.contains("quality")
                            || lower_path.contains("audit")))
                {
                    partial_failures.push(child_path.clone());
                }
                collect_publish_gate_markers(child, &child_path, ai_fallbacks, partial_failures);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                collect_publish_gate_markers(
                    child,
                    &format!("{path}[{index}]"),
                    ai_fallbacks,
                    partial_failures,
                );
            }
        }
        Value::String(text) => {
            let lower = text.to_ascii_lowercase();
            if (lower.contains("ai fallback")
                || lower.contains("llm fallback")
                || lower.contains("rust-local-fallback")
                || lower.contains("deterministic-local-fallback"))
                && (lower_path.contains("note") || lower_path.contains("audit"))
            {
                ai_fallbacks.push(path.to_string());
            }
            if lower.contains("partial failure") && lower_path.contains("audit") {
                partial_failures.push(path.to_string());
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn json_value_is_nonempty(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

pub(crate) fn export_authoring_v2_core(root: &Path, input: Value) -> CommandResult<Value> {
    let input: ExportAuthoringV2Input = serde_json::from_value(input)
        .map_err(|error| format!("authoring_v2_invalid_export_request:{error}"))?;
    safe_job_dir(root, &input.job_id)?;
    let artifact_layout = ensure_job_artifact_layout(root, &input.job_id)?;
    let export_lock_path = artifact_layout
        .export_history_dir
        .join("phase5-export.lock");
    let export_lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&export_lock_path)
        .map_err(|error| format!("authoring_v2_export_lock_open:{error}"))?;
    export_lock
        .lock_exclusive()
        .map_err(|error| format!("authoring_v2_export_lock_acquire:{error}"))?;

    // ── M1 typed-preflight 直通（计划 §13.3/§13.4）────────────────────
    // 调用方显式传入 canonical DS（library_items_v2 权威稿）时：文件只是派生
    // 缓存，不再作为事实源读会话/校验文件 revision；发布门禁换成当前稿检查
    // （无历史 fallback/partial 扫描、无全局 humanVerified、无 SourceReview 文件依赖）。
    // staging/资产物化/原子 rename 与 legacy 完全同一条代码路径。
    let db_direct = input.authoring.is_some();
    // revision=0 是绑定标记：verify 据此以 shadow 文件（canonical 的派生缓存）为绑定源。
    // edit_version 单独记录在 manifest，仅供展示/审计。
    let (mut authoring_value, revision) = if db_direct {
        (input.authoring.clone().expect("checked above"), 0u64)
    } else {
        load_current_authoring(root, &input.job_id)?
    };
    if db_direct && authoring_value.get("jobId").and_then(Value::as_str) != Some(input.job_id.as_str())
    {
        // DB 直通专属前置检查；legacy 路径维持原有检查顺序（readiness 内做同一检查）。
        return Err("authoring_v2_export_blocked:job_id_mismatch".to_string());
    }
    if !db_direct {
        if let Some(expected_revision) = input.revision {
            if expected_revision != revision {
                return Err(format!(
                    "revision_conflict:current={revision}:requested={expected_revision}"
                ));
            }
        }
    }
    refresh_quality_report(root, &input.job_id, &mut authoring_value)?;
    let authoring: IeltsAuthoringIRV2 = serde_json::from_value(authoring_value.clone())
        .map_err(|error| format!("AUTHORING_SCHEMA_INVALID:{error}"))?;
    let quality_state = match &authoring.quality.state {
        crate::schema::quality_report_v2::ReadinessStateV2::Ready => "ready",
        crate::schema::quality_report_v2::ReadinessStateV2::ReviewRequired => "review_required",
        crate::schema::quality_report_v2::ReadinessStateV2::Blocked => "blocked",
    };
    if quality_state != "ready" {
        return Err(format!(
            "authoring_v2_export_blocked:quality_state={quality_state}"
        ));
    }
    let publish_proof: Value = if db_direct {
        let check = check_publish_preflight(&input.job_id, revision, &authoring_value);
        if !check
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(format!(
                "publish_check_failed:{}",
                serde_json::to_string(&check).unwrap_or_default()
            ));
        }
        check
    } else {
        validate_authoring_v2_publish_readiness(root, &input.job_id, revision, &authoring_value)?
    };
    let runtime = compile_reading_source_v2(&authoring).map_err(|issues| {
        format!(
            "authoring_v2_export_compile_blocked:{}",
            serde_json::to_string(&issues).unwrap_or_default()
        )
    })?;

    let export_dir = PathBuf::from(input.export_dir.trim());
    if !export_dir.is_absolute() {
        return Err("authoring_v2_export_dir_must_be_absolute".to_string());
    }
    fs::create_dir_all(&export_dir)
        .map_err(|error| format!("authoring_v2_export_dir_create:{error}"))?;
    let export_id = Uuid::new_v4().simple().to_string();
    let output_name = format!(
        "{}-r{}-{}",
        safe_export_component(&authoring.exam.exam_id),
        revision,
        export_id
    );
    let output_dir = export_dir.join(output_name);
    let staging_dir = export_dir.join(format!(".phase5-staging-{export_id}"));
    let journal_path = artifact_layout
        .export_history_dir
        .join(format!("phase5-v2-{export_id}.journal.json"));
    let write_journal = |status: &str, error: Option<&str>| -> CommandResult<()> {
        let journal = json!({
            "schemaVersion": "AuthoringV2ExportJournalV1",
            "jobId": input.job_id.clone(),
            "revision": revision,
            "status": status,
            "stagingDir": staging_dir.to_string_lossy().to_string(),
            "outputDir": output_dir.to_string_lossy().to_string(),
            "error": error,
            "updatedAt": Utc::now().to_rfc3339()
        });
        write_canonical_json_atomic(&journal_path, &journal).map(|_| ())
    };
    write_journal("staging", None)?;
    if let Err(error) = fs::create_dir_all(&staging_dir) {
        let create_error = format!("authoring_v2_export_staging_create:{error}");
        let _ = write_journal("failed", Some(&create_error));
        return Err(create_error);
    }
    let authoring_path = staging_dir.join("authoring-ir-v2.json");
    let runtime_path = staging_dir.join("reading-source-v2.json");
    let manifest_path = staging_dir.join("manifest-v2.json");
    let materialize_result: CommandResult<()> = (|| {
        let authoring_receipt = write_canonical_json_atomic(&authoring_path, &authoring_value)?;
        let runtime_value = serde_json::to_value(&runtime).map_err(|error| error.to_string())?;
        let runtime_receipt = write_canonical_json_atomic(&runtime_path, &runtime_value)?;
        materialize_authoring_assets(&artifact_layout.job_dir, &staging_dir, &authoring.assets)?;
        let manifest_value = json!({
            "schemaVersion": "AuthoringV2ExportReceiptV1",
            "jobId": input.job_id,
            "examId": authoring.exam.exam_id,
            "revision": revision,
            "editVersion": if db_direct { input.edit_version.unwrap_or(revision) } else { revision },
            "authoringSource": if db_direct { "canonical_ds" } else { "artifact_session" },
            "sourceDocumentId": authoring.source_document_id,
            "files": ["authoring-ir-v2.json", "reading-source-v2.json", "manifest-v2.json"],
            "authoringSha256": authoring_receipt.sha256,
            "runtimeSha256": runtime_receipt.sha256,
            "assetCount": authoring.assets.len(),
            "assets": authoring.assets.iter().map(|asset| json!({
                "assetId": &asset.asset_id,
                "kind": &asset.kind,
                "mime": &asset.mime,
                "relativePath": &asset.relative_path,
                "sha256": &asset.sha256,
                "byteLength": asset.byte_length
            })).collect::<Vec<_>>(),
            "v1FilesRemainReadable": true,
            "pdfPerQuestionLlmRepair": false,
            "reviewRequired": quality_state != "ready",
            "publishProof": publish_proof
        });
        write_canonical_json_atomic(&manifest_path, &manifest_value)?;
        fs::rename(&staging_dir, &output_dir)
            .map_err(|error| format!("authoring_v2_export_commit:{error}"))?;
        Ok(())
    })();
    if let Err(error) = materialize_result {
        let _ = fs::remove_dir_all(&staging_dir);
        if let Err(journal_error) = write_journal("failed", Some(&error)) {
            return Err(format!("{error};journal_write_failed:{journal_error}"));
        }
        return Err(error);
    }

    let receipt = json!({
        "schemaVersion": "AuthoringV2ExportReceiptV1",
        "jobId": input.job_id.clone(),
        "examId": authoring.exam.exam_id,
        "revision": revision,
        "outputDir": output_dir,
        "authoringPath": output_dir.join("authoring-ir-v2.json"),
        "runtimePath": output_dir.join("reading-source-v2.json"),
        "manifestPath": output_dir.join("manifest-v2.json"),
        "v1FilesRemainReadable": true,
        "pdfPerQuestionLlmRepair": false,
        "publishProof": publish_proof
    });
    let history_path = format!("export-history/phase5-v2-{}-{}.json", revision, export_id);
    let history_file_path = artifact_layout.job_dir.join(&history_path);
    let history_receipt = match write_artifact_json(root, &input.job_id, &history_path, &receipt) {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = fs::remove_dir_all(&output_dir);
            let _ = write_journal("failed", Some(&error));
            return Err(error);
        }
    };
    if let Err(error) = write_journal("committed", None) {
        let _ = fs::remove_file(&history_file_path);
        let _ = fs::remove_dir_all(&output_dir);
        let message = format!("authoring_v2_export_journal_commit:{error}");
        let _ = write_journal("failed", Some(&message));
        return Err(message);
    }
    Ok(json!({
        "receipt": receipt,
        "history": history_receipt,
        "outputDir": output_dir,
        "revision": revision,
        "examId": authoring.exam.exam_id
    }))
}

fn safe_asset_relative_path(raw: &str) -> CommandResult<PathBuf> {
    let path = PathBuf::from(raw.trim());
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("authoring_v2_asset_path_unsafe:{raw}"));
    }
    for component in path.components() {
        if let Component::Normal(value) = component {
            let segment = value.to_string_lossy();
            if !is_safe_path_segment(&segment) {
                return Err(format!("authoring_v2_asset_path_unsafe:{raw}"));
            }
        }
    }
    Ok(path)
}

fn materialize_authoring_assets(
    source_root: &Path,
    staging_dir: &Path,
    assets: &[AssetDescriptorV2],
) -> CommandResult<()> {
    if assets.is_empty() {
        return Ok(());
    }
    let source_root = fs::canonicalize(source_root)
        .map_err(|error| format!("authoring_v2_asset_root_unavailable:{error}"))?;
    let mut seen_paths = BTreeSet::new();
    for asset in assets {
        let relative = safe_asset_relative_path(&asset.relative_path)?;
        if !seen_paths.insert(relative.clone()) {
            return Err(format!(
                "authoring_v2_asset_relative_path_duplicate:{}",
                asset.relative_path
            ));
        }
        let source = source_root.join(&relative);
        let source_real = fs::canonicalize(&source).map_err(|error| {
            format!(
                "authoring_v2_asset_source_missing:{}:{error}",
                asset.asset_id
            )
        })?;
        if !source_real.starts_with(&source_root) {
            return Err(format!(
                "authoring_v2_asset_source_escape:{}",
                asset.asset_id
            ));
        }
        let target = staging_dir.join(&relative);
        let (actual_hash, actual_size) = stage_file_with_hash(&source_real, &target)
            .map_err(|error| format!("authoring_v2_asset_stage:{}:{error}", asset.asset_id))?;
        if actual_size != asset.byte_length {
            let _ = fs::remove_file(&target);
            return Err(format!(
                "authoring_v2_asset_size_mismatch:{}:expected={}:actual={}",
                asset.asset_id, asset.byte_length, actual_size
            ));
        }
        if !actual_hash.eq_ignore_ascii_case(&asset.sha256) {
            let _ = fs::remove_file(&target);
            return Err(format!(
                "authoring_v2_asset_hash_mismatch:{}:expected={}:actual={}",
                asset.asset_id, asset.sha256, actual_hash
            ));
        }
    }
    Ok(())
}

fn safe_export_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "exam".to_string()
    } else {
        sanitized
    }
}

fn build_editor_session(root: &Path, job_id: &str) -> CommandResult<Value> {
    let (authoring, revision) = load_current_authoring(root, job_id)?;
    let source = if revision > 0 { "revision" } else { "shadow" };
    Ok(json!({
        "schemaVersion": SESSION_SCHEMA_VERSION,
        "jobId": job_id,
        "authoring": authoring,
        "revision": revision,
        "source": source,
        "revisions": list_revision_records(root, job_id)?,
        "v1FilesRemainReadable": true,
    }))
}

fn load_current_authoring(root: &Path, job_id: &str) -> CommandResult<(Value, u64)> {
    let current = recover_current_revision(root, job_id)?;
    if current.revision > 0 {
        let value = read_revision(root, job_id, current.revision)?;
        validate_authoring(&value)?;
        return Ok((value, current.revision));
    }

    let path = job_dir(root, job_id).join(AUTHORING_V2_SHADOW_FILE);
    let value = read_json_opt(&path)?.ok_or_else(|| {
        format!(
            "AUTHORING_V2_NOT_AVAILABLE:shadow_missing:{}",
            path.display()
        )
    })?;
    validate_authoring(&value)?;
    Ok((value, 0))
}

/// M1 起 library::commands 在 DB 保存后复用质量重算（canonical → 派生质量块）。
pub(crate) fn refresh_quality_report(
    root: &Path,
    job_id: &str,
    authoring: &mut Value,
) -> CommandResult<()> {
    let previous_quality = authoring.get("quality").cloned();
    let physical_shadow = read_json_opt(&job_dir(root, job_id).join(DOCUMENT_V2_SHADOW_FILE))?
        .filter(|shadow| physical_shadow_matches_authoring(shadow, authoring));
    let mut quality = evaluate_quality(authoring, physical_shadow.as_ref());
    preserve_issue_resolutions(&mut quality, previous_quality.as_ref());
    authoring
        .as_object_mut()
        .ok_or_else(|| "AUTHORING_SCHEMA_INVALID:authoring must be an object".to_string())?
        .insert("quality".to_string(), quality);
    Ok(())
}

fn physical_shadow_matches_authoring(shadow: &Value, authoring: &Value) -> bool {
    let authoring_source_ids = authoring
        .get("exam")
        .and_then(|exam| exam.get("sourceFiles"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|source| source.get("sourceFileId").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let mut physical_source_ids = shadow
        .get("sourceFiles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|source| source.get("sourceFileId").and_then(Value::as_str));
    shadow.get("schemaVersion").and_then(Value::as_str) == Some("DocumentIRV2")
        && shadow.get("jobId").and_then(Value::as_str)
            == authoring.get("jobId").and_then(Value::as_str)
        && shadow.get("documentId").and_then(Value::as_str)
            == authoring.get("sourceDocumentId").and_then(Value::as_str)
        && physical_source_ids.any(|source_id| authoring_source_ids.contains(source_id))
}

fn preserve_issue_resolutions(quality: &mut Value, previous_quality: Option<&Value>) {
    let previous_details = previous_quality
        .and_then(|value| value.get("issues"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|issue| {
            let issue_id = issue.get("issueId").and_then(Value::as_str)?;
            let details = issue.get("details").and_then(Value::as_object)?;
            let resolution = details.get("resolution").and_then(Value::as_str)?;
            if !matches!(resolution, "resolved" | "ignored") {
                return None;
            }
            let mut preserved = BTreeMap::new();
            preserved.insert(
                "resolution".to_string(),
                Value::String(resolution.to_string()),
            );
            if let Some(note) = details.get("note") {
                preserved.insert("note".to_string(), note.clone());
            }
            Some((issue_id.to_string(), preserved))
        })
        .collect::<BTreeMap<_, _>>();

    let Some(issues) = quality.get_mut("issues").and_then(Value::as_array_mut) else {
        return;
    };
    for issue in issues {
        let Some(issue_id) = issue.get("issueId").and_then(Value::as_str) else {
            continue;
        };
        let Some(preserved) = previous_details.get(issue_id) else {
            continue;
        };
        let Some(issue_object) = issue.as_object_mut() else {
            continue;
        };
        let details = issue_object
            .entry("details")
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(details_object) = details.as_object_mut() else {
            continue;
        };
        for (key, value) in preserved {
            details_object.insert(key.clone(), value.clone());
        }
    }
}

/// M1 起 DB 编辑事务复用同一 schema 校验。
pub(crate) fn validate_authoring(value: &Value) -> CommandResult<()> {
    if value.get("schemaVersion").and_then(Value::as_str) != Some("IeltsAuthoringIRV2") {
        return Err("AUTHORING_SCHEMA_INVALID:expected=IeltsAuthoringIRV2".to_string());
    }
    serde_json::from_value::<IeltsAuthoringIRV2>(value.clone())
        .map(|_| ())
        .map_err(|error| format!("AUTHORING_SCHEMA_INVALID:{error}"))
}

/// Record that this revision came from a human edit.
///
/// This used to hardcode `audit.humanVerified = false`. The export gate requires that flag to be
/// true and V2 has no path that ever sets it back (V1 derives it in
/// `authoring_review::refresh_authoring_review_state`; V2 has no equivalent), so the flag was
/// monotonically false and the FIRST edit permanently blocked publishing. That inverted the
/// intent: it did not protect students, it forced authors to publish an unedited draft or not at
/// all. A save is by definition a human acting on the document, so an already-verified document
/// stays verified; an unverified one stays unverified. Content safety still comes from the rest of
/// the gate -- zero unresolved blocker issues, no unresolved answers, quality `ready`, compiler
/// pass, asset closure -- all recomputed from the current document on every export.
fn mark_user_audit(document: &mut Value, revision: u64) {
    if let Some(audit) = document.get_mut("audit").and_then(Value::as_object_mut) {
        let already_verified = audit.get("humanVerified").and_then(Value::as_bool) == Some(true);
        audit.insert("revision".to_string(), json!(revision));
        audit.insert("source".to_string(), json!("user"));
        audit.insert("humanVerified".to_string(), json!(already_verified));
        audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
    }
}

/// 单条 AuthoringPatchV2 应用（对 `serde_json::Value` 形态的权威稿操作）。
/// M1 起 `library::repository` 的编辑事务复用同一套 patch 语义，
/// 保证文件链（legacy）与 DB 链（canonical）行为一致。
pub(crate) fn apply_patch(document: &mut Value, patch: &Value) -> CommandResult<()> {
    let object = patch
        .as_object()
        .ok_or_else(|| "authoring_v2_patch_must_be_object".to_string())?;
    let op = required_string(object, "op")?;
    match op {
        "replaceText" => replace_text(document, object),
        "setNodeAttrs" => set_node_attrs(document, object),
        "replaceContent" => replace_content(document, object),
        "insertNode" => insert_node(document, object),
        "deleteNode" => delete_node(document, object),
        "moveNode" => move_node(document, object),
        "cropAsset" => crop_asset(document, object),
        "setHotspot" => set_hotspot(document, object),
        "removeHotspot" => remove_hotspot(document, object),
        "setTaskType" => set_task_type(document, object),
        "setQuestionExpression" => set_question_expression(document, object),
        "setResponseCardinality" => set_response_cardinality(document, object),
        "setResponseGroup" => set_response_group(document, object),
        "setOptionBank" => set_option_bank(document, object),
        "insertAnswerSlot" => insert_answer_slot(document, object),
        "deleteAnswerSlot" => delete_answer_slot(document, object),
        "setAnswer" => set_answer(document, object),
        "bindSource" => bind_source(document, object),
        "resolveIssue" => resolve_issue(document, object),
        _ => Err(format!("AUTHORING_PATCH_UNSUPPORTED:{op}")),
    }
}

fn replace_text(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let node_id = required_string(patch, "nodeId")?;
    let from = required_u64(patch, "from")? as usize;
    let to = required_u64(patch, "to")? as usize;
    let text = required_string(patch, "text")?;
    if text.chars().count() > 100_000 {
        return Err("AUTHORING_PATCH_TEXT_TOO_LARGE:max=100000".to_string());
    }
    let node = find_object_mut_by_id(document, node_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"))?;
    if node.get("type").and_then(Value::as_str) != Some("text") {
        return Err(format!("AUTHORING_PATCH_TEXT_NODE_REQUIRED:{node_id}"));
    }
    let original = node
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("AUTHORING_PATCH_TEXT_MISSING:{node_id}"))?;
    let chars = original.chars().collect::<Vec<_>>();
    if from > to || to > chars.len() {
        return Err(format!(
            "AUTHORING_PATCH_TEXT_RANGE_INVALID:{node_id}:from={from}:to={to}:length={}",
            chars.len()
        ));
    }
    let mut next = chars[..from].iter().collect::<String>();
    next.push_str(text);
    next.extend(chars[to..].iter());
    node.insert("text".to_string(), Value::String(next));
    mark_user_edited(
        node,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn set_node_attrs(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let node_id = required_string(patch, "nodeId")?;
    let attrs = patch
        .get("attrs")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_ATTRS_REQUIRED".to_string())?;
    let node = find_object_mut_by_id(document, node_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"))?;
    for (key, value) in attrs {
        if !is_safe_node_attribute(key) {
            return Err(format!("AUTHORING_PATCH_ATTR_NOT_ALLOWED:{key}"));
        }
        node.insert(key.clone(), value.clone());
    }
    if let Some(remove_attrs) = patch.get("removeAttrs").and_then(Value::as_array) {
        for key in remove_attrs {
            let key = key
                .as_str()
                .ok_or_else(|| "AUTHORING_PATCH_REMOVE_ATTRS_INVALID".to_string())?;
            if !is_safe_node_attribute(key) {
                return Err(format!("AUTHORING_PATCH_ATTR_NOT_ALLOWED:{key}"));
            }
            node.remove(key);
        }
    }
    mark_user_edited(
        node,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn replace_content(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let target = patch
        .get("target")
        .ok_or_else(|| "AUTHORING_PATCH_CONTENT_TARGET_REQUIRED".to_string())?;
    let content = patch
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| "AUTHORING_PATCH_CONTENT_REQUIRED".to_string())?
        .clone();
    let mut existing_answer_slots = Vec::new();
    with_content_target_mut(document, target, |nodes| {
        answer_slot_ids_in_values(nodes, &mut existing_answer_slots);
        Ok(())
    })?;
    let mut next_answer_slots = Vec::new();
    answer_slot_ids_in_values(&content, &mut next_answer_slots);
    let removed = existing_answer_slots
        .into_iter()
        .filter(|slot_id| !next_answer_slots.iter().any(|next| next == slot_id))
        .collect::<Vec<_>>();
    if !removed.is_empty() {
        return Err(format!(
            "AUTHORING_PATCH_ANSWER_SLOT_LOSS:{}",
            removed.join(",")
        ));
    }
    with_content_target_mut(document, target, |nodes| {
        *nodes = content;
        Ok(())
    })
}

fn insert_node(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let target = patch
        .get("target")
        .ok_or_else(|| "AUTHORING_PATCH_CONTENT_TARGET_REQUIRED".to_string())?;
    let index = required_u64(patch, "index")? as usize;
    let node = patch
        .get("node")
        .ok_or_else(|| "AUTHORING_PATCH_NODE_REQUIRED".to_string())?
        .clone();
    let parent_id = patch.get("parentId").and_then(Value::as_str);
    with_content_parent_mut(document, target, parent_id, |nodes| {
        if index > nodes.len() {
            return Err(format!("AUTHORING_PATCH_INDEX_INVALID:{index}"));
        }
        nodes.insert(index, node);
        Ok(())
    })
}

fn delete_node(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let node_id = required_string(patch, "nodeId")?;
    let contains_answer_slot = find_object_mut_by_id(document, node_id)
        .map(|node| value_contains_content_type(&Value::Object(node.clone()), "answer_slot"))
        .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"))?;
    if contains_answer_slot
        && !patch
            .get("allowAnswerSlotRemoval")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Err(format!(
            "AUTHORING_PATCH_NODE_CONTAINS_ANSWER_SLOT:{node_id}"
        ));
    }
    remove_content_node(document, node_id)
        .map(|_| ())
        .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"))
}

fn move_node(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let node_id = required_string(patch, "nodeId")?;
    let target = patch
        .get("target")
        .ok_or_else(|| "AUTHORING_PATCH_CONTENT_TARGET_REQUIRED".to_string())?;
    let index = required_u64(patch, "index")? as usize;
    let parent_id = patch.get("parentId").and_then(Value::as_str);
    let backup = document.clone();
    let Some((node, _old_index)) = remove_content_node(document, node_id) else {
        return Err(format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"));
    };
    let result = with_content_parent_mut(document, target, parent_id, |nodes| {
        // `index` is the insertion index after the source node has been removed.
        // This matches the editor's move-up/move-down contract and also works
        // when the destination is a different parent container.
        if index > nodes.len() {
            return Err(format!("AUTHORING_PATCH_INDEX_INVALID:{index}"));
        }
        nodes.insert(index, node);
        Ok(())
    });
    if result.is_err() {
        // Preserve the document atomically if the destination is invalid.
        *document = backup;
        return result;
    }
    Ok(())
}

fn crop_asset(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let node_id = required_string(patch, "nodeId")?;
    let node = find_object_mut_by_id(document, node_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"))?;
    if !matches!(
        node.get("type").and_then(Value::as_str),
        Some("figure" | "image" | "diagram")
    ) {
        return Err(format!("AUTHORING_PATCH_ASSET_NODE_REQUIRED:{node_id}"));
    }
    let crop = patch.get("crop");
    if let Some(crop) = crop.filter(|value| !value.is_null()) {
        validate_normalized_rect(crop, "CROP")?;
        node.insert("crop".to_string(), crop.clone());
    } else {
        node.remove("crop");
    }
    mark_user_edited(
        node,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn set_hotspot(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let node_id = required_string(patch, "nodeId")?;
    let hotspot = patch
        .get("hotspot")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_HOTSPOT_REQUIRED".to_string())?;
    let hotspot_id = hotspot
        .get("hotspotId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "AUTHORING_PATCH_HOTSPOT_ID_REQUIRED".to_string())?;
    let slot_id = hotspot
        .get("slotId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "AUTHORING_PATCH_HOTSPOT_SLOT_ID_REQUIRED".to_string())?;
    let _ = slot_id;
    validate_normalized_rect(
        hotspot
            .get("normalizedRect")
            .ok_or_else(|| "AUTHORING_PATCH_HOTSPOT_RECT_REQUIRED".to_string())?,
        "HOTSPOT",
    )?;
    if let Some(anchor) = hotspot.get("labelAnchor") {
        validate_normalized_anchor(anchor)?;
    }
    let node = find_object_mut_by_id(document, node_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"))?;
    if !matches!(
        node.get("type").and_then(Value::as_str),
        Some("figure" | "diagram")
    ) {
        return Err(format!("AUTHORING_PATCH_HOTSPOT_NODE_REQUIRED:{node_id}"));
    }
    let hotspots = node
        .entry("hotspots")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| "AUTHORING_PATCH_HOTSPOTS_INVALID".to_string())?;
    if let Some(existing) = hotspots
        .iter_mut()
        .find(|item| item.get("hotspotId").and_then(Value::as_str) == Some(hotspot_id))
    {
        *existing = Value::Object(hotspot.clone());
    } else {
        hotspots.push(Value::Object(hotspot.clone()));
    }
    mark_user_edited(
        node,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn remove_hotspot(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let node_id = required_string(patch, "nodeId")?;
    let hotspot_id = required_string(patch, "hotspotId")?;
    let node = find_object_mut_by_id(document, node_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"))?;
    if !matches!(
        node.get("type").and_then(Value::as_str),
        Some("figure" | "diagram")
    ) {
        return Err(format!("AUTHORING_PATCH_HOTSPOT_NODE_REQUIRED:{node_id}"));
    }
    let hotspots = node
        .get_mut("hotspots")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("AUTHORING_PATCH_HOTSPOT_NOT_FOUND:{hotspot_id}"))?;
    let before = hotspots.len();
    hotspots.retain(|item| item.get("hotspotId").and_then(Value::as_str) != Some(hotspot_id));
    if hotspots.len() == before {
        return Err(format!("AUTHORING_PATCH_HOTSPOT_NOT_FOUND:{hotspot_id}"));
    }
    mark_user_edited(
        node,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn set_task_type(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let task_id = required_string(patch, "taskId")?;
    let task_type = required_string(patch, "taskType")?;
    if !is_supported_task_type(task_type) {
        return Err(format!("AUTHORING_PATCH_TASK_TYPE_INVALID:{task_type}"));
    }
    let task = find_object_by_field_mut(document, "taskId", task_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?;
    task.insert("taskType".to_string(), Value::String(task_type.to_string()));
    if let Some(signature) = task
        .get_mut("instructionSignature")
        .and_then(Value::as_object_mut)
    {
        signature.insert("taskType".to_string(), Value::String(task_type.to_string()));
    }
    mark_user_edited(
        task,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn set_question_expression(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let task_id = required_string(patch, "taskId")?;
    let expression = patch
        .get("expression")
        .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_REQUIRED".to_string())?;
    let expected_numbers = expand_question_expression(expression)?;
    let task = find_object_by_field_mut(document, "taskId", task_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?;
    task.insert("displayRange".to_string(), expression.clone());
    if let Some(signature) = task
        .get_mut("instructionSignature")
        .and_then(Value::as_object_mut)
    {
        signature.insert(
            "expectedQuestionNumbers".to_string(),
            json!(expected_numbers),
        );
        signature.insert(
            "expectedSlotCount".to_string(),
            json!(expected_numbers.len()),
        );
    }
    mark_user_edited(
        task,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn set_response_cardinality(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let task_id = required_string(patch, "taskId")?;
    let response_id = required_string(patch, "responseGroupId")?;
    let cardinality = patch
        .get("cardinality")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_CARDINALITY_REQUIRED".to_string())?;
    let min = cardinality
        .get("min")
        .and_then(Value::as_u64)
        .ok_or_else(|| "AUTHORING_PATCH_CARDINALITY_MIN_REQUIRED".to_string())?;
    let max = cardinality
        .get("max")
        .and_then(Value::as_u64)
        .ok_or_else(|| "AUTHORING_PATCH_CARDINALITY_MAX_REQUIRED".to_string())?;
    let exact = cardinality.get("exact").and_then(Value::as_u64);
    if max < min || exact.is_some_and(|value| value < min || value > max) {
        return Err("AUTHORING_PATCH_CARDINALITY_INVALID".to_string());
    }
    let task = find_object_by_field_mut(document, "taskId", task_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?;
    let groups = task
        .get_mut("responseGroups")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("AUTHORING_PATCH_RESPONSE_GROUPS_MISSING:{task_id}"))?;
    let group = groups
        .iter_mut()
        .find(|item| item.get("responseGroupId").and_then(Value::as_str) == Some(response_id))
        .ok_or_else(|| format!("AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:{response_id}"))?;
    let group = group
        .as_object_mut()
        .ok_or_else(|| "AUTHORING_PATCH_RESPONSE_GROUP_INVALID".to_string())?;
    group.insert(
        "cardinality".to_string(),
        json!({"min": min, "max": max, "exact": exact}),
    );
    mark_user_edited(
        group,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn set_response_group(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let task_id = required_string(patch, "taskId")?;
    let response = patch
        .get("responseGroup")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_RESPONSE_GROUP_REQUIRED".to_string())?;
    let response_id = response
        .get("responseGroupId")
        .and_then(Value::as_str)
        .ok_or_else(|| "AUTHORING_PATCH_RESPONSE_GROUP_ID_REQUIRED".to_string())?;
    let task = find_object_by_field_mut(document, "taskId", task_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?;
    let groups = task
        .get_mut("responseGroups")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("AUTHORING_PATCH_RESPONSE_GROUPS_MISSING:{task_id}"))?;
    let target = groups
        .iter_mut()
        .find(|item| item.get("responseGroupId").and_then(Value::as_str) == Some(response_id));
    let Some(target) = target else {
        return Err(format!(
            "AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:{response_id}"
        ));
    };
    *target = Value::Object(response.clone());
    mark_user_edited(
        task,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn set_option_bank(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let task_id = required_string(patch, "taskId")?;
    let option_bank = patch
        .get("optionBank")
        .ok_or_else(|| "AUTHORING_PATCH_OPTION_BANK_REQUIRED".to_string())?;
    if !option_bank.is_null() && !option_bank.is_object() {
        return Err("AUTHORING_PATCH_OPTION_BANK_INVALID".to_string());
    }
    let task = find_object_by_field_mut(document, "taskId", task_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?;
    if option_bank.is_null() {
        task.remove("optionBank");
    } else {
        task.insert("optionBank".to_string(), option_bank.clone());
    }
    mark_user_edited(
        task,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn insert_answer_slot(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let task_id = required_string(patch, "taskId")?;
    let response_id = required_string(patch, "responseGroupId")?;
    let slot_index = required_u64(patch, "slotIndex")? as usize;
    let node = patch
        .get("node")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_SLOT_NODE_REQUIRED".to_string())?;
    let slot = patch
        .get("slot")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_SLOT_REQUIRED".to_string())?;
    let value = patch
        .get("value")
        .ok_or_else(|| "AUTHORING_PATCH_ANSWER_REQUIRED".to_string())?;
    let slot_id = slot
        .get("slotId")
        .and_then(Value::as_str)
        .ok_or_else(|| "AUTHORING_PATCH_SLOT_ID_REQUIRED".to_string())?;
    if node.get("type").and_then(Value::as_str) != Some("answer_slot")
        || node.get("slotId").and_then(Value::as_str) != Some(slot_id)
    {
        return Err("AUTHORING_PATCH_SLOT_NODE_MISMATCH".to_string());
    }
    let expression = patch
        .get("expression")
        .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_REQUIRED".to_string())?;
    expand_question_expression(expression)?;
    if document
        .get("answerSlots")
        .and_then(Value::as_object)
        .is_some_and(|slots| slots.contains_key(slot_id))
    {
        return Err(format!("AUTHORING_PATCH_SLOT_ALREADY_EXISTS:{slot_id}"));
    }
    let backup = document.clone();
    let result = (|| {
        insert_node(document, patch)?;
        document
            .get_mut("answerSlots")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "AUTHORING_PATCH_ANSWER_SLOTS_MISSING".to_string())?
            .insert(slot_id.to_string(), Value::Object(slot.clone()));
        document
            .get_mut("answerKey")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "AUTHORING_PATCH_ANSWER_KEY_MISSING".to_string())?
            .insert(slot_id.to_string(), value.clone());
        let task = find_object_by_field_mut(document, "taskId", task_id)
            .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?;
        let group = task
            .get_mut("responseGroups")
            .and_then(Value::as_array_mut)
            .and_then(|groups| {
                groups.iter_mut().find(|item| {
                    item.get("responseGroupId").and_then(Value::as_str) == Some(response_id)
                })
            })
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:{response_id}"))?;
        let slot_ids = group
            .get_mut("slotIds")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "AUTHORING_PATCH_SLOT_IDS_MISSING".to_string())?;
        if slot_index > slot_ids.len() {
            return Err(format!("AUTHORING_PATCH_INDEX_INVALID:{slot_index}"));
        }
        slot_ids.insert(slot_index, Value::String(slot_id.to_string()));
        set_question_expression(document, patch)
    })();
    if result.is_err() {
        *document = backup;
    }
    result
}

fn delete_answer_slot(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let task_id = required_string(patch, "taskId")?;
    let response_id = required_string(patch, "responseGroupId")?;
    let node_id = required_string(patch, "nodeId")?;
    let slot_id = required_string(patch, "slotId")?;
    let expression = patch
        .get("expression")
        .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_REQUIRED".to_string())?;
    expand_question_expression(expression)?;
    let node = find_object_mut_by_id(document, node_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_SLOT_NODE_NOT_FOUND:{slot_id}"))?;
    if node.get("type").and_then(Value::as_str) != Some("answer_slot")
        || node.get("slotId").and_then(Value::as_str) != Some(slot_id)
    {
        return Err(format!("AUTHORING_PATCH_SLOT_NODE_NOT_FOUND:{slot_id}"));
    }
    let backup = document.clone();
    let result = (|| {
        remove_content_node(document, node_id)
            .ok_or_else(|| format!("AUTHORING_PATCH_SLOT_NODE_NOT_FOUND:{slot_id}"))?;
        document
            .get_mut("answerSlots")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "AUTHORING_PATCH_ANSWER_SLOTS_MISSING".to_string())?
            .remove(slot_id)
            .ok_or_else(|| format!("AUTHORING_PATCH_SLOT_NOT_FOUND:{slot_id}"))?;
        document
            .get_mut("answerKey")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "AUTHORING_PATCH_ANSWER_KEY_MISSING".to_string())?
            .remove(slot_id);
        let task = find_object_by_field_mut(document, "taskId", task_id)
            .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?;
        let group = task
            .get_mut("responseGroups")
            .and_then(Value::as_array_mut)
            .and_then(|groups| {
                groups.iter_mut().find(|item| {
                    item.get("responseGroupId").and_then(Value::as_str) == Some(response_id)
                })
            })
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:{response_id}"))?;
        let slot_ids = group
            .get_mut("slotIds")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "AUTHORING_PATCH_SLOT_IDS_MISSING".to_string())?;
        slot_ids.retain(|value| value.as_str() != Some(slot_id));
        set_question_expression(document, patch)
    })();
    if result.is_err() {
        *document = backup;
    }
    result
}

fn set_answer(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let slot_id = required_string(patch, "slotId")?;
    let value = patch
        .get("value")
        .ok_or_else(|| "AUTHORING_PATCH_ANSWER_REQUIRED".to_string())?;
    let slots = document
        .get("answerSlots")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_ANSWER_SLOTS_MISSING".to_string())?;
    if !slots.contains_key(slot_id) {
        return Err(format!("AUTHORING_PATCH_SLOT_NOT_FOUND:{slot_id}"));
    }
    let answer = document
        .get_mut("answerKey")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "AUTHORING_PATCH_ANSWER_KEY_MISSING".to_string())?;
    if !matches!(
        value.get("kind").and_then(Value::as_str),
        Some("text" | "option" | "unresolved")
    ) {
        return Err(format!("AUTHORING_PATCH_ANSWER_KIND_INVALID:{slot_id}"));
    }
    answer.insert(slot_id.to_string(), value.clone());
    Ok(())
}

fn bind_source(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let entity_id = required_string(patch, "entityId")?;
    let anchors = patch
        .get("anchors")
        .and_then(Value::as_array)
        .ok_or_else(|| "AUTHORING_PATCH_ANCHORS_REQUIRED".to_string())?;
    if anchors.iter().any(|anchor| !anchor.is_object()) {
        return Err("AUTHORING_PATCH_ANCHORS_INVALID".to_string());
    }
    let entity = find_object_by_any_id_mut(document, entity_id)
        .ok_or_else(|| format!("AUTHORING_PATCH_ENTITY_NOT_FOUND:{entity_id}"))?;
    entity.insert("sourceAnchors".to_string(), Value::Array(anchors.clone()));
    mark_user_edited(
        entity,
        preserve_provenance(patch),
        restore_provenance_status(patch),
    );
    Ok(())
}

fn resolve_issue(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    let issue_id = required_string(patch, "issueId")?;
    let resolution = required_string(patch, "resolution")?;
    if !matches!(resolution, "resolved" | "ignored") {
        return Err(format!(
            "AUTHORING_PATCH_ISSUE_RESOLUTION_INVALID:{resolution}"
        ));
    }
    let issues = document
        .get_mut("quality")
        .and_then(Value::as_object_mut)
        .and_then(|quality| quality.get_mut("issues"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "AUTHORING_PATCH_ISSUES_MISSING".to_string())?;
    let issue = issues
        .iter_mut()
        .find(|item| item.get("issueId").and_then(Value::as_str) == Some(issue_id))
        .ok_or_else(|| format!("AUTHORING_PATCH_ISSUE_NOT_FOUND:{issue_id}"))?;
    let details = issue
        .as_object_mut()
        .and_then(|object| {
            object
                .entry("details")
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
        })
        .ok_or_else(|| "AUTHORING_PATCH_ISSUE_DETAILS_INVALID".to_string())?;
    details.insert(
        "resolution".to_string(),
        Value::String(resolution.to_string()),
    );
    if let Some(note) = patch.get("note").and_then(Value::as_str) {
        details.insert("note".to_string(), Value::String(note.to_string()));
    }
    Ok(())
}

fn with_content_target_mut<F>(
    document: &mut Value,
    target: &Value,
    callback: F,
) -> CommandResult<()>
where
    F: FnOnce(&mut Vec<Value>) -> CommandResult<()>,
{
    let target = target
        .as_object()
        .ok_or_else(|| "AUTHORING_PATCH_CONTENT_TARGET_INVALID".to_string())?;
    let kind = target
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "AUTHORING_PATCH_CONTENT_TARGET_KIND_REQUIRED".to_string())?;
    let node = match kind {
        "node" => {
            let node_id = target
                .get("nodeId")
                .and_then(Value::as_str)
                .ok_or_else(|| "AUTHORING_PATCH_CONTENT_TARGET_NODE_REQUIRED".to_string())?;
            find_object_mut_by_id(document, node_id)
                .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{node_id}"))?
        }
        "passage" => document
            .get_mut("passage")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "AUTHORING_PATCH_PASSAGE_NOT_FOUND".to_string())?,
        "taskInstructions" | "taskStimulus" => {
            let task_id = target
                .get("taskId")
                .and_then(Value::as_str)
                .ok_or_else(|| "AUTHORING_PATCH_TASK_ID_REQUIRED".to_string())?;
            find_object_by_field_mut(document, "taskId", task_id)
                .ok_or_else(|| format!("AUTHORING_PATCH_TASK_NOT_FOUND:{task_id}"))?
        }
        "responsePrompt" => {
            let response_id = target
                .get("responseGroupId")
                .and_then(Value::as_str)
                .ok_or_else(|| "AUTHORING_PATCH_RESPONSE_GROUP_ID_REQUIRED".to_string())?;
            find_object_by_field_mut(document, "responseGroupId", response_id)
                .ok_or_else(|| format!("AUTHORING_PATCH_RESPONSE_GROUP_NOT_FOUND:{response_id}"))?
        }
        "option" => {
            let option_id = target
                .get("optionId")
                .and_then(Value::as_str)
                .ok_or_else(|| "AUTHORING_PATCH_OPTION_ID_REQUIRED".to_string())?;
            find_object_by_field_mut(document, "optionId", option_id)
                .ok_or_else(|| format!("AUTHORING_PATCH_OPTION_NOT_FOUND:{option_id}"))?
        }
        _ => return Err(format!("AUTHORING_PATCH_CONTENT_TARGET_UNSUPPORTED:{kind}")),
    };
    let key = match kind {
        "passage" | "option" => "content",
        "taskInstructions" => "instructions",
        "taskStimulus" => "stimulus",
        "responsePrompt" => "prompt",
        "node" => "children",
        _ => unreachable!(),
    };
    if kind == "node" {
        let array = content_array_mut(node)
            .ok_or_else(|| "AUTHORING_PATCH_NODE_NOT_CONTAINER".to_string())?;
        return callback(array);
    }
    let field = node
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let array = field
        .as_array_mut()
        .ok_or_else(|| format!("AUTHORING_PATCH_CONTENT_TARGET_NOT_ARRAY:{kind}"))?;
    callback(array)
}

fn with_content_parent_mut<F>(
    document: &mut Value,
    target: &Value,
    parent_id: Option<&str>,
    callback: F,
) -> CommandResult<()>
where
    F: FnOnce(&mut Vec<Value>) -> CommandResult<()>,
{
    if let Some(parent_id) = parent_id {
        let parent = find_object_mut_by_id(document, parent_id)
            .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_FOUND:{parent_id}"))?;
        let nodes = content_array_mut(parent)
            .ok_or_else(|| format!("AUTHORING_PATCH_NODE_NOT_CONTAINER:{parent_id}"))?;
        return callback(nodes);
    }
    with_content_target_mut(document, target, callback)
}

fn content_array_mut(object: &mut Map<String, Value>) -> Option<&mut Vec<Value>> {
    let key = ["children", "items", "rows", "cells", "caption", "steps"]
        .into_iter()
        .find(|key| {
            object
                .get(*key)
                .and_then(Value::as_array)
                .map(|array| array.iter().all(|item| item.is_object()))
                .unwrap_or(false)
        })?;
    object.get_mut(key).and_then(Value::as_array_mut)
}

fn is_content_node_object(object: &Map<String, Value>) -> bool {
    matches!(
        object.get("type").and_then(Value::as_str),
        Some(
            "doc"
                | "paragraph"
                | "heading"
                | "text"
                | "hard_break"
                | "bullet_list"
                | "ordered_list"
                | "list_item"
                | "table"
                | "table_row"
                | "table_cell"
                | "figure"
                | "image"
                | "figcaption"
                | "flowchart"
                | "flow_step"
                | "diagram"
                | "answer_slot"
                | "option_bank"
                | "horizontal_rule"
        )
    )
}

fn remove_content_node(value: &mut Value, node_id: &str) -> Option<(Value, usize)> {
    match value {
        Value::Array(items) => {
            let mut index = 0;
            while index < items.len() {
                let matches = items[index]
                    .as_object()
                    .map(|object| {
                        is_content_node_object(object)
                            && object.get("id").and_then(Value::as_str) == Some(node_id)
                    })
                    .unwrap_or(false);
                if matches {
                    return Some((items.remove(index), index));
                }
                if let Some(found) = remove_content_node(&mut items[index], node_id) {
                    return Some(found);
                }
                index += 1;
            }
            None
        }
        Value::Object(object) => {
            for child in object.values_mut() {
                if let Some(found) = remove_content_node(child, node_id) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn value_contains_content_type(value: &Value, wanted: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.get("type").and_then(Value::as_str) == Some(wanted)
                || object
                    .values()
                    .any(|child| value_contains_content_type(child, wanted))
        }
        Value::Array(items) => items
            .iter()
            .any(|item| value_contains_content_type(item, wanted)),
        _ => false,
    }
}

fn answer_slot_ids_in_values(values: &[Value], output: &mut Vec<String>) {
    for value in values {
        match value {
            Value::Object(object) => {
                if object.get("type").and_then(Value::as_str) == Some("answer_slot") {
                    if let Some(slot_id) = object.get("slotId").and_then(Value::as_str) {
                        if !output.iter().any(|existing| existing == slot_id) {
                            output.push(slot_id.to_string());
                        }
                    }
                }
                for child in object.values() {
                    if let Value::Array(items) = child {
                        answer_slot_ids_in_values(items, output);
                    }
                }
            }
            Value::Array(items) => answer_slot_ids_in_values(items, output),
            _ => {}
        }
    }
}

fn validate_normalized_rect(value: &Value, label: &str) -> CommandResult<()> {
    let values = value
        .as_array()
        .ok_or_else(|| format!("AUTHORING_PATCH_{label}_RECT_INVALID"))?;
    if values.len() != 4
        || values.iter().any(|value| {
            value
                .as_f64()
                .is_none_or(|number| !(0.0..=1.0).contains(&number))
        })
    {
        return Err(format!("AUTHORING_PATCH_{label}_RECT_INVALID"));
    }
    let x = values[0].as_f64().unwrap_or_default();
    let y = values[1].as_f64().unwrap_or_default();
    let width = values[2].as_f64().unwrap_or_default();
    let height = values[3].as_f64().unwrap_or_default();
    if width <= 0.0 || height <= 0.0 || x + width > 1.0 || y + height > 1.0 {
        return Err(format!("AUTHORING_PATCH_{label}_RECT_OUT_OF_BOUNDS"));
    }
    Ok(())
}

fn validate_normalized_anchor(value: &Value) -> CommandResult<()> {
    let values = value
        .as_array()
        .ok_or_else(|| "AUTHORING_PATCH_HOTSPOT_LABEL_ANCHOR_INVALID".to_string())?;
    if values.len() != 2
        || values.iter().any(|value| {
            value
                .as_f64()
                .is_none_or(|number| !(0.0..=1.0).contains(&number))
        })
    {
        return Err("AUTHORING_PATCH_HOTSPOT_LABEL_ANCHOR_INVALID".to_string());
    }
    Ok(())
}

fn required_string<'a>(object: &'a Map<String, Value>, key: &str) -> CommandResult<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("AUTHORING_PATCH_FIELD_REQUIRED:{key}"))
}

fn required_u64(object: &Map<String, Value>, key: &str) -> CommandResult<u64> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("AUTHORING_PATCH_FIELD_REQUIRED:{key}"))
}

fn find_object_mut_by_id<'a>(value: &'a mut Value, id: &str) -> Option<&'a mut Map<String, Value>> {
    match value {
        Value::Object(object) => {
            if object.get("id").and_then(Value::as_str) == Some(id) {
                return Some(object);
            }
            for child in object.values_mut() {
                if let Some(found) = find_object_mut_by_id(child, id) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(found) = find_object_mut_by_id(item, id) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_object_by_field_mut<'a>(
    value: &'a mut Value,
    field: &str,
    expected: &str,
) -> Option<&'a mut Map<String, Value>> {
    match value {
        Value::Object(object) => {
            if object.get(field).and_then(Value::as_str) == Some(expected) {
                return Some(object);
            }
            for child in object.values_mut() {
                if let Some(found) = find_object_by_field_mut(child, field, expected) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(found) = find_object_by_field_mut(item, field, expected) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_object_by_any_id_mut<'a>(
    value: &'a mut Value,
    expected: &str,
) -> Option<&'a mut Map<String, Value>> {
    match value {
        Value::Object(object) => {
            let matches_identifier = ["id", "taskId", "responseGroupId", "slotId"]
                .iter()
                .any(|field| object.get(*field).and_then(Value::as_str) == Some(expected));
            if matches_identifier {
                return Some(object);
            }
            for child in object.values_mut() {
                if let Some(found) = find_object_by_any_id_mut(child, expected) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(found) = find_object_by_any_id_mut(item, expected) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn preserve_provenance(patch: &Map<String, Value>) -> bool {
    patch
        .get("preserveProvenance")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn restore_provenance_status(patch: &Map<String, Value>) -> Option<&str> {
    patch.get("restoreProvenanceStatus").and_then(Value::as_str)
}

fn mark_user_edited(object: &mut Map<String, Value>, preserve: bool, restore: Option<&str>) {
    if let Some(status) = restore {
        object.insert(
            "provenanceStatus".to_string(),
            Value::String(status.to_string()),
        );
    } else if !preserve && object.contains_key("provenanceStatus") {
        object.insert(
            "provenanceStatus".to_string(),
            Value::String("user_edited".to_string()),
        );
    }
}

fn is_safe_node_attribute(key: &str) -> bool {
    matches!(
        key,
        "provenanceStatus"
            | "align"
            | "indentLevel"
            | "level"
            | "altText"
            | "placeholder"
            | "displayLabel"
            | "inline"
            | "label"
            | "slotIds"
            | "display"
            | "crop"
    )
}

fn is_supported_task_type(value: &str) -> bool {
    matches!(
        value,
        "single_choice"
            | "multiple_choice"
            | "true_false_not_given"
            | "yes_no_not_given"
            | "matching_information"
            | "matching_headings"
            | "matching_features"
            | "matching_sentence_endings"
            | "classification"
            | "sentence_completion"
            | "summary_completion"
            | "note_completion"
            | "table_completion"
            | "form_completion"
            | "flowchart_completion"
            | "diagram_label_completion"
            | "plan_map_label_completion"
            | "short_answer"
    )
}

fn expand_question_expression(expression: &Value) -> CommandResult<Vec<u64>> {
    let object = expression
        .as_object()
        .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_INVALID".to_string())?;
    match object.get("kind").and_then(Value::as_str) {
        Some("range") => {
            let start = object
                .get("start")
                .and_then(Value::as_u64)
                .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_START_REQUIRED".to_string())?;
            let end = object
                .get("end")
                .and_then(Value::as_u64)
                .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_END_REQUIRED".to_string())?;
            if start == 0 || end < start || end - start > 200 {
                return Err("AUTHORING_PATCH_EXPRESSION_RANGE_INVALID".to_string());
            }
            Ok((start..=end).collect())
        }
        Some("set") => parse_number_array(object.get("values")),
        Some("mixed") => {
            let values = object
                .get("values")
                .and_then(Value::as_array)
                .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_VALUES_REQUIRED".to_string())?;
            let mut numbers = Vec::new();
            for value in values {
                if let Some(number) = value.as_u64() {
                    if number == 0 {
                        return Err("AUTHORING_PATCH_EXPRESSION_NUMBER_INVALID".to_string());
                    }
                    numbers.push(number);
                } else {
                    numbers.extend(expand_question_expression(&json!({
                        "kind": "range",
                        "start": value.get("start").and_then(Value::as_u64),
                        "end": value.get("end").and_then(Value::as_u64),
                    }))?);
                }
            }
            if numbers.is_empty() || numbers.len() > 200 {
                return Err("AUTHORING_PATCH_EXPRESSION_VALUES_INVALID".to_string());
            }
            Ok(numbers)
        }
        _ => Err("AUTHORING_PATCH_EXPRESSION_KIND_INVALID".to_string()),
    }
}

fn parse_number_array(value: Option<&Value>) -> CommandResult<Vec<u64>> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_VALUES_REQUIRED".to_string())?;
    if values.is_empty() || values.len() > 200 {
        return Err("AUTHORING_PATCH_EXPRESSION_VALUES_INVALID".to_string());
    }
    let mut numbers = Vec::with_capacity(values.len());
    for value in values {
        let number = value
            .as_u64()
            .filter(|number| *number > 0)
            .ok_or_else(|| "AUTHORING_PATCH_EXPRESSION_NUMBER_INVALID".to_string())?;
        numbers.push(number);
    }
    Ok(numbers)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_patch, expand_question_expression, export_authoring_v2_core,
        materialize_authoring_assets, physical_shadow_matches_authoring,
        preserve_issue_resolutions, resolve_authoring_asset_preview_core,
        validate_authoring_v2_publish_readiness, AUTHORING_V2_SHADOW_FILE,
    };
    use crate::schema::common::{AssetDescriptorV2, AssetExtractionModeV2, AssetKindV2};
    use serde_json::{json, Value};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_asset_dirs() -> (PathBuf, PathBuf) {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ielts-authoring-v2-assets-{suffix}"));
        let staging = root.join("staging");
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::create_dir_all(&staging).unwrap();
        (root, staging)
    }

    fn text_document() -> serde_json::Value {
        json!({
            "type": "paragraph",
            "id": "paragraph-1",
            "provenanceStatus": "source",
            "children": [{
                "type": "text",
                "id": "text-1",
                "provenanceStatus": "source",
                "text": "Choose TWO letters."
            }]
        })
    }

    fn structured_document() -> serde_json::Value {
        json!({
            "passage": {
                "content": [{
                    "type": "paragraph",
                    "id": "paragraph-1",
                    "provenanceStatus": "source",
                    "children": [{
                        "type": "text",
                        "id": "text-1",
                        "provenanceStatus": "source",
                        "text": "First"
                    }]
                }, {
                    "type": "figure",
                    "id": "figure-1",
                    "provenanceStatus": "source",
                    "assetId": "asset-1",
                    "display": {},
                    "hotspots": []
                }]
            },
            "taskGroups": [{
                "taskId": "task-1",
                "instructions": [],
                "responseGroups": [{
                    "responseGroupId": "response-1",
                    "cardinality": {"min": 1, "max": 1, "exact": 1}
                }]
            }],
            "quality": {"issues": [{"issueId":"issue-1"}]}
        })
    }

    #[test]
    fn replace_text_uses_character_offsets_and_marks_provenance() {
        let mut document = text_document();
        apply_patch(
            &mut document,
            &json!({"op":"replaceText","nodeId":"text-1","from":7,"to":10,"text":"THREE"}),
        )
        .unwrap();
        assert_eq!(document["children"][0]["text"], "Choose THREE letters.");
        assert_eq!(document["children"][0]["provenanceStatus"], "user_edited");
    }

    #[test]
    fn expression_expansion_is_bounded_and_deterministic() {
        assert_eq!(
            expand_question_expression(
                &json!({"kind":"mixed","values":[14,{"start":15,"end":16}]})
            )
            .unwrap(),
            vec![14, 15, 16]
        );
        assert!(expand_question_expression(&json!({"kind":"range","start":9,"end":8})).is_err());
    }

    #[test]
    fn unsupported_node_attributes_are_rejected() {
        let mut document = text_document();
        assert!(apply_patch(
            &mut document,
            &json!({"op":"setNodeAttrs","nodeId":"text-1","attrs":{"id":"forbidden"}}),
        )
        .is_err());
    }

    #[test]
    fn structural_content_patches_support_insert_move_delete_crop_hotspot_and_cardinality() {
        let mut document = structured_document();
        apply_patch(
            &mut document,
            &json!({
                "op":"insertNode",
                "target":{"kind":"passage"},
                "index":1,
                "node":{"type":"paragraph","id":"paragraph-2","provenanceStatus":"manual","children":[]}
            }),
        )
        .unwrap();
        assert_eq!(document["passage"]["content"][1]["id"], "paragraph-2");
        apply_patch(
            &mut document,
            &json!({"op":"moveNode","nodeId":"paragraph-2","target":{"kind":"passage"},"index":0}),
        )
        .unwrap();
        assert_eq!(document["passage"]["content"][0]["id"], "paragraph-2");
        apply_patch(
            &mut document,
            &json!({"op":"moveNode","nodeId":"paragraph-2","target":{"kind":"passage"},"index":1}),
        )
        .unwrap();
        assert_eq!(document["passage"]["content"][1]["id"], "paragraph-2");
        apply_patch(
            &mut document,
            &json!({"op":"deleteNode","nodeId":"paragraph-2"}),
        )
        .unwrap();
        assert!(document["passage"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| node["id"] != "paragraph-2"));
        apply_patch(
            &mut document,
            &json!({"op":"cropAsset","nodeId":"figure-1","crop":[0.1,0.1,0.8,0.8]}),
        )
        .unwrap();
        apply_patch(
            &mut document,
            &json!({"op":"setHotspot","nodeId":"figure-1","hotspot":{"hotspotId":"hotspot-1","slotId":"slot-1","normalizedRect":[0.1,0.1,0.2,0.2]}}),
        )
        .unwrap();
        apply_patch(
            &mut document,
            &json!({"op":"setResponseCardinality","taskId":"task-1","responseGroupId":"response-1","cardinality":{"min":1,"max":2}}),
        )
        .unwrap();
        assert_eq!(
            document["passage"]["content"][1]["crop"],
            json!([0.1, 0.1, 0.8, 0.8])
        );
        assert_eq!(
            document["passage"]["content"][1]["hotspots"][0]["slotId"],
            "slot-1"
        );
        assert_eq!(
            document["taskGroups"][0]["responseGroups"][0]["cardinality"]["max"],
            2
        );
    }

    #[test]
    fn invalid_crop_and_hotspot_rects_are_rejected() {
        let mut document = structured_document();
        assert!(apply_patch(
            &mut document,
            &json!({"op":"cropAsset","nodeId":"figure-1","crop":[0.8,0.8,0.5,0.5]}),
        )
        .is_err());
        assert!(apply_patch(
            &mut document,
            &json!({"op":"setHotspot","nodeId":"figure-1","hotspot":{"hotspotId":"hotspot-1","slotId":"slot-1","normalizedRect":[0.1,0.1,0.0,0.2]}}),
        )
        .is_err());
    }

    #[test]
    fn replace_content_cannot_drop_an_existing_answer_slot() {
        let mut document = structured_document();
        document["passage"]["content"][0]["children"] = json!([{
            "type": "answer_slot",
            "id": "answer-slot-node-1",
            "slotId": "slot-1",
            "displayLabel": "14",
            "inline": true,
            "provenanceStatus": "source"
        }]);
        let error = apply_patch(
            &mut document,
            &json!({
                "op":"replaceContent",
                "target":{"kind":"node","nodeId":"paragraph-1"},
                "content":[]
            }),
        )
        .expect_err("replaceContent must not delete answer slots");
        assert!(error.contains("AUTHORING_PATCH_ANSWER_SLOT_LOSS:slot-1"));
    }

    #[test]
    fn inserted_answer_slot_can_be_removed_only_by_an_explicit_inverse_patch() {
        let mut document = structured_document();
        apply_patch(
            &mut document,
            &json!({
                "op":"insertNode",
                "target":{"kind":"passage"},
                "index":1,
                "node":{
                    "type":"paragraph",
                    "id":"answer-slot-parent",
                    "provenanceStatus":"manual",
                    "children":[{
                        "type":"answer_slot",
                        "id":"inserted-answer-slot-node",
                        "slotId":"slot-1",
                        "displayLabel":"14",
                        "inline":true,
                        "provenanceStatus":"manual"
                    }]
                }
            }),
        )
        .unwrap();
        assert!(apply_patch(
            &mut document,
            &json!({"op":"deleteNode","nodeId":"answer-slot-parent"}),
        )
        .is_err());
        apply_patch(
            &mut document,
            &json!({"op":"deleteNode","nodeId":"answer-slot-parent","allowAnswerSlotRemoval":true}),
        )
        .unwrap();
        assert!(document["passage"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|node| node["id"] != "answer-slot-parent"));
    }

    #[test]
    fn semantic_option_bank_and_answer_slot_patches_keep_registries_in_sync() {
        let mut document = json!({
            "passage":{"content":[]},
            "taskGroups":[{
                "taskId":"task-1",
                "displayRange":{"kind":"set","values":[1]},
                "instructionSignature":{"expectedQuestionNumbers":[1],"expectedSlotCount":1},
                "instructions":[],
                "optionBank":{"optionBankId":"bank-1","scope":"task_group","options":[],"allowReuse":false,"sourceAnchors":[]},
                "responseGroups":[{
                    "responseGroupId":"response-1",
                    "prompt":[{"type":"paragraph","id":"prompt-1","sourceAnchors":[],"provenanceStatus":"source","children":[{"type":"answer_slot","id":"slot-node-1","slotId":"slot-1","displayLabel":"1","inline":true,"sourceAnchors":[],"provenanceStatus":"source"}]}],
                    "slotIds":["slot-1"]
                }]
            }],
            "answerSlots":{"slot-1":{"slotId":"slot-1","questionNumber":1}},
            "answerKey":{"slot-1":{"kind":"unresolved"}}
        });
        apply_patch(&mut document, &json!({
            "op":"setOptionBank","taskId":"task-1",
            "optionBank":{"optionBankId":"bank-1","scope":"task_group","options":[{"optionId":"option-a","label":"A","content":[],"sourceAnchors":[]}],"allowReuse":false,"sourceAnchors":[]}
        })).unwrap();
        assert_eq!(
            document["taskGroups"][0]["optionBank"]["options"][0]["label"],
            "A"
        );

        apply_patch(&mut document, &json!({
            "op":"insertAnswerSlot","taskId":"task-1","responseGroupId":"response-1",
            "target":{"kind":"responsePrompt","responseGroupId":"response-1"},"parentId":"prompt-1","index":1,"slotIndex":1,
            "node":{"type":"answer_slot","id":"slot-node-2","slotId":"slot-2","displayLabel":"2","inline":true,"sourceAnchors":[],"provenanceStatus":"manual"},
            "slot":{"slotId":"slot-2","questionNumber":2},"value":{"kind":"unresolved"},
            "expression":{"kind":"range","start":1,"end":2}
        })).unwrap();
        assert_eq!(
            document["taskGroups"][0]["responseGroups"][0]["slotIds"],
            json!(["slot-1", "slot-2"])
        );
        assert!(document["answerSlots"]["slot-2"].is_object());
        assert_eq!(
            document["taskGroups"][0]["instructionSignature"]["expectedSlotCount"],
            2
        );

        apply_patch(&mut document, &json!({
            "op":"deleteAnswerSlot","taskId":"task-1","responseGroupId":"response-1","nodeId":"slot-node-2","slotId":"slot-2",
            "expression":{"kind":"set","values":[1]}
        })).unwrap();
        assert_eq!(
            document["taskGroups"][0]["responseGroups"][0]["slotIds"],
            json!(["slot-1"])
        );
        assert!(document["answerSlots"].get("slot-2").is_none());
        assert!(document["answerKey"].get("slot-2").is_none());
    }

    #[test]
    fn undo_patch_can_restore_provenance_without_marking_user_edited() {
        let mut document = text_document();
        apply_patch(
            &mut document,
            &json!({
                "op":"replaceText",
                "nodeId":"text-1",
                "from":0,
                "to":6,
                "text":"Edited"
            }),
        )
        .unwrap();
        apply_patch(
            &mut document,
            &json!({
                "op":"replaceText",
                "nodeId":"text-1",
                "from":0,
                "to":6,
                "text":"Choose",
                "preserveProvenance":true,
                "restoreProvenanceStatus":"source"
            }),
        )
        .unwrap();
        assert_eq!(document["children"][0]["provenanceStatus"], "source");
    }

    #[test]
    fn export_materializes_and_verifies_non_empty_assets() {
        let (source_root, staging) = temp_asset_dirs();
        let bytes = b"diagram-bytes";
        let relative_path = "assets/diagram.png";
        fs::write(source_root.join(relative_path), bytes).unwrap();
        let descriptor = AssetDescriptorV2 {
            asset_id: "diagram-1".to_string(),
            kind: AssetKindV2::RasterImage,
            mime: "image/png".to_string(),
            relative_path: relative_path.to_string(),
            sha256: crate::hash_bytes(bytes),
            byte_length: bytes.len() as u64,
            width_px: Some(10),
            height_px: Some(10),
            duration_ms: None,
            extraction_mode: AssetExtractionModeV2::Embedded,
            alt_text: None,
            decorative: Some(false),
            source_anchor: None,
            diagram_question_region: None,
        };
        materialize_authoring_assets(&source_root, &staging, &[descriptor.clone()]).unwrap();
        assert_eq!(fs::read(staging.join(relative_path)).unwrap(), bytes);

        let mut bad = descriptor;
        bad.sha256 = "0".repeat(64);
        let error = materialize_authoring_assets(&source_root, &staging.join("bad"), &[bad])
            .expect_err("asset hash mismatch must fail closed");
        assert!(error.contains("authoring_v2_asset_hash_mismatch"));
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn authoring_asset_preview_resolves_only_manifest_backed_image_bytes() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ielts-authoring-v2-preview-{suffix}"));
        let job_id = "asset-preview";
        let job_dir = root.join("jobs").join(job_id);
        fs::create_dir_all(job_dir.join("assets")).unwrap();
        let bytes = b"not-a-real-png-but-manifest-closed";
        fs::write(job_dir.join("assets/diagram.png"), bytes).unwrap();
        let mut authoring: serde_json::Value = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        authoring["jobId"] = json!(job_id);
        authoring["assets"] = json!([{
            "assetId": "diagram-1",
            "kind": "raster_image",
            "mime": "image/png",
            "relativePath": "assets/diagram.png",
            "sha256": crate::hash_bytes(bytes),
            "byteLength": bytes.len(),
            "widthPx": 20,
            "heightPx": 10,
            "extractionMode": "embedded"
        }]);
        fs::write(
            job_dir.join(AUTHORING_V2_SHADOW_FILE),
            serde_json::to_vec(&authoring).unwrap(),
        )
        .unwrap();

        let preview = resolve_authoring_asset_preview_core(&root, job_id, "diagram-1").unwrap();
        assert_eq!(preview["mime"], "image/png");
        assert_eq!(preview["widthPx"], 20);
        assert!(preview["resourceUri"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
        assert!(resolve_authoring_asset_preview_core(&root, job_id, "missing").is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_rejects_non_ready_quality_state_before_materialization() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ielts-authoring-v2-quality-gate-{suffix}"));
        let job_id = "quality-gate";
        let job_dir = root.join("jobs").join(job_id);
        fs::create_dir_all(&job_dir).unwrap();
        let authoring: serde_json::Value = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        let mut authoring = authoring;
        authoring["quality"]["state"] = json!("ready");
        fs::write(
            job_dir.join(AUTHORING_V2_SHADOW_FILE),
            serde_json::to_vec(&authoring).unwrap(),
        )
        .unwrap();

        let export_dir = root.join("exports");
        let error = export_authoring_v2_core(
            &root,
            json!({
                "jobId": job_id,
                "exportDir": export_dir.to_string_lossy()
            }),
        )
        .expect_err("review_required authoring must not pass strict export");
        assert_eq!(
            error,
            "authoring_v2_export_blocked:quality_state=review_required"
        );
        assert!(!export_dir.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_readiness_rejects_human_unverified_stale_review_and_ai_fallback() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ielts-authoring-v2-readiness-{suffix}"));
        let job_id = "readiness-gate";
        let job_dir = root.join("jobs").join(job_id);
        fs::create_dir_all(&job_dir).unwrap();

        let mut authoring: Value = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        authoring["jobId"] = json!(job_id);
        authoring["quality"]["state"] = json!("ready");
        authoring["audit"]["humanVerified"] = json!(false);
        let error =
            validate_authoring_v2_publish_readiness(&root, job_id, 0, &authoring).unwrap_err();
        assert_eq!(
            error,
            "authoring_v2_export_blocked:human_verification_required"
        );

        authoring["audit"]["humanVerified"] = json!(true);
        fs::write(
            job_dir.join("source-review.json"),
            serde_json::to_vec(&json!({
                "schemaVersion": "SourceReviewV1",
                "jobId": job_id,
                "required": true,
                "resolved": true,
                "stale": true,
                "fingerprint": "old",
                "parserWarnings": [],
                "lowConfidenceBlocks": [],
                "resolvedAt": null,
                "note": null
            }))
            .unwrap(),
        )
        .unwrap();
        let error =
            validate_authoring_v2_publish_readiness(&root, job_id, 0, &authoring).unwrap_err();
        assert_eq!(error, "authoring_v2_export_blocked:source_review_stale");

        let _ = fs::remove_file(job_dir.join("source-review.json"));
        authoring["audit"]["notes"] = json!(["AI fallback was used"]);
        let error =
            validate_authoring_v2_publish_readiness(&root, job_id, 0, &authoring).unwrap_err();
        assert!(
            error.starts_with("authoring_v2_export_blocked:ai_fallback="),
            "{error}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn quality_refresh_rejects_stale_physical_shadow_and_preserves_resolution() {
        let authoring = json!({
            "jobId": "job-1",
            "sourceDocumentId": "document-1",
            "exam": {"sourceFiles": [{"sourceFileId": "source-1"}]},
            "quality": {
                "issues": [{
                    "issueId": "issue-1",
                    "details": {"resolution": "ignored", "note": "reviewed"}
                }]
            }
        });
        let physical = json!({
            "schemaVersion": "DocumentIRV2",
            "jobId": "job-1",
            "documentId": "document-1",
            "sourceFiles": [{"sourceFileId": "source-1"}]
        });
        assert!(physical_shadow_matches_authoring(&physical, &authoring));

        let mut stale = physical.clone();
        stale["jobId"] = json!("other-job");
        assert!(!physical_shadow_matches_authoring(&stale, &authoring));

        let mut quality = json!({
            "issues": [{"issueId": "issue-1", "details": {"source": "recomputed"}}]
        });
        preserve_issue_resolutions(&mut quality, authoring.get("quality"));
        assert_eq!(quality["issues"][0]["details"]["resolution"], "ignored");
        assert_eq!(quality["issues"][0]["details"]["note"], "reviewed");
    }
}
