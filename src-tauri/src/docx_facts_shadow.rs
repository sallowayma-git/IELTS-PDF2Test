use crate::artifact_store::write_canonical_json_atomic;
use crate::pdf_facts_shadow::{extract_pdf_facts_shadow, SHADOW_COMPARE_FILE};
use crate::pdf_ingest::build_compare_report;
use crate::schema::document_ir_v2::DocumentIRV2;
use crate::{CommandResult, ImportJob, SourceFile};
use chrono::Utc;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, io::Write, path::Path};

use crate::docx_ingest::{
    drawings::collect_drawings,
    model::{
        DocxBlock, DocxDocumentModel, DocxIssue, DocxIssueSeverity, DocxParagraph, DocxRun,
        DocxRunFormatting, DocxRunKind, DocxTable,
    },
    numbering::{assign_numbering_labels, DocxNumberingCatalog},
    open_docx,
    paragraphs::parse_document_model,
    render_fallback::{render_docx, requested_from_environment, DocxRenderAssistResult},
    sections::DocxSection,
    smartart::collect_composite_fallbacks,
    styles::DocxStyleCatalog,
    text_boxes::collect_text_boxes,
    xml::parse_xml,
    DocxPackage, DocxPackageLimits,
};

pub(crate) fn write_docx_facts_shadow(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
    output_path: &Path,
) -> CommandResult<Value> {
    write_docx_facts_shadow_with_v1(job, source, input_path, output_path, None)
}

pub(crate) fn write_docx_facts_shadow_with_v1(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
    output_path: &Path,
    v1_document: Option<&Value>,
) -> CommandResult<Value> {
    let output_parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(output_parent)
        .map_err(|error| format!("docx_shadow_output_dir_create_failed:{error}"))?;
    let staging_root = output_parent.join(format!(
        ".docx-shadow-txn-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(staging_root.join("assets").join("shadow").join("docx"))
        .map_err(|error| format!("docx_shadow_staging_create_failed:{error}"))?;
    let result = (|| -> CommandResult<Value> {
        let value =
            extract_docx_facts_shadow_internal(job, source, input_path, Some(&staging_root))?;
        let output_name = output_path
            .file_name()
            .ok_or_else(|| "docx_shadow_output_file_name_missing".to_string())?;
        write_canonical_json_atomic(&staging_root.join(output_name), &value)?;
        let compare = build_compare_report(&job.job_id, &value, v1_document);
        write_canonical_json_atomic(&staging_root.join(SHADOW_COMPARE_FILE), &compare)?;
        commit_docx_shadow_bundle(&staging_root, output_path)?;
        Ok(value)
    })();
    if staging_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
    }
    result
}

fn remove_docx_shadow_path(path: &Path) -> CommandResult<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|error| format!("docx_shadow_path_remove_failed:{}:{error}", path.display()))
}

fn rollback_docx_shadow_bundle(
    targets: &[std::path::PathBuf],
    backups: &[std::path::PathBuf],
    installed: &[usize],
    backed_up: &[usize],
    backup_root: &Path,
) -> Option<String> {
    let mut failures = Vec::new();
    for index in installed.iter().rev().copied() {
        if let Err(error) = remove_docx_shadow_path(&targets[index]) {
            failures.push(format!("remove:{}:{error}", targets[index].display()));
        }
    }
    for index in backed_up.iter().rev().copied() {
        if let Err(error) = fs::rename(&backups[index], &targets[index]) {
            failures.push(format!("restore:{}:{error}", targets[index].display()));
        }
    }
    if failures.is_empty() {
        if let Err(error) = fs::remove_dir_all(backup_root) {
            failures.push(format!("cleanup:{}:{error}", backup_root.display()));
        }
    }
    if failures.is_empty() {
        None
    } else {
        Some(format!(
            "DOCX_SHADOW_ROLLBACK_FAILED:{}:backup_preserved={}",
            failures.join(";"),
            backup_root.display()
        ))
    }
}

fn commit_docx_shadow_bundle(staging_root: &Path, output_path: &Path) -> CommandResult<()> {
    commit_docx_shadow_bundle_with_hook(staging_root, output_path, |_, _, _, _| Ok(()))
}

fn commit_docx_shadow_bundle_with_hook<F>(
    staging_root: &Path,
    output_path: &Path,
    mut before_install: F,
) -> CommandResult<()>
where
    F: FnMut(usize, &Path, &Path, &Path) -> CommandResult<()>,
{
    let output_parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    let output_name = output_path
        .file_name()
        .ok_or_else(|| "docx_shadow_output_file_name_missing".to_string())?;
    let staged = [
        staging_root.join(output_name),
        staging_root.join(SHADOW_COMPARE_FILE),
        staging_root.join("assets").join("shadow").join("docx"),
    ];
    for path in &staged {
        if !path.exists() {
            return Err(format!(
                "docx_shadow_staged_component_missing:{}",
                path.display()
            ));
        }
    }
    let targets = [
        output_path.to_path_buf(),
        output_parent.join(SHADOW_COMPARE_FILE),
        output_parent.join("assets").join("shadow").join("docx"),
    ];
    let backup_root = output_parent.join(format!(
        ".docx-shadow-backup-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&backup_root)
        .map_err(|error| format!("docx_shadow_backup_create_failed:{error}"))?;
    let backups = [
        backup_root.join("artifact.json"),
        backup_root.join("compare.json"),
        backup_root.join("assets-shadow"),
    ];
    let mut backed_up = Vec::<usize>::new();
    for (index, target) in targets.iter().enumerate() {
        if !target.exists() {
            continue;
        }
        if let Err(error) = fs::rename(target, &backups[index]) {
            let rollback =
                rollback_docx_shadow_bundle(&targets, &backups, &[], &backed_up, &backup_root);
            return Err(match rollback {
                Some(rollback) => format!(
                    "docx_shadow_backup_failed:{}:{error};{rollback}",
                    target.display()
                ),
                None => format!("docx_shadow_backup_failed:{}:{error}", target.display()),
            });
        }
        backed_up.push(index);
    }

    let mut installed = Vec::<usize>::new();
    for (index, source) in staged.iter().enumerate() {
        if let Some(parent) = targets[index].parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                let rollback = rollback_docx_shadow_bundle(
                    &targets,
                    &backups,
                    &installed,
                    &backed_up,
                    &backup_root,
                );
                return Err(match rollback {
                    Some(rollback) => {
                        format!("docx_shadow_target_dir_create_failed:{error};{rollback}")
                    }
                    None => format!("docx_shadow_target_dir_create_failed:{error}"),
                });
            }
        }
        if let Err(error) = before_install(index, source, &targets[index], &backup_root) {
            let rollback = rollback_docx_shadow_bundle(
                &targets,
                &backups,
                &installed,
                &backed_up,
                &backup_root,
            );
            return Err(match rollback {
                Some(rollback) => {
                    format!("docx_shadow_commit_hook_failed:index={index}:{error};{rollback}")
                }
                None => format!("docx_shadow_commit_hook_failed:index={index}:{error}"),
            });
        }
        if let Err(error) = fs::rename(source, &targets[index]) {
            let rollback = rollback_docx_shadow_bundle(
                &targets,
                &backups,
                &installed,
                &backed_up,
                &backup_root,
            );
            return Err(match rollback {
                Some(rollback) => format!(
                    "docx_shadow_commit_failed:{}:{error};{rollback}",
                    targets[index].display()
                ),
                None => format!(
                    "docx_shadow_commit_failed:{}:{error}",
                    targets[index].display()
                ),
            });
        }
        installed.push(index);
    }
    let _ = fs::remove_dir_all(&backup_root);
    Ok(())
}

pub(crate) fn extract_docx_facts_shadow(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
) -> CommandResult<Value> {
    extract_docx_facts_shadow_internal(job, source, input_path, None)
}

fn extract_docx_facts_shadow_internal(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
    artifact_root: Option<&Path>,
) -> CommandResult<Value> {
    extract_docx_facts_shadow_with_renderer(
        job,
        source,
        input_path,
        artifact_root,
        requested_from_environment(),
        &render_docx,
    )
}

fn extract_docx_facts_shadow_with_renderer(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
    artifact_root: Option<&Path>,
    render_requested: bool,
    renderer: &dyn Fn(&Path, bool) -> DocxRenderAssistResult,
) -> CommandResult<Value> {
    let extraction_started_at = Utc::now();
    let package = open_docx(input_path, DocxPackageLimits::default())?;
    let main_document_part = package
        .main_document_part()
        .map(str::to_string)
        .or_else(|| {
            package
                .part_bytes("word/document.xml")
                .map(|_| "word/document.xml".to_string())
        })
        .ok_or_else(|| "docx_shadow_main_document_part_missing".to_string())?;
    let document_xml = package
        .part_bytes(&main_document_part)
        .ok_or_else(|| format!("docx_shadow_document_part_missing:{main_document_part}"))?;
    let root = parse_xml(document_xml, &main_document_part)?;
    let (styles, style_warning) = parse_optional_styles(&package);
    let (numbering, numbering_warning) = parse_optional_numbering(&package);
    let mut model = parse_document_model(&root, &main_document_part, &styles, &numbering);
    if let Some(warning) = style_warning {
        model.warnings.push(warning);
    }
    if let Some(warning) = numbering_warning {
        model.warnings.push(warning);
    }
    collect_drawings(&root, &package, &main_document_part, &mut model);
    collect_text_boxes(&root, &styles, &numbering, &mut model);
    collect_composite_fallbacks(&package, &main_document_part, &mut model);
    assign_numbering_labels(&mut model, &numbering);
    let mut render_result = renderer(input_path, render_requested);
    let rendered_pdf_path = render_result.rendered_pdf().map(Path::to_path_buf);
    let mut rendered_pages = if let Some(rendered_pdf) = rendered_pdf_path {
        match extract_pdf_facts_shadow(job, source, &rendered_pdf) {
            Ok(value) => value.get("pages").and_then(Value::as_array).cloned(),
            Err(error) => {
                render_result.mark_geometry_failure(format!(
                    "rendered PDF geometry extraction failed: {error}"
                ));
                None
            }
        }
    } else {
        None
    };
    render_result.record_issue(&mut model);
    let render_assist = render_result.metadata();
    let (assets, page_asset_ids, asset_warnings, drawing_asset_ids) =
        extract_docx_assets(&package, source, &model, artifact_root)?;
    model.warnings.extend(asset_warnings);

    let mut pages = if let Some(ref mut pages) = rendered_pages {
        bind_rendered_pages_to_ooxml(pages, &model, source);
        pages.clone()
    } else {
        build_pages(&model, source)
    };
    for (page_index, asset_ids) in page_asset_ids {
        if let Some(page) = pages.get_mut(page_index as usize) {
            if let Some(object) = page.as_object_mut() {
                object.insert("assetIds".to_string(), json!(asset_ids));
            }
        }
    }
    attach_drawing_assets_to_regions(&mut pages, &model, &drawing_asset_ids);
    let mut coverage_ledger = coverage_ledger_from_pages(&pages);
    coverage_ledger.extend(assets.iter().filter_map(|asset| {
        asset
            .get("assetId")
            .and_then(Value::as_str)
            .map(|asset_id| {
                json!({
                    "sourceNodeId": asset_id,
                    "disposition": "unassigned",
                    "targetIds": [],
                    "reason": "DOCX visual asset retained; semantic assignment is deferred"
                })
            })
    }));

    let mut warnings = model.warnings.clone();
    warnings.extend(model.issues.iter().map(issue_warning_text));
    let publish_blocked = model
        .issues
        .iter()
        .any(|issue| issue.severity == DocxIssueSeverity::Error);
    let issue_values = model.issues.iter().map(issue_value).collect::<Vec<_>>();
    let extraction_completed_at = Utc::now();
    let value = json!({
        "schemaVersion": "DocumentIRV2",
        "documentId": format!("document-{}", &source.sha256[..source.sha256.len().min(16)]),
        "jobId": job.job_id,
        "sourceFiles": [{
            "sourceFileId": source.file_id,
            "originalName": source.original_name,
            "mediaType": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "sha256": source.sha256,
            "byteLength": source.size_bytes,
            "role": source_role(&source.role)
        }],
        "pages": pages,
        "assets": assets,
        "coverageLedger": coverage_ledger,
        "parser": {
            "provider": "rust-parser:docx:ooxml:shadow",
            "providerVersion": "phase3-c002-c009",
            "extractionStartedAt": extraction_started_at.to_rfc3339(),
            "extractionCompletedAt": extraction_completed_at.to_rfc3339(),
            "options": {
                "featureFlag": "documentIrV2Shadow",
                "sourceKind": "docx",
                "coordinateOrigin": "top-left",
                "pageIndexBase": 0,
                "charRangeScope": "paragraph",
                "semanticLayers": false,
                "renderAssist": render_assist,
                "publishBlocked": publish_blocked,
                "ooxml": {
                    "mainDocumentPart": main_document_part,
                    "styleDefinitionCount": styles.styles.len(),
                    "numberingInstanceCount": numbering.instances.len(),
                    "paragraphCount": model.blocks.iter().filter(|block| matches!(block, DocxBlock::Paragraph(_))).count(),
                    "tableCount": model.blocks.iter().filter(|block| matches!(block, DocxBlock::Table(_))).count(),
                    "drawingCount": model.drawings.len(),
                    "textBoxCount": model.text_boxes.len(),
                    "compositeDrawingCount": model.composites.len(),
                    "relationships": relationship_values(&package),
                    "drawings": model.drawings.iter().map(drawing_value).collect::<Vec<_>>(),
                    "textBoxes": model.text_boxes.iter().map(text_box_value).collect::<Vec<_>>(),
                    "compositeDrawings": model.composites.iter().map(composite_value).collect::<Vec<_>>(),
                    "numberingFacts": numbering_values(&model),
                    "fieldFacts": field_values(&model),
                    "auxiliaryParts": auxiliary_part_values(&package),
                    "sections": model.sections.iter().map(section_value).collect::<Vec<_>>(),
                    "issues": issue_values
                }
            },
            "warnings": warnings
        }
    });
    let typed = serde_json::from_value::<DocumentIRV2>(value)
        .map_err(|error| format!("docx_facts_shadow_schema_validation_failed:{error}"))?;
    if !typed.is_supported_schema_version() {
        return Err("docx_facts_shadow_schema_version_unsupported".to_string());
    }
    serde_json::to_value(typed).map_err(|error| error.to_string())
}

