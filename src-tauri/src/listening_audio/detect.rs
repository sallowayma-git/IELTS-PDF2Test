//! Import-time modality hint: does this source look like an IELTS Listening paper?
//!
//! The hint only decides whether the import flow asks the user to confirm Listening and
//! provide audio. It never sets a publishable modality on its own: the user confirms (or
//! switches back to Reading) in the dialog, and that confirmed value is what `import_files`
//! persists.
//!
//! Evidence is the first page of a PDF or the first paragraphs of a DOCX. Real listening
//! PDFs often come out of the text layer letter-spaced (`L i s t e n i n g`), so cues are
//! matched on a whitespace-free, lower-cased view. A single word "listening" is not enough:
//! Reading passages are titled "Listening to the Ocean". At least two independent listening
//! cues are required, and any Reading cue wins.

use std::io::Read;
use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModalityDetection {
    pub path: String,
    /// `listening` | `reading` | `unknown`
    pub modality: String,
    pub cues: Vec<String>,
}

const LISTENING_CUES: &[(&str, &[&str])] = &[
    ("listening_heading", &["listening"]),
    (
        "while_you_listen",
        &["whileyouarelistening", "asyoulisten", "whileyoulisten"],
    ),
    (
        "you_will_hear",
        &[
            "youwillhear",
            "youwillnowhear",
            "youhearsome",
            "youwillbegiven",
        ],
    ),
    (
        "four_parts",
        &["fourparts", "4parts", "foursections", "4sections"],
    ),
    (
        "recording",
        &["therecording", "recordingsonce", "hearthe", "heareach"],
    ),
    (
        "transfer_answers",
        &["transferyouranswers", "totransferyour"],
    ),
    (
        "section_questions",
        &["section1questions", "part1questions", "section1question"],
    ),
];

const READING_CUES: &[(&str, &[&str])] = &[
    ("reading_passage", &["readingpassage"]),
    (
        "spend_twenty_minutes",
        &["youshouldspendabout20minutes", "spendabout20minutes"],
    ),
    (
        "read_the_text",
        &[
            "readthetext",
            "readthepassage",
            "passage1below",
            "readingpassage1",
        ],
    ),
];

fn squash(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn matched(squashed: &str, table: &[(&str, &[&str])]) -> Vec<String> {
    table
        .iter()
        .filter(|(_, needles)| needles.iter().any(|needle| squashed.contains(needle)))
        .map(|(cue, _)| (*cue).to_string())
        .collect()
}

/// Classifies first-page text. Returns (`listening` | `reading` | `unknown`, cues).
pub(crate) fn detect_text_modality(text: &str) -> (String, Vec<String>) {
    let squashed = squash(text);
    if squashed.is_empty() {
        return ("unknown".to_string(), vec!["no_text".to_string()]);
    }
    let listening = matched(&squashed, LISTENING_CUES);
    let reading = matched(&squashed, READING_CUES);
    let mut cues: Vec<String> = listening
        .iter()
        .map(|cue| format!("listening:{cue}"))
        .collect();
    cues.extend(reading.iter().map(|cue| format!("reading:{cue}")));
    let modality = if !reading.is_empty() {
        "reading"
    } else if listening.len() >= 2 {
        "listening"
    } else if listening.len() == 1 {
        "unknown"
    } else {
        "reading"
    };
    (modality.to_string(), cues)
}

fn first_page_pdf_text(path: &Path) -> Option<String> {
    let pages = pdf_extract::extract_text_by_pages(path).ok()?;
    pages.into_iter().find(|page| !page.trim().is_empty())
}

/// First ~40 paragraphs of `word/document.xml`, enough for a cover page and instructions.
fn first_paragraphs_docx_text(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name("word/document.xml").ok()?;
    let mut xml = String::new();
    entry.read_to_string(&mut xml).ok()?;
    let mut out = String::new();
    let mut paragraphs = 0usize;
    for chunk in xml.split("</w:p>") {
        let mut rest = chunk;
        while let Some(start) = rest.find("<w:t") {
            let after = &rest[start..];
            let Some(open_end) = after.find('>') else {
                break;
            };
            let body = &after[open_end + 1..];
            let Some(close) = body.find("</w:t>") else {
                break;
            };
            if !after[..open_end].ends_with('/') {
                out.push_str(&body[..close]);
            }
            rest = &body[close..];
        }
        out.push('\n');
        paragraphs += 1;
        if paragraphs >= 40 {
            break;
        }
    }
    Some(out)
}

pub(crate) fn detect_file_modality(path: &str) -> ModalityDetection {
    let lower = path.to_ascii_lowercase();
    let text = if lower.ends_with(".pdf") {
        first_page_pdf_text(Path::new(path))
    } else if lower.ends_with(".docx") {
        first_paragraphs_docx_text(Path::new(path))
    } else if lower.ends_with(".txt") || lower.ends_with(".md") {
        std::fs::read_to_string(path)
            .ok()
            .map(|text| text.chars().take(4000).collect())
    } else {
        None
    };
    let (modality, cues) = match text {
        Some(text) => detect_text_modality(&text),
        None => ("unknown".to_string(), vec!["unreadable".to_string()]),
    };
    ModalityDetection {
        path: path.to_string(),
        modality,
        cues,
    }
}

pub(crate) fn detect_import_modality(paths: &[String]) -> Vec<ModalityDetection> {
    paths
        .iter()
        .map(|path| detect_file_modality(path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letter_spaced_listening_cover_is_listening() {
        let text = "L I S T E N I N G\nYou will hear a number of different recordings and you will have to answer questions on what you hear.\nWhile you are listening, write your answers on the question paper.\nThere are four parts to the test.";
        let (modality, cues) = detect_text_modality(text);
        assert_eq!(modality, "listening", "{cues:?}");
        assert!(cues.iter().any(|cue| cue == "listening:while_you_listen"));
        assert!(cues.iter().any(|cue| cue == "listening:four_parts"));
    }

    #[test]
    fn reading_passage_titled_listening_to_the_ocean_is_reading() {
        let text = "READING PASSAGE 1\nYou should spend about 20 minutes on Questions 1-13, which are based on Reading Passage 1 below.\nListening to the Ocean\nThe results of some recent research answer some long-standing questions. Scientists hear the recording of whale calls.";
        let (modality, cues) = detect_text_modality(text);
        assert_eq!(modality, "reading", "{cues:?}");
        assert!(cues.iter().any(|cue| cue.starts_with("reading:")));
    }

    #[test]
    fn a_lone_listening_word_is_not_enough() {
        let (modality, _) = detect_text_modality(
            "Listening to the Ocean\nThe oceans cover more than 70 per cent of the planet.",
        );
        assert_eq!(modality, "unknown");
        let (modality, _) = detect_text_modality("The history of glass");
        assert_eq!(modality, "reading");
        let (modality, cues) = detect_text_modality("   ");
        assert_eq!(
            (modality.as_str(), cues),
            ("unknown", vec!["no_text".to_string()])
        );
    }

    #[test]
    fn unreadable_file_is_unknown() {
        let detection = detect_file_modality("Z:/definitely/missing.pdf");
        assert_eq!(detection.modality, "unknown");
    }

    #[test]
    fn real_listening_fixture_first_page_is_listening_when_present() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/golden/private-real/listening-vol7-t9.pdf");
        if !path.exists() {
            eprintln!("skipped: private fixture absent");
            return;
        }
        let detection = detect_file_modality(&path.to_string_lossy());
        assert_eq!(detection.modality, "listening", "{:?}", detection.cues);
    }
}
