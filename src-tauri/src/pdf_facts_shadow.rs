use crate::artifact_store::write_canonical_json_atomic;
use crate::job_store::load_job;
use crate::pdf_ingest::{
    append_page_asset_ids, append_page_visual_objects, bounds_for_points, build_compare_report,
    build_ocr_plan_summary, collect_page_geometries, enrich_page, svg_escape, PdfPageGeometry,
};
use crate::schema::document_ir_v2::DocumentIRV2;
use crate::util::{read_json, write_text};
use crate::{CommandResult, ImportJob, SourceFile};
use chrono::Utc;
use pdf_extract::{
    ColorSpace, Document, MediaBox, OutputDev, OutputError, Path as PdfPath, PathOp, Transform,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub(crate) const SHADOW_ARTIFACT_FILE: &str = "document-ir-v2.shadow.json";
pub(crate) const SHADOW_ERROR_FILE: &str = "document-ir-v2.shadow.error.json";
pub(crate) const SHADOW_OVERLAY_FILE: &str = "document-ir-v2.shadow.overlay.svg";
pub(crate) const SHADOW_COMPARE_FILE: &str = "document-ir-v2.shadow.compare.json";

const MAX_PDF_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 1_000;
const MAX_PDF_OBJECTS: usize = 1_000_000;
const MAX_IMAGE_PIXELS: u64 = 200_000_000;
const MAX_EMBEDDED_ASSET_BYTES: usize = 256 * 1024 * 1024;
const MAX_TOTAL_IMAGE_PIXELS: u64 = 800_000_000;
const MAX_TOTAL_EMBEDDED_ASSET_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default)]
struct PdfAssetBudget {
    total_image_pixels: u64,
    total_embedded_asset_bytes: u64,
}

impl PdfAssetBudget {
    fn reserve_image_pixels(&mut self, amount: u64) -> CommandResult<()> {
        let next = self.total_image_pixels.checked_add(amount).ok_or_else(|| {
            format!(
                "PDF_RESOURCE_LIMIT_TOTAL_IMAGE_PIXELS:total=overflow:max={MAX_TOTAL_IMAGE_PIXELS}"
            )
        })?;
        if next > MAX_TOTAL_IMAGE_PIXELS {
            return Err(format!(
                "PDF_RESOURCE_LIMIT_TOTAL_IMAGE_PIXELS:total={next}:max={MAX_TOTAL_IMAGE_PIXELS}"
            ));
        }
        self.total_image_pixels = next;
        Ok(())
    }

    fn reserve_embedded_asset_bytes(&mut self, amount: u64) -> CommandResult<()> {
        let next = self
            .total_embedded_asset_bytes
            .checked_add(amount)
            .ok_or_else(|| {
                format!(
                    "PDF_RESOURCE_LIMIT_TOTAL_IMAGE_BYTES:total=overflow:max={MAX_TOTAL_EMBEDDED_ASSET_BYTES}"
                )
            })?;
        if next > MAX_TOTAL_EMBEDDED_ASSET_BYTES {
            return Err(format!(
                "PDF_RESOURCE_LIMIT_TOTAL_IMAGE_BYTES:total={next}:max={MAX_TOTAL_EMBEDDED_ASSET_BYTES}"
            ));
        }
        self.total_embedded_asset_bytes = next;
        Ok(())
    }
}

struct RawGlyph {
    value: Value,
    area: f64,
    unicode_error: bool,
    text_len: usize,
    source_line_break_after: bool,
}

struct PageFacts {
    page_num: u32,
    geometry: PdfPageGeometry,
    glyphs: Vec<RawGlyph>,
    vector_paths: Vec<Value>,
}

struct PdfFactsCollector {
    pages: Vec<PageFacts>,
    current: Option<PageFacts>,
    page_geometries: BTreeMap<u32, PdfPageGeometry>,
    source_file_id: String,
    source_hash: String,
    char_offset: u32,
}

impl PdfFactsCollector {
    fn new(
        page_geometries: BTreeMap<u32, PdfPageGeometry>,
        source_file_id: String,
        source_hash: String,
    ) -> Self {
        Self {
            pages: Vec::new(),
            current: None,
            page_geometries,
            source_file_id,
            source_hash,
            char_offset: 0,
        }
    }

    fn record_path(&mut self, ctm: &Transform, path: &PdfPath, paint: &str, color: &[f64]) {
        let Some(page) = self.current.as_mut() else {
            return;
        };
        let Some(value) = path_value(
            page.page_num,
            &page.geometry,
            &self.source_file_id,
            &self.source_hash,
            page.vector_paths.len(),
            ctm,
            path,
            paint,
            color,
        ) else {
            return;
        };
        page.vector_paths.push(value);
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
        let geometry = self
            .page_geometries
            .get(&page_num)
            .cloned()
            .unwrap_or_else(|| {
                PdfPageGeometry::fallback(page_num.saturating_sub(1), width, height, 0)
            });
        self.current = Some(PageFacts {
            page_num,
            geometry,
            glyphs: Vec::new(),
            vector_paths: Vec::new(),
        });
        self.char_offset = 0;
        Ok(())
    }

    fn end_page(&mut self) -> Result<(), OutputError> {
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
        let Some(page) = self.current.as_mut() else {
            return Ok(());
        };

        let safe_font_size = finite_positive(font_size, 1.0);
        let safe_width = finite_positive(width.abs(), 0.01);
        let advance = (safe_width * safe_font_size).max(0.01);
        let glyph_height = safe_font_size.max(0.5);

        let native_origin = (trm.m31, trm.m32);
        let advance_point = (
            native_origin.0 + trm.m11 * advance,
            native_origin.1 + trm.m12 * advance,
        );
        let height_point = (
            native_origin.0 + trm.m21 * glyph_height,
            native_origin.1 + trm.m22 * glyph_height,
        );
        let far_point = (
            advance_point.0 + trm.m21 * glyph_height,
            advance_point.1 + trm.m22 * glyph_height,
        );
        let native_corners = [native_origin, advance_point, far_point, height_point];
        let display_corners = native_corners.map(|point| page.geometry.display_point(point));
        let native_bounds =
            bounds_for_points(&native_corners).unwrap_or(crate::pdf_ingest::Bounds {
                x: native_origin.0,
                y: native_origin.1,
                width: 0.01,
                height: 0.01,
            });
        let display_bounds =
            bounds_for_points(&display_corners).unwrap_or(crate::pdf_ingest::Bounds {
                x: 0.0,
                y: 0.0,
                width: 0.01,
                height: 0.01,
            });
        let display_origin = page.geometry.display_point(native_origin);
        let display_advance = page.geometry.display_point(advance_point);

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
        let rect = page.geometry.display_rect(display_bounds);
        let native_rect = page.geometry.native_rect(native_bounds);
        let quad = json!({
            "points": display_corners
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
            "nativeBBox": native_rect,
            "displayBBox": rect.clone(),
            "pdfToDisplay": page.geometry.pdf_to_display,
            "charRange": {"start": start, "end": self.char_offset},
            "extractionMode": "pdf_native",
            "sourceHash": self.source_hash
        });
        let value = json!({
            "id": glyph_id,
            "text": text,
            "bbox": rect,
            "quad": quad,
            "origin": {"x": clean_number(display_origin.0), "y": clean_number(display_origin.1)},
            "baseline": clean_number(display_origin.1),
            "angleRad": clean_number((display_advance.1 - display_origin.1).atan2(display_advance.0 - display_origin.0)),
            "style": {"fontSizePt": clean_number(safe_font_size * page.geometry.user_unit)},
            "unicodeMapError": unicode_error,
            "visibilityObserved": false,
            "unicodeMapErrorObserved": false,
            "geometryBasis": "text_matrix_derived",
            "confidence": if unicode_error {0.20} else {0.62},
            "source": "native",
            "sourceAnchor": source_anchor
        });
        page.glyphs.push(RawGlyph {
            value,
            area: display_bounds.area(),
            unicode_error,
            text_len,
            source_line_break_after: false,
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
        if let Some(glyph) = self
            .current
            .as_mut()
            .and_then(|page| page.glyphs.last_mut())
        {
            glyph.source_line_break_after = true;
        }
        Ok(())
    }

    fn stroke(
        &mut self,
        ctm: &Transform,
        _colorspace: &ColorSpace,
        color: &[f64],
        path: &PdfPath,
    ) -> Result<(), OutputError> {
        self.record_path(ctm, path, "stroke", color);
        Ok(())
    }

    fn fill(
        &mut self,
        ctm: &Transform,
        _colorspace: &ColorSpace,
        color: &[f64],
        path: &PdfPath,
    ) -> Result<(), OutputError> {
        self.record_path(ctm, path, "fill", color);
        Ok(())
    }
}

fn transformed_point(transform: &Transform, point: (f64, f64)) -> (f64, f64) {
    (
        transform.m11 * point.0 + transform.m21 * point.1 + transform.m31,
        transform.m12 * point.0 + transform.m22 * point.1 + transform.m32,
    )
}

fn path_value(
    page_num: u32,
    geometry: &PdfPageGeometry,
    source_file_id: &str,
    source_hash: &str,
    path_index: usize,
    ctm: &Transform,
    path: &PdfPath,
    paint: &str,
    color: &[f64],
) -> Option<Value> {
    let id = format!("p{:03}-path{:05}", page_num, path_index + 1);
    let mut points = Vec::<(f64, f64)>::new();
    let mut native_points = Vec::<(f64, f64)>::new();
    let mut commands = Vec::<Value>::new();
    for op in &path.ops {
        match op {
            PathOp::MoveTo(x, y) => {
                let native = transformed_point(ctm, (*x, *y));
                let point = geometry.display_point(native);
                native_points.push(native);
                points.push(point);
                commands.push(
                    json!({"op": "move", "x": clean_number(point.0), "y": clean_number(point.1)}),
                );
            }
            PathOp::LineTo(x, y) => {
                let native = transformed_point(ctm, (*x, *y));
                let point = geometry.display_point(native);
                native_points.push(native);
                points.push(point);
                commands.push(
                    json!({"op": "line", "x": clean_number(point.0), "y": clean_number(point.1)}),
                );
            }
            PathOp::CurveTo(x1, y1, x2, y2, x, y) => {
                let native_first = transformed_point(ctm, (*x1, *y1));
                let native_second = transformed_point(ctm, (*x2, *y2));
                let native_third = transformed_point(ctm, (*x, *y));
                let first = geometry.display_point(native_first);
                let second = geometry.display_point(native_second);
                let third = geometry.display_point(native_third);
                native_points.extend([native_first, native_second, native_third]);
                points.extend([first, second, third]);
                commands.push(json!({
                    "op": "curve",
                    "points": [
                        clean_number(first.0), clean_number(first.1),
                        clean_number(second.0), clean_number(second.1),
                        clean_number(third.0), clean_number(third.1)
                    ]
                }));
            }
            PathOp::Rect(x, y, width, height) => {
                let native_corners = [
                    (*x, *y),
                    (*x + *width, *y),
                    (*x + *width, *y + *height),
                    (*x, *y + *height),
                ]
                .into_iter()
                .map(|point| transformed_point(ctm, point))
                .collect::<Vec<_>>();
                native_points.extend(native_corners.iter().copied());
                let corners = native_corners
                    .iter()
                    .map(|point| geometry.display_point(*point))
                    .collect::<Vec<_>>();
                points.extend(corners.iter().copied());
                if let Some(first) = corners.first() {
                    commands.push(json!({"op": "move", "x": clean_number(first.0), "y": clean_number(first.1)}));
                    for point in corners.iter().skip(1) {
                        commands.push(json!({"op": "line", "x": clean_number(point.0), "y": clean_number(point.1)}));
                    }
                    commands.push(json!({"op": "close"}));
                }
            }
            PathOp::Close => commands.push(json!({"op": "close"})),
        }
    }
    if points.is_empty() {
        return None;
    }
    let min_x = points
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min);
    let max_x = points
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = points
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let max_y = points
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let width = (max_x - min_x).max(0.01);
    let height = (max_y - min_y).max(0.01);
    let native_bbox = bounds_for_points(&native_points);
    Some(json!({
        "id": id,
        "bbox": geometry.display_rect(crate::pdf_ingest::Bounds {x: min_x, y: min_y, width, height}),
        "commands": commands,
        "strokeColor": if paint == "stroke" { color_to_hex(color) } else { None },
        "fillColor": if paint == "fill" { color_to_hex(color) } else { None },
        "isAxisAlignedRule": (width <= 2.6 && height >= 12.0) || (height <= 2.6 && width >= 12.0),
        "sourceAnchor": {
            "sourceFileId": source_file_id,
            "pageIndex": page_num.saturating_sub(1),
            "nodeIds": [format!("p{:03}-path{:05}", page_num, path_index + 1)],
            "bbox": geometry.display_rect(crate::pdf_ingest::Bounds {x: min_x, y: min_y, width, height}),
            "displayBBox": geometry.display_rect(crate::pdf_ingest::Bounds {x: min_x, y: min_y, width, height}),
            "nativeBBox": native_bbox.map(|bounds| geometry.native_rect(bounds)),
            "pdfToDisplay": geometry.pdf_to_display,
            "extractionMode": "pdf_native",
            "sourceHash": source_hash
        }
    }))
}

fn color_to_hex(color: &[f64]) -> Option<String> {
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b) = match color {
        [gray] => {
            let value = channel(*gray);
            (value, value, value)
        }
        [r, g, b, ..] if color.len() == 3 => (channel(*r), channel(*g), channel(*b)),
        [c, m, y, k, ..] => (
            channel((1.0 - c) * (1.0 - k)),
            channel((1.0 - m) * (1.0 - k)),
            channel((1.0 - y) * (1.0 - k)),
        ),
        _ => return None,
    };
    Some(format!("#{r:02X}{g:02X}{b:02X}FF"))
}

pub(crate) fn write_pdf_facts_shadow(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
    output_path: &Path,
) -> CommandResult<Value> {
    write_pdf_facts_shadow_with_v1(job, source, input_path, output_path, None)
}

