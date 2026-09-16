import type { RecognitionDecisionItemV1, RecognitionDecisionViewV1 } from "../../api/recognitionClient";
import type { AnswerValueV2, AuthoringPatchV2 } from "../../types";

// 识别决策的**呈现规则**（纯函数，可单测）。
//
// 与《识别闭环契约》§2.4 / §2.7 一一对应：
//   - `agreed` 不产生逐项问题（只在汇总计数里）；
//   - `severity=info` 不进主问题列表；
//   - `auto_fixed` 已经在后台原子写入权威稿 → 顶部单独一组、默认折叠、只给「撤销」；
//   - `needs_review` → 一条待确认建议卡，接受 / 保持现状；
//   - `unverifiable` → 只说明「无法验证 + 原因码」，**不提供接受**（不允许强行选一个版本）；
//   - `dependencyGroup` 相同的项必须整组同向操作。
//
// 前端只消费后端给的这一份统一建议集合，不自己再算一版，也不显示多份「本地稿/云端稿」。

export interface DecisionGroup {
  /** null 表示该项不参与依赖组，独立成组。 */
  groupId: string | null;
  items: RecognitionDecisionItemV1[];
}

/** 主问题列表里要显示的项：排除 agreed / info / superseded。 */
export function visibleDecisionItems(view: RecognitionDecisionViewV1 | undefined): RecognitionDecisionItemV1[] {
  if (!view?.items?.length) return [];
  return view.items.filter((item) => {
    if (item.resolution === "agreed") return false;
    if (item.severity === "info") return false;
    if (item.status === "superseded") return false;
    return true;
  });
}

/** 已自动修正并写入权威稿的项：顶部展示，默认折叠，只提供撤销。 */
export function autoFixedItems(view: RecognitionDecisionViewV1 | undefined): RecognitionDecisionItemV1[] {
  return visibleDecisionItems(view).filter((item) => item.resolution === "auto_fixed");
}

/**
 * 解析 `auto_fixed` 项的撤销补丁（契约 §4.4 / §6.1）。
 *
 * 撤销**必须**把 `undo` 当作一条编辑器命令提交，经 V2 patch 事务把值改回修正前。
 * 这里曾经是 `submit(..., "reject")`：而后端 reject 的语义是
 * 「只改状态，不碰权威稿」（`reconcile/commands.rs` 的拒绝分支），于是界面说
 * 「已保持现状」、权威稿里自动修正却原样留着 —— 正是「假完成」。
 *
 * 形状认不出来时返回 `undefined`：宁可不给按钮让用户手动改回，也不给一个
 * 按了没用的「撤销」。
 */
export function parseUndoPatch(undo: unknown): AuthoringPatchV2 | undefined {
  if (!undo || typeof undo !== "object" || Array.isArray(undo)) return undefined;
  const record = undo as Record<string, unknown>;
  // 目前只有 `setAnswer` 会被自动应用，撤销面也只认它（与后端 `undo_patch_for` 一致）。
  if (record.op !== "setAnswer") return undefined;
  if (typeof record.slotId !== "string" || !record.slotId) return undefined;
  if (!record.value || typeof record.value !== "object") return undefined;
  return { op: "setAnswer", slotId: record.slotId, value: record.value as AnswerValueV2 };
}

/** 两个答案值是不是同一个值（按契约的 `{kind, values|labels}` 形状比，未知形状返回 false）。 */
function sameAnswerValue(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (!a || !b || typeof a !== "object" || typeof b !== "object") return false;
  const x = a as { kind?: unknown; values?: unknown; labels?: unknown };
  const y = b as { kind?: unknown; values?: unknown; labels?: unknown };
  if (x.kind !== y.kind) return false;
  if (x.kind !== "text" && x.kind !== "option") return false;
  const listOf = (v: { kind?: unknown; values?: unknown; labels?: unknown }) => (v.kind === "option" ? v.labels : v.values);
  const nx = listOf(x);
  const ny = listOf(y);
  if (!Array.isArray(nx) || !Array.isArray(ny)) return false;
  return nx.length === ny.length && nx.every((value, index) => String(value) === String(ny[index]));
}

/**
 * 撤销**是否已经生效** —— 只看权威稿里的值，不看任何会话内状态。
 *
 * 为什么必须这样判（本轮修正）：面板原先用一个会话内的 `Set<decisionId>` 记「我点过撤销」，
 * 那是**完成依据**而不是事实依据 —— 刷新页面、换一台机器、或后端重放，
 * 这个 Set 就没了，界面又会把「撤销」按钮放回来。
 *
 * 后端目前**没有**持久化的「已撤销」状态：`RecognitionResolutionV1` 只有
 * `agreed | auto_fixed | needs_review | unverifiable`，`build_view` 也会把
 * `auto_fixed` 的项无条件留在 `autoApplied` 里。所以不能靠状态码判。
 *
 * 但撤销的语义本身是**可观测的持久化事实**：`undo` 补丁带着「改回哪个值」，
 * 只要权威稿里那个答案位已经等于这个值，撤销就已经生效（无论是刚点的、
 * 上次会话点的、还是用户自己手改回去的）。重开后依然成立。
 *
 * @param undo 后端给的撤销补丁（`{op:'setAnswer', slotId, value}`）
 * @param answerKey 当前**已保存**权威稿的答案位
 */
