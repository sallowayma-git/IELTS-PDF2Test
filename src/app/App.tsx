import { useEffect, useRef, useState } from "react";
import { AppShell } from "../components/AppShell";
import { ImportPage } from "../features/import/ImportPage";
import { useImportBadge } from "../features/import/useImportBadge";
import { LibraryPage } from "../features/library/LibraryPage";
import { ExamWorkspacePage } from "../features/editor/ExamWorkspacePage";
import { SettingsPage } from "../features/settings/SettingsPage";
import { DataPage } from "../features/data/DataPage";
import { applyLegacyRedirect, go, parseRoute, type RouteState } from "./router";

export function App() {
  const [route, setRoute] = useState<RouteState>(() => parseRoute());
  const importBadge = useImportBadge();
  // 启动默认落点：没有指定 hash 时，有未完成导入进「导入」，否则进「题库」（只决定一次）。
  const openedWithoutHash = useRef(!window.location.hash);
  const startupRouteDecided = useRef(false);

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

  useEffect(() => {
    if (startupRouteDecided.current || !openedWithoutHash.current || importBadge === undefined) return;
    startupRouteDecided.current = true;
    go(importBadge > 0 ? "/import" : "/library");
  }, [importBadge]);

  return (
    <AppShell route={route} importBadge={importBadge}>
      {route.name === "import" ? <ImportPage /> : null}
      {route.name === "library" ? <LibraryPage intent={route.intent} pickItemId={route.pickItemId} modality={route.modality} part={route.part} /> : null}
      {route.name === "workspace" && route.itemId ? (
        <ExamWorkspacePage key={route.itemId} itemId={route.itemId} intent={route.intent} />
      ) : null}
      {route.name === "data" ? <DataPage view={route.data ?? { kind: "batches" }} /> : null}
      {route.name === "settings" ? <SettingsPage /> : null}
    </AppShell>
  );
}
