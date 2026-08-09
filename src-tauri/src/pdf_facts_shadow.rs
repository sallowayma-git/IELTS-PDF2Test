use crate::artifact_store::write_canonical_json_atomic;
use crate::job_store::load_job;
use crate::schema::document_ir_v2::DocumentIRV2;
use crate::util::{read_json, write_text};
use crate::{CommandResult, ImportJob, SourceFile};
use chrono::Utc;
use pdf_extract::{Document, MediaBox, OutputDev, OutputError, Transform};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub(crate) const SHADOW_ARTIFACT_FILE: &str = "document-ir-v2.shadow.json";
pub(crate) const SHADOW_ERROR_FILE: &str = "document-ir-v2.shadow.error.json";
pub(crate) const SHADOW_OVERLAY_FILE: &str = "document-ir-v2.shadow.overlay.svg";

struct RawGlyph {
    value: Value,
    area: f64,
    unicode_error: bool,
    text_len: usize,
}

struct PageFacts {
    page_num: u32,
    width: f64,
    height: f64,
    media_box: (f64, f64, f64, f64),
    rotation: u16,
    glyphs: Vec<RawGlyph>,
}

struct PdfFactsCollector {
    pages: Vec<PageFacts>,
    current: Option<PageFacts>,
    flip_ctm: Option<Transform>,
    page_rotations: BTreeMap<u32, u16>,
    source_file_id: String,
    source_hash: String,
    char_offset: u32,
}

impl PdfFactsCollector {
    fn new(
        page_rotations: BTreeMap<u32, u16>,
        source_file_id: String,
        source_hash: String,
    ) -> Self {
        Self {
            pages: Vec::new(),
            current: None,
            flip_ctm: None,
            page_rotations,
            source_file_id,
            source_hash,
            char_offset: 0,
        }
    }
}

impl OutputDev for PdfFactsCollector {
    fn begin_page(
        &mut self,
        page_num: u32,
        media_box: &MediaBox,
        _art_box: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), OutputError> {
        let width = (media_box.urx - media_box.llx).abs().max(1.0);
        let height = (media_box.ury - media_box.lly).abs().max(1.0);
        let rotation = self.page_rotations.get(&page_num).copied().unwrap_or(0);
        self.current = Some(PageFacts {
            page_num,
            width,
            height,
            media_box: (media_box.llx, media_box.lly, media_box.urx, media_box.ury),
            rotation,
            glyphs: Vec::new(),
        });
        self.flip_ctm = Some(Transform::row_major(
            1.0,
            0.0,
            0.0,
            -1.0,
            0.0,
            media_box.ury - media_box.lly,
        ));
        self.char_offset = 0;
        Ok(())
    }

    fn end_page(&mut self) -> Result<(), OutputError> {
        self.flip_ctm = None;
        if let Some(page) = self.current.take() {
            self.pages.push(page);
        }
        Ok(())
    }

