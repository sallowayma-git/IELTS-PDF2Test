import type {
  AuthoringPatch,
  BuildPackInput,
  CreateJobInput,
  DocumentIr,
  ExportNasLibraryInput,
  ExportReadingJsInput,
  ExportResult,
  ImportJob,
  JsExportResult,
  NasExportResult,
  JobFilter,
  JobMetaPatch,
  DiagnosticsSettings,
  EnvironmentPreflightReport,
  LlmProfilePublic,
  LlmSuggestion,
  LlmTestResult,
  ManualTranscriptionInput,
  VisionTranscriptionInput,
  AutoPipelineReport,
  PackBuildResult,
  ParseOptions,
  PreviewAssets,
  ReadingAuthoringIr,
  SaveLlmProfileInput,
  SourceFile,
  SourceFileRole,
  SourceReview,
  SplitCandidates,
  ValidationReport,
  WritingJob,
  CreateWritingJobInput,
  WritingJobPatch,
  WritingJobFilter,
  ExportWritingLibraryInput,
  WritingExportResult,
  LibraryFilter,
  LibraryExamSummary,
  LibraryExamDetail,
  LibraryMetaPatch,
  LibraryStats
} from "../types";
import { devFallbackInvoke, type JobDetail } from "../services/devFallbackBackend";

const isTauriRuntime = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauriRuntime()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(name, args);
  }
  return devFallbackInvoke<T>(name, args);
}

export async function createImportJob(input: CreateJobInput): Promise<ImportJob> {
  return command("create_import_job", { input });
}

export async function listJobs(filter: JobFilter = {}): Promise<ImportJob[]> {
  return command("list_jobs", { filter });
}

export async function getJob(jobId: string): Promise<JobDetail> {
  return command("get_job", { jobId });
}

export async function updateJobMeta(jobId: string, patch: JobMetaPatch): Promise<ImportJob> {
  return command("update_job_meta", { jobId, patch });
}

export async function deleteJob(jobId: string): Promise<void> {
  return command("delete_job", { jobId });
}

export async function importSourceFile(
  jobId: string,
  filePath: string,
  role: SourceFileRole,
  sizeBytes = 0,
  textContent?: string,
  binaryContentBase64?: string
): Promise<SourceFile> {
  return command("import_source_file", { jobId, filePath, role, sizeBytes, textContent, binaryContentBase64 });
}

export async function parseDocument(jobId: string, options: ParseOptions): Promise<DocumentIr> {
  return command("parse_document", { jobId, options });
}

export async function rerunOcr(jobId: string, pageIndices: number[]): Promise<DocumentIr> {
  return command("rerun_ocr", { jobId, pageIndices });
}

export async function applyManualTranscription(jobId: string, input: ManualTranscriptionInput): Promise<DocumentIr> {
  return command("apply_manual_transcription", { jobId, input });
}

export async function applyVisionTranscription(jobId: string, input?: VisionTranscriptionInput): Promise<DocumentIr> {
  return command("apply_vision_transcription", { jobId, input });
}

export async function resolveSourceReview(jobId: string, note?: string): Promise<SourceReview> {
  return command("resolve_source_review", { jobId, note });
}

export async function runRuleSplit(jobId: string, input?: { allowOverwrite?: boolean }): Promise<SplitCandidates> {
  return command("run_rule_split", { jobId, input });
}

export async function saveSplitAdjustments(jobId: string, patch: SplitCandidates): Promise<SplitCandidates> {
  return command("save_split_adjustments", { jobId, patch });
}

export async function buildAuthoringIr(jobId: string, input?: { allowOverwrite?: boolean }): Promise<ReadingAuthoringIr> {
  return command("build_authoring_ir", { jobId, input });
}

export async function updateAuthoringIr(jobId: string, patch: AuthoringPatch): Promise<ReadingAuthoringIr> {
  return command("update_authoring_ir", { jobId, patch });
}

export async function renderGroupHtml(jobId: string, groupId: string): Promise<{ groupId: string; bodyHtml: string }> {
  return command("render_group_html", { jobId, groupId });
}

export async function listLlmProfiles(): Promise<LlmProfilePublic[]> {
  return command("list_llm_profiles");
}

export async function runEnvironmentPreflight(): Promise<EnvironmentPreflightReport> {
  return command("run_environment_preflight");
}

export async function getDiagnosticsSettings(): Promise<DiagnosticsSettings> {
  return command("get_diagnostics_settings");
}

export async function saveDiagnosticsSettings(settings: DiagnosticsSettings): Promise<DiagnosticsSettings> {
  return command("save_diagnostics_settings", { settings });
}

