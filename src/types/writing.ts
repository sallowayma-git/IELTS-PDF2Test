/** 写作任务类型（与 Rust writing_store::WritingJob 镜像）。 */
export type WritingTaskType = "task1" | "task2";

/** 写作任务状态（比阅读简单：无 PDF 解析中间态）。 */
export type WritingJobStatus = "Draft" | "ExportReady" | "Exported";

/** 写作创作任务（手输 prompt，无 passage/questionGroups/answerKey）。 */
export interface WritingJob {
  jobId: string;
  title: string;
  taskType: WritingTaskType;
  examId: string;
  promptText: string;
  suggestedWordCount: number;
  status: WritingJobStatus;
  createdAt: string;
  updatedAt: string;
}

export interface CreateWritingJobInput {
  title?: string;
  taskType?: WritingTaskType;
  promptText?: string;
  suggestedWordCount?: number;
}

/** 部分更新（所有字段可选）。 */
export interface WritingJobPatch {
  title?: string;
  taskType?: WritingTaskType;
  examId?: string;
  promptText?: string;
  suggestedWordCount?: number;
  status?: WritingJobStatus;
}

export interface WritingJobFilter {
  taskType?: WritingTaskType;
  search?: string;
}

/**
 * 导出用 source（与 NAS 端 ExamWritingTaskPayload 契约对齐）。
 * schemaVersion 固定 "WritingExamSourceV1"。
 */
export interface WritingExamSourceV1 {
  schemaVersion: "WritingExamSourceV1";
  examId: string;
  taskType: WritingTaskType;
  promptText: string;
  suggestedWordCount: number;
  meta: { title: string; taskType: WritingTaskType };
}

/** 写作导出结果（与 Rust export_writing_library_core 返回对齐）。 */
export interface WritingExportResult {
  mode: "writing-library";
  jobIds: string[];
  taskTypes: WritingTaskType[];
  assetCount: number;
  libraryRoot: string;
  writingExamsDir: string;
  version: string;
  files: Array<{ name: string; content: string }>;
  report: {
    status: string;
    version: string;
    generatedAt: string;
    summary: { runtime: string; writingTaskCount: number; manifestFileCount: number };
    errors: unknown[];
  };
  exportSummary: {
    type: "writing-library";
    runtime: string;
    jobIds: string[];
    version: string;
    outputDir: string;
    writingExamsDir: string;
    assetCount: number;
    exportedAt: string;
  };
  cleanup: Array<{ jobId: string; taskType: WritingTaskType; status: string }>;
}

/** 导出输入。 */
export interface ExportWritingLibraryInput {
  jobIds: string[]; // [task1JobId, task2JobId]
  exportDir?: string;
}
