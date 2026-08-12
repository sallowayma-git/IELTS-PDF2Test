import type { SourceAnchorV2 } from "./schema-common-v2";

export type ReviewSeverity = "info" | "warning" | "blocking";
export type ReadinessState = "ready" | "review_required" | "blocked";

export type ReviewTargetType = "document" | "page" | "region" | "task" | "response_group" | "slot" | "asset";

export type SuggestedAction =
  | "assign_role"
  | "edit_text"
  | "merge_lines"
  | "split_prompt"
  | "attach_option_bank"
  | "confirm_table"
  | "confirm_figure"
  | "replace_asset"
  | "enter_answer"
  | "ignore_with_reason";

export interface ReviewIssueV2 {
  issueId: string;
  code: string;
  severity: ReviewSeverity;
  message: string;
  targetType: ReviewTargetType;
  targetId: string;
  sourceAnchors: SourceAnchorV2[];
  suggestedActions: SuggestedAction[];
  details?: Record<string, unknown>;
}

export interface QualityCoverageEntryV2 {
  sourceNodeId: string;
  significant: boolean;
  disposition: "assigned" | "ignored_with_reason" | "unassigned";
  targetIds: string[];
  reason?: string;
}

export interface CoverageStatusV2 {
  physicalShadow: "available" | "missing";
  complete: boolean;
  significantSourceNodeCount: number;
  explainedSourceNodeCount: number;
  unassignedSourceNodeIds: string[];
}

export interface CompilerProbeV2 {
  status: "passed" | "failed";
  schemaVersion: string;
  issueCodes: string[];
  details: string[];
}

export interface CompilerProbesV2 {
  v2Runtime: CompilerProbeV2;
  v1Compatibility: CompilerProbeV2;
}

export interface QualityReportV2 {
  schemaVersion: "QualityReportV2";
  state: ReadinessState;
  documentScore: number;
  sourceCoverage: number;
  coverageLedger: QualityCoverageEntryV2[];
  coverageStatus: CoverageStatusV2;
  compilerProbes: CompilerProbesV2;
  taskScores: Record<string, number>;
  hardFailures: string[];
  issues: ReviewIssueV2[];
  metrics: Record<string, number>;
  evaluatedAt: string;
  evaluatorVersion: string;
}

export const QUALITY_REPORT_V2_SCHEMA_VERSION = "QualityReportV2" as const;
