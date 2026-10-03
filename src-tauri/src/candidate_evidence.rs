//! Source-only, conservative page scoping for the real candidate-import workflow.
use crate::{reconcile::candidate::CandidateChunk, util::job_dir, CommandResult};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

fn section_header(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim().to_ascii_lowercase();
        ["reading passage", "part", "section"].iter().any(|prefix| {
            line.strip_prefix(prefix)
                .map(|rest| {
                    let rest = rest.trim_start();
                    rest.chars()
                        .next()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        })
    })
}

fn auxiliary_page(text: &str) -> bool {
    if text.trim().is_empty() {
        return true;
    }
    text.lines().any(|line| {
        let lower = line.trim().to_ascii_lowercase();
        if lower.contains("answer key")
            || lower.starts_with("answers")
            || lower.starts_with("solutions")
        {
            return true;
        }
        // Compact answer rows without a header, including a single inline answer.
        let line = line.trim();
        let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
        let rest = line[digits.len()..].trim_start_matches(|c: char| {
            c.is_whitespace() || c == '.' || c == ')' || c == ':' || c == '-'
        });
        !digits.is_empty() && !rest.is_empty() && rest.split_whitespace().count() <= 5
    })
}

/// Only use an explicit section heading with a complete matching source range.
/// Unknown boundaries keep the whole PDF. Blank/scanned and answer-like pages
/// are shared across chunks; both sides of every section boundary are retained.
pub(crate) fn plan_pages(pages: &[String], plan: &[CandidateChunk]) -> Option<Vec<Vec<u32>>> {
    if plan.len() < 2 || pages.is_empty() {
        return None;
    }
    if pages.iter().any(|text| {
        let lower = text.to_ascii_lowercase();
        [
            "see page",
            "on page",
            "next page",
            "previous page",
            "following page",
            "refer to page",
        ]
        .iter()
        .any(|phrase| lower.contains(phrase))
    }) {
        return None;
    }
    let mut starts = Vec::new();
    for chunk in plan {
        let wanted: BTreeSet<u32> = chunk.question_numbers.iter().copied().collect();
        let start = pages.iter().position(|text| {
            section_header(text)
                && crate::ielts_grammar::source_coverage::declared_question_blocks(text)
                    .iter()
                    .any(|numbers| numbers.iter().copied().collect::<BTreeSet<_>>() == wanted)
        })?;
        if starts.last().map(|last| *last >= start).unwrap_or(false) {
            return None;
        }
        starts.push(start);
    }
    // An extra section outside the recognised plan makes the boundary ambiguous.
    if pages
        .iter()
        .enumerate()
        .any(|(index, text)| section_header(text) && !starts.contains(&index))
    {
        return None;
    }
    let auxiliary: BTreeSet<usize> = pages
        .iter()
        .enumerate()
        .filter(|(_, text)| auxiliary_page(text))
        .map(|(index, _)| index)
        .collect();
    Some(
        starts
            .iter()
            .enumerate()
            .map(|(index, start)| {
                let end = starts.get(index + 1).copied().unwrap_or(pages.len() - 1);
                let mut selected = auxiliary.clone();
                selected.extend(0..starts[0]); // Shared cover / instructions.
                selected.extend(start.saturating_sub(1)..=end);
                selected
                    .into_iter()
                    .map(|index| (index + 1) as u32)
                    .collect()
            })
            .collect(),
    )
}

