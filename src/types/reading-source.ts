import type { AnswerValue, GroupKind } from "./authoring-ir";
import type { Frequency, PassageCategory } from "./job";

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
}

export interface ExportResult {
  examId: string;
  files: Array<{ name: string; content: string }>;
  outputDir?: string;
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
  exportSummary?: unknown;
  cleanup?: Array<{
    cleaned?: boolean;
    retainedFullProcessArtifacts?: boolean;
    message?: string;
    [key: string]: unknown;
  }>;
}

export interface ExportReadingJsInput {
  jobIds: string[];
  exportDir?: string;
}

export interface ExportNasLibraryInput {
  jobIds: string[];
  exportDir?: string;
  version?: string;
}

export interface NasExportResult {
  mode: "nas-library";
  jobIds: string[];
  examIds: string[];
  assetCount: number;
  libraryRoot?: string;
  sourceDir?: string;
  publishDir?: string;
  version?: string;
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

export interface BuildPackInput {
  packId: string;
  version: string;
  institution: string;
  description: string;
  validFrom?: string;
  validTo?: string;
  jobIds: string[];
}

export interface PackBuildResult {
  packId: string;
  outputPath: string;
  files: string[];
  zipSizeBytes?: number;
  entryCount?: number;
  manifest?: unknown;
  exportSummary?: unknown;
  cleanup?: unknown;
  createdAt?: string;
}
