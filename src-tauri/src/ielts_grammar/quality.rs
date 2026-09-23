use chrono::Utc;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

use crate::environment::recognition_blockers_gate_enabled;
use crate::reading_source::ReadingExamSourceV1;
use crate::reading_source_v2::CompilerIssueV2;
use crate::schema::IeltsAuthoringIRV2;
use crate::schema::ielts_authoring_v2::QuestionNumberExpressionV2;
use crate::validator::validate_reading_source_contract;

use super::instruction_signature::infer_instruction_signature;
use super::instruction_zone::semantic_lines_from_v2_shadow;
use super::issue_codes::*;
use super::listening_parts::detect_listening_parts;
use super::source_coverage::{assess as assess_source_question_coverage, QuestionCoverageStatus};

#[derive(Debug, Clone, Default)]
struct GroupEvaluation {
    score: f64,
}

#[derive(Debug, Clone, Default)]
struct SourceCoverageSummary {
    score: f64,
    significant_count: usize,
    assigned_count: usize,
    unassigned_ids: Vec<String>,
    ledger: Vec<Value>,
    physical_available: bool,
}

/// 发布时冻结的原文证据（「题库保存」：发布后原文件与过程文件被删除）。
///
/// 只有两类结论会被冻结：原文声明的题号集合（之后每次评估仍与当前稿逐题核对），
/// 以及节点覆盖在发布时的结论（原文件已不存在，无法重算，只能如实标注
/// `verified_at_publish_source_purged`）。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FrozenSourceEvidence {
    pub declared_question_numbers: Vec<u32>,
    pub declarations: Vec<String>,
    pub node_score: f64,
    pub node_complete: bool,
    pub significant_count: usize,
    pub explained_count: usize,
}

impl FrozenSourceEvidence {
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        if value.get("schemaVersion").and_then(Value::as_str) != Some("PublishSourceEvidenceV1") {
            return None;
        }
        let numbers = value
            .get("declaredQuestionNumbers")?
            .as_array()?
            .iter()
            .map(|number| number.as_u64().and_then(|number| u32::try_from(number).ok()))
            .collect::<Option<Vec<_>>>()?;
        let declarations = value
            .get("declarations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect();
        let node = value.get("nodeCoverage")?;
        Some(Self {
            declared_question_numbers: numbers,
            declarations,
            node_score: node.get("score").and_then(Value::as_f64)?,
            node_complete: node.get("complete").and_then(Value::as_bool)?,
            significant_count: node
                .get("significantSourceNodeCount")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
            explained_count: node
                .get("explainedSourceNodeCount")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
        })
    }
}

/// 发布成功时要冻结的证据摘要（只读计算，不改稿）。
///
/// 题号声明只有在发布时能可靠解析（非 Undetermined）才冻结；否则冻结空集合，
/// 之后仍报 Undetermined，而不是被原文件删除「洗」成完整。
pub(crate) fn publish_evidence_summary(authoring: &Value, physical_shadow: Option<&Value>) -> Value {
    let assessment = assess_source_question_coverage(authoring, physical_shadow);
    let summary = source_coverage_summary(authoring, physical_shadow);
    let reliable = assessment.status != QuestionCoverageStatus::Undetermined;
    json!({
        "schemaVersion": "PublishSourceEvidenceV1",
        "questionCoverage": assessment.as_value(),
        "declaredQuestionNumbers": if reliable { assessment.declared_question_numbers.clone() } else { Vec::new() },
        "declarations": if reliable { assessment.declarations.clone() } else { Vec::new() },
        "nodeCoverage": {
            "physicalShadow": if summary.physical_available { "available" } else { "missing" },
            "score": round(summary.score),
            "complete": summary.physical_available && summary.unassigned_ids.is_empty(),
            "significantSourceNodeCount": summary.significant_count,
            "explainedSourceNodeCount": summary.assigned_count,
            "unassignedSourceNodeIds": summary.unassigned_ids
        }
    })
}

/// 一个 `extractionMode == "user_upload"` 资产的核对结论。
///
/// 用户上传的听力音频**不在** PDF 的 physical shadow 里——它根本没进过 PDF。拿它去和
/// shadow 比对必然对不上，那是把「用户上传的资产」当成「PDF 里抽出来的资产」。它的权威
/// 依据是受管音频台账 `listening_audio_assets_v1` 加上磁盘上的那份文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedAudioCheckV1 {
    /// 台账有行、受管文件在、sha256 与资产声明一致、探测通过。四条全中才是它。
    Verified,
    /// 台账里没有这个 assetId 的行。
    NoRecord,
    /// 台账有行，但受管文件不在磁盘上。
    FileMissing,
    /// 受管文件的内容哈希与声明不一致（换过文件、或文件被改过）。
    HashMismatch,
    /// 探测没有通过（解码失败 / 近静音 / 编码不支持……）。
    ProbeBlocked,
}

/// 受管音频核对事实：由 `listening_audio` 读台账 + 文件系统产出，质量门禁只消费。
///
/// 刻意做成**纯数据**：门禁不开连接、不碰文件系统（`evaluate_quality` 能在纯单测里跑
/// 全靠这一点），IO 留给唯一的产出方。**缺项按 [`ManagedAudioCheckV1::NoRecord`] 处理**
/// ——「查不到」不等于「没问题」。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ManagedAudioFactsV1 {
    checks: BTreeMap<String, ManagedAudioCheckV1>,
}

impl ManagedAudioFactsV1 {
    pub(crate) fn new(checks: BTreeMap<String, ManagedAudioCheckV1>) -> Self {
        Self { checks }
    }

    /// 一个 assetId 的结论。没记过的资产一律 `NoRecord`，绝不默认通过。
    pub(crate) fn check(&self, asset_id: &str) -> ManagedAudioCheckV1 {
        self.checks
            .get(asset_id)
            .copied()
            .unwrap_or(ManagedAudioCheckV1::NoRecord)
    }
}

/// `user_upload` 资产的核对结论 → 硬失败码。`Verified` 没有码（它不阻断）。
fn managed_audio_failure_code(check: ManagedAudioCheckV1) -> Option<&'static str> {
    match check {
        ManagedAudioCheckV1::Verified => None,
        ManagedAudioCheckV1::NoRecord | ManagedAudioCheckV1::FileMissing => {
            Some(ASSET_REFERENCE_MISSING)
        }
        ManagedAudioCheckV1::HashMismatch => Some(ASSET_HASH_MISMATCH),
        ManagedAudioCheckV1::ProbeBlocked => Some(LISTENING_AUDIO_PROBE_BLOCKED),
    }
}

/// 原文件已在发布后删除的条目：题号覆盖按冻结声明核对，节点覆盖报告为
/// `verified_at_publish_source_purged`（非阻断），不再因缺 shadow 报 0.0。
pub(crate) fn evaluate_quality_with_frozen_evidence(
    authoring: &Value,
    frozen: &FrozenSourceEvidence,
) -> Value {
    evaluate_quality_with_evidence(authoring, None, Some(frozen), None)
}

/// 同上，但把受管音频事实一并带进来。已发布条目改档时影子证据在、音频也在，
/// 两个依据都得能用；只给冻结节证据会让一次「保存」把音频判成不存在。
pub(crate) fn evaluate_quality_with_frozen_evidence_and_managed_audio(
    authoring: &Value,
    frozen: &FrozenSourceEvidence,
    managed_audio: Option<&ManagedAudioFactsV1>,
) -> Value {
    evaluate_quality_with_evidence(authoring, None, Some(frozen), managed_audio)
}

pub(crate) fn evaluate_quality(authoring: &Value, physical_shadow: Option<&Value>) -> Value {
    evaluate_quality_with_gate(
        authoring,
        physical_shadow,
        recognition_blockers_gate_enabled(),
    )
}

/// `evaluate_quality`，并带上受管音频核对事实。
///
/// `managed_audio == None` 且稿里**确实**有 `user_upload` 资产时按不通过处理（见
/// `validate_assets`）：读不到台账就不能声称音频没问题。稿里没有这类资产时两边等价。
pub(crate) fn evaluate_quality_with_managed_audio(
    authoring: &Value,
    physical_shadow: Option<&Value>,
    managed_audio: Option<&ManagedAudioFactsV1>,
) -> Value {
    evaluate_quality_with_evidence(authoring, physical_shadow, None, managed_audio)
}

/// 唯一实现：物理影子 / 冻结证据 / 受管音频事实三路依据在这里合流。
fn evaluate_quality_with_evidence(
    authoring: &Value,
    physical_shadow: Option<&Value>,
    frozen: Option<&FrozenSourceEvidence>,
    managed_audio: Option<&ManagedAudioFactsV1>,
) -> Value {
    evaluate_quality_inner(
        authoring,
        physical_shadow,
        frozen,
        managed_audio,
        recognition_blockers_gate_enabled(),
    )
}

/// 质量就绪度的**唯一**三态。
///
/// 与 `schema::quality_report_v2::ReadinessStateV2` 同形，但它是**判据类型**而不是
/// 序列化契约：这样发布判据不必依赖一个可以被人改写的存储字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QualityReadiness {
    Ready,
    ReviewRequired,
    Blocked,
}

impl QualityReadiness {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::ReviewRequired => "review_required",
            Self::Blocked => "blocked",
        }
    }
}

/// 就绪度规则的**唯一实现**。阈值集中在这里，改一次就改全部。
///
/// - `hardFailures` 非空 ⇒ `Blocked`
/// - `documentScore < 0.95`、任一分组 `taskScore < 0.92`、`sourceCoverage < 0.995`、
///   或存在未处理的 blocking issue ⇒ `ReviewRequired`
/// - 否则 `Ready`
pub(crate) fn readiness_from_facts(
    hard_failures: &[String],
    document_score: f64,
    has_low_task_score: bool,
    source_coverage: f64,
    unresolved_blocking_issue_count: usize,
) -> QualityReadiness {
    if !hard_failures.is_empty() {
        return QualityReadiness::Blocked;
    }
    if document_score < 0.95
        || has_low_task_score
        || source_coverage < 0.995
        || unresolved_blocking_issue_count > 0
    {
        return QualityReadiness::ReviewRequired;
    }
    QualityReadiness::Ready
}

fn severity_label(severity: &crate::schema::quality_report_v2::ReviewSeverityV2) -> &'static str {
    use crate::schema::quality_report_v2::ReviewSeverityV2;
    match severity {
        ReviewSeverityV2::Blocking => "blocking",
        ReviewSeverityV2::Warning => "warning",
        ReviewSeverityV2::Info => "info",
    }
}

/// 从当前稿的原始事实重新得到就绪度，套用**同一条**规则。
///
/// 刻意**不读** `quality.state`：存储值可能陈旧，也可能被手改成 `ready`。
/// 判据必须由当前稿重新算出。
///
/// `Err` 表示**无法判定**（quality 块缺失，或不是合法的 `QualityReportV2`）。
/// 调用方必须把它当成"没能判定"，**不得**当成通过 —— 这正是修前缺的那一态。
pub(crate) fn quality_readiness(authoring: &Value) -> Result<QualityReadiness, String> {
    use crate::schema::quality_report_v2::QualityReportV2;
    let quality = authoring
        .get("quality")
        .cloned()
        .ok_or("AUTHORING_SCHEMA_INVALID:quality_missing")?;
    let report: QualityReportV2 = serde_json::from_value(quality)
        .map_err(|error| format!("AUTHORING_SCHEMA_INVALID:quality:{error}"))?;
    let unresolved_blocking_issue_count = report
        .issues
        .iter()
        .filter(|issue| {
            let details = issue
                .details
                .as_ref()
                .map(|details| Value::Object(details.clone().into_iter().collect()));
            crate::authoring_validation::blocking_issue_unresolved_parts(
                severity_label(&issue.severity),
                details.as_ref(),
            )
        })
        .count();
    Ok(readiness_from_facts(
        &report.hard_failures,
        report.document_score,
        report.task_scores.values().any(|score| *score < 0.92),
        effective_source_coverage(
            report.source_coverage,
            report.coverage_status.physical_shadow
                == crate::schema::quality_report_v2::PhysicalShadowStatusV2::VerifiedAtPublishSourcePurged,
        ),
        unresolved_blocking_issue_count,
    ))
}

/// 节点覆盖在原文件被删除后不再参与就绪度（`verified_at_publish_source_purged`）。
/// 只有 `evaluate_quality_inner` 能写出这个状态，且只在条目确实被清理、且真的没有
/// physical shadow 时；非清理条目缺 shadow 仍是 `missing` 并照旧阻断。
fn effective_source_coverage(source_coverage: f64, source_purged: bool) -> f64 {
    if source_purged {
        1.0
    } else {
        source_coverage
    }
}

/// `evaluate_quality` with the §6.8/§6.11 recognition gate passed explicitly.
///
/// The staging decision is an environment flag in production, but tests must not
/// mutate process-global state, so the decision is a parameter here and the
/// production wrapper supplies the flag.
pub(crate) fn evaluate_quality_with_gate(
    authoring: &Value,
    physical_shadow: Option<&Value>,
    recognition_gate_enabled: bool,
) -> Value {
    evaluate_quality_inner(
        authoring,
        physical_shadow,
        None,
        None,
        recognition_gate_enabled,
    )
}

fn evaluate_quality_inner(
    authoring: &Value,
    physical_shadow: Option<&Value>,
    frozen: Option<&FrozenSourceEvidence>,
    managed_audio: Option<&ManagedAudioFactsV1>,
    recognition_gate_enabled: bool,
) -> Value {
    // 冻结证据只在「确实没有 physical shadow」时生效；有 shadow 就按事实重算。
    let frozen = frozen.filter(|_| physical_shadow.is_none());
    let groups = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let slots = authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let answer_key = authoring
        .get("answerKey")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut issues = Vec::new();
    let mut hard_failures = Vec::new();
    let mut task_scores = BTreeMap::new();
    let mut expected_numbers = BTreeSet::new();
    let mut actual_numbers = Vec::new();

    // Independent view of the question domain: it comes from the raw physical
    // source, never from task groups or cloud output.  Listening uses the same
    // document-level check (its declared `Questions a-b` ranges are read the same
    // way) and adds the per-part view in `validate_listening_parts`.
    let question_coverage = Some(match frozen {
        Some(frozen) => super::source_coverage::assess_against_frozen_declaration(
            authoring,
            &frozen.declared_question_numbers,
            &frozen.declarations,
        ),
        None => assess_source_question_coverage(authoring, physical_shadow),
    });

    if let Some(assessment) = question_coverage.as_ref() {
        match assessment.status {
            QuestionCoverageStatus::Missing => {
                let mut coverage_issue = issue(
                    SOURCE_QUESTION_COVERAGE_MISSING,
                    "blocking",
                    "原文声明的题号集合与当前稿不一致，可能有题目被漏掉。",
                    "document",
                    "document",
                    Vec::new(),
                    vec!["split_prompt", "edit_text"],
                );
                coverage_issue["details"] = assessment.as_value();
                push_issue(&mut issues, &mut hard_failures, coverage_issue);
            }
            QuestionCoverageStatus::Undetermined => {
                let mut coverage_issue = issue(
                    SOURCE_QUESTION_COVERAGE_UNDETERMINED,
                    "warning",
                    "无法可靠解析原文声明的题号范围，未把 source coverage 判为完整。",
                    "document",
                    "document",
                    Vec::new(),
                    vec!["assign_role", "edit_text"],
                );
                coverage_issue["details"] = assessment.as_value();
                push_issue(&mut issues, &mut hard_failures, coverage_issue);
            }
            QuestionCoverageStatus::Complete => {}
        }
    }

    validate_exam_id(authoring, &mut issues, &mut hard_failures);
    validate_passage(authoring, &mut issues, &mut hard_failures);
    validate_listening_parts(authoring, physical_shadow, &mut issues, &mut hard_failures);
    validate_listening_media(authoring, &mut issues, &mut hard_failures);
    validate_assets(
        authoring,
        physical_shadow,
        managed_audio,
        &mut issues,
        &mut hard_failures,
    );
    validate_identifier_and_reference_closure(authoring, &mut issues, &mut hard_failures);
    validate_provenance(authoring, &mut issues, &mut hard_failures);
    validate_scoring_semantics(authoring, &mut issues, &mut hard_failures);
    validate_source_ownership(authoring, physical_shadow, &mut issues, &mut hard_failures);
    validate_recognition_blockers(
        authoring,
        recognition_gate_enabled,
        &mut issues,
        &mut hard_failures,
    );

    if groups.is_empty() {
        push_issue(
            &mut issues,
            &mut hard_failures,
            issue(
                QUESTION_RANGE_UNPARSED,
                "blocking",
                "未找到可解析的 IELTS 题组范围。",
                "document",
                "document",
                Vec::new(),
                vec!["assign_role", "edit_text"],
            ),
        );
    }

    for group in &groups {
        let task_id = group
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or("unknown-task");
        let evaluation = evaluate_group(
            group,
            &slots,
            &answer_key,
            &mut expected_numbers,
            &mut actual_numbers,
            &mut issues,
            &mut hard_failures,
        );
        task_scores.insert(task_id.to_string(), round(evaluation.score));
    }

    let mut seen = BTreeSet::new();
    for number in actual_numbers {
        if !seen.insert(number) {
            push_issue(
                &mut issues,
                &mut hard_failures,
                issue(
                    QUESTION_NUMBER_DUPLICATE,
                    "blocking",
                    &format!("题号 {number} 被多个 slot 声明。"),
                    "document",
                    "document",
                    Vec::new(),
                    vec!["split_prompt", "edit_text"],
                ),
            );
        }
    }
    for expected in expected_numbers.iter().copied() {
        if !slots.values().any(|slot| {
            slot.get("questionNumber")
                .and_then(Value::as_u64)
                .is_some_and(|number| number as u32 == expected)
        }) {
            push_issue(
                &mut issues,
                &mut hard_failures,
                issue(
                    QUESTION_NUMBER_MISSING,
                    "blocking",
                    &format!("题组声明了题号 {expected}，但没有对应 AnswerSlot。"),
                    "document",
                    "document",
                    Vec::new(),
                    vec!["edit_text", "split_prompt"],
                ),
            );
        }
    }

    let source_summary = match frozen {
        Some(frozen) => SourceCoverageSummary {
            score: frozen.node_score,
            significant_count: frozen.significant_count,
            assigned_count: frozen.explained_count,
            unassigned_ids: Vec::new(),
            ledger: Vec::new(),
            physical_available: false,
        },
        None => source_coverage_summary(authoring, physical_shadow),
    };
    let source_coverage = source_summary.score;
    let source_purged = frozen.is_some();
    if source_purged {
        // 原文件已按「题库保存」规则删除：节点覆盖是发布时的结论，不能重算，
        // 也不再阻断（发布即确认）。题号覆盖仍按冻结声明逐题核对（见上）。
    } else if !source_summary.physical_available {
        push_issue(
            &mut issues,
            &mut hard_failures,
            issue(
                PHYSICAL_SHADOW_MISSING,
                "warning",
                "缺少 DocumentIRV2 physical shadow，不能证明显著源区域覆盖完整。",
                "document",
                "document",
                Vec::new(),
                vec!["assign_role"],
            ),
        );
    } else if !source_summary.unassigned_ids.is_empty() {
        let mut coverage_issue = issue(
            SIGNIFICANT_REGION_UNASSIGNED,
            "blocking",
            "仍有显著源区域未被题目、passage 或有理由的忽略记录解释。",
            "document",
            "document",
            Vec::new(),
            vec!["assign_role"],
        );
        coverage_issue["details"] = json!({
            "significantSourceNodeCount": source_summary.significant_count,
            "assignedSourceNodeCount": source_summary.assigned_count,
            "unassignedSourceNodeIds": &source_summary.unassigned_ids
        });
        push_issue(&mut issues, &mut hard_failures, coverage_issue);
    }

    let compiler_probes = evaluate_compiler_probes(authoring);
    append_compiler_probe_issue(
        &compiler_probes,
        "v2Runtime",
        RUNTIME_COMPILER_FAILED,
        "ReadingExamSourceV2 runtime compiler 或其 schema validation 失败。",
        &mut issues,
        &mut hard_failures,
    );
    append_compiler_probe_issue(
        &compiler_probes,
        "v1Compatibility",
        V1_COMPATIBILITY_COMPILER_FAILED,
        "V1 compatibility compiler 或 ReadingExamSourceV1 validation 失败。",
        &mut issues,
        &mut hard_failures,
    );

    let document_score = if task_scores.is_empty() {
        0.0
    } else {
        task_scores.values().sum::<f64>() / task_scores.len() as f64
    };
    // Only UNRESOLVED, BLOCKING issues hold readiness back. This mirrors the export gate's own
    // `unresolved_blockers` predicate exactly (authoring_v2_commands: severity == Blocking and no
    // resolution in {resolved, ignored}).
    //
    // The previous `!issues.is_empty()` disagreed with that gate in two ways that both blocked
    // good documents: a single `info` note made an otherwise-perfect paper unpublishable, and
    // resolving or ignoring an issue changed nothing -- so `preserve_issue_resolutions` went to the
    // trouble of carrying resolutions forward across every save that nothing ever read.
    //
    // 谓词本身只有一份实现（`authoring_validation::blocking_issue_unresolved`），
    // 规则本身也只有一份实现（下面的 `readiness_from_facts`）——
    // `state` 与发布判据（`authoring_validation::publish_verdict`）都从这里取。
    let unresolved_blocking_issue_count = issues
        .iter()
        .filter(|issue| crate::authoring_validation::blocking_issue_unresolved(issue))
        .count();
    let state = readiness_from_facts(
        &hard_failures,
        document_score,
        task_scores.values().any(|score| *score < 0.92),
        effective_source_coverage(source_coverage, source_purged),
        unresolved_blocking_issue_count,
    )
    .as_str();
    let mut report = json!({
        "schemaVersion": "QualityReportV2",
        "state": state,
        "documentScore": round(document_score),
        "sourceCoverage": round(source_coverage),
        "coverageLedger": source_summary.ledger,
        "coverageStatus": {
            "physicalShadow": if source_purged {
                "verified_at_publish_source_purged"
            } else if source_summary.physical_available {
                "available"
            } else {
                "missing"
            },
            "complete": match frozen {
                Some(frozen) => frozen.node_complete,
                None => source_summary.physical_available && source_summary.unassigned_ids.is_empty(),
            },
            "significantSourceNodeCount": source_summary.significant_count,
            "explainedSourceNodeCount": source_summary.assigned_count,
            "unassignedSourceNodeIds": source_summary.unassigned_ids
        },
        "compilerProbes": compiler_probes,
        "taskScores": task_scores,
        "hardFailures": hard_failures,
        "issues": issues,
        "metrics": {
            "taskCount": groups.len() as f64,
            "slotCount": slots.len() as f64,
            "expectedQuestionCount": expected_numbers.len() as f64,
            "sourceCoverage": round(source_coverage),
            "significantSourceNodeCount": source_summary.significant_count as f64,
            "assignedSourceNodeCount": source_summary.assigned_count as f64,
            "unassignedSourceNodeCount": source_summary.unassigned_ids.len() as f64
        },
        "evaluatedAt": Utc::now().to_rfc3339(),
        "evaluatorVersion": "phase4-pr07-hard-gate-v2"
    });
    if let Some(assessment) = question_coverage {
        report["questionCoverage"] = assessment.as_value();
    }
    report
}

