use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionModeV2 {
    PdfNative,
    PdfOcr,
    PdfRenderedCrop,
    DocxOoxml,
    DocxRenderedFallback,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct RectV2 {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub unit: CoordinateUnitV2,
    pub origin: CoordinateOriginV2,
    pub page_rotation: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized: Option<[f64; 4]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CoordinateUnitV2 {
    Pt,
    Emu,
    Px,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CoordinateOriginV2 {
    #[serde(rename = "top-left")]
    TopLeft,
    #[serde(rename = "bottom-left")]
    BottomLeft,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QuadV2 {
    pub points: [f64; 8],
    pub unit: CoordinateUnitV2,
    pub origin: CoordinateOriginV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SourceCharRangeV2 {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SourceAnchorV2 {
    pub source_file_id: String,
    pub page_index: i32,
    pub node_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<RectV2>,
    #[serde(rename = "nativeBBox", skip_serializing_if = "Option::is_none")]
    pub native_bbox: Option<RectV2>,
    #[serde(rename = "displayBBox", skip_serializing_if = "Option::is_none")]
    pub display_bbox: Option<RectV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_to_display: Option<[f64; 6]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub char_range: Option<SourceCharRangeV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ooxml_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship_id: Option<String>,
    pub extraction_mode: ExtractionModeV2,
    pub source_hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<SourceVariantV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SourceVariantV2 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub extraction_mode: ExtractionModeV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<RectV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceFileRoleV2 {
    QuestionPaper,
    AnswerKey,
    Explanation,
    Supplement,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SourceFileRecordV2 {
    pub source_file_id: String,
    pub original_name: String,
    pub media_type: String,
    pub sha256: String,
    pub byte_length: u64,
    pub role: SourceFileRoleV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TextStyleV2 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size_pt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superscript: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscript: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AssetKindV2 {
    RasterImage,
    VectorRender,
    PageCrop,
    Diagram,
    Chart,
    Audio,
    Thumbnail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AssetExtractionModeV2 {
    Embedded,
    PageCrop,
    RenderedVector,
    DocxMedia,
    UserUpload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AssetDescriptorV2 {
    pub asset_id: String,
    pub kind: AssetKindV2,
    pub mime: String,
    pub relative_path: String,
    pub sha256: String,
    pub byte_length: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_px: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_px: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub extraction_mode: AssetExtractionModeV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decorative: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_anchor: Option<SourceAnchorV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagram_question_region: Option<DiagramQuestionRegionV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DiagramQuestionRegionV2 {
    pub question_range: [u32; 2],
    pub expected_numbers: Vec<u32>,
    pub recovery_status: DiagramQuestionRecoveryStatusV2,
    pub number_closure: bool,
    pub source_backed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DiagramQuestionRecoveryStatusV2 {
    OcrRequired,
    Recovered,
}

pub type JsonObjectV2 = BTreeMap<String, Value>;

/// Encode contract JSON with recursively sorted object keys.
///
/// Arrays retain their semantic order; only object key order is normalized.
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&canonicalize_json(value))
}

pub fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut canonical = serde_json::Map::new();
            for key in keys {
                if let Some(child) = object.get(key) {
                    canonical.insert(key.clone(), canonicalize_json(child));
                }
            }
            Value::Object(canonical)
        }
        _ => value.clone(),
    }
}

/// Encode contract JSON the way the *student runtime* re-encodes it.
///
/// The published `ReadingExamSourceV2` carries a `runtimeSha256` that the
/// student loader recomputes from the parsed payload using JavaScript's
/// `JSON.stringify` (`NasJsDirectReadingAssetProvider.canonicalJson`). Rust and
/// JavaScript agree on key order and string escaping, but not on numbers:
/// `serde_json` writes `1.0` / `60.0` where JavaScript writes `1` / `60`, and
/// they disagree on exponent form too. A producer that hashes the Rust form
/// therefore publishes a package the student rejects with
/// `reading_source_integrity_failed`, even though the payload is identical.
///
/// This serializer is the ECMAScript-compatible canonical form and is the
/// only one that may be used for cross-repo runtime checksums.
pub fn canonical_json_bytes_js(value: &Value) -> Vec<u8> {
    let mut out = String::new();
    write_js_json(&canonicalize_json(value), &mut out);
    out.into_bytes()
}

fn write_js_json(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => out.push_str(&js_number_to_string(number)),
        // serde_json's string escaping already matches JSON.stringify:
        // short escapes for \b \t \n \f \r, \u00xx for the remaining control
        // characters, and raw UTF-8 for everything else.
        Value::String(text) => match serde_json::to_string(text) {
            Ok(escaped) => out.push_str(&escaped),
            Err(_) => out.push_str("\"\""),
        },
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_js_json(item, out);
            }
            out.push(']');
        }
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                match serde_json::to_string(key) {
                    Ok(escaped) => out.push_str(&escaped),
                    Err(_) => out.push_str("\"\""),
                }
                out.push(':');
                if let Some(child) = object.get(*key) {
                    write_js_json(child, out);
                }
            }
            out.push('}');
        }
    }
}

/// JSON 只有一种数字类型（IEEE-754 double），`JSON.stringify` 输出的是**double**
/// 的最短往返表示。Rust 侧 `serde_json` 会把整数字面量保存成精确的 i64/u64，
/// 直接输出精确位数就会与 JS 不一致——例如 `9007199254740993` 在 JS 里先被舍入成
/// `9007199254740992`。因此只有 `|v| <= 2^53`（double 可精确表示的整数范围）才走
/// 精确输出，其余一律按 double 语义输出。
const JS_MAX_EXACT_INTEGER: u64 = 1u64 << 53;

fn js_number_to_string(number: &serde_json::Number) -> String {
    if let Some(value) = number.as_i64() {
        return if value.unsigned_abs() <= JS_MAX_EXACT_INTEGER {
            value.to_string()
        } else {
            js_f64_to_string(value as f64)
        };
    }
    if let Some(value) = number.as_u64() {
        return if value <= JS_MAX_EXACT_INTEGER {
            value.to_string()
        } else {
            js_f64_to_string(value as f64)
        };
    }
    match number.as_f64() {
        Some(value) => js_f64_to_string(value),
        None => number.to_string(),
    }
}

/// ECMAScript `Number::toString` for finite doubles (JSON.stringify semantics).
fn js_f64_to_string(value: f64) -> String {
    if !value.is_finite() {
        // JSON.stringify(NaN) === JSON.stringify(Infinity) === "null".
        return "null".to_string();
    }
    if value == 0.0 {
        // Covers -0.0 as well: JSON.stringify(-0) === "0".
        return "0".to_string();
    }
    let negative = value.is_sign_negative();
    // Rust's Display for f64 is the shortest round-tripping decimal and never
    // uses exponent notation, which keeps the digit parsing below trivial.
    let plain = format!("{}", value.abs());
    let (digits, exponent) = shortest_decimal_parts(&plain);
    let k = digits.len() as i32;
    let mut text = String::new();
    if negative {
        text.push('-');
    }
    if k <= exponent && exponent <= 21 {
        text.push_str(&digits);
        for _ in 0..(exponent - k) {
            text.push('0');
        }
    } else if 0 < exponent && exponent <= 21 {
        text.push_str(&digits[..exponent as usize]);
        text.push('.');
        text.push_str(&digits[exponent as usize..]);
    } else if -6 < exponent && exponent <= 0 {
        text.push_str("0.");
        for _ in 0..(-exponent) {
            text.push('0');
        }
        text.push_str(&digits);
    } else {
        let exponent_value = exponent - 1;
        if k == 1 {
            text.push_str(&digits);
        } else {
            text.push_str(&digits[..1]);
            text.push('.');
            text.push_str(&digits[1..]);
        }
        text.push('e');
        text.push(if exponent_value >= 0 { '+' } else { '-' });
        text.push_str(&exponent_value.abs().to_string());
    }
    text
}

/// Split a plain decimal string (`"0.00120"`) into shortest significant digits
/// and the ECMAScript exponent `n` such that `value = digits * 10^(n - len)`.
fn shortest_decimal_parts(plain: &str) -> (String, i32) {
    let (integer_part, fraction_part) = match plain.split_once('.') {
        Some((integer, fraction)) => (integer, fraction),
        None => (plain, ""),
    };
    let all_digits = format!("{integer_part}{fraction_part}");
    let point_position = integer_part.len() as i32;
    let trimmed_leading = all_digits.trim_start_matches('0');
    let digits = trimmed_leading.trim_end_matches('0');
    let trailing_zeros = (trimmed_leading.len() - digits.len()) as i32;
    let exponent = digits.len() as i32 + point_position - all_digits.len() as i32 + trailing_zeros;
    (digits.to_string(), exponent)
}

#[cfg(test)]
mod js_canonical_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integral_floats_lose_the_trailing_zero_like_javascript() {
        // These are exactly the values that made a real published package fail
        // the student's runtime checksum: Rust wrote 1.0/60.0/80.0 and the
        // student recomputed over 1/60/80.
        let value =
            json!({"confidence": 1.0, "display": {"widthPercent": 60.0, "maxWidthPx": 80.0}});
        assert_eq!(
            String::from_utf8(canonical_json_bytes_js(&value)).unwrap(),
            r#"{"confidence":1,"display":{"maxWidthPx":80,"widthPercent":60}}"#
        );
    }

    #[test]
    fn number_formatting_matches_ecmascript() {
        assert_eq!(js_f64_to_string(0.0), "0");
        assert_eq!(js_f64_to_string(-0.0), "0");
        assert_eq!(js_f64_to_string(1.0), "1");
        assert_eq!(js_f64_to_string(-2.5), "-2.5");
        assert_eq!(js_f64_to_string(0.12), "0.12");
        assert_eq!(js_f64_to_string(0.0001), "0.0001");
        assert_eq!(js_f64_to_string(0.000001), "0.000001");
        assert_eq!(js_f64_to_string(0.0000001), "1e-7");
        assert_eq!(js_f64_to_string(1e21), "1e+21");
        assert_eq!(js_f64_to_string(123.456), "123.456");
    }

    #[test]
    fn key_order_and_strings_match_javascript() {
        let value = json!({"b": 1, "a": "x\ny\u{1}", "c": [1, true, null]});
        assert_eq!(
            String::from_utf8(canonical_json_bytes_js(&value)).unwrap(),
            "{\"a\":\"x\\ny\\u0001\",\"b\":1,\"c\":[1,true,null]}"
        );
    }

    /// JSON 只有 double 一种数字类型，`JSON.stringify` 输出的是 double 的最短往返表示。
    /// 超过 2^53 的整数在 JS 侧会先被舍入，如果这里直接输出精确位数，发布端算出的
    /// `runtimeSha256` 就与学生端重算的哈希不一致，学生端会以
    /// `reading_source_integrity_failed` 拒绝整包。
    /// 期望值取自真实 Node：`JSON.stringify(JSON.parse(<字面量>))`。
    #[test]
    fn large_integers_are_rounded_like_javascript() {
        for (source, expected) in [
            ("9007199254740992", "9007199254740992"), // 2^53：double 可精确表示
            ("9007199254740993", "9007199254740992"), // 2^53+1：JS 会舍入
            ("-9007199254740993", "-9007199254740992"),
            ("18446744073709551615", "18446744073709552000"), // u64::MAX
            ("123456789012345678901234567890", "1.2345678901234568e+29"),
        ] {
            let value: Value = serde_json::from_str(source).unwrap();
            assert_eq!(
                String::from_utf8(canonical_json_bytes_js(&value)).unwrap(),
                expected,
                "canonical JSON for {source} must match JSON.stringify"
            );
        }
    }
}
