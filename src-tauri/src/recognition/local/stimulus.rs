//! Physical table and visual-stimulus compilation (plan §6.10).
//!
//! Two rules from §6.10 drive this module:
//!
//! 1. A table comes from `DocumentIRV2.pages[].tables[]` — real rows, real cells,
//!    real row/col spans. §6.10 explicitly forbids rebuilding a table as
//!    "one question per row", which loses spanning cells and reorders content.
//! 2. A flowchart or diagram falls back to the source crop plus a normalized-rect
//!    hotspot per slot. Reconstruction may keep being attempted elsewhere, but a
//!    failed reconstruction must never lose the question surface. When neither the
//!    reconstruction nor the crop exists, that is a blocker, not a silent gap.

use super::question_blocks::region_text;
use super::{
    QuestionBlockCandidateV1, TableCellCandidateV1, TableRowCandidateV1,
    TableStimulusCandidateV1, VisualHotspotCandidateV1, VisualStimulusCandidateV1,
};
use crate::ielts_grammar::issue_codes;
use crate::schema::common::RectV2;
use crate::schema::document_ir_v2::{PageNodeV2, TableNodeV2};
use std::collections::BTreeMap;

/// Below this the physical table's topology is not trustworthy enough to render as a
/// grid, so the crop fallback must exist instead.
const MIN_TABLE_TOPOLOGY_CONFIDENCE: f64 = 0.6;
/// Slack when deciding whether a question number belongs to a figure region.
const HOTSPOT_SLACK_PT: f64 = 4.0;

pub(super) struct StimulusBuild {
    pub tables: Vec<TableStimulusCandidateV1>,
    pub visuals: Vec<VisualStimulusCandidateV1>,
}

/// Compiles the page's stimulus: physical tables, and figure/diagram regions enriched
/// with their source crop and slot overlays.
pub(super) fn build_stimulus(
    page: &PageNodeV2,
    blocks: &[QuestionBlockCandidateV1],
    region_visuals: Vec<VisualStimulusCandidateV1>,
) -> StimulusBuild {
    let tables = page
        .tables
        .iter()
        .enumerate()
        .map(|(index, table)| compile_table(page, table, index))
        .collect();
    let visuals = region_visuals
        .into_iter()
        .map(|visual| enrich_visual(page, blocks, visual))
        .collect();
    StimulusBuild { tables, visuals }
}

// ------------------------------------------------------------------- tables ---

fn compile_table(
    page: &PageNodeV2,
    table: &TableNodeV2,
    index: usize,
) -> TableStimulusCandidateV1 {
    let mut issues = Vec::new();
    let mut by_row: BTreeMap<u32, Vec<TableCellCandidateV1>> = BTreeMap::new();
    let mut unresolved_cell = false;

    for cell in &table.cells {
        let text = cell
            .content_region_ids
            .iter()
            .filter_map(|region_id| page.regions.iter().find(|region| region.id == *region_id))
            .map(|region| region_text(page, region))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        // A cell that claims content but resolves to nothing means the cell's
        // ownership cannot be established; reporting it beats rendering it blank.
        if text.is_empty() && !cell.content_region_ids.is_empty() {
            unresolved_cell = true;
        }
        by_row
            .entry(cell.row)
            .or_default()
            .push(TableCellCandidateV1 {
                cell_id: cell.cell_id.clone(),
                row: cell.row,
                col: cell.col,
                row_span: cell.row_span,
                col_span: cell.col_span,
                text,
                bbox: cell.bbox.clone(),
            });
    }

    let rows = by_row
        .into_iter()
        .map(|(row, mut cells)| {
            cells.sort_by_key(|cell| cell.col);
            TableRowCandidateV1 { row, cells }
        })
        .collect::<Vec<_>>();

    if unresolved_cell {
        issues.push(issue_codes::SOURCE_OWNERSHIP_CONFLICT.to_string());
    }

    // §6.10: reconstruction failure must not lose the surface. Without a crop there
    // is nothing left to render, so this blocks rather than degrades.
    if table.topology_confidence < MIN_TABLE_TOPOLOGY_CONFIDENCE
        && table.visual_fallback_asset_id.is_none()
    {
        issues.push(issue_codes::ASSET_REFERENCE_MISSING.to_string());
    }

    TableStimulusCandidateV1 {
        stimulus_id: format!("table-{}-{}", page.page_index, index + 1),
        page_index: page.page_index,
        table_id: table.id.clone(),
        bbox: table.bbox.clone(),
        rows,
        asset_id: table.visual_fallback_asset_id.clone(),
        confidence: table.topology_confidence.min(table.content_confidence).clamp(0.0, 1.0),
        issues,
    }
}

