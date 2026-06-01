import { useState } from "react";
import { chooseExportDirectory } from "../api/desktopDialogs";
import { exportReadingAssets } from "../api/tauriCommands";
import type { ExportResult } from "../types";

export function ExportPage({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [result, setResult] = useState<ExportResult | undefined>();
  const [exportDir, setExportDir] = useState<string>("local://exports");

  async function chooseDir() {
    const selected = await chooseExportDirectory();
    if (selected) setExportDir(selected);
  }

  async function run() {
    setResult(await exportReadingAssets(jobId, exportDir));
    refresh();
  }

  return (
    <section className="page-enter" data-testid="export-page">
      <div className="section-heading spread">
        <div><p className="eyebrow">导出</p><h2>导出单题文件和清单</h2></div>
        <div className="button-row"><button className="ghost" data-testid="choose-export-dir" onClick={chooseDir}>选择导出目录</button><button className="primary" data-testid="generate-export" onClick={run}>生成导出产物</button></div>
      </div>
      <div className="export-grid">
        <section className="form-section">
          <h3>输出文件</h3>
          <div className="path-picker"><span><strong>导出目录</strong><small>{exportDir}</small></span></div>
          {result?.cleanup?.message ? <p data-testid="cleanup-message" className={result.cleanup.cleaned ? "success-text" : "warning-box"}>{result.cleanup.message}</p> : null}
          {result?.files.map((file) => <button className="file-line" data-testid="export-file" key={file.name}>{file.name}<span>{file.content.length} bytes</span></button>) ?? <p className="empty">尚未导出。</p>}
        </section>
        <aside className="inspector"><p className="eyebrow">导出预览</p><h3>单题脚本</h3><pre>{result?.files.find((file) => file.name.endsWith(".js") && file.name !== "manifest.js")?.content.slice(0, 900) ?? "尚未生成导出脚本。"}</pre></aside>
      </div>
    </section>
  );
}
