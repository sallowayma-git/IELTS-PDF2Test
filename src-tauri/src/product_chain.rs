//! Real backend chain test (plan P0-T02 / section 19.7).
//!
//! The convergence plan makes a real backend chain the entry gate for everything else: the UI
//! flow smoke runs against `devFallbackBackend` in a browser and therefore proves nothing about
//! Rust, SQLite, the artifact layout, or the NAS publisher. This module drives the real command
//! cores against a temp app root so the chain has a runnable gate.
//!
//! Two tests, split where the product genuinely splits:
//!
//! 1. `product_chain_pdf_import_reaches_editable_session_and_persists_one_character_edit`
//!    covers import -> local pipeline -> V2 session -> one-character edit -> reload. This is the
//!    path the new workspace uses, and it closes two coverage holes the backend audit found:
//!    `get_authoring_v2_core` had zero test callers, and both V2 shadow writes fail silently
//!    (they replace the shadow with a `.error.json` and let the pipeline return Ok), so a missing
//!    shadow only surfaced two commands later as AUTHORING_V2_NOT_AVAILABLE.
//!
//! 2. `product_chain_ready_authoring_exports_and_publishes_to_nas` covers export -> NAS publish.
//!    It cannot start from a public fixture: publish requires `quality.state == "ready"`, which
//!    requires every task score >= 0.92, document score >= 0.95 and source coverage >= 0.995
//!    against a physical DocumentIRV2 shadow. No fixture PDF in the repo reaches that, and the
//!    private corpus that does is intentionally not committed. So this half starts from the
//!    proven-ready authoring fixture plus a matching physical shadow and drives the real
//!    `export_authoring_v2_core` + `publish_nas_package_v2_core`, including the export-binding
//!    check that makes export a mandatory link before publish.

use crate::authoring_v2_commands::{
    apply_authoring_v2_patches_core, export_authoring_v2_core, get_authoring_v2_core,
    AUTHORING_V2_SHADOW_FILE,
};
use crate::auto_pipeline::run_auto_pipeline_core;
use crate::job_store::{load_job, make_job, save_job};
use crate::nas_package_v2::publish_nas_package_v2_core;
use crate::pdf_facts_shadow::SHADOW_ARTIFACT_FILE as DOCUMENT_V2_SHADOW_FILE;
use crate::recognition::QUESTION_LAYOUT_GRAPH_ARTIFACT_FILE as QUESTION_LAYOUT_GRAPH_FILE;
use crate::util::{
    ensure_app_dirs, ensure_job_dirs, file_type_from_name, hash_file_or_path, job_dir,
    sanitize_filename, write_bytes, write_json,
};
use crate::{AutoPipelineInput, CreateJobInput, ImportJob, SourceFile};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const READY_AUTHORING_FIXTURE: &str =
    "fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json";

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("product-chain-{label}-{}", Uuid::new_v4().simple()))
}

fn workspace_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

fn chain_job(title: &str) -> ImportJob {
    make_job(CreateJobInput {
        title: Some(title.to_string()),
        category: Some("P1".to_string()),
        frequency: Some("medium".to_string()),
        tags: Some(vec!["product-chain".to_string()]),
        llm_profile_id: None,
    })
}

/// Stage the fixture exactly the way `import_source_file_core` would, including its
/// `<sha[..8]>-<sanitized>` stored-name scheme, so later stages read a realistic job directory.
fn attach_source(root: &Path, job: &mut ImportJob, fixture_relative: &str, role: &str) {
    let fixture = workspace_path(fixture_relative);
    let original_name = fixture
        .file_name()
        .and_then(|value| value.to_str())
        .expect("fixture name must be UTF-8")
        .to_string();
    let (hash, size, bytes) = hash_file_or_path(&fixture).expect("fixture must be readable");
    let stored_name = format!("{}-{}", &hash[..8], sanitize_filename(&original_name));
    ensure_job_dirs(&job_dir(root, &job.job_id)).unwrap();
    write_bytes(
        &job_dir(root, &job.job_id)
            .join("uploads")
            .join(&stored_name),
        &bytes.expect("fixture bytes must be present"),
    )
    .unwrap();
    job.source_files.push(SourceFile {
        file_id: format!("file-{}", Uuid::new_v4().simple()),
        original_name,
        stored_name,
        file_type: file_type_from_name(fixture_relative).to_string(),
        sha256: hash,
        size_bytes: size,
        role: role.to_string(),
        imported_at: Utc::now(),
    });
}

/// Both V2 shadow writers swallow their errors into a sibling `.error.json`. Assert presence and
/// surface that file in the panic message, otherwise the real cause stays two commands away.
fn assert_shadow(dir: &Path, shadow_file: &str, stage: &str) {
    let shadow = dir.join(shadow_file);
    if shadow.is_file() {
        return;
    }
    let error_file = dir.join(format!(
        "{}.error.json",
        shadow_file.trim_end_matches(".json")
    ));
    let detail = fs::read_to_string(&error_file)
        .unwrap_or_else(|_| format!("no {} present either", error_file.display()));
    panic!(
        "{stage}: {} missing after the pipeline; writer error was: {detail}",
        shadow.display()
    );
}

/// P4-T02: the auto import path must derive `QuestionLayoutGraphV1` from the same physical
/// `DocumentIRV2` the authoring layer consumes.
///
/// Deliberately format-agnostic and invoked from both the PDF and the DOCX chain test: the P4-T01
/// defect was exactly a PDF-only branch, so a new physical-derived artifact must be asserted on
/// both branches through one helper rather than re-created per format.
fn assert_question_layout_graph(dir: &Path, job: &ImportJob, physical: &Value) {
    let path = dir.join(QUESTION_LAYOUT_GRAPH_FILE);
    assert!(
        path.is_file(),
        "the auto pipeline must materialize the question layout graph at {}",
        path.display()
    );
    let graph: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        graph.get("schemaVersion").and_then(Value::as_str),
        Some("QuestionLayoutGraphV1"),
        "the layout graph must be schema-gated: {graph}"
    );
    assert_eq!(
        graph.get("jobId").and_then(Value::as_str),
        Some(job.job_id.as_str()),
        "the layout graph must be bound to the originating job"
    );

    // Derived, not fabricated: page geometry must be the physical page geometry.
    let graph_pages = graph
        .get("pages")
        .and_then(Value::as_array)
        .expect("layout graph pages");
    let physical_pages = physical
        .get("pages")
        .and_then(Value::as_array)
        .expect("physical pages");
    assert_eq!(
        graph.get("documentId").and_then(Value::as_str),
        physical.get("documentId").and_then(Value::as_str),
        "the layout graph must be derived from the same physical document"
    );
    assert_eq!(graph_pages.len(), physical_pages.len());
    for (graph_page, physical_page) in graph_pages.iter().zip(physical_pages) {
        assert_eq!(
            graph_page.get("pageIndex").and_then(Value::as_u64),
            physical_page.get("pageIndex").and_then(Value::as_u64)
        );
        assert_eq!(
            graph_page.get("widthPt").and_then(Value::as_f64),
            physical_page.get("widthPt").and_then(Value::as_f64)
        );
        assert_eq!(
            graph_page.get("heightPt").and_then(Value::as_f64),
            physical_page.get("heightPt").and_then(Value::as_f64)
        );
    }

    // Every question block must trace back to a number token the page reported, and no block may
    // silently carry an empty stem: either it has physical nodes or it declares PROMPT_EMPTY.
    let token_values: Vec<u64> = graph_pages
        .iter()
        .flat_map(|page| {
            page.get("numberTokens")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|token| token.get("value").and_then(Value::as_u64))
        .collect();
    let blocks = graph
        .get("questionBlocks")
        .and_then(Value::as_array)
        .expect("question blocks");
    for block in blocks {
        let number = block
            .get("questionNumber")
            .and_then(Value::as_u64)
            .expect("question number");
        assert!(
            token_values.contains(&number),
            "block {number} has no matching number token on any page"
        );
        let stem_empty = block
            .get("stemNodeIds")
            .and_then(Value::as_array)
            .is_some_and(|ids| ids.is_empty());
        let declares_empty = block
            .get("ambiguities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|code| code.as_str() == Some("PROMPT_EMPTY"));
        assert!(
            !stem_empty || declares_empty,
            "an empty stem must be reported as PROMPT_EMPTY, never silently: {block}"
        );
    }
    assert!(
        graph.get("unassignedEvidence").is_some_and(Value::is_array),
        "the layout graph must carry its unassigned-evidence ledger: {graph}"
    );
}

/// First text node id + text in the authoring document, in document order.
fn first_text_node(value: &Value) -> Option<(String, String)> {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("text") {
                if let (Some(id), Some(text)) = (
                    map.get("id").and_then(Value::as_str),
                    map.get("text").and_then(Value::as_str),
                ) {
                    if !text.trim().is_empty() {
                        return Some((id.to_string(), text.to_string()));
                    }
                }
            }
            map.values().find_map(first_text_node)
        }
        Value::Array(items) => items.iter().find_map(first_text_node),
        _ => None,
    }
}

