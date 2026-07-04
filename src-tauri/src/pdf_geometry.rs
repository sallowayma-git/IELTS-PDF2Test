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

#[derive(Clone, Copy, Debug)]
struct LayoutSection {
    index: usize,
    y_top: f32,
    y_bottom: f32,
    column_count: u8,
    split_x: Option<f32>,
}

#[derive(Clone, Debug)]
struct BlockWithLayout {
    text: String,
    bbox: [f32; 4],
    section_index: usize,
    column_index: u8,
    column_count: u8,
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

        let grouped = build_blocks_from_chars(&chars);

        let blocks = grouped
            .iter()
            .enumerate()
            .map(|(ordinal, layout_block)| {
                let block = document_block_with_layout(
                    format!("b{:03}", block_counter),
                    &layout_block.text,
                    page_number,
                    ordinal,
                    0.98,
                    layout_block.bbox,
                    layout_block.section_index,
                    layout_block.column_index,
                    layout_block.column_count,
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

fn estimate_y_tolerance(chars: &[CharWithOrigin]) -> f32 {
    if chars.is_empty() {
        return 3.0;
    }
    let mut ys: Vec<f32> = chars.iter().map(|c| c.y).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let span = ys.last().unwrap_or(&0.0) - ys.first().unwrap_or(&0.0);
    (span / 40.0).max(2.0).min(6.0)
}

fn detect_column_split_x(
    chars: &[CharWithOrigin],
    x_min_global: f32,
    x_max_global: f32,
) -> Option<f32> {
    if chars.len() < 24 {
        return None;
    }
    let page_x_span = (x_max_global - x_min_global).max(1.0);
    let bin_width = 4.0;
    let bin_count = ((page_x_span / bin_width).ceil() as usize).max(1);
    let mut histogram = vec![0u32; bin_count];
    for ch in chars {
        let idx = (((ch.x - x_min_global) / bin_width).floor() as usize).min(bin_count - 1);
        histogram[idx] += 1;
    }
    let lo = (bin_count as f32 * 0.2) as usize;
    let hi = (bin_count as f32 * 0.8) as usize;
    if lo >= hi {
        return None;
    }
    let mut best_gutter_lo = 0usize;
    let mut best_gutter_len = 0usize;
    let mut run_lo = lo;
    let mut run_len = 0usize;
    for index in lo..hi {
        if histogram[index] <= 2 {
            if run_len == 0 {
                run_lo = index;
            }
            run_len += 1;
            if run_len > best_gutter_len {
                best_gutter_len = run_len;
                best_gutter_lo = run_lo;
            }
        } else {
            run_len = 0;
        }
    }
    let min_gutter_width = (page_x_span * 0.10).max(28.0);
    let min_gutter_bins = ((min_gutter_width / bin_width).ceil() as usize).max(4);
    if best_gutter_len < min_gutter_bins {
        return None;
    }
    let gutter_mid_bin = best_gutter_lo + best_gutter_len / 2;
    let split_x = x_min_global + gutter_mid_bin as f32 * bin_width;
    let left_count = chars.iter().filter(|ch| ch.x < split_x).count();
    let right_count = chars.len().saturating_sub(left_count);
    let min_side_chars = ((chars.len() as f32 * 0.18).ceil() as usize).max(6);
    if left_count < min_side_chars || right_count < min_side_chars {
        return None;
    }
    let y_tol = estimate_y_tolerance(chars);
    let mut sorted = chars.to_vec();
    sorted.sort_by(|a, b| {
        b.y.partial_cmp(&a.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut rows: Vec<(f32, usize, bool, bool)> = Vec::new();
    for ch in sorted {
        let mut placed = false;
        for row in rows.iter_mut().rev().take(4) {
            if (ch.y - row.0).abs() <= y_tol {
                row.0 = (row.0 * row.1 as f32 + ch.y) / (row.1 as f32 + 1.0);
                row.1 += 1;
                if ch.x < split_x {
                    row.2 = true;
                } else {
                    row.3 = true;
                }
                placed = true;
                break;
            }
        }
        if !placed {
            rows.push((ch.y, 1, ch.x < split_x, ch.x >= split_x));
        }
    }
    let rows_with_both_sides = rows
        .iter()
        .filter(|(_, _, has_left, has_right)| *has_left && *has_right)
        .count();
    if rows.len() >= 2 && rows_with_both_sides * 3 >= rows.len() * 2 {
        return None;
    }
    Some(split_x)
}

fn detect_layout_sections(chars: &[CharWithOrigin]) -> Vec<LayoutSection> {
    if chars.is_empty() {
        return Vec::new();
    }
    #[derive(Clone, Copy)]
    struct BandHint {
        y_top: f32,
        y_bottom: f32,
        split_x: Option<f32>,
        char_count: usize,
    }

    let x_min_global = chars.iter().map(|c| c.x).fold(f32::MAX, f32::min);
    let x_max_global = chars.iter().map(|c| c.x).fold(f32::MIN, f32::max);
    let y_min = chars.iter().map(|c| c.y).fold(f32::MAX, f32::min);
    let y_max = chars.iter().map(|c| c.y).fold(f32::MIN, f32::max);
    let y_span = (y_max - y_min).max(1.0);
    let page_x_span = (x_max_global - x_min_global).max(1.0);
    let band_height = (y_span / 10.0).clamp(56.0, 88.0);
    let band_count = ((y_span / band_height).ceil() as usize).max(1);
    let mut bands = Vec::with_capacity(band_count);

    for band_index in 0..band_count {
        let y_top = y_max - band_index as f32 * band_height;
        let y_bottom = if band_index + 1 == band_count {
            y_min - 0.5
        } else {
            y_top - band_height
        };
        let band_chars = chars
            .iter()
            .copied()
            .filter(|ch| ch.y <= y_top && ch.y > y_bottom)
            .collect::<Vec<_>>();
        bands.push(BandHint {
            y_top,
            y_bottom,
            split_x: detect_column_split_x(&band_chars, x_min_global, x_max_global),
            char_count: band_chars.len(),
        });
    }

    if bands.len() >= 3 {
        for index in 1..bands.len() - 1 {
            if bands[index].split_x.is_some() || bands[index].char_count > 24 {
                continue;
            }
            let Some(prev_split) = bands[index - 1].split_x else {
                continue;
            };
            let Some(next_split) = bands[index + 1].split_x else {
                continue;
            };
            if (prev_split - next_split).abs() <= page_x_span * 0.08 {
                bands[index].split_x = Some((prev_split + next_split) * 0.5);
            }
        }
    }

    let mut sections = Vec::new();
    for band in bands.into_iter().filter(|band| band.char_count > 0) {
        let band_column_count = if band.split_x.is_some() { 2 } else { 1 };
        let can_merge = sections.last().is_some_and(|current: &LayoutSection| {
            if current.column_count != band_column_count {
                return false;
            }
            match (current.split_x, band.split_x) {
                (Some(left), Some(right)) => (left - right).abs() <= page_x_span * 0.08,
                (None, None) => true,
                _ => false,
            }
        });
        if can_merge {
            if let Some(current) = sections.last_mut() {
                current.y_bottom = band.y_bottom;
                if let (Some(left), Some(right)) = (current.split_x, band.split_x) {
                    current.split_x = Some((left + right) * 0.5);
                }
            }
            continue;
        }
        sections.push(LayoutSection {
            index: sections.len(),
            y_top: band.y_top,
            y_bottom: band.y_bottom,
            column_count: band_column_count,
            split_x: band.split_x,
        });
    }

    if sections.is_empty() {
        sections.push(LayoutSection {
            index: 0,
            y_top: y_max,
            y_bottom: y_min - 0.5,
            column_count: 1,
            split_x: None,
        });
    }
    sections
}

/// Build paragraph-ish blocks from page characters while honoring per-section
/// layout changes such as "top half 2-column, bottom half single-column".
fn build_blocks_from_chars(chars: &[CharWithOrigin]) -> Vec<BlockWithLayout> {
    if chars.is_empty() {
        return Vec::new();
    }
    let y_tol = estimate_y_tolerance(chars);
    let sections = detect_layout_sections(chars);
    let mut result = Vec::new();
    for section in sections {
        let section_chars = chars
            .iter()
            .copied()
            .filter(|ch| ch.y <= section.y_top && ch.y >= section.y_bottom)
            .collect::<Vec<_>>();
        if section_chars.is_empty() {
            continue;
        }
        if section.column_count == 1 {
            let lines = build_lines_within_column(&section_chars, y_tol);
            let grouped = group_lines_into_blocks(&lines);
            result.extend(grouped.into_iter().map(|(text, bbox)| BlockWithLayout {
                text,
                bbox,
                section_index: section.index,
                column_index: 0,
                column_count: 1,
            }));
            continue;
        }
        let split_x = section.split_x.unwrap_or(f32::MAX);
        let mut column_left = Vec::new();
        let mut column_right = Vec::new();
        for ch in section_chars {
            if ch.x >= split_x {
                column_right.push(ch);
            } else {
                column_left.push(ch);
            }
        }
        for (column_index, column_chars) in [(0u8, column_left), (1u8, column_right)] {
            if column_chars.is_empty() {
                continue;
            }
            let lines = build_lines_within_column(&column_chars, y_tol);
            let grouped = group_lines_into_blocks(&lines);
            result.extend(grouped.into_iter().map(|(text, bbox)| BlockWithLayout {
                text,
                bbox,
                section_index: section.index,
                column_index,
                column_count: 2,
            }));
        }
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
        a.y.partial_cmp(&b.y)
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
        line.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));

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
fn looks_like_hard_line_break(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_lowercase();
    lower.starts_with("reading passage")
        || lower.starts_with("questions ")
        || lower.starts_with("question ")
        || lower.starts_with("answers")
        || lower.contains("answer key")
}

fn group_lines_into_blocks(lines: &[(String, [f32; 4])]) -> Vec<(String, [f32; 4])> {
    let mut groups: Vec<(String, [f32; 4])> = Vec::new();
    for (text, bbox) in lines {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let should_join = match groups.last() {
            Some((prev_text, prev_bbox)) => {
                let prev_height = (prev_bbox[3] - prev_bbox[1]).abs().max(1.0);
                let current_height = (bbox[3] - bbox[1]).abs().max(1.0);
                let gap = (prev_bbox[1] - bbox[3]).max(0.0);
                let left_delta = (bbox[0] - prev_bbox[0]).abs();
                let center_delta =
                    (((bbox[0] + bbox[2]) * 0.5) - ((prev_bbox[0] + prev_bbox[2]) * 0.5)).abs();
                let width_delta = ((bbox[2] - bbox[0]) - (prev_bbox[2] - prev_bbox[0])).abs();
                gap <= prev_height.max(current_height) * 0.9
                    && left_delta <= 28.0
                    && center_delta <= 40.0
                    && width_delta <= 80.0
                    && !looks_like_hard_line_break(trimmed)
                    && !prev_text.trim_end().ends_with(':')
            }
            None => false,
        };
        if should_join {
            let (prev_text, prev_bbox) = groups.last().unwrap();
            let merged_text = format!("{} {}", prev_text, trimmed);
            let merged_bbox = [
                prev_bbox[0].min(bbox[0]),
                prev_bbox[1].min(bbox[1]),
                prev_bbox[2].max(bbox[2]),
                prev_bbox[3].max(bbox[3]),
            ];
            *groups.last_mut().unwrap() = (merged_text, merged_bbox);
        } else {
            groups.push((trimmed.to_string(), *bbox));
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

fn document_block_with_layout(
    block_id: String,
    text: &str,
    page_index: usize,
    ordinal: usize,
    confidence: f64,
    bbox: [f32; 4],
    section_index: usize,
    column_index: u8,
    column_count: u8,
) -> Value {
    let mut block = document_block_with_bbox(block_id, text, page_index, ordinal, confidence, bbox);
    if let Some(obj) = block.as_object_mut() {
        obj.insert("_epic8LayoutSection".to_string(), json!(section_index));
        obj.insert("_epic8ColumnIndex".to_string(), json!(column_index));
        obj.insert("_epic8SectionColumns".to_string(), json!(column_count));
        let layout_hints = obj
            .entry("layoutHints".to_string())
            .or_insert_with(|| json!({}));
        if let Some(layout_obj) = layout_hints.as_object_mut() {
            layout_obj.insert(
                "section".to_string(),
                json!({
                    "index": section_index,
                    "columns": {
                        "count": column_count,
                        "current": column_index
                    }
                }),
            );
        }
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
    warnings.push("used bundled pdfium page renderer for vision transcription input".to_string());

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

    fn push_text_line(chars: &mut Vec<CharWithOrigin>, text: &str, x_start: f32, y: f32) {
        let mut x = x_start;
        for ch in text.chars() {
            if ch == ' ' {
                x += 8.0;
                continue;
            }
            chars.push(CharWithOrigin { ch, x, y });
            x += 4.8;
        }
    }

    #[test]
    fn pdfium_library_path_does_not_panic() {
        // Resolution must be safe to call even when no library is bundled.
        let _ = pdfium_library_path();
    }

    #[test]
    fn pdfium_parse_yields_real_bbox_when_library_present() {
        let sample =
            Path::new(r"D:\xwechat_files\wxid_zg93z3d7b4aq21_8fcc\msg\file\2026-06\PDF(1).pdf");
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

    #[test]
    fn build_blocks_from_chars_preserves_mixed_column_sections() {
        let mut chars = Vec::new();
        push_text_line(
            &mut chars,
            "LEFT PASSAGE TEXT WITH CLEAR COLUMN SHAPE",
            78.0,
            760.0,
        );
        push_text_line(
            &mut chars,
            "LEFT PASSAGE LINE THAT CONTINUES NATURALLY",
            78.0,
            751.0,
        );
        push_text_line(
            &mut chars,
            "RIGHT PASSAGE TEXT WITH CLEAR COLUMN SHAPE",
            336.0,
            742.0,
        );
        push_text_line(
            &mut chars,
            "RIGHT PASSAGE LINE THAT CONTINUES NATURALLY",
            336.0,
            733.0,
        );
        push_text_line(
            &mut chars,
            "A LATER FULL WIDTH PARAGRAPH CONTINUES DOWN THE PAGE WITH DIFFERENT WORD SPACING",
            82.0,
            500.0,
        );
        push_text_line(
            &mut chars,
            "Readers then see one uninterrupted block instead of a forced two column split",
            82.0,
            491.0,
        );

        let blocks = build_blocks_from_chars(&chars);

        assert!(
            blocks
                .iter()
                .any(|block| block.column_count == 2 && block.column_index == 0),
            "left column block should be detected"
        );
        assert!(
            blocks
                .iter()
                .any(|block| block.column_count == 2 && block.column_index == 1),
            "right column block should be detected"
        );

        let single_column_blocks = blocks
            .iter()
            .filter(|block| block.column_count == 1)
            .collect::<Vec<_>>();
        assert_eq!(
            single_column_blocks.len(),
            1,
            "bottom single-column section should remain a single block"
        );
        assert!(
            single_column_blocks[0]
                .text
                .contains("FULL WIDTH PARAGRAPH CONTINUES"),
            "single-column tail should survive as original passage text"
        );
        let max_two_column_section = blocks
            .iter()
            .filter(|block| block.column_count == 2)
            .map(|block| block.section_index)
            .max()
            .unwrap_or(0);
        assert!(
            single_column_blocks[0].section_index > max_two_column_section,
            "single-column tail should be emitted after the earlier two-column section"
        );
    }
}
