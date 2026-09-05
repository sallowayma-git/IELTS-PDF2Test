import { useEffect, useMemo, useState } from "react";
import { getJob, runAutoPipeline, runCloudReview } from "../../api/tauriCommands";
import { chooseExportDirectory } from "../../api/desktopDialogs";
import { describePublishError, publishItem } from "../../api/publishClient";
import { go, legacyPath, libraryPath, type LibraryIntent } from "../../app/router";
import { ExamCanvas } from "../../exam-canvas/ExamCanvas";
import type { JobDetail } from "../../types";
import { readAppSettings, writeAppSettings } from "../settings/appSettings";
import { blockerCount, deriveActionableIssues } from "./actionableIssues";
import { useCanonicalEditor } from "./useCanonicalEditor";

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

export function ExamWorkspacePage({ itemId, intent }: { itemId: string; intent?: LibraryIntent }) {
  const editor = useCanonicalEditor(itemId);
  const [detail, setDetail] = useState<JobDetail | undefined>();
  const [sourceOpen, setSourceOpen] = useState(false);
  const [issuesOpen, setIssuesOpen] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | undefined>();
  const [busyAction, setBusyAction] = useState<string | undefined>();
  const [notice, setNotice] = useState<string | undefined>();
  const [titleEditing, setTitleEditing] = useState(false);
  // 窄窗（<980px）下两栏改为顶部 tab 切换，而不是把 passage 与 questions 堆成一长列。
  const [narrowPane, setNarrowPane] = useState<"passage" | "questions">("questions");

  useEffect(() => {
    getJob(itemId).then(setDetail).catch(() => setDetail(undefined));
  }, [itemId]);

  const issues = useMemo(() => deriveActionableIssues(editor.draft), [editor.draft]);
  const blockers = blockerCount(issues);

  useEffect(() => {
    if (intent === "publish") setNotice("检查下面的问题后，点右上角「发布」把这道题发到 NAS。");
  }, [intent]);

  async function withBusy(key: string, work: () => Promise<void>) {
    setBusyAction(key);
    setMenuOpen(false);
    try {
      await work();
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
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
        <button className="ghost small" onClick={() => go(libraryPath())}>← 返回题库</button>

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

        <div className="workspace-header-actions">
          {editor.saveState !== "idle" ? (
            <span className={`save-state ${editor.saveState}`} data-testid="workspace-save-state">
              {SAVE_LABEL[editor.saveState]}
            </span>
          ) : null}
          <button className="ghost small" onClick={() => setSourceOpen(true)}>查看原文件</button>
          <button
            className={`ghost small ${blockers ? "has-blockers" : ""}`}
            data-testid="workspace-issues"
            onClick={() => setIssuesOpen((open) => !open)}
          >
            问题 {issues.length}
          </button>
          <button className="ghost small" disabled={!editor.canUndo} onClick={editor.undo}>撤销</button>
          <button className="ghost small" disabled={!editor.canRedo} onClick={editor.redo}>重做</button>
          <button className="primary small" data-testid="workspace-publish" disabled={Boolean(busyAction)} onClick={publish}>
            {busyAction === "publish" ? "正在发布…" : "发布"}
          </button>
          <button className="ghost small" aria-label="更多操作" onClick={() => setMenuOpen((open) => !open)}>⋯</button>
        </div>

        {menuOpen ? (
          <div className="workspace-menu" role="menu">
            <button role="menuitem" onClick={() => withBusy("local", async () => {
              await runAutoPipeline(itemId, { executionMode: "localOnly", target: "editableDraft", allowOverwrite: true });
              editor.reload();
              setNotice("已重新运行本地识别。");
            })}>重新运行本地识别</button>
            <button role="menuitem" onClick={() => withBusy("cloud", async () => {
              await runCloudReview(itemId);
              editor.reload();
              setNotice("已重新运行云端识别。");
            })}>重新运行云端识别</button>
            <button role="menuitem" onClick={() => go(legacyPath("preview", itemId))}>
              打开旧版确认与编辑页（兼容）
            </button>
          </div>
        ) : null}
      </header>

      {notice ? (
        <p className="workspace-notice" role="status">
          {notice}
          <button className="ghost small" onClick={() => setNotice(undefined)} aria-label="关闭提示">×</button>
        </p>
      ) : null}
      {editor.saveMessage ? <p className="workspace-notice warning" role="alert">{editor.saveMessage}</p> : null}

      {issuesOpen ? (
        <aside className="workspace-issues" aria-label="需要确认的问题" data-testid="workspace-issue-list">
          {issues.length ? (
            <ul>
              {issues.map((issue) => (
                <li key={issue.issueId} className={issue.severity}>
                  <button onClick={() => {
                    setSelectedId(issue.targetId);
                    document.querySelector(`[data-editor-id="${issue.targetId}"], [data-question-id="${issue.targetId}"], [data-response-group-id="${issue.targetId}"]`)
                      ?.scrollIntoView({ block: "center", behavior: "smooth" });
                  }}>
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
            <p className="error-text">这道题还没有可编辑的题稿：{editor.loadError}</p>
            <div className="button-row">
              <button className="primary small" onClick={() => withBusy("local", async () => {
                await runAutoPipeline(itemId, { executionMode: "localOnly", target: "editableDraft" });
                editor.reload();
              })}>
                运行本地识别
              </button>
              <button className="ghost small" onClick={() => go(legacyPath("preview", itemId))}>打开旧版页面（兼容）</button>
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
          />
        ) : null}
      </div>

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
              <p className="drawer-hint">
                需要逐页比对 bbox 或补录题面时，可以打开
                <button className="ghost small" onClick={() => go(legacyPath("document", itemId))}>旧版源文档确认页</button>
                （兼容期保留）。
              </p>
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
