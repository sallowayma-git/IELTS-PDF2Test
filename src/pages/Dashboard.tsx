import { createImportJob, importSourceFile, parseDocument, runRuleSplit, buildAuthoringIr } from "../api/tauriCommands";
import { StatusPill } from "../components/StatusPill";
import { go } from "../app/router";
import type { ImportJob, JobStatus } from "../types";

const statusOrder: JobStatus[] = ["Working", "NeedsReview", "DraftSaved", "ExportReady", "Exported", "Cleaned"];

export function Dashboard({ jobs, refresh }: { jobs: ImportJob[]; refresh: () => void }) {
  const counts = statusOrder.map((status) => ({ status, count: jobs.filter((job) => job.status === status).length }));

  async function createDemo() {
    try {
      const job = await createImportJob({ title: "The Rise and Fall of Detective Stories", category: "P1", frequency: "medium", tags: ["demo", "mvp"] });
      await importSourceFile(job.jobId, "demo-reading.pdf", "MainQuestion", 512000);
      await parseDocument(job.jobId, { mode: "auto" });
      await runRuleSplit(job.jobId);
      await buildAuthoringIr(job.jobId);
      refresh();
      go(`/jobs/${job.jobId}/groups`);
    } catch (error) {
      console.error(error);
      go("/jobs/new");
    }
  }

  return (
    <section className="dashboard page-enter">
      <div className="hero-panel">
        <div>
          <p className="eyebrow">Epic 8 local studio</p>
          <h2>从导入文档到可加载题库 JS 的本地生产线</h2>
          <p>当前实现以 Tauri 本地应用为主，界面运行在桌面内嵌界面中；旧 Web 设计仅作为导出契约参考。</p>
        </div>
        <div className="hero-actions">
          <button className="primary" onClick={() => go("/jobs/new")}>新建导题任务</button>
          <button className="ghost" onClick={createDemo}>开发演示任务</button>
        </div>
      </div>

      <div className="metric-row">
        {counts.map((item) => (
          <div className="metric" key={item.status}>
            <span>{item.status}</span>
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
            {!jobs.length ? <p className="empty">暂无任务。创建一个任务或生成演示任务开始。</p> : null}
          </div>
        </section>
        <aside className="inspector">
          <p className="eyebrow">MVP contract</p>
          <h3>本阶段验收链路</h3>
          <ol className="check-list">
            <li>上传/创建 Job 并保留来源文件记录</li>
            <li>生成 Document IR 并可审阅 block</li>
            <li>规则粗切 Passage、题组和答案</li>
            <li>编辑 Authoring IR 并模板化渲染</li>
            <li>导出 ReadingExamSourceV1、单题 JS、manifest</li>
          </ol>
        </aside>
      </div>
    </section>
  );
}
