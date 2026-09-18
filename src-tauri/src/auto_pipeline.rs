use crate::{
    authoring_pipeline::{
        dynamic_block_text, dynamic_document_blocks, make_dynamic_authoring_ir,
        make_dynamic_split_candidates, merge_answer_source_candidates, parse_dynamic_answer_text,
    },
    authoring_review::refresh_authoring_review_state,
    cleanup::minimize_process_artifacts_after_authoring,
    docx_facts_shadow::write_docx_facts_shadow_with_v1,
    environment::{authoring_v2_shadow_enabled, quality_gate_v2_enabled},
    ielts_grammar::{
        build_authoring_v2_shadow, write_authoring_v2_shadow,
        SHADOW_ARTIFACT_FILE as AUTHORING_V2_SHADOW_ARTIFACT_FILE,
        SHADOW_COMPARE_FILE as AUTHORING_V2_SHADOW_COMPARE_FILE,
        SHADOW_ERROR_FILE as AUTHORING_V2_SHADOW_ERROR_FILE,
    },
    job_store::{load_job, update_job},
    llm_gateway::run_llm_gateway,
    llm_profiles::{find_profile, load_llm_api_key, load_profiles},
    llm_suggestions::{
        apply_suggestion_to_authoring, deterministic_llm_output, llm_suggestion_auto_apply_issues,
        llm_suggestion_quote_mismatches, make_adjudication_input,
        make_cloud_paper_generation_input, make_llm_input, make_source_verification_input,
        make_vision_answer_extraction_input, make_vision_transcription_input, save_llm_suggestion,
    },
    main_source_file,
    parser::{
        extract_pdf_images_for_vision, image_count_from_extraction, missing_source_document_ir,
        parse_source_document, vision_transcription_document_ir,
    },
    pdf_facts_shadow::{
        write_pdf_facts_shadow_with_v1, SHADOW_ARTIFACT_FILE as DOCUMENT_V2_SHADOW_ARTIFACT_FILE,
        SHADOW_ERROR_FILE as DOCUMENT_V2_SHADOW_ERROR_FILE,
    },
    reading_source::{
        answer_key_from_authoring, display_map_from_authoring, question_order_from_authoring,
    },
    recognition::{
        write_question_layout_graph_artifact, QUESTION_LAYOUT_GRAPH_ARTIFACT_FILE,
        QUESTION_LAYOUT_GRAPH_ERROR_FILE,
    },
    runtime_validation::validate_for_runtime_gate,
    source_review::{
        low_confidence_block_ids, parser_warnings, source_review_issues, source_review_status,
        write_source_review_status,
    },
    util::{ensure_job_dirs, job_dir, read_json_opt, write_json, write_text},
    AutoPipelineInput, CommandResult, ImportJob, JobStatus, RunCloudReviewInput, SourceFile,
    WorkflowStep,
};
use crate::artifact_store::write_canonical_json_atomic;
use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::thread;
use uuid::Uuid;
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use zip::ZipArchive;

const LOCAL_PLACEHOLDER_PROFILE_ID: &str = "profile-local-placeholder";

fn answer_key_sources(job: &ImportJob) -> Vec<&SourceFile> {
    job.source_files
        .iter()
        .filter(|source| source.role == "AnswerKey")
        .collect()
}

pub(crate) fn parse_answer_source_candidates(
    root: &Path,
    job: &ImportJob,
    mode: &str,
) -> CommandResult<Vec<Value>> {
    let mut candidates = Vec::new();
    for source in answer_key_sources(job) {
        let upload_path = job_dir(root, &job.job_id)
            .join("uploads")
            .join(&source.stored_name);
        if !upload_path.exists() {
            candidates.push(json!({
                "source": format!("answer-source-missing:{}", source.file_id),
                "sourceFileId": source.file_id,
                "sourceStoredName": source.stored_name,
                "answers": {},
                "warnings": [format!("Answer key source file missing: {}", source.original_name)]
            }));
            continue;
        }
        if !matches!(source.file_type.as_str(), "txt" | "md" | "pdf" | "docx") {
            candidates.push(json!({
                "source": format!("answer-source-unsupported:{}", source.file_id),
                "sourceFileId": source.file_id,
                "sourceStoredName": source.stored_name,
                "answers": {},
                "warnings": [format!("Unsupported answer key source type: {}", source.file_type)]
            }));
            continue;
        }
        let parser_output = root.join("cache").join("parser").join(format!(
            "{}-answer-{}-document-ir.json",
            job.job_id, source.file_id
        ));
        let answer_doc = parse_source_document(job, source, &upload_path, &parser_output, mode)?;
        let mut answers = serde_json::Map::new();
        for block in dynamic_document_blocks(Some(&answer_doc)) {
            for (key, value) in parse_dynamic_answer_text(&dynamic_block_text(&block)) {
                answers.insert(key, value);
            }
        }
        let warnings = parser_warnings(Some(&answer_doc));
        candidates.push(json!({
            "source": format!("answer-source:{}", source.file_id),
            "sourceFileId": source.file_id,
            "sourceStoredName": source.stored_name,
            "provider": answer_doc.pointer("/parser/provider").cloned().unwrap_or(Value::Null),
            "answers": answers,
            "warnings": warnings
        }));
    }
    Ok(candidates)
}

pub(crate) fn select_llm_profile(
    root: &Path,
    job: &ImportJob,
    requested_profile_id: Option<String>,
) -> Option<String> {
    let profiles = load_profiles(root).unwrap_or_default();
    let preferred = requested_profile_id.or_else(|| job.active_llm_profile_id.clone());
    if let Some(profile_id) = preferred {
        let enabled = profiles.iter().any(|profile| {
            profile.get("profileId").and_then(Value::as_str) == Some(profile_id.as_str())
                && profile
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
        });
        if enabled {
            return Some(profile_id);
        }
    }

    let enabled_profiles = profiles.into_iter().filter(|profile| {
        profile
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    });

    let mut best: Option<(String, (u8, u8, u8))> = None;
    for profile in enabled_profiles {
        let Some(profile_id) = profile
            .get("profileId")
            .and_then(Value::as_str)
            .map(ToString::to_string)
        else {
            continue;
        };
        let has_api_key = profile
            .get("hasApiKey")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let is_placeholder = profile_id == LOCAL_PLACEHOLDER_PROFILE_ID;
        let is_openai_compatible = profile
            .get("provider")
            .and_then(Value::as_str)
            .map(|value| value == "OpenAiCompatible")
            .unwrap_or(false);
        let score = (
            if has_api_key { 2 } else { 0 },
            if !is_placeholder { 1 } else { 0 },
            if is_openai_compatible { 1 } else { 0 },
        );
        match best {
            Some((_, current_score)) if current_score >= score => {}
            _ => best = Some((profile_id, score)),
        }
    }
    best.map(|(profile_id, _)| profile_id)
}

pub(crate) fn main_pdf_needs_vision_transcription(job: &ImportJob, doc: &Value) -> bool {
    let Some(source) = main_source_file(job) else {
        return false;
    };
    if source.file_type != "pdf" {
        return false;
    }
    let warnings = parser_warnings(Some(doc)).join("\n").to_lowercase();
    let provider = doc
        .pointer("/parser/provider")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let split = make_dynamic_split_candidates(&job.job_id, job, Some(doc));
    let lacks_reliable_groups = !legacy_has_reliable_question_groups(&split);
    provider != "vision-llm-transcription"
        && (warnings.contains("no extractable text")
            || warnings.contains("ocr/manual review required")
            || !low_confidence_block_ids(Some(doc), 0.5).is_empty()
            || lacks_reliable_groups)
}

fn has_reliable_question_groups(
    job_id: &str,
    job: &ImportJob,
    doc: &Value,
    physical_shadow: Option<&Value>,
) -> bool {
    let split = make_dynamic_split_candidates(job_id, job, Some(doc));
    if !quality_gate_v2_enabled() {
        return legacy_has_reliable_question_groups(&split);
    }
    let v1_authoring = make_dynamic_authoring_ir(job, &split, Some(doc));
    let v2_authoring =
        build_authoring_v2_shadow(job, &v1_authoring, &split, Some(doc), physical_shadow).ok();
    question_groups_are_reliable(true, &split, v2_authoring.as_ref())
}

fn question_groups_are_reliable(
    quality_gate_enabled: bool,
    split: &Value,
    v2_authoring: Option<&Value>,
) -> bool {
    if !quality_gate_enabled {
        return legacy_has_reliable_question_groups(split);
    }
    v2_authoring.is_some_and(|authoring| {
        authoring.pointer("/quality/state").and_then(Value::as_str) == Some("ready")
            && authoring
                .get("answerSlots")
                .and_then(Value::as_object)
                .is_some_and(|slots| !slots.is_empty())
    })
}

fn quality_gate_requires_review(quality_gate_enabled: bool, quality: &Value) -> bool {
    quality_gate_enabled && quality.get("state").and_then(Value::as_str) != Some("ready")
}

fn quality_gate_review_count(quality_gate_enabled: bool, quality: &Value) -> u32 {
    if !quality_gate_requires_review(quality_gate_enabled, quality) {
        return 0;
    }
    let issue_count = quality
        .get("issues")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let hard_failure_count = quality
        .get("hardFailures")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    issue_count.max(hard_failure_count).max(1) as u32
}

fn physical_shadow_matches_source(shadow: &Value, job: &ImportJob) -> bool {
    let Some(source) = main_source_file(job) else {
        return false;
    };
    shadow.get("schemaVersion").and_then(Value::as_str) == Some("DocumentIRV2")
        && shadow.get("jobId").and_then(Value::as_str) == Some(job.job_id.as_str())
        && shadow
            .get("sourceFiles")
            .and_then(Value::as_array)
            .is_some_and(|sources| {
                sources.iter().any(|candidate| {
                    candidate.get("sourceFileId").and_then(Value::as_str)
                        == Some(source.file_id.as_str())
                        && candidate.get("sha256").and_then(Value::as_str)
                            == Some(source.sha256.as_str())
                })
            })
}

fn current_physical_shadow(dir: &Path, job: &ImportJob) -> Option<Value> {
    read_json_opt(&dir.join(DOCUMENT_V2_SHADOW_ARTIFACT_FILE))
        .ok()
        .flatten()
        .filter(|shadow| physical_shadow_matches_source(shadow, job))
}

/// P4-T02: derive `QuestionLayoutGraphV1` from the job's physical `DocumentIRV2`.
///
/// The recognition main path reads `DocumentIRV2` directly instead of the V1
/// `questionGroupCandidates`, so the geometric layout graph has to be produced
/// with the physical document, not reconstructed later from split candidates.
///
/// Deliberately non-fatal: the import has already produced a valid physical
/// document, and the graph is additive evidence. A failure is recorded beside
/// the artifact rather than failing the job.
fn materialize_question_layout_graph(dir: &Path, job: &ImportJob) {
    let Some(document) = current_physical_shadow(dir, job) else {
        return;
    };
    let graph_path = dir.join(QUESTION_LAYOUT_GRAPH_ARTIFACT_FILE);
    let error_path = dir.join(QUESTION_LAYOUT_GRAPH_ERROR_FILE);
    match write_question_layout_graph_artifact(&document, &graph_path) {
        Ok(graph) => {
            let _ = fs::remove_file(&error_path);
            let _ = graph;
        }
        Err(error) => {
            let _ = write_json(
                &error_path,
                &json!({
                    "schemaVersion": "QuestionLayoutGraphErrorV1",
                    "jobId": job.job_id,
                    "error": error,
                    "recordedAt": Utc::now().to_rfc3339()
                }),
            );
        }
    }
}

fn write_pipeline_authoring_v2_shadow(
    dir: &Path,
    job: &ImportJob,
    authoring: &Value,
    split: &Value,
    document: Option<&Value>,
    physical_shadow: Option<&Value>,
) -> CommandResult<()> {
    if !authoring_v2_shadow_enabled() {
        return Ok(());
    }
    let shadow_path = dir.join(AUTHORING_V2_SHADOW_ARTIFACT_FILE);
    let error_path = dir.join(AUTHORING_V2_SHADOW_ERROR_FILE);
    // G2-T04：direct canonical（QLG → IeltsAuthoringIRV2，不经 V1 authoring）。
    // flag 默认关闭；物理层或 QLG 图不可用时回退 V1 链（可回滚）。
    if crate::environment::qlg_direct_canonical_enabled() {
        let graph_path = dir.join(crate::recognition::QUESTION_LAYOUT_GRAPH_ARTIFACT_FILE);
        if physical_shadow.is_none() {
            eprintln!("[direct-canonical] physical shadow missing; falling back to V1 chain");
        }
        if let (Some(physical), Ok(graph_value)) = (physical_shadow, fs::read_to_string(&graph_path)) {
            let asset_resolver = |asset_id: &str| -> Option<crate::recognition::direct_canonical::ResolvedVisualAsset> {
                let candidate = dir.join("assets").join(format!("{asset_id}.png"));
                let bytes = fs::read(&candidate).ok()?;
                Some(crate::recognition::direct_canonical::ResolvedVisualAsset {
                    sha256: format!("{:x}", Sha256::digest(&bytes)),
                    byte_length: bytes.len() as u64,
                    relative_path: candidate
                        .strip_prefix(dir)
                        .unwrap_or(&candidate)
                        .to_string_lossy()
                        .to_string(),
                })
            };
            match serde_json::from_str::<crate::recognition::local::QuestionLayoutGraphV1>(&graph_value)
                .map_err(|error| format!("direct_canonical_graph_deserialize:{error}"))
                .and_then(|graph| {
                    crate::recognition::direct_canonical::build_direct_canonical(
                        job,
                        &graph,
                        physical,
                        split,
                        &asset_resolver,
                    )
                }) {
                Ok(direct) => {
                    // 与 V1 链同纪律：原子写 + 清理陈旧 compare（轮 2 verifier V4）。
                    write_canonical_json_atomic(&shadow_path, &direct)?;
                    let _ = fs::remove_file(&error_path);
                    let _ = fs::remove_file(dir.join(AUTHORING_V2_SHADOW_COMPARE_FILE));
                    return Ok(());
                }
                Err(error) => {
                    eprintln!("[direct-canonical] build failed; falling back to V1 chain: {error}");
                }
            }
        }
    }
    match write_authoring_v2_shadow(
        dir,
        job,
        authoring,
        split,
        document,
        physical_shadow,
        &shadow_path,
    ) {
        Ok(_) => {
            let _ = fs::remove_file(error_path);
        }
        Err(error) => {
            let _ = fs::remove_file(&shadow_path);
            let _ = fs::remove_file(dir.join(AUTHORING_V2_SHADOW_COMPARE_FILE));
            write_json(
                &error_path,
                &json!({
                    "schemaVersion": "IeltsAuthoringIRV2ShadowErrorV1",
                    "jobId": job.job_id,
                    "error": error,
                    "recordedAt": Utc::now().to_rfc3339()
                }),
            )?;
        }
    }
    Ok(())
}

