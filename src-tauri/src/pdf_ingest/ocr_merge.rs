#![cfg_attr(not(test), allow(dead_code))]

use super::Bounds;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub(crate) struct OcrProviderFacts {
    pub provider: String,
    pub provider_version: String,
    pub language: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct OcrMergeResult {
    pub glyphs: Vec<Value>,
    pub issues: Vec<Value>,
    pub matched_count: usize,
    pub additive_count: usize,
    pub conflict_count: usize,
}

fn normalized_text(value: &Value) -> String {
    value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<String>()
        .to_lowercase()
}

fn bounds(value: &Value) -> Option<Bounds> {
    super::bounds_of(value, "bbox")
}

fn variant(token: &Value, provider: &OcrProviderFacts) -> Value {
    json!({
        "text": token.get("text").and_then(Value::as_str).unwrap_or_default(),
        "extractionMode": "pdf_ocr",
        "bbox": token.get("bbox").cloned(),
        "confidence": token.get("confidence").and_then(Value::as_f64).unwrap_or(0.0),
        "provider": provider.provider,
        "providerVersion": provider.provider_version,
        "language": provider.language,
        "nodeIds": token.get("id").and_then(Value::as_str).into_iter().collect::<Vec<_>>()
    })
}

fn append_variant(native: &mut Value, candidate: Value) {
    let Some(anchor) = native
        .get_mut("sourceAnchor")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let variants = anchor
        .entry("variants".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(values) = variants.as_array_mut() {
        values.push(candidate);
    }
}

fn additive_token(token: &Value, provider: &OcrProviderFacts, additive_index: usize) -> Value {
    let mut value = token.clone();
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "id".to_string(),
            json!(format!("ocr-additive-{:05}", additive_index + 1)),
        );
        object.insert("source".to_string(), json!("ocr"));
        object.insert("geometryBasis".to_string(), json!("ocr_observed"));
        object.insert("visibilityObserved".to_string(), json!(true));
        object.insert("unicodeMapErrorObserved".to_string(), json!(true));
        object
            .entry("unicodeMapError".to_string())
            .or_insert(json!(false));
        let source_anchor = object
            .entry("sourceAnchor".to_string())
            .or_insert_with(|| json!({}));
        if let Some(anchor) = source_anchor.as_object_mut() {
            anchor.insert("extractionMode".to_string(), json!("pdf_ocr"));
            anchor.insert("variants".to_string(), json!([variant(token, provider)]));
        }
    }
    value
}

