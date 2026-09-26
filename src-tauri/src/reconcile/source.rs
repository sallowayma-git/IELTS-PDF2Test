//! 原文件核验（第三条证据链）。
//!
//! 职责边界：**只输出问题与修正建议，不制造第二份权威稿**。
//! 核验依据只有原文件（`DocumentIRV2` 的页文本 / 行 / 区域）与本地稿的
//! source anchors；没有可读文本时一律判 `NotVerifiable`，不得把「两路一致」
//! 当成「已被原文验证」。
//!
//! A3：确定性抽取之上再挂一条**可选的**模型通道（`SourceVerifyRunner`）。
//! 两者的分工是固定的，不可互换：
//! - 确定性结论**先全部产出**，模型只能给「确定性抽不到」的槽位补证据；
//! - 模型**永远不能**推翻一条已经成立的确定性结论（原文里明明可读的答案行，
//!   不该被一次模型幻觉改写）；
//! - 模型给出的结论带 `deterministic = false`，因此**不参与自动写入**
//!   （见 [`SourceVerificationV1::deterministic_suggested`]）。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::candidate::{answer_strings, expand_question_numbers, normalize_text};
use super::engine::{classify_cloud_error, ModelCallFailure, SourceVerifyRunner};
use super::rules::answer_compare_key;
use crate::schema::recognition_v1::{
    reason, ChainKindV1, ChainStatusV1, DecisionEvidenceV1, DecisionFieldV1, DecisionTargetTypeV1,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceVerdictV1 {
    /// 原文件明确支持当前值。
    Confirmed,
    /// 原文件明确给出**不同**的值（本地已有值）。
    Contradicted,
    /// 本地没有值，原文件给出了一个值（可安全补全的低风险情形）。
    Suggested,
    /// 无可读文本或缺少证据面：不得判断。
    NotVerifiable,
}

fn value_is_empty(answer: &Value) -> bool {
    match answer.get("kind").and_then(Value::as_str) {
        Some("text") => answer
            .get("values")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .all(|v| v.trim().is_empty())
            })
            .unwrap_or(true),
        Some("option") => answer
            .get("labels")
            .and_then(Value::as_array)
            .map(|labels| labels.is_empty())
            .unwrap_or(true),
        _ => true,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SourceFindingV1 {
    pub target_type: DecisionTargetTypeV1,
    pub target_id: String,
    pub field: DecisionFieldV1,
    pub verdict: SourceVerdictV1,
    pub reason_code: String,
    pub evidence: Vec<DecisionEvidenceV1>,
    /// 修正建议（仅 `Contradicted` / `Suggested` 时给出），仍是建议而非权威值。
    pub suggested_value: Option<Value>,
    /// 这条结论是**确定性抽取**得到的，还是模型读原文件得到的。
    ///
    /// 必须区分：`suggested_value` 会被 [`crate::reconcile::adjudicate`] 的自动应用
    /// 守卫当作「可靠证据」使用（守卫 3：建议值 == 拟写入值）。模型结论是概率性的，
    /// 让它充当该守卫的输入，等于让一次模型回复直接授权改题稿——与
    /// `adjudicate.rs` 的模块级约束（自动应用只依赖确定性条件）相悖。
    /// 因此自动写入只认 `deterministic == true` 的建议。
    pub deterministic: bool,
}

/// 模型核验通道的运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SourceModelStatusV1 {
    /// 没有可用的模型通道，或本批不需要模型（没有待核验槽位）。
    #[default]
    NotRun,
    /// 调用成功，结论已按「只补确定性缺口」的规则合并。
    Succeeded,
    /// 调用了但没有拿到可用结论（超时 / 非法输出 / 不支持输入）。
    Unusable,
    /// 预算耗尽：本批**没有尝试**调用。
    BudgetExhausted,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SourceVerificationV1 {
    pub status: ChainStatusV1,
    pub reason_code: Option<String>,
    /// A3：模型通道的运行状态。`NotRun` = 从未调用。
    pub model_status: SourceModelStatusV1,
    /// A3：模型通道的稳定原因码（仅在有值可解释时给出，取值来自 [`reason`]）。
    pub model_reason_code: Option<String>,
    /// `key = (targetType, targetId, field)`。
    pub findings: BTreeMap<(DecisionTargetTypeV1, String, DecisionFieldV1), SourceFindingV1>,
    pub page_texts: Vec<String>,
    /// 页文本里出现的裸题号（用于「题号是否真的存在于原文件」）。
    pub question_tokens: BTreeSet<u32>,
    /// 从页文本抽取的答案行：题号 → 答案文本（仅在能被确定性解析时存在）。
    pub answer_rows: BTreeMap<u32, String>,
}

impl SourceVerificationV1 {
    pub(crate) fn finding(
        &self,
        target_type: DecisionTargetTypeV1,
        target_id: &str,
        field: DecisionFieldV1,
    ) -> Option<&SourceFindingV1> {
        self.findings
            .get(&(target_type, target_id.to_string(), field))
    }

    pub(crate) fn is_confirmed(
        &self,
        target_type: DecisionTargetTypeV1,
        target_id: &str,
        field: DecisionFieldV1,
    ) -> bool {
        self.finding(target_type, target_id, field)
            .map(|finding| finding.verdict == SourceVerdictV1::Confirmed)
            .unwrap_or(false)
    }

    pub(crate) fn verdict(
        &self,
        target_type: DecisionTargetTypeV1,
        target_id: &str,
        field: DecisionFieldV1,
    ) -> Option<SourceVerdictV1> {
        self.finding(target_type, target_id, field)
            .map(|finding| finding.verdict)
    }

    /// 原文件给出的建议值（`Contradicted` 或 `Suggested`）。**含模型结论**，用于展示与
    /// 生成待用户确认的 patch。
    pub(crate) fn suggested(
        &self,
        target_type: DecisionTargetTypeV1,
        target_id: &str,
        field: DecisionFieldV1,
    ) -> Option<&Value> {
        self.finding(target_type, target_id, field)
            .filter(|finding| {
                matches!(
                    finding.verdict,
                    SourceVerdictV1::Contradicted | SourceVerdictV1::Suggested
                )
            })
            .and_then(|finding| finding.suggested_value.as_ref())
    }

    /// 模型通道**调用了但没拿到可用结论**时的原因码，用于链状态如实上报。
    ///
    /// 刻意不涵盖其余三种状态：
    /// - `NotRun`：没配模型、或本批没有待核验槽位。改写链原因码会让未配模型的用户
    ///   看到满屏「模型未参与核验」，而真正的原因（原文没有可核验证据）反而被顶掉；
    /// - `BudgetExhausted`：额度/配置问题，同样不该覆盖原文证据的结论；
    /// - `Succeeded`：本来就不需要解释。
    pub(crate) fn unusable_reason(&self) -> Option<&str> {
        if self.model_status != SourceModelStatusV1::Unusable {
            return None;
        }
        Some(
            self.model_reason_code
                .as_deref()
                .unwrap_or(reason::MODEL_INVALID_OUTPUT),
        )
    }

    /// **只**取确定性抽取得到的建议值（排除模型结论）。
    ///
    /// 这是自动应用守卫 3 的唯一合法输入：自动写入必须只依赖确定性条件
    /// ——与 [`crate::reconcile::adjudicate`] 的模块级约束一致。
    /// 用 [`Self::suggested`] 代替它，等于让一次模型回复直接授权改题稿。
    pub(crate) fn deterministic_suggested(
        &self,
        target_type: DecisionTargetTypeV1,
        target_id: &str,
        field: DecisionFieldV1,
    ) -> Option<&Value> {
        self.finding(target_type, target_id, field)
            .filter(|finding| {
                finding.deterministic
                    && matches!(
                        finding.verdict,
                        SourceVerdictV1::Contradicted | SourceVerdictV1::Suggested
                    )
            })
            .and_then(|finding| finding.suggested_value.as_ref())
    }
}

#[allow(clippy::too_many_arguments)]
fn push_finding(
    verification: &mut SourceVerificationV1,
    target_type: DecisionTargetTypeV1,
    target_id: &str,
    field: DecisionFieldV1,
    verdict: SourceVerdictV1,
    reason_code: &str,
    evidence: Vec<DecisionEvidenceV1>,
    suggested_value: Option<Value>,
    deterministic: bool,
) {
    verification.findings.insert(
        (target_type, target_id.to_string(), field),
        SourceFindingV1 {
            target_type,
            target_id: target_id.to_string(),
            field,
            verdict,
            reason_code: reason_code.to_string(),
            evidence,
            suggested_value,
            deterministic,
        },
    );
}

/// 把「原文给出的答案字符串」转成与**本地答案同形状**的 `AnswerValueV2`。
///
/// 形状必须与本地一致：`answer_compare_key` 是形状敏感的（`text` → `values.join("|")`、
/// `option` → `opt:LABELS:assignment`），形状不一致会在**每一道题**上制造假分歧。
/// 确定性抽取与模型结论**共用这一份实现**：两个来源各写一套形状规则，正是假分歧的温床。
///
/// 返回 `None` 表示无法合法映射（本地是选项题、而原文给出的是一句选项文本而非字母标签）：
/// 此时宁可不给建议，也不构造一个形状可疑的候选值。
///
/// **非选项题一律按文本形状产出**，包括本地值是 `unresolved`（或 `kind` 缺失）的情形。
/// 这里刻意不给 `None`：本地无值时不存在「可沿用的形状」可供对齐，而 A3 之前的行为
/// 就是文本形状。若在此返回 `None`，「本地空值 + 原文有可读答案行 ⇒ 建议补全」这条既有
/// 路径会在**未接入模型**的场景下直接失效（自动补空候选清零、依赖组降级用例跟着变红）。
/// 形状对齐的目的是避免与本地既有值比对时制造假分歧；本地没有值时没有这个风险。
fn suggested_in_local_shape(
    answer_kind: &str,
    answer_text: &str,
    assignment: &str,
) -> Option<Value> {
    match answer_kind {
        "option" => {
            let labels = answer_text
                .split_whitespace()
                .map(|token| {
                    token
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .to_uppercase()
                })
                .filter(|token| token.len() == 1 && token.chars().all(|c| c.is_ascii_alphabetic()))
                .collect::<Vec<String>>();
            if labels.is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "kind": "option",
                "labels": labels,
                "assignment": assignment
            }))
        }
        _ => Some(serde_json::json!({"kind": "text", "values": [answer_text]})),
    }
}

