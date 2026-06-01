import { useEffect, useState } from "react";
import { generatePreviewAssets, getJob, runPreviewE2e, validateAuthoringIr } from "../api/tauriCommands";
import { go } from "../app/router";
import type { PreviewAssets, ValidationReport } from "../types";

function buildSrcDoc(assets: PreviewAssets): string {
  return `<!doctype html><html><head><meta charset="utf-8"><style>body{font-family:Georgia,serif;margin:0;padding:24px;background:#f5f1e8;color:#17211f;line-height:1.6}.layout{display:grid;grid-template-columns:1fr 420px;gap:28px}.pane{background:#fffaf0;border:1px solid #d8cfbf;padding:22px}.choice-row{display:flex;gap:10px;flex-wrap:wrap}.completion-table{width:100%;border-collapse:collapse}.completion-table th,.completion-table td{border:1px solid #c8beaa;padding:8px}.question-umbrella-ranges{padding-left:18px;color:#5d4630}input{font:inherit;padding:6px}</style></head><body><div class="layout"><article class="pane">${assets.source.passage.blocks.map((block) => block.html).join("")}</article><section class="pane">${assets.source.meta.questionIntroHtml}${assets.source.questionGroups.map((group) => group.bodyHtml).join("")}</section></div></body></html>`;
}

export function UnifiedPreview({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [assets, setAssets] = useState<PreviewAssets | undefined>();
  const [report, setReport] = useState<ValidationReport | undefined>();
  const runtimeMode = report?.runtime?.mode;

  async function load() {
    const detail = await getJob(jobId);
    setAssets(detail.previewAssets);
    setReport(detail.validationReport);
  }

  useEffect(() => {
    load().catch(console.error);
  }, [jobId]);

  async function generate() {
    await validateAuthoringIr(jobId);
    const next = await generatePreviewAssets(jobId);
    setAssets(next);
    await load();
    refresh();
  }

  async function e2e() {
    const next = await runPreviewE2e(jobId);
    setReport(next);
    refresh();
  }

  return (
    <section className="page-enter" data-testid="unified-preview">
      <div className="section-heading spread">
        <div><p className="eyebrow">Unified Runtime Preview</p><h2>统一阅读页预览</h2></div>
        <div className="button-row"><button className="ghost" data-testid="generate-preview-assets" onClick={generate}>重新生成预览</button><button className="primary" data-testid="run-preview-e2e" disabled={!assets} onClick={e2e}>自动填正确答案 E2E</button><button className="ghost" data-testid="go-export" onClick={() => go(`/jobs/${jobId}/export`)}>导出</button></div>
      </div>
      <div className="warning-box">
        <strong>可视化窗口是隔离的本地模板预览。</strong>
        <p>
          导出/Pack 默认以 Rust 静态合同校验和人工审核门禁为准；右侧 E2E 是开发/诊断用真实运行时检查。当前运行时模式：
          <code data-testid="runtime-mode">{runtimeMode ?? "not-run"}</code>
          {runtimeMode === "real" ? "，真实统一阅读页已通过自动填答验证。" : "，未运行真实运行时诊断不阻断生产导出。"}
        </p>
      </div>
      <div className="preview-grid">
        <iframe title="reading-preview" data-testid="reading-preview-frame" sandbox="" srcDoc={assets ? buildSrcDoc(assets) : "<p>Generate preview assets first.</p>"} />
        <aside className="inspector">
          <p className="eyebrow">Validation</p>
          <h3>{report?.passed ? "通过" : "待校验"}</h3>
          <div className="layer-list">
            {report?.layers.map((layer) => {
              const warnings = layer.warningCount ?? 0;
              const errors = layer.errorCount ?? (layer.passed ? 0 : layer.issueCount);
              const label = errors > 0 ? `${errors} errors` : warnings > 0 ? `pass · ${warnings} warnings` : "pass";
              return <div key={layer.layer}><span>{layer.layer}</span><strong>{label}</strong></div>;
            })}
          </div>
          {report?.issues.length ? <details open><summary>issues</summary><pre>{JSON.stringify(report.issues, null, 2)}</pre></details> : null}
          <details open><summary>collectedAnswers</summary><pre>{JSON.stringify(report?.runtime?.collectedAnswers ?? {}, null, 2)}</pre></details>
          <details><summary>scoreInfo</summary><pre>{JSON.stringify({ scoreInfo: report?.runtime?.scoreInfo, wrongScoreInfo: report?.runtime?.wrongScoreInfo, navButtonCount: report?.runtime?.navButtonCount, questionCount: report?.runtime?.questionCount }, null, 2)}</pre></details>
          <details><summary>console errors</summary><pre>{JSON.stringify(report?.runtime?.consoleErrors ?? [], null, 2)}</pre></details>
          <details open><summary>answerKey</summary><pre>{JSON.stringify(assets?.source.answerKey ?? {}, null, 2)}</pre></details>
        </aside>
      </div>
    </section>
  );
}
