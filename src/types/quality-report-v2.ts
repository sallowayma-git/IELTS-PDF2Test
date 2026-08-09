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

export interface QualityReportV2 {
  schemaVersion: "QualityReportV2";
  state: ReadinessState;
  documentScore: number;
  sourceCoverage: number;
  taskScores: Record<string, number>;
  hardFailures: string[];
  issues: ReviewIssueV2[];
  metrics: Record<string, number>;
  evaluatedAt: string;
  evaluatorVersion: string;
}

export const QUALITY_REPORT_V2_SCHEMA_VERSION = "QualityReportV2" as const;