fn node_text(value: &Value, node_id: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if map.get("id").and_then(Value::as_str) == Some(node_id) {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    return Some(text.to_string());
                }
            }
            map.values().find_map(|child| node_text(child, node_id))
        }
        Value::Array(items) => items.iter().find_map(|child| node_text(child, node_id)),
        _ => None,
    }
}

/// Every source node id the authoring document anchors to. The physical region must claim exactly
/// this set for the quality gate to report full source coverage -- collecting every `id` field
/// instead would add task/slot/option ids that are not source nodes and the coverage check fails.
/// The `quality` subtree is skipped because its own anchors are derived, not source-backed.
fn collect_anchored_node_ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_anchored_node_ids(item, out);
            }
        }
        Value::Object(object) => {
            for node_id in object
                .get("sourceAnchors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|anchor| anchor.get("nodeIds").and_then(Value::as_array))
                .flatten()
                .filter_map(Value::as_str)
            {
                out.insert(node_id.to_string());
            }
            for (key, child) in object {
                if key != "quality" {
                    collect_anchored_node_ids(child, out);
                }
            }
        }
        _ => {}
    }
}

/// A physical DocumentIRV2 shadow whose single region claims every authoring node id, which is
/// what the quality gate needs to report full source coverage. Mirrors the shape proven in
/// `ielts_grammar::quality` tests.
fn physical_shadow_for(authoring: &Value) -> Value {
    let mut node_ids = BTreeSet::new();
    collect_anchored_node_ids(authoring, &mut node_ids);
    let source_hash = "a".repeat(64);
    // The shadow is only accepted when its schemaVersion, jobId and documentId match the authoring
    // document AND at least one of its sourceFileIds is declared in `exam.sourceFiles`. A mismatch
    // makes the export gate silently fall back to "no physical shadow", i.e. never ready --
    // surfaced only as an opaque `quality_state=review_required`.
    let source_file_id = authoring
        .pointer("/exam/sourceFiles/0/sourceFileId")
        .and_then(Value::as_str)
        .unwrap_or("source-pdf-1")
        .to_string();
    let anchor = json!({
        "sourceFileId": source_file_id,
        "pageIndex": 0,
        "nodeIds": ["region-question-surface"],
        "extractionMode": "pdf_native",
        "sourceHash": source_hash
    });
    json!({
        "schemaVersion": "DocumentIRV2",
        "documentId": authoring.get("sourceDocumentId").and_then(Value::as_str).unwrap_or("document-1"),
        "jobId": authoring.get("jobId").and_then(Value::as_str).unwrap_or("job-1"),
        "sourceFiles": [{
            "sourceFileId": source_file_id,
            "originalName": "early-approaches.pdf",
            "mediaType": "application/pdf",
            "sha256": source_hash,
            "byteLength": 1,
            "role": "question_paper"
        }],
        "pages": [{
            "pageIndex": 0,
            "widthPt": 612.0,
            "heightPt": 792.0,
            "rotation": 0,
            "glyphs": [],
            "spans": [],
            "lines": [],
            "regions": [{
                "id": "region-question-surface",
                "kind": "text",
                "bbox": {"x": 10.0, "y": 10.0, "width": 500.0, "height": 200.0, "unit": "pt", "origin": "top-left", "pageRotation": 0},
                "childLineIds": node_ids,
                "childObjectIds": [],
                "confidence": 1.0,
                "sourceAnchors": [anchor]
            }],
            "vectorPaths": [],
            "tables": [],
            "assetIds": [],
            "readingOrder": ["region-question-surface"],
            "quality": {
                "classification": "born_digital",
                "nativeCharacterCount": 100,
                "unicodeErrorRatio": 0.0,
                "duplicateTextRatio": 0.0,
                "imageCoverageRatio": 0.0,
                "textCoverageRatio": 1.0,
                "rotationConfidence": 1.0,
                "requiresOcrRegions": []
            }
        }],
        "assets": [],
        "extraction": {
            "engine": "product-chain-fixture",
            "engineVersion": "1.0.0",
            "extractedAt": "2026-01-01T00:00:00Z",
            "warnings": []
        }
    })
}

/// P4-T01: the auto import path must materialize the same physical DocumentIRV2 evidence for a
/// DOCX main source as the manual parse command. Before this, `run_auto_pipeline_core` filtered the
/// physical writer to `file_type == "pdf"`, so an auto-imported DOCX reached authoring and quality
/// evaluation with no physical layer at all.
#[test]
fn product_chain_docx_import_materializes_the_physical_document_ir_v2() {
    let root = temp_root("docx-physical");
    ensure_app_dirs(&root).unwrap();
    let mut job = chain_job("Product chain DOCX physical ingest");
    attach_source(
        &root,
        &mut job,
        "fixtures/parser/complex-reading.docx",
        "MainQuestion",
    );
    save_job(&root, &job).unwrap();

    let report = run_auto_pipeline_core(
        &root,
        &job.job_id,
        Some(AutoPipelineInput {
            profile_id: None,
            confidence_threshold: Some(0.85),
            parse_mode: None,
            execution_mode: Some("localOnly".to_string()),
            target: Some("editableDraft".to_string()),
            allow_overwrite: Some(true),
        }),
    )
    .expect("local pipeline must complete for a DOCX main source");
    assert!(
        report.get("status").and_then(Value::as_str).is_some(),
        "pipeline report must carry a job status: {report}"
    );

    let dir = job_dir(&root, &job.job_id);
    assert!(
        dir.join("authoring-ir.json").is_file(),
        "DOCX import must produce the V1 editable draft"
    );
    assert_shadow(&dir, DOCUMENT_V2_SHADOW_FILE, "docx stage 1 physical shadow");
    let physical: Value =
        serde_json::from_slice(&fs::read(dir.join(DOCUMENT_V2_SHADOW_FILE)).unwrap()).unwrap();
    assert_eq!(
        physical.get("schemaVersion").and_then(Value::as_str),
        Some("DocumentIRV2"),
        "DOCX physical ingest must still produce a schema-gated DocumentIRV2"
    );
    assert_eq!(
        physical.get("jobId").and_then(Value::as_str),
        Some(job.job_id.as_str()),
        "the physical document must be bound to the originating job"
    );
    // A DOCX physical layer is OOXML-structural (pages/regions from the document model) unless
    // EPIC8_DOCX_RENDER_ASSIST turns on rendered geometry. Either way it must carry real pages,
    // not an empty shell.
    assert!(
        physical
            .get("pages")
            .and_then(Value::as_array)
            .is_some_and(|pages| !pages.is_empty()),
        "DOCX physical ingest must produce at least one page: {physical}"
    );
    // P4-T02: the layout graph is format-agnostic, so the DOCX branch must produce it too.
    assert_question_layout_graph(&dir, &job, &physical);

    // The point of the fix: without a physical layer the authoring V2 shadow is never produced, so a
    // DOCX job can never open an editable session and can never become ready/publishable. Assert the
    // session end to end, not just the physical artifact.
    assert_shadow(&dir, AUTHORING_V2_SHADOW_FILE, "docx stage 1 authoring shadow");
    let session = get_authoring_v2_core(&root, &job.job_id)
        .expect("a DOCX import must be able to open the authoring V2 session");
    assert_eq!(
        session.get("schemaVersion").and_then(Value::as_str),
        Some("AuthoringEditorSessionV1")
    );
    let authoring = session
        .get("authoring")
        .cloned()
        .expect("session must carry the authoring document");
    first_text_node(&authoring).expect("DOCX draft must contain at least one text node");
}

