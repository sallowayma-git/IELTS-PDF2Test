import { useEffect, useMemo, useState } from "react";
import { deleteLibraryExam, getLibraryStats, listLibraryExams, searchLibraryExams } from "../api/tauriCommands";
import { go } from "../app/router";
import { libraryStatusLabel } from "../utils/displayLabels";
import type { LibraryExamSummary, LibraryStats, LibraryStatus, LibrarySubject } from "../types";

const subjectLabel: Record<LibrarySubject, string> = { reading: "阅读", writing: "写作" };

export function LibraryPage({ refresh }: { refresh: () => void }) {
  const [items, setItems] = useState<LibraryExamSummary[]>([]);
  const [stats, setStats] = useState<LibraryStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [subject, setSubject] = useState<LibrarySubject | "">("");
  const [status, setStatus] = useState<LibraryStatus | "">("");
  const [search, setSearch] = useState("");
  const [refreshTick, setRefreshTick] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const reload = () => setRefreshTick((v) => v + 1);

  useEffect(() => {
    setLoading(true);
    const filter = {
      subject: subject || undefined,
      status: status || undefined
    };
    const trimmed = search.trim();
    const load = trimmed
      ? searchLibraryExams(trimmed)
      : listLibraryExams(filter);
    load
      .then((list) => {
        // 搜索接口只按 query 过滤，前端二次应用 subject/status 筛选，避免丢弃已选条件。
        let filtered = list;
        if (trimmed) {
          if (subject) filtered = filtered.filter((e) => e.subject === subject);
          if (status) filtered = filtered.filter((e) => e.status === status);
        }
        setItems(filtered);
      })
      .catch(console.error)
      .finally(() => setLoading(false));
    getLibraryStats().then(setStats).catch(console.error);
  }, [subject, status, search, refreshTick]);

  async function remove(id: string) {
    if (!confirm("确定从题库中删除该题目？")) return;
    setError(null);
    try {
      await deleteLibraryExam(id);
      reload();
      refresh();
    } catch (e) {
      setError(`删除失败：${e}`);
    }
  }

  const statusOrder: (LibraryStatus)[] = ["draft", "needs_review", "ready", "exported"];
  const statusCounts = useMemo(() => {
    const map: Record<string, number> = { draft: 0, needs_review: 0, ready: 0, exported: 0 };
    for (const it of items) map[it.status] = (map[it.status] ?? 0) + 1;
    return map;
  }, [items]);

  return (
    <section className="page-enter">
      <div className="section-heading spread">
        <div>
          <p className="eyebrow">Library</p>
          <h2>题库管理</h2>
        </div>
        <button className="primary" onClick={() => go("/jobs/new")}>新建导题任务</button>
      </div>

      {error ? <p className="empty">{error}</p> : null}

      {stats ? (
        <div className="metric-row">
          {statusOrder.map((s) => (
            <div className="metric" key={s}>
              <span>{libraryStatusLabel(s)}</span>
              <strong>{stats.byStatus[s] ?? 0}</strong>
            </div>
          ))}
          <div className="metric">
            <span>合计</span>
            <strong>{stats.total}</strong>
          </div>
        </div>
      ) : null}

      <div className="library-filters">
        <select value={subject} onChange={(e) => setSubject(e.target.value as LibrarySubject | "")}>
          <option value="">全部学科</option>
          <option value="reading">阅读</option>
          <option value="writing">写作</option>
        </select>
        <select value={status} onChange={(e) => setStatus(e.target.value as LibraryStatus | "")}>
          <option value="">全部状态</option>
          {statusOrder.map((s) => (
            <option key={s} value={s}>{libraryStatusLabel(s)}（{statusCounts[s] ?? 0}）</option>
          ))}
        </select>
        <input
          className="library-search"
          placeholder="按标题 / examId / 标签搜索…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      <div className="job-table tall">
        {items.map((exam) => (
          <div className="job-row static" key={exam.id}>
            <button className="link-cell" onClick={() => go(`/library/${exam.id}`)}>
              <strong>{exam.title}</strong>
              <small>{exam.examId ?? exam.id}</small>
            </button>
            <span className={`status-pill status-${exam.status}`}>{libraryStatusLabel(exam.status)}</span>
            <span>{subjectLabel[exam.subject]}</span>
            <span>{exam.category ?? "—"}</span>
            <span>{exam.updatedAt.slice(0, 10)}</span>
            <button className="ghost small" onClick={() => go(`/library/${exam.id}`)}>查看</button>
            <button className="danger small" onClick={() => remove(exam.id)}>删除</button>
          </div>
        ))}
        {loading ? <p className="empty">加载中…</p> : null}
        {!loading && !items.length ? <p className="empty">题库为空。完成导题或写作创作后，题目会自动进入题库。</p> : null}
      </div>
    </section>
  );
}
