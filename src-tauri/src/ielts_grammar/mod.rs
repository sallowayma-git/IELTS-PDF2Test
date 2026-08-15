//! Phase 4 IELTS grammar and reliability shadow layer.
//!
//! The parser in this module consumes V1 split/authoring evidence and, when
//! available, the Phase 2/3 physical shadow lines.  It never writes either V1
//! artifact and it never calls an LLM.  The result is a separately validated
//! `IeltsAuthoringIRV2` shadow artifact that can be compared and reviewed before
//! any future promotion decision.

mod anchors;
mod answer_key;
mod completion;
mod diagram;
mod evidence;
mod instruction_signature;
mod instruction_zone;
mod issue_codes;
mod option_bank;
mod option_run;
mod prompt_assembler;
mod quality;
mod question_number;
mod reading;
#[cfg(test)]
mod real_pdf_acceptance;

pub(crate) use quality::evaluate_quality;

use crate::artifact_store::write_canonical_json_atomic;
use crate::schema::ielts_authoring_v2::{QuestionNumberExpressionV2, TaskTypeV2};
use crate::schema::IeltsAuthoringIRV2;
use crate::{CommandResult, ImportJob, SourceFile};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

use anchors::{detect_question_anchors, QuestionAnchor};
use answer_key::{answer_key_from_v1, answer_value_for_slot};
use completion::{answer_slot_node, completion_host_type, completion_placeholder};
use diagram::diagram_candidate;
use evidence::{anchor_from_value, source_anchor_from_job};
use instruction_signature::{infer_instruction_signature, is_completion_task, task_type_label};
use instruction_zone::{
    collect_instruction_zone, normalize_instruction_text, semantic_lines_from_v1_document,
    semantic_lines_from_v2_shadow, SemanticLine,
};
use option_bank::{detect_option_bank, option_bank_value};
use option_run::{detect_option_runs, option_run_value, run_matches_alphabet, OptionRun};
use prompt_assembler::assemble_prompt;
use question_number::{expand_expression, parse_question_expression};
use reading::{is_paper_section_header, passage_nodes, visual_passage_lines};

pub(crate) const SHADOW_ARTIFACT_FILE: &str = "authoring-ir-v2.shadow.json";
pub(crate) const SHADOW_COMPARE_FILE: &str = "authoring-ir-v2.shadow.compare.json";
pub(crate) const SHADOW_ERROR_FILE: &str = "authoring-ir-v2.shadow.error.json";

pub(crate) fn write_authoring_v2_shadow(
    job_dir: &Path,
    job: &ImportJob,
    v1_authoring: &Value,
    split: &Value,
    v1_document: Option<&Value>,
    physical_shadow: Option<&Value>,
    output_path: &Path,
) -> CommandResult<Value> {
    let value = build_authoring_v2_shadow(job, v1_authoring, split, v1_document, physical_shadow)?;
    write_canonical_json_atomic(output_path, &value)?;
    let compare = build_shadow_compare(job, v1_authoring, &value, physical_shadow);
    write_canonical_json_atomic(&job_dir.join(SHADOW_COMPARE_FILE), &compare)?;
    Ok(value)
}

