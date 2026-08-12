use super::{
    model::{DocxCompositeDrawing, DocxDocumentModel, DocxDrawing, DocxDrawingKind, DocxIssue},
    package::DocxPackage,
    xml::{local_name_of, XmlNode},
};
use std::collections::BTreeMap;

pub(crate) fn collect_drawings(
    root: &XmlNode,
    package: &DocxPackage,
    main_document_part: &str,
    model: &mut DocxDocumentModel,
) {
    walk_node(
        root,
        &format!("/{main_document_part}"),
        None,
        main_document_part,
        package,
        model,
    );
}

fn walk_node(
    node: &XmlNode,
    path: &str,
    paragraph_path: Option<&str>,
    source_part: &str,
    package: &DocxPackage,
    model: &mut DocxDocumentModel,
) {
    let current_paragraph = if node.is("p") {
        Some(path.to_string())
    } else {
        paragraph_path.map(str::to_string)
    };
    if node.is("drawing") || node.is("pict") {
        parse_drawing_node(
            node,
            path,
            current_paragraph.as_deref(),
            source_part,
            package,
            model,
        );
    }
    let mut counters = BTreeMap::<String, usize>::new();
    for child in &node.children {
        let local_name = local_name_of(&child.name);
        let count = next_occurrence(&mut counters, local_name);
        walk_node(
            child,
            &format!("{path}/{local_name}[{count}]"),
            current_paragraph.as_deref(),
            source_part,
            package,
            model,
        );
    }
}

fn parse_drawing_node(
    node: &XmlNode,
    path: &str,
    paragraph_path: Option<&str>,
    source_part: &str,
    package: &DocxPackage,
    model: &mut DocxDocumentModel,
) {
    let anchor = node.child("anchor");
    let inline = node.child("inline");
    let container = anchor.or(inline);
    let floating = anchor.is_some();
    let extent = container.and_then(|container| container.child("extent"));
    let width_emu = extent
        .and_then(|extent| extent.attr("cx"))
        .and_then(|value| value.parse::<i64>().ok());
    let height_emu = extent
        .and_then(|extent| extent.attr("cy"))
        .and_then(|value| value.parse::<i64>().ok());
    let x_emu = container
        .and_then(|container| container.child("positionH"))
        .and_then(|position| position.child("posOffset"))
        .and_then(|offset| offset.text_content().trim().parse::<i64>().ok());
    let y_emu = container
        .and_then(|container| container.child("positionV"))
        .and_then(|position| position.child("posOffset"))
        .and_then(|offset| offset.text_content().trim().parse::<i64>().ok());
    let relative_height = container
        .and_then(|container| container.attr("relativeHeight"))
        .and_then(|value| value.parse::<i64>().ok());
    let wrap = container.and_then(|container| {
        container
            .children
            .iter()
            .find(|child| local_name_of(&child.name).starts_with("wrap"))
            .map(|child| local_name_of(&child.name).to_string())
    });
    let doc_pr = container.and_then(|container| container.child("docPr"));
    let alt_text = doc_pr
        .and_then(|node| node.attr("descr"))
        .map(str::to_string);
    let title = doc_pr
        .and_then(|node| node.attr("title"))
        .map(str::to_string);
    let rotation = node
        .descendants_named("xfrm")
        .into_iter()
        .find_map(|xfrm| xfrm.attr("rot").and_then(|value| value.parse::<i64>().ok()));
    let crop = node
        .descendants_named("srcRect")
        .into_iter()
        .next()
        .map(|rect| {
            [
                parse_crop(rect.attr("l")),
                parse_crop(rect.attr("t")),
                parse_crop(rect.attr("r")),
                parse_crop(rect.attr("b")),
            ]
        });

    let mut relationship_ids = Vec::new();
    for candidate in node
        .descendants_named("blip")
        .into_iter()
        .chain(node.descendants_named("chart"))
        .chain(node.descendants_named("relIds"))
        .chain(node.descendants_named("imagedata"))
    {
        for attribute in ["embed", "link", "id", "dm", "lo", "qs", "cs"] {
            if let Some(value) = candidate.attr(attribute) {
                if !relationship_ids.iter().any(|item| item == value) {
                    relationship_ids.push(value.to_string());
                }
            }
        }
    }
    let relationship_id = relationship_ids.first().cloned();
    let relationship = relationship_id.as_deref().and_then(|id| {
        package
            .relationships_for(source_part)
            .iter()
            .find(|relationship| relationship.id == id)
    });
    let relationship_target =
        relationship.and_then(|relationship| relationship.resolved_target.clone());
    let mut relationship_targets = Vec::new();
    let mut external_relationship_ids = Vec::new();
    for candidate_id in &relationship_ids {
        let candidate_relationship = package
            .relationships_for(source_part)
            .iter()
            .find(|relationship| relationship.id == *candidate_id);
        if candidate_relationship.is_none() {
            model.issues.push(DocxIssue::warning(
                "DOCX_DRAWING_RELATIONSHIP_MISSING",
                format!(
                    "drawing relationship {candidate_id} is not present in package relationships"
                ),
                Some(path.to_string()),
            ));
        } else if candidate_relationship.is_some_and(|relationship| relationship.is_external()) {
            let mut issue = DocxIssue::error(
                "DOCX_EXTERNAL_ASSET_MISSING",
                format!("external drawing relationship {candidate_id} is not fetched"),
                Some(path.to_string()),
            );
            issue.relationship_id = Some(candidate_id.to_string());
            model.issues.push(issue);
            external_relationship_ids.push(candidate_id.clone());
        } else if let Some(target) =
            candidate_relationship.and_then(|relationship| relationship.resolved_target.clone())
        {
            if !relationship_targets.contains(&target) {
                relationship_targets.push(target);
            }
        }
    }
    if relationship_ids.is_empty() {
        model.issues.push(DocxIssue::warning(
            "DOCX_DRAWING_RELATIONSHIP_MISSING",
            "drawing contains no resolvable relationship identifier",
            Some(path.to_string()),
        ));
    }

    let kind = drawing_kind(node, package, source_part);
    let external = !external_relationship_ids.is_empty();
    let drawing = DocxDrawing {
        path: path.to_string(),
        relationship_id,
        relationship_ids: relationship_ids.clone(),
        relationship_target,
        relationship_targets,
        external_relationship_ids,
        external,
        width_emu,
        height_emu,
        x_emu,
        y_emu,
        floating,
        relative_height,
        wrap,
        alt_text,
        title,
        rotation,
        crop,
        source_paragraph_path: paragraph_path.map(str::to_string),
        source_kind: kind.clone(),
    };
    model.drawings.push(drawing);

    if matches!(
        kind,
        DocxDrawingKind::Chart | DocxDrawingKind::SmartArt | DocxDrawingKind::Shape
    ) {
        let text = node
            .descendants_named("t")
            .into_iter()
            .map(XmlNode::text_content)
            .collect::<Vec<_>>()
            .join("");
        let preview_relationship_id = node
            .descendants_named("blip")
            .into_iter()
            .find_map(|blip| blip.attr("embed").or_else(|| blip.attr("link")))
            .map(str::to_string);
        model.composites.push(DocxCompositeDrawing {
            path: path.to_string(),
            kind,
            relationship_ids,
            text,
            preview_relationship_id,
        });
    }
}

