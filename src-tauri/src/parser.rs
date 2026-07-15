use crate::authoring_pipeline::collapse_whitespace;
use crate::environment::{
    cloud_pdf_vision_enabled, command_failure, find_sidecar, local_ocr_enabled,
    pdf_renderer_setting, resolve_python_command,
};
use crate::util::{read_json, write_json};
use crate::{hash_bytes, html_escape, main_source_file, CommandResult, ImportJob, SourceFile};
use chrono::Utc;
use quick_xml::{events::Event, Reader};
use serde_json::{json, Value};
use std::{collections::HashMap, fs, io::Read, path::Path, process::Command};
use zip::ZipArchive;

#[derive(Debug, Clone)]
struct TableCellIr {
    row: usize,
    col: usize,
    text: String,
    col_span: Option<usize>,
    vertical_merge: Option<String>,
}

#[derive(Debug, Clone)]
struct TableIr {
    cells: Vec<TableCellIr>,
    rows: usize,
    cols: usize,
}

fn role_hint_for_text(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("answers")
        || lower.starts_with("## answers")
        || lower.contains("answer key")
        || lower.contains("答案")
        || looks_like_answer_key_block(text)
    {
        Some("answer")
    } else if lower.contains("questions ")
        || lower.starts_with("question ")
        || lower.contains("true") && lower.contains("false") && lower.contains("not given")
        || lower.contains("choose one")
        || lower.contains("complete the")
    {
        Some("question")
    } else if lower.contains("reading passage") || lower.starts_with("passage ") {
        Some("passage")
    } else {
        None
    }
}

fn looks_like_answer_key_block(text: &str) -> bool {
    let mut answer_lines = 0usize;
    let mut total_lines = 0usize;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        total_lines += 1;
        let mut chars = line.chars().peekable();
        let mut digit_count = 0usize;
        while chars.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            digit_count += 1;
            chars.next();
        }
        if digit_count == 0 {
            return false;
        }
        while chars
            .peek()
            .is_some_and(|ch| matches!(ch, '.' | ')' | ':' | '、') || ch.is_whitespace())
        {
            chars.next();
        }
        let answer = chars.collect::<String>();
        let answer_word_count = answer.split_whitespace().count();
        if answer.trim().is_empty() || answer_word_count > 6 || answer.contains('?') {
            return false;
        }
        answer_lines += 1;
    }
    total_lines >= 2 && answer_lines == total_lines
}

fn block_type_for_text(text: &str) -> &'static str {
    let trimmed = text.trim();
    if trimmed.starts_with('#')
        || trimmed.to_uppercase().starts_with("READING PASSAGE")
        || trimmed.to_lowercase().starts_with("questions ")
    {
        "header"
    } else if trimmed.contains('|') && trimmed.matches('|').count() >= 2 {
        "table"
    } else if trimmed
        .lines()
        .any(|line| line.trim_start().starts_with("- ") || line.trim_start().starts_with("* "))
    {
        "list"
    } else {
        "paragraph"
    }
}

fn markdownish_to_html(text: &str, block_type: &str) -> String {
    let trimmed = text.trim();
    match block_type {
        "header" => format!(
            "<h3>{}</h3>",
            html_escape(trimmed.trim_start_matches('#').trim())
        ),
        "table" => {
            let rows = trimmed
                .lines()
                .filter(|line| {
                    (line.contains('|') || line.contains('\t'))
                        && !line
                            .trim()
                            .chars()
                            .all(|ch| matches!(ch, '|' | '-' | ':' | ' '))
                })
                .map(|line| {
                    let parts = if line.contains('\t') {
                        line.split('\t').collect::<Vec<_>>()
                    } else {
                        line.trim_matches('|').split('|').collect::<Vec<_>>()
                    };
                    let cells = parts
                        .iter()
                        .map(|cell| format!("<td>{}</td>", html_escape(cell.trim())))
                        .collect::<String>();
                    format!("<tr>{}</tr>", cells)
                })
                .collect::<String>();
            format!("<table>{}</table>", rows)
        }
        "list" => {
            let items = trimmed
                .lines()
                .map(|line| {
                    line.trim_start()
                        .trim_start_matches("- ")
                        .trim_start_matches("* ")
                        .trim()
                })
                .filter(|line| !line.is_empty())
                .map(|line| format!("<li>{}</li>", html_escape(line)))
                .collect::<String>();
            format!("<ul>{}</ul>", items)
        }
        _ => format!("<p>{}</p>", html_escape(trimmed)),
    }
}

// --- pub(crate) re-exports so the pdfium geometry backend can reuse the same
// block-shaping helpers without duplicating the heuristics. ---
pub(crate) fn block_type_for_text_pub(text: &str) -> &'static str {
    block_type_for_text(text)
}

pub(crate) fn role_hint_for_text_pub(text: &str) -> Option<&'static str> {
    role_hint_for_text(text)
}

pub(crate) fn markdownish_to_html_pub(text: &str, block_type: &str) -> String {
    markdownish_to_html(text, block_type)
}

pub(crate) fn stabilize_pdf_image_extraction_fields_pub(
    extraction: &mut Value,
    renderer_adapter: Option<&str>,
    renderer_provider: Option<&str>,
    renderer_version: Option<Value>,
    dpi: Option<u64>,
    failure_reason: Option<&str>,
    requires_manual_review: Option<bool>,
) {
    stabilize_pdf_image_extraction_fields(
        extraction,
        renderer_adapter,
        renderer_provider,
        renderer_version,
        dpi,
        failure_reason,
        requires_manual_review,
    )
}

fn table_ir_to_html(table: &TableIr) -> String {
    let rows = (0..table.rows)
        .map(|row| {
            let cells = (0..table.cols)
                .map(|col| {
                    let text = table
                        .cells
                        .iter()
                        .find(|cell| cell.row == row && cell.col == col)
                        .map(|cell| cell.text.as_str())
                        .unwrap_or_default();
                    format!("<td>{}</td>", html_escape(text))
                })
                .collect::<String>();
            format!("<tr>{}</tr>", cells)
        })
        .collect::<String>();
    format!("<table>{}</table>", rows)
}

fn table_ir_to_value(table: &TableIr) -> Value {
    json!({
        "rows": table.rows,
        "cols": table.cols,
        "cells": table.cells.iter().map(|cell| {
            json!({
                "row": cell.row,
                "col": cell.col,
                "text": cell.text,
                "colSpan": cell.col_span,
                "verticalMerge": cell.vertical_merge
            })
        }).collect::<Vec<_>>()
    })
}

fn paragraph_text_chunks(content: &str) -> Vec<String> {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks = Vec::new();
    let mut current = Vec::new();

    for line in normalized.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line.to_string());
        }
    }
    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }
    blocks
}

fn semantic_text_chunks(content: &str) -> Vec<String> {
    let paragraphs = paragraph_text_chunks(content);
    if paragraphs.len() > 1 {
        return paragraphs;
    }

    let text = collapse_whitespace(content);
    if text.is_empty() {
        return Vec::new();
    }

    let lower = text.to_lowercase();
    let mut markers = Vec::new();
    for pattern in [
        "reading passage ",
        "questions ",
        "question ",
        "answer key",
        "answers",
    ] {
        markers.extend(lower.match_indices(pattern).map(|(index, _)| index));
    }
    markers.sort_unstable();
    markers.dedup();
    if markers.len() <= 1 {
        return vec![text];
    }

    let mut chunks = Vec::new();
    if markers[0] > 0 {
        chunks.push(text[..markers[0]].trim().to_string());
    }
    for (position, start) in markers.iter().enumerate() {
        let end = markers.get(position + 1).copied().unwrap_or(text.len());
        let chunk = text[*start..end].trim();
        if !chunk.is_empty() {
            chunks.push(chunk.to_string());
        }
    }
    chunks
}

fn document_block(
    block_id: String,
    text: &str,
    page_index: usize,
    ordinal: usize,
    confidence: f64,
) -> Value {
    let block_type = block_type_for_text(text);
    let role_hint = role_hint_for_text(text);
    let y0 = 72 + ((ordinal % 16) as i32 * 42);
    let mut block = json!({
        "blockId": block_id,
        "blockType": block_type,
        "text": text,
        "html": markdownish_to_html(text, block_type),
        "bbox": [72, y0, 520, (y0 + 36).min(794)],
        "confidence": confidence,
        "pageIndex": page_index
    });
    if let Some(role) = role_hint {
        block["roleHint"] = json!(role);
    }
    block
}

fn text_document_ir(job: &ImportJob, source: &SourceFile, content: &str, mode: &str) -> Value {
    let mut blocks = paragraph_text_chunks(content);
    if blocks.is_empty() {
        blocks.push(job.title.clone());
    }

    let document_blocks = blocks
        .iter()
        .enumerate()
        .map(|(index, text)| document_block(format!("b{:03}", index + 1), text, 1, index, 1.0))
        .collect::<Vec<_>>();

    json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": [{
            "pageIndex": 1,
            "width": 595,
            "height": 842,
            "blocks": document_blocks
        }],
        "assets": [],
        "parser": {
            "provider": "local-text-parser",
            "version": "0.2.0",
            "mode": mode,
            "warnings": [],
            "sourceFileId": source.file_id,
            "sourceStoredName": source.stored_name
        }
    })
}

fn parse_text_with_rust_parser(
    job: &ImportJob,
    source: &SourceFile,
    upload_path: &Path,
    output_path: &Path,
    mode: &str,
) -> CommandResult<Value> {
    let content = fs::read_to_string(upload_path)
        .map_err(|error| format!("rust_text_read_failed:{}:{}", upload_path.display(), error))?;
    let mut ir = text_document_ir(job, source, &content, mode);
    let provider = if source.file_type == "md" {
        "rust-parser:text:markdown"
    } else {
        "rust-parser:text:plain"
    };
    if let Some(parser) = ir.get_mut("parser").and_then(Value::as_object_mut) {
        parser.insert("provider".to_string(), json!(provider));
        parser.insert("version".to_string(), json!("0.3.0"));
    }
    write_json(output_path, &ir)?;
    Ok(ir)
}

