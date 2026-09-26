use super::question_number::{
    expand_expression, parse_question_expression, question_expression_end,
    starts_with_question_heading,
};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SemanticLine {
    pub id: String,
    pub text: String,
    pub source_anchor: Value,
    pub page_index: i32,
    pub order: usize,
    pub role: String,
    pub bbox: Option<[f64; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InstructionZone {
    pub text: String,
    pub line_ids: Vec<String>,
    pub source_anchors: Vec<Value>,
    pub start_index: usize,
    pub end_index: usize,
    pub confidence: f64,
    pub warnings: Vec<String>,
}

pub(crate) fn collect_instruction_zone(
    lines: &[SemanticLine],
    heading_index: usize,
    expected_numbers: &[u32],
) -> InstructionZone {
    let mut selected = Vec::new();
    let mut warnings = Vec::new();
    let mut end_index = heading_index;
    // IELTS papers close their instruction block with a sentence that points
    // at the answer sheet (`Write your answers in boxes 7-13 on your answer
    // sheet.`).  Notes/table/flow-chart bodies start right after that
    // sentence, and those rows rarely open with a question number, so the
    // scan needs the closing sentence as a boundary signal too.
    let mut closing_instruction_seen = false;
    // 收尾指令之后的折行续行规则需要知道上一条已收进的行是不是图例行
    // （标签行或解释行）：图例自己的折行（"the writer"）以小写开头，要续收；
    // 笔记标题以大写开头，不许混进来。
    let mut last_accepted_was_legend = false;
    for (index, line) in lines.iter().enumerate().skip(heading_index) {
        let text = normalize_instruction_text(&line.text);
        if index > heading_index && is_task_boundary(&text, expected_numbers) {
            end_index = index;
            break;
        }
        if index > heading_index && is_option_run_start(&text) {
            end_index = index;
            break;
        }
        if index > heading_index && is_new_task_heading(&text) {
            end_index = index;
            break;
        }
        // A printed answer blank (`preventing 7 ____`, `8 ………`) is question
        // body, never instruction text.
        if index > heading_index && line_contains_answer_blank(&text) {
            end_index = index;
            break;
        }
        if index > heading_index
            && line_has_expected_number_before_blank(&text, expected_numbers)
        {
            end_index = index;
            break;
        }
        // After the answer-sheet sentence only the reuse hint (`NB …` /
        // `Note: …`) and the agree/disagree or option legend may follow; a
        // notes title such as `The role of capsaicin` ends the instructions.
        if index > heading_index
            && closing_instruction_seen
            && !may_follow_closing_instruction(&text, last_accepted_was_legend)
        {
            end_index = index;
            break;
        }
        if is_closing_answer_instruction(&text) {
            closing_instruction_seen = true;
        }
        // is_legend_line 系列函数期望**已小写**的输入（may_follow_closing_instruction
        // 内部也是先 to_ascii_lowercase 再调用）；这里直接传原始大小写的 text 会让
        // "YES if …" 这类大写标签行判不出图例，折行续行规则随之失效。
        last_accepted_was_legend = is_legend_line(&text.to_ascii_lowercase());
        selected.push((index, line, text));
        end_index = index + 1;
    }

    if selected.is_empty() {
        warnings.push("instruction_zone_empty".to_string());
    }
    let mut text_parts = Vec::new();
    let mut line_ids = Vec::new();
    let mut anchors = Vec::new();
    for (_, line, text) in selected {
        let part = if text.to_ascii_lowercase().contains("question") {
            trim_question_line_after_first_item(text, expected_numbers)
        } else {
            text
        };
        if !part.is_empty() {
            text_parts.push(part);
            line_ids.push(line.id.clone());
            anchors.push(line.source_anchor.clone());
        }
    }
    let confidence = if text_parts.is_empty() {
        0.0
    } else if warnings.is_empty() {
        0.92
    } else {
        0.68
    };
    InstructionZone {
        text: text_parts.join(" "),
        line_ids,
        source_anchors: anchors,
        start_index: heading_index,
        end_index,
        confidence,
        warnings,
    }
}

pub(crate) fn normalize_instruction_text(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            '\u{00a0}' | '\u{2007}' | '\u{202f}' => ' ',
            _ => ch,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn semantic_lines_from_v1_document(document: Option<&Value>) -> Vec<SemanticLine> {
    let mut lines = Vec::new();
    for (page_position, page) in document
        .and_then(|value| value.get("pages"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let page_index = page
            .get("pageIndex")
            .and_then(Value::as_i64)
            .map(|value| if value > 0 { value - 1 } else { value })
            .unwrap_or(page_position as i64) as i32;
        for (block_position, block) in page
            .get("blocks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let text = block
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| block.get("html").and_then(Value::as_str))
                .map(normalize_instruction_text)
                .unwrap_or_default();
            if text.is_empty() {
                continue;
            }
            let id = block
                .get("blockId")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("page-{}-block-{}", page_index + 1, block_position));
            lines.push(SemanticLine {
                id: id.clone(),
                text,
                source_anchor: json_source_anchor(
                    &id,
                    page_index,
                    block.get("bbox").and_then(Value::as_array),
                ),
                page_index,
                order: lines.len(),
                role: block
                    .get("roleHint")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                bbox: parse_bbox(block.get("bbox")),
            });
        }
    }
    lines
}

pub(crate) fn semantic_lines_from_v2_shadow(shadow: &Value) -> Vec<SemanticLine> {
    let mut lines = Vec::new();
    for (page_position, page) in shadow
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let page_index = page
            .get("pageIndex")
            .and_then(Value::as_i64)
            .unwrap_or(page_position as i64) as i32;
        for line in page
            .get("lines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let text = line
                .get("text")
                .and_then(Value::as_str)
                .map(normalize_instruction_text)
                .unwrap_or_default();
            if text.is_empty() {
                continue;
            }
            let id = line
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_else(|| "shadow-line")
                .to_string();
            let anchor = aggregate_line_source_anchor(line, &id, page_index);
            lines.push(SemanticLine {
                id,
                text,
                source_anchor: anchor,
                page_index,
                order: lines.len(),
                role: String::new(),
                bbox: line.get("bbox").and_then(|value| parse_bbox(Some(value))),
            });
        }
    }
    lines
}

fn aggregate_line_source_anchor(line: &Value, id: &str, page_index: i32) -> Value {
    let anchors = line
        .get("sourceAnchors")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let Some(mut aggregate) = anchors.first().cloned() else {
        return json_source_anchor(&id, page_index, None);
    };
    let mut seen = BTreeSet::new();
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
        .filter(|node_id| !node_id.trim().is_empty())
        .filter(|node_id| seen.insert((*node_id).to_string()))
        .map(|node_id| Value::String(node_id.to_string()))
        .collect::<Vec<_>>();
    if node_ids.is_empty() {
        return json_source_anchor(&id, page_index, None);
    }
    aggregate["pageIndex"] = Value::from(page_index);
    aggregate["nodeIds"] = Value::Array(node_ids);
    if let Some(bbox) = line.get("bbox") {
        aggregate["bbox"] = bbox.clone();
    }
    aggregate
}

fn is_task_boundary(text: &str, expected_numbers: &[u32]) -> bool {
    let Some(number) = parse_leading_number(text) else {
        return false;
    };
    expected_numbers.contains(&number)
}

fn is_option_run_start(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars();
    matches!(chars.next(), Some('A') | Some('a'))
        && matches!(chars.next(), Some('.') | Some(')') | Some(':') | Some(' '))
        && !chars.as_str().trim().is_empty()
}

fn is_new_task_heading(text: &str) -> bool {
    starts_with_question_heading(text)
}

/// Whether a contiguous run of blank characters starting at `start` reaches
/// `min_width` printed cells (a Unicode ellipsis glyph prints as wide as
/// three ordinary cells, matching `completion.rs`'s blank-width rule).
fn blank_run_reaches(chars: &[char], start: usize, min_width: usize) -> bool {
    let mut width = 0usize;
    let mut index = start;
    while index < chars.len() {
        let width_cell = match chars[index] {
            '…' | '⋯' => 3,
            ch if is_instruction_blank_char(ch) => 1,
            _ => break,
        };
        width += width_cell;
        index += 1;
    }
    width >= min_width
}

/// Blank shapes the papers print into question rows.  Dashes are
/// deliberately absent: prose hyphens are too common for a boundary signal.
fn is_instruction_blank_char(ch: char) -> bool {
    matches!(ch, '_' | '\u{ff3f}' | '.' | '…' | '⋯' | '□')
}

fn line_contains_answer_blank(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len()).any(|index| {
        // Printed-width thresholds per shape: `___` fills a slot, `....` a
        // dotted slot, one ellipsis glyph is sentence punctuation while two
        // print a slot, and a pair of drawn boxes is a slot.  Lone dots in
        // `e.g.` / `etc.` and one closing `…` stay prose.
        let (min_width, min_run) = match chars[index] {
            '_' | '\u{ff3f}' => (3, 3),
            '.' => (4, 4),
            '…' | '⋯' => (2, 2),
            '□' => (2, 2),
            _ => return false,
        };
        let mut run = 0;
        while index + run < chars.len() && is_instruction_blank_char(chars[index + run]) {
            run += 1;
        }
        run >= min_run && blank_run_reaches(&chars[index..index + run], 0, min_width)
    })
}

