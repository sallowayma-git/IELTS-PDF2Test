import type {
  ListeningAudioIssueCodeV1,
  ListeningAudioProbeV1
} from "./listening-runtime-v1";

export const LISTENING_AUDIO_PROBE_RESULT_V1_SCHEMA_VERSION = "ListeningAudioProbeResultV1" as const;

export interface ListeningAudioProbePolicyV1 {
  supportedMimes: Array<"audio/wav" | "audio/mpeg" | "audio/mp4" | string>;
  nearSilentRmsThreshold: number;
  severeClippingSampleRatio: number;
}

export interface ListeningAudioSignalMetricsV1 {
  decodedSampleCount: number;
  peakAmplitude: number;
  rmsAmplitude: number;
  clippedSampleRatio: number;
}

export interface ListeningAudioProbeResultV1 {
  schemaVersion: typeof LISTENING_AUDIO_PROBE_RESULT_V1_SCHEMA_VERSION;
  fileName: string;
  byteLength: number;
  sha256: string;
  mime?: string;
  container?: string;
  codec?: string;
  durationMs?: number;
  channels?: number;
  sampleRateHz?: number;
  signal?: ListeningAudioSignalMetricsV1;
  probe: ListeningAudioProbeV1;
  details: string[];
}

export const DEFAULT_LISTENING_AUDIO_PROBE_POLICY_V1: Readonly<ListeningAudioProbePolicyV1> = Object.freeze({
  supportedMimes: ["audio/wav", "audio/mpeg", "audio/mp4"],
  nearSilentRmsThreshold: 0.001,
  severeClippingSampleRatio: 0.01
});

export function blockingListeningAudioIssueCodes(result: ListeningAudioProbeResultV1): ListeningAudioIssueCodeV1[] {
  return result.probe.status === "passed" ? [] : [...result.probe.issueCodes];
}
