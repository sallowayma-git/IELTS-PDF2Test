import type { IeltsAuthoringIRV2 } from "../../types";
import { command } from "../../api/tauriCommands";

// 学生预览的编译入口（P2 门禁收敛）。
//
// 预览**必须**走产品真正使用的那条编译路径：学生端渲染的是 Rust 编译器
// （`compile_reading_source_v2` / `compile_listening_source_v1`）的产物。这里直接
// 调同一个 Rust 编译命令（`compile_authoring_preview`），不再在 TS 侧另做一份
// 编译/校验——那份民办实现曾与 Rust 各说各话（Rust 修过的漏到 TS，用户照旧看到
// `RUNTIME_OPTION_BANK_MISSING`）。
//
// 按 `modality` 分派由 Rust 编译器自己做（稿件自身的 modality 决定目标契约），
// 这里只负责把当前内存草稿交给它：
//   - 编译失败（结构非法 / 热点越界 / 资源路径不安全 / Section 缺音频）→ 返回可定位的错误，
//     **不**渲染预览，避免用户对着过期画面继续编辑；
//   - 编译成功 → 交给共享的 ExamCanvas student 模式渲染，交互语义与学生端一致。
//
// 注意：编译成功**不等于**可发布。可发布还要过发布门禁（P2 起：编译通过即可发布，
// 其余为警告），两者在界面上分开显示。

export interface PreviewCompileIssue {
  code: string;
  targetId: string;
  message: string;
}

export type PreviewCompiledSource =
  | { modality: "reading" }
  | { modality: "listening" };

export type PreviewCompileResult =
  | {
      ok: true;
      compiled: PreviewCompiledSource;
      summary: {
        modality: "reading" | "listening";
        taskGroups: number;
        slots: number;
        assets: number;
        answeredSlots: number;
        /** 听力稿的 Section 数（每个 Section 一段独立音频）；阅读稿恒为 0。 */
        listeningParts: number;
        /** 答案键类型与槽位交互不一致的槽位（真实学生端会在提交阶段拒绝整份提交）。 */
        answerKeyIssues: PreviewCompileIssue[];
      };
    }
  | { ok: false; issue: PreviewCompileIssue };

interface RustPreviewCompileResult {
  ok: boolean;
  issues: PreviewCompileIssue[];
  summary: {
    modality: "reading" | "listening";
    taskGroups: number;
    slots: number;
    assets: number;
    answeredSlots: number;
    listeningParts: number;
    answerKeyIssues: PreviewCompileIssue[];
  } | null;
}

function compileIssue(code: string, targetId: string, message: string): PreviewCompileIssue {
  return { code, targetId, message };
}

/** 当前草稿过 Rust 编译器（不产生任何写入，也不读取学生答案）。 */
export async function compilePreviewSource(
  draft: IeltsAuthoringIRV2 | undefined
): Promise<PreviewCompileResult | undefined> {
  if (!draft) return undefined;
  let result: RustPreviewCompileResult;
  try {
    result = await command<RustPreviewCompileResult>("compile_authoring_preview", {
      authoring: draft
    });
  } catch (error) {
    const raw = error instanceof Error ? error.message : String(error);
    return { ok: false, issue: compileIssue("PREVIEW_COMPILE_FAILED", "exam", raw) };
  }
  if (!result?.ok || !result.summary) {
    const first = result?.issues?.[0];
    return {
      ok: false,
      issue: first
        ? { code: first.code, targetId: first.targetId ?? "exam", message: first.message }
        : compileIssue("PREVIEW_COMPILE_FAILED", "exam", "编译未通过。")
    };
  }
  return {
    ok: true,
    compiled:
      result.summary.modality === "listening"
        ? { modality: "listening" }
        : { modality: "reading" },
    summary: {
      modality: result.summary.modality,
      taskGroups: result.summary.taskGroups,
      slots: result.summary.slots,
      assets: result.summary.assets,
      answeredSlots: result.summary.answeredSlots,
      listeningParts: result.summary.listeningParts,
      // 编译通过即答案键类型一致（Rust 编译器逐槽位校验 interaction↔kind）。
      answerKeyIssues: []
    }
  };
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
