import { useEffect, useMemo, useState } from "react";
import { chooseExportDirectory } from "../api/desktopDialogs";
import { exportNasLibrary, exportReadingAssets, exportReadingJs, exportWritingLibrary, listWritingJobs } from "../api/tauriCommands";
import { StatusPill } from "../components/StatusPill";
import type { ExportResult, ImportJob, JsExportResult, NasExportResult, ValidationIssue, ValidationPolicy, WritingExportResult, WritingJob } from "../types";
import { validationIssueDisplay } from "../utils/displayLabels";
import { takePublishIntent } from "../utils/publishIntent";

type ExportMode = "single-js" | "batch-js" | "full-assets" | "nas-library" | "writing-library";

const EXPORTABLE_STATUSES = new Set(["Working", "NeedsReview", "DraftSaved", "ExportReady", "Exported", "Cleaned"]);
const DEFAULT_NAS_STATUSES = new Set(["DraftSaved", "ExportReady", "Exported", "Cleaned"]);

interface ExportDiagnostics {
  title: string;
  issues: ValidationIssue[];
  guidance: string[];
  canForce: boolean;
}

function parseValidationPayload(message: string): { issues?: ValidationIssue[] } | undefined {
  const jsonStart = message.indexOf("{");
  if (jsonStart < 0) return undefined;
  try {
    const parsed = JSON.parse(message.slice(jsonStart)) as { issues?: ValidationIssue[] };
    return Array.isArray(parsed.issues) ? parsed : undefined;
  } catch {
    return undefined;
  }
}

function plainExportGuidance(message: string): string[] {
  const guidance = [
    /answer is empty|答案为空|答案不能为空/i.test(message) ? "部分题目没有答案；答案为可选项，不会单独阻止导出。" : "",
    /human verified|人工确认|NeedsReview|待审核/i.test(message) ? "题目或题组还没有完成确认，请在题稿编辑页完成确认。" : "",
    /source review|parser warning|low-confidence parsed block|源文档/i.test(message) ? "源文档识别结果仍需确认，请回到源文档审核页处理。" : "",
    /cloudComparison|云端/i.test(message) ? "云端整卷对照仍有差异，请在题稿编辑页核对云端/本地差异。" : ""
  ].filter(Boolean);
  return guidance.length ? guidance : ["导出前检查没有通过。你可以返回编辑，也可以忽略内容检查继续导出。"];
}

function buildExportDiagnostics(caught: unknown): ExportDiagnostics {
  const message = caught instanceof Error ? caught.message : String(caught);
  const issues = parseValidationPayload(message)?.issues ?? [];
  if (!issues.length) {
    return {
      title: message.includes("validation_failed") ? "导出前还有项目需要确认" : "导出没有完成",
      issues: [],
      guidance: message.includes("validation_failed") ? plainExportGuidance(message) : [message],
      canForce: message.includes("validation_failed")
    };
  }

  const answerEmpty = issues.filter((issue) => issue.path.includes("$.answerKey"));
  const sourceReview = issues.filter((issue) => issue.path.includes("$.sourceReview"));
  const verification = issues.filter((issue) => issue.path.includes(".verified") || issue.path.includes("$.audit.humanVerified") || issue.path.includes("$.job.status"));
  const cloud = issues.filter((issue) => issue.path.includes("cloudComparison"));
  const guidance = [
    answerEmpty.length ? `${answerEmpty.length} 道题未设置答案；仍可导出，之后可继续补充。` : "",
    sourceReview.length ? "源文件识别结果仍需确认；如果是图片页 PDF，请核对视觉补全内容后确认。" : "",
    verification.length ? "题组或题目还没有人工确认；可在题稿编辑页逐题确认，或确认当前题组。" : "",
    cloud.length ? "云端整卷对照发现差异；本地稿未被覆盖，请核对题型、填空位置和答案。" : ""
  ].filter(Boolean);

  return {
    title: "导出前还有项目需要确认",
    issues,
    guidance: guidance.length ? guidance : ["请根据下方校验项完成确认，或忽略内容检查继续导出。"],
    canForce: true
  };
}

