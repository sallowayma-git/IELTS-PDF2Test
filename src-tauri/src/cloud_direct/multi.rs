//! Cloud-owned passage boundaries, independent normalization and durable fan-out.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const SOURCE_SCOPE_FILE: &str = "cloud-direct-source.json";

pub(super) fn restrict_saved_scope(
    root: &Path,
    job_id: &str,
    texts: &mut Vec<(usize, String)>,
    images: &mut Value,
) -> CommandResult<bool> {
    let path = crate::util::job_dir(root, job_id).join(SOURCE_SCOPE_FILE);
    let conn = crate::library::repository::open_library_connection(root)?;
    let durable = crate::processing::queue::get_job(&conn, job_id)?
        .and_then(|job| job.progress.get("cloudDocumentPages").cloned());
    let scope: Value = if let Some(pages) = durable {
        json!({"pages":pages})
    } else if path.exists() {
        crate::util::read_json(&path)?
    } else {
        return Ok(false);
    };
    let pages: BTreeSet<usize> = scope["pages"]
        .as_array()
        .ok_or("cloud_direct_batch_scope_invalid")?
        .iter()
        .filter_map(Value::as_u64)
        .map(|n| n as usize)
        .collect();
    if pages.is_empty() {
        return Err("cloud_direct_batch_scope_empty".into());
    }
    restrict_pages(texts, images, &pages);
    Ok(true)
}

fn restrict_pages(texts: &mut Vec<(usize, String)>, images: &mut Value, pages: &BTreeSet<usize>) {
    texts.retain(|(n, _)| pages.contains(n));
    if let Some(items) = images.get_mut("pages").and_then(Value::as_array_mut) {
        items.retain(|p| {
            p["pageIndex"]
                .as_u64()
                .is_some_and(|n| pages.contains(&(n as usize)))
        });
    }
}

fn evidence_pages(texts: &[(usize, String)], images: &Value) -> BTreeSet<usize> {
    texts
        .iter()
        .map(|(n, _)| *n)
        .chain(
            images["pages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p["pageIndex"].as_u64())
                .map(|n| n as usize),
        )
        .collect()
}

/// Minimum non-whitespace characters for a text page to count as content that
/// some document should claim. Cover pages and short instructions stay below it.
const COVERAGE_MIN_CHARS: usize = 1500;
const MAX_COVERAGE_FOLLOWUPS: usize = 1;

/// Pages an entry claims. Lenient by design: a hallucinated or out-of-range page
/// number is dropped with a warning instead of rejecting every article; an entry
/// without usable sourcePages keeps the whole evidence as its scope.
struct EntryScope {
    pages: Vec<u32>,
    first_source: u32,
}

fn page_list(
    entry: &Value,
    field: &str,
    available: &BTreeSet<usize>,
    label: &str,
    warnings: &mut Vec<String>,
) -> Vec<u32> {
    let mut pages = Vec::new();
    let Some(items) = entry.get(field) else {
        return pages;
    };
    let Some(items) = items.as_array() else {
        warnings.push(format!("cloud_direct_batch_pages_invalid:{label}:{field}"));
        return pages;
    };
    for item in items {
        match item
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0 && available.contains(&(*n as usize)))
        {
            Some(n) => pages.push(n),
            None => warnings.push(format!(
                "cloud_direct_batch_page_ignored:{label}:{field}:{item}"
            )),
        }
    }
    pages
}

fn entry_scope(
    entry: &Value,
    available: &BTreeSet<usize>,
    label: &str,
    warnings: &mut Vec<String>,
) -> EntryScope {
    let source = page_list(entry, "sourcePages", available, label, warnings);
    let answers = page_list(entry, "answerPages", available, label, warnings);
    let mut pages: BTreeSet<u32> = source.iter().chain(answers.iter()).copied().collect();
    let first_source = match source.iter().min() {
        Some(first) => *first,
        None => {
            warnings.push(format!("cloud_direct_batch_source_pages_missing:{label}"));
            pages.extend(available.iter().map(|n| *n as u32));
            *pages.iter().next().unwrap_or(&1)
        }
    };
    EntryScope {
        pages: pages.into_iter().collect(),
        first_source,
    }
}

