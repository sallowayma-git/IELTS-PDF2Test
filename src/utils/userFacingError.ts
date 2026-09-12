// 用户可见错误文案层（计划 §15.1 / §15.2；audit A7-F04）。
//
// 目标：普通用户界面只出现人话；后端机器码、JSON payload、文件路径等内部细节一律收敛到
// `internalDetail`，只用于开发者模式与日志，不再直接渲染。
//
// 约定：
//  - 已知机器码前缀 -> 固定的人话文案；
//  - 已经是人话（含中文）的错误串（例如结构操作抛出的中文提示）原样透传；
//  - 其余纯 ASCII 的机器串 -> 通用兜底文案，原文保留在 `internalDetail`。

export type UserErrorCategory =
  | "conflict"
  | "not_ready"
  | "not_found"
  | "validation"
  | "permission"
  | "runtime"
  | "too_large"
  | "unknown";

export interface UserFacingError {
  category: UserErrorCategory;
  /** 稳定的内部标识（机器码前缀），仅用于诊断。 */
  code?: string;
  /** 给普通用户看的文本。 */
  userMessage: string;
  /** 原始错误串（机器码 / JSON / 路径），只在开发者模式或日志中出现。 */
  internalDetail: string;
}

const CODE_PATTERN = /\b([A-Z][A-Z0-9_]{3,})\b/;
const CJK_PATTERN = /[\u4e00-\u9fff]/;
const DEFAULT_FALLBACK = "操作没有完成，请稍后重试。";

function rawMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function extractCode(raw: string): string | undefined {
  if (raw.startsWith("publish_check_failed:")) return "publish_check_failed";
  return raw.match(CODE_PATTERN)?.[1];
}

/** `publish_check_failed:{...}` 的 payload 里带有后端准备好的 blocker 文案。 */
function publishCheckMessage(raw: string): string | undefined {
  if (!raw.startsWith("publish_check_failed:")) return undefined;
  try {
    const check = JSON.parse(raw.slice("publish_check_failed:".length)) as {
      blockers?: Array<{ userMessage?: string }>;
    };
    return check.blockers?.find((blocker) => blocker.userMessage)?.userMessage
      ?? "题目还有未完成的内容，请补齐后再次发布。";
  } catch {
    return "题目还有未完成的内容，请补齐后再次发布。";
  }
}

function classify(raw: string): { category: UserErrorCategory; userMessage?: string } {
  const publish = publishCheckMessage(raw);
  if (publish) return { category: "validation", userMessage: publish };
  if (raw.includes("EDIT_VERSION_CONFLICT")) {
    return { category: "conflict", userMessage: "这道题在别处也被改过，请刷新后重试。" };
  }
  if (raw.includes("ITEM_DS_NOT_SEEDED") || raw.includes("AUTHORING_V2_NOT_AVAILABLE")) {
    return { category: "not_ready", userMessage: "这道题还没有生成可编辑的题稿。运行本地识别后就能编辑。" };
  }
  if (raw.includes("ITEM_NOT_FOUND")) {
    return { category: "not_found", userMessage: "这道题已不在题库中，请返回题库刷新。" };
  }
  if (raw.includes("authoring_v2_export_blocked")) {
    return { category: "validation", userMessage: "请先补齐题干、选项或答案，再次发布。" };
  }
  if (raw.includes("requires_tauri_runtime")) {
    return { category: "runtime", userMessage: "这个操作需要在桌面应用中运行。" };
  }
  if (raw.includes("source_file_too_large")) {
    return { category: "too_large", userMessage: "文件太大，无法导入。" };
  }
  if (raw.startsWith("library_v2_tx")) {
    return { category: "unknown", userMessage: "保存没有完成，请稍后重试。" };
  }
  if (/permission|denied|readonly|read-only/i.test(raw)) {
    return { category: "permission", userMessage: "目标目录不可写，请检查权限。" };
  }
  return { category: "unknown" };
}

/** 把任意错误归一化为「用户文案 + 内部细节」。 */
export function toUserFacingError(error: unknown, fallback: string = DEFAULT_FALLBACK): UserFacingError {
  const raw = rawMessage(error);
  const { category, userMessage } = classify(raw);
  const code = extractCode(raw);
  if (userMessage) return { category, code, userMessage, internalDetail: raw };
  // 已是人话（含中文）的错误直接透传，例如结构操作抛出的中文提示。
  if (CJK_PATTERN.test(raw)) return { category, code, userMessage: raw, internalDetail: raw };
  return { category, code, userMessage: fallback, internalDetail: raw };
}

/** 便捷函数：只取用户文案。 */
export function userMessageOf(error: unknown, fallback?: string): string {
  return toUserFacingError(error, fallback).userMessage;
}
