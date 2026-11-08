import { useState } from "react";
import { analyticsApi, type ExamView } from "../../api/analyticsClient";
import { go } from "../../app/router";
import { taskTypeLabel } from "../../utils/displayLabels";
import { BarRows, Histogram } from "./Charts";
import { dataViewPath } from "./dataRoute";
import { errorText, HARD_ACCURACY_THRESHOLD, nText, num, pct, paperText, questionVerdict, secText, timeText, useLoad } from "./dataFormat";

export function ExamPanel({ batchId, paperId }: { batchId: string; paperId: string }) {
  const exam = useLoad<ExamView>(() => analyticsApi.exam(batchId, paperId), [batchId, paperId], "读取考试数据失败，请稍后重试。");
  const [notice, setNotice] = useState<string | undefined>(undefined);
  const [exportError, setExportError] = useState<string | undefined>(undefined);
  const [exporting, setExporting] = useState(false);

  async function exportExcel() {
    setExporting(true);
    setExportError(undefined);
    setNotice(undefined);
    try {
      const result = await analyticsApi.exportXlsx(batchId, paperId);
      if (result) setNotice(`已导出 ${result.path}（${result.sheets.join(" / ")}）`);
    } catch (caught) {
      setExportError(errorText(caught, "导出失败，请稍后重试。"));
    } finally {
      setExporting(false);
    }
  }

  const view = exam.data;
  if (exam.error) return <p className="error-text" role="alert">{exam.error}</p>;
  if (!view) return <p className="data-empty">正在读取…</p>;

  const partItems = view.parts.map((part) => ({
    key: `part-${part.passageIndex}`,
    label: part.label,
    value: part.accuracy,
    detail: `${nText(part.n)} · ${num(part.correct, 1)}/${num(part.total, 1)} 点`
  }));
  const kindItems = view.kinds.map((kind) => ({
    key: `kind-${kind.kind}`,
    label: taskTypeLabel(kind.kind),
    value: kind.accuracy,
    detail: `${nText(kind.n)} · ${num(kind.correctPoints, 1)}/${num(kind.points, 1)} 点`
  }));
  const scoreMax = view.score.maxTotal;

  return (
    <section className="data-exam" data-testid="data-exam">
      <header className="data-toolbar">
        <div>
          <h1>{batchId} · 卷 {paperText(paperId)}</h1>
          <p className="data-subtitle">{nText(view.n)} 人有效计分（原始分 / 正确率）。</p>
        </div>
        <div className="data-toolbar-actions">
          <button className="ghost" type="button" data-testid="data-export" disabled={exporting} onClick={() => void exportExcel()}>
            {exporting ? "正在导出…" : "导出 Excel"}
          </button>
        </div>
      </header>
      {notice ? <p className="data-notice" role="status" data-testid="data-export-notice">{notice}</p> : null}
      {exportError ? <p className="error-text" role="alert">{exportError}</p> : null}

      <div className="data-stat-grid">
        <Stat label="有效计分" value={nText(view.n)} hint={`${view.excluded.length} 人不计入`} />
        <Stat label="均分" value={num(view.score.mean)} hint={`${nText(view.n)}${scoreMax != null ? ` · 满分 ${num(scoreMax)}` : ""}`} />
        <Stat label="中位数" value={num(view.score.median)} hint={nText(view.n)} />
        <Stat label="最高 / 最低" value={`${num(view.score.max)} / ${num(view.score.min)}`} hint={nText(view.n)} />
        <Stat label="平均正确率" value={pct(view.score.meanAccuracy)} hint={nText(view.n)} />
      </div>

      {view.excluded.length > 0 ? (
        <section className="data-section" data-testid="data-excluded">
          <h2>不计入（{view.excluded.length}）</h2>
          <ul className="data-list">
            {view.excluded.map((item) => (
              <li key={item.attemptId}>{item.candidateName}：{item.reason}</li>
            ))}
          </ul>
        </section>
      ) : null}

      <section className="data-section">
        <h2>原始分分布 <small>{nText(view.n)}</small></h2>
        <Histogram bins={view.histogram} label={`原始分分布，${nText(view.n)}`} />
      </section>

      <div className="data-two-col">
        <section className="data-section">
          <h2>Part 正确率 <small>按得分点</small></h2>
          <BarRows items={partItems} ariaLabel="Part 正确率" />
        </section>
        <section className="data-section">
          <h2>题型正确率 <small>分母为得分点</small></h2>
          <BarRows items={kindItems} ariaLabel="题型正确率" />
        </section>
      </div>

      <section className="data-section" data-testid="data-hard">
        <h2>难题 Top 10 <small>难题结论需 N ≥ {view.hardConclusionMinN}，正确率 &lt; {Math.round(HARD_ACCURACY_THRESHOLD * 100)}%</small></h2>
        {view.hardQuestions.length === 0 ? (
          <p className="data-empty">暂无判分题目。</p>
        ) : (
          <table className="data-table">
            <thead>
              <tr><th>题号</th><th>Part</th><th>题型</th><th>正确率</th><th>样本</th><th>结论</th><th /></tr>
            </thead>
            <tbody>
              {view.hardQuestions.map((question) => (
                <tr key={`${question.passageIndex}|${question.questionId}`} data-testid="data-hard-row">
                  <td>{question.displayLabel}</td>
                  <td>{question.passageLabel}</td>
                  <td>{taskTypeLabel(question.questionKind)}</td>
                  <td>{pct(question.accuracy)}</td>
                  <td>{nText(question.n)}</td>
                  <td>{questionVerdict(question)}</td>
                  <td>
                    <button
                      className="ghost small"
                      type="button"
                      onClick={() =>
                        go(dataViewPath({
                          kind: "question",
                          batchId,
                          paperId,
                          passageIndex: question.passageIndex,
                          questionId: question.questionId
                        }))
                      }
                    >
                      看分布
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <section className="data-section" data-testid="data-matrix">
        <h2>学生 × 题目 <small>{nText(view.n)} · 对 / 错 / — 为无判分</small></h2>
        {view.matrix.length === 0 || view.questions.length === 0 ? (
          <p className="data-empty">暂无判分记录。</p>
        ) : (
          <div className="data-matrix-wrap">
            <table className="data-matrix">
              <thead>
                <tr>
                  <th scope="col">学生</th>
                  {view.questions.map((question) => (
                    <th scope="col" key={`${question.passageIndex}|${question.questionId}`} title={`${question.passageLabel} · ${question.displayLabel}`}>
                      {question.displayLabel}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {view.matrix.map((row) => (
                  <tr key={row.attemptId}>
                    <th scope="row">
                      <button className="link-button" type="button" onClick={() => go(dataViewPath({ kind: "student", attemptId: row.attemptId }))}>
                        {row.candidateName}
                      </button>
                    </th>
                    {row.cells.map((cell, index) => {
                      const question = view.questions[index];
                      const label = cell === true ? "对" : cell === false ? "错" : "—";
                      const cls = cell === true ? "data-cell-ok" : cell === false ? "data-cell-bad" : "data-cell-none";
                      return (
                        <td key={`${row.attemptId}|${index}`} className={`data-cell ${cls}`} title={`${row.candidateName} · ${question?.displayLabel ?? ""}：${label}`}>
                          {label}
                        </td>
                      );
                    })}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section className="data-section" data-testid="data-students">
        <h2>学生成绩 <small>{nText(view.n)}</small></h2>
        {view.students.length === 0 ? (
          <p className="data-empty">还没有有效计分的学生。</p>
        ) : (
          <table className="data-table">
            <thead>
              <tr>
                <th>姓名</th><th>卡号</th><th>原始分</th><th>正确率</th>
                {view.parts.map((part) => <th key={part.passageIndex}>{part.label}</th>)}
                <th>用时</th><th>交卷</th>
              </tr>
            </thead>
            <tbody>
              {view.students.map((student) => (
                <tr key={student.attemptId}>
                  <td>
                    <button className="link-button" type="button" onClick={() => go(dataViewPath({ kind: "student", attemptId: student.attemptId }))}>
                      {student.candidateName}
                    </button>
                  </td>
                  <td className="data-mono">{student.candidateId}</td>
                  <td>{num(student.rawScore)} / {num(student.rawTotal)}</td>
                  <td>{pct(student.accuracy)}</td>
                  {student.partCorrect.map((value, index) => (
                    <td key={view.parts[index]?.passageIndex ?? index}>{value == null ? "—" : num(value)}</td>
                  ))}
                  <td>{secText(student.totalDurationSec)}</td>
                  <td>{timeText(student.submittedAt)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <footer className="data-footer">{view.disclaimer}</footer>
    </section>
  );
}

function Stat({ label, value, hint }: { label: string; value: string; hint: string }) {
  return (
    <div className="data-stat">
      <span className="data-stat-label">{label}</span>
      <strong className="data-stat-value">{value}</strong>
      <span className="data-stat-hint">{hint}</span>
    </div>
  );
}
