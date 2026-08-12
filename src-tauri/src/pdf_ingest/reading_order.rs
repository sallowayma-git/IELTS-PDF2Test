use super::{clean, Bounds};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

fn region_id(region: &Value) -> Option<String> {
    region
        .get("id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn region_bounds(region: &Value) -> Bounds {
    super::bounds_of(region, "bbox").unwrap_or(Bounds {
        x: 0.0,
        y: 0.0,
        width: 0.01,
        height: 0.01,
    })
}

fn column(region: &Value) -> Option<u32> {
    region
        .get("columnIndex")
        .and_then(Value::as_u64)
        .map(|value| value as u32)
}

fn section(region: &Value) -> u64 {
    region
        .get("sectionIndex")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn base_compare(left: &Value, right: &Value, column_count: u32) -> Ordering {
    let left_bounds = region_bounds(left);
    let right_bounds = region_bounds(right);
    let left_column = column(left);
    let right_column = column(right);
    if section(left) != section(right) {
        return section(left).cmp(&section(right));
    }
    if column_count > 1 && left_column == right_column && left_column.is_some() {
        left_bounds
            .y
            .partial_cmp(&right_bounds.y)
            .unwrap_or(Ordering::Equal)
    } else if column_count > 1 && left_column != right_column {
        match (left_column, right_column) {
            (None, Some(_)) | (Some(_), None) => left_bounds
                .y
                .partial_cmp(&right_bounds.y)
                .unwrap_or(Ordering::Equal),
            (Some(left_column), Some(right_column)) => left_column.cmp(&right_column),
            _ => Ordering::Equal,
        }
    } else {
        left_bounds
            .y
            .partial_cmp(&right_bounds.y)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                left_bounds
                    .x
                    .partial_cmp(&right_bounds.x)
                    .unwrap_or(Ordering::Equal)
            })
    }
}