pub(crate) fn build_authoring_v2_shadow(
    job: &ImportJob,
    v1_authoring: &Value,
    split: &Value,
    v1_document: Option<&Value>,
    physical_shadow: Option<&Value>,
) -> CommandResult<Value> {
    let source = job
        .source_files
        .iter()
        .find(|source| source.role.eq_ignore_ascii_case("mainquestion"))
        .or_else(|| job.source_files.first());
    let source_file_id = source
        .map(|file| file.file_id.as_str())
        .unwrap_or("unknown-source");
    let source_hash = source.map(|file| file.sha256.as_str()).unwrap_or("unknown");
    let source_type = source.map(|file| file.file_type.as_str()).unwrap_or("txt");
    let source_document_id = physical_shadow
        .and_then(|value| value.get("documentId"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            v1_document
                .and_then(|value| value.get("documentId"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| format!("document-{}", job.job_id));

    let physical_lines = physical_shadow
        .map(semantic_lines_from_v2_shadow)
        .unwrap_or_default();
    let v1_lines = semantic_lines_from_v1_document(v1_document);
    let fallback_lines = if !physical_lines.is_empty() {
        physical_lines.clone()
    } else {
        v1_lines.clone()
    };
    let assets = physical_shadow
        .and_then(|value| value.get("assets"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let passage = build_passage(
        job,
        v1_authoring,
        split,
        &v1_lines,
        &fallback_lines,
        &physical_lines,
        &assets,
        source_file_id,
        source_hash,
        source_type,
    );
    let mut task_groups = Vec::new();
    let mut answer_slots = Map::new();
    let answer_key_v1 = answer_key_from_v1(v1_authoring);
    for (index, candidate) in split
        .get("questionGroupCandidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let task_id = candidate
            .get("groupId")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("task-{}", index + 1));
        let group_lines = group_lines(
            candidate,
            &v1_lines,
            &fallback_lines,
            &physical_lines,
            job,
            source,
        );
        let heading = candidate
            .get("heading")
            .and_then(Value::as_str)
            .unwrap_or("Questions");
        let instruction_text = candidate
            .get("instructionText")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .unwrap_or(heading);
        let expression = parse_question_expression(heading)
            .or_else(|| parse_question_expression(instruction_text))
            .or_else(|| expression_from_candidate(candidate));
        let Some(expression) = expression else {
            task_groups.push(unresolved_task_group(
                &task_id,
                heading,
                source_anchor_from_job(
                    source_file_id,
                    source_hash,
                    source_type,
                    candidate_block_ids(candidate),
                    0,
                    None,
                ),
            ));
            continue;
        };
        let expected_numbers = expand_expression(&expression);
        let heading_index = group_lines
            .iter()
            .position(|line| {
                line.text
                    .to_ascii_lowercase()
                    .contains(&heading.to_ascii_lowercase())
            })
            .unwrap_or(0);
        let zone = collect_instruction_zone(&group_lines, heading_index, &expected_numbers);
        let zone_text = if zone.text.trim().is_empty() {
            normalize_instruction_text(instruction_text)
        } else {
            zone.text.clone()
        };
        let kind_hint = candidate.get("kindHint").and_then(Value::as_str);
        let signature_result = infer_instruction_signature(
            &zone_text,
            &expression,
            kind_hint,
            valid_anchors(&zone.source_anchors),
        );
        let task_type = signature_result.signature.task_type.clone();
        let task_type_name = task_type_label(&task_type);
        let question_anchors = detect_question_anchors(&group_lines, &expected_numbers);
        let v1_group = find_v1_group(v1_authoring, &task_id);
        let option_runs = detect_option_runs(&group_lines);
        let option_bank = detect_option_bank(
            &group_lines,
            &zone_text,
            signature_result.signature.allow_option_reuse,
            matches!(
                task_type,
                TaskTypeV2::MatchingInformation
                    | TaskTypeV2::MatchingHeadings
                    | TaskTypeV2::MatchingFeatures
                    | TaskTypeV2::MatchingSentenceEndings
                    | TaskTypeV2::Classification
            ),
        );
        let task_source_anchors = group_source_anchors(
            &zone.source_anchors,
            &question_anchors,
            &group_lines,
            &physical_lines,
            source_file_id,
            source_hash,
            source_type,
        );
        let option_bank_value = option_bank
            .as_ref()
            .map(|bank| option_bank_value(bank, &task_id))
            .or_else(|| {
                fixed_response_option_bank(
                    &task_id,
                    &task_type,
                    &signature_result.signature,
                    candidate,
                    v1_group,
                    &group_lines,
                    &task_source_anchors,
                )
            });
        let option_bank_ref = option_bank_value
            .as_ref()
            .and_then(|bank| bank.get("optionBankId"))
            .and_then(Value::as_str);
        let (responses, mut slots, used_answers) = build_responses_and_slots(
            &task_id,
            &task_type,
            &signature_result.signature,
            &expected_numbers,
            &question_anchors,
            &group_lines,
            candidate,
            v1_group,
            option_bank_ref,
            &option_runs,
            &answer_key_v1,
            &task_source_anchors,
        );
        let instructions = vec![paragraph_node(
            &format!("{task_id}-instructions"),
            &zone_text,
            task_source_anchors.clone(),
            Vec::new(),
        )];
        let stimulus = build_stimulus(
            &task_id,
            &task_type,
            &group_lines,
            &zone,
            &expected_numbers,
            &assets,
        );
        rehost_structured_slots(&task_id, &task_type, &mut slots);
        for (slot_id, slot) in slots {
            answer_slots.insert(slot_id, slot);
        }
        let mut group = json!({
            "taskId": task_id,
            "displayRange": expression,
            "taskType": task_type_name,
            "instructions": instructions,
            "instructionSignature": serde_json::to_value(&signature_result.signature).map_err(|error| error.to_string())?,
            "responseGroups": responses,
            "sourceAnchors": task_source_anchors,
            "quality": {"score": 0.0, "sourceCoverage": 0.0, "hardFailures": []},
            "reviewState": "unreviewed"
        });
        if let Some(stimulus) = stimulus {
            group["stimulus"] = Value::Array(stimulus);
        }
        if let Some(bank) = option_bank_value {
            group["optionBank"] = bank;
        }
        let _ = used_answers;
        task_groups.push(group);
    }

    let mut answer_key_v2 = answer_slots_answer_key(&answer_slots, &answer_key_v1);
    align_answer_key_assignments(&mut answer_key_v2, &task_groups);
    let mut authoring = json!({
        "schemaVersion": "IeltsAuthoringIRV2",
        "jobId": job.job_id,
        "exam": exam_value(job, source),
        "modality": "reading",
        "passage": passage,
        "taskGroups": task_groups,
        "answerSlots": answer_slots,
        "answerKey": answer_key_v2,
        "assets": assets,
        "sourceDocumentId": source_document_id,
        "quality": {},
        "audit": {
            "revision": 0,
            "source": "auto_extract",
            "humanVerified": false,
            "llmUsed": false,
            "updatedAt": job.updated_at.to_rfc3339(),
            "notes": ["Phase 4 grammar shadow; V1 remains authoritative."]
        }
    });
    let quality = quality::evaluate_quality(&authoring, physical_shadow);
    authoring["quality"] = quality.clone();
    refresh_group_quality(&mut authoring);
    serde_json::from_value::<IeltsAuthoringIRV2>(authoring.clone())
        .map_err(|error| format!("authoring_ir_v2_shadow_schema_validation_failed:{error}"))?;
    Ok(authoring)
}

fn build_passage(
    job: &ImportJob,
    v1_authoring: &Value,
    split: &Value,
    v1_lines: &[SemanticLine],
    fallback_lines: &[SemanticLine],
    physical_lines: &[SemanticLine],
    assets: &[Value],
    source_file_id: &str,
    source_hash: &str,
    source_type: &str,
) -> Value {
    let candidate = split
        .get("passageCandidates")
        .and_then(Value::as_array)
        .and_then(|items| items.first());
    let range = candidate
        .and_then(|value| value.get("range"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let range_ids = range.iter().cloned().collect::<BTreeSet<_>>();
    let passage_pages = v1_lines
        .iter()
        .filter(|line| range_ids.contains(&line.id))
        .map(|line| line.page_index)
        .collect::<BTreeSet<_>>();
    let question_pages = split
        .get("questionGroupCandidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|candidate| {
            candidate
                .get("sectionEvidence")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|evidence| evidence.get("pageIndex").and_then(Value::as_i64))
                .map(|page| if page > 0 { page - 1 } else { page } as i64)
                .filter_map(|page| i32::try_from(page).ok())
        })
        .collect::<BTreeSet<_>>();
    let mut passage_lines = v1_lines
        .iter()
        .filter(|line| {
            range_ids.contains(&line.id)
                || (passage_pages.contains(&line.page_index)
                    && !question_pages.contains(&line.page_index))
        })
        .filter_map(|line| {
            let mut line = line.clone();
            if !range_ids.contains(&line.id) {
                line.text = trim_passage_preamble(&line.text);
            }
            (!line.text.trim().is_empty()).then_some(line)
        })
        .collect::<Vec<_>>();
    passage_lines.retain(|line| {
        !is_paper_section_header(&normalize_instruction_text(&line.text).to_ascii_lowercase())
    });
    if passage_lines.is_empty() {
        passage_lines = v1_authoring
            .pointer("/passage/htmlBlocks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|block| {
                let text = block
                    .get("html")
                    .and_then(Value::as_str)
                    .map(strip_html)
                    .filter(|text| !text.is_empty())?;
                let id = block
                    .get("blockId")
                    .and_then(Value::as_str)
                    .unwrap_or("passage-block")
                    .to_string();
                Some(SemanticLine {
                    id: id.clone(),
                    text,
                    source_anchor: source_anchor_from_job(
                        source_file_id,
                        source_hash,
                        source_type,
                        vec![id],
                        0,
                        None,
                    ),
                    page_index: 0,
                    order: 0,
                    role: "passage".to_string(),
                    bbox: None,
                })
            })
            .collect();
    }
    if passage_lines.is_empty() {
        passage_lines = visual_passage_lines(fallback_lines);
    }
    rebind_lines_to_physical(&mut passage_lines, physical_lines);
    let title = candidate
        .and_then(|value| value.get("title"))
        .and_then(Value::as_str)
        .unwrap_or(&job.title);
    let anchors = passage_lines
        .iter()
        .map(|line| line.source_anchor.clone())
        .collect::<Vec<_>>();
    let mut anchors = anchors;
    let passage_pages = passage_lines
        .iter()
        .map(|line| line.page_index)
        .collect::<std::collections::BTreeSet<_>>();
    anchors.extend(physical_span_anchors(&anchors, physical_lines));
    anchors.extend(passage_preamble_anchors(physical_lines, &passage_pages));
    let anchors = valid_anchors(&anchors);
    let mut content = passage_nodes(title, &passage_lines, anchors.clone());
    for asset in assets.iter().filter(|asset| {
        asset.get("kind").and_then(Value::as_str) == Some("raster_image")
            && asset.get("extractionMode").and_then(Value::as_str) == Some("embedded")
            && asset
                .pointer("/sourceAnchor/pageIndex")
                .and_then(Value::as_u64)
                .is_some_and(|page| {
                    i32::try_from(page)
                        .ok()
                        .is_some_and(|page| passage_pages.contains(&page))
                })
    }) {
        let Some(asset_id) = asset.get("assetId").and_then(Value::as_str) else {
            continue;
        };
        let Some(source_anchor) = asset.get("sourceAnchor").cloned() else {
            continue;
        };
        content.push(json!({
            "type": "image",
            "id": format!("passage-image-{asset_id}"),
            "sourceAnchors": [source_anchor],
            "provenanceStatus": "source",
            "assetId": asset_id,
            "altText": asset.get("altText").and_then(Value::as_str).unwrap_or("Passage image"),
            "display": {}
        }));
    }
    json!({
        "title": title,
        "content": content,
        "paragraphMap": {},
        "sourceAnchors": anchors
    })
}

fn trim_passage_preamble(text: &str) -> String {
    let normalized = normalize_instruction_text(text);
    let lower = normalized.to_ascii_lowercase();
    if !lower.starts_with("you should spend about") {
        return normalized;
    }
    lower
        .find("below")
        .and_then(|index| normalized.get(index + "below".len()..))
        .map(str::trim)
        .filter(|suffix| !suffix.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn group_lines(
    candidate: &Value,
    v1_lines: &[SemanticLine],
    fallback_lines: &[SemanticLine],
    physical_lines: &[SemanticLine],
    job: &ImportJob,
    source: Option<&SourceFile>,
) -> Vec<SemanticLine> {
    let block_ids = candidate_block_ids(candidate);
    let mut lines = candidate
        .get("sectionEvidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|evidence| {
            let id = evidence
                .get("blockId")
                .and_then(Value::as_str)
                .unwrap_or("evidence")
                .to_string();
            let text = evidence
                .get("textPreview")
                .and_then(Value::as_str)
                .map(normalize_instruction_text)
                .filter(|text| !text.is_empty())?;
            let page_index = evidence
                .get("pageIndex")
                .and_then(Value::as_i64)
                .map(|value| if value > 0 { value - 1 } else { value })
                .unwrap_or(0) as i32;
            let bbox = evidence
                .get("bbox")
                .and_then(Value::as_array)
                .filter(|items| items.len() == 4)
                .map(|items| {
                    [
                        items[0].as_f64().unwrap_or(0.0),
                        items[1].as_f64().unwrap_or(0.0),
                        items[2].as_f64().unwrap_or(0.0),
                        items[3].as_f64().unwrap_or(0.0),
                    ]
                });
            Some(SemanticLine {
                id: id.clone(),
                text,
                source_anchor: source_anchor_from_job(
                    source
                        .map(|file| file.file_id.as_str())
                        .unwrap_or("unknown-source"),
                    source.map(|file| file.sha256.as_str()).unwrap_or("unknown"),
                    source.map(|file| file.file_type.as_str()).unwrap_or("txt"),
                    vec![id],
                    page_index,
                    bbox,
                ),
                page_index,
                order: 0,
                role: "question".to_string(),
                bbox,
            })
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines = v1_lines
            .iter()
            .filter(|line| block_ids.contains(&line.id))
            .cloned()
            .collect();
    }
    if lines.is_empty() {
        lines = fallback_lines.to_vec();
    }
    rebind_lines_to_physical(&mut lines, physical_lines);
    for (index, line) in lines.iter_mut().enumerate() {
        line.order = index;
    }
    let _ = job;
    lines
}

fn rebind_lines_to_physical(lines: &mut [SemanticLine], physical_lines: &[SemanticLine]) {
    if physical_lines.is_empty() {
        return;
    }
    for line in lines {
        let key = physical_text_key(&line.text);
        if key.is_empty() {
            continue;
        }
        let page_lines = physical_lines
            .iter()
            .filter(|candidate| candidate.page_index == line.page_index)
            .filter(|candidate| !physical_text_key(&candidate.text).is_empty())
            .collect::<Vec<_>>();
        let mut matches = Vec::<Vec<&SemanticLine>>::new();
        for start in 0..page_lines.len() {
            let mut candidate_key = String::new();
            for end in start..page_lines.len() {
                candidate_key.push_str(&physical_text_key(&page_lines[end].text));
                if candidate_key == key {
                    matches.push(page_lines[start..=end].to_vec());
                    break;
                }
                if candidate_key.len() >= key.len() {
                    break;
                }
            }
        }
        let matched: Vec<&SemanticLine> = if let [matched] = matches.as_slice() {
            matched.clone()
        } else if let Some(bbox) = line.bbox {
            let overlapping = page_lines
                .iter()
                .filter(|candidate| {
                    candidate.bbox.is_some_and(|candidate_bbox| {
                        bbox_intersects(bbox, candidate_bbox)
                            || bbox_contains_center(bbox, candidate_bbox)
                    })
                })
                .copied()
                .collect::<Vec<_>>();
            if overlapping.is_empty() {
                continue;
            }
            // A section-evidence bbox is authoritative layout evidence when its
            // extracted preview is truncated or split differently than PDF lines.
            overlapping
        } else {
            continue;
        };
        line.source_anchor = merged_physical_anchor(&matched);
        line.bbox = union_line_bbox(&matched);
    }
}

fn bbox_intersects(left: [f64; 4], right: [f64; 4]) -> bool {
    let left_right = left[0] + left[2];
    let right_right = right[0] + right[2];
    let left_bottom = left[1] + left[3];
    let right_bottom = right[1] + right[3];
    left[0] < right_right
        && right[0] < left_right
        && left[1] < right_bottom
        && right[1] < left_bottom
}

fn bbox_contains_center(container: [f64; 4], candidate: [f64; 4]) -> bool {
    let center_x = candidate[0] + candidate[2] / 2.0;
    let center_y = candidate[1] + candidate[3] / 2.0;
    center_x >= container[0]
        && center_x <= container[0] + container[2]
        && center_y >= container[1]
        && center_y <= container[1] + container[3]
}

fn merged_physical_anchor(lines: &[&SemanticLine]) -> Value {
    let mut anchor = lines[0].source_anchor.clone();
    let mut seen = BTreeSet::new();
    let node_ids = lines
        .iter()
        .flat_map(|line| {
            line.source_anchor
                .get("nodeIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str)
        .filter(|node_id| !node_id.trim().is_empty())
        .filter(|node_id| seen.insert((*node_id).to_string()))
        .map(|node_id| Value::String(node_id.to_string()))
        .collect::<Vec<_>>();
    anchor["nodeIds"] = Value::Array(node_ids);
    if let Some([x, y, width, height]) = union_line_bbox(lines) {
        anchor["bbox"] = json!({
            "x": x,
            "y": y,
            "width": width,
            "height": height,
            "unit": "pt",
            "origin": "top-left",
            "pageRotation": 0
        });
    }
    if lines.len() > 1 {
        if let Some(object) = anchor.as_object_mut() {
            for key in ["nativeBBox", "displayBBox", "pdfToDisplay", "charRange"] {
                object.remove(key);
            }
        }
    }
    anchor
}

fn union_line_bbox(lines: &[&SemanticLine]) -> Option<[f64; 4]> {
    let boxes = lines
        .iter()
        .filter_map(|line| line.bbox)
        .collect::<Vec<_>>();
    let first = boxes.first()?;
    let min_x = boxes.iter().map(|bbox| bbox[0]).fold(first[0], f64::min);
    let min_y = boxes.iter().map(|bbox| bbox[1]).fold(first[1], f64::min);
    let max_x = boxes
        .iter()
        .map(|bbox| bbox[0] + bbox[2])
        .fold(first[0] + first[2], f64::max);
    let max_y = boxes
        .iter()
        .map(|bbox| bbox[1] + bbox[3])
        .fold(first[1] + first[3], f64::max);
    Some([min_x, min_y, max_x - min_x, max_y - min_y])
}

fn physical_text_key(text: &str) -> String {
    normalize_instruction_text(text)
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn passage_preamble_anchors(
    physical_lines: &[SemanticLine],
    passage_pages: &BTreeSet<i32>,
) -> Vec<Value> {
    let mut anchors = Vec::new();
    for (start, line) in physical_lines.iter().enumerate() {
        if !passage_pages.contains(&line.page_index) {
            continue;
        }
        let key = physical_text_key(&line.text);
        if !key.starts_with("youshouldspendabout") || !key.contains("questions") {
            continue;
        }
        let mut candidate = Vec::new();
        // The "READING PASSAGE N" banner sits on the physical line directly
        // above the instruction line on the same page; anchor it to the passage
        // so the title region is not flagged as significant-but-unassigned.
        if let Some(previous) = start.checked_sub(1).and_then(|index| physical_lines.get(index)) {
            if previous.page_index == line.page_index {
                let previous_key = physical_text_key(&previous.text);
                if previous_key.starts_with("readingpassage")
                    || previous_key.starts_with("readingsection")
                    || previous_key.contains("passage")
                {
                    candidate.push(previous.source_anchor.clone());
                }
            }
        }
        for continuation in physical_lines.iter().skip(start).take(6) {
            if continuation.page_index != line.page_index {
                break;
            }
            candidate.push(continuation.source_anchor.clone());
            if physical_text_key(&continuation.text).ends_with("below") {
                break;
            }
        }
        // The "below" terminator can be wrapped onto a later physical line; when
        // the continuation does not end there, extend through the previous
        // candidate so the whole preamble (including the passage title banner)
        // is anchored to the passage and not flagged as unassigned.
        if !candidate.is_empty() {
            anchors.extend(candidate);
        }
    }
    valid_anchors(&anchors)
}

fn build_responses_and_slots(
    task_id: &str,
    task_type: &TaskTypeV2,
    signature: &crate::schema::ielts_authoring_v2::InstructionSignatureV2,
    expected_numbers: &[u32],
    question_anchors: &[QuestionAnchor],
    lines: &[SemanticLine],
    candidate: &Value,
    v1_group: Option<&Value>,
    option_bank_ref: Option<&str>,
    option_runs: &[OptionRun],
    answer_key: &Map<String, Value>,
    task_anchors: &[Value],
) -> (Vec<Value>, Map<String, Value>, Map<String, Value>) {
    let mut responses = Vec::new();
    let mut slots = Map::new();
    let mut used_answers = Map::new();
    let shared = signature
        .selection_cardinality
        .as_ref()
        .and_then(|cardinality| cardinality.exact)
        .is_some_and(|count| count as usize == expected_numbers.len() && count > 1)
        && matches!(
            task_type,
            TaskTypeV2::MultipleChoice | TaskTypeV2::ShortAnswer
        );
    let shared_option_values = if let Some(run) = option_runs
        .iter()
        .find(|run| run_matches_alphabet(run, signature.option_alphabet.as_deref()))
    {
        option_run_value(run, &format!("{task_id}-option"))
    } else {
        fixed_options_from_v1(task_id, candidate, v1_group, task_anchors)
    };
    let option_bank_ref = option_bank_ref.map(ToString::to_string);
    if shared {
        let group_id = format!("{task_id}-responses");
        let prompt_text = shared_prompt(v1_group, lines, expected_numbers.first().copied());
        let prompt_anchor = task_anchors.first().cloned();
        let prompt = vec![paragraph_node(
            &format!("{task_id}-shared-prompt"),
            &prompt_text,
            task_anchors.to_vec(),
            Vec::new(),
        )];
        let mut slot_ids = Vec::new();
        for number in expected_numbers {
            let slot_id = format!("q{number}");
            slot_ids.push(slot_id.clone());
            let anchor = question_anchor(question_anchors, *number)
                .map(|anchor| anchor.source_anchor.clone())
                .or_else(|| prompt_anchor.clone())
                .unwrap_or_else(|| task_anchors.first().cloned().unwrap_or_else(empty_anchor));
            let slot = slot_value(
                &slot_id,
                *number,
                task_type,
                &format!("{task_id}-shared-prompt"),
                anchor,
                signature,
            );
            if let Some(value) = answer_value_for_slot(answer_key, &slot_id, *number).as_object() {
                used_answers.insert(slot_id.clone(), Value::Object(value.clone()));
            }
            slots.insert(slot_id, slot);
        }
        responses.push(response_value(
            &group_id,
            "choice",
            prompt,
            slot_ids,
            if option_bank_ref.is_some() {
                None
            } else {
                Some(shared_option_values)
            },
            option_bank_ref,
            signature,
            task_type,
            task_anchors,
        ));
        return (responses, slots, used_answers);
    }

    let aggregate_per_slot = matches!(
        task_type,
        TaskTypeV2::TrueFalseNotGiven
            | TaskTypeV2::YesNoNotGiven
            | TaskTypeV2::MatchingInformation
            | TaskTypeV2::MatchingHeadings
            | TaskTypeV2::MatchingFeatures
            | TaskTypeV2::MatchingSentenceEndings
            | TaskTypeV2::Classification
    ) || is_completion_task(task_type);
    let mut aggregate_prompt = Vec::new();
    let mut aggregate_slot_ids = Vec::new();
    let mut aggregate_options = None;
    for (index, number) in expected_numbers.iter().enumerate() {
        let slot_id = format!("q{number}");
        let anchor = question_anchor(question_anchors, *number)
            .map(|anchor| anchor.source_anchor.clone())
            .or_else(|| task_anchors.first().cloned())
            .unwrap_or_else(empty_anchor);
        let candidate_question_value = candidate_question(candidate, *number);
        let v1_question = v1_group.and_then(|group| find_v1_question(group, *number));
        let candidate_prompt = candidate_question_value
            .and_then(|question| question.get("prompt"))
            .and_then(Value::as_str)
            .or_else(|| {
                v1_question
                    .and_then(|question| question.get("prompt"))
                    .and_then(Value::as_str)
            });
        let current_anchor = question_anchor(question_anchors, *number);
        let next_anchor = expected_numbers
            .get(index + 1)
            .and_then(|next| question_anchor(question_anchors, *next));
        let option_values = option_run_for_question(
            option_runs,
            lines,
            current_anchor,
            next_anchor,
            index,
            signature.option_alphabet.as_deref(),
        )
        .map(|run| option_run_value(run, &format!("{task_id}-option-{number}")))
        .unwrap_or_else(|| {
            fixed_options_from_v1(
                &format!("{task_id}-{number}"),
                candidate,
                v1_group,
                task_anchors,
            )
        });
        let prompt_result = assemble_prompt(
            candidate_prompt,
            *number,
            current_anchor,
            next_anchor,
            lines,
            Some(anchor.clone()),
        );
        let completion_fallback = if prompt_result.text.is_empty() && is_completion_task(task_type)
        {
            completion_prompt_fallback(lines)
        } else {
            None
        };
        let prompt_text = completion_fallback
            .as_ref()
            .map(|(text, _)| text.clone())
            .unwrap_or_else(|| prompt_result.text.clone());
        let prompt_anchors = completion_fallback
            .as_ref()
            .map(|(_, anchors)| anchors.clone())
            .unwrap_or_else(|| prompt_result.source_anchors.clone());
        let host_id = format!("{task_id}-prompt-{number}");
        let mut children = Vec::new();
        if !prompt_text.is_empty() {
            children.push(text_node(
                &format!("{host_id}-text"),
                &prompt_text,
                prompt_anchors
                    .first()
                    .cloned()
                    .or_else(|| Some(anchor.clone())),
            ));
        }
        if is_completion_task(task_type) {
            children.push(answer_slot_node(
                &slot_id,
                &number.to_string(),
                anchor.clone(),
                completion_placeholder(task_type),
            ));
        }
        let prompt = vec![paragraph_node(
            &host_id,
            if prompt_text.is_empty() {
                "[prompt pending review]"
            } else {
                &prompt_text
            },
            prompt_anchors,
            children,
        )];
        let slot = slot_value(&slot_id, *number, task_type, &host_id, anchor, signature);
        let answer = answer_value_for_slot(answer_key, &slot_id, *number);
        used_answers.insert(slot_id.clone(), answer);
        slots.insert(slot_id.clone(), slot);
        let group_kind = response_kind(task_type);
        if aggregate_per_slot {
            aggregate_prompt.extend(prompt);
            aggregate_slot_ids.push(slot_id);
            if !is_completion_task(task_type)
                && option_bank_ref.is_none()
                && aggregate_options.is_none()
            {
                aggregate_options = Some(option_values);
            }
        } else {
            responses.push(response_value(
                &format!("{task_id}-response-{}", number),
                group_kind,
                prompt,
                vec![slot_id],
                if option_bank_ref.is_some() {
                    None
                } else {
                    Some(option_values)
                },
                option_bank_ref.clone(),
                signature,
                task_type,
                task_anchors,
            ));
        }
    }
    if aggregate_per_slot && !aggregate_slot_ids.is_empty() {
        responses.push(response_value(
            &format!("{task_id}-responses"),
            response_kind(task_type),
            aggregate_prompt,
            aggregate_slot_ids,
            if option_bank_ref.is_some() {
                None
            } else {
                aggregate_options
            },
            option_bank_ref,
            signature,
            task_type,
            task_anchors,
        ));
    }
    (responses, slots, used_answers)
}

fn response_value(
    response_id: &str,
    kind: &str,
    prompt: Vec<Value>,
    slot_ids: Vec<String>,
    options: Option<Vec<Value>>,
    option_bank_ref: Option<String>,
    signature: &crate::schema::ielts_authoring_v2::InstructionSignatureV2,
    task_type: &TaskTypeV2,
    anchors: &[Value],
) -> Value {
    let exact = signature
        .selection_cardinality
        .as_ref()
        .and_then(|cardinality| cardinality.exact)
        .or_else(|| {
            (matches!(
                task_type,
                TaskTypeV2::SingleChoice
                    | TaskTypeV2::TrueFalseNotGiven
                    | TaskTypeV2::YesNoNotGiven
            ))
            .then_some(1)
        })
        .or(Some(1));
    let assignment = signature
        .answer_assignment
        .as_ref()
        .map(assignment_label)
        .unwrap_or("per_slot");
    let mut response = json!({
        "responseGroupId": response_id,
        "kind": kind,
        "prompt": prompt,
        "slotIds": slot_ids,
        "cardinality": {"min": exact.unwrap_or(1), "max": exact.unwrap_or(1), "exact": exact},
        "assignment": assignment,
        "scoringPolicy": if assignment == "unordered_set" || is_completion_task(task_type) { "per_slot_ielts_normalized" } else { "per_slot_binary" },
        "duplicatePolicy": "reject_submission",
        "allowOptionReuse": signature.allow_option_reuse.unwrap_or(false),
        "sourceAnchors": anchors
    });
    let object = response
        .as_object_mut()
        .expect("response group serialization must produce an object");
    if let Some(options) = options {
        object.insert("options".to_string(), Value::Array(options));
    }
    if let Some(option_bank_ref) = option_bank_ref {
        object.insert("optionBankRef".to_string(), Value::String(option_bank_ref));
    }
    response
}

fn slot_value(
    slot_id: &str,
    number: u32,
    task_type: &TaskTypeV2,
    host_node_id: &str,
    source_anchor: Value,
    signature: &crate::schema::ielts_authoring_v2::InstructionSignatureV2,
) -> Value {
    let interaction = match task_type {
        TaskTypeV2::SingleChoice | TaskTypeV2::TrueFalseNotGiven | TaskTypeV2::YesNoNotGiven => {
            "radio"
        }
        TaskTypeV2::MultipleChoice => "checkbox",
        TaskTypeV2::MatchingHeadings
        | TaskTypeV2::MatchingInformation
        | TaskTypeV2::MatchingFeatures
        | TaskTypeV2::MatchingSentenceEndings
        | TaskTypeV2::Classification => "select",
        TaskTypeV2::DiagramLabelCompletion | TaskTypeV2::PlanMapLabelCompletion => "hotspot",
        _ if is_completion_task(task_type) || matches!(task_type, TaskTypeV2::ShortAnswer) => {
            "text"
        }
        _ => "text",
    };
    let host_type = completion_host_type(task_type);
    let mut slot = json!({
        "slotId": slot_id,
        "questionNumber": number,
        "displayLabel": number.to_string(),
        "hostNodeId": host_node_id,
        "hostType": host_type,
        "interaction": interaction,
        "participation": "scoring",
        "sourceAnchors": [source_anchor],
        "confidence": signature.confidence
    });
    if let Some(word_limit) = &signature.word_limit {
        let mut constraints = json!({});
        if let Some(max_words) = word_limit.max_words {
            constraints["maxWords"] = json!(max_words);
        }
        if let Some(max_numbers) = word_limit.max_numbers {
            constraints["maxNumbers"] = json!(max_numbers);
        }
        slot["constraints"] = constraints;
    }
    slot
}

fn fixed_options_from_v1(
    id_prefix: &str,
    candidate: &Value,
    v1_group: Option<&Value>,
    anchors: &[Value],
) -> Vec<Value> {
    let mut labels = candidate
        .pointer("/classification/interaction/options")
        .and_then(Value::as_array)
        .or_else(|| {
            v1_group.and_then(|group| {
                group
                    .get("questions")
                    .and_then(Value::as_array)
                    .and_then(|questions| questions.first())
                    .and_then(|question| question.pointer("/interaction/options"))
                    .and_then(Value::as_array)
            })
        })
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if labels.is_empty() {
        labels = Vec::new();
    }
    labels
        .into_iter()
        .enumerate()
        .map(|(index, label)| {
            let anchor = anchors.first().cloned().unwrap_or_else(empty_anchor);
            json!({
                "optionId": format!("{id_prefix}-fixed-option-{}", index + 1),
                "label": label,
                "content": [text_node(
                    &format!("{id_prefix}-fixed-option-text-{}", index + 1),
                    &label,
                    Some(anchor.clone()),
                )],
                "sourceAnchors": [anchor]
            })
        })
        .collect()
}

fn fixed_response_option_bank(
    task_id: &str,
    task_type: &TaskTypeV2,
    signature: &crate::schema::ielts_authoring_v2::InstructionSignatureV2,
    candidate: &Value,
    v1_group: Option<&Value>,
    lines: &[SemanticLine],
    anchors: &[Value],
) -> Option<Value> {
    let canonical_labels = match task_type {
        TaskTypeV2::TrueFalseNotGiven => Some(["TRUE", "FALSE", "NOT GIVEN"]),
        TaskTypeV2::YesNoNotGiven => Some(["YES", "NO", "NOT GIVEN"]),
        _ => None,
    };
    let is_matching = matches!(
        task_type,
        TaskTypeV2::MatchingInformation
            | TaskTypeV2::MatchingHeadings
            | TaskTypeV2::MatchingFeatures
            | TaskTypeV2::MatchingSentenceEndings
            | TaskTypeV2::Classification
    );
    let is_shared_unordered = matches!(task_type, TaskTypeV2::MultipleChoice)
        && signature
            .answer_assignment
            .as_ref()
            .is_some_and(|assignment| assignment_label(assignment) == "unordered_set");
    if canonical_labels.is_none() && !is_matching && !is_shared_unordered {
        return None;
    }
    let mut options = fixed_options_from_v1(task_id, candidate, v1_group, anchors);
    if options.is_empty() {
        let canonical_labels = canonical_labels?;
        let anchor = anchors.first().cloned().unwrap_or_else(empty_anchor);
        options = canonical_labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                json!({
                    "optionId": format!("{task_id}-fixed-option-{}", index + 1),
                    "label": label,
                    "content": [text_node(
                        &format!("{task_id}-fixed-option-text-{}", index + 1),
                        label,
                        Some(anchor.clone()),
                    )],
                    "sourceAnchors": [anchor.clone()]
                })
            })
            .collect();
    }
    enrich_fixed_options_from_source_lines(task_id, &mut options, lines);
    let title = lines
        .iter()
        .map(|line| {
            (
                normalize_instruction_text(&line.text),
                line.source_anchor.clone(),
            )
        })
        .find(|(text, _)| {
            let lower = text.to_ascii_lowercase();
            lower == "list of people"
                || lower == "list of headings"
                || lower == "list of features"
                || lower == "list of categories"
                || lower == "list of options"
        })
        .map(|(text, anchor)| {
            vec![text_node(
                &format!("{task_id}-option-bank-title"),
                &text,
                Some(anchor),
            )]
        });
    let allow_reuse = if canonical_labels.is_some() {
        true
    } else {
        signature.allow_option_reuse.unwrap_or(false)
    };
    let mut bank = json!({
        "optionBankId": format!("{task_id}-option-bank"),
        "scope": "task_group",
        "options": options,
        "allowReuse": allow_reuse,
        "sourceAnchors": anchors
    });
    if let Some(title) = title {
        bank["title"] = json!(title);
    }
    Some(bank)
}

fn enrich_fixed_options_from_source_lines(
    task_id: &str,
    options: &mut [Value],
    lines: &[SemanticLine],
) {
    let labels = options
        .iter()
        .filter_map(|option| option.get("label").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if labels.len() < 2 {
        return;
    }
    let Some((texts, anchor)) = lines.iter().find_map(|line| {
        split_inline_option_texts(&line.text, &labels)
            .map(|texts| (texts, line.source_anchor.clone()))
    }) else {
        return;
    };
    for (index, (option, text)) in options.iter_mut().zip(texts).enumerate() {
        option["content"] = json!([text_node(
            &format!("{task_id}-source-option-text-{}", index + 1),
            &text,
            Some(anchor.clone()),
        )]);
        option["sourceAnchors"] = json!([anchor.clone()]);
    }
}

fn split_inline_option_texts(text: &str, labels: &[String]) -> Option<Vec<String>> {
    let normalized = normalize_instruction_text(text);
    let mut positions = Vec::with_capacity(labels.len());
    let mut search_from = 0;
    for (index, label) in labels.iter().enumerate() {
        let position = normalized[search_from..]
            .match_indices(label)
            .map(|(offset, _)| search_from + offset)
            .find(|position| {
                let before_ok = *position == 0
                    || normalized[..*position]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace);
                let after = *position + label.len();
                let after_ok = normalized[after..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_whitespace() || matches!(ch, '.' | ')' | ':'));
                before_ok && after_ok && (index > 0 || *position == 0)
            })?;
        positions.push(position);
        search_from = position + label.len();
    }
    let mut values = Vec::with_capacity(labels.len());
    for (index, position) in positions.iter().enumerate() {
        let start = position + labels[index].len();
        let end = positions
            .get(index + 1)
            .copied()
            .unwrap_or(normalized.len());
        let value = normalized[start..end]
            .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, '.' | ')' | ':'))
            .trim()
            .to_string();
        if value.is_empty() {
            return None;
        }
        values.push(value);
    }
    Some(values)
}

fn shared_prompt(
    v1_group: Option<&Value>,
    lines: &[SemanticLine],
    first_number: Option<u32>,
) -> String {
    let v1_prompt = v1_group
        .and_then(|group| group.get("questions"))
        .and_then(Value::as_array)
        .and_then(|questions| questions.first())
        .and_then(|question| question.get("prompt"))
        .and_then(Value::as_str)
        .map(normalize_instruction_text)
        .filter(|text| !text.is_empty());
    v1_prompt
        .or_else(|| {
            first_number.and_then(|number| {
                lines
                    .iter()
                    .find(|line| line.text.trim_start().starts_with(&number.to_string()))
                    .map(|line| normalize_instruction_text(&line.text))
            })
        })
        .or_else(|| shared_prompt_from_lines(lines))
        .unwrap_or_else(|| "[shared prompt pending review]".to_string())
}

fn shared_prompt_from_lines(lines: &[SemanticLine]) -> Option<String> {
    let option_start = detect_option_runs(lines)
        .first()
        .and_then(|run| run.options.first())
        .and_then(|option| lines.iter().position(|line| line.id == option.line_id))
        .unwrap_or(lines.len());
    let prompt = lines
        .iter()
        .take(option_start)
        .map(|line| normalize_instruction_text(&line.text))
        .filter(|text| !text.is_empty())
        .filter(|text| !is_shared_prompt_instruction(text))
        .collect::<Vec<_>>();
    (!prompt.is_empty()).then(|| prompt.join(" "))
}

fn is_shared_prompt_instruction(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.starts_with("questions ")
        || lower.starts_with("question ")
        || lower.starts_with("choose ")
        || lower.starts_with("complete ")
        || lower.starts_with("look at ")
        || lower.starts_with("match each ")
        || lower.starts_with("write ")
        || lower.starts_with("in boxes ")
        || lower.starts_with("nb ")
}

fn completion_prompt_fallback(lines: &[SemanticLine]) -> Option<(String, Vec<Value>)> {
    let mut text = Vec::new();
    let mut anchors = Vec::new();
    for line in lines {
        let normalized = normalize_instruction_text(&line.text);
        if normalized.is_empty() || !is_completion_prompt_instruction(&normalized) {
            continue;
        }
        text.push(normalized);
        anchors.push(line.source_anchor.clone());
    }
    (!text.is_empty()).then(|| (text.join(" "), anchors))
}

fn is_completion_prompt_instruction(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.starts_with("complete ")
        || lower.starts_with("choose ")
        || lower.starts_with("write ")
        || lower.starts_with("use no more than ")
        || lower.starts_with("fill ")
}

fn build_stimulus(
    task_id: &str,
    task_type: &TaskTypeV2,
    lines: &[SemanticLine],
    zone: &instruction_zone::InstructionZone,
    expected_numbers: &[u32],
    assets: &[Value],
) -> Option<Vec<Value>> {
    if !is_completion_task(task_type) {
        return None;
    }
    let expected_text = expected_numbers
        .iter()
        .map(|number| number.to_string())
        .collect::<BTreeSet<_>>();
    let mut nodes = lines
        .iter()
        .filter(|line| !zone.line_ids.contains(&line.id))
        .filter(|line| {
            !expected_text
                .iter()
                .any(|number| line.text.trim_start().starts_with(&format!("{number}.")))
        })
        .filter(|line| !line.text.trim().is_empty())
        .take(8)
        .map(|line| {
            paragraph_node(
                &format!("stimulus-{}", line.id),
                &line.text,
                vec![line.source_anchor.clone()],
                Vec::new(),
            )
        })
        .collect::<Vec<_>>();
    if matches!(task_type, TaskTypeV2::TableCompletion) {
        return Some(vec![table_completion_node(
            task_id,
            lines,
            expected_numbers,
        )]);
    }
    if matches!(task_type, TaskTypeV2::FlowchartCompletion) {
        return Some(vec![flowchart_completion_node(
            task_id,
            lines,
            expected_numbers,
        )]);
    }
    let diagram_asset_id = assets.iter().find_map(|asset| {
        asset
            .get("assetId")
            .and_then(Value::as_str)
            .filter(|asset_id| !asset_id.trim().is_empty())
    });
    if let Some(diagram) = diagram_candidate(
        task_type,
        lines,
        task_id,
        diagram_asset_id,
        expected_numbers,
    ) {
        nodes.insert(0, diagram);
    }
    (!nodes.is_empty()).then_some(nodes)
}

fn option_run_for_question<'a>(
    option_runs: &'a [OptionRun],
    lines: &[SemanticLine],
    current_anchor: Option<&QuestionAnchor>,
    next_anchor: Option<&QuestionAnchor>,
    fallback_index: usize,
    option_alphabet: Option<&str>,
) -> Option<&'a OptionRun> {
    let matching = option_runs
        .iter()
        .filter(|run| run_matches_alphabet(run, option_alphabet))
        .collect::<Vec<_>>();
    if let Some(current) = current_anchor {
        let end = next_anchor
            .map(|anchor| anchor.line_index)
            .unwrap_or(usize::MAX);
        if let Some(run) = matching.iter().copied().find(|run| {
            run.options
                .first()
                .and_then(|option| lines.iter().position(|line| line.id == option.line_id))
                .is_some_and(|line_index| line_index > current.line_index && line_index < end)
        }) {
            return Some(run);
        }
    }
    matching
        .get(fallback_index)
        .copied()
        .or_else(|| matching.first().copied())
}

fn rehost_structured_slots(task_id: &str, task_type: &TaskTypeV2, slots: &mut Map<String, Value>) {
    for slot in slots.values_mut() {
        let Some(slot_id) = slot
            .get("slotId")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        match task_type {
            TaskTypeV2::TableCompletion => {
                slot["hostNodeId"] = json!(format!("{task_id}-table-cell-{slot_id}"));
                slot["hostType"] = json!("table_cell");
            }
            TaskTypeV2::FlowchartCompletion => {
                slot["hostNodeId"] = json!(format!("{task_id}-flow-step-{slot_id}"));
                slot["hostType"] = json!("flow_step");
            }
            TaskTypeV2::DiagramLabelCompletion | TaskTypeV2::PlanMapLabelCompletion => {
                slot["hostNodeId"] = json!(format!("{task_id}-hotspot-{slot_id}"));
                slot["hostType"] = json!("figure_hotspot");
            }
            _ => {}
        }
    }
}

fn table_completion_node(task_id: &str, lines: &[SemanticLine], numbers: &[u32]) -> Value {
    let rows = numbers
        .iter()
        .map(|number| {
            let slot_id = format!("q{number}");
            let anchor = source_anchor_for_number(lines, *number);
            let cell_id = format!("{task_id}-table-cell-{slot_id}");
            json!({
                "type": "table_row",
                "id": format!("{task_id}-table-row-{slot_id}"),
                "sourceAnchors": [anchor.clone()],
                "provenanceStatus": "derived",
                "cells": [{
                    "type": "table_cell",
                    "id": cell_id,
                    "sourceAnchors": [anchor.clone()],
                    "provenanceStatus": "derived",
                    "rowSpan": 1,
                    "colSpan": 1,
                    "headerScope": "none",
                    "children": [paragraph_node(
                        &format!("{task_id}-table-prompt-{slot_id}"),
                        &prompt_for_number(lines, *number),
                        vec![anchor.clone()],
                        vec![answer_slot_node(
                            &slot_id,
                            &number.to_string(),
                            anchor,
                            completion_placeholder(&TaskTypeV2::TableCompletion),
                        )],
                    )]
                }]
            })
        })
        .collect::<Vec<_>>();
    json!({
        "type": "table",
        "id": format!("{task_id}-table"),
        "sourceAnchors": lines.iter().map(|line| line.source_anchor.clone()).collect::<Vec<_>>(),
        "provenanceStatus": "derived",
        "rows": rows
    })
}

fn flowchart_completion_node(task_id: &str, lines: &[SemanticLine], numbers: &[u32]) -> Value {
    let steps = numbers
        .iter()
        .map(|number| {
            let slot_id = format!("q{number}");
            let anchor = source_anchor_for_number(lines, *number);
            json!({
                "type": "flow_step",
                "id": format!("{task_id}-flow-step-{slot_id}"),
                "sourceAnchors": [anchor.clone()],
                "provenanceStatus": "derived",
                "label": number.to_string(),
                "children": [paragraph_node(
                    &format!("{task_id}-flow-prompt-{slot_id}"),
                    &prompt_for_number(lines, *number),
                    vec![anchor.clone()],
                    vec![answer_slot_node(
                        &slot_id,
                        &number.to_string(),
                        anchor,
                        completion_placeholder(&TaskTypeV2::FlowchartCompletion),
                    )],
                )],
                "slotIds": [slot_id]
            })
        })
        .collect::<Vec<_>>();
    json!({
        "type": "flowchart",
        "id": format!("{task_id}-flowchart"),
        "sourceAnchors": lines.iter().map(|line| line.source_anchor.clone()).collect::<Vec<_>>(),
        "provenanceStatus": "derived",
        "steps": steps
    })
}

fn source_anchor_for_number(lines: &[SemanticLine], number: u32) -> Value {
    lines
        .iter()
        .find(|line| line.text.trim_start().starts_with(&number.to_string()))
        .or_else(|| lines.first())
        .map(|line| line.source_anchor.clone())
        .unwrap_or_else(empty_anchor)
}

fn prompt_for_number(lines: &[SemanticLine], number: u32) -> String {
    lines
        .iter()
        .find(|line| line.text.trim_start().starts_with(&number.to_string()))
        .map(|line| normalize_instruction_text(&line.text))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| format!("Answer {number}"))
}

fn paragraph_node(id: &str, text: &str, source_anchors: Vec<Value>, children: Vec<Value>) -> Value {
    let children = if children.is_empty() {
        vec![text_node(
            &format!("{id}-text"),
            text,
            source_anchors.first().cloned(),
        )]
    } else {
        children
    };
    json!({
        "type": "paragraph",
        "id": id,
        "sourceAnchors": source_anchors,
        "provenanceStatus": "derived",
        "children": children
    })
}

fn text_node(id: &str, text: &str, source_anchor: Option<Value>) -> Value {
    json!({
        "type": "text",
        "id": id,
        "sourceAnchors": source_anchor.into_iter().collect::<Vec<_>>(),
        "provenanceStatus": "source",
        "text": text
    })
}

fn exam_value(job: &ImportJob, _source: Option<&SourceFile>) -> Value {
    json!({
        "examId": format!("{}-{}", job.category.clone().unwrap_or_else(|| "P1".to_string()).to_ascii_lowercase(), &job.job_id[job.job_id.len().saturating_sub(8)..]),
        "title": job.title,
        "category": job.category.clone().unwrap_or_else(|| "P1".to_string()),
        "frequency": job.frequency.clone().unwrap_or_else(|| "medium".to_string()),
        "language": "en",
        "tags": job.tags,
        "sourceFiles": job.source_files.iter().map(|file| json!({
            "sourceFileId": file.file_id,
            "role": source_role(&file.role)
        })).collect::<Vec<_>>()
    })
}

fn source_role(role: &str) -> &'static str {
    let lower = role.to_ascii_lowercase();
    if lower.contains("answer") {
        "answer_key"
    } else if lower.contains("audio") {
        "audio"
    } else if lower.contains("transcript") {
        "transcript"
    } else if lower.contains("supplement") {
        "supplement"
    } else {
        "question_paper"
    }
}

fn answer_slots_answer_key(
    slots: &Map<String, Value>,
    v1_answer_key: &Map<String, Value>,
) -> Value {
    let mut answer_key = Map::new();
    for (slot_id, slot) in slots {
        let number = slot
            .get("questionNumber")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        answer_key.insert(
            slot_id.clone(),
            answer_key::answer_value_for_slot(v1_answer_key, slot_id, number),
        );
    }
    Value::Object(answer_key)
}

fn align_answer_key_assignments(answer_key: &mut Value, task_groups: &[Value]) {
    let Some(entries) = answer_key.as_object_mut() else {
        return;
    };
    for response in task_groups
        .iter()
        .filter_map(|group| group.get("responseGroups").and_then(Value::as_array))
        .flatten()
        .filter(|response| {
            response.get("assignment").and_then(Value::as_str) == Some("unordered_set")
        })
    {
        for slot_id in response
            .get("slotIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let Some(answer) = entries.get_mut(slot_id) else {
                continue;
            };
            if answer.get("kind").and_then(Value::as_str) == Some("option") {
                answer["assignment"] = json!("unordered_set");
            }
        }
    }
}

