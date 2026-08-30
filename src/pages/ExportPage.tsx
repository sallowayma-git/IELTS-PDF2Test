import { useEffect, useMemo, useState } from "react";
import { chooseExportDirectory } from "../api/desktopDialogs";
import { exportAuthoringV2, exportNasLibrary, exportNasPackageV2, exportWritingLibrary, getAuthoringV2, listWritingJobs } from "../api/tauriCommands";
import { StatusPill } from "../components/StatusPill";
import type { ImportJob, NasExportResult, NasPackageV2PublishResult, ValidationIssue, ValidationPolicy, WritingExportResult, WritingJob } from "../types";
import { validationIssueDisplay } from "../utils/displayLabels";
import { takePublishIntent } from "../utils/publishIntent";

type ExportMode = "nas-library" | "writing-library";

const EXPORTABLE_STATUSES = new Set(["Working", "NeedsReview", "DraftSaved", "ExportReady", "Exported", "Cleaned"]);
const DEFAULT_NAS_STATUSES = new Set(["DraftSaved", "ExportReady", "Exported", "Cleaned"]);
const NAS_EXPORT_DIR_KEY = "ielts-author-studio.confirmed-nas-export-dir.v1";
const NAS_PACKAGE_V2_ENABLED = true;

function isLocalPlaceholder(path: string): boolean {
  return path.trim().toLowerCase().startsWith("local://");
}

