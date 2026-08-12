use super::{
    model::{DocxDocumentModel, DocxIssue, DocxTextBox},
    numbering::DocxNumberingCatalog,
    paragraphs::parse_paragraph_node,
    styles::DocxStyleCatalog,
    xml::{local_name_of, XmlNode},
};
use std::collections::BTreeMap;

pub(crate) fn collect_text_boxes(
    root: &XmlNode,
    styles: &DocxStyleCatalog,
    numbering: &DocxNumberingCatalog,
    model: &mut DocxDocumentModel,
) {
    walk_node(
        root,
        &format!("/{}", model.main_document_part),
        None,
        false,
        None,
        None,
        None,
        None,
        styles,
        numbering,
        model,
    );
}

fn walk_node(
    node: &XmlNode,
    path: &str,
    source_paragraph_path: Option<&str>,
    inherited_floating: bool,
    inherited_x_emu: Option<i64>,
    inherited_y_emu: Option<i64>,
    inherited_width_emu: Option<i64>,
    inherited_height_emu: Option<i64>,
    styles: &DocxStyleCatalog,
    numbering: &DocxNumberingCatalog,
    model: &mut DocxDocumentModel,
) {
    let source_paragraph_path = if node.is("p") {
        Some(path.to_string())
    } else {
        source_paragraph_path.map(str::to_string)
    };
    let floating = inherited_floating || node.is("anchor") || node.is("shape");
    let mut x_emu = inherited_x_emu;
    let mut y_emu = inherited_y_emu;
    let mut width_emu = inherited_width_emu;
    let mut height_emu = inherited_height_emu;
    if node.is("anchor") {
        x_emu = node
            .child("positionH")
            .and_then(|position| position.child("posOffset"))
            .and_then(|offset| offset.text_content().trim().parse::<i64>().ok());
        y_emu = node
            .child("positionV")
            .and_then(|position| position.child("posOffset"))
            .and_then(|offset| offset.text_content().trim().parse::<i64>().ok());
    } else if node.is("shape") {
        let (style_x, style_y, style_width, style_height) = parse_vml_geometry(node.attr("style"));
        x_emu = x_emu.or(style_x);
        y_emu = y_emu.or(style_y);
        width_emu = width_emu.or(style_width);
        height_emu = height_emu.or(style_height);
    }
    if node.is("txbxContent") {
        let text_box_path = path.to_string();
        let mut paragraphs = Vec::new();
        let section = model.sections.first().cloned();
        let mut paragraph_index = 0_usize;
        for child in &node.children {
            if child.is("p") {
                paragraph_index += 1;
                paragraphs.push(parse_paragraph_node(
                    child,
                    &format!("{text_box_path}/p[{paragraph_index}]"),
                    styles,
                    numbering,
                    section.clone(),
                    true,
                    &mut model.warnings,
                ));
            }
        }
        if floating && (x_emu.is_none() || y_emu.is_none()) {
            model.issues.push(DocxIssue::warning(
                "DOCX_FLOATING_ORDER_AMBIGUOUS",
                "floating text box has no complete anchor coordinates; OOXML order is retained",
                Some(text_box_path.clone()),
            ));
        }
        model.text_boxes.push(DocxTextBox {
            path: text_box_path,
            paragraphs,
            floating,
            x_emu,
            y_emu,
            source_paragraph_path,
            width_emu,
            height_emu,
        });
        return;
    }

    let mut counters = BTreeMap::<String, usize>::new();
    for child in &node.children {
        let local_name = local_name_of(&child.name);
        let count = next_occurrence(&mut counters, local_name);
        walk_node(
            child,
            &format!("{path}/{local_name}[{count}]"),
            source_paragraph_path.as_deref(),
            floating,
            x_emu,
            y_emu,
            width_emu,
            height_emu,
            styles,
            numbering,
            model,
        );
    }
}

fn parse_vml_geometry(style: Option<&str>) -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
    let mut x_emu = None;
    let mut y_emu = None;
    let mut width_emu = None;
    let mut height_emu = None;
    for declaration in style.unwrap_or_default().split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let (number, emu_per_unit) = if let Some(number) = value.strip_suffix("pt") {
            (number, 12_700.0)
        } else if let Some(number) = value.strip_suffix("px") {
            (number, 9_525.0)
        } else {
            continue;
        };
        let Some(number) = number.trim().parse::<f64>().ok() else {
            continue;
        };
        let emu = (number * emu_per_unit).round() as i64;
        if property.trim().eq_ignore_ascii_case("margin-left")
            || property.trim().eq_ignore_ascii_case("left")
        {
            x_emu = Some(emu);
        } else if property.trim().eq_ignore_ascii_case("margin-top")
            || property.trim().eq_ignore_ascii_case("top")
        {
            y_emu = Some(emu);
        } else if property.trim().eq_ignore_ascii_case("width") {
            width_emu = Some(emu);
        } else if property.trim().eq_ignore_ascii_case("height") {
            height_emu = Some(emu);
        }
    }
    (x_emu, y_emu, width_emu, height_emu)
}

fn next_occurrence(counters: &mut BTreeMap<String, usize>, name: &str) -> usize {
    let entry = counters.entry(name.to_string()).or_insert(0);
    *entry += 1;
    *entry
}

#[cfg(test)]
mod tests {
    use super::collect_text_boxes;
    use crate::docx_ingest::{
        model::DocxDocumentModel, numbering::DocxNumberingCatalog, styles::DocxStyleCatalog,
        xml::parse_xml,
    };

    #[test]
    fn extracts_text_box_paragraphs_and_marks_uncertain_float_order() {
        let root = parse_xml(
            br#"<w:document xmlns:w="urn:w" xmlns:wp="urn:wp"><w:body><w:p><w:r><w:drawing><wp:anchor><wps:wsp xmlns:wps="urn:wps"><wps:txbx><w:txbxContent><w:p><w:r><w:t>box text</w:t></w:r></w:p></w:txbxContent></wps:txbx></wps:wsp></wp:anchor></w:drawing></w:r></w:p></w:body></w:document>"#,
            "textbox",
        )
        .unwrap();
        let mut model = DocxDocumentModel {
            main_document_part: "word/document.xml".to_string(),
            ..DocxDocumentModel::default()
        };
        model
            .sections
            .push(super::super::sections::DocxSection::default_for(0));
        collect_text_boxes(
            &root,
            &DocxStyleCatalog::default(),
            &DocxNumberingCatalog::default(),
            &mut model,
        );
        assert_eq!(model.text_boxes.len(), 1);
        assert_eq!(model.text_boxes[0].paragraphs[0].raw_text(), "box text");
        assert!(model
            .issues
            .iter()
            .any(|issue| issue.code == "DOCX_FLOATING_ORDER_AMBIGUOUS"));
    }
}
