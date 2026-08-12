use super::{rect, Bounds};
use serde_json::Value;

#[derive(Debug, Default, Clone)]
pub(crate) struct OcrAnalysis {
    pub duplicate_text_ratio: f64,
    pub ocr_regions: Vec<Value>,
    pub warnings: Vec<String>,
}

fn overlap(a: Bounds, b: Bounds) -> f64 {
    a.iou(b)
}

pub(crate) fn analyze(page: &Value) -> OcrAnalysis {
    let glyphs = page
        .get("glyphs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut duplicate_glyphs = 0usize;
    let mut conflict = false;
    let mut duplicate_regions = Vec::new();
    for (index, left) in glyphs.iter().enumerate() {
        let Some(left_bbox) = super::bounds_of(left, "bbox") else {
            continue;
        };
        for right in glyphs.iter().skip(index + 1) {
            let Some(right_bbox) = super::bounds_of(right, "bbox") else {
                continue;
            };
            if overlap(left_bbox, right_bbox) < 0.82 {
                continue;
            }
            let left_text = left.get("text").and_then(Value::as_str).unwrap_or_default();
            let right_text = right
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            duplicate_glyphs += 1;
            let union = left_bbox.union(right_bbox);
            duplicate_regions.push(rect(
                union,
                page.get("rotation").and_then(Value::as_u64).unwrap_or(0) as u16,
            ));
            if left_text != right_text {
                conflict = true;
            }
        }
    }
    let ratio = if glyphs.is_empty() {
        0.0
    } else {
        (duplicate_glyphs as f64 * 2.0 / glyphs.len() as f64).min(1.0)
    };
    let mut warnings = Vec::new();
    let hidden_text_detected = page
        .get("_hiddenTextDetected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if ratio > 0.0 {
        warnings.push("PDF_HIDDEN_TEXT_MISALIGNED".to_string());
    }
    if conflict {
        warnings.push("PDF_NATIVE_OCR_CONFLICT".to_string());
    }
    let mut ocr_regions = page
        .get("quality")
        .and_then(|quality| quality.get("requiresOcrRegions"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if conflict {
        ocr_regions.extend(duplicate_regions);
    }
    if hidden_text_detected {
        warnings.push("PDF_HIDDEN_TEXT_MISALIGNED".to_string());
        let width = page.get("widthPt").and_then(Value::as_f64).unwrap_or(1.0);
        let height = page.get("heightPt").and_then(Value::as_f64).unwrap_or(1.0);
        ocr_regions.push(rect(
            Bounds {
                x: 0.0,
                y: 0.0,
                width: width.max(0.01),
                height: height.max(0.01),
            },
            page.get("rotation").and_then(Value::as_u64).unwrap_or(0) as u16,
        ));
    }
    OcrAnalysis {
        duplicate_text_ratio: ratio,
        ocr_regions,
        warnings,
    }
}
