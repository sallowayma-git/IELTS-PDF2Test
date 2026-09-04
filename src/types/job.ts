export type JobStatus =
  | "Working"
  | "NeedsReview"
  | "DraftSaved"
  | "ExportReady"
  | "Exported"
  | "Cleaned";

export type WorkflowStep =
  | "Upload"
  | "DocumentReview"
  | "Split"
  | "Authoring"
  | "LlmReview"
  | "Preview"
  | "Export"
  | "Pack";

export type PassageCategory = "P1" | "P2" | "P3";
export type Frequency = "low" | "medium" | "high";
export type SourceFileRole = "MainQuestion" | "AnswerKey" | "Explanation" | "Asset";
export type SourceFileType = "pdf" | "docx" | "txt" | "md" | "image" | "unknown";

export interface IssueCounts {
  errors: number;
  warnings: number;
  needsReview: number;
}

export interface SourceFile {
  fileId: string;
  originalName: string;
  storedName: string;
  fileType: SourceFileType;
  sha256: string;
  sizeBytes: number;
  role: SourceFileRole;
  importedAt: string;
}

export interface ImportJob {
  jobId: string;
  title: string;
  status: JobStatus;
  category?: PassageCategory;
  frequency?: Frequency;
  tags: string[];
  sourceFiles: SourceFile[];
  activeLlmProfileId?: string;
  createdAt: string;
  updatedAt: string;
  currentStep: WorkflowStep;
  issueCounts: IssueCounts;
}

export interface CreateJobInput {
  title?: string;
  category?: PassageCategory;
  frequency?: Frequency;
  tags?: string[];
  llmProfileId?: string;
}

export interface JobFilter {
  status?: JobStatus;
  search?: string;
}

export interface JobMetaPatch {
  title?: string;
  category?: PassageCategory;
  frequency?: Frequency;
  tags?: string[];
  activeLlmProfileId?: string;
}

export interface VisionAnswerCandidate {
  questionNumber: string;
  questionId?: string | null;
  answer: unknown;
  confidence?: number | null;
  evidence?: {
    questionNumber?: string | number;
    pageIndex?: number;
    quote?: string;
  } | null;
}

export interface VisionAnswerCandidates {
  schemaVersion: string;
  jobId: string;
  profileId?: string | null;
  candidateCount: number;
  candidates: VisionAnswerCandidate[];
}