/// `preventing 7 ____` — an expected question number printed immediately in
/// front of an answer blank marks the first question row, even when the row
/// opens with a bullet.
fn line_has_expected_number_before_blank(text: &str, expected_numbers: &[u32]) -> bool {
    if expected_numbers.is_empty() {
        return false;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
        let number: String = chars[start..index].iter().collect();
        let Ok(number) = number.parse::<u32>() else {
            continue;
        };
        if !expected_numbers.contains(&number) {
            continue;
        }
        // Skip the blank field's own separators (`7. ____`, `7) ____`).
        let mut cursor = index;
        while cursor < chars.len() && chars[cursor].is_whitespace() {
            cursor += 1;
        }
        if matches!(chars.get(cursor), Some('.' | ')' | ']' | ':' | '-')) {
            cursor += 1;
            while cursor < chars.len() && chars[cursor].is_whitespace() {
                cursor += 1;
            }
        }
        if blank_run_reaches(&chars, cursor, 2) {
            return true;
        }
    }
    false
}

/// The answer-sheet sentence (`Write your answers in boxes 7-13 on your
/// answer sheet.`, `Write the correct letter, A-H, in boxes 27-31 on your
/// answer sheet.`) closes the instruction block on real IELTS papers.
fn is_closing_answer_instruction(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    (lower.contains("write your answers") && lower.contains("in boxes"))
        || lower.contains("write the correct letter")
        || (lower.contains("write") && lower.contains("answer sheet"))
}

