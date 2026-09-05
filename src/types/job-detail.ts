import type {
  AutoPipelineReport,
  DocumentIr,
  ImportJob,
  LlmSuggestion,
  PreviewAssets,
  ReadingAuthoringIr,
  SourceReview,
  SplitCandidates,
  ValidationReport,
  VisionAnswerCandidates
} from ".";

/** 完整 job 详情（M1 起）从 devFallbackBackend 迁到 types：
 *  生产 transport 与测试替身都要用这个形状，类型不能反向依赖测试替身。 */
export interface JobDetail {
  job: ImportJob;
  documentIr?: DocumentIr;
  sourceReview?: SourceReview;
  splitCandidates?: SplitCandidates;
  authoringIr?: ReadingAuthoringIr;
  validationReport?: ValidationReport;
  previewAssets?: PreviewAssets;
  pipelineReport?: AutoPipelineReport;
  visionAnswerCandidates?: VisionAnswerCandidates;
  llmSuggestions: LlmSuggestion[];
}
