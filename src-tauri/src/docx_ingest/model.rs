use super::sections::DocxSection;
use super::{numbering::DocxNumberingResolved, styles::DocxResolvedParagraphStyle};

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxRun {
    pub(crate) path: String,
    pub(crate) kind: DocxRunKind,
    pub(crate) text: String,
    pub(crate) style_id: Option<String>,
    pub(crate) direct_formatting: DocxRunFormatting,
    pub(crate) resolved_formatting: DocxRunFormatting,
    pub(crate) relationship_ids: Vec<String>,
    pub(crate) deleted: bool,
    pub(crate) inserted: bool,
    pub(crate) break_type: Option<String>,
    pub(crate) xml_space_preserve: bool,
    pub(crate) field_instruction: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum DocxRunKind {
    #[default]
    Text,
    Tab,
    Break,
    Field,
    Drawing,
    Bookmark,
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxRunFormatting {
    pub(crate) bold: Option<bool>,
    pub(crate) italic: Option<bool>,
    pub(crate) underline: Option<String>,
    pub(crate) strike: Option<bool>,
    pub(crate) font_size_half_points: Option<u32>,
    pub(crate) font_name: Option<String>,
    pub(crate) color: Option<String>,
    pub(crate) language: Option<String>,
    pub(crate) vertical_align: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxParagraph {
    pub(crate) path: String,
    pub(crate) style_id: Option<String>,
    pub(crate) numbering_id: Option<String>,
    pub(crate) numbering_level: Option<u32>,
    pub(crate) direct_formatting: DocxParagraphFormatting,
    pub(crate) resolved_style: Option<DocxResolvedParagraphStyle>,
    pub(crate) resolved_numbering: Option<DocxNumberingResolved>,
    pub(crate) runs: Vec<DocxRun>,
    pub(crate) section: Option<DocxSection>,
    pub(crate) in_text_box: bool,
    pub(crate) source_order: usize,
    pub(crate) numbering_label: Option<String>,
}

impl DocxParagraph {
    pub(crate) fn raw_text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }

    pub(crate) fn has_visible_text(&self) -> bool {
        self.numbering_label.is_some()
            || self
                .runs
                .iter()
                .any(|run| !run.text.is_empty() || run.kind == DocxRunKind::Drawing)
    }

    pub(crate) fn display_text(&self) -> String {
        let mut value = String::new();
        if let Some(label) = self.numbering_label.as_deref() {
            value.push_str(label);
            value.push('\t');
        }
        value.push_str(&self.raw_text());
        value
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxParagraphFormatting {
    pub(crate) alignment: Option<String>,
    pub(crate) spacing_before_twips: Option<i32>,
    pub(crate) spacing_after_twips: Option<i32>,
    pub(crate) line_twips: Option<i32>,
    pub(crate) left_indent_twips: Option<i32>,
    pub(crate) right_indent_twips: Option<i32>,
    pub(crate) first_line_indent_twips: Option<i32>,
    pub(crate) hanging_indent_twips: Option<i32>,
    pub(crate) keep_next: Option<bool>,
    pub(crate) keep_lines: Option<bool>,
    pub(crate) page_break_before: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxTable {
    pub(crate) path: String,
    pub(crate) grid_widths_twips: Vec<u32>,
    pub(crate) rows: Vec<DocxTableRow>,
    pub(crate) nested: Vec<DocxTable>,
    pub(crate) source_order: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxTableRow {
    pub(crate) path: String,
    pub(crate) cells: Vec<DocxTableCell>,
    pub(crate) grid_before: u32,
    pub(crate) grid_after: u32,
    pub(crate) height_twips: Option<u32>,
    pub(crate) height_rule: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DocxCellPadding {
    pub(crate) top_twips: Option<u32>,
    pub(crate) right_twips: Option<u32>,
    pub(crate) bottom_twips: Option<u32>,
    pub(crate) left_twips: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxTableCell {
    pub(crate) path: String,
    pub(crate) grid_span: u32,
    pub(crate) v_merge: Option<String>,
    pub(crate) row_span: u32,
    pub(crate) width_twips: Option<u32>,
    pub(crate) width_type: Option<String>,
    pub(crate) vertical_alignment: Option<String>,
    pub(crate) shading: Option<String>,
    pub(crate) padding: DocxCellPadding,
    pub(crate) border_evidence: Vec<String>,
    pub(crate) paragraphs: Vec<DocxParagraph>,
    pub(crate) tables: Vec<DocxTable>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxDrawing {
    pub(crate) path: String,
    pub(crate) relationship_id: Option<String>,
    pub(crate) relationship_ids: Vec<String>,
    pub(crate) relationship_target: Option<String>,
    pub(crate) relationship_targets: Vec<String>,
    pub(crate) external_relationship_ids: Vec<String>,
    pub(crate) external: bool,
    pub(crate) width_emu: Option<i64>,
    pub(crate) height_emu: Option<i64>,
    pub(crate) x_emu: Option<i64>,
    pub(crate) y_emu: Option<i64>,
    pub(crate) floating: bool,
    pub(crate) relative_height: Option<i64>,
    pub(crate) wrap: Option<String>,
    pub(crate) alt_text: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) rotation: Option<i64>,
    pub(crate) crop: Option<[f64; 4]>,
    pub(crate) source_paragraph_path: Option<String>,
    pub(crate) source_kind: DocxDrawingKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum DocxDrawingKind {
    #[default]
    Image,
    Shape,
    Chart,
    SmartArt,
    Vml,
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxCompositeDrawing {
    pub(crate) path: String,
    pub(crate) kind: DocxDrawingKind,
    pub(crate) relationship_ids: Vec<String>,
    pub(crate) text: String,
    pub(crate) preview_relationship_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxTextBox {
    pub(crate) path: String,
    pub(crate) paragraphs: Vec<DocxParagraph>,
    pub(crate) floating: bool,
    pub(crate) x_emu: Option<i64>,
    pub(crate) y_emu: Option<i64>,
    pub(crate) source_paragraph_path: Option<String>,
    pub(crate) width_emu: Option<i64>,
    pub(crate) height_emu: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxDocumentModel {
    pub(crate) main_document_part: String,
    pub(crate) blocks: Vec<DocxBlock>,
    pub(crate) drawings: Vec<DocxDrawing>,
    pub(crate) composites: Vec<DocxCompositeDrawing>,
    pub(crate) text_boxes: Vec<DocxTextBox>,
    pub(crate) sections: Vec<DocxSection>,
    pub(crate) warnings: Vec<String>,
    pub(crate) issues: Vec<DocxIssue>,
}

#[derive(Debug, Clone)]
pub(crate) enum DocxBlock {
    Paragraph(DocxParagraph),
    Table(DocxTable),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocxIssue {
    pub(crate) code: String,
    pub(crate) severity: DocxIssueSeverity,
    pub(crate) message: String,
    pub(crate) path: Option<String>,
    pub(crate) relationship_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DocxIssueSeverity {
    Warning,
    Error,
}

impl DocxIssue {
    pub(crate) fn warning(code: &str, message: impl Into<String>, path: Option<String>) -> Self {
        Self {
            code: code.to_string(),
            severity: DocxIssueSeverity::Warning,
            message: message.into(),
            path,
            relationship_id: None,
        }
    }

    pub(crate) fn error(code: &str, message: impl Into<String>, path: Option<String>) -> Self {
        Self {
            code: code.to_string(),
            severity: DocxIssueSeverity::Error,
            message: message.into(),
            path,
            relationship_id: None,
        }
    }
}