/// 按 `findings` 统计答案槽位的确认数与总数。
///
/// 状态必须在**模型结论合并之后**再算：模型把某些槽位从「无法判断」提升为「已确认」后，
/// 链状态若还用合并前的计数，就会出现「结论变强了、状态还说没有证据」。
fn slot_answer_counts(verification: &SourceVerificationV1) -> (usize, usize) {
    let mut confirmed = 0usize;
    let mut total = 0usize;
    for ((target_type, _, field), finding) in &verification.findings {
        if *target_type != DecisionTargetTypeV1::Slot || *field != DecisionFieldV1::Answer {
            continue;
        }
        total += 1;
        if finding.verdict == SourceVerdictV1::Confirmed {
            confirmed += 1;
        }
    }
    (confirmed, total)
}

/// 抽取页文本。`DocumentIRV2` 缺失（或被最小化清理）时返回空列表。
pub(crate) fn page_texts(document_ir: Option<&Value>) -> Vec<String> {
    let Some(document) = document_ir else {
        return Vec::new();
    };
    let Some(pages) = document.get("pages").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut texts = Vec::with_capacity(pages.len());
    for page in pages {
        let mut text = String::new();
        if let Some(lines) = page.get("lines").and_then(Value::as_array) {
            for line in lines {
                if let Some(value) = line.get("text").and_then(Value::as_str) {
                    text.push_str(value);
                    text.push('\n');
                }
            }
        }
        // 行缺失时退回 spans（部分来源只有 span 级文本）。
        if text.trim().is_empty() {
            if let Some(spans) = page.get("spans").and_then(Value::as_array) {
                for span in spans {
                    if let Some(value) = span.get("text").and_then(Value::as_str) {
                        text.push_str(value);
                        text.push(' ');
                    }
                }
            }
        }
        texts.push(text);
    }
    texts
}

fn collect_question_tokens(texts: &[String]) -> BTreeSet<u32> {
    let mut tokens = BTreeSet::new();
    for text in texts {
        for raw in text.split(|c: char| !c.is_ascii_digit()) {
            if raw.is_empty() || raw.len() > 3 {
                continue;
            }
            if let Ok(number) = raw.parse::<u32>() {
                if (1..=200).contains(&number) {
                    tokens.insert(number);
                }
            }
        }
    }
    tokens
}

