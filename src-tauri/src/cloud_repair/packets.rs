//! 校核包（repair packets）：把「这一次校核要看的内容」按差异切成自洽的小块。
//!
//! ## 为什么要有这一层
//!
//! 修复循环以前每一轮都把整份 PDF（base64 附件）、整卷题组索引、**全部**差异、以及
//! 全部历史工具结果一起发给模型。一轮里 90% 的上下文与「这一轮要裁决的那道题」无关：
//! 代价是每次调用都又慢又贵，而且模型更容易在无关内容上分心。更要紧的是，
//! 「上下文里有整份原文」这件事本身不可验证——模型看到的是整份文件，它引用的句子
//! 落在哪一页没人能核对。
//!
//! 校核包把「要校核什么」和「看什么才能校核」绑在一起：每个包只带这部分题目需要的
//! 稿件切片、云端候选切片和原文范围，并**显式列出省略了什么、怎么取回来**。
//!
//! ## 边界
//!
//! 本模块是**纯函数**：输入是已经读好的稿件与原文页索引，输出是切好的包。它不调模型、
//! 不写库、不读磁盘——页图裁剪与原文抓取由 [`super::grab`] 负责，编排由
//! [`super::run_repair_loop`] 负责。这样「切分规则对不对」可以单独测，不必先起一个
//! 假模型服务。
//!
//! ## 页号约定（与 `super::source_page_texts` 一致，别改错）
//!
//! 权威稿锚点里的 `pageIndex` 是 **0-based**（节点 id 前缀 `p004-` 对应 `pageIndex: 3`）；
//! 页图产物（`cache/vision/pdf-images.json`）与 `read_source` 的页对象是 **1-based**。
//! 本模块对外（`scope.pages`、`sourceEvidence.pages[].pageIndex`）统一输出 **1-based**，
//! 换算只发生在 [`anchor_pages`] 一处。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use crate::reconcile::candidate::{expand_question_numbers, nodes_text, normalize_text};

/// 包上下文的粗略预算（估算 token）。超出时按 responseGroup 拆包。
pub(crate) const PACKET_TOKEN_BUDGET: usize = 24_000;
/// 字符 → token 的粗估系数。
///
/// **口径已知偏乐观，刻意不改**（审计发现 A-10）：`chars / 4` 对英文大致成立，
/// 但中文约 1 token ≈ 1–1.5 字符，会被低估 3–4 倍。题面与说明里中文不少，所以
/// `estimatedInputTokens` 偏小、`enforce_packet_budget` 的触发点比真实值晚。
/// 不改的理由：这个数只用于**包内预算**与对外对账，不参与任何正确性判断；把它调大
/// 会同时移动 §6 的对比基线，而收益只是「更早一点丢图」。真正的上限由
/// `PACKET_TOKEN_BUDGET` 与逐包记录共同兜住。读这个数时按「乐观下界」理解。
pub(crate) const PACKET_CHARS_PER_TOKEN: usize = 4;
/// 一张图按固定值估算（区域图与整页图同量级，宁可高估）。
pub(crate) const PACKET_IMAGE_TOKENS: usize = 1_200;
/// 锚点离页上下边缘多近算「贴着边」（题目跨页），需要左右各扩一页。
const PAGE_EDGE_RATIO: f64 = 0.05;
/// `scopeManifest` 里最多列举几条省略项（写太长反而挤占预算）。
const MANIFEST_MAX_OMISSIONS: usize = 8;

/// 原文里的一行，带稳定 id（`p{page}:l{index}`，均从 1 开始）。
///
/// 稳定 id 是「引文可核对」的前提：模型必须逐字复制行文本并带上行 id 与页码，
/// 后端才能判断它引的到底是不是原文里真实存在的那一行。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SourceLine {
    pub id: String,
    pub text: String,
}

/// DOCX 里的一段（按段落 id 给出目标附近的窗口）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SourceParagraph {
    pub id: String,
    pub text: String,
}

/// 一张页图（绝对路径 + 点尺寸）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PageImageRef {
    pub path: String,
    pub mime_type: String,
    /// 页宽（点）。
    pub width: f64,
    /// 页高（点）。
    pub height: f64,
}

/// 原文页索引：由 [`super::grab::load_source_index`] 读好，本模块只消费。
#[derive(Debug, Clone, Default)]
pub(crate) struct SourcePageIndex {
    pub source_file_id: String,
    /// `pdf` / `docx` / `text`。
    pub kind: String,
    /// 1-based 页号 → 行。
    pub lines: BTreeMap<u32, Vec<SourceLine>>,
    /// 1-based 页号 → 页图。
    pub page_images: BTreeMap<u32, PageImageRef>,
    /// 1-based 答案页。
    pub answer_pages: Vec<u32>,
    /// 答案页是否**真的定位到了**。`false` 时不许拿空数组冒充「没有答案页」。
    pub answer_pages_known: bool,
    /// DOCX：按出现顺序的段落。
    pub paragraphs: Vec<SourceParagraph>,
}

impl SourcePageIndex {
    fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.page_images.is_empty() && self.paragraphs.is_empty()
    }
}

/// 切分一个包的输入。
pub(crate) struct PacketPlanInput<'a> {
    pub canonical: &'a Value,
    pub candidate: &'a Value,
    pub differences: &'a [Value],
    pub blocking_issues: &'a [Value],
    pub protected: &'a BTreeSet<String>,
    pub source: &'a SourcePageIndex,
    pub edit_version: i64,
}

/// 题组索引：把「任意目标 id」解析成「它属于哪个题组」。
#[derive(Debug, Default)]
struct GroupIndex {
    /// taskId → 题组
    groups: BTreeMap<String, Value>,
    /// responseGroupId → taskId
    response_owner: BTreeMap<String, String>,
    /// slotId → taskId
    slot_owner: BTreeMap<String, String>,
    /// optionBankId → taskId
    option_bank_owner: BTreeMap<String, String>,
    /// taskId → 题号
    numbers: BTreeMap<String, Vec<u32>>,
}

impl GroupIndex {
    fn build(document: &Value) -> Self {
        let mut index = GroupIndex::default();
        let Some(groups) = document.get("taskGroups").and_then(Value::as_array) else {
            return index;
        };
        for group in groups {
            let Some(task_id) = group.get("taskId").and_then(Value::as_str) else {
                continue;
            };
            let task_id = task_id.to_string();
            index.numbers.insert(
                task_id.clone(),
                group
                    .get("displayRange")
                    .map(expand_question_numbers)
                    .unwrap_or_default(),
            );
            if let Some(bank_id) = group
                .pointer("/optionBank/optionBankId")
                .and_then(Value::as_str)
            {
                index
                    .option_bank_owner
                    .insert(bank_id.to_string(), task_id.clone());
            }
            for response in group
                .get("responseGroups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(response_id) = response.get("responseGroupId").and_then(Value::as_str) {
                    index
                        .response_owner
                        .insert(response_id.to_string(), task_id.clone());
                }
                for slot_id in response
                    .get("slotIds")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    index
                        .slot_owner
                        .insert(slot_id.to_string(), task_id.clone());
                }
            }
            index.groups.insert(task_id, group.clone());
        }
        index
    }

    /// 目标 → 题组。三种目标类型各自查各自的表。
    fn lookup(&self, target_type: &str, target_id: &str) -> Option<String> {
        match target_type {
            "task_group" => self.groups.contains_key(target_id).then(|| target_id.to_string()),
            "response_group" => self.response_owner.get(target_id).cloned(),
            "slot" => self.slot_owner.get(target_id).cloned(),
            _ => None,
        }
    }

    /// 题号区间与 `numbers` 有交集的题组。
    fn groups_intersecting(&self, numbers: &[u32]) -> BTreeSet<String> {
        let wanted: BTreeSet<u32> = numbers.iter().copied().collect();
        self.numbers
            .iter()
            .filter(|(_, own)| own.iter().any(|number| wanted.contains(number)))
            .map(|(task_id, _)| task_id.clone())
            .collect()
    }
}

/// 一条差异 / 一个阻塞问题归谁。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Owner {
    /// 一个或多个题组。
    Groups(BTreeSet<String>),
    /// 听力 Part：单独成包。
    Part(String),
    /// 文档级：没有可归属的题组，包只带索引。
    Document,
}

/// 差异归属解析：**先本地、再云端**。
///
/// 为什么要有「再云端」这一步：本地把 1-5 / 6-7 切成两组、云端把它们切成 1-7 一组时，
/// 差异的 `targetId` 是**云端的**题组 id，本地索引里根本没有它。直接归到「文档级」
/// 就等于把一次边界分歧降级成一条无目标的疑问——用户看到的是「云端读到了一些东西」，
/// 却看不到分歧本身。所以这里把云端题组的题号区间映射回本地题组，让两边都进同一个包。
fn owner_of(difference: &Value, canonical: &GroupIndex, candidate: &GroupIndex) -> Owner {
    let target_type = difference
        .get("targetType")
        .and_then(Value::as_str)
        .unwrap_or("");
    let target_id = difference.get("targetId").and_then(Value::as_str).unwrap_or("");
    if target_type == "part" {
        return Owner::Part(target_id.to_string());
    }
    if let Some(task_id) = canonical.lookup(target_type, target_id) {
        return Owner::Groups(BTreeSet::from([task_id]));
    }
    if let Some(candidate_task) = candidate.lookup(target_type, target_id) {
        let numbers = candidate
            .numbers
            .get(&candidate_task)
            .cloned()
            .unwrap_or_default();
        let owners = canonical.groups_intersecting(&numbers);
        if !owners.is_empty() {
            return Owner::Groups(owners);
        }
        // 云端有、本地完全没有的题号（新增题组）：仍然带着这个差异，但只能落进文档包。
        return Owner::Document;
    }
    Owner::Document
}