fn parse_optional_styles(package: &DocxPackage) -> (DocxStyleCatalog, Option<String>) {
    match package.part_bytes("word/styles.xml") {
        Some(bytes) => DocxStyleCatalog::parse(bytes)
            .map(|catalog| (catalog, None))
            .unwrap_or_else(|error| {
                (
                    DocxStyleCatalog::default(),
                    Some(format!("DOCX_STYLES_PARSE_FAILED:{error}")),
                )
            }),
        None => (DocxStyleCatalog::default(), None),
    }
}

fn parse_optional_numbering(package: &DocxPackage) -> (DocxNumberingCatalog, Option<String>) {
    match package.part_bytes("word/numbering.xml") {
        Some(bytes) => DocxNumberingCatalog::parse(bytes)
            .map(|catalog| (catalog, None))
            .unwrap_or_else(|error| {
                (
                    DocxNumberingCatalog::default(),
                    Some(format!("DOCX_NUMBERING_PARSE_FAILED:{error}")),
                )
            }),
        None => (DocxNumberingCatalog::default(), None),
    }
}

fn source_role(role: &str) -> &'static str {
    match role {
        "MainQuestion" => "question_paper",
        "AnswerKey" => "answer_key",
        "Explanation" => "explanation",
        "Supplement" => "supplement",
        _ => "unknown",
    }
}

fn issue_warning_text(issue: &DocxIssue) -> String {
    format!(
        "{}:{}{}",
        issue.code,
        issue.message,
        issue
            .path
            .as_deref()
            .map(|path| format!(":{path}"))
            .unwrap_or_default()
    )
}

fn issue_value(issue: &DocxIssue) -> Value {
    json!({
        "code": issue.code,
        "severity": match issue.severity {
            DocxIssueSeverity::Warning => "warning",
            DocxIssueSeverity::Error => "error"
        },
        "message": issue.message,
        "path": issue.path,
        "relationshipId": issue.relationship_id
    })
}

fn section_value(section: &DocxSection) -> Value {
    json!({
        "index": section.index,
        "pageWidthTwips": section.page_width_twips,
        "pageHeightTwips": section.page_height_twips,
        "orientation": section.orientation,
        "marginTopTwips": section.margin_top_twips,
        "marginRightTwips": section.margin_right_twips,
        "marginBottomTwips": section.margin_bottom_twips,
        "marginLeftTwips": section.margin_left_twips,
        "columns": section.columns.iter().map(|column| json!({
            "widthTwips": column.width_twips,
            "spaceTwips": column.space_twips
        })).collect::<Vec<_>>(),
        "equalWidth": section.equal_width,
        "separator": section.separator,
        "headerRelationshipIds": section.header_relationship_ids,
        "footerRelationshipIds": section.footer_relationship_ids,
        "titlePage": section.title_page
        ,"breakType": section.break_type
    })
}

fn relationship_values(package: &DocxPackage) -> Vec<Value> {
    package
        .relationships()
        .map(|relationship| {
            json!({
                "sourcePart": relationship.source_part,
                "id": relationship.id,
                "type": relationship.relationship_type,
                "target": relationship.target,
                "targetMode": relationship.target_mode,
                "external": relationship.is_external(),
                "resolvedTarget": relationship.resolved_target
            })
        })
        .collect()
}

fn drawing_kind_value(kind: &crate::docx_ingest::model::DocxDrawingKind) -> &'static str {
    use crate::docx_ingest::model::DocxDrawingKind;
    match kind {
        DocxDrawingKind::Image => "image",
        DocxDrawingKind::Shape => "shape",
        DocxDrawingKind::Chart => "chart",
        DocxDrawingKind::SmartArt => "smartart",
        DocxDrawingKind::Vml => "vml",
        DocxDrawingKind::Unknown => "unknown",
    }
}

fn drawing_value(drawing: &crate::docx_ingest::model::DocxDrawing) -> Value {
    json!({
        "path": drawing.path,
        "kind": drawing_kind_value(&drawing.source_kind),
        "relationshipId": drawing.relationship_id,
        "relationshipIds": drawing.relationship_ids,
        "relationshipTarget": drawing.relationship_target,
        "relationshipTargets": drawing.relationship_targets,
        "externalRelationshipIds": drawing.external_relationship_ids,
        "external": drawing.external,
        "widthEmu": drawing.width_emu,
        "heightEmu": drawing.height_emu,
        "xEmu": drawing.x_emu,
        "yEmu": drawing.y_emu,
        "floating": drawing.floating,
        "relativeHeight": drawing.relative_height,
        "wrap": drawing.wrap,
        "altText": drawing.alt_text,
        "title": drawing.title,
        "rotation": drawing.rotation,
        "crop": drawing.crop,
        "sourceParagraphPath": drawing.source_paragraph_path
    })
}

fn text_box_value(text_box: &crate::docx_ingest::model::DocxTextBox) -> Value {
    json!({
        "path": text_box.path,
        "text": text_box.paragraphs.iter().map(DocxParagraph::display_text).collect::<Vec<_>>().join("\n"),
        "paragraphPaths": text_box.paragraphs.iter().map(|paragraph| paragraph.path.clone()).collect::<Vec<_>>(),
        "floating": text_box.floating,
        "xEmu": text_box.x_emu,
        "yEmu": text_box.y_emu,
        "widthEmu": text_box.width_emu,
        "heightEmu": text_box.height_emu,
        "sourceParagraphPath": text_box.source_paragraph_path
    })
}

fn composite_value(composite: &crate::docx_ingest::model::DocxCompositeDrawing) -> Value {
    json!({
        "path": composite.path,
        "kind": drawing_kind_value(&composite.kind),
        "relationshipIds": composite.relationship_ids,
        "accessibleText": composite.text,
        "previewRelationshipId": composite.preview_relationship_id
    })
}

fn numbering_values(model: &DocxDocumentModel) -> Vec<Value> {
    let mut values = Vec::new();
    for paragraph in all_paragraphs(model) {
        if let Some(numbering) = &paragraph.resolved_numbering {
            values.push(json!({
                "paragraphPath": paragraph.path,
                "numId": numbering.num_id,
                "abstractNumId": numbering.abstract_id,
                "level": numbering.level,
                "format": numbering.format,
                "levelText": numbering.text,
                "start": numbering.start,
                "leftIndentTwips": numbering.left_indent_twips,
                "hangingIndentTwips": numbering.hanging_indent_twips
                ,"renderedLabel": paragraph.numbering_label
            }));
        }
    }
    values
}

fn field_values(model: &DocxDocumentModel) -> Vec<Value> {
    let mut values = Vec::new();
    for paragraph in all_paragraphs(model) {
        for run in &paragraph.runs {
            if run.kind == crate::docx_ingest::model::DocxRunKind::Field {
                values.push(json!({
                    "paragraphPath": paragraph.path,
                    "runPath": run.path,
                    "instruction": run.field_instruction,
                    "fieldBoundary": run.break_type,
                    "displayText": run.text
                }));
            }
        }
    }
    values
}

fn all_paragraphs(model: &DocxDocumentModel) -> Vec<&DocxParagraph> {
    fn table_paragraphs<'a>(table: &'a DocxTable, output: &mut Vec<&'a DocxParagraph>) {
        for row in &table.rows {
            for cell in &row.cells {
                output.extend(cell.paragraphs.iter());
                for nested in &cell.tables {
                    table_paragraphs(nested, output);
                }
            }
        }
    }
    let mut output = Vec::new();
    for block in &model.blocks {
        match block {
            DocxBlock::Paragraph(paragraph) => output.push(paragraph),
            DocxBlock::Table(table) => table_paragraphs(table, &mut output),
        }
    }
    for text_box in &model.text_boxes {
        output.extend(text_box.paragraphs.iter());
    }
    output
}

fn auxiliary_part_values(package: &DocxPackage) -> Vec<Value> {
    package
        .entries()
        .filter(|entry| {
            !entry.is_directory
                && (entry.path.starts_with("word/header")
                    || entry.path.starts_with("word/footer")
                    || entry.path == "word/footnotes.xml"
                    || entry.path == "word/endnotes.xml"
                    || entry.path == "word/comments.xml"
                    || entry.path == "word/settings.xml"
                    || entry.path == "word/fontTable.xml"
                    || entry.path.starts_with("word/theme/")
                    || entry.path.starts_with("word/diagrams/")
                    || entry.path.starts_with("word/charts/")
                    || entry.path.starts_with("word/embeddings/"))
        })
        .map(|entry| {
            let accessible_text = package
                .part_bytes(&entry.path)
                .and_then(|bytes| super_parse_accessible_text(bytes, &entry.path));
            json!({
                "path": entry.path,
                "contentType": package.content_type(&entry.path),
                "byteLength": entry.uncompressed_size,
                "sha256": package.part_bytes(&entry.path).map(crate::hash_bytes),
                "accessibleText": accessible_text,
                "executedOrImported": false
            })
        })
        .collect()
}

