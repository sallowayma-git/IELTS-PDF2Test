//! 云端候选文本与原卷文本层的本地对齐校验（纯计算，绝不调用云端）。
//!
//! 云端识别在断行、说明/题目边界、题型上通常好于本地，产品决定让云端为主、整体覆盖本地稿；
//! 但覆盖前必须证明「云端内容确实对得上原卷」。本模块就是那道零成本门禁：把云端文本按句子
//! 在原卷文本层里定位，给出命中相似度、命中的源区域，并据此合成真实的 `sourceAnchors`
//! （采纳阶段据此把云端节点的 provenance 盖成 source），再做结构级判定（顺序单调、长度比例、
//! 显著区域覆盖）与一份给人复核的确定性抽查样本。
//!
//! 匹配在「空格无关」的归一 key 上做：先统一大小写/引号/破折号/省略号，折叠空白，按行尾连字符
//! 去断行，再抹掉所有空白。这样 pdfium 的字符间距（`D o t h e`）与普通排版空格差异一并消解，
//! 云端整句多数能落成精确子串；落不上的走滑窗相似度回退。

// 采纳策略（cloud_adoption）已接入公共入口；少数内部字段仅测试/未来接入引用。
#![allow(dead_code)]

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// 对齐/采纳阈值。**云端为主**：顺序对、文本大体相似就直接采用云端，修复循环只处理确有
/// 必要的目标。所有可调门槛集中在此，不散落到各处。
#[derive(Debug, Clone)]
pub(crate) struct AlignmentConfig {
    /// 单句判为命中的最低相似度。命中即视为「小差异」，直接采用云端。
    pub sentence_threshold: f64,
    /// 低于此相似度的句子判为「整句对不上 / 凭空句」（大差异），单列为修复目标。
    pub invented_threshold: f64,
    /// 原文整体采纳时容忍的凭空句上限：个数不超过此值。
    pub invented_max_count: usize,
    /// 原文整体采纳时容忍的凭空句上限：占原文句总数的比例不超过此值。
    pub invented_max_ratio: f64,
    /// 原文节点整体采用云端所需的命中句比例。
    pub passage_pass_ratio: f64,
    /// 题组整体采用云端所需的内容对齐比例（命中句 / 题目内容句总数）。
    pub group_align_ratio: f64,
    /// 云端原文可读长度 / 原卷原文可读长度的允许下、上限（防截断、防冗余注水）。
    pub length_ratio_min: f64,
    pub length_ratio_max: f64,
    /// 显著源区域被覆盖的最低比例。
    pub coverage_min: f64,
    /// 命中位置按阅读顺序递增的最低比例（最长非降子序列 / 命中总数）。顺序错是结构性问题，
    /// 门槛不随其它阈值放宽。
    pub order_pass_ratio: f64,
    /// 抽查种子（由 sourceSha256 + batchId 派生，保证可复现）。
    pub sample_seed: u64,
    /// 每个节点抽查的句子上限。
    pub sample_per_node: usize,
}

impl Default for AlignmentConfig {
    fn default() -> Self {
        Self {
            sentence_threshold: 0.8,
            invented_threshold: 0.6,
            invented_max_count: 2,
            invented_max_ratio: 0.03,
            passage_pass_ratio: 0.85,
            group_align_ratio: 0.8,
            length_ratio_min: 0.5,
            length_ratio_max: 1.5,
            coverage_min: 0.7,
            order_pass_ratio: 0.95,
            sample_seed: 0,
            sample_per_node: 3,
        }
    }
}

// ---------------------------------------------------------------------------
// 归一
// ---------------------------------------------------------------------------

