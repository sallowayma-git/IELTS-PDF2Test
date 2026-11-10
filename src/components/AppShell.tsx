import type { ReactNode } from "react";
import { ChartColumn, FileInput, Library, Settings2 } from "lucide-react";
import type { RouteState } from "../app/router";
import { go } from "../app/router";
import wonderLogo from "../assets/wonder-ielts-logo-square.png";

// 外壳：一级入口「导入 / 题库 / 数据 / 设置」。导入入口带角标（识别中 + 待检查 + 失败）。
// 工作区是沉浸式全屏，不显示侧栏。
const NAV = [
  { label: "导入", icon: FileInput, path: "/import", match: "import" as const },
  { label: "题库", icon: Library, path: "/library", match: "library" as const },
  { label: "数据", icon: ChartColumn, path: "/data", match: "data" as const },
  { label: "设置", icon: Settings2, path: "/settings", match: "settings" as const }
];

export function AppShell({
  route,
  importBadge,
  children
}: {
  route: RouteState;
  /** 导入角标；`undefined` 表示还没读到，不显示数字。 */
  importBadge?: number;
  children: ReactNode;
}) {
  const immersive = route.name === "workspace";

  return (
    <div className={`shell ${immersive ? "shell-immersive" : ""}`}>
      {immersive ? null : (
        <aside className="sidebar">
          <div className="sidebar-header">
            <button className="brand" onClick={() => go("/library")}>
              <img className="brand-mark" src={wonderLogo} alt="Wonder IELTS" />
              <span className="brand-copy">
                <strong>IELTS Author</strong>
                <small>题库工作台</small>
              </span>
            </button>
          </div>
          <nav className="primary-nav" aria-label="主导航">
            {NAV.map((entry) => (
              <button
                key={entry.path}
                className={route.name === entry.match ? "active" : ""}
                aria-current={route.name === entry.match ? "page" : undefined}
                onClick={() => go(entry.path)}
                title={entry.label}
                data-testid={`nav-${entry.match}`}
              >
                <span className="nav-short" aria-hidden="true"><entry.icon size={18} strokeWidth={1.8} /></span>
                <span className="nav-label">{entry.label}</span>
                {entry.match === "import" && importBadge ? (
                  <span className="nav-badge" data-testid="nav-import-badge" aria-label={`${importBadge} 项待处理`}>{importBadge}</span>
                ) : null}
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
