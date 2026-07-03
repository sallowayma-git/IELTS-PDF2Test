import { useEffect, useMemo, useState } from "react";
import { deleteLibraryExam, getLibraryStats, listLibraryExams, listTrashedExams, restoreLibraryExam, searchLibraryExams } from "../api/tauriCommands";
import { go } from "../app/router";
import { libraryStatusLabel, normalizeLibraryStatus } from "../utils/displayLabels";
import type { LibraryExamSummary, LibraryStats, LibraryStatus, LibrarySubject } from "../types";

const subjectLabel: Record<LibrarySubject, string> = { reading: "阅读", writing: "写作" };
type TabView = "active" | "trash";

function normalizeSummaryStatus(exam: LibraryExamSummary): LibraryExamSummary {
  const status = normalizeLibraryStatus(exam.status) ?? "draft";
  return status === exam.status ? exam : { ...exam, status };
}

export function LibraryPage({ refresh }: { refresh: () => void }) {
  const [items, setItems] = useState<LibraryExamSummary[]>([]);
  const [trashed, setTrashed] = useState<LibraryExamSummary[]>([]);
  const [stats, setStats] = useState<LibraryStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [subject, setSubject] = useState<LibrarySubject | "">("");
  const [status, setStatus] = useState<LibraryStatus | "">("");
  const [search, setSearch] = useState("");
  const [refreshTick, setRefreshTick] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<TabView>("active");

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
        const normalized = list.map(normalizeSummaryStatus);
        // 搜索接口只按 query 过滤，前端二次应用 subject/status 筛选，避免丢弃已选条件。
        let filtered = normalized;
        if (trimmed) {
          if (subject) filtered = filtered.filter((e) => e.subject === subject);
          if (status) filtered = filtered.filter((e) => e.status === status);
        }
        setItems(filtered);
      })
      .catch(console.error)
      .finally(() => setLoading(false));
    getLibraryStats().then(setStats).catch(console.error);
    // 回收站数据（仅在 trash tab 时拉取，避免无谓请求；但拉取成本低，一并刷新）。
    listTrashedExams().then((list) => setTrashed(list.map(normalizeSummaryStatus))).catch(() => setTrashed([]));
  }, [subject, status, search, refreshTick]);

  async function remove(id: string) {
    if (!confirm("确定将该题目移入回收站？（可恢复）")) return;
    setError(null);
    try {
      await deleteLibraryExam(id);
      reload();
      refresh();
    } catch (e) {
      setError(`删除失败：${e}`);
    }
  }

  async function restore(id: string) {
    setError(null);
    try {
      await restoreLibraryExam(id);
      reload();
      refresh();
    } catch (e) {
      setError(`恢复失败：${e}`);
    }
  }

  const statusOrder: (LibraryStatus)[] = ["draft", "needs_review", "ready", "exported"];
  const statusCounts = useMemo(() => {
    const map: Record<string, number> = { draft: 0, needs_review: 0, ready: 0, exported: 0 };
    for (const it of items) map[it.status] = (map[it.status] ?? 0) + 1;
    return map;
  }, [items]);

  const showItems = tab === "active" ? items : trashed;

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

      {tab === "active" && stats ? (
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

      <div className="library-tabs">
        <button className={tab === "active" ? "active" : ""} onClick={() => setTab("active")}>全部题目（{items.length}）</button>
        <button className={tab === "trash" ? "active" : ""} onClick={() => setTab("trash")}>回收站（{trashed.length}）</button>
      </div>

      {tab === "active" ? (
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
      ) : null}

      <div className="job-table tall">
        {showItems.map((exam) => {
          const visualStatus = normalizeLibraryStatus(exam.status) ?? "draft";
          return (
            <div className="job-row static" key={exam.id}>
            <button className="link-cell" onClick={() => tab === "active" ? go(`/library/${exam.id}`) : undefined}>
              <strong>{exam.title}</strong>
              <small>{exam.examId ?? exam.id}</small>
            </button>
            <span className={`status-pill status-${visualStatus}`}>{libraryStatusLabel(visualStatus)}</span>
            <span>{subjectLabel[exam.subject]}</span>
            <span>{exam.category ?? "—"}</span>
            <span>{exam.updatedAt.slice(0, 10)}</span>
            {tab === "active" ? (
              <>
                <button className="ghost small" onClick={() => go(`/library/${exam.id}`)}>查看</button>
                <button className="danger small" onClick={() => remove(exam.id)}>删除</button>
              </>
            ) : (
              <>
                <button className="ghost small" onClick={() => restore(exam.id)}>恢复</button>
                <span className="empty compact">已移入回收站</span>
              </>
            )}
            </div>
          );
        })}
        {loading ? <p className="empty">加载中…</p> : null}
        {!loading && !showItems.length && tab === "active" ? <p className="empty">题库为空。完成导题或写作创作后，题目会自动进入题库。</p> : null}
        {!loading && !showItems.length && tab === "trash" ? <p className="empty">回收站为空。</p> : null}
      </div>
    </section>
  );
}
