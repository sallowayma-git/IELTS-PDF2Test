//! 题型 → 呈现约束的单一事实源。识别、编辑器、门禁与云端提示词都从这里取规则，
//! 前端经 `src/types/taskPresentationRules.json`（由本模块测试生成并比对）读取同一份数据。

use super::ielts_authoring_v2::{
    AnswerSlotHostTypeV2, AssignmentV2, InteractionV2, ResponseGroupKindV2, TaskTypeV2,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresentationKind {
    ParagraphDropzone,
    RowDropzone,
    InlineDropzone,
    RadioList,
    RadioMatrix,
    CheckboxSet,
    InlineText,
    FigureInputs,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostPlacement {
    PassageBeforeParagraph,
    QuestionRow,
    PromptAfterQuestionNumber,
    InlineStimulus,
    TableCell,
    FlowStep,
    FigureBelow,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExampleSlotPolicy {
    NoAnswerControl,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OptionSource {
    None,
    FixedTruthLabels,
    PerSlotOptions,
    GroupOptions,
    OptionBank,
    ParagraphMap,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OptionAlphabet {
    None,
    FixedTruth,
    Letters,
    LettersAbcd,
    Roman,
    ParagraphLetters,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GroupGranularity {
    PerSlot,
    TaskGroup,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OptionReusePolicy {
    NotApplicable,
    Always,
    Never,
    InstructionControlled,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskPresentationRule {
    pub task_type: TaskTypeV2,
    pub variant: &'static str,
    pub response_kind: ResponseGroupKindV2,
    pub assignment: AssignmentV2,
    pub interaction: InteractionV2,
    /// 第一个为默认宿主；其余为明确允许的兼容宿主。
    pub host_types: &'static [AnswerSlotHostTypeV2],
    pub host_placement: HostPlacement,
    pub presentation: PresentationKind,
    pub example_slot_policy: ExampleSlotPolicy,
    pub option_source: OptionSource,
    pub option_alphabet: OptionAlphabet,
    /// `OptionAlphabet::FixedTruth` 的固定选项文本；TFNG 与 YNNG 只差这一处。
    pub fixed_option_labels: &'static [&'static str],
    pub option_reuse_policy: OptionReusePolicy,
    pub option_reuse_default: bool,
    pub group_granularity: GroupGranularity,
}

pub const VARIANT_DEFAULT: &str = "default";
pub const VARIANT_WORD_BANK: &str = "word_bank";
pub const VARIANT_LETTERS: &str = "letters";

const TFNG_LABELS: &[&str] = &["TRUE", "FALSE", "NOT GIVEN"];
const YNNG_LABELS: &[&str] = &["YES", "NO", "NOT GIVEN"];

#[allow(clippy::too_many_arguments)]
const fn choice_rule(
    task_type: TaskTypeV2,
    assignment: AssignmentV2,
    interaction: InteractionV2,
    presentation: PresentationKind,
    option_source: OptionSource,
    option_alphabet: OptionAlphabet,
    fixed_option_labels: &'static [&'static str],
    group_granularity: GroupGranularity,
    option_reuse_policy: OptionReusePolicy,
    option_reuse_default: bool,
) -> TaskPresentationRule {
    TaskPresentationRule {
        task_type,
        variant: VARIANT_DEFAULT,
        response_kind: ResponseGroupKindV2::Choice,
        assignment,
        interaction,
        host_types: &[AnswerSlotHostTypeV2::Prompt],
        host_placement: HostPlacement::PromptAfterQuestionNumber,
        presentation,
        example_slot_policy: ExampleSlotPolicy::NoAnswerControl,
        option_source,
        option_alphabet,
        fixed_option_labels,
        option_reuse_policy,
        option_reuse_default,
        group_granularity,
    }
}

#[allow(clippy::too_many_arguments)]
const fn matching_rule(
    task_type: TaskTypeV2,
    variant: &'static str,
    option_source: OptionSource,
    interaction: InteractionV2,
    host_types: &'static [AnswerSlotHostTypeV2],
    host_placement: HostPlacement,
    presentation: PresentationKind,
    option_alphabet: OptionAlphabet,
    group_granularity: GroupGranularity,
    option_reuse_policy: OptionReusePolicy,
    option_reuse_default: bool,
) -> TaskPresentationRule {
    TaskPresentationRule {
        task_type,
        variant,
        response_kind: ResponseGroupKindV2::Matching,
        assignment: AssignmentV2::PerSlot,
        interaction,
        host_types,
        host_placement,
        presentation,
        example_slot_policy: ExampleSlotPolicy::NoAnswerControl,
        option_source,
        option_alphabet,
        fixed_option_labels: &[],
        option_reuse_policy,
        option_reuse_default,
        group_granularity,
    }
}

const fn text_rule(
    task_type: TaskTypeV2,
    host_types: &'static [AnswerSlotHostTypeV2],
    presentation: PresentationKind,
    host_placement: HostPlacement,
) -> TaskPresentationRule {
    TaskPresentationRule {
        task_type,
        variant: VARIANT_DEFAULT,
        response_kind: ResponseGroupKindV2::TextEntry,
        assignment: AssignmentV2::PerSlot,
        interaction: InteractionV2::Text,
        host_types,
        host_placement,
        presentation,
        example_slot_policy: ExampleSlotPolicy::NoAnswerControl,
        option_source: OptionSource::None,
        option_alphabet: OptionAlphabet::None,
        fixed_option_labels: &[],
        option_reuse_policy: OptionReusePolicy::NotApplicable,
        option_reuse_default: false,
        group_granularity: GroupGranularity::TaskGroup,
    }
}

static RULES: &[TaskPresentationRule] = &[
    choice_rule(
        TaskTypeV2::TrueFalseNotGiven,
        AssignmentV2::PerSlot,
        InteractionV2::Radio,
        PresentationKind::RadioList,
        OptionSource::FixedTruthLabels,
        OptionAlphabet::FixedTruth,
        TFNG_LABELS,
        GroupGranularity::PerSlot,
        OptionReusePolicy::Always,
        true,
    ),
    choice_rule(
        TaskTypeV2::YesNoNotGiven,
        AssignmentV2::PerSlot,
        InteractionV2::Radio,
        PresentationKind::RadioList,
        OptionSource::FixedTruthLabels,
        OptionAlphabet::FixedTruth,
        YNNG_LABELS,
        GroupGranularity::PerSlot,
        OptionReusePolicy::Always,
        true,
    ),
    choice_rule(
        TaskTypeV2::SingleChoice,
        AssignmentV2::PerSlot,
        InteractionV2::Radio,
        PresentationKind::RadioList,
        OptionSource::PerSlotOptions,
        OptionAlphabet::LettersAbcd,
        &[],
        GroupGranularity::PerSlot,
        OptionReusePolicy::NotApplicable,
        false,
    ),
    choice_rule(
        TaskTypeV2::MultipleChoice,
        AssignmentV2::UnorderedSet,
        InteractionV2::Checkbox,
        PresentationKind::CheckboxSet,
        OptionSource::OptionBank,
        OptionAlphabet::Letters,
        &[],
        GroupGranularity::TaskGroup,
        OptionReusePolicy::NotApplicable,
        false,
    ),
    matching_rule(
        TaskTypeV2::MatchingHeadings,
        VARIANT_DEFAULT,
        OptionSource::OptionBank,
        InteractionV2::Dragdrop,
        &[AnswerSlotHostTypeV2::PassageParagraph],
        HostPlacement::PassageBeforeParagraph,
        PresentationKind::ParagraphDropzone,
        OptionAlphabet::Roman,
        GroupGranularity::TaskGroup,
        OptionReusePolicy::Never,
        false,
    ),
    matching_rule(
        TaskTypeV2::MatchingInformation,
        VARIANT_DEFAULT,
        OptionSource::ParagraphMap,
        InteractionV2::Radio,
        &[AnswerSlotHostTypeV2::Prompt],
        HostPlacement::QuestionRow,
        PresentationKind::RadioMatrix,
        OptionAlphabet::ParagraphLetters,
        GroupGranularity::PerSlot,
        OptionReusePolicy::Always,
        true,
    ),
    matching_rule(
        TaskTypeV2::MatchingFeatures,
        VARIANT_DEFAULT,
        OptionSource::OptionBank,
        InteractionV2::Dragdrop,
        &[AnswerSlotHostTypeV2::Prompt],
        HostPlacement::QuestionRow,
        PresentationKind::RowDropzone,
        OptionAlphabet::Letters,
        GroupGranularity::TaskGroup,
        OptionReusePolicy::InstructionControlled,
        false,
    ),
    matching_rule(
        TaskTypeV2::Classification,
        VARIANT_DEFAULT,
        OptionSource::OptionBank,
        InteractionV2::Dragdrop,
        &[AnswerSlotHostTypeV2::Prompt],
        HostPlacement::QuestionRow,
        PresentationKind::RowDropzone,
        OptionAlphabet::Letters,
        GroupGranularity::TaskGroup,
        OptionReusePolicy::InstructionControlled,
        false,
    ),
    matching_rule(
        TaskTypeV2::MatchingSentenceEndings,
        VARIANT_DEFAULT,
        OptionSource::OptionBank,
        InteractionV2::Dragdrop,
        &[AnswerSlotHostTypeV2::Prompt],
        HostPlacement::QuestionRow,
        PresentationKind::RowDropzone,
        OptionAlphabet::Letters,
        GroupGranularity::TaskGroup,
        OptionReusePolicy::InstructionControlled,
        false,
    ),
    text_rule(
        TaskTypeV2::SummaryCompletion,
        &[
            AnswerSlotHostTypeV2::Paragraph,
            AnswerSlotHostTypeV2::Prompt,
        ],
        PresentationKind::InlineText,
        HostPlacement::InlineStimulus,
    ),
    matching_rule(
        TaskTypeV2::SummaryCompletion,
        VARIANT_WORD_BANK,
        OptionSource::OptionBank,
        InteractionV2::Dragdrop,
        &[
            AnswerSlotHostTypeV2::Paragraph,
            AnswerSlotHostTypeV2::Prompt,
        ],
        HostPlacement::InlineStimulus,
        PresentationKind::InlineDropzone,
        OptionAlphabet::Letters,
        GroupGranularity::TaskGroup,
        OptionReusePolicy::Never,
        false,
    ),
    text_rule(
        TaskTypeV2::SentenceCompletion,
        &[
            AnswerSlotHostTypeV2::Prompt,
            AnswerSlotHostTypeV2::Paragraph,
        ],
        PresentationKind::InlineText,
        HostPlacement::InlineStimulus,
    ),
    text_rule(
        TaskTypeV2::NoteCompletion,
        &[
            AnswerSlotHostTypeV2::Paragraph,
            AnswerSlotHostTypeV2::Prompt,
        ],
        PresentationKind::InlineText,
        HostPlacement::InlineStimulus,
    ),
    text_rule(
        TaskTypeV2::TableCompletion,
        &[AnswerSlotHostTypeV2::TableCell],
        PresentationKind::InlineText,
        HostPlacement::TableCell,
    ),
    text_rule(
        TaskTypeV2::FormCompletion,
        &[
            AnswerSlotHostTypeV2::TableCell,
            AnswerSlotHostTypeV2::Paragraph,
            AnswerSlotHostTypeV2::Prompt,
        ],
        PresentationKind::InlineText,
        HostPlacement::TableCell,
    ),
    text_rule(
        TaskTypeV2::FlowchartCompletion,
        &[
            AnswerSlotHostTypeV2::FlowStep,
            AnswerSlotHostTypeV2::Paragraph,
        ],
        PresentationKind::InlineText,
        HostPlacement::FlowStep,
    ),
    text_rule(
        TaskTypeV2::DiagramLabelCompletion,
        &[AnswerSlotHostTypeV2::Prompt],
        PresentationKind::FigureInputs,
        HostPlacement::FigureBelow,
    ),
    text_rule(
        TaskTypeV2::PlanMapLabelCompletion,
        &[AnswerSlotHostTypeV2::Prompt],
        PresentationKind::InlineText,
        HostPlacement::QuestionRow,
    ),
    matching_rule(
        TaskTypeV2::PlanMapLabelCompletion,
        VARIANT_LETTERS,
        OptionSource::OptionBank,
        InteractionV2::Dragdrop,
        &[AnswerSlotHostTypeV2::Prompt],
        HostPlacement::QuestionRow,
        PresentationKind::RowDropzone,
        OptionAlphabet::Letters,
        GroupGranularity::TaskGroup,
        OptionReusePolicy::Never,
        false,
    ),
    text_rule(
        TaskTypeV2::ShortAnswer,
        &[AnswerSlotHostTypeV2::Prompt],
        PresentationKind::InlineText,
        HostPlacement::PromptAfterQuestionNumber,
    ),
];

pub fn presentation_rules() -> &'static [TaskPresentationRule] {
    RULES
}

/// 有选项库时，summary_completion 走 word_bank、plan_map_label_completion 走 letters；
/// 其它题型的选项库本就是默认规则的一部分，不改变变体。
pub fn rule_for(task_type: &TaskTypeV2, has_option_bank: bool) -> &'static TaskPresentationRule {
    let variant = match (task_type, has_option_bank) {
        (TaskTypeV2::SummaryCompletion, true) => VARIANT_WORD_BANK,
        (TaskTypeV2::PlanMapLabelCompletion, true) => VARIANT_LETTERS,
        _ => VARIANT_DEFAULT,
    };
    RULES
        .iter()
        .find(|rule| &rule.task_type == task_type && rule.variant == variant)
        .expect("每个题型都有 default 规则，词库变体由测试覆盖")
}

pub(crate) fn wire_name<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        other => panic!("规则表枚举必须序列化为字符串: {other:?}"),
    }
}

/// 给 LLM 的规则表（Markdown）。取值与 JSON 线上字段一致，模型可直接照抄。
pub fn rules_prompt_table() -> String {
    let mut table = String::from(
        "| taskType | variant | responseGroup.kind | assignment | interaction | hostTypes (first = default) | hostPlacement | presentation | exampleSlotPolicy | optionSource | optionAlphabet | optionReusePolicy | allowOptionReuse default | responseGroup granularity |\n\
         |---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for rule in RULES {
        let hosts = rule
            .host_types
            .iter()
            .map(wire_name)
            .collect::<Vec<_>>()
            .join(", ");
        let alphabet = if rule.fixed_option_labels.is_empty() {
            wire_name(&rule.option_alphabet)
        } else {
            format!(
                "{} ({})",
                wire_name(&rule.option_alphabet),
                rule.fixed_option_labels.join(" / ")
            )
        };
        let granularity = match rule.group_granularity {
            GroupGranularity::PerSlot => "one per scoring question, one slot each",
            GroupGranularity::TaskGroup => "one shared group for this task group",
        };
        table.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            wire_name(&rule.task_type),
            rule.variant,
            wire_name(&rule.response_kind),
            wire_name(&rule.assignment),
            wire_name(&rule.interaction),
            hosts,
            wire_name(&rule.host_placement),
            wire_name(&rule.presentation),
            wire_name(&rule.example_slot_policy),
            wire_name(&rule.option_source),
            alphabet,
            wire_name(&rule.option_reuse_policy),
            rule.option_reuse_default,
            granularity,
        ));
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn all_task_types() -> Vec<TaskTypeV2> {
        let all = vec![
            TaskTypeV2::SingleChoice,
            TaskTypeV2::MultipleChoice,
            TaskTypeV2::TrueFalseNotGiven,
            TaskTypeV2::YesNoNotGiven,
            TaskTypeV2::MatchingInformation,
            TaskTypeV2::MatchingHeadings,
            TaskTypeV2::MatchingFeatures,
            TaskTypeV2::MatchingSentenceEndings,
            TaskTypeV2::Classification,
            TaskTypeV2::SentenceCompletion,
            TaskTypeV2::SummaryCompletion,
            TaskTypeV2::NoteCompletion,
            TaskTypeV2::TableCompletion,
            TaskTypeV2::FormCompletion,
            TaskTypeV2::FlowchartCompletion,
            TaskTypeV2::DiagramLabelCompletion,
            TaskTypeV2::PlanMapLabelCompletion,
            TaskTypeV2::ShortAnswer,
        ];
        // 穷举 match：TaskTypeV2 新增变体时这里编译失败，逼着把它加进上面的列表和规则表。
        for task_type in &all {
            match task_type {
                TaskTypeV2::SingleChoice
                | TaskTypeV2::MultipleChoice
                | TaskTypeV2::TrueFalseNotGiven
                | TaskTypeV2::YesNoNotGiven
                | TaskTypeV2::MatchingInformation
                | TaskTypeV2::MatchingHeadings
                | TaskTypeV2::MatchingFeatures
                | TaskTypeV2::MatchingSentenceEndings
                | TaskTypeV2::Classification
                | TaskTypeV2::SentenceCompletion
                | TaskTypeV2::SummaryCompletion
                | TaskTypeV2::NoteCompletion
                | TaskTypeV2::TableCompletion
                | TaskTypeV2::FormCompletion
                | TaskTypeV2::FlowchartCompletion
                | TaskTypeV2::DiagramLabelCompletion
                | TaskTypeV2::PlanMapLabelCompletion
                | TaskTypeV2::ShortAnswer => {}
            }
        }
        all
    }

    #[test]
    fn every_task_type_has_exactly_one_default_rule() {
        for task_type in all_task_types() {
            let defaults = presentation_rules()
                .iter()
                .filter(|rule| rule.task_type == task_type && rule.variant == VARIANT_DEFAULT)
                .count();
            assert_eq!(defaults, 1, "{task_type:?} 必须恰有一条 default 规则");
        }
    }

    #[test]
    fn rules_are_unique_per_task_type_and_variant_and_have_hosts() {
        let rules = presentation_rules();
        for (index, rule) in rules.iter().enumerate() {
            assert!(!rule.host_types.is_empty(), "{rule:?} 缺默认宿主");
            assert!(
                [VARIANT_DEFAULT, VARIANT_WORD_BANK, VARIANT_LETTERS].contains(&rule.variant),
                "未知变体 {}",
                rule.variant
            );
            assert_eq!(
                rule.option_alphabet == OptionAlphabet::FixedTruth,
                !rule.fixed_option_labels.is_empty(),
                "固定选项文本只属于 fixed_truth 字母表: {rule:?}"
            );
            assert!(
                rules[index + 1..]
                    .iter()
                    .all(|other| other.task_type != rule.task_type
                        || other.variant != rule.variant),
                "{:?}/{} 重复",
                rule.task_type,
                rule.variant
            );
        }
    }

    #[test]
    fn rule_for_picks_bank_variants_only_for_summary_and_plan_map() {
        let summary = rule_for(&TaskTypeV2::SummaryCompletion, true);
        assert_eq!(summary.variant, VARIANT_WORD_BANK);
        assert_eq!(summary.presentation, PresentationKind::InlineDropzone);
        assert_eq!(summary.interaction, InteractionV2::Dragdrop);
        assert_eq!(
            rule_for(&TaskTypeV2::SummaryCompletion, false).presentation,
            PresentationKind::InlineText
        );

        let plan_map = rule_for(&TaskTypeV2::PlanMapLabelCompletion, true);
        assert_eq!(plan_map.variant, VARIANT_LETTERS);
        assert_eq!(plan_map.response_kind, ResponseGroupKindV2::Matching);
        assert_eq!(
            rule_for(&TaskTypeV2::PlanMapLabelCompletion, false).presentation,
            PresentationKind::InlineText
        );

        // 标题配对本身就靠选项库，有库也仍是 default。
        let headings = rule_for(&TaskTypeV2::MatchingHeadings, true);
        assert_eq!(headings.variant, VARIANT_DEFAULT);
        assert_eq!(headings.presentation, PresentationKind::ParagraphDropzone);
        assert_eq!(
            headings.host_types.first(),
            Some(&AnswerSlotHostTypeV2::PassageParagraph)
        );
        assert_eq!(
            rule_for(&TaskTypeV2::MatchingInformation, true).group_granularity,
            GroupGranularity::PerSlot
        );
    }

    #[test]
    fn contract_explicitly_defines_host_location_grouping_and_reuse_policy() {
        let json = |task_type: &TaskTypeV2, has_bank| {
            serde_json::to_value(rule_for(task_type, has_bank)).expect("规则必须可序列化")
        };

        let headings = json(&TaskTypeV2::MatchingHeadings, true);
        assert_eq!(headings["hostPlacement"], "passage_before_paragraph");
        assert_eq!(headings["groupGranularity"], "task_group");
        assert_eq!(headings["optionReusePolicy"], "never");

        let information = json(&TaskTypeV2::MatchingInformation, false);
        assert_eq!(information["optionSource"], "paragraph_map");
        assert_eq!(information["optionAlphabet"], "paragraph_letters");
        assert_eq!(information["groupGranularity"], "per_slot");
        assert_eq!(information["optionReusePolicy"], "always");

        let features = json(&TaskTypeV2::MatchingFeatures, true);
        assert_eq!(features["hostPlacement"], "question_row");
        assert_eq!(features["optionReusePolicy"], "instruction_controlled");
        assert_eq!(features["optionReuseDefault"], false);

        let summary = json(&TaskTypeV2::SummaryCompletion, true);
        assert_eq!(summary["hostPlacement"], "inline_stimulus");
        assert_eq!(summary["groupGranularity"], "task_group");

        let diagram = json(&TaskTypeV2::DiagramLabelCompletion, false);
        assert_eq!(diagram["hostPlacement"], "figure_below");
        assert!(presentation_rules().iter().all(|rule| {
            serde_json::to_value(rule).expect("规则必须可序列化")["groupGranularity"] != "any"
        }));
    }

    #[test]
    fn choose_two_uses_one_unordered_task_group_over_a_shared_option_bank() {
        let rule = rule_for(&TaskTypeV2::MultipleChoice, false);
        assert_eq!(rule.assignment, AssignmentV2::UnorderedSet);
        assert_eq!(rule.group_granularity, GroupGranularity::TaskGroup);
        assert_eq!(rule.option_source, OptionSource::OptionBank);
    }

    #[test]
    fn prompt_table_lists_every_task_type_and_variant() {
        let table = rules_prompt_table();
        for rule in presentation_rules() {
            let row_prefix = format!("| {} | {} |", wire_name(&rule.task_type), rule.variant);
            assert!(table.contains(&row_prefix), "规则表缺 {row_prefix}");
        }
        assert!(table.contains("passage_paragraph"));
        assert!(table.contains("TRUE / FALSE / NOT GIVEN"));
        assert!(table.contains("YES / NO / NOT GIVEN"));
    }

    fn rules_json_path() -> std::path::PathBuf {
        let manifest = env!("CARGO_MANIFEST_DIR").trim_end_matches(['\\', '/']);
        std::path::Path::new(manifest)
            .parent()
            .expect("src-tauri 必须有父目录")
            .join("src/types/taskPresentationRules.json")
    }

    #[test]
    fn frontend_rules_json_matches_rust_rules() {
        let expected = serde_json::to_value(presentation_rules()).expect("规则表可序列化");
        let path = rules_json_path();
        if std::env::var("UPDATE_TASK_PRESENTATION_RULES").as_deref() == Ok("1") {
            let text = serde_json::to_string_pretty(&expected).expect("规则表可序列化") + "\n";
            std::fs::write(&path, text)
                .unwrap_or_else(|error| panic!("写入 {path:?} 失败: {error}"));
        }
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("读取 {path:?} 失败: {error}；用 UPDATE_TASK_PRESENTATION_RULES=1 生成")
        });
        let actual: Value = serde_json::from_str(&text).expect("规则 JSON 必须合法");
        assert_eq!(
            actual, expected,
            "前端规则 JSON 与 Rust 规则表不一致；用 UPDATE_TASK_PRESENTATION_RULES=1 重写"
        );
    }
}
