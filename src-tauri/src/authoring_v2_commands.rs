//! Phase 5 structured-authoring persistence.
//!
//! This module is deliberately separate from the V1 authoring commands.  A
//! V2 edit starts from the Phase 4 shadow artifact (or from an existing V2
//! revision), applies a small, explicit patch vocabulary, validates the full
//! typed authoring document, and appends an immutable revision.  The legacy
//! `authoring-ir.json` file is never rewritten here.

use crate::artifact_store::{
    append_revision, ensure_job_artifact_layout, list_revision_records, read_revision,
    recover_current_revision, write_artifact_json, write_canonical_json_atomic,
    write_js_canonical_json_atomic, RevisionSourceV2,
};
use crate::ielts_grammar::evaluate_quality;
use crate::ielts_grammar::quality::derive_instruction_signature_for_group;
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

/// 发布前检查：把发布门禁的判断以结构化 blocker 形式交给编辑器。
///
/// 编辑器此前只有一套纯前端近似检查（`deriveActionableIssues`），于是会出现
/// 「界面显示 0 个问题、点发布却失败」且文案不可操作。这里复用发布路径上同一个
/// `check_publish_preflight`，让「编辑器问题列表」与「发布门禁」是同一份判断。
///
/// **不是只读**：为了与发布同等宽松，它会先跑一次权威稿播种（见下），
/// 因此可能创建题库行或补种缺失的 canonical 文档——与工作区加载时走的是同一步，
/// 且该步骤是幂等的、不覆盖用户已编辑的稿子。
pub(crate) fn get_publish_preflight_core(root: &Path, job_id: &str) -> CommandResult<Value> {
    safe_job_dir(root, job_id)?;
    // 预检必须与发布**同等宽松**：`publish_items_core` 在读权威稿之前会先跑
    // `migrate_single_item` 播种它（工作区加载时也走同一步）。若预检跳过这一步，
    // 就会把一个发布本会接受的条目报成阻断——那是反方向的假信号。
    crate::library::migration::migrate_single_item(root, job_id)?;
    let canonical = crate::library::repository::open_library_connection(root)
        .and_then(|conn| crate::library::repository::get_canonical_ds(&conn, job_id))?;
    let Some((authoring_value, version)) = canonical else {
        // 播种后仍然没有权威稿：发布必然以同一个码失败，预检如实报出来。
        return Ok(json!({
            "schemaVersion": "PublishCheckResultV1",
            "jobId": job_id,
            "editVersion": 0,
            "passed": false,
            "blockers": [{
                "code": "ITEM_DS_NOT_SEEDED",
                "targetId": Value::Null,
                "userMessage": "这道题还没有可发布的权威稿，请先完成识别并保存，再发布。",
                "action": "open_workspace",
                "internal": "canonical_ds_missing"
            }],
            "warnings": []
        }));
    };
    Ok(check_publish_preflight(
        root,
        job_id,
        version.max(0) as u64,
        &authoring_value,
    ))
}

