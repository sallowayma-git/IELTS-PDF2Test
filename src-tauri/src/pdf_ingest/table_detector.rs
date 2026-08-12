use super::{clean, rect, Bounds};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::BTreeSet;

fn path_bounds(path: &Value) -> Option<Bounds> {
    super::bounds_of(path, "bbox")
}

fn path_id(path: &Value) -> Option<String> {
    path.get("id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn cluster(mut values: Vec<f64>, tolerance: f64) -> Vec<f64> {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let mut clusters: Vec<Vec<f64>> = Vec::new();
    for value in values {
        if let Some(last) = clusters.last_mut() {
            let mean = last.iter().sum::<f64>() / last.len() as f64;
            if (value - mean).abs() <= tolerance {
                last.push(value);
                continue;
            }
        }
        clusters.push(vec![value]);
    }
    clusters
        .into_iter()
        .map(|items| items.iter().sum::<f64>() / items.len() as f64)
        .collect()
}

fn line_ids_for_cell(lines: &[Value], regions: &[Value], cell: Bounds) -> Vec<String> {
    let line_ids = lines
        .iter()
        .filter_map(|line| {
            let id = line.get("id").and_then(Value::as_str)?;
            let bounds = super::bounds_of(line, "bbox")?;
            cell.intersection(bounds)
                .filter(|overlap| overlap.area() >= bounds.area() * 0.25)
                .map(|_| id.to_string())
        })
        .collect::<BTreeSet<_>>();
    regions
        .iter()
        .filter_map(|region| {
            let id = region.get("id").and_then(Value::as_str)?;
            let mut child_line_ids = region
                .get("childLineIds")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(Value::as_str);
            child_line_ids
                .any(|line_id| line_ids.contains(line_id))
                .then_some(id.to_string())
        })
        .collect()
}

fn anchor_list(paths: &[Value], regions: &[Value]) -> Vec<Value> {
    paths
        .iter()
        .filter_map(|path| path.get("sourceAnchor").cloned())
        .chain(regions.iter().flat_map(|region| {
            region
                .get("sourceAnchors")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        }))
        .collect()
}

fn segment_overlap(start: f64, end: f64, other_start: f64, other_end: f64) -> f64 {
    (end.min(other_end) - start.max(other_start)).max(0.0)
}

fn path_segments(path: &Value) -> Vec<((f64, f64), (f64, f64))> {
    let mut segments = Vec::new();
    let mut current = None;
    let mut first = None;
    for command in path
        .get("commands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match command.get("op").and_then(Value::as_str) {
            Some("move") => {
                current = Some((
                    command.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                    command.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                ));
                first = current;
            }
            Some("line") => {
                let next = (
                    command.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                    command.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                );
                if let Some(previous) = current {
                    segments.push((previous, next));
                }
                current = Some(next);
            }
            Some("close") => {
                if let (Some(previous), Some(start)) = (current, first) {
                    segments.push((previous, start));
                }
                current = first;
            }
            _ => {}
        }
    }
    segments
}

fn vertical_boundary_present(paths: &[Value], x: f64, top: f64, bottom: f64) -> bool {
    let height = (bottom - top).max(0.01);
    paths.iter().flat_map(path_segments).any(|(start, end)| {
        let mean_x = (start.0 + end.0) / 2.0;
        (start.0 - end.0).abs() <= 3.0
            && (mean_x - x).abs() <= 3.0
            && segment_overlap(start.1.min(end.1), start.1.max(end.1), top, bottom) >= height * 0.60
    })
}

fn horizontal_boundary_present(paths: &[Value], y: f64, left: f64, right: f64) -> bool {
    let width = (right - left).max(0.01);
    paths.iter().flat_map(path_segments).any(|(start, end)| {
        let mean_y = (start.1 + end.1) / 2.0;
        (start.1 - end.1).abs() <= 3.0
            && (mean_y - y).abs() <= 3.0
            && segment_overlap(start.0.min(end.0), start.0.max(end.0), left, right) >= width * 0.60
    })
}

fn evidence_paths_for_cell(paths: &[Value], cell: Bounds) -> Vec<Value> {
    paths
        .iter()
        .filter(|path| {
            path_bounds(path).is_some_and(|path_bbox| {
                path_bbox.intersection(cell).is_some()
                    || (path_bbox.x - cell.x).abs() <= 3.0
                    || (path_bbox.right() - cell.right()).abs() <= 3.0
                    || (path_bbox.y - cell.y).abs() <= 3.0
                    || (path_bbox.bottom() - cell.bottom()).abs() <= 3.0
            })
        })
        .cloned()
        .collect()
}

