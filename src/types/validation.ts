export type Severity = "error" | "warning" | "info";
export type ValidationLayer = "AuthoringIR" | "ReadingExamSourceV1" | "DomProtocol" | "RuntimePreview";

export interface ValidationIssue {
  issueId: string;
  severity: Severity;
  layer: ValidationLayer;
  path: string;
  message: string;
  fixHint?: string;
}

export interface ValidationLayerReport {
  layer: ValidationLayer;
  passed: boolean;
  issueCount: number;
}

export interface ValidationReport {
  jobId: string;
  passed: boolean;
  layers: ValidationLayerReport[];
  issues: ValidationIssue[];
  generatedAt: string;
  runtime?: {
    adapter?: string;
    examId?: string;
    jobId?: string;
    registeredIds?: string[];
    navButtonCount?: number;
    questionCount?: number;
    collectedAnswers?: Record<string, unknown>;
    scoreInfo?: unknown;
    wrongScoreInfo?: unknown;
    consoleErrors?: string[];
  };
}
