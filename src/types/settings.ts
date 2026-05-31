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
  secretStorageBackend?: "keychain" | "file" | "none";
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

export interface LlmSuggestion {
  suggestionId: string;
  jobId: string;
  groupId: string;
  kind?: string;
  confidence: number;
  patch: unknown;
  questions?: unknown[];
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
  };
  validationPassed: boolean;
  realRuntimePassed?: boolean;
  runtimeMode?: string;
  status: string;
  currentStep: string;
  generatedAt: string;
  validationReport?: unknown;
}