fn add_forward_edge(
    edges: &mut BTreeMap<(usize, usize), (&'static str, f64)>,
    base_rank: &[usize],
    from: usize,
    to: usize,
    relation: &'static str,
    confidence: f64,
) {
    if from == to || base_rank[from] >= base_rank[to] {
        return;
    }
    edges.entry((from, to)).or_insert((relation, confidence));
}

pub(crate) fn apply_reading_order(
    regions: &mut [Value],
    line_to_region: &BTreeMap<String, String>,
    column_count: u32,
    gutter_confidence: f64,
) -> OrderResult {
    let mut base = (0..regions.len()).collect::<Vec<_>>();
    base.sort_by(|a, b| base_compare(&regions[*a], &regions[*b], column_count));
    let mut base_rank = vec![usize::MAX; regions.len()];
    for (rank, index) in base.iter().enumerate() {
        base_rank[*index] = rank;
    }
    let id_to_index = regions
        .iter()
        .enumerate()
        .filter_map(|(index, region)| region_id(region).map(|id| (id, index)))
        .collect::<BTreeMap<_, _>>();
    let mut constraints = BTreeMap::<(usize, usize), (&'static str, f64)>::new();

    let mut by_section_column = BTreeMap::<(u64, Option<u32>), Vec<usize>>::new();
    for index in 0..regions.len() {
        by_section_column
            .entry((section(&regions[index]), column(&regions[index])))
            .or_default()
            .push(index);
    }
    for indexes in by_section_column.values_mut() {
        indexes.sort_by_key(|index| base_rank[*index]);
        for pair in indexes.windows(2) {
            add_forward_edge(
                &mut constraints,
                &base_rank,
                pair[0],
                pair[1],
                "same_column_above",
                gutter_confidence,
            );
        }
    }

    for ((section_index, left_column), left_regions) in &by_section_column {
        let Some(left_column) = left_column else {
            continue;
        };
        let Some(right_regions) = by_section_column.get(&(*section_index, Some(left_column + 1)))
        else {
            continue;
        };
        if let (Some(from), Some(to)) = (left_regions.last(), right_regions.first()) {
            add_forward_edge(
                &mut constraints,
                &base_rank,
                *from,
                *to,
                "column_transition",
                gutter_confidence,
            );
        }
    }

    for ((section_index, column_index), column_regions) in &by_section_column {
        if column_index.is_none() {
            continue;
        }
        let Some(first_column_region) = column_regions.first() else {
            continue;
        };
        let first_top = region_bounds(&regions[*first_column_region]).y;
        if let Some(full_width_regions) = by_section_column.get(&(*section_index, None)) {
            for full_width in full_width_regions {
                if region_bounds(&regions[*full_width]).bottom() <= first_top + 2.0 {
                    add_forward_edge(
                        &mut constraints,
                        &base_rank,
                        *full_width,
                        *first_column_region,
                        "full_width_heading_to_column",
                        gutter_confidence,
                    );
                }
            }
        }
    }

    let source_sequence = line_to_region
        .values()
        .filter_map(|id| id_to_index.get(id).copied())
        .fold(Vec::<usize>::new(), |mut sequence, index| {
            if sequence.last().copied() != Some(index) {
                sequence.push(index);
            }
            sequence
        });
    for pair in source_sequence.windows(2) {
        if section(&regions[pair[0]]) == section(&regions[pair[1]])
            && column(&regions[pair[0]]) == column(&regions[pair[1]])
        {
            add_forward_edge(
                &mut constraints,
                &base_rank,
                pair[0],
                pair[1],
                "source_line_sequence",
                0.98,
            );
        }
    }

    let mut outgoing = vec![Vec::<usize>::new(); regions.len()];
    let mut indegree = vec![0usize; regions.len()];
    for &(from, to) in constraints.keys() {
        outgoing[from].push(to);
        indegree[to] += 1;
    }
    let mut remaining = (0..regions.len()).collect::<BTreeSet<_>>();
    let mut indexes = Vec::with_capacity(regions.len());
    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .filter(|index| indegree[**index] == 0)
            .min_by_key(|index| base_rank[**index])
            .copied();
        let Some(next) = next else {
            break;
        };
        remaining.remove(&next);
        indexes.push(next);
        for to in &outgoing[next] {
            indegree[*to] = indegree[*to].saturating_sub(1);
        }
    }
    let mut cycle_edges_removed = Vec::new();
    if !remaining.is_empty() {
        for index in &base {
            if remaining.remove(index) {
                indexes.push(*index);
            }
        }
        for ((from, to), (relation, confidence)) in &constraints {
            if indegree[*to] > 0 {
                cycle_edges_removed.push(json!({
                    "from": region_id(&regions[*from]),
                    "to": region_id(&regions[*to]),
                    "relation": relation,
                    "confidence": clean(*confidence)
                }));
            }
        }
    }
    let primary = indexes
        .iter()
        .filter_map(|index| region_id(&regions[*index]))
        .collect::<Vec<_>>();
    let mut alternative = indexes.clone();
    alternative.sort_by(|a, b| {
        let left = region_bounds(&regions[*a]);
        let right = region_bounds(&regions[*b]);
        left.y
            .partial_cmp(&right.y)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.x.partial_cmp(&right.x).unwrap_or(Ordering::Equal))
    });
    let alternative_ids = alternative
        .iter()
        .filter_map(|index| region_id(&regions[*index]))
        .collect::<Vec<_>>();
    let alternatives = if alternative_ids != primary {
        vec![alternative_ids.clone()]
    } else {
        Vec::new()
    };
    for (rank, index) in indexes.iter().enumerate() {
        if let Some(object) = regions[*index].as_object_mut() {
            object.insert("readingOrderRank".to_string(), json!(rank as u32));
            if column_count > 1 && !alternatives.is_empty() {
                object.insert("readingOrderAlternatives".to_string(), json!(alternatives));
            }
            object.insert("confidence".to_string(), json!(clean(gutter_confidence)));
        }
    }
    OrderResult {
        primary,
        alternatives,
        edges: constraints
            .into_iter()
            .filter_map(|((from, to), (relation, confidence))| {
                Some(json!({
                    "from": region_id(&regions[from])?,
                    "to": region_id(&regions[to])?,
                    "relation": relation,
                    "confidence": clean(confidence)
                }))
            })
            .collect(),
        cycle_edges_removed,
        confidence: gutter_confidence,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OrderResult {
    pub primary: Vec<String>,
    pub alternatives: Vec<Vec<String>>,
    pub edges: Vec<Value>,
    pub cycle_edges_removed: Vec<Value>,
    #[allow(dead_code)]
    pub confidence: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(id: &str, x: f64, y: f64, column: Option<u32>) -> Value {
        let mut value = json!({
            "id": id,
            "kind": "text",
            "bbox": {"x": x, "y": y, "width": 80.0, "height": 12.0},
            "sectionIndex": 0
        });
        if let Some(column) = column {
            value["columnIndex"] = json!(column);
        }
        value
    }

    #[test]
    fn graph_uses_line_membership_and_column_constraints() {
        let mut regions = vec![
            region("heading", 0.0, 0.0, None),
            region("left-1", 0.0, 30.0, Some(0)),
            region("left-middle", 0.0, 45.0, Some(0)),
            region("left-2", 0.0, 60.0, Some(0)),
            region("right-1", 120.0, 30.0, Some(1)),
        ];
        let line_to_region = BTreeMap::from([
            ("line-001".to_string(), "heading".to_string()),
            ("line-002".to_string(), "left-1".to_string()),
            ("line-003".to_string(), "left-2".to_string()),
            ("line-004".to_string(), "right-1".to_string()),
        ]);
        let result = apply_reading_order(&mut regions, &line_to_region, 2, 0.93);
        assert_eq!(
            result.primary,
            vec!["heading", "left-1", "left-middle", "left-2", "right-1"]
        );
        let relations = result
            .edges
            .iter()
            .filter_map(|edge| edge["relation"].as_str())
            .collect::<BTreeSet<_>>();
        assert!(relations.contains("full_width_heading_to_column"));
        assert!(relations.contains("same_column_above"));
        assert!(relations.contains("source_line_sequence"));
        assert!(relations.contains("column_transition"));
        assert!(result.cycle_edges_removed.is_empty());
    }
}
