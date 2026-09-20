import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, Undo2, Redo2, MoreHorizontal, FileSearch, X } from "lucide-react";
import { command, getJob } from "../../api/tauriCommands";
import { retryProcessing, cancelProcessing, subscribeProcessing } from "../../api/processingClient";
import { chooseExportDirectory } from "../../api/desktopDialogs";
import { describePublishError, publishItem } from "../../api/publishClient";
import { go, libraryPath, type LibraryIntent } from "../../app/router";
import { ExamCanvas } from "../../exam-canvas/ExamCanvas";
import { compileStructureAction } from "../../exam-canvas/structureActions";
import { SelectionInspector } from "./SelectionInspector";
import type { IeltsAuthoringIRV2, JobDetail } from "../../types";
import { readAppSettings, writeAppSettings } from "../settings/appSettings";
import { blockerCount, deriveActionableIssues, mergePublishGateIssues } from "./actionableIssues";
import {
  buildUserTasks,
  rootCausesOf,
  splitVisibleTasks,
  type UserTaskActionV1,
  type UserTaskV1
} from "./userTasks";
import { compilePreviewSource, describePreviewPublishLimitation } from "./studentPreview";
import { RecognitionPanel } from "./RecognitionPanel";
import { useCanonicalEditor } from "./useCanonicalEditor";
import { describeDeferredRemoteRefresh } from "./remoteVersion";
import { toUserFacingError } from "../../utils/userFacingError";
import { getPublishPreflight, type PublishCheckResultV1 } from "../../api/workspaceClient";
import { answerPageStatusOf } from "./answerPageStatus";

// 题目工作区（计划 §16.6 / §9.10）。
// 打开就是最终 IELTS 题面；左侧 passage、右侧 questions 由 ExamCanvas 渲染。
// 顶部有「编辑 / 学生预览」开关：两者渲染同一份草稿、同一套交互语义，只有作答状态来源不同。
// 已取代的页面：LibraryExamDetail、UnifiedPreview、StructuredAuthoringEditorV2 的主职责。

const SAVE_LABEL = {
  idle: "",
  saving: "正在保存…",
  saved: "已保存",
  // 冲突与失败对用户是**同一件事**：这次没存上，再试一次。
  // 「保存冲突」是内部机制的说法（本轮任务书第一节），不进普通界面。
  failed: "保存失败，请重试",
  conflict: "保存失败，请重试"
} as const;

/** 降级文案分层（计划 §9.10 / findings F-M0-3）：普通用户只看到人话，
 *  原始错误码与路径只在开发者模式下作为附注出现。 */
function describeLoadError(raw?: string): string {
  if (!raw) return "这道题还没有可编辑的题稿。";
  if (raw.includes("AUTHORING_V2_NOT_AVAILABLE") || raw.includes("ITEM_DS_NOT_SEEDED")) {
    return "这道题还没有生成可编辑的题稿。运行本地识别后就能编辑。";
  }
  if (raw.includes("ITEM_NOT_FOUND")) return "这道题已不在题库中，请返回题库刷新。";
  return "这道题暂时打不开，请稍后重试。";
}

