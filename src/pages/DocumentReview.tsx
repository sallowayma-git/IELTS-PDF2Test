import { useEffect, useState } from "react";
import { getJob, rerunOcr, runRuleSplit } from "../api/tauriCommands";
import { go } from "../app/router";
import type { DocumentIr, ImportJob } from "../types";

export function DocumentReview({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [job, setJob] = useState<ImportJob | null>(null);
  const [documentIr, setDocumentIr] = useState<DocumentIr | undefined>();
  const [selected, setSelected] = useState<string | undefined>();

  async function load() {
    const detail = await getJob(jobId);
    setJob(detail.job);
    setDocumentIr(detail.documentIr);
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

  const blocks = documentIr?.pages.flatMap((page) => page.blocks.map((block) => ({ ...block, pageIndex: page.pageIndex }))) ?? [];
  const selectedBlock = blocks.find((block) => block.blockId === selected) ?? blocks[0];

  return (
    <section className="document-review page-enter">
      <div className="section-heading spread">
        <div>
          <p className="eyebrow">Document IR</p>
          <h2>文档解析预览</h2>
        </div>
        <div className="button-row"><button className="ghost" onClick={ocr}>重新 OCR</button><button className="primary" onClick={split}>进入粗切</button></div>
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
          </dl>
          <pre>{selectedBlock ? JSON.stringify(selectedBlock, null, 2) : "Document IR missing. Create or parse a job first."}</pre>
        </aside>
      </div>
    </section>
  );
}
