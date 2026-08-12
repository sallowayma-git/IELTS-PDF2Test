use super::common::SourceAnchorV2;
use serde::{Deserialize, Serialize};

pub const CONTENT_DOC_V2_SCHEMA_VERSION: &str = "ContentDocV2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceStatusV2 {
    Source,
    Derived,
    UserEdited,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BaseContentNodeV2 {
    pub id: String,
    pub source_anchors: Vec<SourceAnchorV2>,
    pub provenance_status: ProvenanceStatusV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum TextMarkV2 {
    Simple(String),
    Link { link: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContentDisplayV2 {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width_px: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<ContentAlignV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ContentAlignV2 {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DiagramHotspotV2 {
    pub hotspot_id: String,
    pub slot_id: String,
    pub normalized_rect: [f64; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_anchor: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub children: Vec<ContentNodeV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub children: Vec<ContentNodeV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<ParagraphAlignV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_level: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ParagraphAlignV2 {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeadingNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub level: u8,
    pub children: Vec<ContentNodeV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marks: Option<Vec<TextMarkV2>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HardBreakNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BulletListNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub items: Vec<ListItemNodeV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrderedListNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub items: Vec<ListItemNodeV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListItemNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub children: Vec<ContentNodeV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TableContentNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub rows: Vec<TableRowContentNodeV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<Vec<ContentNodeV2>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_table_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visual_fallback_asset_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TableRowContentNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub cells: Vec<TableCellContentNodeV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TableCellContentNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub row_span: u32,
    pub col_span: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_scope: Option<HeaderScopeV2>,
    pub children: Vec<ContentNodeV2>,
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
pub struct FigureNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub asset_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<Vec<ContentNodeV2>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotspots: Option<Vec<DiagramHotspotV2>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f64; 4]>,
    pub display: ContentDisplayV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub asset_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f64; 4]>,
    pub display: ContentDisplayV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FigcaptionNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub children: Vec<ContentNodeV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowchartNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub steps: Vec<FlowStepNodeV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ContentDisplayV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowStepNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub children: Vec<ContentNodeV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagramNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub asset_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotspots: Option<Vec<DiagramHotspotV2>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f64; 4]>,
    pub display: ContentDisplayV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnswerSlotNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub slot_id: String,
    pub display_label: String,
    pub inline: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OptionContentNodeV2 {
    pub option_id: String,
    pub label: String,
    pub children: Vec<ContentNodeV2>,
    pub source_anchors: Vec<SourceAnchorV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OptionBankNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
    pub option_bank_id: String,
    pub options: Vec<OptionContentNodeV2>,
    pub allow_reuse: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HorizontalRuleNodeV2 {
    #[serde(flatten)]
    pub base: BaseContentNodeV2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ContentNodeV2 {
    #[serde(rename = "doc")]
    Doc(DocNodeV2),
    #[serde(rename = "paragraph")]
    Paragraph(ParagraphNodeV2),
    #[serde(rename = "heading")]
    Heading(HeadingNodeV2),
    #[serde(rename = "text")]
    Text(TextNodeV2),
    #[serde(rename = "hard_break")]
    HardBreak(HardBreakNodeV2),
    #[serde(rename = "bullet_list")]
    BulletList(BulletListNodeV2),
    #[serde(rename = "ordered_list")]
    OrderedList(OrderedListNodeV2),
    #[serde(rename = "list_item")]
    ListItem(ListItemNodeV2),
    #[serde(rename = "table")]
    Table(TableContentNodeV2),
    #[serde(rename = "table_row")]
    TableRow(TableRowContentNodeV2),
    #[serde(rename = "table_cell")]
    TableCell(TableCellContentNodeV2),
    #[serde(rename = "figure")]
    Figure(FigureNodeV2),
    #[serde(rename = "image")]
    Image(ImageNodeV2),
    #[serde(rename = "figcaption")]
    Figcaption(FigcaptionNodeV2),
    #[serde(rename = "flowchart")]
    Flowchart(FlowchartNodeV2),
    #[serde(rename = "flow_step")]
    FlowStep(FlowStepNodeV2),
    #[serde(rename = "diagram")]
    Diagram(DiagramNodeV2),
    #[serde(rename = "answer_slot")]
    AnswerSlot(AnswerSlotNodeV2),
    #[serde(rename = "option_bank")]
    OptionBank(OptionBankNodeV2),
    #[serde(rename = "horizontal_rule")]
    HorizontalRule(HorizontalRuleNodeV2),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ContentDocV2 {
    pub schema_version: String,
    pub document_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_document_id: Option<String>,
    pub root: Vec<ContentNodeV2>,
}

impl ContentDocV2 {
    pub fn is_supported_schema_version(&self) -> bool {
        self.schema_version == CONTENT_DOC_V2_SCHEMA_VERSION
    }
}