function restoredNasDirectory(): string {
  const stored = window.localStorage.getItem(NAS_EXPORT_DIR_KEY)?.trim() ?? "";
  return stored && !isLocalPlaceholder(stored) ? stored : "";
}

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
  const [mode, setMode] = useState<ExportMode>("nas-library");
  const [nasResult, setNasResult] = useState<NasExportResult | undefined>();
  const [nasPackageResult, setNasPackageResult] = useState<NasPackageV2PublishResult | undefined>();
  const [exportDir, setExportDir] = useState<string>(restoredNasDirectory);
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
  const hasConfirmedExportDir = Boolean(exportDir.trim()) && !isLocalPlaceholder(exportDir);

  useEffect(() => {
    setMode("nas-library");
    setSelected(jobId ? [jobId] : []);
    setError(undefined);
    setNasResult(undefined);
    setNasPackageResult(undefined);
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
    if (!selectedDir) return;
    if (isLocalPlaceholder(selectedDir)) {
      const title = "当前环境无法选择真实 NAS 目录";
      setError(title);
      setDiagnostics({
        title,
        issues: [],
        guidance: ["请在桌面应用中选择 NAS 挂载目录或共享盘目录后再发布。"],
        canForce: false
      });
      return;
    }
    setExportDir(selectedDir);
    window.localStorage.setItem(NAS_EXPORT_DIR_KEY, selectedDir);
    setError(undefined);
    setDiagnostics(undefined);
  }

  function switchMode(nextMode: ExportMode) {
    setMode(nextMode);
    setError(undefined);
    setDiagnostics(undefined);
  }

  async function run(validationPolicy: ValidationPolicy = "strict") {
    if (!hasConfirmedExportDir) return;
    setRunning(true);
    setError(undefined);
    setNasResult(undefined);
    setNasPackageResult(undefined);
    setDiagnostics(undefined);
    try {
      if (NAS_PACKAGE_V2_ENABLED) {
        if (selected.length !== 1) throw new Error("nas_package_v2_requires_single_authoring_job");
        const session = await getAuthoringV2(selected[0]);
        const materialized = await exportAuthoringV2({ jobId: selected[0], exportDir, revision: session.revision });
        setNasPackageResult(await exportNasPackageV2({
          libraryRoot: exportDir,
          sourcePath: materialized.receipt.runtimePath,
          assetRoot: materialized.receipt.outputDir,
          examId: materialized.examId,
          minimumRuntimeVersion: "0.2.0"
        }));
      } else {
        setNasResult(await exportNasLibrary({ jobIds: selected, exportDir, validationPolicy }));
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
    if (!hasConfirmedExportDir) return;
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
      : nasResult?.files.find((file) => file.name.endsWith(".js") && file.name !== "manifest.js")
          ?.content.slice(0, 900) ?? "尚未生成 NAS 题库脚本。";

  const resultFiles = mode === "writing-library" ? writingResult?.files : nasResult?.files;

  const cleanupMessage = mode === "nas-library" && nasResult?.cleanup?.length
    ? "已完成 NAS 发布，并清理对应任务的过程文件。"
    : undefined;

  const cleanupSuccess = Boolean(nasResult?.cleanup?.every((item) => item.cleaned));

  const activeReadingResult = mode === "nas-library" ? nasResult : undefined;
  const validationOverridden = activeReadingResult?.validationOverridden === true;
  const ignoredIssueCount = activeReadingResult?.ignoredIssueCount ?? 0;
  const uiBusy = running || writingRunning;

  return (
    <section className="page-enter" data-testid="export-page">
      <div className="section-heading spread">
        <div>
          <p className="eyebrow">NAS 导出</p>
          <h2>{mode === "writing-library" ? "发布写作题库" : "发布阅读题库到 NAS"}</h2>
        </div>
        <div className="button-row">
          <button className="ghost" data-testid="choose-export-dir" disabled={uiBusy} onClick={chooseDir}>选择 NAS 目录</button>
          {mode !== "writing-library" ? (
            <>
              <button
                className="primary"
                data-testid="generate-export"
                disabled={running || !selected.length || !hasConfirmedExportDir}
                onClick={() => void run("strict")}
              >
                {running ? "正在发布..." : NAS_PACKAGE_V2_ENABLED ? "发布 Reading V2 包到 NAS" : `发布 ${selected.length} 道题到 NAS`}
              </button>
              {!NAS_PACKAGE_V2_ENABLED ? (
                <button
                  className="ghost"
                  data-testid="force-export"
                  disabled={running || !selected.length || !hasConfirmedExportDir}
                  onClick={() => void run("force")}
                >
                  忽略内容检查并发布
                </button>
              ) : null}
            </>
          ) : null}
        </div>
      </div>
      <div className="export-grid">
        <section className="form-section">
          <h3>发布内容</h3>
          <p className="muted-inline">
            {mode === "nas-library"
              ? "阅读题统一发布到 NAS 题库目录；再次发布会更新同名题目，并保留目录中未选中的旧题。"
              : "写作题库会发布到同一 NAS 根目录下的 writing-exams 子目录。"}
          </p>
          {mode === "nas-library" && !NAS_PACKAGE_V2_ENABLED ? <p className="muted-inline">“忽略内容检查并发布”只跳过题稿与审核门禁；目录、写入和数据生成错误仍会停止。</p> : null}
          <div className="button-row">
            <button
              className={mode === "nas-library" ? "primary" : "ghost"}
              data-testid="mode-nas-library"
              disabled={uiBusy}
              onClick={() => switchMode("nas-library")}
            >
              阅读题库
            </button>
            {!jobId ? (
              <button
                className={mode === "writing-library" ? "primary" : "ghost"}
                data-testid="mode-writing-library"
                disabled={uiBusy}
                onClick={() => switchMode("writing-library")}
              >
                写作题库
              </button>
            ) : null}
          </div>
          <div className="path-picker">
            <span>
              <strong>NAS 题库目录</strong>
              <small>{hasConfirmedExportDir ? exportDir : "尚未选择；发布前必须选择学生端可访问的共享目录"}</small>
            </span>
          </div>
          {mode === "nas-library" ? (
            <>
              <div className="spread">
                <h3>{`本次更新（${selected.length} 道）`}</h3>
                <div className="button-row">
                  <button className="ghost small" type="button" disabled={uiBusy} onClick={() => setSelected(exportable.map((job) => job.jobId))}>全选</button>
                  <button className="ghost small" type="button" disabled={uiBusy} onClick={() => setSelected([])}>清空</button>
                </div>
              </div>
              <p className="muted-inline">未勾选题目不会被修改，也不会从现有 <code>manifest.js</code> 中移除。</p>
              {exportable.map((job) => (
                <label className="pick-row" key={job.jobId}>
                  <input
                    type="checkbox"
                    data-testid="export-job-checkbox"
                    disabled={uiBusy}
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
                    disabled={uiBusy}
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
                  disabled={writingRunning || !canExportWriting || !hasConfirmedExportDir}
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
              {diagnostics?.canForce ? <small>可使用页面顶部的“忽略内容检查并发布”；文件、路径或数据无法生成时仍会停止。</small> : null}
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
          {mode === "nas-library" && nasPackageResult ? (
            <div className="warning-box" data-testid="nas-package-v2-export-result">
              <strong>Reading V2 NAS 发布完成</strong>
              <p>exam：<code>{nasPackageResult.examId}</code> · assets：{nasPackageResult.assetCount}</p>
              <p>student probe：{nasPackageResult.probe.passed ? "通过" : "失败"}</p>
              <small>manifest：<code>{nasPackageResult.manifestPath}</code><br />report：<code>{nasPackageResult.reportPath}</code></small>
            </div>
          ) : null}
          {mode === "nas-library" && nasResult?.version ? (
            <div className="warning-box" data-testid="nas-export-result">
              <strong>NAS 版本</strong>
              <p>阅读题库已更新到所选共享目录；学生端可从这里读取 <code>manifest.js</code> 与题目脚本。</p>
              <p>本次更新 {nasResult.assetCount} 道，当前目录共 {nasResult.manifestAssetCount ?? nasResult.assetCount} 道。</p>
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
          <h3>{mode === "nas-library" ? "NAS 题库脚本" : "写作题库"}</h3>
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
