use serde_json::{json, Value};

use crate::schema::common::ExtractionModeV2;

pub(crate) fn source_anchor_from_job(
    source_file_id: &str,
    source_hash: &str,
    file_type: &str,
    node_ids: Vec<String>,
    page_index: i32,
    bbox: Option<[f64; 4]>,
) -> Value {
    let extraction_mode = if file_type.eq_ignore_ascii_case("docx") {
        "docx_ooxml"
    } else if file_type.eq_ignore_ascii_case("pdf") {
        "pdf_native"
    } else {
        "manual"
    };
    let mut value = json!({
        "sourceFileId": source_file_id,
        "pageIndex": page_index,
        "nodeIds": node_ids,
        "extractionMode": extraction_mode,
        "sourceHash": source_hash
    });
    if let Some([x, y, width, height]) = bbox {
        value["bbox"] = json!({
            "x": x,
            "y": y,
            "width": width.max(0.01),
            "height": height.max(0.01),
            "unit": "pt",
            "origin": "top-left",
            "pageRotation": 0
        });
    }
    value
}

pub(crate) fn anchor_from_value(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let source_file_id = object.get("sourceFileId")?.as_str()?;
    let page_index = object.get("pageIndex")?.as_i64()? as i32;
    let node_ids = object
        .get("nodeIds")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mode = object
        .get("extractionMode")
        .and_then(Value::as_str)
        .and_then(|mode| match mode {
            "pdf_native" => Some(ExtractionModeV2::PdfNative),
            "pdf_ocr" => Some(ExtractionModeV2::PdfOcr),
            "pdf_rendered_crop" => Some(ExtractionModeV2::PdfRenderedCrop),
            "docx_ooxml" => Some(ExtractionModeV2::DocxOoxml),
            "docx_rendered_fallback" => Some(ExtractionModeV2::DocxRenderedFallback),
            "manual" => Some(ExtractionModeV2::Manual),
            _ => None,
        })?;
    let source_hash = object.get("sourceHash")?.as_str()?;
    let mut normalized = json!({
        "sourceFileId": source_file_id,
        "pageIndex": page_index,
        "nodeIds": node_ids,
        "extractionMode": serde_json::to_value(mode).ok()?,
        "sourceHash": source_hash
    });
    if let Some(bbox) = object.get("bbox") {
        normalized["bbox"] = bbox.clone();
    }
    Some(normalized)
}

pub(crate) fn plain_text_from_nodes(nodes: &[Value]) -> String {
    nodes
        .iter()
        .filter_map(|node| {
            if node.get("type").and_then(Value::as_str) == Some("text") {
                node.get("text").and_then(Value::as_str)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
