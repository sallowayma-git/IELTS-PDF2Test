//! G2-T04（第一片）：`QuestionLayoutGraphV1` → `IeltsAuthoringIRV2` 直出。
//!
//! 产品的 direct canonical builder：从 QLG（指令区/题块/选项串/共享选项库/
//! 表格刺激/视觉刺激/未分配证据）+ 物理 `DocumentIRV2` + pre-V1 答案候选
//! （split 的 answerKeyCandidates，候选阶段产物，非 V1 authoring 输出）构建
//! canonical。**不读取 V1 authoring**（authoring-ir.json / ReadingAuthoringIrV1）。
//!
//! Feature flag：`QLG_DIRECT_CANONICAL`（默认关闭，见 `environment.rs`）。
//! 关闭时主链行为与历史完全一致（可回滚）。
//!
//! 诚实降级原则（计划 §6.8）：
//! - QLG 未解析出题型 → 题组保留（不丢内容），`recognitionWarnings` 记录
//!   `INSTRUCTION_SIGNATURE_UNRESOLVED`，`recognitionBlockers` 携带同一稳定码；
//! - 未分配证据超阈值 → `SIGNIFICANT_REGION_UNASSIGNED` 阻断码；
//! - 答案候选缺失 → answerKey 记 `unresolved`（发布门会拦，用户可补）。

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::local::{
    OptionBankCandidateV1, QuestionLayoutGraphV1, TableStimulusCandidateV1,
    TaskGroupCandidateV1, VisualStimulusCandidateV1,
};
use crate::authoring_pipeline::dynamic_answer_map_from_split;
use crate::ielts_grammar::answer_key::answer_value_for_slot;
use crate::ielts_grammar::issue_codes;
use crate::schema::ielts_authoring_v2::TaskTypeV2;
use crate::{ImportJob, CommandResult};

/// §6.11：超过该字符数的未分配正文必须产生稳定阻断码。
const SIGNIFICANT_UNASSIGNED_CHAR_THRESHOLD: usize = 200;

fn typed_anchor_value(anchor: &crate::schema::common::SourceAnchorV2) -> Value {
    serde_json::to_value(anchor).unwrap_or(Value::Null)
}

fn anchor(source_hash: &str, node_ids: Vec<String>, page_index: i32) -> Value {
    json!({
        "sourceFileId": "source-pdf-1",
        "sourceHash": source_hash,
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

fn interaction_for(task_type: TaskTypeV2, bank_bound: bool) -> &'static str {
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

/// 把 QLG 选项（label+text+节点 id）编译成 `OptionV2` 形状（label/content/sourceAnchors）。
fn option_value(source_hash: &str, option: &super::local::OptionCandidateV2) -> Value {
    let mut node_ids = option.text_node_ids.clone();
    node_ids.push(option.label_node_id.clone());
    let anchors = vec![anchor(source_hash, node_ids, 0)];
    json!({
        "optionId": format!("opt-{}", option.label.to_ascii_lowercase()),
        "label": option.label,
        "content": [text_node(
            &format!("opt-{}-text", option.label.to_ascii_lowercase()),
            &option.text,
            anchors.clone()
        )],
        "sourceAnchors": anchors
    })
}

fn option_bank_value(source_hash: &str, bank: &OptionBankCandidateV1) -> Value {
    json!({
        "optionBankId": bank.bank_id,
        "scope": "task_group",
        "options": bank.options.iter().map(|option| option_value(source_hash, option)).collect::<Vec<_>>(),
        "allowReuse": true,
        "sourceAnchors": bank.options.first().map(|first| {
            let mut ids = first.text_node_ids.clone();
            ids.push(first.label_node_id.clone());
            vec![anchor(source_hash, ids, bank.page_index as i32)]
        }).unwrap_or_default()
    })
}

/// QLG 表格刺激 → 语义表格内容节点（保留物理行/列/跨行拓扑，§6.10）。
fn table_stimulus_node(source_hash: &str, stimulus: &TableStimulusCandidateV1) -> Value {
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
                    "sourceAnchors": [anchor(source_hash, vec![cell.cell_id.clone()], stimulus.page_index as i32)],
                    "provenanceStatus": "source",
                    "rowSpan": cell.row_span,
                    "colSpan": cell.col_span,
                    "children": [text_node(
                        &format!("{}-cell-{}-{}-text", stimulus.stimulus_id, cell.row, cell.col),
                        &cell.text,
                        vec![anchor(source_hash, vec![cell.cell_id.clone()], stimulus.page_index as i32)]
                    )]
                })).collect::<Vec<_>>()
            })
        })
        .collect();
    json!({
        "id": stimulus.stimulus_id,
        "type": "table",
        "sourceAnchors": [anchor(source_hash, vec![stimulus.table_id.clone()], stimulus.page_index as i32)],
        "provenanceStatus": "source",
        "rows": rows,
        "sourceTableId": stimulus.table_id,
        "visualFallbackAssetId": stimulus.asset_id
    })
}

