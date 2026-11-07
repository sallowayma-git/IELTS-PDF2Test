import { analyticsApi, type QuestionView } from "../../api/analyticsClient";
import { taskTypeLabel } from "../../utils/displayLabels";
import { BarRows } from "./Charts";
import { msText, nText, num, pct, questionVerdict, useLoad } from "./dataFormat";

export function QuestionPanel({
  batchId,
  paperId,
  passageIndex,
  questionId
}: {
  batchId: string;
  paperId: string;
  passageIndex: number;
  questionId: string;
}) {
  const question = useLoad<QuestionView>(
    () => analyticsApi.question(batchId, paperId, passageIndex, questionId),
    [batchId, paperId, passageIndex, questionId],
    "读取题目数据失败，请稍后重试。"
  );
  const view = question.data;
  if (question.error) return <p className="error-text" role="alert">{question.error}</p>;
  if (!view) return <p className="data-empty">正在读取…</p>;

  const items = view.distribution.map((bucket, index) => ({
    key: `bucket-${index}`,
    label: bucket.answer,
    value: bucket.share,
    detail: `${nText(bucket.count)}${bucket.isCorrect == null ? "" : bucket.isCorrect ? " · 正确答案" : " · 错误答案"}`
  }));

  return (
    <section className="data-question" data-testid="data-question">
      <header className="data-toolbar">
        <div>
          <h1>{view.displayLabel} <small>{view.passageLabel}</small></h1>
          <p className="data-subtitle">{taskTypeLabel(view.questionKind)} · 标准答案 {view.correctAnswer}</p>
        </div>
      </header>

      <div className="data-stat-grid">
        <div className="data-stat">
          <span className="data-stat-label">正确率</span>
          <strong className="data-stat-value">{pct(view.accuracy)}</strong>
          <span className="data-stat-hint">{nText(view.n)}</span>
        </div>
        <div className="data-stat">
          <span className="data-stat-label">答对人数</span>
          <strong className="data-stat-value">{view.correctCount}</strong>
          <span className="data-stat-hint">{nText(view.n)}</span>
        </div>
        <div className="data-stat">
          <span className="data-stat-label">平均停留</span>
          <strong className="data-stat-value">{msText(view.avgElapsedMs)}</strong>
          <span className="data-stat-hint">{nText(view.n)}</span>
        </div>
        <div className="data-stat">
          <span className="data-stat-label">平均改答</span>
          <strong className="data-stat-value">{num(view.avgChangeCount)}</strong>
          <span className="data-stat-hint">{nText(view.n)}</span>
        </div>
      </div>
      <p className="data-notice">{questionVerdict(view)}</p>

      <section className="data-section">
        <h2>答案分布 <small>{nText(view.n)}</small></h2>
        <BarRows items={items} ariaLabel="答案分布" />
      </section>

      <footer className="data-footer">{view.disclaimer}</footer>
    </section>
  );
}