/// Persist a real PDF subset and its page manifest. Build the complete new input
/// before returning it: any failure leaves the caller's full-source input intact.
pub(crate) fn scope_input(
    root: &Path,
    job_id: &str,
    input: &Value,
    selected: &[u32],
    original_count: usize,
) -> CommandResult<Value> {
    if selected.is_empty() || selected.len() >= original_count {
        return Ok(input.clone());
    }
    let original = input
        .get("pdfPath")
        .and_then(Value::as_str)
        .ok_or("candidate_pdf_path_missing")?;
    let root_path = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let source_path = fs::canonicalize(original).map_err(|e| e.to_string())?;
    if !source_path.starts_with(&root_path) {
        return Err("candidate_pdf_outside_root".into());
    }
    let bytes = fs::read(&source_path).map_err(|e| e.to_string())?;
    let subset = crate::pdf_geometry::subset_pdf_pages(&bytes, selected, original_count)?;
    let mut digest = Sha256::new();
    digest.update(&bytes);
    for page in selected {
        digest.update(page.to_le_bytes());
    }
    let hash = format!("{:x}", digest.finalize());
    let folder = job_dir(root, job_id)
        .join("cache")
        .join("candidate-evidence");
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(format!("{}.pdf", &hash[..24]));
    fs::write(&path, &subset).map_err(|e| e.to_string())?;
    crate::util::write_json(
        &path.with_extension("json"),
        &json!({
            "originalPageCount":original_count,"sourcePageMap":selected,
            "originalBytes":bytes.len(),"subsetBytes":subset.len(),
            "sourceFileId":input.pointer("/sourceFile/fileId"),"questionNumbers":input.pointer("/chunk/questionNumbers")
        }),
    )?;
    let mut scoped = input.clone();
    scoped["pdfPath"] = json!(path.to_string_lossy());
    scoped["chunk"]["sourcePageMap"] = json!(selected);
    scoped["chunk"]["originalPageCount"] = json!(original_count);
    scoped["chunk"]["evidenceScope"] = json!("source_section");
    let images = input.get("pages").and_then(Value::as_array);
    let mut filtered = Vec::new();
    for (position, original_page) in selected.iter().enumerate() {
        if let Some(page) = images
            .into_iter()
            .flatten()
            .find(|page| page["pageIndex"].as_u64() == Some(u64::from(*original_page)))
        {
            let mut page = page.clone();
            page["sourcePageIndex"] = json!(original_page);
            page["pageIndex"] = json!(position + 1);
            filtered.push(page);
        }
    }
    // Never let image fallback silently omit a selected source page.
    let complete_images = filtered.len() == selected.len()
        && filtered.iter().all(|page| {
            page["images"]
                .as_array()
                .map(|images| !images.is_empty())
                .unwrap_or(false)
        });
    if !complete_images {
        return Err("candidate_scoped_page_images_incomplete".into());
    }
    scoped["pages"] = json!(filtered);
    Ok(scoped)
}

pub(crate) fn scope_or_full(
    root: &Path,
    job_id: &str,
    input: &Value,
    selected: &[u32],
    original_count: usize,
) -> Value {
    match scope_input(root, job_id, input, selected, original_count) {
        Ok(mut scoped) => {
            if scoped.pointer("/chunk/sourcePageMap").is_none() {
                scoped["chunk"]["evidenceScope"] = json!("full_source");
                scoped["chunk"]["originalPageCount"] = json!(original_count);
            }
            scoped
        }
        Err(error) => {
            let mut full = input.clone();
            full["chunk"]["evidenceScope"] = json!("full_source");
            full["chunk"]["evidenceScopeReason"] =
                json!(error.split(':').next().unwrap_or("scope_unavailable"));
            full["chunk"]["originalPageCount"] = json!(original_count);
            full
        }
    }
}

