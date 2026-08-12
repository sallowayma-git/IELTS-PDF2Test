use crate::schema::common::SourceAnchorV2;
use crate::schema::ielts_authoring_v2::{
    AssignmentV2, CardinalityV2, InstructionSignatureV2, TaskTypeV2, WordLimitV2,
};
use serde_json::Value;

use super::instruction_zone::normalize_instruction_text;
use super::question_number::expand_expression;
use crate::schema::ielts_authoring_v2::QuestionNumberExpressionV2;

#[derive(Debug, Clone)]
pub(crate) struct SignatureResult {
    pub signature: InstructionSignatureV2,
    pub warnings: Vec<String>,
}

pub(crate) fn infer_instruction_signature(
    text: &str,
    expression: &QuestionNumberExpressionV2,
    kind_hint: Option<&str>,
    evidence_anchors: Vec<Value>,
) -> SignatureResult {
    let normalized_text = normalize_instruction_text(text);
    let lower = normalized_text.to_ascii_lowercase();
    let expected_question_numbers = expand_expression(expression);
    let task_type = infer_task_type(&lower, kind_hint);
    let selection_cardinality = selection_cardinality(&lower);
    let option_alphabet = infer_option_alphabet(&lower);
    let word_limit = parse_word_limit(&lower);
    let allow_option_reuse = parse_reuse_policy(&lower, &task_type);
    let answer_assignment = if selection_cardinality
        .as_ref()
        .and_then(|cardinality| cardinality.exact)
        .is_some_and(|count| count > 1)
    {
        Some(AssignmentV2::UnorderedSet)
    } else if matches!(
        task_type,
        TaskTypeV2::MatchingInformation
            | TaskTypeV2::MatchingHeadings
            | TaskTypeV2::MatchingFeatures
            | TaskTypeV2::MatchingSentenceEndings
            | TaskTypeV2::Classification
    ) {
        Some(AssignmentV2::PerSlot)
    } else {
        Some(AssignmentV2::PerSlot)
    };

    let mut warnings = Vec::new();
    if expected_question_numbers.is_empty() {
        warnings.push("instruction_signature_has_no_expected_questions".to_string());
    }
    if !has_strong_task_cue(&lower, &task_type) {
        warnings.push("instruction_signature_weak_task_cue".to_string());
    }
    if is_completion_task(&task_type) && word_limit.is_none() {
        warnings.push("completion_word_limit_not_found".to_string());
    }
    let confidence = signature_confidence(&lower, &task_type, &warnings);
    let evidence_anchors = evidence_anchors
        .into_iter()
        .filter_map(|anchor| serde_json::from_value::<SourceAnchorV2>(anchor).ok())
        .collect();
    SignatureResult {
        signature: InstructionSignatureV2 {
            normalized_text,
            task_type,
            expected_question_numbers: expected_question_numbers.clone(),
            expected_slot_count: expected_question_numbers.len() as u32,
            option_alphabet,
            selection_cardinality,
            answer_assignment,
            allow_option_reuse,
            word_limit,
            evidence_anchors,
            confidence,
        },
        warnings,
    }
}

pub(crate) fn task_type_label(task_type: &TaskTypeV2) -> &'static str {
    match task_type {
        TaskTypeV2::SingleChoice => "single_choice",
        TaskTypeV2::MultipleChoice => "multiple_choice",
        TaskTypeV2::TrueFalseNotGiven => "true_false_not_given",
        TaskTypeV2::YesNoNotGiven => "yes_no_not_given",
        TaskTypeV2::MatchingInformation => "matching_information",
        TaskTypeV2::MatchingHeadings => "matching_headings",
        TaskTypeV2::MatchingFeatures => "matching_features",
        TaskTypeV2::MatchingSentenceEndings => "matching_sentence_endings",
        TaskTypeV2::Classification => "classification",
        TaskTypeV2::SentenceCompletion => "sentence_completion",
        TaskTypeV2::SummaryCompletion => "summary_completion",
        TaskTypeV2::NoteCompletion => "note_completion",
        TaskTypeV2::TableCompletion => "table_completion",
        TaskTypeV2::FormCompletion => "form_completion",
        TaskTypeV2::FlowchartCompletion => "flowchart_completion",
        TaskTypeV2::DiagramLabelCompletion => "diagram_label_completion",
        TaskTypeV2::PlanMapLabelCompletion => "plan_map_label_completion",
        TaskTypeV2::ShortAnswer => "short_answer",
    }
}