fn statement_options(source_hash: &str) -> Vec<Value> {
    ["TRUE", "FALSE", "NOT GIVEN"]
        .iter()
        .map(|label| {
            let option_id = format!("opt-{}", label.to_ascii_lowercase().replace(' ', "-"));
            json!({
                "optionId": option_id,
                "label": label,
                "content": [text_node(
                    &format!("{option_id}-text"),
                    label,
                    vec![anchor(source_hash, vec![option_id.clone()], 0)]
                )],
                "sourceAnchors": [anchor(source_hash, vec![option_id], 0)]
            })
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
fn build_task_group(
    job: &ImportJob,
    graph: &QuestionLayoutGraphV1,
    group: &TaskGroupCandidateV1,
    blocks_by_id: &BTreeMap<String, &super::local::QuestionBlockCandidateV1>,
    banks_by_id: &BTreeMap<String, &OptionBankCandidateV1>,
    tables_by_id: &BTreeMap<String, &TableStimulusCandidateV1>,
    visuals_by_id: &BTreeMap<String, &VisualStimulusCandidateV1>,
    slots_out: &mut Vec<Value>,
    source_hash: &str,
) -> Value {
    let resolved_type: Option<TaskTypeV2> = group.task_type.clone();
    let has_resolved_type = resolved_type.is_some();
    let task_type = resolved_type.clone().unwrap_or(TaskTypeV2::ShortAnswer);
    let type_name = task_type_name(task_type.clone());
    let bank = group
        .option_bank_ref
        .as_deref()
        .and_then(|bank_id| banks_by_id.get(bank_id))
        .copied();

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

    // 指令节点（带锚点或显式 manual 语义，quality 检查要求）。
    let instruction_anchor = zone
        .and_then(|zone| zone.source_anchor.as_ref())
        .map(typed_anchor_value)
        .unwrap_or_else(|| anchor(source_hash, vec![group.group_id.clone()], 0));
    let instruction_text = zone.map(|zone| zone.text.clone()).unwrap_or_default();
    let instructions = vec![text_node(
        &format!("{}-instructions", group.group_id),
        &instruction_text,
        vec![instruction_anchor.clone()],
    )];

    let signature = json!({
        "normalizedText": instruction_text,
        "taskType": type_name,
        "expectedQuestionNumbers": group.question_numbers,
        "expectedSlotCount": group.question_numbers.len() as u32,
        "evidenceAnchors": [instruction_anchor],
        "confidence": zone.map(|zone| zone.confidence).unwrap_or(0.0)
    });

    // 共享选项库（§6.9）。
    let option_bank = bank.map(|bank| option_bank_value(source_hash, bank));
    let bank_options: Vec<Value> = bank
        .map(|bank| bank.options.iter().map(|option| option_value(source_hash, option)).collect())
        .unwrap_or_default();

    // 刺激（表格拓扑 / 视觉 fallback）。
    let mut stimulus = Vec::new();
    for stimulus_ref in &group.stimulus_refs {
        if let Some(table) = tables_by_id.get(stimulus_ref) {
            stimulus.push(table_stimulus_node(source_hash, table));
        }
    }

    // 视觉刺激：有 crop 资产 → source-faithful fallback 记入 assets。
    let mut visual_assets = Vec::new();
    for stimulus_ref in &group.stimulus_refs {
        if let Some(visual) = visuals_by_id.get(stimulus_ref) {
            if let Some(asset_id) = &visual.asset_id {
                visual_assets.push(json!({
                    "assetId": asset_id,
                    "type": "figure_crop",
                    "altText": "Source-faithful figure crop",
                    "display": {},
                    "hotspots": visual.hotspots.iter().map(|hotspot| json!({
                        "slotId": hotspot.slot_id,
                        "questionNumber": hotspot.question_number,
                        "normalizedRect": hotspot.normalized_rect
                    })).collect::<Vec<_>>()
                }));
            }
        }
    }

    // 题块 → 响应组与槽位。
    let mut response_groups = Vec::new();
    let mut slots: Vec<Value> = Vec::new();
    for block_id in &group.block_ids {
        let Some(block) = blocks_by_id.get(block_id) else {
            continue;
        };
        let slot_id = format!("q{}", block.question_number);
        let stem_anchors = vec![anchor(source_hash, block.stem_node_ids.clone(), block.page_index as i32)];
        let is_bank_bound = bank.is_some()
            || matches!(&task_type, TaskTypeV2::MatchingHeadings
                | TaskTypeV2::MatchingInformation
                | TaskTypeV2::MatchingFeatures
                | TaskTypeV2::MatchingSentenceEndings
                | TaskTypeV2::Classification);
        let mut options = match (block.option_run.as_ref(), bank) {
            (Some(run), _) => run.options.iter().map(|option| option_value(source_hash, option)).collect::<Vec<_>>(),
            (None, Some(_)) if !is_bank_bound => Vec::new(),
            _ => Vec::new(),
        };
        // TFNG/YNNG 的语句选项由题型定义，不依赖共享库（verifier P2-a）。
        if options.is_empty()
            && matches!(task_type, TaskTypeV2::TrueFalseNotGiven | TaskTypeV2::YesNoNotGiven)
        {
            options = statement_options(source_hash);
        }
        let prompt_nodes = vec![text_node(
            &format!("{}-stem", block.candidate_id),
            &block.stem_text,
            stem_anchors.clone(),
        )];
        // kind 词表与主链 response_kind 对齐（choice/matching/diagram_hotspot/text_entry）。
        let kind = if is_bank_bound && matches!(task_type, TaskTypeV2::SummaryCompletion
                | TaskTypeV2::NoteCompletion | TaskTypeV2::TableCompletion | TaskTypeV2::FormCompletion
                | TaskTypeV2::SentenceCompletion | TaskTypeV2::FlowchartCompletion) {
            "matching"
        } else {
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
        };
        let mut response = json!({
            "responseGroupId": format!("{}-response-{}", group.group_id, block.question_number),
            "kind": kind,
            "prompt": prompt_nodes,
            "slotIds": [slot_id],
            "cardinality": {"min": 1, "max": 1, "exact": 1},
            "assignment": "per_slot",
            "scoringPolicy": "per_slot_binary",
            "duplicatePolicy": "reject_submission",
            "allowOptionReuse": bank.is_some(),
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
            "questionNumber": block.question_number,
            "displayLabel": block.question_number.to_string(),
            "hostNodeId": block.stem_node_ids.first().cloned().unwrap_or(block.candidate_id.clone()),
            "hostType": "prompt",
            "interaction": interaction_for(task_type.clone(), is_bank_bound),
            "participation": "scoring",
            "sourceAnchors": [block
                .number_anchor
                .as_ref()
                .map(typed_anchor_value)
                .unwrap_or_else(|| anchor(source_hash, vec![block.candidate_id.clone()], block.page_index as i32))],
            "confidence": block.boundary_confidence
        }));
    }
    // 共享库 matching：单个响应组承载全部 slot（per_slot + bank）。
    if response_groups.is_empty() && !group.question_numbers.is_empty() && bank.is_some() {
        let slot_ids: Vec<String> = group
            .question_numbers
            .iter()
            .map(|number| format!("q{number}"))
            .collect();
        for number in &group.question_numbers {
            slots_out.push(json!({
                "slotId": format!("q{number}"),
                "questionNumber": number,
                "displayLabel": number.to_string(),
                "hostNodeId": group.group_id,
                "hostType": "prompt",
                "interaction": "select",
                "participation": "scoring",
                "sourceAnchors": [anchor(source_hash, vec![group.group_id.clone()], group.page_indices.first().copied().unwrap_or(0) as i32)],
                "confidence": group.confidence
            }));
        }
        response_groups.push(json!({
            "responseGroupId": format!("{}-responses", group.group_id),
            "kind": "matching",
            "prompt": [],
            "slotIds": slot_ids,
            "cardinality": {"min": 1, "max": 1, "exact": 1},
            "assignment": "per_slot",
            "scoringPolicy": "per_slot_binary",
            "duplicatePolicy": "reject_submission",
            "allowOptionReuse": true,
            "options": bank_options,
            "optionBankRef": group.option_bank_ref,
            "sourceAnchors": []
        }));
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
        "sourceAnchors": [anchor(source_hash, vec![group.group_id.clone()], group.page_indices.first().copied().unwrap_or(0) as i32)],
        "quality": {
            "score": 0.0,
            "sourceCoverage": 0.0,
            "hardFailures": if !has_resolved_type {
                vec![issue_codes::INSTRUCTION_SIGNATURE_UNRESOLVED]
            } else {
                Vec::new()
            }
        }
    });
    if let Some(option_bank) = option_bank {
        built["optionBank"] = option_bank;
    }
    if !stimulus.is_empty() {
        built["stimulus"] = json!(stimulus);
    }
    // 视觉刺激的 crop 资产在本片尚未物化：如实记录，不伪造 fallback。
    if !visual_assets.is_empty() {
        for visual in &visual_assets {
            built["recognitionWarnings"]
                .as_array_mut()
                .unwrap()
                .push(json!(format!("VISUAL_FALLBACK_ASSET_NOT_MATERIALIZED:{}", visual["assetId"].as_str().unwrap_or_default())));
        }
    }
    // TaskGroupV2 契约要求 reviewState；quality 阶段会按 issues 更新。
    built["reviewState"] = json!("unreviewed");
    let _ = job;
    built
}

/// direct canonical 入口：QLG + 物理 DocumentIRV2 + pre-V1 答案候选 → canonical。
/// 不读取 V1 authoring。
#[allow(clippy::too_many_lines)]
pub(crate) fn build_direct_canonical(
    job: &ImportJob,
    graph: &QuestionLayoutGraphV1,
    physical: &Value,
    split: &Value,
) -> CommandResult<Value> {
    let blocks_by_id: BTreeMap<String, &super::local::QuestionBlockCandidateV1> = graph
        .question_blocks
        .iter()
        .map(|block| (block.candidate_id.clone(), block))
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

    // 锚点的 sourceHash 必须与物理源文件一致（quality/coverage 依赖锚点匹配）。
    let source_hash = physical
        .pointer("/sourceFiles/0/sha256")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // passage：物理 DocumentIRV2 的 passage 角色区域行文本，逐段节点。
    let mut passage_content = Vec::new();
    for (page_index, page) in graph.pages.iter().enumerate() {
        for region in &page.regions {
            if region.role != super::local::SemanticRegionRole::Passage {
                continue;
            }
            // chunks(4) 天然包含尾块：任何行数都完整入正文，不丢尾行（verifier P1-a）。
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
                            .find(|line| line.get("id").and_then(Value::as_str) == Some(line_id.as_str()))
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
                    vec![anchor(source_hash.as_str(), line_slice.to_vec(), page_index as i32)],
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
                &blocks_by_id,
                &banks_by_id,
                &tables_by_id,
                &visuals_by_id,
                &mut slot_values,
                source_hash.as_str(),
            )
        })
        .collect();

    // 答案键：pre-V1 答案候选（split.answerKeyCandidates，已合并答案源文件）。
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

    // 未分配证据（§6.11）：超阈值 → 稳定阻断码。
    let mut recognition_blockers = crate::recognition::blocking_issues_from_physical(Some(physical));
    for evidence in &graph.unassigned_evidence {
        if evidence.text_char_count >= SIGNIFICANT_UNASSIGNED_CHAR_THRESHOLD {
            recognition_blockers.push(issue_codes::SIGNIFICANT_REGION_UNASSIGNED.to_string());
            break;
        }
    }
    for group in &graph.task_groups {
        for issue in &group.issues {
            if !recognition_blockers.contains(issue) {
                recognition_blockers.push(issue.clone());
            }
        }
    }
    for group in &task_groups {
        let unresolved = group
            .pointer("/quality/hardFailures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|code| code == issue_codes::INSTRUCTION_SIGNATURE_UNRESOLVED);
        if unresolved && !recognition_blockers.iter().any(|code| code == issue_codes::INSTRUCTION_SIGNATURE_UNRESOLVED) {
            recognition_blockers.push(issue_codes::INSTRUCTION_SIGNATURE_UNRESOLVED.to_string());
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
        "assets": [],
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
    // answerSlots：schema 是「slotId → slot」映射。
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

    let quality = crate::ielts_grammar::quality::evaluate_quality(&authoring, Some(physical));
    authoring["quality"] = quality;

    // 契约：产出必须能反序列化为 typed IeltsAuthoringIRV2（schema 门）。
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

    fn sample_anchor() -> Value {
        anchor("a", vec!["node-1".to_string()], 0)
    }

    fn sample_graph() -> QuestionLayoutGraphV1 {
        serde_json::from_value(json!({
            "schemaVersion": "QuestionLayoutGraphV1",
            "documentId": "doc-1",
            "jobId": "job-1",
            "pages": [{
                "pageIndex": 0, "widthPt": 612.0, "heightPt": 792.0, "rotation": 0,
                "regions": [{"regionId": "r-passage", "kind": "text", "role": "passage",
                    "roleConfidence": 0.9, "roleFeatures": [], "bbox": {"x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                    "childLineIds": ["line-1", "line-2", "line-3", "line-4"]}],
                "numberTokens": [], "textNodeCount": 4
            }],
            "instructionZones": [{
                "zoneId": "zone-1", "pageIndex": 0, "regionId": "r-questions",
                "questionRange": [1, 3], "expectedNumbers": [1, 2, 3],
                "taskHint": "Choose the correct letter", "text": "Questions 1-3 Choose the correct letter A-D.",
                "sourceAnchor": {"sourceFileId": "source-pdf-1", "pageIndex": 0, "sourceHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "nodeIds": ["zone-1"], "extractionMode": "pdf_native"},
                "confidence": 0.9
            }],
            "questionBlocks": [
                {"candidateId": "block-1", "questionNumber": 1, "pageIndex": 0,
                 "numberAnchor": {"sourceFileId": "source-pdf-1", "pageIndex": 0, "sourceHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "nodeIds": ["n1"], "extractionMode": "pdf_native"},
                 "numberBbox": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                 "stemNodeIds": ["s1"], "stemText": "The passage mentions chili peppers.",
                 "optionRun": {"runId": "run-1", "labels": ["A", "B", "C", "D"],
                     "options": [
                         {"label": "A", "labelNodeId": "oa", "text": "first", "textNodeIds": ["ta"], "bbox": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}},
                         {"label": "B", "labelNodeId": "ob", "text": "second", "textNodeIds": ["tb"], "bbox": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}},
                         {"label": "C", "labelNodeId": "oc", "text": "third", "textNodeIds": ["tc"], "bbox": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}},
                         {"label": "D", "labelNodeId": "od", "text": "fourth", "textNodeIds": ["td"], "bbox": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}}
                     ],
                     "labelColumnX": null, "confidence": 0.9},
                 "sharedOptionBankRef": null, "visualObjectRefs": [],
                 "sourceCoverage": 1.0, "boundaryConfidence": 0.9, "ambiguities": []},
                {"candidateId": "block-2", "questionNumber": 2, "pageIndex": 0,
                 "numberAnchor": {"sourceFileId": "source-pdf-1", "pageIndex": 0, "sourceHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "nodeIds": ["n2"], "extractionMode": "pdf_native"},
                 "numberBbox": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                 "stemNodeIds": ["s2"], "stemText": "Chili peppers originated in Asia.",
                 "optionRun": null, "sharedOptionBankRef": null, "visualObjectRefs": [],
                 "sourceCoverage": 1.0, "boundaryConfidence": 0.9, "ambiguities": []},
                {"candidateId": "block-3", "questionNumber": 3, "pageIndex": 0,
                 "numberAnchor": {"sourceFileId": "source-pdf-1", "pageIndex": 0, "sourceHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "nodeIds": ["n3"], "extractionMode": "pdf_native"},
                 "numberBbox": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                 "stemNodeIds": ["s3"], "stemText": "Peppers were always considered spicy.",
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
            "tableStimuli": [{
                "stimulusId": "table-1", "pageIndex": 0, "tableId": "phys-table-1",
                "bbox": {"x": 0.0, "y": 0.0, "width": 100.0, "height": 50.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                "rows": [{"row": 0, "cells": [{"cellId": "c-0-0", "row": 0, "col": 0, "rowSpan": 1, "colSpan": 2, "text": "Year / Event", "bbox": {"x": 0.0, "y": 0.0, "width": 50.0, "height": 10.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}}]}],
                "assetId": null, "confidence": 0.9, "issues": []
            }],
            "unassignedEvidence": [{"sourceNodeId": "line-9", "pageIndex": 0, "reason": "no question interval", "textPreview": "x", "textCharCount": 300, "bbox": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}}]
        })).expect("sample graph must deserialize")
    }

    fn sample_job() -> ImportJob {
        crate::job_store::make_job(crate::CreateJobInput {
            title: Some("Direct canonical sample".to_string()),
            category: Some("P1".to_string()),
            frequency: None,
            tags: None,
            llm_profile_id: None,
        })
    }

    fn sample_physical() -> Value {
        json!({
            "schemaVersion": "DocumentIRV2",
            "documentId": "doc-1",
            "jobId": "job-1",
            "pages": [{"pageIndex": 0, "lines": [
                {"id": "line-1", "text": "Chili peppers have a long history."},
                {"id": "line-2", "text": "They originated in the Americas."},
                {"id": "line-3", "text": "They spread across the world after Columbus."},
                {"id": "line-4", "text": "Today many varieties are cultivated."}
            ]}],
            "assets": []
        })
    }

    fn sample_split() -> Value {
        json!({"answerKeyCandidates": [{"kind": "inline", "answers": {"q1": "TRUE", "q2": "FALSE", "q3": "NOT GIVEN"}}]})
    }

    /// Scout C 契约检查单（QLG 携带范围内）：direct 产物的不变量。
    #[test]
    fn direct_canonical_contract_on_synthetic_graph() {
        let job = sample_job();
        let graph = sample_graph();
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split())
            .expect("direct canonical must build");
        assert_eq!(built["schemaVersion"], "IeltsAuthoringIRV2");
        assert_eq!(built["jobId"], job.job_id);
        assert!(
            !built.pointer("/passage/content").and_then(Value::as_array).unwrap().is_empty(),
            "passage 必须从物理层构建且非空"
        );

        let groups = built.get("taskGroups").and_then(Value::as_array).unwrap();
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group["taskType"], "true_false_not_given");
        // 指令带锚点（quality 检查要求）。
        assert_eq!(
            group.pointer("/instructions/0/sourceAnchors/0/nodeIds").and_then(Value::as_array).unwrap()[0],
            "zone-1"
        );
        // instructionSignature 必须带题型/期望题号/证据锚点。
        assert_eq!(group.pointer("/instructionSignature/taskType").cloned(), Some(json!("true_false_not_given")));
        assert_eq!(group.pointer("/instructionSignature/expectedQuestionNumbers").cloned(), Some(json!([1, 2, 3])));

        let response_groups = group.get("responseGroups").and_then(Value::as_array).unwrap();
        assert_eq!(response_groups.len(), 3, "三个题块各一个响应组");
        let first_options = response_groups[0].get("options").and_then(Value::as_array).unwrap();
        assert_eq!(first_options.len(), 4, "选项 A-D 不丢失");
        assert_eq!(first_options[0]["label"], "A");
        assert_eq!(first_options[0]["content"][0]["text"], "first");
        assert!(
            first_options[0].pointer("/sourceAnchors/0/nodeIds").is_some(),
            "选项必须携带 sourceAnchors"
        );

        let slots = built.get("answerSlots").and_then(Value::as_object).unwrap();
        assert_eq!(slots.len(), 3);
        assert_eq!(slots["q1"]["interaction"], "radio");
        assert_eq!(slots["q1"]["participation"], "scoring");

        // answerKey 键集 == answerSlots 键集（允许 unresolved，不允许缺键）。
        let key = built.get("answerKey").and_then(Value::as_object).unwrap();
        assert_eq!(key.len(), slots.len());
        for slot_id in slots.keys() {
            assert!(key.contains_key(slot_id), "answerKey 缺 {slot_id}");
        }
        assert_eq!(key["q2"]["kind"], "text");
        assert_eq!(key["q2"]["values"], json!(["FALSE"]));

        // 未分配证据超阈值 → 稳定阻断码（§6.11）。
        let blockers = built.get("recognitionBlockers").and_then(Value::as_array).unwrap();
        assert!(
            blockers.iter().any(|b| b == "SIGNIFICANT_REGION_UNASSIGNED"),
            "显著未分配正文必须产生稳定 blocker"
        );

        // schema 门：能反序列化回 typed IeltsAuthoringIRV2。
        let typed: Result<crate::schema::ielts_authoring_v2::IeltsAuthoringIRV2, _> =
            serde_json::from_value(built.clone());
        assert!(typed.is_ok(), "direct 产物必须通过 IeltsAuthoringIRV2 schema：{:?}", typed.err());
    }

    /// 表格拓扑保留：QLG 表格刺激 → 语义 table 节点（行/列/跨行不丢）。
    #[test]
    fn table_stimulus_preserves_topology() {
        let job = sample_job();
        let mut graph = sample_graph();
        graph.task_groups[0].task_type = Some(TaskTypeV2::TableCompletion);
        graph.task_groups[0].stimulus_refs = vec!["table-1".to_string()];
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split())
            .expect("direct canonical must build");
        let group = &built["taskGroups"][0];
        let stimulus = group.get("stimulus").and_then(Value::as_array).unwrap();
        assert_eq!(stimulus[0]["type"], "table");
        assert_eq!(stimulus[0]["sourceTableId"], "phys-table-1");
        let row = stimulus[0]["rows"][0].clone();
        let cell = row["cells"][0].clone();
        assert_eq!(cell["colSpan"], 2, "跨列 span 必须保留");
        assert_eq!(cell["children"][0]["text"], "Year / Event");
    }

    /// verifier P1-a 复审：passage 分段含尾块，行数非 4 倍数不丢行。
    #[test]
    fn passage_chunks_preserve_tail_lines() {
        let mut graph = sample_graph();
        graph.pages[0].regions[0].child_line_ids = (0..7).map(|index| format!("line-{index}")).collect();
        let physical = json!({
            "schemaVersion": "DocumentIRV2", "documentId": "doc-1", "jobId": "job-1",
            "pages": [{"pageIndex": 0, "lines": (0..7).map(|index| json!({
                "id": format!("line-{index}"), "text": format!("passage line {index}")
            })).collect::<Vec<_>>()}], "assets": []
        });
        let built = build_direct_canonical(&sample_job(), &graph, &physical, &sample_split())
            .expect("must build");
        let content = built.pointer("/passage/content").and_then(Value::as_array).unwrap();
        let text = content.iter().map(|node| node["text"].as_str().unwrap_or("")).collect::<Vec<_>>().join(" ");
        for index in 0..7 {
            assert!(text.contains(&format!("passage line {index}")), "丢行 {index}");
        }
        assert_eq!(content.len(), 2, "7 行按 4 行分段应为 2 段（含尾块）");
    }

    /// 题型未解析 → 诚实降级：题组保留 + 稳定阻断码，不伪造类型。
    #[test]
    fn unresolved_task_type_degrades_honestly() {
        let job = sample_job();
        let mut graph = sample_graph();
        graph.task_groups[0].task_type = None;
        let built = build_direct_canonical(&job, &graph, &sample_physical(), &sample_split())
            .expect("direct canonical must build");
        let group = &built["taskGroups"][0];
        assert!(
            group
                .get("recognitionWarnings")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .any(|warning| warning == "INSTRUCTION_SIGNATURE_UNRESOLVED"),
            "未解析题型必须记录"
        );
        let blockers = built.get("recognitionBlockers").and_then(Value::as_array).unwrap();
        assert!(blockers.iter().any(|b| b == "INSTRUCTION_SIGNATURE_UNRESOLVED"));
    }
}
