use serde_json::{json, Value};

use super::instruction_zone::{normalize_instruction_text, SemanticLine};

pub(crate) fn passage_nodes(
    title: &str,
    lines: &[SemanticLine],
    source_anchors: Vec<Value>,
) -> Vec<Value> {
    passage_nodes_with_layout(title, lines, source_anchors, false)
}

/// Layout recovery is enabled only for real PDF geometry. DOCX paragraphs and
/// the text-only parser's fabricated rectangles are already explicit boundaries.
pub(crate) fn passage_nodes_with_layout(
    title: &str,
    lines: &[SemanticLine],
    source_anchors: Vec<Value>,
    recover_pdf_layout: bool,
) -> Vec<Value> {
    let mut nodes = Vec::new();
    if !title.trim().is_empty() {
        nodes.push(json!({
            "type": "heading",
            "id": "passage-title",
            "sourceAnchors": source_anchors.clone(),
            "provenanceStatus": "source",
            "level": 2,
            "children": [text_node("passage-title-text", title, source_anchors.first().cloned())]
        }));
    }
    let markers = paragraph_markers(lines);
    if !markers.is_empty() {
        nodes.extend(labelled_paragraph_nodes(lines, &markers));
    } else {
        let mut paragraphs: Vec<(usize, String, Vec<Value>)> = Vec::new();
        let mut previous: Option<&SemanticLine> = None;
        let heights = lines
            .iter()
            .filter_map(|line| line.bbox)
            .map(|bbox| bbox[3])
            .filter(|height| height.is_finite() && *height > 0.0)
            .collect::<Vec<_>>();
        let mut heights = heights;
        heights.sort_by(f64::total_cmp);
        let line_height = heights.get(heights.len() / 2).copied().unwrap_or(12.0);
        for (index, line) in lines.iter().enumerate() {
            let text = normalize_instruction_text(&line.text);
            if text.is_empty() {
                continue;
            }
            let continuation = recover_pdf_layout
                && previous.is_some_and(|previous| {
                    layout_continues_paragraph(previous, line, line_height)
                });
            if continuation {
                if let Some((_, paragraph, anchors)) = paragraphs.last_mut() {
                    append_wrapped_text(paragraph, &text);
                    anchors.push(line.source_anchor.clone());
                }
            } else {
                paragraphs.push((index, text, vec![line.source_anchor.clone()]));
            }
            previous = Some(line);
        }
        for (index, text, anchors) in paragraphs {
            nodes.push(paragraph_node(
                &format!("passage-paragraph-{}", index + 1),
                &text,
                None,
                anchors,
            ));
        }
    }
    nodes
}

fn layout_continues_paragraph(
    previous: &SemanticLine,
    current: &SemanticLine,
    line_height: f64,
) -> bool {
    let (Some(a), Some(b)) = (previous.bbox, current.bbox) else {
        return false;
    };
    if a.iter().chain(b.iter()).any(|value| !value.is_finite())
        || a[2] <= 0.0
        || b[2] <= 0.0
        || a[3] <= 0.0
        || b[3] <= 0.0
    {
        return false;
    }
    // Font changes and explicit heading roles must retain their own boundary.
    if previous.role.contains("heading")
        || current.role.contains("heading")
        || a[3].max(b[3]) > a[3].min(b[3]) * 1.4
    {
        return false;
    }
    let overlap = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let left_shift = b[0] - a[0];
    if overlap <= 0.0 || left_shift > line_height * 0.75 || left_shift < -line_height * 2.5 {
        return false;
    }
    if current.page_index == previous.page_index + 1 {
        // Without explicit labels, join across pages only with strong lexical
        // continuation evidence. A fresh capitalised paragraph stays separate.
        return !ends_sentence(&previous.text)
            && current
                .text
                .trim_start()
                .chars()
                .next()
                .is_some_and(char::is_lowercase);
    }
    if current.page_index != previous.page_index {
        return false;
    }
    let gap = b[1] - (a[1] + a[3]);
    // Same baseline fragments can be pieces of one printed line; separate
    // columns are excluded above and by the horizontal distance here.
    if (b[1] - a[1]).abs() < line_height * 0.35 {
        let horizontal_gap = b[0] - (a[0] + a[2]);
        return horizontal_gap >= -line_height * 0.2 && horizontal_gap <= line_height;
    }
    gap >= -line_height * 0.2 && gap <= line_height * 0.65
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .trim_end_matches(['\"', '\'', '”', '’', ')', ']'])
        .ends_with(['.', '!', '?', '。', '！', '？'])
}