fn find_v1_group<'a>(authoring: &'a Value, task_id: &str) -> Option<&'a Value> {
    authoring
        .get("groups")
        .and_then(Value::as_array)
        .and_then(|groups| {
            groups
                .iter()
                .find(|group| group.get("groupId").and_then(Value::as_str) == Some(task_id))
        })
}

fn candidate_question<'a>(candidate: &'a Value, number: u32) -> Option<&'a Value> {
    candidate
        .get("questions")
        .and_then(Value::as_array)
        .and_then(|questions| {
            questions.iter().find(|question| {
                question
                    .get("displayNumber")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<u32>().ok())
                    == Some(number)
            })
        })
}

fn find_v1_question<'a>(group: &'a Value, number: u32) -> Option<&'a Value> {
    group
        .get("questions")
        .and_then(Value::as_array)
        .and_then(|questions| {
            questions.iter().find(|question| {
                question
                    .get("displayNumber")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<u32>().ok())
                    == Some(number)
            })
        })
}

fn question_anchor<'a>(anchors: &'a [QuestionAnchor], number: u32) -> Option<&'a QuestionAnchor> {
    anchors
        .iter()
        .find(|anchor| anchor.question_number == number)
}

fn group_source_anchors(
    zone_anchors: &[Value],
    question_anchors: &[QuestionAnchor],
    lines: &[SemanticLine],
    physical_lines: &[SemanticLine],
    source_file_id: &str,
    source_hash: &str,
    source_type: &str,
) -> Vec<Value> {
    let mut values = zone_anchors.to_vec();
    values.extend(
        question_anchors
            .iter()
            .map(|anchor| anchor.source_anchor.clone()),
    );
    values.extend(lines.iter().map(|line| line.source_anchor.clone()));
    values.extend(physical_span_anchors(&values, physical_lines));
    if values.is_empty() {
        values.push(source_anchor_from_job(
            source_file_id,
            source_hash,
            source_type,
            Vec::new(),
            0,
            None,
        ));
    }
    valid_anchors(&values)
}