fn legacy_has_reliable_question_groups(split: &Value) -> bool {
    split
        .get("questionGroupCandidates")
        .and_then(Value::as_array)
        .map(|groups| {
            groups.iter().any(|group| {
                group
                    .get("requiresManualQuestionImport")
                    .and_then(Value::as_bool)
                    != Some(true)
                    && group
                        .get("questionRange")
                        .and_then(Value::as_array)
                        .map(|range| range.len() == 2)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn main_source_is_pdf(job: &ImportJob) -> bool {
    main_source_file(job)
        .map(|source| source.file_type.as_str() == "pdf")
        .unwrap_or(false)
}

fn main_pdf_upload_path(root: &Path, job: &ImportJob) -> CommandResult<(SourceFile, PathBuf)> {
    let source = main_source_file(job)
        .cloned()
        .ok_or_else(|| "no_main_source_file".to_string())?;
    if source.file_type != "pdf" {
        return Err(format!("main_source_is_not_pdf:{}", source.file_type));
    }
    let upload_path = job_dir(root, &job.job_id)
        .join("uploads")
        .join(&source.stored_name);
    if !upload_path.exists() {
        return Err(format!(
            "main_source_file_missing_for_vision:{}",
            upload_path.display()
        ));
    }
    Ok((source, upload_path))
}

/// 云端识别链的源文件解析：PDF 与 DOCX 都允许通过。
///
/// 与 [`main_pdf_upload_path`] 的关键区别是**不要求** `file_type == "pdf"`。
/// 云端链需要拒绝的是「没有可用源文件」，而不是「来源不是 PDF」——否则 DOCX
/// 永远到不了模型，云端识别对 DOCX 形同不存在。非 PDF 的证据面改由
/// [`document_ir_source_text`] 提供，见 [`generate_cloud_reading_outline`]。
fn main_source_for_cloud(root: &Path, job: &ImportJob) -> CommandResult<(SourceFile, PathBuf)> {
    let source = main_source_file(job)
        .cloned()
        .ok_or_else(|| "no_main_source_file".to_string())?;
    let upload_path = job_dir(root, &job.job_id)
        .join("uploads")
        .join(&source.stored_name);
    if !upload_path.exists() {
        return Err(format!(
            "main_source_file_missing_for_cloud:{}",
            upload_path.display()
        ));
    }
    Ok((source, upload_path))
}

/// 从作业的 `DocumentIRV2` 影子稿中抽取纯文本，作为非 PDF 来源的云端证据面。
///
/// 只按页/行顺序拼接原文件里**已经出现过**的字符串，不掺入任何本地结论：
/// 模型看到的证据必须来自原文件本身，否则「本地抽错了、云端也跟着错」，
/// 三路核验就退化成了自我印证。抽不出文本时返回 `None`，由调用方如实报错。
fn document_ir_source_text(root: &Path, job_id: &str) -> Option<String> {
    let ir = read_json_opt(&job_dir(root, job_id).join("document-ir.json"))
        .ok()
        .flatten()?;
    let mut out = String::new();
    for page in ir.get("pages").and_then(Value::as_array).into_iter().flatten() {
        let lines = page.get("lines").and_then(Value::as_array);
        let spans = page.get("spans").and_then(Value::as_array);
        if let Some(lines) = lines {
            for line in lines {
                if let Some(text) = line.get("text").and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        out.push_str(text);
                        out.push('\n');
                    }
                }
            }
        } else if let Some(spans) = spans {
            for span in spans {
                if let Some(text) = span.get("text").and_then(Value::as_str) {
                    out.push_str(text);
                }
            }
            out.push('\n');
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 云端证据面准备：**从原始上传文件直接抽取纯文本**，绝不依赖本地识别产物
/// `document-ir.json`，因此与本地语义识别并行起飞时不会因后者尚未落盘而 race/失败。
///
/// 这是修复「DOCX 云端输入依赖本地识别产物」的核心 seam：本地链与云端链都可以
/// 各自独立地准备证据面（PDF 走视觉抽取 `pdfPath`，DOCX/TXT/MD 走这里的原文件直读），
/// 云端不再「等待本地语义识别完成」。
///
/// - `pdf`：返回 `None`——PDF 证据面由视觉抽取（`pdfPath` + `main_pdf_vision_extraction`）
///   提供，不需要纯文本；
/// - `txt` / `md`：直接按 UTF-8 读取原文件；
/// - `docx`：拆 zip 读 `word/document.xml`（及页眉页脚），收集 `<w:t>` 文本，对原文件
///   做一遍**独立**抽取；
/// - 其它 / 缺文件 / 抽不出文本：返回 `None`，由调用方如实报 `cloud_source_text_unavailable`。
fn prepare_cloud_source_evidence(root: &Path, job: &ImportJob) -> Option<String> {
    let source = main_source_file(job)?;
    let upload_path = job_dir(root, &job.job_id)
        .join("uploads")
        .join(&source.stored_name);
    if !upload_path.exists() {
        return None;
    }
    match source.file_type.as_str() {
        "pdf" => None,
        "txt" | "md" => {
            // `sourceText` 网关路径（`llm_gateway.rs`）仍按此文本工作。
            fs::read_to_string(&upload_path).ok().filter(|text| !text.trim().is_empty())
        }
        "docx" => extract_docx_plain_text(&upload_path),
        _ => None,
    }
}

/// 从原始 DOCX 文件直接抽取纯文本（独立于本地识别）。只读 `word/document.xml`、
/// 页眉页脚、脚注/尾注等文本承载部件，收集 `<w:t>` 内容，并在段落边界补换行。
fn extract_docx_plain_text(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;
    let mut parts: Vec<String> = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).ok()?;
        let name = entry.name().to_string();
        let is_text_part = name == "word/document.xml"
            || name.starts_with("word/header")
            || name.starts_with("word/footer")
            || name == "word/footnotes.xml"
            || name == "word/endnotes.xml";
        if !is_text_part {
            continue;
        }
        let mut bytes = Vec::new();
        // `ZipFile` 在 `ZipArchive` 借用下读取，写入自有 buffer 后脱离借用。
        entry.read_to_end(&mut bytes).ok()?;
        parts.push(docx_part_plain_text(&bytes));
    }
    let mut out = String::new();
    for part in parts {
        let trimmed = part.trim();
        if !trimmed.is_empty() {
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 解析单个 DOCX XML 部件，抽取可见文本：`<w:t>` 内容收集为文本，段落（`w:p`）
/// 边界补一个换行，`w:tab`/`w:br`/`w:cr` 转成制表/换行。这与本地识别的 `document-ir.json`
/// 抽出无关，只反映原文件里的字符串。
fn docx_part_plain_text(xml: &[u8]) -> String {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut out = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => {
                let name = event.name();
                let local = local_name_of(name.as_ref());
                if local == "t" {
                    in_text = true;
                } else if local == "p" {
                    out.push('\n');
                } else if local == "tab" {
                    out.push('\t');
                } else if local == "br" || local == "cr" {
                    out.push('\n');
                }
            }
            Ok(Event::Empty(event)) => {
                let name = event.name();
                let local = local_name_of(name.as_ref());
                if local == "tab" {
                    out.push('\t');
                } else if local == "br" || local == "cr" {
                    out.push('\n');
                }
            }
            Ok(Event::End(event)) => {
                let name = event.name();
                let local = local_name_of(name.as_ref());
                if local == "t" {
                    in_text = false;
                } else if local == "p" {
                    out.push('\n');
                }
            }
            Ok(Event::Text(event)) => {
                if in_text {
                    let raw = String::from_utf8_lossy(event.as_ref());
                    if let Ok(value) = unescape(raw.as_ref()) {
                        out.push_str(value.as_ref());
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// XML 元素名的本地名（去掉 `w:` 之类命名空间前缀），用于 `<w:t>`/`<w:p>` 判断。
fn local_name_of(name: &[u8]) -> &str {
    let text = std::str::from_utf8(name).unwrap_or("");
    text.rsplit(':').next().unwrap_or("")
}

fn main_pdf_vision_extraction(root: &Path, job: &ImportJob) -> CommandResult<(Value, PathBuf)> {
    let (_source, upload_path) = main_pdf_upload_path(root, job)?;
    let dir = job_dir(root, &job.job_id);
    let cache_dir = dir.join("cache").join("vision");
    let extraction_path = cache_dir.join("pdf-images.json");
    let asset_dir = cache_dir.join("assets");
    let extraction =
        extract_pdf_images_for_vision(&job.job_id, &upload_path, &extraction_path, &asset_dir)?;
    Ok((extraction, asset_dir))
}

pub(crate) fn vision_transcription_for_job(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    note: Option<&str>,
) -> CommandResult<(Value, Value)> {
    vision_transcription_for_job_with_gateway(root, job, profile_id, note, &mut run_llm_gateway)
}

fn vision_transcription_for_job_with_gateway<F>(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    note: Option<&str>,
    llm_gateway: &mut F,
) -> CommandResult<(Value, Value)>
where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value>,
{
    let (extraction, asset_dir) = main_pdf_vision_extraction(root, job)?;
    vision_transcription_with_extraction(
        root,
        job,
        profile_id,
        note,
        &extraction,
        &asset_dir,
        llm_gateway,
    )
}

#[allow(clippy::too_many_arguments)]
fn vision_transcription_with_extraction<F>(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    note: Option<&str>,
    extraction: &Value,
    asset_dir: &Path,
    llm_gateway: &mut F,
) -> CommandResult<(Value, Value)>
where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value>,
{
    let profile = find_profile(root, profile_id)?;
    let image_count = image_count_from_extraction(extraction);
    if image_count == 0 {
        return Err("vision_transcription_no_extractable_pdf_images".to_string());
    }

    let input = make_vision_transcription_input(&profile, job, profile_id, extraction);
    let api_key = load_llm_api_key(root, profile_id);
    let output = llm_gateway(
        root,
        &job.job_id,
        "transcribe_pdf_images",
        &input,
        api_key.as_deref(),
    )?;
    let text = output
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(format!(
            "vision_transcription_empty:{}",
            output.get("warnings").cloned().unwrap_or_else(|| json!([]))
        ));
    }
    let confidence = output
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.6);
    let warnings = output
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let evidence = json!({
        "profileId": profile_id,
        "imageCount": image_count,
        "extraction": {
            "warnings": extraction.get("warnings").cloned().unwrap_or_else(|| json!([])),
            "assetDir": asset_dir
        },
        "model": output.get("evidence").cloned().unwrap_or_else(|| json!({}))
    });
    let ir = vision_transcription_document_ir(job, &text, confidence, warnings, evidence, note);
    Ok((ir, output))
}

fn vision_answer_candidate_for_job<F>(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    extraction: &Value,
    llm_gateway: &mut F,
) -> CommandResult<(Value, Value)>
where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value>,
{
    let profile = find_profile(root, profile_id)?;
    let image_count = image_count_from_extraction(extraction);
    if image_count == 0 {
        return Err("vision_answer_no_extractable_pdf_images".to_string());
    }
    let input = make_vision_answer_extraction_input(&profile, job, profile_id, extraction);
    let api_key = load_llm_api_key(root, profile_id);
    let output = llm_gateway(
        root,
        &job.job_id,
        "extract_pdf_image_answers",
        &input,
        api_key.as_deref(),
    )?;
    let answers = output.get("answers").cloned().unwrap_or_else(|| json!({}));
    let candidate = json!({
        "schemaVersion": "VisionAnswerCandidateV1",
        "source": format!("vision-answer-images:{}", profile_id),
        "provider": profile.get("provider").cloned().unwrap_or_else(|| json!("OpenAiCompatible")),
        "profileId": profile_id,
        "model": profile.get("model").cloned().unwrap_or(Value::Null),
        "jobId": job.job_id,
        "answers": answers,
        "confidence": output.get("confidence").cloned().unwrap_or(Value::Null),
        "warnings": output.get("warnings").cloned().unwrap_or_else(|| json!([])),
        "evidence": output.get("evidence").cloned().unwrap_or_else(|| json!([])),
        "imageCount": image_count,
        "extractionWarnings": extraction.get("warnings").cloned().unwrap_or_else(|| json!([]))
    });
    Ok((candidate, output))
}

fn normalized_compare_text(value: &Value) -> String {
    match value {
        Value::Array(items) => items
            .iter()
            .map(normalized_compare_text)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join("|"),
        Value::String(text) => text
            .trim()
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | '.' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase()
            .replace(['-', '_'], " "),
        Value::Number(_) | Value::Bool(_) => value.to_string().to_ascii_uppercase(),
        _ => String::new(),
    }
}

fn answer_key_by_display_from_authoring(ir: &Value) -> serde_json::Map<String, Value> {
    let mut answers = serde_json::Map::new();
    for group in ir
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for question in group
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let display = question
                .get("displayNumber")
                .and_then(Value::as_str)
                .or_else(|| question.get("id").and_then(Value::as_str))
                .unwrap_or_default()
                .trim_start_matches('q')
                .trim_start_matches('Q')
                .to_string();
            if display.is_empty() {
                continue;
            }
            answers.insert(
                display,
                question.get("answer").cloned().unwrap_or_else(|| json!("")),
            );
        }
    }
    answers
}

pub(crate) fn answer_question_ids_from_authoring(ir: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for group in ir
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for question in group
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let qid = question
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if qid.is_empty() {
                continue;
            }
            if !crate::authoring_review::answer_is_empty(question.get("answer")) {
                ids.push(qid);
            }
        }
    }
    ids
}

pub(crate) fn empty_answer_question_ids_from_authoring(ir: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for group in ir
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for question in group
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let qid = question
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if qid.is_empty() {
                continue;
            }
            if crate::authoring_review::answer_is_empty(question.get("answer")) {
                ids.push(qid);
            }
        }
    }
    ids
}

fn empty_prompt_question_ids_from_authoring(ir: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for group in ir
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for question in group
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let qid = question
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if qid.is_empty() {
                continue;
            }
            let prompt = question
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if prompt.trim().is_empty() {
                ids.push(qid);
            }
        }
    }
    ids
}

fn build_vision_transcription_summary_issue(
    vision_transcription: &Value,
    ir: &Value,
) -> Option<Value> {
    let attempted = vision_transcription
        .get("attempted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let applied = vision_transcription
        .get("applied")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let failure = vision_transcription
        .get("failure")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let profile_id = vision_transcription
        .get("profileId")
        .and_then(Value::as_str);
    let missing_prompt_question_ids = empty_prompt_question_ids_from_authoring(ir);
    let profile_unavailable =
        profile_id.is_none() || profile_id == Some(LOCAL_PLACEHOLDER_PROFILE_ID);

    let message = if missing_prompt_question_ids.is_empty() && failure.is_none() {
        return None;
    } else if !attempted && profile_unavailable {
        "未配置可用云端模型，视觉题目识别未启动；当前仅保留本地解析结果，题干已留空。"
    } else if !attempted {
        "视觉题目识别未启动；当前仅保留本地解析结果，题干已留空。"
    } else if !applied {
        "视觉题目识别已尝试，但未生成可靠题组；当前保留本地解析结果，题干已留空。"
    } else if !missing_prompt_question_ids.is_empty() {
        "视觉题目识别已尝试，但仍有题干未能可靠提取；当前未识别题干已留空，请人工补齐。"
    } else {
        return None;
    };

    Some(json!({
        "layer": "Parser",
        "path": "$.parser.visionTranscription",
        "kind": "vision_transcription_summary",
        "status": "needs_review",
        "message": message,
        "attempted": attempted,
        "applied": applied,
        "profileId": vision_transcription.get("profileId").cloned().unwrap_or(Value::Null),
        "confidence": vision_transcription.get("confidence").cloned().unwrap_or(Value::Null),
        "warnings": vision_transcription.get("warnings").cloned().unwrap_or_else(|| json!([])),
        "failure": vision_transcription.get("failure").cloned().unwrap_or(Value::Null),
        "missingPromptQuestionIds": missing_prompt_question_ids
    }))
}

fn outline_group_summary_from_local(ir: &Value) -> Value {
    json!(ir
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|group| {
            json!({
                "range": group.get("questionRange").cloned().unwrap_or(Value::Null),
                "kind": group.get("kind").cloned().unwrap_or(Value::Null),
                "layoutHint": group.pointer("/layout/layoutHint").cloned().unwrap_or(Value::Null),
                "questionIds": group.get("questions").and_then(Value::as_array).map(|questions| {
                    questions.iter().filter_map(|question| question.get("id").and_then(Value::as_str)).collect::<Vec<_>>()
                }).unwrap_or_default()
            })
        })
        .collect::<Vec<_>>())
}

fn outline_group_summary_from_cloud(output: &Value) -> Value {
    json!(output
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|group| {
            json!({
                "range": group.get("range").cloned().unwrap_or(Value::Null),
                "kind": group.get("kind").cloned().unwrap_or(Value::Null),
                "layoutHint": group.get("layoutHint").cloned().unwrap_or(Value::Null),
                "questionIds": group.get("questionIds").cloned().unwrap_or_else(|| json!([]))
            })
        })
        .collect::<Vec<_>>())
}