fn validate_scoring_semantics(
    authoring: &Value,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let slots = authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for task in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for response in task
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let response_id = response
                .get("responseGroupId")
                .and_then(Value::as_str)
                .unwrap_or("unknown-response");
            let scoring_policy = response.get("scoringPolicy").and_then(Value::as_str);
            if !matches!(
                scoring_policy,
                Some(
                    "per_slot_binary"
                        | "per_slot_ielts_normalized"
                        | "exact_set"
                        | "all_or_nothing"
                )
            ) {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        SCORING_POLICY_UNRESOLVED,
                        "blocking",
                        "ResponseGroup 缺少可执行且明确的 scoring policy。",
                        "response_group",
                        response_id,
                        anchors_from(response),
                        vec!["edit_text"],
                    ),
                );
            }
            for slot_id in response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let participation = slots
                    .get(slot_id)
                    .and_then(|slot| slot.get("participation"))
                    .and_then(Value::as_str);
                let code = match participation {
                    Some("example") => Some(EXAMPLE_SCORING_CONFLICT),
                    Some("non_scoring") => Some(SCORING_POLICY_UNRESOLVED),
                    _ => None,
                };
                if let Some(code) = code {
                    push_issue(
                        issues,
                        hard_failures,
                        issue(
                            code,
                            "blocking",
                            if code == EXAMPLE_SCORING_CONFLICT {
                                "Example slot 被纳入当前计分 response group。"
                            } else {
                                "Non-scoring slot 缺少可证明的排除计分策略。"
                            },
                            "slot",
                            slot_id,
                            slots.get(slot_id).map(anchors_from).unwrap_or_default(),
                            vec!["edit_text"],
                        ),
                    );
                }
            }
        }
    }
}

fn validate_exam_id(authoring: &Value, issues: &mut Vec<Value>, hard_failures: &mut Vec<String>) {
    let exam_id = authoring
        .pointer("/exam/examId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !crate::util::is_safe_path_segment(exam_id) {
        push_issue(
            issues,
            hard_failures,
            issue(
                EXAM_ID_INVALID,
                "blocking",
                "examId 必须是非空、唯一可寻址且路径安全的 ASCII 标识。",
                "document",
                if exam_id.is_empty() {
                    "document"
                } else {
                    exam_id
                },
                Vec::new(),
                vec!["edit_text"],
            ),
        );
    }
}

/// Question numbers a task group covers, from its `displayRange`.
fn task_group_question_numbers(authoring: &Value, task_id: &str) -> Vec<u32> {
    let Some(groups) = authoring.get("taskGroups").and_then(Value::as_array) else {
        return Vec::new();
    };
    let Some(group) = groups
        .iter()
        .find(|group| group.get("taskId").and_then(Value::as_str) == Some(task_id))
    else {
        return Vec::new();
    };
    group
        .get("displayRange")
        .cloned()
        .and_then(|value| serde_json::from_value::<QuestionNumberExpressionV2>(value).ok())
        .map(|expression| super::question_number::expand_expression(&expression))
        .unwrap_or_default()
}

/// Listening-only, **per part**: the question domain the original paper declares
/// inside each section, compared with what the draft actually covers.
///
/// The document-level source coverage compares two sets; it cannot see *where* a
/// question was declared. A listening section whose declared numbers are not all
/// covered by its task groups is a gap no other check can attribute, so it gets
/// its own code and its own target. When the physical source is gone the check
/// cannot run at all and stays silent — it never claims the part is complete.
fn validate_listening_parts(
    authoring: &Value,
    physical_shadow: Option<&Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    if authoring.get("modality").and_then(Value::as_str) != Some("listening") {
        return;
    }
    let Some(physical) = physical_shadow else {
        return;
    };
    let lines = semantic_lines_from_v2_shadow(physical);
    if lines.is_empty() {
        return;
    }
    let texts = lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>();
    let detected = detect_listening_parts(&texts);
    if detected.parts.is_empty() {
        return;
    }
    let declared_parts = authoring
        .pointer("/listening/parts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for part in &detected.parts {
        let part_id = format!("part-{}", part.ordinal);
        let Some(draft_part) = declared_parts
            .iter()
            .find(|value| value.get("partId").and_then(Value::as_str) == Some(part_id.as_str()))
        else {
            push_issue(
                issues,
                hard_failures,
                issue(
                    LISTENING_PART_MISSING,
                    "blocking",
                    "原文存在这个听力部分，当前稿里没有对应的 part。",
                    "part",
                    &part_id,
                    Vec::new(),
                    vec!["split_prompt", "edit_text"],
                ),
            );
            continue;
        };
        let covered = draft_part
            .get("taskIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .flat_map(|task_id| task_group_question_numbers(authoring, task_id))
            .collect::<BTreeSet<u32>>();
        let missing = part
            .question_numbers
            .iter()
            .copied()
            .filter(|number| !covered.contains(number))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            continue;
        }
        let mut coverage_issue = issue(
            LISTENING_PART_COVERAGE_MISSING,
            "blocking",
            "原文在这一部分声明的题号，没有全部落到题组里，可能漏题。",
            "part",
            &part_id,
            Vec::new(),
            vec!["split_prompt", "edit_text"],
        );
        coverage_issue["details"] = json!({
            "missingQuestionNumbers": missing,
            "declaredQuestionNumbers": part.question_numbers,
        });
        push_issue(issues, hard_failures, coverage_issue);
    }
}

/// Listening-only: every part must carry audio that passed its probe.
///
/// Both codes are blocking: a listening paper without usable audio is not
/// publishable as a student-loadable exam. Publishing anyway is the user's
/// explicit one-click override (`published_forced`), which is a different,
/// recorded decision — not something this gate may silently allow.
fn validate_listening_media(
    authoring: &Value,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    if authoring.get("modality").and_then(Value::as_str) != Some("listening") {
        return;
    }
    let Some(parts) = authoring
        .pointer("/listening/parts")
        .and_then(Value::as_array)
    else {
        return;
    };
    for part in parts {
        let Some(part_id) = part.get("partId").and_then(Value::as_str) else {
            continue;
        };
        match part.get("media").filter(|value| !value.is_null()) {
            None => {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        LISTENING_AUDIO_MISSING,
                        "blocking",
                        "这个听力部分还没有绑定音频。",
                        "part",
                        part_id,
                        Vec::new(),
                        vec!["assign_role"],
                    ),
                );
            }
            Some(media) => {
                let probe = media.get("probe");
                let passed = probe
                    .and_then(|probe| probe.get("status"))
                    .and_then(Value::as_str)
                    == Some("passed")
                    && probe
                        .and_then(|probe| probe.get("issueCodes"))
                        .and_then(Value::as_array)
                        .is_some_and(|codes| codes.is_empty());
                if passed {
                    continue;
                }
                let mut audio_issue = issue(
                    LISTENING_AUDIO_PROBE_BLOCKED,
                    "blocking",
                    "这个听力部分的音频没有通过探测，播放或判分可能不可用。",
                    "part",
                    part_id,
                    Vec::new(),
                    vec!["assign_role"],
                );
                audio_issue["details"] = json!({
                    "assetId": media.get("assetId").cloned().unwrap_or(Value::Null),
                    "probe": probe.cloned().unwrap_or(Value::Null),
                });
                push_issue(issues, hard_failures, audio_issue);
            }
        }
    }
}

fn validate_passage(authoring: &Value, issues: &mut Vec<Value>, hard_failures: &mut Vec<String>) {    if authoring.get("modality").and_then(Value::as_str) == Some("listening") {
        return;
    }
    let content = authoring.pointer("/passage/content");
    let mut text = Vec::new();
    if let Some(nodes) = content.and_then(Value::as_array) {
        collect_text(nodes, &mut text);
    }
    let has_text = text.iter().any(|value| !is_prompt_placeholder(value));
    let has_visual_fallback = content.is_some_and(|value| {
        node_contains_type(value, "image")
            || node_contains_type(value, "figure")
            || node_contains_type(value, "diagram")
    });
    if !has_text && !has_visual_fallback {
        push_issue(
            issues,
            hard_failures,
            issue(
                PASSAGE_CONTENT_MISSING,
                "blocking",
                "Reading passage 必须包含有效文本或明确的 visual fallback。",
                "document",
                authoring
                    .pointer("/exam/examId")
                    .and_then(Value::as_str)
                    .unwrap_or("document"),
                Vec::new(),
                vec!["edit_text", "confirm_figure"],
            ),
        );
    }
}

fn validate_identifier_and_reference_closure(
    authoring: &Value,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let slots = authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let slot_ids = slots.keys().cloned().collect::<BTreeSet<_>>();
    let answer_ids = authoring
        .get("answerKey")
        .and_then(Value::as_object)
        .map(|answers| answers.keys().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();

    for (slot_key, slot) in &slots {
        if slot_key.trim().is_empty()
            || slot.get("slotId").and_then(Value::as_str) != Some(slot_key.as_str())
        {
            push_issue(
                issues,
                hard_failures,
                issue(
                    SLOT_ID_MISMATCH,
                    "blocking",
                    "answerSlots map key 必须与非空 slotId 完全一致。",
                    "slot",
                    slot_key,
                    anchors_from(slot),
                    vec!["edit_text"],
                ),
            );
        }
    }
    for slot_id in slot_ids.difference(&answer_ids) {
        push_issue(
            issues,
            hard_failures,
            issue(
                ANSWER_KEY_MISSING_SLOT,
                "blocking",
                "AnswerSlot 没有对应的 answerKey entry。",
                "slot",
                slot_id,
                slots.get(slot_id).map(anchors_from).unwrap_or_default(),
                vec!["enter_answer"],
            ),
        );
    }
    for answer_id in answer_ids.difference(&slot_ids) {
        push_issue(
            issues,
            hard_failures,
            issue(
                ANSWER_KEY_ORPHAN_SLOT,
                "blocking",
                "answerKey 引用了不存在的 AnswerSlot。",
                "slot",
                answer_id,
                Vec::new(),
                vec!["enter_answer"],
            ),
        );
    }

    let mut task_ids = BTreeSet::new();
    let mut response_ids = BTreeSet::new();
    let mut assignments = slot_ids
        .iter()
        .map(|slot_id| (slot_id.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    for group in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let task_id = group
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if task_id.is_empty() || !task_ids.insert(task_id.to_string()) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    TASK_ID_DUPLICATE,
                    "blocking",
                    "taskId 必须非空且在文档内唯一。",
                    "task",
                    if task_id.is_empty() {
                        "document"
                    } else {
                        task_id
                    },
                    anchors_from(group),
                    vec!["edit_text"],
                ),
            );
        }
        if group.get("taskType").and_then(Value::as_str)
            != group
                .pointer("/instructionSignature/taskType")
                .and_then(Value::as_str)
        {
            push_issue(
                issues,
                hard_failures,
                issue(
                    TASK_TYPE_SIGNATURE_MISMATCH,
                    "blocking",
                    "taskType 与 instruction signature 声明冲突。",
                    "task",
                    if task_id.is_empty() {
                        "document"
                    } else {
                        task_id
                    },
                    anchors_from(group),
                    vec!["edit_text"],
                ),
            );
        }
        let bank_id = group
            .pointer("/optionBank/optionBankId")
            .and_then(Value::as_str);
        for response in group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let response_id = response
                .get("responseGroupId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if response_id.is_empty() || !response_ids.insert(response_id.to_string()) {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        RESPONSE_GROUP_ID_DUPLICATE,
                        "blocking",
                        "responseGroupId 必须非空且在文档内唯一。",
                        "response_group",
                        if response_id.is_empty() {
                            task_id
                        } else {
                            response_id
                        },
                        anchors_from(response),
                        vec!["edit_text"],
                    ),
                );
            }
            if let Some(reference) = response.get("optionBankRef").and_then(Value::as_str) {
                if bank_id != Some(reference) {
                    push_issue(
                        issues,
                        hard_failures,
                        issue(
                            OPTION_BANK_REFERENCE_MISSING,
                            "blocking",
                            "optionBankRef 未闭合到同一 task group 的 option bank。",
                            "response_group",
                            if response_id.is_empty() {
                                task_id
                            } else {
                                response_id
                            },
                            anchors_from(response),
                            vec!["attach_option_bank"],
                        ),
                    );
                }
            }
            let mut local = BTreeSet::new();
            for slot_id in response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !slot_ids.contains(slot_id) || !local.insert(slot_id) {
                    push_issue(
                        issues,
                        hard_failures,
                        issue(
                            SLOT_REFERENCE_MISSING,
                            "blocking",
                            "response group 引用了不存在或重复的 slot。",
                            "response_group",
                            if response_id.is_empty() {
                                task_id
                            } else {
                                response_id
                            },
                            anchors_from(response),
                            vec!["edit_text", "split_prompt"],
                        ),
                    );
                } else if let Some(count) = assignments.get_mut(slot_id) {
                    *count += 1;
                }
            }
        }
    }
    for (slot_id, count) in assignments {
        if count != 1 {
            push_issue(
                issues,
                hard_failures,
                issue(
                    SLOT_GROUP_ASSIGNMENT_INVALID,
                    "blocking",
                    "每个 AnswerSlot 必须恰好属于一个 response group。",
                    "slot",
                    &slot_id,
                    slots.get(&slot_id).map(anchors_from).unwrap_or_default(),
                    vec!["split_prompt", "edit_text"],
                ),
            );
        }
    }
}

fn validate_provenance(
    authoring: &Value,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    for group in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let task_id = group
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or("document");
        let instructions = group.get("instructions").and_then(Value::as_array);
        if instructions.is_none_or(|nodes| {
            nodes.is_empty() || nodes.iter().any(|node| !has_direct_provenance(node))
        }) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    INSTRUCTION_PROVENANCE_MISSING,
                    "blocking",
                    "instructions 必须非空，且每个 instruction 都必须有 source anchor 或明确 manual provenance。",
                    "task",
                    task_id,
                    Vec::new(),
                    vec!["assign_role", "edit_text"],
                ),
            );
        }
        let signature_anchors = group
            .pointer("/instructionSignature/evidenceAnchors")
            .and_then(Value::as_array);
        if signature_anchors.is_none_or(Vec::is_empty) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    INSTRUCTION_SIGNATURE_EVIDENCE_MISSING,
                    "blocking",
                    "instructionSignature.evidenceAnchors 必须包含直接源证据。",
                    "task",
                    task_id,
                    Vec::new(),
                    vec!["assign_role", "edit_text"],
                ),
            );
        }
        let stimulus = group.get("stimulus").and_then(Value::as_array);
        for response in group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let response_id = response
                .get("responseGroupId")
                .and_then(Value::as_str)
                .unwrap_or(task_id);
            let context = response
                .get("prompt")
                .and_then(Value::as_array)
                .filter(|nodes| !nodes.is_empty())
                .or(stimulus);
            if context.is_some_and(|nodes| nodes.iter().any(|node| !has_direct_provenance(node))) {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        PROVENANCE_MISSING,
                        "blocking",
                        "prompt/stimulus 缺少 source anchor 或明确 manual provenance。",
                        "response_group",
                        response_id,
                        anchors_from(response),
                        vec!["assign_role", "edit_text"],
                    ),
                );
            }
            if let Some(options) = response.get("options").and_then(Value::as_array) {
                validate_option_provenance(options, response_id, issues, hard_failures);
            }
        }
        if let Some(options) = group
            .pointer("/optionBank/options")
            .and_then(Value::as_array)
        {
            validate_option_provenance(options, task_id, issues, hard_failures);
        }
    }
    for (slot_id, slot) in authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if !has_direct_provenance(slot) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    PROVENANCE_MISSING,
                    "blocking",
                    "AnswerSlot 缺少 source anchor 或明确 manual provenance。",
                    "slot",
                    slot_id,
                    Vec::new(),
                    vec!["assign_role", "edit_text"],
                ),
            );
        }
    }
}

fn validate_option_provenance(
    options: &[Value],
    target_id: &str,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    for option in options {
        if !has_direct_provenance(option) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    PROVENANCE_MISSING,
                    "blocking",
                    "option 缺少 source anchor 或明确 manual provenance。",
                    "response_group",
                    option
                        .get("optionId")
                        .and_then(Value::as_str)
                        .unwrap_or(target_id),
                    Vec::new(),
                    vec!["assign_role", "edit_text"],
                ),
            );
        }
    }
}

fn has_direct_provenance(value: &Value) -> bool {
    if value.get("provenanceStatus").and_then(Value::as_str) == Some("manual")
        || value
            .get("sourceAnchors")
            .and_then(Value::as_array)
            .is_some_and(|anchors| !anchors.is_empty())
        || value.get("sourceAnchor").is_some_and(Value::is_object)
    {
        true
    } else {
        false
    }
}

fn anchors_from(value: &Value) -> Vec<Value> {
    value
        .get("sourceAnchors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn evaluate_compiler_probes(authoring: &Value) -> Value {
    let typed = typed_authoring_for_probe(authoring);
    let (v2_probe, v1_probe) = match typed {
        Ok(typed) => {
            let schema_version = crate::listening_source_v1::runtime_schema_version(&typed.modality);
            let v2 = match crate::listening_source_v1::compile_exam_source_v2(&typed) {
                Ok(runtime) => {
                    let round_trip = runtime
                        .document()
                        .map_err(|error| error.to_string())
                        .and_then(|value| {
                            // Re-parse against the contract the compiler actually
                            // chose: a listening source read back as a reading one
                            // would report a false round-trip failure.
                            let reparsed = match &runtime {
                                crate::listening_source_v1::CompiledExamSourceV2::Reading(_) => {
                                    serde_json::from_value::<
                                        crate::reading_source_v2::ReadingExamSourceV2,
                                    >(value)
                                    .map(|_| ())
                                }
                                crate::listening_source_v1::CompiledExamSourceV2::Listening(_) => {
                                    serde_json::from_value::<
                                        crate::schema::listening_runtime_v1::ListeningExamSourceV1,
                                    >(value)
                                    .map(|_| ())
                                }
                            };
                            reparsed.map_err(|error| error.to_string())
                        });
                    match round_trip {
                        Ok(()) => compiler_probe("passed", schema_version, Vec::new(), Vec::new()),
                        Err(error) => compiler_probe(
                            "failed",
                            schema_version,
                            vec!["RUNTIME_SCHEMA_ROUND_TRIP_FAILED".to_string()],
                            vec![error],
                        ),
                    }
                }
                Err(compiler_issues) => {
                    compiler_probe_from_v2_issues(compiler_issues, schema_version)
                }
            };
            let v1 = probe_v1_compatibility(authoring);
            (v2, v1)
        }
        Err(error) => {
            let failed = compiler_probe(
                "failed",
                "IeltsAuthoringIRV2",
                vec![AUTHORING_SCHEMA_INVALID.to_string()],
                vec![error],
            );
            (failed.clone(), failed)
        }
    };
    json!({"v2Runtime": v2_probe, "v1Compatibility": v1_probe})
}

fn typed_authoring_for_probe(authoring: &Value) -> Result<IeltsAuthoringIRV2, String> {
    let mut candidate = authoring.clone();
    candidate["quality"] = probe_quality_placeholder();
    let typed = serde_json::from_value::<IeltsAuthoringIRV2>(candidate)
        .map_err(|error| format!("IeltsAuthoringIRV2 serde validation failed: {error}"))?;
    if !typed.is_supported_schema_version() {
        return Err(format!(
            "unsupported authoring schema version: {}",
            typed.schema_version
        ));
    }
    Ok(typed)
}

fn probe_quality_placeholder() -> Value {
    json!({
        "schemaVersion": "QualityReportV2",
        "state": "review_required",
        "documentScore": 0.0,
        "sourceCoverage": 0.0,
        "coverageLedger": [],
        "coverageStatus": {
            "physicalShadow": "missing",
            "complete": false,
            "significantSourceNodeCount": 0,
            "explainedSourceNodeCount": 0,
            "unassignedSourceNodeIds": []
        },
        "compilerProbes": {
            "v2Runtime": {"status":"failed","schemaVersion":"ReadingExamSourceV2","issueCodes":["PROBE_PENDING"],"details":["probe pending"]},
            "v1Compatibility": {"status":"failed","schemaVersion":"ReadingExamSourceV1","issueCodes":["PROBE_PENDING"],"details":["probe pending"]}
        },
        "taskScores": {},
        "hardFailures": [],
        "issues": [],
        "metrics": {},
        "evaluatedAt": Utc::now().to_rfc3339(),
        "evaluatorVersion": "phase4-pr07-probe-placeholder"
    })
}

fn compiler_probe_from_v2_issues(issues: Vec<CompilerIssueV2>, schema_version: &str) -> Value {
    let mut codes = issues
        .iter()
        .map(|issue| issue.code.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    codes.sort();
    compiler_probe(
        "failed",
        schema_version,
        codes,
        issues
            .into_iter()
            .map(|issue| format!("{}:{}:{}", issue.code, issue.target_id, issue.message))
            .collect(),
    )
}

fn compiler_probe(
    status: &str,
    schema_version: &str,
    issue_codes: Vec<String>,
    details: Vec<String>,
) -> Value {
    json!({
        "status": status,
        "schemaVersion": schema_version,
        "issueCodes": issue_codes,
        "details": details
    })
}

fn append_compiler_probe_issue(
    probes: &Value,
    probe_key: &str,
    code: &str,
    message: &str,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let Some(probe) = probes.get(probe_key) else {
        return;
    };
    if probe.get("status").and_then(Value::as_str) != Some("failed") {
        return;
    }
    let mut value = issue(
        code,
        "blocking",
        message,
        "document",
        "document",
        Vec::new(),
        vec!["edit_text"],
    );
    value["details"] = json!({
        "probe": probe_key,
        "schemaVersion": probe.get("schemaVersion").cloned().unwrap_or(Value::Null),
        "issueCodes": probe.get("issueCodes").cloned().unwrap_or_else(|| json!([])),
        "compilerDetails": probe.get("details").cloned().unwrap_or_else(|| json!([]))
    });
    push_issue(issues, hard_failures, value);
    if probe
        .get("issueCodes")
        .and_then(Value::as_array)
        .is_some_and(|codes| {
            codes
                .iter()
                .any(|item| item.as_str() == Some(AUTHORING_SCHEMA_INVALID))
        })
    {
        push_issue(
            issues,
            hard_failures,
            issue(
                AUTHORING_SCHEMA_INVALID,
                "blocking",
                "IeltsAuthoringIRV2 无法通过 typed schema round-trip。",
                "document",
                "document",
                Vec::new(),
                vec!["edit_text"],
            ),
        );
    }
}

fn probe_v1_compatibility(authoring: &Value) -> Value {
    let source = compile_v1_compatibility_shadow(authoring);
    let typed_result = serde_json::from_value::<ReadingExamSourceV1>(source.clone());
    let mut details = Vec::new();
    let mut codes = BTreeSet::new();
    if let Err(error) = typed_result {
        codes.insert("V1_COMPAT_TYPED_SCHEMA_INVALID".to_string());
        details.push(error.to_string());
    }
    for issue in validate_reading_source_contract(&source) {
        codes.insert(
            issue
                .get("layer")
                .and_then(Value::as_str)
                .map(|layer| format!("V1_{}_VALIDATION_FAILED", layer.to_ascii_uppercase()))
                .unwrap_or_else(|| "V1_SCHEMA_VALIDATION_FAILED".to_string()),
        );
        details.push(format!(
            "{}:{}",
            issue.get("path").and_then(Value::as_str).unwrap_or("$"),
            issue
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("validation failed")
        ));
    }
    if codes.is_empty() {
        compiler_probe("passed", "ReadingExamSourceV1", Vec::new(), Vec::new())
    } else {
        compiler_probe(
            "failed",
            "ReadingExamSourceV1",
            codes.into_iter().collect(),
            details,
        )
    }
}

fn compile_v1_compatibility_shadow(authoring: &Value) -> Value {
    let slots = authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut ordered_slots = slots.iter().collect::<Vec<_>>();
    ordered_slots.sort_by_key(|(slot_id, slot)| {
        (
            slot.get("questionNumber")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX),
            (*slot_id).clone(),
        )
    });
    let question_order = ordered_slots
        .iter()
        .map(|(slot_id, _)| (*slot_id).clone())
        .collect::<Vec<_>>();
    let question_display_map = ordered_slots
        .iter()
        .map(|(slot_id, slot)| {
            (
                (*slot_id).clone(),
                Value::String(
                    slot.get("displayLabel")
                        .and_then(Value::as_str)
                        .unwrap_or(slot_id)
                        .to_string(),
                ),
            )
        })
        .collect::<Map<_, _>>();
    let answer_key = question_order
        .iter()
        .map(|slot_id| {
            (
                slot_id.clone(),
                v1_answer_value(authoring.pointer(&format!("/answerKey/{slot_id}"))),
            )
        })
        .collect::<Map<_, _>>();
    let question_groups = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|group| v1_group_value(group, &slots))
        .collect::<Vec<_>>();
    let passage_html = authoring
        .pointer("/passage/content")
        .map(render_content_text)
        .unwrap_or_default();
    let exam_id = authoring
        .pointer("/exam/examId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    json!({
        "schemaVersion": "ReadingExamSourceV1",
        "examId": exam_id,
        "meta": {
            "title": authoring.pointer("/exam/title").and_then(Value::as_str).unwrap_or("Untitled Reading"),
            "category": authoring.pointer("/exam/category").and_then(Value::as_str).unwrap_or("P1"),
            "frequency": authoring.pointer("/exam/frequency").and_then(Value::as_str).unwrap_or("medium"),
            "pdfFilename": "",
            "legacyPath": "",
            "legacyFilename": "",
            "questionIntroHtml": "<h3>Questions</h3>",
            "questionUmbrellaRanges": []
        },
        "passage": {"blocks":[{"blockId":"passage-main","kind":"html","html":format!("<p>{}</p>", crate::html_escape(&passage_html))}]},
        "questionGroups": question_groups,
        "answerKey": answer_key,
        "sourceRefs": {"primaryHtml":"","primaryProvider":"quality_gate_v2_probe","shuiHtml":null,"shuiPdf":"","ieltsHtml":null},
        "audit": {"matchStatus":"shadow_probe","matchConfidence":0.0,"verifiedAt":null,"notes":"PR-07 read-only V1 compatibility probe"},
        "questionOrder": question_order,
        "questionDisplayMap": question_display_map
    })
}

