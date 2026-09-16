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
import type { JobDetail } from "../../types";
import type { AuthoringPatchV2 } from "../../types";
import { applyAuthoringV2Patches } from "../../services/authoringV2Patches";
import { readAppSettings, writeAppSettings } from "../settings/appSettings";
import { blockerCount, deriveActionableIssues, mergePublishGateIssues } from "./actionableIssues";
import { compilePreviewSource, describePreviewPublishLimitation } from "./studentPreview";
import { RecognitionPanel } from "./RecognitionPanel";
import { useCanonicalEditor } from "./useCanonicalEditor";
import { toUserFacingError } from "../../utils/userFacingError";
import { getPublishPreflight, type PublishCheckResultV1 } from "../../api/workspaceClient";

// 题目工作区（计划 §16.6 / §9.10）。
// 打开就是最终 IELTS 题面；左侧 passage、右侧 questions 由 ExamCanvas 渲染。
// 顶部有「编辑 / 学生预览」开关：两者渲染同一份草稿、同一套交互语义，只有作答状态来源不同。
// 已取代的页面：LibraryExamDetail、UnifiedPreview、StructuredAuthoringEditorV2 的主职责。

const SAVE_LABEL = {
  idle: "",
  saving: "正在保存",
  saved: "已保存",
  failed: "保存失败",
  conflict: "保存冲突"
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
    subscribeProcessing((id) => {
      if (id !== itemId) return;
      getJob(itemId).then(setDetail).catch(() => {});
      if (!editor.pendingCount) editor.reload();
    }).then((unlisten) => { if (stopped) unlisten(); else stop = unlisten; }).catch(console.error);
    return () => { stopped = true; stop?.(); };
  }, [itemId, editor.pendingCount, editor.reload]);

  const localIssues = useMemo(() => deriveActionableIssues(editor.draft), [editor.draft]);
  const issues = useMemo(() => mergePublishGateIssues(localIssues, preflight), [localIssues, preflight]);
  const blockers = blockerCount(issues);

  // 学生预览：先走产品真正使用的编译器校验当前草稿。编译失败就**不**渲染预览，
  // 而是给出可定位的问题，避免用户对着过期画面继续编辑。
  const preview = useMemo(() => compilePreviewSource(editor.draft), [editor.draft]);
  const previewLimitation = useMemo(
    () => describePreviewPublishLimitation({
      pendingCount: editor.pendingCount,
      savedVersion: editor.version,
      blockerCount: blockers,
      // 预览能渲染 ≠ 学生端能提交：答案键类型不匹配时题面照常画出，但真实学生端会在
      // 提交阶段拒绝整份提交。这个数字必须进限制说明，否则预览就是「假完成」。
      runtimeIssueCount: preview?.ok ? preview.summary.answerKeyIssues.length : 0
    }),
    [editor.pendingCount, editor.version, blockers, preview]
  );

  // 识别建议的重拉时机：保存完成、版本变化、识别阶段推进。
  const recognitionRefreshKey = `${editor.version}:${editor.pendingCount}:${editor.saveState}:${detail?.job.currentStep ?? ""}`;

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
    const candidates = hostNodeId && hostNodeId !== targetId ? [targetId, hostNodeId] : [targetId];
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
    <section className="workspace-page" data-testid="exam-workspace">
      <header className="workspace-header">
        <div className="workspace-header-left">
          <button
            className="workspace-back-button"
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
              onClick={() => setIssuesOpen((open) => !open)}
              aria-label={blockers ? `问题 ${issues.length} 项，其中阻断问题 ${blockers} 项` : `问题 ${issues.length} 项`}
            >
              问题 {issues.length}{blockers ? ` · 阻断 ${blockers}` : ""}
            </button>
            <button
              data-testid="workspace-recognition-toggle"
              aria-expanded={recognitionOpen}
              onClick={() => setRecognitionOpen((open) => !open)}
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

      {issuesOpen && mode === "edit" ? (
        <aside className="workspace-issues" aria-label="需要确认的问题" data-testid="workspace-issue-list">
          {issues.length ? (
            <ul>
              {issues.map((issue) => (
                <li key={issue.issueId} className={issue.severity} data-severity={issue.severity}>
                  {/* `data-issue-code` / `data-issue-source` / `data-issue-root-cause` / `data-issue-fact-id`
                      是这一行的机器可读身份：校验脚本要靠它们把「门禁那半」和「本地那半」分开断言，
                      并断言「门禁里每个根因都在界面上出现了」。
                      光看 code 分不出来源、也分不出根因：本地与门禁都可能写 `ANSWER_MISSING`，
                      而门禁把**所有**质量码都写成 `ISSUE_UNRESOLVED`。
                      `data-issue-fact-id` 是后端给出的**稳定事实 id**，同一目标上的两条不同事实
                      靠它区分（上一轮只能靠文案）。 */}
                  <button
                    data-issue-target-id={issue.targetId}
                    data-issue-code={issue.code}
                    data-issue-source={issue.source ?? "local"}
                    data-issue-root-cause={issue.rootCause ?? issue.code}
                    data-issue-fact-id={issue.factId ?? ""}
                    onClick={() => locateTarget(issue.targetId)}
                  >
                    {issue.severity === "blocker" ? <span className="severity-badge" aria-label="阻断问题">⚠</span> : null}
                    {issue.userMessage}
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="empty compact">没有需要确认的问题。</p>
          )}
          {locateMiss ? (
            <p className="empty compact" role="status" data-testid="workspace-locate-miss">
              这条问题不在题面上（目标「{locateMiss}」是整份文档级别），页面上没有可以跳过去的位置。
            </p>
          ) : null}
        </aside>
      ) : null}

      {recognitionOpen && mode === "edit" ? (
        <RecognitionPanel
          itemId={itemId}
          editVersion={editor.version}
          refreshKey={recognitionRefreshKey}
          onLocate={locateTarget}
          onApplied={() => editor.reload()}
          // 「这一项撤销过了吗」由权威稿自己回答（值已等于撤销补丁的目标值），
          // 不用会话内状态 —— 否则刷新/重开就会把「撤销」按钮放回来。
          answerKey={editor.draft?.answerKey as Record<string, unknown> | undefined}
          onUndoAutoFix={async (patch: AuthoringPatchV2) => {
            const draft = editor.draft;
            if (!draft) throw new Error("题稿还没有加载完成，请稍后再试。");
            // 先离线试算：补丁不适用（例如答案位已被删除）时立刻抛错，
            // 而不是让编辑器静默记一条失败状态、面板却显示「已撤销」。
            applyAuthoringV2Patches(draft, [patch]);
            editor.applyPatch(patch);
            // 走版本化事务：值改回旧值、版本递增，并进入编辑器的撤销栈（可 Ctrl+Z 反悔）。
            await editor.flush();
            // `persist()` 在「已有保存在飞」时复用同一个 promise，补丁可能刚好落在
            // 保存循环退出之后；再 flush 一次，确保待发送队列真的清空才敢说「已撤销」。
            await editor.flush();
          }}
        />
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
              已保存版本 v{editor.version} · {previewLimitation.message}
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
                      {preview.summary.answerKeyIssues.slice(0, 8).map((item) => (
                        <li key={`${item.code}:${item.targetId}`} data-preview-runtime-code={item.code} data-preview-runtime-target={item.targetId}>
                          <button
                            type="button"
                            className="workspace-preview-runtime-locate"
                            onClick={() => { setMode("edit"); locateTarget(item.targetId); }}
                          >
                            {item.targetId}
                          </button>
                          <small>（{item.code}）</small>
                        </li>
                      ))}
                    </ul>
                    {preview.summary.answerKeyIssues.length > 8 ? <small>仅显示前 8 处。</small> : null}
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
