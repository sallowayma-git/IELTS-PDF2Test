//! 导入前把「一份 PDF 里的一整套卷子」拆成多篇阅读条目。
//!
//! 产品语义：阅读题库的一条 = 一篇 Passage（P1/P2/P3）。用户常拿到的是整套卷子
//! （一份 PDF 里 Passage 1–3，甚至 Test 1–4 × 3 篇）。以前整份文件只建一条，识别链
//! 只认第一篇，其余篇章静默丢失。现在导入时先按原文标题行切页，每篇建一条独立条目，
//! 用户在**题库主界面**按行切换，不需要在预览里切换篇章。
//!
//! 切分规则（只看文本层的整行标题，**不猜**）：
//!   - 篇章起点：整行是 `READING PASSAGE n`（n = 1..3，兼容字母间隔的文本层）；
//!   - 套题上下文：整行是 `TEST n`（页眉常见），篇章标题带上所属 Test；
//!   - 答案页：整行是 `ANSWER KEY` / `ANSWERS` / `READING ANSWER KEY` 这一类标题，
//!     从该页到文件末尾（遇到新篇章标题前）都算答案页，附加到每一篇的子 PDF 末尾，
//!     这样各条目的答案页识别照常工作；
//!   - 同一篇标题在后续页重复出现（续页页眉）视为续页，不开新篇；
//!   - 少于两篇时不拆分（返回单段），由调用方按整份导入。
//!
//! 听力不在这里拆：听力条目 = 一整套 Part 1–4，音频按 Part 绑定在同一条目上。

use std::path::Path;

use serde::Serialize;

