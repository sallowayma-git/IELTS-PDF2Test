import { useEffect, useState } from "react";
import { applyManualTranscription, applyVisionTranscription, getJob, rerunOcr, resolveSourceReview, runRuleSplit } from "../api/tauriCommands";
import { go } from "../app/router";
import type { AutoPipelineReport, DocumentBlock, DocumentIr, ImportJob, SourceReview } from "../types";

export function DocumentReview({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [job, setJob] = useState<ImportJob | null>(null);
  const [documentIr, setDocumentIr] = useState<DocumentIr | undefined>();
  const [sourceReview, setSourceReview] = useState<SourceReview | undefined>();
  const [pipelineReport, setPipelineReport] = useState<AutoPipelineReport | undefined>();
  const [selected, setSelected] = useState<string | undefined>();
  const [manualText, setManualText] = useState("");
  const [manualNote, setManualNote] = useState("");
  const [visionBusy, setVisionBusy] = useState(false);
  const [visionError, setVisionError] = useState<string | undefined>();

  async function load() {
    const detail = await getJob(jobId);
    setJob(detail.job);
    setDocumentIr(detail.documentIr);
    setSourceReview(detail.sourceReview);
    setPipelineReport(detail.pipelineReport);
  }

  useEffect(() => {
    load().catch(console.error);
  }, [jobId]);

  async function split() {
    await runRuleSplit(jobId);
    refresh();
    go(`/jobs/${jobId}/split`);
  }

  async function ocr() {
    await rerunOcr(jobId, [1]);
    await load();
    refresh();
  }

  async function resolveReview() {
    await resolveSourceReview(jobId, "人工确认源文档解析风险已处理");
    await load();
    refresh();
  }

  async function applyTranscription() {
    await applyManualTranscription(jobId, { text: manualText, note: manualNote || undefined });
    setManualText("");
    await load();
    refresh();
  }

  async function visionTranscribe() {
    setVisionBusy(true);
    setVisionError(undefined);
    try {
      await applyVisionTranscription(jobId, { note: "视觉大模型转录；需人工逐页核验" });
      await load();
      refresh();
    } catch (caught) {
      setVisionError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setVisionBusy(false);
    }
  }

  const blocks = documentIr?.pages.flatMap((page) => page.blocks.map((block) => ({ ...block, pageIndex: page.pageIndex }))) ?? [];
  const selectedBlock = blocks.find((block) => block.blockId === selected) ?? blocks[0];
  const lowConfidenceBlocks = blocks.filter((block: DocumentBlock) => block.confidence < 0.5);
  const parserWarnings = documentIr?.parser.warnings ?? [];

  return (
    <section className="document-review page-enter" data-testid="document-review">
      <div className="section-heading spread">
        <div>
          <p className="eyebrow">Document IR</p>
          <h2>文档解析预览</h2>
        </div>
        <div className="button-row"><button className="ghost" data-testid="rerun-ocr" onClick={ocr}>重新 OCR</button><button className="ghost" data-testid="vision-transcribe" disabled={visionBusy} onClick={visionTranscribe}>{visionBusy ? "视觉转录中..." : "视觉 LLM 转录"}</button><button className="ghost" data-testid="resolve-source-review" disabled={!sourceReview?.required || sourceReview.resolved} onClick={resolveReview}>确认源文档审核</button><button className="primary" data-testid="go-split" onClick={split}>进入粗切</button></div>
      </div>
      <div className="review-grid">
        <aside className="document-canvas">
          <div className="paper-page">
            {blocks.map((block) => (
              <button key={block.blockId} className={`bbox ${selected === block.blockId ? "active" : ""} role-${block.roleHint ?? "none"}`} onClick={() => setSelected(block.blockId)}>
                <span>{block.blockId}</span>
              </button>
            ))}
          </div>
        </aside>
        <section className="block-list">
          {blocks.map((block) => (
            <button key={block.blockId} className={selected === block.blockId ? "block-card active" : "block-card"} onClick={() => setSelected(block.blockId)}>
              <span>{block.blockId} · {block.blockType} · {Math.round(block.confidence * 100)}%</span>
              <strong>{block.roleHint ?? "unmarked"}</strong>
              <p>{block.text}</p>
            </button>
          ))}
        </section>
        <aside className="inspector">
          <p className="eyebrow">Block Inspector</p>
          <h3>{selectedBlock?.blockId ?? "No block"}</h3>
          <dl>
            <dt>job</dt><dd>{job?.jobId}</dd>
            <dt>role</dt><dd>{selectedBlock?.roleHint ?? "none"}</dd>
            <dt>confidence</dt><dd>{selectedBlock ? Math.round(selectedBlock.confidence * 100) : 0}%</dd>
            <dt>source review</dt><dd data-testid="source-review-status">{sourceReview?.required ? (sourceReview.resolved ? "resolved" : "required") : "not required"}</dd>
          </dl>
          {sourceReview?.required ? <><h4>Source Review Gate</h4><pre data-testid="source-review-json">{JSON.stringify(sourceReview, null, 2)}</pre></> : null}
          <details open={Boolean(sourceReview?.required || lowConfidenceBlocks.length || parserWarnings.length)}>
            <summary>扫描 PDF / OCR 失败处理</summary>
            <p>图片型 PDF 默认优先尝试视觉大模型转录，生成 `vision-llm-transcription` DocumentIR；该结果仍会保留源文档审核门禁，必须人工确认后才能发布。若模型不可用或页面图片无法提取，可粘贴人工转录内容。</p>
            <button className="ghost wide" data-testid="vision-transcribe-detail" disabled={visionBusy} onClick={visionTranscribe}>{visionBusy ? "视觉转录中..." : "用视觉大模型生成转录稿"}</button>
            {visionError ? <p className="error-text">{visionError}</p> : null}
            <label>转录备注<input value={manualNote} onChange={(event) => setManualNote(event.target.value)} placeholder="例如：人工核对第 1-3 页" /></label>
            <label>转录文本<textarea data-testid="manual-transcription-text" value={manualText} onChange={(event) => setManualText(event.target.value)} placeholder="粘贴 passage、questions、answers；保留 Questions 1-5 / Answers 这类标题有助于自动切分。" /></label>
            <button className="primary wide" data-testid="apply-manual-transcription" disabled={!manualText.trim()} onClick={applyTranscription}>应用手工转录</button>
          </details>
          {parserWarnings.length ? <><h4>Parser Warnings</h4><pre data-testid="parser-warnings">{JSON.stringify(parserWarnings, null, 2)}</pre></> : null}
          {lowConfidenceBlocks.length ? <><h4>低置信块</h4><pre data-testid="low-confidence-blocks">{JSON.stringify(lowConfidenceBlocks.map((block) => ({ blockId: block.blockId, confidence: block.confidence, roleHint: block.roleHint, text: block.text })), null, 2)}</pre></> : null}
          {pipelineReport ? <><h4>Auto Pipeline</h4><pre data-testid="pipeline-report">{JSON.stringify({ status: pipelineReport.status, currentStep: pipelineReport.currentStep, parser: pipelineReport.parser, llm: pipelineReport.llm, runtimeMode: pipelineReport.runtimeMode }, null, 2)}</pre></> : null}
          <pre>{selectedBlock ? JSON.stringify(selectedBlock, null, 2) : "Document IR missing. Create or parse a job first."}</pre>
        </aside>
      </div>
    </section>
  );
}
