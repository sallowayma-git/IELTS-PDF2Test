import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  applyRecognitionDecisions,
  describeCloudStatus,
  getRecognitionDecision,
  type DecisionBatchOutcomeV1,
  type RecognitionDecisionItemV1,
  type RecognitionDecisionViewV1
} from "../../api/recognitionClient";
import {
  autoFixedItems,
  canAccept,
  decisionActionLabel,
  decisionTargetId,
  describeStaleness,
  emptyStateMessage,
  formatDecisionValue,
  formatEvidence,
  groupByDependency,
  isDecided,
  isUndoAlreadyApplied,
  parseUndoPatch,
  pendingDecisionCount,
  reviewItems
} from "./recognitionDecisions";
import { toUserFacingError } from "../../utils/userFacingError";
import type { AuthoringPatchV2 } from "../../types";

// 识别建议面板（契约 §2.3 / §2.4 / §2.6 的前端侧）。
//
// 界面只呈现**一份**统一建议集合：
//   - 顶部一行云端状态（没跑 / 在跑 / 完成 / 失败 / 不可用 + 稳定原因码）；
//   - 汇总计数（一致 / 已自动修正 / 待确认 / 无法验证）；
//   - 「已自动修正」单独一组、默认折叠，只提供撤销；撤销走**编辑器事务**
//     （`undo` 补丁 + 版本递增），不是 reject 决策 —— reject 只改状态、不回滚权威稿，
//     用它做撤销会让界面显示「已保持现状」而自动修正仍留在稿里；
//   - 「待确认」「无法验证」逐条一张卡：当前值 vs 建议值 + 证据引文；
//     无法验证的**不给**「采用修正」，只能「保持现状」；
//   - 依赖组整组同向（单独接受会造成结构损坏）。
//
// 关键降级：云端/识别建议不可用时，本地编辑与保存必须照常可用。这里任何失败都
// 只影响本面板，不阻断编辑，也不把「没结果」显示成「没有问题」。

export interface RecognitionPanelProps {
  itemId: string;
  /** 当前已保存版本，用于展示与过期判断。 */
  editVersion: number;
  /** 保存/识别状态变化时用它触发重新拉取。 */
  refreshKey: string;
  onLocate: (targetId: string) => void;
  /** 应用成功后通知外层重新加载权威稿（版本会变）。 */
  onApplied: () => void;
  /**
   * 撤销一项自动修正：把 `undo` 当作**编辑器命令**提交，经版本化事务改回旧值。
   * 必须由外层接到编辑器的 `applyPatch` + `flush`，不能在这里直接写权威稿。
   */
  onUndoAutoFix: (patch: AuthoringPatchV2) => Promise<void>;
  /**
   * 当前**已保存**权威稿的答案位。
   *
   * 「这一项撤销过了吗」必须从它推出来（见 `isUndoAlreadyApplied`），不能用会话内状态：
   * 后端没有持久化的「已撤销」状态码，而会话内的 `Set` 一刷新就没了，
   * 界面又会把「撤销」按钮放回来 —— 那是**假完成**。
   */
  answerKey: Record<string, unknown> | undefined;
}