fn super_parse_accessible_text(bytes: &[u8], label: &str) -> Option<String> {
    let root = parse_xml(bytes, label).ok()?;
    let text = root
        .descendants_named("t")
        .into_iter()
        .chain(root.descendants_named("v"))
        .map(|node| node.text_content())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then_some(text)
}

struct DocxPageBuilder {
    page_index: u32,
    section: DocxSection,
    source_file_id: String,
    source_hash: String,
    cursor_y: f64,
    source_order: u32,
    char_offset: u32,
    glyphs: Vec<Value>,
    spans: Vec<Value>,
    lines: Vec<Value>,
    regions: Vec<Value>,
    tables: Vec<Value>,
    reading_order: Vec<String>,
    warnings: Vec<String>,
}

impl DocxPageBuilder {
    fn new(page_index: u32, section: DocxSection, source: &SourceFile) -> Self {
        let margin_top = section.margin_top_twips.unwrap_or(1_440) as f64 / 20.0;
        Self {
            page_index,
            section,
            source_file_id: source.file_id.clone(),
            source_hash: source.sha256.clone(),
            cursor_y: margin_top,
            source_order: 0,
            char_offset: 0,
            glyphs: Vec::new(),
            spans: Vec::new(),
            lines: Vec::new(),
            regions: Vec::new(),
            tables: Vec::new(),
            reading_order: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn left_margin_pt(&self) -> f64 {
        self.section.margin_left_twips.unwrap_or(1_440) as f64 / 20.0
    }

    fn content_width_pt(&self) -> f64 {
        let page_width = self.section.page_width_twips as f64 / 20.0;
        page_width
            - self.section.margin_left_twips.unwrap_or(1_440) as f64 / 20.0
            - self.section.margin_right_twips.unwrap_or(1_440) as f64 / 20.0
    }

    fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.tables.is_empty()
    }

    fn add_paragraph(&mut self, paragraph: &DocxParagraph) -> String {
        self.add_paragraph_at(paragraph, self.left_margin_pt(), self.cursor_y)
    }

    fn add_paragraph_at(
        &mut self,
        paragraph: &DocxParagraph,
        origin_x: f64,
        origin_y: f64,
    ) -> String {
        let line_id = format!(
            "docx-p{:03}-line{:04}",
            self.page_index,
            self.lines.len() + 1
        );
        let line_height = paragraph
            .resolved_style
            .as_ref()
            .and_then(|style| style.paragraph.line_twips)
            .map(|value| value.abs() as f64 / 20.0)
            .unwrap_or_else(|| {
                paragraph
                    .runs
                    .iter()
                    .find_map(|run| run.resolved_formatting.font_size_half_points)
                    .map(|value| (value as f64 / 2.0) * 1.25)
                    .unwrap_or(14.0)
            })
            .max(8.0);
        let mut x = origin_x
            + paragraph
                .direct_formatting
                .left_indent_twips
                .unwrap_or_default() as f64
                / 20.0;
        let label_run = paragraph.numbering_label.as_ref().map(|label| DocxRun {
            path: format!("{}/@numberingLabel", paragraph.path),
            kind: DocxRunKind::Text,
            text: format!("{label}\t"),
            resolved_formatting: paragraph
                .runs
                .first()
                .map(|run| run.resolved_formatting.clone())
                .unwrap_or_default(),
            ..DocxRun::default()
        });
        let mut span_ids = Vec::new();
        let mut line_anchors = Vec::new();
        for run in label_run.iter().chain(paragraph.runs.iter()) {
            if run.text.is_empty() && run.kind != crate::docx_ingest::model::DocxRunKind::Drawing {
                continue;
            }
            let span_id = format!(
                "docx-p{:03}-span{:04}",
                self.page_index,
                self.spans.len() + 1
            );
            let span_start_x = x;
            let run_start = self.char_offset;
            let mut glyph_ids = Vec::new();
            for (index, character) in run.text.chars().enumerate() {
                let char_start = run_start.saturating_add(index as u32);
                self.char_offset = self.char_offset.max(char_start.saturating_add(1));
                if character == '\n' || character == '\r' {
                    continue;
                }
                let width = char_width(character, &run.resolved_formatting);
                if character == '\t' {
                    x += width;
                    continue;
                }
                let glyph_id = format!(
                    "docx-p{:03}-glyph{:05}",
                    self.page_index,
                    self.glyphs.len() + 1
                );
                let bbox = rect(x, origin_y, width, line_height, 0);
                self.glyphs.push(json!({
                    "id": glyph_id,
                    "text": character.to_string(),
                    "bbox": bbox,
                    "origin": {"x": clean_number(x), "y": clean_number(origin_y + line_height)},
                    "baseline": clean_number(origin_y + line_height),
                    "style": text_style(&run.resolved_formatting),
                    "unicodeMapError": false,
                    "hidden": false,
                    "visibilityObserved": true,
                    "unicodeMapErrorObserved": false,
                    "geometryBasis": "ooxml_layout_derived",
                    "confidence": 0.98,
                    "source": "native",
                    "sourceAnchor": source_anchor(
                        &self.source_file_id,
                        self.page_index,
                        &glyph_id,
                        Some(&run.path),
                        None,
                        Some(json!({"start": char_start, "end": char_start.saturating_add(1)})),
                        Some(bbox.clone()),
                        &self.source_hash
                    )
                }));
                glyph_ids.push(glyph_id);
                x += width;
            }
            let run_end = self.char_offset.max(run_start);
            let span_bbox = rect(
                span_start_x,
                origin_y,
                (x - span_start_x).max(0.01),
                line_height,
                0,
            );
            let span_anchor = source_anchor(
                &self.source_file_id,
                self.page_index,
                &span_id,
                Some(&run.path),
                None,
                Some(json!({"start": run_start, "end": run_end})),
                Some(span_bbox.clone()),
                &self.source_hash,
            );
            self.spans.push(json!({
                "id": span_id,
                "glyphIds": glyph_ids,
                "text": run.text,
                "bbox": span_bbox,
                "style": text_style(&run.resolved_formatting),
                "whitespaceBefore": whitespace_origin(run.text.chars().next()),
                "whitespaceAfter": whitespace_origin(run.text.chars().next_back()),
                "confidence": 0.98,
                "sourceAnchors": [span_anchor]
            }));
            span_ids.push(span_id);
            line_anchors.push(source_anchor(
                &self.source_file_id,
                self.page_index,
                &line_id,
                Some(&run.path),
                None,
                Some(json!({"start": run_start, "end": run_end})),
                None,
                &self.source_hash,
            ));
        }
        let line_text = paragraph.display_text();
        let line_bbox = rect(
            origin_x,
            origin_y,
            (x - origin_x).max(0.01).min(self.content_width_pt()),
            line_height,
            0,
        );
        let paragraph_anchor = source_anchor(
            &self.source_file_id,
            self.page_index,
            &line_id,
            Some(&paragraph.path),
            None,
            Some(
                json!({"start": self.char_offset.saturating_sub(line_text.chars().count() as u32), "end": self.char_offset}),
            ),
            Some(line_bbox.clone()),
            &self.source_hash,
        );
        line_anchors.push(paragraph_anchor.clone());
        self.lines.push(json!({
            "id": line_id,
            "spanIds": span_ids,
            "text": line_text,
            "bbox": line_bbox.clone(),
            "baseline": clean_number(origin_y + line_height),
            "writingMode": "horizontal-tb",
            "indentationPt": paragraph.direct_formatting.left_indent_twips.unwrap_or_default() as f64 / 20.0,
            "hangingIndentPt": paragraph.direct_formatting.hanging_indent_twips.map(|value| value as f64 / 20.0),
            "lineHeightPt": line_height,
            "hardBreakAfter": true,
            "sourceOrder": self.source_order,
            "confidence": 0.96,
            "sourceAnchors": line_anchors
        }));
        let region_id = format!(
            "docx-p{:03}-region{:04}",
            self.page_index,
            self.regions.len() + 1
        );
        let kind = paragraph_region_kind(paragraph);
        self.regions.push(json!({
            "id": region_id,
            "kind": kind,
            "bbox": line_bbox,
            "childLineIds": [line_id],
            "childObjectIds": [],
            "sectionIndex": paragraph.section.as_ref().map(|section| section.index),
            "readingOrderRank": self.source_order,
            "confidence": 0.94,
            "sourceAnchors": [paragraph_anchor]
        }));
        self.reading_order.push(region_id.clone());
        self.reading_order.push(line_id);
        self.source_order = self.source_order.saturating_add(1);
        let spacing_after = paragraph
            .direct_formatting
            .spacing_after_twips
            .unwrap_or(120) as f64
            / 20.0;
        self.cursor_y = self.cursor_y.max(origin_y + line_height + spacing_after);
        region_id
    }

    fn add_table(&mut self, table: &DocxTable) {
        let table_id = format!(
            "docx-p{:03}-table{:04}",
            self.page_index,
            self.tables.len() + 1
        );
        let top = self.cursor_y;
        let grid_widths = if table.grid_widths_twips.is_empty() {
            vec![1_440; table_column_count(table).max(1)]
        } else {
            table.grid_widths_twips.clone()
        };
        let table_width = grid_widths
            .iter()
            .map(|value| *value as f64 / 20.0)
            .sum::<f64>();
        let row_heights = table
            .rows
            .iter()
            .map(|row| {
                row.height_twips
                    .map(|value| value as f64 / 20.0)
                    .unwrap_or(28.0)
                    .max(1.0)
            })
            .collect::<Vec<_>>();
        let mut row_offsets = Vec::with_capacity(row_heights.len() + 1);
        row_offsets.push(0.0);
        for height in &row_heights {
            row_offsets.push(row_offsets.last().copied().unwrap_or_default() + height);
        }
        let mut cells = Vec::new();
        for (row_index, row) in table.rows.iter().enumerate() {
            let mut column = row.grid_before;
            for cell in &row.cells {
                let col_span = cell.grid_span.max(1);
                if cell.row_span == 0 {
                    column = column.saturating_add(col_span);
                    continue;
                }
                let x = self.left_margin_pt()
                    + grid_widths
                        .iter()
                        .take(column as usize)
                        .map(|value| *value as f64 / 20.0)
                        .sum::<f64>();
                let width = grid_widths
                    .iter()
                    .skip(column as usize)
                    .take(col_span as usize)
                    .map(|value| *value as f64 / 20.0)
                    .sum::<f64>()
                    .max(1.0);
                let row_top = top + row_offsets[row_index];
                let row_end = row_index
                    .saturating_add(cell.row_span.max(1) as usize)
                    .min(row_heights.len());
                let cell_height = (row_offsets[row_end] - row_offsets[row_index]).max(1.0);
                let cell_bbox = rect(x, row_top, width, cell_height, 0);
                let mut content_region_ids = Vec::new();
                let padding_left = cell.padding.left_twips.unwrap_or_default() as f64 / 20.0;
                let padding_top = cell.padding.top_twips.unwrap_or_default() as f64 / 20.0;
                let mut content_y = row_top + padding_top;
                for paragraph in &cell.paragraphs {
                    for (segment, _) in paragraph_segments(paragraph) {
                        let saved_cursor = self.cursor_y;
                        self.cursor_y = content_y;
                        content_region_ids.push(self.add_paragraph_at(
                            &segment,
                            x + padding_left,
                            content_y,
                        ));
                        content_y = self.cursor_y;
                        self.cursor_y = saved_cursor;
                    }
                }
                for nested in &cell.tables {
                    let saved_cursor = self.cursor_y;
                    self.cursor_y = content_y;
                    self.add_table(nested);
                    self.cursor_y = saved_cursor;
                }
                let mut border_evidence = vec!["ooxml".to_string()];
                border_evidence.extend(cell.border_evidence.iter().cloned());
                if let Some(shading) = &cell.shading {
                    border_evidence.push(format!("shading:{shading}"));
                }
                if let Some(v_merge) = &cell.v_merge {
                    border_evidence.push(format!("vMerge:{v_merge}"));
                }
                if let Some(width_type) = &cell.width_type {
                    border_evidence.push(format!("widthType:{width_type}"));
                }
                cells.push(json!({
                    "cellId": format!("{table_id}-r{}-c{}", row_index, column),
                    "row": row_index,
                    "col": column,
                    "rowSpan": cell.row_span,
                    "colSpan": col_span,
                    "bbox": cell_bbox.clone(),
                    "contentRegionIds": content_region_ids,
                    "widthPt": cell.width_twips.map(|value| value as f64 / 20.0),
                    "rowHeightPt": row.height_twips.map(|value| value as f64 / 20.0),
                    "rowHeightRule": row.height_rule,
                    "verticalAlignment": cell.vertical_alignment,
                    "paddingPt": {
                        "top": cell.padding.top_twips.map(|value| value as f64 / 20.0),
                        "right": cell.padding.right_twips.map(|value| value as f64 / 20.0),
                        "bottom": cell.padding.bottom_twips.map(|value| value as f64 / 20.0),
                        "left": cell.padding.left_twips.map(|value| value as f64 / 20.0)
                    },
                    "borderEvidence": border_evidence,
                    "confidence": 0.94,
                    "sourceAnchors": [source_anchor(
                        &self.source_file_id,
                        self.page_index,
                        &format!("{table_id}-r{}-c{}", row_index, column),
                        Some(&cell.path),
                        None,
                        None,
                        Some(cell_bbox),
                        &self.source_hash
                    )]
                }));
                column = column.saturating_add(col_span);
            }
        }
        let table_height = row_offsets.last().copied().unwrap_or(28.0).max(1.0);
        let table_bbox = rect(
            self.left_margin_pt(),
            top,
            table_width.max(self.content_width_pt().min(72.0)),
            table_height,
            0,
        );
        self.tables.push(json!({
            "id": table_id,
            "bbox": table_bbox.clone(),
            "rows": table.rows.len(),
            "cols": grid_widths.len().max(table_column_count(table)),
            "cells": cells,
            "detectionMode": "ooxml",
            "topologyConfidence": if table.grid_widths_twips.is_empty() {0.82} else {0.99},
            "contentConfidence": 0.95,
            "sourceAnchors": [source_anchor(
                &self.source_file_id,
                self.page_index,
                &table.path,
                Some(&table.path),
                None,
                None,
                Some(table_bbox),
                &self.source_hash
            )]
        }));
        self.reading_order.push(format!(
            "docx-p{:03}-table{:04}",
            self.page_index,
            self.tables.len()
        ));
        self.source_order = self.source_order.saturating_add(1);
        self.cursor_y = top + table_height + 8.0;
    }

    fn finish(self) -> Value {
        let page_width = self.section.page_width_twips as f64 / 20.0;
        let page_height = self.section.page_height_twips as f64 / 20.0;
        let text_coverage = if page_width > 0.0 && page_height > 0.0 {
            ((self.glyphs.len() as f64 * 18.0) / (page_width * page_height)).min(1.0)
        } else {
            0.0
        };
        json!({
            "pageIndex": self.page_index,
            "widthPt": page_width,
            "heightPt": page_height,
            "rotation": 0,
            "glyphs": self.glyphs,
            "spans": self.spans,
            "lines": self.lines,
            "regions": self.regions,
            "vectorPaths": [],
            "tables": self.tables,
            "assetIds": [],
            "readingOrder": self.reading_order,
            "quality": {
                "classification": "born_digital",
                "nativeCharacterCount": self.glyphs.len(),
                "unicodeErrorRatio": 0.0,
                "duplicateTextRatio": 0.0,
                "imageCoverageRatio": 0.0,
                "textCoverageRatio": text_coverage,
                "rotationConfidence": 1.0,
                "requiresOcrRegions": [],
                "warnings": self.warnings
            }
        })
    }
}

#[derive(Debug, Clone)]
struct SemanticLineFact {
    order: usize,
    path: String,
    text: String,
}

#[derive(Debug, Clone)]
struct SemanticLineBinding {
    path: String,
    text: String,
    line_text: String,
}

fn semantic_line_facts(model: &DocxDocumentModel) -> Vec<SemanticLineFact> {
    fn push_table(table: &DocxTable, base_order: usize, output: &mut Vec<SemanticLineFact>) {
        let mut offset = 0_usize;
        for row in &table.rows {
            for cell in &row.cells {
                for paragraph in &cell.paragraphs {
                    output.push(SemanticLineFact {
                        order: base_order.saturating_mul(1_000).saturating_add(offset),
                        path: paragraph.path.clone(),
                        text: paragraph.display_text(),
                    });
                    offset = offset.saturating_add(1);
                }
                for nested in &cell.tables {
                    push_table(nested, base_order, output);
                }
            }
        }
    }
    let mut facts = Vec::new();
    for block in &model.blocks {
        match block {
            DocxBlock::Paragraph(paragraph) => facts.push(SemanticLineFact {
                order: paragraph.source_order.saturating_mul(1_000),
                path: paragraph.path.clone(),
                text: paragraph.display_text(),
            }),
            DocxBlock::Table(table) => push_table(table, table.source_order, &mut facts),
        }
    }
    for (index, text_box) in model.text_boxes.iter().enumerate() {
        let source_order = text_box
            .source_paragraph_path
            .as_deref()
            .and_then(|path| {
                model.blocks.iter().find_map(|block| match block {
                    DocxBlock::Paragraph(paragraph) if paragraph.path == path => {
                        Some(paragraph.source_order)
                    }
                    _ => None,
                })
            })
            .unwrap_or(usize::MAX / 2);
        for (paragraph_index, paragraph) in text_box.paragraphs.iter().enumerate() {
            facts.push(SemanticLineFact {
                order: source_order
                    .saturating_mul(1_000)
                    .saturating_add(500 + index * 20 + paragraph_index),
                path: paragraph.path.clone(),
                text: paragraph.display_text(),
            });
        }
    }
    facts.retain(|fact| !normalize_render_text(&fact.text).is_empty());
    facts.sort_by_key(|fact| fact.order);
    facts
}

fn normalize_render_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn render_match_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn semantic_line_segments(value: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut spaces = 0_usize;
    let mut boundary = false;

    let flush = |segments: &mut Vec<String>, current: &mut String| {
        let value = current.trim();
        if !value.is_empty() {
            segments.push(value.to_string());
        }
        current.clear();
    };

    for character in value.chars() {
        if character == '\t' {
            boundary = true;
            spaces = 0;
        } else if character.is_whitespace() {
            spaces = spaces.saturating_add(1);
        } else {
            if boundary || spaces >= 2 {
                flush(&mut segments, &mut current);
            } else if spaces == 1 && !current.is_empty() {
                current.push(' ');
            }
            boundary = false;
            spaces = 0;
            current.push(character);
        }
    }
    flush(&mut segments, &mut current);
    segments
}

fn matched_semantic_line_text(semantic_text: &str, rendered_text: &str) -> String {
    let rendered_key = render_match_key(rendered_text);
    let segments = semantic_line_segments(semantic_text);
    segments
        .iter()
        .find(|segment| render_match_key(segment) == rendered_key)
        .or_else(|| {
            segments.iter().find(|segment| {
                let segment_key = render_match_key(segment);
                !segment_key.is_empty()
                    && (segment_key.contains(&rendered_key) || rendered_key.contains(&segment_key))
            })
        })
        .cloned()
        .unwrap_or_else(|| semantic_text.to_string())
}

fn bind_rendered_pages_to_ooxml(
    pages: &mut [Value],
    model: &DocxDocumentModel,
    source: &SourceFile,
) {
    let semantic = semantic_line_facts(model);
    let mut semantic_cursor = 0_usize;
    // A tab/space-aligned OOXML paragraph can render as one visual line per column.
    let mut active_split_match = None;
    for page in pages {
        if let Some(object) = page.as_object_mut() {
            object.insert("assetIds".to_string(), json!([]));
        }
        let line_bindings = page
            .get("lines")
            .and_then(Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(|line| {
                        let id = line.get("id")?.as_str()?.to_string();
                        let raw_text = line.get("text")?.as_str()?;
                        let text = normalize_render_text(raw_text);
                        let match_index = semantic_match_index(
                            &semantic,
                            &text,
                            &mut semantic_cursor,
                            &mut active_split_match,
                        );
                        let binding = match_index.map(|index| SemanticLineBinding {
                            path: semantic[index].path.clone(),
                            text: semantic[index].text.clone(),
                            line_text: matched_semantic_line_text(&semantic[index].text, raw_text),
                        });
                        Some((id, binding))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();

        rewrite_rendered_page_anchors(page, &line_bindings, source);
    }
}

fn semantic_match_index(
    semantic: &[SemanticLineFact],
    rendered_text: &str,
    semantic_cursor: &mut usize,
    active_split_match: &mut Option<usize>,
) -> Option<usize> {
    if rendered_text.is_empty() {
        return None;
    }
    if let Some(index) = *active_split_match {
        let candidate = render_match_key(&semantic[index].text);
        let rendered = render_match_key(rendered_text);
        if candidate.contains(&rendered) || rendered.contains(&candidate) {
            if candidate == rendered || rendered.contains(&candidate) {
                *semantic_cursor = (*semantic_cursor).max(index.saturating_add(1));
                *active_split_match = None;
            }
            return Some(index);
        }
        *semantic_cursor = (*semantic_cursor).max(index.saturating_add(1));
        *active_split_match = None;
    }

    let match_index = semantic
        .iter()
        .enumerate()
        .skip(*semantic_cursor)
        .find(|(_, fact)| {
            let candidate = render_match_key(&fact.text);
            let rendered = render_match_key(rendered_text);
            candidate == rendered
                || (!candidate.is_empty()
                    && (candidate.contains(&rendered) || rendered.contains(&candidate)))
        })
        .map(|(index, _)| index)
        .or_else(|| {
            semantic
                .iter()
                .enumerate()
                .find(|(_, fact)| render_match_key(&fact.text) == render_match_key(rendered_text))
                .map(|(index, _)| index)
        })?;
    let candidate = render_match_key(&semantic[match_index].text);
    let rendered = render_match_key(rendered_text);
    if candidate == rendered || rendered.contains(&candidate) {
        *semantic_cursor = (*semantic_cursor).max(match_index.saturating_add(1));
    } else {
        *semantic_cursor = match_index;
        *active_split_match = Some(match_index);
    }
    Some(match_index)
}

fn rewrite_rendered_page_anchors(
    page: &mut Value,
    line_bindings: &BTreeMap<String, Option<SemanticLineBinding>>,
    source: &SourceFile,
) {
    let span_paths = page
        .get("lines")
        .and_then(Value::as_array)
        .map(|lines| {
            lines
                .iter()
                .flat_map(|line| {
                    let path = line
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(|id| line_bindings.get(id))
                        .cloned()
                        .flatten();
                    line.get("spanIds")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(move |id| (id.to_string(), path.clone()))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let glyph_paths = page
        .get("spans")
        .and_then(Value::as_array)
        .map(|spans| {
            spans
                .iter()
                .flat_map(|span| {
                    let path = span
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(|id| span_paths.get(id))
                        .cloned()
                        .flatten();
                    span.get("glyphIds")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(move |id| (id.to_string(), path.clone()))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    for (field, bindings, singular) in [
        ("glyphs", &glyph_paths, true),
        ("spans", &span_paths, false),
        ("lines", line_bindings, false),
    ] {
        if let Some(items) = page.get_mut(field).and_then(Value::as_array_mut) {
            for item in items {
                let path = item
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| bindings.get(id))
                    .cloned()
                    .flatten();
                if field == "lines" {
                    if let (Some(binding), Some(object)) = (path.as_ref(), item.as_object_mut()) {
                        object.insert("text".to_string(), json!(binding.line_text));
                    }
                }
                if singular {
                    if let Some(anchor) = item.get_mut("sourceAnchor") {
                        rewrite_render_anchor(anchor, path.as_ref(), source);
                    }
                } else if let Some(anchors) =
                    item.get_mut("sourceAnchors").and_then(Value::as_array_mut)
                {
                    for anchor in anchors {
                        rewrite_render_anchor(anchor, path.as_ref(), source);
                    }
                }
            }
        }
    }
    if let Some(regions) = page.get_mut("regions").and_then(Value::as_array_mut) {
        for region in regions {
            let fallback_binding = region
                .get("childLineIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .find_map(|id| line_bindings.get(id).cloned().flatten());
            if let Some(object) = region.as_object_mut() {
                object.insert("childObjectIds".to_string(), json!([]));
            }
            if let Some(anchors) = region
                .get_mut("sourceAnchors")
                .and_then(Value::as_array_mut)
            {
                for anchor in anchors {
                    let binding = anchor
                        .get("nodeIds")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .find_map(|id| glyph_paths.get(id).cloned().flatten())
                        .or_else(|| fallback_binding.clone());
                    rewrite_render_anchor(anchor, binding.as_ref(), source);
                }
            }
        }
    }
    for field in ["vectorPaths", "tables"] {
        if let Some(items) = page.get_mut(field).and_then(Value::as_array_mut) {
            for item in items {
                if let Some(anchor) = item.get_mut("sourceAnchor") {
                    rewrite_render_anchor(anchor, None, source);
                }
                if let Some(anchors) = item.get_mut("sourceAnchors").and_then(Value::as_array_mut) {
                    for anchor in anchors {
                        rewrite_render_anchor(anchor, None, source);
                    }
                }
            }
        }
    }
}

fn rewrite_render_anchor(
    anchor: &mut Value,
    binding: Option<&SemanticLineBinding>,
    source: &SourceFile,
) {
    if let Some(object) = anchor.as_object_mut() {
        object.insert(
            "extractionMode".to_string(),
            json!("docx_rendered_fallback"),
        );
        object.insert("sourceHash".to_string(), json!(source.sha256));
        object.insert("sourceFileId".to_string(), json!(source.file_id));
        if let Some(binding) = binding {
            object.insert("ooxmlPath".to_string(), json!(binding.path));
            let semantic_variant = json!({
                "text": binding.text,
                "extractionMode": "docx_ooxml",
                "confidence": 1.0,
                "provider": "rust-parser:docx:ooxml:shadow",
                "providerVersion": "phase3-c002-c009",
                "nodeIds": []
            });
            let variants = object
                .entry("variants".to_string())
                .or_insert_with(|| json!([]));
            if let Some(variants) = variants.as_array_mut() {
                let duplicate = variants.iter().any(|variant| {
                    variant.get("extractionMode") == semantic_variant.get("extractionMode")
                        && variant.get("text") == semantic_variant.get("text")
                });
                if !duplicate {
                    variants.push(semantic_variant);
                }
            }
        }
    }
}

fn attach_drawing_assets_to_regions(
    pages: &mut [Value],
    model: &DocxDocumentModel,
    drawing_asset_ids: &BTreeMap<String, String>,
) {
    for drawing in &model.drawings {
        let Some(asset_id) = drawing_asset_ids.get(&drawing.path) else {
            continue;
        };
        let paragraph_path = drawing
            .source_paragraph_path
            .as_deref()
            .unwrap_or(drawing.path.as_str());
        for page in pages.iter_mut() {
            let Some(regions) = page.get_mut("regions").and_then(Value::as_array_mut) else {
                continue;
            };
            for region in regions {
                let matches_path = region
                    .get("sourceAnchors")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|anchor| {
                        anchor
                            .get("ooxmlPath")
                            .and_then(Value::as_str)
                            .is_some_and(|path| path.starts_with(paragraph_path))
                    });
                if matches_path {
                    if let Some(ids) = region
                        .get_mut("childObjectIds")
                        .and_then(Value::as_array_mut)
                    {
                        if !ids.iter().any(|value| value == asset_id) {
                            ids.push(json!(asset_id));
                        }
                    }
                }
            }
        }
    }
}

fn build_pages(model: &DocxDocumentModel, source: &SourceFile) -> Vec<Value> {
    enum LayoutEvent<'a> {
        Block(&'a DocxBlock),
        TextBox(&'a crate::docx_ingest::model::DocxTextBox),
    }
    let default_section = model
        .sections
        .first()
        .cloned()
        .unwrap_or_else(|| DocxSection::default_for(0));
    let mut events = model
        .blocks
        .iter()
        .map(|block| {
            let order = match block {
                DocxBlock::Paragraph(paragraph) => paragraph.source_order,
                DocxBlock::Table(table) => table.source_order,
            };
            (order.saturating_mul(10), LayoutEvent::Block(block))
        })
        .collect::<Vec<_>>();
    for text_box in &model.text_boxes {
        let order = text_box
            .source_paragraph_path
            .as_deref()
            .and_then(|path| {
                model.blocks.iter().find_map(|block| match block {
                    DocxBlock::Paragraph(paragraph) if paragraph.path == path => {
                        Some(paragraph.source_order)
                    }
                    _ => None,
                })
            })
            .unwrap_or(usize::MAX / 20);
        events.push((
            order.saturating_mul(10).saturating_add(1),
            LayoutEvent::TextBox(text_box),
        ));
    }
    events.sort_by_key(|(order, _)| *order);

    let mut builders = vec![DocxPageBuilder::new(0, default_section.clone(), source)];
    let mut current_page = 0_usize;
    let mut current_section = default_section.index;
    for (_, event) in events {
        match event {
            LayoutEvent::Block(DocxBlock::Paragraph(paragraph)) => {
                let section = paragraph
                    .section
                    .clone()
                    .unwrap_or_else(|| default_section.clone());
                let page_break_before = paragraph
                    .resolved_style
                    .as_ref()
                    .and_then(|style| style.paragraph.page_break_before)
                    .unwrap_or(false);
                let section_starts_page = section.index != current_section
                    && !matches!(section.break_type.as_deref(), Some("continuous"));
                if (page_break_before || section_starts_page) && !builders[current_page].is_empty()
                {
                    current_page += 1;
                    builders.push(DocxPageBuilder::new(
                        current_page as u32,
                        section.clone(),
                        source,
                    ));
                }
                current_section = section.index;
                for (segment, page_break_after) in paragraph_segments(paragraph) {
                    if segment.has_visible_text() || segment.runs.is_empty() {
                        builders[current_page].add_paragraph(&segment);
                    }
                    if page_break_after {
                        current_page += 1;
                        builders.push(DocxPageBuilder::new(
                            current_page as u32,
                            section.clone(),
                            source,
                        ));
                    }
                }
            }
            LayoutEvent::Block(DocxBlock::Table(table)) => builders[current_page].add_table(table),
            LayoutEvent::TextBox(text_box) => {
                let origin_x = text_box
                    .x_emu
                    .map(|value| builders[current_page].left_margin_pt() + value as f64 / 12_700.0);
                let origin_y = text_box.y_emu.map(|value| value as f64 / 12_700.0);
                for paragraph in &text_box.paragraphs {
                    for (segment, page_break_after) in paragraph_segments(paragraph) {
                        if let (Some(x), Some(y)) = (origin_x, origin_y) {
                            builders[current_page].add_paragraph_at(&segment, x, y);
                        } else {
                            builders[current_page].add_paragraph(&segment);
                        }
                        if page_break_after {
                            current_page += 1;
                            builders.push(DocxPageBuilder::new(
                                current_page as u32,
                                segment
                                    .section
                                    .clone()
                                    .unwrap_or_else(|| default_section.clone()),
                                source,
                            ));
                        }
                    }
                }
            }
        }
    }
    while builders.len() > 1 && builders.last().is_some_and(DocxPageBuilder::is_empty) {
        builders.pop();
    }
    builders.into_iter().map(DocxPageBuilder::finish).collect()
}

fn paragraph_segments(paragraph: &DocxParagraph) -> Vec<(DocxParagraph, bool)> {
    let mut segments = Vec::new();
    let mut current = paragraph.clone();
    current.runs.clear();
    let mut first_segment = true;
    for run in &paragraph.runs {
        if run.kind == crate::docx_ingest::model::DocxRunKind::Break {
            let page_break = run.break_type.as_deref() == Some("page");
            segments.push((current, page_break));
            current = paragraph.clone();
            current.runs.clear();
            current.numbering_label = None;
            first_segment = false;
        } else {
            current.runs.push(run.clone());
        }
    }
    if !first_segment {
        current.numbering_label = None;
    }
    segments.push((current, false));
    segments
}

fn table_column_count(table: &DocxTable) -> usize {
    table
        .rows
        .iter()
        .map(|row| {
            row.grid_before as usize
                + row
                    .cells
                    .iter()
                    .map(|cell| cell.grid_span.max(1) as usize)
                    .sum::<usize>()
                + row.grid_after as usize
        })
        .max()
        .unwrap_or(1)
}

fn paragraph_region_kind(paragraph: &DocxParagraph) -> &'static str {
    if paragraph
        .resolved_style
        .as_ref()
        .and_then(|style| style.outline_level)
        .is_some()
    {
        "title"
    } else if paragraph.resolved_numbering.is_some() {
        "list"
    } else if paragraph
        .runs
        .iter()
        .any(|run| run.kind == crate::docx_ingest::model::DocxRunKind::Drawing)
    {
        "figure"
    } else {
        "text"
    }
}

fn char_width(character: char, formatting: &DocxRunFormatting) -> f64 {
    let font_size = formatting
        .font_size_half_points
        .map(|value| value as f64 / 2.0)
        .unwrap_or(11.0)
        .max(6.0);
    if character == '\t' {
        font_size * 2.5
    } else if character.is_whitespace() {
        font_size * 0.32
    } else {
        font_size * if character.is_ascii() { 0.52 } else { 0.9 }
    }
}

fn text_style(formatting: &DocxRunFormatting) -> Value {
    json!({
        "fontName": formatting.font_name,
        "fontSizePt": formatting.font_size_half_points.map(|value| value as f64 / 2.0),
        "weight": formatting.bold.map(|bold| if bold {700} else {400}),
        "bold": formatting.bold,
        "italic": formatting.italic,
        "underline": formatting.underline.as_ref().map(|value| !value.eq_ignore_ascii_case("none")),
        "strike": formatting.strike,
        "color": formatting.color,
        "language": formatting.language,
        "superscript": formatting.vertical_align.as_deref().map(|value| value.eq_ignore_ascii_case("superscript")),
        "subscript": formatting.vertical_align.as_deref().map(|value| value.eq_ignore_ascii_case("subscript"))
    })
}

fn whitespace_origin(character: Option<char>) -> &'static str {
    if character.is_some_and(char::is_whitespace) {
        "source"
    } else {
        "none"
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64, page_rotation: u16) -> Value {
    json!({
        "x": clean_number(x),
        "y": clean_number(y),
        "width": clean_number(width.max(0.01)),
        "height": clean_number(height.max(0.01)),
        "unit": "pt",
        "origin": "top-left",
        "pageRotation": page_rotation
    })
}

fn source_anchor(
    source_file_id: &str,
    page_index: u32,
    node_id: &str,
    ooxml_path: Option<&str>,
    relationship_id: Option<&str>,
    char_range: Option<Value>,
    bbox: Option<Value>,
    source_hash: &str,
) -> Value {
    json!({
        "sourceFileId": source_file_id,
        "pageIndex": page_index,
        "nodeIds": [node_id],
        "bbox": bbox,
        "charRange": char_range,
        "ooxmlPath": ooxml_path,
        "relationshipId": relationship_id,
        "extractionMode": "docx_ooxml",
        "sourceHash": source_hash
    })
}

fn clean_number(value: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn coverage_ledger_from_pages(pages: &[Value]) -> Vec<Value> {
    let mut entries = Vec::new();
    for page in pages {
        for field in ["regions", "lines", "tables"] {
            if let Some(items) = page.get(field).and_then(Value::as_array) {
                for item in items {
                    if let Some(id) = item
                        .get("id")
                        .or_else(|| item.get("cellId"))
                        .and_then(Value::as_str)
                    {
                        entries.push(json!({
                            "sourceNodeId": id,
                            "disposition": "unassigned",
                            "targetIds": [],
                            "reason": "DOCX semantic shadow node awaits authoring-layer assignment"
                        }));
                    }
                }
            }
        }
    }
    entries
}

fn extract_docx_assets(
    package: &DocxPackage,
    source: &SourceFile,
    model: &DocxDocumentModel,
    artifact_root: Option<&Path>,
) -> CommandResult<(
    Vec<Value>,
    BTreeMap<u32, Vec<String>>,
    Vec<String>,
    BTreeMap<String, String>,
)> {
    let mut assets = Vec::new();
    let mut page_asset_ids = BTreeMap::<u32, Vec<String>>::new();
    let mut warnings = Vec::new();
    let mut seen_targets = BTreeMap::<String, String>::new();
    let mut drawing_asset_ids = BTreeMap::<String, String>::new();
    for drawing in &model.drawings {
        let mut selected_asset = None::<(u8, String)>;
        for target in &drawing.relationship_targets {
            let Some(bytes) = package.part_bytes(target) else {
                warnings.push(format!(
                    "DOCX_DRAWING_TARGET_MISSING:{}:{}",
                    drawing.path, target
                ));
                continue;
            };
            let asset_id = if let Some(asset_id) = seen_targets.get(target) {
                asset_id.clone()
            } else {
                let hash = crate::hash_bytes(bytes);
                let asset_id = format!("docx-asset-{}", &hash[..hash.len().min(16)]);
                let extension = asset_extension(target, package.content_type(target));
                let relative_path = format!("assets/shadow/docx/{asset_id}.{extension}");
                if let Some(root) = artifact_root {
                    write_shadow_asset(root, &relative_path, bytes)?;
                } else {
                    warnings.push(format!(
                        "DOCX_ASSET_NOT_PERSISTED:{}:{}",
                        drawing.path, target
                    ));
                }
                let page_index = page_for_source_path(model, &drawing.path);
                let kind = asset_kind_for_target(target);
                let mime = package
                    .content_type(target)
                    .map(str::to_string)
                    .unwrap_or_else(|| mime_for_target(target).to_string());
                let relationship_id = package
                    .relationships_for(&model.main_document_part)
                    .iter()
                    .find(|relationship| {
                        drawing.relationship_ids.contains(&relationship.id)
                            && relationship.resolved_target.as_deref() == Some(target.as_str())
                    })
                    .map(|relationship| relationship.id.as_str());
                assets.push(json!({
                    "assetId": asset_id,
                    "kind": kind,
                    "mime": mime,
                    "relativePath": relative_path,
                    "sha256": hash,
                    "byteLength": bytes.len() as u64,
                    "extractionMode": "docx_media",
                    "altText": drawing.alt_text.clone().or_else(|| Some(format!("DOCX embedded asset {target}"))),
                    "decorative": false,
                    "sourceAnchor": source_anchor(
                        &source.file_id,
                        page_index,
                        &asset_id,
                        Some(&drawing.path),
                        relationship_id,
                        None,
                        None,
                        &source.sha256
                    )
                }));
                seen_targets.insert(target.to_string(), asset_id.clone());
                asset_id
            };
            let page_index = page_for_source_path(model, &drawing.path);
            let ids = page_asset_ids.entry(page_index).or_default();
            if !ids.iter().any(|item| item == &asset_id) {
                ids.push(asset_id.clone());
            }
            let priority = u8::from(target.starts_with("word/media/"));
            if selected_asset
                .as_ref()
                .is_none_or(|(selected_priority, _)| priority > *selected_priority)
            {
                selected_asset = Some((priority, asset_id));
            }
        }
        if let Some((_, asset_id)) = selected_asset {
            drawing_asset_ids.insert(drawing.path.clone(), asset_id);
        }
    }

    for entry in package.entries() {
        if !entry.path.starts_with("word/media/") || entry.is_directory {
            continue;
        }
        if seen_targets.contains_key(&entry.path) {
            continue;
        }
        let Some(bytes) = package.part_bytes(&entry.path) else {
            continue;
        };
        let hash = crate::hash_bytes(bytes);
        let asset_id = format!("docx-asset-{}", &hash[..hash.len().min(16)]);
        let extension = asset_extension(&entry.path, package.content_type(&entry.path));
        let relative_path = format!("assets/shadow/docx/{asset_id}.{extension}");
        if let Some(root) = artifact_root {
            write_shadow_asset(root, &relative_path, bytes)?;
        } else {
            warnings.push(format!("DOCX_ASSET_NOT_PERSISTED:{}", entry.path));
        }
        assets.push(json!({
            "assetId": asset_id,
            "kind": "raster_image",
            "mime": package.content_type(&entry.path).unwrap_or_else(|| mime_for_target(&entry.path)),
            "relativePath": relative_path,
            "sha256": hash,
            "byteLength": bytes.len() as u64,
            "extractionMode": "docx_media",
            "altText": format!("Unreferenced DOCX media asset {}", entry.path),
            "decorative": false,
            "sourceAnchor": source_anchor(
                &source.file_id,
                0,
                &asset_id,
                Some(&format!("/{}", entry.path)),
                None,
                None,
                None,
                &source.sha256
            )
        }));
        page_asset_ids.entry(0).or_default().push(asset_id);
        warnings.push(format!("DOCX_UNREFERENCED_MEDIA_RETAINED:{}", entry.path));
    }
    Ok((assets, page_asset_ids, warnings, drawing_asset_ids))
}

fn asset_kind_for_target(target: &str) -> &'static str {
    if target.starts_with("word/charts/") {
        "chart"
    } else if target.starts_with("word/diagrams/") {
        "diagram"
    } else if target.starts_with("word/media/") {
        "raster_image"
    } else {
        "vector_render"
    }
}

fn mime_for_target(target: &str) -> &'static str {
    match target
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

fn asset_extension(target: &str, mime: Option<&str>) -> String {
    target
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .or_else(|| mime.and_then(|mime| mime.rsplit_once('/').map(|(_, extension)| extension)))
        .unwrap_or("bin")
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn page_for_source_path(model: &DocxDocumentModel, source_path: &str) -> u32 {
    let mut current_page = 0_u32;
    for block in &model.blocks {
        match block {
            DocxBlock::Paragraph(paragraph) => {
                if source_path.starts_with(&paragraph.path) {
                    return paragraph
                        .section
                        .as_ref()
                        .map(|section| section.index)
                        .unwrap_or(current_page);
                }
                current_page = paragraph
                    .section
                    .as_ref()
                    .map(|section| section.index)
                    .unwrap_or(current_page);
            }
            DocxBlock::Table(table) => {
                if source_path.starts_with(&table.path) {
                    return current_page;
                }
            }
        }
    }
    0
}

fn write_shadow_asset(root: &Path, relative_path: &str, bytes: &[u8]) -> CommandResult<()> {
    let target = root.join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    let parent = target
        .parent()
        .ok_or_else(|| format!("docx_asset_parent_missing:{}", target.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("docx_asset_dir_create_failed:{error}"))?;
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("docx_asset_file_name_missing:{}", target.display()))?;
    let temporary = parent.join(format!(
        ".{file_name}.tmp-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> CommandResult<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("docx_asset_temp_create_failed:{error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("docx_asset_temp_write_failed:{error}"))?;
        file.flush()
            .map_err(|error| format!("docx_asset_temp_flush_failed:{error}"))?;
        file.sync_all()
            .map_err(|error| format!("docx_asset_temp_sync_failed:{error}"))?;
        match fs::rename(&temporary, &target) {
            Ok(()) => Ok(()),
            Err(_)
                if target.exists()
                    && fs::read(&target)
                        .map(|value| value == bytes)
                        .unwrap_or(false) =>
            {
                let _ = fs::remove_file(&temporary);
                Ok(())
            }
            Err(error) => Err(format!("docx_asset_replace_failed:{error}")),
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        commit_docx_shadow_bundle_with_hook, extract_docx_facts_shadow,
        extract_docx_facts_shadow_with_renderer, paragraph_segments,
        write_docx_facts_shadow_with_v1, SHADOW_COMPARE_FILE,
    };
    use crate::docx_ingest::{
        model::{DocxParagraph, DocxRun, DocxRunKind},
        open_docx,
        render_fallback::DocxRenderAssistResult,
        DocxPackageLimits,
    };
    use crate::{hash_bytes, ImportJob, IssueCounts, JobStatus, SourceFile, WorkflowStep};
    use chrono::Utc;
    use std::{
        fs,
        io::{Cursor, Write},
        path::PathBuf,
    };
    use uuid::Uuid;
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    const SHADOW_ARTIFACT_FILE: &str = "document-ir-v2.shadow.json";

    fn package_bytes(external_image: bool) -> Vec<u8> {
        let content_types = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/></Types>"#;
        let relationships = if external_image {
            br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.invalid/missing.png" TargetMode="External"/></Relationships>"#.to_vec()
        } else {
            br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/></Relationships>"#.to_vec()
        };
        let document = br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p><w:pPr><w:cols w:num="2"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t xml:space="preserve"> A </w:t><w:tab/><w:br w:type="page"/></w:r><w:r><w:drawing><wp:inline><wp:extent cx="914400" cy="457200"/><wp:docPr id="1" descr="embedded figure"/><a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rId9"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p><w:tbl><w:tblPr><w:tblCellMar><w:top w:w="80" w:type="dxa"/><w:start w:w="100" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="720"/><w:gridCol w:w="720"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="640" w:hRule="atLeast"/></w:trPr><w:tc><w:p/></w:tc><w:tc><w:tcPr><w:tcW w:w="720" w:type="dxa"/><w:vAlign w:val="center"/></w:tcPr><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:cols w:num="2" w:space="360" w:sep="1"/></w:sectPr></w:body></w:document>"#;
        let styles = br#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Heading"><w:name w:val="Heading 1"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/></w:rPr></w:style></w:styles>"#;
        let numbering = br#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#;
        let mut output = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut output);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (path, bytes) in [
            ("[Content_Types].xml", content_types.as_slice()),
            ("word/document.xml", document.as_slice()),
            ("word/_rels/document.xml.rels", relationships.as_slice()),
            ("word/styles.xml", styles.as_slice()),
            ("word/numbering.xml", numbering.as_slice()),
        ] {
            writer.start_file(path, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        if !external_image {
            writer.start_file("word/media/image1.png", options).unwrap();
            writer.write_all(b"PNG-PHASE3").unwrap();
        }
        writer.finish().unwrap();
        output.into_inner()
    }

    fn job_and_source(bytes: &[u8]) -> (ImportJob, SourceFile) {
        let now = Utc::now();
        let job_id = format!("phase3-docx-test-{}", Uuid::new_v4().simple());
        let source = SourceFile {
            file_id: "source-docx-phase3".to_string(),
            original_name: "phase3-rich.docx".to_string(),
            stored_name: "phase3-rich.docx".to_string(),
            file_type: "docx".to_string(),
            sha256: hash_bytes(bytes),
            size_bytes: bytes.len() as u64,
            role: "MainQuestion".to_string(),
            imported_at: now,
        };
        let job = ImportJob {
            job_id,
            title: "phase3".to_string(),
            status: JobStatus::Working,
            category: None,
            frequency: None,
            tags: Vec::new(),
            source_files: vec![source.clone()],
            active_llm_profile_id: None,
            created_at: now,
            updated_at: now,
            current_step: WorkflowStep::Upload,
            issue_counts: IssueCounts::default(),
        };
        (job, source)
    }

    #[test]
    fn rich_docx_shadow_preserves_structure_assets_and_strict_schema() {
        let bytes = package_bytes(false);
        let (job, source) = job_and_source(&bytes);
        let temp = make_temp_dir();
        let input = temp.join("rich.docx");
        let output = temp.join("document-ir-v2.shadow.json");
        fs::write(&input, &bytes).unwrap();
        let value = write_docx_facts_shadow_with_v1(&job, &source, &input, &output, None).unwrap();
        let pages = value["pages"].as_array().unwrap();
        assert!(
            pages
                .iter()
                .map(|page| page["glyphs"].as_array().unwrap().len())
                .sum::<usize>()
                >= 4
        );
        let table = pages
            .iter()
            .flat_map(|page| page["tables"].as_array().unwrap())
            .next()
            .unwrap();
        assert_eq!(table["cells"].as_array().unwrap().len(), 2);
        assert_eq!(table["cells"][1]["widthPt"], 36.0);
        assert_eq!(table["cells"][1]["rowHeightPt"], 32.0);
        assert_eq!(table["cells"][1]["rowHeightRule"], "atLeast");
        assert_eq!(table["cells"][1]["verticalAlignment"], "center");
        assert_eq!(table["cells"][1]["paddingPt"]["top"], 4.0);
        assert_eq!(table["cells"][1]["paddingPt"]["left"], 5.0);
        assert_eq!(
            value["parser"]["options"]["ooxml"]["numberingFacts"][0]["renderedLabel"],
            "1."
        );
        assert_eq!(pages[0]["lines"][0]["text"], "1.\t A \t");
        assert_eq!(value["assets"].as_array().unwrap().len(), 1);
        assert_eq!(
            value["parser"]["options"]["renderAssist"]["mode"],
            "semantic-only"
        );
        assert!(temp
            .join("assets/shadow/docx")
            .read_dir()
            .unwrap()
            .next()
            .is_some());
        assert!(output.exists());
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn failed_docx_shadow_attempt_preserves_the_previous_complete_bundle() {
        let invalid = b"not-a-docx";
        let (job, source) = job_and_source(invalid);
        let temp = make_temp_dir();
        let input = temp.join("invalid.docx");
        let output = temp.join("document-ir-v2.shadow.json");
        let compare = temp.join("document-ir-v2.shadow.compare.json");
        let assets = temp.join("assets").join("shadow").join("docx");
        fs::create_dir_all(&assets).unwrap();
        fs::write(&input, invalid).unwrap();
        fs::write(&output, b"old artifact").unwrap();
        fs::write(&compare, b"old compare").unwrap();
        fs::write(assets.join("old.bin"), b"old asset").unwrap();

        assert!(write_docx_facts_shadow_with_v1(&job, &source, &input, &output, None).is_err());
        assert_eq!(fs::read(&output).unwrap(), b"old artifact");
        assert_eq!(fs::read(&compare).unwrap(), b"old compare");
        assert_eq!(fs::read(assets.join("old.bin")).unwrap(), b"old asset");
        assert!(!fs::read_dir(&temp).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".docx-shadow-txn-")));
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn docx_shadow_mid_commit_failure_restores_previous_bytes() {
        let root = make_temp_dir();
        let staging = root.join(".docx-shadow-txn-fixture");
        let output = root.join(SHADOW_ARTIFACT_FILE);
        let compare = root.join(SHADOW_COMPARE_FILE);
        let assets = root.join("assets").join("shadow").join("docx");
        fs::create_dir_all(&assets).unwrap();
        fs::create_dir_all(staging.join("assets").join("shadow").join("docx")).unwrap();
        fs::write(&output, b"old artifact\0\xff").unwrap();
        fs::write(&compare, b"old compare\r\n").unwrap();
        fs::write(assets.join("old.bin"), b"old asset\0\x01").unwrap();
        fs::write(staging.join(SHADOW_ARTIFACT_FILE), b"new artifact").unwrap();
        fs::write(staging.join(SHADOW_COMPARE_FILE), b"new compare").unwrap();
        fs::write(
            staging
                .join("assets")
                .join("shadow")
                .join("docx")
                .join("new.bin"),
            b"new asset",
        )
        .unwrap();

        let error = commit_docx_shadow_bundle_with_hook(
            &staging,
            &output,
            |index, _source, _target, _backup_root| {
                if index == 1 {
                    Err("phase3 injected commit failure".to_string())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert!(error.starts_with("docx_shadow_commit_hook_failed:index=1:"));
        assert!(!error.contains("DOCX_SHADOW_ROLLBACK_FAILED"));
        assert_eq!(fs::read(&output).unwrap(), b"old artifact\0\xff");
        assert_eq!(fs::read(&compare).unwrap(), b"old compare\r\n");
        assert_eq!(
            fs::read(assets.join("old.bin")).unwrap(),
            b"old asset\0\x01"
        );
        assert!(!assets.join("new.bin").exists());
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".docx-shadow-backup-")
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn docx_shadow_rollback_failure_is_fail_closed_and_preserves_backup_root() {
        let root = make_temp_dir();
        let staging = root.join(".docx-shadow-txn-fixture");
        let output = root.join(SHADOW_ARTIFACT_FILE);
        let compare = root.join(SHADOW_COMPARE_FILE);
        let assets = root.join("assets").join("shadow").join("docx");
        fs::create_dir_all(&assets).unwrap();
        fs::create_dir_all(staging.join("assets").join("shadow").join("docx")).unwrap();
        fs::write(&output, b"old artifact").unwrap();
        fs::write(&compare, b"old compare").unwrap();
        fs::write(assets.join("old.bin"), b"old asset").unwrap();
        fs::write(staging.join(SHADOW_ARTIFACT_FILE), b"new artifact").unwrap();
        fs::write(staging.join(SHADOW_COMPARE_FILE), b"new compare").unwrap();
        fs::write(
            staging
                .join("assets")
                .join("shadow")
                .join("docx")
                .join("new.bin"),
            b"new asset",
        )
        .unwrap();

        let error = commit_docx_shadow_bundle_with_hook(
            &staging,
            &output,
            |index, _source, _target, backup_root| {
                if index == 1 {
                    fs::rename(
                        backup_root.join("artifact.json"),
                        backup_root.join("artifact.preserved.json"),
                    )
                    .unwrap();
                    Err("phase3 injected rollback failure".to_string())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert!(error.contains("DOCX_SHADOW_ROLLBACK_FAILED"));
        assert!(error.contains("backup_preserved="));
        let backup_root = fs::read_dir(&root)
            .unwrap()
            .find_map(|entry| {
                let path = entry.unwrap().path();
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".docx-shadow-backup-")
                    .then_some(path)
            })
            .expect("rollback failure must preserve its backup root");
        assert_eq!(
            fs::read(backup_root.join("artifact.preserved.json")).unwrap(),
            b"old artifact"
        );
        assert!(
            !output.exists(),
            "failed rollback must not leave the new artifact"
        );
        assert_eq!(fs::read(&compare).unwrap(), b"old compare");
        assert_eq!(fs::read(assets.join("old.bin")).unwrap(), b"old asset");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn paragraph_line_and_page_breaks_create_distinct_segments_without_repeating_numbering() {
        let paragraph = DocxParagraph {
            numbering_label: Some("4.".to_string()),
            runs: vec![
                DocxRun {
                    text: "first".to_string(),
                    ..DocxRun::default()
                },
                DocxRun {
                    kind: DocxRunKind::Break,
                    text: "\n".to_string(),
                    break_type: Some("textWrapping".to_string()),
                    ..DocxRun::default()
                },
                DocxRun {
                    text: "second".to_string(),
                    ..DocxRun::default()
                },
                DocxRun {
                    kind: DocxRunKind::Break,
                    text: "\n".to_string(),
                    break_type: Some("page".to_string()),
                    ..DocxRun::default()
                },
                DocxRun {
                    text: "third".to_string(),
                    ..DocxRun::default()
                },
            ],
            ..DocxParagraph::default()
        };
        let segments = paragraph_segments(&paragraph);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].0.display_text(), "4.\tfirst");
        assert_eq!(segments[1].0.display_text(), "second");
        assert_eq!(segments[2].0.display_text(), "third");
        assert!(!segments[0].1);
        assert!(segments[1].1);
        assert!(!segments[2].1);
    }

    #[test]
    fn external_image_sets_publish_block_and_explicit_issue_code() {
        let bytes = package_bytes(true);
        let (job, source) = job_and_source(&bytes);
        let temp = make_temp_dir();
        let input = temp.join("external.docx");
        fs::write(&input, &bytes).unwrap();
        let value = extract_docx_facts_shadow(&job, &source, &input).unwrap();
        assert_eq!(value["parser"]["options"]["publishBlocked"], true);
        let issues = value["parser"]["options"]["ooxml"]["issues"]
            .as_array()
            .unwrap();
        assert!(issues.iter().any(|issue| {
            issue["code"] == "DOCX_EXTERNAL_ASSET_MISSING" && issue["severity"] == "error"
        }));
        assert!(value["assets"].as_array().unwrap().is_empty());
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn checked_in_phase3_docx_packages_reach_typed_shadow_ir() {
        let fixture_root = phase3_fixture_root();
        for name in [
            "docx-external-image.docx",
            "docx-floating-text-box.docx",
            "docx-section-columns.docx",
            "docx-smartart.docx",
            "docx-table-merged-cells.docx",
        ] {
            let path = fixture_root.join(name);
            let bytes = fs::read(&path).unwrap_or_else(|error| {
                panic!("missing phase3 fixture {}: {error}", path.display())
            });
            let (job, source) = job_and_source(&bytes);
            let value = extract_docx_facts_shadow(&job, &source, &path).unwrap_or_else(|error| {
                panic!("phase3 fixture {} failed: {error}", path.display())
            });
            assert_eq!(value["schemaVersion"], "DocumentIRV2");
            assert!(!value["pages"].as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn render_provider_pdf_geometry_binds_split_option_columns_to_ooxml() {
        let fixture_root = phase3_fixture_root();
        let input = fixture_root.join("render-assisted-two-column-options.docx");
        let provider_output =
            fixture_root.join("render-assisted-two-column-options.provider-output.pdf");
        let bytes = fs::read(&input).expect("render-assist DOCX fixture must exist");
        let (job, source) = job_and_source(&bytes);
        let renderer = |_: &std::path::Path, requested: bool| {
            assert!(
                requested,
                "the injected provider must run in render-assisted mode"
            );
            DocxRenderAssistResult::from_rendered_pdf(
                "phase3-test-provider-output",
                provider_output.clone(),
            )
            .expect("checked-in provider output must be a valid PDF")
        };
        let value =
            extract_docx_facts_shadow_with_renderer(&job, &source, &input, None, true, &renderer)
                .expect("render-assisted fixture must reach typed DocumentIRV2");

        let render = &value["parser"]["options"]["renderAssist"];
        assert_eq!(render["provider"], "phase3-test-provider-output");
        assert_eq!(render["geometryAuthority"], "render-assisted");
        let page = &value["pages"][0];
        let lines = page["lines"].as_array().unwrap();
        let line = |text: &str| {
            lines
                .iter()
                .find(|line| line["text"] == text)
                .unwrap_or_else(|| {
                    let available = lines
                        .iter()
                        .filter_map(|line| line["text"].as_str())
                        .collect::<Vec<_>>();
                    panic!("rendered line missing: {text}; available={available:?}")
                })
        };
        let left_a = line("A Alpine survey");
        let right_b = line("B Coastal archive");
        let left_c = line("C Desert expedition");
        let right_d = line("D Forest census");
        assert!(
            right_b["bbox"]["x"].as_f64().unwrap()
                > left_a["bbox"]["x"].as_f64().unwrap()
                    + left_a["bbox"]["width"].as_f64().unwrap()
                    + 100.0
        );
        assert!(
            right_d["bbox"]["x"].as_f64().unwrap()
                > left_c["bbox"]["x"].as_f64().unwrap()
                    + left_c["bbox"]["width"].as_f64().unwrap()
                    + 100.0
        );

        assert_render_binding(
            left_a,
            "/word/document.xml/body/p[3]",
            "A Alpine survey\tB Coastal archive",
        );
        assert_render_binding(
            right_b,
            "/word/document.xml/body/p[3]",
            "A Alpine survey\tB Coastal archive",
        );
        assert_render_binding(
            left_c,
            "/word/document.xml/body/p[4]",
            "C Desert expedition          D Forest census",
        );
        assert_render_binding(
            right_d,
            "/word/document.xml/body/p[4]",
            "C Desert expedition          D Forest census",
        );

        let columns = page["regions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|region| region["columnIndex"].as_u64())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(columns, std::collections::BTreeSet::from([0, 1]));
        let reading_order = page["readingOrder"].as_array().unwrap();
        assert!(!reading_order.is_empty());
        for option in [left_a, right_b, left_c, right_d] {
            let line_id = option["id"].as_str().unwrap();
            let region = page["regions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|region| {
                    region["childLineIds"]
                        .as_array()
                        .is_some_and(|ids| ids.iter().any(|id| id == line_id))
                })
                .unwrap_or_else(|| panic!("option line {line_id} was not assigned to a region"));
            assert!(reading_order.iter().any(|id| id == &region["id"]));
            assert!(region["sourceAnchors"]
                .as_array()
                .is_some_and(|anchors| anchors.iter().any(|anchor| {
                    anchor["extractionMode"] == "docx_rendered_fallback"
                        && anchor["ooxmlPath"].as_str().is_some()
                })));
        }
    }

    #[test]
    fn adversarial_phase3_docx_fixtures_enforce_the_checked_in_matrix() {
        let fixture_root = phase3_fixture_root();
        let matrix: serde_json::Value = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/docx/phase3-bad-word-fixtures.json"
        ))
        .unwrap();
        let fixtures = matrix["fixtures"].as_array().unwrap();
        assert_eq!(fixtures.len(), 14);
        let mut sources = fixtures
            .iter()
            .map(|fixture| fixture["source"].as_str().unwrap())
            .collect::<Vec<_>>();
        sources.sort_unstable();
        sources.dedup();
        assert_eq!(sources.len(), 14);
        assert!(sources
            .iter()
            .all(|source| fixture_root.join(source).is_file()));

        for (name, code) in [
            ("bad-01-zip-slip-entry.docx", "DOCX_PACKAGE_PATH_UNSAFE"),
            (
                "bad-02-duplicate-casefolded-entry.docx",
                "DOCX_PACKAGE_DUPLICATE_ENTRY",
            ),
            (
                "bad-03-encrypted-entry.docx",
                "DOCX_PACKAGE_ENCRYPTED_ENTRY",
            ),
            (
                "bad-04-missing-content-types.docx",
                "DOCX_PACKAGE_CONTENT_TYPES_MISSING",
            ),
            (
                "bad-05-broken-internal-relationship.docx",
                "DOCX_PACKAGE_RELATIONSHIP_TARGET_MISSING",
            ),
        ] {
            let path = fixture_root.join(name);
            let error = open_docx(&path, DocxPackageLimits::default()).unwrap_err();
            assert!(
                error.contains(code),
                "fixture {} returned {error}, expected {code}",
                path.display()
            );
        }

        let external = extract_checked_in_fixture("bad-06-external-image.docx");
        assert_eq!(external["parser"]["options"]["publishBlocked"], true);
        assert!(has_issue(&external, "DOCX_EXTERNAL_ASSET_MISSING"));

        let empty_cell = extract_checked_in_fixture("bad-07-empty-table-cell.docx");
        let cells = empty_cell["pages"][0]["tables"][0]["cells"]
            .as_array()
            .unwrap();
        assert_eq!(cells.len(), 4);
        let retained_empty = cells
            .iter()
            .find(|cell| cell["row"] == 0 && cell["col"] == 1)
            .unwrap();
        assert!(retained_empty["sourceAnchors"][0]["ooxmlPath"]
            .as_str()
            .is_some_and(|path| path.ends_with("/tr[1]/tc[2]")));

        let ambiguous_merge = extract_checked_in_fixture("bad-08-vmerge-without-restart.docx");
        assert!(has_warning(&ambiguous_merge, "TABLE_TOPOLOGY_AMBIGUOUS"));
        let merge_cells = ambiguous_merge["pages"][0]["tables"][0]["cells"]
            .as_array()
            .unwrap();
        assert!(!merge_cells
            .iter()
            .any(|cell| cell["row"] == 0 && cell["col"] == 0));

        let floating = extract_checked_in_fixture("bad-09-floating-textbox-without-offset.docx");
        assert!(has_issue(&floating, "DOCX_FLOATING_ORDER_AMBIGUOUS"));

        let vml = extract_checked_in_fixture("bad-10-vml-textbox.docx");
        let vml_box = &vml["parser"]["options"]["ooxml"]["textBoxes"][0];
        assert_eq!(vml_box["text"], "VML text box");
        assert_eq!(vml_box["xEmu"], 152_400);
        assert_eq!(vml_box["yEmu"], 228_600);
        assert_eq!(vml_box["widthEmu"], 1_524_000);
        assert_eq!(vml_box["heightEmu"], 508_000);

        let smartart = extract_checked_in_fixture("bad-11-smartart-without-preview.docx");
        let smartart_facts = &smartart["parser"]["options"]["ooxml"];
        assert!(smartart_facts["compositeDrawings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "smartart" && item["accessibleText"] == "SmartArt node"));
        assert!(has_issue(&smartart, "UNSUPPORTED_DOCX_COMPOSITE_DRAWING"));
        assert_eq!(smartart["parser"]["options"]["publishBlocked"], true);
        assert!(smartart["pages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|page| page["regions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|region| region["kind"] == "figure")));
        assert!(smartart["assets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|asset| asset["kind"] == "diagram"));
        assert!(smartart_facts["auxiliaryParts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|part| part["path"] == "word/diagrams/data1.xml"));

        let chart = extract_checked_in_fixture("bad-12-chart-without-preview.docx");
        let chart_facts = &chart["parser"]["options"]["ooxml"];
        assert!(chart_facts["compositeDrawings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "chart" && item["accessibleText"] == "Chart title"));
        assert!(chart["assets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|asset| asset["kind"] == "chart"));
        assert_eq!(chart["parser"]["options"]["publishBlocked"], true);
        assert!(has_issue(&chart, "UNSUPPORTED_DOCX_COMPOSITE_DRAWING"));
        assert!(chart["pages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|page| page["regions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|region| region["kind"] == "figure")));
        assert_eq!(
            chart["parser"]["options"]["renderAssist"]["geometryAuthority"],
            "ooxml-semantic-only"
        );

        let columns = extract_checked_in_fixture("bad-13-two-column-section.docx");
        let section = &columns["parser"]["options"]["ooxml"]["sections"][0];
        assert_eq!(section["columns"].as_array().unwrap().len(), 2);
        assert!(section["columns"]
            .as_array()
            .unwrap()
            .iter()
            .all(|column| column["spaceTwips"] == 720));
        assert_eq!(
            columns["parser"]["options"]["renderAssist"]["geometryAuthority"],
            "ooxml-semantic-only"
        );

        let raw = extract_checked_in_fixture("bad-14-raw-whitespace-and-breaks.docx");
        assert_eq!(raw["pages"].as_array().unwrap().len(), 2);
        assert_eq!(raw["pages"][0]["lines"][0]["text"], "  lead  \ttabbed");
        assert_eq!(raw["pages"][1]["lines"][0]["text"], "3");
        let fields = raw["parser"]["options"]["ooxml"]["fieldFacts"]
            .as_array()
            .unwrap();
        assert!(fields
            .iter()
            .any(|field| { field["instruction"] == " PAGE " && field["displayText"] == "" }));
    }

    fn phase3_fixture_root() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures")
            .join("golden")
            .join("synthetic")
            .join("docx")
    }

    fn extract_checked_in_fixture(name: &str) -> serde_json::Value {
        let path = phase3_fixture_root().join(name);
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("missing phase3 fixture {}: {error}", path.display()));
        let (job, source) = job_and_source(&bytes);
        extract_docx_facts_shadow(&job, &source, &path)
            .unwrap_or_else(|error| panic!("phase3 fixture {} failed: {error}", path.display()))
    }

    fn has_issue(value: &serde_json::Value, code: &str) -> bool {
        value["parser"]["options"]["ooxml"]["issues"]
            .as_array()
            .is_some_and(|issues| issues.iter().any(|issue| issue["code"] == code))
    }

    fn has_warning(value: &serde_json::Value, code: &str) -> bool {
        value["parser"]["warnings"]
            .as_array()
            .is_some_and(|warnings| {
                warnings
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .any(|warning| warning.contains(code))
            })
    }

    fn assert_render_binding(line: &serde_json::Value, path_suffix: &str, semantic_text: &str) {
        let anchors = line["sourceAnchors"].as_array().unwrap();
        assert!(!anchors.is_empty());
        assert!(
            anchors.iter().all(|anchor| {
                anchor["extractionMode"] == "docx_rendered_fallback"
                    && anchor["ooxmlPath"]
                        .as_str()
                        .is_some_and(|path| path.ends_with(path_suffix))
            }),
            "line {} did not bind every anchor to {path_suffix}: {anchors:?}",
            line["text"]
        );
        assert!(anchors.iter().any(|anchor| {
            anchor["variants"].as_array().is_some_and(|variants| {
                variants.iter().any(|variant| {
                    variant["extractionMode"] == "docx_ooxml" && variant["text"] == semantic_text
                })
            })
        }));
    }

    fn make_temp_dir() -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("phase3-docx-shadow-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
