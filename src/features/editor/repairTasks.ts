import type { CloudRepairTaskV1, RecognitionDecisionViewV1 } from "../../api/recognitionClient";

// 云端自主修复的**剩余任务**呈现规则（纯函数，可单测）。
//
// 这一层存在的理由：新主链的云端修复会**自己动手改稿**，用户不再是「逐条接受建议」的
// 审批者，而是「处理云端解决不了的那几件事」。于是普通界面必须换一份清单——以修复摘要的
// `remainingTasks` 为准，而不是继续铺开修复之前算出来的建议卡。两套同时显示的结果是：
// 云端明明已经改好的项，下面还挂着一张「要不要采用修正」的卡，用户被迫做一次没有意义的
// 决定。
//
// 三条硬约束：
//  1. **旧批次照旧**。没有 `repair` 记录（旧批次 / 无云导入）时这套规则完全不生效，
//     界面走原来的建议卡路径——不能因为新链路把老数据变成空白面板。
//  2. **每条任务都要有真能做完的动作**。后端给的 `action` 只决定「往哪儿送」，
//     绝不生成「确认」「忽略」这类点了不改任何东西的按钮。
//  3. **没有定位目标就如实说**。`targetIds` 为空的任务（文档级疑问、来源覆盖缺口）
//     不能给一个点了没反应的「定位」。

/** 剩余任务上可以给的动作。`targetId` 为空表示这条不落到具体题面节点。 */
export type RepairTaskActionId = "locate" | "open-source" | "fix-on-page";

export interface RepairTaskActionV1 {
  id: RepairTaskActionId;
  label: string;
  targetId?: string;
}

/** 一条待用户处理的剩余任务。 */
export interface RepairTaskViewV1 {
  taskId: string;
  message: string;
  /** 阻断项：不处理就不能导出。 */
  blocking: boolean;
  /** 后端给的处理方式（诊断与文案用，不直接渲染成内部术语）。 */
  action: string;
  targetIds: string[];
  actions: RepairTaskActionV1[];
}

/**
 * 这条批次是否走「修复摘要 + 剩余任务」的界面。
 *
 * 判据只有一条：**有没有修复记录**。`repair` 为 `null` 表示旧批次或本次无云导入，
 * 此时剩余任务清单根本不存在（`remainingTasks` 缺失），界面必须回到旧路径，
 * 而不是显示一个「0 处待处理」的假空状态。
 */
export function usesRepairTaskList(view: RecognitionDecisionViewV1 | undefined): boolean {
  return Boolean(view?.repair);
}

/** 修复是否还在进行中。进行中不展示剩余清单（后端此刻给的就是空数组）。 */
export function repairInFlight(view: RecognitionDecisionViewV1 | undefined): boolean {
  return view?.repair?.status === "running";
}

/**
 * 把后端的一条剩余任务翻译成界面任务。
 *
 * 动作的选取规则（顺序有意义）：
 *  - 有具体目标 → 「定位到题面」：把用户送到那道题/那个节点上；
 *  - 目标是阻断的结构问题 → 额外说明「必须在这里改」；
 *  - 没有目标、且后端说要看原文件 → 「打开原文件」：这是唯一真能推进它的动作；
 *  - 什么都没有 → **不给按钮**，只保留后端那句话。宁可让用户自己找，
 *    也不给一个按下去没有任何反应的按钮。
 */
export function toRepairTaskView(task: CloudRepairTaskV1): RepairTaskViewV1 {
  const targetIds = (task.targetIds ?? []).filter(
    (value): value is string => typeof value === "string" && value.length > 0
  );
  const action = typeof task.action === "string" ? task.action : "";
  const blocking = task.blocking === true;
  const actions: RepairTaskActionV1[] = [];

  if (targetIds.length) {
    actions.push({ id: "locate", label: "定位到题面", targetId: targetIds[0] });
  } else if (action === "review_source") {
    // 文档级任务：没有题面节点可定位，唯一的真动作是打开原文件去核对。
    actions.push({ id: "open-source", label: "打开原文件核对" });
  }
  if (blocking && targetIds.length) {
    // 阻断项必须在题面上改掉才有用，光「定位过去」不够，所以补一条明确的动作。
    actions.push({ id: "fix-on-page", label: "去题面修改", targetId: targetIds[0] });
  }

  return {
    taskId: task.userTaskId,
    message: task.message?.trim() || "云端留下了这条内容，需要你确认。",
    blocking,
    action,
    targetIds,
    actions
  };
}

/** 剩余任务清单（顺序与后端一致，后端已按「质量问题 → 差异 → 疑问 → 覆盖缺口」排好）。 */
export function repairTasks(view: RecognitionDecisionViewV1 | undefined): RepairTaskViewV1[] {
  if (!usesRepairTaskList(view)) return [];
  return (view?.repair?.remainingTasks ?? []).map(toRepairTaskView);
}

/** 还有几处需要用户处理（阻断 + 非阻断）。 */
export function repairTaskCount(view: RecognitionDecisionViewV1 | undefined): number {
  return repairTasks(view).length;
}

/** 其中几处是阻断（决定能不能导出）。 */
export function repairBlockerCount(view: RecognitionDecisionViewV1 | undefined): number {
  return repairTasks(view).filter((task) => task.blocking).length;
}

/**
 * 面板顶部那一行：把「云端做了什么」和「你还剩什么」说成一句话。
 *
 * 为什么不能直接用 `describeRepairStatus`：那句话说的是**修复循环的状态**，
 * 而这里要回答的是「我现在还要不要做事」。两者都有用，但不能互相顶替——
 * 「修复已完成」+「还剩 3 处」是最容易让用户误判的组合。
 */
export function repairHeadline(view: RecognitionDecisionViewV1 | undefined): string {
  if (repairInFlight(view)) return "云端正在自动修复，先不用管；完成后这里会列出剩余问题。";
  const tasks = repairTasks(view);
  if (!tasks.length) return "云端自动修复已完成，没有需要你处理的问题。";
  const blockers = tasks.filter((task) => task.blocking).length;
  return blockers > 0
    ? `云端自动修复已完成，还有 ${tasks.length} 处需要你处理（其中 ${blockers} 处不处理不能导出）。`
    : `云端自动修复已完成，还有 ${tasks.length} 处建议你确认。`;
}

/**
 * 走新清单时面板是不是「没什么可说的」。
 *
 * 与旧的 `isRecognitionQuiet` 判据不同，这里**不看修复之前算出来的建议计数**：
 * 那些建议正是云端刚刚处理过的东西，拿它们决定面板收不收起，会出现「面板收成一行，
 * 同时还有三条剩余任务挂着」的矛盾。判据只有两件事：
 *   - 修复还在进行 → 不安静（用户需要看到它在跑，且此时没有剩余清单）；
 *   - 还有剩余任务 → 不安静。
 * 修复完成且没有剩余 → 安静，收敛成一行。
 *
 * 不适用（旧批次）时返回 `false`，让调用方走旧判据。
 */
export function isRepairPanelQuiet(view: RecognitionDecisionViewV1 | undefined): boolean {
  if (!usesRepairTaskList(view)) return false;
  if (repairInFlight(view)) return false;
  return repairTasks(view).length === 0;
}