pub(crate) fn manual_transcription_document_ir(
    job: &ImportJob,
    content: &str,
    note: Option<&str>,
) -> Value {
    let source = main_source_file(job)
        .cloned()
        .unwrap_or_else(|| SourceFile {
            file_id: "manual-source".to_string(),
            original_name: "manual-transcription.txt".to_string(),
            stored_name: "manual-transcription.txt".to_string(),
            file_type: "txt".to_string(),
            sha256: hash_bytes(content.as_bytes()),
            size_bytes: content.len() as u64,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        });
    let mut ir = text_document_ir(job, &source, content, "manual");
    if let Some(parser) = ir.get_mut("parser").and_then(Value::as_object_mut) {
        parser.insert("provider".to_string(), json!("manual-transcription"));
        parser.insert("version".to_string(), json!("0.3.0"));
        parser.insert("mode".to_string(), json!("manual"));
        parser.insert(
            "warnings".to_string(),
            json!(["manual transcription supplied by operator; verify against source PDF before publish"]),
        );
        if let Some(note) = note.filter(|value| !value.trim().is_empty()) {
            parser.insert("note".to_string(), json!(note));
        }
    }
    ir
}

pub(crate) fn vision_transcription_document_ir(
    job: &ImportJob,
    content: &str,
    confidence: f64,
    warnings: Vec<String>,
    evidence: Value,
    note: Option<&str>,
) -> Value {
    let source = main_source_file(job)
        .cloned()
        .unwrap_or_else(|| SourceFile {
            file_id: "vision-source".to_string(),
            original_name: "vision-transcription.txt".to_string(),
            stored_name: "vision-transcription.txt".to_string(),
            file_type: "txt".to_string(),
            sha256: hash_bytes(content.as_bytes()),
            size_bytes: content.len() as u64,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        });
    let mut ir = text_document_ir(job, &source, content, "ocr");
    let normalized_confidence = confidence.clamp(0.0, 1.0);
    for block in ir
        .get_mut("pages")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .flat_map(|page| {
            page.get_mut("blocks")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
        })
    {
        if let Some(obj) = block.as_object_mut() {
            obj.insert("confidence".to_string(), json!(normalized_confidence));
        }
    }
    if let Some(parser) = ir.get_mut("parser").and_then(Value::as_object_mut) {
        parser.insert("provider".to_string(), json!("vision-llm-transcription"));
        parser.insert("version".to_string(), json!("0.1.0"));
        parser.insert("mode".to_string(), json!("ocr"));
        let mut parser_warnings =
            vec!["vision LLM transcription; verify against source PDF before publish".to_string()];
        parser_warnings.extend(
            warnings
                .into_iter()
                .filter(|warning| !warning.trim().is_empty()),
        );
        parser.insert("warnings".to_string(), json!(parser_warnings));
        parser.insert("evidence".to_string(), evidence);
        if let Some(note) = note.filter(|value| !value.trim().is_empty()) {
            parser.insert("note".to_string(), json!(note));
        }
    }
    ir
}

fn append_parser_warning(ir: &mut Value, warning: String) {
    if let Some(parser) = ir.get_mut("parser").and_then(Value::as_object_mut) {
        let warnings = parser.entry("warnings").or_insert_with(|| json!([]));
        if let Some(items) = warnings.as_array_mut() {
            items.push(json!(warning));
        }
    }
}

fn parse_pdf_with_rust_text_extractor(
    job: &ImportJob,
    source: &SourceFile,
    upload_path: &Path,
    output_path: &Path,
    mode: &str,
) -> CommandResult<Value> {
    // Prefer the pdfium backend, which yields REAL per-character coordinates
    // so multi-column detection and reading-order reconstruction actually
    // work. Fall back to the text-layer extractor (no coordinates) only when
    // the native pdfium library is unavailable or the PDF cannot be opened.
    match crate::pdf_geometry::parse_pdf_with_pdfium(job, source, upload_path, output_path, mode) {
        Ok(ir) => return Ok(ir),
        Err(failure) if failure.starts_with("pdfium_library_unavailable") => {
            // Library missing: fall through silently to the text-layer path.
        }
        Err(failure) if failure.starts_with("pdfium_bind") => {
            // Library binding failed: fall through silently to the text-layer path.
        }
        Err(other) => {
            // pdfium was loadable but the specific PDF failed. Record a
            // warning and fall back, so a single corrupt PDF doesn't block
            // the whole pipeline.
            let mut fallback =
                parse_pdf_with_text_layer(job, source, upload_path, output_path, mode)?;
            if let Some(parser) = fallback.get_mut("parser").and_then(Value::as_object_mut) {
                let warnings = parser
                    .entry("warnings".to_string())
                    .or_insert_with(|| json!([]));
                if let Some(items) = warnings.as_array_mut() {
                    items.push(json!(format!(
                        "pdfium_backend_fell_back_to_text_layer:{}",
                        other
                    )));
                }
            }
            return Ok(fallback);
        }
    }
    parse_pdf_with_text_layer(job, source, upload_path, output_path, mode)
}

/// Text-layer-only PDF parser (no real coordinates). Retained as the fallback
/// for environments where the native pdfium library is not available. Blocks
/// carry a fabricated `bbox: [72, y0, 520, y0+36]` envelope; column detection
/// (`dynamic_block_column`) therefore degrades to column 0 on this path.
fn parse_pdf_with_text_layer(
    job: &ImportJob,
    source: &SourceFile,
    upload_path: &Path,
    output_path: &Path,
    mode: &str,
) -> CommandResult<Value> {
    let extracted_pages = pdf_extract::extract_text_by_pages(upload_path)
        .map_err(|error| format!("rust_pdf_extract_failed:{}", error))?;
    let mut warnings = Vec::<String>::new();
    let mut block_counter = 1usize;
    let pages = if extracted_pages.is_empty() {
        warnings.push("PDF has no readable pages; OCR/manual review required".to_string());
        vec![json!({
            "pageIndex": 1,
            "width": 595,
            "height": 842,
            "blocks": [document_block(
                "b001".to_string(),
                "[No extractable text on page 1]",
                1,
                0,
                0.2,
            )]
        })]
    } else {
        extracted_pages
            .iter()
            .enumerate()
            .map(|(page_index, page_text)| {
                let page_number = page_index + 1;
                let mut chunks = semantic_text_chunks(page_text);
                if chunks.is_empty() {
                    warnings.push(format!(
                        "page {} has no extractable text; OCR/manual review required",
                        page_number
                    ));
                    chunks.push(format!("[No extractable text on page {}]", page_number));
                }
                let blocks = chunks
                    .iter()
                    .enumerate()
                    .map(|(ordinal, chunk)| {
                        let is_placeholder = chunk.starts_with("[No extractable text");
                        let block = document_block(
                            format!("b{:03}", block_counter),
                            chunk,
                            page_number,
                            ordinal,
                            if is_placeholder { 0.2 } else { 0.98 },
                        );
                        block_counter += 1;
                        block
                    })
                    .collect::<Vec<_>>();
                json!({
                    "pageIndex": page_number,
                    "width": 595,
                    "height": 842,
                    "blocks": blocks
                })
            })
            .collect::<Vec<_>>()
    };

    let ir = json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": pages,
        "assets": [],
        "parser": {
            "provider": "rust-parser:pdf:pdf-extract",
            "version": "0.1.0",
            "mode": mode,
            "warnings": warnings,
            "sourceFileId": source.file_id,
            "sourceStoredName": source.stored_name
        }
    });
    write_json(output_path, &ir)?;
    Ok(ir)
}

#[derive(Debug, Clone)]
struct DocxRawBlock {
    kind: &'static str,
    text: String,
    table: Option<TableIr>,
    layout_hints: Option<Value>,
}

fn is_word_tag(name: &[u8], tag: &[u8]) -> bool {
    name == tag || name.ends_with(&[b":", tag].concat())
}

#[derive(Debug, Clone, Default)]
struct DocxTableCell {
    text: String,
    col_span: Option<usize>,
    vertical_merge: Option<String>,
}

fn push_docx_table_block(blocks: &mut Vec<DocxRawBlock>, rows: &[Vec<DocxTableCell>]) {
    if !rows
        .iter()
        .flat_map(|row| row.iter())
        .any(|cell| !cell.text.trim().is_empty())
    {
        return;
    }
    let cols = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| cell.col_span.unwrap_or(1).max(1))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    let mut cells = Vec::new();
    let mut text_rows = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        let mut text_cells = Vec::new();
        let mut col_index = 0usize;
        for cell in row {
            let text = cell.text.clone();
            cells.push(TableCellIr {
                row: row_index,
                col: col_index,
                text: text.clone(),
                col_span: cell.col_span,
                vertical_merge: cell.vertical_merge.clone(),
            });
            text_cells.push(text);
            col_index += cell.col_span.unwrap_or(1).max(1);
        }
        while col_index < cols {
            cells.push(TableCellIr {
                row: row_index,
                col: col_index,
                text: String::new(),
                col_span: None,
                vertical_merge: None,
            });
            text_cells.push(String::new());
            col_index += 1;
        }
        text_rows.push(text_cells.join("\t"));
    }
    blocks.push(DocxRawBlock {
        kind: "table",
        text: text_rows.join("\n"),
        table: Some(TableIr {
            cells,
            rows: rows.len(),
            cols,
        }),
        layout_hints: Some(json!({
            "source": "docx-ooxml-table",
            "rows": rows.len(),
            "cols": cols
        })),
    });
}

#[derive(Debug, Clone, Default)]
struct DocxParagraphMeta {
    style_id: Option<String>,
    style_name: Option<String>,
    based_on_style_id: Option<String>,
    resolved_style_id: Option<String>,
    numbering_level: Option<u32>,
    numbering_id: Option<String>,
    numbering_format: Option<String>,
    numbering_text: Option<String>,
    rendered_numbering_label: Option<String>,
    abstract_numbering_id: Option<String>,
    style_heading_level: Option<u32>,
    section_columns: Option<DocxSectionColumns>,
    // Run-level formatting captured from w:rPr. These are None when the run
    // properties were absent; Some(false) when explicitly turned off (e.g.
    // <w:b w:val="0"/>). Used to recognise passage titles / sub-headings in
    // documents that have no Heading styles (a common export pattern).
    is_bold: Option<bool>,
    is_italic: Option<bool>,
    max_font_size_half_pts: Option<u32>,
    min_font_size_half_pts: Option<u32>,
    justification: Option<String>,
    has_page_break_before: Option<bool>,
    /// Set by the parser after the paragraph text is known, so heading-level
    /// inference can apply length/punctuation heuristics.
    run_heading_level: Option<u32>,
}

