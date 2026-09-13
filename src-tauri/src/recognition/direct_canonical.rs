//! G2-T04：`QuestionLayoutGraphV1` → `IeltsAuthoringIRV2` 直出。
//!
//! 产品的 direct canonical builder：从 QLG（指令区/题块/选项串/共享选项库/
//! 表格刺激/视觉刺激/未分配证据）+ 物理 `DocumentIRV2` + pre-V1 答案候选
//! （split 的 answerKeyCandidates，候选阶段产物，非 V1 authoring 输出）构建
//! canonical。**不读取 V1 authoring**（authoring-ir.json / ReadingAuthoringIrV1）。
//!
//! Feature flag：`QLG_DIRECT_CANONICAL`（默认关闭，见 `environment.rs`）。
//! 关闭时主链行为与历史完全一致（可回滚）。
//!
//! 语义契约（G2 第一组，正反测试锁定）：
//! 1. TFNG/YNNG 语句选项按题型分开（TRUE/FALSE/NOT GIVEN vs YES/NO/NOT GIVEN）；
//! 2. Multiple choice cardinality 来自指令签名解析（Choose TWO → exact=2），
//!    无法解析时产生稳定 blocker，不默认 1；
//! 3. group.block_ids 缺失的题块：保留声明题号、生成稳定 blocker，
//!    题号不从 answerSlots/answerKey/quality 分母消失；
//! 4. 视觉/地图/图示：合法 crop 物化进顶层 assets + 组 stimulus，
//!    无法物化时成为发布 blocker（warning 不替代）；
//! 5. 锚点使用真实 job sourceFileId、真实物理节点页码、绑定文件哈希，
//!    合成选项不伪造 source node，且每个锚点都指向真实物理节点（构建期校验）；
//! 6. node/option/response id 按 task/group/slot scope 唯一；
//! 7. 未分配证据按页聚合判定，不依赖单条 ≥ 阈值。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use super::local::{
    OptionBankCandidateV1, QuestionLayoutGraphV1, TableStimulusCandidateV1, TaskGroupCandidateV1,
    VisualStimulusCandidateV1,
};
use crate::authoring_pipeline::dynamic_answer_map_from_split;
use crate::ielts_grammar::answer_key::answer_value_for_slot;
use crate::ielts_grammar::issue_codes;
use crate::schema::ielts_authoring_v2::TaskTypeV2;
use crate::{ImportJob, CommandResult};

/// §6.11：单页未分配正文的聚合字符数阈值。
const SIGNIFICANT_UNASSIGNED_PAGE_CHARS: usize = 200;

/// 新增稳定码（issue_codes 词表扩展，G2 第一组）。
pub(crate) const MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED: &str =
    "MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED";
pub(crate) const QUESTION_BLOCK_MISSING: &str = "QUESTION_BLOCK_MISSING";
pub(crate) const VISUAL_FALLBACK_ASSET_NOT_MATERIALIZED: &str =
    "VISUAL_FALLBACK_ASSET_NOT_MATERIALIZED";
pub(crate) const ANCHOR_TARGET_INVALID: &str = "ANCHOR_TARGET_INVALID";

/// 锚点上下文：sourceFileId 来自真实 job source file，哈希与该文件绑定。
#[derive(Debug, Clone)]
pub(crate) struct AnchorContext {
    pub source_file_id: String,
    pub source_hash: String,
}

/// 可物化的视觉资产（由调用方提供解析器，builder 不猜路径）。
#[derive(Debug, Clone)]
pub(crate) struct ResolvedVisualAsset {
    pub sha256: String,
    pub byte_length: u64,
    pub relative_path: String,
}

fn anchor(ctx: &AnchorContext, node_ids: Vec<String>, page_index: i32) -> Value {
    json!({
        "sourceFileId": ctx.source_file_id,
        "sourceHash": ctx.source_hash,
        "pageIndex": page_index,
        "nodeIds": node_ids,
        "extractionMode": "pdf_native"
    })
}

fn text_node(id: &str, text: &str, anchors: Vec<Value>) -> Value {
    json!({
        "id": id,
        "type": "text",
        "text": text,
        "sourceAnchors": anchors,
        "provenanceStatus": "source"
    })
}

fn typed_anchor_value(anchor: &crate::schema::common::SourceAnchorV2) -> Value {
    serde_json::to_value(anchor).unwrap_or(Value::Null)
}

fn task_type_name(task_type: TaskTypeV2) -> &'static str {
    match task_type {
        TaskTypeV2::SingleChoice => "single_choice",
        TaskTypeV2::MultipleChoice => "multiple_choice",
        TaskTypeV2::TrueFalseNotGiven => "true_false_not_given",
        TaskTypeV2::YesNoNotGiven => "yes_no_not_given",
        TaskTypeV2::MatchingInformation => "matching_information",
        TaskTypeV2::MatchingHeadings => "matching_headings",
        TaskTypeV2::MatchingFeatures => "matching_features",
        TaskTypeV2::MatchingSentenceEndings => "matching_sentence_endings",
        TaskTypeV2::Classification => "classification",
        TaskTypeV2::SentenceCompletion => "sentence_completion",
        TaskTypeV2::SummaryCompletion => "summary_completion",
        TaskTypeV2::NoteCompletion => "note_completion",
        TaskTypeV2::TableCompletion => "table_completion",
        TaskTypeV2::FormCompletion => "form_completion",
        TaskTypeV2::FlowchartCompletion => "flowchart_completion",
        TaskTypeV2::DiagramLabelCompletion => "diagram_label_completion",
        TaskTypeV2::PlanMapLabelCompletion => "plan_map_label_completion",
        TaskTypeV2::ShortAnswer => "short_answer",
    }
}

fn interaction_for(task_type: &TaskTypeV2, bank_bound: bool) -> &'static str {
    match task_type {
        TaskTypeV2::SingleChoice | TaskTypeV2::TrueFalseNotGiven | TaskTypeV2::YesNoNotGiven => "radio",
        TaskTypeV2::MultipleChoice => "checkbox",
        TaskTypeV2::MatchingHeadings
        | TaskTypeV2::MatchingInformation
        | TaskTypeV2::MatchingFeatures
        | TaskTypeV2::MatchingSentenceEndings
        | TaskTypeV2::Classification => "select",
        TaskTypeV2::DiagramLabelCompletion | TaskTypeV2::PlanMapLabelCompletion => "hotspot",
        _ if bank_bound => "select",
        _ => "text",
    }
}

