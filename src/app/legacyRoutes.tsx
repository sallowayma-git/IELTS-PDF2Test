// 兼容期旧页面的唯一入口（计划 §16.2 / §20.2）。
//
// 产品导航已收敛为「题库 / 设置」两个入口，但收敛不等于在替代能力落地前删除用户已有能力：
// 云端复核触发、视觉答案候选、LLM 题组建议、手工转录和写作创作分别要等 P6-P8 才有等价实现。
// 因此这些页面保留在不可导航的 `#/legacy/...` 下，并在打开时打印一条降级日志。
// P10「旧链删除」阶段随 §20.2 清单整体删除本文件。
import { useEffect, useState } from "react";
import { listJobs } from "../api/tauriCommands";
import type { ImportJob } from "../types";
import { Dashboard } from "../pages/Dashboard";
import { DocumentReview } from "../pages/DocumentReview";
import { ExportPage } from "../pages/ExportPage";
import { ImportWizard } from "../pages/ImportWizard";
import { JobList } from "../pages/JobList";
import { LibraryExamDetail } from "../pages/LibraryExamDetail";
import { StructuredAuthoringEditorV2 } from "../pages/StructuredAuthoringEditorV2";
import { UnifiedPreview } from "../pages/UnifiedPreview";
import { WritingStudio } from "../pages/WritingStudio";
import { go, libraryPath, type LegacyPageName } from "./router";

const LEGACY_LABEL: Record<LegacyPageName, string> = {
  dashboard: "工作台",
  jobs: "导题任务",
  import: "新建导题",
  document: "源文档确认",
  preview: "确认与编辑",
  "authoring-v2": "结构化编辑器",
  export: "NAS 导出",
  writing: "写作题创作",
  "library-exam": "题库详情"
};

/** 只有需要 jobs 列表的旧页面才拉取，避免把这次请求带回全局 App。 */
function useLegacyJobs(enabled: boolean, refreshToken: number): ImportJob[] {
  const [jobs, setJobs] = useState<ImportJob[]>([]);
  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    listJobs()
      .then((list) => {
        if (!cancelled) setJobs(list);
      })
      .catch((error) => console.error("[legacy] listJobs failed", error));
    return () => {
      cancelled = true;
    };
  }, [enabled, refreshToken]);
  return jobs;
}

export function LegacyRoutes({ page, itemId }: { page: LegacyPageName; itemId?: string }) {
  const [refreshToken, setRefreshToken] = useState(0);
  const refresh = () => setRefreshToken((value) => value + 1);
  const needsJobs = page === "dashboard" || page === "jobs" || page === "export";
  const jobs = useLegacyJobs(needsJobs, refreshToken);

  useEffect(() => {
    console.info(`[legacy] rendering retired page "${page}" — this surface is removed in the legacy-cleanup phase.`);
  }, [page]);

  const banner = (
    <div className="legacy-banner" role="status">
      <span>
        这是兼容期保留的旧页面「{LEGACY_LABEL[page]}」，不在正式导航中，后续版本会移除。
      </span>
      <button className="ghost small" onClick={() => go(libraryPath())}>
        返回题库
      </button>
    </div>
  );

  return (
    <>
      {banner}
      {page === "dashboard" ? <Dashboard jobs={jobs} refresh={refresh} /> : null}
      {page === "jobs" ? <JobList jobs={jobs} refresh={refresh} /> : null}
      {page === "import" ? <ImportWizard refresh={refresh} /> : null}
      {page === "writing" ? <WritingStudio refresh={refresh} /> : null}
      {page === "export" ? <ExportPage jobId={itemId} jobs={jobs} refresh={refresh} /> : null}
      {page === "document" && itemId ? <DocumentReview jobId={itemId} refresh={refresh} /> : null}
      {page === "preview" && itemId ? <UnifiedPreview jobId={itemId} refresh={refresh} /> : null}
      {page === "authoring-v2" && itemId ? <StructuredAuthoringEditorV2 jobId={itemId} refresh={refresh} /> : null}
      {page === "library-exam" && itemId ? <LibraryExamDetail examId={itemId} refresh={refresh} /> : null}
      {(page === "document" || page === "preview" || page === "authoring-v2" || page === "library-exam") && !itemId ? (
        <p className="empty">这个兼容页面需要在 URL 中带上题目 id，例如 <code>#/legacy/{page}/&lt;id&gt;</code>。</p>
      ) : null}
    </>
  );
}
