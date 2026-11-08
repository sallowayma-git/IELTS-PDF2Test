//! 导入前把「一份 PDF 里的一整套卷子」拆成多篇阅读条目，并把答案页按篇分开。
//!
//! 产品语义：阅读题库的一条 = 一篇 Passage（P1/P2/P3）。用户常拿到的是整套卷子
//! （一份 PDF 里 Passage 1–3，甚至 Test 1–4 × 3 篇，末尾一份总答案）。以前整份文件只建
//! 一条，识别链只认第一篇。现在导入时先切页，每篇建一条独立条目，用户在**题库主界面**
//! 按行切换篇章。
//!
//! ## 分篇：多信号互证，不靠单一标题
//!
//! 每页先抽取互相独立的信号：
//!   - `heading`：整行是 `READING PASSAGE n` / `PASSAGE n`（兼容字母间隔的文本层）；
//!   - `instruction`：`You should spend about 20 minutes on Questions a–b, which are based
//!     on Reading Passage n`——给出篇序号和题号范围，单独就是很强的起点证据；
//!   - `layout`：版式交替。整套卷子里每篇都是「正文页 → 题目页」（或统一的「题目页 →
//!     正文页」），从第二种页回到第一种页就是一个候选分界；
//!   - `numbering`：题号连续接力（上一篇到 13，这一篇从 14 开始）或从 1 重新开始（新一套）；
//!   - `band`：题号落在该篇的常规区间（1–13 / 14–26 / 27–40）；
//!   - `test`：页眉 `Test n` 变化。
//!
//! 起点只在出现 `heading` / `instruction` / `layout` / `test` 时才会开新篇；之后用
//! `numbering`、`band` 回头核对。只有版式一种信号、题号又对不上的分界会被撤回（并回
//! 上一篇）。每篇带 `confidence`：显式信号 + 至少一种佐证 = high，仅显式或仅
//! 版式+题号+区间三者一致 = medium，其余 low。任何一篇是 low、或篇序号与题号区间矛盾时，
//! `default_split = false`——界面照样把计划摆出来，但默认不拆，由用户决定。
//!
//! ## 答案：按篇分开
//!
//! 答案页（`ANSWER KEY` 标题 + 编号短行，或整页几乎都是「题号 答案」短行）不再整页
//! 附给每一篇。逐行解析出 `(Test, 听/读区, 题号, 答案)`，按 Test 与该篇的题号范围分给
//! 各篇，写成每篇自己的答案文本（导入时作为该条目的 AnswerKey 源文件）。听力区的答案
//! 不会进阅读条目。某篇的题号覆盖不足 80% 时（扫描件、复杂表格），才退回把**该套**的
//! 答案页附到这一篇的子 PDF 末尾，并如实标出来。
//!
//! 听力不在这里拆：听力条目 = 一整套 Part 1–4，音频按 Part 绑定在同一条目上。

use std::path::Path;

use serde::Serialize;

