//! Shadow-only physical PDF ingest helpers.
//!
//! This module deliberately stops at the physical/document layer.  It does not
//! infer IELTS task types, mutate the V1 document, or call an LLM.  The module
//! is fed by the facts collected in `pdf_facts_shadow` and writes only the V2
//! shadow representation.

mod compare_report;
mod coordinates;
mod line_builder;
mod ocr_merge;
mod ocr_router;
mod reading_order;
mod region_builder;
mod table_detector;

pub(crate) use compare_report::build_compare_report;
pub(crate) use coordinates::{bounds_for_points, collect_page_geometries, PdfPageGeometry};

use serde_json::{json, Value};
use std::collections::BTreeSet;

// A few page-number/footer glyphs commonly survive on otherwise image-only
// scans. Require both a small text body and measurable page coverage before
// treating native glyphs as semantic text for mixed-page classification.
const MIN_MEANINGFUL_NATIVE_CHARACTERS: u64 = 24;
const MIN_MEANINGFUL_TEXT_COVERAGE: f64 = 0.002;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub fn right(self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(self) -> f64 {
        self.y + self.height
    }

    pub fn center_x(self) -> f64 {
        self.x + self.width / 2.0
    }

    pub fn center_y(self) -> f64 {
        self.y + self.height / 2.0
    }

    pub fn area(self) -> f64 {
        self.width.max(0.0) * self.height.max(0.0)
    }

    pub fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self {
            x: left,
            y: top,
            width: (right - left).max(0.01),
            height: (bottom - top).max(0.01),
        }
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > left && bottom > top).then_some(Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    }

    pub fn iou(self, other: Self) -> f64 {
        let intersection = self
            .intersection(other)
            .map(|value| value.area())
            .unwrap_or(0.0);
        let union = self.area() + other.area() - intersection;
        if union <= 0.0 {
            0.0
        } else {
            intersection / union
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct PageLayerSummary {
    pub line_count: usize,
    pub region_count: usize,
    pub table_count: usize,
    pub vector_path_count: usize,
    pub visual_object_count: usize,
    pub annotation_count: usize,
    pub column_count: u32,
    pub reading_order_confidence: f64,
    pub ocr_region_count: usize,
    pub duplicate_text_ratio: f64,
    pub warnings: Vec<String>,
}

pub(super) fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

pub(super) fn bounds(value: &Value) -> Option<Bounds> {
    Some(Bounds {
        x: number(value, "x")?,
        y: number(value, "y")?,
        width: number(value, "width")?.max(0.01),
        height: number(value, "height")?.max(0.01),
    })
}

pub(super) fn bounds_of(value: &Value, key: &str) -> Option<Bounds> {
    value.get(key).and_then(bounds)
}

pub(super) fn rect(bounds: Bounds, page_rotation: u16) -> Value {
    json!({
        "x": clean(bounds.x),
        "y": clean(bounds.y),
        "width": clean(bounds.width.max(0.01)),
        "height": clean(bounds.height.max(0.01)),
        "unit": "pt",
        "origin": "top-left",
        "pageRotation": page_rotation
    })
}

pub(super) fn clean(value: f64) -> f64 {
    if value.is_finite() {
        (value * 1_000_000.0).round() / 1_000_000.0
    } else {
        0.0
    }
}

pub(super) fn source_anchors_from_children(
    lines: &[Value],
    child_line_ids: &[String],
) -> Vec<Value> {
    let child_ids = child_line_ids.iter().collect::<BTreeSet<_>>();
    lines
        .iter()
        .filter(|line| {
            line.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| child_ids.contains(&id.to_string()))
        })
        .flat_map(|line| {
            line.get("sourceAnchors")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .collect()
}

fn append_visual_regions(page: &Value, regions: &mut Vec<Value>, page_rotation: u16) -> usize {
    let mut added = 0;
    let mut seen = BTreeSet::new();
    for visual in page
        .get("_visualObjects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(object_id) = visual.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !seen.insert(object_id.to_string()) {
            continue;
        }
        let Some(bbox) = visual.get("bbox").and_then(bounds) else {
            continue;
        };
        let kind = visual
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("figure");
        let region_id = format!("region-visual-{object_id}");
        let source_anchors = visual
            .get("sourceAnchor")
            .cloned()
            .map(|anchor| vec![anchor])
            .unwrap_or_default();
        regions.push(json!({
            "id": region_id,
            "kind": kind,
            "bbox": rect(bbox, page_rotation),
            "childLineIds": [],
            "childObjectIds": [object_id],
            "confidence": visual.get("confidence").and_then(Value::as_f64).unwrap_or(0.35),
            "sourceAnchors": source_anchors
        }));
        added += 1;
    }
    added
}

fn append_annotation_regions(
    page: &Value,
    lines: &[Value],
    regions: &mut Vec<Value>,
    page_rotation: u16,
) -> usize {
    let mut added = 0;
    for annotation in page
        .get("annotations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(annotation_id) = annotation.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(annotation_bounds) = annotation.get("bbox").and_then(bounds) else {
            continue;
        };
        let child_line_ids = lines
            .iter()
            .filter_map(|line| {
                let line_bounds = bounds_of(line, "bbox")?;
                line_bounds
                    .intersection(annotation_bounds)
                    .filter(|overlap| overlap.area() >= line_bounds.area() * 0.05)
                    .and_then(|_| line.get("id").and_then(Value::as_str))
                    .map(ToString::to_string)
            })
            .collect::<Vec<_>>();
        let kind = if annotation.get("subtype").and_then(Value::as_str) == Some("Widget") {
            "form"
        } else {
            "unknown"
        };
        let region_id = format!("region-annotation-{annotation_id}");
        let source_anchors = annotation
            .get("sourceAnchor")
            .cloned()
            .map(|anchor| vec![anchor])
            .unwrap_or_default();
        regions.push(json!({
            "id": region_id,
            "kind": kind,
            "bbox": rect(annotation_bounds, page_rotation),
            "childLineIds": child_line_ids,
            "childObjectIds": [annotation_id],
            "confidence": annotation.get("confidence").and_then(Value::as_f64).unwrap_or(0.85),
            "sourceAnchors": source_anchors
        }));
        added += 1;
    }
    added
}

/// Build all physical layers for one page.  The input page already contains
/// native glyph facts and vector paths from the PR-02 collector.
pub(crate) fn enrich_page(page: &mut Value) -> PageLayerSummary {
    let page_width = number(page, "widthPt").unwrap_or(1.0).max(1.0);
    let page_height = number(page, "heightPt").unwrap_or(1.0).max(1.0);
    let page_rotation = page.get("rotation").and_then(Value::as_u64).unwrap_or(0) as u16;

    let ocr_merge = ocr_merge::merge_provider_output(page);
    let mut lines = line_builder::build_lines(page);
    if let Some(glyphs) = page.get_mut("glyphs").and_then(Value::as_array_mut) {
        for glyph in glyphs {
            if let Some(object) = glyph.as_object_mut() {
                object.remove("_sourceLineBreakAfter");
            }
        }
    }
    let spans = lines
        .iter_mut()
        .filter_map(|line| line.as_object_mut()?.remove("spans"))
        .flat_map(|value| value.as_array().cloned().unwrap_or_default())
        .collect::<Vec<_>>();
    let region_build = region_builder::build_regions(page_width, page_height, &lines);
    let mut regions = region_build.regions;
    let tables = table_detector::detect_tables(
        page,
        &lines,
        &mut regions,
        page_width,
        page_height,
        page_rotation,
    );
    let low_confidence_table_without_fallback = tables.iter().any(|table| {
        table
            .get("topologyConfidence")
            .and_then(Value::as_f64)
            .is_some_and(|confidence| confidence < 0.70)
            && table.get("visualFallbackAssetId").is_none()
    });
    let visual_object_count = append_visual_regions(page, &mut regions, page_rotation);
    let annotation_count = append_annotation_regions(page, &lines, &mut regions, page_rotation);
    let order = reading_order::apply_reading_order(
        &mut regions,
        &region_build.line_to_region,
        region_build.column_count,
        region_build.gutter_confidence,
    );
    let ocr = ocr_router::analyze(page);
    let region_count = regions.len();

    if let Some(object) = page.as_object_mut() {
        object.insert("lines".to_string(), Value::Array(lines.clone()));
        object.insert("spans".to_string(), Value::Array(spans));
        object.insert("regions".to_string(), Value::Array(regions));
        object.insert("tables".to_string(), Value::Array(tables.clone()));
        object.insert(
            "readingOrder".to_string(),
            Value::Array(order.primary.iter().cloned().map(Value::String).collect()),
        );
        object.insert(
            "readingOrderGraph".to_string(),
            json!({
                "primary": order.primary,
                "alternatives": order.alternatives,
                "edges": order.edges,
                "cycleEdgesRemoved": order.cycle_edges_removed,
                "confidence": clean(order.confidence)
            }),
        );

        if let Some(quality) = object.get_mut("quality").and_then(Value::as_object_mut) {
            quality.insert(
                "duplicateTextRatio".to_string(),
                json!(clean(ocr.duplicate_text_ratio)),
            );
            if !ocr.ocr_regions.is_empty() {
                quality.insert(
                    "requiresOcrRegions".to_string(),
                    Value::Array(ocr.ocr_regions.clone()),
                );
            }
            if let Some(warnings) = quality.get_mut("warnings").and_then(Value::as_array_mut) {
                let mut seen = warnings
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect::<BTreeSet<_>>();
                for warning in &ocr.warnings {
                    if seen.insert(warning.clone()) {
                        warnings.push(json!(warning));
                    }
                }
                if ocr_merge.conflict_count > 0 {
                    let warning = "PDF_NATIVE_OCR_CONFLICT".to_string();
                    if seen.insert(warning.clone()) {
                        warnings.push(json!(warning));
                    }
                }
                if region_build.column_count > 1 && region_build.gutter_confidence < 0.72 {
                    let warning = "PAGE_READING_ORDER_AMBIGUOUS".to_string();
                    if seen.insert(warning.clone()) {
                        warnings.push(json!(warning));
                    }
                }
                if low_confidence_table_without_fallback {
                    let warning = "TABLE_TOPOLOGY_LOW_CONFIDENCE_NO_VISUAL_FALLBACK".to_string();
                    if seen.insert(warning.clone()) {
                        warnings.push(json!(warning));
                    }
                }
            }
        }
    }

    let mut warnings = ocr.warnings;
    if ocr_merge.conflict_count > 0 {
        warnings.push("PDF_NATIVE_OCR_CONFLICT".to_string());
    }
    if region_build.column_count > 1 && region_build.gutter_confidence < 0.72 {
        warnings.push("PAGE_READING_ORDER_AMBIGUOUS".to_string());
    }
    if low_confidence_table_without_fallback {
        warnings.push("TABLE_TOPOLOGY_LOW_CONFIDENCE_NO_VISUAL_FALLBACK".to_string());
    }
    PageLayerSummary {
        line_count: lines.len(),
        region_count,
        table_count: tables.len(),
        vector_path_count: page
            .get("vectorPaths")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        visual_object_count,
        annotation_count,
        column_count: region_build.column_count,
        reading_order_confidence: region_build.gutter_confidence,
        ocr_region_count: ocr.ocr_regions.len(),
        duplicate_text_ratio: ocr.duplicate_text_ratio,
        warnings,
    }
}

pub(crate) fn append_page_asset_ids(page: &mut Value, asset_ids: &[String]) {
    let Some(object) = page.as_object_mut() else {
        return;
    };
    let mut ids = object
        .get("assetIds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(ToString::to_string))
        .collect::<BTreeSet<_>>();
    ids.extend(asset_ids.iter().cloned());
    object.insert(
        "assetIds".to_string(),
        Value::Array(ids.into_iter().map(Value::String).collect()),
    );
}

pub(crate) fn append_page_visual_objects(page: &mut Value, assets: &[Value], asset_ids: &[String]) {
    let Some(object) = page.as_object_mut() else {
        return;
    };
    let page_index = object.get("pageIndex").and_then(Value::as_u64).unwrap_or(0);
    let page_rotation = object.get("rotation").and_then(Value::as_u64).unwrap_or(0) as u16;
    let page_width = object
        .get("widthPt")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .max(1.0);
    let page_height = object
        .get("heightPt")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .max(1.0);
    let page_rect = rect(
        Bounds {
            x: 0.0,
            y: 0.0,
            width: page_width,
            height: page_height,
        },
        page_rotation,
    );
    let mut visual_objects = object
        .get("_visualObjects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut existing = visual_objects
        .iter()
        .filter_map(|value| value.get("id").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let image_placements = object
        .get("imagePlacements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for asset_id in asset_ids {
        let Some(asset) = assets
            .iter()
            .find(|asset| asset.get("assetId").and_then(Value::as_str) == Some(asset_id.as_str()))
        else {
            continue;
        };
        let placements = image_placements
            .iter()
            .filter(|placement| {
                placement.get("assetId").and_then(Value::as_str) == Some(asset_id.as_str())
            })
            .collect::<Vec<_>>();
        if !placements.is_empty() {
            for placement in placements {
                let Some(placement_id) = placement.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if !existing.insert(placement_id.to_string()) {
                    continue;
                }
                visual_objects.push(json!({
                    "id": placement_id,
                    "kind": "figure",
                    "bbox": placement.get("bbox").cloned().unwrap_or_else(|| page_rect.clone()),
                    "confidence": placement.get("confidence").and_then(Value::as_f64).unwrap_or(0.50),
                    "placementFallback": false,
                    "sourceAnchor": placement.get("sourceAnchor").cloned()
                }));
            }
            continue;
        }
        if existing.contains(asset_id) {
            continue;
        }
        let mut source_anchor = asset.get("sourceAnchor").cloned().unwrap_or_else(|| {
            json!({
                "sourceFileId": "unknown",
                "pageIndex": page_index,
                "nodeIds": [asset_id],
                "extractionMode": "pdf_native",
                "sourceHash": ""
            })
        });
        let has_precise_bbox = source_anchor.get("bbox").is_some();
        if !has_precise_bbox {
            if let Some(anchor) = source_anchor.as_object_mut() {
                anchor.insert("bbox".to_string(), page_rect.clone());
            }
        }
        let bbox = source_anchor
            .get("bbox")
            .cloned()
            .unwrap_or_else(|| page_rect.clone());
        let kind = match asset.get("kind").and_then(Value::as_str) {
            Some("vector_render") | Some("diagram") => "diagram",
            _ => "figure",
        };
        visual_objects.push(json!({
            "id": asset_id,
            "kind": kind,
            "bbox": bbox,
            "confidence": if has_precise_bbox { 0.72 } else { 0.20 },
            "placementFallback": !has_precise_bbox,
            "sourceAnchor": source_anchor
        }));
        existing.insert(asset_id.clone());
    }
    let has_raster_asset = asset_ids.iter().any(|asset_id| {
        assets.iter().any(|asset| {
            asset.get("assetId").and_then(Value::as_str) == Some(asset_id.as_str())
                && asset.get("kind").and_then(Value::as_str) == Some("raster_image")
        })
    });
    if has_raster_asset {
        let covered_area = image_placements
            .iter()
            .filter_map(|placement| bounds_of(placement, "bbox"))
            .map(Bounds::area)
            .sum::<f64>();
        let coverage = if image_placements.is_empty() {
            // An embedded raster without a recoverable `Do` placement is still
            // an image-bearing page. Conservatively route the page through the
            // disabled-by-default OCR plan instead of declaring it empty.
            1.0
        } else {
            (covered_area / (page_width * page_height)).clamp(0.0, 1.0)
        };
        if let Some(quality) = object.get_mut("quality").and_then(Value::as_object_mut) {
            let native_character_count = quality
                .get("nativeCharacterCount")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let text_coverage = quality
                .get("textCoverageRatio")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let meaningful_native_text = native_character_count >= MIN_MEANINGFUL_NATIVE_CHARACTERS
                && text_coverage >= MIN_MEANINGFUL_TEXT_COVERAGE;
            let previous_classification = quality
                .get("classification")
                .and_then(Value::as_str)
                .unwrap_or("empty");
            let classification = if !meaningful_native_text {
                "scanned"
            } else if previous_classification == "garbled" {
                "garbled"
            } else {
                "mixed"
            };
            let ocr_regions = if matches!(classification, "scanned" | "mixed") {
                let mut regions = image_placements
                    .iter()
                    .filter_map(|placement| placement.get("bbox").cloned())
                    .collect::<Vec<_>>();
                if regions.is_empty() || classification == "scanned" {
                    regions = vec![page_rect.clone()];
                }
                regions
            } else {
                Vec::new()
            };
            quality.insert("imageCoverageRatio".to_string(), json!(clean(coverage)));
            quality.insert("classification".to_string(), json!(classification));
            quality.insert("requiresOcrRegions".to_string(), Value::Array(ocr_regions));
            if let (Some(warning), Some(warnings)) = (
                match classification {
                    "scanned" => Some("PDF_IMAGE_ONLY_PAGE_REQUIRES_OCR"),
                    "mixed" => Some("PDF_MIXED_PAGE_IMAGE_REGION_REQUIRES_OCR"),
                    _ => None,
                },
                quality.get_mut("warnings").and_then(Value::as_array_mut),
            ) {
                if !warnings.iter().any(|value| value.as_str() == Some(warning)) {
                    warnings.push(json!(warning));
                }
            }
        }
    }
    if !visual_objects.is_empty() {
        object.insert("_visualObjects".to_string(), Value::Array(visual_objects));
    }
}

pub(crate) fn build_ocr_plan_summary(pages: &[Value]) -> Value {
    let page_plans = pages
        .iter()
        .map(|page| {
            let page_index = page
                .get("pageIndex")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let regions = page
                .get("quality")
                .and_then(|value| value.get("requiresOcrRegions"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let classification = page
                .get("quality")
                .and_then(|value| value.get("classification"))
                .and_then(Value::as_str)
                .unwrap_or("empty");
            json!({
                "pageIndex": page_index,
                "mode": if regions.is_empty() { "none" } else if classification == "scanned" { "full_page" } else { "selective_region" },
                "regions": regions,
                "nativeTextPreserved": true,
                "mergePolicy": "native_primary_ocr_alternative_or_additive",
                "llmRepairEnabled": false
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schemaVersion": "PdfSelectiveOcrPlanV1",
        "scope": "page_or_region",
        "plans": page_plans,
        "nativeTextOverwrite": false,
        "engine": "not_configured",
        "language": "eng",
        "dpi": 300,
        "notes": [
            "OCR is a shadow plan only in Phase 2; no OCR text is silently substituted.",
            "PDF per-question LLM repair remains disabled."
        ]
    })
}

pub(crate) fn svg_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn svg_num(value: f64) -> String {
    format!("{:.3}", clean(value))
}

fn node_bbox_for_id(page: &Value, node_id: &str) -> Option<Bounds> {
    for collection in [
        "glyphs",
        "spans",
        "lines",
        "regions",
        "vectorPaths",
        "tables",
        "imagePlacements",
        "markedContent",
        "annotations",
    ] {
        if let Some(items) = page.get(collection).and_then(Value::as_array) {
            for item in items {
                let matches = item.get("id").and_then(Value::as_str) == Some(node_id)
                    || item.get("cellId").and_then(Value::as_str) == Some(node_id);
                if matches {
                    if let Some(bbox) = bounds_of(item, "bbox") {
                        return Some(bbox);
                    }
                    if let Some(anchor_bbox) = item
                        .get("sourceAnchor")
                        .and_then(|anchor| anchor.get("bbox"))
                        .and_then(bounds)
                    {
                        return Some(anchor_bbox);
                    }
                }
                if collection == "tables" {
                    if let Some(cells) = item.get("cells").and_then(Value::as_array) {
                        if let Some(cell) = cells.iter().find(|cell| {
                            cell.get("cellId").and_then(Value::as_str) == Some(node_id)
                        }) {
                            return bounds_of(cell, "bbox");
                        }
                    }
                }
            }
        }
    }
    None
}

fn unassigned_node_ids(shadow: &Value) -> BTreeSet<String> {
    shadow
        .get("coverageLedger")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("disposition").and_then(Value::as_str) == Some("unassigned"))
        .filter_map(|entry| {
            entry
                .get("sourceNodeId")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect()
}

fn kind_color(kind: &str) -> &'static str {
    match kind {
        "title" => "#7b2cbf",
        "table" => "#2a9d8f",
        "header" | "footer" | "page_number" => "#6c757d",
        "figure" | "diagram" => "#f4a261",
        "unknown" => "#d00000",
        _ => "#457b9d",
    }
}

pub(crate) fn render_overlay(shadow: &Value) -> (String, usize, usize) {
    let pages = shadow
        .get("pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut offsets = Vec::with_capacity(pages.len());
    let mut total_height = 0.0;
    let mut max_width: f64 = 1.0;
    let unassigned_ids = unassigned_node_ids(shadow);
    for page in &pages {
        let width = page
            .get("widthPt")
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .max(1.0);
        let height = page
            .get("heightPt")
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .max(1.0);
        offsets.push((total_height, width, height));
        total_height += height + 28.0;
        max_width = max_width.max(width);
    }
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        svg_num(max_width), svg_num(total_height), svg_num(max_width), svg_num(total_height)
    );
    svg.push_str(
        "<style>text{font-family:monospace;font-size:6px}.page{fill:#fffdf7;stroke:#8a8175;stroke-width:.8}.glyph{fill:none;stroke:#d1495b;stroke-width:.45}.line{fill:none;stroke:#1d3557;stroke-width:.7}.region{fill-opacity:.06;stroke-width:.9}.order{stroke:#e76f51;stroke-width:1.2;fill:none;marker-end:url(#arrow)}.table{fill:none;stroke:#2a9d8f;stroke-width:.75}.ocr{fill:#ffbe0b;fill-opacity:.12;stroke:#fb8500;stroke-dasharray:3 2}.unassigned{fill:#d00000;fill-opacity:.08;stroke:#d00000;stroke-dasharray:2 2}.label{fill:#343a40}</style><defs><marker id=\"arrow\" markerWidth=\"6\" markerHeight=\"6\" refX=\"5\" refY=\"3\" orient=\"auto\"><path d=\"M0,0 L6,3 L0,6 z\" fill=\"#e76f51\"/></marker></defs>",
    );
    let mut glyph_count = 0usize;
    let mut line_count = 0usize;
    for (page_index, page) in pages.iter().enumerate() {
        let (offset, width, height) = offsets[page_index];
        svg.push_str(&format!(
            "<g data-page-index=\"{page_index}\" transform=\"translate(0 {})\"><g data-layer=\"source\"><rect class=\"page\" x=\"0\" y=\"0\" width=\"{}\" height=\"{}\"/></g>",
            svg_num(offset), svg_num(width), svg_num(height)
        ));
        svg.push_str("<g data-layer=\"glyphs\">");
        if let Some(glyphs) = page.get("glyphs").and_then(Value::as_array) {
            for glyph in glyphs {
                let Some(bbox) = glyph.get("bbox").and_then(bounds) else {
                    continue;
                };
                let text = svg_escape(glyph.get("text").and_then(Value::as_str).unwrap_or("�"));
                svg.push_str(&format!(
                    "<rect class=\"glyph\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/><text class=\"label\" x=\"{}\" y=\"{}\">{}</text>",
                    svg_num(bbox.x), svg_num(bbox.y), svg_num(bbox.width), svg_num(bbox.height), svg_num(bbox.x), svg_num(bbox.bottom()), text
                ));
                glyph_count += 1;
            }
        }
        svg.push_str("</g><g data-layer=\"lines\">");
        if let Some(lines) = page.get("lines").and_then(Value::as_array) {
            for (index, line) in lines.iter().enumerate() {
                let Some(bbox) = line.get("bbox").and_then(bounds) else {
                    continue;
                };
                svg.push_str(&format!(
                    "<rect class=\"line\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/><text class=\"label\" x=\"{}\" y=\"{}\">L{}</text>",
                    svg_num(bbox.x), svg_num(bbox.y), svg_num(bbox.width), svg_num(bbox.height), svg_num(bbox.x), svg_num(bbox.y.max(5.0)), index + 1
                ));
                line_count += 1;
            }
        }
        svg.push_str("</g><g data-layer=\"regions\">");
        if let Some(regions) = page.get("regions").and_then(Value::as_array) {
            for region in regions {
                let Some(bbox) = region.get("bbox").and_then(bounds) else {
                    continue;
                };
                let kind = region
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let color = kind_color(kind);
                let column = region
                    .get("columnIndex")
                    .and_then(Value::as_u64)
                    .map(|value| format!(" C{value}"))
                    .unwrap_or_default();
                svg.push_str(&format!(
                    "<rect class=\"region\" stroke=\"{}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/><text class=\"label\" x=\"{}\" y=\"{}\">{}{} </text>",
                    color, svg_num(bbox.x), svg_num(bbox.y), svg_num(bbox.width), svg_num(bbox.height), svg_num(bbox.x), svg_num(bbox.y + 6.0), svg_escape(kind), column
                ));
            }
        }
        svg.push_str("</g><g data-layer=\"reading-order\">");
        let mut region_centers = std::collections::BTreeMap::<String, (f64, f64)>::new();
        if let Some(regions) = page.get("regions").and_then(Value::as_array) {
            for region in regions {
                if let (Some(id), Some(bbox)) = (
                    region.get("id").and_then(Value::as_str),
                    region.get("bbox").and_then(bounds),
                ) {
                    region_centers.insert(id.to_string(), (bbox.center_x(), bbox.center_y()));
                }
            }
        }
        let order = page
            .get("readingOrder")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        for pair in order.windows(2) {
            if let (Some(start), Some(end)) =
                (region_centers.get(pair[0]), region_centers.get(pair[1]))
            {
                svg.push_str(&format!(
                    "<path class=\"order\" d=\"M{} {} L{} {}\"/>",
                    svg_num(start.0),
                    svg_num(start.1),
                    svg_num(end.0),
                    svg_num(end.1)
                ));
            }
        }
        for (rank, id) in order.iter().enumerate() {
            if let Some((x, y)) = region_centers.get(*id) {
                svg.push_str(&format!(
                    "<text class=\"label\" x=\"{}\" y=\"{}\">O{}</text>",
                    svg_num(*x),
                    svg_num(*y + 7.0),
                    rank + 1
                ));
            }
        }
        svg.push_str("</g><g data-layer=\"tables-assets\">");
        if let Some(paths) = page.get("vectorPaths").and_then(Value::as_array) {
            for path in paths {
                if let Some(bbox) = path.get("bbox").and_then(bounds) {
                    svg.push_str(&format!(
                        "<rect class=\"table\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                        svg_num(bbox.x),
                        svg_num(bbox.y),
                        svg_num(bbox.width),
                        svg_num(bbox.height)
                    ));
                }
            }
        }
        if let Some(tables) = page.get("tables").and_then(Value::as_array) {
            for table in tables {
                if let Some(cells) = table.get("cells").and_then(Value::as_array) {
                    for cell in cells {
                        if let Some(bbox) = cell.get("bbox").and_then(bounds) {
                            svg.push_str(&format!("<rect class=\"table\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>", svg_num(bbox.x), svg_num(bbox.y), svg_num(bbox.width), svg_num(bbox.height)));
                        }
                    }
                }
            }
        }
        svg.push_str("</g><g data-layer=\"ocr\">");
        if let Some(regions) = page
            .get("quality")
            .and_then(|quality| quality.get("requiresOcrRegions"))
            .and_then(Value::as_array)
        {
            for region in regions {
                if let Some(bbox) = bounds(region) {
                    svg.push_str(&format!(
                        "<rect class=\"ocr\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                        svg_num(bbox.x),
                        svg_num(bbox.y),
                        svg_num(bbox.width),
                        svg_num(bbox.height)
                    ));
                }
            }
        }
        svg.push_str("</g><g data-layer=\"unassigned\">");
        for node_id in &unassigned_ids {
            if let Some(bbox) = node_bbox_for_id(page, node_id) {
                svg.push_str(&format!(
                    "<rect class=\"unassigned\" data-source-node-id=\"{}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                    svg_escape(node_id),
                    svg_num(bbox.x),
                    svg_num(bbox.y),
                    svg_num(bbox.width),
                    svg_num(bbox.height)
                ));
            }
        }
        svg.push_str("</g></g>");
    }
    svg.push_str("</svg>");
    (svg, glyph_count, line_count)
}