fn anchors_for_cell(
    paths: &[Value],
    lines: &[Value],
    regions: &[Value],
    cell: Bounds,
) -> Vec<Value> {
    let path_anchors = evidence_paths_for_cell(paths, cell)
        .into_iter()
        .filter_map(|path| path.get("sourceAnchor").cloned());
    let line_anchors = lines
        .iter()
        .filter(|line| {
            super::bounds_of(line, "bbox").is_some_and(|bbox| {
                cell.intersection(bbox)
                    .is_some_and(|overlap| overlap.area() >= bbox.area() * 0.20)
            })
        })
        .flat_map(|line| {
            line.get("sourceAnchors")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        });
    let region_anchors = regions
        .iter()
        .filter(|region| {
            super::bounds_of(region, "bbox").is_some_and(|bbox| {
                cell.intersection(bbox)
                    .is_some_and(|overlap| overlap.area() >= bbox.area() * 0.20)
            })
        })
        .flat_map(|region| {
            region
                .get("sourceAnchors")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        });
    path_anchors
        .chain(line_anchors)
        .chain(region_anchors)
        .collect()
}

fn make_table(
    index: usize,
    x: &[f64],
    y: &[f64],
    page_rotation: u16,
    paths: &[Value],
    lines: &[Value],
    regions: &[Value],
    mode: &str,
    confidence: f64,
) -> Option<Value> {
    if x.len() < 2 || y.len() < 2 {
        return None;
    }
    let bbox = Bounds {
        x: x[0],
        y: y[0],
        width: (x[x.len() - 1] - x[0]).max(0.01),
        height: (y[y.len() - 1] - y[0]).max(0.01),
    };
    let row_count = y.len() - 1;
    let col_count = x.len() - 1;
    let mut cells = Vec::new();
    let mut occupied = vec![vec![false; col_count]; row_count];
    for row in 0..row_count {
        for col in 0..col_count {
            if occupied[row][col] {
                continue;
            }
            let mut col_span = 1usize;
            if mode == "ruling_lines" {
                while col + col_span < col_count
                    && !vertical_boundary_present(paths, x[col + col_span], y[row], y[row + 1])
                {
                    col_span += 1;
                }
            }
            let mut row_span = 1usize;
            if mode == "ruling_lines" {
                while row + row_span < row_count
                    && !horizontal_boundary_present(
                        paths,
                        y[row + row_span],
                        x[col],
                        x[col + col_span],
                    )
                    && (col..col + col_span)
                        .all(|candidate_col| !occupied[row + row_span][candidate_col])
                {
                    row_span += 1;
                }
            }
            for occupied_row in occupied.iter_mut().skip(row).take(row_span) {
                for slot in occupied_row.iter_mut().skip(col).take(col_span) {
                    *slot = true;
                }
            }
            let cell_bbox = Bounds {
                x: x[col],
                y: y[row],
                width: (x[col + col_span] - x[col]).max(0.01),
                height: (y[row + row_span] - y[row]).max(0.01),
            };
            let content_region_ids = regions
                .iter()
                .filter_map(|region| {
                    let id = region.get("id").and_then(Value::as_str)?;
                    let region_bbox = super::bounds_of(region, "bbox")?;
                    cell_bbox
                        .intersection(region_bbox)
                        .filter(|overlap| overlap.area() >= region_bbox.area() * 0.2)
                        .map(|_| id.to_string())
                })
                .collect::<Vec<_>>();
            let evidence_paths = evidence_paths_for_cell(paths, cell_bbox);
            cells.push(json!({
                "cellId": format!("p-table-{}-r{}-c{}", index + 1, row, col),
                "row": row as u32,
                "col": col as u32,
                "rowSpan": row_span as u32,
                "colSpan": col_span as u32,
                "bbox": rect(cell_bbox, page_rotation),
                "contentRegionIds": if content_region_ids.is_empty() { line_ids_for_cell(lines, regions, cell_bbox) } else { content_region_ids },
                "headerScope": if row == 0 { "row" } else { "none" },
                "borderEvidence": evidence_paths.iter().filter_map(path_id).collect::<Vec<_>>(),
                "confidence": clean(confidence),
                "sourceAnchors": anchors_for_cell(paths, lines, regions, cell_bbox)
            }));
        }
    }
    Some(json!({
        "id": format!("p-table-{:04}", index + 1),
        "bbox": rect(bbox, page_rotation),
        "rows": row_count as u32,
        "cols": col_count as u32,
        "cells": cells,
        "detectionMode": mode,
        "topologyConfidence": clean(confidence),
        "contentConfidence": clean(if mode == "ruling_lines" {0.86} else {0.62}),
        "sourceAnchors": anchor_list(paths, regions)
    }))
}