fn semantic_group_kind_family(kind: &str) -> &str {
    match kind {
        "summary_completion" | "sentence_completion" | "note_completion" | "notes_completion" => {
            "completion"
        }
        value => value,
    }
}

fn expected_question_ids(start: u64, end: u64) -> Vec<String> {
    (start..=end).map(|number| format!("q{}", number)).collect()
}

fn local_question_ids(group: &Value) -> Vec<String> {
    group
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| question.get("id").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect()
}

fn cloud_question_ids(group: &Value) -> Vec<String> {
    group
        .get("questionIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect()
}

fn text_has_inline_blank_marker(text: &str) -> bool {
    text.contains("___") || text.contains("……") || text.contains("...") || text.contains("_____")
}

fn text_has_notes_completion_signal(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("complete the notes")
        || lower.contains("note completion")
        || lower.contains("notes below")
        || (lower.contains("notes") && text_has_inline_blank_marker(text))
}

fn local_inline_notes_evidence(group: &Value) -> bool {
    let local_layout = group
        .pointer("/layout/layoutHint")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if local_layout != "inline_completion" {
        return false;
    }
    let notes = group
        .pointer("/layout/notes")
        .and_then(Value::as_str)
        .unwrap_or_default();
    text_has_notes_completion_signal(notes)
        || (text_has_inline_blank_marker(notes) && local_question_ids(group).len() > 1)
}

fn cloud_evidence_quote_count(group: &Value) -> usize {
    group
        .pointer("/evidence/quotes")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn cloud_outline_comparison(ir: &Value, output: &Value) -> Value {
    let local_groups = ir
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let cloud_groups = output
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut issues = Vec::<Value>::new();
    let mut observations = Vec::<Value>::new();
    if cloud_groups.is_empty() {
        issues.push(json!({
            "kind": "cloud_groups_missing",
            "message": "云端对照没有返回可核对的题组。"
        }));
    }

    for cloud_group in &cloud_groups {
        let range = cloud_group
            .get("range")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let start = range.first().and_then(Value::as_u64).unwrap_or(0);
        let end = range.get(1).and_then(Value::as_u64).unwrap_or(start);
        if start == 0 || end == 0 {
            issues.push(json!({
                "kind": "cloud_group_range_missing",
                "message": "云端对照返回了缺少题号范围的题组。"
            }));
            continue;
        }
        let local = local_groups.iter().find(|group| {
            let range = group
                .get("questionRange")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            range.first().and_then(Value::as_u64) == Some(start)
                && range.get(1).and_then(Value::as_u64) == Some(end)
        });
        let Some(local) = local else {
            issues.push(json!({
                "kind": "cloud_group_missing_locally",
                "range": [start, end],
                "message": format!("云端对照发现 Q{}-{} 题组，但本地题稿中没有对应题组。", start, end)
            }));
            continue;
        };
        let expected_qids = expected_question_ids(start, end);
        let local_qids = local_question_ids(local);
        if !local_qids.is_empty() && local_qids != expected_qids {
            issues.push(json!({
                "kind": "local_group_question_ids_do_not_match_range",
                "range": [start, end],
                "localQuestionIds": local_qids,
                "expectedQuestionIds": expected_qids,
                "message": format!("Q{}-{} 的本地题号归属不完整，请确认同一题组内是否包含所有题号。", start, end)
            }));
        }
        let cloud_qids = cloud_question_ids(cloud_group);
        if cloud_qids.is_empty() {
            observations.push(json!({
                "kind": "cloud_question_ids_missing",
                "range": [start, end],
                "message": "云端对照没有返回逐题 questionIds，本次只按题号范围和本地题稿证据核对。"
            }));
        } else if cloud_qids != expected_qids {
            issues.push(json!({
                "kind": "cloud_group_question_ids_do_not_match_range",
                "range": [start, end],
                "cloudQuestionIds": cloud_qids,
                "expectedQuestionIds": expected_qids,
                "message": format!("云端对照返回的 Q{}-{} 题号归属不完整，请确认是否被拆成了独立题。", start, end)
            }));
        }
        if cloud_evidence_quote_count(cloud_group) == 0 {
            observations.push(json!({
                "kind": "cloud_group_evidence_missing",
                "range": [start, end],
                "message": "云端对照没有提供可见原文证据，结构判断已降权。"
            }));
        }
        let cloud_kind = cloud_group
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("");
        let local_kind = local.get("kind").and_then(Value::as_str).unwrap_or("");
        let cloud_family = semantic_group_kind_family(cloud_kind);
        let local_family = semantic_group_kind_family(local_kind);
        if !cloud_kind.is_empty() && !local_kind.is_empty() && cloud_kind != local_kind {
            if cloud_family == local_family && local_family == "completion" {
                observations.push(json!({
                    "kind": "cloud_completion_kind_normalized",
                    "range": [start, end],
                    "localKind": local_kind,
                    "cloudKind": cloud_kind,
                    "message": "云端和本地同属填空题家族，题型命名差异不作为结构错误。"
                }));
            } else {
                issues.push(json!({
                    "kind": "cloud_group_kind_mismatch",
                    "range": [start, end],
                    "localKind": local_kind,
                    "cloudKind": cloud_kind,
                    "message": format!("Q{}-{} 的题型与云端对照不一致，请确认题型。", start, end)
                }));
            }
        }
        let cloud_layout = cloud_group
            .get("layoutHint")
            .and_then(Value::as_str)
            .unwrap_or("");
        let local_layout = local
            .pointer("/layout/layoutHint")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !cloud_layout.is_empty() && !local_layout.is_empty() && cloud_layout != local_layout {
            if local_layout == "inline_completion"
                && local_inline_notes_evidence(local)
                && cloud_family == "completion"
                && local_family == "completion"
            {
                observations.push(json!({
                    "kind": "cloud_layout_deprioritized_by_local_inline_notes",
                    "range": [start, end],
                    "localLayout": local_layout,
                    "cloudLayout": cloud_layout,
                    "message": "本地已识别到连续 notes 原文和内联空格，云端列表布局不覆盖本地结构。"
                }));
            } else {
                issues.push(json!({
                    "kind": "cloud_group_layout_mismatch",
                    "range": [start, end],
                    "localLayout": local_layout,
                    "cloudLayout": cloud_layout,
                    "message": format!("Q{}-{} 的填答布局与云端对照不一致，请确认是否应在原文空格内作答。", start, end)
                }));
            }
        }
    }

    let local_answers = answer_key_by_display_from_authoring(ir);
    if let Some(cloud_answers) = output.get("answerKey").and_then(Value::as_object) {
        for (number, cloud_value) in cloud_answers {
            let Some(local_value) = local_answers.get(number) else {
                issues.push(json!({
                    "kind": "cloud_answer_missing_locally",
                    "questionNumber": number,
                    "message": format!("云端对照发现第 {} 题答案，但本地题稿中没有对应答案。", number)
                }));
                continue;
            };
            let local_text = normalized_compare_text(local_value);
            let cloud_text = normalized_compare_text(cloud_value);
            if !cloud_text.is_empty() && local_text.is_empty() {
                observations.push(json!({
                    "kind": "cloud_answer_candidate_only",
                    "questionNumber": number,
                    "cloudAnswer": cloud_value,
                    "message": format!("云端对照提供了第 {} 题答案候选，本地题稿未覆盖。", number)
                }));
            } else if !cloud_text.is_empty() && local_text != cloud_text {
                issues.push(json!({
                    "kind": "cloud_answer_mismatch",
                    "questionNumber": number,
                    "localAnswer": local_value,
                    "cloudAnswer": cloud_value,
                    "message": format!("第 {} 题答案与云端对照不一致，请确认答案。", number)
                }));
            }
        }
    }

    let max_issues = 20usize;
    let truncated = issues.len().saturating_sub(max_issues);
    issues.truncate(max_issues);
    json!({
        "passed": issues.is_empty() && truncated == 0,
        "issueCount": issues.len() + truncated,
        "issues": issues,
        "observations": observations,
        "truncatedIssueCount": truncated
    })
}

pub(crate) fn append_authoring_audit_issue(ir: &mut Value, issue: Value) {
    let Some(audit) = ir.get_mut("audit").and_then(Value::as_object_mut) else {
        return;
    };
    let message = issue
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if message.trim().is_empty() {
        return;
    }
    let issues = audit
        .entry("issues".to_string())
        .or_insert_with(|| json!([]));
    if !issues.is_array() {
        *issues = json!([]);
    }
    if let Some(items) = issues.as_array_mut() {
        if let Some(kind) = issue.get("kind").and_then(Value::as_str) {
            if matches!(
                kind,
                "cloud_comparison_summary"
                    | "vision_transcription_summary"
                    | "vision_answer_extraction_summary"
            ) {
                items.retain(|item| item.get("kind").and_then(Value::as_str) != Some(kind));
            }
        }
        if !items
            .iter()
            .any(|item| item.get("message").and_then(Value::as_str) == Some(&message))
        {
            items.push(issue);
        }
    }
}

fn cloud_outline_check_for_job<F>(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    extraction: &Value,
    ir: &Value,
    llm_gateway: &mut F,
) -> Value
where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value>,
{
    let output =
        cloud_outline_generate_with_gateway(root, job, profile_id, extraction, llm_gateway);
    cloud_outline_report_from_output(profile_id, ir, output)
}

/// Cloud outline LLM conversion. Depends only on the uploaded PDF page
/// images and the profile, so it can run concurrently with the local rule
/// conversion; the read-only comparison against the local draft happens
/// separately once the local draft exists.
fn cloud_outline_generate_with_gateway<F>(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    extraction: &Value,
    llm_gateway: &mut F,
) -> Result<Value, String>
where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value>,
{
    let profile = find_profile(root, profile_id)?;
    let (source, upload_path) = main_pdf_upload_path(root, job)?;
    let input = make_cloud_paper_generation_input(
        &profile,
        job,
        profile_id,
        &source,
        &upload_path,
        extraction,
    );
    let api_key = load_llm_api_key(root, profile_id);
    llm_gateway(
        root,
        &job.job_id,
        "generate_pdf_reading_outline",
        &input,
        api_key.as_deref(),
    )
}

/// 云端全量识别（识别闭环的正门）：返回原始 `CloudReadingOutlineV1` JSON。
///
/// 走真实 LLM 网关（`generate_pdf_reading_outline`），不做任何本地兜底：
/// 失败必须让调用方看到真实错误，由 `reconcile::engine::classify_cloud_error`
/// 归类成稳定原因码。这样「没验证」永远不会被写成「已验证」。
///
/// **PDF 与 DOCX 都必须走通这条链**（任务书第二/九项）：
/// - PDF：附原始 PDF 文件（`pdfPath`），失败回退到渲染页图；
/// - 非 PDF：不能把 DOCX 当作 `application/pdf` 附件发出去，改为附
///   `DocumentIRV2` 抽出的原文文本（`sourceText`）。
pub(crate) fn generate_cloud_reading_outline(
    root: &Path,
    job_id: &str,
    profile_id: Option<&str>,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let selected = profile_id
        .map(str::to_string)
        .or_else(|| job.active_llm_profile_id.clone())
        .ok_or_else(|| "NO_PROFILE".to_string())?;
    let profile = find_profile(root, &selected)?;
    let (source, upload_path) = main_source_for_cloud(root, &job)?;
    let is_pdf = source.file_type == "pdf";
    let extraction = if is_pdf {
        main_pdf_vision_extraction(root, &job)
            .map(|(extraction, _asset_dir)| extraction)
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let mut input = make_cloud_paper_generation_input(
        &profile,
        &job,
        &selected,
        &source,
        &upload_path,
        &extraction,
    );
    if !is_pdf {
        // `data_url_for_pdf` 会按 `data:application/pdf` 发送 `pdfPath`，
        // 对 DOCX 是错误声明，必须先摘掉，否则模型收到一个它无法解析的
        // 「PDF」。没有可抽取文本时如实失败，不假装识别过。
        if let Some(object) = input.as_object_mut() {
            object.remove("pdfPath");
        }
        // 证据面必须来自**原文件本身**的独立抽取，绝不能读本地识别产物
        // `document-ir.json`：本地与云端并行起飞时，本地可能还没落盘该文件，
        // 云端若依赖它就会 race/失败（见 `prepare_cloud_source_evidence`）。
        let source_text = prepare_cloud_source_evidence(root, &job)
            .ok_or_else(|| format!("cloud_source_text_unavailable:{job_id}"))?;
        input["sourceText"] = json!(source_text);
    }
    let api_key = load_llm_api_key(root, &selected);
    run_llm_gateway(
        root,
        job_id,
        "generate_pdf_reading_outline",
        &input,
        api_key.as_deref(),
    )
}

/// 云端**完整候选**识别的第一段：原文件证据面 + 真实网关调用。
///
/// 返回的是**模型原始 JSON**（尚未接上后端身份）。之所以只做到这一步：
/// 云端与本地并发起飞，而候选的 `batch_id` 由 `(job_id, source_sha256, base_edit_version)`
/// 派生、`base_edit_version` 要到本地冻结之后才成立。把「调用」与「定身份 + 落盘」分开，
/// 既保持并发，又不让候选挂在一个并不存在的批次上（那比没有候选更危险，因为它看着可信）。
///
/// 一次受约束修复：首次输出被结构校验拒了，把**被拒原因原样**回给模型再问一次。
/// 不带原因地重试同一句话，只会再拿到同一种错误——那不是修复，只是多烧一次配额。
pub(crate) fn generate_cloud_authoring_candidate_raw(
    root: &Path,
    job_id: &str,
    profile_id: Option<&str>,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let selected = profile_id
        .map(str::to_string)
        .or_else(|| job.active_llm_profile_id.clone())
        .ok_or_else(|| "NO_PROFILE".to_string())?;
    let profile = find_profile(root, &selected)?;
    let (source, upload_path) = main_source_for_cloud(root, &job)?;
    let is_pdf = source.file_type == "pdf";
    let extraction = if is_pdf {
        main_pdf_vision_extraction(root, &job)
            .map(|(extraction, _asset_dir)| extraction)
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let mut input = crate::llm_suggestions::make_cloud_authoring_candidate_input(
        &profile,
        &job,
        &selected,
        &source,
        &upload_path,
        &extraction,
    );
    if !is_pdf {
        // `data_url_for_pdf` 会按 `data:application/pdf` 发送 `pdfPath`，对 DOCX 是
        // 错误声明，必须先摘掉；证据面改为原文件独立抽出的文本。
        if let Some(object) = input.as_object_mut() {
            object.remove("pdfPath");
        }
        let source_text = prepare_cloud_source_evidence(root, &job)
            .ok_or_else(|| format!("cloud_source_text_unavailable:{job_id}"))?;
        input["sourceText"] = json!(source_text);
    }
    let api_key = load_llm_api_key(root, &selected);
    match run_llm_gateway(
        root,
        job_id,
        "generate_authoring_candidate",
        &input,
        api_key.as_deref(),
    ) {
        Ok(value) => Ok(value),
        Err(first_error) => {
            // 只有**结构**类拒绝才值得再问一次；网络/配置类错误重试同一句话毫无意义。
            if !first_error.starts_with("cloud_authoring_output_") {
                return Err(first_error);
            }
            if let Some(object) = input.as_object_mut() {
                object.insert("repairNote".to_string(), json!(first_error));
            }
            run_llm_gateway(
                root,
                job_id,
                "generate_authoring_candidate",
                &input,
                api_key.as_deref(),
            )
            .map_err(|second_error| format!("cloud_authoring_candidate_rejected:{second_error}"))
        }
    }
}

/// 云端**完整候选**识别的第二段：接上后端身份、重算质量、独立落盘。
///
/// 三段纪律：
/// 1. **候选不写权威稿**：只落候选 artifact，不调用 `seed_canonical_ds`，
///    不碰 `library_items_v2.canonical_ds_json`；
/// 2. **身份由后端给**：job / source / exam / audit / quality / 稳定 ID 一律后端生成；
/// 3. **质量真算**：候选先带一个显式「尚未评估」占位块，这里用同一套质量管线覆盖，
///    绝不把占位块当成结论。
pub(crate) fn finalize_cloud_authoring_candidate(
    root: &Path,
    job_id: &str,
    batch_id: &str,
    base_edit_version: i64,
    raw: &Value,
) -> CommandResult<crate::schema::cloud_repair_v1::CloudAuthoringCandidateV1> {
    use crate::library::repository::{get_canonical_ds, open_library_connection};

    let job = load_job(root, job_id)?;
    let canonical = {
        let conn = open_library_connection(root)?;
        get_canonical_ds(&conn, job_id)?.map(|(ds, _)| ds)
    };
    let source_sha256 = crate::reconcile::commands::source_sha256_for_job(root, job_id);
    let source_file_id = canonical
        .as_ref()
        .and_then(|ds| ds.pointer("/exam/sourceFiles/0/sourceFileId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| job_id.to_string());
    let exam = canonical
        .as_ref()
        .and_then(|ds| ds.get("exam"))
        .filter(|exam| exam.is_object())
        .cloned()
        .unwrap_or_else(|| {
            json!({
                "examId": job_id,
                "title": job.title,
                "language": "en",
                "tags": job.tags,
                "sourceFiles": [{"sourceFileId": source_file_id, "role": "question_paper"}]
            })
        });
    let modality = canonical
        .as_ref()
        .and_then(|ds| ds.get("modality"))
        .and_then(Value::as_str)
        .unwrap_or("reading")
        .to_string();
    let source_document_id = canonical
        .as_ref()
        .and_then(|ds| ds.get("sourceDocumentId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{job_id}-document"));
    // 抽取方式按**原文件类型**登记，而不是按模型声明：PDF 走原生文本层，DOCX 走 OOXML。
    let (main_source, _) = main_source_for_cloud(root, &job)?;
    let extraction_mode = if main_source.file_type == "pdf" {
        "pdf_native"
    } else {
        "docx_ooxml"
    };

    let generated_at = Utc::now().to_rfc3339();
    let identity = crate::reconcile::candidate::CloudAuthoringIdentity {
        job_id,
        item_id: job_id,
        batch_id,
        source_file_id: &source_file_id,
        source_sha256: &source_sha256,
        base_edit_version,
        generated_at: &generated_at,
        exam,
        modality: &modality,
        source_document_id: &source_document_id,
        extraction_mode,
    };
    let mut normalized =
        crate::reconcile::candidate::normalize_cloud_authoring(&identity, canonical.as_ref(), raw)?;
    if normalized.document.is_object() {
        // 用**同一套**质量管线评估候选，覆盖占位块。物理影子对不上时 `evaluate_quality`
        // 自己会如实标 `physicalShadow: missing`，这里不做任何粉饰。
        crate::authoring_v2_commands::refresh_quality_report(root, job_id, &mut normalized.document)?;
    }
    let candidate =
        crate::reconcile::candidate::cloud_authoring_candidate_from_normalized(&identity, normalized)?;
    crate::reconcile::store::write_cloud_authoring_candidate(root, batch_id, &candidate)?;
    Ok(candidate)
}

/// 修复回合的原文证据：PDF 走已抽取的页文本，非 PDF 走独立抽取的全文。
///
/// **不读本地识别产物**（`document-ir.json`）：它与本地识别并行，可能还没落盘；
/// 云端修复看到的必须是**原文件本身**的抽取结果。
pub(crate) fn cloud_source_evidence(root: &Path, job_id: &str) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let (source, _) = main_source_for_cloud(root, &job)?;
    if source.file_type == "pdf" {
        let extraction = main_pdf_vision_extraction(root, &job)
            .map(|(extraction, _asset_dir)| extraction)
            .unwrap_or(Value::Null);
        let pages = extraction
            .get("pages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(json!({
            "kind": "pdf",
            "sourceFileId": source.file_id,
            "originalName": source.original_name,
            "pages": pages
        }))
    } else {
        let text = prepare_cloud_source_evidence(root, &job).unwrap_or_default();
        Ok(json!({
            "kind": "text",
            "sourceFileId": source.file_id,
            "originalName": source.original_name,
            "text": text
        }))
    }
}

/// 修复回合：把「当前稿上下文 + 已发生的真实执行结果」交给真实模型，取回一个工具消息。
///
/// 证据面规则与完整候选识别一致（模型看到的必须是**原文件**）：PDF 附原文件本身，
/// 非 PDF 附独立抽取的原文文本。**不接受任意路径**——来源只由后端按 job 解析。
pub(crate) fn repair_authoring_step_through_gateway(
    root: &Path,
    job_id: &str,
    profile_id: Option<&str>,
    context: &Value,
    observations: &[Value],
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let selected = profile_id
        .map(str::to_string)
        .or_else(|| job.active_llm_profile_id.clone())
        .ok_or_else(|| "NO_PROFILE".to_string())?;
    let profile = find_profile(root, &selected)?;
    let (source, upload_path) = main_source_for_cloud(root, &job)?;
    let is_pdf = source.file_type == "pdf";
    let mut input = crate::llm_suggestions::make_repair_authoring_step_input(
        &profile,
        &job,
        &selected,
        &source,
        &upload_path,
        context,
        observations,
    );
    if !is_pdf {
        if let Some(object) = input.as_object_mut() {
            object.remove("pdfPath");
        }
        // 没有可抽取文本时如实失败，不让模型在空证据上猜。
        let source_text = prepare_cloud_source_evidence(root, &job)
            .ok_or_else(|| format!("cloud_source_text_unavailable:{job_id}"))?;
        input["sourceText"] = json!(source_text);
    }
    let api_key = load_llm_api_key(root, &selected);
    run_llm_gateway(
        root,
        job_id,
        "repair_authoring_step",
        &input,
        api_key.as_deref(),
    )
}

/// A4：把一批**待裁定的分歧项**交给真实模型，走与云端识别同一个 LLM 网关。
///
/// 证据面规则与云端识别完全一致（同一条产品要求：模型看到的必须是**原文件**）：
/// - PDF：附原文件本身，让模型自己翻页；
/// - 非 PDF：附 `DocumentIRV2` 抽出的原文文本，且必须来自**独立抽取**
///   （[`prepare_cloud_source_evidence`]），绝不读本地识别产物——并行起飞时它可能还没落盘。
///
/// 与云端识别的一个差别：**不把「没有证据面」降级成一条警告就继续**。裁决的意义就是读原文，
/// 拿不到证据面时如实失败，由裁决层归类成原因码并留给人工，绝不让模型在空证据上猜。
pub(crate) fn adjudicate_divergence_through_gateway(
    root: &Path,
    job_id: &str,
    profile_id: Option<&str>,
    divergences: &[Value],
    repair_note: Option<&str>,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let selected = profile_id
        .map(str::to_string)
        .or_else(|| job.active_llm_profile_id.clone())
        .ok_or_else(|| "NO_PROFILE".to_string())?;
    let profile = find_profile(root, &selected)?;
    let (source, upload_path) = main_source_for_cloud(root, &job)?;
    let is_pdf = source.file_type == "pdf";
    let mut input = make_adjudication_input(
        &profile,
        &job,
        &selected,
        &source,
        &upload_path,
        divergences,
        repair_note,
    );
    if !is_pdf {
        // 同云端识别：`data_url_for_pdf` 会按 `data:application/pdf` 发 `pdfPath`，
        // 对 DOCX 是错误声明，必须先摘掉。
        if let Some(object) = input.as_object_mut() {
            object.remove("pdfPath");
        }
        let source_text = prepare_cloud_source_evidence(root, &job)
            .ok_or_else(|| format!("adjudication_source_text_unavailable:{job_id}"))?;
        input["sourceText"] = json!(source_text);
    }
    let api_key = load_llm_api_key(root, &selected);
    run_llm_gateway(
        root,
        job_id,
        "adjudicate_divergence",
        &input,
        api_key.as_deref(),
    )
}

/// A3：把一批**确定性抽取判不了的槽位**交给真实模型，回原文件查值。
///
/// 证据面规则与云端识别/裁决完全一致（同一条产品要求：模型看到的必须是**原文件**）：
/// - PDF：附原文件本身，让模型自己翻页（扫描版答案页因此也能读）；
/// - 非 PDF：附 `DocumentIRV2` 抽出的原文文本，且必须来自**独立抽取**
///   （[`prepare_cloud_source_evidence`]），绝不读本地识别产物——并行起飞时它可能还没落盘。
///
/// 与云端识别的差别同裁决：**不把「没有证据面」降级成一条警告就继续**。
/// 核验的意义就是读原文，拿不到证据面时如实失败，绝不让模型在空证据上「确认」。
pub(crate) fn verify_source_answers_through_gateway(
    root: &Path,
    job_id: &str,
    profile_id: Option<&str>,
    slots: &[Value],
    repair_note: Option<&str>,
) -> CommandResult<Value> {
    let job = load_job(root, job_id)?;
    let selected = profile_id
        .map(str::to_string)
        .or_else(|| job.active_llm_profile_id.clone())
        .ok_or_else(|| "NO_PROFILE".to_string())?;
    let profile = find_profile(root, &selected)?;
    let (source, upload_path) = main_source_for_cloud(root, &job)?;
    let is_pdf = source.file_type == "pdf";
    let mut input = make_source_verification_input(
        &profile,
        &job,
        &selected,
        &source,
        &upload_path,
        slots,
        repair_note,
    );
    if !is_pdf {
        // 同云端识别：`data_url_for_pdf` 会按 `data:application/pdf` 发 `pdfPath`，
        // 对 DOCX 是错误声明，必须先摘掉。
        if let Some(object) = input.as_object_mut() {
            object.remove("pdfPath");
        }
        let source_text = prepare_cloud_source_evidence(root, &job)
            .ok_or_else(|| format!("source_verification_source_text_unavailable:{job_id}"))?;
        input["sourceText"] = json!(source_text);
    }
    let api_key = load_llm_api_key(root, &selected);
    run_llm_gateway(
        root,
        job_id,
        "verify_source_answers",
        &input,
        api_key.as_deref(),
    )
}

fn cloud_outline_report_from_output(
    profile_id: &str,
    ir: &Value,
    output: Result<Value, String>,
) -> Value {
    let mut report = json!({
        "attempted": true,
        "passed": false,
        "profileId": profile_id,
        "failure": null,
        "warningCount": 0,
        "issues": [],
        "outputConfidence": null
    });
    match output {
        Ok(output) => {
            let comparison = cloud_outline_comparison(ir, &output);
            report["passed"] = comparison
                .get("passed")
                .cloned()
                .unwrap_or(Value::Bool(false));
            report["warningCount"] = comparison
                .get("issueCount")
                .cloned()
                .unwrap_or_else(|| json!(0));
            report["issues"] = comparison
                .get("issues")
                .cloned()
                .unwrap_or_else(|| json!([]));
            report["observations"] = comparison
                .get("observations")
                .cloned()
                .unwrap_or_else(|| json!([]));
            report["comparison"] = comparison;
            report["localSummary"] = outline_group_summary_from_local(ir);
            report["cloudSummary"] = outline_group_summary_from_cloud(&output);
            report["outputConfidence"] = output.get("confidence").cloned().unwrap_or(Value::Null);
            report["outputWarnings"] = output.get("warnings").cloned().unwrap_or_else(|| json!([]));
        }
        Err(error) => {
            report["failure"] = json!(error);
            report["issues"] = json!([{
                "kind": "cloud_check_failed",
                "message": "云端对照没有完成，请人工确认题组和答案。"
            }]);
            report["warningCount"] = json!(1);
        }
    }
    report
}

pub(crate) fn run_auto_pipeline_core(
    root: &Path,
    job_id: &str,
    input: Option<AutoPipelineInput>,
) -> CommandResult<Value> {
    run_auto_pipeline_core_with_gateway(root, job_id, input, run_llm_gateway)
}

fn record_group_llm_review(
    ir: &mut Value,
    group_id: &str,
    status: &str,
    confidence: f64,
    warning: String,
    suggestion: &Value,
) {
    let Some(group) = ir
        .get_mut("groups")
        .and_then(Value::as_array_mut)
        .and_then(|groups| {
            groups
                .iter_mut()
                .find(|group| group.get("groupId").and_then(Value::as_str) == Some(group_id))
        })
    else {
        return;
    };
    let Some(obj) = group.as_object_mut() else {
        return;
    };
    let warnings = obj
        .entry("reviewWarnings".to_string())
        .or_insert_with(|| json!([]));
    if !warnings.is_array() {
        *warnings = json!([]);
    }
    if let Some(items) = warnings.as_array_mut() {
        if !items.iter().any(|item| item.as_str() == Some(&warning)) {
            items.push(json!(warning));
        }
    }
    obj.insert(
        "llmReview".to_string(),
        json!({
            "required": true,
            "status": status,
            "confidence": confidence,
            "suggestionId": suggestion.get("suggestionId").cloned().unwrap_or(Value::Null),
            "suggestedKind": suggestion.get("kind").cloned().unwrap_or(Value::Null),
            "warnings": suggestion.get("warnings").cloned().unwrap_or_else(|| json!([])),
            "evidence": suggestion.get("evidence").cloned().unwrap_or_else(|| json!({})),
            "recordedAt": Utc::now().to_rfc3339()
        }),
    );
}

struct CloudWorkerOutcome {
    vision_answer: Result<(Value, Value), String>,
    outline: Result<Value, String>,
}

fn lock_gateway<F>(shared: &Mutex<F>) -> std::sync::MutexGuard<'_, F> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Cloud conversion worker. Runs concurrently with the local rule conversion:
/// it renders the PDF page images once, then performs the two cloud
/// conversions that depend only on those images (vision answer candidates and
/// the read-only cloud outline). Results are delivered over channels; the
/// read-only comparison against the local draft happens on the main thread
/// after the local draft exists.
fn run_cloud_conversion_worker<F>(
    root: &Path,
    job: &ImportJob,
    profile_id: &str,
    shared_gateway: &Mutex<F>,
    extraction_tx: mpsc::Sender<Result<(Value, PathBuf), String>>,
    worker_tx: mpsc::Sender<CloudWorkerOutcome>,
) where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value>,
{
    // The worker owns its senders and catches its own panics: a panic inside
    // the cloud conversions must degrade into an Err outcome (local draft
    // keeps working) instead of hanging recv() or poisoning the pipeline.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let extraction = match main_pdf_vision_extraction(root, job) {
            Ok((extraction, asset_dir)) => {
                let _ = extraction_tx.send(Ok((extraction.clone(), asset_dir)));
                extraction
            }
            Err(error) => {
                let _ = extraction_tx.send(Err(error.clone()));
                return CloudWorkerOutcome {
                    vision_answer: Err(error.clone()),
                    outline: Err(error),
                };
            }
        };
        let vision_answer = {
            let mut guard = lock_gateway(shared_gateway);
            vision_answer_candidate_for_job(root, job, profile_id, &extraction, &mut *guard)
        };
        let outline = {
            let mut guard = lock_gateway(shared_gateway);
            cloud_outline_generate_with_gateway(root, job, profile_id, &extraction, &mut *guard)
        };
        CloudWorkerOutcome {
            vision_answer,
            outline,
        }
    }))
    .unwrap_or_else(|_| CloudWorkerOutcome {
        vision_answer: Err("cloud_conversion_worker_panicked".to_string()),
        outline: Err("cloud_conversion_worker_panicked".to_string()),
    });
    let _ = worker_tx.send(outcome);
}

