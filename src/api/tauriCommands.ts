import type {
  AuthoringPatch,
  BuildPackInput,
  CreateJobInput,
  DocumentIr,
  ExportResult,
  ImportJob,
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
  ValidationReport
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

export async function runAutoPipeline(jobId: string, input?: { profileId?: string; confidenceThreshold?: number; parseMode?: ParseOptions["mode"]; target?: "editableDraft"; allowOverwrite?: boolean }): Promise<AutoPipelineReport> {
  return command("run_auto_pipeline", { jobId, input });
}

export async function exportReadingAssets(jobId: string, exportDir = "local://exports"): Promise<ExportResult> {
  return command("export_reading_assets", { jobId, exportDir });
}

export async function buildPack(input: BuildPackInput): Promise<PackBuildResult> {
  return command("build_pack", { input });
}