/// 统一大小写、引号、破折号、省略号，并把所有空白折叠成单个空格。保留可读词边界，
/// 供句子切分与长度统计使用。
fn normalize_readable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = false;
    for ch in text.chars() {
        let mapped = match ch {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' | '`' | '\u{00B4}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' | '\u{00AB}' | '\u{00BB}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{00AD}' => '-',
            '\u{00A0}' | '\u{2007}' | '\u{202F}' | '\u{2009}' | '\u{200A}' | '\u{2002}'
            | '\u{2003}' | '\t' => ' ',
            _ => ch,
        };
        if mapped == '…' {
            // 省略号展开成三点，避免与普通句点切分冲突。
            out.push_str("...");
            prev_space = false;
            continue;
        }
        if mapped.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.extend(mapped.to_lowercase());
            prev_space = false;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// 空格无关的匹配 key：抹掉全部空白。字符间距差异（`d o t h e`）与普通空格在此一并消失。
fn compact_key(readable: &str) -> String {
    readable.chars().filter(|ch| !ch.is_whitespace()).collect()
}

// ---------------------------------------------------------------------------
// 原卷文本层模型
// ---------------------------------------------------------------------------

/// 原卷里一个可对齐的最小源单元（一般是一行；带上其所属区域的真实锚点）。
struct SourceUnit {
    /// 归一 key 流中的 [start, end)。
    range: (usize, usize),
    /// 该单元的真实 SourceAnchorV2（复用原卷区域/行自带的锚点；缺失时合成）。
    anchor: Value,
    /// 命中该单元后计入云端节点 nodeIds 的源节点 id（区域 id 与行 id）。
    source_node_ids: Vec<String>,
    page_index: i64,
    /// 是否为「显著」内容（正文/标题/列表/表格等；页眉页脚图注不计）。
    significant: bool,
    /// 折叠空白后的可读长度，用于长度比例统计。
    readable_len: usize,
}

/// 原卷文本层（DocumentIRV2）的对齐视图。没有文本层时 [`build_source_model`] 返回 `None`。
struct SourceModel {
    /// 全文空格无关 key（按阅读顺序拼接所有源单元），以字符序列存放便于滑窗与切片。
    key: Vec<char>,
    units: Vec<SourceUnit>,
    total_readable_len: usize,
    significant_ids: BTreeSet<String>,
}

/// 文本承载型区域（其文本参与显著覆盖判定）。非正文的页眉页脚、页码、图片等排除在外；
/// 其余一律按显著处理，以适配不同来源对区域类型的不同拼写。
fn is_significant_region_kind(kind: &str) -> bool {
    let lower = kind.to_ascii_lowercase();
    !matches!(
        lower.as_str(),
        "figure"
            | "image"
            | "picture"
            | "pageheader"
            | "page_header"
            | "header"
            | "pagefooter"
            | "page_footer"
            | "footer"
            | "pagenumber"
            | "page_number"
            | "whitespace"
            | "decoration"
            | "background"
            | "artifact"
    )
}

/// 顶层来源文件 id 与哈希（供合成锚点用；缺失时留空字符串）。
fn source_file_identity(document_ir: &Value) -> (String, String) {
    for key in ["sourceFiles", "source_files"] {
        if let Some(file) = document_ir
            .get(key)
            .and_then(Value::as_array)
            .and_then(|files| files.first())
        {
            let id = file
                .get("sourceFileId")
                .or_else(|| file.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let hash = file
                .get("sourceHash")
                .or_else(|| file.get("hash"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            return (id, hash);
        }
    }
    (String::new(), String::new())
}

/// 从 DocumentIRV2 物理层构建对齐视图。返回 `None` 表示没有可用文本层（扫描卷）。
fn build_source_model(document_ir: &Value) -> Option<SourceModel> {
    if document_ir.get("schemaVersion").and_then(Value::as_str) != Some("DocumentIRV2") {
        return None;
    }
    let pages = document_ir.get("pages").and_then(Value::as_array)?;
    let (file_id, file_hash) = source_file_identity(document_ir);

    let mut key: Vec<char> = Vec::new();
    let mut units: Vec<SourceUnit> = Vec::new();
    let mut significant_ids = BTreeSet::new();
    let mut total_readable_len = 0usize;

    for page in pages {
        let page_index = page
            .get("pageIndex")
            .or_else(|| page.get("page_index"))
            .and_then(Value::as_i64)
            .unwrap_or(0);

        // 行 → 所属区域（区域携带真实锚点与显著性）。
        let mut line_region: BTreeMap<String, (String, Option<Value>, bool)> = BTreeMap::new();
        for region in page
            .get("regions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let region_id = region.get("id").and_then(Value::as_str).unwrap_or("");
            let kind = region.get("kind").and_then(Value::as_str).unwrap_or("");
            let significant = is_significant_region_kind(kind);
            let anchor = region
                .get("sourceAnchors")
                .or_else(|| region.get("source_anchors"))
                .and_then(Value::as_array)
                .and_then(|anchors| anchors.first())
                .cloned();
            for child in region
                .get("childLineIds")
                .or_else(|| region.get("child_line_ids"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                line_region.insert(
                    child.to_string(),
                    (region_id.to_string(), anchor.clone(), significant),
                );
            }
        }

        let mut ordered: Vec<(u64, usize, &Value)> = page
            .get("lines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(idx, line)| {
                let order = line
                    .get("sourceOrder")
                    .or_else(|| line.get("source_order"))
                    .and_then(Value::as_u64)
                    .unwrap_or(idx as u64);
                (order, idx, line)
            })
            .collect();
        ordered.sort_by_key(|(order, idx, _)| (*order, *idx));

        for (_, _, line) in ordered {
            let Some(raw) = line.get("text").and_then(Value::as_str) else {
                continue;
            };
            let line_id = line.get("id").and_then(Value::as_str).unwrap_or("");
            let readable = normalize_readable(raw);
            if readable.is_empty() {
                continue;
            }
            // 行尾连字符是软断行：去掉后与下一行直接相接（"imp-" + "ortant" → "important"）。
            let mut compact = compact_key(&readable);
            if compact.ends_with('-') {
                compact.pop();
            }
            if compact.is_empty() {
                continue;
            }
            let start = key.len();
            key.extend(compact.chars());
            let end = key.len();

            let region = line_region.get(line_id);
            let mut source_node_ids = Vec::new();
            if let Some((region_id, _, _)) = region {
                if !region_id.is_empty() {
                    source_node_ids.push(region_id.clone());
                }
            }
            if !line_id.is_empty() {
                source_node_ids.push(line_id.to_string());
            }
            let readable_len = readable.chars().count();
            let significant = match region {
                Some((_, _, sig)) => *sig,
                // 无区域归属时以长度粗判显著性，滤掉页码/短页眉。
                None => readable_len >= 25,
            };
            let anchor = region
                .and_then(|(_, anchor, _)| anchor.clone())
                .unwrap_or_else(|| {
                    json!({
                        "sourceFileId": file_id,
                        "pageIndex": page_index,
                        "nodeIds": source_node_ids.clone(),
                        "sourceHash": file_hash,
                    })
                });
            if significant {
                for id in &source_node_ids {
                    significant_ids.insert(id.clone());
                }
            }
            total_readable_len += readable_len;
            units.push(SourceUnit {
                range: (start, end),
                anchor,
                source_node_ids,
                page_index,
                significant,
                readable_len,
            });
        }

    }

    if key.is_empty() {
        return None;
    }
    Some(SourceModel {
        key,
        units,
        total_readable_len,
        significant_ids,
    })
}

// ---------------------------------------------------------------------------
// 相似度与句子匹配
// ---------------------------------------------------------------------------

/// 字符二元组多重集的 Dice 系数，作为滑窗的廉价预筛。
fn bigram_dice(a: &[char], b: &[char]) -> f64 {
    if a.len() < 2 || b.len() < 2 {
        // 太短无法取二元组时退回字符集合 Dice。
        let sa: BTreeSet<char> = a.iter().copied().collect();
        let sb: BTreeSet<char> = b.iter().copied().collect();
        if sa.is_empty() && sb.is_empty() {
            return 1.0;
        }
        let inter = sa.intersection(&sb).count();
        return 2.0 * inter as f64 / (sa.len() + sb.len()) as f64;
    }
    let mut counts: BTreeMap<(char, char), i32> = BTreeMap::new();
    for pair in a.windows(2) {
        *counts.entry((pair[0], pair[1])).or_default() += 1;
    }
    let mut inter = 0i32;
    for pair in b.windows(2) {
        let entry = counts.entry((pair[0], pair[1])).or_default();
        if *entry > 0 {
            *entry -= 1;
            inter += 1;
        }
    }
    let total = (a.len() - 1) + (b.len() - 1);
    2.0 * inter as f64 / total as f64
}

/// 归一编辑距离相似度（1 - 距离/较长长度），用于最终打分。
fn levenshtein_ratio(a: &[char], b: &[char]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut prev: Vec<usize> = (0..=short.len()).collect();
    let mut curr = vec![0usize; short.len() + 1];
    for (i, lc) in long.iter().enumerate() {
        curr[0] = i + 1;
        for (j, sc) in short.iter().enumerate() {
            let cost = if lc == sc { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    let dist = prev[short.len()];
    1.0 - dist as f64 / long.len().max(1) as f64
}

/// 在字符序列 `hay` 中从 `from` 起找子串 `needle` 的起点。
fn find_sub(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    (from.min(last + 1)..=last).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// 把可读文本切成句子（在 . ! ? 处断句），无标点时整体作一句。
fn split_sentences(readable: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = readable.chars().collect();
    for (idx, ch) in chars.iter().enumerate() {
        current.push(*ch);
        if matches!(ch, '.' | '!' | '?') {
            let next_is_boundary = chars
                .get(idx + 1)
                .map_or(true, |next| next.is_whitespace());
            if next_is_boundary {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    sentences.push(trimmed.to_string());
                }
                current.clear();
            }
        }
    }
    let tail = current.trim();
    if !tail.is_empty() {
        sentences.push(tail.to_string());
    }
    if sentences.is_empty() && !readable.trim().is_empty() {
        sentences.push(readable.trim().to_string());
    }
    sentences
}

struct SentenceHit {
    similarity: f64,
    start: usize,
    end: usize,
}

/// 原卷 key 里 `needle` 是否至少出现两次（重复句不参与单调判定）。
fn occurs_at_least_twice(hay: &[char], needle: &[char]) -> bool {
    match find_sub(hay, needle, 0) {
        Some(first) => find_sub(hay, needle, first + 1).is_some(),
        None => false,
    }
}

/// 在 `[from..]` 区域内滑窗找与 `sentence` 最相似的一段（预筛 + 编辑距离细化）。
fn best_fuzzy(hay: &[char], sentence: &[char], from: usize) -> SentenceHit {
    let win = sentence.len().min(hay.len()).max(1);
    if from + win > hay.len() {
        let start = hay.len().saturating_sub(win);
        let end = hay.len();
        let ratio = levenshtein_ratio(sentence, &hay[start..end]);
        return SentenceHit { similarity: ratio, start, end };
    }
    let step = (win / 8).max(1);
    let mut best_start = from;
    let mut best_prefilter = -1.0f64;
    let mut i = from;
    while i + win <= hay.len() {
        let d = bigram_dice(sentence, &hay[i..i + win]);
        if d > best_prefilter {
            best_prefilter = d;
            best_start = i;
        }
        i += step;
    }
    let lo = best_start.saturating_sub(step).max(from);
    let hi = (best_start + step).min(hay.len().saturating_sub(win));
    let mut best = SentenceHit {
        similarity: 0.0,
        start: best_start,
        end: (best_start + win).min(hay.len()),
    };
    for start in lo..=hi {
        let end = (start + win).min(hay.len());
        let ratio = levenshtein_ratio(sentence, &hay[start..end]);
        if ratio > best.similarity {
            best = SentenceHit { similarity: ratio, start, end };
        }
    }
    best
}

/// 在原卷 key 里为一句云端文本定位。顺序感知：先在命中游标 `from` 之后的窗口里找
/// （精确优先、再模糊），够好就用以保持阅读顺序；否则回退全局搜索。
fn match_sentence(model: &SourceModel, sentence: &[char], from: usize, accept: f64) -> SentenceHit {
    let hay = &model.key;
    if sentence.is_empty() {
        return SentenceHit { similarity: 1.0, start: 0, end: 0 };
    }
    if hay.is_empty() {
        return SentenceHit { similarity: 0.0, start: 0, end: 0 };
    }
    let from = from.min(hay.len());
    if let Some(pos) = find_sub(hay, sentence, from) {
        return SentenceHit { similarity: 1.0, start: pos, end: pos + sentence.len() };
    }
    let forward = best_fuzzy(hay, sentence, from);
    if forward.similarity >= accept {
        return forward;
    }
    if let Some(pos) = find_sub(hay, sentence, 0) {
        return SentenceHit { similarity: 1.0, start: pos, end: pos + sentence.len() };
    }
    let global = best_fuzzy(hay, sentence, 0);
    if global.similarity >= forward.similarity {
        global
    } else {
        forward
    }
}

// ---------------------------------------------------------------------------
// 云端节点抽取与对齐结果
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Passage,
    Instruction,
    Prompt,
    Option,
    Stimulus,
}

impl NodeKind {
    /// 题目内容（说明/题干/选项/摘要笔记）未对齐要阻断题组；原文（passage）未对齐只是不采用云端原文。
    fn is_question_content(self) -> bool {
        !matches!(self, NodeKind::Passage)
    }
    fn as_str(self) -> &'static str {
        match self {
            NodeKind::Passage => "passage",
            NodeKind::Instruction => "instruction",
            NodeKind::Prompt => "prompt",
            NodeKind::Option => "option",
            NodeKind::Stimulus => "stimulus",
        }
    }
}

/// 一个待对齐的云端文本节点。
struct TextNodeInput {
    node_id: String,
    kind: NodeKind,
    task_id: Option<String>,
    text: String,
}

/// 单个云端节点的对齐结论。
#[derive(Debug, Clone)]
pub(crate) struct NodeAlignment {
    pub node_id: String,
    pub kind_label: &'static str,
    pub task_id: Option<String>,
    pub sentence_count: usize,
    pub matched_sentences: usize,
    pub min_similarity: f64,
    pub mean_similarity: f64,
    /// 命中原卷的源节点 id（区域/行），用于合成 nodeIds。
    pub source_node_ids: BTreeSet<String>,
    /// 由命中区域汇集的真实 sourceAnchors（采纳阶段据此盖 provenanceStatus=source）。
    pub anchors: Vec<Value>,
    /// 含低于 invented 阈值的句子（云端凭空生成）。
    pub has_invented: bool,
    /// 该节点是否整体判为对齐（题目内容要求全部句子命中、无凭空句）。
    pub aligned: bool,
}

/// 递归收集一个内容节点子树下所有 `text` 字符串，拼成该节点的可见文本。
fn gather_node_text(node: &Value, out: &mut String) {
    match node {
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                if !text.is_empty() {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(text);
                }
            }
            for (key, child) in map {
                if key == "text" {
                    continue;
                }
                gather_node_text(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| gather_node_text(item, out)),
        _ => {}
    }
}

fn node_text(node: &Value) -> String {
    let mut out = String::new();
    gather_node_text(node, &mut out);
    out
}

fn push_node(inputs: &mut Vec<TextNodeInput>, node: &Value, kind: NodeKind, task_id: &Option<String>, id_key: &str) {
    let text = node_text(node);
    if text.trim().is_empty() {
        return;
    }
    let node_id = node
        .get(id_key)
        .or_else(|| node.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    inputs.push(TextNodeInput { node_id, kind, task_id: task_id.clone(), text });
}

/// 枚举候选里所有可对齐文本节点：原文段落、说明、题干、选项、摘要/笔记。
fn collect_text_nodes(authoring: &Value) -> Vec<TextNodeInput> {
    let mut inputs = Vec::new();
    for node in authoring
        .pointer("/passage/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        push_node(&mut inputs, node, NodeKind::Passage, &None, "id");
    }
    for group in authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let task_id = group
            .get("taskId")
            .and_then(Value::as_str)
            .map(str::to_string);
        for node in group.get("instructions").and_then(Value::as_array).into_iter().flatten() {
            push_node(&mut inputs, node, NodeKind::Instruction, &task_id, "id");
        }
        for node in group.get("stimulus").and_then(Value::as_array).into_iter().flatten() {
            push_node(&mut inputs, node, NodeKind::Stimulus, &task_id, "id");
        }
        for response in group.get("responseGroups").and_then(Value::as_array).into_iter().flatten() {
            for node in response.get("prompt").and_then(Value::as_array).into_iter().flatten() {
                push_node(&mut inputs, node, NodeKind::Prompt, &task_id, "id");
            }
            for option in response.get("options").and_then(Value::as_array).into_iter().flatten() {
                push_node(&mut inputs, option, NodeKind::Option, &task_id, "optionId");
            }
        }
        for option in group
            .pointer("/optionBank/options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            push_node(&mut inputs, option, NodeKind::Option, &task_id, "optionId");
        }
    }
    inputs
}

struct SentenceEval {
    text: String,
    similarity: f64,
    start: usize,
    end: usize,
    /// key 太短（<3 字符）无法有意义定位，不计入命中率与顺序。
    trivial: bool,
    source_node_ids: Vec<String>,
}

/// 单元索引与其命中区间求交，返回被覆盖到的源单元下标。
fn units_overlapping(model: &SourceModel, start: usize, end: usize) -> Vec<usize> {
    if end <= start {
        return Vec::new();
    }
    model
        .units
        .iter()
        .enumerate()
        .filter(|(_, unit)| unit.range.0 < end && unit.range.1 > start)
        .map(|(index, _)| index)
        .collect()
}

/// 对齐单个云端文本节点。
fn align_node(
    model: &SourceModel,
    input: &TextNodeInput,
    config: &AlignmentConfig,
    covered: &mut BTreeSet<usize>,
    matched_positions: &mut Vec<usize>,
    cursor: &mut usize,
) -> (NodeAlignment, Vec<SentenceEval>) {
    let readable = normalize_readable(&input.text);
    let sentences = split_sentences(&readable);
    let mut evals = Vec::with_capacity(sentences.len());
    let mut source_node_ids = BTreeSet::new();
    let mut anchor_keys = BTreeSet::new();
    let mut anchors = Vec::new();
    let mut matched = 0usize;
    let mut non_trivial = 0usize;
    let mut sim_sum = 0.0;
    let mut min_sim = 1.0f64;
    let mut has_invented = false;

    for sentence in &sentences {
        let key: Vec<char> = compact_key(sentence).chars().collect();
        if key.len() < 3 {
            evals.push(SentenceEval {
                text: sentence.clone(),
                similarity: 1.0,
                start: 0,
                end: 0,
                trivial: true,
                source_node_ids: Vec::new(),
            });
            continue;
        }
        let hit = match_sentence(model, &key, *cursor, config.sentence_threshold);
        non_trivial += 1;
        sim_sum += hit.similarity;
        min_sim = min_sim.min(hit.similarity);
        if hit.similarity < config.invented_threshold {
            has_invented = true;
        }
        let mut sentence_ids = Vec::new();
        if hit.similarity >= config.sentence_threshold {
            matched += 1;
            // 短句（<5 词）与原卷中重复出现的句子不参与单调判定：它们的命中位置有歧义，
            // 会造成假逆序。词数按 ≥2 字符的词元统计——字形逐字空格（"n a m e"）不能把
            // "candidate name" 这类短标签撑成长句。命中率与覆盖率仍照常计入。
            let word_count = sentence
                .split_whitespace()
                .filter(|word| word.chars().count() >= 2)
                .count();
            if word_count >= 5 && !occurs_at_least_twice(&model.key, &key) {
                matched_positions.push(hit.start);
            }
            // 命中游标只前进不后退，供顺序感知匹配定位下一句。
            *cursor = (*cursor).max(hit.end);
            for index in units_overlapping(model, hit.start, hit.end) {
                covered.insert(index);
                let unit = &model.units[index];
                for id in &unit.source_node_ids {
                    source_node_ids.insert(id.clone());
                    sentence_ids.push(id.clone());
                }
                let anchor_key = serde_json::to_string(&unit.anchor).unwrap_or_default();
                if anchor_keys.insert(anchor_key) {
                    anchors.push(unit.anchor.clone());
                }
            }
        }
        evals.push(SentenceEval {
            text: sentence.clone(),
            similarity: hit.similarity,
            start: hit.start,
            end: hit.end,
            trivial: false,
            source_node_ids: sentence_ids,
        });
    }

    let mean = if non_trivial == 0 { 1.0 } else { sim_sum / non_trivial as f64 };
    let sentence_count = non_trivial;
    // 云端为主：命中比例达标即视为该节点小差异、直接采用。凭空句不再逐节点硬阻断——是否
    // 整体采纳由题组比例（≥group_align_ratio）与原文凭空句容忍在文档级判定。
    let hit_ratio = if sentence_count == 0 {
        1.0
    } else {
        matched as f64 / sentence_count as f64
    };
    let threshold = if input.kind.is_question_content() {
        config.group_align_ratio
    } else {
        config.passage_pass_ratio
    };
    let aligned = hit_ratio >= threshold;
    let alignment = NodeAlignment {
        node_id: input.node_id.clone(),
        kind_label: input.kind.as_str(),
        task_id: input.task_id.clone(),
        sentence_count,
        matched_sentences: matched,
        min_similarity: if sentence_count == 0 { 1.0 } else { min_sim },
        mean_similarity: mean,
        source_node_ids,
        anchors,
        has_invented,
        aligned,
    };
    (alignment, evals)
}

// ---------------------------------------------------------------------------
// 文档级校验与报告
// ---------------------------------------------------------------------------

/// 给人复核的确定性抽查样本。
#[derive(Debug, Clone)]
pub(crate) struct SampleEntry {
    pub node_id: String,
    pub kind_label: &'static str,
    pub task_id: Option<String>,
    pub sentence: String,
    pub similarity: f64,
    pub source_node_ids: Vec<String>,
}

/// 对齐校验的完整结论（事实层；采纳策略在别处据此决定）。
#[derive(Debug, Clone)]
pub(crate) struct AlignmentReport {
    pub nodes: Vec<NodeAlignment>,
    /// 原文（passage）整体命中率是否达标、可整体采用云端原文。
    pub passage_pass: bool,
    pub passage_sentence_total: usize,
    pub passage_sentence_matched: usize,
    /// 原文里整句对不上/凭空的句子（{nodeId, sentence, similarity}）——即使原文整体采纳，
    /// 这些仍单列为修复目标。
    pub passage_invented: Vec<Value>,
    /// 凭空句是否在容忍范围内（在范围内可整体采纳原文）。
    pub passage_invented_ok: bool,
    /// 云端可读总长 / 原卷可读总长。
    pub length_ratio: f64,
    pub length_ratio_ok: bool,
    /// 显著源区域被命中的比例。
    pub coverage: f64,
    pub coverage_ok: bool,
    /// 命中位置在阅读顺序上的逆序次数（相邻大幅回跳，仅供诊断）。
    pub order_violations: usize,
    /// 命中位置按阅读顺序递增的比例（最长非降子序列 / 命中总数）。
    pub order_in_order_ratio: f64,
    pub monotonic: bool,
    /// 每个题组是否所有题目内容节点都对齐，及不合格原因。
    pub group_aligned: BTreeMap<String, bool>,
    pub group_reasons: BTreeMap<String, Vec<String>>,
    pub samples: Vec<SampleEntry>,
}

/// 校验结果：扫描卷无文本层时无法对齐，保持保守行为。
pub(crate) enum AlignmentOutcome {
    NoTextLayer,
    Assessed(Box<AlignmentReport>),
}

/// FNV-1a，用于抽查的可复现选取。
fn fnv1a(seed: u64, text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64 ^ seed;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// 为一个节点确定性地选取至多 `take` 个句子下标。
fn sample_indices(seed: u64, node_id: &str, sentence_count: usize, take: usize) -> Vec<usize> {
    if sentence_count == 0 || take == 0 {
        return Vec::new();
    }
    let mut state = fnv1a(seed, node_id).max(1);
    let mut picked = BTreeSet::new();
    let limit = take.min(sentence_count);
    while picked.len() < limit {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        picked.insert((state % sentence_count as u64) as usize);
    }
    picked.into_iter().collect()
}

/// 命中位置允许的回溯容差（字符）：同区域内小幅回跳不算乱序。
const ORDER_SLACK: usize = 100;

/// 最长非降子序列长度（O(n log n)）：命中位置里有多少能构成阅读顺序递增的一条链。
/// 单个离群命中不会连累整条链，据此判断错位是零星还是大面积。
fn longest_non_decreasing(positions: &[usize]) -> usize {
    let mut tails: Vec<usize> = Vec::new();
    for &value in positions {
        // 上界查找：允许相等（非降），把 value 放到第一个 > value 的位置。
        let idx = tails.partition_point(|&tail| tail <= value);
        if idx == tails.len() {
            tails.push(value);
        } else {
            tails[idx] = value;
        }
    }
    tails.len()
}

/// 对整个云端候选做对齐校验。原卷无文本层时返回 `NoTextLayer`。
pub(crate) fn assess_alignment(
    document_ir: &Value,
    authoring: &Value,
    config: &AlignmentConfig,
) -> AlignmentOutcome {
    let Some(model) = build_source_model(document_ir) else {
        return AlignmentOutcome::NoTextLayer;
    };
    let inputs = collect_text_nodes(authoring);
    let mut nodes = Vec::new();
    let mut covered: BTreeSet<usize> = BTreeSet::new();
    let mut matched_positions: Vec<usize> = Vec::new();
    let mut cloud_len = 0usize;
    let mut passage_total = 0usize;
    let mut passage_matched = 0usize;
    let mut passage_invented: Vec<Value> = Vec::new();
    let mut group_matched: BTreeMap<String, usize> = BTreeMap::new();
    let mut group_total: BTreeMap<String, usize> = BTreeMap::new();
    let mut group_aligned: BTreeMap<String, bool> = BTreeMap::new();
    let mut group_reasons: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut samples = Vec::new();
    let mut cursor = 0usize;

    for input in &inputs {
        cloud_len += normalize_readable(&input.text).chars().count();
        let (alignment, evals) =
            align_node(&model, input, config, &mut covered, &mut matched_positions, &mut cursor);

        if input.kind == NodeKind::Passage {
            for eval in &evals {
                if !eval.trivial {
                    passage_total += 1;
                    if eval.similarity >= config.sentence_threshold {
                        passage_matched += 1;
                    } else if eval.similarity < config.invented_threshold {
                        // 整句对不上/凭空句：即使原文整体采纳，也单列为修复目标。
                        passage_invented.push(json!({
                            "nodeId": alignment.node_id,
                            "sentence": eval.text.chars().take(160).collect::<String>(),
                            "similarity": eval.similarity,
                        }));
                    }
                }
            }
        }
        // 题组内容对齐按**比例**判定（命中句 / 题目内容句总数 ≥ group_align_ratio），
        // 而不是要求每个节点都完美——个别措辞差异不该整组打回修复。
        if let Some(task_id) = &input.task_id {
            group_aligned.entry(task_id.clone()).or_insert(true);
            if input.kind.is_question_content() {
                *group_matched.entry(task_id.clone()).or_default() += alignment.matched_sentences;
                *group_total.entry(task_id.clone()).or_default() += alignment.sentence_count;
                if !alignment.aligned {
                    group_reasons.entry(task_id.clone()).or_default().push(format!(
                        "{} 节点 {} 命中 {}/{}，最弱相似度 {:.2}{}",
                        alignment.kind_label,
                        alignment.node_id,
                        alignment.matched_sentences,
                        alignment.sentence_count,
                        alignment.min_similarity,
                        if alignment.has_invented { "，含凭空句" } else { "" }
                    ));
                }
            }
        }

        let non_trivial: Vec<usize> = evals
            .iter()
            .enumerate()
            .filter(|(_, eval)| !eval.trivial)
            .map(|(index, _)| index)
            .collect();
        for pick in sample_indices(config.sample_seed, &alignment.node_id, non_trivial.len(), config.sample_per_node) {
            let eval = &evals[non_trivial[pick]];
            samples.push(SampleEntry {
                node_id: alignment.node_id.clone(),
                kind_label: alignment.kind_label,
                task_id: alignment.task_id.clone(),
                sentence: eval.text.chars().take(160).collect(),
                similarity: eval.similarity,
                source_node_ids: eval.source_node_ids.clone(),
            });
        }
        nodes.push(alignment);
    }

    let significant_total = model.significant_ids.len();
    let mut covered_significant: BTreeSet<String> = BTreeSet::new();
    for index in &covered {
        let unit = &model.units[*index];
        if unit.significant {
            for id in &unit.source_node_ids {
                if model.significant_ids.contains(id) {
                    covered_significant.insert(id.clone());
                }
            }
        }
    }
    let coverage = if significant_total == 0 {
        1.0
    } else {
        covered_significant.len() as f64 / significant_total as f64
    };
    let length_ratio = if model.total_readable_len == 0 {
        0.0
    } else {
        cloud_len as f64 / model.total_readable_len as f64
    };
    let order_violations = matched_positions
        .windows(2)
        .filter(|pair| pair[1] + ORDER_SLACK < pair[0])
        .count();
    let order_in_order_ratio = if matched_positions.len() < 2 {
        1.0
    } else {
        longest_non_decreasing(&matched_positions) as f64 / matched_positions.len() as f64
    };
    let passage_pass = passage_total == 0
        || (passage_matched as f64 / passage_total as f64) >= config.passage_pass_ratio;
    // 原文凭空句容忍：少量（≤invented_max_count 且 ≤invented_max_ratio 占比）仍整体采纳，
    // 这些句子交给修复循环单独处理，而不是因此整篇原文都不采纳。
    let passage_invented_ok = passage_invented.len() <= config.invented_max_count
        && (passage_total == 0
            || (passage_invented.len() as f64) <= config.invented_max_ratio * passage_total as f64);

    // 题组内容对齐比例达标即采纳该组；不足则记原因，进云端校核清单。
    for (task_id, total) in &group_total {
        let matched = group_matched.get(task_id).copied().unwrap_or(0);
        let ratio = if *total == 0 {
            1.0
        } else {
            matched as f64 / *total as f64
        };
        let ok = ratio >= config.group_align_ratio;
        group_aligned.insert(task_id.clone(), ok);
        if !ok {
            group_reasons.entry(task_id.clone()).or_default().push(format!(
                "题组内容对齐比例 {:.2}（命中 {}/{}）低于 {:.2}",
                ratio, matched, total, config.group_align_ratio
            ));
        }
    }

    AlignmentOutcome::Assessed(Box::new(AlignmentReport {
        nodes,
        passage_pass,
        passage_sentence_total: passage_total,
        passage_sentence_matched: passage_matched,
        passage_invented,
        passage_invented_ok,
        length_ratio,
        length_ratio_ok: length_ratio >= config.length_ratio_min
            && length_ratio <= config.length_ratio_max,
        coverage,
        coverage_ok: coverage >= config.coverage_min,
        order_violations,
        order_in_order_ratio,
        monotonic: order_in_order_ratio >= config.order_pass_ratio,
        group_aligned,
        group_reasons,
        samples,
    }))
}

impl AlignmentReport {
    /// 序列化成采纳记录/回报可直接嵌入的结构。
    pub(crate) fn to_json(&self) -> Value {
        let nodes = self
            .nodes
            .iter()
            .map(|node| {
                json!({
                    "nodeId": node.node_id,
                    "kind": node.kind_label,
                    "taskId": node.task_id,
                    "sentenceCount": node.sentence_count,
                    "matchedSentences": node.matched_sentences,
                    "minSimilarity": node.min_similarity,
                    "meanSimilarity": node.mean_similarity,
                    "aligned": node.aligned,
                    "hasInvented": node.has_invented,
                    "sourceNodeIds": node.source_node_ids.iter().cloned().collect::<Vec<_>>(),
                    "anchors": node.anchors,
                })
            })
            .collect::<Vec<_>>();
        let samples = self
            .samples
            .iter()
            .map(|sample| {
                json!({
                    "nodeId": sample.node_id,
                    "kind": sample.kind_label,
                    "taskId": sample.task_id,
                    "sentence": sample.sentence,
                    "similarity": sample.similarity,
                    "sourceNodeIds": sample.source_node_ids,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "passagePass": self.passage_pass,
            "passageSentenceTotal": self.passage_sentence_total,
            "passageSentenceMatched": self.passage_sentence_matched,
            "passageInvented": self.passage_invented,
            "passageInventedOk": self.passage_invented_ok,
            "lengthRatio": self.length_ratio,
            "lengthRatioOk": self.length_ratio_ok,
            "coverage": self.coverage,
            "coverageOk": self.coverage_ok,
            "orderViolations": self.order_violations,
            "orderInOrderRatio": self.order_in_order_ratio,
            "monotonic": self.monotonic,
            "groupAligned": serde_json::to_value(&self.group_aligned).unwrap_or(Value::Null),
            "groupReasons": serde_json::to_value(&self.group_reasons).unwrap_or(Value::Null),
            "nodes": nodes,
            "samples": samples,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单页原卷：每行独立成一个 text 区域，带真实锚点。
    fn source_doc(lines: &[(&str, &str)]) -> Value {
        let mut line_vals = Vec::new();
        let mut region_vals = Vec::new();
        for (order, (id, text)) in lines.iter().enumerate() {
            line_vals.push(json!({"id": id, "text": text, "sourceOrder": order}));
            let region_id = format!("r-{id}");
            region_vals.push(json!({
                "id": region_id,
                "kind": "text",
                "childLineIds": [id],
                "sourceAnchors": [{
                    "sourceFileId": "f1",
                    "pageIndex": 1,
                    "nodeIds": [region_id, id],
                    "sourceHash": "h1"
                }]
            }));
        }
        json!({
            "schemaVersion": "DocumentIRV2",
            "sourceFiles": [{"sourceFileId": "f1", "sourceHash": "h1"}],
            "pages": [{"pageIndex": 0, "lines": line_vals, "regions": region_vals}]
        })
    }

    fn passage_authoring(nodes: &[(&str, &str)]) -> Value {
        let content = nodes
            .iter()
            .map(|(id, text)| json!({"id": id, "type": "text", "text": text}))
            .collect::<Vec<_>>();
        json!({"passage": {"content": content}, "taskGroups": []})
    }

    fn report(document_ir: &Value, authoring: &Value) -> AlignmentReport {
        match assess_alignment(document_ir, authoring, &AlignmentConfig::default()) {
            AlignmentOutcome::Assessed(report) => *report,
            AlignmentOutcome::NoTextLayer => panic!("expected text layer"),
        }
    }

    #[test]
    fn glyph_spaced_source_still_aligns_and_reuses_region_anchor() {
        // pdfium 把整句拆成字符间距；归一后与云端正常空格落成同一 key。
        let doc = source_doc(&[("l1", "T h e   h e a t   o f   c h i l i   p e p p e r s .")]);
        let authoring = passage_authoring(&[("p1", "The heat of chili peppers.")]);
        let report = report(&doc, &authoring);
        let node = &report.nodes[0];
        assert_eq!(node.matched_sentences, 1);
        assert!((node.min_similarity - 1.0).abs() < 1e-9, "应精确命中");
        assert!(node.aligned);
        assert!(
            node.anchors.iter().any(|anchor| anchor
                .pointer("/nodeIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id == "r-l1"))),
            "应复用原卷区域的真实锚点：{:?}",
            node.anchors
        );
    }

    #[test]
    fn hyphenated_line_break_joins_across_lines() {
        // "imp-\nortant" 断行连字符要去掉再相接。
        let doc = source_doc(&[("l1", "This is an imp-"), ("l2", "ortant discovery.")]);
        let authoring = passage_authoring(&[("p1", "This is an important discovery.")]);
        let report = report(&doc, &authoring);
        assert_eq!(report.nodes[0].matched_sentences, 1);
        assert!((report.nodes[0].min_similarity - 1.0).abs() < 1e-9);
    }

    #[test]
    fn scanned_pdf_without_text_layer_is_not_assessable() {
        let doc = json!({"schemaVersion": "DocumentIRV1", "pages": []});
        let authoring = passage_authoring(&[("p1", "anything")]);
        assert!(matches!(
            assess_alignment(&doc, &authoring, &AlignmentConfig::default()),
            AlignmentOutcome::NoTextLayer
        ));
    }

    #[test]
    fn invented_sentence_is_flagged_and_blocks_alignment() {
        let doc = source_doc(&[("l1", "Bees pollinate flowers throughout the spring season.")]);
        let authoring = passage_authoring(&[(
            "p1",
            "Bees pollinate flowers throughout the spring season. Dragons breathe fire over the distant frozen mountains at midnight.",
        )]);
        let report = report(&doc, &authoring);
        let node = &report.nodes[0];
        assert!(node.has_invented, "第二句应判为凭空句：{node:?}");
        assert!(!node.aligned);
        assert!(!report.passage_pass, "原文命中率 0.5 应不达标");
    }

    #[test]
    fn truncated_cloud_passage_fails_length_ratio() {
        let long = "Capsaicin binds to receptors that normally sense heat and abrasion, \
            which is why the brain reads a chili pepper as physically hot even though no \
            temperature change occurs, a quirk that has shaped cuisines across the world.";
        let doc = source_doc(&[("l1", long)]);
        let authoring = passage_authoring(&[("p1", "Capsaicin binds to receptors.")]);
        let report = report(&doc, &authoring);
        assert!(
            report.length_ratio < AlignmentConfig::default().length_ratio_min,
            "截断稿长度比应偏低：{}",
            report.length_ratio
        );
        assert!(!report.length_ratio_ok);
    }

    #[test]
    fn out_of_reading_order_matches_are_detected() {
        let filler: String = std::iter::repeat("neutral filler text here. ").take(12).collect();
        let doc = source_doc(&[
            ("l1", "Alpha statement introduces the first idea."),
            ("l2", &filler),
            ("l3", "Beta statement introduces the second idea."),
        ]);
        let authoring = passage_authoring(&[
            ("p1", "Beta statement introduces the second idea."),
            ("p2", "Alpha statement introduces the first idea."),
        ]);
        let report = report(&doc, &authoring);
        assert!(report.order_violations > 0, "应检出乱序");
        assert!(!report.monotonic);
    }

    #[test]
    fn small_perturbations_are_adopted_without_entering_repair() {
        // 小扰动（个别换词、标点、空格）：相似度仍 ≥0.8，判为命中、直接采用云端，不进修复清单。
        let doc = source_doc(&[
            ("l1", "The Roman palace at Fishbourne was discovered by workmen digging a trench in 1960."),
            ("l2", "Archaeologists later uncovered mosaic floors of exceptional quality."),
            ("l3", "The site attracts many thousands of visitors every single year."),
        ]);
        let authoring = passage_authoring(&[
            ("p1", "The Roman palace at Fishbourne was discovered by workmen digging a trench in 1960 ."),
            ("p2", "Archaeologists later uncovered mosaic floors of exceptional quality!"),
            ("p3", "The site attracts many thousand of visitors every single year."),
        ]);
        let report = report(&doc, &authoring);
        assert_eq!(
            report.passage_sentence_matched, report.passage_sentence_total,
            "小扰动应全部判为命中"
        );
        assert!(report.passage_pass, "命中率达标应可整体采纳");
        assert!(
            report.passage_invented.is_empty(),
            "小扰动不得进修复清单：{:?}",
            report.passage_invented
        );
        assert!(report.passage_invented_ok);
    }

    #[test]
    fn large_perturbations_enter_the_repair_list() {
        // 大扰动（整句被换成原卷没有的内容）：判为凭空句，进修复清单（passage_invented）。
        let doc = source_doc(&[
            ("l1", "The Roman palace at Fishbourne was discovered by workmen digging a trench in 1960."),
            ("l2", "Archaeologists later uncovered mosaic floors of exceptional quality."),
            ("l3", "The site attracts many thousands of visitors every single year."),
        ]);
        let authoring = passage_authoring(&[
            ("p1", "The Roman palace at Fishbourne was discovered by workmen digging a trench in 1960."),
            ("p2", "Archaeologists later uncovered mosaic floors of exceptional quality."),
            ("p3", "Quarterly financial statements must be filed with the regulator before April."),
        ]);
        let report = report(&doc, &authoring);
        assert!(
            report
                .passage_invented
                .iter()
                .any(|entry| entry.get("nodeId").and_then(Value::as_str) == Some("p3")),
            "大扰动整句应进修复清单：{:?}",
            report.passage_invented
        );
    }

    #[test]
    fn coverage_reflects_uncovered_significant_regions() {
        let doc = source_doc(&[
            ("l1", "The first paragraph explains the origin of the study."),
            ("l2", "The second paragraph reports the main experimental result."),
            ("l3", "The third paragraph discusses limitations and future work."),
        ]);
        let partial = passage_authoring(&[
            ("p1", "The first paragraph explains the origin of the study."),
            ("p2", "The second paragraph reports the main experimental result."),
        ]);
        assert!(!report(&doc, &partial).coverage_ok, "漏掉第三段应覆盖不达标");

        let full = passage_authoring(&[
            ("p1", "The first paragraph explains the origin of the study."),
            ("p2", "The second paragraph reports the main experimental result."),
            ("p3", "The third paragraph discusses limitations and future work."),
        ]);
        let full_report = report(&doc, &full);
        assert!(full_report.coverage_ok);
        assert!((full_report.coverage - 1.0).abs() < 1e-9);
    }

    #[test]
    fn task_group_blocks_when_instruction_is_invented() {
        let doc = source_doc(&[
            ("l1", "Do the following statements agree with the information given?"),
            ("l2", "Write TRUE FALSE or NOT GIVEN next to each statement."),
            ("l3", "The museum was built beside the original Roman foundations."),
        ]);
        let group = |instruction: &str| {
            json!({
                "passage": {"content": []},
                "taskGroups": [{
                    "taskId": "g1",
                    "instructions": [{"id": "i1", "type": "text", "text": instruction}],
                    "responseGroups": [{
                        "responseGroupId": "rg1",
                        "prompt": [{"id": "pr1", "type": "text",
                            "text": "The museum was built beside the original Roman foundations."}],
                        "options": []
                    }]
                }]
            })
        };

        let aligned = report(&doc, &group("Write TRUE FALSE or NOT GIVEN next to each statement."));
        assert_eq!(aligned.group_aligned.get("g1"), Some(&true));

        let invented = report(&doc, &group("Match each heading to the correct paragraph below."));
        assert_eq!(invented.group_aligned.get("g1"), Some(&false));
        assert!(invented.group_reasons.get("g1").is_some_and(|r| !r.is_empty()));
    }

    #[test]
    fn sampling_is_deterministic_for_a_given_seed() {
        let a = sample_indices(1234, "node-x", 20, 3);
        let b = sample_indices(1234, "node-x", 20, 3);
        assert_eq!(a, b);
        assert_eq!(a.len(), 3);
        let unique: BTreeSet<usize> = a.iter().copied().collect();
        assert_eq!(unique.len(), 3, "抽样下标应互不相同");
        assert_ne!(a, sample_indices(1234, "node-y", 20, 3), "不同节点应取不同样本");
    }

    // --- 9 份私有卷的真实文本层对齐（语料门控；缺卷或未开启时跳过）---

    /// 复刻 pdf_facts_shadow 测试里构造 job/source 的方式（该文件的辅助是私有的）。
    fn shadow_job_source(abs_pdf: &std::path::Path, file_id: &str) -> (crate::ImportJob, crate::SourceFile) {
        let mut job = crate::job_store::make_job(crate::CreateJobInput {
            title: Some("alignment shadow fixture".to_string()),
            category: Some("phase2".to_string()),
            frequency: Some("medium".to_string()),
            tags: Some(vec!["reconcile-alignment".to_string()]),
            ..Default::default()
        });
        let bytes = std::fs::read(abs_pdf).expect("private PDF must exist");
        let original_name = abs_pdf
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("fixture.pdf")
            .to_string();
        let source = crate::SourceFile {
            file_id: file_id.to_string(),
            original_name: original_name.clone(),
            stored_name: original_name,
            file_type: "pdf".to_string(),
            sha256: crate::hash_bytes(&bytes),
            size_bytes: bytes.len() as u64,
            role: "MainQuestion".to_string(),
            imported_at: chrono::Utc::now(),
        };
        job.source_files = vec![source.clone()];
        (job, source)
    }

    /// 按软断行规则拼接若干行文本（行尾连字符去掉后无缝相接，否则空格分隔）。
    fn clean_join(lines: &[&str]) -> String {
        let mut out = String::new();
        for line in lines {
            let piece = line.trim();
            if piece.is_empty() {
                continue;
            }
            if out.ends_with('-') {
                out.pop();
                out.push_str(piece);
            } else if out.is_empty() {
                out.push_str(piece);
            } else {
                out.push(' ');
                out.push_str(piece);
            }
        }
        out
    }

    /// 用原卷文本层复刻一份「理想云端候选」的 passage：忠实还原每个显著区域的原文
    /// （区域缺失时退回整页行、孤立长行单独成节点），以此验证忠实内容不会被误判。
    fn ideal_passage_from_shadow(shadow: &Value) -> Value {
        let mut content = Vec::new();
        let mut counter = 0usize;
        for page in shadow.get("pages").and_then(Value::as_array).into_iter().flatten() {
            let mut line_text: BTreeMap<String, String> = BTreeMap::new();
            let mut line_order: Vec<String> = Vec::new();
            for line in page.get("lines").and_then(Value::as_array).into_iter().flatten() {
                if let (Some(id), Some(text)) = (
                    line.get("id").and_then(Value::as_str),
                    line.get("text").and_then(Value::as_str),
                ) {
                    line_text.insert(id.to_string(), text.to_string());
                    line_order.push(id.to_string());
                }
            }
            let mut claimed: BTreeSet<String> = BTreeSet::new();
            let mut regions: Vec<(&Value, u64)> = page
                .get("regions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(idx, region)| {
                    let rank = region
                        .get("readingOrderRank")
                        .or_else(|| region.get("reading_order_rank"))
                        .and_then(Value::as_u64)
                        .unwrap_or(idx as u64);
                    (region, rank)
                })
                .collect();
            regions.sort_by_key(|(_, rank)| *rank);
            for (region, _) in regions {
                let kind = region.get("kind").and_then(Value::as_str).unwrap_or("");
                let child_ids: Vec<String> = region
                    .get("childLineIds")
                    .or_else(|| region.get("child_line_ids"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                for id in &child_ids {
                    claimed.insert(id.clone());
                }
                if !is_significant_region_kind(kind) {
                    continue;
                }
                let lines: Vec<&str> = child_ids
                    .iter()
                    .filter_map(|id| line_text.get(id).map(String::as_str))
                    .collect();
                let text = clean_join(&lines);
                if !text.trim().is_empty() {
                    content.push(json!({"id": format!("ideal-{counter}"), "type": "text", "text": text}));
                    counter += 1;
                }
            }
            // 未归入任何区域的长行（可能是无区域信息的来源）也还原，避免覆盖率虚低。
            for id in &line_order {
                if claimed.contains(id) {
                    continue;
                }
                if let Some(text) = line_text.get(id) {
                    if text.trim().chars().count() >= 25 {
                        let joined = clean_join(&[text.as_str()]);
                        content.push(json!({"id": format!("ideal-{counter}"), "type": "text", "text": joined}));
                        counter += 1;
                    }
                }
            }
        }
        json!({"passage": {"content": content}, "taskGroups": []})
    }

    #[test]
    fn nine_private_pdfs_ideal_candidate_aligns_against_real_text_layer() {
        let truth_path =
            crate::test_support::workspace_path("fixtures/golden/private-pdf-task-presentation-stage2.json");
        let truth: Value = serde_json::from_slice(&std::fs::read(&truth_path).unwrap())
            .expect("stage2 truth JSON parses");
        let fixtures = truth["fixtures"].as_array().expect("fixtures array");
        let paths: Vec<std::path::PathBuf> = fixtures
            .iter()
            .filter_map(|fixture| fixture["sourcePath"].as_str())
            .map(crate::test_support::workspace_path)
            .collect();
        if !crate::test_support::private_corpus_ready(
            "nine_private_pdfs_ideal_candidate_aligns_against_real_text_layer",
            &paths,
        ) {
            return;
        }

        let mut assessed = 0usize;
        let mut table = String::from(
            "\n私有卷 理想候选全量对齐通过率表\nfixture | 文本层 | 原文命中 | 覆盖率 | 长度比 | 单调(顺序比) | passage节点 | 锚点数\n",
        );
        let mut failures = Vec::new();
        for (index, fixture) in fixtures.iter().enumerate() {
            let id = fixture["fixtureId"].as_str().unwrap_or("?");
            let source_path = fixture["sourcePath"].as_str().unwrap();
            let abs = crate::test_support::workspace_path(source_path);
            let (job, source) = shadow_job_source(&abs, &format!("file-align-{index}"));
            let shadow = crate::pdf_facts_shadow::extract_pdf_facts_shadow(&job, &source, &abs)
                .unwrap_or_else(|error| panic!("{id}: 影子抽取失败：{error}"));
            let candidate = ideal_passage_from_shadow(&shadow);
            match assess_alignment(&shadow, &candidate, &AlignmentConfig::default()) {
                AlignmentOutcome::NoTextLayer => {
                    table.push_str(&format!("{id} | 无 | - | - | - | - | - | -\n"));
                }
                AlignmentOutcome::Assessed(report) => {
                    assessed += 1;
                    let anchor_total: usize = report.nodes.iter().map(|node| node.anchors.len()).sum();
                    let passage_nodes = report.nodes.iter().filter(|n| n.kind_label == "passage").count();
                    table.push_str(&format!(
                        "{id} | 有 | {}/{} | {:.2} | {:.2} | {}({:.2}) | {} | {}\n",
                        report.passage_sentence_matched,
                        report.passage_sentence_total,
                        report.coverage,
                        report.length_ratio,
                        if report.monotonic { "是" } else { "否" },
                        report.order_in_order_ratio,
                        passage_nodes,
                        anchor_total,
                    ));
                    if !(report.passage_pass && report.coverage_ok && report.monotonic && report.length_ratio_ok) {
                        failures.push(format!(
                            "{id}: passage_pass={} coverage_ok={}({:.2}) monotonic={}(顺序比 {:.2}, 逆序 {}) length_ratio_ok={}({:.2})",
                            report.passage_pass, report.coverage_ok, report.coverage,
                            report.monotonic, report.order_in_order_ratio, report.order_violations,
                            report.length_ratio_ok, report.length_ratio
                        ));
                    }
                }
            }
        }
        eprintln!("{table}");
        assert!(assessed >= 8, "至少 8 份卷应有可用文本层，实得 {assessed}");
        assert!(failures.is_empty(), "忠实还原的候选不应被判失败：{failures:?}");
    }
}


