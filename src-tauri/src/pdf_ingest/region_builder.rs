use super::{clean, rect, Bounds};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(crate) struct RegionBuild {
    pub regions: Vec<Value>,
    pub line_to_region: BTreeMap<String, String>,
    pub column_count: u32,
    pub gutter_confidence: f64,
}

fn line_bounds(line: &Value) -> Option<Bounds> {
    super::bounds_of(line, "bbox")
}

fn line_id(line: &Value) -> Option<String> {
    line.get("id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn full_width(bounds: Bounds, page_width: f64) -> bool {
    bounds.width >= page_width * 0.72
        || (bounds.x <= page_width * 0.08 && bounds.right() >= page_width * 0.92)
}

fn detect_gutters(lines: &[Value], page_width: f64, _page_height: f64) -> (Vec<(f64, f64)>, f64) {
    let body = lines
        .iter()
        .filter_map(line_bounds)
        .filter(|bounds| !full_width(*bounds, page_width))
        .collect::<Vec<_>>();
    if body.len() < 4 {
        return (Vec::new(), 1.0);
    }
    let bins = ((page_width / 4.0).ceil() as usize).clamp(32, 512);
    let bin_width = page_width / bins as f64;
    let mut occupancy = vec![0u32; bins];
    for bounds in &body {
        let start = (bounds.x / bin_width).floor().max(0.0) as usize;
        let end = (bounds.right() / bin_width).ceil().min(bins as f64) as usize;
        for index in start..end.min(bins) {
            occupancy[index] = occupancy[index].saturating_add(1);
        }
    }
    // A heading or callout may cross a real gutter once. Requiring completely
    // empty bins makes one such line collapse otherwise stable 2/3-column
    // layouts, so tolerate one crossing and require parallel text on both sides.
    let maximum_crossings = 1u32;
    let minimum_gap = page_width * 0.035;
    let mut gaps = Vec::new();
    let mut index = 0usize;
    while index < bins {
        if occupancy[index] > maximum_crossings {
            index += 1;
            continue;
        }
        let start = index;
        while index < bins && occupancy[index] <= maximum_crossings {
            index += 1;
        }
        let end = index;
        let gap = (end - start) as f64 * bin_width;
        let left = body
            .iter()
            .filter(|bounds| bounds.right() <= start as f64 * bin_width + 2.0)
            .collect::<Vec<_>>();
        let right = body
            .iter()
            .filter(|bounds| bounds.x >= end as f64 * bin_width - 2.0)
            .collect::<Vec<_>>();
        let parallel_text = left.iter().any(|left_bounds| {
            right.iter().any(|right_bounds| {
                let tolerance = left_bounds.height.max(right_bounds.height) * 1.5;
                left_bounds.y <= right_bounds.bottom() + tolerance
                    && right_bounds.y <= left_bounds.bottom() + tolerance
            })
        });
        if gap >= minimum_gap && parallel_text {
            gaps.push((start as f64 * bin_width, end as f64 * bin_width));
        }
    }
    let coverage = if gaps.is_empty() {
        1.0
    } else {
        let mean_gap = gaps.iter().map(|(start, end)| end - start).sum::<f64>() / gaps.len() as f64;
        (mean_gap / page_width).clamp(0.0, 1.0)
    };
    (gaps, (0.55 + coverage).clamp(0.55, 1.0))
}

fn column_for(bounds: Bounds, gutters: &[(f64, f64)], page_width: f64) -> Option<u32> {
    if full_width(bounds, page_width) {
        return None;
    }
    let mut column = 0u32;
    for (start, end) in gutters {
        if bounds.center_x() > *end {
            column += 1;
        } else if bounds.center_x() >= *start {
            column = if bounds.x < *start {
                column
            } else {
                column + 1
            };
            break;
        }
    }
    Some(column)
}

fn region_kind(
    lines: &[Value],
    line_ids: &[String],
    bbox: Bounds,
    page_height: f64,
) -> &'static str {
    let selected = lines
        .iter()
        .filter(|line| {
            line.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| line_ids.iter().any(|candidate| candidate == id))
        })
        .collect::<Vec<_>>();
    let text = selected
        .iter()
        .filter_map(|line| line.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let font_sizes = selected
        .iter()
        .flat_map(|line| line.get("spans").and_then(Value::as_array))
        .flatten()
        .filter_map(|span| span.get("style"))
        .filter_map(|style| style.get("fontSizePt"))
        .filter_map(Value::as_f64)
        .collect::<Vec<_>>();
    let average_font = if font_sizes.is_empty() {
        0.0
    } else {
        font_sizes.iter().sum::<f64>() / font_sizes.len() as f64
    };
    if text.to_ascii_uppercase() == text
        && text.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 4
    {
        "title"
    } else if average_font > 14.0 || text.to_ascii_lowercase().contains("reading passage") {
        "title"
    } else if bbox.y < 50.0 && text.to_ascii_lowercase().contains("ielts") {
        "header"
    } else if bbox.y > 0.86 * page_height
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace())
    {
        "page_number"
    } else {
        "text"
    }
}

