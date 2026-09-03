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
      answerCount?: number;
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
