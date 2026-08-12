import { useEffect, useMemo, useState } from "react";
import { AppShell } from "../components/AppShell";
import { Dashboard } from "../pages/Dashboard";
import { DocumentReview } from "../pages/DocumentReview";
import { ImportWizard } from "../pages/ImportWizard";
import { JobList } from "../pages/JobList";
import { Settings } from "../pages/Settings";
import { UnifiedPreview } from "../pages/UnifiedPreview";
import { ExportPage } from "../pages/ExportPage";
import { WritingStudio } from "../pages/WritingStudio";
import { LibraryPage } from "../pages/LibraryPage";
import { LibraryExamDetail } from "../pages/LibraryExamDetail";
import { StructuredAuthoringEditorV2 } from "../pages/StructuredAuthoringEditorV2";
import { getJob, listJobs } from "../api/tauriCommands";
import type { ImportJob } from "../types";
import { parseRoute, type RouteState } from "./router";
import { isPhase5EditorEnabled } from "../config/featureFlags";

export function App() {
  const [route, setRoute] = useState<RouteState>(() => parseRoute());
  const [jobs, setJobs] = useState<ImportJob[]>([]);
  const [activeJob, setActiveJob] = useState<ImportJob | undefined>();
  const [refreshToken, setRefreshToken] = useState(0);

  const refresh = () => setRefreshToken((value) => value + 1);

  useEffect(() => {
    const onHash = () => setRoute(parseRoute());
    window.addEventListener("hashchange", onHash);
    if (!window.location.hash) window.location.hash = "/dashboard";
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  useEffect(() => {
    listJobs().then(setJobs).catch(console.error);
  }, [refreshToken]);

  useEffect(() => {
    if (!route.jobId) {
      setActiveJob(undefined);
      return;
    }
    getJob(route.jobId)
      .then((detail) => setActiveJob(detail.job))
      .catch(() => setActiveJob(undefined));
  }, [route.jobId, refreshToken]);

  const page = useMemo(() => {
    const common = { refresh };
    if (route.name === "dashboard") return <Dashboard jobs={jobs} refresh={refresh} />;
    if (route.name === "jobs") return <JobList jobs={jobs} refresh={refresh} />;
    if (route.name === "new") return <ImportWizard refresh={refresh} />;
    if (route.jobId && route.name === "document") return <DocumentReview jobId={route.jobId} {...common} />;
    if (route.jobId && (route.name === "split" || route.name === "groups" || route.name === "llm-review")) {
      return <UnifiedPreview jobId={route.jobId} {...common} />;
    }
    if (route.jobId && route.name === "preview") return <UnifiedPreview jobId={route.jobId} {...common} />;
    if (route.jobId && route.name === "authoring-v2") return isPhase5EditorEnabled()
      ? <StructuredAuthoringEditorV2 jobId={route.jobId} refresh={refresh} />
      : <UnifiedPreview jobId={route.jobId} {...common} />;
    if (route.name === "phase5") return isPhase5EditorEnabled()
      ? <StructuredAuthoringEditorV2 jobId="phase5-editor-fixture" refresh={refresh} />
      : <Dashboard jobs={jobs} refresh={refresh} />;
    if (route.jobId && route.name === "export") return <ExportPage jobId={route.jobId} jobs={jobs} {...common} />;
    if (route.name === "export") return <ExportPage jobs={jobs} {...common} />;
    if (route.name === "writing") return <WritingStudio {...common} />;
    if (route.name === "library") return <LibraryPage {...common} />;
    if (route.name === "libraryExam" && route.examId) return <LibraryExamDetail examId={route.examId} {...common} />;
    if (route.name === "settings") return <Settings refresh={refresh} />;
    return <Dashboard jobs={jobs} refresh={refresh} />;
  }, [jobs, route, refreshToken]);

  return (
    <AppShell route={route} activeJob={activeJob}>
      {page}
    </AppShell>
  );
}
