//! 云端直出主链（cloud-direct 方案 §2.1 / P1）。
//!
//! 一次云端调用把整份 PDF 转成完整 `IeltsAuthoringIRV2` 题稿（原文 + 题组 + 答案），
//! 确定性归一化（只补不拒）后用**唯一**的 Rust 编译器 `compile_reading_source_v2`
//! 校验；有结构错误时带错误清单修 1 次。产物直接作为正式稿写入（`audit.source =
//! cloud_direct`），不再走 reconcile / adoption / 修复循环 / 裁决。
//!
//! 契约起点是主线程验证过的直出 prompt（`docs/plans/cloud-direct/prompt/`，
//! 3 份真实 PDF 40/40 答案正确、V2 校验 0 问题），以 `include_str!` 在编译期纳入，
//! 不复制、不改写该目录下的文件。

use serde_json::{json, Map, Value};

use crate::job_store::load_job;
use crate::llm_gateway::run_llm_gateway;
use crate::llm_profiles::{find_profile, load_llm_api_key};
use crate::reading_source_v2::CompilerIssueV2;
use crate::schema::ielts_authoring_v2::IELTS_AUTHORING_IR_V2_SCHEMA_VERSION;
use crate::CommandResult;

/// 网关注册的直出命令名。
pub(crate) const DIRECT_COMMAND_NAME: &str = "generate_exam_direct";
/// 直出请求默认超时（profile 未显式配置时）。识别类整卷请求需要更宽的时间预算。
pub(crate) const DIRECT_DEFAULT_TIMEOUT_MS: u64 = 240_000;
/// 直出请求每次的最大输出 token（整卷题稿，输出远大于默认 16k）。
pub(crate) const DIRECT_MAX_OUTPUT_TOKENS: u64 = 32_000;

/// 主线程验证过的 system prompt（唯一契约来源，见模块文档）。
///
/// 以**产品资源**（`src-tauri/resources/cloud-direct/`）为准，`docs/plans/` 下
/// 只留副本：产品资源随包走、可被审计，docs 是方案记录（B15）。
const DIRECT_SYSTEM_PROMPT: &str = include_str!("../resources/cloud-direct/system.md");
/// 验证时使用的范例（western celebrity：heading / features / summary）。
const DIRECT_EXAMPLE: &str =
    include_str!("../resources/cloud-direct/example-headings-features-summary.json");
/// 补充范例：TFNG + 单选（逐题 prompt + 固定/自备选项）。
const DIRECT_EXAMPLE_TFNG: &str =
    include_str!("../resources/cloud-direct/example-tfng-single-choice.json");
/// 补充范例：表格填空 + 流程图填空（inline answer_slot 挂 table_cell / flow_step）。
const DIRECT_EXAMPLE_COMPLETION: &str =
    include_str!("../resources/cloud-direct/example-table-flowchart-completion.json");

/// 全部范例（含名字，供 prompt 逐份渲染）。顺序即 prompt 里的呈现顺序。
pub(crate) fn direct_examples() -> Vec<(&'static str, Value)> {
    [
        (
            "matching_headings + matching_features + summary_completion",
            DIRECT_EXAMPLE,
        ),
        (
            "true_false_not_given + single_choice",
            DIRECT_EXAMPLE_TFNG,
        ),
        (
            "table_completion + flowchart_completion",
            DIRECT_EXAMPLE_COMPLETION,
        ),
    ]
    .into_iter()
    .map(|(name, raw)| {
        (
            name,
            serde_json::from_str::<Value>(raw).unwrap_or(Value::Null),
        )
    })
    .collect()
}

/// 云端调用预算：首次 1 次 + 截断分块重试 2 次（`finish_reason=length`）+
/// 编译错误修复 1 次；分块用满时不再修复。验收标准为 ≤3 次/份。
const MAX_DIRECT_CALLS: usize = 3;
/// `finish_reason=length` 时的分块数（按页段对半切，每块仍产出完整结构）。
const TRUNCATED_CHUNK_COUNT: usize = 2;
/// 保真度提示阈值：段落词覆盖率低于它就在编辑器里标「请核对」（非阻断）。
const FIDELITY_PARAGRAPH_THRESHOLD: f64 = 0.8;

/// 直出链的一次完整结果。
#[derive(Debug)]
pub(crate) struct DirectExamOutcome {
    /// 完整的 `IeltsAuthoringIRV2` 值（已接后端身份、已归一化）。
    pub document: Value,
    /// 本次直出实际发出的云端调用次数。
    pub calls: usize,
    /// 是否发生过 `finish_reason=length` 分块重试。
    pub chunked: bool,
    /// 最终编译问题（空 = 编译通过）。
    pub compile_issues: Vec<CompilerIssueV2>,
    /// 非阻断警告（模型 warnings + 归一化说明 + 保真度说明）。
    pub warnings: Vec<String>,
    /// 保真度报告（正文与 PDF 文字层 diff）；扫描件（无文字层）为 None。
    pub fidelity: Option<Value>,
}

/// 直出链路入口：证据面 → 调用 → 归一化 → 编译（+1 次修复）→ 可用性判定。
///
/// 返回 `Err` 的一切原因（无 profile、非 PDF、无证据面、调用失败、产物完全不可用）
/// 都由调用方回退到现有本地链——直出与本地是二选一，不做合并。
pub(crate) fn run_exam_direct(
    root: &std::path::Path,
    job_id: &str,
    profile_id: Option<&str>,
) -> CommandResult<DirectExamOutcome> {
    let job = load_job(root, job_id)?;
    let selected = profile_id
        .map(str::to_string)
        .or_else(|| job.active_llm_profile_id.clone())
        .ok_or_else(|| "NO_PROFILE".to_string())?;
    let profile = find_profile(root, &selected)?;
    let api_key = load_llm_api_key(root, &selected);
    let (source, upload_path) = crate::auto_pipeline::main_source_for_cloud(root, &job)?;
    if source.file_type != "pdf" {
        // 直出链只接 PDF：DOCX 等来源的回退由调用方走本地链（本轮不做 DOCX 直出）。
        return Err(format!("cloud_direct_source_not_pdf:{}", source.file_type));
    }
    // 每页文字层：独立抽取，绝不读本地识别产物（原文件自己的文本才是证据）。
    let page_texts = pdf_page_text_layers(&upload_path);
    // 页图：复用 vision 抽取缓存（扫描页必须带图）。抽取失败**不静默降级**：
    // 记一条警告随产物进编辑器（B14），提醒人工核对扫描内容。
    let mut evidence_warnings: Vec<String> = Vec::new();
    let extraction = match crate::auto_pipeline::main_pdf_vision_extraction(root, &job) {
        Ok((extraction, _asset_dir)) => extraction,
        Err(error) => {
            evidence_warnings.push(format!(
                "cloud_direct_images_unavailable:{error}（扫描页可能没有图证据，已按文字层处理，请核对扫描内容）"
            ));
            Value::Null
        }
    };
    let image_pages = select_image_pages(&extraction, &page_texts);
    if page_texts.is_empty() && crate::parser::image_count_from_extraction(&image_pages) == 0 {
        return Err("cloud_direct_no_evidence".to_string());
    }

    let mut calls = 0usize;
    let mut chunked = false;
    let base_input = make_direct_input(
        &profile,
        &job,
        &selected,
        &source,
        &upload_path,
        &page_texts,
        &image_pages,
        None,
    );
    let mut raw = match run_llm_gateway(root, job_id, DIRECT_COMMAND_NAME, &base_input, api_key.as_deref()) {
        Ok(value) => {
            calls += 1;
            value
        }
        // 模型偶尔返回围栏/散文而不是 JSON：带一条格式纠正说明再要一次
        // （只补不拒的「重试一次」，不是更严的拒收）。
        Err(error) if is_direct_format_failure(&error) => {
            // 首次请求失败了，但它确实是一次消耗 token 的 HTTP 调用：先计入，
            // 否则调用预算会少算一次真实请求。
            calls += 1;
            let mut retry_input = base_input.clone();
            retry_input["jsonNote"] = json!(
                "Your previous reply was not a single valid JSON object. Return ONLY the JSON object described in the system prompt: no Markdown, no code fences, no commentary, starting with { and ending with }."
            );
            match run_llm_gateway(root, job_id, DIRECT_COMMAND_NAME, &retry_input, api_key.as_deref()) {
                Ok(value) => {
                    calls += 1;
                    value
                }
                Err(_) => return Err(error),
            }
        }
        Err(error) if is_output_truncated(&error) => {
            // 输出截断：按页段分块重试一次（每块仍直出完整结构，按题号拼接）。
            // 被截断的首次请求同样是一次云端调用——计入后才守得住 ≤3 次/份
            // （首次 1 + 两块 2 = 3，修复回合随之让位）。
            chunked = true;
            calls += 1;
            let merged = run_truncated_chunks(
                root,
                job_id,
                &profile,
                &job,
                &selected,
                &source,
                &upload_path,
                &page_texts,
                &image_pages,
                api_key.as_deref(),
                &mut calls,
            )?;
            merged
        }
        Err(error) => return Err(error),
    };

    let identity = DirectIdentity::new(&job, &source, root);
    let mut document = normalize_direct_output(root, &raw, &identity)?;
    let mut compile_issues = compile_document(&document);
    // 编译有错 → 把错误清单和当前 JSON 发回云端修 1 次（受调用预算约束）。
    if !compile_issues.is_empty() && calls < MAX_DIRECT_CALLS {
        let repair_input = make_direct_input(
            &profile,
            &job,
            &selected,
            &source,
            &upload_path,
            &page_texts,
            &image_pages,
            Some((&document, &compile_issues)),
        );
        match run_llm_gateway(root, job_id, DIRECT_COMMAND_NAME, &repair_input, api_key.as_deref()) {
            Ok(repaired) => {
                calls += 1;
                raw = repaired;
                match normalize_direct_output(root, &raw, &identity) {
                    Ok(next) => {
                        document = next;
                        compile_issues = compile_document(&document);
                    }
                    Err(error) => {
                        // 修复回合的产物无法归一化：保留上一份（结构可用的那一份）。
                        return Ok(DirectExamOutcome {
                            document,
                            calls,
                            chunked,
                            compile_issues,
                            warnings: vec![format!("cloud_direct_repair_unusable:{error}")],
                            fidelity: None,
                        });
                    }
                }
            }
            Err(error) => {
                // 修复调用失败：保留首次产物并如实标注（编辑器里可见）。
                let mut warnings = vec![format!("cloud_direct_repair_failed:{error}")];
                warnings.extend(document_warnings(&document));
                return Ok(DirectExamOutcome {
                    document,
                    calls,
                    chunked,
                    compile_issues,
                    warnings,
                    fidelity: None,
                });
            }
        }
    }

    let mut document = document;
    let mut warnings = document_warnings(&document);
    warnings.splice(0..0, evidence_warnings);
    if !compile_issues.is_empty() {
        warnings.push(format!(
            "cloud_direct_compile_issues:{}",
            compile_issues
                .iter()
                .map(|issue| format!("{}:{}", issue.code, issue.target_id))
                .collect::<Vec<_>>()
                .join(",")
        ));
        // 修复后仍有问题 = 用户还需要改的地方。写进 quality.issues（warning 级，
        // 不重复阻断发布——编译失败本身已由发布门禁的 COMPILE_FAILED 负责），
        // 编辑器问题列表据此逐条展示，不再只躺在诊断文件里。
        attach_compile_issues(&mut document, &compile_issues);
    }
    let fidelity = assess_passage_fidelity(&mut document, &page_texts, &mut warnings);
    Ok(DirectExamOutcome {
        document,
        calls,
        chunked,
        compile_issues,
        warnings,
        fidelity,
    })
}

/// `finish_reason=length` 判定：`openai_chat_content` 对该情形的机器码。
fn is_output_truncated(error: &str) -> bool {
    error.starts_with("llm_output_truncated:")
}

/// 可重试一次的格式类失败：模型没给合法 JSON（解析失败 / 空内容 / 非对象）。
/// 传输层错误（超时、连接、HTTP）不在此列——它们由分块/回退路径负责。
fn is_direct_format_failure(error: &str) -> bool {
    error.starts_with("llm_json_parse_failed:")
        || error.starts_with("llm_empty_content")
        || error.starts_with("exam_direct_output_not_object")
}

/// 截断分块重试：页段对半切，每块直出完整结构，再按题号拼接。
fn run_truncated_chunks(
    root: &std::path::Path,
    job_id: &str,
    profile: &Value,
    job: &crate::ImportJob,
    profile_id: &str,
    source: &crate::SourceFile,
    upload_path: &std::path::Path,
    page_texts: &[(usize, String)],
    image_pages: &Value,
    api_key: Option<&str>,
    calls: &mut usize,
) -> CommandResult<Value> {
    let total_pages = page_texts
        .iter()
        .map(|(index, _)| *index)
        .chain(
            image_pages
                .get("pages")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|page| page.get("pageIndex").and_then(Value::as_u64))
                .filter_map(|index| usize::try_from(index).ok()),
        )
        .max()
        .unwrap_or(1)
        .max(1);
    let chunk_size = total_pages.div_ceil(TRUNCATED_CHUNK_COUNT).max(1);
    let mut chunks: Vec<Value> = Vec::new();
    for start in (1..=total_pages).step_by(chunk_size) {
        if *calls >= MAX_DIRECT_CALLS {
            return Err("cloud_direct_chunk_budget_exhausted".to_string());
        }
        let end = (start + chunk_size - 1).min(total_pages);
        let chunk_texts: Vec<(usize, String)> = page_texts
            .iter()
            .filter(|(index, _)| *index >= start && *index <= end)
            .cloned()
            .collect();
        let chunk_images = restrict_image_pages(image_pages, start, end);
        let mut input = make_direct_input(
            profile,
            job,
            profile_id,
            source,
            upload_path,
            &chunk_texts,
            &chunk_images,
            None,
        );
        input["chunkNote"] = json!(format!(
            "The output was truncated, so the paper is converted in page chunks. \
This request covers pages {start}-{end} of the original PDF. Convert ONLY the \
questions whose numbers are printed on these pages; include the passage only if \
it is printed on these pages. Question numbers stay global."
        ));
        let output = run_llm_gateway(root, job_id, DIRECT_COMMAND_NAME, &input, api_key)?;
        *calls += 1;
        chunks.push(output);
    }
    Ok(merge_direct_chunks(chunks))
}

/// 拼接分块直出结果：题组 / 槽位 / 答案按题号合并，正文取内容最多的一份。
///
/// 每块都会从 `group-1` / `q1-prompt` 开始编号，原样拼接会产出重复 taskId 触发
/// `RUNTIME_TASK_ID_DUPLICATE`（再白白烧一次修复）。因此非正文块的全部 id 与引用
/// 按块加前缀重写；段前投放框（matching_headings）的主机按段落标签重定向到正文块
/// 的真实段落节点。`slotIds` / 答案键是全局题号，不重写。
fn merge_direct_chunks(chunks: Vec<Value>) -> Value {
    // 1) 选正文块（内容节点最多的一块）。
    let mut passage_source: Option<usize> = None;
    let mut best_len = 0usize;
    for (index, chunk) in chunks.iter().enumerate() {
        let len = chunk
            .pointer("/passage/content")
            .and_then(Value::as_array)
            .map(|content| content.len())
            .unwrap_or(0);
        if len > best_len {
            best_len = len;
            passage_source = Some(index);
        }
    }
    // 正文块的 段落标签 → 段落节点 id（非正文块的段前主机重定向目标）。
    let source_labels: std::collections::BTreeMap<String, String> = passage_source
        .and_then(|index| chunks.get(index))
        .and_then(|chunk| chunk.pointer("/passage/paragraphMap"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(label, node_id)| node_id.as_str().map(|id| (label, id.to_string())))
        .collect();
    // 每块的 段落节点 id → 标签（用于把段前主机翻译成正文块节点）。
    let chunk_labels: Vec<std::collections::BTreeMap<String, String>> = chunks
        .iter()
        .map(|chunk| {
            chunk
                .pointer("/passage/paragraphMap")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(label, node_id)| node_id.as_str().map(|id| (id.to_string(), label)))
                .collect()
        })
        .collect();

    let mut merged = Map::new();
    let mut task_groups: Vec<Value> = Vec::new();
    let mut answer_slots = Map::new();
    let mut answer_key = Map::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut passage_sources = 0usize;
    for (index, chunk) in chunks.into_iter().enumerate() {
        let mut chunk = chunk;
        let is_passage_source = Some(index) == passage_source;
        if !is_passage_source {
            // 2) 非正文块：id 加前缀 + 段前主机按标签重定向到正文块。
            let labels = chunk_labels[index].clone();
            let prefix = format!("chunk{index}-");
            rewrite_chunk_ids(&mut chunk, &prefix);
            redirect_passage_hosts(&mut chunk, &labels, &source_labels);
        }
        if let Some(groups) = chunk.get("taskGroups").and_then(Value::as_array) {
            task_groups.extend(groups.iter().cloned());
        }
        for (key, value) in chunk.get("answerSlots").and_then(Value::as_object).into_iter().flatten() {
            answer_slots.insert(key.clone(), value.clone());
        }
        for (key, value) in chunk.get("answerKey").and_then(Value::as_object).into_iter().flatten() {
            answer_key.insert(key.clone(), value.clone());
        }
        if let Some(items) = chunk.get("warnings").and_then(Value::as_array) {
            warnings.extend(items.iter().filter_map(Value::as_str).map(str::to_string));
        }
        if chunk
            .get("passage")
            .and_then(|passage| passage.get("content"))
            .and_then(Value::as_array)
            .is_some_and(|content| !content.is_empty())
        {
            passage_sources += 1;
        }
        if merged.get("meta").is_none() {
            if let Some(meta) = chunk.get("meta") {
                merged.insert("meta".to_string(), meta.clone());
            }
        }
        if is_passage_source {
            if let Some(passage) = chunk.get("passage") {
                merged.insert("passage".to_string(), passage.clone());
            }
        }
    }
    if passage_sources > 1 {
        warnings.push(
            "cloud_direct_chunked_passage:多个页段都返回了正文，已取节点数最多的一份，请人工核对".to_string(),
        );
    }
    merged.insert("taskGroups".to_string(), Value::Array(task_groups));
    merged.insert("answerSlots".to_string(), Value::Object(answer_slots));
    merged.insert("answerKey".to_string(), Value::Object(answer_key));
    merged.insert("warnings".to_string(), Value::Array(warnings.into_iter().map(Value::String).collect()));
    Value::Object(merged)
}

