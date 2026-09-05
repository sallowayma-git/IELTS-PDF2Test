import type { ReactNode } from "react";
import type { RouteState } from "../app/router";
import { go, libraryPath } from "../app/router";
import wonderLogo from "../assets/wonder-ielts-logo-square.png";

// 极简外壳（计划 §16.3）：只有「题库」和「设置」两个一级入口。
// 已删除：转化工具展开组、流水线 stepper、activeJob 技术条、category/frequency/错误数。
// 各页面自己负责顶部栏，AppShell 不读取任何 job 状态。
const NAV = [
  { label: "题库", short: "库", path: "/library", match: "library" as const },
  { label: "设置", short: "设", path: "/settings", match: "settings" as const }
];

export function AppShell({ route, children }: { route: RouteState; children: ReactNode }) {
  // 工作区要的是最终考试界面的信息密度，外壳不套大圆角卡片，也不占用侧栏宽度。
  const immersive = route.name === "workspace";

  return (
    <div className={`shell ${immersive ? "shell-immersive" : ""}`}>
      {immersive ? null : (
        <aside className="sidebar">
          <div className="sidebar-header">
            <button className="brand" onClick={() => go(libraryPath())}>
              <img className="brand-mark" src={wonderLogo} alt="Wonder IELTS" />
              <span className="brand-copy">
                <strong>IELTS Author</strong>
                <small>题库工作台</small>
              </span>
            </button>
          </div>
          <nav className="primary-nav">
            {NAV.map((entry) => (
              <button
                key={entry.path}
                className={route.name === entry.match ? "active" : ""}
                onClick={() => go(entry.path)}
                title={entry.label}
              >
                <span className="nav-short">{entry.short}</span>
                <span className="nav-label">{entry.label}</span>
              </button>
            ))}
          </nav>
        </aside>
      )}
      <main className="app-main">
        <div className={immersive ? "app-main-content immersive" : "app-main-content"}>{children}</div>
      </main>
    </div>
  );
}
