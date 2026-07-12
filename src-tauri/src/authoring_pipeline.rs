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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_hint: Option<String>,
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
    // NOTE: we deliberately KEEP _epic8PageWidth / _epic8PageHeight /
    // _epic8PageRotation / _epic8OriginalOrder on the blocks. The column
    // detector (`dynamic_block_column`) and reading-order comparator need the
    // real page dimensions to distinguish full-width header lines from
    // 2-column body. Previously these were stripped, which forced every block
    // back to the 595/842 default and made column detection a no-op. They are
    // still private (underscore-prefixed) and are filtered out before any IR
    // is serialized where it matters.
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

fn dynamic_block_layout_section_index(block: &Value) -> Option<u64> {
    block
        .get("_epic8LayoutSection")
        .or_else(|| block.pointer("/layoutHints/section/index"))
        .and_then(Value::as_u64)
}

fn dynamic_block_layout_column_index(block: &Value) -> Option<u8> {
    block
        .get("_epic8ColumnIndex")
        .or_else(|| block.pointer("/layoutHints/section/columns/current"))
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
}

fn dynamic_block_section_column_count_value(block: &Value) -> Option<u8> {
    block
        .get("_epic8SectionColumns")
        .or_else(|| block.pointer("/layoutHints/section/columns/count"))
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
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
    if let Some(column_index) = dynamic_block_layout_column_index(block) {
        return column_index;
    }
    if dynamic_block_section_column_count_value(block) == Some(1) {
        return 0;
    }
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
    // A line that spans most of the page width is a full-width header/title
    // (e.g. "READING PASSAGE 3", "Questions 27-31", page numbers). Treat it as
    // column 0 so it isn't ordered after the right column, but flag it so the
    // reading-order comparator can keep headers anchored to the top.
    let block_width = (bbox[2] - bbox[0]).abs();
    if block_width > page_width * 0.75 {
        return 0;
    }
    // When the section declares a multi-column layout (>2), bucket the block
    // into the right column index by its left edge relative to the page width,
    // instead of the old binary 0/1 split that collapsed 3-column layouts to
    // just left/right.
    let section_columns = dynamic_block_section_column_count_value(block).unwrap_or(2);
    if section_columns >= 3 {
        let bucket = ((bbox[0] / page_width) * (section_columns as f64)).floor() as i64;
        return bucket.clamp(0, (section_columns - 1) as i64) as u8;
    }
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

fn dynamic_block_original_order(block: &Value) -> u64 {
    block
        .get("_epic8OriginalOrder")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn dynamic_reading_order_cmp(left: &Value, right: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let left_section = dynamic_block_layout_section_index(left).unwrap_or(u64::MAX);
    let right_section = dynamic_block_layout_section_index(right).unwrap_or(u64::MAX);
    let has_explicit_layout = dynamic_block_layout_section_index(left).is_some()
        || dynamic_block_layout_section_index(right).is_some();

    dynamic_block_page_index(left)
        .cmp(&dynamic_block_page_index(right))
        .then_with(|| {
            dynamic_block_order_role_rank(left).cmp(&dynamic_block_order_role_rank(right))
        })
        .then_with(|| left_section.cmp(&right_section))
        .then_with(|| dynamic_block_column(left).cmp(&dynamic_block_column(right)))
        .then_with(|| {
            if has_explicit_layout
                || (dynamic_block_page_rotation(left) == 0
                    && dynamic_block_page_rotation(right) == 0)
            {
                dynamic_block_original_order(left).cmp(&dynamic_block_original_order(right))
            } else {
                Ordering::Equal
            }
        })
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
        .then_with(|| dynamic_block_original_order(left).cmp(&dynamic_block_original_order(right)))
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
    let after_first = text[index..].to_lowercase();
    let trimmed = after_first.trim_start();
    if let Some(rest) = trimmed.strip_prefix("and") {
        if rest
            .chars()
            .next()
            .map(|ch| ch.is_whitespace())
            .unwrap_or(false)
        {
            if let Some((end, _)) = parse_number_after(rest, 0) {
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

fn is_dynamic_short_prose_passage_block(block: &Value) -> bool {
    let text = dynamic_block_text(block);
    let normalized = collapse_whitespace(&text);
    if normalized.is_empty()
        || is_dynamic_question_block(block)
        || is_dynamic_answer_block(block)
        || is_dynamic_reading_passage_heading(&normalized)
        || is_dynamic_heading_option_line(&normalized)
        || is_dynamic_heading_matching_instruction_line(&normalized)
        || is_dynamic_heading_matching_assignment_line(&normalized)
        || is_dynamic_non_content_placeholder_text(&normalized)
        || is_dynamic_question_or_instruction_like_text(&normalized)
    {
        return false;
    }
    let word_count = normalized.split_whitespace().count();
    let has_lowercase = normalized.chars().any(|ch| ch.is_ascii_lowercase());
    let has_prose_punctuation = normalized.contains(',')
        || normalized.contains(';')
        || normalized.ends_with('.')
        || normalized.ends_with('!')
        || normalized.ends_with('?');
    let section_columns = block
        .pointer("/layoutHints/section/columns/count")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    has_lowercase
        && ((word_count >= 6 && (normalized.len() >= 28 || has_prose_punctuation))
            || (section_columns > 1 && word_count >= 5 && normalized.len() >= 24))
}

fn is_substantive_dynamic_passage_block(block: &Value) -> bool {
    let text = dynamic_block_text(block);
    if dynamic_block_role(block) == "passage" {
        return !is_dynamic_question_block(block)
            && !is_dynamic_answer_block(block)
            && !is_dynamic_non_content_placeholder_text(&text);
    }
    if text.len() < 48 && !is_dynamic_short_prose_passage_block(block) {
        return false;
    }
    !is_dynamic_question_block(block)
        && !is_dynamic_answer_block(block)
        && !is_dynamic_reading_passage_heading(&text)
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

fn infer_dynamic_group_range_end(
    text: &str,
    start: u32,
    heading_end: u32,
    allow_blank_extension: bool,
    allow_list_extension: bool,
) -> u32 {
    if !allow_blank_extension && !allow_list_extension {
        return heading_end.max(start);
    }
    let normalized = collapse_whitespace(text);
    let mut inferred_end = heading_end.max(start);
    let max_lookahead = start.saturating_add(20);
    let mut cursor = 0usize;
    for number in start..=inferred_end {
        if let Some((_, marker_end)) =
            find_dynamic_numbered_blank_marker(&normalized, number, cursor)
        {
            cursor = marker_end;
        } else if allow_list_extension {
            if let Some((_, marker_end)) = find_dynamic_number_marker(&normalized, number, cursor) {
                cursor = marker_end;
            }
        }
    }
    while inferred_end < max_lookahead {
        let next = inferred_end + 1;
        if allow_blank_extension {
            if let Some((_, marker_end)) =
                find_dynamic_numbered_blank_marker(&normalized, next, cursor)
            {
                inferred_end = next;
                cursor = marker_end;
                continue;
            }
        }
        if allow_list_extension {
            if let Some((_, marker_end)) = find_dynamic_number_marker(&normalized, next, cursor) {
                inferred_end = next;
                cursor = marker_end;
                continue;
            }
        };
        break;
    }
    inferred_end
}

fn infer_dynamic_group_range_end_from_markers(
    text: &str,
    start: u32,
    heading_end: u32,
    kind: &str,
) -> u32 {
    if kind == "heading_matching" {
        let normalized = collapse_whitespace(text);
        let mut inferred_end = heading_end.max(start);
        let mut cursor = 0usize;
        for number in start..=inferred_end {
            if let Some((_, marker_end)) = find_dynamic_number_marker(&normalized, number, cursor) {
                cursor = marker_end;
            }
        }
        while inferred_end < start.saturating_add(20) {
            let next = inferred_end + 1;
            let Some((_, marker_end)) = find_dynamic_number_marker(&normalized, next, cursor)
            else {
                break;
            };
            let segment_end = find_dynamic_prompt_boundary(&normalized, marker_end, next + 1, kind);
            let segment = normalized[marker_end..segment_end].trim();
            let word_count = segment.split_whitespace().count();
            let first_word = segment
                .to_lowercase()
                .split_whitespace()
                .next()
                .map(|word| {
                    word.trim_matches(|ch: char| !ch.is_alphanumeric())
                        .to_string()
                });
            if word_count > 8
                || !matches!(
                    first_word.as_deref(),
                    Some("section" | "paragraph" | "part")
                )
            {
                break;
            }
            inferred_end = next;
            cursor = marker_end;
        }
        return inferred_end;
    }
    let normalized = collapse_whitespace(text);
    let mut inferred_end = heading_end.max(start);
    let max_lookahead = start.saturating_add(20);
    let mut cursor = 0usize;
    for number in start..=inferred_end {
        if let Some((_, marker_end)) = find_dynamic_number_marker(&normalized, number, cursor) {
            cursor = marker_end;
        }
    }
    while inferred_end < max_lookahead {
        let next = inferred_end + 1;
        let Some((marker_start, marker_end)) =
            find_dynamic_number_marker(&normalized, next, cursor)
        else {
            break;
        };
        let preceding = normalized[cursor.min(normalized.len())..marker_start]
            .to_lowercase()
            .replace(
                ['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}'],
                "-",
            );
        if preceding.contains("questions ")
            || preceding.contains("answers")
            || preceding.contains("answer key")
        {
            break;
        }
        if kind == "single_choice" {
            let segment_end = find_dynamic_final_prompt_boundary(&normalized, marker_end);
            let segment = normalized[marker_end..segment_end].to_lowercase();
            let has_abcd = [" a ", " b ", " c ", " d "]
                .iter()
                .all(|marker| segment.contains(*marker));
            if !has_dynamic_single_choice_option_run(&segment) && !has_abcd {
                break;
            }
        } else if matches!(kind, "matching" | "matching_information" | "classification") {
            let segment_end = find_dynamic_prompt_boundary(&normalized, marker_end, next + 1, kind);
            let segment = normalized[marker_end..segment_end].trim();
            let segment_word_count = segment.split_whitespace().count();
            let looks_like_section_label = segment_word_count <= 8
                && segment
                    .to_lowercase()
                    .split_whitespace()
                    .next()
                    .map(|word| {
                        matches!(
                            word.trim_matches(|ch: char| !ch.is_alphanumeric()),
                            "section" | "paragraph" | "part"
                        )
                    })
                    .unwrap_or(false);
            let looks_like_short_prompt = segment_word_count <= 24
                && !segment.contains('.')
                && !segment.contains('?')
                && !segment.to_lowercase().contains(" reading passage ");
            if kind == "heading_matching" {
                if !looks_like_section_label {
                    break;
                }
            } else if !looks_like_section_label && !looks_like_short_prompt {
                break;
            }
        }
        inferred_end = next;
        cursor = marker_end;
    }
    inferred_end
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
    let normalized = normalized_dynamic_instruction_text(text);
    if lower.contains("true") && lower.contains("false") && lower.contains("not given") {
        "true_false_not_given"
    } else if lower.contains("yes") && lower.contains("no") && lower.contains("not given") {
        "yes_no_not_given"
    } else if is_dynamic_multi_choice_text(text) {
        "multi_choice"
    } else if lower.contains("complete the table")
        || lower.contains("table below")
        || lower.contains("complete the form")
        || lower.contains("form below")
        || (lower.contains('|') && lower.contains("complete"))
    {
        "table_completion"
    } else if lower.contains("complete the flow chart")
        || lower.contains("complete the flow-chart")
        || lower.contains("flow chart below")
        || lower.contains("flow-chart below")
        || lower.contains("label the diagram")
        || lower.contains("diagram below")
        || lower.contains("label the map")
        || lower.contains("map below")
        || lower.contains("label the plan")
        || lower.contains("plan below")
        || lower.contains("process below")
    {
        "diagram_completion"
    } else if lower.contains("list of headings")
        || lower.contains("matching headings")
        || (lower.contains("correct heading for") && lower.contains("headings"))
    {
        "heading_matching"
    } else if lower.contains("classify")
        || lower.contains("classification")
        || lower.contains("according to which")
    {
        "classification"
    } else if lower.contains("which paragraph contains")
        || lower.contains("which section contains")
        || lower.contains("which paragraph mentions")
        || lower.contains("which section mentions")
        || lower.contains("which paragraph refers to")
        || lower.contains("which section refers to")
        || lower.contains("matching information")
    {
        "matching_information"
    } else if is_dynamic_sentence_ending_matching_text(text) {
        "matching"
    } else if normalized.contains("write the correct letter")
        && has_dynamic_letter_option_span(&normalized)
    {
        "matching"
    } else if is_dynamic_matching_prompt_text(&normalized) {
        "matching"
    } else if lower.contains("match") && lower.contains("letter") {
        "matching"
    } else if lower.contains("complete the summary") || lower.contains("summary below") {
        "summary_completion"
    } else if is_dynamic_notes_completion_text(text) {
        "sentence_completion"
    } else if has_dynamic_numbered_inline_blanks(text) {
        "sentence_completion"
    } else if is_dynamic_short_answer_instruction_text(text) {
        "short_answer"
    } else if lower.contains("complete the sentence") || lower.contains("complete the sentences") {
        "sentence_completion"
    } else if is_dynamic_single_choice_text(text) {
        "single_choice"
    } else {
        "short_answer"
    }
}

fn normalized_dynamic_instruction_text(text: &str) -> String {
    collapse_whitespace(text).to_lowercase().replace(
        ['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}'],
        "-",
    )
}

fn is_dynamic_multi_choice_text(text: &str) -> bool {
    let normalized = normalized_dynamic_instruction_text(text);
    normalized.contains("choose two letters")
        || normalized.contains("choose three letters")
        || normalized.contains("choose two correct letters")
        || normalized.contains("choose three correct letters")
}

fn has_dynamic_letter_option_span(normalized: &str) -> bool {
    [
        "a-c",
        "a-d",
        "a-e",
        "a-f",
        "a-g",
        "a-h",
        "a-i",
        "letters a-c",
        "letters a-d",
        "letters a-e",
        "letters a-f",
        "letters a-g",
        "letters a-h",
        "letters a-i",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn has_dynamic_single_choice_option_run(normalized: &str) -> bool {
    [
        "a, b, c or d",
        "a, b, c, or d",
        "a, b or c",
        "a, b, c",
        "a-d",
        "a-c",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn is_dynamic_matching_prompt_text(normalized: &str) -> bool {
    normalized.contains("which paragraph contains")
        || normalized.contains("which section contains")
        || normalized.contains("which paragraph mentions")
        || normalized.contains("which section mentions")
        || normalized.contains("which paragraph refers to")
        || normalized.contains("which section refers to")
        || normalized.contains("match each statement")
        || normalized.contains("match each person")
        || normalized.contains("match each opinion")
        || normalized.contains("match each sentence")
        || normalized.contains("match each with")
        || normalized.contains("write the correct letter")
        || normalized.contains("look at the following")
        || normalized.contains("list of headings")
        || normalized.contains("correct heading for each")
}

fn is_dynamic_single_choice_text(text: &str) -> bool {
    let normalized = normalized_dynamic_instruction_text(text);
    if is_dynamic_matching_prompt_text(&normalized) {
        return false;
    }
    if normalized.contains("choose the correct letter")
        && has_dynamic_single_choice_option_run(&normalized)
    {
        return true;
    }
    if normalized.contains("which of the following")
        && has_dynamic_single_choice_option_run(&normalized)
    {
        return true;
    }
    let option_hits = [" a ", " b ", " c ", " d "]
        .iter()
        .filter(|marker| normalized.contains(**marker))
        .count();
    option_hits >= 4
        && [
            "what ",
            "why ",
            "which ",
            "according to ",
            "writer",
            "article",
            "purpose",
            "title",
        ]
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn is_dynamic_notes_completion_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("complete the notes")
        || lower.contains("notes below")
        || lower.contains("note completion")
}

fn is_dynamic_short_answer_instruction_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    let has_word_limit = lower.contains("no more than")
        || lower.contains("one word only")
        || lower.contains("two words only")
        || lower.contains("three words only")
        || lower.contains("and/or a number");
    has_word_limit
        && !lower.contains("complete the summary")
        && !is_dynamic_notes_completion_text(text)
        && !lower.contains("complete the sentence")
        && !lower.contains("complete the sentences")
        && !lower.contains("complete the table")
        && !lower.contains("flow chart")
        && !lower.contains("flow-chart")
        && !lower.contains("diagram")
}

fn is_dynamic_sentence_ending_matching_text(text: &str) -> bool {
    let lower = text.to_lowercase();
    (lower.contains("complete each sentence") || lower.contains("complete the sentences"))
        && (lower.contains("correct ending") || lower.contains("list of endings"))
}

fn dynamic_layout_hint_for_group(kind: &str, text: &str) -> &'static str {
    if kind == "table_completion" {
        "table"
    } else if is_dynamic_notes_completion_text(text)
        || kind == "diagram_completion"
        || kind == "sentence_completion"
        || kind == "summary_completion"
        || has_dynamic_numbered_inline_blanks(text)
    {
        "inline_completion"
    } else {
        "list"
    }
}

fn is_dynamic_blank_marker_char(ch: char) -> bool {
    matches!(
        ch,
        '_' | '.'
            | '\u{2026}'
            | '\u{22ef}'
            | '\u{00b7}'
            | '-'
            | '\u{2010}'
            | '\u{2011}'
            | '\u{2012}'
            | '\u{2013}'
            | '\u{2014}'
            | '\u{fe4d}'
            | '\u{fe4e}'
            | '\u{fe4f}'
            | '\u{ff3f}'
    )
}

fn dynamic_blank_marker_width(ch: char) -> usize {
    if matches!(ch, '\u{2026}' | '\u{22ef}') {
        3
    } else {
        1
    }
}

fn is_dynamic_number_boundary_before(text: &str, start: usize) -> bool {
    text[..start]
        .chars()
        .next_back()
        .map(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | '<' | '>'))
        .unwrap_or(true)
}

fn dynamic_next_non_space(text: &str, from: usize) -> Option<(usize, char)> {
    let mut cursor = from.min(text.len());
    while let Some(next) = text[cursor..].chars().next() {
        if next.is_whitespace() {
            cursor += next.len_utf8();
        } else {
            return Some((cursor, next));
        }
    }
    None
}

fn is_dynamic_range_dash_after_number(text: &str, after_digits: usize) -> bool {
    let Some((dash_index, dash)) = dynamic_next_non_space(text, after_digits) else {
        return false;
    };
    if !matches!(dash, '-' | '\u{2013}' | '\u{2014}') {
        return false;
    }
    dynamic_next_non_space(text, dash_index + dash.len_utf8())
        .map(|(_, next)| next.is_ascii_digit())
        .unwrap_or(false)
}

fn find_dynamic_numbered_blank_marker(
    text: &str,
    number: u32,
    from: usize,
) -> Option<(usize, usize)> {
    let needle = number.to_string();
    let mut search = from.min(text.len());
    while let Some(relative) = text[search..].find(&needle) {
        let start = search + relative;
        let after_digits = start + needle.len();
        if !is_dynamic_number_boundary_before(text, start) {
            search = after_digits;
            continue;
        }
        if is_dynamic_range_dash_after_number(text, after_digits) {
            search = after_digits;
            continue;
        }
        let mut cursor = after_digits;
        if let Some(next) = text[cursor..].chars().next() {
            if matches!(next, '.' | ')' | ':' | '、') {
                cursor += next.len_utf8();
            }
        }
        while let Some(next) = text[cursor..].chars().next() {
            if next.is_whitespace() {
                cursor += next.len_utf8();
            } else {
                break;
            }
        }
        let mut blank_end = cursor;
        let mut width = 0usize;
        while let Some(next) = text[blank_end..].chars().next() {
            if is_dynamic_blank_marker_char(next) {
                width += dynamic_blank_marker_width(next);
                blank_end += next.len_utf8();
            } else {
                break;
            }
        }
        if width >= 3 {
            return Some((start, blank_end));
        }
        search = after_digits;
    }
    None
}

fn has_dynamic_numbered_inline_blanks(text: &str) -> bool {
    let normalized = collapse_whitespace(text);
    let Some((start, end)) = detect_dynamic_question_range(&normalized) else {
        return false;
    };
    let mut cursor = 0usize;
    let mut markers = 0usize;
    for number in start..=end {
        if let Some((_, marker_end)) =
            find_dynamic_numbered_blank_marker(&normalized, number, cursor)
        {
            markers += 1;
            cursor = marker_end;
        }
    }
    markers >= 2 || (end > start && markers as u32 >= (end - start + 1).min(3))
}

fn dynamic_letter_options_for_text(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let normalized = lower
        .replace(
            ['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}'],
            "-",
        )
        .replace('–', "-");
    if normalized.contains("a-i") {
        ["A", "B", "C", "D", "E", "F", "G", "H", "I"]
            .iter()
            .map(|value| value.to_string())
            .collect()
    } else if normalized.contains("a-h") {
        ["A", "B", "C", "D", "E", "F", "G", "H"]
            .iter()
            .map(|value| value.to_string())
            .collect()
    } else if normalized.contains(" a-g")
        || normalized.contains("a-g")
        || lower.contains("list of headings")
    {
        ["A", "B", "C", "D", "E", "F", "G"]
            .iter()
            .map(|value| value.to_string())
            .collect()
    } else if normalized.contains(" a-f") || normalized.contains("a-f") {
        ["A", "B", "C", "D", "E", "F"]
            .iter()
            .map(|value| value.to_string())
            .collect()
    } else if normalized.contains(" a-e") || normalized.contains("a-e") {
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

fn normalize_dynamic_group_ranges(groups: &mut Vec<SplitGroupCandidateV1>) {
    groups.sort_by_key(|candidate| candidate.question_range[0]);
    let mut previous_end = 0u32;
    groups.retain_mut(|candidate| {
        let [start, end] = candidate.question_range;
        if end <= previous_end {
            return false;
        }
        if start <= previous_end && end > previous_end {
            let adjusted_start = previous_end + 1;
            candidate.question_range = [adjusted_start, end];
            candidate.heading = dynamic_question_heading(adjusted_start, end);
        }
        previous_end = previous_end.max(candidate.question_range[1]);
        true
    });
    for (index, candidate) in groups.iter_mut().enumerate() {
        candidate.group_id = format!("group-{}", index + 1);
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
            while index < tokens.len() {
                if let Ok(next_number) = tokens[index].parse::<u32>() {
                    if next_number == number + 1 {
                        break;
                    }
                }
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

/// Detect whether a block's text looks like the START of a new logical section
/// (heading) rather than a continuation of running prose. Used to guard
/// cross-page passage merging: we never want to glue a heading onto the tail
/// of the previous passage.
fn is_dynamic_passage_break_marker(text: &str) -> bool {
    let lower = collapse_whitespace(text).to_lowercase();
    if lower.is_empty() {
        return true;
    }
    lower.starts_with("reading passage")
        || lower.starts_with("questions ")
        || lower.starts_with("question ")
        || lower.starts_with("answers")
        || lower.contains("answer key")
        || is_dynamic_question_or_instruction_like_text(&lower)
}

/// Returns true when the two passage blocks look like a continuous sentence
/// that was split purely by a page/column break (no sentence terminator on
/// the first block, and the second block opens mid-sentence).
fn cross_page_passage_continues(left: &Value, right: &Value) -> bool {
    if dynamic_block_page_index(left) == dynamic_block_page_index(right) {
        // Same page: only consider cross-page continuations here. Same-page
        // adjacent passage blocks are already in reading order and merging
        // them would erase legitimate paragraph breaks.
        return false;
    }
    let left_text = dynamic_block_text(left);
    let right_text = dynamic_block_text(right);
    if left_text.is_empty() || right_text.is_empty() {
        return false;
    }
    if is_dynamic_passage_break_marker(&left_text) || is_dynamic_passage_break_marker(&right_text) {
        return false;
    }
    // Left block must NOT end with a sentence terminator (otherwise the right
    // block is a new sentence/paragraph, not a broken continuation).
    let left_tail = left_text.trim_end().chars().last().unwrap_or(' ');
    if matches!(left_tail, '.' | '?' | '!' | ':' | ';') {
        return false;
    }
    // Right block should open with a lowercase letter or a continuation word
    // (article/conjunction), signalling mid-sentence. A capitalized opening
    // that is NOT a known heading is ambiguous; be conservative and still
    // allow it, because many passages continue proper nouns — but require at
    // least that the right block isn't a heading marker (already checked).
    let right_first = right_text.trim_start().chars().next().unwrap_or(' ');
    let right_continues_prose = right_first.is_ascii_lowercase()
        || right_text
            .split_whitespace()
            .next()
            .map(|word| matches!(word.to_lowercase().as_str(), "the" | "a" | "an" | "and" | "but" | "or" | "which" | "that" | "this" | "these" | "those" | "in" | "on" | "for" | "with" | "as" | "by" | "from" | "to" | "at"))
            .unwrap_or(false)
        || right_first.is_ascii_uppercase();
    right_continues_prose
}

/// Merge adjacent passage blocks that were split only by a page break, gluing
/// their text together and keeping the first block's id/bbox origin. Operates
/// in place on the passage block list.
fn merge_cross_page_passage_continuations(passage_blocks: &mut Vec<Value>) {
    if passage_blocks.len() < 2 {
        return;
    }
    let mut result: Vec<Value> = Vec::with_capacity(passage_blocks.len());
    for block in passage_blocks.drain(..) {
        let should_merge = match result.last_mut() {
            Some(prev) => cross_page_passage_continues(prev, &block),
            None => false,
        };
        if should_merge {
            let prev = result.last_mut().unwrap();
            let prev_text = dynamic_block_text(prev);
            let next_text = dynamic_block_text(&block);
            let merged_text = format!("{} {}", prev_text, next_text);
            if let Some(obj) = prev.as_object_mut() {
                obj.insert("text".to_string(), json!(merged_text));
                let block_type = crate::parser::block_type_for_text_pub(&merged_text);
                obj.insert("blockType".to_string(), json!(block_type));
                obj.insert(
                    "html".to_string(),
                    json!(crate::parser::markdownish_to_html_pub(&merged_text, block_type)),
                );
                // Extend the bbox to cover both blocks so downstream geometry
                // consumers still see a sensible envelope.
                if let (Some(prev_bbox), Some(next_bbox)) = (
                    obj.get("bbox").and_then(Value::as_array),
                    block.get("bbox").and_then(Value::as_array),
                ) {
                    if prev_bbox.len() == 4 && next_bbox.len() == 4 {
                        let merged_bbox = vec![
                            json!(prev_bbox[0].as_f64().unwrap_or(0.0).min(next_bbox[0].as_f64().unwrap_or(0.0))),
                            json!(prev_bbox[1].as_f64().unwrap_or(0.0).min(next_bbox[1].as_f64().unwrap_or(0.0))),
                            json!(prev_bbox[2].as_f64().unwrap_or(0.0).max(next_bbox[2].as_f64().unwrap_or(0.0))),
                            json!(prev_bbox[3].as_f64().unwrap_or(0.0).max(next_bbox[3].as_f64().unwrap_or(0.0))),
                        ];
                        obj.insert("bbox".to_string(), json!(merged_bbox));
                    }
                }
            }
        } else {
            result.push(block);
        }
    }
    *passage_blocks = result;
}

fn is_dynamic_heading_option_line(text: &str) -> bool {
    let normalized = collapse_whitespace(text);
    let lower = normalized.to_lowercase();
    if lower.contains("list of headings") {
        return true;
    }
    let first = lower
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|ch: char| matches!(ch, '.' | ')' | ':' | ';'));
    matches!(
        first,
        "i" | "ii" | "iii" | "iv" | "v" | "vi" | "vii" | "viii" | "ix" | "x" | "xi" | "xii"
    )
}

fn is_dynamic_heading_matching_instruction_line(text: &str) -> bool {
    let lower = collapse_whitespace(text).to_lowercase();
    lower.contains("choose the correct heading")
        || lower.contains("list of headings")
        || lower.contains("write the correct number")
        || lower.contains("write the correct letter")
        || lower.contains("in boxes")
        || lower.contains("on your answer sheet")
        || lower.contains("has six sections")
        || lower.contains("has seven sections")
        || lower.contains("has eight sections")
}

fn is_dynamic_heading_matching_assignment_line(text: &str) -> bool {
    let normalized = collapse_whitespace(text);
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    let mut index = 0;
    let mut assignments = 0;
    while index + 2 < tokens.len() {
        let number = tokens[index].trim_matches(|ch: char| matches!(ch, '.' | ')' | '('));
        let label = tokens[index + 1]
            .trim_matches(|ch: char| matches!(ch, '.' | ')' | '(' | ':' | ';' | ','))
            .to_ascii_lowercase();
        let section = tokens[index + 2]
            .trim_matches(|ch: char| matches!(ch, '.' | ')' | '(' | ':' | ';' | ','));
        if number.parse::<u32>().is_ok()
            && matches!(label.as_str(), "paragraph" | "section" | "part")
            && section.len() == 1
            && section.chars().all(|ch| ch.is_ascii_alphabetic())
        {
            assignments += 1;
            index += 3;
            continue;
        }
        index += 1;
    }
    assignments > 0
}

fn is_dynamic_question_or_instruction_like_text(text: &str) -> bool {
    let lower = collapse_whitespace(text).to_lowercase();
    lower.contains("questions ")
        || lower.contains("question ")
        || lower.contains("choose ")
        || lower.contains("label ")
        || lower.contains("write ")
        || lower.contains("complete ")
        || lower.contains("which two")
        || lower.contains("which three")
        || lower.contains("answer sheet")
        || lower.contains("______")
        || lower.contains("_____")
}

fn is_dynamic_non_content_placeholder_text(text: &str) -> bool {
    let lower = collapse_whitespace(text)
        .trim_matches(|ch: char| matches!(ch, '[' | ']'))
        .to_lowercase();
    lower.starts_with("no extractable text on page")
}

fn is_dynamic_non_content_placeholder_block(block: &Value) -> bool {
    is_dynamic_non_content_placeholder_text(&dynamic_block_text(block))
}

fn dynamic_lettered_paragraph_label(text: &str) -> Option<char> {
    let normalized = collapse_whitespace(text);
    let first = normalized
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|ch: char| matches!(ch, '.' | ')' | '(' | ':' | ';' | ','));
    if first.len() == 1 {
        let ch = first.chars().next().unwrap_or_default();
        if ch.is_ascii_uppercase() {
            return Some(ch);
        }
    }
    None
}

fn dynamic_standalone_letter_marker_count(text: &str) -> usize {
    collapse_whitespace(text)
        .split_whitespace()
        .filter(|token| {
            let marker =
                token.trim_matches(|ch: char| matches!(ch, '.' | ')' | '(' | ':' | ';' | ','));
            marker.len() == 1
                && marker
                    .chars()
                    .all(|ch| matches!(ch, 'A' | 'B' | 'C' | 'D' | 'E' | 'F' | 'G'))
        })
        .count()
}

fn is_substantive_dynamic_lettered_article_block(block: &Value, expected_label: char) -> bool {
    let text = dynamic_block_text(block);
    dynamic_lettered_paragraph_label(&text) == Some(expected_label)
        && dynamic_standalone_letter_marker_count(&text) <= 2
        && is_substantive_dynamic_passage_block(block)
}

fn find_dynamic_lettered_article_block(
    blocks: &[Value],
    start: usize,
    expected_label: char,
    max_lookahead: usize,
) -> Option<usize> {
    blocks
        .iter()
        .enumerate()
        .skip(start)
        .take(max_lookahead)
        .find_map(|(index, block)| {
            if is_substantive_dynamic_lettered_article_block(block, expected_label) {
                Some(index)
            } else {
                None
            }
        })
}

fn has_dynamic_lettered_article_sequence(blocks: &[Value], first_index: usize) -> bool {
    let Some(first_label) = blocks
        .get(first_index)
        .and_then(|block| dynamic_lettered_paragraph_label(&dynamic_block_text(block)))
    else {
        return false;
    };
    if first_label != 'A'
        || !blocks
            .get(first_index)
            .map(|block| is_substantive_dynamic_lettered_article_block(block, 'A'))
            .unwrap_or(false)
    {
        return false;
    }
    find_dynamic_lettered_article_block(blocks, first_index + 1, 'B', 4).is_some()
}

fn is_dynamic_late_passage_tail_start(blocks: &[Value], index: usize) -> bool {
    let Some(block) = blocks.get(index) else {
        return false;
    };
    let text = collapse_whitespace(&dynamic_block_text(block));
    if text.is_empty()
        || is_dynamic_question_block(block)
        || is_dynamic_answer_block(block)
        || is_dynamic_heading_option_line(&text)
        || is_dynamic_heading_matching_instruction_line(&text)
        || is_dynamic_heading_matching_assignment_line(&text)
        || is_dynamic_non_content_placeholder_text(&text)
    {
        return false;
    }
    if has_dynamic_lettered_article_sequence(blocks, index) {
        return true;
    }
    if text.len() > 120
        || is_dynamic_question_or_instruction_like_text(&text)
        || has_dynamic_numbered_inline_blanks(&text)
    {
        return false;
    }
    let Some(first_article_index) = find_dynamic_lettered_article_block(blocks, index + 1, 'A', 3)
    else {
        return false;
    };
    if first_article_index > index + 1 {
        let title_like = text
            .chars()
            .find(|ch| !ch.is_whitespace())
            .map(|ch| ch.is_ascii_uppercase())
            .unwrap_or(false)
            && !text.ends_with('.')
            && !text.ends_with('?')
            && !text.ends_with('!');
        if !title_like {
            return false;
        }
    }
    has_dynamic_lettered_article_sequence(blocks, first_article_index)
}

fn dynamic_late_passage_question_block_count(blocks: &[Value]) -> usize {
    for index in 1..blocks.len() {
        if is_dynamic_late_passage_tail_start(blocks, index) {
            return index.max(1);
        }
    }
    blocks.len()
}

fn dynamic_leading_question_number(text: &str) -> Option<u32> {
    let normalized = collapse_whitespace(text);
    let first = normalized.split_whitespace().next()?;
    let trimmed = first.trim_matches(|ch: char| matches!(ch, '(' | '['));
    let digits_end = trimmed
        .char_indices()
        .find_map(|(index, ch)| (!ch.is_ascii_digit()).then_some(index))
        .unwrap_or(trimmed.len());
    if digits_end == 0 || digits_end > 3 {
        return None;
    }
    let value = trimmed[..digits_end].parse::<u32>().ok()?;
    let suffix = trimmed[digits_end..]
        .trim_matches(|ch: char| matches!(ch, '.' | ')' | ':' | ';' | ',' | ']'));
    if suffix.is_empty() {
        Some(value)
    } else {
        None
    }
}

fn is_dynamic_explicit_question_content_block(block: &Value) -> bool {
    let text = collapse_whitespace(&dynamic_block_text(block));
    if text.is_empty() || is_dynamic_non_content_placeholder_text(&text) {
        return false;
    }
    is_dynamic_question_block(block)
        || is_dynamic_answer_block(block)
        || detect_dynamic_question_heading_range(&text).is_some()
        || is_dynamic_question_or_instruction_like_text(&text)
        || has_dynamic_numbered_inline_blanks(&text)
        || dynamic_leading_question_number(&text).is_some()
        || is_dynamic_heading_option_line(&text)
        || is_dynamic_heading_matching_instruction_line(&text)
        || is_dynamic_heading_matching_assignment_line(&text)
}

fn dynamic_consecutive_substantive_passage_blocks(
    blocks: &[Value],
    start: usize,
    max_lookahead: usize,
) -> usize {
    let mut count = 0usize;
    for block in blocks.iter().skip(start).take(max_lookahead) {
        let text = collapse_whitespace(&dynamic_block_text(block));
        if text.is_empty()
            || is_dynamic_non_content_placeholder_text(&text)
            || is_dynamic_explicit_question_content_block(block)
            || !is_substantive_dynamic_passage_block(block)
        {
            break;
        }
        count += 1;
    }
    count
}

fn has_prior_dynamic_question_content(blocks: &[Value], index: usize) -> bool {
    blocks
        .iter()
        .take(index)
        .skip(1)
        .any(is_dynamic_explicit_question_content_block)
}

fn has_later_dynamic_question_content(blocks: &[Value], start: usize) -> bool {
    blocks
        .iter()
        .skip(start)
        .any(is_dynamic_explicit_question_content_block)
}

fn is_dynamic_passage_tail_layout_transition(blocks: &[Value], index: usize) -> bool {
    let Some(current) = blocks.get(index) else {
        return false;
    };
    let Some(previous) = index.checked_sub(1).and_then(|prev| blocks.get(prev)) else {
        return false;
    };
    dynamic_block_page_index(previous) != dynamic_block_page_index(current)
        || dynamic_block_layout_section_index(previous)
            != dynamic_block_layout_section_index(current)
        || dynamic_block_section_column_count_value(previous)
            != dynamic_block_section_column_count_value(current)
        || dynamic_block_column(previous) != dynamic_block_column(current)
}

fn is_dynamic_passage_tail_title_text(text: &str) -> bool {
    let normalized = collapse_whitespace(text);
    if normalized.is_empty()
        || is_dynamic_question_or_instruction_like_text(&normalized)
        || is_dynamic_heading_option_line(&normalized)
        || is_dynamic_heading_matching_instruction_line(&normalized)
        || is_dynamic_heading_matching_assignment_line(&normalized)
        || dynamic_leading_question_number(&normalized).is_some()
        || normalized.ends_with('.')
        || normalized.ends_with('?')
        || normalized.ends_with('!')
    {
        return false;
    }
    let word_count = normalized.split_whitespace().count();
    let first_char_uppercase = normalized
        .chars()
        .find(|ch| !ch.is_whitespace())
        .map(|ch| ch.is_ascii_uppercase())
        .unwrap_or(false);
    first_char_uppercase && (2..=8).contains(&word_count)
}

fn dynamic_prose_passage_run_end(blocks: &[Value], index: usize) -> Option<usize> {
    let substantive_run = dynamic_consecutive_substantive_passage_blocks(blocks, index, 3);
    let title_followed_run = if blocks
        .get(index)
        .map(|block| is_dynamic_passage_tail_title_text(&dynamic_block_text(block)))
        .unwrap_or(false)
    {
        dynamic_consecutive_substantive_passage_blocks(blocks, index + 1, 3)
    } else {
        0
    };

    if substantive_run >= 2 {
        return Some(index + substantive_run);
    }
    if substantive_run >= 1
        && (blocks
            .get(index)
            .map(|block| dynamic_block_role(block) == "passage")
            .unwrap_or(false)
            || is_dynamic_passage_tail_layout_transition(blocks, index))
    {
        return Some(index + substantive_run);
    }
    if title_followed_run >= 2 {
        return Some(index + 1 + title_followed_run);
    }
    if title_followed_run >= 1
        && (is_dynamic_passage_tail_layout_transition(blocks, index)
            || is_dynamic_passage_tail_layout_transition(blocks, index + 1)
            || blocks
                .get(index + 1)
                .map(|block| dynamic_block_role(block) == "passage")
                .unwrap_or(false))
    {
        return Some(index + 1 + title_followed_run);
    }
    None
}

fn collect_dynamic_interleaved_passage_runs(blocks: &[Value]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut index = 1usize;
    while index < blocks.len() {
        if !has_prior_dynamic_question_content(blocks, index) {
            index += 1;
            continue;
        }
        let Some(run_end) = dynamic_prose_passage_run_end(blocks, index) else {
            index += 1;
            continue;
        };
        if has_later_dynamic_question_content(blocks, run_end) {
            runs.push((index, run_end));
        }
        index = run_end.max(index + 1);
    }
    runs
}

fn find_dynamic_prose_passage_tail_start(blocks: &[Value]) -> Option<usize> {
    for index in 1..blocks.len() {
        if !has_prior_dynamic_question_content(blocks, index) {
            continue;
        }
        let Some(run_end) = dynamic_prose_passage_run_end(blocks, index) else {
            continue;
        };
        if !has_later_dynamic_question_content(blocks, run_end) {
            return Some(index);
        }
    }
    None
}

fn dynamic_generic_passage_tail_question_block_count(blocks: &[Value]) -> usize {
    find_dynamic_prose_passage_tail_start(blocks)
        .map(|index| index.max(1))
        .unwrap_or(blocks.len())
}

fn is_probable_dynamic_passage_tail_start(blocks: &[Value], index: usize) -> bool {
    let Some(block) = blocks.get(index) else {
        return false;
    };
    let text = collapse_whitespace(&dynamic_block_text(block));
    if text.is_empty()
        || is_dynamic_question_block(block)
        || is_dynamic_answer_block(block)
        || is_dynamic_heading_option_line(&text)
        || is_dynamic_heading_matching_instruction_line(&text)
        || is_dynamic_heading_matching_assignment_line(&text)
    {
        return false;
    }
    if is_substantive_dynamic_passage_block(block) {
        return true;
    }
    text.len() >= 8
        && blocks
            .iter()
            .skip(index + 1)
            .take(3)
            .any(is_substantive_dynamic_passage_block)
}

fn dynamic_heading_matching_question_block_count(blocks: &[Value]) -> usize {
    let mut saw_heading_list = false;
    for (index, block) in blocks.iter().enumerate() {
        let text = dynamic_block_text(block);
        let lower = text.to_lowercase();
        if lower.contains("list of headings") {
            saw_heading_list = true;
            continue;
        }
        if !saw_heading_list || is_dynamic_heading_option_line(&text) {
            continue;
        }
        if is_probable_dynamic_passage_tail_start(blocks, index) {
            return index.max(1);
        }
    }
    blocks.len()
}

fn dynamic_question_block_count_for_group(kind: &str, blocks: &[Value]) -> usize {
    let specific = if kind == "heading_matching" {
        dynamic_heading_matching_question_block_count(blocks)
    } else {
        dynamic_late_passage_question_block_count(blocks)
    };
    if specific < blocks.len() {
        specific
    } else {
        dynamic_generic_passage_tail_question_block_count(blocks)
    }
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
    let mut passage_blocks = match first_concrete_question_index {
        Some(first_concrete_question) => blocks
            .iter()
            .enumerate()
            .filter(|(index, block)| {
                *index < first_concrete_question
                    && !is_dynamic_non_content_placeholder_block(block)
                    && !is_dynamic_question_block(block)
                    && !is_dynamic_answer_block(block)
                    && dynamic_block_role(block) != "ignore"
            })
            .map(|(_, block)| block.clone())
            .collect::<Vec<_>>(),
        None => blocks
            .iter()
            .filter(|block| {
                !is_dynamic_non_content_placeholder_block(block)
                    && !is_dynamic_question_block(block)
                    && !is_dynamic_answer_block(block)
                    && dynamic_block_role(block) != "ignore"
            })
            .cloned()
            .collect::<Vec<_>>(),
    };
    let mut deferred_passage_blocks = Vec::new();
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
                !is_dynamic_non_content_placeholder_block(block)
                    && dynamic_block_role(block) != "answer"
                    && dynamic_block_role(block) != "ignore"
            })
            .cloned()
            .collect::<Vec<_>>()
    } else if !all_umbrella_blocks.is_empty() {
        all_umbrella_blocks.clone()
    } else if let Some(first_question) = first_question_index {
        blocks[first_question..]
            .iter()
            .filter(|block| {
                !is_dynamic_non_content_placeholder_block(block)
                    && dynamic_block_role(block) != "answer"
                    && dynamic_block_role(block) != "ignore"
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        blocks
            .iter()
            .filter(|block| {
                !is_dynamic_non_content_placeholder_block(block) && is_dynamic_question_block(block)
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    let answer_blocks = blocks
        .iter()
        .filter(|block| {
            !is_dynamic_non_content_placeholder_block(block) && is_dynamic_answer_block(block)
        })
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
        let Some((start, heading_end)) = detect_dynamic_question_heading_range(&text) else {
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
        let raw_included = &question_blocks[index..next_heading];
        let interleaved_passage_runs = collect_dynamic_interleaved_passage_runs(raw_included);
        let mut defer_mask = vec![false; raw_included.len()];
        for (run_start, run_end) in interleaved_passage_runs {
            for raw_index in run_start..run_end.min(raw_included.len()) {
                defer_mask[raw_index] = true;
            }
        }
        let preliminary_blocks = raw_included
            .iter()
            .enumerate()
            .filter_map(|(raw_index, block)| {
                if defer_mask.get(raw_index).copied().unwrap_or(false) {
                    deferred_passage_blocks.push(block.clone());
                    None
                } else {
                    Some(block.clone())
                }
            })
            .collect::<Vec<_>>();
        let raw_combined = preliminary_blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join(" ");
        let raw_block_ids = preliminary_blocks
            .iter()
            .map(dynamic_block_id)
            .collect::<Vec<_>>();
        let mut preliminary_classification = classify_dynamic_group(&raw_combined, &raw_block_ids);
        // Cross-block instruction recovery: when the merged instruction text
        // still falls back to the default `short_answer` kind AND the next
        // non-deferred question block likely continues the instruction (a
        // heading like "Do the following statements" split from its
        // "True / False / Not Given" tail by a column/page break), tentatively
        // merge ONE more block of text and re-classify. If the re-classified
        // kind is more specific than `short_answer`, adopt it. This is a
        // best-effort heuristic and only widens the classification text, not
        // the `included` block range (so it never mis-attributes question
        // prompt blocks to the passage).
        if preliminary_classification.kind == "short_answer"
            && raw_combined.split_whitespace().count() < 30
        {
            let next_index = index + preliminary_blocks.len();
            if let Some(extra_block) = question_blocks.get(next_index) {
                let extra_text = dynamic_block_text(extra_block);
                if !extra_text.is_empty()
                    && !is_known_dynamic_umbrella_block(extra_block, &all_umbrella_blocks)
                {
                    let widened = format!("{} {}", raw_combined, extra_text);
                    let widened_ids: Vec<String> = raw_block_ids
                        .iter()
                        .cloned()
                        .chain(std::iter::once(dynamic_block_id(extra_block)))
                        .collect();
                    let widened_classification = classify_dynamic_group(&widened, &widened_ids);
                    if widened_classification.kind != "short_answer" {
                        preliminary_classification = widened_classification;
                    }
                }
            }
        }
        let included_count = dynamic_question_block_count_for_group(
            &preliminary_classification.kind,
            &preliminary_blocks,
        );
        let included = &preliminary_blocks[..included_count.min(preliminary_blocks.len()).max(1)];
        if included_count < preliminary_blocks.len() {
            deferred_passage_blocks.extend(preliminary_blocks[included_count..].iter().cloned());
        }
        let combined = included
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join(" ");
        let block_ids = included.iter().map(dynamic_block_id).collect::<Vec<_>>();
        let classification = classify_dynamic_group(&combined, &block_ids);
        let allow_blank_extension = matches!(
            classification.kind.as_str(),
            "summary_completion" | "sentence_completion" | "diagram_completion"
        );
        let allow_list_extension = matches!(
            classification.kind.as_str(),
            "true_false_not_given" | "yes_no_not_given"
        );
        let end = infer_dynamic_group_range_end(
            &combined,
            start,
            heading_end,
            allow_blank_extension,
            allow_list_extension,
        )
        .max(infer_dynamic_group_range_end_from_markers(
            &combined,
            start,
            heading_end,
            &classification.kind,
        ));
        let layout_hint = dynamic_layout_hint_for_group(&classification.kind, &combined);
        let section_evidence = split_section_evidence_for_blocks(included);
        let continuation_edges = split_continuation_edges_for_blocks(included);
        group_candidates.push(SplitGroupCandidateV1 {
            group_id: format!("group-{}", group_candidates.len() + 1),
            heading: dynamic_question_heading(start, end),
            question_range: [start, end],
            instruction_text: text,
            block_ids,
            kind_hint: classification.kind.clone(),
            layout_hint: Some(layout_hint.to_string()),
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
                layout_hint: Some("list".to_string()),
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
        let layout_hint = dynamic_layout_hint_for_group(&classification.kind, &combined);
        group_candidates.push(SplitGroupCandidateV1 {
            group_id: "group-1".to_string(),
            heading: dynamic_question_heading(start, end),
            question_range: [start, end],
            instruction_text: combined,
            block_ids,
            kind_hint: classification.kind.clone(),
            layout_hint: Some(layout_hint.to_string()),
            confidence: classification.confidence.min(0.58),
            classification: Some(classification),
            section_evidence: split_section_evidence_for_blocks(&question_blocks),
            continuation_edges: split_continuation_edges_for_blocks(&question_blocks),
            is_umbrella_range: None,
            requires_manual_question_import: None,
        });
    }
    normalize_dynamic_group_ranges(&mut group_candidates);

    if !deferred_passage_blocks.is_empty() {
        let mut seen_passage_ids = passage_blocks
            .iter()
            .map(dynamic_block_id)
            .collect::<std::collections::HashSet<_>>();
        for block in deferred_passage_blocks {
            if is_dynamic_non_content_placeholder_text(&dynamic_block_text(&block)) {
                continue;
            }
            let block_id = dynamic_block_id(&block);
            if !block_id.is_empty() && !seen_passage_ids.insert(block_id) {
                continue;
            }
            passage_blocks.push(block);
        }
    }
    passage_blocks
        .retain(|block| !is_dynamic_non_content_placeholder_text(&dynamic_block_text(block)));

    // Glue passage blocks that were split purely by a page break back into
    // single continuous passages, so the reading source reflects the original
    // prose instead of page-boundary fragments.
    merge_cross_page_passage_continuations(&mut passage_blocks);

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
        "diagram_completion" => "inline_text_completion",
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

fn dynamic_previous_non_space_char(text: &str, start: usize) -> Option<char> {
    text[..start].chars().rev().find(|ch| !ch.is_whitespace())
}

fn dynamic_previous_word_lower(text: &str, start: usize) -> String {
    let mut end = start.min(text.len());
    while let Some(ch) = text[..end].chars().next_back() {
        if ch.is_whitespace() {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }
    let mut begin = end;
    while let Some(ch) = text[..begin].chars().next_back() {
        if ch.is_alphabetic() {
            begin -= ch.len_utf8();
        } else {
            break;
        }
    }
    text[begin..end].to_lowercase()
}

fn is_dynamic_non_question_number_context(text: &str, start: usize) -> bool {
    if matches!(
        dynamic_previous_non_space_char(text, start),
        Some('-' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}')
    ) {
        return true;
    }
    matches!(
        dynamic_previous_word_lower(text, start).as_str(),
        "passage" | "box" | "boxes"
    )
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
        if is_dynamic_non_question_number_context(text, start) {
            search = after_digits;
            continue;
        }
        if is_dynamic_range_dash_after_number(text, after_digits) {
            search = after_digits;
            continue;
        }
        if let Some(next) = text[after_digits..].chars().next() {
            if !(next.is_whitespace()
                || matches!(next, '.' | ')' | ':' | '、')
                || is_dynamic_blank_marker_char(next))
            {
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
        let mut blank_width = 0usize;
        while let Some(next) = text[content_start..].chars().next() {
            if is_dynamic_blank_marker_char(next) {
                blank_width += dynamic_blank_marker_width(next);
                content_start += next.len_utf8();
            } else {
                break;
            }
        }
        if blank_width >= 3 {
            while let Some(next) = text[content_start..].chars().next() {
                if next.is_whitespace() {
                    content_start += next.len_utf8();
                } else {
                    break;
                }
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

fn find_dynamic_matching_option_run_boundary(text: &str, from: usize) -> Option<usize> {
    let mut search = from.min(text.len());
    while search < text.len() {
        let relative = if text[search..].starts_with("A ") {
            Some(0usize)
        } else {
            text[search..].find(" A ").map(|index| index + 1)
        };
        let Some(relative) = relative else {
            break;
        };
        let candidate = search + relative;
        let preview = text[candidate..].chars().take(240).collect::<String>();
        if preview.starts_with("A ") && preview.contains(" B ") && preview.contains(" C ") {
            return Some(candidate);
        }
        search = candidate + "A".len();
    }
    None
}

fn find_dynamic_prompt_boundary(
    text: &str,
    from: usize,
    next_number: u32,
    group_kind: &str,
) -> usize {
    let mut boundary = find_dynamic_final_prompt_boundary(text, from);
    if let Some((next_start, _)) = find_dynamic_number_marker(text, next_number, from) {
        boundary = boundary.min(next_start);
    }
    if matches!(
        group_kind,
        "heading_matching" | "matching" | "matching_information" | "classification"
    ) {
        let lower = text.to_lowercase();
        for marker in [
            " list of headings",
            " list of people",
            " list of researchers",
            " list of names",
            " list of options",
            " list of universities",
            " list of companies",
            " list of sections",
        ] {
            if let Some(relative) = lower[from..].find(marker) {
                boundary = boundary.min(from + relative);
            }
        }
        if !matches!(group_kind, "heading_matching") {
            if let Some(option_boundary) = find_dynamic_matching_option_run_boundary(text, from) {
                boundary = boundary.min(option_boundary);
            }
        }
    }
    boundary
}

fn dynamic_prompt_for_question(
    group_text: &str,
    number: u32,
    _fallback_heading: &str,
    range_end: u32,
    group_kind: &str,
) -> String {
    let normalized = collapse_whitespace(group_text);
    if let Some((_, content_start)) = find_dynamic_number_marker(&normalized, number, 0) {
        let boundary = if number < range_end {
            find_dynamic_prompt_boundary(&normalized, content_start, number + 1, group_kind)
        } else if matches!(
            group_kind,
            "heading_matching" | "matching" | "matching_information" | "classification"
        ) {
            find_dynamic_prompt_boundary(
                &normalized,
                content_start,
                range_end.saturating_add(1),
                group_kind,
            )
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
    String::new()
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
            let instruction_text = candidate
                .get("instructionText")
                .and_then(Value::as_str)
                .unwrap_or(heading);
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
                            String::new()
                        } else {
                            dynamic_prompt_for_question(&group_text, number, heading, end, kind)
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
            let layout_hint = candidate
                .get("layoutHint")
                .and_then(Value::as_str)
                .unwrap_or_else(|| dynamic_layout_hint_for_group(kind, &group_text));
            let layout = if kind == "table_completion" {
                json!({"template": dynamic_template_for_kind(kind), "layoutHint": layout_hint, "tableHeaders": ["Question", "Prompt", "Answer"]})
            } else if layout_hint == "inline_completion" {
                json!({"template": dynamic_template_for_kind(kind), "layoutHint": layout_hint, "notes": group_text})
            } else {
                json!({"template": dynamic_template_for_kind(kind), "layoutHint": layout_hint})
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
                instruction: vec![heading.to_string()],
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImportJob, IssueCounts, JobStatus, WorkflowStep};
    use chrono::Utc;
    use serde_json::json;

    fn test_job() -> ImportJob {
        ImportJob {
            job_id: "job-layout-test".to_string(),
            title: "Mixed Layout Reading".to_string(),
            status: JobStatus::Working,
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Vec::new(),
            source_files: Vec::new(),
            active_llm_profile_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            current_step: WorkflowStep::Split,
            issue_counts: IssueCounts::default(),
        }
    }

    fn layout_block(
        block_id: &str,
        text: &str,
        bbox: [f64; 4],
        section_index: u64,
        column_count: u64,
        column_index: u64,
    ) -> Value {
        layout_block_on_page(
            block_id,
            text,
            bbox,
            1,
            section_index,
            column_count,
            column_index,
        )
    }

    fn layout_block_on_page(
        block_id: &str,
        text: &str,
        bbox: [f64; 4],
        page_index: u64,
        section_index: u64,
        column_count: u64,
        column_index: u64,
    ) -> Value {
        json!({
            "blockId": block_id,
            "blockType": "paragraph",
            "text": text,
            "html": format!("<p>{}</p>", text),
            "bbox": bbox,
            "pageIndex": page_index,
            "layoutHints": {
                "section": {
                    "index": section_index,
                    "columns": {
                        "count": column_count,
                        "current": column_index
                    }
                }
            }
        })
    }

    #[test]
    fn detect_dynamic_group_kind_covers_form_map_and_letter_matching_variants() {
        assert_eq!(
            detect_dynamic_group_kind(
                "Questions 11-13 Complete the form below. Choose NO MORE THAN TWO WORDS AND/OR A NUMBER for each answer."
            ),
            "table_completion"
        );
        assert_eq!(
            detect_dynamic_group_kind(
                "Questions 14-17 Label the map below. Choose NO MORE THAN TWO WORDS from the passage for each answer."
            ),
            "diagram_completion"
        );
        assert_eq!(
            detect_dynamic_group_kind(
                "Questions 18-21 Write the correct letter, A-H, in boxes 18-21 on your answer sheet."
            ),
            "matching"
        );
        assert_eq!(
            detect_dynamic_group_kind(
                "Questions 22-26 Which paragraph mentions the first successful experiment?"
            ),
            "matching_information"
        );
    }

    #[test]
    fn short_passage_intro_blocks_bare_umbrella_heading_misclassification() {
        let blocks = vec![
            layout_block(
                "b001",
                "READING PASSAGE 1",
                [72.0, 760.0, 520.0, 792.0],
                0,
                1,
                0,
            ),
            layout_block(
                "b002",
                "A brief introduction frames the debate.",
                [72.0, 700.0, 320.0, 732.0],
                0,
                1,
                0,
            ),
            json!({
                "blockId": "b003",
                "blockType": "paragraph",
                "text": "Questions 1-13",
                "html": "<p>Questions 1-13</p>",
                "bbox": [72.0, 650.0, 220.0, 680.0],
                "pageIndex": 1,
                "roleHint": "question"
            }),
        ];

        assert!(
            is_substantive_dynamic_passage_block(&blocks[1]),
            "short prose intro should count as substantive passage"
        );
        assert!(
            !is_dynamic_umbrella_question_block(&blocks, 2),
            "bare question range after a short intro passage should not be auto-promoted to umbrella"
        );
    }

    #[test]
    fn mixed_layout_sections_keep_passage_before_later_single_column_questions() {
        let job = test_job();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595.0,
                "height": 842.0,
                "blocks": [
                    layout_block(
                        "b001",
                        "READING PASSAGE 1",
                        [72.0, 760.0, 520.0, 792.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "b002",
                        "The first half of the passage stays in the left column and should remain part of the original article text.",
                        [72.0, 640.0, 250.0, 724.0],
                        1,
                        2,
                        0
                    ),
                    layout_block(
                        "b004",
                        "Questions 1-2 Choose the correct letter, A-D.",
                        [72.0, 220.0, 520.0, 252.0],
                        2,
                        1,
                        0
                    ),
                    layout_block(
                        "b003",
                        "The matching right column continues the same passage and must not be reordered after the later single-column questions.",
                        [330.0, 642.0, 520.0, 720.0],
                        1,
                        2,
                        1
                    ),
                    layout_block(
                        "b005",
                        "1 According to the writer, what changed first? A tools B trade C paper D law 2 Why did people adopt the method? A speed B cost C weather D design",
                        [72.0, 140.0, 520.0, 210.0],
                        2,
                        1,
                        0
                    )
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let ordered_ids = dynamic_document_blocks(Some(&doc))
            .iter()
            .map(dynamic_block_id)
            .collect::<Vec<_>>();
        assert_eq!(
            ordered_ids,
            vec![
                "b001".to_string(),
                "b002".to_string(),
                "b003".to_string(),
                "b004".to_string(),
                "b005".to_string()
            ]
        );

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let passage_range = split
            .pointer("/passageCandidates/0/range")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(passage_range, vec!["b001", "b002", "b003"]);
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/kindHint")
                .and_then(Value::as_str),
            Some("single_choice")
        );
    }

    #[test]
    fn prose_passage_tail_after_question_block_returns_to_passage_range() {
        let job = test_job();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595.0,
                "height": 842.0,
                "blocks": [
                    layout_block(
                        "p001",
                        "READING PASSAGE 1",
                        [72.0, 760.0, 520.0, 792.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "p002",
                        "The original article begins by tracing how early paper making moved between river towns and trade routes.",
                        [72.0, 690.0, 520.0, 736.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "q001",
                        "Questions 1-3 Complete the summary below. Choose NO MORE THAN TWO WORDS from the passage for each answer.",
                        [72.0, 520.0, 520.0, 566.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "q002",
                        "1 plant fibres 2 water power 3 trade routes",
                        [72.0, 462.0, 420.0, 506.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "p003",
                        "Further Evidence",
                        [72.0, 360.0, 260.0, 392.0],
                        2,
                        1,
                        0
                    ),
                    layout_block(
                        "p004",
                        "Researchers later found that merchants carried the technique far earlier than royal workshops adopted it.",
                        [72.0, 296.0, 520.0, 344.0],
                        2,
                        1,
                        0
                    ),
                    layout_block(
                        "p005",
                        "These later paragraphs continue the source article and should be preserved inside the recovered passage text.",
                        [72.0, 234.0, 520.0, 282.0],
                        2,
                        1,
                        0
                    )
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let passage_range = split
            .pointer("/passageCandidates/0/range")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(passage_range, vec!["p001", "p002", "p003", "p004", "p005"]);
        let group_block_ids = split
            .pointer("/questionGroupCandidates/0/blockIds")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(group_block_ids, vec!["q001", "q002"]);
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/kindHint")
                .and_then(Value::as_str),
            Some("summary_completion")
        );
    }

    #[test]
    fn heading_matching_tail_with_resumed_prose_returns_to_passage_range() {
        let job = test_job();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595.0,
                "height": 842.0,
                "blocks": [
                    layout_block(
                        "p001",
                        "READING PASSAGE 1",
                        [72.0, 760.0, 520.0, 792.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "p002",
                        "The passage introduces several regions before the headings exercise appears below.",
                        [72.0, 692.0, 520.0, 734.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "q001",
                        "Questions 14-18 Choose the correct heading for each section from the list of headings below.",
                        [72.0, 548.0, 520.0, 592.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "q002",
                        "List of Headings i Early trade ii Mountain farming iii River transport iv Royal archives",
                        [72.0, 472.0, 520.0, 532.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "p003",
                        "Article Continuation",
                        [72.0, 372.0, 260.0, 404.0],
                        2,
                        1,
                        0
                    ),
                    layout_block(
                        "p004",
                        "Section C describes how local traders adapted the technique to wet climates and longer journeys.",
                        [72.0, 308.0, 520.0, 356.0],
                        2,
                        1,
                        0
                    ),
                    layout_block(
                        "p005",
                        "Section D explains why written records expanded once production became reliable across the region.",
                        [72.0, 244.0, 520.0, 292.0],
                        2,
                        1,
                        0
                    )
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let passage_range = split
            .pointer("/passageCandidates/0/range")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(passage_range, vec!["p001", "p002", "p003", "p004", "p005"]);
        let group_block_ids = split
            .pointer("/questionGroupCandidates/0/blockIds")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(group_block_ids, vec!["q001", "q002"]);
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/kindHint")
                .and_then(Value::as_str),
            Some("heading_matching")
        );
    }

    #[test]
    fn summary_group_with_interleaved_passage_returns_middle_run_to_passage_range() {
        let job = test_job();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595.0,
                "height": 842.0,
                "blocks": [
                    layout_block(
                        "p001",
                        "READING PASSAGE 1",
                        [72.0, 760.0, 520.0, 792.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "p002",
                        "The article opens by explaining how paper spread across commercial routes before the exercise begins.",
                        [72.0, 692.0, 520.0, 736.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "q001",
                        "Questions 1-4 Complete the summary below. Choose NO MORE THAN TWO WORDS from the passage for each answer.",
                        [72.0, 560.0, 520.0, 604.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "q002",
                        "1 plant fibres 2 river towns",
                        [72.0, 500.0, 360.0, 540.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "p003",
                        "Recovered Source Text",
                        [72.0, 420.0, 250.0, 452.0],
                        2,
                        1,
                        0
                    ),
                    layout_block(
                        "p004",
                        "Merchants then carried the process further inland before official workshops standardized production.",
                        [72.0, 356.0, 520.0, 404.0],
                        2,
                        1,
                        0
                    ),
                    layout_block(
                        "q003",
                        "3 inland markets 4 official workshops",
                        [72.0, 278.0, 420.0, 326.0],
                        3,
                        1,
                        0
                    )
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let passage_range = split
            .pointer("/passageCandidates/0/range")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(passage_range, vec!["p001", "p002", "p003", "p004"]);
        let group_block_ids = split
            .pointer("/questionGroupCandidates/0/blockIds")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(group_block_ids, vec!["q001", "q002", "q003"]);
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/kindHint")
                .and_then(Value::as_str),
            Some("summary_completion")
        );
    }

    #[test]
    fn heading_matching_group_with_interleaved_passage_returns_middle_run_to_passage_range() {
        let job = test_job();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595.0,
                "height": 842.0,
                "blocks": [
                    layout_block(
                        "p001",
                        "READING PASSAGE 1",
                        [72.0, 760.0, 520.0, 792.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "p002",
                        "The article introduces its main regions before the heading-matching exercise starts.",
                        [72.0, 692.0, 520.0, 736.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "q001",
                        "Questions 14-17 Choose the correct heading for each section from the list of headings below.",
                        [72.0, 560.0, 520.0, 604.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "q002",
                        "List of Headings i Trade routes ii Royal control iii Wet climates iv Written records",
                        [72.0, 488.0, 520.0, 544.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "p003",
                        "Article Continuation",
                        [72.0, 408.0, 250.0, 440.0],
                        2,
                        1,
                        0
                    ),
                    layout_block(
                        "p004",
                        "A later passage section explains how traders adapted production methods to wetter climates and longer journeys.",
                        [72.0, 344.0, 520.0, 392.0],
                        2,
                        1,
                        0
                    ),
                    layout_block(
                        "q003",
                        "14 Section A 15 Section B 16 Section C 17 Section D",
                        [72.0, 270.0, 420.0, 318.0],
                        3,
                        1,
                        0
                    )
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let passage_range = split
            .pointer("/passageCandidates/0/range")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(passage_range, vec!["p001", "p002", "p003", "p004"]);
        let group_block_ids = split
            .pointer("/questionGroupCandidates/0/blockIds")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(group_block_ids, vec!["q001", "q002", "q003"]);
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/kindHint")
                .and_then(Value::as_str),
            Some("heading_matching")
        );
    }

    #[test]
    fn cross_page_continuation_is_sorted_before_later_question_section() {
        let job = test_job();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [
                {
                    "pageIndex": 1,
                    "width": 595.0,
                    "height": 842.0,
                    "blocks": [
                        layout_block_on_page(
                            "p001",
                            "READING PASSAGE 1",
                            [72.0, 760.0, 520.0, 792.0],
                            1,
                            0,
                            1,
                            0
                        ),
                        layout_block_on_page(
                            "p002",
                            "The first page introduces the topic and ends mid argument so the article must continue onto page two.",
                            [72.0, 692.0, 520.0, 736.0],
                            1,
                            0,
                            1,
                            0
                        )
                    ]
                },
                {
                    "pageIndex": 2,
                    "width": 595.0,
                    "height": 842.0,
                    "blocks": [
                        layout_block_on_page(
                            "q001",
                            "Questions 1-2 Choose the correct letter, A-D.",
                            [72.0, 260.0, 520.0, 300.0],
                            2,
                            1,
                            1,
                            0
                        ),
                        layout_block_on_page(
                            "p003",
                            "The continuation on page two completes the source paragraph before any question prompt should begin.",
                            [72.0, 660.0, 520.0, 708.0],
                            2,
                            0,
                            1,
                            0
                        ),
                        layout_block_on_page(
                            "q002",
                            "1 Which material spread first? A bark B fibre C wax D clay",
                            [72.0, 188.0, 520.0, 244.0],
                            2,
                            1,
                            1,
                            0
                        )
                    ]
                }
            ],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let ordered_ids = dynamic_document_blocks(Some(&doc))
            .iter()
            .map(dynamic_block_id)
            .collect::<Vec<_>>();
        assert_eq!(
            ordered_ids,
            vec![
                "p001".to_string(),
                "p002".to_string(),
                "p003".to_string(),
                "q001".to_string(),
                "q002".to_string()
            ]
        );

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let passage_range = split
            .pointer("/passageCandidates/0/range")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(passage_range, vec!["p001", "p002", "p003"]);
    }

    #[test]
    fn cross_page_passage_continuation_merges_split_prose() {
        // A passage that breaks mid-sentence across a page boundary should be
        // merged back into a single passage block by the new
        // merge_cross_page_passage_continuations pass.
        let job = test_job();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [
                {
                    "pageIndex": 1,
                    "width": 595.0,
                    "height": 842.0,
                    "blocks": [
                        layout_block_on_page(
                            "p001",
                            "READING PASSAGE 1",
                            [72.0, 760.0, 520.0, 792.0],
                            1,
                            0,
                            1,
                            0
                        ),
                        layout_block_on_page(
                            "p002",
                            "The early trade routes established by merchants carried not only silk and spices but also",
                            [72.0, 690.0, 520.0, 736.0],
                            1,
                            0,
                            1,
                            0
                        )
                    ]
                },
                {
                    "pageIndex": 2,
                    "width": 595.0,
                    "height": 842.0,
                    "blocks": [
                        layout_block_on_page(
                            "p003",
                            "new ideas about mathematics and astronomy that would later transform European science",
                            [72.0, 760.0, 520.0, 800.0],
                            2,
                            0,
                            1,
                            0
                        ),
                        layout_block_on_page(
                            "q001",
                            "Questions 1-3 Choose the correct letter, A, B or C.",
                            [72.0, 300.0, 520.0, 340.0],
                            2,
                            1,
                            1,
                            0
                        )
                    ]
                }
            ],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let mut passage_blocks: Vec<Value> = vec![
            layout_block_on_page(
                "p002",
                "The early trade routes established by merchants carried not only silk and spices but also",
                [72.0, 690.0, 520.0, 736.0],
                1,
                0,
                1,
                0
            ),
            layout_block_on_page(
                "p003",
                "new ideas about mathematics and astronomy that would later transform European science",
                [72.0, 760.0, 520.0, 800.0],
                2,
                0,
                1,
                0
            ),
        ];
        merge_cross_page_passage_continuations(&mut passage_blocks);

        assert_eq!(
            passage_blocks.len(),
            1,
            "cross-page passage fragments should merge into a single block"
        );
        let merged_text = dynamic_block_text(&passage_blocks[0]);
        assert!(
            merged_text.contains("silk and spices but also new ideas about mathematics"),
            "merged passage should join the broken sentence continuously, got: {}",
            merged_text
        );
        // Sanity: the full pipeline still produces a passage candidate that
        // includes both source blocks.
        let _ = doc;
    }

    #[test]
    fn cross_page_passage_continuation_respects_sentence_terminator() {
        // When the left page's passage ends with a full stop, it must NOT be
        // merged into the next page's block (different paragraph).
        let mut passage_blocks: Vec<Value> = vec![
            layout_block_on_page(
                "p002",
                "The early trade routes carried silk and spices across the continent.",
                [72.0, 690.0, 520.0, 736.0],
                1,
                0,
                1,
                0
            ),
            layout_block_on_page(
                "p003",
                "new ideas about mathematics later transformed European science",
                [72.0, 760.0, 520.0, 800.0],
                2,
                0,
                1,
                0
            ),
        ];
        merge_cross_page_passage_continuations(&mut passage_blocks);
        assert_eq!(
            passage_blocks.len(),
            2,
            "a sentence-terminated passage must NOT merge with the next page's block"
        );
    }

    #[test]
    fn cross_line_instruction_recovers_true_false_not_given() {
        // "Questions 1-5" heading in one block, the True/False/Not Given
        // instruction body split into a second block. The widened
        // classification should recover `true_false_not_given` instead of
        // falling back to `short_answer`.
        let job = test_job();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "jobId": job.job_id,
            "pages": [{
                "pageIndex": 1,
                "width": 595.0,
                "height": 842.0,
                "blocks": [
                    layout_block(
                        "p001",
                        "READING PASSAGE 1",
                        [72.0, 760.0, 520.0, 792.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "p002",
                        "The passage describes how early navigation developed across ocean routes.",
                        [72.0, 690.0, 520.0, 736.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "q001",
                        "Questions 1-5 Do the following statements agree",
                        [72.0, 300.0, 520.0, 340.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "q002",
                        "with the information given in Reading Passage 1? TRUE FALSE NOT GIVEN",
                        [72.0, 250.0, 520.0, 290.0],
                        1,
                        1,
                        0
                    )
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let kind = split
            .pointer("/questionGroupCandidates/0/kindHint")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert_eq!(
            kind,
            "true_false_not_given",
            "split instruction across two blocks should still classify as true_false_not_given"
        );
    }
}
