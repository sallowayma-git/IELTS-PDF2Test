//! 题库条目的 Part 标签判定。
//!
//! 语义：阅读 = Passage 1/2/3（显示为 P1/P2/P3）；听力 = Part 1–4；写作 = Task 1/2。
//!
//! 判定优先级（高→低），判不出就返回 `None`（**不猜**）：
//!   1. manual   —— 用户手动设置；
//!   2. content  —— 原文标题行（"READING PASSAGE n" / "PART n" / "SECTION n"）；
//!   3. range    —— 题号范围（阅读 1–13→P1，14–26→P2，27–40→P3；跨段或不连续→不判定）；
//!   4. filename —— 文件名/标题里的 "P1" / "Passage 1" / "Part 1" / "Section 1" / "Task 1"。
//!
//! 写作直接用 taskType（task1→Task 1，task2→Task 2）。
//!
//! 判定结果连同**来源**一起存进 `library_items_v2`（见 schema 迁移 v10）。本模块是纯函数，
//! 输入都由调用方从条目数据（原文行 / 题号 / 文件名 / 手动值）备好，方便单测钉死期望值。

/// 判定来源，落库为字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartSource {
    Manual,
    Content,
    Range,
    Filename,
}

impl PartSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PartSource::Manual => "manual",
            PartSource::Content => "content",
            PartSource::Range => "range",
            PartSource::Filename => "filename",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetectedPart {
    pub label: String,
    pub source: PartSource,
}

pub(crate) struct PartDetectionInput<'a> {
    pub modality: &'a str,
    /// 用户手动设置的标签（已是展示形态，如 "P2" / "Part 3" / "Task 1"）。
    pub manual_label: Option<&'a str>,
    /// 写作 taskType（"task1" / "task2"）。
    pub task_type: Option<&'a str>,
    /// 原文行（用于标题行判定）。
    pub source_lines: &'a [String],
    /// 题号（用于范围判定，主要是阅读）。
    pub question_numbers: &'a [u32],
    /// 文件名或标题。
    pub filename: &'a str,
}

/// 阅读题号 → Passage：1–13→1，14–26→2，27–40→3。范围外返回 `None`。
fn reading_band(number: u32) -> Option<u32> {
    match number {
        1..=13 => Some(1),
        14..=26 => Some(2),
        27..=40 => Some(3),
        _ => None,
    }
}

fn reading_label(ordinal: u32) -> Option<String> {
    (1..=3).contains(&ordinal).then(|| format!("P{ordinal}"))
}

fn listening_label(ordinal: u32) -> Option<String> {
    (1..=4).contains(&ordinal).then(|| format!("Part {ordinal}"))
}

/// 从一行里解析 "READING PASSAGE n" 标题（整行就是标题，尾部可有标点）。
fn parse_reading_passage_heading(line: &str) -> Option<u32> {
    let upper = line.trim().to_ascii_uppercase();
    let rest = upper.strip_prefix("READING PASSAGE")?;
    let rest = rest.trim_start();
    let mut chars = rest.chars();
    let digit = chars.next()?.to_digit(10)?;
    // 允许尾随标点/空白，但不允许再跟数字（"READING PASSAGE 12" 不是有效的 1..=3）。
    if chars
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    (1..=3).contains(&digit).then_some(digit)
}

/// 内容判定：扫描原文行找第一处 Passage/Part/Section 标题。
fn detect_from_content(modality: &str, lines: &[String]) -> Option<String> {
    match modality {
        "reading" => lines
            .iter()
            .find_map(|line| parse_reading_passage_heading(line))
            .and_then(reading_label),
        "listening" => lines.iter().find_map(|line| {
            crate::ielts_grammar::listening_parts::parse_part_heading(line)
                .and_then(|(_, ordinal)| listening_label(ordinal))
        }),
        _ => None,
    }
}

/// 范围判定（阅读）：题号必须落在同一段且**连续**，否则不判定。
fn detect_from_range(modality: &str, numbers: &[u32]) -> Option<String> {
    if modality != "reading" || numbers.is_empty() {
        return None;
    }
    let mut sorted: Vec<u32> = numbers.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let (min, max) = (*sorted.first()?, *sorted.last()?);
    // 不连续 → 不判定。
    if (max - min + 1) as usize != sorted.len() {
        return None;
    }
    // 跨段 → 不判定（首尾必须落在同一段）。
    let band = reading_band(min)?;
    if reading_band(max)? != band {
        return None;
    }
    reading_label(band)
}