/// 阻塞质量问题的归属：按 `targetIds` 找题组，找不到就是文档级。
fn owner_of_issue(issue: &Value, canonical: &GroupIndex, candidate: &GroupIndex) -> Owner {
    let owners: BTreeSet<String> = issue
        .get("targetIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|target_id| {
            canonical
                .lookup("slot", target_id)
                .or_else(|| canonical.lookup("response_group", target_id))
                .or_else(|| canonical.lookup("task_group", target_id))
                .or_else(|| {
                    candidate.lookup("slot", target_id).and_then(|candidate_task| {
                        let numbers = candidate
                            .numbers
                            .get(&candidate_task)
                            .cloned()
                            .unwrap_or_default();
                        canonical
                            .groups_intersecting(&numbers)
                            .into_iter()
                            .next()
                    })
                })
        })
        .collect();
    if owners.is_empty() {
        Owner::Document
    } else {
        Owner::Groups(owners)
    }
}

/// 并查集：题组合并（共用选项库 / 同一段 stimulus / 切法不同）。
#[derive(Debug, Default)]
struct UnionFind {
    parent: BTreeMap<String, String>,
}

impl UnionFind {
    fn add(&mut self, key: &str) {
        self.parent
            .entry(key.to_string())
            .or_insert_with(|| key.to_string());
    }

    fn find(&mut self, key: &str) -> String {
        self.add(key);
        let parent = self.parent.get(key).cloned().unwrap_or_else(|| key.to_string());
        if parent == key {
            return parent;
        }
        let root = self.find(&parent);
        self.parent.insert(key.to_string(), root.clone());
        root
    }

    fn union(&mut self, left: &str, right: &str) {
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root != right_root {
            self.parent.insert(right_root, left_root);
        }
    }

    fn components(&mut self) -> BTreeMap<String, BTreeSet<String>> {
        let keys: Vec<String> = self.parent.keys().cloned().collect();
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for key in keys {
            let root = self.find(&key);
            out.entry(root).or_default().insert(key);
        }
        out
    }
}

/// 把题组合并成包。
///
/// 合并的三条判据（任务书 4.1 规则 2）：
/// 1. **共用选项库**：两组挂同一个 `optionBankId`，拆开就会让模型看不到选项归属；
/// 2. **同一段 stimulus**：一段 notes 被本地切成两组时，两组各自看到半段材料，
///    谁都无法判断整段是否读对；
/// 3. **本地与云端切法不同**：本地 1-5 / 6-7、云端 1-7 时，分歧**就是边界本身**，
///    拆开等于让模型在看不到完整边界的情况下评价边界。
fn merge_components(canonical: &GroupIndex, candidate: &GroupIndex) -> Vec<BTreeSet<String>> {
    let mut union = UnionFind::default();
    for task_id in canonical.groups.keys() {
        union.add(task_id);
    }
    // 1) 共用选项库。
    let mut by_bank: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (task_id, group) in &canonical.groups {
        if let Some(bank_id) = group
            .pointer("/optionBank/optionBankId")
            .and_then(Value::as_str)
        {
            by_bank
                .entry(bank_id.to_string())
                .or_default()
                .push(task_id.clone());
        }
    }
    for members in by_bank.values() {
        for pair in members.windows(2) {
            union.union(&pair[0], &pair[1]);
        }
    }
    // 2) 同一段 stimulus（非空且规范化后相同）。
    let mut by_stimulus: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (task_id, group) in &canonical.groups {
        let text = nodes_text(group.get("stimulus").unwrap_or(&Value::Null));
        let key = normalize_text(&text);
        if key.is_empty() {
            continue;
        }
        by_stimulus.entry(key).or_default().push(task_id.clone());
    }
    for members in by_stimulus.values() {
        for pair in members.windows(2) {
            union.union(&pair[0], &pair[1]);
        }
    }
    // 3) 云端题组的题号区间落在多个本地题组上 ⇒ 这些本地题组必须在一起。
    for (_, numbers) in &candidate.numbers {
        let intersecting = canonical.groups_intersecting(numbers);
        let members: Vec<String> = intersecting.into_iter().collect();
        for pair in members.windows(2) {
            union.union(&pair[0], &pair[1]);
        }
    }
    let mut components: Vec<BTreeSet<String>> = union.components().into_values().collect();
    // 组内按题号排序，保证包与包的顺序稳定（诊断与测试都要能比较）。
    for component in components.iter_mut() {
        let mut ordered: Vec<String> = component.iter().cloned().collect();
        ordered.sort_by_key(|task_id| {
            canonical
                .numbers
                .get(task_id)
                .and_then(|numbers| numbers.first().copied())
                .unwrap_or(u32::MAX)
        });
        *component = ordered.into_iter().collect();
    }
    components.sort_by(|left, right| {
        let key = |component: &BTreeSet<String>| {
            component
                .iter()
                .filter_map(|task_id| canonical.numbers.get(task_id))
                .filter_map(|numbers| numbers.first().copied())
                .min()
                .unwrap_or(u32::MAX)
        };
        key(left).cmp(&key(right))
    });
    components
}

/// 差异的类别，决定排序与「要不要带答案页」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DifferenceKind {
    /// 阻断问题：最先处理。
    Blocking,
    /// 答案类差异：必须带答案页。
    Answer,
    /// 文本 / 结构类差异。
    Text,
}

fn difference_kind(difference: &Value) -> DifferenceKind {
    let target_type = difference
        .get("targetType")
        .and_then(Value::as_str)
        .unwrap_or("");
    let field = difference.get("field").and_then(Value::as_str).unwrap_or("");
    if target_type == "slot" && field == "answer" {
        DifferenceKind::Answer
    } else {
        DifferenceKind::Text
    }
}

/// 一条锚点的页与 bbox（1-based 页号）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct AnchorPage {
    page: u32,
    /// `(top, bottom)`，已按 origin 归一到「从上往下」。缺 bbox 时为 `None`。
    edges: Option<(f64, f64)>,
}

