use crate::schema::common::SourceAnchorV2;
use crate::schema::ielts_authoring_v2::{
    AssignmentV2, CardinalityV2, InstructionSignatureV2, TaskTypeV2, WordLimitV2,
};
use serde_json::Value;

use super::instruction_zone::normalize_instruction_text;
use super::question_number::{expand_expression, instruction_cue_text};
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
    // Cue matching reads a derived view in which compact instruction phrases
    // (`Completetheformbelow`) are re-spaced; `normalized_text` keeps the source.
    let lower = instruction_cue_text(&normalized_text);
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
    if is_completion_task(&task_type) && word_limit.is_none() && option_alphabet.is_none() {
        warnings.push("completion_word_limit_not_found".to_string());
    }
    // `short_answer` is the V1.5 classifier's **fallback**, not a positive
    // finding: it means "no specific structure signal was recognised". Reading it
    // as a competing claim made the gate fire on the classifier's ignorance —
    // `Choose FOUR correct answers, A-F` came back as
    // `task_type_conflict:instruction=multiple_choice;structure_hint=short_answer`
    // and the quality gate blocked the whole paper with `TASK_TYPE_CONFLICT`.
    // Only a hint that names a concrete structure counts as evidence.
    let structure_hint = kind_hint.filter(|hint| !hint.eq_ignore_ascii_case("short_answer"));
    if let (Some(instruction_type), Some(structure_type)) = (
        infer_task_type_from_cues(&lower),
        task_type_from_kind_hint(structure_hint),
    ) {
        if !task_types_structurally_compatible(&instruction_type, &structure_type) {
            warnings.push(format!(
                "task_type_conflict:instruction={};structure_hint={}",
                task_type_label(&instruction_type),
                task_type_label(&structure_type)
            ));
        }
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

fn task_types_structurally_compatible(left: &TaskTypeV2, right: &TaskTypeV2) -> bool {
    if left == right {
        return true;
    }
    let both_completion = is_completion_task(left) && is_completion_task(right);
    let both_matching = matches!(
        left,
        TaskTypeV2::MatchingInformation
            | TaskTypeV2::MatchingHeadings
            | TaskTypeV2::MatchingFeatures
            | TaskTypeV2::MatchingSentenceEndings
            | TaskTypeV2::Classification
    ) && matches!(
        right,
        TaskTypeV2::MatchingInformation
            | TaskTypeV2::MatchingHeadings
            | TaskTypeV2::MatchingFeatures
            | TaskTypeV2::MatchingSentenceEndings
            | TaskTypeV2::Classification
    );
    both_completion || both_matching
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
    infer_task_type_from_cues(lower)
        .or_else(|| task_type_from_kind_hint(kind_hint))
        .unwrap_or(TaskTypeV2::ShortAnswer)
}

fn infer_task_type_from_cues(lower: &str) -> Option<TaskTypeV2> {
    if lower.contains("true") && lower.contains("false") && lower.contains("not given") {
        return Some(TaskTypeV2::TrueFalseNotGiven);
    }
    if lower.contains("yes") && lower.contains("no") && lower.contains("not given") {
        return Some(TaskTypeV2::YesNoNotGiven);
    }
    if lower.contains("list of headings") || lower.contains("correct heading for each paragraph") {
        return Some(TaskTypeV2::MatchingHeadings);
    }
    if lower.contains("list of people")
        || lower.contains("list of features")
        || lower.contains("list of categories")
        || (lower.contains("match each statement") && lower.contains("list of"))
    {
        return Some(TaskTypeV2::MatchingFeatures);
    }
    if lower.contains("sentence endings") || lower.contains("endings") && lower.contains("match") {
        return Some(TaskTypeV2::MatchingSentenceEndings);
    }
    if lower.contains("which paragraph")
        || lower.contains("which section")
        || lower.contains("match each statement with")
    {
        return Some(TaskTypeV2::MatchingInformation);
    }
    if lower.contains("complete the table") || lower.contains("complete the table below") {
        return Some(TaskTypeV2::TableCompletion);
    }
    if lower.contains("complete the form") {
        return Some(TaskTypeV2::FormCompletion);
    }
    if lower.contains("complete the flow") || lower.contains("flow-chart") {
        return Some(TaskTypeV2::FlowchartCompletion);
    }
    if lower.contains("summary") && lower.contains("complete") {
        return Some(TaskTypeV2::SummaryCompletion);
    }
    if (lower.contains("note") || lower.contains("notes")) && lower.contains("complete") {
        return Some(TaskTypeV2::NoteCompletion);
    }
    if lower.contains("complete the sentences") || lower.contains("complete each sentence") {
        return Some(TaskTypeV2::SentenceCompletion);
    }
    if lower.contains("diagram") && (lower.contains("label") || lower.contains("complete")) {
        return Some(TaskTypeV2::DiagramLabelCompletion);
    }
    let has_map_or_plan_word = lower
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .any(|word| matches!(word, "map" | "plan"));
    if has_map_or_plan_word && (lower.contains("label") || lower.contains("complete")) {
        return Some(TaskTypeV2::PlanMapLabelCompletion);
    }
    if lower.contains("choose")
        && (lower.contains("two") || lower.contains("three"))
        && (lower.contains("letter") || lower.contains("option"))
    {
        return Some(TaskTypeV2::MultipleChoice);
    }
    // Listening papers select four or five labels from one shared bank:
    // `Choose FOUR correct answers, A-F`, `Choose FIVE correct letters, A-G`.
    //
    // This is a **feature match against a shared bank**, not a multiple-choice
    // question: several numbered rows each take one letter from a single printed
    // box. Typing it `multiple_choice` closed the bank gate in `mod.rs`
    // (`detect_option_bank` runs only for matching-family tasks), so those rows
    // came out with an empty option list — nothing to click in the UI, plus
    // `OPTION_RUN_INCOMPLETE` and `RESPONSE_GROUP_POLICY_MISMATCH`.
    //
    // The declared letter range keeps this branch from swallowing the ordinary
    // `Choose TWO letters, A-E` cue above, which really is a multiple choice.
    if ["four", "five", "six"].iter().any(|count| {
        lower.contains(&format!("choose {count} correct"))
            || lower.contains(&format!("choose {count} letters"))
            || lower.contains(&format!("choose {count} answers"))
    }) && infer_option_alphabet(lower).is_some()
    {
        return Some(TaskTypeV2::MatchingFeatures);
    }
    if lower.contains("choose the correct letter")
        || lower.contains("choose the correct answer")
        || lower.contains("select the correct")
    {
        return Some(TaskTypeV2::SingleChoice);
    }
    if lower.contains("match") || lower.contains("matching") {
        return Some(TaskTypeV2::MatchingInformation);
    }
    None
}

fn task_type_from_kind_hint(kind_hint: Option<&str>) -> Option<TaskTypeV2> {
    Some(
        match kind_hint.unwrap_or_default().to_ascii_lowercase().as_str() {
            "true_false_not_given" => TaskTypeV2::TrueFalseNotGiven,
            "yes_no_not_given" => TaskTypeV2::YesNoNotGiven,
            "single_choice" => TaskTypeV2::SingleChoice,
            "multi_choice" | "multiple_choice" => TaskTypeV2::MultipleChoice,
            "heading_matching" | "matching_headings" => TaskTypeV2::MatchingHeadings,
            "matching_information" => TaskTypeV2::MatchingInformation,
            "matching_features" => TaskTypeV2::MatchingFeatures,
            "classification" => TaskTypeV2::Classification,
            "table_completion" => TaskTypeV2::TableCompletion,
            "form_completion" => TaskTypeV2::FormCompletion,
            "summary_completion" => TaskTypeV2::SummaryCompletion,
            "note_completion" => TaskTypeV2::NoteCompletion,
            "diagram_completion" => TaskTypeV2::DiagramLabelCompletion,
            "flowchart_completion" => TaskTypeV2::FlowchartCompletion,
            "sentence_completion" => TaskTypeV2::SentenceCompletion,
            "short_answer" => TaskTypeV2::ShortAnswer,
            _ => return None,
        },
    )
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
    // 单字母区间。这张表要与 `authoring_pipeline.rs` 的选项库标签推导对齐——那边
    // （`dynamic_declared_option_bank_labels`）覆盖 a-e … a-j，而这里以前只有
    // a-d / a-e / a-i / a-g，**缺了 a-f 与 a-h**。两处回答的是同一个问题
    // （「题干声明了哪一段字母」），认不出来的一侧会直接变成发布阻断。
    //
    // 真实链路验收里的一份 summary completion，题干原文是
    //   "Complete the summary using the list of words and phrases, A-H, below.
    //    Write the correct letter, A-H, in boxes 27-31 on your answer sheet."
    // `normalizedText` 里确实含 `a-h`，但表里没有这一项，于是：
    //   optionAlphabet = None → 该题组被判成「非选择型」→ 题干里又没有 word limit
    //   → WORD_LIMIT_UNPARSED（blocking）。`quality.rs:1566` 只认 optionAlphabet
    //   或 wordLimit，而这两个字段**界面上都没有入口**，所以那份卷子无论用户
    //   怎么操作都发布不出去。
    //
    // 方向仍然是「认不出来就不认」——只有题干真的写出该区间才返回。
    let alphabet_for = |start: char, end: char| -> Option<String> {
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
        None
    };

    // ① **原有**四种区间：保持在 `paragraphs`/`sections` 分支**之前**，
    //    与改动前逐字相同 ⇒ 任何既有输入的返回值都不变。
    //
    //    为什么这很重要（为什么不能把它们一起挪到分支之后）：真实 IELTS 高频写法
    //      "The reading passage has seven paragraphs, A-G. Write the correct letter A-G."
    //    同时含 `paragraphs` 与 `A-G`。若区间判在分支之后，它就会返回
    //    `paragraph_letters`；而 `expected_option_labels` 对没有 `-` 的名字返回
    //    `None` ⇒ 约束消失 ⇒ 真实的标签错配不再被 `OPTION_ALPHABET_MISMATCH` 拦下。
    //    那是**静默移除一条既有阻塞检查**，与本轮的目标相反。
    for (start, end) in [('a', 'd'), ('a', 'e'), ('a', 'i'), ('a', 'g')] {
        if let Some(alphabet) = alphabet_for(start, end) {
            return Some(alphabet);
        }
    }
    if lower.contains("paragraphs") || lower.contains("sections") {
        return Some("paragraph_letters".to_string());
    }
    // ② **本轮新增**的三种区间（a-f / a-h / a-j）放在分支**之后**。
    //    这样"新增严格只影响原本返回 `None` 的输入"：含 `paragraphs`/`sections` 的
    //    题干仍然走 `paragraph_letters`（它同样是 `Some`、非 null ⇒ 仍被判为选择型，
    //    `WORD_LIMIT_UNPARSED` 不会误报），也不会因此新增 `OPTION_ALPHABET_MISMATCH`
    //    约束。两条目标因此不冲突。
    for (start, end) in [('a', 'f'), ('a', 'h'), ('a', 'j')] {
        if let Some(alphabet) = alphabet_for(start, end) {
            return Some(alphabet);
        }
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
        // `option_alphabet` 断言是**补上的**：这条用例原来只看 task_type / confidence /
        // warnings，所以 `option_alphabet` 的行为怎么变它都是绿的 —— 正是这次
        // 「a-f / a-h / a-j 增补是否改了既有输入」的问题被藏住的原因。
        //
        // 现在两句都是**单数** `paragraph` / `section`（不是复数），因此都走不到
        // `paragraph_letters` 那个分支，仍然由字母区间作答：
        //   A-G ∈ 原有四种；A-F ∈ 本轮新增三种。
        for (text, expected_alphabet) in [
            (
                "Questions 5-8 Which paragraph contains the following information? Write A-G.",
                "A-G",
            ),
            (
                "Questions 14-19 Which section contains the following information? Write A-F.",
                "A-F",
            ),
        ] {
            let result = infer_instruction_signature(text, &range(), None, Vec::new());
            assert_eq!(result.signature.task_type, TaskTypeV2::MatchingInformation);
            assert!(result.signature.confidence >= 0.9, "{result:?}");
            assert!(result.warnings.is_empty(), "{result:?}");
            assert_eq!(
                result.signature.option_alphabet.as_deref(),
                Some(expected_alphabet),
                "text={text:?}"
            );
        }
    }

    /// 拆表的**分界点**：`paragraphs`（复数）与字母区间同时出现时，谁说话。
    ///
    /// 这条是 `single_letter_ranges_cover_the_span_the_pipeline_already_supports`
    /// 的对照用例 —— 那条里所有题干都不含 `paragraphs`/`sections`，所以它证明不了
    /// 「分支与区间循环的相对顺序」。这里把两种情形并排放：
    ///
    /// | 题干 | 期望 | 理由 |
    /// |---|---|---|
    /// | `…seven paragraphs, A-G…` | `Some("A-G")` | a-g 属**原有**四种，判在分支之前 ⇒ 与改动前逐字相同 |
    /// | `…six paragraphs, A-F…` | `Some("paragraph_letters")` | a-f 属**新增**三种，判在分支之后 ⇒ 不改变既有行为 |
    ///
    /// 若把整张表挪到分支之后，第一行会变成 `paragraph_letters` ⇒
    /// `expected_option_labels` 拿不到字母集 ⇒ 既有的 `OPTION_ALPHABET_MISMATCH`
    /// 检查被**静默移除**。这条用例就是用来钉住这一点的。
    #[test]
    fn paragraph_letter_cues_take_precedence_only_for_the_newly_added_ranges() {
        let established = infer_instruction_signature(
            "The reading passage has seven paragraphs, A-G. Write the correct letter A-G.",
            &range(),
            None,
            Vec::new(),
        );
        assert_eq!(
            established.signature.option_alphabet.as_deref(),
            Some("A-G"),
            "原有区间必须仍判在 paragraph_letters 分支之前"
        );

        let newly_added = infer_instruction_signature(
            "The reading passage has six paragraphs, A-F. Write the correct letter A-F.",
            &range(),
            None,
            Vec::new(),
        );
        assert_eq!(
            newly_added.signature.option_alphabet.as_deref(),
            Some("paragraph_letters"),
            "新增区间不得抢在 paragraph_letters 分支之前"
        );
        // `paragraph_letters` 仍是 `Some`/非 null ⇒ 依旧算选择型，`WORD_LIMIT_UNPARSED` 不误报。
        assert!(newly_added.signature.option_alphabet.is_some());
    }

    /// a-j 的 `to` / en-dash 变体（表格用例只覆盖了 a-h 的两种写法）。
    #[test]
    fn newly_added_ranges_accept_space_and_en_dash_spellings() {
        for text in [
            "Choose the correct letter, A to J.",
            "Choose the correct letter, A–J.",
            "Choose the correct letter, A-J.",
        ] {
            let result = infer_instruction_signature(text, &range(), None, Vec::new());
            assert_eq!(
                result.signature.option_alphabet.as_deref(),
                Some("A-J"),
                "text={text:?}"
            );
        }
        for text in [
            "Choose the correct letter, A to F.",
            "Choose the correct letter, A–F.",
        ] {
            let result = infer_instruction_signature(text, &range(), None, Vec::new());
            assert_eq!(
                result.signature.option_alphabet.as_deref(),
                Some("A-F"),
                "text={text:?}"
            );
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
    fn word_list_summary_completion_declaring_a_to_h_is_selection_type() {
        // 真实链路验收里那份稿子的题干原文（PDF `demanding-reading-passage-3.pdf`，
        // 第 27–31 题）。它声明的是 A-H 词表，答案写的是**字母**，因此属于选择型
        // completion：不该再要求 IELTS word limit。
        //
        // 改动前 `optionAlphabet` 是 None，题组被判成非选择型，于是报
        // `WORD_LIMIT_UNPARSED`（blocking）——整卷发不出去，而界面上没有任何入口
        // 能设置 `optionAlphabet` 或 `wordLimit`。这条用例钉住这个行为。
        let result = infer_instruction_signature(
            "Questions 27 - 31 Complete the summary using the list of words and phrases, A-H, below. \
             Write the correct letter, A-H, in boxes 27-31 on your answer sheet.",
            &range(),
            Some("summary_completion"),
            Vec::new(),
        );
        assert_eq!(result.signature.task_type, TaskTypeV2::SummaryCompletion);
        assert_eq!(result.signature.option_alphabet.as_deref(), Some("A-H"));
        assert!(
            !result
                .warnings
                .iter()
                .any(|warning| warning == "completion_word_limit_not_found"),
            "认出了 A-H 就不该再报找不到 word limit，warnings={:?}",
            result.warnings
        );
    }

    #[test]
    fn single_letter_ranges_cover_the_span_the_pipeline_already_supports() {
        // 表驱动：既钉住新加的 a-f / a-h，也钉住原有的 a-d / a-e / a-g / a-i
        // 没有被挤掉——`authoring_pipeline.rs` 的选项库标签推导早就覆盖
        // a-e … a-j，这里必须与之对齐，否则两侧对同一份题干给出不同结论。
        for (text, expected) in [
            ("Choose the correct letter, A-D.", "A-D"),
            ("Choose the correct letter, A-E.", "A-E"),
            ("Choose the correct letter, A-F.", "A-F"),
            ("Choose the correct letter, A-G.", "A-G"),
            ("Choose the correct letter, A-H.", "A-H"),
            ("Choose the correct letter, A-I.", "A-I"),
            ("Choose the correct letter, A-J.", "A-J"),
            // 空格与 en dash 两种写法都要认（PDF 常把连字符排成 en dash）。
            ("Choose the correct letter, A to H.", "A-H"),
            ("Choose the correct letter, A–H.", "A-H"),
        ] {
            let result = infer_instruction_signature(text, &range(), None, Vec::new());
            assert_eq!(
                result.signature.option_alphabet.as_deref(),
                Some(expected),
                "text={text:?}"
            );
        }
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

    #[test]
    fn incompatible_instruction_and_structure_hint_are_blocking_evidence() {
        let result = infer_instruction_signature(
            "Choose the correct letter, A, B or C.",
            &range(),
            Some("table_completion"),
            Vec::new(),
        );
        assert_eq!(result.signature.task_type, TaskTypeV2::SingleChoice);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.starts_with("task_type_conflict:")));
    }

    #[test]
    fn explicit_note_cue_can_refine_generic_completion_hint() {
        let result = infer_instruction_signature(
            "Complete the notes below. Write ONE WORD ONLY.",
            &range(),
            Some("sentence_completion"),
            Vec::new(),
        );
        assert_eq!(result.signature.task_type, TaskTypeV2::NoteCompletion);
        assert!(!result
            .warnings
            .iter()
            .any(|warning| warning.starts_with("task_type_conflict:")));
    }

    fn signature_shape(text: &str, expression: &QuestionNumberExpressionV2) -> String {
        let result = infer_instruction_signature(text, expression, None, Vec::new());
        let signature = result.signature;
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            signature.task_type,
            signature.expected_question_numbers,
            signature.option_alphabet,
            signature.selection_cardinality,
            signature.answer_assignment,
            signature.word_limit,
            signature.confidence,
            result.warnings
        )
    }

    #[test]
    fn compact_listening_instructions_match_their_spaced_signatures() {
        let cases = [
            (
                "Questions1-4 Completetheformbelow WriteNOMORETHANTWOWORDSforeachanswer.",
                "Questions 1-4 Complete the form below Write NO MORE THAN TWO WORDS for each answer.",
                QuestionNumberExpressionV2::Range { start: 1, end: 4 },
            ),
            (
                "Questions8-10 Completetheformbelow WriteNOMORETHANTWOWORDSANDIORANUMBERforeachanswer.",
                "Questions 8-10 Complete the form below Write NO MORE THAN TWO WORDS AND/OR A NUMBER for each answer.",
                QuestionNumberExpressionV2::Range { start: 8, end: 10 },
            ),
            (
                "Questions17-20 ChooseFOURcorrectanswers,A-F,nexttoquestions17-20.",
                "Questions 17-20 Choose FOUR correct answers, A-F, next to questions 17-20.",
                QuestionNumberExpressionV2::Range { start: 17, end: 20 },
            ),
            (
                "Questions21-25 ChooseFIVEcorrectletters,A-G,nexttoquestions21-25.",
                "Questions 21-25 Choose FIVE correct letters, A-G, next to questions 21-25.",
                QuestionNumberExpressionV2::Range { start: 21, end: 25 },
            ),
            (
                "Questions31-40 Completethenotesbelow WriteNOMORETHANTWOWORDSforeachanswer.",
                "Questions 31-40 Complete the notes below Write NO MORE THAN TWO WORDS for each answer.",
                QuestionNumberExpressionV2::Range { start: 31, end: 40 },
            ),
        ];
        for (compact, spaced, expression) in cases {
            assert_eq!(
                signature_shape(compact, &expression),
                signature_shape(spaced, &expression),
                "compact={compact:?}"
            );
        }
    }

    /// `Choose FOUR/FIVE correct answers/letters, A-X, next to questions N-M` is a
    /// **feature match against one shared bank**, not a multiple-choice question:
    /// each numbered row takes a single letter from the box A-X, so the task has a
    /// bank (and therefore matching semantics) rather than per-question options.
    ///
    /// Tagging it `multiple_choice` closed the bank gate in `mod.rs`
    /// (`detect_option_bank` only runs for matching-family tasks), which left
    /// q17-q25 with an empty option list: no controls in the UI, plus
    /// `OPTION_RUN_INCOMPLETE` / `RESPONSE_GROUP_POLICY_MISMATCH`.
    #[test]
    fn choose_four_and_five_are_feature_matches_with_declared_alphabets() {
        let four = infer_instruction_signature(
            "Questions 17-20 Choose FOUR correct answers, A-F, next to questions 17-20.",
            &QuestionNumberExpressionV2::Range { start: 17, end: 20 },
            None,
            Vec::new(),
        )
        .signature;
        assert_eq!(four.task_type, TaskTypeV2::MatchingFeatures);
        assert_eq!(four.option_alphabet.as_deref(), Some("A-F"));
        assert_eq!(four.selection_cardinality.and_then(|c| c.exact), Some(4));
        // One shared label pool for the whole group, scored per slot.
        assert_eq!(
            four.answer_assignment,
            Some(AssignmentV2::UnorderedSet),
            "four rows draw from one pool, so the assignment stays group-wide"
        );

        let five = infer_instruction_signature(
            "Questions 21-25 Choose FIVE correct letters, A-G, next to questions 21-25.",
            &QuestionNumberExpressionV2::Range { start: 21, end: 25 },
            None,
            Vec::new(),
        )
        .signature;
        assert_eq!(five.task_type, TaskTypeV2::MatchingFeatures);
        assert_eq!(five.option_alphabet.as_deref(), Some("A-G"));
        assert_eq!(five.selection_cardinality.and_then(|c| c.exact), Some(5));
    }

    /// The V1.5 structure hint for these rows is the generic `short_answer`
    /// fallback. It must not be read as a competing structural claim, or the
    /// group carries `task_type_conflict:` and the quality gate blocks it with
    /// `TASK_TYPE_CONFLICT`.
    #[test]
    fn choose_four_reads_a_generic_structure_hint_as_no_claim() {
        let result = infer_instruction_signature(
            "Questions 17-20 What information does the guide give about each of the following collections? Choose FOUR correct answers, A-F, next to questions 17-20.",
            &QuestionNumberExpressionV2::Range { start: 17, end: 20 },
            Some("short_answer"),
            Vec::new(),
        );
        assert_eq!(result.signature.task_type, TaskTypeV2::MatchingFeatures);
        assert!(
            !result
                .warnings
                .iter()
                .any(|warning| warning.starts_with("task_type_conflict:")),
            "a generic fallback hint is not evidence of a conflict: {:?}",
            result.warnings
        );
    }

    #[test]
    fn compact_word_limit_with_and_or_number_is_parsed() {
        let limit = infer_instruction_signature(
            "WriteNOMORETHANTWOWORDSANDIORANUMBERforeachanswer.",
            &range(),
            Some("form_completion"),
            Vec::new(),
        )
        .signature
        .word_limit
        .expect("word limit");
        assert_eq!(limit.max_words, Some(2));
        assert_eq!(limit.max_numbers, Some(1));
        assert_eq!(limit.words_and_or_number, Some(true));
    }
}
