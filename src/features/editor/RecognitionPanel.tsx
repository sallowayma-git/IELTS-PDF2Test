import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  applyRecognitionDecisions,
  canUndoRepair,
  describeRepairStatus,
  describeVerificationStatus,
  getRecognitionDecision,
  undoCloudRepair,
  type DecisionBatchOutcomeV1,
  type RecognitionDecisionItemV1,
  type RecognitionDecisionViewV1
} from "../../api/recognitionClient";
import {
  autoFixedItems,
  canAccept,
  decisionActionLabel,
  decisionStatusLabel,
  decisionTargetId,
  describeStaleness,
  emptyStateMessage,
  formatDecisionValue,
  formatEvidence,
  groupByDependency,
  hasAnyChainRun,
  isDecided,
  isRecognitionQuiet,
  pendingDecisionCount,
  recognitionInFlight,
  reviewItems,
  undoState
} from "./recognitionDecisions";
import {
  isRepairPanelQuiet,
  repairHeadline,
  repairInFlight,
  repairTasks,
  usesRepairTaskList
} from "./repairTasks";
import { toUserFacingError } from "../../utils/userFacingError";

// 识别建议面板（契约 §2.3 / §2.4 / §2.6 的前端侧）。
//
// 界面只呈现**一份**统一建议集合：
//   - 顶部一行云端状态（没跑 / 在跑 / 完成 / 失败 / 不可用 + 稳定原因码）；
//   - 汇总计数（一致 / 已自动修正 / 待确认 / 无法验证）；
//   - 「已自动修正」单独一组、默认折叠，只提供撤销；撤销走**正式后端命令**
//     （`apply_recognition_decisions` 的 `undo[]`）：后端在**同一个编辑事务**里回滚权威稿
//     并把决策状态落成 `undone`，所以「值改回去了」和「决策不再算已修正」一起成立；
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
  /**
   * 打开原文件抽屉。
   *
   * 剩余任务里有一类是**文档级**的（云端读不到原文件的某一块、或模型留下了无法定位到
   * 具体题面的疑问）。它们没有题面节点可定位，唯一真能推进的动作就是让用户去看原文件。
   * 没有这个入口，那些任务就只剩一句话，用户点不动也做不完。
   */
  onOpenSource: () => void;
  /** 应用成功后通知外层重新加载权威稿（版本会变）。 */
  onApplied: () => void;
  /**
   * 当前**已保存**权威稿的答案位。
   *
   * 只作为「已撤销」的**次要**判据（见 `undoState`）：权威事实是后端持久化的
   * `status === "undone"`。留着它是因为废弃的编辑器补丁撤销只回滚了稿、没写状态，
   * 那批历史数据得靠稿里的值认出来。
   */
  answerKey: Record<string, unknown> | undefined;
}