/// 从任意节点树里收集 `sourceAnchors` 覆盖的页（换算到 **1-based**）。
///
/// 换算只在这里做一次：权威稿锚点是 0-based，页图与 `read_source` 是 1-based。
/// 两处各转一次是这类代码最经典的错法——模型会引到隔壁页的句子，而引用看起来「有出处」。
fn anchor_pages(value: &Value, out: &mut Vec<AnchorPage>) {
    match value {
        Value::Array(items) => {
            for item in items {
                anchor_pages(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(anchors) = map.get("sourceAnchors").and_then(Value::as_array) {
                for anchor in anchors {
                    let Some(zero_based) = anchor.get("pageIndex").and_then(Value::as_i64) else {
                        continue;
                    };
                    if zero_based < 0 {
                        continue;
                    }
                    let edges = anchor.get("bbox").and_then(|bbox| {
                        let x = bbox.get("x")?.as_f64()?;
                        let y = bbox.get("y")?.as_f64()?;
                        let width = bbox.get("width")?.as_f64()?;
                        let height = bbox.get("height")?.as_f64()?;
                        let _ = x;
                        let _ = width;
                        let bottom_left =
                            bbox.get("origin").and_then(Value::as_str) == Some("bottom-left");
                        Some(if bottom_left {
                            (y + height, y)
                        } else {
                            (y, y + height)
                        })
                    });
                    out.push(AnchorPage {
                        page: zero_based as u32 + 1,
                        edges,
                    });
                }
            }
            for child in map.values() {
                anchor_pages(child, out);
            }
        }
        _ => {}
    }
}

fn anchor_pages_of(document: &Value, task_ids: &BTreeSet<String>) -> Vec<AnchorPage> {
    let mut out = Vec::new();
    for group in document
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(task_id) = group.get("taskId").and_then(Value::as_str) else {
            continue;
        };
        if !task_ids.contains(task_id) {
            continue;
        }
        anchor_pages(group, &mut out);
    }
    out
}

/// 已知的最大页号（文本层与页图取并集）。两者都空时返回 `None`。
fn max_known_page(source: &SourcePageIndex) -> Option<u32> {
    let from_lines = source.lines.keys().next_back().copied();
    let from_images = source.page_images.keys().next_back().copied();
    match (from_lines, from_images) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}

/// 锚点贴页边 ⇒ 题目跨页 ⇒ 左右各扩一页。
///
/// 两侧都要夹住已知页范围：往回扩要 `page > 1`，往外扩要 `page < 最后一页`。
/// 少了外侧上界就会把不存在的页写进 `scope.pages`，模型照它去取只会拿到「不存在」。
fn expand_edge_pages(pages: &[u32], anchors: &[AnchorPage], source: &SourcePageIndex) -> Vec<u32> {
    let mut out: BTreeSet<u32> = pages.iter().copied().collect();
    let last_page = max_known_page(source);
    for anchor in anchors {
        let Some((top, bottom)) = anchor.edges else {
            continue;
        };
        let Some(image) = source.page_images.get(&anchor.page) else {
            continue;
        };
        if image.height <= 0.0 {
            continue;
        }
        let margin = image.height * PAGE_EDGE_RATIO;
        let touches_edge = top <= margin || bottom >= image.height - margin;
        if !touches_edge {
            continue;
        }
        if anchor.page > 1 {
            out.insert(anchor.page - 1);
        }
        if last_page.is_some_and(|last| anchor.page < last) {
            out.insert(anchor.page + 1);
        }
    }
    out.into_iter().collect()
}

/// 答案页定位：**先用答案页识别产物，再退回文本层搜索**。
///
/// 定位不到时如实返回 `false`（调用方写 `answerPagesUnknown`），**不猜**——
/// 猜出来的答案页会让模型在错误的页上「确认」答案，那比承认不知道糟得多。
fn locate_answer_pages(
    source: &SourcePageIndex,
    question_numbers: &[u32],
) -> (Vec<u32>, bool) {
    if !source.answer_pages.is_empty() {
        let mut pages = source.answer_pages.clone();
        pages.sort_unstable();
        pages.dedup();
        return (pages, true);
    }
    if question_numbers.is_empty() {
        return (Vec::new(), false);
    }
    // 文本层搜索：一页里出现「题号 + 空格 + 值」的行达到两条以上，才认它是答案区。
    let wanted: BTreeSet<u32> = question_numbers.iter().copied().collect();
    let mut found = Vec::new();
    for (page, lines) in &source.lines {
        let matches = lines
            .iter()
            .filter(|line| {
                let trimmed = line.text.trim_start();
                let digits: String = trimmed.chars().take_while(char::is_ascii_digit).collect();
                let Ok(number) = digits.parse::<u32>() else {
                    return false;
                };
                if !wanted.contains(&number) {
                    return false;
                }
                // 题号后面必须还有内容，否则「14」这样的孤行更像页码或题面。
                trimmed[digits.len()..].trim().len() >= 1
            })
            .count();
        if matches >= 2 {
            found.push(*page);
        }
    }
    found.sort_unstable();
    found.dedup();
    let known = !found.is_empty();
    (found, known)
}

/// 估算一段 JSON 的输入量（token）。
fn estimate_tokens(value: &Value, images: usize) -> usize {
    let chars = serde_json::to_string(value)
        .map(|text| text.chars().count())
        .unwrap_or(0);
    chars / PACKET_CHARS_PER_TOKEN + images * PACKET_IMAGE_TOKENS
}

/// 整卷极简索引：让模型知道「其他内容在哪」，但不带其他内容本身。
fn paper_map(
    canonical: &Value,
    source: &SourcePageIndex,
    canonical_index: &GroupIndex,
) -> Value {
    let groups: Vec<Value> = canonical
        .get("taskGroups")
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter_map(|group| {
                    let task_id = group.get("taskId").and_then(Value::as_str)?;
                    let numbers = canonical_index
                        .numbers
                        .get(task_id)
                        .cloned()
                        .unwrap_or_default();
                    let mut pages = Vec::new();
                    let mut anchors = Vec::new();
                    anchor_pages(group, &mut anchors);
                    for anchor in anchors {
                        if !pages.contains(&anchor.page) {
                            pages.push(anchor.page);
                        }
                    }
                    pages.sort_unstable();
                    Some(json!({
                        "taskId": task_id,
                        "numbers": numbers,
                        "taskType": group.get("taskType").cloned().unwrap_or(Value::Null),
                        "pages": pages,
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    // 每页上有什么：题号 / 文章段落 / 答案区。只写类别，不写内容。
    let mut page_tags: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for group in groups.iter() {
        let Some(page_list) = group.get("pages").and_then(Value::as_array) else {
            continue;
        };
        let numbers = group.get("numbers").and_then(Value::as_array).cloned().unwrap_or_default();
        for page in page_list.iter().filter_map(Value::as_u64) {
            let entry = page_tags.entry(page as u32).or_default();
            for number in &numbers {
                let tag = format!("q{number}");
                if !entry.contains(&tag) {
                    entry.push(tag);
                }
            }
        }
    }
    for page in source.answer_pages.iter() {
        let entry = page_tags.entry(*page).or_default();
        if !entry.iter().any(|tag| tag == "answers") {
            entry.push("answers".to_string());
        }
    }
    for page in source.lines.keys() {
        page_tags.entry(*page).or_default();
    }
    let pages: Vec<Value> = page_tags
        .into_iter()
        .map(|(page, mut tags)| {
            tags.sort();
            if tags.is_empty() {
                tags.push("passage".to_string());
            }
            json!({
                "page": page,
                "has": tags,
                "paragraphLabels": paragraph_labels_on_page(source, page),
            })
        })
        .collect();
    json!({"taskGroups": groups, "pages": pages})
}

/// 这一页上**看起来像段落标号**的行首字母（`C` / `C.`）。
///
/// 判据刻意与 [`super::grab::read_passage`] 的标签匹配保持一致：模型据 `paperMap`
/// 知道「这一页有 C 段」，再用 `read_passage {"paragraphLabels":["C"]}` 就一定取得到。
/// 两边判据不同才是真正危险的事——`paperMap` 说有、工具说没有，模型只会乱猜。
fn paragraph_labels_on_page(source: &SourcePageIndex, page: u32) -> Vec<String> {
    let mut labels: BTreeSet<String> = BTreeSet::new();
    for line in source.lines.get(&page).into_iter().flatten() {
        let head: String = line
            .text
            .trim_start()
            .to_ascii_uppercase()
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric())
            .collect();
        if head.len() == 1 && head.chars().all(|ch| ch.is_ascii_uppercase()) {
            labels.insert(head);
        }
    }
    labels.into_iter().collect()
}

/// 一个包的内部形态（用于拆包与预算判断）。
#[derive(Debug, Clone)]
struct PacketDraft {
    /// 包内题组（文档包为空）。
    task_ids: BTreeSet<String>,
    /// 听力 Part（单独成包时非空）。
    part_id: Option<String>,
    differences: Vec<Value>,
    blocking_issues: Vec<Value>,
    document_only: bool,
}

impl PacketDraft {
    fn kind(&self) -> DifferenceKind {
        if !self.blocking_issues.is_empty() {
            DifferenceKind::Blocking
        } else if self
            .differences
            .iter()
            .any(|difference| difference_kind(difference) == DifferenceKind::Answer)
        {
            DifferenceKind::Answer
        } else {
            DifferenceKind::Text
        }
    }

    fn numbers(&self, canonical_index: &GroupIndex) -> Vec<u32> {
        let mut numbers: Vec<u32> = self
            .task_ids
            .iter()
            .filter_map(|task_id| canonical_index.numbers.get(task_id))
            .flatten()
            .copied()
            .collect();
        numbers.sort_unstable();
        numbers.dedup();
        numbers
    }
}

/// 切分：先归属、再合并、再按预算拆分、最后排序。
///
/// 返回的是**包的 JSON 列表**（`RepairPacketV1` 的形状），调用方按序逐包推进。
/// `packets[].sourceEvidence.regions[].image` 在裁剪完成前是 `null`——
/// 由 [`super::grab::materialize_regions`] 填上真实路径。
pub(crate) fn plan_packets(input: &PacketPlanInput<'_>) -> Vec<Value> {
    let canonical_index = GroupIndex::build(input.canonical);
    let candidate_index = GroupIndex::build(input.candidate);

    // ── 1) 归属 ──────────────────────────────────────────────────────────
    let mut group_differences: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut group_issues: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut part_differences: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut document_differences: Vec<Value> = Vec::new();
    let mut document_issues: Vec<Value> = Vec::new();

    for difference in input.differences {
        match owner_of(difference, &canonical_index, &candidate_index) {
            Owner::Groups(task_ids) => {
                for task_id in task_ids {
                    group_differences
                        .entry(task_id)
                        .or_default()
                        .push(difference.clone());
                }
            }
            Owner::Part(part_id) => {
                part_differences
                    .entry(part_id)
                    .or_default()
                    .push(difference.clone());
            }
            Owner::Document => document_differences.push(difference.clone()),
        }
    }
    for issue in input.blocking_issues {
        match owner_of_issue(issue, &canonical_index, &candidate_index) {
            Owner::Groups(task_ids) => {
                for task_id in task_ids {
                    group_issues.entry(task_id).or_default().push(issue.clone());
                }
            }
            _ => document_issues.push(issue.clone()),
        }
    }

    // ── 2) 合并 ──────────────────────────────────────────────────────────
    let mut drafts: Vec<PacketDraft> = Vec::new();
    for component in merge_components(&canonical_index, &candidate_index) {
        let mut draft = PacketDraft {
            task_ids: component.clone(),
            part_id: None,
            differences: Vec::new(),
            blocking_issues: Vec::new(),
            document_only: false,
        };
        for task_id in &component {
            if let Some(entries) = group_differences.get(task_id) {
                draft.differences.extend(entries.iter().cloned());
            }
            if let Some(entries) = group_issues.get(task_id) {
                draft.blocking_issues.extend(entries.iter().cloned());
            }
        }
        // 合并后可能产生完全重复的差异（一条差异归到多个题组）：按身份去重。
        dedupe(&mut draft.differences);
        dedupe(&mut draft.blocking_issues);
        // **没有差异也没有阻断问题的题组不成包**：给它开一个包只会让模型去核一份
        // 本来就没有分歧的内容，白烧一轮预算。
        if draft.differences.is_empty() && draft.blocking_issues.is_empty() {
            continue;
        }
        drafts.push(draft);
    }
    for (part_id, entries) in part_differences {
        let task_ids = input
            .candidate
            .get("listening")
            .and_then(|listening| listening.get("parts"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|part| part.get("partId").and_then(Value::as_str) == Some(part_id.as_str()))
            .and_then(|part| part.get("taskIds").and_then(Value::as_array))
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .filter(|task_id| canonical_index.groups.contains_key(*task_id))
                    .map(str::to_string)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        drafts.push(PacketDraft {
            task_ids,
            part_id: Some(part_id),
            differences: entries,
            blocking_issues: Vec::new(),
            document_only: false,
        });
    }
    if !document_differences.is_empty() || !document_issues.is_empty() {
        drafts.push(PacketDraft {
            task_ids: BTreeSet::new(),
            part_id: None,
            differences: document_differences,
            blocking_issues: document_issues,
            document_only: true,
        });
    }
    // 一条差异都没有时也要给模型**一次**机会：它可能读到原文件后发现「本地和候选都错了」，
    // 也可能留下疑问。这个包只带索引与诊断，不带任何稿件内容，代价很小。
    //
    // 注意（审计发现 A-11）：文档包 `task_ids` 是空的，因此 `read_draft` 在文档包里**永远
    // 会被拒**（`mod.rs::scope_error`：空选择器 → `CLOUD_DRAFT_SCOPE_REQUIRED`）。这是有意
    // 的——文档包没有「这个包内的稿件切片」可读，`draftSlice` 本身也是空的。想读稿的模型
    // 必须用题号/题组 id 去 `read_draft`，而那会把目标落到真正拥有它的那个包上。
    // 该行为由 `tests::a_document_packet_never_hands_out_a_draft_slice` 固定。
    if drafts.is_empty() {
        drafts.push(PacketDraft {
            task_ids: BTreeSet::new(),
            part_id: None,
            differences: Vec::new(),
            blocking_issues: Vec::new(),
            document_only: true,
        });
    }

    // ── 3) 拆分（超预算时按 responseGroup）──────────────────────────────
    let mut split: Vec<PacketDraft> = Vec::new();
    for draft in drafts {
        let size = draft_size(&draft, input, &canonical_index, &candidate_index);
        if size <= PACKET_TOKEN_BUDGET || draft.task_ids.len() <= 1 {
            split.push(draft);
            continue;
        }
        split.extend(split_by_response_group(
            &draft,
            input,
            &canonical_index,
            &candidate_index,
        ));
    }

    // ── 4) 排序：阻断 → 答案 → 文本 ─────────────────────────────────────
    split.sort_by_key(|draft| (draft.kind(), draft.numbers(&canonical_index)));

    // ── 5) 组装 ─────────────────────────────────────────────────────────
    split
        .iter()
        .map(|draft| build_packet(draft, input, &canonical_index, &candidate_index))
        .collect()
}

/// 包的稳定 id：由「本地题组 + 差异键 + 阻断问题 + 是否文档包」派生。
///
/// 为什么不能用序号：`apply_edits` 之后要**重切受影响的包**（任务书 §4.3），而序号会随
/// 重排整体漂移——「哪些包已经做完」「哪条裁定属于哪个包」于是全部错位。用身份派生，
/// 内容没变 id 就不变；内容变了才换 id，而那本来就该当成另一个包。
fn packet_id_for(draft: &PacketDraft) -> String {
    let mut parts: Vec<String> = Vec::new();
    if draft.document_only {
        parts.push("document".to_string());
    }
    if let Some(part_id) = &draft.part_id {
        parts.push(format!("part:{part_id}"));
    }
    for task_id in &draft.task_ids {
        parts.push(format!("task:{task_id}"));
    }
    let mut differences: Vec<String> = draft
        .differences
        .iter()
        .map(|difference| {
            format!(
                "{}:{}:{}",
                difference.get("targetType").and_then(Value::as_str).unwrap_or(""),
                difference.get("targetId").and_then(Value::as_str).unwrap_or(""),
                difference.get("field").and_then(Value::as_str).unwrap_or(""),
            )
        })
        .collect();
    differences.sort();
    differences.dedup();
    parts.extend(differences.into_iter().map(|key| format!("diff:{key}")));
    let mut issues: Vec<String> = draft
        .blocking_issues
        .iter()
        .map(|issue| {
            issue
                .get("issueId")
                .or_else(|| issue.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        })
        .collect();
    issues.sort();
    issues.dedup();
    parts.extend(issues.into_iter().map(|key| format!("issue:{key}")));
    format!("pkt-{:08x}", fnv1a(&parts.join("|")))
}

/// FNV-1a。只为「同一份身份稳定给出同一个 id」，不需要抗碰撞强度。
fn fnv1a(text: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// 差异去重：合并后同一个题组可能被多个来源写进来同一条差异。
fn dedupe(entries: &mut Vec<Value>) {
    let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
    entries.retain(|entry| {
        let key = (
            entry.get("targetType").and_then(Value::as_str).unwrap_or("").to_string(),
            entry.get("targetId").and_then(Value::as_str).unwrap_or("").to_string(),
            entry.get("field").and_then(Value::as_str).unwrap_or("").to_string(),
        );
        seen.insert(key)
    });
}

/// 粗略估算一个包的输入量。
fn draft_size(
    draft: &PacketDraft,
    input: &PacketPlanInput<'_>,
    canonical_index: &GroupIndex,
    candidate_index: &GroupIndex,
) -> usize {
    let (pages, _) = scope_pages(draft, input, canonical_index, candidate_index);
    let lines: usize = pages
        .iter()
        .map(|page| input.source.lines.get(page).map(Vec::len).unwrap_or(0))
        .sum();
    let payload = json!({
        "differences": draft.differences,
        "blockingIssues": draft.blocking_issues,
        "groups": draft.task_ids.len(),
        "lines": lines,
    });
    estimate_tokens(&payload, pages.len())
}

/// 超预算时按 responseGroup 拆开；拆开后每个子包都附上共用的选项库与说明文字
/// （见 [`build_packet`]：`draftSlice` 里题组是整组带上的）。
fn split_by_response_group(
    draft: &PacketDraft,
    input: &PacketPlanInput<'_>,
    canonical_index: &GroupIndex,
    candidate_index: &GroupIndex,
) -> Vec<PacketDraft> {
    let _ = (input, candidate_index);
    let mut out: Vec<PacketDraft> = Vec::new();
    let mut group_level = PacketDraft {
        task_ids: draft.task_ids.clone(),
        part_id: draft.part_id.clone(),
        differences: Vec::new(),
        blocking_issues: draft.blocking_issues.clone(),
        document_only: false,
    };
    // 目标 → 它属于哪个 responseGroup（用于把差异分到子包）。
    let mut response_of_slot: BTreeMap<String, String> = BTreeMap::new();
    for task_id in &draft.task_ids {
        let Some(group) = canonical_index.groups.get(task_id) else {
            continue;
        };
        for response in group
            .get("responseGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(response_id) = response.get("responseGroupId").and_then(Value::as_str) else {
                continue;
            };
            for slot_id in response
                .get("slotIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                response_of_slot.insert(slot_id.to_string(), response_id.to_string());
            }
        }
    }
    let mut by_response: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for difference in draft.differences.iter().cloned() {
        let target_id = difference
            .get("targetId")
            .and_then(Value::as_str)
            .unwrap_or("");
        match response_of_slot.get(target_id) {
            Some(response_id) => by_response
                .entry(response_id.clone())
                .or_default()
                .push(difference),
            // 题组级差异（instructions / stimulus / option_bank / 边界）：它们说的是
            // 整组的事，分给任何单个子包都会让另外几个子包看不到。单独一个子包。
            None => group_level.differences.push(difference),
        }
    }
    for (_, differences) in by_response {
        out.push(PacketDraft {
            task_ids: draft.task_ids.clone(),
            part_id: draft.part_id.clone(),
            differences,
            blocking_issues: Vec::new(),
            document_only: false,
        });
    }
    if !group_level.differences.is_empty() || !group_level.blocking_issues.is_empty() {
        out.push(group_level);
    }
    if out.is_empty() {
        out.push(draft.clone());
    }
    out
}

/// 本包的原文范围（1-based 页）。
fn scope_pages(
    draft: &PacketDraft,
    input: &PacketPlanInput<'_>,
    canonical_index: &GroupIndex,
    candidate_index: &GroupIndex,
) -> (Vec<u32>, bool) {
    let canonical_anchors = anchor_pages_of(input.canonical, &draft.task_ids);
    let mut pages: BTreeSet<u32> = canonical_anchors.iter().map(|anchor| anchor.page).collect();

    // 云端候选在同样目标上的引文页。
    let candidate_anchors = anchor_pages_of(input.candidate, &draft.task_ids);
    for anchor in &candidate_anchors {
        pages.insert(anchor.page);
    }
    // 云端自己声明「读不全」的区域：只带贴近本包范围的，否则每个包都要背上整卷的缺口。
    if let Some(regions) = input
        .candidate
        .get("unresolvedRegions")
        .and_then(Value::as_array)
    {
        let span = page_span(&pages);
        for region in regions {
            let Some(page) = region.get("pageIndex").and_then(Value::as_u64) else {
                continue;
            };
            let page = page as u32;
            if let Some((low, high)) = span {
                if page + 1 >= low && page.saturating_sub(1) <= high {
                    pages.insert(page);
                }
            } else {
                pages.insert(page);
            }
        }
    }

    // 答案类差异 ⇒ 必须带答案页。定位不到时如实记下来（由调用方写 answerPagesUnknown）。
    let numbers = draft.numbers(canonical_index);
    let needs_answer_pages = draft
        .differences
        .iter()
        .any(|difference| difference_kind(difference) == DifferenceKind::Answer);
    let mut answer_pages_known = true;
    if needs_answer_pages {
        let (answer_pages, known) = locate_answer_pages(input.source, &numbers);
        answer_pages_known = known;
        for page in answer_pages {
            pages.insert(page);
        }
    }

    let mut anchors = canonical_anchors;
    anchors.extend(candidate_anchors);
    let mut pages = expand_edge_pages(&pages.into_iter().collect::<Vec<_>>(), &anchors, input.source);
    pages.sort_unstable();
    pages.dedup();
    let _ = candidate_index;
    (pages, answer_pages_known)
}

fn page_span(pages: &BTreeSet<u32>) -> Option<(u32, u32)> {
    let low = pages.iter().next().copied()?;
    let high = pages.iter().next_back().copied()?;
    Some((low, high))
}

/// 组装一个包的 JSON。
fn build_packet(
    draft: &PacketDraft,
    input: &PacketPlanInput<'_>,
    canonical_index: &GroupIndex,
    candidate_index: &GroupIndex,
) -> Value {
    let packet_id = packet_id_for(draft);
    let numbers = draft.numbers(canonical_index);
    let (pages, answer_pages_known) = scope_pages(draft, input, canonical_index, candidate_index);
    let needs_answer_pages = draft
        .differences
        .iter()
        .any(|difference| difference_kind(difference) == DifferenceKind::Answer);
    let (answer_pages, _) = if needs_answer_pages {
        locate_answer_pages(input.source, &numbers)
    } else {
        (Vec::new(), true)
    };

    let task_ids: Vec<String> = draft.task_ids.iter().cloned().collect();
    let draft_slice = if draft.document_only {
        json!({
            "editVersion": input.edit_version,
            "taskGroups": [],
            "answerSlots": {},
            "answerKey": {},
            "note": "This packet has no task-group target; it only carries the paper index and the listed problems.",
        })
    } else {
        super::read_draft_section(
            input.canonical,
            input.edit_version,
            &json!({"taskGroupIds": task_ids}),
        )
    };
    let candidate_slice = if draft.document_only {
        Value::Null
    } else {
        super::read_draft_section(
            input.candidate,
            0,
            &json!({"questionNumbers": numbers, "taskGroupIds": task_ids}),
        )
    };

    // 原文证据：范围内各页的文本层（逐行带稳定 id）。
    let source_pages: Vec<Value> = pages
        .iter()
        .map(|page| {
            let lines: Vec<Value> = input
                .source
                .lines
                .get(page)
                .map(|lines| {
                    lines
                        .iter()
                        .map(|line| json!({"id": line.id, "text": line.text}))
                        .collect()
                })
                .unwrap_or_default();
            json!({"pageIndex": page, "lines": lines})
        })
        .collect();

    // 区域图请求：题组锚点的 bbox 外扩后的裁剪请求；锚点缺失时由调用方改用整页图。
    let regions = region_requests(draft, input, canonical_index);

    let protected: Vec<String> = input
        .protected
        .iter()
        .filter(|target| {
            draft.document_only
                || draft.task_ids.iter().any(|task_id| *task_id == **target)
                || numbers.iter().any(|number| target.contains(&format!("q{number}")))
        })
        .cloned()
        .collect();

    let manifest = scope_manifest(draft, input, &pages, &numbers, &source_pages);

    let mut packet = json!({
        "contextMode": "packets",
        "schemaVersion": "RepairPacketV1",
        "packetId": packet_id,
        "escalationLevel": 0,
        "taskIds": task_ids,
        "questionNumbers": numbers,
        "partId": draft.part_id,
        "documentOnly": draft.document_only,
        "paperMap": paper_map(input.canonical, input.source, canonical_index),
        "scopeManifest": manifest,
        "scope": {
            "pages": pages,
            "answerPages": answer_pages,
            // 定位不到就明说。空数组 + `true` 会让模型以为「这份卷子没有答案页」。
            "answerPagesUnknown": !answer_pages_known,
        },
        "differences": draft.differences,
        "blockingIssues": draft.blocking_issues,
        "protectedTargets": protected,
        "draftSlice": draft_slice,
        "candidateSlice": candidate_slice,
        "sourceEvidence": {
            "kind": input.source.kind,
            "sourceFileId": input.source.source_file_id,
            "pages": source_pages,
            "paragraphs": paragraph_windows(draft, input, canonical_index),
            "regions": regions,
        },
    });
    let images = packet
        .pointer("/sourceEvidence/regions")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let estimated = estimate_tokens(&packet, images);
    if let Some(object) = packet.as_object_mut() {
        object.insert("estimatedInputTokens".to_string(), json!(estimated));
    }
    packet
}

/// 题组锚点 → 裁剪请求（pageIndex + bbox）。同一页多个锚点只留一个区域请求。
///
/// 缺 bbox 的**那一页**退整页图，判据是「**这一组**在这一页有没有 bbox」，不是
/// 「这一页在整个包里有没有 bbox」：
/// - 按整组判时（A-7 之前的写法），「同组里 2 页有 bbox、1 页没有」的那一页会既没有
///   区域图也没有整页图，而 `scope.pages` 里明明写着它；
/// - 按草稿判时（A-7 之后、本条修正之前的写法），同一个包里**另一个**题组在这一页裁了
///   区域图，就把本组这一页的整页图退路一起吞掉 —— 而那两条 bbox 覆盖的版面未必重叠，
///   本组要核的内容可能整块落在对方的裁剪范围之外。判据下沉到组内后结果也不再随
///   `draft.task_ids` 的字典序遍历顺序摇摆。
///
/// 去重（`seen`）仍然是跨组的：同一页的整页图只需要一张。
fn region_requests(
    draft: &PacketDraft,
    input: &PacketPlanInput<'_>,
    canonical_index: &GroupIndex,
) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut seen: BTreeSet<(u32, String)> = BTreeSet::new();
    for task_id in &draft.task_ids {
        let Some(group) = canonical_index.groups.get(task_id) else {
            continue;
        };
        let mut anchors = Vec::new();
        anchor_pages(group, &mut anchors);
        let mut bboxes = Vec::new();
        collect_bboxes(group, &mut bboxes);
        // 本组已经有区域图的页：本组自己的锚点页不再退整页图。
        let mut covered: BTreeSet<u32> = BTreeSet::new();
        for (page, bbox) in bboxes.iter() {
            covered.insert(*page);
            let key = (*page, serde_json::to_string(bbox).unwrap_or_default());
            if !seen.insert(key) {
                continue;
            }
            out.push(json!({
                "pageIndex": page,
                "bbox": bbox.clone(),
                "taskId": task_id,
                "image": Value::Null,
            }));
        }
        for anchor in anchors {
            if covered.contains(&anchor.page) {
                continue;
            }
            if !seen.insert((anchor.page, String::new())) {
                continue;
            }
            out.push(json!({
                "pageIndex": anchor.page,
                "bbox": Value::Null,
                "taskId": task_id,
                "image": Value::Null,
            }));
        }
    }
    let _ = input;
    out
}

/// 收集节点树里的 bbox（0-based 页 → 1-based 页）。
fn collect_bboxes(value: &Value, out: &mut Vec<(u32, Value)>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_bboxes(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(anchors) = map.get("sourceAnchors").and_then(Value::as_array) {
                for anchor in anchors {
                    let Some(zero_based) = anchor.get("pageIndex").and_then(Value::as_i64) else {
                        continue;
                    };
                    if zero_based < 0 {
                        continue;
                    }
                    let Some(bbox) = anchor.get("bbox") else {
                        continue;
                    };
                    if bbox.is_null() {
                        continue;
                    }
                    out.push((zero_based as u32 + 1, bbox.clone()));
                }
            }
            for child in map.values() {
                collect_bboxes(child, out);
            }
        }
        _ => {}
    }
}

/// DOCX：按段落 id 给出目标附近的段落窗口。
fn paragraph_windows(
    draft: &PacketDraft,
    input: &PacketPlanInput<'_>,
    canonical_index: &GroupIndex,
) -> Vec<Value> {
    if input.source.paragraphs.is_empty() {
        return Vec::new();
    }
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for task_id in &draft.task_ids {
        let Some(group) = canonical_index.groups.get(task_id) else {
            continue;
        };
        let mut ids = Vec::new();
        collect_paragraph_ids(group, &mut ids);
        wanted.extend(ids);
    }
    if wanted.is_empty() {
        return Vec::new();
    }
    let positions: BTreeMap<&str, usize> = input
        .source
        .paragraphs
        .iter()
        .enumerate()
        .map(|(index, paragraph)| (paragraph.id.as_str(), index))
        .collect();
    let mut keep: BTreeSet<usize> = BTreeSet::new();
    for id in &wanted {
        if let Some(position) = positions.get(id.as_str()) {
            for offset in position.saturating_sub(2)..=(position + 2).min(input.source.paragraphs.len().saturating_sub(1)) {
                keep.insert(offset);
            }
        }
    }
    keep.into_iter()
        .map(|index| {
            let paragraph = &input.source.paragraphs[index];
            json!({"id": paragraph.id, "text": paragraph.text})
        })
        .collect()
}

fn collect_paragraph_ids(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_paragraph_ids(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(id) = map.get("id").and_then(Value::as_str) {
                if map.get("type").and_then(Value::as_str).is_some() {
                    out.push(id.to_string());
                }
            }
            for child in map.values() {
                collect_paragraph_ids(child, out);
            }
        }
        _ => {}
    }
}

/// `scopeManifest`：**放进来了什么、省略了什么、省略的内容用哪个工具能取到**。
///
/// 这一段不是说明文字，是契约的一部分：模型必须能据此知道「我现在看不到文章正文，
/// 要看就用 `read_passage`」，否则它只能凭印象下结论。
fn scope_manifest(
    draft: &PacketDraft,
    input: &PacketPlanInput<'_>,
    pages: &[u32],
    numbers: &[u32],
    source_pages: &[Value],
) -> Value {
    let included = json!({
        "pages": pages,
        "lines": source_pages
            .iter()
            .map(|page| page.get("lines").and_then(Value::as_array).map(Vec::len).unwrap_or(0))
            .sum::<usize>(),
        "taskGroups": draft.task_ids.iter().cloned().collect::<Vec<_>>(),
        "questionNumbers": numbers,
        "differences": draft.differences.len(),
        "blockingIssues": draft.blocking_issues.len(),
    });

    let mut omitted: Vec<Value> = Vec::new();
    let total_pages = input.source.lines.len();
    if total_pages > pages.len() {
        omitted.push(json!({
            "what": format!("{} source pages outside this packet", total_pages - pages.len()),
            "howToFetch": "read_source with a page range (at most 3 pages per call), or search_source with a quote you remember",
        }));
    }
    let total_groups = input
        .canonical
        .get("taskGroups")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if total_groups > draft.task_ids.len() {
        omitted.push(json!({
            "what": format!(
                "{} task groups outside this packet",
                total_groups - draft.task_ids.len()
            ),
            "howToFetch": "read_draft with questionNumbers, or read_candidate with taskIds",
        }));
    }
    omitted.push(json!({
        "what": "the reading passage / audio script body is never sent in full",
        "howToFetch": "read_passage with paragraph labels or question numbers",
    }));
    if draft
        .differences
        .iter()
        .any(|difference| difference_kind(difference) == DifferenceKind::Answer)
    {
        omitted.push(json!({
            "what": "the rest of the answer key",
            "howToFetch": "read_source with the answer page range",
        }));
    }
    omitted.truncate(MANIFEST_MAX_OMISSIONS);

    json!({
        "included": included,
        "omitted": omitted,
        "tools": {
            "read_source": "page range or quote is REQUIRED; at most 3 pages per call",
            "search_source": "find the page and line ids of a sentence you remember",
            "read_page_region": "one page image, optionally cropped to a bbox",
            "read_passage": "passage paragraphs by label or by question number",
            "read_candidate": "cloud candidate slices by taskIds or questionNumbers",
            "read_draft": "the current draft, limited to this packet unless you ask for more",
            "report_insufficient_context": "when the evidence you need is NOT in this packet, ask for it instead of guessing",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(page: u32, index: usize, text: &str) -> SourceLine {
        SourceLine {
            id: format!("p{page}:l{index}"),
            text: text.to_string(),
        }
    }

    fn index_with_pages(pages: &[(u32, &[&str])]) -> SourcePageIndex {
        let mut source = SourcePageIndex {
            source_file_id: "src-1".to_string(),
            kind: "pdf".to_string(),
            answer_pages_known: false,
            ..Default::default()
        };
        for (page, lines) in pages {
            source.lines.insert(
                *page,
                lines
                    .iter()
                    .enumerate()
                    .map(|(position, text)| line(*page, position + 1, text))
                    .collect(),
            );
            source.page_images.insert(
                *page,
                PageImageRef {
                    path: format!("/tmp/page-{page}.png"),
                    mime_type: "image/png".to_string(),
                    width: 595.0,
                    height: 842.0,
                },
            );
        }
        source
    }

    fn node(id: &str, text: &str, page_zero_based: i64) -> Value {
        json!({
            "type": "paragraph",
            "id": id,
            "sourceAnchors": [{
                "sourceFileId": "src-1",
                "pageIndex": page_zero_based,
                "nodeIds": [id],
                "extractionMode": "pdf_native",
                "sourceHash": "a".repeat(64)
            }],
            "children": [{"type": "text", "id": format!("{id}-t"), "text": text, "sourceAnchors": []}],
        })
    }

    /// 带可选 bbox 的锚点节点（`bbox` 是 PDF 点坐标，`origin: top-left`）。
    fn anchored_node(id: &str, page_zero_based: i64, bbox: Option<(f64, f64)>) -> Value {
        let mut anchor = json!({
            "sourceFileId": "src-1",
            "pageIndex": page_zero_based,
            "nodeIds": [id],
            "extractionMode": "pdf_native",
            "sourceHash": "a".repeat(64),
        });
        if let Some((y, height)) = bbox {
            anchor["bbox"] = json!({
                "x": 0.0,
                "y": y,
                "width": 100.0,
                "height": height,
                "origin": "top-left",
            });
        }
        json!({"type": "paragraph", "id": id, "sourceAnchors": [anchor], "children": []})
    }

    /// 一个三题组的本地稿：1-5（true/false）、6-7（completion）、8-10（matching）。
    fn canonical_paper() -> Value {
        let group = |task_id: &str, kind: &str, range: (u64, u64), page: i64, slots: &[&str]| {
            let response_id = format!("{task_id}-rg");
            json!({
                "taskId": task_id,
                "displayRange": {"kind": "range", "start": range.0, "end": range.1},
                "taskType": kind,
                "instructions": [node(&format!("{task_id}-ins"), "instructions", page)],
                "stimulus": [],
                "optionBank": Value::Null,
                "responseGroups": [{
                    "responseGroupId": response_id,
                    "kind": "text_entry",
                    "prompt": [node(&format!("{task_id}-prompt"), "prompt text", page)],
                    "slotIds": slots,
                }],
                "sourceAnchors": [],
            })
        };
        json!({
            "taskGroups": [
                group("tg-1-5", "true_false_not_given", (1, 5), 0, &["q1", "q2", "q3", "q4", "q5"]),
                group("tg-6-7", "sentence_completion", (6, 7), 0, &["q6", "q7"]),
                group("tg-8-10", "matching_information", (8, 10), 1, &["q8", "q9", "q10"]),
            ],
            "answerSlots": {
                "q1": {"slotId": "q1", "questionNumber": 1, "displayLabel": "1", "hostType": "prompt", "interaction": "text", "participation": "scoring", "confidence": 0.9},
                "q6": {"slotId": "q6", "questionNumber": 6, "displayLabel": "6", "hostType": "prompt", "interaction": "text", "participation": "scoring", "confidence": 0.9},
                "q8": {"slotId": "q8", "questionNumber": 8, "displayLabel": "8", "hostType": "prompt", "interaction": "text", "participation": "scoring", "confidence": 0.9},
            },
            "answerKey": {
                "q1": {"kind": "text", "values": ["TRUE"]},
                "q6": {"kind": "text", "values": ["example"]},
                "q8": {"kind": "text", "values": ["B"]},
            },
        })
    }

    fn difference(target_type: &str, target_id: &str, field: &str, canonical: Value, candidate: Value) -> Value {
        json!({
            "targetType": target_type,
            "targetId": target_id,
            "field": field,
            "canonical": canonical,
            "candidate": candidate,
            "contextDigest": "digest",
        })
    }

    fn plan(canonical: &Value, candidate: &Value, differences: &[Value], source: &SourcePageIndex) -> Vec<Value> {
        plan_packets(&PacketPlanInput {
            canonical,
            candidate,
            differences,
            blocking_issues: &[],
            protected: &BTreeSet::new(),
            source,
            edit_version: 7,
        })
    }

    /// 每条差异归到它所属的题组；不同题组各自成包。
    #[test]
    fn differences_are_owned_by_their_task_group() {
        let canonical = canonical_paper();
        let source = index_with_pages(&[(1, &["1 TRUE", "2 FALSE"]), (2, &["8 B"])]);
        let differences = vec![
            difference("slot", "q1", "answer", json!("TRUE"), json!("FALSE")),
            difference("slot", "q8", "answer", json!("B"), json!("C")),
        ];
        let packets = plan(&canonical, &Value::Null, &differences, &source);
        assert_eq!(packets.len(), 2, "两个题组的差异必须各自成包：{packets:#?}");
        let ids: Vec<&str> = packets
            .iter()
            .filter_map(|packet| packet.get("packetId").and_then(Value::as_str))
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(|id| id.starts_with("pkt-")), "{ids:?}");
        assert_ne!(ids[0], ids[1], "不同内容必须有不同的包 id");
        for packet in &packets {
            let task_ids = packet["taskIds"].as_array().expect("taskIds");
            assert_eq!(task_ids.len(), 1, "每包只该带它自己的题组：{packet:#?}");
        }
    }

    /// 包 id 必须由**身份**派生，而不是序号：`apply_edits` 之后要重切受影响的包，
    /// 序号会随重排整体漂移，于是「哪些包已经做完」全部错位。
    #[test]
    fn packet_ids_are_derived_from_identity_and_stay_stable_across_replanning() {
        let canonical = canonical_paper();
        let source = index_with_pages(&[(1, &["1 TRUE", "2 FALSE"]), (2, &["8 B"])]);
        let differences = vec![
            difference("slot", "q1", "answer", json!("TRUE"), json!("FALSE")),
            difference("slot", "q8", "answer", json!("B"), json!("C")),
        ];
        let first = plan(&canonical, &Value::Null, &differences, &source);
        // 同样的输入再切一次：id 必须逐字相同（否则重切就等于换了一批包）。
        let again = plan(&canonical, &Value::Null, &differences, &source);
        let ids = |packets: &[Value]| -> Vec<String> {
            packets
                .iter()
                .filter_map(|packet| packet.get("packetId").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        };
        assert_eq!(ids(&first), ids(&again), "同样的输入必须切出同样的 id");

        // 只剩第二组的差异时，第二组的包 id 必须**没变**（第一组消失不影响它）。
        let only_second = vec![difference("slot", "q8", "answer", json!("B"), json!("C"))];
        let replanned = plan(&canonical, &Value::Null, &only_second, &source);
        assert_eq!(replanned.len(), 1);
        assert_eq!(ids(&replanned)[0], ids(&first)[1], "未受影响的包 id 不该漂移");
    }

    /// 规则 2 第三条：本地 1-5 / 6-7、云端 1-7 ⇒ 三个题组必须落进同一个包。
    ///
    /// 这一条如果拆开，模型会分别在两个包里看到「1-5 对不对」与「6-7 对不对」，
    /// 而真正的分歧是**边界**：云端认为 1-7 是一组。看不到完整边界就无法评价它。
    #[test]
    fn a_different_cloud_split_merges_the_overlapping_groups() {
        let canonical = canonical_paper();
        let source = index_with_pages(&[(1, &["1 TRUE"]), (2, &["8 B"])]);
        let candidate = json!({
            "taskGroups": [{
                "taskId": "cloud-tg-1-7",
                "displayRange": {"kind": "range", "start": 1, "end": 7},
                "taskType": "true_false_not_given",
                "responseGroups": [],
            }],
        });
        // 差异挂在**云端**的题组 id 上（本地没有这个 id）。
        let differences = vec![difference(
            "task_group",
            "cloud-tg-1-7",
            "task_group",
            Value::Null,
            json!({"taskId": "cloud-tg-1-7"}),
        )];
        let packets = plan(&canonical, &candidate, &differences, &source);
        assert_eq!(packets.len(), 1, "交叉切分必须合成一个包：{packets:#?}");
        let task_ids: Vec<String> = packets[0]["taskIds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        assert_eq!(
            task_ids,
            vec!["tg-1-5".to_string(), "tg-6-7".to_string()],
            "1-5 与 6-7 都要在包里，否则模型看不到边界分歧"
        );
    }

    /// 答案类差异的包必须包含答案页；定位不到时如实写 `answerPagesUnknown`。
    #[test]
    fn an_answer_difference_brings_the_answer_page_or_says_it_does_not_know() {
        let canonical = canonical_paper();
        let source = index_with_pages(&[
            (1, &["1 TRUE", "2 FALSE"]),
            (3, &["8 B", "9 C", "10 D"]),
        ]);
        let differences = vec![difference("slot", "q8", "answer", json!("B"), json!("C"))];

        // ① 答案页识别产物说第 3 页是答案页 ⇒ 包里必须有第 3 页，且不许写 unknown。
        let mut known = source.clone();
        known.answer_pages = vec![3];
        known.answer_pages_known = true;
        let packets = plan(&canonical, &Value::Null, &differences, &known);
        assert_eq!(packets.len(), 1);
        let pages: Vec<u64> = packets[0]["scope"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_u64)
            .collect();
        assert!(pages.contains(&3), "答案页必须在范围内：{pages:?}");
        assert_eq!(packets[0]["scope"]["answerPagesUnknown"], json!(false));

        // ② 既没有答案页产物、文本层里也找不到答案区 ⇒ 必须写 unknown，
        //    而不是静默给一个空数组让模型以为「这份卷子没有答案页」。
        let mut unknown = index_with_pages(&[
            (1, &["1 TRUE", "2 FALSE"]),
            (3, &["Choose the correct heading", "Another question stem"]),
        ]);
        unknown.answer_pages = Vec::new();
        unknown.answer_pages_known = false;
        let packets = plan(&canonical, &Value::Null, &differences, &unknown);
        assert_eq!(packets[0]["scope"]["answerPagesUnknown"], json!(true));
        assert_eq!(packets[0]["scope"]["answerPages"], json!([]));
    }

    /// 答案页识别产物缺失时，从文本层搜「题号 + 值」的行来定位答案区。
    #[test]
    fn answer_pages_fall_back_to_the_text_layer_search() {
        let canonical = canonical_paper();
        let source = index_with_pages(&[
            (1, &["1 TRUE", "2 FALSE"]),
            (3, &["Answers", "1 TRUE", "2 FALSE", "3 NOT GIVEN"]),
        ]);
        let differences = vec![difference("slot", "q1", "answer", json!("TRUE"), json!("FALSE"))];
        let packets = plan(&canonical, &Value::Null, &differences, &source);
        let pages: Vec<u64> = packets[0]["scope"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_u64)
            .collect();
        assert!(pages.contains(&3), "文本层里能找到答案区就必须带上：{pages:?}");
        assert_eq!(packets[0]["scope"]["answerPagesUnknown"], json!(false));
    }

    /// 页号起始：锚点是 0-based，包里必须报 1-based，且**不出现范围外的页**。
    #[test]
    fn anchor_pages_are_reported_one_based_and_stay_inside_the_scope() {
        let canonical = canonical_paper();
        let source = index_with_pages(&[(1, &["page one"]), (2, &["page two"]), (3, &["page three"])]);
        // tg-8-10 的锚点在 `pageIndex: 1`（0-based）⇒ 1-based 第 2 页。
        let differences = vec![difference("slot", "q8", "answer", json!("B"), json!("C"))];
        let packets = plan(&canonical, &Value::Null, &differences, &source);
        let packet = &packets[0];
        let pages: Vec<u64> = packet["scope"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_u64)
            .collect();
        assert_eq!(pages, vec![2], "锚点 pageIndex=1 必须换算成 1-based 的第 2 页：{pages:?}");
        let reported: Vec<u64> = packet["sourceEvidence"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|page| page.get("pageIndex").and_then(Value::as_u64))
            .collect();
        assert_eq!(reported, vec![2], "证据页必须与 scope 一致");
        for line in packet["sourceEvidence"]["pages"][0]["lines"].as_array().unwrap() {
            let id = line["id"].as_str().unwrap();
            assert!(id.starts_with("p2:"), "行 id 必须带正确的页号，实际 {id}");
        }
    }

    /// 锚点贴**最后一页**的页边时，不能扩出不存在的页（审计发现 A-6）。
    ///
    /// 往回扩那一侧有 `page > 1` 保护，往外扩那一侧原来没有上界：锚点压到最后一页的
    /// 页底，`scope.pages` 就会多出一个「最后一页 + 1」。那一页在文本层和页图里都不存在，
    /// 模型照 `scopeManifest` 去要只会拿到「不存在」，白白耗掉一轮。
    #[test]
    fn an_edge_anchor_on_the_last_page_does_not_invent_a_page() {
        let source = index_with_pages(&[(1, &["one"]), (2, &["two"]), (3, &["three"])]);
        let last_page_edge = vec![AnchorPage {
            page: 3,
            edges: Some((800.0, 842.0)),
        }];
        let pages = expand_edge_pages(&[3], &last_page_edge, &source);
        assert_eq!(
            pages,
            vec![2, 3],
            "最后一页贴边只该往回扩，不该扩出第 4 页：{pages:?}"
        );

        // 中间页贴边仍要左右各扩一页：上界不能把正常行为也一起掐掉。
        let middle_page_edge = vec![AnchorPage {
            page: 2,
            edges: Some((0.0, 20.0)),
        }];
        let pages = expand_edge_pages(&[2], &middle_page_edge, &source);
        assert_eq!(pages, vec![1, 2, 3], "中间页贴边必须左右都扩：{pages:?}");
    }

    /// 一个题组里「有的页有 bbox、有的页没有」时，缺 bbox 的**那一页**必须退成整页图
    /// （审计发现 A-7）。
    ///
    /// 判据原来是整组级的（`bboxes.is_empty()`）：组里只要有一页有 bbox，另一页缺 bbox
    /// 就既拿不到区域图、也拿不到整页图 —— 而 `scope.pages` 里明明写着这一页。
    #[test]
    fn a_page_without_a_bbox_falls_back_to_a_whole_page_inside_a_group_that_has_bboxes() {
        let canonical = json!({
            "taskGroups": [{
                "taskId": "tg-mixed",
                "displayRange": {"kind": "range", "start": 1, "end": 3},
                "taskType": "matching_information",
                "instructions": [
                    anchored_node("tg-mixed-1", 0, Some((10.0, 20.0))),
                    anchored_node("tg-mixed-2", 1, None),
                    anchored_node("tg-mixed-3", 2, Some((10.0, 20.0))),
                ],
                "stimulus": [],
                "optionBank": Value::Null,
                "responseGroups": [],
                "sourceAnchors": [],
            }],
            "answerSlots": {},
            "answerKey": {},
        });
        let source = index_with_pages(&[(1, &["one"]), (2, &["two"]), (3, &["three"])]);
        let index = GroupIndex::build(&canonical);
        let draft = PacketDraft {
            task_ids: BTreeSet::from(["tg-mixed".to_string()]),
            part_id: None,
            differences: Vec::new(),
            blocking_issues: Vec::new(),
            document_only: false,
        };
        let input = PacketPlanInput {
            canonical: &canonical,
            candidate: &Value::Null,
            differences: &[],
            blocking_issues: &[],
            protected: &BTreeSet::new(),
            source: &source,
            edit_version: 1,
        };
        let requests = region_requests(&draft, &input, &index);
        let mut pages: Vec<(u64, bool)> = requests
            .iter()
            .map(|request| {
                (
                    request["pageIndex"].as_u64().unwrap(),
                    !request["bbox"].is_null(),
                )
            })
            .collect();
        pages.sort();
        assert_eq!(
            pages,
            vec![(1, true), (2, false), (3, true)],
            "缺 bbox 的那一页必须退成整页图，不能整组一起漏掉：{requests:#?}"
        );
    }

    /// 同一个包里的两个题组：一个有 bbox、一个没有 ⇒ 缺 bbox 的那一组**仍要**退整页图。
    ///
    /// A-7 的修法引入了 `pages_requested`，但把它声明在 `for task_id` **外面**，判据于是
    /// 变成**草稿级**的：包内任一题组在某一页有 bbox，其他题组在这一页的「缺 bbox 退整页图」
    /// 就被吞掉。而这一页恰恰是后一组要核的内容 —— 前一组裁出来的那条区域未必覆盖它。
    /// 判据必须是**组内**的（`covered`），只有去重（`seen`）才跨组。
    ///
    /// 题组顺序还会让结果摇摆：`draft.task_ids` 是 `BTreeSet`，谁先被遍历取决于 id 字典序，
    /// 于是同一个包可能给 1 张图、也可能给 2 张。这条用例把两种顺序都固定成 2 张。
    #[test]
    fn a_group_without_a_bbox_still_gets_a_whole_page_when_another_group_has_one() {
        let shared_bank = json!({"optionBankId": "bank-shared", "options": []});
        let group = |task_id: &str, range: (u64, u64), node: Value, slot: &str| {
            json!({
                "taskId": task_id,
                "displayRange": {"kind": "range", "start": range.0, "end": range.1},
                "taskType": "matching_information",
                "instructions": [node],
                "stimulus": [],
                "optionBank": shared_bank.clone(),
                "responseGroups": [{
                    "responseGroupId": format!("{task_id}-rg"),
                    "kind": "text_entry",
                    "prompt": [],
                    "slotIds": [slot],
                }],
                "sourceAnchors": [],
            })
        };
        let canonical = json!({
            "taskGroups": [
                group("tg-a", (1, 2), anchored_node("tg-a-1", 0, Some((10.0, 20.0))), "q1"),
                group("tg-b", (3, 4), anchored_node("tg-b-1", 0, None), "q3"),
            ],
            "answerSlots": {
                "q1": {"slotId": "q1", "questionNumber": 1, "displayLabel": "1", "hostType": "prompt", "interaction": "text", "participation": "scoring", "confidence": 0.9},
                "q3": {"slotId": "q3", "questionNumber": 3, "displayLabel": "3", "hostType": "prompt", "interaction": "text", "participation": "scoring", "confidence": 0.9},
            },
            "answerKey": {
                "q1": {"kind": "text", "values": ["TRUE"]},
                "q3": {"kind": "text", "values": ["TRUE"]},
            },
        });
        let source = index_with_pages(&[(1, &["one"]), (2, &["two"])]);
        let index = GroupIndex::build(&canonical);

        // 直测 `region_requests`：两种题组顺序都必须给出「区域图 + 整页图」两张。
        for order in [
            ["tg-a", "tg-b"],
            ["tg-b", "tg-a"],
        ] {
            let draft = PacketDraft {
                task_ids: order.iter().map(|id| id.to_string()).collect(),
                part_id: None,
                differences: Vec::new(),
                blocking_issues: Vec::new(),
                document_only: false,
            };
            let input = PacketPlanInput {
                canonical: &canonical,
                candidate: &Value::Null,
                differences: &[],
                blocking_issues: &[],
                protected: &BTreeSet::new(),
                source: &source,
                edit_version: 1,
            };
            let requests = region_requests(&draft, &input, &index);
            let mut pages: Vec<(u64, bool)> = requests
                .iter()
                .map(|request| {
                    (
                        request["pageIndex"].as_u64().unwrap(),
                        !request["bbox"].is_null(),
                    )
                })
                .collect();
            // 同页先排「有 bbox」再排「整页」，免得元组序把 `false` 排到前面。
            pages.sort_by(|left, right| left.0.cmp(&right.0).then(right.1.cmp(&left.1)));
            assert_eq!(
                pages,
                vec![(1, true), (1, false)],
                "顺序 {order:?}：有 bbox 的题组给区域图，没有的必须退整页图：{requests:#?}"
            );
        }

        // 走真实切包路径，确认这两个题组确实会被并进**同一个**包（共用选项库 ⇒ 规则 2），
        // 否则上面那条对草稿的手工构造就站不住。
        let differences = vec![
            difference("slot", "q1", "answer", json!("TRUE"), json!("FALSE")),
            difference("slot", "q3", "answer", json!("TRUE"), json!("FALSE")),
        ];
        let packets = plan(&canonical, &Value::Null, &differences, &source);
        assert_eq!(packets.len(), 1, "共用选项库的题组必须并成一个包：{packets:#?}");
        let task_ids: Vec<&str> = packets[0]["taskIds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(task_ids, vec!["tg-a", "tg-b"], "并包结果：{packets:#?}");
        let regions = packets[0]["sourceEvidence"]["regions"].as_array().unwrap();
        let mut planned: Vec<(u64, bool)> = regions
            .iter()
            .map(|region| {
                (
                    region["pageIndex"].as_u64().unwrap(),
                    !region["bbox"].is_null(),
                )
            })
            .collect();
        planned.sort_by(|left, right| left.0.cmp(&right.0).then(right.1.cmp(&left.1)));
        assert_eq!(
            planned,
            vec![(1, true), (1, false)],
            "并包后仍要给缺 bbox 的题组一张整页图：{regions:#?}"
        );
    }

    /// `paperMap` 必须告诉模型每一页有哪些段落标号（审计发现 A-8）。
    ///
    /// 判据要与 `read_passage` 的标签匹配一致：`paperMap` 说「这一页有 C」，
    /// 用 `read_passage {"paragraphLabels":["C"]}` 就必须真的取得到。两边判据不同
    /// 才是危险的事——`paperMap` 说有、工具说没有，模型只会乱猜。
    #[test]
    fn the_paper_map_names_the_paragraph_labels_on_each_page() {
        let source = index_with_pages(&[
            (
                1,
                &["The passage begins here.", "A First idea.", "B Second idea."],
            ),
            (2, &["Questions 14-15", "C Third idea."]),
        ]);
        let canonical = canonical_paper();
        let index = GroupIndex::build(&canonical);
        let map = paper_map(&canonical, &source, &index);
        let labels_of = |page: u64| -> Vec<String> {
            map["pages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["page"] == json!(page))
                .and_then(|entry| entry["paragraphLabels"].as_array())
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        assert_eq!(
            labels_of(1),
            vec!["A".to_string(), "B".to_string()],
            "第 1 页有 A、B 两段：{map:#?}"
        );
        assert_eq!(labels_of(2), vec!["C".to_string()], "第 2 页只有 C 段：{map:#?}");

        // 与工具判据一致：`paperMap` **报出来的每一个标号**都必须真的能被 `read_passage`
        // 取到。刻意写成「遍历报出来的全部标号」而不是写死一个 "C"：写死就只能证明
        // 「C 这一条对」，而契约是「报出来的都对」——`paperMap` 说有、工具说没有，
        // 模型只会乱猜，那正是这条判据要挡的事。
        let mut checked = 0usize;
        for page in [1u64, 2] {
            for label in labels_of(page) {
                let mut budget = crate::cloud_repair::grab::GrabBudget::new();
                let found = crate::cloud_repair::grab::read_passage(
                    &source,
                    &json!({"paragraphLabels": [label]}),
                    &mut budget,
                )
                .unwrap_or_else(|error| {
                    panic!("paperMap 报了第 {page} 页的 {label}，工具却拒绝：{error}")
                });
                let paragraphs = found["paragraphs"].as_array().cloned().unwrap_or_default();
                assert!(
                    !paragraphs.is_empty(),
                    "paperMap 报了第 {page} 页的 {label}，工具却取不到任何段落：{found:#?}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 3, "第 1 页 A/B、第 2 页 C，共 3 个标号都要被核对过");
    }

    /// 阻断问题排在答案差异之前，答案差异排在文本差异之前。
    #[test]
    fn packets_are_ordered_blocking_then_answers_then_text() {
        let canonical = canonical_paper();
        let source = index_with_pages(&[(1, &["1 TRUE", "2 FALSE"]), (2, &["8 B"])]);
        let differences = vec![
            difference("task_group", "tg-8-10", "instructions", json!("a"), json!("b")),
            difference("slot", "q1", "answer", json!("TRUE"), json!("FALSE")),
        ];
        // 阻断问题落在**第三个**题组上：三种类别各占一个包，顺序才看得出来。
        let issues = vec![json!({
            "code": "ANSWER_KEY_MISSING_SLOT",
            "severity": "blocking",
            "message": "q6 has no answer",
            "targetType": "slot",
            "targetId": "q6",
            "targetIds": ["q6"],
        })];
        let packets = plan_packets(&PacketPlanInput {
            canonical: &canonical,
            candidate: &Value::Null,
            differences: &differences,
            blocking_issues: &issues,
            protected: &BTreeSet::new(),
            source: &source,
            edit_version: 7,
        });
        assert_eq!(packets.len(), 3, "三个题组各自成包：{packets:#?}");
        assert_eq!(packets[0]["taskIds"], json!(["tg-6-7"]), "阻断包排第一");
        assert_eq!(packets[0]["blockingIssues"].as_array().unwrap().len(), 1);
        assert_eq!(packets[1]["taskIds"], json!(["tg-1-5"]), "答案差异排第二");
        assert_eq!(packets[1]["differences"][0]["field"], json!("answer"));
        assert_eq!(packets[2]["taskIds"], json!(["tg-8-10"]), "文本差异排最后");
        assert_eq!(packets[2]["differences"][0]["field"], json!("instructions"));
    }

    /// 文档级问题（没有可归属的目标）单独成包，且只带索引。
    #[test]
    fn a_document_level_issue_becomes_an_index_only_packet() {
        let canonical = canonical_paper();
        let source = index_with_pages(&[(1, &["1 TRUE"])]);
        let issues = vec![json!({
            "code": "SIGNIFICANT_REGION_UNASSIGNED",
            "severity": "blocking",
            "message": "a region on page 1 is not covered",
            "targetIds": [],
        })];
        let packets = plan_packets(&PacketPlanInput {
            canonical: &canonical,
            candidate: &Value::Null,
            differences: &[],
            blocking_issues: &issues,
            protected: &BTreeSet::new(),
            source: &source,
            edit_version: 7,
        });
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0]["documentOnly"], json!(true));
        assert_eq!(packets[0]["taskIds"], json!([]));
        assert!(packets[0]["paperMap"]["taskGroups"].as_array().unwrap().len() == 3);
        assert_eq!(
            packets[0]["draftSlice"]["taskGroups"],
            json!([]),
            "文档包不带稿件内容，只带索引"
        );
    }

    /// 共用选项库的两个题组必须合成一个包。
    #[test]
    fn a_shared_option_bank_merges_the_groups() {
        let mut canonical = canonical_paper();
        let bank = json!({
            "optionBankId": "bank-shared",
            "scope": "task_group",
            "options": [{"optionId": "opt-a", "label": "A", "content": []}],
            "allowReuse": false,
        });
        let groups = canonical["taskGroups"].as_array_mut().unwrap();
        for group in groups.iter_mut() {
            if group["taskId"] == json!("tg-1-5") || group["taskId"] == json!("tg-8-10") {
                group["optionBank"] = bank.clone();
            }
        }
        let source = index_with_pages(&[(1, &["1 TRUE"]), (2, &["8 B"])]);
        let differences = vec![
            difference("slot", "q1", "answer", json!("TRUE"), json!("FALSE")),
            difference("slot", "q8", "answer", json!("B"), json!("C")),
        ];
        let packets = plan(&canonical, &Value::Null, &differences, &source);
        assert_eq!(packets.len(), 1, "共用选项库必须合并：{packets:#?}");
        assert_eq!(packets[0]["taskIds"].as_array().unwrap().len(), 2);
    }

    /// 同一段 stimulus 被切成两组时必须合在一起。
    #[test]
    fn a_shared_stimulus_merges_the_groups() {
        let mut canonical = canonical_paper();
        let stimulus = json!([node("shared-stim", "the same notes block", 0)]);
        let groups = canonical["taskGroups"].as_array_mut().unwrap();
        for group in groups.iter_mut() {
            if group["taskId"] == json!("tg-1-5") || group["taskId"] == json!("tg-6-7") {
                group["stimulus"] = stimulus.clone();
            }
        }
        let source = index_with_pages(&[(1, &["1 TRUE"])]);
        let differences = vec![difference("slot", "q1", "answer", json!("TRUE"), json!("FALSE"))];
        let packets = plan(&canonical, &Value::Null, &differences, &source);
        assert_eq!(packets.len(), 1, "同一段 stimulus 必须合并：{packets:#?}");
        assert_eq!(packets[0]["taskIds"].as_array().unwrap().len(), 2);
    }

    /// `scopeManifest` 必须说清省略了什么、怎么取回来。
    #[test]
    fn the_scope_manifest_names_what_is_omitted_and_how_to_fetch_it() {
        let canonical = canonical_paper();
        let source = index_with_pages(&[(1, &["1 TRUE"]), (2, &["8 B"]), (3, &["9 C"])]);
        let differences = vec![difference("slot", "q1", "answer", json!("TRUE"), json!("FALSE"))];
        let packets = plan(&canonical, &Value::Null, &differences, &source);
        let manifest = &packets[0]["scopeManifest"];
        let omitted = manifest["omitted"].as_array().expect("omitted 必须是数组");
        assert!(!omitted.is_empty(), "必须显式列出省略项");
        for entry in omitted {
            assert!(
                entry.get("what").and_then(Value::as_str).is_some()
                    && entry.get("howToFetch").and_then(Value::as_str).is_some(),
                "每条省略项都要说清「省了什么」与「怎么取」：{entry}"
            );
        }
        assert!(
            manifest["tools"]["report_insufficient_context"].is_string(),
            "工具表里必须有 report_insufficient_context"
        );
        assert!(packets[0]["estimatedInputTokens"].as_u64().unwrap() > 0);
    }
}
