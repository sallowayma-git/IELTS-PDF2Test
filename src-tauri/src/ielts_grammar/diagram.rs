use serde_json::{json, Value};

use crate::schema::ielts_authoring_v2::TaskTypeV2;

use super::instruction_zone::SemanticLine;

pub(crate) fn diagram_candidate(
    task_type: &TaskTypeV2,
    lines: &[SemanticLine],
    task_id: &str,
    asset_id: Option<&str>,
    expected_numbers: &[u32],
) -> Option<Value> {
    if !matches!(
        task_type,
        TaskTypeV2::DiagramLabelCompletion | TaskTypeV2::PlanMapLabelCompletion
    ) {
        return None;
    }
    let anchor = lines.first().map(|line| line.source_anchor.clone())?;
    let asset_id = asset_id?;
    let hotspots = expected_numbers
        .iter()
        .filter_map(|number| {
            let line = lines
                .iter()
                .find(|line| line.text.trim_start().starts_with(&number.to_string()))?;
            let normalized_rect = normalized_rect(line)?;
            let slot_id = format!("q{number}");
            Some(json!({
                "hotspotId": format!("{task_id}-hotspot-{slot_id}"),
                "slotId": slot_id,
                "normalizedRect": normalized_rect,
                "labelAnchor": [
                    normalized_rect[0] + normalized_rect[2] / 2.0,
                    normalized_rect[1] + normalized_rect[3] / 2.0
                ]
            }))
        })
        .collect::<Vec<_>>();
    Some(json!({
        "type": "diagram",
        "id": format!("{task_id}-diagram"),
        "sourceAnchors": [anchor],
        "provenanceStatus": "source",
        "assetId": asset_id,
        "hotspots": hotspots,
        "display": {"align": "left"}
    }))
}

fn normalized_rect(line: &SemanticLine) -> Option<[f64; 4]> {
    line.source_anchor
        .pointer("/displayBBox/normalized")
        .or_else(|| line.source_anchor.pointer("/bbox/normalized"))
        .and_then(rect_from_value)
}

fn rect_from_value(value: &Value) -> Option<[f64; 4]> {
    if let Some(items) = value.as_array().filter(|items| items.len() == 4) {
        let rect = [
            items[0].as_f64()?,
            items[1].as_f64()?,
            items[2].as_f64()?,
            items[3].as_f64()?,
        ];
        return valid_normalized_rect(&rect).then_some(rect);
    }
    let rect = [
        value.get("x")?.as_f64()?,
        value.get("y")?.as_f64()?,
        value.get("width")?.as_f64()?,
        value.get("height")?.as_f64()?,
    ];
    valid_normalized_rect(&rect).then_some(rect)
}

fn valid_normalized_rect(rect: &[f64; 4]) -> bool {
    rect.iter().all(|value| value.is_finite())
        && rect[0] >= 0.0
        && rect[1] >= 0.0
        && rect[2] > 0.0
        && rect[3] > 0.0
        && rect[0] + rect[2] <= 1.0
        && rect[1] + rect[3] <= 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diagram_hotspots_require_normalized_geometry_and_bind_slots() {
        let lines = vec![SemanticLine {
            id: "q1".to_string(),
            text: "1 Label".to_string(),
            source_anchor: json!({"displayBBox":{"normalized":[0.1,0.2,0.2,0.1]}}),
            page_index: 0,
            order: 0,
            role: String::new(),
            bbox: None,
        }];
        let node = diagram_candidate(
            &TaskTypeV2::DiagramLabelCompletion,
            &lines,
            "task",
            Some("asset"),
            &[1],
        )
        .unwrap();
        assert_eq!(
            node.pointer("/hotspots/0/slotId").and_then(Value::as_str),
            Some("q1")
        );
        assert_eq!(
            node.pointer("/hotspots/0/normalizedRect/0")
                .and_then(Value::as_f64),
            Some(0.1)
        );
    }
}