pub(crate) fn write_pdf_facts_shadow_with_v1(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
    output_path: &Path,
    v1_document: Option<&Value>,
) -> CommandResult<Value> {
    let output_parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(output_parent)
        .map_err(|error| format!("pdf_shadow_output_dir_create_failed:{}", error))?;
    let staging_root =
        output_parent.join(format!(".pdf-shadow-txn-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(staging_root.join("assets").join("shadow").join("pdf"))
        .map_err(|error| format!("pdf_shadow_staging_create_failed:{}", error))?;
    let result = (|| -> CommandResult<Value> {
        let mut value = extract_pdf_facts_shadow_internal(
            job,
            source,
            input_path,
            Some(staging_root.as_path()),
        )?;
        merge_v1_diagram_question_region_assets(&mut value, v1_document, source, &staging_root)?;
        let typed = serde_json::from_value::<DocumentIRV2>(value)
            .map_err(|error| format!("pdf_facts_shadow_v1_asset_bridge_schema_failed:{}", error))?;
        let value = serde_json::to_value(typed).map_err(|error| error.to_string())?;
        let output_name = output_path
            .file_name()
            .ok_or_else(|| "pdf_shadow_output_file_name_missing".to_string())?;
        write_canonical_json_atomic(&staging_root.join(output_name), &value)?;
        let compare = build_compare_report(&job.job_id, &value, v1_document);
        write_canonical_json_atomic(&staging_root.join(SHADOW_COMPARE_FILE), &compare)?;
        commit_shadow_bundle(&staging_root, output_path)?;
        Ok(value)
    })();
    if staging_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
    }
    result
}

/// Promotes the source-backed page crops discovered by the live pdfium V1
/// parser into the canonical V2 asset contract. The shadow extractor cannot
/// infer this semantic crop from PDF object resources alone: the evidence is
/// the V1 range heading + diagram instruction + real page geometry.
fn merge_v1_diagram_question_region_assets(
    shadow: &mut Value,
    v1_document: Option<&Value>,
    source: &SourceFile,
    staging_root: &Path,
) -> CommandResult<()> {
    let Some(v1_document) = v1_document else {
        return Ok(());
    };
    let v1_assets = v1_document
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for asset in v1_assets {
        let Some(region) = asset.get("diagramQuestionRegion") else {
            continue;
        };
        let Some(asset_id) = asset.get("assetId").and_then(Value::as_str) else {
            continue;
        };
        let Some(source_path) = asset.get("path").and_then(Value::as_str).map(Path::new) else {
            continue;
        };
        if !source_path.exists() {
            return Err(format!(
                "diagram_question_region_source_asset_missing:{}:{}",
                asset_id,
                source_path.display()
            ));
        }
        let bytes = fs::read(source_path).map_err(|error| {
            format!(
                "diagram_question_region_source_asset_read_failed:{}:{}",
                source_path.display(),
                error
            )
        })?;
        let relative_path = format!("assets/shadow/pdf/{asset_id}.png");
        write_shadow_asset(staging_root, &relative_path, &bytes)?;
        let page_number = asset.get("pageIndex").and_then(Value::as_u64).unwrap_or(1);
        let v1_bbox = asset
            .get("bbox")
            .and_then(Value::as_array)
            .filter(|bbox| bbox.len() == 4);
        let page_height = v1_document
            .get("pages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|page| page.get("pageIndex").and_then(Value::as_u64) == Some(page_number))
            .and_then(|page| page.get("height"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let rects = v1_bbox.map(|bbox| {
            let x0 = bbox[0].as_f64().unwrap_or(0.0);
            let y0 = bbox[1].as_f64().unwrap_or(0.0);
            let x1 = bbox[2].as_f64().unwrap_or(x0);
            let y1 = bbox[3].as_f64().unwrap_or(y0);
            let width = (x1 - x0).max(0.01);
            let height = (y1 - y0).max(0.01);
            (
                rect_value(
                    x0,
                    (page_height - y1).max(0.0),
                    width,
                    height,
                    "top-left",
                    0,
                ),
                rect_value(x0, y0, width, height, "bottom-left", 0),
            )
        });
        let display_bbox = rects.as_ref().map(|(display, _)| display.clone());
        let native_bbox = rects.as_ref().map(|(_, native)| native.clone());
        let descriptor = json!({
            "assetId": asset_id,
            "kind": "page_crop",
            "mime": "image/png",
            "relativePath": relative_path,
            "sha256": crate::hash_bytes(&bytes),
            "byteLength": bytes.len() as u64,
            "widthPx": asset.get("width").and_then(Value::as_u64).unwrap_or(0),
            "heightPx": asset.get("height").and_then(Value::as_u64).unwrap_or(0),
            "extractionMode": "page_crop",
            "altText": format!("Diagram question region on page {}", page_number),
            "decorative": false,
            "diagramQuestionRegion": region,
            "sourceAnchor": {
                "sourceFileId": source.file_id,
                "pageIndex": page_number.saturating_sub(1),
                "nodeIds": [format!("{}-source-region", asset_id)],
                "bbox": display_bbox,
                "displayBBox": display_bbox,
                "nativeBBox": native_bbox,
                "extractionMode": "pdf_rendered_crop",
                "sourceHash": source.sha256
            }
        });
        if let Some(assets) = shadow.get_mut("assets").and_then(Value::as_array_mut) {
            if !assets
                .iter()
                .any(|item| item.get("assetId").and_then(Value::as_str) == Some(asset_id))
            {
                assets.push(descriptor);
            }
        }
        if let Some(page) = shadow
            .get_mut("pages")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .find(|page| {
                page.get("pageIndex").and_then(Value::as_u64) == Some(page_number.saturating_sub(1))
            })
        {
            let ids = page
                .as_object_mut()
                .map(|object| object.entry("assetIds").or_insert_with(|| json!([])))
                .and_then(Value::as_array_mut);
            if let Some(ids) = ids {
                if !ids.iter().any(|id| id.as_str() == Some(asset_id)) {
                    ids.push(json!(asset_id));
                }
            }
        }
        if let Some(ledger) = shadow
            .get_mut("coverageLedger")
            .and_then(Value::as_array_mut)
        {
            ledger.push(json!({
                "sourceNodeId": asset_id,
                "disposition": "unassigned",
                "targetIds": [],
                "reason": "diagram question region retained; targeted OCR and exact number closure required"
            }));
        }
        if let Some(warnings) = shadow
            .pointer_mut("/parser/warnings")
            .and_then(Value::as_array_mut)
        {
            warnings.push(json!(format!(
                "DIAGRAM_QUESTION_REGION_OCR_REQUIRED:page={}:assetId={}",
                page_number, asset_id
            )));
        }
    }
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> CommandResult<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|error| format!("pdf_shadow_path_remove_failed:{}:{}", path.display(), error))
}

fn rollback_shadow_bundle(
    targets: &[std::path::PathBuf],
    backups: &[std::path::PathBuf],
    installed: &[usize],
    backed_up: &[usize],
    backup_root: &Path,
) -> Option<String> {
    let mut failures = Vec::new();
    for index in installed.iter().rev().copied() {
        if let Err(error) = remove_path_if_exists(&targets[index]) {
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
            "PDF_SHADOW_ROLLBACK_FAILED:{}:backup_preserved={}",
            failures.join(";"),
            backup_root.display()
        ))
    }
}

fn commit_shadow_bundle(staging_root: &Path, output_path: &Path) -> CommandResult<()> {
    commit_shadow_bundle_with_hook(staging_root, output_path, |_, _, _, _| Ok(()))
}

fn commit_shadow_bundle_with_hook<F>(
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
        .ok_or_else(|| "pdf_shadow_output_file_name_missing".to_string())?;
    let staged = [
        staging_root.join(output_name),
        staging_root.join(SHADOW_COMPARE_FILE),
        staging_root.join("assets").join("shadow").join("pdf"),
    ];
    for path in &staged {
        if !path.exists() {
            return Err(format!(
                "pdf_shadow_staged_component_missing:{}",
                path.display()
            ));
        }
    }
    let targets = [
        output_path.to_path_buf(),
        output_parent.join(SHADOW_COMPARE_FILE),
        output_parent.join("assets").join("shadow").join("pdf"),
    ];
    let backup_root = output_parent.join(format!(
        ".pdf-shadow-backup-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&backup_root)
        .map_err(|error| format!("pdf_shadow_backup_create_failed:{}", error))?;
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
                rollback_shadow_bundle(&targets, &backups, &[], &backed_up, &backup_root);
            return Err(match rollback {
                Some(rollback) => format!(
                    "pdf_shadow_backup_failed:{}:{};{rollback}",
                    target.display(),
                    error
                ),
                None => format!("pdf_shadow_backup_failed:{}:{}", target.display(), error),
            });
        }
        backed_up.push(index);
    }

    let mut installed = Vec::<usize>::new();
    for (index, source) in staged.iter().enumerate() {
        if let Some(parent) = targets[index].parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                let rollback = rollback_shadow_bundle(
                    &targets,
                    &backups,
                    &installed,
                    &backed_up,
                    &backup_root,
                );
                return Err(match rollback {
                    Some(rollback) => {
                        format!("pdf_shadow_target_dir_create_failed:{error};{rollback}")
                    }
                    None => format!("pdf_shadow_target_dir_create_failed:{error}"),
                });
            }
        }
        if let Err(error) = before_install(index, source, &targets[index], &backup_root) {
            let rollback =
                rollback_shadow_bundle(&targets, &backups, &installed, &backed_up, &backup_root);
            return Err(match rollback {
                Some(rollback) => {
                    format!("pdf_shadow_commit_hook_failed:index={index}:{error};{rollback}")
                }
                None => format!("pdf_shadow_commit_hook_failed:index={index}:{error}"),
            });
        }
        if let Err(error) = fs::rename(source, &targets[index]) {
            let rollback =
                rollback_shadow_bundle(&targets, &backups, &installed, &backed_up, &backup_root);
            return Err(match rollback {
                Some(rollback) => format!(
                    "pdf_shadow_commit_failed:{}:{};{rollback}",
                    targets[index].display(),
                    error
                ),
                None => format!(
                    "pdf_shadow_commit_failed:{}:{}",
                    targets[index].display(),
                    error
                ),
            });
        }
        installed.push(index);
    }
    let _ = fs::remove_dir_all(&backup_root);
    Ok(())
}

pub(crate) fn extract_pdf_facts_shadow(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
) -> CommandResult<Value> {
    extract_pdf_facts_shadow_internal(job, source, input_path, None)
}

