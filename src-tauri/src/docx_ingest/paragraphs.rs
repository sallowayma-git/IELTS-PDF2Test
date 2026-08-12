use super::{
    model::{
        DocxBlock, DocxDocumentModel, DocxParagraph, DocxParagraphFormatting, DocxRun, DocxRunKind,
    },
    numbering::DocxNumberingCatalog,
    sections::{collect_sections, DocxSection},
    styles::{parse_paragraph_formatting, parse_run_formatting, DocxStyleCatalog},
    tables::parse_table_node,
    xml::{local_name_of, XmlNode},
};
use std::collections::BTreeMap;

pub(crate) fn parse_document_model(
    root: &XmlNode,
    main_document_part: &str,
    styles: &DocxStyleCatalog,
    numbering: &DocxNumberingCatalog,
) -> DocxDocumentModel {
    let mut model = DocxDocumentModel {
        main_document_part: main_document_part.to_string(),
        sections: collect_sections(root),
        ..DocxDocumentModel::default()
    };
    let Some(body) = root.child("body") else {
        model.issues.push(super::model::DocxIssue::error(
            "DOCX_DOCUMENT_BODY_MISSING",
            "word/document.xml does not contain w:body",
            Some(format!("/{main_document_part}")),
        ));
        return model;
    };

    let mut paragraph_index = 0_usize;
    let mut table_index = 0_usize;
    let mut section_index = 0_usize;
    for (body_index, child) in body.children.iter().enumerate() {
        if child.is("p") {
            paragraph_index += 1;
            let section = model.sections.get(section_index).cloned();
            let path = format!("/{main_document_part}/body/p[{paragraph_index}]");
            let mut paragraph = parse_paragraph_node(
                child,
                &path,
                styles,
                numbering,
                section,
                false,
                &mut model.warnings,
            );
            paragraph.source_order = body_index;
            let has_section_break = child
                .child("pPr")
                .and_then(|ppr| ppr.child("sectPr"))
                .is_some();
            model.blocks.push(DocxBlock::Paragraph(paragraph));
            if has_section_break {
                section_index = section_index.saturating_add(1);
            }
        } else if child.is("tbl") {
            table_index += 1;
            let section = model.sections.get(section_index).cloned();
            let path = format!("/{main_document_part}/body/tbl[{table_index}]");
            let mut table = parse_table_node(
                child,
                &path,
                styles,
                numbering,
                section,
                &mut model.warnings,
            );
            table.source_order = body_index;
            model.blocks.push(DocxBlock::Table(table));
        }
    }
    model
}

pub(crate) fn parse_paragraph_node(
    node: &XmlNode,
    path: &str,
    styles: &DocxStyleCatalog,
    numbering: &DocxNumberingCatalog,
    section: Option<DocxSection>,
    in_text_box: bool,
    warnings: &mut Vec<String>,
) -> DocxParagraph {
    let ppr = node.child("pPr");
    let style_id = ppr
        .and_then(|node| node.child("pStyle"))
        .and_then(|node| node.attr("val"))
        .map(str::to_string);
    let direct_numbering_id = ppr
        .and_then(|node| node.child("numPr"))
        .and_then(|node| node.child("numId"))
        .and_then(|node| node.attr("val"))
        .map(str::to_string);
    let direct_numbering_level = ppr
        .and_then(|node| node.child("numPr"))
        .and_then(|node| node.child("ilvl"))
        .and_then(|node| node.attr("val"))
        .and_then(|value| value.parse::<u32>().ok());
    let direct_formatting = ppr.map(parse_paragraph_formatting).unwrap_or_default();
    let direct_paragraph_run = ppr
        .and_then(|node| node.child("rPr"))
        .map(parse_run_formatting)
        .unwrap_or_default();
    let resolved_style = styles.resolve_paragraph(
        style_id.as_deref(),
        &direct_formatting,
        &direct_paragraph_run,
    );
    let numbering_id = direct_numbering_id.or_else(|| resolved_style.numbering_id.clone());
    let numbering_level = direct_numbering_level.or(resolved_style.numbering_level);
    let resolved_numbering = numbering.resolve(numbering_id.as_deref(), numbering_level);
    let mut paragraph = DocxParagraph {
        path: path.to_string(),
        style_id,
        numbering_id,
        numbering_level,
        direct_formatting,
        resolved_style: Some(resolved_style.clone()),
        resolved_numbering,
        runs: Vec::new(),
        section,
        in_text_box,
        source_order: 0,
        numbering_label: None,
    };

    let mut counters = BTreeMap::<String, usize>::new();
    for child in &node.children {
        let local_name = local_name_of(&child.name).to_string();
        let occurrence = next_occurrence(&mut counters, &local_name);
        let child_path = format!("{path}/{local_name}[{occurrence}]");
        match local_name.as_str() {
            "r" => parse_run_node(
                child,
                &child_path,
                false,
                &[],
                styles,
                &resolved_style.run,
                &mut paragraph.runs,
            ),
            "hyperlink" => {
                let mut relationships = Vec::new();
                if let Some(id) = child.attr("id") {
                    relationships.push(id.to_string());
                }
                parse_run_container(
                    child,
                    &child_path,
                    false,
                    &relationships,
                    styles,
                    &resolved_style.run,
                    &mut paragraph.runs,
                );
            }
            "ins" => parse_run_container(
                child,
                &child_path,
                true,
                &[],
                styles,
                &resolved_style.run,
                &mut paragraph.runs,
            ),
            "del" => warnings.push(format!("DOCX_TRACKED_DELETION_IGNORED:{}", child_path)),
            "fldSimple" => {
                if let Some(instruction) = child.attr("instr") {
                    paragraph.runs.push(DocxRun {
                        path: format!("{child_path}/@instr"),
                        kind: DocxRunKind::Field,
                        text: String::new(),
                        resolved_formatting: resolved_style.run.clone(),
                        field_instruction: Some(instruction.to_string()),
                        break_type: Some("instruction".to_string()),
                        ..DocxRun::default()
                    });
                }
                parse_run_container(
                    child,
                    &child_path,
                    false,
                    &[],
                    styles,
                    &resolved_style.run,
                    &mut paragraph.runs,
                );
            }
            _ => {}
        }
    }
    paragraph
}

