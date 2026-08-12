use super::common::{
    AssetDescriptorV2, CoordinateUnitV2, QuadV2, RectV2, SourceAnchorV2, SourceFileRecordV2,
    TextStyleV2,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const DOCUMENT_IR_V2_SCHEMA_VERSION: &str = "DocumentIRV2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PointV2 {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct GlyphNodeV2 {
    pub id: String,
    pub text: String,
    pub bbox: RectV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quad: Option<QuadV2>,
    pub origin: PointV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle_rad: Option<f64>,
    pub style: TextStyleV2,
    pub unicode_map_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    pub visibility_observed: bool,
    pub unicode_map_error_observed: bool,
    pub geometry_basis: GlyphGeometryBasisV2,
    pub confidence: f64,
    pub source: GlyphSourceV2,
    pub source_anchor: SourceAnchorV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GlyphGeometryBasisV2 {
    PdfiumCharBox,
    TextMatrixDerived,
    OcrObserved,
    OoxmlLayoutDerived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum GlyphSourceV2 {
    Native,
    Ocr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SpanNodeV2 {
    pub id: String,
    pub glyph_ids: Vec<String>,
    pub text: String,
    pub bbox: RectV2,
    pub style: TextStyleV2,
    pub whitespace_before: WhitespaceOriginV2,
    pub whitespace_after: WhitespaceOriginV2,
    pub confidence: f64,
    pub source_anchors: Vec<SourceAnchorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WhitespaceOriginV2 {
    None,
    Source,
    Synthetic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LineNodeV2 {
    pub id: String,
    pub span_ids: Vec<String>,
    pub text: String,
    pub bbox: RectV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<f64>,
    pub writing_mode: WritingModeV2,
    pub indentation_pt: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hanging_indent_pt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height_pt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hard_break_after: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_gaps_pt: Vec<f64>,
    pub source_order: u32,
    pub confidence: f64,
    pub source_anchors: Vec<SourceAnchorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WritingModeV2 {
    HorizontalTb,
    VerticalRl,
    VerticalLr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalRegionKindV2 {
    Text,
    Title,
    List,
    Table,
    Figure,
    Diagram,
    Form,
    Header,
    Footer,
    PageNumber,
    Rule,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct RegionNodeV2 {
    pub id: String,
    pub kind: PhysicalRegionKindV2,
    pub bbox: RectV2,
    pub child_line_ids: Vec<String>,
    pub child_object_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading_order_rank: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading_order_alternatives: Option<Vec<Vec<String>>>,
    pub confidence: f64,
    pub source_anchors: Vec<SourceAnchorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum VectorPathCommandV2 {
    Move { x: f64, y: f64 },
    Line { x: f64, y: f64 },
    Curve { points: Vec<f64> },
    Close,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct VectorPathV2 {
    pub id: String,
    pub bbox: RectV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<VectorPathCommandV2>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_axis_aligned_rule: Option<bool>,
    pub source_anchor: SourceAnchorV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TableCellV2 {
    pub cell_id: String,
    pub row: u32,
    pub col: u32,
    pub row_span: u32,
    pub col_span: u32,
    pub bbox: RectV2,
    pub content_region_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_pt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_height_pt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_height_rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_alignment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding_pt: Option<TableCellPaddingV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_scope: Option<HeaderScopeV2>,
    pub border_evidence: Vec<String>,
    pub confidence: f64,
    pub source_anchors: Vec<SourceAnchorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TableCellPaddingV2 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HeaderScopeV2 {
    Row,
    Column,
    Both,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TableNodeV2 {
    pub id: String,
    pub bbox: RectV2,
    pub rows: u32,
    pub cols: u32,
    pub cells: Vec<TableCellV2>,
    pub detection_mode: TableDetectionModeV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption_region_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visual_fallback_asset_id: Option<String>,
    pub topology_confidence: f64,
    pub content_confidence: f64,
    pub source_anchors: Vec<SourceAnchorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TableDetectionModeV2 {
    Ooxml,
    RulingLines,
    TextAlignment,
    VisionModel,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PageQualityV2 {
    pub classification: PageClassificationV2,
    pub native_character_count: u32,
    pub unicode_error_ratio: f64,
    pub duplicate_text_ratio: f64,
    pub image_coverage_ratio: f64,
    pub text_coverage_ratio: f64,
    pub rotation_confidence: f64,
    pub requires_ocr_regions: Vec<RectV2>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PageClassificationV2 {
    BornDigital,
    Mixed,
    Scanned,
    Garbled,
    Empty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PageNodeV2 {
    pub page_index: u32,
    pub width_pt: f64,
    pub height_pt: f64,
    pub rotation: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_box: Option<RectV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop_box: Option<RectV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_transform: Option<PageTransformV2>,
    pub glyphs: Vec<GlyphNodeV2>,
    pub spans: Vec<SpanNodeV2>,
    pub lines: Vec<LineNodeV2>,
    pub regions: Vec<RegionNodeV2>,
    pub vector_paths: Vec<VectorPathV2>,
    pub tables: Vec<TableNodeV2>,
    pub asset_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_placements: Vec<PdfImagePlacementV2>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marked_content: Vec<PdfMarkedContentV2>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<PdfAnnotationV2>,
    pub reading_order: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading_order_graph: Option<ReadingOrderGraphV2>,
    pub quality: PageQualityV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ReadingOrderGraphV2 {
    pub primary: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Vec<String>>,
    pub edges: Vec<ReadingOrderEdgeV2>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cycle_edges_removed: Vec<ReadingOrderEdgeV2>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ReadingOrderEdgeV2 {
    pub from: String,
    pub to: String,
    pub relation: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PageTransformV2 {
    pub user_unit: f64,
    pub pdf_to_display: [f64; 6],
    pub display_to_normalized: [f64; 6],
    pub display_width_pt: f64,
    pub display_height_pt: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PdfImagePlacementV2 {
    pub id: String,
    pub asset_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<RectV2>,
    #[serde(rename = "nativeBBox", skip_serializing_if = "Option::is_none")]
    pub native_bbox: Option<RectV2>,
    pub object_transform: [f64; 6],
    #[serde(rename = "clipBBox", skip_serializing_if = "Option::is_none")]
    pub clip_bbox: Option<RectV2>,
    pub confidence: f64,
    pub source_anchor: SourceAnchorV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PdfMarkedContentV2 {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structure_path: Vec<String>,
    pub source_anchor: SourceAnchorV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PdfAnnotationV2 {
    pub id: String,
    pub subtype: String,
    pub bbox: RectV2,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appearance_asset_id: Option<String>,
    pub source_anchor: SourceAnchorV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CoverageDispositionV2 {
    Passage,
    Question,
    Instruction,
    Option,
    Answer,
    Explanation,
    HeaderFooter,
    Decorative,
    IgnoredWithReason,
    Unassigned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CoverageEntryV2 {
    pub source_node_id: String,
    pub disposition: CoverageDispositionV2,
    pub target_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ParserMetadataV2 {
    pub provider: String,
    pub provider_version: String,
    pub extraction_started_at: String,
    pub extraction_completed_at: String,
    pub options: BTreeMap<String, Value>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DocumentIRV2 {
    pub schema_version: String,
    pub document_id: String,
    pub job_id: String,
    pub source_files: Vec<SourceFileRecordV2>,
    pub pages: Vec<PageNodeV2>,
    pub assets: Vec<AssetDescriptorV2>,
    pub coverage_ledger: Vec<CoverageEntryV2>,
    pub parser: ParserMetadataV2,
}

impl DocumentIRV2 {
    pub fn is_supported_schema_version(&self) -> bool {
        self.schema_version == DOCUMENT_IR_V2_SCHEMA_VERSION
    }
}

#[allow(dead_code)]
fn _keep_coordinate_unit_in_schema_module(_: CoordinateUnitV2) {}
