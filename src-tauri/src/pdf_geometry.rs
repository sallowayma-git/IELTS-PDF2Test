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

#[derive(Clone, Debug)]
struct LayoutSection {
    y_top: f32,
    y_bottom: f32,
    column_count: u8,
    /// Column separators (x coords). For 2 columns: one gutter; for 3
    /// columns: two gutters. Empty for single-column sections. The first
    /// element doubles as the legacy `split_x` used by banner heuristics.
    gutters: Vec<f32>,
}

impl LayoutSection {
    /// Legacy single split_x (first gutter), for backward-compatible banner
    /// heuristics. Returns `f32::MAX` when there are no gutters.
    fn split_x(&self) -> f32 {
        self.gutters.first().copied().unwrap_or(f32::MAX)
    }
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
    // Cross-page section continuation state: when the previous page ended mid
    // multi-column passage text and the next page opens with the SAME column
    // structure, we keep counting the global section index so the reading-order
    // comparator treats the two pages as one continuous multi-column flow.
    // `prev_page_last_section` holds (column_count, gutters, global_section_no)
    // of the previous page's last section.
    let mut prev_page_last_section: Option<(u8, Vec<f32>, usize)> = None;

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
            prev_page_last_section = None;
            continue;
        }

        let grouped = build_blocks_from_chars(&chars);

        // Determine this page's section structure so we can decide whether the
        // opening section continues the previous page's flow.
        let page_sections = detect_layout_sections(&chars);
        let (section_offset, continues_previous) = cross_page_section_continuation(
            &page_sections,
            &grouped,
            prev_page_last_section.as_ref(),
        );

        let blocks = grouped
            .iter()
            .enumerate()
            .map(|(ordinal, layout_block)| {
                let adjusted_section = layout_block.section_index + section_offset;
                let global_section = if continues_previous {
                    Some(adjusted_section)
                } else {
                    None
                };
                let block = document_block_with_layout(
                    format!("b{:03}", block_counter),
                    &layout_block.text,
                    page_number,
                    ordinal,
                    0.98,
                    layout_block.bbox,
                    adjusted_section,
                    layout_block.column_index,
                    layout_block.column_count,
                    global_section,
                );
                block_counter += 1;
                block
            })
            .collect::<Vec<_>>();

        // Update the cross-page continuation state for the next page: record
        // this page's last section (column structure + its global section
        // number). When this page continued the previous one, the last section
        // keeps the shared global counter; otherwise it starts a new range at
        // `section_offset + page_section_count - 1`.
        if let Some(last_section) = page_sections.last() {
            let last_global = section_offset + page_sections.len().saturating_sub(1);
            prev_page_last_section = Some((
                last_section.column_count,
                last_section.gutters.clone(),
                last_global,
            ));
        }

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

