use super::{
    model::{DocxParagraphFormatting, DocxRunFormatting},
    xml::{parse_xml, XmlNode},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxStyleDefinition {
    pub(crate) style_id: String,
    pub(crate) style_type: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) based_on: Option<String>,
    pub(crate) paragraph: DocxParagraphFormatting,
    pub(crate) run: DocxRunFormatting,
    pub(crate) outline_level: Option<u32>,
    pub(crate) numbering_id: Option<String>,
    pub(crate) numbering_level: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxResolvedParagraphStyle {
    pub(crate) style_id: Option<String>,
    pub(crate) style_name: Option<String>,
    pub(crate) based_on_style_id: Option<String>,
    pub(crate) paragraph: DocxParagraphFormatting,
    pub(crate) run: DocxRunFormatting,
    pub(crate) outline_level: Option<u32>,
    pub(crate) numbering_id: Option<String>,
    pub(crate) numbering_level: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxStyleCatalog {
    pub(crate) document_defaults: DocxRunFormatting,
    pub(crate) paragraph_defaults: DocxParagraphFormatting,
    pub(crate) styles: BTreeMap<String, DocxStyleDefinition>,
}

impl DocxStyleCatalog {
    pub(crate) fn parse(styles_xml: &[u8]) -> Result<Self, String> {
        let root = parse_xml(styles_xml, "word/styles.xml")?;
        let mut catalog = Self::default();
        if let Some(defaults) = root.child("docDefaults") {
            if let Some(paragraph_properties) = defaults
                .child("pPrDefault")
                .and_then(|node| node.child("pPr"))
            {
                catalog.paragraph_defaults = parse_paragraph_formatting(paragraph_properties);
            }
            if let Some(run_properties) = defaults
                .child("rPrDefault")
                .and_then(|node| node.child("rPr"))
            {
                catalog.document_defaults = parse_run_formatting(run_properties);
            }
        }
        for style in root.children_named("style") {
            let style_id = style.attr("styleId").unwrap_or_default().trim();
            if style_id.is_empty() {
                continue;
            }
            let paragraph_node = style.child("pPr");
            let run_node = style.child("rPr");
            let definition = DocxStyleDefinition {
                style_id: style_id.to_string(),
                style_type: style.attr("type").map(str::to_string),
                name: style
                    .child("name")
                    .and_then(|node| node.attr("val"))
                    .map(str::to_string),
                based_on: style
                    .child("basedOn")
                    .and_then(|node| node.attr("val"))
                    .map(str::to_string),
                paragraph: paragraph_node
                    .map(parse_paragraph_formatting)
                    .unwrap_or_default(),
                run: run_node.map(parse_run_formatting).unwrap_or_default(),
                outline_level: paragraph_node
                    .and_then(|node| node.child("outlineLvl"))
                    .and_then(|node| node.attr("val"))
                    .and_then(|value| value.parse::<u32>().ok()),
                numbering_id: paragraph_node
                    .and_then(|node| node.child("numPr"))
                    .and_then(|node| node.child("numId"))
                    .and_then(|node| node.attr("val"))
                    .map(str::to_string),
                numbering_level: paragraph_node
                    .and_then(|node| node.child("numPr"))
                    .and_then(|node| node.child("ilvl"))
                    .and_then(|node| node.attr("val"))
                    .and_then(|value| value.parse::<u32>().ok()),
            };
            catalog.styles.insert(style_id.to_string(), definition);
        }
        Ok(catalog)
    }

    pub(crate) fn resolve_paragraph(
        &self,
        style_id: Option<&str>,
        direct_paragraph: &DocxParagraphFormatting,
        direct_run: &DocxRunFormatting,
    ) -> DocxResolvedParagraphStyle {
        let mut resolved = DocxResolvedParagraphStyle {
            style_id: style_id.map(str::to_string),
            paragraph: self.paragraph_defaults.clone(),
            run: self.document_defaults.clone(),
            ..DocxResolvedParagraphStyle::default()
        };
        let mut chain = Vec::<&DocxStyleDefinition>::new();
        let mut current = style_id;
        let mut seen = Vec::<&str>::new();
        for _ in 0..32 {
            let Some(style_key) = current else { break };
            if seen.contains(&style_key) {
                break;
            }
            seen.push(style_key);
            let Some(style) = self.styles.get(style_key) else {
                break;
            };
            chain.push(style);
            current = style.based_on.as_deref();
        }
        for style in chain.iter().rev() {
            merge_paragraph_formatting(&mut resolved.paragraph, &style.paragraph);
            merge_run_formatting(&mut resolved.run, &style.run);
            resolved.style_name = style.name.clone().or(resolved.style_name);
            resolved.based_on_style_id = style.based_on.clone().or(resolved.based_on_style_id);
            resolved.outline_level = style.outline_level.or(resolved.outline_level);
            resolved.numbering_id = style.numbering_id.clone().or(resolved.numbering_id);
            resolved.numbering_level = style.numbering_level.or(resolved.numbering_level);
        }
        merge_paragraph_formatting(&mut resolved.paragraph, direct_paragraph);
        merge_run_formatting(&mut resolved.run, direct_run);
        resolved
    }

    pub(crate) fn style(&self, style_id: &str) -> Option<&DocxStyleDefinition> {
        self.styles.get(style_id)
    }

    pub(crate) fn resolve_run(
        &self,
        paragraph_run: &DocxRunFormatting,
        style_id: Option<&str>,
        direct_run: &DocxRunFormatting,
    ) -> DocxRunFormatting {
        let mut resolved = paragraph_run.clone();
        if let Some(style_id) = style_id {
            let mut chain = Vec::new();
            let mut current = Some(style_id);
            let mut seen = Vec::new();
            for _ in 0..32 {
                let Some(style_key) = current else { break };
                if seen.contains(&style_key) {
                    break;
                }
                seen.push(style_key);
                let Some(style) = self.styles.get(style_key) else {
                    break;
                };
                chain.push(style);
                current = style.based_on.as_deref();
            }
            for style in chain.iter().rev() {
                merge_run_formatting(&mut resolved, &style.run);
            }
        }
        merge_run_formatting(&mut resolved, direct_run);
        resolved
    }
}

pub(crate) fn parse_paragraph_formatting(node: &XmlNode) -> DocxParagraphFormatting {
    let mut formatting = DocxParagraphFormatting::default();
    if let Some(jc) = node.child("jc").and_then(|child| child.attr("val")) {
        formatting.alignment = Some(jc.to_string());
    }
    if let Some(spacing) = node.child("spacing") {
        formatting.spacing_before_twips = parse_i32(spacing.attr("before"));
        formatting.spacing_after_twips = parse_i32(spacing.attr("after"));
        formatting.line_twips = parse_i32(spacing.attr("line"));
    }
    if let Some(ind) = node.child("ind") {
        formatting.left_indent_twips = parse_i32(ind.attr("left"));
        formatting.right_indent_twips = parse_i32(ind.attr("right"));
        formatting.first_line_indent_twips = parse_i32(ind.attr("firstLine"));
        formatting.hanging_indent_twips = parse_i32(ind.attr("hanging"));
    }
    formatting.keep_next = node.child("keepNext").map(on_value);
    formatting.keep_lines = node.child("keepLines").map(on_value);
    formatting.page_break_before = node.child("pageBreakBefore").map(on_value);
    formatting
}

pub(crate) fn parse_run_formatting(node: &XmlNode) -> DocxRunFormatting {
    let mut formatting = DocxRunFormatting::default();
    formatting.bold = child_bool(node, "b");
    formatting.italic = child_bool(node, "i");
    formatting.strike = child_bool(node, "strike");
    formatting.underline = node
        .child("u")
        .map(|child| child.attr("val").unwrap_or("single").to_string());
    formatting.font_size_half_points = node
        .child("sz")
        .and_then(|child| child.attr("val"))
        .and_then(|value| value.parse().ok());
    formatting.font_name = node.child("rFonts").and_then(|child| {
        ["ascii", "hAnsi", "eastAsia", "cs"]
            .iter()
            .find_map(|name| child.attr(name))
            .map(str::to_string)
    });
    formatting.color = node
        .child("color")
        .and_then(|child| child.attr("val"))
        .map(str::to_string);
    formatting.language = node
        .child("lang")
        .and_then(|child| child.attr("val"))
        .map(str::to_string);
    formatting.vertical_align = node
        .child("vertAlign")
        .and_then(|child| child.attr("val"))
        .map(str::to_string);
    formatting
}

fn merge_paragraph_formatting(
    target: &mut DocxParagraphFormatting,
    source: &DocxParagraphFormatting,
) {
    if source.alignment.is_some() {
        target.alignment = source.alignment.clone();
    }
    if source.spacing_before_twips.is_some() {
        target.spacing_before_twips = source.spacing_before_twips;
    }
    if source.spacing_after_twips.is_some() {
        target.spacing_after_twips = source.spacing_after_twips;
    }
    if source.line_twips.is_some() {
        target.line_twips = source.line_twips;
    }
    if source.left_indent_twips.is_some() {
        target.left_indent_twips = source.left_indent_twips;
    }
    if source.right_indent_twips.is_some() {
        target.right_indent_twips = source.right_indent_twips;
    }
    if source.first_line_indent_twips.is_some() {
        target.first_line_indent_twips = source.first_line_indent_twips;
    }
    if source.hanging_indent_twips.is_some() {
        target.hanging_indent_twips = source.hanging_indent_twips;
    }
    if source.keep_next.is_some() {
        target.keep_next = source.keep_next;
    }
    if source.keep_lines.is_some() {
        target.keep_lines = source.keep_lines;
    }
    if source.page_break_before.is_some() {
        target.page_break_before = source.page_break_before;
    }
}

fn merge_run_formatting(target: &mut DocxRunFormatting, source: &DocxRunFormatting) {
    if source.bold.is_some() {
        target.bold = source.bold;
    }
    if source.italic.is_some() {
        target.italic = source.italic;
    }
    if source.underline.is_some() {
        target.underline = source.underline.clone();
    }
    if source.strike.is_some() {
        target.strike = source.strike;
    }
    if source.font_size_half_points.is_some() {
        target.font_size_half_points = source.font_size_half_points;
    }
    if source.font_name.is_some() {
        target.font_name = source.font_name.clone();
    }
    if source.color.is_some() {
        target.color = source.color.clone();
    }
    if source.language.is_some() {
        target.language = source.language.clone();
    }
    if source.vertical_align.is_some() {
        target.vertical_align = source.vertical_align.clone();
    }
}

pub(crate) fn overlay_run_formatting(
    base: &DocxRunFormatting,
    source: &DocxRunFormatting,
) -> DocxRunFormatting {
    let mut result = base.clone();
    merge_run_formatting(&mut result, source);
    result
}

fn child_bool(node: &XmlNode, name: &str) -> Option<bool> {
    node.child(name).map(on_value)
}

fn on_value(node: &XmlNode) -> bool {
    !matches!(
        node.attr("val")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "off" | "none" | "no"
    )
}

fn parse_i32(value: Option<&str>) -> Option<i32> {
    value.and_then(|value| value.parse::<i32>().ok())
}

#[cfg(test)]
mod tests {
    use super::DocxStyleCatalog;

    #[test]
    fn resolves_defaults_based_on_style_and_direct_formatting() {
        let catalog = DocxStyleCatalog::parse(
            br#"<w:styles xmlns:w="urn:w">
                <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Aptos"/><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults>
                <w:style w:type="paragraph" w:styleId="Base"><w:name w:val="Base"/><w:pPr><w:spacing w:after="120"/><w:numPr><w:ilvl w:val="2"/><w:numId w:val="7"/></w:numPr></w:pPr><w:rPr><w:b/></w:rPr></w:style>
                <w:style w:type="paragraph" w:styleId="Heading"><w:basedOn w:val="Base"/><w:name w:val="Heading 2"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style>
            </w:styles>"#,
        )
        .unwrap();
        let resolved =
            catalog.resolve_paragraph(Some("Heading"), &Default::default(), &Default::default());
        assert_eq!(resolved.style_name.as_deref(), Some("Heading 2"));
        assert_eq!(resolved.paragraph.spacing_after_twips, Some(120));
        assert_eq!(resolved.run.bold, Some(true));
        assert_eq!(resolved.run.font_size_half_points, Some(22));
        assert_eq!(resolved.outline_level, Some(1));
        assert_eq!(resolved.numbering_id.as_deref(), Some("7"));
        assert_eq!(resolved.numbering_level, Some(2));
    }
}