fn ruled_candidate(
    page: &Value,
    page_rotation: u16,
    lines: &[Value],
    regions: &[Value],
    index: usize,
) -> Option<Value> {
    let paths = page
        .get("vectorPaths")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let vertical = paths
        .iter()
        .filter_map(|path| {
            let bounds = path_bounds(path)?;
            (bounds.width <= 2.6 && bounds.height >= 12.0).then_some(bounds.center_x())
        })
        .collect::<Vec<_>>();
    let horizontal = paths
        .iter()
        .filter_map(|path| {
            let bounds = path_bounds(path)?;
            (bounds.height <= 2.6 && bounds.width >= 12.0).then_some(bounds.center_y())
        })
        .collect::<Vec<_>>();
    let rectangles = paths
        .iter()
        .filter_map(path_bounds)
        .filter(|bounds| {
            bounds.width >= 12.0
                && bounds.height >= 12.0
                && bounds.width
                    <= page
                        .get("widthPt")
                        .and_then(Value::as_f64)
                        .unwrap_or(f64::INFINITY)
                && bounds.height
                    <= page
                        .get("heightPt")
                        .and_then(Value::as_f64)
                        .unwrap_or(f64::INFINITY)
        })
        .collect::<Vec<_>>();
    let rectangle_x = rectangles
        .iter()
        .flat_map(|bounds| [bounds.x, bounds.right()]);
    let rectangle_y = rectangles
        .iter()
        .flat_map(|bounds| [bounds.y, bounds.bottom()]);
    let xs = cluster(vertical.into_iter().chain(rectangle_x).collect(), 3.0);
    let ys = cluster(horizontal.into_iter().chain(rectangle_y).collect(), 3.0);
    if xs.len() < 2 || ys.len() < 2 || xs.len() * ys.len() > 400 {
        return None;
    }
    let table_paths = paths
        .iter()
        .filter_map(|path| {
            let bounds = path_bounds(path)?;
            let in_box = bounds.x >= xs[0] - 4.0
                && bounds.right() <= xs[xs.len() - 1] + 4.0
                && bounds.y >= ys[0] - 4.0
                && bounds.bottom() <= ys[ys.len() - 1] + 4.0;
            in_box.then_some(path.clone())
        })
        .collect::<Vec<_>>();
    let confidence = if table_paths.len() >= xs.len() + ys.len() {
        0.94
    } else {
        0.72
    };
    make_table(
        index,
        &xs,
        &ys,
        page_rotation,
        &table_paths,
        lines,
        regions,
        "ruling_lines",
        confidence,
    )
}