fn parse_run_container(
    container: &XmlNode,
    path: &str,
    inserted: bool,
    inherited_relationships: &[String],
    styles: &DocxStyleCatalog,
    paragraph_run_formatting: &super::model::DocxRunFormatting,
    output: &mut Vec<DocxRun>,
) {
    let mut counters = BTreeMap::<String, usize>::new();
    for child in &container.children {
        if !child.is("r") {
            if child.is("hyperlink") || child.is("fldSimple") {
                let occurrence = next_occurrence(&mut counters, local_name_of(&child.name));
                let child_path = format!(
                    "{path}/{name}[{occurrence}]",
                    name = local_name_of(&child.name)
                );
                parse_run_container(
                    child,
                    &child_path,
                    inserted,
                    inherited_relationships,
                    styles,
                    paragraph_run_formatting,
                    output,
                );
            }
            continue;
        }
        let occurrence = next_occurrence(&mut counters, "r");
        parse_run_node(
            child,
            &format!("{path}/r[{occurrence}]"),
            inserted,
            inherited_relationships,
            styles,
            paragraph_run_formatting,
            output,
        );
    }
}

fn parse_run_node(
    node: &XmlNode,
    path: &str,
    inserted: bool,
    inherited_relationships: &[String],
    styles: &DocxStyleCatalog,
    paragraph_run_formatting: &super::model::DocxRunFormatting,
    output: &mut Vec<DocxRun>,
) {
    let run_properties = node.child("rPr");
    let style_id = run_properties
        .and_then(|node| node.child("rStyle"))
        .and_then(|node| node.attr("val"))
        .map(str::to_string);
    let direct_formatting = run_properties.map(parse_run_formatting).unwrap_or_default();
    let resolved_formatting = styles.resolve_run(
        paragraph_run_formatting,
        style_id.as_deref(),
        &direct_formatting,
    );
    let mut counters = BTreeMap::<String, usize>::new();
    for child in &node.children {
        let local_name = local_name_of(&child.name).to_string();
        if local_name == "rPr" {
            continue;
        }
        let occurrence = next_occurrence(&mut counters, &local_name);
        let child_path = format!("{path}/{local_name}[{occurrence}]");
        let (kind, text, break_type, xml_space_preserve, field_instruction) =
            match local_name.as_str() {
                "t" => (
                    DocxRunKind::Text,
                    child.text_content(),
                    None,
                    child.attr("space").is_some_and(|value| value == "preserve"),
                    None,
                ),
                "tab" => (
                    DocxRunKind::Tab,
                    "\t".to_string(),
                    Some("tab".to_string()),
                    false,
                    None,
                ),
                "br" => {
                    let break_type = child.attr("type").unwrap_or("textWrapping").to_string();
                    (
                        DocxRunKind::Break,
                        "\n".to_string(),
                        Some(break_type),
                        false,
                        None,
                    )
                }
                "cr" => (
                    DocxRunKind::Break,
                    "\n".to_string(),
                    Some("line".to_string()),
                    false,
                    None,
                ),
                "noBreakHyphen" => (
                    DocxRunKind::Text,
                    "\u{2011}".to_string(),
                    Some("noBreakHyphen".to_string()),
                    false,
                    None,
                ),
                "softHyphen" => (
                    DocxRunKind::Text,
                    "\u{00ad}".to_string(),
                    Some("softHyphen".to_string()),
                    false,
                    None,
                ),
                "fldChar" => (
                    DocxRunKind::Field,
                    String::new(),
                    child.attr("fldCharType").map(str::to_string),
                    false,
                    None,
                ),
                "instrText" => (
                    DocxRunKind::Field,
                    String::new(),
                    Some("instruction".to_string()),
                    child.attr("space").is_some_and(|value| value == "preserve"),
                    Some(child.text_content()),
                ),
                "drawing" => (
                    DocxRunKind::Drawing,
                    String::new(),
                    Some("drawing".to_string()),
                    false,
                    None,
                ),
                "pict" | "object" => (
                    DocxRunKind::Drawing,
                    String::new(),
                    Some(local_name.clone()),
                    false,
                    None,
                ),
                "bookmarkStart" | "bookmarkEnd" => (
                    DocxRunKind::Bookmark,
                    String::new(),
                    child.attr("id").map(str::to_string),
                    false,
                    None,
                ),
                _ => continue,
            };
        let mut relationship_ids = inherited_relationships.to_vec();
        for relationship_node in child
            .descendants_named("blip")
            .into_iter()
            .chain(child.descendants_named("imagedata"))
        {
            for attribute in ["embed", "link", "id"] {
                if let Some(value) = relationship_node.attr(attribute) {
                    if !relationship_ids.iter().any(|item| item == value) {
                        relationship_ids.push(value.to_string());
                    }
                }
            }
        }
        output.push(DocxRun {
            path: child_path,
            kind,
            text,
            style_id: style_id.clone(),
            direct_formatting: direct_formatting.clone(),
            resolved_formatting: resolved_formatting.clone(),
            relationship_ids,
            deleted: false,
            inserted,
            break_type,
            xml_space_preserve,
            field_instruction,
        });
    }
}

