import { analyticsApi, type StudentView } from "../../api/analyticsClient";
import { go } from "../../app/router";
import { taskTypeLabel } from "../../utils/displayLabels";
import { dataViewPath } from "./dataRoute";
import { msText, num, pct, secText, timeText, useLoad } from "./dataFormat";

export function StudentPanel({ attemptId }: { attemptId: string }) {
  const student = useLoad<StudentView>(() => analyticsApi.student(attemptId), [attemptId], "读取学生数据失败，请稍后重试。");
  const view = student.data;
  if (student.error) return <p className="error-text" role="alert">{student.error}</p>;
  if (!view) return <p className="data-empty">正在读取…</p>;

  return (
    <section className="data-student" data-testid="data-student">
      <header className="data-toolbar">
        <div>
          <h1>{view.candidateName}</h1>
          <p className="data-subtitle">卡号 {view.candidateId} · 交卷 {timeText(view.submittedAt)}</p>
        </div>
      </header>

      <div className="data-stat-grid">
        <div className="data-stat">
          <span className="data-stat-label">原始分</span>
          <strong className="data-stat-value">{num(view.rawScore)} / {num(view.rawTotal)}</strong>
          <span className="data-stat-hint">{view.valid ? "有效计分" : `不计入：${view.invalidReason ?? ""}`}</span>
        </div>
        <div className="data-stat">
          <span className="data-stat-label">正确率</span>
          <strong className="data-stat-value">{pct(view.accuracy)}</strong>
          <span className="data-stat-hint">按得分点（N = 本人作答）</span>
        </div>
      </div>

      <section className="data-section">
        <h2>各篇作答</h2>
        {view.overlapNote ? <p className="data-notice" role="note">{view.overlapNote}</p> : null}
        <div className="data-table-scroll" role="region" aria-label="分篇成绩" tabIndex={0}>
          <table className="data-table">
            <thead>
              <tr><th>Part</th><th>得分点</th><th>正确率</th><th>用时</th><th>作答区间</th><th /></tr>
            </thead>
            <tbody>
              {view.passages.map((passage) => (
                <tr key={passage.passageIndex}>
                  <td>{passage.label}{passage.title ? <span className="data-muted"> {passage.title}</span> : null}</td>
                  <td>{num(passage.correct)} / {num(passage.total)}</td>
                  <td>{pct(passage.accuracy)}</td>
                  <td>
                    {secText(passage.durationSec)}
                    {passage.overlap ? <span className="data-chip warn" title="与其他篇的作答区间重叠">用时可能重叠</span> : null}
                  </td>
                  <td className="data-mono">{timeText(passage.startedAt)} — {timeText(passage.endedAt)}</td>
                  <td />
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      <section className="data-section">
        <h2>错题明细 <small>{view.wrongAnswers.length} 题</small></h2>
        {view.wrongAnswers.length === 0 ? (
          <p className="data-empty">没有错题。</p>
        ) : (
          <div className="data-table-scroll" role="region" aria-label="错题明细" tabIndex={0}>
            <table className="data-table">
              <thead>
                <tr>
                  <th>题号</th><th>Part</th><th>题型</th><th>学生答案</th><th>标准答案</th>
                  <th>停留</th><th>改答</th><th>访问</th><th />
                </tr>
              </thead>
              <tbody>
                {view.wrongAnswers.map((wrong) => (
                  <tr key={`${wrong.passageIndex}|${wrong.questionId}`}>
                    <td>{wrong.displayLabel}</td>
                    <td>{wrong.passageLabel}</td>
                    <td>{taskTypeLabel(wrong.questionKind)}</td>
                    <td>{wrong.userAnswer}</td>
                    <td>{wrong.correctAnswer}</td>
                    <td>{msText(wrong.elapsedMs)}</td>
                    <td>{wrong.changeCount}</td>
                    <td>{wrong.visitCount}</td>
                    <td>
                      <button
                        className="ghost small"
                        type="button"
                        onClick={() =>
                          go(dataViewPath({
                            kind: "question",
                            batchId: view.batchId,
                            paperId: view.paperId,
                            passageIndex: wrong.passageIndex,
                            questionId: wrong.questionId
                          }))
                        }
                      >
                        看此题
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {view.writings.length > 0 ? (
        <section className="data-section">
          <h2>写作</h2>
          <div className="data-table-scroll" role="region" aria-label="写作记录" tabIndex={0}>
            <table className="data-table">
              <thead>
                <tr><th>任务</th><th>字数</th><th>用时</th><th>提交</th></tr>
              </thead>
              <tbody>
                {view.writings.map((writing, index) => (
                  <tr key={`${writing.taskType}-${index}`}>
                    <td>{writing.taskType === "task1" ? "Task 1" : writing.taskType === "task2" ? "Task 2" : writing.taskType}</td>
                    <td>{writing.wordCount ?? "—"}</td>
                    <td>{secText(writing.durationSec)}</td>
                    <td>{timeText(writing.submittedAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      ) : null}

      <footer className="data-footer">{view.disclaimer}</footer>
    </section>
  );
}