/// All cloud-produced locators are attachment-relative until the completed
/// response (including a missing-field patch) has been assembled. Translate once.
pub(crate) fn restore_source_pages(output: &mut Value, chunk: Option<&Value>) -> CommandResult<()> {
    let Some(map) = chunk
        .and_then(|chunk| chunk.get("sourcePageMap"))
        .and_then(Value::as_array)
    else {
        return Ok(());
    };
    fn restore(value: &mut Value, map: &[Value]) -> CommandResult<()> {
        match value {
            Value::Array(values) => {
                for value in values {
                    restore(value, map)?;
                }
            }
            Value::Object(object) => {
                if let Some(page) = object.get_mut("pageIndex") {
                    let index = page
                        .as_u64()
                        .filter(|page| *page > 0)
                        .ok_or("cloud_candidate_attachment_page_invalid")?;
                    *page = map
                        .get((index - 1) as usize)
                        .filter(|page| page.as_u64().map(|page| page > 0).unwrap_or(false))
                        .ok_or("cloud_candidate_attachment_page_outside_scope")?
                        .clone();
                }
                for (key, value) in object.iter_mut() {
                    if key != "pageIndex" {
                        restore(value, map)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    restore(output, map)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn paper() -> Vec<String> {
        vec![
            "READING PASSAGE 1\nQuestions 1-13",
            "Passage one continuation",
            "Question continuation",
            "READING PASSAGE 2\nQuestions 14-26",
            "Passage two continuation",
            "Question continuation",
            "READING PASSAGE 3\nQuestions 27-40",
            "Passage three continuation",
            "Question continuation",
            "Answer key\n1 TRUE\n14 B\n27 C",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }
    #[test]
    fn scopes_keep_boundary_answer_and_unreadable_pages() {
        let mut pages = paper();
        let plan = crate::reconcile::candidate::plan_candidate_chunks(&pages.join("\n"));
        let scopes = plan_pages(&pages, &plan).unwrap();
        assert_eq!(
            scopes,
            vec![
                vec![1, 2, 3, 4, 10],
                vec![3, 4, 5, 6, 7, 10],
                vec![6, 7, 8, 9, 10]
            ]
        );
        pages[4] = String::new();
        assert!(plan_pages(&pages, &plan)
            .unwrap()
            .iter()
            .all(|scope| scope.contains(&5)));
        pages[4] = "15 B".into();
        assert!(plan_pages(&pages, &plan)
            .unwrap()
            .iter()
            .all(|scope| scope.contains(&5)));
        pages[3] = "Questions 14-26".into();
        assert!(plan_pages(&pages, &plan).is_none());
    }
    #[test]
    fn ambiguous_sections_and_partial_ranges_keep_full_source() {
        let mut pages = paper();
        let plan = crate::reconcile::candidate::plan_candidate_chunks(&pages.join("\n"));
        pages[4] = "READING PASSAGE 2\ncontinuation".into();
        assert!(plan_pages(&pages, &plan).is_none());
        pages = paper();
        pages[3] = "READING PASSAGE 2\nQuestions 14-20".into();
        assert!(plan_pages(&pages, &plan).is_none());
    }
    #[test]
    fn locators_map_to_original_pages_and_outside_scope_is_rejected() {
        let chunk = json!({"sourcePageMap":[3,4,9]});
        let mut output = json!({"answerPageEvidence":[{"pageIndex":3}],"taskGroups":[{"sourceAnchors":[{"pageIndex":1}]}],"unresolvedRegions":[{"pageIndex":2}]});
        restore_source_pages(&mut output, Some(&chunk)).unwrap();
        assert_eq!(output["answerPageEvidence"][0]["pageIndex"], 9);
        assert_eq!(output["taskGroups"][0]["sourceAnchors"][0]["pageIndex"], 3);
        assert_eq!(output["unresolvedRegions"][0]["pageIndex"], 4);
        assert!(restore_source_pages(&mut json!({"pageIndex":4}), Some(&chunk)).is_err());
    }
    #[test]
    fn explicit_cross_page_references_keep_the_whole_source() {
        let mut pages = paper();
        let plan = crate::reconcile::candidate::plan_candidate_chunks(&pages.join("\n"));
        pages[1] = "Use the diagram on page 8 to answer these questions.".into();
        assert!(plan_pages(&pages, &plan).is_none());
    }

    #[test]
    fn failed_scoping_preserves_full_pdf_and_records_the_reason() {
        let root = std::env::temp_dir().join(format!(
            "candidate-evidence-fallback-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("original.pdf");
        fs::write(
            &path,
            include_bytes!("../../fixtures/parser/chunked-reading-evidence.pdf"),
        )
        .unwrap();
        let input = json!({"pdfPath":path.to_string_lossy(),"chunk":{"questionNumbers":[14,15]},"pages":[]});
        let full = scope_or_full(&root, "job", &input, &[3, 4, 10], 10);
        assert_eq!(full["pdfPath"], input["pdfPath"]);
        assert_eq!(full["pages"], input["pages"]);
        assert!(full.pointer("/chunk/sourcePageMap").is_none());
        assert_eq!(full["chunk"]["evidenceScope"], "full_source");
        assert_eq!(
            full["chunk"]["evidenceScopeReason"],
            "candidate_scoped_page_images_incomplete"
        );
        assert!(crate::pdf_geometry::subset_pdf_pages(
            include_bytes!("../../fixtures/parser/chunked-reading-evidence.pdf"),
            &[0, 11],
            10
        )
        .is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