/// 确定性答案行抽取：`^\s*(\d{1,3})\s*[.)\]:-]?\s+(.+)$`。
///
/// 这不是 OCR，也不是模型推断：只针对**原文件里已经可读**的答案表。
/// 页文本不可读（扫描答案页）时结果为空，核验因此保持 `NotVerifiable`。
fn collect_answer_rows(texts: &[String]) -> BTreeMap<u32, String> {
    let mut rows = BTreeMap::new();
    for text in texts {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut digits = String::new();
            let mut rest: Option<&str> = None;
            for (index, ch) in trimmed.char_indices() {
                if ch.is_ascii_digit() && digits.len() < 3 {
                    digits.push(ch);
                    continue;
                }
                if digits.is_empty() {
                    break;
                }
                rest = Some(&trimmed[index..]);
                break;
            }
            let (Some(rest), false) = (rest, digits.is_empty()) else {
                continue;
            };
            let Ok(number) = digits.parse::<u32>() else {
                continue;
            };
            if !(1..=200).contains(&number) {
                continue;
            }
            let value = rest.trim_start_matches(|c: char| {
                c == '.' || c == ')' || c == ':' || c == ']' || c == '-' || c.is_whitespace()
            });
            // 答案行必须有内容且不是另一个长句（限制长度，避免误把正文当成答案）。
            if value.is_empty()
                || value.chars().count() > 40
                || value.split_whitespace().count() > 6
            {
                continue;
            }
            if value.contains("http") {
                continue;
            }
            rows.entry(number).or_insert_with(|| value.to_string());
        }
    }
    rows
}