fn drawing_kind(node: &XmlNode, package: &DocxPackage, source_part: &str) -> DocxDrawingKind {
    if node.is("pict") {
        return DocxDrawingKind::Vml;
    }
    if node.descendants_named("chart").into_iter().any(|chart| {
        chart
            .attr("id")
            .or_else(|| chart.attr("embed"))
            .and_then(|id| {
                package
                    .relationships_for(source_part)
                    .iter()
                    .find(|relationship| relationship.id == id)
            })
            .is_some_and(|relationship| relationship.relationship_type.ends_with("/chart"))
    }) {
        return DocxDrawingKind::Chart;
    }
    if node.descendants_named("relIds").into_iter().any(|rel_ids| {
        ["dm", "lo", "qs", "cs"].iter().any(|attribute| {
            rel_ids
                .attr(attribute)
                .and_then(|id| {
                    package
                        .relationships_for(source_part)
                        .iter()
                        .find(|relationship| relationship.id == id)
                })
                .is_some()
        })
    }) {
        return DocxDrawingKind::SmartArt;
    }
    if node
        .descendants_named("txbxContent")
        .into_iter()
        .next()
        .is_some()
    {
        return DocxDrawingKind::Shape;
    }
    if node.descendants_named("blip").into_iter().next().is_some() {
        DocxDrawingKind::Image
    } else {
        DocxDrawingKind::Unknown
    }
}

fn parse_crop(value: Option<&str>) -> f64 {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| value / 100_000.0)
        .unwrap_or(0.0)
}

fn next_occurrence(counters: &mut BTreeMap<String, usize>, name: &str) -> usize {
    let entry = counters.entry(name.to_string()).or_insert(0);
    *entry += 1;
    *entry
}
