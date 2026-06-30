import { useEffect, useState, type ReactNode } from "react";
import type { ImportJob } from "../types";
import type { RouteState } from "../app/router";
import { go } from "../app/router";
import { StatusPill } from "./StatusPill";

const nav = [
  { label: "工作台", short: "台", path: "/dashboard", match: "dashboard" },
  { label: "导题任务", short: "任务", path: "/jobs", match: "jobs" },
  { label: "新建导题", short: "新建", path: "/jobs/new", match: "new" },
  { label: "写作题创作", short: "写作", path: "/writing", match: "writing" },
  { label: "导出/组卷", short: "导出", path: "/packs", match: "packs" },
  { label: "设置", short: "设", path: "/settings", match: "settings" }
];

const steps = [
  ["document", "源文档确认"],
  ["preview", "确认与编辑"],
  ["export", "导出发布"]
] as const;

export function AppShell({ route, activeJob, children }: { route: RouteState; activeJob?: ImportJob; children: ReactNode }) {
  const [collapsed, setCollapsed] = useState(false);
  const activeStep = route.name === "split" || route.name === "groups" || route.name === "llm-review"
    ? "preview"
    : route.name;

  useEffect(() => {
    setCollapsed(window.localStorage.getItem("ielts-author-studio.sidebar-collapsed") === "1");
  }, []);

  function toggleSidebar() {
    setCollapsed((current) => {
      const next = !current;
      window.localStorage.setItem("ielts-author-studio.sidebar-collapsed", next ? "1" : "0");
      return next;
    });
  }

  return (
    <div className={`shell ${collapsed ? "sidebar-collapsed" : ""}`}>
      <aside className="sidebar">
        <div className="sidebar-header">
          <button className="brand" onClick={() => go("/dashboard")}>
            <span className="brand-mark">IA</span>
            <span className="brand-copy">
              <strong>IELTS Author</strong>
              <small>Epic 8 Studio</small>
            </span>
          </button>
          <button
            className="ghost small sidebar-toggle"
            onClick={toggleSidebar}
            title={collapsed ? "展开导航栏" : "收起导航栏"}
            aria-label={collapsed ? "展开导航栏" : "收起导航栏"}
          >
            {collapsed ? "›" : "‹"}
          </button>
        </div>
        <nav className="primary-nav">
          {nav.map((item) => (
            <button key={item.path} className={route.name === item.match ? "active" : ""} onClick={() => go(item.path)} title={item.label}>
              <span className="nav-short">{item.short}</span>
              <span className="nav-label">{item.label}</span>
            </button>
          ))}
        </nav>
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
