use crate::{html_escape, ImportJob};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// Real IELTS matching/classification banks in the corpus extend beyond J
// (A-K/A-L are common, and some shared banks reach N). Keep one canonical
// terminal so split, prompt recovery, bank closure and review all agree.
const DYNAMIC_MAX_OPTION_LABEL: char = 'N';

fn is_dynamic_letter_option_label(label: char) -> bool {
    matches!(label, 'A'..=DYNAMIC_MAX_OPTION_LABEL)
}

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

fn has_explicit_zero_question_group_marker(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("仅原文无题")
        || lower.contains("仅文章无题")
        || lower.contains("无题版")
        || lower.contains("passage-only")
        || lower.contains("passage_only")
        || lower.contains("passage only")
        || lower.contains("no-question-groups")
        || lower.contains("no question groups")
        || lower.contains("no questions")
        || lower.contains("expected-zero-question-groups")
}

fn job_explicitly_declares_zero_question_groups(job: &ImportJob) -> bool {
    has_explicit_zero_question_group_marker(&job.title)
        || job
            .tags
            .iter()
            .any(|tag| has_explicit_zero_question_group_marker(tag))
        || job.source_files.iter().any(|source| {
            has_explicit_zero_question_group_marker(&source.original_name)
                || has_explicit_zero_question_group_marker(&source.stored_name)
        })
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
    if let Some((start, end)) = detect_dynamic_question_range(&text) {
        if end.saturating_sub(start) >= 9
            && has_opening_dynamic_question_range_position(blocks, index)
        {
            let nearby = nearby_dynamic_question_context(blocks, index);
            if has_dynamic_umbrella_question_context(&nearby) {
                return true;
            }
        }
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

fn infer_dynamic_heading_matching_range_end_from_blocks(
    blocks: &[Value],
    start: u32,
    heading_end: u32,
) -> u32 {
    let mut inferred_end = heading_end.max(start);
    let max_lookahead = start.saturating_add(20);

    for block in blocks {
        let text = dynamic_block_text(block);
        let Some(number) = dynamic_leading_question_number(&text) else {
            continue;
        };
        if number <= inferred_end {
            continue;
        }
        if number != inferred_end.saturating_add(1) || number > max_lookahead {
            break;
        }

        let prompt = strip_dynamic_leading_question_marker(&text, number);
        let word_count = prompt.split_whitespace().count();
        let first_word = prompt.to_lowercase().split_whitespace().next().map(|word| {
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
        inferred_end = number;
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
    } else if lower.contains("complete the summary") || lower.contains("summary below") {
        // IELTS summary completion can use an A-J phrase bank. The explicit
        // task shape is more specific than the generic "write the correct
        // letter" signal and must keep its completion layout; the interaction
        // is upgraded separately when a complete option bank is present.
        "summary_completion"
    } else if is_dynamic_sentence_ending_matching_text(text) {
        "matching"
    } else if normalized.contains("write the correct letter")
        && has_dynamic_letter_option_span(&normalized)
        && !is_dynamic_single_choice_text(text)
    {
        "matching"
    } else if is_dynamic_matching_prompt_text(&normalized) {
        "matching"
    } else if lower.contains("match") && lower.contains("letter") {
        "matching"
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
        "a-j",
        "a-k",
        "a-l",
        "a-m",
        "a-n",
        "letters a-c",
        "letters a-d",
        "letters a-e",
        "letters a-f",
        "letters a-g",
        "letters a-h",
        "letters a-i",
        "letters a-j",
        "letters a-k",
        "letters a-l",
        "letters a-m",
        "letters a-n",
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
        || normalized.contains("match each method")
        || normalized.contains("match each technique")
        || normalized.contains("match each with")
        || normalized.contains("look at the following")
        || normalized.contains("list of headings")
        || normalized.contains("correct heading for each")
}

fn is_dynamic_single_choice_text(text: &str) -> bool {
    let normalized = normalized_dynamic_instruction_text(text);
    let explicit_letter_list = ["a, b, c or d", "a, b, c, or d", "a, b or c", "a, b, or c"]
        .iter()
        .any(|marker| normalized.contains(marker));
    if (normalized.contains("choose the correct letter")
        || normalized.contains("write the correct letter"))
        && explicit_letter_list
    {
        return true;
    }
    if is_dynamic_matching_prompt_text(&normalized) {
        return false;
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

fn find_dynamic_numbered_blank_marker_in_range(
    text: &str,
    range_start: u32,
    range_end: u32,
    from: usize,
) -> Option<(usize, usize)> {
    if range_start > range_end {
        return None;
    }
    (range_start..=range_end)
        .filter_map(|number| find_dynamic_numbered_blank_marker(text, number, from))
        .min_by_key(|(start, _)| *start)
}

fn find_dynamic_response_blank_start(text: &str, from: usize) -> Option<usize> {
    let mut cursor = from.min(text.len());
    let mut run_start = cursor;
    let mut width = 0usize;
    while cursor < text.len() {
        let ch = text[cursor..].chars().next()?;
        if is_dynamic_blank_marker_char(ch) {
            if width == 0 {
                run_start = cursor;
            }
            width += dynamic_blank_marker_width(ch);
            if width >= 3 {
                return Some(run_start);
            }
        } else {
            width = 0;
        }
        cursor += ch.len_utf8();
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

fn dynamic_explicit_letter_list_terminal(text: &str) -> Option<char> {
    let tokens = text
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| !ch.is_ascii_alphabetic())
                .to_ascii_uppercase()
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    for start in 0..tokens.len() {
        if tokens[start] != "A" {
            continue;
        }
        let mut expected = 'A';
        let mut count = 0usize;
        let mut index = start;
        while index < tokens.len() {
            if matches!(tokens[index].as_str(), "OR" | "AND") {
                index += 1;
                continue;
            }
            let mut chars = tokens[index].chars();
            let Some(label) = chars.next() else {
                break;
            };
            if chars.next().is_some() || label != expected || !is_dynamic_letter_option_label(label)
            {
                break;
            }
            count += 1;
            expected = ((expected as u8).saturating_add(1)) as char;
            index += 1;
        }
        if count >= 3 {
            return Some(((expected as u8).saturating_sub(1)) as char);
        }
    }
    None
}

fn dynamic_letter_options_for_text(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let normalized = lower
        .replace(
            ['\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}'],
            "-",
        )
        .replace('–', "-");
    if normalized.contains("a-n") {
        ('A'..='N').map(|label| label.to_string()).collect()
    } else if normalized.contains("a-m") {
        ('A'..='M').map(|label| label.to_string()).collect()
    } else if normalized.contains("a-l") {
        ('A'..='L').map(|label| label.to_string()).collect()
    } else if normalized.contains("a-k") {
        ('A'..='K').map(|label| label.to_string()).collect()
    } else if normalized.contains("a-j") {
        ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"]
            .iter()
            .map(|value| value.to_string())
            .collect()
    } else if normalized.contains("a-i") {
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
    } else if let Some(terminal) = dynamic_explicit_letter_list_terminal(text) {
        ('A'..=terminal).map(|label| label.to_string()).collect()
    } else {
        ["A", "B", "C", "D"]
            .iter()
            .map(|value| value.to_string())
            .collect()
    }
}

/// Return the roman-numeral answer bank declared by a matching-headings
/// instruction (for example `Write the correct number, i-x`).  Headings use
/// roman labels while the paragraph/section prompts themselves commonly use
/// A-H; treating the latter as the option bank is a semantic error.
fn dynamic_declared_roman_bank_labels(text: &str) -> Vec<String> {
    let normalized = normalized_dynamic_instruction_text(text);
    // Remove whitespace around the range dash so both `i-x` and `i - x`
    // survive extraction, including PDFs that emit an en dash.
    let compact = normalized
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let terminals = [
        ("xii", 12u8),
        ("xi", 11),
        ("x", 10),
        ("ix", 9),
        ("viii", 8),
        ("vii", 7),
        ("vi", 6),
        ("v", 5),
        ("iv", 4),
        ("iii", 3),
        ("ii", 2),
    ];
    let terminal = terminals
        .iter()
        .find_map(|(label, value)| compact.contains(&format!("i-{label}")).then_some(*value));
    let Some(terminal) = terminal else {
        return Vec::new();
    };
    let labels = [
        "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii",
    ];
    labels[..terminal as usize]
        .iter()
        .map(|label| (*label).to_string())
        .collect()
}

fn dynamic_heading_options_for_text(text: &str) -> Vec<String> {
    let declared = dynamic_declared_roman_bank_labels(text);
    if !declared.is_empty() {
        return declared;
    }
    // If a source omits the range but clearly declares a heading list, use a
    // conservative roman bank. The concrete option rows are still required
    // by `dynamic_group_option_bank` before this reaches a publishable IR.
    if normalized_dynamic_instruction_text(text).contains("list of headings") {
        return ["i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x"]
            .iter()
            .map(|label| (*label).to_string())
            .collect();
    }
    dynamic_letter_options_for_text(text)
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
        "heading_matching" => GroupInteractionClassificationV1 {
            r#type: "matching".to_string(),
            options: dynamic_heading_options_for_text(text),
            allow_option_reuse,
            min_selections: None,
            max_selections: None,
        },
        "matching" | "matching_information" | "classification" => {
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

const QUESTION_RANGE_EXPANDED_FROM_SOURCE_ANCHORS: &str =
    "QUESTION_RANGE_EXPANDED_FROM_SOURCE_ANCHORS";

/// Reconcile a printed range heading with the concrete numbered stems that
/// were physically retained in the same bounded question group. Some source
/// papers contain an off-by-one heading (for example `Questions 24-26`
/// immediately followed by stems 23, 24, 25 and 26). Dropping the concrete
/// stem is worse than preserving the typo: expand only when the source anchors
/// form an unbroken run through the declared range boundary.
///
/// Requiring at least two in-range anchors and a contiguous boundary run keeps
/// ordinary numbered passage prose from expanding a group merely because it
/// happens to occur in a recovery window.
fn reconcile_dynamic_group_range_with_source_anchors(
    candidate: &mut SplitGroupCandidateV1,
    blocks: &[Value],
) {
    let [declared_start, declared_end] = candidate.question_range;
    if declared_start == 0 || declared_end < declared_start {
        return;
    }

    let mut anchors = candidate
        .block_ids
        .iter()
        .filter_map(|id| blocks.iter().find(|block| dynamic_block_id(block) == *id))
        .filter(|block| dynamic_block_role(block) != "passage")
        .filter_map(|block| dynamic_leading_question_number(&dynamic_block_text(block)))
        .collect::<Vec<_>>();
    anchors.sort_unstable();
    anchors.dedup();

    let in_range_count = anchors
        .iter()
        .filter(|number| **number >= declared_start && **number <= declared_end)
        .count();
    if in_range_count < 2 {
        return;
    }

    let mut expanded_start = declared_start;
    if anchors.binary_search(&declared_start).is_ok() {
        while expanded_start > 1 && anchors.binary_search(&(expanded_start - 1)).is_ok() {
            expanded_start -= 1;
        }
    }
    let mut expanded_end = declared_end;
    if anchors.binary_search(&declared_end).is_ok() {
        while expanded_end < 40 && anchors.binary_search(&(expanded_end + 1)).is_ok() {
            expanded_end += 1;
        }
    }
    if expanded_start == declared_start && expanded_end == declared_end {
        return;
    }

    candidate.question_range = [expanded_start, expanded_end];
    candidate.heading = dynamic_question_heading(expanded_start, expanded_end);
    if let Some(classification) = candidate.classification.as_mut() {
        if !classification
            .warnings
            .iter()
            .any(|warning| warning == QUESTION_RANGE_EXPANDED_FROM_SOURCE_ANCHORS)
        {
            classification
                .warnings
                .push(QUESTION_RANGE_EXPANDED_FROM_SOURCE_ANCHORS.to_string());
        }
    }
}

fn normalize_dynamic_group_ranges(groups: &mut Vec<SplitGroupCandidateV1>, blocks: &[Value]) {
    for candidate in groups.iter_mut() {
        reconcile_dynamic_group_range_with_source_anchors(candidate, blocks);
    }
    groups.sort_by_key(|candidate| candidate.question_range[0]);
    let candidates = std::mem::take(groups);
    let mut normalized: Vec<SplitGroupCandidateV1> = Vec::with_capacity(candidates.len());
    let mut previous_end = 0u32;
    for mut candidate in candidates {
        let [start, end] = candidate.question_range;
        if end <= previous_end {
            continue;
        }
        if start <= previous_end && end > previous_end {
            // A malformed source can print the same number at the end of one
            // block and the start of the next group (for example a YES/NO
            // group headed 32–36 followed by a matching group headed 36–40).
            // Prefer the later group when the earlier candidate has no
            // source-backed leading marker for the overlap number. This keeps
            // the concrete stem/option block attached to the right task
            // instead of manufacturing an empty question in the first group.
            let overlap = start;
            let earlier_has_overlap = normalized
                .last()
                .map(|previous| {
                    previous
                        .block_ids
                        .iter()
                        .filter_map(|id| blocks.iter().find(|block| dynamic_block_id(block) == *id))
                        .any(|block| {
                            let text = dynamic_block_text(block);
                            dynamic_leading_question_number(&text) == Some(overlap)
                                || find_dynamic_number_marker(&text, overlap, 0).is_some()
                        })
                })
                .unwrap_or(false);
            if !earlier_has_overlap && start > 0 {
                if let Some(previous) = normalized.last_mut() {
                    previous.question_range[1] = start - 1;
                    previous.heading = dynamic_question_heading(
                        previous.question_range[0],
                        previous.question_range[1],
                    );
                }
                previous_end = start - 1;
            } else {
                let adjusted_start = previous_end + 1;
                candidate.question_range = [adjusted_start, end];
                candidate.heading = dynamic_question_heading(adjusted_start, end);
            }
        }
        previous_end = previous_end.max(candidate.question_range[1]);
        normalized.push(candidate);
    }
    for (index, candidate) in normalized.iter_mut().enumerate() {
        candidate.group_id = format!("group-{}", index + 1);
    }
    *groups = normalized;
}

/// Choice prompts and their declared option run are sometimes omitted from a
/// split candidate even though they sit between two retained numbered
/// questions. Reattach only a locally proven, contiguous run and the wrapped
/// prompt lines between its owning question marker and A. This covers both a
/// group tail and an interior A-C + trailing-D layout without sweeping in
/// unrelated passage prose.
fn extend_dynamic_choice_option_blocks(groups: &mut [SplitGroupCandidateV1], blocks: &[Value]) {
    for group_index in 0..groups.len() {
        let kind = groups[group_index].kind_hint.as_str();
        if !matches!(kind, "single_choice" | "multi_choice") {
            continue;
        }
        let mut group_positions = groups[group_index]
            .block_ids
            .iter()
            .filter_map(|id| {
                blocks
                    .iter()
                    .position(|block| dynamic_block_id(block) == *id)
            })
            .collect::<Vec<_>>();
        group_positions.sort_unstable();
        let (Some(group_start), Some(group_end)) = (
            group_positions.first().copied(),
            group_positions.last().copied(),
        ) else {
            continue;
        };
        let boundary = groups
            .iter()
            .skip(group_index + 1)
            .filter_map(|candidate| candidate.block_ids.first())
            .filter_map(|id| {
                blocks
                    .iter()
                    .position(|block| dynamic_block_id(block) == *id)
            })
            .min()
            .unwrap_or(blocks.len());
        let scan_end = boundary.min(group_end.saturating_add(18)).min(blocks.len());
        let [range_start, range_end] = groups[group_index].question_range;
        let mut recovered_indices = std::collections::BTreeSet::new();
        for target_index in group_start..scan_end {
            let Some((option_start, option_end)) =
                dynamic_choice_option_run_bounds(blocks, target_index)
            else {
                continue;
            };
            if option_start < group_start || option_end > scan_end {
                continue;
            }
            let question_index =
                (option_start.saturating_sub(8)..option_start)
                    .rev()
                    .find(|index| {
                        let block_id = dynamic_block_id(&blocks[*index]);
                        groups[group_index]
                            .block_ids
                            .iter()
                            .any(|id| id == &block_id)
                            && dynamic_leading_question_number(&dynamic_block_text(&blocks[*index]))
                                .is_some_and(|number| (range_start..=range_end).contains(&number))
                    });
            let Some(question_index) = question_index else {
                continue;
            };
            recovered_indices.extend(question_index + 1..option_end);
        }
        // A declared terminal option may itself wrap after its label block.
        // The normal run detector proves A..terminal closure and deliberately
        // stops at the terminal label, so retain only source-contiguous,
        // same-layout continuation blocks that immediately follow it.
        let declaration_text = std::iter::once(groups[group_index].instruction_text.clone())
            .chain(groups[group_index].block_ids.iter().filter_map(|id| {
                blocks
                    .iter()
                    .find(|block| dynamic_block_id(block) == *id)
                    .map(dynamic_block_text)
            }))
            .collect::<Vec<_>>()
            .join(" ");
        let declared_terminal = dynamic_letter_options_for_text(&declaration_text)
            .last()
            .and_then(|label| label.chars().next());
        if let Some(terminal) = declared_terminal {
            let is_owned = |index: usize, recovered: &std::collections::BTreeSet<usize>| {
                recovered.contains(&index)
                    || groups[group_index]
                        .block_ids
                        .iter()
                        .any(|id| id == &dynamic_block_id(&blocks[index]))
            };
            if let Some(terminal_index) = (group_start..scan_end).rev().find(|index| {
                is_owned(*index, &recovered_indices)
                    && dynamic_leading_option_label_and_text(&dynamic_block_text(&blocks[*index]))
                        .and_then(|(label, _)| label.chars().next())
                        == Some(terminal)
            }) {
                let mut previous_index = terminal_index;
                let mut continuation_index = terminal_index + 1;
                while continuation_index < scan_end {
                    let continuation = &blocks[continuation_index];
                    let text = dynamic_block_text(continuation);
                    if dynamic_leading_option_label_and_text(&text).is_some()
                        || dynamic_leading_question_number(&text).is_some()
                        || is_dynamic_instruction_signal(&text)
                        || is_dynamic_prompt_terminal_heading(&text)
                        || !(is_dynamic_same_row_option_continuation(
                            &blocks[previous_index],
                            continuation,
                        ) || is_dynamic_wrapped_option_continuation(
                            &blocks[previous_index],
                            continuation,
                        ))
                    {
                        break;
                    }
                    recovered_indices.insert(continuation_index);
                    previous_index = continuation_index;
                    continuation_index += 1;
                }
            }
        }
        for index in recovered_indices {
            let block_id = dynamic_block_id(&blocks[index]);
            if !block_id.is_empty()
                && !groups[group_index]
                    .block_ids
                    .iter()
                    .any(|id| id == &block_id)
            {
                groups[group_index].block_ids.push(block_id);
            }
        }
        groups[group_index].block_ids.sort_by_key(|id| {
            blocks
                .iter()
                .position(|block| dynamic_block_id(block) == *id)
                .unwrap_or(usize::MAX)
        });
        groups[group_index].block_ids.dedup();
    }
}

fn dynamic_inline_option_labels(text: &str, first_label: char) -> Vec<char> {
    dynamic_inline_choice_parts_from_label_with_minimum(text, first_label, 2)
        .map(|(_, options)| {
            options
                .iter()
                .filter_map(|(label, _)| label.chars().next())
                .collect()
        })
        .unwrap_or_else(|| vec![first_label])
}

fn dynamic_bank_candidate_labels_with_prefix_limit(
    text: &str,
    terminal: char,
    prefix_word_limit: usize,
) -> Vec<char> {
    let normalized = collapse_whitespace(text);
    ('A'..=terminal)
        .filter_map(|label| {
            let (start, content_start) = find_dynamic_option_marker(&normalized, label, 0)?;
            let option_text = normalized[content_start..].trim();
            if option_text.is_empty() {
                return None;
            }
            let prefix = normalized[..start].trim();
            let prefix_is_fragment = prefix.is_empty()
                || (prefix.split_whitespace().count() <= prefix_word_limit
                    && prefix
                        .split_whitespace()
                        .all(|word| word.chars().all(|ch| ch.is_ascii_alphabetic()))
                    && prefix
                        .chars()
                        .next()
                        .is_some_and(|ch| ch.is_ascii_lowercase()));
            prefix_is_fragment.then_some(label)
        })
        .collect()
}

fn dynamic_bank_candidate_labels(text: &str, terminal: char) -> Vec<char> {
    dynamic_bank_candidate_labels_with_prefix_limit(text, terminal, 1)
}

fn dynamic_completion_bank_candidate_labels(text: &str, terminal: char) -> Vec<char> {
    let normalized = collapse_whitespace(text);
    ('A'..=terminal)
        .filter_map(|label| {
            let (start, content_start) = find_dynamic_option_marker(&normalized, label, 0)?;
            if normalized[content_start..].trim().is_empty() {
                return None;
            }
            let prefix = normalized[..start].trim();
            let prefix_is_fragment = prefix.is_empty()
                || (prefix.split_whitespace().count() <= 3
                    && prefix.split_whitespace().all(|word| {
                        word.chars().all(|ch| ch.is_ascii_lowercase())
                            || word.chars().all(|ch| ch.is_ascii_digit())
                    }));
            prefix_is_fragment.then_some(label)
        })
        .collect()
}

/// Reattach a declared matching bank that geometry places above the next
/// question heading even when the parser's linear block order interleaves the
/// heading first. This occurs in two-column layouts such as an A/B row with a
/// wrapped C option in the right column.
fn extend_dynamic_matching_option_blocks(groups: &mut [SplitGroupCandidateV1], blocks: &[Value]) {
    for group_index in 0..groups.len() {
        let kind = groups[group_index].kind_hint.as_str();
        if !matches!(
            kind,
            "matching"
                | "matching_information"
                | "classification"
                | "summary_completion"
                | "sentence_completion"
                | "table_completion"
                | "diagram_completion"
        ) {
            continue;
        }
        let group_text = groups[group_index]
            .block_ids
            .iter()
            .filter_map(|id| blocks.iter().find(|block| dynamic_block_id(block) == *id))
            .map(dynamic_block_text)
            .chain(std::iter::once(
                groups[group_index].instruction_text.clone(),
            ))
            .collect::<Vec<_>>()
            .join(" ");
        let separate_bank_expected = matches!(kind, "matching" | "classification")
            || (kind == "matching_information"
                && is_dynamic_prompt_option_bank_heading(&group_text))
            || (is_dynamic_completion_kind(kind)
                && has_dynamic_completion_option_bank_cue(&group_text));
        let terminal = groups[group_index]
            .classification
            .as_ref()
            .and_then(|classification| classification.interaction.options.last())
            .and_then(|label| label.chars().next())
            .or_else(|| {
                dynamic_declared_letter_bank_labels(&group_text)
                    .last()
                    .and_then(|label| label.chars().next())
            })
            .filter(|label| matches!(label, 'C'..=DYNAMIC_MAX_OPTION_LABEL));
        let Some(terminal) = terminal else {
            continue;
        };

        // A multi-column PDF can place a missing bank row after the next
        // question heading in parser order. Recover only labels that are
        // declared by this group and lie in the same visual row neighborhood
        // as an already attached bank row.
        if separate_bank_expected && is_dynamic_completion_kind(kind) {
            let group_values = groups[group_index]
                .block_ids
                .iter()
                .filter_map(|id| blocks.iter().find(|block| dynamic_block_id(block) == *id))
                .cloned()
                .collect::<Vec<_>>();
            let observed = dynamic_group_option_bank(&group_values, "matching")
                .into_iter()
                .filter_map(|(label, _)| label.chars().next())
                .collect::<std::collections::BTreeSet<_>>();
            let missing = ('A'..=terminal)
                .filter(|label| !observed.contains(label))
                .collect::<std::collections::BTreeSet<_>>();
            if !missing.is_empty() {
                let page_indices = group_values
                    .iter()
                    .map(dynamic_block_page_index)
                    .collect::<std::collections::BTreeSet<_>>();
                let anchor_bboxes = group_values
                    .iter()
                    .filter(|block| {
                        !dynamic_completion_bank_candidate_labels(
                            &dynamic_block_text(block),
                            terminal,
                        )
                        .is_empty()
                            || !dynamic_table_option_rows(block).is_empty()
                    })
                    .filter_map(dynamic_block_normalized_bbox)
                    .collect::<Vec<_>>();
                let group_ids = groups[group_index]
                    .block_ids
                    .iter()
                    .collect::<std::collections::HashSet<_>>();
                let mut recovered = Vec::new();
                for (index, block) in blocks.iter().enumerate() {
                    let block_id = dynamic_block_id(block);
                    if block_id.is_empty()
                        || group_ids.contains(&block_id)
                        || !page_indices.contains(&dynamic_block_page_index(block))
                        || detect_dynamic_question_heading_range(&dynamic_block_text(block))
                            .is_some()
                        || dynamic_leading_question_number(&dynamic_block_text(block)).is_some()
                        || is_dynamic_answer_block(block)
                        || is_dynamic_prompt_terminal_heading(&dynamic_block_text(block))
                    {
                        continue;
                    }
                    let labels = dynamic_completion_bank_candidate_labels(
                        &dynamic_block_text(block),
                        terminal,
                    );
                    if !labels.iter().any(|label| missing.contains(label)) {
                        continue;
                    }
                    let Some(candidate_bbox) = dynamic_block_normalized_bbox(block) else {
                        continue;
                    };
                    let near_anchor = anchor_bboxes.iter().any(|anchor| {
                        let vertical_distance = if candidate_bbox[3] < anchor[1] {
                            anchor[1] - candidate_bbox[3]
                        } else if anchor[3] < candidate_bbox[1] {
                            candidate_bbox[1] - anchor[3]
                        } else {
                            0.0
                        };
                        vertical_distance <= 54.0
                    });
                    if near_anchor {
                        recovered.push(index);
                    }
                }
                for index in recovered {
                    let block_id = dynamic_block_id(&blocks[index]);
                    if !groups[group_index]
                        .block_ids
                        .iter()
                        .any(|id| id == &block_id)
                    {
                        groups[group_index].block_ids.push(block_id);
                    }
                }
                groups[group_index].block_ids.sort_by_key(|id| {
                    blocks
                        .iter()
                        .position(|block| dynamic_block_id(block) == *id)
                        .unwrap_or(usize::MAX)
                });
                groups[group_index].block_ids.dedup();
            }
        }
        let mut last_label = None;
        let mut anchor_index = None;
        for block_id in &groups[group_index].block_ids {
            let Some(index) = blocks
                .iter()
                .position(|block| dynamic_block_id(block) == *block_id)
            else {
                continue;
            };
            let text = dynamic_block_text(&blocks[index]);
            let Some((label, _)) = dynamic_leading_option_label_and_text(&text) else {
                continue;
            };
            let Some(first_label) = label.chars().next().filter(|_| label.len() == 1) else {
                continue;
            };
            for recovered in dynamic_inline_option_labels(&text, first_label) {
                if last_label
                    .map(|current| recovered > current)
                    .unwrap_or(true)
                {
                    last_label = Some(recovered);
                    anchor_index = Some(index);
                }
            }
        }
        if (last_label.is_none() || anchor_index.is_none()) && separate_bank_expected {
            // The common layout is numbered prompts followed by a complete
            // `List of ...` bank. Split detection intentionally stops at the
            // last numbered prompt, so graft the following sibling blocks
            // only when they prove the exact declared A-terminal sequence on
            // the same page and before the next group.
            let mut positions = groups[group_index]
                .block_ids
                .iter()
                .filter_map(|id| {
                    blocks
                        .iter()
                        .position(|block| dynamic_block_id(block) == *id)
                })
                .collect::<Vec<_>>();
            positions.sort_unstable();
            let Some(group_end) = positions.last().copied() else {
                continue;
            };
            let page_index = dynamic_block_page_index(&blocks[group_end]);
            let next_group_start = groups
                .iter()
                .skip(group_index + 1)
                .flat_map(|candidate| candidate.block_ids.iter())
                .filter_map(|id| {
                    blocks
                        .iter()
                        .position(|block| dynamic_block_id(block) == *id)
                })
                .filter(|index| *index > group_end)
                .min()
                .unwrap_or(blocks.len());
            let search_end = blocks
                .iter()
                .enumerate()
                .skip(group_end + 1)
                .take_while(|(index, block)| {
                    *index < next_group_start
                        && dynamic_block_page_index(block) == page_index
                        && *index <= group_end.saturating_add(36)
                })
                .map(|(index, _)| index + 1)
                .last()
                .unwrap_or(group_end + 1);
            let mut expected = 'A';
            let mut option_start = None;
            let mut recovered_indices = Vec::new();
            let mut completed = false;
            for index in group_end + 1..search_end {
                let text = dynamic_block_text(&blocks[index]);
                if detect_dynamic_question_heading_range(&text).is_some()
                    || dynamic_leading_question_number(&text).is_some()
                    || is_dynamic_answer_block(&blocks[index])
                {
                    break;
                }
                let leading =
                    dynamic_leading_option_label_and_text(&text).filter(|(label, option_text)| {
                        label.len() == 1 && !option_text.trim().is_empty()
                    });
                if option_start.is_none() {
                    if leading.as_ref().and_then(|(label, _)| label.chars().next()) != Some('A') {
                        continue;
                    }
                    option_start = Some(index);
                }
                if let Some((label, _)) = leading {
                    let Some(first_label) = label.chars().next() else {
                        break;
                    };
                    if first_label != expected {
                        break;
                    }
                    let labels = dynamic_inline_option_labels(&text, first_label);
                    let mut next_expected = expected;
                    let mut valid_sequence = true;
                    for recovered in labels {
                        if recovered != next_expected || recovered > terminal {
                            valid_sequence = false;
                            break;
                        }
                        next_expected = ((recovered as u8).saturating_add(1)) as char;
                    }
                    if !valid_sequence {
                        break;
                    }
                    expected = next_expected;
                    recovered_indices.push(index);
                    if expected > terminal {
                        completed = true;
                        break;
                    }
                } else if option_start.is_some()
                    && !is_dynamic_instruction_signal(&text)
                    && !is_dynamic_prompt_terminal_heading(&text)
                {
                    // A wrapped option row can occupy one or more unlabeled
                    // blocks before the next exact label.
                    recovered_indices.push(index);
                } else {
                    break;
                }
            }
            if completed {
                if let Some(start) = option_start {
                    let header_index = start.checked_sub(1).filter(|index| {
                        is_dynamic_prompt_option_bank_heading(&dynamic_block_text(&blocks[*index]))
                    });
                    if let Some(index) = header_index {
                        recovered_indices.insert(0, index);
                    }
                }
                for index in recovered_indices {
                    let block_id = dynamic_block_id(&blocks[index]);
                    if !block_id.is_empty()
                        && !groups[group_index]
                            .block_ids
                            .iter()
                            .any(|id| id == &block_id)
                    {
                        groups[group_index].block_ids.push(block_id);
                    }
                }
                groups[group_index].block_ids.sort_by_key(|id| {
                    blocks
                        .iter()
                        .position(|block| dynamic_block_id(block) == *id)
                        .unwrap_or(usize::MAX)
                });
                groups[group_index].block_ids.dedup();
            }
            continue;
        }
        let (Some(last_label), Some(anchor_index)) = (last_label, anchor_index) else {
            continue;
        };
        if last_label >= terminal {
            continue;
        }
        let page_index = dynamic_block_page_index(&blocks[anchor_index]);
        let next_heading_bbox = groups
            .iter()
            .skip(group_index + 1)
            .filter_map(|candidate| candidate.block_ids.first())
            .filter_map(|id| blocks.iter().find(|block| dynamic_block_id(block) == *id))
            .find(|block| dynamic_block_page_index(block) == page_index)
            .and_then(dynamic_block_normalized_bbox);
        let mut expected = ((last_label as u8).saturating_add(1)) as char;
        let mut recovered_indices = Vec::new();
        let mut previous_index = anchor_index;
        while expected <= terminal {
            let Some(anchor_bbox) = dynamic_block_normalized_bbox(&blocks[previous_index]) else {
                break;
            };
            let candidate_index = blocks
                .iter()
                .enumerate()
                .filter(|(index, block)| {
                    !recovered_indices.contains(index)
                        && dynamic_block_page_index(block) == page_index
                        && !groups[group_index]
                            .block_ids
                            .iter()
                            .any(|id| id == &dynamic_block_id(block))
                })
                .filter_map(|(index, block)| {
                    let text = dynamic_block_text(block);
                    let (label, option_text) = dynamic_leading_option_label_and_text(&text)?;
                    (label.chars().next() == Some(expected)
                        && label.len() == 1
                        && !option_text.trim().is_empty())
                    .then_some((index, block))
                })
                .filter_map(|(index, block)| {
                    let bbox = dynamic_block_normalized_bbox(block)?;
                    let below_anchor = bbox[3] <= anchor_bbox[1] + 10.0;
                    let above_next = next_heading_bbox
                        .map(|next| bbox[1] >= next[3] - 10.0)
                        .unwrap_or(true);
                    // In multi-column extraction the next label can be in a
                    // different column from the last observed label. Vertical
                    // proximity and the next-heading boundary are stronger
                    // evidence than x alignment, so do not reject a valid
                    // bank row merely because its column changed.
                    let nearby = anchor_bbox[1] - bbox[3] <= 180.0;
                    (below_anchor && above_next && nearby)
                        .then_some((index, anchor_bbox[1] - bbox[3]))
                })
                .min_by(|left, right| left.1.total_cmp(&right.1))
                .map(|(index, _)| index);
            let Some(candidate_index) = candidate_index else {
                break;
            };
            recovered_indices.push(candidate_index);
            previous_index = candidate_index;
            loop {
                let continuation = blocks
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| !recovered_indices.contains(index))
                    .filter(|(_, block)| {
                        is_dynamic_same_row_option_continuation(&blocks[previous_index], block)
                    })
                    .min_by(|(_, left), (_, right)| {
                        dynamic_block_normalized_bbox(left)
                            .map(|bbox| bbox[0])
                            .unwrap_or(f64::MAX)
                            .total_cmp(
                                &dynamic_block_normalized_bbox(right)
                                    .map(|bbox| bbox[0])
                                    .unwrap_or(f64::MAX),
                            )
                    })
                    .map(|(index, _)| index);
                let Some(index) = continuation else {
                    break;
                };
                recovered_indices.push(index);
                previous_index = index;
            }
            expected = ((expected as u8).saturating_add(1)) as char;
        }
        for index in recovered_indices {
            let block_id = dynamic_block_id(&blocks[index]);
            if !block_id.is_empty()
                && !groups[group_index]
                    .block_ids
                    .iter()
                    .any(|id| id == &block_id)
            {
                groups[group_index].block_ids.push(block_id);
            }
        }
        groups[group_index].block_ids.sort_by_key(|id| {
            blocks
                .iter()
                .position(|block| dynamic_block_id(block) == *id)
                .unwrap_or(usize::MAX)
        });
        groups[group_index].block_ids.dedup();
    }
}

fn normalized_answer_value(raw: &str) -> Value {
    let upper = raw.trim().to_uppercase();
    if matches!(
        upper.as_str(),
        "TRUE"
            | "FALSE"
            | "YES"
            | "NO"
            | "NOT GIVEN"
            | "A"
            | "B"
            | "C"
            | "D"
            | "E"
            | "F"
            | "G"
            | "H"
            | "I"
            | "J"
            | "K"
            | "L"
            | "M"
            | "N"
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
            .map(|word| {
                matches!(
                    word.to_lowercase().as_str(),
                    "the"
                        | "a"
                        | "an"
                        | "and"
                        | "but"
                        | "or"
                        | "which"
                        | "that"
                        | "this"
                        | "these"
                        | "those"
                        | "in"
                        | "on"
                        | "for"
                        | "with"
                        | "as"
                        | "by"
                        | "from"
                        | "to"
                        | "at"
                )
            })
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
                    json!(crate::parser::markdownish_to_html_pub(
                        &merged_text,
                        block_type
                    )),
                );
                // Extend the bbox to cover both blocks so downstream geometry
                // consumers still see a sensible envelope.
                if let (Some(prev_bbox), Some(next_bbox)) = (
                    obj.get("bbox").and_then(Value::as_array),
                    block.get("bbox").and_then(Value::as_array),
                ) {
                    if prev_bbox.len() == 4 && next_bbox.len() == 4 {
                        let merged_bbox = vec![
                            json!(prev_bbox[0]
                                .as_f64()
                                .unwrap_or(0.0)
                                .min(next_bbox[0].as_f64().unwrap_or(0.0))),
                            json!(prev_bbox[1]
                                .as_f64()
                                .unwrap_or(0.0)
                                .min(next_bbox[1].as_f64().unwrap_or(0.0))),
                            json!(prev_bbox[2]
                                .as_f64()
                                .unwrap_or(0.0)
                                .max(next_bbox[2].as_f64().unwrap_or(0.0))),
                            json!(prev_bbox[3]
                                .as_f64()
                                .unwrap_or(0.0)
                                .max(next_bbox[3].as_f64().unwrap_or(0.0))),
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
        || lower.starts_with("match each ")
        || lower.starts_with("match the ")
        || ((lower.starts_with("nb ") || lower.starts_with("note "))
            && (lower.contains("use any letter")
                || lower.contains("use each letter")
                || lower.contains("more than once")))
        || is_dynamic_response_legend_text(&lower)
        || lower.contains("which two")
        || lower.contains("which three")
        || lower.contains("answer sheet")
        || lower.contains("______")
        || lower.contains("_____")
}

fn is_dynamic_response_legend_text(text: &str) -> bool {
    let lower = collapse_whitespace(text)
        .trim_matches(|ch: char| {
            matches!(ch, ':' | ';' | ',' | '.' | '-' | '\u{2013}' | '\u{2014}')
        })
        .to_lowercase();
    if matches!(
        lower.as_str(),
        "true" | "false" | "yes" | "no" | "not given"
    ) {
        return true;
    }

    let response_labels = ["true ", "false ", "yes ", "no ", "not given "];
    let has_response_label = response_labels
        .iter()
        .any(|prefix| lower.starts_with(*prefix));
    // A split response legend can begin with `if`, but ordinary passage prose
    // commonly does too. A comma followed by more prose is strong evidence
    // that this is a sentence rather than a compact IELTS legend definition.
    if !has_response_label && lower.contains(',') {
        return false;
    }

    let definition = response_labels
        .iter()
        .find(|prefix| lower.starts_with(**prefix))
        .and_then(|_| lower.find(" if ").map(|index| &lower[index + 1..]))
        .unwrap_or(lower.as_str());
    let normalized_definition = definition.replace("im possible", "impossible");

    [
        "if the statement agrees with the information",
        "if the statement contradicts the information",
        "if the statement agrees with the claims",
        "if the statement contradicts the claims",
        "if the statement agrees with the views",
        "if the statement contradicts the views",
        "if there is no information on this",
        "if there is no information about this",
        "if there is no information given",
    ]
    .iter()
    .any(|canonical| normalized_definition.starts_with(canonical))
        || (normalized_definition.starts_with("if it is impossible to say")
            && (normalized_definition.contains("writer thinks")
                || normalized_definition.contains("writer's opinion")
                || normalized_definition.contains("author thinks")
                || normalized_definition.contains("author's opinion")))
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
            marker.len() == 1 && marker.chars().all(is_dynamic_letter_option_label)
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

fn dynamic_choice_option_run_bounds(
    blocks: &[Value],
    target_index: usize,
) -> Option<(usize, usize)> {
    dynamic_choice_option_run_bounds_with_shared_context(blocks, target_index, false)
}

fn dynamic_choice_option_run_bounds_with_shared_context(
    blocks: &[Value],
    target_index: usize,
    allow_shared_choice_context: bool,
) -> Option<(usize, usize)> {
    // Include the opening A block when checking the final J option in an
    // A-J run (nine intervening labels, plus a little layout slack).
    let search_start = target_index.saturating_sub(10);
    for start in search_start..=target_index.min(blocks.len().saturating_sub(1)) {
        if dynamic_lettered_paragraph_label(&dynamic_block_text(&blocks[start])) != Some('A') {
            continue;
        }
        // A wrapped question stem can occupy several physical blocks between
        // the numbered marker and its A/B/C run (for example, a long prompt
        // followed by a continuation line and then the first option). Keep a
        // bounded look-back, but do not require the marker to be in the last
        // three blocks; that was enough to misclassify the option tail as
        // passage prose.
        let has_preceding_question = blocks[..start]
            .iter()
            .rev()
            .take(8)
            .any(|block| dynamic_leading_question_number(&dynamic_block_text(block)).is_some());
        let has_shared_choice_instruction = allow_shared_choice_context
            && blocks[..start].iter().rev().take(12).any(|block| {
                let lower = normalized_dynamic_instruction_text(&dynamic_block_text(block));
                lower.contains("choose the correct")
                    || lower.contains("choose two")
                    || lower.contains("choose three")
            });
        if !has_preceding_question && !has_shared_choice_instruction {
            continue;
        }
        let mut expected = 'A';
        let mut end = start;
        let mut label_count = 0usize;
        while end < blocks.len() {
            let option_text = dynamic_block_text(&blocks[end]);
            if dynamic_lettered_paragraph_label(&option_text) == Some(expected) {
                // pdfium can flatten several labels into one physical line:
                // `B ... C ... D ...`. Count the whole contiguous sequence,
                // not just the leading B, so the split layer retains the
                // block instead of misclassifying it as passage prose.
                let inline_labels =
                    dynamic_inline_choice_parts_from_label_with_minimum(&option_text, expected, 2)
                        .map(|(_, options)| options)
                        .unwrap_or_else(|| vec![(expected.to_string(), String::new())]);
                label_count += inline_labels.len();
                expected = inline_labels
                    .last()
                    .and_then(|(label, _)| label.chars().next())
                    .map(|label| ((label as u8).saturating_add(1)) as char)
                    .unwrap_or_else(|| ((expected as u8).saturating_add(1)) as char);
                end += 1;
                if expected > DYNAMIC_MAX_OPTION_LABEL {
                    break;
                }
                continue;
            }

            // A wrapped choice can put the remainder of a long B/C option in
            // its own paragraph. Tolerate one such continuation only when the
            // immediately following block resumes the exact A-J sequence.
            // The preceding numbered question and the three-label minimum
            // below keep ordinary lettered article paragraphs out of this
            // narrow exception.
            let continuation_is_safe = label_count > 0
                && end + 1 < blocks.len()
                && dynamic_leading_question_number(&dynamic_block_text(&blocks[end])).is_none()
                && !is_dynamic_question_block(&blocks[end])
                && !is_dynamic_answer_block(&blocks[end])
                && !is_dynamic_instruction_signal(&dynamic_block_text(&blocks[end]))
                && dynamic_lettered_paragraph_label(&dynamic_block_text(&blocks[end + 1]))
                    == Some(expected);
            if continuation_is_safe {
                end += 1;
                continue;
            }
            break;
        }
        if label_count >= 3 && (start..end).contains(&target_index) {
            return Some((start, end));
        }
    }
    None
}

fn dynamic_last_choice_option_run_bounds(blocks: &[Value]) -> Option<(usize, usize)> {
    (0..blocks.len())
        .filter_map(|index| {
            dynamic_choice_option_run_bounds_with_shared_context(blocks, index, true)
        })
        .max_by_key(|(_, end)| *end)
}

fn is_dynamic_late_passage_tail_start(blocks: &[Value], index: usize) -> bool {
    let Some(block) = blocks.get(index) else {
        return false;
    };
    let text = collapse_whitespace(&dynamic_block_text(block));
    if text.is_empty()
        || is_dynamic_question_block(block)
        || is_dynamic_answer_block(block)
        || dynamic_leading_question_number(&text).is_some()
        || is_dynamic_heading_option_line(&text)
        || is_dynamic_heading_matching_instruction_line(&text)
        || is_dynamic_heading_matching_assignment_line(&text)
        || is_dynamic_non_content_placeholder_text(&text)
    {
        return false;
    }
    if dynamic_choice_option_run_bounds(blocks, index).is_some() {
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

fn dynamic_leading_question_marker(text: &str) -> Option<(u32, usize)> {
    let normalized = collapse_whitespace(text);
    let first = normalized.split_whitespace().next()?;
    let marker_start = first
        .chars()
        .next()
        .filter(|ch| matches!(ch, '(' | '['))
        .map(char::len_utf8)
        .unwrap_or(0);
    let trimmed = &first[marker_start..];
    let digits_end = trimmed
        .char_indices()
        .find_map(|(index, ch)| (!ch.is_ascii_digit()).then_some(index))
        .unwrap_or(trimmed.len());
    if digits_end == 0 || digits_end > 3 {
        return None;
    }
    let value = trimmed[..digits_end].parse::<u32>().ok()?;
    if value > 40 {
        return None;
    }
    let marker_end = marker_start + digits_end;
    let suffix = &normalized[marker_end..];
    let valid_boundary = suffix
        .chars()
        .next()
        .map(|ch| {
            ch.is_whitespace()
                || matches!(ch, '.' | ')' | ':' | ';' | ',' | ']' | '、')
                // Some rotated/text-matrix PDFs lose the visual gap between
                // the question number and an uppercase opening word:
                // `14 High-level ...` becomes `14High-level ...`.
                || ch.is_ascii_uppercase()
        })
        .unwrap_or(true);
    valid_boundary.then_some((value, marker_end))
}

fn dynamic_leading_question_number(text: &str) -> Option<u32> {
    dynamic_leading_question_marker(text).map(|(number, _)| number)
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
    if dynamic_choice_option_run_bounds(blocks, index).is_some() {
        return None;
    }
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

/// Completion stimuli are prose-shaped by design. Only treat an apparent
/// prose run as interleaved passage when it begins in a genuinely different
/// page/layout section from the preceding question content. Moving from the
/// left column to the right column inside the same section is normal reading
/// flow and must not discard wrapped text between numbered gaps.
fn collect_dynamic_completion_interleaved_passage_runs(blocks: &[Value]) -> Vec<(usize, usize)> {
    collect_dynamic_interleaved_passage_runs(blocks)
        .into_iter()
        .filter(|(run_start, _)| {
            let Some(previous) = run_start.checked_sub(1).and_then(|index| blocks.get(index))
            else {
                return false;
            };
            let Some(first) = blocks.get(*run_start) else {
                return false;
            };
            dynamic_block_page_index(previous) != dynamic_block_page_index(first)
                || dynamic_block_layout_section_index(previous)
                    != dynamic_block_layout_section_index(first)
                || dynamic_block_section_column_count_value(previous)
                    != dynamic_block_section_column_count_value(first)
        })
        .collect()
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
        || dynamic_leading_question_number(&text).is_some()
        || is_dynamic_heading_option_line(&text)
        || is_dynamic_heading_matching_instruction_line(&text)
        || is_dynamic_heading_matching_assignment_line(&text)
    {
        return false;
    }
    if dynamic_choice_option_run_bounds(blocks, index).is_some() {
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

fn dynamic_completion_letter_bank_task_end(kind: &str, blocks: &[Value]) -> Option<usize> {
    dynamic_declared_completion_bank_span(kind, blocks)
        .map(|(_, end)| end)
        .or_else(|| dynamic_partial_completion_bank_task_end(kind, blocks))
}

fn dynamic_completion_text_has_sentence_end_after(text: &str, from: usize) -> bool {
    text.get(from.min(text.len())..).is_some_and(|tail| {
        tail.chars()
            .any(|ch| matches!(ch, '.' | '!' | '?' | '\u{2022}'))
    })
}

fn is_dynamic_local_completion_continuation(previous: &Value, current: &Value) -> bool {
    if dynamic_block_page_index(previous) != dynamic_block_page_index(current)
        || dynamic_block_layout_section_index(previous)
            != dynamic_block_layout_section_index(current)
        || dynamic_block_section_column_count_value(previous)
            != dynamic_block_section_column_count_value(current)
        || dynamic_block_column(previous) != dynamic_block_column(current)
    {
        return false;
    }

    let current_role = dynamic_block_role(current);
    if matches!(current_role, "passage" | "answer" | "ignore")
        || is_dynamic_answer_block(current)
        || is_dynamic_question_heading_text(&dynamic_block_text(current))
    {
        return false;
    }

    let (Some(previous_bbox), Some(current_bbox)) = (
        dynamic_block_normalized_bbox(previous),
        dynamic_block_normalized_bbox(current),
    ) else {
        return false;
    };
    let previous_height = (previous_bbox[3] - previous_bbox[1]).abs().max(1.0);
    let current_height = (current_bbox[3] - current_bbox[1]).abs().max(1.0);
    let vertical_gap = previous_bbox[1] - current_bbox[3];
    vertical_gap >= -2.0 && vertical_gap <= (previous_height.max(current_height) * 2.2).max(20.0)
}

/// Close a linear completion task at the block containing its declared final
/// numbered blank. A wrapped sentence may consume nearby blocks, but only
/// inside the same physical page/layout stream. Page, column, section and role
/// transitions are ownership boundaries: when the source is ambiguous we keep
/// the later prose in the passage instead of fabricating a very long final
/// prompt.
fn dynamic_completion_final_gap_task_end(kind: &str, blocks: &[Value]) -> Option<usize> {
    if !is_dynamic_completion_kind(kind) {
        return None;
    }
    let (_, final_number) = blocks
        .iter()
        .find_map(|block| detect_dynamic_question_heading_range(&dynamic_block_text(block)))?;
    let (marker_index, marker_end) = blocks.iter().enumerate().find_map(|(index, block)| {
        find_dynamic_numbered_blank_marker(&dynamic_block_text(block), final_number, 0)
            .map(|(_, marker_end)| (index, marker_end))
    })?;

    let marker_text = dynamic_block_text(&blocks[marker_index]);
    let mut task_end = marker_index + 1;
    if dynamic_completion_text_has_sentence_end_after(&marker_text, marker_end) {
        return Some(task_end);
    }

    let mut previous_index = marker_index;
    while let Some(current) = blocks.get(task_end) {
        if !is_dynamic_local_completion_continuation(&blocks[previous_index], current) {
            break;
        }
        let current_text = dynamic_block_text(current);
        // A new numbered gap is a new logical item, never continuation prose
        // for the declared final slot even when malformed source ranges overlap.
        if (1..=40)
            .any(|number| find_dynamic_numbered_blank_marker(&current_text, number, 0).is_some())
        {
            break;
        }
        task_end += 1;
        previous_index += 1;
        if dynamic_completion_text_has_sentence_end_after(&current_text, 0) {
            break;
        }
    }
    Some(task_end)
}

fn dynamic_question_block_count_for_group(kind: &str, blocks: &[Value]) -> usize {
    if let Some(task_end) = dynamic_completion_letter_bank_task_end(kind, blocks) {
        return task_end.max(1);
    }
    if let Some(task_end) = dynamic_completion_final_gap_task_end(kind, blocks) {
        return task_end.max(1);
    }
    let specific = if kind == "heading_matching" {
        dynamic_heading_matching_question_block_count(blocks)
    } else {
        dynamic_late_passage_question_block_count(blocks)
    };
    let inferred = if specific < blocks.len() {
        specific
    } else {
        dynamic_generic_passage_tail_question_block_count(blocks)
    };
    if matches!(kind, "single_choice" | "multi_choice") {
        dynamic_last_choice_option_run_bounds(blocks)
            .map(|(_, option_end)| inferred.max(option_end))
            .unwrap_or(inferred)
    } else {
        inferred
    }
}

fn dynamic_leading_option_label_and_text(text: &str) -> Option<(String, String)> {
    // Word can emit zero-width format characters around native numbering
    // labels (for example `A.\u{200c}`). They are not content and would
    // otherwise become part of the marker token.
    let normalized = collapse_whitespace(
        &text
            .chars()
            .filter(|ch| {
                !matches!(
                    ch,
                    '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
                )
            })
            .collect::<String>(),
    );
    let first = normalized.split_whitespace().next()?;
    let label = first.trim_matches(|ch: char| {
        matches!(ch, '(' | ')' | '[' | ']' | '.' | ':' | ';' | ',' | '、')
    });
    // IELTS lettered option labels are uppercase. Treating a lowercase
    // one-letter word (for example the split `e` in `Catherin` + `e Price`)
    // as an E option prevents geometric word-fragment recovery and also
    // promotes ordinary prose articles into option banks.
    let is_letter = label.len() == 1 && label.chars().all(is_dynamic_letter_option_label);
    let lower = label.to_ascii_lowercase();
    let is_roman = matches!(
        lower.as_str(),
        "i" | "ii" | "iii" | "iv" | "v" | "vi" | "vii" | "viii" | "ix" | "x" | "xi" | "xii"
    );
    if !is_letter && !is_roman {
        return None;
    }
    let content = normalized[first.len()..]
        .trim_start_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, ')' | ']' | '.' | ':' | ';' | ',' | '、')
        })
        .trim()
        .to_string();
    let normalized_label = if is_roman && label.chars().any(|ch| ch.is_ascii_lowercase()) {
        lower
    } else {
        label.to_ascii_uppercase()
    };
    Some((normalized_label, content))
}

fn dynamic_table_option_rows(block: &Value) -> Vec<(String, String)> {
    let Some(cells) = block.pointer("/table/cells").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut rows = std::collections::BTreeMap::<u64, Vec<(u64, String)>>::new();
    for cell in cells {
        let Some(row) = cell.get("row").and_then(Value::as_u64) else {
            continue;
        };
        let col = cell.get("col").and_then(Value::as_u64).unwrap_or(0);
        let text = cell.get("text").and_then(Value::as_str).unwrap_or_default();
        if !text.trim().is_empty() {
            rows.entry(row).or_default().push((col, text.to_string()));
        }
    }

    for row in rows.values_mut() {
        row.sort_by_key(|(col, _)| *col);
    }

    // Some Word tables keep the first option label in its own cell while the
    // remaining labels stay embedded in the adjacent value cell, for example
    // `A | natural evolution B creative thought C indigenous plants D trout`.
    // Parse the complete table before accepting each dedicated label row as a
    // single option. The shared inline parser requires a consecutive run of at
    // least three labels and rejects duplicate expected labels, which keeps
    // ordinary values such as `Type B vitamin deficiency` intact.
    let table_text = rows
        .values()
        .map(|row| {
            row.iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" ");
    if let Some((prompt, inline_options)) = dynamic_inline_choice_parts(&table_text) {
        if prompt.is_empty() {
            return inline_options;
        }
    }

    let mut options = Vec::<(String, String)>::new();
    for row in rows.into_values() {
        // A dedicated label cell is stronger layout evidence than a bare
        // capital letter in the value cell. Keep the complete value intact:
        // phrases such as `Type B vitamin deficiency`, `vitamin B`, and
        // `Section B` are ordinary option text, not hidden option markers.
        let independent_label = row.first().and_then(|(_, text)| {
            dynamic_leading_option_label_and_text(text)
                .filter(|(_, content)| content.is_empty())
                .map(|(label, _)| label)
        });
        if let Some(label) = independent_label.filter(|_| row.len() >= 2) {
            let option_text = collapse_whitespace(
                &row.iter()
                    .skip(1)
                    .map(|(_, text)| text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            if !option_text.is_empty()
                && !options
                    .iter()
                    .any(|(existing_label, _)| existing_label == &label)
            {
                options.push((label, option_text));
            }
            continue;
        }

        let row_text = row
            .into_iter()
            .map(|(_, text)| text)
            .collect::<Vec<_>>()
            .join(" ");
        let Some((label, option_text)) = dynamic_leading_option_label_and_text(&row_text) else {
            continue;
        };
        if option_text.is_empty() {
            continue;
        }

        let row_options = label
            .chars()
            .next()
            .filter(|_| label.len() == 1)
            .and_then(|first_label| {
                dynamic_inline_choice_parts_from_label(&row_text, first_label)
                    .map(|(_, inline_options)| inline_options)
            })
            .unwrap_or_else(|| vec![(label.clone(), option_text.clone())]);

        for option in row_options {
            if !options
                .iter()
                .any(|(existing_label, _)| existing_label == &option.0)
            {
                options.push(option);
            }
        }
    }
    options
}

fn is_dynamic_instruction_signal(text: &str) -> bool {
    let lower = normalized_dynamic_instruction_text(text);
    is_dynamic_question_heading_text(&lower)
        || lower.contains("do the following statements")
        || lower.contains("choose the correct")
        || lower.contains("choose two")
        || lower.contains("choose three")
        || lower.contains("write the correct")
        || lower.contains("complete the")
        || lower.contains("match each")
        || lower.contains("match the")
        || lower.contains("label the")
        || lower.contains("which paragraph")
        || lower.contains("which section")
        || lower.contains("true false not given")
        || (lower.contains("true") && lower.contains("false") && lower.contains("not given"))
        || (lower.contains("yes") && lower.contains("no") && lower.contains("not given"))
        || lower.contains("no more than")
        || lower.contains("one word only")
        || lower.contains("two words only")
        || lower.contains("three words only")
        || lower.contains("list of headings")
        || lower.contains("list of options")
        || lower.contains("answer the questions")
}

pub(crate) fn has_dynamic_ielts_question_instruction_evidence(text: &str) -> bool {
    let normalized = normalized_dynamic_instruction_text(text);
    let lower = normalized.trim_start_matches('#').trim_start();
    let starts_with = |prefix: &str| lower.starts_with(prefix);
    let contains_any = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));

    (starts_with("choose ")
        && contains_any(&[
            "correct",
            "letter",
            "answer",
            "one word",
            "two words",
            "three words",
            "from the passage",
        ]))
        || (starts_with("complete ")
            && contains_any(&[
                "table",
                "form",
                "notes",
                "summary",
                "sentence",
                "flow chart",
                "flow-chart",
                "diagram",
                "map",
                "plan",
            ]))
        || (starts_with("write ")
            && contains_any(&[
                "correct letter",
                "correct number",
                "no more than",
                "one word only",
                "two words only",
                "three words only",
                "in boxes",
                "answer sheet",
            ]))
        || ((starts_with("match ") || starts_with("label "))
            && contains_any(&[
                "heading",
                "information",
                "statement",
                "feature",
                "paragraph",
                "section",
                "diagram",
                "map",
                "plan",
            ]))
        || starts_with("do the following statements")
        || starts_with("answer the questions")
        || starts_with("which paragraph")
        || starts_with("which section")
        || lower.contains("true false not given")
        || (lower.contains("true") && lower.contains("false") && lower.contains("not given"))
        || (lower.contains("yes") && lower.contains("no") && lower.contains("not given"))
        || lower.contains("list of headings")
}

fn has_dynamic_concrete_question_number_evidence(text: &str) -> bool {
    let Some(number) = dynamic_leading_question_number(text).filter(|number| *number <= 40) else {
        return false;
    };
    let prompt = strip_dynamic_leading_question_marker(text, number);
    if prompt.is_empty() {
        return false;
    }
    let lower = prompt.to_lowercase();
    let first_word = lower
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|ch: char| !ch.is_alphanumeric());
    prompt.contains('?')
        || has_dynamic_numbered_inline_blanks(text)
        || dynamic_inline_choice_parts(&prompt).is_some()
        || matches!(
            first_word,
            "what" | "which" | "who" | "whose" | "when" | "where" | "why" | "how"
        )
}

/// Heading-free numbered material is ambiguous: articles, reports and source
/// passages frequently use consecutive numbered sections.  Treat such a run as
/// questions only when every number has local question syntax and the material
/// between adjacent numbers remains question-sized.  Explicit IELTS range and
/// instruction blocks bypass this inference gate elsewhere.
fn has_credible_heading_free_numbered_questions(blocks: &[Value]) -> bool {
    let markers = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let text = dynamic_block_text(block);
            let number = dynamic_leading_question_number(&text)?;
            (number <= 40).then_some((index, text))
        })
        .collect::<Vec<_>>();
    if markers.len() < 2 {
        return false;
    }
    if !markers
        .iter()
        .all(|(_, text)| has_dynamic_concrete_question_number_evidence(text))
    {
        return false;
    }

    let mut segment_word_counts = markers
        .iter()
        .enumerate()
        .map(|(position, (start, _))| {
            let end = markers
                .get(position + 1)
                .map(|(index, _)| *index)
                .unwrap_or(blocks.len());
            blocks[*start..end]
                .iter()
                .map(dynamic_block_text)
                .map(|text| text.split_whitespace().count())
                .sum::<usize>()
        })
        .collect::<Vec<_>>();
    segment_word_counts.sort_unstable();
    let median_words = segment_word_counts[segment_word_counts.len() / 2];
    let longest_words = segment_word_counts.last().copied().unwrap_or(0);
    median_words <= 64 && longest_words <= 160
}

/// A parser role hint is only a routing hint: older DocumentIR artifacts can
/// contain false `question` roles for ordinary prose using words such as
/// "the author questions whether ...". Before any heading-free fallback can
/// create a question group, require evidence that is specific to an IELTS
/// task independently of that role hint.
fn has_dynamic_fallback_question_group_evidence(blocks: &[Value]) -> bool {
    let has_explicit_evidence = blocks.iter().any(|block| {
        let text = dynamic_block_text(block);
        detect_dynamic_question_heading_range(&text).is_some()
            || has_dynamic_ielts_question_instruction_evidence(&text)
    });
    has_explicit_evidence || has_credible_heading_free_numbered_questions(blocks)
}

fn dynamic_instruction_window_start(blocks: &[Value], question_index: usize) -> usize {
    let mut start = question_index;
    let mut saw_signal = false;
    for index in (question_index.saturating_sub(8)..question_index).rev() {
        let text = dynamic_block_text(&blocks[index]);
        if text.is_empty()
            || is_dynamic_answer_block(&blocks[index])
            || dynamic_leading_question_number(&text).is_some()
            || is_dynamic_reading_passage_heading(&text)
        {
            break;
        }
        let word_count = text.split_whitespace().count();
        let support_line = word_count <= 24
            && (dynamic_leading_option_label_and_text(&text).is_some()
                || !is_substantive_dynamic_passage_block(&blocks[index]));
        if is_dynamic_instruction_signal(&text) {
            saw_signal = true;
            start = index;
        } else if support_line && !saw_signal {
            // Keep looking for the actual instruction line, but do not move
            // the group boundary across option blocks that belong to the
            // preceding numbered question.
        } else {
            break;
        }
    }
    if saw_signal {
        start
    } else {
        question_index
    }
}

#[derive(Debug, Clone, Copy)]
struct DynamicNumberedGroupSpan {
    instruction_start: usize,
    first_question_index: usize,
    last_question_index: usize,
    start: u32,
    end: u32,
}

fn dynamic_numbered_group_spans(blocks: &[Value]) -> Vec<DynamicNumberedGroupSpan> {
    let markers = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            if is_dynamic_answer_block(block) {
                return None;
            }
            let number = dynamic_leading_question_number(&dynamic_block_text(block))?;
            (number <= 40).then_some((index, number))
        })
        .collect::<Vec<_>>();
    if markers.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::<(usize, usize)>::new();
    let mut start_position = 0usize;
    for position in 1..markers.len() {
        let (previous_index, previous_number) = markers[position - 1];
        let (current_index, current_number) = markers[position];
        let has_new_instruction = blocks[previous_index + 1..current_index]
            .iter()
            .any(|block| is_dynamic_instruction_signal(&dynamic_block_text(block)));
        if current_number != previous_number.saturating_add(1) || has_new_instruction {
            ranges.push((start_position, position - 1));
            start_position = position;
        }
    }
    ranges.push((start_position, markers.len() - 1));

    ranges
        .into_iter()
        .filter_map(|(first_position, last_position)| {
            let (first_question_index, start) = markers[first_position];
            let (last_question_index, end) = markers[last_position];
            let instruction_start = dynamic_instruction_window_start(blocks, first_question_index);
            let marker_count = last_position - first_position + 1;
            (marker_count >= 2 || instruction_start < first_question_index).then_some(
                DynamicNumberedGroupSpan {
                    instruction_start,
                    first_question_index,
                    last_question_index,
                    start,
                    end,
                },
            )
        })
        .collect()
}

fn dynamic_instruction_text_from_blocks(blocks: &[Value], first_question_index: usize) -> String {
    let text = blocks[..first_question_index.min(blocks.len())]
        .iter()
        .map(dynamic_block_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    collapse_whitespace(&text)
}

fn make_dynamic_numbered_group_candidates(blocks: &[Value]) -> Vec<SplitGroupCandidateV1> {
    let spans = dynamic_numbered_group_spans(blocks);
    let answer_index = blocks
        .iter()
        .position(is_dynamic_answer_block)
        .unwrap_or(blocks.len());
    let mut candidates = Vec::new();
    for (position, span) in spans.iter().enumerate() {
        let next_start = spans
            .get(position + 1)
            .map(|next| next.instruction_start)
            .unwrap_or(answer_index);
        let included_end = next_start
            .max(span.last_question_index + 1)
            .min(blocks.len());
        let included = &blocks[span.instruction_start..included_end];
        if included.is_empty() {
            continue;
        }
        let relative_first_question = span
            .first_question_index
            .saturating_sub(span.instruction_start);
        let instruction_text =
            dynamic_instruction_text_from_blocks(included, relative_first_question);
        let combined = included
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");
        let block_ids = included.iter().map(dynamic_block_id).collect::<Vec<_>>();
        let mut classification = classify_dynamic_group(&combined, &block_ids);
        classification.warnings.push(
            "Question range was inferred from consecutive question numbers because no explicit range heading was found."
                .to_string(),
        );
        let layout_hint = dynamic_layout_hint_for_group(&classification.kind, &combined);
        candidates.push(SplitGroupCandidateV1 {
            group_id: format!("group-{}", candidates.len() + 1),
            heading: dynamic_question_heading(span.start, span.end),
            question_range: [span.start, span.end],
            instruction_text: if instruction_text.is_empty() {
                dynamic_question_heading(span.start, span.end)
            } else {
                instruction_text
            },
            block_ids,
            kind_hint: classification.kind.clone(),
            layout_hint: Some(layout_hint.to_string()),
            confidence: classification.confidence.min(0.78),
            classification: Some(classification),
            section_evidence: split_section_evidence_for_blocks(included),
            continuation_edges: split_continuation_edges_for_blocks(included),
            is_umbrella_range: None,
            requires_manual_question_import: None,
        });
    }
    candidates
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

    let explicitly_zero_question_groups = job_explicitly_declares_zero_question_groups(job);
    let numbered_spans = dynamic_numbered_group_spans(&blocks);
    // A passage-only source may legitimately contain numbered prose (for
    // example, a list of two historical stages). Do not let those bare
    // consecutive numbers establish a question area. Explicit `Questions
    // N-M` headings are still discovered below and remain authoritative.
    let inferred_question_area_start = (!explicitly_zero_question_groups)
        .then(|| numbered_spans.first().map(|span| span.instruction_start))
        .flatten();
    let inferred_first_question_index = (!explicitly_zero_question_groups)
        .then(|| numbered_spans.first().map(|span| span.first_question_index))
        .flatten();
    let first_question_index = blocks
        .iter()
        .position(is_dynamic_question_block)
        .or(inferred_first_question_index);
    let first_range_heading_index = blocks.iter().enumerate().find_map(|(index, block)| {
        let text = dynamic_block_text(block);
        if detect_dynamic_question_heading_range(&text).is_some()
            && !is_dynamic_umbrella_question_block(&blocks, index)
        {
            Some(index)
        } else {
            None
        }
    });
    let first_concrete_question_index = first_range_heading_index.or(inferred_question_area_start);
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
                    && !is_dynamic_answer_block(block)
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
                    && !is_dynamic_answer_block(block)
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
    let fallback_question_group_evidence =
        has_dynamic_fallback_question_group_evidence(&question_blocks);

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
        let raw_kind = detect_dynamic_group_kind(
            &raw_included
                .iter()
                .map(dynamic_block_text)
                .collect::<Vec<_>>()
                .join(" "),
        );
        let declared_completion_bank_task =
            dynamic_completion_letter_bank_task_end(raw_kind, raw_included).is_some();
        let interleaved_passage_runs = if declared_completion_bank_task {
            // Summary/list completion is intentionally prose-shaped. Once an
            // exact A-terminal bank closes the task on the source page, the
            // prose between numbered gaps is question stimulus, not passage.
            Vec::new()
        } else if is_dynamic_completion_kind(raw_kind) {
            collect_dynamic_completion_interleaved_passage_runs(raw_included)
        } else {
            collect_dynamic_interleaved_passage_runs(raw_included)
        };
        let mut defer_mask = vec![false; raw_included.len()];
        for (run_start, run_end) in interleaved_passage_runs {
            for raw_index in run_start..run_end.min(raw_included.len()) {
                defer_mask[raw_index] = true;
            }
        }
        if let Some(umbrella_index) =
            raw_included
                .iter()
                .enumerate()
                .skip(1)
                .find_map(|(raw_index, candidate)| {
                    is_known_dynamic_umbrella_block(candidate, &all_umbrella_blocks)
                        .then_some(raw_index)
                })
        {
            // In question-first files the next passage starts after the last
            // concrete question group. Its whole-paper range instruction is a
            // reliable ownership boundary even when the following prose is too
            // short to satisfy the generic three-block passage-run heuristic.
            // Preserve the actual passage content, but keep the umbrella itself
            // solely as range metadata.
            let passage_start = umbrella_index
                .checked_sub(1)
                .filter(|previous| {
                    raw_included.get(*previous).is_some_and(|block| {
                        is_dynamic_reading_passage_heading(&dynamic_block_text(block))
                    })
                })
                .unwrap_or(umbrella_index + 1);
            for raw_index in passage_start..raw_included.len() {
                if raw_index != umbrella_index {
                    defer_mask[raw_index] = true;
                }
            }
        }
        let preliminary_blocks = raw_included
            .iter()
            .enumerate()
            .filter_map(|(raw_index, block)| {
                if defer_mask.get(raw_index).copied().unwrap_or(false) {
                    deferred_passage_blocks.push(block.clone());
                    None
                } else if is_known_dynamic_umbrella_block(block, &all_umbrella_blocks) {
                    // A question-first document can be followed immediately by
                    // the next passage's whole-paper instruction (for example
                    // `You should spend ... Questions 14-26 ...`).  It carries
                    // useful range provenance, but it is not stimulus for the
                    // preceding concrete subgroup.  Keep it in
                    // `umbrellaQuestionRanges` and out of the group's prompt
                    // source, just as we already do when selecting subgroup
                    // headings above.
                    None
                } else if declared_completion_bank_task
                    && is_dynamic_prompt_terminal_heading(&dynamic_block_text(block))
                {
                    // Column-major extraction can place a footer heading
                    // between option-bank columns. It is neither question
                    // content nor a reason to discard the later columns.
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
        let first_numbered_block = included
            .iter()
            .position(|candidate| {
                dynamic_leading_question_number(&dynamic_block_text(candidate)).is_some()
            })
            .unwrap_or(1.min(included.len()));
        let recovered_instruction =
            dynamic_instruction_text_from_blocks(included, first_numbered_block);
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
        ))
        .max(if classification.kind == "heading_matching" {
            infer_dynamic_heading_matching_range_end_from_blocks(included, start, heading_end)
        } else {
            heading_end
        });
        let layout_hint = dynamic_layout_hint_for_group(&classification.kind, &combined);
        let section_evidence = split_section_evidence_for_blocks(included);
        let continuation_edges = split_continuation_edges_for_blocks(included);
        group_candidates.push(SplitGroupCandidateV1 {
            group_id: format!("group-{}", group_candidates.len() + 1),
            heading: dynamic_question_heading(start, end),
            question_range: [start, end],
            instruction_text: if recovered_instruction.is_empty() {
                text
            } else {
                recovered_instruction
            },
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

    if group_candidates.is_empty()
        && !explicitly_zero_question_groups
        && fallback_question_group_evidence
    {
        group_candidates = make_dynamic_numbered_group_candidates(&question_blocks);
    }

    if group_candidates.is_empty() && !umbrella_ranges.is_empty() {
        if !explicitly_zero_question_groups {
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
        }
    } else if group_candidates.is_empty()
        && !question_blocks.is_empty()
        && !explicitly_zero_question_groups
        && fallback_question_group_evidence
    {
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
    if group_candidates.is_empty()
        && !explicitly_zero_question_groups
        && !fallback_question_group_evidence
    {
        // Preserve all source prose when a stale or overly broad parser hint
        // routed an ordinary article block into `question_blocks`. With no
        // independent IELTS evidence there is no valid question boundary.
        passage_blocks = blocks
            .iter()
            .filter(|block| {
                !is_dynamic_non_content_placeholder_block(block)
                    && !is_dynamic_answer_block(block)
                    && dynamic_block_role(block) != "ignore"
            })
            .cloned()
            .collect();
    }
    extend_dynamic_choice_option_blocks(&mut group_candidates, &blocks);
    extend_dynamic_matching_option_blocks(&mut group_candidates, &blocks);
    normalize_dynamic_group_ranges(&mut group_candidates, &blocks);

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
    if group_candidates.is_empty() && explicitly_zero_question_groups {
        issues.push(
            "Source is explicitly marked as passage-only; no question groups were created."
                .to_string(),
        );
    } else if group_candidates.is_empty() {
        if !numbered_spans.is_empty() && !fallback_question_group_evidence {
            issues.push(
                "QUESTION_STRUCTURE_NOT_DETECTED: Consecutive numbered prose was preserved as passage because no explicit question range, IELTS instruction, or credible question-sized sequence was found."
                    .to_string(),
            );
        } else {
            issues.push("No question range heading detected; manual split required.".to_string());
        }
    } else if group_candidates
        .iter()
        .any(|candidate| candidate.requires_manual_question_import == Some(true))
    {
        issues.push("Only umbrella question range detected; concrete question prompts must be imported or entered manually.".to_string());
    }
    if answer_map.is_empty() && !(explicitly_zero_question_groups && group_candidates.is_empty()) {
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
        "heading_matching" => {
            json!({"type": "matching", "options": ["i", "ii", "iii", "iv", "v", "vi", "vii"], "allowOptionReuse": false})
        }
        "matching" | "matching_information" | "classification" => {
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
    if matches!(
        dynamic_previous_word_lower(text, start).as_str(),
        "passage" | "box" | "boxes" | "question" | "questions"
    ) {
        return true;
    }
    let clause = text[..start]
        .rsplit(|ch| matches!(ch, '\n' | '\r' | '.' | '?' | '!'))
        .next()
        .unwrap_or_default();
    let normalized = collapse_whitespace(clause).to_ascii_lowercase();
    if let Some(question_word) = normalized.rfind("questions ") {
        let suffix = normalized[question_word + "questions ".len()..]
            .split_whitespace()
            .collect::<Vec<_>>();
        if !suffix.is_empty()
            && suffix.iter().all(|token| {
                token
                    .trim_matches(|ch: char| {
                        matches!(ch, '-' | '\u{2010}' | '\u{2013}' | '\u{2014}')
                    })
                    .parse::<u32>()
                    .is_ok()
                    || *token == "and"
            })
        {
            return true;
        }
    }
    false
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

fn is_dynamic_prompt_terminal_heading(text: &str) -> bool {
    let normalized = collapse_whitespace(text)
        .trim_matches(|ch: char| matches!(ch, ':' | ';' | '-' | '\u{2013}' | '\u{2014}'))
        .trim()
        .to_ascii_lowercase();
    normalized == "key"
        || normalized == "answer key"
        || normalized.starts_with("answer key:")
        || normalized == "answers"
        || normalized.starts_with("answers:")
        || normalized == "disclaimer"
        || normalized.starts_with("disclaimer:")
}

fn find_dynamic_prompt_line_boundary<F>(text: &str, from: usize, predicate: F) -> Option<usize>
where
    F: Fn(&str) -> bool,
{
    let mut line_start = 0usize;
    for line in text.split_inclusive('\n') {
        let line_end = line_start + line.len();
        if line_end > from && predicate(line.trim()) {
            return Some(line_start.max(from));
        }
        line_start = line_end;
    }
    if line_start < text.len() && line_start >= from && predicate(text[line_start..].trim()) {
        return Some(line_start);
    }
    None
}

fn find_dynamic_ascii_section_marker(text: &str, from: usize, marker: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let mut search = from.min(text.len());
    while let Some(relative) = lower[search..].find(marker) {
        let start = search + relative;
        let end = start + marker.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .map(|ch| ch.is_whitespace() || matches!(ch, ':' | ';' | '.' | ')' | ']'))
            .unwrap_or(true);
        let after_ok = text[end..]
            .chars()
            .next()
            .map(|ch| {
                ch.is_whitespace() || matches!(ch, ':' | ';' | '.' | '-' | '\u{2013}' | '\u{2014}')
            })
            .unwrap_or(true);
        let starts_line = text[..start]
            .rsplit(['\n', '\r'])
            .next()
            .unwrap_or_default()
            .trim()
            .is_empty();
        let followed_by_colon = text[end..]
            .chars()
            .next()
            .map(|ch| ch == ':')
            .unwrap_or(false);
        // `answers` is also a normal English verb. Treat it as a section
        // marker only in heading position (or in the common `Answers:` form)
        // so prompts such as "Which paragraph answers ..." stay intact.
        let heading_like = marker != "answers" || starts_line || followed_by_colon;
        if before_ok && after_ok && heading_like {
            return Some(start);
        }
        search = end;
    }
    None
}

fn find_dynamic_final_prompt_boundary(text: &str, from: usize) -> usize {
    let mut boundary = find_dynamic_prompt_line_boundary(text, from, |line| {
        is_dynamic_prompt_terminal_heading(line)
            || detect_dynamic_question_heading_range(line).is_some()
    })
    .unwrap_or(text.len());
    for marker in ["answer key", "answers", "disclaimer"] {
        if let Some(index) = find_dynamic_ascii_section_marker(text, from, marker) {
            boundary = boundary.min(index);
        }
    }
    boundary
}

fn is_dynamic_prompt_option_bank_heading(text: &str) -> bool {
    let lower = collapse_whitespace(text).to_ascii_lowercase();
    let compact = lower
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    [
        "list of headings",
        "list of people",
        "list of researchers",
        "list of names",
        "list of options",
        "list of universities",
        "list of companies",
        "list of sections",
        "list of words",
        "list of phrases",
        "list of endings",
    ]
    .iter()
    .any(|marker| {
        lower.starts_with(marker)
            || compact.starts_with(
                &marker
                    .chars()
                    .filter(|ch| ch.is_ascii_alphanumeric())
                    .collect::<String>(),
            )
    })
}

fn has_dynamic_prompt_option_bank_context(group_kind: &str, group_text: &str) -> bool {
    if matches!(
        group_kind,
        "heading_matching" | "matching" | "matching_information" | "classification"
    ) {
        return true;
    }
    let normalized = normalized_dynamic_instruction_text(group_text);
    has_dynamic_letter_option_span(&normalized)
        && [
            "list of words",
            "list of options",
            "list of phrases",
            "list of endings",
            "using the list",
            "from the list",
        ]
        .iter()
        .any(|cue| normalized.contains(cue))
}

fn dynamic_prompt_bank_labels(text: &str) -> Vec<char> {
    if dynamic_leading_option_label_and_text(text).and_then(|(label, _)| label.chars().next())
        != Some('A')
    {
        return Vec::new();
    }
    ('A'..=DYNAMIC_MAX_OPTION_LABEL)
        .filter(|label| find_dynamic_option_marker(text, *label, 0).is_some())
        .collect()
}

fn find_dynamic_prompt_option_bank_boundary(text: &str, from: usize) -> Option<usize> {
    let mut lines = Vec::<(usize, &str)>::new();
    let mut line_start = 0usize;
    for line in text.split_inclusive('\n') {
        lines.push((line_start, line.trim()));
        line_start += line.len();
    }
    if line_start < text.len() {
        lines.push((line_start, text[line_start..].trim()));
    }

    for (position, (candidate_start, line)) in lines.iter().enumerate() {
        if *candidate_start < from || dynamic_prompt_bank_labels(line).first() != Some(&'A') {
            continue;
        }
        let mut labels = std::collections::BTreeSet::new();
        for (_, option_line) in lines.iter().skip(position).take(10) {
            let Some((label, _)) = dynamic_leading_option_label_and_text(option_line) else {
                break;
            };
            let Some(label) = label.chars().next() else {
                break;
            };
            if !is_dynamic_letter_option_label(label) {
                break;
            }
            labels.insert(label);
            for inline in dynamic_prompt_bank_labels(option_line) {
                labels.insert(inline);
            }
        }
        if labels.len() >= 3 {
            return Some(*candidate_start);
        }
    }
    None
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
    find_dynamic_prompt_boundary_with_context(text, from, next_number, group_kind, text)
}

fn find_dynamic_completion_display_question_boundary(
    text: &str,
    next_number: u32,
    from: usize,
) -> Option<usize> {
    let mut search = from.min(text.len());
    while let Some((start, content_start)) = find_dynamic_number_marker(text, next_number, search) {
        let prefix = text[..start].trim_end();
        let starts_logical_item = prefix.is_empty()
            || prefix
                .chars()
                .next_back()
                .is_some_and(|ch| matches!(ch, '\n' | '\r' | '.' | '?' | '!' | '\u{2022}'));
        if !starts_logical_item {
            search = content_start.max(start + next_number.to_string().len());
            continue;
        }
        let first_word_char = text[content_start..]
            .chars()
            .skip_while(|ch| ch.is_whitespace() || matches!(ch, '"' | '\'' | '(' | '['))
            .next();
        if !first_word_char.is_some_and(|ch| ch.is_uppercase()) {
            // A range-local number followed by lower-case prose is much more
            // likely to be a quantity ("7 participants") than a displayed
            // IELTS question number.
            search = content_start.max(start + next_number.to_string().len());
            continue;
        }
        let local_end = (content_start + 520).min(text.len());
        let local = &text[content_start..local_end];
        if let Some(blank_start) = find_dynamic_response_blank_start(local, 0) {
            let before_blank = &local[..blank_start];
            if before_blank.chars().any(|ch| ch.is_alphabetic()) {
                return Some(start);
            }
        }
        search = content_start.max(start + next_number.to_string().len());
    }
    None
}

fn find_dynamic_cross_block_completion_number_boundary(
    current: &Value,
    current_text: &str,
    next: &Value,
    next_number: u32,
) -> Option<usize> {
    if !is_dynamic_local_completion_continuation(current, next) {
        return None;
    }
    let next_text = dynamic_block_text(next);
    let first_non_space = next_text
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(index, _)| index)?;
    if find_dynamic_response_blank_start(&next_text, first_non_space) != Some(first_non_space) {
        return None;
    }

    let mut search = 0usize;
    let mut trailing = None;
    while let Some((start, content_start)) =
        find_dynamic_number_marker(current_text, next_number, search)
    {
        if current_text[content_start..].trim().is_empty() {
            trailing = Some(start);
        }
        search = content_start.max(start + next_number.to_string().len());
    }
    trailing
}

fn find_dynamic_prompt_boundary_with_context(
    text: &str,
    from: usize,
    next_number: u32,
    group_kind: &str,
    group_text: &str,
) -> usize {
    let mut boundary = find_dynamic_final_prompt_boundary(text, from);
    // A completion slot is stronger evidence than a generic question number.
    // In notes and timelines a slot is commonly introduced by a bullet/dash
    // (`- 10 ______`), which the generic detector intentionally rejects to
    // avoid treating years and numeric prose as question anchors.  Using the
    // numbered blank here keeps that protection while still closing the
    // current prompt at the next real response slot.
    let next_marker = if is_dynamic_completion_kind(group_kind) {
        find_dynamic_numbered_blank_marker(text, next_number, from).or_else(|| {
            find_dynamic_completion_display_question_boundary(text, next_number, from)
                .map(|start| (start, start))
        })
    } else {
        find_dynamic_number_marker(text, next_number, from)
    };
    if let Some((next_start, _)) = next_marker {
        boundary = boundary.min(next_start);
    }
    if has_dynamic_prompt_option_bank_context(group_kind, group_text) {
        if let Some(index) = find_dynamic_prompt_line_boundary(text, from, |line| {
            is_dynamic_prompt_option_bank_heading(line)
        }) {
            boundary = boundary.min(index);
        }
        if matches!(
            group_kind,
            "heading_matching" | "matching" | "matching_information" | "classification"
        ) {
            let lower = text.to_ascii_lowercase();
            for marker in [
                "list of headings",
                "list of people",
                "list of researchers",
                "list of names",
                "list of options",
                "list of universities",
                "list of companies",
                "list of sections",
            ] {
                if let Some(relative) = lower[from..].find(marker) {
                    boundary = boundary.min(from + relative);
                }
            }
        }
        if !matches!(group_kind, "heading_matching") {
            if let Some(option_boundary) = find_dynamic_matching_option_run_boundary(text, from) {
                boundary = boundary.min(option_boundary);
            }
        }
        if let Some(option_boundary) = find_dynamic_prompt_option_bank_boundary(text, from) {
            boundary = boundary.min(option_boundary);
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
    let searchable = group_text.trim();
    let marker = if is_dynamic_completion_kind(group_kind) {
        find_dynamic_numbered_blank_marker(searchable, number, 0)
            .map(|(start, _)| (start, start + number.to_string().len()))
            .or_else(|| {
                let start =
                    find_dynamic_completion_display_question_boundary(searchable, number, 0)?;
                let (_, content_start) = find_dynamic_number_marker(searchable, number, start)?;
                Some((start, content_start))
            })
    } else {
        find_dynamic_number_marker(searchable, number, 0)
    };
    if let Some((_, content_start)) = marker {
        let mut boundary = if number < range_end {
            find_dynamic_prompt_boundary_with_context(
                searchable,
                content_start,
                number + 1,
                group_kind,
                group_text,
            )
        } else {
            find_dynamic_prompt_boundary_with_context(
                searchable,
                content_start,
                range_end.saturating_add(1),
                group_kind,
                group_text,
            )
        };
        if is_dynamic_completion_kind(group_kind) && number < range_end {
            if let Some((next_start, _)) = find_dynamic_numbered_blank_marker_in_range(
                searchable,
                number + 1,
                range_end,
                content_start,
            ) {
                boundary = boundary.min(next_start);
            }
        }
        let prompt = collapse_whitespace(&searchable[content_start..boundary]);
        let prompt = prompt.trim().trim_end_matches([';', ',']).trim();
        if !prompt.is_empty() {
            return prompt.to_string();
        }
    }
    String::new()
}

fn strip_dynamic_leading_question_marker(text: &str, number: u32) -> String {
    let normalized = collapse_whitespace(text);
    let Some((detected, marker_end)) = dynamic_leading_question_marker(&normalized) else {
        return normalized;
    };
    if detected != number {
        return normalized;
    }
    normalized[marker_end..]
        .trim_start_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, ')' | ']' | '.' | ':' | ';' | ',' | '、')
        })
        .trim()
        .to_string()
}

fn find_dynamic_option_marker(text: &str, label: char, from: usize) -> Option<(usize, usize)> {
    let mut search = from.min(text.len());
    while search < text.len() {
        let relative = text[search..].find(label)?;
        let start = search + relative;
        let before_ok = text[..start]
            .chars()
            .next_back()
            .map(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | ':' | ';'))
            .unwrap_or(true);
        if !before_ok {
            search = start + label.len_utf8();
            continue;
        }
        let mut content_start = start + label.len_utf8();
        if let Some(next) = text[content_start..].chars().next() {
            if !(next.is_whitespace() || matches!(next, '.' | ')' | ']' | ':' | '、')) {
                search = content_start;
                continue;
            }
            if matches!(next, '.' | ')' | ']' | ':' | '、') {
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

fn dynamic_inline_choice_parts_from_label_with_minimum(
    text: &str,
    first_label: char,
    minimum_marker_count: usize,
) -> Option<(String, Vec<(String, String)>)> {
    if !matches!(first_label, 'A'..='M') {
        return None;
    }
    let normalized = collapse_whitespace(
        &text
            .chars()
            .filter(|ch| {
                !matches!(
                    ch,
                    '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
                )
            })
            .collect::<String>(),
    );
    let mut markers = Vec::<(char, usize, usize)>::new();
    let mut from = 0usize;
    for label in first_label..=DYNAMIC_MAX_OPTION_LABEL {
        let Some((start, content_start)) = find_dynamic_option_marker(&normalized, label, from)
        else {
            break;
        };
        markers.push((label, start, content_start));
        from = content_start;
    }
    if markers.len() < minimum_marker_count.max(2)
        || markers.first().map(|item| item.0) != Some(first_label)
    {
        return None;
    }

    // A numbered stem can legitimately begin with the word "A" (for
    // example, "A reference is made ...") immediately before the real A
    // option marker. Since the expected sequence starts at A, the greedy
    // marker scan otherwise consumes the stem as option A and loses the real
    // option. If another A marker occurs before B and the prefix is clearly a
    // substantive stem, move the first marker to that later occurrence.
    if first_label == 'A' && markers.len() >= 2 {
        let first_content = markers[0].2;
        let next_label_start = markers[1].1;
        let mut search = first_content;
        let mut duplicate = None;
        while let Some((start, content_start)) =
            find_dynamic_option_marker(&normalized, 'A', search)
        {
            if start >= next_label_start {
                break;
            }
            duplicate = Some((start, content_start));
            search = content_start;
        }
        if let Some((start, content_start)) = duplicate {
            let prefix_words = normalized[..start].split_whitespace().count();
            if prefix_words >= 3 {
                markers[0] = ('A', start, content_start);
            }
        }
    }

    // If the same expected label appears twice before the following label,
    // the first occurrence may be prose (`Type B`, `vitamin B`, `Section B`)
    // rather than a marker. Refuse the ambiguous split instead of consuming
    // the genuine option boundary that follows it.
    for (index, (label, start, _)) in markers.iter().enumerate().skip(1) {
        let end = markers
            .get(index + 1)
            .map(|item| item.1)
            .unwrap_or(normalized.len());
        if find_dynamic_option_marker(&normalized, *label, start + label.len_utf8())
            .is_some_and(|(duplicate, _)| duplicate < end)
        {
            return None;
        }
    }
    let prompt = normalized[..markers[0].1].trim().to_string();
    let mut options = Vec::new();
    for (index, (label, _, content_start)) in markers.iter().enumerate() {
        let end = markers
            .get(index + 1)
            .map(|item| item.1)
            .unwrap_or(normalized.len());
        let option_text = normalized[*content_start..end]
            .trim()
            .trim_end_matches([';', ','])
            .trim()
            .to_string();
        options.push((label.to_string(), option_text));
    }
    Some((prompt, options))
}

fn dynamic_inline_choice_parts_from_label(
    text: &str,
    first_label: char,
) -> Option<(String, Vec<(String, String)>)> {
    // Bare inline capitals are weak evidence. IELTS choice runs normally have
    // at least A/B/C; two letters alone are common in ordinary English prose.
    dynamic_inline_choice_parts_from_label_with_minimum(text, first_label, 3)
}

/// Parse a physical option row whose columns are emitted in visual order
/// rather than label order (for example `A ... D ... G ... J ...`). The
/// contiguous choice parser intentionally rejects this shape because it is
/// unsafe for ordinary prose; a declared completion bank supplies the extra
/// semantic constraint needed here. Callers still require the complete
/// declared A-terminal set before exposing these as runtime options.
fn dynamic_declared_bank_parts(
    text: &str,
    declared_labels: &[String],
) -> Option<Vec<(String, String)>> {
    if declared_labels.len() < 2 {
        return None;
    }
    let normalized = collapse_whitespace(
        &text
            .chars()
            .filter(|ch| {
                !matches!(
                    ch,
                    '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
                )
            })
            .collect::<String>(),
    );
    let mut markers = declared_labels
        .iter()
        .filter_map(|label| {
            let letter = label.chars().next()?;
            let (start, content_start) = find_dynamic_option_marker(&normalized, letter, 0)?;
            Some((letter, start, content_start))
        })
        .collect::<Vec<_>>();
    markers.sort_by_key(|(_, start, _)| *start);
    if markers.len() < 2 {
        return None;
    }
    // A physical bank row starts at its first label. The only safe prefix is a
    // short lower-case word fragment emitted before the next column's label
    // (`ctions D assumption`). Without this boundary, ordinary stimulus prose
    // such as `A study compared B cells ...` can be promoted as option rows.
    let prefix = normalized[..markers[0].1].trim();
    let prefix_is_cross_column_fragment = !prefix.is_empty()
        && markers[0].0 > 'A'
        && prefix.split_whitespace().count() <= 3
        && prefix
            .split_whitespace()
            .all(|word| word.chars().all(|ch| ch.is_ascii_lowercase()));
    if !prefix.is_empty() && !prefix_is_cross_column_fragment {
        return None;
    }
    // Column-major rows may skip labels (`A D G I`), but their source order is
    // still monotonic. A backward label is much stronger evidence for prose
    // capitals than for an IELTS bank, so fail closed.
    if markers.windows(2).any(|pair| pair[1].0 <= pair[0].0) {
        return None;
    }
    let mut options = Vec::with_capacity(markers.len());
    for (index, (label, _, content_start)) in markers.iter().enumerate() {
        let end = markers
            .get(index + 1)
            .map(|(_, start, _)| *start)
            .unwrap_or(normalized.len());
        let option_text = normalized[*content_start..end]
            .trim()
            .trim_end_matches([';', ','])
            .trim()
            .to_string();
        // In a column-major PDF row the last label can be emitted at the end
        // of the left column while its text starts in a later right-column
        // block (`D ... E` followed by `68 seconds F ...`). Preserve that
        // pending label so the geometry-aware continuation pass can fill it.
        // Empty interior labels remain invalid because their ownership is
        // ambiguous even under an explicit A-terminal declaration.
        if option_text.is_empty() && index + 1 != markers.len() {
            return None;
        }
        options.push((label.to_string(), option_text));
    }
    Some(options)
}

fn dynamic_inline_choice_parts(text: &str) -> Option<(String, Vec<(String, String)>)> {
    dynamic_inline_choice_parts_from_label(text, 'A')
}

fn is_dynamic_prompt_option_bank_start_block(blocks: &[Value], index: usize) -> bool {
    let Some(first) = blocks.get(index) else {
        return false;
    };
    if dynamic_leading_option_label_and_text(&dynamic_block_text(first))
        .and_then(|(label, _)| label.chars().next())
        != Some('A')
    {
        return false;
    }

    let mut labels = std::collections::BTreeSet::new();
    for block in blocks.iter().skip(index).take(10) {
        let text = dynamic_block_text(block);
        let Some((label, _)) = dynamic_leading_option_label_and_text(&text) else {
            break;
        };
        let Some(label) = label.chars().next() else {
            break;
        };
        if !is_dynamic_letter_option_label(label) {
            break;
        }
        labels.insert(label);
        for inline in ('A'..=DYNAMIC_MAX_OPTION_LABEL)
            .filter(|candidate| find_dynamic_option_marker(&text, *candidate, 0).is_some())
        {
            labels.insert(inline);
        }
    }
    labels.len() >= 3
}

/// Recover a shared multiple-choice stem and its A-J option run when the
/// source declares a range such as `Questions 14 and 15` instead of prefixing
/// the common stem with either individual number. This is source-backed: a
/// concrete stem after the instruction and a contiguous option run are both
/// required.
fn dynamic_shared_choice_prompt_and_options(
    blocks: &[Value],
) -> Option<(String, Vec<(String, String)>)> {
    let inline_shared = (0..blocks.len()).rev().find_map(|option_start| {
        let first = dynamic_block_text(&blocks[option_start]);
        let Some((label, _)) = dynamic_leading_option_label_and_text(&first) else {
            return None;
        };
        if label != "A" {
            return None;
        }
        // A/B/C… options are often flattened into one physical block, or
        // split across two blocks when the final labels wrap. Join only the
        // short run immediately following A; question/instruction markers
        // remain hard boundaries so passage prose is never consumed.
        let mut option_text = first;
        for continuation in blocks.iter().skip(option_start + 1).take(9) {
            let text = dynamic_block_text(continuation);
            if text.trim().is_empty()
                || dynamic_leading_question_number(&text).is_some()
                || is_dynamic_instruction_signal(&text)
                || is_dynamic_prompt_terminal_heading(&text)
            {
                break;
            }
            option_text.push(' ');
            option_text.push_str(&text);
        }
        let (_, options) =
            dynamic_inline_choice_parts_from_label_with_minimum(&option_text, 'A', 3)?;
        let control_start = (0..option_start)
            .rev()
            .find(|index| is_dynamic_choice_control_signal(&dynamic_block_text(&blocks[*index])))
            .map(|index| index + 1)
            .unwrap_or(0);
        let prompt = collapse_whitespace(
            &blocks[control_start..option_start]
                .iter()
                .map(dynamic_block_text)
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        );
        (!prompt.is_empty()).then_some((prompt, options))
    });
    if let Some(shared) = inline_shared {
        return Some(shared);
    }

    let (option_start, option_end) = dynamic_last_choice_option_run_bounds(blocks)?;
    let stem_start = (0..option_start)
        .rev()
        .find(|index| is_dynamic_instruction_signal(&dynamic_block_text(&blocks[*index])))
        .map(|index| index + 1)
        .unwrap_or(0);
    if stem_start >= option_start {
        return None;
    }
    let prompt = collapse_whitespace(
        &blocks[stem_start..option_start]
            .iter()
            .map(dynamic_block_text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    );
    if prompt.is_empty() {
        return None;
    }

    let mut options = Vec::<(String, String)>::new();
    for block in &blocks[option_start..option_end] {
        let text = dynamic_block_text(block);
        if let Some((label, option_text)) =
            dynamic_leading_option_label_and_text(&text).filter(|(label, option_text)| {
                label.len() == 1
                    && label.chars().all(is_dynamic_letter_option_label)
                    && !option_text.is_empty()
            })
        {
            options.push((label, option_text));
        } else if let Some((_, option_text)) = options.last_mut() {
            let continuation = collapse_whitespace(&text);
            if !continuation.is_empty() {
                *option_text = collapse_whitespace(&format!("{} {}", option_text, continuation));
            }
        }
    }
    (options.len() >= 3).then_some((prompt, options))
}

fn is_dynamic_choice_control_signal(text: &str) -> bool {
    let lower = normalized_dynamic_instruction_text(text);
    lower.contains("choose ")
        || lower.starts_with("choose")
        || lower.contains("write the correct")
        || lower.contains("answer sheet")
        || lower.contains("in boxes")
}

fn dynamic_question_prompt_and_options(
    group_blocks: &[Value],
    group_text: &str,
    number: u32,
    heading: &str,
    range_end: u32,
    kind: &str,
) -> (String, Vec<(String, String)>) {
    // Shared matching/classification banks belong to the response model, not
    // to the final question's prompt.  A range's last numbered stem has no
    // following question number to stop it, so use the source-backed bank
    // closure as an additional physical boundary.  This is deliberately not
    // inferred from a bare leading `A`: the helper requires the complete
    // declared A-terminal bank and selects its right-most valid A source.
    let group_option_bank_start = dynamic_group_option_bank_start_index(group_blocks, kind);
    if matches!(kind, "single_choice" | "multi_choice")
        && !group_blocks.iter().any(|block| {
            dynamic_leading_question_number(&dynamic_block_text(block)) == Some(number)
        })
    {
        if let Some(shared) = dynamic_shared_choice_prompt_and_options(group_blocks) {
            return shared;
        }
    }
    let Some((start_index, marker_start)) = group_blocks
        .iter()
        .enumerate()
        .find_map(|(index, block)| {
            let text = dynamic_block_text(block);
            (dynamic_leading_question_number(&text) == Some(number)).then_some((index, 0usize))
        })
        .or_else(|| {
            group_blocks.iter().enumerate().find_map(|(index, block)| {
                let text = dynamic_block_text(block);
                if is_dynamic_completion_kind(kind) {
                    find_dynamic_numbered_blank_marker(&text, number, 0)
                        .map(|(marker_start, _)| (marker_start, marker_start))
                        .or_else(|| {
                            find_dynamic_completion_display_question_boundary(&text, number, 0)
                                .map(|marker_start| (marker_start, marker_start))
                        })
                } else {
                    find_dynamic_number_marker(&text, number, 0)
                }
                .map(|(marker_start, _)| (index, marker_start))
            })
        })
    else {
        if matches!(kind, "single_choice" | "multi_choice") {
            if let Some(shared) = dynamic_shared_choice_prompt_and_options(group_blocks) {
                return shared;
            }
        }
        let fallback = dynamic_prompt_for_question(group_text, number, heading, range_end, kind);
        // Inline completion groups often defer the fill-in text into the
        // passage; the numbered blank lives there, not in the instruction
        // blocks. When the prompt derives from the instruction and is empty,
        // let the caller run the gap recovery pass over the following blocks.
        if fallback.trim().is_empty()
            && is_dynamic_completion_kind(kind)
            && dynamic_layout_hint_for_group(kind, group_text) == "inline_completion"
        {
            return (fallback, Vec::new());
        }
        if matches!(kind, "single_choice" | "multi_choice") {
            if let Some((prompt, options)) = dynamic_inline_choice_parts(&fallback) {
                return (prompt, options);
            }
        }
        return (fallback, Vec::new());
    };

    let end_index = group_blocks
        .iter()
        .enumerate()
        .skip(start_index + 1)
        .find_map(|(index, block)| {
            let text = dynamic_block_text(block);
            if is_dynamic_completion_kind(kind) {
                let candidate = number + 1;
                if candidate > range_end
                    || dynamic_leading_question_number(&text) != Some(candidate)
                {
                    return None;
                }
                let mut local_parts = Vec::new();
                for local_block in group_blocks.iter().skip(index) {
                    let local_text = dynamic_block_text(local_block);
                    if !local_parts.is_empty()
                        && dynamic_leading_question_number(&local_text)
                            .is_some_and(|next| next > candidate && next <= range_end)
                    {
                        break;
                    }
                    local_parts.push(local_text);
                    if local_parts.len() >= 3 {
                        break;
                    }
                }
                let local = local_parts.join(" ");
                (find_dynamic_completion_display_question_boundary(&local, candidate, 0) == Some(0))
                    .then_some(index)
            } else {
                dynamic_leading_question_number(&text)
                    .filter(|candidate| *candidate > number && *candidate <= range_end)
                    .map(|_| index)
            }
        })
        .unwrap_or(group_blocks.len());
    let mut prompt_parts = Vec::new();
    let mut options = Vec::new();
    // Keep the physical source block that most recently contributed an
    // option.  PDF text extraction commonly emits a long option as
    // `A first line` followed by one or more indented, unlabelled blocks.
    // Without preserving this source relationship those continuation lines
    // fall through into the question prompt.
    let mut last_option_block_index: Option<usize> = None;
    let mut absolute_index = start_index;
    while absolute_index < end_index {
        if group_option_bank_start.is_some_and(|bank_start| absolute_index >= bank_start) {
            break;
        }
        let relative = absolute_index - start_index;
        let block = &group_blocks[absolute_index];
        let raw_text = dynamic_block_text(block);
        let option_bank_context = has_dynamic_prompt_option_bank_context(kind, group_text);
        if relative > 0
            && (is_dynamic_prompt_terminal_heading(&raw_text)
                || (option_bank_context
                    && (is_dynamic_prompt_option_bank_heading(&raw_text)
                        || is_dynamic_prompt_option_bank_start_block(
                            group_blocks,
                            absolute_index,
                        ))))
        {
            break;
        }
        let text = if relative == 0 {
            if dynamic_leading_question_number(&raw_text) == Some(number) {
                strip_dynamic_leading_question_marker(&raw_text, number)
            } else {
                let after_number = marker_start.saturating_add(number.to_string().len());
                collapse_whitespace(
                    raw_text[after_number.min(raw_text.len())..].trim_start_matches(|ch: char| {
                        ch.is_whitespace() || matches!(ch, ')' | ']' | '.' | ':' | ';' | ',' | '、')
                    }),
                )
            }
        } else {
            raw_text
        };
        let mut boundary = if number < range_end {
            find_dynamic_prompt_boundary_with_context(&text, 0, number + 1, kind, group_text)
        } else {
            find_dynamic_prompt_boundary_with_context(
                &text,
                0,
                range_end.saturating_add(1),
                kind,
                group_text,
            )
        };
        if is_dynamic_completion_kind(kind) && number < range_end {
            if let Some(next_block) = group_blocks.get(absolute_index + 1) {
                if let Some(next_start) = find_dynamic_cross_block_completion_number_boundary(
                    block,
                    &text,
                    next_block,
                    number + 1,
                ) {
                    boundary = boundary.min(next_start);
                }
            }
        }
        if is_dynamic_completion_kind(kind) && number < range_end {
            if let Some((next_start, _)) =
                find_dynamic_numbered_blank_marker_in_range(&text, number + 1, range_end, 0)
            {
                boundary = boundary.min(next_start);
            }
        }
        let stop_after_block = boundary < text.len();
        let text = collapse_whitespace(&text[..boundary]);
        if stop_after_block && !matches!(kind, "single_choice" | "multi_choice") {
            if !text.trim().is_empty() {
                prompt_parts.push(text);
            }
            break;
        }
        if matches!(
            kind,
            "heading_matching" | "matching" | "matching_information" | "classification"
        ) && dynamic_inline_choice_parts(&text)
            .map(|(prompt, options)| prompt.is_empty() && !options.is_empty())
            .unwrap_or(false)
        {
            absolute_index += 1;
            continue;
        }
        if matches!(kind, "single_choice" | "multi_choice") {
            let table_options = dynamic_table_option_rows(block)
                .into_iter()
                .filter(|(label, option_text)| {
                    label.len() == 1
                        && label.chars().all(is_dynamic_letter_option_label)
                        && !option_text.is_empty()
                })
                .collect::<Vec<_>>();
            if !table_options.is_empty() {
                for option in table_options {
                    if !options
                        .iter()
                        .any(|(existing_label, _)| existing_label == &option.0)
                    {
                        options.push(option);
                    }
                }
                last_option_block_index = Some(absolute_index);
                if stop_after_block {
                    break;
                }
                absolute_index += 1;
                continue;
            }
            let declared_choice_terminal = dynamic_letter_options_for_text(group_text)
                .last()
                .and_then(|label| label.chars().next());
            let inline_choice = dynamic_inline_choice_parts(&text).or_else(|| {
                let expected_label = options
                    .last()
                    .and_then(|(label, _)| label.chars().next())
                    .map(|label| ((label as u8).saturating_add(1)) as char)
                    .unwrap_or('A');
                let (label, _) = dynamic_leading_option_label_and_text(&text)?;
                let first_label = label.chars().next()?;
                if label.len() != 1 || first_label != expected_label {
                    return None;
                }
                dynamic_inline_choice_parts_from_label(&text, first_label).or_else(|| {
                    // A flattened partial tail can be followed by the final
                    // label in its own block (`B ... C ...` then `D ...`).
                    // Accept the partial sequence only when the next physical
                    // block resumes the exact declared order, or when this
                    // block itself reaches the declared terminal.
                    dynamic_inline_choice_parts_from_label_with_minimum(&text, first_label, 2)
                        .filter(|(_, recovered)| {
                            let Some(last_label) =
                                recovered.last().and_then(|(label, _)| label.chars().next())
                            else {
                                return false;
                            };
                            let Some(terminal) = declared_choice_terminal else {
                                return false;
                            };
                            if last_label == terminal {
                                return true;
                            }
                            if last_label >= terminal {
                                return false;
                            }
                            let expected_next = ((last_label as u8).saturating_add(1)) as char;
                            group_blocks
                                .get(absolute_index + 1)
                                .and_then(|block| {
                                    dynamic_leading_option_label_and_text(&dynamic_block_text(
                                        block,
                                    ))
                                })
                                .and_then(|(label, _)| label.chars().next())
                                == Some(expected_next)
                        })
                })
            });
            if let Some((inline_prompt, inline_options)) = inline_choice {
                if !inline_prompt.is_empty() {
                    prompt_parts.push(inline_prompt);
                }
                for option in inline_options {
                    if !options
                        .iter()
                        .any(|(existing_label, _)| existing_label == &option.0)
                    {
                        options.push(option);
                    }
                }
                last_option_block_index = Some(absolute_index);
                if stop_after_block {
                    break;
                }
                absolute_index += 1;
                continue;
            }
            if let Some((label, mut option_text)) = (relative > 0)
                .then(|| dynamic_leading_option_label_and_text(&text))
                .flatten()
            {
                if label.len() == 1 && label.chars().all(is_dynamic_letter_option_label) {
                    // PDF layouts frequently emit the choice letter alone, then
                    // the option prose on the next block:
                    //   "A"
                    //   "changing the bed linen"
                    // Absorb one following non-label prose block so the option
                    // set closes instead of discarding the bare letter.
                    let mut consumed_continuation = false;
                    if option_text.is_empty() {
                        if let Some(next_block) = group_blocks.get(absolute_index + 1) {
                            let next_text = collapse_whitespace(&dynamic_block_text(next_block));
                            let next_is_question =
                                dynamic_leading_question_number(&next_text).is_some();
                            let next_is_option =
                                dynamic_leading_option_label_and_text(&next_text).is_some();
                            if !next_text.is_empty()
                                && !next_is_question
                                && !next_is_option
                                && !is_dynamic_instruction_signal(&next_text)
                                && !is_dynamic_prompt_terminal_heading(&next_text)
                            {
                                option_text = next_text;
                                consumed_continuation = true;
                            }
                        }
                    }
                    if !option_text.is_empty()
                        && !options
                            .iter()
                            .any(|(existing_label, _)| existing_label == &label)
                    {
                        options.push((label, option_text));
                        last_option_block_index = Some(if consumed_continuation {
                            absolute_index + 1
                        } else {
                            absolute_index
                        });
                        if stop_after_block {
                            break;
                        }
                        absolute_index += if consumed_continuation { 2 } else { 1 };
                        continue;
                    }
                }
            }
        }
        if matches!(kind, "single_choice" | "multi_choice")
            && dynamic_leading_option_label_and_text(&text).is_none()
            && !is_dynamic_instruction_signal(&text)
            && !is_dynamic_prompt_terminal_heading(&text)
            && dynamic_leading_question_number(&text).is_none()
        {
            if let (Some(previous_index), Some((_, option_text))) =
                (last_option_block_index, options.last_mut())
            {
                let previous = &group_blocks[previous_index];
                if is_dynamic_same_row_option_continuation(previous, block)
                    || is_dynamic_wrapped_option_continuation(previous, block)
                {
                    append_dynamic_option_continuation(option_text, &text);
                    last_option_block_index = Some(absolute_index);
                    if stop_after_block {
                        break;
                    }
                    absolute_index += 1;
                    continue;
                }
            }
        }
        if !text.trim().is_empty() && (relative == 0 || !is_dynamic_instruction_signal(&text)) {
            prompt_parts.push(text);
        }
        if stop_after_block {
            break;
        }
        absolute_index += 1;
    }
    let mut prompt = collapse_whitespace(&prompt_parts.join(" "));
    if options.is_empty() && matches!(kind, "single_choice" | "multi_choice") {
        if let Some((plain_prompt, inline_options)) = dynamic_inline_choice_parts(&prompt) {
            prompt = plain_prompt;
            options = inline_options;
        }
    }
    if prompt.is_empty() {
        prompt = dynamic_prompt_for_question(group_text, number, heading, range_end, kind);
    }
    if prompt.trim().is_empty()
        && is_dynamic_completion_kind(kind)
        && dynamic_layout_hint_for_group(kind, group_text) == "inline_completion"
    {
        // The numbered blank was deferred into the passage; leave the prompt
        // empty so the caller's gap recovery pass fills it from the blocks
        // that follow the group.
        return (String::new(), options);
    }
    (prompt, options)
}

fn is_dynamic_completion_kind(kind: &str) -> bool {
    matches!(
        kind,
        "summary_completion" | "sentence_completion" | "table_completion" | "diagram_completion"
    )
}

/// Recover the sentence containing the numbered blank (e.g. "24…………") from a
/// run of fill-in text. Sentence boundaries keep the prompt self-contained
/// instead of swallowing the whole trailing text or neighbouring sections.
fn dynamic_gap_sentence_prompt(text: &str, number: u32) -> String {
    let Some((marker_start, marker_end)) = find_dynamic_numbered_blank_marker(text, number, 0)
    else {
        return String::new();
    };
    let sentence_start = text[..marker_start]
        .rfind(". ")
        .map(|index| index + 2)
        .unwrap_or(0);
    let sentence_end = text[marker_end..]
        .find(". ")
        .map(|index| marker_end + index + 2)
        .unwrap_or(text.len());
    let number_end = marker_start + number.to_string().len();
    let prompt = format!(
        "{}{}",
        &text[sentence_start..marker_start],
        &text[number_end..sentence_end]
    );
    collapse_whitespace(prompt.trim())
}

pub(crate) fn dynamic_completion_foreign_slots(
    prompt: &str,
    number: u32,
    range_start: u32,
    range_end: u32,
) -> Vec<u32> {
    (range_start..=range_end)
        .filter(|candidate| *candidate != number)
        .filter(|candidate| {
            find_dynamic_numbered_blank_marker(prompt, *candidate, 0).is_some()
                || find_dynamic_completion_display_question_boundary(prompt, *candidate, 0)
                    .is_some()
        })
        .collect()
}

/// Reduce a non-linear table/diagram block to the local source segment owned
/// by one numbered response slot. This is intentionally source-backed: the
/// current slot must be physically present, and neighbouring numbered blanks
/// are hard segment boundaries. It does not guess row order or synthesize
/// missing labels.
fn dynamic_local_completion_prompt_for_number(
    blocks: &[Value],
    number: u32,
    range_start: u32,
    range_end: u32,
) -> Option<(String, String)> {
    for block in blocks {
        let text = dynamic_block_text(block);
        let Some((marker_start, _)) = find_dynamic_numbered_blank_marker(&text, number, 0) else {
            continue;
        };
        let mut markers = (range_start..=range_end)
            .filter_map(|candidate| {
                find_dynamic_numbered_blank_marker(&text, candidate, 0)
                    .map(|(start, end)| (candidate, start, end))
            })
            .collect::<Vec<_>>();
        markers.sort_by_key(|(_, start, _)| *start);
        let Some(current_index) = markers
            .iter()
            .position(|(candidate, start, _)| *candidate == number && *start == marker_start)
        else {
            continue;
        };
        let segment_start = current_index
            .checked_sub(1)
            .and_then(|index| markers.get(index))
            .map(|(_, _, end)| *end)
            .unwrap_or(0);
        let segment_end = markers
            .get(current_index + 1)
            .map(|(_, start, _)| *start)
            .unwrap_or(text.len());
        let local = dynamic_gap_sentence_prompt(&text[segment_start..segment_end], number);
        if local.trim().is_empty()
            || !dynamic_completion_foreign_slots(&local, number, range_start, range_end).is_empty()
        {
            continue;
        }
        return Some((local, dynamic_block_id(block)));
    }
    None
}

fn is_dynamic_note_bullet(text: &str) -> bool {
    text.trim_start()
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '\u{2022}' | '\u{25cf}' | '-' | '\u{2013}' | '\u{2014}'))
}

fn dynamic_text_has_blank_run(text: &str) -> bool {
    let mut width = 0usize;
    for ch in text.chars() {
        if is_dynamic_blank_marker_char(ch) {
            width += dynamic_blank_marker_width(ch);
            if width >= 3 {
                return true;
            }
        } else if !ch.is_whitespace() {
            width = 0;
        }
    }
    false
}

fn dynamic_note_leading_number_span(text: &str, number: u32) -> Option<(usize, usize)> {
    let mut start = 0usize;
    for (index, ch) in text.char_indices() {
        if ch.is_whitespace()
            || matches!(ch, '\u{2022}' | '\u{25cf}' | '-' | '\u{2013}' | '\u{2014}')
        {
            start = index + ch.len_utf8();
            continue;
        }
        break;
    }
    let digits = number.to_string();
    if !text[start..].starts_with(&digits) {
        return None;
    }
    let end = start + digits.len();
    let after_ok = text[end..]
        .chars()
        .next()
        .map(|ch| !ch.is_ascii_digit())
        .unwrap_or(true);
    after_ok.then_some((start, end))
}

fn dynamic_note_response_marker(text: &str, number: u32) -> bool {
    find_dynamic_numbered_blank_marker(text, number, 0).is_some()
        || dynamic_note_leading_number_span(text, number)
            .is_some_and(|(_, end)| dynamic_text_has_blank_run(&text[end..]))
        || find_dynamic_completion_display_question_boundary(text, number, 0).is_some_and(
            |marker_start| {
                find_dynamic_number_marker(text, number, marker_start).is_some_and(
                    |(_, content_start)| dynamic_text_has_blank_run(&text[content_start..]),
                )
            },
        )
}

fn dynamic_note_row_text_prompt(text: &str, number: u32) -> String {
    let direct = dynamic_gap_sentence_prompt(text, number);
    if !direct.is_empty() {
        return direct;
    }
    if let Some((marker_start, marker_end)) = dynamic_note_leading_number_span(text, number) {
        if dynamic_text_has_blank_run(&text[marker_end..]) {
            return collapse_whitespace(&format!(
                "{}{}",
                &text[..marker_start],
                &text[marker_end..]
            ));
        }
    }
    let Some(marker_start) = find_dynamic_completion_display_question_boundary(text, number, 0)
    else {
        return String::new();
    };
    let Some((_, content_start)) = find_dynamic_number_marker(text, number, marker_start) else {
        return String::new();
    };
    if !dynamic_text_has_blank_run(&text[content_start..]) {
        return String::new();
    }
    collapse_whitespace(&format!(
        "{}{}",
        &text[..marker_start],
        &text[content_start..]
    ))
}

fn is_dynamic_note_section_heading(block: &Value, response_left: Option<f64>) -> bool {
    let text = collapse_whitespace(&dynamic_block_text(block));
    if text.is_empty()
        || text.split_whitespace().count() > 6
        || text.chars().count() > 64
        || text.ends_with(['.', '?', '!', ';'])
        || is_dynamic_note_bullet(&text)
        || is_dynamic_instruction_signal(&text)
        || is_dynamic_question_heading_text(&text)
        || (1..=40).any(|candidate| dynamic_note_response_marker(&text, candidate))
    {
        return false;
    }

    match (dynamic_block_normalized_bbox(block), response_left) {
        (Some(bbox), Some(response_left)) => bbox[0] + 8.0 <= response_left,
        // Synthetic/unit inputs and some non-PDF sources have no geometry.
        // Keep the textual fallback deliberately narrow; the caller still
        // requires an explicit bullet-owned numbered blank.
        _ => text.split_whitespace().count() <= 4,
    }
}

/// Recover one row from a note-like completion tree.  PDF extraction commonly
/// flattens the visual hierarchy into independent blocks:
///
/// ```text
/// Construction
/// • descriptive row without a response
/// • Over a million tiles from 9 ______
/// Use
/// • 11 ______ companies ...
/// ```
///
/// A question owns its numbered bullet plus wrapped continuations, while the
/// most recent less-indented short heading supplies the row's semantic context.
/// A new bullet, response slot, or section heading is a hard boundary.  This is
/// source-backed and intentionally does not manufacture a heading or slot.
fn dynamic_note_row_prompt_for_number(
    blocks: &[Value],
    number: u32,
    range_start: u32,
    range_end: u32,
) -> Option<(String, Vec<String>)> {
    let marker_index = blocks.iter().position(|block| {
        dynamic_note_response_marker(&dynamic_block_text(block), number)
            && is_dynamic_note_bullet(&dynamic_block_text(block))
    })?;
    let marker_block = &blocks[marker_index];
    let response_left = dynamic_block_normalized_bbox(marker_block).map(|bbox| bbox[0]);

    let heading_index = (0..marker_index)
        .rev()
        .find(|index| is_dynamic_note_section_heading(&blocks[*index], response_left))?;

    // A note tree is stronger than an isolated bullet.  Require another bullet
    // in the same source run so ordinary bulleted prose is never promoted to a
    // completion layout solely because it happens to contain a number.
    if !blocks.iter().enumerate().any(|(index, block)| {
        index != marker_index && is_dynamic_note_bullet(&dynamic_block_text(block))
    }) {
        return None;
    }

    let mut row_end = marker_index + 1;
    while row_end < blocks.len() {
        let block = &blocks[row_end];
        let text = dynamic_block_text(block);
        let has_response_slot = (range_start..=range_end)
            .any(|candidate| dynamic_note_response_marker(&text, candidate));
        if has_response_slot
            || is_dynamic_note_bullet(&text)
            || is_dynamic_note_section_heading(block, response_left)
            || is_dynamic_instruction_signal(&text)
        {
            break;
        }
        if !is_dynamic_local_completion_continuation(&blocks[row_end - 1], block) {
            break;
        }
        row_end += 1;
    }

    let row_text = blocks[marker_index..row_end]
        .iter()
        .map(dynamic_block_text)
        .collect::<Vec<_>>()
        .join(" ");
    let row_prompt = dynamic_note_row_text_prompt(&row_text, number);
    if row_prompt.trim().is_empty()
        || !dynamic_completion_foreign_slots(&row_prompt, number, range_start, range_end).is_empty()
    {
        return None;
    }

    let heading = collapse_whitespace(&dynamic_block_text(&blocks[heading_index]));
    let prompt = collapse_whitespace(&format!("{heading}: {row_prompt}"));
    let source_ids = std::iter::once(&blocks[heading_index])
        .chain(blocks[marker_index..row_end].iter())
        .map(dynamic_block_id)
        .filter(|block_id| !block_id.is_empty())
        .collect::<Vec<_>>();
    Some((prompt, source_ids))
}

fn dynamic_gap_sentence_prompt_from_blocks(
    blocks: &[Value],
    number: u32,
) -> Option<(String, Vec<String>)> {
    for marker_index in 0..blocks.len() {
        let marker_text = dynamic_block_text(&blocks[marker_index]);
        if find_dynamic_numbered_blank_marker(&marker_text, number, 0).is_none() {
            continue;
        }
        let start = marker_index
            .checked_sub(1)
            .filter(|previous_index| {
                let previous = &blocks[*previous_index];
                let previous_has_slot = (1..=40).any(|candidate| {
                    find_dynamic_numbered_blank_marker(&dynamic_block_text(previous), candidate, 0)
                        .is_some()
                });
                !previous_has_slot
                    && is_dynamic_local_completion_continuation(previous, &blocks[marker_index])
            })
            .unwrap_or(marker_index);
        let end = (marker_index + 2).min(blocks.len());
        let prompt = dynamic_gap_sentence_prompt(
            &blocks[start..end]
                .iter()
                .map(dynamic_block_text)
                .collect::<Vec<_>>()
                .join(" "),
            number,
        );
        if !prompt.is_empty() {
            let source_ids = blocks[start..end]
                .iter()
                .map(dynamic_block_id)
                .filter(|block_id| !block_id.is_empty())
                .collect::<Vec<_>>();
            return Some((prompt, source_ids));
        }
    }

    // A parser may split the number and its underline into adjacent blocks.
    // Use a small local window, never the entire document tail.
    for start in 0..blocks.len() {
        let end = (start + 3).min(blocks.len());
        let prompt = dynamic_gap_sentence_prompt(
            &blocks[start..end]
                .iter()
                .map(dynamic_block_text)
                .collect::<Vec<_>>()
                .join(" "),
            number,
        );
        if !prompt.is_empty() {
            let source_ids = blocks[start..end]
                .iter()
                .map(dynamic_block_id)
                .filter(|block_id| !block_id.is_empty())
                .collect::<Vec<_>>();
            return Some((prompt, source_ids));
        }
    }
    None
}

fn dynamic_completion_rows(blocks: &[Value]) -> Vec<(String, Vec<String>)> {
    let mut recovered = Vec::new();
    for block in blocks {
        let block_id = dynamic_block_id(block);
        if let Some(cells) = block.pointer("/table/cells").and_then(Value::as_array) {
            let mut rows = std::collections::BTreeMap::<u64, Vec<(u64, String)>>::new();
            for cell in cells {
                let Some(row) = cell.get("row").and_then(Value::as_u64) else {
                    continue;
                };
                let col = cell.get("col").and_then(Value::as_u64).unwrap_or(0);
                let text = cell.get("text").and_then(Value::as_str).unwrap_or_default();
                if !text.trim().is_empty() {
                    rows.entry(row)
                        .or_default()
                        .push((col, collapse_whitespace(text)));
                }
            }
            for mut row in rows.into_values() {
                row.sort_by_key(|(col, _)| *col);
                let values = row.into_iter().map(|(_, text)| text).collect::<Vec<_>>();
                if values.len() >= 2 {
                    recovered.push((block_id.clone(), values));
                }
            }
            continue;
        }

        let text = dynamic_block_text(block);
        if text.contains('|') {
            let cells = text
                .split('|')
                .map(collapse_whitespace)
                .filter(|cell| !cell.is_empty())
                .collect::<Vec<_>>();
            if cells.len() >= 2 {
                recovered.push((block_id, cells));
            }
        }
    }
    recovered
}

fn is_dynamic_completion_table_header(cells: &[String]) -> bool {
    const HEADER_WORDS: &[&str] = &[
        "answer",
        "category",
        "description",
        "feature",
        "function",
        "information",
        "item",
        "location",
        "name",
        "notes",
        "prompt",
        "question",
        "type",
    ];
    cells.iter().all(|cell| {
        let normalized = cell
            .trim_matches(|ch: char| !ch.is_ascii_alphabetic())
            .to_ascii_lowercase();
        HEADER_WORDS.contains(&normalized.as_str())
    })
}

/// Recover a completion-table row from real table cells or the parser's
/// geometry-preserving `col | col` representation. Explicit numbered blanks
/// win. Ordinal row mapping is allowed only when one clear header plus exactly
/// one structurally consistent data row per question is present.
fn dynamic_table_row_prompt_for_number(
    blocks: &[Value],
    number: u32,
    range_start: u32,
    range_end: u32,
) -> Option<(String, String, bool)> {
    let expected_count = range_end.saturating_sub(range_start).saturating_add(1) as usize;
    let mut rows = Vec::new();
    for (block_id, cells) in dynamic_completion_rows(blocks) {
        let row_count = expected_count.saturating_add(1);
        let can_expand_flattened = row_count > 1
            && cells.len() >= row_count.saturating_mul(2)
            && cells.len() % row_count == 0;
        let column_count = can_expand_flattened.then_some(cells.len() / row_count);
        if let Some(column_count) = column_count
            .filter(|count| *count >= 2 && is_dynamic_completion_table_header(&cells[..*count]))
        {
            rows.extend(
                cells
                    .chunks(column_count)
                    .map(|row| (block_id.clone(), row.to_vec())),
            );
        } else {
            rows.push((block_id, cells));
        }
    }
    for (block_id, cells) in &rows {
        let has_explicit_number = cells.iter().any(|cell| {
            find_dynamic_numbered_blank_marker(cell, number, 0).is_some()
                || dynamic_leading_question_number(cell) == Some(number)
        });
        if has_explicit_number {
            return Some((cells.join(" | "), block_id.clone(), false));
        }
    }

    let data_rows = rows
        .into_iter()
        .filter(|(_, cells)| !is_dynamic_completion_table_header(cells))
        .collect::<Vec<_>>();
    let consistent_columns = data_rows
        .first()
        .map(|(_, cells)| cells.len())
        .filter(|count| *count >= 2)
        .is_some_and(|count| data_rows.iter().all(|(_, cells)| cells.len() == count));
    if data_rows.len() != expected_count || !consistent_columns || number < range_start {
        return None;
    }
    let row_index = number.saturating_sub(range_start) as usize;
    let (block_id, cells) = data_rows.get(row_index)?;
    Some((cells.join(" | "), block_id.clone(), true))
}

fn dynamic_declared_letter_bank_labels(text: &str) -> Vec<String> {
    let normalized = normalized_dynamic_instruction_text(text);
    let end = [
        ("a-n", 'N'),
        ("a-m", 'M'),
        ("a-l", 'L'),
        ("a-k", 'K'),
        ("a-j", 'J'),
        ("a-i", 'I'),
        ("a-h", 'H'),
        ("a-g", 'G'),
        ("a-f", 'F'),
        ("a-e", 'E'),
        ("a-d", 'D'),
        ("a-c", 'C'),
    ]
    .into_iter()
    .find_map(|(marker, end)| normalized.contains(marker).then_some(end))
    .or_else(|| dynamic_explicit_letter_list_terminal(text));
    let Some(end) = end else {
        return Vec::new();
    };

    ('A'..=end).map(|label| label.to_string()).collect()
}

fn has_dynamic_completion_option_bank_cue(text: &str) -> bool {
    let normalized = normalized_dynamic_instruction_text(text);
    [
        "list of words",
        "list of options",
        "list of phrases",
        "list of endings",
        "using the list",
        "using the words",
        "using the phrases",
        "from the list",
        "from the box",
        "in the box below",
        "in the box above",
        "in the following box",
        "words below",
        "options below",
        "phrases below",
        "endings below",
    ]
    .iter()
    .any(|cue| {
        normalized.match_indices(cue).any(|(start, _)| {
            let end = start + cue.len();
            normalized[..start]
                .chars()
                .next_back()
                .map(|ch| !ch.is_alphanumeric())
                .unwrap_or(true)
                && normalized[end..]
                    .chars()
                    .next()
                    .map(|ch| !ch.is_alphanumeric())
                    .unwrap_or(true)
        })
    })
}

fn validated_dynamic_completion_option_bank(
    blocks: &[Value],
    options: &[(String, String)],
) -> Vec<(String, String)> {
    let group_text = blocks
        .iter()
        .map(dynamic_block_text)
        .collect::<Vec<_>>()
        .join(" ");
    if !has_dynamic_completion_option_bank_cue(&group_text) {
        return Vec::new();
    }

    let declared_labels = dynamic_declared_letter_bank_labels(&group_text);
    let labels = if declared_labels.is_empty() {
        let mut contiguous = Vec::new();
        for label in 'A'..=DYNAMIC_MAX_OPTION_LABEL {
            let label = label.to_string();
            if options.iter().any(|(candidate, _)| candidate == &label) {
                contiguous.push(label);
            } else {
                break;
            }
        }
        if contiguous.len() < 2 {
            return Vec::new();
        }
        contiguous
    } else {
        declared_labels
    };

    let mut validated = Vec::with_capacity(labels.len());
    for label in labels {
        let Some((_, option_text)) = options
            .iter()
            .find(|(candidate, option_text)| candidate == &label && !option_text.trim().is_empty())
        else {
            // A declared A-H bank with a missing label is not safe to expose
            // as a partial selector; retain the normal free-text completion.
            return Vec::new();
        };
        validated.push((label, option_text.clone()));
    }
    validated
}

fn is_dynamic_same_row_option_continuation(previous: &Value, current: &Value) -> bool {
    if dynamic_block_page_index(previous) != dynamic_block_page_index(current)
        || dynamic_leading_option_label_and_text(&dynamic_block_text(current)).is_some()
        || is_dynamic_instruction_signal(&dynamic_block_text(current))
        || dynamic_leading_question_number(&dynamic_block_text(current)).is_some()
    {
        return false;
    }
    let (Some(left), Some(right)) = (
        dynamic_block_normalized_bbox(previous),
        dynamic_block_normalized_bbox(current),
    ) else {
        return false;
    };
    let vertical_overlap = left[3].min(right[3]) - left[1].max(right[1]);
    let horizontal_gap = right[0] - left[2];
    vertical_overlap >= -2.0 && (-3.0..=24.0).contains(&horizontal_gap)
}

fn is_dynamic_wrapped_option_continuation(previous: &Value, current: &Value) -> bool {
    if dynamic_block_page_index(previous) != dynamic_block_page_index(current)
        || dynamic_block_section_column_count_value(previous)
            != dynamic_block_section_column_count_value(current)
        || dynamic_block_column(previous) != dynamic_block_column(current)
        || dynamic_leading_option_label_and_text(&dynamic_block_text(current)).is_some()
        || is_dynamic_instruction_signal(&dynamic_block_text(current))
        || is_dynamic_prompt_terminal_heading(&dynamic_block_text(current))
        || dynamic_leading_question_number(&dynamic_block_text(current)).is_some()
    {
        return false;
    }
    let (Some(previous_bbox), Some(current_bbox)) = (
        dynamic_block_normalized_bbox(previous),
        dynamic_block_normalized_bbox(current),
    ) else {
        return false;
    };
    let vertical_gap = previous_bbox[1] - current_bbox[3];
    let previous_height = (previous_bbox[3] - previous_bbox[1]).abs().max(1.0);
    let current_height = (current_bbox[3] - current_bbox[1]).abs().max(1.0);
    let indented = current_bbox[0] >= previous_bbox[0] - 3.0;
    vertical_gap >= -2.0
        && vertical_gap <= (previous_height.max(current_height) * 1.9).max(18.0)
        && indented
}

fn append_dynamic_option_continuation(option_text: &mut String, continuation: &str) {
    let continuation = continuation.trim();
    if continuation.is_empty() {
        return;
    }
    let trailing_fragment = option_text
        .split_whitespace()
        .next_back()
        .map(|token| token.len() == 1 && token.chars().all(|ch| ch.is_ascii_uppercase()))
        .unwrap_or(false);
    let opens_lowercase = continuation
        .chars()
        .next()
        .map(|ch| ch.is_ascii_lowercase())
        .unwrap_or(false);
    let mut continuation_parts = continuation.splitn(2, char::is_whitespace);
    let first_continuation_word = continuation_parts.next().unwrap_or_default();
    let remaining_continuation = continuation_parts.next().unwrap_or_default().trim();
    let single_lowercase_fragment = first_continuation_word.len() == 1
        && first_continuation_word
            .chars()
            .all(|ch| ch.is_ascii_lowercase())
        && option_text
            .split_whitespace()
            .next_back()
            .map(|token| token.len() >= 2 && token.chars().all(|ch| ch.is_ascii_alphabetic()))
            .unwrap_or(false);
    if trailing_fragment && opens_lowercase {
        option_text.push_str(continuation);
    } else if single_lowercase_fragment {
        option_text.push_str(first_continuation_word);
        if !remaining_continuation.is_empty() {
            option_text.push(' ');
            option_text.push_str(remaining_continuation);
        }
    } else {
        *option_text = collapse_whitespace(&format!("{} {}", option_text, continuation));
    }
}

fn dynamic_group_option_bank_unbounded(blocks: &[Value], kind: &str) -> Vec<(String, String)> {
    let completion_kind = is_dynamic_completion_kind(kind);
    if !completion_kind
        && !matches!(
            kind,
            "heading_matching" | "matching" | "matching_information" | "classification"
        )
    {
        return Vec::new();
    }
    let group_text = blocks
        .iter()
        .map(dynamic_block_text)
        .collect::<Vec<_>>()
        .join(" ");
    let declared_roman_labels = if kind == "heading_matching" {
        dynamic_declared_roman_bank_labels(&group_text)
    } else {
        Vec::new()
    };
    // A heading-matching instruction commonly says both `paragraphs A-H`
    // and `headings i-x`. A-H describes the question prompts, not the answer
    // bank, so letter closure must never run for this kind.
    let declared_terminal = (kind != "heading_matching")
        .then(|| dynamic_declared_letter_bank_labels(&group_text))
        .unwrap_or_default()
        .last()
        .and_then(|label| label.chars().next());
    let declared_bank_labels = if completion_kind {
        dynamic_declared_letter_bank_labels(&group_text)
    } else {
        Vec::new()
    };
    let mut options = Vec::new();
    let mut previous_option_block: Option<&Value> = None;
    for block_index in 0..blocks.len() {
        let block = &blocks[block_index];
        let mut accepted_table_option = false;
        for (label, option_text) in dynamic_table_option_rows(block) {
            let valid = if kind == "heading_matching" {
                matches!(
                    label.as_str(),
                    "i" | "ii"
                        | "iii"
                        | "iv"
                        | "v"
                        | "vi"
                        | "vii"
                        | "viii"
                        | "ix"
                        | "x"
                        | "xi"
                        | "xii"
                )
            } else {
                label.len() == 1 && label.chars().all(is_dynamic_letter_option_label)
            };
            if valid
                && !options
                    .iter()
                    .any(|item: &(String, String)| item.0 == label)
            {
                options.push((label, option_text));
                accepted_table_option = true;
            }
        }
        if accepted_table_option {
            previous_option_block = Some(block);
            continue;
        }
        let text = dynamic_block_text(block);
        if is_dynamic_prompt_option_bank_heading(&text) {
            // A fragmented instruction can look like an option (`B or C.`)
            // before the explicit bank heading. The heading establishes the
            // semantic bank boundary, so discard any provisional labels and
            // rebuild only from the declared rows that follow it.
            options.clear();
            previous_option_block = None;
            continue;
        }
        if dynamic_leading_question_number(&text).is_some() {
            previous_option_block = None;
            continue;
        }
        if completion_kind {
            if let Some(inline_options) = dynamic_declared_bank_parts(&text, &declared_bank_labels)
            {
                if let Some((first_label, _)) = inline_options.first() {
                    if let Some(first_label) = first_label.chars().next() {
                        if first_label > 'A' {
                            let normalized = collapse_whitespace(&text);
                            if let Some((marker_start, _)) =
                                find_dynamic_option_marker(&normalized, first_label, 0)
                            {
                                let prefix = normalized[..marker_start].trim();
                                let prefix_is_fragment = prefix.split_whitespace().count() <= 3
                                    && prefix.chars().all(|ch| ch.is_ascii_alphabetic())
                                    && prefix
                                        .chars()
                                        .next()
                                        .is_some_and(|ch| ch.is_ascii_lowercase());
                                if prefix_is_fragment {
                                    let predecessor = (first_label as u8 - 1) as char;
                                    if let Some((_, predecessor_text)) = options
                                        .iter_mut()
                                        .find(|(label, _)| label == &predecessor.to_string())
                                    {
                                        append_dynamic_option_continuation(
                                            predecessor_text,
                                            prefix,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                for (label, option_text) in inline_options {
                    if !options
                        .iter()
                        .any(|item: &(String, String)| item.0 == label)
                    {
                        options.push((label, option_text));
                    }
                }
                previous_option_block = Some(block);
                continue;
            }
        }
        if kind != "heading_matching" {
            let expected_label = options
                .last()
                .and_then(|(label, _)| label.chars().next())
                .and_then(|label| {
                    (label < DYNAMIC_MAX_OPTION_LABEL).then_some((label as u8 + 1) as char)
                });
            let inline_options =
                dynamic_leading_option_label_and_text(&text).and_then(|(label, _)| {
                    let first_label = label.chars().next()?;
                    if label.len() != 1 || !matches!(first_label, 'A'..='M') {
                        return None;
                    }
                    dynamic_inline_choice_parts_from_label(&text, first_label).or_else(|| {
                        // A final two-label tail (for example G/H in a
                        // declared A-H bank) is safe only when earlier labels
                        // already establish the sequence and this block ends
                        // exactly at the declared terminal.
                        let terminal = declared_terminal?;
                        if terminal <= first_label {
                            return None;
                        }
                        dynamic_inline_choice_parts_from_label_with_minimum(&text, first_label, 2)
                            .filter(|(_, recovered)| {
                                let labels = recovered
                                    .iter()
                                    .filter_map(|(label, _)| label.chars().next())
                                    .collect::<Vec<_>>();
                                labels.last().is_some_and(|last| *last <= terminal)
                                    && labels
                                        .windows(2)
                                        .all(|pair| pair[1] as u8 == pair[0] as u8 + 1)
                                    && (expected_label == Some(first_label)
                                        || declared_terminal.is_some())
                            })
                    })
                });
            if let Some((prompt, inline_options)) = inline_options {
                if prompt.is_empty() {
                    for (label, option_text) in inline_options {
                        if !options
                            .iter()
                            .any(|item: &(String, String)| item.0 == label)
                        {
                            options.push((label, option_text));
                        }
                    }
                    previous_option_block = Some(block);
                    continue;
                }
            }
        }
        let Some((label, option_text)) = dynamic_leading_option_label_and_text(&text) else {
            let candidate_terminal = declared_terminal.unwrap_or(DYNAMIC_MAX_OPTION_LABEL);
            let candidate_label = if completion_kind && declared_terminal.is_some() {
                dynamic_completion_bank_candidate_labels(&text, candidate_terminal)
            } else {
                dynamic_bank_candidate_labels(&text, candidate_terminal)
            }
            .into_iter()
            .find(|label| {
                !options
                    .iter()
                    .any(|(existing, _)| existing == &label.to_string())
            });
            if let Some(candidate_label) = candidate_label {
                if let Some((marker_start, _)) =
                    find_dynamic_option_marker(&text, candidate_label, 0)
                {
                    let prefix = collapse_whitespace(&text[..marker_start]);
                    if !prefix.is_empty()
                        && !is_dynamic_instruction_signal(&prefix)
                        && !is_dynamic_prompt_terminal_heading(&prefix)
                    {
                        let predecessor = (candidate_label as u8).checked_sub(1).map(char::from);
                        if let Some(predecessor) = predecessor {
                            let predecessor_block =
                                blocks[..block_index].iter().rev().find(|previous| {
                                    find_dynamic_option_marker(
                                        &dynamic_block_text(previous),
                                        predecessor,
                                        0,
                                    )
                                    .is_some()
                                });
                            let same_row = predecessor_block.is_some_and(|previous| {
                                is_dynamic_same_row_option_continuation(previous, block)
                            });
                            let tight_gap = predecessor_block
                                .and_then(|previous| {
                                    Some((
                                        dynamic_block_normalized_bbox(previous)?,
                                        dynamic_block_normalized_bbox(block)?,
                                    ))
                                })
                                .is_some_and(|(previous_bbox, current_bbox)| {
                                    let gap = current_bbox[0] - previous_bbox[2];
                                    (-2.0..=8.0).contains(&gap)
                                });
                            if let Some((_, predecessor_text)) = options
                                .iter_mut()
                                .find(|(label, _)| label == &predecessor.to_string())
                            {
                                let prefix_first =
                                    prefix.split_whitespace().next().unwrap_or_default();
                                let trailing_fragment = predecessor_text
                                    .split_whitespace()
                                    .next_back()
                                    .unwrap_or_default();
                                let joins_word = same_row
                                    && prefix_first
                                        .chars()
                                        .next()
                                        .is_some_and(|ch| ch.is_ascii_lowercase())
                                    && prefix_first.chars().all(|ch| ch.is_ascii_alphabetic())
                                    && (tight_gap
                                        || ((2..=4).contains(&trailing_fragment.len())
                                            && trailing_fragment
                                                .chars()
                                                .all(|ch| ch.is_ascii_lowercase())));
                                if joins_word {
                                    predecessor_text.push_str(&prefix);
                                } else {
                                    append_dynamic_option_continuation(predecessor_text, &prefix);
                                }
                            }
                        }
                    }
                    let option_tail = collapse_whitespace(&text[marker_start..]);
                    let recovered = dynamic_inline_choice_parts_from_label_with_minimum(
                        &option_tail,
                        candidate_label,
                        2,
                    )
                    .map(|(_, recovered)| recovered)
                    .or_else(|| {
                        dynamic_leading_option_label_and_text(&option_tail).and_then(
                            |(label, option_text)| {
                                (label == candidate_label.to_string()
                                    && !option_text.trim().is_empty())
                                .then_some(vec![(label, option_text)])
                            },
                        )
                    });
                    if let Some(recovered) = recovered {
                        let additions = recovered
                            .into_iter()
                            .filter(|(label, _)| {
                                !options.iter().any(|(existing, _)| existing == label)
                            })
                            .collect::<Vec<_>>();
                        options.extend(additions);
                        previous_option_block = Some(block);
                        continue;
                    }
                }
            }
            let expected_label = options
                .last()
                .and_then(|(label, _)| label.chars().next())
                .and_then(|label| {
                    (label < DYNAMIC_MAX_OPTION_LABEL).then_some((label as u8 + 1) as char)
                });
            if completion_kind {
                let pending_label = options
                    .iter()
                    .find(|(_, option_text)| option_text.trim().is_empty())
                    .and_then(|(label, _)| label.chars().next());
                if let Some(pending_label) = pending_label {
                    let has_pending_block = blocks[..block_index].iter().rev().any(|previous| {
                        find_dynamic_option_marker(&dynamic_block_text(previous), pending_label, 0)
                            .is_some_and(|(_, content_start)| {
                                dynamic_block_text(previous)[content_start..]
                                    .trim()
                                    .is_empty()
                            })
                            && (is_dynamic_same_row_option_continuation(previous, block)
                                || is_dynamic_wrapped_option_continuation(previous, block))
                    });
                    if has_pending_block {
                        if let Some((_, pending_text)) =
                            options.iter_mut().find(|(label, option_text)| {
                                label.chars().next() == Some(pending_label)
                                    && option_text.trim().is_empty()
                            })
                        {
                            append_dynamic_option_continuation(pending_text, &text);
                            previous_option_block = Some(block);
                            continue;
                        }
                    }
                }
            }
            if let (Some(previous), Some((_, last_text))) =
                (previous_option_block, options.last_mut())
            {
                if let Some(expected_label) = expected_label {
                    if let Some((marker_start, _)) =
                        find_dynamic_option_marker(&text, expected_label, 0)
                    {
                        let prefix = collapse_whitespace(&text[..marker_start]);
                        if !prefix.is_empty()
                            && !is_dynamic_instruction_signal(&prefix)
                            && !is_dynamic_prompt_terminal_heading(&prefix)
                        {
                            let same_row = is_dynamic_same_row_option_continuation(previous, block);
                            let split_word_fragment = same_row
                                && prefix.split_whitespace().count() == 1
                                && prefix.chars().all(|ch| ch.is_ascii_alphabetic())
                                && prefix
                                    .chars()
                                    .next()
                                    .is_some_and(|ch| ch.is_ascii_lowercase())
                                && last_text
                                    .chars()
                                    .next_back()
                                    .is_some_and(|ch| ch.is_ascii_alphabetic());
                            if split_word_fragment {
                                last_text.push_str(&prefix);
                            } else {
                                append_dynamic_option_continuation(last_text, &prefix);
                            }
                        }

                        let option_tail = collapse_whitespace(&text[marker_start..]);
                        let recovered = dynamic_inline_choice_parts_from_label_with_minimum(
                            &option_tail,
                            expected_label,
                            2,
                        )
                        .map(|(_, recovered)| recovered)
                        .or_else(|| {
                            dynamic_leading_option_label_and_text(&option_tail).and_then(
                                |(label, option_text)| {
                                    (label == expected_label.to_string()
                                        && !option_text.trim().is_empty())
                                    .then_some(vec![(label, option_text)])
                                },
                            )
                        });
                        if let Some(recovered) = recovered {
                            for (label, option_text) in recovered {
                                if !options
                                    .iter()
                                    .any(|item: &(String, String)| item.0 == label)
                                {
                                    options.push((label, option_text));
                                }
                            }
                            previous_option_block = Some(block);
                            continue;
                        }
                    }
                }
                if is_dynamic_same_row_option_continuation(previous, block)
                    || is_dynamic_wrapped_option_continuation(previous, block)
                {
                    append_dynamic_option_continuation(last_text, &text);
                    previous_option_block = Some(block);
                }
            }
            continue;
        };
        let valid = if kind == "heading_matching" {
            matches!(
                label.as_str(),
                "i" | "ii" | "iii" | "iv" | "v" | "vi" | "vii" | "viii" | "ix" | "x" | "xi" | "xii"
            )
        } else {
            label.len() == 1 && label.chars().all(is_dynamic_letter_option_label)
        };
        if valid
            && !options
                .iter()
                .any(|item: &(String, String)| item.0 == label)
        {
            // A standalone label can be split from its text by a narrow
            // multi-column layout (`G` on one row, `complex H ...` on the
            // next). Keep an empty placeholder only for a declared completion
            // bank; the bounded continuation logic below must fill it before
            // validation can expose the bank.
            if !option_text.is_empty() || (completion_kind && declared_terminal.is_some()) {
                options.push((label, option_text));
            }
            previous_option_block = Some(block);
        }
    }
    if kind == "heading_matching" && !declared_roman_labels.is_empty() {
        let mut closed = Vec::with_capacity(declared_roman_labels.len());
        for label in declared_roman_labels {
            let Some((_, text)) = options
                .iter()
                .find(|(candidate, text)| candidate == &label && !text.trim().is_empty())
            else {
                return Vec::new();
            };
            closed.push((label, text.clone()));
        }
        closed
    } else if completion_kind {
        validated_dynamic_completion_option_bank(blocks, &options)
    } else if let Some(terminal) = declared_terminal {
        let mut closed = Vec::new();
        for label in 'A'..=terminal {
            let label = label.to_string();
            let Some((_, text)) = options
                .iter()
                .find(|(candidate, text)| candidate == &label && !text.trim().is_empty())
            else {
                return Vec::new();
            };
            closed.push((label, text.clone()));
        }
        closed
    } else {
        options
    }
}

fn is_dynamic_completion_bank_hard_boundary(block: &Value) -> bool {
    let text = dynamic_block_text(block);
    detect_dynamic_question_heading_range(&text).is_some()
        || dynamic_leading_question_number(&text).is_some()
        || is_dynamic_reading_passage_heading(&text)
        || is_dynamic_answer_block(block)
}

fn is_dynamic_first_person_i_prose(text: &str) -> bool {
    let Some((label, content)) = dynamic_leading_option_label_and_text(text) else {
        return false;
    };
    if label != "I" {
        return false;
    }
    let first_word = content
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|ch: char| !ch.is_ascii_alphabetic())
        .to_ascii_lowercase();
    matches!(
        first_word.as_str(),
        "am" | "was"
            | "have"
            | "had"
            | "do"
            | "did"
            | "think"
            | "believe"
            | "agree"
            | "argue"
            | "consider"
            | "feel"
            | "find"
            | "know"
            | "see"
            | "want"
            | "need"
            | "expect"
            | "can"
            | "could"
            | "will"
            | "would"
            | "should"
            | "may"
            | "might"
    )
}

fn is_dynamic_declared_completion_terminal_source(
    block: &Value,
    declared_labels: &[String],
    terminal: char,
    declaration_text: &str,
) -> bool {
    if dynamic_table_option_rows(block)
        .iter()
        .any(|(label, option_text)| {
            label == &terminal.to_string() && !option_text.trim().is_empty()
        })
    {
        return true;
    }
    let text = dynamic_block_text(block);
    if dynamic_declared_bank_parts(&text, declared_labels).is_some_and(|parts| {
        parts.into_iter().any(|(label, option_text)| {
            label == terminal.to_string() && !option_text.trim().is_empty()
        })
    }) {
        return true;
    }
    // A cross-column word tail can precede the terminal marker in the same
    // physical block (`urprising but satisfying I magical`). The full bank
    // parser intentionally needs two markers, but the terminal source check
    // may accept this one marker when its lower-case prefix is a bounded word
    // fragment rather than ordinary sentence prose.
    let normalized = collapse_whitespace(&text);
    if let Some((marker_start, content_start)) =
        find_dynamic_option_marker(&normalized, terminal, 0)
    {
        let prefix = normalized[..marker_start.min(normalized.len())].trim();
        let prefix_is_fragment = prefix.is_empty()
            || (terminal > 'A'
                && prefix.split_whitespace().count() <= 3
                && prefix
                    .split_whitespace()
                    .all(|word| word.chars().all(|ch| ch.is_ascii_lowercase())));
        if prefix_is_fragment
            && normalized[content_start.min(normalized.len())..]
                .trim()
                .len()
                > 0
        {
            if terminal != 'I' || !is_dynamic_first_person_i_prose(&text) {
                return true;
            }
        }
    }
    let terminal_is_leading =
        dynamic_leading_option_label_and_text(&text).is_some_and(|(label, option_text)| {
            label == terminal.to_string() && !option_text.trim().is_empty()
        });
    if !terminal_is_leading {
        return false;
    }
    // A bare capital I is also the English first-person pronoun. In word and
    // phrase banks, an `I believe ...` passage continuation is not source
    // evidence for option I. `list of endings` remains exempt because genuine
    // ending banks can intentionally contain complete clauses.
    let declaration = normalized_dynamic_instruction_text(declaration_text);
    terminal != 'I'
        || declaration.contains("list of endings")
        || !is_dynamic_first_person_i_prose(&text)
}

/// Locate the smallest source-backed span that closes an explicitly declared
/// completion bank. A and the terminal alone are insufficient: every declared
/// label must be recovered with non-empty source text before a task boundary.
/// Candidate A starts are tried from right to left so an earlier stimulus line
/// beginning `A ... B ...` cannot steal labels from the real bank that follows.
fn dynamic_declared_completion_bank_span(kind: &str, blocks: &[Value]) -> Option<(usize, usize)> {
    if !is_dynamic_completion_kind(kind) {
        return None;
    }
    let declaration_text = blocks
        .iter()
        .map(dynamic_block_text)
        .collect::<Vec<_>>()
        .join(" ");
    if !has_dynamic_completion_option_bank_cue(&declaration_text) {
        return None;
    }
    let declared_labels = dynamic_declared_letter_bank_labels(&declaration_text);
    let terminal = declared_labels
        .last()
        .and_then(|label| label.chars().next())?;
    if declared_labels.len() < 3 {
        return None;
    }

    let starts = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let text = dynamic_block_text(block);
            let leading_a = dynamic_leading_option_label_and_text(&text)
                .is_some_and(|(label, option_text)| label == "A" && !option_text.trim().is_empty());
            let table_a = dynamic_table_option_rows(block)
                .iter()
                .any(|(label, option_text)| label == "A" && !option_text.trim().is_empty());
            (leading_a || table_a).then_some(index)
        })
        .collect::<Vec<_>>();

    for start in starts.into_iter().rev() {
        let boundary = blocks
            .iter()
            .enumerate()
            .skip(start + 1)
            .find_map(|(index, block)| {
                is_dynamic_completion_bank_hard_boundary(block).then_some(index)
            })
            .unwrap_or(blocks.len());
        let mut terminal_marker_seen = false;
        for end in start + 1..=boundary {
            let terminal_block = &blocks[end - 1];
            terminal_marker_seen |=
                find_dynamic_option_marker(&dynamic_block_text(terminal_block), terminal, 0)
                    .is_some();
            // The terminal label itself may be the final glyph in one column
            // and acquire its text only from a later same-row block. Wait for
            // the marker, then let full A-terminal source closure decide when
            // the span is complete.
            if !terminal_marker_seen {
                continue;
            }
            let declaration = json!({
                "blockId": "synthetic-declared-completion-bank-context",
                "text": format!("Complete using the list of words, A-{terminal}, below.")
            });
            let scoped = std::iter::once(declaration)
                .chain(blocks[start..end].iter().cloned())
                .collect::<Vec<_>>();
            let recovered = dynamic_group_option_bank_unbounded(&scoped, kind);
            if recovered.len() == declared_labels.len()
                && recovered
                    .iter()
                    .zip(&declared_labels)
                    .all(|((actual, text), expected)| actual == expected && !text.trim().is_empty())
            {
                return Some((start, end));
            }
        }
    }
    None
}

/// Geometry may place one or two bank cells after the following question
/// heading in linear block order; `extend_dynamic_matching_option_blocks`
/// reattaches those cells later by bbox. For task/passage ownership, accept a
/// nearly closed prefix only when most of the declared labels and the genuine
/// terminal are already source-backed. This is deliberately much stronger
/// than the former A+terminal shortcut and cannot be satisfied by A/B stimulus
/// prose followed by an ordinary first-person `I ...` sentence.
fn dynamic_partial_completion_bank_task_end(kind: &str, blocks: &[Value]) -> Option<usize> {
    if !is_dynamic_completion_kind(kind) {
        return None;
    }
    let declaration_text = blocks
        .iter()
        .map(dynamic_block_text)
        .collect::<Vec<_>>()
        .join(" ");
    if !has_dynamic_completion_option_bank_cue(&declaration_text) {
        return None;
    }
    let declared_labels = dynamic_declared_letter_bank_labels(&declaration_text);
    let terminal = declared_labels
        .last()
        .and_then(|label| label.chars().next())?;
    if declared_labels.len() < 5 {
        return None;
    }
    let declared = declared_labels
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut observed = std::collections::BTreeSet::<String>::new();
    let mut terminal_end = None;
    for (index, block) in blocks.iter().enumerate() {
        for (label, option_text) in dynamic_table_option_rows(block) {
            if declared.contains(&label) && !option_text.trim().is_empty() {
                observed.insert(label);
            }
        }
        let text = dynamic_block_text(block);
        if let Some(parts) = dynamic_declared_bank_parts(&text, &declared_labels) {
            observed.extend(parts.into_iter().filter_map(|(label, option_text)| {
                (declared.contains(&label) && !option_text.trim().is_empty()).then_some(label)
            }));
        } else if let Some((label, option_text)) = dynamic_leading_option_label_and_text(&text) {
            if declared.contains(&label) && !option_text.trim().is_empty() {
                observed.insert(label);
            }
        }
        if is_dynamic_declared_completion_terminal_source(
            block,
            &declared_labels,
            terminal,
            &declaration_text,
        ) {
            terminal_end = Some(index + 1);
        }
    }
    let minimum_source_labels = declared_labels.len().saturating_sub(2).max(5);
    (observed.len() >= minimum_source_labels).then_some(terminal_end?)
}

fn dynamic_group_option_bank(blocks: &[Value], kind: &str) -> Vec<(String, String)> {
    if is_dynamic_completion_kind(kind) {
        let declaration_text = blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join(" ");
        let declared_labels = dynamic_declared_letter_bank_labels(&declaration_text);
        if has_dynamic_completion_option_bank_cue(&declaration_text) && !declared_labels.is_empty()
        {
            let Some((start, end)) = dynamic_declared_completion_bank_span(kind, blocks) else {
                return Vec::new();
            };
            let terminal = declared_labels
                .last()
                .and_then(|label| label.chars().next())
                .unwrap_or('A');
            let declaration = json!({
                "blockId": "synthetic-declared-completion-bank-context",
                "text": format!("Complete using the list of words, A-{terminal}, below.")
            });
            let scoped = std::iter::once(declaration)
                .chain(blocks[start..end].iter().cloned())
                .collect::<Vec<_>>();
            return dynamic_group_option_bank_unbounded(&scoped, kind);
        }
    }
    if matches!(kind, "matching" | "matching_information" | "classification") {
        let declaration_text = blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join(" ");
        let declared_labels = dynamic_declared_letter_bank_labels(&declaration_text);
        if declared_labels.len() >= 3 {
            // A question/stimulus line can legitimately begin with the
            // article A. Prefer the right-most A source whose suffix closes
            // the entire declared bank, rather than letting that prose claim
            // option A and shadow the real bank below it.
            for start in (0..blocks.len()).rev() {
                let starts_at_a = dynamic_table_option_rows(&blocks[start])
                    .iter()
                    .any(|(label, text)| label == "A" && !text.trim().is_empty())
                    || dynamic_leading_option_label_and_text(&dynamic_block_text(&blocks[start]))
                        .is_some_and(|(label, text)| label == "A" && !text.trim().is_empty());
                if !starts_at_a {
                    continue;
                }
                let recovered = dynamic_group_option_bank_unbounded(&blocks[start..], kind);
                if recovered.len() == declared_labels.len()
                    && recovered
                        .iter()
                        .zip(&declared_labels)
                        .all(|((label, text), declared)| {
                            label == declared && !text.trim().is_empty()
                        })
                {
                    return recovered;
                }
            }
        }
    }
    dynamic_group_option_bank_unbounded(blocks, kind)
}

/// Return the physical start of a separately declared shared option bank.
///
/// Looking only for a leading `A` is unsafe because normal question prose can
/// begin with the article A.  Instead, prove that the suffix beginning at that
/// block reproduces the complete, explicitly declared A-terminal bank already
/// recovered for the group.  Trying candidates from right to left ensures an
/// earlier prose `A ...` cannot shadow the genuine bank.
fn dynamic_group_option_bank_start_index(blocks: &[Value], kind: &str) -> Option<usize> {
    if !matches!(
        kind,
        "heading_matching" | "matching" | "matching_information" | "classification"
    ) {
        return None;
    }
    let declaration_text = blocks
        .iter()
        .map(dynamic_block_text)
        .collect::<Vec<_>>()
        .join(" ");
    let declared_labels = if kind == "heading_matching" {
        dynamic_declared_roman_bank_labels(&declaration_text)
    } else {
        dynamic_declared_letter_bank_labels(&declaration_text)
    };
    if declared_labels.len() < 3 {
        return None;
    }
    let expected = dynamic_group_option_bank(blocks, kind);
    if expected.len() != declared_labels.len()
        || expected
            .iter()
            .zip(&declared_labels)
            .any(|((label, text), declared)| label != declared || text.trim().is_empty())
    {
        return None;
    }

    blocks.iter().enumerate().rev().find_map(|(index, block)| {
        let starts_at_first_label = if kind == "heading_matching" {
            dynamic_table_option_rows(block)
                .iter()
                .any(|(label, text)| label == &declared_labels[0] && !text.trim().is_empty())
        } else {
            dynamic_table_option_rows(block)
                .iter()
                .any(|(label, text)| label == &declared_labels[0] && !text.trim().is_empty())
                || dynamic_leading_option_label_and_text(&dynamic_block_text(block)).is_some_and(
                    |(label, text)| label == declared_labels[0] && !text.trim().is_empty(),
                )
        };
        if !starts_at_first_label {
            return None;
        }
        let recovered = dynamic_group_option_bank_unbounded(&blocks[index..], kind);
        (recovered == expected).then_some(index)
    })
}

fn dynamic_interaction_with_option_texts(
    candidate: &Value,
    kind: &str,
    option_texts: &[(String, String)],
) -> Value {
    let mut interaction = dynamic_interaction_from_candidate(candidate, kind);
    if option_texts.is_empty() {
        return interaction;
    }
    let labels = option_texts
        .iter()
        .map(|(label, _)| Value::String(label.clone()))
        .collect::<Vec<_>>();
    let texts = option_texts
        .iter()
        .map(|(label, text)| (label.clone(), Value::String(text.clone())))
        .collect::<serde_json::Map<_, _>>();
    if let Some(object) = interaction.as_object_mut() {
        if is_dynamic_completion_kind(kind) {
            object.insert("type".to_string(), Value::String("matching".to_string()));
            object.remove("placeholder");
            if !object.contains_key("allowOptionReuse") {
                object.insert("allowOptionReuse".to_string(), Value::Bool(false));
            }
        }
        object.insert("options".to_string(), Value::Array(labels));
        object.insert("optionTexts".to_string(), Value::Object(texts));
    }
    interaction
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

fn dynamic_bounded_recovery_blocks(
    blocks: &[Value],
    candidates: &[Value],
    candidate_index: usize,
    current_block_ids: &[String],
) -> Vec<Value> {
    let positions_for = |ids: &[String]| {
        ids.iter()
            .filter_map(|id| {
                blocks
                    .iter()
                    .position(|block| dynamic_block_id(block) == *id)
            })
            .collect::<Vec<_>>()
    };
    let current_positions = positions_for(current_block_ids);
    let Some(current_start) = current_positions.iter().min().copied() else {
        return Vec::new();
    };
    let current_end = current_positions
        .iter()
        .max()
        .copied()
        .unwrap_or(current_start);
    let next_group_start = candidates
        .iter()
        .skip(candidate_index + 1)
        .filter_map(|candidate| {
            let ids = candidate
                .get("blockIds")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            positions_for(&ids)
                .into_iter()
                .filter(|position| *position > current_end)
                .min()
        })
        .min()
        .unwrap_or(blocks.len());
    let answer_start = blocks
        .iter()
        .enumerate()
        .skip(current_end.saturating_add(1))
        .find(|(_, block)| is_dynamic_answer_block(block))
        .map(|(index, _)| index)
        .unwrap_or(blocks.len());
    let end = next_group_start.min(answer_start).max(current_end + 1);
    blocks[current_start..end.min(blocks.len())]
        .iter()
        .filter(|block| {
            !is_dynamic_non_content_placeholder_block(block)
                && !is_dynamic_answer_block(block)
                && dynamic_block_role(block) != "answer"
                && dynamic_block_role(block) != "ignore"
        })
        .cloned()
        .collect()
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

    let group_candidates = split
        .get("questionGroupCandidates")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let groups = group_candidates
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
            let group_blocks = block_ids
                .iter()
                .filter_map(|block_id| blocks_by_id.get(block_id))
                .cloned()
                .collect::<Vec<_>>();
            let recovery_blocks = dynamic_bounded_recovery_blocks(
                &blocks,
                &group_candidates,
                index,
                &block_ids,
            );
            let group_text = {
                let text = group_blocks
                    .iter()
                    .map(dynamic_block_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.trim().is_empty() { instruction_text.to_string() } else { text }
            };
            let (start, end) = dynamic_range_from_candidate(candidate);
            let mut review_warnings = candidate
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
            let layout_hint = candidate
                .get("layoutHint")
                .and_then(Value::as_str)
                .unwrap_or_else(|| dynamic_layout_hint_for_group(kind, &group_text));
            let group_option_bank = dynamic_group_option_bank(&group_blocks, kind);
            let table_row_mapping_inferred = std::cell::Cell::new(false);
            let questions = (start..=end)
                .map(|number| {
                    let display = number.to_string();
                    let qid = format!("q{}", display);
                    let mut manual_import = requires_manual_question_import
                        || (matches!(kind, "short_answer" | "matching")
                            && group_blocks.len() == 1
                            && is_dynamic_umbrella_question_range(
                                &dynamic_block_text(group_blocks.first().unwrap_or(&Value::Null)),
                            ));
                    let mut question_source_block_ids = block_ids.clone();
                    let completion_recovery = !manual_import && is_dynamic_completion_kind(kind);
                    let (mut prompt, mut question_options) = if manual_import {
                        (String::new(), Vec::new())
                    } else {
                        dynamic_question_prompt_and_options(
                            &group_blocks,
                            &group_text,
                            number,
                            heading,
                            end,
                            kind,
                        )
                    };
                    if !manual_import && matches!(kind, "single_choice" | "multi_choice") {
                        let declared_labels = dynamic_letter_options_for_text(&group_text);
                        let primary_complete = !declared_labels.is_empty()
                            && question_options.len() == declared_labels.len()
                            && question_options
                                .iter()
                                .zip(declared_labels.iter())
                                .all(|((label, text), declared)| {
                                    label == declared && !text.trim().is_empty()
                                });
                        if !primary_complete && recovery_blocks.len() > group_blocks.len() {
                            let (recovered_prompt, recovered_options) =
                                dynamic_question_prompt_and_options(
                                    &recovery_blocks,
                                    &group_text,
                                    number,
                                    heading,
                                    end,
                                    kind,
                                );
                            let recovered_complete = recovered_options.len()
                                == declared_labels.len()
                                && recovered_options
                                    .iter()
                                    .zip(declared_labels.iter())
                                    .all(|((label, text), declared)| {
                                        label == declared && !text.trim().is_empty()
                                    });
                            if recovered_complete {
                                question_options = recovered_options;
                                // Preserve the primary prompt unless the
                                // expanded physical window supplies a clear
                                // question ending. This prevents a trailing
                                // passage line from replacing a good stem.
                                if (prompt.trim().is_empty()
                                    || (!prompt.contains('?')
                                        && recovered_prompt.contains('?')))
                                    && !recovered_prompt.trim().is_empty()
                                {
                                    prompt = recovered_prompt;
                                }
                                for block in &recovery_blocks {
                                    let source_id = dynamic_block_id(block);
                                    if !source_id.is_empty()
                                        && !question_source_block_ids
                                            .iter()
                                            .any(|id| id == &source_id)
                                    {
                                        question_source_block_ids.push(source_id);
                                    }
                                }
                            }
                        }
                    }
                    if completion_recovery && layout_hint == "inline_completion" {
                        if let Some((recovered, source_ids)) = dynamic_note_row_prompt_for_number(
                            &group_blocks,
                            number,
                            start,
                            end,
                        ) {
                            // Prefer the geometry-closed note row over the
                            // generic linear prompt.  The latter starts after
                            // the numeric marker and can therefore discard the
                            // row prefix or run into the following section.
                            prompt = recovered;
                            for source_id in source_ids {
                                if !question_source_block_ids.iter().any(|id| id == &source_id) {
                                    question_source_block_ids.push(source_id);
                                }
                            }
                        }
                    }
                    if completion_recovery && prompt.trim().is_empty() {
                        if let Some((recovered, block_id, inferred)) =
                            dynamic_table_row_prompt_for_number(&group_blocks, number, start, end)
                        {
                            prompt = recovered;
                            if inferred {
                                table_row_mapping_inferred.set(true);
                            }
                            if !block_id.is_empty()
                                && !question_source_block_ids.iter().any(|id| id == &block_id)
                            {
                                question_source_block_ids.push(block_id);
                            }
                        }
                        if prompt.trim().is_empty() {
                            if let Some((recovered, source_ids)) =
                                dynamic_gap_sentence_prompt_from_blocks(&recovery_blocks, number)
                            {
                                prompt = recovered;
                                for source_id in source_ids {
                                    if !question_source_block_ids.iter().any(|id| id == &source_id)
                                    {
                                        question_source_block_ids.push(source_id);
                                    }
                                }
                            }
                        }
                    }
                    if completion_recovery
                        && !dynamic_completion_foreign_slots(&prompt, number, start, end).is_empty()
                    {
                        if let Some((localized, source_id)) =
                            dynamic_local_completion_prompt_for_number(
                                &recovery_blocks,
                                number,
                                start,
                                end,
                            )
                        {
                            prompt = localized;
                            if !source_id.is_empty()
                                && !question_source_block_ids.iter().any(|id| id == &source_id)
                            {
                                question_source_block_ids.push(source_id);
                            }
                        }
                    }
                    if completion_recovery
                        && !dynamic_completion_foreign_slots(&prompt, number, start, end).is_empty()
                    {
                        // A prompt that still contains another response slot
                        // is not a closed single-question surface. Keep the
                        // source text for author repair, but never mark it
                        // publishable as an automatically recovered question.
                        manual_import = true;
                    }
                    if prompt.trim().is_empty() {
                        // A missing source-backed prompt is a hard authoring
                        // blocker. Never manufacture a range/instruction
                        // placeholder that could be mistaken for a recovered
                        // question and published to students.
                        manual_import = true;
                    }
                    let option_texts = if question_options.is_empty() {
                        group_option_bank.as_slice()
                    } else {
                        question_options.as_slice()
                    };
                    QuestionDraftV1 {
                        id: qid,
                        display_number: display.clone(),
                        prompt,
                        interaction: dynamic_interaction_with_option_texts(
                            candidate,
                            kind,
                            option_texts,
                        ),
                        answer: answer_by_display
                            .get(&number.to_string())
                            .cloned()
                            .unwrap_or_else(|| json!("")),
                        source_block_ids: question_source_block_ids,
                        confidence: candidate
                            .get("confidence")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.72),
                        verified: false,
                        requires_manual_question_import: manual_import,
                    }
                })
                .collect::<Vec<_>>();
            if table_row_mapping_inferred.get()
                && !review_warnings
                    .iter()
                    .any(|warning| warning == "TABLE_ROW_MAPPING_INFERRED")
            {
                review_warnings.push("TABLE_ROW_MAPPING_INFERRED".to_string());
            }
            let mut group_source_block_ids = block_ids.clone();
            for question in &questions {
                for block_id in &question.source_block_ids {
                    if !group_source_block_ids.iter().any(|existing| existing == block_id) {
                        group_source_block_ids.push(block_id.clone());
                    }
                }
            }
            let group_requires_manual_question_import = requires_manual_question_import
                || questions
                    .iter()
                    .any(|question| question.requires_manual_question_import);
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
                instruction: {
                    let mut instructions = vec![heading.to_string()];
                    if !instruction_text.trim().is_empty()
                        && collapse_whitespace(instruction_text) != collapse_whitespace(heading)
                    {
                        instructions.push(instruction_text.to_string());
                    }
                    instructions
                },
                questions,
                layout,
                review_warnings,
                classification_evidence,
                section_evidence,
                continuation_edges,
                allow_option_reuse,
                source_block_ids: group_source_block_ids,
                confidence: candidate
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.72),
                verified: false,
                is_umbrella_range: candidate
                    .get("isUmbrellaRange")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                requires_manual_question_import: group_requires_manual_question_import,
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
    use crate::{ImportJob, IssueCounts, JobStatus, SourceFile, WorkflowStep};
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

    fn two_column_option_table(rows: &[(&str, &str)]) -> Value {
        let cells = rows
            .iter()
            .enumerate()
            .flat_map(|(row, (label, option_text))| {
                [
                    json!({"row": row, "col": 0, "text": label}),
                    json!({"row": row, "col": 1, "text": option_text}),
                ]
            })
            .collect::<Vec<_>>();
        json!({
            "blockId": "option-table",
            "blockType": "table",
            "text": rows
                .iter()
                .map(|(label, option_text)| format!("{}\t{}", label, option_text))
                .collect::<Vec<_>>()
                .join("\n"),
            "table": {
                "rows": rows.len(),
                "cols": 2,
                "cells": cells
            }
        })
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
        assert_eq!(
            detect_dynamic_group_kind(
                "Questions 27-31 Complete the summary using the list of phrases, A-J, below. Write the correct letter, A-J, in boxes 27-31."
            ),
            "summary_completion"
        );
    }

    #[test]
    fn tea_style_roman_table_populates_all_heading_options() {
        let block = two_column_option_table(&[
            ("i.\u{200c}", "Tea reaches Europe"),
            ("ii\u{200b}", "An accidental discovery"),
            ("iii\u{feff}", "Changes in production"),
            ("iv", "A drink for every class"),
        ]);

        let options = dynamic_group_option_bank(&[block], "heading_matching");
        assert_eq!(
            options,
            vec![
                ("i".to_string(), "Tea reaches Europe".to_string()),
                ("ii".to_string(), "An accidental discovery".to_string()),
                ("iii".to_string(), "Changes in production".to_string()),
                ("iv".to_string(), "A drink for every class".to_string()),
            ]
        );

        let interaction =
            dynamic_interaction_with_option_texts(&json!({}), "heading_matching", &options);
        assert_eq!(
            interaction.get("options"),
            Some(&json!(["i", "ii", "iii", "iv"]))
        );
        assert_eq!(
            interaction
                .pointer("/optionTexts/iii")
                .and_then(Value::as_str),
            Some("Changes in production")
        );
    }

    #[test]
    fn heading_matching_uses_roman_bank_not_paragraph_letters() {
        let mut blocks = vec![json!({
            "blockId": "instruction",
            "blockType": "paragraph",
            "text": "Questions 1-8 Reading Passage 1 has eight paragraphs A-H. Choose the correct heading for each paragraph from the list of headings below. Write the correct number, i-x, in boxes 1-8. List of Headings"
        })];
        for (index, (label, text)) in [
            ("i", "Not enough tea to meet demand"),
            ("ii", "Religious objections"),
            ("iii", "In and sometimes out of fashion"),
            ("iv", "A connection between tea and religion"),
            ("v", "A luxury item"),
            ("vi", "News of tea reaches another continent"),
            ("vii", "Is tea a good or a bad thing?"),
            ("viii", "A chance discovery"),
            ("ix", "Tea-making as a ritual"),
            ("x", "Difficulties in importing tea"),
        ]
        .into_iter()
        .enumerate()
        {
            blocks.push(json!({
                "blockId": format!("heading-{index}"),
                "blockType": "paragraph",
                "text": format!("{label} {text}"),
            }));
        }

        let options = dynamic_group_option_bank(&blocks, "heading_matching");
        assert_eq!(
            options
                .iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>(),
            vec!["i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x"]
        );
        assert!(options.iter().all(|(_, text)| !text.trim().is_empty()));

        let classification = classify_dynamic_group(
            &blocks
                .iter()
                .map(dynamic_block_text)
                .collect::<Vec<_>>()
                .join(" "),
            &[],
        );
        assert_eq!(classification.kind, "heading_matching");
        assert_eq!(
            classification.interaction.options,
            vec!["i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x"]
        );
        let interaction = dynamic_interaction_with_option_texts(
            &json!({"classification": {"interaction": classification.interaction}}),
            "heading_matching",
            &options,
        );
        assert_eq!(
            interaction.get("options"),
            Some(&json!([
                "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x"
            ]))
        );
        assert_eq!(
            interaction
                .pointer("/optionTexts/x")
                .and_then(Value::as_str),
            Some("Difficulties in importing tea")
        );
    }

    #[test]
    fn heading_matching_block_markers_override_truncated_heading_end() {
        let blocks = [
            "27 Section A",
            "28 Section B",
            "29 Section C",
            "30 Section D",
            "31 Section E",
            "32 Section F",
            "to certain places",
            "sense-of-place research",
        ]
        .iter()
        .enumerate()
        .map(|(index, text)| {
            json!({
                "blockId": format!("b{:03}", index + 1),
                "blockType": "paragraph",
                "text": text,
            })
        })
        .collect::<Vec<_>>();

        assert_eq!(
            infer_dynamic_heading_matching_range_end_from_blocks(&blocks, 27, 31),
            32
        );

        let unrelated = vec![json!({
            "blockId": "b999",
            "blockType": "paragraph",
            "text": "32 A long passage sentence that is not a section assignment and must not extend the range.",
        })];
        assert_eq!(
            infer_dynamic_heading_matching_range_end_from_blocks(&unrelated, 27, 31),
            31
        );
    }

    #[test]
    fn two_column_letter_table_populates_complete_matching_option_bank() {
        let block = two_column_option_table(&[
            ("A", "China"),
            ("B.\u{200c}", "Japan"),
            ("C", "India"),
            ("D", "Sri Lanka"),
            ("E", "Turkey"),
            ("F", "Russia"),
            ("G", "Britain"),
        ]);

        let options = dynamic_group_option_bank(&[block], "matching");
        let interaction = dynamic_interaction_with_option_texts(&json!({}), "matching", &options);
        assert_eq!(
            interaction.get("options"),
            Some(&json!(["A", "B", "C", "D", "E", "F", "G"]))
        );
        assert_eq!(
            interaction
                .pointer("/optionTexts/B")
                .and_then(Value::as_str),
            Some("Japan")
        );
        assert_eq!(
            interaction
                .pointer("/optionTexts/G")
                .and_then(Value::as_str),
            Some("Britain")
        );
    }

    #[test]
    fn declared_a_j_bank_keeps_the_j_option_across_detection_and_filtering() {
        let labels = ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"];
        let rows = labels
            .iter()
            .map(|label| (*label, *label))
            .collect::<Vec<_>>();
        let blocks = vec![
            json!({
                "blockId": "instruction",
                "blockType": "paragraph",
                "text": "Complete the summary using the list of words, A-J, below."
            }),
            two_column_option_table(&rows),
        ];

        assert_eq!(
            dynamic_letter_options_for_text("Choose from A-J."),
            labels
                .iter()
                .map(|label| label.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            dynamic_declared_letter_bank_labels("Use the list of words A-J below."),
            labels
                .iter()
                .map(|label| label.to_string())
                .collect::<Vec<_>>()
        );
        let options = dynamic_group_option_bank(&blocks, "summary_completion");
        assert_eq!(
            options
                .iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>(),
            labels
        );
        assert_eq!(options.last().map(|(_, text)| text.as_str()), Some("J"));
    }

    #[test]
    fn explicit_letter_bank_turns_completion_interactions_into_selectable_matching() {
        let blocks = vec![
            json!({
                "blockId": "q36-40-instruction",
                "blockType": "paragraph",
                "text": "Complete the summary using the list of words, A-H, below."
            }),
            json!({
                "blockId": "q36-40-prompts",
                "blockType": "paragraph",
                "text": "The research frees our 36 _______ and may lead to 37 _______."
            }),
            json!({
                "blockId": "q36-40-options-1",
                "blockType": "paragraph",
                "text": "A natural evolution B creative thought C indigenous plants D trout"
            }),
            json!({
                "blockId": "q36-40-options-2",
                "blockType": "paragraph",
                "text": "E pollution"
            }),
            json!({
                "blockId": "q36-40-options-3",
                "blockType": "paragraph",
                "text": "F restoration"
            }),
            json!({
                "blockId": "q36-40-options-4",
                "blockType": "paragraph",
                "text": "G native fish H extinction"
            }),
        ];
        let candidate = json!({
            "classification": {
                "interaction": {
                    "type": "text",
                    "options": [],
                    "allowOptionReuse": false
                }
            }
        });

        for kind in [
            "summary_completion",
            "sentence_completion",
            "table_completion",
            "diagram_completion",
        ] {
            let options = dynamic_group_option_bank(&blocks, kind);
            assert_eq!(
                options
                    .iter()
                    .map(|(label, _)| label.as_str())
                    .collect::<Vec<_>>(),
                vec!["A", "B", "C", "D", "E", "F", "G", "H"],
                "{kind} should retain the complete A-H bank"
            );
            assert_eq!(
                options.last().map(|(_, text)| text.as_str()),
                Some("extinction")
            );

            let interaction = dynamic_interaction_with_option_texts(&candidate, kind, &options);
            assert_eq!(
                interaction.get("type").and_then(Value::as_str),
                Some("matching")
            );
            assert_eq!(
                interaction.get("options"),
                Some(&json!(["A", "B", "C", "D", "E", "F", "G", "H"]))
            );
            assert_eq!(
                interaction
                    .pointer("/optionTexts/G")
                    .and_then(Value::as_str),
                Some("native fish")
            );
        }
    }

    #[test]
    fn completion_without_an_explicit_letter_bank_remains_free_text() {
        let blocks = vec![
            json!({
                "blockId": "instruction",
                "blockType": "paragraph",
                "text": "Complete the summary below. Choose ONE WORD ONLY from the passage for each answer."
            }),
            json!({
                "blockId": "summary",
                "blockType": "paragraph",
                "text": "A short phrase begins this sentence. B is mentioned later, but neither is an option bank. 1 _______ 2 _______"
            }),
        ];
        let candidate = json!({
            "classification": {
                "interaction": {
                    "type": "text",
                    "options": [],
                    "allowOptionReuse": false
                }
            }
        });

        for kind in [
            "summary_completion",
            "sentence_completion",
            "table_completion",
            "diagram_completion",
        ] {
            let options = dynamic_group_option_bank(&blocks, kind);
            assert!(options.is_empty(), "{kind} inferred a false option bank");
            let interaction = dynamic_interaction_with_option_texts(&candidate, kind, &options);
            assert_eq!(
                interaction.get("type").and_then(Value::as_str),
                Some("text")
            );
            assert_eq!(interaction.get("options"), Some(&json!([])));
            assert!(interaction.get("optionTexts").is_none());
        }
    }

    #[test]
    fn answer_boxes_and_vitamin_b_prose_do_not_create_a_completion_option_bank() {
        let blocks = vec![
            json!({
                "blockId": "instruction",
                "blockType": "paragraph",
                "text": "Complete the sentences below. Write ONE WORD ONLY in the boxes 1 and 2 on your answer sheet."
            }),
            json!({
                "blockId": "ordinary-prose",
                "blockType": "paragraph",
                "text": "A vitamin B deficiency can affect the body's normal metabolism."
            }),
        ];

        assert!(!has_dynamic_completion_option_bank_cue(
            "Write ONE WORD ONLY in the boxes 1 and 2 on your answer sheet."
        ));
        assert!(has_dynamic_completion_option_bank_cue(
            "Choose the correct words from the box below."
        ));
        assert!(has_dynamic_completion_option_bank_cue(
            "Use the words in the box below."
        ));
        for kind in [
            "summary_completion",
            "sentence_completion",
            "table_completion",
            "diagram_completion",
        ] {
            assert!(
                dynamic_group_option_bank(&blocks, kind).is_empty(),
                "{kind} treated vitamin B prose as a letter bank"
            );
        }
    }

    #[test]
    fn inline_single_choice_block_splits_all_option_labels_before_leading_fallback() {
        let blocks = vec![json!({
            "blockId": "q1",
            "blockType": "paragraph",
            "text": "1 Where are bovids found? A Africa B Eurasia C the Americas D Oceania"
        })];

        let (prompt, options) = dynamic_question_prompt_and_options(
            &blocks,
            "",
            1,
            "Questions 1-1",
            1,
            "single_choice",
        );

        assert_eq!(prompt, "Where are bovids found?");
        assert_eq!(
            options,
            vec![
                ("A".to_string(), "Africa".to_string()),
                ("B".to_string(), "Eurasia".to_string()),
                ("C".to_string(), "the Americas".to_string()),
                ("D".to_string(), "Oceania".to_string()),
            ]
        );
    }

    #[test]
    fn bare_question_number_with_split_option_lines_recovers_prompt_and_abc() {
        // Customer PDF layout: number alone, stem alone, each option label alone.
        let blocks = vec![
            json!({
                "blockId": "n5",
                "blockType": "paragraph",
                "text": "5"
            }),
            json!({
                "blockId": "stem5",
                "blockType": "paragraph",
                "text": "Which extra service does the agency agree to provide?"
            }),
            json!({"blockId": "a5", "blockType": "paragraph", "text": "A"}),
            json!({
                "blockId": "a5t",
                "blockType": "paragraph",
                "text": "changing the bed linen"
            }),
            json!({"blockId": "b5", "blockType": "paragraph", "text": "B"}),
            json!({
                "blockId": "b5t",
                "blockType": "paragraph",
                "text": "washing the windows"
            }),
            json!({"blockId": "c5", "blockType": "paragraph", "text": "C"}),
            json!({
                "blockId": "c5t",
                "blockType": "paragraph",
                "text": "cleaning the fridge"
            }),
            json!({
                "blockId": "n6",
                "blockType": "paragraph",
                "text": "6"
            }),
            json!({
                "blockId": "stem6",
                "blockType": "paragraph",
                "text": "What does the agent say about the parking?"
            }),
        ];

        let (prompt, options) = dynamic_question_prompt_and_options(
            &blocks,
            "Questions 5-7 Choose the correct answer.",
            5,
            "Questions 5-7",
            7,
            "single_choice",
        );

        assert_eq!(
            prompt,
            "Which extra service does the agency agree to provide?"
        );
        assert_eq!(
            options,
            vec![
                ("A".to_string(), "changing the bed linen".to_string()),
                ("B".to_string(), "washing the windows".to_string()),
                ("C".to_string(), "cleaning the fridge".to_string()),
            ]
        );
    }

    #[test]
    fn sweet_trouble_style_b_to_d_tail_continues_a_single_choice_run() {
        let blocks = vec![
            json!({
                "blockId": "q6",
                "blockType": "paragraph",
                "text": "6 According to the writer, cane growers are expected to"
            }),
            json!({
                "blockId": "q6-a",
                "blockType": "paragraph",
                "text": "A expand their farms."
            }),
            json!({
                "blockId": "q6-bcd",
                "blockType": "paragraph",
                "text": "B sell their land. C find jobs elsewhere. D seek financial help."
            }),
        ];

        let (prompt, options) = dynamic_question_prompt_and_options(
            &blocks,
            "",
            6,
            "Questions 6-6",
            6,
            "single_choice",
        );

        assert_eq!(
            prompt,
            "According to the writer, cane growers are expected to"
        );
        assert_eq!(
            options
                .iter()
                .map(|item| item.0.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B", "C", "D"]
        );
        assert_eq!(options[3].1, "seek financial help.");
    }

    #[test]
    fn split_and_authoring_keep_flattened_b_to_d_option_tail() {
        let job = test_job();
        let texts = [
            ("passage", "READING PASSAGE 1"),
            ("heading", "Questions 1-2"),
            ("instruction", "Choose the correct letter, A, B, C or D."),
            (
                "write",
                "Write the correct letter in boxes 1-2 on your answer sheet.",
            ),
            ("q1", "1 What is the first answer?"),
            ("q1-options", "A alpha B beta C gamma D delta"),
            ("q2", "2 What is the second answer?"),
            ("q2-a", "A one"),
            ("q2-bcd", "B two C three D four"),
        ];
        let blocks = texts
            .iter()
            .enumerate()
            .map(|(index, (block_id, text))| {
                json!({
                    "blockId": block_id,
                    "blockType": if *block_id == "heading" { "header" } else { "paragraph" },
                    "text": text,
                    "pageIndex": 1,
                    "bbox": [54.0, 720.0 - index as f64 * 30.0, 520.0, 738.0 - index as f64 * 30.0]
                })
            })
            .collect::<Vec<_>>();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "blocks": blocks}],
            "assets": []
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert!(split["questionGroupCandidates"][0]["blockIds"]
            .as_array()
            .unwrap()
            .contains(&json!("q2-bcd")));
        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let q2 = &authoring["groups"][0]["questions"][1];
        assert_eq!(q2["prompt"], json!("What is the second answer?"));
        assert_eq!(q2["interaction"]["options"], json!(["A", "B", "C", "D"]));
        assert!(q2["sourceBlockIds"]
            .as_array()
            .unwrap()
            .contains(&json!("q2-bcd")));
    }

    #[test]
    fn matching_bank_closes_across_interleaved_next_heading_by_geometry() {
        let job = test_job();
        let blocks = vec![
            json!({"blockId":"passage","blockType":"header","text":"READING PASSAGE 2","pageIndex":4,"bbox":[54.0,780.0,220.0,792.0]}),
            json!({"blockId":"q20-heading","blockType":"header","text":"Questions 20-23","pageIndex":4,"bbox":[54.0,752.0,145.0,761.0]}),
            json!({"blockId":"q20-instruction","blockType":"paragraph","text":"Match each statement with the correct researcher, A, B or C.","pageIndex":4,"bbox":[54.0,692.0,375.0,701.0]}),
            json!({"blockId":"q20","blockType":"paragraph","text":"20 first claim","pageIndex":4,"bbox":[54.0,602.0,300.0,611.0]}),
            json!({"blockId":"q21","blockType":"paragraph","text":"21 second claim","pageIndex":4,"bbox":[54.0,572.0,300.0,581.0]}),
            json!({"blockId":"q22","blockType":"paragraph","text":"22 third claim","pageIndex":4,"bbox":[54.0,542.0,300.0,551.0]}),
            json!({"blockId":"q23","blockType":"paragraph","text":"23 fourth claim","pageIndex":4,"bbox":[54.0,512.0,300.0,521.0]}),
            json!({"blockId":"bank-heading","blockType":"paragraph","text":"List of Researchers","pageIndex":4,"bbox":[244.0,465.0,350.0,474.0]}),
            json!({"blockId":"bank-ab","blockType":"paragraph","text":"A Simon Peers B Nicholas Godley","pageIndex":4,"bbox":[240.0,425.0,349.0,450.0]}),
            // Parser order is wrong here: the next heading is emitted before
            // the visually higher C row.
            json!({"blockId":"q24-heading","blockType":"header","text":"Questions 24-26","pageIndex":4,"bbox":[54.0,348.0,145.0,357.0]}),
            json!({"blockId":"bank-c","blockType":"paragraph","text":"C Todd B","pageIndex":4,"bbox":[240.0,410.0,292.0,419.0]}),
            json!({"blockId":"bank-c-tail","blockType":"paragraph","text":"lackledge","pageIndex":4,"bbox":[299.0,410.0,351.0,419.0]}),
            json!({"blockId":"q24-instruction","blockType":"paragraph","text":"Complete the summary below. Choose ONE WORD ONLY.","pageIndex":4,"bbox":[54.0,318.0,390.0,327.0]}),
            json!({"blockId":"q24","blockType":"paragraph","text":"24 ________","pageIndex":4,"bbox":[54.0,288.0,200.0,297.0]}),
            json!({"blockId":"q25","blockType":"paragraph","text":"25 ________","pageIndex":4,"bbox":[54.0,258.0,200.0,267.0]}),
            json!({"blockId":"q26","blockType":"paragraph","text":"26 ________","pageIndex":4,"bbox":[54.0,228.0,200.0,237.0]}),
        ];
        let doc = json!({"schemaVersion":"DocumentIRV1","pages":[{"pageIndex":4,"blocks":blocks}],"assets":[]});

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let first_ids = split["questionGroupCandidates"][0]["blockIds"]
            .as_array()
            .unwrap();
        assert!(first_ids.contains(&json!("bank-c")));
        assert!(first_ids.contains(&json!("bank-c-tail")));
        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let first = &authoring["groups"][0];
        for question in first["questions"].as_array().unwrap() {
            assert_eq!(question["interaction"]["options"], json!(["A", "B", "C"]));
            assert_eq!(
                question["interaction"]["optionTexts"]["C"],
                json!("Todd Blackledge")
            );
        }
    }

    #[test]
    fn matching_bank_recovers_cross_column_final_label_after_next_heading() {
        let job = test_job();
        let blocks = vec![
            json!({"blockId":"q20-heading","blockType":"header","text":"Questions 20-23","pageIndex":4,"bbox":[36.0,789.0,120.0,798.0]}),
            json!({"blockId":"q20-instruction","blockType":"paragraph","text":"Match each statement with the correct researcher, A, B, C or D.","pageIndex":4,"bbox":[36.0,730.0,372.0,739.0]}),
            json!({"blockId":"q20","blockType":"paragraph","text":"20 first statement","pageIndex":4,"bbox":[36.0,642.0,300.0,651.0]}),
            json!({"blockId":"q21","blockType":"paragraph","text":"21 second statement","pageIndex":4,"bbox":[36.0,612.0,300.0,621.0]}),
            json!({"blockId":"q22","blockType":"paragraph","text":"22 third statement","pageIndex":4,"bbox":[36.0,582.0,300.0,591.0]}),
            json!({"blockId":"q23","blockType":"paragraph","text":"23 fourth statement","pageIndex":4,"bbox":[36.0,552.0,300.0,561.0]}),
            json!({"blockId":"bank-heading","blockType":"paragraph","text":"List of Researchers","pageIndex":4,"bbox":[231.0,495.0,336.0,504.0]}),
            json!({"blockId":"bank-ab","blockType":"paragraph","text":"A Dieter Hochuli B John Martin","pageIndex":4,"bbox":[214.0,456.0,329.0,480.0]}),
            // The C row is emitted in the left column, so the final D row is
            // more than 120pt away in x even though both belong to one bank.
            json!({"blockId":"bank-c","blockType":"paragraph","text":"C Richard Major","pageIndex":4,"bbox":[36.0,440.0,327.0,449.0]}),
            json!({"blockId":"q24-heading","blockType":"header","text":"Questions 24-26","pageIndex":4,"bbox":[36.0,358.0,120.0,367.0]}),
            json!({"blockId":"bank-d","blockType":"paragraph","text":"D Catherin","pageIndex":4,"bbox":[214.0,425.0,295.0,434.0]}),
            json!({"blockId":"bank-d-tail","blockType":"paragraph","text":"e Price","pageIndex":4,"bbox":[302.0,425.0,333.0,434.0]}),
            json!({"blockId":"q24-instruction","blockType":"paragraph","text":"Complete the summary below. Choose ONE WORD ONLY.","pageIndex":4,"bbox":[36.0,328.0,390.0,337.0]}),
            json!({"blockId":"q24","blockType":"paragraph","text":"24 ________","pageIndex":4,"bbox":[36.0,298.0,200.0,307.0]}),
            json!({"blockId":"q25","blockType":"paragraph","text":"25 ________","pageIndex":4,"bbox":[36.0,268.0,200.0,277.0]}),
            json!({"blockId":"q26","blockType":"paragraph","text":"26 ________","pageIndex":4,"bbox":[36.0,238.0,200.0,247.0]}),
        ];
        let doc = json!({"schemaVersion":"DocumentIRV1","pages":[{"pageIndex":4,"blocks":blocks}],"assets":[]});

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let first_ids = split["questionGroupCandidates"][0]["blockIds"]
            .as_array()
            .unwrap();
        assert!(first_ids.contains(&json!("bank-d")));
        assert!(first_ids.contains(&json!("bank-d-tail")));
        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let first = &authoring["groups"][0];
        for question in first["questions"].as_array().unwrap() {
            assert_eq!(
                question["interaction"]["options"],
                json!(["A", "B", "C", "D"])
            );
            assert_eq!(
                question["interaction"]["optionTexts"]["D"],
                json!("Catherine Price")
            );
        }
    }

    #[test]
    fn wrapped_choice_prompt_keeps_option_run_beyond_three_block_lookback() {
        let blocks = vec![
            json!({"blockId":"q34","text":"34 Which aspect of playing an instrument was included"}),
            json!({"blockId":"q34-wrap-1","text":"in the follow-up study"}),
            json!({"blockId":"q34-wrap-2","text":"but not in the first study?"}),
            json!({"blockId":"q34-spacer","text":"Select one answer."}),
            json!({"blockId":"q34-abc","text":"A duration B starting age C years since playing"}),
            json!({"blockId":"q34-d","text":"D childhood practice"}),
        ];

        assert_eq!(dynamic_choice_option_run_bounds(&blocks, 4), Some((4, 6)));
        assert_eq!(dynamic_choice_option_run_bounds(&blocks, 5), Some((4, 6)));
    }

    #[test]
    fn rotated_text_matrix_recovers_number_without_visual_space() {
        assert_eq!(
            dynamic_leading_question_number("14High-level workers react positively to stress"),
            Some(14)
        );
        assert_eq!(
            strip_dynamic_leading_question_marker(
                "14High-level workers react positively to stress",
                14
            ),
            "High-level workers react positively to stress"
        );
        assert_eq!(
            dynamic_leading_question_number("14th-century working practices"),
            None
        );
        assert_eq!(
            dynamic_leading_question_number("2020Research findings were published"),
            None
        );
    }

    #[test]
    fn bilingual_style_b_to_d_tail_preserves_all_choice_labels() {
        let blocks = vec![
            json!({
                "blockId": "q29",
                "blockType": "paragraph",
                "text": "29 The mothers who took part in the research"
            }),
            json!({
                "blockId": "q29-a",
                "blockType": "paragraph",
                "text": "A compensated for greater exposure to English."
            }),
            json!({
                "blockId": "q29-bcd",
                "blockType": "paragraph",
                "text": "B took language learning more seriously. C spent more time with their children. D preferred their partners not to use Japanese."
            }),
        ];

        let (_, options) = dynamic_question_prompt_and_options(
            &blocks,
            "",
            29,
            "Questions 29-29",
            29,
            "single_choice",
        );

        assert_eq!(
            options
                .iter()
                .map(|item| item.0.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B", "C", "D"]
        );
        assert_eq!(options[2].1, "spent more time with their children.");
    }

    #[test]
    fn animal_connection_style_wrapped_options_are_not_mistaken_for_passage_sections() {
        let blocks = [
            "37 What is the writer's main argument?",
            "A Prehistoric art allows us to date migration patterns.",
            "B Early humans valued knowledge about animal behaviour and",
            "appearance.",
            "C There were lifestyle differences between people in different",
            "regions.",
            "D There was little spoken interaction between groups.",
        ]
        .iter()
        .enumerate()
        .map(|(index, text)| {
            json!({
                "blockId": format!("animal-{index}"),
                "blockType": "paragraph",
                "text": text
            })
        })
        .collect::<Vec<_>>();

        assert_eq!(dynamic_choice_option_run_bounds(&blocks, 1), Some((1, 7)));
        assert_eq!(dynamic_choice_option_run_bounds(&blocks, 3), Some((1, 7)));
        assert_eq!(dynamic_choice_option_run_bounds(&blocks, 6), Some((1, 7)));
    }

    #[test]
    fn inline_choice_parser_does_not_promote_body_letters_without_a_run() {
        for text in [
            "A Type B vitamin deficiency",
            "A Treatment for vitamin B",
            "A Details from Section B",
        ] {
            assert!(
                dynamic_inline_choice_parts(text).is_none(),
                "ordinary body letter was treated as an option boundary: {text}"
            );
        }

        let (_, options) =
            dynamic_inline_choice_parts("A Africa B Eurasia C the Americas D Oceania")
                .expect("a contiguous A-D run should remain recoverable");
        assert_eq!(options.len(), 4);
    }

    #[test]
    fn prompt_stops_at_next_number_marker_inside_a_continuation_block() {
        let blocks = [
            "23 ________ and give orders to robots working on the surface of the planet. This would",
            "increase the speed of 24 ________ with the robots.",
            "In such ways, robots might be used in commercial enterprises or",
            "25 ________. However, the final aim may be 26 ________.",
        ]
        .iter()
        .enumerate()
        .map(|(index, text)| json!({"blockId": format!("q{index}"), "text": text}))
        .collect::<Vec<_>>();
        let group_text = blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");

        let (prompt, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            23,
            "Questions 22-26",
            26,
            "summary_completion",
        );

        assert_eq!(
            prompt,
            "________ and give orders to robots working on the surface of the planet. This would increase the speed of"
        );
        assert!(!prompt.contains("24"));
    }

    #[test]
    fn matching_prompt_stops_before_standalone_list_and_option_bank() {
        let blocks = [
            "31 Multitasking can lead to a medical problem.",
            "List of People",
            "A John Ridley Stroop",
            "B Ernst Poppel",
            "C David E. Meyer",
            "D Edward Hallowell",
        ]
        .iter()
        .enumerate()
        .map(|(index, text)| json!({"blockId": format!("q{index}"), "text": text}))
        .collect::<Vec<_>>();
        let group_text = blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");

        let (prompt, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            31,
            "Questions 27-31",
            31,
            "matching",
        );

        assert_eq!(prompt, "Multitasking can lead to a medical problem.");
        assert!(!prompt.contains("List of People"));
        assert!(!prompt.contains("John Ridley"));
    }

    #[test]
    fn matching_prompt_stops_before_rotation_spaced_bank_heading() {
        let blocks = [
            "18 Workers commonly expect their workloads to lessen over time",
            "List of P eople",
            "A Neil Plumridge",
            "B Gal Zauberman",
            "C Jan Elsner",
        ]
        .iter()
        .enumerate()
        .map(|(index, text)| json!({"blockId": format!("q{index}"), "text": text}))
        .collect::<Vec<_>>();
        let group_text = blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");

        let (prompt, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            18,
            "Questions 14-18",
            18,
            "matching",
        );

        assert_eq!(
            prompt,
            "Workers commonly expect their workloads to lessen over time"
        );
    }

    #[test]
    fn explicit_bank_heading_discards_option_like_instruction_fragments() {
        let blocks = [
            "Questions 21-26",
            "Match each statement with the correct person, A,",
            "B or C.",
            "21 Too little evidence exists to support the theory.",
            "List of People",
            "A John Alroy",
            "B Ross D. E. MacPhee",
            "C Russell W. Graham",
        ]
        .iter()
        .enumerate()
        .map(|(index, text)| json!({"blockId": format!("b{index}"), "text": text}))
        .collect::<Vec<_>>();

        assert_eq!(
            dynamic_group_option_bank(&blocks, "matching"),
            vec![
                ("A".to_string(), "John Alroy".to_string()),
                ("B".to_string(), "Ross D. E. MacPhee".to_string()),
                ("C".to_string(), "Russell W. Graham".to_string()),
            ]
        );
    }

    #[test]
    fn numeric_percentage_inside_choice_prompt_is_not_a_question_boundary() {
        let blocks = [
            json!({"blockId":"q30","text":"30 What point is the writer making when he says that"}),
            json!({"blockId":"q30-percent","text":"16 % of the sample"}),
            json!({"blockId":"q30-wrap","text":"group did not know where the museum was?"}),
            json!({"blockId":"q30-options","text":"A first B second C third D fourth"}),
            json!({"blockId":"q31","text":"31 What happened next?"}),
        ];

        let group_text = blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");
        let (prompt, options) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            30,
            "Questions 27-31",
            31,
            "single_choice",
        );

        assert_eq!(
            prompt,
            "What point is the writer making when he says that 16 % of the sample group did not know where the museum was?"
        );
        assert_eq!(
            options,
            vec![
                ("A".to_string(), "first".to_string()),
                ("B".to_string(), "second".to_string()),
                ("C".to_string(), "third".to_string()),
                ("D".to_string(), "fourth".to_string()),
            ]
        );
    }

    #[test]
    fn list_completion_prompts_stop_at_inline_next_numbers_and_final_letter_bank() {
        let blocks = [
            "Complete the summary using the list of words, A-I, below.",
            "38 ______, which had previously been applied in other scientific research. The importance of",
            "CGI has led to the 39 ______ of Marschner's model by special-effects studios.",
            "Marschner's model has led to the 40 ______ of cinematography.",
            "A light",
            "D use",
            "B transparency",
            "E astrophysics",
            "C age",
            "F mathematics",
            "G improvement H colour I translucency",
        ]
        .iter()
        .enumerate()
        .map(|(index, text)| json!({"blockId": format!("q{index}"), "text": text}))
        .collect::<Vec<_>>();
        let group_text = blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");

        let (prompt_38, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            38,
            "Questions 37-40",
            40,
            "summary_completion",
        );
        let (prompt_40, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            40,
            "Questions 37-40",
            40,
            "summary_completion",
        );

        assert!(prompt_38.ends_with("The importance of CGI has led to the"));
        assert!(!prompt_38.contains("39"));
        assert_eq!(prompt_40, "______ of cinematography.");
        assert!(!prompt_40.contains("A light"));
    }

    #[test]
    fn note_completion_slots_close_on_numbered_blanks_not_timeline_numbers() {
        let blocks = [
            "9 ______ and the local way of life made her dissatisfied",
            "• 1908 – returned to London",
            "• 1911–1919 – the work improved by 10 percent",
            "– 10 ______ prevented the writers from staying together",
            "in Paris – spent time with distinguished",
            "11 ______",
            "– from 1916, illness restricted her travel",
            "13 ______ published after her death",
        ]
        .iter()
        .enumerate()
        .map(|(index, text)| json!({"blockId": format!("n{index}"), "text": text}))
        .collect::<Vec<_>>();
        let group_text = blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");

        let (prompt_9, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            9,
            "Questions 9-13",
            13,
            "sentence_completion",
        );
        assert!(prompt_9.contains("1908"));
        assert!(prompt_9.contains("1911–1919"));
        assert!(prompt_9.contains("10 percent"));
        assert!(!prompt_9.contains("10 ______"));
        assert!(!prompt_9.contains("11 ______"));

        let (prompt_10, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            10,
            "Questions 9-13",
            13,
            "sentence_completion",
        );
        assert_eq!(
            prompt_10,
            "______ prevented the writers from staying together in Paris – spent time with distinguished"
        );
        assert!(!prompt_10.contains("11 ______"));

        let (prompt_13, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            13,
            "Questions 9-13",
            13,
            "sentence_completion",
        );
        assert_eq!(prompt_13, "______ published after her death");
    }

    #[test]
    fn non_linear_completion_recovery_never_borrows_a_foreign_slot_block() {
        let group_blocks = [
            json!({"blockId":"q11","text":"The rock eventually 11 _______. Once extracted, the limestone"}),
            json!({"blockId":"q13","text":"or, through a 13 _______ process,"}),
        ];
        let group_text = group_blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");
        let (prompt_11, _) = dynamic_question_prompt_and_options(
            &group_blocks,
            &group_text,
            11,
            "Questions 8-13",
            13,
            "diagram_completion",
        );
        assert_eq!(
            prompt_11,
            "_______. Once extracted, the limestone or, through a"
        );
        assert!(!prompt_11.contains("13 ______"));

        let recovery_blocks = [
            json!({"blockId":"q13","text":"or, through a 13 _______ process,"}),
            json!({"blockId":"q12","text":"is made into 12 _______."}),
            json!({"blockId":"tail","text":"can be used to make quicklime."}),
        ];
        let (prompt_12, source_ids) =
            dynamic_gap_sentence_prompt_from_blocks(&recovery_blocks, 12).expect("q12 prompt");
        assert_eq!(
            prompt_12,
            "is made into _______. can be used to make quicklime."
        );
        assert!(!prompt_12.contains("13 ______"));
        assert_eq!(source_ids, vec!["q12".to_string(), "tail".to_string()]);
    }

    #[test]
    fn local_completion_recovery_splits_multiple_slots_in_one_geometry_block() {
        let blocks = [
            json!({
                "blockId":"flow-row",
                "text":"The 8 ______ drill rows of 9 ______."
            }),
            json!({
                "blockId":"other-row",
                "text":"10 ______ are used."
            }),
        ];

        let (q8, q8_source) =
            dynamic_local_completion_prompt_for_number(&blocks, 8, 8, 10).expect("q8");
        let (q9, q9_source) =
            dynamic_local_completion_prompt_for_number(&blocks, 9, 8, 10).expect("q9");
        assert_eq!(q8, "The ______ drill rows of");
        assert_eq!(q9, "drill rows of ______.");
        assert_eq!(q8_source, "flow-row");
        assert_eq!(q9_source, "flow-row");
        assert!(dynamic_completion_foreign_slots(&q8, 8, 8, 10).is_empty());
        assert!(dynamic_completion_foreign_slots(&q9, 9, 8, 10).is_empty());
        assert!(dynamic_local_completion_prompt_for_number(&blocks, 11, 8, 11).is_none());
    }

    #[test]
    fn note_rows_keep_section_context_and_close_before_neighbouring_bullets() {
        let block = |id: &str, text: &str, left: f64, top: f64, right: f64| {
            json!({
                "blockId": id,
                "text": text,
                "pageIndex": 4,
                "bbox": [left, top - 8.4, right, top],
                "_epic8LayoutSection": 0,
                "_epic8SectionColumns": 1,
                "_epic8ColumnIndex": 0
            })
        };
        let blocks = vec![
            block("title", "A landmark building", 227.0, 613.0, 360.0),
            block("cost-heading", "Final cost", 61.0, 590.0, 114.0),
            block("q8", "• 8 $ ____________", 82.0, 567.0, 184.0),
            block("construction", "Construction", 61.0, 536.0, 129.0),
            block(
                "construction-row-1",
                "• A large platform acting as a base for the building",
                82.0,
                512.0,
                341.0,
            ),
            block(
                "construction-row-2",
                "• Concrete panels used to make shells",
                82.0,
                489.0,
                427.0,
            ),
            block(
                "q9",
                "• Over a million tiles from 9 ____________",
                82.0,
                465.0,
                303.0,
            ),
            block(
                "q10",
                "• 10 ____________ from Australia covering the outside walls",
                82.0,
                442.0,
                399.0,
            ),
            block("use", "Use", 61.0, 411.0, 77.0),
            block(
                "q11",
                "• 11 ____________ performing-arts companies have their home base at the Opera",
                83.0,
                388.0,
                516.0,
            ),
            block("q11-wrap", "House", 107.0, 372.0, 136.0),
            block("outside", "Outside", 61.0, 317.0, 100.0),
            block(
                "q12",
                "• A large 12 ____________ at the foot of a wide staircase",
                82.0,
                294.0,
                381.0,
            ),
            block("alterations", "Alterations", 61.0, 263.0, 118.0),
            block(
                "plain-alteration",
                "• A colonnade was added in 2006",
                82.0,
                239.0,
                255.0,
            ),
            block(
                "q13",
                "• Openings made the 13 ____________ visible from foyers",
                82.0,
                216.0,
                389.0,
            ),
        ];

        let expected = [
            (8, "Final cost: • $ ____________"),
            (
                9,
                "Construction: • Over a million tiles from ____________",
            ),
            (
                10,
                "Construction: • ____________ from Australia covering the outside walls",
            ),
            (
                11,
                "Use: • ____________ performing-arts companies have their home base at the Opera House",
            ),
            (
                12,
                "Outside: • A large ____________ at the foot of a wide staircase",
            ),
            (
                13,
                "Alterations: • Openings made the ____________ visible from foyers",
            ),
        ];
        for (number, expected_prompt) in expected {
            let (prompt, source_ids) =
                dynamic_note_row_prompt_for_number(&blocks, number, 8, 13).expect("note row");
            assert_eq!(prompt, expected_prompt, "Q{number}");
            assert!(
                dynamic_completion_foreign_slots(&prompt, number, 8, 13).is_empty(),
                "Q{number} must not own another response slot"
            );
            assert!(source_ids.iter().any(|id| id == &format!("q{number}")));
        }
    }

    #[test]
    fn isolated_bulleted_numeric_prose_is_not_promoted_to_a_note_tree() {
        let blocks = [json!({
            "blockId": "only-row",
            "text": "• In 8 regions the survey recorded ______ responses"
        })];
        assert!(dynamic_note_row_prompt_for_number(&blocks, 8, 8, 8).is_none());
    }

    #[test]
    fn sentence_completion_stops_at_next_display_number_before_its_later_gap() {
        let blocks = [
            json!({"blockId":"q6-a","text":"6 In France, people changed their opinion because the King put a"}),
            json!({"blockId":"q6-b","text":"potato ______ in his buttonhole."}),
            json!({"blockId":"q7-a","text":"7 Frederick recognised the potential, but had to handle the ______ from ordinary"}),
            json!({"blockId":"q7-b","text":"people."}),
            json!({"blockId":"q8","text":"8 The King adopted ______ psychology."}),
        ];
        let group_text = blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");

        let (q6, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            6,
            "Questions 6-8",
            8,
            "sentence_completion",
        );
        let (q7, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            7,
            "Questions 6-8",
            8,
            "sentence_completion",
        );
        assert_eq!(
            q6,
            "In France, people changed their opinion because the King put a potato ______ in his buttonhole."
        );
        assert_eq!(
            q7,
            "Frederick recognised the potential, but had to handle the ______ from ordinary people."
        );
        assert!(!q6.contains("7 Frederick"));
        assert!(!q7.contains("8 The King"));
    }

    #[test]
    fn quantities_years_and_percentages_are_not_display_question_boundaries() {
        assert_eq!(
            find_dynamic_completion_display_question_boundary(
                "The trial continued for 7 years and produced a ______ improvement.",
                7,
                0,
            ),
            None
        );
        assert_eq!(
            find_dynamic_completion_display_question_boundary(
                "The first phase ended. 7 participants reported a ______ response.",
                7,
                0,
            ),
            None
        );
        assert_eq!(
            find_dynamic_completion_display_question_boundary(
                "The result rose by 7 percent before reaching ______.",
                7,
                0,
            ),
            None
        );
    }

    #[test]
    fn trailing_next_number_joins_a_blank_in_the_adjacent_geometry_block() {
        let current = layout_block(
            "q25",
            "25 ______ may be less effective than simply reviewing your 26",
            [306.6, 724.0, 547.4, 728.0],
            0,
            2,
            1,
        );
        let next = layout_block("q26", "________.", [306.6, 713.0, 344.4, 717.0], 0, 2, 1);
        assert_eq!(
            find_dynamic_cross_block_completion_number_boundary(
                &current,
                &dynamic_block_text(&current),
                &next,
                26,
            ),
            dynamic_block_text(&current).rfind("26")
        );

        let other_column = layout_block(
            "other-column",
            "________.",
            [54.0, 713.0, 100.0, 717.0],
            0,
            2,
            0,
        );
        assert_eq!(
            find_dynamic_cross_block_completion_number_boundary(
                &current,
                &dynamic_block_text(&current),
                &other_column,
                26,
            ),
            None
        );
        let ordinary_next = layout_block(
            "ordinary",
            "continues with ordinary prose.",
            [306.6, 713.0, 480.0, 717.0],
            0,
            2,
            1,
        );
        assert_eq!(
            find_dynamic_cross_block_completion_number_boundary(
                &current,
                &dynamic_block_text(&current),
                &ordinary_next,
                26,
            ),
            None
        );
        let neil = layout_block(
            "q20",
            "20 Which method was suggested by Neil",
            [306.6, 724.0, 520.0, 728.0],
            0,
            2,
            1,
        );
        assert_eq!(
            find_dynamic_cross_block_completion_number_boundary(
                &neil,
                &dynamic_block_text(&neil),
                &next,
                21,
            ),
            None
        );
    }

    #[test]
    fn inline_completion_recovers_display_number_then_later_gap_in_one_block() {
        let blocks = [json!({
            "blockId":"q4-5",
            "text":"Questions 4-5 Complete the notes below. 4 The passage is about tidal _____. 5 The records are called _____."
        })];
        let group_text = dynamic_block_text(&blocks[0]);
        let (q4, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            4,
            "Questions 4-5",
            5,
            "sentence_completion",
        );
        let (q5, _) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            5,
            "Questions 4-5",
            5,
            "sentence_completion",
        );
        assert_eq!(q4, "The passage is about tidal _____.");
        assert_eq!(q5, "The records are called _____.");
        assert!(dynamic_completion_foreign_slots(&q4, 4, 4, 5).is_empty());
    }

    #[test]
    fn final_prompt_stops_before_disclaimer_key_and_answers_headings() {
        for terminal in ["Disclaimer", "Key", "Answers: 26 Rotterdam"] {
            let blocks = vec![
                json!({"blockId": "q26", "text": "26 A proposal for part of the city could be applied"}),
                json!({"blockId": "q26-tail", "text": "on a large scale."}),
                json!({"blockId": "terminal", "text": terminal}),
                json!({"blockId": "tail", "text": "This material is not part of the question."}),
            ];
            let group_text = blocks
                .iter()
                .map(dynamic_block_text)
                .collect::<Vec<_>>()
                .join("\n");
            let (prompt, _) = dynamic_question_prompt_and_options(
                &blocks,
                &group_text,
                26,
                "Questions 22-26",
                26,
                "sentence_completion",
            );
            assert_eq!(
                prompt, "A proposal for part of the city could be applied on a large scale.",
                "terminal={terminal}"
            );
        }
    }

    #[test]
    fn option_and_terminal_boundaries_do_not_cut_ordinary_article_words() {
        let article = "A recent study discusses B vitamins and C levels in ordinary prose.";
        assert_eq!(
            find_dynamic_prompt_boundary_with_context(
                article,
                0,
                2,
                "summary_completion",
                "Complete the sentence with one word only.",
            ),
            article.len()
        );
        assert!(!is_dynamic_prompt_terminal_heading(
            "Key factors determine the final outcome."
        ));
        let answers_verb = "Which paragraph answers the following question about migration?";
        assert_eq!(
            find_dynamic_final_prompt_boundary(answers_verb, 0),
            answers_verb.len()
        );
    }

    #[test]
    fn independent_table_label_cells_protect_letters_inside_option_text() {
        let table = two_column_option_table(&[
            ("A.\u{200c}", "Type B vitamin deficiency"),
            ("B.\u{200c}", "vitamin B"),
            ("C.\u{200c}", "Section B"),
            ("D.\u{200c}", "none of these"),
        ]);
        let blocks = vec![
            json!({
                "blockId": "q1",
                "blockType": "paragraph",
                "text": "1 Where are bovids found?"
            }),
            table,
        ];

        let (prompt, options) = dynamic_question_prompt_and_options(
            &blocks,
            "",
            1,
            "Questions 1-1",
            1,
            "single_choice",
        );

        assert_eq!(prompt, "Where are bovids found?");
        assert_eq!(
            options,
            vec![
                ("A".to_string(), "Type B vitamin deficiency".to_string()),
                ("B".to_string(), "vitamin B".to_string()),
                ("C".to_string(), "Section B".to_string()),
                ("D".to_string(), "none of these".to_string()),
            ]
        );
    }

    #[test]
    fn single_cell_table_row_can_split_a_complete_inline_choice_run() {
        let table = json!({
            "blockId": "inline-option-table",
            "blockType": "table",
            "table": {
                "rows": 1,
                "cols": 1,
                "cells": [{
                    "row": 0,
                    "col": 0,
                    "text": "A Africa B Eurasia C the Americas D Oceania"
                }]
            }
        });
        assert_eq!(
            dynamic_table_option_rows(&table),
            vec![
                ("A".to_string(), "Africa".to_string()),
                ("B".to_string(), "Eurasia".to_string()),
                ("C".to_string(), "the Americas".to_string()),
                ("D".to_string(), "Oceania".to_string()),
            ]
        );
    }

    #[test]
    fn table_level_choice_run_recovers_labels_embedded_after_dedicated_cells() {
        let tuatara = two_column_option_table(&[
            (
                "A",
                "natural evolution B creative thought C indigenous plants D trout",
            ),
            ("E", "pollution"),
            ("F", "restoration"),
            ("G", "native fish H extinction"),
        ]);
        let expected_tuatara = vec![
            ("A".to_string(), "natural evolution".to_string()),
            ("B".to_string(), "creative thought".to_string()),
            ("C".to_string(), "indigenous plants".to_string()),
            ("D".to_string(), "trout".to_string()),
            ("E".to_string(), "pollution".to_string()),
            ("F".to_string(), "restoration".to_string()),
            ("G".to_string(), "native fish".to_string()),
            ("H".to_string(), "extinction".to_string()),
        ];
        assert_eq!(dynamic_table_option_rows(&tuatara), expected_tuatara);
        assert_eq!(
            dynamic_group_option_bank(
                &[
                    json!({
                        "blockId": "instruction",
                        "blockType": "paragraph",
                        "text": "Complete the summary using the list of words, A-H, below."
                    }),
                    tuatara,
                ],
                "summary_completion",
            ),
            vec![
                ("A".to_string(), "natural evolution".to_string()),
                ("B".to_string(), "creative thought".to_string()),
                ("C".to_string(), "indigenous plants".to_string()),
                ("D".to_string(), "trout".to_string()),
                ("E".to_string(), "pollution".to_string()),
                ("F".to_string(), "restoration".to_string()),
                ("G".to_string(), "native fish".to_string()),
                ("H".to_string(), "extinction".to_string()),
            ]
        );

        let bovids = two_column_option_table(&[
            ("A", "Their horns are shed B They have upper incisors"),
            (
                "C",
                "They store food in the body D Their hooves are undivided",
            ),
        ]);
        let expected_bovids = vec![
            ("A".to_string(), "Their horns are shed".to_string()),
            ("B".to_string(), "They have upper incisors".to_string()),
            ("C".to_string(), "They store food in the body".to_string()),
            ("D".to_string(), "Their hooves are undivided".to_string()),
        ];
        assert_eq!(dynamic_table_option_rows(&bovids), expected_bovids);
        let question_blocks = vec![
            json!({
                "blockId": "q3",
                "blockType": "paragraph",
                "text": "3 Which of the following features do all bovids have in common?"
            }),
            bovids,
        ];
        let (_, question_options) = dynamic_question_prompt_and_options(
            &question_blocks,
            "Questions 1-3 Choose the correct letter, A, B, C or D.",
            3,
            "Questions 1-3",
            3,
            "single_choice",
        );
        assert_eq!(question_options, expected_bovids);
    }

    #[test]
    fn explicit_range_keeps_long_choice_options_inside_question_group() {
        let job = test_job();
        let texts = [
            "READING PASSAGE 1",
            "Archive access",
            "The archive moved into a larger building so that more visitors could inspect its maps and records.",
            "Questions 1-2 Write the correct letter, A, B, C or D, in boxes 1 and 2.",
            "1 Why did the archive move?",
            "A The managers wanted a smaller collection with fewer public visitors and shorter opening hours.",
            "B The existing building did not provide enough room for the collection or for researchers.",
            "C The city required every museum to move outside the central business district immediately.",
            "D The archive planned to replace all paper records with a completely digital collection.",
            "2 What can visitors inspect?",
            "A Maps and related records are available to visitors in the new public research room.",
            "B Only photographs can be requested because maps remain permanently unavailable.",
            "C Tickets from previous exhibitions are the only records shown to members of the public.",
            "D Furniture can be inspected, but all written material is stored at another location.",
            "Answers 1 B 2 A",
        ];
        let blocks = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                json!({
                    "blockId": format!("b{:03}", index + 1),
                    "blockType": "paragraph",
                    "text": text,
                    "html": format!("<p>{}</p>", text),
                    "pageIndex": 1,
                    "confidence": 0.99
                })
            })
            .collect::<Vec<_>>();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "width": 595, "height": 842, "blocks": blocks}],
            "assets": []
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let group_blocks = split
            .pointer("/questionGroupCandidates/0/blockIds")
            .and_then(Value::as_array)
            .unwrap();
        assert!(group_blocks
            .iter()
            .any(|value| value.as_str() == Some("b010")));
        assert!(group_blocks
            .iter()
            .any(|value| value.as_str() == Some("b014")));

        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            ir.pointer("/groups/0/kind").and_then(Value::as_str),
            Some("single_choice")
        );
        assert_eq!(
            ir.pointer("/groups/0/questions/1/prompt")
                .and_then(Value::as_str),
            Some("What can visitors inspect?")
        );
        assert_eq!(
            ir.pointer("/groups/0/questions/1/interaction/optionTexts/A")
                .and_then(Value::as_str),
            Some("Maps and related records are available to visitors in the new public research room.")
        );
    }

    #[test]
    fn split_umbrella_context_does_not_become_fake_single_choice_group() {
        let job = test_job();
        let texts = [
            "READING PASSAGE 2",
            "You should spend about 20 minutes on Questions 14-26, which are based on Reading",
            "Passage 2 below.",
            "Muscle Loss",
            "A This passage paragraph describes the effects of long periods without exercise on human muscle.",
            "B This passage paragraph continues the discussion without containing any concrete question prompt.",
        ];
        let blocks = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                json!({
                    "blockId": format!("b{:03}", index + 1),
                    "blockType": "paragraph",
                    "text": text,
                    "html": format!("<p>{}</p>", text),
                    "pageIndex": 1,
                    "roleHint": if index == 1 { "question" } else { "passage" },
                    "confidence": 0.99
                })
            })
            .collect::<Vec<_>>();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "width": 595, "height": 842, "blocks": blocks}],
            "assets": []
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split.pointer("/questionGroupCandidates/0/questionRange"),
            Some(&json!([14, 26]))
        );
        assert_eq!(
            split
                .pointer("/questionGroupCandidates/0/requiresManualQuestionImport")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_ne!(
            split
                .pointer("/questionGroupCandidates/0/kindHint")
                .and_then(Value::as_str),
            Some("single_choice")
        );
    }

    #[test]
    fn explicitly_marked_passage_only_source_has_zero_groups_but_keeps_umbrella_evidence() {
        let mut job = test_job();
        job.title = "Muscle Loss".to_string();
        job.source_files = vec![SourceFile {
            file_id: "passage-only-source".to_string(),
            original_name: "121. P2(仅原文无题) - Muscle Loss 肌肉流失.pdf".to_string(),
            stored_name: "121. P2(仅原文无题) - Muscle Loss 肌肉流失.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: "passage-only".to_string(),
            size_bytes: 1,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        }];
        let texts = [
            "READING PASSAGE 2",
            "You should spend about 20 minutes on Questions 14-26, which are based on Reading",
            "Passage 2 below.",
            "Muscle Loss",
            "A This passage paragraph describes the effects of long periods without exercise on human muscle.",
            "B This passage paragraph continues the discussion without containing any concrete question prompt.",
        ];
        let blocks = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                json!({
                    "blockId": format!("b{:03}", index + 1),
                    "blockType": "paragraph",
                    "text": text,
                    "html": format!("<p>{}</p>", text),
                    "pageIndex": 1,
                    "roleHint": if index == 1 { "question" } else { "passage" },
                    "confidence": 0.99
                })
            })
            .collect::<Vec<_>>();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "width": 595, "height": 842, "blocks": blocks}],
            "assets": []
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .get("questionGroupCandidates")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            split.pointer("/umbrellaQuestionRanges/0/questionRange"),
            Some(&json!([14, 26]))
        );
        let issues = split
            .get("issues")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(issues.iter().any(|issue| {
            issue
                .as_str()
                .unwrap_or_default()
                .contains("explicitly marked as passage-only")
        }));
        assert!(!issues.iter().any(|issue| {
            issue
                .as_str()
                .unwrap_or_default()
                .contains("concrete question prompts must be imported")
        }));

        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            authoring
                .get("groups")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            authoring.pointer("/passage/questionUmbrellaRanges/0/questionRange"),
            Some(&json!([14, 26]))
        );
    }

    #[test]
    fn explicitly_marked_passage_only_source_keeps_numbered_prose_out_of_question_fallback() {
        let mut job = test_job();
        job.title = "A numbered history (passage-only)".to_string();
        let texts = [
            "READING PASSAGE 1",
            "Two stages in the development of the archive",
            "1. The first stage established a small collection for local researchers.",
            "2. The second stage opened the collection to visitors from other regions.",
        ];
        let blocks = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                json!({
                    "blockId": format!("b{:03}", index + 1),
                    "blockType": "paragraph",
                    "text": text,
                    "html": format!("<p>{}</p>", text),
                    "pageIndex": 1,
                    "roleHint": "passage",
                    "confidence": 0.99
                })
            })
            .collect::<Vec<_>>();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "width": 595, "height": 842, "blocks": blocks}],
            "assets": []
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .get("questionGroupCandidates")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            split.pointer("/passageCandidates/0/range"),
            Some(&json!(["b001", "b002", "b003", "b004"]))
        );
    }

    #[test]
    fn whale_oil_article_question_word_does_not_create_fallback_group() {
        let job = test_job();
        let blocks = vec![
            json!({
                "blockId": "b001",
                "blockType": "header",
                "text": "NATURE | Vol 450 | BOOKS & ARTS",
                "html": "<h3>NATURE | Vol 450 | BOOKS & ARTS</h3>",
                "pageIndex": 1,
                "confidence": 0.99
            }),
            json!({
                "blockId": "b002",
                "blockType": "paragraph",
                "text": "It is a question as pertinent today as it was in 1818, with taxonomists assessing different types of data.",
                "html": "<p>It is a question as pertinent today as it was in 1818.</p>",
                "pageIndex": 1,
                "confidence": 0.99
            }),
            json!({
                "blockId": "b003",
                "blockType": "paragraph",
                "text": "Appropriately, Burnett takes the debate out of the courtroom. Amid much else, the author questions whether Linnaeus really brought order to the taxonomic chaos.",
                "html": "<p>Appropriately, Burnett takes the debate out of the courtroom.</p>",
                "pageIndex": 1,
                // Reproduce the stale role emitted for the real Whale Oil PDF
                // before parser question-role detection was tightened.
                "roleHint": "question",
                "confidence": 0.99
            }),
            json!({
                "blockId": "b004",
                "blockType": "paragraph",
                "text": "The ensuing legal tussle had to settle the superficially simple question of whether whale oil was fish oil.",
                "html": "<p>The ensuing legal tussle had to settle the issue.</p>",
                "pageIndex": 1,
                "confidence": 0.99
            }),
        ];
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "width": 595, "height": 842, "blocks": blocks}],
            "assets": []
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .get("questionGroupCandidates")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            split.pointer("/passageCandidates/0/range"),
            Some(&json!(["b001", "b002", "b003", "b004"]))
        );

        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            authoring
                .get("groups")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn long_numbered_article_sections_do_not_become_heading_free_questions() {
        let job = test_job();
        let long_prose = "Tourism changes local institutions and economic relationships over many years while residents, public agencies, investors, and visitors respond to incentives that were never designed as examination tasks. The discussion continues with historical examples, qualifications, competing interpretations, and consequences for communities, infrastructure, culture, employment, public finance, conservation, and political authority across multiple regions and generations.";
        let texts = [
            "READING PASSAGE 3",
            "Nine tensions in the development of tourism",
            "1. Tourism promises authenticity while changing the host community.",
            long_prose,
            "2. Investment can expand access while increasing dependency.",
            long_prose,
            "3. What is environmentally sustainable is often commercially difficult.",
            long_prose,
            "4. Cultural preservation can also transform the culture being presented.",
            long_prose,
        ];
        let blocks = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                json!({
                    "blockId": format!("b{:03}", index + 1),
                    "blockType": "paragraph",
                    "text": text,
                    "html": format!("<p>{}</p>", text),
                    "pageIndex": 1 + index / 4,
                    "roleHint": if index >= 2 { "question" } else { "passage" },
                    "confidence": 0.99
                })
            })
            .collect::<Vec<_>>();
        let all_block_ids = blocks.iter().map(dynamic_block_id).collect::<Vec<_>>();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "width": 595, "height": 842, "blocks": blocks}],
            "assets": []
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .get("questionGroupCandidates")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            split.pointer("/passageCandidates/0/range"),
            Some(&json!(all_block_ids))
        );
        assert!(split
            .get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|issue| issue.starts_with("QUESTION_STRUCTURE_NOT_DETECTED:")));

        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            authoring
                .get("groups")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn short_explicit_questions_without_a_range_heading_remain_recoverable() {
        let job = test_job();
        let texts = [
            "The archive preserves records from the nineteenth century.",
            "1 What service did the archive introduce first?",
            "2 Why did local researchers support the change?",
        ];
        let blocks = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                json!({
                    "blockId": format!("b{:03}", index + 1),
                    "blockType": "paragraph",
                    "text": text,
                    "html": format!("<p>{}</p>", text),
                    "pageIndex": 1,
                    "roleHint": if index == 0 { "passage" } else { "question" },
                    "confidence": 0.99
                })
            })
            .collect::<Vec<_>>();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "width": 595, "height": 842, "blocks": blocks}],
            "assets": []
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split.pointer("/questionGroupCandidates/0/questionRange"),
            Some(&json!([1, 2]))
        );
        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            authoring
                .pointer("/groups/0/questions")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(authoring
            .pointer("/groups/0/questions/0/prompt")
            .and_then(Value::as_str)
            .is_some_and(|prompt| prompt.ends_with('?')));
    }

    #[test]
    fn heading_free_fallback_accepts_explicit_ielts_instruction_evidence() {
        let job = test_job();
        let blocks = vec![
            json!({
                "blockId": "b001",
                "blockType": "paragraph",
                "text": "The archive preserves records from the nineteenth century.",
                "html": "<p>The archive preserves records from the nineteenth century.</p>",
                "pageIndex": 1,
                "roleHint": "passage"
            }),
            json!({
                "blockId": "b002",
                "blockType": "paragraph",
                "text": "Choose the correct answer for each question.",
                "html": "<p>Choose the correct answer for each question.</p>",
                "pageIndex": 1,
                "roleHint": "question"
            }),
            json!({
                "blockId": "b003",
                "blockType": "paragraph",
                "text": "The extracted question area requires source review.",
                "html": "<p>The extracted question area requires source review.</p>",
                "pageIndex": 1,
                "roleHint": "question"
            }),
            json!({
                "blockId": "b004",
                "blockType": "paragraph",
                "text": "Answers 1 A 2 B",
                "html": "<p>Answers 1 A 2 B</p>",
                "pageIndex": 1,
                "roleHint": "answer"
            }),
        ];
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "width": 595, "height": 842, "blocks": blocks}],
            "assets": []
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .get("questionGroupCandidates")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            split.pointer("/questionGroupCandidates/0/questionRange"),
            Some(&json!([1, 2]))
        );
    }

    #[test]
    fn passage_only_marker_does_not_remove_an_explicit_concrete_question_group() {
        let mut job = test_job();
        job.tags = vec!["passage-only".to_string()];
        let texts = [
            "READING PASSAGE 1",
            "The archive expanded gradually over several decades.",
            "Questions 1-2 Choose the correct letter, A, B, C or D.",
            "1 What did the archive collect first? A maps B coins C tools D letters",
            "2 Who could initially use it? A pupils B local researchers C tourists D traders",
        ];
        let blocks = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                json!({
                    "blockId": format!("b{:03}", index + 1),
                    "blockType": "paragraph",
                    "text": text,
                    "html": format!("<p>{}</p>", text),
                    "pageIndex": 1,
                    "confidence": 0.99
                })
            })
            .collect::<Vec<_>>();
        let doc = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"pageIndex": 1, "width": 595, "height": 842, "blocks": blocks}],
            "assets": []
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        assert_eq!(
            split
                .get("questionGroupCandidates")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            split.pointer("/questionGroupCandidates/0/questionRange"),
            Some(&json!([1, 2]))
        );
        assert_ne!(
            split
                .pointer("/questionGroupCandidates/0/requiresManualQuestionImport")
                .and_then(Value::as_bool),
            Some(true)
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
    fn completion_prose_continuing_into_right_column_stays_in_question_group() {
        let blocks = vec![
            layout_block(
                "q001",
                "Questions 22-26 Complete the summary below.",
                [28.0, 790.0, 260.0, 802.0],
                0,
                2,
                0,
            ),
            layout_block(
                "q002",
                "22 ________ at work. On average, workers who",
                [28.0, 710.0, 274.0, 722.0],
                0,
                2,
                0,
            ),
            layout_block(
                "q003",
                "take time off because of",
                [306.0, 790.0, 430.0, 802.0],
                0,
                2,
                1,
            ),
            layout_block(
                "q004",
                "stress stay away for 23 ________.",
                [306.0, 778.0, 520.0, 790.0],
                0,
                2,
                1,
            ),
        ];

        assert!(collect_dynamic_completion_interleaved_passage_runs(&blocks).is_empty());
    }

    #[test]
    fn final_completion_gap_does_not_absorb_prose_from_the_next_page() {
        let blocks = vec![
            layout_block_on_page(
                "q001",
                "Questions 22-26 Complete the summary below.",
                [54.0, 508.0, 530.0, 520.0],
                2,
                0,
                1,
                0,
            ),
            layout_block_on_page(
                "q002",
                "25 ________. However, the final aim may be the 26 ________ of space, and",
                [54.0, 218.0, 536.0, 230.0],
                2,
                0,
                1,
                0,
            ),
            layout_block_on_page(
                "p001",
                "many debates. There is no doubt that the vehicle becomes much more complex and costly when people are on board.",
                [76.0, 584.0, 537.0, 701.0],
                3,
                0,
                1,
                0,
            ),
        ];

        assert_eq!(
            dynamic_completion_final_gap_task_end("summary_completion", &blocks),
            Some(2)
        );
        assert_eq!(
            dynamic_question_block_count_for_group("summary_completion", &blocks),
            2
        );
    }

    #[test]
    fn final_completion_gap_keeps_tight_same_stream_sentence_wrap() {
        let blocks = vec![
            layout_block_on_page(
                "q001",
                "Questions 22-26 Complete the summary below.",
                [54.0, 508.0, 530.0, 520.0],
                2,
                4,
                1,
                0,
            ),
            layout_block_on_page(
                "q002",
                "The final aim may be the 26 ________ of the",
                [54.0, 238.0, 536.0, 250.0],
                2,
                4,
                1,
                0,
            ),
            layout_block_on_page(
                "q003",
                "nearest planets.",
                [54.0, 220.0, 180.0, 232.0],
                2,
                4,
                1,
                0,
            ),
            layout_block_on_page(
                "p001",
                "Unrelated passage prose begins after a visible vertical separation and must remain outside the task.",
                [54.0, 120.0, 530.0, 160.0],
                2,
                4,
                1,
                0,
            ),
        ];

        assert_eq!(
            dynamic_completion_final_gap_task_end("summary_completion", &blocks),
            Some(3)
        );
    }

    #[test]
    fn final_completion_gap_stops_before_another_numbered_gap() {
        let blocks = vec![
            layout_block(
                "q001",
                "Question 1 Complete the sentence below.",
                [54.0, 508.0, 530.0, 520.0],
                0,
                1,
                0,
            ),
            layout_block(
                "q002",
                "The result was 1 ________ and",
                [54.0, 238.0, 536.0, 250.0],
                0,
                1,
                0,
            ),
            layout_block(
                "q003",
                "2 ________ in the later exercise.",
                [54.0, 220.0, 300.0, 232.0],
                0,
                1,
                0,
            ),
        ];

        assert_eq!(
            dynamic_completion_final_gap_task_end("sentence_completion", &blocks),
            Some(2)
        );
    }

    #[test]
    fn whole_paper_umbrella_after_completion_is_not_absorbed_by_last_prompt() {
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
                            "q001",
                            "Questions 1-2 Complete the summary below. Choose ONE WORD ONLY from the passage for each answer.",
                            [54.0, 720.0, 530.0, 746.0],
                            1,
                            0,
                            1,
                            0,
                        ),
                        layout_block_on_page(
                            "q002",
                            "The first result was 1 ________.",
                            [54.0, 650.0, 530.0, 672.0],
                            1,
                            0,
                            1,
                            0,
                        ),
                        layout_block_on_page(
                            "q003",
                            "2 ________.",
                            [54.0, 620.0, 530.0, 642.0],
                            1,
                            0,
                            1,
                            0,
                        ),
                    ]
                },
                {
                    "pageIndex": 2,
                    "width": 595.0,
                    "height": 842.0,
                    "blocks": [
                        layout_block_on_page(
                            "p001",
                            "READING PASSAGE 1",
                            [54.0, 760.0, 530.0, 786.0],
                            2,
                            0,
                            1,
                            0,
                        ),
                        layout_block_on_page(
                            "u001",
                            "You should spend about 20 minutes on Questions 1-2, which are based on Reading Passage 1.",
                            [54.0, 720.0, 530.0, 742.0],
                            2,
                            0,
                            1,
                            0,
                        ),
                        layout_block_on_page(
                            "p002",
                            "The source article begins here and contains enough substantive prose to establish passage ownership.",
                            [54.0, 660.0, 530.0, 700.0],
                            2,
                            0,
                            1,
                            0,
                        ),
                        layout_block_on_page(
                            "p003",
                            "A second paragraph continues the discussion without supplying any additional question stimulus.",
                            [54.0, 600.0, 530.0, 640.0],
                            2,
                            0,
                            1,
                            0,
                        ),
                    ]
                }
            ],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let group_ids = split
            .pointer("/questionGroupCandidates/0/blockIds")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(group_ids, vec!["q001", "q002", "q003"]);
        assert_eq!(
            split
                .pointer("/umbrellaQuestionRanges/0/blockId")
                .and_then(Value::as_str),
            Some("u001")
        );

        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        assert_eq!(
            authoring
                .pointer("/groups/0/questions/1/prompt")
                .and_then(Value::as_str),
            Some("________.")
        );
        assert!(!authoring
            .pointer("/groups/0/questions/1/sourceBlockIds")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|id| id.as_str() == Some("u001")));
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
                0,
            ),
            layout_block_on_page(
                "p003",
                "new ideas about mathematics later transformed European science",
                [72.0, 760.0, 520.0, 800.0],
                2,
                0,
                1,
                0,
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
            kind, "true_false_not_given",
            "split instruction across two blocks should still classify as true_false_not_given"
        );
    }

    #[test]
    fn multi_column_response_legend_stays_in_true_false_not_given_group() {
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
                        "The passage describes how merchants developed faster trading routes.",
                        [72.0, 690.0, 520.0, 736.0],
                        0,
                        1,
                        0
                    ),
                    layout_block(
                        "q001",
                        "Questions 1-2",
                        [72.0, 340.0, 520.0, 365.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "q002",
                        "Do the following statements agree with the information given in Reading Passage 1?",
                        [72.0, 315.0, 520.0, 335.0],
                        1,
                        1,
                        0
                    ),
                    layout_block(
                        "q003",
                        "In boxes 1-2 on your answer sheet, write",
                        [72.0, 290.0, 520.0, 310.0],
                        2,
                        1,
                        0
                    ),
                    layout_block("q004", "TRUE", [72.0, 265.0, 120.0, 285.0], 3, 3, 0),
                    layout_block(
                        "q005",
                        "if the statement agrees with the information",
                        [150.0, 265.0, 350.0, 285.0],
                        3,
                        3,
                        1
                    ),
                    layout_block("q006", "FALSE", [72.0, 240.0, 120.0, 260.0], 4, 3, 0),
                    layout_block(
                        "q007",
                        "if the statement contradicts the information",
                        [150.0, 240.0, 350.0, 260.0],
                        4,
                        3,
                        1
                    ),
                    layout_block(
                        "q008",
                        "NOT GIVEN",
                        [72.0, 215.0, 140.0, 235.0],
                        5,
                        3,
                        0
                    ),
                    layout_block(
                        "q009",
                        "if there is no information on this",
                        [150.0, 215.0, 350.0, 235.0],
                        5,
                        3,
                        1
                    ),
                    layout_block(
                        "q010",
                        "1 The company faced strong competition.",
                        [72.0, 180.0, 520.0, 200.0],
                        6,
                        1,
                        0
                    ),
                    layout_block(
                        "q011",
                        "2 The fastest voyage took less than a year.",
                        [72.0, 150.0, 520.0, 170.0],
                        6,
                        1,
                        0
                    )
                ]
            }],
            "assets": [],
            "parser": {"provider":"unit-test","version":"0.0.0","mode":"auto","warnings":[]}
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let candidate = &split["questionGroupCandidates"][0];
        assert_eq!(
            candidate.get("kindHint").and_then(Value::as_str),
            Some("true_false_not_given")
        );
        assert!(candidate
            .get("instructionText")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("NOT GIVEN"));
        let block_ids = candidate
            .get("blockIds")
            .and_then(Value::as_array)
            .expect("group block ids");
        assert!(block_ids.contains(&json!("q008")));
        assert!(block_ids.contains(&json!("q009")));
    }

    #[test]
    fn narrow_ielts_instruction_fragments_are_not_passage_prose() {
        for text in [
            "YES if the statement agrees with the claims of the writer",
            "NOT GIVEN",
            "if it is im possible to say what the writer thinks",
            "Match each person with the correct finding.",
            "NB You may use any letter more than once.",
        ] {
            assert!(
                is_dynamic_question_or_instruction_like_text(text),
                "expected IELTS instruction fragment: {text}"
            );
        }
        assert!(!is_dynamic_question_or_instruction_like_text(
            "The two colours match the markings found on the earlier vessel."
        ));
        for prose in [
            "If there is no rainfall in spring, the seedlings remain dormant.",
            "If there is no information available, historians consult the physical archive.",
            "If there is no information given in the records, researchers consult the physical archive.",
            "If the statement carved into the tablet is genuine, it dates from the earlier period.",
        ] {
            assert!(
                !is_dynamic_question_or_instruction_like_text(prose),
                "ordinary passage prose was mistaken for a response legend: {prose}"
            );
        }
    }

    #[test]
    fn question_range_heading_does_not_poison_following_flattened_question_numbers() {
        let text = "Questions 1-3 1 The diaries are stored here 2 Visitors can inspect maps 3 The archive moved";
        let (_, content_start) = find_dynamic_number_marker(text, 1, 0).expect("q1 marker");
        assert_eq!(
            collapse_whitespace(&text[content_start..])
                .split_whitespace()
                .next(),
            Some("The")
        );
        let (_, q2_start) = find_dynamic_number_marker(text, 2, 0).expect("q2 marker");
        assert_eq!(
            collapse_whitespace(&text[q2_start..])
                .split_whitespace()
                .next(),
            Some("Visitors")
        );
        let and_text =
            "Questions 1 and 2 Choose the correct letter 1 The first stem 2 The second stem";
        let (_, and_q2_start) =
            find_dynamic_number_marker(and_text, 2, 0).expect("q2 after and heading");
        assert_eq!(
            collapse_whitespace(&and_text[and_q2_start..])
                .split_whitespace()
                .next(),
            Some("The")
        );
    }

    #[test]
    fn pipe_table_rows_recover_distinct_prompts_only_with_closed_row_mapping() {
        let blocks = vec![
            json!({"blockId":"b1","text":"Item | Location"}),
            json!({"blockId":"b2","text":"maps | room"}),
            json!({"blockId":"b3","text":"diaries | room"}),
        ];
        let q4 = dynamic_table_row_prompt_for_number(&blocks, 4, 4, 5).expect("q4 row");
        let q5 = dynamic_table_row_prompt_for_number(&blocks, 5, 4, 5).expect("q5 row");
        assert_eq!(q4.0, "maps | room");
        assert_eq!(q5.0, "diaries | room");
        assert_eq!(q4.1, "b2");
        assert_eq!(q5.1, "b3");
        assert!(q4.2 && q5.2, "ordinal mapping must be review-visible");

        let ambiguous = vec![
            json!({"blockId":"h","text":"Item | Location"}),
            json!({"blockId":"r1","text":"maps | room"}),
            json!({"blockId":"r2","text":"diaries | room"}),
            json!({"blockId":"r3","text":"tools | room"}),
        ];
        assert!(dynamic_table_row_prompt_for_number(&ambiguous, 4, 4, 5).is_none());

        let flattened = vec![json!({
            "blockId":"flat",
            "text":"Feature | Function | clues | help readers reason | alibis | complicate the mystery | narrators | guide interpretation"
        })];
        assert_eq!(
            dynamic_table_row_prompt_for_number(&flattened, 6, 6, 8).map(|row| row.0),
            Some("clues | help readers reason".to_string())
        );
        assert_eq!(
            dynamic_table_row_prompt_for_number(&flattened, 8, 6, 8).map(|row| row.0),
            Some("narrators | guide interpretation".to_string())
        );
    }

    #[test]
    fn unresolved_non_manual_prompt_stays_empty_and_is_marked_manual() {
        let job = test_job();
        let doc = json!({
            "schemaVersion":"DocumentIRV1",
            "pages":[{"pageIndex":1,"blocks":[
                {"blockId":"b1","blockType":"header","text":"Questions 1-2 Choose the correct letter."}
            ]}],
            "assets":[]
        });
        let split = json!({
            "questionGroupCandidates":[{
                "groupId":"group-1","heading":"Questions 1-2",
                "instructionText":"Questions 1-2 Choose the correct letter.",
                "questionRange":[1,2],"kindHint":"single_choice","layoutHint":"list",
                "blockIds":["b1"],"classification":{"interaction":{"type":"radio","options":["A","B","C"]}}
            }],
            "passageCandidates":[],"issues":[]
        });
        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        for question in ir["groups"][0]["questions"].as_array().unwrap() {
            assert_eq!(question["prompt"], json!(""));
            assert_eq!(question["requiresManualQuestionImport"], json!(true));
            assert!(!question["prompt"]
                .as_str()
                .unwrap()
                .contains("Questions 1 item"));
        }
        assert_eq!(ir["groups"][0]["requiresManualQuestionImport"], json!(true));
    }

    #[test]
    fn shared_multi_choice_stem_and_options_survive_wrapped_blocks() {
        let job = test_job();
        let texts = [
            "READING PASSAGE 1",
            "Questions 14 and 15",
            "Choose TWO letters, A-E.",
            "Write the correct letters in boxes 14 and 15.",
            "According to the writer, which TWO of the following are characteristics of the classical",
            "approach to organisational design?",
            "A a marked ranking order for employees",
            "B giving importance to everyone's work",
            "C the advancement of older workers",
            "D a neutral working environment",
            "E increased benefits for workers",
        ];
        let blocks = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                json!({
                    "blockId": format!("b{}", index + 1), "blockType":"paragraph", "text":text,
                    "html":format!("<p>{}</p>", text), "pageIndex":1, "confidence":0.99
                })
            })
            .collect::<Vec<_>>();
        let doc = json!({"schemaVersion":"DocumentIRV1","pages":[{"pageIndex":1,"blocks":blocks}],"assets":[]});
        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let ir = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        for question in ir["groups"][0]["questions"].as_array().unwrap() {
            let prompt = question["prompt"].as_str().unwrap();
            assert!(
                prompt.contains("characteristics"),
                "unexpected shared prompt: {prompt}"
            );
            assert!(!prompt.starts_with("Questions "));
            assert_eq!(
                question["interaction"]["optionTexts"]
                    .as_object()
                    .unwrap()
                    .len(),
                5
            );
        }
    }

    #[test]
    fn declared_letter_banks_close_through_n_without_truncation() {
        let labels = ('A'..='N')
            .map(|label| label.to_string())
            .collect::<Vec<_>>();
        let blocks = std::iter::once(json!({
            "blockId": "instruction",
            "blockType": "paragraph",
            "text": "Complete the summary using the list of phrases, A-N, below."
        }))
        .chain(('A'..='N').map(|label| {
            json!({
                "blockId": format!("option-{label}"),
                "blockType": "paragraph",
                "text": format!("{label} phrase {label}")
            })
        }))
        .collect::<Vec<_>>();

        assert_eq!(dynamic_letter_options_for_text("Choose from A-N."), labels);
        assert_eq!(
            dynamic_declared_letter_bank_labels("Use the list A-N below."),
            labels
        );
        let options = dynamic_group_option_bank(&blocks, "summary_completion");
        assert_eq!(
            options
                .iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>(),
            ('A'..='N')
                .map(|label| label.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(options.last().map(|item| item.1.as_str()), Some("phrase N"));
    }

    #[test]
    fn complete_post_question_matching_bank_is_grafted_and_preserves_wrapping() {
        let job = test_job();
        let mut blocks = vec![
            json!({"blockId":"passage","blockType":"header","text":"READING PASSAGE 2","pageIndex":2,"bbox":[54.0,790.0,220.0,804.0]}),
            json!({"blockId":"heading","blockType":"header","text":"Questions 21-26","pageIndex":2,"bbox":[54.0,750.0,145.0,762.0]}),
            json!({"blockId":"instruction","blockType":"paragraph","text":"Match each method with the correct description, A-H.","pageIndex":2,"bbox":[54.0,720.0,430.0,734.0]}),
        ];
        for number in 21..=26 {
            let offset = (number - 21) as f64 * 24.0;
            blocks.push(json!({
                "blockId": format!("q{number}"),
                "blockType": "paragraph",
                "text": format!("{number} Method {number}"),
                "pageIndex": 2,
                "bbox": [54.0, 675.0 - offset, 260.0, 689.0 - offset]
            }));
        }
        blocks.extend([
            json!({"blockId":"bank-heading","blockType":"paragraph","text":"List of Descriptions","pageIndex":2,"bbox":[300.0,520.0,430.0,534.0]}),
            json!({"blockId":"bank-a","blockType":"paragraph","text":"A first description","pageIndex":2,"bbox":[300.0,494.0,430.0,508.0]}),
            json!({"blockId":"bank-b","blockType":"paragraph","text":"B second description","pageIndex":2,"bbox":[300.0,470.0,430.0,484.0]}),
            json!({"blockId":"bank-c","blockType":"paragraph","text":"C applies to single-sex or","pageIndex":2,"bbox":[300.0,446.0,430.0,460.0]}),
            json!({"blockId":"bank-c-wrap","blockType":"paragraph","text":"mixed settings.","pageIndex":2,"bbox":[316.0,429.0,415.0,442.0]}),
            json!({"blockId":"bank-d","blockType":"paragraph","text":"D fourth description","pageIndex":2,"bbox":[300.0,405.0,430.0,419.0]}),
            json!({"blockId":"bank-e","blockType":"paragraph","text":"E fifth description","pageIndex":2,"bbox":[300.0,381.0,430.0,395.0]}),
            json!({"blockId":"bank-f","blockType":"paragraph","text":"F sixth description","pageIndex":2,"bbox":[300.0,357.0,430.0,371.0]}),
            json!({"blockId":"bank-g","blockType":"paragraph","text":"G seventh description","pageIndex":2,"bbox":[300.0,333.0,430.0,347.0]}),
            json!({"blockId":"bank-h","blockType":"paragraph","text":"H eighth description","pageIndex":2,"bbox":[300.0,309.0,430.0,323.0]}),
            json!({"blockId":"next-heading","blockType":"header","text":"Questions 27-30","pageIndex":2,"bbox":[54.0,270.0,145.0,282.0]}),
            json!({"blockId":"next-instruction","blockType":"paragraph","text":"Complete the notes below. Choose ONE WORD ONLY.","pageIndex":2,"bbox":[54.0,244.0,430.0,258.0]}),
        ]);
        let doc = json!({
            "schemaVersion":"DocumentIRV1",
            "pages":[{"pageIndex":2,"width":595.0,"height":842.0,"blocks":blocks}],
            "assets":[]
        });

        let group_text = "Questions 21-26 Match each method with the correct description, A-H.";
        let first_classification = classify_dynamic_group(group_text, &[]);
        assert_eq!(first_classification.kind, "matching");
        let mut groups = vec![
            SplitGroupCandidateV1 {
                group_id: "group-1".to_string(),
                heading: "Questions 21-26".to_string(),
                question_range: [21, 26],
                instruction_text: group_text.to_string(),
                block_ids: std::iter::once("heading".to_string())
                    .chain(std::iter::once("instruction".to_string()))
                    .chain((21..=26).map(|number| format!("q{number}")))
                    .collect(),
                kind_hint: "matching".to_string(),
                layout_hint: Some("list".to_string()),
                confidence: first_classification.confidence,
                classification: Some(first_classification),
                section_evidence: Vec::new(),
                continuation_edges: Vec::new(),
                is_umbrella_range: None,
                requires_manual_question_import: None,
            },
            SplitGroupCandidateV1 {
                group_id: "group-2".to_string(),
                heading: "Questions 27-30".to_string(),
                question_range: [27, 30],
                instruction_text: "Questions 27-30 Complete the notes below. Choose ONE WORD ONLY."
                    .to_string(),
                block_ids: vec!["next-heading".to_string(), "next-instruction".to_string()],
                kind_hint: "sentence_completion".to_string(),
                layout_hint: Some("inline_completion".to_string()),
                confidence: 0.8,
                classification: None,
                section_evidence: Vec::new(),
                continuation_edges: Vec::new(),
                is_umbrella_range: None,
                requires_manual_question_import: None,
            },
        ];
        extend_dynamic_matching_option_blocks(&mut groups, &blocks);
        assert!(groups[0].block_ids.contains(&"bank-a".to_string()));
        assert!(groups[0].block_ids.contains(&"bank-h".to_string()));
        assert!(!groups[1].block_ids.contains(&"bank-a".to_string()));

        let split = SplitCandidatesV1 {
            job_id: job.job_id.clone(),
            passage_candidates: vec![],
            question_group_candidates: groups,
            answer_key_candidates: vec![],
            umbrella_question_ranges: vec![],
            issues: vec![],
        }
        .to_value();
        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        for question in authoring["groups"][0]["questions"].as_array().unwrap() {
            assert_eq!(
                question["interaction"]["options"],
                json!(["A", "B", "C", "D", "E", "F", "G", "H"])
            );
            assert_eq!(
                question["interaction"]["optionTexts"]["C"],
                json!("applies to single-sex or mixed settings.")
            );
        }
    }

    #[test]
    fn summary_bank_keeps_prose_stimulus_and_recovers_split_word_before_next_label() {
        let job = test_job();
        let texts = [
            ("heading", "Questions 27-32"),
            (
                "instruction",
                "Complete the summary using the list of phrases, A-L, below.",
            ),
            (
                "write",
                "Write the correct letter, A-L, in boxes 27-32 on your answer sheet.",
            ),
            (
                "context-1",
                "Multinational companies seek several approaches when language barriers affect daily operations.",
            ),
            (
                "context-2",
                "Using the native language gives them a realistic base in another country, but problems arise with overseas",
            ),
            ("q27", "27 ________. For example, key"),
            ("q28", "28 ________ differ between countries."),
            ("q29", "29 ________ can support spoken language."),
            ("q30", "30 ________ are processed in another language."),
            ("q31", "31 ________; translators may also need"),
            ("q32", "32 ________ work."),
            ("bank-a", "A gestures"),
            ("bank-bc", "B clients C transa"),
            ("bank-d", "ctions D assumption"),
            (
                "bank-eh",
                "E accurate F documents G managers H body language",
            ),
            ("bank-il", "I long-term J effective K rivals L costly"),
            ("next-heading", "Questions 33-39"),
            (
                "next-instruction",
                "Answer the questions below. Choose NO MORE THAN THREE WORDS from the passage.",
            ),
        ];
        let blocks = texts
            .iter()
            .enumerate()
            .map(|(index, (block_id, text))| {
                let (x0, x1, y) = match *block_id {
                    "bank-bc" => (190.0, 342.0, 420.0),
                    "bank-d" => (349.0, 476.0, 420.0),
                    _ => (54.0, 530.0, 780.0 - index as f64 * 24.0),
                };
                json!({
                    "blockId": block_id,
                    "blockType": if block_id.contains("heading") { "header" } else { "paragraph" },
                    "text": text,
                    "pageIndex": 3,
                    "bbox": [x0, y, x1, y + 13.0]
                })
            })
            .collect::<Vec<_>>();
        let doc = json!({
            "schemaVersion":"DocumentIRV1",
            "pages":[{"pageIndex":3,"width":595.0,"height":842.0,"blocks":blocks}],
            "assets":[]
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let first_ids = split["questionGroupCandidates"][0]["blockIds"]
            .as_array()
            .unwrap();
        for expected in ["context-1", "context-2", "bank-d", "bank-eh", "bank-il"] {
            assert!(
                first_ids.contains(&json!(expected)),
                "completion task lost source block {expected}: {first_ids:?}"
            );
        }
        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let interaction = &authoring["groups"][0]["questions"][0]["interaction"];
        assert_eq!(
            interaction["options"],
            json!(["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L"])
        );
        assert_eq!(interaction["optionTexts"]["C"], json!("transactions"));
        assert_eq!(interaction["optionTexts"]["D"], json!("assumption"));
    }

    #[test]
    fn column_major_summary_bank_ignores_interleaved_disclaimer_and_closes() {
        let job = test_job();
        let blocks = vec![
            json!({"blockId":"heading","blockType":"header","text":"Questions 38-40","pageIndex":5}),
            json!({"blockId":"instruction","blockType":"paragraph","text":"Complete the summary using the list of words, A-H, below.","pageIndex":5}),
            json!({"blockId":"write","blockType":"paragraph","text":"Write the correct letter, A-H, in boxes 38-40.","pageIndex":5}),
            json!({"blockId":"context","blockType":"paragraph","text":"The museum has wide appeal, but its design creates a","pageIndex":5}),
            json!({"blockId":"q38","blockType":"paragraph","text":"38 ________ effect for visitors, and the architect","pageIndex":5}),
            json!({"blockId":"q38-wrap-1","blockType":"paragraph","text":"had a different objective when designing the underground gallery:","pageIndex":5}),
            json!({"blockId":"q38-wrap-2","blockType":"paragraph","text":"the building presented a practical problem before the","pageIndex":5}),
            json!({"blockId":"q39","blockType":"paragraph","text":"39 ________ could be resolved. Its future","pageIndex":5}),
            json!({"blockId":"q40","blockType":"paragraph","text":"40 ________ remains uncertain","pageIndex":5}),
            json!({"blockId":"bank-a","blockType":"paragraph","text":"A challenge","pageIndex":5}),
            json!({"blockId":"bank-e","blockType":"paragraph","text":"E lasting","pageIndex":5}),
            json!({"blockId":"disclaimer","blockType":"paragraph","text":"Disclaimer","pageIndex":5}),
            json!({"blockId":"bank-b","blockType":"paragraph","text":"B pipe","pageIndex":5}),
            json!({"blockId":"bank-f","blockType":"paragraph","text":"F essential","pageIndex":5}),
            json!({"blockId":"bank-cd","blockType":"paragraph","text":"C visual D contribution","pageIndex":5}),
            json!({"blockId":"bank-gh","blockType":"paragraph","text":"G ambition H attraction","pageIndex":5}),
            json!({"blockId":"footer","blockType":"paragraph","text":"Compiled and formatted for non-commercial educational use only.","pageIndex":5}),
        ];
        let doc = json!({
            "schemaVersion":"DocumentIRV1",
            "pages":[{"pageIndex":5,"width":595.0,"height":842.0,"blocks":blocks}],
            "assets":[]
        });

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let ids = split["questionGroupCandidates"][0]["blockIds"]
            .as_array()
            .unwrap();
        assert!(ids.contains(&json!("q38-wrap-1")));
        assert!(ids.contains(&json!("q38-wrap-2")));
        assert!(ids.contains(&json!("bank-gh")));
        assert!(!ids.contains(&json!("disclaimer")));
        assert!(!ids.contains(&json!("footer")));

        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let interaction = &authoring["groups"][0]["questions"][0]["interaction"];
        assert_eq!(
            interaction["options"],
            json!(["A", "B", "C", "D", "E", "F", "G", "H"])
        );
        assert_eq!(interaction["optionTexts"]["E"], json!("lasting"));
    }

    #[test]
    fn wrapped_choice_runs_close_for_abc_plus_d_and_ab_plus_cd() {
        let instrument = vec![
            json!({"blockId":"q34","text":"34 Which aspect of playing an instrument was included"}),
            json!({"blockId":"q34-wrap-1","text":"in the follow-up study"}),
            json!({"blockId":"q34-wrap-2","text":"but not in the first study?"}),
            json!({"blockId":"q34-abc","text":"A duration B starting age C years since playing"}),
            json!({"blockId":"q34-d","text":"D childhood practice"}),
        ];
        let (prompt, options) = dynamic_question_prompt_and_options(
            &instrument,
            "Choose the correct letter, A, B, C or D.",
            34,
            "Questions 34-34",
            34,
            "single_choice",
        );
        assert_eq!(
            prompt,
            "Which aspect of playing an instrument was included in the follow-up study but not in the first study?"
        );
        assert_eq!(
            options
                .iter()
                .map(|item| item.0.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B", "C", "D"]
        );

        let ocean = vec![
            json!({"blockId":"q12","text":"12 When do the signals become easiest to observe?"}),
            json!({"blockId":"q12-ab","text":"A when scientists track them. B at different times of year."}),
            json!({"blockId":"q12-cd","text":"C during calm weather. D after storms."}),
        ];
        let (_, options) = dynamic_question_prompt_and_options(
            &ocean,
            "Choose the correct letter, A, B, C or D.",
            12,
            "Questions 12-12",
            12,
            "single_choice",
        );
        assert_eq!(
            options,
            vec![
                ("A".to_string(), "when scientists track them.".to_string()),
                ("B".to_string(), "at different times of year.".to_string()),
                ("C".to_string(), "during calm weather.".to_string()),
                ("D".to_string(), "after storms.".to_string()),
            ]
        );
    }

    #[test]
    fn choice_option_absorbs_indented_wrapped_source_lines_until_next_label() {
        let blocks = vec![
            json!({"blockId":"q35","text":"35 What is the writer's main conclusion?","pageIndex":4,"_epic8LayoutSection":1,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[57.0,420.0,310.0,430.0]}),
            json!({"blockId":"a","text":"A Greater emphasis will benefit both patients and","pageIndex":4,"_epic8LayoutSection":1,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[57.0,390.0,507.0,400.0]}),
            json!({"blockId":"a-wrap-1","text":"practitioners in their daily work","pageIndex":4,"_epic8LayoutSection":1,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[78.0,376.0,260.0,386.0]}),
            json!({"blockId":"a-wrap-2","text":"and during later training.","pageIndex":4,"_epic8LayoutSection":1,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[78.0,362.0,220.0,372.0]}),
            json!({"blockId":"b","text":"B A different conclusion.","pageIndex":4,"_epic8LayoutSection":1,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[57.0,340.0,240.0,350.0]}),
            json!({"blockId":"c","text":"C A third conclusion.","pageIndex":4,"_epic8LayoutSection":1,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[57.0,320.0,230.0,330.0]}),
            json!({"blockId":"d","text":"D A final conclusion.","pageIndex":4,"_epic8LayoutSection":1,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[57.0,300.0,230.0,310.0]}),
        ];
        let (prompt, options) = dynamic_question_prompt_and_options(
            &blocks,
            "Choose the correct letter, A, B, C or D.",
            35,
            "Questions 35-35",
            35,
            "single_choice",
        );

        assert_eq!(prompt, "What is the writer's main conclusion?");
        assert_eq!(
            options[0],
            (
                "A".to_string(),
                "Greater emphasis will benefit both patients and practitioners in their daily work and during later training.".to_string()
            )
        );
    }

    #[test]
    fn choice_option_does_not_absorb_unlabelled_text_from_another_layout_column() {
        let blocks = vec![
            json!({"blockId":"q21","text":"21 Which statement is correct?","pageIndex":3,"_epic8LayoutSection":2,"_epic8SectionColumns":2,"_epic8ColumnIndex":0,"bbox":[40.0,500.0,260.0,510.0]}),
            json!({"blockId":"a","text":"A the source-backed answer","pageIndex":3,"_epic8LayoutSection":2,"_epic8SectionColumns":2,"_epic8ColumnIndex":0,"bbox":[40.0,470.0,250.0,480.0]}),
            json!({"blockId":"other-column","text":"unrelated passage prose","pageIndex":3,"_epic8LayoutSection":2,"_epic8SectionColumns":2,"_epic8ColumnIndex":1,"bbox":[320.0,456.0,520.0,466.0]}),
            json!({"blockId":"b","text":"B the second answer","pageIndex":3,"_epic8LayoutSection":2,"_epic8SectionColumns":2,"_epic8ColumnIndex":0,"bbox":[40.0,440.0,230.0,450.0]}),
            json!({"blockId":"c","text":"C the third answer","pageIndex":3,"_epic8LayoutSection":2,"_epic8SectionColumns":2,"_epic8ColumnIndex":0,"bbox":[40.0,420.0,230.0,430.0]}),
        ];
        let (_, options) = dynamic_question_prompt_and_options(
            &blocks,
            "Choose the correct letter, A, B or C.",
            21,
            "Questions 21-21",
            21,
            "single_choice",
        );

        assert_eq!(options[0].1, "the source-backed answer");
        assert!(options.iter().all(|(_, text)| !text.contains("unrelated")));
    }

    #[test]
    fn shared_choice_terminal_option_retains_its_wrapped_source_continuation() {
        let blocks = vec![
            json!({"blockId":"heading","text":"Questions 21 and 22","pageIndex":4,"bbox":[54.0,750.0,160.0,760.0]}),
            json!({"blockId":"instruction","text":"Choose TWO letters, A-E.","pageIndex":4,"bbox":[54.0,720.0,190.0,730.0]}),
            json!({"blockId":"stem","text":"Which TWO statements are made in the passage?","pageIndex":4,"bbox":[54.0,680.0,400.0,690.0]}),
            json!({"blockId":"a","text":"A first answer.","pageIndex":4,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[75.0,650.0,260.0,660.0]}),
            json!({"blockId":"b","text":"B second answer.","pageIndex":4,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[75.0,630.0,260.0,640.0]}),
            json!({"blockId":"c","text":"C third answer.","pageIndex":4,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[75.0,610.0,260.0,620.0]}),
            json!({"blockId":"d","text":"D fourth answer.","pageIndex":4,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[75.0,590.0,260.0,600.0]}),
            json!({"blockId":"e","text":"E final answer affected by","pageIndex":4,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[75.0,570.0,300.0,580.0]}),
            json!({"blockId":"e-wrap","text":"the altitude.","pageIndex":4,"_epic8SectionColumns":1,"_epic8ColumnIndex":0,"bbox":[96.0,556.0,180.0,566.0]}),
            json!({"blockId":"foreign-column","text":"passage prose","pageIndex":4,"_epic8SectionColumns":2,"_epic8ColumnIndex":1,"bbox":[320.0,542.0,500.0,552.0]}),
        ];
        let candidate = serde_json::from_value::<SplitGroupCandidateV1>(json!({
            "groupId":"group-1",
            "heading":"Questions 21-22",
            "questionRange":[21,22],
            "instructionText":"Choose TWO letters, A-E.",
            "blockIds":["heading","instruction","stem","a","b","c","d","e"],
            "kindHint":"multi_choice",
            "layoutHint":"list",
            "confidence":0.78,
            "classification":null,
            "sectionEvidence":[],
            "continuationEdges":[]
        }))
        .unwrap();
        let mut groups = vec![candidate];

        extend_dynamic_choice_option_blocks(&mut groups, &blocks);

        assert!(groups[0].block_ids.contains(&"e-wrap".to_string()));
        assert!(!groups[0].block_ids.contains(&"foreign-column".to_string()));
        let owned = groups[0]
            .block_ids
            .iter()
            .filter_map(|id| {
                blocks
                    .iter()
                    .find(|block| dynamic_block_id(block) == *id)
                    .cloned()
            })
            .collect::<Vec<_>>();
        let (_, options) = dynamic_shared_choice_prompt_and_options(&owned).unwrap();
        assert_eq!(
            options.last(),
            Some(&(
                "E".to_string(),
                "final answer affected by the altitude.".to_string()
            ))
        );
    }

    #[test]
    fn question_stem_beginning_with_article_a_keeps_the_real_a_option() {
        let blocks = vec![json!({
            "blockId":"q33",
            "text":"33 A reference is made to earlier research because it A supports the new method. B explains the sample size. C contradicts the results. D supplies a definition."
        })];
        let (prompt, options) = dynamic_question_prompt_and_options(
            &blocks,
            "Choose the correct letter, A, B, C or D.",
            33,
            "Questions 33-33",
            33,
            "single_choice",
        );

        assert_eq!(prompt, "A reference is made to earlier research because it");
        assert_eq!(
            options[0],
            ("A".to_string(), "supports the new method.".to_string())
        );
        assert_eq!(
            options
                .iter()
                .map(|item| item.0.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B", "C", "D"]
        );
    }

    #[test]
    fn separate_choice_stem_beginning_with_article_a_does_not_absorb_its_bank() {
        let blocks = vec![
            json!({"blockId":"q33","text":"33 A reference is made to anger and sadness in order to show that"}),
            json!({"blockId":"q33-options","text":"A personal feelings can alter our physical condition. B some human behaviour has no clear explanation. C placebos, like emotions, are experienced by everyone. D people find some physical reactions hard to control."}),
        ];
        let (prompt, options) = dynamic_question_prompt_and_options(
            &blocks,
            "Choose the correct letter, A, B, C or D.",
            33,
            "Questions 33-33",
            33,
            "single_choice",
        );

        assert_eq!(
            prompt,
            "A reference is made to anger and sadness in order to show that"
        );
        assert_eq!(options.len(), 4);
        assert_eq!(
            options[0],
            (
                "A".to_string(),
                "personal feelings can alter our physical condition.".to_string()
            )
        );
    }

    #[test]
    fn final_matching_stem_stops_before_closed_shared_bank() {
        let blocks = vec![
            json!({"blockId":"instruction","text":"Match each person with the correct idea, A-G."}),
            json!({"blockId":"q19","text":"19 Muhammad Peter Davis"}),
            json!({"blockId":"q20","text":"20 Arthur Rosenfeld"}),
            json!({"blockId":"q21","text":"21 R van der Ley"}),
            json!({"blockId":"q22","text":"22 Amory Lovins"}),
            json!({"blockId":"bank-a","text":"A The choice of a certain construction material can have a socio-economic impact."}),
            json!({"blockId":"bank-b","text":"B Throughout the world, people are rejecting traditional housing design."}),
            json!({"blockId":"bank-c","text":"C Houses should meet physical and social needs."}),
            json!({"blockId":"bank-d","text":"D Traditional knowledge can be superior to modern knowledge."}),
            json!({"blockId":"bank-e","text":"E An innovation can save heating and cooling costs."}),
            json!({"blockId":"bank-f","text":"F Solar energy can meet village energy needs."}),
            json!({"blockId":"bank-g","text":"G A simple solution can save air-conditioning costs."}),
        ];
        let group_text = blocks
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");
        let (prompt, question_options) = dynamic_question_prompt_and_options(
            &blocks,
            &group_text,
            22,
            "Questions 19-22",
            22,
            "matching",
        );

        assert_eq!(prompt, "Amory Lovins");
        assert!(question_options.is_empty());
        let bank = dynamic_group_option_bank(&blocks, "matching");
        assert_eq!(bank.len(), 7);
        assert_eq!(
            dynamic_group_option_bank_start_index(&blocks, "matching"),
            Some(5)
        );
    }

    #[test]
    fn article_a_and_prose_letters_do_not_create_or_shadow_a_shared_bank_boundary() {
        let no_bank = vec![
            json!({"blockId":"instruction","text":"Match the statements with the researchers."}),
            json!({"blockId":"q1","text":"1 A house in Section B was compared with Building C."}),
        ];
        let no_bank_text = no_bank
            .iter()
            .map(dynamic_block_text)
            .collect::<Vec<_>>()
            .join("\n");
        let (prompt, _) = dynamic_question_prompt_and_options(
            &no_bank,
            &no_bank_text,
            1,
            "Questions 1-1",
            1,
            "matching",
        );
        assert_eq!(prompt, "A house in Section B was compared with Building C.");
        assert_eq!(
            dynamic_group_option_bank_start_index(&no_bank, "matching"),
            None
        );

        let with_bank = vec![
            json!({"blockId":"instruction","text":"Match each statement with the correct ending, A-D."}),
            json!({"blockId":"q1","text":"1 The writer refers to"}),
            json!({"blockId":"prose-a","text":"A house in Section B was compared with Building C."}),
            json!({"blockId":"bank-a","text":"A the first ending"}),
            json!({"blockId":"bank-b","text":"B the second ending"}),
            json!({"blockId":"bank-c","text":"C the third ending"}),
            json!({"blockId":"bank-d","text":"D the fourth ending"}),
        ];
        assert_eq!(
            dynamic_group_option_bank_start_index(&with_bank, "matching"),
            Some(3)
        );
    }

    #[test]
    fn declared_completion_bank_recovers_column_major_rows_and_split_word_tail() {
        let declared = dynamic_declared_letter_bank_labels(
            "Complete the summary using the list of words, A-I, below.",
        );
        assert_eq!(
            dynamic_declared_bank_parts("A more B counterintuitive C simple", &declared),
            Some(vec![
                ("A".to_string(), "more".to_string()),
                ("B".to_string(), "counterintuitive".to_string()),
                ("C".to_string(), "simple".to_string()),
            ])
        );
        assert_eq!(
            dynamic_declared_bank_parts(
                "A results D qualities G surroundings I movement",
                &declared
            ),
            Some(vec![
                ("A".to_string(), "results".to_string()),
                ("D".to_string(), "qualities".to_string()),
                ("G".to_string(), "surroundings".to_string()),
                ("I".to_string(), "movement".to_string()),
            ])
        );
    }

    #[test]
    fn completion_bank_rejoins_cross_column_split_words() {
        let blocks = vec![
            json!({"blockId":"instruction","text":"Complete the summary using the list of words and phrases A-I."}),
            json!({"blockId":"a-b-c","text":"A disruptive and unpredictable  B everyday  C loveable"}),
            json!({"blockId":"d","text":"D controversial"}),
            json!({"blockId":"g","text":"G unconfident"}),
            json!({"blockId":"e","text":"E exo","pageIndex":3,"bbox":[262.8,174.6,296.4,183.0]}),
            json!({"blockId":"h","text":"H uns","pageIndex":3,"bbox":[262.8,156.6,297.1,165.0]}),
            json!({"blockId":"e-tail-f","text":"tic and ordinary  F isolated","pageIndex":3,"bbox":[303.1,174.6,507.4,183.0]}),
            json!({"blockId":"h-tail-i","text":"urprising but satisfying  I magical","pageIndex":3,"bbox":[303.1,156.6,511.4,165.0]}),
        ];
        assert_eq!(
            dynamic_group_option_bank(&blocks, "summary_completion"),
            vec![
                ("A".to_string(), "disruptive and unpredictable".to_string()),
                ("B".to_string(), "everyday".to_string()),
                ("C".to_string(), "loveable".to_string()),
                ("D".to_string(), "controversial".to_string()),
                ("E".to_string(), "exotic and ordinary".to_string()),
                ("F".to_string(), "isolated".to_string()),
                ("G".to_string(), "unconfident".to_string()),
                ("H".to_string(), "unsurprising but satisfying".to_string()),
                ("I".to_string(), "magical".to_string()),
            ]
        );
    }

    #[test]
    fn completion_bank_rejoins_column_major_label_only_cells() {
        let blocks = vec![
            json!({"blockId":"instruction","text":"Complete the summary using the list of words and phrases A-K."}),
            json!({"blockId":"a-c","text":"A form and function B long yawns C 3 seconds","pageIndex":3,"bbox":[108.84,398.76,451.55,407.16]}),
            json!({"blockId":"d-e","text":"D fixed-action pattern E","pageIndex":3,"bbox":[108.84,369.0,256.51,377.4]}),
            json!({"blockId":"g-h","text":"G reflex H","pageIndex":3,"bbox":[108.84,339.24,256.56,347.64]}),
            json!({"blockId":"j-k","text":"J 6 seconds K","pageIndex":3,"bbox":[108.84,309.72,256.56,318.12]}),
            json!({"blockId":"e-f","text":"68 seconds F short yawns","pageIndex":3,"bbox":[276.84,369.0,460.87,377.4]}),
            json!({"blockId":"h-i","text":"sneeze I short duration","pageIndex":3,"bbox":[276.84,339.24,469.55,347.64]}),
            json!({"blockId":"k-tail","text":"half-yawns","pageIndex":3,"bbox":[276.84,309.72,328.20,318.12]}),
        ];

        assert_eq!(
            dynamic_group_option_bank(&blocks, "summary_completion"),
            vec![
                ("A".to_string(), "form and function".to_string()),
                ("B".to_string(), "long yawns".to_string()),
                ("C".to_string(), "3 seconds".to_string()),
                ("D".to_string(), "fixed-action pattern".to_string()),
                ("E".to_string(), "68 seconds".to_string()),
                ("F".to_string(), "short yawns".to_string()),
                ("G".to_string(), "reflex".to_string()),
                ("H".to_string(), "sneeze".to_string()),
                ("I".to_string(), "short duration".to_string()),
                ("J".to_string(), "6 seconds".to_string()),
                ("K".to_string(), "half-yawns".to_string()),
            ]
        );
    }

    #[test]
    fn declared_completion_bank_requires_full_source_closure_not_a_b_and_prose_i() {
        let blocks = vec![
            json!({"blockId":"instruction","text":"Complete the summary using the list of words, A-I, below."}),
            json!({"blockId":"q1","text":"1 ________ describes the result."}),
            json!({"blockId":"stimulus","text":"A study compared B cells in the sample."}),
            json!({"blockId":"passage-i","text":"I believe the later paragraph explains the remaining evidence."}),
        ];

        assert_eq!(
            dynamic_completion_letter_bank_task_end("summary_completion", &blocks),
            None
        );
        assert!(dynamic_group_option_bank(&blocks, "summary_completion").is_empty());
        let declared = dynamic_declared_letter_bank_labels(&dynamic_block_text(&blocks[0]));
        assert!(dynamic_declared_bank_parts(
            "The stimulus says A study compared B cells.",
            &declared
        )
        .is_none());
        assert!(dynamic_declared_bank_parts("D fourth B second", &declared).is_none());
    }

    #[test]
    fn declared_completion_bank_uses_late_real_a_boundary_and_stops_before_prose_i() {
        let blocks = vec![
            json!({"blockId":"instruction","text":"Complete the summary using the list of words, A-I, below."}),
            json!({"blockId":"q1","text":"1 ________ describes the result."}),
            json!({"blockId":"stimulus","text":"A study compared B cells in the sample."}),
            json!({"blockId":"bank-a-c","text":"A actual alpha B actual beta C actual gamma"}),
            json!({"blockId":"bank-d-f","text":"D actual delta E actual epsilon F actual zeta"}),
            json!({"blockId":"bank-g-i","text":"G actual eta H actual theta I actual iota"}),
            json!({"blockId":"passage-i","text":"I believe this sentence belongs to the following passage."}),
        ];

        assert_eq!(
            dynamic_completion_letter_bank_task_end("summary_completion", &blocks),
            Some(6)
        );
        let options = dynamic_group_option_bank(&blocks, "summary_completion");
        assert_eq!(options.len(), 9);
        assert_eq!(options[0], ("A".to_string(), "actual alpha".to_string()));
        assert_eq!(options[8], ("I".to_string(), "actual iota".to_string()));
    }

    #[test]
    fn source_backed_contiguous_stems_expand_an_off_by_one_heading_range() {
        let job = test_job();
        let blocks = vec![
            json!({"blockId":"heading","blockType":"header","role":"question","text":"Questions 24-26","pageIndex":6,"bbox":[54.0,753.0,138.0,762.0]}),
            json!({"blockId":"instruction","blockType":"paragraph","role":"question","text":"Do the following statements agree with the information given in Reading Passage 2? Write TRUE, FALSE or NOT GIVEN.","pageIndex":6,"bbox":[54.0,694.0,496.0,733.0]}),
            json!({"blockId":"q23","blockType":"paragraph","text":"23 The air temperature in modern Malaysian houses is lower than the air temperature outside.","pageIndex":6,"bbox":[54.0,588.0,512.0,612.0]}),
            json!({"blockId":"q24","blockType":"paragraph","text":"24 The construction industry produces substantial emissions.","pageIndex":6,"bbox":[54.0,558.0,480.0,567.0]}),
            json!({"blockId":"q25","blockType":"paragraph","text":"25 Wind towers are widespread today.","pageIndex":6,"bbox":[54.0,529.0,465.0,538.0]}),
            json!({"blockId":"q26","blockType":"paragraph","text":"26 Super-windows can be installed cheaply.","pageIndex":6,"bbox":[54.0,499.0,477.0,508.0]}),
        ];
        let doc = json!({"schemaVersion":"DocumentIRV1","pages":[{"pageIndex":6,"blocks":blocks}],"assets":[]});

        let split = make_dynamic_split_candidates(&job.job_id, &job, Some(&doc));
        let group = &split["questionGroupCandidates"][0];
        assert_eq!(group["questionRange"], json!([23, 26]));
        assert_eq!(group["heading"], json!("Questions 23-26"));
        assert!(group["classification"]["warnings"]
            .as_array()
            .unwrap()
            .contains(&json!(QUESTION_RANGE_EXPANDED_FROM_SOURCE_ANCHORS)));

        let authoring = make_dynamic_authoring_ir(&job, &split, Some(&doc));
        let group = &authoring["groups"][0];
        assert_eq!(group["questionRange"], json!([23, 26]));
        assert_eq!(group["questions"].as_array().unwrap().len(), 4);
        assert_eq!(group["questions"][0]["displayNumber"], json!("23"));
        assert_eq!(
            group["questions"][0]["prompt"],
            json!("The air temperature in modern Malaysian houses is lower than the air temperature outside.")
        );
        assert!(group["reviewWarnings"]
            .as_array()
            .unwrap()
            .contains(&json!(QUESTION_RANGE_EXPANDED_FROM_SOURCE_ANCHORS)));
    }

    #[test]
    fn passage_number_next_to_a_group_does_not_expand_its_heading_range() {
        let blocks = vec![
            json!({"blockId":"passage-23","roleHint":"passage","text":"23 houses were included in the historical survey."}),
            json!({"blockId":"q24","text":"24 The first real statement."}),
            json!({"blockId":"q25","text":"25 The second real statement."}),
            json!({"blockId":"q26","text":"26 The third real statement."}),
        ];
        let candidate = serde_json::from_value::<SplitGroupCandidateV1>(json!({
            "groupId":"group-1",
            "heading":"Questions 24-26",
            "questionRange":[24,26],
            "instructionText":"Questions 24-26",
            "blockIds":["passage-23","q24","q25","q26"],
            "kindHint":"true_false_not_given",
            "layoutHint":"list",
            "confidence":0.82,
            "classification":{
                "kind":"true_false_not_given",
                "interaction":{"type":"radio","options":["TRUE","FALSE","NOT GIVEN"],"allowOptionReuse":false},
                "confidence":0.82,
                "warnings":[],
                "evidence":["q24","q25","q26"]
            },
            "sectionEvidence":[],
            "continuationEdges":[]
        }))
        .unwrap();
        let mut groups = vec![candidate];

        normalize_dynamic_group_ranges(&mut groups, &blocks);

        assert_eq!(groups[0].question_range, [24, 26]);
        assert_eq!(groups[0].heading, "Questions 24-26");
        assert!(!groups[0]
            .classification
            .as_ref()
            .unwrap()
            .warnings
            .iter()
            .any(|warning| warning == QUESTION_RANGE_EXPANDED_FROM_SOURCE_ANCHORS));
    }
}