fn borderless_candidate(
    page_width: f64,
    page_height: f64,
    page: &Value,
    page_rotation: u16,
    lines: &[Value],
    regions: &[Value],
    index: usize,
) -> Option<Value> {
    if page
        .get("vectorPaths")
        .and_then(Value::as_array)
        .is_some_and(|paths| !paths.is_empty())
    {
        return None;
    }
    let mut rows = lines
        .iter()
        .filter_map(|line| {
            let bounds = super::bounds_of(line, "bbox")?;
            Some((bounds.center_y(), bounds.x))
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    let mut y_clusters: Vec<Vec<(f64, f64)>> = Vec::new();
    for row in rows {
        if let Some(last) = y_clusters.last_mut() {
            if (row.0 - last[0].0).abs() <= 8.0 {
                last.push(row);
                continue;
            }
        }
        y_clusters.push(vec![row]);
    }
    let candidate_rows = y_clusters
        .into_iter()
        .filter(|row| row.len() >= 2)
        .collect::<Vec<_>>();
    if candidate_rows.len() < 3 {
        return None;
    }
    let mut anchors = candidate_rows
        .iter()
        .flatten()
        .map(|(_, x)| *x)
        .collect::<Vec<_>>();
    anchors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let x_clusters = cluster(anchors, 18.0);
    if x_clusters.len() < 2 || x_clusters.len() > 8 {
        return None;
    }
    let left = x_clusters[0].max(0.0);
    let right = (x_clusters.last().copied().unwrap_or(left) + page_width / x_clusters.len() as f64)
        .min(page_width);
    if right - left < page_width * 0.2 || right - left > page_width * 0.95 || page_height <= 0.0 {
        return None;
    }
    let ys = candidate_rows
        .iter()
        .map(|row| row.iter().map(|(y, _)| *y).sum::<f64>() / row.len() as f64)
        .collect::<Vec<_>>();
    let mut xs = x_clusters;
    xs.push(right);
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    make_table(
        index,
        &xs,
        &ys,
        page_rotation,
        &[],
        lines,
        regions,
        "text_alignment",
        0.58,
    )
}

pub(crate) fn detect_tables(
    page: &Value,
    lines: &[Value],
    regions: &mut [Value],
    page_width: f64,
    page_height: f64,
    page_rotation: u16,
) -> Vec<Value> {
    let mut tables = Vec::new();
    if let Some(table) = ruled_candidate(page, page_rotation, lines, regions, tables.len()) {
        tables.push(table);
    }
    if tables.is_empty() {
        if let Some(table) = borderless_candidate(
            page_width,
            page_height,
            page,
            page_rotation,
            lines,
            regions,
            tables.len(),
        ) {
            tables.push(table);
        }
    }
    let fallback_asset_id = page
        .get("assetIds")
        .and_then(Value::as_array)
        .and_then(|ids| ids.first())
        .and_then(Value::as_str)
        .map(ToString::to_string);
    for table in &mut tables {
        if table
            .get("topologyConfidence")
            .and_then(Value::as_f64)
            .is_some_and(|confidence| confidence < 0.70)
        {
            if let (Some(object), Some(asset_id)) =
                (table.as_object_mut(), fallback_asset_id.as_ref())
            {
                object.insert("visualFallbackAssetId".to_string(), json!(asset_id));
            }
        }
    }
    for table in &tables {
        let Some(table_bbox) = super::bounds_of(table, "bbox") else {
            continue;
        };
        for region in regions.iter_mut() {
            let Some(region_bbox) = super::bounds_of(region, "bbox") else {
                continue;
            };
            if table_bbox
                .intersection(region_bbox)
                .is_some_and(|overlap| overlap.area() >= region_bbox.area() * 0.25)
            {
                if let Some(object) = region.as_object_mut() {
                    object.insert("kind".to_string(), json!("table"));
                    if let Some(ids) = object
                        .get_mut("childObjectIds")
                        .and_then(Value::as_array_mut)
                    {
                        if let Some(id) = table.get("id").cloned() {
                            ids.push(id);
                        }
                    }
                }
            }
        }
    }
    tables
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, start: (f64, f64), end: (f64, f64)) -> Value {
        let bbox = Bounds {
            x: start.0.min(end.0),
            y: start.1.min(end.1),
            width: (start.0 - end.0).abs().max(0.01),
            height: (start.1 - end.1).abs().max(0.01),
        };
        json!({
            "id": id,
            "bbox": rect(bbox, 0),
            "commands": [
                {"op": "move", "x": start.0, "y": start.1},
                {"op": "line", "x": end.0, "y": end.1}
            ],
            "sourceAnchor": {
                "sourceFileId": "fixture",
                "pageIndex": 0,
                "nodeIds": [id],
                "extractionMode": "pdf_native",
                "sourceHash": "fixture"
            }
        })
    }

    #[test]
    fn missing_internal_rule_creates_a_merged_cell_with_cell_specific_evidence() {
        let paths = vec![
            rule("top", (0.0, 0.0), (100.0, 0.0)),
            rule("middle", (0.0, 30.0), (100.0, 30.0)),
            rule("bottom", (0.0, 60.0), (100.0, 60.0)),
            rule("left", (0.0, 0.0), (0.0, 60.0)),
            rule("right", (100.0, 0.0), (100.0, 60.0)),
            rule("partial-middle", (50.0, 30.0), (50.0, 60.0)),
        ];
        let table = make_table(
            0,
            &[0.0, 50.0, 100.0],
            &[0.0, 30.0, 60.0],
            0,
            &paths,
            &[],
            &[],
            "ruling_lines",
            0.9,
        )
        .unwrap();
        let cells = table["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0]["rowSpan"].as_u64(), Some(1));
        assert_eq!(cells[0]["colSpan"].as_u64(), Some(2));
        assert!(!cells[0]["borderEvidence"].as_array().unwrap().is_empty());
        assert!(!cells[0]["sourceAnchors"].as_array().unwrap().is_empty());
        assert_eq!(cells[1]["colSpan"].as_u64(), Some(1));
        assert_eq!(cells[2]["colSpan"].as_u64(), Some(1));
    }
}