#[test]
fn product_chain_pdf_import_reaches_editable_session_and_persists_one_character_edit() {
    let root = temp_root("edit");
    ensure_app_dirs(&root).unwrap();
    let mut job = chain_job("Product chain PDF import");
    // A PDF main source exercises the PDF branch of the unified physical ingest. The DOCX branch is
    // covered by `product_chain_docx_import_materializes_the_physical_document_ir_v2`.
    attach_source(
        &root,
        &mut job,
        "fixtures/parser/complex-reading.pdf",
        "MainQuestion",
    );
    save_job(&root, &job).unwrap();

    // ---- Stage 1: real local pipeline ----
    let report = run_auto_pipeline_core(
        &root,
        &job.job_id,
        Some(AutoPipelineInput {
            profile_id: None,
            confidence_threshold: Some(0.85),
            parse_mode: None,
            execution_mode: Some("localOnly".to_string()),
            target: Some("editableDraft".to_string()),
            allow_overwrite: Some(true),
        }),
    )
    .expect("local pipeline must complete for a born-digital PDF");
    assert!(
        report.get("status").and_then(Value::as_str).is_some(),
        "pipeline report must carry a job status: {report}"
    );

    let dir = job_dir(&root, &job.job_id);
    assert!(
        dir.join("authoring-ir.json").is_file(),
        "V1 editable draft must exist"
    );
    assert_shadow(&dir, DOCUMENT_V2_SHADOW_FILE, "stage 1 physical shadow");
    assert_shadow(&dir, AUTHORING_V2_SHADOW_FILE, "stage 1 authoring shadow");

    // P4-T02: the PDF branch of the unified physical ingest must also yield the geometric
    // question layout graph, derived from those physical facts.
    let physical: Value =
        serde_json::from_slice(&fs::read(dir.join(DOCUMENT_V2_SHADOW_FILE)).unwrap()).unwrap();
    assert_question_layout_graph(&dir, &job, &physical);

    // The pipeline must have gone through the real SQLite dual write.
    assert!(
        root.join("authoring_hub.db").is_file(),
        "the pipeline must have created the real SQLite database"
    );
    let saved = load_job(&root, &job.job_id).unwrap();
    assert_eq!(saved.job_id, job.job_id);

    // ---- Stage 2: the workspace opens the V2 session ----
    // `get_authoring_v2_core` had zero test callers before this test.
    let session = get_authoring_v2_core(&root, &job.job_id)
        .expect("workspace must be able to open the authoring V2 session");
    assert_eq!(
        session.get("schemaVersion").and_then(Value::as_str),
        Some("AuthoringEditorSessionV1")
    );
    let base_revision = session
        .get("revision")
        .and_then(Value::as_u64)
        .expect("session must expose a revision");
    let authoring = session
        .get("authoring")
        .cloned()
        .expect("session must carry the authoring document");

    // ---- Stage 3: change exactly one character, the way the workspace does ----
    let (node_id, original_text) =
        first_text_node(&authoring).expect("recognised draft must contain at least one text node");
    let char_count = original_text.chars().count();
    assert!(
        char_count > 1,
        "text node must be long enough to drop a character"
    );
    let expected_text = original_text
        .chars()
        .take(char_count - 1)
        .collect::<String>();

    let applied = apply_authoring_v2_patches_core(
        &root,
        json!({
            "jobId": job.job_id,
            "baseRevision": base_revision,
            // `from: 0, to: <code point count>` is whole-node replacement, which is what the
            // frontend EditorCommandV1 `set_text` compiles to. Rust counts Unicode scalars here,
            // so the frontend must use Array.from(text).length, not JS string .length.
            "patches": [{
                "op": "replaceText",
                "nodeId": node_id,
                "from": 0,
                "to": char_count,
                "text": expected_text
            }]
        }),
    )
    .expect("one-character edit must save");
    let next_revision = applied
        .get("revision")
        .and_then(Value::as_u64)
        .expect("apply must return the saved revision");
    assert!(
        next_revision > base_revision,
        "saving must advance the revision: {base_revision} -> {next_revision}"
    );

    // ---- Stage 4: reopening must show the edit (the "refresh and it is still there" gate) ----
    let reopened = get_authoring_v2_core(&root, &job.job_id).expect("session must reload");
    let reloaded_text = node_text(
        reopened
            .get("authoring")
            .expect("reloaded session carries authoring"),
        &node_id,
    )
    .expect("edited node must still exist after reload");
    assert_eq!(
        reloaded_text, expected_text,
        "the edit must survive a reload"
    );

    // A stale base revision must be rejected rather than silently overwriting.
    let conflict = apply_authoring_v2_patches_core(
        &root,
        json!({
            "jobId": job.job_id,
            "baseRevision": base_revision,
            "patches": [{
                "op": "replaceText",
                "nodeId": node_id,
                "from": 0,
                "to": expected_text.chars().count(),
                "text": "stale write"
            }]
        }),
    );
    assert!(
        conflict.is_err(),
        "a stale baseRevision must be rejected, got: {conflict:?}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn product_chain_ready_authoring_exports_and_publishes_to_nas() {
    let root = temp_root("publish");
    ensure_app_dirs(&root).unwrap();
    let job = chain_job("Product chain publish");
    save_job(&root, &job).unwrap();
    let dir = job_dir(&root, &job.job_id);
    ensure_job_dirs(&dir).unwrap();

    // Seed the proven-ready authoring document, retargeted at this job.
    let mut authoring: Value = serde_json::from_slice(
        &fs::read(workspace_path(READY_AUTHORING_FIXTURE))
            .expect("ready authoring fixture must exist"),
    )
    .expect("ready authoring fixture must be valid JSON");
    authoring
        .as_object_mut()
        .expect("authoring fixture must be an object")
        .insert("jobId".to_string(), json!(job.job_id));
    write_json(&dir.join(AUTHORING_V2_SHADOW_FILE), &authoring).unwrap();
    write_json(
        &dir.join(DOCUMENT_V2_SHADOW_FILE),
        &physical_shadow_for(&authoring),
    )
    .unwrap();

    // The session must load. Its stored `quality.state` is deliberately `review_required` in the
    // fixture ("Ready must be recomputed by the PR-07 test with a fresh physical shadow"), and
    // export recomputes quality from the physical shadow, so the stored value is not the gate --
    // a successful export is. Surface the stored state for diagnosis only.
    let session = get_authoring_v2_core(&root, &job.job_id)
        .expect("seeded ready authoring must open as a session");
    eprintln!(
        "seeded session stored quality.state = {:?} (export recomputes it)",
        session.pointer("/authoring/quality/state")
    );

    // Recompute exactly the way export does, and fail here with the real reasons rather than
    // letting export report an opaque `quality_state=review_required`.
    let shadow: Value =
        serde_json::from_slice(&fs::read(dir.join(DOCUMENT_V2_SHADOW_FILE)).unwrap()).unwrap();
    let recomputed = crate::ielts_grammar::evaluate_quality(&authoring, Some(&shadow));
    assert_eq!(
        recomputed.get("state").and_then(Value::as_str),
        Some("ready"),
        "seeded fixture + physical shadow must recompute to ready. documentScore={:?} sourceCoverage={:?} hardFailures={:?} issues={}",
        recomputed.get("documentScore"),
        recomputed.get("sourceCoverage"),
        recomputed.get("hardFailures"),
        serde_json::to_string(recomputed.get("issues").unwrap_or(&Value::Null)).unwrap_or_default()
    );

    // ---- Stage 1: real export. This is a mandatory link: the publisher re-derives and verifies
    // the export binding (runtime file name, sibling manifest hash, publish proof), so a
    // hand-forged runtime file cannot be published. ----
    let export_dir = root.join("exports").join("product-chain");
    let exported =
        export_authoring_v2_core(&root, json!({"jobId": job.job_id, "exportDir": export_dir}))
            .expect("ready authoring must export");
    let runtime_path = exported
        .pointer("/receipt/runtimePath")
        .and_then(Value::as_str)
        .expect("export receipt must carry the runtime path")
        .to_string();
    let output_dir = exported
        .pointer("/receipt/outputDir")
        .and_then(Value::as_str)
        .expect("export receipt must carry the output dir")
        .to_string();
    let exam_id = exported
        .get("examId")
        .and_then(Value::as_str)
        .expect("export receipt must carry the exam id")
        .to_string();
    assert!(
        Path::new(&runtime_path).is_file(),
        "exported runtime must exist on disk"
    );

    // ---- Stage 2: real NAS publish ----
    // `libraryRoot` is normalised by stripping a trailing `publish/` child, so assert on the
    // parent the publisher actually writes to rather than the path passed in.
    let nas_parent = root.join("nas");
    let published = publish_nas_package_v2_core(
        &root,
        json!({
            "libraryRoot": nas_parent.join("publish"),
            "sourcePath": runtime_path,
            "assetRoot": output_dir,
            "examId": exam_id,
            "minimumRuntimeVersion": "0.2.0"
        }),
    )
    .expect("a ready, exported item must publish");

    assert_eq!(
        published.get("status").and_then(Value::as_str),
        Some("committed")
    );
    assert_eq!(
        published.pointer("/probe/passed").and_then(Value::as_bool),
        Some(true),
        "publish probe must pass: {published}"
    );
    let manifest_path = published
        .get("manifestPath")
        .and_then(Value::as_str)
        .expect("publish result must name the manifest");
    assert!(
        Path::new(manifest_path).is_file(),
        "manifest must exist after commit"
    );
    assert!(
        nas_parent.join("manifest.js").is_file(),
        "manifest must land in the normalised NAS parent, not the publish child"
    );
    assert!(
        !nas_parent.join("publish").join("manifest.js").exists(),
        "publish child must not receive a second manifest"
    );

    let _ = fs::remove_dir_all(root);
}

/// M0-T5（跨仓 NAS 契约入口）：把一份真实 publisher 产物落到仓库固定目录，
/// 供 `scripts/e2e/nas-student-contract.mjs` 按学生端 manifest 规则校验。
/// 默认忽略（会写仓库 artifacts 目录、不清理），需要时手动运行：
///   cargo test --manifest-path src-tauri/Cargo.toml dump_published_package_for_nas_contract -- --ignored --nocapture
#[test]
#[ignore = "writes artifacts/nas-contract-fixture for the cross-repo contract check; run explicitly"]
fn dump_published_package_for_nas_contract() {
    let root = temp_root("nas-dump");
    ensure_app_dirs(&root).unwrap();
    let job = chain_job("NAS contract dump");
    save_job(&root, &job).unwrap();
    let dir = job_dir(&root, &job.job_id);
    ensure_job_dirs(&dir).unwrap();

    let mut authoring: Value = serde_json::from_slice(
        &fs::read(workspace_path(READY_AUTHORING_FIXTURE))
            .expect("ready authoring fixture must exist"),
    )
    .expect("ready authoring fixture must be valid JSON");
    authoring
        .as_object_mut()
        .expect("authoring fixture must be an object")
        .insert("jobId".to_string(), json!(job.job_id));
    write_json(&dir.join(AUTHORING_V2_SHADOW_FILE), &authoring).unwrap();
    write_json(
        &dir.join(DOCUMENT_V2_SHADOW_FILE),
        &physical_shadow_for(&authoring),
    )
    .unwrap();

    let export_dir = root.join("exports").join("nas-dump");
    let exported = export_authoring_v2_core(
        &root,
        json!({ "jobId": job.job_id, "exportDir": export_dir }),
    )
    .expect("ready authoring must export");
    let runtime_path = exported
        .pointer("/receipt/runtimePath")
        .and_then(Value::as_str)
        .expect("export receipt must carry the runtime path")
        .to_string();
    let output_dir = exported
        .pointer("/receipt/outputDir")
        .and_then(Value::as_str)
        .expect("export receipt must carry the output dir")
        .to_string();
    let exam_id = exported
        .get("examId")
        .and_then(Value::as_str)
        .expect("export receipt must carry the exam id")
        .to_string();

    // 输出目录固定在仓库 artifacts 下，先清空旧 dump 保证幂等。
    let nas_parent = workspace_path("artifacts/nas-contract-fixture");
    let _ = fs::remove_dir_all(&nas_parent);
    fs::create_dir_all(&nas_parent).unwrap();
    let published = publish_nas_package_v2_core(
        &root,
        json!({
            "libraryRoot": nas_parent.join("publish"),
            "sourcePath": runtime_path,
            "assetRoot": output_dir,
            "examId": exam_id,
            "minimumRuntimeVersion": "0.2.0"
        }),
    )
    .expect("a ready, exported item must publish");
    assert_eq!(
        published.get("status").and_then(Value::as_str),
        Some("committed")
    );
    eprintln!(
        "NAS contract fixture written to {} (examId {})",
        nas_parent.display(),
        exam_id
    );
    let _ = fs::remove_dir_all(root);
}

/// M6 跨仓学生端 E2E 输入：把三份真实 publisher 产物（P1/P2/P3）落到
/// artifacts/nas-e2e-fixture，供学生端 Electron 驱动加载、渲染、作答与计分。
/// 每份都带真实图片资产与 diagram hotspot，且 hotspotId 与 answerKey 的 label 一致。
/// 默认忽略（写仓库 artifacts 目录、不清理），需要时手动运行：
///   cargo test --manifest-path src-tauri/Cargo.toml dump_published_v2_visual_package_for_student_e2e -- --ignored --nocapture
#[test]
#[ignore = "writes artifacts/nas-e2e-fixture for the student Electron E2E; run explicitly"]
fn dump_published_v2_visual_package_for_student_e2e() {
    let nas_parent = workspace_path("artifacts/nas-e2e-fixture");
    let _ = fs::remove_dir_all(&nas_parent);
    fs::create_dir_all(&nas_parent).unwrap();
    // 学生端套卷固定要求 P1/P2/P3 各一份；三份都由真实 export + NAS publish 产出，
    // 学生端因此消费的是 publisher 原样输出，而不是测试手写的包。
    for (exam_id, category, title) in [
        ("v2-p1", "P1", "V2 Reading Part 1"),
        ("v2-p2", "P2", "V2 Reading Part 2"),
        ("v2-p3", "P3", "V2 Reading Part 3"),
    ] {
        dump_v2_visual_package_for_part(&nas_parent, exam_id, category, title);
    }
    let manifest =
        fs::read_to_string(nas_parent.join("manifest.js")).expect("published manifest must exist");
    for exam_id in ["v2-p1", "v2-p2", "v2-p3"] {
        assert!(manifest.contains(exam_id), "manifest must carry {exam_id}");
    }
    eprintln!("NAS e2e fixture written to {}", nas_parent.display());
}

fn dump_v2_visual_package_for_part(nas_parent: &Path, exam_id: &str, category: &str, title: &str) {
    let root = temp_root(&format!("nas-e2e-dump-{exam_id}"));
    ensure_app_dirs(&root).unwrap();
    let job = chain_job("NAS e2e visual dump");
    save_job(&root, &job).unwrap();
    let dir = job_dir(&root, &job.job_id);
    ensure_job_dirs(&dir).unwrap();

    let mut authoring: Value = serde_json::from_slice(
        &fs::read(workspace_path(READY_AUTHORING_FIXTURE))
            .expect("ready authoring fixture must exist"),
    )
    .expect("ready authoring fixture must be valid JSON");
    // 每份都是独立考试身份：examId 决定包目录、脚本名与 manifest key，
    // category 决定学生端套卷把它放在 P1/P2/P3 的哪一段。
    {
        let exam = authoring
            .get_mut("exam")
            .and_then(Value::as_object_mut)
            .expect("ready fixture must carry exam identity");
        exam.insert("examId".to_string(), json!(exam_id));
        exam.insert("category".to_string(), json!(category));
        exam.insert("title".to_string(), json!(title));
    }
    let existing_anchor = authoring
        .pointer("/passage/content/0/sourceAnchors/0")
        .cloned()
        .unwrap_or_else(|| json!({
            "sourceFileId": "source-pdf-1",
            "pageIndex": 0,
            "nodeIds": ["region-question-surface"],
            "extractionMode": "pdf_native",
            "sourceHash": "a".repeat(64)
        }));
    // 1x1 红色 PNG（CRC 已验证）：真实学生端 <img> 可解码的最小视觉资产。
    let png = build_e2e_png();
    let png_sha = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&png);
        format!("{:x}", hasher.finalize())
    };
    {
        let object = authoring.as_object_mut().unwrap();
        object.insert("jobId".to_string(), json!(job.job_id));
        object["assets"] = json!([{
            "assetId": "img-map",
            "kind": "raster_image",
            "mime": "image/png",
            "relativePath": "img-map.png",
            "sha256": png_sha,
            "byteLength": png.len() as u64,
            "widthPx": 1,
            "heightPx": 1,
            "extractionMode": "embedded",
            "altText": "Task map with two labelled regions"
        }]);
        let anchor = existing_anchor;
        let passage = object.get_mut("passage").and_then(Value::as_object_mut).unwrap();
        let content = passage.get_mut("content").and_then(Value::as_array_mut).unwrap();
        content.push(json!({
            "id": "passage-figure-map",
            "type": "figure",
            "provenanceStatus": "source",
            "sourceAnchors": [anchor],
            "assetId": "img-map",
            "display": {"widthPercent": 60, "align": "center"},
            "caption": [{
                "id": "figure-map-caption",
                "type": "text",
                "provenanceStatus": "source",
                "sourceAnchors": [],
                "text": "Map referenced by Questions 14 and 15."
            }]
        }));
        let task = object
            .get_mut("taskGroups")
            .and_then(Value::as_array_mut)
            .unwrap()
            .get_mut(0)
            .and_then(Value::as_object_mut)
            .unwrap();
        task.insert(
            "stimulus".to_string(),
            json!([{
                "id": "task-map-diagram",
                "type": "diagram",
                "provenanceStatus": "source",
                "sourceAnchors": [],
                "assetId": "img-map",
                "display": {"widthPercent": 80, "align": "center"},
                "hotspots": [
                    {"hotspotId": "B", "slotId": "q14", "normalizedRect": [0.12, 0.18, 0.3, 0.22]},
                    {"hotspotId": "D", "slotId": "q15", "normalizedRect": [0.55, 0.55, 0.3, 0.22]}
                ]
            }]),
        );
    }
    write_json(&dir.join(AUTHORING_V2_SHADOW_FILE), &authoring).unwrap();
    let visual_descriptor = json!({
        "assetId": "img-map",
        "kind": "raster_image",
        "mime": "image/png",
        "relativePath": "img-map.png",
        "sha256": png_sha,
        "byteLength": png.len() as u64,
        "widthPx": 1,
        "heightPx": 1,
        "extractionMode": "embedded",
        "altText": "Task map with two labelled regions"
    });
    let mut shadow = physical_shadow_for(&authoring);
    shadow["assets"] = json!([visual_descriptor]);
    write_json(&dir.join(DOCUMENT_V2_SHADOW_FILE), &shadow).unwrap();

    {
        let shadow: Value = serde_json::from_slice(&fs::read(dir.join(DOCUMENT_V2_SHADOW_FILE)).unwrap()).unwrap();
        let recomputed = crate::ielts_grammar::evaluate_quality(&authoring, Some(&shadow));
        eprintln!(
            "visual dump quality: state={:?} hardFailures={:?} issues={}",
            recomputed.get("state"),
            recomputed.get("hardFailures"),
            serde_json::to_string(recomputed.get("issues").unwrap_or(&Value::Null)).unwrap_or_default()
        );
    }

    // export 从 job 目录物化资产文件；先把 PNG 放到 job 目录再导出。
    fs::write(dir.join("img-map.png"), &png).expect("png must be written into the job dir");

    let export_dir = root.join("exports").join("nas-e2e-dump");
    let exported = export_authoring_v2_core(
        &root,
        json!({ "jobId": job.job_id, "exportDir": export_dir }),
    )
    .expect("visual authoring must export");
    let runtime_path = exported
        .pointer("/receipt/runtimePath")
        .and_then(Value::as_str)
        .expect("export receipt must carry the runtime path")
        .to_string();
    let output_dir = exported
        .pointer("/receipt/outputDir")
        .and_then(Value::as_str)
        .expect("export receipt must carry the output dir")
        .to_string();
    let exported_exam_id = exported
        .get("examId")
        .and_then(Value::as_str)
        .expect("export receipt must carry the exam id")
        .to_string();
    assert_eq!(
        exported_exam_id, exam_id,
        "the compiler must keep the exam identity we authored"
    );

    // 发布者从 assetRoot（export 产物目录）解析资产；export 已把 job 目录中的
    // PNG 物化到该目录，无需再手写文件。

    let published = publish_nas_package_v2_core(
        &root,
        json!({
            "libraryRoot": nas_parent.join("publish"),
            "sourcePath": runtime_path,
            "assetRoot": output_dir,
            "examId": exam_id,
            "minimumRuntimeVersion": "0.2.0"
        }),
    )
    .expect("a visual ready item must publish");
    assert_eq!(
        published.get("status").and_then(Value::as_str),
        Some("committed")
    );
    assert_eq!(
        published.pointer("/probe/passed").and_then(Value::as_bool),
        Some(true),
        "publish probe must pass for the visual package: {published}"
    );
    eprintln!(
        "NAS e2e part published: examId={exam_id} category={category} root={}",
        nas_parent.display()
    );
    let _ = fs::remove_dir_all(root);
}