/// 收集一块直出产物里的全部 id（内容节点 + 题组/响应组/选项库/选项）。
/// 刻意不收 `answerSlots` / `answerKey` 的键与 `slotIds`：它们是全局题号。
fn collect_chunk_ids(value: &Value, ids: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_chunk_ids(item, ids);
            }
        }
        Value::Object(map) => {
            for key in ["taskId", "responseGroupId", "optionBankId", "optionId"] {
                if let Some(id) = map.get(key).and_then(Value::as_str) {
                    ids.insert(id.to_string());
                }
            }
            if map.get("type").and_then(Value::as_str).is_some() {
                if let Some(id) = map.get("id").and_then(Value::as_str) {
                    ids.insert(id.to_string());
                }
            }
            for key in [
                "children", "items", "rows", "cells", "caption", "steps", "content",
                "instructions", "stimulus", "prompt", "title", "options",
                "taskGroups", "responseGroups", "optionBank",
            ] {
                if let Some(child) = map.get(key) {
                    collect_chunk_ids(child, ids);
                }
            }
        }
        _ => {}
    }
}

/// 按 id 映射重写一块产物里的 id 字段与引用字段（hostNodeId / optionBankRef）。
fn rewrite_chunk_references(value: &mut Value, map: &std::collections::BTreeMap<String, String>) {
    match value {
        Value::Array(items) => {
            for item in items.iter_mut() {
                rewrite_chunk_references(item, map);
            }
        }
        Value::Object(fields) => {
            for key in ["taskId", "responseGroupId", "optionBankId", "optionId", "id"] {
                if key == "id" && fields.get("type").and_then(Value::as_str).is_none() {
                    continue;
                }
                if let Some(id) = fields.get(key).and_then(Value::as_str).map(str::to_string) {
                    if let Some(rewritten) = map.get(&id) {
                        fields.insert(key.to_string(), json!(rewritten));
                    }
                }
            }
            for key in ["hostNodeId", "optionBankRef"] {
                if let Some(id) = fields.get(key).and_then(Value::as_str).map(str::to_string) {
                    if let Some(rewritten) = map.get(&id) {
                        fields.insert(key.to_string(), json!(rewritten));
                    }
                }
            }
            for key in [
                "children", "items", "rows", "cells", "caption", "steps", "content",
                "instructions", "stimulus", "prompt", "title", "options",
                "taskGroups", "responseGroups", "optionBank",
            ] {
                if let Some(child) = fields.get_mut(key) {
                    rewrite_chunk_references(child, map);
                }
            }
            // answerSlots 的值里有 hostNodeId 需要重写；键（题号）不动。
            if let Some(slots) = fields.get_mut("answerSlots").and_then(Value::as_object_mut) {
                for slot in slots.values_mut() {
                    rewrite_chunk_references(slot, map);
                }
            }
        }
        _ => {}
    }
}

/// 非正文块 id 加前缀（同一模型每块都从 group-1 开始编号）。
fn rewrite_chunk_ids(chunk: &mut Value, prefix: &str) {
    let mut ids = std::collections::BTreeSet::new();
    collect_chunk_ids(chunk, &mut ids);
    let map: std::collections::BTreeMap<String, String> = ids
        .into_iter()
        .map(|id| (id.clone(), format!("{prefix}{id}")))
        .collect();
    rewrite_chunk_references(chunk, &map);
}

/// 段前投放框主机重定向：非正文块里 `hostType=passage_paragraph` 的槽位，
/// 主机按段落标签换成正文块的真实段落节点（否则指向被丢弃的正文副本）。
fn redirect_passage_hosts(
    chunk: &mut Value,
    own_labels: &std::collections::BTreeMap<String, String>,
    source_labels: &std::collections::BTreeMap<String, String>,
) {
    let Some(slots) = chunk.get_mut("answerSlots").and_then(Value::as_object_mut) else {
        return;
    };
    for slot in slots.values_mut() {
        if slot.get("hostType").and_then(Value::as_str) != Some("passage_paragraph") {
            continue;
        }
        let Some(host) = slot.get("hostNodeId").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        let Some(label) = own_labels.get(&host) else { continue };
        if let Some(target) = source_labels.get(label) {
            slot["hostNodeId"] = json!(target);
        }
    }
}

/// 页图子集：只保留 `[start, end]` 范围内的页（分块重试时用）。
fn restrict_image_pages(extraction: &Value, start: usize, end: usize) -> Value {
    let mut next = extraction.clone();
    if let Some(pages) = next.get_mut("pages").and_then(Value::as_array_mut) {
        pages.retain(|page| {
            page.get("pageIndex")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .is_some_and(|index| index >= start && index <= end)
        });
    }
    next
}

/// 每页文字层（1-based 页码 → 文本）。抽不出来返回空 Vec（调用方按扫描件处理）。
///
/// 只读**原文件自己的**文本层：这是给模型的提示与保真度对照的基准，
/// 本地识别的任何结论都不参与（证据面纪律，与云端候选链一致）。
fn pdf_page_text_layers(pdf_path: &std::path::Path) -> Vec<(usize, String)> {
    let path = pdf_path.to_path_buf();
    let pages = std::panic::catch_unwind(move || {
        pdf_extract::extract_text_by_pages(&path).ok()
    })
    .ok()
    .flatten()
    .unwrap_or_default();
    pages
        .into_iter()
        .enumerate()
        .map(|(index, text)| (index + 1, text))
        .filter(|(_, text)| !text.trim().is_empty())
        .collect()
}

/// 选择要作为**图片**发送的页：没有文字层的页（扫描页）必须带图；
/// 有文字层的页只发文字层（验证过的低成本做法，约 2 万 token/份）。
/// 文字层短于此字符数、且页上带图的页面，也按扫描页附图（B14）。
///
/// 扫描的答案页常常带一两行可抽取文字（页眉、题号），但它仍是「得看图才知道
/// 答案」的页——只看「完全没有文字层」会把这类页漏成文字页，模型看不到答案。
/// 200 字符约等于一行标题：真正的正文页都远长于此，成本可控。
const SPARSE_TEXT_PAGE_CHARS: usize = 200;

fn select_image_pages(extraction: &Value, page_texts: &[(usize, String)]) -> Value {
    let text_by_page: std::collections::BTreeMap<usize, usize> = page_texts
        .iter()
        .map(|(index, text)| (*index, text.chars().count()))
        .collect();
    let mut next = extraction.clone();
    if let Some(pages) = next.get_mut("pages").and_then(Value::as_array_mut) {
        pages.retain(|page| {
            let index = page.get("pageIndex").and_then(Value::as_u64);
            // 无文字层，或文字层短到不可能是正文（扫描答案页的零星 OCR）。
            let sparse = index
                .and_then(|index| usize::try_from(index).ok())
                .is_some_and(|index| {
                    text_by_page
                        .get(&index)
                        .is_none_or(|chars| *chars < SPARSE_TEXT_PAGE_CHARS)
                });
            sparse && crate::parser::image_count_from_extraction(&json!({"pages":[page]})) > 0
        });
    }
    next
}

/// 直出身份：模型无权填写的字段全部由这里给出（与本地管线同源的身份约定）。
///
/// `source_file_id` 目前只服务于诊断输出（调用序列与可读性），字段保留但不进人判据。
#[allow(dead_code)]
pub(crate) struct DirectIdentity {
    job_id: String,
    exam: Value,
    source_file_id: String,
    source_document_id: String,
    generated_at: String,
}

impl DirectIdentity {
    fn new(job: &crate::ImportJob, source: &crate::SourceFile, _root: &std::path::Path) -> Self {
        let category = job
            .category
            .clone()
            .unwrap_or_else(|| "P1".to_string());
        // examId 与本地管线同一约定（`ielts_grammar::exam_value`）：发布路径安全
        // 且同一 job 稳定。
        let exam_id = format!(
            "{}-{}",
            category.to_ascii_lowercase(),
            &job.job_id[job.job_id.len().saturating_sub(8)..]
        );
        Self {
            job_id: job.job_id.clone(),
            exam: json!({
                "examId": exam_id,
                "title": job.title,
                "category": category,
                "frequency": job.frequency.clone().unwrap_or_else(|| "medium".to_string()),
                "language": "en",
                "tags": job.tags,
                "sourceFiles": [{"sourceFileId": source.file_id, "role": "question_paper"}]
            }),
            source_file_id: source.file_id.clone(),
            // 直出链不跑本地解析，没有物理影子文档：sourceDocumentId 按 job 派生。
            source_document_id: format!("document-{}", job.job_id),
            generated_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// 直出请求输入。证据面 = 每页文字层 + 扫描页渲染图；契约 = system prompt +
/// taskPresentationRules + 范例。**不传** `localNodeTargets` / `sourceParagraphs`。
#[allow(clippy::too_many_arguments)]
fn make_direct_input(
    profile: &Value,
    job: &crate::ImportJob,
    profile_id: &str,
    source: &crate::SourceFile,
    pdf_path: &std::path::Path,
    page_texts: &[(usize, String)],
    image_pages: &Value,
    repair: Option<(&Value, &[CompilerIssueV2])>,
) -> Value {
    let page_texts: Vec<Value> = page_texts
        .iter()
        .map(|(index, text)| json!({"pageIndex": index, "text": text}))
        .collect();
    let mut input = json!({
        "mode": DIRECT_COMMAND_NAME,
        "modality": "reading",
        "job": {"jobId": job.job_id, "title": job.title, "category": job.category, "tags": job.tags},
        "profile": direct_profile_payload(profile, profile_id),
        "sourceFile": {
            "fileId": source.file_id,
            "originalName": source.original_name,
            "fileType": source.file_type,
            "sizeBytes": source.size_bytes
        },
        "pdfPath": pdf_path.to_string_lossy(),
        // 每页文字层。
        "pageTexts": page_texts,
        // 扫描页渲染图（`pages[].images[]`，由网关注入 data URL）。
        "pages": image_pages.get("pages").cloned().unwrap_or_else(|| json!([])),
        "extractionWarnings": image_pages.get("warnings").cloned().unwrap_or_else(|| json!([])),
        "outputContract": {
            "schema": "IeltsAuthoringIRV2-direct",
            "topLevelKeys": ["meta", "passage", "taskGroups", "answerSlots", "answerKey", "warnings"],
            "note": "Follow the system prompt exactly; it is the full output contract."
        },
        "taskPresentationRules": crate::schema::task_presentation::rules_prompt_table(),
        // 范例：主范例 + 两份补充（TFNG/单选、表格/流程图填空），覆盖题型差异面。
        "example": serde_json::from_str::<Value>(DIRECT_EXAMPLE).unwrap_or(Value::Null),
        "examples": direct_examples()
            .into_iter()
            .map(|(name, document)| json!({"name": name, "document": document}))
            .collect::<Vec<_>>(),
    });
    if let Some((document, issues)) = repair {
        // 修复回合：错误清单 + 当前 JSON 一起发回，要求只修结构、保持内容。
        input["repair"] = json!({
            "instruction": "The previous output failed the exam compiler. Fix ONLY the listed \
structural problems in the JSON below and return the COMPLETE corrected exam. Keep all content \
(passage text, questions, options, answers) unchanged unless a listed problem requires it.",
            "compilerIssues": issues.iter().map(|issue| json!({
                "code": issue.code,
                "targetId": issue.target_id,
                "message": issue.message
            })).collect::<Vec<_>>(),
            "currentOutput": document
        });
    }
    input
}

/// profile 载荷：直出的超时/输出上限比普通候选更宽（整卷题稿）。
///
/// 超时取 `max(配置值, 240s)`：`profile_payload` 总会写 `timeoutMs`（用户 60s 配置
/// 或 120s 缺省），整卷直出在 60s 下会被传输层超时打断，所以直出有自己的下限——
/// 用户配置更长时尊重用户。
fn direct_profile_payload(profile: &Value, profile_id: &str) -> Value {
    let mut payload = crate::llm_suggestions::profile_payload(profile, profile_id);
    if let Some(object) = payload.as_object_mut() {
        let configured = object.get("timeoutMs").and_then(Value::as_u64).unwrap_or(0);
        object.insert(
            "timeoutMs".to_string(),
            json!(configured.max(DIRECT_DEFAULT_TIMEOUT_MS)),
        );
        if object.get("maxOutputTokens").and_then(Value::as_u64).is_none() {
            object.insert("maxOutputTokens".to_string(), json!(DIRECT_MAX_OUTPUT_TOKENS));
        }
    }
    payload
}

/// 供网关注入使用的直出 system prompt（契约唯一来源）。
pub(crate) fn direct_system_prompt() -> &'static str {
    DIRECT_SYSTEM_PROMPT
}

/// 模型 raw warnings → 文档 audit.notes（编辑器与发布记录里可见）。
fn document_warnings(document: &Value) -> Vec<String> {
    document
        .pointer("/audit/notes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// 归一化（确定性、只补不拒）
//
// 模型只负责内容；一切可由题型/结构推导的字段在这里补齐，缺失这些字段**不**拒收。
// 身份（job/exam/audit/quality）一律后端给出，模型输出里的同名字段不采信。
// ─────────────────────────────────────────────────────────────────────────────

/// taskType 枚举别名归一枚举（模型可能用驼峰 / 连字符写法）。
fn normalize_task_type(value: &Value) -> Option<&'static str> {
    let raw = value.as_str()?.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    Some(match raw.as_str() {
        "single_choice" => "single_choice",
        "multiple_choice" => "multiple_choice",
        "true_false_not_given" => "true_false_not_given",
        "yes_no_not_given" => "yes_no_not_given",
        "matching_information" => "matching_information",
        "matching_headings" => "matching_headings",
        "matching_features" => "matching_features",
        "matching_sentence_endings" => "matching_sentence_endings",
        "classification" => "classification",
        "sentence_completion" => "sentence_completion",
        "summary_completion" => "summary_completion",
        "note_completion" => "note_completion",
        "table_completion" => "table_completion",
        "form_completion" => "form_completion",
        "flowchart_completion" => "flowchart_completion",
        "diagram_label_completion" => "diagram_label_completion",
        "plan_map_label_completion" => "plan_map_label_completion",
        "short_answer" => "short_answer",
        _ => return None,
    })
}

/// 收集文档里全部内容节点 id（题组 / 正文 / 选项内容都算）。
fn collect_node_ids(value: &Value, ids: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_node_ids(item, ids);
            }
        }
        Value::Object(map) => {
            if let Some(id) = map.get("id").and_then(Value::as_str) {
                ids.insert(id.to_string());
            }
            for key in ["children", "items", "rows", "cells", "caption", "steps", "content"] {
                if let Some(child) = map.get(key) {
                    collect_node_ids(child, ids);
                }
            }
        }
        _ => {}
    }
}