impl DocxParagraphMeta {
    fn is_list(&self) -> bool {
        self.numbering_id.is_some() || self.numbering_level.is_some()
    }

    fn heading_level(&self) -> Option<u32> {
        if let Some(level) = self.style_heading_level {
            return Some(level);
        }
        // Derive a level from the paragraph style id (e.g. "Heading1" /
        // "标题2") when present. If there is no style id at all, fall through
        // to the run-format-derived level rather than returning early.
        let from_style = self.style_id.as_deref().and_then(|raw_style| {
            let style = raw_style.to_ascii_lowercase();
            style
                .strip_prefix("heading")
                .or_else(|| style.strip_prefix("标题"))
                .and_then(|suffix| suffix.chars().find(|ch| ch.is_ascii_digit()))
                .and_then(|ch| ch.to_digit(10))
        });
        from_style.or(self.run_heading_level)
    }

    fn block_kind(&self) -> &'static str {
        if self.heading_level().is_some() {
            "header"
        } else if self.is_list() {
            "list"
        } else {
            "paragraph"
        }
    }

    /// Infer a synthetic heading level from run formatting when there is no
    /// Heading style and no list numbering. Only fires for short, bold,
    /// period-less paragraphs — the signature of a passage sub-heading or
    /// title in Normal-styled IELTS DOCX exports.
    fn infer_run_heading_level(&self, text: &str) -> Option<u32> {
        if self.style_heading_level.is_some() || self.is_list() {
            return None;
        }
        let bold = self.is_bold.unwrap_or(false);
        let trimmed = text.trim();
        let count = trimmed.chars().count();
        let ends_period = matches!(trimmed.chars().last(), Some('.') | Some('。'));
        let centered = self
            .justification
            .as_deref()
            .map(|value| value.eq_ignore_ascii_case("center"))
            .unwrap_or(false);
        if !bold || trimmed.is_empty() || count > 30 || ends_period {
            return None;
        }
        // A bold, short, period-less paragraph with no Heading style is a
        // passage sub-heading (level 3) or, when centered, the passage title
        // (level 2). This rescues IELTS DOCX exports that mark structure only
        // via run formatting rather than Heading styles.
        if centered {
            Some(2)
        } else {
            Some(3)
        }
    }

    fn layout_hints(&self) -> Option<Value> {
        let has_run_format = self.is_bold.is_some()
            || self.is_italic.is_some()
            || self.max_font_size_half_pts.is_some()
            || self.justification.is_some()
            || self.has_page_break_before.unwrap_or(false);
        if self.style_id.is_none()
            && self.numbering_id.is_none()
            && self.numbering_level.is_none()
            && self.section_columns.is_none()
            && !has_run_format
        {
            return None;
        }
        let run_format = if has_run_format {
            json!({
                "bold": self.is_bold.unwrap_or(false),
                "italic": self.is_italic.unwrap_or(false),
                "fontSize": self.max_font_size_half_pts,
                "centered": self.justification.as_deref().map(|value| value.eq_ignore_ascii_case("center")).unwrap_or(false),
                "pageBreakBefore": self.has_page_break_before.unwrap_or(false)
            })
        } else {
            Value::Null
        };
        Some(json!({
            "source": "docx-ooxml-paragraph",
            "styleId": self.style_id,
            "styleName": self.style_name,
            "basedOnStyleId": self.based_on_style_id,
            "resolvedStyleId": self.resolved_style_id,
            "headingLevel": self.heading_level(),
            "numbering": if self.is_list() {
                json!({
                    "level": self.numbering_level,
                    "id": self.numbering_id,
                    "abstractId": self.abstract_numbering_id,
                    "format": self.numbering_format,
                    "text": self.numbering_text,
                    "renderedLabel": self.rendered_numbering_label
                })
            } else {
                Value::Null
            },
            "section": self.section_columns.as_ref().map(|columns| json!({
                "columns": {
                    "count": columns.count,
                    "spaceTwips": columns.space_twips,
                    "equalWidth": columns.equal_width
                }
            })).unwrap_or(Value::Null),
            "runFormat": run_format
        }))
    }
}

fn attr_value(event: &quick_xml::events::BytesStart<'_>, attr_name: &[u8]) -> Option<String> {
    event
        .attributes()
        .flatten()
        .find(|attr| is_word_tag(attr.key.as_ref(), attr_name))
        .and_then(|attr| {
            std::str::from_utf8(attr.value.as_ref())
                .ok()
                .map(ToString::to_string)
        })
}

#[derive(Debug, Clone, Default)]
struct DocxStyleDef {
    style_id: String,
    name: Option<String>,
    based_on: Option<String>,
    outline_level: Option<u32>,
    numbering_level: Option<u32>,
    numbering_id: Option<String>,
}

#[derive(Debug, Clone)]
struct DocxNumberingLevelDef {
    level: u32,
    format: Option<String>,
    text: Option<String>,
    start: u32,
}