export function RecognitionPanel({ itemId, editVersion, refreshKey, onLocate, onApplied, onUndoAutoFix, answerKey }: RecognitionPanelProps) {
  const [view, setView] = useState<RecognitionDecisionViewV1 | undefined>();
  const [loadError, setLoadError] = useState<string | undefined>();
  const [notice, setNotice] = useState<string | undefined>();
  const [busyGroup, setBusyGroup] = useState<string | undefined>();
  const [autoFixedOpen, setAutoFixedOpen] = useState(false);
  // 幂等键按「一次用户意图」保存：重试必须复用同一个 requestId，否则后端会当成新写入。
  const requestIds = useRef(new Map<string, string>());

  const load = useCallback(async () => {
    try {
      const next = await getRecognitionDecision(itemId);
      setView(next);
      setLoadError(undefined);
    } catch (error) {
      // 命令还不存在 / 云端不可用 / 后端失败都走这里：面板降级，编辑不受影响。
      setView(undefined);
      setLoadError(toUserFacingError(error, "暂时读不到识别建议。").userMessage);
    }
  }, [itemId]);

  useEffect(() => { void load(); }, [load, refreshKey]);

  const autoFixed = useMemo(() => autoFixedItems(view), [view]);
  const review = useMemo(() => reviewItems(view), [view]);
  const groups = useMemo(() => groupByDependency(review), [review]);
  const staleness = describeStaleness(view);
  const pending = pendingDecisionCount(view);

  async function submit(groupKey: string, items: RecognitionDecisionItemV1[], action: "accept" | "reject") {
    if (!view || busyGroup) return;
    const key = `${view.batchId}:${groupKey}:${action}`;
    if (!requestIds.current.has(key)) requestIds.current.set(key, crypto.randomUUID());
    setBusyGroup(groupKey);
    setNotice(undefined);
    try {
      const result: DecisionBatchOutcomeV1 = await applyRecognitionDecisions({
        itemId: view.itemId,
        batchId: view.batchId,
        baseEditVersion: view.currentEditVersion,
        requestId: requestIds.current.get(key)!,
        decisions: items.map((item) => ({ decisionId: item.decisionId, action }))
      });
      const parts: string[] = [];
      if (result.accepted.length) parts.push(`已采用 ${result.accepted.length} 项`);
      if (result.rejected.length) parts.push(`已保持现状 ${result.rejected.length} 项`);
      if (result.replayed) parts.push("这次是重试，没有重复写入");
      // superseded 不是失败：题稿已经被用户改过，这条建议不再适用（你的修改赢了）。
      if (result.stale.length) parts.push(`${result.stale.length} 项因为题稿已经改过而没有应用，你的修改保持不变`);
      if (result.failed.length) parts.push(`${result.failed.length} 项没有应用成功（${result.failed[0].code}）`);
      setNotice(parts.join("；") || "已记录这次处理。");
      // 写入返回里已经带了归一化后的 view（契约 §5「无需二次读取」），直接用，省一次往返。
      if (result.view) setView(result.view);
      else await load();
      if (result.accepted.length) onApplied();
    } catch (error) {
      setNotice(toUserFacingError(error, "这次处理没有生效，请重试。").userMessage);
    } finally {
      setBusyGroup(undefined);
    }
  }

  /**
   * 撤销一项自动修正。
   *
   * **不能**走 reject：后端拒绝分支的语义是「只改状态，不碰权威稿」，那样界面会
   * 说「已保持现状」而权威稿里自动修正原样留着。这里把 `undo` 交给编辑器的
   * 版本化事务，值真的改回去、版本真的递增，失败也会 reject 出来如实告知。
   *
   * 这里**不再**记录「我点过撤销」：改完值之后，权威稿本身就说明了一切
   * （`isUndoAlreadyApplied` 会在渲染时按 `answerKey` 重新判定），
   * 所以重开、刷新、换会话都不会把「撤销」按钮放回来。
   */
  async function undoAutoFix(item: RecognitionDecisionItemV1, patch: AuthoringPatchV2) {
    if (busyGroup) return;
    setBusyGroup(item.decisionId);
    setNotice(undefined);
    try {
      await onUndoAutoFix(patch);
      setNotice("已把这一项改回自动修正前的值。");
    } catch (error) {
      setNotice(toUserFacingError(error, "撤销没有生效，题稿保持原样。").userMessage);
    } finally {
      setBusyGroup(undefined);
    }
  }

  return (
    <aside className="workspace-recognition" aria-label="识别建议" data-testid="workspace-recognition">
      <header className="workspace-recognition-head">
        <h2>识别建议</h2>
        <button className="ghost small" onClick={() => void load()} aria-label="刷新识别建议">刷新</button>
      </header>

      {loadError ? (
        <p className="workspace-notice warning" role="alert" data-testid="workspace-recognition-error">
          {loadError}本地编辑和保存不受影响。
        </p>
      ) : null}

      {view ? (
        <>
          <p className="workspace-recognition-status" data-testid="workspace-recognition-cloud">
            {describeCloudStatus(view.cloudStatus, view.cloudReasonCode)}
          </p>
          <ul className="workspace-recognition-summary" data-testid="workspace-recognition-summary">
            <li data-count="agreed">一致 {view.summary.agreed}</li>
            <li data-count="auto_fixed">已自动修正 {view.summary.autoFixed}</li>
            <li data-count="needs_review">待确认 {view.summary.needsReview}</li>
            <li data-count="unverifiable">无法验证 {view.summary.unverifiable}</li>
          </ul>
          <p className="workspace-recognition-meta">题稿版本 v{view.currentEditVersion} · 批次基线 v{view.baseEditVersion}</p>

          {staleness ? (
            <p className="workspace-notice warning" role="alert" data-testid="workspace-recognition-stale">
              {staleness}
            </p>
          ) : null}
          {notice ? <p className="workspace-notice" role="status" data-testid="workspace-recognition-notice">{notice}</p> : null}

          {autoFixed.length ? (
            <section className="workspace-recognition-autofixed" data-testid="workspace-recognition-autofixed">
              <button className="ghost small" aria-expanded={autoFixedOpen} onClick={() => setAutoFixedOpen((open) => !open)}>
                已自动修正 {autoFixed.length} 项{autoFixedOpen ? "（收起）" : "（展开）"}
              </button>
              {autoFixedOpen ? (
                <ul>
                  {autoFixed.map((item) => {
                    const undoPatch = parseUndoPatch(item.undo);
                    // 「已撤销」= 权威稿里那个答案位已经等于撤销补丁要改回的值。
                    // 这是**持久化事实**，不是会话内记忆：重开/刷新后同样成立。
                    const undone = isUndoAlreadyApplied(item.undo, answerKey);
                    return (
                      <li key={item.decisionId} data-decision-id={item.decisionId} data-undone={undone ? "true" : "false"}>
                        <span>{item.userMessage}</span>
                        <span className="workspace-recognition-values">
                          现在：{formatDecisionValue(item.cloudValue ?? item.localValue)}
                        </span>
                        {undone ? (
                          <span className="workspace-recognition-undone" data-testid={`workspace-recognition-undone-${item.decisionId}`}>
                            已撤销，已改回自动修正前的值。
                          </span>
                        ) : undoPatch ? (
                          <button
                            className="ghost small"
                            data-testid={`workspace-recognition-undo-${item.decisionId}`}
                            disabled={Boolean(busyGroup)}
                            onClick={() => void undoAutoFix(item, undoPatch)}
                          >{busyGroup === item.decisionId ? "正在撤销…" : "撤销"}</button>
                        ) : (
                          // 没有可用的撤销补丁就不放按钮：宁可让用户手动改回去，
                          // 也不给一个按了不生效的「撤销」。
                          <span className="workspace-recognition-undone" data-testid={`workspace-recognition-undo-unavailable-${item.decisionId}`}>
                            这条没有带可撤销的信息，请在题面上手动改回原值。
                          </span>
                        )}
                      </li>
                    );
                  })}
                </ul>
              ) : null}
            </section>
          ) : null}

          {pending === 0 ? (
            <p className="empty compact" data-testid="workspace-recognition-empty">
              {emptyStateMessage(view)}
            </p>
          ) : null}

          {groups.map((group) => {
            const groupKey = group.groupId ?? group.items[0].decisionId;
            const multi = group.items.length > 1;
            return (
              <section
                key={groupKey}
                className="workspace-recognition-group"
                data-testid="workspace-recognition-group"
                data-dependency-group={group.groupId ?? ""}
              >
                {multi ? (
                  <p className="workspace-recognition-group-hint" data-testid="workspace-recognition-group-hint">
                    这一组 {group.items.length} 项必须一起处理，单独处理会破坏结构。
                  </p>
                ) : null}
                {group.items.map((item) => {
                  const labels = decisionActionLabel(item);
                  const decided = isDecided(item);
                  const targetId = decisionTargetId(item);
                  return (
                    <article
                      key={item.decisionId}
                      className={`workspace-recognition-card severity-${item.severity}`}
                      data-testid="workspace-recognition-card"
                      data-decision-id={item.decisionId}
                      data-resolution={item.resolution}
                      data-status={item.status}
                      data-target-id={targetId}
                    >
                      <header>
                        <strong>{item.title}</strong>
                        <button className="ghost small" data-testid={`workspace-recognition-locate-${item.decisionId}`} onClick={() => onLocate(targetId)}>
                          定位
                        </button>
                      </header>
                      <p>{item.userMessage}</p>
                      {formatEvidence(item.evidence).length ? (
                        <ul className="workspace-recognition-evidence">
                          {formatEvidence(item.evidence).map((line, index) => <li key={index}>{line}</li>)}
                        </ul>
                      ) : null}
                      <p className="workspace-recognition-values" data-testid={`workspace-recognition-values-${item.decisionId}`}>
                        当前：{formatDecisionValue(item.localValue)}
                        {item.resolution === "unverifiable" ? "" : ` → 建议：${formatDecisionValue(item.cloudValue)}`}
                      </p>
                      {item.resolution === "unverifiable" ? (
                        <p className="workspace-recognition-reason">
                          无法验证，不能替你选一个版本{item.reasonCode ? `（${item.reasonCode}）` : ""}。
                        </p>
                      ) : null}
                      {decided ? (
                        <p className="workspace-recognition-decided">
                          {item.status === "accepted" ? "已采用" : item.status === "rejected" ? "已保持现状" : `处理失败（${item.code}）`}
                        </p>
                      ) : (
                        <div className="button-row">
                          {canAccept(item) ? (
                            <button
                              className="primary small"
                              data-testid={`workspace-recognition-accept-${item.decisionId}`}
                              disabled={Boolean(busyGroup)}
                              onClick={() => void submit(groupKey, group.items, "accept")}
                            >{busyGroup === groupKey ? "正在应用…" : labels.accept}</button>
                          ) : null}
                          <button
                            className="ghost small"
                            data-testid={`workspace-recognition-keep-${item.decisionId}`}
                            disabled={Boolean(busyGroup)}
                            onClick={() => void submit(groupKey, group.items, "reject")}
                          >{labels.keep}</button>
                        </div>
                      )}
                    </article>
                  );
                })}
              </section>
            );
          })}
        </>
      ) : null}
    </aside>
  );
}
