import { command } from "./tauriCommands";

// 「数据」模块的 Tauri 命令客户端（后端：src-tauri/src/analytics）。
// 只读学生端作答快照；这里的类型与 Rust 侧 serde camelCase 输出一一对应。

export interface ImportOutcome {
  file: string;
  status: "ok" | "skipped" | "failed" | string;
  attemptId: string | null;
  attemptsImported: number;
  error: string | null;
}

export interface ImportReport {
  scanned: number;
  ok: number;
  skipped: number;
  failed: number;
  attemptsImported: number;
  outcomes: ImportOutcome[];
  finishedAt: string;
  snapshotsDirFound: boolean;
}

export interface ImportRecord {
  file: string;
  status: string;
  terminalId: string | null;
  attemptId: string | null;
  batchId: string | null;
  paperId: string | null;
  exportedAt: string | null;
  importedAt: string;
  error: string | null;
}

export interface BatchRow {
  batchId: string;
  /** 空串表示旧套题（没有 paperId）。 */
  paperId: string;
  checkedIn: number;
  inProgress: number;
  submittedBeacon: number;
  voided: number;
  imported: number;
  validScored: number;
  importExceptions: number;
  lastSubmittedAt: string | null;
}

export interface IdentityAttempt {
  attemptId: string;
  candidateId: string;
  candidateName: string;
  batchId: string;
  paperId: string;
}

export interface IdentityIssue {
  kind: "same_id_diff_name" | "same_name_diff_id" | string;
  candidateId: string;
  candidateName: string;
  attempts: IdentityAttempt[];
}

export interface Overview {
  batches: BatchRow[];
  exceptions: ImportRecord[];
  identityIssues: IdentityIssue[];
  beaconErrors: string[];
  beaconCount: number;
  lastImportAt: string | null;
  disclaimer: string;
}

export interface ExcludedAttempt {
  attemptId: string;
  candidateName: string;
  reason: string;
}

export interface ScoreSummary {
  mean: number | null;
  median: number | null;
  max: number | null;
  min: number | null;
  meanAccuracy: number | null;
  maxTotal: number | null;
}

export interface HistBin {
  from: number;
  to: number;
  count: number;
}

export interface PartStat {
  passageIndex: number;
  label: string;
  n: number;
  correct: number;
  total: number;
  accuracy: number | null;
}

export interface KindStat {
  kind: string;
  n: number;
  points: number;
  correctPoints: number;
  accuracy: number | null;
}

export interface QuestionStat {
  passageIndex: number;
  passageLabel: string;
  questionId: string;
  displayLabel: string;
  questionKind: string;
  n: number;
  correctCount: number;
  accuracy: number | null;
  hardConclusion: boolean;
}

export interface MatrixRow {
  attemptId: string;
  candidateName: string;
  /** 与 questions 一一对应：true 对、false 错、null 无判分记录。 */
  cells: Array<boolean | null>;
}

export interface StudentRow {
  attemptId: string;
  candidateId: string;
  candidateName: string;
  rawScore: number;
  rawTotal: number;
  accuracy: number | null;
  /** 与 parts 一一对应。 */
  partCorrect: Array<number | null>;
  submittedAt: string | null;
  totalDurationSec: number | null;
}

export interface ExamView {
  batchId: string;
  paperId: string;
  n: number;
  excluded: ExcludedAttempt[];
  hardConclusionMinN: number;
  disclaimer: string;
  score: ScoreSummary;
  histogram: HistBin[];
  parts: PartStat[];
  kinds: KindStat[];
  hardQuestions: QuestionStat[];
  questions: QuestionStat[];
  matrix: MatrixRow[];
  students: StudentRow[];
}

export interface StudentPassage {
  passageIndex: number;
  label: string;
  title: string | null;
  correct: number;
  total: number;
  accuracy: number | null;
  durationSec: number | null;
  startedAt: string | null;
  endedAt: string | null;
  overlap: boolean;
}

export interface WrongAnswer {
  passageIndex: number;
  passageLabel: string;
  questionId: string;
  displayLabel: string;
  questionKind: string;
  userAnswer: string;
  correctAnswer: string;
  weight: number;
  elapsedMs: number;
  changeCount: number;
  visitCount: number;
}

export interface WritingRow {
  taskType: string;
  wordCount: number | null;
  durationSec: number | null;
  submittedAt: string | null;
}

export interface StudentView {
  attemptId: string;
  batchId: string;
  paperId: string;
  candidateId: string;
  candidateName: string;
  submittedAt: string | null;
  valid: boolean;
  invalidReason: string | null;
  rawScore: number;
  rawTotal: number;
  accuracy: number | null;
  passages: StudentPassage[];
  overlapNote: string | null;
  wrongAnswers: WrongAnswer[];
  writings: WritingRow[];
  disclaimer: string;
}

export interface AnswerBucket {
  answer: string;
  count: number;
  share: number | null;
  isCorrect: boolean | null;
}

export interface QuestionView {
  batchId: string;
  paperId: string;
  passageIndex: number;
  passageLabel: string;
  questionId: string;
  displayLabel: string;
  questionKind: string;
  n: number;
  correctCount: number;
  accuracy: number | null;
  hardConclusion: boolean;
  correctAnswer: string;
  avgElapsedMs: number | null;
  avgChangeCount: number | null;
  distribution: AnswerBucket[];
  disclaimer: string;
}

export interface ExportSummary {
  path: string;
  sheets: string[];
  n: number;
}

export const analyticsApi = {
  importSnapshots: (submissionsRoot: string) =>
    command<ImportReport>("analytics_import_snapshots", { submissionsRoot }),
  overview: (submissionsRoot: string) => command<Overview>("analytics_overview", { submissionsRoot }),
  exam: (batchId: string, paperId: string) => command<ExamView>("analytics_exam", { batchId, paperId }),
  student: (attemptId: string) => command<StudentView>("analytics_student", { attemptId }),
  question: (batchId: string, paperId: string, passageIndex: number, questionId: string) =>
    command<QuestionView>("analytics_question", { batchId, paperId, passageIndex, questionId }),
  exportXlsx: (batchId: string, paperId: string) =>
    command<ExportSummary | null>("analytics_export_xlsx", { batchId, paperId })
};
