import type { JobStatus, ValidationLayer, WorkflowStep } from "../types";

export const jobStatusLabels: Record<JobStatus, string> = {
  Working: "处理中",
  NeedsReview: "待审核",
  DraftSaved: "可编辑题稿已保存",
  ExportReady: "可导出",
  Exported: "已导出",
  Cleaned: "已清理"
};

export const workflowStepLabels: Record<WorkflowStep, string> = {
  Upload: "上传文件",
  DocumentReview: "核对源文档",
  Split: "识别题组与答案",
  Authoring: "题稿编辑",
  LlmReview: "确认识别结果",
  Preview: "预览与校验",
  Export: "导出",
  Pack: "组卷"
};

export const validationLayerLabels: Record<ValidationLayer, string> = {
  AuthoringIR: "可编辑题稿",
  ReadingExamSourceV1: "导出数据",
  DomProtocol: "答题控件",
  RuntimePreview: "预览检查"
};

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
