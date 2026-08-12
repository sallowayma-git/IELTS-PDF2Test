use super::xml::XmlNode;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DocxColumn {
    pub(crate) width_twips: Option<u32>,
    pub(crate) space_twips: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DocxSection {
    pub(crate) index: u32,
    pub(crate) page_width_twips: u32,
    pub(crate) page_height_twips: u32,
    pub(crate) orientation: Option<String>,
    pub(crate) margin_top_twips: Option<u32>,
    pub(crate) margin_right_twips: Option<u32>,
    pub(crate) margin_bottom_twips: Option<u32>,
    pub(crate) margin_left_twips: Option<u32>,
    pub(crate) columns: Vec<DocxColumn>,
    pub(crate) equal_width: bool,
    pub(crate) separator: bool,
    pub(crate) header_relationship_ids: Vec<String>,
    pub(crate) footer_relationship_ids: Vec<String>,
    pub(crate) title_page: bool,
    pub(crate) break_type: Option<String>,
}

impl DocxSection {
    pub(crate) fn default_for(index: u32) -> Self {
        Self {
            index,
            page_width_twips: 12_240,
            page_height_twips: 15_840,
            columns: vec![DocxColumn::default()],
            equal_width: true,
            ..Self::default()
        }
    }

    pub(crate) fn column_count(&self) -> u32 {
        self.columns.len().max(1) as u32
    }
}

pub(crate) fn parse_section(node: &XmlNode, index: u32) -> DocxSection {
    let mut section = DocxSection::default_for(index);
    if let Some(size) = node.child("pgSz") {
        section.page_width_twips = parse_u32(size.attr("w")).unwrap_or(section.page_width_twips);
        section.page_height_twips = parse_u32(size.attr("h")).unwrap_or(section.page_height_twips);
        section.orientation = size.attr("orient").map(str::to_string);
    }
    if let Some(margin) = node.child("pgMar") {
        section.margin_top_twips = parse_u32(margin.attr("top"));
        section.margin_right_twips = parse_u32(margin.attr("right"));
        section.margin_bottom_twips = parse_u32(margin.attr("bottom"));
        section.margin_left_twips = parse_u32(margin.attr("left"));
    }
    if let Some(cols) = node.child("cols") {
        section.equal_width = cols
            .attr("equalWidth")
            .map(|value| !matches!(value, "0" | "false" | "off"))
            .unwrap_or(true);
        section.separator = cols
            .attr("sep")
            .map(|value| !matches!(value, "0" | "false" | "off"))
            .unwrap_or(false);
        let count = parse_u32(cols.attr("num")).unwrap_or(1).max(1);
        let common_space = parse_u32(cols.attr("space"));
        let explicit_columns = cols
            .children_named("col")
            .into_iter()
            .map(|column| DocxColumn {
                width_twips: parse_u32(column.attr("w")),
                space_twips: parse_u32(column.attr("space")),
            })
            .collect::<Vec<_>>();
        section.columns = if explicit_columns.is_empty() {
            (0..count)
                .map(|_| DocxColumn {
                    width_twips: None,
                    space_twips: common_space,
                })
                .collect()
        } else {
            explicit_columns
        };
    }
    section.header_relationship_ids = node
        .children_named("headerReference")
        .into_iter()
        .filter_map(|reference| reference.attr("id").map(str::to_string))
        .collect();
    section.footer_relationship_ids = node
        .children_named("footerReference")
        .into_iter()
        .filter_map(|reference| reference.attr("id").map(str::to_string))
        .collect();
    section.title_page = node.child("titlePg").is_some();
    section.break_type = node
        .child("type")
        .and_then(|node| node.attr("val"))
        .map(str::to_string);
    section
}

pub(crate) fn collect_sections(root: &XmlNode) -> Vec<DocxSection> {
    let mut result = Vec::new();
    let mut index = 0_u32;
    for node in root.descendants_named("sectPr") {
        result.push(parse_section(node, index));
        index += 1;
    }
    if result.is_empty() {
        result.push(DocxSection::default_for(0));
    }
    result
}

fn parse_u32(value: Option<&str>) -> Option<u32> {
    value.and_then(|value| value.parse::<u32>().ok())
}

#[cfg(test)]
mod tests {
    use super::{collect_sections, parse_section};
    use crate::docx_ingest::xml::parse_xml;

    #[test]
    fn parses_landscape_multi_column_section_and_references() {
        let root = parse_xml(
            br#"<w:sectPr xmlns:w="urn:w"><w:pgSz w:w="12240" w:h="15840" w:orient="landscape"/><w:pgMar w:top="720" w:left="900"/><w:cols w:num="2" w:space="360" w:sep="1"/><w:headerReference w:id="rIdH"/><w:footerReference w:id="rIdF"/></w:sectPr>"#,
            "section",
        )
        .unwrap();
        let section = parse_section(&root, 0);
        assert_eq!(section.page_width_twips, 12240);
        assert_eq!(section.page_height_twips, 15840);
        assert_eq!(section.orientation.as_deref(), Some("landscape"));
        assert_eq!(section.column_count(), 2);
        assert!(section.separator);
        assert_eq!(section.header_relationship_ids, vec!["rIdH"]);
        assert_eq!(collect_sections(&root).len(), 1);
    }
}