fn v1_group_value(group: &Value, slots: &Map<String, Value>) -> Value {
    let task_id = group
        .get("taskId")
        .and_then(Value::as_str)
        .unwrap_or("task");
    let kind = v1_question_kind(
        group
            .get("taskType")
            .and_then(Value::as_str)
            .unwrap_or("short_answer"),
    );
    let slot_ids = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut body = format!("<section id=\"{}\">", crate::html_escape(task_id));
    let lead = group_prompt_text(group);
    if !lead.is_empty() {
        body.push_str(&format!("<p>{}</p>", crate::html_escape(&lead)));
    }
    for response in group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let response_slot_ids = response
            .get("slotIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let shared_unordered = response.get("assignment").and_then(Value::as_str)
            == Some("unordered_set")
            && response_slot_ids.len() > 1;
        if shared_unordered {
            let displays = response_slot_ids
                .iter()
                .map(|slot_id| {
                    slots
                        .get(*slot_id)
                        .and_then(|slot| slot.get("displayLabel"))
                        .and_then(Value::as_str)
                        .unwrap_or(slot_id)
                })
                .collect::<Vec<_>>()
                .join(" and ");
            let shared_name = response_slot_ids.join("_");
            let shared_question_ids = response_slot_ids.join(",");
            body.push_str(&format!(
                "<div class=\"question shared-response\"><strong>{}</strong>",
                crate::html_escape(&displays)
            ));
            for (label, text) in options_for_slot(group, response_slot_ids[0]) {
                body.push_str(&format!(
                    "<label><input type=\"checkbox\" name=\"{}\" data-question-ids=\"{}\" value=\"{}\"> {} {}</label>",
                    crate::html_escape(&shared_name),
                    crate::html_escape(&shared_question_ids),
                    crate::html_escape(&label),
                    crate::html_escape(&label),
                    crate::html_escape(&text)
                ));
            }
            body.push_str("</div>");
            continue;
        }
        for slot_id in response_slot_ids {
            let display = slots
                .get(slot_id)
                .and_then(|slot| slot.get("displayLabel"))
                .and_then(Value::as_str)
                .unwrap_or(slot_id);
            let options = options_for_slot(group, slot_id);
            body.push_str(&format!(
                "<div class=\"question\"><strong>{}</strong>",
                crate::html_escape(display)
            ));
            if options.is_empty() {
                body.push_str(&format!(
                    "<input type=\"text\" id=\"{}_input\" name=\"{}\">",
                    crate::html_escape(slot_id),
                    crate::html_escape(slot_id)
                ));
            } else {
                body.push_str(&format!(
                    "<select name=\"{}\" id=\"{}_input\">",
                    crate::html_escape(slot_id),
                    crate::html_escape(slot_id)
                ));
                for (label, text) in options {
                    body.push_str(&format!(
                        "<option value=\"{}\">{} {}</option>",
                        crate::html_escape(&label),
                        crate::html_escape(&label),
                        crate::html_escape(&text)
                    ));
                }
                body.push_str("</select>");
            }
            body.push_str("</div>");
        }
    }
    body.push_str("</section>");
    json!({
        "groupId": task_id,
        "kind": kind,
        "questionIds": slot_ids,
        "bodyHtml": body,
        "leadHtml": format!("<p>{}</p>", crate::html_escape(&lead)),
        "allowOptionReuse": group.pointer("/instructionSignature/allowOptionReuse").and_then(Value::as_bool).unwrap_or(false)
    })
}

fn v1_question_kind(task_type: &str) -> &'static str {
    match task_type {
        "multiple_choice" => "multi_choice",
        "matching_headings" => "heading_matching",
        "matching_information" => "matching_information",
        "matching_features" | "matching_sentence_endings" => "matching",
        "classification" => "classification",
        "summary_completion" | "note_completion" | "form_completion" | "flowchart_completion" => {
            "summary_completion"
        }
        "table_completion" => "table_completion",
        "diagram_label_completion" | "plan_map_label_completion" => "diagram_completion",
        "sentence_completion" => "sentence_completion",
        "true_false_not_given" => "true_false_not_given",
        "yes_no_not_given" => "yes_no_not_given",
        "single_choice" => "single_choice",
        _ => "short_answer",
    }
}

fn options_for_slot(group: &Value, slot_id: &str) -> Vec<(String, String)> {
    let response = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(slot_id)))
        });
    let options = response
        .and_then(|response| response.get("options").and_then(Value::as_array))
        .or_else(|| {
            group
                .pointer("/optionBank/options")
                .and_then(Value::as_array)
        });
    options
        .into_iter()
        .flatten()
        .map(|option| {
            (
                option
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                option
                    .get("content")
                    .map(render_content_text)
                    .unwrap_or_default(),
            )
        })
        .collect()
}

fn v1_answer_value(answer: Option<&Value>) -> Value {
    let Some(answer) = answer else {
        return Value::String(String::new());
    };
    let values = match answer.get("kind").and_then(Value::as_str) {
        Some("option") => answer.get("labels"),
        Some("text") => answer.get("values"),
        _ => None,
    }
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
    if values.len() == 1 {
        values[0].clone()
    } else {
        Value::Array(values)
    }
}

fn render_content_text(value: &Value) -> String {
    let mut parts = Vec::new();
    collect_text_value(value, &mut parts);
    parts
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_text_value(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| collect_text_value(item, out)),
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                out.push(text.to_string());
            }
            for (key, child) in object {
                if key != "text" {
                    collect_text_value(child, out);
                }
            }
        }
        _ => {}
    }
}

fn evaluate_group(
    group: &Value,
    slots: &Map<String, Value>,
    answer_key: &Map<String, Value>,
    expected_numbers: &mut BTreeSet<u32>,
    actual_numbers: &mut Vec<u32>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) -> GroupEvaluation {
    let hard_failure_count_before = hard_failures.len();
    let task_id = group
        .get("taskId")
        .and_then(Value::as_str)
        .unwrap_or("unknown-task");
    let anchors = group
        .get("sourceAnchors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let signature = group.get("instructionSignature");
    let task_type = signature
        .and_then(|value| value.get("taskType"))
        .and_then(Value::as_str)
        .unwrap_or("short_answer");
    let expected = signature
        .and_then(|value| value.get("expectedQuestionNumbers"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .map(|number| number as u32)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if group
        .get("recognitionWarnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|warning| warning.starts_with("task_type_conflict:"))
    {
        push_issue(
            issues,
            hard_failures,
            issue(
                TASK_TYPE_CONFLICT,
                "blocking",
                "题型指令与恢复出的结构证据冲突，必须确认题型后才能发布。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "assign_role"],
            ),
        );
    }
    expected_numbers.extend(expected.iter().copied());
    let group_slot_ids = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|slot_id| slot_id.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();
    let unique_group_slot_ids = group_slot_ids.iter().cloned().collect::<BTreeSet<_>>();
    let expected_slot_count = signature
        .and_then(|value| value.get("expectedSlotCount"))
        .and_then(Value::as_u64)
        .unwrap_or(expected.len() as u64) as usize;
    let display_numbers = display_range_numbers(group.get("displayRange"));
    if expected_slot_count != unique_group_slot_ids.len()
        || (!display_numbers.is_empty()
            && display_numbers != expected.iter().copied().collect::<BTreeSet<_>>())
    {
        push_issue(
            issues,
            hard_failures,
            issue(
                CARDINALITY_SLOT_MISMATCH,
                "blocking",
                "displayRange、expectedSlotCount、expectedQuestionNumbers 与实际 slots 必须一致。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "split_prompt"],
            ),
        );
    }
    for slot_id in &group_slot_ids {
        if let Some(slot) = slots.get(slot_id) {
            if let Some(number) = slot.get("questionNumber").and_then(Value::as_u64) {
                actual_numbers.push(number as u32);
            }
        }
    }
    let slot_coverage = if expected.is_empty() {
        0.0
    } else {
        expected
            .iter()
            .filter(|number| {
                group_slot_ids.iter().any(|slot_id| {
                    slots
                        .get(slot_id)
                        .and_then(|slot| slot.get("questionNumber"))
                        .and_then(Value::as_u64)
                        .is_some_and(|actual| actual as u32 == **number)
                })
            })
            .count() as f64
            / expected.len() as f64
    };
    if slot_coverage < 1.0 {
        push_issue(
            issues,
            hard_failures,
            issue(
                QUESTION_NUMBER_MISSING,
                "blocking",
                "题组的 expected question numbers 与 slots 不一致。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "split_prompt"],
            ),
        );
    }

    let prompt_text = group_prompt_text(group);
    let prompt_coverage = if prompt_text.is_empty() { 0.0 } else { 1.0 };
    if prompt_text.is_empty() && !group_slot_ids.is_empty() {
        push_issue(
            issues,
            hard_failures,
            issue(
                PROMPT_EMPTY,
                "blocking",
                "计分 slot 没有可定位的题干或 stimulus。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "split_prompt"],
            ),
        );
    }
    if group_has_ambiguous_prompt(group) && !group_slot_ids.is_empty() {
        push_issue(
            issues,
            hard_failures,
            issue(
                PROMPT_BOUNDARY_AMBIGUOUS,
                "blocking",
                "题干边界无法从 source evidence 中确定。",
                "task",
                task_id,
                anchors.clone(),
                vec!["split_prompt", "edit_text"],
            ),
        );
    }

    let signature_confidence = signature
        .and_then(|value| value.get("confidence"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if signature_confidence < 0.7 {
        push_issue(
            issues,
            hard_failures,
            issue(
                INSTRUCTION_SIGNATURE_UNRESOLVED,
                "warning",
                "题型 instruction signature 证据不足。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "assign_role"],
            ),
        );
    }

    let option_completeness = validate_options(
        group,
        task_type,
        task_id,
        anchors.clone(),
        issues,
        hard_failures,
    );
    let cardinality_score = validate_cardinality(
        group,
        task_type,
        task_id,
        anchors.clone(),
        issues,
        hard_failures,
    );
    if is_completion_type(task_type) {
        // A word/number limit is mandatory ONLY for *text-entry* completion,
        // where the candidate writes words or numbers sourced from the passage
        // (sentence / summary / note / table / form / flowchart / diagram /
        // plan-map completion). For *selection-type* completion the answer is
        // chosen from a fixed option bank / lettered option alphabet (e.g.
        // "Complete the sentences with the correct letter, A-D" or "Complete the
        // summary using a word A-I from the box"); there the answer is a letter,
        // so an IELTS word limit is meaningless and must NOT block publishing.
        //
        // We detect selection-type via the DERIVED `optionAlphabet` on the
        // instruction signature: when the signature resolved a lettered option
        // set, the task is selection-type. If we cannot tell (no optionAlphabet
        // was resolved) we fail conservatively and keep requiring a word limit
        // rather than silently passing a possibly malformed completion.
        let selection_type = signature
            .and_then(|value| value.get("optionAlphabet"))
            .is_some_and(|value| !value.is_null());
        if !selection_type && signature.and_then(|value| value.get("wordLimit")).is_none() {
            push_issue(
                issues,
                hard_failures,
                issue(
                    WORD_LIMIT_UNPARSED,
                    "blocking",
                    "completion 题组未解析出 IELTS word limit。",
                    "task",
                    task_id,
                    anchors.clone(),
                    vec!["edit_text", "confirm_table"],
                ),
            );
        }
        validate_completion_host(
            group,
            slots,
            task_id,
            anchors.clone(),
            issues,
            hard_failures,
        );
    }
    validate_answers(
        group,
        answer_key,
        task_id,
        anchors.clone(),
        signature,
        issues,
        hard_failures,
    );
    validate_visual_task(
        group,
        task_type,
        task_id,
        anchors.clone(),
        issues,
        hard_failures,
    );

    let source_anchor_coverage = if anchors.is_empty() { 0.0 } else { 1.0 };
    let type_consistency = if task_type == "short_answer" {
        0.72
    } else {
        1.0
    };
    let score = 0.18 * signature_confidence
        + 0.18 * slot_coverage
        + 0.16 * prompt_coverage
        + 0.14 * option_completeness
        + 0.10 * source_anchor_coverage
        + 0.08 * cardinality_score
        + 0.08 * type_consistency
        + 0.08
            * if hard_failures.len() == hard_failure_count_before {
                1.0
            } else {
                0.0
            };
    GroupEvaluation {
        score: score.clamp(0.0, 1.0),
    }
}

fn display_range_numbers(value: Option<&Value>) -> BTreeSet<u32> {
    let Some(value) = value else {
        return BTreeSet::new();
    };
    match value.get("kind").and_then(Value::as_str) {
        Some("range") => {
            let start = value
                .get("start")
                .and_then(Value::as_u64)
                .unwrap_or_default() as u32;
            let end = value.get("end").and_then(Value::as_u64).unwrap_or_default() as u32;
            if start == 0 || end < start {
                BTreeSet::new()
            } else {
                (start..=end).collect()
            }
        }
        Some("set") => value
            .get("values")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|number| number as u32)
            .collect(),
        Some("mixed") => {
            let mut output = BTreeSet::new();
            for item in value
                .get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(number) = item.as_u64() {
                    output.insert(number as u32);
                } else if let (Some(start), Some(end)) = (
                    item.get("start").and_then(Value::as_u64),
                    item.get("end").and_then(Value::as_u64),
                ) {
                    if start > 0 && end >= start {
                        output.extend((start as u32)..=(end as u32));
                    }
                }
            }
            output
        }
        _ => BTreeSet::new(),
    }
}

fn validate_options(
    group: &Value,
    task_type: &str,
    task_id: &str,
    anchors: Vec<Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) -> f64 {
    let expected_labels = group
        .pointer("/instructionSignature/optionAlphabet")
        .and_then(Value::as_str)
        .and_then(expected_labels_from_alphabet);
    let response_groups = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let choice_required = matches!(
        task_type,
        "single_choice"
            | "multiple_choice"
            | "true_false_not_given"
            | "yes_no_not_given"
            | "matching_information"
            | "matching_headings"
            | "matching_features"
            | "matching_sentence_endings"
            | "classification"
    );
    if !choice_required {
        return 1.0;
    }
    let mut total = 0usize;
    let mut complete = 0usize;
    for response in response_groups {
        let options = response
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let bank_ref = response.get("optionBankRef").and_then(Value::as_str);
        let bank_options = bank_ref.and_then(|bank_id| {
            let bank = group.get("optionBank")?;
            (bank.get("optionBankId").and_then(Value::as_str) == Some(bank_id))
                .then(|| bank.get("options").and_then(Value::as_array))
                .flatten()
        });
        let bank_complete = bank_options.is_some_and(|items| {
            items.len() >= 2 && items.iter().all(option_has_renderable_content)
        });
        if options.is_empty() && !bank_complete {
            push_issue(
                issues,
                hard_failures,
                issue(
                    if task_type.starts_with("matching") || task_type == "classification" {
                        OPTION_BANK_MISSING
                    } else {
                        OPTION_RUN_INCOMPLETE
                    },
                    "blocking",
                    "题组需要选项或公共 option bank，但未找到闭合的选项集合。",
                    "response_group",
                    response
                        .get("responseGroupId")
                        .and_then(Value::as_str)
                        .unwrap_or(task_id),
                    anchors.clone(),
                    vec!["attach_option_bank", "edit_text"],
                ),
            );
            total += 1;
            continue;
        }
        total += 1;
        if !options.is_empty() {
            let nonempty = options.iter().all(option_has_renderable_content);
            let labels_match = expected_labels
                .as_ref()
                .is_none_or(|expected| option_labels(&options) == *expected);
            if nonempty && labels_match {
                complete += 1;
            } else {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        if nonempty {
                            OPTION_ALPHABET_MISMATCH
                        } else {
                            OPTION_RUN_INCOMPLETE
                        },
                        "blocking",
                        "固定选项存在 label，但缺少可渲染的 option text。",
                        "response_group",
                        response
                            .get("responseGroupId")
                            .and_then(Value::as_str)
                            .unwrap_or(task_id),
                        anchors.clone(),
                        vec!["edit_text"],
                    ),
                );
            }
        } else if bank_complete {
            let labels_match = expected_labels.as_ref().is_none_or(|expected| {
                bank_options.is_some_and(|items| option_labels(items) == *expected)
            });
            if labels_match {
                complete += 1;
            } else {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        OPTION_ALPHABET_MISMATCH,
                        "blocking",
                        "option bank labels 与 instruction signature 的 alphabet 不一致。",
                        "response_group",
                        response
                            .get("responseGroupId")
                            .and_then(Value::as_str)
                            .unwrap_or(task_id),
                        anchors.clone(),
                        vec!["attach_option_bank", "edit_text"],
                    ),
                );
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        complete as f64 / total as f64
    }
}

fn validate_cardinality(
    group: &Value,
    task_type: &str,
    task_id: &str,
    anchors: Vec<Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) -> f64 {
    let Some(signature) = group.get("instructionSignature") else {
        return 0.0;
    };
    let expected = signature
        .get("selectionCardinality")
        .and_then(|value| value.get("exact"))
        .and_then(Value::as_u64)
        .map(|number| number as usize);
    let Some(expected) = expected else {
        return 1.0;
    };
    let responses = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let response_slot_count = responses
        .iter()
        .map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0)
        })
        .sum::<usize>();
    let group_expected = signature
        .get("expectedQuestionNumbers")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let shared_multiple_choice = task_type == "multiple_choice" && expected > 1;
    let response_policy_valid = responses.iter().all(|response| {
        let slot_count = response
            .get("slotIds")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let cardinality = response.get("cardinality");
        let exact = cardinality
            .and_then(|value| value.get("exact"))
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let min = cardinality
            .and_then(|value| value.get("min"))
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let max = cardinality
            .and_then(|value| value.get("max"))
            .and_then(Value::as_u64)
            .map(|value| value as usize);
        let assignment = response.get("assignment").and_then(Value::as_str);
        let allow_reuse = response.get("allowOptionReuse").and_then(Value::as_bool);
        if shared_multiple_choice {
            slot_count == expected
                && exact == Some(expected)
                && min == Some(expected)
                && max == Some(expected)
                && assignment == Some("unordered_set")
                && allow_reuse == Some(false)
                && response_option_count(group, response) > expected
        } else {
            match assignment {
                Some("per_slot") => {
                    slot_count > 0
                        && exact == Some(expected)
                        && min == Some(expected)
                        && max == Some(expected)
                }
                Some("unordered_set") | Some("ordered_slots") => {
                    slot_count > 0
                        && exact == Some(slot_count)
                        && min == Some(slot_count)
                        && max == Some(slot_count)
                }
                _ => false,
            }
        }
    });
    let valid = if task_type == "multiple_choice" {
        response_slot_count == group_expected
            && expected <= response_slot_count
            && response_policy_valid
    } else {
        response_policy_valid
    };
    if !response_policy_valid {
        push_issue(
            issues,
            hard_failures,
            issue(
                RESPONSE_GROUP_POLICY_MISMATCH,
                "blocking",
                &format!("题组 {task_id} 的 response group policy 与 instruction 不一致。"),
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "split_prompt"],
            ),
        );
    }
    if !valid && response_policy_valid {
        push_issue(
            issues,
            hard_failures,
            issue(
                CARDINALITY_SLOT_MISMATCH,
                "blocking",
                &format!("题组 {task_id} 的 Choose-N cardinality 与 slots 不一致。"),
                "task",
                task_id,
                anchors,
                vec!["split_prompt", "edit_text"],
            ),
        );
        0.0
    } else {
        1.0
    }
}