fn response_kind(task_type: &TaskTypeV2, bank_bound: bool) -> &'static str {
    if bank_bound
        && matches!(
            task_type,
            TaskTypeV2::SummaryCompletion
                | TaskTypeV2::NoteCompletion
                | TaskTypeV2::TableCompletion
                | TaskTypeV2::FormCompletion
                | TaskTypeV2::SentenceCompletion
                | TaskTypeV2::FlowchartCompletion
        )
    {
        return "matching";
    }
    match task_type {
        TaskTypeV2::SingleChoice
        | TaskTypeV2::MultipleChoice
        | TaskTypeV2::TrueFalseNotGiven
        | TaskTypeV2::YesNoNotGiven => "choice",
        TaskTypeV2::MatchingInformation
        | TaskTypeV2::MatchingHeadings
        | TaskTypeV2::MatchingFeatures
        | TaskTypeV2::MatchingSentenceEndings
        | TaskTypeV2::Classification => "matching",
        TaskTypeV2::DiagramLabelCompletion | TaskTypeV2::PlanMapLabelCompletion => "diagram_hotspot",
        _ => "text_entry",
    }
}

/// 指令文本中的选择数量（"Choose TWO letters" / "Choose 2"）。
pub(crate) fn parse_choose_cardinality(instruction_text: &str) -> Option<u32> {
    let normalized = instruction_text.to_ascii_uppercase();
    let marker = normalized.find("CHOOSE")?;
    let tail = &normalized[marker..(marker + 40).min(normalized.len())];
    let words = [
        ("TWO", 2), ("THREE", 3), ("FOUR", 4), ("FIVE", 5), ("SIX", 6), ("SEVEN", 7), ("EIGHT", 8),
    ];
    for (word, value) in words {
        if tail.contains(word) {
            return Some(value);
        }
    }
    tail.split_whitespace()
        .find_map(|token| token.parse::<u32>().ok())
        .filter(|value| (2..=10).contains(value))
}

/// 语句选项：TFNG 与 YNNG 按题型分开；合成选项不伪造 source node
/// （anchors 为空，content 文本可渲染）。
fn statement_options(task_type: TaskTypeV2, scope: &str) -> Vec<Value> {
    let labels: &[&str] = match task_type {
        TaskTypeV2::YesNoNotGiven => &["YES", "NO", "NOT GIVEN"],
        _ => &["TRUE", "FALSE", "NOT GIVEN"],
    };
    labels
        .iter()
        .map(|label| {
            let option_id = format!("opt-{}-{}", scope, label.to_ascii_lowercase().replace(' ', "-"));
            json!({
                "optionId": option_id,
                "label": label,
                "content": [text_node(
                    &format!("{option_id}-text"),
                    label,
                    Vec::new()
                )],
                "sourceAnchors": []
            })
        })
        .collect()
}

fn option_value(ctx: &AnchorContext, scope: &str, option: &super::local::OptionCandidateV2) -> Value {
    let option_id = format!("opt-{}-{}", scope, option.label.to_ascii_lowercase());
    let mut node_ids = option.text_node_ids.clone();
    node_ids.push(option.label_node_id.clone());
    let anchors = vec![anchor(ctx, node_ids, 0)];
    json!({
        "optionId": option_id,
        "label": option.label,
        "content": [text_node(
            &format!("{option_id}-text"),
            &option.text,
            anchors.clone()
        )],
        "sourceAnchors": anchors
    })
}

fn option_bank_value(
    ctx: &AnchorContext,
    scope: &str,
    bank: &OptionBankCandidateV1,
) -> Value {
    json!({
        "optionBankId": bank.bank_id.clone(),
        "scope": "task_group",
        "options": bank.options.iter().map(|option| option_value(ctx, scope, option)).collect::<Vec<_>>(),
        "allowReuse": true,
        "sourceAnchors": bank.options.first().map(|first| {
            let mut ids = first.text_node_ids.clone();
            ids.push(first.label_node_id.clone());
            vec![anchor(ctx, ids, bank.page_index as i32)]
        }).unwrap_or_default()
    })
}

/// QLG 表格刺激 → 语义表格节点（保留物理行/列/跨行拓扑，§6.10）。
fn table_stimulus_node(
    ctx: &AnchorContext,
    stimulus: &TableStimulusCandidateV1,
) -> Value {
    let rows: Vec<Value> = stimulus
        .rows
        .iter()
        .map(|row| {
            json!({
                "id": format!("{}-row-{}", stimulus.stimulus_id, row.row),
                "type": "table_row",
                "sourceAnchors": [],
                "provenanceStatus": "source",
                "cells": row.cells.iter().map(|cell| json!({
                    "id": cell.cell_id,
                    "type": "table_cell",
                    "sourceAnchors": [anchor(ctx, vec![cell.cell_id.clone()], stimulus.page_index as i32)],
                    "provenanceStatus": "source",
                    "rowSpan": cell.row_span,
                    "colSpan": cell.col_span,
                    "children": [text_node(
                        &format!("{}-cell-{}-{}-text", stimulus.stimulus_id, cell.row, cell.col),
                        &cell.text,
                        vec![anchor(ctx, vec![cell.cell_id.clone()], stimulus.page_index as i32)]
                    )]
                })).collect::<Vec<_>>()
            })
        })
        .collect();
    json!({
        "id": stimulus.stimulus_id,
        "type": "table",
        "sourceAnchors": [anchor(ctx, vec![stimulus.table_id.clone()], stimulus.page_index as i32)],
        "provenanceStatus": "source",
        "rows": rows,
        "sourceTableId": stimulus.table_id,
        "visualFallbackAssetId": stimulus.asset_id
    })
}