/// 与 dump 测试共用的 1x1 红 PNG（CRC 已验证），足以驱动 <img> 真实解码。
fn build_e2e_png() -> Vec<u8> {
    vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8,
        6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 218, 99, 252, 207, 192,
        80, 15, 0, 4, 133, 1, 128, 132, 169, 140, 33, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
        130,
    ]
}

/// Editing must not make a publishable item unpublishable.
///
/// `mark_user_audit` used to hardcode `audit.humanVerified = false` on every save, while the
/// export gate requires it to be true. Because V2 has no path that ever sets it back to true
/// (V1 derives it in `refresh_authoring_review_state`; V2 had no equivalent), the flag was
/// monotonically false: the first edit permanently blocked publishing. That inverts the intent --
/// it does not protect students, it forces authors to publish an unedited draft or not at all.
/// The real content safety is the rest of the gate (zero unresolved blocker issues, no unresolved
/// answers, quality ready, compiler pass, asset closure), all recomputed from the current DS.
#[test]
fn product_chain_editing_a_publishable_item_keeps_it_publishable() {
    let root = temp_root("edit-then-publish");
    ensure_app_dirs(&root).unwrap();
    let job = chain_job("Product chain edit then publish");
    save_job(&root, &job).unwrap();
    let dir = job_dir(&root, &job.job_id);
    ensure_job_dirs(&dir).unwrap();

    let mut authoring: Value = serde_json::from_slice(
        &fs::read(workspace_path(READY_AUTHORING_FIXTURE))
            .expect("ready authoring fixture must exist"),
    )
    .unwrap();
    authoring
        .as_object_mut()
        .unwrap()
        .insert("jobId".to_string(), json!(job.job_id));
    write_json(&dir.join(AUTHORING_V2_SHADOW_FILE), &authoring).unwrap();
    write_json(
        &dir.join(DOCUMENT_V2_SHADOW_FILE),
        &physical_shadow_for(&authoring),
    )
    .unwrap();

    // Baseline: the untouched item exports.
    export_authoring_v2_core(
        &root,
        json!({"jobId": job.job_id, "exportDir": root.join("exports").join("before")}),
    )
    .expect("untouched ready item must export");

    // One human edit through the real editor command.
    let session = get_authoring_v2_core(&root, &job.job_id).unwrap();
    let base_revision = session.get("revision").and_then(Value::as_u64).unwrap();
    let (node_id, original_text) =
        first_text_node(session.get("authoring").expect("session carries authoring"))
            .expect("fixture must contain a text node");
    let char_count = original_text.chars().count();
    let edited = original_text
        .chars()
        .take(char_count - 1)
        .collect::<String>();
    apply_authoring_v2_patches_core(
        &root,
        json!({
            "jobId": job.job_id,
            "baseRevision": base_revision,
            "patches": [{
                "op": "replaceText",
                "nodeId": node_id,
                "from": 0,
                "to": char_count,
                "text": edited
            }]
        }),
    )
    .expect("editing a ready item must save");

    let after_edit = get_authoring_v2_core(&root, &job.job_id).unwrap();
    assert_eq!(
        after_edit.pointer("/authoring/audit/humanVerified"),
        Some(&json!(true)),
        "a save must not downgrade an already-verified document"
    );

    // The whole point: the edited item must still be publishable.
    let exported = export_authoring_v2_core(
        &root,
        json!({"jobId": job.job_id, "exportDir": root.join("exports").join("after")}),
    )
    .expect("an edited ready item must still export");
    let runtime_path = exported
        .pointer("/receipt/runtimePath")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let runtime = fs::read_to_string(&runtime_path).unwrap();
    assert!(
        runtime.contains(&edited),
        "the compiled runtime must carry the edited text"
    );

    let _ = fs::remove_dir_all(root);
}