fn validate_completion_host(
    group: &Value,
    slots: &Map<String, Value>,
    task_id: &str,
    anchors: Vec<Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let slot_ids = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|slot_id| slot_id.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();
    let mut content_roots = group
        .get("stimulus")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    content_roots.extend(
        group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|response| response.get("prompt").and_then(Value::as_array))
            .flatten()
            .cloned(),
    );

    // Textual completion is a single visible document, not a collection of
    // detached question prompts.  Once the geometry pass has recovered the
    // numbered rows, every expected slot must be hosted exactly once by the
    // canonical stimulus tree.  Accepting a slot that only exists in a
    // response prompt makes a fragmented/empty stimulus look publishable and
    // is the source of the previous false-green completion reports.  Table,
    // flowchart and figure tasks have their own structural host checks below,
    // so this closure applies to paragraph-like completion only.
    let task_type = group
        .get("instructionSignature")
        .and_then(|signature| signature.get("taskType"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let inline_completion = matches!(
        task_type,
        "sentence_completion" | "summary_completion" | "note_completion" | "form_completion"
    );
    if inline_completion {
        let stimulus = group
            .get("stimulus")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let expected_numbers = group
            .get("instructionSignature")
            .and_then(|signature| signature.get("expectedQuestionNumbers"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|number| format!("q{number}"))
            .collect::<Vec<_>>();
        let missing_inline_slots = expected_numbers
            .iter()
            .filter(|slot_id| count_slot_nodes_in_roots(stimulus, slot_id) != 1)
            .cloned()
            .collect::<Vec<_>>();
        if !missing_inline_slots.is_empty() {
            let mut closure_issue = issue(
                SLOT_HOST_MISSING,
                "blocking",
                "文本 completion 的 canonical stimulus 必须为每个题号提供唯一的 inline answer slot。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "split_prompt"],
            );
            closure_issue["details"] = json!({
                "missingInlineSlotIds": missing_inline_slots,
                "expectedInlineSlotCount": expected_numbers.len()
            });
            push_issue(issues, hard_failures, closure_issue);
        }
    }
    let invalid = slot_ids.iter().any(|slot_id| {
        let Some(slot) = slots.get(slot_id) else {
            return true;
        };
        let host_id = slot
            .get("hostNodeId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let host_type = slot
            .get("hostType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        host_id.is_empty()
            || !content_roots
                .iter()
                .any(|node| node_contains_slot_host(node, host_id, host_type, slot_id))
    });
    if invalid {
        push_issue(
            issues,
            hard_failures,
            issue(
                SLOT_HOST_MISSING,
                "blocking",
                "completion slot 没有可渲染的宿主节点。",
                "task",
                task_id,
                anchors.clone(),
                vec!["edit_text", "confirm_table"],
            ),
        );
    }
    let duplicated = slot_ids.iter().any(|slot_id| {
        content_roots
            .iter()
            .map(|node| count_answer_slot_nodes(node, slot_id))
            .sum::<usize>()
            > 1
    });
    if duplicated {
        push_issue(
            issues,
            hard_failures,
            issue(
                SLOT_HOST_DUPLICATE,
                "blocking",
                "同一个 completion slot 被渲染到多个内容节点中。",
                "task",
                task_id,
                anchors,
                vec!["edit_text", "confirm_table"],
            ),
        );
    }
}

fn count_slot_nodes_in_roots(nodes: &[Value], slot_id: &str) -> usize {
    nodes
        .iter()
        .map(|node| count_answer_slot_nodes(node, slot_id))
        .sum()
}

fn count_answer_slot_nodes(node: &Value, slot_id: &str) -> usize {
    let own = usize::from(
        node.get("type").and_then(Value::as_str) == Some("answer_slot")
            && node.get("slotId").and_then(Value::as_str) == Some(slot_id),
    );
    own + ["children", "items", "rows", "cells", "steps"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .map(|child| count_answer_slot_nodes(child, slot_id))
        .sum::<usize>()
}

fn node_contains_slot_host(node: &Value, host_id: &str, host_type: &str, slot_id: &str) -> bool {
    if host_type == "figure_hotspot" {
        if node
            .get("hotspots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|hotspot| {
                hotspot.get("hotspotId").and_then(Value::as_str) == Some(host_id)
                    && hotspot.get("slotId").and_then(Value::as_str) == Some(slot_id)
                    && hotspot
                        .get("normalizedRect")
                        .is_some_and(valid_normalized_rect)
            })
        {
            return true;
        }
    } else if node.get("id").and_then(Value::as_str) == Some(host_id)
        && node_contains_answer_slot(node, slot_id)
    {
        return true;
    }
    ["children", "items", "rows", "cells", "steps"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .any(|child| node_contains_slot_host(child, host_id, host_type, slot_id))
}

fn node_contains_answer_slot(node: &Value, slot_id: &str) -> bool {
    if node.get("type").and_then(Value::as_str) == Some("answer_slot")
        && node.get("slotId").and_then(Value::as_str) == Some(slot_id)
    {
        return true;
    }
    ["children", "items", "rows", "cells", "steps"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .any(|child| node_contains_answer_slot(child, slot_id))
}

fn valid_normalized_rect(value: &Value) -> bool {
    let Some(items) = value.as_array().filter(|items| items.len() == 4) else {
        return false;
    };
    let Some(rect) = items.iter().map(Value::as_f64).collect::<Option<Vec<_>>>() else {
        return false;
    };
    rect.iter().all(|value| value.is_finite())
        && rect[0] >= 0.0
        && rect[1] >= 0.0
        && rect[2] > 0.0
        && rect[3] > 0.0
        && rect[0] + rect[2] <= 1.0
        && rect[1] + rect[3] <= 1.0
}

fn validate_answers(
    group: &Value,
    answer_key: &Map<String, Value>,
    task_id: &str,
    anchors: Vec<Value>,
    signature: Option<&Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let slot_ids = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| {
            response
                .get("slotIds")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|slot_id| slot_id.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();
    for slot_id in slot_ids {
        let answer = answer_key.get(&slot_id);
        let unresolved = answer
            .and_then(|value| value.get("kind"))
            .and_then(Value::as_str)
            == Some("unresolved");
        if unresolved || !answer_key.contains_key(&slot_id) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    ANSWER_KEY_MISSING_SLOT,
                    "blocking",
                    "计分 slot 没有可验证的答案 key。",
                    "slot",
                    &slot_id,
                    anchors.clone(),
                    vec!["enter_answer"],
                ),
            );
            continue;
        }
        if let Some(answer) = answer {
            validate_answer_value(
                group,
                &slot_id,
                answer,
                signature,
                anchors.clone(),
                issues,
                hard_failures,
            );
        }
    }
    let _ = task_id;
}

fn validate_answer_value(
    group: &Value,
    slot_id: &str,
    answer: &Value,
    signature: Option<&Value>,
    anchors: Vec<Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    match answer.get("kind").and_then(Value::as_str) {
        Some("option") => {
            let allowed = response_option_labels(group);
            let answer_labels = answer
                .get("labels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|label| label.to_ascii_uppercase())
                .collect::<Vec<_>>();
            if !allowed.is_empty() && answer_labels.iter().any(|label| !allowed.contains(label)) {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        ANSWER_OPTION_NOT_IN_BANK,
                        "blocking",
                        "answer key 的选项 label 不在该题组的 option run 或 option bank 中。",
                        "slot",
                        slot_id,
                        anchors,
                        vec!["attach_option_bank", "enter_answer"],
                    ),
                );
            }
        }
        Some("text") => {
            let values = answer
                .get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            let max_words = signature
                .and_then(|value| value.get("wordLimit"))
                .and_then(|value| value.get("maxWords"))
                .and_then(Value::as_u64)
                .map(|value| value as usize);
            let max_numbers = signature
                .and_then(|value| value.get("wordLimit"))
                .and_then(|value| value.get("maxNumbers"))
                .and_then(Value::as_u64)
                .map(|value| value as usize);
            let violates = values.iter().any(|value| {
                let words = value.split_whitespace().count();
                let numbers = value
                    .split_whitespace()
                    .filter(|token| token.chars().all(|ch| ch.is_ascii_digit()))
                    .count();
                max_words.is_some_and(|limit| words > limit)
                    || max_numbers.is_some_and(|limit| numbers > limit)
            });
            if violates {
                push_issue(
                    issues,
                    hard_failures,
                    issue(
                        ANSWER_WORD_LIMIT_VIOLATION,
                        "blocking",
                        "answer key 超出 instruction signature 声明的 word/number limit。",
                        "slot",
                        slot_id,
                        anchors,
                        vec!["enter_answer", "edit_text"],
                    ),
                );
            }
        }
        _ => {}
    }
}

fn response_option_labels(group: &Value) -> BTreeSet<String> {
    let mut labels = BTreeSet::new();
    if let Some(responses) = group.get("responseGroups").and_then(Value::as_array) {
        for response in responses {
            if let Some(options) = response.get("options").and_then(Value::as_array) {
                labels.extend(options.iter().filter_map(|option| {
                    option
                        .get("label")
                        .and_then(Value::as_str)
                        .map(|label| label.to_ascii_uppercase())
                }));
            }
        }
    }
    if let Some(options) = group
        .pointer("/optionBank/options")
        .and_then(Value::as_array)
    {
        labels.extend(options.iter().filter_map(|option| {
            option
                .get("label")
                .and_then(Value::as_str)
                .map(|label| label.to_ascii_uppercase())
        }));
    }
    labels
}

fn expected_labels_from_alphabet(alphabet: &str) -> Option<BTreeSet<String>> {
    let alphabet = alphabet.trim().to_ascii_uppercase();
    let (start, end) = alphabet.split_once('-')?;
    let start = start.trim().chars().next()?;
    let end = end.trim().chars().next()?;
    if !start.is_ascii_uppercase() || !end.is_ascii_uppercase() || start > end {
        return None;
    }
    Some(
        (start as u8..=end as u8)
            .map(|value| (value as char).to_string())
            .collect(),
    )
}

fn option_labels(options: &[Value]) -> BTreeSet<String> {
    options
        .iter()
        .filter_map(|option| option.get("label").and_then(Value::as_str))
        .map(|label| label.trim().to_ascii_uppercase())
        .filter(|label| !label.is_empty())
        .collect()
}

fn response_option_count(group: &Value, response: &Value) -> usize {
    response
        .get("options")
        .and_then(Value::as_array)
        .map(Vec::len)
        .or_else(|| {
            let bank_ref = response.get("optionBankRef").and_then(Value::as_str)?;
            let bank = group.get("optionBank")?;
            (bank.get("optionBankId").and_then(Value::as_str) == Some(bank_ref))
                .then(|| bank.get("options").and_then(Value::as_array).map(Vec::len))
                .flatten()
        })
        .unwrap_or(0)
}

fn validate_visual_task(
    group: &Value,
    task_type: &str,
    task_id: &str,
    anchors: Vec<Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let stimulus = group.get("stimulus").and_then(Value::as_array);
    let has_diagram = stimulus
        .into_iter()
        .flatten()
        .any(|node| node_contains_type(node, "diagram"));
    let has_table = stimulus
        .into_iter()
        .flatten()
        .any(|node| node_contains_type(node, "table"));
    let has_flowchart = stimulus
        .into_iter()
        .flatten()
        .any(|node| node_contains_type(node, "flowchart"));
    let diagram_task = matches!(
        task_type,
        "diagram_label_completion" | "plan_map_label_completion"
    );
    if diagram_task && !has_diagram {
        push_issue(
            issues,
            hard_failures,
            issue(
                ASSET_REFERENCE_MISSING,
                "blocking",
                "diagram/map 题组没有可验证的 figure asset reference。",
                "task",
                task_id,
                anchors.clone(),
                vec!["replace_asset", "confirm_figure"],
            ),
        );
        if group_has_figure_slots(group) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    SLOT_OUTSIDE_FIGURE,
                    "blocking",
                    "hotspot slot 没有落在可定位的 figure 节点内。",
                    "task",
                    task_id,
                    anchors.clone(),
                    vec!["confirm_figure", "edit_text"],
                ),
            );
        }
    }
    if diagram_task && has_diagram && !diagram_hotspots_are_closed(group) {
        push_issue(
            issues,
            hard_failures,
            issue(
                HOTSPOT_GEOMETRY_INVALID,
                "blocking",
                "diagram/map hotspots 未覆盖全部 slots，或 normalizedRect 无效。",
                "task",
                task_id,
                anchors.clone(),
                vec!["confirm_figure", "edit_text"],
            ),
        );
    }
    if task_type == "table_completion" && !has_table {
        push_issue(
            issues,
            hard_failures,
            issue(
                SLOT_HOST_MISSING,
                "blocking",
                "table completion 没有可渲染的 table stimulus。",
                "task",
                task_id,
                anchors.clone(),
                vec!["confirm_table", "edit_text"],
            ),
        );
    }
    if task_type == "flowchart_completion" && !has_flowchart {
        push_issue(
            issues,
            hard_failures,
            issue(
                SLOT_HOST_MISSING,
                "blocking",
                "flowchart completion 没有可渲染的 flowchart stimulus。",
                "task",
                task_id,
                anchors,
                vec!["confirm_figure", "edit_text"],
            ),
        );
    }
}

fn diagram_hotspots_are_closed(group: &Value) -> bool {
    let slot_ids = group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| response.get("slotIds").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    if slot_ids.is_empty() {
        return false;
    }
    let hotspots = group
        .get("stimulus")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(collect_hotspots)
        .collect::<Vec<_>>();
    let hotspot_slots = hotspots
        .iter()
        .filter(|hotspot| {
            hotspot
                .get("normalizedRect")
                .is_some_and(valid_normalized_rect)
                && hotspot
                    .get("hotspotId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.trim().is_empty())
        })
        .filter_map(|hotspot| hotspot.get("slotId").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    hotspot_slots == slot_ids && hotspots.len() == slot_ids.len()
}

fn collect_hotspots(node: &Value) -> Vec<&Value> {
    let mut hotspots = node
        .get("hotspots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    for key in ["children", "items", "rows", "cells", "steps"] {
        for child in node
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            hotspots.extend(collect_hotspots(child));
        }
    }
    hotspots
}

fn group_has_figure_slots(group: &Value) -> bool {
    group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response| response.get("slotIds").and_then(Value::as_array))
        .flatten()
        .any(|slot_id| {
            group
                .get("taskType")
                .and_then(Value::as_str)
                .is_some_and(|task_type| {
                    matches!(
                        task_type,
                        "diagram_label_completion" | "plan_map_label_completion"
                    )
                })
                && slot_id.as_str().is_some()
        })
}

fn node_contains_type(node: &Value, expected: &str) -> bool {
    if node.get("type").and_then(Value::as_str) == Some(expected) {
        return true;
    }
    ["children", "items", "rows", "cells", "steps"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .any(|child| node_contains_type(child, expected))
}

/// `user_upload` 资产的四种核对里，`Verified` 还差最后一道：part media 的引用。
///
/// 台账核的是「磁盘上那份文件」；`listening.parts[].media` 才是学生端会去取的那条引用。
/// 两边指的不是同一份音频时，播放出来的就不是这一节的内容——不报出来等于让学生听着
/// 第二节的音频答第一节的题。返回第一个不一致的 partId。
fn user_upload_part_media_hash_conflict(
    authoring: &Value,
    asset_id: &str,
    declared_hash: &str,
) -> Option<String> {
    authoring
        .pointer("/listening/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|part| {
            part.pointer("/media/assetId").and_then(Value::as_str) == Some(asset_id)
        })
        .find(|part| {
            part.pointer("/media/sha256").and_then(Value::as_str) != Some(declared_hash)
        })
        .and_then(|part| part.get("partId").and_then(Value::as_str))
        .map(ToString::to_string)
}

/// `extractionMode == "user_upload"` 资产的核对：台账有行、受管文件在磁盘上、内容哈希与
/// 声明一致、探测通过，且 part media 引用的是同一份音频。
///
/// **任一条不符都是硬失败，包括「拿不到台账事实」**。把「读不到台账」当成通过，就是那条
/// 让一切看起来都是绿的、学生端却打不开的老路；宁可如实报「无法核对」。判定依据全部来自
/// 调用方传进来的 [`ManagedAudioFactsV1`]，这里不开连接、不碰文件系统。
fn validate_user_upload_asset(
    authoring: &Value,
    asset_id: &str,
    declared_hash: &str,
    managed_audio: Option<&ManagedAudioFactsV1>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let check = managed_audio.map(|facts| facts.check(asset_id));
    let (code, message, reason) = match check {
        // 台账四条全中：再核 part media 指向的是不是同一份音频。
        Some(ManagedAudioCheckV1::Verified) => {
            match user_upload_part_media_hash_conflict(authoring, asset_id, declared_hash) {
                Some(part_id) => (
                    ASSET_HASH_MISMATCH,
                    "听力 part 的 media 引用的不是这份音频（sha256 不一致）。",
                    format!("part_media_hash_conflict:{part_id}"),
                ),
                None => return,
            }
        }
        Some(ManagedAudioCheckV1::HashMismatch) => (
            ASSET_HASH_MISMATCH,
            "受管音频文件的内容与声明的 sha256 不一致。",
            "hash_mismatch".to_string(),
        ),
        Some(ManagedAudioCheckV1::ProbeBlocked) => (
            LISTENING_AUDIO_PROBE_BLOCKED,
            "受管音频没有通过探测，播放或判分可能不可用。",
            "probe_blocked".to_string(),
        ),
        Some(ManagedAudioCheckV1::NoRecord) => (
            ASSET_REFERENCE_MISSING,
            "受管音频台账里没有这条资产的记录。",
            "no_ledger_record".to_string(),
        ),
        Some(ManagedAudioCheckV1::FileMissing) => (
            ASSET_REFERENCE_MISSING,
            "受管音频台账有记录，但文件不在磁盘上。",
            "managed_file_missing".to_string(),
        ),
        // 稿里有用户上传的资产却拿不到台账事实：核对不了 == 不通过。
        None => (
            ASSET_REFERENCE_MISSING,
            "无法核对这份用户上传的音频（受管音频台账不可用）。",
            "managed_audio_facts_unavailable".to_string(),
        ),
    };
    let mut audio_issue = issue(
        code,
        "blocking",
        message,
        "asset",
        asset_id,
        Vec::new(),
        vec!["replace_asset"],
    );
    audio_issue["details"] = json!({
        "assetId": asset_id,
        "managedAudioReason": reason,
    });
    push_issue(issues, hard_failures, audio_issue);
}

fn validate_assets(
    authoring: &Value,
    physical_shadow: Option<&Value>,
    managed_audio: Option<&ManagedAudioFactsV1>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let assets = authoring
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let asset_ids = assets
        .iter()
        .filter_map(|asset| asset.get("assetId").and_then(Value::as_str))
        .filter(|asset_id| !asset_id.trim().is_empty())
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let mut seen_asset_ids = BTreeSet::new();
    let physical_assets = physical_shadow
        .and_then(|physical| physical.get("assets"))
        .and_then(Value::as_array)
        .map(|assets| {
            assets
                .iter()
                .filter_map(|asset| {
                    asset
                        .get("assetId")
                        .and_then(Value::as_str)
                        .map(|id| (id.to_string(), asset))
                })
                .collect::<BTreeMap<_, _>>()
        });
    for asset in &assets {
        let asset_id = asset
            .get("assetId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if asset
            .pointer("/diagramQuestionRegion/recoveryStatus")
            .and_then(Value::as_str)
            == Some("ocr_required")
            || asset
                .pointer("/diagramQuestionRegion/numberClosure")
                .and_then(Value::as_bool)
                == Some(false)
        {
            push_issue(
                issues,
                hard_failures,
                issue(
                    DIAGRAM_QUESTION_REGION_OCR_REQUIRED,
                    "blocking",
                    "图示题区域已保留，但栅格题号和标签尚未完成 OCR 与编号闭包。",
                    "asset",
                    if asset_id.is_empty() {
                        "unknown-asset"
                    } else {
                        asset_id
                    },
                    Vec::new(),
                    vec!["confirm_figure", "edit_text"],
                ),
            );
        }
        if !asset_id.is_empty() && !seen_asset_ids.insert(asset_id.to_string()) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    ASSET_ID_DUPLICATE,
                    "blocking",
                    "assetId 必须在文档内唯一。",
                    "asset",
                    asset_id,
                    Vec::new(),
                    vec!["replace_asset"],
                ),
            );
        }
        let relative_path = asset
            .get("relativePath")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if asset_id.trim().is_empty() || relative_path.trim().is_empty() {
            push_issue(
                issues,
                hard_failures,
                issue(
                    ASSET_REFERENCE_MISSING,
                    "blocking",
                    "asset descriptor 缺少 assetId 或 relativePath。",
                    "asset",
                    if asset_id.is_empty() {
                        "unknown-asset"
                    } else {
                        asset_id
                    },
                    Vec::new(),
                    vec!["replace_asset"],
                ),
            );
        }
        if !relative_path_is_safe(relative_path) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    ASSET_PATH_UNSAFE,
                    "blocking",
                    "asset descriptor 必须使用不含 traversal、drive prefix 或绝对路径的相对路径。",
                    "asset",
                    if asset_id.is_empty() {
                        "unknown-asset"
                    } else {
                        asset_id
                    },
                    Vec::new(),
                    vec!["replace_asset"],
                ),
            );
        }
        let hash = asset
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if hash.len() != 64 || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
            push_issue(
                issues,
                hard_failures,
                issue(
                    ASSET_HASH_MISMATCH,
                    "blocking",
                    "asset descriptor 的 sha256 不是可验证的 SHA-256。",
                    "asset",
                    if asset_id.is_empty() {
                        "unknown-asset"
                    } else {
                        asset_id
                    },
                    Vec::new(),
                    vec!["replace_asset"],
                ),
            );
        }
        // 用户上传的资产（听力音频）**不**走 physical shadow。
        //
        // 音频从来没进过 PDF，拿它去和 shadow 的资产表按 assetId 比对必然对不上：用户按
        // 界面提示把四段音频绑好了，门禁却逐段报「这个资产在原文里不存在」。它的权威依据
        // 是受管音频台账 `listening_audio_assets_v1` 加上磁盘上那份文件，见
        // `validate_user_upload_asset`。逐 part 的 `LISTENING_AUDIO_*` 另有一条判据。
        if asset.get("extractionMode").and_then(Value::as_str) == Some("user_upload") {
            validate_user_upload_asset(
                authoring,
                asset_id,
                hash,
                managed_audio,
                issues,
                hard_failures,
            );
        } else if let Some(physical_assets) = &physical_assets {
            match physical_assets.get(asset_id) {
                None => push_issue(
                    issues,
                    hard_failures,
                    issue(
                        ASSET_REFERENCE_MISSING,
                        "blocking",
                        "authoring asset 在 physical shadow 中不存在。",
                        "asset",
                        if asset_id.is_empty() {
                            "unknown-asset"
                        } else {
                            asset_id
                        },
                        Vec::new(),
                        vec!["replace_asset"],
                    ),
                ),
                Some(physical_asset)
                    if physical_asset.get("sha256").and_then(Value::as_str) != Some(hash) =>
                {
                    push_issue(
                        issues,
                        hard_failures,
                        issue(
                            ASSET_HASH_MISMATCH,
                            "blocking",
                            "authoring asset hash 与 physical shadow 不一致。",
                            "asset",
                            asset_id,
                            Vec::new(),
                            vec!["replace_asset"],
                        ),
                    );
                }
                _ => {}
            }
        }
    }
    let mut referenced = BTreeSet::new();
    collect_asset_references(authoring.get("passage"), &mut referenced);
    collect_asset_references(authoring.get("taskGroups"), &mut referenced);
    for asset_id in referenced.difference(&asset_ids) {
        push_issue(
            issues,
            hard_failures,
            issue(
                ASSET_REFERENCE_MISSING,
                "blocking",
                "content node 引用了不存在的 asset descriptor。",
                "asset",
                asset_id,
                Vec::new(),
                vec!["replace_asset"],
            ),
        );
    }
}