use crate::CommandResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PdfSplitSegment {
    /// 展示用标签，例如 `Test 2 · P3`；没有 Test 上下文时就是 `P3`。
    pub label: String,
    pub test: Option<u32>,
    pub passage: u32,
    /// 本篇自己的页（1-based，升序）。
    pub pages: Vec<u32>,
    /// 起始页上方还有上一篇的内容（标题不在页首），该页同时归入两篇。
    pub shares_start_page: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PdfSplitPlan {
    pub page_count: u32,
    pub segments: Vec<PdfSplitSegment>,
    /// 附加到每一篇末尾的答案页（1-based，升序）。
    pub answer_pages: Vec<u32>,
    /// 第一篇之前、没有归入任何篇章的页（封面、说明、听力部分等）。
    pub unassigned_pages: Vec<u32>,
}

impl PdfSplitPlan {
    /// 至少两篇才值得拆。
    pub(crate) fn is_multi(&self) -> bool {
        self.segments.len() >= 2
    }

    /// 某一篇子 PDF 应包含的页：本篇页 + 答案页（去重、升序）。
    pub(crate) fn pages_for(&self, segment: &PdfSplitSegment) -> Vec<u32> {
        let mut pages: Vec<u32> = segment
            .pages
            .iter()
            .chain(self.answer_pages.iter())
            .copied()
            .collect();
        pages.sort_unstable();
        pages.dedup();
        pages
    }
}

fn squash(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 整行 `READING PASSAGE n`（n = 1..3）。允许行尾标点；字母间隔的文本层也能命中。
fn passage_heading(line: &str) -> Option<u32> {
    let squashed = squash(line);
    let rest = squashed.strip_prefix("readingpassage")?;
    let ordinal: u32 = match rest {
        "1" | "one" => 1,
        "2" | "two" => 2,
        "3" | "three" => 3,
        _ => return None,
    };
    Some(ordinal)
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
    matches!(
        squashed.as_str(),
        "answerkey"
            | "answerkeys"
            | "answers"
            | "readinganswerkey"
            | "readinganswers"
            | "listeningandreadinganswerkeys"
            | "listeningandreadinganswerkey"
            | "readingandlisteninganswerkeys"
            | "keyanswers"
    ) || (squashed.starts_with("answerkey") && squashed.len() <= "answerkeytest99".len()
        && squashed["answerkey".len()..]
            .strip_prefix("test")
            .unwrap_or(&squashed["answerkey".len()..])
            .chars()
            .all(|ch| ch.is_ascii_digit()))
}

/// 标题在页首附近：此前的非空行不超过这么多（页眉、页码、Test n）。
const HEADING_TOP_LINES: usize = 4;

/// 按逐页文本生成切分计划。`pages[i]` 是第 i+1 页的文本。
pub(crate) fn plan_from_page_texts(pages: &[String]) -> PdfSplitPlan {
    let page_count = pages.len() as u32;
    let mut segments: Vec<PdfSplitSegment> = Vec::new();
    let mut answer_pages: Vec<u32> = Vec::new();
    let mut unassigned_pages: Vec<u32> = Vec::new();
    let mut current_test: Option<u32> = None;
    let mut in_answers = false;

    for (index, text) in pages.iter().enumerate() {
        let page = index as u32 + 1;
        let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();

        let mut page_test: Option<u32> = None;
        let mut heading: Option<(u32, usize)> = None;
        let mut answers_here = false;
        for (line_index, line) in lines.iter().enumerate() {
            if page_test.is_none() {
                page_test = test_heading(line);
            }
            if heading.is_none() {
                if let Some(passage) = passage_heading(line) {
                    heading = Some((passage, line_index));
                }
            }
            if answer_heading(line) {
                answers_here = true;
            }
        }
        if let Some(test) = page_test {
            current_test = Some(test);
        }

        // 新篇章标题优先于答案页状态：答案页之后还有下一套题（Test 2…）时继续切。
        let new_segment = heading.filter(|(passage, _)| {
            segments.last().map_or(true, |last| {
                in_answers || last.passage != *passage || last.test != current_test
            })
        });
        if let Some((passage, line_index)) = new_segment {
            in_answers = false;
            let shares_start_page = line_index > HEADING_TOP_LINES
                && segments.last().is_some_and(|last| last.pages.last() == Some(&(page - 1)));
            if shares_start_page {
                if let Some(last) = segments.last_mut() {
                    last.pages.push(page);
                }
            }
            segments.push(PdfSplitSegment {
                label: match current_test {
                    Some(test) => format!("Test {test} · P{passage}"),
                    None => format!("P{passage}"),
                },
                test: current_test,
                passage,
                pages: vec![page],
                shares_start_page,
            });
            continue;
        }

        if answers_here && heading.is_none() {
            in_answers = true;
        }
        if in_answers {
            answer_pages.push(page);
        } else if let Some(last) = segments.last_mut() {
            last.pages.push(page);
        } else {
            unassigned_pages.push(page);
        }
    }

    PdfSplitPlan {
        page_count,
        segments,
        answer_pages,
        unassigned_pages,
    }
}

/// 读 PDF 文本层并生成切分计划。读不到文本（扫描件）时返回空计划（不拆）。
pub(crate) fn plan_pdf_file(path: &Path) -> PdfSplitPlan {
    let is_pdf = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        return PdfSplitPlan::default();
    }
    match pdf_extract::extract_text_by_pages(path) {
        Ok(pages) => plan_from_page_texts(&pages),
        Err(error) => {
            eprintln!("[import] split plan: text layer unreadable {}: {error}", path.display());
            PdfSplitPlan::default()
        }
    }
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
    let subset =
        crate::pdf_geometry::subset_pdf_pages(&bytes, &pages, plan.page_count as usize)?;
    std::fs::write(output, subset).map_err(|error| format!("split_write_segment:{error}"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportSplitPreview {
    pub path: String,
    pub page_count: u32,
    pub segments: Vec<PdfSplitSegment>,
    pub answer_pages: Vec<u32>,
    pub unassigned_pages: Vec<u32>,
}

/// `plan_import_split` 命令核心：逐文件给出切分预览。只有两篇及以上才返回段。
pub(crate) fn preview_import_split(paths: &[String]) -> Vec<ImportSplitPreview> {
    paths
        .iter()
        .map(|path| {
            let plan = plan_pdf_file(Path::new(path));
            let multi = plan.is_multi();
            ImportSplitPreview {
                path: path.clone(),
                page_count: plan.page_count,
                segments: if multi { plan.segments } else { Vec::new() },
                answer_pages: if multi { plan.answer_pages } else { Vec::new() },
                unassigned_pages: if multi { plan.unassigned_pages } else { Vec::new() },
            }
        })
        .collect()
}

/// 测试用：手写一份最小 PDF（Helvetica，每页若干行文本），不依赖外部 fixture。
#[cfg(test)]
pub(crate) fn minimal_text_pdf(pages: &[&[&str]]) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    let page_count = pages.len();
    // 1 catalog, 2 pages, 3 font, then per page: page obj + content obj.
    let kids: Vec<String> = (0..page_count).map(|i| format!("{} 0 R", 4 + i * 2)).collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {page_count} >>", kids.join(" ")));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());
    for (index, lines) in pages.iter().enumerate() {
        let content_id = 5 + index * 2;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents {content_id} 0 R >>"
        ));
        let mut stream = String::from("BT /F1 12 Tf 14 TL 72 780 Td\n");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| text.to_string()).collect()
    }

    #[test]
    fn one_test_with_three_passages_splits_into_three() {
        let plan = plan_from_page_texts(&pages(&[
            "READING PASSAGE 1\nYou should spend about 20 minutes\nThe history of glass",
            "more passage one",
            "Questions 1-13",
            "READING PASSAGE 2\nThe wild horse",
            "Questions 14-26",
            "READING PASSAGE 3\nMemory and sleep",
            "Questions 27-40",
        ]));
        assert!(plan.is_multi());
        assert_eq!(plan.segments.len(), 3);
        assert_eq!(plan.segments[0].pages, vec![1, 2, 3]);
        assert_eq!(plan.segments[1].pages, vec![4, 5]);
        assert_eq!(plan.segments[2].pages, vec![6, 7]);
        assert_eq!(plan.segments[2].label, "P3");
        assert!(plan.answer_pages.is_empty());
    }

    #[test]
    fn answer_pages_attach_to_every_passage() {
        let plan = plan_from_page_texts(&pages(&[
            "Cover page",
            "READING PASSAGE 1\ntext",
            "READING PASSAGE 2\ntext",
            "READING PASSAGE 3\ntext",
            "Answer Key\n1 TRUE\n2 FALSE",
            "28 C\n29 B",
        ]));
        assert_eq!(plan.unassigned_pages, vec![1]);
        assert_eq!(plan.answer_pages, vec![5, 6]);
        assert_eq!(plan.pages_for(&plan.segments[1]), vec![3, 5, 6]);
    }

    #[test]
    fn letter_spaced_headings_and_test_context_are_recognised() {
        let plan = plan_from_page_texts(&pages(&[
            "Test 1\nR E A D I N G  P A S S A G E  1\ntext",
            "Test 1\nREADING PASSAGE 2\ntext",
            "Test 1\nREADING PASSAGE 3\ntext",
            "Test 2\nREADING PASSAGE 1\ntext",
            "Test 2\nREADING PASSAGE 2\ntext",
        ]));
        let labels: Vec<_> = plan.segments.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["Test 1 · P1", "Test 1 · P2", "Test 1 · P3", "Test 2 · P1", "Test 2 · P2"]);
    }

    #[test]
    fn repeated_running_heading_is_a_continuation_not_a_new_passage() {
        let plan = plan_from_page_texts(&pages(&[
            "READING PASSAGE 1\ntext",
            "READING PASSAGE 1\ncontinued",
            "READING PASSAGE 2\ntext",
        ]));
        assert_eq!(plan.segments.len(), 2);
        assert_eq!(plan.segments[0].pages, vec![1, 2]);
    }

    #[test]
    fn single_passage_or_prose_mentions_do_not_split() {
        let single = plan_from_page_texts(&pages(&["READING PASSAGE 2\ntext", "Questions 14-26"]));
        assert!(!single.is_multi());
        // 正文里提到 Reading Passage 1 不是标题行。
        let prose = plan_from_page_texts(&pages(&[
            "READING PASSAGE 1\nAs noted in Reading Passage 2 below, ...",
            "Questions 1-13",
        ]));
        assert_eq!(prose.segments.len(), 1);
    }

    #[test]
    fn heading_low_on_a_page_shares_that_page_with_the_previous_passage() {
        let plan = plan_from_page_texts(&pages(&[
            "READING PASSAGE 1\ntext",
            "Questions 10-13\na\nb\nc\nd\ne\nREADING PASSAGE 2\nThe wild horse",
            "Questions 14-26",
        ]));
        assert_eq!(plan.segments[0].pages, vec![1, 2]);
        assert_eq!(plan.segments[1].pages, vec![2, 3]);
        assert!(plan.segments[1].shares_start_page);
    }

    #[test]
    fn next_test_after_an_answer_key_starts_new_passages() {
        let plan = plan_from_page_texts(&pages(&[
            "Test 1\nREADING PASSAGE 1",
            "Test 1\nREADING PASSAGE 2",
            "Answers\n1 A",
            "Test 2\nREADING PASSAGE 1",
            "Test 2\nREADING PASSAGE 2",
        ]));
        assert_eq!(plan.segments.len(), 4);
        assert_eq!(plan.answer_pages, vec![3]);
        assert_eq!(plan.segments[2].label, "Test 2 · P1");
    }

    #[test]
    fn real_pdf_text_layer_yields_the_same_plan() {
        let dir = std::env::temp_dir().join(format!("pdf-split-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("full-test.pdf");
        std::fs::write(
            &path,
            minimal_text_pdf(&[
                &["READING PASSAGE 1", "The history of glass"],
                &["Questions 1-13"],
                &["READING PASSAGE 2", "The wild horse"],
                &["READING PASSAGE 3", "Memory and sleep"],
                &["Answer Key", "1 TRUE"],
            ]),
        )
        .unwrap();
        let plan = plan_pdf_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(plan.page_count, 5);
        let pages: Vec<_> = plan.segments.iter().map(|s| s.pages.clone()).collect();
        assert_eq!(pages, vec![vec![1, 2], vec![3], vec![4]]);
        assert_eq!(plan.answer_pages, vec![5]);
    }

    #[test]
    fn heading_parsers_reject_near_misses() {
        assert_eq!(passage_heading("READING PASSAGE 4"), None);
        assert_eq!(passage_heading("Reading Passage 12"), None);
        assert_eq!(passage_heading("Questions on Reading Passage 1"), None);
        assert_eq!(test_heading("Test 1"), Some(1));
        assert_eq!(test_heading("Testing 1"), None);
        assert_eq!(test_heading("Test your vocabulary"), None);
        assert!(answer_heading("ANSWER KEY"));
        assert!(answer_heading("Answer key Test 3"));
        assert!(!answer_heading("Write your answers in boxes 1-5"));
    }
}
