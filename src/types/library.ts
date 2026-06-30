// 题库管理（library）类型 —— 对应后端 lib.rs 中的 Library* 结构。
// subject 区分阅读/写作；status 用统一枚举（draft|needs_review|ready|exported）。

export type LibrarySubject = "reading" | "writing";

/** 题库统一状态枚举（映射自阅读 JobStatus 与写作 WritingJobStatus）。 */
export type LibraryStatus = "draft" | "needs_review" | "ready" | "exported";

export interface LibraryFilter {
  subject?: LibrarySubject;
  status?: LibraryStatus;
  category?: string;
  limit?: number;
  offset?: number;
}

/** 题库列表项（不含题目正文 payload，列表/搜索用）。 */
export interface LibraryExamSummary {
  id: string;
  examId?: string;
  title: string;
  subject: LibrarySubject;
  category?: string;
  frequency?: string;
  status: LibraryStatus;
  taskType?: string;
  tags: string[];
  sourceHash?: string;
  issueErrors: number;
  issueWarnings: number;
  createdAt: string;
  updatedAt: string;
}

/** 题库详情项 = summary + payload（题目正文整体）。
 *  payload 对阅读是 ReadingAuthoringIr 或 ImportJob 序列化；对写作是 WritingJob 序列化。 */
export interface LibraryExamDetail {
  summary: LibraryExamSummary;
  payload: unknown;
}

export interface LibraryMetaPatch {
  title?: string;
  category?: string;
  frequency?: string;
  status?: LibraryStatus;
  taskType?: string;
  tags?: string[];
}

export interface LibraryStats {
  total: number;
  bySubject: Record<string, number>;
  byStatus: Record<string, number>;
  byCategory: Record<string, number>;
}
