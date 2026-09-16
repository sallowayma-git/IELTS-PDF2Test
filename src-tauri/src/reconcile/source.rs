//! 原文件核验（第三条证据链）。
//!
//! 职责边界：**只输出问题与修正建议，不制造第二份权威稿**。
//! 核验依据只有原文件（`DocumentIRV2` 的页文本 / 行 / 区域）与本地稿的
//! source anchors；没有可读文本时一律判 `NotVerifiable`，不得把「两路一致」
//! 当成「已被原文验证」。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::candidate::{expand_question_numbers, normalize_text};
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
            .map(|values| values.iter().filter_map(Value::as_str).all(|v| v.trim().is_empty()))
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
    /// 修正建议（仅 `Contradicted` 时给出），仍是建议而非权威值。
    pub suggested_value: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SourceVerificationV1 {
    pub status: ChainStatusV1,
    pub reason_code: Option<String>,
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
        self.finding(target_type, target_id, field).map(|finding| finding.verdict)
    }

    /// 原文件给出的建议值（`Contradicted` 或 `Suggested`）。
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
}

fn push_finding(
    verification: &mut SourceVerificationV1,
    target_type: DecisionTargetTypeV1,
    target_id: &str,
    field: DecisionFieldV1,
    verdict: SourceVerdictV1,
    reason_code: &str,
    evidence: Vec<DecisionEvidenceV1>,
    suggested_value: Option<Value>,
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
        },
    );
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
            if value.is_empty() || value.chars().count() > 40 || value.split_whitespace().count() > 6 {
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
pub(crate) fn verify_against_source(
    document_ir: Option<&Value>,
    local_slots: &[(String, u32, Option<Value>, bool, String)],
    local_groups: &[(String, Vec<u32>)],
) -> SourceVerificationV1 {
    let texts = page_texts(document_ir);
    let has_text = texts.iter().any(|text| !text.trim().is_empty());
    let mut verification = SourceVerificationV1 {
        page_texts: texts.clone(),
        question_tokens: collect_question_tokens(&texts),
        answer_rows: collect_answer_rows(&texts),
        ..Default::default()
    };

    if !has_text {
        verification.status = ChainStatusV1::NotRun;
        verification.reason_code = Some(
            if document_ir.is_some() {
                reason::SOURCE_FILE_UNREADABLE.to_string()
            } else {
                reason::EVIDENCE_MISSING.to_string()
            },
        );
        // 没有文本时仍然记录「无法验证」的 finding，避免上层把缺席当成已验证。
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
            );
        }
        return verification;
    }

    let mut confirmed = 0usize;
    let mut total = 0usize;
    for (slot_id, question_number, answer, has_anchors, _prompt) in local_slots {
        total += 1;
        let question_present = verification.question_tokens.contains(question_number);
        let evidence = vec![DecisionEvidenceV1 {
            chain: ChainKindV1::Source,
            anchor_kind: if question_present { "question_token" } else { "none" }.to_string(),
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
            );
            continue;
        }
        let answer_kind = answer.get("kind").and_then(Value::as_str).unwrap_or("");
        // 本地没有值、但原文件有可读答案行：这是「建议补全」，不是分歧。
        if value_is_empty(answer) {
            if let Some(row) = verification.answer_rows.get(question_number) {
                let suggested = if answer_kind == "option" {
                    serde_json::json!({
                        "kind": "option",
                        "labels": row
                            .split_whitespace()
                            .map(|token| token.trim_matches(|c: char| !c.is_alphanumeric()).to_uppercase())
                            .filter(|token| token.len() == 1 && token.chars().all(|c| c.is_ascii_alphabetic()))
                            .collect::<Vec<String>>(),
                        "assignment": "per_slot"
                    })
                } else {
                    serde_json::json!({"kind": "text", "values": [row]})
                };
                push_finding(
                    &mut verification,
                    DecisionTargetTypeV1::Slot,
                    slot_id,
                    DecisionFieldV1::Answer,
                    SourceVerdictV1::Suggested,
                    reason::RULES_MATCH,
                    evidence,
                    Some(suggested),
                );
                continue;
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
            );
            continue;
        }
        let matched = match answer_kind {
            "text" => {
                let values = answer.get("values").and_then(Value::as_array).cloned().unwrap_or_default();
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
                let labels = answer.get("labels").and_then(Value::as_array).cloned().unwrap_or_default();
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
                            if label.chars().count() <= 2 && label.chars().all(|c| c.is_ascii_alphabetic()) {
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
                confirmed += 1;
                push_finding(
                    &mut verification,
                    DecisionTargetTypeV1::Slot,
                    slot_id,
                    DecisionFieldV1::Answer,
                    SourceVerdictV1::Confirmed,
                    reason::RULES_MATCH,
                    evidence,
                    None,
                );
            }
            Some(SourceVerdictV1::Contradicted) => {
                let row = verification.answer_rows.get(question_number).cloned().unwrap_or_default();
                let suggested = match answer_kind {
                    "option" => serde_json::json!({
                        "kind": "option",
                        "labels": row
                            .split_whitespace()
                            .map(|token| token.trim_matches(|c: char| !c.is_alphanumeric()).to_uppercase())
                            .filter(|token| token.len() == 1 && token.chars().all(|c| c.is_ascii_alphabetic()))
                            .collect::<Vec<String>>(),
                        "assignment": answer.get("assignment").cloned().unwrap_or(Value::String("per_slot".to_string()))
                    }),
                    _ => serde_json::json!({"kind": "text", "values": [row]}),
                };
                push_finding(
                    &mut verification,
                    DecisionTargetTypeV1::Slot,
                    slot_id,
                    DecisionFieldV1::Answer,
                    SourceVerdictV1::Contradicted,
                    reason::SUBSTANTIVE_DIVERGENCE,
                    evidence,
                    Some(suggested),
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
                );
            }
        }
    }

    // 题组级：题干与选项是否在原文件中出现。
    for (task_id, numbers) in local_groups {
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
            if prompt_present { reason::RULES_MATCH } else { reason::NO_SOURCE_EVIDENCE },
            Vec::new(),
            None,
        );
    }

    verification.status = if total == 0 {
        ChainStatusV1::NotRun
    } else if confirmed == total {
        ChainStatusV1::Succeeded
    } else if confirmed > 0 {
        ChainStatusV1::Partial
    } else {
        ChainStatusV1::Partial
    };
    verification.reason_code = match verification.status {
        ChainStatusV1::Succeeded => None,
        _ => Some(reason::NO_SOURCE_EVIDENCE.to_string()),
    };
    verification
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
        let verification = verify_against_source(None, &slots, &[]);
        assert_eq!(verification.status, ChainStatusV1::NotRun);
        assert_eq!(verification.reason_code.as_deref(), Some(reason::EVIDENCE_MISSING));
        assert!(!verification.is_confirmed(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Answer));
    }

    /// 验收项 4：各路结果一致但缺乏原文证据，不得被误判为已验证。
    #[test]
    fn answer_without_source_anchors_is_not_confirmed_even_when_text_is_readable() {
        let document = document_with_pages(&["14 stencilling\n15 books"]);
        let slots = vec![
            ("slot-14".to_string(), 14, Some(json!({"kind":"text","values":["stencilling"]})), true, String::new()),
            ("slot-15".to_string(), 15, Some(json!({"kind":"text","values":["books"]})), false, String::new()),
        ];
        let verification = verify_against_source(Some(&document), &slots, &[]);
        assert!(verification.is_confirmed(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Answer));
        assert!(
            !verification.is_confirmed(DecisionTargetTypeV1::Slot, "slot-15", DecisionFieldV1::Answer),
            "没有 source anchors 的答案必须保持未验证"
        );
        let finding = verification
            .finding(DecisionTargetTypeV1::Slot, "slot-15", DecisionFieldV1::Answer)
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
        let verification = verify_against_source(Some(&document), &slots, &[]);
        let suggested = verification
            .suggested(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Answer)
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
        let verification = verify_against_source(Some(&document), &slots, &[]);
        assert_eq!(verification.status, ChainStatusV1::NotRun);
        assert_eq!(verification.reason_code.as_deref(), Some(reason::SOURCE_FILE_UNREADABLE));
    }

    /// Fix 1(a)：值仅出现在页面正文中（无该题答案行）不得判 Confirmed。
    #[test]
    fn page_level_substring_match_is_not_confirmed() {
        // "stencilling" 出现在文章正文里，但原文件没有「14 答案行」，
        // 且题号 14 仍作为裸数字出现（通过 question_token 闸门）。
        let document = document_with_pages(&[
            "On page 14 we read that the artist was stencilling the wall.",
        ]);
        let slots = vec![(
            "slot-14".to_string(),
            14,
            Some(json!({"kind":"text","values":["stencilling"]})),
            true,
            String::new(),
        )];
        let verification = verify_against_source(Some(&document), &slots, &[]);
        assert!(
            !verification.is_confirmed(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Answer),
            "页面级子串命中不得判为 Confirmed"
        );
        let finding = verification
            .finding(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Answer)
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
        let verification = verify_against_source(Some(&document), &slots, &[]);
        assert!(
            verification.is_confirmed(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Answer),
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
        let verification = verify_against_source(Some(&document), &slots, &[]);
        let finding = verification
            .finding(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Answer)
            .expect("应有 finding");
        assert_eq!(
            finding.verdict,
            SourceVerdictV1::Contradicted,
            "短标签子串命中（B ∈ babbage）不得 Confirmed，应判定为与原文答案行不符"
        );
        assert!(
            !verification.is_confirmed(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Answer),
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
        let verification = verify_against_source(Some(&document), &slots, &[]);
        assert!(
            verification.is_confirmed(DecisionTargetTypeV1::Slot, "slot-14", DecisionFieldV1::Answer),
            "答案行词元 B 与标签 B 相等必须 Confirmed"
        );
    }
}