export function isUndoAlreadyApplied(undo: unknown, answerKey: Record<string, unknown> | undefined): boolean {
  const patch = parseUndoPatch(undo);
  if (!patch || patch.op !== "setAnswer") return false;
  return sameAnswerValue(answerKey?.[patch.slotId], patch.value);
}


/** 需要用户处理的项：实质变化 / 证据不足，以及无法验证。 */
export function reviewItems(view: RecognitionDecisionViewV1 | undefined): RecognitionDecisionItemV1[] {
  return visibleDecisionItems(view).filter((item) => item.resolution === "needs_review" || item.resolution === "unverifiable");
}

/**
 * 按依赖组聚合。`dependencyGroup` 相同的项必须同时接受或同时拒绝：
 * 单独接受会造成结构损坏（例如「插入槽位」+「写答案」只做一半）。
 */
export function groupByDependency(items: readonly RecognitionDecisionItemV1[]): DecisionGroup[] {
  const groups = new Map<string, RecognitionDecisionItemV1[]>();
  const order: string[] = [];
  for (const item of items) {
    const key = item.dependencyGroup ?? `solo:${item.decisionId}`;
    if (!groups.has(key)) {
      groups.set(key, []);
      order.push(key);
    }
    groups.get(key)!.push(item);
  }
  return order.map((key) => ({ groupId: key.startsWith("solo:") ? null : key, items: groups.get(key)! }));
}

/** 无法验证的项不允许「采用修正」，只能「保持现状」。 */
export function canAccept(item: RecognitionDecisionItemV1): boolean {
  if (item.resolution === "unverifiable") return false;
  return item.status === "open";
}

/** 已决策过的项不再重复操作（幂等展示）。 */
export function isDecided(item: RecognitionDecisionItemV1): boolean {
  return item.status === "accepted" || item.status === "rejected" || item.status === "failed";
}

export function decisionActionLabel(item: RecognitionDecisionItemV1): { accept: string; keep: string } {
  if (item.resolution === "unverifiable") return { accept: "", keep: "保持现状" };
  return { accept: "采用修正", keep: "保持现状" };
}

/** 定位到题面时使用的 id（优先节点/答案位，其次题组，最后整篇）。 */
export function decisionTargetId(item: RecognitionDecisionItemV1): string {
  return item.target.nodeId || item.target.targetId || item.target.taskId || "exam";
}

/** 问题列表与发布门禁共用的「待处理」条数。 */
export function pendingDecisionCount(view: RecognitionDecisionViewV1 | undefined): number {
  return reviewItems(view).filter((item) => item.status === "open").length;
}

/**
 * 四路链路是否至少跑过一路。
 * 都没跑时列表必然是空的，此时把空列表说成「没有问题」就是假信息——
 * 只能说明「还没有可核对的结果」。
 */
export function hasAnyChainRun(view: RecognitionDecisionViewV1 | undefined): boolean {
  if (!view) return false;
  return view.localStatus !== "not_started" || view.cloudStatus !== "not_started" || view.items.length > 0;
}

/** 空列表时该说哪句话。 */
export function emptyStateMessage(view: RecognitionDecisionViewV1 | undefined): string {
  const review = reviewItems(view);
  if (review.length) return "这些问题都已经处理过了。";
  return hasAnyChainRun(view)
    ? "识别结果没有需要你确认的地方。"
    : "识别还没有产出可核对的结果，这里暂时没有建议可看。";
}

/** 过期批次：用户已经改过题稿，这批建议不能直接应用，必须显式提示并允许重新核验。 */
export function describeStaleness(view: RecognitionDecisionViewV1 | undefined): string | undefined {
  if (!view?.stale) return undefined;
  return `这批识别建议基于 v${view.baseEditVersion} 生成，题稿已经改到 v${view.currentEditVersion}。建议里与改动冲突的部分不会覆盖你的修改，需要重新核验后再处理。`;
}

/**
 * 把「当前值 / 建议值」渲染成一句话。
 * 只认契约里的 `{kind:'option'|'text', ...}` 形状；未知形状原样 JSON，
 * 不猜测、不美化，避免把看不懂的东西显示成「没问题」。
 */
export function formatDecisionValue(value: unknown): string {
  if (value === null || value === undefined) return "（无）";
  if (typeof value === "string") return value || "（空）";
  if (Array.isArray(value)) return value.map((entry) => formatDecisionValue(entry)).join(", ") || "（空）";
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    if (record.kind === "option" && Array.isArray(record.labels)) {
      return record.labels.length ? record.labels.map(String).join(", ") : "（空）";
    }
    if (record.kind === "text" && Array.isArray(record.values)) {
      const joined = record.values.map((entry) => String(entry ?? "").trim()).filter(Boolean).join(", ");
      return joined || "（空）";
    }
    return JSON.stringify(record);
  }
  return String(value);
}

/** 证据面的一句话摘要（哪一路、哪一页、引文）。 */
export function formatEvidence(evidence: RecognitionDecisionItemV1["evidence"]): string[] {
  if (!evidence?.length) return [];
  const chainLabel: Record<string, string> = { local: "本机识别", cloud: "云端识别", source: "原文件" };
  return evidence.map((entry) => {
    const where = [chainLabel[entry.chain] ?? entry.chain, entry.pageIndex ? `第 ${entry.pageIndex + 1} 页` : null]
      .filter(Boolean)
      .join(" · ");
    return entry.quote ? `${where}：${entry.quote}` : where;
  });
}
