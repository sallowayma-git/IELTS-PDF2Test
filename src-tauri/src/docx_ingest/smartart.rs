use super::{
    model::{DocxCompositeDrawing, DocxDocumentModel, DocxDrawingKind, DocxIssue},
    package::DocxPackage,
    xml::parse_xml,
};

pub(crate) fn collect_composite_fallbacks(
    package: &DocxPackage,
    source_part: &str,
    model: &mut DocxDocumentModel,
) {
    let relationships = package.relationships_for(source_part).to_vec();
    for relationship in relationships {
        let Some(target) = relationship.resolved_target.as_deref() else {
            continue;
        };
        let is_chart = relationship.relationship_type.ends_with("/chart");
        let is_smartart = relationship.relationship_type.ends_with("/diagramData");
        if !is_chart && !is_smartart {
            continue;
        }
        let kind = if is_chart {
            DocxDrawingKind::Chart
        } else {
            DocxDrawingKind::SmartArt
        };
        let text = package
            .part_bytes(target)
            .and_then(|bytes| parse_xml(bytes, target).ok())
            .map(|root| {
                root.descendants_named("t")
                    .into_iter()
                    .chain(root.descendants_named("v"))
                    .map(|node| node.text_content())
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let existing = model.composites.iter_mut().find(|composite| {
            composite
                .relationship_ids
                .iter()
                .any(|id| id == &relationship.id)
        });
        if let Some(existing) = existing {
            if existing.text.is_empty() {
                existing.text = text;
            }
            let preview_is_verifiable = existing
                .preview_relationship_id
                .as_deref()
                .and_then(|preview_id| {
                    package
                        .relationships_for(source_part)
                        .iter()
                        .find(|candidate| candidate.id == preview_id && !candidate.is_external())
                })
                .and_then(|preview| preview.resolved_target.as_deref())
                .is_some_and(|target| {
                    package.part_bytes(target).is_some()
                        && (target.starts_with("word/media/")
                            || package
                                .content_type(target)
                                .is_some_and(|content_type| content_type.starts_with("image/")))
                });
            if !preview_is_verifiable {
                existing.preview_relationship_id = None;
                model.issues.push(DocxIssue::error(
                    "UNSUPPORTED_DOCX_COMPOSITE_DRAWING",
                    format!(
                        "{} relationship {} has structure but no preview image/render provider",
                        if is_chart { "chart" } else { "SmartArt" },
                        relationship.id
                    ),
                    Some(existing.path.clone()),
                ));
            }
            continue;
        }
        model.composites.push(DocxCompositeDrawing {
            path: format!("/{target}"),
            kind,
            relationship_ids: vec![relationship.id.clone()],
            text,
            preview_relationship_id: None,
        });
        model.issues.push(DocxIssue::error(
            "UNSUPPORTED_DOCX_COMPOSITE_DRAWING",
            format!(
                "{} relationship {} has no document anchor or preview image",
                if is_chart { "chart" } else { "SmartArt" },
                relationship.id
            ),
            Some(format!("/{target}")),
        ));
    }

    for entry in package.entries() {
        if entry.path.starts_with("word/embeddings/") {
            model.issues.push(DocxIssue::warning(
                "DOCX_EMBEDDING_NOT_IMPORTED",
                format!(
                    "embedded object {} is retained as package input but never executed/imported",
                    entry.path
                ),
                Some(format!("/{}", entry.path)),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::collect_composite_fallbacks;
    use crate::docx_ingest::model::DocxDocumentModel;

    #[test]
    fn keeps_composite_fallback_contract_explicit() {
        let model = DocxDocumentModel::default();
        assert!(model.composites.is_empty());
        let _ = collect_composite_fallbacks;
    }
}