pub(crate) fn is_completion_task(task_type: &TaskTypeV2) -> bool {
    matches!(
        task_type,
        TaskTypeV2::SentenceCompletion
            | TaskTypeV2::SummaryCompletion
            | TaskTypeV2::NoteCompletion
            | TaskTypeV2::TableCompletion
            | TaskTypeV2::FormCompletion
            | TaskTypeV2::FlowchartCompletion
            | TaskTypeV2::DiagramLabelCompletion
            | TaskTypeV2::PlanMapLabelCompletion
    )
}

fn infer_task_type(lower: &str, kind_hint: Option<&str>) -> TaskTypeV2 {
    if lower.contains("true") && lower.contains("false") && lower.contains("not given") {
        return TaskTypeV2::TrueFalseNotGiven;
    }
    if lower.contains("yes") && lower.contains("no") && lower.contains("not given") {
        return TaskTypeV2::YesNoNotGiven;
    }
    if lower.contains("list of headings") || lower.contains("correct heading for each paragraph") {
        return TaskTypeV2::MatchingHeadings;
    }
    if lower.contains("list of people")
        || lower.contains("list of features")
        || lower.contains("list of categories")
        || (lower.contains("match each statement") && lower.contains("list of"))
    {
        return TaskTypeV2::MatchingFeatures;
    }
    if lower.contains("sentence endings") || lower.contains("endings") && lower.contains("match") {
        return TaskTypeV2::MatchingSentenceEndings;
    }
    if lower.contains("which paragraph")
        || lower.contains("which section")
        || lower.contains("match each statement with")
    {
        return TaskTypeV2::MatchingInformation;
    }
    if lower.contains("complete the table") || lower.contains("complete the table below") {
        return TaskTypeV2::TableCompletion;
    }
    if lower.contains("complete the form") {
        return TaskTypeV2::FormCompletion;
    }
    if lower.contains("complete the flow") || lower.contains("flow-chart") {
        return TaskTypeV2::FlowchartCompletion;
    }
    if lower.contains("summary") && lower.contains("complete") {
        return TaskTypeV2::SummaryCompletion;
    }
    if (lower.contains("note") || lower.contains("notes")) && lower.contains("complete") {
        return TaskTypeV2::NoteCompletion;
    }
    if lower.contains("complete the sentences") || lower.contains("complete each sentence") {
        return TaskTypeV2::SentenceCompletion;
    }
    if lower.contains("diagram") && (lower.contains("label") || lower.contains("complete")) {
        return TaskTypeV2::DiagramLabelCompletion;
    }
    let has_map_or_plan_word = lower
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .any(|word| matches!(word, "map" | "plan"));
    if has_map_or_plan_word && (lower.contains("label") || lower.contains("complete")) {
        return TaskTypeV2::PlanMapLabelCompletion;
    }
    if lower.contains("choose")
        && (lower.contains("two") || lower.contains("three"))
        && (lower.contains("letter") || lower.contains("option"))
    {
        return TaskTypeV2::MultipleChoice;
    }
    if lower.contains("choose the correct letter")
        || lower.contains("choose the correct answer")
        || lower.contains("select the correct")
    {
        return TaskTypeV2::SingleChoice;
    }
    if lower.contains("match") || lower.contains("matching") {
        return TaskTypeV2::MatchingInformation;
    }
    match kind_hint.unwrap_or_default().to_ascii_lowercase().as_str() {
        "true_false_not_given" => TaskTypeV2::TrueFalseNotGiven,
        "yes_no_not_given" => TaskTypeV2::YesNoNotGiven,
        "single_choice" => TaskTypeV2::SingleChoice,
        "multi_choice" | "multiple_choice" => TaskTypeV2::MultipleChoice,
        "heading_matching" | "matching_headings" => TaskTypeV2::MatchingHeadings,
        "classification" | "matching_features" => TaskTypeV2::MatchingFeatures,
        "table_completion" => TaskTypeV2::TableCompletion,
        "summary_completion" => TaskTypeV2::SummaryCompletion,
        "note_completion" => TaskTypeV2::NoteCompletion,
        "diagram_completion" => TaskTypeV2::DiagramLabelCompletion,
        "flowchart_completion" => TaskTypeV2::FlowchartCompletion,
        "sentence_completion" => TaskTypeV2::SentenceCompletion,
        _ => TaskTypeV2::ShortAnswer,
    }
}