fn physical_span_anchors(anchors: &[Value], physical_lines: &[SemanticLine]) -> Vec<Value> {
    let anchored_node_ids = anchors
        .iter()
        .flat_map(anchor_node_ids)
        .collect::<BTreeSet<_>>();
    if anchored_node_ids.is_empty() {
        return Vec::new();
    }

    let mut matched_ranges = std::collections::BTreeMap::<i32, (usize, usize)>::new();
    for (index, line) in physical_lines.iter().enumerate() {
        if anchor_node_ids(&line.source_anchor)
            .iter()
            .any(|node_id| anchored_node_ids.contains(node_id))
        {
            matched_ranges
                .entry(line.page_index)
                .and_modify(|range| range.1 = index)
                .or_insert((index, index));
        }
    }

    matched_ranges
        .into_iter()
        .flat_map(|(page_index, (start, end))| {
            physical_lines[start..=end]
                .iter()
                .filter(move |line| line.page_index == page_index)
                .map(|line| line.source_anchor.clone())
        })
        .collect()
}

fn anchor_node_ids(anchor: &Value) -> Vec<String> {
    anchor
        .get("nodeIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|node_id| !node_id.trim().is_empty())
        .map(ToString::to_string)
        .collect()
}

fn valid_anchors(values: &[Value]) -> Vec<Value> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for value in values {
        let Some(anchor) = anchor_from_value(value) else {
            continue;
        };
        let key = serde_json::to_string(&anchor).unwrap_or_default();
        if seen.insert(key) {
            result.push(anchor);
        }
    }
    result
}

