export interface Phase0FeatureFlags {
  documentIrV2: boolean;
  authoringV2: boolean;
  runtimeSourceV2: boolean;
  nasPackageV2: boolean;
  listeningV1: boolean;
  pdfPerQuestionLlmRepair: boolean;
}

export const DEFAULT_PHASE0_FEATURE_FLAGS: Readonly<Phase0FeatureFlags> = Object.freeze({
  documentIrV2: false,
  authoringV2: false,
  runtimeSourceV2: false,
  nasPackageV2: false,
  listeningV1: false,
  pdfPerQuestionLlmRepair: false
});

export type Phase0FeatureFlagName = keyof Phase0FeatureFlags;

export function resolvePhase0FeatureFlags(
  overrides: Partial<Phase0FeatureFlags> = {}
): Phase0FeatureFlags {
  return {
    ...DEFAULT_PHASE0_FEATURE_FLAGS,
    ...overrides,
    // This safety constraint is not overridable by a caller during Phase 0.
    pdfPerQuestionLlmRepair: false
  };
}

export interface Phase1FeatureFlags extends Phase0FeatureFlags {
  documentIrV2Shadow: boolean;
}

export const DEFAULT_PHASE1_FEATURE_FLAGS: Readonly<Phase1FeatureFlags> = Object.freeze({
  ...DEFAULT_PHASE0_FEATURE_FLAGS,
  documentIrV2Shadow: false
});

export type Phase1FeatureFlagName = keyof Phase1FeatureFlags;

export function resolvePhase1FeatureFlags(
  overrides: Partial<Phase1FeatureFlags> = {}
): Phase1FeatureFlags {
  return {
    ...DEFAULT_PHASE1_FEATURE_FLAGS,
    ...overrides,
    // Phase 1 shadow extraction is opt-in development instrumentation only.
    pdfPerQuestionLlmRepair: false
  };
}
