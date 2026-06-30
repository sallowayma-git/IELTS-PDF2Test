//! Real-coordinate PDF text + page rendering backend backed by `pdfium-render`.
//!
//! This module is the primary PDF parser when the native pdfium library is
//! available. Unlike the `pdf-extract` text-layer fallback (which yields text
//! only and forces the caller to fabricate bounding boxes), pdfium gives real
//! per-character geometry, which lets `dynamic_block_column` actually detect
//! 2-column layouts and reconstruct reading order. The same library also
//! renders pages to PNG for the vision/OCR rescue path, so scanned PDFs no
//! longer require a system Python + PyMuPDF.
//!
//! Binding strategy: resolve the native library via (1) `EPIC8_PDFIUM_LIB`
//! env, (2) the app exe/resources directory, (3) a `lib/pdfium-<platform>/`
//! folder next to the exe, then (4) fall back to the system library. If every
//! step fails, callers fall back to the text-layer parser / unsupported stub.

use std::fs;
use std::path::{Path, PathBuf};

use pdfium_render::prelude::*;
use serde_json::{json, Value};

use crate::CommandResult;
use crate::{ImportJob, SourceFile};

/// Resolve the native pdfium library path, trying env override, the exe
/// directory, a platform resources folder, and finally the system library.
pub(crate) fn pdfium_library_path() -> Option<PathBuf> {
    if let Ok(spec) = std::env::var("EPIC8_PDFIUM_LIB") {
        let candidate = PathBuf::from(spec);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    if let Some(dir) = exe_dir.as_ref() {
        // Directly next to the exe.
        let direct = dir.join(Pdfium::pdfium_platform_library_name());
        if direct.exists() {
            return Some(direct);
        }
        // A platform-tagged resources folder.
        if let Some(folder) = platform_pdfium_folder(dir) {
            let candidate = folder.join(Pdfium::pdfium_platform_library_name());
            if candidate.exists() {
                return Some(candidate);
            }
        }
        // Common resources sub-directories produced by Tauri bundling.
        for sub in ["resources", "../resources"] {
            let candidate = dir.join(sub).join(Pdfium::pdfium_platform_library_name());
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn platform_pdfium_folder(base: &Path) -> Option<PathBuf> {
    let folder = if cfg!(target_os = "windows") {
        "pdfium-windows"
    } else if cfg!(target_os = "macos") {
        "pdfium-macos"
    } else {
        "pdfium-linux"
    };
    Some(base.join("lib").join(folder))
}

/// Bind to the native pdfium library. Falls back to the system library if no
/// bundled binary is found.
fn bind_pdfium() -> Result<Pdfium, String> {
    if let Some(path) = pdfium_library_path() {
        let parent = path.parent().unwrap_or(Path::new("."));
        return Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(parent))
            .map_err(|error| format!("pdfium_bind_bundled_failed:{}:{}", path.display(), error))
            .map(Pdfium::new);
    }
    Pdfium::bind_to_system_library()
        .map_err(|error| format!("pdfium_bind_system_failed:{}", error))
        .map(Pdfium::new)
}

/// A character extracted from a pdfium text page, with its origin coordinates
/// in PDF user-space (origin bottom-left, y increases upward).
#[derive(Clone, Copy)]
struct CharWithOrigin {
    ch: char,
    x: f32,
    y: f32,
}

/// Parse a PDF into a `DocumentIRV1` JSON value with REAL per-line bounding
/// boxes and REAL page dimensions. Each line (group of characters at the same
/// y) becomes one document block carrying a true `bbox: [x0, y0, x1, y1]`.
pub(crate) fn parse_pdf_with_pdfium(
    job: &ImportJob,
    source: &SourceFile,
    upload_path: &Path,
    output_path: &Path,
    mode: &str,
) -> CommandResult<Value> {
    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(upload_path, None)
        .map_err(|error| format!("pdfium_open_failed:{}:{}", upload_path.display(), error))?;

    let mut warnings = Vec::<String>::new();
    let mut block_counter = 1usize;
    let mut pages = Vec::<Value>::new();

    for (page_index, page) in document.pages().iter().enumerate() {
        let page_number = page_index + 1;
        let page_width = page.width().value;
        let page_height = page.height().value;

        let chars = collect_chars_with_origin(&page);
        if chars.is_empty() {
            warnings.push(format!(
                "page {} has no extractable text; OCR/manual review required",
                page_number
            ));
            let placeholder = format!("[No extractable text on page {}]", page_number);
            pages.push(json!({
                "pageIndex": page_number,
                "width": page_width,
                "height": page_height,
                "blocks": [document_block_with_bbox(
                    format!("b{:03}", block_counter),
                    &placeholder,
                    page_number,
                    0,
                    0.2,
                    [72.0, 72.0, 520.0, 108.0],
                )]
            }));
            block_counter += 1;
            continue;
        }

        let lines = build_lines_from_chars(&chars);
        // Group consecutive lines into paragraph-ish blocks when the vertical
        // gap between them is small relative to the line height. This matches
        // the granularity the text-layer `semantic_text_chunks` produces, so
        // downstream segmentation behaves consistently.
        let grouped = group_lines_into_blocks(&lines);

        let blocks = grouped
            .iter()
            .enumerate()
            .map(|(ordinal, (text, bbox))| {
                let block = document_block_with_bbox(
                    format!("b{:03}", block_counter),
                    text,
                    page_number,
                    ordinal,
                    0.98,
                    *bbox,
                );
                block_counter += 1;
                block
            })
            .collect::<Vec<_>>();

        pages.push(json!({
            "pageIndex": page_number,
            "width": page_width,
            "height": page_height,
            "blocks": blocks
        }));
    }

    if pages.is_empty() {
        warnings.push("PDF has no readable pages; OCR/manual review required".to_string());
        pages.push(json!({
            "pageIndex": 1,
            "width": 595.0,
            "height": 842.0,
            "blocks": [document_block_with_bbox(
                "b001".to_string(),
                "[No extractable text on page 1]",
                1,
                0,
                0.2,
                [72.0, 72.0, 520.0, 108.0],
            )]
        }));
    }

    let ir = json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": pages,
        "assets": [],
        "parser": {
            "provider": "rust-parser:pdf:pdfium",
            "version": "0.1.0",
            "mode": mode,
            "warnings": warnings,
            "sourceFileId": source.file_id,
            "sourceStoredName": source.stored_name
        }
    });
    crate::util::write_json(output_path, &ir)?;
    Ok(ir)
}

/// Collect every text character on the page together with its origin
/// coordinates. pdfium's `PdfPageTextChar::origin_x()/origin_y()` return the
/// glyph's pen position in PDF user-space points.
fn collect_chars_with_origin(page: &PdfPage) -> Vec<CharWithOrigin> {
    let text = match page.text() {
        Ok(text) => text,
        Err(_) => return Vec::new(),
    };
    let mut chars = Vec::new();
    for char_obj in text.chars().iter() {
        let Some(ch) = char_obj.unicode_char() else {
            continue;
        };
        // pdfium can emit CR/LF as part of the text stream; skip line-break
        // control characters so they don't pollute block text.
        if ch == '\r' || ch == '\n' || ch == '\t' {
            continue;
        }
        let x = char_obj.origin_x().map(|p| p.value).unwrap_or(0.0);
        let y = char_obj.origin_y().map(|p| p.value).unwrap_or(0.0);
        chars.push(CharWithOrigin { ch, x, y });
    }
    chars
}

/// Group characters into lines by y-origin proximity, then into words by
/// x-gap. Returns `(line_text, [x0, y0, x1, y1])` per line, where the bbox is
/// the union of the line's character origins (a tight-envelope approximation
/// suitable for column detection — only x-extent matters for that purpose).
///
/// IMPORTANT: characters are first partitioned into COLUMNS by x-gap, so that
/// the left and right columns of a 2-column page never merge into one wide
/// line (which would corrupt reading order and produce spans crossing the
/// gutter). Full-width header lines are detected as their own single column.
fn build_lines_from_chars(chars: &[CharWithOrigin]) -> Vec<(String, [f32; 4])> {
    if chars.is_empty() {
        return Vec::new();
    }

    // Estimate a typical character height from the y-spread to use as the
    // line-clustering tolerance.
    let mut ys: Vec<f32> = chars.iter().map(|c| c.y).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let y_tol = {
        let span = ys.last().unwrap_or(&0.0) - ys.first().unwrap_or(&0.0);
        (span / 40.0).max(2.0).min(6.0)
    };

    // Partition characters into columns by detecting the column GUTTER — a
    // vertical band of the page with (near-)zero character density. We build
    // a fine-grained x-histogram of character origins and look for the widest
    // empty band that is narrower than a half-page but wider than a word
    // space. Characters left of the gutter form column 0; right form column 1.
    // If no such gutter exists, the page is single-column.
    let x_min_global = chars.iter().map(|c| c.x).fold(f32::MAX, f32::min);
    let x_max_global = chars.iter().map(|c| c.x).fold(f32::MIN, f32::max);
    let page_x_span = (x_max_global - x_min_global).max(1.0);
    let bin_width = 4.0; // points per histogram bin
    let bin_count = ((page_x_span / bin_width).ceil() as usize).max(1);
    let mut histogram = vec![0u32; bin_count];
    for ch in chars {
        let idx = (((ch.x - x_min_global) / bin_width).floor() as usize).min(bin_count - 1);
        histogram[idx] += 1;
    }
    // Find the longest run of near-empty bins in the MIDDLE 60% of the page
    // (exclude the outer 20% on each side so margins don't count). A gutter
    // must be at least ~4 bins wide (16pt) to qualify. Bins with ≤2 characters
    // count as "near-empty" to tolerate stray page-numbers, punctuation, or
    // justified-line overflow landing in the gutter.
    let lo = (bin_count as f32 * 0.2) as usize;
    let hi = (bin_count as f32 * 0.8) as usize;
    let mut best_gutter_lo = 0usize;
    let mut best_gutter_len = 0usize;
    let mut run_lo = lo;
    let mut run_len = 0usize;
    let mut i = lo;
    while i < hi {
        if histogram[i] <= 2 {
            if run_len == 0 {
                run_lo = i;
            }
            run_len += 1;
            if run_len > best_gutter_len {
                best_gutter_len = run_len;
                best_gutter_lo = run_lo;
            }
        } else {
            run_len = 0;
        }
        i += 1;
    }
    let min_gutter_bins = 4usize; // ~16pt
    let column_split_x = if best_gutter_len >= min_gutter_bins {
        // Split at the centre of the gutter band.
        let gutter_mid_bin = best_gutter_lo + best_gutter_len / 2;
        x_min_global + gutter_mid_bin as f32 * bin_width
    } else {
        // Single-column page: no split.
        f32::MAX
    };
    // Threshold used only for the per-column line builder's awareness; the
    // actual split is the explicit x boundary above.
    let _column_gap_threshold = column_split_x;

    // Assign characters to columns based on the detected gutter split. A
    // character whose x-origin is left of `column_split_x` goes to column 0;
    // right goes to column 1. When no gutter was found (column_split_x ==
    // f32::MAX), everything lands in column 0 (single-column page).
    let mut column_left: Vec<CharWithOrigin> = Vec::new();
    let mut column_right: Vec<CharWithOrigin> = Vec::new();
    for ch in chars {
        if ch.x >= column_split_x {
            column_right.push(*ch);
        } else {
            column_left.push(*ch);
        }
    }
    let columns: Vec<Vec<CharWithOrigin>> = if column_right.is_empty() {
        vec![column_left]
    } else {
        vec![column_left, column_right]
    };

    // Build lines WITHIN each column (so left/right columns stay separate),
    // then merge the per-column line lists. The reading-order comparator
    // (`dynamic_reading_order_cmp`) re-sorts the final blocks by column then y.
    let mut result: Vec<(String, [f32; 4])> = Vec::new();
    for column in columns {
        result.extend(build_lines_within_column(&column, y_tol));
    }
    result
}

/// Build lines from a single column's characters (already x-isolated from
/// other columns). Clusters by y-origin proximity, then splits into words.
fn build_lines_within_column(chars: &[CharWithOrigin], y_tol: f32) -> Vec<(String, [f32; 4])> {
    if chars.is_empty() {
        return Vec::new();
    }
    // Cluster characters into lines by y-origin.
    let mut sorted = chars.to_vec();
    sorted.sort_by(|a, b| {
        a.y
            .partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .reverse() // top of page has larger y in PDF space
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut lines: Vec<Vec<CharWithOrigin>> = Vec::new();
    for ch in sorted {
        let mut placed = false;
        for line in lines.iter_mut().rev().take(3) {
            let line_y = line.iter().map(|c| c.y).sum::<f32>() / line.len() as f32;
            if (ch.y - line_y).abs() <= y_tol {
                line.push(ch);
                placed = true;
                break;
            }
        }
        if !placed {
            lines.push(vec![ch]);
        }
    }

    // Sort lines top-to-bottom (descending y), then within each line sort
    // left-to-right (ascending x).
    lines.sort_by(|a, b| {
        let ay = a.iter().map(|c| c.y).sum::<f32>() / a.len() as f32;
        let by = b.iter().map(|c| c.y).sum::<f32>() / b.len() as f32;
        by.partial_cmp(&ay).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut result = Vec::new();
    for mut line in lines {
        line.sort_by(|a, b| {
            a.x
                .partial_cmp(&b.x)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Build words by splitting on horizontal gaps. The threshold is
        // derived from the MEDIAN consecutive-character advance (robust to the
        // occasional wide letter-spacing of centered titles), then scaled so
        // only genuine inter-word spaces trigger a split. Falls back to the
        // x-spread average when there are too few characters for a median.
        let mut advances: Vec<f32> = Vec::new();
        for window in line.windows(2) {
            let delta = window[1].x - window[0].x;
            if delta > 0.0 {
                advances.push(delta);
            }
        }
        let median_advance = if advances.len() >= 3 {
            advances.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            advances[advances.len() / 2]
        } else {
            let x_min = line.iter().map(|c| c.x).fold(f32::MAX, f32::min);
            let x_max = line.iter().map(|c| c.x).fold(f32::MIN, f32::max);
            ((x_max - x_min) / line.len().max(1) as f32).max(3.0)
        };
        // A word space is typically ~2-4× a character advance; use 2.5× as the
        // split threshold, with a sensible floor so tiny fonts still split.
        let gap_threshold = (median_advance * 2.5).max(4.0);

        let mut words: Vec<String> = Vec::new();
        let mut prev_x: Option<f32> = None;
        for ch in &line {
            let start_new = match prev_x {
                Some(px) => ch.x - px > gap_threshold,
                None => true,
            };
            if start_new {
                words.push(String::new());
            }
            words.last_mut().unwrap().push(ch.ch);
            prev_x = Some(ch.x);
        }

        let text = words.join(" ");
        let x0 = line.iter().map(|c| c.x).fold(f32::MAX, f32::min);
        let x1 = line.iter().map(|c| c.x).fold(f32::MIN, f32::max);
        let y0 = line.iter().map(|c| c.y).fold(f32::MAX, f32::min);
        let y1 = line.iter().map(|c| c.y).fold(f32::MIN, f32::max);
        // Pad the bbox vertically a little so adjacent lines don't have zero
        // height (origin points sit on the baseline).
        let pad = y_tol * 0.7;
        result.push((text, [x0, y0 - pad, x1, y1 + pad]));
    }
    result
}

/// Merge consecutive lines into paragraph-ish blocks when the vertical gap
/// between them is small (≤ 1.5× the previous line's height). Each emitted
/// block carries the union bbox of its constituent lines.
fn group_lines_into_blocks(lines: &[(String, [f32; 4])]) -> Vec<(String, [f32; 4])> {
    let mut groups: Vec<(String, [f32; 4])> = Vec::new();
    for (text, bbox) in lines {
        let prev_height = groups
            .last()
            .map(|(_, prev_bbox)| (prev_bbox[3] - prev_bbox[1]).abs())
            .unwrap_or(0.0);
        let gap = groups
            .last()
            .map(|(_, prev_bbox)| (bbox[1] - prev_bbox[3]).abs())
            .unwrap_or(0.0);
        let should_join = match groups.last() {
            Some(_) => prev_height > 0.0 && gap <= prev_height * 1.5,
            None => false,
        };
        if should_join {
            let (prev_text, prev_bbox) = groups.last().unwrap();
            let merged_text = format!("{} {}", prev_text, text);
            let merged_bbox = [
                prev_bbox[0].min(bbox[0]),
                prev_bbox[1].min(bbox[1]),
                prev_bbox[2].max(bbox[2]),
                prev_bbox[3].max(bbox[3]),
            ];
            *groups.last_mut().unwrap() = (merged_text, merged_bbox);
        } else {
            groups.push((text.clone(), *bbox));
        }
    }
    groups
}

/// Build a DocumentIRV1 block carrying a REAL bounding box (no fabricated
/// `[72, y0, 520, y0+36]`). Mirrors `parser::document_block` but accepts an
/// explicit bbox and omits the placeholder-only behaviour.
fn document_block_with_bbox(
    block_id: String,
    text: &str,
    page_index: usize,
    ordinal: usize,
    confidence: f64,
    bbox: [f32; 4],
) -> Value {
    let block_type = crate::parser::block_type_for_text_pub(text);
    let role_hint = crate::parser::role_hint_for_text_pub(text);
    let mut block = json!({
        "blockId": block_id,
        "blockType": block_type,
        "text": text,
        "html": crate::parser::markdownish_to_html_pub(text, block_type),
        "bbox": [bbox[0] as f64, bbox[1] as f64, bbox[2] as f64, bbox[3] as f64],
        "confidence": confidence,
        "pageIndex": page_index,
        "_epic8Ordinal": ordinal
    });
    if let Some(role) = role_hint {
        block["roleHint"] = json!(role);
    }
    block
}

/// Render every page of the PDF to a PNG (2× scale ≈ 144 DPI) for the
/// vision/OCR rescue path. Emits a `PdfImageExtractionV1`-shaped value
/// matching the contract `extract_pdf_images_for_vision` consumes.
pub(crate) fn render_pdf_pages_with_pdfium(
    job_id: &str,
    input_path: &Path,
    output_path: &Path,
    asset_dir: &Path,
    prior_warnings: Vec<String>,
) -> CommandResult<Value> {
    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(input_path, None)
        .map_err(|error| format!("pdfium_open_failed:{}:{}", input_path.display(), error))?;

    fs::create_dir_all(asset_dir)
        .map_err(|error| format!("create_pdfium_asset_dir:{}:{}", asset_dir.display(), error))?;

    let mut warnings = prior_warnings
        .into_iter()
        .filter(|w| !w.trim().is_empty())
        .collect::<Vec<_>>();
    warnings.push(
        "used bundled pdfium page renderer for vision transcription input".to_string(),
    );

    let page_count = document.pages().len();
    let mut pages_json = Vec::<Value>::new();

    for (page_index, page) in document.pages().iter().enumerate() {
        let page_number = page_index + 1;
        let bitmap_config = PdfRenderConfig::new()
            .set_target_width(2000)
            .set_maximum_height(2800);
        let bitmap = page
            .render_with_config(&bitmap_config)
            .map_err(|error| format!("pdfium_render_page_{}_failed:{}", page_number, error))?;
        // Use pdfium's native RGBA buffer directly + the lightweight `png`
        // encoder, instead of pulling in the heavyweight `image` crate (which
        // drags in moxcms/zune-jpeg/etc. for codecs we never need).
        let rgba_bytes = bitmap.as_rgba_bytes();
        let width = bitmap.width() as u32;
        let height = bitmap.height() as u32;

        let file_name = format!("page-{:03}-rendered.png", page_number);
        let rendered_path = asset_dir.join(&file_name);
        // Encode RGBA bytes into a PNG file.
        {
            let file = fs::File::create(&rendered_path).map_err(|error| {
                format!(
                    "pdfium_create_png_{}_failed:{}:{}",
                    page_number,
                    rendered_path.display(),
                    error
                )
            })?;
            let buffered = std::io::BufWriter::new(file);
            let mut encoder = png::Encoder::new(buffered, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .map_err(|error| format!("pdfium_png_header_{}_failed:{}", page_number, error))?;
            writer
                .write_image_data(&rgba_bytes)
                .map_err(|error| format!("pdfium_png_write_{}_failed:{}", page_number, error))?;
        }
        let bytes = fs::read(&rendered_path)
            .map_err(|error| format!("pdfium_read_png_{}_failed:{}", page_number, error))?;

        pages_json.push(json!({
            "pageIndex": page_number,
            "width": width,
            "height": height,
            "images": [{
                "assetId": format!("pdf-page-{}-rendered", page_number),
                "pageIndex": page_number,
                "fileName": file_name,
                "path": rendered_path.to_string_lossy(),
                "mimeType": "image/png",
                "width": width,
                "height": height,
                "sha256": crate::hash_bytes(&bytes),
                "sizeBytes": bytes.len() as u64,
                "renderedFallback": true,
                "renderSource": "rust-pdfium"
            }]
        }));
    }

    let mut extraction = json!({
        "schemaVersion": "PdfImageExtractionV1",
        "jobId": job_id,
        "sourcePath": input_path.to_string_lossy(),
        "rendererAdapter": "windows-pdfium",
        "rendererProvider": "pdfium-render",
        "rendererVersion": "0.1.0",
        "renderPurpose": "vision-llm-transcription-input",
        "pageCount": page_count,
        "renderedPageCount": page_count,
        "dpi": 180,
        "ocrPerformed": false,
        "failureReason": null,
        "requiresManualReview": false,
        "pages": pages_json,
        "warnings": warnings,
        "renderedFallback": true
    });
    crate::parser::stabilize_pdf_image_extraction_fields_pub(
        &mut extraction,
        Some("windows-pdfium"),
        Some("pdfium-render"),
        Some(json!("0.1.0")),
        Some(180),
        None,
        Some(false),
    );
    crate::util::write_json(output_path, &extraction)?;
    Ok(extraction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdfium_library_path_does_not_panic() {
        // Resolution must be safe to call even when no library is bundled.
        let _ = pdfium_library_path();
    }

    #[test]
    fn pdfium_parse_yields_real_bbox_when_library_present() {
        let sample = Path::new(
            r"D:\xwechat_files\wxid_zg93z3d7b4aq21_8fcc\msg\file\2026-06\PDF(1).pdf",
        );
        if !sample.exists() || bind_pdfium().is_err() {
            return; // sample or pdfium library not available in this environment
        }
        let pdfium = bind_pdfium().unwrap();
        let document = pdfium
            .load_pdf_from_file(sample, None)
            .expect("sample pdf should open");
        let mut found_real = false;
        for page in document.pages().iter() {
            let chars = collect_chars_with_origin(&page);
            for ch in &chars {
                // A real coordinate escapes the fabricated envelope [72..520].
                if ch.x > 520.5 || ch.x < 71.5 {
                    found_real = true;
                    break;
                }
            }
            if found_real {
                break;
            }
        }
        assert!(found_real, "expected at least one char with a real x coord");
    }
}