/// 文件名判定：找 "passage n" / "p n" / "part n" / "section n" / "task n"。
fn detect_from_filename(modality: &str, filename: &str) -> Option<String> {
    let lower = filename.to_ascii_lowercase();
    let digit_after = |needle: &str| -> Option<u32> {
        let mut from = 0usize;
        while let Some(pos) = lower[from..].find(needle) {
            let abs = from + pos;
            // 词边界：needle 必须是一个 token 的开头，否则 "group 3"→'p'、"deep 2"、
            // "step 1"、"department 3" 这类词内子串会被误判成 Part（裸 "p" 尤其危险）。
            let boundary = abs == 0 || !lower.as_bytes()[abs - 1].is_ascii_alphanumeric();
            if boundary {
                let rest = lower[abs + needle.len()..].trim_start();
                if let Some(digit) = rest.chars().next().and_then(|ch| ch.to_digit(10)) {
                    // 防止把 "P12"/"part 10" 里的多位数误判成单段。
                    let mut chars = rest.chars();
                    chars.next();
                    if !chars.next().is_some_and(|ch| ch.is_ascii_digit()) {
                        return Some(digit);
                    }
                }
            }
            from = abs + needle.len();
        }
        None
    };
    match modality {
        "reading" => ["passage ", "passage", "p"]
            .iter()
            .find_map(|needle| digit_after(needle))
            .and_then(reading_label),
        "listening" => ["part ", "part", "section ", "section"]
            .iter()
            .find_map(|needle| digit_after(needle))
            .and_then(listening_label),
        "writing" => ["task ", "task"]
            .iter()
            .find_map(|needle| digit_after(needle))
            .filter(|d| (1..=2).contains(d))
            .map(|d| format!("Task {d}")),
        _ => None,
    }
}

/// 写作直接用 taskType。
fn detect_writing_task(task_type: Option<&str>) -> Option<String> {
    match task_type {
        Some("task1") => Some("Task 1".to_string()),
        Some("task2") => Some("Task 2".to_string()),
        _ => None,
    }
}

