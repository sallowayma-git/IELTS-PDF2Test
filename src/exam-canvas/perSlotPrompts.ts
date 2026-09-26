import type { AnswerSlotV2, ContentNodeV2, ResponseGroupV2 } from "../types";

/**
 * 判断题（TFNG/YNNG）逐题题干（per-slot prompt）判定。
 *
 * 识别产物契约：一个 responseGroup 只有一个 responseGroup-responses，其 response.prompt
 * 是"每题一段"的顶层节点列表（id 形如 group-1-prompt-1…），answerSlot.hostNodeId 指向
 * 自己那一段（golden metadata 声明 promptMode: "per_slot"）。启用时画布把该题陈述渲染到
 * 题号后面，顶部只保留未被认领的公共段。
 *
 * 全有或全无：任何条件不满足都返回 undefined，调用方保持旧版式（整块 prompt 在顶部），
 * 避免只拆一半让版式比原来更乱。学生端渲染器使用与本函数完全相同的规则。
 */

export interface PerSlotPromptLayout {
  /** slotId → 该题在 response.prompt 顶层的宿主节点。 */
  readonly hostNodeBySlotId: ReadonlyMap<string, ContentNodeV2>;
  /** 被认领的顶层节点 id；顶部公共 prompt 只渲染未被认领的节点。 */
  readonly claimedNodeIds: ReadonlySet<string>;
}

/** answerSlots 的最小结构（只需要 hostNodeId），便于学生端复用同一规则。 */
export type SlotHostTable = Readonly<Record<string, Pick<AnswerSlotV2, "hostNodeId"> | undefined>> | undefined;

function nodeContainsAnswerSlot(node: ContentNodeV2): boolean {
  if (node.type === "answer_slot") return true;
  if ("children" in node) return childrenContainAnswerSlot(node.children);
  if ("items" in node) return node.items.some((item) => childrenContainAnswerSlot(item.children));
  if ("rows" in node) return node.rows.some((row) => row.cells.some((cell) => childrenContainAnswerSlot(cell.children)));
  if ("steps" in node) return node.steps.some((step) => childrenContainAnswerSlot(step.children));
  return false;
}

function childrenContainAnswerSlot(nodes: readonly ContentNodeV2[] | undefined): boolean {
  return (nodes ?? []).some(nodeContainsAnswerSlot);
}

/**
 * @param response    待判定的 responseGroup（异常输入一律按不启用处理）
 * @param answerSlots slotId → 答案位（至少含 hostNodeId）的查找表
 * @returns 启用时给出 slotId → 宿主节点与已认领节点 id 集合；不启用返回 undefined
 */
export function perSlotPrompts(
  response: ResponseGroupV2 | undefined | null,
  answerSlots: SlotHostTable
): PerSlotPromptLayout | undefined {
  // c) unordered_set 共享多选保持原样。
  if (!response || response.assignment === "unordered_set") return undefined;
  const prompt = response.prompt;
  if (!Array.isArray(prompt) || prompt.length === 0) return undefined;
  const slotIds = response.slotIds;
  // 空 slotIds：没有题目可认领段落，顶部 prompt 必须保持原样。
  if (!Array.isArray(slotIds) || slotIds.length === 0) return undefined;

  const hostNodeBySlotId = new Map<string, ContentNodeV2>();
  const claimedNodeIds = new Set<string>();
  for (const slotId of slotIds) {
    const hostNodeId = answerSlots?.[slotId]?.hostNodeId;
    if (!hostNodeId) return undefined;
    // a) 宿主必须是 prompt 顶层节点且两两不同；b) 宿主内部不得嵌 answer_slot
    //（内嵌答案位是 embedded 填空，保持原样，否则同一答案位会出现两个输入框）。
    const hostNode = prompt.find((node) => node?.id === hostNodeId);
    if (!hostNode || claimedNodeIds.has(hostNodeId) || nodeContainsAnswerSlot(hostNode)) return undefined;
    hostNodeBySlotId.set(slotId, hostNode);
    claimedNodeIds.add(hostNodeId);
  }
  return { hostNodeBySlotId, claimedNodeIds };
}