fn append_wrapped_text(paragraph: &mut String, text: &str) {
    if paragraph.ends_with('\u{00ad}') {
        paragraph.pop();
    } else if !(paragraph.ends_with('-')
        && paragraph[..paragraph.len() - 1].chars().last().is_some_and(char::is_alphanumeric)
        && text.chars().next().is_some_and(char::is_lowercase)) {
        paragraph.push(' ');
    }
    paragraph.push_str(text);
}

/// Build a stable label → real passage-node map. Empty maps stay empty so
/// matching-information and heading tasks cannot acquire guessed paragraph IDs.
pub(crate) fn paragraph_map_from_nodes(nodes: &[Value]) -> Value {
    let mut map = serde_json::Map::new();
    let mut seen = std::collections::BTreeSet::new();
    for node in nodes {
        let (Some(label), Some(id)) = (
            node.get("paragraphLabel").and_then(Value::as_str),
            node.get("id").and_then(Value::as_str),
        ) else {
            continue;
        };
        // Duplicate source labels are ambiguous; omit them from the target map.
        if !seen.insert(label.to_string()) {
            map.remove(label);
            continue;
        }
        map.insert(label.to_string(), Value::String(id.to_string()));
    }
    Value::Object(map)
}

fn labelled_paragraph_nodes(
    lines: &[SemanticLine],
    markers: &std::collections::BTreeMap<usize, (String, String)>,
) -> Vec<Value> {
    #[derive(Default)]
    struct Paragraph {
        label: Option<String>,
        text: Vec<String>,
        anchors: Vec<Value>,
        first_index: usize,
    }

    let mut paragraphs = Vec::<Paragraph>::new();
    let mut label_counts = std::collections::BTreeMap::<String, usize>::new();
    for (index, line) in lines.iter().enumerate() {
        let normalized = normalize_instruction_text(&line.text);
        if normalized.is_empty() {
            continue;
        }
        if let Some((label, remainder)) = markers.get(&index).cloned() {
            let count = label_counts.entry(label.clone()).or_default();
            *count += 1;
            paragraphs.push(Paragraph {
                label: (*count == 1).then_some(label),
                text: (!remainder.is_empty())
                    .then_some(remainder)
                    .into_iter()
                    .collect(),
                anchors: vec![line.source_anchor.clone()],
                first_index: index,
            });
        } else if let Some(paragraph) = paragraphs.last_mut() {
            paragraph.text.push(normalized);
            paragraph.anchors.push(line.source_anchor.clone());
        } else {
            // Preserve source text before the first explicit label as ordinary
            // content; it is not a synthetic paragraph target.
            paragraphs.push(Paragraph {
                label: None,
                text: vec![normalized],
                anchors: vec![line.source_anchor.clone()],
                first_index: index,
            });
        }
    }

    paragraphs
        .into_iter()
        .filter_map(|paragraph| {
            let mut text = String::new();
            for fragment in &paragraph.text {
                if text.is_empty() {
                    text.push_str(fragment);
                } else {
                    append_wrapped_text(&mut text, fragment);
                }
            }
            let text = text.trim().to_string();
            if text.is_empty() {
                return None;
            }
            let id = paragraph
                .label
                .as_ref()
                .map(|label| format!("passage-paragraph-{label}"))
                .unwrap_or_else(|| format!("passage-paragraph-{}", paragraph.first_index + 1));
            Some(paragraph_node(
                &id,
                &text,
                paragraph.label.as_deref(),
                paragraph.anchors,
            ))
        })
        .collect()
}