/// 按优先级判定 Part 标签，判不出返回 `None`。
pub(crate) fn detect_part(input: &PartDetectionInput<'_>) -> Option<DetectedPart> {
    // 1. 手动设置最高优先。
    if let Some(manual) = input.manual_label.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(DetectedPart {
            label: manual.to_string(),
            source: PartSource::Manual,
        });
    }

    // 写作：taskType 即答案（当作 content 级来源）。
    if input.modality == "writing" {
        return detect_writing_task(input.task_type)
            .or_else(|| detect_from_filename("writing", input.filename))
            .map(|label| DetectedPart {
                label,
                source: PartSource::Content,
            });
    }

    // 2. 原文标题。
    if let Some(label) = detect_from_content(input.modality, input.source_lines) {
        return Some(DetectedPart {
            label,
            source: PartSource::Content,
        });
    }
    // 3. 题号范围。
    if let Some(label) = detect_from_range(input.modality, input.question_numbers) {
        return Some(DetectedPart {
            label,
            source: PartSource::Range,
        });
    }
    // 4. 文件名。
    if let Some(label) = detect_from_filename(input.modality, input.filename) {
        return Some(DetectedPart {
            label,
            source: PartSource::Filename,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(
        modality: &'a str,
        filename: &'a str,
        lines: &'a [String],
        numbers: &'a [u32],
    ) -> PartDetectionInput<'a> {
        PartDetectionInput {
            modality,
            manual_label: None,
            task_type: None,
            source_lines: lines,
            question_numbers: numbers,
            filename,
        }
    }

    fn detect(i: &PartDetectionInput<'_>) -> Option<(String, PartSource)> {
        detect_part(i).map(|d| (d.label, d.source))
    }

    #[test]
    fn filename_passage_and_p_forms() {
        // 任务示例："7. P1 - Chili peppers" → P1。
        assert_eq!(
            detect(&input("reading", "7. P1 - Chili peppers", &[], &[])),
            Some(("P1".to_string(), PartSource::Filename))
        );
        assert_eq!(
            detect(&input("reading", "Passage 3 - Sleep study", &[], &[])),
            Some(("P3".to_string(), PartSource::Filename))
        );
        assert_eq!(
            detect(&input("listening", "Listening Part 4 mock", &[], &[])),
            Some(("Part 4".to_string(), PartSource::Filename))
        );
        assert_eq!(
            detect(&input("listening", "Section 2 recording", &[], &[])),
            Some(("Part 2".to_string(), PartSource::Filename))
        );
    }

    #[test]
    fn content_heading_beats_filename() {
        let lines = vec!["READING PASSAGE 2".to_string(), "Some body text".to_string()];
        // 文件名说 P1，但原文标题说 Passage 2 —— 内容优先。
        assert_eq!(
            detect(&input("reading", "p1-conformity", &lines, &[])),
            Some(("P2".to_string(), PartSource::Content))
        );
        let listening = vec!["SECTION 3".to_string()];
        assert_eq!(
            detect(&input("listening", "whatever", &listening, &[])),
            Some(("Part 3".to_string(), PartSource::Content))
        );
    }

    #[test]
    fn range_maps_reading_bands_when_contiguous_and_single_band() {
        assert_eq!(
            detect(&input("reading", "no-hint", &[], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13])),
            Some(("P1".to_string(), PartSource::Range))
        );
        assert_eq!(
            detect(&input("reading", "no-hint", &[], &(14..=26).collect::<Vec<_>>())),
            Some(("P2".to_string(), PartSource::Range))
        );
        assert_eq!(
            detect(&input("reading", "no-hint", &[], &(27..=40).collect::<Vec<_>>())),
            Some(("P3".to_string(), PartSource::Range))
        );
    }

    #[test]
    fn range_refuses_when_spanning_bands_or_non_contiguous() {
        // 跨段（P1 到 P2）→ 不判定。
        assert_eq!(detect(&input("reading", "no-hint", &[], &(10..=20).collect::<Vec<_>>())), None);
        // 不连续 → 不判定。
        assert_eq!(detect(&input("reading", "no-hint", &[], &[1, 2, 40])), None);
        // 范围外 → 不判定。
        assert_eq!(detect(&input("reading", "no-hint", &[], &[41, 42])), None);
    }

    #[test]
    fn content_beats_range() {
        let lines = vec!["READING PASSAGE 1".to_string()];
        // 原文说 Passage 1，题号却像 P2；内容优先于范围。
        assert_eq!(
            detect(&input("reading", "x", &lines, &(14..=26).collect::<Vec<_>>())),
            Some(("P1".to_string(), PartSource::Content))
        );
    }

    #[test]
    fn writing_uses_task_type() {
        let i = PartDetectionInput {
            modality: "writing",
            manual_label: None,
            task_type: Some("task2"),
            source_lines: &[],
            question_numbers: &[],
            filename: "essay",
        };
        assert_eq!(detect(&i), Some(("Task 2".to_string(), PartSource::Content)));
    }

    #[test]
    fn manual_overrides_everything() {
        let lines = vec!["READING PASSAGE 1".to_string()];
        let i = PartDetectionInput {
            modality: "reading",
            manual_label: Some("P3"),
            task_type: None,
            source_lines: &lines,
            question_numbers: &(1..=13).collect::<Vec<_>>(),
            filename: "Passage 1",
        };
        assert_eq!(detect(&i), Some(("P3".to_string(), PartSource::Manual)));
    }

    #[test]
    fn filename_does_not_false_positive_on_word_internal_letters() {
        // 词内 p / part 不得触发 Part。
        assert_eq!(detect(&input("reading", "Group 3 elements", &[], &[])), None);
        assert_eq!(detect(&input("reading", "Deep 2 dive", &[], &[])), None);
        assert_eq!(detect(&input("reading", "Step 1 guide", &[], &[])), None);
        assert_eq!(detect(&input("listening", "Department 3 memo", &[], &[])), None);
        // 但真正的 token 仍然识别。
        assert_eq!(
            detect(&input("reading", "notes p 2 draft", &[], &[])),
            Some(("P2".to_string(), PartSource::Filename))
        );
    }

    #[test]
    fn no_signal_returns_none() {
        assert_eq!(detect(&input("reading", "untitled document", &[], &[])), None);
    }
}