fn selection_cardinality(lower: &str) -> Option<CardinalityV2> {
    let count = [
        ("one", 1),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("six", 6),
    ]
    .iter()
    .find_map(|(word, number)| {
        let marker = format!("choose {word}");
        if lower.contains(&marker)
            || lower.contains(&format!("what {word}"))
            || lower.contains(&format!("which {word}"))
            || lower.contains(&format!("{word} answers"))
            || lower.contains(&format!("{word} letters"))
        {
            Some(*number)
        } else {
            None
        }
    });
    count.map(|exact| CardinalityV2 {
        min: exact,
        max: exact,
        exact: Some(exact),
    })
}

fn infer_option_alphabet(lower: &str) -> Option<String> {
    if lower.contains("roman") || lower.contains("list of headings") {
        return Some("roman".to_string());
    }
    if lower.contains("a, b, c, or d")
        || lower.contains("a, b, c or d")
        || lower.contains("a, b, c, and d")
        || lower.contains("a, b, c and d")
    {
        return Some("A-D".to_string());
    }
    for (start, end) in [('a', 'd'), ('a', 'e'), ('a', 'i'), ('a', 'g')] {
        let compact = format!("{start}-{end}");
        if lower.contains(&compact)
            || lower.contains(&format!("{} to {}", start, end))
            || lower.contains(&format!("{}–{}", start, end))
        {
            return Some(format!(
                "{}-{}",
                start.to_ascii_uppercase(),
                end.to_ascii_uppercase()
            ));
        }
    }
    if lower.contains("paragraphs") || lower.contains("sections") {
        return Some("paragraph_letters".to_string());
    }
    None
}

fn parse_reuse_policy(lower: &str, task_type: &TaskTypeV2) -> Option<bool> {
    if lower.contains("more than once")
        || lower.contains("may be used any number of times")
        || lower.contains("can be used more than once")
    {
        return Some(true);
    }
    if lower.contains("once only")
        || lower.contains("only once")
        || lower.contains("do not use any letter more than once")
    {
        return Some(false);
    }
    if matches!(
        task_type,
        TaskTypeV2::TrueFalseNotGiven | TaskTypeV2::YesNoNotGiven
    ) {
        return Some(true);
    }
    matches!(
        task_type,
        TaskTypeV2::MatchingHeadings | TaskTypeV2::MatchingSentenceEndings
    )
    .then_some(false)
}

fn parse_word_limit(lower: &str) -> Option<WordLimitV2> {
    let max_words = if lower.contains("one word") {
        Some(1)
    } else if lower.contains("two words") {
        Some(2)
    } else if lower.contains("three words") {
        Some(3)
    } else if lower.contains("four words") {
        Some(4)
    } else {
        None
    };
    let max_numbers = lower.contains("a number").then_some(1);
    if max_words.is_none() && max_numbers.is_none() {
        return None;
    }
    Some(WordLimitV2 {
        max_words,
        max_numbers,
        words_and_or_number: (lower.contains("and/or") || lower.contains("and or")).then_some(true),
    })
}

fn has_strong_task_cue(lower: &str, task_type: &TaskTypeV2) -> bool {
    match task_type {
        TaskTypeV2::TrueFalseNotGiven => lower.contains("agree") || lower.contains("statements"),
        TaskTypeV2::YesNoNotGiven => lower.contains("views") || lower.contains("claims"),
        TaskTypeV2::MatchingInformation => {
            lower.contains("which paragraph")
                || lower.contains("which section")
                || lower.contains("match")
        }
        TaskTypeV2::MatchingHeadings
        | TaskTypeV2::MatchingFeatures
        | TaskTypeV2::MatchingSentenceEndings => {
            lower.contains("match") || lower.contains("heading")
        }
        TaskTypeV2::SingleChoice | TaskTypeV2::MultipleChoice => lower.contains("choose"),
        _ => lower.contains("complete") || lower.contains("answer") || lower.contains("write"),
    }
}