/// Readiness must key on UNRESOLVED BLOCKING issues only, matching the export gate.
///
/// Locks in the contract fixed alongside `mark_user_audit`: the state computation used to treat
/// `!issues.is_empty()` as review_required, so one `info` note made a perfect paper unpublishable
/// and resolving an issue changed nothing even though every save carefully preserved resolutions.
#[test]
fn readiness_keys_on_unresolved_blocking_issues_only() {
    let authoring: Value = serde_json::from_slice(
        &fs::read(workspace_path(READY_AUTHORING_FIXTURE))
            .expect("ready authoring fixture must exist"),
    )
    .unwrap();
    let shadow = physical_shadow_for(&authoring);

    let state_with = |issue: Option<Value>| -> String {
        let mut document = authoring.clone();
        if let Some(issue) = issue {
            document
                .get_mut("quality")
                .and_then(Value::as_object_mut)
                .unwrap()
                .insert("issues".to_string(), json!([issue]));
        } else {
            document
                .get_mut("quality")
                .and_then(Value::as_object_mut)
                .unwrap()
                .insert("issues".to_string(), json!([]));
        }
        crate::ielts_grammar::evaluate_quality(&document, Some(&shadow))
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("<missing>")
            .to_string()
    };

    // Baseline: no issues at all.
    assert_eq!(state_with(None), "ready");

    // `evaluate_quality` recomputes the issue list from the document, so a hand-injected issue on
    // the stored quality block cannot itself change the outcome -- that is exactly the point: the
    // state must come from the current document, not from a historical issue list.
    assert_eq!(
        state_with(Some(json!({
            "issueId": "synthetic-info",
            "code": "PHYSICAL_SHADOW_MISSING",
            "severity": "info",
            "message": "informational only",
            "targetType": "document",
            "targetId": "document",
            "sourceAnchors": [],
            "suggestedActions": []
        }))),
        "ready",
        "an informational note must not make a document unpublishable"
    );
    assert_eq!(
        state_with(Some(json!({
            "issueId": "synthetic-warning",
            "code": "PHYSICAL_SHADOW_MISSING",
            "severity": "warning",
            "message": "warning only",
            "targetType": "document",
            "targetId": "document",
            "sourceAnchors": [],
            "suggestedActions": []
        }))),
        "ready",
        "a warning must not make a document unpublishable"
    );
}