/// Persist the structured, confirmable vision answer candidates. The raw LLM
/// output stays in vision-answer-output.json for diagnostics; this file is the
/// one users confirm against in the editor.
fn write_vision_answer_candidates_file(
    dir: &Path,
    job_id: &str,
    candidate: &Value,
) -> CommandResult<()> {
    let mut evidence_by_number = std::collections::BTreeMap::<String, Value>::new();
    for item in candidate
        .get("evidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let raw_number = item.get("questionNumber");
        let number = match raw_number {
            Some(Value::String(text)) => text.trim().trim_start_matches(['q', 'Q']).to_string(),
            Some(Value::Number(value)) => value.to_string(),
            _ => continue,
        };
        if number.is_empty() {
            continue;
        }
        evidence_by_number.insert(number, item.clone());
    }
    let confidence = candidate.get("confidence").and_then(Value::as_f64);
    // Preserve dismissal state across regeneration: a background cloud review
    // rewrites this file, and ignored candidates must stay ignored.
    let previous_dismissals: std::collections::BTreeMap<String, Value> =
        read_json_opt(&dir.join("vision-answer-candidates.json"))
            .ok()
            .flatten()
            .and_then(|previous| {
                let map = previous.get("candidates")?.as_array()?;
                Some(
                    map.iter()
                        .filter_map(|item| {
                            let dismissed = item.get("dismissedAt")?;
                            if dismissed.is_null() {
                                return None;
                            }
                            let number = item.get("questionNumber")?.as_str()?.to_string();
                            Some((number, dismissed.clone()))
                        })
                        .collect(),
                )
            })
            .unwrap_or_default();
    let mut candidates = Vec::new();
    if let Some(answers) = candidate.get("answers").and_then(Value::as_object) {
        for (number, answer) in answers {
            let number = number.trim().trim_start_matches(['q', 'Q']).to_string();
            if number.is_empty() {
                continue;
            }
            let question_id = number
                .parse::<u32>()
                .ok()
                .map(|value| format!("q{}", value));
            let mut entry = json!({
                "questionNumber": number,
                "questionId": question_id,
                "answer": answer.clone(),
                "confidence": confidence,
                "evidence": evidence_by_number.get(&number).cloned().unwrap_or(Value::Null)
            });
            if let Some(dismissed_at) = previous_dismissals.get(&number) {
                entry["dismissedAt"] = dismissed_at.clone();
            }
            candidates.push(entry);
        }
    }
    candidates.sort_by_key(|item| {
        item.get("questionNumber")
            .and_then(Value::as_str)
            .and_then(|number| number.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });
    write_json(
        &dir.join("vision-answer-candidates.json"),
        &json!({
            "schemaVersion": "VisionAnswerCandidatesV1",
            "jobId": job_id,
            "profileId": candidate.get("profileId").cloned().unwrap_or(Value::Null),
            "provider": candidate.get("provider").cloned().unwrap_or(Value::Null),
            "model": candidate.get("model").cloned().unwrap_or(Value::Null),
            "candidateCount": candidates.len(),
            "candidates": candidates
        }),
    )?;
    Ok(())
}

/// Build a blockId -> text map from the document IR so suggestion evidence
/// quotes can be verified against the real source content.
pub(crate) fn source_block_text_map(
    doc: Option<&Value>,
) -> std::collections::BTreeMap<String, String> {
    dynamic_document_blocks(doc)
        .into_iter()
        .filter_map(|block| {
            let id = block.get("blockId").and_then(Value::as_str)?.to_string();
            Some((id, dynamic_block_text(&block)))
        })
        .collect()
}

pub(crate) fn run_auto_pipeline_core_with_gateway<F>(
    root: &Path,
    job_id: &str,
    input: Option<AutoPipelineInput>,
    mut llm_gateway: F,
) -> CommandResult<Value>
where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value> + Send,
{
    let job_id = job_id.to_string();
    let options = input.unwrap_or_default();
    let parse_mode = options.parse_mode.as_deref().unwrap_or("auto");
    let confidence_threshold = options.confidence_threshold.unwrap_or(0.85).clamp(0.0, 1.0);
    let local_only = matches!(options.execution_mode.as_deref(), Some("localOnly"));
    let cloud_diagnostics_opted_in = !local_only && options.profile_id.is_some();
    let target = options.target.as_deref().unwrap_or("editableDraft");
    let allow_overwrite = options.allow_overwrite.unwrap_or(false);

    let mut job = load_job(root, &job_id)?;
    let dir = job_dir(root, &job_id);
    ensure_job_dirs(&dir)?;
    if dir.join("authoring-ir.json").exists() && !allow_overwrite {
        return Err(
            "editable_draft_exists; pass allowOverwrite=true before regenerating draft".to_string(),
        );
    }

    // Profile selection happens before the local conversion so the cloud
    // conversions can start concurrently with it. The cloud outline is a
    // read-only diagnostic: it never becomes the authoritative draft.
    let profile_id = cloud_diagnostics_opted_in
        .then(|| select_llm_profile(root, &job, options.profile_id.clone()))
        .flatten();
    let selected_profile_id = profile_id.clone();
    let shared_gateway = Mutex::new(llm_gateway);
    let cloud_worker_spawned = selected_profile_id.is_some() && main_source_is_pdf(&job);
    let (extraction_tx, extraction_rx) = mpsc::channel::<Result<(Value, PathBuf), String>>();
    let (worker_tx, worker_rx) = mpsc::channel::<CloudWorkerOutcome>();

    let pipeline_result: CommandResult<Value> = thread::scope(|scope| -> CommandResult<Value> {
        if cloud_worker_spawned {
            let worker_profile_id = selected_profile_id.clone().unwrap_or_default();
            let worker_job = job.clone();
            let gateway_ref = &shared_gateway;
            scope.spawn(move || {
                run_cloud_conversion_worker(
                    root,
                    &worker_job,
                    &worker_profile_id,
                    gateway_ref,
                    extraction_tx,
                    worker_tx,
                );
            });
        }
        let mut job = job;

        let has_doc = dir.join("document-ir.json").exists();
        if !has_doc {
            let ir = if let Some(source) = main_source_file(&job) {
                let upload_path = dir.join("uploads").join(&source.stored_name);
                if matches!(source.file_type.as_str(), "txt" | "md" | "pdf" | "docx")
                    && upload_path.exists()
                {
                    let parser_output = root
                        .join("cache")
                        .join("parser")
                        .join(format!("{}-document-ir.json", job_id));
                    parse_source_document(&job, source, &upload_path, &parser_output, parse_mode)?
                } else {
                    missing_source_document_ir(
                        &job,
                        parse_mode,
                        &format!(
                            "main source file missing or unsupported: type={}, path={}",
                            source.file_type,
                            upload_path.display()
                        ),
                    )
                }
            } else {
                missing_source_document_ir(&job, parse_mode, "no MainQuestion source file")
            };
            write_json(&dir.join("document-ir.json"), &ir)?;
            let _ = write_source_review_status(root, &job_id, Some(&ir), false, None)?;
            job = update_job(root, &job_id, |item| {
                let review = source_review_status(root, &job_id, Some(&ir))
                    .unwrap_or_else(|_| json!({"required": true, "resolved": false}));
                item.status = if review
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    JobStatus::NeedsReview
                } else {
                    JobStatus::Working
                };
                item.current_step = WorkflowStep::DocumentReview;
                item.issue_counts.needs_review = source_review_issues(&review).len() as u32;
            })?;
        }

        // Auto import is the main product path. Materialize the physical V2
        // structure here as well as in the manual parse command so authoring,
        // source overlays and quality evaluation receive the same glyph/line/
        // region evidence.
        if current_physical_shadow(&dir, &job).is_none() {
            if let (Some(source), Some(document)) = (
                main_source_file(&job)
                    .filter(|source| matches!(source.file_type.as_str(), "pdf" | "docx")),
                read_json_opt(&dir.join("document-ir.json"))?,
            ) {
                let upload_path = dir.join("uploads").join(&source.stored_name);
                if upload_path.exists() {
                    let shadow_path = dir.join(DOCUMENT_V2_SHADOW_ARTIFACT_FILE);
                    let error_path = dir.join(DOCUMENT_V2_SHADOW_ERROR_FILE);
                    // One physical-ingest entry for both formats: the auto import path
                    // must materialize the same DocumentIRV2 evidence as the manual parse
                    // command, otherwise DOCX jobs reach authoring/quality without a
                    // physical layer (see P4-T01 "unify the physical extraction entry").
                    let shadow_result = if source.file_type == "pdf" {
                        write_pdf_facts_shadow_with_v1(
                            &job,
                            source,
                            &upload_path,
                            &shadow_path,
                            Some(&document),
                        )
                    } else {
                        write_docx_facts_shadow_with_v1(
                            &job,
                            source,
                            &upload_path,
                            &shadow_path,
                            Some(&document),
                        )
                    };
                    match shadow_result {
                        Ok(_) => {
                            let _ = fs::remove_file(error_path);
                        }
                        Err(error) => {
                            write_json(
                                &error_path,
                                &json!({
                                    "schemaVersion": "DocumentIRV2StructureErrorV1",
                                    "jobId": job.job_id,
                                    "sourceFileId": source.file_id,
                                    "error": error,
                                    "recordedAt": Utc::now().to_rfc3339()
                                }),
                            )?;
                        }
                    }
                }
            }
        }

        // P4-T02: the geometric question-layout graph is derived from the same
        // physical document the authoring layer consumes, so recognition stops
        // starting from V1 split candidates.
        materialize_question_layout_graph(&dir, &job);

        let mut vision_transcription = json!({
            "attempted": false,
            "applied": false,
            "profileId": profile_id,
            "warnings": [],
            "failure": null
        });
        let mut vision_answer_extraction = json!({
            "attempted": false,
            "applied": false,
            "profileId": selected_profile_id,
            "answerCount": 0,
            "warnings": [],
            "failure": null
        });

        let doc = read_json_opt(&dir.join("document-ir.json"))?;
        let physical_shadow = current_physical_shadow(&dir, &job);
        let needs_pdf_vision_transcription = doc
            .as_ref()
            .map(|current_doc| main_pdf_needs_vision_transcription(&job, current_doc))
            .unwrap_or(false);
        if needs_pdf_vision_transcription && profile_id.is_none() {
            if let Some(obj) = vision_transcription.as_object_mut() {
                obj.insert(
                    "failure".to_string(),
                    json!("no_enabled_llm_profile_available_for_pdf_vision_transcription"),
                );
            }
        }
        if let (Some(profile_id_for_vision), Some(_current_doc)) =
            (profile_id.as_deref(), doc.as_ref())
        {
            // Cloud vision is an explicitly opted-in diagnostic. It may produce independent
            // artifacts, but it never replaces the authoritative local DocumentIRV1.
            if needs_pdf_vision_transcription {
                if let Some(obj) = vision_transcription.as_object_mut() {
                    obj.insert("attempted".to_string(), json!(true));
                }
                let extraction_result = if cloud_worker_spawned {
                    extraction_rx.recv().ok()
                } else {
                    None
                };
                match extraction_result {
                    Some(Ok((extraction, asset_dir))) => {
                        let mut guard = lock_gateway(&shared_gateway);
                        match vision_transcription_with_extraction(
                            root,
                            &job,
                            profile_id_for_vision,
                            Some("auto pipeline vision transcription"),
                            &extraction,
                            &asset_dir,
                            &mut *guard,
                        ) {
                            Ok((vision_ir, vision_output)) => {
                                write_text(
                                    &dir.join("vision-transcription.txt"),
                                    vision_output
                                        .get("text")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default(),
                                )?;
                                write_json(
                                    &dir.join("vision-transcription-output.json"),
                                    &vision_output,
                                )?;
                                let vision_has_reliable_groups = has_reliable_question_groups(
                                    &job_id,
                                    &job,
                                    &vision_ir,
                                    physical_shadow.as_ref(),
                                );
                                if let Some(obj) = vision_transcription.as_object_mut() {
                                    obj.insert("applied".to_string(), json!(false));
                                    obj.insert("diagnosticOnly".to_string(), json!(true));
                                    obj.insert(
                                        "wouldPassQualityGate".to_string(),
                                        json!(vision_has_reliable_groups),
                                    );
                                    obj.insert(
                                        "confidence".to_string(),
                                        vision_output
                                            .get("confidence")
                                            .cloned()
                                            .unwrap_or(Value::Null),
                                    );
                                    obj.insert(
                                        "warnings".to_string(),
                                        vision_output
                                            .get("warnings")
                                            .cloned()
                                            .unwrap_or_else(|| json!([])),
                                    );
                                }
                                if !vision_has_reliable_groups {
                                    let warning = "vision transcription diagnostic did not produce reliable question groups; authoritative local parse was unchanged";
                                    if let Some(obj) = vision_transcription.as_object_mut() {
                                        obj.insert("failure".to_string(), json!(warning));
                                    }
                                }
                            }
                            Err(error) => {
                                if let Some(obj) = vision_transcription.as_object_mut() {
                                    obj.insert("failure".to_string(), json!(error));
                                }
                            }
                        }
                    }
                    Some(Err(error)) => {
                        if let Some(obj) = vision_transcription.as_object_mut() {
                            obj.insert("failure".to_string(), json!(error));
                        }
                    }
                    None => {
                        if let Some(obj) = vision_transcription.as_object_mut() {
                            obj.insert(
                                "failure".to_string(),
                                json!(
                                    "cloud_conversion_worker_unavailable_for_vision_transcription"
                                ),
                            );
                        }
                    }
                }
            }
        }

        if cloud_worker_spawned {
            if let Some(obj) = vision_answer_extraction.as_object_mut() {
                obj.insert("attempted".to_string(), json!(true));
            }
        }

        let source_review = source_review_status(root, &job_id, doc.as_ref())?;
        let parser_warnings = source_review
            .get("parserWarnings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let low_confidence_blocks = source_review
            .get("lowConfidenceBlocks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let mut split = make_dynamic_split_candidates(&job_id, &job, doc.as_ref());
        let answer_candidates = parse_answer_source_candidates(root, &job, parse_mode)?;
        merge_answer_source_candidates(&mut split, answer_candidates);
        write_json(&dir.join("split-candidates.json"), &split)?;
        job = update_job(root, &job_id, |item| {
            item.status = JobStatus::Working;
            item.current_step = WorkflowStep::Split;
        })?;

        let mut ir = make_dynamic_authoring_ir(&job, &split, doc.as_ref());
        write_json(&dir.join("authoring-ir.json"), &ir)?;
        job = update_job(root, &job_id, |item| {
            item.status = JobStatus::DraftSaved;
            item.current_step = WorkflowStep::Authoring;
        })?;

        let mut low_confidence_groups = Vec::<String>::new();
        let mut blocked_auto_apply_groups = Vec::<String>::new();
        let mut high_confidence_applied_groups = Vec::<String>::new();
        let mut llm_failures = Vec::<String>::new();
        let mut suggestion_count = 0u32;
        let mut applied_count = 0u32;

        let should_run_group_repair = !local_only && !main_source_is_pdf(&job);
        let source_block_texts = source_block_text_map(doc.as_ref());
        if should_run_group_repair {
            if let Some(profile_id) = selected_profile_id.clone() {
                let profile = find_profile(root, &profile_id)?;
                if let Some(groups) = ir.get("groups").and_then(Value::as_array).cloned() {
                    for group in groups {
                        let group_id = group
                            .get("groupId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if group_id.is_empty() {
                            continue;
                        }
                        let api_key = load_llm_api_key(root, &profile_id);
                        let llm_input =
                            make_llm_input(&profile, &job, &group, &profile_id, "extract_group");
                        let output = {
                            let mut guard = lock_gateway(&shared_gateway);
                            (&mut *guard)(
                                root,
                                &job_id,
                                "extract_group",
                                &llm_input,
                                api_key.as_deref(),
                            )
                        }
                        .unwrap_or_else(|error| {
                            llm_failures.push(format!("{}:{}", group_id, error));
                            deterministic_llm_output(
                                &group,
                                "extract_group",
                                format!("llm gateway fallback: {}", error),
                            )
                        });
                        let confidence = output
                            .get("confidence")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0);
                        suggestion_count += 1;

                        let suggestion = json!({
                            "suggestionId": format!("suggestion-{}", Uuid::new_v4().simple()),
                            "jobId": job_id,
                            "groupId": group_id,
                            "profileId": profile_id,
                            "kind": output.get("kind").cloned().unwrap_or_else(|| json!(group.get("kind").and_then(Value::as_str).unwrap_or("short_answer"))),
                            "confidence": confidence,
                            "patch": output.get("patch").cloned().unwrap_or_else(|| json!([])),
                            "questions": output.get("questions").cloned().unwrap_or_else(|| json!([])),
                            "warnings": output.get("warnings").cloned().unwrap_or_else(|| json!([])),
                            "evidence": output.get("evidence").cloned().unwrap_or_else(|| json!({})),
                            "createdAt": Utc::now().to_rfc3339()
                        });
                        let _ = save_llm_suggestion(root, &job_id, &suggestion);

                        if confidence >= confidence_threshold {
                            let selected = vec![
                                "kind".to_string(),
                                "layout".to_string(),
                                "questions".to_string(),
                            ];
                            let mut auto_apply_issues =
                                llm_suggestion_auto_apply_issues(&ir, &suggestion, &selected);
                            auto_apply_issues.extend(llm_suggestion_quote_mismatches(
                                &suggestion,
                                &source_block_texts,
                            ));
                            if auto_apply_issues.is_empty()
                                && apply_suggestion_to_authoring(&mut ir, &suggestion, &selected)
                                    .is_ok()
                            {
                                if let Some(groups) =
                                    ir.get_mut("groups").and_then(Value::as_array_mut)
                                {
                                    if let Some(group) = groups.iter_mut().find(|group| {
                                        group.get("groupId").and_then(Value::as_str)
                                            == Some(group_id.as_str())
                                    }) {
                                        if let Some(obj) = group.as_object_mut() {
                                            obj.insert("autoApplied".to_string(), json!(true));
                                            obj.insert(
                                                "lastAutoAppliedSuggestionId".to_string(),
                                                suggestion
                                                    .get("suggestionId")
                                                    .cloned()
                                                    .unwrap_or(Value::Null),
                                            );
                                        }
                                    }
                                }
                                applied_count += 1;
                                high_confidence_applied_groups.push(
                                    suggestion
                                        .get("groupId")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string(),
                                );
                            } else {
                                blocked_auto_apply_groups.push(group_id.clone());
                                let warning = format!(
                                    "{}:auto_apply_blocked:{}",
                                    group_id,
                                    auto_apply_issues.join(",")
                                );
                                record_group_llm_review(
                                    &mut ir,
                                    &group_id,
                                    "auto_apply_blocked",
                                    confidence,
                                    format!("识别结果需要确认：{}", auto_apply_issues.join(",")),
                                    &suggestion,
                                );
                                llm_failures.push(warning);
                            }
                        } else {
                            let group_id = suggestion
                                .get("groupId")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            record_group_llm_review(
                                &mut ir,
                                &group_id,
                                "low_confidence",
                                confidence,
                                format!(
                                    "识别结果置信度 {:.2} 低于自动采用阈值 {:.2}，请确认该题组。",
                                    confidence, confidence_threshold
                                ),
                                &suggestion,
                            );
                            low_confidence_groups.push(group_id);
                        }
                    }
                }
            } else {
                llm_failures.push("no_enabled_llm_profile_available".to_string());
            }
        }

        if let Some(obj) = ir.as_object_mut() {
            obj.insert(
                "answerKey".to_string(),
                answer_key_from_authoring(&Value::Object(obj.clone())),
            );
            obj.insert(
                "questionOrder".to_string(),
                json!(question_order_from_authoring(&Value::Object(obj.clone()))),
            );
            obj.insert(
                "questionDisplayMap".to_string(),
                display_map_from_authoring(&Value::Object(obj.clone())),
            );
        }
        if let Some(issue) = build_vision_transcription_summary_issue(&vision_transcription, &ir) {
            append_authoring_audit_issue(&mut ir, issue);
        }
        // Join the cloud conversion worker here. The local draft already exists,
        // so a slow or failed cloud model can only delay the read-only comparison
        // and answer candidates, never the draft itself.
        let worker_outcome: Option<CloudWorkerOutcome> = if cloud_worker_spawned {
            Some(worker_rx.recv().unwrap_or(CloudWorkerOutcome {
                vision_answer: Err("cloud_conversion_worker_terminated".to_string()),
                outline: Err("cloud_conversion_worker_terminated".to_string()),
            }))
        } else {
            None
        };
        if let Some(outcome) = worker_outcome.as_ref() {
            match &outcome.vision_answer {
                Ok((candidate, output)) => {
                    let answer_count = candidate
                        .get("answers")
                        .and_then(Value::as_object)
                        .map(|answers| answers.len())
                        .unwrap_or(0);
                    write_json(&dir.join("vision-answer-output.json"), output)?;
                    write_vision_answer_candidates_file(&dir, &job_id, candidate)?;
                    if let Some(obj) = vision_answer_extraction.as_object_mut() {
                        obj.insert("applied".to_string(), json!(false));
                        obj.insert("diagnosticOnly".to_string(), json!(true));
                        obj.insert("answerCount".to_string(), json!(answer_count));
                        obj.insert(
                            "confidence".to_string(),
                            output.get("confidence").cloned().unwrap_or(Value::Null),
                        );
                        obj.insert(
                            "warnings".to_string(),
                            output.get("warnings").cloned().unwrap_or_else(|| json!([])),
                        );
                    }
                }
                Err(error) => {
                    if let Some(obj) = vision_answer_extraction.as_object_mut() {
                        obj.insert("failure".to_string(), json!(error));
                    }
                }
            }
        }
        if let Some(obj) = vision_answer_extraction.as_object_mut() {
            let filled = answer_question_ids_from_authoring(&ir);
            let missing = empty_answer_question_ids_from_authoring(&ir);
            obj.insert("filledQuestionIds".to_string(), json!(filled.clone()));
            obj.insert("missingQuestionIds".to_string(), json!(missing.clone()));
            if obj
                .get("attempted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                // The summary must not claim the vision model "filled" answers:
                // candidates are confirm-only, so filled/missing reflect the local
                // draft state while answerCount reflects what the model produced.
                let failure = obj.get("failure").and_then(Value::as_str).is_some();
                let answer_count = obj.get("answerCount").and_then(Value::as_u64).unwrap_or(0);
                let message = if failure {
                    "视觉答案抽取没有完成，本地答案保持不变；请人工核对图片答案页或使用手工转录。"
                } else if answer_count == 0 {
                    "视觉模型已检查 PDF 图片答案页，但没有产出可用的答案候选；空答案仍需人工补齐。"
                } else {
                    "视觉模型产出了答案候选，尚未写入题稿；请在题稿编辑页逐题采用或忽略。"
                };
                append_authoring_audit_issue(
                    &mut ir,
                    json!({
                        "layer": "Parser",
                        "path": "$.parser.visionAnswerExtraction",
                        "kind": "vision_answer_extraction_summary",
                        "message": message,
                        "attempted": obj.get("attempted").cloned().unwrap_or(Value::Bool(false)),
                        "applied": obj.get("applied").cloned().unwrap_or(Value::Bool(false)),
                        "answerCount": obj.get("answerCount").cloned().unwrap_or_else(|| json!(0)),
                        "filledQuestionIds": filled,
                        "missingQuestionIds": missing,
                        "confidence": obj.get("confidence").cloned().unwrap_or(Value::Null),
                        "failure": obj.get("failure").cloned().unwrap_or(Value::Null)
                    }),
                );
            }
        }
        let remaining_authoring_review = refresh_authoring_review_state(&mut ir);
        if let Some(audit) = ir.get_mut("audit").and_then(Value::as_object_mut) {
            audit.insert("llmUsed".to_string(), json!(suggestion_count > 0));
            audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
            audit.insert(
                "revision".to_string(),
                json!(audit.get("revision").and_then(Value::as_u64).unwrap_or(0) + 1),
            );
        }

        let mut cloud_comparison = json!({
            "attempted": false,
            "passed": false,
            "profileId": selected_profile_id,
            "failure": null,
            "warningCount": 0,
            "issues": []
        });
        if let Some(outcome) = worker_outcome.as_ref() {
            // The cloud outline was generated concurrently with the local rule
            // conversion; only the read-only comparison needed the local draft.
            cloud_comparison = cloud_outline_report_from_output(
                selected_profile_id.as_deref().unwrap_or_default(),
                &ir,
                outcome.outline.clone(),
            );
        }
        let cloud_warning_count = cloud_comparison
            .get("warningCount")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| {
                if cloud_comparison
                    .get("failure")
                    .and_then(Value::as_str)
                    .is_some()
                {
                    1
                } else {
                    0
                }
            }) as u32;
        let cloud_needs_confirmation = cloud_comparison
            .get("attempted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && (!cloud_comparison
                .get("passed")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || cloud_warning_count > 0);
        let cloud_attempted = cloud_comparison
            .get("attempted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if cloud_attempted {
            let local_summary = cloud_comparison
                .get("localSummary")
                .cloned()
                .unwrap_or_else(|| outline_group_summary_from_local(&ir));
            let cloud_passed = cloud_comparison
                .get("passed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            append_authoring_audit_issue(
                &mut ir,
                json!({
                    "layer": "QualityCheck",
                    "path": "$.audit.cloudComparison",
                    "kind": "cloud_comparison_summary",
                    "status": if cloud_needs_confirmation { "needs_review" } else { "passed" },
                    "message": if cloud_needs_confirmation {
                        "云端对照没有通过，请确认题组、填空位置和答案；本地题稿未被云端结果覆盖。"
                    } else {
                        "云端对照通过：题组、题型、填答布局和答案未发现显著差异。"
                    },
                    "attempted": cloud_comparison.get("attempted").cloned().unwrap_or(Value::Bool(false)),
                    "passed": cloud_passed,
                    "warningCount": cloud_warning_count,
                    "failure": cloud_comparison.get("failure").cloned().unwrap_or(Value::Null),
                    "issues": cloud_comparison.get("issues").cloned().unwrap_or_else(|| json!([])),
                    "observations": cloud_comparison.get("observations").cloned().unwrap_or_else(|| json!([])),
                    "localSummary": local_summary,
                    "cloudSummary": cloud_comparison.get("cloudSummary").cloned().unwrap_or_else(|| json!([]))
                }),
            );
        }
        write_json(&dir.join("authoring-ir.json"), &ir)?;
        write_pipeline_authoring_v2_shadow(
            &dir,
            &job,
            &ir,
            &split,
            doc.as_ref(),
            physical_shadow.as_ref(),
        )?;

        let report = validate_for_runtime_gate(root, &job_id, &ir, false)?;
        let report_passed = report
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let runtime_mode = report
            .get("runtime")
            .and_then(|runtime| runtime.get("mode"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let static_runtime_passed = report_passed && runtime_mode == "static-rust";

        let v2_quality_gate = if quality_gate_v2_enabled() {
            match build_authoring_v2_shadow(&job, &ir, &split, doc.as_ref(), physical_shadow.as_ref()) {
            Ok(authoring_v2) => authoring_v2.get("quality").cloned().unwrap_or_else(
                || json!({"state":"blocked","issues":[],"hardFailures":["QUALITY_REPORT_MISSING"]}),
            ),
            Err(error) => json!({
                "state": "blocked",
                "issues": [],
                "hardFailures": ["QUALITY_GATE_EVALUATION_FAILED"],
                "error": error
            }),
        }
        } else {
            json!({"state":"disabled","issues":[],"hardFailures":[]})
        };
        let v2_quality_requires_review =
            quality_gate_requires_review(quality_gate_v2_enabled(), &v2_quality_gate);
        let v2_quality_issue_count =
            quality_gate_review_count(quality_gate_v2_enabled(), &v2_quality_gate);

        let requires_parser_review = !source_review_issues(&source_review).is_empty();
        let requires_authoring_review = remaining_authoring_review > 0;

        let has_review_blocks = !low_confidence_groups.is_empty()
            || !blocked_auto_apply_groups.is_empty()
            || requires_parser_review
            || cloud_needs_confirmation
            || requires_authoring_review
            || v2_quality_requires_review;
        let next_status = if has_review_blocks {
            JobStatus::NeedsReview
        } else if target == "editableDraft" {
            JobStatus::DraftSaved
        } else if static_runtime_passed {
            JobStatus::ExportReady
        } else if report_passed {
            JobStatus::DraftSaved
        } else {
            JobStatus::NeedsReview
        };
        let next_step =
            if target == "editableDraft" || requires_parser_review || requires_authoring_review {
                WorkflowStep::Authoring
            } else if !low_confidence_groups.is_empty() || !blocked_auto_apply_groups.is_empty() {
                WorkflowStep::Authoring
            } else if static_runtime_passed {
                WorkflowStep::Export
            } else {
                WorkflowStep::Preview
            };
        let next_route = if has_review_blocks {
            "groups"
        } else {
            "preview"
        };
        let user_status = if has_review_blocks {
            "needsConfirmation"
        } else {
            "draftReady"
        };
        let user_message = if cloud_needs_confirmation {
            "题稿已生成，但云端对照提示存在不一致，请在题稿编辑页确认题组、填空位置和答案。"
        } else if requires_parser_review {
            "题稿已生成，但源文件识别结果需要你确认。"
        } else if !low_confidence_groups.is_empty() || !blocked_auto_apply_groups.is_empty() {
            "题稿已生成，请在题稿编辑页确认部分识别结果。"
        } else if requires_authoring_review {
            "题稿已生成，还有题干、答案或题型需要你确认。"
        } else if v2_quality_requires_review {
            "题稿已生成，但 V2 质量门发现结构或来源证据问题，需要审核。"
        } else {
            "题稿已生成，可以开始检查和编辑。"
        };

        update_job(root, &job_id, |item| {
            item.status = next_status.clone();
            item.current_step = next_step.clone();
            item.issue_counts.needs_review = low_confidence_groups.len() as u32
                + blocked_auto_apply_groups.len() as u32
                + source_review_issues(&source_review).len() as u32
                + cloud_warning_count
                + remaining_authoring_review
                + v2_quality_issue_count;
            let issues = report
                .get("issues")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            item.issue_counts.errors = issues
                .iter()
                .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("error"))
                .count() as u32;
            item.issue_counts.warnings = issues
                .iter()
                .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("warning"))
                .count() as u32;
        })?;

        let pipeline_report = json!({
            "jobId": job_id,
            "confidenceThreshold": confidence_threshold,
            "llm": {
                "profileId": selected_profile_id,
                "suggestionCount": suggestion_count,
                "appliedCount": applied_count,
                "highConfidenceAppliedGroups": high_confidence_applied_groups,
                "lowConfidenceGroups": low_confidence_groups,
                "blockedAutoApplyGroups": blocked_auto_apply_groups,
                "failures": llm_failures
            },
            "validationPassed": report_passed,
            "staticRuntimePassed": static_runtime_passed,
            "realRuntimeDiagnosticPassed": false,
            "runtimeMode": runtime_mode,
            "parser": {
                "warnings": parser_warnings,
                "lowConfidenceBlocks": low_confidence_blocks,
                "visionTranscription": vision_transcription,
                "visionAnswerExtraction": vision_answer_extraction
            },
            "quality": {
                "cloudComparison": cloud_comparison,
                "v2GateEnabled": quality_gate_v2_enabled(),
                "v2Gate": v2_quality_gate
            },
            "authoring": {
                "remainingReviewItems": remaining_authoring_review
            },
            "status": format!("{:?}", next_status),
            "currentStep": format!("{:?}", next_step),
            "userStatus": user_status,
            "userMessage": user_message,
            "nextRoute": next_route,
            "generatedAt": Utc::now().to_rfc3339(),
            "validationReport": report
        });
        write_json(&dir.join("pipeline-report.json"), &pipeline_report)?;
        let _ = minimize_process_artifacts_after_authoring(root, &job_id, "run_auto_pipeline")?;
        Ok(pipeline_report)
    });
    pipeline_result
}

