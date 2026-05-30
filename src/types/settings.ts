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