/// Lines that legitimately continue after a closing instruction: the reuse
/// hint (`NB You may use any letter more than once.`), a wrapped tail of the
/// same sentence (`on your answer sheet.`), and the agree/disagree legend
/// (`TRUE if the statement agrees … NOT GIVEN if there is no information on
/// this`).
fn may_follow_closing_instruction(text: &str, last_accepted_was_legend: bool) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("answer sheet") {
        return true;
    }
    let trimmed = lower.trim_start();
    if trimmed.starts_with("nb ") || trimmed.starts_with("nb:") || trimmed == "nb" {
        return true;
    }
    if trimmed.starts_with("note ") || trimmed.starts_with("note:") {
        return true;
    }
    if is_legend_line(&lower) {
        return true;
    }
    // 图例解释的折行续行（"the writer"、"information on this"）：仅当上一条已
    // 收进的行是图例行、且本行以小写字母开头。以大写开头的行（笔记标题、
    // "No one knows …"）不因此被收进说明区。
    if last_accepted_was_legend
        && text
            .trim_start()
            .starts_with(|ch: char| ch.is_lowercase())
    {
        return true;
    }
    false
}

/// 收尾指令之后允许续收的"图例行"：纯标签行或解释行（含标签 + if 的行）。
/// 用于接受判定与 `last_accepted_was_legend` 状态跟踪两处，口径必须一致。
fn is_legend_line(lower: &str) -> bool {
    is_legend_label_line(lower) || is_agreement_legend_line(lower)
}

/// 只由图例标签组成的行（任意个、空格分隔）：`FALSE NOT GIVEN`、
/// `TRUE FALSE NOT GIVEN`。两栏排版的判断题图例会把两个标签挤进同一物理行
/// （fishbourne-roman-palace group-1 实测 b021）。
fn is_legend_label_line(lower: &str) -> bool {
    let trimmed = lower.trim();
    !trimmed.is_empty()
        && trimmed.split_whitespace().all(|token| {
            matches!(token, "true" | "false" | "yes" | "no" | "not" | "given")
        })
}