// ------------------------------------------------------------------ visuals ---

fn enrich_visual(
    page: &PageNodeV2,
    blocks: &[QuestionBlockCandidateV1],
    visual: VisualStimulusCandidateV1,
) -> VisualStimulusCandidateV1 {
    let asset_id = resolve_asset(page, &visual.bbox);
    let mut hotspots = Vec::new();
    let mut issues = Vec::new();

    let mut orphaned_slot = false;
    for block in blocks.iter().filter(|block| block.page_index == page.page_index) {
        let number = &block.number_bbox;
        let center_x = number.x + number.width / 2.0;
        let center_y = number.y + number.height / 2.0;
        let inside = contains(&visual.bbox, center_x, center_y, HOTSPOT_SLACK_PT);
        let overlaps_vertically = number.y <= bottom(&visual.bbox) + HOTSPOT_SLACK_PT
            && bottom(number) >= visual.bbox.y - HOTSPOT_SLACK_PT;
        if !inside {
            // Vertically inside the figure but horizontally outside means the number
            // was meant to sit on the figure and the geometry disagrees.
            if overlaps_vertically {
                orphaned_slot = true;
            }
            continue;
        }
        match normalize_rect(number, &visual.bbox) {
            Some(normalized_rect) => hotspots.push(VisualHotspotCandidateV1 {
                slot_id: format!("q{}", block.question_number),
                question_number: block.question_number,
                normalized_rect,
                confidence: visual.confidence.min(block.boundary_confidence),
            }),
            None => issues.push(issue_codes::HOTSPOT_GEOMETRY_INVALID.to_string()),
        }
    }

    if orphaned_slot {
        issues.push(issue_codes::SLOT_OUTSIDE_FIGURE.to_string());
    }
    // §6.10: the surface is preserved when *either* the reconstruction (hotspots) or
    // the source crop exists. A figure that questions point at but that has neither has
    // lost its question surface, so it blocks instead of degrading quietly. A figure no
    // question references is background art, not evidence, and must not block.
    if !visual.question_refs.is_empty() && hotspots.is_empty() {
        issues.push(issue_codes::SLOT_OUTSIDE_FIGURE.to_string());
        if asset_id.is_none() {
            issues.push(issue_codes::ASSET_REFERENCE_MISSING.to_string());
        }
    }

    VisualStimulusCandidateV1 {
        asset_id,
        hotspots,
        issues,
        ..visual
    }
}

/// The raster asset whose placement covers the figure region, if any. Vector-drawn
/// diagrams have none, and inventing a reference would fabricate provenance.
fn resolve_asset(page: &PageNodeV2, region_bbox: &RectV2) -> Option<String> {
    let best = page
        .image_placements
        .iter()
        .filter_map(|placement| {
            let bbox = placement.bbox.as_ref()?;
            let overlap = overlap_area(bbox, region_bbox);
            (overlap > 0.0).then_some((overlap, placement.asset_id.clone()))
        })
        .max_by(|left, right| left.0.total_cmp(&right.0));
    best.map(|(_, asset_id)| asset_id)
}

/// Normalizes a rect into the stimulus box as `[x, y, width, height]` in 0..1.
/// Returns `None` for degenerate geometry so the caller reports it instead of
/// emitting a hotspot that cannot be rendered.
fn normalize_rect(inner: &RectV2, outer: &RectV2) -> Option<[f64; 4]> {
    if outer.width <= 0.0 || outer.height <= 0.0 {
        return None;
    }
    let x = (inner.x - outer.x) / outer.width;
    let y = (inner.y - outer.y) / outer.height;
    let width = inner.width / outer.width;
    let height = inner.height / outer.height;
    let values = [x, y, width, height];
    if values.iter().any(|value| !value.is_finite()) || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let clamped: Vec<f64> = values.iter().map(|value| value.clamp(0.0, 1.0)).collect();
    Some([clamped[0], clamped[1], clamped[2], clamped[3]])
}