    fn output_character(
        &mut self,
        trm: &Transform,
        width: f64,
        _spacing: f64,
        font_size: f64,
        character: &str,
    ) -> Result<(), OutputError> {
        let Some(flip_ctm) = self.flip_ctm.clone() else {
            return Ok(());
        };
        let Some(page) = self.current.as_mut() else {
            return Ok(());
        };

        let positioned = trm.post_transform(&flip_ctm);
        let safe_font_size = finite_positive(font_size, 1.0);
        let safe_width = finite_positive(width.abs(), 0.01);
        let advance = (safe_width * safe_font_size).max(0.01);
        let glyph_height = safe_font_size.max(0.5);

        let origin = (positioned.m31, positioned.m32);
        let advance_point = (
            origin.0 + positioned.m11 * advance,
            origin.1 + positioned.m12 * advance,
        );
        let height_point = (
            origin.0 + positioned.m21 * glyph_height,
            origin.1 + positioned.m22 * glyph_height,
        );
        let far_point = (
            advance_point.0 + positioned.m21 * glyph_height,
            advance_point.1 + positioned.m22 * glyph_height,
        );
        let corners = [origin, advance_point, far_point, height_point];
        let min_x = corners
            .iter()
            .map(|point| point.0)
            .fold(f64::INFINITY, f64::min);
        let max_x = corners
            .iter()
            .map(|point| point.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = corners
            .iter()
            .map(|point| point.1)
            .fold(f64::INFINITY, f64::min);
        let max_y = corners
            .iter()
            .map(|point| point.1)
            .fold(f64::NEG_INFINITY, f64::max);

        let unicode_error = character.is_empty() || character.contains('\u{fffd}');
        let text = if character.is_empty() {
            "�".to_string()
        } else {
            character.to_string()
        };
        let text_len = text.chars().count().max(1);
        let start = self.char_offset;
        self.char_offset = self.char_offset.saturating_add(text_len as u32);
        let glyph_id = format!("p{:03}-g{:05}", page.page_num, page.glyphs.len() + 1);
        let rect = rect_value(
            min_x,
            min_y,
            (max_x - min_x).max(0.01),
            (max_y - min_y).max(0.01),
            "top-left",
            page.rotation,
        );
        let quad = json!({
            "points": corners
                .iter()
                .flat_map(|point| [clean_number(point.0), clean_number(point.1)])
                .collect::<Vec<_>>(),
            "unit": "pt",
            "origin": "top-left"
        });
        let source_anchor = json!({
            "sourceFileId": self.source_file_id,
            "pageIndex": page.page_num.saturating_sub(1),
            "nodeIds": [glyph_id.clone()],
            "bbox": rect.clone(),
            "charRange": {"start": start, "end": self.char_offset},
            "extractionMode": "pdf_native",
            "sourceHash": self.source_hash
        });
        let value = json!({
            "id": glyph_id,
            "text": text,
            "bbox": rect,
            "quad": quad,
            "origin": {"x": clean_number(origin.0), "y": clean_number(origin.1)},
            "baseline": clean_number(origin.1),
            "angleRad": clean_number(positioned.m12.atan2(positioned.m11)),
            "style": {"fontSizePt": clean_number(safe_font_size)},
            "unicodeMapError": unicode_error,
            "hidden": false,
            "confidence": if unicode_error {0.25} else {0.98},
            "source": "native",
            "sourceAnchor": source_anchor
        });
        page.glyphs.push(RawGlyph {
            value,
            area: ((max_x - min_x).max(0.01) * (max_y - min_y).max(0.01)).max(0.0),
            unicode_error,
            text_len,
        });
        Ok(())
    }

    fn begin_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_line(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
}

pub(crate) fn write_pdf_facts_shadow(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
    output_path: &Path,
) -> CommandResult<Value> {
    let value = extract_pdf_facts_shadow(job, source, input_path)?;
    write_canonical_json_atomic(output_path, &value)?;
    Ok(value)
}

pub(crate) fn extract_pdf_facts_shadow(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
) -> CommandResult<Value> {
    let extraction_started_at = Utc::now();
    let (document, load_warnings) = load_document_with_xref_repair(input_path)?;
    let page_rotations = document
        .get_pages()
        .into_iter()
        .map(|(page_num, object_id)| (page_num, page_rotation(&document, object_id)))
        .collect::<BTreeMap<_, _>>();
    let mut collector = PdfFactsCollector::new(
        page_rotations,
        source.file_id.clone(),
        source.sha256.clone(),
    );
    pdf_extract::output_doc(&document, &mut collector)
        .map_err(|error| format!("pdf_facts_shadow_extract_failed:{}", error))?;

    let pages = collector
        .pages
        .into_iter()
        .map(page_value)
        .collect::<Vec<_>>();
    let coverage_ledger = coverage_ledger_from_pages(&pages);
    let pages = pages
        .into_iter()
        .map(|mut page| {
            if let Some(object) = page.as_object_mut() {
                object.remove("_coverage");
            }
            page
        })
        .collect::<Vec<_>>();
    let source_file = json!({
        "sourceFileId": source.file_id,
        "originalName": source.original_name,
        "mediaType": "application/pdf",
        "sha256": source.sha256,
        "byteLength": source.size_bytes,
        "role": source_role(&source.role)
    });
    let extraction_completed_at = Utc::now();
    let value = json!({
        "schemaVersion": "DocumentIRV2",
        "documentId": format!("document-{}", &source.sha256[..source.sha256.len().min(16)]),
        "jobId": job.job_id,
        "sourceFiles": [source_file],
        "pages": pages,
        "assets": [],
        "coverageLedger": coverage_ledger,
        "parser": {
            "provider": "rust-parser:pdf:pdf-extract:shadow",
            "providerVersion": "0.10.0",
            "extractionStartedAt": extraction_started_at.to_rfc3339(),
            "extractionCompletedAt": extraction_completed_at.to_rfc3339(),
            "options": {
                "featureFlag": "documentIrV2Shadow",
                "coordinateOrigin": "top-left",
                "pageIndexBase": 0,
                "charRangeScope": "page",
                "semanticLayers": false
            },
            "warnings": load_warnings
        }
    });
    let typed = serde_json::from_value::<DocumentIRV2>(value)
        .map_err(|error| format!("pdf_facts_shadow_schema_validation_failed:{}", error))?;
    if !typed.is_supported_schema_version() {
        return Err("pdf_facts_shadow_schema_version_unsupported".to_string());
    }
    serde_json::to_value(typed).map_err(|error| error.to_string())
}

pub(crate) fn debug_document_ir_v2_overlay_core(root: &Path, job_id: &str) -> CommandResult<Value> {
    load_job(root, job_id)?;
    let job_dir = crate::util::safe_job_dir(root, job_id)?;
    let shadow_path = job_dir.join(SHADOW_ARTIFACT_FILE);
    let shadow: Value = read_json(&shadow_path)?;
    let pages = shadow
        .get("pages")
        .and_then(Value::as_array)
        .ok_or_else(|| "document_ir_v2_shadow_pages_missing".to_string())?;
    let mut page_offsets = Vec::with_capacity(pages.len());
    let mut total_height = 0.0;
    let mut max_width: f64 = 1.0;
    for page in pages {
        let width = number_at(page, "/widthPt").unwrap_or(1.0).max(1.0);
        let height = number_at(page, "/heightPt").unwrap_or(1.0).max(1.0);
        page_offsets.push((total_height, width, height));
        total_height += height + 24.0;
        max_width = max_width.max(width);
    }

    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{max_width:.3}\" height=\"{total_height:.3}\" viewBox=\"0 0 {max_width:.3} {total_height:.3}\">"
    );
    svg.push_str("<style>text{font-family:monospace;fill:#154734} .page{fill:#fffdf7;stroke:#8a8175;stroke-width:.8} .glyph{fill:none;stroke:#d1495b;stroke-width:.45}</style>");
    let mut glyph_count = 0usize;
    for (page_index, page) in pages.iter().enumerate() {
        let (offset, width, height) = page_offsets[page_index];
        svg.push_str(&format!(
            "<g data-page-index=\"{page_index}\" transform=\"translate(0 {offset:.3})\"><rect class=\"page\" x=\"0\" y=\"0\" width=\"{width:.3}\" height=\"{height:.3}\"/>"
        ));
        if let Some(glyphs) = page.get("glyphs").and_then(Value::as_array) {
            for glyph in glyphs {
                let bbox = glyph.get("bbox").unwrap_or(&Value::Null);
                let x = number_at(bbox, "/x").unwrap_or(0.0);
                let y = number_at(bbox, "/y").unwrap_or(0.0);
                let glyph_width = number_at(bbox, "/width").unwrap_or(0.01).max(0.01);
                let glyph_height = number_at(bbox, "/height").unwrap_or(0.01).max(0.01);
                let text = xml_escape(glyph.get("text").and_then(Value::as_str).unwrap_or("�"));
                svg.push_str(&format!(
                    "<rect class=\"glyph\" x=\"{x:.3}\" y=\"{y:.3}\" width=\"{glyph_width:.3}\" height=\"{glyph_height:.3}\"/><text x=\"{x:.3}\" y=\"{:.3}\" font-size=\"{:.3}\">{text}</text>",
                    y + glyph_height,
                    number_at(glyph.get("style").unwrap_or(&Value::Null), "/fontSizePt")
                        .unwrap_or(6.0)
                        .clamp(3.0, 18.0)
                ));
                glyph_count += 1;
            }
        }
        svg.push_str("</g>");
    }
    svg.push_str("</svg>");
    let overlay_path = job_dir.join("debug").join(SHADOW_OVERLAY_FILE);
    write_text(&overlay_path, &svg)?;
    Ok(json!({
        "shadowPath": shadow_path.to_string_lossy(),
        "overlayPath": overlay_path.to_string_lossy(),
        "pageCount": pages.len(),
        "glyphCount": glyph_count
    }))
}

fn page_value(page: PageFacts) -> Value {
    let native_character_count = page
        .glyphs
        .iter()
        .map(|glyph| glyph.text_len)
        .sum::<usize>();
    let unicode_error_count = page
        .glyphs
        .iter()
        .filter(|glyph| glyph.unicode_error)
        .count();
    let unicode_error_ratio = if page.glyphs.is_empty() {
        0.0
    } else {
        unicode_error_count as f64 / page.glyphs.len() as f64
    };
    let text_coverage_ratio = (page.glyphs.iter().map(|glyph| glyph.area).sum::<f64>()
        / (page.width * page.height))
        .clamp(0.0, 1.0);
    let classification = if page.glyphs.is_empty() {
        "scanned"
    } else if unicode_error_ratio > 0.1 {
        "garbled"
    } else {
        "born_digital"
    };
    let mut warnings = Vec::new();
    if page.glyphs.is_empty() {
        warnings.push(
            "no native glyphs extracted; page may be scanned or image-only; OCR classification is deferred"
                .to_string(),
        );
    }
    if unicode_error_count > 0 {
        warnings.push(format!(
            "{} native glyphs had no reliable Unicode mapping",
            unicode_error_count
        ));
    }
    let requires_ocr_regions = if page.glyphs.is_empty() {
        vec![rect_value(
            0.0,
            0.0,
            page.width,
            page.height,
            "top-left",
            page.rotation,
        )]
    } else {
        Vec::new()
    };
    let glyph_values = page
        .glyphs
        .iter()
        .map(|glyph| glyph.value.clone())
        .collect::<Vec<_>>();
    let coverage = page
        .glyphs
        .iter()
        .filter_map(|glyph| {
            glyph.value.get("id").and_then(Value::as_str).map(|id| {
                json!({
                    "sourceNodeId": id,
                    "disposition": "unassigned",
                    "targetIds": [],
                    "reason": "PR-02 records physical facts only; semantic assignment is deferred"
                })
            })
        })
        .collect::<Vec<_>>();
    json!({
        "pageIndex": page.page_num.saturating_sub(1),
        "widthPt": clean_number(page.width),
        "heightPt": clean_number(page.height),
        "rotation": page.rotation,
        "mediaBox": rect_value(page.media_box.0, page.media_box.1, page.width, page.height, "bottom-left", page.rotation),
        "glyphs": glyph_values,
        "spans": [],
        "lines": [],
        "regions": [],
        "vectorPaths": [],
        "tables": [],
        "assetIds": [],
        "readingOrder": [],
        "quality": {
            "classification": classification,
            "nativeCharacterCount": native_character_count,
            "unicodeErrorRatio": clean_number(unicode_error_ratio),
            "duplicateTextRatio": 0.0,
            "imageCoverageRatio": 0.0,
            "textCoverageRatio": clean_number(text_coverage_ratio),
            "rotationConfidence": if matches!(page.rotation, 0 | 90 | 180 | 270) {1.0} else {0.0},
            "requiresOcrRegions": requires_ocr_regions,
            "warnings": warnings
        },
        "_coverage": coverage
    })
}

fn load_document_with_xref_repair(input_path: &Path) -> CommandResult<(Document, Vec<String>)> {
    let bytes =
        fs::read(input_path).map_err(|error| format!("pdf_facts_shadow_read_failed:{}", error))?;
    match Document::load_mem(&bytes) {
        Ok(document) if !classic_stream_lengths_need_repair(&bytes) => Ok((document, Vec::new())),
        Ok(document) => {
            let Some(repaired) = repair_classic_pdf_structure(&bytes) else {
                return Ok((
                    document,
                    vec![
                        "detected a classic PDF stream-length mismatch, but no safe shadow repair was available"
                            .to_string(),
                    ],
                ));
            };
            match Document::load_mem(&repaired) {
                Ok(document) => Ok((
                    document,
                    vec![
                        "repaired malformed classic PDF stream length before native shadow extraction"
                            .to_string(),
                    ],
                )),
                Err(error) => Ok((
                    document,
                    vec![format!(
                        "detected a classic PDF stream-length mismatch, but shadow repair failed; original retained: {}",
                        error
                    )],
                )),
            }
        }
        Err(original_error) => {
            let repaired = repair_classic_pdf_structure(&bytes)
                .ok_or_else(|| format!("pdf_facts_shadow_load_failed:{}", original_error))?;
            let document = Document::load_mem(&repaired).map_err(|repaired_error| {
                format!(
                    "pdf_facts_shadow_load_failed:{}; xref repair failed:{}",
                    original_error, repaired_error
                )
            })?;
            Ok((
                document,
                vec![format!(
                    "repaired malformed classic PDF xref or stream length before native shadow extraction: {}",
                    original_error
                )],
            ))
        }
    }
}

fn classic_stream_lengths_need_repair(bytes: &[u8]) -> bool {
    let Some(xref_position) = token_positions(bytes, b"xref").last().copied() else {
        return false;
    };
    repair_stream_lengths(&bytes[..xref_position]) != bytes[..xref_position]
}

fn repair_classic_pdf_structure(bytes: &[u8]) -> Option<Vec<u8>> {
    let xref_position = token_positions(bytes, b"xref").last().copied()?;
    let trailer_position = token_positions(&bytes[xref_position + 4..], b"trailer")
        .first()
        .map(|position| xref_position + 4 + position)?;
    let startxref_position = token_positions(bytes, b"startxref").last().copied()?;
    let eof_position = token_positions(bytes, b"%%EOF").last().copied()?;
    let body = repair_stream_lengths(&bytes[..xref_position]);
    let object_offsets = indirect_object_offsets(&body);
    if object_offsets.is_empty() {
        return None;
    }
    let max_object_id = *object_offsets.keys().max()?;
    let xref_start = body.len();
    let mut canonical_xref = Vec::new();
    canonical_xref.extend_from_slice(b"xref\r\n");
    canonical_xref.extend_from_slice(format!("0 {}\r\n", max_object_id + 1).as_bytes());
    canonical_xref.extend_from_slice(b"0000000000 65535 f\r\n");
    for object_id in 1..=max_object_id {
        if let Some(offset) = object_offsets.get(&object_id) {
            canonical_xref.extend_from_slice(format!("{:010} 00000 n\r\n", offset).as_bytes());
        } else {
            canonical_xref.extend_from_slice(b"0000000000 65535 f\r\n");
        }
    }
    canonical_xref.extend_from_slice(&bytes[trailer_position..startxref_position]);
    canonical_xref.extend_from_slice(format!("startxref\r\n{}\r\n%%EOF", xref_start).as_bytes());

    let mut repaired = body;
    repaired.extend_from_slice(&canonical_xref);
    repaired.extend_from_slice(&bytes[eof_position + b"%%EOF".len()..]);
    Some(repaired)
}

fn repair_stream_lengths(bytes: &[u8]) -> Vec<u8> {
    let stream_positions = token_positions(bytes, b"stream");
    let mut replacements = Vec::new();
    for stream_position in stream_positions {
        let Some(data_start) = stream_data_start(bytes, stream_position) else {
            continue;
        };
        let Some(endstream_position) = token_positions(&bytes[data_start..], b"endstream")
            .first()
            .map(|position| data_start + position)
        else {
            continue;
        };
        let Some(object_marker) = token_positions(&bytes[..stream_position], b"obj")
            .last()
            .copied()
        else {
            continue;
        };
        let length_position =
            token_positions(&bytes[object_marker + 3..stream_position], b"/Length")
                .last()
                .copied()
                .map(|position| object_marker + 3 + position);
        let Some(length_position) = length_position else {
            continue;
        };
        let mut number_start = length_position + b"/Length".len();
        while bytes
            .get(number_start)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            number_start += 1;
        }
        let mut number_end = number_start;
        while bytes
            .get(number_end)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            number_end += 1;
        }
        if number_end == number_start {
            continue;
        }
        let mut after_number = number_end;
        while bytes
            .get(after_number)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            after_number += 1;
        }
        if bytes
            .get(after_number)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let replacement = (endstream_position - data_start).to_string().into_bytes();
        if bytes[number_start..number_end] != replacement {
            replacements.push((number_start, number_end, replacement));
        }
    }

    if replacements.is_empty() {
        return bytes.to_vec();
    }
    let mut repaired = bytes.to_vec();
    for (start, end, replacement) in replacements.into_iter().rev() {
        repaired.splice(start..end, replacement);
    }
    repaired
}

