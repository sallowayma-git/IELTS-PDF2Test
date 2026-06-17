import { useEffect, useState } from "react";
import { applyManualTranscription, applyVisionTranscription, buildAuthoringIr, getJob, rerunOcr, resolveSourceReview, runRuleSplit } from "../api/tauriCommands";
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

  async function continueToPreview() {
    try {
      const detail = await getJob(jobId);
      if (!detail.authoringIr) {
        await runRuleSplit(jobId);
        await buildAuthoringIr(jobId);
      }
      refresh();
      go(`/jobs/${jobId}/preview`);
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : String(caught);
      setVisionError(message.includes("editable_draft_exists")
        ? "当前任务已有题稿，请直接进入确认与编辑；默认不会重新识别并覆盖现有题稿。"
        : message);
    }
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
      await applyVisionTranscription(jobId, { note: "识别扫描件文字；需人工逐页核验" });
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
          <p className="eyebrow">源文档确认</p>
          <h2>识别结果预览</h2>
        </div>
        <div className="button-row"><button className="ghost" data-testid="rerun-ocr" onClick={ocr}>重新识别文字</button><button className="ghost" data-testid="vision-transcribe" disabled={visionBusy} onClick={visionTranscribe}>{visionBusy ? "正在识别..." : "识别扫描件文字"}</button><button className="ghost" data-testid="resolve-source-review" disabled={!sourceReview?.required || sourceReview.resolved} onClick={resolveReview}>确认源文档</button><button className="primary" data-testid="go-preview" onClick={continueToPreview}>进入确认与编辑</button></div>
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
            <summary>扫描件文字识别处理</summary>
            <p>如果 PDF 是扫描件，可以先识别扫描件文字，再对照源文件确认。若识别不可用或页面图片无法提取，可粘贴手工整理的文字。</p>
            <button className="ghost wide" data-testid="vision-transcribe-detail" disabled={visionBusy} onClick={visionTranscribe}>{visionBusy ? "正在识别..." : "识别扫描件文字"}</button>
            {visionError ? <p className="error-text">{visionError}</p> : null}
            <label>转录备注<input value={manualNote} onChange={(event) => setManualNote(event.target.value)} placeholder="例如：人工核对第 1-3 页" /></label>
            <label>转录文本<textarea data-testid="manual-transcription-text" value={manualText} onChange={(event) => setManualText(event.target.value)} placeholder="粘贴文章、题目和答案；保留“Questions 1-5”“Answers”这类标题有助于自动切分。" /></label>
            <button className="primary wide" data-testid="apply-manual-transcription" disabled={!manualText.trim()} onClick={applyTranscription}>应用手工转录</button>
          </details>
          {parserWarnings.length ? <><h4>解析提醒</h4><pre data-testid="parser-warnings">{JSON.stringify(parserWarnings, null, 2)}</pre></> : null}
          {lowConfidenceBlocks.length ? <><h4>低置信块</h4><pre data-testid="low-confidence-blocks">{JSON.stringify(lowConfidenceBlocks.map((block) => ({ blockId: block.blockId, confidence: block.confidence, roleHint: block.roleHint, text: block.text })), null, 2)}</pre></> : null}
          {pipelineReport ? <><h4>自动处理进度</h4><dl data-testid="pipeline-report"><dt>任务状态</dt><dd>{jobStatusLabel(pipelineReport.status)}</dd><dt>当前步骤</dt><dd>{workflowStepLabel(pipelineReport.currentStep)}</dd><dt>解析提醒</dt><dd>{pipelineReport.parser?.warnings.length ?? 0} 条</dd><dt>低置信内容</dt><dd>{pipelineReport.parser?.lowConfidenceBlocks.length ?? 0} 个</dd><dt>需要确认的识别结果</dt><dd>{pipelineReport.llm.suggestionCount} 条，已采用 {pipelineReport.llm.appliedCount} 条</dd><dt>检查结果</dt><dd>{runtimeModeLabel(pipelineReport.runtimeMode)}</dd></dl></> : null}
          {selectedBlock ? <dl><dt>段落编号</dt><dd>{selectedBlock.blockId}</dd><dt>内容类型</dt><dd>{selectedBlock.blockType}</dd><dt>文字</dt><dd>{selectedBlock.text ?? "无文字内容"}</dd></dl> : <p className="empty">当前没有保留完整识别中间结果。若自动处理已完成，请直接进入题组确认或题稿编辑；源文档确认结果仍会保留。</p>}
        </aside>
      </div>
    </section>
  );
}