fn bottom(rect: &RectV2) -> f64 {
    rect.y + rect.height
}

fn contains(rect: &RectV2, x: f64, y: f64, slack: f64) -> bool {
    x >= rect.x - slack
        && x <= rect.x + rect.width + slack
        && y >= rect.y - slack
        && y <= bottom(rect) + slack
}

fn overlap_area(left: &RectV2, right: &RectV2) -> f64 {
    let width = (left.x + left.width).min(right.x + right.width) - left.x.max(right.x);
    let height = (bottom(left)).min(bottom(right)) - left.y.max(right.y);
    width.max(0.0) * height.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::common::{CoordinateOriginV2, CoordinateUnitV2};
    use crate::schema::document_ir_v2::{DocumentIRV2, TableCellV2};
    use serde_json::{json, Value};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RectV2 {
        RectV2 {
            x,
            y,
            width,
            height,
            unit: CoordinateUnitV2::Pt,
            origin: CoordinateOriginV2::TopLeft,
            page_rotation: 0,
            normalized: None,
        }
    }

    fn anchor(node_id: &str) -> Value {
        json!({
            "sourceFileId": "file-1",
            "pageIndex": 0,
            "nodeIds": [node_id],
            "extractionMode": "pdf_native",
            "sourceHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        })
    }

    fn document(pages: Value) -> DocumentIRV2 {
        serde_json::from_value(json!({
            "schemaVersion": "DocumentIRV2",
            "documentId": "document-1",
            "jobId": "job-1",
            "sourceFiles": [{
                "sourceFileId": "file-1",
                "originalName": "sample.pdf",
                "mediaType": "application/pdf",
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "byteLength": 1,
                "role": "question_paper"
            }],
            "pages": pages,
            "assets": [],
            "coverageLedger": [],
            "parser": {
                "provider": "p4-t05-test",
                "providerVersion": "0.1.0",
                "extractionStartedAt": "2026-09-12T00:00:00Z",
                "extractionCompletedAt": "2026-09-12T00:00:01Z",
                "options": {},
                "warnings": []
            }
        }))
        .expect("fixture document")
    }

    fn page_value(
        lines: Value,
        regions: Value,
        tables: Value,
        image_placements: Value,
        asset_ids: Value,
    ) -> Value {
        json!({
            "pageIndex": 0,
            "widthPt": 595.0,
            "heightPt": 842.0,
            "rotation": 0,
            "glyphs": [],
            "spans": [],
            "lines": lines,
            "regions": regions,
            "vectorPaths": [],
            "tables": tables,
            "assetIds": asset_ids,
            "imagePlacements": image_placements,
            "readingOrder": [],
            "quality": {
                "classification": "born_digital",
                "nativeCharacterCount": 1,
                "unicodeErrorRatio": 0.0,
                "duplicateTextRatio": 0.0,
                "imageCoverageRatio": 0.0,
                "textCoverageRatio": 1.0,
                "rotationConfidence": 1.0,
                "requiresOcrRegions": [],
                "warnings": []
            }
        })
    }

    fn line(id: &str, text: &str, y: f64) -> Value {
        json!({
            "id": id,
            "spanIds": [],
            "text": text,
            "bbox": {"x": 72.0, "y": y, "width": 200.0, "height": 12.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
            "writingMode": "horizontal-tb",
            "indentationPt": 0.0,
            "sourceOrder": 0,
            "confidence": 1.0,
            "sourceAnchors": [anchor(id)]
        })
    }

    fn region(id: &str, kind: &str, bbox: Value, line_ids: &[&str]) -> Value {
        json!({
            "id": id,
            "kind": kind,
            "bbox": bbox,
            "childLineIds": line_ids,
            "childObjectIds": [],
            "confidence": 1.0,
            "sourceAnchors": [anchor(id)]
        })
    }

    fn cell(cell_id: &str, row: u32, col: u32, region_id: &str, bbox: RectV2) -> TableCellV2 {
        serde_json::from_value(json!({
            "cellId": cell_id,
            "row": row,
            "col": col,
            "rowSpan": 1,
            "colSpan": 1,
            "bbox": {"x": bbox.x, "y": bbox.y, "width": bbox.width, "height": bbox.height, "unit": "pt", "origin": "top-left", "pageRotation": 0},
            "contentRegionIds": [region_id],
            "borderEvidence": [],
            "confidence": 1.0,
            "sourceAnchors": [anchor(cell_id)]
        }))
        .expect("fixture cell")
    }

    fn block(number: u32, y: f64) -> QuestionBlockCandidateV1 {
        QuestionBlockCandidateV1 {
            candidate_id: format!("block-0-{number}"),
            question_number: number,
            page_index: 0,
            number_anchor: None,
            number_bbox: rect(300.0, y, 8.0, 12.0),
            stem_node_ids: Vec::new(),
            stem_text: String::new(),
            option_run: None,
            shared_option_bank_ref: None,
            visual_object_refs: Vec::new(),
            source_coverage: 1.0,
            boundary_confidence: 1.0,
            ambiguities: Vec::new(),
        }
    }

    fn region_visual(bbox: RectV2, question_refs: Vec<u32>) -> VisualStimulusCandidateV1 {
        VisualStimulusCandidateV1 {
            stimulus_id: "stimulus-0-1".to_string(),
            page_index: 0,
            region_id: "r-figure".to_string(),
            kind: crate::schema::document_ir_v2::PhysicalRegionKindV2::Diagram,
            bbox,
            confidence: 0.9,
            question_refs,
            asset_id: None,
            hotspots: Vec::new(),
            issues: Vec::new(),
        }
    }

    #[test]
    fn physical_table_keeps_its_rows_cols_and_spans() {
        let table_bbox = rect(72.0, 400.0, 300.0, 60.0);
        let mut table_cell = cell("c-1-1", 1, 1, "r-cell-a", rect(72.0, 420.0, 150.0, 20.0));
        table_cell.row_span = 2;
        let table_value = json!({
            "id": "table-1",
            "bbox": {"x": table_bbox.x, "y": table_bbox.y, "width": table_bbox.width, "height": table_bbox.height, "unit": "pt", "origin": "top-left", "pageRotation": 0},
            "rows": 2,
            "cols": 2,
            "cells": [
                serde_json::to_value(&table_cell).unwrap(),
                serde_json::to_value(cell("c-1-2", 1, 2, "r-cell-b", rect(222.0, 420.0, 150.0, 20.0))).unwrap(),
                serde_json::to_value(cell("c-2-2", 2, 2, "r-cell-c", rect(222.0, 440.0, 150.0, 20.0))).unwrap()
            ],
            "detectionMode": "ruling_lines",
            "topologyConfidence": 0.95,
            "contentConfidence": 0.9,
            "sourceAnchors": [anchor("table-1")]
        });
        // Deliberately declare the rows out of order to prove they are regrouped.
        let document = document(json!([page_value(
            json!([line("l-a", "Year", 420.0), line("l-b", "Value", 420.0), line("l-c", "2001", 440.0)]),
            json!([
                region("r-cell-a", "text", json!({"x": 72.0, "y": 420.0, "width": 150.0, "height": 20.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}), &["l-a"]),
                region("r-cell-b", "text", json!({"x": 222.0, "y": 420.0, "width": 150.0, "height": 20.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}), &["l-b"]),
                region("r-cell-c", "text", json!({"x": 222.0, "y": 440.0, "width": 150.0, "height": 20.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}), &["l-c"])
            ]),
            json!([table_value]),
            json!([]),
            json!([])
        )]));

        let page = &document.pages[0];
        let build = build_stimulus(page, &[], Vec::new());
        assert_eq!(build.tables.len(), 1);
        let table = &build.tables[0];
        assert_eq!(table.table_id, "table-1");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].row, 1);
        assert_eq!(table.rows[1].row, 2);
        // The spanning cell survives as a spanning cell rather than being duplicated.
        assert_eq!(table.rows[0].cells[0].row_span, 2);
        assert_eq!(table.rows[0].cells[0].text, "Year");
        assert_eq!(table.rows[0].cells[1].text, "Value");
        assert_eq!(table.rows[1].cells[0].text, "2001");
        assert!(table.issues.is_empty(), "{:?}", table.issues);
    }

    #[test]
    fn low_topology_table_without_a_crop_blocks_instead_of_losing_the_surface() {
        let table_value = json!({
            "id": "table-1",
            "bbox": {"x": 72.0, "y": 400.0, "width": 300.0, "height": 60.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
            "rows": 1,
            "cols": 1,
            "cells": [],
            "detectionMode": "vision_model",
            "topologyConfidence": 0.3,
            "contentConfidence": 0.3,
            "sourceAnchors": [anchor("table-1")]
        });
        let document = document(json!([page_value(
            json!([]),
            json!([]),
            json!([table_value]),
            json!([]),
            json!([])
        )]));
        let build = build_stimulus(&document.pages[0], &[], Vec::new());
        assert!(build.tables[0]
            .issues
            .contains(&issue_codes::ASSET_REFERENCE_MISSING.to_string()));
    }

    #[test]
    fn diagram_hybrid_emits_normalized_hotspots_over_the_source_crop() {
        let figure_bbox = rect(100.0, 200.0, 400.0, 200.0);
        let document = document(json!([page_value(
            json!([]),
            json!([region("r-figure", "diagram", json!({"x": 100.0, "y": 200.0, "width": 400.0, "height": 200.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}), &[])]),
            json!([]),
            json!([{
                "id": "placement-1",
                "assetId": "source-crop-1",
                "bbox": {"x": 100.0, "y": 200.0, "width": 400.0, "height": 200.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                "objectTransform": [1, 0, 0, 1, 0, 0],
                "confidence": 1.0,
                "sourceAnchor": anchor("placement-1")
            }]),
            json!(["source-crop-1"])
        )]));
        let blocks = vec![block(27, 250.0), block(28, 300.0)];
        let build = build_stimulus(
            &document.pages[0],
            &blocks,
            vec![region_visual(figure_bbox.clone(), vec![27, 28])],
        );
        let visual = &build.visuals[0];
        assert_eq!(visual.asset_id.as_deref(), Some("source-crop-1"));
        assert_eq!(visual.hotspots.len(), 2);
        let hotspot = &visual.hotspots[0];
        assert_eq!(hotspot.slot_id, "q27");
        // 300pt into a 400pt-wide box, 50pt into a 200pt-tall box.
        assert!((hotspot.normalized_rect[0] - 0.5).abs() < 1e-9);
        assert!((hotspot.normalized_rect[1] - 0.25).abs() < 1e-9);
        for value in hotspot.normalized_rect {
            assert!((0.0..=1.0).contains(&value));
        }
        assert!(visual.issues.is_empty(), "{:?}", visual.issues);
    }

    #[test]
    fn referenced_slots_that_cannot_be_attached_are_reported() {
        // The figure sits far from the question numbers, so no hotspot can be placed.
        let document = document(json!([page_value(
            json!([]),
            json!([region("r-figure", "diagram", json!({"x": 100.0, "y": 700.0, "width": 200.0, "height": 100.0, "unit": "pt", "origin": "top-left", "pageRotation": 0}), &[])]),
            json!([]),
            json!([]),
            json!([])
        )]));
        let blocks = vec![block(27, 250.0)];
        let build = build_stimulus(
            &document.pages[0],
            &blocks,
            vec![region_visual(rect(100.0, 700.0, 200.0, 100.0), vec![27])],
        );
        let visual = &build.visuals[0];
        assert!(visual.hotspots.is_empty());
        assert!(visual
            .issues
            .contains(&issue_codes::SLOT_OUTSIDE_FIGURE.to_string()));
        assert!(visual
            .issues
            .contains(&issue_codes::ASSET_REFERENCE_MISSING.to_string()));
    }
}
