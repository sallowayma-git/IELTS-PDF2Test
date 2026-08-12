use super::{clean, rect, Bounds};
use serde_json::{json, Value};
use std::cmp::Ordering;

#[derive(Debug, Clone)]
struct Glyph {
    id: String,
    text: String,
    bbox: Bounds,
    baseline: f64,
    angle: f64,
    font_size: f64,
    confidence: f64,
    source_order: usize,
    style: Value,
    source_anchor: Value,
    source_line_break_after: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineOrientation {
    Horizontal,
    Vertical,
}

fn orientation(glyph: &Glyph) -> LineOrientation {
    if glyph.angle.sin().abs() > glyph.angle.cos().abs() {
        LineOrientation::Vertical
    } else {
        LineOrientation::Horizontal
    }
}

fn glyphs(page: &Value) -> Vec<Glyph> {
    page.get("glyphs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(source_order, value)| {
            let bbox = super::bounds(value.get("bbox")?)?;
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("�")
                .to_string();
            let font_size = value
                .get("style")
                .and_then(|style| style.get("fontSizePt"))
                .and_then(Value::as_f64)
                .unwrap_or(bbox.height.max(1.0));
            Some(Glyph {
                id: value.get("id")?.as_str()?.to_string(),
                text,
                baseline: value
                    .get("baseline")
                    .and_then(Value::as_f64)
                    .unwrap_or(bbox.bottom() - bbox.height * 0.16),
                angle: value.get("angleRad").and_then(Value::as_f64).unwrap_or(0.0),
                confidence: value
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.5)
                    .clamp(0.0, 1.0),
                style: value.get("style").cloned().unwrap_or_else(|| json!({})),
                source_anchor: value.get("sourceAnchor").cloned().unwrap_or(Value::Null),
                source_line_break_after: value
                    .get("_sourceLineBreakAfter")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                bbox,
                font_size: font_size.max(0.5),
                source_order,
            })
        })
        .collect()
}

fn vertical_overlap(a: Bounds, b: Bounds) -> f64 {
    let top = a.y.max(b.y);
    let bottom = a.bottom().min(b.bottom());
    let overlap = (bottom - top).max(0.0);
    overlap / a.height.min(b.height).max(0.01)
}

