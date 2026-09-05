import type { AnswerValueV2, AuthoringPatchV2, IeltsAuthoringIRV2 } from "../types";

/** 归一化矩形 [x, y, width, height]，与 `DiagramHotspotV2.normalizedRect` 同构。 */
export type NormalizedRect = [number, number, number, number];

// EditorCommandV1（计划 §9.4）。
//
// 相对现有 `AuthoringPatchV2` 的区别只有一点，但很重要：文本编辑用 `set_text` 表达整节点替换，
// 并带上 `expectedText` 做乐观并发校验，而不是让 UI 计算字符下标区间。
// 中文、emoji 和组合字符下，字符下标运算是最容易出错的部分。
//
// 编译目标仍是现有的 `replaceText`，因为后端 `replace_text` 已经用 Unicode scalar（char）计数，
// `from: 0, to: <code point 数>` 就是「整节点替换」。等 P4 后端落地原生 `set_text` 后，
// 这里只需换 compileEditorCommand 的一行。
export type EditorCommandV1 =
  | { op: "set_text"; nodeId: string; expectedText: string; text: string }
  | { op: "set_answer"; slotId: string; value: AnswerValueV2 }
  | { op: "set_slot_placement"; nodeId: string; slotId: string; rect: NormalizedRect };

/** 与 Rust `chars().count()` 一致的长度：按 Unicode code point 计，不是 UTF-16 code unit。 */
export function codePointLength(text: string): number {
  return Array.from(text).length;
}

export class EditorCommandConflictError extends Error {
  constructor(readonly nodeId: string, readonly expectedText: string, readonly actualText: string) {
    super(`EDITOR_COMMAND_STALE_TEXT:${nodeId}`);
    this.name = "EditorCommandConflictError";
  }
}

function findTextNode(document: IeltsAuthoringIRV2, nodeId: string): { text: string } | undefined {
  let found: { text: string } | undefined;
  const walk = (value: unknown): void => {
    if (found || !value || typeof value !== "object") return;
    if (Array.isArray(value)) {
      for (const entry of value) walk(entry);
      return;
    }
    const record = value as Record<string, unknown>;
    if (record.id === nodeId && record.type === "text" && typeof record.text === "string") {
      found = { text: record.text };
      return;
    }
    for (const entry of Object.values(record)) walk(entry);
  };
  walk(document);
  return found;
}

/** 把 EditorCommandV1 编译成后端已支持的 patch。文本命令会先校验 expectedText。 */
export function compileEditorCommand(command: EditorCommandV1, document: IeltsAuthoringIRV2): AuthoringPatchV2 {
  if (command.op === "set_answer") {
    return { op: "setAnswer", slotId: command.slotId, value: command.value };
  }
  if (command.op === "set_slot_placement") {
    return {
      op: "setHotspot",
      nodeId: command.nodeId,
      hotspot: { hotspotId: `${command.slotId}-hotspot`, slotId: command.slotId, normalizedRect: command.rect }
    };
  }
  const node = findTextNode(document, command.nodeId);
  const actual = node?.text ?? "";
  if (actual !== command.expectedText) {
    throw new EditorCommandConflictError(command.nodeId, command.expectedText, actual);
  }
  return { op: "replaceText", nodeId: command.nodeId, from: 0, to: codePointLength(actual), text: command.text };
}

export function compileEditorCommands(commands: EditorCommandV1[], document: IeltsAuthoringIRV2): AuthoringPatchV2[] {
  return commands.map((command) => compileEditorCommand(command, document));
}
