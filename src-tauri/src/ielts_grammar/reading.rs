use serde_json::{json, Value};

use super::instruction_zone::{normalize_instruction_text, SemanticLine};

pub(crate) fn passage_nodes(
    title: &str,
    lines: &[SemanticLine],
    source_anchors: Vec<Value>,
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
    for (index, line) in lines.iter().enumerate() {
        let text = normalize_instruction_text(&line.text);
        if text.is_empty() {
            continue;
        }
        nodes.push(json!({
            "type": "paragraph",
            "id": format!("passage-paragraph-{}", index + 1),
            "sourceAnchors": [line.source_anchor.clone()],
            "provenanceStatus": "source",
            "children": [text_node(
                &format!("passage-text-{}", index + 1),
                &text,
                Some(line.source_anchor.clone()),
            )]
        }));
    }
    nodes
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
}
