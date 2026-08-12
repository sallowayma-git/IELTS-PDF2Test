use super::{
    model::{DocxCellPadding, DocxTable, DocxTableCell, DocxTableRow},
    numbering::DocxNumberingCatalog,
    paragraphs::parse_paragraph_node,
    sections::DocxSection,
    styles::DocxStyleCatalog,
    xml::XmlNode,
};
use std::collections::BTreeMap;

pub(crate) fn parse_table_node(
    node: &XmlNode,
    path: &str,
    styles: &DocxStyleCatalog,
    numbering: &DocxNumberingCatalog,
    section: Option<DocxSection>,
    warnings: &mut Vec<String>,
) -> DocxTable {
    let grid_widths_twips = node
        .child("tblGrid")
        .map(|grid| {
            grid.children_named("gridCol")
                .into_iter()
                .filter_map(|column| column.attr("w").and_then(|value| value.parse::<u32>().ok()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let table_properties = node.child("tblPr");
    let default_padding = table_properties
        .and_then(|properties| properties.child("tblCellMar"))
        .map(parse_cell_padding)
        .unwrap_or_default();
    let default_borders = table_properties
        .and_then(|properties| properties.child("tblBorders"))
        .map(|borders| parse_border_evidence(borders, "table"))
        .unwrap_or_default();

    let mut rows = Vec::new();
    let mut vertical_merges: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for (row_index, row_node) in node.children_named("tr").into_iter().enumerate() {
        let row_path = format!("{path}/tr[{}]", row_index + 1);
        let row_properties = row_node.child("trPr");
        let grid_before = row_properties
            .and_then(|node| node.child("gridBefore"))
            .and_then(|node| node.attr("val"))
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let grid_after = row_properties
            .and_then(|node| node.child("gridAfter"))
            .and_then(|node| node.attr("val"))
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let row_height = row_properties.and_then(|properties| properties.child("trHeight"));
        let height_twips = row_height
            .and_then(|height| height.attr("val"))
            .and_then(|value| value.parse::<u32>().ok());
        let height_rule = row_height
            .and_then(|height| height.attr("hRule"))
            .map(str::to_string);
        let mut cells = Vec::new();
        let mut grid_column = grid_before;
        for (cell_index, cell_node) in row_node.children_named("tc").into_iter().enumerate() {
            let cell_path = format!("{row_path}/tc[{}]", cell_index + 1);
            let properties = cell_node.child("tcPr");
            let width_node = properties.and_then(|node| node.child("tcW"));
            let padding = properties
                .and_then(|node| node.child("tcMar"))
                .map(parse_cell_padding)
                .map(|overrides| merge_cell_padding(&default_padding, &overrides))
                .unwrap_or_else(|| default_padding.clone());
            let mut border_evidence = default_borders.clone();
            if let Some(cell_borders) = properties.and_then(|node| node.child("tcBorders")) {
                border_evidence.extend(parse_border_evidence(cell_borders, "cell"));
            }
            let grid_span = properties
                .and_then(|node| node.child("gridSpan"))
                .and_then(|node| node.attr("val"))
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(1)
                .max(1);
            let v_merge = properties
                .and_then(|node| node.child("vMerge"))
                .map(|node| node.attr("val").unwrap_or("continue").to_string());
            let mut cell = DocxTableCell {
                path: cell_path.clone(),
                grid_span,
                v_merge: v_merge.clone(),
                row_span: if v_merge.as_deref() == Some("continue") {
                    0
                } else {
                    1
                },
                width_twips: width_node
                    .and_then(|node| node.attr("w"))
                    .and_then(|value| value.parse::<u32>().ok()),
                width_type: width_node
                    .and_then(|node| node.attr("type"))
                    .map(str::to_string),
                vertical_alignment: properties
                    .and_then(|node| node.child("vAlign"))
                    .and_then(|node| node.attr("val"))
                    .map(str::to_string),
                shading: properties
                    .and_then(|node| node.child("shd"))
                    .and_then(|node| node.attr("fill"))
                    .map(str::to_string),
                padding,
                border_evidence,
                paragraphs: Vec::new(),
                tables: Vec::new(),
            };

            let mut paragraph_index = 0_usize;
            let mut table_index = 0_usize;
            for child in &cell_node.children {
                if child.is("p") {
                    paragraph_index += 1;
                    cell.paragraphs.push(parse_paragraph_node(
                        child,
                        &format!("{cell_path}/p[{paragraph_index}]"),
                        styles,
                        numbering,
                        section.clone(),
                        false,
                        warnings,
                    ));
                } else if child.is("tbl") {
                    table_index += 1;
                    cell.tables.push(parse_table_node(
                        child,
                        &format!("{cell_path}/tbl[{table_index}]"),
                        styles,
                        numbering,
                        section.clone(),
                        warnings,
                    ));
                }
            }
            if cell.paragraphs.is_empty() && cell.tables.is_empty() {
                warnings.push(format!("DOCX_EMPTY_TABLE_CELL:{}", cell_path));
            }

            if v_merge.as_deref() == Some("continue") {
                let previous_cell = (0..grid_span)
                    .find_map(|offset| vertical_merges.get(&(grid_column + offset)).copied());
                if let Some((previous_row, previous_cell)) = previous_cell {
                    if let Some(previous) = rows
                        .get_mut(previous_row)
                        .and_then(|row: &mut DocxTableRow| row.cells.get_mut(previous_cell))
                    {
                        previous.row_span = previous.row_span.saturating_add(1);
                    }
                } else {
                    warnings.push(format!(
                        "TABLE_TOPOLOGY_AMBIGUOUS:{}:vMerge continue without restart",
                        cell_path
                    ));
                }
            } else if v_merge.as_deref() == Some("restart") {
                for offset in 0..grid_span {
                    vertical_merges.insert(grid_column + offset, (row_index, cell_index));
                }
            } else {
                for offset in 0..grid_span {
                    vertical_merges.remove(&(grid_column + offset));
                }
            }
            grid_column = grid_column.saturating_add(grid_span);
            cells.push(cell);
        }
        rows.push(DocxTableRow {
            path: row_path,
            cells,
            grid_before,
            grid_after,
            height_twips,
            height_rule,
        });
    }

    DocxTable {
        path: path.to_string(),
        grid_widths_twips,
        rows,
        nested: Vec::new(),
        source_order: 0,
    }
}

fn parse_cell_padding(node: &XmlNode) -> DocxCellPadding {
    let side = |names: &[&str]| {
        names.iter().find_map(|name| {
            node.child(name)
                .and_then(|value| value.attr("w"))
                .and_then(|value| value.parse::<u32>().ok())
        })
    };
    DocxCellPadding {
        top_twips: side(&["top"]),
        right_twips: side(&["end", "right"]),
        bottom_twips: side(&["bottom"]),
        left_twips: side(&["start", "left"]),
    }
}

fn merge_cell_padding(defaults: &DocxCellPadding, overrides: &DocxCellPadding) -> DocxCellPadding {
    DocxCellPadding {
        top_twips: overrides.top_twips.or(defaults.top_twips),
        right_twips: overrides.right_twips.or(defaults.right_twips),
        bottom_twips: overrides.bottom_twips.or(defaults.bottom_twips),
        left_twips: overrides.left_twips.or(defaults.left_twips),
    }
}

fn parse_border_evidence(node: &XmlNode, source: &str) -> Vec<String> {
    node.children
        .iter()
        .filter(|border| {
            matches!(
                border
                    .name
                    .rsplit_once(':')
                    .map(|(_, name)| name)
                    .unwrap_or(border.name.as_str()),
                "top" | "start" | "left" | "bottom" | "end" | "right" | "insideH" | "insideV"
            )
        })
        .map(|border| {
            let edge = border
                .name
                .rsplit_once(':')
                .map(|(_, name)| name)
                .unwrap_or(&border.name);
            let style = border.attr("val").unwrap_or("unspecified");
            let size = border.attr("sz").unwrap_or("unspecified");
            let color = border.attr("color").unwrap_or("auto");
            let space = border.attr("space").unwrap_or("0");
            format!(
                "ooxml-{source}-border:{edge}:style={style}:size-eighth-pt={size}:color={color}:space-pt={space}"
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_table_node;
    use crate::docx_ingest::{
        numbering::DocxNumberingCatalog, sections::DocxSection, styles::DocxStyleCatalog,
        xml::parse_xml,
    };

    #[test]
    fn preserves_empty_cells_spans_merges_and_nested_tables() {
        let root = parse_xml(
            br#"<w:tbl xmlns:w="urn:w">
                <w:tblPr><w:tblCellMar><w:top w:w="80" w:type="dxa"/><w:start w:w="100" w:type="dxa"/><w:bottom w:w="120" w:type="dxa"/><w:end w:w="140" w:type="dxa"/></w:tblCellMar><w:tblBorders><w:top w:val="single" w:sz="8" w:color="000000"/></w:tblBorders></w:tblPr>
                <w:tblGrid><w:gridCol w:w="720"/><w:gridCol w:w="1440"/></w:tblGrid>
                <w:tr><w:trPr><w:trHeight w:val="640" w:hRule="atLeast"/></w:trPr><w:tc><w:tcPr><w:tcW w:w="2160" w:type="dxa"/><w:gridSpan w:val="2"/><w:vMerge w:val="restart"/><w:vAlign w:val="center"/><w:tcMar><w:start w:w="160" w:type="dxa"/></w:tcMar><w:tcBorders><w:bottom w:val="double" w:sz="12" w:color="FF0000" w:space="1"/></w:tcBorders></w:tcPr><w:p><w:r><w:t>top</w:t></w:r></w:p></w:tc></w:tr>
                <w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr>
                <w:tr><w:tc><w:p/></w:tc><w:tc><w:p/><w:tbl><w:tr><w:tc><w:p><w:r><w:t>nested</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:tc></w:tr>
            </w:tbl>"#,
            "table",
        )
        .unwrap();
        let mut warnings = Vec::new();
        let table = parse_table_node(
            &root,
            "/word/document.xml/body/tbl[1]",
            &DocxStyleCatalog::default(),
            &DocxNumberingCatalog::default(),
            Some(DocxSection::default_for(0)),
            &mut warnings,
        );
        assert_eq!(table.grid_widths_twips, vec![720, 1440]);
        assert_eq!(table.rows[0].height_twips, Some(640));
        assert_eq!(table.rows[0].height_rule.as_deref(), Some("atLeast"));
        assert_eq!(table.rows[0].cells[0].grid_span, 2);
        assert_eq!(table.rows[0].cells[0].row_span, 2);
        assert_eq!(table.rows[0].cells[0].width_twips, Some(2160));
        assert_eq!(table.rows[0].cells[0].width_type.as_deref(), Some("dxa"));
        assert_eq!(
            table.rows[0].cells[0].vertical_alignment.as_deref(),
            Some("center")
        );
        assert_eq!(table.rows[0].cells[0].padding.top_twips, Some(80));
        assert_eq!(table.rows[0].cells[0].padding.left_twips, Some(160));
        assert_eq!(table.rows[0].cells[0].padding.bottom_twips, Some(120));
        assert_eq!(table.rows[0].cells[0].padding.right_twips, Some(140));
        assert!(table.rows[0].cells[0]
            .border_evidence
            .iter()
            .any(|item| item.contains("table-border:top:style=single")));
        assert!(table.rows[0].cells[0]
            .border_evidence
            .iter()
            .any(|item| item.contains("cell-border:bottom:style=double")));
        assert_eq!(table.rows[1].cells[0].row_span, 0);
        assert_eq!(
            table.rows[2].cells[1].tables[0].rows[0].cells[0].paragraphs[0].raw_text(),
            "nested"
        );
        assert!(warnings.iter().all(|warning| !warning.contains("TOPOLOGY")));
    }
}