export function RecognitionPanel({ itemId, editVersion, refreshKey, onLocate, onOpenSource, onApplied, answerKey }: RecognitionPanelProps) {
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

  // ── 结果晚到时的自刷新 ────────────────────────────────────────────────
  //
  // 面板打开的时刻通常**早于**批次落盘：批次是裁决之后才写的，本地链也会先出稿、
  // 云端继续排队。而外层给的重拉键在批次落地那一刻**全都不变**——`job.currentStep`
  // 在本地识别结束时就已是 `Authoring`，之后的云端识别、原文件核验、裁决都不会再动它
  // （实测 `job.json` 的终值就是 `Authoring`）。于是面板会一直停在
  // 「识别还没有产出可核对的结果」，而同一时刻 IPC 已经能读到几十条候选。
  //
  // 处理：只要「还没有批次」或「还有链路在排队/运行」，就按节拍自己重拉，
  // 直到结果齐了、或到达上限。上限是必须的：一个永远不会跑识别的题不该被无限轮询，
  // 而「刷新」按钮始终在，用户随时可以手动重拉。
  const POLL_INTERVAL_MS = 2500;
  const MAX_POLLS = 48; // ≈2 分钟：一次真实导入的本地+云端+裁决都在这之内落定
  // 云端修复循环的预算是**十分钟**，用上面那个 2 分钟上限会让面板在修复跑完之前
  // 停止跟随——用户看到的是「正在自动修复」，然后就再也没有下文了。
  const MAX_REPAIR_POLLS = 300; // ≈12.5 分钟，覆盖一次完整修复加上落盘余量
  const [pollCount, setPollCount] = useState(0);

  useEffect(() => {
    // 修复进行中也要盯着：批次行此刻的 `cloud_status` 是本地周期写的 `not_run`
    // （本地周期看不见云端），只看链状态会以为「已经跑完了」，于是修复进度与最终的
    // 剩余清单都不会自己出现，用户必须手动点刷新。
    const repairing = repairInFlight(view);
    if (!recognitionInFlight(view) && !repairing) return;
    if (pollCount >= (repairing ? MAX_REPAIR_POLLS : MAX_POLLS)) return;
    const timer = window.setTimeout(() => {
      setPollCount((value) => value + 1);
      void load();
    }, POLL_INTERVAL_MS);
    return () => window.clearTimeout(timer);
  }, [load, view, pollCount]);

  const autoFixed = useMemo(() => autoFixedItems(view), [view]);
  const review = useMemo(() => reviewItems(view), [view]);
  const groups = useMemo(() => groupByDependency(review), [review]);
  const staleness = describeStaleness(view);
  // 新主链（有修复记录）与旧批次（没有）走**两套**界面，判据只有「有没有修复记录」：
  //   - 有修复记录：云端已经自己改过稿，界面以修复摘要 + 剩余任务为准；
  //   - 没有：走原来的建议卡路径，老数据不会因为新链路变成空白面板。
  // 两套同时渲染就会出现「云端已改好的项下面还挂着一张要不要采用的卡」。
  const usingRepairList = usesRepairTaskList(view);
  const tasks = useMemo(() => repairTasks(view), [view]);
  const pending = usingRepairList ? tasks.length : pendingDecisionCount(view);

  // 没什么可说的就收敛成一行短状态（本轮任务书第二节）：四颗全零的计数胶囊不携带信息，
  // 却要把一整块面板高度从题稿身上拿走。只要有一条建议、一条自动修正或一次过期提示，
  // `isRecognitionQuiet` 就会返回 false，面板照常展开。
  //
  // 新路径的判据换了一份（见 `isRepairPanelQuiet`）：修复之前算出来的建议计数不能用来
  // 决定面板收不收起——那些建议正是云端刚处理过的东西。
  const quiet = usingRepairList
    ? isRepairPanelQuiet(view) && autoFixed.length === 0 && !staleness
    : isRecognitionQuiet(view);
  // 操作回执（「已采用 N 项」等）是用户刚做完动作的反馈，不能因为面板「安静」就吞掉。
  const collapsed = quiet && !notice;

  // 整轮修复撤销的忙碌键。与逐项的 groupKey 共用一个状态位，避免两个撤销并发写同一份稿。
  const REPAIR_UNDO_KEY = "cloud-repair-undo";

  async function submit(groupKey: string, items: RecognitionDecisionItemV1[], action: "accept" | "reject" | "undo") {
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
      if (result.undone.length) parts.push(`已撤销 ${result.undone.length} 项，权威稿已改回自动修正前的值`);
      if (result.replayed) parts.push("这次是重试，没有重复写入");
      // superseded 不是失败：题稿已经被用户改过，这条建议不再适用（你的修改赢了）。
      // 重复撤销也会走这里（后端 `RECOGNITION_ALREADY_RESOLVED`），不该报成错误。
      if (result.stale.length) parts.push(`${result.stale.length} 项因为题稿已经改过或已经处理过而没有再次应用，你的修改保持不变`);
      if (result.failed.length) {
        // 失败原因用**后端那句话**（它已经是给用户看的文案），但**不带错误码**：
        // 错误码是给日志和开发者用的，普通界面出现 `USER_EDITED_AFTER_APPLY`
        // 这种词只会让用户困惑（本轮任务书第一节）。
        const first = result.failed[0];
        const suffix = result.failed.length > 1 ? `（共 ${result.failed.length} 项）` : "";
        parts.push(`${first.message || "没有应用成功"}${suffix}`);
      }
      setNotice(parts.join("；") || "已记录这次处理。");
      // 写入返回里已经带了归一化后的 view（契约 §5「无需二次读取」），直接用，省一次往返。
      if (result.view) setView(result.view);
      else await load();
      // 接受与撤销都会改权威稿、递增版本 → 都要让外层重新加载。
      if (result.accepted.length || result.undone.length) onApplied();
    } catch (error) {
      setNotice(toUserFacingError(error, "这次处理没有生效，请重试。").userMessage);
    } finally {
      setBusyGroup(undefined);
    }
  }

  /**
   * 撤销一项自动修正 —— 走**正式后端命令**，不再是编辑器补丁。
   *
   * 编辑器补丁那条老路只能改权威稿，**改不动决策状态**：值回去了、状态还是 `accepted`，
   * 界面继续把它算作「已自动修正」，重开后撤销按钮又冒出来。现在把这一项交给
   * `apply_recognition_decisions` 的 `undo[]`，后端在同一个编辑事务里回滚 + 落 `undone`。
   *
   * 强制保护也在后端：用户若在自动修正之后又改过这个槽位，后端返回
   * `USER_EDITED_AFTER_APPLY` 并拒绝回滚，界面如实显示那句话，绝不覆盖用户的改动。
   */
  function undoAutoFix(item: RecognitionDecisionItemV1) {
    return submit(item.decisionId, [item], "undo");
  }

  /**
   * 撤销**整轮**云端自动修复。
   *
   * 与上面的逐项撤销是两件事，不能混用：
   *  - 逐项撤销走 `apply_recognition_decisions` 的 `undo[]`，回滚的是**一条建议**；
   *  - 整轮撤销走 `undo_cloud_repair`（Rust 批次撤销），回滚的是**这一轮修复写下的
   *    所有修改**，依据是 journal 里的 `repairRunId`。
   *
   * 绝不能拿前端本地 undoStack 来做这件事：本地栈只覆盖用户自己的编辑，用它回滚
   * 云端写入会与批次 journal 错位，出现「界面上值回去了、后端仍认为修改在位」。
   */
  async function undoRepair() {
    if (!view?.repair?.repairRunId || busyGroup) return;
    setBusyGroup(REPAIR_UNDO_KEY);
    setNotice(undefined);
    try {
      await undoCloudRepair(view.itemId, view.repair.repairRunId, view.currentEditVersion);
      setNotice("已撤销本轮自动修复，权威稿已改回修复前的值。");
      // 撤销改了权威稿、递增版本 → 外层必须重新加载。
      onApplied();
      await load();
    } catch (error) {
      setNotice(toUserFacingError(error, "撤销没有成功，请重试。").userMessage);
    } finally {
      setBusyGroup(undefined);
    }
  }

  return (
    <aside
      className="workspace-recognition"
      aria-label="识别建议"
      data-testid="workspace-recognition"
      // 安静态（没什么可说的）供样式收紧面板自身的上下内边距：一句话不该还占一整块面板的高度。
      data-quiet={collapsed ? "true" : "false"}
    >
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
          {/* 状态行始终保留（`data-testid` 是既有验收脚本的锚点），安静时它**就是**整块面板的内容。
              四路链路一次都没跑过时，必须在这行里补上「还没有可核对的结果」——只留
              「题稿已生成，可以开始编辑」会被读成「查过了，没问题」（见 `emptyStateMessage`）。 */}
          <p
            className={`workspace-recognition-status${collapsed ? " workspace-recognition-idle" : ""}`}
            data-testid="workspace-recognition-cloud"
          >
            {describeVerificationStatus({
              localStatus: view.localStatus,
              cloudStatus: view.cloudStatus,
              // `source`/`adjudication` 必须带进来：丢掉它们就只能把「没核验」说成「核验完成」
              // （A3/A4 之后 source 会 partial、adjudication 会 not_run）。
              sourceStatus: view.sourceStatus,
              adjudicationStatus: view.adjudicationStatus,
              // 待处理条数用 `pendingDecisionCount`（含 `failed`），不是 `summary.needsReview`：
              // 后者漏掉「处理失败」的项，会让状态行说「没有需要处理的问题」而卡片还在。
              pendingCount: pending
            })}
            {collapsed && !hasAnyChainRun(view) ? ` ${emptyStateMessage(view)}` : null}
          </p>

          {/* 云端自主修复这一行**只在真的有修复记录时**出现：没有记录（旧批次、无云导入）
              不是「已修复」，但也不值得占一行位置。文案由 `describeRepairStatus` 决定，
              它保证「没有记录」不会被说成完成、`completed` 也会把剩余条数一并说出来。 */}
          {view.repair ? (
            <p className="workspace-recognition-repair" data-testid="workspace-recognition-repair">
              <span>{describeRepairStatus(view.repair)}</span>
              {canUndoRepair(view.repair) ? (
                <button
                  className="ghost small"
                  disabled={busyGroup === REPAIR_UNDO_KEY}
                  onClick={() => void undoRepair()}
                  data-testid="workspace-recognition-repair-undo"
                >
                  撤销本次自动修复
                </button>
              ) : null}
            </p>
          ) : null}

          {/* 新主链：云端已经自己改过稿，这里给的是**云端解决不了、必须由人处理**的清单。
              与下面那套建议卡互斥——两套同时渲染，用户就会看到「云端已改好的项下面还挂着
              一张要不要采用的卡」，被迫做一次没有意义的决定。 */}
          {usingRepairList ? (
            <>
              <p className="workspace-recognition-repair-headline" data-testid="workspace-recognition-repair-headline">
                {repairHeadline(view)}
              </p>
              {tasks.length ? (
                <ul className="workspace-recognition-tasks" data-testid="workspace-recognition-repair-tasks">
                  {tasks.map((task) => (
                    <li
                      key={task.taskId}
                      data-testid="workspace-recognition-repair-task"
                      data-task-id={task.taskId}
                      data-blocking={task.blocking ? "true" : "false"}
                      data-task-action={task.action}
                    >
                      <p className="workspace-recognition-task-message">{task.message}</p>
                      {task.blocking ? (
                        <p className="workspace-recognition-reason">这一处不处理就不能导出。</p>
                      ) : null}
                      {task.actions.length ? (
                        <div className="button-row">
                          {task.actions.map((action) => (
                            <button
                              key={action.id}
                              className="ghost small"
                              data-testid={`workspace-recognition-task-${action.id}-${task.taskId}`}
                              onClick={() => {
                                // 「打开原文件」是文档级任务唯一的真动作；其余都靠定位。
                                if (action.id === "open-source") {
                                  onOpenSource();
                                  return;
                                }
                                if (action.targetId) onLocate(action.targetId);
                              }}
                            >{action.label}</button>
                          ))}
                        </div>
                      ) : (
                        // 没有可定位的目标就不放按钮：点了不动的按钮比没有按钮更糟。
                        <p className="workspace-recognition-reason">
                          这一条没有对应的题面位置，请对照原文件核对后直接在题面上修改。
                        </p>
                      )}
                    </li>
                  ))}
                </ul>
              ) : null}
            </>
          ) : (
            <>
              {quiet ? null : (
                <ul className="workspace-recognition-summary" data-testid="workspace-recognition-summary">
                  <li data-count="agreed">一致 {view.summary.agreed}</li>
                  <li data-count="auto_fixed">已自动修正 {view.summary.autoFixed}</li>
                  <li data-count="needs_review">待确认 {view.summary.needsReview}</li>
                  <li data-count="unverifiable">无法验证 {view.summary.unverifiable}</li>
                </ul>
              )}
            </>
          )}

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
                    // 「已撤销」优先看后端持久化的状态，其次才是权威稿的值（见 `undoState`）。
                    const state = undoState(item, answerKey);
                    const busy = busyGroup === item.decisionId;
                    return (
                      <li
                        key={item.decisionId}
                        data-decision-id={item.decisionId}
                        data-undone={state === "undone" ? "true" : "false"}
                        data-undo-state={state}
                      >
                        <span>{item.userMessage}</span>
                        <span className="workspace-recognition-values">
                          现在：{formatDecisionValue(item.cloudValue ?? item.localValue)}
                        </span>
                        {state === "undone" ? (
                          <span className="workspace-recognition-undone" data-testid={`workspace-recognition-undone-${item.decisionId}`}>
                            已撤销，已改回自动修正前的值。
                          </span>
                        ) : state === "available" ? (
                          <button
                            className="ghost small"
                            data-testid={`workspace-recognition-undo-${item.decisionId}`}
                            disabled={Boolean(busyGroup)}
                            onClick={() => void undoAutoFix(item)}
                          >{busy ? "正在撤销…" : "撤销"}</button>
                        ) : (
                          // 后端没有可回滚的补丁就不放按钮：宁可让用户手动改回去，
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

          {/* 安静时这一句已经被并进上面的状态行，这里不再重复渲染（否则又成了一块空面板）。
              新主链不走这一句：它的「还剩什么」由上面的修复摘要那行回答。 */}
          {!usingRepairList && !quiet && pending === 0 ? (
            <p className="empty compact" data-testid="workspace-recognition-empty">
              {emptyStateMessage(view)}
            </p>
          ) : null}

          {/* 旧批次的建议卡。新主链不渲染它们（见上面 `usingRepairList` 的说明）。 */}
          {(usingRepairList ? [] : groups).map((group) => {
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
                        <p className="workspace-recognition-decided" data-status={item.status}>
                          {decisionStatusLabel(item)}
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