/// 给缺失 id 的内容节点补一个确定性的、全局唯一的 id。
fn fill_missing_node_ids(value: &mut Value, used: &mut std::collections::BTreeSet<String>, prefix: &str) {
    match value {
        Value::Array(items) => {
            for item in items.iter_mut() {
                fill_missing_node_ids(item, used, prefix);
            }
        }
        Value::Object(map) => {
            let is_node = map.get("type").and_then(Value::as_str).is_some();
            if is_node {
                let has_id = map
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.trim().is_empty());
                if !has_id {
                    let node_type = map
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("node")
                        .to_string();
                    let mut ordinal = 1u32;
                    let candidate = loop {
                        let candidate = format!("{prefix}-{node_type}-{ordinal}");
                        if used.insert(candidate.clone()) {
                            break candidate;
                        }
                        ordinal += 1;
                    };
                    map.insert("id".to_string(), json!(candidate));
                }
            }
            for key in ["children", "items", "rows", "cells", "caption", "steps"] {
                if let Some(child) = map.get_mut(key) {
                    fill_missing_node_ids(child, used, prefix);
                }
            }
        }
        _ => {}
    }
}

/// 丢弃引用不存在资产的图形节点（直出链不登记资产；保留即发布时悬挂引用）。
/// 返回被丢弃的节点 id（留痕为警告）。
fn drop_asset_nodes(value: &mut Value, dropped: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            items.retain_mut(|item| {
                drop_asset_nodes(item, dropped);
                let node_type = item.get("type").and_then(Value::as_str).unwrap_or("");
                if matches!(node_type, "figure" | "image" | "diagram") {
                    if let Some(id) = item.get("id").and_then(Value::as_str) {
                        dropped.push(id.to_string());
                    }
                    false
                } else {
                    true
                }
            });
        }
        Value::Object(map) => {
            for key in [
                "children", "items", "rows", "cells", "caption", "steps", "content",
                "instructions", "stimulus", "prompt", "title", "options",
            ] {
                if let Some(child) = map.get_mut(key) {
                    drop_asset_nodes(child, dropped);
                }
            }
        }
        _ => {}
    }
}