/// 对本地/云端候选的每个槽位做原文件核验。
///
/// `slots` 来自候选的 `(slot_id, question_number, answer, has_source_evidence, prompt)`。
///
/// `verifier` 是 A3 的模型通道（可选）：确定性结论**全部产出之后**才调用它，
/// 且只用它补「确定性抽不到」的槽位。`None` 时行为与未接入 A3 时逐字一致。
pub(crate) fn verify_against_source(
    document_ir: Option<&Value>,
    local_slots: &[(String, u32, Option<Value>, bool, String)],
    local_groups: &[(String, Vec<u32>)],
    verifier: Option<SourceVerifyRunner<'_>>,
) -> SourceVerificationV1 {
    let texts = page_texts(document_ir);
    let has_text = texts.iter().any(|text| !text.trim().is_empty());
    let mut verification = SourceVerificationV1 {
        page_texts: texts.clone(),
        question_tokens: collect_question_tokens(&texts),
        answer_rows: collect_answer_rows(&texts),
        ..Default::default()
    };

    // 没有文本时仍然记录「无法验证」的 finding，避免上层把缺席当成已验证。
    //
    // 但**不因此提前返回**：扫描版答案页恰恰是模型最有价值的场景——确定性抽取读不到，
    // 而模型可以直接读原文件（网关侧附 PDF 附件）。是否交给模型由下面的通道统一决定。
    if !has_text {
        for (slot_id, _, _, _, _) in local_slots {
            push_finding(
                &mut verification,
                DecisionTargetTypeV1::Slot,
                slot_id,
                DecisionFieldV1::Answer,
                SourceVerdictV1::NotVerifiable,
                reason::NO_SOURCE_EVIDENCE,
                Vec::new(),
                None,
                true,
            );
        }
    }

    for (slot_id, question_number, answer, has_anchors, _prompt) in local_slots {
        // 无文本时确定性核验已在上面的分支里如实记录，这里不再重复一遍。
        if !has_text {
            break;
        }
        let question_present = verification.question_tokens.contains(question_number);
        let evidence = vec![DecisionEvidenceV1 {
            chain: ChainKindV1::Source,
            anchor_kind: if question_present {
                "question_token"
            } else {
                "none"
            }
            .to_string(),
            page_index: None,
            quote: None,
            anchor: None,
        }];
        let Some(answer) = answer.as_ref() else {
            push_finding(
                &mut verification,
                DecisionTargetTypeV1::Slot,
                slot_id,
                DecisionFieldV1::Answer,
                SourceVerdictV1::NotVerifiable,
                reason::EVIDENCE_MISSING,
                evidence,
                None,
                true,
            );
            continue;
        };
        // 只有本地稿给出 source anchors 时，才认为存在「原文证据面」。
        if !*has_anchors {
            push_finding(
                &mut verification,
                DecisionTargetTypeV1::Slot,
                slot_id,
                DecisionFieldV1::Answer,
                SourceVerdictV1::NotVerifiable,
                reason::NO_SOURCE_EVIDENCE,
                evidence,
                None,
                true,
            );
            continue;
        }
        if !question_present {
            push_finding(
                &mut verification,
                DecisionTargetTypeV1::Slot,
                slot_id,
                DecisionFieldV1::Answer,
                SourceVerdictV1::NotVerifiable,
                reason::NO_SOURCE_EVIDENCE,
                evidence,
                None,
                true,
            );
            continue;
        }
        let answer_kind = answer.get("kind").and_then(Value::as_str).unwrap_or("");
        // 本地没有值、但原文件有可读答案行：这是「建议补全」，不是分歧。
        if value_is_empty(answer) {
            if let Some(row) = verification.answer_rows.get(question_number) {
                // 形状对齐由 `suggested_in_local_shape` 统一实现；第三个参数沿用原有的
                // 硬编码 assignment = "per_slot"（本地没有值时不存在可沿用的 assignment）。
                if let Some(suggested) = suggested_in_local_shape(answer_kind, row, "per_slot") {
                    push_finding(
                        &mut verification,
                        DecisionTargetTypeV1::Slot,
                        slot_id,
                        DecisionFieldV1::Answer,
                        SourceVerdictV1::Suggested,
                        reason::RULES_MATCH,
                        evidence,
                        Some(suggested),
                        true,
                    );
                    continue;
                }
                // 映射不出本地形状（例如选项题但答案行里没有字母标签）⇒ 不给建议，
                // 落到下面的「无法判断」。构造一个空 labels 的选项值只会制造形状垃圾。
            }
            push_finding(
                &mut verification,
                DecisionTargetTypeV1::Slot,
                slot_id,
                DecisionFieldV1::Answer,
                SourceVerdictV1::NotVerifiable,
                reason::EVIDENCE_MISSING,
                evidence,
                None,
                true,
            );
            continue;
        }
        let matched = match answer_kind {
            "text" => {
                let values = answer
                    .get("values")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let expected = verification.answer_rows.get(question_number);
                // 仅当值直接出现在「该题号对应的答案行」里才确认——这是把答案绑定到
                // 本题的确定性证据（答案行由 `collect_answer_rows` 从原文件的可读答案表
                // 抽取，形如 `14 stencilling`）。页面级子串命中（值只是恰好出现在原文件
                // 某处正文里）不足以确认：否则「这个词出现在文章里」会被误报成「答案正确」，
                // 产生假的 Confirmed。无答案行可对照时保持 NotVerifiable，绝不 Confirmed。
                let row_match = expected
                    .zip(values.first())
                    .and_then(|(row, value)| value.as_str().map(|value| (row, value)))
                    .map(|(row, value)| normalize_text(row) == normalize_text(value))
                    .unwrap_or(false);
                if row_match {
                    Some(SourceVerdictV1::Confirmed)
                } else if expected.is_some() {
                    Some(SourceVerdictV1::Contradicted)
                } else {
                    None
                }
            }
            "option" => {
                let labels = answer
                    .get("labels")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let row = verification.answer_rows.get(question_number);
                match row {
                    Some(row) => {
                        let row_normalized = normalize_text(row);
                        // 把答案行切成词元（字母数字串），供短标签做词元相等匹配。
                        let row_tokens: Vec<&str> = row_normalized
                            .split(|c: char| !c.is_alphanumeric())
                            .filter(|token| !token.is_empty())
                            .collect();
                        let agrees = labels.iter().filter_map(Value::as_str).any(|label| {
                            let label = label.trim().to_lowercase();
                            if label.is_empty() {
                                return false;
                            }
                            // 短字母标签（单/双字符，如 A/B/C/D）：必须用**词元相等**，
                            // 否则 `B` 会命中 babbage/biology/carbon，造成假确认——与
                            // 已修掉的「文章里出现过 ≠ 答案正确」是同一类错误。
                            // 仅当标签足够长（≥3 字符，通常是选项文本而非字母标签）
                            // 时，才放宽到「整行相等或子串包含」：长文本区分度高，
                            // 子串误命中风险低，且保留对选项文本答案行的兼容。
                            if label.chars().count() <= 2
                                && label.chars().all(|c| c.is_ascii_alphabetic())
                            {
                                row_tokens.iter().any(|token| *token == label)
                            } else {
                                row_normalized == label || row_normalized.contains(&label)
                            }
                        });
                        Some(if agrees {
                            SourceVerdictV1::Confirmed
                        } else {
                            SourceVerdictV1::Contradicted
                        })
                    }
                    None => None,
                }
            }
            _ => None,
        };
        match matched {
            Some(SourceVerdictV1::Confirmed) => {
                push_finding(
                    &mut verification,
                    DecisionTargetTypeV1::Slot,
                    slot_id,
                    DecisionFieldV1::Answer,
                    SourceVerdictV1::Confirmed,
                    reason::RULES_MATCH,
                    evidence,
                    None,
                    true,
                );
            }
            Some(SourceVerdictV1::Contradicted) => {
                let row = verification
                    .answer_rows
                    .get(question_number)
                    .cloned()
                    .unwrap_or_default();
                let assignment = answer
                    .get("assignment")
                    .and_then(Value::as_str)
                    .unwrap_or("per_slot");
                push_finding(
                    &mut verification,
                    DecisionTargetTypeV1::Slot,
                    slot_id,
                    DecisionFieldV1::Answer,
                    SourceVerdictV1::Contradicted,
                    reason::SUBSTANTIVE_DIVERGENCE,
                    evidence,
                    suggested_in_local_shape(answer_kind, &row, assignment),
                    true,
                );
            }
            _ => {
                push_finding(
                    &mut verification,
                    DecisionTargetTypeV1::Slot,
                    slot_id,
                    DecisionFieldV1::Answer,
                    SourceVerdictV1::NotVerifiable,
                    reason::NO_SOURCE_EVIDENCE,
                    evidence,
                    None,
                    true,
                );
            }
        }
    }

    // 题组级：题干与选项是否在原文件中出现。
    for (task_id, numbers) in local_groups {
        if !has_text {
            break;
        }
        let prompt_present = numbers
            .iter()
            .any(|number| verification.question_tokens.contains(number));
        push_finding(
            &mut verification,
            DecisionTargetTypeV1::Task,
            task_id,
            DecisionFieldV1::Prompt,
            if prompt_present {
                SourceVerdictV1::Confirmed
            } else {
                SourceVerdictV1::NotVerifiable
            },
            if prompt_present {
                reason::RULES_MATCH
            } else {
                reason::NO_SOURCE_EVIDENCE
            },
            Vec::new(),
            None,
            true,
        );
    }

    // ── A3：模型通道 ────────────────────────────────────────────────────
    //
    // 确定性结论此刻已**全部产出**，模型只能给「确定性抽不到」的槽位补证据。
    if let Some(verifier) = verifier {
        merge_model_verification(&mut verification, verifier, local_slots);
    }

    // 状态按**合并后**的 findings 重算：模型把某些槽位从「无法判断」提升为
    // 「已确认」后，链状态必须跟着变，否则会出现「结论变强了、状态还说没有证据」。
    let (confirmed, total) = slot_answer_counts(&verification);
    verification.status = if total == 0 {
        ChainStatusV1::NotRun
    } else if !has_text && confirmed == 0 {
        // 原文读不出文本、模型也没给出证据 ⇒ 与未接入 A3 时逐字一致。
        ChainStatusV1::NotRun
    } else if confirmed == total {
        ChainStatusV1::Succeeded
    } else {
        ChainStatusV1::Partial
    };
    verification.reason_code = if !has_text && confirmed == 0 {
        Some(
            if document_ir.is_some() {
                reason::SOURCE_FILE_UNREADABLE
            } else {
                reason::EVIDENCE_MISSING
            }
            .to_string(),
        )
    } else if verification.status == ChainStatusV1::Succeeded {
        None
    } else {
        Some(reason::NO_SOURCE_EVIDENCE.to_string())
    };
    verification
}