/// `TRUE` / `FALSE` / `NOT GIVEN` / `YES` / `NO` legend rows and their
/// explanation tails (`if the statement contradicts the information`).
///
/// 判定收紧到"长得像图例"：解释尾行（`if the statement…` / `if there is…`，含
/// `NOT GIVEN` 折行后的 `GIVEN if …`）、恰好等于一个短标签的换行行，或以短标签
/// 开头且紧跟 `if`（`NO if the statement contradicts…`）。旧的"行内出现任一标签
/// 词就算图例"会把以 `No …` 开头的笔记标题误判成图例，让它躲过收尾指令后的
/// 停止条件被吞进说明区。
fn is_agreement_legend_line(lower: &str) -> bool {
    const LEGEND_LABELS: [&str; 6] = ["true", "false", "yes", "no", "not given", "not"];
    let trimmed = lower.trim();
    if trimmed.starts_with("if the statement")
        || trimmed.starts_with("if there is")
        || trimmed.starts_with("given if ")
    {
        return true;
    }
    if LEGEND_LABELS.contains(&trimmed) {
        return true;
    }
    LEGEND_LABELS.iter().any(|label| {
        trimmed
            .strip_prefix(label)
            .is_some_and(|rest| rest.trim_start().starts_with("if "))
    })
}

fn trim_question_line_after_first_item(text: String, expected_numbers: &[u32]) -> String {
    let Some(expression) = parse_question_expression(&text) else {
        return text;
    };
    let _ = expand_expression(&expression);
    let Some(expression_end) = question_expression_end(&text) else {
        return text;
    };
    let Some(index) = find_first_question_item(&text, expression_end, expected_numbers) else {
        return text;
    };
    text[..index].trim().to_string()
}

fn find_first_question_item(text: &str, start: usize, expected_numbers: &[u32]) -> Option<usize> {
    let mut offset = start;
    while offset < text.len() {
        let relative = text[offset..].find(|ch: char| ch.is_ascii_digit())?;
        let index = offset + relative;
        let end = text[index..]
            .char_indices()
            .find(|(_, ch)| !ch.is_ascii_digit())
            .map(|(relative, _)| index + relative)
            .unwrap_or(text.len());
        if let Ok(number) = text[index..end].parse::<u32>() {
            if expected_numbers.contains(&number) {
                let after = text[end..].trim_start();
                if after.starts_with('.') || after.starts_with(')') || after.starts_with(':') {
                    return Some(index);
                }
            }
        }
        offset = text[end..]
            .chars()
            .next()
            .map(|ch| end + ch.len_utf8())
            .unwrap_or(text.len());
    }
    None
}

fn parse_leading_number(text: &str) -> Option<u32> {
    let trimmed = text.trim_start();
    let digits = trimmed
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .map(|(index, _)| index)
        .last()
        .map(|index| index + 1)?;
    trimmed[..digits].parse().ok()
}

fn parse_bbox(value: Option<&Value>) -> Option<[f64; 4]> {
    if let Some(items) = value.and_then(Value::as_array) {
        return (items.len() == 4).then(|| {
            [
                items[0].as_f64().unwrap_or(0.0),
                items[1].as_f64().unwrap_or(0.0),
                items[2].as_f64().unwrap_or(0.0),
                items[3].as_f64().unwrap_or(0.0),
            ]
        });
    }
    let object = value?.as_object()?;
    Some([
        object.get("x")?.as_f64()?,
        object.get("y")?.as_f64()?,
        object.get("width")?.as_f64()?,
        object.get("height")?.as_f64()?,
    ])
}

