import { useEffect, useMemo, useState } from "react";
import { chooseExportDirectory } from "../api/desktopDialogs";
import { buildPack, exportNasLibrary, exportReadingAssets, exportReadingJs, exportWritingLibrary, listWritingJobs } from "../api/tauriCommands";
import { StatusPill } from "../components/StatusPill";
import type { ExportResult, ImportJob, JsExportResult, NasExportResult, PackBuildResult, ValidationIssue, WritingExportResult, WritingJob } from "../types";
import { validationIssueDisplay } from "../utils/displayLabels";

type ExportMode = "single-js" | "batch-js" | "full-assets" | "nas-library" | "writing-library";

const EXPORTABLE_STATUSES = new Set(["DraftSaved", "ExportReady", "Exported", "Cleaned"]);
const PACKABLE_STATUSES = new Set(["ExportReady", "Exported", "Cleaned"]);
const EXPORT_INTENT_KEY_PREFIX = "ielts-author-studio.export-intent.";

interface ExportDiagnostics {
  title: string;
  issues: ValidationIssue[];
  guidance: string[];
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
    /answer is empty|答案为空|答案不能为空/i.test(message) ? "仍有题目缺少答案，请在题稿编辑页补齐后再导出。" : "",
    /human verified|人工确认|NeedsReview|待审核/i.test(message) ? "题目或题组还没有完成确认，请在题稿编辑页完成确认。" : "",
    /source review|parser warning|low-confidence parsed block|源文档/i.test(message) ? "源文档识别结果仍需确认，请回到源文档审核页处理。" : "",
    /cloudComparison|云端/i.test(message) ? "云端整卷对照仍有差异，请在题稿编辑页核对云端/本地差异。" : ""
  ].filter(Boolean);
  return guidance.length ? guidance : ["导出前检查没有通过，请回到题稿编辑页完成缺失答案和人工确认。"];
}