fn extract_pdf_facts_shadow_internal(
    job: &ImportJob,
    source: &SourceFile,
    input_path: &Path,
    asset_root: Option<&Path>,
) -> CommandResult<Value> {
    let extraction_started_at = Utc::now();
    let (document, load_warnings) = load_document_with_xref_repair(input_path)?;
    enforce_document_resource_limits(document.get_pages().len(), document.objects.len())?;
    let page_geometries = collect_page_geometries(&document);
    let mut collector = PdfFactsCollector::new(
        page_geometries,
        source.file_id.clone(),
        source.sha256.clone(),
    );
    pdf_extract::output_doc(&document, &mut collector)
        .map_err(|error| format!("pdf_facts_shadow_extract_failed:{}", error))?;

    let mut pages = collector
        .pages
        .into_iter()
        .map(page_value)
        .collect::<Vec<_>>();
    let (mut assets, mut page_asset_ids, mut asset_warnings, asset_budget) =
        extract_pdf_visual_assets(&document, source, asset_root)?;
    let object_warnings = collect_page_object_facts(&document, source, &assets, &mut pages);
    let (vector_assets, vector_page_asset_ids, vector_warnings) =
        extract_pdf_vector_assets(&pages, source, asset_root)?;
    assets.extend(vector_assets);
    asset_warnings.extend(vector_warnings);
    for (page_index, vector_ids) in vector_page_asset_ids {
        let target = page_asset_ids.entry(page_index).or_default();
        for asset_id in vector_ids {
            if !target.contains(&asset_id) {
                target.push(asset_id);
            }
        }
    }
    for (page_index, asset_ids) in &page_asset_ids {
        if let Some(page) = pages
            .iter_mut()
            .find(|page| page.get("pageIndex").and_then(Value::as_u64) == Some(*page_index as u64))
        {
            append_page_asset_ids(page, asset_ids);
            append_page_visual_objects(page, &assets, asset_ids);
        }
    }
    let annotation_warnings =
        collect_pdf_annotations(&document, source, asset_root, &mut assets, &mut pages);
    let hidden_text_pages = document
        .get_pages()
        .into_iter()
        .filter_map(|(page_num, page_id)| {
            page_contains_hidden_text(&document, page_id).then_some(page_num.saturating_sub(1))
        })
        .collect::<BTreeSet<_>>();
    let mut layer_summaries = Vec::with_capacity(pages.len());
    for page in &mut pages {
        let page_index = page
            .get("pageIndex")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32;
        if hidden_text_pages.contains(&page_index) {
            if let Some(object) = page.as_object_mut() {
                object.insert("_hiddenTextDetected".to_string(), Value::Bool(true));
            }
        }
        layer_summaries.push(enrich_page(page));
    }
    let physical_summary = json!({
        "pageCount": layer_summaries.len(),
        "lineCount": layer_summaries.iter().map(|summary| summary.line_count).sum::<usize>(),
        "regionCount": layer_summaries.iter().map(|summary| summary.region_count).sum::<usize>(),
        "tableCount": layer_summaries.iter().map(|summary| summary.table_count).sum::<usize>(),
        "vectorPathCount": layer_summaries.iter().map(|summary| summary.vector_path_count).sum::<usize>(),
        "visualObjectCount": layer_summaries.iter().map(|summary| summary.visual_object_count).sum::<usize>(),
        "annotationCount": layer_summaries.iter().map(|summary| summary.annotation_count).sum::<usize>(),
        "columnCountMax": layer_summaries.iter().map(|summary| summary.column_count).max().unwrap_or(0),
        "readingOrderConfidenceMin": layer_summaries.iter().map(|summary| summary.reading_order_confidence).fold(1.0_f64, f64::min),
        "ocrRegionCount": layer_summaries.iter().map(|summary| summary.ocr_region_count).sum::<usize>(),
        "duplicateTextRatioMax": layer_summaries.iter().map(|summary| summary.duplicate_text_ratio).fold(0.0_f64, f64::max),
        "warnings": layer_summaries.iter().flat_map(|summary| summary.warnings.iter().cloned()).collect::<std::collections::BTreeSet<_>>()
    });
    let mut coverage_ledger = coverage_ledger_from_pages(&pages);
    for asset in &assets {
        if let Some(asset_id) = asset.get("assetId").and_then(Value::as_str) {
            if coverage_ledger
                .iter()
                .any(|entry| entry.get("sourceNodeId").and_then(Value::as_str) == Some(asset_id))
            {
                continue;
            }
            coverage_ledger.push(json!({
                "sourceNodeId": asset_id,
                "disposition": "unassigned",
                "targetIds": [],
                "reason": "embedded visual asset retained in job store; semantic assignment is deferred"
            }));
        }
    }
    let preflight = build_pdf_preflight(&document, &pages, asset_budget);
    let pages = pages
        .into_iter()
        .map(|mut page| {
            if let Some(object) = page.as_object_mut() {
                object.remove("_coverage");
                object.remove("_hiddenTextDetected");
                object.remove("_visualObjects");
                object.remove("_annotations");
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
    let mut warnings = load_warnings;
    warnings.extend(asset_warnings);
    warnings.extend(annotation_warnings);
    warnings.extend(object_warnings);
    let value = json!({
        "schemaVersion": "DocumentIRV2",
        "documentId": format!("document-{}", &source.sha256[..source.sha256.len().min(16)]),
        "jobId": job.job_id,
        "sourceFiles": [source_file],
        "pages": pages,
        "assets": assets,
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
                "semanticLayers": false,
                "preflight": preflight,
                "physicalLayers": {
                    "lineBuilder": "adaptive-baseline-v1",
                    "regionBuilder": "projection-gutter-v1",
                    "readingOrder": "column-aware-dag-shadow-v1",
                    "tableDetector": "ruled-and-borderless-shadow-v1",
                    "figureExtraction": "embedded-xobject-and-vector-render-shadow-v1",
                    "summary": physical_summary
                },
                "ocrPlan": build_ocr_plan_summary(&pages)
            },
            "warnings": warnings
        }
    });
    let typed = serde_json::from_value::<DocumentIRV2>(value)
        .map_err(|error| format!("pdf_facts_shadow_schema_validation_failed:{}", error))?;
    if !typed.is_supported_schema_version() {
        return Err("pdf_facts_shadow_schema_version_unsupported".to_string());
    }
    serde_json::to_value(typed).map_err(|error| error.to_string())
}

fn object_dictionary(object: &pdf_extract::Object) -> Option<&pdf_extract::Dictionary> {
    match object {
        pdf_extract::Object::Dictionary(dictionary) => Some(dictionary),
        pdf_extract::Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

fn dictionary_name(dictionary: &pdf_extract::Dictionary, key: &[u8]) -> Option<String> {
    dictionary
        .get(key)
        .ok()
        .and_then(|value| value.as_name().ok())
        .map(|value| String::from_utf8_lossy(value).to_string())
}

fn build_pdf_preflight(
    document: &Document,
    pages: &[Value],
    asset_budget: PdfAssetBudget,
) -> Value {
    let encrypted = document.trailer.get(b"Encrypt").is_ok();
    let mut embedded_file_count = 0usize;
    let mut has_javascript = false;
    let mut has_launch_actions = false;
    for object in document.objects.values() {
        let Some(dictionary) = object_dictionary(object) else {
            continue;
        };
        if dictionary_name(dictionary, b"Type").as_deref() == Some("EmbeddedFile") {
            embedded_file_count += 1;
        }
        let action = dictionary_name(dictionary, b"S");
        has_javascript |= action.as_deref() == Some("JavaScript") || dictionary.get(b"JS").is_ok();
        has_launch_actions |= action.as_deref() == Some("Launch");
    }
    let mut warnings = Vec::<String>::new();
    if encrypted {
        warnings.push("PDF_ENCRYPTED_UNSUPPORTED".to_string());
    }
    if embedded_file_count > 0 {
        warnings.push("PDF_EMBEDDED_FILES_IGNORED".to_string());
    }
    if has_javascript {
        warnings.push("PDF_JAVASCRIPT_IGNORED".to_string());
    }
    if has_launch_actions {
        warnings.push("PDF_LAUNCH_ACTION_IGNORED".to_string());
    }
    let page_reports = pages
        .iter()
        .map(|page| {
            json!({
                "pageIndex": page.get("pageIndex").and_then(Value::as_u64).unwrap_or(0),
                "widthPt": page.get("widthPt").and_then(Value::as_f64).unwrap_or(0.0),
                "heightPt": page.get("heightPt").and_then(Value::as_f64).unwrap_or(0.0),
                "rotation": page.get("rotation").and_then(Value::as_u64).unwrap_or(0),
                "nativeCharCount": page.pointer("/quality/nativeCharacterCount").and_then(Value::as_u64).unwrap_or(0),
                "unicodeErrorRatio": page.pointer("/quality/unicodeErrorRatio").and_then(Value::as_f64).unwrap_or(0.0),
                "imageCoverageRatio": page.pointer("/quality/imageCoverageRatio").and_then(Value::as_f64).unwrap_or(0.0),
                "textBBoxCoverageRatio": page.pointer("/quality/textCoverageRatio").and_then(Value::as_f64).unwrap_or(0.0),
                "vectorObjectCount": page.get("vectorPaths").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
                "duplicateTextRatio": page.pointer("/quality/duplicateTextRatio").and_then(Value::as_f64).unwrap_or(0.0),
                "classification": page.pointer("/quality/classification").and_then(Value::as_str).unwrap_or("mixed")
            })
        })
        .collect::<Vec<_>>();
    json!({
        "encrypted": encrypted,
        "pageCount": pages.len(),
        "objectCount": document.objects.len(),
        "embeddedFileCount": embedded_file_count,
        "hasJavaScript": has_javascript,
        "hasLaunchActions": has_launch_actions,
        "totalImagePixels": asset_budget.total_image_pixels,
        "totalEmbeddedAssetBytes": asset_budget.total_embedded_asset_bytes,
        "pageReports": page_reports,
        "limits": {
            "maxPdfBytes": MAX_PDF_BYTES,
            "maxPages": MAX_PDF_PAGES,
            "maxObjects": MAX_PDF_OBJECTS,
            "maxImagePixels": MAX_IMAGE_PIXELS,
            "maxEmbeddedAssetBytes": MAX_EMBEDDED_ASSET_BYTES,
            "maxTotalImagePixels": MAX_TOTAL_IMAGE_PIXELS,
            "maxTotalEmbeddedAssetBytes": MAX_TOTAL_EMBEDDED_ASSET_BYTES
        },
        "warnings": warnings
    })
}

fn enforce_document_resource_limits(page_count: usize, object_count: usize) -> CommandResult<()> {
    if page_count > MAX_PDF_PAGES {
        return Err(format!(
            "PDF_RESOURCE_LIMIT_PAGES:{page_count}>{MAX_PDF_PAGES}"
        ));
    }
    if object_count > MAX_PDF_OBJECTS {
        return Err(format!(
            "PDF_RESOURCE_LIMIT_OBJECTS:{object_count}>{MAX_PDF_OBJECTS}"
        ));
    }
    Ok(())
}

fn dereference_pdf_object<'a>(
    document: &'a Document,
    object: &'a pdf_extract::Object,
) -> Option<&'a pdf_extract::Object> {
    match object {
        pdf_extract::Object::Reference(reference) => document.get_object(*reference).ok(),
        _ => Some(object),
    }
}

fn page_resource_dictionaries(
    document: &Document,
    page_id: pdf_extract::ObjectId,
) -> Vec<&pdf_extract::Dictionary> {
    let Ok((direct, inherited_ids)) = document.get_page_resources(page_id) else {
        return Vec::new();
    };
    let mut resources = Vec::new();
    if let Some(direct) = direct {
        resources.push(direct);
    }
    for resource_id in inherited_ids {
        if let Ok(dictionary) = document.get_dictionary(resource_id) {
            resources.push(dictionary);
        }
    }
    resources
}

fn pdf_number(object: &pdf_extract::Object) -> Option<f64> {
    match object {
        pdf_extract::Object::Integer(value) => Some(*value as f64),
        pdf_extract::Object::Real(value) => Some(*value as f64),
        _ => None,
    }
}

fn page_contains_hidden_text(document: &Document, page_id: pdf_extract::ObjectId) -> bool {
    let Ok(content) = document.get_and_decode_page_content(page_id) else {
        return false;
    };
    let mut white_fill = false;
    let mut text_render_mode = 0i64;
    for operation in content.operations {
        let operands = &operation.operands;
        match operation.operator.as_str() {
            "g" => {
                white_fill = operands
                    .first()
                    .and_then(pdf_number)
                    .is_some_and(|value| value >= 0.98);
            }
            "rg" => {
                white_fill = operands.len() >= 3
                    && operands[..3].iter().filter_map(pdf_number).count() == 3
                    && operands[..3]
                        .iter()
                        .filter_map(pdf_number)
                        .all(|value| value >= 0.98);
            }
            "k" => {
                white_fill = operands.len() >= 4
                    && operands[..4].iter().filter_map(pdf_number).count() == 4
                    && operands[..4]
                        .iter()
                        .filter_map(pdf_number)
                        .all(|value| value <= 0.02);
            }
            "Tr" => {
                text_render_mode = operands
                    .first()
                    .and_then(|value| pdf_number(value).map(|value| value as i64))
                    .unwrap_or(0);
            }
            "Tj" | "TJ" | "'" | "\"" => {
                if white_fill || matches!(text_render_mode, 3 | 7) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn image_filters(stream: &pdf_extract::Stream) -> Vec<String> {
    let Some(filter) = stream.dict.get(b"Filter").ok() else {
        return Vec::new();
    };
    match filter {
        pdf_extract::Object::Name(name) => vec![String::from_utf8_lossy(name).to_string()],
        pdf_extract::Object::Array(values) => values
            .iter()
            .filter_map(|value| value.as_name().ok())
            .map(|name| String::from_utf8_lossy(name).to_string())
            .collect(),
        _ => Vec::new(),
    }
}

fn image_filter(stream: &pdf_extract::Stream) -> String {
    let filters = image_filters(stream);
    if filters.is_empty() {
        "raw".to_string()
    } else {
        filters.join("+")
    }
}

fn image_color_channels(document: &Document, stream: &pdf_extract::Stream) -> Option<u8> {
    let color_space = stream
        .dict
        .get(b"ColorSpace")
        .ok()
        .and_then(|value| dereference_pdf_object(document, value))?;
    let name = color_space.as_name().ok()?;
    match name {
        b"DeviceGray" => Some(1),
        b"DeviceRGB" => Some(3),
        b"DeviceCMYK" => Some(4),
        _ => None,
    }
}

fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(12 + payload.len());
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(payload);
    let mut crc_input = Vec::with_capacity(kind.len() + payload.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(payload);
    chunk.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    chunk
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in bytes {
        a = (a + *byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn zlib_store(bytes: &[u8]) -> Vec<u8> {
    let mut encoded = vec![0x78, 0x01];
    if bytes.is_empty() {
        encoded.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    } else {
        for (index, chunk) in bytes.chunks(65_535).enumerate() {
            let final_block = index == bytes.len().div_ceil(65_535) - 1;
            encoded.push(if final_block { 1 } else { 0 });
            let length = chunk.len() as u16;
            encoded.extend_from_slice(&length.to_le_bytes());
            encoded.extend_from_slice(&(!length).to_le_bytes());
            encoded.extend_from_slice(chunk);
        }
    }
    encoded.extend_from_slice(&adler32(bytes).to_be_bytes());
    encoded
}

fn encode_png(raw: &[u8], width: u32, height: u32, channels: u8, bits: u8) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || bits != 8 || !(channels == 1 || channels == 3) {
        return None;
    }
    let row_bytes = width as usize * channels as usize;
    let expected = row_bytes.checked_mul(height as usize)?;
    if raw.len() != expected {
        return None;
    }
    let mut scanlines = Vec::with_capacity((row_bytes + 1) * height as usize);
    for row in raw.chunks(row_bytes) {
        scanlines.push(0);
        scanlines.extend_from_slice(row);
    }
    let color_type = if channels == 1 { 0 } else { 2 };
    let ihdr = [
        (width >> 24) as u8,
        (width >> 16) as u8,
        (width >> 8) as u8,
        width as u8,
        (height >> 24) as u8,
        (height >> 16) as u8,
        (height >> 8) as u8,
        height as u8,
        8,
        color_type,
        0,
        0,
        0,
    ];
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    let compressed = zlib_store(&scanlines);
    png.extend_from_slice(&png_chunk(b"IDAT", &compressed));
    png.extend_from_slice(&png_chunk(b"IEND", &[]));
    Some(png)
}

fn image_payload(
    document: &Document,
    stream: &pdf_extract::Stream,
    width: Option<u32>,
    height: Option<u32>,
) -> (Vec<u8>, &'static str, &'static str, Option<String>) {
    let filters = image_filters(stream);
    let filter_label = image_filter(stream);
    let dct = filters
        .iter()
        .any(|filter| filter == "DCTDecode" || filter == "DCT");
    let jpx = filters.iter().any(|filter| filter == "JPXDecode");
    if dct {
        return (stream.content.clone(), "image/jpeg", "jpg", None);
    }
    if jpx {
        return (stream.content.clone(), "image/jp2", "jp2", None);
    }

    let bits = stream
        .dict
        .get(b"BitsPerComponent")
        .ok()
        .and_then(pdf_number)
        .map(|value| value as u8)
        .unwrap_or(8);
    if let (Some(width), Some(height), Some(channels)) =
        (width, height, image_color_channels(document, stream))
    {
        let decoded_upper_bound = u64::from(width)
            .saturating_mul(u64::from(height))
            .saturating_mul(u64::from(channels))
            .saturating_mul(u64::from(bits).max(1))
            .div_ceil(8);
        if decoded_upper_bound > MAX_EMBEDDED_ASSET_BYTES as u64 {
            return (
                stream.content.clone(),
                "application/octet-stream",
                "bin",
                Some(format!(
                    "PDF_RESOURCE_LIMIT_IMAGE_DECODE_SKIPPED: estimated decoded bytes {decoded_upper_bound} exceed {MAX_EMBEDDED_ASSET_BYTES}; encoded bytes preserved"
                )),
            );
        }
    }
    let decoded = if filters.is_empty() {
        Ok(stream.content.clone())
    } else {
        stream.decompressed_content()
    };
    let Ok(decoded) = decoded else {
        return (
            stream.content.clone(),
            "application/octet-stream",
            "bin",
            Some(format!(
                "embedded PDF image uses unsupported filter chain {filter_label}; raw bytes preserved"
            )),
        );
    };
    if let (Some(width), Some(height), Some(channels)) =
        (width, height, image_color_channels(document, stream))
    {
        if let Some(png) = encode_png(&decoded, width, height, channels, bits) {
            return (png, "image/png", "png", None);
        }
    }
    (
        decoded,
        "application/octet-stream",
        "bin",
        Some(format!(
            "embedded PDF image filter chain {filter_label} decoded but could not be wrapped as PNG; decoded bytes preserved"
        )),
    )
}

fn write_shadow_asset(root: &Path, relative_path: &str, bytes: &[u8]) -> CommandResult<()> {
    let target = root.join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    let parent = target
        .parent()
        .ok_or_else(|| format!("asset_parent_missing:{}", target.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("asset_dir_create_failed:{}", error))?;
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("asset_file_name_missing:{}", target.display()))?;
    let temporary = parent.join(format!(
        ".{file_name}.tmp-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> CommandResult<()> {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("asset_temp_create_failed:{}", error))?;
        file.write_all(bytes)
            .map_err(|error| format!("asset_temp_write_failed:{}", error))?;
        file.flush()
            .map_err(|error| format!("asset_temp_flush_failed:{}", error))?;
        file.sync_all()
            .map_err(|error| format!("asset_temp_sync_failed:{}", error))?;
        match fs::rename(&temporary, &target) {
            Ok(()) => {}
            Err(_error) if target.exists() => {
                if fs::read(&target)
                    .map(|existing| existing == bytes)
                    .unwrap_or(false)
                {
                    let _ = fs::remove_file(&temporary);
                    return Ok(());
                }
                fs::remove_file(&target)
                    .map_err(|replace_error| format!("asset_replace_failed:{}", replace_error))?;
                fs::rename(&temporary, &target)
                    .map_err(|replace_error| format!("asset_replace_failed:{}", replace_error))?;
            }
            Err(error) => return Err(format!("asset_replace_failed:{}", error)),
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn extract_pdf_visual_assets(
    document: &Document,
    source: &SourceFile,
    asset_root: Option<&Path>,
) -> CommandResult<(
    Vec<Value>,
    BTreeMap<u32, Vec<String>>,
    Vec<String>,
    PdfAssetBudget,
)> {
    let Some(asset_root) = asset_root else {
        return Ok((
            Vec::new(),
            BTreeMap::new(),
            vec!["embedded image asset bytes were not persisted because no shadow artifact root was supplied".to_string()],
            PdfAssetBudget::default(),
        ));
    };
    let mut assets: Vec<Value> = Vec::new();
    let mut page_asset_ids = BTreeMap::<u32, Vec<String>>::new();
    let mut seen_objects = BTreeMap::<(u32, u16), String>::new();
    let mut warnings = Vec::new();
    let mut budget = PdfAssetBudget::default();
    for (page_num, page_id) in document.get_pages() {
        let xobject_dictionaries = page_resource_dictionaries(document, page_id)
            .into_iter()
            .filter_map(|resources| {
                resources
                    .get(b"XObject")
                    .ok()
                    .and_then(|object| dereference_pdf_object(document, object))
                    .and_then(|object| object.as_dict().ok())
            })
            .collect::<Vec<_>>();
        for (name, object) in xobject_dictionaries
            .into_iter()
            .flat_map(|objects| objects.iter())
        {
            let Some(reference) = object.as_reference().ok() else {
                continue;
            };
            let Some(pdf_object) = document.get_object(reference).ok() else {
                continue;
            };
            let Ok(stream) = pdf_object.as_stream() else {
                continue;
            };
            let subtype = stream
                .dict
                .get(b"Subtype")
                .ok()
                .and_then(|value| dereference_pdf_object(document, value))
                .and_then(|value| value.as_name().ok())
                .map(|value| String::from_utf8_lossy(value).to_string());
            if subtype.as_deref() != Some("Image") {
                continue;
            }
            if let Some(asset_id) = seen_objects.get(&reference).cloned() {
                let page_assets = page_asset_ids
                    .entry(page_num.saturating_sub(1))
                    .or_default();
                if !page_assets.contains(&asset_id) {
                    page_assets.push(asset_id);
                }
                continue;
            }
            let width_px = stream
                .dict
                .get(b"Width")
                .ok()
                .and_then(pdf_number)
                .map(|value| value.max(0.0) as u32);
            let height_px = stream
                .dict
                .get(b"Height")
                .ok()
                .and_then(pdf_number)
                .map(|value| value.max(0.0) as u32);
            let image_pixels =
                u64::from(width_px.unwrap_or(0)).saturating_mul(u64::from(height_px.unwrap_or(0)));
            if image_pixels > MAX_IMAGE_PIXELS {
                return Err(format!(
                    "PDF_RESOURCE_LIMIT_IMAGE_PIXELS:page={page_num}:pixels={image_pixels}:max={MAX_IMAGE_PIXELS}"
                ));
            }
            budget.reserve_image_pixels(image_pixels)?;
            let (asset_bytes, mime, extension, payload_warning) =
                image_payload(document, stream, width_px, height_px);
            if asset_bytes.len() > MAX_EMBEDDED_ASSET_BYTES {
                return Err(format!(
                    "PDF_RESOURCE_LIMIT_IMAGE_BYTES:page={page_num}:bytes={}:max={MAX_EMBEDDED_ASSET_BYTES}",
                    asset_bytes.len()
                ));
            }
            if asset_bytes.is_empty() {
                budget.total_image_pixels = budget.total_image_pixels.saturating_sub(image_pixels);
                warnings.push(format!(
                    "empty embedded PDF image XObject {} on page {}",
                    String::from_utf8_lossy(name),
                    page_num
                ));
                continue;
            }
            budget.reserve_embedded_asset_bytes(asset_bytes.len() as u64)?;
            let asset_id = format!("pdf-image-p{:03}-{}-{}", page_num, reference.0, reference.1);
            seen_objects.insert(reference, asset_id.clone());
            let relative_path = format!("assets/shadow/pdf/{asset_id}.{extension}");
            write_shadow_asset(asset_root, &relative_path, &asset_bytes)?;
            if let Some(payload_warning) = payload_warning {
                warnings.push(format!(
                    "embedded PDF image {} on page {}: {payload_warning}",
                    String::from_utf8_lossy(name),
                    page_num
                ));
            }
            assets.push(json!({
                    "assetId": asset_id,
                    "kind": "raster_image",
                    "mime": mime,
                    "relativePath": relative_path,
                    "sha256": crate::hash_bytes(&asset_bytes),
                    "byteLength": asset_bytes.len() as u64,
                    "widthPx": width_px,
                    "heightPx": height_px,
                    "extractionMode": "embedded",
                    "altText": format!("Embedded PDF image XObject {} on page {}", String::from_utf8_lossy(name), page_num),
                    "decorative": false,
                    "sourceAnchor": {
                        "sourceFileId": source.file_id,
                        "pageIndex": page_num.saturating_sub(1),
                        "nodeIds": [format!("xobject-{}-{}", reference.0, reference.1)],
                        "extractionMode": "pdf_native",
                        "sourceHash": source.sha256
                    }
            }));
            let page_assets = page_asset_ids
                .entry(page_num.saturating_sub(1))
                .or_default();
            if !page_assets.contains(&asset_id) {
                page_assets.push(asset_id);
            }
        }
    }
    Ok((assets, page_asset_ids, warnings, budget))
}

fn extract_pdf_vector_assets(
    pages: &[Value],
    source: &SourceFile,
    asset_root: Option<&Path>,
) -> CommandResult<(Vec<Value>, BTreeMap<u32, Vec<String>>, Vec<String>)> {
    let Some(asset_root) = asset_root else {
        return Ok((Vec::new(), BTreeMap::new(), Vec::new()));
    };
    let mut assets = Vec::new();
    let mut page_asset_ids = BTreeMap::new();
    let warnings = Vec::new();
    for page in pages {
        let page_index = page.get("pageIndex").and_then(Value::as_u64).unwrap_or(0) as u32;
        let page_width = page
            .get("widthPt")
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .max(1.0);
        let page_height = page
            .get("heightPt")
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .max(1.0);
        let page_rotation = page.get("rotation").and_then(Value::as_u64).unwrap_or(0) as u16;
        let paths = page
            .get("vectorPaths")
            .and_then(Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter(|path| {
                        path.get("isAxisAlignedRule").and_then(Value::as_bool) != Some(true)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if paths.len() < 2 {
            continue;
        }
        let Some(bbox) = union_json_bboxes(&paths) else {
            continue;
        };
        let asset_id = format!("pdf-vector-p{:03}", page_index + 1);
        let all_paths = page
            .get("vectorPaths")
            .and_then(Value::as_array)
            .map(|values| values.iter().collect::<Vec<_>>())
            .unwrap_or_default();
        let svg = render_vector_svg(page_width, page_height, &all_paths, page, bbox);
        let bytes = svg.into_bytes();
        let relative_path = format!("assets/shadow/pdf/{asset_id}.svg");
        write_shadow_asset(asset_root, &relative_path, &bytes)?;
        let node_ids = paths
            .iter()
            .filter_map(|path| path.get("id").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assets.push(json!({
            "assetId": asset_id,
            "kind": "vector_render",
            "mime": "image/svg+xml",
            "relativePath": relative_path,
            "sha256": crate::hash_bytes(&bytes),
            "byteLength": bytes.len() as u64,
            "extractionMode": "rendered_vector",
            "altText": format!("Vector figure fallback for PDF page {}", page_index + 1),
            "decorative": false,
            "sourceAnchor": {
                "sourceFileId": source.file_id,
                "pageIndex": page_index,
                "nodeIds": node_ids,
                "bbox": rect_value(bbox.0, bbox.1, bbox.2, bbox.3, "top-left", page_rotation),
                "extractionMode": "pdf_native",
                "sourceHash": source.sha256
            }
        }));
        page_asset_ids.insert(page_index, vec![asset_id]);
    }
    Ok((assets, page_asset_ids, warnings))
}

fn union_json_bboxes(values: &[&Value]) -> Option<(f64, f64, f64, f64)> {
    let mut left = f64::INFINITY;
    let mut top = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut bottom = f64::NEG_INFINITY;
    for value in values {
        let bbox = value.get("bbox")?;
        let x = bbox.get("x").and_then(Value::as_f64)?;
        let y = bbox.get("y").and_then(Value::as_f64)?;
        let width = bbox.get("width").and_then(Value::as_f64)?.max(0.01);
        let height = bbox.get("height").and_then(Value::as_f64)?.max(0.01);
        left = left.min(x);
        top = top.min(y);
        right = right.max(x + width);
        bottom = bottom.max(y + height);
    }
    (left.is_finite() && top.is_finite() && right > left && bottom > top).then_some((
        left,
        top,
        right - left,
        bottom - top,
    ))
}

fn render_vector_svg(
    width: f64,
    height: f64,
    paths: &[&Value],
    page: &Value,
    figure_bbox: (f64, f64, f64, f64),
) -> String {
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        clean_number(width),
        clean_number(height),
        clean_number(width),
        clean_number(height)
    );
    for path in paths {
        let Some(commands) = path.get("commands").and_then(Value::as_array) else {
            continue;
        };
        let mut data = String::new();
        for command in commands {
            match command.get("op").and_then(Value::as_str) {
                Some("move") => data.push_str(&format!(
                    "M {} {} ",
                    clean_number(command.get("x").and_then(Value::as_f64).unwrap_or(0.0)),
                    clean_number(command.get("y").and_then(Value::as_f64).unwrap_or(0.0))
                )),
                Some("line") => data.push_str(&format!(
                    "L {} {} ",
                    clean_number(command.get("x").and_then(Value::as_f64).unwrap_or(0.0)),
                    clean_number(command.get("y").and_then(Value::as_f64).unwrap_or(0.0))
                )),
                Some("curve") => {
                    let points = command
                        .get("points")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_f64)
                        .map(clean_number)
                        .collect::<Vec<_>>();
                    if points.len() == 6 {
                        data.push_str(&format!(
                            "C {} {} {} {} {} {} ",
                            points[0], points[1], points[2], points[3], points[4], points[5]
                        ));
                    }
                }
                Some("close") => data.push_str("Z "),
                _ => {}
            }
        }
        if !data.is_empty() {
            let stroke = path
                .get("strokeColor")
                .and_then(Value::as_str)
                .map(|value| &value[..7.min(value.len())])
                .unwrap_or("none");
            let fill = path
                .get("fillColor")
                .and_then(Value::as_str)
                .map(|value| &value[..7.min(value.len())])
                .unwrap_or("none");
            svg.push_str(&format!(
                "<path d=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
                data.trim(),
                fill,
                stroke
            ));
        }
    }
    let figure = crate::pdf_ingest::Bounds {
        x: figure_bbox.0,
        y: figure_bbox.1,
        width: figure_bbox.2,
        height: figure_bbox.3,
    };
    for glyph in page
        .get("glyphs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(glyph_bbox) = glyph.get("bbox").and_then(|value| {
            Some(crate::pdf_ingest::Bounds {
                x: value.get("x")?.as_f64()?,
                y: value.get("y")?.as_f64()?,
                width: value.get("width")?.as_f64()?,
                height: value.get("height")?.as_f64()?,
            })
        }) else {
            continue;
        };
        if figure.intersection(glyph_bbox).is_none() {
            continue;
        }
        let text = glyph
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"{}\" fill=\"#111111\">{}</text>",
            clean_number(glyph_bbox.x),
            clean_number(glyph_bbox.bottom()),
            clean_number(glyph_bbox.height.max(6.0)),
            svg_escape(text)
        ));
    }
    svg.push_str("</svg>");
    svg
}

type PdfMatrix = [f64; 6];

fn matrix_point(matrix: PdfMatrix, point: (f64, f64)) -> (f64, f64) {
    (
        matrix[0] * point.0 + matrix[2] * point.1 + matrix[4],
        matrix[1] * point.0 + matrix[3] * point.1 + matrix[5],
    )
}

fn matrix_compose(outer: PdfMatrix, inner: PdfMatrix) -> PdfMatrix {
    [
        outer[0] * inner[0] + outer[2] * inner[1],
        outer[1] * inner[0] + outer[3] * inner[1],
        outer[0] * inner[2] + outer[2] * inner[3],
        outer[1] * inner[2] + outer[3] * inner[3],
        outer[0] * inner[4] + outer[2] * inner[5] + outer[4],
        outer[1] * inner[4] + outer[3] * inner[5] + outer[5],
    ]
}

fn pdf_text_value(object: &pdf_extract::Object) -> Option<String> {
    match object {
        pdf_extract::Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).to_string()),
        pdf_extract::Object::Name(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
        _ => None,
    }
}

fn pdf_primitive_value(object: &pdf_extract::Object) -> Option<Value> {
    match object {
        pdf_extract::Object::String(bytes, _) => {
            Some(Value::String(String::from_utf8_lossy(bytes).to_string()))
        }
        pdf_extract::Object::Name(bytes) => {
            Some(Value::String(String::from_utf8_lossy(bytes).to_string()))
        }
        pdf_extract::Object::Integer(value) => Some(json!(value)),
        pdf_extract::Object::Real(value) => Some(json!(value)),
        pdf_extract::Object::Boolean(value) => Some(json!(value)),
        pdf_extract::Object::Null => Some(Value::Null),
        pdf_extract::Object::Array(values) => Some(Value::Array(
            values.iter().filter_map(pdf_primitive_value).collect(),
        )),
        _ => None,
    }
}

fn asset_id_for_reference(assets: &[Value], reference: pdf_extract::ObjectId) -> Option<String> {
    let node_id = format!("xobject-{}-{}", reference.0, reference.1);
    assets.iter().find_map(|asset| {
        asset
            .get("sourceAnchor")?
            .get("nodeIds")?
            .as_array()?
            .iter()
            .any(|id| id.as_str() == Some(node_id.as_str()))
            .then(|| asset.get("assetId")?.as_str().map(ToString::to_string))?
    })
}

fn marked_content_property(
    document: &Document,
    operand: Option<&pdf_extract::Object>,
    key: &[u8],
) -> Option<Value> {
    let object = operand.and_then(|value| dereference_pdf_object(document, value))?;
    let dictionary = object.as_dict().ok()?;
    let value = dictionary
        .get(key)
        .ok()
        .and_then(|value| dereference_pdf_object(document, value))?;
    pdf_primitive_value(value)
}

fn collect_page_object_facts(
    document: &Document,
    source: &SourceFile,
    assets: &[Value],
    pages: &mut [Value],
) -> Vec<String> {
    let geometries = collect_page_geometries(document);
    let mut warnings = Vec::new();
    for (page_num, page_id) in document.get_pages() {
        let Some(page) = pages.iter_mut().find(|page| {
            page.get("pageIndex").and_then(Value::as_u64) == Some(page_num.saturating_sub(1) as u64)
        }) else {
            continue;
        };
        let Some(geometry) = geometries.get(&page_num) else {
            continue;
        };
        let Ok(content) = document.get_and_decode_page_content(page_id) else {
            warnings.push(format!(
                "PDF page {page_num} content stream could not be decoded for placement/marked-content facts"
            ));
            continue;
        };
        let xobjects = page_resource_dictionaries(document, page_id)
            .into_iter()
            .filter_map(|resources| {
                resources
                    .get(b"XObject")
                    .ok()
                    .and_then(|value| dereference_pdf_object(document, value))
                    .and_then(|value| value.as_dict().ok())
            })
            .collect::<Vec<_>>();
        let mut ctm: PdfMatrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let mut stack = Vec::<PdfMatrix>::new();
        let mut placements = Vec::new();
        let mut marked = Vec::new();
        for operation in content.operations {
            match operation.operator.as_str() {
                "q" => stack.push(ctm),
                "Q" => ctm = stack.pop().unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
                "cm" if operation.operands.len() >= 6 => {
                    let values = operation.operands[..6]
                        .iter()
                        .filter_map(pdf_number)
                        .collect::<Vec<_>>();
                    if values.len() == 6 {
                        ctm = matrix_compose(
                            ctm,
                            [
                                values[0], values[1], values[2], values[3], values[4], values[5],
                            ],
                        );
                    }
                }
                "Do" => {
                    let Some(name) = operation
                        .operands
                        .first()
                        .and_then(|value| value.as_name().ok())
                    else {
                        continue;
                    };
                    let Some(reference) = xobjects.iter().find_map(|objects| {
                        objects
                            .get(name)
                            .ok()
                            .and_then(|value| value.as_reference().ok())
                    }) else {
                        continue;
                    };
                    let Some(asset_id) = asset_id_for_reference(assets, reference) else {
                        continue;
                    };
                    let native_points = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
                        .map(|point| matrix_point(ctm, point));
                    let Some(native_bbox) = bounds_for_points(&native_points) else {
                        continue;
                    };
                    let display_bbox = geometry.display_bounds(native_bbox);
                    let placement_id = format!(
                        "p{:03}-image-placement-{:04}",
                        page_num,
                        placements.len() + 1
                    );
                    let display_rect = geometry.display_rect(display_bbox);
                    let native_rect = geometry.native_rect(native_bbox);
                    placements.push(json!({
                        "id": placement_id,
                        "assetId": asset_id,
                        "bbox": display_rect.clone(),
                        "nativeBBox": native_rect.clone(),
                        "objectTransform": ctm.map(clean_number),
                        "confidence": 0.90,
                        "sourceAnchor": {
                            "sourceFileId": source.file_id,
                            "pageIndex": page_num.saturating_sub(1),
                            "nodeIds": [placement_id, format!("xobject-{}-{}", reference.0, reference.1)],
                            "bbox": display_rect.clone(),
                            "nativeBBox": native_rect,
                            "displayBBox": display_rect,
                            "pdfToDisplay": geometry.pdf_to_display,
                            "extractionMode": "pdf_native",
                            "sourceHash": source.sha256
                        }
                    }));
                }
                "BMC" | "BDC" => {
                    let tag = operation.operands.first().and_then(pdf_text_value);
                    let property = operation.operands.get(1);
                    let actual_text = marked_content_property(document, property, b"ActualText")
                        .and_then(|value| value.as_str().map(ToString::to_string));
                    let alt_text = marked_content_property(document, property, b"Alt")
                        .and_then(|value| value.as_str().map(ToString::to_string));
                    let mcid = marked_content_property(document, property, b"MCID")
                        .and_then(|value| value.as_i64())
                        .map(|value| value as i32);
                    let id = format!("p{:03}-marked-{:04}", page_num, marked.len() + 1);
                    let variants = actual_text
                        .as_ref()
                        .map(|text| {
                            json!([{
                                "text": text,
                                "extractionMode": "pdf_native",
                                "confidence": 0.50,
                                "provider": "pdf-marked-content-candidate",
                                "providerVersion": "1",
                                "nodeIds": [id.clone()]
                            }])
                        })
                        .unwrap_or_else(|| json!([]));
                    marked.push(json!({
                        "id": id,
                        "mcid": mcid,
                        "tag": tag,
                        "actualText": actual_text,
                        "altText": alt_text,
                        "structurePath": [],
                        "sourceAnchor": {
                            "sourceFileId": source.file_id,
                            "pageIndex": page_num.saturating_sub(1),
                            "nodeIds": [id],
                            "extractionMode": "pdf_native",
                            "sourceHash": source.sha256,
                            "variants": variants
                        }
                    }));
                }
                _ => {}
            }
        }
        if let Some(object) = page.as_object_mut() {
            object.insert("imagePlacements".to_string(), Value::Array(placements));
            object.insert("markedContent".to_string(), Value::Array(marked));
        }
    }
    warnings
}

fn collect_pdf_annotations(
    document: &Document,
    source: &SourceFile,
    asset_root: Option<&Path>,
    assets: &mut Vec<Value>,
    pages: &mut [Value],
) -> Vec<String> {
    let mut warnings = Vec::new();
    let geometries = collect_page_geometries(document);
    for (page_num, page_id) in document.get_pages() {
        let Some(page) = pages.iter_mut().find(|page| {
            page.get("pageIndex").and_then(Value::as_u64) == Some(page_num.saturating_sub(1) as u64)
        }) else {
            continue;
        };
        let Some(page_dictionary) = document.get_dictionary(page_id).ok() else {
            continue;
        };
        let Some(annotations) = page_dictionary
            .get(b"Annots")
            .ok()
            .and_then(|value| dereference_pdf_object(document, value))
            .and_then(|value| value.as_array().ok())
        else {
            continue;
        };
        let Some(geometry) = geometries.get(&page_num) else {
            continue;
        };
        let mut page_annotations = page
            .get("annotations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for annotation in annotations {
            let Ok(object_id) = annotation.as_reference() else {
                continue;
            };
            let Some(dictionary) = document.get_dictionary(object_id).ok() else {
                continue;
            };
            let subtype = dictionary
                .get(b"Subtype")
                .ok()
                .and_then(|value| dereference_pdf_object(document, value))
                .and_then(|value| value.as_name().ok())
                .map(|value| String::from_utf8_lossy(value).to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            let Some(rectangle) = dictionary
                .get(b"Rect")
                .ok()
                .and_then(|value| dereference_pdf_object(document, value))
                .and_then(|value| value.as_array().ok())
            else {
                warnings.push(format!(
                    "PDF annotation {} on page {} has no usable Rect",
                    object_id.0, page_num
                ));
                continue;
            };
            if rectangle.len() < 4 {
                continue;
            }
            let Some(x1) = pdf_number(&rectangle[0]) else {
                continue;
            };
            let Some(y1) = pdf_number(&rectangle[1]) else {
                continue;
            };
            let Some(x2) = pdf_number(&rectangle[2]) else {
                continue;
            };
            let Some(y2) = pdf_number(&rectangle[3]) else {
                continue;
            };
            let native_bbox = crate::pdf_ingest::Bounds {
                x: x1.min(x2),
                y: y1.min(y2),
                width: (x2 - x1).abs().max(0.01),
                height: (y2 - y1).abs().max(0.01),
            };
            let display_bbox = geometry.display_bounds(native_bbox);
            let display_rect = geometry.display_rect(display_bbox);
            let native_rect = geometry.native_rect(native_bbox);
            let annotation_id = format!(
                "p{:03}-annotation-{}-{}",
                page_num, object_id.0, object_id.1
            );
            let source_anchor = json!({
                "sourceFileId": source.file_id,
                "pageIndex": page_num.saturating_sub(1),
                "nodeIds": [format!("pdf-object-{}-{}", object_id.0, object_id.1)],
                "bbox": display_rect.clone(),
                "nativeBBox": native_rect,
                "displayBBox": display_rect.clone(),
                "pdfToDisplay": geometry.pdf_to_display,
                "extractionMode": "pdf_native",
                "sourceHash": source.sha256
            });
            let field_name = annotation_inherited_object(document, object_id, b"T")
                .as_ref()
                .and_then(pdf_text_value);
            let field_type = annotation_inherited_object(document, object_id, b"FT")
                .as_ref()
                .and_then(pdf_text_value);
            let value = annotation_inherited_object(document, object_id, b"V")
                .as_ref()
                .and_then(pdf_primitive_value);
            let default_value = annotation_inherited_object(document, object_id, b"DV")
                .as_ref()
                .and_then(pdf_primitive_value);
            let field_flags = annotation_inherited_object(document, object_id, b"Ff")
                .as_ref()
                .and_then(pdf_number)
                .map(|value| form_field_flags(value as u32))
                .unwrap_or_default();
            let has_appearance = dictionary.get(b"AP").is_ok();
            let appearance_asset_id = if subtype == "Widget" && has_appearance {
                if let Some(asset_root) = asset_root {
                    let asset_id = format!(
                        "pdf-widget-appearance-p{:03}-{}-{}",
                        page_num, object_id.0, object_id.1
                    );
                    let svg = widget_appearance_fallback_svg(
                        display_bbox.width,
                        display_bbox.height,
                        field_name.as_deref(),
                        value.as_ref(),
                        object_id,
                    );
                    let relative_path = format!("assets/shadow/pdf/{asset_id}.svg");
                    if let Err(error) =
                        write_shadow_asset(asset_root, &relative_path, svg.as_bytes())
                    {
                        warnings.push(format!(
                            "PDF_WIDGET_APPEARANCE_ASSET_FAILED: page {page_num} object {} {}: {error}",
                            object_id.0, object_id.1
                        ));
                        None
                    } else {
                        assets.push(json!({
                            "assetId": asset_id,
                            "kind": "vector_render",
                            "mime": "image/svg+xml",
                            "relativePath": relative_path,
                            "sha256": crate::hash_bytes(svg.as_bytes()),
                            "byteLength": svg.len() as u64,
                            "widthPx": (display_bbox.width * 2.0).ceil().max(1.0) as u32,
                            "heightPx": (display_bbox.height * 2.0).ceil().max(1.0) as u32,
                            "extractionMode": "rendered_vector",
                            "altText": format!("PDF widget appearance fallback for {}", field_name.as_deref().unwrap_or("unnamed field")),
                            "decorative": false,
                            "sourceAnchor": source_anchor.clone()
                        }));
                        append_page_asset_ids(page, std::slice::from_ref(&asset_id));
                        warnings.push(format!(
                            "PDF_WIDGET_APPEARANCE_FALLBACK_RENDERED: page {page_num} object {} {}",
                            object_id.0, object_id.1
                        ));
                        Some(asset_id)
                    }
                } else {
                    warnings.push(format!(
                        "PDF_WIDGET_APPEARANCE_RETAINED_NOT_PERSISTED: page {page_num} object {} {}",
                        object_id.0, object_id.1
                    ));
                    None
                }
            } else {
                None
            };
            page_annotations.push(json!({
                "id": annotation_id,
                "subtype": subtype,
                "bbox": display_rect,
                "fieldName": field_name,
                "fieldType": field_type,
                "value": value,
                "defaultValue": default_value,
                "flags": field_flags,
                "appearanceAssetId": appearance_asset_id,
                "confidence": if subtype == "Widget" { 0.92 } else { 0.72 },
                "sourceAnchor": source_anchor
            }));
        }
        if !page_annotations.is_empty() {
            warnings.push(format!(
                "PDF page {} contains {} annotation/widget object(s) retained in shadow",
                page_num,
                page_annotations.len()
            ));
            if let Some(object) = page.as_object_mut() {
                object.insert("annotations".to_string(), Value::Array(page_annotations));
            }
        }
    }
    warnings
}

fn widget_appearance_fallback_svg(
    width: f64,
    height: f64,
    field_name: Option<&str>,
    value: Option<&Value>,
    object_id: pdf_extract::ObjectId,
) -> String {
    let width = width.max(1.0);
    let height = height.max(1.0);
    let label = value
        .and_then(|value| value.as_str())
        .or(field_name)
        .unwrap_or("form field");
    let font_size = (height * 0.45).clamp(6.0, 18.0);
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.3}\" height=\"{height:.3}\" viewBox=\"0 0 {width:.3} {height:.3}\" role=\"img\" aria-label=\"PDF widget appearance fallback\" data-pdf-object=\"{}-{}\"><rect x=\"0.5\" y=\"0.5\" width=\"{:.3}\" height=\"{:.3}\" fill=\"#FFFFFF\" stroke=\"#111111\"/><text x=\"4\" y=\"{:.3}\" font-family=\"sans-serif\" font-size=\"{font_size:.3}\" fill=\"#111111\">{}</text></svg>",
        object_id.0,
        object_id.1,
        (width - 1.0).max(0.0),
        (height - 1.0).max(0.0),
        (height * 0.68).max(font_size),
        svg_escape(label)
    )
}

fn annotation_inherited_object(
    document: &Document,
    object_id: pdf_extract::ObjectId,
    key: &[u8],
) -> Option<pdf_extract::Object> {
    let mut current = object_id;
    for _ in 0..32 {
        let dictionary = document.get_dictionary(current).ok()?;
        if let Ok(value) = dictionary.get(key) {
            return dereference_pdf_object(document, value).cloned();
        }
        current = dictionary.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn form_field_flags(flags: u32) -> Vec<String> {
    [
        (0, "read_only"),
        (1, "required"),
        (2, "no_export"),
        (12, "multiline"),
        (13, "password"),
        (14, "no_toggle_to_off"),
        (15, "radio"),
        (16, "push_button"),
        (17, "combo"),
        (18, "edit"),
        (19, "sort"),
        (20, "file_select"),
        (21, "multi_select"),
    ]
    .into_iter()
    .filter_map(|(bit, name)| (flags & (1 << bit) != 0).then(|| name.to_string()))
    .collect()
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
    let (svg, glyph_count, line_count) = crate::pdf_ingest::render_overlay(&shadow);
    let overlay_path = job_dir.join("debug").join(SHADOW_OVERLAY_FILE);
    write_text(&overlay_path, &svg)?;
    Ok(json!({
        "shadowPath": shadow_path.to_string_lossy(),
        "overlayPath": overlay_path.to_string_lossy(),
        "pageCount": pages.len(),
        "glyphCount": glyph_count,
        "lineCount": line_count,
        "layers": ["source", "glyphs", "lines", "regions", "reading-order", "tables-assets", "ocr", "unassigned"]
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
        / (page.geometry.display_width * page.geometry.display_height))
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
            page.geometry.display_width,
            page.geometry.display_height,
            "top-left",
            page.geometry.rotation,
        )]
    } else {
        Vec::new()
    };
    let glyph_values = page
        .glyphs
        .iter()
        .map(|glyph| {
            let mut value = glyph.value.clone();
            if glyph.source_line_break_after {
                value
                    .as_object_mut()
                    .expect("glyph facts are always objects")
                    .insert("_sourceLineBreakAfter".to_string(), Value::Bool(true));
            }
            value
        })
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
        "widthPt": clean_number(page.geometry.display_width),
        "heightPt": clean_number(page.geometry.display_height),
        "rotation": page.geometry.rotation,
        "mediaBox": page.geometry.native_rect(page.geometry.media_box),
        "cropBox": page.geometry.native_rect(page.geometry.crop_box),
        "pageTransform": page.geometry.page_transform_value(),
        "glyphs": glyph_values,
        "spans": [],
        "lines": [],
        "regions": [],
        "vectorPaths": page.vector_paths,
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
            "rotationConfidence": if matches!(page.geometry.rotation, 0 | 90 | 180 | 270) {1.0} else {0.0},
            "requiresOcrRegions": requires_ocr_regions,
            "warnings": warnings
        },
        "_coverage": coverage
    })
}

fn load_document_with_xref_repair(input_path: &Path) -> CommandResult<(Document, Vec<String>)> {
    let byte_length = fs::metadata(input_path)
        .map_err(|error| format!("pdf_facts_shadow_metadata_failed:{}", error))?
        .len();
    enforce_pdf_byte_limit(byte_length)?;
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

fn enforce_pdf_byte_limit(byte_length: u64) -> CommandResult<()> {
    if byte_length > MAX_PDF_BYTES {
        return Err(format!(
            "PDF_RESOURCE_LIMIT_BYTES:{byte_length}>{MAX_PDF_BYTES}"
        ));
    }
    Ok(())
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
    let mut entries_by_id = BTreeMap::<String, Value>::new();
    for page in pages {
        for entry in page
            .get("_coverage")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = entry.get("sourceNodeId").and_then(Value::as_str) {
                entries_by_id
                    .entry(id.to_string())
                    .or_insert_with(|| entry.clone());
            }
        }
        for (collection, reason) in [
            (
                "glyphs",
                "native or OCR glyph fact retained; semantic assignment is deferred",
            ),
            (
                "spans",
                "physical text span retained; semantic assignment is deferred",
            ),
            (
                "lines",
                "physical line retained; semantic assignment is deferred",
            ),
            (
                "regions",
                "physical region retained; semantic assignment is deferred",
            ),
            (
                "vectorPaths",
                "vector path retained; visual role is deferred",
            ),
            (
                "tables",
                "table candidate retained; semantic cell review is deferred",
            ),
            (
                "annotations",
                "PDF annotation/widget retained; form or interaction assignment is deferred",
            ),
            (
                "imagePlacements",
                "image placement and object transform retained; semantic assignment is deferred",
            ),
            (
                "markedContent",
                "PDF marked-content evidence retained as a candidate, not unconditional text truth",
            ),
        ] {
            if let Some(items) = page.get(collection).and_then(Value::as_array) {
                for item in items {
                    let Some(id) = item.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    entries_by_id.entry(id.to_string()).or_insert_with(|| {
                        json!({
                            "sourceNodeId": id,
                            "disposition": "unassigned",
                            "targetIds": [],
                            "reason": reason
                        })
                    });
                    if collection == "tables" {
                        for cell in item
                            .get("cells")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            let Some(cell_id) = cell.get("cellId").and_then(Value::as_str) else {
                                continue;
                            };
                            entries_by_id.entry(cell_id.to_string()).or_insert_with(|| {
                                json!({
                                    "sourceNodeId": cell_id,
                                    "disposition": "unassigned",
                                    "targetIds": [],
                                    "reason": "physical table cell retained; semantic cell assignment is deferred"
                                })
                            });
                        }
                    }
                }
            }
        }
    }
    entries_by_id.into_values().collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_store::make_job;
    use crate::{CreateJobInput, SourceFile};
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::env;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn fixture_path(relative_path: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(relative_path)
    }

    fn fixture_source_named(
        relative_path: &str,
        file_id: &str,
    ) -> (ImportJob, SourceFile, PathBuf) {
        let mut job = make_job(CreateJobInput {
            title: Some("PDF facts shadow fixture".to_string()),
            category: Some("phase2".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["phase2-pr03-pr04".to_string()]),
            llm_profile_id: None,
        });
        let path = fixture_path(relative_path);
        let bytes = fs::read(&path).expect("PDF facts fixture must exist");
        let original_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("fixture.pdf")
            .to_string();
        let source = SourceFile {
            file_id: file_id.to_string(),
            original_name: original_name.clone(),
            stored_name: original_name,
            file_type: "pdf".to_string(),
            sha256: crate::hash_bytes(&bytes),
            size_bytes: bytes.len() as u64,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        };
        job.source_files = vec![source.clone()];
        (job, source, path)
    }

    fn fixture_source() -> (ImportJob, SourceFile) {
        let (job, source, _) =
            fixture_source_named("fixtures/parser/complex-reading.pdf", "file-pr02");
        (job, source)
    }

    fn widget_pdf_bytes() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [5 0 R] >>",
            "<< /Length 0 >>\nstream\n\nendstream",
            "<< /Type /Annot /Subtype /Widget /Rect [72 700 144 720] /FT /Tx /T (answer) /V (42) /AP << /N 6 0 R >> >>",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 72 20] /Length 0 >>\nstream\n\nendstream",
        ];
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0usize];
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes
                .extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
        }
        let xref_offset = bytes.len();
        bytes.extend_from_slice(format!("xref\n0 {}\n", offsets.len()).as_bytes());
        bytes.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        bytes
    }

    fn temporary_pdf_source(
        bytes: &[u8],
        file_id: &str,
        original_name: &str,
    ) -> (ImportJob, SourceFile, PathBuf) {
        let mut job = make_job(CreateJobInput {
            title: Some("PDF facts shadow temporary fixture".to_string()),
            category: Some("phase2".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["phase2-pr04".to_string()]),
            llm_profile_id: None,
        });
        let source = SourceFile {
            file_id: file_id.to_string(),
            original_name: original_name.to_string(),
            stored_name: original_name.to_string(),
            file_type: "pdf".to_string(),
            sha256: crate::hash_bytes(bytes),
            size_bytes: bytes.len() as u64,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        };
        job.source_files = vec![source.clone()];
        let path = env::temp_dir().join(format!("{file_id}.pdf"));
        fs::write(&path, bytes).unwrap();
        (job, source, path)
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
        assert_eq!(
            value["parser"]["options"]["preflight"]["pageCount"].as_u64(),
            Some(value["pages"].as_array().unwrap().len() as u64)
        );
        assert_eq!(
            value["parser"]["options"]["preflight"]["hasJavaScript"].as_bool(),
            Some(false)
        );
        assert!(value["parser"]["options"]["preflight"]["pageReports"]
            .as_array()
            .is_some_and(|reports| !reports.is_empty()));
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
        for layer in [
            "source",
            "glyphs",
            "lines",
            "regions",
            "reading-order",
            "tables-assets",
            "ocr",
            "unassigned",
        ] {
            assert!(overlay.contains(&format!("data-layer=\"{layer}\"")));
        }
        assert_eq!(result["layers"].as_array().unwrap().len(), 8);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn phase2_physical_layers_keep_line_span_region_and_anchor_integrity() {
        let (job, source, input) =
            fixture_source_named("fixtures/parser/complex-reading.pdf", "file-phase2-complex");
        let value = extract_pdf_facts_shadow(&job, &source, &input).unwrap();
        let typed = serde_json::from_value::<DocumentIRV2>(value.clone()).unwrap();
        assert!(typed.is_supported_schema_version());

        let page = &value["pages"][0];
        let glyphs = page["glyphs"].as_array().unwrap();
        let spans = page["spans"].as_array().unwrap();
        let lines = page["lines"].as_array().unwrap();
        let regions = page["regions"].as_array().unwrap();
        assert!(!glyphs.is_empty());
        assert!(!spans.is_empty());
        assert!(!lines.is_empty());
        assert!(!regions.is_empty());

        let span_ids = spans
            .iter()
            .filter_map(|span| span["id"].as_str())
            .collect::<BTreeSet<_>>();
        for line in lines {
            assert!(line.get("spans").is_none());
            assert!(!line["spanIds"].as_array().unwrap().is_empty());
            assert!(!line["sourceAnchors"].as_array().unwrap().is_empty());
            for span_id in line["spanIds"].as_array().unwrap() {
                assert!(span_ids.contains(span_id.as_str().unwrap()));
            }
        }
        for span in spans {
            assert!(!span["glyphIds"].as_array().unwrap().is_empty());
            assert!(!span["sourceAnchors"].as_array().unwrap().is_empty());
        }
        for region in regions {
            assert!(!region["childLineIds"].as_array().unwrap().is_empty());
            assert!(!region["sourceAnchors"].as_array().unwrap().is_empty());
        }
        let reading_order = page["readingOrder"].as_array().unwrap();
        assert_eq!(reading_order.len(), regions.len());
        assert_eq!(
            reading_order
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
                .len(),
            regions.len()
        );
        assert_eq!(
            value["parser"]["options"]["physicalLayers"]["summary"]["lineCount"].as_u64(),
            Some(lines.len() as u64)
        );
        assert_eq!(
            value["parser"]["options"]["ocrPlan"]["nativeTextOverwrite"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn all_synthetic_pdf_fixtures_round_trip_physical_layers_without_schema_loss() {
        let fixtures = [
            "pdf-borderless-table.pdf",
            "pdf-broken-font.pdf",
            "pdf-flowchart.pdf",
            "pdf-header-footer.pdf",
            "pdf-hidden-ocr.pdf",
            "pdf-image-only.pdf",
            "pdf-map-hotspot.pdf",
            "pdf-mixed-text-image.pdf",
            "pdf-native-ocr-conflict.pdf",
            "pdf-question-before-passage.pdf",
            "pdf-rotated-page.pdf",
            "pdf-ruled-table.pdf",
            "pdf-three-column.pdf",
            "pdf-two-column.pdf",
            "pdf-vector-diagram.pdf",
        ];
        for (index, file_name) in fixtures.iter().enumerate() {
            let relative = format!("fixtures/golden/synthetic/pdf/{file_name}");
            let file_id = format!("file-phase2-all-{index}");
            let (job, source, input) = fixture_source_named(&relative, &file_id);
            let value = extract_pdf_facts_shadow(&job, &source, &input)
                .unwrap_or_else(|error| panic!("{file_name}: {error}"));
            let typed = serde_json::from_value::<DocumentIRV2>(value.clone())
                .unwrap_or_else(|error| panic!("{file_name}: {error}"));
            assert!(typed.is_supported_schema_version());
            let pages = value["pages"].as_array().unwrap();
            assert!(!pages.is_empty(), "{file_name}: no pages");
            let total_line_count = pages
                .iter()
                .filter_map(|page| page["lines"].as_array())
                .map(Vec::len)
                .sum::<usize>();
            assert_eq!(
                value["parser"]["options"]["physicalLayers"]["summary"]["lineCount"].as_u64(),
                Some(total_line_count as u64),
                "{file_name}: physical summary should match all pages"
            );
            for page in pages {
                let glyphs = page["glyphs"].as_array().unwrap();
                let spans = page["spans"].as_array().unwrap();
                let lines = page["lines"].as_array().unwrap();
                let regions = page["regions"].as_array().unwrap();
                if !glyphs.is_empty() {
                    assert!(
                        !lines.is_empty(),
                        "{file_name}: glyphs collapsed to no lines"
                    );
                    assert!(
                        !spans.is_empty(),
                        "{file_name}: glyphs collapsed to no spans"
                    );
                }
                let region_ids = regions
                    .iter()
                    .filter_map(|region| region["id"].as_str())
                    .collect::<BTreeSet<_>>();
                let order_ids = page["readingOrder"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>();
                assert_eq!(order_ids, region_ids, "{file_name}: reading order mismatch");
            }
            if *file_name == "pdf-image-only.pdf" {
                assert_eq!(
                    value["parser"]["options"]["ocrPlan"]["plans"][0]["mode"].as_str(),
                    Some("full_page")
                );
            }
        }
    }

    #[test]
    fn available_private_real_pdf_corpus_preserves_physical_layer_invariants() {
        let manifest_path = fixture_path("fixtures/golden/manifest.json");
        let manifest: Value = serde_json::from_slice(
            &fs::read(&manifest_path).expect("golden corpus manifest should be readable"),
        )
        .expect("golden corpus manifest should be valid JSON");
        let inputs = manifest["requiredPrivateCorpus"]
            .as_array()
            .expect("manifest should select the authoritative private corpus")
            .iter()
            .map(|fixture| {
                let source_path = fixture["sourcePath"]
                    .as_str()
                    .expect("authoritative private fixture should declare sourcePath");
                fixture_path(source_path)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            inputs.len(),
            8,
            "manifest should select exactly eight authoritative private PDFs"
        );
        for (index, input) in inputs.iter().enumerate() {
            let relative = input
                .strip_prefix(fixture_path(""))
                .expect("private fixture should stay under workspace")
                .to_string_lossy()
                .replace('\\', "/");
            let (job, source, input) =
                fixture_source_named(&relative, &format!("file-phase2-private-{index}"));
            let value = extract_pdf_facts_shadow(&job, &source, &input)
                .unwrap_or_else(|error| panic!("{}: {error}", source.original_name));
            let typed = serde_json::from_value::<DocumentIRV2>(value.clone())
                .unwrap_or_else(|error| panic!("{}: {error}", source.original_name));
            assert!(typed.is_supported_schema_version());
            let pages = value["pages"].as_array().unwrap();
            assert!(!pages.is_empty(), "{}: no pages", source.original_name);
            let total_line_count = pages
                .iter()
                .filter_map(|page| page["lines"].as_array())
                .map(Vec::len)
                .sum::<usize>();
            assert_eq!(
                value["parser"]["options"]["physicalLayers"]["summary"]["lineCount"].as_u64(),
                Some(total_line_count as u64),
                "{}: line summary mismatch",
                source.original_name
            );
            for page in pages {
                let glyphs = page["glyphs"].as_array().unwrap();
                let has_content_glyph = glyphs.iter().any(|glyph| {
                    glyph["text"]
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty())
                });
                if !glyphs.is_empty() {
                    assert!(
                        !page["lines"].as_array().unwrap().is_empty(),
                        "{} page {}: glyphs collapsed to no lines",
                        source.original_name,
                        page["pageIndex"]
                    );
                    if has_content_glyph {
                        assert!(
                            !page["spans"].as_array().unwrap().is_empty(),
                            "{} page {}: content glyphs collapsed to no spans",
                            source.original_name,
                            page["pageIndex"]
                        );
                    }
                }
                let region_ids = page["regions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|region| region["id"].as_str())
                    .collect::<BTreeSet<_>>();
                let order_ids = page["readingOrder"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>();
                assert_eq!(
                    region_ids, order_ids,
                    "{}: order mismatch",
                    source.original_name
                );
            }
        }
    }

    fn assert_column_fixture(file_name: &str, minimum_columns: usize) {
        let relative = format!("fixtures/golden/synthetic/pdf/{file_name}");
        let (job, source, input) = fixture_source_named(&relative, "file-phase2-columns");
        let value = extract_pdf_facts_shadow(&job, &source, &input).unwrap();
        let page = &value["pages"][0];
        let regions = page["regions"].as_array().unwrap();
        let columns = regions
            .iter()
            .filter_map(|region| region["columnIndex"].as_u64())
            .collect::<BTreeSet<_>>();
        assert!(
            columns.len() >= minimum_columns,
            "{file_name} detected columns: {columns:?}"
        );
        let order = page["readingOrder"].as_array().unwrap();
        assert_eq!(order.len(), regions.len());
        let region_ids = regions
            .iter()
            .filter_map(|region| region["id"].as_str())
            .collect::<BTreeSet<_>>();
        let ordered_ids = order
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(ordered_ids, region_ids);
        assert!(page["lines"].as_array().unwrap().len() >= 4);
    }

    #[test]
    fn synthetic_two_and_three_column_reading_order_is_geometry_aware() {
        assert_column_fixture("pdf-two-column.pdf", 2);
        assert_column_fixture("pdf-three-column.pdf", 3);
    }

    #[test]
    fn ruled_and_borderless_table_candidates_are_shadow_only_and_anchored() {
        let (ruled_job, ruled_source, ruled_input) = fixture_source_named(
            "fixtures/golden/synthetic/pdf/pdf-ruled-table.pdf",
            "file-phase2-ruled-table",
        );
        let ruled = extract_pdf_facts_shadow(&ruled_job, &ruled_source, &ruled_input).unwrap();
        let ruled_page = &ruled["pages"][0];
        assert!(!ruled_page["vectorPaths"].as_array().unwrap().is_empty());
        let ruled_tables = ruled_page["tables"].as_array().unwrap();
        assert!(!ruled_tables.is_empty(), "ruled table candidate missing");
        assert_eq!(
            ruled_tables[0]["detectionMode"].as_str(),
            Some("ruling_lines")
        );
        assert!(!ruled_tables[0]["cells"].as_array().unwrap().is_empty());
        for path in ruled_page["vectorPaths"].as_array().unwrap() {
            assert!(path["sourceAnchor"].is_object());
        }

        let (borderless_job, borderless_source, borderless_input) = fixture_source_named(
            "fixtures/golden/synthetic/pdf/pdf-borderless-table.pdf",
            "file-phase2-borderless-table",
        );
        let borderless =
            extract_pdf_facts_shadow(&borderless_job, &borderless_source, &borderless_input)
                .unwrap();
        let borderless_tables = borderless["pages"][0]["tables"].as_array().unwrap();
        assert!(
            !borderless_tables.is_empty(),
            "borderless table candidate missing"
        );
        assert_eq!(
            borderless_tables[0]["detectionMode"].as_str(),
            Some("text_alignment")
        );
    }

    #[test]
    fn hidden_ocr_conflicts_are_reported_without_native_text_overwrite() {
        for file_name in ["pdf-hidden-ocr.pdf", "pdf-native-ocr-conflict.pdf"] {
            let relative = format!("fixtures/golden/synthetic/pdf/{file_name}");
            let (job, source, input) = fixture_source_named(&relative, "file-phase2-ocr");
            let value = extract_pdf_facts_shadow(&job, &source, &input).unwrap();
            let page = &value["pages"][0];
            let warnings = page["quality"]["warnings"].as_array().unwrap();
            assert!(
                warnings.iter().any(|warning| {
                    warning.as_str() == Some("PDF_HIDDEN_TEXT_MISALIGNED")
                        || warning.as_str() == Some("PDF_NATIVE_OCR_CONFLICT")
                }),
                "{file_name} did not report an OCR/native mismatch: {warnings:?}"
            );
            assert_eq!(
                value["parser"]["options"]["ocrPlan"]["nativeTextOverwrite"].as_bool(),
                Some(false)
            );
        }
    }

    #[test]
    fn embedded_images_are_written_to_shadow_asset_store_with_integrity_metadata() {
        let root = env::temp_dir().join(format!("phase2-assets-{}", Uuid::new_v4().simple()));
        crate::util::ensure_app_dirs(&root).unwrap();
        let (job, source, input) = fixture_source_named(
            "fixtures/golden/synthetic/pdf/pdf-image-only.pdf",
            "file-phase2-image",
        );
        crate::job_store::save_job(&root, &job).unwrap();
        let job_dir = crate::util::job_dir(&root, &job.job_id);
        crate::util::ensure_job_dirs(&job_dir).unwrap();
        let output = job_dir.join(SHADOW_ARTIFACT_FILE);
        let value = write_pdf_facts_shadow(&job, &source, &input, &output).unwrap();
        let assets = value["assets"].as_array().unwrap();
        assert!(!assets.is_empty());
        assert_eq!(assets[0]["mime"].as_str(), Some("image/png"));
        let page = &value["pages"][0];
        let placement_area = page["imagePlacements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|placement| {
                placement["bbox"]["width"].as_f64().unwrap()
                    * placement["bbox"]["height"].as_f64().unwrap()
            })
            .sum::<f64>();
        let expected_coverage = placement_area
            / (page["widthPt"].as_f64().unwrap() * page["heightPt"].as_f64().unwrap());
        let reported_coverage = page["quality"]["imageCoverageRatio"].as_f64().unwrap();
        assert!((reported_coverage - expected_coverage).abs() < 1e-6);
        assert!((0.20..1.0).contains(&reported_coverage));
        assert_eq!(
            value["pages"][0]["quality"]["classification"].as_str(),
            Some("scanned")
        );
        let asset_id = assets[0]["assetId"].as_str().unwrap();
        assert!(value["pages"][0]["assetIds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id.as_str() == Some(asset_id)));
        let relative_path = assets[0]["relativePath"].as_str().unwrap();
        let asset_path = job_dir.join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let bytes = fs::read(&asset_path).unwrap();
        assert_eq!(
            bytes.len() as u64,
            assets[0]["byteLength"].as_u64().unwrap()
        );
        assert_eq!(
            crate::hash_bytes(&bytes),
            assets[0]["sha256"].as_str().unwrap()
        );
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn indirect_page_resources_preserve_real_chili_passage_and_answer_images() {
        let root = env::temp_dir().join(format!(
            "phase4-chili-indirect-assets-{}",
            Uuid::new_v4().simple()
        ));
        crate::util::ensure_app_dirs(&root).unwrap();
        let (job, source, input) = fixture_source_named(
            "fixtures/golden/private-real/chili-peppers.pdf",
            "file-phase4-chili-images",
        );
        crate::job_store::save_job(&root, &job).unwrap();
        let job_dir = crate::util::job_dir(&root, &job.job_id);
        crate::util::ensure_job_dirs(&job_dir).unwrap();
        let output = job_dir.join(SHADOW_ARTIFACT_FILE);
        let value = write_pdf_facts_shadow(&job, &source, &input, &output).unwrap();
        let assets = value["assets"].as_array().unwrap();
        assert_eq!(assets.len(), 2, "expected passage and answer-page rasters");
        let asset_pages = assets
            .iter()
            .filter_map(|asset| {
                asset
                    .pointer("/sourceAnchor/pageIndex")
                    .and_then(Value::as_u64)
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(asset_pages, BTreeSet::from([0, 4]));
        for asset in assets {
            let relative_path = asset["relativePath"].as_str().unwrap();
            let asset_path =
                job_dir.join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let bytes = fs::read(asset_path).unwrap();
            assert_eq!(asset["byteLength"].as_u64(), Some(bytes.len() as u64));
            assert_eq!(
                asset["sha256"].as_str(),
                Some(crate::hash_bytes(&bytes).as_str())
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn real_organisational_image_only_answer_pages_route_to_full_page_ocr_plan() {
        let root = env::temp_dir().join(format!(
            "phase4-organisational-image-only-{}",
            Uuid::new_v4().simple()
        ));
        crate::util::ensure_app_dirs(&root).unwrap();
        let (job, source, input) = fixture_source_named(
            "fixtures/golden/private-real/organisational-design.pdf",
            "file-phase4-organisational-image-only",
        );
        crate::job_store::save_job(&root, &job).unwrap();
        let job_dir = crate::util::job_dir(&root, &job.job_id);
        crate::util::ensure_job_dirs(&job_dir).unwrap();
        let output = job_dir.join(SHADOW_ARTIFACT_FILE);
        let value = write_pdf_facts_shadow(&job, &source, &input, &output).unwrap();

        let assets = value["assets"].as_array().unwrap();
        let answer_asset_pages = assets
            .iter()
            .filter_map(|asset| {
                asset
                    .pointer("/sourceAnchor/pageIndex")
                    .and_then(Value::as_u64)
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(answer_asset_pages, BTreeSet::from([6, 7]));
        for page_index in [6_u64, 7] {
            let page = value["pages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|page| page["pageIndex"].as_u64() == Some(page_index))
                .unwrap();
            assert_eq!(page["quality"]["classification"].as_str(), Some("scanned"));
            assert!(page["quality"]["nativeCharacterCount"].as_u64().unwrap() <= 4);
            assert!(page["quality"]["imageCoverageRatio"].as_f64().unwrap() >= 0.15);
            assert!(!page["quality"]["requiresOcrRegions"]
                .as_array()
                .unwrap()
                .is_empty());
            assert!(page["quality"]["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|warning| warning.as_str() == Some("PDF_IMAGE_ONLY_PAGE_REQUIRES_OCR")));
            let plan = value["parser"]["options"]["ocrPlan"]["plans"]
                .as_array()
                .unwrap()
                .iter()
                .find(|plan| plan["pageIndex"].as_u64() == Some(page_index))
                .unwrap();
            assert_eq!(plan["mode"].as_str(), Some("full_page"));
            assert_eq!(plan["nativeTextPreserved"].as_bool(), Some(true));
            assert_eq!(plan["llmRepairEnabled"].as_bool(), Some(false));
        }
        assert_eq!(
            value["parser"]["options"]["ocrPlan"]["engine"].as_str(),
            Some("not_configured")
        );
        assert!(value.get("answerKey").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mixed_native_text_and_raster_routes_only_image_regions_without_performing_ocr() {
        let root = env::temp_dir().join(format!("phase2-mixed-ocr-{}", Uuid::new_v4().simple()));
        crate::util::ensure_app_dirs(&root).unwrap();
        let (job, source, input) = fixture_source_named(
            "fixtures/golden/synthetic/pdf/pdf-mixed-text-image.pdf",
            "file-phase2-mixed-ocr",
        );
        crate::job_store::save_job(&root, &job).unwrap();
        let job_dir = crate::util::job_dir(&root, &job.job_id);
        crate::util::ensure_job_dirs(&job_dir).unwrap();
        let output = job_dir.join(SHADOW_ARTIFACT_FILE);
        let value = write_pdf_facts_shadow(&job, &source, &input, &output).unwrap();
        let page = &value["pages"][0];
        assert_eq!(page["quality"]["classification"].as_str(), Some("mixed"));
        assert!(page["quality"]["nativeCharacterCount"].as_u64().unwrap() >= 24);
        assert!(page["quality"]["imageCoverageRatio"].as_f64().unwrap() > 0.0);
        assert!(!page["quality"]["requiresOcrRegions"]
            .as_array()
            .unwrap()
            .is_empty());
        let plan = &value["parser"]["options"]["ocrPlan"]["plans"][0];
        assert_eq!(plan["mode"].as_str(), Some("selective_region"));
        assert_eq!(plan["nativeTextPreserved"].as_bool(), Some(true));
        assert_eq!(plan["llmRepairEnabled"].as_bool(), Some(false));
        assert_eq!(
            value["parser"]["options"]["ocrPlan"]["engine"].as_str(),
            Some("not_configured")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn vector_figure_fallback_and_widget_regions_are_shadow_only_and_anchored() {
        let root = env::temp_dir().join(format!("phase2-visuals-{}", Uuid::new_v4().simple()));
        crate::util::ensure_app_dirs(&root).unwrap();
        let (vector_job, vector_source, vector_input) = fixture_source_named(
            "fixtures/golden/synthetic/pdf/pdf-vector-diagram.pdf",
            "file-phase2-vector-figure",
        );
        crate::job_store::save_job(&root, &vector_job).unwrap();
        let vector_job_dir = crate::util::job_dir(&root, &vector_job.job_id);
        crate::util::ensure_job_dirs(&vector_job_dir).unwrap();
        let vector_output = vector_job_dir.join(SHADOW_ARTIFACT_FILE);
        let vector_value =
            write_pdf_facts_shadow(&vector_job, &vector_source, &vector_input, &vector_output)
                .unwrap();
        let vector_asset = vector_value["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["kind"].as_str() == Some("vector_render"))
            .expect("vector figure should have a rendered fallback asset");
        let vector_asset_id = vector_asset["assetId"].as_str().unwrap();
        assert_eq!(vector_asset["mime"].as_str(), Some("image/svg+xml"));
        assert!(vector_value["pages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|page| {
                page["assetIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|asset_id| asset_id.as_str() == Some(vector_asset_id))
            }));
        let vector_region = vector_value["pages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|page| page["regions"].as_array().unwrap())
            .find(|region| {
                region["kind"].as_str() == Some("diagram")
                    && region["childObjectIds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|object_id| object_id.as_str() == Some(vector_asset_id))
            })
            .expect("vector asset should be represented by a diagram region");
        assert!(!vector_region["sourceAnchors"]
            .as_array()
            .unwrap()
            .is_empty());
        let vector_path = vector_job_dir.join(
            vector_asset["relativePath"]
                .as_str()
                .unwrap()
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        let vector_bytes = fs::read(vector_path).unwrap();
        assert!(vector_bytes.starts_with(b"<svg "));
        let vector_hash = crate::hash_bytes(&vector_bytes);
        assert_eq!(vector_asset["sha256"].as_str(), Some(vector_hash.as_str()));

        let (widget_job, widget_source, widget_input) =
            temporary_pdf_source(&widget_pdf_bytes(), "file-phase2-widget", "widget.pdf");
        crate::job_store::save_job(&root, &widget_job).unwrap();
        let widget_job_dir = crate::util::job_dir(&root, &widget_job.job_id);
        crate::util::ensure_job_dirs(&widget_job_dir).unwrap();
        let widget_output = widget_job_dir.join(SHADOW_ARTIFACT_FILE);
        let widget_value =
            write_pdf_facts_shadow(&widget_job, &widget_source, &widget_input, &widget_output)
                .expect("widget fixture should extract");
        let typed = serde_json::from_value::<DocumentIRV2>(widget_value.clone()).unwrap();
        assert!(typed.is_supported_schema_version());
        assert_eq!(
            widget_value["parser"]["options"]["physicalLayers"]["summary"]["annotationCount"]
                .as_u64(),
            Some(1)
        );
        let form_region = widget_value["pages"][0]["regions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|region| region["kind"].as_str() == Some("form"))
            .expect("widget should produce a form region");
        assert_eq!(form_region["childObjectIds"].as_array().unwrap().len(), 1);
        assert_eq!(form_region["sourceAnchors"].as_array().unwrap().len(), 1);
        let annotation = &widget_value["pages"][0]["annotations"][0];
        let appearance_asset_id = annotation["appearanceAssetId"]
            .as_str()
            .expect("widget AP must produce a persisted fallback asset");
        let appearance_asset = widget_value["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["assetId"].as_str() == Some(appearance_asset_id))
            .expect("appearance asset descriptor missing");
        assert_eq!(appearance_asset["mime"].as_str(), Some("image/svg+xml"));
        let appearance_path = widget_job_dir.join(
            appearance_asset["relativePath"]
                .as_str()
                .unwrap()
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        assert!(fs::read_to_string(appearance_path)
            .unwrap()
            .contains("data-pdf-object=\"5-0\""));
        assert!(widget_value["coverageLedger"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["sourceNodeId"] == form_region["childObjectIds"][0]));
        let _ = fs::remove_file(widget_input);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn compare_report_is_written_next_to_shadow_artifact_and_keeps_v1_authoritative() {
        let root = env::temp_dir().join(format!("phase2-compare-{}", Uuid::new_v4().simple()));
        crate::util::ensure_app_dirs(&root).unwrap();
        let (job, source, input) = fixture_source_named(
            "fixtures/golden/synthetic/pdf/pdf-two-column.pdf",
            "file-phase2-compare",
        );
        crate::job_store::save_job(&root, &job).unwrap();
        let job_dir = crate::util::job_dir(&root, &job.job_id);
        crate::util::ensure_job_dirs(&job_dir).unwrap();
        let output = job_dir.join(SHADOW_ARTIFACT_FILE);
        let v1 = json!({
            "schemaVersion": "DocumentIRV1",
            "pages": [{"blocks": [{"text": "V1 remains the authoring source."}]}]
        });
        let v1_before = v1.clone();
        write_pdf_facts_shadow_with_v1(&job, &source, &input, &output, Some(&v1)).unwrap();
        let report: Value = crate::util::read_json(&job_dir.join(SHADOW_COMPARE_FILE)).unwrap();
        assert_eq!(report["status"].as_str(), Some("complete"));
        assert_eq!(
            report["policy"]["v1RemainsAuthoritative"].as_bool(),
            Some(true)
        );
        assert_eq!(report["policy"]["v2EntersAuthoring"].as_bool(), Some(false));
        assert!(report["summary"]["lineCount"].as_u64().unwrap() > 0);
        assert_eq!(
            report["summary"]["sourceAnchorCoverage"].as_f64(),
            Some(1.0)
        );
        assert!(report["pages"][0]["readingOrder"].as_array().unwrap().len() > 0);
        assert_eq!(v1, v1_before);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn shadow_bundle_commit_replaces_artifacts_and_assets_as_one_staged_set() {
        let root = env::temp_dir().join(format!("phase2-bundle-{}", Uuid::new_v4().simple()));
        let staging = root.join(".pdf-shadow-txn-fixture");
        let output = root.join(SHADOW_ARTIFACT_FILE);
        fs::create_dir_all(root.join("assets").join("shadow").join("pdf")).unwrap();
        fs::create_dir_all(staging.join("assets").join("shadow").join("pdf")).unwrap();
        fs::write(&output, b"old artifact").unwrap();
        fs::write(root.join(SHADOW_COMPARE_FILE), b"old compare").unwrap();
        fs::write(
            root.join("assets")
                .join("shadow")
                .join("pdf")
                .join("old.bin"),
            b"old",
        )
        .unwrap();
        fs::write(staging.join(SHADOW_ARTIFACT_FILE), b"new artifact").unwrap();
        fs::write(staging.join(SHADOW_COMPARE_FILE), b"new compare").unwrap();
        fs::write(
            staging
                .join("assets")
                .join("shadow")
                .join("pdf")
                .join("new.bin"),
            b"new",
        )
        .unwrap();

        commit_shadow_bundle(&staging, &output).unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"new artifact");
        assert_eq!(
            fs::read(root.join(SHADOW_COMPARE_FILE)).unwrap(),
            b"new compare"
        );
        assert!(!root
            .join("assets")
            .join("shadow")
            .join("pdf")
            .join("old.bin")
            .exists());
        assert_eq!(
            fs::read(
                root.join("assets")
                    .join("shadow")
                    .join("pdf")
                    .join("new.bin")
            )
            .unwrap(),
            b"new"
        );
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".pdf-shadow-backup-")
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn shadow_bundle_mid_commit_failure_restores_previous_bytes() {
        let root = env::temp_dir().join(format!(
            "phase2-bundle-rollback-{}",
            Uuid::new_v4().simple()
        ));
        let staging = root.join(".pdf-shadow-txn-fixture");
        let output = root.join(SHADOW_ARTIFACT_FILE);
        let compare = root.join(SHADOW_COMPARE_FILE);
        let assets = root.join("assets").join("shadow").join("pdf");
        fs::create_dir_all(&assets).unwrap();
        fs::create_dir_all(staging.join("assets").join("shadow").join("pdf")).unwrap();
        fs::write(&output, b"old artifact\0\xff").unwrap();
        fs::write(&compare, b"old compare\r\n").unwrap();
        fs::write(assets.join("old.bin"), b"old asset\0\x01").unwrap();
        fs::write(staging.join(SHADOW_ARTIFACT_FILE), b"new artifact").unwrap();
        fs::write(staging.join(SHADOW_COMPARE_FILE), b"new compare").unwrap();
        fs::write(
            staging
                .join("assets")
                .join("shadow")
                .join("pdf")
                .join("new.bin"),
            b"new asset",
        )
        .unwrap();

        let error = commit_shadow_bundle_with_hook(
            &staging,
            &output,
            |index, _source, _target, _backup_root| {
                if index == 1 {
                    Err("phase2 injected commit failure".to_string())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert!(error.starts_with("pdf_shadow_commit_hook_failed:index=1:"));
        assert!(!error.contains("PDF_SHADOW_ROLLBACK_FAILED"));
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
                .starts_with(".pdf-shadow-backup-")
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn shadow_bundle_rollback_failure_is_fail_closed_and_preserves_backup_root() {
        let root = env::temp_dir().join(format!(
            "phase2-bundle-rollback-failure-{}",
            Uuid::new_v4().simple()
        ));
        let staging = root.join(".pdf-shadow-txn-fixture");
        let output = root.join(SHADOW_ARTIFACT_FILE);
        let compare = root.join(SHADOW_COMPARE_FILE);
        let assets = root.join("assets").join("shadow").join("pdf");
        fs::create_dir_all(&assets).unwrap();
        fs::create_dir_all(staging.join("assets").join("shadow").join("pdf")).unwrap();
        fs::write(&output, b"old artifact").unwrap();
        fs::write(&compare, b"old compare").unwrap();
        fs::write(assets.join("old.bin"), b"old asset").unwrap();
        fs::write(staging.join(SHADOW_ARTIFACT_FILE), b"new artifact").unwrap();
        fs::write(staging.join(SHADOW_COMPARE_FILE), b"new compare").unwrap();
        fs::write(
            staging
                .join("assets")
                .join("shadow")
                .join("pdf")
                .join("new.bin"),
            b"new asset",
        )
        .unwrap();

        let error = commit_shadow_bundle_with_hook(
            &staging,
            &output,
            |index, _source, _target, backup_root| {
                if index == 1 {
                    fs::rename(
                        backup_root.join("artifact.json"),
                        backup_root.join("artifact.preserved.json"),
                    )
                    .unwrap();
                    Err("phase2 injected rollback failure".to_string())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert!(error.contains("PDF_SHADOW_ROLLBACK_FAILED"));
        assert!(error.contains("backup_preserved="));
        let backup_root = fs::read_dir(&root)
            .unwrap()
            .find_map(|entry| {
                let path = entry.unwrap().path();
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".pdf-shadow-backup-")
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
    fn pdf_preflight_limits_fail_with_stable_codes() {
        assert!(enforce_pdf_byte_limit(MAX_PDF_BYTES).is_ok());
        assert_eq!(
            enforce_pdf_byte_limit(MAX_PDF_BYTES + 1).unwrap_err(),
            format!(
                "PDF_RESOURCE_LIMIT_BYTES:{}>{}",
                MAX_PDF_BYTES + 1,
                MAX_PDF_BYTES
            )
        );
        assert!(enforce_document_resource_limits(MAX_PDF_PAGES, MAX_PDF_OBJECTS).is_ok());
        assert!(enforce_document_resource_limits(MAX_PDF_PAGES + 1, 0)
            .unwrap_err()
            .starts_with("PDF_RESOURCE_LIMIT_PAGES:"));
        assert!(enforce_document_resource_limits(0, MAX_PDF_OBJECTS + 1)
            .unwrap_err()
            .starts_with("PDF_RESOURCE_LIMIT_OBJECTS:"));

        let mut pixel_budget = PdfAssetBudget {
            total_image_pixels: MAX_TOTAL_IMAGE_PIXELS - 1,
            ..PdfAssetBudget::default()
        };
        assert!(pixel_budget.reserve_image_pixels(1).is_ok());
        assert_eq!(pixel_budget.total_image_pixels, MAX_TOTAL_IMAGE_PIXELS);
        assert!(pixel_budget
            .reserve_image_pixels(1)
            .unwrap_err()
            .starts_with("PDF_RESOURCE_LIMIT_TOTAL_IMAGE_PIXELS:"));

        let mut byte_budget = PdfAssetBudget {
            total_embedded_asset_bytes: MAX_TOTAL_EMBEDDED_ASSET_BYTES - 1,
            ..PdfAssetBudget::default()
        };
        assert!(byte_budget.reserve_embedded_asset_bytes(1).is_ok());
        assert_eq!(
            byte_budget.total_embedded_asset_bytes,
            MAX_TOTAL_EMBEDDED_ASSET_BYTES
        );
        assert!(byte_budget
            .reserve_embedded_asset_bytes(1)
            .unwrap_err()
            .starts_with("PDF_RESOURCE_LIMIT_TOTAL_IMAGE_BYTES:"));
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

    #[test]
    fn v1_diagram_region_bridge_emits_typed_v2_asset_and_page_ownership() {
        let (job, source, input) =
            fixture_source_named("fixtures/parser/complex-reading.pdf", "diagram-source");
        let root = env::temp_dir().join(format!("epic8-diagram-v2-bridge-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let crop_path = root.join("source-crop.png");
        fs::write(&crop_path, b"source-backed-diagram-region").unwrap();

        let v1 = json!({
            "pages": [{"pageIndex": 1, "height": 842.0}],
            "assets": [{
                "assetId": "diagram-question-region-p001-q4-5",
                "pageIndex": 1,
                "path": crop_path,
                "width": 1200,
                "height": 700,
                "bbox": [36.0, 80.0, 559.0, 430.0],
                "diagramQuestionRegion": {
                    "questionRange": [4, 5],
                    "expectedNumbers": [4, 5],
                    "recoveryStatus": "ocr_required",
                    "numberClosure": false,
                    "sourceBacked": true
                }
            }]
        });

        let output = root.join(SHADOW_ARTIFACT_FILE);
        let shadow =
            write_pdf_facts_shadow_with_v1(&job, &source, &input, &output, Some(&v1)).unwrap();
        let typed: DocumentIRV2 = serde_json::from_value(shadow).unwrap();
        let asset = typed
            .assets
            .iter()
            .find(|asset| asset.asset_id == "diagram-question-region-p001-q4-5")
            .unwrap();
        let region = asset.diagram_question_region.as_ref().unwrap();
        assert_eq!(region.question_range, [4, 5]);
        assert_eq!(region.expected_numbers, vec![4, 5]);
        assert!(!region.number_closure);
        assert!(asset.source_anchor.is_some());
        assert!(typed.pages[0]
            .asset_ids
            .iter()
            .any(|id| id == &asset.asset_id));
        assert!(root.join(&asset.relative_path).exists());
        assert!(output.exists());
        let _ = fs::remove_dir_all(root);
    }
}
