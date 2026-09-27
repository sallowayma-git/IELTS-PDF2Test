import { useEffect, useMemo, useState } from "react";
import {
  chooseExportDirectory
} from "../../api/desktopDialogs";
import {
  createWritingJob,
  deleteWritingJob,
  exportWritingLibrary,
  listWritingJobs,
  updateWritingJob
} from "../../api/tauriCommands";
import { StatusPill } from "../../components/StatusPill";
import { toUserFacingError } from "../../utils/userFacingError";
import type { WritingJob, WritingJobStatus, WritingTaskType } from "../../types";

// 题库「写作」子标签的创作面板：新建/编辑/删除写作题，并就地导出 NAS 写作题库
// （选一道 Task 1 + 一道 Task 2 → 选目录 → export_writing_library）。

const TASK_DEFAULTS: Record<WritingTaskType, { suggested: number; label: string }> = {
  task1: { suggested: 150, label: "图表描述题 (Task 1)" },
  task2: { suggested: 250, label: "议论文 (Task 2)" }
};

export function WritingPanel({ refresh }: { refresh: () => void }) {
  const [jobs, setJobs] = useState<WritingJob[]>([]);
  const [selectedJobId, setSelectedJobId] = useState<string | undefined>();
  const [editing, setEditing] = useState<WritingJob | undefined>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | undefined>();
  const [notice, setNotice] = useState<string | undefined>();
  const [creatingTaskType, setCreatingTaskType] = useState<WritingTaskType>("task1");
  const [newTitle, setNewTitle] = useState("");

  const ready = useMemo(() => jobs.filter((j) => j.status === "ExportReady" || j.status === "Exported"), [jobs]);

  function showError(error: unknown, fallback?: string) {
    const facing = toUserFacingError(error, fallback);
    console.error("[writing-panel]", facing.internalDetail);
    setError(facing.userMessage);
  }

  async function reload() {
    try {
      const list = await listWritingJobs();
      setJobs(list);
      if (!selectedJobId && list.length) setSelectedJobId(list[0].jobId);
      const current = list.find((j) => j.jobId === selectedJobId);
      if (current) setEditing({ ...current });
    } catch (e) {
      showError(e, "写作任务列表加载失败，请稍后重试。");
    }
  }

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const current = jobs.find((j) => j.jobId === selectedJobId);
    setEditing(current ? { ...current } : undefined);
  }, [selectedJobId, jobs]);

  async function handleCreate() {
    setBusy(true);
    setError(undefined);
    try {
      const job = await createWritingJob({
        title: newTitle.trim() || `写作题 ${creatingTaskType}`,
        taskType: creatingTaskType,
        suggestedWordCount: TASK_DEFAULTS[creatingTaskType].suggested
      });
      setNewTitle("");
      await reload();
      setSelectedJobId(job.jobId);
      refresh();
    } catch (e) {
      showError(e, "创建写作任务失败，请稍后重试。");
    } finally {
      setBusy(false);
    }
  }

  async function handleSave() {
    if (!editing) return;
    setBusy(true);
    setError(undefined);
    try {
      await updateWritingJob(editing.jobId, {
        title: editing.title,
        taskType: editing.taskType,
        examId: editing.examId,
        promptText: editing.promptText,
        suggestedWordCount: editing.suggestedWordCount,
        status: editing.status
      });
      await reload();
      refresh();
    } catch (e) {
      showError(e, "保存写作任务失败，请稍后重试。");
    } finally {
      setBusy(false);
    }
  }

  async function handleMarkReady() {
    if (!editing) return;
    if (!editing.promptText.trim()) {
      setError("题目内容 (promptText) 不能为空。");
      return;
    }
    setBusy(true);
    setError(undefined);
    try {
      const updated = await updateWritingJob(editing.jobId, { status: "ExportReady" as WritingJobStatus });
      setEditing({ ...updated });
      await reload();
      refresh();
    } catch (e) {
      showError(e, "标记可导出失败，请稍后重试。");
    } finally {
      setBusy(false);
    }
  }

  async function handleDelete() {
    if (!editing) return;
    if (!window.confirm(`确认删除写作任务「${editing.title}」？此操作不可恢复。`)) return;
    setBusy(true);
    setError(undefined);
    try {
      await deleteWritingJob(editing.jobId);
      setSelectedJobId(undefined);
      setEditing(undefined);
      await reload();
      refresh();
    } catch (e) {
      showError(e, "删除写作任务失败，请稍后重试。");
    } finally {
      setBusy(false);
    }
  }

  // 就地导出：需各一道「可导出」的 Task 1 与 Task 2；选目录后调 export_writing_library。
  async function handleExport() {
    const task1 = ready.find((j) => j.taskType === "task1");
    const task2 = ready.find((j) => j.taskType === "task2");
    if (!task1 || !task2) {
      setError("导出写作题库需要各一道「可导出」的 Task 1 与 Task 2（先在编辑区「标记可导出」）。");
      return;
    }
    const dir = await chooseExportDirectory();
    if (!dir) return;
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      await exportWritingLibrary({ jobIds: [task1.jobId, task2.jobId], exportDir: dir });
      await reload();
      refresh();
      setNotice(`已导出写作题库到 ${dir}`);
    } catch (e) {
      showError(e, "导出写作题库失败，请稍后重试。");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="writing-panel page-enter" data-testid="library-writing-panel">
      <div className="hero-panel">
        <div>
          <p className="eyebrow">写作题库创作</p>
          <h2>手输 Task 1 / Task 2 题目，导出为 NAS 端可识别的写作题库</h2>
          <p>各建一道「可导出」的 Task 1 与 Task 2，然后就地导出到 NAS 目录。</p>
        </div>
        <div className="hero-actions">
          <button className="primary" disabled={busy} onClick={handleExport} data-testid="writing-export">
            {busy ? "处理中…" : "导出写作题库"}
          </button>
        </div>
      </div>

      {notice ? <p className="library-notice" role="status" data-testid="writing-notice">{notice}<button className="ghost small" onClick={() => setNotice(undefined)} aria-label="关闭提示">×</button></p> : null}

      <div className="two-column">
        <section>
          <div className="section-heading">
            <p className="eyebrow">新建写作任务</p>
            <h3>创建</h3>
          </div>
          <div className="writing-create-form">
            <label>
              <span>题目类型</span>
              <select value={creatingTaskType} onChange={(e) => setCreatingTaskType(e.target.value as WritingTaskType)}>
                <option value="task1">Task 1（图表描述，建议 150 词）</option>
                <option value="task2">Task 2（议论文，建议 250 词）</option>
              </select>
            </label>
            <label>
              <span>标题（可选）</span>
              <input value={newTitle} onChange={(e) => setNewTitle(e.target.value)} placeholder={`写作题 ${creatingTaskType}`} />
            </label>
            <button className="primary" disabled={busy} onClick={handleCreate}>{busy ? "创建中…" : "新建写作任务"}</button>
          </div>

          <div className="section-heading" style={{ marginTop: 24 }}>
            <p className="eyebrow">写作任务列表</p>
            <h3>已有任务</h3>
          </div>
          <div className="job-table" data-testid="writing-job-table">
            {jobs.map((job) => (
              <button
                key={job.jobId}
                className={`job-row${job.jobId === selectedJobId ? " is-active" : ""}`}
                onClick={() => setSelectedJobId(job.jobId)}
              >
                <span>
                  <strong>{job.title}</strong>
                  <small>{job.taskType} · {job.examId}</small>
                </span>
                <StatusPill status={mapWritingStatusToJobStatus(job.status)} />
                <span>{job.updatedAt.slice(0, 10)}</span>
              </button>
            ))}
            {!jobs.length ? <p className="empty">暂无写作任务。在上方创建一个。</p> : null}
          </div>
        </section>

        <aside className="inspector writing-editor-pane">
          {editing ? (
            <>
              <div className="section-heading">
                <p className="eyebrow">编辑题目</p>
                <h3>{editing.title}</h3>
              </div>
              <div className="writing-edit-form">
                <label>
                  <span>标题</span>
                  <input value={editing.title} onChange={(e) => setEditing({ ...editing, title: e.target.value })} />
                </label>
                <label>
                  <span>题目类型</span>
                  <select
                    value={editing.taskType}
                    onChange={(e) => setEditing({ ...editing, taskType: e.target.value as WritingTaskType })}
                  >
                    <option value="task1">Task 1</option>
                    <option value="task2">Task 2</option>
                  </select>
                </label>
                <label>
                  <span>建议字数</span>
                  <input
                    type="number"
                    min={1}
                    value={editing.suggestedWordCount}
                    onChange={(e) => setEditing({ ...editing, suggestedWordCount: Math.max(1, Number(e.target.value) || 0) })}
                  />
                </label>
                <label>
                  <span>题目 ID（examId）</span>
                  <input value={editing.examId} onChange={(e) => setEditing({ ...editing, examId: e.target.value })} />
                </label>
                <label>
                  <span>题目内容 (promptText)</span>
                  <textarea
                    className="writing-prompt-textarea"
                    value={editing.promptText}
                    onChange={(e) => setEditing({ ...editing, promptText: e.target.value })}
                    placeholder="输入写作题目全文（含图表说明、写作要求等）…"
                    rows={14}
                  />
                </label>
                <div className="writing-edit-actions">
                  <button className="primary" disabled={busy} onClick={handleSave}>保存</button>
                  <button disabled={busy || editing.status !== "Draft"} onClick={handleMarkReady}>标记可导出</button>
                  <button className="ghost" disabled={busy} onClick={handleDelete}>删除</button>
                </div>
                <p className="writing-status-hint">
                  当前状态：<StatusPill status={mapWritingStatusToJobStatus(editing.status)} />
                  {editing.status === "Exported" ? "（已导出，仍可编辑后重新导出）" : ""}
                </p>
              </div>
            </>
          ) : (
            <p className="empty">从左侧选择一个写作任务进行编辑，或新建一个。</p>
          )}
          {error ? <p className="error-text" style={{ marginTop: 12 }}>{error}</p> : null}
        </aside>
      </div>
    </section>
  );
}

function mapWritingStatusToJobStatus(status: WritingJobStatus): "Working" | "DraftSaved" | "ExportReady" | "Exported" {
  if (status === "Exported") return "Exported";
  if (status === "ExportReady") return "ExportReady";
  return "DraftSaved";
}
