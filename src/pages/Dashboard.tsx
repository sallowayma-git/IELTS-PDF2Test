import { StatusPill } from "../components/StatusPill";
import { go } from "../app/router";
import type { ImportJob, JobStatus } from "../types";
import { jobStatusLabel } from "../utils/displayLabels";

const statusOrder: JobStatus[] = ["Working", "NeedsReview", "DraftSaved", "ExportReady", "Exported", "Cleaned"];

export function Dashboard({ jobs, refresh }: { jobs: ImportJob[]; refresh: () => void }) {
  const counts = statusOrder.map((status) => ({ status, count: jobs.filter((job) => job.status === status).length }));

  return (
    <section className="dashboard page-enter">
      <div className="hero-panel">
        <div>
          <p className="eyebrow">本地作者工具</p>
          <h2>从导入文件到可发布题稿的本地流程</h2>
          <p>当前实现以桌面本地应用为主，界面运行在桌面窗口内；旧 Web 设计仅作为输出格式参考。</p>
        </div>
        <div className="hero-actions">
          <button className="primary" onClick={() => go("/jobs/new")}>新建导题任务</button>
          <button className="ghost" onClick={() => go("/writing")}>写作题创作</button>
        </div>
      </div>

      <div className="metric-row">
        {counts.map((item) => (
          <div className="metric" key={item.status}>
            <span>{jobStatusLabel(item.status)}</span>
            <strong>{item.count}</strong>
          </div>
        ))}
      </div>

      <div className="two-column">
        <section>
          <div className="section-heading">
            <p className="eyebrow">Recent jobs</p>
            <h3>最近导题任务</h3>
          </div>
          <div className="job-table">
            {jobs.slice(0, 8).map((job) => (
              <button className="job-row" key={job.jobId} onClick={() => go(`/jobs/${job.jobId}/document`)}>
                <span>
                  <strong>{job.title}</strong>
                  <small>{job.jobId}</small>
                </span>
                <StatusPill status={job.status} />
                <span>{job.category}</span>
                <span>{job.updatedAt.slice(0, 10)}</span>
              </button>
            ))}
            {!jobs.length ? <p className="empty">暂无任务。创建一个任务开始。</p> : null}
          </div>
        </section>
        <aside className="inspector">
          <p className="eyebrow">工作提示</p>
          <h3>普通流程</h3>
          <ol className="check-list">
            <li>选择本地 PDF、DOCX、TXT 或 MD 文件</li>
            <li>核对解析结果，必要时补充人工转录</li>
            <li>确认题组、题型和答案</li>
            <li>编辑可编辑题稿并完成预览校验</li>
            <li>导出单题文件或加入 Pack</li>
          </ol>
        </aside>
      </div>
    </section>
  );
}
