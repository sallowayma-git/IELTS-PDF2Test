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