/// 把模型核验结论合并进确定性结果。
///
/// 合并规则只有一条，但它承载了整个 A3 的安全边界：
/// **只填确定性抽不到的槽位，绝不推翻已成立的确定性结论**。
/// 原文里明明可读的答案行不该被一次模型幻觉改写；反之，确定性读不到的地方
/// （扫描版答案页、答案页不在 `DocumentIRV2` 里）正是模型的价值所在。
fn merge_model_verification(
    verification: &mut SourceVerificationV1,
    verifier: SourceVerifyRunner<'_>,
    local_slots: &[(String, u32, Option<Value>, bool, String)],
) {
    // 只有「确定性核验判不了」**且本地确实有值可比对**的槽位才交给模型。
    // 没有本地值的槽位要的是「给出一个全新答案」，那属于模型发明答案，本方案不做：
    // 因此模型通道没有 `suggested` 结论，只有 `confirmed` / `contradicted`。
    let pending: Vec<(String, u32, Value)> = local_slots
        .iter()
        .filter_map(|(slot_id, question_number, answer, _, _)| {
            let finding = verification.finding(
                DecisionTargetTypeV1::Slot,
                slot_id,
                DecisionFieldV1::Answer,
            )?;
            if finding.verdict != SourceVerdictV1::NotVerifiable {
                return None;
            }
            let answer = answer.as_ref()?;
            Some((slot_id.clone(), *question_number, answer.clone()))
        })
        .collect();
    if pending.is_empty() {
        return;
    }

    let payload: Vec<Value> = pending
        .iter()
        .map(|(slot_id, question_number, answer)| {
            serde_json::json!({
                "slotId": slot_id,
                "questionNumber": question_number,
                "localValue": answer
            })
        })
        .collect();

    let raw = match verifier(&payload) {
        Ok(raw) => raw,
        Err(ModelCallFailure::BudgetExhausted) => {
            verification.model_status = SourceModelStatusV1::BudgetExhausted;
            verification.model_reason_code =
                Some(reason::SOURCE_VERIFY_BUDGET_EXHAUSTED.to_string());
            return;
        }
        Err(ModelCallFailure::Model(error)) => {
            let failure = classify_cloud_error(&error);
            verification.model_status = SourceModelStatusV1::Unusable;
            verification.model_reason_code = Some(failure.reason_code);
            return;
        }
    };
    verification.model_status = SourceModelStatusV1::Succeeded;
    verification.model_reason_code = None;

    let Some(findings) = raw.get("findings").and_then(Value::as_array) else {
        return;
    };
    // 网关侧已拒绝重复 slotId；这里的 map 只是取用，不承担去重职责。
    let by_slot: BTreeMap<&str, &Value> = findings
        .iter()
        .filter_map(|finding| {
            finding
                .get("slotId")
                .and_then(Value::as_str)
                .map(|slot_id| (slot_id, finding))
        })
        .collect();

    for (slot_id, question_number, local_answer) in &pending {
        let Some(finding) = by_slot.get(slot_id.as_str()) else {
            continue;
        };
        match finding.get("verdict").and_then(Value::as_str) {
            // 「原文支持本地这个值」：升级为已确认。这条结论**只会减少噪声**
            // （把 `Unverifiable` 变成 `Agreed`），不会改动题稿，也不需要建议值。
            Some("confirmed") => {
                let Some(evidence) = model_evidence(finding, *question_number, verification) else {
                    continue;
                };
                push_finding(
                    verification,
                    DecisionTargetTypeV1::Slot,
                    slot_id,
                    DecisionFieldV1::Answer,
                    SourceVerdictV1::Confirmed,
                    reason::RULES_MATCH,
                    evidence,
                    None,
                    false,
                );
            }
            // 「原文给的是另一个值」：作为实质分歧暴露，附建议值供用户一键接受。
            // `deterministic = false` 保证它**不会**被自动写入题稿
            // （见 `SourceVerificationV1::deterministic_suggested`）。
            Some("contradicted") => {
                let Some(evidence) = model_evidence(finding, *question_number, verification) else {
                    continue;
                };
                let Some(observed) = finding.get("observedValue") else {
                    continue;
                };
                let observed_text = answer_strings(observed).join(" ");
                if observed_text.trim().is_empty() {
                    continue;
                }
                let local_kind = local_answer
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let assignment = local_answer
                    .get("assignment")
                    .and_then(Value::as_str)
                    .unwrap_or("per_slot");
                let Some(suggested) =
                    suggested_in_local_shape(local_kind, &observed_text, assignment)
                else {
                    continue;
                };
                // 与本地同形同值 ⇒ 这不是分歧，不得虚构一条。
                if answer_compare_key(Some(&suggested)) == answer_compare_key(Some(local_answer)) {
                    continue;
                }
                push_finding(
                    verification,
                    DecisionTargetTypeV1::Slot,
                    slot_id,
                    DecisionFieldV1::Answer,
                    SourceVerdictV1::Contradicted,
                    reason::SUBSTANTIVE_DIVERGENCE,
                    evidence,
                    Some(suggested),
                    false,
                );
            }
            // `not_verifiable`（模型也读不出来）与任何未知取值：保持原结论不变。
            _ => {}
        }
    }
}

/// 模型结论的证据条目。
///
/// 返回 `None` 表示**没有出处**：`confirmed` 与 `contradicted` 都是对原文件的断言，
/// 缺 `quote` 或 `pageIndex` 就不采纳。网关侧已按同一规则拒绝整份输出，
/// 这里是第二道闸，因为 `verify_against_source` 也能被直接调用（测试、其他调用方）。
fn model_evidence(
    finding: &Value,
    question_number: u32,
    verification: &SourceVerificationV1,
) -> Option<Vec<DecisionEvidenceV1>> {
    let quote = finding
        .get("quote")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|quote| !quote.trim().is_empty())?;
    // 页码必须 ≥ 1：本仓库的引用约定里 0 表示「没有页码」，不是「第 0 页」。
    let page_index = finding
        .get("pageIndex")
        .and_then(Value::as_u64)
        .filter(|index| *index >= 1)
        .and_then(|index| u32::try_from(index).ok())?;
    Some(vec![DecisionEvidenceV1 {
        chain: ChainKindV1::Source,
        anchor_kind: if verification.question_tokens.contains(&question_number) {
            "model_quote"
        } else {
            "model_quote_question_token_absent"
        }
        .to_string(),
        page_index: Some(page_index),
        quote: Some(quote),
        anchor: Some(serde_json::json!({
            "origin": "model",
            "confidence": finding.get("confidence").cloned().unwrap_or(Value::Null),
        })),
    }])
}