fn relative_path_is_safe(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    !normalized.is_empty()
        && !normalized.starts_with('/')
        && !normalized.contains(':')
        && normalized
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn collect_asset_references(value: Option<&Value>, out: &mut BTreeSet<String>) {
    let Some(value) = value else {
        return;
    };
    match value {
        Value::Array(items) => {
            for item in items {
                collect_asset_references(Some(item), out);
            }
        }
        Value::Object(object) => {
            for key in ["assetId", "visualFallbackAssetId"] {
                if let Some(asset_id) = object.get(key).and_then(Value::as_str) {
                    if !asset_id.trim().is_empty() {
                        out.insert(asset_id.to_string());
                    }
                }
            }
            for child in object.values() {
                collect_asset_references(Some(child), out);
            }
        }
        _ => {}
    }
}

fn group_prompt_text(group: &Value) -> String {
    let mut texts = Vec::new();
    if let Some(nodes) = group.get("stimulus").and_then(Value::as_array) {
        collect_text(nodes, &mut texts);
    }
    if let Some(responses) = group.get("responseGroups").and_then(Value::as_array) {
        for response in responses {
            if let Some(nodes) = response.get("prompt").and_then(Value::as_array) {
                collect_text(nodes, &mut texts);
            }
        }
    }
    texts.join(" ").trim().to_string()
}

fn group_has_ambiguous_prompt(group: &Value) -> bool {
    group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|response| response.get("prompt"))
        .flat_map(|prompt| prompt.as_array().into_iter().flatten())
        .any(node_contains_prompt_placeholder)
}

fn option_has_renderable_content(option: &Value) -> bool {
    option
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|content| {
            !content.is_empty()
                && content.iter().any(|node| {
                    node.get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| {
                            let normalized = text.trim();
                            !normalized.is_empty() && normalized != "[missing option text]"
                        })
                })
        })
}

fn is_prompt_placeholder(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    normalized == "[prompt pending review]"
        || normalized == "[shared prompt pending review]"
        || normalized == "prompt pending review"
}

fn node_contains_prompt_placeholder(node: &Value) -> bool {
    if node
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(is_prompt_placeholder)
    {
        return true;
    }
    ["children", "items", "rows", "cells"]
        .iter()
        .filter_map(|key| node.get(*key).and_then(Value::as_array))
        .flatten()
        .any(node_contains_prompt_placeholder)
}

fn collect_text(nodes: &[Value], out: &mut Vec<String>) {
    for node in nodes {
        if let Some(text) = node.get("text").and_then(Value::as_str) {
            if !text.trim().is_empty() && !is_prompt_placeholder(text) {
                out.push(text.trim().to_string());
            }
        }
        for key in ["children", "items", "rows", "cells"] {
            if let Some(children) = node.get(key).and_then(Value::as_array) {
                collect_text(children, out);
            }
        }
    }
}

fn is_completion_type(value: &str) -> bool {
    matches!(
        value,
        "sentence_completion"
            | "summary_completion"
            | "note_completion"
            | "table_completion"
            | "form_completion"
            | "flowchart_completion"
            | "diagram_label_completion"
            | "plan_map_label_completion"
    )
}

fn calculate_source_coverage(authoring: &Value, physical_shadow: Option<&Value>) -> f64 {
    source_coverage_summary(authoring, physical_shadow).score
}

fn validate_source_ownership(
    authoring: &Value,
    physical_shadow: Option<&Value>,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    let Some(physical) = physical_shadow.filter(|value| physical_shadow_is_usable(value)) else {
        return;
    };
    let significant_ids = significant_physical_nodes(physical)
        .into_keys()
        .collect::<BTreeSet<_>>();
    if significant_ids.is_empty() {
        return;
    }

    let mut owners_by_node = BTreeMap::<String, BTreeSet<String>>::new();
    if let Some(passage) = authoring.get("passage") {
        let mut node_ids = BTreeSet::new();
        collect_direct_anchor_node_ids(passage, &mut node_ids);
        for node_id in node_ids.intersection(&significant_ids) {
            owners_by_node
                .entry(node_id.clone())
                .or_default()
                .insert("passage".to_string());
        }
    }
    for group in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let owner = group
            .get("taskId")
            .and_then(Value::as_str)
            .unwrap_or("unknown-task")
            .to_string();
        let mut node_ids = BTreeSet::new();
        collect_direct_anchor_node_ids(group, &mut node_ids);
        for node_id in node_ids.intersection(&significant_ids) {
            owners_by_node
                .entry(node_id.clone())
                .or_default()
                .insert(owner.clone());
        }
    }

    let conflicts = owners_by_node
        .into_iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(source_node_id, owners)| {
            json!({
                "sourceNodeId": source_node_id,
                "owners": owners.into_iter().collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    if conflicts.is_empty() {
        return;
    }

    let mut ownership_issue = issue(
        SOURCE_OWNERSHIP_CONFLICT,
        "blocking",
        "同一显著源节点被 passage 或多个题组重复占有，题组边界尚未闭合。",
        "document",
        "document",
        Vec::new(),
        vec!["split_prompt", "assign_role"],
    );
    ownership_issue["details"] = json!({
        "conflictCount": conflicts.len(),
        "conflicts": conflicts.into_iter().take(50).collect::<Vec<_>>()
    });
    push_issue(issues, hard_failures, ownership_issue);
}

fn collect_direct_anchor_node_ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_direct_anchor_node_ids(item, out);
            }
        }
        Value::Object(object) => {
            for node_id in object
                .get("sourceAnchors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|anchor| anchor.get("nodeIds").and_then(Value::as_array))
                .flatten()
                .filter_map(Value::as_str)
            {
                out.insert(node_id.to_string());
            }
            for node_id in object
                .get("sourceAnchor")
                .and_then(Value::as_object)
                .and_then(|anchor| anchor.get("nodeIds"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                out.insert(node_id.to_string());
            }
            for (key, child) in object {
                if !matches!(
                    key.as_str(),
                    "quality" | "coverageLedger" | "coverageStatus"
                ) {
                    collect_direct_anchor_node_ids(child, out);
                }
            }
        }
        _ => {}
    }
}

fn source_text_key(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn source_coverage_summary(
    authoring: &Value,
    physical_shadow: Option<&Value>,
) -> SourceCoverageSummary {
    let Some(physical) = physical_shadow.filter(|physical| physical_shadow_is_usable(physical))
    else {
        return SourceCoverageSummary::default();
    };
    let source_nodes = significant_physical_nodes(physical);
    if source_nodes.is_empty() {
        return SourceCoverageSummary::default();
    }
    let mut targets = BTreeMap::<String, BTreeSet<String>>::new();
    collect_anchor_targets(authoring, None, &mut targets);
    for asset_id in authoring
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| asset.get("assetId").and_then(Value::as_str))
    {
        targets
            .entry(asset_id.to_string())
            .or_default()
            .insert(asset_id.to_string());
    }
    let ignored = physical_ignored_reasons(physical);
    let mut ledger = Vec::new();
    let mut unassigned_ids = Vec::new();
    let mut assigned_count = 0usize;
    for (source_node_id, aliases) in &source_nodes {
        let mut target_id_set = aliases
            .iter()
            .chain(std::iter::once(source_node_id))
            .flat_map(|alias| targets.get(alias).into_iter().flatten().cloned())
            .collect::<BTreeSet<_>>();
        for alias in aliases {
            let Some(text_key) = alias.strip_prefix("__text:") else {
                continue;
            };
            if text_key.len() < 12 {
                continue;
            }
            for (target_key, target_values) in &targets {
                let Some(target_text) = target_key.strip_prefix("__text:") else {
                    continue;
                };
                if target_text.len() >= 12
                    && (target_text.contains(text_key) || text_key.contains(target_text))
                {
                    target_id_set.extend(target_values.iter().cloned());
                }
            }
        }
        let target_ids = target_id_set.into_iter().collect::<Vec<_>>();
        // Regions are containers over their child lines/spans/glyphs.  A
        // semantic anchor may legitimately point at a child node without
        // naming the container id itself; close that parent through its
        // already-expanded aliases before declaring it unassigned.
        let child_target_ids = aliases
            .iter()
            .flat_map(|alias| targets.get(alias).into_iter().flatten().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let target_ids = target_ids
            .into_iter()
            .chain(child_target_ids)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let reason = ignored
            .get(source_node_id)
            .cloned()
            .or_else(|| aliases.iter().find_map(|alias| ignored.get(alias).cloned()));
        let disposition = if !target_ids.is_empty() {
            assigned_count += 1;
            "assigned"
        } else if reason.is_some() {
            assigned_count += 1;
            "ignored_with_reason"
        } else {
            unassigned_ids.push(source_node_id.clone());
            "unassigned"
        };
        let mut entry = json!({
            "sourceNodeId": source_node_id,
            "significant": true,
            "disposition": disposition,
            "targetIds": target_ids
        });
        if let Some(reason) = reason {
            entry["reason"] = Value::String(reason);
        }
        ledger.push(entry);
    }
    SourceCoverageSummary {
        score: assigned_count as f64 / source_nodes.len() as f64,
        significant_count: source_nodes.len(),
        assigned_count,
        unassigned_ids,
        ledger,
        physical_available: true,
    }
}

fn physical_shadow_is_usable(physical: &Value) -> bool {
    physical.get("schemaVersion").and_then(Value::as_str) == Some("DocumentIRV2")
        && physical
            .get("documentId")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        && physical
            .get("jobId")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        && physical
            .get("sourceFiles")
            .and_then(Value::as_array)
            .is_some_and(|sources| !sources.is_empty())
        && physical
            .get("pages")
            .and_then(Value::as_array)
            .is_some_and(|pages| !pages.is_empty())
}

fn collect_anchor_targets(
    value: &Value,
    inherited_target: Option<&str>,
    out: &mut BTreeMap<String, BTreeSet<String>>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_anchor_targets(item, inherited_target, out);
            }
        }
        Value::Object(object) => {
            let target = [
                "slotId",
                "responseGroupId",
                "optionId",
                "optionBankId",
                "taskId",
                "id",
                "assetId",
                "examId",
            ]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str))
            .or(inherited_target)
            .unwrap_or("document");
            for node_id in object
                .get("sourceAnchors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|anchor| anchor.get("nodeIds").and_then(Value::as_array))
                .flatten()
                .filter_map(Value::as_str)
            {
                out.entry(node_id.to_string())
                    .or_default()
                    .insert(target.to_string());
            }
            for key in ["text", "html", "textPreview"] {
                if let Some(text_key) = object
                    .get(key)
                    .and_then(Value::as_str)
                    .map(source_text_key)
                    .filter(|key| key.len() >= 12)
                {
                    out.entry(format!("__text:{text_key}"))
                        .or_default()
                        .insert(target.to_string());
                }
            }
            for node_id in object
                .get("sourceAnchor")
                .and_then(Value::as_object)
                .and_then(|anchor| anchor.get("nodeIds"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                out.entry(node_id.to_string())
                    .or_default()
                    .insert(target.to_string());
            }
            for (key, child) in object {
                if !matches!(
                    key.as_str(),
                    "quality" | "coverageLedger" | "coverageStatus"
                ) {
                    collect_anchor_targets(child, Some(target), out);
                }
            }
        }
        _ => {}
    }
}

fn physical_ignored_reasons(physical: &Value) -> BTreeMap<String, String> {
    let mut ignored = physical
        .get("coverageLedger")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry.get("disposition").and_then(Value::as_str) == Some("ignored_with_reason")
        })
        .filter_map(|entry| {
            let id = entry.get("sourceNodeId").and_then(Value::as_str)?;
            let reason = entry
                .get("reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.trim().is_empty())?;
            Some((id.to_string(), reason.to_string()))
        })
        .collect::<BTreeMap<_, _>>();

    for page in physical
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let line_texts = page
            .get("lines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|line| {
                let id = line.get("id").and_then(Value::as_str)?;
                let text = line.get("text").and_then(Value::as_str).unwrap_or_default();
                Some((id.to_string(), text.to_string()))
            })
            .collect::<BTreeMap<_, _>>();
        let ocr_page = page
            .pointer("/quality/classification")
            .and_then(Value::as_str)
            == Some("scanned")
            && page
                .pointer("/quality/requiresOcrRegions")
                .and_then(Value::as_array)
                .is_some_and(|regions| !regions.is_empty());

        for region in page
            .get("regions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(region_id) = region.get("id").and_then(Value::as_str) else {
                continue;
            };
            if ignored.contains_key(region_id) {
                continue;
            }
            let kind = region
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let child_line_ids = region
                .get("childLineIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            let region_text = child_line_ids
                .iter()
                .filter_map(|line_id| line_texts.get(*line_id))
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            let has_semantic_text = region_text.chars().any(char::is_alphanumeric);
            let child_object_count = region
                .get("childObjectIds")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            let narrow_region = region
                .get("bbox")
                .and_then(Value::as_object)
                .and_then(|bbox| bbox.get("width"))
                .and_then(Value::as_f64)
                .is_some_and(|width| width <= 8.0);

            // The PDF extractor can materialize a one-column sliver as a table
            // with an empty line and a synthetic table object. It carries no
            // authorable text or asset, so keep it explained without treating
            // it as a real table task.
            if kind == "table" && !has_semantic_text && child_object_count > 0 && narrow_region {
                ignored.insert(
                    region_id.to_string(),
                    "narrow_empty_table_layout_artifact".to_string(),
                );
                continue;
            }

            let compact_text = source_text_key(&region_text);
            if kind == "table"
                && (compact_text.starts_with("youshouldspendabout") || compact_text == "2below")
            {
                ignored.insert(
                    region_id.to_string(),
                    "exam_instruction_layout_fragment".to_string(),
                );
                continue;
            }

            if ocr_page
                && (compact_text.contains("答案")
                    || compact_text.contains("answer")
                    || compact_text.contains("explanation")
                    || compact_text.contains("分析")
                    || compact_text.contains("下页")
                    || compact_text.contains("nextpage"))
            {
                ignored.insert(
                    region_id.to_string(),
                    "answer_explanation_overlay_on_ocr_page".to_string(),
                );
            }
        }
    }
    ignored
}

fn significant_physical_nodes(physical: &Value) -> BTreeMap<String, BTreeSet<String>> {
    fn insert_node(
        output: &mut BTreeMap<String, BTreeSet<String>>,
        id: Option<&str>,
        aliases: impl IntoIterator<Item = String>,
    ) {
        let Some(id) = id.filter(|id| !id.trim().is_empty()) else {
            return;
        };
        output.entry(id.to_string()).or_default().extend(aliases);
    }

    fn string_values(value: Option<&Value>) -> Vec<String> {
        value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect()
    }

    let mut output = BTreeMap::new();
    for page in physical
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let spans = page
            .get("spans")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let span_glyph_ids = spans
            .iter()
            .filter_map(|span| {
                span.get("id")
                    .and_then(Value::as_str)
                    .map(|id| (id.to_string(), string_values(span.get("glyphIds"))))
            })
            .collect::<BTreeMap<_, _>>();
        let lines = page
            .get("lines")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut referenced_span_ids = BTreeSet::new();
        let mut line_aliases = BTreeMap::<String, Vec<String>>::new();
        for line in lines {
            let span_ids = string_values(line.get("spanIds"));
            referenced_span_ids.extend(span_ids.iter().cloned());
            let aliases =
                span_ids
                    .iter()
                    .cloned()
                    .chain(span_ids.iter().flat_map(|span_id| {
                        span_glyph_ids.get(span_id).into_iter().flatten().cloned()
                    }))
                    .collect::<Vec<_>>();
            if let Some(line_id) = line.get("id").and_then(Value::as_str) {
                line_aliases.insert(line_id.to_string(), aliases);
            }
        }
        let referenced_glyph_ids = span_glyph_ids
            .values()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut region_line_ids = BTreeSet::new();
        let mut region_aliases = BTreeMap::<String, Vec<String>>::new();
        let object_aliases = ["annotations", "imagePlacements"]
            .into_iter()
            .flat_map(|field| {
                page.get(field)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(|item| {
                let id = item.get("id").and_then(Value::as_str)?;
                let aliases = item
                    .get("assetId")
                    .and_then(Value::as_str)
                    .map(|asset_id| vec![asset_id.to_string()])
                    .unwrap_or_default();
                Some((id.to_string(), aliases))
            })
            .collect::<BTreeMap<_, _>>();
        for region in page
            .get("regions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let child_lines = string_values(region.get("childLineIds"));
            region_line_ids.extend(child_lines.iter().cloned());
            let child_objects = string_values(region.get("childObjectIds"));
            let mut aliases =
                child_lines
                    .iter()
                    .cloned()
                    .chain(child_lines.iter().flat_map(|line_id| {
                        line_aliases.get(line_id).into_iter().flatten().cloned()
                    }))
                    .chain(child_objects.iter().cloned())
                    .chain(child_objects.iter().flat_map(|object_id| {
                        object_aliases.get(object_id).into_iter().flatten().cloned()
                    }))
                    .collect::<Vec<_>>();
            let region_text = child_lines
                .iter()
                .filter_map(|line_id| {
                    lines
                        .iter()
                        .find(|line| line.get("id").and_then(Value::as_str) == Some(line_id))
                        .and_then(|line| line.get("text").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join(" ");
            let region_text_key = source_text_key(&region_text);
            if region_text_key.len() >= 12 {
                aliases.push(format!("__text:{region_text_key}"));
            }
            if let Some(region_id) = region.get("id").and_then(Value::as_str) {
                region_aliases.insert(region_id.to_string(), aliases.clone());
            }
            let kind = region
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if matches!(kind, "header" | "footer" | "page_number" | "rule") {
                continue;
            }
            let resolved_child_lines = child_lines
                .iter()
                .filter_map(|line_id| {
                    lines
                        .iter()
                        .find(|line| line.get("id").and_then(Value::as_str) == Some(line_id))
                })
                .collect::<Vec<_>>();
            let has_semantic_text = resolved_child_lines.iter().any(|line| {
                line.get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.chars().any(char::is_alphanumeric))
            });
            let unresolved_explicit_lines =
                !child_lines.is_empty() && resolved_child_lines.len() < child_lines.len();
            let structural_kind = !matches!(kind, "text" | "title");
            if !has_semantic_text
                && child_objects.is_empty()
                && !unresolved_explicit_lines
                && !structural_kind
            {
                continue;
            }
            insert_node(
                &mut output,
                region.get("id").and_then(Value::as_str),
                aliases,
            );
        }
        for line in lines {
            let id = line.get("id").and_then(Value::as_str);
            if id.is_some_and(|id| region_line_ids.contains(id)) {
                continue;
            }
            let aliases = id
                .and_then(|id| line_aliases.get(id))
                .cloned()
                .unwrap_or_default();
            insert_node(&mut output, id, aliases);
        }
        for span in spans {
            let id = span.get("id").and_then(Value::as_str);
            if id.is_some_and(|id| referenced_span_ids.contains(id)) {
                continue;
            }
            let aliases = id
                .and_then(|id| span_glyph_ids.get(id))
                .cloned()
                .unwrap_or_default();
            insert_node(&mut output, id, aliases);
        }
        for glyph in page
            .get("glyphs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = glyph.get("id").and_then(Value::as_str);
            let meaningful = glyph
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty());
            if !meaningful || id.is_some_and(|id| referenced_glyph_ids.contains(id)) {
                continue;
            }
            insert_node(&mut output, id, Vec::new());
        }
        let mut table_border_path_ids = BTreeSet::new();
        for table in page
            .get("tables")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut aliases = Vec::new();
            for cell in table
                .get("cells")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(cell_id) = cell.get("cellId").and_then(Value::as_str) {
                    aliases.push(cell_id.to_string());
                }
                let content_region_ids = string_values(cell.get("contentRegionIds"));
                aliases.extend(content_region_ids.iter().cloned());
                aliases.extend(content_region_ids.iter().flat_map(|region_id| {
                    region_aliases.get(region_id).into_iter().flatten().cloned()
                }));
                let border_ids = string_values(cell.get("borderEvidence"));
                table_border_path_ids.extend(border_ids.iter().cloned());
                aliases.extend(border_ids);
            }
            insert_node(
                &mut output,
                table.get("id").and_then(Value::as_str),
                aliases,
            );
        }
        for path in page
            .get("vectorPaths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let path_id = path.get("id").and_then(Value::as_str);
            if path.get("isAxisAlignedRule").and_then(Value::as_bool) == Some(true)
                || path_id.is_some_and(|id| table_border_path_ids.contains(id))
            {
                continue;
            }
            insert_node(&mut output, path_id, Vec::new());
        }
        for field in ["annotations", "imagePlacements"] {
            for item in page
                .get(field)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let aliases = item
                    .get("assetId")
                    .and_then(Value::as_str)
                    .map(|id| vec![id.to_string()])
                    .unwrap_or_default();
                insert_node(&mut output, item.get("id").and_then(Value::as_str), aliases);
            }
        }
        for item in page
            .get("markedContent")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let meaningful = ["actualText", "altText"].iter().any(|key| {
                item.get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.trim().is_empty())
            });
            if meaningful {
                insert_node(
                    &mut output,
                    item.get("id").and_then(Value::as_str),
                    Vec::new(),
                );
            }
        }
    }
    for asset in physical
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        insert_node(
            &mut output,
            asset.get("assetId").and_then(Value::as_str),
            Vec::new(),
        );
    }
    output
}

fn issue(
    code: &str,
    severity: &str,
    message: &str,
    target_type: &str,
    target_id: &str,
    source_anchors: Vec<Value>,
    suggested_actions: Vec<&str>,
) -> Value {
    // `issueId` must identify a *fact*, not just (code, target). The previous
    // scheme `phase4-{code}-{target_id}` collapsed two genuinely different
    // problems reported against the same target into one id (e.g. the two
    // `SLOT_HOST_MISSING` variants in `validate_completion_host`, or a code
    // emitted once per question number). The frontend de-duplicated them into a
    // single row and silently swallowed a real issue.
    //
    // The slug is a deterministic hash of the full discriminating payload, so:
    //   (1) two different facts on the same target get different ids; and
    //   (2) the same fact recomputed on a later save yields an *identical* id,
    // which is what lets `preserve_issue_resolutions` carry resolution/status
    // forward instead of losing it on every quality recompute.
    let fact = format!(
        "{code}|{target_type}|{target_id}|{severity}|{message}|{}",
        suggested_actions.join(",")
    );
    let slug = issue_id_slug(&fact);
    json!({
        "issueId": format!("phase4-{code}-{target_id}-{slug}"),
        "code": code,
        "severity": severity,
        "message": message,
        "targetType": target_type,
        "targetId": target_id,
        "sourceAnchors": source_anchors,
        "suggestedActions": suggested_actions
    })
}

