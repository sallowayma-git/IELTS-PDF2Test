import { useEffect, useState } from "react";
import { AppShell } from "../components/AppShell";
import { LibraryPage } from "../features/library/LibraryPage";
import { ExamWorkspacePage } from "../features/editor/ExamWorkspacePage";
import { SettingsPage } from "../features/settings/SettingsPage";
import { LegacyRoutes } from "./legacyRoutes";
import { applyLegacyRedirect, parseRoute, type RouteState } from "./router";

export function App() {
  const [route, setRoute] = useState<RouteState>(() => parseRoute());

  useEffect(() => {
    const sync = () => {
      // 旧链接先原地换成新链接；replace 会再触发一次 hashchange，由下一轮 sync 解析。
      if (applyLegacyRedirect()) return;
      setRoute(parseRoute());
    };
    window.addEventListener("hashchange", sync);
    if (!window.location.hash) window.location.hash = "/library";
    else sync();
    return () => window.removeEventListener("hashchange", sync);
  }, []);

  return (
    <AppShell route={route}>
      {route.name === "library" ? <LibraryPage intent={route.intent} /> : null}
      {route.name === "workspace" && route.itemId ? (
        <ExamWorkspacePage itemId={route.itemId} intent={route.intent} />
      ) : null}
      {route.name === "settings" ? <SettingsPage /> : null}
      {route.name === "legacy" && route.legacyPage ? (
        <LegacyRoutes page={route.legacyPage} itemId={route.itemId} />
      ) : null}
    </AppShell>
  );
}
