import type { LibraryItemSummaryV2 } from "../../api/workspaceClient";
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
  reconciling: "云端检查",
  // 识别完成就是识别完成：不再按质量状态分成「待检查 / 可发布」两种结论——
  // 发布前的检查由用户按「发布」时一并完成，题库行不替他下结论。
  action_required: "识别完成",
  ready: "识别完成",
  published: "已发布",
  failed: "失败"
};

/** 用户可选的筛选面，比内部阶段更粗。 */
export type LibraryFilterTab = "all" | "processing" | "action_required" | "ready" | "failed" | "trash";

export const FILTER_TAB_LABEL: Record<LibraryFilterTab, string> = {
  all: "全部",
  processing: "处理中",
  action_required: "待检查",
  ready: "已完成",
  failed: "失败",
  trash: "回收站"
};

const PROCESSING_STAGES: readonly LibraryStageV1[] = ["queued", "local", "cloud", "reconciling"];

export function isProcessingStage(stage: LibraryStageV1): boolean {
  return PROCESSING_STAGES.includes(stage);
}

export type LibraryModality = "reading" | "listening" | "writing";

/** The backend item row is the modality authority; the legacy summary only knows reading/writing. */
export function rowModality(summary: LibraryExamSummary | undefined, v2?: LibraryItemSummaryV2): LibraryModality {
  if (v2?.modality === "listening") return "listening";
  if (summary?.subject === "writing" || v2?.modality === "writing") return "writing";
  return "reading";
}

export interface LibraryRowV1 {
  id: string;
  title: string;
  modality: LibraryModality;
  stage: LibraryStageV1;
  /** 一行人话说明，例如「本地识别完成 · 云端识别中」或「本地 PDF 无法读取」。 */
  detail?: string;
  /** 只有处理中的行有进度；使用阶段权重而不是编造的百分比（计划 §12.3）。 */
  progressPercent?: number;
  actionableCount: number;
  category?: string;
  updatedAt: string;
  inTrash: boolean;
  /** 已发布，且那次发布时检查没有全部通过、由用户点击发布放行（后端 `published_forced`）。 */
  publishedForced: boolean;
  /**
   * 已发布，但**学生端打不开这道题**（后端 `published_not_loadable` /
   * `published_forced_not_loadable`）：授权快照发出去了，包却没装进学生清单。
   * 题库行必须让用户看出这件事，否则他会以为学生已经能做了。
   */
  publishedNotLoadable: boolean;
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

function detailFor(
  stage: LibraryStageV1,
  job: ImportJob | undefined,
  actionable: number,
  v2?: LibraryItemSummaryV2
): string | undefined {
  if (stage === "local") return "正在读取原文件并识别题目";
  if (stage === "cloud") return "本地识别完成 · 云端识别中，可先打开编辑";
  // 这一段就是最长十分钟的云端自动检查（修复循环）；「正在合并」让人以为要等它。
  if (stage === "reconciling") return "云端正在自动检查，可先打开编辑";
  if (stage === "queued") return "等待开始识别";
  if (stage === "action_required") {
    // G1/A4-F03：恢复上限路径不得谎称"已自动排队重试"，按真实错误码给出人话。
    const errorCode = v2?.processing?.lastErrorCode;
    if (errorCode === "retry_exhausted") return "已达到自动恢复上限，请手动重试";
    if (errorCode === "interrupted") return "已自动排队重试";
    return "可以打开编辑";
  }
  if (stage === "failed") {
    // G1 对抗审计：用户主动取消的行不得显示成"识别失败，可以重试"。
    if (v2?.processing?.stage === "cancelled") return "已取消";
    return "识别失败，可以重试";
  }
  if (stage === "published") {
    // 打不开这件事必须先说：光说「已发布」会让用户在题库里以为学生已经能做这道题。
    if (v2?.status === "published_not_loadable") return "已发布，但学生端暂时无法打开";
    if (v2?.status === "published_forced_not_loadable") return "已发布（强制发布），但学生端暂时无法打开";
    if (v2?.status === "published_forced") return "已发布（强制发布）";
    return job?.status === "Cleaned" ? "已发布并清理过程文件" : "已发布";
  }
  return undefined;
}

export function buildRow(
  id: string,
  job: ImportJob | undefined,
  summary: LibraryExamSummary | undefined,
  options: { inTrash?: boolean } = {},
  v2?: LibraryItemSummaryV2
): LibraryRowV1 {
  const processingStage: Record<string, LibraryStageV1> = {
    queued: "queued", running: "local", local_recognition: "local", cloud_recognition: "cloud",
    reconciling: "reconciling", failed: "failed", cancelled: "failed"
  };
  const itemStage: Record<string, LibraryStageV1> = {
    ready: "ready", action_required: "action_required", published: "published", published_forced: "published",
    // 打不开的两个状态仍然属于「已发布」阶段（用户确实发布过），差别在 detail 与
    // publishedNotLoadable 上如实说出来，而不是把它降级成失败。
    published_not_loadable: "published", published_forced_not_loadable: "published",
    failed: "failed", processing: "queued", migration_required: "action_required"
  };
  // G1/A4-F03：重启恢复把任务停在 ready_for_review + action_required，但
  // library_items_v2.status 仍是 processing；不识别这个组合会把重试耗尽的
  // 条目显示成"排队中"。识别后按真实状态展示并让 detailFor 读错误码。
  const stage: LibraryStageV1 =
    (v2?.processing
      ? v2.processing.stage === "ready_for_review" && v2.processing.localStatus === "action_required"
        ? ("action_required" as LibraryStageV1)
        : processingStage[v2.processing.stage]
      : undefined)
    ?? (v2 ? itemStage[v2.status] : undefined) ?? deriveStage(job, summary);
  const actionable = actionableFrom(job?.issueCounts) || (summary?.issueErrors ?? 0);
  return {
    id,
    // M1：V2 仓库是标题的权威（工作区改名写 library_items_v2）；只在已填充权威稿时覆盖。
    title: v2?.title ?? job?.title ?? summary?.title ?? id,
    modality: rowModality(summary, v2),
    stage,
    detail: detailFor(stage, job, actionable, v2),
    progressPercent: job && isProcessingStage(stage) ? STEP_PROGRESS[job.currentStep] : undefined,
    actionableCount: actionable,
    category: summary?.category ?? job?.category,
    updatedAt: v2?.updatedAt ?? job?.updatedAt ?? summary?.updatedAt ?? "",
    inTrash: Boolean(options.inTrash),
    publishedForced:
      stage === "published" &&
      (v2?.status === "published_forced" || v2?.status === "published_forced_not_loadable"),
    publishedNotLoadable:
      stage === "published" &&
      (v2?.status === "published_not_loadable" || v2?.status === "published_forced_not_loadable"),
    raw: {
      jobStatus: job?.status,
      currentStep: job?.currentStep,
      libraryStatus: summary?.status,
      issueCounts: job?.issueCounts
    }
  };
}

/** 失败 / 已取消的行可以直接在题库里重试（它们没有可编辑的题稿，打开也没用）。 */
export function canRetryRow(row: LibraryRowV1): boolean {
  return row.stage === "failed" && !row.inTrash;
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
