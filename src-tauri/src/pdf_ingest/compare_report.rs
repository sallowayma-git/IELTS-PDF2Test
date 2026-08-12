use serde_json::{json, Value};
use std::collections::BTreeSet;

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tokens(value: &str) -> Vec<String> {
    normalize(value)
        .split(' ')
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn v1_page_text(page: &Value) -> String {
    page.get("blocks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn v2_page_text(page: &Value) -> String {
    let lines = page
        .get("lines")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let regions = page
        .get("regions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let line_map = lines
        .iter()
        .filter_map(|line| {
            Some((
                line.get("id")?.as_str()?.to_string(),
                line.get("text")?.as_str()?.to_string(),
            ))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let order = page
        .get("readingOrder")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for region_id in order.iter().filter_map(Value::as_str) {
        if let Some(region) = regions
            .iter()
            .find(|region| region.get("id").and_then(Value::as_str) == Some(region_id))
        {
            for line_id in region
                .get("childLineIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if let Some(text) = line_map.get(line_id) {
                    if seen.insert(line_id.to_string()) {
                        result.push(text.clone());
                    }
                }
            }
        }
    }
    for line in &lines {
        if let (Some(id), Some(text)) = (
            line.get("id").and_then(Value::as_str),
            line.get("text").and_then(Value::as_str),
        ) {
            if seen.insert(id.to_string()) {
                result.push(text.to_string());
            }
        }
    }
    result.join("\n")
}

fn common_tokens(left: &[String], right: &[String]) -> usize {
    let mut left_counts = left.iter().fold(
        std::collections::BTreeMap::<&str, usize>::new(),
        |mut counts, token| {
            *counts.entry(token.as_str()).or_default() += 1;
            counts
        },
    );
    right.iter().fold(0usize, |common, token| {
        let Some(count) = left_counts.get_mut(token.as_str()) else {
            return common;
        };
        if *count == 0 {
            common
        } else {
            *count -= 1;
            common + 1
        }
    })
}

pub(crate) fn build_compare_report(job_id: &str, shadow: &Value, v1: Option<&Value>) -> Value {
    let Some(v1) = v1 else {
        return json!({
            "schemaVersion": "DocumentIRV2CompareReportV1",
            "jobId": job_id,
            "status": "skipped",
            "reason": "v1_document_not_supplied",
            "pages": []
        });
    };
    let v1_pages = v1
        .get("pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let v2_pages = shadow
        .get("pages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let max_pages = v1_pages.len().max(v2_pages.len());
    let mut changed_pages = 0usize;
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut page_reports = Vec::new();
    for index in 0..max_pages {
        let left = v1_pages.get(index).map(v1_page_text).unwrap_or_default();
        let right = v2_pages.get(index).map(v2_page_text).unwrap_or_default();
        let left_tokens = tokens(&left);
        let right_tokens = tokens(&right);
        let common = common_tokens(&left_tokens, &right_tokens);
        let page_added = right_tokens.len().saturating_sub(common);
        let page_removed = left_tokens.len().saturating_sub(common);
        added += page_added;
        removed += page_removed;
        let equal = normalize(&left) == normalize(&right);
        if !equal {
            changed_pages += 1;
        }
        page_reports.push(json!({
            "pageIndex": index,
            "textEqualAfterWhitespaceNormalization": equal,
            "v1CharacterCount": left.chars().count(),
            "v2CharacterCount": right.chars().count(),
            "v1TokenCount": left_tokens.len(),
            "v2TokenCount": right_tokens.len(),
            "commonTokenCount": common,
            "addedTokenCount": page_added,
            "removedTokenCount": page_removed,
            "v1Text": left,
            "v2Text": right,
            "lineCount": v2_pages.get(index).and_then(|page| page.get("lines")).and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "regionCount": v2_pages.get(index).and_then(|page| page.get("regions")).and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "readingOrder": v2_pages.get(index).and_then(|page| page.get("readingOrder")).cloned().unwrap_or_else(|| json!([]))
        }));
    }
    let line_count = v2_pages
        .iter()
        .filter_map(|page| page.get("lines").and_then(Value::as_array))
        .map(Vec::len)
        .sum::<usize>();
    let anchored_line_count = v2_pages
        .iter()
        .filter_map(|page| page.get("lines").and_then(Value::as_array))
        .flatten()
        .filter(|line| {
            line.get("sourceAnchors")
                .and_then(Value::as_array)
                .is_some_and(|anchors| !anchors.is_empty())
        })
        .count();
    let region_count = v2_pages
        .iter()
        .filter_map(|page| page.get("regions").and_then(Value::as_array))
        .map(Vec::len)
        .sum::<usize>();
    let table_count = v2_pages
        .iter()
        .filter_map(|page| page.get("tables").and_then(Value::as_array))
        .map(Vec::len)
        .sum::<usize>();
    let asset_count = shadow
        .get("assets")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    json!({
        "schemaVersion": "DocumentIRV2CompareReportV1",
        "jobId": job_id,
        "status": "complete",
        "summary": {
            "v1PageCount": v1_pages.len(),
            "v2PageCount": v2_pages.len(),
            "pagesCompared": max_pages,
            "changedPageCount": changed_pages,
            "v1TokenCount": v1_pages.iter().map(v1_page_text).map(|text| tokens(&text).len()).sum::<usize>(),
            "v2TokenCount": v2_pages.iter().map(v2_page_text).map(|text| tokens(&text).len()).sum::<usize>(),
            "addedTokenCount": added,
            "removedTokenCount": removed,
            "lineCount": line_count,
            "regionCount": region_count,
            "tableCount": table_count,
            "assetCount": asset_count,
            "sourceAnchorCoverage": if line_count == 0 {0.0} else {anchored_line_count as f64 / line_count as f64}
        },
        "pages": page_reports,
        "policy": {
            "v1RemainsAuthoritative": true,
            "v2EntersAuthoring": false,
            "differencesRequireReview": true,
            "pdfPerQuestionLlmRepair": false
        }
    })
}