/// 题组题号展开（供核验调用方使用）。
pub(crate) fn group_question_numbers(display_range: &Value) -> Vec<u32> {
    expand_question_numbers(display_range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document_with_pages(pages: &[&str]) -> Value {
        json!({
            "pages": pages.iter().enumerate().map(|(index, text)| json!({
                "pageIndex": index,
                "lines": text.lines().map(|line| json!({"text": line})).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        })
    }

    #[test]
    fn missing_document_marks_every_answer_unverifiable() {
        let slots = vec![(
            "slot-14".to_string(),
            14,
            Some(json!({"kind":"text","values":["stencilling"]})),
            true,
            String::new(),
        )];
        let verification = verify_against_source(None, &slots, &[], None);
        assert_eq!(verification.status, ChainStatusV1::NotRun);
        assert_eq!(
            verification.reason_code.as_deref(),
            Some(reason::EVIDENCE_MISSING)
        );
        assert!(!verification.is_confirmed(
            DecisionTargetTypeV1::Slot,
            "slot-14",
            DecisionFieldV1::Answer
        ));
    }

    /// 验收项 4：各路结果一致但缺乏原文证据，不得被误判为已验证。
    #[test]
    fn answer_without_source_anchors_is_not_confirmed_even_when_text_is_readable() {
        let document = document_with_pages(&["14 stencilling\n15 books"]);
        let slots = vec![
            (
                "slot-14".to_string(),
                14,
                Some(json!({"kind":"text","values":["stencilling"]})),
                true,
                String::new(),
            ),
            (
                "slot-15".to_string(),
                15,
                Some(json!({"kind":"text","values":["books"]})),
                false,
                String::new(),
            ),
        ];
        let verification = verify_against_source(Some(&document), &slots, &[], None);
        assert!(verification.is_confirmed(
            DecisionTargetTypeV1::Slot,
            "slot-14",
            DecisionFieldV1::Answer
        ));
        assert!(
            !verification.is_confirmed(
                DecisionTargetTypeV1::Slot,
                "slot-15",
                DecisionFieldV1::Answer
            ),
            "没有 source anchors 的答案必须保持未验证"
        );
        let finding = verification
            .finding(
                DecisionTargetTypeV1::Slot,
                "slot-15",
                DecisionFieldV1::Answer,
            )
            .expect("finding required");
        assert_eq!(finding.reason_code, reason::NO_SOURCE_EVIDENCE);
    }

    #[test]
    fn readable_answer_key_contradicts_a_wrong_local_value_and_suggests_a_fix() {
        let document = document_with_pages(&["14 stencilling\n15 books"]);
        let slots = vec![(
            "slot-14".to_string(),
            14,
            Some(json!({"kind":"text","values":["painting"]})),
            true,
            String::new(),
        )];
        let verification = verify_against_source(Some(&document), &slots, &[], None);
        let suggested = verification
            .suggested(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer,
            )
            .expect("contradiction must carry a suggestion");
        assert_eq!(suggested["values"][0], json!("stencilling"));
    }

    #[test]
    fn unreadable_scanned_pages_leave_everything_unverifiable() {
        // 页存在但没有可抽取文本（扫描答案页的真实情形）。
        let document = document_with_pages(&["", ""]);
        let slots = vec![(
            "slot-14".to_string(),
            14,
            Some(json!({"kind":"text","values":["stencilling"]})),
            true,
            String::new(),
        )];
        let verification = verify_against_source(Some(&document), &slots, &[], None);
        assert_eq!(verification.status, ChainStatusV1::NotRun);
        assert_eq!(
            verification.reason_code.as_deref(),
            Some(reason::SOURCE_FILE_UNREADABLE)
        );
    }

    /// Fix 1(a)：值仅出现在页面正文中（无该题答案行）不得判 Confirmed。
    #[test]
    fn page_level_substring_match_is_not_confirmed() {
        // "stencilling" 出现在文章正文里，但原文件没有「14 答案行」，
        // 且题号 14 仍作为裸数字出现（通过 question_token 闸门）。
        let document =
            document_with_pages(&["On page 14 we read that the artist was stencilling the wall."]);
        let slots = vec![(
            "slot-14".to_string(),
            14,
            Some(json!({"kind":"text","values":["stencilling"]})),
            true,
            String::new(),
        )];
        let verification = verify_against_source(Some(&document), &slots, &[], None);
        assert!(
            !verification.is_confirmed(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer
            ),
            "页面级子串命中不得判为 Confirmed"
        );
        let finding = verification
            .finding(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer,
            )
            .expect("应有 finding");
        assert_eq!(finding.verdict, SourceVerdictV1::NotVerifiable);
    }

    /// Fix 1(b)：值出现在该题号对应的答案行里必须 Confirmed。
    #[test]
    fn answer_row_for_question_number_confirms() {
        let document = document_with_pages(&["14 stencilling\n15 books"]);
        let slots = vec![(
            "slot-14".to_string(),
            14,
            Some(json!({"kind":"text","values":["stencilling"]})),
            true,
            String::new(),
        )];
        let verification = verify_against_source(Some(&document), &slots, &[], None);
        assert!(
            verification.is_confirmed(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer
            ),
            "出现在本题答案行里的答案必须 Confirmed"
        );
    }

    /// Fix 1 补强：短字母标签不得被答案行里的子串命中假确认。
    /// 答案行 `14 babbage`、本地标签 `B` → 必须是 Contradicted（不是 Confirmed）。
    #[test]
    fn option_short_label_is_not_confirmed_by_substring_in_answer_row() {
        let document = document_with_pages(&["14 babbage"]);
        let slots = vec![(
            "slot-14".to_string(),
            14,
            Some(json!({"kind":"option","labels":["B"],"assignment":"per_slot"})),
            true,
            String::new(),
        )];
        let verification = verify_against_source(Some(&document), &slots, &[], None);
        let finding = verification
            .finding(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer,
            )
            .expect("应有 finding");
        assert_eq!(
            finding.verdict,
            SourceVerdictV1::Contradicted,
            "短标签子串命中（B ∈ babbage）不得 Confirmed，应判定为与原文答案行不符"
        );
        assert!(
            !verification.is_confirmed(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer
            ),
            "B 不得因命中 babbage 而被假确认"
        );
    }

    /// Fix 1 补强：短字母标签与答案行词元相等 → Confirmed。
    #[test]
    fn option_short_label_matches_answer_row_token() {
        let document = document_with_pages(&["14 B"]);
        let slots = vec![(
            "slot-14".to_string(),
            14,
            Some(json!({"kind":"option","labels":["B"],"assignment":"per_slot"})),
            true,
            String::new(),
        )];
        let verification = verify_against_source(Some(&document), &slots, &[], None);
        assert!(
            verification.is_confirmed(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer
            ),
            "答案行词元 B 与标签 B 相等必须 Confirmed"
        );
    }

    // ── A3：模型核验通道 ───────────────────────────────────────────────

    /// 只有模型能回答的槽位：正文里没有可读答案行（`14 stencilling` 这种），
    /// 确定性核验只能判「无法判断」。
    fn slot_14_with(answer: Value) -> Vec<(String, u32, Option<Value>, bool, String)> {
        vec![("slot-14".to_string(), 14, Some(answer), true, String::new())]
    }

    fn confirmed_finding() -> Value {
        json!({"findings": [{
            "slotId": "slot-14",
            "questionNumber": 14,
            "verdict": "confirmed",
            "quote": "the artist was stencilling the wall",
            "pageIndex": 2,
            "confidence": 0.9
        }]})
    }

    /// A3 主路径：确定性抽不到、模型带着原文出处确认 → 槽位升级为已确认。
    #[test]
    fn model_verification_confirms_a_slot_the_extractor_cannot_bind() {
        let document = document_with_pages(&["Question 14 asks about stencilling."]);
        let slots = slot_14_with(json!({"kind":"text","values":["stencilling"]}));

        let baseline = verify_against_source(Some(&document), &slots, &[], None);
        assert_eq!(
            baseline.verdict(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer
            ),
            Some(SourceVerdictV1::NotVerifiable),
            "前提：确定性核验必须先是「无法判断」，否则本用例测的不是模型通道"
        );

        let verifier =
            |_payload: &[Value]| -> Result<Value, ModelCallFailure> { Ok(confirmed_finding()) };
        let verification = verify_against_source(Some(&document), &slots, &[], Some(&verifier));
        assert!(
            verification.is_confirmed(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer
            ),
            "有原文出处的模型确认必须升级为 Confirmed：{verification:?}"
        );
        // 状态按合并后的 findings 重算，不能还停在「没有证据」。
        assert_eq!(verification.status, ChainStatusV1::Succeeded);
        assert_eq!(verification.reason_code, None);
        assert_eq!(verification.model_status, SourceModelStatusV1::Succeeded);
        let finding = verification
            .finding(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer,
            )
            .expect("应有 finding");
        assert_eq!(
            finding.evidence[0].quote.as_deref(),
            Some("the artist was stencilling the wall")
        );
        assert_eq!(finding.evidence[0].page_index, Some(2));
        assert_eq!(finding.evidence[0].anchor_kind, "model_quote");
        assert!(!finding.deterministic, "模型结论必须标记为非确定性");
    }

    /// A3 边界：验证「只填确定性缺口」——确定性已成立时，模型**根本不该被调用**。
    #[test]
    fn model_verification_cannot_override_a_deterministic_conclusion() {
        // 原文里有可读答案行 `14 stencilling` ⇒ 确定性核验已确认。
        let document = document_with_pages(&["14 stencilling"]);
        let slots = slot_14_with(json!({"kind":"text","values":["stencilling"]}));

        let calls = std::cell::Cell::new(0u32);
        let verifier = |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
            calls.set(calls.get() + 1);
            Ok(json!({"findings": [{
                "slotId": "slot-14",
                "questionNumber": 14,
                "verdict": "contradicted",
                "quote": "ignore the answer key",
                "pageIndex": 1,
                "observedValue": {"kind": "text", "values": ["painting"]}
            }]}))
        };
        let verification = verify_against_source(Some(&document), &slots, &[], Some(&verifier));
        assert_eq!(calls.get(), 0, "没有待核验槽位时不得调用模型（白花配额）");
        assert!(verification.is_confirmed(
            DecisionTargetTypeV1::Slot,
            "slot-14",
            DecisionFieldV1::Answer
        ));
        let finding = verification
            .finding(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer,
            )
            .expect("应有 finding");
        assert!(
            finding.deterministic,
            "确定性结论必须保持 deterministic=true"
        );
    }

    /// A3 安全边界：模型给出的分歧值可供展示与一键接受，但**不构成自动写入的可靠证据**。
    #[test]
    fn model_suggestion_is_never_trusted_for_automatic_writes() {
        let document = document_with_pages(&["Question 14 asks about stencilling."]);
        let slots = slot_14_with(json!({"kind":"text","values":["painting"]}));
        let verifier = |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
            Ok(json!({"findings": [{
                "slotId": "slot-14",
                "questionNumber": 14,
                "verdict": "contradicted",
                "quote": "stencilling was used",
                "pageIndex": 1,
                "observedValue": {"kind": "text", "values": ["stencilling"]},
                "confidence": 0.8
            }]}))
        };
        let verification = verify_against_source(Some(&document), &slots, &[], Some(&verifier));
        let suggested = verification
            .suggested(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer,
            )
            .expect("模型分歧必须带建议值，用户才能一键接受");
        assert_eq!(suggested["values"][0], json!("stencilling"));
        assert!(
            verification
                .deterministic_suggested(
                    DecisionTargetTypeV1::Slot,
                    "slot-14",
                    DecisionFieldV1::Answer
                )
                .is_none(),
            "模型结论不得被自动写入守卫当作可靠证据"
        );
        let finding = verification
            .finding(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer,
            )
            .expect("应有 finding");
        assert_eq!(finding.verdict, SourceVerdictV1::Contradicted);
        assert!(!finding.deterministic);
    }

    /// A3：模型给出的值与本地同形同值 ⇒ 这不是分歧，不得虚构一条。
    #[test]
    fn model_verification_does_not_invent_a_divergence_that_is_agreement() {
        let document = document_with_pages(&["Question 14 asks about stencilling."]);
        let slots = slot_14_with(json!({"kind":"text","values":["stencilling"]}));
        let verifier = |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
            Ok(json!({"findings": [{
                "slotId": "slot-14",
                "questionNumber": 14,
                "verdict": "contradicted",
                "quote": "stencilling",
                "pageIndex": 1,
                "observedValue": {"kind": "text", "values": ["stencilling"]}
            }]}))
        };
        let verification = verify_against_source(Some(&document), &slots, &[], Some(&verifier));
        assert_eq!(
            verification.verdict(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer
            ),
            Some(SourceVerdictV1::NotVerifiable),
            "值与本地相同就不构成分歧"
        );
    }

    /// A3：原文读不出文本（扫描版答案页）**仍然**交给模型 —— 这正是 A3 最有价值的场景。
    #[test]
    fn unreadable_source_still_consults_the_model() {
        // 页存在但没有可抽取文本。
        let document = document_with_pages(&["", ""]);
        let slots = slot_14_with(json!({"kind":"text","values":["stencilling"]}));
        let verifier = |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
            Ok(json!({"findings": [{
                "slotId": "slot-14",
                "questionNumber": 14,
                "verdict": "confirmed",
                "quote": "14 stencilling",
                "pageIndex": 4
            }]}))
        };
        let verification = verify_against_source(Some(&document), &slots, &[], Some(&verifier));
        assert!(verification.is_confirmed(
            DecisionTargetTypeV1::Slot,
            "slot-14",
            DecisionFieldV1::Answer
        ));
        assert_eq!(verification.status, ChainStatusV1::Succeeded);
        assert_eq!(
            verification.reason_code, None,
            "证据已由模型补上，不得再报「原文不可读」"
        );
    }

    /// A3：没有原文出处的断言不采纳（网关侧是第一道闸，这里是第二道）。
    #[test]
    fn model_claims_without_a_source_locator_are_ignored() {
        let document = document_with_pages(&["Question 14 asks about stencilling."]);
        let slots = slot_14_with(json!({"kind":"text","values":["stencilling"]}));

        let cases = [
            // quote 是空白
            json!({"slotId":"slot-14","questionNumber":14,"verdict":"confirmed","quote":"   ","pageIndex":2}),
            // pageIndex = 0：本仓库约定 0 表示「没有页码」
            json!({"slotId":"slot-14","questionNumber":14,"verdict":"confirmed","quote":"text","pageIndex":0}),
            // 缺 pageIndex
            json!({"slotId":"slot-14","questionNumber":14,"verdict":"confirmed","quote":"text"}),
        ];
        for case in cases {
            let response = json!({"findings": [case.clone()]});
            let verifier = move |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
                Ok(response.clone())
            };
            let verification = verify_against_source(Some(&document), &slots, &[], Some(&verifier));
            assert_eq!(
                verification.verdict(
                    DecisionTargetTypeV1::Slot,
                    "slot-14",
                    DecisionFieldV1::Answer
                ),
                Some(SourceVerdictV1::NotVerifiable),
                "无出处的断言不得升级结论：{case}"
            );
        }
    }

    /// A3：模型通道失败必须如实上报，且**不得**改动确定性结论。
    #[test]
    fn model_channel_failure_is_reported_and_leaves_findings_untouched() {
        let document = document_with_pages(&["Question 14 asks about stencilling."]);
        let slots = slot_14_with(json!({"kind":"text","values":["stencilling"]}));
        let verifier = |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
            Err(ModelCallFailure::Model(
                "request timeout after 30s".to_string(),
            ))
        };
        let verification = verify_against_source(Some(&document), &slots, &[], Some(&verifier));
        assert_eq!(verification.model_status, SourceModelStatusV1::Unusable);
        assert_eq!(
            verification.unusable_reason(),
            Some(reason::MODEL_TIMEOUT),
            "失败分类必须复用 classify_cloud_error，不另造一套"
        );
        assert_eq!(
            verification.verdict(
                DecisionTargetTypeV1::Slot,
                "slot-14",
                DecisionFieldV1::Answer
            ),
            Some(SourceVerdictV1::NotVerifiable)
        );
        assert_eq!(
            verification.reason_code.as_deref(),
            Some(reason::NO_SOURCE_EVIDENCE)
        );
    }

    /// A3：预算耗尽 ≠ 模型失败。两者原因码必须分清，且预算耗尽不该改写链状态原因码。
    #[test]
    fn model_budget_exhaustion_is_not_reported_as_a_model_failure() {
        let document = document_with_pages(&["Question 14 asks about stencilling."]);
        let slots = slot_14_with(json!({"kind":"text","values":["stencilling"]}));
        let verifier = |_payload: &[Value]| -> Result<Value, ModelCallFailure> {
            Err(ModelCallFailure::BudgetExhausted)
        };
        let verification = verify_against_source(Some(&document), &slots, &[], Some(&verifier));
        assert_eq!(
            verification.model_status,
            SourceModelStatusV1::BudgetExhausted
        );
        assert_eq!(
            verification.model_reason_code.as_deref(),
            Some(reason::SOURCE_VERIFY_BUDGET_EXHAUSTED)
        );
        assert_eq!(
            verification.unusable_reason(),
            None,
            "预算耗尽不是「模型不可用」，不得顶掉原文证据的结论"
        );
    }

    /// A3：交给模型的必须只是「确定性判不了」的槽位，并带本地值供其比对。
    #[test]
    fn only_unresolved_slots_are_sent_to_the_model() {
        let document = document_with_pages(&["14 stencilling"]);
        let slots = vec![
            (
                "slot-14".to_string(),
                14,
                Some(json!({"kind":"text","values":["stencilling"]})),
                true,
                String::new(),
            ),
            (
                "slot-15".to_string(),
                15,
                Some(json!({"kind":"text","values":["books"]})),
                true,
                String::new(),
            ),
        ];
        let seen = std::cell::RefCell::new(Vec::<Value>::new());
        let verifier = |payload: &[Value]| -> Result<Value, ModelCallFailure> {
            *seen.borrow_mut() = payload.to_vec();
            Ok(json!({"findings": []}))
        };
        let _ = verify_against_source(Some(&document), &slots, &[], Some(&verifier));
        let sent = seen.borrow();
        assert_eq!(sent.len(), 1, "只应送出确定性判不了的那一个槽位：{sent:?}");
        assert_eq!(sent[0]["slotId"], json!("slot-15"));
        assert_eq!(sent[0]["questionNumber"], json!(15));
        assert_eq!(sent[0]["localValue"]["values"][0], json!("books"));
    }

    /// A3：不注入模型通道时，行为与未接入 A3 时逐字一致（回归护栏）。
    #[test]
    fn absent_verifier_keeps_the_channel_out_of_the_way() {
        let document = document_with_pages(&["Question 14 asks about stencilling."]);
        let slots = slot_14_with(json!({"kind":"text","values":["stencilling"]}));
        let verification = verify_against_source(Some(&document), &slots, &[], None);
        assert_eq!(verification.model_status, SourceModelStatusV1::NotRun);
        assert_eq!(verification.model_reason_code, None);
        assert_eq!(verification.unusable_reason(), None);
        assert_eq!(verification.status, ChainStatusV1::Partial);
        assert_eq!(
            verification.reason_code.as_deref(),
            Some(reason::NO_SOURCE_EVIDENCE)
        );
    }
}
