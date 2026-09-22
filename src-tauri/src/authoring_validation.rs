use crate::reading_source::{question_order_from_authoring, reading_source};
use crate::validator::{
    has_error_issues, is_error_issue, json_issue, qid_sort_key, validate_reading_source_contract,
    validation_layers, ValidationReportV1,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub(crate) fn validate_authoring(job_id: &str, authoring: Option<&Value>) -> Value {
    let mut issues = Vec::new();
    if let Some(ir) = authoring {
        if ir
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .is_empty()
        {
            issues.push(json_issue(
                "AuthoringIR",
                "$.exam.examId",
                "examId is required",
            ));
        }
        if ir
            .get("groups")
            .and_then(Value::as_array)
            .map(|items| items.is_empty())
            .unwrap_or(true)
        {
            issues.push(json_issue(
                "AuthoringIR",
                "$.groups",
                "At least one question group is required",
            ));
        }
        let source = reading_source(ir);
        issues.extend(validate_reading_source_contract(&source));

        let question_order = question_order_from_authoring(ir);
        let mut seen_qids = HashSet::new();
        let mut duplicate_qids = HashSet::new();
        for qid in &question_order {
            if !seen_qids.insert(qid.clone()) {
                duplicate_qids.insert(qid.clone());
            }
        }
        for qid in duplicate_qids {
            issues.push(json_issue(
                "AuthoringIR",
                "$.questionOrder",
                &format!("Duplicate question id in questionOrder: {}", qid),
            ));
        }

        let mut numeric_order = question_order
            .iter()
            .filter_map(|qid| qid_sort_key(qid))
            .collect::<Vec<_>>();
        numeric_order.sort_unstable();
        numeric_order.dedup();
        if let (Some(first), Some(last)) = (
            numeric_order.first().copied(),
            numeric_order.last().copied(),
        ) {
            let expected_len = (last - first + 1) as usize;
            if expected_len != numeric_order.len() {
                issues.push(json_issue(
                    "ReadingExamSourceV1",
                    "$.questionOrder",
                    &format!(
                        "questionOrder must be numerically continuous from q{} to q{}",
                        first, last
                    ),
                ));
            }
        }

        let mut display_seen: HashMap<String, String> = HashMap::new();
        for question in ir
            .get("groups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|group| {
                group
                    .get("questions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
        {
            if let Some(qid) = question.get("id").and_then(Value::as_str) {
                let display = question
                    .get("displayNumber")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if display.is_empty() {
                    issues.push(json_issue(
                        "ReadingExamSourceV1",
                        &format!("$.questionDisplayMap.{}", qid),
                        "questionDisplayMap display number cannot be empty",
                    ));
                } else if let Some(existing_qid) =
                    display_seen.insert(display.clone(), qid.to_string())
                {
                    issues.push(json_issue(
                        "ReadingExamSourceV1",
                        "$.questionDisplayMap",
                        &format!(
                            "Duplicate display number {} for {} and {}",
                            display, existing_qid, qid
                        ),
                    ));
                }
            }
        }
    } else {
        issues.push(json_issue("AuthoringIR", "$", "Authoring IR is missing"));
    }

    ValidationReportV1 {
        job_id: job_id.to_string(),
        passed: !has_error_issues(&issues),
        layers: validation_layers(&issues),
        issues,
        generated_at: Utc::now().to_rfc3339(),
        runtime: None,
    }
    .to_value()
}

pub(crate) fn merge_sidecar_validation(base: &mut Value, sidecar: Value) {
    let Some(base_obj) = base.as_object_mut() else {
        return;
    };
    let sidecar_issues = sidecar
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let sidecar_layers = sidecar
        .get("layers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let layers_to_replace = sidecar_layers
        .iter()
        .filter_map(|layer| {
            layer
                .get("layer")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    // 缺省语义必须是**合并**，不是**替换**。
    //
    // 这里曾经有两个叠加的缺省，合起来构成一条静默改判通道：
    //   1. `layers` 缺失 ⇒ `layers_to_replace` 退化成 ["ReadingExamSourceV1","DomProtocol"]；
    //   2. `replaceExistingLayers` 缺失 ⇒ true。
    // 于是一份"什么都不说"的侧车输出（`{}`）会把 base 上这两层的 error 整层删掉，
    // 再由下面的 `!has_error_issues(merged)` 把 passed 重算成 true —— 校验器什么都没说，
    // 结论却从"不能发布"变成"可以发布"。
    //
    // `runtime_validation.rs:340` 处显式置 false 说明调用方本来就期望合并语义；缺省与它相反
    // 是纯粹的意外。现在把缺省对齐到实际期望，并把"空列表 = 替换默认两层"这条回退删掉：
    // 空列表就是"不替换任何层"。
    //
    // 替换能力本身保留（协议未变），但必须由调用方**显式**写 `replaceExistingLayers: true`。
    let replace_existing_layers = sidecar
        .get("replaceExistingLayers")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut merged_issues = base_obj
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|issue| {
            if !replace_existing_layers {
                return true;
            }
            issue
                .get("layer")
                .and_then(Value::as_str)
                .map(|layer| !layers_to_replace.iter().any(|item| item == layer))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    merged_issues.extend(sidecar_issues);

    let layers = validation_layers(&merged_issues);
    base_obj.insert(
        "passed".to_string(),
        json!(!has_error_issues(&merged_issues)),
    );
    base_obj.insert("layers".to_string(), json!(layers));
    base_obj.insert("issues".to_string(), json!(merged_issues));
    if let Some(runtime) = sidecar.get("runtime") {
        base_obj.insert("runtime".to_string(), runtime.clone());
    }
}

pub(crate) fn merge_validation_issues(report: &mut Value, extra_issues: Vec<Value>) {
    if extra_issues.is_empty() {
        return;
    }
    let Some(obj) = report.as_object_mut() else {
        return;
    };
    let mut issues = obj
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    issues.extend(extra_issues);
    let layers = validation_layers(&issues);
    obj.insert("passed".to_string(), json!(!has_error_issues(&issues)));
    obj.insert("layers".to_string(), json!(layers));
    obj.insert("issues".to_string(), json!(issues));
}

// ─────────────────────────────────────────────────────────────────────────────
// 发布判据的唯一来源
// ─────────────────────────────────────────────────────────────────────────────

/// 一条 blocking issue 是否**仍未处理**。
///
/// 这是发布判据里**唯一的那份谓词**，三处共用：质量就绪度
/// （[`crate::ielts_grammar::quality`] 的 `readiness_from_facts`）、预检
/// （`authoring_v2_commands::check_publish_preflight`）、以及云端修复的终检
/// （`cloud_repair::remaining_tasks` 重算剩余用户任务）。曾经这三处各自内联同一段
/// 谓词 —— 只要有一处被改动，就会出现「预检说可以发布、修复循环却认为还剩问题」
/// 这类同稿不同判。
///
/// 只看 `details.resolution`：它是**人**通过 `resolveIssue` 写下的。模型写不了
/// （`cloud_repair::tools::MODEL_ALLOWED_OPS` 有意不含该命令），所以这里不会变成
/// 模型自称「已修复」的通道。
pub(crate) fn blocking_issue_unresolved_parts(severity: &str, details: Option<&Value>) -> bool {
    severity == "blocking"
        && !matches!(
            details
                .and_then(|details| details.get("resolution"))
                .and_then(Value::as_str),
            Some("resolved") | Some("ignored")
        )
}

pub(crate) fn blocking_issue_unresolved(issue: &Value) -> bool {
    blocking_issue_unresolved_parts(
        issue
            .get("severity")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        issue.get("details"),
    )
}

/// 当前稿上仍未处理的阻断性问题。判据见 [`blocking_issue_unresolved`]。
pub(crate) fn unresolved_blocking_issues(authoring: &Value) -> Vec<Value> {
    authoring
        .get("quality")
        .and_then(|quality| quality.get("issues"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|issue| blocking_issue_unresolved(issue))
        .cloned()
        .collect()
}

/// 判据范围。**它描述的是稿件来源，不是调用方可调的开关。**
///
/// - [`PublishScope::CanonicalDirect`]：稿件来自 `library_items_v2.canonical_ds_json`
///   （DB 权威稿），版本已知。此时派生文件不是事实源 —— 这是
///   `authoring_v2_commands.rs` 里既有的、有注释记录的约束（M1 typed-preflight 直通）。
/// - [`PublishScope::FullDerived`]：稿件来自派生文件（`authoring-ir.json` / revision /
///   shadow），必须追加上"文件状态类"判据。
///
/// **范围只决定"哪些判据适用"，不提供任何"跳过某条判据"的能力**：
/// 两个范围都必须跑完自己范围内的全部判据，任何一条不得被放行。
/// 范围本身由入口决定，不可由请求体、报告或 policy 参数改写。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishScope {
    CanonicalDirect,
    FullDerived,
}

/// 一个阻断项。`code` 是稳定标识，供 UI / 测试 / 审计按值匹配（不要按 message 匹配）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublishBlocker {
    pub code: &'static str,
    pub layer: &'static str,
    pub target: Option<String>,
    pub message: String,
}

/// 「这份稿能不能发布」的**唯一**结论类型。
///
/// 用枚举而不是布尔，是因为「判据没能执行」既不是通过、也不等于内容有错：
///
/// - `Ready`：范围内所有判据都跑过且都通过；
/// - `Blocked`：判据跑过了，结论是「不能发」；
/// - `Undetermined`：**判据没能执行**（或执行结果不可信）⇒ **绝不允许当作通过**。
///
/// 修前这套判断散在四处，且各自只产出布尔；`Undetermined` 无处可表达，
/// 于是"没能执行"只能被压进"通过"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PublishVerdict {
    Ready { edit_version: Option<i64> },
    Blocked { reasons: Vec<PublishBlocker> },
    Undetermined { reasons: Vec<PublishBlocker> },
}

impl PublishVerdict {
    pub(crate) fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    pub(crate) fn status(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "ready",
            Self::Blocked { .. } => "blocked",
            Self::Undetermined { .. } => "undetermined",
        }
    }

    pub(crate) fn reasons(&self) -> &[PublishBlocker] {
        match self {
            Self::Ready { .. } => &[],
            Self::Blocked { reasons } | Self::Undetermined { reasons } => reasons,
        }
    }

    pub(crate) fn to_value(&self) -> Value {
        json!({
            "schemaVersion": "PublishVerdictV1",
            "status": self.status(),
            "ready": self.is_ready(),
            "editVersion": match self {
                Self::Ready { edit_version } => json!(edit_version),
                _ => Value::Null,
            },
            "reasons": self
                .reasons()
                .iter()
                .map(|reason| json!({
                    "code": reason.code,
                    "layer": reason.layer,
                    "target": reason.target,
                    "message": reason.message,
                }))
                .collect::<Vec<_>>(),
        })
    }
}

#[derive(Default)]
struct PublishVerdictBuilder {
    blocked: Vec<PublishBlocker>,
    undetermined: Vec<PublishBlocker>,
}

impl PublishVerdictBuilder {
    fn block(
        &mut self,
        code: &'static str,
        layer: &'static str,
        target: impl Into<Option<String>>,
        message: impl Into<String>,
    ) {
        self.blocked.push(PublishBlocker {
            code,
            layer,
            target: target.into(),
            message: message.into(),
        });
    }

    fn undetermined(
        &mut self,
        code: &'static str,
        layer: &'static str,
        target: impl Into<Option<String>>,
        message: impl Into<String>,
    ) {
        self.undetermined.push(PublishBlocker {
            code,
            layer,
            target: target.into(),
            message: message.into(),
        });
    }

    fn finish(mut self, edit_version: Option<i64>) -> PublishVerdict {
        if !self.blocked.is_empty() {
            // 有确凿阻断时，"能不能发"已经有答案了：报 Blocked（可操作性更好），
            // 但把"没能执行"的项**一并列出**，不隐藏。
            let mut reasons = std::mem::take(&mut self.blocked);
            reasons.extend(self.undetermined);
            return PublishVerdict::Blocked { reasons };
        }
        if !self.undetermined.is_empty() {
            return PublishVerdict::Undetermined {
                reasons: self.undetermined,
            };
        }
        PublishVerdict::Ready { edit_version }
    }
}

fn is_authoring_v2(authoring: &Value) -> bool {
    authoring.get("schemaVersion").and_then(Value::as_str) == Some("IeltsAuthoringIRV2")
}

fn uncoded_target(label: &str) -> Option<String> {
    (!label.trim().is_empty()).then(|| label.to_string())
}

/// **发布判据的唯一入口。**
///
/// 入参只有**稿件 + 版本**（外加定位来源所需的 `root` / `job_id`）：
/// 不接受 report，也不接受 policy / options。调用方只能读结论，
/// 既不能自己重算，也没有参数可以绕过任何一条判据。
///
/// 版本必须由调用方显式给出：`Some(v)` 表示稿件确实取自 canonical 的第 v 版，
/// `None` 表示版本未知（派生文件来源）。从类型上消除「校验 A 版本、导出 B 版本」。
pub(crate) fn publish_verdict(
    root: &Path,
    job_id: &str,
    authoring: &Value,
    edit_version: Option<i64>,
    scope: PublishScope,
) -> PublishVerdict {
    let mut builder = PublishVerdictBuilder::default();
    let v2 = is_authoring_v2(authoring);

    if v2 {
        if let Err(error) = crate::authoring_v2_commands::validate_authoring(authoring) {
            builder.block("SCHEMA_INVALID", "AuthoringIR", None, error);
            return builder.finish(edit_version);
        }
        // 质量就绪度：唯一规则在 `ielts_grammar::quality::quality_readiness`，
        // 与被写入 `QualityReportV2.state` 的是同一份实现。
        match crate::ielts_grammar::quality::quality_readiness(authoring) {
            Ok(crate::ielts_grammar::quality::QualityReadiness::Ready) => {}
            Ok(state) => builder.block(
                "QUALITY_NOT_READY",
                "QualityReportV2",
                None,
                format!("quality_state={}", state.as_str()),
            ),
            Err(reason) => builder.undetermined(
                "QUALITY_REPORT_UNAVAILABLE",
                "QualityReportV2",
                None,
                reason,
            ),
        }
        let unresolved_answers = authoring
            .get("answerKey")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|answers| answers.iter())
            .filter(|(_, value)| value.get("kind").and_then(Value::as_str) == Some("unresolved"))
            .map(|(slot_id, _)| slot_id.clone())
            .collect::<Vec<_>>();
        for slot_id in unresolved_answers {
            builder.block(
                "ANSWER_MISSING",
                "AnswerKey",
                uncoded_target(&slot_id),
                format!("{slot_id} 还没有答案"),
            );
        }
        for issue in unresolved_blocking_issues(authoring) {
            builder.block(
                "ISSUE_UNRESOLVED",
                "QualityReportV2",
                issue
                    .get("targetId")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                issue
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("这道题有需要处理的问题。")
                    .to_string(),
            );
        }
        // 编译链：V2 的唯一编译入口。编译不过就不是"能不能发"的问题，
        // 而是根本没有可发布产物。目标契约按稿件自己的 modality 命名——听力卷
        // 编译进 `ListeningExamSourceV1`，报成阅读契约会让用户按错误的契约去排查。
        if let Ok(typed) = serde_json::from_value::<crate::schema::IeltsAuthoringIRV2>(
            authoring.clone(),
        ) {
            let schema_version =
                crate::listening_source_v1::runtime_schema_version(&typed.modality);
            if let Err(issues) = crate::listening_source_v1::compile_exam_source_v2(&typed) {
                builder.block(
                    "COMPILE_FAILED",
                    schema_version,
                    None,
                    serde_json::to_string(&issues).unwrap_or_default(),
                );
            }
        }
    } else {
        // V1 家族：契约 + 题号连续性 + display 唯一性 + 权威性重审，
        // 全部走既有唯一实现，再把它的 error 逐条升级成阻断项。
        let report = validate_authoring(job_id, Some(authoring));
        for issue in report
            .get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|issue| is_error_issue(issue))
        {
            builder.block(
                "VALIDATION_ERROR",
                "ReadingExamSourceV1",
                issue
                    .get("path")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                issue
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("校验未通过")
                    .to_string(),
            );
        }
        for issue in crate::authoring_review::authoring_review_issues(authoring) {
            builder.block(
                "AUTHORING_REVIEW_ISSUE",
                "AuthoringIR",
                issue
                    .get("path")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                issue
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("来源信息不完整")
                    .to_string(),
            );
        }
    }

    // ── 以下两项是"文件状态类"判据，只在 FullDerived 范围适用 ──────────────
    // canonical 直通时派生文件不是事实源（既有设计，见 `authoring_v2_commands.rs`
    // 的 M1 typed-preflight 直通注释）。这不是"放行"，而是判据集合不同；
    // 两份范围各自的判据都会跑满。范围差异已在 NOTES 里逐条列出。
    if scope == PublishScope::FullDerived {
        if authoring
            .pointer("/audit/humanVerified")
            .and_then(Value::as_bool)
            != Some(true)
        {
            builder.block(
                "HUMAN_VERIFICATION_REQUIRED",
                "AuthoringIR",
                Some("$.audit.humanVerified".to_string()),
                "All questions must be human verified before publish",
            );
        }
        if v2 {
            // 稿本身必须就是这道题。canonical 直通走的是"按 item_id 取稿"，
            // 稿与 id 的一致性由取出方式保证；文件派生来源没有这层保证，必须自己查。
            if authoring.get("jobId").and_then(Value::as_str) != Some(job_id) {
                builder.block(
                    "JOB_ID_MISMATCH",
                    "AuthoringIR",
                    Some("$.jobId".to_string()),
                    format!(
                        "authoring jobId={:?} does not match requested job {}",
                        authoring.get("jobId").and_then(Value::as_str),
                        job_id
                    ),
                );
            }
            // 历史痕迹（AI fallback / 部分失败标记）。这些是**文件**上的证据，
            // canonical 直通时文件不是事实源，因此只在 FullDerived 范围适用。
            let mut ai_fallbacks = Vec::new();
            let mut partial_failures = Vec::new();
            crate::authoring_v2_commands::collect_publish_gate_markers(
                authoring,
                "authoring",
                &mut ai_fallbacks,
                &mut partial_failures,
            );
            for relative in ["authoring-ir.json", "pipeline-report.json"] {
                let path = crate::util::job_dir(root, job_id).join(relative);
                if let Ok(Some(value)) = crate::util::read_json_opt(&path) {
                    crate::authoring_v2_commands::collect_publish_gate_markers(
                        &value,
                        relative,
                        &mut ai_fallbacks,
                        &mut partial_failures,
                    );
                }
            }
            if !ai_fallbacks.is_empty() {
                builder.block(
                    "AI_FALLBACK",
                    "PublishGate",
                    None,
                    ai_fallbacks.join(","),
                );
            }
            if !partial_failures.is_empty() {
                builder.block(
                    "PARTIAL_FAILURE",
                    "PublishGate",
                    None,
                    partial_failures.join(","),
                );
            }
        }
    }

    // 来源复核状态：两个范围都要（它是当前稿的事实，不是历史痕迹）。
    match crate::source_review::source_review_status_for_job(root, job_id) {
        Err(error) => builder.undetermined(
            "SOURCE_REVIEW_UNAVAILABLE",
            "SourceReview",
            None,
            error,
        ),
        Ok(source_review) => {
            if scope == PublishScope::FullDerived {
                if source_review.get("schemaVersion").and_then(Value::as_str)
                    != Some("SourceReviewV1")
                    || source_review.get("jobId").and_then(Value::as_str) != Some(job_id)
                {
                    builder.block(
                        "SOURCE_REVIEW_INVALID",
                        "SourceReview",
                        None,
                        "source review status is not a SourceReviewV1 for this job",
                    );
                }
                if source_review.get("stale").and_then(Value::as_bool) == Some(true) {
                    builder.block(
                        "SOURCE_REVIEW_STALE",
                        "SourceReview",
                        None,
                        "source review is stale: the document changed after it was reviewed",
                    );
                }
                if source_review.get("resolved").and_then(Value::as_bool) != Some(true) {
                    builder.block(
                        "SOURCE_REVIEW_UNRESOLVED",
                        "SourceReview",
                        None,
                        "source review is not resolved",
                    );
                }
            }
            for issue in crate::source_review::source_review_issues(&source_review) {
                builder.block(
                    "SOURCE_REVIEW_ISSUE",
                    "SourceReview",
                    issue
                        .get("path")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    issue
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("来源复核未完成")
                        .to_string(),
                );
            }
        }
    }

    // ── 必判的对等校验器：**没能执行 ≠ 通过** ────────────────────────────
    // 只有被显式要求（`EPIC8_NODE_VALIDATOR_DIAGNOSTICS` 为真）时它才是必判项；
    // 未启用时不属于判据集合，不产生 Undetermined。启用后一旦脚本缺失 / spawn 失败 /
    // 产物畸形 / 退出码与结论矛盾，结论必须是 Undetermined —— 修前那条路径只留一条
    // 不阻塞的 warning，等于把"没能执行"当成"通过"。
    if crate::environment::node_validator_diagnostics_enabled() {
        let source = reading_source(authoring);
        match crate::runtime_validation::validate_with_node_sidecar(root, job_id, &source) {
            Ok(_) => {}
            Err(error) => builder.undetermined(
                "VALIDATOR_UNAVAILABLE",
                "ReadingExamSourceV1",
                None,
                format!(
                    "Node validator was required (EPIC8_NODE_VALIDATOR_DIAGNOSTICS) but did not produce a usable report: {error}"
                ),
            ),
        }
    }

    builder.finish(edit_version)
}

/// **跨路径版本漂移检出**（NOTES §5.3 / §5.4）。
///
/// 预检读 canonical（DB 权威稿 + `current_edit_version`），而预览与遗留导出读
/// `authoring-ir.json` **文件**；DB 编辑从不重写该文件（见 `library/repository.rs` 与
/// `authoring_v2_commands.rs` 的既有记录）⇒ 一个被 V2 编辑过的 job，其文件可能陈旧
/// 甚至缺失，"校验的那份稿"和"预检看到的那份稿"可以不是同一份，**且全程无版本检查**。
///
/// 本轮**只加检出，不做同步修复**，也**不阻塞**：
///
/// - 不做同步：让三条路径都按版本读同一份 canonical 是独立一轮的工作量
///   —— 它需要「按版本 N 读取可发布文档」的单一入口，而这个入口当前不存在（§5.5）。
/// - 不阻塞：阻塞等价于**强制同步** —— 现网所有「被 V2 编辑过的 job 走遗留导出」
///   会立刻失败，那是产品行为变更，得单独决定，不能夹在一次收敛里顺手做掉。
///
/// 因此这里只把"两份稿是不是同一份"变成**可观察**的：结论如实写进产物
/// （`versionAlignment`）。"查不了"与"查了且一致"必须可区分 —— 所以有三态：
/// `checked:false` / `aligned:true` / `aligned:false`，而不是一个布尔。
pub(crate) fn version_alignment(root: &Path, job_id: &str, used: &Value) -> Value {
    let connection = match crate::library::repository::open_library_connection(root) {
        Ok(connection) => connection,
        Err(error) => {
            return json!({
                "schemaVersion": "VersionAlignmentV1",
                "checked": false,
                "reason": "library_unavailable",
                "detail": error,
            });
        }
    };
    match crate::library::repository::get_canonical_ds(&connection, job_id) {
        Err(error) => json!({
            "schemaVersion": "VersionAlignmentV1",
            "checked": false,
            "reason": "canonical_read_failed",
            "detail": error,
        }),
        Ok(None) => json!({
            "schemaVersion": "VersionAlignmentV1",
            "checked": true,
            "source": "derived_file",
            "canonicalPresent": false,
            "aligned": Value::Null,
            "note": "this item has no canonical document; the derived file is the only source that exists, so there is nothing to drift from",
        }),
        Ok(Some((canonical, canonical_edit_version))) => {
            let aligned = canonical == *used;
            json!({
                "schemaVersion": "VersionAlignmentV1",
                "checked": true,
                "source": "derived_file",
                "canonicalPresent": true,
                "canonicalEditVersion": canonical_edit_version,
                "aligned": aligned,
                "note": if aligned {
                    "the derived file equals the canonical document; no cross-path drift observed"
                } else {
                    "the derived file differs from the canonical document: the preflight (reads canonical) and this path (reads the file) are not looking at the same document. Detection only - this does not block the export in this round."
                },
            })
        }
    }
}

/// 把 [`version_alignment`] 的结论写进产物。只新增一个字段，不改任何既有字段。
pub(crate) fn record_version_alignment(report: &mut Value, root: &Path, job_id: &str, used: &Value) {
    let alignment = version_alignment(root, job_id, used);
    if let Some(object) = report.as_object_mut() {
        object.insert("versionAlignment".to_string(), alignment);
    }
}

/// 把结论写进一份 `ValidationReportV1`，让**产物与结论不可能互相矛盾**。
///
/// 只改两个字段，不动 `issues` / `layers` 的既有内容：
/// - `passed` 改为由结论派生（修前 `report.passed` 自己就是第四个判据来源）；
/// - 追加 `publishVerdict`（含全部 reasons），使"为什么不能发"在产物里可查。
pub(crate) fn apply_publish_verdict(report: &mut Value, verdict: &PublishVerdict, scope: PublishScope) {
    let Some(object) = report.as_object_mut() else {
        return;
    };
    object.insert("passed".to_string(), json!(verdict.is_ready()));
    object.insert("publishVerdict".to_string(), verdict.to_value());
    object.insert(
        "publishScope".to_string(),
        json!(match scope {
            PublishScope::CanonicalDirect => "canonical-direct",
            PublishScope::FullDerived => "full-derived",
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error_issue(layer: &str, message: &str) -> Value {
        json!({
            "issueId": format!("issue-{}", message),
            "severity": "error",
            "layer": layer,
            "path": "$.probe",
            "message": message
        })
    }

    fn base_report_with_two_errors() -> Value {
        json!({
            "jobId": "job-sidecar-overlay",
            "passed": false,
            "layers": [],
            "issues": [
                error_issue("ReadingExamSourceV1", "rust says q14 is missing"),
                error_issue("DomProtocol", "rust says dropzone is broken")
            ]
        })
    }

    fn issue_count(report: &Value) -> usize {
        report
            .get("issues")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0)
    }

    fn verdict_semantics(verdict: &Value) -> Value {
        json!({
            "status": verdict.get("status"),
            "ready": verdict.get("ready"),
            "reasons": verdict.get("reasons")
        })
    }

    #[test]
    fn publish_verdict_is_equivalent_across_preflight_two_legacy_exports_and_publish() {
        let root = std::env::temp_dir().join(format!(
            "publish-verdict-equivalence-{}",
            uuid::Uuid::new_v4().simple()
        ));
        crate::util::ensure_app_dirs(&root).unwrap();
        let job_id = "early-approaches-architecture-proof";
        crate::util::ensure_job_dirs(&crate::util::job_dir(&root, job_id)).unwrap();
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json");
        let mut ready: Value = crate::util::read_json(&fixture_path).unwrap();
        // The standalone golden intentionally records a missing physical shadow.  For this
        // equivalence test the manuscript itself is made ready so all four entrances exercise
        // the verdict rather than the fixture's unrelated coverage note.
        ready["quality"]["state"] = json!("ready");
        ready["quality"]["sourceCoverage"] = json!(1.0);
        ready["quality"]["hardFailures"] = json!([]);
        ready["quality"]["issues"] = json!([]);
        ready["audit"]["humanVerified"] = json!(true);

        let preflight = crate::authoring_v2_commands::check_publish_preflight(
            &root,
            job_id,
            7,
            &ready,
        )
        .get("publishVerdict")
        .cloned()
        .expect("preflight must expose the shared verdict");
        // These are the exact helper calls used by reading-assets and reading-js respectively.
        // They are invoked separately here to pin both export entrances to the same semantics.
        let assets = crate::export_pack::legacy_export_verdict(&root, job_id, &ready).to_value();
        let javascript = crate::export_pack::legacy_export_verdict(&root, job_id, &ready).to_value();
        let publish = publish_verdict(
            &root,
            job_id,
            &ready,
            Some(7),
            PublishScope::CanonicalDirect,
        )
        .to_value();
        let ready_entries = [&preflight, &assets, &javascript, &publish];
        for entry in ready_entries {
            assert_eq!(
                verdict_semantics(entry),
                verdict_semantics(&preflight),
                "all ready entrances must share one semantic verdict: {entry}"
            );
        }
        assert_eq!(preflight["status"], json!("ready"));

        // Counterexample for the blocked branch: an unresolved answer must stop every entrance,
        // not just the UI preflight.
        let mut blocked = ready.clone();
        blocked["answerKey"]["q14"] = json!({"kind": "unresolved"});
        let blocked_preflight = crate::authoring_v2_commands::check_publish_preflight(
            &root,
            job_id,
            8,
            &blocked,
        )
        .get("publishVerdict")
        .cloned()
        .expect("blocked preflight must expose the shared verdict");
        let blocked_assets = crate::export_pack::legacy_export_verdict(&root, job_id, &blocked).to_value();
        let blocked_javascript = crate::export_pack::legacy_export_verdict(&root, job_id, &blocked).to_value();
        let blocked_publish = publish_verdict(
            &root,
            job_id,
            &blocked,
            Some(8),
            PublishScope::CanonicalDirect,
        )
        .to_value();
        let blocked_entries = [
            &blocked_preflight,
            &blocked_assets,
            &blocked_javascript,
            &blocked_publish,
        ];
        for entry in blocked_entries {
            assert_eq!(entry["status"], json!("blocked"), "{entry}");
            assert_eq!(entry["ready"], json!(false), "{entry}");
            assert!(
                entry["reasons"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|reason| reason["code"] == json!("ANSWER_MISSING")),
                "each entrance must expose the unresolved answer blocker: {entry}"
            );
            assert_eq!(
                verdict_semantics(entry),
                verdict_semantics(&blocked_preflight),
                "blocked entrances must share one semantic verdict: {entry}"
            );
        }

        let _ = std::fs::remove_dir_all(root);
    }

    /// 反例 ②（#14）：预检读 canonical（DB 权威稿），而预览 / 遗留导出读
    /// `authoring-ir.json` **文件**；DB 编辑从不重写该文件 ⇒ 两条路径校验的**不是同一份稿**，
    /// 而全程没有任何版本检查（NOTES §5.4 (c)）。
    ///
    /// 这里造出该状态：库里版本 3 的权威稿与文件内容不同 ⇒ 检出必须报 `aligned:false`
    /// 并带上 `canonicalEditVersion`，而不是默默放行。
    /// 本轮口径是**只检出、不同步修复、不阻塞**（理由见 `version_alignment` 的文档）。
    #[test]
    fn version_alignment_detects_the_file_source_diverging_from_canonical() {
        let root = std::env::temp_dir().join(format!(
            "authoring-validation-version-alignment-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let job_id = "alignment-job";
        let canonical = json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "jobId": job_id,
            "marker": "canonical"
        });
        let connection = crate::library::repository::open_library_connection(&root).unwrap();
        connection
            .execute(
                "INSERT INTO library_items_v2 (id, modality, title, status, current_edit_version, canonical_ds_json, source_asset_id, created_at, updated_at, deleted_at)
                 VALUES (?1, 'reading', 'Alignment', 'ready', 3, ?2, NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL)",
                rusqlite::params![job_id, canonical.to_string()],
            )
            .unwrap();
        drop(connection);

        let stale_file = json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "jobId": job_id,
            "marker": "stale-file"
        });
        let drift = version_alignment(&root, job_id, &stale_file);
        assert_eq!(drift.get("checked").and_then(Value::as_bool), Some(true));
        assert_eq!(
            drift.get("canonicalEditVersion").and_then(Value::as_i64),
            Some(3),
            "{drift}"
        );
        assert_eq!(
            drift.get("aligned").and_then(Value::as_bool),
            Some(false),
            "文件与 canonical 不同必须被检出：{drift}"
        );

        // 同一份稿时不得误报 —— 否则这个字段一上线就是噪音。
        let aligned = version_alignment(&root, job_id, &canonical);
        assert_eq!(
            aligned.get("aligned").and_then(Value::as_bool),
            Some(true),
            "{aligned}"
        );

        // 库里没有权威稿 ⇒ 如实说"没有可漂移的对象"，而不是含糊地报成"对齐"。
        let absent = version_alignment(&root, "no-such-item", &stale_file);
        assert_eq!(
            absent.get("canonicalPresent").and_then(Value::as_bool),
            Some(false),
            "{absent}"
        );
        assert!(
            absent.get("aligned").map(Value::is_null).unwrap_or(false),
            "「没有权威稿」不等于「一致」：{absent}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    /// 反例 ①：空侧车输出（无 `layers`、无 `issues`、无 `replaceExistingLayers`）。
    ///
    /// 修前：`layers_to_replace` 退化为 `["ReadingExamSourceV1","DomProtocol"]`，
    /// `replaceExistingLayers` 缺省 true ⇒ 两条 Rust error 被删掉，`passed` 翻成 true。
    /// 修后：缺省是**合并**，Rust 的两条 error 必须原样保留。
    #[test]
    fn sidecar_report_without_layers_must_not_erase_rust_errors() {
        let mut base = base_report_with_two_errors();
        merge_sidecar_validation(&mut base, json!({}));

        assert_eq!(
            base.get("passed").and_then(Value::as_bool),
            Some(false),
            "空的侧车输出不得把 Rust 自查的 error 抹掉"
        );
        assert_eq!(
            issue_count(&base),
            2,
            "两层的 Rust error 必须全部保留：{}",
            serde_json::to_string_pretty(&base).unwrap_or_default()
        );
    }

    /// 反例 ②：侧车只报自己的 layer、不比 base 少一个字段，但 base 有 DomProtocol error。
    #[test]
    fn sidecar_issues_are_merged_not_substituted() {
        let mut base = base_report_with_two_errors();
        merge_sidecar_validation(
            &mut base,
            json!({
                "layers": [{"layer": "ReadingExamSourceV1", "passed": true, "issueCount": 0}],
                "issues": []
            }),
        );

        assert_eq!(base.get("passed").and_then(Value::as_bool), Some(false));
        assert_eq!(issue_count(&base), 2);
    }

    /// 反向：显式要求替换时，仍允许替换（能力保留，但必须显式）。
    #[test]
    fn explicit_replace_existing_layers_still_replaces() {
        let mut base = base_report_with_two_errors();
        merge_sidecar_validation(
            &mut base,
            json!({
                "layers": [{"layer": "ReadingExamSourceV1", "passed": true, "issueCount": 0}],
                "issues": [],
                "replaceExistingLayers": true
            }),
        );

        assert_eq!(
            base.get("passed").and_then(Value::as_bool),
            Some(false),
            "DomProtocol 未被列入替换范围，它的 error 必须留下"
        );
        assert_eq!(issue_count(&base), 1);
    }

    /// 侧车自带 warning 时必须并进来（不能被替换语义吞掉）。
    #[test]
    fn sidecar_warnings_are_preserved_in_merge() {
        let mut base = json!({
            "jobId": "job-sidecar-warning",
            "passed": true,
            "layers": [],
            "issues": []
        });
        merge_sidecar_validation(
            &mut base,
            json!({
                "issues": [{
                    "issueId": "issue-w",
                    "severity": "warning",
                    "layer": "ReadingExamSourceV1",
                    "path": "$.probe",
                    "message": "node parity warning"
                }]
            }),
        );

        assert_eq!(issue_count(&base), 1);
        assert_eq!(base.get("passed").and_then(Value::as_bool), Some(true));
    }
}
