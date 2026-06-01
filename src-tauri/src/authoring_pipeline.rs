use crate::{html_escape, ImportJob};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PassageCandidateV1 {
    pub range: Vec<String>,
    pub title: String,
    pub category_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GroupInteractionClassificationV1 {
    pub r#type: String,
    pub options: Vec<String>,
    pub allow_option_reuse: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_selections: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_selections: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GroupClassificationV1 {
    pub kind: String,
    pub interaction: GroupInteractionClassificationV1,
    pub confidence: f64,
    pub warnings: Vec<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SplitSectionEvidenceV1 {
    pub block_id: String,
    pub page_index: u64,
    pub column: u8,
    pub role: String,
    pub text_preview: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_bbox: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_rotation: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_rows: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_cols: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_has_col_spans: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_has_vertical_merges: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table_merged_cell_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_level: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numbering_level: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numbering_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_column_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SplitContinuationEdgeV1 {
    pub from_block_id: String,
    pub to_block_id: String,
    pub reason: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SplitGroupCandidateV1 {
    pub group_id: String,
    pub heading: String,
    pub question_range: [u32; 2],
    pub instruction_text: String,
    pub block_ids: Vec<String>,
    pub kind_hint: String,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<GroupClassificationV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub section_evidence: Vec<SplitSectionEvidenceV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub continuation_edges: Vec<SplitContinuationEdgeV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_umbrella_range: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_manual_question_import: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UmbrellaQuestionRangeV1 {
    pub heading: String,
    pub question_range: [u32; 2],
    pub block_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnswerKeyCandidateV1 {
    pub source: String,
    pub answers: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SplitCandidatesV1 {
    pub job_id: String,
    pub passage_candidates: Vec<PassageCandidateV1>,
    pub question_group_candidates: Vec<SplitGroupCandidateV1>,
    pub answer_key_candidates: Vec<AnswerKeyCandidateV1>,
    pub umbrella_question_ranges: Vec<UmbrellaQuestionRangeV1>,
    pub issues: Vec<String>,
}

impl SplitCandidatesV1 {
    fn to_value(&self) -> Value {
        serde_json::to_value(self)
            .expect("SplitCandidatesV1 only contains JSON-serializable fields")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthoringSourceFileV1 {
    pub file_id: String,
    pub original_name: String,
    pub stored_name: String,
    pub file_type: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub role: String,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExamMetaDraftV1 {
    pub exam_id: String,
    pub title: String,
    pub category: String,
    pub frequency: String,
    pub tags: Vec<String>,
    pub source_files: Vec<AuthoringSourceFileV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PassageHtmlBlockV1 {
    pub block_id: String,
    pub html: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PassageDraftV1 {
    pub title: String,
    pub html_blocks: Vec<PassageHtmlBlockV1>,
    pub source_block_ids: Vec<String>,
    pub question_umbrella_ranges: Vec<UmbrellaQuestionRangeV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuestionDraftV1 {
    pub id: String,
    pub display_number: String,
    pub prompt: String,
    pub interaction: Value,
    pub answer: Value,
    pub source_block_ids: Vec<String>,
    pub confidence: f64,
    pub verified: bool,
    pub requires_manual_question_import: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuestionGroupDraftV1 {
    pub group_id: String,
    pub kind: String,
    pub question_range: [u32; 2],
    pub instruction: Vec<String>,
    pub questions: Vec<QuestionDraftV1>,
    pub layout: Value,
    pub review_warnings: Vec<String>,
    pub classification_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub section_evidence: Vec<SplitSectionEvidenceV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub continuation_edges: Vec<SplitContinuationEdgeV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_option_reuse: Option<bool>,
    pub source_block_ids: Vec<String>,
    pub confidence: f64,
    pub verified: bool,
    pub is_umbrella_range: bool,
    pub requires_manual_question_import: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthoringAuditV1 {
    pub llm_used: bool,
    pub human_verified: bool,
    pub issues: Vec<Value>,
    pub revision: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadingAuthoringIrV1 {
    pub schema_version: String,
    pub job_id: String,
    pub exam: ExamMetaDraftV1,
    pub passage: PassageDraftV1,
    pub groups: Vec<QuestionGroupDraftV1>,
    pub answer_key: serde_json::Map<String, Value>,
    pub question_order: Vec<String>,
    pub question_display_map: serde_json::Map<String, Value>,
    pub audit: AuthoringAuditV1,
}

impl ReadingAuthoringIrV1 {
    fn to_value(&self) -> Value {
        serde_json::to_value(self)
            .expect("ReadingAuthoringIrV1 only contains JSON-serializable fields")
    }
}

fn split_candidates(job_id: &str) -> Value {
    json!({
        "jobId": job_id,
        "passageCandidates": [{"range":["b001","b002","b003"],"title":"The Rise and Fall of Detective Stories","categoryHint":"P1"}],
        "questionGroupCandidates": [
            {"groupId":"group-1","heading":"Questions 1-5","questionRange":[1,5],"instructionText":"Do the following statements agree with the information given in Reading Passage 1?","blockIds":["b004","b005"],"kindHint":"true_false_not_given","confidence":0.88},
            {"groupId":"group-2","heading":"Questions 6-8","questionRange":[6,8],"instructionText":"Complete the table below. Choose ONE WORD ONLY from the passage for each answer.","blockIds":["b006","b007"],"kindHint":"table_completion","confidence":0.84}
        ],
        "answerKeyCandidates": [{"source":"answer-block:b008","answers":{"1":"FALSE","2":"TRUE","3":"NOT GIVEN","4":"TRUE","5":"FALSE","6":"clues","7":"alibis","8":"narrators"}}],
        "issues": []
    })
}

pub(crate) fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
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
    collapse_whitespace(&output)
}

pub(crate) fn dynamic_document_blocks(doc: Option<&Value>) -> Vec<Value> {
    let mut blocks = doc
        .and_then(|value| value.get("pages"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .flat_map(|(page_position, page)| {
            let page_index = page
                .get("pageIndex")
                .and_then(Value::as_u64)
                .unwrap_or((page_position + 1) as u64);
            let page_width = page.get("width").and_then(Value::as_f64).unwrap_or(595.0);
            let page_height = page.get("height").and_then(Value::as_f64).unwrap_or(842.0);
            let page_rotation = page
                .get("rotation")
                .or_else(|| page.pointer("/layoutHints/rotation"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            page.get("blocks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
                .map(move |(block_position, block)| {
                    let mut block = block.clone();
                    if block.get("pageIndex").and_then(Value::as_u64).is_none() {
                        block["pageIndex"] = json!(page_index);
                    }
                    if block.get("pageRotation").and_then(Value::as_i64).is_none() {
                        block["pageRotation"] = json!(normalize_rotation_degrees(page_rotation));
                    }
                    block["_epic8PageWidth"] = json!(page_width);
                    block["_epic8PageHeight"] = json!(page_height);
                    block["_epic8PageRotation"] = json!(page_rotation);
                    block["_epic8OriginalOrder"] = json!(block_position);
                    block
                })
        })
        .collect::<Vec<_>>();
    blocks.sort_by(dynamic_reading_order_cmp);
    for block in &mut blocks {
        if let Some(map) = block.as_object_mut() {
            map.remove("_epic8PageWidth");
            map.remove("_epic8PageHeight");
            map.remove("_epic8PageRotation");
            map.remove("_epic8OriginalOrder");
        }
    }
    blocks
}

pub(crate) fn dynamic_block_text(block: &Value) -> String {
    let text = block
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if !text.is_empty() {
        return collapse_whitespace(text);
    }
    block
        .get("html")
        .and_then(Value::as_str)
        .map(strip_html)
        .unwrap_or_default()
}

pub(crate) fn dynamic_block_id(block: &Value) -> String {
    block
        .get("blockId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn dynamic_block_role(block: &Value) -> &str {
    block
        .get("roleHint")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn dynamic_block_page_index(block: &Value) -> u64 {
    block.get("pageIndex").and_then(Value::as_u64).unwrap_or(1)
}

fn dynamic_block_bbox(block: &Value) -> Option<[f64; 4]> {
    let values = block.get("bbox")?.as_array()?;
    if values.len() != 4 {
        return None;
    }
    Some([
        values.first()?.as_f64()?,
        values.get(1)?.as_f64()?,
        values.get(2)?.as_f64()?,
        values.get(3)?.as_f64()?,
    ])
}

fn normalize_rotation_degrees(value: i64) -> i64 {
    value.rem_euclid(360)
}

fn dynamic_block_page_rotation(block: &Value) -> i64 {
    normalize_rotation_degrees(
        block
            .get("_epic8PageRotation")
            .or_else(|| block.get("pageRotation"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
    )
}

fn rotate_point_to_upright(x: f64, y: f64, width: f64, height: f64, rotation: i64) -> (f64, f64) {
    match normalize_rotation_degrees(rotation) {
        90 => (y, width - x),
        180 => (width - x, height - y),
        270 => (height - y, x),
        _ => (x, y),
    }
}

fn dynamic_block_normalized_bbox(block: &Value) -> Option<[f64; 4]> {
    let bbox = dynamic_block_bbox(block)?;
    let rotation = dynamic_block_page_rotation(block);
    if rotation == 0 {
        return Some(bbox);
    }
    let page_width = block
        .get("_epic8PageWidth")
        .and_then(Value::as_f64)
        .unwrap_or(595.0);
    let page_height = block
        .get("_epic8PageHeight")
        .and_then(Value::as_f64)
        .unwrap_or(842.0);
    let points = [
        rotate_point_to_upright(bbox[0], bbox[1], page_width, page_height, rotation),
        rotate_point_to_upright(bbox[2], bbox[1], page_width, page_height, rotation),
        rotate_point_to_upright(bbox[2], bbox[3], page_width, page_height, rotation),
        rotate_point_to_upright(bbox[0], bbox[3], page_width, page_height, rotation),
    ];
    let min_x = points.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
    let min_y = points.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
    let max_x = points
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = points
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);
    Some([min_x, min_y, max_x, max_y])
}

fn dynamic_block_column(block: &Value) -> u8 {
    let Some(bbox) = dynamic_block_normalized_bbox(block) else {
        return 0;
    };
    let page_width = if matches!(dynamic_block_page_rotation(block), 90 | 270) {
        block
            .get("_epic8PageHeight")
            .and_then(Value::as_f64)
            .unwrap_or(842.0)
    } else {
        block
            .get("_epic8PageWidth")
            .and_then(Value::as_f64)
            .unwrap_or(595.0)
    };
    if bbox[0] >= page_width * 0.45 {
        1
    } else {
        0
    }
}

fn dynamic_block_text_preview(block: &Value) -> String {
    let text = dynamic_block_text(block);
    text.chars().take(120).collect::<String>()
}

fn table_merge_summary(block: &Value) -> (Option<bool>, Option<bool>, Option<u64>) {
    let Some(cells) = block.pointer("/table/cells").and_then(Value::as_array) else {
        return (None, None, None);
    };
    let has_col_spans = cells
        .iter()
        .any(|cell| cell.get("colSpan").and_then(Value::as_u64).unwrap_or(1) > 1);
    let has_vertical_merges = cells
        .iter()
        .any(|cell| cell.get("verticalMerge").and_then(Value::as_str).is_some());
    let merged_cell_count = cells
        .iter()
        .filter(|cell| {
            cell.get("colSpan").and_then(Value::as_u64).unwrap_or(1) > 1
                || cell.get("verticalMerge").and_then(Value::as_str).is_some()
        })
        .count() as u64;
    (
        Some(has_col_spans),
        Some(has_vertical_merges),
        Some(merged_cell_count),
    )
}

fn split_section_evidence_for_blocks(blocks: &[Value]) -> Vec<SplitSectionEvidenceV1> {
    blocks
        .iter()
        .map(|block| {
            let (table_has_col_spans, table_has_vertical_merges, table_merged_cell_count) =
                table_merge_summary(block);
            SplitSectionEvidenceV1 {
                block_id: dynamic_block_id(block),
                page_index: dynamic_block_page_index(block),
                column: dynamic_block_column(block),
                role: dynamic_block_role(block).to_string(),
                text_preview: dynamic_block_text_preview(block),
                bbox: dynamic_block_bbox(block),
                normalized_bbox: dynamic_block_normalized_bbox(block),
                page_rotation: Some(dynamic_block_page_rotation(block)),
                table_rows: block.pointer("/table/rows").and_then(Value::as_u64),
                table_cols: block.pointer("/table/cols").and_then(Value::as_u64),
                table_has_col_spans,
                table_has_vertical_merges,
                table_merged_cell_count,
                heading_level: block
                    .pointer("/layoutHints/headingLevel")
                    .and_then(Value::as_u64),
                numbering_level: block
                    .pointer("/layoutHints/numbering/level")
                    .and_then(Value::as_u64),
                numbering_id: block
                    .pointer("/layoutHints/numbering/id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                section_column_count: block
                    .pointer("/layoutHints/section/columns/count")
                    .and_then(Value::as_u64),
            }
        })
        .collect()
}

fn split_continuation_edges_for_blocks(blocks: &[Value]) -> Vec<SplitContinuationEdgeV1> {
    blocks
        .windows(2)
        .filter_map(|pair| {
            let from = pair.first()?;
            let to = pair.get(1)?;
            let from_id = dynamic_block_id(from);
            let to_id = dynamic_block_id(to);
            if from_id.is_empty() || to_id.is_empty() {
                return None;
            }
            let reason = if dynamic_block_page_index(from) != dynamic_block_page_index(to) {
                "cross-page-continuation"
            } else if dynamic_block_column(from) != dynamic_block_column(to) {
                "cross-column-continuation"
            } else {
                "same-section-continuation"
            };
            Some(SplitContinuationEdgeV1 {
                from_block_id: from_id,
                to_block_id: to_id,
                reason: reason.to_string(),
                confidence: 0.72,
            })
        })
        .collect()
}

fn dynamic_block_order_role_rank(block: &Value) -> u8 {
    match dynamic_block_role(block) {
        "answer" => 3,
        "ignore" => 4,
        _ => 0,
    }
}

fn dynamic_reading_order_cmp(left: &Value, right: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    dynamic_block_page_index(left)
        .cmp(&dynamic_block_page_index(right))
        .then_with(|| {
            dynamic_block_order_role_rank(left).cmp(&dynamic_block_order_role_rank(right))
        })
        .then_with(|| dynamic_block_column(left).cmp(&dynamic_block_column(right)))
        .then_with(|| {
            let left_y = dynamic_block_normalized_bbox(left).map(|bbox| bbox[1]);
            let right_y = dynamic_block_normalized_bbox(right).map(|bbox| bbox[1]);
            left_y.partial_cmp(&right_y).unwrap_or(Ordering::Equal)
        })
        .then_with(|| {
            let left_x = dynamic_block_normalized_bbox(left).map(|bbox| bbox[0]);
            let right_x = dynamic_block_normalized_bbox(right).map(|bbox| bbox[0]);
            left_x.partial_cmp(&right_x).unwrap_or(Ordering::Equal)
        })
        .then_with(|| {
            left.get("_epic8OriginalOrder")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .cmp(
                    &right
                        .get("_epic8OriginalOrder")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )
        })
}

fn find_question_word(text: &str) -> Option<(usize, usize)> {
    let lower = text.to_lowercase();
    if let Some(index) = lower.find("questions") {
        Some((index, "questions".len()))
    } else {
        lower
            .find("question")
            .map(|index| (index, "question".len()))
    }
}

fn parse_number_after(text: &str, start: usize) -> Option<(u32, usize)> {
    let mut index = start.min(text.len());
    while let Some(ch) = text[index..].chars().next() {
        if ch.is_whitespace() || matches!(ch, ':' | '#' | '.' | ')') {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    let number_start = index;
    while let Some(ch) = text[index..].chars().next() {
        if ch.is_ascii_digit() {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    if index == number_start {
        return None;
    }
    text[number_start..index]
        .parse::<u32>()
        .ok()
        .map(|value| (value, index))
}

fn detect_dynamic_question_range(text: &str) -> Option<(u32, u32)> {
    let (word_index, word_len) = find_question_word(text)?;
    let (start, mut index) = parse_number_after(text, word_index + word_len)?;
    while let Some(ch) = text[index..].chars().next() {
        if ch.is_whitespace() {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    if let Some(ch) = text[index..].chars().next() {
        if matches!(ch, '-' | '\u{2013}' | '\u{2014}') {
            if let Some((end, _)) = parse_number_after(text, index + ch.len_utf8()) {
                return Some((start, end));
            }
        }
    }
    Some((start, start))
}

pub(crate) fn is_dynamic_umbrella_question_range(text: &str) -> bool {
    let Some((start, end)) = detect_dynamic_question_range(text) else {
        return false;
    };
    end > start && has_dynamic_umbrella_question_context(text)
}

fn is_bare_dynamic_question_range_heading(text: &str) -> bool {
    let trimmed = text.trim_start().trim_start_matches('#').trim_start();
    if !is_dynamic_question_heading_text(trimmed) {
        return false;
    }
    let Some((start, end)) = detect_dynamic_question_range(trimmed) else {
        return false;
    };
    if end <= start {
        return false;
    }
    let heading = dynamic_question_heading(start, end)
        .to_lowercase()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let normalized = trimmed
        .to_lowercase()
        .replace(['\u{2013}', '\u{2014}'], "-")
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let suffix = normalized
        .strip_prefix(&heading)
        .unwrap_or(normalized.as_str())
        .trim_matches(|ch: char| matches!(ch, '.' | ':' | ';' | '-' | ' '));
    suffix.is_empty()
}

fn normalized_question_context(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_dynamic_umbrella_question_context(text: &str) -> bool {
    let lower = normalized_question_context(text);
    if !lower.contains("reading passage") {
        return false;
    }
    let based_on_passage = lower.contains("based on reading passage")
        || lower.contains("based on the reading passage");
    let passage_reference = lower.contains("refer to reading passage")
        || lower.contains("refer to the reading passage")
        || lower.contains("relate to reading passage")
        || lower.contains("relate to the reading passage");

    lower.contains("which are based on reading passage")
        || lower.contains("which are based on the reading passage")
        || lower.contains("which is based on reading passage")
        || lower.contains("which is based on the reading passage")
        || (based_on_passage && (lower.contains("below") || lower.contains("you should spend")))
        || (passage_reference && lower.contains("below"))
        || (lower.contains("you should spend") && lower.contains("about"))
}

fn nearby_dynamic_question_context(blocks: &[Value], index: usize) -> String {
    let start = index.saturating_sub(3);
    let end = (index + 4).min(blocks.len());
    normalized_question_context(
        &blocks[start..end]
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn has_later_dynamic_concrete_subgroup(
    blocks: &[Value],
    index: usize,
    start: u32,
    end: u32,
) -> bool {
    blocks.iter().skip(index + 1).any(|candidate| {
        let text = dynamic_block_text(candidate);
        let Some((candidate_start, candidate_end)) = detect_dynamic_question_heading_range(&text)
        else {
            return false;
        };
        candidate_end > candidate_start
            && candidate_start >= start
            && candidate_end <= end
            && (candidate_start, candidate_end) != (start, end)
            && !is_dynamic_umbrella_question_range(&text)
    })
}

fn is_dynamic_reading_passage_heading(text: &str) -> bool {
    text.trim_start()
        .to_uppercase()
        .starts_with("READING PASSAGE")
}

fn is_substantive_dynamic_passage_block(block: &Value) -> bool {
    let text = dynamic_block_text(block);
    if text.len() < 48 {
        return false;
    }
    dynamic_block_role(block) == "passage"
        || (!is_dynamic_question_block(block)
            && !is_dynamic_answer_block(block)
            && !is_dynamic_reading_passage_heading(&text))
}

fn has_opening_dynamic_question_range_position(blocks: &[Value], index: usize) -> bool {
    let Some(header_index) =
        blocks
            .iter()
            .take(index + 1)
            .enumerate()
            .rev()
            .find_map(|(candidate_index, block)| {
                let text = dynamic_block_text(block);
                if is_dynamic_reading_passage_heading(&text) {
                    Some(candidate_index)
                } else {
                    None
                }
            })
    else {
        return false;
    };
    if index.saturating_sub(header_index) > 4 {
        return false;
    }
    !blocks[header_index + 1..index]
        .iter()
        .any(is_substantive_dynamic_passage_block)
}

fn is_dynamic_umbrella_question_block(blocks: &[Value], index: usize) -> bool {
    let Some(block) = blocks.get(index) else {
        return false;
    };
    let text = dynamic_block_text(block);
    if is_dynamic_umbrella_question_range(&text) {
        return true;
    }
    if !is_bare_dynamic_question_range_heading(&text) {
        return false;
    }
    let Some((start, end)) = detect_dynamic_question_range(&text) else {
        return false;
    };
    let is_full_passage_span = end.saturating_sub(start) >= 9;
    let nearby = nearby_dynamic_question_context(blocks, index);
    let has_opening_position = has_opening_dynamic_question_range_position(blocks, index);
    let has_opening_context = is_full_passage_span
        && has_opening_position
        && (nearby.contains("reading passage")
            || (nearby.contains("you should spend") && nearby.contains("about")));
    has_opening_context || has_later_dynamic_concrete_subgroup(blocks, index, start, end)
}

fn is_known_dynamic_umbrella_block(block: &Value, umbrella_blocks: &[Value]) -> bool {
    let block_id = dynamic_block_id(block);
    if !block_id.is_empty() {
        return umbrella_blocks
            .iter()
            .any(|candidate| dynamic_block_id(candidate) == block_id);
    }
    umbrella_blocks.iter().any(|candidate| candidate == block)
}

fn is_dynamic_question_heading_text(text: &str) -> bool {
    let lower = text
        .trim_start()
        .trim_start_matches('#')
        .trim_start()
        .to_lowercase();
    lower.starts_with("questions ") || lower.starts_with("question ")
}

fn detect_dynamic_question_heading_range(text: &str) -> Option<(u32, u32)> {
    if is_dynamic_question_heading_text(text) {
        detect_dynamic_question_range(text)
    } else {
        None
    }
}

fn is_dynamic_question_block(block: &Value) -> bool {
    dynamic_block_role(block) == "question"
        || detect_dynamic_question_range(&dynamic_block_text(block)).is_some()
}

fn is_dynamic_answer_block(block: &Value) -> bool {
    let lower = dynamic_block_text(block).to_lowercase();
    dynamic_block_role(block) == "answer"
        || lower.starts_with("answers")
        || lower.contains("answer key")
}

fn detect_dynamic_group_kind(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if lower.contains("true") && lower.contains("false") && lower.contains("not given") {
        "true_false_not_given"
    } else if lower.contains("yes") && lower.contains("no") && lower.contains("not given") {
        "yes_no_not_given"
    } else if lower.contains("choose")
        && (lower.contains("two") || lower.contains("three"))
        && lower.contains("letter")
    {
        "multi_choice"
    } else if lower.contains("complete the table")
        || lower.contains("table below")
        || (lower.contains('|') && lower.contains("complete"))
    {
        "table_completion"
    } else if lower.contains("list of headings") || lower.contains("matching headings") {
        "heading_matching"
    } else if lower.contains("which paragraph contains")
        || lower.contains("matching information")
        || lower.contains("match each statement")
    {
        "matching_information"
    } else if lower.contains("classify")
        || lower.contains("classification")
        || lower.contains("according to")
    {
        "classification"
    } else if lower.contains("match") && lower.contains("letter") {
        "matching"
    } else if lower.contains("choose") && lower.contains("letter") {
        "single_choice"
    } else if lower.contains("complete the summary") {
        "summary_completion"
    } else if lower.contains("complete the sentence") || lower.contains("complete the sentences") {
        "sentence_completion"
    } else {
        "short_answer"
    }
}

fn dynamic_letter_options_for_text(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    if lower.contains(" a-g") || lower.contains("a-g") || lower.contains("list of headings") {
        ["A", "B", "C", "D", "E", "F", "G"]
            .iter()
            .map(|value| value.to_string())
            .collect()
    } else if lower.contains(" a-f") || lower.contains("a-f") {
        ["A", "B", "C", "D", "E", "F"]
            .iter()
            .map(|value| value.to_string())
            .collect()
    } else if lower.contains(" a-e") || lower.contains("a-e") {
        ["A", "B", "C", "D", "E"]
            .iter()
            .map(|value| value.to_string())
            .collect()
    } else {
        ["A", "B", "C", "D"]
            .iter()
            .map(|value| value.to_string())
            .collect()
    }
}

fn dynamic_selection_count(text: &str) -> Option<u32> {
    let lower = text.to_lowercase();
    if lower.contains("choose three") || lower.contains("three letters") {
        Some(3)
    } else if lower.contains("choose two") || lower.contains("two letters") {
        Some(2)
    } else {
        None
    }
}

fn dynamic_option_reuse_rule(kind: &str, text: &str) -> (bool, Option<String>) {
    let lower = text.to_lowercase();
    if lower.contains("may use any letter more than once")
        || lower.contains("may be used more than once")
        || lower.contains("use any letter more than once")
    {
        return (true, None);
    }
    if lower.contains("each option may be used once only")
        || lower.contains("use each letter once only")
        || lower.contains("each letter may be used once only")
    {
        return (false, None);
    }
    match kind {
        "classification" | "matching_information" => (
            true,
            Some("Option reuse was inferred from question type; source wording did not state it explicitly.".to_string()),
        ),
        "heading_matching" | "matching" | "single_choice" | "multi_choice" => (
            false,
            Some("Option reuse was inferred from question type; source wording did not state it explicitly.".to_string()),
        ),
        _ => (false, None),
    }
}

fn classify_dynamic_group(text: &str, block_ids: &[String]) -> GroupClassificationV1 {
    let kind = detect_dynamic_group_kind(text).to_string();
    let lower = text.to_lowercase();
    let mut warnings = Vec::new();
    let (allow_option_reuse, reuse_warning) = dynamic_option_reuse_rule(&kind, text);
    if let Some(warning) = reuse_warning {
        warnings.push(warning);
    }
    let interaction = match kind.as_str() {
        "true_false_not_given" => GroupInteractionClassificationV1 {
            r#type: "radio".to_string(),
            options: vec![
                "TRUE".to_string(),
                "FALSE".to_string(),
                "NOT GIVEN".to_string(),
            ],
            allow_option_reuse,
            min_selections: None,
            max_selections: None,
        },
        "yes_no_not_given" => GroupInteractionClassificationV1 {
            r#type: "radio".to_string(),
            options: vec!["YES".to_string(), "NO".to_string(), "NOT GIVEN".to_string()],
            allow_option_reuse,
            min_selections: None,
            max_selections: None,
        },
        "single_choice" => GroupInteractionClassificationV1 {
            r#type: "radio".to_string(),
            options: dynamic_letter_options_for_text(text),
            allow_option_reuse,
            min_selections: None,
            max_selections: None,
        },
        "multi_choice" => {
            let count = dynamic_selection_count(text).unwrap_or(2);
            GroupInteractionClassificationV1 {
                r#type: "checkbox".to_string(),
                options: dynamic_letter_options_for_text(text),
                allow_option_reuse,
                min_selections: Some(count),
                max_selections: Some(count),
            }
        }
        "heading_matching" | "matching" | "matching_information" | "classification" => {
            GroupInteractionClassificationV1 {
                r#type: "matching".to_string(),
                options: dynamic_letter_options_for_text(text),
                allow_option_reuse,
                min_selections: None,
                max_selections: None,
            }
        }
        _ => GroupInteractionClassificationV1 {
            r#type: "text".to_string(),
            options: Vec::new(),
            allow_option_reuse,
            min_selections: None,
            max_selections: None,
        },
    };
    let confidence = if warnings.is_empty() {
        0.82
    } else if lower.contains("may be used more than once")
        || lower.contains("use each letter once only")
        || lower.contains("choose two")
        || lower.contains("choose three")
    {
        0.78
    } else {
        0.68
    };
    GroupClassificationV1 {
        kind,
        interaction,
        confidence,
        warnings,
        evidence: block_ids.to_vec(),
    }
}

fn dynamic_question_heading(start: u32, end: u32) -> String {
    if start == end {
        format!("Questions {}", start)
    } else {
        format!("Questions {}-{}", start, end)
    }
}

fn normalized_answer_value(raw: &str) -> Value {
    let upper = raw.trim().to_uppercase();
    if matches!(
        upper.as_str(),
        "TRUE" | "FALSE" | "YES" | "NO" | "NOT GIVEN" | "A" | "B" | "C" | "D"
    ) {
        json!(upper)
    } else {
        json!(raw.trim())
    }
}

fn clean_answer_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| matches!(ch, '.' | ')' | '(' | ':' | ';' | ','))
        .to_string()
}

pub(crate) fn parse_dynamic_answer_text(text: &str) -> serde_json::Map<String, Value> {
    let normalized = text
        .chars()
        .map(|ch| {
            if matches!(ch, '\n' | '\r' | ';' | ',') {
                ' '
            } else {
                ch
            }
        })
        .collect::<String>();
    let tokens = normalized
        .split_whitespace()
        .map(clean_answer_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut answers = serde_json::Map::new();
    let mut index = 0;
    while index < tokens.len() {
        if let Ok(number) = tokens[index].parse::<u32>() {
            index += 1;
            let mut value_tokens = Vec::new();
            while index < tokens.len() && tokens[index].parse::<u32>().is_err() {
                value_tokens.push(tokens[index].clone());
                index += 1;
            }
            if !value_tokens.is_empty() {
                answers.insert(
                    number.to_string(),
                    normalized_answer_value(&value_tokens.join(" ")),
                );
            }
        } else {
            index += 1;
        }
    }
    answers
}

fn infer_dynamic_passage_title(job: &ImportJob, passage_blocks: &[Value]) -> String {
    passage_blocks
        .iter()
        .map(dynamic_block_text)
        .find(|text| !text.is_empty() && !text.to_uppercase().starts_with("READING PASSAGE"))
        .unwrap_or_else(|| job.title.clone())
}

pub(crate) fn make_dynamic_split_candidates(
    job_id: &str,
    job: &ImportJob,
    doc: Option<&Value>,
) -> Value {
    let blocks = dynamic_document_blocks(doc);
    if blocks.is_empty() {
        return split_candidates(job_id);
    }

    let first_question_index = blocks.iter().position(is_dynamic_question_block);
    let first_concrete_question_index = blocks.iter().enumerate().find_map(|(index, block)| {
        let text = dynamic_block_text(block);
        if detect_dynamic_question_heading_range(&text).is_some()
            && !is_dynamic_umbrella_question_block(&blocks, index)
        {
            Some(index)
        } else {
            None
        }
    });
    let first_answer_index = blocks.iter().position(is_dynamic_answer_block);
    let passage_blocks = match first_concrete_question_index {
        Some(first_concrete_question) => blocks
            .iter()
            .enumerate()
            .filter(|(index, block)| {
                *index < first_concrete_question
                    && !is_dynamic_question_block(block)
                    && !is_dynamic_answer_block(block)
                    && dynamic_block_role(block) != "ignore"
            })
            .map(|(_, block)| block.clone())
            .collect::<Vec<_>>(),
        None => blocks
            .iter()
            .filter(|block| {
                !is_dynamic_question_block(block)
                    && !is_dynamic_answer_block(block)
                    && dynamic_block_role(block) != "ignore"
            })
            .cloned()
            .collect::<Vec<_>>(),
    };
    let all_umbrella_blocks = blocks
        .iter()
        .enumerate()
        .filter(|(index, _)| is_dynamic_umbrella_question_block(&blocks, *index))
        .map(|(_, block)| block)
        .cloned()
        .collect::<Vec<_>>();
    let question_blocks = if let Some(first_question) = first_concrete_question_index {
        blocks[first_question..]
            .iter()
            .filter(|block| {
                dynamic_block_role(block) != "answer" && dynamic_block_role(block) != "ignore"
            })
            .cloned()
            .collect::<Vec<_>>()
    } else if !all_umbrella_blocks.is_empty() {
        all_umbrella_blocks.clone()
    } else if let Some(first_question) = first_question_index {
        blocks[first_question..]
            .iter()
            .filter(|block| {
                dynamic_block_role(block) != "answer" && dynamic_block_role(block) != "ignore"
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        blocks
            .iter()
            .filter(|block| is_dynamic_question_block(block))
            .cloned()
            .collect::<Vec<_>>()
    };
    let answer_blocks = blocks
        .iter()
        .filter(|block| is_dynamic_answer_block(block))
        .cloned()
        .collect::<Vec<_>>();

    let mut answer_map = serde_json::Map::new();
    for block in &answer_blocks {
        for (key, value) in parse_dynamic_answer_text(&dynamic_block_text(block)) {
            answer_map.insert(key, value);
        }
    }
    let mut answer_numbers = answer_map
        .keys()
        .filter_map(|key| key.parse::<u32>().ok())
        .collect::<Vec<_>>();
    answer_numbers.sort_unstable();

    let umbrella_ranges = all_umbrella_blocks
        .iter()
        .filter_map(|block| {
            let text = dynamic_block_text(block);
            detect_dynamic_question_range(&text).and_then(|(start, end)| {
                if end <= start {
                    return None;
                }
                Some(UmbrellaQuestionRangeV1 {
                    heading: dynamic_question_heading(start, end),
                    question_range: [start, end],
                    block_id: dynamic_block_id(block),
                    text,
                })
            })
        })
        .collect::<Vec<_>>();

    let mut group_candidates = Vec::new();
    for (index, block) in question_blocks.iter().enumerate() {
        let text = dynamic_block_text(block);
        if is_known_dynamic_umbrella_block(block, &all_umbrella_blocks) {
            continue;
        }
        let Some((start, end)) = detect_dynamic_question_heading_range(&text) else {
            continue;
        };
        let next_heading = question_blocks
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, candidate)| {
                detect_dynamic_question_heading_range(&dynamic_block_text(candidate)).is_some()
                    && !is_known_dynamic_umbrella_block(candidate, &all_umbrella_blocks)
            })
            .map(|(candidate_index, _)| candidate_index)
            .unwrap_or(question_blocks.len());
        let included = &question_blocks[index..next_heading];
        let combined = included
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join(" ");
        let block_ids = included.iter().map(dynamic_block_id).collect::<Vec<_>>();
        let classification = classify_dynamic_group(&combined, &block_ids);
        let section_evidence = split_section_evidence_for_blocks(included);
        let continuation_edges = split_continuation_edges_for_blocks(included);
        group_candidates.push(SplitGroupCandidateV1 {
            group_id: format!("group-{}", group_candidates.len() + 1),
            heading: dynamic_question_heading(start, end),
            question_range: [start, end],
            instruction_text: text,
            block_ids,
            kind_hint: classification.kind.clone(),
            confidence: classification.confidence,
            classification: Some(classification),
            section_evidence,
            continuation_edges,
            is_umbrella_range: None,
            requires_manual_question_import: None,
        });
    }

    if group_candidates.is_empty() && !umbrella_ranges.is_empty() {
        for umbrella in &umbrella_ranges {
            let [start, end] = umbrella.question_range;
            group_candidates.push(SplitGroupCandidateV1 {
                group_id: format!("group-{}", group_candidates.len() + 1),
                heading: dynamic_question_heading(start, end),
                question_range: [start, end],
                instruction_text: umbrella.text.clone(),
                block_ids: if umbrella.block_id.is_empty() {
                    Vec::new()
                } else {
                    vec![umbrella.block_id.clone()]
                },
                kind_hint: "short_answer".to_string(),
                confidence: 0.35,
                classification: Some(classify_dynamic_group(
                    &umbrella.text,
                    std::slice::from_ref(&umbrella.block_id),
                )),
                section_evidence: Vec::new(),
                continuation_edges: Vec::new(),
                is_umbrella_range: Some(true),
                requires_manual_question_import: Some(true),
            });
        }
    } else if group_candidates.is_empty() && !question_blocks.is_empty() {
        let start = answer_numbers.first().copied().unwrap_or(1);
        let end = answer_numbers.last().copied().unwrap_or(start);
        let combined = question_blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");
        let block_ids = question_blocks
            .iter()
            .map(dynamic_block_id)
            .collect::<Vec<_>>();
        let classification = classify_dynamic_group(&combined, &block_ids);
        group_candidates.push(SplitGroupCandidateV1 {
            group_id: "group-1".to_string(),
            heading: dynamic_question_heading(start, end),
            question_range: [start, end],
            instruction_text: combined,
            block_ids,
            kind_hint: classification.kind.clone(),
            confidence: classification.confidence.min(0.58),
            classification: Some(classification),
            section_evidence: split_section_evidence_for_blocks(&question_blocks),
            continuation_edges: split_continuation_edges_for_blocks(&question_blocks),
            is_umbrella_range: None,
            requires_manual_question_import: None,
        });
    }

    let fallback_passage_range = if let Some(first_question) = first_question_index {
        blocks[..first_question]
            .iter()
            .map(dynamic_block_id)
            .collect::<Vec<_>>()
    } else {
        blocks
            .iter()
            .take(3)
            .map(dynamic_block_id)
            .collect::<Vec<_>>()
    };
    let passage_range = if passage_blocks.is_empty() {
        fallback_passage_range
    } else {
        passage_blocks
            .iter()
            .map(dynamic_block_id)
            .collect::<Vec<_>>()
    };
    let mut issues = Vec::new();
    if group_candidates.is_empty() {
        issues.push("No question range heading detected; manual split required.".to_string());
    } else if group_candidates
        .iter()
        .any(|candidate| candidate.requires_manual_question_import == Some(true))
    {
        issues.push("Only umbrella question range detected; concrete question prompts must be imported or entered manually.".to_string());
    }
    if answer_map.is_empty() {
        issues.push("No answer key detected; answers must be entered manually.".to_string());
    }
    if let (Some(first_answer), Some(first_question)) = (first_answer_index, first_question_index) {
        if first_answer < first_question {
            issues.push(
                "Answer block appears before question block; verify split order.".to_string(),
            );
        }
    }

    SplitCandidatesV1 {
        job_id: job_id.to_string(),
        passage_candidates: vec![PassageCandidateV1 {
            range: passage_range,
            title: infer_dynamic_passage_title(job, &passage_blocks),
            category_hint: job.category.clone().unwrap_or_else(|| "P1".to_string()),
        }],
        question_group_candidates: group_candidates,
        answer_key_candidates: if answer_map.is_empty() {
            Vec::new()
        } else {
            vec![AnswerKeyCandidateV1 {
                source: answer_blocks
                    .iter()
                    .map(dynamic_block_id)
                    .collect::<Vec<_>>()
                    .join(","),
                answers: answer_map,
            }]
        },
        umbrella_question_ranges: umbrella_ranges,
        issues,
    }
    .to_value()
}

pub(crate) fn dynamic_interaction_for_kind(kind: &str) -> Value {
    match kind {
        "true_false_not_given" => {
            json!({"type": "radio", "options": ["TRUE", "FALSE", "NOT GIVEN"]})
        }
        "yes_no_not_given" => json!({"type": "radio", "options": ["YES", "NO", "NOT GIVEN"]}),
        "single_choice" => json!({"type": "radio", "options": ["A", "B", "C", "D"]}),
        "multi_choice" => {
            json!({"type": "checkbox", "options": ["A", "B", "C", "D", "E", "F"], "minSelections": 2, "maxSelections": 2})
        }
        "heading_matching" | "matching" | "matching_information" | "classification" => {
            json!({"type": "matching", "options": ["A", "B", "C", "D"], "allowOptionReuse": kind == "classification" || kind == "matching_information"})
        }
        _ => json!({"type": "text", "placeholder": "answer"}),
    }
}

pub(crate) fn dynamic_template_for_kind(kind: &str) -> &'static str {
    match kind {
        "true_false_not_given" => "tfng_list",
        "yes_no_not_given" => "ynng_list",
        "single_choice" => "single_choice_list",
        "multi_choice" => "multi_choice_checkbox",
        "heading_matching" => "heading_matching",
        "matching" => "matching_list",
        "matching_information" => "matching_information",
        "classification" => "classification",
        "table_completion" => "table_completion",
        "summary_completion" => "summary_text_completion",
        "sentence_completion" => "inline_text_completion",
        _ => "short_answer_list",
    }
}

fn dynamic_interaction_from_candidate(candidate: &Value, kind: &str) -> Value {
    candidate
        .pointer("/classification/interaction")
        .cloned()
        .unwrap_or_else(|| dynamic_interaction_for_kind(kind))
}

fn find_dynamic_number_marker(text: &str, number: u32, from: usize) -> Option<(usize, usize)> {
    let needle = number.to_string();
    let mut search = from.min(text.len());
    while let Some(relative) = text[search..].find(&needle) {
        let start = search + relative;
        let after_digits = start + needle.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .map(|ch| ch.is_whitespace() || matches!(ch, '(' | '['))
            .unwrap_or(true);
        if !before_ok {
            search = after_digits;
            continue;
        }
        if let Some(next) = text[after_digits..].chars().next() {
            if matches!(next, '-' | '\u{2013}' | '\u{2014}') {
                search = after_digits;
                continue;
            }
            if !(next.is_whitespace() || matches!(next, '.' | ')' | ':' | '、')) {
                search = after_digits;
                continue;
            }
        }
        let mut content_start = after_digits;
        if let Some(next) = text[content_start..].chars().next() {
            if matches!(next, '.' | ')' | ':' | '、') {
                content_start += next.len_utf8();
            }
        }
        while let Some(next) = text[content_start..].chars().next() {
            if next.is_whitespace() {
                content_start += next.len_utf8();
            } else {
                break;
            }
        }
        return Some((start, content_start));
    }
    None
}

fn find_dynamic_final_prompt_boundary(text: &str, from: usize) -> usize {
    let lower = text.to_lowercase();
    [" questions ", " answers", " answer key"]
        .iter()
        .filter_map(|marker| lower[from..].find(marker).map(|relative| from + relative))
        .min()
        .unwrap_or(text.len())
}

fn dynamic_prompt_for_question(
    group_text: &str,
    number: u32,
    fallback_heading: &str,
    range_end: u32,
) -> String {
    let normalized = collapse_whitespace(group_text);
    if let Some((_, content_start)) = find_dynamic_number_marker(&normalized, number, 0) {
        let boundary = if number < range_end {
            find_dynamic_number_marker(&normalized, number + 1, content_start)
                .map(|(next_start, _)| next_start)
                .unwrap_or(normalized.len())
        } else {
            find_dynamic_final_prompt_boundary(&normalized, content_start)
        };
        let prompt = normalized[content_start..boundary]
            .trim()
            .trim_end_matches([';', ','])
            .trim();
        if !prompt.is_empty() {
            return prompt.to_string();
        }
    }
    format!("{} item {}", fallback_heading, number)
}

fn dynamic_block_html(block: &Value) -> String {
    block
        .get("html")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            format!(
                "<p>{}</p>",
                html_escape(
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                )
            )
        })
}

fn dynamic_answer_map_from_split(split: &Value) -> serde_json::Map<String, Value> {
    let mut answers = serde_json::Map::new();
    for candidate in split
        .get("answerKeyCandidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(map) = candidate.get("answers").and_then(Value::as_object) {
            for (key, value) in map {
                answers.insert(key.clone(), value.clone());
            }
        }
    }
    answers
}

pub(crate) fn merge_answer_source_candidates(split: &mut Value, answer_candidates: Vec<Value>) {
    if answer_candidates.is_empty() {
        return;
    }
    if let Some(obj) = split.as_object_mut() {
        let has_any_answers = answer_candidates.iter().any(|candidate| {
            candidate
                .get("answers")
                .and_then(Value::as_object)
                .map(|answers| !answers.is_empty())
                .unwrap_or(false)
        });
        let candidates = obj
            .entry("answerKeyCandidates".to_string())
            .or_insert_with(|| json!([]));
        if !candidates.is_array() {
            *candidates = json!([]);
        }
        if let Some(items) = candidates.as_array_mut() {
            for candidate in answer_candidates {
                items.push(candidate);
            }
        }
        if has_any_answers {
            let issues = obj
                .get("issues")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|issue| {
                    issue.as_str()
                        != Some("No answer key detected; answers must be entered manually.")
                })
                .collect::<Vec<_>>();
            obj.insert("issues".to_string(), Value::Array(issues));
        }
    }
}

fn dynamic_range_from_candidate(candidate: &Value) -> (u32, u32) {
    let values = candidate
        .get("questionRange")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let start = values.first().and_then(Value::as_u64).unwrap_or(1) as u32;
    let end = values
        .get(1)
        .and_then(Value::as_u64)
        .unwrap_or(start as u64) as u32;
    (start, end)
}

pub(crate) fn make_dynamic_authoring_ir(
    job: &ImportJob,
    split: &Value,
    doc: Option<&Value>,
) -> Value {
    let exam_id = format!(
        "{}-{}-{}",
        job.category
            .clone()
            .unwrap_or_else(|| "P1".to_string())
            .to_lowercase(),
        job.frequency
            .clone()
            .unwrap_or_else(|| "medium".to_string()),
        &job.job_id[job.job_id.len().saturating_sub(8)..]
    );
    let blocks = dynamic_document_blocks(doc);
    let blocks_by_id = blocks
        .iter()
        .map(|block| (dynamic_block_id(block), block.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let answer_by_display = dynamic_answer_map_from_split(split);
    let first_passage = split
        .get("passageCandidates")
        .and_then(Value::as_array)
        .and_then(|items| items.first());
    let passage_source_ids = first_passage
        .and_then(|candidate| candidate.get("range"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let passage_html = passage_source_ids
        .iter()
        .filter_map(|block_id| blocks_by_id.get(block_id))
        .map(dynamic_block_html)
        .collect::<Vec<_>>()
        .join("\n");
    let passage_title = first_passage
        .and_then(|candidate| candidate.get("title"))
        .and_then(Value::as_str)
        .unwrap_or(&job.title)
        .to_string();

    let groups = split
        .get("questionGroupCandidates")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let kind = candidate.get("kindHint").and_then(Value::as_str).unwrap_or("short_answer");
            let heading = candidate.get("heading").and_then(Value::as_str).unwrap_or("Questions");
            let instruction_text = candidate.get("instructionText").and_then(Value::as_str).unwrap_or(heading);
            let requires_manual_question_import = candidate
                .get("requiresManualQuestionImport")
                .and_then(Value::as_bool)
                == Some(true);
            let block_ids = candidate
                .get("blockIds")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).map(ToString::to_string).collect::<Vec<_>>())
                .unwrap_or_default();
            let group_text = {
                let text = block_ids
                    .iter()
                    .filter_map(|block_id| blocks_by_id.get(block_id))
                    .map(dynamic_block_text)
                    .collect::<Vec<_>>()
                    .join(" ");
                if text.trim().is_empty() { instruction_text.to_string() } else { text }
            };
            let (start, end) = dynamic_range_from_candidate(candidate);
            let review_warnings = candidate
                .pointer("/classification/warnings")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let classification_evidence = candidate
                .pointer("/classification/evidence")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| block_ids.clone());
            let section_evidence = candidate
                .get("sectionEvidence")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|item| serde_json::from_value::<SplitSectionEvidenceV1>(item).ok())
                .collect::<Vec<_>>();
            let continuation_edges = candidate
                .get("continuationEdges")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|item| serde_json::from_value::<SplitContinuationEdgeV1>(item).ok())
                .collect::<Vec<_>>();
            let questions = (start..=end)
                .map(|number| {
                    let display = number.to_string();
                    let qid = format!("q{}", display);
                    QuestionDraftV1 {
                        id: qid,
                        display_number: display.clone(),
                        prompt: if requires_manual_question_import {
                            format!("Manual import required for question {}", number)
                        } else {
                            dynamic_prompt_for_question(&group_text, number, heading, end)
                        },
                        interaction: dynamic_interaction_from_candidate(candidate, kind),
                        answer: answer_by_display
                            .get(&number.to_string())
                            .cloned()
                            .unwrap_or_else(|| json!("")),
                        source_block_ids: block_ids.clone(),
                        confidence: candidate
                            .get("confidence")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.72),
                        verified: false,
                        requires_manual_question_import,
                    }
                })
                .collect::<Vec<_>>();
            let layout = if kind == "table_completion" {
                json!({"template": dynamic_template_for_kind(kind), "tableHeaders": ["Question", "Prompt", "Answer"]})
            } else {
                json!({"template": dynamic_template_for_kind(kind)})
            };
            let allow_option_reuse = candidate
                .pointer("/classification/interaction/allowOptionReuse")
                .and_then(Value::as_bool);
            QuestionGroupDraftV1 {
                group_id: candidate
                    .get("groupId")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("group-{}", index + 1)),
                kind: kind.to_string(),
                question_range: [start, end],
                instruction: vec![instruction_text.to_string()],
                questions,
                layout,
                review_warnings,
                classification_evidence,
                section_evidence,
                continuation_edges,
                allow_option_reuse,
                source_block_ids: block_ids,
                confidence: candidate
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.72),
                verified: false,
                is_umbrella_range: candidate
                    .get("isUmbrellaRange")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                requires_manual_question_import,
            }
        })
        .collect::<Vec<_>>();

    let answer_key = {
        let mut map = serde_json::Map::new();
        for group in &groups {
            for question in &group.questions {
                map.insert(question.id.clone(), question.answer.clone());
            }
        }
        map
    };
    let question_order = groups
        .iter()
        .flat_map(|group| group.questions.iter())
        .map(|question| question.id.clone())
        .collect::<Vec<_>>();
    let mut display_map = serde_json::Map::new();
    for group in &groups {
        for question in &group.questions {
            display_map.insert(question.id.clone(), json!(question.display_number));
        }
    }
    let split_issues = split
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let question_umbrella_ranges = split
        .get("umbrellaQuestionRanges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| serde_json::from_value::<UmbrellaQuestionRangeV1>(item).ok())
        .collect::<Vec<_>>();

    ReadingAuthoringIrV1 {
        schema_version: "ReadingAuthoringIRV1".to_string(),
        job_id: job.job_id.clone(),
        exam: ExamMetaDraftV1 {
            exam_id,
            title: job.title.clone(),
            category: job.category.clone().unwrap_or_else(|| "P1".to_string()),
            frequency: job
                .frequency
                .clone()
                .unwrap_or_else(|| "medium".to_string()),
            tags: job.tags.clone(),
            source_files: job
                .source_files
                .iter()
                .map(|source| AuthoringSourceFileV1 {
                    file_id: source.file_id.clone(),
                    original_name: source.original_name.clone(),
                    stored_name: source.stored_name.clone(),
                    file_type: source.file_type.clone(),
                    sha256: source.sha256.clone(),
                    size_bytes: source.size_bytes,
                    role: source.role.clone(),
                    imported_at: source.imported_at.to_rfc3339(),
                })
                .collect(),
        },
        passage: PassageDraftV1 {
            title: passage_title,
            html_blocks: vec![PassageHtmlBlockV1 {
                block_id: "passage-main".to_string(),
                html: if passage_html.trim().is_empty() {
                    format!("<h2>{}</h2>", html_escape(&job.title))
                } else {
                    passage_html
                },
            }],
            source_block_ids: passage_source_ids,
            question_umbrella_ranges,
        },
        groups,
        answer_key,
        question_order,
        question_display_map: display_map,
        audit: AuthoringAuditV1 {
            llm_used: false,
            human_verified: false,
            issues: split_issues,
            revision: 1,
            updated_at: Utc::now().to_rfc3339(),
        },
    }
    .to_value()
}
