import type { JobStatus, LibraryStatus, RawLibraryStatus, TaskTypeV2, ValidationIssue, ValidationLayer, WorkflowStep } from "../types";

export const jobStatusLabels: Record<JobStatus, string> = {
  Working: "处理中",
  NeedsReview: "待审核",
  DraftSaved: "可编辑题稿已保存",
  ExportReady: "可导出",
  Exported: "已导出",
  Cleaned: "已清理"
};

export const libraryStatusLabels: Record<LibraryStatus, string> = {
  draft: "草稿",
  needs_review: "待审核",
  ready: "已定稿",
  exported: "已发布"
};

export function normalizeLibraryStatus(status: RawLibraryStatus | string | undefined): LibraryStatus | undefined {
  if (!status) return undefined;
  if (status === "review_required") return "needs_review";
  if (status === "published") return "exported";
  if (status === "draft" || status === "needs_review" || status === "ready" || status === "exported") return status;
  return undefined;
}

export function libraryStatusLabel(status: RawLibraryStatus | string | undefined): string {
  const normalized = normalizeLibraryStatus(status);
  if (!normalized) return status ? String(status) : "未知状态";
  return libraryStatusLabels[normalized];
}

export const workflowStepLabels: Record<WorkflowStep, string> = {
  Upload: "上传文件",
  DocumentReview: "核对源文档",
  Split: "后台识别题组与答案",
  Authoring: "确认与编辑",
  LlmReview: "后台识别复核",
  Preview: "预览与编辑",
  Export: "导出",
  Pack: "发布"
};

export const validationLayerLabels: Record<ValidationLayer, string> = {
  AuthoringIR: "可编辑题稿",
  ReadingExamSourceV1: "导出数据",
  DomProtocol: "答题控件",
  RuntimePreview: "预览检查"
};

/**
 * 题型的中文名（**用户文案**，不是内部枚举名）。
 *
 * 题面里每个题组的小标题此前直接渲染 `task.taskType`，于是用户看到的是
 * `summary_completion` 这种内部标识符。这里给出全部 `TaskTypeV2` 的对照；
 * 用 `Record<TaskTypeV2, string>` 而不是 `Partial<...>` 是刻意的——
 * 契约以后新增题型时类型检查会立刻报缺项，不会再漏一个没翻译的名字。
 */
export const taskTypeLabels: Record<TaskTypeV2, string> = {
  single_choice: "单选题",
  multiple_choice: "多选题",
  true_false_not_given: "判断题（TRUE / FALSE / NOT GIVEN）",
  yes_no_not_given: "判断题（YES / NO / NOT GIVEN）",
  matching_information: "段落信息匹配",
  matching_headings: "段落标题匹配",
  matching_features: "特征匹配",
  matching_sentence_endings: "句子结尾匹配",
  classification: "分类匹配",
  sentence_completion: "句子填空",
  summary_completion: "摘要填空",
  note_completion: "笔记填空",
  table_completion: "表格填空",
  form_completion: "表单填空",
  flowchart_completion: "流程图填空",
  diagram_label_completion: "图表标注",
  plan_map_label_completion: "平面图标注",
  short_answer: "简答题"
};

/**
 * 题型 → 用户文案。
 * 认不出的取值**原样返回**而不是套一层「未知题型」：题型来自后端契约，
 * 前端多一层兜底文案只会把「后端多了一个题型」这件事藏起来。
 */
export function taskTypeLabel(taskType: TaskTypeV2 | string | undefined): string {
  if (!taskType) return "题型未标注";
  return taskTypeLabels[taskType as TaskTypeV2] ?? String(taskType);
}

export function jobStatusLabel(status: JobStatus | string | undefined): string {
  if (!status) return "未知状态";
  return jobStatusLabels[status as JobStatus] ?? String(status);
}

export function workflowStepLabel(step: WorkflowStep | string | undefined): string {
  if (!step) return "未知步骤";
  return workflowStepLabels[step as WorkflowStep] ?? String(step);
}

export function validationLayerLabel(layer: ValidationLayer | string): string {
  return validationLayerLabels[layer as ValidationLayer] ?? String(layer);
}

export function runtimeModeLabel(mode: string | undefined): string {
  if (!mode) return "未运行";
  if (mode === "real") return "真实预览已通过";
  if (mode === "static-rust") return "基础检查已通过";
  if (mode === "fallback") return "开发预览检查";
  return mode;
}

function questionLabelFromPath(path: string): string {
  const match = path.match(/(?:answerKey|questions)[.\[]['"]?(q?\d+)[\]'"]?/i);
  if (!match) return "";
  const number = match[1].replace(/^q/i, "");
  return number ? `Q${number}` : "";
}

function issueAreaLabel(issue: ValidationIssue): string {
  const path = issue.path;
  const question = questionLabelFromPath(path);
  if (path.includes("cloudComparison")) return question ? `云端整卷对照 · ${question}` : "云端整卷对照";
  if (path.includes("$.answerKey")) return question ? `答案 · ${question}` : "答案";
  if (path.includes("$.sourceReview")) return "源文档审核";
  if (path.includes(".reviewWarnings")) return "题型/选项规则";
  if (path.includes(".verified") || path.includes("$.audit.humanVerified") || path.includes("$.job.status")) {
    return question ? `人工确认 · ${question}` : "人工确认";
  }
  if (path.includes(".prompt") || path.includes(".questions")) return question ? `题干 · ${question}` : "题干";
  return validationLayerLabel(issue.layer);
}

function issueMessageLabel(issue: ValidationIssue): string {
  const message = issue.message;
  if (message.includes("Job is still marked NeedsReview")) return "任务仍处于待审核状态，请完成需要确认的项目后再导出。";
  if (message.includes("All questions must be human verified") || message.includes("All questions and answers must be human verified")) return "所有题目都需要人工确认后才能发布。";
  if (message.includes("Question answer is empty") || message.includes("missing from answerKey") || message.includes("answerKey is empty")) return "题目未设置答案，将作为未评分题导出。";
  if (message.includes("Low-confidence question requires human verification")) return "低置信题目需要人工确认。";
  if (message.includes("Low-confidence group requires human verification")) return "低置信题组需要人工确认。";
  if (message.includes("Low-confidence parsed block requires source review")) return "源文档中有低置信识别内容，需要先确认源文档。";
  if (message.includes("Parser warning must be manually resolved")) {
    const page = message.match(/page\s+(\d+)/i)?.[1];
    return page ? `第 ${page} 页没有可提取文字，需要完成视觉识别或人工确认。` : "源文档解析提醒需要人工确认。";
  }
  if (message.includes("Question-group classification warning requires author review")) return "题型或选项规则需要人工确认。";
  return message;
}

export function validationIssueDisplay(issue: ValidationIssue): { title: string; detail: string; action: string } {
  const area = issueAreaLabel(issue);
  const detail = issueMessageLabel(issue);
  const action = issue.fixHint || (
    area.includes("答案")
      ? issue.severity === "error"
        ? "可返回题稿补充答案，或使用忽略检查继续导出。"
        : "答案为可选项，可在之后继续补充。"
      : area.includes("源文档")
        ? "回到源文档审核页确认扫描页、低置信块或视觉补全文本。"
        : area.includes("云端")
          ? "在题稿编辑页查看云端/本地差异，确认后再导出。"
          : "在题稿编辑页完成相关确认后重试。"
  );
  return { title: area, detail, action };
}
