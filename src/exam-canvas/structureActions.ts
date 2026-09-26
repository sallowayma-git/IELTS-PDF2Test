import type { AuthoringPatchV2, ContentNodeV2, IeltsAuthoringIRV2, QuestionNumberExpressionV2 } from "../types";
import { locateContentNode } from "../services/authoringV2Patches";
import type { ExamCanvasStructureAction } from "./ExamCanvas";

export function manualParagraph(text = "新内容"): ContentNodeV2 {
  const id = crypto.randomUUID();
  return { type: "paragraph", id: `p-${id}`, sourceAnchors: [], provenanceStatus: "manual",
    children: [{ type: "text", id: `t-${id}`, sourceAnchors: [], provenanceStatus: "manual", text }] };
}

function hasSlot(value: unknown): boolean {
  if (!value || typeof value !== "object") return false;
  if (Array.isArray(value)) return value.some(hasSlot);
  const node = value as Record<string, unknown>;
  return node.type === "answer_slot" || Object.values(node).some(hasSlot);
}

function expression(numbers: number[]): QuestionNumberExpressionV2 {
  return { kind: "set", values: [...new Set(numbers)].sort((a, b) => a - b) };
}

export function compileStructureAction(current: IeltsAuthoringIRV2, action: ExamCanvasStructureAction): AuthoringPatchV2 | undefined {
  const stamp = crypto.randomUUID();
  if (action.type === "option.add" || action.type === "option.move" || action.type === "option.delete") {
    const task = current.taskGroups.find((candidate) => candidate.taskId === action.taskId);
    const group = task?.responseGroups.find((candidate) => candidate.responseGroupId === action.responseGroupId);
    if (!task || !group) return;
    const shared = task.optionBank && (!group.options?.length || group.optionBankRef === task.optionBank.optionBankId);
    const options = structuredClone(shared ? task.optionBank!.options : group.options ?? []);
    if (action.type === "option.add") {
      const used = new Set(options.map((option) => option.label));
      const label = "ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("").find((candidate) => !used.has(candidate)) ?? String(options.length + 1);
      const index = action.afterOptionId ? options.findIndex((option) => option.optionId === action.afterOptionId) + 1 : options.length;
      options.splice(index, 0, { optionId: `option-${stamp}`, label, content: [manualParagraph()], sourceAnchors: [], provenanceStatus: "manual" });
    } else {
      const index = options.findIndex((option) => option.optionId === action.optionId);
      if (index < 0) return;
      if (action.type === "option.delete") {
        const slotIds = shared ? task.responseGroups.flatMap((response) => response.slotIds) : group.slotIds;
        if (slotIds.some((slotId) => {
          const answer = current.answerKey[slotId];
          return answer?.kind === "option" && answer.labels.includes(options[index].label);
        })) throw new Error("这个选项已用作本题答案，请先修改答案。");
        if (options.length <= 1) throw new Error("至少保留一个选项。");
        options.splice(index, 1);
      } else {
        // 选项带着自己的字母一起移动，答案里引用的字母因此仍然指向同一条选项。
        const [moved] = options.splice(index, 1);
        const before = action.beforeOptionId === undefined ? options.length : options.findIndex((option) => option.optionId === action.beforeOptionId);
        if (before < 0 || before === index) return;
        options.splice(before, 0, moved);
      }
    }
    return shared
      ? { op: "setOptionBank", taskId: task.taskId, optionBank: { ...task.optionBank!, options } }
      : { op: "setResponseGroup", taskId: task.taskId, responseGroup: { ...group, options } };
  }
  if (action.type === "table.row.add" || action.type === "table.row.delete" || action.type === "table.column.add" || action.type === "table.column.delete") {
    const table = locateContentNode(current, action.tableId)?.node;
    if (!table || table.type !== "table") return;
    const rows = structuredClone(table.rows);
    if (rows.some((row) => row.cells.some((cell) => cell.rowSpan !== 1 || cell.colSpan !== 1))) {
      throw new Error("请先在单元格设置中拆分合并单元格，再增删行列。");
    }
    const cell = (id: string): Extract<ContentNodeV2, { type: "table_cell" }> => ({
      type: "table_cell", id, rowSpan: 1, colSpan: 1, headerScope: "none",
      sourceAnchors: [], provenanceStatus: "manual", children: [manualParagraph("")]
    });
    if (action.type === "table.row.add") {
      const index = action.afterRowId ? rows.findIndex((row) => row.id === action.afterRowId) + 1 : rows.length;
      rows.splice(index, 0, { type: "table_row", id: `row-${stamp}`, sourceAnchors: [], provenanceStatus: "manual",
        cells: Array.from({ length: rows[0]?.cells.length || 1 }, (_, index) => cell(`cell-${stamp}-${index}`)) });
    } else if (action.type === "table.row.delete") {
      const index = rows.findIndex((row) => row.id === action.rowId);
      if (index < 0 || rows.length <= 1) return;
      if (hasSlot(rows[index])) throw new Error("这一行包含答案位，请先移动或删除答案位。");
      rows.splice(index, 1);
    } else if (action.type === "table.column.add") {
      const index = action.afterColumnIndex === undefined ? rows[0]?.cells.length ?? 0 : action.afterColumnIndex + 1;
      rows.forEach((row, rowIndex) => row.cells.splice(index, 0, cell(`cell-${stamp}-${rowIndex}`)));
    } else {
      if (rows.some((row) => hasSlot(row.cells[action.columnIndex]))) throw new Error("这一列包含答案位，请先移动或删除答案位。");
      if (rows.some((row) => row.cells.length <= 1)) return;
      rows.forEach((row) => row.cells.splice(action.columnIndex, 1));
    }
    return { op: "replaceContent", target: { kind: "node", nodeId: table.id }, content: rows };
  }
  if (action.type === "answer-slot.insert") {
    const location = locateContentNode(current, action.afterNodeId);
    if (!location || location.node.type !== "answer_slot") return;
    const existing = current.answerSlots[location.node.slotId];
    const task = current.taskGroups.find((task) => task.responseGroups.some((group) => group.slotIds.includes(existing.slotId)));
    const group = task?.responseGroups.find((group) => group.slotIds.includes(existing.slotId));
    if (!task || !group) return;
    const questionNumber = Math.max(0, ...Object.values(current.answerSlots).map((slot) => slot.questionNumber)) + 1;
    const slotId = `q${questionNumber}-${stamp}`;
    const numbers = task.responseGroups.flatMap((response) => response.slotIds.map((id) => current.answerSlots[id].questionNumber));
    return { op: "insertAnswerSlot", taskId: task.taskId, responseGroupId: group.responseGroupId,
      target: location.target, parentId: location.parentId, index: location.index + 1, slotIndex: group.slotIds.indexOf(existing.slotId) + 1,
      node: { ...location.node, id: `slot-${stamp}`, slotId, displayLabel: String(questionNumber), sourceAnchors: [], provenanceStatus: "manual" },
      slot: { ...existing, slotId, questionNumber, displayLabel: String(questionNumber), hostNodeId: location.parentId ?? existing.hostNodeId,
        sourceAnchors: [], provenanceStatus: "manual", confidence: 1 }, value: { kind: "unresolved" }, expression: expression([...numbers, questionNumber]) };
  }
  const task = current.taskGroups.find((task) => task.responseGroups.some((group) => group.slotIds.includes(action.slotId)));
  const group = task?.responseGroups.find((group) => group.slotIds.includes(action.slotId));
  if (!task || !group || group.slotIds.length <= 1) throw new Error("每题至少保留一个答案位。");
  const numbers = task.responseGroups.flatMap((response) => response.slotIds.filter((id) => id !== action.slotId).map((id) => current.answerSlots[id].questionNumber));
  return { op: "deleteAnswerSlot", taskId: task.taskId, responseGroupId: group.responseGroupId, nodeId: action.nodeId, slotId: action.slotId, expression: expression(numbers) };
}