/// Collect explicit standalone labels and conservative inline prefixes. Some
/// PDF text layers flatten the visually isolated bold `A` into `A The ...`;
/// inline prefixes are accepted only when they form an ordered paragraph
/// label sequence, so an ordinary sentence such as `A study found ...` cannot
/// create a paragraph map by itself.
fn paragraph_markers(
    lines: &[SemanticLine],
) -> std::collections::BTreeMap<usize, (String, String)> {
    let mut markers = std::collections::BTreeMap::new();
    let mut prefixes = Vec::<(usize, String, String)>::new();
    for (index, line) in lines.iter().enumerate() {
        let normalized = normalize_instruction_text(&line.text);
        if let Some(marker) = paragraph_marker(&normalized) {
            markers.insert(index, marker);
        } else if let Some(prefix) = paragraph_prefix(&normalized) {
            prefixes.push((index, prefix.0, prefix.1));
        }
    }

    let mut best_run = Vec::<(usize, String, String)>::new();
    for start in 0..prefixes.len() {
        if prefixes[start].1 != "A" {
            continue;
        }
        let mut run = vec![prefixes[start].clone()];
        let mut expected = 'B';
        for candidate in prefixes.iter().skip(start + 1) {
            if candidate.1.chars().next() == Some(expected) && candidate.1.len() == 1 {
                run.push(candidate.clone());
                expected = char::from_u32(expected as u32 + 1).unwrap_or(expected);
                if expected > 'G' {
                    break;
                }
            } else {
                break;
            }
        }
        if run.len() > best_run.len() {
            best_run = run;
        }
    }
    if best_run.len() >= 2 {
        for (index, label, remainder) in best_run {
            markers.insert(index, (label, remainder));
        }
    }
    markers
}

pub(crate) fn has_source_paragraph_markers(lines: &[SemanticLine]) -> bool {
    !paragraph_markers(lines).is_empty()
}

fn paragraph_prefix(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    let mut chars = trimmed.chars();
    let label = chars.next()?;
    if !matches!(label, 'A'..='G') {
        return None;
    }
    let separator = chars.next()?;
    if !separator.is_ascii_whitespace() && !matches!(separator, '.' | ')' | ':') {
        return None;
    }
    let remainder = chars.as_str().trim_start().to_string();
    let first_word = remainder
        .trim_start_matches(['‘', '’', '“', '”', '\"', '\''])
        .split_whitespace()
        .next()?;
    if !first_word
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_uppercase())
    {
        return None;
    }
    Some((label.to_string(), remainder))
}

/// Recognize only explicit standalone labels or the printed `Paragraph A`
/// form. A capital letter followed by ordinary prose is not enough evidence.
fn paragraph_marker(text: &str) -> Option<(String, String)> {
    let normalized = normalize_instruction_text(text);
    let trimmed = normalized.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("paragraph ") {
        let tail = trimmed["paragraph ".len()..].trim_start();
        let mut chars = tail.chars();
        let label = chars.next()?;
        if !label.is_ascii_uppercase() {
            return None;
        }
        let remainder = chars
            .as_str()
            .trim_start_matches(|ch| matches!(ch, '.' | ')' | ':' | ' '))
            .trim();
        if remainder.is_empty()
            || remainder
                .chars()
                .next()
                .is_some_and(|ch| matches!(ch, '.' | ')' | ':'))
        {
            return Some((label.to_string(), String::new()));
        }
        return Some((label.to_string(), remainder.to_string()));
    }
    let stripped = trimmed.trim_end_matches(['.', ')', ':']).trim();
    let mut chars = stripped.chars();
    let label = chars.next()?;
    if label.is_ascii_uppercase() && chars.next().is_none() {
        return Some((label.to_string(), String::new()));
    }
    None
}

fn paragraph_node(id: &str, text: &str, label: Option<&str>, anchors: Vec<Value>) -> Value {
    let text_anchor = anchors.first().cloned();
    let mut node = json!({
        "type": "paragraph",
        "id": id,
        "sourceAnchors": anchors,
        "provenanceStatus": "source",
        "children": [text_node(&format!("{id}-text"), text, text_anchor)]
    });
    if let Some(label) = label {
        node["paragraphLabel"] = json!(label);
    }
    node
}

pub(crate) fn visual_passage_lines(lines: &[SemanticLine]) -> Vec<SemanticLine> {
    lines
        .iter()
        .filter(|line| {
            let text = normalize_instruction_text(&line.text);
            let lower = line.text.to_ascii_lowercase();
            !line.role.to_ascii_lowercase().contains("question")
                && !line.role.to_ascii_lowercase().contains("option")
                && !looks_like_numbered_question(&text)
                && !looks_like_option_label(&text)
                && !is_paper_section_header(&lower)
                && !lower.starts_with("question")
                && !lower.starts_with("in boxes")
                && !lower.starts_with("choose ")
                && !lower.starts_with("complete ")
                && !lower.starts_with("answers")
        })
        .cloned()
        .collect()
}

pub(crate) fn is_paper_section_header(lower: &str) -> bool {
    let compact = lower
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    compact.starts_with("readingpassage")
        && compact["readingpassage".len()..]
            .chars()
            .all(|ch| ch.is_ascii_digit())
}