fn next_occurrence(counters: &mut BTreeMap<String, usize>, name: &str) -> usize {
    let entry = counters.entry(name.to_string()).or_insert(0);
    *entry += 1;
    *entry
}

#[allow(dead_code)]
fn _paragraph_formatting_is_used(_formatting: &DocxParagraphFormatting) {}

#[cfg(test)]
mod tests {
    use super::{parse_document_model, parse_paragraph_node};
    use crate::docx_ingest::{
        numbering::DocxNumberingCatalog, sections::DocxSection, styles::DocxStyleCatalog,
        xml::parse_xml,
    };

    #[test]
    fn preserves_tabs_breaks_fields_spaces_insertions_and_deletion_warning() {
        let root = parse_xml(
            br#"<w:p xmlns:w="urn:w">
                <w:pPr><w:pStyle w:val="Heading"/><w:numPr><w:ilvl w:val="1"/><w:numId w:val="7"/></w:numPr></w:pPr>
                <w:r><w:t xml:space="preserve"> A </w:t><w:tab/><w:br w:type="page"/><w:noBreakHyphen/></w:r>
                <w:hyperlink w:id="rId5"><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r></w:hyperlink>
                <w:ins><w:r><w:t>new</w:t></w:r></w:ins>
                <w:del><w:r><w:delText>old</w:delText></w:r></w:del>
            </w:p>"#,
            "paragraph",
        )
        .unwrap();
        let mut warnings = Vec::new();
        let paragraph = parse_paragraph_node(
            &root,
            "/word/document.xml/body/p[1]",
            &DocxStyleCatalog::default(),
            &DocxNumberingCatalog::default(),
            Some(DocxSection::default_for(0)),
            false,
            &mut warnings,
        );
        assert_eq!(paragraph.raw_text(), " A \t\n‑new");
        assert!(paragraph
            .runs
            .iter()
            .any(|run| run.field_instruction.as_deref() == Some(" PAGE ") && run.text.is_empty()));
        assert!(paragraph.runs.iter().any(|run| run.xml_space_preserve));
        assert!(paragraph
            .runs
            .iter()
            .any(|run| run.break_type.as_deref() == Some("page")));
        assert!(paragraph.runs.iter().any(|run| run.inserted));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("TRACKED_DELETION")));
    }

    #[test]
    fn parses_document_blocks_and_section_metadata() {
        let root = parse_xml(
            br#"<w:document xmlns:w="urn:w"><w:body><w:p><w:r><w:t>one</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>two</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:sectPr><w:cols w:num="2"/></w:sectPr></w:body></w:document>"#,
            "document",
        )
        .unwrap();
        let model = parse_document_model(
            &root,
            "word/document.xml",
            &DocxStyleCatalog::default(),
            &DocxNumberingCatalog::default(),
        );
        assert_eq!(model.blocks.len(), 2);
        assert_eq!(model.sections[0].column_count(), 2);
    }
}
