import { useEffect, useState, type ReactNode } from "react";
import type { ImportJob } from "../types";
import type { RouteState } from "../app/router";
import { go } from "../app/router";
import { StatusPill } from "./StatusPill";
import wonderLogo from "../assets/wonder-ielts-logo-square.png";

// 导航结构：4 个一级入口。工作台 / 题库管理 / 设置 为无子项的一级；
// 「转化工具」为可展开分组，收录原有一次工具的 4 个页面。
type LeafNav = { label: string; short: string; path: string; match: string };
type NavEntry =
  | ({ kind: "leaf" } & LeafNav)
  | { kind: "group"; label: string; short: string; match: string[]; defaultOpen?: boolean; children: LeafNav[] };

const navEntries: NavEntry[] = [
  { kind: "leaf", label: "工作台", short: "台", path: "/dashboard", match: "dashboard" },
  {
    kind: "group",
    label: "转化工具",
    short: "转化",
    match: ["jobs", "new", "document", "split", "groups", "llm-review", "preview", "export", "writing"],
    defaultOpen: true,
    children: [
      { label: "导题任务", short: "任务", path: "/jobs", match: "jobs" },
      { label: "新建导题", short: "新建", path: "/jobs/new", match: "new" },
      { label: "写作题创作", short: "写作", path: "/writing", match: "writing" },
      { label: "NAS 导出", short: "导出", path: "/export", match: "export" }
    ]
  },
  { kind: "leaf", label: "题库管理", short: "题库", path: "/library", match: "library" },
  { kind: "leaf", label: "设置", short: "设", path: "/settings", match: "settings" }
];

const steps = [
  ["document", "源文档确认"],
  ["preview", "确认与编辑"],
  ["export", "导出发布"]
] as const;

const EXPAND_KEY = "ielts-author-studio.nav.expanded.";

export function AppShell({ route, activeJob, children }: { route: RouteState; activeJob?: ImportJob; children: ReactNode }) {
  const [collapsed, setCollapsed] = useState(false);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const activeStep = route.name === "split" || route.name === "groups" || route.name === "llm-review"
    ? "preview"
    : route.name;

  useEffect(() => {
    setCollapsed(window.localStorage.getItem("ielts-author-studio.sidebar-collapsed") === "1");
    // 读取各分组的展开态；默认展开的分组在未记录时视为展开。
    const next: Record<string, boolean> = {};
    for (const entry of navEntries) {
      if (entry.kind === "group") {
        const stored = window.localStorage.getItem(EXPAND_KEY + entry.label);
        next[entry.label] = stored === null ? !!entry.defaultOpen : stored === "1";
      }
    }
    setExpanded(next);
  }, []);

  // 当前路由命中某分组子项时，自动展开该分组。
  useEffect(() => {
    setExpanded((current) => {
      let changed = false;
      const next = { ...current };
      for (const entry of navEntries) {
        if (entry.kind === "group" && entry.match.includes(route.name) && !next[entry.label]) {
          next[entry.label] = true;
          changed = true;
        }
      }
      return changed ? next : current;
    });
  }, [route.name]);

  function toggleSidebar() {
    setCollapsed((current) => {
      const next = !current;
      window.localStorage.setItem("ielts-author-studio.sidebar-collapsed", next ? "1" : "0");
      return next;
    });
  }

  function toggleGroup(label: string) {
    setExpanded((current) => {
      const next = { ...current, [label]: !current[label] };
      window.localStorage.setItem(EXPAND_KEY + label, next[label] ? "1" : "0");
      return next;
    });
  }

  return (
    <div className={`shell ${collapsed ? "sidebar-collapsed" : ""}`}>
      <aside className="sidebar">
        <div className="sidebar-header">
          <button className="brand" onClick={() => go("/dashboard")}>
            <img className="brand-mark" src={wonderLogo} alt="Wonder IELTS" />
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
          {navEntries.map((entry) => {
            if (entry.kind === "leaf") {
              const active = route.name === entry.match || (entry.match === "library" && route.name === "libraryExam");
              return (
                <button key={entry.path} className={active ? "active" : ""} onClick={() => go(entry.path)} title={entry.label}>
                  <span className="nav-short">{entry.short}</span>
                  <span className="nav-label">{entry.label}</span>
                </button>
              );
            }
            const isOpen = !!expanded[entry.label];
            const groupActive = entry.match.includes(route.name);
            return (
              <div className={`nav-group ${isOpen ? "open" : ""}`} key={entry.label}>
                <button
                  className={`nav-group-title ${groupActive ? "in-active-group" : ""}`}
                  // 折叠态：分组标题直接跳首个子项（子项列表被 CSS 隐藏，无法展开）；
                  // 展开态：切换分组展开/收起。
                  onClick={() => (collapsed ? go(entry.children[0].path) : toggleGroup(entry.label))}
                  title={collapsed ? `${entry.label}：${entry.children[0].label}` : entry.label}
                >
                  <span className="nav-short">{entry.short}</span>
                  <span className="nav-label">{entry.label}</span>
                  <span className="nav-caret">{isOpen ? "▾" : "▸"}</span>
                </button>
                {isOpen ? (
                  <div className="nav-children">
                    {entry.children.map((child) => (
                      <button
                        key={child.path}
                        className={`nav-child ${route.name === child.match ? "active" : ""}`}
                        onClick={() => go(child.path)}
                        title={child.label}
                      >
                        <span className="nav-label">{child.label}</span>
                      </button>
                    ))}
                  </div>
                ) : null}
              </div>
            );
          })}
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