/// 视觉刺激 → figure 节点（crop 已物化时）。
fn figure_stimulus_node(
    ctx: &AnchorContext,
    stimulus: &VisualStimulusCandidateV1,
) -> Value {
    let hotspots: Vec<Value> = stimulus
        .hotspots
        .iter()
        .map(|hotspot| {
            json!({
                "hotspotId": format!("{}-hotspot-{}", stimulus.stimulus_id, hotspot.question_number),
                "slotId": format!("q{}", hotspot.question_number),
                "normalizedRect": hotspot.normalized_rect
            })
        })
        .collect();
    json!({
        "id": stimulus.stimulus_id,
        "type": "figure",
        "sourceAnchors": [anchor(ctx, vec![stimulus.region_id.clone()], stimulus.page_index as i32)],
        "provenanceStatus": "source",
        "assetId": stimulus.asset_id,
        "hotspots": hotspots,
        "display": {}
    })
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
fn build_task_group(
    job: &ImportJob,
    graph: &QuestionLayoutGraphV1,
    group: &TaskGroupCandidateV1,
    blocks_by_number: &BTreeMap<u32, &super::local::QuestionBlockCandidateV1>,
    banks_by_id: &BTreeMap<String, &OptionBankCandidateV1>,
    tables_by_id: &BTreeMap<String, &TableStimulusCandidateV1>,
    visuals_by_id: &BTreeMap<String, &VisualStimulusCandidateV1>,
    slots_out: &mut Vec<Value>,
    ctx: &AnchorContext,
) -> Value {
    let resolved_type: Option<TaskTypeV2> = group.task_type.clone();
    let has_resolved_type = resolved_type.is_some();
    let task_type = resolved_type.unwrap_or(TaskTypeV2::ShortAnswer);
    let type_name = task_type_name(task_type.clone());
    let bank = group
        .option_bank_ref
        .as_deref()
        .and_then(|bank_id| banks_by_id.get(bank_id))
        .copied();
    let bank_bound = bank.is_some();
    let scope = group.group_id.as_str();

    let zone = graph
        .instruction_zones
        .iter()
        .find(|zone| {
            zone.question_range
                .map(|[start, end]| {
                    group
                        .question_numbers
                        .first()
                        .is_some_and(|first| *first >= start)
                        && group
                            .question_numbers
                            .last()
                            .is_some_and(|last| *last <= end)
                })
                .unwrap_or(false)
        })
        .or_else(|| graph.instruction_zones.first());

    let mut warnings: Vec<String> = group.issues.clone();
    if !has_resolved_type {
        warnings.push(issue_codes::INSTRUCTION_SIGNATURE_UNRESOLVED.to_string());
    }

    let instruction_anchor = zone
        .and_then(|zone| zone.source_anchor.as_ref())
        .map(typed_anchor_value)
        .unwrap_or_else(|| anchor(ctx, vec![group.group_id.clone()], 0));
    // 组级锚点必须指向真实物理节点（组 1-5）：优先指令区的 nodeIds。
    let group_anchor_ids: Vec<String> = zone
        .and_then(|zone| zone.source_anchor.as_ref())
        .map(|candidate| candidate.node_ids.clone())
        .unwrap_or_default();
    let instruction_text = zone.map(|zone| zone.text.clone()).unwrap_or_default();
    let instructions = vec![text_node(
        &format!("{}-instructions", group.group_id),
        &instruction_text,
        vec![instruction_anchor.clone()],
    )];

    // 组 1-2：Multiple choice cardinality 来自指令解析；无法确定 → blocker，不默认 1。
    let mut extra_blockers: Vec<String> = Vec::new();
    let selection_cardinality = if matches!(task_type, TaskTypeV2::MultipleChoice) {
        match zone.and_then(|zone| parse_choose_cardinality(&zone.text)) {
            Some(exact) => Some(json!({"min": exact, "max": exact, "exact": exact})),
            None => {
                extra_blockers.push(MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED.to_string());
                warnings.push(MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED.to_string());
                None
            }
        }
    } else {
        None
    };
    let mut signature = json!({
        "normalizedText": instruction_text,
        "taskType": type_name,
        "expectedQuestionNumbers": group.question_numbers,
        "expectedSlotCount": group.question_numbers.len() as u32,
        "evidenceAnchors": [instruction_anchor],
        "confidence": zone.map(|zone| zone.confidence).unwrap_or(0.0)
    });
    if let Some(cardinality) = selection_cardinality {
        signature["selectionCardinality"] = cardinality;
    }

    let option_bank = bank.map(|bank| option_bank_value(ctx, scope, bank));

    let mut stimulus = Vec::new();
    for stimulus_ref in &group.stimulus_refs {
        if let Some(table) = tables_by_id.get(stimulus_ref) {
            stimulus.push(table_stimulus_node(ctx, table));
        }
    }

    // 组 1-3：题块按声明题号驱动；缺失题块保留题号 + 稳定 blocker，不消失。
    let mut response_groups = Vec::new();
    let mut missing_numbers = Vec::new();
    for number in &group.question_numbers {
        let slot_id = format!("q{number}");
        let Some(block) = blocks_by_number.get(number).copied() else {
            missing_numbers.push(*number);
            slots_out.push(json!({
                "slotId": slot_id,
                "questionNumber": number,
                "displayLabel": number.to_string(),
                "hostNodeId": group.group_id,
                "hostType": "prompt",
                "interaction": interaction_for(&task_type, bank_bound),
                "participation": "scoring",
                "sourceAnchors": if group_anchor_ids.is_empty() {
                    Vec::new()
                } else {
                    vec![anchor(ctx, group_anchor_ids.clone(), group.page_indices.first().copied().unwrap_or(0) as i32)]
                },
                "confidence": group.confidence
            }));
            response_groups.push(json!({
                "responseGroupId": format!("{}-response-{number}", group.group_id),
                "kind": response_kind(&task_type, bank_bound),
                "prompt": [],
                "slotIds": [slot_id],
                "cardinality": {"min": 1, "max": 1, "exact": 1},
                "assignment": "per_slot",
                "scoringPolicy": "per_slot_binary",
                "duplicatePolicy": "reject_submission",
                "allowOptionReuse": bank_bound,
                "sourceAnchors": []
            }));
            continue;
        };
        let stem_anchors = vec![anchor(ctx, block.stem_node_ids.clone(), block.page_index as i32)];
        let is_bank_bound = bank_bound
            || matches!(
                task_type,
                TaskTypeV2::MatchingHeadings
                    | TaskTypeV2::MatchingInformation
                    | TaskTypeV2::MatchingFeatures
                    | TaskTypeV2::MatchingSentenceEndings
                    | TaskTypeV2::Classification
            );
        let response_scope = format!("{}-response-{number}", group.group_id);
        let mut options = match (block.option_run.as_ref(), bank) {
            (Some(run), _) => run
                .options
                .iter()
                .map(|option| option_value(ctx, &response_scope, option))
                .collect::<Vec<_>>(),
            (None, Some(_)) if !is_bank_bound => Vec::new(),
            _ => Vec::new(),
        };
        // 语句选项按题型生成，不依赖共享库（组 1-1）；不伪造 source node（组 1-5）。
        if options.is_empty()
            && matches!(
                task_type,
                TaskTypeV2::TrueFalseNotGiven | TaskTypeV2::YesNoNotGiven
            )
        {
            options = statement_options(task_type.clone(), &slot_id);
        }
        let prompt_nodes = vec![text_node(
            &format!("{}-stem", block.candidate_id),
            &block.stem_text,
            stem_anchors.clone(),
        )];
        let mut response = json!({
            "responseGroupId": format!("{}-response-{number}", group.group_id),
            "kind": response_kind(&task_type, bank_bound),
            "prompt": prompt_nodes,
            "slotIds": [slot_id],
            "cardinality": {"min": 1, "max": 1, "exact": 1},
            "assignment": "per_slot",
            "scoringPolicy": "per_slot_binary",
            "duplicatePolicy": "reject_submission",
            "allowOptionReuse": bank_bound,
            "sourceAnchors": stem_anchors
        });
        if !options.is_empty() {
            response["options"] = json!(options);
        }
        if bank.is_some() {
            response["optionBankRef"] = json!(group.option_bank_ref.clone().unwrap_or_default());
        }
        response_groups.push(response);
        slots_out.push(json!({
            "slotId": slot_id,
            "questionNumber": number,
            "displayLabel": number.to_string(),
            "hostNodeId": block.stem_node_ids.first().cloned().unwrap_or(block.candidate_id.clone()),
            "hostType": "prompt",
            "interaction": interaction_for(&task_type, is_bank_bound),
            "participation": "scoring",
            "sourceAnchors": [block
                .number_anchor
                .as_ref()
                .map(typed_anchor_value)
                .unwrap_or_else(|| anchor(ctx, vec![block.candidate_id.clone()], block.page_index as i32))],
            "confidence": block.boundary_confidence
        }));
    }
    if !missing_numbers.is_empty() {
        warnings.push(QUESTION_BLOCK_MISSING.to_string());
        extra_blockers.push(QUESTION_BLOCK_MISSING.to_string());
    }

    let display_range = group.display_range.unwrap_or_else(|| {
        let first = group.question_numbers.first().copied().unwrap_or(1);
        let last = group.question_numbers.last().copied().unwrap_or(first);
        [first, last]
    });

    let mut built = json!({
        "taskId": group.group_id,
        "displayRange": {"kind": "range", "start": display_range[0], "end": display_range[1]},
        "taskType": type_name,
        "instructions": instructions,
        "instructionSignature": signature,
        "recognitionWarnings": warnings,
        "responseGroups": response_groups,
        "sourceAnchors": if group_anchor_ids.is_empty() {
            Vec::new()
        } else {
            vec![anchor(ctx, group_anchor_ids.clone(), group.page_indices.first().copied().unwrap_or(0) as i32)]
        },
        "quality": {
            "score": 0.0,
            "sourceCoverage": 0.0,
            "hardFailures": if !has_resolved_type {
                vec![issue_codes::INSTRUCTION_SIGNATURE_UNRESOLVED.to_string()]
            } else {
                extra_blockers.clone()
            }
        },
        "reviewState": "unreviewed"
    });
    if let Some(option_bank) = option_bank {
        built["optionBank"] = option_bank;
    }
    if !stimulus.is_empty() {
        built["stimulus"] = json!(stimulus);
    }
    let _ = job;
    built
}

/// 物理节点 id 全集（lines/regions/tables/assets），锚点语义校验的基准。
fn physical_node_ids(physical: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for value in [
        physical.pointer("/pages"),
        physical.get("regions"),
        physical.get("tables"),
    ]
    .into_iter()
    .flatten()
    {
        walk_collection_ids(value, &mut ids);
    }
    ids
}

fn walk_collection_ids(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| walk_collection_ids(item, ids)),
        Value::Object(map) => {
            if let Some(id) = map.get("id").and_then(Value::as_str) {
                ids.insert(id.to_string());
            }
            for child in map.values() {
                if child.is_array() || child.is_object() {
                    walk_collection_ids(child, ids);
                }
            }
        }
        _ => {}
    }
}

