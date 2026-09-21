import type { IeltsAuthoringIRV2 } from "../../types";
import { buildReadingSourceV2FromAuthoring, validateReadingAnswerKeyKinds } from "../../services/readingRuntimeV2";
import { ReadingRuntimeError, type ReadingExamSourceV2 } from "../../types/reading-runtime-v2";

// 学生预览的编译入口。
//
// 预览**必须**走产品真正使用的那条编译路径：发布器把 `IeltsAuthoringIRV2` 编成
// `ReadingExamSourceV2` 后写进 NAS 包，学生端渲染的是编译产物。如果预览自己另做一份
// 近似渲染，就会出现「预览能看、发布后被学生端拒绝」这类只在真机上暴露的问题。
//
// 所以这里先调用同一个 `buildReadingSourceV2FromAuthoring`：
//   - 编译失败（结构非法 / 热点越界 / 资源路径不安全）→ 返回可定位的错误，**不**渲染预览，
//     避免用户对着过期画面继续编辑；
//   - 编译成功 → 交给共享的 ExamCanvas student 模式渲染，交互语义与学生端一致。
//
// 注意：编译成功**不等于**可发布。可发布还要过发布门禁，两者在界面上分开显示。

export interface PreviewCompileIssue {
  code: string;
  targetId: string;
  message: string;
}

export type PreviewCompileResult =
  | {
      ok: true;
      source: ReadingExamSourceV2;
      summary: {
        taskGroups: number;
        slots: number;
        assets: number;
        answeredSlots: number;
        /** 答案键类型与槽位交互不一致的槽位（真实学生端会在提交阶段拒绝整份提交）。 */
        answerKeyIssues: PreviewCompileIssue[];
      };
    }
  | { ok: false; issue: PreviewCompileIssue };

function compileIssue(error: unknown): PreviewCompileIssue {
  if (error instanceof ReadingRuntimeError) {
    return { code: error.code, targetId: error.targetId ?? "exam", message: error.message };
  }
  const raw = error instanceof Error ? error.message : String(error);
  return { code: "PREVIEW_COMPILE_FAILED", targetId: "exam", message: raw };
}

/** 纯函数：编译当前草稿。不产生任何写入，也不读取学生答案。 */
export function compilePreviewSource(draft: IeltsAuthoringIRV2 | undefined): PreviewCompileResult | undefined {
  if (!draft) return undefined;
  try {
    const source = buildReadingSourceV2FromAuthoring(draft);
    const slots = Object.keys(source.answerSlots).length;
    const answeredSlots = Object.values(source.answerKey).filter((value) => {
      if (!value) return false;
      if (value.kind === "text") return value.values.some((entry) => entry.trim().length > 0);
      if (value.kind === "option") return value.labels.length > 0;
      return false;
    }).length;
    // 编译通过 ≠ 学生端能收下。答案键类型不匹配时题面照样渲染，但真实学生端会在提交阶段
    // 拒绝整份提交。这里如实带出来，让预览把「能看」和「能提交」分开说，不留假完成。
    const answerKeyIssues = validateReadingAnswerKeyKinds(source).map((item) => ({
      code: item.code,
      targetId: item.targetId,
      message: item.message
    }));
    return {
      ok: true,
      source,
      summary: {
        taskGroups: source.taskGroups.length,
        slots,
        assets: Object.keys(source.assets.assets).length,
        answeredSlots,
        answerKeyIssues
      }
    };
  } catch (error) {
    return { ok: false, issue: compileIssue(error) };
  }
}

export interface PreviewPublishLimitation {
  level: "info" | "warning";
  message: string;
}

/**
 * 预览里如实说明「现在看到的内容」与「发布后学生看到的内容」的差距。
 *
 * 预览渲染的是**当前内存草稿**，而发布读的是**已保存的权威稿**。有未保存修改时
 * 二者不同，必须明说，否则用户会以为预览里的改动已经能发布了。
 *
 * **刻意不收 `savedVersion`**（本轮任务书第一节）：`v1/v2/v3` 是内部并发保护的计数，
 * 不是用户概念。以前这里会渲染「发布时只会使用已保存的 v7」，用户既不知道 v7 是什么，
 * 也无法据此做任何事。参数干脆不提供，避免以后又有人把它拼回文案里。
 */
export function describePreviewPublishLimitation(input: {
  pendingCount: number;
  blockerCount: number;
  /** 答案键类型与槽位不匹配的槽位数（真实学生端会在提交阶段拒绝整份提交）。 */
  runtimeIssueCount?: number;
}): PreviewPublishLimitation {
  const parts: string[] = [];
  let level: "info" | "warning" = "info";
  if (input.pendingCount > 0) {
    level = "warning";
    parts.push(`当前预览包含 ${input.pendingCount} 项还没有保存的修改；发布时只会使用已保存的内容。`);
  } else {
    parts.push("预览与已保存的内容一致。");
  }
  if (input.runtimeIssueCount && input.runtimeIssueCount > 0) {
    level = "warning";
    parts.push(`其中 ${input.runtimeIssueCount} 个答案位的答案形式与题目不匹配，学生提交时会被判为无效；已列在「待补充」里。`);
  }
  return { level, message: parts.join("") };
}
