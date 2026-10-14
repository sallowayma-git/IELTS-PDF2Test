import type { InteractionSpec } from "./authoring-ir";
export type LlmProvider = "OpenAiCompatible" | "AnthropicCompatible" | "Ollama" | "Custom";

export interface LlmProfilePublic {
  profileId: string;
  name: string;
  provider: LlmProvider;
  baseUrl: string;
  model: string;
  temperature: number;
  timeoutMs: number;
  forceJson: boolean;
  enabled: boolean;
  hasApiKey: boolean;
  apiKeySecretRef?: string;
  secretStorageBackend?: "os" | "keychain" | "file" | "none";
  secretStorageMessage?: string;
}

export interface SaveLlmProfileInput {
  profileId?: string;
  name: string;
  provider: LlmProvider;
  baseUrl: string;
  model: string;
  apiKey?: string;
  temperature: number;
  timeoutMs: number;
  forceJson: boolean;
  enabled: boolean;
}

export interface LlmTestResult {
  ok: boolean;
  message: string;
  latencyMs: number;
}

export interface EnvironmentPreflightCheck {
  name: string;
  ok: boolean;
  severity: "error" | "warning" | "info";
  message: string;
  details?: unknown;
}

export interface EnvironmentPreflightReport {
  schemaVersion: "EnvironmentPreflightV1";
  ok: boolean;
  errors: number;
  warnings: number;
  checks: EnvironmentPreflightCheck[];
  generatedAt: string;
}

export interface DiagnosticsSettings {
  keepFullProcessArtifacts: boolean;
}

/**
 * 流水线模式（cloud-direct 方案 §2）：`direct` = 云端一次直出整份题稿（默认）；
 * `legacy` = 现有本地首稿 + 云端候选合并链（行为不变）。导入时快照进 job，
 * 切换设置不改变已在处理中的作业。
 */
export type PipelineMode = "direct" | "legacy";

export interface PipelineSettingsV1 {
  schemaVersion: "PipelineSettingsV1";
  pipelineMode: PipelineMode;
}

export interface LlmSuggestionPatch {
  op?: string;
  path?: string;
  value?: string;
}

export interface LlmSuggestionQuestion {
  id: string;
  prompt?: string;
  interaction?: InteractionSpec | {
    type?: string;
    options?: string[];
  };
  [key: string]: unknown;
}

export interface LlmSuggestion {
  suggestionId: string;
  jobId: string;
  groupId: string;
  kind?: string;
  confidence: number;
  patch?: LlmSuggestionPatch[] | unknown;
  questions?: LlmSuggestionQuestion[];
  evidence?: unknown;
  warnings: string[];
  createdAt: string;
}

export interface AutoPipelineReport {
  jobId: string;
  confidenceThreshold: number;
  llm: {
    suggestionCount: number;
    appliedCount: number;
    highConfidenceAppliedGroups: string[];
    lowConfidenceGroups: string[];
    blockedAutoApplyGroups?: string[];
    failures: string[];
    profileId?: string;
  };
  parser?: {
    warnings: string[];
    lowConfidenceBlocks: string[];
    visionTranscription?: {
      attempted: boolean;
      applied: boolean;
      profileId?: string | null;
      warnings?: string[];
      failure?: string | null;
      confidence?: number;
    };
    visionAnswerExtraction?: {
      attempted: boolean;
      applied: boolean;
      profileId?: string | null;
      state?: "succeeded" | "failed" | "not_executed";
      stateReason?: string;
      answerPageImageCount?: number | null;
      answerPageIndexes?: number[];
      answerCount?: number;
      constraintViolations?: Array<{ code?: string; questionNumber?: string | null; message?: string; [key: string]: unknown }>;
      reviewWarnings?: Array<{ code?: string; message?: string; [key: string]: unknown }>;
      warnings?: string[];
      failure?: string | null;
      confidence?: number;
      filledQuestionIds?: string[];
      missingQuestionIds?: string[];
    };
  };
  quality?: {
    cloudComparison?: {
      attempted: boolean;
      passed: boolean;
      profileId?: string | null;
      warningCount?: number;
      failure?: string | null;
      issues?: Array<{ message?: string; [key: string]: unknown }>;
      observations?: Array<{ message?: string; [key: string]: unknown }>;
      localSummary?: Array<{ range?: number[]; kind?: string; layoutHint?: string; questionIds?: string[] }>;
      cloudSummary?: Array<{ range?: number[]; kind?: string; layoutHint?: string; questionIds?: string[] }>;
      comparison?: unknown;
    };
  };
  validationPassed: boolean;
  staticRuntimePassed?: boolean;
  realRuntimePassed?: boolean;
  runtimeMode?: string;
  authoring?: {
    remainingReviewItems: number;
  };
  status: string;
  currentStep: string;
  userStatus?: "draftReady" | "needsConfirmation" | "failed";
  userMessage?: string;
  nextRoute?: "preview" | "groups" | "document" | "review";
  generatedAt: string;
  validationReport?: unknown;
}