/// Deterministic FNV-1a 64-bit digest, rendered as hex. Two equal fact strings
/// always produce the same slug, so fact-based `issueId`s are stable across
/// recomputes. The slug carries no randomness.
fn issue_id_slug(fact: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in fact.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// §6.8 / §6.11: the local recognition graph's hard closures, carried on the
/// document as `recognitionBlockers`.
///
/// **逐目标重新判断，不重发冻结裁决。** `recognitionBlockers` 是导入那一刻、本地识别
/// 针对冻结原文给出的判决。云端修复随后可能已经真的补齐了那个题组的题面 / 选项库 /
/// 缺答——继续把旧裁决原样当阻塞重发，会让已经修好的问题永远挂在用户待办里（用户被
/// 叫去做一件系统已经做完的事），「修复完成」也就永远达不成。
///
/// 但也不能反过来一概清空：来源覆盖类阻塞（显著源区域未被解释、图区需 OCR、视觉资产
/// 未能物化）说的是**原文本身**没被解释干净——改稿子改不掉它，只有重新看原文才能确认。
/// 这类一律保留。
///
/// 判据严格单向：只有能**仅凭当前 canonical 证明条件已满足**时才移除该条；证明不了就
/// 保留。未知 code 一律保留（「不认识」不等于「已修好」）。模型也不能靠 `resolveIssue`
/// 抹掉它们——`cloud_repair::tools::MODEL_ALLOWED_OPS` 有意不含该命令。
fn validate_recognition_blockers(
    authoring: &Value,
    gate_enabled: bool,
    issues: &mut Vec<Value>,
    hard_failures: &mut Vec<String>,
) {
    if !gate_enabled {
        return;
    }
    for (code, target) in recognition_blocker_entries(authoring) {
        if blocker_condition_satisfied_on_current_draft(authoring, &code, &target) {
            continue;
        }
        let (target_type, target_id) = blocker_issue_target(authoring, &target);
        // `details.blockerCode` 让评审面板能区分「识别阻塞」与「质量校验给出的同名事实」：
        // 同一个 code（例如 PROMPT_EMPTY）可能既由识别图给出、又由本文件的逐组校验给出，
        // 两者的事实与处置不同，合并待办时必须分得开。
        let mut blocker_issue = issue(
            &code,
            "blocking",
            &format!("本地识别未能在原文中确认「{target}」的完整题面结构，发布前需人工核对。"),
            target_type,
            &target_id,
            Vec::new(),
            vec!["assign_role", "edit_text"],
        );
        blocker_issue["details"] = json!({
            "blockerCode": code,
            "blockerTarget": target,
        });
        push_issue(issues, hard_failures, blocker_issue);
    }
}

/// 把 `recognitionBlockers` / `recognitionBlockerTargets` 摊平成 `(code, target)`。
///
/// 优先用带目标的版本：它能把阻塞指到具体题组 / 题号上，用户才知道该去核哪一处。
/// 没有配目标的 code **不能因此被丢掉**——挂到 `document` 上，与旧行为一致（旧实现把
/// 所有 code 都当文档级阻塞）。
fn recognition_blocker_entries(authoring: &Value) -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    if let Some(targets) = authoring
        .get("recognitionBlockerTargets")
        .and_then(Value::as_array)
    {
        for item in targets {
            let (Some(code), Some(target)) = (
                item.get("code").and_then(Value::as_str),
                item.get("target").and_then(Value::as_str),
            ) else {
                continue;
            };
            if seen.insert((code.to_string(), target.to_string())) {
                entries.push((code.to_string(), target.to_string()));
            }
        }
    }
    let covered: BTreeSet<String> = entries.iter().map(|(code, _)| code.clone()).collect();
    if let Some(codes) = authoring
        .get("recognitionBlockers")
        .and_then(Value::as_array)
    {
        for code in codes.iter().filter_map(Value::as_str) {
            if covered.contains(code) {
                continue;
            }
            if seen.insert((code.to_string(), "document".to_string())) {
                entries.push((code.to_string(), "document".to_string()));
            }
        }
    }
    entries
}

/// blocker 的目标 → 质量报告的 `(targetType, targetId)`。
///
/// 能解析成 canonical 里的对象就用真实对象（`slot` / `task`），评审面板才指得到具体
/// 位置；解析不出来（旧文档没有 `recognitionBlockerTargets`，或目标只存在于物理文档里）
/// 就退回文档级——**绝不因为指不到位置就把阻塞丢掉**。
fn blocker_issue_target(authoring: &Value, target: &str) -> (&'static str, String) {
    if let Some(slot_id) = slot_id_for_blocker_target(authoring, target) {
        return ("slot", slot_id);
    }
    let is_task = authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .is_some_and(|groups| {
            groups
                .iter()
                .any(|group| group.get("taskId").and_then(Value::as_str) == Some(target))
        });
    if is_task {
        return ("task", target.to_string());
    }
    ("document", "recognition".to_string())
}

/// 只有能**仅凭当前 canonical 证明**该 blocker 的内容条件已满足时才返回 `true`。
///
/// 来源覆盖类（`SIGNIFICANT_REGION_UNASSIGNED`、`DIAGRAM_QUESTION_REGION_OCR_REQUIRED`、
/// `VISUAL_FALLBACK_ASSET_NOT_MATERIALIZED`…）与未知 code 一律落到 `_ => false`：它们的
/// 判据不在稿子里，只能靠重新看原文确认，不能因为「稿子看起来没问题」就放行。
fn blocker_condition_satisfied_on_current_draft(
    authoring: &Value,
    code: &str,
    target: &str,
) -> bool {
    match code {
        // 题面为空：目标题组 / 槽位现在有了非空题面。
        PROMPT_EMPTY => blocker_prompt_present(authoring, target),
        // 声明的题号没有对应题块：现在每个声明题号都能在 `answerSlots` 里找到。
        crate::recognition::direct_canonical::QUESTION_BLOCK_MISSING => {
            blocker_declared_questions_all_have_slots(authoring, target)
        }
        // 多选题作答基数没能解析：目标题组现在既有选项库、又有明确的作答基数。
        crate::recognition::direct_canonical::MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED => {
            blocker_option_bank_and_cardinality_resolved(authoring, target)
        }
        // 缺答：目标槽位现在有答案（`unresolved` 不算）。
        ANSWER_KEY_MISSING_SLOT => blocker_target_slots_all_answered(authoring, target),
        _ => false,
    }
}

fn blocker_prompt_present(authoring: &Value, target: &str) -> bool {
    if let Some(group) = task_group_by_blocker_target(authoring, target) {
        let mut text = Vec::new();
        for key in ["instructions", "stimulus"] {
            if let Some(nodes) = group.get(key).and_then(Value::as_array) {
                collect_instruction_node_text(nodes, &mut text);
            }
        }
        for response_group in group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(nodes) = response_group.get("prompt").and_then(Value::as_array) {
                collect_instruction_node_text(nodes, &mut text);
            }
        }
        return !text.is_empty();
    }
    // 槽位题面 = 它所在宿主节点的文本。
    let Some(slot) = slot_by_blocker_target(authoring, target) else {
        return false;
    };
    let Some(host_id) = slot.get("hostNodeId").and_then(Value::as_str) else {
        return false;
    };
    node_text_present(authoring, host_id)
}

fn blocker_declared_questions_all_have_slots(authoring: &Value, target: &str) -> bool {
    let Some(group) = task_group_by_blocker_target(authoring, target) else {
        return false;
    };
    let declared = declared_question_numbers(group);
    // 声明本身读不出来 ⇒ 无法证明「题块齐了」，保留阻塞。
    if declared.is_empty() {
        return false;
    }
    declared
        .iter()
        .all(|number| slot_id_for_question_number(authoring, *number).is_some())
}

fn blocker_option_bank_and_cardinality_resolved(authoring: &Value, target: &str) -> bool {
    let Some(group) = task_group_by_blocker_target(authoring, target) else {
        return false;
    };
    let has_bank = group
        .get("optionBank")
        .and_then(|bank| bank.get("options"))
        .and_then(Value::as_array)
        .is_some_and(|options| !options.is_empty());
    if !has_bank {
        return false;
    }
    group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|response_group| {
            response_group
                .get("cardinality")
                .and_then(Value::as_object)
                .is_some_and(|cardinality| {
                    let exact = cardinality.get("exact").and_then(Value::as_u64);
                    let min = cardinality.get("min").and_then(Value::as_u64);
                    let max = cardinality.get("max").and_then(Value::as_u64);
                    exact.is_some() || (min.is_some() && max.is_some())
                })
        })
}

fn blocker_target_slots_all_answered(authoring: &Value, target: &str) -> bool {
    let Some(slot_id) = slot_id_for_blocker_target(authoring, target) else {
        return false;
    };
    authoring
        .get("answerKey")
        .and_then(Value::as_object)
        .and_then(|key| key.get(&slot_id))
        .is_some_and(answer_present)
}

/// `AnswerValueV2` 是否真的承载了一个答案。`{"kind":"unresolved"}` 与空 labels/values
/// 都不算——否则「把答案标成未解」就能冒充修复。
fn answer_present(answer: &Value) -> bool {
    let non_empty = |items: Option<&Vec<Value>>| {
        items.is_some_and(|items| {
            items
                .iter()
                .any(|item| item.as_str().is_some_and(|item| !item.trim().is_empty()))
        })
    };
    match answer.get("kind").and_then(Value::as_str) {
        Some("option") => non_empty(answer.get("labels").and_then(Value::as_array)),
        Some("text") => non_empty(answer.get("values").and_then(Value::as_array)),
        _ => false,
    }
}

/// 题号 → 题组。`target` 既可以是 `taskId`，也可以是 `q{number}`（题号所属的题组）。
fn task_group_by_blocker_target<'a>(authoring: &'a Value, target: &str) -> Option<&'a Value> {
    let groups = authoring.get("taskGroups").and_then(Value::as_array)?;
    if let Some(group) = groups
        .iter()
        .find(|group| group.get("taskId").and_then(Value::as_str) == Some(target))
    {
        return Some(group);
    }
    let number: u32 = target.strip_prefix('q')?.parse().ok()?;
    groups
        .iter()
        .find(|group| group_owns_question_number(authoring, group, number))
}

fn group_owns_question_number(authoring: &Value, group: &Value, number: u32) -> bool {
    group
        .get("responseGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|response_group| {
            response_group
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str)
        .any(|slot_id| slot_question_number(authoring, slot_id) == Some(number))
}

fn slot_by_blocker_target<'a>(authoring: &'a Value, target: &str) -> Option<&'a Value> {
    let slot_id = slot_id_for_blocker_target(authoring, target)?;
    authoring
        .get("answerSlots")
        .and_then(Value::as_object)?
        .get(&slot_id)
}

/// `q{number}` → canonical 里的真实 `slotId`。非 `q{n}` 形式、或没有对应槽位 ⇒ `None`。
fn slot_id_for_blocker_target(authoring: &Value, target: &str) -> Option<String> {
    let number: u32 = target.strip_prefix('q')?.parse().ok()?;
    slot_id_for_question_number(authoring, number)
}

fn slot_id_for_question_number(authoring: &Value, number: u32) -> Option<String> {
    authoring
        .get("answerSlots")
        .and_then(Value::as_object)?
        .iter()
        .find(|(_, slot)| {
            slot.get("questionNumber").and_then(Value::as_u64) == Some(number as u64)
        })
        .map(|(slot_id, _)| slot_id.clone())
}

fn slot_question_number(authoring: &Value, slot_id: &str) -> Option<u32> {
    authoring
        .get("answerSlots")
        .and_then(Value::as_object)?
        .get(slot_id)?
        .get("questionNumber")
        .and_then(Value::as_u64)
        .map(|number| number as u32)
}

/// 题组声明的题号：优先 `instructionSignature.expectedQuestionNumbers`，退回
/// `displayRange`。两者都读不出来 ⇒ 空（调用方据此保留阻塞）。
fn declared_question_numbers(group: &Value) -> Vec<u32> {
    if let Some(numbers) = group
        .get("instructionSignature")
        .and_then(|signature| signature.get("expectedQuestionNumbers"))
        .and_then(Value::as_array)
    {
        let numbers: Vec<u32> = numbers
            .iter()
            .filter_map(Value::as_u64)
            .map(|number| number as u32)
            .collect();
        if !numbers.is_empty() {
            return numbers;
        }
    }
    let Some(range) = group
        .get("displayRange")
        .and_then(|range| serde_json::from_value::<QuestionNumberExpressionV2>(range.clone()).ok())
    else {
        return Vec::new();
    };
    match range {
        QuestionNumberExpressionV2::Range { start, end } => (start..=end).collect(),
        QuestionNumberExpressionV2::Set { values } => values,
        QuestionNumberExpressionV2::Mixed { values } => values
            .into_iter()
            .flat_map(|value| match value {
                crate::schema::ielts_authoring_v2::QuestionNumberValueV2::Number(number) => {
                    vec![number]
                }
                crate::schema::ielts_authoring_v2::QuestionNumberValueV2::Range { start, end } => {
                    (start..=end).collect()
                }
            })
            .collect(),
    }
}

/// 整份稿里按内容节点 `id` 找节点，返回它的文本是否非空。
///
/// 只认 `id` 键：题组用 `taskId`、响应组用 `responseGroupId`、槽位用 `slotId`、资产用
/// `assetId`，都不会被误命中。
fn node_text_present(value: &Value, node_id: &str) -> bool {
    match value {
        Value::Object(object) => {
            if object.get("id").and_then(Value::as_str) == Some(node_id) {
                return object
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty());
            }
            object.values().any(|child| node_text_present(child, node_id))
        }
        Value::Array(items) => items.iter().any(|item| node_text_present(item, node_id)),
        _ => false,
    }
}