fn candidate_block_ids(candidate: &Value) -> Vec<String> {
    candidate
        .get("blockIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect()
}

fn expression_from_candidate(candidate: &Value) -> Option<QuestionNumberExpressionV2> {
    let values = candidate.get("questionRange").and_then(Value::as_array)?;
    let start = values.first()?.as_u64()? as u32;
    let end = values
        .get(1)
        .and_then(Value::as_u64)
        .unwrap_or(start as u64) as u32;
    if start == end {
        Some(QuestionNumberExpressionV2::Set {
            values: vec![start],
        })
    } else {
        Some(QuestionNumberExpressionV2::Range { start, end })
    }
}

fn response_kind(task_type: &TaskTypeV2) -> &'static str {
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
        TaskTypeV2::DiagramLabelCompletion | TaskTypeV2::PlanMapLabelCompletion => {
            "diagram_hotspot"
        }
        _ => "text_entry",
    }
}

fn assignment_label(value: &crate::schema::ielts_authoring_v2::AssignmentV2) -> &'static str {
    match value {
        crate::schema::ielts_authoring_v2::AssignmentV2::PerSlot => "per_slot",
        crate::schema::ielts_authoring_v2::AssignmentV2::UnorderedSet => "unordered_set",
        crate::schema::ielts_authoring_v2::AssignmentV2::OrderedSlots => "ordered_slots",
    }
}

