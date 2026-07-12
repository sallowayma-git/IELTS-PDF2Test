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
        output.push_str(&format!(
            "<span class=\"inline-completion\" data-question-id=\"{}\"><strong>{}</strong> <input type=\"text\" id=\"{}_input\" name=\"{}\" placeholder=\"answer\"></span>",
            html_escape(&qid),
            html_escape(&display),
            html_escape(&qid),
            html_escape(&qid)
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
                .map(|q| {
                    let qid = string_at(q, "id");
                    format!(
                        "<li><label><strong>{}</strong> {} <input type=\"text\" id=\"{}_input\" name=\"{}\"></label></li>",
                        html_escape(string_at(q, "displayNumber")),
                        html_escape(string_at(q, "prompt")),
                        html_escape(qid),
                        html_escape(qid)
                    )
                })
                .collect::<String>();
            format!("<ol>{}</ol>", rows)
        })
    } else if kind == "table_completion" {
        let rows = questions
            .iter()
            .map(|q| {
                let qid = string_at(q, "id");
                format!(
                    "<tr><td><strong>{}</strong></td><td>{}</td><td><input type=\"text\" id=\"{}_input\" name=\"{}\" placeholder=\"answer\"></td></tr>",
                    html_escape(string_at(q, "displayNumber")),
                    html_escape(string_at(q, "prompt")),
                    html_escape(qid),
                    html_escape(qid)
                )
            })
            .collect::<String>();
        format!("<table class=\"completion-table\"><thead><tr><th>Question</th><th>Prompt</th><th>Answer</th></tr></thead><tbody>{}</tbody></table>", rows)
    } else {
        let rows = questions
            .iter()
            .map(|q| {
                let qid = string_at(q, "id");
                let options = q
                    .pointer("/interaction/options")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if kind == "multi_choice" {
                    let controls = options
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|option| {
                            format!(
                                "<label><input name=\"{}\" type=\"checkbox\" value=\"{}\"> {}</label>",
                                html_escape(qid),
                                html_escape(option),
                                html_escape(option)
                            )
                        })
                        .collect::<String>();
                    format!(
                        "<li><div><strong>{}</strong> {}</div><div class=\"choice-row\">{}</div></li>",
                        html_escape(string_at(q, "displayNumber")),
                        html_escape(string_at(q, "prompt")),
                        controls
                    )
                } else if !options.is_empty() {
                    let controls = options
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|option| {
                            format!(
                                "<label><input name=\"{}\" type=\"radio\" value=\"{}\"> {}</label>",
                                html_escape(qid),
                                html_escape(option),
                                html_escape(option)
                            )
                        })
                        .collect::<String>();
                    format!(
                        "<li><div><strong>{}</strong> {}</div><div class=\"choice-row\">{}</div></li>",
                        html_escape(string_at(q, "displayNumber")),
                        html_escape(string_at(q, "prompt")),
                        controls
                    )
                } else {
                    format!(
                        "<li><label><strong>{}</strong> {} <input type=\"text\" id=\"{}_input\" name=\"{}\"></label></li>",
                        html_escape(string_at(q, "displayNumber")),
                        html_escape(string_at(q, "prompt")),
                        html_escape(qid),
                        html_escape(qid)
                    )
                }
            })
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