fn looks_like_numbered_question(text: &str) -> bool {
    let trimmed = text.trim_start();
    let digit_count = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_count == 0 {
        return false;
    }
    matches!(
        trimmed[digit_count..].chars().next(),
        Some('.') | Some(')') | Some(':')
    )
}

fn looks_like_option_label(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars();
    let Some(label) = chars.next() else {
        return false;
    };
    if !label.is_ascii_uppercase() {
        return false;
    }
    matches!(chars.next(), Some('.') | Some(')') | Some(':') | Some(' '))
}

fn text_node(id: &str, text: &str, source_anchor: Option<Value>) -> Value {
    let mut node = json!({
        "type": "text",
        "id": id,
        "sourceAnchors": [],
        "provenanceStatus": "source",
        "text": text
    });
    if let Some(anchor) = source_anchor {
        node["sourceAnchors"] = json!([anchor]);
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, text: &str) -> SemanticLine {
        SemanticLine {
            id: id.to_string(),
            text: text.to_string(),
            source_anchor: json!({"nodeIds":[id]}),
            page_index: 0,
            order: 0,
            role: String::new(),
            bbox: None,
        }
    }

    #[test]
    fn fallback_passage_excludes_questions_before_the_passage() {
        let lines = vec![
            line("q1", "1. Celebrity culture changed rapidly."),
            line("q2", "2. The public followed new media."),
            line("p1", "Celebrity culture has a long history."),
            line("p2", "Newspapers made performers widely known."),
        ];
        let filtered = visual_passage_lines(&lines);
        assert_eq!(
            filtered
                .iter()
                .map(|line| line.id.as_str())
                .collect::<Vec<_>>(),
            vec!["p1", "p2"]
        );
    }

    #[test]
    fn fallback_passage_excludes_vertical_option_lines_without_punctuation() {
        let lines = vec![
            line("a", "A Emma"),
            line("p", "Emma led the research team."),
        ];
        let filtered = visual_passage_lines(&lines);
        assert_eq!(
            filtered
                .iter()
                .map(|line| line.id.as_str())
                .collect::<Vec<_>>(),
            vec!["p"]
        );
    }

    #[test]
    fn fallback_passage_excludes_question_page_reading_passage_header() {
        let lines = vec![
            line("header", "RE ADI NG P AS S AGE 2"),
            line("body", "Celebrity culture has a long history."),
        ];
        let filtered = visual_passage_lines(&lines);
        assert_eq!(
            filtered
                .iter()
                .map(|line| line.id.as_str())
                .collect::<Vec<_>>(),
            vec!["body"]
        );
    }

    #[test]
    fn explicit_paragraph_markers_label_and_group_only_real_targets() {
        let lines = vec![
            line("label-a", "A"),
            line("a-1", "The first paragraph starts here."),
            line("a-2", "It continues on the next source line."),
            line("label-b", "Paragraph B"),
            line("b-1", "The second paragraph has its own text."),
        ];
        let nodes = passage_nodes("Passage", &lines, Vec::new());
        let labelled = nodes
            .iter()
            .filter(|node| node.get("paragraphLabel").is_some())
            .collect::<Vec<_>>();

        assert_eq!(labelled.len(), 2);
        assert_eq!(labelled[0]["paragraphLabel"], "A");
        assert_eq!(labelled[0]["id"], "passage-paragraph-A");
        assert_eq!(
            labelled[0]["children"][0]["text"],
            "The first paragraph starts here. It continues on the next source line."
        );
        assert_eq!(labelled[1]["paragraphLabel"], "B");
        assert_eq!(
            labelled[1]["children"][0]["text"],
            "The second paragraph has its own text."
        );
    }

    #[test]
    fn flattened_bold_letter_prefixes_form_source_backed_paragraphs() {
        let lines = vec![
            line("p-a", "A The first source paragraph starts here."),
            line("p-a-cont", "It continues on the next physical line."),
            line("p-b", "B It is followed by another paragraph."),
            line("p-c", "C A number of factors are relevant."),
        ];
        let nodes = passage_nodes("Passage", &lines, Vec::new());
        assert_eq!(
            paragraph_map_from_nodes(&nodes),
            json!({
                "A":"passage-paragraph-A",
                "B":"passage-paragraph-B",
                "C":"passage-paragraph-C"
            })
        );
        assert_eq!(
            nodes[1]["children"][0]["text"],
            json!(
                "The first source paragraph starts here. It continues on the next physical line."
            )
        );
        assert_eq!(
            nodes[2]["children"][0]["text"],
            json!("It is followed by another paragraph.")
        );
    }

    #[test]
    fn an_ordinary_article_a_does_not_invent_paragraph_labels() {
        let lines = vec![
            line("ordinary", "A study of the ocean follows."),
            line("next", "Beneath the surface, sound travels."),
        ];
        let nodes = passage_nodes("Passage", &lines, Vec::new());
        assert_eq!(paragraph_map_from_nodes(&nodes), json!({}));
        assert_eq!(
            nodes[1]["children"][0]["text"],
            json!("A study of the ocean follows.")
        );
    }

    #[test]
    fn passage_without_explicit_markers_does_not_invent_paragraph_labels() {
        let nodes = passage_nodes(
            "Passage",
            &[
                line("p1", "An unlabelled passage paragraph."),
                line("p2", "Another source line."),
            ],
            Vec::new(),
        );
        assert!(nodes
            .iter()
            .all(|node| node.get("paragraphLabel").is_none()));
    }
    fn laid_out_line(id: &str, text: &str, x: f64, y: f64, width: f64) -> SemanticLine {
        let mut value = line(id, text);
        value.bbox = Some([x, y, width, 12.0]);
        value
    }

    #[test]
    fn pdf_wrapped_passage_restores_paragraphs_without_merging_real_boundaries() {
        let lines = vec![
            laid_out_line("p1-a", "A normal paragraph starts", 72.0, 100.0, 300.0),
            laid_out_line("p1-b", "and ends with a short line.", 72.0, 116.0, 140.0),
            laid_out_line("p2-a", "A second paragraph follows.", 72.0, 144.0, 300.0),
            laid_out_line(
                "p2-b",
                "Its wrapped sentence continues.",
                72.0,
                160.0,
                260.0,
            ),
            laid_out_line("p3", "An indented paragraph.", 88.0, 176.0, 260.0),
        ];
        let nodes = passage_nodes_with_layout("", &lines, vec![], true);
        assert_eq!(nodes.len(), 3);
        assert_eq!(
            nodes[0]["children"][0]["text"],
            "A normal paragraph starts and ends with a short line."
        );
        assert_eq!(nodes[0]["sourceAnchors"].as_array().unwrap().len(), 2);
        assert_eq!(
            nodes[1]["children"][0]["text"],
            "A second paragraph follows. Its wrapped sentence continues."
        );
        assert_eq!(paragraph_map_from_nodes(&nodes), json!({}));
        assert_eq!(
            passage_nodes_with_layout("", &lines, vec![], false).len(),
            5,
            "DOCX or fabricated geometry must retain explicit source paragraphs"
        );
    }

    #[test]
    fn pdf_paragraph_recovery_keeps_columns_headings_and_missing_layout_separate() {
        let mut heading = laid_out_line("heading", "A heading", 72.0, 100.0, 300.0);
        heading.role = "heading".into();
        let lines = vec![
            heading,
            laid_out_line("body", "Body starts here.", 72.0, 116.0, 230.0),
            laid_out_line("column", "A different column.", 340.0, 132.0, 200.0),
            line("unknown", "No layout evidence."),
        ];
        assert_eq!(passage_nodes_with_layout("", &lines, vec![], true).len(), 4);
    }

    #[test]
    fn pdf_paragraph_recovery_joins_image_wrapping_and_proven_page_continuation() {
        let mut continuation = laid_out_line(
            "next-page",
            "continues on the next page.",
            72.0,
            60.0,
            400.0,
        );
        continuation.page_index = 1;
        let mut new_paragraph = laid_out_line(
            "new-page-paragraph",
            "A fresh paragraph.",
            72.0,
            90.0,
            400.0,
        );
        new_paragraph.page_index = 1;
        let lines = vec![
            laid_out_line(
                "narrow",
                "This wraps around an image and",
                72.0,
                100.0,
                230.0,
            ),
            laid_out_line(
                "wide",
                "continues across the full page and",
                72.0,
                116.0,
                460.0,
            ),
            continuation,
            new_paragraph,
        ];
        let nodes = passage_nodes_with_layout("", &lines, vec![], true);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0]["children"][0]["text"], "This wraps around an image and continues across the full page and continues on the next page.");
    }
}
