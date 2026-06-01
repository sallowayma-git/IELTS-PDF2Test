import { deleteJob } from "../api/tauriCommands";
import { go } from "../app/router";
import { StatusPill } from "../components/StatusPill";
import type { ImportJob, WorkflowStep } from "../types";

const stepPath: Record<WorkflowStep, string> = {
  Upload: "document",
  DocumentReview: "document",
  Split: "split",
  Authoring: "groups",
  LlmReview: "llm-review",
  Preview: "preview",
  Export: "export",
  Pack: "pack"
};

export function JobList({ jobs, refresh }: { jobs: ImportJob[]; refresh: () => void }) {
  async function remove(jobId: string) {
    await deleteJob(jobId);
    refresh();
  }

  return (
    <section className="page-enter">
      <div className="section-heading spread">
        <div>
          <p className="eyebrow">Jobs</p>
          <h2>导题任务</h2>
        </div>
        <button className="primary" onClick={() => go("/jobs/new")}>新建</button>
      </div>
      <div className="job-table tall">
        {jobs.map((job) => (
          <div className="job-row static" key={job.jobId}>
            <button className="link-cell" onClick={() => go(`/jobs/${job.jobId}/document`)}>
              <strong>{job.title}</strong>
              <small>{job.jobId}</small>
            </button>
            <StatusPill status={job.status} />
            <span>{job.category}/{job.frequency}</span>
            <span>{job.sourceFiles.length} 个文件</span>
            <button className="ghost small" onClick={() => go(`/jobs/${job.jobId}/${stepPath[job.currentStep] ?? "document"}`)}>打开</button>
            <button className="danger small" onClick={() => remove(job.jobId)}>删除</button>
          </div>
        ))}
        {!jobs.length ? <p className="empty">任务列表为空。</p> : null}
      </div>
    </section>
  );
}
