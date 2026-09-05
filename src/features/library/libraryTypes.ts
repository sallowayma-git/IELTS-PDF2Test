import type { ImportJob, IssueCounts, JobStatus, LibraryExamSummary, LibraryStatus, WorkflowStep } from "../../types";

// 题库行的统一模型（计划 §2.1 / §11.3）。
// 用户只看到一个简短阶段，不看到 JobStatus / WorkflowStep / LibraryStatus 三套内部枚举。
export type LibraryStageV1 =
  | "queued"
  | "local"
  | "cloud"
  | "reconciling"
  | "action_required"
  | "ready"
  | "published"
  | "failed";

export const STAGE_LABEL: Record<LibraryStageV1, string> = {
  queued: "排队中",
  local: "本地识别",
  cloud: "云端识别",
  reconciling: "合并结果",
  action_required: "待检查",
  ready: "可发布",
  published: "已发布",
  failed: "失败"
};

/** 用户可选的筛选面，比内部阶段更粗。 */
export type LibraryFilterTab = "all" | "processing" | "action_required" | "ready" | "failed" | "trash";

export const FILTER_TAB_LABEL: Record<LibraryFilterTab, string> = {
  all: "全部",
  processing: "处理中",
  action_required: "待检查",
  ready: "可发布",
  failed: "失败",
  trash: "回收站"
};

const PROCESSING_STAGES: readonly LibraryStageV1[] = ["queued", "local", "cloud", "reconciling"];

export function isProcessingStage(stage: LibraryStageV1): boolean {
  return PROCESSING_STAGES.includes(stage);
}

export interface LibraryRowV1 {
  id: string;
  title: string;
  modality: "reading" | "writing";
  stage: LibraryStageV1;
  /** 一行人话说明，例如「本地识别完成 · 云端识别中」或「本地 PDF 无法读取」。 */
  detail?: string;
  /** 只有处理中的行有进度；使用阶段权重而不是编造的百分比（计划 §12.3）。 */
  progressPercent?: number;
  actionableCount: number;
  category?: string;
  updatedAt: string;
  inTrash: boolean;
  /** 仅供开发者排查，不渲染在行上。 */
  raw: {
    jobStatus?: JobStatus;
    currentStep?: WorkflowStep;
    libraryStatus?: LibraryStatus;
    issueCounts?: IssueCounts;
  };
}

// 阶段权重来自计划 §12.3；本地链路还没有细分事件，这里按 WorkflowStep 的可观测节点取值。
const STEP_PROGRESS: Record<WorkflowStep, number> = {
  Upload: 5,
  DocumentReview: 25,
  Split: 45,
  Authoring: 60,
  LlmReview: 80,
  Preview: 90,
  Export: 100,
  Pack: 100
};

const STEP_STAGE: Record<WorkflowStep, LibraryStageV1> = {
  Upload: "queued",
  DocumentReview: "local",
  Split: "local",
  Authoring: "local",
  LlmReview: "cloud",
  Preview: "reconciling",
  Export: "ready",
  Pack: "ready"
};

function actionableFrom(counts: IssueCounts | undefined): number {
  if (!counts) return 0;
  return counts.errors + counts.needsReview;
}

/** 把内部 JobStatus + WorkflowStep + LibraryStatus 折叠为一个用户可读阶段。 */
export function deriveStage(job: ImportJob | undefined, summary: LibraryExamSummary | undefined): LibraryStageV1 {
  const actionable = actionableFrom(job?.issueCounts);
  if (job) {
    switch (job.status) {
      case "Working":
        return STEP_STAGE[job.currentStep];
      case "NeedsReview":
        return "action_required";
      case "DraftSaved":
        return actionable > 0 ? "action_required" : "ready";
      case "ExportReady":
        return "ready";
      case "Exported":
      case "Cleaned":
        return "published";
    }
  }
  switch (summary?.status) {
    case "needs_review":
      return "action_required";
    case "ready":
      return "ready";
    case "exported":
      return "published";
    case "draft":
      return actionable > 0 ? "action_required" : "ready";
    default:
      return "queued";
  }
}

function detailFor(stage: LibraryStageV1, job: ImportJob | undefined, actionable: number): string | undefined {
  if (stage === "local") return "正在读取原文件并识别题目";
  if (stage === "cloud") return "本地识别完成 · 云端识别中";
  if (stage === "reconciling") return "正在合并本地与云端结果";
  if (stage === "queued") return "等待开始识别";
  if (stage === "action_required") return actionable > 0 ? `${actionable} 处需要确认` : "有内容需要确认";
  if (stage === "failed") return "识别失败，可以重试";
  if (stage === "published") return job?.status === "Cleaned" ? "已发布并清理过程文件" : "已发布";
  return undefined;
}

export function buildRow(
  id: string,
  job: ImportJob | undefined,
  summary: LibraryExamSummary | undefined,
  options: { inTrash?: boolean } = {}
): LibraryRowV1 {
  const stage = deriveStage(job, summary);
  const actionable = actionableFrom(job?.issueCounts) || (summary?.issueErrors ?? 0);
  return {
    id,
    title: job?.title ?? summary?.title ?? id,
    modality: summary?.subject === "writing" ? "writing" : "reading",
    stage,
    detail: detailFor(stage, job, actionable),
    progressPercent: job && isProcessingStage(stage) ? STEP_PROGRESS[job.currentStep] : undefined,
    actionableCount: actionable,
    category: summary?.category ?? job?.category,
    updatedAt: job?.updatedAt ?? summary?.updatedAt ?? "",
    inTrash: Boolean(options.inTrash),
    raw: {
      jobStatus: job?.status,
      currentStep: job?.currentStep,
      libraryStatus: summary?.status,
      issueCounts: job?.issueCounts
    }
  };
}

export function matchesTab(row: LibraryRowV1, tab: LibraryFilterTab): boolean {
  if (tab === "trash") return row.inTrash;
  if (row.inTrash) return false;
  if (tab === "all") return true;
  if (tab === "processing") return isProcessingStage(row.stage);
  if (tab === "action_required") return row.stage === "action_required";
  if (tab === "ready") return row.stage === "ready" || row.stage === "published";
  return row.stage === "failed";
}

export function matchesSearch(row: LibraryRowV1, query: string): boolean {
  const trimmed = query.trim().toLowerCase();
  if (!trimmed) return true;
  return row.title.toLowerCase().includes(trimmed) || row.id.toLowerCase().includes(trimmed);
}