export function ExportPage({
  jobId,
  jobs,
  refresh
}: {
  jobId?: string;
  jobs: ImportJob[];
  refresh: () => void;
}) {
  const [mode, setMode] = useState<ExportMode>(jobId ? "single-js" : "nas-library");
  const [singleResult, setSingleResult] = useState<ExportResult | undefined>();
  const [jsResult, setJsResult] = useState<JsExportResult | undefined>();
  const [nasResult, setNasResult] = useState<NasExportResult | undefined>();
  const [exportDir, setExportDir] = useState<string>("local://exports");
  const [selected, setSelected] = useState<string[]>(jobId ? [jobId] : []);
  const [error, setError] = useState<string | undefined>();
  const [diagnostics, setDiagnostics] = useState<ExportDiagnostics | undefined>();
  const [running, setRunning] = useState(false);

  // ---------- 写作导出状态 ----------
  const [writingJobs, setWritingJobs] = useState<WritingJob[]>([]);
  const [writingSelected, setWritingSelected] = useState<string[]>([]);
  const [writingResult, setWritingResult] = useState<WritingExportResult | undefined>();
  const [writingRunning, setWritingRunning] = useState(false);

  const exportable = useMemo(
    () => jobs.filter((job) => EXPORTABLE_STATUSES.has(job.status)),
    [jobs]
  );
  const focusedJobId = jobId ?? exportable[0]?.jobId ?? "";
  const canRunSingle = Boolean(focusedJobId);

  useEffect(() => {
    setMode(jobId ? "single-js" : "nas-library");
    setSelected(jobId ? [jobId] : []);
    setError(undefined);
    setSingleResult(undefined);
    setJsResult(undefined);
    setNasResult(undefined);
    setDiagnostics(undefined);
  }, [jobId]);

  useEffect(() => {
    if (jobId) {
      setSelected([jobId]);
    } else {
      setSelected((current) => {
        const allowed = new Set(exportable.map((job) => job.jobId));
        const next = current.filter((id) => allowed.has(id));
        if (next.length) return next;
        const ready = exportable.filter((job) => DEFAULT_NAS_STATUSES.has(job.status)).map((job) => job.jobId);
        return ready.length ? ready : exportable.map((job) => job.jobId);
      });
    }

  }, [jobId, exportable]);

  async function chooseDir() {
    const selectedDir = await chooseExportDirectory();
    if (selectedDir) setExportDir(selectedDir);
  }

  async function run(nextMode: ExportMode = mode, nextSelected: string[] = selected, validationPolicy: ValidationPolicy = "strict") {
    setRunning(true);
    setError(undefined);
    setSingleResult(undefined);
    setJsResult(undefined);
    setNasResult(undefined);
    setDiagnostics(undefined);
    try {
      if (nextMode === "full-assets") {
        if (!focusedJobId) throw new Error("no_export_focus_job");
        setSingleResult(await exportReadingAssets(focusedJobId, exportDir, validationPolicy));
      } else if (nextMode === "nas-library") {
        setNasResult(await exportNasLibrary({ jobIds: nextSelected, exportDir, validationPolicy }));
      } else if (nextMode === "single-js") {
        if (!focusedJobId) throw new Error("no_export_focus_job");
        setJsResult(await exportReadingJs({ jobIds: [focusedJobId], exportDir, validationPolicy }));
      } else {
        setJsResult(await exportReadingJs({ jobIds: nextSelected, exportDir, validationPolicy }));
      }
      refresh();
    } catch (caught) {
      const nextDiagnostics = buildExportDiagnostics(caught);
      setDiagnostics(nextDiagnostics);
      setError(nextDiagnostics.title);
    } finally {
      setRunning(false);
    }
  }

  useEffect(() => {
    if (jobId) return;
    const intent = takePublishIntent();
    if (!intent) return;
    setMode(intent.mode);
    const intentJobId = intent.mode === "nas-library" ? intent.jobId : undefined;
    if (intentJobId) {
      setSelected((current) => current.includes(intentJobId) ? current : [intentJobId, ...current]);
    }
  }, [jobId]);

  // 加载写作任务列表（写作导出模式用）
  const [writingRefreshTick, setWritingRefreshTick] = useState(0);
  useEffect(() => {
    void listWritingJobs().then((list) => {
      setWritingJobs(list);
      setWritingSelected((current) => {
        const exportableList = list.filter((j) => j.status === "ExportReady" || j.status === "Exported");
        const currentExportable = current.filter((id) => exportableList.some((job) => job.jobId === id));
        if (currentExportable.length) return currentExportable;
        const task1 = exportableList.find((j) => j.taskType === "task1");
        const task2 = exportableList.find((j) => j.taskType === "task2");
        return [task1?.jobId, task2?.jobId].filter(Boolean) as string[];
      });
    }).catch(() => setWritingJobs([]));
  }, [writingRefreshTick]);

  const writingExportable = useMemo(
    () => writingJobs.filter((j) => j.status === "ExportReady" || j.status === "Exported"),
    [writingJobs]
  );
  const selectedWritingJobs = writingSelected
    .map((id) => writingExportable.find((job) => job.jobId === id))
    .filter((job): job is WritingJob => Boolean(job));
  const canExportWriting = selectedWritingJobs.length === 2
    && new Set(selectedWritingJobs.map((job) => job.taskType)).size === 2;

  async function runWritingExport() {
    setWritingRunning(true);
    setError(undefined);
    setWritingResult(undefined);
    try {
      const result = await exportWritingLibrary({ jobIds: writingSelected, exportDir });
      setWritingResult(result);
      refresh();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setWritingRunning(false);
    }
  }

  const previewText =
    mode === "writing-library"
      ? writingResult
        ? `写作题库 ${writingResult.version}\n${writingResult.writingExamsDir}`
        : "尚未导出写作题库。"
      : mode === "full-assets"
      ? singleResult?.files.find((file) => file.name.endsWith(".js") && file.name !== "manifest.js")
          ?.content.slice(0, 900) ?? "尚未生成导出脚本。"
      : mode === "nas-library"
        ? nasResult?.files.find((file) => file.name.endsWith(".js") && file.name !== "manifest.js")
            ?.content.slice(0, 900) ?? "尚未生成 NAS 题库脚本。"
      : jsResult?.files.find((file) => file.name.endsWith(".js") && file.name !== "manifest.js")
          ?.content.slice(0, 900) ?? "尚未生成题目脚本。";

  const resultFiles =
    mode === "writing-library"
      ? undefined
      : mode === "full-assets"
      ? singleResult?.files
      : mode === "nas-library"
        ? nasResult?.files
        : jsResult?.files;

  const cleanupMessage =
    mode === "full-assets"
      ? singleResult?.cleanup?.message
      : mode === "nas-library"
        ? nasResult?.cleanup?.length
          ? "已完成 NAS 发布，并清理对应任务的过程文件。"
          : undefined
        : jsResult?.cleanup?.length
          ? "已完成导出，并清理对应任务的过程文件。"
          : undefined;

  const cleanupSuccess =
    mode === "full-assets"
      ? Boolean(singleResult?.cleanup?.cleaned)
      : mode === "nas-library"
        ? Boolean(nasResult?.cleanup?.every((item) => item.cleaned))
        : Boolean(jsResult?.cleanup?.every((item) => item.cleaned));

  const activeReadingResult = mode === "full-assets" ? singleResult : mode === "nas-library" ? nasResult : jsResult;
  const validationOverridden = activeReadingResult?.validationOverridden === true;
  const ignoredIssueCount = activeReadingResult?.ignoredIssueCount ?? 0;

  return (
    <section className="page-enter" data-testid="export-page">
      <div className="section-heading spread">
        <div>
          <p className="eyebrow">发布中心</p>
          <h2>{jobId ? "导出题目" : mode === "writing-library" ? "发布写作题库" : "发布到 NAS 题库"}</h2>
        </div>
        <div className="button-row">
          <button className="ghost" data-testid="choose-export-dir" onClick={chooseDir}>{mode === "nas-library" ? "选择 NAS 目录" : "选择导出目录"}</button>
          {mode !== "writing-library" ? (
            <button
              className="primary"
              data-testid="generate-export"
              disabled={running || (!canRunSingle && (mode === "single-js" || mode === "full-assets")) || ((mode === "batch-js" || mode === "nas-library") && !selected.length)}
              onClick={() => void run()}
            >
              {running
                ? "正在导出..."
                : mode === "full-assets"
                ? "导出完整文件"
                : mode === "batch-js"
                  ? "批量导出 JS"
                  : mode === "nas-library"
                    ? `发布 ${selected.length} 道题到 NAS`
                    : "导出当前题目 JS"}
            </button>
          ) : null}
        </div>
      </div>
      <div className="export-grid">
        <section className="form-section">
          <h3>{jobId ? "导出方式" : "发布方式"}</h3>
          {!jobId ? <p className="muted-inline">批量发布默认写入 NAS 题库目录；其他导出格式可按需切换。</p> : null}
          <div className="button-row">
            {jobId ? (
              <button
                className={mode === "single-js" ? "primary" : "ghost"}
                data-testid="mode-single-js"
                onClick={() => {
                  setMode("single-js");
                  setSelected([jobId]);
                }}
              >
                当前题目 JS
              </button>
            ) : null}
            {!jobId ? (
              <button
                className={mode === "nas-library" ? "primary" : "ghost"}
                data-testid="mode-nas-library"
                onClick={() => setMode("nas-library")}
              >
                NAS 题库
              </button>
            ) : null}
            <button
              className={mode === "batch-js" ? "primary" : "ghost"}
              data-testid="mode-batch-js"
              onClick={() => setMode("batch-js")}
            >
              批量导出 JS
            </button>
            {jobId ? (
              <button
                className={mode === "full-assets" ? "primary" : "ghost"}
                data-testid="mode-full-assets"
                onClick={() => setMode("full-assets")}
              >
                完整导出
              </button>
            ) : null}
            {jobId ? (
              <button
                className={mode === "nas-library" ? "primary" : "ghost"}
                data-testid="mode-nas-library"
                onClick={() => setMode("nas-library")}
              >
                NAS 题库
              </button>
            ) : null}
            <button
              className={mode === "writing-library" ? "primary" : "ghost"}
              data-testid="mode-writing-library"
              onClick={() => setMode("writing-library")}
            >
              写作题库
            </button>
          </div>
          <div className="path-picker">
            <span>
              <strong>导出目录</strong>
              <small>{exportDir}</small>
            </span>
          </div>
          {mode === "batch-js" || mode === "nas-library" ? (
            <>
              <div className="spread">
                <h3>{mode === "nas-library" ? `本次发布清单（${selected.length} 道）` : `选择题目（${selected.length} 道）`}</h3>
                <div className="button-row">
                  <button className="ghost small" type="button" onClick={() => setSelected(exportable.map((job) => job.jobId))}>全选</button>
                  <button className="ghost small" type="button" onClick={() => setSelected([])}>清空</button>
                </div>
              </div>
              {mode === "nas-library" ? <p className="muted-inline">发布会按当前清单生成 NAS 的 <code>manifest.js</code>，未勾选题目不会写入本次清单。</p> : null}
              {exportable.map((job) => (
                <label className="pick-row" key={job.jobId}>
                  <input
                    type="checkbox"
                    data-testid="export-job-checkbox"
                    checked={selected.includes(job.jobId)}
                    onChange={(event) =>
                      setSelected((current) =>
                        event.target.checked
                          ? current.includes(job.jobId)
                            ? current
                            : [...current, job.jobId]
                          : current.filter((id) => id !== job.jobId)
                      )
                    }
                  />
                  <span>{job.title}</span>
                  <StatusPill status={job.status} />
                </label>
              ))}
              {!exportable.length ? <p className="empty">当前没有可直接导出的题目。</p> : null}
            </>
          ) : null}
          {mode === "writing-library" ? (
            <>
              <h3>选择 Task 1 与 Task 2 写作任务</h3>
              <p className="muted-inline">写作题库需选一个 Task 1 + 一个 Task 2，且二者都需先在写作创作页标记为「可导出」。</p>
              {writingExportable.map((job) => (
                <label className="pick-row" key={job.jobId}>
                  <input
                    type="checkbox"
                    data-testid="writing-job-checkbox"
                    checked={writingSelected.includes(job.jobId)}
                    onChange={(event) =>
                      setWritingSelected((current) => {
                        if (!event.target.checked) return current.filter((id) => id !== job.jobId);
                        const withoutSameTask = current.filter((id) => {
                          const selectedJob = writingExportable.find((item) => item.jobId === id);
                          return selectedJob?.taskType !== job.taskType;
                        });
                        return [...withoutSameTask, job.jobId].slice(-2);
                      })
                    }
                  />
                  <span>{job.title}</span>
                  <small>{job.taskType} · {job.examId}</small>
                  <StatusPill status={mapWritingStatusToJobStatus(job.status)} />
                </label>
              ))}
              {!writingExportable.length ? (
                <p className="empty">暂无可导出的写作任务。请到「写作创作」页创建并标记为可导出。</p>
              ) : !canExportWriting ? (
                <p className="warning-box">需同时勾选一个 Task 1 和一个 Task 2。</p>
              ) : null}
              <p>
                <button
                  className="primary"
                  data-testid="export-writing-library"
                  disabled={writingRunning || !canExportWriting}
                  onClick={() => void runWritingExport()}
                >
                  {writingRunning ? "正在导出写作题库..." : "导出写作题库"}
                </button>
              </p>
              {writingResult ? (
                <div className="warning-box" data-testid="writing-export-result">
                  <strong>写作题库已导出</strong>
                  <p>版本：<code>{writingResult.version}</code></p>
                  <p>输出目录：<code>{writingResult.writingExamsDir}</code></p>
                  <p>文件：{writingResult.files.map((f) => f.name).join("、")}</p>
                  <small>学生端会直接读取这里的 <code>writing-exams</code> 子目录，不需要再手动搬运文件。</small>
                </div>
              ) : null}
            </>
          ) : null}
          {error ? (
            <div className="warning-box" data-testid="export-error">
              <strong>{error}</strong>
              {diagnostics?.guidance.map((item) => <p key={item}>{item}</p>)}
              {diagnostics?.issues.length ? <small>共 {diagnostics.issues.length} 个校验项；下方列出前 8 个。</small> : null}
              {diagnostics?.canForce ? (
                <div className="button-row">
                  <button className="danger" data-testid="force-export" disabled={running} onClick={() => void run(mode, selected, "force")}>忽略全部检查并继续导出</button>
                  <small>只忽略内容与审核门禁；文件、路径或数据无法生成时仍会停止。</small>
                </div>
              ) : null}
            </div>
          ) : null}
          {validationOverridden ? (
            <div className="warning-box" data-testid="export-override-result">
              <strong>已按要求忽略检查并完成导出</strong>
              <p>本次保留了 {ignoredIssueCount} 个校验项，后续仍可回到题稿继续补充。</p>
            </div>
          ) : null}
          {cleanupMessage ? (
            <p
              data-testid="cleanup-message"
              className={cleanupSuccess ? "success-text" : "warning-box"}
            >
              {cleanupMessage}
            </p>
          ) : null}
          {mode === "nas-library" && nasResult?.version ? (
            <div className="warning-box" data-testid="nas-export-result">
              <strong>NAS 版本</strong>
              <p>阅读题库已平铺写入所选目录，学生端直接选择这个目录即可读取 <code>manifest.js</code> 与题目脚本。</p>
              {nasResult.readingExamsDir ? <p><code>{nasResult.readingExamsDir}</code></p> : null}
              <code>{nasResult.version}</code>
            </div>
          ) : null}
        </section>
        <section className="form-section">
          <h3>输出文件</h3>
          {diagnostics?.issues.length ? (
            <div className="issue-list" data-testid="export-issue-list">
              {diagnostics.issues.slice(0, 8).map((issue) => {
                const display = validationIssueDisplay(issue);
                return (
                  <div key={issue.issueId}>
                    <strong>{display.title}</strong>
                    <small>{display.detail}</small>
                    <small>{display.action}</small>
                  </div>
                );
              })}
            </div>
          ) : null}
          {resultFiles?.map((file) => (
            <button className="file-line" data-testid="export-file" key={file.name}>
              {file.name}
              <span>{file.content.length} bytes</span>
            </button>
          )) ?? <p className="empty">尚未导出。</p>}
        </section>
        <aside className="inspector">
          <p className="eyebrow">导出预览</p>
          <h3>{mode === "batch-js" ? "批量题目脚本" : mode === "nas-library" ? "NAS 题库脚本" : "题目脚本"}</h3>
          <pre>{previewText}</pre>
        </aside>
      </div>

    </section>
  );
}

function mapWritingStatusToJobStatus(status: WritingJob["status"]): "Working" | "DraftSaved" | "ExportReady" | "Exported" {
  if (status === "Exported") return "Exported";
  if (status === "ExportReady") return "ExportReady";
  return "DraftSaved";
}
