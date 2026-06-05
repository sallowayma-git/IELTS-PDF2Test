import { useEffect, useMemo, useState } from "react";
import { chooseExportDirectory } from "../api/desktopDialogs";
import { exportNasLibrary, exportReadingAssets, exportReadingJs } from "../api/tauriCommands";
import { StatusPill } from "../components/StatusPill";
import type { ExportResult, ImportJob, JsExportResult, NasExportResult } from "../types";

type ExportMode = "single-js" | "batch-js" | "full-assets" | "nas-library";

const EXPORTABLE_STATUSES = new Set(["ExportReady", "Exported", "Cleaned"]);

export function ExportPage({
  jobId,
  jobs,
  refresh
}: {
  jobId: string;
  jobs: ImportJob[];
  refresh: () => void;
}) {
  const [mode, setMode] = useState<ExportMode>("single-js");
  const [singleResult, setSingleResult] = useState<ExportResult | undefined>();
  const [jsResult, setJsResult] = useState<JsExportResult | undefined>();
  const [nasResult, setNasResult] = useState<NasExportResult | undefined>();
  const [exportDir, setExportDir] = useState<string>("local://exports");
  const [selected, setSelected] = useState<string[]>([jobId]);
  const [error, setError] = useState<string | undefined>();

  const exportable = useMemo(
    () => jobs.filter((job) => EXPORTABLE_STATUSES.has(job.status)),
    [jobs]
  );

  useEffect(() => {
    setSelected([jobId]);
    setError(undefined);
    setSingleResult(undefined);
    setJsResult(undefined);
    setNasResult(undefined);
  }, [jobId]);

  async function chooseDir() {
    const selectedDir = await chooseExportDirectory();
    if (selectedDir) setExportDir(selectedDir);
  }

  async function run() {
    setError(undefined);
    setSingleResult(undefined);
    setJsResult(undefined);
    setNasResult(undefined);
    try {
      if (mode === "full-assets") {
        setSingleResult(await exportReadingAssets(jobId, exportDir));
      } else if (mode === "nas-library") {
        setNasResult(await exportNasLibrary({ jobIds: selected, exportDir }));
      } else if (mode === "single-js") {
        setJsResult(await exportReadingJs({ jobIds: [jobId], exportDir }));
      } else {
        setJsResult(await exportReadingJs({ jobIds: selected, exportDir }));
      }
      refresh();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }

  const previewText =
    mode === "full-assets"
      ? singleResult?.files.find((file) => file.name.endsWith(".js") && file.name !== "manifest.js")
          ?.content.slice(0, 900) ?? "尚未生成导出脚本。"
      : mode === "nas-library"
        ? nasResult?.files.find((file) => file.name.startsWith("source/") && file.name.endsWith(".js"))
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
          <p className="eyebrow">导出</p>
          <h2>导出题目 JS 文件</h2>
        </div>
        <div className="button-row">
          <button className="ghost" data-testid="choose-export-dir" onClick={chooseDir}>选择导出目录</button>
          <button
            className="primary"
            data-testid="generate-export"
            disabled={(mode === "batch-js" || mode === "nas-library") && !selected.length}
            onClick={run}
          >
            {mode === "full-assets"
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
          <div className="button-row">
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
            <button
              className={mode === "batch-js" ? "primary" : "ghost"}
              data-testid="mode-batch-js"
              onClick={() => setMode("batch-js")}
            >
              批量导出 JS
            </button>
            <button
              className={mode === "full-assets" ? "primary" : "ghost"}
              data-testid="mode-full-assets"
              onClick={() => setMode("full-assets")}
            >
              完整导出
            </button>
            <button
              className={mode === "nas-library" ? "primary" : "ghost"}
              data-testid="mode-nas-library"
              onClick={() => setMode("nas-library")}
            >
              NAS 题库
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
          {error ? <p className="warning-box" data-testid="export-error">{error}</p> : null}
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
              <p>将题目脚本写入 `source/`，并重建 `publish/library.db`、`library.version.json`、`library.db.sha256` 与 `report.json`。</p>
              <code>{nasResult.version}</code>
            </div>
          ) : null}
        </section>
        <section className="form-section">
          <h3>输出文件</h3>
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