fn json_source_anchor(id: &str, page_index: i32, bbox: Option<&Vec<Value>>) -> Value {
    let bbox = bbox.and_then(|items| parse_bbox(Some(&Value::Array(items.clone()))));
    let bbox_value = bbox.map(|[x, y, width, height]| {
        serde_json::json!({
            "x": x,
            "y": y,
            "width": width.max(0.01),
            "height": height.max(0.01),
            "unit": "pt",
            "origin": "top-left",
            "pageRotation": 0
        })
    });
    let mut anchor = serde_json::json!({
        "sourceFileId": "unknown-source",
        "pageIndex": page_index,
        "nodeIds": [id],
        "extractionMode": "manual",
        "sourceHash": "unknown"
    });
    if let Some(bbox) = bbox_value {
        anchor["bbox"] = bbox;
    }
    anchor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, text: &str) -> SemanticLine {
        SemanticLine {
            id: id.to_string(),
            text: text.to_string(),
            source_anchor: serde_json::json!({"nodeIds":[id]}),
            page_index: 0,
            order: 0,
            role: String::new(),
            bbox: None,
        }
    }

    #[test]
    fn instruction_zone_stops_before_first_expected_question() {
        let lines = vec![
            line("h", "Questions 1-3"),
            line(
                "i",
                "Do the following statements agree? TRUE FALSE NOT GIVEN",
            ),
            line("q1", "1. The first statement"),
            line("q2", "2. The second statement"),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[1, 2, 3]);
        assert!(zone.text.contains("TRUE FALSE NOT GIVEN"));
        assert!(!zone.text.contains("first statement"));
        assert_eq!(zone.end_index, 2);
    }

    #[test]
    fn instruction_zone_stops_before_vertical_option_run() {
        let lines = vec![
            line("heading", "Questions 7-8"),
            line("instruction", "Choose the correct letter, A-D."),
            line("a", "A First option"),
            line("b", "B Second option"),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[7, 8]);
        assert!(zone.text.contains("Choose the correct letter"));
        assert!(!zone.text.contains("First option"));
    }

    #[test]
    fn heading_line_keeps_legend_but_drops_first_embedded_question() {
        let lines = vec![line(
            "h",
            "Questions 1-3 Do the following statements agree? TRUE FALSE NOT GIVEN 1. First statement",
        )];
        let zone = collect_instruction_zone(&lines, 0, &[1, 2, 3]);
        assert!(zone.text.contains("TRUE FALSE NOT GIVEN"));
        assert!(!zone.text.contains("First statement"));
    }

    #[test]
    fn instruction_scan_advances_across_multibyte_punctuation() {
        let text = "Questions 1-3 Complete the notes … example 2005 … 1. First answer";
        let index = find_first_question_item(text, "Questions 1-3".len(), &[1, 2, 3]);
        assert_eq!(index, text.find("1. First answer"));
    }

    #[test]
    fn v1_block_conversion_preserves_page_and_source_id() {
        let document = serde_json::json!({
            "pages": [{"pageIndex": 1, "blocks": [{"blockId":"b1","text":"Questions 1-2","bbox":[1.0,2.0,3.0,4.0]}]}]
        });
        let lines = semantic_lines_from_v1_document(Some(&document));
        assert_eq!(lines[0].id, "b1");
        assert_eq!(lines[0].page_index, 0);
        assert_eq!(lines[0].bbox, Some([1.0, 2.0, 3.0, 4.0]));
    }

    #[test]
    fn physical_line_anchor_aggregates_every_glyph_node() {
        let shadow = serde_json::json!({
            "pages": [{
                "pageIndex": 0,
                "lines": [{
                    "id": "p001-l0001",
                    "text": "Questions 1-2",
                    "bbox": {"x":1.0,"y":2.0,"width":30.0,"height":4.0,"unit":"pt","origin":"top-left","pageRotation":0},
                    "sourceAnchors": [
                        {"sourceFileId":"source","pageIndex":0,"nodeIds":["g1"],"extractionMode":"pdf_native","sourceHash":"hash"},
                        {"sourceFileId":"source","pageIndex":0,"nodeIds":["g2"],"extractionMode":"pdf_native","sourceHash":"hash"},
                        {"sourceFileId":"source","pageIndex":0,"nodeIds":["g2","g3"],"extractionMode":"pdf_native","sourceHash":"hash"}
                    ]
                }]
            }]
        });
        let lines = semantic_lines_from_v2_shadow(&shadow);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0].source_anchor["nodeIds"],
            serde_json::json!(["g1", "g2", "g3"])
        );
        assert_eq!(
            lines[0].source_anchor["bbox"],
            shadow["pages"][0]["lines"][0]["bbox"]
        );
    }

    #[test]
    fn physical_line_conversion_accepts_object_bboxes() {
        let shadow = serde_json::json!({
            "pages": [{
                "pageIndex": 0,
                "lines": [{
                    "id": "p001-l0001",
                    "text": "A physical line",
                    "bbox": {"x": 10.0, "y": 20.0, "width": 30.0, "height": 4.0},
                    "sourceAnchors": [{"nodeIds": ["g1"]}]
                }]
            }]
        });
        let lines = semantic_lines_from_v2_shadow(&shadow);
        assert_eq!(lines[0].bbox, Some([10.0, 20.0, 30.0, 4.0]));
    }

    #[test]
    fn instruction_zone_stops_at_a_compact_next_question_heading() {
        let lines = vec![
            line("h", "Questions 1-4"),
            line("i", "Complete the form below"),
            line("n", "Questions5-7"),
            line("c", "Choose the correct answer."),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[1, 2, 3, 4]);
        assert_eq!(zone.line_ids, vec!["h", "i"]);
        assert_eq!(zone.end_index, 2);
    }

    // Real Chili group-2 shape: the answer-sheet sentence closes the
    // instructions, then the notes title and bullets follow.  The zone must
    // end at the closing instruction instead of swallowing the notes body
    // until the next leading question number.
    #[test]
    fn instruction_zone_stops_after_closing_instruction_before_notes_body() {
        let lines = vec![
            line("h", "Questions 7-13"),
            line("i1", "Complete the notes below."),
            line("i2", "Choose ONE WORD ONLY from the passage for each answer."),
            line("close", "Write your answers in boxes 7-13 on your answer sheet."),
            line("title", "The role of capsaicin"),
            line("sub", "Chili seeds and capsaicin"),
            line("b1", "• certain birds and other animals eat chili fruit and spread the seeds"),
            line("b2", "• some animals destroy the seeds, preventing 7 __________"),
            line("b3", "8 __________"),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[7, 8, 9, 10, 11, 12, 13]);
        assert_eq!(zone.line_ids, vec!["h", "i1", "i2", "close"]);
        assert!(zone.text.contains("Write your answers in boxes 7-13 on your answer sheet."));
        assert!(!zone.text.contains("capsaicin"));
        assert!(!zone.text.contains("preventing"));
    }

    // The notes body can also start with a bullet row whose number is
    // embedded before a printed blank (`preventing 7 ____`); without a
    // closing sentence the blank itself must still end the zone.
    #[test]
    fn instruction_zone_stops_before_line_carrying_answer_blank() {
        let lines = vec![
            line("h", "Questions 7-13"),
            line("i", "Complete the notes below."),
            line("title", "Chili seeds and capsaicin"),
            line("b2", "• some animals destroy the seeds, preventing 7 __________"),
            line("b3", "8 __________"),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[7, 8, 9, 10, 11, 12, 13]);
        assert_eq!(zone.line_ids, vec!["h", "i", "title"]);
        assert!(!zone.text.contains("preventing"));
        assert!(!zone.text.contains("__________"));
    }

    // A closing instruction must not truncate legitimate continuations:
    // the TFNG legend rows and an `NB …` reuse hint stay inside the zone.
    #[test]
    fn instruction_zone_keeps_tfng_legend_and_nb_after_closing_instruction() {
        let lines = vec![
            line("h", "Questions 1-6"),
            line("i", "In boxes 1-6 on your answer sheet, write"),
            line(
                "legend",
                "TRUE if the statement agrees with the information FALSE if the statement contradicts the information NOT GIVEN if there is no information on this",
            ),
            line("nb", "NB You may use any letter more than once."),
            line("q1", "1 First statement"),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[1, 2, 3, 4, 5, 6]);
        assert!(zone.text.contains("TRUE if the statement agrees"));
        assert!(zone.text.contains("NOT GIVEN if there is no information on this"));
        assert!(zone.text.contains("NB You may use any letter more than once."));
        assert!(!zone.text.contains("First statement"));
    }

    // A notes title that merely STARTS with "No" must not be mistaken for a
    // legend row ("NO if the statement …") and swallowed into the
    // instructions after the closing sentence.
    #[test]
    fn instruction_zone_stops_before_notes_title_starting_with_no() {
        let lines = vec![
            line("h", "Questions 7-13"),
            line("i", "Complete the notes below."),
            line("close", "Write your answers in boxes 7-13 on your answer sheet."),
            line("title", "No one knows exactly when people first dried chilies"),
            line("b1", "• certain birds eat chili fruit and spread the seeds"),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[7, 8]);
        assert_eq!(zone.line_ids, vec!["h", "i", "close"]);
        assert!(!zone.text.contains("No one knows"));
        assert!(!zone.text.contains("certain birds"));
    }

    // A wrapped legend row keeps counting as legend even when "NOT GIVEN"
    // broke across physical lines (`NOT` / `GIVEN if there is …`).
    #[test]
    fn instruction_zone_keeps_wrapped_legend_rows_after_closing_instruction() {
        let lines = vec![
            line("h", "Questions 1-3"),
            line("close", "In boxes 1-3 on your answer sheet, write"),
            line("legend", "TRUE if the statement agrees with the information"),
            line("wrap", "GIVEN if there is no information on this"),
            line("q1", "1 First statement"),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[1, 2, 3]);
        assert!(zone.text.contains("TRUE if the statement agrees"));
        assert!(zone.text.contains("GIVEN if there is no information on this"));
        assert!(!zone.text.contains("First statement"));
    }

    // 真实行序列（tmp/phase4-real-pdf-acceptance/fishbourne-roman-palace/
    // split-candidates-v1.actual.json，group-1 sectionEvidence b017..b023）。
    // 两栏排版的判断题图例把 FALSE 与 NOT GIVEN 挤进同一物理行——这行只由标签
    // 组成，必须继续收进说明区，后面的解释折行（b022）不能丢。
    #[test]
    fn instruction_zone_keeps_fishbourne_two_column_tfng_legend() {
        let lines = vec![
            line("b017", "Questions 1–6"),
            line(
                "b018",
                "Do the following statements agree with the information given in Reading Passage 1?",
            ),
            line("b019", "In boxes 1–6 on your answer sheet, write"),
            line("b020", "TRUE if the statement agrees with the information"),
            line("b021", "FALSE NOT GIVEN"),
            line(
                "b022",
                "if the statement contradicts the information if there is no inform ation on this",
            ),
            line(
                "b023",
                "1 Fishbourne Palace was the first structure to be built on its site.",
            ),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[1, 2, 3, 4, 5, 6]);
        assert!(zone.text.contains("TRUE if the statement agrees"), "{}", zone.text);
        assert!(zone.text.contains("FALSE NOT GIVEN"), "{}", zone.text);
        assert!(
            zone.text.contains("if the statement contradicts the information"),
            "{}",
            zone.text
        );
        assert!(
            zone.text.contains("if there is no inform ation on this"),
            "{}",
            zone.text
        );
        assert!(!zone.text.contains("Fishbourne Palace was"), "{}", zone.text);
    }

    // YNNG 图例解释的折行续行（"the writer"）：上一条已收进的行是图例行、本行以
    // 小写字母开头 → 续收，图例的三段语义保持完整。
    #[test]
    fn instruction_zone_keeps_lowercase_legend_wrap_after_legend_row() {
        let lines = vec![
            line("h", "Questions 1-3"),
            line("close", "In boxes 1-3 on your answer sheet, write"),
            line("legend1", "YES if the statement agrees with the views of"),
            line("wrap1", "the writer"),
            line("legend2", "NO if the statement contradicts the views of"),
            line("wrap2", "the writer"),
            line("legend3", "NOT GIVEN if it is impossible to say what"),
            line("wrap3", "the writer thinks about this"),
            line("q1", "1 First statement"),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[1, 2, 3]);
        assert!(
            zone.text.contains("agrees with the views of the writer"),
            "{}",
            zone.text
        );
        assert!(
            zone.text
                .contains("impossible to say what the writer thinks about this"),
            "{}",
            zone.text
        );
        assert!(!zone.text.contains("First statement"), "{}", zone.text);
    }

    // 反例钉住折行续行的边界：上一条虽是图例行，但本行以大写开头（笔记标题），
    // 不得作为折行续行被收进说明区（f6bc641 要防的情形不回潮）。
    #[test]
    fn instruction_zone_stops_before_uppercase_line_after_legend_row() {
        let lines = vec![
            line("h", "Questions 1-3"),
            line("close", "In boxes 1-3 on your answer sheet, write"),
            line("legend", "TRUE if the statement agrees with the information"),
            line("title", "Notes on the palace"),
            line("b1", "• bullet body"),
        ];
        let zone = collect_instruction_zone(&lines, 0, &[1, 2, 3]);
        assert_eq!(zone.line_ids, vec!["h", "close", "legend"]);
        assert!(!zone.text.contains("Notes on the palace"), "{}", zone.text);
        assert!(!zone.text.contains("bullet body"), "{}", zone.text);
    }
}