fn build_raw_rows(chars: &[CharWithOrigin], y_tol: f32) -> Vec<Vec<CharWithOrigin>> {
    if chars.is_empty() {
        return Vec::new();
    }
    let mut sorted = chars.to_vec();
    sorted.sort_by(|a, b| {
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .reverse()
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut rows: Vec<Vec<CharWithOrigin>> = Vec::new();
    for ch in sorted {
        let mut placed = false;
        for row in rows.iter_mut().rev().take(3) {
            let row_y = row.iter().map(|c| c.y).sum::<f32>() / row.len() as f32;
            if (ch.y - row_y).abs() <= y_tol {
                row.push(ch);
                placed = true;
                break;
            }
        }
        if !placed {
            rows.push(vec![ch]);
        }
    }

    rows.sort_by(|a, b| {
        let ay = a.iter().map(|c| c.y).sum::<f32>() / a.len() as f32;
        let by = b.iter().map(|c| c.y).sum::<f32>() / b.len() as f32;
        by.partial_cmp(&ay).unwrap_or(std::cmp::Ordering::Equal)
    });
    for row in &mut rows {
        row.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    }
    rows
}

fn line_word_gap_threshold(line: &[CharWithOrigin]) -> f32 {
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
    (median_advance * 2.5).max(4.0)
}

fn line_bbox(line: &[CharWithOrigin], y_tol: f32) -> [f32; 4] {
    let x0 = line.iter().map(|c| c.x).fold(f32::MAX, f32::min);
    let x1 = line.iter().map(|c| c.x).fold(f32::MIN, f32::max);
    let y0 = line.iter().map(|c| c.y).fold(f32::MAX, f32::min);
    let y1 = line.iter().map(|c| c.y).fold(f32::MIN, f32::max);
    let pad = y_tol * 0.7;
    [x0, y0 - pad, x1, y1 + pad]
}

fn line_split_gap(line: &[CharWithOrigin], split_x: f32) -> Option<f32> {
    let left = line.iter().rev().find(|ch| ch.x < split_x)?;
    let right = line.iter().find(|ch| ch.x >= split_x)?;
    Some((right.x - left.x).max(0.0))
}

fn is_full_width_banner_row(
    line: &[CharWithOrigin],
    split_x: f32,
    x_min_global: f32,
    x_max_global: f32,
) -> bool {
    if line.len() < 4 {
        return false;
    }
    let page_x_span = (x_max_global - x_min_global).max(1.0);
    let left_count = line.iter().filter(|ch| ch.x < split_x).count();
    let right_count = line.len().saturating_sub(left_count);
    if left_count > 0 && right_count > 0 {
        let split_gap = line_split_gap(line, split_x).unwrap_or(page_x_span);
        let max_continuous_gap = line_word_gap_threshold(line).max(page_x_span * 0.035);
        return split_gap <= max_continuous_gap.max(14.0);
    }

    let bbox = line_bbox(line, estimate_y_tolerance(line));
    let width = bbox[2] - bbox[0];
    let center = (bbox[0] + bbox[2]) * 0.5;
    let center_diff = (center - split_x).abs();
    center_diff <= page_x_span * 0.07
        && width >= page_x_span * 0.10
        && width <= page_x_span * 0.42
        && bbox[0] >= split_x - page_x_span * 0.24
        && bbox[2] <= split_x + page_x_span * 0.24
}

fn band_looks_like_full_width_banner(
    chars: &[CharWithOrigin],
    split_x: f32,
    x_min_global: f32,
    x_max_global: f32,
) -> bool {
    let rows = build_raw_rows(chars, estimate_y_tolerance(chars));
    if rows.is_empty() {
        return false;
    }
    // Previously this hard-capped at <= 3 rows, which made multi-line centered
    // banners (e.g. 4-6 line figure captions or section titles spanning a
    // gutter) fail. Now judge by the banner-row RATIO: a band is a banner band
    // when at least half of its rows look full-width. A guard against a very
    // long band that just happens to be all prose is the row-level check
    // inside `is_full_width_banner_row` (centered + narrow width).
    let banner_rows = rows
        .iter()
        .filter(|row| is_full_width_banner_row(row, split_x, x_min_global, x_max_global))
        .count();
    banner_rows * 2 >= rows.len()
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
    // Reject the candidate gutter when too many rows straddle split_x with
    // only a SMALL gap at that x — that pattern means split_x is an
    // intra-line word gap, not a column separator. But rows that straddle
    // split_x with a LARGE gap (a real column gutter crossing one line, which
    // happens in 3-column layouts where each row naturally spans the first
    // gutter) are fine and must NOT cause rejection. This generalises the
    // old "rows_with_both_sides * 3 >= rows.len() * 2" heuristic, which
    // incorrectly rejected 3-column layouts.
    let mut straddle_with_small_gap = 0usize;
    let small_gap_threshold = page_x_span * 0.035;
    for row in &rows {
        let has_left = row.2;
        let has_right = row.3;
        if !(has_left && has_right) {
            continue;
        }
        // Reconstruct this row's chars (sorted by x) to measure the gap at
        // split_x. line_split_gap assumes an x-sorted line.
        let mut row_chars: Vec<CharWithOrigin> = chars
            .iter()
            .copied()
            .filter(|ch| (ch.y - row.0).abs() <= y_tol)
            .collect();
        row_chars.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        let gap_at_split = line_split_gap(&row_chars, split_x).unwrap_or(page_x_span);
        if gap_at_split <= small_gap_threshold.max(14.0) {
            straddle_with_small_gap += 1;
        }
    }
    if rows.len() >= 2 && straddle_with_small_gap * 3 >= rows.len() * 2 {
        return None;
    }
    Some(split_x)
}

/// Detect ALL column gutters on a character set by recursively finding the
/// deepest histogram valley, then re-running the search on the left and right
/// sub-ranges. Returns a sorted list of gutter x-coordinates. Empty = single
/// column; length 1 = two columns; length 2 = three columns.
///
/// The recursion stops when a sub-range has too few characters or the found
/// gutter falls outside the central 20%-80% band of that sub-range (i.e. the
/// sub-range is a single column).
fn detect_column_gutters(chars: &[CharWithOrigin], x_min: f32, x_max: f32) -> Vec<f32> {
    // A single text row does not contain enough vertical evidence to
    // distinguish real column gutters from ordinary word spacing. Let the
    // surrounding layout section provide the column hint for sparse edge
    // bands instead of manufacturing gutters from one line.
    if build_raw_rows(chars, estimate_y_tolerance(chars)).len() < 2 {
        return Vec::new();
    }
    let mut gutters = Vec::new();
    detect_column_gutters_recursive(chars, x_min, x_max, &mut gutters, 0);
    gutters.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    gutters.dedup_by(|a, b| (*a - *b).abs() < 4.0);
    gutters
}

fn detect_column_gutters_recursive(
    chars: &[CharWithOrigin],
    x_min: f32,
    x_max: f32,
    gutters: &mut Vec<f32>,
    depth: u8,
) {
    // Cap recursion: at most 2 gutters => 3 columns.
    if depth >= 2 || gutters.len() >= 2 || chars.len() < 24 {
        return;
    }
    let Some(split_x) = detect_column_split_x(chars, x_min, x_max) else {
        return;
    };
    gutters.push(split_x);
    // Recurse into left and right sub-ranges relative to this gutter.
    let left: Vec<CharWithOrigin> = chars.iter().copied().filter(|ch| ch.x < split_x).collect();
    let right: Vec<CharWithOrigin> = chars.iter().copied().filter(|ch| ch.x >= split_x).collect();
    if !left.is_empty() && gutters.len() < 2 {
        detect_column_gutters_recursive(&left, x_min, split_x, gutters, depth + 1);
    }
    if !right.is_empty() && gutters.len() < 2 {
        detect_column_gutters_recursive(&right, split_x, x_max, gutters, depth + 1);
    }
}

fn detect_layout_sections(chars: &[CharWithOrigin]) -> Vec<LayoutSection> {
    if chars.is_empty() {
        return Vec::new();
    }
    #[derive(Clone)]
    struct BandHint {
        y_top: f32,
        y_bottom: f32,
        gutters: Vec<f32>,
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
            gutters: detect_column_gutters(&band_chars, x_min_global, x_max_global),
            char_count: band_chars.len(),
        });
    }

    // Fill thin empty bands between two bands that agree on their gutters, so a
    // brief density valley doesn't split a continuous multi-column region in
    // half. Only fills when the surrounding gutters match (same column count
    // and close gutter x), and the band itself doesn't look like a full-width
    // banner crossing the gutter.
    if bands.len() >= 3 {
        for index in 1..bands.len() - 1 {
            if !bands[index].gutters.is_empty() || bands[index].char_count > 24 {
                continue;
            }
            let prev = &bands[index - 1];
            let next = &bands[index + 1];
            if prev.gutters.len() != next.gutters.len() || prev.gutters.is_empty() {
                continue;
            }
            let gutters_close = prev
                .gutters
                .iter()
                .zip(next.gutters.iter())
                .all(|(l, r)| (l - r).abs() <= page_x_span * 0.08);
            if !gutters_close {
                continue;
            }
            let candidate_gutters: Vec<f32> = prev
                .gutters
                .iter()
                .zip(next.gutters.iter())
                .map(|(l, r)| (l + r) * 0.5)
                .collect();
            let band_chars = chars
                .iter()
                .copied()
                .filter(|ch| ch.y <= bands[index].y_top && ch.y > bands[index].y_bottom)
                .collect::<Vec<_>>();
            let looks_like_banner = candidate_gutters.first().is_some_and(|&split_x| {
                band_looks_like_full_width_banner(&band_chars, split_x, x_min_global, x_max_global)
            });
            if !looks_like_banner {
                bands[index].gutters = candidate_gutters;
            }
        }
    }

    // A section can end (or begin) with one sparse row that falls across the
    // fixed band boundary. Reuse the adjacent multi-column layout when that
    // row is not a centered/full-width banner; otherwise it would be treated
    // as a new single-column section or have word gaps mistaken for gutters.
    if bands.len() >= 2 {
        for index in [0usize, bands.len() - 1] {
            if !bands[index].gutters.is_empty() {
                continue;
            }
            let band_chars = chars
                .iter()
                .copied()
                .filter(|ch| ch.y <= bands[index].y_top && ch.y > bands[index].y_bottom)
                .collect::<Vec<_>>();
            if build_raw_rows(&band_chars, estimate_y_tolerance(&band_chars)).len() > 1 {
                continue;
            }
            let neighbor_index = if index == 0 { 1 } else { bands.len() - 2 };
            let neighbor = &bands[neighbor_index];
            if neighbor.gutters.is_empty() {
                continue;
            }
            let Some(&split_x) = neighbor.gutters.first() else {
                continue;
            };
            if !band_looks_like_full_width_banner(&band_chars, split_x, x_min_global, x_max_global)
            {
                bands[index].gutters = neighbor.gutters.clone();
            }
        }
    }

    let mut sections = Vec::new();
    for band in bands.into_iter().filter(|band| band.char_count > 0) {
        let band_column_count = (band.gutters.len() + 1) as u8;
        let can_merge = sections.last().is_some_and(|current: &LayoutSection| {
            if current.column_count != band_column_count {
                return false;
            }
            if current.gutters.len() != band.gutters.len() {
                return false;
            }
            current
                .gutters
                .iter()
                .zip(band.gutters.iter())
                .all(|(l, r)| (l - r).abs() <= page_x_span * 0.08)
        });
        if can_merge {
            if let Some(current) = sections.last_mut() {
                current.y_bottom = band.y_bottom;
                for (current_gutter, band_gutter) in
                    current.gutters.iter_mut().zip(band.gutters.iter())
                {
                    *current_gutter = (*current_gutter + band_gutter) * 0.5;
                }
            }
            continue;
        }
        sections.push(LayoutSection {
            y_top: band.y_top,
            y_bottom: band.y_bottom,
            column_count: band_column_count,
            gutters: band.gutters,
        });
    }

    if sections.is_empty() {
        sections.push(LayoutSection {
            y_top: y_max,
            y_bottom: y_min - 0.5,
            column_count: 1,
            gutters: Vec::new(),
        });
    }
    sections
}