fn make_region(
    page_index: u64,
    index: usize,
    lines: &[Value],
    line_ids: &[String],
    column_index: Option<u32>,
    section_index: u32,
    page_rotation: u16,
    page_height: f64,
) -> Value {
    let bbox = line_ids
        .iter()
        .filter_map(|id| {
            lines
                .iter()
                .find(|line| line_id(line).as_deref() == Some(id))
        })
        .filter_map(line_bounds)
        .reduce(Bounds::union)
        .unwrap_or(Bounds {
            x: 0.0,
            y: 0.0,
            width: 0.01,
            height: 0.01,
        });
    let region_id = format!("p{:03}-r{:04}", page_index + 1, index + 1);
    let anchors = super::source_anchors_from_children(lines, line_ids);
    json!({
        "id": region_id,
        "kind": region_kind(lines, line_ids, bbox, page_height),
        "bbox": rect(bbox, page_rotation),
        "childLineIds": line_ids,
        "childObjectIds": [],
        "columnIndex": column_index,
        "sectionIndex": section_index,
        "confidence": clean(if column_index.is_some() {0.86} else {0.94}),
        "sourceAnchors": anchors
    })
}

pub(crate) fn build_regions(page_width: f64, page_height: f64, lines: &[Value]) -> RegionBuild {
    let page_index = lines
        .first()
        .and_then(|line| line.get("sourceAnchors"))
        .and_then(Value::as_array)
        .and_then(|anchors| anchors.first())
        .and_then(|anchor| anchor.get("pageIndex"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let page_rotation = lines
        .first()
        .and_then(|line| line.get("bbox"))
        .and_then(|bbox| bbox.get("pageRotation"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u16;
    let (global_gutters, _) = detect_gutters(lines, page_width, page_height);
    let mut ordered = lines.to_vec();
    ordered.sort_by(|a, b| {
        let left = line_bounds(a).unwrap_or(Bounds {
            x: 0.0,
            y: 0.0,
            width: 0.01,
            height: 0.01,
        });
        let right = line_bounds(b).unwrap_or(Bounds {
            x: 0.0,
            y: 0.0,
            width: 0.01,
            height: 0.01,
        });
        left.y
            .partial_cmp(&right.y)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.x.partial_cmp(&right.x).unwrap_or(Ordering::Equal))
    });
    let mut sections = Vec::<(u32, Vec<Value>)>::new();
    if global_gutters.is_empty() {
        sections.push((0, ordered));
    } else {
        let mut current = Vec::new();
        let mut next_index = 0u32;
        for line in ordered {
            let is_banner = line_bounds(&line).is_some_and(|bounds| full_width(bounds, page_width));
            if is_banner {
                if !current.is_empty() {
                    sections.push((next_index, std::mem::take(&mut current)));
                    next_index += 1;
                }
                sections.push((next_index, vec![line]));
                next_index += 1;
            } else {
                current.push(line);
            }
        }
        if !current.is_empty() {
            sections.push((next_index, current));
        }
    }
    let mut column_count = 1u32;
    let mut gutter_confidence = 1.0f64;
    let mut by_section_column: BTreeMap<(u32, Option<u32>), Vec<Value>> = BTreeMap::new();
    for (section_index, section_lines) in sections {
        let (gutters, confidence) = detect_gutters(&section_lines, page_width, page_height);
        column_count = column_count.max(gutters.len() as u32 + 1);
        gutter_confidence = gutter_confidence.min(confidence);
        for line in section_lines {
            let Some(bounds) = line_bounds(&line) else {
                continue;
            };
            let column = column_for(bounds, &gutters, page_width);
            by_section_column
                .entry((section_index, column))
                .or_default()
                .push(line);
        }
    }
    for values in by_section_column.values_mut() {
        values.sort_by(|a, b| {
            let ay = line_bounds(a).map(|value| value.y).unwrap_or(0.0);
            let by = line_bounds(b).map(|value| value.y).unwrap_or(0.0);
            ay.partial_cmp(&by)
                .unwrap_or(Ordering::Equal)
                .then_with(|| {
                    line_bounds(a)
                        .map(|value| value.x)
                        .unwrap_or(0.0)
                        .partial_cmp(&line_bounds(b).map(|value| value.x).unwrap_or(0.0))
                        .unwrap_or(Ordering::Equal)
                })
        });
    }

    let mut regions = Vec::new();
    let mut line_to_region = BTreeMap::new();
    for ((section_index, column), column_lines) in by_section_column {
        let mut current = Vec::<String>::new();
        let mut previous: Option<Bounds> = None;
        for line in column_lines {
            let Some(id) = line_id(&line) else { continue };
            let Some(bounds) = line_bounds(&line) else {
                continue;
            };
            let gap = previous.map(|last| bounds.y - last.bottom()).unwrap_or(0.0);
            let should_split = !current.is_empty()
                && (column.is_none() && gap > 14.0 || column.is_some() && gap > 10.0);
            if should_split {
                let region = make_region(
                    page_index,
                    regions.len(),
                    lines,
                    &current,
                    column,
                    section_index,
                    page_rotation,
                    page_height,
                );
                let region_id = region
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                for line_id in &current {
                    line_to_region.insert(line_id.clone(), region_id.clone());
                }
                regions.push(region);
                current.clear();
            }
            current.push(id);
            previous = Some(bounds);
        }
        if !current.is_empty() {
            let region = make_region(
                page_index,
                regions.len(),
                lines,
                &current,
                column,
                section_index,
                page_rotation,
                page_height,
            );
            let region_id = region
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            for line_id in &current {
                line_to_region.insert(line_id.clone(), region_id.clone());
            }
            regions.push(region);
        }
    }
    regions.sort_by(|a, b| {
        let a_section = a.get("sectionIndex").and_then(Value::as_u64).unwrap_or(0);
        let b_section = b.get("sectionIndex").and_then(Value::as_u64).unwrap_or(0);
        let a_column = a.get("columnIndex").and_then(Value::as_u64);
        let b_column = b.get("columnIndex").and_then(Value::as_u64);
        let ay = super::bounds_of(a, "bbox")
            .map(|value| value.y)
            .unwrap_or(0.0);
        let by = super::bounds_of(b, "bbox")
            .map(|value| value.y)
            .unwrap_or(0.0);
        a_section
            .cmp(&b_section)
            .then_with(|| a_column.cmp(&b_column))
            .then_with(|| ay.partial_cmp(&by).unwrap_or(Ordering::Equal))
    });
    RegionBuild {
        regions,
        line_to_region,
        column_count,
        gutter_confidence,
    }
}