fn signature_confidence(lower: &str, task_type: &TaskTypeV2, warnings: &[String]) -> f64 {
    let strong = has_strong_task_cue(lower, task_type);
    let base = if strong { 0.94 } else { 0.72 };
    (base - warnings.len() as f64 * 0.08).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ielts_authoring_v2::QuestionNumberExpressionV2;

    fn range() -> QuestionNumberExpressionV2 {
        QuestionNumberExpressionV2::Range { start: 1, end: 3 }
    }

    #[test]
    fn which_paragraph_and_section_are_strong_matching_information_cues() {
        for text in [
            "Questions 5-8 Which paragraph contains the following information? Write A-G.",
            "Questions 14-19 Which section contains the following information? Write A-F.",
        ] {
            let result = infer_instruction_signature(text, &range(), None, Vec::new());
            assert_eq!(result.signature.task_type, TaskTypeV2::MatchingInformation);
            assert!(result.signature.confidence >= 0.9, "{result:?}");
            assert!(result.warnings.is_empty(), "{result:?}");
        }
    }

    #[test]
    fn recognizes_tfng_and_ynng_as_distinct_signatures() {
        let tfng = infer_instruction_signature(
            "Do the following statements agree with the information given? TRUE FALSE NOT GIVEN",
            &range(),
            None,
            Vec::new(),
        );
        assert_eq!(tfng.signature.task_type, TaskTypeV2::TrueFalseNotGiven);
        let ynng = infer_instruction_signature(
            "Do the following statements agree with the views of the writer? YES NO NOT GIVEN",
            &range(),
            None,
            Vec::new(),
        );
        assert_eq!(ynng.signature.task_type, TaskTypeV2::YesNoNotGiven);
    }

    #[test]
    fn extracts_choose_two_and_word_limit_reuse_policy() {
        let result = infer_instruction_signature(
            "Choose TWO letters, A-E. You may use any letter more than once.",
            &range(),
            None,
            Vec::new(),
        );
        assert_eq!(result.signature.task_type, TaskTypeV2::MultipleChoice);
        assert_eq!(
            result.signature.selection_cardinality.unwrap().exact,
            Some(2)
        );
        assert_eq!(result.signature.option_alphabet.as_deref(), Some("A-E"));
        assert_eq!(result.signature.allow_option_reuse, Some(true));

        let completion = infer_instruction_signature(
            "Complete the notes below. NO MORE THAN TWO WORDS AND/OR A NUMBER.",
            &range(),
            Some("note_completion"),
            Vec::new(),
        );
        assert_eq!(completion.signature.task_type, TaskTypeV2::NoteCompletion);
        assert_eq!(completion.signature.word_limit.unwrap().max_words, Some(2));
    }

    #[test]
    fn does_not_classify_according_to_as_single_choice() {
        let result = infer_instruction_signature(
            "According to the passage, complete the sentence.",
            &range(),
            None,
            Vec::new(),
        );
        assert_ne!(result.signature.task_type, TaskTypeV2::SingleChoice);
    }

    #[test]
    fn note_completion_wins_over_incidental_plan_and_choose_word_limit_text() {
        let chili = infer_instruction_signature(
            "Complete the notes below. Choose ONE WORD ONLY. Unlike many other plants, chilies contain capsaicin.",
            &range(),
            Some("sentence_completion"),
            Vec::new(),
        );
        assert_eq!(chili.signature.task_type, TaskTypeV2::NoteCompletion);
        assert_eq!(chili.signature.word_limit.unwrap().max_words, Some(1));

        let fishbourne = infer_instruction_signature(
            "Complete the notes below. Choose NO MORE THAN TWO WORDS AND/OR A NUMBER from the passage.",
            &range(),
            Some("sentence_completion"),
            Vec::new(),
        );
        assert_eq!(fishbourne.signature.task_type, TaskTypeV2::NoteCompletion);
        assert_eq!(fishbourne.signature.word_limit.unwrap().max_words, Some(2));
    }

    #[test]
    fn statement_to_named_period_list_is_matching_features() {
        let result = infer_instruction_signature(
            "Look at the statements and the list of historical periods below. Match each statement with the correct historical period, A, B, C, or D.",
            &range(),
            Some("matching"),
            Vec::new(),
        );
        assert_eq!(result.signature.task_type, TaskTypeV2::MatchingFeatures);
        assert_eq!(result.signature.option_alphabet.as_deref(), Some("A-D"));
    }
}