/// Re-derive `instructionSignature` for a task group from its current
/// instruction text. Called after the user edits instruction text (via the
/// `replaceText` patch op in `authoring_v2_commands`) so that quality is judged
/// against the NEW instruction instead of a stale signature.
///
/// The signature is a *derived* artifact. Only the text-derived fields are
/// recomputed here: `normalizedText`, `optionAlphabet`, `selectionCardinality`,
/// `allowOptionReuse`, `wordLimit`, `answerAssignment`, `confidence` and
/// `evidenceAnchors`. User-confirmed question numbering
/// (`expectedQuestionNumbers` / `expectedSlotCount`) is preserved from the
/// previous signature, so an explicit `setQuestionExpression` patch is not
/// lost on a later text edit.
pub(crate) fn derive_instruction_signature_for_group(group: &mut Value) {
    let Some(instructions) = group.get("instructions").and_then(Value::as_array) else {
        return;
    };
    let mut parts = Vec::new();
    collect_instruction_node_text(instructions, &mut parts);
    if parts.is_empty() {
        return;
    }
    let text = parts.join(" ");
    let expression: QuestionNumberExpressionV2 = group
        .get("displayRange")
        .and_then(|value| serde_json::from_value::<QuestionNumberExpressionV2>(value.clone()).ok())
        .unwrap_or(QuestionNumberExpressionV2::Range { start: 1, end: 1 });
    let kind_hint = group.get("taskType").and_then(Value::as_str);
    let evidence_anchors: Vec<Value> = group
        .get("instructionSignature")
        .and_then(|signature| signature.get("evidenceAnchors"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let result = infer_instruction_signature(&text, &expression, kind_hint, evidence_anchors);
    let mut signature_value = match serde_json::to_value(&result.signature) {
        Ok(value) => value,
        Err(_) => return,
    };
    if let Some(old) = group.get("instructionSignature") {
        if let Some(object) = signature_value.as_object_mut() {
            if let Some(numbers) = old.get("expectedQuestionNumbers") {
                object.insert("expectedQuestionNumbers".to_string(), numbers.clone());
            }
            if let Some(count) = old.get("expectedSlotCount") {
                object.insert("expectedSlotCount".to_string(), count.clone());
            }
        }
    }
    if let Some(object) = group.as_object_mut() {
        object.insert("instructionSignature".to_string(), signature_value);
    }
}

fn collect_instruction_node_text(nodes: &[Value], out: &mut Vec<String>) {
    for node in nodes {
        if let Some(text) = node.get("text").and_then(Value::as_str) {
            if !text.trim().is_empty() {
                out.push(text.to_string());
            }
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            collect_instruction_node_text(children, out);
        }
    }
}

fn push_issue(issues: &mut Vec<Value>, hard_failures: &mut Vec<String>, value: Value) {
    let code = value
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if value.get("severity").and_then(Value::as_str) == Some("blocking") {
        if !hard_failures.contains(&code) {
            hard_failures.push(code);
        }
    }
    if !issues.iter().any(|item| item == &value) {
        issues.push(value);
    }
}

fn round(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    // 这两个码定义在识别侧（`direct_canonical`），不在 `issue_codes` 词表里；测试要按
    // 真实码构造阻塞，不能就地复制字面量（否则词表改名时测试会静默失配）。
    use crate::recognition::direct_canonical::{
        MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED, QUESTION_BLOCK_MISSING,
    };

    fn early_approaches() -> Value {
        serde_json::from_str(include_str!(
            "../../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .expect("checked-in PR-06 fixture must be valid JSON")
    }

    fn collect_fixture_node_ids(value: &Value, output: &mut BTreeSet<String>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    collect_fixture_node_ids(item, output);
                }
            }
            Value::Object(object) => {
                for node_id in object
                    .get("sourceAnchors")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .flat_map(|anchor| anchor.get("nodeIds").and_then(Value::as_array))
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    output.insert(node_id.to_string());
                }
                for (key, child) in object {
                    if key != "quality" {
                        collect_fixture_node_ids(child, output);
                    }
                }
            }
            _ => {}
        }
    }

    fn valid_physical_shadow(authoring: &Value) -> Value {
        let mut node_ids = BTreeSet::new();
        collect_fixture_node_ids(authoring, &mut node_ids);
        let source_hash = "a".repeat(64);
        let physical = json!({
            "schemaVersion":"DocumentIRV2",
            "documentId":authoring.get("sourceDocumentId").and_then(Value::as_str).unwrap_or("document-1"),
            "jobId":authoring.get("jobId").and_then(Value::as_str).unwrap_or("job-1"),
            "sourceFiles":[{
                "sourceFileId":"source-pdf-1",
                "originalName":"early-approaches.pdf",
                "mediaType":"application/pdf",
                "sha256":source_hash,
                "byteLength":1,
                "role":"question_paper"
            }],
            "pages":[{
                "pageIndex":0,
                "widthPt":612.0,
                "heightPt":792.0,
                "rotation":0,
                "glyphs":[],
                "spans":[],
                "lines":[],
                "regions":[{
                    "id":"region-question-surface",
                    "kind":"text",
                    "bbox":{"x":10.0,"y":10.0,"width":500.0,"height":200.0,"unit":"pt","origin":"top-left","pageRotation":0},
                    "childLineIds":node_ids,
                    "childObjectIds":[],
                    "confidence":1.0,
                    "sourceAnchors":[{"sourceFileId":"source-pdf-1","pageIndex":0,"nodeIds":["region-question-surface"],"extractionMode":"pdf_native","sourceHash":source_hash}]
                }],
                "vectorPaths":[],
                "tables":[],
                "assetIds":[],
                "readingOrder":["region-question-surface"],
                "quality":{
                    "classification":"born_digital",
                    "nativeCharacterCount":100,
                    "unicodeErrorRatio":0.0,
                    "duplicateTextRatio":0.0,
                    "imageCoverageRatio":0.0,
                    "textCoverageRatio":1.0,
                    "rotationConfidence":1.0,
                    "requiresOcrRegions":[],
                    "warnings":[]
                }
            }],
            "assets":[],
            "coverageLedger":[{"sourceNodeId":"region-question-surface","disposition":"unassigned","targetIds":[],"reason":"semantic assignment is evaluated by QualityReportV2"}],
            "parser":{"provider":"phase4-test","providerVersion":"1","extractionStartedAt":"2026-08-10T00:00:00Z","extractionCompletedAt":"2026-08-10T00:00:01Z","options":{},"warnings":[]}
        });
        serde_json::from_value::<crate::schema::DocumentIRV2>(physical.clone())
            .expect("quality proof physical shadow must be a typed DocumentIRV2");
        physical
    }

    fn issue_for_target(report: &Value, code: &str, target_id: &str) -> bool {
        report
            .get("issues")
            .and_then(Value::as_array)
            .is_some_and(|issues| {
                issues.iter().any(|issue| {
                    issue.get("code").and_then(Value::as_str) == Some(code)
                        && issue.get("targetId").and_then(Value::as_str) == Some(target_id)
                })
            })
    }

    fn hard_failures_of(report: &Value) -> Vec<String> {
        report
            .get("hardFailures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    /// 一份带受管音频的听力稿：一个 part、一条 `user_upload` 音频资产、part media 指向它。
    ///
    /// 直接复用 `test_support::listening_exam`（它按真实上传规则从字节算 sha，并写出
    /// `extractionMode:"user_upload"` 的描述符与「探测通过」的 part media），保证这段夹具
    /// 不会和产品的音频契约脱节。
    fn listening_authoring_with_bound_audio() -> (Value, String) {
        let authoring = serde_json::to_value(crate::test_support::listening_exam(vec![
            crate::test_support::ListeningPartSpec::range(1, 1, 10),
        ]))
        .expect("listening fixture serialises");
        let asset_id = authoring["assets"][0]["assetId"]
            .as_str()
            .expect("fixture carries one audio asset")
            .to_string();
        (authoring, asset_id)
    }

    /// PDF 的 physical shadow：它**没有**这段音频（音频从来不在 PDF 里）。
    fn shadow_without_the_audio() -> Value {
        json!({"schemaVersion":"DocumentIRV2","documentId":"doc-listening","jobId":"job-1","sourceFiles":[],"pages":[],"assets":[]})
    }

    fn facts(entries: &[(&str, ManagedAudioCheckV1)]) -> ManagedAudioFactsV1 {
        ManagedAudioFactsV1::new(
            entries
                .iter()
                .map(|(asset_id, check)| ((*asset_id).to_string(), *check))
                .collect(),
        )
    }

    /// 该资产上「受管音频核对」给出的理由。
    ///
    /// 断言必须走这个字段，不能只断言硬失败码在不在：旧的「拿资产去比 PDF shadow」也会
    /// 报同一个 `ASSET_REFERENCE_MISSING`，只按码断言的话两边都能过——等于没测。
    fn managed_audio_reason(report: &Value, asset_id: &str) -> Option<String> {
        report
            .get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|issue| {
                issue.get("targetId").and_then(Value::as_str) == Some(asset_id)
                    && issue
                        .pointer("/details/managedAudioReason")
                        .and_then(Value::as_str)
                        .is_some()
            })
            .and_then(|issue| {
                issue
                    .pointer("/details/managedAudioReason")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
    }

    /// 已绑定的用户上传音频**不得**再拿去和 PDF 的 physical shadow 比对。
    ///
    /// 旧实现把 `user_upload` 资产和 shadow 里的资产一视同仁地按 assetId 查，音频在 shadow
    /// 里当然不存在，于是每一段音频都报一条 `ASSET_REFERENCE_MISSING`——用户按提示绑好了
    /// 音频，门禁却说他没绑。逐 part 的 `LISTENING_AUDIO_MISSING` / `_PROBE_BLOCKED` 才是
    /// 音频该走的那条判据。
    #[test]
    fn a_bound_user_upload_audio_is_not_compared_against_the_pdf_shadow() {
        let (authoring, asset_id) = listening_authoring_with_bound_audio();
        let report = evaluate_quality_with_managed_audio(
            &authoring,
            Some(&shadow_without_the_audio()),
            Some(&facts(&[(&asset_id, ManagedAudioCheckV1::Verified)])),
        );
        assert!(
            !hard_failures_of(&report).contains(&ASSET_REFERENCE_MISSING.to_string()),
            "已绑定且核对通过的音频不该报 ASSET_REFERENCE_MISSING：{:?}",
            hard_failures_of(&report)
        );
        assert!(
            !hard_failures_of(&report).contains(&ASSET_HASH_MISMATCH.to_string()),
            "sha 一致时不该报 ASSET_HASH_MISMATCH：{:?}",
            hard_failures_of(&report)
        );
    }

    /// 台账里查不到记录 ⇒ 硬失败，且原因要能看出来是「台账里没有」。
    #[test]
    fn an_audio_asset_with_no_ledger_row_is_a_hard_failure() {
        let (authoring, asset_id) = listening_authoring_with_bound_audio();
        let report = evaluate_quality_with_managed_audio(
            &authoring,
            Some(&shadow_without_the_audio()),
            Some(&facts(&[(&asset_id, ManagedAudioCheckV1::NoRecord)])),
        );
        assert!(
            hard_failures_of(&report).contains(&ASSET_REFERENCE_MISSING.to_string()),
            "{report:#}"
        );
        assert_eq!(
            managed_audio_reason(&report, &asset_id).as_deref(),
            Some("no_ledger_record"),
            "必须是受管音频核对给出的结论，不是拿资产去比 PDF shadow：{report:#}"
        );
    }

    /// 台账有行但受管文件不在磁盘上 ⇒ 硬失败（不是「跳过检查」）。
    #[test]
    fn a_missing_managed_audio_file_is_a_hard_failure() {
        let (authoring, asset_id) = listening_authoring_with_bound_audio();
        let report = evaluate_quality_with_managed_audio(
            &authoring,
            Some(&shadow_without_the_audio()),
            Some(&facts(&[(&asset_id, ManagedAudioCheckV1::FileMissing)])),
        );
        assert!(
            hard_failures_of(&report).contains(&ASSET_REFERENCE_MISSING.to_string()),
            "{report:#}"
        );
        assert_eq!(
            managed_audio_reason(&report, &asset_id).as_deref(),
            Some("managed_file_missing"),
            "{report:#}"
        );
    }

    /// 受管文件内容哈希与声明不一致 ⇒ `ASSET_HASH_MISMATCH`。
    #[test]
    fn a_managed_audio_hash_mismatch_is_a_hard_failure() {
        let (authoring, asset_id) = listening_authoring_with_bound_audio();
        let report = evaluate_quality_with_managed_audio(
            &authoring,
            Some(&shadow_without_the_audio()),
            Some(&facts(&[(&asset_id, ManagedAudioCheckV1::HashMismatch)])),
        );
        assert!(
            hard_failures_of(&report).contains(&ASSET_HASH_MISMATCH.to_string()),
            "{report:#}"
        );
        assert_eq!(
            managed_audio_reason(&report, &asset_id).as_deref(),
            Some("hash_mismatch"),
            "{report:#}"
        );
    }

    /// 探测没通过 ⇒ 硬失败（复用逐 part 音频那一条码，不另造词汇）。
    #[test]
    fn a_managed_audio_probe_failure_is_a_hard_failure() {
        let (authoring, asset_id) = listening_authoring_with_bound_audio();
        let report = evaluate_quality_with_managed_audio(
            &authoring,
            Some(&shadow_without_the_audio()),
            Some(&facts(&[(&asset_id, ManagedAudioCheckV1::ProbeBlocked)])),
        );
        assert!(
            hard_failures_of(&report).contains(&LISTENING_AUDIO_PROBE_BLOCKED.to_string()),
            "{report:#}"
        );
        assert_eq!(
            managed_audio_reason(&report, &asset_id).as_deref(),
            Some("probe_blocked"),
            "{report:#}"
        );
    }

    /// 稿里有 `user_upload` 资产却拿不到台账事实 ⇒ 硬失败。
    ///
    /// 「读不到台账」不能等价于「音频没问题」：那正是让一切看起来通过、学生端却打不开的
    /// 那条捷径。宁可如实报「无法核对」，也不给一次假通过。
    #[test]
    fn a_user_upload_asset_without_ledger_facts_fails_closed() {
        let (authoring, asset_id) = listening_authoring_with_bound_audio();
        let report =
            evaluate_quality_with_managed_audio(&authoring, Some(&shadow_without_the_audio()), None);
        assert!(
            hard_failures_of(&report).contains(&ASSET_REFERENCE_MISSING.to_string()),
            "{report:#}"
        );
        assert_eq!(
            managed_audio_reason(&report, &asset_id).as_deref(),
            Some("managed_audio_facts_unavailable"),
            "{report:#}"
        );
    }

    /// part media 声明的 sha 与资产描述符不一致 ⇒ 硬失败（台账说通过也不行）。
    ///
    /// 台账核的是「磁盘上那份文件」，part media 才是学生端会去取的那条引用；两边指的不是
    /// 同一份音频时，播放出来的就不是这一节的内容。
    #[test]
    fn an_audio_asset_whose_part_media_disagrees_on_the_hash_is_a_hard_failure() {
        let (mut authoring, asset_id) = listening_authoring_with_bound_audio();
        authoring["listening"]["parts"][0]["media"]["sha256"] = json!("b".repeat(64));
        let report = evaluate_quality_with_managed_audio(
            &authoring,
            Some(&shadow_without_the_audio()),
            Some(&facts(&[(&asset_id, ManagedAudioCheckV1::Verified)])),
        );
        assert!(
            hard_failures_of(&report).contains(&ASSET_HASH_MISMATCH.to_string()),
            "{report:#}"
        );
        assert!(
            managed_audio_reason(&report, &asset_id)
                .is_some_and(|reason| reason.starts_with("part_media_hash_conflict:")),
            "{report:#}"
        );
    }

    #[test]
    fn quality_gate_blocks_range_only_group_without_slots_and_answers() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {"taskType":"single_choice","expectedQuestionNumbers":[1,2],"confidence":0.95},
                "instructions": [{"type":"text","text":"Choose the correct letter."}],
                "responseGroups": []
            }],
            "answerSlots": {},
            "answerKey": {}
        });
        let report = evaluate_quality(&authoring, None);
        assert_eq!(report.get("state").and_then(Value::as_str), Some("blocked"));
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty()));
    }

    #[test]
    fn quality_gate_reports_review_when_only_source_coverage_is_weak() {
        let authoring = json!({
            "taskGroups": [],
            "answerSlots": {},
            "answerKey": {}
        });
        let report = evaluate_quality(&authoring, None);
        assert_eq!(report.get("state").and_then(Value::as_str), Some("blocked"));
    }

    #[test]
    fn source_question_coverage_catches_a_question_deleted_from_the_canonical_draft() {
        let mut authoring = early_approaches();
        authoring["modality"] = json!("reading");
        authoring["taskGroups"][0]["displayRange"] = json!({"kind":"range","start":14,"end":14});
        authoring["taskGroups"][0]["instructionSignature"]["expectedQuestionNumbers"] = json!([14]);
        authoring["taskGroups"][0]["instructionSignature"]["expectedSlotCount"] = json!(1);
        authoring["taskGroups"][0]["responseGroups"] = json!([{
            "responseGroupId":"early-approaches-q14-15-response-14",
            "slotIds":["q14"],
            "prompt":[{"id":"q14-stem","text":"Which approach?"}]
        }]);
        authoring["answerSlots"].as_object_mut().unwrap().remove("q15");
        authoring["answerKey"].as_object_mut().unwrap().remove("q15");

        let mut physical = valid_physical_shadow(&authoring);
        physical["pages"][0]["lines"] = json!([
            {"id":"declared-range","text":"Questions 14-15"},
            {"id":"declared-boxes","text":"Write your answers in boxes 14-15"}
        ]);

        let report = evaluate_quality(&authoring, Some(&physical));
        assert_eq!(report["questionCoverage"]["status"], "missing", "{report:#}");
        assert_eq!(report["questionCoverage"]["missingQuestionNumbers"], json!([15]));
        assert_eq!(report["state"], "blocked", "{report:#}");
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "SOURCE_QUESTION_COVERAGE_MISSING"));
    }

    #[test]
    fn source_question_coverage_is_undetermined_when_source_declaration_cannot_be_parsed() {
        let authoring = early_approaches();
        let mut physical = valid_physical_shadow(&authoring);
        physical["pages"][0]["lines"] = json!([
            {"id":"unparsed-declaration","text":"Questions are based on the passage below."}
        ]);

        let report = evaluate_quality(&authoring, Some(&physical));
        assert_eq!(report["questionCoverage"]["status"], "undetermined", "{report:#}");
        assert!(!report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "SOURCE_QUESTION_COVERAGE_MISSING"));
        assert!(report["issues"].as_array().unwrap().iter().any(|issue| {
            issue.get("code").and_then(Value::as_str)
                == Some("SOURCE_QUESTION_COVERAGE_UNDETERMINED")
        }));
    }

    #[test]
    fn quality_gate_does_not_count_instruction_text_as_question_prompt() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {"taskType":"single_choice","expectedQuestionNumbers":[1],"confidence":0.95},
                "instructions": [{"type":"text","text":"Choose the correct letter."}],
                "responseGroups": [{
                    "responseGroupId":"task-1-response-1",
                    "slotIds":["q1"],
                    "options": [{"content":[{"text":"A choice"}]}]
                }]
            }],
            "answerSlots": {"q1":{"questionNumber":1}},
            "answerKey": {"q1":{"kind":"option","labels":["A"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| { items.iter().any(|item| item.as_str() == Some(PROMPT_EMPTY)) }));
    }

    #[test]
    fn quality_gate_rejects_answer_label_outside_renderable_options() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {"taskType":"single_choice","expectedQuestionNumbers":[1],"confidence":0.95},
                "stimulus": [{"type":"paragraph","text":"Select the correct answer."}],
                "responseGroups": [{
                    "responseGroupId":"task-1-response-1",
                    "slotIds":["q1"],
                    "options": [
                        {"label":"A","content":[{"text":"First choice"}]},
                        {"label":"B","content":[{"text":"Second choice"}]}
                    ]
                }]
            }],
            "answerSlots": {"q1":{"questionNumber":1}},
            "answerKey": {"q1":{"kind":"option","labels":["C"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some(ANSWER_OPTION_NOT_IN_BANK))
            }));
    }

    #[test]
    fn quality_gate_rejects_completion_answer_over_word_limit() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {
                    "taskType":"sentence_completion",
                    "expectedQuestionNumbers":[1],
                    "wordLimit":{"maxWords":1},
                    "confidence":0.95
                },
                "stimulus": [{"type":"paragraph","text":"Complete the sentence."}],
                "responseGroups": [{"responseGroupId":"task-1-response-1","slotIds":["q1"]}]
            }],
            "answerSlots": {"q1":{"questionNumber":1,"hostNodeId":"paragraph-1"}},
            "answerKey": {"q1":{"kind":"text","values":["two words"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some(ANSWER_WORD_LIMIT_VIOLATION))
            }));
    }

    #[test]
    fn quality_gate_rejects_diagram_task_without_figure_stimulus() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {
                    "taskType":"diagram_label_completion",
                    "expectedQuestionNumbers":[1],
                    "wordLimit":{"maxWords":1},
                    "confidence":0.95
                },
                "stimulus": [{"type":"paragraph","text":"Label the diagram."}],
                "responseGroups": [{"responseGroupId":"task-1-response-1","slotIds":["q1"]}]
            }],
            "answerSlots": {"q1":{"questionNumber":1,"hostNodeId":"figure-1"}},
            "answerKey": {"q1":{"kind":"text","values":["label"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.as_str() == Some(ASSET_REFERENCE_MISSING))
            }));
    }

    #[test]
    fn quality_gate_rejects_diagram_task_without_closed_hotspots() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {"taskType":"diagram_label_completion","expectedQuestionNumbers":[1],"wordLimit":{"maxWords":1},"confidence":0.95},
                "stimulus": [{"type":"diagram","id":"figure-1","hotspots":[]}],
                "responseGroups": [{"responseGroupId":"task-1-response-1","slotIds":["q1"]}]
            }],
            "answerSlots": {"q1":{"questionNumber":1,"hostNodeId":"hotspot-1","hostType":"figure_hotspot"}},
            "answerKey": {"q1":{"kind":"text","values":["label"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| items
                .iter()
                .any(|item| item.as_str() == Some(HOTSPOT_GEOMETRY_INVALID))));
    }

    #[test]
    fn quality_gate_rejects_shared_response_policy_mismatch() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "instructionSignature": {"taskType":"multiple_choice","expectedQuestionNumbers":[1,2],"optionAlphabet":"A-D","selectionCardinality":{"min":2,"max":2,"exact":2},"confidence":0.95},
                "stimulus": [{"type":"paragraph","text":"Choose two factors."}],
                "responseGroups": [{"responseGroupId":"task-1-response-1","slotIds":["q1","q2"],"options":[{"label":"A","content":[{"text":"A"}]},{"label":"B","content":[{"text":"B"}]},{"label":"C","content":[{"text":"C"}]},{"label":"D","content":[{"text":"D"}]}],"cardinality":{"min":2,"max":2,"exact":2},"assignment":"per_slot","allowOptionReuse":false}]
            }],
            "answerSlots": {"q1":{"questionNumber":1},"q2":{"questionNumber":2}},
            "answerKey": {"q1":{"kind":"option","labels":["A"]},"q2":{"kind":"option","labels":["B"]}}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(report
            .get("hardFailures")
            .and_then(Value::as_array)
            .is_some_and(|items| items
                .iter()
                .any(|item| item.as_str() == Some(RESPONSE_GROUP_POLICY_MISMATCH))));
    }

    #[test]
    fn per_slot_cardinality_is_valid_for_multiple_slots_when_each_slot_takes_one_answer() {
        let authoring = json!({
            "taskGroups": [{
                "taskId":"task-1",
                "taskType":"summary_completion",
                "instructionSignature": {
                    "taskType":"summary_completion",
                    "expectedQuestionNumbers":[1,2],
                    "selectionCardinality":{"min":1,"max":1,"exact":1},
                    "wordLimit":{"maxWords":1,"maxNumbers":1,"wordsAndOrNumber":true},
                    "confidence":0.95
                },
                "stimulus": [{"type":"paragraph","text":"Complete both gaps."}],
                "responseGroups": [{
                    "responseGroupId":"task-1-response-1",
                    "slotIds":["q1","q2"],
                    "cardinality":{"min":1,"max":1,"exact":1},
                    "assignment":"per_slot",
                    "scoringPolicy":"per_slot_ielts_normalized",
                    "allowOptionReuse":false
                }]
            }],
            "answerSlots": {
                "q1":{"questionNumber":1,"hostNodeId":"p1","participation":"scoring"},
                "q2":{"questionNumber":2,"hostNodeId":"p2","participation":"scoring"}
            },
            "answerKey": {
                "q1":{"kind":"text","values":["one"]},
                "q2":{"kind":"text","values":["two"]}
            }
        });
        let report = evaluate_quality(&authoring, None);
        assert!(!report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == RESPONSE_GROUP_POLICY_MISMATCH));
    }

    #[test]
    fn unresolved_scoring_and_scored_example_use_stable_blocking_codes() {
        let baseline = early_approaches();
        let physical = valid_physical_shadow(&baseline);
        let mut missing_policy = baseline.clone();
        missing_policy["taskGroups"][0]["responseGroups"][0]
            .as_object_mut()
            .unwrap()
            .remove("scoringPolicy");
        let report = evaluate_quality(&missing_policy, Some(&physical));
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == SCORING_POLICY_UNRESOLVED));

        let mut example = baseline;
        example["answerSlots"]["q14"]["participation"] = json!("example");
        let report = evaluate_quality(&example, Some(&physical));
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == EXAMPLE_SCORING_CONFLICT));
    }

    #[test]
    fn source_coverage_uses_unique_physical_ledger_ids() {
        let authoring = json!({"sourceAnchors":[{"nodeIds":["line-1"]}]});
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[{"regions":[],"lines":[{"id":"line-1"},{"id":"line-2"}]}],
            "assets":[],
            "coverageLedger":[
                {"sourceNodeId":"line-1","disposition":"unassigned","targetIds":[]},
                {"sourceNodeId":"line-2","disposition":"unassigned","targetIds":[]}
            ]
        });
        assert_eq!(calculate_source_coverage(&authoring, Some(&physical)), 0.5);
        let report = evaluate_quality(&authoring, Some(&physical));
        let details = report
            .get("issues")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|issue| {
                issue.get("code").and_then(Value::as_str) == Some(SIGNIFICANT_REGION_UNASSIGNED)
            })
            .and_then(|issue| issue.get("details"))
            .unwrap();
        assert_eq!(
            details.get("unassignedSourceNodeIds"),
            Some(&json!(["line-2"]))
        );
    }

    #[test]
    fn source_ownership_blocks_the_same_physical_line_in_two_tasks() {
        let authoring = json!({
            "taskGroups": [
                {"taskId":"task-1","sourceAnchors":[{"nodeIds":["line-shared"]}]},
                {"taskId":"task-2","sourceAnchors":[{"nodeIds":["line-shared"]}]}
            ]
        });
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[{"lines":[{"id":"line-shared","text":"Question source"}],"regions":[]}]
        });
        let mut issues = Vec::new();
        let mut hard_failures = Vec::new();
        validate_source_ownership(&authoring, Some(&physical), &mut issues, &mut hard_failures);
        assert!(hard_failures
            .iter()
            .any(|code| code == SOURCE_OWNERSHIP_CONFLICT));
    }

    #[test]
    fn completion_duplicate_slot_nodes_are_blocking() {
        let group = json!({
            "taskId":"task-1",
            "responseGroups":[{
                "slotIds":["q1"],
                "prompt":[
                    {"type":"paragraph","id":"host-1","children":[{"type":"answer_slot","slotId":"q1"}]},
                    {"type":"paragraph","id":"host-2","children":[{"type":"answer_slot","slotId":"q1"}]}
                ]
            }]
        });
        let slots = serde_json::from_value::<Map<String, Value>>(json!({
            "q1":{"slotId":"q1","hostNodeId":"host-1","hostType":"paragraph"}
        }))
        .unwrap();
        let mut issues = Vec::new();
        let mut hard_failures = Vec::new();
        validate_completion_host(
            &group,
            &slots,
            "task-1",
            Vec::new(),
            &mut issues,
            &mut hard_failures,
        );
        assert!(hard_failures.iter().any(|code| code == SLOT_HOST_DUPLICATE));
    }

    #[test]
    fn completion_requires_inline_slot_closure_in_canonical_stimulus() {
        let group = json!({
            "taskId":"task-closure",
            "instructionSignature": {
                "taskType":"summary_completion",
                "expectedQuestionNumbers":[1,2],
                "wordLimit":{"maxWords":1},
                "confidence":0.95
            },
            "stimulus":[{"type":"paragraph","id":"stimulus-1","children":[{"type":"text","text":"A summary without its gaps."}]}],
            "responseGroups":[{"responseGroupId":"response-closure","slotIds":["q1","q2"],"prompt":[]}]
        });
        let slots = serde_json::from_value::<Map<String, Value>>(json!({
            "q1":{"slotId":"q1","hostNodeId":"stimulus-1","hostType":"paragraph"},
            "q2":{"slotId":"q2","hostNodeId":"stimulus-1","hostType":"paragraph"}
        }))
        .unwrap();
        let mut issues = Vec::new();
        let mut hard_failures = Vec::new();
        validate_completion_host(
            &group,
            &slots,
            "task-closure",
            Vec::new(),
            &mut issues,
            &mut hard_failures,
        );
        assert!(hard_failures.iter().any(|code| code == SLOT_HOST_MISSING));
        assert!(issues.iter().any(|value| {
            value
                .get("details")
                .and_then(|details| details.get("missingInlineSlotIds"))
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.len() == 2)
        }));
    }

    #[test]
    fn source_coverage_blocks_orphan_spans_and_glyphs() {
        let authoring = json!({"sourceAnchors":[{"nodeIds":["line-1"]}]});
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[{
                "regions":[],
                "lines":[{"id":"line-1","spanIds":["span-1"]}],
                "spans":[
                    {"id":"span-1","glyphIds":["glyph-1"]},
                    {"id":"span-orphan","glyphIds":["glyph-2"]}
                ],
                "glyphs":[
                    {"id":"glyph-1","text":"A"},
                    {"id":"glyph-2","text":"B"},
                    {"id":"glyph-orphan","text":"C"},
                    {"id":"glyph-whitespace","text":" "}
                ]
            }],
            "assets":[]
        });

        let summary = source_coverage_summary(&authoring, Some(&physical));
        assert_eq!(summary.significant_count, 3);
        assert_eq!(summary.assigned_count, 1);
        assert_eq!(
            summary.unassigned_ids,
            vec!["glyph-orphan".to_string(), "span-orphan".to_string()]
        );
        assert!(!summary
            .unassigned_ids
            .contains(&"glyph-whitespace".to_string()));
        assert!((summary.score - (1.0 / 3.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn table_regions_expand_to_glyphs_and_border_paths_are_not_separate_orphans() {
        let authoring = json!({"sourceAnchors":[{"nodeIds":["glyph-1"]}]});
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[{
                "glyphs":[{"id":"glyph-1"}],
                "spans":[{"id":"span-1","glyphIds":["glyph-1"]}],
                "lines":[{"id":"line-1","spanIds":["span-1"]}],
                "regions":[{"id":"region-1","kind":"table","childLineIds":["line-1"],"childObjectIds":[]}],
                "tables":[{"id":"table-1","cells":[{
                    "cellId":"cell-1",
                    "contentRegionIds":["region-1"],
                    "borderEvidence":["path-border"]
                }]}],
                "vectorPaths":[{"id":"path-border","isAxisAlignedRule":false}]
            }],
            "assets":[]
        });

        let summary = source_coverage_summary(&authoring, Some(&physical));
        assert_eq!(summary.significant_count, 2);
        assert_eq!(summary.assigned_count, 2);
        assert!(summary.unassigned_ids.is_empty());
    }

    #[test]
    fn source_coverage_explains_narrow_empty_tables_and_ocr_answer_notes() {
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[
                {
                    "quality":{"classification":"born_digital","requiresOcrRegions":[]},
                    "lines":[{"id":"blank-line","text":""}],
                    "regions":[{"id":"empty-table","kind":"table","bbox":{"width":3.0,"height":12.0},"childLineIds":["blank-line"],"childObjectIds":["table-1"]}]
                },
                {
                    "quality":{"classification":"scanned","requiresOcrRegions":[{}]},
                    "lines":[{"id":"note-line","text":"Q3答案可能有争议，争议分析见下页"}],
                    "regions":[{"id":"answer-note","kind":"text","childLineIds":["note-line"],"childObjectIds":[]}]
                }
            ],
            "assets":[]
        });
        let summary = source_coverage_summary(&json!({}), Some(&physical));
        assert_eq!(summary.significant_count, 2);
        assert_eq!(summary.assigned_count, 2);
        assert!(summary.unassigned_ids.is_empty());
        assert!(summary
            .ledger
            .iter()
            .all(|entry| entry.get("disposition").and_then(Value::as_str)
                == Some("ignored_with_reason")));
    }

    #[test]
    fn matching_authoring_asset_descriptor_closes_the_physical_asset() {
        let authoring = json!({"assets":[{"assetId":"asset-1"}]});
        let physical = json!({
            "schemaVersion":"DocumentIRV2","documentId":"document-1","jobId":"job-1",
            "sourceFiles":[{"sourceFileId":"source-1"}],
            "pages":[{}],
            "assets":[{"assetId":"asset-1"}]
        });
        let summary = source_coverage_summary(&authoring, Some(&physical));
        assert_eq!(summary.significant_count, 1);
        assert_eq!(summary.assigned_count, 1);
        assert!(summary.unassigned_ids.is_empty());
    }

    #[test]
    fn unsafe_assets_and_missing_references_are_blocking() {
        let authoring = json!({
            "taskGroups": [],
            "answerSlots": {},
            "answerKey": {},
            "assets": [{"assetId":"figure-1","relativePath":"../figure.png","sha256":"bad"}],
            "passage": [{"type":"image","assetId":"missing"}]
        });
        let report = evaluate_quality(&authoring, None);
        let hard = report
            .get("hardFailures")
            .and_then(Value::as_array)
            .unwrap();
        assert!(hard
            .iter()
            .any(|item| item.as_str() == Some(ASSET_PATH_UNSAFE)));
        assert!(hard
            .iter()
            .any(|item| item.as_str() == Some(ASSET_REFERENCE_MISSING)));
    }

    #[test]
    fn ready_fixture_proves_coverage_compilers_and_shared_v1_semantics() {
        let authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        let report = evaluate_quality(&authoring, Some(&physical));
        assert_eq!(report["state"], "ready", "{report:#}");
        assert_eq!(report["coverageStatus"]["complete"], true);
        assert_eq!(report["compilerProbes"]["v2Runtime"]["status"], "passed");
        assert_eq!(
            report["compilerProbes"]["v1Compatibility"]["status"],
            "passed"
        );
        serde_json::from_value::<crate::schema::QualityReportV2>(report.clone())
            .expect("ready report must deserialize as QualityReportV2");

        let v1 = compile_v1_compatibility_shadow(&authoring);
        let html = v1["questionGroups"][0]["bodyHtml"].as_str().unwrap();
        assert_eq!(html.matches("type=\"checkbox\"").count(), 5);
        assert_eq!(html.matches("shared-response").count(), 1);
        assert!(!html.contains("<select"));
    }

    #[test]
    fn missing_malformed_and_empty_physical_shadows_cannot_be_ready() {
        let authoring = early_approaches();
        for physical in [
            None,
            Some(json!({"schemaVersion":"DocumentIRV2"})),
            Some({
                let mut empty = valid_physical_shadow(&authoring);
                empty["pages"][0]["regions"] = json!([]);
                empty["coverageLedger"] = json!([]);
                empty
            }),
        ] {
            let report = evaluate_quality(&authoring, physical.as_ref());
            assert_eq!(report["state"], "review_required", "{report:#}");
            assert_eq!(report["coverageStatus"]["physicalShadow"], "missing");
        }
    }

    #[test]
    fn unassigned_significant_region_is_blocking_but_child_anchor_assigns_parent_region() {
        let authoring = early_approaches();
        let assigned = valid_physical_shadow(&authoring);
        assert_eq!(
            evaluate_quality(&authoring, Some(&assigned))["state"],
            "ready"
        );

        let mut unassigned = assigned;
        unassigned["pages"][0]["regions"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id":"region-unassigned","kind":"figure",
                "bbox":{"x":10.0,"y":300.0,"width":100.0,"height":100.0,"unit":"pt","origin":"top-left","pageRotation":0},
                "childLineIds":[],"childObjectIds":[],"confidence":1.0,
                "sourceAnchors":[{"sourceFileId":"source-pdf-1","pageIndex":0,"nodeIds":["region-unassigned"],"extractionMode":"pdf_native","sourceHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]
            }));
        let report = evaluate_quality(&authoring, Some(&unassigned));
        assert_eq!(report["state"], "blocked");
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == SIGNIFICANT_REGION_UNASSIGNED));
    }

    #[test]
    fn quality_hard_invariant_mutations_have_stable_issue_codes() {
        let baseline = early_approaches();
        let physical = valid_physical_shadow(&baseline);

        let mut invalid_exam = baseline.clone();
        invalid_exam["exam"]["examId"] = json!("../unsafe");
        assert!(
            evaluate_quality(&invalid_exam, Some(&physical))["hardFailures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|code| code == EXAM_ID_INVALID)
        );

        let mut no_passage = baseline.clone();
        no_passage["passage"]["content"] = json!([]);
        assert!(
            evaluate_quality(&no_passage, Some(&physical))["hardFailures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|code| code == PASSAGE_CONTENT_MISSING)
        );

        let mut bad_count = baseline.clone();
        bad_count["taskGroups"][0]["instructionSignature"]["expectedSlotCount"] = json!(3);
        assert!(
            evaluate_quality(&bad_count, Some(&physical))["hardFailures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|code| code == CARDINALITY_SLOT_MISMATCH)
        );

        let mut missing_provenance = baseline.clone();
        missing_provenance["answerSlots"]["q14"]["sourceAnchors"] = json!([]);
        let report = evaluate_quality(&missing_provenance, Some(&physical));
        assert!(issue_for_target(&report, PROVENANCE_MISSING, "q14"));

        let mut manual_provenance = missing_provenance;
        manual_provenance["answerSlots"]["q14"]["provenanceStatus"] = json!("manual");
        let report = evaluate_quality(&manual_provenance, Some(&physical));
        assert!(!issue_for_target(&report, PROVENANCE_MISSING, "q14"));
    }

    #[test]
    fn instruction_and_signature_provenance_are_required_for_ready() {
        let baseline = early_approaches();
        let physical = valid_physical_shadow(&baseline);
        let task_id = baseline["taskGroups"][0]["taskId"]
            .as_str()
            .unwrap()
            .to_owned();

        let ready = evaluate_quality(&baseline, Some(&physical));
        assert_eq!(ready["state"], "ready", "{ready:#}");
        assert!(!issue_for_target(
            &ready,
            INSTRUCTION_PROVENANCE_MISSING,
            &task_id
        ));
        assert!(!issue_for_target(
            &ready,
            INSTRUCTION_SIGNATURE_EVIDENCE_MISSING,
            &task_id
        ));

        let mut empty_instructions = baseline.clone();
        empty_instructions["taskGroups"][0]["instructions"] = json!([]);
        let report = evaluate_quality(&empty_instructions, Some(&physical));
        assert_eq!(report["state"], "blocked");
        assert!(issue_for_target(
            &report,
            INSTRUCTION_PROVENANCE_MISSING,
            &task_id
        ));

        let mut unanchored_instruction = baseline.clone();
        unanchored_instruction["taskGroups"][0]["instructions"][0]["sourceAnchors"] = json!([]);
        let report = evaluate_quality(&unanchored_instruction, Some(&physical));
        assert_eq!(report["state"], "blocked");
        assert!(issue_for_target(
            &report,
            INSTRUCTION_PROVENANCE_MISSING,
            &task_id
        ));

        let mut empty_signature_evidence = baseline;
        empty_signature_evidence["taskGroups"][0]["instructionSignature"]["evidenceAnchors"] =
            json!([]);
        let report = evaluate_quality(&empty_signature_evidence, Some(&physical));
        assert_eq!(report["state"], "blocked");
        assert!(issue_for_target(
            &report,
            INSTRUCTION_SIGNATURE_EVIDENCE_MISSING,
            &task_id
        ));
    }

    #[test]
    fn duplicate_assets_and_physical_hash_mismatch_are_blocking() {
        let baseline = early_approaches();
        let mut duplicate = baseline.clone();
        let descriptor = json!({
            "assetId": "figure-1",
            "kind": "raster_image",
            "mime": "image/png",
            "relativePath": "assets/figure-1.png",
            "sha256": "b".repeat(64),
            "byteLength": 12,
            "extractionMode": "embedded"
        });
        duplicate["assets"] = json!([descriptor.clone(), descriptor]);
        let report = evaluate_quality(&duplicate, Some(&valid_physical_shadow(&baseline)));
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == ASSET_ID_DUPLICATE));

        let mut mismatched = baseline.clone();
        mismatched["assets"] = json!([{
            "assetId": "figure-1",
            "kind": "raster_image",
            "mime": "image/png",
            "relativePath": "assets/figure-1.png",
            "sha256": "b".repeat(64),
            "byteLength": 12,
            "extractionMode": "embedded"
        }]);
        let mut physical = valid_physical_shadow(&baseline);
        physical["assets"] = json!([{
            "assetId": "figure-1",
            "sha256": "c".repeat(64)
        }]);
        let report = evaluate_quality(&mismatched, Some(&physical));
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == ASSET_HASH_MISMATCH));
    }

    #[test]
    fn recognition_blockers_are_consumed_but_only_block_when_the_gate_is_open() {
        let mut authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);

        // The graph's verdict is carried on the document; without the gate the
        // main chain still records it but publication is not held back.
        authoring["recognitionBlockers"] = json!(["PROMPT_EMPTY"]);
        authoring["recognitionBlockerTargets"] = json!([
            { "code": "PROMPT_EMPTY", "target": "question-block-1" }
        ]);

        let staged_off = evaluate_quality_with_gate(&authoring, Some(&physical), false);
        assert_eq!(staged_off["state"], "ready", "{staged_off:#}");
        assert!(
            !staged_off["hardFailures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|code| code.as_str() == Some("PROMPT_EMPTY")),
            "gate closed must not surface the recognition blocker as a hard failure"
        );

        let staged_on = evaluate_quality_with_gate(&authoring, Some(&physical), true);
        assert_eq!(staged_on["state"], "blocked", "{staged_on:#}");
        assert!(staged_on["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code.as_str() == Some("PROMPT_EMPTY")));
        assert!(issue_for_target(&staged_on, "PROMPT_EMPTY", "recognition"));

        // An absent verdict must never be read as "clean": the gate is a
        // publication decision, not a substitute for the recognition result.
        let mut silent = early_approaches();
        silent.as_object_mut().unwrap().remove("recognitionBlockers");
        let report = evaluate_quality_with_gate(&silent, Some(&physical), true);
        assert_eq!(report["state"], "ready", "{report:#}");
    }

    /// 找一条**识别阻塞**（而不是同名的质量事实）。
    ///
    /// 必须按 `details.blockerCode` 判：同一个 code（如 `PROMPT_EMPTY`）既可能由识别图
    /// 给出、又可能由本文件的逐组校验给出，只看 code + targetId 会把后者误当成前者，
    /// 让测试在实现被删掉之后仍然通过。
    fn blocker_issue_for(report: &Value, code: &str, target_id: &str) -> bool {
        report
            .get("issues")
            .and_then(Value::as_array)
            .is_some_and(|issues| {
                issues.iter().any(|issue| {
                    issue
                        .pointer("/details/blockerCode")
                        .and_then(Value::as_str)
                        == Some(code)
                        && issue.get("targetId").and_then(Value::as_str) == Some(target_id)
                })
            })
    }

    /// 目标真的被修好之后，旧阻塞必须消失——否则「云端自主修复」永远收敛不了，
    /// 用户会被叫去做一件系统已经做完的事。
    ///
    /// 两个方向都要钉：修好 ⇒ 移除；再次弄坏 ⇒ 阻塞回来（证明移除是因为条件真的满足，
    /// 而不是被无条件清空）。
    #[test]
    fn recognition_blocker_is_dropped_once_the_repaired_target_is_really_fixed() {
        let mut authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        let task_id = authoring["taskGroups"][0]["taskId"]
            .as_str()
            .expect("fixture group has a taskId")
            .to_string();
        authoring["recognitionBlockers"] = json!([PROMPT_EMPTY]);
        authoring["recognitionBlockerTargets"] =
            json!([{ "code": PROMPT_EMPTY, "target": task_id }]);

        // 题组现在有题面 ⇒ 条件已满足 ⇒ 不再作为识别阻塞，且指向真实题组。
        let fixed = evaluate_quality_with_gate(&authoring, Some(&physical), true);
        assert!(
            !blocker_issue_for(&fixed, PROMPT_EMPTY, &task_id),
            "已修好的目标不该继续挂识别阻塞: {fixed:#}"
        );

        // 把题面清空 ⇒ 条件重新成立 ⇒ 阻塞回来。
        let mut broken = authoring.clone();
        broken["taskGroups"][0]["instructions"] = json!([]);
        broken["taskGroups"][0]["responseGroups"][0]["prompt"] = json!([]);
        let report = evaluate_quality_with_gate(&broken, Some(&physical), true);
        assert!(
            blocker_issue_for(&report, PROMPT_EMPTY, &task_id),
            "题面为空时阻塞必须回来: {report:#}"
        );
        assert!(
            report["hardFailures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|code| code.as_str() == Some(PROMPT_EMPTY)),
            "{report:#}"
        );
    }

    /// 多选题作答基数：题组既有选项库、又有明确基数 ⇒ 条件已满足。
    #[test]
    fn recognition_blocker_for_multiple_choice_cardinality_is_dropped_when_resolved() {
        let mut authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        let task_id = authoring["taskGroups"][0]["taskId"]
            .as_str()
            .expect("fixture group has a taskId")
            .to_string();
        authoring["recognitionBlockers"] = json!([MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED]);
        authoring["recognitionBlockerTargets"] =
            json!([{ "code": MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED, "target": task_id }]);

        let fixed = evaluate_quality_with_gate(&authoring, Some(&physical), true);
        assert!(
            !blocker_issue_for(&fixed, MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED, &task_id),
            "基数已解析后不该继续挂阻塞: {fixed:#}"
        );

        // 基数被清掉 ⇒ 条件重新成立。
        let mut broken = authoring.clone();
        broken["taskGroups"][0]["responseGroups"][0]["cardinality"] = json!({});
        let report = evaluate_quality_with_gate(&broken, Some(&physical), true);
        assert!(
            blocker_issue_for(&report, MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED, &task_id),
            "基数缺失时必须保留阻塞: {report:#}"
        );
    }

    /// 来源覆盖类与未知 code **不能**因为「稿子看起来没问题」被放行。
    ///
    /// 这类阻塞说的是原文没被解释干净——改稿子改不掉它，只有重新看原文才能确认。
    /// 未知 code 同理：不认识 ≠ 已修好。
    #[test]
    fn recognition_blocker_keeps_source_coverage_and_unknown_codes() {
        let mut authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        authoring["recognitionBlockers"] = json!([SIGNIFICANT_REGION_UNASSIGNED, "SOME_NEW_BLOCKER"]);

        let report = evaluate_quality_with_gate(&authoring, Some(&physical), true);
        assert!(
            blocker_issue_for(&report, SIGNIFICANT_REGION_UNASSIGNED, "recognition"),
            "来源覆盖阻塞必须保留: {report:#}"
        );
        assert!(
            blocker_issue_for(&report, "SOME_NEW_BLOCKER", "recognition"),
            "未知 code 必须保留（不认识 ≠ 已修好）: {report:#}"
        );
    }

    /// 目标解析不出来时退回文档级：绝不因为「指不到具体位置」就把阻塞丢掉。
    ///
    /// 同时锁住旧批次（没有 `recognitionBlockerTargets`）的兼容行为——旧行为就是
    /// 把每个 code 都当文档级阻塞报出来。
    #[test]
    fn recognition_blocker_falls_back_to_document_level_when_target_is_unresolvable() {
        let mut authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        authoring["recognitionBlockers"] = json!([QUESTION_BLOCK_MISSING]);
        authoring["recognitionBlockerTargets"] =
            json!([{ "code": QUESTION_BLOCK_MISSING, "target": "question-block-1" }]);

        let report = evaluate_quality_with_gate(&authoring, Some(&physical), true);
        assert!(
            blocker_issue_for(&report, QUESTION_BLOCK_MISSING, "recognition"),
            "指不到 canonical 对象时退回文档级: {report:#}"
        );

        // 旧批次：完全没有 recognitionBlockerTargets。
        let mut legacy = early_approaches();
        legacy["recognitionBlockers"] = json!([QUESTION_BLOCK_MISSING]);
        let legacy_report = evaluate_quality_with_gate(&legacy, Some(&physical), true);
        assert!(
            blocker_issue_for(&legacy_report, QUESTION_BLOCK_MISSING, "recognition"),
            "旧批次仍须按文档级报出: {legacy_report:#}"
        );
    }

    /// 槽位级阻塞：真的补上答案才移除；把答案标成 `unresolved` 不算补上。
    #[test]
    fn recognition_blocker_for_a_slot_is_dropped_only_when_a_real_answer_exists() {
        let mut authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        authoring["recognitionBlockers"] = json!([ANSWER_KEY_MISSING_SLOT]);
        authoring["recognitionBlockerTargets"] =
            json!([{ "code": ANSWER_KEY_MISSING_SLOT, "target": "q14" }]);

        // fixture 里 q14 有真答案 ⇒ 已满足，且目标解析到真实槽位。
        let fixed = evaluate_quality_with_gate(&authoring, Some(&physical), true);
        assert!(
            !blocker_issue_for(&fixed, ANSWER_KEY_MISSING_SLOT, "q14"),
            "有真答案后不该继续挂缺答阻塞: {fixed:#}"
        );

        // 标成 `unresolved`：明确未解，阻塞必须保留——否则「把答案改成未解」就能冒充修复。
        let mut unresolved = authoring.clone();
        unresolved["answerKey"]["q14"] = json!({ "kind": "unresolved" });
        let report = evaluate_quality_with_gate(&unresolved, Some(&physical), true);
        assert!(
            blocker_issue_for(&report, ANSWER_KEY_MISSING_SLOT, "q14"),
            "`unresolved` 不算有答案: {report:#}"
        );
    }

    /// `q{n}` 目标能解析到题组：声明题号都有槽位 ⇒ 移除；缺一个 ⇒ 保留。
    #[test]
    fn recognition_blocker_for_declared_questions_is_dropped_when_slots_exist() {
        let mut authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        authoring["recognitionBlockers"] = json!([QUESTION_BLOCK_MISSING]);
        authoring["recognitionBlockerTargets"] =
            json!([{ "code": QUESTION_BLOCK_MISSING, "target": "q14" }]);

        let report = evaluate_quality_with_gate(&authoring, Some(&physical), true);
        assert!(
            !blocker_issue_for(&report, QUESTION_BLOCK_MISSING, "q14"),
            "声明题号都有槽位时不该继续挂阻塞: {report:#}"
        );

        // 删掉一个声明题号对应的槽位 ⇒ 条件重新成立。
        let mut missing = authoring.clone();
        missing["answerSlots"]
            .as_object_mut()
            .unwrap()
            .remove("q15");
        let broken = evaluate_quality_with_gate(&missing, Some(&physical), true);
        assert!(
            blocker_issue_for(&broken, QUESTION_BLOCK_MISSING, "q14"),
            "有声明题号缺槽位时必须保留阻塞: {broken:#}"
        );
    }

    #[test]
    fn selection_type_completion_without_word_limit_is_not_blocked() {
        // "Complete the sentences with the correct letter, A-D" draws its
        // answer from a fixed lettered option bank, so no IELTS word limit is
        // expected and the missing word limit must NOT block publishing.
        let authoring = json!({
            "taskGroups": [{
                "taskId": "task-1",
                "instructionSignature": {
                    "taskType": "sentence_completion",
                    "optionAlphabet": "A-D",
                    "expectedQuestionNumbers": [1, 2],
                    "confidence": 0.95
                },
                "instructions": [{"type":"text","text":"Complete the sentences below with the correct letter, A, B, C or D."}],
                "responseGroups": []
            }],
            "answerSlots": {},
            "answerKey": {}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(
            !issue_for_target(&report, WORD_LIMIT_UNPARSED, "task-1"),
            "selection-type completion must not require a word limit: {report:#}"
        );
    }

    #[test]
    fn text_entry_completion_without_word_limit_is_blocked() {
        // A genuine text-entry completion with no resolved word limit is still
        // a structural error and must block.
        let authoring = json!({
            "taskGroups": [{
                "taskId": "task-1",
                "instructionSignature": {
                    "taskType": "sentence_completion",
                    "expectedQuestionNumbers": [1, 2],
                    "confidence": 0.95
                },
                "instructions": [{"type":"text","text":"Complete the sentences below."}],
                "responseGroups": []
            }],
            "answerSlots": {},
            "answerKey": {}
        });
        let report = evaluate_quality(&authoring, None);
        assert!(
            issue_for_target(&report, WORD_LIMIT_UNPARSED, "task-1"),
            "text-entry completion without a word limit must still block: {report:#}"
        );
    }

    #[test]
    fn issue_id_collides_only_when_fact_is_identical() {
        // Two genuinely different facts reported against the SAME target must
        // get DIFFERENT issueIds (the two SLOT_HOST_MISSING variants share code
        // and target but describe different problems).
        let inline = issue(
            SLOT_HOST_MISSING,
            "blocking",
            "canonical stimulus must host each question with a unique inline slot",
            "task",
            "task-1",
            Vec::new(),
            vec!["edit_text"],
        );
        let host = issue(
            SLOT_HOST_MISSING,
            "blocking",
            "completion slot has no renderable host node",
            "task",
            "task-1",
            Vec::new(),
            vec!["edit_text"],
        );
        assert_ne!(inline["issueId"], host["issueId"]);

        // The SAME fact recomputed twice must yield the SAME id, so recorded
        // resolution/status survives every quality recompute.
        let a = issue(
            WORD_LIMIT_UNPARSED,
            "blocking",
            "completion 题组未解析出 IELTS word limit。",
            "task",
            "task-1",
            Vec::new(),
            vec!["edit_text", "confirm_table"],
        );
        let b = issue(
            WORD_LIMIT_UNPARSED,
            "blocking",
            "completion 题组未解析出 IELTS word limit。",
            "task",
            "task-1",
            Vec::new(),
            vec!["edit_text", "confirm_table"],
        );
        assert_eq!(a["issueId"], b["issueId"]);
    }

    // ── 题库保存：原文件在发布后被删除，质量评估改用发布时冻结的证据 ──

    fn frozen_from(authoring: &Value, physical: &Value, declared: &[u32]) -> FrozenSourceEvidence {
        let mut summary = publish_evidence_summary(authoring, Some(physical));
        summary["declaredQuestionNumbers"] = json!(declared);
        summary["declarations"] = json!(["Questions 14-15"]);
        FrozenSourceEvidence::from_value(&summary).expect("frozen evidence must parse")
    }

    #[test]
    fn purged_source_uses_frozen_evidence_instead_of_a_zero_coverage_score() {
        let authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        let frozen = frozen_from(&authoring, &physical, &[14, 15]);
        let report = evaluate_quality_with_frozen_evidence(&authoring, &frozen);
        assert_eq!(report["state"], "ready", "{report:#}");
        assert_eq!(
            report["coverageStatus"]["physicalShadow"],
            "verified_at_publish_source_purged"
        );
        assert_ne!(report["sourceCoverage"], json!(0.0), "不得因原文件被删而报 0.0");
        assert!(report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .all(|issue| issue["code"] != PHYSICAL_SHADOW_MISSING));
        assert_eq!(report["questionCoverage"]["status"], "complete");
        let mut with_quality = authoring.clone();
        with_quality["quality"] = report;
        assert_eq!(quality_readiness(&with_quality), Ok(QualityReadiness::Ready));
    }

    #[test]
    fn purged_source_still_checks_question_numbers_against_the_frozen_declaration() {
        let authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        // 删掉一题：冻结声明 {14,15}，当前稿只剩 14。
        let mut removed = authoring.clone();
        removed["answerSlots"].as_object_mut().unwrap().remove("q15");
        removed["answerKey"].as_object_mut().unwrap().remove("q15");
        let frozen = frozen_from(&authoring, &physical, &[14, 15]);
        let report = evaluate_quality_with_frozen_evidence(&removed, &frozen);
        assert_eq!(report["questionCoverage"]["status"], "missing", "{report:#}");
        assert_eq!(report["questionCoverage"]["missingQuestionNumbers"], json!([15]));
        assert!(report["hardFailures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == SOURCE_QUESTION_COVERAGE_MISSING));
        assert_ne!(report["state"], "ready");

        // 加一题同样被判为不一致。
        let frozen_one = frozen_from(&authoring, &physical, &[14]);
        let report = evaluate_quality_with_frozen_evidence(&authoring, &frozen_one);
        assert_eq!(report["questionCoverage"]["extraQuestionNumbers"], json!([15]));
        assert_ne!(report["state"], "ready");
    }

    #[test]
    fn frozen_evidence_without_a_declaration_stays_undetermined() {
        let authoring = early_approaches();
        let physical = valid_physical_shadow(&authoring);
        let frozen = frozen_from(&authoring, &physical, &[]);
        let report = evaluate_quality_with_frozen_evidence(&authoring, &frozen);
        assert_eq!(report["questionCoverage"]["status"], "undetermined");
    }
}