fn stream_data_start(bytes: &[u8], stream_position: usize) -> Option<usize> {
    let mut data_start = stream_position + b"stream".len();
    match bytes.get(data_start..data_start + 2) {
        Some(b"\r\n") => data_start += 2,
        Some(b"\n\r") => data_start += 2,
        Some(b"\n") | Some(b"\r") => data_start += 1,
        _ => return None,
    }
    Some(data_start)
}

fn indirect_object_offsets(bytes: &[u8]) -> BTreeMap<usize, usize> {
    let mut offsets = BTreeMap::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if index > 0 && !bytes[index - 1].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        let Some((object_id, after_id)) = parse_decimal_at(bytes, index) else {
            index += 1;
            continue;
        };
        let mut cursor = after_id;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let Some((_generation, after_generation)) = parse_decimal_at(bytes, cursor) else {
            index += 1;
            continue;
        };
        cursor = after_generation;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor..cursor + 3) == Some(b"obj")
            && bytes
                .get(cursor + 3)
                .map(|byte| byte.is_ascii_whitespace())
                .unwrap_or(true)
        {
            offsets.entry(object_id).or_insert(index);
        }
        index = after_id;
    }
    offsets
}

fn parse_decimal_at(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == start {
        return None;
    }
    let value = std::str::from_utf8(&bytes[start..end])
        .ok()
        .and_then(|value| value.parse::<usize>().ok())?;
    Some((value, end))
}

