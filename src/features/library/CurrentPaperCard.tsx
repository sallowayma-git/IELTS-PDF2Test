import type { CurrentPaperV1 } from "../../api/paperClient";

// 页面顶部：NAS 上「当前试卷」。paper === undefined 表示还在读取；null 表示尚未发布。
export function CurrentPaperCard({
  paper,
  destination,
  error
}: {
  paper: CurrentPaperV1 | null | undefined;
  destination?: string;
  /** 读取 NAS 失败的用户文案：与「尚未发布」区分，不能把读取失败说成没有试卷。 */
  error?: string;
}) {
  if (!destination) {
    return (
      <section className="current-paper-card" data-testid="current-paper-card">
        <strong>当前试卷</strong>
        <span className="muted">尚未设置 NAS 目录，设置后可查看当前试卷。</span>
      </section>
    );
  }
  if (error) {
    return (
      <section className="current-paper-card" data-testid="current-paper-card">
        <strong>当前试卷</strong>
        <span className="error-text" role="alert">{error}</span>
      </section>
    );
  }
  if (paper === undefined) {
    return (
      <section className="current-paper-card" data-testid="current-paper-card">
        <strong>当前试卷</strong>
        <span className="muted">读取中…</span>
      </section>
    );
  }
  if (paper === null) {
    return (
      <section className="current-paper-card" data-testid="current-paper-card">
        <strong>当前试卷</strong>
        <span className="muted">尚未发布。在下方选齐阅读 P1 / P2 / P3 后发布为当前试卷。</span>
      </section>
    );
  }
  const published = new Date(paper.publishedAt);
  const publishedText = Number.isNaN(published.getTime()) ? paper.publishedAt : published.toLocaleString("zh-CN");
  return (
    <section className="current-paper-card" data-testid="current-paper-card">
      <div className="current-paper-head">
        <strong>当前试卷</strong>
        <span className="muted" data-testid="current-paper-meta">
          编号 {paper.paperId.slice(0, 8)} · 发布于 {publishedText}
        </span>
      </div>
      <ul className="current-paper-parts">
        {paper.reading.map((entry) => (
          <li key={entry.examId}><b>{entry.part}</b> {entry.title}</li>
        ))}
        {paper.listening.length ? <li><b>听力</b> {paper.listening.length} 篇</li> : null}
      </ul>
    </section>
  );
}