/// Decide whether the current page's opening section continues the previous
/// page's last section, so multi-column passage text that flows across pages
/// stays in one continuous reading order instead of restarting at section 0.
///
/// Returns `(section_index_offset, continues_previous)`. When the page does
/// NOT continue, both are 0/false (the page's sections number from 0 as
/// before). When it DOES continue, `section_index_offset` rewrites this page's
/// section indices so the opening section shares the previous page's last
/// section number, and `continues_previous` is true so `_epic8GlobalSection`
/// gets emitted for downstream reading-order continuity.
fn cross_page_section_continuation(
    page_sections: &[LayoutSection],
    blocks: &[BlockWithLayout],
    prev_page_last_section: Option<&(u8, Vec<f32>, usize)>,
) -> (usize, bool) {
    let Some((prev_columns, prev_gutters, prev_last_global)) = prev_page_last_section else {
        return (0, false);
    };
    let prev_last_global = *prev_last_global;
    let Some(first_section) = page_sections.first() else {
        return (0, false);
    };
    if first_section.column_count != *prev_columns {
        return (0, false);
    }
    if first_section.gutters.len() != prev_gutters.len() {
        return (0, false);
    }
    // Gutters must be roughly at the same x for the columns to be the same flow.
    let all_x = first_section
        .gutters
        .iter()
        .chain(prev_gutters.iter())
        .copied();
    let span = all_x.clone().fold(f32::MIN, f32::max) - all_x.fold(f32::MAX, f32::min);
    let span = span.max(1.0);
    let gutters_match = first_section
        .gutters
        .iter()
        .zip(prev_gutters.iter())
        .all(|(l, r)| (l - r).abs() <= span * 0.08);
    if !gutters_match {
        return (0, false);
    }
    // Don't continue if the page opens with a section heading (READING PASSAGE,
    // Questions N-M) — that signals a new logical block, not a continuation.
    let opens_with_section_heading = blocks
        .first()
        .map(|block| looks_like_hard_line_break(&block.text))
        .unwrap_or(false);
    if opens_with_section_heading {
        return (0, false);
    }
    // Rewrite this page's section 0 to equal the previous page's last section
    // number, so the two pages share a continuous section flow.
    (prev_last_global, true)
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
    let mut emitted_section_index = 0usize;
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
            let lines = build_lines_within_column_refined(&section_chars, y_tol);
            let grouped = group_lines_into_blocks(&lines);
            result.extend(grouped.into_iter().map(|(text, bbox)| BlockWithLayout {
                text,
                bbox,
                section_index: emitted_section_index,
                column_index: 0,
                column_count: 1,
            }));
            emitted_section_index += 1;
            continue;
        }
        // Multi-column section. Use the first gutter for the banner heuristic
        // (a full-width banner crosses every gutter, so checking the leftmost
        // one is sufficient and keeps backward-compatible behaviour).
        let split_x = section.split_x();
        let section_x_min = section_chars.iter().map(|c| c.x).fold(f32::MAX, f32::min);
        let section_x_max = section_chars.iter().map(|c| c.x).fold(f32::MIN, f32::max);
        let rows = build_raw_rows(&section_chars, y_tol);
        let mut segments: Vec<(bool, Vec<CharWithOrigin>)> = Vec::new();
        for row in rows {
            let is_banner = is_full_width_banner_row(&row, split_x, section_x_min, section_x_max);
            match segments.last_mut() {
                Some((current_banner, segment_chars)) if *current_banner == is_banner => {
                    segment_chars.extend(row);
                }
                _ => segments.push((is_banner, row)),
            }
        }

        for (is_banner, segment_chars) in segments {
            if is_banner {
                let lines = build_lines_within_column_refined(&segment_chars, y_tol);
                let grouped = group_lines_into_blocks(&lines);
                result.extend(grouped.into_iter().map(|(text, bbox)| BlockWithLayout {
                    text,
                    bbox,
                    section_index: emitted_section_index,
                    column_index: 0,
                    column_count: 1,
                }));
                emitted_section_index += 1;
                continue;
            }

            // Split the segment into N column buckets by the section gutters.
            // For a 2-column section this is equivalent to the old left/right
            // hard-coded split; for 3 columns it yields left/middle/right.
            let column_count = section.column_count;
            let mut columns: Vec<Vec<CharWithOrigin>> =
                (0..column_count).map(|_| Vec::new()).collect();
            for ch in segment_chars {
                let column_index = section
                    .gutters
                    .iter()
                    .filter(|gutter| ch.x >= **gutter)
                    .count() as u8;
                let bucket = column_index as usize;
                if bucket < columns.len() {
                    columns[bucket].push(ch);
                } else {
                    columns.last_mut().unwrap().push(ch);
                }
            }
            for (column_index, column_chars) in columns.into_iter().enumerate() {
                if column_chars.is_empty() {
                    continue;
                }
                let lines = build_lines_within_column_refined(&column_chars, y_tol);
                let grouped = group_lines_into_blocks(&lines);
                result.extend(grouped.into_iter().map(|(text, bbox)| BlockWithLayout {
                    text,
                    bbox,
                    section_index: emitted_section_index,
                    column_index: column_index as u8,
                    column_count,
                }));
            }
            emitted_section_index += 1;
        }
    }
    result
}