fn empty_anchor() -> Value {
    json!({
        "sourceFileId": "unknown-source",
        "pageIndex": -1,
        "nodeIds": [],
        "extractionMode": "manual",
        "sourceHash": "unknown"
    })
}

fn unresolved_task_group(task_id: &str, heading: &str, anchor: Value) -> Value {
    json!({
        "taskId": task_id,
        "displayRange": {"kind":"set","values":[]},
        "taskType": "short_answer",
        "instructions": [paragraph_node(
            &format!("{task_id}-instructions"),
            heading,
            vec![anchor.clone()],
            Vec::new(),
        )],
        "instructionSignature": {
            "normalizedText": heading,
            "taskType": "short_answer",
            "expectedQuestionNumbers": [],
            "expectedSlotCount": 0,
            "answerAssignment": "per_slot",
            "evidenceAnchors": [anchor.clone()],
            "confidence": 0.0
        },
        "responseGroups": [],
        "sourceAnchors": [anchor],
        "quality": {"score":0.0,"sourceCoverage":0.0,"hardFailures":["QUESTION_RANGE_UNPARSED"]},
        "reviewState": "unreviewed"
    })
}

fn refresh_group_quality(authoring: &mut Value) {
    let report = authoring
        .get("quality")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let scores = report
        .get("taskScores")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let issues = report
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(groups) = authoring
        .get_mut("taskGroups")
        .and_then(Value::as_array_mut)
    {
        for group in groups {
            let task_id = group
                .get("taskId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let score = scores.get(&task_id).and_then(Value::as_f64).unwrap_or(0.0);
            let hard_failures = issues
                .iter()
                .filter(|issue| {
                    issue.get("targetId").and_then(Value::as_str) == Some(task_id.as_str())
                        && issue.get("severity").and_then(Value::as_str) == Some("blocking")
                })
                .filter_map(|issue| issue.get("code").and_then(Value::as_str))
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            group["quality"] = json!({
                "score": score,
                "sourceCoverage": if group.get("sourceAnchors").and_then(Value::as_array).is_some_and(|items| !items.is_empty()) { 1.0 } else { 0.0 },
                "hardFailures": hard_failures
            });
        }
    }
}

fn build_shadow_compare(
    job: &ImportJob,
    v1_authoring: &Value,
    v2_authoring: &Value,
    physical_shadow: Option<&Value>,
) -> Value {
    let v1_groups = v1_authoring
        .get("groups")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let v2_groups = v2_authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let v1_slots = v1_authoring
        .get("questionOrder")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let v2_slots = v2_authoring
        .get("answerSlots")
        .and_then(Value::as_object)
        .map(Map::len)
        .unwrap_or(0);
    json!({
        "schemaVersion": "IeltsAuthoringIRV2CompareReportV1",
        "jobId": job.job_id,
        "status": "complete",
        "summary": {
            "v1GroupCount": v1_groups,
            "v2GroupCount": v2_groups,
            "v1SlotCount": v1_slots,
            "v2SlotCount": v2_slots,
            "v2QualityState": v2_authoring.pointer("/quality/state").cloned().unwrap_or(Value::Null)
        },
        "physicalShadowAvailable": physical_shadow.is_some(),
        "policy": {
            "v1RemainsAuthoritative": true,
            "v2EntersAuthoring": false,
            "v2EntersExport": false,
            "differencesRequireReview": true,
            "pdfPerQuestionLlmRepair": false
        }
    })
}

fn strip_html(input: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                output.push(' ');
            }
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    normalize_instruction_text(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn semantic_line(id: &str, text: &str, page_index: i32, node_id: &str) -> SemanticLine {
        SemanticLine {
            id: id.to_string(),
            text: text.to_string(),
            source_anchor: json!({"nodeIds":[node_id]}),
            page_index,
            order: 0,
            role: String::new(),
            bbox: None,
        }
    }

    fn anchored_semantic_line(
        id: &str,
        text: &str,
        page_index: i32,
        node_id: &str,
    ) -> SemanticLine {
        let mut line = semantic_line(id, text, page_index, node_id);
        line.source_anchor = json!({
            "sourceFileId":"source-1",
            "pageIndex":page_index,
            "nodeIds":[node_id],
            "extractionMode":"pdf_native",
            "sourceHash":"a".repeat(64)
        });
        line
    }

    #[test]
    fn passage_preamble_is_preserved_as_supporting_source_evidence() {
        let lines = vec![
            anchored_semantic_line(
                "line-1",
                "You should spend about 20 minutes on Questions 1-13, which are based on Reading",
                0,
                "g1",
            ),
            anchored_semantic_line("line-2", "Passage 1 below.", 0, "g2"),
            anchored_semantic_line("line-3", "Passage title", 0, "g3"),
        ];

        let anchors = passage_preamble_anchors(&lines, &BTreeSet::from([0]));

        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0]["nodeIds"], json!(["g1"]));
        assert_eq!(anchors[1]["nodeIds"], json!(["g2"]));
    }

    #[test]
    fn task_group_source_evidence_includes_all_rebound_group_lines() {
        let lines = vec![
            anchored_semantic_line("line-1", "Questions 1-2", 0, "g1"),
            anchored_semantic_line("line-2", "A option", 0, "g2"),
            anchored_semantic_line("line-3", "2 final prompt", 0, "g3"),
        ];

        let anchors =
            group_source_anchors(&[], &[], &lines, &lines, "source-1", &"a".repeat(64), "pdf");
        let node_ids = anchors
            .iter()
            .flat_map(|anchor| {
                anchor
                    .get("nodeIds")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();

        assert_eq!(node_ids, vec!["g1", "g2", "g3"]);
    }

    #[test]
    fn shared_prompt_recovers_question_text_when_v1_prompt_is_empty() {
        let lines = vec![
            semantic_line("heading", "Questions 14 and 15", 0, "heading"),
            semantic_line("choose", "Choose TWO letters, A-E.", 0, "choose"),
            semantic_line(
                "write",
                "Write the correct letters in boxes 14 and 15.",
                0,
                "write",
            ),
            semantic_line(
                "question",
                "According to the writer, which TWO are characteristics of the approach?",
                0,
                "question",
            ),
            semantic_line("options", "A first option B second option", 0, "options"),
        ];
        let v1_group = json!({"questions":[{"prompt":""}]});

        assert_eq!(
            shared_prompt(Some(&v1_group), &lines, Some(14)),
            "According to the writer, which TWO are characteristics of the approach?"
        );
    }

    #[test]
    fn completion_prompt_fallback_preserves_source_instructions() {
        let lines = vec![
            semantic_line("heading", "Questions 24-26", 0, "heading"),
            semantic_line("complete", "Complete the summary below.", 0, "complete"),
            semantic_line(
                "choose",
                "Choose ONE WORD ONLY from the passage for each answer.",
                0,
                "choose",
            ),
            semantic_line("write", "Write your answers in boxes 24-26.", 0, "write"),
        ];

        let (text, anchors) = completion_prompt_fallback(&lines).expect("fallback prompt");
        assert_eq!(
            text,
            "Complete the summary below. Choose ONE WORD ONLY from the passage for each answer. Write your answers in boxes 24-26."
        );
        assert_eq!(anchors.len(), 3);
    }

    #[test]
    fn group_source_evidence_closes_same_page_physical_span_without_cross_page_leak() {
        let mut first = anchored_semantic_line("physical-1", "Question heading", 0, "g1");
        first.bbox = Some([10.0, 10.0, 100.0, 10.0]);
        let mut middle = anchored_semantic_line("physical-2", "Option legend", 0, "g2");
        middle.bbox = Some([10.0, 22.0, 100.0, 10.0]);
        let mut last = anchored_semantic_line("physical-3", "Question prompt", 0, "g3");
        last.bbox = Some([10.0, 34.0, 100.0, 10.0]);
        let other_page = anchored_semantic_line("physical-4", "Other page", 1, "g4");
        let physical = vec![first, middle, last, other_page];

        let anchors = group_source_anchors(
            &[
                physical[0].source_anchor.clone(),
                physical[2].source_anchor.clone(),
            ],
            &[],
            &[],
            &physical,
            "source-1",
            &"a".repeat(64),
            "pdf",
        );
        let node_ids = anchors
            .iter()
            .flat_map(anchor_node_ids)
            .collect::<BTreeSet<_>>();
        assert!(node_ids.contains("g1"));
        assert!(node_ids.contains("g2"));
        assert!(node_ids.contains("g3"));
        assert!(!node_ids.contains("g4"));
    }

    #[test]
    fn passage_preamble_does_not_cross_into_question_page() {
        let lines = vec![
            anchored_semantic_line(
                "line-1",
                "You should spend about 20 minutes on Questions 1-13, which are based on Reading",
                0,
                "g1",
            ),
            anchored_semantic_line("line-2", "Passage 1 below.", 0, "g2"),
            anchored_semantic_line("line-3", "Question page text", 1, "g3"),
        ];
        let anchors = passage_preamble_anchors(&lines, &BTreeSet::from([1]));
        assert!(anchors.is_empty());
    }

    #[test]
    fn v1_evidence_rebinds_only_to_a_unique_same_page_physical_line() {
        let physical = vec![
            semantic_line("physical-1", "Questions 1 - 2", 0, "g1"),
            semantic_line("physical-2", "Repeated", 0, "g2"),
            semantic_line("physical-3", "Repeated", 0, "g3"),
            semantic_line("physical-4", "Other page", 1, "g4"),
        ];
        let mut evidence = vec![
            semantic_line("b1", "Questions 1-2", 0, "b1"),
            semantic_line("b2", "Repeated", 0, "b2"),
            semantic_line("b3", "Other page", 0, "b3"),
        ];

        rebind_lines_to_physical(&mut evidence, &physical);

        assert_eq!(evidence[0].source_anchor["nodeIds"], json!(["g1"]));
        assert_eq!(evidence[1].source_anchor["nodeIds"], json!(["b2"]));
        assert_eq!(evidence[2].source_anchor["nodeIds"], json!(["b3"]));
    }

    #[test]
    fn v1_block_rebinds_to_every_line_in_a_unique_contiguous_physical_span() {
        let mut first = semantic_line("physical-1", "A long passage starts", 0, "g1");
        first.bbox = Some([10.0, 20.0, 100.0, 10.0]);
        let mut second = semantic_line("physical-2", "and continues here.", 0, "g2");
        second.bbox = Some([10.0, 32.0, 90.0, 10.0]);
        let physical = vec![first, second];
        let mut evidence = vec![semantic_line(
            "b1",
            "A long passage starts and continues here.",
            0,
            "b1",
        )];

        rebind_lines_to_physical(&mut evidence, &physical);

        assert_eq!(evidence[0].source_anchor["nodeIds"], json!(["g1", "g2"]));
        assert_eq!(evidence[0].bbox, Some([10.0, 20.0, 100.0, 22.0]));
        assert_eq!(evidence[0].source_anchor["bbox"]["height"], json!(22.0));
    }

    #[test]
    fn v1_evidence_bbox_fallback_rebinds_all_overlapping_physical_lines() {
        let mut first = semantic_line("physical-1", "First physical line", 0, "g1");
        first.bbox = Some([10.0, 20.0, 100.0, 10.0]);
        let mut second = semantic_line("physical-2", "Second physical line", 0, "g2");
        second.bbox = Some([10.0, 32.0, 90.0, 10.0]);
        let mut other_page = semantic_line("physical-3", "Other page", 1, "g3");
        other_page.bbox = Some([10.0, 20.0, 100.0, 22.0]);
        let physical = vec![first, second, other_page];
        let mut evidence = vec![semantic_line("b1", "Truncated preview", 0, "b1")];
        evidence[0].bbox = Some([5.0, 15.0, 120.0, 35.0]);

        rebind_lines_to_physical(&mut evidence, &physical);

        assert_eq!(evidence[0].source_anchor["nodeIds"], json!(["g1", "g2"]));
        assert_eq!(evidence[0].bbox, Some([10.0, 20.0, 100.0, 22.0]));
    }

    fn job() -> ImportJob {
        ImportJob {
            job_id: "phase4-test-job".to_string(),
            title: "Grammar fixture".to_string(),
            status: crate::JobStatus::Working,
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: vec!["fixture".to_string()],
            source_files: vec![SourceFile {
                file_id: "file-1".to_string(),
                original_name: "fixture.pdf".to_string(),
                stored_name: "fixture.pdf".to_string(),
                file_type: "pdf".to_string(),
                sha256: "a".repeat(64),
                size_bytes: 1,
                role: "MainQuestion".to_string(),
                imported_at: Utc::now(),
            }],
            active_llm_profile_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            current_step: crate::WorkflowStep::Authoring,
            issue_counts: crate::IssueCounts::default(),
        }
    }

    #[test]
    fn builds_typed_shadow_without_mutating_v1_shape() {
        let v1 = json!({
            "schemaVersion":"ReadingAuthoringIRV1",
            "groups":[{
                "groupId":"group-1",
                "questionRange":[1,3],
                "questions":[
                    {"id":"q1","displayNumber":"1","prompt":"First statement","interaction":{"options":["TRUE","FALSE","NOT GIVEN"]},"answer":"TRUE"},
                    {"id":"q2","displayNumber":"2","prompt":"Second statement","interaction":{"options":["TRUE","FALSE","NOT GIVEN"]},"answer":"FALSE"},
                    {"id":"q3","displayNumber":"3","prompt":"Third statement","interaction":{"options":["TRUE","FALSE","NOT GIVEN"]},"answer":"NOT GIVEN"}
                ]
            }],
            "answerKey":{"q1":"TRUE","q2":"FALSE","q3":"NOT GIVEN"},
            "passage":{"htmlBlocks":[]}
        });
        let split = json!({
            "questionGroupCandidates":[{
                "groupId":"group-1",
                "heading":"Questions 1-3",
                "instructionText":"Questions 1-3 Do the following statements agree? TRUE FALSE NOT GIVEN",
                "questionRange":[1,3],
                "kindHint":"true_false_not_given",
                "sectionEvidence":[
                    {"blockId":"h","textPreview":"Questions 1-3 Do the following statements agree? TRUE FALSE NOT GIVEN","pageIndex":1},
                    {"blockId":"q1","textPreview":"1. First statement","pageIndex":1},
                    {"blockId":"q2","textPreview":"2. Second statement","pageIndex":1},
                    {"blockId":"q3","textPreview":"3. Third statement","pageIndex":1}
                ]
            }],
            "passageCandidates":[]
        });
        let value = build_authoring_v2_shadow(&job(), &v1, &split, None, None).unwrap();
        assert_eq!(
            value.get("schemaVersion").and_then(Value::as_str),
            Some("IeltsAuthoringIRV2")
        );
        assert_eq!(
            value
                .pointer("/taskGroups/0/instructionSignature/taskType")
                .and_then(Value::as_str),
            Some("true_false_not_given")
        );
        assert_eq!(
            value
                .get("answerSlots")
                .and_then(Value::as_object)
                .map(Map::len),
            Some(3)
        );
        assert!(value.pointer("/taskGroups/0/optionBank/title").is_none());
        assert_eq!(
            v1.get("schemaVersion").and_then(Value::as_str),
            Some("ReadingAuthoringIRV1")
        );
    }

    #[test]
    fn matching_people_uses_one_source_backed_bank_and_response_group() {
        let questions = (20..=25)
            .map(|number| {
                json!({
                    "id":format!("q{number}"),
                    "displayNumber":number.to_string(),
                    "prompt":format!("Statement {number}"),
                    "interaction":{"options":["A","B","C","D"]}
                })
            })
            .collect::<Vec<_>>();
        let v1 = json!({
            "schemaVersion":"ReadingAuthoringIRV1",
            "groups":[{"groupId":"group-1","questionRange":[20,25],"questions":questions}],
            "answerKey":{"q20":"A","q21":"B","q22":"C","q23":"D","q24":"A","q25":"B"},
            "passage":{"htmlBlocks":[]}
        });
        let split = json!({
            "questionGroupCandidates":[{
                "groupId":"group-1",
                "heading":"Questions 20-25",
                "instructionText":"Questions 20-25 Match each statement with the correct person, A, B, C or D. NB You may use any letter more than once.",
                "questionRange":[20,25],
                "kindHint":"matching_features",
                "sectionEvidence":[
                    {"blockId":"h","textPreview":"Questions 20-25 Match each statement with the correct person, A, B, C or D. NB You may use any letter more than once.","pageIndex":4},
                    {"blockId":"q20","textPreview":"20 Statement 20","pageIndex":4},
                    {"blockId":"q21","textPreview":"21 Statement 21","pageIndex":4},
                    {"blockId":"q22","textPreview":"22 Statement 22","pageIndex":4},
                    {"blockId":"q23","textPreview":"23 Statement 23","pageIndex":4},
                    {"blockId":"q24","textPreview":"24 Statement 24","pageIndex":4},
                    {"blockId":"q25","textPreview":"25 Statement 25","pageIndex":4},
                    {"blockId":"title","textPreview":"List of People","pageIndex":4},
                    {"blockId":"people","textPreview":"A Alan Macfarlane B Professor Ludovic Vallier C Dr Meritxell Huch D Dr Madeline Lancaster","pageIndex":4}
                ]
            }],
            "passageCandidates":[]
        });
        let value = build_authoring_v2_shadow(&job(), &v1, &split, None, None).unwrap();
        let group = &value["taskGroups"][0];
        assert_eq!(
            group["optionBank"]["options"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|option| option.get("label").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["A", "B", "C", "D"]
        );
        assert_eq!(
            group.pointer("/optionBank/options/0/content/0/text"),
            Some(&json!("Alan Macfarlane"))
        );
        assert_eq!(group["responseGroups"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            group.pointer("/responseGroups/0/responseGroupId"),
            Some(&json!("group-1-responses"))
        );
        assert_eq!(
            group.pointer("/responseGroups/0/optionBankRef"),
            Some(&json!("group-1-option-bank"))
        );
        assert!(group.pointer("/responseGroups/0/options").is_none());
    }

    #[test]
    fn completion_response_ignores_incidental_v1_choice_options() {
        let questions = (24..=26)
            .map(|number| {
                json!({
                    "id":format!("q{number}"),
                    "displayNumber":number.to_string(),
                    "prompt":format!("Sentence {number} ____"),
                    "interaction":{"options":["A","B","C","D"]}
                })
            })
            .collect::<Vec<_>>();
        let v1 = json!({
            "schemaVersion":"ReadingAuthoringIRV1",
            "groups":[{"groupId":"group-1","questionRange":[24,26],"questions":questions}],
            "answerKey":{"q24":"alpha","q25":"beta","q26":"gamma"},
            "passage":{"htmlBlocks":[]}
        });
        let split = json!({
            "questionGroupCandidates":[{
                "groupId":"group-1",
                "heading":"Questions 24-26",
                "instructionText":"Questions 24-26 Complete the sentences below. Choose NO MORE THAN TWO WORDS from the passage for each answer.",
                "questionRange":[24,26],
                "kindHint":"sentence_completion",
                "sectionEvidence":[
                    {"blockId":"h","textPreview":"Questions 24-26 Complete the sentences below. Choose NO MORE THAN TWO WORDS.","pageIndex":1},
                    {"blockId":"q24","textPreview":"24 Sentence 24 ____","pageIndex":1},
                    {"blockId":"q25","textPreview":"25 Sentence 25 ____","pageIndex":1},
                    {"blockId":"q26","textPreview":"26 Sentence 26 ____","pageIndex":1}
                ]
            }],
            "passageCandidates":[]
        });

        let value = build_authoring_v2_shadow(&job(), &v1, &split, None, None).unwrap();
        let response = &value["taskGroups"][0]["responseGroups"][0];
        assert_eq!(
            value.pointer("/taskGroups/0/taskType"),
            Some(&json!("sentence_completion"))
        );
        assert_eq!(response["slotIds"], json!(["q24", "q25", "q26"]));
        assert!(response.get("options").is_none());
        assert!(response.get("optionBankRef").is_none());
    }

    #[test]
    fn choose_two_aligns_unordered_answer_assignment_and_compiles() {
        let v1 = json!({
            "schemaVersion":"ReadingAuthoringIRV1",
            "groups":[{
                "groupId":"group-1",
                "questionRange":[14,15],
                "questions":[
                    {"id":"q14","displayNumber":"14","prompt":"and","interaction":{"options":["A","B","C","D","E"]}},
                    {"id":"q15","displayNumber":"15","prompt":"and","interaction":{"options":["A","B","C","D","E"]}}
                ]
            }],
            "answerKey":{"q14":"A","q15":"D"},
            "passage":{"htmlBlocks":[]}
        });
        let split = json!({
            "questionGroupCandidates":[{
                "groupId":"group-1",
                "heading":"Questions 14 and 15",
                "instructionText":"Questions 14 and 15 Choose TWO letters, A-E.",
                "questionRange":[14,15],
                "kindHint":"multi_choice",
                "sectionEvidence":[
                    {"blockId":"h","textPreview":"Questions 14 and 15 Choose TWO letters, A-E.","pageIndex":4},
                    {"blockId":"prompt","textPreview":"Which TWO characteristics apply?","pageIndex":4},
                    {"blockId":"options","textPreview":"A First B Second C Third D Fourth E Fifth","pageIndex":4}
                ]
            }],
            "passageCandidates":[]
        });
        let value = build_authoring_v2_shadow(&job(), &v1, &split, None, None).unwrap();
        assert_eq!(
            value.pointer("/taskGroups/0/responseGroups/0/responseGroupId"),
            Some(&json!("group-1-responses"))
        );
        assert_eq!(
            value.pointer("/taskGroups/0/responseGroups/0/assignment"),
            Some(&json!("unordered_set"))
        );
        assert_eq!(
            value.pointer("/taskGroups/0/responseGroups/0/scoringPolicy"),
            Some(&json!("per_slot_ielts_normalized"))
        );
        assert_eq!(
            value.pointer("/answerKey/q14/assignment"),
            Some(&json!("unordered_set"))
        );
        assert_eq!(
            value.pointer("/answerKey/q15/assignment"),
            Some(&json!("unordered_set"))
        );
        assert_eq!(
            value.pointer("/quality/compilerProbes/v2Runtime/status"),
            Some(&json!("passed"))
        );
        assert_eq!(
            value.pointer("/quality/compilerProbes/v1Compatibility/status"),
            Some(&json!("passed"))
        );
    }

    #[test]
    fn checked_in_complex_fixture_projects_question_groups_and_slots() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures/golden/baseline/v1/parser-complex-reading.json");
        let wrapper: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let payload = wrapper.get("payload").unwrap();
        let value = build_authoring_v2_shadow(
            &job(),
            payload.get("authoringIr").unwrap(),
            payload.get("splitCandidates").unwrap(),
            payload.get("documentIr"),
            None,
        )
        .unwrap();
        assert_eq!(
            value
                .get("taskGroups")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(
            value
                .get("answerSlots")
                .and_then(Value::as_object)
                .map(Map::len),
            Some(5)
        );
        assert_eq!(
            value
                .pointer("/taskGroups/0/instructionSignature/taskType")
                .and_then(Value::as_str),
            Some("true_false_not_given")
        );
        assert_eq!(
            value
                .pointer("/taskGroups/1/instructionSignature/taskType")
                .and_then(Value::as_str),
            Some("table_completion")
        );
    }

    #[test]
    fn phase4_fixture_matrix_covers_expression_signature_and_fallback_rules() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures/golden/synthetic/ielts/phase4-grammar-fixtures.json");
        let matrix: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();

        for case in matrix
            .get("questionExpressions")
            .and_then(Value::as_array)
            .unwrap()
        {
            let text = case.get("text").and_then(Value::as_str).unwrap();
            let parsed = parse_question_expression(text);
            if case
                .get("accepted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let expression = parsed.expect("accepted question expression should parse");
                assert_eq!(
                    serde_json::to_value(&expression).unwrap(),
                    case.get("expression").cloned().unwrap()
                );
                assert_eq!(
                    expand_expression(&expression),
                    case.get("numbers")
                        .and_then(Value::as_array)
                        .unwrap()
                        .iter()
                        .filter_map(Value::as_u64)
                        .map(|number| number as u32)
                        .collect::<Vec<_>>()
                );
            } else {
                assert!(parsed.is_none(), "unexpected expression parse for {text}");
            }
        }

        for case in matrix
            .get("instructionSignatures")
            .and_then(Value::as_array)
            .unwrap()
        {
            let expression =
                parse_question_expression(case.get("expression").and_then(Value::as_str).unwrap())
                    .expect("fixture signature expression should parse");
            let result = infer_instruction_signature(
                case.get("text").and_then(Value::as_str).unwrap(),
                &expression,
                case.get("kindHint").and_then(Value::as_str),
                Vec::new(),
            );
            assert_eq!(
                task_type_label(&result.signature.task_type),
                case.get("taskType").and_then(Value::as_str).unwrap()
            );
            assert_eq!(
                result.signature.expected_question_numbers.len(),
                case.get("expectedQuestionCount")
                    .and_then(Value::as_u64)
                    .unwrap() as usize
            );
            if let Some(expected) = case.get("selectionCardinality").and_then(Value::as_u64) {
                assert_eq!(
                    result
                        .signature
                        .selection_cardinality
                        .and_then(|value| value.exact),
                    Some(expected as u32)
                );
            }
            if let Some(expected) = case.get("allowOptionReuse").and_then(Value::as_bool) {
                assert_eq!(result.signature.allow_option_reuse, Some(expected));
            }
            if let Some(expected) = case.get("maxWords").and_then(Value::as_u64) {
                assert_eq!(
                    result
                        .signature
                        .word_limit
                        .as_ref()
                        .and_then(|limit| limit.max_words),
                    Some(expected as u32)
                );
            }
            if let Some(expected) = case.get("maxNumbers").and_then(Value::as_u64) {
                assert_eq!(
                    result
                        .signature
                        .word_limit
                        .as_ref()
                        .and_then(|limit| limit.max_numbers),
                    Some(expected as u32)
                );
            }
        }

        let bank_case = matrix
            .get("semanticScenarios")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|case| {
                case.get("id").and_then(Value::as_str) == Some("matching_people_option_bank")
            })
            .unwrap();
        let lines = bank_case
            .get("lines")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .enumerate()
            .map(|(order, line)| SemanticLine {
                id: line.get("id").and_then(Value::as_str).unwrap().to_string(),
                text: line
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
                source_anchor: json!({"nodeIds":[line.get("id").and_then(Value::as_str).unwrap()]}),
                page_index: 0,
                order,
                role: String::new(),
                bbox: None,
            })
            .collect::<Vec<_>>();
        let bank = detect_option_bank(
            &lines,
            bank_case
                .get("instruction")
                .and_then(Value::as_str)
                .unwrap(),
            Some(false),
            true,
        )
        .expect("matching people fixture should have a closed option bank");
        assert_eq!(
            bank.run.options.len(),
            bank_case
                .get("expectedOptionCount")
                .and_then(Value::as_u64)
                .unwrap() as usize
        );
    }
}
