import { useState } from "react";
import { analyticsApi, type ImportReport, type IdentityIssue, type Overview } from "../../api/analyticsClient";
import { go } from "../../app/router";
import { dataViewPath } from "./dataRoute";
import { errorText, nText, paperText, timeText, useLoad } from "./dataFormat";

export function BatchOverviewPanel({ root }: { root: string }) {
  const [report, setReport] = useState<ImportReport | undefined>(undefined);
  const [importError, setImportError] = useState<string | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  const overview = useLoad<Overview | undefined>(
    () => analyticsApi.overview(root).then((value) => value),
    [root],
    "读取数据总览失败，请稍后重试。",
    Boolean(root)
  );

  async function runImport() {
    setBusy(true);
    setImportError(undefined);
    try {
      const result = await analyticsApi.importSnapshots(root);
      setReport(result);
      overview.reload();
    } catch (caught) {
      setImportError(errorText(caught, "导入失败，请稍后重试。"));
    } finally {
      setBusy(false);
    }
  }

  if (!root) {
    return (
      <>
        <header className="data-toolbar">
          <div>
            <h1>数据</h1>
            <p className="data-subtitle">查看考试批次、学生成绩和题目作答情况。</p>
          </div>
        </header>
        <section className="data-empty-state" data-testid="data-not-configured">
          <h2>还没有配置作答数据目录</h2>
          <p>请在「设置 → 作答数据目录」选择学生端的考试作答存储目录（其中应有 snapshots/ 与 status/）。</p>
          <button className="primary" type="button" onClick={() => go("/settings")}>去设置</button>
        </section>
      </>
    );
  }

  const data = overview.data;
  return (
    <section className="data-batches" data-testid="data-batches">
      <header className="data-toolbar">
        <div>
          <h1>数据</h1>
          <p className="data-subtitle">只读学生端交卷快照，判分以学生端记录为准；只展示原始分与正确率。</p>
        </div>
        <div className="data-toolbar-actions">
          <span className="data-root" title={root}>作答数据目录：{root}</span>
          <button className="primary" type="button" data-testid="data-import" disabled={busy} onClick={() => void runImport()}>
            {busy ? "正在导入…" : "从 NAS 导入"}
          </button>
        </div>
      </header>

      {report ? (
        <p className="data-notice" role="status" data-testid="data-import-result">
          扫描 {report.scanned} 份快照：新导入 {report.ok}，跳过 {report.skipped}，失败 {report.failed}（写入 {report.attemptsImported} 份作答）。
          {report.snapshotsDirFound ? null : " 未找到 snapshots 目录，请确认作答数据目录是学生端的存储根。"}
        </p>
      ) : null}
      {importError ? <p className="error-text" role="alert">{importError}</p> : null}
      {overview.error ? <p className="error-text" role="alert">{overview.error}</p> : null}
      {overview.loading && !data ? <p className="data-empty">正在读取…</p> : null}

      {data ? (
        <>
          {data.batches.length === 0 ? (
            <p className="data-empty" data-testid="data-no-batches">
              还没有可统计的交卷快照。学生端交卷后，点「从 NAS 导入」即可出现在这里。
            </p>
          ) : (
            <div className="data-batch-grid">
              {data.batches.map((batch) => {
                const examPath = dataViewPath({ kind: "exam", batchId: batch.batchId, paperId: batch.paperId });
                return (
                  <article className="data-card" key={`${batch.batchId}|${batch.paperId}`} data-testid="data-batch-card">
                    <header>
                      <h2>{batch.batchId}</h2>
                      <span className="data-chip">卷 {paperText(batch.paperId)}</span>
                    </header>
                    <dl className="data-metrics">
                      <div>
                        <dt>信标（实时）</dt>
                        <dd>已签到 {batch.checkedIn} · 进行中 {batch.inProgress} · 已交卷 {batch.submittedBeacon} · 作废 {batch.voided}</dd>
                      </div>
                      <div>
                        <dt>快照（已导入）</dt>
                        <dd>
                          已导入 {batch.imported} 人 · 有效计分 {nText(batch.validScored)}
                          {batch.importExceptions > 0 ? ` · 导入异常 ${batch.importExceptions}` : ""}
                        </dd>
                      </div>
                      <div>
                        <dt>最近交卷</dt>
                        <dd>{timeText(batch.lastSubmittedAt)}</dd>
                      </div>
                    </dl>
                    <button className="ghost small" type="button" onClick={() => go(examPath)}>查看考试</button>
                  </article>
                );
              })}
            </div>
          )}

          {data.exceptions.length > 0 ? (
            <section className="data-section" data-testid="data-exceptions">
              <h2>导入异常队列（{data.exceptions.length}）</h2>
              <p className="data-subtitle">这些快照没有进入统计；不影响其他快照。修复源文件后重新导入即可。</p>
              <div className="data-table-scroll" role="region" aria-label="导入异常记录" tabIndex={0}>
                <table className="data-table">
                  <thead>
                    <tr><th>快照</th><th>原因</th><th>时间</th></tr>
                  </thead>
                  <tbody>
                    {data.exceptions.map((record) => (
                      <tr key={`${record.file}|${record.importedAt}`}>
                        <td className="data-mono">{record.file}</td>
                        <td>{record.error ?? "未知原因"}</td>
                        <td>{timeText(record.importedAt)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          ) : null}

          {data.identityIssues.length > 0 ? (
            <section className="data-section" data-testid="data-identity">
              <h2>待核对（身份）</h2>
              <ul className="data-list">
                {data.identityIssues.map((issue) => (
                  <li key={`${issue.kind}|${issue.candidateId}|${issue.candidateName}`}>{identityText(issue)}</li>
                ))}
              </ul>
            </section>
          ) : null}

          {data.beaconErrors.length > 0 ? (
            <p className="data-subtitle">状态信标有 {data.beaconErrors.length} 个文件无法解析，已忽略。</p>
          ) : null}
        </>
      ) : null}

      <footer className="data-footer">{data?.disclaimer ?? "机构模拟测试原始分，非 IELTS 官方成绩。"}</footer>
    </section>
  );
}

function identityText(issue: IdentityIssue): string {
  const names = Array.from(new Set(issue.attempts.map((attempt) => attempt.candidateName))).join("、");
  const ids = Array.from(new Set(issue.attempts.map((attempt) => attempt.candidateId))).join("、");
  if (issue.kind === "same_id_diff_name") return `卡号 ${issue.candidateId} 对应不同姓名：${names}`;
  return `姓名「${issue.candidateName}」对应不同卡号：${ids}`;
}