function buildExportDiagnostics(caught: unknown): ExportDiagnostics {
  const message = caught instanceof Error ? caught.message : String(caught);
  const issues = parseValidationPayload(message)?.issues ?? [];
  if (!issues.length) {
    return {
      title: message.includes("validation_failed") ? "导出前还有项目需要确认" : "导出没有完成",
      issues: [],
      guidance: message.includes("validation_failed") ? plainExportGuidance(message) : [message]
    };
  }

  const answerEmpty = issues.filter((issue) => issue.path.includes("$.answerKey"));
  const sourceReview = issues.filter((issue) => issue.path.includes("$.sourceReview"));
  const verification = issues.filter((issue) => issue.path.includes(".verified") || issue.path.includes("$.audit.humanVerified") || issue.path.includes("$.job.status"));
  const cloud = issues.filter((issue) => issue.path.includes("cloudComparison"));
  const guidance = [
    answerEmpty.length ? `${answerEmpty.length} 道题仍缺少答案，请在题稿编辑页补齐。` : "",
    sourceReview.length ? "源文件识别结果仍需确认；如果是图片页 PDF，请核对视觉补全内容后确认。" : "",
    verification.length ? "题组或题目还没有人工确认；可在题稿编辑页逐题确认，或确认当前题组。" : "",
    cloud.length ? "云端整卷对照发现差异；本地稿未被覆盖，请核对题型、填空位置和答案。" : ""
  ].filter(Boolean);

  return {
    title: "导出前还有项目需要确认",
    issues,
    guidance: guidance.length ? guidance : ["请根据下方校验项完成确认后再次导出。"]
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
  const [mode, setMode] = useState<ExportMode>(jobId ? "single-js" : "batch-js");
  const [singleResult, setSingleResult] = useState<ExportResult | undefined>();
  const [jsResult, setJsResult] = useState<JsExportResult | undefined>();
  const [nasResult, setNasResult] = useState<NasExportResult | undefined>();
  const [exportDir, setExportDir] = useState<string>("local://exports");
  const [selected, setSelected] = useState<string[]>(jobId ? [jobId] : []);
  const [packSelected, setPackSelected] = useState<string[]>([]);
  const [packId, setPackId] = useState("pack-20260617-basic");
  const [packVersion, setPackVersion] = useState("0.1.0");
  const [packInstitution, setPackInstitution] = useState("internal");
  const [packDescription, setPackDescription] = useState("IELTS Author Studio generated pack");
  const [packValidFrom, setPackValidFrom] = useState("");
  const [packValidTo, setPackValidTo] = useState("");
  const [packResult, setPackResult] = useState<PackBuildResult | undefined>();
  const [packError, setPackError] = useState<string | undefined>();
  const [error, setError] = useState<string | undefined>();
  const [diagnostics, setDiagnostics] = useState<ExportDiagnostics | undefined>();
  const [running, setRunning] = useState(false);
  const [packRunning, setPackRunning] = useState(false);

  // ---------- 写作导出状态 ----------
  const [writingJobs, setWritingJobs] = useState<WritingJob[]>([]);
  const [writingSelected, setWritingSelected] = useState<string[]>([]);
  const [writingResult, setWritingResult] = useState<WritingExportResult | undefined>();
  const [writingRunning, setWritingRunning] = useState(false);

  const exportable = useMemo(
    () => jobs.filter((job) => EXPORTABLE_STATUSES.has(job.status)),
    [jobs]
  );
  const packable = useMemo(
    () => jobs.filter((job) => PACKABLE_STATUSES.has(job.status)),
    [jobs]
  );
  const focusedJobId = jobId ?? exportable[0]?.jobId ?? "";
  const canRunSingle = Boolean(focusedJobId);

  useEffect(() => {
    setMode(jobId ? "single-js" : "batch-js");
    setSelected(jobId ? [jobId] : []);
    setPackSelected([]);
    setError(undefined);
    setSingleResult(undefined);
    setJsResult(undefined);
    setNasResult(undefined);
    setDiagnostics(undefined);
    setPackResult(undefined);
    setPackError(undefined);
  }, [jobId]);

  useEffect(() => {
    if (jobId) {
      setSelected([jobId]);
    } else {
      setSelected((current) => {
        const allowed = new Set(exportable.map((job) => job.jobId));
        const next = current.filter((id) => allowed.has(id));
        return next.length ? next : exportable.map((job) => job.jobId).slice(0, 1);
      });
    }

    setPackSelected((current) => {
      const allowed = new Set(packable.map((job) => job.jobId));
      const next = current.filter((id) => allowed.has(id));
      if (next.length) return next;
      if (jobId && allowed.has(jobId)) return [jobId];
      return [];
    });
  }, [jobId, exportable, packable]);

  async function chooseDir() {
    const selectedDir = await chooseExportDirectory();
    if (selectedDir) setExportDir(selectedDir);
  }

  async function run(nextMode: ExportMode = mode, nextSelected: string[] = selected) {
    setRunning(true);
    setError(undefined);
    setSingleResult(undefined);
    setJsResult(undefined);
    setNasResult(undefined);
    setDiagnostics(undefined);
    try {
      if (nextMode === "full-assets") {
        if (!focusedJobId) throw new Error("no_export_focus_job");
        setSingleResult(await exportReadingAssets(focusedJobId, exportDir));
      } else if (nextMode === "nas-library") {
        setNasResult(await exportNasLibrary({ jobIds: nextSelected, exportDir }));
      } else if (nextMode === "single-js") {
        if (!focusedJobId) throw new Error("no_export_focus_job");
        setJsResult(await exportReadingJs({ jobIds: [focusedJobId], exportDir }));
      } else {
        setJsResult(await exportReadingJs({ jobIds: nextSelected, exportDir }));
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

  async function runPack() {
    setPackRunning(true);
    setPackError(undefined);
    try {
      setPackResult(await buildPack({
        packId,
        version: packVersion,
        institution: packInstitution,
        description: packDescription,
        validFrom: packValidFrom || undefined,
        validTo: packValidTo || undefined,
        jobIds: packSelected
      }));
      refresh();
    } catch (caught) {
      setPackError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setPackRunning(false);
    }
  }

  useEffect(() => {
    if (!jobId) return;
    const key = `${EXPORT_INTENT_KEY_PREFIX}${jobId}`;
    const intent = window.sessionStorage.getItem(key);
    if (intent !== "single-js") return;
    window.sessionStorage.removeItem(key);
    setMode("single-js");
    setSelected([jobId]);
    void run("single-js", [jobId]);
  }, [jobId]);

  // 加载写作任务列表（写作导出模式用）
  const [writingRefreshTick, setWritingRefreshTick] = useState(0);
  useEffect(() => {
    void listWritingJobs().then((list) => {
      setWritingJobs(list);
      setWritingSelected((current) => {
        if (current.length) return current;
        // 默认预选一个 task1 + 一个 task2
        const task1 = list.find((j) => j.taskType === "task1");
        const task2 = list.find((j) => j.taskType === "task2");
        return [task1?.jobId, task2?.jobId].filter(Boolean) as string[];
      });
    }).catch(() => setWritingJobs([]));
  }, [writingRefreshTick]);

  const writingExportable = useMemo(
    () => writingJobs.filter((j) => j.status === "ExportReady" || j.status === "Exported"),
    [writingJobs]
  );
  const writingTask1 = writingExportable.find((j) => j.taskType === "task1");
  const writingTask2 = writingExportable.find((j) => j.taskType === "task2");
  const canExportWriting = Boolean(writingTask1) && Boolean(writingTask2)
    && writingSelected.length === 2
    && new Set(writingSelected).size === 2;

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
    mode === "full-assets"
      ? singleResult?.files.find((file) => file.name.endsWith(".js") && file.name !== "manifest.js")
          ?.content.slice(0, 900) ?? "尚未生成导出脚本。"
      : mode === "nas-library"
        ? nasResult?.files.find((file) => file.name.endsWith(".js") && file.name !== "manifest.js")
            ?.content.slice(0, 900) ?? "尚未生成 NAS 题库脚本。"
      : jsResult?.files.find((file) => file.name.endsWith(".js") && file.name !== "manifest.js")
          ?.content.slice(0, 900) ?? "尚未生成题目脚本。";

  const resultFiles =
    mode === "full-assets"
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

  return (
    <section className="page-enter" data-testid="export-page">
      <div className="section-heading spread">
        <div>
          <p className="eyebrow">发布</p>
          <h2>导出与组卷</h2>
        </div>
        <div className="button-row">
          <button className="ghost" data-testid="choose-export-dir" onClick={chooseDir}>选择导出目录</button>
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
                  ? "发布 NAS 题库"
                  : "导出当前题目 JS"}
          </button>
        </div>
      </div>
      <div className="export-grid">
        <section className="form-section">
          <h3>导出方式</h3>
          {!jobId ? <p className="muted-inline">当前页面已经合并了批量导出和 Pack 组卷。单题导出仍可从具体题目进入。</p> : null}
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
            <button
              className={mode === "nas-library" ? "primary" : "ghost"}
              data-testid="mode-nas-library"
              onClick={() => setMode("nas-library")}
            >
              NAS 题库
            </button>
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
              <h3>{mode === "nas-library" ? "选择纳入题库的题目" : "选择题目"}</h3>
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
                        const next = event.target.checked
                          ? (current.includes(job.jobId) ? current : [...current, job.jobId])
                          : current.filter((id) => id !== job.jobId);
                        // 最多选两个，且应覆盖 task1+task2
                        return next.slice(-2);
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
                  <small>把 {writingResult.writingExamsDir} 下的 manifest.js + task1.js + task2.js 放进 NAS 学生端 publish/assets/generated/writing-exams/ 即可被识别。</small>
                </div>
              ) : null}
            </>
          ) : null}
          {error ? (
            <div className="warning-box" data-testid="export-error">
              <strong>{error}</strong>
              {diagnostics?.guidance.map((item) => <p key={item}>{item}</p>)}
              {diagnostics?.issues.length ? <small>共 {diagnostics.issues.length} 个校验项；下方列出前 8 个。</small> : null}
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
            <div className="warning-box">
              <strong>NAS 版本</strong>
              <p>将 manifest 与题目脚本直接写入所选数据目录，供 NAS 学生端直接消费。</p>
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

      <div className="pack-grid merged-pack-grid" data-testid="pack-builder">
        <section className="form-section">
          <div className="spread">
            <h3>发布包题目</h3>
            <button className="primary" data-testid="build-pack" disabled={packRunning || !packSelected.length} onClick={() => void runPack()}>
              {packRunning ? "正在生成..." : "生成发布包"}
            </button>
          </div>
          <p className="muted-inline">Pack 已并入当前发布中心，用于一次打包多份已发布就绪的题目。</p>
          {packable.map((job) => (
            <label className="pick-row" key={job.jobId}>
              <input
                type="checkbox"
                data-testid="pack-job-checkbox"
                checked={packSelected.includes(job.jobId)}
                onChange={(event) =>
                  setPackSelected((current) =>
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
          {!packable.length ? <p className="empty">当前没有可打包的题目。</p> : null}
        </section>
        <section className="form-section contrast">
          <h3>发布包设置</h3>
          <label>packId<input value={packId} onChange={(event) => setPackId(event.target.value)} /></label>
          <label>version<input value={packVersion} onChange={(event) => setPackVersion(event.target.value)} /></label>
          <label>institution<input value={packInstitution} onChange={(event) => setPackInstitution(event.target.value)} /></label>
          <label>validFrom<input type="date" value={packValidFrom} onChange={(event) => setPackValidFrom(event.target.value)} /></label>
          <label>validTo<input type="date" value={packValidTo} onChange={(event) => setPackValidTo(event.target.value)} /></label>
          <label>description<textarea value={packDescription} onChange={(event) => setPackDescription(event.target.value)} /></label>
        </section>
        <aside className="inspector">
          <p className="eyebrow">发布包结果</p>
          <h3>{packResult?.packId ?? "未生成"}</h3>
          {packError ? <p className="error-text" data-testid="pack-error">{packError}</p> : null}
          {packResult ? (
            <dl data-testid="pack-result">
              <dt>输出路径</dt><dd>{packResult.outputPath}</dd>
              <dt>文件数量</dt><dd>{packResult.files.length}</dd>
              <dt>压缩包大小</dt><dd>{packResult.zipSizeBytes ?? "未记录"}</dd>
            </dl>
          ) : <p className="empty" data-testid="pack-result">尚未生成发布包。</p>}
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
