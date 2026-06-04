import type { Frequency, PassageCategory, SourceFile } from "./job";

export type GroupKind =
  | "single_choice"
  | "multi_choice"
  | "true_false_not_given"
  | "yes_no_not_given"
  | "matching"
  | "heading_matching"
  | "matching_information"
  | "classification"
  | "summary_completion"
  | "table_completion"
  | "diagram_completion"
  | "short_answer"
  | "sentence_completion";

export type InteractionType = "radio" | "checkbox" | "text" | "textarea" | "select" | "dragdrop" | "table" | "diagram" | "matching";
export type AnswerValue = string | string[];

export interface ExamMetaDraft {
  examId: string;
  title: string;
  category: PassageCategory;
  frequency: Frequency;
  tags: string[];
  sourceFiles?: SourceFile[];
}

export interface PassageDraft {
  title: string;
  htmlBlocks: Array<{ blockId: string; html: string }>;
  sourceBlockIds: string[];
  questionUmbrellaRanges?: QuestionUmbrellaRange[];
}

export interface QuestionUmbrellaRange {
  heading: string;
  questionRange: [number, number];
  blockId: string;
  text: string;
}

export interface InteractionSpec {
  type: InteractionType;
  options?: string[];
  placeholder?: string;
  allowOptionReuse?: boolean;
  minSelections?: number;
  maxSelections?: number;
}

export interface LayoutSpec {
  template: string;
  layoutHint?: "inline_completion" | "table" | "list";
  tableHeaders?: string[];
  notes?: string;
}

export interface QuestionDraft {
  id: string;
  displayNumber: string;
  prompt: string;
  interaction: InteractionSpec;
  answer?: AnswerValue;
  sourceBlockIds: string[];
  confidence: number;
  verified: boolean;
  requiresManualQuestionImport?: boolean;
}

export interface QuestionGroupDraft {
  groupId: string;
  kind: GroupKind;
  questionRange?: [number, number];
  instruction: string[];
  questions: QuestionDraft[];
  layout: LayoutSpec;
  reviewWarnings?: string[];
  classificationEvidence?: string[];
  sectionEvidence?: SplitSectionEvidence[];
  continuationEdges?: SplitContinuationEdge[];
  allowOptionReuse?: boolean;
  sourceBlockIds: string[];
  confidence: number;
  verified: boolean;
  isUmbrellaRange?: boolean;
  requiresManualQuestionImport?: boolean;
  llmReview?: {
    required: boolean;
    status: "low_confidence" | "auto_apply_blocked" | string;
    confidence: number;
    suggestionId?: string | null;
    suggestedKind?: string | null;
    warnings?: string[];
    evidence?: unknown;
    recordedAt?: string;
  };
}

export interface AuthoringAudit {
  llmUsed: boolean;
  humanVerified: boolean;
  issues: string[];
  revision: number;
  updatedAt: string;
}

export interface ReadingAuthoringIr {
  schemaVersion: "ReadingAuthoringIRV1";
  jobId: string;
  exam: ExamMetaDraft;
  passage: PassageDraft;
  groups: QuestionGroupDraft[];
  answerKey: Record<string, AnswerValue>;
  questionOrder: string[];
  questionDisplayMap: Record<string, string>;
  audit: AuthoringAudit;
}

export interface SplitGroupCandidate {
  groupId: string;
  heading: string;
  questionRange: [number, number];
  instructionText: string;
  blockIds: string[];
  kindHint?: GroupKind;
  layoutHint?: "inline_completion" | "table" | "list";
  confidence: number;
  classification?: {
    kind: GroupKind;
    interaction: InteractionSpec;
    confidence: number;
    warnings: string[];
    evidence: string[];
  };
  sectionEvidence?: SplitSectionEvidence[];
  continuationEdges?: SplitContinuationEdge[];
  isUmbrellaRange?: boolean;
  requiresManualQuestionImport?: boolean;
}

export interface SplitSectionEvidence {
  blockId: string;
  pageIndex: number;
  column: number;
  role: string;
  textPreview: string;
  bbox?: [number, number, number, number];
  normalizedBbox?: [number, number, number, number];
  pageRotation?: number;
  tableRows?: number;
  tableCols?: number;
  tableHasColSpans?: boolean;
  tableHasVerticalMerges?: boolean;
  tableMergedCellCount?: number;
  headingLevel?: number;
  numberingLevel?: number;
  numberingId?: string;
}

export interface SplitContinuationEdge {
  fromBlockId: string;
  toBlockId: string;
  reason: string;
  confidence: number;
}

export interface SplitCandidates {
  jobId: string;
  passageCandidates: Array<{ range: string[]; title: string; categoryHint: PassageCategory }>;
  questionGroupCandidates: SplitGroupCandidate[];
  umbrellaQuestionRanges?: Array<{
    heading: string;
    questionRange: [number, number];
    blockId: string;
    text: string;
  }>;
  answerKeyCandidates: Array<{ source: string; answers: Record<string, AnswerValue> }>;
  issues: string[];
}

export interface AuthoringPatch {
  ir: ReadingAuthoringIr;
}
