import type { IeltsAuthoringIRV2 } from "../../types";

/**
 * 题面定位的公共实现（供两处共用）：
 *   - ExamWorkspacePage.locateTarget —— 「待补充」清单 / 识别面板 / 预览问题的「去填写」「定位」；
 *   - QuestionNavBar —— 底部题号导航点击后滚动到对应题目。
 *
 * 三跳查找规则（缺一跳都会变成「点了没反应」）：
 *   1. slotId（如 `q27`）：只有非行内列表版式才把 `data-question-id` 打在元素上；
 *   2. `answerSlots[slotId].hostNodeId`：答案位的宿主内容节点（`data-editor-id`）；
 *   3. 内容节点 id：内联填空的输入框渲染在 stimulus 内部，宿主元素带的是
 *      `slot-node-q27` 这类内容节点 id，前两跳都找不到它。
 */

/** 草稿里承载某个答案位的**内容节点 id**。
 *
 * 只遍历 `taskGroups`：内联答案位一定在题组的 prompt / stimulus 里（passage 里不会有
 * 可作答的答案位），这样既够用又不用深走整份草稿（草稿里的 sourceAnchors 很大）。
 */
export function contentNodeIdsForSlot(draft: IeltsAuthoringIRV2 | undefined, slotId: string): string[] {
  if (!draft || !slotId) return [];
  const found: string[] = [];
  const seen = new Set<unknown>();
  const walk = (node: unknown): void => {
    if (!node || typeof node !== "object" || seen.has(node)) return;
    seen.add(node);
    if (Array.isArray(node)) {
      for (const item of node) walk(item);
      return;
    }
    const record = node as Record<string, unknown>;
    if (record.slotId === slotId && typeof record.id === "string" && record.id) found.push(record.id);
    for (const value of Object.values(record)) walk(value);
  };
  walk(draft.taskGroups);
  return found;
}

/** 候选 id 列表：slotId → hostNodeId → 内容节点 id，去重、去空。 */
export function locateCandidateIds(draft: IeltsAuthoringIRV2 | undefined, targetId: string): string[] {
  const hostNodeId = draft?.answerSlots?.[targetId]?.hostNodeId;
  return [targetId, hostNodeId, ...contentNodeIdsForSlot(draft, targetId)]
    .filter((value): value is string => typeof value === "string" && value.length > 0)
    .filter((value, index, all) => all.indexOf(value) === index);
}

/** 在题面 DOM 上按三跳规则找 targetId 对应的可定位元素；找不到返回 undefined。 */
export function findTargetElement(
  targetId: string,
  draft: IeltsAuthoringIRV2 | undefined,
  doc: Document = document
): HTMLElement | undefined {
  const candidates = locateCandidateIds(draft, targetId);
  return Array.from(doc.querySelectorAll<HTMLElement>(
    "[data-editor-id], [data-question-id], [data-response-group-id]"
  )).find((element) => [
    element.dataset.editorId,
    element.dataset.questionId,
    element.dataset.responseGroupId
  ].some((value) => value !== undefined && candidates.includes(value)));
}
