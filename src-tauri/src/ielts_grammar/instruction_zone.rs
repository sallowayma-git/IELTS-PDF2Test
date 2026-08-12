use super::question_number::{
    expand_expression, parse_question_expression, question_expression_end,
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
    let lower = text.to_ascii_lowercase();
    lower.starts_with("questions ") || lower.starts_with("question ")
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
        offset = end.saturating_add(1);
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
    let items = value?.as_array()?;
    (items.len() == 4).then(|| {
        [
            items[0].as_f64().unwrap_or(0.0),
            items[1].as_f64().unwrap_or(0.0),
            items[2].as_f64().unwrap_or(0.0),
            items[3].as_f64().unwrap_or(0.0),
        ]
    })
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
}