/// Build lines from a single column's characters (already x-isolated from
/// other columns). Clusters by y-origin proximity, then splits into words.
#[allow(dead_code)]
fn build_lines_within_column(chars: &[CharWithOrigin], y_tol: f32) -> Vec<(String, [f32; 4])> {
    if chars.is_empty() {
        return Vec::new();
    }
    let lines = build_raw_rows(chars, y_tol);

    let mut result = Vec::new();
    for line in lines {
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

fn build_lines_within_column_refined(
    chars: &[CharWithOrigin],
    y_tol: f32,
) -> Vec<(String, [f32; 4])> {
    if chars.is_empty() {
        return Vec::new();
    }
    let lines = build_raw_rows(chars, y_tol);
    let mut result = Vec::new();
    for line in lines {
        let gap_threshold = line_word_gap_threshold(&line);
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
        result.push((words.join(" "), line_bbox(&line, y_tol)));
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
    // Track per-group running stats so the merge thresholds can adapt to the
    // paragraph being built (e.g. wider/justified paragraphs, indented first
    // lines) instead of using fixed pixel constants that fail on real-world
    // typography.
    #[derive(Clone)]
    struct GroupStats {
        text: String,
        bbox: [f32; 4],
        line_lefts: Vec<f32>,
        line_widths: Vec<f32>,
        line_rights: Vec<f32>,
        line_heights: Vec<f32>,
    }

    impl GroupStats {
        fn new(text: String, bbox: [f32; 4]) -> Self {
            let height = (bbox[3] - bbox[1]).abs().max(1.0);
            let width = (bbox[2] - bbox[0]).abs().max(1.0);
            GroupStats {
                text,
                bbox,
                line_lefts: vec![bbox[0]],
                line_widths: vec![width],
                line_rights: vec![bbox[2]],
                line_heights: vec![height],
            }
        }

        fn mean_left(&self) -> f32 {
            if self.line_lefts.is_empty() {
                return 0.0;
            }
            self.line_lefts.iter().sum::<f32>() / self.line_lefts.len() as f32
        }

        fn mean_height(&self) -> f32 {
            if self.line_heights.is_empty() {
                return 12.0;
            }
            self.line_heights.iter().sum::<f32>() / self.line_heights.len() as f32
        }

        fn mean_width(&self) -> f32 {
            if self.line_widths.is_empty() {
                return 0.0;
            }
            self.line_widths.iter().sum::<f32>() / self.line_widths.len() as f32
        }

        fn push_line(&mut self, text: &str, bbox: [f32; 4]) {
            self.text = format!("{} {}", self.text, text);
            self.bbox = [
                self.bbox[0].min(bbox[0]),
                self.bbox[1].min(bbox[1]),
                self.bbox[2].max(bbox[2]),
                self.bbox[3].max(bbox[3]),
            ];
            self.line_lefts.push(bbox[0]);
            self.line_widths.push((bbox[2] - bbox[0]).abs().max(1.0));
            self.line_rights.push(bbox[2]);
            self.line_heights.push((bbox[3] - bbox[1]).abs().max(1.0));
        }
    }

    let mut groups: Vec<GroupStats> = Vec::new();
    for (text, bbox) in lines {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let should_join = match groups.last_mut() {
            Some(prev) => {
                let prev_height = prev.mean_height();
                let current_height = ((bbox[3] - bbox[1]).abs()).max(1.0);
                let gap = (prev.bbox[1] - bbox[3]).max(0.0);
                let left_delta = (bbox[0] - prev.bbox[0]).abs();
                let center_delta =
                    (((bbox[0] + bbox[2]) * 0.5) - ((prev.bbox[0] + prev.bbox[2]) * 0.5)).abs();
                let width_delta = ((bbox[2] - bbox[0]) - prev.mean_width()).abs();
                // Adaptive thresholds: scale by the paragraph's running mean
                // height/width so larger fonts and wider justified columns
                // tolerate larger deltas. Floor by the original constants so
                // small-font text still merges tightly.
                let gap_tol = prev_height.max(current_height) * 0.9;
                let left_tol = (prev_height * 2.3).max(28.0);
                let center_tol = (prev_height * 3.3).max(40.0);
                let width_tol = (prev.mean_width() * 0.5).max(80.0);
                // First-line indent detection: a line that starts clearly to
                // the right of the paragraph's mean left edge signals a new
                // paragraph (classic indented first line). ~2 character widths
                // of indent is enough to be confident.
                let char_width = (prev_height * 0.45).max(2.0);
                let indented = bbox[0] > prev.mean_left() + char_width * 2.0;
                !indented
                    && gap <= gap_tol
                    && left_delta <= left_tol
                    && center_delta <= center_tol
                    && width_delta <= width_tol
                    && !looks_like_hard_line_break(trimmed)
                    && !prev.text.trim_end().ends_with(':')
            }
            None => false,
        };
        if should_join {
            groups.last_mut().unwrap().push_line(trimmed, *bbox);
        } else {
            groups.push(GroupStats::new(trimmed.to_string(), *bbox));
        }
    }
    groups
        .into_iter()
        .map(|stats| (stats.text, stats.bbox))
        .collect()
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
    global_section: Option<usize>,
) -> Value {
    let mut block = document_block_with_bbox(block_id, text, page_index, ordinal, confidence, bbox);
    if let Some(obj) = block.as_object_mut() {
        obj.insert("_epic8LayoutSection".to_string(), json!(section_index));
        obj.insert("_epic8ColumnIndex".to_string(), json!(column_index));
        obj.insert("_epic8SectionColumns".to_string(), json!(column_count));
        if let Some(global) = global_section {
            obj.insert("_epic8GlobalSection".to_string(), json!(global));
        }
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

    #[test]
    fn build_blocks_from_chars_preserves_centered_banner_inside_two_column_flow() {
        let mut chars = Vec::new();
        push_text_line(
            &mut chars,
            "LEFT COLUMN INTRODUCES THE FIRST IDEA",
            78.0,
            760.0,
        );
        push_text_line(
            &mut chars,
            "RIGHT COLUMN ADDS SUPPORTING DETAIL",
            336.0,
            756.0,
        );
        push_text_line(
            &mut chars,
            "LEFT COLUMN CONTINUES THE ARGUMENT",
            78.0,
            752.0,
        );
        push_text_line(
            &mut chars,
            "RIGHT COLUMN CONTINUES THE ARGUMENT",
            336.0,
            748.0,
        );
        push_text_line(&mut chars, "FURTHER EVIDENCE FROM JAPAN", 238.0, 744.0);
        push_text_line(
            &mut chars,
            "LEFT COLUMN RESUMES BELOW THE BANNER",
            78.0,
            736.0,
        );
        push_text_line(
            &mut chars,
            "RIGHT COLUMN RESUMES BELOW THE BANNER",
            336.0,
            732.0,
        );
        push_text_line(
            &mut chars,
            "LEFT COLUMN CLOSES WITH A FINAL CLAIM",
            78.0,
            728.0,
        );
        push_text_line(
            &mut chars,
            "RIGHT COLUMN CLOSES WITH A FINAL CLAIM",
            336.0,
            724.0,
        );

        let blocks = build_blocks_from_chars(&chars);
        let debug_blocks = blocks
            .iter()
            .map(|block| {
                format!(
                    "[section={} col={}/{} text={}]",
                    block.section_index, block.column_index, block.column_count, block.text
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let banner_index = blocks
            .iter()
            .position(|block| {
                block.column_count == 1 && block.text.contains("FURTHER EVIDENCE FROM JAPAN")
            })
            .expect("centered banner should remain a single-column block");

        assert_eq!(
            blocks[banner_index].text, "FURTHER EVIDENCE FROM JAPAN",
            "banner text should not be split into separate column fragments"
        );
        assert!(
            banner_index >= 2,
            "banner should appear after the opening two-column blocks: {}",
            debug_blocks
        );
        assert!(
            blocks
                .iter()
                .skip(banner_index + 1)
                .any(|block| block.column_count == 2 && block.column_index == 0),
            "left column should continue after the centered banner: {}",
            debug_blocks
        );
        assert!(
            blocks
                .iter()
                .skip(banner_index + 1)
                .any(|block| block.column_count == 2 && block.column_index == 1),
            "right column should continue after the centered banner: {}",
            debug_blocks
        );
    }

    #[test]
    fn detect_column_gutters_finds_three_columns() {
        // Three non-overlapping columns separated by two gutters around x=185
        // and x=350.
        // Each column needs enough characters for the recursive gutter detector
        // to satisfy its per-side minimum (>=24 chars per sub-range).
        let mut chars = Vec::new();
        let ys = [760.0, 751.0, 742.0, 733.0, 724.0, 715.0, 706.0, 697.0];
        // Left column
        for (i, y) in ys.iter().enumerate() {
            push_text_line(&mut chars, &format!("LEFT COLUMN LINE {}", i + 1), 60.0, *y);
        }
        // Middle column
        for (i, y) in ys.iter().enumerate() {
            push_text_line(
                &mut chars,
                &format!("MIDDLE COLUMN LINE {}", i + 1),
                220.0,
                *y,
            );
        }
        // Right column
        for (i, y) in ys.iter().enumerate() {
            push_text_line(
                &mut chars,
                &format!("RIGHT COLUMN LINE {}", i + 1),
                380.0,
                *y,
            );
        }

        let x_min = chars.iter().map(|c| c.x).fold(f32::MAX, f32::min);
        let x_max = chars.iter().map(|c| c.x).fold(f32::MIN, f32::max);
        let gutters = detect_column_gutters(&chars, x_min, x_max);

        assert_eq!(
            gutters.len(),
            2,
            "expected two gutters for a 3-column layout, got {:?}",
            gutters
        );
        // First gutter should sit between left and middle column (~x=185).
        assert!(
            gutters[0] > 180.0 && gutters[0] < 240.0,
            "first gutter should be near x=200, got {}",
            gutters[0]
        );
        // Second gutter should sit between middle and right column (~x=350).
        assert!(
            gutters[1] > 300.0 && gutters[1] < 360.0,
            "second gutter should be near x=340, got {}",
            gutters[1]
        );
    }

    #[test]
    fn build_blocks_from_chars_preserves_three_column_sections() {
        let mut chars = Vec::new();
        let ys = [760.0, 751.0, 742.0, 733.0, 724.0, 715.0, 706.0, 697.0];
        for y in ys.iter() {
            push_text_line(&mut chars, "LEFT COL LINE TEXT", 60.0, *y);
            push_text_line(&mut chars, "MIDDLE COL LINE TEXT", 220.0, *y);
            push_text_line(&mut chars, "RIGHT COL LINE TEXT", 380.0, *y);
        }

        let blocks = build_blocks_from_chars(&chars);
        let column_indices: std::collections::HashSet<u8> = blocks
            .iter()
            .filter(|block| block.column_count == 3)
            .map(|block| block.column_index)
            .collect();
        assert!(
            column_indices.contains(&0)
                && column_indices.contains(&1)
                && column_indices.contains(&2),
            "expected all three column indices in a 3-column section, got blocks: {:?}",
            blocks
                .iter()
                .map(|block| format!(
                    "[section={} col={}/{:?} text={}]",
                    block.section_index, block.column_index, block.column_count, block.text
                ))
                .collect::<Vec<_>>()
        );
        assert!(
            blocks.iter().any(|block| block.column_count == 3),
            "expected at least one 3-column section block"
        );
    }

    #[test]
    fn build_blocks_from_chars_preserves_single_column_then_two_column_layout() {
        // Top: single-column full-width paragraph. Bottom: two-column flow.
        // This is the reverse of the existing mixed-column test and guards
        // against the layout switch being missed when it goes 1 -> 2.
        let mut chars = Vec::new();
        // Several full-width lines at the top so the single-column section has
        // enough density to be detected as its own section.
        for (i, y) in [760.0, 751.0, 742.0, 733.0].iter().enumerate() {
            push_text_line(
                &mut chars,
                &format!(
                    "FULL WIDTH INTRODUCTORY PARAGRAPH LINE NUMBER {} SPANS THE WHOLE PAGE",
                    i + 1
                ),
                78.0,
                *y,
            );
        }
        // Now switch to two columns lower down (a large y-gap separates them so
        // the layout detector sees two distinct bands).
        for y in [500.0, 491.0, 482.0, 473.0].iter() {
            push_text_line(&mut chars, "LEFT COLUMN PASSAGE TEXT LINE", 78.0, *y);
            push_text_line(&mut chars, "RIGHT COLUMN PASSAGE TEXT LINE", 336.0, *y);
        }

        let blocks = build_blocks_from_chars(&chars);
        let single_column_blocks: Vec<_> = blocks
            .iter()
            .filter(|block| block.column_count == 1)
            .collect();
        let two_column_blocks: Vec<_> = blocks
            .iter()
            .filter(|block| block.column_count == 2)
            .collect();

        assert!(
            !single_column_blocks.is_empty(),
            "expected a leading single-column section"
        );
        assert!(
            !two_column_blocks.is_empty(),
            "expected a trailing two-column section, got blocks: {:?}",
            blocks
                .iter()
                .map(|block| format!(
                    "[section={} col={}/{:?} text={}]",
                    block.section_index, block.column_index, block.column_count, block.text
                ))
                .collect::<Vec<_>>()
        );
        // The single-column section must come BEFORE the two-column section
        // (lower section_index) so reading order is preserved.
        let max_single_section = single_column_blocks
            .iter()
            .map(|block| block.section_index)
            .max()
            .unwrap_or(0);
        let min_two_section = two_column_blocks
            .iter()
            .map(|block| block.section_index)
            .min()
            .unwrap_or(0);
        assert!(
            min_two_section > max_single_section,
            "two-column section should come after the single-column intro"
        );
        assert!(
            two_column_blocks
                .iter()
                .any(|block| block.column_index == 0),
            "left column should be present in the two-column section"
        );
        assert!(
            two_column_blocks
                .iter()
                .any(|block| block.column_index == 1),
            "right column should be present in the two-column section"
        );
    }

    #[test]
    fn group_lines_into_blocks_respects_first_line_indent() {
        // Two paragraphs in a single column. The second paragraph's first line
        // is indented by ~3 character widths; it should NOT be merged into the
        // first paragraph.
        let mut chars = Vec::new();
        // Paragraph 1 (left edge x=80)
        push_text_line(&mut chars, "FIRST PARAGRAPH LINE ONE", 80.0, 760.0);
        push_text_line(&mut chars, "FIRST PARAGRAPH LINE TWO", 80.0, 751.0);
        // Paragraph 2 first line indented to x=100 (>2 char widths of indent)
        push_text_line(&mut chars, "SECOND PARAGRAPH OPENS HERE", 100.0, 742.0);
        push_text_line(&mut chars, "SECOND PARAGRAPH LINE TWO", 80.0, 733.0);

        let blocks = build_blocks_from_chars(&chars);
        // We expect at least 2 distinct paragraph blocks (one per paragraph).
        assert!(
            blocks.len() >= 2,
            "expected the indented paragraph to be split into its own block, got {} blocks: {:?}",
            blocks.len(),
            blocks
                .iter()
                .map(|block| block.text.clone())
                .collect::<Vec<_>>()
        );
        assert!(
            blocks
                .iter()
                .any(|block| block.text.contains("SECOND PARAGRAPH OPENS")),
            "the indented opening line should belong to a block containing the second paragraph"
        );
    }

    #[test]
    fn band_looks_like_full_width_banner_accepts_multi_row_banner() {
        // A 5-row centered banner crossing a gutter at x=300. Previously the
        // <=3 row cap would reject this; the ratio-based check should accept.
        let mut chars = Vec::new();
        for (i, text) in [
            "FIGURE ONE CAPTION LINE",
            "WITH A SECOND LINE OF DETAIL",
            "AND A THIRD LINE OF CLARITY",
            "FOLLOWED BY A FOURTH LINE",
            "AND A FIFTH FINAL LINE",
        ]
        .iter()
        .enumerate()
        {
            // Center each line around x=180 (narrow + centered).
            let line_width = text.len() as f32 * 4.8;
            let x_start = 300.0 - line_width * 0.5;
            push_text_line(&mut chars, text, x_start, 760.0 - i as f32 * 9.0);
        }

        let rows = build_raw_rows(&chars, estimate_y_tolerance(&chars));
        assert!(
            rows.len() > 3,
            "test setup should have >3 banner rows, got {}",
            rows.len()
        );
        let result = band_looks_like_full_width_banner(&chars, 300.0, 60.0, 540.0);
        assert!(
            result,
            "a 5-row centered banner should be detected as a full-width banner band"
        );
    }
}
