import { useEffect, useState } from "react";
import { getLibraryExam, updateLibraryExamMeta } from "../api/tauriCommands";
import { go } from "../app/router";
import { libraryStatusLabel } from "../utils/displayLabels";
import { sanitizeHtml } from "../utils/sanitizeHtml";
import type { LibraryExamDetail, LibraryStatus, LibrarySubject } from "../types";
import type { ReadingAuthoringIr } from "../types";
import type { WritingJob } from "../types";

const subjectLabel: Record<LibrarySubject, string> = { reading: "阅读", writing: "写作" };
const statusOptions: LibraryStatus[] = ["draft", "needs_review", "ready", "exported"];

/** 运行时类型守卫：判断 payload 是否为真正的 ReadingAuthoringIr（而非 ImportJob fallback）。 */
function isReadingAuthoringIr(p: unknown): p is ReadingAuthoringIr {
  return (
    !!p &&
    typeof p === "object" &&
    (p as { schemaVersion?: unknown }).schemaVersion === "ReadingAuthoringIRV1"
  );
}

/** 运行时类型守卫：判断 payload 是否为 WritingJob。 */
function isWritingJob(p: unknown): p is WritingJob {
  return !!p && typeof p === "object" && typeof (p as { promptText?: unknown }).promptText === "string";
}