fn token_positions(bytes: &[u8], token: &[u8]) -> Vec<usize> {
    bytes
        .windows(token.len())
        .enumerate()
        .filter_map(|(index, window)| {
            if window != token {
                return None;
            }
            let before_ok = index == 0 || bytes[index - 1].is_ascii_whitespace();
            let after = index + token.len();
            let after_ok = after >= bytes.len() || bytes[after].is_ascii_whitespace();
            (before_ok && after_ok).then_some(index)
        })
        .collect()
}

fn coverage_ledger_from_pages(pages: &[Value]) -> Vec<Value> {
    pages
        .iter()
        .flat_map(|page| {
            page.get("_coverage")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .collect()
}

fn page_rotation(document: &Document, object_id: pdf_extract::ObjectId) -> u16 {
    let mut current = object_id;
    for _ in 0..32 {
        let Ok(dictionary) = document.get_dictionary(current) else {
            return 0;
        };
        if let Ok(value) = dictionary.get(b"Rotate") {
            let rotation = match value {
                pdf_extract::Object::Integer(value) => *value,
                pdf_extract::Object::Reference(reference) => document
                    .get_object(*reference)
                    .ok()
                    .and_then(|object| object.as_i64().ok())
                    .unwrap_or(0),
                _ => 0,
            };
            let normalized = rotation.rem_euclid(360);
            return match normalized {
                90 | 180 | 270 => normalized as u16,
                _ => 0,
            };
        }
        let Some(parent) = dictionary
            .get(b"Parent")
            .ok()
            .and_then(|value| value.as_reference().ok())
        else {
            return 0;
        };
        current = parent;
    }
    0
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

fn rect_value(x: f64, y: f64, width: f64, height: f64, origin: &str, page_rotation: u16) -> Value {
    json!({
        "x": clean_number(x),
        "y": clean_number(y),
        "width": clean_number(width.max(0.01)),
        "height": clean_number(height.max(0.01)),
        "unit": "pt",
        "origin": origin,
        "pageRotation": page_rotation
    })
}

fn finite_positive(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

fn clean_number(value: f64) -> f64 {
    if value.is_finite() {
        (value * 1_000_000.0).round() / 1_000_000.0
    } else {
        0.0
    }
}

fn number_at(value: &Value, pointer: &str) -> Option<f64> {
    value.get(pointer.trim_start_matches('/'))?.as_f64()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_store::make_job;
    use crate::{CreateJobInput, SourceFile};
    use std::env;
    use uuid::Uuid;

    fn fixture_source() -> (ImportJob, SourceFile) {
        let mut job = make_job(CreateJobInput {
            title: Some("PDF facts shadow fixture".to_string()),
            category: Some("P1".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["phase1-pr02".to_string()]),
            llm_profile_id: None,
        });
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures")
            .join("parser")
            .join("complex-reading.pdf");
        let bytes = fs::read(&path).expect("PDF facts fixture must exist");
        let source = SourceFile {
            file_id: "file-pr02".to_string(),
            original_name: "complex-reading.pdf".to_string(),
            stored_name: "complex-reading.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: crate::hash_bytes(&bytes),
            size_bytes: bytes.len() as u64,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        };
        job.source_files = vec![source.clone()];
        (job, source)
    }

    #[test]
    fn extracts_native_glyph_boxes_font_size_angle_and_zero_based_anchors() {
        let (job, source) = fixture_source();
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures")
            .join("parser")
            .join("complex-reading.pdf");
        let value = extract_pdf_facts_shadow(&job, &source, &path).unwrap();
        assert_eq!(
            value.get("schemaVersion").and_then(Value::as_str),
            Some("DocumentIRV2")
        );
        let page = &value["pages"][0];
        assert_eq!(page["pageIndex"].as_u64(), Some(0));
        assert!(page["glyphs"].as_array().unwrap().len() > 10);
        let glyph = &page["glyphs"][0];
        assert!(glyph["bbox"]["width"].as_f64().unwrap() > 0.0);
        assert!(glyph["style"]["fontSizePt"].as_f64().unwrap() > 0.0);
        assert!(glyph["angleRad"].as_f64().unwrap().is_finite());
        assert_eq!(glyph["sourceAnchor"]["pageIndex"].as_i64(), Some(0));
        assert_eq!(
            glyph["sourceAnchor"]["sourceFileId"].as_str(),
            Some("file-pr02")
        );
    }

    #[test]
    fn overlay_command_writes_a_debug_svg_without_requiring_parser_integration() {
        let root = env::temp_dir().join(format!("phase1-pr02-{}", Uuid::new_v4().simple()));
        crate::util::ensure_app_dirs(&root).unwrap();
        let (job, source) = fixture_source();
        crate::job_store::save_job(&root, &job).unwrap();
        crate::util::ensure_job_dirs(&crate::util::job_dir(&root, &job.job_id)).unwrap();
        let input = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures")
            .join("parser")
            .join("complex-reading.pdf");
        write_pdf_facts_shadow(
            &job,
            &source,
            &input,
            &crate::util::job_dir(&root, &job.job_id).join(SHADOW_ARTIFACT_FILE),
        )
        .unwrap();
        let result = debug_document_ir_v2_overlay_core(&root, &job.job_id).unwrap();
        let overlay_path = Path::new(result["overlayPath"].as_str().unwrap());
        let overlay = fs::read_to_string(overlay_path).unwrap();
        assert!(overlay.starts_with("<svg "));
        assert!(overlay.contains("class=\"glyph\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn repaired_classic_structure_has_canonical_xref_and_stream_length() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures")
            .join("parser")
            .join("complex-reading.pdf");
        let bytes = fs::read(path).unwrap();
        let repaired = repair_classic_pdf_structure(&bytes).unwrap();
        let xref_position = token_positions(&repaired, b"xref").last().copied().unwrap();
        assert!(String::from_utf8_lossy(&repaired[..xref_position]).contains("/Length 882"));
        assert_eq!(
            String::from_utf8_lossy(&repaired[1261..]),
            "xref\r\n0 6\r\n0000000000 65535 f\r\n0000000010 00000 n\r\n0000000062 00000 n\r\n0000000122 00000 n\r\n0000000251 00000 n\r\n0000000324 00000 n\r\ntrailer\r\n<< /Size 6 /Root 1 0 R >>\r\nstartxref\r\n1261\r\n%%EOF\r\n"
        );
    }
}
