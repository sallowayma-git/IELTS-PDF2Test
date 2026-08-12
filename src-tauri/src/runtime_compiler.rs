use crate::reading_source_v2::{compile_reading_source_v2, CompilerIssueV2, ReadingExamSourceV2};
use crate::schema::IeltsAuthoringIRV2;

pub(crate) trait ExamCompiler {
    type Output;

    fn compile(&self, source: &IeltsAuthoringIRV2) -> Result<Self::Output, Vec<CompilerIssueV2>>;
    fn validate(&self, compiled: &Self::Output) -> Vec<CompilerIssueV2>;
}

pub(crate) struct ReadingExamCompilerV2;

impl ExamCompiler for ReadingExamCompilerV2 {
    type Output = ReadingExamSourceV2;

    fn compile(&self, source: &IeltsAuthoringIRV2) -> Result<Self::Output, Vec<CompilerIssueV2>> {
        compile_reading_source_v2(source)
    }

    fn validate(&self, compiled: &Self::Output) -> Vec<CompilerIssueV2> {
        crate::reading_source_v2::validate_reading_source_v2(compiled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_trait_is_objectively_wired_to_the_reading_v2_validator() {
        fn assert_compiler<T: ExamCompiler<Output = ReadingExamSourceV2>>() {}
        assert_compiler::<ReadingExamCompilerV2>();
    }
}