export function LibraryExamDetail({ examId, refresh }: { examId: string; refresh: () => void }) {
  const [detail, setDetail] = useState<LibraryExamDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState("");
  const [status, setStatus] = useState<LibraryStatus>("draft");
  const [tags, setTags] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setLoading(true);
    setError(null);
    getLibraryExam(examId)
      .then((d) => {
        setDetail(d);
        if (d) {
          setTitle(d.summary.title);
          setStatus(d.summary.status);
          setTags(d.summary.tags.join(", "));
        }
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [examId]);

  async function save() {
    if (!detail) return;
    setError(null);
    const patch = {
      title,
      status,
      tags: tags.split(",").map((t) => t.trim()).filter(Boolean)
    };
    try {
      const updated = await updateLibraryExamMeta(examId, patch);
      if (updated) {
        setDetail({ summary: updated, payload: detail.payload });
        setEditing(false);
        refresh();
      } else {
        setError("未找到该题目，可能已被删除。");
      }
    } catch (e) {
      setError(`保存失败：${e}`);
    }
  }

  if (loading) return <section className="page-enter"><p className="empty">加载中…</p></section>;
  if (!detail) return (
    <section className="page-enter">
      <p className="empty">未找到该题目。<button className="link" onClick={() => go("/library")}>返回题库</button></p>
    </section>
  );

  const summary = detail.summary;
  const payload = detail.payload;
  const isReading = summary.subject === "reading";
  const readingIr = isReadingAuthoringIr(payload) ? payload : null;
  const writingJob = isWritingJob(payload) ? payload : null;
  const payloadFallback = isReading && !readingIr; // payload 是 ImportJob 而非 authoring-ir

  return (
    <section className="page-enter">
      {error ? <p className="empty">{error}</p> : null}
      <div className="section-heading spread">
        <div>
          <p className="eyebrow">{subjectLabel[summary.subject]} · 题库</p>
          <h2>{summary.title}</h2>
          <small>{summary.examId ?? summary.id}</small>
        </div>
        <div className="hero-actions">
          <button className="ghost" onClick={() => go("/library")}>返回题库</button>
          {isReading ? (
            <button className="primary" onClick={() => go(`/jobs/${summary.id}/export`)}>从该题导出</button>
          ) : (
            <button className="ghost" disabled title="写作导出需在导出页同时选择 task1 + task2">从该题导出</button>
          )}
        </div>
      </div>

      <div className="two-column">
        <section>
          {editing ? (
            <div className="inspector">
              <p className="eyebrow">编辑元数据</p>
              <label className="field">
                <span>标题</span>
                <input value={title} onChange={(e) => setTitle(e.target.value)} />
              </label>
              <label className="field">
                <span>状态</span>
                <select value={status} onChange={(e) => setStatus(e.target.value as LibraryStatus)}>
                  {statusOptions.map((s) => <option key={s} value={s}>{libraryStatusLabel(s)}</option>)}
                </select>
              </label>
              <label className="field">
                <span>标签（逗号分隔）</span>
                <input value={tags} onChange={(e) => setTags(e.target.value)} />
              </label>
              <div className="hero-actions">
                <button className="primary" onClick={save}>保存</button>
                <button className="ghost" onClick={() => setEditing(false)}>取消</button>
              </div>
            </div>
          ) : (
            <div className="inspector">
              <p className="eyebrow">元数据</p>
              <dl className="meta-list">
                <div><dt>学科</dt><dd>{subjectLabel[summary.subject]}</dd></div>
                <div><dt>分类</dt><dd>{summary.category ?? "—"}</dd></div>
                <div><dt>频次</dt><dd>{summary.frequency ?? "—"}</dd></div>
                <div><dt>状态</dt><dd><span className={`status-pill status-${summary.status}`}>{libraryStatusLabel(summary.status)}</span></dd></div>
                {summary.taskType ? <div><dt>任务类型</dt><dd>{summary.taskType}</dd></div> : null}
                <div><dt>标签</dt><dd>{summary.tags.length ? summary.tags.join("、") : "—"}</dd></div>
                <div><dt>错误/警告</dt><dd>{summary.issueErrors} / {summary.issueWarnings}</dd></div>
                <div><dt>更新时间</dt><dd>{summary.updatedAt.slice(0, 19).replace("T", " ")}</dd></div>
              </dl>
              <button className="ghost" onClick={() => setEditing(true)}>编辑元数据</button>
            </div>
          )}
        </section>

        <section>
          <div className="section-heading">
            <p className="eyebrow">题目内容</p>
            <h3>{isReading ? "阅读题稿" : "写作题目"}</h3>
          </div>
          {payloadFallback ? (
            <div className="inspector">
              <p className="empty">该题目尚未生成可编辑题稿（缺少 authoring-ir）。</p>
              <p className="eyebrow">下一步</p>
              <p>请在转化工具中打开该任务，完成「确认与编辑」步骤生成题稿后，题库详情将展示完整内容。</p>
              <button className="primary" onClick={() => go(`/jobs/${summary.id}/document`)}>打开转化任务</button>
            </div>
          ) : null}
          {readingIr ? (
            <div className="library-payload">
              <p className="eyebrow">原文段落（{readingIr.passage.htmlBlocks.length} 块）</p>
              <div className="read-only-html">
                {readingIr.passage.htmlBlocks.map((b) => <div key={b.blockId} dangerouslySetInnerHTML={{ __html: sanitizeHtml(b.html) }} />)}
              </div>
              <p className="eyebrow">题组（{readingIr.groups.length} 组）</p>
              <div className="job-table">
                {readingIr.groups.map((g) => (
                  <div className="job-row static" key={g.groupId}>
                    <strong>{g.kind}</strong>
                    <span>{g.questions.length} 题</span>
                    <span>{g.questionRange ? `${g.questionRange[0]}–${g.questionRange[1]}` : "—"}</span>
                    <span className={`status-pill status-${g.verified ? "exported" : "needs_review"}`}>{g.verified ? "已确认" : "待确认"}</span>
                  </div>
                ))}
              </div>
              <p className="eyebrow">答案键（{Object.keys(readingIr.answerKey).length} 项）</p>
              <div className="answer-key-grid">
                {Object.entries(readingIr.answerKey).map(([q, a]) => (
                  <div key={q}><strong>{q}</strong>: {Array.isArray(a) ? a.join("、") : a}</div>
                ))}
              </div>
            </div>
          ) : null}
          {writingJob ? (
            <div className="library-payload">
              <p className="eyebrow">题目要求（建议 {writingJob.suggestedWordCount} 词）</p>
              <div className="read-only-html"><p>{writingJob.promptText || "（空）"}</p></div>
            </div>
          ) : null}
        </section>
      </div>
    </section>
  );
}