fn entry_title(entry: &Value) -> String {
    entry
        .pointer("/document/meta/title")
        .or_else(|| entry.get("title"))
        .or_else(|| entry.pointer("/meta/title"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(80)
        .collect()
}

/// Text pages with substantial content that no document claims as source or
/// answer evidence. Image pages are not judged (usually scanned answer keys).
fn uncovered_pages(entries: &[Value], texts: &[(usize, String)]) -> Vec<usize> {
    let claimed: BTreeSet<usize> = entries
        .iter()
        .flat_map(|entry| {
            ["sourcePages", "answerPages"]
                .into_iter()
                .flat_map(move |field| {
                    entry
                        .get(field)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
        })
        .filter_map(Value::as_u64)
        .map(|n| n as usize)
        .collect();
    texts
        .iter()
        .filter(|(n, text)| {
            !claimed.contains(n)
                && text.chars().filter(|c| !c.is_whitespace()).count() >= COVERAGE_MIN_CHARS
        })
        .map(|(n, _)| *n)
        .collect()
}

fn same_source_pages(a: &Value, b: &Value) -> bool {
    let pages = |v: &Value| {
        v.get("sourcePages")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_u64)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default()
    };
    let (a, b) = (pages(a), pages(b));
    !a.is_empty() && a == b
}

fn push_raw_warnings(raw: &mut Value, warnings: impl IntoIterator<Item = String>) {
    if !raw["warnings"].is_array() {
        raw["warnings"] = json!([]);
    }
    if let Some(list) = raw["warnings"].as_array_mut() {
        list.extend(warnings.into_iter().map(Value::String));
    }
}

/// Coverage follow-up: same cached request prefix (system prompt, examples and
/// every page), plus a short tail naming what was already returned and which
/// pages nobody claimed. Previous document bodies are never re-sent.
#[allow(clippy::too_many_arguments)]
fn coverage_followup(
    root: &Path,
    job: &crate::ImportJob,
    source: &crate::SourceFile,
    profile: &Value,
    profile_id: &str,
    pdf: &Path,
    texts: &[(usize, String)],
    images: &Value,
    api_key: Option<&str>,
    raw: &mut Value,
    calls: &mut usize,
    manifest: bool,
) {
    for _ in 0..MAX_COVERAGE_FOLLOWUPS {
        let entries = raw["documents"].as_array().cloned().unwrap_or_default();
        let missing = uncovered_pages(&entries, texts);
        if missing.is_empty() {
            return;
        }
        let returned: Vec<_> = entries
            .iter()
            .map(|entry| {
                json!({"title": entry_title(entry),
            "sourcePages": entry.get("sourcePages").cloned().unwrap_or(json!([]))})
            })
            .collect();
        let mut input = make_direct_input(
            root, profile, job, profile_id, source, pdf, texts, images, None,
        );
        if manifest {
            input["outputMode"] = json!("document_manifest");
        }
        let body = if manifest {
            "manifest entries (sourcePages, answerPages, testLabel, title only)"
        } else {
            "complete documents entries"
        };
        input["chunkNote"] = json!(format!("COVERAGE FOLLOW-UP. Your previous reply already returned these documents: {}. Original pages {:?} contain substantial text but no returned document lists them in sourcePages or answerPages. Return ONLY {body} for reading passages on those pages that you did NOT return before, in the same documents envelope. Do not repeat documents already returned. If those pages hold no additional passage (for example instructions or answer keys that belong to returned documents), return {{\"documents\":[],\"warnings\":[\"<what those pages contain>\"]}}.", Value::Array(returned), missing));
        *calls += 1;
        match run_llm_gateway(root, &job.job_id, DIRECT_COMMAND_NAME, &input, api_key) {
            Ok(reply) => {
                let mut added = 0;
                for entry in reply["documents"].as_array().into_iter().flatten() {
                    if entry.is_object()
                        && !entries.iter().any(|known| same_source_pages(known, entry))
                    {
                        if let Some(list) = raw["documents"].as_array_mut() {
                            list.push(entry.clone());
                            added += 1;
                        }
                    }
                }
                push_raw_warnings(
                    raw,
                    reply["warnings"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string),
                );
                if added == 0 {
                    break;
                }
            }
            Err(error) => {
                push_raw_warnings(
                    raw,
                    [format!(
                        "cloud_direct_coverage_followup_failed:{}",
                        error.chars().take(120).collect::<String>()
                    )],
                );
                break;
            }
        }
    }
    let still = uncovered_pages(
        raw["documents"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        texts,
    );
    if !still.is_empty() {
        push_raw_warnings(
            raw,
            [format!(
                "cloud_direct_pages_uncovered:{}",
                still
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            )],
        );
    }
}

/// A scoped re-recognition owns one item: keep its first returned entry.
pub(super) fn keep_first_scoped_document(raw: &mut Value) {
    let extra = raw["documents"]
        .as_array()
        .map_or(0, |docs| docs.len().saturating_sub(1));
    if extra > 0 {
        if let Some(docs) = raw["documents"].as_array_mut() {
            docs.truncate(1);
        }
        push_raw_warnings(
            raw,
            [format!(
                "cloud_direct_scoped_extra_documents_ignored:{extra}"
            )],
        );
    }
}

fn is_empty_document(document: &Value) -> bool {
    document
        .pointer("/passage/content")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
        && document
            .get("taskGroups")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
}

fn child_id(parent: &str, first_page: u32, ordinal_on_page: usize) -> String {
    let digest = Sha256::digest(format!("{parent}:{first_page}:{ordinal_on_page}").as_bytes());
    format!("cloud-{:x}", digest)[..30].to_string()
}

/// Normalized batch: outcomes in physical source order, plus the index of each
/// outcome's entry in the raw `documents` array (repair targets raw entries).
pub(super) struct NormalizedBatch {
    pub outcomes: Vec<DirectExamOutcome>,
    pub warnings: Vec<String>,
    pub raw_indexes: Vec<usize>,
}

/// Normalizes every entry independently. One unusable article is skipped with
/// a warning; the batch fails only when no article is usable.
pub(super) fn normalize_batch(
    root: &Path,
    job: &crate::ImportJob,
    source: &crate::SourceFile,
    raw: &Value,
    texts: &[(usize, String)],
    images: &Value,
) -> CommandResult<NormalizedBatch> {
    let entries = raw["documents"]
        .as_array()
        .filter(|docs| !docs.is_empty())
        .ok_or("cloud_direct_batch_documents_empty")?;
    let available = evidence_pages(texts, images);
    if available.is_empty() {
        return Err("cloud_direct_batch_no_evidence".into());
    }
    let mut warnings = Vec::new();
    let mut entries: Vec<_> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.is_object())
        .map(|(index, entry)| {
            (
                index,
                entry,
                entry_scope(
                    entry,
                    &available,
                    &format!("documents[{index}]"),
                    &mut warnings,
                ),
            )
        })
        .collect();
    // Physical source order owns stable item identity; repeated q1/P1 in another
    // test never shares an answer map with its predecessor.
    entries.sort_by_key(|(_, _, scope)| scope.first_source);
    let mut ordinals = BTreeMap::<u32, usize>::new();
    let mut outcomes = Vec::new();
    let mut raw_indexes = Vec::new();
    let mut first_error = None;
    for (index, entry, scope) in entries.iter() {
        let index = *index;
        let ordinal = ordinals.entry(scope.first_source).or_default();
        *ordinal += 1;
        let mut identity = DirectIdentity::new(job, source, root);
        // The first usable article reuses the placeholder item; ids of the
        // others depend only on physical position, never on failures before them.
        if !outcomes.is_empty() {
            identity.job_id = child_id(&job.job_id, scope.first_source, *ordinal);
            identity.source_document_id = format!("document-{}", identity.job_id);
            identity.exam["examId"] = json!(format!("reading-{}", &identity.job_id[6..]));
        }
        let selected_texts: Vec<_> = texts
            .iter()
            .filter(|(n, _)| scope.pages.contains(&(*n as u32)))
            .cloned()
            .collect();
        identity.part_evidence =
            json!({"pages": selected_texts.iter().map(|(_, text)| text).collect::<Vec<_>>()});
        let document_raw = entry.get("document").unwrap_or(entry);
        if let Some(title) = document_raw
            .pointer("/meta/title")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            identity.exam["title"] = json!(title);
        }
        let mut document = match normalize_direct_output(root, document_raw, &identity) {
            Ok(document) if !is_empty_document(&document) => document,
            Ok(_) => {
                warnings.push(format!(
                    "cloud_direct_batch_document_empty:{index}:{}",
                    entry_title(entry)
                ));
                continue;
            }
            Err(error) => {
                warnings.push(format!(
                    "cloud_direct_batch_document_skipped:{index}:{}:{error}",
                    entry_title(entry)
                ));
                first_error.get_or_insert(format!("cloud_direct_batch_document:{index}:{error}"));
                continue;
            }
        };
        let compile_issues = direct_structural_issues(&document);
        let mut doc_warnings = document_warnings(&document);
        let fidelity = assess_passage_fidelity(&mut document, &selected_texts, &mut doc_warnings);
        outcomes.push(DirectExamOutcome {
            document,
            calls: 0,
            chunked: false,
            compile_issues,
            warnings: doc_warnings,
            fidelity,
            source_pages: scope.pages.clone(),
            additional_documents: Vec::new(),
        });
        raw_indexes.push(index);
    }
    if outcomes.is_empty() {
        return Err(first_error.unwrap_or_else(|| "cloud_direct_batch_documents_empty".into()));
    }
    Ok(NormalizedBatch {
        outcomes,
        warnings,
        raw_indexes,
    })
}

/// Same-conversation repair of ONLY the documents that failed the compiler.
/// The model's previous turn is trimmed to those documents, so accepted
/// articles are neither re-sent nor re-generated (output tokens dominate cost).
/// Returns the repaired raw envelope when it is no worse than the original.
#[allow(clippy::too_many_arguments)]
fn repair_failed_documents(
    root: &Path,
    job: &crate::ImportJob,
    source: &crate::SourceFile,
    profile: &Value,
    profile_id: &str,
    pdf: &Path,
    texts: &[(usize, String)],
    images: &Value,
    api_key: Option<&str>,
    raw: &Value,
    batch: &NormalizedBatch,
) -> Result<Option<(Value, NormalizedBatch)>, String> {
    let failed: Vec<(usize, usize)> = batch
        .outcomes
        .iter()
        .enumerate()
        .filter(|(_, doc)| !doc.compile_issues.is_empty())
        .map(|(pos, _)| (pos, batch.raw_indexes[pos]))
        .collect();
    if failed.is_empty() {
        return Ok(None);
    }
    let originals = raw["documents"].as_array().cloned().unwrap_or_default();
    let subset: Vec<Value> = failed
        .iter()
        .map(|(_, raw_index)| originals[*raw_index].clone())
        .collect();
    let issues: Vec<_> = failed
        .iter()
        .enumerate()
        .flat_map(|(k, (pos, _))| {
            batch.outcomes[*pos]
                .compile_issues
                .iter()
                .map(move |issue| {
                    let mut issue = issue.clone();
                    issue.target_id = format!("documents[{k}]:{}", issue.target_id);
                    issue
                })
        })
        .collect();
    let previous = json!({"documents": subset});
    let mut input = make_direct_input(
        root,
        profile,
        job,
        profile_id,
        source,
        pdf,
        texts,
        images,
        Some((&previous, &issues)),
    );
    input["repair"]["instruction"] = json!(format!("Your previous reply above is trimmed to the {n} document(s) that failed the exam compiler; the other documents were accepted and are not shown. Fix ONLY the listed structural problems and return the documents envelope with exactly these {n} document(s), complete and in the same order. Keep sourcePages, answerPages and all content (passage text, questions, options, answers) unchanged unless a listed problem requires it.", n = failed.len()));
    let repaired = run_llm_gateway(root, &job.job_id, DIRECT_COMMAND_NAME, &input, api_key)?;
    let returned = repaired["documents"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if returned.len() != failed.len() {
        return Err("cloud_direct_repair_document_count_changed".into());
    }
    let mut merged = raw.clone();
    for ((_, raw_index), fixed) in failed.iter().zip(returned) {
        // Boundaries stay the ones already accepted; only the body is replaced.
        let body = fixed.get("document").cloned().unwrap_or(fixed);
        merged["documents"][*raw_index]["document"] = body;
    }
    let next = normalize_batch(root, job, source, &merged, texts, images)?;
    let issue_count = |b: &NormalizedBatch| {
        b.outcomes
            .iter()
            .map(|d| d.compile_issues.len())
            .sum::<usize>()
    };
    if next.outcomes.len() < batch.outcomes.len() || issue_count(&next) > issue_count(batch) {
        return Err("cloud_direct_repair_not_better".into());
    }
    Ok(Some((merged, next)))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_batch(
    root: &Path,
    job: &crate::ImportJob,
    source: &crate::SourceFile,
    profile: &Value,
    profile_id: &str,
    pdf: &Path,
    texts: &[(usize, String)],
    images: &Value,
    api_key: Option<&str>,
    mut raw: Value,
    mut calls: usize,
    chunked: bool,
    evidence_warnings: &[String],
    check_coverage: bool,
) -> CommandResult<DirectExamOutcome> {
    if check_coverage
        && raw["documents"]
            .as_array()
            .is_some_and(|docs| !docs.is_empty())
    {
        coverage_followup(
            root, job, source, profile, profile_id, pdf, texts, images, api_key, &mut raw,
            &mut calls, false,
        );
    }
    let mut batch = normalize_batch(root, job, source, &raw, texts, images)?;
    let mut repair_warning = None;
    if batch
        .outcomes
        .iter()
        .any(|doc| !doc.compile_issues.is_empty())
        && calls < MAX_DIRECT_CALLS
    {
        calls += 1;
        match repair_failed_documents(
            root, job, source, profile, profile_id, pdf, texts, images, api_key, &raw, &batch,
        ) {
            Ok(Some((merged, next))) => {
                raw = merged;
                batch = next;
            }
            Ok(None) => {}
            Err(e) => {
                repair_warning = Some(format!(
                    "cloud_direct_repair_failed:{}",
                    e.chars().take(160).collect::<String>()
                ))
            }
        }
    }
    let NormalizedBatch {
        outcomes: mut documents,
        warnings: batch_warnings,
        ..
    } = batch;
    // Envelope warnings that name one article's title belong to that article
    // only; the rest (coverage, skipped pages) concern every document.
    let titles: Vec<String> = documents
        .iter()
        .map(|doc| {
            doc.document["exam"]["title"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_lowercase()
        })
        .collect();
    let envelope: Vec<String> = raw["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    for (position, document) in documents.iter_mut().enumerate() {
        document.warnings.extend(evidence_warnings.iter().cloned());
        document.warnings.extend(
            envelope
                .iter()
                .filter(|warning| {
                    let lower = warning.to_lowercase();
                    let named: Vec<usize> = titles
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| t.len() >= 4 && lower.contains(t.as_str()))
                        .map(|(i, _)| i)
                        .collect();
                    named.is_empty() || named.contains(&position)
                })
                .cloned(),
        );
        document.warnings.extend(batch_warnings.iter().cloned());
        if let Some(warning) = &repair_warning {
            document.warnings.push(warning.clone());
        }
        if !document.compile_issues.is_empty() {
            attach_compile_issues(&mut document.document, &document.compile_issues);
        }
        document.chunked = chunked;
    }
    let mut first = documents.remove(0);
    first.calls = calls;
    first.additional_documents = documents;
    Ok(first)
}

/// Long output: let the cloud identify article boundaries first, then request
/// each complete article with its own answer pages. Never merge by question no.
/// One article that fails or is truncated is reported, not fatal for the rest.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_truncated_documents(
    root: &Path,
    job: &crate::ImportJob,
    source: &crate::SourceFile,
    profile: &Value,
    profile_id: &str,
    pdf: &Path,
    texts: &[(usize, String)],
    images: &Value,
    api_key: Option<&str>,
    mut calls: usize,
    warnings: &[String],
    check_coverage: bool,
) -> CommandResult<DirectExamOutcome> {
    let mut manifest_input = make_direct_input(
        root, profile, job, profile_id, source, pdf, texts, images, None,
    );
    manifest_input["outputMode"] = json!("document_manifest");
    manifest_input["chunkNote"] = json!("The complete output exceeded the response limit. Return ONLY a short manifest: {documents:[{sourcePages:[original 1-based question/article pages],answerPages:[matching original answer pages],title:string}],warnings:[]}. One entry per complete Reading passage, including every test. Do not transcribe document bodies in this response. Source pages may overlap when two passages share a page. Do not merge tests when numbering restarts.");
    let mut manifest = run_llm_gateway(
        root,
        &job.job_id,
        DIRECT_COMMAND_NAME,
        &manifest_input,
        api_key,
    )?;
    calls += 1;
    if manifest["documents"].as_array().is_none_or(Vec::is_empty) {
        return Err("cloud_direct_batch_manifest_empty".into());
    }
    if check_coverage {
        coverage_followup(
            root,
            job,
            source,
            profile,
            profile_id,
            pdf,
            texts,
            images,
            api_key,
            &mut manifest,
            &mut calls,
            true,
        );
    }
    let entries = manifest["documents"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let available = evidence_pages(texts, images);
    let mut collected = Vec::new();
    let mut skipped = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let mut scope_warnings = Vec::new();
        let scope = entry_scope(
            entry,
            &available,
            &format!("manifest[{index}]"),
            &mut scope_warnings,
        );
        skipped.extend(scope_warnings);
        let mut scoped_texts = texts.to_vec();
        let mut scoped_images = images.clone();
        restrict_pages(
            &mut scoped_texts,
            &mut scoped_images,
            &scope.pages.iter().map(|n| *n as usize).collect(),
        );
        let mut input = make_direct_input(
            root,
            profile,
            job,
            profile_id,
            source,
            pdf,
            &scoped_texts,
            &scoped_images,
            None,
        );
        input["chunkNote"] = json!(format!("Convert exactly ONE complete article from this manifest entry: {entry}. Only the supplied original source pages are in scope. Return documents with exactly one entry, including the whole article, its own questions and its own answers. Keep original page numbers. A shared answer page may contain other tests: ignore their answers."));
        calls += 1;
        let output = match run_llm_gateway(root, &job.job_id, DIRECT_COMMAND_NAME, &input, api_key)
        {
            Ok(output) => output,
            Err(error) => {
                skipped.push(format!(
                    "cloud_direct_batch_article_failed:{index}:{}:{}",
                    entry_title(entry),
                    error.chars().take(120).collect::<String>()
                ));
                continue;
            }
        };
        let doc = match output["documents"].as_array() {
            Some(docs) if docs.is_empty() => {
                skipped.push(format!(
                    "cloud_direct_batch_article_empty:{index}:{}",
                    entry_title(entry)
                ));
                continue;
            }
            Some(docs) => {
                if docs.len() > 1 {
                    skipped.push(format!(
                        "cloud_direct_scoped_extra_documents_ignored:{}",
                        docs.len() - 1
                    ));
                }
                docs[0].get("document").unwrap_or(&docs[0]).clone()
            }
            None => output,
        };
        let mut entry = entry.clone();
        entry["document"] = doc;
        collected.push(entry);
    }
    if collected.is_empty() {
        return Err(format!(
            "cloud_direct_batch_articles_all_failed:{}",
            skipped.join(";").chars().take(200).collect::<String>()
        ));
    }
    let mut batch_warnings: Vec<Value> =
        manifest["warnings"].as_array().cloned().unwrap_or_default();
    batch_warnings.extend(skipped.into_iter().map(Value::String));
    finish_batch(
        root,
        job,
        source,
        profile,
        profile_id,
        pdf,
        texts,
        images,
        api_key,
        json!({"documents": collected, "warnings": batch_warnings}),
        calls,
        true,
        warnings,
        false,
    )
}

/// Child ids and source scopes are deterministic. All new canonical rows and
/// terminal queue rows commit together; replays never replace edited siblings.
pub(crate) fn persist_direct_documents(
    root: &Path,
    job_id: &str,
    outcome: &DirectExamOutcome,
    worker_id: &str,
) -> CommandResult<Vec<String>> {
    if outcome.additional_documents.is_empty() && outcome.source_pages.is_empty() {
        write_direct_result(root, job_id, outcome)?;
        write_direct_canonical_leased(root, job_id, &outcome.document, worker_id)?;
        return Ok(vec![job_id.to_string()]);
    }
    let parent = load_job(root, job_id)?;
    let parent_dir = crate::util::job_dir(root, job_id);
    let mut conn = crate::library::repository::open_library_connection(root)?;
    check_direct_lease(&conn, job_id, Some(worker_id))?;
    let progress = crate::processing::queue::get_job(&conn, job_id)?
        .ok_or("cloud_direct_batch_job_missing")?
        .progress;
    let documents: Vec<_> = std::iter::once(outcome)
        .chain(outcome.additional_documents.iter())
        .collect();
    let ids: Vec<String> = documents
        .iter()
        .map(|doc| doc.document["jobId"].as_str().unwrap_or(job_id).to_string())
        .collect();
    let mut staged_dirs = Vec::new();
    let result = (|| -> CommandResult<()> {
        for (doc, id) in documents.iter().zip(&ids).skip(1) {
            if crate::library::repository::get_item(&conn, id)?.is_some() {
                continue;
            }
            let dir = crate::util::job_dir(root, id);
            if !dir.exists() {
                staged_dirs.push(dir.clone());
            }
            std::fs::create_dir_all(dir.join("uploads")).map_err(|e| e.to_string())?;
            let mut job = parent.clone();
            job.job_id = id.clone();
            job.title = doc.document["exam"]["title"]
                .as_str()
                .unwrap_or(&parent.title)
                .to_string();
            job.status = crate::JobStatus::NeedsReview;
            job.current_step = crate::WorkflowStep::Authoring;
            job.category = doc.document["exam"]["category"]
                .as_str()
                .map(str::to_string);
            for source in &job.source_files {
                std::fs::copy(
                    parent_dir.join("uploads").join(&source.stored_name),
                    dir.join("uploads").join(&source.stored_name),
                )
                .map_err(|e| format!("cloud_direct_batch_copy:{e}"))?;
            }
            // Do not invoke legacy save_job's independent database write here.
            crate::util::write_json(&dir.join("job.json"), &job)?;
            let budget = parent_dir.join(crate::processing::commands::CLOUD_TOKEN_BUDGET_FILE);
            if budget.exists() {
                std::fs::copy(
                    budget,
                    dir.join(crate::processing::commands::CLOUD_TOKEN_BUDGET_FILE),
                )
                .map_err(|e| e.to_string())?;
            }
            crate::util::write_json(
                &dir.join(SOURCE_SCOPE_FILE),
                &json!({"parentJobId":job_id,"pages":doc.source_pages}),
            )?;
            write_direct_result(root, id, doc)?;
        }
        let had_primary = crate::library::repository::get_canonical_ds(&conn, job_id)?.is_some();
        // A replay of an already committed fan-out must preserve human edits.
        // A scoped one-document re-recognition retains the existing replace flow.
        if had_primary && outcome.additional_documents.is_empty() {
            write_direct_canonical_leased(root, job_id, &outcome.document, worker_id)?;
        }
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        check_direct_lease(&tx, job_id, Some(worker_id))?;
        for (index, (doc, id)) in documents.iter().zip(&ids).enumerate() {
            let title = doc.document["exam"]["title"]
                .as_str()
                .unwrap_or(&parent.title);
            crate::library::repository::upsert_item_shell(
                &tx,
                &crate::library::repository::UpsertItemInput {
                    id,
                    modality: "reading",
                    title,
                    status: "action_required",
                    source_asset_id: None,
                },
            )?;
            let seeded = crate::library::repository::seed_canonical_ds(
                &tx,
                id,
                &doc.document.to_string(),
                "action_required",
            )?;
            if seeded {
                crate::library::repository::rename_item(&tx, id, title)?;
                crate::library::repository::set_item_part(
                    &tx,
                    id,
                    doc.document["exam"]["category"].as_str(),
                    Some("content"),
                )?;
            }
            if index > 0 {
                let mut child_progress = progress.clone();
                child_progress["cloudDocumentPages"] = json!(doc.source_pages);
                child_progress["cloudParentJobId"] = json!(job_id);
                let inserted = crate::processing::queue::enqueue(&tx, id, id, id, &child_progress)?;
                if inserted {
                    tx.execute("UPDATE processing_jobs_v2 SET stage='ready_for_review',local_status='not_run',cloud_status='succeeded',reconcile_status='not_run',event_seq=1 WHERE id=?1", [id]).map_err(|e| e.to_string())?;
                    let status = if doc
                        .document
                        .pointer("/quality/state")
                        .and_then(Value::as_str)
                        == Some("ready")
                    {
                        "ready"
                    } else {
                        "action_required"
                    };
                    crate::library::repository::set_item_status(&tx, id, status)?;
                }
            }
        }
        let mut primary_progress = progress.clone();
        if !outcome.additional_documents.is_empty()
            || primary_progress.get("cloudDocumentPages").is_some()
        {
            primary_progress["cloudDocumentPages"] = json!(outcome.source_pages);
            primary_progress["cloudParentJobId"] = json!(job_id);
        }
        // The commit is the success/cancellation linearization point for the
        // entire batch, including its first row. A later cancel cannot leave
        // children ready while marking their successfully committed parent cancelled.
        tx.execute("UPDATE processing_jobs_v2 SET stage='ready_for_review',local_status='not_run',cloud_status='succeeded',reconcile_status='not_run',progress_json=?2,lease_owner=NULL,lease_expires_at=NULL,event_seq=event_seq+1 WHERE id=?1", rusqlite::params![job_id, primary_progress.to_string()]).map_err(|e| e.to_string())?;
        let primary = crate::library::repository::get_canonical_ds(&tx, job_id)?
            .ok_or("cloud_direct_batch_primary_missing")?
            .0;
        let status = if primary.pointer("/quality/state").and_then(Value::as_str) == Some("ready") {
            "ready"
        } else {
            "action_required"
        };
        crate::library::repository::set_item_status(&tx, job_id, status)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        for dir in staged_dirs {
            let _ = std::fs::remove_dir_all(dir);
        }
        return result.map(|_| ids);
    }
    // Canonical rows and source scopes above are durable even if diagnostics
    // cannot be written after the successful transaction.
    if let Err(error) = write_direct_result(root, job_id, outcome) {
        eprintln!("[cloud-direct] diagnostics: {error}");
    }
    if !outcome.source_pages.is_empty()
        && (!outcome.additional_documents.is_empty() || parent_dir.join(SOURCE_SCOPE_FILE).exists())
    {
        if let Err(error) = crate::util::write_json(
            &parent_dir.join(SOURCE_SCOPE_FILE),
            &json!({"parentJobId":job_id,"pages":outcome.source_pages}),
        ) {
            eprintln!("[cloud-direct] source scope cache: {error}");
        }
    }
    Ok(ids)
}
