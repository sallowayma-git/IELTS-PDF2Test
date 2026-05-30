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
    <section className="page-enter">
      <div className="section-heading spread">
        <div><p className="eyebrow">Export</p><h2>导出 JS / manifest</h2></div>
        <div className="button-row"><button className="ghost" onClick={chooseDir}>选择导出目录</button><button className="primary" onClick={run}>生成导出产物</button></div>
      </div>
      <div className="export-grid">
        <section className="form-section">
          <h3>输出文件</h3>
          <div className="path-picker"><span><strong>导出目录</strong><small>{exportDir}</small></span></div>
          {result?.files.map((file) => <button className="file-line" key={file.name}>{file.name}<span>{file.content.length} bytes</span></button>) ?? <p className="empty">尚未导出。</p>}
        </section>
        <aside className="inspector"><p className="eyebrow">Contract</p><h3>单题 JS 包装</h3><pre>{result?.files.find((file) => file.name.endsWith(".js") && file.name !== "manifest.js")?.content.slice(0, 900) ?? "No wrapper generated."}</pre></aside>
      </div>
    </section>
  );
}
