import type { ImportJob } from "../types";
import type { RouteState } from "../app/router";
import { go } from "../app/router";
import { StatusPill } from "./StatusPill";

const nav = [
  { label: "工作台", path: "/dashboard", match: "dashboard" },
  { label: "导题任务", path: "/jobs", match: "jobs" },
  { label: "新建导题", path: "/jobs/new", match: "new" },
  { label: "Pack 组卷", path: "/packs", match: "packs" },
  { label: "设置", path: "/settings", match: "settings" }
];

const steps = [
  ["document", "源文档确认"],
  ["preview", "确认与编辑"],
  ["export", "导出发布"]
] as const;

export function AppShell({ route, activeJob, children }: { route: RouteState; activeJob?: ImportJob; children: React.ReactNode }) {
  const activeStep = route.name === "split" || route.name === "groups" || route.name === "llm-review"
    ? "preview"
    : route.name;
  return (
    <div className="shell">
      <aside className="sidebar">
        <button className="brand" onClick={() => go("/dashboard")}>
          <span className="brand-mark">IA</span>
          <span>
            <strong>IELTS Author</strong>
            <small>Epic 8 Studio</small>
          </span>
        </button>
        <nav className="primary-nav">
          {nav.map((item) => (
            <button key={item.path} className={route.name === item.match ? "active" : ""} onClick={() => go(item.path)}>
              {item.label}
            </button>
          ))}
        </nav>
        <div className="sidebar-note">
          <span>本地处理说明</span>
          <strong>桌面端本地处理</strong>
          <p>文件解析、题稿生成、校验和导出都在本地应用内完成；开发预览只用于调试，不代表普通用户流程。</p>
        </div>
      </aside>
      <main className="workspace">
        <header className="topbar">
          <div>
            <p className="eyebrow">本地处理流程</p>
            <h1>{activeJob ? activeJob.title : "作者端工作台"}</h1>
          </div>
          {activeJob ? (
            <div className="job-strip">
              <StatusPill status={activeJob.status} />
              <span>{activeJob.category}</span>
              <span>{activeJob.frequency}</span>
              <span>{activeJob.issueCounts.errors} 个错误</span>
            </div>
          ) : (
            <button className="primary" onClick={() => go("/jobs/new")}>新建导题任务</button>
          )}
        </header>
        {activeJob ? (
          <div className="stepper">
            {steps.map(([step, label]) => (
              <button key={step} className={activeStep === step ? "active" : ""} onClick={() => go(`/jobs/${activeJob.jobId}/${step}`)}>
                {label}
              </button>
            ))}
          </div>
        ) : null}
        <div className="surface">{children}</div>
      </main>
    </div>
  );
}