use crate::CommandResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SplitAnswer {
    pub number: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PdfSplitSegment {
    /// 展示用标签，例如 `Test 2 · P3`；没有 Test 上下文时就是 `P3`。
    pub label: String,
    pub test: Option<u32>,
    pub passage: u32,
    /// 本篇自己的页（1-based，升序）。
    pub pages: Vec<u32>,
    /// 起始页上方还有上一篇的内容（起点不在页首），该页同时归入两篇。
    pub shares_start_page: bool,
    /// 本篇题号范围（来自题目标题行 / 说明行）。
    pub question_range: Option<(u32, u32)>,
    /// 支持这个分界的信号：heading / instruction / layout / numbering / band / test。
    pub signals: Vec<String>,
    /// high / medium / low。
    pub confidence: String,
    /// 从答案页分出来、只属于本篇的答案。
    pub answers: Vec<SplitAnswer>,
    /// 答案没能按题号分出来时，退回附到本篇末尾的答案页（只含本套的答案页）。
    pub fallback_answer_pages: Vec<u32>,
}

impl PdfSplitSegment {
    /// 答案文本形态（导入时作为 AnswerKey 源文件写出）。
    pub(crate) fn answer_text(&self) -> Option<String> {
        if self.answers.is_empty() {
            return None;
        }
        Some(
            self.answers
                .iter()
                .map(|answer| format!("{} {}", answer.number, answer.text))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PdfSplitPlan {
    pub page_count: u32,
    pub segments: Vec<PdfSplitSegment>,
    /// 识别为答案页的页（1-based，升序）。
    pub answer_pages: Vec<u32>,
    /// 没有归入任何篇章的页（封面、说明、听力部分等）。
    pub unassigned_pages: Vec<u32>,
    /// 信号是否足够一致，可以默认拆分。
    pub default_split: bool,
    /// 给用户看的核对提示（篇序号与题号矛盾、答案没分全等）。
    pub warnings: Vec<String>,
}

impl PdfSplitPlan {
    /// 至少两篇才值得拆。
    pub(crate) fn is_multi(&self) -> bool {
        self.segments.len() >= 2
    }

    /// 某一篇子 PDF 应包含的页：本篇页 + 退回附带的答案页（去重、升序）。
    pub(crate) fn pages_for(&self, segment: &PdfSplitSegment) -> Vec<u32> {
        let mut pages: Vec<u32> = segment
            .pages
            .iter()
            .chain(segment.fallback_answer_pages.iter())
            .copied()
            .collect();
        pages.sort_unstable();
        pages.dedup();
        pages
    }
}

// ───────────────────────── 文本工具 ─────────────────────────

fn squash(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 小写、统一破折号、空白折叠。
fn normalize(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '–' | '—' | '−' | '‑' => '-',
            ch => ch.to_ascii_lowercase(),
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn ordinal_word(rest: &str) -> Option<u32> {
    match rest {
        "1" | "one" => Some(1),
        "2" | "two" => Some(2),
        "3" | "three" => Some(3),
        _ => None,
    }
}

/// 整行 `READING PASSAGE n` 或 `PASSAGE n`（n = 1..3）。
fn passage_heading(line: &str) -> Option<u32> {
    let squashed = squash(line);
    squashed
        .strip_prefix("readingpassage")
        .or_else(|| squashed.strip_prefix("passage"))
        .and_then(ordinal_word)
}

/// 整行 `TEST n`（1..=99）。
fn test_heading(line: &str) -> Option<u32> {
    let squashed = squash(line);
    let rest = squashed.strip_prefix("test")?;
    if rest.is_empty() || rest.len() > 2 || !rest.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok().filter(|n| (1..=99).contains(n))
}

/// 整行答案页标题。
fn answer_heading(line: &str) -> bool {
    let squashed = squash(line);
    if squashed.len() > 40 {
        return false;
    }
    let body = squashed
        .trim_start_matches("listeningandreading")
        .trim_start_matches("readingandlistening")
        .trim_start_matches("reading")
        .trim_start_matches("listening");
    let tail = body
        .strip_prefix("answerkeys")
        .or_else(|| body.strip_prefix("answerkey"))
        .or_else(|| body.strip_prefix("answers"))
        .or_else(|| body.strip_prefix("keyanswers"));
    tail.is_some_and(|tail| {
        let tail = tail.strip_prefix("test").unwrap_or(tail);
        tail.chars().all(|ch| ch.is_ascii_digit())
    })
}

/// `a-b` / `a to b` / `a and b` / 单个 `a`，返回 (起, 止)。
fn parse_range(text: &str) -> Option<(u32, u32)> {
    let text = text.trim_start();
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let start: u32 = digits.parse().ok()?;
    let rest = text[digits.len()..].trim_start();
    let rest = rest
        .strip_prefix('-')
        .or_else(|| rest.strip_prefix("to"))
        .or_else(|| rest.strip_prefix("and"));
    let end = match rest {
        Some(rest) => {
            let rest = rest.trim_start();
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()?
        }
        None => start,
    };
    (1..=60).contains(&start).then_some(())?;
    (end >= start && end <= 60).then_some((start, end))
}

/// 题目标题行：`Questions 14-26` / `Question 5`（整行以它开头）。
fn question_heading_range(line: &str) -> Option<(u32, u32)> {
    let normalized = normalize(line);
    let rest = normalized
        .strip_prefix("questions")
        .or_else(|| normalized.strip_prefix("question"))?;
    if rest.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
        return None;
    }
    parse_range(rest)
}

/// 一行是「题号 + 短答案」吗（答案页的典型行；题干是长句，不算）。
fn numbered_short_line(line: &str) -> bool {
    let normalized = normalize(line);
    let digits: String = normalized.chars().take_while(char::is_ascii_digit).collect();
    let Ok(number) = digits.parse::<u32>() else { return false };
    if !(1..=40).contains(&number) {
        return false;
    }
    let rest = normalized[digits.len()..].trim_start_matches(['.', ')']).trim();
    let words = rest.split_whitespace().count();
    (1..=6).contains(&words)
}

fn is_prose_line(line: &str) -> bool {
    let words = line.split_whitespace().count();
    words >= 9 && !line.trim_start().starts_with(|ch: char| ch.is_ascii_digit())
}

fn reading_band(number: u32) -> Option<u32> {
    match number {
        1..=13 => Some(1),
        14..=26 => Some(2),
        27..=40 => Some(3),
        _ => None,
    }
}

fn range_band(range: Option<(u32, u32)>) -> Option<u32> {
    let (start, end) = range?;
    let (a, b) = (reading_band(start)?, reading_band(end)?);
    (a == b).then_some(a)
}

// ───────────────────────── 逐页信号 ─────────────────────────

#[derive(Debug, Default)]
struct PageFeatures {
    test: Option<u32>,
    /// (篇序号, 所在行)
    heading: Option<(u32, usize)>,
    /// (篇序号, 题号范围, 所在行)
    instruction: Option<(Option<u32>, Option<(u32, u32)>, usize)>,
    question_ranges: Vec<(u32, u32)>,
    prose_lines: usize,
    numbered_short: usize,
    non_empty: usize,
    answer_heading: bool,
}

impl PageFeatures {
    fn has_questions(&self) -> bool {
        !self.question_ranges.is_empty()
    }
    fn is_prose(&self) -> bool {
        self.prose_lines >= 6 && !self.has_questions()
    }
    /// 起点所在行（标题或说明行，取较早者）。
    fn start_line(&self) -> Option<usize> {
        match (self.heading.map(|h| h.1), self.instruction.map(|i| i.2)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

fn page_features(text: &str) -> PageFeatures {
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let mut features = PageFeatures { non_empty: lines.len(), ..Default::default() };
    for (index, line) in lines.iter().enumerate() {
        if features.test.is_none() {
            features.test = test_heading(line);
        }
        if features.heading.is_none() {
            // Some producers emit READING / PASSAGE / 2 as separate text runs.
            // Only join unfinished heading fragments, never prose references.
            let mut heading = line.to_string();
            for next in lines.iter().skip(index + 1).take(2) {
                if !matches!(squash(&heading).as_str(), "reading" | "readingpassage" | "passage") {
                    break;
                }
                heading.push(' ');
                heading.push_str(next);
            }
            if let Some(passage) = passage_heading(&heading) {
                features.heading = Some((passage, index));
            }
        }
        if let Some(range) = question_heading_range(line) {
            features.question_ranges.push(range);
        }
        if answer_heading(line) {
            features.answer_heading = true;
        }
        if numbered_short_line(line) {
            features.numbered_short += 1;
        }
        if is_prose_line(line) {
            features.prose_lines += 1;
        }
        // A long instruction may span 3-6 text runs, and PDF fonts may omit
        // spaces entirely. Preserve punctuation for the question range.
        if features.instruction.is_none() {
            let window: String = normalize(&lines[index..lines.len().min(index + 6)].join(" "))
                .chars().filter(|ch| !ch.is_whitespace()).collect();
            if let Some(at) = window.find("spendabout20minutes").or_else(|| window.find("spend20minutes")) {
                let tail = &window[at..];
                let range = tail
                    .find("questions")
                    .and_then(|pos| parse_range(&tail[pos + "questions".len()..]));
                let passage = tail.find("readingpassage").and_then(|pos| {
                    let rest = &tail[pos + "readingpassage".len()..];
                    // "1onpages2and3" also occurs with missing inter-word spaces.
                    let number: String = rest.chars().take_while(char::is_ascii_digit).collect();
                    ordinal_word(&number).or_else(|| ["one", "two", "three"].iter()
                        .find(|word| rest.starts_with(**word)).and_then(|word| ordinal_word(word)))
                });
                features.instruction = Some((passage, range, index));
            }
        }
    }
    features
}

fn is_answer_page(features: &PageFeatures, previous_was_answer: bool) -> bool {
    if features.heading.is_some() || features.instruction.is_some() || features.has_questions() {
        return false;
    }
    let dense = features.numbered_short * 2 >= features.non_empty.max(1);
    (features.answer_heading && features.numbered_short >= 3)
        || (features.numbered_short >= 8 && dense && features.prose_lines <= 2)
        || (previous_was_answer && features.numbered_short >= 3 && dense)
}

// ───────────────────────── 分篇 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageKind {
    Prose,
    Questions,
}

struct Draft {
    test: Option<u32>,
    explicit_passage: Option<u32>,
    pages: Vec<u32>,
    shares_start_page: bool,
    signals: Vec<&'static str>,
    ranges: Vec<(u32, u32)>,
    seen_second_kind: bool,
}

impl Draft {
    fn range(&self) -> Option<(u32, u32)> {
        let start = self.ranges.iter().map(|r| r.0).min()?;
        let end = self.ranges.iter().map(|r| r.1).max()?;
        Some((start, end))
    }
    /// 题号整段落在同一个常规区间时的篇序号。
    fn band(&self) -> Option<u32> {
        range_band(self.range())
    }
    fn signal(&mut self, name: &'static str) {
        if !self.signals.contains(&name) {
            self.signals.push(name);
        }
    }
}

/// 标题在页首附近：此前的非空行不超过这么多（页眉、页码、Test n）。
const HEADING_TOP_LINES: usize = 4;

/// 按逐页文本生成切分计划。`pages[i]` 是第 i+1 页的文本。
pub(crate) fn plan_from_page_texts(pages: &[String]) -> PdfSplitPlan {
    let page_count = pages.len() as u32;
    let features: Vec<PageFeatures> = pages.iter().map(|text| page_features(text)).collect();

    // 1. 答案页。
    let mut answer_flags = vec![false; features.len()];
    for index in 0..features.len() {
        let previous = index > 0 && answer_flags[index - 1];
        answer_flags[index] = is_answer_page(&features[index], previous);
    }

    // 2. 版式基准：第一页有内容的是正文还是题目。
    let first_kind = features
        .iter()
        .zip(&answer_flags)
        .filter(|(_, answer)| !**answer)
        .find_map(|(f, _)| {
            if f.heading.is_some() || f.instruction.is_some() || f.is_prose() {
                Some(PageKind::Prose)
            } else if f.has_questions() {
                Some(PageKind::Questions)
            } else {
                None
            }
        })
        .unwrap_or(PageKind::Prose);

    // 3. 逐页开篇。
    let mut drafts: Vec<Draft> = Vec::new();
    let mut unassigned_pages = Vec::new();
    let mut current_test: Option<u32> = None;
    let mut after_answers = false;
    for (index, f) in features.iter().enumerate() {
        let page = index as u32 + 1;
        if answer_flags[index] {
            after_answers = true;
            continue;
        }
        let test_changed = f.test.is_some() && f.test != current_test && current_test.is_some();
        if f.test.is_some() {
            current_test = f.test;
        }
        let explicit = f
            .heading
            .map(|h| h.0)
            .or_else(|| f.instruction.and_then(|i| i.0));
        let kind = if f.heading.is_some() || f.instruction.is_some() || f.is_prose() {
            Some(PageKind::Prose)
        } else if f.has_questions() {
            Some(PageKind::Questions)
        } else {
            None
        };

        let new_explicit = (f.heading.is_some() || f.instruction.is_some())
            && drafts.last().map_or(true, |last| {
                after_answers
                    || test_changed
                    || explicit.is_some_and(|n| last.explicit_passage != Some(n))
                    || (explicit.is_none() && last.seen_second_kind)
            });
        let new_layout = kind == Some(first_kind)
            && drafts.last().is_some_and(|last| last.seen_second_kind);
        // A full declared question band is an independent boundary even when
        // prose/heading extraction failed. Partial task groups never trigger it.
        let new_question_band = f.heading.is_none() && f.instruction.is_none()
            && f.question_ranges.first().is_some_and(|range| matches!(range, (14, 26) | (27, 40))
                && drafts.last().is_some_and(|last| last.range().is_some_and(|prev| prev.1 + 1 == range.0)));
        let starts = drafts.is_empty() && kind.is_some()
            || new_explicit
            || new_layout
            || new_question_band
            || (test_changed && kind.is_some());

        if starts {
            let start_line = f.start_line().unwrap_or(0);
            let shares_start_page = start_line > HEADING_TOP_LINES
                && drafts.last().is_some_and(|last| last.pages.last() == Some(&(page - 1)));
            if shares_start_page {
                if let Some(last) = drafts.last_mut() {
                    last.pages.push(page);
                }
            }
            let mut draft = Draft {
                test: current_test,
                explicit_passage: explicit,
                pages: vec![page],
                shares_start_page,
                signals: Vec::new(),
                ranges: Vec::new(),
                seen_second_kind: false,
            };
            if f.heading.is_some() {
                draft.signal("heading");
            }
            if f.instruction.is_some() {
                draft.signal("instruction");
            }
            if new_layout {
                draft.signal("layout");
            }
            if test_changed {
                draft.signal("test");
            }
            if new_question_band {
                draft.signal("question_band");
            }
            drafts.push(draft);
            after_answers = false;
        } else if let Some(last) = drafts.last_mut() {
            last.pages.push(page);
        } else {
            unassigned_pages.push(page);
            continue;
        }
        let draft = drafts.last_mut().expect("draft");
        if draft.explicit_passage.is_none() {
            draft.explicit_passage = explicit;
        }
        draft.ranges.extend(f.question_ranges.iter().copied());
        if let Some((_, Some(range), _)) = f.instruction {
            draft.ranges.push(range);
        }
        if kind.is_some_and(|k| k != first_kind) {
            draft.seen_second_kind = true;
        }
    }

    // 4. 只有版式信号的分界必须由题号证实：题号接力**并且**换到了另一篇的区间
    //    （或从 1 重新开始＝新一套）。同一篇的「题后还有正文续页」到这里会被撤回。
    let mut merged: Vec<Draft> = Vec::new();
    for draft in drafts {
        let layout_only = draft.signals.iter().all(|s| *s == "layout");
        let confirmed = match (merged.last(), draft.range()) {
            (Some(prev), Some(cur)) => {
                let restart = cur.0 == 1 && prev.range().is_some_and(|r| r.1 > 1);
                let relay = prev.range().is_some_and(|r| cur.0 == r.1 + 1);
                let band_changed = match (prev.band(), draft.band()) {
                    (Some(a), Some(b)) => a != b,
                    (None, Some(_)) => true,
                    _ => false,
                };
                restart || (relay && band_changed)
            }
            (None, _) => true,
            _ => false,
        };
        if layout_only && !confirmed && !merged.is_empty() {
            let last = merged.last_mut().expect("merged");
            last.pages.extend(draft.pages);
            last.pages.dedup();
            last.ranges.extend(draft.ranges);
            continue;
        }
        merged.push(draft);
    }

    // 5. 篇序号、佐证信号与置信度。
    let mut warnings = Vec::new();
    let mut segments: Vec<PdfSplitSegment> = Vec::new();
    let mut ordinal_in_test = 0u32;
    let mut previous: Option<(Option<u32>, Option<(u32, u32)>)> = None;
    // 版式交替本身就是证据：同一套里后面的篇是靠版式切出来的，这一套的第一篇也算有版式信号。
    let layout_tests: Vec<Option<u32>> = merged
        .iter()
        .filter(|d| d.signals.contains(&"layout"))
        .map(|d| d.test)
        .collect();
    let mut prior_test: Option<Option<u32>> = None;
    for draft in merged.iter_mut() {
        let first_of_test = prior_test != Some(draft.test);
        prior_test = Some(draft.test);
        if first_of_test && layout_tests.contains(&draft.test) {
            draft.signal("layout");
        }
    }
    for draft in &merged {
        let same_test = previous.is_some_and(|(test, _)| test == draft.test);
        ordinal_in_test = if same_test { ordinal_in_test + 1 } else { 1 };
        let range = draft.range();
        let band = draft.band();
        let passage = draft.explicit_passage.or(band).unwrap_or(ordinal_in_test.min(3));
        let mut signals: Vec<String> = draft.signals.iter().map(|s| s.to_string()).collect();
        let numbering = match (previous.and_then(|p| p.1), range) {
            (Some(prev), Some(cur)) => cur.0 == prev.1 + 1 || cur.0 == 1,
            (None, Some(cur)) => cur.0 == 1,
            _ => false,
        };
        if numbering {
            signals.push("numbering".to_string());
        }
        // 有显式篇序号时：区间与之一致才算佐证。没有显式序号时序号本来就取自区间，
        // 只有区间相对上一篇确实换了才算一个独立信号。
        let band_agrees = match draft.explicit_passage {
            Some(n) => band == Some(n),
            None => band.is_some() && previous.map_or(true, |(_, prev)| range_band(prev) != band),
        };
        if band_agrees {
            signals.push("band".to_string());
        }
        let explicit = draft.signals.iter().any(|s| matches!(*s, "heading" | "instruction"));
        let conflict = draft.explicit_passage.is_some() && band.is_some() && !band_agrees;
        let label = match draft.test {
            Some(test) => format!("Test {test} · P{passage}"),
            None => format!("P{passage}"),
        };
        if conflict {
            warnings.push(format!("{label}：标题写的是 Passage {passage}，题号范围却像另一篇，请核对。"));
        }
        let corroborated = numbering || band_agrees || draft.signals.contains(&"layout");
        let confidence = if conflict {
            "low"
        } else if explicit && corroborated {
            "high"
        } else if explicit || ((draft.signals.contains(&"layout") || draft.signals.contains(&"question_band")) && numbering && band_agrees) {
            "medium"
        } else {
            "low"
        };
        segments.push(PdfSplitSegment {
            label,
            test: draft.test,
            passage,
            pages: draft.pages.clone(),
            shares_start_page: draft.shares_start_page,
            question_range: range,
            signals,
            confidence: confidence.to_string(),
            answers: Vec::new(),
            fallback_answer_pages: Vec::new(),
        });
        previous = Some((draft.test, range));
    }

    // 6. 答案按篇分开。
    let answer_pages: Vec<u32> = answer_flags
        .iter()
        .enumerate()
        .filter(|(_, flag)| **flag)
        .map(|(index, _)| index as u32 + 1)
        .collect();
    assign_answers(pages, &features, &answer_pages, &mut segments, &mut warnings);

    let default_split = segments.len() >= 2
        && segments.iter().all(|s| s.confidence != "low");
    PdfSplitPlan {
        page_count,
        segments,
        answer_pages,
        unassigned_pages,
        default_split,
        warnings,
    }
}

// ───────────────────────── 答案分篇 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnswerSection {
    Unknown,
    Listening,
    Reading,
    Other,
}

fn section_heading(line: &str) -> Option<AnswerSection> {
    let squashed = squash(line);
    if squashed.len() > 40 {
        return None;
    }
    if squashed.starts_with("listening") && !squashed.starts_with("listeningandreading") {
        Some(AnswerSection::Listening)
    } else if squashed.starts_with("reading") || squashed.starts_with("academicreading") || squashed.starts_with("generaltrainingreading") {
        Some(AnswerSection::Reading)
    } else if squashed.starts_with("writing") || squashed.starts_with("speaking") {
        Some(AnswerSection::Other)
    } else {
        None
    }
}

/// 一行里的「题号 答案」。两栏版式的文本层常把 `1 TRUE   14 C` 放在同一行。
fn parse_answer_line(line: &str) -> Vec<(u32, String)> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let mut entries: Vec<(u32, String)> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].trim_end_matches(['.', ')']);
        let number = token.parse::<u32>().ok().filter(|n| (1..=40).contains(n));
        // 题号只出现在行首或一个答案之后；不在行首、前面也没有答案的数字不是题号。
        let Some(number) = number.filter(|_| index == 0 || !entries.is_empty()) else {
            if let Some(last) = entries.last_mut() {
                if !last.1.is_empty() {
                    last.1.push(' ');
                }
                last.1.push_str(tokens[index]);
            } else {
                return Vec::new();
            }
            index += 1;
            continue;
        };
        // 题号后面紧跟另一个题号 → 前一个其实是答案（数字答案）。
        if let Some(last) = entries.last_mut() {
            if last.1.is_empty() {
                last.1 = tokens[index].to_string();
                index += 1;
                continue;
            }
        }
        entries.push((number, String::new()));
        index += 1;
    }
    entries.retain(|(_, text)| !text.trim().is_empty() && text.split_whitespace().count() <= 8);
    entries
}

fn assign_answers(
    pages: &[String],
    features: &[PageFeatures],
    answer_pages: &[u32],
    segments: &mut [PdfSplitSegment],
    warnings: &mut Vec<String>,
) {
    if answer_pages.is_empty() || segments.is_empty() {
        return;
    }
    let has_listening_section = answer_pages.iter().any(|page| {
        pages[*page as usize - 1]
            .lines()
            .any(|line| section_heading(line) == Some(AnswerSection::Listening))
    });
    // 答案页所在位置之前最后一篇的 Test：没有 Test 标注时按位置归属。
    let starts: Vec<(u32, Option<u32>)> = segments
        .iter()
        .filter_map(|s| s.pages.first().map(|first| (*first, s.test)))
        .collect();
    let positional_test = |page: u32| -> Option<u32> {
        starts.iter().filter(|(first, _)| *first < page).last().and_then(|(_, test)| *test)
    };

    // (Test, 答案页) 用于退回附页。
    let mut pages_by_test: Vec<(Option<u32>, u32)> = Vec::new();
    for page in answer_pages {
        let text = &pages[*page as usize - 1];
        let mut test = features[*page as usize - 1].test.or_else(|| positional_test(*page));
        let mut section = AnswerSection::Unknown;
        let mut page_tests: Vec<Option<u32>> = vec![test];
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            if let Some(t) = test_heading(line) {
                test = Some(t);
                section = AnswerSection::Unknown;
                page_tests.push(test);
                continue;
            }
            if let Some(s) = section_heading(line) {
                section = s;
                if passage_heading(line).is_none() {
                    continue;
                }
            }
            if passage_heading(line).is_some() || normalize(line).contains("question") {
                // `Reading Passage 1, Questions 1-13` 这类分区行只提供上下文。
                if normalize(line).contains("reading passage") {
                    section = AnswerSection::Reading;
                }
                continue;
            }
            let reading = match section {
                AnswerSection::Reading => true,
                AnswerSection::Unknown => !has_listening_section,
                _ => false,
            };
            if !reading {
                continue;
            }
            for (number, answer) in parse_answer_line(line) {
                let target = segments.iter_mut().find(|s| {
                    (s.test.is_none() || test.is_none() || s.test == test)
                        && s.question_range.is_some_and(|(a, b)| (a..=b).contains(&number))
                });
                if let Some(segment) = target {
                    if !segment.answers.iter().any(|a| a.number == number) {
                        segment.answers.push(SplitAnswer { number, text: answer });
                    }
                }
            }
        }
        page_tests.dedup();
        for t in page_tests {
            pages_by_test.push((t, *page));
        }
    }

    for segment in segments.iter_mut() {
        segment.answers.sort_by_key(|a| a.number);
        let expected = segment
            .question_range
            .map(|(a, b)| (b - a + 1) as usize)
            .unwrap_or(0);
        let covered = segment.answers.len();
        if expected > 0 && covered * 5 >= expected * 4 {
            continue;
        }
        // 覆盖不足：答案不按文本分，退回把本套的答案页附上。
        segment.answers.clear();
        let mut fallback: Vec<u32> = pages_by_test
            .iter()
            .filter(|(t, _)| t.is_none() || segment.test.is_none() || *t == segment.test)
            .map(|(_, page)| *page)
            .collect();
        fallback.sort_unstable();
        fallback.dedup();
        if !fallback.is_empty() {
            warnings.push(format!(
                "{}：答案页没能按题号分出（{covered}/{expected}），已把答案页附到这一篇，请在题稿里核对答案。",
                segment.label
            ));
        }
        segment.fallback_answer_pages = fallback;
    }
}

// ───────────────────────── 文件入口 ─────────────────────────

fn text_poor_page(text: &str) -> bool {
    // Scans often have a selectable watermark/page number, so non-empty text
    // does not establish that the article or its heading was actually read.
    text.chars().filter(char::is_ascii_alphabetic).take(40).count() < 40
}

/// Two independent text readers. A failed/empty extraction is an unknown
/// outcome, never evidence that the file contains only one passage.
pub(crate) fn plan_pdf_file(path: &Path) -> PdfSplitPlan {
    let is_pdf = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        return PdfSplitPlan::default();
    }
    let mut warnings = Vec::new();
    let primary = match pdf_extract::extract_text_by_pages(path) {
        Ok(pages) => pages,
        Err(error) => {
            eprintln!("[import] split plan: text layer unreadable {}: {error}", path.display());
            warnings.push("PDF 文本读取失败，分篇检查可能不完整。".to_string());
            Vec::new()
        }
    };
    let mut plan = plan_from_page_texts(&primary);
    if !plan.is_multi() || !plan.default_split || primary.iter().any(|page| text_poor_page(page)) {
        match crate::pdf_geometry::extract_page_texts_for_split(path) {
            Ok(pages) => {
                let alternative = plan_from_page_texts(&pages);
                // Keep each reader's page plan intact; concatenating their text
                // would duplicate answer lines and manufacture boundary evidence.
                if alternative.is_multi() && (!plan.is_multi() || (!plan.default_split && alternative.default_split)) {
                    plan = alternative;
                } else if plan.page_count == 0 {
                    plan = alternative;
                }
                if pages.iter().any(|page| text_poor_page(page)) {
                    warnings.push("部分页面没有可读文字或文字过少，可能是扫描页；无法确认是否还有其他文章，需通过云端图像识别或人工核对分篇。".to_string());
                }
            }
            Err(error) => {
                eprintln!("[import] split plan: PDFium unavailable: {error}");
                warnings.push("备用 PDF 读取器不可用，分篇检查可能不完整；请检查是否使用了完整安装包。".to_string());
            }
        }
    }
    plan.warnings.extend(warnings);
    plan
}

/// 把一篇的页写成独立 PDF（原页面直接复制，不栅格化）。
pub(crate) fn write_segment_pdf(
    source: &Path,
    plan: &PdfSplitPlan,
    segment: &PdfSplitSegment,
    output: &Path,
) -> CommandResult<()> {
    let bytes = std::fs::read(source).map_err(|error| format!("split_read_source:{error}"))?;
    let pages = plan.pages_for(segment);
    let subset = crate::pdf_geometry::subset_pdf_pages(&bytes, &pages, plan.page_count as usize)
        .or_else(|error| {
            eprintln!("[import] split: PDFium page copy failed, trying built-in copier: {error}");
            subset_pdf_builtin(&bytes, &pages, plan.page_count)
        })?;
    std::fs::write(output, subset).map_err(|error| format!("split_write_segment:{error}"))
}

/// Native-library-independent page copier. Keep the original page tree and
/// inherited resources, fonts, images and rotation; remove only unselected pages.
fn subset_pdf_builtin(bytes: &[u8], pages: &[u32], original_count: u32) -> CommandResult<Vec<u8>> {
    let mut document = lopdf::Document::load_mem(bytes).map_err(|error| format!("split_pdf_open:{error}"))?;
    if document.get_pages().len() != original_count as usize || pages.is_empty()
        || pages.iter().any(|page| *page == 0 || *page > original_count)
        || pages.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("split_pdf_page_selection_invalid".into());
    }
    let removed: Vec<u32> = (1..=original_count).filter(|page| !pages.contains(page)).collect();
    document.delete_pages(&removed);
    document.prune_objects();
    if document.get_pages().len() != pages.len() {
        return Err("split_pdf_copy_incomplete".into());
    }
    let mut output = Vec::new();
    document.save_to(&mut output).map_err(|error| format!("split_pdf_save:{error}"))?;
    Ok(output)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportSplitPreview {
    pub path: String,
    pub page_count: u32,
    pub segments: Vec<PdfSplitSegment>,
    pub answer_pages: Vec<u32>,
    pub unassigned_pages: Vec<u32>,
    pub default_split: bool,
    pub warnings: Vec<String>,
}

/// `plan_import_split` 命令核心：逐文件给出切分预览。只有两篇及以上才返回段。
pub(crate) fn preview_import_split(paths: &[String]) -> Vec<ImportSplitPreview> {
    paths
        .iter()
        .map(|path| {
            let plan = plan_pdf_file(Path::new(path));
            if !plan.is_multi() {
                return ImportSplitPreview {
                    path: path.clone(),
                    page_count: plan.page_count,
                    segments: Vec::new(),
                    answer_pages: Vec::new(),
                    unassigned_pages: Vec::new(),
                    default_split: false,
                    warnings: plan.warnings,
                };
            }
            ImportSplitPreview {
                path: path.clone(),
                page_count: plan.page_count,
                segments: plan.segments,
                answer_pages: plan.answer_pages,
                unassigned_pages: plan.unassigned_pages,
                default_split: plan.default_split,
                warnings: plan.warnings,
            }
        })
        .collect()
}

/// 测试用：手写一份最小 PDF（Helvetica，每页若干行文本），不依赖外部 fixture。
#[cfg(test)]
pub(crate) fn minimal_text_pdf(pages: &[&[&str]]) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    let page_count = pages.len();
    let kids: Vec<String> = (0..page_count).map(|i| format!("{} 0 R", 4 + i * 2)).collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {page_count} >>", kids.join(" ")));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());
    for (index, lines) in pages.iter().enumerate() {
        let content_id = 5 + index * 2;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents {content_id} 0 R >>"
        ));
        let mut stream = String::from("BT /F1 10 Tf 12 TL 40 800 Td\n");
        for line in lines.iter() {
            let escaped = line.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
            stream.push_str(&format!("({escaped}) Tj T*\n"));
        }
        stream.push_str("ET");
        objects.push(format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
    );
    out
}