fn cloud_warning_count(cloud_comparison: &Value) -> u32 {
    cloud_comparison
        .get("warningCount")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            if cloud_comparison
                .get("failure")
                .and_then(Value::as_str)
                .is_some()
            {
                1
            } else {
                0
            }
        }) as u32
}

pub(crate) fn run_cloud_review_core(
    root: &Path,
    job_id: &str,
    input: Option<RunCloudReviewInput>,
) -> CommandResult<Value> {
    run_cloud_review_core_with_gateway(root, job_id, input, &mut run_llm_gateway)
}

pub(crate) fn run_cloud_review_core_with_gateway<F>(
    root: &Path,
    job_id: &str,
    input: Option<RunCloudReviewInput>,
    mut llm_gateway: F,
) -> CommandResult<Value>
where
    F: FnMut(&Path, &str, &str, &Value, Option<&str>) -> CommandResult<Value>,
{
    let options = input.unwrap_or_default();
    let mut job = load_job(root, job_id)?;
    let dir = job_dir(root, job_id);
    ensure_job_dirs(&dir)?;

    let mut ir = read_json_opt(&dir.join("authoring-ir.json"))?
        .ok_or_else(|| "authoring_ir_missing_for_cloud_review".to_string())?;
    let selected_profile_id = select_llm_profile(root, &job, options.profile_id.clone());
    let mut cloud_comparison = json!({
        "attempted": false,
        "passed": false,
        "profileId": selected_profile_id,
        "failure": null,
        "warningCount": 0,
        "issues": []
    });

    if let Some(profile_id_for_cloud) = selected_profile_id.as_deref() {
        if main_source_is_pdf(&job) {
            match main_pdf_vision_extraction(root, &job) {
                Ok((extraction, _asset_dir)) => {
                    cloud_comparison = cloud_outline_check_for_job(
                        root,
                        &job,
                        profile_id_for_cloud,
                        &extraction,
                        &ir,
                        &mut llm_gateway,
                    );
                    // Refresh the confirmable vision answer candidates in the
                    // background as well, so a localOnly import of a scanned
                    // PDF still lands candidates for manual confirmation.
                    match vision_answer_candidate_for_job(
                        root,
                        &job,
                        profile_id_for_cloud,
                        &extraction,
                        &mut llm_gateway,
                    ) {
                        Ok((candidate, output)) => {
                            let answer_count = candidate
                                .get("answers")
                                .and_then(Value::as_object)
                                .map(|answers| answers.len())
                                .unwrap_or(0);
                            let _ = write_json(&dir.join("vision-answer-output.json"), &output);
                            let candidates_written =
                                write_vision_answer_candidates_file(&dir, job_id, &candidate);
                            let filled = answer_question_ids_from_authoring(&ir);
                            let missing = empty_answer_question_ids_from_authoring(&ir);
                            append_authoring_audit_issue(
                                &mut ir,
                                json!({
                                    "layer": "Parser",
                                    "path": "$.parser.visionAnswerExtraction",
                                    "kind": "vision_answer_extraction_summary",
                                    "message": if candidates_written.is_err() {
                                        "视觉答案候选落盘失败；请人工核对图片答案页。"
                                    } else if answer_count == 0 {
                                        "视觉模型已检查 PDF 图片答案页，但没有产出可用的答案候选；空答案仍需人工补齐。"
                                    } else {
                                        "视觉模型产出了答案候选，尚未写入题稿；请在题稿编辑页逐题采用或忽略。"
                                    },
                                    "attempted": true,
                                    "applied": false,
                                    "diagnosticOnly": true,
                                    "answerCount": answer_count,
                                    "filledQuestionIds": filled,
                                    "missingQuestionIds": missing,
                                    "confidence": output.get("confidence").cloned().unwrap_or(Value::Null),
                                    "warnings": output.get("warnings").cloned().unwrap_or_else(|| json!([])),
                                    "failure": Value::Null
                                }),
                            );
                        }
                        Err(error) => {
                            append_authoring_audit_issue(
                                &mut ir,
                                json!({
                                    "layer": "Parser",
                                    "path": "$.parser.visionAnswerExtraction",
                                    "kind": "vision_answer_extraction_summary",
                                    "message": "视觉答案抽取没有完成，本地答案保持不变；请人工核对图片答案页或使用手工转录。",
                                    "attempted": true,
                                    "applied": false,
                                    "diagnosticOnly": true,
                                    "answerCount": 0,
                                    "failure": error
                                }),
                            );
                        }
                    }
                }
                Err(error) => {
                    cloud_comparison["attempted"] = json!(true);
                    cloud_comparison["failure"] = json!(error);
                }
            }
        }
    }

    let cloud_warning_count = cloud_warning_count(&cloud_comparison);
    let cloud_needs_confirmation = cloud_comparison
        .get("attempted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && (!cloud_comparison
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || cloud_warning_count > 0);
    if cloud_comparison
        .get("attempted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let local_summary = cloud_comparison
            .get("localSummary")
            .cloned()
            .unwrap_or_else(|| outline_group_summary_from_local(&ir));
        let cloud_passed = cloud_comparison
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Merge-on-write: the LLM calls above can take tens of seconds, and
        // the user may have saved edits in the meantime. Re-read the draft and
        // replay the review issues onto the fresh content so the background
        // review never clobbers user answers. append_authoring_audit_issue
        // replaces same-kind issues, so the replay is idempotent.
        match read_json_opt(&dir.join("authoring-ir.json"))? {
            Some(mut fresh_ir) => {
                let review_issue = {
                    let mut issue_ir = ir.clone();
                    append_authoring_audit_issue(
                        &mut issue_ir,
                        json!({
                            "layer": "QualityCheck",
                            "path": "$.audit.cloudComparison",
                            "kind": "cloud_comparison_summary",
                            "status": if cloud_needs_confirmation { "needs_review" } else { "passed" },
                            "message": if cloud_needs_confirmation {
                                "云端对照没有通过，请确认题组、填空位置和答案；本地题稿未被云端结果覆盖。"
                            } else {
                                "云端对照通过：题组、题型、填答布局和答案未发现显著差异。"
                            },
                            "attempted": cloud_comparison.get("attempted").cloned().unwrap_or(Value::Bool(false)),
                            "passed": cloud_passed,
                            "warningCount": cloud_warning_count,
                            "failure": cloud_comparison.get("failure").cloned().unwrap_or(Value::Null),
                            "issues": cloud_comparison.get("issues").cloned().unwrap_or_else(|| json!([])),
                            "observations": cloud_comparison.get("observations").cloned().unwrap_or_else(|| json!([])),
                            "localSummary": local_summary,
                            "cloudSummary": cloud_comparison.get("cloudSummary").cloned().unwrap_or_else(|| json!([]))
                        }),
                    );
                    issue_ir
                        .get("audit")
                        .and_then(|audit| audit.get("issues"))
                        .and_then(Value::as_array)
                        .and_then(|issues| {
                            issues
                                .iter()
                                .find(|issue| {
                                    issue.get("kind").and_then(Value::as_str)
                                        == Some("cloud_comparison_summary")
                                })
                                .cloned()
                        })
                        .unwrap_or(Value::Null)
                };
                append_authoring_audit_issue(&mut fresh_ir, review_issue);
                if let Some(audit) = fresh_ir.get_mut("audit").and_then(Value::as_object_mut) {
                    audit.insert("updatedAt".to_string(), json!(Utc::now().to_rfc3339()));
                }
                ir = fresh_ir;
            }
            None => {
                append_authoring_audit_issue(
                    &mut ir,
                    json!({
                        "layer": "QualityCheck",
                        "path": "$.audit.cloudComparison",
                        "kind": "cloud_comparison_summary",
                        "status": if cloud_needs_confirmation { "needs_review" } else { "passed" },
                        "message": if cloud_needs_confirmation {
                            "云端对照没有通过，请确认题组、填空位置和答案；本地题稿未被云端结果覆盖。"
                        } else {
                            "云端对照通过：题组、题型、填答布局和答案未发现显著差异。"
                        },
                        "attempted": cloud_comparison.get("attempted").cloned().unwrap_or(Value::Bool(false)),
                        "passed": cloud_passed,
                        "warningCount": cloud_warning_count,
                        "failure": cloud_comparison.get("failure").cloned().unwrap_or(Value::Null),
                        "issues": cloud_comparison.get("issues").cloned().unwrap_or_else(|| json!([])),
                        "observations": cloud_comparison.get("observations").cloned().unwrap_or_else(|| json!([])),
                        "localSummary": local_summary,
                        "cloudSummary": cloud_comparison.get("cloudSummary").cloned().unwrap_or_else(|| json!([]))
                    }),
                );
            }
        }
        write_json(&dir.join("authoring-ir.json"), &ir)?;
    }

    let source_doc = read_json_opt(&dir.join("document-ir.json"))?;
    let physical_shadow = current_physical_shadow(&dir, &job);
    let split = make_dynamic_split_candidates(job_id, &job, source_doc.as_ref());
    let v2_quality_gate = if quality_gate_v2_enabled() {
        match build_authoring_v2_shadow(
            &job,
            &ir,
            &split,
            source_doc.as_ref(),
            physical_shadow.as_ref(),
        ) {
            Ok(authoring_v2) => authoring_v2.get("quality").cloned().unwrap_or_else(
                || json!({"state":"blocked","issues":[],"hardFailures":["QUALITY_REPORT_MISSING"]}),
            ),
            Err(error) => json!({
                "state": "blocked",
                "issues": [],
                "hardFailures": ["QUALITY_GATE_EVALUATION_FAILED"],
                "error": error
            }),
        }
    } else {
        json!({"state":"disabled","issues":[],"hardFailures":[]})
    };
    let v2_quality_requires_review =
        quality_gate_requires_review(quality_gate_v2_enabled(), &v2_quality_gate);
    let v2_quality_issue_count =
        quality_gate_review_count(quality_gate_v2_enabled(), &v2_quality_gate);
    let source_review = source_review_status(root, job_id, source_doc.as_ref())?;
    let source_review_issue_count = source_review_issues(&source_review).len() as u32;
    let parser_warnings = source_review
        .get("parserWarnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let low_confidence_blocks = source_review
        .get("lowConfidenceBlocks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    let mut review_ir = ir.clone();
    let remaining_authoring_review = refresh_authoring_review_state(&mut review_ir);
    let report = match read_json_opt(&dir.join("validation-report.json"))? {
        Some(existing) => existing,
        None => {
            let generated = validate_for_runtime_gate(root, job_id, &ir, false)?;
            write_json(&dir.join("validation-report.json"), &generated)?;
            generated
        }
    };
    let report_passed = report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let runtime_mode = report
        .get("runtime")
        .and_then(|runtime| runtime.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let static_runtime_passed = report_passed && runtime_mode == "static-rust";
    let requires_parser_review = source_review_issue_count > 0;

    let mut pipeline_report =
        read_json_opt(&dir.join("pipeline-report.json"))?.unwrap_or_else(|| {
            json!({
                "jobId": job_id,
                "confidenceThreshold": 0.85,
                "llm": {
                    "profileId": selected_profile_id,
                    "suggestionCount": 0,
                    "appliedCount": 0,
                    "highConfidenceAppliedGroups": [],
                    "lowConfidenceGroups": [],
                    "blockedAutoApplyGroups": [],
                    "failures": []
                },
                "parser": {
                    "warnings": [],
                    "lowConfidenceBlocks": [],
                    "visionTranscription": {
                        "attempted": false,
                        "applied": false,
                        "profileId": Value::Null,
                        "warnings": [],
                        "failure": Value::Null
                    },
                    "visionAnswerExtraction": {
                        "attempted": false,
                        "applied": false,
                        "profileId": Value::Null,
                        "answerCount": 0,
                        "warnings": [],
                        "failure": Value::Null
                    }
                },
                "quality": {
                    "cloudComparison": cloud_comparison
                }
            })
        });

    let low_confidence_group_count = pipeline_report
        .pointer("/llm/lowConfidenceGroups")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0) as u32;
    let blocked_auto_apply_count = pipeline_report
        .pointer("/llm/blockedAutoApplyGroups")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0) as u32;

    let has_review_blocks = low_confidence_group_count > 0
        || blocked_auto_apply_count > 0
        || requires_parser_review
        || cloud_needs_confirmation
        || remaining_authoring_review > 0
        || v2_quality_requires_review;
    let next_status = if has_review_blocks {
        JobStatus::NeedsReview
    } else {
        JobStatus::DraftSaved
    };
    let next_step = if static_runtime_passed && !has_review_blocks {
        WorkflowStep::Export
    } else {
        WorkflowStep::Authoring
    };
    let user_status = if has_review_blocks {
        "needsConfirmation"
    } else {
        "draftReady"
    };
    let user_message = if cloud_needs_confirmation {
        "题稿已生成，但云端对照提示存在不一致，请在题稿编辑页确认题组、填空位置和答案。"
    } else if requires_parser_review {
        "题稿已生成，但源文件识别结果需要你确认。"
    } else if low_confidence_group_count > 0 || blocked_auto_apply_count > 0 {
        "题稿已生成，请在题稿编辑页确认部分识别结果。"
    } else if remaining_authoring_review > 0 {
        "题稿已生成，还有题干、答案或题型需要你确认。"
    } else if v2_quality_requires_review {
        "题稿已生成，但 V2 质量门发现结构或来源证据问题，需要审核。"
    } else {
        "题稿已生成，云端复核已完成。"
    };

    update_job(root, job_id, |item| {
        item.status = next_status.clone();
        item.current_step = next_step.clone();
        item.issue_counts.needs_review = low_confidence_group_count
            + blocked_auto_apply_count
            + source_review_issue_count
            + cloud_warning_count
            + remaining_authoring_review
            + v2_quality_issue_count;
        let issues = report
            .get("issues")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        item.issue_counts.errors = issues
            .iter()
            .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("error"))
            .count() as u32;
        item.issue_counts.warnings = issues
            .iter()
            .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("warning"))
            .count() as u32;
    })?;
    job = load_job(root, job_id)?;

    if let Some(llm) = pipeline_report
        .get_mut("llm")
        .and_then(Value::as_object_mut)
    {
        llm.insert("profileId".to_string(), json!(selected_profile_id));
    } else {
        pipeline_report["llm"] = json!({
            "profileId": selected_profile_id,
            "suggestionCount": 0,
            "appliedCount": 0,
            "highConfidenceAppliedGroups": [],
            "lowConfidenceGroups": [],
            "blockedAutoApplyGroups": [],
            "failures": []
        });
    }
    if let Some(parser) = pipeline_report
        .get_mut("parser")
        .and_then(Value::as_object_mut)
    {
        parser.insert("warnings".to_string(), json!(parser_warnings));
        parser.insert(
            "lowConfidenceBlocks".to_string(),
            json!(low_confidence_blocks),
        );
    }
    if let Some(quality) = pipeline_report
        .get_mut("quality")
        .and_then(Value::as_object_mut)
    {
        quality.insert("cloudComparison".to_string(), cloud_comparison.clone());
        quality.insert(
            "v2GateEnabled".to_string(),
            json!(quality_gate_v2_enabled()),
        );
        quality.insert("v2Gate".to_string(), v2_quality_gate.clone());
    } else {
        pipeline_report["quality"] = json!({
            "cloudComparison": cloud_comparison,
            "v2GateEnabled": quality_gate_v2_enabled(),
            "v2Gate": v2_quality_gate
        });
    }
    pipeline_report["jobId"] = json!(job.job_id);
    pipeline_report["validationPassed"] = json!(report_passed);
    pipeline_report["staticRuntimePassed"] = json!(static_runtime_passed);
    pipeline_report["runtimeMode"] = json!(runtime_mode);
    pipeline_report["authoring"] = json!({ "remainingReviewItems": remaining_authoring_review });
    pipeline_report["status"] = json!(format!("{:?}", next_status));
    pipeline_report["currentStep"] = json!(format!("{:?}", next_step));
    pipeline_report["userStatus"] = json!(user_status);
    pipeline_report["userMessage"] = json!(user_message);
    pipeline_report["nextRoute"] = json!("preview");
    pipeline_report["generatedAt"] = json!(Utc::now().to_rfc3339());
    pipeline_report["validationReport"] = report.clone();
    write_json(&dir.join("pipeline-report.json"), &pipeline_report)?;
    Ok(pipeline_report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IssueCounts;
    use crate::job_store::save_job;
    use crate::util::{ensure_app_dirs, ensure_job_dirs, job_dir, write_json};
    use serde_json::{json, Value};
    use std::fs;
    use uuid::Uuid;

    fn sample_job() -> ImportJob {
        let now = Utc::now();
        ImportJob {
            job_id: "job-1".to_string(),
            title: "Fixture".to_string(),
            status: JobStatus::Working,
            category: None,
            frequency: None,
            tags: Vec::new(),
            source_files: vec![SourceFile {
                file_id: "source-1".to_string(),
                original_name: "fixture.pdf".to_string(),
                stored_name: "fixture.pdf".to_string(),
                file_type: "pdf".to_string(),
                sha256: "a".repeat(64),
                size_bytes: 123,
                role: "MainQuestion".to_string(),
                imported_at: now,
            }],
            active_llm_profile_id: None,
            created_at: now,
            updated_at: now,
            current_step: WorkflowStep::DocumentReview,
            issue_counts: IssueCounts::default(),
        }
    }

    /// 目标 3(a)：DOCX 主源经 `main_source_for_cloud` 必须被接受，绝不返回 `main_source_is_not_pdf`。
    #[test]
    fn main_source_for_cloud_accepts_docx() {
        let root = std::env::temp_dir().join(format!("pdf2test-docx-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).unwrap();
        let mut job = sample_job();
        job.job_id = "docx-job".to_string();
        job.source_files = vec![SourceFile {
            file_id: "doc-1".to_string(),
            original_name: "source.docx".to_string(),
            stored_name: "source.docx".to_string(),
            file_type: "docx".to_string(),
            sha256: "b".repeat(64),
            size_bytes: 100,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        }];
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        let upload = job_dir(&root, &job.job_id).join("uploads").join("source.docx");
        fs::write(&upload, b"PK\x03\x04 dummy docx payload").unwrap();
        let (source, _path) = main_source_for_cloud(&root, &job)
            .expect("DOCX 主源应被云端链接受（旧实现会返回 main_source_is_not_pdf）");
        assert_eq!(source.file_type, "docx");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目标 3(a) 的反向对照：DOCX 缺上传文件时，报错应是「文件缺失」而非「类型非 PDF」。
    #[test]
    fn main_source_for_cloud_rejects_missing_file_not_wrong_type() {
        let root = std::env::temp_dir().join(format!("pdf2test-docx-missing-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).unwrap();
        let mut job = sample_job();
        job.job_id = "docx-missing".to_string();
        job.source_files = vec![SourceFile {
            file_id: "doc-1".to_string(),
            original_name: "source.docx".to_string(),
            stored_name: "source.docx".to_string(),
            file_type: "docx".to_string(),
            sha256: "b".repeat(64),
            size_bytes: 100,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        }];
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        // 故意不写 uploads 文件。
        let err = main_source_for_cloud(&root, &job).expect_err("缺上传文件应报错");
        assert!(
            err.contains("main_source_file_missing_for_cloud"),
            "报错应为文件缺失，实际: {err}"
        );
        assert!(
            !err.contains("main_source_is_not_pdf"),
            "DOCX 不得被判为「非 PDF」，实际: {err}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目标 3(b) 的构件：`document_ir_source_text` 必须把 DocumentIRV2 的
    /// `pages[].lines[].text` 顺序拼成纯文本（缺失文本时回退 `pages[].spans[].text`），
    /// 且抽取不出文本时返回 None。这正是 `generate_cloud_reading_outline` 送给网关的
    /// `sourceText`（非 PDF 路径）。
    #[test]
    fn document_ir_source_text_flattens_pages_and_lines() {
        let root = std::env::temp_dir().join(format!("pdf2test-ir-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).unwrap();
        let job = sample_job();
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        write_json(
            &job_dir(&root, &job.job_id).join("document-ir.json"),
            &json!({"pages":[
                {"pageIndex":0,"lines":[{"text":"First line of page one."},{"text":"Second line."}]},
                {"pageIndex":1,"spans":[{"text":"Span text on page two."}]}
            ]}),
        )
        .unwrap();
        let text = document_ir_source_text(&root, &job.job_id).expect("文本应被抽出");
        assert!(text.contains("First line of page one."));
        assert!(text.contains("Second line."));
        assert!(text.contains("Span text on page two."));
        // 只应含原文件文本，不得混入与证据面无关的东西。
        assert!(!text.contains("pdfPath"));
        // 空 IR → 抽不出文本 → None。
        write_json(
            &job_dir(&root, &job.job_id).join("document-ir.json"),
            &json!({"pages":[]}),
        )
        .unwrap();
        assert!(document_ir_source_text(&root, &job.job_id).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 目标 3(b) 修复：云端证据面必须来自**原文件**的独立抽取，不依赖本地识别产物
    /// `document-ir.json`。本测试构造一个真实 .docx（仅含 `word/document.xml`），
    /// 故意**不写** `document-ir.json`，断言 `prepare_cloud_source_evidence` 仍能量出
    /// 原文件文本——证明 DOCX 云端输入不再等待/依赖本地语义识别落盘。
    #[test]
    fn cloud_source_evidence_independent_of_local_artifact() {
        let root = std::env::temp_dir()
            .join(format!("pdf2test-cloud-evidence-{}", Uuid::new_v4().simple()));
        ensure_app_dirs(&root).unwrap();
        let mut job = sample_job();
        job.job_id = "cloud-evidence-docx".to_string();
        job.source_files = vec![SourceFile {
            file_id: "doc-1".to_string(),
            original_name: "source.docx".to_string(),
            stored_name: "source.docx".to_string(),
            file_type: "docx".to_string(),
            sha256: "b".repeat(64),
            size_bytes: 100,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        }];
        save_job(&root, &job).unwrap();
        ensure_job_dirs(&job_dir(&root, &job.job_id)).unwrap();
        let upload = job_dir(&root, &job.job_id)
            .join("uploads")
            .join("source.docx");
        write_minimal_docx_for_evidence(
            &upload,
            "READING PASSAGE 1\nQuestions 1-2 Choose TWO letters.\nAnswers one two",
        );
        // 故意不写 document-ir.json —— 旧实现会在此处 race/失败。
        assert!(!job_dir(&root, &job.job_id).join("document-ir.json").exists());
        let evidence = prepare_cloud_source_evidence(&root, &job)
            .expect("云端证据面应从原文件独立抽取，不依赖 document-ir.json");
        assert!(
            evidence.contains("READING PASSAGE 1"),
            "应含原文件文本，实际: {evidence}"
        );
        assert!(
            evidence.contains("Answers one two"),
            "应含原文件文本，实际: {evidence}"
        );

        // 等价对照：纯文本源也走同样独立的原文件抽取路径。
        let mut txt_job = sample_job();
        txt_job.job_id = "cloud-evidence-txt".to_string();
        txt_job.source_files = vec![SourceFile {
            file_id: "txt-1".to_string(),
            original_name: "source.txt".to_string(),
            stored_name: "source.txt".to_string(),
            file_type: "txt".to_string(),
            sha256: "c".repeat(64),
            size_bytes: 20,
            role: "MainQuestion".to_string(),
            imported_at: Utc::now(),
        }];
        save_job(&root, &txt_job).unwrap();
        ensure_job_dirs(&job_dir(&root, &txt_job.job_id)).unwrap();
        let txt_upload = job_dir(&root, &txt_job.job_id)
            .join("uploads")
            .join("source.txt");
        fs::write(&txt_upload, "Plain text passage for the cloud.").unwrap();
        let txt_evidence = prepare_cloud_source_evidence(&root, &txt_job)
            .expect("纯文本源也应独立抽取");
        assert!(
            txt_evidence.contains("Plain text passage"),
            "实际: {txt_evidence}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 测试用：写出一个最小但合法（quick-xml 可解析）的 .docx，只含 `word/document.xml`。
    fn write_minimal_docx_for_evidence(path: &Path, body_text: &str) {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("[Content_Types].xml", options).unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
        zip.add_directory("word/", options).unwrap();
        zip.start_file("word/document.xml", options).unwrap();
        let document = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="urn:w"><w:body><w:p><w:r><w:t xml:space="preserve">{body_text}</w:t></w:r></w:p></w:body></w:document>"#,
            body_text = body_text
        );
        zip.write_all(document.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn v2_quality_gate_off_preserves_legacy_reliability() {
        let reliable = json!({
            "questionGroupCandidates": [{
                "questionRange": [1, 2],
                "requiresManualQuestionImport": false
            }]
        });
        let blocked_v2 = json!({
            "answerSlots": {"q1": {"questionNumber": 1}},
            "quality": {"state": "blocked"}
        });
        assert!(question_groups_are_reliable(
            false,
            &reliable,
            Some(&blocked_v2)
        ));
    }

    #[test]
    fn v2_quality_gate_on_requires_ready_report_and_slots() {
        let unreliable_legacy = json!({"questionGroupCandidates": []});
        let ready = json!({
            "answerSlots": {"q1": {"questionNumber": 1}},
            "quality": {"state": "ready"}
        });
        let blocked = json!({
            "answerSlots": {"q1": {"questionNumber": 1}},
            "quality": {"state": "blocked"}
        });
        let empty = json!({"answerSlots": {}, "quality": {"state": "ready"}});
        assert!(question_groups_are_reliable(
            true,
            &unreliable_legacy,
            Some(&ready)
        ));
        assert!(!question_groups_are_reliable(
            true,
            &unreliable_legacy,
            Some(&blocked)
        ));
        assert!(!question_groups_are_reliable(
            true,
            &unreliable_legacy,
            Some(&empty)
        ));
        assert!(!question_groups_are_reliable(
            true,
            &unreliable_legacy,
            None
        ));
    }

    #[test]
    fn v2_quality_review_count_never_hides_a_non_ready_state() {
        let blocked_without_details = json!({"state": "blocked"});
        assert_eq!(
            quality_gate_review_count(false, &blocked_without_details),
            0
        );
        assert_eq!(quality_gate_review_count(true, &blocked_without_details), 1);
        assert!(quality_gate_requires_review(true, &blocked_without_details));
        assert!(!quality_gate_requires_review(
            true,
            &json!({"state": "ready"})
        ));
    }

    #[test]
    fn physical_shadow_freshness_requires_schema_job_source_and_hash() {
        let job = sample_job();
        let valid = json!({
            "schemaVersion": "DocumentIRV2",
            "jobId": "job-1",
            "sourceFiles": [{
                "sourceFileId": "source-1",
                "sha256": "a".repeat(64)
            }]
        });
        assert!(physical_shadow_matches_source(&valid, &job));
        for pointer in [
            "/schemaVersion",
            "/jobId",
            "/sourceFiles/0/sourceFileId",
            "/sourceFiles/0/sha256",
        ] {
            let mut stale = valid.clone();
            *stale.pointer_mut(pointer).expect("fixture pointer") = json!("stale");
            assert!(!physical_shadow_matches_source(&stale, &job), "{pointer}");
        }
        assert!(!physical_shadow_matches_source(
            &json!({"schemaVersion": "DocumentIRV2", "jobId": "job-1", "sourceFiles": []}),
            &job
        ));
    }
}