fn cluster_horizontal_glyphs(mut glyphs: Vec<Glyph>) -> Vec<Vec<Glyph>> {
    glyphs.sort_by(|a, b| {
        a.baseline
            .partial_cmp(&b.baseline)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.bbox.x.partial_cmp(&b.bbox.x).unwrap_or(Ordering::Equal))
            .then_with(|| a.source_order.cmp(&b.source_order))
    });
    let median_font = {
        let mut values = glyphs
            .iter()
            .map(|glyph| glyph.font_size)
            .collect::<Vec<_>>();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        values.get(values.len() / 2).copied().unwrap_or(10.0)
    };
    let mut clusters: Vec<Vec<Glyph>> = Vec::new();
    for glyph in glyphs {
        let tolerance = (glyph.font_size.max(median_font) * 0.22).clamp(0.8, 4.5);
        let best = clusters
            .iter()
            .enumerate()
            .filter_map(|(index, cluster)| {
                let baseline = cluster.iter().map(|item| item.baseline).sum::<f64>()
                    / cluster.len().max(1) as f64;
                let representative = cluster.last()?.bbox;
                let distance = (baseline - glyph.baseline).abs();
                if distance <= tolerance
                    && vertical_overlap(representative, glyph.bbox) >= 0.35
                    && (cluster.last()?.angle - glyph.angle).abs() < 0.18
                {
                    Some((index, distance))
                } else {
                    None
                }
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
            .map(|value| value.0);
        if let Some(index) = best {
            clusters[index].push(glyph);
        } else {
            clusters.push(vec![glyph]);
        }
    }
    clusters
        .into_iter()
        .flat_map(split_horizontal_clusters)
        .collect()
}

fn horizontal_overlap(a: Bounds, b: Bounds) -> f64 {
    let left = a.x.max(b.x);
    let right = a.right().min(b.right());
    let overlap = (right - left).max(0.0);
    overlap / a.width.min(b.width).max(0.01)
}

fn cluster_vertical_glyphs(mut glyphs: Vec<Glyph>) -> Vec<Vec<Glyph>> {
    glyphs.sort_by(|a, b| {
        a.bbox
            .center_x()
            .partial_cmp(&b.bbox.center_x())
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.bbox.y.partial_cmp(&b.bbox.y).unwrap_or(Ordering::Equal))
            .then_with(|| a.source_order.cmp(&b.source_order))
    });
    let median_font = median(
        &glyphs
            .iter()
            .map(|glyph| glyph.font_size)
            .collect::<Vec<_>>(),
        10.0,
    );
    let mut clusters: Vec<Vec<Glyph>> = Vec::new();
    for glyph in glyphs {
        let tolerance = (glyph.font_size.max(median_font) * 0.28).clamp(0.8, 5.0);
        let best = clusters
            .iter()
            .enumerate()
            .filter_map(|(index, cluster)| {
                let center_x = cluster.iter().map(|item| item.bbox.center_x()).sum::<f64>()
                    / cluster.len().max(1) as f64;
                let representative = cluster.last()?;
                let distance = (center_x - glyph.bbox.center_x()).abs();
                if distance <= tolerance
                    && horizontal_overlap(representative.bbox, glyph.bbox) >= 0.30
                    && (representative.angle - glyph.angle).abs() < 0.24
                {
                    Some((index, distance))
                } else {
                    None
                }
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
            .map(|value| value.0);
        if let Some(index) = best {
            clusters[index].push(glyph);
        } else {
            clusters.push(vec![glyph]);
        }
    }
    clusters
        .into_iter()
        .flat_map(split_vertical_clusters)
        .collect()
}

fn split_vertical_clusters(mut cluster: Vec<Glyph>) -> Vec<Vec<Glyph>> {
    cluster.sort_by(|a, b| {
        a.bbox
            .y
            .partial_cmp(&b.bbox.y)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.source_order.cmp(&b.source_order))
    });
    if cluster.len() < 2 {
        return vec![cluster];
    }
    let font_size = median(
        &cluster
            .iter()
            .map(|glyph| glyph.font_size)
            .collect::<Vec<_>>(),
        10.0,
    );
    let gaps = cluster
        .windows(2)
        .map(|pair| (pair[1].bbox.y - pair[0].bbox.bottom()).max(0.0))
        .collect::<Vec<_>>();
    let typical_gap = median(
        &gaps
            .iter()
            .copied()
            .filter(|gap| *gap > 0.0)
            .collect::<Vec<_>>(),
        0.0,
    );
    let split_threshold = (font_size * 2.4).max(typical_gap * 3.5).max(12.0);
    let mut result = Vec::new();
    let mut current = Vec::new();
    for (index, glyph) in cluster.into_iter().enumerate() {
        if index > 0
            && gaps.get(index - 1).copied().unwrap_or_default() > split_threshold
            && !current.is_empty()
        {
            result.push(std::mem::take(&mut current));
        }
        current.push(glyph);
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

fn cluster_glyphs(glyphs: Vec<Glyph>) -> Vec<Vec<Glyph>> {
    let (vertical, horizontal): (Vec<_>, Vec<_>) = glyphs
        .into_iter()
        .partition(|glyph| orientation(glyph) == LineOrientation::Vertical);
    let mut clusters = cluster_horizontal_glyphs(horizontal);
    clusters.extend(cluster_vertical_glyphs(vertical));
    clusters
}

fn split_horizontal_clusters(mut cluster: Vec<Glyph>) -> Vec<Vec<Glyph>> {
    cluster.sort_by(|a, b| {
        a.bbox
            .x
            .partial_cmp(&b.bbox.x)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.source_order.cmp(&b.source_order))
    });
    if cluster.len() < 2 {
        return vec![cluster];
    }
    let font_size = median(
        &cluster
            .iter()
            .map(|glyph| glyph.font_size)
            .collect::<Vec<_>>(),
        10.0,
    );
    let gaps = cluster
        .windows(2)
        .map(|pair| (pair[1].bbox.x - pair[0].bbox.right()).max(0.0))
        .collect::<Vec<_>>();
    let typical_gap = median(
        &gaps
            .iter()
            .copied()
            .filter(|gap| *gap > 0.0)
            .collect::<Vec<_>>(),
        0.0,
    );
    let split_threshold = (font_size * 2.4).max(typical_gap * 3.5).max(12.0);
    let mut result = Vec::new();
    let mut current = Vec::new();
    for (index, glyph) in cluster.into_iter().enumerate() {
        let should_split =
            index > 0 && gaps.get(index - 1).copied().unwrap_or_default() > split_threshold;
        if should_split && !current.is_empty() {
            result.push(std::mem::take(&mut current));
        }
        current.push(glyph);
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

fn median(values: &[f64], fallback: f64) -> f64 {
    if values.is_empty() {
        return fallback;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    sorted[sorted.len() / 2]
}

fn build_spans(
    items: &[Glyph],
    page_rotation: u16,
    line_id: &str,
    line_orientation: LineOrientation,
) -> (Vec<Value>, String, f64, Vec<f64>) {
    let mut ordered = items.to_vec();
    ordered.sort_by(|a, b| {
        let left = if line_orientation == LineOrientation::Vertical {
            a.bbox.y
        } else {
            a.bbox.x
        };
        let right = if line_orientation == LineOrientation::Vertical {
            b.bbox.y
        } else {
            b.bbox.x
        };
        left.partial_cmp(&right)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.source_order.cmp(&b.source_order))
    });
    let gaps = ordered
        .windows(2)
        .filter_map(|pair| {
            let gap = if line_orientation == LineOrientation::Vertical {
                pair[1].bbox.y - pair[0].bbox.bottom()
            } else {
                pair[1].bbox.x - pair[0].bbox.right()
            };
            (gap > 0.0).then_some(gap)
        })
        .filter(|gap| *gap > 0.0)
        .collect::<Vec<_>>();
    let median_gap = median(&gaps, 0.0);
    let median_font = median(
        &ordered
            .iter()
            .map(|glyph| glyph.font_size)
            .collect::<Vec<_>>(),
        10.0,
    );
    let synthetic_threshold = (median_font * 0.28).max(median_gap * 1.75).max(1.0);

    let mut spans = Vec::new();
    let mut current: Vec<Glyph> = Vec::new();
    let mut whitespace_before = "none";
    let mut line_text = String::new();
    let mut confidence_sum = 0.0;

    let flush = |spans: &mut Vec<Value>, current: &mut Vec<Glyph>, whitespace_before: &mut &str| {
        if current.is_empty() {
            return;
        }
        let bbox = current
            .iter()
            .map(|glyph| glyph.bbox)
            .reduce(Bounds::union)
            .unwrap_or(Bounds {
                x: 0.0,
                y: 0.0,
                width: 0.01,
                height: 0.01,
            });
        let text = current
            .iter()
            .map(|glyph| glyph.text.as_str())
            .collect::<String>();
        let source_anchors = current
            .iter()
            .filter_map(|glyph| {
                (!glyph.source_anchor.is_null()).then_some(glyph.source_anchor.clone())
            })
            .collect::<Vec<_>>();
        let span_id = format!("{}-s{:03}", line_id, spans.len() + 1);
        spans.push(json!({
            "id": span_id,
            "glyphIds": current.iter().map(|glyph| glyph.id.clone()).collect::<Vec<_>>(),
            "text": text,
            "bbox": rect(bbox, page_rotation),
            "style": current.first().map(|glyph| glyph.style.clone()).unwrap_or_else(|| json!({})),
            "whitespaceBefore": *whitespace_before,
            "whitespaceAfter": "none",
            "confidence": clean(current.iter().map(|glyph| glyph.confidence).sum::<f64>() / current.len() as f64),
            "sourceAnchors": source_anchors
        }));
        current.clear();
        *whitespace_before = "none";
    };

    for (index, glyph) in ordered.iter().enumerate() {
        let previous = ordered.get(index.wrapping_sub(1));
        let gap = previous
            .map(|item| {
                if line_orientation == LineOrientation::Vertical {
                    glyph.bbox.y - item.bbox.bottom()
                } else {
                    glyph.bbox.x - item.bbox.right()
                }
            })
            .unwrap_or(0.0);
        let source_space = glyph.text.chars().all(char::is_whitespace);
        let has_gap = gap > synthetic_threshold;
        if source_space {
            if !current.is_empty() {
                flush(&mut spans, &mut current, &mut whitespace_before);
            }
            whitespace_before = "source";
            continue;
        }
        if !current.is_empty() && (has_gap || whitespace_before == "source") {
            if line_text.ends_with(' ') == false {
                line_text.push(' ');
            }
            flush(&mut spans, &mut current, &mut whitespace_before);
            whitespace_before = if has_gap { "synthetic" } else { "source" };
        }
        if !current.is_empty() && line_text.ends_with(' ') == false {
            // Keep the glyph adjacent to the existing span.  The span text is
            // intentionally word-like while the line text retains spaces.
        }
        if current.is_empty() && !line_text.is_empty() && !line_text.ends_with(' ') {
            // A gap before a new span is handled above; this branch covers an
            // explicit space glyph whose span was flushed.
            line_text.push(' ');
        }
        line_text.push_str(&glyph.text);
        confidence_sum += glyph.confidence;
        current.push(glyph.clone());
    }
    flush(&mut spans, &mut current, &mut whitespace_before);
    (
        spans,
        line_text,
        confidence_sum / ordered.len().max(1) as f64,
        gaps.into_iter().map(clean).collect(),
    )
}

pub(crate) fn build_lines(page: &Value) -> Vec<Value> {
    let page_rotation = page.get("rotation").and_then(Value::as_u64).unwrap_or(0) as u16;
    let mut lines = cluster_glyphs(glyphs(page));
    lines.sort_by(|a, b| {
        let ay = a
            .iter()
            .map(|glyph| glyph.bbox.y)
            .fold(f64::INFINITY, f64::min);
        let by = b
            .iter()
            .map(|glyph| glyph.bbox.y)
            .fold(f64::INFINITY, f64::min);
        ay.partial_cmp(&by)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                let ax = a
                    .iter()
                    .map(|glyph| glyph.bbox.x)
                    .fold(f64::INFINITY, f64::min);
                let bx = b
                    .iter()
                    .map(|glyph| glyph.bbox.x)
                    .fold(f64::INFINITY, f64::min);
                ax.partial_cmp(&bx).unwrap_or(Ordering::Equal)
            })
    });

    lines
        .into_iter()
        .enumerate()
        .map(|(index, items)| {
            let line_id = format!(
                "p{:03}-l{:04}",
                page.get("pageIndex").and_then(Value::as_u64).unwrap_or(0) + 1,
                index + 1
            );
            let bbox = items
                .iter()
                .map(|glyph| glyph.bbox)
                .reduce(Bounds::union)
                .unwrap_or(Bounds { x: 0.0, y: 0.0, width: 0.01, height: 0.01 });
            let baseline = median(
                &items.iter().map(|glyph| glyph.baseline).collect::<Vec<_>>(),
                bbox.bottom(),
            );
            let line_orientation = items
                .first()
                .map(orientation)
                .unwrap_or(LineOrientation::Horizontal);
            let (spans, text, confidence, inline_gaps) =
                build_spans(&items, page_rotation, &line_id, line_orientation);
            let mut line = json!({
                "id": line_id,
                "spanIds": spans.iter().filter_map(|span| span.get("id").cloned()).collect::<Vec<_>>(),
                "text": text.trim().to_string(),
                "bbox": rect(bbox, page_rotation),
                "writingMode": if line_orientation == LineOrientation::Vertical { "vertical-rl" } else { "horizontal-tb" },
                "indentationPt": clean(if line_orientation == LineOrientation::Vertical { bbox.y } else { bbox.x }),
                "lineHeightPt": clean(if line_orientation == LineOrientation::Vertical { bbox.width } else { bbox.height }),
                "inlineGapsPt": inline_gaps,
                "sourceOrder": items.iter().map(|glyph| glyph.source_order).min().unwrap_or(index) as u32,
                "confidence": clean(confidence),
                "sourceAnchors": items.iter().filter_map(|glyph| (!glyph.source_anchor.is_null()).then_some(glyph.source_anchor.clone())).collect::<Vec<_>>(),
                "spans": spans
            });
            if items.iter().any(|glyph| glyph.source_line_break_after) {
                if let Some(object) = line.as_object_mut() {
                    object.insert("hardBreakAfter".to_string(), Value::Bool(true));
                    object.insert(
                        "breakBasis".to_string(),
                        Value::String("pdf_output_device_end_line".to_string()),
                    );
                }
            }
            if line_orientation == LineOrientation::Horizontal {
                if let Some(object) = line.as_object_mut() {
                    object.insert("baseline".to_string(), json!(clean(baseline)));
                }
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(id: &str, text: &str, x: f64, y: f64, angle: f64) -> Value {
        json!({
            "id": id,
            "text": text,
            "bbox": {
                "x": x,
                "y": y,
                "width": 8.0,
                "height": 10.0,
                "origin": "top-left",
                "pageRotation": 0
            },
            "baseline": y + 8.0,
            "angleRad": angle,
            "style": {"fontSizePt": 10.0},
            "confidence": 0.8,
            "sourceAnchor": {"pageIndex": 0}
        })
    }

    #[test]
    fn rotated_glyphs_are_retained_as_vertical_lines() {
        let page = json!({
            "pageIndex": 0,
            "rotation": 0,
            "glyphs": [
                glyph("g1", "A", 120.0, 10.0, std::f64::consts::FRAC_PI_2),
                glyph("g2", "B", 120.0, 21.0, std::f64::consts::FRAC_PI_2)
            ]
        });
        let lines = build_lines(&page);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["text"].as_str(), Some("AB"));
        assert_eq!(lines[0]["writingMode"].as_str(), Some("vertical-rl"));
        assert_eq!(lines[0]["spanIds"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn unobserved_line_breaks_are_not_fabricated_and_gaps_are_recorded() {
        let page = json!({
            "pageIndex": 0,
            "rotation": 0,
            "glyphs": [
                glyph("g1", "A", 10.0, 10.0, 0.0),
                glyph("g2", "B", 30.0, 10.0, 0.0)
            ]
        });
        let lines = build_lines(&page);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].get("hardBreakAfter").is_none());
        assert!(lines[0].get("breakBasis").is_none());
        assert_eq!(lines[0]["inlineGapsPt"][0].as_f64(), Some(12.0));
    }
}
