use crate::{authoring_review::answer_is_empty, html_escape};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadingSourceMetaV1 {
    pub title: String,
    pub category: String,
    pub frequency: String,
    pub pdf_filename: String,
    pub legacy_path: String,
    pub legacy_filename: String,
    pub question_intro_html: String,
    pub question_umbrella_ranges: Vec<UmbrellaQuestionRangeV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UmbrellaQuestionRangeV1 {
    pub heading: String,
    pub question_range: [u32; 2],
    pub block_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadingPassageBlockV1 {
    pub block_id: String,
    pub kind: String,
    pub html: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadingPassageV1 {
    pub blocks: Vec<ReadingPassageBlockV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadingQuestionGroupV1 {
    pub group_id: String,
    pub kind: String,
    pub question_ids: Vec<String>,
    pub body_html: String,
    pub lead_html: String,
    pub allow_option_reuse: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadingSourceRefsV1 {
    pub primary_html: String,
    pub primary_provider: String,
    pub shui_html: Option<String>,
    pub shui_pdf: String,
    pub ielts_html: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadingSourceAuditV1 {
    pub match_status: String,
    pub match_confidence: f64,
    pub verified_at: Option<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadingExamSourceV1 {
    pub schema_version: String,
    pub exam_id: String,
    pub meta: ReadingSourceMetaV1,
    pub passage: ReadingPassageV1,
    pub question_groups: Vec<ReadingQuestionGroupV1>,
    pub answer_key: serde_json::Map<String, Value>,
    pub source_refs: ReadingSourceRefsV1,
    pub audit: ReadingSourceAuditV1,
    pub question_order: Vec<String>,
    pub question_display_map: serde_json::Map<String, Value>,
}

impl ReadingExamSourceV1 {
    fn to_value(&self) -> Value {
        serde_json::to_value(self)
            .expect("ReadingExamSourceV1 only contains JSON-serializable fields")
    }
}

fn string_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn group_layout_hint(group: &Value) -> &str {
    group
        .pointer("/layout/layoutHint")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn group_layout_template(group: &Value) -> &str {
    group
        .pointer("/layout/template")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn group_layout_notes(group: &Value) -> &str {
    group
        .pointer("/layout/notes")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn question_display_and_id(question: &Value) -> Option<(String, String)> {
    let display = string_at(question, "displayNumber").trim();
    let qid = string_at(question, "id").trim();
    if display.is_empty() || qid.is_empty() {
        None
    } else {
        Some((display.to_string(), qid.to_string()))
    }
}

fn render_option_content(question: &Value, option: &str) -> String {
    let label = html_escape(option);
    let option_text = question
        .pointer("/interaction/optionTexts")
        .and_then(Value::as_object)
        .and_then(|texts| texts.get(option))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    match option_text {
        Some(text) => format!(
            "<span class=\"choice-label\">{}</span> <span class=\"choice-text\">{}</span>",
            label,
            html_escape(text)
        ),
        None => label,
    }
}

fn question_has_options(question: &Value) -> bool {
    question
        .pointer("/interaction/options")
        .and_then(Value::as_array)
        .map(|options| options.iter().any(|option| option.as_str().is_some()))
        .unwrap_or(false)
}

fn render_option_controls(question: &Value, input_type: &str) -> String {
    let qid = string_at(question, "id");
    question
        .pointer("/interaction/options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|option| {
            format!(
                "<label><input name=\"{}\" type=\"{}\" value=\"{}\"> {}</label>",
                html_escape(qid),
                input_type,
                html_escape(option),
                render_option_content(question, option)
            )
        })
        .collect::<String>()
}

fn render_question_list_item(question: &Value, checkbox: bool) -> String {
    let qid = string_at(question, "id");
    if checkbox || question_has_options(question) {
        let input_type = if checkbox { "checkbox" } else { "radio" };
        format!(
            "<li><div><strong>{}</strong> {}</div><div class=\"choice-row\">{}</div></li>",
            html_escape(string_at(question, "displayNumber")),
            html_escape(string_at(question, "prompt")),
            render_option_controls(question, input_type)
        )
    } else {
        format!(
            "<li><label><strong>{}</strong> {} <input type=\"text\" id=\"{}_input\" name=\"{}\"></label></li>",
            html_escape(string_at(question, "displayNumber")),
            html_escape(string_at(question, "prompt")),
            html_escape(qid),
            html_escape(qid)
        )
    }
}

fn is_inline_blank_marker_char(ch: char) -> bool {
    matches!(
        ch,
        '_' | '.'
            | '\u{2026}'
            | '\u{22ef}'
            | '\u{00b7}'
            | '-'
            | '\u{2010}'
            | '\u{2011}'
            | '\u{2012}'
            | '\u{2013}'
            | '\u{2014}'
            | '\u{fe4d}'
            | '\u{fe4e}'
            | '\u{fe4f}'
            | '\u{ff3f}'
    )
}

fn inline_blank_marker_width(ch: char) -> usize {
    if matches!(ch, '\u{2026}' | '\u{22ef}') {
        3
    } else {
        1
    }
}

fn next_non_space(text: &str, from: usize) -> Option<(usize, char)> {
    let mut cursor = from.min(text.len());
    while let Some(next) = text[cursor..].chars().next() {
        if next.is_whitespace() {
            cursor += next.len_utf8();
        } else {
            return Some((cursor, next));
        }
    }
    None
}

fn is_range_dash_after_number(text: &str, after_digits: usize) -> bool {
    let Some((dash_index, dash)) = next_non_space(text, after_digits) else {
        return false;
    };
    if !matches!(dash, '-' | '\u{2013}' | '\u{2014}') {
        return false;
    }
    next_non_space(text, dash_index + dash.len_utf8())
        .map(|(_, next)| next.is_ascii_digit())
        .unwrap_or(false)
}

fn find_inline_marker(text: &str, display: &str, from: usize) -> Option<(usize, usize)> {
    let mut search = from.min(text.len());
    while let Some(relative) = text[search..].find(display) {
        let start = search + relative;
        let mut after = start + display.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .map(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | '<' | '>'))
            .unwrap_or(true);
        if !before_ok {
            search = after;
            continue;
        }
        if is_range_dash_after_number(text, after) {
            search = after;
            continue;
        }
        let mut cursor = after;
        if let Some(next) = text[cursor..].chars().next() {
            if matches!(next, '.' | ')' | ':' | '、') {
                cursor += next.len_utf8();
            }
        }
        let whitespace_start = cursor;
        while let Some(next) = text[cursor..].chars().next() {
            if next.is_whitespace() {
                cursor += next.len_utf8();
            } else {
                break;
            }
        }
        let mut blank_end = cursor;
        let mut blank_width = 0usize;
        while let Some(next) = text[blank_end..].chars().next() {
            if is_inline_blank_marker_char(next) {
                blank_width += inline_blank_marker_width(next);
                blank_end += next.len_utf8();
            } else {
                break;
            }
        }
        if blank_width >= 3 {
            return Some((start, blank_end));
        }
        if cursor > whitespace_start {
            after = cursor;
        }
        search = after;
    }
    None
}

fn render_inline_completion_from_notes(group: &Value, questions: &[Value]) -> Option<String> {
    let notes = group_layout_notes(group).trim();
    if notes.is_empty() {
        return None;
    }
    let mut output = String::new();
    let mut cursor = 0usize;
    for question in questions {
        let Some((display, qid)) = question_display_and_id(question) else {
            continue;
        };
        let Some((start, end)) = find_inline_marker(notes, &display, cursor) else {
            continue;
        };
        output.push_str(&html_escape(&notes[cursor..start]));
        let control = if question_has_options(question) {
            format!(
                "<span class=\"choice-row\">{}</span>",
                render_option_controls(question, "radio")
            )
        } else {
            format!(
                "<input type=\"text\" id=\"{}_input\" name=\"{}\" placeholder=\"answer\">",
                html_escape(&qid),
                html_escape(&qid)
            )
        };
        output.push_str(&format!(
            "<span class=\"inline-completion\" data-question-id=\"{}\"><strong>{}</strong> {}</span>",
            html_escape(&qid),
            html_escape(&display),
            control
        ));
        cursor = end;
    }
    output.push_str(&html_escape(&notes[cursor..]));
    if output.contains("inline-completion") {
        Some(format!("<div class=\"notes-completion\">{}</div>", output))
    } else {
        None
    }
}

pub(crate) fn render_group_body_html(group: &Value) -> String {
    let group_id = string_at(group, "groupId");
    let kind = string_at(group, "kind");
    let lead = group
        .get("instruction")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|item| format!("<p>{}</p>", html_escape(item)))
                .collect::<String>()
        })
        .unwrap_or_default();
    let questions = group
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let layout_hint = group_layout_hint(group);
    let template = group_layout_template(group);
    let body = if layout_hint == "inline_completion" || template == "inline_text_completion" {
        render_inline_completion_from_notes(group, &questions).unwrap_or_else(|| {
            let rows = questions
                .iter()
                .map(|q| render_question_list_item(q, false))
                .collect::<String>();
            format!("<ol>{}</ol>", rows)
        })
    } else if kind == "table_completion" {
        let rows = questions
            .iter()
            .map(|q| {
                let qid = string_at(q, "id");
                let control = if question_has_options(q) {
                    format!(
                        "<div class=\"choice-row\">{}</div>",
                        render_option_controls(q, "radio")
                    )
                } else {
                    format!(
                        "<input type=\"text\" id=\"{}_input\" name=\"{}\" placeholder=\"answer\">",
                        html_escape(qid),
                        html_escape(qid)
                    )
                };
                format!(
                    "<tr><td><strong>{}</strong></td><td>{}</td><td>{}</td></tr>",
                    html_escape(string_at(q, "displayNumber")),
                    html_escape(string_at(q, "prompt")),
                    control
                )
            })
            .collect::<String>();
        format!("<table class=\"completion-table\"><thead><tr><th>Question</th><th>Prompt</th><th>Answer</th></tr></thead><tbody>{}</tbody></table>", rows)
    } else {
        let rows = questions
            .iter()
            .map(|q| render_question_list_item(q, kind == "multi_choice"))
            .collect::<String>();
        format!("<ol>{}</ol>", rows)
    };
    format!(
        "<section class=\"reading-question-group\" id=\"{}\"><div class=\"group-lead\">{}</div>{}</section>",
        html_escape(group_id),
        lead,
        body
    )
}

pub(crate) fn answer_key_from_authoring(authoring: &Value) -> Value {
    let mut map = serde_json::Map::new();
    if let Some(groups) = authoring.get("groups").and_then(Value::as_array) {
        for group in groups {
            if let Some(questions) = group.get("questions").and_then(Value::as_array) {
                for question in questions {
                    if let Some(qid) = question.get("id").and_then(Value::as_str) {
                        let answer = question.get("answer");
                        if !answer_is_empty(answer) {
                            map.insert(qid.to_string(), answer.cloned().unwrap_or(Value::Null));
                        }
                    }
                }
            }
        }
    }
    Value::Object(map)
}

pub(crate) fn question_order_from_authoring(authoring: &Value) -> Vec<String> {
    authoring
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| {
            group
                .get("questions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|question| {
            question
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect()
}

pub(crate) fn display_map_from_authoring(authoring: &Value) -> Value {
    let mut map = serde_json::Map::new();
    for group in authoring
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
            if let (Some(qid), Some(display)) = (
                question.get("id").and_then(Value::as_str),
                question.get("displayNumber").and_then(Value::as_str),
            ) {
                map.insert(qid.to_string(), Value::String(display.to_string()));
            }
        }
    }
    Value::Object(map)
}

fn authoring_source_file(authoring: &Value) -> Option<Value> {
    authoring
        .pointer("/exam/sourceFiles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|source| source.get("role").and_then(Value::as_str) == Some("MainQuestion"))
        .cloned()
}

fn umbrella_ranges_from_authoring(authoring: &Value) -> Vec<UmbrellaQuestionRangeV1> {
    authoring
        .pointer("/passage/questionUmbrellaRanges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| serde_json::from_value::<UmbrellaQuestionRangeV1>(item.clone()).ok())
        .collect()
}

fn question_intro_html(umbrella_ranges: &[UmbrellaQuestionRangeV1]) -> String {
    if umbrella_ranges.is_empty() {
        return "<h3>Questions</h3>".to_string();
    }
    let items = umbrella_ranges
        .iter()
        .map(|range| {
            format!(
                "<li><strong>{}</strong><span>Q{}-{}</span></li>",
                html_escape(&range.heading),
                range.question_range[0],
                range.question_range[1]
            )
        })
        .collect::<String>();
    format!(
        "<h3>Questions</h3><ul class=\"question-umbrella-ranges\">{}</ul>",
        items
    )
}

pub(crate) fn reading_source(authoring: &Value) -> Value {
    let groups = authoring
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|group| {
            let question_ids = group
                .get("questions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|q| q.get("id").and_then(Value::as_str).map(ToString::to_string))
                .collect::<Vec<_>>();
            ReadingQuestionGroupV1 {
                group_id: group
                    .get("groupId")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "group".to_string()),
                kind: group
                    .get("kind")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "short_answer".to_string()),
                question_ids,
                body_html: render_group_body_html(&group),
                lead_html: group
                    .get("instruction")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(|item| format!("<p>{}</p>", html_escape(item)))
                            .collect::<String>()
                    })
                    .unwrap_or_default(),
                allow_option_reuse: group.get("allowOptionReuse").and_then(Value::as_bool),
            }
        })
        .collect::<Vec<_>>();

    let source_file = authoring_source_file(authoring);
    let pdf_filename = source_file
        .as_ref()
        .and_then(|source| source.get("originalName"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("source.pdf");
    let stored_name = source_file
        .as_ref()
        .and_then(|source| source.get("storedName"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("source.pdf");
    let source_file_id = source_file
        .as_ref()
        .and_then(|source| source.get("fileId"))
        .and_then(Value::as_str)
        .unwrap_or("unknown-source");
    let source_sha256 = source_file
        .as_ref()
        .and_then(|source| source.get("sha256"))
        .and_then(Value::as_str)
        .unwrap_or("unknown-sha256");
    let human_verified = authoring
        .pointer("/audit/humanVerified")
        .and_then(Value::as_bool)
        == Some(true);
    let question_umbrella_ranges = umbrella_ranges_from_authoring(authoring);
    let question_intro_html = question_intro_html(&question_umbrella_ranges);

    let passage_blocks = authoring
        .pointer("/passage/htmlBlocks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| {
            vec![json!({
                "blockId":"passage-main",
                "kind":"html",
                "html":""
            })]
        })
        .into_iter()
        .map(|block| ReadingPassageBlockV1 {
            block_id: block
                .get("blockId")
                .and_then(Value::as_str)
                .unwrap_or("passage-main")
                .to_string(),
            kind: block
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("html")
                .to_string(),
            html: block
                .get("html")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .collect::<Vec<_>>();

    ReadingExamSourceV1 {
        schema_version: "ReadingExamSourceV1".to_string(),
        exam_id: authoring
            .pointer("/exam/examId")
            .and_then(Value::as_str)
            .unwrap_or("local-authoring-exam")
            .to_string(),
        meta: ReadingSourceMetaV1 {
            title: authoring
                .pointer("/exam/title")
                .and_then(Value::as_str)
                .unwrap_or("Untitled Reading")
                .to_string(),
            category: authoring
                .pointer("/exam/category")
                .and_then(Value::as_str)
                .unwrap_or("P1")
                .to_string(),
            frequency: authoring
                .pointer("/exam/frequency")
                .and_then(Value::as_str)
                .unwrap_or("medium")
                .to_string(),
            pdf_filename: pdf_filename.to_string(),
            legacy_path: String::new(),
            legacy_filename: String::new(),
            question_intro_html,
            question_umbrella_ranges,
        },
        passage: ReadingPassageV1 {
            blocks: passage_blocks,
        },
        question_groups: groups,
        answer_key: answer_key_from_authoring(authoring)
            .as_object()
            .cloned()
            .unwrap_or_default(),
        source_refs: ReadingSourceRefsV1 {
            primary_html: format!(
                "author-imports/{}/intermediate.html",
                authoring
                    .get("jobId")
                    .and_then(Value::as_str)
                    .unwrap_or("job")
            ),
            primary_provider: "author_web".to_string(),
            shui_html: None,
            shui_pdf: format!("uploads/{}", stored_name),
            ielts_html: None,
        },
        audit: ReadingSourceAuditV1 {
            match_status: if human_verified {
                "author_verified".to_string()
            } else {
                "needs_review".to_string()
            },
            match_confidence: if human_verified { 1.0 } else { 0.0 },
            verified_at: human_verified.then(|| Utc::now().to_rfc3339()),
            notes: format!(
                "provider:author_tauri;sourceFileId:{};sourceSha256:{};signature:radio,text,table",
                source_file_id, source_sha256
            ),
        },
        question_order: question_order_from_authoring(authoring),
        question_display_map: display_map_from_authoring(authoring)
            .as_object()
            .cloned()
            .unwrap_or_default(),
    }
    .to_value()
}

#[cfg(test)]
mod tests {
    use super::{reading_source, render_group_body_html};
    use serde_json::{json, Value};

    #[test]
    fn option_texts_are_displayed_without_changing_submission_values() {
        let group = json!({
            "groupId": "g1",
            "kind": "single_choice",
            "instruction": [],
            "questions": [{
                "id": "q1",
                "displayNumber": "1",
                "prompt": "Choose one.",
                "interaction": {
                    "type": "radio",
                    "options": ["A", "B"],
                    "optionTexts": {
                        "A": "First & best",
                        "B": "Second <choice>"
                    }
                }
            }]
        });

        let html = render_group_body_html(&group);

        assert!(html.contains("value=\"A\""));
        assert!(html.contains("value=\"B\""));
        assert!(html.contains(
            "<span class=\"choice-label\">A</span> <span class=\"choice-text\">First &amp; best</span>"
        ));
        assert!(html.contains(
            "<span class=\"choice-label\">B</span> <span class=\"choice-text\">Second &lt;choice&gt;</span>"
        ));
        assert!(!html.contains("value=\"First &amp; best\""));
    }

    #[test]
    fn options_without_option_texts_keep_the_legacy_display() {
        let group = json!({
            "groupId": "g1",
            "kind": "single_choice",
            "instruction": [],
            "questions": [{
                "id": "q1",
                "displayNumber": "1",
                "prompt": "Choose one.",
                "interaction": {"type": "radio", "options": ["A"]}
            }]
        });

        let html = render_group_body_html(&group);

        assert!(html.contains("<input name=\"q1\" type=\"radio\" value=\"A\"> A</label>"));
        assert!(!html.contains("choice-text"));
    }

    #[test]
    fn authoring_option_banks_reach_single_choice_and_matching_list_student_html() {
        let authoring = json!({
            "schemaVersion": "ReadingAuthoringIRV1",
            "jobId": "job-option-bank",
            "exam": {
                "examId": "exam-option-bank",
                "title": "Option bank rendering",
                "category": "P1",
                "frequency": "medium",
                "tags": [],
                "sourceFiles": []
            },
            "passage": {
                "title": "Passage",
                "htmlBlocks": [{"blockId": "passage-main", "html": "<p>Passage</p>"}],
                "sourceBlockIds": []
            },
            "groups": [
                {
                    "groupId": "single-choice",
                    "kind": "single_choice",
                    "instruction": ["Choose the correct letter, A, B, C or D."],
                    "layout": {"template": "single_choice_list", "layoutHint": "list"},
                    "questions": [{
                        "id": "q1",
                        "displayNumber": "1",
                        "prompt": "Why was the archive moved?",
                        "interaction": {
                            "type": "radio",
                            "options": ["A", "B", "C", "D"],
                            "optionTexts": {
                                "A": "To reduce staffing",
                                "B": "To provide more space",
                                "C": "To protect rare maps",
                                "D": "To improve public transport"
                            }
                        }
                    }]
                },
                {
                    "groupId": "matching-list",
                    "kind": "matching",
                    "instruction": ["Complete the list using the correct letter, A, B, C or D."],
                    "layout": {"template": "matching_list", "layoutHint": "list"},
                    "allowOptionReuse": false,
                    "questions": [{
                        "id": "q2",
                        "displayNumber": "2",
                        "prompt": "Research location",
                        "interaction": {
                            "type": "matching",
                            "options": ["A", "B", "C", "D"],
                            "optionTexts": {
                                "A": "University at Albany",
                                "B": "University of Leeds",
                                "C": "University of London",
                                "D": "University of Oxford"
                            }
                        }
                    }]
                }
            ],
            "answerKey": {},
            "questionOrder": ["q1", "q2"],
            "questionDisplayMap": {"q1": "1", "q2": "2"},
            "audit": {
                "llmUsed": false,
                "humanVerified": false,
                "issues": [],
                "revision": 1,
                "updatedAt": "2026-07-15T00:00:00Z"
            }
        });

        let source = reading_source(&authoring);
        let cases = [
            (
                "/questionGroups/0/bodyHtml",
                "q1",
                [
                    ("A", "To reduce staffing"),
                    ("B", "To provide more space"),
                    ("C", "To protect rare maps"),
                    ("D", "To improve public transport"),
                ],
            ),
            (
                "/questionGroups/1/bodyHtml",
                "q2",
                [
                    ("A", "University at Albany"),
                    ("B", "University of Leeds"),
                    ("C", "University of London"),
                    ("D", "University of Oxford"),
                ],
            ),
        ];

        for (pointer, question_id, options) in cases {
            let html = source.pointer(pointer).and_then(Value::as_str).unwrap();
            for (label, text) in options {
                assert!(
                    html.contains(&format!(
                        "<input name=\"{}\" type=\"radio\" value=\"{}\">",
                        question_id, label
                    )),
                    "missing label submission value {label} in {pointer}: {html}"
                );
                assert!(
                    html.contains(&format!(
                        "<span class=\"choice-label\">{}</span> <span class=\"choice-text\">{}</span>",
                        label, text
                    )),
                    "missing rendered option text for {label} in {pointer}: {html}"
                );
                assert!(
                    !html.contains(&format!("value=\"{}\"", text)),
                    "option text became a submission value in {pointer}: {html}"
                );
            }
        }
    }

    #[test]
    fn completion_option_banks_override_text_layouts_without_changing_free_text() {
        let authoring = json!({
            "groups": [
                {
                    "groupId": "summary-bank",
                    "kind": "summary_completion",
                    "instruction": ["Choose the correct letter, A-H."],
                    "layout": {
                        "template": "summary_text_completion",
                        "layoutHint": "inline_completion",
                        "notes": "The final result was 36 _______."
                    },
                    "questions": [{
                        "id": "q36",
                        "displayNumber": "36",
                        "prompt": "The final result was",
                        "interaction": {
                            "type": "matching",
                            "options": ["A", "B", "C", "D", "E", "F", "G", "H"],
                            "optionTexts": {
                                "A": "natural evolution",
                                "B": "seasonal migration",
                                "C": "habitat loss",
                                "D": "water pollution",
                                "E": "commercial fishing",
                                "F": "introduced plants",
                                "G": "native fish",
                                "H": "extinction"
                            },
                            "allowOptionReuse": false
                        }
                    }]
                },
                {
                    "groupId": "summary-free-text",
                    "kind": "summary_completion",
                    "instruction": ["Write ONE WORD ONLY."],
                    "layout": {
                        "template": "summary_text_completion",
                        "layoutHint": "inline_completion",
                        "notes": "The archive contains 41 _______."
                    },
                    "questions": [{
                        "id": "q41",
                        "displayNumber": "41",
                        "prompt": "The archive contains",
                        "interaction": {"type": "text"}
                    }]
                },
                {
                    "groupId": "table-bank",
                    "kind": "table_completion",
                    "instruction": ["Choose the correct letter, A-D."],
                    "layout": {"template": "table_completion", "layoutHint": "table"},
                    "questions": [{
                        "id": "q42",
                        "displayNumber": "42",
                        "prompt": "Research location",
                        "interaction": {
                            "type": "matching",
                            "options": ["A", "B", "C", "D"],
                            "optionTexts": {
                                "A": "University at Albany",
                                "B": "University of Leeds",
                                "C": "University of London",
                                "D": "University of Oxford"
                            },
                            "allowOptionReuse": false
                        }
                    }]
                },
                {
                    "groupId": "table-free-text",
                    "kind": "table_completion",
                    "instruction": ["Write ONE WORD ONLY."],
                    "layout": {"template": "table_completion", "layoutHint": "table"},
                    "questions": [{
                        "id": "q43",
                        "displayNumber": "43",
                        "prompt": "Archive material",
                        "interaction": {"type": "text"}
                    }]
                }
            ]
        });

        let source = reading_source(&authoring);
        let summary_bank = source
            .pointer("/questionGroups/0/bodyHtml")
            .and_then(Value::as_str)
            .unwrap();
        assert!(summary_bank.contains("notes-completion"));
        for label in ["A", "B", "C", "D", "E", "F", "G", "H"] {
            assert!(summary_bank.contains(&format!(
                "<input name=\"q36\" type=\"radio\" value=\"{}\">",
                label
            )));
        }
        assert!(summary_bank.contains(
            "<span class=\"choice-label\">H</span> <span class=\"choice-text\">extinction</span>"
        ));
        assert!(!summary_bank.contains("id=\"q36_input\""));
        assert!(!summary_bank.contains("value=\"extinction\""));

        let summary_free_text = source
            .pointer("/questionGroups/1/bodyHtml")
            .and_then(Value::as_str)
            .unwrap();
        assert!(summary_free_text.contains(
            "<input type=\"text\" id=\"q41_input\" name=\"q41\" placeholder=\"answer\">"
        ));
        assert!(!summary_free_text.contains("type=\"radio\""));

        let table_bank = source
            .pointer("/questionGroups/2/bodyHtml")
            .and_then(Value::as_str)
            .unwrap();
        assert!(table_bank.contains("completion-table"));
        assert!(table_bank.contains("<input name=\"q42\" type=\"radio\" value=\"D\">"));
        assert!(table_bank.contains(
            "<span class=\"choice-label\">D</span> <span class=\"choice-text\">University of Oxford</span>"
        ));
        assert!(!table_bank.contains("id=\"q42_input\""));
        assert!(!table_bank.contains("value=\"University of Oxford\""));

        let table_free_text = source
            .pointer("/questionGroups/3/bodyHtml")
            .and_then(Value::as_str)
            .unwrap();
        assert!(table_free_text.contains(
            "<input type=\"text\" id=\"q43_input\" name=\"q43\" placeholder=\"answer\">"
        ));
        assert!(!table_free_text.contains("type=\"radio\""));
    }
}
