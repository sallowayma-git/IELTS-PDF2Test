import { useEffect, useState } from "react";
import { applyManualTranscription, applyVisionTranscription, getJob, rerunOcr, resolveSourceReview, runRuleSplit } from "../api/tauriCommands";
import { go } from "../app/router";
import type { AutoPipelineReport, DocumentBlock, DocumentIr, ImportJob, SourceReview } from "../types";
import { jobStatusLabel, workflowStepLabel, runtimeModeLabel } from "../utils/displayLabels";

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
  const sourceReviewStatus = sourceReview?.required ? (sourceReview.resolved ? "已确认" : "需确认") : "无需确认";

  return (
    <section className="document-review page-enter" data-testid="document-review">
      <div className="section-heading spread">
        <div>
          <p className="eyebrow">文档解析结果</p>
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
          <p className="eyebrow">文本块详情</p>
          <h3>{selectedBlock?.blockId ?? "No block"}</h3>
          <dl>
            <dt>任务</dt><dd>{job?.jobId}</dd>
            <dt>类型</dt><dd>{selectedBlock?.roleHint ?? "未标记"}</dd>
            <dt>置信度</dt><dd>{selectedBlock ? Math.round(selectedBlock.confidence * 100) : 0}%</dd>
            <dt>源文档审核</dt><dd data-testid="source-review-status">{sourceReviewStatus}</dd>
          </dl>
          {sourceReview?.required ? <><h4>源文档审核</h4><dl data-testid="source-review-json"><dt>状态</dt><dd>{sourceReviewStatus}</dd><dt>解析提醒</dt><dd>{sourceReview.parserWarnings.length}</dd><dt>低置信内容</dt><dd>{sourceReview.lowConfidenceBlocks.length}</dd>{sourceReview.note ? <><dt>备注</dt><dd>{sourceReview.note}</dd></> : null}</dl></> : null}
          <details open={Boolean(sourceReview?.required || lowConfidenceBlocks.length || parserWarnings.length)}>
            <summary>扫描 PDF / OCR 失败处理</summary>
            <p>图片型 PDF 默认优先尝试视觉大模型转录，生成文档解析结果；该结果仍会保留源文档审核门禁，必须人工确认后才能发布。若模型不可用或页面图片无法提取，可粘贴人工转录内容。</p>
            <button className="ghost wide" data-testid="vision-transcribe-detail" disabled={visionBusy} onClick={visionTranscribe}>{visionBusy ? "视觉转录中..." : "用视觉大模型生成转录稿"}</button>
            {visionError ? <p className="error-text">{visionError}</p> : null}
            <label>转录备注<input value={manualNote} onChange={(event) => setManualNote(event.target.value)} placeholder="例如：人工核对第 1-3 页" /></label>
            <label>转录文本<textarea data-testid="manual-transcription-text" value={manualText} onChange={(event) => setManualText(event.target.value)} placeholder="粘贴文章、题目和答案；保留“Questions 1-5”“Answers”这类标题有助于自动切分。" /></label>
            <button className="primary wide" data-testid="apply-manual-transcription" disabled={!manualText.trim()} onClick={applyTranscription}>应用手工转录</button>
          </details>
          {parserWarnings.length ? <><h4>解析提醒</h4><pre data-testid="parser-warnings">{JSON.stringify(parserWarnings, null, 2)}</pre></> : null}
          {lowConfidenceBlocks.length ? <><h4>低置信块</h4><pre data-testid="low-confidence-blocks">{JSON.stringify(lowConfidenceBlocks.map((block) => ({ blockId: block.blockId, confidence: block.confidence, roleHint: block.roleHint, text: block.text })), null, 2)}</pre></> : null}
          {pipelineReport ? <><h4>自动处理进度</h4><dl data-testid="pipeline-report"><dt>任务状态</dt><dd>{jobStatusLabel(pipelineReport.status)}</dd><dt>当前步骤</dt><dd>{workflowStepLabel(pipelineReport.currentStep)}</dd><dt>解析提醒</dt><dd>{pipelineReport.parser?.warnings.length ?? 0} 条</dd><dt>低置信内容</dt><dd>{pipelineReport.parser?.lowConfidenceBlocks.length ?? 0} 个</dd><dt>模型建议</dt><dd>{pipelineReport.llm.suggestionCount} 条，已自动应用 {pipelineReport.llm.appliedCount} 条</dd><dt>预览检查</dt><dd>{runtimeModeLabel(pipelineReport.runtimeMode)}</dd></dl></> : null}
          {selectedBlock ? <dl><dt>段落编号</dt><dd>{selectedBlock.blockId}</dd><dt>内容类型</dt><dd>{selectedBlock.blockType}</dd><dt>文字</dt><dd>{selectedBlock.text ?? "无文字内容"}</dd></dl> : <p className="empty">当前没有保留完整解析中间结果。若自动处理已完成，请直接进入粗切/结构编辑；源文档审核结果仍保存在最小可编辑态中。</p>}
        </aside>
      </div>
    </section>
  );
}
