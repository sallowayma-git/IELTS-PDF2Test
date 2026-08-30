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
use std::sync::{Mutex, MutexGuard, OnceLock};

use pdfium_render::prelude::*;
use serde_json::{json, Value};

use crate::CommandResult;
use crate::{ImportJob, SourceFile};

/// pdfium binds the native library process-wide on first use. Multiple tests
/// in the same process each call `bind_pdfium()`, which the binding registry
/// (`BINDINGS`) rejects a second time. Cache one `Pdfium` instance and hand
/// out a guard to the shared instance so geometry tests can run back-to-back
/// in the default parallel harness (the guard serializes them).
static PDFIUM_INSTANCE: OnceLock<Mutex<Result<Pdfium, String>>> = OnceLock::new();

fn pdfium_instance() -> Result<MutexGuard<'static, Result<Pdfium, String>>, String> {
    let cached = PDFIUM_INSTANCE.get_or_init(|| Mutex::new(bind_pdfium()));
    let guard = cached
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_err() {
        return Err("pdfium_bind_failed_global".to_string());
    }
    Ok(guard)
}

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
    // Developer/test trees: `npm run fetch:pdfium` installs the library next
    // to the crate manifest (`src-tauri/lib/pdfium-<platform>/`). Cargo test
    // binaries live under `target/debug/deps/`, which the exe-relative probes
    // above cannot see, so fall back to the manifest directory when Cargo
    // exposed it at runtime (dev/test only; packaged apps resolve via the exe).
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        if let Some(folder) = platform_pdfium_folder(Path::new(&manifest_dir)) {
            let candidate = folder.join(Pdfium::pdfium_platform_library_name());
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

#[derive(Clone, Debug, PartialEq)]
struct DiagramQuestionRegionCandidate {
    page_index: usize,
    insert_after: usize,
    question_start: u32,
    question_end: u32,
    bbox: [f32; 4],
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
    let _pdfium_guard = pdfium_instance()?;
    let pdfium = _pdfium_guard.as_ref().map_err(|error| error.clone())?;
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

    let diagram_asset_dir = output_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            "{}-assets/diagram-question-regions",
            output_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("document-ir")
        ));
    let diagram_assets = recover_diagram_question_region_assets(
        &document,
        &mut pages,
        &diagram_asset_dir,
        &mut block_counter,
        &mut warnings,
    )?;

    let ir = json!({
        "schemaVersion": "DocumentIRV1",
        "jobId": job.job_id,
        "pages": pages,
        "assets": diagram_assets,
        "parser": {
            "provider": "rust-parser:pdf:pdfium",
            "version": "0.1.0",
            "mode": mode,
            "recognitionPipeline": "geometry_structure_v2",
            "geometryAuthoritative": true,
            "degradedFallback": false,
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

/// Normalize whitespace artifacts introduced by PDF text matrices without
/// changing the source characters themselves.  Pdfium can expose a small
/// positive advance as a word gap after a page rotation, which yields text
/// such as `troubles .` or `High - level`.  Closing punctuation never needs a
/// leading space, and an ASCII hyphen between a multi-letter word and a
/// lowercase continuation is a compound-word join rather than a list/range
/// separator.  Keep the rule deliberately narrow so `A - B` remains spaced.
fn canonicalize_extracted_line_text(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars = collapsed.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(collapsed.len());
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        if matches!(
            ch,
            '.' | ',' | ';' | ':' | '?' | '!' | '%' | ')' | ']' | '}'
        ) {
            while output.ends_with(' ') {
                output.pop();
            }
            output.push(ch);
            index += 1;
            continue;
        }

        if ch == '-' {
            let previous_word_len = output
                .trim_end()
                .chars()
                .rev()
                .take_while(|candidate| candidate.is_alphabetic())
                .count();
            let next = chars
                .iter()
                .skip(index + 1)
                .copied()
                .find(|candidate| !candidate.is_whitespace());
            if previous_word_len >= 2 && next.is_some_and(|candidate| candidate.is_lowercase()) {
                while output.ends_with(' ') {
                    output.pop();
                }
                output.push('-');
                index += 1;
                while index < chars.len() && chars[index].is_whitespace() {
                    index += 1;
                }
                continue;
            }
        }

        output.push(ch);
        index += 1;
    }

    output
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
        // Keep the crossing-gap threshold tied to typography rather than a
        // percentage of the page width. A compact two-column gutter can be
        // only ~18pt; treating that as a continuous banner line would merge
        // both columns before the geometry splitter gets a chance to bucket
        // them. Centered banners still have a normal word gap here.
        let max_continuous_gap = line_word_gap_threshold(line).max(14.0).min(16.0);
        // A row can have a short literal gap even though it is two independent
        // column lines (a long left question ending just before an option in
        // the right column). Preserve the split when both sides start from
        // their respective column edges. Centered banners start near the
        // middle of the page and therefore do not satisfy this geometry.
        let left_start = line
            .iter()
            .filter(|ch| ch.x < split_x)
            .map(|ch| ch.x)
            .fold(f32::MAX, f32::min);
        let right_start = line
            .iter()
            .filter(|ch| ch.x >= split_x)
            .map(|ch| ch.x)
            .fold(f32::MAX, f32::min);
        // A reflowed two-column PDF can place a long left-column line right up
        // against the next column's first glyph. In that case the right glyph
        // starts only a few points after the gutter (the nominal 18pt gutter
        // is consumed by the left line's final word). Treat this as two
        // independent column starts when the left side is page-edge anchored
        // and substantial enough to be a real text line. The old lower bound
        // (`split + 2%`) rejected this exact geometry and classified it as a
        // full-width banner.
        let right_column_edge_offset = right_start - split_x;
        let left_last_char = line
            .iter()
            .filter(|ch| ch.x < split_x && !ch.ch.is_whitespace())
            .max_by(|a, b| a.x.total_cmp(&b.x))
            .map(|ch| ch.ch);
        let mut right_chars = line
            .iter()
            .filter(|ch| ch.x >= split_x && !ch.ch.is_whitespace())
            .copied()
            .collect::<Vec<_>>();
        right_chars.sort_by(|a, b| a.x.total_cmp(&b.x));
        let right_first_char = right_chars.first().map(|ch| ch.ch);
        let right_lowercase_prefix_len = right_chars
            .iter()
            .take_while(|ch| ch.ch.is_ascii_lowercase())
            .count();
        let right_prefix_is_option_tail = right_lowercase_prefix_len > 0
            && right_chars
                .iter()
                .nth(right_lowercase_prefix_len)
                .is_some_and(|ch| ch.ch.is_ascii_uppercase())
            && right_lowercase_prefix_len <= 3;
        // A word may be physically split across a compact gutter (`Ques` +
        // `tions`, or `Ne` + `il`). Treat that as one continuous row when the
        // next fragment is long enough to be a word continuation. A short
        // lowercase prefix followed by an uppercase option label (`n C`) is
        // deliberately excluded; that is a question/option boundary and is
        // repaired after column assignment instead.
        let split_word_continuation = left_last_char.is_some_and(|ch| ch.is_ascii_lowercase())
            && right_first_char.is_some_and(|ch| ch.is_ascii_lowercase())
            && right_lowercase_prefix_len > 0
            && !right_prefix_is_option_tail
            && split_gap <= max_continuous_gap;
        let split_number_continuation = left_last_char.is_some_and(|ch| ch.is_ascii_digit())
            && right_first_char.is_some_and(|ch| ch.is_ascii_digit())
            && split_gap <= max_continuous_gap;
        let independent_column_starts = left_start <= x_min_global + page_x_span * 0.12
            && right_column_edge_offset >= -1.0
            && right_column_edge_offset <= 5.0
            && left_count >= 20;
        if independent_column_starts {
            return false;
        }
        if split_word_continuation || split_number_continuation {
            return true;
        }
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

/// Repair a word that was physically split at a narrow column boundary.
///
/// The safe legacy case is an option label following the final letter of a
/// question (`a` + `n C ...`). Move only that short lowercase prefix back to
/// the left row. Lowercase text alone is ambiguous and remains untouched
/// unless the dominant-edge repair below can prove its source ownership.
fn repair_cross_column_word_prefix(
    left: &mut Vec<CharWithOrigin>,
    right: &mut Vec<CharWithOrigin>,
    split_x: f32,
) {
    if left.is_empty() || right.is_empty() {
        return;
    }
    left.sort_by(|a, b| a.x.total_cmp(&b.x));
    right.sort_by(|a, b| a.x.total_cmp(&b.x));
    let left_last = left.last().copied();
    let right_first = right.first().copied();
    let (Some(left_last), Some(right_first)) = (left_last, right_first) else {
        return;
    };
    if !left_last.ch.is_ascii_alphabetic()
        || right_first.x - split_x < -1.0
        || right_first.x - split_x > 5.0
        || right_first.x - left_last.x > 14.0
    {
        return;
    }

    // Keep this legacy repair to the unambiguous option-tail shape (`n C`).
    // Ordinary lowercase text, including a short all-lowercase right-column
    // opening, is not sufficient evidence by itself. Longer source spans such
    // as `from price` and whitespace-bearing `e time therapies` are handled
    // below only after a repeated physical right edge has been established.
    let lowercase_prefix_len = right
        .iter()
        .enumerate()
        .take_while(|(index, ch)| {
            if !ch.ch.is_ascii_lowercase() {
                return false;
            }
            if *index == 0 {
                return true;
            }
            ch.x - right[*index - 1].x <= 14.0
        })
        .count();
    if lowercase_prefix_len == 0 || lowercase_prefix_len > 3 {
        return;
    }
    let is_option_tail = right
        .get(lowercase_prefix_len)
        .is_some_and(|ch| ch.ch.is_ascii_uppercase());
    if !is_option_tail {
        return;
    }
    let moved = right.drain(..lowercase_prefix_len).collect::<Vec<_>>();
    left.extend(moved);
    left.sort_by(|a, b| a.x.total_cmp(&b.x));
}

/// Find the repeated physical start of the right column within one layout
/// segment. The histogram gutter can be pulled left when a few long left
/// lines enter the otherwise empty gutter. A true right-column edge is more
/// stable: it recurs at (roughly) the same x on several independent rows.
///
/// Run starts, rather than just the first glyph to the right of `split_x`,
/// matter here. A source row can contain both a left-column spill (`from`) and
/// the real right-column text (`price ...`) separated by the remaining gutter.
fn dominant_right_column_edge(rows: &[Vec<CharWithOrigin>], split_x: f32) -> Option<f32> {
    #[derive(Clone, Copy)]
    struct EdgeCluster {
        x_sum: f32,
        row_count: usize,
        last_row: usize,
    }

    let mut candidates = Vec::<(usize, f32)>::new();
    for (row_index, row) in rows.iter().enumerate() {
        let mut glyphs = row
            .iter()
            .copied()
            .filter(|ch| !ch.ch.is_whitespace())
            .collect::<Vec<_>>();
        glyphs.sort_by(|a, b| a.x.total_cmp(&b.x));
        if glyphs.is_empty() {
            continue;
        }
        let run_gap = line_word_gap_threshold(&glyphs).max(14.0);
        for (index, ch) in glyphs.iter().enumerate() {
            if ch.x < split_x {
                continue;
            }
            let starts_run =
                index == 0 || glyphs[index - 1].x < split_x || ch.x - glyphs[index - 1].x > run_gap;
            if starts_run {
                candidates.push((row_index, ch.x));
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|left, right| left.1.total_cmp(&right.1));
    let mut clusters = Vec::<EdgeCluster>::new();
    for (row_index, x) in candidates {
        let matching = clusters.iter_mut().find(|cluster| {
            let center = cluster.x_sum / cluster.row_count as f32;
            (center - x).abs() <= 6.0
        });
        if let Some(cluster) = matching {
            // Count each physical row at most once in a cluster.
            if cluster.last_row != row_index {
                cluster.x_sum += x;
                cluster.row_count += 1;
                cluster.last_row = row_index;
            }
        } else {
            clusters.push(EdgeCluster {
                x_sum: x,
                row_count: 1,
                last_row: row_index,
            });
        }
    }

    clusters
        .into_iter()
        .filter(|cluster| {
            let center = cluster.x_sum / cluster.row_count as f32;
            center - split_x >= 16.0
                && cluster.row_count >= 3
                && cluster.row_count * 3 >= rows.len().max(1)
        })
        .max_by(|left, right| {
            left.row_count.cmp(&right.row_count).then_with(|| {
                let left_x = left.x_sum / left.row_count as f32;
                let right_x = right.x_sum / right.row_count as f32;
                left_x.total_cmp(&right_x)
            })
        })
        .map(|cluster| cluster.x_sum / cluster.row_count as f32)
}

/// Reattach a left-column tail that the histogram gutter stranded in the
/// right bucket. This deliberately requires three independent facts:
///
/// 1. a repeated right-column edge was established by other rows;
/// 2. this row has real text at that edge as well as a prefix before it; and
/// 3. that prefix is physically continuous with the left-column row.
///
/// The conjunction is what keeps genuine right-column openings such as
/// `in contrast ...` or a standalone `map` in the right column: without a
/// second run at the dominant edge they are never moved. Whitespace glyphs
/// are retained with the moved source span, so the repair works with pdfium
/// text layers that explicitly emit spaces.
fn repair_cross_column_prefix_before_dominant_edge(
    left: &mut Vec<CharWithOrigin>,
    right: &mut Vec<CharWithOrigin>,
    split_x: f32,
    dominant_edge: f32,
) {
    if left.is_empty() || right.is_empty() || dominant_edge - split_x < 16.0 {
        return;
    }
    left.sort_by(|a, b| a.x.total_cmp(&b.x));
    right.sort_by(|a, b| a.x.total_cmp(&b.x));

    let edge_floor = dominant_edge - 8.0;
    let Some(left_last) = left.iter().rev().find(|ch| !ch.ch.is_whitespace()) else {
        return;
    };
    let Some(prefix_first) = right
        .iter()
        .find(|ch| !ch.ch.is_whitespace() && ch.x < edge_floor)
    else {
        return;
    };
    let right_glyphs = right
        .iter()
        .copied()
        .filter(|ch| !ch.ch.is_whitespace())
        .collect::<Vec<_>>();
    if let Some(edge_run_index) = right_glyphs.iter().position(|ch| ch.x >= edge_floor) {
        if edge_run_index == 0 {
            return;
        }
        let run_gap = line_word_gap_threshold(&right_glyphs).max(14.0);
        if right_glyphs[edge_run_index].x - right_glyphs[edge_run_index - 1].x <= run_gap {
            return;
        }
    } else if right_glyphs.len() > 3 {
        // A very short row can consist solely of a stranded word tail (`Ne`
        // + `il`). Once a repeated real right edge is known, up to three
        // source-continuous glyphs entirely inside the gutter are not a real
        // right-column opening. Longer text still needs a second run at the
        // dominant edge as corroborating evidence.
        return;
    }

    let mut combined = left.clone();
    combined.extend(right.iter().copied().filter(|ch| ch.x < edge_floor));
    combined.sort_by(|a, b| a.x.total_cmp(&b.x));
    let continuity_limit = line_word_gap_threshold(&combined).max(14.0).min(16.0);
    if prefix_first.x - left_last.x > continuity_limit {
        return;
    }

    let mut moved = Vec::new();
    let mut retained = Vec::new();
    for ch in right.drain(..) {
        if ch.x < edge_floor {
            moved.push(ch);
        } else {
            retained.push(ch);
        }
    }
    if !moved.iter().any(|ch| !ch.ch.is_whitespace()) {
        *right = retained;
        return;
    }
    left.extend(moved);
    left.sort_by(|a, b| a.x.total_cmp(&b.x));
    *right = retained;
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
    // A narrow gutter can still be a real column boundary when it repeats on
    // the same baseline across several rows. Keep the old page-span minimum
    // for ordinary layouts, but allow a >=16pt candidate through to the row
    // evidence check below. This avoids manufacturing a split from one wide
    // word gap while supporting compact two-column question sheets.
    let narrow_gutter_min_width = 16.0;
    let narrow_gutter_min_bins = (narrow_gutter_min_width / bin_width).ceil() as usize;
    let is_narrow_candidate =
        best_gutter_len >= narrow_gutter_min_bins && best_gutter_len < min_gutter_bins;
    if best_gutter_len < min_gutter_bins && !is_narrow_candidate {
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
    let mut straddle_rows = 0usize;
    let mut repeated_large_gap = 0usize;
    let mut left_supported_rows = 0usize;
    let mut right_supported_rows = 0usize;
    let small_gap_threshold = if is_narrow_candidate {
        // A compact gutter is deliberately below the old page-span-derived
        // threshold. Ordinary word spaces remain below this fixed bound.
        14.0
    } else {
        page_x_span * 0.035
    };
    for row in &rows {
        let has_left = row.2;
        let has_right = row.3;
        // Reconstruct this row's chars (sorted by x) both to count repeated
        // vertical support on each side and, for crossing rows, to measure the
        // gap at split_x. line_split_gap assumes an x-sorted line.
        let mut row_chars: Vec<CharWithOrigin> = chars
            .iter()
            .copied()
            .filter(|ch| (ch.y - row.0).abs() <= y_tol)
            .collect();
        row_chars.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        let left_chars = row_chars.iter().filter(|ch| ch.x < split_x).count();
        let right_chars = row_chars.len().saturating_sub(left_chars);
        if left_chars >= 4 {
            left_supported_rows += 1;
        }
        if right_chars >= 4 {
            right_supported_rows += 1;
        }
        if !(has_left && has_right) {
            continue;
        }
        if left_chars < 4 || right_chars < 4 {
            continue;
        }
        straddle_rows += 1;
        let gap_at_split = line_split_gap(&row_chars, split_x).unwrap_or(page_x_span);
        if gap_at_split <= small_gap_threshold.max(14.0) {
            straddle_with_small_gap += 1;
        }
        // Require a gap materially larger than a normal inter-word space for
        // the narrow-candidate path. The repeated x-position plus this
        // per-row gap evidence is what distinguishes a compact gutter from a
        // paragraph whose words happen to leave one large space.
        let row_gap_threshold = (line_word_gap_threshold(&row_chars) * 1.15).max(14.0);
        if gap_at_split >= row_gap_threshold {
            repeated_large_gap += 1;
        }
    }
    // Character totals alone are not column evidence: one long instruction
    // row can put dozens of glyphs on the otherwise empty side of a candidate
    // while all table rows remain on the left. Require at least two distinct
    // baselines with substantive text on each side. This intentionally favors
    // a source-preserving single flow when there is not enough geometry to
    // prove a second column.
    if left_supported_rows < 2 || right_supported_rows < 2 {
        return None;
    }
    if is_narrow_candidate && (straddle_rows < 3 || repeated_large_gap * 3 < straddle_rows * 2) {
        return None;
    }
    // Judge word-gap evidence against the rows that actually CROSS the
    // candidate, not every row in the band. A single-column completion block
    // often has several short table rows plus two long control/answer rows;
    // using all rows as the denominator let the repeated word gap in those
    // long rows masquerade as a gutter (the checked-in complex fixture became
    // three columns). Two or more crossing rows whose split gaps are mostly
    // ordinary word spaces are affirmative evidence that this is continuous
    // text. A lone crossing row is left to the banner logic so a centered
    // heading does not erase a real surrounding column layout.
    if straddle_rows >= 2 && straddle_with_small_gap * 3 >= straddle_rows * 2 {
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

/// Return whether an apparent middle column is only a continuation of text
/// that crossed the first gutter. A genuine middle column starts materially
/// inside its bucket; a false recursive split has its first glyph almost on
/// the candidate gutter in repeated rows.
fn middle_bucket_is_gutter_continuation(
    rows: &[Vec<CharWithOrigin>],
    first_split: f32,
    second_split: f32,
) -> bool {
    let mut middle_rows = 0usize;
    let mut near_gutter_rows = 0usize;
    let mut typographically_continuous_rows = 0usize;
    for row in rows {
        let middle = row
            .iter()
            .filter(|ch| ch.x >= first_split && ch.x < second_split)
            .collect::<Vec<_>>();
        if middle.len() < 4 {
            continue;
        }
        middle_rows += 1;
        let middle_start = middle.iter().map(|ch| ch.x).fold(f32::MAX, f32::min);
        if middle_start - first_split <= 8.0 {
            near_gutter_rows += 1;
            // Starting close to the detected split is not sufficient evidence
            // that this bucket is a stranded tail. Compact real columns can
            // start immediately after a narrow gutter as well. Require the
            // preceding glyph on the same baseline to be separated by no more
            // than an ordinary word gap; a stable, wider separation is direct
            // evidence for an independent middle column.
            if let Some(split_gap) = line_split_gap(row, first_split) {
                if split_gap <= line_word_gap_threshold(row) {
                    typographically_continuous_rows += 1;
                }
            }
        }
    }
    middle_rows >= 2
        && near_gutter_rows * 3 >= middle_rows * 2
        && typographically_continuous_rows * 3 >= middle_rows * 2
}

/// Decide whether moving a sparse edge band's gutter rightward only returns
/// source-continuous text tails to the left column. The adjacent band must
/// establish a repeated physical right-column edge, and every glyph whose
/// owner would change must continue a same-baseline left run. Genuine shifted
/// two-column panels fail this proof and retain their own gutter.
fn sparse_edge_ownership_changes_are_left_continuations(
    band_chars: &[CharWithOrigin],
    current_split: f32,
    adjacent_split: f32,
    adjacent_chars: &[CharWithOrigin],
) -> bool {
    if current_split >= adjacent_split {
        return false;
    }
    let adjacent_rows = build_raw_rows(adjacent_chars, estimate_y_tolerance(adjacent_chars));
    let Some(dominant_edge) = dominant_right_column_edge(&adjacent_rows, adjacent_split) else {
        return false;
    };
    if dominant_edge <= adjacent_split {
        return false;
    }

    let rows = build_raw_rows(band_chars, estimate_y_tolerance(band_chars));
    let mut changed_row_count = 0usize;
    for row in rows {
        let mut glyphs = row
            .iter()
            .copied()
            .filter(|ch| !ch.ch.is_whitespace())
            .collect::<Vec<_>>();
        glyphs.sort_by(|left, right| left.x.total_cmp(&right.x));
        let changed = glyphs
            .iter()
            .copied()
            .filter(|ch| ch.x >= current_split && ch.x < adjacent_split)
            .collect::<Vec<_>>();
        if changed.is_empty() {
            continue;
        }
        changed_row_count += 1;
        if changed.iter().any(|ch| ch.x >= dominant_edge - 8.0) {
            return false;
        }
        let Some(left_last) = glyphs.iter().rev().find(|ch| ch.x < current_split).copied() else {
            return false;
        };
        let continuity_limit = line_word_gap_threshold(&glyphs).max(14.0).min(16.0);
        if changed[0].x - left_last.x > continuity_limit {
            return false;
        }
    }
    changed_row_count > 0
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
    let page_x_span = (x_max_global - x_min_global).max(1.0);
    let band_ranges = adaptive_horizontal_band_ranges(chars, y_min, y_max);
    let mut bands = Vec::with_capacity(band_ranges.len());

    for (y_top, y_bottom) in band_ranges {
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

    // Fill empty bands between two bands that agree on their gutters, so a
    // sparse or text-heavy row that does not independently expose a histogram
    // valley does not split a continuous multi-column region in half. Only
    // fills when the surrounding gutters match (same column count and close
    // gutter x), and the band itself does not look like a full-width banner
    // crossing the gutter.
    if bands.len() >= 3 {
        for index in 1..bands.len() - 1 {
            if !bands[index].gutters.is_empty() {
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
        // Repair the lower edge first so a reliable upper band is not
        // overwritten by a misleading sparse lower-band candidate.
        for index in [bands.len() - 1, 0usize] {
            let band_chars = chars
                .iter()
                .copied()
                .filter(|ch| ch.y <= bands[index].y_top && ch.y > bands[index].y_bottom)
                .collect::<Vec<_>>();
            if build_raw_rows(&band_chars, estimate_y_tolerance(&band_chars)).len() > 6 {
                continue;
            }
            let neighbor_index = if index == 0 { 1 } else { bands.len() - 2 };
            let neighbor = &bands[neighbor_index];
            if neighbor.gutters.is_empty() {
                continue;
            }
            // A sparse edge band may expose a misleading gutter, but equal
            // column counts alone do not prove that it belongs to the same
            // layout. An inset panel can have two real columns whose gutter
            // is far from the adjacent two-column body. Replace an explicit
            // edge candidate only when the gutter positions are close AND
            // moving them would preserve every source glyph's column owner.
            // An empty edge band can still inherit an established layout;
            // that is the one-row continuation case this repair exists for.
            if !bands[index].gutters.is_empty() {
                if bands[index].gutters.len() != neighbor.gutters.len() {
                    continue;
                }
                let gutter_tolerance = (page_x_span * 0.04).clamp(12.0, 24.0);
                let gutters_close = bands[index]
                    .gutters
                    .iter()
                    .zip(neighbor.gutters.iter())
                    .all(|(edge, adjacent)| (edge - adjacent).abs() <= gutter_tolerance);
                let ownership_unchanged = band_chars
                    .iter()
                    .filter(|ch| !ch.ch.is_whitespace())
                    .all(|ch| {
                        let edge_owner = bands[index]
                            .gutters
                            .iter()
                            .filter(|&&gutter| ch.x >= gutter)
                            .count();
                        let adjacent_owner = neighbor
                            .gutters
                            .iter()
                            .filter(|&&gutter| ch.x >= gutter)
                            .count();
                        edge_owner == adjacent_owner
                    });
                let source_continuation_proves_inheritance =
                    if bands[index].gutters.len() == 1 && neighbor.gutters.len() == 1 {
                        let neighbor_chars = chars
                            .iter()
                            .copied()
                            .filter(|ch| ch.y <= neighbor.y_top && ch.y > neighbor.y_bottom)
                            .collect::<Vec<_>>();
                        sparse_edge_ownership_changes_are_left_continuations(
                            &band_chars,
                            bands[index].gutters[0],
                            neighbor.gutters[0],
                            &neighbor_chars,
                        )
                    } else {
                        false
                    };
                if !(gutters_close && ownership_unchanged)
                    && !source_continuation_proves_inheritance
                {
                    continue;
                }
            }
            let Some(&split_x) = neighbor.gutters.first() else {
                continue;
            };
            let looks_like_banner =
                band_looks_like_full_width_banner(&band_chars, split_x, x_min_global, x_max_global);
            if !looks_like_banner {
                bands[index].gutters = neighbor.gutters.clone();
            }
        }
    }

    // A recursive histogram split can create a fake middle column when a
    // long line continues a few points past the first candidate gutter. Only
    // collapse that extra gutter when repeated middle rows begin immediately
    // at the gutter; genuine three-column transitions have a real inset
    // middle-column start and remain untouched.
    if bands.len() >= 2 {
        for index in 0..bands.len() {
            if bands[index].gutters.len() != 2 {
                continue;
            }
            let neighbor_index = if index > 0 { index - 1 } else { 1 };
            let neighbor = &bands[neighbor_index];
            if neighbor.gutters.len() != 1 {
                continue;
            }
            let Some(&neighbor_split) = neighbor.gutters.first() else {
                continue;
            };
            let first_split = bands[index].gutters[0];
            let second_split = bands[index].gutters[1];
            if (second_split - neighbor_split).abs() > page_x_span * 0.08 {
                continue;
            }
            let band_chars = chars
                .iter()
                .copied()
                .filter(|ch| ch.y <= bands[index].y_top && ch.y > bands[index].y_bottom)
                .collect::<Vec<_>>();
            let rows = build_raw_rows(&band_chars, estimate_y_tolerance(&band_chars));
            if middle_bucket_is_gutter_continuation(&rows, first_split, second_split) {
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

/// Build analysis bands whose boundaries fall between physical text rows.
///
/// The previous fixed 56-88pt grid was stable, but a layout transition could
/// land a few points either side of an arbitrary grid boundary. That mixed a
/// full-width heading with the first two-column rows, or split the final row
/// of an option bank into a different layout. Keep the same approximate band
/// size while snapping every cut to the strongest nearby inter-row gap.
fn adaptive_horizontal_band_ranges(
    chars: &[CharWithOrigin],
    y_min: f32,
    y_max: f32,
) -> Vec<(f32, f32)> {
    let y_span = (y_max - y_min).max(1.0);
    let target_height = (y_span / 10.0).clamp(56.0, 88.0);
    let rows = build_raw_rows(chars, estimate_y_tolerance(chars));
    let row_centers = rows
        .iter()
        .map(|row| row.iter().map(|ch| ch.y).sum::<f32>() / row.len().max(1) as f32)
        .collect::<Vec<_>>();
    if row_centers.len() < 3 {
        return vec![(y_max, y_min - 0.5)];
    }

    let gaps = row_centers
        .windows(2)
        .map(|pair| {
            let top = pair[0];
            let bottom = pair[1];
            ((top + bottom) * 0.5, (top - bottom).abs())
        })
        .collect::<Vec<_>>();
    let mut cuts: Vec<f32> = Vec::new();
    let mut band_top = y_max;
    while band_top - target_height > y_min {
        let target = band_top - target_height;
        let min_cut = band_top - target_height * 1.4;
        let max_cut = band_top - target_height * 0.6;
        let cut = gaps
            .iter()
            .copied()
            .filter(|(midpoint, _)| *midpoint >= min_cut && *midpoint <= max_cut)
            .max_by(|(left_mid, left_gap), (right_mid, right_gap)| {
                let left_score = *left_gap - (*left_mid - target).abs() * 0.12;
                let right_score = *right_gap - (*right_mid - target).abs() * 0.12;
                left_score.total_cmp(&right_score)
            })
            .map(|(midpoint, _)| midpoint)
            .unwrap_or(target);
        if band_top - cut < 1.0 || cuts.last().is_some_and(|last| (*last - cut).abs() < 1.0) {
            break;
        }
        cuts.push(cut);
        band_top = cut;
    }

    let mut ranges = Vec::with_capacity(cuts.len() + 1);
    let mut top = y_max;
    for cut in cuts {
        ranges.push((top, cut));
        top = cut;
    }
    ranges.push((top, y_min - 0.5));
    ranges
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
            // Keep row boundaries while assigning characters so a narrow
            // gutter cannot strand the tail of a word in the next column.
            // This only changes text ownership for the conservative lowercase
            // prefix pattern handled by `repair_cross_column_word_prefix`.
            let segment_rows = build_raw_rows(&segment_chars, y_tol);
            let dominant_right_edge = if column_count == 2 {
                dominant_right_column_edge(&segment_rows, section.split_x())
            } else {
                None
            };
            for mut row in segment_rows {
                if column_count == 2 {
                    let split_x = section.split_x();
                    let mut left = row
                        .iter()
                        .copied()
                        .filter(|ch| ch.x < split_x)
                        .collect::<Vec<_>>();
                    let mut right = row
                        .iter()
                        .copied()
                        .filter(|ch| ch.x >= split_x)
                        .collect::<Vec<_>>();
                    repair_cross_column_word_prefix(&mut left, &mut right, split_x);
                    if let Some(dominant_edge) = dominant_right_edge {
                        repair_cross_column_prefix_before_dominant_edge(
                            &mut left,
                            &mut right,
                            split_x,
                            dominant_edge,
                        );
                    }
                    columns[0].extend(left);
                    columns[1].extend(right);
                } else {
                    for ch in row.drain(..) {
                        let column_index = section
                            .gutters
                            .iter()
                            .filter(|gutter| ch.x >= **gutter)
                            .count() as usize;
                        let bucket = column_index.min(columns.len() - 1);
                        columns[bucket].push(ch);
                    }
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
        result.push((
            canonicalize_extracted_line_text(&words.join(" ")),
            line_bbox(&line, y_tol),
        ));
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

/// Finds raster-backed diagram/plan/map tasks for which the PDF text layer
/// contains the declared question range and instructions, but not the labels
/// drawn into the image. The candidate is deliberately strict: without both
/// an explicit range heading and a nearby `Label the ... below` instruction,
/// no visual question region is manufactured.
fn diagram_question_region_candidates(pages: &[Value]) -> Vec<DiagramQuestionRegionCandidate> {
    let mut candidates = Vec::new();
    for page in pages {
        let page_index = page.get("pageIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
        let page_width = page.get("width").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let page_height = page.get("height").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let Some(blocks) = page.get("blocks").and_then(Value::as_array) else {
            continue;
        };
        if page_index == 0 || page_width <= 72.0 || page_height <= 144.0 {
            continue;
        }

        for (heading_index, heading) in blocks.iter().enumerate() {
            let heading_text = heading
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some((question_start, question_end)) = strict_questions_range(heading_text) else {
                continue;
            };

            let mut label_index = None;
            let mut instruction_end = heading_index;
            for (index, block) in blocks.iter().enumerate().skip(heading_index + 1).take(7) {
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if strict_questions_range(text).is_some() {
                    break;
                }
                let normalized = normalized_instruction_text(text);
                if is_diagram_label_instruction(&normalized) {
                    label_index = Some(index);
                    instruction_end = index;
                    continue;
                }
                if label_index.is_some() && is_diagram_instruction_tail(&normalized) {
                    instruction_end = index;
                    continue;
                }
                if label_index.is_some() && !normalized.is_empty() {
                    break;
                }
            }
            let Some(_label_index) = label_index else {
                continue;
            };

            let instruction_bottom = blocks[instruction_end]
                .get("bbox")
                .and_then(json_bbox)
                .map(|bbox| bbox[1]);
            let Some(instruction_bottom) = instruction_bottom else {
                continue;
            };

            // If another declared question group begins on this page, stop the
            // crop above it. This prevents a diagram region from swallowing a
            // later question or an unrelated lower-page passage section.
            let next_heading = blocks
                .iter()
                .enumerate()
                .skip(instruction_end + 1)
                .find_map(|(index, block)| {
                    let text = block.get("text").and_then(Value::as_str)?;
                    strict_questions_range(text)?;
                    json_bbox(block.get("bbox")?).map(|bbox| (index, bbox[3]))
                });
            let region_end = next_heading.map(|(index, _)| index).unwrap_or(blocks.len());
            let native_region_blocks = &blocks[instruction_end + 1..region_end];
            let native_number_closure = (question_start..=question_end).all(|number| {
                native_region_blocks.iter().any(|block| {
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| text_contains_standalone_number(text, number))
                })
            });
            if native_number_closure {
                // The source text layer already exposes every declared slot.
                // Do not add an OCR-required asset or downgrade a recoverable
                // vector/text diagram merely because it also has visuals.
                continue;
            }

            let x0 = 36.0_f32;
            let x1 = (page_width - 36.0).max(x0);
            let y1 = (instruction_bottom - 10.0).min(page_height - 1.0);
            let y0 = next_heading
                .map(|(_, value)| value + 10.0)
                .unwrap_or(36.0)
                .max(0.0);
            if x1 - x0 < 100.0 || y1 - y0 < 72.0 {
                continue;
            }
            candidates.push(DiagramQuestionRegionCandidate {
                page_index,
                insert_after: instruction_end,
                question_start,
                question_end,
                bbox: [x0, y0, x1, y1],
            });
        }
    }
    candidates
}

fn normalized_instruction_text(text: &str) -> String {
    text.to_lowercase()
        .replace(['\u{2013}', '\u{2014}'], "-")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_diagram_label_instruction(normalized: &str) -> bool {
    let starts_with_label = normalized.starts_with("label the ");
    let visual_kind =
        normalized.contains("diagram") || normalized.contains("plan") || normalized.contains("map");
    starts_with_label && visual_kind && normalized.contains("below")
}

fn is_diagram_instruction_tail(normalized: &str) -> bool {
    normalized.starts_with("choose ")
        || normalized.starts_with("write ")
        || normalized.starts_with("use ")
        || normalized.contains("word only")
        || normalized.contains("words from the passage")
        || normalized.contains("answer sheet")
}

fn strict_questions_range(text: &str) -> Option<(u32, u32)> {
    let normalized = normalized_instruction_text(text);
    let suffix = normalized
        .strip_prefix("questions ")
        .or_else(|| normalized.strip_prefix("question "))?;
    let numeric = suffix
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '-' || ch.is_whitespace())
        .collect::<String>();
    let mut parts = numeric.split('-');
    let start = parts.next()?.trim().parse::<u32>().ok()?;
    let end = parts.next()?.trim().parse::<u32>().ok()?;
    if parts.next().is_some() || start == 0 || end < start || end - start > 40 {
        return None;
    }
    Some((start, end))
}

fn text_contains_standalone_number(text: &str, number: u32) -> bool {
    let digits = number.to_string();
    text.match_indices(&digits).any(|(start, matched)| {
        let end = start + matched.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !ch.is_ascii_digit());
        let after_ok = text[end..]
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_ascii_digit());
        before_ok && after_ok
    })
}

fn json_bbox(value: &Value) -> Option<[f32; 4]> {
    let values = value.as_array()?;
    if values.len() != 4 {
        return None;
    }
    Some([
        values[0].as_f64()? as f32,
        values[1].as_f64()? as f32,
        values[2].as_f64()? as f32,
        values[3].as_f64()? as f32,
    ])
}

fn recover_diagram_question_region_assets(
    document: &PdfDocument<'_>,
    pages: &mut [Value],
    asset_dir: &Path,
    block_counter: &mut usize,
    warnings: &mut Vec<String>,
) -> CommandResult<Vec<Value>> {
    let mut candidates = diagram_question_region_candidates(pages);
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    // Insert lower regions first so an earlier insertion on the same page
    // cannot shift the source index recorded for a later diagram task.
    candidates.sort_by(|left, right| {
        left.page_index
            .cmp(&right.page_index)
            .then(right.insert_after.cmp(&left.insert_after))
    });
    fs::create_dir_all(asset_dir).map_err(|error| {
        format!(
            "create_diagram_question_region_asset_dir:{}:{}",
            asset_dir.display(),
            error
        )
    })?;

    let mut assets = Vec::new();
    for candidate in candidates {
        let page = document
            .pages()
            .get((candidate.page_index - 1) as PdfPageIndex)
            .map_err(|error| {
                format!(
                    "diagram_question_region_page_{}_failed:{}",
                    candidate.page_index, error
                )
            })?;
        let bitmap = page
            .render_with_config(
                &PdfRenderConfig::new()
                    .set_target_width(2000)
                    .set_maximum_height(2800),
            )
            .map_err(|error| {
                format!(
                    "diagram_question_region_render_page_{}_failed:{}",
                    candidate.page_index, error
                )
            })?;
        let full_width = bitmap.width() as u32;
        let full_height = bitmap.height() as u32;
        let page_width = page.width().value;
        let page_height = page.height().value;
        let (rgba, crop_width, crop_height) = crop_pdf_bbox_from_rgba(
            &bitmap.as_rgba_bytes(),
            full_width,
            full_height,
            page_width,
            page_height,
            candidate.bbox,
        )?;

        let asset_id = format!(
            "diagram-question-region-p{:03}-q{}-{}",
            candidate.page_index, candidate.question_start, candidate.question_end
        );
        let file_name = format!("{}.png", asset_id);
        let asset_path = asset_dir.join(&file_name);
        write_rgba_png(&asset_path, crop_width, crop_height, &rgba)?;
        let bytes = fs::read(&asset_path).map_err(|error| {
            format!(
                "read_diagram_question_region_asset:{}:{}",
                asset_path.display(),
                error
            )
        })?;
        let expected_numbers =
            (candidate.question_start..=candidate.question_end).collect::<Vec<_>>();
        assets.push(json!({
            "assetId": asset_id,
            "type": "image",
            "sourceKind": "page_crop",
            "pageIndex": candidate.page_index,
            "path": asset_path.to_string_lossy(),
            "fileName": file_name,
            "mimeType": "image/png",
            "width": crop_width,
            "height": crop_height,
            "bbox": candidate.bbox,
            "sha256": crate::hash_bytes(&bytes),
            "sizeBytes": bytes.len() as u64,
            "diagramQuestionRegion": {
                "questionRange": [candidate.question_start, candidate.question_end],
                "expectedNumbers": expected_numbers,
                "recoveryStatus": "ocr_required",
                "numberClosure": false,
                "sourceBacked": true
            }
        }));

        let block = json!({
            "blockId": format!("b{:03}", *block_counter),
            "blockType": "image",
            "text": "",
            "html": "",
            "bbox": candidate.bbox,
            "confidence": 1.0,
            "pageIndex": candidate.page_index,
            "roleHint": "question",
            "assetId": asset_id,
            "layoutHints": {
                "diagramQuestionRegion": {
                    "questionRange": [candidate.question_start, candidate.question_end],
                    "expectedNumbers": expected_numbers,
                    "regionBbox": candidate.bbox,
                    "recoveryStatus": "ocr_required",
                    "numberClosure": false,
                    "sourceBacked": true
                }
            }
        });
        *block_counter += 1;
        if let Some(blocks) = pages
            .iter_mut()
            .find(|page| {
                page.get("pageIndex").and_then(Value::as_u64) == Some(candidate.page_index as u64)
            })
            .and_then(|page| page.get_mut("blocks"))
            .and_then(Value::as_array_mut)
        {
            let insertion = (candidate.insert_after + 1).min(blocks.len());
            blocks.insert(insertion, block);
            for (ordinal, block) in blocks.iter_mut().enumerate() {
                if let Some(obj) = block.as_object_mut() {
                    obj.insert("_epic8Ordinal".to_string(), json!(ordinal));
                }
            }
        }
        warnings.push(format!(
            "DIAGRAM_QUESTION_REGION_OCR_REQUIRED:page={}:questions={}-{}:assetId={}",
            candidate.page_index, candidate.question_start, candidate.question_end, asset_id
        ));
    }
    Ok(assets)
}

fn crop_pdf_bbox_from_rgba(
    rgba: &[u8],
    full_width: u32,
    full_height: u32,
    page_width: f32,
    page_height: f32,
    bbox: [f32; 4],
) -> CommandResult<(Vec<u8>, u32, u32)> {
    if full_width == 0 || full_height == 0 || page_width <= 0.0 || page_height <= 0.0 {
        return Err("diagram_question_region_invalid_page_geometry".to_string());
    }
    let x0 = ((bbox[0] / page_width) * full_width as f32)
        .floor()
        .clamp(0.0, full_width.saturating_sub(1) as f32) as u32;
    let x1 = ((bbox[2] / page_width) * full_width as f32)
        .ceil()
        .clamp((x0 + 1) as f32, full_width as f32) as u32;
    let top = (((page_height - bbox[3]) / page_height) * full_height as f32)
        .floor()
        .clamp(0.0, full_height.saturating_sub(1) as f32) as u32;
    let bottom = (((page_height - bbox[1]) / page_height) * full_height as f32)
        .ceil()
        .clamp((top + 1) as f32, full_height as f32) as u32;
    let crop_width = x1 - x0;
    let crop_height = bottom - top;
    let stride = full_width as usize * 4;
    if rgba.len() < stride * full_height as usize {
        return Err("diagram_question_region_rgba_buffer_too_short".to_string());
    }
    let mut cropped = Vec::with_capacity(crop_width as usize * crop_height as usize * 4);
    for row in top..bottom {
        let start = row as usize * stride + x0 as usize * 4;
        let end = start + crop_width as usize * 4;
        cropped.extend_from_slice(&rgba[start..end]);
    }
    Ok((cropped, crop_width, crop_height))
}

fn write_rgba_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> CommandResult<()> {
    let file = fs::File::create(path).map_err(|error| {
        format!(
            "create_diagram_question_region_png:{}:{}",
            path.display(),
            error
        )
    })?;
    let buffered = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(buffered, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("diagram_question_region_png_header:{}", error))?;
    writer
        .write_image_data(rgba)
        .map_err(|error| format!("diagram_question_region_png_write:{}", error))
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
    let _pdfium_guard = pdfium_instance()?;
    let pdfium = _pdfium_guard.as_ref().map_err(|error| error.clone())?;
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

    fn push_fixed_glyph_line(
        chars: &mut Vec<CharWithOrigin>,
        prefix: &str,
        fill: char,
        glyph_count: usize,
        x_start: f32,
        y: f32,
    ) {
        assert!(prefix.len() <= glyph_count);
        let text = format!(
            "{}{}",
            prefix,
            std::iter::repeat_n(fill, glyph_count - prefix.len()).collect::<String>()
        );
        push_text_line(chars, &text, x_start, y);
    }

    #[test]
    fn pdfium_library_path_does_not_panic() {
        // Resolution must be safe to call even when no library is bundled.
        let _ = pdfium_library_path();
    }

    fn diagram_test_block(id: &str, text: &str, y0: f64, y1: f64) -> Value {
        json!({
            "blockId": id,
            "blockType": "paragraph",
            "text": text,
            "bbox": [54.0, y0, 520.0, y1],
            "pageIndex": 3
        })
    }

    #[test]
    fn detects_roller_and_elephant_raster_diagram_question_regions() {
        let pages = vec![json!({
            "pageIndex": 3,
            "width": 595.2,
            "height": 841.92,
            "blocks": [
                diagram_test_block("b028", "Questions 14–16", 756.1, 759.2),
                diagram_test_block("b029", "Label the diagram below.", 726.6, 729.7),
                diagram_test_block("b030", "Choose ONE WORD ONLY from the passage for each answer.", 696.8, 699.9),
                diagram_test_block("b031", "Write your answers in boxes 14–16 on your answer sheet.", 667.3, 670.4),
                diagram_test_block("b040", "Questions 28-31", 320.0, 323.0),
                diagram_test_block("b041", "Label the diagram below.", 290.0, 293.0),
                diagram_test_block("b042", "Choose NO MORE THAN TWO WORDS from the passage for each answer.", 260.0, 263.0),
                diagram_test_block("b043", "Write your answers in boxes 28-31 on your answer sheet.", 230.0, 233.0)
            ]
        })];

        let candidates = diagram_question_region_candidates(&pages);
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            (candidates[0].question_start, candidates[0].question_end),
            (14, 16)
        );
        assert_eq!(
            (candidates[1].question_start, candidates[1].question_end),
            (28, 31)
        );
        assert!(
            candidates[0].bbox[1] > candidates[1].bbox[3],
            "the first crop must stop above the next declared question group"
        );
        assert_eq!(
            candidates[1].bbox[1], 36.0,
            "the final diagram crop should extend to the safe page margin"
        );
    }

    #[test]
    fn diagram_region_detection_fails_closed_without_strict_range_and_label_pair() {
        let pages = vec![json!({
            "pageIndex": 1,
            "width": 595.2,
            "height": 841.92,
            "blocks": [
                diagram_test_block("b001", "The questions raised by this diagram are important.", 756.0, 760.0),
                diagram_test_block("b002", "Label the diagram below.", 726.0, 730.0),
                diagram_test_block("b003", "Questions 10-12", 696.0, 700.0),
                diagram_test_block("b004", "Complete the notes below.", 666.0, 670.0),
                diagram_test_block("b005", "A map below the valley illustrates the route.", 636.0, 640.0)
            ]
        })];
        assert!(diagram_question_region_candidates(&pages).is_empty());
        assert_eq!(strict_questions_range("Questions 14–16"), Some((14, 16)));
        assert_eq!(strict_questions_range("Questions 28-31"), Some((28, 31)));
        assert_eq!(strict_questions_range("Questions in the passage"), None);
    }

    #[test]
    fn diagram_region_detection_does_not_downgrade_complete_native_slot_text() {
        let pages = vec![json!({
            "pageIndex": 2,
            "width": 595.2,
            "height": 841.92,
            "blocks": [
                diagram_test_block("b001", "Questions 14-16", 756.0, 760.0),
                diagram_test_block("b002", "Label the diagram below.", 726.0, 730.0),
                diagram_test_block("b003", "Choose ONE WORD ONLY from the passage for each answer.", 696.0, 700.0),
                diagram_test_block("b004", "Write your answers in boxes 14-16 on your answer sheet.", 666.0, 670.0),
                diagram_test_block("b005", "14 ______ upper bearing", 540.0, 544.0),
                diagram_test_block("b006", "15 ______ lower bearing", 470.0, 474.0),
                diagram_test_block("b007", "16 ______ drive housing", 400.0, 404.0)
            ]
        })];

        assert!(diagram_question_region_candidates(&pages).is_empty());
        assert!(text_contains_standalone_number("slot 16 ______", 16));
        assert!(!text_contains_standalone_number("year 2016", 16));
    }

    #[test]
    fn crop_pdf_bbox_maps_bottom_left_pdf_coordinates_to_top_left_pixels() {
        // Four 2x2 RGBA pixels: red/green on the top row, blue/white below.
        let rgba = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let (top_right, width, height) =
            crop_pdf_bbox_from_rgba(&rgba, 2, 2, 100.0, 100.0, [50.0, 50.0, 100.0, 100.0])
                .expect("valid crop");
        assert_eq!((width, height), (1, 1));
        assert_eq!(top_right, vec![0, 255, 0, 255]);
    }

    #[test]
    fn real_raster_diagram_fixtures_emit_source_backed_fail_closed_regions() {
        let fixture_root =
            Path::new(r"C:\Users\lenovo\Desktop\working space\0.3.1 working\ReadingPractice\PDF");
        let fixtures = [
            ("46. P2 - Roller coaster 过山车.pdf", 14_u64, 16_u64),
            (
                "66. P3 - Elephant Communication 大象交流.pdf",
                28_u64,
                31_u64,
            ),
        ];
        if fixtures
            .iter()
            .any(|(name, _, _)| !fixture_root.join(name).exists())
        {
            return;
        }

        let artifact_root = std::env::temp_dir().join("epic8-diagram-region-real-fixtures");
        fs::create_dir_all(&artifact_root).expect("create diagram artifact directory");
        for (name, expected_start, expected_end) in fixtures {
            let input = fixture_root.join(name);
            let output = artifact_root.join(format!("{}-document-ir.json", expected_start));
            let mut job = crate::job_store::make_job(crate::CreateJobInput {
                title: Some(name.to_string()),
                ..Default::default()
            });
            let source = SourceFile {
                file_id: format!("diagram-fixture-{}", expected_start),
                original_name: name.to_string(),
                stored_name: name.to_string(),
                file_type: "pdf".to_string(),
                sha256: "fixture".to_string(),
                size_bytes: fs::metadata(&input).map(|meta| meta.len()).unwrap_or(0),
                role: "MainQuestion".to_string(),
                imported_at: chrono::Utc::now(),
            };
            job.source_files = vec![source.clone()];

            let ir = parse_pdf_with_pdfium(&job, &source, &input, &output, "auto")
                .expect("real raster diagram fixture should parse");
            let assets = ir
                .get("assets")
                .and_then(Value::as_array)
                .expect("assets array");
            let asset = assets
                .iter()
                .find(|asset| {
                    asset
                        .pointer("/diagramQuestionRegion/questionRange/0")
                        .and_then(Value::as_u64)
                        == Some(expected_start)
                })
                .expect("declared diagram question region asset");
            assert_eq!(
                asset
                    .pointer("/diagramQuestionRegion/questionRange/1")
                    .and_then(Value::as_u64),
                Some(expected_end)
            );
            assert_eq!(
                asset
                    .pointer("/diagramQuestionRegion/recoveryStatus")
                    .and_then(Value::as_str),
                Some("ocr_required")
            );
            assert_eq!(
                asset
                    .pointer("/diagramQuestionRegion/numberClosure")
                    .and_then(Value::as_bool),
                Some(false)
            );
            let asset_path = asset
                .get("path")
                .and_then(Value::as_str)
                .map(Path::new)
                .expect("asset path");
            assert!(asset_path.exists(), "rendered region must persist on disk");
            assert!(
                fs::metadata(asset_path)
                    .map(|meta| meta.len() > 10_000)
                    .unwrap_or(false),
                "rendered region should contain substantial source pixels"
            );
            let image_block = ir
                .get("pages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|page| {
                    page.get("blocks")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .find(|block| {
                    block.get("assetId").and_then(Value::as_str)
                        == asset.get("assetId").and_then(Value::as_str)
                })
                .expect("source-backed image block");
            assert_eq!(image_block.get("text").and_then(Value::as_str), Some(""));

            let shadow_dir = artifact_root.join(format!("q{}-shadow", expected_start));
            fs::create_dir_all(&shadow_dir).expect("create V2 shadow fixture directory");
            let shadow_output = shadow_dir.join(crate::pdf_facts_shadow::SHADOW_ARTIFACT_FILE);
            let shadow = crate::pdf_facts_shadow::write_pdf_facts_shadow_with_v1(
                &job,
                &source,
                &input,
                &shadow_output,
                Some(&ir),
            )
            .expect("V1 region asset should bridge into the physical shadow");
            let promoted = shadow
                .get("assets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|item| item.get("assetId") == asset.get("assetId"))
                .expect("canonical V2 page_crop asset");
            assert_eq!(
                promoted.get("kind").and_then(Value::as_str),
                Some("page_crop")
            );
            assert_eq!(
                promoted.get("extractionMode").and_then(Value::as_str),
                Some("page_crop")
            );
            serde_json::from_value::<crate::schema::DocumentIRV2>(shadow.clone())
                .expect("promoted diagram asset must preserve the typed DocumentIRV2 contract");
            let split = crate::authoring_pipeline::make_dynamic_split_candidates(
                &job.job_id,
                &job,
                Some(&ir),
            );
            let v1_authoring =
                crate::authoring_pipeline::make_dynamic_authoring_ir(&job, &split, Some(&ir));
            let v2_authoring = crate::ielts_grammar::build_authoring_v2_shadow(
                &job,
                &v1_authoring,
                &split,
                Some(&ir),
                Some(&shadow),
            )
            .expect("product grammar path should consume the promoted physical asset");
            assert!(v2_authoring
                .get("assets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|item| item.get("assetId") == asset.get("assetId")));
            assert!(v2_authoring
                .pointer("/quality/hardFailures")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|failure| {
                    failure.as_str() == Some("DIAGRAM_QUESTION_REGION_OCR_REQUIRED")
                }));
            let relative_path = promoted
                .get("relativePath")
                .and_then(Value::as_str)
                .expect("V2 relative path");
            assert!(
                shadow_dir
                    .join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR))
                    .exists(),
                "promoted V2 asset must be committed beside the product shadow artifact"
            );
        }
        eprintln!(
            "diagram question-region artifacts preserved at {}",
            artifact_root.display()
        );
    }

    #[test]
    fn extracted_line_text_normalizes_rotation_spacing_without_joining_ranges() {
        assert_eq!(
            canonicalize_extracted_line_text("14 High - level workers"),
            "14 High-level workers"
        );
        assert_eq!(
            canonicalize_extracted_line_text("A taking a break . B resting ."),
            "A taking a break. B resting."
        );
        assert_eq!(
            canonicalize_extracted_line_text("23 ________ . This absence"),
            "23 ________. This absence"
        );
        assert_eq!(canonicalize_extracted_line_text("A - B"), "A - B");
    }

    #[test]
    fn pdfium_parse_yields_real_bbox_when_library_present() {
        let sample =
            Path::new(r"D:\xwechat_files\wxid_zg93z3d7b4aq21_8fcc\msg\file\2026-06\PDF(1).pdf");
        if !sample.exists() {
            return; // sample or pdfium library not available in this environment
        }
        let guard = match pdfium_instance() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let pdfium = match guard.as_ref() {
            Ok(pdfium) => pdfium,
            Err(_) => return,
        };
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
    fn adaptive_bands_snap_to_large_inter_row_layout_gap() {
        let mut chars = Vec::new();
        for (index, y) in [760.0, 748.0, 736.0, 724.0, 712.0, 660.0, 648.0, 636.0]
            .into_iter()
            .enumerate()
        {
            push_text_line(&mut chars, &format!("ROW {index} HAS ENOUGH TEXT"), 72.0, y);
        }
        // Extend the page span so the nominal first cut is around y=704; the
        // adaptive cut should instead use the strong 712->660 transition.
        push_text_line(&mut chars, "LOWER PAGE TEXT", 72.0, 460.0);
        let ranges = adaptive_horizontal_band_ranges(&chars, 460.0, 760.0);
        assert!(
            ranges
                .iter()
                .any(|(_, bottom)| (*bottom - 686.0).abs() < 1.0),
            "expected a cut in the large inter-row gap, got {ranges:?}"
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

    #[test]
    fn build_blocks_splits_repeated_same_baseline_columns_with_narrow_gutter() {
        // Two dense columns share every baseline, like a PDF text layer that
        // emits the left and right question columns on the same physical row.
        // The gap between the final left glyph origin (x=282.4) and first
        // right glyph origin (x=300.4) is only 18pt. The old page-span-based
        // minimum gutter (>=10%, about 53pt here) merged each pair of rows.
        let mut chars = Vec::new();
        for (index, y) in [760.0, 749.0, 738.0, 727.0, 716.0, 705.0]
            .iter()
            .enumerate()
        {
            push_fixed_glyph_line(
                &mut chars,
                &format!("LEFT{:02}", index + 1),
                'L',
                54,
                28.0,
                *y,
            );
            push_fixed_glyph_line(
                &mut chars,
                &format!("RIGHT{:02}", index + 1),
                'R',
                54,
                300.4,
                *y,
            );
        }

        let x_min = chars.iter().map(|c| c.x).fold(f32::MAX, f32::min);
        let x_max = chars.iter().map(|c| c.x).fold(f32::MIN, f32::max);
        let gutters = detect_column_gutters(&chars, x_min, x_max);
        assert_eq!(
            gutters.len(),
            1,
            "the repeated 18pt gap should be accepted as one gutter: {:?}",
            gutters
        );
        assert!(
            gutters[0] > 282.4 && gutters[0] < 300.4,
            "gutter should lie between the two columns, got {}",
            gutters[0]
        );

        let blocks = build_blocks_from_chars(&chars);
        let left = blocks
            .iter()
            .find(|block| block.column_count == 2 && block.column_index == 0)
            .expect("left narrow-gutter column should be emitted separately");
        let right = blocks
            .iter()
            .find(|block| block.column_count == 2 && block.column_index == 1)
            .expect("right narrow-gutter column should be emitted separately");
        assert!(left.text.contains("LEFT01") && !left.text.contains("RIGHT01"));
        assert!(right.text.contains("RIGHT01") && !right.text.contains("LEFT01"));
        assert!(
            blocks
                .iter()
                .all(|block| { !(block.text.contains("LEFT") && block.text.contains("RIGHT")) }),
            "no emitted block may merge text from both columns: {:?}",
            blocks
                .iter()
                .map(|block| (&block.text, block.column_index, block.column_count))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn repair_cross_column_word_prefix_moves_only_lowercase_word_tails() {
        let mut left = vec![CharWithOrigin {
            ch: 'a',
            x: 290.0,
            y: 700.0,
        }];
        let mut right = vec![
            CharWithOrigin {
                ch: 'n',
                x: 301.0,
                y: 700.0,
            },
            CharWithOrigin {
                ch: 'C',
                x: 309.0,
                y: 700.0,
            },
        ];
        repair_cross_column_word_prefix(&mut left, &mut right, 300.0);
        assert_eq!(left.iter().map(|ch| ch.ch).collect::<String>(), "an");
        assert_eq!(right.iter().map(|ch| ch.ch).collect::<String>(), "C");

        let mut left = vec![
            CharWithOrigin {
                ch: 'N',
                x: 285.0,
                y: 680.0,
            },
            CharWithOrigin {
                ch: 'e',
                x: 290.0,
                y: 680.0,
            },
        ];
        let mut right = vec![
            CharWithOrigin {
                ch: 'i',
                x: 301.0,
                y: 680.0,
            },
            CharWithOrigin {
                ch: 'l',
                x: 306.0,
                y: 680.0,
            },
        ];
        repair_cross_column_word_prefix(&mut left, &mut right, 300.0);
        assert_eq!(left.iter().map(|ch| ch.ch).collect::<String>(), "Ne");
        assert_eq!(right.iter().map(|ch| ch.ch).collect::<String>(), "il");

        let mut left = vec![CharWithOrigin {
            ch: 't',
            x: 290.0,
            y: 660.0,
        }];
        let mut right = vec![CharWithOrigin {
            ch: 'A',
            x: 301.0,
            y: 660.0,
        }];
        repair_cross_column_word_prefix(&mut left, &mut right, 300.0);
        assert_eq!(left.iter().map(|ch| ch.ch).collect::<String>(), "t");
        assert_eq!(right.iter().map(|ch| ch.ch).collect::<String>(), "A");
    }

    #[test]
    fn banner_detection_keeps_independent_column_word_tail_in_column_flow() {
        let mut line = Vec::new();
        for index in 0..55 {
            let ch = match index {
                53 => 'N',
                54 => 'e',
                _ => 'x',
            };
            line.push(CharWithOrigin {
                ch,
                x: 28.0 + index as f32 * 4.8,
                y: 700.0,
            });
        }
        line.extend([
            CharWithOrigin {
                ch: 'i',
                x: 301.0,
                y: 700.0,
            },
            CharWithOrigin {
                ch: 'l',
                x: 306.0,
                y: 700.0,
            },
        ]);

        assert!(
            !is_full_width_banner_row(&line, 300.0, 28.0, 520.0),
            "a page-edge left line plus a split-adjacent lowercase tail is a two-column row"
        );
    }

    #[test]
    fn middle_bucket_continuation_distinguishes_false_three_column_band() {
        let continuation_rows = (0..3)
            .map(|row_index| {
                let mut row = Vec::new();
                for index in 0..8 {
                    row.push(CharWithOrigin {
                        ch: 'l',
                        // The left fragment ends one normal glyph advance
                        // before the would-be middle bucket. This is a word
                        // continuation, not an independent column boundary.
                        x: 126.4 + index as f32 * 4.8,
                        y: 700.0 - row_index as f32 * 11.0,
                    });
                    row.push(CharWithOrigin {
                        ch: 'm',
                        x: 164.8 + index as f32 * 4.8,
                        y: 700.0 - row_index as f32 * 11.0,
                    });
                    row.push(CharWithOrigin {
                        ch: 'r',
                        x: 306.6 + index as f32 * 4.8,
                        y: 700.0 - row_index as f32 * 11.0,
                    });
                }
                row.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
                row
            })
            .collect::<Vec<_>>();
        assert!(middle_bucket_is_gutter_continuation(
            &continuation_rows,
            164.0,
            264.0
        ));

        // A compact genuine middle column can also start immediately after
        // first_split. What distinguishes it is the stable typographic gutter
        // before that start, not an arbitrary minimum inset from the split.
        let compact_real_middle_rows = continuation_rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|ch| CharWithOrigin {
                        x: if ch.x < 164.0 { ch.x - 12.0 } else { ch.x },
                        ..*ch
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert!(!middle_bucket_is_gutter_continuation(
            &compact_real_middle_rows,
            164.0,
            264.0
        ));
    }

    #[test]
    fn sparse_internal_band_inherits_matching_two_column_gutter() {
        let mut chars = Vec::new();
        for (index, y) in [760.0, 749.0, 738.0, 727.0].iter().enumerate() {
            push_fixed_glyph_line(&mut chars, &format!("TOPLEFT{index}"), 'L', 34, 60.0, *y);
            push_fixed_glyph_line(&mut chars, &format!("TOPRIGHT{index}"), 'R', 34, 340.0, *y);
        }
        // This one-row continuation occupies only the left column and cannot
        // prove a gutter by itself. It sits in its own fixed-height band.
        push_text_line(&mut chars, "SPARSE LEFT CONTINUATION", 60.0, 680.0);
        for (index, y) in [635.0, 624.0, 613.0, 602.0].iter().enumerate() {
            push_fixed_glyph_line(&mut chars, &format!("BOTTOMLEFT{index}"), 'L', 34, 60.0, *y);
            push_fixed_glyph_line(
                &mut chars,
                &format!("BOTTOMRIGHT{index}"),
                'R',
                34,
                340.0,
                *y,
            );
        }

        let sections = detect_layout_sections(&chars);
        assert_eq!(
            sections.len(),
            1,
            "the sparse internal band must not fracture one two-column flow: {sections:?}"
        );
        assert_eq!(sections[0].column_count, 2);

        let blocks = build_blocks_from_chars(&chars);
        let sparse = blocks
            .iter()
            .find(|block| block.text.contains("SPARSE LEFT CONTINUATION"))
            .expect("sparse continuation block");
        assert_eq!(sparse.column_count, 2);
        assert_eq!(sparse.column_index, 0);
    }

    #[test]
    fn sparse_edge_band_keeps_shifted_two_column_ownership() {
        let mut chars = Vec::new();
        // The sparse top band is a genuine inset two-column panel. Both of
        // its columns sit to the left of the body layout's gutter, so copying
        // the body gutter would collapse TOPRIGHT into the left column.
        for (index, y) in [760.0, 749.0, 738.0].iter().enumerate() {
            push_fixed_glyph_line(&mut chars, &format!("TOPLEFT{index}"), 'L', 16, 60.0, *y);
            // Offset the two columns vertically, as happens in question cards
            // whose left and right items wrap to different baselines.
            push_fixed_glyph_line(
                &mut chars,
                &format!("TOPRIGHT{index}"),
                'R',
                13,
                230.0,
                *y - 4.0,
            );
        }
        for y in [690.0, 679.0, 668.0, 657.0].iter() {
            push_fixed_glyph_line(&mut chars, "B", 'L', 42, 60.0, *y);
            push_fixed_glyph_line(&mut chars, "R", 'R', 10, 330.0, *y);
        }

        let page_x_min = chars.iter().map(|ch| ch.x).fold(f32::MAX, f32::min);
        let page_x_max = chars.iter().map(|ch| ch.x).fold(f32::MIN, f32::max);
        let top_chars = chars
            .iter()
            .copied()
            .filter(|ch| ch.y > 704.0)
            .collect::<Vec<_>>();
        let top_gutters = detect_column_gutters(&top_chars, page_x_min, page_x_max);
        assert_eq!(
            top_gutters.len(),
            1,
            "test precondition: sparse inset panel must expose its own gutter: {top_gutters:?}"
        );

        let sections = detect_layout_sections(&chars);
        assert_eq!(
            sections.iter().map(|s| s.column_count).collect::<Vec<_>>(),
            vec![2, 2],
            "same column count must not erase a real gutter transition: {sections:?}"
        );
        assert!(
            (sections[0].gutters[0] - sections[1].gutters[0]).abs() > 50.0,
            "the inset panel and body must retain their distinct gutters: {sections:?}"
        );

        let blocks = build_blocks_from_chars(&chars);
        let top_right = blocks
            .iter()
            .find(|block| block.text.contains("TOPRIGHT"))
            .expect("top-right panel column");
        assert_eq!(top_right.column_count, 2);
        assert_eq!(
            top_right.column_index, 1,
            "TOPRIGHT must remain owned by the inset panel's right column: {blocks:?}"
        );
    }

    #[test]
    fn sparse_edge_inheritance_accepts_only_source_continuous_changed_tail() {
        let mut adjacent = Vec::new();
        for (index, y) in [760.0, 749.0, 738.0, 727.0].iter().enumerate() {
            push_fixed_glyph_line(&mut adjacent, &format!("LEFT{index}"), 'L', 34, 28.0, *y);
            push_fixed_glyph_line(&mut adjacent, &format!("RIGHT{index}"), 'R', 24, 306.0, *y);
        }

        let mut sparse = Vec::new();
        push_text_line(&mut sparse, "sug", 210.0, 700.0);
        push_text_line(&mut sparse, "gested by Neil", 229.0, 700.0);
        for (index, y) in [689.0, 678.0, 667.0].iter().enumerate() {
            push_fixed_glyph_line(&mut sparse, &format!("OPTION{index}"), 'L', 20, 28.0, *y);
            push_fixed_glyph_line(&mut sparse, &format!("ANSWER{index}"), 'R', 24, 306.0, *y);
        }

        assert!(sparse_edge_ownership_changes_are_left_continuations(
            &sparse, 228.0, 280.0, &adjacent
        ));

        let mut independent = sparse.clone();
        for ch in independent
            .iter_mut()
            .filter(|ch| ch.y == 700.0 && ch.x >= 229.0)
        {
            ch.y -= 4.0;
        }
        assert!(
            !sparse_edge_ownership_changes_are_left_continuations(
                &independent,
                228.0,
                280.0,
                &adjacent,
            ),
            "a separate-baseline inset column is not a source-continuous left tail"
        );
    }

    #[test]
    fn genuine_three_to_two_column_transition_is_not_collapsed() {
        let mut chars = Vec::new();
        for (index, y) in [760.0, 751.0, 742.0, 733.0, 724.0, 715.0]
            .iter()
            .enumerate()
        {
            push_fixed_glyph_line(&mut chars, &format!("LEFT3{index}"), 'L', 20, 60.0, *y);
            push_fixed_glyph_line(&mut chars, &format!("MIDDLE3{index}"), 'M', 20, 220.0, *y);
            push_fixed_glyph_line(&mut chars, &format!("RIGHT3{index}"), 'R', 20, 380.0, *y);
        }
        // The lower layout intentionally merges the old left+middle region
        // into a wide left column. Its one gutter is close to the upper
        // section's second gutter, which is exactly the shape that a false
        // three-column repair must distinguish from a real 3 -> 2 switch.
        for (index, y) in [704.0, 695.0, 686.0, 677.0, 668.0, 659.0]
            .iter()
            .enumerate()
        {
            push_fixed_glyph_line(&mut chars, &format!("WIDELEFT2{index}"), 'L', 50, 60.0, *y);
            push_fixed_glyph_line(&mut chars, &format!("RIGHT2{index}"), 'R', 20, 380.0, *y);
        }

        let sections = detect_layout_sections(&chars);
        assert_eq!(
            sections.iter().map(|s| s.column_count).collect::<Vec<_>>(),
            vec![3, 2],
            "a genuine adjacent 3 -> 2 transition must remain intact: {sections:?}"
        );
    }

    #[test]
    fn compact_genuine_three_to_two_column_transition_is_not_collapsed() {
        let mut chars = Vec::new();
        for (index, y) in [760.0, 751.0, 742.0, 733.0, 724.0, 715.0]
            .iter()
            .enumerate()
        {
            // The middle column begins only one narrow typographic gutter
            // after the left column. Its first glyph therefore sits close to
            // the recursively detected first split, but the repeated 17.8pt
            // separation is still an independent-column boundary.
            push_fixed_glyph_line(&mut chars, &format!("CL{index}"), 'L', 20, 60.0, *y);
            push_fixed_glyph_line(&mut chars, &format!("CM{index}"), 'M', 20, 169.0, *y);
            push_fixed_glyph_line(&mut chars, &format!("CR{index}"), 'R', 20, 380.0, *y);
        }
        for (index, y) in [704.0, 695.0, 686.0, 677.0, 668.0, 659.0]
            .iter()
            .enumerate()
        {
            push_fixed_glyph_line(&mut chars, &format!("CWL{index}"), 'L', 50, 60.0, *y);
            push_fixed_glyph_line(&mut chars, &format!("CWR{index}"), 'R', 20, 380.0, *y);
        }

        let sections = detect_layout_sections(&chars);
        assert_eq!(
            sections.iter().map(|s| s.column_count).collect::<Vec<_>>(),
            vec![3, 2],
            "a compact but independently separated middle column must survive an adjacent 3 -> 2 transition: {sections:?}"
        );
    }

    #[test]
    fn checked_in_complex_fixture_keeps_completion_rows_single_column() {
        let guard = match pdfium_instance() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let cached = guard.as_ref();
        let Ok(pdfium) = cached.as_ref() else {
            return;
        };
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/parser/complex-reading.pdf");
        let document = pdfium
            .load_pdf_from_file(&path, None)
            .expect("checked-in complex fixture should open");
        let page = document.pages().get(0).expect("fixture page");
        let chars = collect_chars_with_origin(&page);
        let sections = detect_layout_sections(&chars);
        assert!(
            sections.iter().all(|section| section.column_count == 1),
            "continuous completion/table/answer rows are not three columns: {sections:?}"
        );

        let blocks = build_blocks_from_chars(&chars);
        let texts = blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();
        assert!(texts.iter().any(|text| {
            text.contains(
                "Questions 4-5 Complete the table below. Choose ONE WORD ONLY from the passage for each answer.",
            )
        }), "Q4-5 instruction must remain one source-backed row: {texts:?}");
        assert!(
            texts
                .iter()
                .any(|text| text.contains("Answers 1 TRUE 2 FALSE 3 NOT GIVEN 4 maps 5 diaries")),
            "answer evidence including q5 diaries must remain intact: {texts:?}"
        );
    }

    #[test]
    fn synthetic_completion_table_rows_do_not_create_recursive_gutters() {
        // Mirrors the geometry of the checked-in complex fixture without
        // requiring the native pdfium library. The long instruction/answer
        // rows and short `item | location` rows are intentionally mixed: a
        // histogram valley in this shape must remain one reading flow.
        let mut chars = Vec::new();
        push_text_line(
            &mut chars,
            "Questions 4-5 Complete the table below. Choose ONE WORD ONLY from the passage for each answer.",
            72.0,
            780.0,
        );
        push_text_line(&mut chars, "Item | Location", 72.0, 758.0);
        push_text_line(&mut chars, "maps | room", 72.0, 736.0);
        push_text_line(&mut chars, "diaries | room", 72.0, 714.0);
        push_text_line(
            &mut chars,
            "Answers 1 TRUE 2 FALSE 3 NOT GIVEN 4 maps 5 diaries",
            72.0,
            692.0,
        );

        let sections = detect_layout_sections(&chars);
        assert!(
            sections.iter().all(|section| section.column_count == 1),
            "single-column completion/table rows must not be recursively split: {sections:?}"
        );
        let blocks = build_blocks_from_chars(&chars);
        let texts = blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>();
        assert!(
            texts.iter().any(|text| text.contains("diaries | room")),
            "the final table row must remain source-backed: {texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|text| text.contains("Answers 1 TRUE 2 FALSE 3 NOT GIVEN 4 maps 5 diaries")),
            "answer evidence must remain intact: {texts:?}"
        );
    }

    #[test]
    fn dominant_right_edge_repairs_only_source_continuous_left_spills() {
        let split_x = 252.0;
        let mut segment_chars = Vec::new();
        for (index, y) in [740.0, 729.0, 718.0, 707.0].iter().enumerate() {
            push_text_line(&mut segment_chars, &format!("LEFT ROW {index}"), 28.0, *y);
            push_text_line(&mut segment_chars, &format!("RIGHT ROW {index}"), 306.0, *y);
        }
        // `from` is source-continuous with NUMBER but lies to the right of
        // the histogram split. `price` begins at the repeated true edge.
        push_text_line(&mut segment_chars, "NUMBER from", 220.0, 696.0);
        push_text_line(&mut segment_chars, "price while claims", 306.0, 696.0);

        let rows = build_raw_rows(&segment_chars, estimate_y_tolerance(&segment_chars));
        let dominant_edge = dominant_right_column_edge(&rows, split_x)
            .expect("the repeated x=306 right-column edge should be established");
        assert!((dominant_edge - 306.0).abs() <= 6.0, "{dominant_edge}");

        let spill_row = rows
            .iter()
            .find(|row| row.iter().any(|ch| ch.ch == 'N' && ch.x >= 220.0))
            .expect("spill row");
        let mut left = spill_row
            .iter()
            .copied()
            .filter(|ch| ch.x < split_x)
            .collect::<Vec<_>>();
        let mut right = spill_row
            .iter()
            .copied()
            .filter(|ch| ch.x >= split_x)
            .collect::<Vec<_>>();
        repair_cross_column_prefix_before_dominant_edge(
            &mut left,
            &mut right,
            split_x,
            dominant_edge,
        );
        let left_text = build_lines_within_column_refined(&left, estimate_y_tolerance(&left))
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>()
            .join(" ");
        let right_text = build_lines_within_column_refined(&right, estimate_y_tolerance(&right))
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(left_text.contains("NUMBER from"), "{left_text:?}");
        assert!(right_text.starts_with("price"), "{right_text:?}");

        // Explicit pdfium whitespace in the stranded span must not stop a
        // multi-word continuation such as `e time` from returning to `mor`.
        let mut left = vec![
            CharWithOrigin {
                ch: 'm',
                x: 240.0,
                y: 680.0,
            },
            CharWithOrigin {
                ch: 'o',
                x: 244.8,
                y: 680.0,
            },
            CharWithOrigin {
                ch: 'r',
                x: 249.6,
                y: 680.0,
            },
        ];
        let mut right = Vec::new();
        push_text_line(&mut right, "e time", 254.4, 680.0);
        push_text_line(&mut right, "therapies", 306.0, 680.0);
        repair_cross_column_prefix_before_dominant_edge(
            &mut left,
            &mut right,
            split_x,
            dominant_edge,
        );
        let left_text = build_lines_within_column_refined(&left, estimate_y_tolerance(&left))
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>()
            .join(" ");
        let right_text = build_lines_within_column_refined(&right, estimate_y_tolerance(&right))
            .into_iter()
            .map(|(text, _)| text)
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(left_text, "more time");
        assert_eq!(right_text, "therapies");

        let mut left = Vec::new();
        push_text_line(&mut left, "Ne", 240.0, 660.0);
        let mut right = Vec::new();
        push_text_line(&mut right, "il", 253.0, 660.0);
        repair_cross_column_prefix_before_dominant_edge(
            &mut left,
            &mut right,
            split_x,
            dominant_edge,
        );
        assert_eq!(left.iter().map(|ch| ch.ch).collect::<String>(), "Neil");
        assert!(right.is_empty());
    }

    #[test]
    fn dominant_right_edge_does_not_steal_real_right_column_openings() {
        let split_x = 252.0;
        let dominant_edge = 306.0;
        let make_left = || {
            vec![
                CharWithOrigin {
                    ch: 'l',
                    x: 238.0,
                    y: 680.0,
                },
                CharWithOrigin {
                    ch: 'e',
                    x: 242.8,
                    y: 680.0,
                },
                CharWithOrigin {
                    ch: 'f',
                    x: 247.6,
                    y: 680.0,
                },
            ]
        };

        // A long, continuous right-column sentence may cross the dominant x
        // coordinate, but it has no second source run separated by a gutter.
        let mut left = make_left();
        let mut right = Vec::new();
        push_text_line(&mut right, "in contrast with earlier results", 253.0, 680.0);
        let original = right.iter().map(|ch| ch.ch).collect::<String>();
        repair_cross_column_prefix_before_dominant_edge(
            &mut left,
            &mut right,
            split_x,
            dominant_edge,
        );
        assert_eq!(right.iter().map(|ch| ch.ch).collect::<String>(), original);

        // A short independent label/item beginning at the established right
        // edge also remains right-owned.
        let mut left = make_left();
        let mut right = Vec::new();
        push_text_line(&mut right, "map", 306.0, 660.0);
        repair_cross_column_prefix_before_dominant_edge(
            &mut left,
            &mut right,
            split_x,
            dominant_edge,
        );
        assert_eq!(right.iter().map(|ch| ch.ch).collect::<String>(), "map");
    }
}