export function ExamWorkspacePage({ itemId, intent }: { itemId: string; intent?: LibraryIntent }) {
  const editor = useCanonicalEditor(itemId);
  const [detail, setDetail] = useState<JobDetail | undefined>();
  const [sourceOpen, setSourceOpen] = useState(false);
  const [issuesOpen, setIssuesOpen] = useState(false);
  /** 最近一次「点了问题却在题面上找不到位置」的目标 id；用于给出如实说明而不是静默无反应。 */
  const [locateMiss, setLocateMiss] = useState<string | undefined>();
  const [recognitionOpen, setRecognitionOpen] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | undefined>();
  const [busyAction, setBusyAction] = useState<string | undefined>();
  const [notice, setNotice] = useState<string | undefined>();
  const [noticeDetail, setNoticeDetail] = useState<string | undefined>();
  const [titleEditing, setTitleEditing] = useState(false);
  const [preflight, setPreflight] = useState<PublishCheckResultV1 | undefined>();
  // 预检拉取失败必须让用户看见：`mergePublishGateIssues` 在 preflight 为 undefined 时
  // 只返回本地启发式检查，界面会显示「没有需要确认的问题」，而点发布仍会被服务端拦下。
  const [preflightError, setPreflightError] = useState<string | undefined>();
  // 窄窗（<980px）下两栏改为顶部 tab 切换，而不是把 passage 与 questions 堆成一长列。
  const [narrowPane, setNarrowPane] = useState<"passage" | "questions">("questions");
  // 编辑 / 学生预览。预览不是新页面，而是同一工作区里的另一个渲染模式。
  const [mode, setMode] = useState<"edit" | "student">("edit");
  // 学生预览的作答状态重置令牌：草稿一变就重新挂载预览，避免把旧题面的作答带到新题面。
  const previewTokenRef = useRef(0);
  const [previewToken, setPreviewToken] = useState(0);
  /** 本题每收到一次处理事件就 +1。识别建议面板靠它感知「识别阶段推进/结果落地」。 */
  const [processingTick, setProcessingTick] = useState(0);
  /** 学生预览里「答案类型不匹配」超过 8 处时是否展开（此前是硬截断「仅显示前 8 处」）。 */
  const [previewIssuesExpanded, setPreviewIssuesExpanded] = useState(false);

  useEffect(() => {
    previewTokenRef.current += 1;
    setPreviewToken(previewTokenRef.current);
  }, [editor.draft]);

  useEffect(() => {
    getJob(itemId).then(setDetail).catch(() => setDetail(undefined));
  }, [itemId]);

  useEffect(() => {
    let stopped = false;
    let stop: (() => void) | undefined;
    subscribeProcessing((update) => {
      if (update.itemId !== itemId) return;
      // 每次阶段推进/终态落地都记一票，作为识别建议面板的重拉信号（见下）。
      setProcessingTick((value) => value + 1);
      getJob(itemId).then(setDetail).catch(() => {});
      // 是否重拉由编辑器判定，这里**不**用 `pendingCount` 提前短路。
      //
      // 以前这里写的是 `if (!editor.pendingCount) editor.reload()`：有未保存修改时
      // 整条事件被丢掉，且丢得没有痕迹——云端在后台自主修复了内容，编辑器永远不会
      // 知道，用户继续在过期题面上改，直到某次保存撞上版本冲突才发现。
      // 现在把事件携带的 `editVersion` 交给编辑器：它据此区分「自己保存的回声」
      // （忽略）、「本地脏时的远端变更」（先记下，保存排空后补读）与「本地干净时的
      // 远端变更」（立即重读）。规则见 `remoteVersion.ts`。
      editor.noteRemoteVersion(update.editVersion);
    }).then((unlisten) => { if (stopped) unlisten(); else stop = unlisten; }).catch(console.error);
    return () => { stopped = true; stop?.(); };
  }, [itemId, editor.noteRemoteVersion]);

  const localIssues = useMemo(() => deriveActionableIssues(editor.draft), [editor.draft]);
  const issues = useMemo(() => mergePublishGateIssues(localIssues, preflight), [localIssues, preflight]);
  const blockers = blockerCount(issues);
  // 普通界面只呈现**任务**，不呈现原始问题行（本轮任务书第二节）：
  // 连续缺答并成区间、同一题组内部问题并成一条、泛化行在具体问题存在时隐藏。
  const taskSummary = useMemo(() => buildUserTasks(editor.draft, issues), [editor.draft, issues]);
  const [tasksExpanded, setTasksExpanded] = useState(false);
  const visibleTasks = useMemo(
    () => splitVisibleTasks(taskSummary.tasks, tasksExpanded),
    [taskSummary.tasks, tasksExpanded]
  );

  // 学生预览：先走产品真正使用的编译器校验当前草稿。编译失败就**不**渲染预览，
  // 而是给出可定位的问题，避免用户对着过期画面继续编辑。
  const preview = useMemo(() => compilePreviewSource(editor.draft), [editor.draft]);
  const previewLimitation = useMemo(
    () => describePreviewPublishLimitation({
      pendingCount: editor.pendingCount,
      blockerCount: blockers,
      // 预览能渲染 ≠ 学生端能提交：答案键类型不匹配时题面照常画出，但真实学生端会在
      // 提交阶段拒绝整份提交。这个数字必须进限制说明，否则预览就是「假完成」。
      runtimeIssueCount: preview?.ok ? preview.summary.answerKeyIssues.length : 0
    }),
    [editor.pendingCount, blockers, preview]
  );

  // 「可以导出」的**唯一**判据是当前题稿的后端发布检查（任务书第 5 条）：
  //   - 任务列表为空（后端门禁的每个阻断都已被某条任务接住，或被判定为泛化重复）；
  //   - 门禁**确实读到了**（`preflight !== undefined` 且 `preflightError` 为空）。
  //     读不到时说「可以导出」就是拿一次失败的检查冒充通过；而门禁**还没回来**时
  //     说「可以导出」更糟——那是在用「还没查」冒充「查过了没问题」（本轮实测就撞上了
  //     这一条：面板挂载即显示「可以导出」，而同一时刻后端门禁报 34 条阻断）。
  //   - 没有待保存修改（`pendingCount === 0`）——门禁评的是**已保存的权威稿**，
  //     草稿还有没落盘的东西时，它评的不是用户眼前这份。
  const canExport = taskSummary.tasks.length === 0 && Boolean(preflight) && !preflightError && editor.pendingCount === 0;
  // 门禁的三种状态，供界面如实措辞，也给验收脚本一个可等待的锚点。
  const preflightState: "loading" | "loaded" | "error" = preflightError ? "error" : preflight ? "loaded" : "loading";
  const answerPageStatus = useMemo(
    () => answerPageStatusOf(detail?.pipelineReport),
    [detail?.pipelineReport]
  );

  // 识别建议的重拉时机：保存完成、版本变化、识别阶段推进。
  //
  // **`processingTick` 是必需的，不是装饰**（F-R14-1）：识别建议（批次）是裁决之后才落盘的，
  // 而落地那一刻 `version`/`pendingCount`/`saveState` 都不会变，`job.currentStep` 也不会变
  // ——它在本地识别结束时就已是 `Authoring`，之后的云端识别、原文件核验、裁决都不再动它
  // （实测 `job.json` 的终值就是 `Authoring`）。只用前四个分量时，面板会永远停在
  // 「识别还没有产出可核对的结果」，而 IPC 早已能读到几十条候选：界面在说假话。
  // 处理事件覆盖了「阶段推进」与「终态落地」，正是缺的那个分量。
  const recognitionRefreshKey = `${editor.version}:${editor.pendingCount}:${editor.saveState}:${detail?.job.currentStep ?? ""}:${processingTick}`;

  /** 把问题/建议定位到题面上的对应节点（与问题列表同一套 data-* 约定）。 */
  /** 点击问题定位到题面上对应的位置。返回是否真的找到了可定位的元素。 */
  function locateTarget(targetId: string): boolean {
    setSelectedId(targetId);
    // 答案位的 id（如 `q27`）**不一定**出现在 DOM 上：completion 的答案位是**行内**渲染在
    // stimulus 里的，宿主元素带的是**内容节点 id**（`data-editor-id`），不是 slotId；
    // 只有非行内列表版式才给元素加 `data-question-id={slotId}`。
    // 因此除了 slotId，还要按 `answerSlots[slotId].hostNodeId` 再找一次 ——
    // 否则「第 27 题还没有答案」这条阻断项点了没有任何反应（实测确实如此）。
    const hostNodeId = editor.draft?.answerSlots?.[targetId]?.hostNodeId;
    // **第三跳：内容节点 id**。实测 `demanding-reading-passage-3.pdf` 上，
    // `answerSlots["q27"].hostNodeId` 是 **stimulus 节点**（`group-1-stimulus-b032`），
    // 而真正渲染答案输入框的那个节点是 `taskGroups[0].stimulus[1].children[3]`
    // （id = `slot-node-q27`，`type = answer_slot`，带 `slotId`）。前两跳都落空，
    // 于是「去填写」只给出「找不到」——按钮没坏，但它没能把用户送到该填的地方。
    // 这里直接在草稿的题组里按 `slotId` 找回承载该答案位的内容节点 id。
    const contentNodeIds = contentNodeIdsForSlot(editor.draft, targetId);
    const candidates = [targetId, hostNodeId, ...contentNodeIds]
      .filter((value): value is string => typeof value === "string" && value.length > 0)
      .filter((value, index, all) => all.indexOf(value) === index);
    const target = Array.from(document.querySelectorAll<HTMLElement>(
      "[data-editor-id], [data-question-id], [data-response-group-id]"
    )).find((element) => [
      element.dataset.editorId,
      element.dataset.questionId,
      element.dataset.responseGroupId
    ].some((value) => value !== undefined && candidates.includes(value)));
    target?.scrollIntoView({ block: "center", behavior: "smooth" });
    // 文档级问题（`SIGNIFICANT_REGION_UNASSIGNED` / `RUNTIME_COMPILER_FAILED` 的 targetId 是
    // "document"）在题面上没有对应元素。以前这里静默什么都不做，用户会以为按钮坏了；
    // 现在如实说明这条问题不在题面上、需要别的手段处理。
    setLocateMiss(target ? undefined : targetId);
    return Boolean(target);
  }

  /**
   * 执行一条用户任务上的动作。
   *
   * 任务书第 5 条：**每个按钮必须有真实作用**。三个动作都绑到工作区里真实存在的能力上，
   * 没有一个按钮是「点了不改门禁」的装饰：
   *   - `fill-answer`  → 把答案控件滚进视野并选中（用户接着在题面上填）；
   *   - `view-source`  → 打开原文件抽屉，并把题面上的对应题组滚进视野；
   *   - `retry-recognition` → 真的重新入队识别，并**重新读取后端结果**（问题列表与
   *     识别建议都以后端为权威重算，而不是本地把卡片抹掉）。
   *
   * 操作后**不**在本地删卡片：门禁没变就还得显示。问题只有在后端结果确实变了之后才消失。
   */
  async function runTaskAction(task: UserTaskV1, action: UserTaskActionV1) {
    if (action.id === "fill-answer") {
      // 定位失败时 `locateTarget` 会给出「这条不在题面上」的如实说明，不会静默无反应。
      locateTarget(action.targetId);
      return;
    }
    if (action.id === "view-source") {
      setSourceOpen(true);
      locateTarget(action.targetId);
      return;
    }
    await withBusy(`task:${task.taskId}`, async () => {
      // 先把未落盘的编辑刷进权威稿，再重新识别——否则识别评的是旧稿，结果会立刻过期。
      await editor.flush();
      await retryProcessing(itemId);
      // 重新入队后**立刻重读后端结果**：门禁与识别建议都可能已经变了，
      // 界面必须跟着后端走，而不是等用户手动刷新。
      editor.reload();
      const result = await getPublishPreflight(itemId).catch(() => undefined);
      if (result) {
        setPreflight(result);
        setPreflightError(undefined);
      }
      setNotice("已重新加入识别队列。识别完成后这里的问题会按新的结果重算。");
    });
  }

  // 发布门禁是后端对「已保存的权威稿」的判断，也是点「发布」时真正会拦下的东西。
  // 待保存队列清空后重新取一次，让界面问题列表与发布门禁保持一致。
  // `editor.loading` 也是依赖：工作区加载会播种权威稿，加载完成后门禁必须重新求值，
  // 否则一次早于加载完成的预检会一直停在过期结论上。
  useEffect(() => {
    if (editor.pendingCount > 0) return;
    let cancelled = false;
    getPublishPreflight(itemId)
      .then((result) => {
        if (cancelled) return;
        setPreflight(result);
        setPreflightError(undefined);
      })
      .catch((error) => {
        if (cancelled) return;
        // 读不到门禁时列表必然不完整，必须显式降级，而不是留一个绿色的空列表。
        setPreflight(undefined);
        setPreflightError(toUserFacingError(error, "暂时读不到发布检查结果。").userMessage);
      });
    return () => { cancelled = true; };
  }, [itemId, editor.pendingCount, editor.saveState, editor.loading]);

  useEffect(() => {
    if (intent === "publish") setNotice("检查下面的问题后，点右上角「发布」把这道题发到 NAS。");
  }, [intent]);

  /** 失败提示一律经用户文案层收敛；机器码/路径只进日志与开发者附注（audit A7-F04）。 */
  function showError(error: unknown, fallback?: string) {
    const { userMessage, internalDetail } = toUserFacingError(error, fallback);
    if (internalDetail && internalDetail !== userMessage) console.error("[workspace]", internalDetail);
    setNotice(userMessage);
    setNoticeDetail(internalDetail);
  }
  function clearNotice() {
    setNotice(undefined);
    setNoticeDetail(undefined);
  }

  async function withBusy(key: string, work: () => Promise<void>) {
    setBusyAction(key);
    setMenuOpen(false);
    try {
      await work();
    } catch (error) {
      // work() 的所有后续步骤（导航/发布等）都排在失败 await 点之后，失败时
      // 本就不会执行，防导航靠的是这个结构而不是 re-throw；onClick 产生的
      // promise 无人接住，re-throw 只会变成 unhandledrejection（5548f7a 的
      // 错误归因，D0 复核修正）。这里只负责把失败暴露给用户。
      showError(error);
    } finally {
      setBusyAction(undefined);
    }
  }

  async function publish() {
    await withBusy("publish", async () => {
      await editor.flush();
      let destination = readAppSettings().nasDestination;
      if (!destination) {
        const picked = await chooseExportDirectory();
        if (!picked) {
          setNotice("请先在设置页选择 NAS 目录，或在这里选一次。");
          return;
        }
        writeAppSettings({ nasDestination: picked });
        destination = picked;
      }
      const outcome = await publishItem(itemId, destination);
      setNotice(outcome.ok ? `发布完成：${outcome.examId ?? itemId}` : outcome.message ?? "发布失败。");
    });
  }

  const title = editor.draft?.exam.title ?? detail?.job.title ?? itemId;
  const processingNote = detail?.job.currentStep === "LlmReview" ? "本地已完成 · 云端识别中" : undefined;

  return (
    <section
      className="workspace-page"
      data-testid="exam-workspace"
      // 当前已保存版本号只作为**机器可读**的并发/隔离判据存在，不进任何用户可见文本
      // （本轮任务书第一节）。验收脚本用它断言「在学生预览里作答不会产生新的编辑修订」——
      // 那条断言此前读的是可见文案里的 `v7`，去版本化之后必须换成这个属性，
      // 否则它会退化成「两次读到同一段静态文字」，永远通过。
      data-edit-version={editor.version}
      data-pending-count={editor.pendingCount}
    >
      <header className="workspace-header">
        <div className="workspace-header-left">
          <button
            className="workspace-back-button"
            data-testid="workspace-back"
            onClick={() => withBusy("leave", async () => {
              await editor.flush();
              go(libraryPath());
            })}
            aria-label="返回题库"
            title="返回题库"
          >
            <ArrowLeft size={16} />
          </button>
          <span className="workspace-brand">IELTS</span>
          <div className="workspace-title">
            <EditableTitle
              title={editor.title ?? title}
              editing={titleEditing}
              onBegin={() => setTitleEditing(true)}
              onCommit={(next) => {
                setTitleEditing(false);
                editor.setTitle(next);
              }}
              onCancel={() => setTitleEditing(false)}
            />
            {processingNote ? <small>{processingNote}</small> : null}
          </div>
        </div>

        <div className="workspace-header-right">
          <div className="workspace-header-actions">
            {editor.saveState !== "idle" ? (
              <span className={`save-state ${editor.saveState}`} data-testid="workspace-save-state">
                {SAVE_LABEL[editor.saveState]}
              </span>
            ) : null}
            <button data-testid="workspace-source" aria-label="查看原文件" onClick={() => setSourceOpen(true)}><FileSearch size={16} /></button>
            <button
              className={blockers ? "has-blockers" : ""}
              data-testid="workspace-issues"
              aria-expanded={issuesOpen}
              // 两个辅助面板**互斥**：打开一个就关掉另一个。它们共用 `.workspace-aside`
              // 这一个有总高度上限的区域，同时展开会把题稿挤到只剩一百多像素。
              onClick={() => setIssuesOpen((open) => {
                const next = !open;
                if (next) setRecognitionOpen(false);
                return next;
              })}
              aria-label={!taskSummary.ready
                ? "正在检查需要处理的问题"
                : taskSummary.blockerCount
                  ? `还有 ${taskSummary.tasks.length} 处需要处理，其中阻断 ${taskSummary.blockerCount} 处`
                  : `还有 ${taskSummary.tasks.length} 处需要处理`}
            >
              {/* 题稿还没打开时不报「问题 0」——那会把「还没查」说成「查过了，没有问题」。 */}
              {taskSummary.ready
                ? `问题 ${taskSummary.tasks.length}${taskSummary.blockerCount ? ` · 阻断 ${taskSummary.blockerCount}` : ""}`
                : "问题 …"}
            </button>
            <button
              data-testid="workspace-recognition-toggle"
              aria-expanded={recognitionOpen}
              // 与「问题」按钮对称的互斥逻辑（见上）。
              onClick={() => setRecognitionOpen((open) => {
                const next = !open;
                if (next) setIssuesOpen(false);
                return next;
              })}
            >识别建议</button>
            <button title="撤销" aria-label="撤销" disabled={!editor.canUndo} onClick={editor.undo}><Undo2 size={16} /></button>
            <button title="重做" aria-label="重做" disabled={!editor.canRedo} onClick={editor.redo}><Redo2 size={16} /></button>
            <button data-testid="workspace-publish" disabled={Boolean(busyAction)} onClick={publish}>
              {busyAction === "publish" ? "正在发布…" : "发布"}
            </button>
            <button aria-label="更多操作" title="更多操作" onClick={() => setMenuOpen((open) => !open)}><MoreHorizontal size={16} /></button>
          </div>

          {menuOpen ? (
            <div className="workspace-menu" role="menu">
              <button role="menuitem" onClick={() => withBusy("local", async () => {
                await editor.flush();
                await retryProcessing(itemId);
                setNotice("已加入识别队列。");
              })}>重新识别</button>
              <button role="menuitem" onClick={() => withBusy("cancel", async () => {
                await cancelProcessing(itemId);
                setNotice("已请求停止识别。");
              })}>停止识别</button>
            </div>
          ) : null}
        </div>
      </header>

      <div className="workspace-sub-header">
        <span className="workspace-sub-header-label">READING</span>
        <div className="workspace-mode-toggle" role="tablist" aria-label="编辑与学生预览">
          <button
            role="tab"
            aria-selected={mode === "edit"}
            className={mode === "edit" ? "active" : ""}
            data-testid="workspace-mode-edit"
            onClick={() => setMode("edit")}
          >编辑</button>
          <button
            role="tab"
            aria-selected={mode === "student"}
            className={mode === "student" ? "active" : ""}
            data-testid="workspace-mode-student"
            onClick={() => setMode("student")}
          >学生预览</button>
        </div>
        <span className="workspace-sub-header-meta">
          {mode === "edit" ? "编辑模式 · 保存后点击「发布」输出到 NAS" : "学生预览 · 与学生端同一套交互语义，这里的作答不会写回题目"}
        </span>
      </div>

      {notice ? (
        <p className="workspace-notice" role="status">
          {notice}
          {noticeDetail && noticeDetail !== notice && readAppSettings().developerMode ? (
            <small className="workspace-notice-detail">技术详情：{noticeDetail}</small>
          ) : null}
          <button className="ghost small" onClick={clearNotice} aria-label="关闭提示">×</button>
        </p>
      ) : null}
      {editor.saveMessage ? <p className="workspace-notice warning" role="alert">{editor.saveMessage}</p> : null}
      {editor.deferredRemoteRefresh ? (
        <p className="workspace-notice" role="status" data-testid="workspace-remote-pending">
          {describeDeferredRemoteRefresh(editor.deferredRemoteRefresh)}
        </p>
      ) : null}
      {editor.saveNotice ? (
        <p className="workspace-notice warning" role="alert" data-testid="workspace-save-notice">
          {editor.saveNotice}
          <button className="ghost small" onClick={editor.dismissSaveNotice} aria-label="知道了">×</button>
        </p>
      ) : null}
      {preflightError ? (
        <p className="workspace-notice warning" role="alert" data-testid="workspace-preflight-error">
          下面的问题列表可能不完整（{preflightError}），发布时仍会按完整规则检查。
        </p>
      ) : null}
      {answerPageStatus ? (
        <div
          className="workspace-notice warning"
          role={answerPageStatus.canRetry ? "alert" : "status"}
          data-testid="workspace-answer-page-status"
          data-answer-page-state={answerPageStatus.state}
          data-answer-page-reason={answerPageStatus.stateReason ?? ""}
        >
          <span>{answerPageStatus.message}</span>
          {answerPageStatus.detail ? <small className="workspace-notice-detail">{answerPageStatus.detail}</small> : null}
          {answerPageStatus.canRetry ? (
            <button
              className="primary small"
              data-testid="workspace-answer-page-retry"
              disabled={Boolean(busyAction)}
              onClick={() => withBusy("answer-page-retry", async () => {
                await editor.flush();
                await retryProcessing(itemId);
                editor.reload();
                setNotice("已重新加入识别队列，答案页识别会重新请求视觉服务。题稿仍可编辑。");
              })}
            >
              重试答案页识别
            </button>
          ) : null}
        </div>
      ) : null}
      {editor.saveState === "conflict" || editor.saveState === "failed" ? (
        <div className="workspace-save-recovery" data-testid="workspace-save-recovery">
          <button
            className="primary small"
            data-testid="workspace-save-retry"
            disabled={editor.conflictRecovering}
            onClick={() => { void editor.recoverFromConflict(); }}
          >
            {editor.conflictRecovering ? "正在重新应用…" : "重试保存"}
          </button>
          <button
            className="ghost small"
            data-testid="workspace-save-discard"
            disabled={editor.conflictRecovering}
            onClick={() => editor.discardLocalChanges()}
          >
            放弃本地修改并重新加载
          </button>
        </div>
      ) : null}

      {/* 两个辅助面板**互斥展开**，并共用**一个**有总高度上限的区域。
          此前它们各自带 `max-height: 34vh` / `46vh`，同时展开会吃掉约 80vh
          （56px 顶栏 + 34px 模式栏之外几乎不剩），题稿被压到约 150px。
          互斥由上面两个按钮的 onClick 保证，共用上限由 `.workspace-aside` 保证——
          两层都要有：只设互斥的话，单个面板仍可能独占大半屏；只设共用上限的话，
          两个面板仍会同时展开去抢同一个上限。 */}
      {(issuesOpen || recognitionOpen) && mode === "edit" ? (
      <div className="workspace-aside" data-testid="workspace-aside">
      {issuesOpen ? (
        <aside
          className="workspace-issues"
          aria-label="需要处理的问题"
          data-testid="workspace-issue-list"
          data-task-count={taskSummary.tasks.length}
          data-merged-rows={taskSummary.mergedRowCount}
          data-can-export={canExport ? "true" : "false"}
          data-preflight-state={preflightState}
          data-tasks-ready={taskSummary.ready ? "true" : "false"}
        >
          {/* 题稿还没读进来时**不渲染任务**。
              `preflight`（后端门禁）与草稿是两条并行的异步链，门禁完全可能先返回；此时
              `buildUserTasks` 拿不到 `answerSlots`，`slotIdsOfTarget` 一律返回空，带题号的
              缺答问题会退化成 `missing-answer:unnumbered`。任务卡看起来正常，点「去填写」
              却定位不到任何元素——因为题面上还没有那道题。实测这是**间歇**的：同一份构建、
              同一份夹具，一次任务 id 是 `missing-answer:q27+…+q40`（题号解析成功、定位命中
              `group-1-stimulus-b032`），另一次退化成 `unnumbered` 且定位失败（F-R15-5）。
              判据用 `taskSummary.ready`（领域规则，可单测），不用 `editor.loading`。 */}
          {!taskSummary.ready ? (
            <p className="empty compact" data-testid="workspace-tasks-loading">{taskSummary.headline}</p>
          ) : taskSummary.tasks.length ? (
            <>
              <p className="workspace-issues-headline" data-testid="workspace-tasks-headline">
                {taskSummary.headline}
              </p>
              <ul>
                {visibleTasks.visible.map((task) => (
                  <li
                    key={task.taskId}
                    className={task.severity}
                    data-severity={task.severity}
                    data-task-id={task.taskId}
                    data-task-kind={task.kind}
                    // 合并后的每项任务都保留**底层问题关联**（任务书第 3 条）：原始问题行不再
                    // 单独渲染，但「门禁报出的每个根因都被某条任务接住」必须仍然可查。
                    // 验收脚本据此断言「合并发生了」而不是「渲染时把行吞掉了」。
                    data-task-covers={rootCausesOf(issues, task).join(",")}
                  >
                    {task.severity === "blocker" ? <span className="severity-badge" aria-label="阻断问题">⚠</span> : null}
                    <span className="workspace-task-title" data-testid={`workspace-task-title-${task.taskId}`}>
                      {task.title}
                    </span>
                    {task.detail ? <small className="workspace-task-detail">{task.detail}</small> : null}
                    <div className="button-row">
                      {task.actions.map((action) => (
                        <button
                          key={action.id}
                          className={action.id === "fill-answer" || action.id === "retry-recognition" ? "primary small" : "ghost small"}
                          data-testid={`workspace-task-action-${task.taskId}-${action.id}`}
                          data-action-id={action.id}
                          data-action-target={action.targetId}
                          disabled={Boolean(busyAction)}
                          onClick={() => { void runTaskAction(task, action); }}
                        >
                          {busyAction === `task:${task.taskId}` ? "正在处理…" : action.label}
                        </button>
                      ))}
                    </div>
                  </li>
                ))}
              </ul>
              {/* 分组后仍然很多时才折叠，且明确告诉用户还剩几组（不再有「仅显示前 N 条」）。 */}
              {visibleTasks.hiddenCount ? (
                <button
                  className="ghost small"
                  data-testid="workspace-tasks-more"
                  onClick={() => setTasksExpanded(true)}
                >
                  还有 {visibleTasks.hiddenCount} 组问题
                </button>
              ) : null}
            </>
          ) : (
            // 「没有问题」不生成问题卡片，只保留这一句（任务书第四节）。
            // 但**只有后端发布检查确实读到了**才敢说「可以导出」。
            <p className="empty compact" data-testid="workspace-tasks-clear">
              {canExport
                ? "可以导出"
                : preflightState === "loading"
                  ? "正在检查是否还有需要处理的问题…"
                  : editor.pendingCount > 0
                    ? "正在保存修改，保存后会重新检查一遍。"
                    : "暂时读不到发布检查结果，还无法确认是否可以导出。"}
            </p>
          )}
          {locateMiss ? (
            <p className="empty compact" role="status" data-testid="workspace-locate-miss">
              {locateMiss === "document"
                // 文档级目标在题面上本来就没有对应元素，这是正常的，如实说明即可。
                ? "这条问题说的是整份题稿，页面上没有可以跳过去的位置。"
                // **不是**文档级：说明题面上确实找不到这个位置（例如填空是内联渲染在题干里的，
                // 宿主元素带的是内容节点 id 而不是答案位 id）。旧文案把这两种情况都说成
                // 「是整份文档级别」，对着一道具体题目说这种话是**假信息**，会让用户以为
                // 自己点错了地方。这里只陈述事实，不编造原因。
                : `这条问题指向的位置在当前题面上没有对应的元素（目标「${locateMiss}」），需要直接在题面上找到它并修改。`}
            </p>
          ) : null}
        </aside>
      ) : null}

      {recognitionOpen ? (
        <RecognitionPanel
          itemId={itemId}
          editVersion={editor.version}
          refreshKey={recognitionRefreshKey}
          onLocate={locateTarget}
          // 文档级剩余任务（云端读不到原文件某一块、模型留下无法定位到题面的疑问）
          // 唯一真能推进的动作就是打开原文件抽屉。
          onOpenSource={() => setSourceOpen(true)}
          onApplied={() => editor.reload()}
          // 「这一项撤销过了吗」的权威答案是后端持久化的 `status === "undone"`；
          // 权威稿的答案位只作为次要判据（兜住废弃编辑器补丁路径写下的历史数据）。
          // 撤销本身走正式后端命令，面板自己发，不再经由编辑器补丁。
          answerKey={editor.draft?.answerKey as Record<string, unknown> | undefined}
        />
      ) : null}
      </div>
      ) : null}

      <div className="workspace-pane-tabs" role="tablist" aria-label="切换原文与题目">
        <button role="tab" aria-selected={narrowPane === "passage"} className={narrowPane === "passage" ? "active" : ""} onClick={() => setNarrowPane("passage")}>原文</button>
        <button role="tab" aria-selected={narrowPane === "questions"} className={narrowPane === "questions" ? "active" : ""} onClick={() => setNarrowPane("questions")}>题目</button>
      </div>

      <div className="workspace-body" data-narrow-pane={narrowPane}>
        {editor.loading ? <p className="empty">正在打开这道题…</p> : null}
        {editor.loadError ? (
          <div className="workspace-load-error">
            <p className="error-text">{describeLoadError(editor.loadError)}</p>
            {readAppSettings().developerMode && editor.loadError ? (
              <p className="empty compact"><small>技术详情：{editor.loadError}</small></p>
            ) : null}
            <div className="button-row">
              <button className="primary small" onClick={() => withBusy("local", async () => {
                await retryProcessing(itemId);
              })}>
                运行本地识别
              </button>
            </div>
          </div>
        ) : null}
        {editor.draft && mode === "edit" ? (
          <ExamCanvas
            authoring={editor.draft}
            mode="author"
            selectedId={selectedId}
            onSelect={setSelectedId}
            onTextCommand={({ nodeId, expectedText, text }) =>
              editor.applyCommand({ op: "set_text", nodeId, expectedText, text })
            }
            onAnswerChange={(slotId, value) => editor.applyCommand({ op: "set_answer", slotId, value })}
            onStructureAction={(action) => {
              try {
                const patch = compileStructureAction(editor.draft!, action);
                if (patch) editor.applyPatch(patch);
              } catch (error) { showError(error, "这个结构修改没有生效，请重试。"); }
            }}
          />
        ) : null}

        {editor.draft && mode === "student" ? (
          <div className="workspace-student-preview" data-testid="workspace-student-preview">
            <p
              className={`workspace-notice ${previewLimitation.level === "warning" ? "warning" : ""}`}
              role="status"
              data-testid="workspace-preview-revision"
            >
              {previewLimitation.message}
            </p>
            {preview && !preview.ok ? (
              // 编译失败：不显示任何题面，只给出可定位的问题。
              <div className="workspace-preview-error" role="alert" data-testid="workspace-preview-error">
                <p className="error-text">这份草稿还不能按学生端要求编译，预览已暂停（避免显示过期画面）。</p>
                <ul>
                  <li data-preview-issue-code={preview.issue.code} data-preview-issue-target={preview.issue.targetId}>
                    {preview.issue.message}
                    <small>（{preview.issue.code} · {preview.issue.targetId}）</small>
                  </li>
                </ul>
                <div className="button-row">
                  <button className="primary small" data-testid="workspace-preview-back" onClick={() => setMode("edit")}>回到编辑</button>
                  <button
                    className="ghost small"
                    onClick={() => {
                      const target = Array.from(document.querySelectorAll<HTMLElement>(
                        "[data-editor-id], [data-question-id], [data-response-group-id]"
                      )).find((element) => [
                        element.dataset.editorId,
                        element.dataset.questionId,
                        element.dataset.responseGroupId
                      ].includes(preview.issue.targetId));
                      if (!target) {
                        setMode("edit");
                        return;
                      }
                      setMode("edit");
                      setSelectedId(preview.issue.targetId);
                      target.scrollIntoView({ block: "center", behavior: "smooth" });
                    }}
                  >定位到问题</button>
                </div>
              </div>
            ) : null}
            {preview?.ok ? (
              <>
                <p className="workspace-preview-summary" data-testid="workspace-preview-summary">
                  {preview.summary.taskGroups} 个题组 · {preview.summary.slots} 个答案位 · {preview.summary.assets} 个资源
                </p>
                {preview.summary.answerKeyIssues.length > 0 ? (
                  <div className="workspace-preview-runtime-issues" role="alert" data-testid="workspace-preview-runtime-issues">
                    <p>
                      以下答案位的答案类型与题目形式不匹配，学生提交时会被判为无效（共 {preview.summary.answerKeyIssues.length} 处）：
                    </p>
                    <ul>
                      {(previewIssuesExpanded ? preview.summary.answerKeyIssues : preview.summary.answerKeyIssues.slice(0, 8)).map((item) => (
                        <li key={`${item.code}:${item.targetId}`} data-preview-runtime-code={item.code} data-preview-runtime-target={item.targetId}>
                          <button
                            type="button"
                            className="workspace-preview-runtime-locate"
                            onClick={() => { setMode("edit"); locateTarget(item.targetId); }}
                          >
                            {item.targetId}
                          </button>
                        </li>
                      ))}
                    </ul>
                    {preview.summary.answerKeyIssues.length > 8 && !previewIssuesExpanded ? (
                      <button
                        className="ghost small"
                        data-testid="workspace-preview-runtime-more"
                        onClick={() => setPreviewIssuesExpanded(true)}
                      >
                        还有 {preview.summary.answerKeyIssues.length - 8} 处
                      </button>
                    ) : null}
                  </div>
                ) : null}
                {/* key 绑草稿版本令牌：草稿一变，预览的作答状态整体重置。 */}
                <ExamCanvas key={`student-preview-${previewToken}`} authoring={editor.draft} mode="student" />
              </>
            ) : null}
          </div>
        ) : null}
      </div>

      {editor.draft && mode === "edit" ? <SelectionInspector draft={editor.draft} selectedId={selectedId} onPatch={editor.applyPatch} onClose={() => setSelectedId(undefined)} /> : null}

      {sourceOpen ? (
        <div className="drawer-scrim" role="presentation" onClick={() => setSourceOpen(false)}>
          <aside className="drawer drawer-wide" role="dialog" aria-modal="true" aria-label="原文件" onClick={(event) => event.stopPropagation()}>
            <header className="drawer-head">
              <h2>原文件</h2>
              <button className="ghost small" onClick={() => setSourceOpen(false)} aria-label="关闭">×</button>
            </header>
            <div className="drawer-body">
              {detail?.job.sourceFiles.length ? (
                <ul className="picked-file-list">
                  {detail.job.sourceFiles.map((file) => (
                    <li key={file.fileId}>
                      <span className="file-name">{file.originalName}</span>
                      <span>{file.role === "AnswerKey" ? "答案文件" : "主文件"}</span>
                      <button className="ghost small" onClick={() => withBusy("source", async () => { await command("open_source_file", { itemId, fileId: file.fileId }); })}>打开原文件</button>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="empty compact">没有找到原文件记录。</p>
              )}
              {detail?.documentIr?.pages?.length ? (
                <div className="source-pages">
                  {detail.documentIr.pages.map((page, index) => (
                    <details key={index}>
                      <summary>第 {index + 1} 页</summary>
                      <pre>{page.blocks?.map((block) => block.text).join("\n") ?? ""}</pre>
                    </details>
                  ))}
                </div>
              ) : null}
            </div>
          </aside>
        </div>
      ) : null}
    </section>
  );
}

/**
 * 草稿里承载某个答案位的**内容节点 id**。
 *
 * 内联填空（completion）的答案输入框渲染在 stimulus 内部，宿主元素带的是**内容节点 id**
 * （`data-editor-id = "slot-node-q27"`），既不是 slotId（`q27`），也不是
 * `answerSlots["q27"].hostNodeId`（那是 **stimulus 节点** id）。只按前两者找会全部落空，
 * 「去填写」就只剩一句「找不到」。
 *
 * 只遍历 `taskGroups`：内联答案位一定在题组的 prompt / stimulus 里（passage 里不会有
 * 可作答的答案位），这样既够用又不用深走整份草稿（草稿里的 sourceAnchors 很大）。
 */
function contentNodeIdsForSlot(draft: IeltsAuthoringIRV2 | undefined, slotId: string): string[] {
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

/** 工作区标题原位编辑（计划 §9.10「标题（可编辑）」，M1 落地）。
 *  与正文编辑同一条版本化保存链：setTitle 只是排队，flush 时随命令批次一起进事务。 */
function EditableTitle({
  title,
  editing,
  onBegin,
  onCommit,
  onCancel
}: {
  title: string;
  editing: boolean;
  onBegin: () => void;
  onCommit: (next: string) => void;
  onCancel: () => void;
}) {
  if (!editing) {
    return (
      <strong className="file-name workspace-title-editable" data-testid="workspace-title">
        <span
          role="button"
          tabIndex={0}
          aria-label={`重命名：${title}`}
          onClick={onBegin}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              onBegin();
            }
          }}
        >
          {title}
        </span>
      </strong>
    );
  }
  return (
    <strong className="file-name">
      <input
        className="workspace-title-input"
        data-testid="workspace-title-input"
        defaultValue={title}
        aria-label="题目标题"
        autoFocus
        onBlur={(event) => onCommit(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            onCommit(event.currentTarget.value);
          } else if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
          }
        }}
      />
    </strong>
  );
}