/// M1（原 P2-T03/T04）验收：Workspace API 的 DB 链必须——
/// 1. 按需迁移填充权威稿（shadow 候选）；
/// 2. 在事务里应用编辑（单字符 + 标题）并推进版本；
/// 3. 新连接读到持久化结果（重启一致性）；
/// 4. **不新增 revision 文件树**（P2-T04）；
/// 5. 把 canonical DS 同步为 shadow 缓存（现有 export/publish 链可读）。
#[test]
fn library_v2_workspace_api_persists_edits_without_new_revisions() {
    let root = temp_root("library-v2");
    ensure_app_dirs(&root).unwrap();
    let job = chain_job("Library v2 edit");
    save_job(&root, &job).unwrap();
    let dir = job_dir(&root, &job.job_id);
    ensure_job_dirs(&dir).unwrap();

    let mut authoring: Value = serde_json::from_slice(
        &fs::read(workspace_path(READY_AUTHORING_FIXTURE))
            .expect("ready authoring fixture must exist"),
    )
    .expect("ready authoring fixture must be valid JSON");
    authoring
        .as_object_mut()
        .expect("authoring fixture must be an object")
        .insert("jobId".to_string(), json!(job.job_id));
    write_json(&dir.join(AUTHORING_V2_SHADOW_FILE), &authoring).unwrap();
    write_json(
        &dir.join(DOCUMENT_V2_SHADOW_FILE),
        &physical_shadow_for(&authoring),
    )
    .unwrap();

    // 1. 首次访问：按需迁移填充权威稿。
    let workspace = crate::library::commands::get_workspace_item_core(&root, &job.job_id)
        .expect("workspace item must load after on-demand migration");
    assert_eq!(workspace.pointer("/item/editVersion"), Some(&json!(1)));
    assert_eq!(
        workspace.pointer("/item/hasCanonicalDs"),
        Some(&json!(true))
    );

    // 2. 找一个真实 text 节点，做单字符编辑 + 标题保存。
    let node_id = find_first_text_node_id(workspace.pointer("/ds").expect("ds must be present"))
        .expect("ready fixture must contain a text node");
    let current_text = find_first_text_node_text(workspace.pointer("/ds").unwrap(), &node_id)
        .expect("text node must carry text");
    let edited = format!("{current_text}-");
    let revisions_before = count_revision_files(&dir);

    let result = crate::library::commands::apply_editor_commands_core(
        &root,
        crate::library::repository::ApplyEditorCommandsInput {
            item_id: job.job_id.clone(),
            base_version: 1,
            request_id: Some(format!("test-{}", job.job_id)),
            commands: vec![json!({
                "op": "replaceText",
                "nodeId": node_id,
                "from": current_text.chars().count() - 1,
                "to": current_text.chars().count(),
                "text": "-"
            })],
            title: Some("Library v2 title".to_string()),
        },
    )
    .expect("DB edit transaction must succeed");
    assert_eq!(result.pointer("/editVersion"), Some(&json!(2)));
    assert_eq!(result.pointer("/appliedCount"), Some(&json!(1)));

    // 3. 新连接（模拟重启）读到持久化结果。
    let conn = crate::library::repository::open_library_connection(&root).unwrap();
    let (ds, version) = crate::library::repository::get_canonical_ds(&conn, &job.job_id)
        .unwrap()
        .expect("canonical ds must be persisted");
    assert_eq!(version, 2);
    assert_eq!(
        ds.pointer("/exam/title").and_then(Value::as_str),
        Some("Library v2 title")
    );
    assert!(
        find_first_text_node_text(&ds, &node_id)
            .unwrap()
            .ends_with('-'),
        "single-character edit must persist"
    );
    let item = crate::library::repository::get_item(&conn, &job.job_id)
        .unwrap()
        .expect("item row must exist");
    assert_eq!(item.title, "Library v2 title");

    // 4. 不新增 revision 文件（新编辑不进文件树）。
    let revisions_after = count_revision_files(&dir);
    assert_eq!(
        revisions_after, revisions_before,
        "DB 编辑不得追加 revision 文件树"
    );

    // 5. 权威稿唯一（F04 修复）：DB 编辑不再回写 shadow 派生缓存。
    //    提交后再读一次权威稿、另行刷新质量并整份覆盖 DB 的派生写入曾是丢稿竞态来源，
    //    现已并入同一事务；文件缓存不再参与保存结果判定。
    let cached: Value =
        serde_json::from_slice(&fs::read(dir.join(AUTHORING_V2_SHADOW_FILE)).unwrap()).unwrap();
    assert_ne!(
        cached.pointer("/exam/title").and_then(Value::as_str),
        Some("Library v2 title"),
        "DB 编辑不得回写 shadow 缓存；否则过期的派生写入会重新出现并可覆盖较新的保存"
    );
    let reloaded = crate::library::commands::get_workspace_item_core(&root, &job.job_id)
        .expect("product workspace API must keep serving the DB draft");
    assert_eq!(
        reloaded.pointer("/ds/exam/title").and_then(Value::as_str),
        Some("Library v2 title"),
        "产品工作区 API 必须从权威稿读取最新内容"
    );

    // 6. DB 直通发布：只给 editVersion，后端自行从 canonical DS 解析权威稿
    //    （typed preflight 无历史痕迹门禁 + NAS 原子提交）。
    let exported = crate::authoring_v2_commands::export_authoring_v2_core(
        &root,
        json!({
            "jobId": job.job_id,
            "exportDir": root.join("exports").join("library-v2"),
            "editVersion": 2
        }),
    )
    .expect("DB direct export must pass the typed preflight");
    let runtime_path = exported
        .pointer("/receipt/runtimePath")
        .and_then(Value::as_str)
        .expect("export receipt must carry the runtime path")
        .to_string();
    let output_dir = exported
        .pointer("/receipt/outputDir")
        .and_then(Value::as_str)
        .expect("export receipt must carry the output dir")
        .to_string();
    let exam_id = exported
        .get("examId")
        .and_then(Value::as_str)
        .expect("export receipt must carry the exam id")
        .to_string();
    assert_eq!(
        exported.get("revision"),
        Some(&json!(0)),
        "DB 直通的绑定标记 revision=0"
    );
    let package_manifest: Value =
        serde_json::from_slice(&fs::read(Path::new(&output_dir).join("manifest-v2.json")).unwrap())
            .unwrap();
    assert_eq!(package_manifest.get("editVersion"), Some(&json!(2)));
    assert_eq!(
        package_manifest
            .get("authoringSource")
            .and_then(Value::as_str),
        Some("canonical_ds")
    );

    let nas_parent = root.join("nas");
    let published = publish_nas_package_v2_core(
        &root,
        json!({
            "libraryRoot": nas_parent.join("publish"),
            "sourcePath": runtime_path,
            "assetRoot": output_dir,
            "examId": exam_id,
            "minimumRuntimeVersion": "0.2.0"
        }),
    )
    .expect("a DB-direct export with a passing typed preflight must publish");
    assert_eq!(
        published.get("status").and_then(Value::as_str),
        Some("committed")
    );
    assert_eq!(
        published.pointer("/probe/passed").and_then(Value::as_bool),
        Some(true),
        "publish probe must pass: {published}"
    );
    let _ = fs::remove_dir_all(root);
}