/// Consume provider output attached by an OCR adapter and merge it into the
/// page's native glyph layer before line reconstruction. The transient input
/// is always removed, including malformed input, so it can never leak into the
/// persisted DocumentIRV2 contract.
pub(crate) fn merge_provider_output(page: &mut Value) -> OcrMergeResult {
    let provider_output = page
        .as_object_mut()
        .and_then(|object| object.remove("_ocrProviderOutput"));
    let Some(provider_output) = provider_output else {
        return OcrMergeResult::default();
    };
    let provider = OcrProviderFacts {
        provider: provider_output
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("unknown-ocr-provider")
            .to_string(),
        provider_version: provider_output
            .get("providerVersion")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        language: provider_output
            .get("language")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    };
    let tokens = provider_output
        .get("tokens")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let native = page
        .get("glyphs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let result = merge_tokens(&native, &tokens, &provider);
    if let Some(object) = page.as_object_mut() {
        object.insert("glyphs".to_string(), Value::Array(result.glyphs.clone()));
    }
    result
}

/// Merge already-observed OCR tokens into native glyph facts. This function is
/// provider-independent so the no-engine Phase 2 shadow can still prove the
/// preservation policy without claiming that OCR ran.
pub(crate) fn merge_tokens(
    native: &[Value],
    ocr: &[Value],
    provider: &OcrProviderFacts,
) -> OcrMergeResult {
    let mut result = OcrMergeResult {
        glyphs: native.to_vec(),
        ..OcrMergeResult::default()
    };
    let mut claimed_native = BTreeSet::<usize>::new();

    for token in ocr {
        let Some(token_bounds) = bounds(token) else {
            continue;
        };
        let best = native
            .iter()
            .enumerate()
            .filter(|(index, _)| !claimed_native.contains(index))
            .filter_map(|(index, candidate)| {
                let overlap = bounds(candidate)?.iou(token_bounds);
                (overlap >= 0.55).then_some((index, overlap))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));

        if let Some((native_index, overlap)) = best {
            claimed_native.insert(native_index);
            result.matched_count += 1;
            let same_text = normalized_text(&native[native_index]) == normalized_text(token);
            append_variant(&mut result.glyphs[native_index], variant(token, provider));
            if !same_text {
                result.conflict_count += 1;
                result.issues.push(json!({
                    "code": "PDF_NATIVE_OCR_CONFLICT",
                    "severity": "warning",
                    "nativeNodeId": native[native_index].get("id").cloned(),
                    "ocrNodeId": token.get("id").cloned(),
                    "overlapIou": super::clean(overlap),
                    "resolution": "native_primary_ocr_variant_preserved"
                }));
            }
        } else {
            result
                .glyphs
                .push(additive_token(token, provider, result.additive_count));
            result.additive_count += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(id: &str, text: &str, x: f64) -> Value {
        json!({
            "id": id,
            "text": text,
            "bbox": {"x": x, "y": 10.0, "width": 10.0, "height": 10.0, "origin": "top-left", "pageRotation": 0},
            "confidence": 0.9,
            "source": "native",
            "sourceAnchor": {"extractionMode": "pdf_native"}
        })
    }

    fn provider() -> OcrProviderFacts {
        OcrProviderFacts {
            provider: "fixture-ocr".to_string(),
            provider_version: "1.0".to_string(),
            language: Some("eng".to_string()),
        }
    }

    #[test]
    fn same_text_keeps_native_primary_and_records_ocr_provenance() {
        let result = merge_tokens(
            &[token("native-1", "Answer", 10.0)],
            &[token("ocr-1", "answer", 10.0)],
            &provider(),
        );
        assert_eq!(result.glyphs.len(), 1);
        assert_eq!(result.glyphs[0]["id"].as_str(), Some("native-1"));
        assert_eq!(result.glyphs[0]["source"].as_str(), Some("native"));
        assert_eq!(
            result.glyphs[0]["sourceAnchor"]["variants"][0]["provider"].as_str(),
            Some("fixture-ocr")
        );
        assert_eq!(result.conflict_count, 0);
    }

    #[test]
    fn missing_native_is_additive_and_conflict_is_preserved_as_variant() {
        let result = merge_tokens(
            &[token("native-1", "cat", 10.0)],
            &[
                token("ocr-conflict", "car", 10.0),
                token("ocr-new", "dog", 80.0),
            ],
            &provider(),
        );
        assert_eq!(result.glyphs.len(), 2);
        assert_eq!(result.glyphs[0]["text"].as_str(), Some("cat"));
        assert_eq!(result.glyphs[1]["source"].as_str(), Some("ocr"));
        assert_eq!(result.conflict_count, 1);
        assert_eq!(result.additive_count, 1);
        assert_eq!(
            result.issues[0]["resolution"].as_str(),
            Some("native_primary_ocr_variant_preserved")
        );
        assert_eq!(
            result.glyphs[0]["sourceAnchor"]["variants"][0]["extractionMode"].as_str(),
            Some("pdf_ocr")
        );
        assert_eq!(
            result.glyphs[1]["sourceAnchor"]["extractionMode"].as_str(),
            Some("pdf_ocr")
        );
        assert!(result.glyphs[1]["sourceAnchor"].get("provider").is_none());
    }

    #[test]
    fn provider_output_is_consumed_and_wired_into_the_page_glyph_layer() {
        let mut page = json!({
            "glyphs": [token("native-1", "cat", 10.0)],
            "_ocrProviderOutput": {
                "provider": "fixture-ocr",
                "providerVersion": "2.0",
                "language": "eng",
                "tokens": [token("ocr-1", "car", 10.0)]
            }
        });
        let result = merge_provider_output(&mut page);
        assert_eq!(result.matched_count, 1);
        assert_eq!(result.conflict_count, 1);
        assert!(page.get("_ocrProviderOutput").is_none());
        assert_eq!(
            page["glyphs"][0]["sourceAnchor"]["variants"][0]["providerVersion"].as_str(),
            Some("2.0")
        );
    }
}
