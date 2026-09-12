import { useEffect, useMemo, useState } from "react";
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
import { readAppSettings, writeAppSettings } from "../settings/appSettings";
import { blockerCount, deriveActionableIssues } from "./actionableIssues";
import { useCanonicalEditor } from "./useCanonicalEditor";
import { toUserFacingError } from "../../utils/userFacingError";

// 题目工作区（计划 §16.6 / §9.10）。
// 打开就是最终 IELTS 题面，没有 编辑/预览 开关；左侧 passage、右侧 questions 由 ExamCanvas 渲染。
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
  const [menuOpen, setMenuOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | undefined>();
  const [busyAction, setBusyAction] = useState<string | undefined>();
  const [notice, setNotice] = useState<string | undefined>();
  const [noticeDetail, setNoticeDetail] = useState<string | undefined>();
  const [titleEditing, setTitleEditing] = useState(false);
  // 窄窗（<980px）下两栏改为顶部 tab 切换，而不是把 passage 与 questions 堆成一长列。
  const [narrowPane, setNarrowPane] = useState<"passage" | "questions">("questions");

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

  const issues = useMemo(() => deriveActionableIssues(editor.draft), [editor.draft]);
  const blockers = blockerCount(issues);

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
            <button onClick={() => setSourceOpen(true)}><FileSearch size={16} /></button>
            <button
              className={blockers ? "has-blockers" : ""}
              data-testid="workspace-issues"
              onClick={() => setIssuesOpen((open) => !open)}
              aria-label={blockers ? `问题 ${issues.length} 项，其中阻断问题 ${blockers} 项` : `问题 ${issues.length} 项`}
            >
              问题 {issues.length}{blockers ? ` · 阻断 ${blockers}` : ""}
            </button>
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
        <span className="workspace-sub-header-meta">编辑模式 · 保存后点击"发布"输出到 NAS</span>
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

      {issuesOpen ? (
        <aside className="workspace-issues" aria-label="需要确认的问题" data-testid="workspace-issue-list">
          {issues.length ? (
            <ul>
              {issues.map((issue) => (
                <li key={issue.issueId} className={issue.severity} data-severity={issue.severity}>
                  <button onClick={() => {
                    setSelectedId(issue.targetId);
                    document.querySelector(`[data-editor-id="${issue.targetId}"], [data-question-id="${issue.targetId}"], [data-response-group-id="${issue.targetId}"]`)
                      ?.scrollIntoView({ block: "center", behavior: "smooth" });
                  }}>
                    {issue.severity === "blocker" ? <span className="severity-badge" aria-label="阻断问题">⚠</span> : null}
                    {issue.userMessage}
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="empty compact">没有需要确认的问题。</p>
          )}
        </aside>
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
        {editor.draft ? (
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
      </div>

      {editor.draft ? <SelectionInspector draft={editor.draft} selectedId={selectedId} onPatch={editor.applyPatch} onClose={() => setSelectedId(undefined)} /> : null}

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