/// 找包含指定 `slotId` 的 answer_slot 节点的最近容器 id（paragraph/table_cell/flow_step）。
fn find_slot_host(value: &Value, slot_id: &str) -> Option<String> {
    fn walk(value: &Value, slot_id: &str) -> Option<String> {
        match value {
            Value::Array(items) => items.iter().find_map(|item| walk(item, slot_id)),
            Value::Object(map) => {
                let node_type = map.get("type").and_then(Value::as_str).unwrap_or("");
                let holds_slot = |children: Option<&Value>| {
                    children
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .any(|child| {
                            child.get("type").and_then(Value::as_str) == Some("answer_slot")
                                && child.get("slotId").and_then(Value::as_str) == Some(slot_id)
                        })
                };
                if matches!(node_type, "paragraph" | "table_cell" | "flow_step")
                    && holds_slot(map.get("children"))
                {
                    return map.get("id").and_then(Value::as_str).map(str::to_string);
                }
                for key in ["children", "items", "rows", "cells", "steps", "content"] {
                    if let Some(child) = map.get(key) {
                        if let Some(found) = walk(child, slot_id) {
                            return Some(found);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
    walk(value, slot_id)
}

/// 指令文本（用于 instruction signature 与题目类型推断）。
fn instruction_text(nodes: &Value) -> String {
    crate::reconcile::candidate::nodes_text(nodes)
}

/// 直出产物 → 完整 `IeltsAuthoringIRV2`。
///
/// 完全不可用（无题组 / 无正文 / 无答案位）时返回 Err，由调用方回退本地链。
pub(crate) fn normalize_direct_output(
    root: &std::path::Path,
    raw: &Value,
    identity: &DirectIdentity,
) -> CommandResult<Value> {
    let task_groups = raw
        .get("taskGroups")
        .and_then(Value::as_array)
        .filter(|groups| !groups.is_empty())
        .cloned()
        .ok_or_else(|| "cloud_direct_no_task_groups".to_string())?;
    let passage = raw
        .get("passage")
        .cloned()
        .unwrap_or(Value::Null);
    let passage_content = passage
        .get("content")
        .and_then(Value::as_array)
        .filter(|content| !content.is_empty())
        .cloned()
        .ok_or_else(|| "cloud_direct_no_passage".to_string())?;
    // 只补不拒：`answerSlots` 为空不再是回退理由——方案只允许「无题组 /
    // 无正文」两种情况回退；题组在但一个题号都没认出来，是可编辑的残稿
    // （编译器会如实报缺位，修复回合或人工继续处理）。
    let answer_slots = raw
        .get("answerSlots")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let answer_key = raw
        .get("answerKey")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let model_warnings: Vec<String> = raw
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    let meta = raw.get("meta").cloned().unwrap_or(Value::Null);

    // ── 1) 内容节点后端字段 + 缺失 id ────────────────────────────────
    let mut passage = passage;
    if let Some(object) = passage.as_object_mut() {
        crate::reconcile::candidate::ensure_source_anchors(object);
        if let Some(content) = object.get_mut("content") {
            crate::reconcile::candidate::fill_content_node_defaults(content);
        }
    }
    let mut used_ids = std::collections::BTreeSet::new();
    if let Some(content) = passage.get("content").and_then(Value::as_array) {
        collect_node_ids(&Value::Array(content.clone()), &mut used_ids);
    }
    let mut task_groups = task_groups;
    let mut dropped_asset_nodes: Vec<String> = Vec::new();
    for group in task_groups.iter_mut() {
        let Some(object) = group.as_object_mut() else {
            continue;
        };
        for key in ["instructions", "stimulus"] {
            if let Some(nodes) = object.get_mut(key) {
                crate::reconcile::candidate::fill_content_node_defaults(nodes);
                collect_node_ids(nodes, &mut used_ids);
            }
        }
        if let Some(bank) = object.get_mut("optionBank").and_then(Value::as_object_mut) {
            crate::reconcile::candidate::ensure_source_anchors(bank);
            if let Some(title) = bank.get_mut("title") {
                crate::reconcile::candidate::fill_content_node_defaults(title);
                collect_node_ids(title, &mut used_ids);
            }
            if let Some(options) = bank.get_mut("options").and_then(Value::as_array_mut) {
                for option in options.iter_mut().filter_map(Value::as_object_mut) {
                    crate::reconcile::candidate::ensure_source_anchors(option);
                    if let Some(content) = option.get_mut("content") {
                        crate::reconcile::candidate::fill_content_node_defaults(content);
                        collect_node_ids(content, &mut used_ids);
                    }
                }
            }
        }
        if let Some(responses) = object.get_mut("responseGroups").and_then(Value::as_array_mut) {
            for response in responses.iter_mut() {
                for key in ["prompt"] {
                    if let Some(nodes) = response.get_mut(key) {
                        crate::reconcile::candidate::fill_content_node_defaults(nodes);
                        collect_node_ids(nodes, &mut used_ids);
                    }
                }
                if let Some(options) = response.get_mut("options").and_then(Value::as_array_mut) {
                    for option in options.iter_mut().filter_map(Value::as_object_mut) {
                        crate::reconcile::candidate::ensure_source_anchors(option);
                        if let Some(content) = option.get_mut("content") {
                            crate::reconcile::candidate::fill_content_node_defaults(content);
                            collect_node_ids(content, &mut used_ids);
                        }
                    }
                }
            }
        }
    }
    // 先登记已有 id，再补缺 id（保证补出来的不与任何已有 id 撞车）。
    if let Some(content) = passage.get_mut("content").and_then(Value::as_array_mut) {
        let mut nodes = Value::Array(std::mem::take(content));
        fill_missing_node_ids(&mut nodes, &mut used_ids, "passage");
        *content = nodes.as_array_mut().map(std::mem::take).unwrap_or_default();
    }
    for group in task_groups.iter_mut() {
        let task_id_hint = group
            .get("taskId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| "group".to_string());
        let Some(object) = group.as_object_mut() else {
            continue;
        };
        for key in ["instructions", "stimulus"] {
            if let Some(nodes) = object.get_mut(key) {
                fill_missing_node_ids(nodes, &mut used_ids, &task_id_hint);
            }
        }
        if let Some(bank) = object.get_mut("optionBank") {
            if let Some(options) = bank.get_mut("options").and_then(Value::as_array_mut) {
                for option in options.iter_mut() {
                    if let Some(content) = option.get_mut("content") {
                        fill_missing_node_ids(content, &mut used_ids, &task_id_hint);
                    }
                }
            }
        }
        if let Some(responses) = object.get_mut("responseGroups").and_then(Value::as_array_mut) {
            for response in responses.iter_mut() {
                if let Some(nodes) = response.get_mut("prompt") {
                    let mut nodes_value = std::mem::replace(nodes, Value::Array(Vec::new()));
                    fill_missing_node_ids(&mut nodes_value, &mut used_ids, &task_id_hint);
                    *nodes = nodes_value;
                }
                if let Some(options) = response.get_mut("options").and_then(Value::as_array_mut) {
                    for option in options.iter_mut() {
                        if let Some(content) = option.get_mut("content") {
                            let mut content_value = std::mem::replace(content, Value::Array(Vec::new()));
                            fill_missing_node_ids(&mut content_value, &mut used_ids, &task_id_hint);
                            *content = content_value;
                        }
                    }
                }
            }
        }
    }
    for group in task_groups.iter_mut() {
        drop_asset_nodes(group, &mut dropped_asset_nodes);
    }
    drop_asset_nodes(&mut passage, &mut dropped_asset_nodes);
    // ── 2) paragraphMap：只保留指向真实段落节点的条目 ─────────────────
    let paragraph_ids: std::collections::BTreeSet<String> = passage_content
        .iter()
        .filter(|node| node.get("type").and_then(Value::as_str) == Some("paragraph"))
        .filter_map(|node| node.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut paragraph_map = Map::new();
    if let Some(map) = passage.get("paragraphMap").and_then(Value::as_object) {
        for (label, node_id) in map {
            let Some(node_id) = node_id.as_str() else { continue };
            if paragraph_ids.contains(node_id) {
                paragraph_map.insert(label.clone(), json!(node_id));
            }
        }
    }
    // 段落节点的 paragraphLabel 与 paragraphMap 对齐（编辑器按字母渲染段前投放框）。
    if let Some(content) = passage.get_mut("content").and_then(Value::as_array_mut) {
        let label_of: std::collections::BTreeMap<String, String> = paragraph_map
            .iter()
            .map(|(label, node_id)| (node_id.as_str().unwrap_or_default().to_string(), label.clone()))
            .collect();
        for node in content.iter_mut() {
            if node.get("type").and_then(Value::as_str) != Some("paragraph") {
                continue;
            }
            let Some(id) = node.get("id").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            if let Some(label) = label_of.get(&id) {
                if let Some(object) = node.as_object_mut() {
                    object.insert("paragraphLabel".to_string(), json!(label));
                }
            }
        }
    }
    let paragraph_map = Value::Object(paragraph_map);
    let passage = Value::Object(
        passage
            .as_object()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .chain(std::iter::once(("paragraphMap".to_string(), paragraph_map.clone())))
            .collect::<Map<String, Value>>(),
    );
    let _ = passage_content;

    // ── 3) 题组：题型规则推导响应组 / 选项 / 签名 / 质量 ────────────────
    let mut normalized_groups: Vec<Value> = Vec::with_capacity(task_groups.len());
    for (index, group) in task_groups.into_iter().enumerate() {
        let task_type = group
            .get("taskType")
            .and_then(normalize_task_type)
            .unwrap_or("short_answer")
            .to_string();
        let task_id = group
            .get("taskId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("group-{}", index + 1));
        let rule = crate::schema::task_presentation::rule_for(
            &parse_task_type(&task_type),
            group
                .get("optionBank")
                .and_then(|bank| bank.get("options"))
                .and_then(Value::as_array)
                .is_some_and(|options| !options.is_empty()),
        );
        // displayRange：模型给的就用；缺失时从槽位题号推导。
        let slot_numbers: Vec<u32> = group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|response| response.get("slotIds").and_then(Value::as_array).into_iter().flatten())
            .filter_map(Value::as_str)
            .filter_map(|slot_id| {
                answer_slots
                    .get(slot_id)
                    .and_then(|slot| slot.get("questionNumber"))
                    .and_then(Value::as_u64)
                    .and_then(|number| u32::try_from(number).ok())
            })
            .collect::<std::collections::BTreeSet<u32>>()
            .into_iter()
            .collect();
        let display_range = group
            .get("displayRange")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| range_from_numbers(&slot_numbers));
        // 指令与 stimulus。
        let instructions = group
            .get("instructions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let instruction_text_value = instruction_text(&Value::Array(instructions.clone()));
        // 响应组：补题型可推导的字段；TFNG/YNNG 补固定选项。
        let mut responses: Vec<Value> = Vec::new();
        let raw_responses = group
            .get("responseGroups")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (position, response) in raw_responses.into_iter().enumerate() {
            let mut response = response;
            let slot_ids: Vec<String> = response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            let response_kind = crate::schema::task_presentation::wire_name(&rule.response_kind);
            let assignment = crate::schema::task_presentation::wire_name(&rule.assignment);
            // choose TWO/THREE（unordered_set）是集合题：整组一次选 N 个，
            // 记分与基数都和逐题单选不同。
            //
            // 集合题的后端字段以**编译器规则**为准：`unordered_set` 要求
            // scoringPolicy = `per_slot_ielts_normalized`（reading_source_v2.rs:956-963）、
            // cardinality 三项都等于组内槽位数（:943-955）。产品契约
            // （`src-tauri/resources/cloud-direct/system.md`）已改成同一口径；
            // 这里仍然强制覆盖，作为**兜底**：旧 prompt、缓存或第三方配置可能仍按
            // 早先的错误写法（exact_set）产出，而这两个字段本就是题型规则推导的
            // 后端字段，模型填了也不算数。
            let unordered_set = assignment == "unordered_set";
            let slot_count = (slot_ids.len().max(1)) as u32;
            let default_scoring = if unordered_set {
                "per_slot_ielts_normalized"
            } else if crate::ielts_grammar::instruction_signature::is_completion_task(&parse_task_type(&task_type))
            {
                "per_slot_ielts_normalized"
            } else {
                "per_slot_binary"
            };
            let object = response
                .as_object_mut()
                .ok_or_else(|| "cloud_direct_response_group_not_object".to_string())?;
            object
                .entry("responseGroupId".to_string())
                .or_insert_with(|| json!(format!("{task_id}-rg-{}", position + 1)));
            object.entry("kind".to_string()).or_insert_with(|| json!(response_kind));
            object.entry("slotIds".to_string()).or_insert_with(|| json!(slot_ids));
            let default_cardinality = if unordered_set {
                json!({"min": slot_count, "max": slot_count, "exact": slot_count})
            } else {
                json!({"min": 1, "max": 1, "exact": 1})
            };
            let cardinality = object
                .entry("cardinality".to_string())
                .or_insert_with(|| default_cardinality.clone());
            // 集合题 cardinality 三项必须都等于槽位数（编译器硬规则）：缺失补、
            // 不一致也纠正——这是题型规则，不是模型的内容决定。
            if unordered_set {
                if let Some(cardinality) = cardinality.as_object_mut() {
                    for key in ["min", "max", "exact"] {
                        cardinality.insert(key.to_string(), json!(slot_count));
                    }
                }
            }
            object
                .entry("assignment".to_string())
                .or_insert_with(|| json!(assignment));
            if unordered_set {
                // 强制覆盖：契约写的 exact_set 过不了编译器（见上方注释）。
                object.insert("scoringPolicy".to_string(), json!(default_scoring));
            } else {
                object
                    .entry("scoringPolicy".to_string())
                    .or_insert_with(|| json!(default_scoring));
            }
            object
                .entry("duplicatePolicy".to_string())
                .or_insert_with(|| json!("reject_submission"));
            object
                .entry("allowOptionReuse".to_string())
                .or_insert_with(|| json!(rule.option_reuse_default));
            object
                .entry("sourceAnchors".to_string())
                .or_insert_with(|| json!([]));
            // 固定标签选项（TFNG/YNNG）：模型没给就按题型生成。
            let fixed_labels: &[&str] = match task_type.as_str() {
                "true_false_not_given" => &["TRUE", "FALSE", "NOT GIVEN"],
                "yes_no_not_given" => &["YES", "NO", "NOT GIVEN"],
                _ => &[],
            };
            if !fixed_labels.is_empty() {
                let has_options = object
                    .get("options")
                    .and_then(Value::as_array)
                    .is_some_and(|options| !options.is_empty());
                if !has_options {
                    let options: Vec<Value> = fixed_labels
                        .iter()
                        .map(|label| {
                            let option_id = format!("{task_id}-fixed-{}", label.to_ascii_lowercase().replace(' ', "-"));
                            json!({
                                "optionId": option_id,
                                "label": label,
                                "content": [{"type": "text", "id": format!("{option_id}-text"), "text": label,
                                    "sourceAnchors": [], "provenanceStatus": "source"}],
                                "sourceAnchors": []
                            })
                        })
                        .collect();
                    object.insert("options".to_string(), Value::Array(options));
                }
            }
            // 每槽位选项（single_choice 的 A–D）：模型漏填时不编造，留待编译/修复。
            responses.push(Value::Object(std::mem::take(object)));
        }
        // instructionSignature：后端派生，不采信模型直接给出的签名。
        let signature_result = crate::ielts_grammar::instruction_signature::infer_instruction_signature(
            &instruction_text_value,
            &parse_number_expression(&display_range),
            Some(&task_type),
            Vec::new(),
        );
        let mut next = Map::new();
        for key in [
            "taskId",
            "displayRange",
            "taskType",
            "instructions",
            "stimulus",
            "optionBank",
            "responseGroups",
            "sourceAnchors",
        ] {
            if let Some(value) = group.get(key) {
                next.insert(key.to_string(), value.clone());
            }
        }
        next.insert("taskId".to_string(), json!(task_id));
        next.insert("displayRange".to_string(), display_range);
        next.insert("taskType".to_string(), json!(task_type));
        next.insert("instructions".to_string(), Value::Array(instructions));
        next.insert("responseGroups".to_string(), Value::Array(responses));
        next.insert(
            "instructionSignature".to_string(),
            serde_json::to_value(&signature_result.signature)
                .map_err(|error| format!("cloud_direct_signature_encode:{error}"))?,
        );
        next.insert(
            "quality".to_string(),
            json!({"score": 0.0, "sourceCoverage": 0.0, "hardFailures": []}),
        );
        next.insert("reviewState".to_string(), json!("unreviewed"));
        next.insert("sourceAnchors".to_string(), json!([]));
        normalized_groups.push(Value::Object(next));
    }

    // ── 4) 答案槽：每个印刷题号恰好一个槽位，字段按题型规则补全 ──────────
    let mut normalized_slots = Map::new();
    for (key, slot) in &answer_slots {
        let mut slot = slot.clone();
        let question_number = slot
            .get("questionNumber")
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok())
            .or_else(|| {
                key.trim_start_matches(['q', 'Q'])
                    .parse::<u32>()
                    .ok()
            })
            .unwrap_or_default();
        // 该槽位所属题组（按 slotIds 归属）；找不到时按 short_answer 的默认规则补字段。
        let task_type = normalized_groups
            .iter()
            .find(|group| {
                group
                    .get("responseGroups")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .flat_map(|response| response.get("slotIds").and_then(Value::as_array).into_iter().flatten())
                    .any(|slot_id| slot_id.as_str() == Some(key.as_str()))
            })
            .and_then(|group| group.get("taskType").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| "short_answer".to_string());
        let rule = crate::schema::task_presentation::rule_for(&parse_task_type(&task_type), false);
        let object = slot
            .as_object_mut()
            .ok_or_else(|| "cloud_direct_answer_slot_not_object".to_string())?;
        object.insert("slotId".to_string(), json!(key));
        object.insert("questionNumber".to_string(), json!(question_number));
        object
            .entry("displayLabel".to_string())
            .or_insert_with(|| json!(question_number.to_string()));
        object
            .entry("hostType".to_string())
            .or_insert_with(|| json!(crate::schema::task_presentation::wire_name(&rule.host_types[0])));
        object
            .entry("interaction".to_string())
            .or_insert_with(|| json!(crate::schema::task_presentation::wire_name(&rule.interaction)));
        // 运行时切片只接受 scoring 槽位；其余参与类型对直出产物没有意义。
        object.insert("participation".to_string(), json!("scoring"));
        object
            .entry("confidence".to_string())
            .or_insert_with(|| json!(0.9));
        object
            .entry("sourceAnchors".to_string())
            .or_insert_with(|| json!([]));
        // hostNodeId 缺失或指向不存在的节点时尽量自愈：行内答案槽 → 所在容器；
        // 否则 → 所属响应组的第一个 prompt 段落。
        let host_missing = object
            .get("hostNodeId")
            .and_then(Value::as_str)
            .map(|host| !used_ids.contains(host))
            .unwrap_or(true);
        let is_passage_host = object
            .get("hostType")
            .and_then(Value::as_str)
            == Some("passage_paragraph");
        if host_missing && !is_passage_host {
            let resolved = find_slot_host(&Value::Array(normalized_groups.clone()), key)
                .or_else(|| {
                    let group = normalized_groups.iter().find(|group| {
                        group
                            .get("responseGroups")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .flat_map(|response| response.get("slotIds").and_then(Value::as_array).into_iter().flatten())
                            .any(|slot_id| slot_id.as_str() == Some(key.as_str()))
                    })?;
                    group
                        .get("responseGroups")
                        .and_then(Value::as_array)?
                        .first()?
                        .get("prompt")
                        .and_then(Value::as_array)?
                        .iter()
                        .find_map(|node| node.get("id").and_then(Value::as_str).map(str::to_string))
                });
            if let Some(host) = resolved {
                object.insert("hostNodeId".to_string(), json!(host));
            }
        }
        normalized_slots.insert(key.clone(), Value::Object(std::mem::take(object)));
    }

    // ── 5) 答案键：每个槽位恰好一条；缺条目的槽位如实标 unresolved ─────────
    // 形状缺口只补不拒：option 条目缺 assignment/labels、text 条目缺 values 时补
    // 默认值（assignment 取所属响应组的 assignment），绝不让整份稿因一个缺字段
    // 反序列化失败而回退。
    let slot_assignment: std::collections::BTreeMap<String, String> = normalized_groups
        .iter()
        .flat_map(|group| {
            group
                .get("responseGroups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|response| {
                    let assignment = response
                        .get("assignment")
                        .and_then(Value::as_str)
                        .unwrap_or("per_slot")
                        .to_string();
                    response
                        .get("slotIds")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(move |slot_id| (slot_id.to_string(), assignment.clone()))
                })
        })
        .collect();
    let mut normalized_answer_key = Map::new();
    for key in normalized_slots.keys() {
        let mut entry = answer_key
            .get(key)
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({"kind": "unresolved"}));
        if let Some(object) = entry.as_object_mut() {
            match object.get("kind").and_then(Value::as_str) {
                Some("option") => {
                    object
                        .entry("assignment".to_string())
                        .or_insert_with(|| {
                            json!(slot_assignment.get(key).cloned().unwrap_or_else(|| "per_slot".to_string()))
                        });
                    object.entry("labels".to_string()).or_insert_with(|| json!([]));
                }
                Some("text") => {
                    object.entry("values".to_string()).or_insert_with(|| json!([]));
                }
                _ => {
                    object.insert("kind".to_string(), json!("unresolved"));
                }
            }
        }
        normalized_answer_key.insert(key.clone(), entry);
    }
    // choose-TWO（unordered_set）语义：编译器要求组内每个槽位恰好带**一个**
    // 唯一字母标签（`labels.len()==1`，全体唯一，合起来等于正确集合）。而直出
    // 契约让模型给每个槽位写「完整正确字母集合」——这里把两种形状对齐：
    //   - 模型给了每槽位单字母（编译器形状）：原样保留；
    //   - 模型给了每槽位完整集合（契约形状）：按首见顺序把唯一字母依次分发到
    //     各槽位（内容全部来自模型，不新增任何字母）；
    //   - 有槽位完全没答案：从已有字母补齐（同组共享同一正确集合）。
    for group in &normalized_groups {
        let assignment = group
            .get("responseGroups")
            .and_then(Value::as_array)
            .and_then(|responses| responses.first())
            .and_then(|response| response.get("assignment"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if assignment != "unordered_set" {
            continue;
        }
        let slot_ids: Vec<String> = group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|response| response.get("slotIds").and_then(Value::as_array).into_iter().flatten())
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        // 组内全部槽位答案的首见唯一字母序列（模型内容的并集）。
        let mut unioned: Vec<String> = Vec::new();
        for slot_id in &slot_ids {
            for label in normalized_answer_key
                .get(slot_id)
                .and_then(|entry| entry.get("labels"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !unioned.iter().any(|seen| seen == label) {
                    unioned.push(label.to_string());
                }
            }
        }
        if unioned.is_empty() {
            continue; // 全组都没答案：如实留 unresolved，交给编辑/编译暴露
        }
        // 按槽位顺序分发：第 i 个槽位拿第 i 个唯一字母。模型给的字母少于槽位数时，
        // 多出来的槽位如实标 unresolved——不复制已有字母冒充第二个答案（那是伪造
        // 内容），编译器的集合题答案策略会把它暴露成待修问题。
        for (index, slot_id) in slot_ids.iter().enumerate() {
            let Some(label) = unioned.get(index).cloned() else {
                normalized_answer_key
                    .insert(slot_id.clone(), json!({"kind": "unresolved"}));
                continue;
            };
            let entry = normalized_answer_key
                .entry(slot_id.clone())
                .or_insert_with(|| json!({"kind": "option", "assignment": "unordered_set"}));
            if entry.get("kind").and_then(Value::as_str) != Some("option") {
                *entry = json!({"kind": "option", "assignment": "unordered_set"});
            }
            if let Some(object) = entry.as_object_mut() {
                object.insert("labels".to_string(), json!([label]));
                object.insert("assignment".to_string(), json!("unordered_set"));
            }
        }
    }

    // ── 6) 装配完整稿件 ────────────────────────────────────────────────
    let mut notes: Vec<String> = vec![
        "cloud_direct: 一次云端调用直出整份题稿（原文+题组+答案），未走本地合并链".to_string(),
    ];
    notes.extend(model_warnings.iter().cloned());
    if !dropped_asset_nodes.is_empty() {
        notes.push(format!(
            "cloud_direct_figures_dropped:{}（直出链不登记图片资产）",
            dropped_asset_nodes.join(",")
        ));
    }
    if let Some(title) = meta
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
    {
        notes.push(format!("cloud_direct_meta_title:{title}"));
    }
    let mut document = Map::new();
    document.insert("schemaVersion".to_string(), json!(IELTS_AUTHORING_IR_V2_SCHEMA_VERSION));
    document.insert("jobId".to_string(), json!(identity.job_id));
    document.insert("exam".to_string(), identity.exam.clone());
    document.insert("modality".to_string(), json!("reading"));
    document.insert("passage".to_string(), passage);
    document.insert("taskGroups".to_string(), Value::Array(normalized_groups));
    document.insert("answerSlots".to_string(), Value::Object(normalized_slots));
    document.insert("answerKey".to_string(), Value::Object(normalized_answer_key));
    document.insert("assets".to_string(), json!([]));
    document.insert("sourceDocumentId".to_string(), json!(identity.source_document_id));
    document.insert("quality".to_string(), json!({}));
    document.insert(
        "audit".to_string(),
        json!({
            "revision": 0,
            "source": "cloud_direct",
            "humanVerified": false,
            "llmUsed": true,
            "updatedAt": identity.generated_at,
            "notes": notes
        }),
    );
    let mut document = Value::Object(document);
    // 质量报告用同一套质量管线重算（占位块不冒充结论）。
    crate::authoring_v2_commands::refresh_quality_report(root, &identity.job_id, &mut document)
        .map_err(|error| format!("cloud_direct_quality_refresh:{error}"))?;
    Ok(document)
}

/// 题型 wire name → `TaskTypeV2`。
fn parse_task_type(name: &str) -> crate::schema::ielts_authoring_v2::TaskTypeV2 {
    use crate::schema::ielts_authoring_v2::TaskTypeV2;
    match name {
        "single_choice" => TaskTypeV2::SingleChoice,
        "multiple_choice" => TaskTypeV2::MultipleChoice,
        "true_false_not_given" => TaskTypeV2::TrueFalseNotGiven,
        "yes_no_not_given" => TaskTypeV2::YesNoNotGiven,
        "matching_information" => TaskTypeV2::MatchingInformation,
        "matching_headings" => TaskTypeV2::MatchingHeadings,
        "matching_features" => TaskTypeV2::MatchingFeatures,
        "matching_sentence_endings" => TaskTypeV2::MatchingSentenceEndings,
        "classification" => TaskTypeV2::Classification,
        "sentence_completion" => TaskTypeV2::SentenceCompletion,
        "summary_completion" => TaskTypeV2::SummaryCompletion,
        "note_completion" => TaskTypeV2::NoteCompletion,
        "table_completion" => TaskTypeV2::TableCompletion,
        "form_completion" => TaskTypeV2::FormCompletion,
        "flowchart_completion" => TaskTypeV2::FlowchartCompletion,
        "diagram_label_completion" => TaskTypeV2::DiagramLabelCompletion,
        "plan_map_label_completion" => TaskTypeV2::PlanMapLabelCompletion,
        _ => TaskTypeV2::ShortAnswer,
    }
}

/// 由题号集合推导 displayRange（连续 → range，否则 set）。
fn range_from_numbers(numbers: &[u32]) -> Value {
    if numbers.is_empty() {
        return json!({"kind": "range", "start": 1, "end": 1});
    }
    let contiguous = numbers
        .windows(2)
        .all(|pair| pair[1] == pair[0] + 1);
    if contiguous {
        json!({"kind": "range", "start": numbers[0], "end": numbers[numbers.len() - 1]})
    } else {
        json!({"kind": "set", "values": numbers})
    }
}

/// displayRange JSON → `QuestionNumberExpressionV2`（签名推导用）。
fn parse_number_expression(
    value: &Value,
) -> crate::schema::ielts_authoring_v2::QuestionNumberExpressionV2 {
    use crate::schema::ielts_authoring_v2::QuestionNumberExpressionV2;
    match value.get("kind").and_then(Value::as_str) {
        Some("set") => QuestionNumberExpressionV2::Set {
            values: value
                .get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|number| number.as_u64())
                .filter_map(|number| u32::try_from(number).ok())
                .collect(),
        },
        _ => QuestionNumberExpressionV2::Range {
            start: value.get("start").and_then(Value::as_u64).unwrap_or(1) as u32,
            end: value.get("end").and_then(Value::as_u64).unwrap_or(1) as u32,
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 编译校验（唯一编译器）与保真度提示（非阻断）
// ─────────────────────────────────────────────────────────────────────────────

/// 归一化产物过**唯一**编译器。反序列化失败本身就是一条编译问题
/// （由修复回合或人工在编辑器里处理），不作为链的异常抛出。
fn compile_document(document: &Value) -> Vec<CompilerIssueV2> {
    match serde_json::from_value::<crate::schema::IeltsAuthoringIRV2>(document.clone()) {
        Ok(typed) => crate::listening_source_v1::compile_exam_source_v2(&typed)
            .err()
            .unwrap_or_default(),
        Err(error) => vec![CompilerIssueV2 {
            code: "AUTHORING_SCHEMA_INVALID".to_string(),
            message: error.to_string(),
            target_id: "document".to_string(),
        }],
    }
}

/// 把编译问题写进 `quality.issues`（warning 级）：编辑器问题列表与发布警告
/// 都能读到「哪里还需要改」。severity 不用 blocking——编译失败已由发布门禁的
/// COMPILE_FAILED 阻断，这里再记一份 blocking 只会让同一事实两种身份。
fn attach_compile_issues(document: &mut Value, issues: &[CompilerIssueV2]) {
    let Some(list) = document
        .pointer_mut("/quality/issues")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for issue in issues {
        list.push(json!({
            "issueId": format!("cloud-direct-compile-{}-{}", issue.code, issue.target_id),
            "code": issue.code,
            "severity": "warning",
            "message": issue.message,
            "targetType": "region",
            "targetId": issue.target_id,
            "sourceAnchors": [],
            "suggestedActions": ["edit_text"],
            "details": {"source": "cloud_direct_compile"}
        }));
    }
}

/// 正文保真度：段落词覆盖率与 PDF 文字层对照。低于阈值的段落在编辑器里
/// 标「请核对」（写入 `quality.issues`，非阻断）。扫描件（无文字层）跳过。
fn assess_passage_fidelity(
    document: &mut Value,
    page_texts: &[(usize, String)],
    warnings: &mut Vec<String>,
) -> Option<Value> {
    if page_texts.is_empty() {
        warnings.push("cloud_direct_fidelity_skipped:扫描件无文字层，保真度检查跳过，请人工核对".to_string());
        return None;
    }
    let pdf_words: std::collections::BTreeSet<String> = page_texts
        .iter()
        .flat_map(|(_, text)| normalize_words(text))
        .collect();
    let Some(content) = document
        .pointer("/passage/content")
        .and_then(Value::as_array)
        .cloned()
    else {
        return None;
    };
    let mut paragraphs = Vec::new();
    let mut flagged = Vec::new();
    let mut total_words = 0usize;
    let mut matched_words = 0usize;
    for node in &content {
        if node.get("type").and_then(Value::as_str) != Some("paragraph") {
            continue;
        }
        let node_id = node
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let label = node
            .get("paragraphLabel")
            .and_then(Value::as_str)
            .map(str::to_string);
        let text = crate::reconcile::candidate::nodes_text(&Value::Array(vec![node.clone()]));
        let words = normalize_words(&text);
        if words.is_empty() {
            continue;
        }
        let unique_words: std::collections::BTreeSet<String> = words.into_iter().collect();
        let hits = unique_words
            .iter()
            .filter(|word| pdf_words.contains(*word))
            .count();
        let coverage = hits as f64 / unique_words.len() as f64;
        total_words += unique_words.len();
        matched_words += hits;
        let is_flagged = coverage < FIDELITY_PARAGRAPH_THRESHOLD;
        if is_flagged {
            flagged.push(node_id.clone());
        }
        paragraphs.push(json!({
            "nodeId": node_id,
            "paragraphLabel": label,
            "wordCount": unique_words.len(),
            "coverage": coverage,
            "flagged": is_flagged
        }));
    }
    let overall = if total_words == 0 {
        1.0
    } else {
        matched_words as f64 / total_words as f64
    };
    let report = json!({
        "schemaVersion": "CloudDirectFidelityV1",
        "threshold": FIDELITY_PARAGRAPH_THRESHOLD,
        "overallCoverage": overall,
        "flaggedParagraphIds": flagged,
        "paragraphs": paragraphs
    });
    // 低覆盖段落写进质量报告（warning 级，非阻断）：编辑器问题列表与发布警告可见。
    if !flagged.is_empty() {
        if let Some(issues) = document
            .pointer_mut("/quality/issues")
            .and_then(Value::as_array_mut)
        {
            for node_id in &flagged {
                let detail = paragraphs
                    .iter()
                    .find(|entry| entry.get("nodeId").and_then(Value::as_str) == Some(node_id.as_str()))
                    .cloned()
                    .unwrap_or(Value::Null);
                issues.push(json!({
                    "issueId": format!("cloud-direct-fidelity-{node_id}"),
                    "code": "CLOUD_DIRECT_FIDELITY_LOW",
                    "severity": "warning",
                    "message": "这段正文与 PDF 文字层差异较大，可能被模型改写或漏译，请核对。",
                    "targetType": "region",
                    "targetId": node_id,
                    "sourceAnchors": [],
                    "suggestedActions": ["edit_text"],
                    "details": {"fidelity": detail}
                }));
            }
        }
    }
    Some(report)
}

/// 词级归一：小写、去首尾标点（与验证脚本同一粗略口径）。
fn normalize_words(text: &str) -> Vec<String> {
    text.split(|character: char| character.is_whitespace() || character.is_ascii_punctuation())
        .map(|word| word.trim().to_ascii_lowercase())
        .filter(|word| !word.is_empty())
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// 落盘：直出结果 → 作业 artifacts（诊断 + 可播种的正式稿影子）
// ─────────────────────────────────────────────────────────────────────────────

/// 直出结果文件名（诊断与编辑器提示读取）。
pub(crate) const DIRECT_RESULT_FILE: &str = "cloud-direct-result.json";
/// 保真度报告文件名。
pub(crate) const DIRECT_FIDELITY_FILE: &str = "cloud-direct-fidelity.json";

/// 把直出稿**整份替换**为权威稿（重新识别场景）。
///
/// 与 [`crate::library::migration::ensure_initial_canonical` 的分工：那个只在
/// **没有**权威稿时播种；重新识别时用户看到的结果必须更新，直出稿不能只停在
/// 影子文件里。替换走与云端采纳同一条 CAS 事务（版本冲突从新快照重试），
/// 人工编辑由事务函数保护；`audit.source=cloud_direct` 如实记录来源。
pub(crate) fn write_direct_canonical(
    root: &std::path::Path,
    job_id: &str,
    document: &Value,
) -> CommandResult<i64> {
    write_direct_canonical_inner(root, job_id, document, None)
}

pub(crate) fn write_direct_canonical_leased(
    root: &std::path::Path, job_id: &str, document: &Value, worker_id: &str,
) -> CommandResult<i64> {
    write_direct_canonical_inner(root, job_id, document, Some(worker_id))
}

fn check_direct_lease(conn: &rusqlite::Connection, job_id: &str, worker_id: Option<&str>) -> CommandResult<()> {
    if let Some(worker) = worker_id {
        let valid: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM processing_jobs_v2 WHERE id=?1 AND lease_owner=?2
             AND lease_expires_at>=?3 AND cancel_requested_at IS NULL
             AND stage IN ('running','local_recognition','cloud_recognition','reconciling'))",
            rusqlite::params![job_id, worker, chrono::Utc::now().to_rfc3339()], |row| row.get(0)
        ).map_err(|e| format!("cloud_direct_lease_check:{e}"))?;
        if !valid { return Err("processing_lease_lost".to_string()); }
    }
    Ok(())
}

fn write_direct_canonical_inner(
    root: &std::path::Path, job_id: &str, document: &Value, worker_id: Option<&str>,
) -> CommandResult<i64> {
    let request_id = format!("cloud-direct:{job_id}:{}", chrono::Utc::now().timestamp_millis());
    let mut last_conflict = None;
    for _attempt in 0..3 {
        let mut conn = crate::library::repository::open_library_connection(root)?;
        let Some((current, current_version)) =
            crate::library::repository::get_canonical_ds(&conn, job_id)?
        else {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| format!("cloud_direct_seed_tx:{e}"))?;
            check_direct_lease(&tx, job_id, worker_id)?;
            if !crate::library::repository::seed_canonical_ds(&tx, job_id, &document.to_string(), "action_required")? {
                return Err("cloud_direct_canonical_not_seeded".to_string());
            }
            tx.commit().map_err(|e| format!("cloud_direct_seed_commit:{e}"))?;
            return Ok(1);
        };
        let mut merged = document.clone();
        // 资源清单是后端/用户身份，不随直出替换。
        if let Some(assets) = current.get("assets") {
            merged["assets"] = assets.clone();
        }
        if let (Some(current_audit), Some(audit)) = (
            current.get("audit").and_then(Value::as_object),
            merged.get_mut("audit").and_then(Value::as_object_mut),
        ) {
            audit.insert(
                "revision".to_string(),
                json!(current_audit
                    .get("revision")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .saturating_add(1)),
            );
            audit.insert("source".to_string(), json!("cloud_direct"));
            // 新内容未经人工确认：verified 不继承（旧稿的确认不覆盖新稿）。
            audit.insert("humanVerified".to_string(), json!(false));
            audit.insert("updatedAt".to_string(), json!(chrono::Utc::now().to_rfc3339()));
        }
        let command = json!({
            "op": "replaceAuthoringDocument",
            "authoring": merged,
        });
        let result = crate::library::repository::apply_editor_commands_tx_with(
            &mut conn,
            &crate::library::repository::ApplyEditorCommandsInput {
                item_id: job_id.to_string(),
                base_version: current_version,
                request_id: Some(request_id.clone()),
                commands: vec![command],
                title: None,
            },
            // 复用 CloudCandidateAdoption：语义即「云端产物提升为正式稿」，
            // 事务里它对人工保护的处理与直出替换需要的一致。
            crate::library::repository::EditOrigin::CloudCandidateAdoption,
            None,
            &|document, patch| {
                if patch.get("op").and_then(Value::as_str) == Some("replaceAuthoringDocument") {
                    let authoring = patch
                        .get("authoring")
                        .filter(|value| value.is_object())
                        .ok_or_else(|| "cloud_direct_document_invalid".to_string())?;
                    *document = authoring.clone();
                    Ok(())
                } else {
                    crate::authoring_v2_commands::apply_patch(document, patch)
                }
            },
            &|document| {
                crate::authoring_v2_commands::refresh_quality_report(root, job_id, document)?;
                crate::authoring_v2_commands::validate_authoring(document)
            },
            &|conn, _| check_direct_lease(conn, job_id, worker_id),
        );
        match result {
            Ok(result) => return Ok(result.edit_version),
            Err(error) if error.starts_with("EDIT_VERSION_CONFLICT") => {
                last_conflict = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_conflict.unwrap_or_else(|| "cloud_direct_canonical_conflict".to_string()))
}

/// 把直出结果写进作业目录：
/// - `cloud-direct-result.json`：完整记录（题稿 + 调用次数 + 编译问题 + 警告）；
/// - `cloud-direct-fidelity.json`：保真度报告（若有）；
/// - `authoring-ir-v2.shadow.json`：归一化后的题稿——`ensure_initial_canonical`
///   据此播种权威稿（与本地链同一入口，直出稿因此走同一套编辑器/发布路径）。
pub(crate) fn write_direct_result(
    root: &std::path::Path,
    job_id: &str,
    outcome: &DirectExamOutcome,
) -> CommandResult<()> {
    let dir = crate::util::job_dir(root, job_id);
    std::fs::create_dir_all(&dir).map_err(|error| format!("cloud_direct_dir_create:{error}"))?;
    let record = json!({
        "schemaVersion": "CloudDirectRunV1",
        "jobId": job_id,
        "calls": outcome.calls,
        "chunked": outcome.chunked,
        "compileIssues": outcome.compile_issues.iter().map(|issue| json!({
            "code": issue.code, "targetId": issue.target_id, "message": issue.message
        })).collect::<Vec<_>>(),
        "warnings": outcome.warnings,
        "document": outcome.document,
        "recordedAt": chrono::Utc::now().to_rfc3339()
    });
    crate::util::write_json(&dir.join(DIRECT_RESULT_FILE), &record)?;
    if let Some(fidelity) = &outcome.fidelity {
        crate::util::write_json(&dir.join(DIRECT_FIDELITY_FILE), fidelity)?;
    }
    crate::util::write_json(
        &dir.join(crate::authoring_v2_commands::AUTHORING_V2_SHADOW_FILE),
        &outcome.document,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn identity() -> DirectIdentity {
        DirectIdentity {
            job_id: "import-20261014000000-abcd1234".to_string(),
            exam: json!({
                "examId": "p1-abcd1234",
                "title": "Test",
                "language": "en",
                "tags": [],
                "sourceFiles": [{"sourceFileId": "src-1", "role": "question_paper"}]
            }),
            source_file_id: "src-1".to_string(),
            source_document_id: "document-import-20261014000000-abcd1234".to_string(),
            generated_at: "2026-10-14T00:00:00Z".to_string(),
        }
    }

    fn paragraph(id: &str, label: Option<&str>, text: &str) -> Value {
        let mut node = json!({
            "type": "paragraph",
            "id": id,
            "children": [{"type": "text", "id": format!("{id}-text"), "text": text}]
        });
        if let Some(label) = label {
            node["paragraphLabel"] = json!(label);
        }
        node
    }

    /// 一份最小但完整的直出产物（TFNG + 段落信息匹配 + 摘要填空）。
    fn minimal_raw() -> Value {
        json!({
            "meta": {"title": "The Pearls", "language": "en", "category": "P1"},
            "passage": {
                "content": [
                    paragraph("passage-paragraph-A", Some("A"), "Paragraph A text about pearls."),
                    paragraph("passage-paragraph-B", Some("B"), "Paragraph B text about shells.")
                ],
                "paragraphMap": {"A": "passage-paragraph-A", "B": "passage-paragraph-B"}
            },
            "taskGroups": [{
                "taskId": "group-1",
                "taskType": "true_false_not_given",
                "displayRange": {"kind": "range", "start": 1, "end": 2},
                "instructions": [{"type": "paragraph", "id": "group-1-instr", "children": [
                    {"type": "text", "id": "group-1-instr-text", "text": "Questions 1-2 Do the following statements agree with the information in Reading Passage 1? TRUE FALSE NOT GIVEN"}
                ]}],
                "responseGroups": [
                    {"responseGroupId": "group-1-rg-1", "kind": "choice", "slotIds": ["q1"],
                     "prompt": [{"type": "paragraph", "id": "group-1-q1-prompt", "children": [
                        {"type": "text", "id": "group-1-q1-text", "text": "Pearls are formed by oysters."}
                     ]}]},
                    {"responseGroupId": "group-1-rg-2", "kind": "choice", "slotIds": ["q2"],
                     "prompt": [{"type": "paragraph", "id": "group-1-q2-prompt", "children": [
                        {"type": "text", "id": "group-1-q2-text", "text": "All shells contain pearls."}
                     ]}]}
                ]
            }],
            "answerSlots": {
                "q1": {"slotId": "q1", "questionNumber": 1, "displayLabel": "1", "hostNodeId": "group-1-q1-prompt", "hostType": "prompt", "interaction": "radio"},
                "q2": {"slotId": "q2", "questionNumber": 2, "displayLabel": "2", "hostNodeId": "group-1-q2-prompt", "hostType": "prompt", "interaction": "radio"}
            },
            "answerKey": {
                "q1": {"kind": "option", "labels": ["TRUE"], "assignment": "per_slot"},
                "q2": {"kind": "option", "labels": ["NOT GIVEN"], "assignment": "per_slot"}
            },
            "warnings": []
        })
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "pdf2test-cloud-direct-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn normalization_fills_backend_owned_fields_without_rejecting() {
        let root = temp_root("normalize");
        let document = normalize_direct_output(&root, &minimal_raw(), &identity())
            .expect("最小直出产物必须能归一化");
        // 身份由后端给出。
        assert_eq!(document["schemaVersion"], json!("IeltsAuthoringIRV2"));
        assert_eq!(document["jobId"], json!("import-20261014000000-abcd1234"));
        assert_eq!(document["modality"], json!("reading"));
        assert_eq!(document["audit"]["source"], json!("cloud_direct"));
        assert_eq!(document["audit"]["llmUsed"], json!(true));
        // TFNG 固定选项由归一化补齐。
        let options = document["taskGroups"][0]["responseGroups"][0]["options"]
            .as_array()
            .expect("TFNG options must be generated");
        let labels: Vec<&str> = options.iter().filter_map(|o| o["label"].as_str()).collect();
        assert_eq!(labels, vec!["TRUE", "FALSE", "NOT GIVEN"]);
        // 题型可推导字段全部补齐。
        let response = &document["taskGroups"][0]["responseGroups"][0];
        assert_eq!(response["cardinality"]["min"], json!(1));
        assert_eq!(response["assignment"], json!("per_slot"));
        assert_eq!(response["scoringPolicy"], json!("per_slot_binary"));
        assert_eq!(response["duplicatePolicy"], json!("reject_submission"));
        assert_eq!(response["allowOptionReuse"], json!(true));
        assert_eq!(response["sourceAnchors"], json!([]));
        assert!(document["taskGroups"][0]["instructionSignature"].is_object());
        assert_eq!(document["taskGroups"][0]["reviewState"], json!("unreviewed"));
        // 槽位：participation 收敛为 scoring，锚点补齐。
        assert_eq!(document["answerSlots"]["q1"]["participation"], json!("scoring"));
        assert_eq!(document["answerSlots"]["q1"]["sourceAnchors"], json!([]));
        assert_eq!(document["answerSlots"]["q1"]["confidence"], json!(0.9));
        // 编译通过（唯一编译器）。
        let issues = compile_document(&document);
        assert!(issues.is_empty(), "归一化产物必须编译通过: {issues:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn normalization_keeps_model_ids_and_labels() {
        let root = temp_root("ids");
        let document = normalize_direct_output(&root, &minimal_raw(), &identity()).unwrap();
        let map = document["passage"]["paragraphMap"].as_object().unwrap();
        assert_eq!(map["A"], json!("passage-paragraph-A"));
        assert_eq!(
            document["passage"]["content"][0]["paragraphLabel"],
            json!("A")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_answer_key_entries_become_unresolved_not_rejections() {
        let root = temp_root("unresolved");
        let mut raw = minimal_raw();
        raw["answerKey"] = json!({"q1": {"kind": "option", "labels": ["TRUE"], "assignment": "per_slot"}});
        let document = normalize_direct_output(&root, &raw, &identity()).unwrap();
        assert_eq!(document["answerKey"]["q2"]["kind"], json!("unresolved"));
        // 未解析答案不再让编译器失败（学生端按未作答计 0 分）。
        let issues = compile_document(&document);
        assert!(
            !issues.iter().any(|issue| issue.code == "RUNTIME_ANSWER_UNRESOLVED"),
            "未解析答案不得作为编译阻断: {issues:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn no_task_groups_is_unusable() {
        let root = temp_root("no-groups");
        let mut raw = minimal_raw();
        raw["taskGroups"] = json!([]);
        let error = normalize_direct_output(&root, &raw, &identity()).unwrap_err();
        assert_eq!(error, "cloud_direct_no_task_groups");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn no_passage_is_unusable() {
        let root = temp_root("no-passage");
        let mut raw = minimal_raw();
        raw["passage"] = json!({"content": []});
        let error = normalize_direct_output(&root, &raw, &identity()).unwrap_err();
        assert_eq!(error, "cloud_direct_no_passage");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fidelity_flags_low_coverage_paragraphs() {
        let root = temp_root("fidelity");
        let mut document = normalize_direct_output(&root, &minimal_raw(), &identity()).unwrap();
        // 篡改一段正文：覆盖率必须掉到阈值以下并被标出。
        if let Some(node) = document
            .pointer_mut("/passage/content/1")
            .and_then(Value::as_object_mut)
        {
            if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
                if let Some(first) = children.first_mut().and_then(Value::as_object_mut) {
                    first.insert(
                        "text".to_string(),
                        json!("completely different invented words here now"),
                    );
                }
            }
        }
        let page_texts = vec![(1usize, "Paragraph A text about pearls. Paragraph B text about shells.".to_string())];
        let mut warnings = Vec::new();
        let report = assess_passage_fidelity(&mut document, &page_texts, &mut warnings)
            .expect("有文字层时必须产出报告");
        let flagged = report["flaggedParagraphIds"].as_array().unwrap();
        assert_eq!(flagged.len(), 1, "只应标记被篡改的那段: {report}");
        assert_eq!(flagged[0], json!("passage-paragraph-B"));
        assert_eq!(report["overallCoverage"].as_f64().unwrap() < 1.0, true);
        // 低覆盖段落入质量报告（warning 级）。
        let issues = document["quality"]["issues"].as_array().unwrap();
        assert!(issues.iter().any(|issue| issue["code"] == json!("CLOUD_DIRECT_FIDELITY_LOW")
            && issue["severity"] == json!("warning")
            && issue["targetId"] == json!("passage-paragraph-B")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fidelity_is_skipped_for_scanned_pdfs() {
        let root = temp_root("fidelity-scan");
        let mut document = normalize_direct_output(&root, &minimal_raw(), &identity()).unwrap();
        let mut warnings = Vec::new();
        let report = assess_passage_fidelity(&mut document, &[], &mut warnings);
        assert!(report.is_none());
        assert!(warnings.iter().any(|warning| warning.contains("扫描件")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn chunk_merge_combines_groups_slots_and_answers() {
        let chunk_a = json!({
            "meta": {"title": "Chunked"},
            "passage": {"content": [paragraph("p1", None, "first half passage")]},
            "taskGroups": [{"taskId": "group-1", "taskType": "true_false_not_given",
                "displayRange": {"kind": "range", "start": 1, "end": 1},
                "instructions": [], "responseGroups": [{"responseGroupId": "g1-rg", "slotIds": ["q1"]}]}],
            "answerSlots": {"q1": {"slotId": "q1", "questionNumber": 1}},
            "answerKey": {"q1": {"kind": "option", "labels": ["TRUE"]}},
            "warnings": ["a"]
        });
        let chunk_b = json!({
            "taskGroups": [{"taskId": "group-2", "taskType": "matching_headings",
                "displayRange": {"kind": "range", "start": 2, "end": 2},
                "instructions": [], "responseGroups": [{"responseGroupId": "g2-rg", "slotIds": ["q2"]}]}],
            "answerSlots": {"q2": {"slotId": "q2", "questionNumber": 2}},
            "answerKey": {"q2": {"kind": "option", "labels": ["iv"]}},
            "warnings": ["b"]
        });
        let merged = merge_direct_chunks(vec![chunk_a, chunk_b]);
        assert_eq!(merged["taskGroups"].as_array().unwrap().len(), 2);
        assert_eq!(merged["answerSlots"].as_object().unwrap().len(), 2);
        assert_eq!(merged["answerKey"]["q2"]["labels"][0], json!("iv"));
        assert_eq!(merged["warnings"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn product_examples_all_normalize_and_compile() {
        // 产品资源里的范例（主范例 + 两份补充）必须真能走完整条直出后处理链：
        // 归一化（只补不拒）→ 唯一 Rust 编译器 0 问题。范例是给模型的契约示范，
        // 示范本身过不了编译器就等于教模型写错——这条测试是范例的验收。
        let root = temp_root("examples");
        let examples = direct_examples();
        assert_eq!(examples.len(), 3, "主范例 + TFNG/单选 + 表格/流程图填空");
        for (name, document) in examples {
            assert!(document.is_object(), "{name}: 范例必须是 JSON 对象");
            let normalized = normalize_direct_output(&root, &document, &identity())
                .unwrap_or_else(|error| panic!("{name}: 归一化失败 {error}"));
            let issues = compile_document(&normalized);
            assert!(issues.is_empty(), "{name}: 范例编译问题 {issues:?}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn product_contract_is_the_single_source_of_truth() {
        // 契约的**唯一事实来源**是产品资源 `src-tauri/resources/cloud-direct/`；
        // `docs/plans/cloud-direct/prompt/` 只是历史验证材料（其中 choose-TWO 的
        // 写法是错的，已在产品副本里按编译器规则改正），因此**不再**断言两者一致。
        //
        // 这条测试改为钉住产品契约本身的关键纪律与集合题口径。
        assert!(direct_system_prompt().contains("You convert ONE IELTS Reading test PDF"));
        assert!(direct_system_prompt().contains("paragraphMap"));
        assert!(direct_system_prompt().contains("answerKey"));
        // 集合题：记分策略必须是编译器口径，且不得再出现 exact_set 的旧写法。
        assert!(
            direct_system_prompt().contains("scoringPolicy per_slot_ielts_normalized"),
            "choose-TWO 必须按编译器口径写 scoringPolicy"
        );
        assert!(
            !direct_system_prompt().contains("scoringPolicy exact_set"),
            "旧的 exact_set 写法不得再出现在契约里"
        );
        assert!(
            direct_system_prompt().contains("exactly ONE of the correct letters"),
            "choose-TWO 的答案键必须是「每槽位一个字母」"
        );
    }

    #[test]
    fn system_prompt_comes_from_the_validated_contract() {
        // 契约以 include_str! 纳入主线程验证过的 prompt：关键纪律必须原样在场。
        assert!(direct_system_prompt().contains("You convert ONE IELTS Reading test PDF"));
        assert!(direct_system_prompt().contains("paragraphMap"));
        assert!(direct_system_prompt().contains("answerKey"));
    }

    #[test]
    fn range_from_numbers_prefers_range_for_contiguous_numbers() {
        assert_eq!(range_from_numbers(&[1, 2, 3]), json!({"kind": "range", "start": 1, "end": 3}));
        assert_eq!(
            range_from_numbers(&[1, 3]),
            json!({"kind": "set", "values": [1, 3]})
        );
    }

    // ── 命令层集成：受控 LLM 服务 + 真实 fixture PDF ─────────────────────
    // 覆盖 run_exam_direct 的完整后端链：证据面 → 调用 → 归一化 → 编译 → 落盘。
    // UI 层验收受环境限制时，这条链是产品后端路径的命令层证据。

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// 受控 OpenAI 兼容服务：每个连接读一个请求，把请求体送进 channel，
    /// 按 `replies` 队列依次回包（finish_reason 可控）。
    struct ControlledService {
        base_url: String,
        requests: mpsc::Receiver<String>,
    }

    fn controlled_service(replies: Vec<(String, u16)>) -> ControlledService {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind controlled service");
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for (body, status) in replies {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut buffer = [0u8; 8192];
                let mut raw = String::new();
                // 读满 content-length（请求体可能远大于 8KB）。
                loop {
                    let Ok(read) = stream.read(&mut buffer) else { break };
                    if read == 0 { break }
                    raw.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    if let Some(length) = raw.split("\r\n\r\n").nth(1).map(|body| body.len()) {
                        let declared = raw
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length").then(|| value.trim().to_string())
                            })
                            .and_then(|value| value.parse::<usize>().ok());
                        if declared.is_some_and(|declared| length >= declared) { break }
                    }
                }
                let _ = sender.send(raw.clone());
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        ControlledService { base_url: format!("http://127.0.0.1:{}/v1", address.port()), requests: receiver }
    }

    fn direct_exam_with_prompt(prompt: &str, finish_reason: &str, question_numbers: &[u32]) -> String {
        let mut slots = serde_json::Map::new();
        let mut key = serde_json::Map::new();
        let mut responses = Vec::new();
        for number in question_numbers {
            let slot_id = format!("q{number}");
            slots.insert(
                slot_id.clone(),
                json!({
                    "slotId": slot_id,
                    "questionNumber": number,
                    "displayLabel": number.to_string(),
                    "hostNodeId": format!("group-1-q{number}-prompt"),
                    "hostType": "prompt",
                    "interaction": "radio"
                }),
            );
            key.insert(
                slot_id.clone(),
                json!({"kind": "option", "labels": ["TRUE"], "assignment": "per_slot"}),
            );
            responses.push(json!({
                "responseGroupId": format!("group-1-q{number}"),
                "kind": "choice",
                "slotIds": [slot_id],
                "prompt": [{"type": "paragraph", "id": format!("group-1-q{number}-prompt"), "children": [
                    {"type": "text", "id": format!("group-1-q{number}-text"), "text": format!("Statement {number} about pearls.")}
                ]}]
            }));
        }
        json!({
            "choices": [{"message": {"content": serde_json::to_string(&json!({
                "meta": {"title": "The Pearls", "language": "en", "category": "P1"},
                "passage": {
                    "content": [paragraph("passage-paragraph-A", Some("A"), prompt)],
                    "paragraphMap": {"A": "passage-paragraph-A"}
                },
                "taskGroups": [{
                    "taskId": "group-1",
                    "taskType": "true_false_not_given",
                    "displayRange": {"kind": "range", "start": question_numbers[0], "end": question_numbers[question_numbers.len()-1]},
                    "instructions": [{"type": "paragraph", "id": "group-1-instr", "children": [
                        {"type": "text", "id": "group-1-instr-text", "text": "Questions 1-2 Do the following statements agree with the information in Reading Passage 1? In boxes 1-2 on your answer sheet write TRUE FALSE NOT GIVEN"}
                    ]}],
                    "responseGroups": responses
                }],
                "answerSlots": Value::Object(slots),
                "answerKey": Value::Object(key),
                "warnings": []
            })).unwrap()}, "finish_reason": finish_reason}],
            "usage": {"prompt_tokens": 1200, "completion_tokens": 340, "total_tokens": 1540}
        }).to_string()
    }

    fn direct_job_with_fixture(root: &std::path::Path) -> crate::ImportJob {
        direct_job_with_named_fixture(root, "complex-reading.pdf")
    }

    fn direct_job_with_named_fixture(root: &std::path::Path, fixture: &str) -> crate::ImportJob {
        let job = crate::job_store::make_job(crate::CreateJobInput {
            title: Some("Fixture Pearls".to_string()),
            category: Some("P1".to_string()),
            frequency: None,
            tags: None,
            llm_profile_id: None,
        });
        crate::job_store::save_job(root, &job).unwrap();
        let dir = crate::util::job_dir(root, &job.job_id);
        std::fs::create_dir_all(dir.join("uploads")).unwrap();
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../fixtures/parser/{fixture}")),
            dir.join("uploads/fixture.pdf"),
        )
        .unwrap();
        let mut job = job;
        job.source_files = vec![crate::SourceFile {
            file_id: "src-1".to_string(),
            original_name: "fixture.pdf".to_string(),
            stored_name: "fixture.pdf".to_string(),
            file_type: "pdf".to_string(),
            sha256: "a".repeat(64),
            size_bytes: 123,
            role: "MainQuestion".to_string(),
            imported_at: job.created_at,
        }];
        crate::job_store::save_job(root, &job).unwrap();
        job
    }

    fn controlled_profile(root: &std::path::Path, base_url: &str) -> String {
        let profile_id = "controlled-direct".to_string();
        crate::llm_profiles::save_profiles(
            root,
            &[json!({
                "profileId": profile_id,
                "provider": "OpenAiCompatible",
                "baseUrl": base_url,
                "model": "controlled-flash",
                "temperature": 0,
                "timeoutMs": 30000,
                "forceJson": true,
                "enabled": true
            })],
        )
        .unwrap();
        profile_id
    }

    #[test]
    fn direct_chain_runs_end_to_end_against_a_controlled_service() {
        let root = temp_root("e2e-direct");
        crate::util::ensure_app_dirs(&root).unwrap();
        let service = controlled_service(vec![(
            direct_exam_with_prompt("Pearls are formed inside oysters.", "stop", &[1, 2]),
            200,
        )]);
        let job = direct_job_with_fixture(&root);
        let profile_id = controlled_profile(&root, &service.base_url);

        let outcome = run_exam_direct(&root, &job.job_id, Some(&profile_id))
            .expect("controlled direct chain must succeed");

        // 调用与编译。
        assert_eq!(outcome.calls, 1);
        assert!(!outcome.chunked);
        assert!(outcome.compile_issues.is_empty(), "{:?}", outcome.compile_issues);
        // 身份与产物。
        assert_eq!(outcome.document["audit"]["source"], json!("cloud_direct"));
        assert_eq!(outcome.document["jobId"], json!(job.job_id));
        assert_eq!(outcome.document["modality"], json!("reading"));
        assert_eq!(outcome.document["answerKey"]["q1"]["labels"][0], json!("TRUE"));
        // 落盘是 write_direct_result 的职责（调度器在可用性判定后调用）。
        write_direct_result(&root, &job.job_id, &outcome).unwrap();
        let job_dir = crate::util::job_dir(&root, &job.job_id);
        assert!(job_dir.join(crate::authoring_v2_commands::AUTHORING_V2_SHADOW_FILE).is_file());
        assert!(job_dir.join(DIRECT_RESULT_FILE).is_file());
        let recorded: Value = crate::util::read_json(&job_dir.join(DIRECT_RESULT_FILE)).unwrap();
        assert_eq!(recorded["calls"], json!(1));
        // 证据面纪律：请求带每页文字层与契约，不带本地节点目标。
        let request = service.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(request.contains("=== PAGE"), "request must carry per-page text layers");
        assert!(request.contains("You convert ONE IELTS Reading test PDF"));
        assert!(request.contains("TASK PRESENTATION RULES"));
        assert!(!request.contains("sourceParagraphs"), "direct chain must not send local targets");
        assert!(!request.contains("localNodeTargets"), "direct chain must not send local targets");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compile_failure_triggers_exactly_one_cloud_repair_round() {
        let root = temp_root("e2e-repair");
        crate::util::ensure_app_dirs(&root).unwrap();
        // 第一份：两个题组共用 taskId ⇒ RUNTIME_TASK_ID_DUPLICATE（编译错）。
        let duplicated = {
            let mut value: Value =
                serde_json::from_str(&direct_exam_with_prompt("Pearls are formed inside oysters.", "stop", &[1, 2])).unwrap();
            let mut exam: Value = serde_json::from_str(value["choices"][0]["message"]["content"].as_str().unwrap()).unwrap();
            let mut groups = exam["taskGroups"].as_array().unwrap().clone();
            groups.push(groups[0].clone());
            exam["taskGroups"] = json!(groups);
            value["choices"][0]["message"]["content"] = json!(serde_json::to_string(&exam).unwrap());
            value.to_string()
        };
        let service = controlled_service(vec![
            (duplicated, 200),
            (direct_exam_with_prompt("Pearls are formed inside oysters.", "stop", &[1, 2]), 200),
        ]);
        let job = direct_job_with_fixture(&root);
        let profile_id = controlled_profile(&root, &service.base_url);

        let outcome = run_exam_direct(&root, &job.job_id, Some(&profile_id))
            .expect("repair round must recover");

        assert_eq!(outcome.calls, 2, "编译错必须带错误清单修正好一次");
        assert!(outcome.compile_issues.is_empty(), "{:?}", outcome.compile_issues);
        let first = service.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = service.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(second.contains("REPAIR ROUND"), "修复回合必须带修复指令");
        assert!(second.contains("RUNTIME_TASK_ID_DUPLICATE"), "修复回合必须带编译错误清单");
        assert!(second.contains("CURRENT OUTPUT TO FIX"), "修复回合必须带当前 JSON");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finish_reason_length_retries_once_in_page_chunks() {
        // 10 个文字页的 fixture：截断后按页段对半切（两块）。
        let root = temp_root("e2e-chunked");
        crate::util::ensure_app_dirs(&root).unwrap();
        let service = controlled_service(vec![
            (direct_exam_with_prompt("Pearls.", "length", &[1, 2]), 200),
            (direct_exam_with_prompt("Pearls are formed inside oysters.", "stop", &[1, 2]), 200),
            (direct_exam_with_prompt("Pearls are formed inside oysters.", "stop", &[3, 4]), 200),
        ]);
        let job = direct_job_with_named_fixture(&root, "chunked-reading-evidence.pdf");
        let profile_id = controlled_profile(&root, &service.base_url);

        let outcome = run_exam_direct(&root, &job.job_id, Some(&profile_id))
            .expect("chunked retry must succeed");

        assert!(outcome.chunked);
        // 被截断的首次请求 + 两块 = 3 次（≤3 次/份，含截断那次）。
        assert_eq!(outcome.calls, 3, "截断后按页段分块重试一次（两块）");
        assert!(outcome.compile_issues.is_empty(), "{:?}", outcome.compile_issues);
        // 两块按题号拼接：槽位合并，正文取节点数最多的一份。
        assert_eq!(outcome.document["answerSlots"].as_object().unwrap().len(), 4);
        // 块间 id 不重号：两块都从 group-1 开始，合并后不得有重复 taskId。
        let task_ids: Vec<&str> = outcome.document["taskGroups"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|group| group["taskId"].as_str())
            .collect();
        let unique: std::collections::BTreeSet<&&str> = task_ids.iter().collect();
        assert_eq!(task_ids.len(), unique.len(), "块间 taskId 必须唯一: {task_ids:?}");
        let first = service.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        let chunk_a = service.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        let chunk_b = service.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!first.contains("page chunks"), "首次请求不带分块说明");
        assert!(chunk_a.contains("page chunks") && chunk_b.contains("page chunks"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn chunk_merge_rewrites_ids_so_chunks_do_not_collide() {
        // 两块都从 group-1 / passage-paragraph-A 开始编号：合并后 id 必须唯一，
        // 段前投放框主机必须重定向到正文块的真实段落。
        let chunk_a = json!({
            "meta": {"title": "Chunked"},
            "passage": {
                "content": [paragraph("passage-paragraph-A", Some("A"), "first half passage"),
                            paragraph("passage-paragraph-B", Some("B"), "second paragraph")],
                "paragraphMap": {"A": "passage-paragraph-A", "B": "passage-paragraph-B"}
            },
            "taskGroups": [{"taskId": "group-1", "taskType": "matching_headings",
                "displayRange": {"kind": "range", "start": 1, "end": 1},
                "instructions": [],
                "optionBank": {"optionBankId": "group-1-bank", "scope": "task_group", "allowReuse": false,
                    "options": [{"optionId": "group-1-opt-i", "label": "i", "content": [{"type": "text", "id": "group-1-opt-i-text", "text": "Heading i"}]}]},
                "responseGroups": [{"responseGroupId": "group-1-rg-1", "kind": "matching", "slotIds": ["q1"], "optionBankRef": "group-1-bank"}]}],
            "answerSlots": {"q1": {"slotId": "q1", "questionNumber": 1, "displayLabel": "1",
                "hostNodeId": "passage-paragraph-A", "hostType": "passage_paragraph", "interaction": "dragdrop"}},
            "answerKey": {"q1": {"kind": "option", "labels": ["i"]}},
            "warnings": []
        });
        let chunk_b = json!({
            "taskGroups": [{"taskId": "group-1", "taskType": "matching_headings",
                "displayRange": {"kind": "range", "start": 2, "end": 2},
                "instructions": [],
                "optionBank": {"optionBankId": "group-1-bank", "scope": "task_group", "allowReuse": false,
                    "options": [{"optionId": "group-1-opt-iv", "label": "iv", "content": [{"type": "text", "id": "group-1-opt-iv-text", "text": "Heading iv"}]}]},
                "responseGroups": [{"responseGroupId": "group-1-rg-1", "kind": "matching", "slotIds": ["q2"], "optionBankRef": "group-1-bank"}]}],
            "answerSlots": {"q2": {"slotId": "q2", "questionNumber": 2, "displayLabel": "2",
                "hostNodeId": "passage-paragraph-B", "hostType": "passage_paragraph", "interaction": "dragdrop"}},
            "answerKey": {"q2": {"kind": "option", "labels": ["iv"]}},
            "warnings": []
        });
        let merged = merge_direct_chunks(vec![chunk_a, chunk_b]);
        // 非正文块（chunk_b）的题组 id 加了前缀。
        let task_ids: Vec<&str> = merged["taskGroups"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|group| group["taskId"].as_str())
            .collect();
        assert_eq!(task_ids, vec!["group-1", "chunk1-group-1"], "{task_ids:?}");
        // 段前主机：chunk_a 的 q1 指向正文块自己的节点；chunk_b 的 q2 主机被
        // 重定向到正文块里同标签（B）的真实节点，而不是被丢弃的副本。
        assert_eq!(merged["answerSlots"]["q1"]["hostNodeId"], json!("passage-paragraph-A"));
        assert_eq!(merged["answerSlots"]["q2"]["hostNodeId"], json!("passage-paragraph-B"));
        // 合并结果可以归一化并编译（不再有重复 taskId）。
        let root = temp_root("merge-normalize");
        let document = normalize_direct_output(&root, &merged, &identity()).unwrap();
        let issues = compile_document(&document);
        assert!(issues.is_empty(), "{issues:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn stale_worker_cannot_seed_or_replace_canonical() {
        use crate::library::repository::*;
        let root = temp_root("stale-worker");
        crate::util::ensure_app_dirs(&root).unwrap();
        let conn = open_library_connection(&root).unwrap();
        upsert_item_shell(&conn, &UpsertItemInput { id: "j", modality: "reading", title: "t", status: "processing", source_asset_id: None }).unwrap();
        crate::processing::queue::enqueue(&conn, "j", "j", "a", &Value::Null).unwrap();
        crate::processing::queue::claim_next(&conn, "new-worker").unwrap().unwrap();
        let document = normalize_direct_output(&root, &json!({
            "passage": {"content": [paragraph("p", None, "text")]},
            "taskGroups": [{"taskId":"g", "taskType":"short_answer", "questions":[]}],
            "answerSlots": {}, "answerKey": {}
        }), &identity()).unwrap();
        assert_eq!(write_direct_canonical_leased(&root,"j",&document,"old-worker").unwrap_err(), "processing_lease_lost");
        assert!(get_canonical_ds(&conn,"j").unwrap().is_none());
        write_direct_canonical_leased(&root,"j",&document,"new-worker").unwrap();
        let before = get_canonical_ds(&conn,"j").unwrap();
        assert_eq!(write_direct_canonical_leased(&root,"j",&document,"old-worker").unwrap_err(), "processing_lease_lost");
        assert_eq!(get_canonical_ds(&conn,"j").unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn direct_quality_skips_local_evidence_but_keeps_actionable_warnings() {
        let root = temp_root("quality-direct");
        crate::util::ensure_app_dirs(&root).unwrap();
        for (name, example) in direct_examples() {
            let mut document = normalize_direct_output(&root, &example, &identity()).unwrap();
            let quality = crate::ielts_grammar::quality::evaluate_quality(&document, None);
            for issue in quality["issues"].as_array().unwrap() {
                assert!(!matches!(issue["code"].as_str().unwrap(), "PHYSICAL_SHADOW_MISSING" | "SOURCE_QUESTION_COVERAGE_UNDETERMINED" | "PROVENANCE_MISSING" | "INSTRUCTION_PROVENANCE_MISSING" | "INSTRUCTION_SIGNATURE_EVIDENCE_MISSING"), "{name}: {issue}");
            }
            document["quality"] = quality;
            let gate = crate::authoring_v2_commands::check_publish_preflight(&root, &identity().job_id, 1, &document);
            assert!(gate["warnings"].as_array().unwrap().len() < 10, "{name}: {gate}");
            assert!(!gate["warnings"].as_array().unwrap().iter().any(|w| w["code"].as_str().unwrap().starts_with("SOURCE_REVIEW") || w["code"].as_str().unwrap().starts_with("RECOGNITION_CHOICE")));
            let first = document["answerKey"].as_object().unwrap().keys().next().unwrap().clone();
            document["answerKey"][&first] = json!({"kind":"unresolved"});
            let gate = crate::authoring_v2_commands::check_publish_preflight(&root, &identity().job_id, 1, &document);
            assert!(gate["warnings"].as_array().unwrap().iter().any(|w| w["code"] == "ANSWER_MISSING"));
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn re_recognition_replaces_the_canonical_document() {
        use crate::library::repository::{get_canonical_ds, open_library_connection, seed_canonical_ds, upsert_item_shell, UpsertItemInput};
        let root = temp_root("e2e-replace");
        crate::util::ensure_app_dirs(&root).unwrap();
        let job = direct_job_with_fixture(&root);
        // 1) 先播种一份旧稿（模拟上一轮识别/直出的正式稿）。
        let conn = open_library_connection(&root).unwrap();
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: &job.job_id,
                modality: "reading",
                title: "Fixture",
                status: "action_required",
                source_asset_id: None,
            },
        )
        .unwrap();
        let old: Value = serde_json::from_str(
            &direct_exam_with_prompt("Old passage text.", "stop", &[9, 10]),
        )
        .unwrap();
        let old_exam: Value =
            serde_json::from_str(old["choices"][0]["message"]["content"].as_str().unwrap()).unwrap();
        let mut old_document = old_exam;
        old_document["audit"] = json!({
            "revision": 0, "source": "auto_extract", "humanVerified": false,
            "llmUsed": false, "updatedAt": "2026-10-14T00:00:00Z", "notes": []
        });
        seed_canonical_ds(&conn, &job.job_id, &old_document.to_string(), "action_required").unwrap();
        drop(conn);
        // 2) 直出稿替换。
        let service = controlled_service(vec![(
            direct_exam_with_prompt("New pearls passage.", "stop", &[1, 2]),
            200,
        )]);
        let profile_id = controlled_profile(&root, &service.base_url);
        let outcome =
            run_exam_direct(&root, &job.job_id, Some(&profile_id)).expect("direct chain must succeed");
        let version = write_direct_canonical(&root, &job.job_id, &outcome.document)
            .expect("replace must succeed");
        // 3) 权威稿必须是新直出稿，而不是旧稿。
        let conn = open_library_connection(&root).unwrap();
        let (ds, current) = get_canonical_ds(&conn, &job.job_id).unwrap().unwrap();
        assert_eq!(current, version);
        assert_eq!(ds["audit"]["source"], json!("cloud_direct"));
        assert_eq!(ds["audit"]["revision"], json!(1), "revision 必须在旧稿上进位");
        assert_eq!(ds["audit"]["humanVerified"], json!(false), "新内容不继承旧确认");
        assert_eq!(
            ds["answerSlots"].as_object().unwrap().len(),
            2,
            "必须是新直出稿的答案位，不是旧稿的"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unusable_output_falls_back_to_the_local_chain() {
        let root = temp_root("e2e-unusable");
        crate::util::ensure_app_dirs(&root).unwrap();
        // 没有题组的「直出」= 完全不可用：归一化必须拒绝（调用方回退本地链）。
        let service = controlled_service(vec![(
            json!({"choices": [{"message": {"content": serde_json::to_string(&json!({
                "meta": {"title": "x", "language": "en"},
                "passage": {"content": [paragraph("p", None, "text")], "paragraphMap": {}},
                "taskGroups": [],
                "answerSlots": {},
                "answerKey": {},
                "warnings": []
            })).unwrap()}, "finish_reason": "stop"}],
            "usage": {"total_tokens": 50}}).to_string(),
            200,
        )]);
        let job = direct_job_with_fixture(&root);
        let profile_id = controlled_profile(&root, &service.base_url);

        let error = run_exam_direct(&root, &job.job_id, Some(&profile_id)).unwrap_err();
        assert_eq!(error, "cloud_direct_no_task_groups");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn non_json_reply_retries_once_with_a_format_note() {
        // 只补不拒的「重试一次」：模型回了散文/围栏代码而不是 JSON 时，
        // 带一条格式纠正说明再要一次；仍不是 JSON 才回退。
        let root = temp_root("e2e-format-retry");
        crate::util::ensure_app_dirs(&root).unwrap();
        let service = controlled_service(vec![
            (
                json!({"choices": [{"message": {"content": "I'm sorry, I can't do that."}, "finish_reason": "stop"}], "usage": {"total_tokens": 30}}).to_string(),
                200,
            ),
            (
                direct_exam_with_prompt("Pearls are formed inside oysters.", "stop", &[1, 2]),
                200,
            ),
        ]);
        let job = direct_job_with_fixture(&root);
        let profile_id = controlled_profile(&root, &service.base_url);

        let outcome = run_exam_direct(&root, &job.job_id, Some(&profile_id))
            .expect("格式重试一次后必须成功");

        // 失败的首次请求 + 成功的重试 = 2 次真实 HTTP 调用（预算据此封顶）。
        assert_eq!(outcome.calls, 2, "解析失败重试恰好一次");
        assert_eq!(outcome.document["answerKey"]["q1"]["labels"][0], json!("TRUE"));
        let first = service.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        let retry = service.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!first.contains("FORMAT NOTE"), "首次请求不带格式说明");
        assert!(retry.contains("FORMAT NOTE"), "重试必须带格式纠正说明");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn non_json_twice_falls_back_instead_of_looping() {
        let root = temp_root("e2e-format-twice");
        crate::util::ensure_app_dirs(&root).unwrap();
        let service = controlled_service(vec![
            (
                json!({"choices": [{"message": {"content": "nope"}, "finish_reason": "stop"}], "usage": {"total_tokens": 10}}).to_string(),
                200,
            ),
            (
                json!({"choices": [{"message": {"content": "```json\\nnope```"}, "finish_reason": "stop"}], "usage": {"total_tokens": 10}}).to_string(),
                200,
            ),
        ]);
        let job = direct_job_with_fixture(&root);
        let profile_id = controlled_profile(&root, &service.base_url);
        let error = run_exam_direct(&root, &job.job_id, Some(&profile_id)).unwrap_err();
        assert!(error.starts_with("llm_json_parse_failed:"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// choose-TWO（multiple_choice → unordered_set）+ 共享受项库的最小直出产物。
    fn choose_two_raw() -> Value {
        json!({
            "meta": {"title": "Choose TWO", "language": "en", "category": "P1"},
            "passage": {
                "content": [paragraph("passage-paragraph-A", Some("A"), "Paragraph A about pearls.")],
                "paragraphMap": {"A": "passage-paragraph-A"}
            },
            "taskGroups": [{
                "taskId": "group-1",
                "taskType": "multiple_choice",
                "displayRange": {"kind": "range", "start": 1, "end": 2},
                "instructions": [{"type": "paragraph", "id": "group-1-instr", "children": [
                    {"type": "text", "id": "group-1-instr-text", "text": "Questions 1-2 Choose TWO letters A-F. Write the correct letters in boxes 1-2 on your answer sheet."}
                ]}],
                "optionBank": {
                    "optionBankId": "group-1-bank",
                    "scope": "task_group",
                    "allowReuse": false,
                    "options": [
                        {"optionId": "group-1-opt-a", "label": "A", "content": [{"type": "text", "id": "group-1-opt-a-text", "text": "Option A"}]},
                        {"optionId": "group-1-opt-b", "label": "B", "content": [{"type": "text", "id": "group-1-opt-b-text", "text": "Option B"}]},
                        {"optionId": "group-1-opt-c", "label": "C", "content": [{"type": "text", "id": "group-1-opt-c-text", "text": "Option C"}]},
                        {"optionId": "group-1-opt-d", "label": "D", "content": [{"type": "text", "id": "group-1-opt-d-text", "text": "Option D"}]},
                        {"optionId": "group-1-opt-e", "label": "E", "content": [{"type": "text", "id": "group-1-opt-e-text", "text": "Option E"}]}
                    ]
                },
                "responseGroups": [{
                    "responseGroupId": "group-1-rg-1",
                    "kind": "choice",
                    "slotIds": ["q1", "q2"],
                    "optionBankRef": "group-1-bank",
                    "prompt": [{"type": "paragraph", "id": "group-1-prompt", "children": [
                        {"type": "text", "id": "group-1-prompt-text", "text": "Choose TWO letters A-F."}
                    ]}]
                }]
            }],
            "answerSlots": {
                "q1": {"slotId": "q1", "questionNumber": 1, "displayLabel": "1", "hostNodeId": "group-1-prompt", "hostType": "prompt", "interaction": "checkbox"},
                "q2": {"slotId": "q2", "questionNumber": 2, "displayLabel": "2", "hostNodeId": "group-1-prompt", "hostType": "prompt", "interaction": "checkbox"}
            },
            // 集合题答案：每个槽位恰好一个正确字母，assignment=unordered_set。
            "answerKey": {
                "q1": {"kind": "option", "labels": ["A"], "assignment": "unordered_set"},
                "q2": {"kind": "option", "labels": ["D"], "assignment": "unordered_set"}
            },
            "warnings": []
        })
    }

    #[test]
    fn choose_two_defaults_follow_compiler_set_policy_and_cardinality() {
        let root = temp_root("normalize-choose-two");
        let document =
            normalize_direct_output(&root, &choose_two_raw(), &identity()).expect("choose-TWO 必须能归一化");
        let response = &document["taskGroups"][0]["responseGroups"][0];
        // 集合题缺省：编译器硬规则（与契约的 exact_set 写法冲突时以编译器为准，
        // 见归一化处注释）——scoringPolicy=per_slot_ielts_normalized；
        // cardinality min/max/exact = 槽位数（要选的字母数）。
        assert_eq!(response["assignment"], json!("unordered_set"));
        assert_eq!(response["scoringPolicy"], json!("per_slot_ielts_normalized"));
        assert_eq!(response["cardinality"]["min"], json!(2));
        assert_eq!(response["cardinality"]["max"], json!(2));
        assert_eq!(response["cardinality"]["exact"], json!(2));
        // 每个槽位一条集合答案（assignment 与 labels 保真）。
        assert_eq!(document["answerKey"]["q1"]["labels"][0], json!("A"));
        assert_eq!(document["answerKey"]["q1"]["assignment"], json!("unordered_set"));
        assert_eq!(document["answerKey"]["q2"]["labels"][0], json!("D"));
        // 编译通过：选项数（5）> 槽位数（2），答案键策略满足。
        let issues = compile_document(&document);
        assert!(issues.is_empty(), "{issues:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn choose_two_contract_shape_is_distributed_to_compiler_shape() {
        // 契约（system.md）让模型给每个槽位写「完整正确字母集合」、scoringPolicy
        // 写 exact_set；编译器要求每槽位恰好一个唯一字母 + per_slot_ielts_normalized。
        // 归一化负责对齐两者：字母按首见顺序分发，记分策略强制为编译器规则。
        let root = temp_root("normalize-choose-two-contract-shape");
        let mut raw = choose_two_raw();
        raw["taskGroups"][0]["responseGroups"][0]["scoringPolicy"] = json!("exact_set");
        raw["answerKey"]["q1"]["labels"] = json!(["A", "D"]);
        raw["answerKey"]["q2"]["labels"] = json!(["A", "D"]);
        let document = normalize_direct_output(&root, &raw, &identity()).unwrap();
        assert_eq!(document["answerKey"]["q1"]["labels"], json!(["A"]));
        assert_eq!(document["answerKey"]["q2"]["labels"], json!(["D"]));
        let response = &document["taskGroups"][0]["responseGroups"][0];
        assert_eq!(response["scoringPolicy"], json!("per_slot_ielts_normalized"));
        let issues = compile_document(&document);
        assert!(issues.is_empty(), "{issues:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn choose_two_with_too_few_letters_marks_the_rest_unresolved() {
        // 模型只给一个字母（2 槽位 choose-TWO 需要 2 个）：第一槽位拿到它，
        // 第二槽位如实标 unresolved——复制同一个字母冒充第二个答案就是伪造内容。
        // 编译器的集合题策略会把它报成待修问题（RUNTIME_ANSWER_KEY_POLICY_INVALID）。
        let root = temp_root("normalize-choose-two-missing");
        let mut raw = choose_two_raw();
        raw["answerKey"] = json!({
            "q1": {"kind": "option", "labels": ["A"], "assignment": "unordered_set"}
        });
        let document = normalize_direct_output(&root, &raw, &identity()).unwrap();
        assert_eq!(document["answerKey"]["q1"]["labels"], json!(["A"]));
        assert_eq!(document["answerKey"]["q2"]["kind"], json!("unresolved"));
        let issues = compile_document(&document);
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "RUNTIME_ANSWER_KEY_POLICY_INVALID"),
            "{issues:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn matching_information_materializes_options_from_paragraph_map() {
        // A7-3 端到端确认：matching_information 的选项由编译器从 paragraphMap
        // 生成（上一轮修的兜底），直出链只要保证段落图指向真实段落即可。
        let root = temp_root("normalize-matching-info");
        let raw = json!({
            "meta": {"title": "Which paragraph", "language": "en", "category": "P1"},
            "passage": {
                "content": [
                    paragraph("passage-paragraph-A", Some("A"), "Paragraph A text."),
                    paragraph("passage-paragraph-B", Some("B"), "Paragraph B text.")
                ],
                "paragraphMap": {"A": "passage-paragraph-A", "B": "passage-paragraph-B"}
            },
            "taskGroups": [{
                "taskId": "group-1",
                "taskType": "matching_information",
                "displayRange": {"kind": "range", "start": 1, "end": 2},
                "instructions": [{"type": "paragraph", "id": "group-1-instr", "children": [
                    {"type": "text", "id": "group-1-instr-text", "text": "Questions 1-2 Which paragraph contains the following information? Write the correct letter A-F in boxes 1-2 on your answer sheet."}
                ]}],
                "responseGroups": [
                    {"responseGroupId": "group-1-rg-1", "kind": "matching", "slotIds": ["q1"]},
                    {"responseGroupId": "group-1-rg-2", "kind": "matching", "slotIds": ["q2"]}
                ]
            }],
            "answerSlots": {
                "q1": {"slotId": "q1", "questionNumber": 1, "displayLabel": "1", "hostNodeId": "passage-paragraph-A", "hostType": "passage_paragraph", "interaction": "dragdrop"},
                "q2": {"slotId": "q2", "questionNumber": 2, "displayLabel": "2", "hostNodeId": "passage-paragraph-B", "hostType": "passage_paragraph", "interaction": "dragdrop"}
            },
            "answerKey": {
                "q1": {"kind": "option", "labels": ["A"], "assignment": "per_slot"},
                "q2": {"kind": "option", "labels": ["B"], "assignment": "per_slot"}
            },
            "warnings": []
        });
        let document = normalize_direct_output(&root, &raw, &identity()).unwrap();
        // 编译通过且拿到完整 question_order / display map（A7-1）：二者由编译器从
        // answerSlots 推导，归一化保证每个槽位有题号与展示标签。
        let typed = serde_json::from_value::<crate::schema::IeltsAuthoringIRV2>(document.clone())
            .expect("归一化产物必须反序列化");
        let compiled = crate::listening_source_v1::compile_exam_source_v2(&typed)
            .expect("matching_information 直出产物必须编译通过");
        let crate::listening_source_v1::CompiledExamSourceV2::Reading(source) = compiled else {
            panic!("阅读稿必须编译出阅读契约");
        };
        assert_eq!(source.question_order, vec!["q1".to_string(), "q2".to_string()]);
        assert_eq!(source.question_display_map["q1"], "1");
        assert_eq!(source.question_display_map["q2"], "2");
        let _ = std::fs::remove_dir_all(root);
    }
}

/// 真实模型的命令层往返验证（默认 `#[ignore]`，需本机真实配置与网络）。
///
/// 产品级验收要走 WebView2（自动化会话里受环境限制），而「直出链用**真实模型**
/// 能产出可编译题稿」这件事**不依赖界面**：本模块直接调用 `run_exam_direct`，
/// 用本机真实配置（profile 从用户配置复制到临时作业根；API key 由 Windows
/// 凭据管理器按 profileId 读取，**不打印、不落盘**），对 5 份样本 PDF 各跑一次。
///
/// 运行：
///   cargo test --lib real_deepseek -- --ignored --nocapture
///
/// 产物：`artifacts/qa/cloud-direct-real-model.json`（每份 PDF 的耗时 / 调用次数 /
/// token / 编译问题数 / 正文覆盖率 / 答案键与题数），供 QA 报告引用。
#[cfg(test)]
mod real_model_tests {
    use super::*;

    const SAMPLE_PDFS: [&str; 5] = [
        "24. P1 - The Pearls 珍珠.pdf",
        "107. P2 - A study of western celebrity 西方名人.pdf",
        "100. P3 - What makes a musical expert_ 音乐天赋.pdf",
        "10. P1 - Maori Fish Hooks 毛利鱼钩.pdf",
        "1. P1 - A Brief History of Tea 茶叶简史.pdf",
    ];

    fn sample_pdf_dir() -> std::path::PathBuf {
        std::env::var("PDF2TEST_REAL_MODEL_PDF_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::path::PathBuf::from("F:/workspace/IELTS Atlas/ReadingPractice/PDF")
            })
    }

    /// 真实应用数据根（`%APPDATA%/com.ielts.author.studio`）。
    fn real_app_root() -> Option<std::path::PathBuf> {
        let appdata = std::env::var("APPDATA").ok()?;
        let root = std::path::PathBuf::from(appdata).join("com.ielts.author.studio");
        root.is_dir().then_some(root)
    }

    fn report_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root")
            .join("artifacts")
            .join("qa")
            .join("cloud-direct-real-model.json")
    }

    /// 用量总计（`llm-token-usage-total.json` 是 camelCase：promptTokens /
    /// completionTokens / cacheHitTokens…）。返回 (prompt, completion, cacheHit, calls)。
    fn usage_totals(job_dir: &std::path::Path) -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
        let Ok(value) = crate::util::read_json::<Value>(&job_dir.join("llm-token-usage-total.json")) else {
            return (None, None, None, None);
        };
        let read = |key: &str| value.get(key).and_then(Value::as_u64);
        (
            read("promptTokens"),
            read("completionTokens"),
            read("cacheHitTokens"),
            read("callCount"),
        )
    }

    /// 真实模型直出：5 份样本 PDF 各一次，逐份记录指标并落盘汇总。
    ///
    /// 这条测试**不预设**模型一定成功：它记录事实（失败也如实记录 error），
    /// 只有「一份都没成功」才判失败——那说明链路或配置有问题，不是模型波动。
    #[test]
    #[ignore = "需要本机真实 DeepSeek 配置与网络；用 --ignored 显式运行"]
    fn real_deepseek_direct_round_trip_on_sample_pdfs() {
        let pdf_dir = sample_pdf_dir();
        if !pdf_dir.is_dir() {
            println!("SKIP: 样本 PDF 目录不存在：{}", pdf_dir.display());
            return;
        }
        let Some(real_root) = real_app_root() else {
            println!("SKIP: 找不到真实应用数据目录（%APPDATA%/com.ielts.author.studio）");
            return;
        };
        let profiles_path = real_root.join("config").join("llm-profiles.json");
        let Ok(profiles) = crate::util::read_json::<Vec<Value>>(&profiles_path) else {
            println!("SKIP: 读不到真实 profile：{}", profiles_path.display());
            return;
        };
        let Some(profile) = profiles.iter().find(|profile| {
            profile
                .get("baseUrl")
                .and_then(Value::as_str)
                .is_some_and(|url| url.contains("deepseek"))
        }) else {
            println!("SKIP: 真实配置里没有 DeepSeek profile");
            return;
        };
        let profile_id = profile
            .get("profileId")
            .and_then(Value::as_str)
            .expect("profile 必须有 profileId")
            .to_string();

        // 临时作业根：把 profile 复制过去（去掉任何内联密钥字段，密钥只走凭据管理器）。
        let root = std::env::temp_dir().join(format!(
            "pdf2test-real-model-{}",
            uuid::Uuid::new_v4().simple()
        ));
        crate::util::ensure_app_dirs(&root).expect("ensure app dirs");
        let mut sanitized = profiles.clone();
        for entry in sanitized.iter_mut() {
            if let Some(object) = entry.as_object_mut() {
                object.remove("apiKey");
                object.remove("api_key");
            }
        }
        crate::llm_profiles::save_profiles(&root, &sanitized).expect("save profiles");
        // 密钥可用性预检：拿不到就明确跳过（而不是让 5 次调用各自失败）。
        if crate::llm_profiles::load_llm_api_key(&root, &profile_id).is_none() {
            println!(
                "SKIP: 凭据管理器里没有 profile {} 的密钥（应用里重新保存一次该连接即可）",
                profile_id
            );
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        let model = profile
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("(unset)")
            .to_string();
        println!("[real-model] profile={profile_id} model={model}");

        let mut results: Vec<Value> = Vec::new();
        let mut succeeded = 0usize;
        for file_name in SAMPLE_PDFS {
            let source_path = pdf_dir.join(file_name);
            if !source_path.is_file() {
                println!("[real-model] 跳过（样本不存在）：{file_name}");
                results.push(json!({"pdf": file_name, "skipped": "sample_missing"}));
                continue;
            }
            let title = file_name.trim_end_matches(".pdf").to_string();
            let job = crate::job_store::make_job(crate::CreateJobInput {
                title: Some(title.clone()),
                category: None,
                frequency: None,
                tags: None,
                llm_profile_id: Some(profile_id.clone()),
            });
            crate::job_store::save_job(&root, &job).expect("save job");
            let job_dir = crate::util::job_dir(&root, &job.job_id);
            std::fs::create_dir_all(job_dir.join("uploads")).expect("uploads dir");
            let stored_name = "source.pdf";
            std::fs::copy(&source_path, job_dir.join("uploads").join(stored_name))
                .expect("copy sample pdf");
            let size_bytes = std::fs::metadata(&source_path).map(|meta| meta.len()).unwrap_or(0);
            let mut job = job;
            job.source_files = vec![crate::SourceFile {
                file_id: "src-1".to_string(),
                original_name: file_name.to_string(),
                stored_name: stored_name.to_string(),
                file_type: "pdf".to_string(),
                sha256: "0".repeat(64),
                size_bytes,
                role: "MainQuestion".to_string(),
                imported_at: job.created_at,
            }];
            crate::job_store::save_job(&root, &job).expect("save job with source");

            let started = std::time::Instant::now();
            let outcome = crate::cloud_direct::run_exam_direct(&root, &job.job_id, Some(&profile_id));
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let (prompt_tokens, completion_tokens, cache_hit_tokens, call_count) = usage_totals(&job_dir);
            let usage = json!({
                "promptTokens": prompt_tokens,
                "completionTokens": completion_tokens,
                "cacheHitTokens": cache_hit_tokens,
                "totalTokens": prompt_tokens.zip(completion_tokens).map(|(p, c)| p + c),
                "callCount": call_count,
            });
            match outcome {
                Ok(outcome) => {
                    succeeded += 1;
                    let entry = json!({
                        "pdf": file_name,
                        "jobId": job.job_id,
                        "ok": true,
                        "durationMs": elapsed_ms,
                        "calls": outcome.calls,
                        "chunked": outcome.chunked,
                        "tokens": usage,
                        "compileIssueCount": outcome.compile_issues.len(),
                        "compileIssues": outcome.compile_issues.iter().map(|issue| json!({
                            "code": issue.code, "targetId": issue.target_id
                        })).collect::<Vec<_>>(),
                        "coverage": outcome.fidelity.as_ref().and_then(|report| report.get("overallCoverage")).cloned(),
                        "flaggedParagraphs": outcome.fidelity.as_ref().and_then(|report| report.get("flaggedParagraphIds")).cloned(),
                        "taskGroups": outcome.document.get("taskGroups").and_then(Value::as_array).map(Vec::len),
                        "slots": outcome.document.get("answerSlots").and_then(Value::as_object).map(|map| map.len()),
                        "answerKey": outcome.document.get("answerKey").cloned(),
                        "warnings": outcome.warnings,
                    });
                    println!(
                        "[real-model] {} => ok calls={} tokens={:?} issues={} coverage={:?} slots={:?} {}ms",
                        file_name,
                        entry["calls"],
                        usage,
                        entry["compileIssueCount"],
                        entry["coverage"],
                        entry["slots"],
                        elapsed_ms
                    );
                    results.push(entry);
                }
                Err(error) => {
                    println!("[real-model] {} => ERR {error} ({elapsed_ms}ms)", file_name);
                    results.push(json!({
                        "pdf": file_name,
                        "jobId": job.job_id,
                        "ok": false,
                        "error": error,
                        "durationMs": elapsed_ms,
                        "tokens": usage,
                    }));
                }
            }
        }

        let summary = json!({
            "schemaVersion": "CloudDirectRealModelRunV1",
            "generatedAt": chrono::Utc::now().to_rfc3339(),
            "profileId": profile_id,
            "model": model,
            "samplePdfDir": pdf_dir.to_string_lossy(),
            "pdfCount": SAMPLE_PDFS.len(),
            "succeeded": succeeded,
            "results": results,
        });
        let path = report_path();
        std::fs::create_dir_all(path.parent().expect("qa dir")).expect("create qa dir");
        crate::util::write_json(&path, &summary).expect("write real-model report");
        println!("[real-model] report: {}", path.display());
        let _ = std::fs::remove_dir_all(&root);
        assert!(succeeded > 0, "至少一份样本必须成功（否则是链路/配置问题，不是模型波动）");
    }
}