impl Default for DocxNumberingLevelDef {
    fn default() -> Self {
        Self {
            level: 0,
            format: None,
            text: None,
            start: 1,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DocxNumberingDefs {
    num_to_abstract: HashMap<String, String>,
    abstract_levels: HashMap<(String, u32), DocxNumberingLevelDef>,
    start_overrides: HashMap<(String, u32), u32>,
}

#[derive(Debug, Clone, Default)]
struct DocxSectionColumns {
    count: Option<u32>,
    space_twips: Option<u32>,
    equal_width: Option<bool>,
}

fn heading_level_from_style_text(value: &str) -> Option<u32> {
    let lower = value.to_ascii_lowercase();
    lower
        .strip_prefix("heading")
        .or_else(|| lower.strip_prefix("标题"))
        .and_then(|suffix| suffix.chars().find(|ch| ch.is_ascii_digit()))
        .and_then(|ch| ch.to_digit(10))
}

fn parse_docx_styles_xml(styles_xml: &[u8]) -> CommandResult<HashMap<String, DocxStyleDef>> {
    let mut reader = Reader::from_reader(styles_xml);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut styles = HashMap::<String, DocxStyleDef>::new();
    let mut current = None::<DocxStyleDef>;

    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("docx_styles_xml_read_failed:{}", error))?
        {
            Event::Start(event) => {
                let name = event.name().as_ref().to_vec();
                if is_word_tag(&name, b"style") {
                    let style_id = attr_value(&event, b"styleId").unwrap_or_default();
                    if !style_id.trim().is_empty() {
                        current = Some(DocxStyleDef {
                            style_id,
                            ..DocxStyleDef::default()
                        });
                    }
                } else if let Some(style) = current.as_mut() {
                    if is_word_tag(&name, b"name") {
                        style.name = attr_value(&event, b"val");
                    } else if is_word_tag(&name, b"basedOn") {
                        style.based_on = attr_value(&event, b"val");
                    } else if is_word_tag(&name, b"outlineLvl") {
                        style.outline_level =
                            attr_value(&event, b"val").and_then(|value| value.parse().ok());
                    } else if is_word_tag(&name, b"ilvl") {
                        style.numbering_level =
                            attr_value(&event, b"val").and_then(|value| value.parse().ok());
                    } else if is_word_tag(&name, b"numId") {
                        style.numbering_id = attr_value(&event, b"val");
                    }
                }
            }
            Event::Empty(event) => {
                let name = event.name().as_ref().to_vec();
                if is_word_tag(&name, b"style") {
                    let style_id = attr_value(&event, b"styleId").unwrap_or_default();
                    if !style_id.trim().is_empty() {
                        styles.insert(
                            style_id.clone(),
                            DocxStyleDef {
                                style_id,
                                ..DocxStyleDef::default()
                            },
                        );
                    }
                } else if let Some(style) = current.as_mut() {
                    if is_word_tag(&name, b"name") {
                        style.name = attr_value(&event, b"val");
                    } else if is_word_tag(&name, b"basedOn") {
                        style.based_on = attr_value(&event, b"val");
                    } else if is_word_tag(&name, b"outlineLvl") {
                        style.outline_level =
                            attr_value(&event, b"val").and_then(|value| value.parse().ok());
                    } else if is_word_tag(&name, b"ilvl") {
                        style.numbering_level =
                            attr_value(&event, b"val").and_then(|value| value.parse().ok());
                    } else if is_word_tag(&name, b"numId") {
                        style.numbering_id = attr_value(&event, b"val");
                    }
                }
            }
            Event::End(event) => {
                let name = event.name().as_ref().to_vec();
                if is_word_tag(&name, b"style") {
                    if let Some(style) = current.take() {
                        styles.insert(style.style_id.clone(), style);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(styles)
}

fn parse_docx_numbering_xml(numbering_xml: &[u8]) -> CommandResult<DocxNumberingDefs> {
    let mut reader = Reader::from_reader(numbering_xml);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut defs = DocxNumberingDefs::default();
    let mut current_abstract_id = None::<String>;
    let mut current_num_id = None::<String>;
    let mut current_level = None::<DocxNumberingLevelDef>;
    let mut current_override_level = None::<u32>;

    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("docx_numbering_xml_read_failed:{}", error))?
        {
            Event::Start(event) => {
                let name = event.name().as_ref().to_vec();
                if is_word_tag(&name, b"abstractNum") {
                    current_abstract_id = attr_value(&event, b"abstractNumId");
                } else if is_word_tag(&name, b"num") {
                    current_num_id = attr_value(&event, b"numId");
                } else if is_word_tag(&name, b"lvl") {
                    current_level = attr_value(&event, b"ilvl")
                        .and_then(|value| value.parse::<u32>().ok())
                        .map(|level| DocxNumberingLevelDef {
                            level,
                            ..DocxNumberingLevelDef::default()
                        });
                } else if is_word_tag(&name, b"lvlOverride") {
                    current_override_level =
                        attr_value(&event, b"ilvl").and_then(|value| value.parse::<u32>().ok());
                } else if let Some(level) = current_level.as_mut() {
                    if is_word_tag(&name, b"numFmt") {
                        level.format = attr_value(&event, b"val");
                    } else if is_word_tag(&name, b"lvlText") {
                        level.text = attr_value(&event, b"val");
                    } else if is_word_tag(&name, b"start") {
                        level.start = attr_value(&event, b"val")
                            .and_then(|value| value.parse::<u32>().ok())
                            .unwrap_or(1);
                    }
                } else if is_word_tag(&name, b"startOverride") {
                    if let (Some(num_id), Some(level), Some(start)) = (
                        current_num_id.as_ref(),
                        current_override_level,
                        attr_value(&event, b"val").and_then(|value| value.parse::<u32>().ok()),
                    ) {
                        defs.start_overrides.insert((num_id.clone(), level), start);
                    }
                } else if is_word_tag(&name, b"abstractNumId") {
                    if let (Some(num_id), Some(abstract_id)) =
                        (current_num_id.as_ref(), attr_value(&event, b"val"))
                    {
                        defs.num_to_abstract.insert(num_id.clone(), abstract_id);
                    }
                }
            }
            Event::Empty(event) => {
                let name = event.name().as_ref().to_vec();
                if let Some(level) = current_level.as_mut() {
                    if is_word_tag(&name, b"numFmt") {
                        level.format = attr_value(&event, b"val");
                    } else if is_word_tag(&name, b"lvlText") {
                        level.text = attr_value(&event, b"val");
                    } else if is_word_tag(&name, b"start") {
                        level.start = attr_value(&event, b"val")
                            .and_then(|value| value.parse::<u32>().ok())
                            .unwrap_or(1);
                    }
                } else if is_word_tag(&name, b"startOverride") {
                    if let (Some(num_id), Some(level), Some(start)) = (
                        current_num_id.as_ref(),
                        current_override_level,
                        attr_value(&event, b"val").and_then(|value| value.parse::<u32>().ok()),
                    ) {
                        defs.start_overrides.insert((num_id.clone(), level), start);
                    }
                } else if is_word_tag(&name, b"abstractNumId") {
                    if let (Some(num_id), Some(abstract_id)) =
                        (current_num_id.as_ref(), attr_value(&event, b"val"))
                    {
                        defs.num_to_abstract.insert(num_id.clone(), abstract_id);
                    }
                }
            }
            Event::End(event) => {
                let name = event.name().as_ref().to_vec();
                if is_word_tag(&name, b"lvl") {
                    if let (Some(abstract_id), Some(level)) =
                        (current_abstract_id.as_ref(), current_level.take())
                    {
                        defs.abstract_levels
                            .insert((abstract_id.clone(), level.level), level);
                    }
                } else if is_word_tag(&name, b"abstractNum") {
                    current_abstract_id = None;
                } else if is_word_tag(&name, b"lvlOverride") {
                    current_override_level = None;
                } else if is_word_tag(&name, b"num") {
                    current_num_id = None;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(defs)
}

fn resolve_docx_style(
    style_id: Option<&str>,
    styles: &HashMap<String, DocxStyleDef>,
) -> (Option<String>, Option<String>, Option<u32>) {
    let Some(style_id) = style_id else {
        return (None, None, None);
    };
    let mut current_id = style_id.to_string();
    let mut based_on = None::<String>;
    let mut name = None::<String>;
    let mut heading_level = heading_level_from_style_text(style_id);
    let mut seen = Vec::<String>::new();
    for _ in 0..12 {
        if seen.contains(&current_id) {
            break;
        }
        seen.push(current_id.clone());
        let Some(style) = styles.get(&current_id) else {
            break;
        };
        if name.is_none() {
            name = style.name.clone();
        }
        if heading_level.is_none() {
            heading_level = style
                .outline_level
                .map(|level| level + 1)
                .or_else(|| {
                    style
                        .name
                        .as_deref()
                        .and_then(heading_level_from_style_text)
                })
                .or_else(|| heading_level_from_style_text(&style.style_id));
        }
        if let Some(parent) = &style.based_on {
            if based_on.is_none() {
                based_on = Some(parent.clone());
            }
            current_id = parent.clone();
        } else {
            break;
        }
    }
    (name, based_on, heading_level)
}

fn resolve_docx_style_numbering(
    style_id: Option<&str>,
    styles: &HashMap<String, DocxStyleDef>,
) -> (Option<String>, Option<u32>) {
    let Some(style_id) = style_id else {
        return (None, None);
    };
    let mut current_id = style_id.to_string();
    let mut seen = Vec::<String>::new();
    for _ in 0..12 {
        if seen.contains(&current_id) {
            break;
        }
        seen.push(current_id.clone());
        let Some(style) = styles.get(&current_id) else {
            break;
        };
        if style.numbering_id.is_some() || style.numbering_level.is_some() {
            return (style.numbering_id.clone(), style.numbering_level);
        }
        let Some(parent) = style.based_on.as_ref() else {
            break;
        };
        current_id = parent.clone();
    }
    (None, None)
}

fn resolve_docx_numbering(
    numbering_id: Option<&str>,
    numbering_level: Option<u32>,
    numbering_defs: &DocxNumberingDefs,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(num_id) = numbering_id else {
        return (None, None, None);
    };
    let abstract_id = numbering_defs.num_to_abstract.get(num_id).cloned();
    let level = numbering_level.unwrap_or(0);
    let level_def = abstract_id
        .as_ref()
        .and_then(|id| numbering_defs.abstract_levels.get(&(id.clone(), level)));
    (
        abstract_id,
        level_def.and_then(|item| item.format.clone()),
        level_def.and_then(|item| item.text.clone()),
    )
}

fn format_docx_alpha(mut value: u32, uppercase: bool) -> String {
    if value == 0 {
        return String::new();
    }
    let mut chars = Vec::new();
    while value > 0 {
        value -= 1;
        let base = if uppercase { b'A' } else { b'a' };
        chars.push((base + (value % 26) as u8) as char);
        value /= 26;
    }
    chars.into_iter().rev().collect()
}

fn format_docx_roman(mut value: u32, uppercase: bool) -> String {
    if value == 0 || value > 3999 {
        return value.to_string();
    }
    let mut output = String::new();
    for (amount, marker) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while value >= amount {
            output.push_str(marker);
            value -= amount;
        }
    }
    if uppercase {
        output
    } else {
        output.to_ascii_lowercase()
    }
}

fn format_docx_number(value: u32, format: Option<&str>) -> Option<String> {
    match format.unwrap_or("decimal") {
        "none" | "bullet" => None,
        "upperLetter" => Some(format_docx_alpha(value, true)),
        "lowerLetter" => Some(format_docx_alpha(value, false)),
        "upperRoman" => Some(format_docx_roman(value, true)),
        "lowerRoman" => Some(format_docx_roman(value, false)),
        "decimalZero" if value < 10 => Some(format!("0{}", value)),
        _ => Some(value.to_string()),
    }
}

fn docx_numbering_start(
    num_id: &str,
    level: u32,
    abstract_id: Option<&str>,
    numbering_defs: &DocxNumberingDefs,
) -> u32 {
    numbering_defs
        .start_overrides
        .get(&(num_id.to_string(), level))
        .copied()
        .or_else(|| {
            abstract_id.and_then(|abstract_id| {
                numbering_defs
                    .abstract_levels
                    .get(&(abstract_id.to_string(), level))
                    .map(|definition| definition.start)
            })
        })
        .unwrap_or(1)
}

fn render_docx_numbering_label(
    meta: &DocxParagraphMeta,
    numbering_defs: &DocxNumberingDefs,
    counters: &mut HashMap<(String, u32), u32>,
) -> Option<String> {
    let num_id = meta.numbering_id.as_deref()?;
    let level = meta.numbering_level.unwrap_or(0);
    let abstract_id = meta.abstract_numbering_id.as_deref();
    let start = docx_numbering_start(num_id, level, abstract_id, numbering_defs);
    let current_value = {
        let current = counters
            .entry((num_id.to_string(), level))
            .and_modify(|value| *value = value.saturating_add(1))
            .or_insert(start);
        *current
    };

    counters.retain(|(candidate_num_id, candidate_level), _| {
        candidate_num_id != num_id || *candidate_level <= level
    });

    let template = meta
        .numbering_text
        .clone()
        .unwrap_or_else(|| format!("%{}.", level + 1));
    let mut output = template;
    for placeholder_level in 0..=8u32 {
        let placeholder = format!("%{}", placeholder_level + 1);
        if !output.contains(&placeholder) {
            continue;
        }
        let value = counters
            .get(&(num_id.to_string(), placeholder_level))
            .copied()
            .unwrap_or_else(|| {
                docx_numbering_start(num_id, placeholder_level, abstract_id, numbering_defs)
            });
        let format = abstract_id.and_then(|abstract_id| {
            numbering_defs
                .abstract_levels
                .get(&(abstract_id.to_string(), placeholder_level))
                .and_then(|definition| definition.format.as_deref())
        });
        let rendered = format_docx_number(value, format)?;
        output = output.replace(&placeholder, &rendered);
    }
    if output.contains('%') {
        let rendered = format_docx_number(current_value, meta.numbering_format.as_deref())?;
        output = rendered;
    }
    let label = collapse_whitespace(&output);
    (!label.is_empty()).then_some(label)
}

fn text_starts_with_docx_label(text: &str, label: &str) -> bool {
    let normalized_label = label.trim().trim_end_matches(['.', ')', ':', '、']);
    let normalized_text = text.trim_start();
    normalized_text.starts_with(label)
        || (!normalized_label.is_empty()
            && normalized_text
                .strip_prefix(normalized_label)
                .and_then(|rest| rest.chars().next())
                .map(|next| next.is_whitespace() || matches!(next, '.' | ')' | ':' | '、'))
                .unwrap_or(false))
}

fn parse_docx_document_xml(
    document_xml: &[u8],
    styles: &HashMap<String, DocxStyleDef>,
    numbering_defs: &DocxNumberingDefs,
) -> CommandResult<Vec<DocxRawBlock>> {
    let mut reader = Reader::from_reader(document_xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut blocks = Vec::<DocxRawBlock>::new();
    let mut in_table = false;
    let mut in_cell = false;
    let mut paragraph_text = String::new();
    let mut cell_text = String::new();
    let mut row_cells = Vec::<DocxTableCell>::new();
    let mut table_rows = Vec::<Vec<DocxTableCell>>::new();
    let mut current_cell = DocxTableCell::default();
    let mut paragraph_meta = DocxParagraphMeta::default();
    let mut active_section_columns = None::<DocxSectionColumns>;
    let mut in_paragraph_properties = false;
    let mut in_numbering_properties = false;
    let mut in_section_properties = false;
    let mut in_table_cell_properties = false;
    // Run-level formatting tracking. w:rPr lives inside w:r and is distinct
    // from w:pPr (paragraph properties). We accumulate bold/italic/font-size
    // across runs so a paragraph is considered bold if ANY run is bold.
    let mut in_run = false;
    let mut in_run_properties = false;
    let mut run_is_bold = false;
    let mut run_is_italic = false;
    let mut run_font_size: Option<u32> = None;
    let mut pending_page_break = false;
    let mut in_paragraph = false;
    let mut in_text_node = false;
    let mut numbering_counters = HashMap::<(String, u32), u32>::new();

    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("docx_xml_read_failed:{}", error))?
        {
            Event::Start(event) => {
                let name = event.name().as_ref().to_vec();
                if is_word_tag(&name, b"tbl") {
                    in_table = true;
                    table_rows.clear();
                } else if is_word_tag(&name, b"tr") && in_table {
                    row_cells.clear();
                } else if is_word_tag(&name, b"tc") && in_table {
                    in_cell = true;
                    cell_text.clear();
                    current_cell = DocxTableCell::default();
                } else if is_word_tag(&name, b"tcPr") && in_cell {
                    in_table_cell_properties = true;
                } else if is_word_tag(&name, b"gridSpan") && (in_table_cell_properties || in_cell) {
                    current_cell.col_span =
                        attr_value(&event, b"val").and_then(|value| value.parse().ok());
                } else if is_word_tag(&name, b"vMerge") && (in_table_cell_properties || in_cell) {
                    current_cell.vertical_merge =
                        Some(attr_value(&event, b"val").unwrap_or_else(|| "continue".to_string()));
                } else if is_word_tag(&name, b"p") {
                    in_paragraph = true;
                    paragraph_text.clear();
                    paragraph_meta = DocxParagraphMeta::default();
                    paragraph_meta.section_columns = active_section_columns.clone();
                } else if is_word_tag(&name, b"pPr") {
                    in_paragraph_properties = true;
                } else if is_word_tag(&name, b"numPr") && in_paragraph_properties {
                    in_numbering_properties = true;
                } else if is_word_tag(&name, b"sectPr") {
                    in_section_properties = true;
                    // A sectPr inside a paragraph's pPr marks a page/section
                    // break that precedes this paragraph's content visually.
                    if in_paragraph_properties {
                        pending_page_break = true;
                    }
                } else if is_word_tag(&name, b"r") {
                    in_run = true;
                    run_is_bold = false;
                    run_is_italic = false;
                    run_font_size = None;
                } else if is_word_tag(&name, b"rPr") && in_run {
                    in_run_properties = true;
                } else if is_word_tag(&name, b"t") {
                    in_text_node = true;
                } else if is_word_tag(&name, b"b") || is_word_tag(&name, b"bCs") {
                    if in_run_properties {
                        run_is_bold = attr_value(&event, b"val")
                            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
                            .unwrap_or(true);
                    }
                } else if is_word_tag(&name, b"i") || is_word_tag(&name, b"iCs") {
                    if in_run_properties {
                        run_is_italic = attr_value(&event, b"val")
                            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
                            .unwrap_or(true);
                    }
                } else if is_word_tag(&name, b"sz") && in_run_properties {
                    run_font_size = attr_value(&event, b"val").and_then(|value| value.parse().ok());
                } else if is_word_tag(&name, b"jc") && in_paragraph_properties {
                    paragraph_meta.justification = attr_value(&event, b"val");
                } else if is_word_tag(&name, b"pStyle") && in_paragraph_properties {
                    paragraph_meta.style_id = attr_value(&event, b"val");
                } else if is_word_tag(&name, b"ilvl") && in_numbering_properties {
                    paragraph_meta.numbering_level =
                        attr_value(&event, b"val").and_then(|value| value.parse::<u32>().ok());
                } else if is_word_tag(&name, b"numId") && in_numbering_properties {
                    paragraph_meta.numbering_id = attr_value(&event, b"val");
                } else if is_word_tag(&name, b"cols") && in_section_properties {
                    let columns = DocxSectionColumns {
                        count: attr_value(&event, b"num").and_then(|value| value.parse().ok()),
                        space_twips: attr_value(&event, b"space")
                            .and_then(|value| value.parse().ok()),
                        equal_width: attr_value(&event, b"equalWidth")
                            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false")),
                    };
                    paragraph_meta.section_columns = Some(columns.clone());
                    active_section_columns = Some(columns);
                } else if is_word_tag(&name, b"gridSpan") && (in_table_cell_properties || in_cell) {
                    current_cell.col_span =
                        attr_value(&event, b"val").and_then(|value| value.parse().ok());
                } else if is_word_tag(&name, b"vMerge") && (in_table_cell_properties || in_cell) {
                    current_cell.vertical_merge =
                        Some(attr_value(&event, b"val").unwrap_or_else(|| "continue".to_string()));
                } else if is_word_tag(&name, b"tab")
                    || is_word_tag(&name, b"br")
                    || is_word_tag(&name, b"cr")
                {
                    if in_paragraph {
                        paragraph_text.push(' ');
                    }
                }
            }
            Event::Empty(event) => {
                let name = event.name().as_ref().to_vec();
                if is_word_tag(&name, b"pStyle") && in_paragraph_properties {
                    paragraph_meta.style_id = attr_value(&event, b"val");
                } else if is_word_tag(&name, b"ilvl") && in_numbering_properties {
                    paragraph_meta.numbering_level =
                        attr_value(&event, b"val").and_then(|value| value.parse::<u32>().ok());
                } else if is_word_tag(&name, b"numId") && in_numbering_properties {
                    paragraph_meta.numbering_id = attr_value(&event, b"val");
                } else if is_word_tag(&name, b"b") || is_word_tag(&name, b"bCs") {
                    if in_run_properties {
                        run_is_bold = attr_value(&event, b"val")
                            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
                            .unwrap_or(true);
                    }
                } else if is_word_tag(&name, b"i") || is_word_tag(&name, b"iCs") {
                    if in_run_properties {
                        run_is_italic = attr_value(&event, b"val")
                            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
                            .unwrap_or(true);
                    }
                } else if is_word_tag(&name, b"sz") && in_run_properties {
                    run_font_size = attr_value(&event, b"val").and_then(|value| value.parse().ok());
                } else if is_word_tag(&name, b"jc") && in_paragraph_properties {
                    paragraph_meta.justification = attr_value(&event, b"val");
                } else if is_word_tag(&name, b"br") || is_word_tag(&name, b"lastRenderedPageBreak")
                {
                    if in_paragraph {
                        paragraph_text.push(' ');
                    }
                } else if is_word_tag(&name, b"cols") && in_section_properties {
                    let columns = DocxSectionColumns {
                        count: attr_value(&event, b"num").and_then(|value| value.parse().ok()),
                        space_twips: attr_value(&event, b"space")
                            .and_then(|value| value.parse().ok()),
                        equal_width: attr_value(&event, b"equalWidth")
                            .map(|value| value != "0" && !value.eq_ignore_ascii_case("false")),
                    };
                    paragraph_meta.section_columns = Some(columns.clone());
                    active_section_columns = Some(columns);
                } else if is_word_tag(&name, b"gridSpan") && (in_table_cell_properties || in_cell) {
                    current_cell.col_span =
                        attr_value(&event, b"val").and_then(|value| value.parse().ok());
                } else if is_word_tag(&name, b"vMerge") && (in_table_cell_properties || in_cell) {
                    current_cell.vertical_merge =
                        Some(attr_value(&event, b"val").unwrap_or_else(|| "continue".to_string()));
                } else if is_word_tag(&name, b"tab")
                    || is_word_tag(&name, b"br")
                    || is_word_tag(&name, b"cr")
                {
                    if in_paragraph {
                        paragraph_text.push(' ');
                    }
                }
            }
            Event::Text(event) => {
                if in_text_node {
                    let text = event
                        .decode()
                        .map_err(|error| format!("docx_text_decode_failed:{}", error))?;
                    paragraph_text.push_str(&text);
                }
            }
            Event::End(event) => {
                let name = event.name().as_ref().to_vec();
                if is_word_tag(&name, b"p") {
                    let collapsed = collapse_whitespace(&paragraph_text);
                    if !collapsed.is_empty() {
                        let mut resolved_meta = paragraph_meta.clone();
                        let (style_name, based_on_style_id, style_heading_level) =
                            resolve_docx_style(resolved_meta.style_id.as_deref(), styles);
                        resolved_meta.style_name = style_name;
                        resolved_meta.based_on_style_id = based_on_style_id;
                        resolved_meta.resolved_style_id = resolved_meta.style_id.clone();
                        resolved_meta.style_heading_level = style_heading_level;
                        if resolved_meta.numbering_id.is_none() {
                            let (style_numbering_id, style_numbering_level) =
                                resolve_docx_style_numbering(
                                    resolved_meta.style_id.as_deref(),
                                    styles,
                                );
                            resolved_meta.numbering_id = style_numbering_id;
                            resolved_meta.numbering_level = style_numbering_level;
                        }
                        let (abstract_id, numbering_format, numbering_text) =
                            resolve_docx_numbering(
                                resolved_meta.numbering_id.as_deref(),
                                resolved_meta.numbering_level,
                                numbering_defs,
                            );
                        resolved_meta.abstract_numbering_id = abstract_id;
                        resolved_meta.numbering_format = numbering_format;
                        resolved_meta.numbering_text = numbering_text;
                        let numbering_label = render_docx_numbering_label(
                            &resolved_meta,
                            numbering_defs,
                            &mut numbering_counters,
                        );
                        resolved_meta.rendered_numbering_label = numbering_label.clone();
                        let visible_text = numbering_label
                            .filter(|label| !text_starts_with_docx_label(&collapsed, label))
                            .map(|label| format!("{} {}", label, collapsed))
                            .unwrap_or_else(|| collapsed.clone());
                        if in_cell {
                            if !cell_text.is_empty() {
                                cell_text.push('\n');
                            }
                            cell_text.push_str(&visible_text);
                        } else if !in_table {
                            // Infer a synthetic heading level from run formatting
                            // (bold/centered/short) when no Heading style applies.
                            resolved_meta.run_heading_level =
                                resolved_meta.infer_run_heading_level(&visible_text);
                            resolved_meta.has_page_break_before = Some(pending_page_break);
                            blocks.push(DocxRawBlock {
                                kind: resolved_meta.block_kind(),
                                text: visible_text,
                                table: None,
                                layout_hints: resolved_meta.layout_hints(),
                            });
                        }
                    }
                    paragraph_text.clear();
                    in_paragraph = false;
                    pending_page_break = false;
                } else if is_word_tag(&name, b"r") {
                    // Fold this run's formatting into the paragraph aggregate.
                    if run_is_bold {
                        paragraph_meta.is_bold = Some(true);
                    } else if paragraph_meta.is_bold.is_none() {
                        paragraph_meta.is_bold = Some(false);
                    }
                    if run_is_italic {
                        paragraph_meta.is_italic = Some(true);
                    } else if paragraph_meta.is_italic.is_none() {
                        paragraph_meta.is_italic = Some(false);
                    }
                    if let Some(size) = run_font_size {
                        paragraph_meta.max_font_size_half_pts = Some(
                            paragraph_meta
                                .max_font_size_half_pts
                                .map(|existing| existing.max(size))
                                .unwrap_or(size),
                        );
                        paragraph_meta.min_font_size_half_pts = Some(
                            paragraph_meta
                                .min_font_size_half_pts
                                .map(|existing| existing.min(size))
                                .unwrap_or(size),
                        );
                    }
                    in_run = false;
                    in_run_properties = false;
                } else if is_word_tag(&name, b"rPr") {
                    in_run_properties = false;
                } else if is_word_tag(&name, b"t") {
                    in_text_node = false;
                } else if is_word_tag(&name, b"tc") && in_table {
                    current_cell.text = collapse_whitespace(&cell_text);
                    row_cells.push(current_cell.clone());
                    cell_text.clear();
                    current_cell = DocxTableCell::default();
                    in_table_cell_properties = false;
                    in_cell = false;
                } else if is_word_tag(&name, b"tr") && in_table {
                    if row_cells.iter().any(|cell| !cell.text.is_empty()) {
                        table_rows.push(row_cells.clone());
                    }
                    row_cells.clear();
                } else if is_word_tag(&name, b"tbl") {
                    push_docx_table_block(&mut blocks, &table_rows);
                    in_table = false;
                    in_cell = false;
                    table_rows.clear();
                    row_cells.clear();
                    cell_text.clear();
                } else if is_word_tag(&name, b"numPr") {
                    in_numbering_properties = false;
                } else if is_word_tag(&name, b"pPr") {
                    in_paragraph_properties = false;
                } else if is_word_tag(&name, b"sectPr") {
                    in_section_properties = false;
                } else if is_word_tag(&name, b"tcPr") {
                    in_table_cell_properties = false;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    Ok(blocks)
}

fn parse_docx_with_rust_ooxml(
    job: &ImportJob,
    source: &SourceFile,
    upload_path: &Path,
    output_path: &Path,
    mode: &str,
) -> CommandResult<Value> {
    let file = fs::File::open(upload_path)
        .map_err(|error| format!("rust_docx_open_failed:{}:{}", upload_path.display(), error))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("rust_docx_zip_open_failed:{}", error))?;
    let mut document = archive
        .by_name("word/document.xml")
        .map_err(|error| format!("rust_docx_missing_document_xml:{}", error))?;
    let mut document_xml = Vec::new();
    document
        .read_to_end(&mut document_xml)
        .map_err(|error| format!("rust_docx_read_document_xml_failed:{}", error))?;
    drop(document);

    let mut styles = HashMap::<String, DocxStyleDef>::new();
    if let Ok(mut styles_file) = archive.by_name("word/styles.xml") {
        let mut styles_xml = Vec::new();
        styles_file
            .read_to_end(&mut styles_xml)
            .map_err(|error| format!("rust_docx_read_styles_xml_failed:{}", error))?;
        styles = parse_docx_styles_xml(&styles_xml)?;
    }
    let mut numbering_defs = DocxNumberingDefs::default();
    if let Ok(mut numbering_file) = archive.by_name("word/numbering.xml") {
        let mut numbering_xml = Vec::new();
        numbering_file
            .read_to_end(&mut numbering_xml)
            .map_err(|error| format!("rust_docx_read_numbering_xml_failed:{}", error))?;
        numbering_defs = parse_docx_numbering_xml(&numbering_xml)?;
    }

    let mut warnings = Vec::<String>::new();
    let document_markup = String::from_utf8_lossy(&document_xml).to_ascii_lowercase();
    if document_markup.contains("<w:drawing")
        || document_markup.contains("<w:pict")
        || document_markup.contains("<a:blip")
    {
        warnings.push(
            "DOCX contains embedded drawings or images that are not yet copied into student assets; diagram, map, or plan questions require source review before export."
                .to_string(),
        );
    }
    let mut raw_blocks = parse_docx_document_xml(&document_xml, &styles, &numbering_defs)?;
    if raw_blocks.is_empty() {
        warnings.push(
            "DOCX contains no extractable paragraphs or tables; manual review required".to_string(),
        );
        raw_blocks.push(DocxRawBlock {
            kind: "paragraph",
            text: upload_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("document")
                .to_string(),
            table: None,
            layout_hints: None,
        });
    }
    let blocks = raw_blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let mut value =
                document_block(format!("b{:03}", index + 1), &block.text, 1, index, 0.99);
            if block.kind == "table" {
                value["blockType"] = json!("table");
                if let Some(table) = &block.table {
                    value["table"] = table_ir_to_value(table);
                    value["html"] = json!(table_ir_to_html(table));
                } else {
                    value["html"] = json!(markdownish_to_html(&block.text, "table"));
                }
            } else if block.kind == "header" || block.kind == "list" {
                value["blockType"] = json!(block.kind);
                value["html"] = json!(markdownish_to_html(&block.text, block.kind));
            }
            if let Some(layout_hints) = &block.layout_hints {
                value["layoutHints"] = layout_hints.clone();
            }
            value
        })
        .collect::<Vec<_>>();

    let ir = json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": [{
            "pageIndex": 1,
            "width": 595,
            "height": 842,
            "blocks": blocks
        }],
        "assets": [],
        "parser": {
            "provider": "rust-parser:docx:ooxml",
            "version": "0.1.0",
            "mode": mode,
            "warnings": warnings,
            "sourceFileId": source.file_id,
            "sourceStoredName": source.stored_name
        }
    });
    write_json(output_path, &ir)?;
    Ok(ir)
}

fn parse_with_python_sidecar(
    job_id: &str,
    input_path: &Path,
    output_path: &Path,
    mode: &str,
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/python-parser/parser.py")
        .ok_or_else(|| "python_parser_sidecar_missing".to_string())?;
    let python =
        resolve_python_command().ok_or_else(|| "python_runtime_unavailable".to_string())?;
    let output = Command::new(&python.program)
        .args(&python.args)
        .arg(&script)
        .arg("parse")
        .arg("--input")
        .arg(input_path)
        .arg("--output")
        .arg(output_path)
        .arg("--job-id")
        .arg(job_id)
        .arg("--mode")
        .arg(mode)
        .output()
        .map_err(|error| {
            format!(
                "python_parser_spawn_failed:{}:{}:{}",
                python.display(),
                script.display(),
                error
            )
        })?;
    if !output.status.success() {
        return Err(command_failure("python-parser", &output));
    }
    read_json(output_path)
}

pub(crate) fn extract_pdf_images_with_python_sidecar(
    job_id: &str,
    input_path: &Path,
    output_path: &Path,
    asset_dir: &Path,
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/python-parser/parser.py")
        .ok_or_else(|| "python_parser_sidecar_missing".to_string())?;
    let python =
        resolve_python_command().ok_or_else(|| "python_runtime_unavailable".to_string())?;
    let output = Command::new(&python.program)
        .args(&python.args)
        .arg(&script)
        .arg("extract_pdf_images")
        .arg("--input")
        .arg(input_path)
        .arg("--output")
        .arg(output_path)
        .arg("--job-id")
        .arg(job_id)
        .arg("--asset-dir")
        .arg(asset_dir)
        .output()
        .map_err(|error| {
            format!(
                "python_parser_spawn_failed:{}:{}:{}",
                python.display(),
                script.display(),
                error
            )
        })?;
    if !output.status.success() {
        return Err(command_failure("python-parser:extract_pdf_images", &output));
    }
    let mut extraction = read_json(output_path)?;
    stabilize_pdf_image_extraction_fields(&mut extraction, None, None, None, None, None, None);
    write_json(output_path, &extraction)?;
    Ok(extraction)
}

pub(crate) fn image_count_from_extraction(extraction: &Value) -> usize {
    extraction
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|page| {
            page.get("images")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .count()
}

fn extraction_page_count(extraction: &Value) -> usize {
    extraction
        .get("pages")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn rendered_page_count_from_extraction(extraction: &Value) -> usize {
    extraction
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|page| {
            page.get("images")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|image| {
                    image
                        .get("renderedFallback")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        || image.get("renderSource").is_some()
                })
        })
        .count()
}

fn extraction_requires_manual_review(extraction: &Value) -> bool {
    extraction
        .get("requiresManualReview")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || extraction
            .get("failureReason")
            .and_then(Value::as_str)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        || image_count_from_extraction(extraction) == 0
}

fn renderer_provider_from_extraction(extraction: &Value) -> &'static str {
    let mut saw_pymupdf = false;
    let mut saw_poppler = false;
    let mut saw_sips = false;
    let mut saw_rendered = false;
    for image in extraction
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|page| {
            page.get("images")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
    {
        let source = image
            .get("renderSource")
            .and_then(Value::as_str)
            .unwrap_or_default();
        saw_pymupdf |= source.contains("pymupdf");
        saw_poppler |= source.contains("pdftoppm") || source.contains("poppler");
        saw_sips |= source.contains("sips");
        saw_rendered |= !source.is_empty()
            || image
                .get("renderedFallback")
                .and_then(Value::as_bool)
                .unwrap_or(false);
    }
    if saw_pymupdf {
        "pymupdf"
    } else if saw_poppler {
        "poppler"
    } else if saw_sips {
        "system-sips"
    } else if saw_rendered {
        "pdf-page-renderer"
    } else {
        "embedded-pdf-images"
    }
}

fn stabilize_pdf_image_extraction_fields(
    extraction: &mut Value,
    renderer_adapter: Option<&str>,
    renderer_provider: Option<&str>,
    renderer_version: Option<Value>,
    dpi: Option<u64>,
    failure_reason: Option<&str>,
    requires_manual_review: Option<bool>,
) {
    let page_count = extraction_page_count(extraction);
    let rendered_page_count = rendered_page_count_from_extraction(extraction);
    let image_count = image_count_from_extraction(extraction);
    let inferred_provider =
        renderer_provider.unwrap_or_else(|| renderer_provider_from_extraction(extraction));
    let inferred_requires_manual_review =
        requires_manual_review.unwrap_or_else(|| extraction_requires_manual_review(extraction));
    let obj = extraction
        .as_object_mut()
        .expect("PdfImageExtractionV1 should be a JSON object");
    obj.entry("schemaVersion".to_string())
        .or_insert_with(|| json!("PdfImageExtractionV1"));
    if let Some(adapter) = renderer_adapter {
        obj.insert("rendererAdapter".to_string(), json!(adapter));
    } else {
        obj.entry("rendererAdapter".to_string())
            .or_insert_with(|| json!("python-sidecar"));
    }
    obj.entry("rendererProvider".to_string())
        .or_insert_with(|| json!(inferred_provider));
    obj.entry("rendererVersion".to_string())
        .or_insert(renderer_version.unwrap_or(Value::Null));
    obj.insert("pageCount".to_string(), json!(page_count));
    obj.insert("renderedPageCount".to_string(), json!(rendered_page_count));
    obj.entry("dpi".to_string())
        .or_insert(json!(dpi.unwrap_or(180)));
    obj.insert("ocrPerformed".to_string(), json!(false));
    obj.insert(
        "requiresManualReview".to_string(),
        json!(inferred_requires_manual_review),
    );
    obj.entry("imageCount".to_string())
        .or_insert_with(|| json!(image_count));
    obj.entry("cloudPdfVisionEnabled".to_string())
        .or_insert_with(|| json!(cloud_pdf_vision_enabled()));
    obj.entry("localOcrEnabled".to_string())
        .or_insert_with(|| json!(local_ocr_enabled()));
    obj.entry("failureReason".to_string()).or_insert_with(|| {
        failure_reason
            .map(|value| json!(value))
            .unwrap_or(Value::Null)
    });
}

fn merge_rendered_page_images(extraction: &Value, rendered: &Value) -> Value {
    let mut merged = extraction.clone();
    let Some(extracted_pages) = merged.get_mut("pages").and_then(Value::as_array_mut) else {
        return extraction.clone();
    };
    let rendered_pages = rendered
        .get("pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for extracted_page in extracted_pages.iter_mut() {
        let page_index = extracted_page
            .get("pageIndex")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let Some(rendered_page) = rendered_pages
            .iter()
            .find(|page| page.get("pageIndex").and_then(Value::as_u64).unwrap_or(0) == page_index)
        else {
            continue;
        };
        let rendered_images = rendered_page
            .get("images")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if rendered_images.is_empty() {
            continue;
        }
        let images = extracted_page
            .as_object_mut()
            .and_then(|obj| obj.get_mut("images"))
            .and_then(Value::as_array_mut);
        if let Some(images) = images {
            images.extend(rendered_images);
        }
    }
    let merged_has_images = image_count_from_extraction(&merged) > 0;
    if let Some(obj) = merged.as_object_mut() {
        let mut warnings = obj
            .get("warnings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(rendered_warnings) = rendered.get("warnings").and_then(Value::as_array) {
            warnings.extend(rendered_warnings.iter().cloned());
        }
        obj.insert("warnings".to_string(), Value::Array(warnings));
        obj.insert(
            "renderedFallback".to_string(),
            json!(
                extraction
                    .get("renderedFallback")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    || rendered
                        .get("renderedFallback")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
            ),
        );
        if !merged_has_images {
            if let Some(reason) = rendered
                .get("failureReason")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                obj.insert("failureReason".to_string(), json!(reason));
            }
            if rendered
                .get("requiresManualReview")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                obj.insert("requiresManualReview".to_string(), json!(true));
            }
        }
    }
    stabilize_pdf_image_extraction_fields(&mut merged, None, None, None, None, None, None);
    merged
}

pub(crate) fn render_pdf_pages_with_adapter(
    job_id: &str,
    input_path: &Path,
    output_path: &Path,
    asset_dir: &Path,
    prior_warnings: Vec<String>,
) -> CommandResult<Value> {
    match pdf_renderer_setting().as_str() {
        "none" => render_pdf_pages_unsupported(
            job_id,
            input_path,
            output_path,
            prior_warnings,
            "renderer_disabled",
            "PDF page rendering is disabled by EPIC8_PDF_RENDERER=none; manual review required for scanned PDFs.",
        ),
        "sips" => {
            #[cfg(target_os = "macos")]
            {
                render_pdf_pages_with_macos_sips(
                    job_id,
                    input_path,
                    output_path,
                    asset_dir,
                    prior_warnings,
                )
            }
            #[cfg(not(target_os = "macos"))]
            {
                render_pdf_pages_unsupported(
                    job_id,
                    input_path,
                    output_path,
                    prior_warnings,
                    "renderer_sips_unsupported_on_platform",
                    "sips PDF rendering is only available on macOS; manual review required for scanned PDFs.",
                )
            }
        }
        "pdfium" => render_pdf_pages_with_pdfium_or_fallback(
            job_id,
            input_path,
            output_path,
            asset_dir,
            prior_warnings,
        ),
        "poppler" => render_pdf_pages_unsupported(
            job_id,
            input_path,
            output_path,
            prior_warnings,
            "renderer_poppler_unimplemented",
            "Poppler page renderer is reserved but not bundled in this runtime; manual review required for scanned PDFs.",
        ),
        "pymupdf" => render_pdf_pages_unsupported(
            job_id,
            input_path,
            output_path,
            prior_warnings,
            "renderer_pymupdf_unimplemented",
            "PyMuPDF page renderer is reserved but not bundled in this Rust runtime; manual review required for scanned PDFs.",
        ),
        _ => {
            #[cfg(target_os = "macos")]
            {
                render_pdf_pages_with_macos_sips(
                    job_id,
                    input_path,
                    output_path,
                    asset_dir,
                    prior_warnings,
                )
            }
            #[cfg(target_os = "windows")]
            {
                render_pdf_pages_with_pdfium_or_fallback(
                    job_id,
                    input_path,
                    output_path,
                    asset_dir,
                    prior_warnings,
                )
            }
            #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
            {
                render_pdf_pages_unsupported(
                    job_id,
                    input_path,
                    output_path,
                    prior_warnings,
                    "renderer_unsupported_platform",
                    "PDF page rendering is unsupported on this platform; use cloud PDF vision if available or manual transcription/review.",
                )
            }
        }
    }
}

/// Try the bundled pdfium page renderer; if the native library is missing or
/// rendering fails, fall back to the unsupported stub so the caller still gets
/// a well-formed `PdfImageExtractionV1` (with `requiresManualReview: true`).
fn render_pdf_pages_with_pdfium_or_fallback(
    job_id: &str,
    input_path: &Path,
    output_path: &Path,
    asset_dir: &Path,
    prior_warnings: Vec<String>,
) -> CommandResult<Value> {
    match crate::pdf_geometry::render_pdf_pages_with_pdfium(
        job_id,
        input_path,
        output_path,
        asset_dir,
        prior_warnings.clone(),
    ) {
        Ok(extraction) => Ok(extraction),
        Err(failure) => {
            let reason = if failure.starts_with("pdfium_bind") {
                "renderer_pdfium_library_unavailable"
            } else {
                "renderer_pdfium_failed"
            };
            let message = format!(
                "PDFium page renderer could not be used ({}); falling back to manual review for scanned PDFs.",
                failure
            );
            render_pdf_pages_unsupported(
                job_id,
                input_path,
                output_path,
                prior_warnings,
                reason,
                &message,
            )
        }
    }
}

fn render_pdf_pages_unsupported(
    job_id: &str,
    input_path: &Path,
    output_path: &Path,
    prior_warnings: Vec<String>,
    failure_reason: &str,
    message: &str,
) -> CommandResult<Value> {
    let mut warnings = prior_warnings
        .into_iter()
        .filter(|warning| !warning.trim().is_empty())
        .collect::<Vec<_>>();
    warnings.push(message.to_string());
    let provider = if cfg!(target_os = "windows") {
        "windows-pdfium"
    } else if cfg!(target_os = "macos") {
        "system-sips"
    } else {
        "unsupported-platform"
    };
    let adapter = if cfg!(target_os = "windows") {
        "windows-pdfium"
    } else if cfg!(target_os = "macos") {
        "macos-sips"
    } else {
        "unsupported-renderer"
    };
    let mut extraction = json!({
        "schemaVersion": "PdfImageExtractionV1",
        "jobId": job_id,
        "sourcePath": input_path.to_string_lossy(),
        "rendererAdapter": adapter,
        "rendererProvider": provider,
        "rendererVersion": null,
        "renderPurpose": "vision-llm-transcription-input",
        "pageCount": 0,
        "renderedPageCount": 0,
        "dpi": 180,
        "ocrPerformed": false,
        "failureReason": failure_reason,
        "requiresManualReview": true,
        "pages": [],
        "warnings": warnings,
        "renderedFallback": false
    });
    stabilize_pdf_image_extraction_fields(
        &mut extraction,
        Some(adapter),
        Some(provider),
        Some(Value::Null),
        Some(180),
        Some(failure_reason),
        Some(true),
    );
    write_json(output_path, &extraction)?;
    Ok(extraction)
}

fn render_pdf_pages_with_macos_sips(
    job_id: &str,
    input_path: &Path,
    output_path: &Path,
    asset_dir: &Path,
    prior_warnings: Vec<String>,
) -> CommandResult<Value> {
    fs::create_dir_all(asset_dir)
        .map_err(|error| format!("create_sips_asset_dir:{}:{}", asset_dir.display(), error))?;
    let rendered_path = asset_dir.join("page-001-rendered.png");
    let output = Command::new("sips")
        .arg("-s")
        .arg("format")
        .arg("png")
        .arg(input_path)
        .arg("--out")
        .arg(&rendered_path)
        .output()
        .map_err(|error| format!("sips_render_spawn_failed:{}", error))?;
    if !output.status.success() {
        return Err(command_failure("sips:render_pdf_page", &output));
    }
    let bytes = fs::read(&rendered_path).map_err(|error| {
        format!(
            "read_sips_rendered_image:{}:{}",
            rendered_path.display(),
            error
        )
    })?;
    if bytes.is_empty() {
        return Err(format!(
            "sips_rendered_image_empty:{}",
            rendered_path.display()
        ));
    }

    let mut warnings = prior_warnings
        .into_iter()
        .filter(|warning| !warning.trim().is_empty())
        .collect::<Vec<_>>();
    warnings.push(
        "used Rust macOS sips rendered-page fallback for vision transcription; verify page image coverage before publish"
            .to_string(),
    );
    warnings.push(
        "sips fallback currently renders a preview image, not full OCR or guaranteed multi-page coverage"
            .to_string(),
    );
    let mut extraction = json!({
        "schemaVersion": "PdfImageExtractionV1",
        "jobId": job_id,
        "sourcePath": input_path.to_string_lossy(),
        "rendererAdapter": "macos-sips",
        "rendererProvider": "system-sips",
        "rendererVersion": null,
        "renderPurpose": "vision-llm-transcription-input",
        "pageCount": 1,
        "renderedPageCount": 1,
        "dpi": 180,
        "ocrPerformed": false,
        "failureReason": null,
        "requiresManualReview": false,
        "futureAdapter": "pdfium-render-page-renderer",
        "pages": [{
            "pageIndex": 1,
            "width": 595,
            "height": 842,
            "images": [{
                "assetId": "pdf-page-1-rendered",
                "pageIndex": 1,
                "fileName": "page-001-rendered.png",
                "path": rendered_path.to_string_lossy(),
                "mimeType": "image/png",
                "width": 0,
                "height": 0,
                "sha256": hash_bytes(&bytes),
                "sizeBytes": bytes.len() as u64,
                "renderedFallback": true,
                "renderSource": "rust-macos-sips"
            }]
        }],
        "warnings": warnings,
        "renderedFallback": true
    });
    stabilize_pdf_image_extraction_fields(
        &mut extraction,
        Some("macos-sips"),
        Some("system-sips"),
        Some(Value::Null),
        Some(180),
        None,
        Some(false),
    );
    write_json(output_path, &extraction)?;
    Ok(extraction)
}

pub(crate) fn extract_pdf_images_for_vision(
    job_id: &str,
    input_path: &Path,
    output_path: &Path,
    asset_dir: &Path,
) -> CommandResult<Value> {
    match extract_pdf_images_with_python_sidecar(job_id, input_path, output_path, asset_dir) {
        Ok(extraction) => {
            let warnings = extraction
                .get("warnings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            match render_pdf_pages_with_adapter(
                job_id,
                input_path,
                output_path,
                asset_dir,
                warnings,
            ) {
                Ok(rendered) => {
                    let merged = merge_rendered_page_images(&extraction, &rendered);
                    write_json(output_path, &merged)?;
                    Ok(merged)
                }
                Err(_) if image_count_from_extraction(&extraction) > 0 => Ok(extraction),
                Err(_) => {
                    let warnings = extraction
                        .get("warnings")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>();
                    render_pdf_pages_with_adapter(
                        job_id,
                        input_path,
                        output_path,
                        asset_dir,
                        warnings,
                    )
                }
            }
        }
        Err(error) => render_pdf_pages_with_adapter(
            job_id,
            input_path,
            output_path,
            asset_dir,
            vec![format!(
                "python PDF image extraction failed; used platform PDF renderer fallback: {}",
                error
            )],
        ),
    }
}

pub(crate) fn parse_source_document(
    job: &ImportJob,
    source: &SourceFile,
    upload_path: &Path,
    output_path: &Path,
    mode: &str,
) -> CommandResult<Value> {
    if matches!(source.file_type.as_str(), "txt" | "md") {
        match parse_text_with_rust_parser(job, source, upload_path, output_path, mode) {
            Ok(ir) => return Ok(ir),
            Err(error) => {
                let fallback =
                    parse_with_python_sidecar(&job.job_id, upload_path, output_path, mode);
                return match fallback {
                    Ok(mut ir) => {
                        append_parser_warning(
                            &mut ir,
                            format!(
                                "rust text parser failed; used Python parser fallback: {}",
                                error
                            ),
                        );
                        write_json(output_path, &ir)?;
                        Ok(ir)
                    }
                    Err(python_error) => Ok(parser_failure_document_ir(
                        job,
                        source,
                        mode,
                        &format!("{}; python fallback failed: {}", error, python_error),
                    )),
                };
            }
        }
    }
    if source.file_type == "pdf" {
        match parse_pdf_with_rust_text_extractor(job, source, upload_path, output_path, mode) {
            Ok(ir) => return Ok(ir),
            Err(error) => {
                let fallback =
                    parse_with_python_sidecar(&job.job_id, upload_path, output_path, mode);
                return match fallback {
                    Ok(mut ir) => {
                        append_parser_warning(
                            &mut ir,
                            format!(
                                "rust pdf-extract failed; used Python parser fallback: {}",
                                error
                            ),
                        );
                        write_json(output_path, &ir)?;
                        Ok(ir)
                    }
                    Err(python_error) => Ok(parser_failure_document_ir(
                        job,
                        source,
                        mode,
                        &format!("{}; python fallback failed: {}", error, python_error),
                    )),
                };
            }
        }
    }
    if source.file_type == "docx" {
        match parse_docx_with_rust_ooxml(job, source, upload_path, output_path, mode) {
            Ok(ir) => return Ok(ir),
            Err(error) => {
                let fallback =
                    parse_with_python_sidecar(&job.job_id, upload_path, output_path, mode);
                return match fallback {
                    Ok(mut ir) => {
                        append_parser_warning(
                            &mut ir,
                            format!(
                                "rust docx OOXML parser failed; used Python parser fallback: {}",
                                error
                            ),
                        );
                        write_json(output_path, &ir)?;
                        Ok(ir)
                    }
                    Err(python_error) => Ok(parser_failure_document_ir(
                        job,
                        source,
                        mode,
                        &format!("{}; python fallback failed: {}", error, python_error),
                    )),
                };
            }
        }
    }

    parse_with_python_sidecar(&job.job_id, upload_path, output_path, mode)
        .or_else(|error| Ok(parser_failure_document_ir(job, source, mode, &error)))
}

pub(crate) fn parser_failure_document_ir(
    job: &ImportJob,
    source: &SourceFile,
    mode: &str,
    error: &str,
) -> Value {
    let warning = format!(
        "parser failed for {} source {}; manual review required: {}",
        source.file_type, source.original_name, error
    );
    json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": [{
            "pageIndex": 1,
            "width": 595,
            "height": 842,
            "blocks": [{
                "blockId": "parser-failure",
                "blockType": "paragraph",
                "text": format!("[Parser failed for {}. Manual review required.]", source.original_name),
                "html": format!("<p>{}</p>", html_escape(&format!("[Parser failed for {}. Manual review required.]", source.original_name))),
                "bbox": [72, 72, 520, 120],
                "confidence": 0.0,
                "roleHint": "ignore"
            }]
        }],
        "assets": [],
        "parser": {
            "provider": "python-parser-sidecar:failure",
            "version": "0.3.0",
            "mode": mode,
            "warnings": [warning, "no-sample-content-generated"],
            "sourceFileId": source.file_id,
            "sourceStoredName": source.stored_name
        }
    })
}

pub(crate) fn missing_source_document_ir(job: &ImportJob, mode: &str, reason: &str) -> Value {
    json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": [{
            "pageIndex": 1,
            "width": 595,
            "height": 842,
            "blocks": [{
                "blockId": "source-missing",
                "blockType": "paragraph",
                "text": "[No main source file is available. Manual import/review required.]",
                "html": "<p>[No main source file is available. Manual import/review required.]</p>",
                "bbox": [72, 72, 520, 120],
                "confidence": 0.0,
                "roleHint": "ignore"
            }]
        }],
        "assets": [],
        "parser": {
            "provider": "local-parser:source-missing",
            "version": "0.3.0",
            "mode": mode,
            "warnings": [format!("main source unavailable; manual review required: {}", reason), "no-sample-content-generated"],
            "sourceFileId": null,
            "sourceStoredName": null
        }
    })
}