export async function saveLlmProfile(input: SaveLlmProfileInput): Promise<LlmProfilePublic> {
  return command("save_llm_profile", { input });
}

export async function deleteLlmProfile(profileId: string): Promise<LlmProfilePublic[]> {
  return command("delete_llm_profile", { profileId });
}

export async function testLlmProfile(profileId: string): Promise<LlmTestResult> {
  return command("test_llm_profile", { profileId });
}

export async function llmClassifyGroup(jobId: string, groupId: string, profileId: string): Promise<LlmSuggestion> {
  return command("llm_classify_group", { jobId, groupId, profileId });
}

export async function llmExtractGroup(jobId: string, groupId: string, profileId: string): Promise<LlmSuggestion> {
  return command("llm_extract_group", { jobId, groupId, profileId });
}

export async function applyLlmSuggestion(jobId: string, suggestionId: string, selectedPaths: string[]): Promise<ReadingAuthoringIr> {
  return command("apply_llm_suggestion", { jobId, suggestionId, selectedPaths });
}

export async function validateAuthoringIr(jobId: string): Promise<ValidationReport> {
  return command("validate_authoring_ir", { jobId });
}

export async function generatePreviewAssets(jobId: string): Promise<PreviewAssets> {
  return command("generate_preview_assets", { jobId });
}

export async function runPreviewE2e(jobId: string): Promise<ValidationReport> {
  return command("run_preview_e2e", { jobId });
}

export async function runAutoPipeline(jobId: string, input?: { profileId?: string; confidenceThreshold?: number; parseMode?: ParseOptions["mode"]; executionMode?: "localOnly" | "full"; target?: "editableDraft"; allowOverwrite?: boolean }): Promise<AutoPipelineReport> {
  return command("run_auto_pipeline", { jobId, input });
}

export async function runCloudReview(jobId: string, input?: { profileId?: string }): Promise<AutoPipelineReport> {
  return command("run_cloud_review", { jobId, input });
}

export async function exportReadingAssets(jobId: string, exportDir = "local://exports"): Promise<ExportResult> {
  return command("export_reading_assets", { jobId, exportDir });
}

export async function exportReadingJs(input: ExportReadingJsInput): Promise<JsExportResult> {
  return command("export_reading_js", { input });
}

export async function exportNasLibrary(input: ExportNasLibraryInput): Promise<NasExportResult> {
  return command("export_nas_library", { input });
}

export async function buildPack(input: BuildPackInput): Promise<PackBuildResult> {
  return command("build_pack", { input });
}

// ---------- 写作题库命令 ----------
export async function createWritingJob(input: CreateWritingJobInput): Promise<WritingJob> {
  return command("create_writing_job", { input });
}

export async function listWritingJobs(filter: WritingJobFilter = {}): Promise<WritingJob[]> {
  return command("list_writing_jobs", { filter });
}

export async function getWritingJob(jobId: string): Promise<WritingJob> {
  return command("get_writing_job", { jobId });
}

export async function updateWritingJob(jobId: string, patch: WritingJobPatch): Promise<WritingJob> {
  return command("update_writing_job", { jobId, patch });
}

export async function deleteWritingJob(jobId: string): Promise<{ deleted: true; jobId: string }> {
  return command("delete_writing_job", { jobId });
}

export async function exportWritingLibrary(input: ExportWritingLibraryInput): Promise<WritingExportResult> {
  return command("export_writing_library", { input });
}

// ---------- 题库管理命令 ----------
export async function listLibraryExams(filter: LibraryFilter = {}): Promise<LibraryExamSummary[]> {
  return command("list_library_exams", { filter });
}

export async function getLibraryExam(id: string): Promise<LibraryExamDetail | null> {
  return command("get_library_exam", { id });
}

export async function updateLibraryExamMeta(id: string, patch: LibraryMetaPatch): Promise<LibraryExamSummary | null> {
  return command("update_library_exam_meta", { id, patch });
}

export async function deleteLibraryExam(id: string): Promise<boolean> {
  return command("delete_library_exam", { id });
}

export async function searchLibraryExams(query: string): Promise<LibraryExamSummary[]> {
  return command("search_library_exams", { query });
}

export async function getLibraryStats(): Promise<LibraryStats> {
  return command("get_library_stats");
}

export async function restoreLibraryExam(id: string): Promise<boolean> {
  return command("restore_library_exam", { id });
}

export async function listTrashedExams(): Promise<LibraryExamSummary[]> {
  return command("list_trashed_exams");
}