/// 测试用：一页像样的正文（≥ 6 行长句）。
#[cfg(test)]
pub(crate) fn prose_lines(topic: &str) -> Vec<String> {
    (0..7)
        .map(|i| format!("Paragraph {i} explains how {topic} developed over many years in several different places."))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(lines: &[&str]) -> String {
        lines.join("\n")
    }
    fn prose_page(head: &[&str], topic: &str) -> String {
        let mut lines: Vec<String> = head.iter().map(|s| s.to_string()).collect();
        lines.extend(prose_lines(topic));
        lines.join("\n")
    }
    fn questions(range: &str, first: u32, count: u32) -> String {
        let mut lines = vec![format!("Questions {range}"), "Do the following statements agree with the information given in the passage?".to_string()];
        for n in first..first + count {
            lines.push(format!("{n} The writer says that this particular statement about the topic is accurate."));
        }
        lines.join("\n")
    }
    fn summary(plan: &PdfSplitPlan) -> Vec<(String, Vec<u32>, String)> {
        plan.segments.iter().map(|s| (s.label.clone(), s.pages.clone(), s.confidence.clone())).collect()
    }

    /// 标准剑桥版式：说明行 + 标题 + 正文 → 题目，三篇，末尾总答案（含听力区）。
    fn cambridge_test() -> Vec<String> {
        vec![
            page(&["LISTENING", "SECTION 1 Questions 1-10", "Complete the form below."]),
            prose_page(&["READING", "READING PASSAGE 1", "You should spend about 20 minutes on Questions 1-13, which are based on Reading Passage 1 below."], "glass"),
            questions("1-13", 1, 13),
            prose_page(&["READING PASSAGE 2", "You should spend about 20 minutes on Questions 14-26, which are based on Reading Passage 2 below."], "horses"),
            questions("14-26", 14, 13),
            prose_page(&["READING PASSAGE 3", "You should spend about 20 minutes on Questions 27-40, which are based on Reading Passage 3 below."], "sleep"),
            questions("27-40", 27, 14),
            {
                let mut lines = vec!["Listening and Reading Answer Keys".to_string(), "LISTENING".to_string()];
                for n in 1..=10 { lines.push(format!("{n} listening{n}")); }
                lines.push("READING".to_string());
                lines.push("Reading Passage 1, Questions 1-13".to_string());
                for n in 1..=13 { lines.push(format!("{n} TRUE")); }
                lines.join("\n")
            },
            {
                let mut lines = vec!["Reading Passage 2, Questions 14-26".to_string()];
                for n in 14..=26 { lines.push(format!("{n} C")); }
                lines.push("Reading Passage 3, Questions 27-40".to_string());
                for n in 27..=40 { lines.push(format!("{n} NOT GIVEN")); }
                lines.join("\n")
            },
        ]
    }

    #[test]
    fn cambridge_layout_splits_with_high_confidence_and_per_passage_answers() {
        let plan = plan_from_page_texts(&cambridge_test());
        assert_eq!(
            summary(&plan),
            vec![
                ("P1".into(), vec![2, 3], "high".into()),
                ("P2".into(), vec![4, 5], "high".into()),
                ("P3".into(), vec![6, 7], "high".into()),
            ]
        );
        assert!(plan.default_split, "{:?}", plan.warnings);
        assert_eq!(plan.unassigned_pages, vec![1]);
        assert_eq!(plan.answer_pages, vec![8, 9]);
        let p1 = &plan.segments[0];
        assert_eq!(p1.question_range, Some((1, 13)));
        assert_eq!(p1.answers.len(), 13);
        assert!(p1.answers.iter().all(|a| a.text == "TRUE"), "听力区的答案不能进阅读：{:?}", p1.answers);
        assert_eq!(plan.segments[1].answers.first(), Some(&SplitAnswer { number: 14, text: "C".into() }));
        assert_eq!(plan.segments[2].answers.len(), 14);
        assert_eq!(plan.segments[2].answers[0].text, "NOT GIVEN");
        // 答案已按篇分出，子 PDF 不再附整页答案。
        assert_eq!(plan.pages_for(&plan.segments[1]), vec![4, 5]);
        assert!(plan.segments[0].signals.iter().any(|s| s == "instruction"));
        assert!(plan.segments[1].signals.iter().any(|s| s == "numbering"));
        assert!(plan.segments[1].signals.iter().any(|s| s == "band"));
    }

    #[test]
    fn passages_without_any_heading_split_on_layout_and_numbering_alone() {
        let plan = plan_from_page_texts(&[
            prose_page(&["The history of glass"], "glass"),
            questions("1-13", 1, 13),
            prose_page(&["The wild horse"], "horses"),
            questions("14-26", 14, 13),
            prose_page(&["Memory and sleep"], "sleep"),
            questions("27-40", 27, 14),
        ]);
        assert_eq!(plan.segments.len(), 3);
        assert_eq!(plan.segments[1].pages, vec![3, 4]);
        assert_eq!(plan.segments[1].passage, 2);
        assert!(plan.segments.iter().all(|s| s.confidence == "medium"), "{:?}", summary(&plan));
        assert!(plan.default_split);
    }

    #[test]
    fn questions_before_passage_layout_pairs_each_passage_with_its_own_questions() {
        let plan = plan_from_page_texts(&[
            questions("1-13", 1, 13),
            prose_page(&[], "glass"),
            questions("14-26", 14, 13),
            prose_page(&[], "horses"),
        ]);
        assert_eq!(plan.segments.len(), 2, "{:?}", summary(&plan));
        assert_eq!(plan.segments[0].pages, vec![1, 2]);
        assert_eq!(plan.segments[1].pages, vec![3, 4]);
        assert_eq!(plan.segments[1].passage, 2);
    }

    #[test]
    fn a_two_page_passage_is_not_split_by_layout() {
        // 正文 → 题目 → 正文续页（同一篇的题后还有正文）→ 题目续：题号不接力 → 不拆。
        let plan = plan_from_page_texts(&[
            prose_page(&["READING PASSAGE 1"], "glass"),
            questions("1-5", 1, 5),
            prose_page(&[], "glass"),
            questions("6-13", 6, 8),
        ]);
        // 6 接在 5 后面，但仍在第一篇的区间里：版式分界被撤回，只有一篇。
        assert_eq!(plan.segments.len(), 1, "{:?}", summary(&plan));
        assert_eq!(plan.segments[0].pages, vec![1, 2, 3, 4]);
    }

    #[test]
    fn heading_only_without_corroboration_is_medium_and_conflicts_are_low() {
        let plan = plan_from_page_texts(&[
            page(&["READING PASSAGE 1", "text"]),
            page(&["READING PASSAGE 2", "text"]),
        ]);
        assert_eq!(plan.segments.len(), 2);
        assert!(plan.segments.iter().all(|s| s.confidence == "medium"));

        let conflict = plan_from_page_texts(&[
            prose_page(&["READING PASSAGE 1"], "glass"),
            questions("1-13", 1, 13),
            prose_page(&["READING PASSAGE 3"], "horses"),
            questions("14-26", 14, 13),
        ]);
        assert_eq!(conflict.segments[1].confidence, "low");
        assert!(!conflict.default_split);
        assert!(!conflict.warnings.is_empty());
    }

    #[test]
    fn several_tests_in_one_file_split_and_route_answers_by_test() {
        let mut pages = Vec::new();
        for test in 1..=2 {
            pages.push(prose_page(&[&format!("Test {test}"), "READING PASSAGE 1"], "glass"));
            pages.push(page(&[&format!("Test {test}"), &questions("1-13", 1, 13)]));
            pages.push(prose_page(&[&format!("Test {test}"), "READING PASSAGE 2"], "horses"));
            pages.push(page(&[&format!("Test {test}"), &questions("14-26", 14, 13)]));
        }
        let mut key = vec!["Answer Key".to_string()];
        for test in 1..=2 {
            key.push(format!("Test {test}"));
            key.push("Reading".to_string());
            for n in 1..=26 { key.push(format!("{n} T{test}A{n}")); }
        }
        pages.push(key.join("\n"));
        let plan = plan_from_page_texts(&pages);
        let labels: Vec<_> = plan.segments.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["Test 1 · P1", "Test 1 · P2", "Test 2 · P1", "Test 2 · P2"]);
        assert_eq!(plan.segments[2].answers[0].text, "T2A1");
        assert_eq!(plan.segments[1].answers[0], SplitAnswer { number: 14, text: "T1A14".into() });
        assert!(plan.default_split, "{:?}", plan.warnings);
    }

    #[test]
    fn unparseable_answers_fall_back_to_that_tests_answer_pages() {
        let plan = plan_from_page_texts(&[
            prose_page(&["READING PASSAGE 1"], "glass"),
            questions("1-13", 1, 13),
            prose_page(&["READING PASSAGE 2"], "horses"),
            questions("14-26", 14, 13),
            page(&["Answer Key", "1 A", "2 B", "3 C", "(table continues as an image)"]),
        ]);
        assert_eq!(plan.answer_pages, vec![5]);
        assert!(plan.segments.iter().all(|s| s.answers.is_empty()));
        assert_eq!(plan.pages_for(&plan.segments[1]), vec![3, 4, 5]);
        assert!(plan.warnings.iter().any(|w| w.contains("答案页没能按题号分出")));
    }

    #[test]
    fn letter_spaced_heading_and_two_column_answer_lines() {
        assert_eq!(passage_heading("R E A D I N G  P A S S A G E  2"), Some(2));
        assert_eq!(passage_heading("Passage 3"), Some(3));
        assert_eq!(
            parse_answer_line("1 TRUE 14 C"),
            vec![(1, "TRUE".to_string()), (14, "C".to_string())]
        );
        assert_eq!(parse_answer_line("27 NOT GIVEN"), vec![(27, "NOT GIVEN".to_string())]);
        assert!(parse_answer_line("Write your answers in boxes 1-5").is_empty());
    }

    #[test]
    fn heading_parsers_reject_near_misses() {
        assert_eq!(passage_heading("READING PASSAGE 4"), None);
        assert_eq!(passage_heading("Questions on Reading Passage 1"), None);
        assert_eq!(test_heading("Test 1"), Some(1));
        assert_eq!(test_heading("Test your vocabulary"), None);
        assert!(answer_heading("ANSWER KEY"));
        assert!(answer_heading("Listening and Reading Answer Keys"));
        assert!(answer_heading("Answer key Test 3"));
        assert!(!answer_heading("Write your answers in boxes 1-5"));
        assert_eq!(question_heading_range("Questions 14–26"), Some((14, 26)));
        assert_eq!(question_heading_range("Questions 14to26"), Some((14, 26)));
        assert_eq!(question_heading_range("Questionnaire 1"), None);
    }

    #[test]
    fn fragmented_headings_and_wrapped_instructions_match_customer_layout() {
        let plan = plan_from_page_texts(&[
            page(&["2", "READING", "PASSAGE", "1", "You should spend about 20 minutes on", "Questions 1-13, which are based on", "Reading Passage 1 on pages 2 and 3."]),
            questions("1-13", 1, 13),
            page(&["6", "READING", "PASSAGE", "2", "You should spend about 20 minutes on", "Questions 14-26, which are based on Reading", "Passage 2 on pages 6 and 7."]),
            questions("14-26", 14, 13),
        ]);
        assert_eq!(plan.segments.len(), 2, "{:?}", summary(&plan));
        assert_eq!(plan.segments[1].pages, vec![3, 4]);
        assert_eq!(plan.segments[1].question_range, Some((14, 26)));
        assert!(plan.default_split);
    }

    #[test]
    fn instruction_alone_handles_missing_spaces_and_many_text_runs() {
        let plan = plan_from_page_texts(&[
            page(&["You should spend", "about 20 minutes", "on Questions 1-13,", "which are based on Reading", "Passage 1 on pages 2 and 3."]),
            page(&["Youshouldspendabout20minutesonQuestions14-26,whicharebasedonReadingPassage2onpages6and7."]),
        ]);
        assert_eq!(plan.segments.len(), 2, "{:?}", summary(&plan));
        assert_eq!(plan.segments[1].passage, 2);
        assert!(plan.default_split);
        let features = page_features("You should spend about 20 minutes on Questions 14 to 26, which are based on Reading Passage 2.");
        assert_eq!(features.instruction.unwrap().1, Some((14, 26)));
    }

    #[test]
    fn full_question_bands_can_suggest_split_without_prose_layout() {
        let plan = plan_from_page_texts(&[
            questions("1-13", 1, 13), questions("14-26", 14, 13), questions("27-40", 27, 14),
        ]);
        assert_eq!(plan.segments.len(), 3, "{:?}", summary(&plan));
        assert!(plan.segments[1].signals.iter().any(|s| s == "question_band"));
        let single = plan_from_page_texts(&[questions("1-5", 1, 5), questions("6-13", 6, 8)]);
        assert_eq!(single.segments.len(), 1);
    }

    #[test]
    fn builtin_copier_keeps_selected_pages_and_inherited_geometry() {
        let bytes = minimal_text_pdf(&[&["first"], &["second"], &["third"]]);
        let mut source = lopdf::Document::load_mem(&bytes).unwrap();
        let page_ids: Vec<_> = source.get_pages().into_values().collect();
        source.get_object_mut(page_ids[2]).unwrap().as_dict_mut().unwrap().set("Rotate", 90);
        let mut bytes = Vec::new();
        source.save_to(&mut bytes).unwrap();
        let result = subset_pdf_builtin(&bytes, &[1, 3], 3).unwrap();
        let output = lopdf::Document::load_mem(&result).unwrap();
        assert_eq!(output.get_pages().len(), 2);
        assert_eq!(output.extract_text(&[1, 2]).unwrap().trim(), "first\nthird");
        let last = output.get_pages()[&2];
        assert_eq!(output.get_object(last).unwrap().as_dict().unwrap().get(b"Rotate").unwrap().as_i64().unwrap(), 90);
        assert!(subset_pdf_builtin(&bytes, &[3, 1], 3).is_err());
        assert!(subset_pdf_builtin(&bytes, &[4], 3).is_err());
        assert!(subset_pdf_builtin(&bytes, &[1], 4).is_err());
    }

    #[test]
    fn failed_preflight_keeps_warnings_in_preview() {
        let previews = preview_import_split(&["does-not-exist.pdf".into()]);
        assert!(previews[0].segments.is_empty());
        assert!(!previews[0].warnings.is_empty());
        assert!(text_poor_page("6\nPublisher watermark"));
        assert!(!text_poor_page("You should spend about twenty minutes on questions fourteen to twenty six"));
    }

    #[test]
    fn real_pdf_text_layer_yields_the_same_plan() {
        let dir = std::env::temp_dir().join(format!("pdf-split-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("full-test.pdf");
        let pages = cambridge_test();
        let owned: Vec<Vec<&str>> = pages.iter().map(|p| p.lines().collect()).collect();
        let refs: Vec<&[&str]> = owned.iter().map(Vec::as_slice).collect();
        std::fs::write(&path, minimal_text_pdf(&refs)).unwrap();
        let plan = plan_pdf_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(plan.page_count, 9);
        let pages: Vec<_> = plan.segments.iter().map(|s| s.pages.clone()).collect();
        assert_eq!(pages, vec![vec![2, 3], vec![4, 5], vec![6, 7]]);
        assert_eq!(plan.segments[1].answers.len(), 13);
        assert!(plan.default_split, "{:?}", plan.warnings);
    }
}