/// 批量发布必须整批原子（F10 修复），且绑定数据库冻结快照（F12 修复）：
///
/// 1. 中途失败时，任何一题都不得出现在 NAS 清单里——不允许「发了一半」；
/// 2. 全部通过时，一次清单替换让整批同时可见，条目只指向不可变的 releases 文件；
/// 3. 发布读取的是 canonical DS 冻结快照，不依赖可变的 shadow 缓存文件。
#[test]
fn publish_items_is_all_or_nothing_across_a_batch() {
    let root = temp_root("publish-items-batch");
    ensure_app_dirs(&root).unwrap();

    // 两道题：各自写入 ready 授权稿 + 匹配的物理 shadow，再按需迁移成权威稿。
    let mut item_ids = Vec::new();
    for index in 0..2 {
        let job = chain_job(&format!("Batch publish {index}"));
        save_job(&root, &job).unwrap();
        let dir = job_dir(&root, &job.job_id);
        ensure_job_dirs(&dir).unwrap();
        let mut authoring: Value = serde_json::from_slice(
            &fs::read(workspace_path(READY_AUTHORING_FIXTURE))
                .expect("ready authoring fixture must exist"),
        )
        .expect("ready authoring fixture must be valid JSON");
        let object = authoring
            .as_object_mut()
            .expect("authoring fixture must be an object");
        object.insert("jobId".to_string(), json!(job.job_id));
        object
            .get_mut("exam")
            .and_then(Value::as_object_mut)
            .expect("fixture must carry an exam object")
            .insert("examId".to_string(), json!(format!("batch-{index}")));
        write_json(&dir.join(AUTHORING_V2_SHADOW_FILE), &authoring).unwrap();
        write_json(
            &dir.join(DOCUMENT_V2_SHADOW_FILE),
            &physical_shadow_for(&authoring),
        )
        .unwrap();
        crate::library::commands::get_workspace_item_core(&root, &job.job_id)
            .expect("on-demand migration must seed the canonical draft");
        item_ids.push(job.job_id);
    }

    let destination = root.join("nas").join("publish");
    let reading_root = crate::export_nas_library::nas_reading_exams_dir(
        &crate::export_nas_library::normalize_nas_library_root(&destination),
    );
    let manifest_path = reading_root.join("manifest.js");
    let batch_input = |fault: Option<&str>| crate::nas_package_v2::PublishItemsInput {
        item_ids: item_ids.clone(),
        destination: destination.to_string_lossy().into_owned(),
        fault: fault.map(str::to_string),
    };

    // 1. 第一题之后中断：整批回滚，清单与 releases 都不得留下痕迹。
    let interrupted =
        crate::nas_package_v2::publish_items_core(&root, batch_input(Some("after_item_1")));
    assert!(
        interrupted.is_err(),
        "注入的批量中断必须向上报错，而不是静默返回部分成功：{interrupted:?}"
    );
    assert!(
        !manifest_path.exists(),
        "批量中断后不得留下 NAS 清单：{}",
        manifest_path.display()
    );
    assert!(
        !reading_root.join("releases").exists(),
        "批量中断后不得留下已提交的 release 目录"
    );

    // 1b. 资源已移入根级但清单尚未替换时中断：根级资源必须回滚，旧清单保持有效。
    let interrupted_before_manifest =
        crate::nas_package_v2::publish_items_core(&root, batch_input(Some("before_manifest")));
    assert!(
        interrupted_before_manifest.is_err(),
        "注入的批量中断必须向上报错，而不是静默返回部分成功：{interrupted_before_manifest:?}"
    );
    for index in 0..2 {
        assert!(
            !reading_root.join("resources").join(format!("batch-{index}")).exists(),
            "清单替换前中断不得留下根级资源目录 batch-{index}"
        );
    }
    assert!(
        fs::read_dir(reading_root.join("releases"))
            .map(|entries| entries.count())
            .unwrap_or(0)
            == 0,
        "清单替换前中断不得留下 release 目录内容"
    );

    // 2. 全部通过：一次原子清单替换让两题同时可见。
    let published = crate::nas_package_v2::publish_items_core(&root, batch_input(None))
        .expect("a batch whose items all pass must publish");
    assert_eq!(
        published
            .get("succeeded")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2),
        "整批通过时两题都应发布：{published}"
    );
    assert_eq!(
        published.get("failed").and_then(Value::as_array).map(Vec::len),
        Some(0)
    );
    let manifest = fs::read_to_string(&manifest_path).expect("batch manifest must exist");
    for index in 0..2 {
        assert!(
            manifest.contains(&format!("\"batch-{index}\"")),
            "清单必须包含 batch-{index}：{manifest}"
        );
    }
    assert!(
        manifest.contains("./releases/"),
        "发布条目必须指向不可变的 release 文件，而不是可被下一次发布覆盖的根目录文件"
    );
    // 学生端 resolver 固定从 reading 根解析 resources/<examId>，
    // 因此批量发布的资源与 asset-manifest 必须落在根级布局，且真实存在。
    for index in 0..2 {
        let resource_dir = reading_root.join("resources").join(format!("batch-{index}"));
        assert!(
            resource_dir.join("asset-manifest.json").is_file(),
            "批量发布的资源必须位于根级 resources/<examId>/asset-manifest.json：{}",
            resource_dir.display()
        );
        assert!(
            manifest.contains(&format!("./resources/batch-{index}/")),
            "清单的资源路径必须指向根级 resources/：{manifest}"
        );
    }

    // 3. 发布后把库里条目标记为已发布，且不推进编辑版本（后续编辑仍是未发布状态）。
    let conn = crate::library::repository::open_library_connection(&root).unwrap();
    for item_id in &item_ids {
        let item = crate::library::repository::get_item(&conn, item_id)
            .unwrap()
            .expect("item row must exist");
        assert_eq!(item.status, "published", "{item_id} 应标记为已发布");
        assert_eq!(item.current_edit_version, 1, "发布不得推进编辑版本");
    }
    let _ = fs::remove_dir_all(root);
}