pub(crate) fn resolve_authoring_asset_preview_core(
    root: &Path,
    job_id: &str,
    asset_id: &str,
) -> CommandResult<Value> {
    let job_root = fs::canonicalize(safe_job_dir(root, job_id)?)
        .map_err(|error| format!("authoring_asset_root_unavailable:{error}"))?;
    let canonical = crate::library::repository::open_library_connection(root)
        .and_then(|conn| crate::library::repository::get_canonical_ds(&conn, job_id))?;
    let authoring_value = match canonical {
        Some((ds, _)) => ds,
        None => load_current_authoring(root, job_id)?.0,
    };
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

/// 本批命令里**人明确处理过**的问题目标（`resolveIssue`）。
///
/// 用于从「受影响目标」里扣除：人在同一次保存里既改了内容、又亲手确认了某个问题，
/// 那条确认是他对**当前**内容的判断，不该被他自己的编辑顺手清掉。只有他没确认过的
/// 目标才按「旧判断已过期」重置。
///
/// 在**施加过本批命令之后**的稿件上调用：`resolveIssue` 已把 resolution 写进
/// `quality.issues[]`，因此按 `issueId` 就能取回它对应的 `targetId`。
pub(crate) fn explicitly_handled_issue_targets(
    authoring: &Value,
    commands: &[Value],
) -> BTreeSet<String> {
    let issues = authoring
        .pointer("/quality/issues")
        .and_then(Value::as_array);
    let mut handled = BTreeSet::new();
    for command in commands {
        if command.get("op").and_then(Value::as_str) != Some("resolveIssue") {
            continue;
        }
        let Some(issue_id) = command.get("issueId").and_then(Value::as_str) else {
            continue;
        };
        if let Some(target_id) = issues
            .into_iter()
            .flatten()
            .find(|issue| issue.get("issueId").and_then(Value::as_str) == Some(issue_id))
            .and_then(|issue| issue.get("targetId"))
            .and_then(Value::as_str)
        {
            handled.insert(target_id.to_string());
        }
    }
    handled
}

/// 一条 blocking issue 是否**仍未处理** —— 发布判据里唯一的那份谓词。
///
/// 实现已下沉到 [`crate::authoring_validation`]（`blocking_issue_unresolved` /
/// `unresolved_blocking_issues`），此处**同名重导出**。保留这个路径而不是去改所有
/// 调用方，是因为 `cloud_repair::remaining_tasks` 等消费者引用的是
/// `crate::authoring_v2_commands::…`：重导出让"实现只有一份"与"引用路径不裂开"
/// 同时成立。曾经这三处各自内联同一段谓词 —— 只要有一处被改动，就会出现
/// 「预检说可以发布、修复循环却认为还剩问题」这类同稿不同判。
///
/// 只重导出 `unresolved_blocking_issues`：谓词本身（`blocking_issue_unresolved`）
/// 的消费者都直接走 `crate::authoring_validation::…`，在这里再挂一个没人走的
/// 别名只会长出一条无用的公开面。
pub(crate) use crate::authoring_validation::unresolved_blocking_issues;

/// M1 typed preflight（计划 §13.3/§13.4）：只检查**当前** canonical DS 与当前 blocker。
/// 与 [`validate_authoring_v2_publish_readiness`] 的差别（均为有意移除）：
/// - 不扫描历史 authoring/pipeline JSON 的 fallback/partial 字符串；
/// - 不要求全局 `audit.humanVerified`；
/// - 不依赖 SourceReviewV1 文件状态。
/// 资源闭包由 publisher 的 staging/probe 与资产 hash 绑定继续保证（§13.5 原子发布保留）。
pub(crate) fn check_publish_preflight(
    root: &Path,
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
    let unresolved_blockers = unresolved_blocking_issues(authoring_value);
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

    // `passed` 不再由这份展示用清单自己算：结论只来自 `publish_verdict`。
    // 清单仍然负责把"为什么"写给用户看，但它**不是**判据。
    let verdict = crate::authoring_validation::publish_verdict(
        root,
        job_id,
        authoring_value,
        Some(edit_version.min(i64::MAX as u64) as i64),
        crate::authoring_validation::PublishScope::CanonicalDirect,
    );
    if !verdict.is_ready() {
        for reason in verdict.reasons() {
            if blockers
                .iter()
                .any(|item| item.get("code").and_then(Value::as_str) == Some(reason.code))
            {
                continue;
            }
            blockers.push(json!({
                "code": reason.code,
                "targetId": reason.target,
                "userMessage": reason.message,
                "action": "open_workspace",
                "layer": reason.layer
            }));
        }
        if blockers.is_empty() {
            blockers.push(json!({
                "code": "PUBLISH_VERDICT_BLOCKED",
                "targetId": Value::Null,
                "userMessage": "这道题暂时不能发布。",
                "action": "open_workspace"
            }));
        }
    }

    json!({
        "schemaVersion": "PublishCheckResultV1",
        "jobId": job_id,
        "editVersion": edit_version,
        "passed": verdict.is_ready(),
        "blockers": blockers,
        "warnings": warnings,
        "publishVerdict": verdict.to_value()
    })
}

pub(crate) fn collect_publish_gate_markers(
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
    let mut input: ExportAuthoringV2Input = serde_json::from_value(input)
        .map_err(|error| format!("authoring_v2_invalid_export_request:{error}"))?;
    if input.authoring.is_some() || input.edit_version.is_some() {
        let conn = crate::library::repository::open_library_connection(root)?;
        let (ds, version) = crate::library::repository::get_canonical_ds(&conn, &input.job_id)?
            .ok_or("ITEM_DS_NOT_SEEDED")?;
        if input.edit_version.is_some_and(|expected| expected != version as u64) {
            return Err(format!("EDIT_VERSION_CONFLICT:current={version}"));
        }
        input.authoring = Some(ds);
        input.edit_version = Some(version as u64);
    }
    export_authoring_snapshot(root, input)
}

/// 发布模式。
///
/// - `Strict`：门禁结论不是 Ready 就拒绝（所有旧导出入口与 NAS 单题发布都走这条）。
/// - `Forced`：用户点击「发布」即放行。门禁**照常计算、结论原样保留**；结论不是
///   Ready 时本次导出被记录为显式放行（`publishOverride`），学生端加载不了的稿只写
///   授权快照（`studentLoadable: false`）。它**不**绕过任何硬性安全/IO 检查：
///   examId 路径安全、资产复制与路径、清单 CAS、锁、重复 examId。
#[derive(Debug, Clone)]
pub(crate) enum PublishMode {
    Strict,
    Forced {
        confirmed_at: String,
        acknowledged_reasons: Vec<String>,
    },
}

pub(crate) fn export_authoring_snapshot(root: &Path, input: ExportAuthoringV2Input) -> CommandResult<Value> {
    export_authoring_snapshot_with_mode(root, input, &PublishMode::Strict)
}

fn has_unresolved_answers(authoring: &Value) -> bool {
    authoring
        .get("answerKey")
        .and_then(Value::as_object)
        .is_some_and(|answers| {
            answers
                .values()
                .any(|value| value.get("kind").and_then(Value::as_str) == Some("unresolved"))
        })
}

pub(crate) fn export_authoring_snapshot_with_mode(
    root: &Path,
    input: ExportAuthoringV2Input,
    mode: &PublishMode,
) -> CommandResult<Value> {
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
    // 发布结论只来自 `publish_verdict`（唯一判据入口）。
    //
    // 修前这里自己算了一遍 `quality_state`，又分别走 preflight（db_direct）与
    // readiness（legacy），三处判据各说各话：预检说可以发布、导出却据另一处拒绝，
    // 或反过来。现在判据只有一份，`scope` 由**来源**决定（canonical 直通 / 文件派生），
    // 不是调用方可调的开关。
    let scope = if db_direct {
        crate::authoring_validation::PublishScope::CanonicalDirect
    } else {
        crate::authoring_validation::PublishScope::FullDerived
    };
    let verdict = crate::authoring_validation::publish_verdict(
        root,
        &input.job_id,
        &authoring_value,
        input
            .edit_version
            .map(|value| value.min(i64::MAX as u64) as i64),
        scope,
    );
    // 放行只在门禁确实不是 Ready 时才**被使用**；结论本身从不被改写。
    let publish_override: Option<Value> = if verdict.is_ready() {
        None
    } else {
        match mode {
            PublishMode::Strict => {
                return Err(format!(
                    "authoring_v2_export_blocked:{}:{}",
                    verdict.status(),
                    serde_json::to_string(&verdict.to_value()).unwrap_or_default()
                ));
            }
            PublishMode::Forced {
                confirmed_at,
                acknowledged_reasons,
            } => {
                // 硬性不变量：门禁里的 EXAM_ID_INVALID 可以被放行，路径安全不能。
                crate::util::validate_path_segment(
                    "exam_id",
                    authoring_value
                        .pointer("/exam/examId")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )?;
                let mut reason_codes: Vec<&str> = Vec::new();
                for reason in verdict.reasons() {
                    if !reason_codes.contains(&reason.code) {
                        reason_codes.push(reason.code);
                    }
                }
                Some(json!({
                    "forced": true,
                    "verdict": verdict.to_value(),
                    "reasons": reason_codes,
                    "acknowledgedReasons": acknowledged_reasons,
                    "confirmedAt": confirmed_at
                }))
            }
        }
    };
    // proof / preflight 仍然产出（给 UI 与审计看"为什么"），但它们**不再参与判据**。
    // `validate_authoring_v2_publish_readiness` 里唯一不在 verdict 内的检查是
    // 「请求的 revision 是否仍是当前 revision」—— 那是请求属性，不是稿件属性。
    // 放行模式下 readiness 会报错（那正是被放行的东西），改用不报错的预检作为证明。
    let publish_proof: Value = if db_direct || publish_override.is_some() {
        check_publish_preflight(root, &input.job_id, revision, &authoring_value)
    } else {
        validate_authoring_v2_publish_readiness(root, &input.job_id, revision, &authoring_value)?
    };
    // `reviewRequired` 也由**同一份**就绪度规则决定（`quality_readiness`，
    // 与 `QualityReportV2.state` 和 `publish_verdict` 共用实现）。走到这里
    // verdict 必然是 `Ready`，而 `Ready` 蕴含 `QualityReadiness::Ready`，所以
    // 正常情况恒为 false；仍然如实算而不是写死常量，是为了让这个字段的含义
    // 只由那条规则定义。
    let review_required = !matches!(
        crate::ielts_grammar::quality::quality_readiness(&authoring_value),
        Ok(crate::ielts_grammar::quality::QualityReadiness::Ready)
    );
    // 学生端加载时拒绝未解析答案与编译不过的运行时。放行模式下这种稿只写授权
    // 快照（不产出运行时、不进学生清单），绝不为了能加载而编造任何内容。
    let runtime = match compile_reading_source_v2(&authoring) {
        Ok(runtime) if publish_override.is_none() => Some(runtime),
        Ok(runtime) => (!has_unresolved_answers(&authoring_value)
            && crate::reading_source_v2::validate_reading_source_v2(&runtime).is_empty())
        .then_some(runtime),
        Err(_) if publish_override.is_some() => None,
        Err(issues) => {
            return Err(format!(
                "authoring_v2_export_compile_blocked:{}",
                serde_json::to_string(&issues).unwrap_or_default()
            ));
        }
    };
    let student_loadable = runtime.is_some();

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
        // 学生端用 JavaScript 的 JSON.stringify 重算 ReadingExamSourceV2 的
        // runtimeSha256，所以运行时源必须按 ECMAScript 数字规则编码，否则整型
        // 浮点（confidence 1.0、widthPercent 60.0）会让发布包被学生端判定为
        // reading_source_integrity_failed。
        let runtime_sha256 = match runtime.as_ref() {
            Some(runtime) => {
                let runtime_value =
                    serde_json::to_value(runtime).map_err(|error| error.to_string())?;
                Some(write_js_canonical_json_atomic(&runtime_path, &runtime_value)?.sha256)
            }
            None => None,
        };
        materialize_authoring_assets(&artifact_layout.job_dir, &staging_dir, &authoring.assets)?;
        let files: Vec<&str> = if student_loadable {
            vec!["authoring-ir-v2.json", "reading-source-v2.json", "manifest-v2.json"]
        } else {
            vec!["authoring-ir-v2.json", "manifest-v2.json"]
        };
        let mut manifest_value = json!({
            "schemaVersion": "AuthoringV2ExportReceiptV1",
            "jobId": input.job_id,
            "examId": authoring.exam.exam_id,
            "revision": revision,
            "editVersion": if db_direct { input.edit_version.unwrap_or(revision) } else { revision },
            "authoringSource": if db_direct { "canonical_ds" } else { "artifact_session" },
            "sourceDocumentId": authoring.source_document_id,
            "files": files,
            "authoringSha256": authoring_receipt.sha256,
            "runtimeSha256": runtime_sha256,
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
            "reviewRequired": review_required,
            "studentLoadable": student_loadable,
            "publishProof": publish_proof
        });
        if let Some(publish_override) = publish_override.as_ref() {
            manifest_value["publishOverride"] = publish_override.clone();
        }
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

    let mut receipt = json!({
        "schemaVersion": "AuthoringV2ExportReceiptV1",
        "jobId": input.job_id.clone(),
        "examId": authoring.exam.exam_id,
        "revision": revision,
        "outputDir": output_dir,
        "authoringPath": output_dir.join("authoring-ir-v2.json"),
        "runtimePath": if student_loadable {
            json!(output_dir.join("reading-source-v2.json"))
        } else {
            Value::Null
        },
        "manifestPath": output_dir.join("manifest-v2.json"),
        "v1FilesRemainReadable": true,
        "pdfPerQuestionLlmRepair": false,
        "reviewRequired": review_required,
        "studentLoadable": student_loadable,
        "publishVerdict": verdict.to_value(),
        "publishProof": publish_proof
    });
    if let Some(publish_override) = publish_override.as_ref() {
        receipt["publishOverride"] = publish_override.clone();
    }
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

/// 读取当前权威稿。
///
/// M1 起唯一权威稿是数据库里的 canonical DS。revision 文件树与 shadow 都是旧的工件形态：
/// revision 只在没有 DB 权威稿时作为来源，shadow 只是导入期写出的缓存。
///
/// 早期实现无条件优先文件，导致「DB 里已保存的编辑」被编辑前的缓存盖住——
/// 与 audit-2026-09-07 F04 同一类根因（派生物凌驾于权威稿之上）。
fn load_current_authoring(root: &Path, job_id: &str) -> CommandResult<(Value, u64)> {
    let current = recover_current_revision(root, job_id)?;
    if current.revision > 0 {
        let value = read_revision(root, job_id, current.revision)?;
        validate_authoring(&value)?;
        return Ok((value, current.revision));
    }

    // DB 权威稿存在时优先；shadow 只服务尚未迁移的 job。
    if let Some((ds, _version)) = crate::library::repository::open_library_connection(root)
        .and_then(|conn| crate::library::repository::get_canonical_ds(&conn, job_id))?
    {
        validate_authoring(&ds)?;
        return Ok((ds, 0));
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
    refresh_quality_report_for_targets(root, job_id, authoring, &BTreeSet::new())
}

/// 同上，但把 `affected_targets` 上**已经过期**的人工 resolution 重置掉。
///
/// 为什么需要这一层：`preserve_issue_resolutions` 是按 `issueId` 把旧 resolution 带过来
/// 的，而 `issueId` 只是「事实」的指纹。云端修复改了某个目标的**内容**之后，只要那条
/// 事实的指纹恰好没变（文案固定、`details` 没覆盖到被改的部分），旧 resolution 就会被
/// 原样继承——用户从没看过新内容，系统却已经替他把问题标成「已解决」。这等于把「有人
/// 处理过旧内容」冒充成「新内容也没问题」，也是让「修复完成」变得不可信的一条捷径。
///
/// 因此：**本次受影响的目标**上的旧 resolution 一律不继承（重置为未处理，重新评价）；
/// 没被本次改动碰过的目标照旧继承——有效的人工处理不该因为别处改了一笔就全部作废。
pub(crate) fn refresh_quality_report_for_targets(
    root: &Path,
    job_id: &str,
    authoring: &mut Value,
    affected_targets: &BTreeSet<String>,
) -> CommandResult<()> {
    let previous_quality = authoring.get("quality").cloned();
    let physical_shadow = read_json_opt(&job_dir(root, job_id).join(DOCUMENT_V2_SHADOW_FILE))?
        .filter(|shadow| physical_shadow_matches_authoring(shadow, authoring));
    // 「题库保存」：发布后原文件与 shadow 被删除的条目改用发布时冻结的证据。
    // 只有**确实被清理过**的条目才有冻结证据；非清理条目缺 shadow 与今天完全一样。
    let mut quality = match physical_shadow.as_ref() {
        Some(shadow) => evaluate_quality(authoring, Some(shadow)),
        None => match crate::library::final_version::load_purged_evidence(root, job_id) {
            Some(frozen) => {
                crate::ielts_grammar::quality::evaluate_quality_with_frozen_evidence(authoring, &frozen)
            }
            None => evaluate_quality(authoring, None),
        },
    };
    preserve_issue_resolutions(&mut quality, previous_quality.as_ref(), affected_targets);
    authoring
        .as_object_mut()
        .ok_or_else(|| "AUTHORING_SCHEMA_INVALID:authoring must be an object".to_string())?
        .insert("quality".to_string(), quality);
    Ok(())
}

pub(crate) fn physical_shadow_matches_authoring(shadow: &Value, authoring: &Value) -> bool {
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

fn preserve_issue_resolutions(
    quality: &mut Value,
    previous_quality: Option<&Value>,
    affected_targets: &BTreeSet<String>,
) {
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
        // 本次受影响的目标：内容变了，人当时对旧内容作出的判断不再成立，**不继承**。
        // 重置成「未处理」后重新评价——宁可多让用户看一眼，也不替他把新内容判定为已解决。
        if issue
            .get("targetId")
            .and_then(Value::as_str)
            .is_some_and(|target_id| affected_targets.contains(target_id))
        {
            continue;
        }
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
        "upsertTaskGroupBundle" => upsert_task_group_bundle(document, object),
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
    // If the edited node is part of a task group's instruction text, the
    // derived `instructionSignature` is now stale. Re-derive it from the new
    // instruction text so quality is judged against the edited instruction.
    if let Some(task_groups) = document.get_mut("taskGroups").and_then(Value::as_array_mut) {
        for group in task_groups {
            if instruction_node_edited(group, &node_id) {
                derive_instruction_signature_for_group(group);
            }
        }
    }
    Ok(())
}

/// True when `node_id` is the id of an instruction node of `group`, or of a
/// descendant text node inside one of its `instructions`.
fn instruction_node_edited(group: &Value, node_id: &str) -> bool {
    group
        .get("instructions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|node| node_equals_or_contains(node, node_id))
}

fn node_equals_or_contains(node: &Value, node_id: &str) -> bool {
    if node.get("id").and_then(Value::as_str) == Some(node_id) {
        return true;
    }
    node.get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|child| node_equals_or_contains(child, node_id))
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

/// 原子地创建（或替换）一个完整任务组及其答案槽与标准答案。
///
/// 这是唯一一个把「任务组 + 答案槽 + 答案键」作为一整个结构一次性落地的新增操作：
/// `setResponseGroup` 只能替换已存在的组、`replaceContent` 又禁止丢失 `answer_slot` 节点，
/// 否则模型只能发几十次零散调用并在其间穿过许多半成品（非法）中间态。本操作要求要么整束落地、
/// 要么完全不变——任何校验失败都应在改动文档之前以 `Err` 返回（先对克隆做全部校验，再回写）。
///
/// 身份由后端拥有：模型可省略 `taskId`/`slotId`/`responseGroupId`，后台按确定性规则推导
/// （基于题号、组内顺序计数），绝不引入随机或时间戳，使整次编辑可被安全重试。
fn upsert_task_group_bundle(document: &mut Value, patch: &Map<String, Value>) -> CommandResult<()> {
    // 模型不被允许注入任何来源/质量/审计类键——这些由保存事务统一处理。
    for forbidden in [
        "provenanceStatus",
        "preserveProvenance",
        "restoreProvenanceStatus",
        "quality",
        "audit",
    ] {
        if patch.contains_key(forbidden) {
            return Err(format!("AUTHORING_PATCH_BUNDLE_FORBIDDEN_KEY:{forbidden}"));
        }
    }

    let task_group = patch
        .get("taskGroup")
        .and_then(Value::as_object)
        .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_TASK_GROUP_REQUIRED".to_string())?;
    let answer_slots_input = patch
        .get("answerSlots")
        .and_then(Value::as_array)
        .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_ANSWER_SLOTS_REQUIRED".to_string())?
        .clone();
    let answer_key_input = patch
        .get("answerKey")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let insert_after = patch
        .get("insertAfterTaskId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let replaces_task_id = patch
        .get("replacesTaskId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());

    let provided_task_id = task_group
        .get("taskId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    // `replacesTaskId` 是 `taskGroup.taskId` 的显式替代；两者都给且不一致即冲突。
    if let (Some(provided), Some(replaces)) = (provided_task_id, replaces_task_id) {
        if provided != replaces {
            return Err("AUTHORING_PATCH_BUNDLE_TARGET_MISMATCH".to_string());
        }
    }

    let task_type = task_group
        .get("taskType")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_TASK_TYPE_REQUIRED".to_string())?;
    if !is_supported_task_type(task_type) {
        return Err(format!("AUTHORING_PATCH_BUNDLE_TASK_TYPE_INVALID:{task_type}"));
    }
    let instructions = task_group
        .get("instructions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let raw_response_groups = task_group
        .get("responseGroups")
        .and_then(Value::as_array)
        .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_RESPONSE_GROUPS_REQUIRED".to_string())?
        .clone();

    // 推导答案槽身份并补齐 `AnswerSlotV2` 必填字段（保留模型额外供给的字段）。
    let mut seen_slot_ids: BTreeSet<String> = BTreeSet::new();
    let mut bundle_slot_ids: Vec<String> = Vec::new();
    let mut derived_answer_slots: Vec<Value> = Vec::new();
    let mut question_numbers: Vec<u64> = Vec::new();
    for slot in &answer_slots_input {
        let slot_object = slot
            .as_object()
            .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_ANSWER_SLOT_INVALID".to_string())?;
        let question_number = slot_object
            .get("questionNumber")
            .and_then(Value::as_u64)
            .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_QUESTION_NUMBER_REQUIRED".to_string())?;
        let slot_id = match slot_object.get("slotId").and_then(Value::as_str) {
            Some(value) if !value.trim().is_empty() => value.to_string(),
            // 缺失则确定性推导，绝不引入随机。
            _ => format!("slot-{question_number}"),
        };
        if !seen_slot_ids.insert(slot_id.clone()) {
            return Err(format!("AUTHORING_PATCH_BUNDLE_DUPLICATE_SLOT_ID:{slot_id}"));
        }
        let mut slot_out = slot_object.clone();
        slot_out.insert("slotId".to_string(), json!(slot_id));
        slot_out.insert("questionNumber".to_string(), json!(question_number));
        slot_out
            .entry("displayLabel".to_string())
            .or_insert_with(|| json!(format!("{question_number}")));
        slot_out
            .entry("hostType".to_string())
            .or_insert_with(|| json!("paragraph"));
        slot_out
            .entry("participation".to_string())
            .or_insert_with(|| json!("scoring"));
        slot_out
            .entry("sourceAnchors".to_string())
            .or_insert_with(|| json!([]));
        slot_out
            .entry("confidence".to_string())
            .or_insert_with(|| json!(1.0));
        bundle_slot_ids.push(slot_id);
        question_numbers.push(question_number);
        derived_answer_slots.push(Value::Object(slot_out));
    }

    // 推导任务组身份：优先显式 id，否则从首个题号确定性推导。
    let lookup_id = provided_task_id.or(replaces_task_id);
    let new_task_id = match lookup_id {
        Some(value) => value.to_string(),
        None => {
            let question_number = question_numbers.first().copied();
            match question_number {
                Some(value) => format!("task-{value}"),
                None => return Err("AUTHORING_PATCH_BUNDLE_TASK_ID_REQUIRED".to_string()),
            }
        }
    };

    // 推导响应组身份与所引用的槽 id 集合。
    let mut seen_response_group_ids: BTreeSet<String> = BTreeSet::new();
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    let mut derived_response_groups: Vec<Value> = Vec::new();
    let mut response_group_counter = 0u32;
    for response_group in &raw_response_groups {
        let response_group_object = response_group
            .as_object()
            .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_RESPONSE_GROUP_INVALID".to_string())?;
        response_group_counter += 1;
        let response_group_id = match response_group_object
            .get("responseGroupId")
            .and_then(Value::as_str)
        {
            Some(value) if !value.trim().is_empty() => value.to_string(),
            _ => format!("{new_task_id}-rg{response_group_counter}"),
        };
        if !seen_response_group_ids.insert(response_group_id.clone()) {
            return Err(format!(
                "AUTHORING_PATCH_BUNDLE_DUPLICATE_RESPONSE_GROUP_ID:{response_group_id}"
            ));
        }
        let kind = response_group_object
            .get("kind")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_RESPONSE_GROUP_KIND_REQUIRED".to_string())?;
        let slot_ids = response_group_object
            .get("slotIds")
            .and_then(Value::as_array)
            .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_SLOT_IDS_REQUIRED".to_string())?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|item| item.to_string())
                    .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_SLOT_ID_INVALID".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        for slot_id in &slot_ids {
            referenced.insert(slot_id.clone());
        }
        let mut response_group_out = Map::new();
        response_group_out.insert("responseGroupId".to_string(), json!(response_group_id));
        response_group_out.insert("kind".to_string(), json!(kind));
        response_group_out.insert("slotIds".to_string(), json!(slot_ids));
        response_group_out.insert(
            "cardinality".to_string(),
            response_group_object
                .get("cardinality")
                .cloned()
                .unwrap_or_else(|| json!({"min": 1, "max": 1, "exact": 1})),
        );
        response_group_out.insert(
            "assignment".to_string(),
            response_group_object
                .get("assignment")
                .cloned()
                .unwrap_or_else(|| json!("per_slot")),
        );
        response_group_out.insert(
            "scoringPolicy".to_string(),
            response_group_object
                .get("scoringPolicy")
                .cloned()
                .unwrap_or_else(|| json!("per_slot_binary")),
        );
        response_group_out.insert(
            "duplicatePolicy".to_string(),
            response_group_object
                .get("duplicatePolicy")
                .cloned()
                .unwrap_or_else(|| json!("reject_submission")),
        );
        response_group_out.insert(
            "allowOptionReuse".to_string(),
            response_group_object
                .get("allowOptionReuse")
                .cloned()
                .unwrap_or_else(|| json!(false)),
        );
        response_group_out.insert("sourceAnchors".to_string(), json!([]));
        for optional in ["prompt", "options", "optionBankRef"] {
            if let Some(value) = response_group_object.get(optional) {
                response_group_out.insert(optional.to_string(), value.clone());
            }
        }
        derived_response_groups.push(Value::Object(response_group_out));
    }

    // 组装完整 `TaskGroupV2`：模型只供给结构，后台补齐其余必填字段（确定性默认值）。
    let display_range = if question_numbers.is_empty() {
        json!({"kind": "set", "values": []})
    } else {
        json!({"kind": "set", "values": question_numbers})
    };
    let instruction_signature = json!({
        "normalizedText": "",
        "taskType": task_type,
        "expectedQuestionNumbers": question_numbers,
        "expectedSlotCount": referenced.len().max(question_numbers.len()) as u64,
        "evidenceAnchors": [],
        "confidence": 1.0,
    });
    let mut group = Map::new();
    group.insert("taskId".to_string(), json!(new_task_id));
    group.insert("taskType".to_string(), json!(task_type));
    group.insert("instructions".to_string(), json!(instructions));
    group.insert("responseGroups".to_string(), json!(derived_response_groups));
    group.insert("displayRange".to_string(), display_range);
    group.insert("instructionSignature".to_string(), instruction_signature);
    group.insert("sourceAnchors".to_string(), json!([]));
    group.insert(
        "quality".to_string(),
        json!({"score": 0.0, "sourceCoverage": 0.0, "hardFailures": []}),
    );
    group.insert("reviewState".to_string(), json!("unreviewed"));
    group.insert("recognitionWarnings".to_string(), json!([]));

    // 交叉引用校验（只读文档，校验失败绝不改动）。
    let doc_task_groups = document
        .get("taskGroups")
        .and_then(Value::as_array)
        .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_TASK_GROUPS_MISSING".to_string())?;
    let doc_answer_slots = document
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let doc_answer_key = document
        .get("answerKey")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    // 目标组（被替换）的索引与它所拥有的旧槽。
    let target_index = lookup_id.and_then(|id| {
        doc_task_groups
            .iter()
            .position(|item| item.get("taskId").and_then(Value::as_str) == Some(id))
    });
    let old_owned_slots: BTreeSet<String> = target_index
        .and_then(|index| doc_task_groups.get(index))
        .map(|old_group| collect_group_slot_ids(old_group))
        .unwrap_or_default();

    // 收集其它组（排除目标组）拥有的槽，用于「槽不可跨组共享」校验与 stale 清理保护。
    let mut other_group_slots: BTreeSet<String> = BTreeSet::new();
    for (index, other_group) in doc_task_groups.iter().enumerate() {
        if Some(index) == target_index {
            continue;
        }
        for slot_id in collect_group_slot_ids(other_group) {
            other_group_slots.insert(slot_id);
        }
    }

    let answer_slots_empty = answer_slots_input.is_empty();
    for slot_id in &referenced {
        let in_bundle = bundle_slot_ids.iter().any(|item| item == slot_id);
        let in_document = doc_answer_slots.contains_key(slot_id);
        if !in_bundle && !in_document {
            // 组引用了槽却没有带来定义：整个 bundle 没有 answerSlots 时给更具体的码。
            if answer_slots_empty {
                return Err("AUTHORING_PATCH_BUNDLE_ANSWER_SLOTS_EMPTY".to_string());
            }
            return Err(format!("AUTHORING_PATCH_BUNDLE_SLOT_REFERENCE_MISSING:{slot_id}"));
        }
        if other_group_slots.contains(slot_id) {
            return Err(format!("AUTHORING_PATCH_BUNDLE_SLOT_CLAIMED:{slot_id}"));
        }
        // 被引用的槽必须最终落到一条答案键上——缺失即报错，绝不臆造标准答案。
        let key_in_bundle = answer_key_input.contains_key(slot_id);
        let key_in_document = doc_answer_key.contains_key(slot_id);
        if !key_in_bundle && !key_in_document {
            return Err(format!("AUTHORING_PATCH_BUNDLE_ANSWER_KEY_MISSING:{slot_id}"));
        }
    }

    // ── 全部校验通过：对克隆做落地，失败整体回退，保证原子性 ─────────────
    let mut next = document.clone();
    {
        let next_task_groups = next
            .get_mut("taskGroups")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_TASK_GROUPS_MISSING".to_string())?;
        match target_index {
            Some(index) => {
                // 原地替换，保持数组位置。
                next_task_groups[index] = Value::Object(group);
            }
            None => {
                // 插入：优先 `insertAfterTaskId` 之后，否则追加到末尾。
                let insert_at = insert_after
                    .and_then(|after| {
                        next_task_groups
                            .iter()
                            .position(|item| item.get("taskId").and_then(Value::as_str) == Some(after))
                    })
                    .map(|index| index + 1)
                    .unwrap_or(next_task_groups.len());
                next_task_groups.insert(insert_at, Value::Object(group));
            }
        }
    }

    // 落地答案槽（仅 upsert bundle 带来的定义；已有的复用定义予以保留）。
    {
        let next_answer_slots = next
            .get_mut("answerSlots")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_ANSWER_SLOTS_MISSING".to_string())?;
        for slot in &derived_answer_slots {
            if let Some(slot_id) = slot.get("slotId").and_then(Value::as_str) {
                next_answer_slots.insert(slot_id.to_string(), slot.clone());
            }
        }
    }

    // 落地答案键（仅 upsert 调用方提供的条目，绝不臆造）。
    {
        let next_answer_key = next
            .get_mut("answerKey")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "AUTHORING_PATCH_BUNDLE_ANSWER_KEY_MISSING".to_string())?;
        for (slot_id, value) in &answer_key_input {
            next_answer_key.insert(slot_id.clone(), value.clone());
        }
    }

    // 替换时清理旧组拥有、但新束不再引用的 stale 槽（除非仍有其它组引用）。
    if let Some(_) = target_index {
        let stale: Vec<String> = old_owned_slots
            .iter()
            .filter(|slot_id| !referenced.contains(*slot_id))
            .filter(|slot_id| !other_group_slots.contains(*slot_id))
            .cloned()
            .collect();
        // 同时清掉这些 stale 槽对应的答案键。
        if !stale.is_empty() {
            if let Some(next_answer_slots) = next.get_mut("answerSlots").and_then(Value::as_object_mut)
            {
                for slot_id in &stale {
                    next_answer_slots.remove(slot_id);
                }
            }
            if let Some(next_answer_key) = next.get_mut("answerKey").and_then(Value::as_object_mut) {
                for slot_id in &stale {
                    next_answer_key.remove(slot_id);
                }
            }
        }
    }

    *document = next;
    Ok(())
}

/// 收集一个任务组通过其 `responseGroups[].slotIds` 引用的全部槽 id。
fn collect_group_slot_ids(group: &Value) -> BTreeSet<String> {
    let mut collected: BTreeSet<String> = BTreeSet::new();
    if let Some(response_groups) = group.get("responseGroups").and_then(Value::as_array) {
        for response_group in response_groups {
            if let Some(slot_ids) = response_group.get("slotIds").and_then(Value::as_array) {
                for slot_id in slot_ids {
                    if let Some(value) = slot_id.as_str() {
                        collected.insert(value.to_string());
                    }
                }
            }
        }
    }
    collected
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
            | "assetId"
            | "rowSpan"
            | "colSpan"
            | "headerScope"
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
        apply_patch, explicitly_handled_issue_targets, expand_question_expression,
        export_authoring_v2_core, materialize_authoring_assets, physical_shadow_matches_authoring,
        preserve_issue_resolutions, resolve_authoring_asset_preview_core,
        unresolved_blocking_issues, validate_authoring_v2_publish_readiness,
        AUTHORING_V2_SHADOW_FILE,
    };
    use crate::schema::common::{AssetDescriptorV2, AssetExtractionModeV2, AssetKindV2};
    use serde_json::{json, Value};
    use std::collections::BTreeSet;
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
        // 修前这里断言的是精确串 `authoring_v2_export_blocked:quality_state=review_required`。
        // 该串现在由 `publish_verdict` 统一产出（多判据合一，见 NOTES §7.4），一次给出
        // 全部阻断项而不只是第一个命中的那条。所以断言改为钉**判据本身**：
        // 必须报 `QUALITY_NOT_READY` 且带上具体 state。
        // 刻意不钉整串格式：钉格式会让每次收敛判据都要改测试，而且判据真丢了也照样绿。
        assert!(
            error.starts_with("authoring_v2_export_blocked:blocked:"),
            "必须是 blocked 结论：{error}"
        );
        assert!(error.contains("QUALITY_NOT_READY"), "{error}");
        assert!(error.contains("quality_state=review_required"), "{error}");
        // 关键实体断言不变：判据没过就一个产物都不许落地。
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
        preserve_issue_resolutions(&mut quality, authoring.get("quality"), &BTreeSet::new());
        assert_eq!(quality["issues"][0]["details"]["resolution"], "ignored");
        assert_eq!(quality["issues"][0]["details"]["note"], "reviewed");
    }

    #[test]
    fn editing_instruction_text_recomputes_signature_before_quality() {
        // Editing instruction text must re-derive the stored `instructionSignature`
        // from the new text (so quality reflects the edited instruction), while
        // preserving the user-confirmed question numbering.
        let mut document = json!({
            "taskGroups": [{
                "taskId": "task-1",
                "displayRange": {"kind":"range","start":1,"end":2},
                "taskType": "sentence_completion",
                "instructions": [{
                    "type":"paragraph",
                    "id":"task-1-instructions",
                    "children":[{
                        "type":"text",
                        "id":"task-1-instructions-text",
                        "text":"Complete the sentences below. Choose NO MORE THAN TWO WORDS."
                    }]
                }],
                "instructionSignature": {
                    "normalizedText":"Complete the sentences below. Choose NO MORE THAN TWO WORDS.",
                    "taskType":"sentence_completion",
                    "expectedQuestionNumbers":[1,2],
                    "expectedSlotCount":2
                },
                "responseGroups": []
            }],
            "quality": {"issues": []}
        });
        // Replace "sentences" (offset 13..22) with "summary" -> a clearly
        // different task type, still a completion.
        apply_patch(
            &mut document,
            &json!({
                "op":"replaceText",
                "nodeId":"task-1-instructions-text",
                "from":13,
                "to":22,
                "text":"summary"
            }),
        )
        .unwrap();
        let signature = &document["taskGroups"][0]["instructionSignature"];
        assert_eq!(
            signature["normalizedText"],
            "Complete the summary below. Choose NO MORE THAN TWO WORDS."
        );
        assert_eq!(signature["taskType"], "summary_completion");
        // User-confirmed question numbering is preserved across the re-derivation.
        assert_eq!(signature["expectedQuestionNumbers"], json!([1, 2]));
        assert_eq!(signature["expectedSlotCount"], 2);
        // Word limit is re-derived from the new (still text-entry) instruction.
        assert_eq!(signature["wordLimit"]["maxWords"], 2);
    }

    #[test]
    fn distinct_facts_on_same_target_do_not_share_resolution_state() {
        // Two genuinely different facts against the same target carry distinct
        // issueIds, so resolving/ignoring one must not bleed into the other.
        // "The same fact" == an identical issueId (code + target + message + ...).
        let previous = json!({
            "issues": [
                {"issueId":"phase4-SLOT_HOST_MISSING-task-1-aaaa","details":{"resolution":"ignored","note":"inline"}},
                {"issueId":"phase4-SLOT_HOST_MISSING-task-1-bbbb","details":{"resolution":"resolved","note":"host"}}
            ]
        });
        let mut quality = json!({
            "issues": [
                {"issueId":"phase4-SLOT_HOST_MISSING-task-1-aaaa","details":{}},
                {"issueId":"phase4-SLOT_HOST_MISSING-task-1-bbbb","details":{}}
            ]
        });
        // 注意实参形状：`preserve_issue_resolutions` 的第二个参数是**整个 quality 对象**
        // （它内部会自行 `.get("issues")`），这里必须传 `Some(&previous)` 而不是
        // `previous.get("issues")`。后者是数组，在其上 `.get("issues")` 恒为 None，
        // 会导致什么都继承不到（曾使本测试误报：拿到的 resolution 是 Null）。
        preserve_issue_resolutions(&mut quality, Some(&previous), &BTreeSet::new());
        assert_eq!(quality["issues"][0]["details"]["resolution"], "ignored");
        assert_eq!(quality["issues"][1]["details"]["resolution"], "resolved");
    }

    /// 本次改动碰过的目标：旧 resolution 必须重置；没碰过的目标照旧继承。
    ///
    /// 这条锁的是「拿旧内容的处理冒充新内容没问题」这条捷径：只要 issueId 恰好没变，
    /// 按 issueId 继承就会把人对**改动前**内容的判断带到**改动后**的稿子上。受影响
    /// 目标必须按未处理重来，而其他有效的人工处理不能被连坐清掉。
    #[test]
    fn affected_targets_lose_their_stale_resolutions_while_others_keep_theirs() {
        let previous = json!({
            "issues": [
                {"issueId":"issue-touched","targetId":"task-1",
                 "details":{"resolution":"resolved","note":"看过了"}},
                {"issueId":"issue-untouched","targetId":"task-2",
                 "details":{"resolution":"ignored","note":"有意保留"}}
            ]
        });
        let mut quality = json!({
            "issues": [
                {"issueId":"issue-touched","targetId":"task-1","details":{}},
                {"issueId":"issue-untouched","targetId":"task-2","details":{}}
            ]
        });

        let affected: BTreeSet<String> = ["task-1".to_string()].into_iter().collect();
        preserve_issue_resolutions(&mut quality, Some(&previous), &affected);

        // 受影响目标：重置（不继承 resolution，也不继承 note）。
        assert!(quality["issues"][0]["details"].get("resolution").is_none(),
            "受影响目标的旧 resolution 必须重置: {quality:#}");
        assert!(quality["issues"][0]["details"].get("note").is_none(),
            "受影响目标的旧 note 也必须一并重置: {quality:#}");
        // 未受影响目标：有效的人工处理保留。
        assert_eq!(quality["issues"][1]["details"]["resolution"], "ignored");
        assert_eq!(quality["issues"][1]["details"]["note"], "有意保留");
    }

    /// 人在本批里亲手确认过的目标要被扣掉：他自己的确认不能被自己的编辑顺手清掉。
    ///
    /// 判据只看 `op == "resolveIssue"` 且该 `issueId` 在当前稿的质量块里能找到目标。
    /// 找不到（例如引用了不存在的 issueId）就什么都不产出——不能凭空造出一个"处理过"
    /// 的目标，那会让某个目标的过期 resolution 逃过重置。
    #[test]
    fn explicitly_handled_targets_come_from_the_batch_own_resolutions() {
        let authoring = json!({
            "quality": {"issues": [
                {"issueId":"i1","targetId":"task-1","severity":"blocking",
                 "details":{"resolution":"resolved"}},
                {"issueId":"i2","targetId":"task-2","severity":"blocking","details":{}}
            ]}
        });
        let commands = json!([
            {"op":"resolveIssue","issueId":"i1","resolution":"resolved"},
            {"op":"replaceText","nodeId":"task-1-instructions-text","from":0,"to":1,"text":"x"},
            {"op":"resolveIssue","issueId":"nope","resolution":"ignored"},
            {"op":"setAnswer","slotId":"q1","value":{"kind":"unresolved"}}
        ]);
        let handled = explicitly_handled_issue_targets(&authoring, commands.as_array().unwrap());
        assert_eq!(
            handled,
            ["task-1".to_string()].into_iter().collect::<BTreeSet<_>>(),
            "只有本批明确 resolveIssue 过的目标才算"
        );
    }

    /// 「仍未处理」的唯一判据：只有 blocking 且没有 resolved/ignored 才算。
    ///
    /// 三处（预检 / 导出 / 云端修复终检）共用它，所以它的边界必须钉死：非阻断的
    /// warning/info 不能算作剩余任务（否则用户会被一条提示拦住），而 `resolved` /
    /// `ignored` 之外的任何取值（含缺失）都算未处理（fail-closed）。
    #[test]
    fn unresolved_blocking_issues_uses_one_predicate_for_every_consumer() {
        let authoring = json!({
            "quality": {
                "issues": [
                    {"issueId":"a","severity":"blocking","code":"X","targetId":"t1","details":{}},
                    {"issueId":"b","severity":"blocking","code":"X","targetId":"t2",
                     "details":{"resolution":"resolved"}},
                    {"issueId":"c","severity":"blocking","code":"X","targetId":"t3",
                     "details":{"resolution":"ignored"}},
                    {"issueId":"d","severity":"warning","code":"X","targetId":"t4","details":{}},
                    {"issueId":"e","severity":"info","code":"X","targetId":"t5","details":{}},
                    // 未知 resolution 取值：不认识 ≠ 已处理。
                    {"issueId":"f","severity":"blocking","code":"X","targetId":"t6",
                     "details":{"resolution":"maybe"}}
                ]
            }
        });
        let unresolved = unresolved_blocking_issues(&authoring);
        let ids: Vec<&str> = unresolved
            .iter()
            .filter_map(|issue| issue.get("issueId").and_then(Value::as_str))
            .collect();
        assert_eq!(ids, vec!["a", "f"], "只有未处理的 blocking 才算剩余问题");
    }

    /// 当前稿没有 quality 块（或它不是对象）时不能 panic，也不能凭空造出剩余问题。
    #[test]
    fn unresolved_blocking_issues_tolerates_a_missing_quality_block() {
        assert!(unresolved_blocking_issues(&json!({})).is_empty());
        assert!(unresolved_blocking_issues(&json!({"quality": null})).is_empty());
        assert!(unresolved_blocking_issues(&json!({"quality": {"issues": null}})).is_empty());
    }

    /// 一个用于 `upsertTaskGroupBundle` 测试的最小规范文档：已有一个 task-1，
    /// 答案槽/答案键注册表均为空。与 `structured_document` 同风格，但补齐了
    /// `answerSlots`/`answerKey` 两个注册表。
    fn bundle_document() -> serde_json::Value {
        json!({
            "taskGroups": [{
                "taskId": "task-1",
                "taskType": "sentence_completion",
                "instructions": [],
                "responseGroups": [{
                    "responseGroupId": "task-1-rg1",
                    "kind": "text_entry",
                    "slotIds": []
                }]
            }],
            "answerSlots": {},
            "answerKey": {},
            "quality": {"issues": []}
        })
    }

    #[test]
    fn upsert_task_group_bundle_creates_new_group_with_derived_ids() {
        let mut document = bundle_document();
        apply_patch(
            &mut document,
            &json!({
                "op": "upsertTaskGroupBundle",
                "taskGroup": {
                    "taskType": "sentence_completion",
                    "instructions": [{"type": "paragraph", "id": "task-27-instructions", "children": []}],
                    "responseGroups": [{
                        "kind": "text_entry",
                        "slotIds": ["slot-27"]
                    }]
                },
                "answerSlots": [{"questionNumber": 27, "interaction": "text"}],
                "answerKey": {"slot-27": {"kind": "text", "values": ["example"]}},
                "insertAfterTaskId": "task-26"
            }),
        )
        .unwrap();
        // 没有 task-26，因此追加到末尾；组数为 2。
        assert_eq!(document["taskGroups"].as_array().unwrap().len(), 2);
        let group = &document["taskGroups"][1];
        // 缺少 taskId → 由首个题号推导为 task-27。
        assert_eq!(group["taskId"], "task-27");
        // 缺少 responseGroupId → 推导为 {taskId}-rg1。
        assert_eq!(group["responseGroups"][0]["responseGroupId"], "task-27-rg1");
        assert_eq!(group["responseGroups"][0]["slotIds"], json!(["slot-27"]));
        // 缺少 slotId → 推导为 slot-27；槽与答案键均已注册。
        assert!(document["answerSlots"]["slot-27"].is_object());
        assert_eq!(document["answerSlots"]["slot-27"]["questionNumber"], 27);
        assert_eq!(
            document["answerKey"]["slot-27"],
            json!({"kind": "text", "values": ["example"]})
        );
    }

    #[test]
    fn upsert_task_group_bundle_replaces_existing_group_keeping_position_and_removing_stale_slot() {
        let mut document = json!({
            "taskGroups": [
                {
                    "taskId": "task-26",
                    "taskType": "sentence_completion",
                    "instructions": [],
                    "responseGroups": [{"responseGroupId": "task-26-rg1", "kind": "text_entry", "slotIds": ["slot-26"]}]
                },
                {
                    "taskId": "task-27",
                    "taskType": "sentence_completion",
                    "instructions": [],
                    "responseGroups": [{"responseGroupId": "task-27-rg1", "kind": "text_entry", "slotIds": ["slot-27-old"]}]
                }
            ],
            "answerSlots": {
                "slot-26": {"slotId": "slot-26", "questionNumber": 26, "interaction": "text"},
                "slot-27-old": {"slotId": "slot-27-old", "questionNumber": 27, "interaction": "text"}
            },
            "answerKey": {
                "slot-26": {"kind": "unresolved"},
                "slot-27-old": {"kind": "unresolved"}
            },
            "quality": {"issues": []}
        });
        apply_patch(
            &mut document,
            &json!({
                "op": "upsertTaskGroupBundle",
                "taskGroup": {
                    "taskId": "task-27",
                    "taskType": "sentence_completion",
                    "instructions": [],
                    "responseGroups": [{"responseGroupId": "task-27-rg1", "kind": "text_entry", "slotIds": ["slot-27-new"]}]
                },
                "answerSlots": [{"slotId": "slot-27-new", "questionNumber": 27, "interaction": "text"}],
                "answerKey": {"slot-27-new": {"kind": "text", "values": ["fresh"]}}
            }),
        )
        .unwrap();
        // 原地替换，数组位置不变：task-26 在前、task-27 在后。
        assert_eq!(document["taskGroups"][0]["taskId"], "task-26");
        assert_eq!(document["taskGroups"][1]["taskId"], "task-27");
        assert_eq!(document["taskGroups"][1]["responseGroups"][0]["slotIds"], json!(["slot-27-new"]));
        // task-26 的槽不受影响。
        assert!(document["answerSlots"].get("slot-26").is_some());
        assert!(document["answerKey"].get("slot-26").is_some());
        // 旧组拥有、新束不再引用的 slot-27-old 被清理。
        assert!(document["answerSlots"].get("slot-27-old").is_none());
        assert!(document["answerKey"].get("slot-27-old").is_none());
        // 新槽已落地。
        assert!(document["answerSlots"].get("slot-27-new").is_some());
        assert_eq!(document["answerKey"]["slot-27-new"], json!({"kind": "text", "values": ["fresh"]}));
    }

    #[test]
    fn upsert_task_group_bundle_rejects_slot_claimed_by_another_group() {
        let mut document = json!({
            "taskGroups": [{
                "taskId": "task-26",
                "taskType": "sentence_completion",
                "instructions": [],
                "responseGroups": [{"responseGroupId": "task-26-rg1", "kind": "text_entry", "slotIds": ["slot-26"]}]
            }],
            "answerSlots": {"slot-26": {"slotId": "slot-26", "questionNumber": 26, "interaction": "text"}},
            "answerKey": {"slot-26": {"kind": "unresolved"}},
            "quality": {"issues": []}
        });
        let error = apply_patch(
            &mut document,
            &json!({
                "op": "upsertTaskGroupBundle",
                "taskGroup": {
                    "taskType": "sentence_completion",
                    "instructions": [],
                    "responseGroups": [{"kind": "text_entry", "slotIds": ["slot-26"]}]
                },
                "answerSlots": [{"questionNumber": 26, "interaction": "text"}],
                "answerKey": {"slot-26": {"kind": "text", "values": ["x"]}}
            }),
        )
        .expect_err("slot owned by another group must be rejected");
        assert!(
            error.contains("AUTHORING_PATCH_BUNDLE_SLOT_CLAIMED:slot-26"),
            "{error}"
        );
    }

    #[test]
    fn upsert_task_group_bundle_rejects_missing_answer_key() {
        let mut document = bundle_document();
        let error = apply_patch(
            &mut document,
            &json!({
                "op": "upsertTaskGroupBundle",
                "taskGroup": {
                    "taskType": "sentence_completion",
                    "instructions": [],
                    "responseGroups": [{"kind": "text_entry", "slotIds": ["slot-27"]}]
                },
                "answerSlots": [{"questionNumber": 27, "interaction": "text"}],
                "answerKey": {}
            }),
        )
        .expect_err("referenced slot without an answer key must be rejected");
        assert!(
            error.contains("AUTHORING_PATCH_BUNDLE_ANSWER_KEY_MISSING:slot-27"),
            "{error}"
        );
    }

    #[test]
    fn upsert_task_group_bundle_preserves_supplied_ids_verbatim() {
        let mut document = bundle_document();
        apply_patch(
            &mut document,
            &json!({
                "op": "upsertTaskGroupBundle",
                "taskGroup": {
                    "taskId": "task-custom",
                    "taskType": "sentence_completion",
                    "instructions": [],
                    "responseGroups": [{"responseGroupId": "rg-custom", "kind": "text_entry", "slotIds": ["slot-custom"]}]
                },
                "answerSlots": [{"slotId": "slot-custom", "questionNumber": 99, "interaction": "text"}],
                "answerKey": {"slot-custom": {"kind": "text", "values": ["verbatim"]}}
            }),
        )
        .unwrap();
        let group = &document["taskGroups"][1];
        assert_eq!(group["taskId"], "task-custom");
        assert_eq!(group["responseGroups"][0]["responseGroupId"], "rg-custom");
        assert_eq!(group["responseGroups"][0]["slotIds"], json!(["slot-custom"]));
        assert!(document["answerSlots"].get("slot-custom").is_some());
        assert_eq!(document["answerSlots"]["slot-custom"]["questionNumber"], 99);
    }

    #[test]
    fn upsert_task_group_bundle_is_deterministic_across_identical_documents() {
        let build = || {
            let mut document = bundle_document();
            apply_patch(
                &mut document,
                &json!({
                    "op": "upsertTaskGroupBundle",
                    "taskGroup": {
                        "taskType": "sentence_completion",
                        "instructions": [],
                        "responseGroups": [{"kind": "text_entry", "slotIds": ["slot-27"]}]
                    },
                    "answerSlots": [{"questionNumber": 27, "interaction": "text"}],
                    "answerKey": {"slot-27": {"kind": "text", "values": ["example"]}}
                }),
            )
            .unwrap();
            document
        };
        let first = build();
        let second = build();
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
    }

    #[test]
    fn upsert_task_group_bundle_rejects_forbidden_provenance_key() {
        let mut document = bundle_document();
        let error = apply_patch(
            &mut document,
            &json!({
                "op": "upsertTaskGroupBundle",
                "provenanceStatus": "source",
                "taskGroup": {
                    "taskType": "sentence_completion",
                    "instructions": [],
                    "responseGroups": [{"kind": "text_entry", "slotIds": ["slot-27"]}]
                },
                "answerSlots": [{"questionNumber": 27, "interaction": "text"}],
                "answerKey": {"slot-27": {"kind": "text", "values": ["example"]}}
            }),
        )
        .expect_err("provenance injection must be rejected");
        assert!(
            error.contains("AUTHORING_PATCH_BUNDLE_FORBIDDEN_KEY:provenanceStatus"),
            "{error}"
        );
    }
}
