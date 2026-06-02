import { useState } from "react";
import { createImportJob, importSourceFile, runAutoPipeline } from "../api/tauriCommands";
import { chooseSourceFile, type PickedPath } from "../api/desktopDialogs";
import { go } from "../app/router";
import type { AutoPipelineReport, Frequency, PassageCategory } from "../types";
import { jobStatusLabel, workflowStepLabel, runtimeModeLabel } from "../utils/displayLabels";

export function ImportWizard({ refresh }: { refresh: () => void }) {
  const [title, setTitle] = useState("");
  const [category, setCategory] = useState<PassageCategory>("P1");
  const [frequency, setFrequency] = useState<Frequency>("medium");
  const [parseMode, setParseMode] = useState<"auto" | "text" | "ocr">("auto");
  const [tags, setTags] = useState("");
  const [sourceFile, setSourceFile] = useState<PickedPath | null>(null);
  const [answerFile, setAnswerFile] = useState<PickedPath | null>(null);
  const [busy, setBusy] = useState(false);
  const [autoReport, setAutoReport] = useState<AutoPipelineReport | undefined>();
  const [error, setError] = useState<string | undefined>();

  async function pickSource() {
    const picked = await chooseSourceFile();
    setSourceFile(picked);
    if (picked?.titleHint) setTitle(picked.titleHint);
  }

  async function pickAnswer() {
    setAnswerFile(await chooseSourceFile());
  }

  async function submit() {
    if (!sourceFile) {
      setError("请选择主文件后再创建任务。生产流程不能静默使用 demo 文件。");
      return;
    }
    if (sourceFile.requiresDesktopParser) {
      setError("当前浏览器开发预览不能解析 PDF/DOCX 的真实内容。请在 Tauri 桌面应用中导入该文件，或先上传 TXT/MD 文本/使用人工转录；系统不会用演示内容替代真实解析。");
      return;
    }
    setBusy(true);
    setError(undefined);
    try {
      const job = await createImportJob({ title, category, frequency, tags: tags.split(",").map((tag) => tag.trim()).filter(Boolean) });
      await importSourceFile(job.jobId, sourceFile.path, "MainQuestion", sourceFile.sizeBytes, sourceFile.textContent);
      if (answerFile) await importSourceFile(job.jobId, answerFile.path, "AnswerKey", answerFile.sizeBytes, answerFile.textContent);
      const report = await runAutoPipeline(job.jobId, { confidenceThreshold: 0.85, parseMode });
      setAutoReport(report);
      refresh();
      const nextStep = typeof report.currentStep === "string" ? report.currentStep : "DocumentReview";
      const nextPath = nextStep === "LlmReview"
        ? "llm-review"
        : nextStep === "Export"
          ? "export"
          : nextStep === "Preview"
            ? "preview"
            : nextStep === "Authoring"
              ? "groups"
              : nextStep === "Split"
                ? "split"
                : "document";
      go(`/jobs/${job.jobId}/${nextPath}`);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setBusy(false);
    }
  }

  function renderAutoReport(report: AutoPipelineReport) {
    return (
      <dl>
        <dt>任务状态</dt><dd>{jobStatusLabel(report.status)}</dd>
        <dt>当前步骤</dt><dd>{workflowStepLabel(report.currentStep)}</dd>
        <dt>模型建议</dt><dd>{report.llm.suggestionCount} 条，已自动应用 {report.llm.appliedCount} 条</dd>
        <dt>仍需审核</dt><dd>{report.authoring?.remainingReviewItems ?? 0} 项</dd>
        <dt>预览检查</dt><dd>{runtimeModeLabel(report.runtimeMode)}</dd>
      </dl>
    );
  }

  return (
    <section className="wizard page-enter" data-testid="import-wizard">
      <div className="section-heading">
        <p className="eyebrow">Import Wizard</p>
        <h2>新建导题任务</h2>
      </div>
      <div className="wizard-grid">
        <section className="form-section">
          <span className="step-number">01</span>
          <h3>选择本地文件</h3>
          <div className="path-picker">
            <span>
              <strong>主文件 PDF/DOCX/TXT/MD</strong>
              <small data-testid="source-file-path">{sourceFile ? sourceFile.path : "尚未选择；桌面端会打开系统文件选择器"}</small>
            </span>
            <button className="ghost small" data-testid="pick-source-file" onClick={pickSource}>选择主文件</button>
          </div>
          <div className="path-picker">
            <span>
              <strong>答案文件 可选</strong>
              <small data-testid="answer-file-path">{answerFile ? answerFile.path : "可选，用于答案候选抽取"}</small>
            </span>
            <button className="ghost small" data-testid="pick-answer-file" onClick={pickAnswer}>选择答案文件</button>
          </div>
          <label>解析模式<select data-testid="parse-mode" value={parseMode} onChange={(event) => setParseMode(event.target.value as typeof parseMode)}><option value="auto">自动</option><option value="text">纯文本</option><option value="ocr">视觉转录</option></select></label>
          {autoReport ? <details data-testid="auto-pipeline-report"><summary>自动处理结果</summary>{renderAutoReport(autoReport)}</details> : null}
        </section>
        <section className="form-section">
          <span className="step-number">02</span>
          <h3>基础信息</h3>
          <label>标题<input data-testid="job-title-input" value={title} onChange={(event) => setTitle(event.target.value)} /></label>
          <label>Passage 分类<select data-testid="category-select" value={category} onChange={(event) => setCategory(event.target.value as PassageCategory)}><option>P1</option><option>P2</option><option>P3</option></select></label>
          <label>难度<select data-testid="frequency-select" value={frequency} onChange={(event) => setFrequency(event.target.value as Frequency)}><option value="low">low</option><option value="medium">medium</option><option value="high">high</option></select></label>
          <label>标签<input data-testid="tags-input" value={tags} onChange={(event) => setTags(event.target.value)} /></label>
        </section>
        <section className="form-section contrast">
          <span className="step-number">03</span>
          <h3>创建任务</h3>
          <p>选择主文件后会创建导题任务；答案文件可选，通常可随主文件一并导入。</p>
          {error ? <p className="error-text" data-testid="import-error">{error}</p> : null}
          <button className="primary wide" data-testid="create-and-auto-process" disabled={busy || !sourceFile} onClick={submit}>{busy ? "自动处理中..." : "创建并自动处理"}</button>
        </section>
      </div>
    </section>
  );
}