/// 旧工件读取链不得凌驾于权威稿之上。
///
/// `get_authoring_v2` 面向仍未退休的旧页面。DB 编辑后 shadow 不再回写（F04 修复），
/// 如果读取仍优先 shadow，旧页面就会显示编辑前的内容。
#[test]
fn legacy_authoring_session_reads_the_db_draft_not_the_stale_shadow() {
    let root = temp_root("legacy-session-db-first");
    ensure_app_dirs(&root).unwrap();
    let job = chain_job("Legacy session reads DB");
    save_job(&root, &job).unwrap();
    let dir = job_dir(&root, &job.job_id);
    ensure_job_dirs(&dir).unwrap();

    let mut authoring: Value = serde_json::from_slice(
        &fs::read(workspace_path(READY_AUTHORING_FIXTURE))
            .expect("ready authoring fixture must exist"),
    )
    .expect("ready authoring fixture must be valid JSON");
    authoring
        .as_object_mut()
        .expect("authoring fixture must be an object")
        .insert("jobId".to_string(), json!(job.job_id));
    let shadow_title = authoring
        .pointer("/exam/title")
        .and_then(Value::as_str)
        .expect("fixture must carry a title")
        .to_string();
    write_json(&dir.join(AUTHORING_V2_SHADOW_FILE), &authoring).unwrap();
    write_json(
        &dir.join(DOCUMENT_V2_SHADOW_FILE),
        &physical_shadow_for(&authoring),
    )
    .unwrap();

    // 按需迁移填充 DB 权威稿，然后通过工作区保存链改标题。
    crate::library::commands::get_workspace_item_core(&root, &job.job_id)
        .expect("on-demand migration must seed the canonical draft");
    crate::library::commands::apply_editor_commands_core(
        &root,
        crate::library::repository::ApplyEditorCommandsInput {
            item_id: job.job_id.clone(),
            base_version: 1,
            request_id: Some(format!("legacy-{}", job.job_id)),
            commands: vec![],
            title: Some("权威稿标题".to_string()),
        },
    )
    .expect("DB title edit must succeed");

    let session = get_authoring_v2_core(&root, &job.job_id).expect("session must load");
    assert_eq!(
        session.pointer("/authoring/exam/title").and_then(Value::as_str),
        Some("权威稿标题"),
        "旧读取链必须返回 DB 权威稿，而不是过期 shadow"
    );
    let on_disk: Value =
        serde_json::from_slice(&fs::read(dir.join(AUTHORING_V2_SHADOW_FILE)).unwrap()).unwrap();
    assert_eq!(
        on_disk.pointer("/exam/title").and_then(Value::as_str),
        Some(shadow_title.as_str()),
        "shadow 仍是编辑前缓存，用于证明上面读取的确实是 DB 权威稿"
    );

    let _ = fs::remove_dir_all(root);
}

fn find_first_text_node_id(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) == Some("text") {
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            return Some(id.to_string());
        }
    }
    match value {
        Value::Array(items) => items.iter().find_map(find_first_text_node_id),
        Value::Object(map) => map.values().find_map(find_first_text_node_id),
        _ => None,
    }
}

fn find_first_text_node_text(value: &Value, node_id: &str) -> Option<String> {
    if value.get("id").and_then(Value::as_str) == Some(node_id) {
        return value
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| find_first_text_node_text(item, node_id)),
        Value::Object(map) => map
            .values()
            .find_map(|item| find_first_text_node_text(item, node_id)),
        _ => None,
    }
}

fn count_revision_files(job_dir: &Path) -> usize {
    let revisions = job_dir.join("authoring").join("revisions");
    match fs::read_dir(&revisions) {
        Ok(entries) => entries.flatten().count(),
        Err(_) => 0,
    }
}
