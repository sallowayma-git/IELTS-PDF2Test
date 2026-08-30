export interface Phase0FeatureFlags {
  documentIrV2: boolean;
  authoringV2: boolean;
  runtimeSourceV2: boolean;
  nasPackageV2: boolean;
  listeningV1: boolean;
  pdfPerQuestionLlmRepair: boolean;
}

export const DEFAULT_PHASE0_FEATURE_FLAGS: Readonly<Phase0FeatureFlags> = Object.freeze({
  documentIrV2: true,
  authoringV2: true,
  runtimeSourceV2: true,
  nasPackageV2: true,
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
  authoringV2Shadow: boolean;
  qualityGateV2: boolean;
}

export const DEFAULT_PHASE1_FEATURE_FLAGS: Readonly<Phase1FeatureFlags> = Object.freeze({
  ...DEFAULT_PHASE0_FEATURE_FLAGS,
  documentIrV2Shadow: true,
  authoringV2Shadow: true,
  qualityGateV2: true
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

export interface Phase5FeatureFlags extends Phase1FeatureFlags {
  authoringEditorV2: boolean;
}

export const DEFAULT_PHASE5_FEATURE_FLAGS: Readonly<Phase5FeatureFlags> = Object.freeze({
  ...DEFAULT_PHASE1_FEATURE_FLAGS,
  authoringEditorV2: true
});

export type Phase5FeatureFlagName = keyof Phase5FeatureFlags;

export function resolvePhase5FeatureFlags(
  overrides: Partial<Phase5FeatureFlags> = {}
): Phase5FeatureFlags {
  return {
    ...DEFAULT_PHASE5_FEATURE_FLAGS,
    ...overrides,
    pdfPerQuestionLlmRepair: false
  };
}

export function isPhase5EditorEnabled(): boolean {
  return true;
}
