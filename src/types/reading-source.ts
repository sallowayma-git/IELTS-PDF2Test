import type { AnswerValue, GroupKind } from "./authoring-ir";
import type { Frequency, PassageCategory } from "./job";
import type { ValidationIssue } from "./validation";

export type IgnoredValidationIssue = ValidationIssue & { jobId: string };

export interface ReadingExamSourceV1 {
  schemaVersion: "ReadingExamSourceV1";
  examId: string;
  meta: {
    title: string;
    category: PassageCategory;
    frequency: Frequency;
    pdfFilename: string;
    legacyPath: string;
    legacyFilename: string;
    questionIntroHtml: string;
    questionUmbrellaRanges?: Array<{
      heading: string;
      questionRange: [number, number];
      blockId: string;
      text: string;
    }>;
  };
  passage: {
    blocks: Array<{ blockId: string; kind: "html"; html: string }>;
  };
  questionGroups: Array<{
    groupId: string;
    kind: GroupKind;
    questionIds: string[];
    bodyHtml: string;
    leadHtml: string;
    allowOptionReuse?: boolean;
  }>;
  answerKey: Record<string, AnswerValue>;
  sourceRefs: {
    primaryHtml: string;
    primaryProvider: "author_web";
    shuiHtml: null;
    shuiPdf: string;
    ieltsHtml: null;
  };
  audit: {
    matchStatus: "author_verified" | "needs_review";
    matchConfidence: number;
    verifiedAt: string | null;
    notes: string;
  };
  questionOrder: string[];
  questionDisplayMap: Record<string, string>;
}

export interface PreviewAssets {
  examId: string;
  manifestPath: string;
  scriptPath: string;
  previewUrl: string;
  source: ReadingExamSourceV1;
  wrapperJs: string;
  manifestJs: string;
  runtimeHtml?: string | null;
}

export interface ExportResult {
  examId: string;
  files: Array<{ name: string; content: string }>;
  outputDir?: string;
  validationPolicy?: ValidationPolicy;
  validationOverridden?: boolean;
  ignoredIssueCount?: number;
  ignoredIssues?: IgnoredValidationIssue[];
  exportSummary?: unknown;
  cleanup?: {
    cleaned?: boolean;
    retainedFullProcessArtifacts?: boolean;
    message?: string;
    [key: string]: unknown;
  };
}

export interface JsExportResult {
  mode: "single" | "batch";
  examIds: string[];
  jobIds: string[];
  files: Array<{ name: string; content: string }>;
  outputDir?: string;
  validationPolicy?: ValidationPolicy;
  validationOverridden?: boolean;
  ignoredIssueCount?: number;
  ignoredIssues?: IgnoredValidationIssue[];
  exportSummary?: unknown;
  cleanup?: Array<{
    cleaned?: boolean;
    retainedFullProcessArtifacts?: boolean;
    message?: string;
    [key: string]: unknown;
  }>;
}
export type ValidationPolicy = "strict" | "force";

export interface ExportReadingJsInput {
  jobIds: string[];
  exportDir?: string;
  validationPolicy?: ValidationPolicy;
}

export interface ExportNasLibraryInput {
  jobIds: string[];
  exportDir?: string;
  version?: string;
  validationPolicy?: ValidationPolicy;
}

export interface NasExportResult {
  mode: "nas-library";
  jobIds: string[];
  examIds: string[];
  assetCount: number;
  manifestAssetCount?: number;
  libraryRoot?: string;
  readingExamsDir?: string;
  sourceDir?: string;
  publishDir?: string;
  version?: string;
  validationPolicy?: ValidationPolicy;
  validationOverridden?: boolean;
  ignoredIssueCount?: number;
  ignoredIssues?: IgnoredValidationIssue[];
  files: Array<{ name: string; content: string }>;
  report?: unknown;
  exportSummary?: unknown;
  cleanup?: Array<{
    cleaned?: boolean;
    retainedFullProcessArtifacts?: boolean;
    message?: string;
    [key: string]: unknown;
  }>;
}

export interface ExportNasPackageV2Input {
  libraryRoot: string;
  sourcePath: string;
  assetRoot?: string;
  examId?: string;
  minimumRuntimeVersion?: string;
  expectedManifestSha256?: string;
  fault?: "after_assets" | "after_source" | "before_manifest" | "manifest_rename";
}

export interface NasPackageV2PublishResult {
  schemaVersion: "NasPackagePublishReportV2";
  status: "committed";
  examId: string;
  manifestPath: string;
  reportPath: string;
  assetCount: number;
  exportId: string;
  probe: {
    passed: boolean;
    checkedAssetIds: string[];
    referencedAssetIds: string[];
    errors: string[];
    warnings: string[];
  };
}