/// 输出中所有锚点 nodeIds 必须指向真实物理节点（组 1-5 语义校验）。
fn validate_anchor_targets(authoring: &Value, physical_ids: &BTreeSet<String>) -> Result<(), String> {
    fn walk(value: &Value, physical_ids: &BTreeSet<String>, errors: &mut Vec<String>) {
        match value {
            Value::Array(items) => items.iter().for_each(|item| walk(item, physical_ids, errors)),
            Value::Object(map) => {
                if map.get("nodeIds").and_then(Value::as_array).is_some() {
                    for node_id in map["nodeIds"].as_array().into_iter().flatten() {
                        if let Some(node_id) = node_id.as_str() {
                            if !physical_ids.contains(node_id) {
                                errors.push(node_id.to_string());
                            }
                        }
                    }
                }
                for child in map.values() {
                    if child.is_array() || child.is_object() {
                        walk(child, physical_ids, errors);
                    }
                }
            }
            _ => {}
        }
    }
    let mut errors = Vec::new();
    walk(authoring, physical_ids, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{ANCHOR_TARGET_INVALID}:{}",
            errors.into_iter().take(5).collect::<Vec<_>>().join(",")
        ))
    }
}

/// direct canonical 入口：QLG + 物理 DocumentIRV2 + pre-V1 答案候选 → canonical。
/// 不读取 V1 authoring。`asset_resolver` 决定视觉 crop 是否可物化。
#[allow(clippy::too_many_lines)]
pub(crate) fn build_direct_canonical(
    job: &ImportJob,
    graph: &QuestionLayoutGraphV1,
    physical: &Value,
    split: &Value,
    asset_resolver: &dyn Fn(&str) -> Option<ResolvedVisualAsset>,
) -> CommandResult<Value> {
    let blocks_by_number: BTreeMap<u32, &super::local::QuestionBlockCandidateV1> = graph
        .question_blocks
        .iter()
        .map(|block| (block.question_number, block))
        .collect();
    let banks_by_id: BTreeMap<String, &OptionBankCandidateV1> = graph
        .option_banks
        .iter()
        .map(|bank| (bank.bank_id.clone(), bank))
        .collect();
    let tables_by_id: BTreeMap<String, &TableStimulusCandidateV1> = graph
        .table_stimuli
        .iter()
        .map(|stimulus| (stimulus.stimulus_id.clone(), stimulus))
        .collect();
    let visuals_by_id: BTreeMap<String, &VisualStimulusCandidateV1> = graph
        .visual_stimuli
        .iter()
        .map(|stimulus| (stimulus.stimulus_id.clone(), stimulus))
        .collect();

    // 组 1-5：锚点上下文使用真实 job source file id 与绑定文件哈希。
    let ctx = AnchorContext {
        source_file_id: job
            .source_files
            .first()
            .map(|file| file.file_id.clone())
            .unwrap_or_else(|| "source-pdf-1".to_string()),
        source_hash: physical
            .pointer("/sourceFiles/0/sha256")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    };

    // passage：物理 passage 角色区域行文本，chunks(4) 含尾块，不丢行（P1-a 修复）。
    let mut passage_content = Vec::new();
    for (page_index, page) in graph.pages.iter().enumerate() {
        for region in &page.regions {
            if region.role != super::local::SemanticRegionRole::Passage {
                continue;
            }
            for (paragraph_index, line_slice) in region.child_line_ids.chunks(4).enumerate() {
                if line_slice.is_empty() {
                    continue;
                }
                let text = line_slice
                    .iter()
                    .filter_map(|line_id| {
                        physical
                            .pointer(&format!("/pages/{page_index}/lines"))
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .find(|line| {
                                line.get("id").and_then(Value::as_str) == Some(line_id.as_str())
                            })
                            .and_then(|line| line.get("text").and_then(Value::as_str))
                            .map(str::to_string)
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                if text.trim().is_empty() {
                    continue;
                }
                passage_content.push(text_node(
                    &format!("passage-p{page_index}-{paragraph_index}"),
                    &text,
                    vec![anchor(&ctx, line_slice.to_vec(), page_index as i32)],
                ));
            }
        }
    }
    let passage = json!({
        "title": job.title,
        "content": passage_content,
        "paragraphMap": {},
        "sourceAnchors": []
    });

    let mut slot_values: Vec<Value> = Vec::new();
    let task_groups: Vec<Value> = graph
        .task_groups
        .iter()
        .map(|group| {
            build_task_group(
                job,
                graph,
                group,
                &blocks_by_number,
                &banks_by_id,
                &tables_by_id,
                &visuals_by_id,
                &mut slot_values,
                &ctx,
            )
        })
        .collect();

    let answer_map = dynamic_answer_map_from_split(split);
    let mut answer_key = serde_json::Map::new();
    for slot in &slot_values {
        let Some(slot_id) = slot.get("slotId").and_then(Value::as_str) else {
            continue;
        };
        let number = slot.get("questionNumber").and_then(Value::as_u64).unwrap_or(0) as u32;
        answer_key.insert(
            slot_id.to_string(),
            answer_value_for_slot(&answer_map, slot_id, number),
        );
    }

    // 组 1-7：未分配证据按页聚合判定，不依赖单条 ≥ 阈值。
    let mut recognition_blockers = crate::recognition::blocking_issues_from_physical(Some(physical));
    let mut unassigned_by_page: BTreeMap<u32, usize> = BTreeMap::new();
    for evidence in &graph.unassigned_evidence {
        *unassigned_by_page.entry(evidence.page_index).or_insert(0) +=
            evidence.text_char_count;
    }
    if unassigned_by_page
        .values()
        .any(|chars| *chars >= SIGNIFICANT_UNASSIGNED_PAGE_CHARS)
    {
        recognition_blockers.push(issue_codes::SIGNIFICANT_REGION_UNASSIGNED.to_string());
    }
    for group in &graph.task_groups {
        for issue in &group.issues {
            if !recognition_blockers.contains(issue) {
                recognition_blockers.push(issue.clone());
            }
        }
    }
    for group in &task_groups {
        let mut group_blockers: Vec<String> = group
            .pointer("/quality/hardFailures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        group_blockers.extend(
            group
                .get("recognitionWarnings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|warning| {
                    matches!(
                        *warning,
                        MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED | QUESTION_BLOCK_MISSING
                    )
                })
                .map(str::to_string),
        );
        for code in group_blockers {
            if !recognition_blockers.contains(&code) {
                recognition_blockers.push(code);
            }
        }
    }

    // 组 1-4：视觉刺激物化——合法 crop → 顶层 assets + 组 stimulus figure；
    // 无法物化 → 发布 blocker（不落 warning）。
    let mut assets: Vec<Value> = Vec::new();
    let mut figure_stimuli: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for group in &graph.task_groups {
        for stimulus_ref in &group.stimulus_refs {
            if let Some(visual) = visuals_by_id.get(stimulus_ref) {
                let resolved = visual.asset_id.as_deref().and_then(asset_resolver);
                match resolved {
                    Some(asset) => {
                        assets.push(json!({
                            "assetId": visual.asset_id,
                            "kind": "page_crop",
                            "mime": "image/png",
                            "relativePath": asset.relative_path,
                            "sha256": asset.sha256,
                            "byteLength": asset.byte_length,
                            "extractionMode": "page_crop",
                            "altText": "Source-faithful figure crop"
                        }));
                        if !visual.hotspots.is_empty() {
                            figure_stimuli
                                .entry(group.group_id.clone())
                                .or_default()
                                .push(figure_stimulus_node(&ctx, visual));
                        }
                    }
                    None => {
                        if !recognition_blockers
                            .iter()
                            .any(|code| code == VISUAL_FALLBACK_ASSET_NOT_MATERIALIZED)
                        {
                            recognition_blockers
                                .push(VISUAL_FALLBACK_ASSET_NOT_MATERIALIZED.to_string());
                        }
                        let _ = group; // blocker 已在顶层 recognitionBlockers 记录
                    }
                }
            }
        }
    }

    let mut authoring = json!({
        "schemaVersion": "IeltsAuthoringIRV2",
        "jobId": job.job_id,
        "exam": {
            "examId": format!("{}-{}", job.category.clone().unwrap_or_else(|| "P1".to_string()).to_ascii_lowercase(), &job.job_id[job.job_id.len().saturating_sub(8)..]),
            "title": job.title,
            "category": job.category.clone().unwrap_or_else(|| "P1".to_string()),
            "frequency": job.frequency.clone().unwrap_or_else(|| "medium".to_string()),
            "language": "en",
            "tags": job.tags,
            "sourceFiles": job.source_files.iter().map(|file| json!({
                "sourceFileId": file.file_id,
                "role": if file.role.to_ascii_lowercase().contains("answer") { "answer_key" } else { "question_paper" }
            })).collect::<Vec<_>>()
        },
        "modality": "reading",
        "passage": passage,
        "taskGroups": task_groups,
        "answerSlots": {},
        "answerKey": answer_key,
        "assets": assets,
        "sourceDocumentId": graph.document_id,
        "quality": {},
        "audit": {
            "revision": 0,
            "source": "auto_extract",
            "humanVerified": false,
            "llmUsed": false,
            "updatedAt": job.updated_at.to_rfc3339(),
            "notes": ["Direct canonical from QuestionLayoutGraphV1 (G2-T04); no V1 authoring input."]
        }
    });
    let mut slot_map = serde_json::Map::new();
    for slot in &slot_values {
        if let Some(slot_id) = slot.get("slotId").and_then(Value::as_str) {
            slot_map.insert(slot_id.to_string(), slot.clone());
        }
    }
    authoring["answerSlots"] = Value::Object(slot_map);
    if !recognition_blockers.is_empty() {
        authoring["recognitionBlockers"] = json!(recognition_blockers);
    }
    if let Some(groups) = authoring.get_mut("taskGroups").and_then(Value::as_array_mut) {
        for group in groups.iter_mut() {
            if let Some(figures) =
                figure_stimuli.get(group.get("taskId").and_then(Value::as_str).unwrap_or_default())
            {
                let stimulus = group
                    .as_object_mut()
                    .expect("group must be an object")
                    .entry("stimulus".to_string())
                    .or_insert_with(|| json!([]));
                if let Some(items) = stimulus.as_array_mut() {
                    items.extend(figures.iter().cloned());
                }
            }
        }
    }

    // 组 1-5：锚点必须指向真实物理节点（构建期语义校验，失败 → 回退 V1 链）。
    let physical_ids = physical_node_ids(physical);
    validate_anchor_targets(&authoring, &physical_ids)?;

    let quality = crate::ielts_grammar::quality::evaluate_quality(&authoring, Some(physical));
    authoring["quality"] = quality;

    let typed: crate::schema::ielts_authoring_v2::IeltsAuthoringIRV2 =
        serde_json::from_value(authoring.clone())
            .map_err(|error| format!("direct_canonical_schema_validation_failed:{error}"))?;
    let _ = typed;
    Ok(authoring)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEST_SOURCE_FILE_ID: &str = "phase4-metrics-fixture";

    fn sample_job() -> ImportJob {
        crate::job_store::make_job(crate::CreateJobInput {
            title: Some("Direct canonical sample".to_string()),
            category: Some("P1".to_string()),
            frequency: None,
            tags: None,
            llm_profile_id: None,
        })
    }

    fn anchor_of(node_id: &str) -> Value {
        json!({"sourceFileId": TEST_SOURCE_FILE_ID, "pageIndex": 0,
            "sourceHash": "a".repeat(64), "nodeIds": [node_id], "extractionMode": "pdf_native"})
    }

    fn sample_physical() -> Value {
        json!({
            "schemaVersion": "DocumentIRV2",
            "documentId": "doc-1",
            "jobId": "job-1",
            "sourceFiles": [{"sha256": "a".repeat(64)}],
            "pages": [{"pageIndex": 0, "lines": (0..8)
                .map(|index| json!({"id": format!("line-{index}"), "text": format!("passage line {index} details")}))
                .collect::<Vec<_>>()}],
            "assets": []
        })
    }

    fn sample_split() -> Value {
        json!({"answerKeyCandidates": [{"kind": "inline", "answers": {"q1": "TRUE", "q2": "FALSE", "q3": "NOT GIVEN"}}]})
    }

    fn no_assets(_: &str) -> Option<ResolvedVisualAsset> {
        None
    }

    fn blockers_of(built: &Value) -> Vec<String> {
        built
            .get("recognitionBlockers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn sample_graph() -> QuestionLayoutGraphV1 {
        serde_json::from_value(json!({
            "schemaVersion": "QuestionLayoutGraphV1",
            "documentId": "doc-1",
            "jobId": "job-1",
            "pages": [{
                "pageIndex": 0, "widthPt": 612.0, "heightPt": 792.0, "rotation": 0,
                "regions": [{"regionId": "line-1", "kind": "text", "role": "passage",
                    "roleConfidence": 0.9, "roleFeatures": [],
                    "bbox": {"x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                    "childLineIds": ["line-1", "line-2", "line-3", "line-4"]}],
                "numberTokens": [], "textNodeCount": 4
            }],
            "instructionZones": [{
                "zoneId": "line-5", "pageIndex": 0, "regionId": "line-5",
                "questionRange": [1, 3], "expectedNumbers": [1, 2, 3],
                "taskHint": "true false not given",
                "text": "Questions 1-3 Do the following statements agree with the information?",
                "sourceAnchor": anchor_of("line-5"), "confidence": 0.9
            }],
            "questionBlocks": [
                {"candidateId": "block-1", "questionNumber": 1, "pageIndex": 0,
                 "numberAnchor": anchor_of("line-5"),
                 "numberBbox": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                 "stemNodeIds": ["line-5"], "stemText": "The passage mentions chili peppers.",
                 "optionRun": null, "sharedOptionBankRef": null, "visualObjectRefs": [],
                 "sourceCoverage": 1.0, "boundaryConfidence": 0.9, "ambiguities": []},
                {"candidateId": "block-2", "questionNumber": 2, "pageIndex": 0,
                 "numberAnchor": anchor_of("line-5"),
                 "numberBbox": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                 "stemNodeIds": ["line-5"], "stemText": "Chili peppers originated in Asia.",
                 "optionRun": null, "sharedOptionBankRef": null, "visualObjectRefs": [],
                 "sourceCoverage": 1.0, "boundaryConfidence": 0.9, "ambiguities": []},
                {"candidateId": "block-3", "questionNumber": 3, "pageIndex": 0,
                 "numberAnchor": anchor_of("line-5"),
                 "numberBbox": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                 "stemNodeIds": ["line-5"], "stemText": "Peppers were always considered spicy.",
                 "optionRun": null, "sharedOptionBankRef": null, "visualObjectRefs": [],
                 "sourceCoverage": 1.0, "boundaryConfidence": 0.9, "ambiguities": []}
            ],
            "taskGroups": [{
                "groupId": "group-1", "pageIndices": [0], "displayRange": [1, 3],
                "questionNumbers": [1, 2, 3], "taskType": "true_false_not_given",
                "taskHint": "true false not given", "blockIds": ["block-1", "block-2", "block-3"],
                "optionBankRef": null, "stimulusRefs": [], "issues": [], "confidence": 0.9
            }],
            "optionBanks": [],
            "visualStimuli": [],
            "tableStimuli": [],
            "unassignedEvidence": []
        })).expect("sample graph must deserialize")
    }

    /// 组 1-1 正向：YNNG 生成 YES/NO/NOT GIVEN，与 TFNG 不同（旧实现恒 TFNG，会失败）。
    #[test]
    fn ynng_options_differ_from_tfng() {
        let job = sample_job();
        let mut graph = sample_graph();
        graph.task_groups[0].task_type = Some(TaskTypeV2::YesNoNotGiven);
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split(), &no_assets)
            .expect("must build");
        let labels: Vec<String> = built["taskGroups"][0]["responseGroups"][0]["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|option| option["label"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(labels, vec!["YES", "NO", "NOT GIVEN"]);
    }

    /// 组 1-2 正反：Choose TWO → exact=2；无法解析 → blocker 且不写默认 cardinality。
    #[test]
    fn multiple_choice_cardinality_from_instruction() {
        let job = sample_job();
        let mut graph = sample_graph();
        graph.task_groups[0].task_type = Some(TaskTypeV2::MultipleChoice);
        graph.instruction_zones[0].text = "Questions 1-3 Choose TWO letters, A-E.".to_string();
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split(), &no_assets)
            .expect("must build");
        let signature = &built["taskGroups"][0]["instructionSignature"];
        assert_eq!(signature.pointer("/selectionCardinality/exact").cloned(), Some(json!(2)));
        assert!(!blockers_of(&built).iter().any(|code| code == MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED));

        graph.instruction_zones[0].text = "Questions 1-3 Pick letters.".to_string();
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split(), &no_assets)
            .expect("must build");
        assert!(blockers_of(&built).iter().any(|code| code == MULTIPLE_CHOICE_CARDINALITY_UNRESOLVED));
        assert!(
            built["taskGroups"][0]["instructionSignature"]
                .get("selectionCardinality")
                .is_none(),
            "未解析时不得默认 cardinality"
        );
    }

    /// 组 1-3 正反：缺失题块保留声明题号 + blocker，分母不消失（旧实现 continue 会失败）。
    #[test]
    fn missing_block_keeps_declared_question_number() {
        let job = sample_job();
        let mut graph = sample_graph();
        graph.task_groups[0].question_numbers = vec![1, 2];
        graph.question_blocks.retain(|block| block.question_number != 2);
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split(), &no_assets)
            .expect("must build");
        let slots = built.get("answerSlots").and_then(Value::as_object).unwrap();
        assert_eq!(slots.len(), 2, "声明题号必须保留");
        assert!(slots.contains_key("q2"), "缺失题块对应的题号不得消失");
        assert!(blockers_of(&built).iter().any(|code| code == QUESTION_BLOCK_MISSING));
        let key = built.get("answerKey").and_then(Value::as_object).unwrap();
        assert_eq!(key.len(), 2, "answerKey 分母同步保留");
    }

    /// 组 1-4 正反：可物化 crop → 顶层 assets + figure stimulus；不可物化 → 发布 blocker。
    #[test]
    fn visual_stimulus_materialization_and_blocker() {
        let job = sample_job();
        let mut graph = sample_graph();
        graph.task_groups[0].task_type = Some(TaskTypeV2::DiagramLabelCompletion);
        graph.task_groups[0].stimulus_refs = vec!["visual-1".to_string()];
        graph.visual_stimuli = vec![serde_json::from_value(json!({
            "stimulusId": "visual-1", "pageIndex": 0, "regionId": "line-6",
            "kind": "figure",
            "bbox": {"x": 0.0, "y": 0.0, "width": 100.0, "height": 50.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
            "confidence": 0.9, "questionRefs": [1], "assetId": "crop-1",
            "hotspots": [{"slotId": "q1", "questionNumber": 1, "normalizedRect": [0.1, 0.1, 0.4, 0.4], "confidence": 0.9}],
            "issues": []
        })).unwrap()];

        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split(), &no_assets)
            .expect("must build");
        assert!(blockers_of(&built).iter().any(|code| code == VISUAL_FALLBACK_ASSET_NOT_MATERIALIZED));
        assert!(built.get("assets").and_then(Value::as_array).unwrap().is_empty());

        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split(), &|asset_id| {
            Some(ResolvedVisualAsset {
                sha256: "b".repeat(64),
                byte_length: 42,
                relative_path: format!("assets/{asset_id}.png"),
            })
        })
        .expect("must build");
        let assets = built.get("assets").and_then(Value::as_array).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0]["assetId"], "crop-1");
        let stimulus = built["taskGroups"][0].pointer("/stimulus/0").unwrap();
        assert_eq!(stimulus["type"], "figure");
        assert_eq!(stimulus["assetId"], "crop-1");
        assert_eq!(stimulus["hotspots"][0]["slotId"], "q1");
    }

    /// 组 1-5 正反：锚点用真实 sourceFileId；合成选项不伪造节点；伪造 id → 构建失败。
    #[test]
    fn anchors_use_real_source_file_and_physical_nodes() {
        let mut job = sample_job();
        job.source_files.push(crate::SourceFile {
            file_id: TEST_SOURCE_FILE_ID.to_string(),
            original_name: "sample.pdf".to_string(),
            stored_name: "sample.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: "a".repeat(64),
            size_bytes: 1,
            role: "MainQuestion".to_string(),
            imported_at: job.created_at,
        });
        let graph = sample_graph();
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split(), &no_assets)
            .expect("must build");
        let file_id = job.source_files[0].file_id.as_str();
        assert_eq!(
            built.pointer("/taskGroups/0/instructions/0/sourceAnchors/0/sourceFileId").cloned(),
            Some(json!(file_id)),
            "sourceFileId 必须来自真实 job source file"
        );
        for response in built["taskGroups"][0]["responseGroups"].as_array().unwrap() {
            for option in response.get("options").and_then(Value::as_array).unwrap_or(&Vec::new()) {
                assert!(
                    option.get("sourceAnchors").and_then(Value::as_array).is_some_and(|anchors| anchors.is_empty()),
                    "合成语句选项不得伪造 source node"
                );
                assert!(!option["content"][0]["text"].as_str().unwrap_or_default().is_empty());
            }
        }
        let mut bad_graph = sample_graph();
        bad_graph.question_blocks[0].stem_node_ids = vec!["fabricated-node".to_string()];
        let error = build_direct_canonical(&job, &bad_graph, &sample_physical(), &sample_split(), &no_assets)
            .expect_err("伪造锚点必须构建失败");
        assert!(error.contains(ANCHOR_TARGET_INVALID), "{error}");
    }

    /// 组 1-6：跨题组同 label 的 option id 按 scope 唯一（旧实现会碰撞）。
    #[test]
    fn option_ids_are_scope_unique() {
        let job = sample_job();
        let mut graph = sample_graph();
        graph.task_groups.push(serde_json::from_value(json!({
            "groupId": "group-2", "pageIndices": [0], "displayRange": [4, 5],
            "questionNumbers": [4, 5], "taskType": "single_choice",
            "taskHint": "choose", "blockIds": ["block-4", "block-5"],
            "optionBankRef": null, "stimulusRefs": [], "issues": [], "confidence": 0.9
        })).unwrap());
        for number in [4u32, 5u32] {
            graph.question_blocks.push(serde_json::from_value(json!({
                "candidateId": format!("block-{number}"), "questionNumber": number, "pageIndex": 0,
                "numberAnchor": anchor_of("line-5"),
                "numberBbox": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                "stemNodeIds": ["line-5"], "stemText": format!("Question {number} stem."),
                "optionRun": {"runId": format!("run-{number}"), "labels": ["A", "B"],
                    "options": [
                        {"label": "A", "labelNodeId": "line-5", "text": "choice a", "textNodeIds": ["line-5"], "bbox": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}},
                        {"label": "B", "labelNodeId": "line-5", "text": "choice b", "textNodeIds": ["line-5"], "bbox": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}}
                    ],
                    "labelColumnX": null, "confidence": 0.9},
                "sharedOptionBankRef": null, "visualObjectRefs": [],
                "sourceCoverage": 1.0, "boundaryConfidence": 0.9, "ambiguities": []
            })).unwrap());
        }
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split(), &no_assets)
            .expect("must build");
        let mut ids = Vec::new();
        for group in built["taskGroups"].as_array().unwrap() {
            for response in group["responseGroups"].as_array().unwrap_or(&Vec::new()) {
                for option in response.get("options").and_then(Value::as_array).unwrap_or(&Vec::new()) {
                    ids.push(option["optionId"].as_str().unwrap_or_default().to_string());
                }
            }
        }
        let unique = ids.iter().cloned().collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), unique.len(), "option id 跨组碰撞：{ids:?}");
    }

    /// 组 1-7：多条短证据按页聚合越过阈值 → blocker（旧单条判定漏报）。
    #[test]
    fn unassigned_evidence_aggregates_by_page() {
        let job = sample_job();
        let mut graph = sample_graph();
        graph.unassigned_evidence = (0..3)
            .map(|index| {
                serde_json::from_value(json!({
                    "sourceNodeId": format!("line-{index}"), "pageIndex": 0,
                    "reason": "no question interval", "textPreview": "x",
                    "textCharCount": 100,
                    "bbox": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}
                })).unwrap()
            })
            .collect();
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split(), &no_assets)
            .expect("must build");
        assert!(blockers_of(&built).iter().any(|code| code == issue_codes::SIGNIFICANT_REGION_UNASSIGNED));
    }

    /// 契约主干：TFNG 正常路径 + schema 门。
    #[test]
    fn direct_canonical_contract_on_synthetic_graph() {
        let job = sample_job();
        let graph = sample_graph();
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split(), &no_assets)
            .expect("must build");
        assert_eq!(built["schemaVersion"], "IeltsAuthoringIRV2");
        let groups = built.get("taskGroups").and_then(Value::as_array).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["taskType"], "true_false_not_given");
        let options = groups[0]["responseGroups"][0]["options"].as_array().unwrap();
        assert_eq!(options.len(), 3);
        assert_eq!(options[0]["label"], "TRUE");
        let slots = built.get("answerSlots").and_then(Value::as_object).unwrap();
        assert_eq!(slots.len(), 3);
        let key = built.get("answerKey").and_then(Value::as_object).unwrap();
        assert_eq!(key.len(), slots.len());
        let typed: Result<crate::schema::ielts_authoring_v2::IeltsAuthoringIRV2, _> =
            serde_json::from_value(built.clone());
        assert!(typed.is_ok(), "direct 产物必须通过 schema：{:?}", typed.err());
    }

    /// verifier P1-a 复审：passage 分段含尾块，行数非 4 倍数不丢行。
    #[test]
    fn passage_chunks_preserve_tail_lines() {
        let job = sample_job();
        let mut graph = sample_graph();
        graph.pages[0].regions[0].child_line_ids =
            (0..7).map(|index| format!("line-{index}")).collect();
        let physical = json!({
            "schemaVersion": "DocumentIRV2", "documentId": "doc-1", "jobId": "job-1",
            "sourceFiles": [{"sha256": "a".repeat(64)}],
            "pages": [{"pageIndex": 0, "lines": (0..7).map(|index| json!({
                "id": format!("line-{index}"), "text": format!("passage line {index} details")
            })).collect::<Vec<_>>()}], "assets": []
        });
        let built = build_direct_canonical(&job, &graph, &physical, &sample_split(), &no_assets)
            .expect("must build");
        let content = built.pointer("/passage/content").and_then(Value::as_array).unwrap();
        let text = content
            .iter()
            .map(|node| node["text"].as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" ");
        for index in 0..7 {
            assert!(text.contains(&format!("passage line {index}")), "丢行 {index}");
        }
        assert_eq!(content.len(), 2, "7 行按 4 行分段应为 2 段（含尾块）");
    }
}
