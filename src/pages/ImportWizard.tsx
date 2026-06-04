import { useState } from "react";
import { createImportJob, importSourceFile, runAutoPipeline } from "../api/tauriCommands";
import { chooseSourceFile, chooseSourceFiles, type PickedPath } from "../api/desktopDialogs";
import { go } from "../app/router";
import type { AutoPipelineReport, Frequency, PassageCategory } from "../types";
import { jobStatusLabel, workflowStepLabel } from "../utils/displayLabels";

export function ImportWizard({ refresh }: { refresh: () => void }) {
  const [title, setTitle] = useState("");
  const [category, setCategory] = useState<PassageCategory>("P1");
  const [frequency, setFrequency] = useState<Frequency>("medium");
  const [parseMode, setParseMode] = useState<"auto" | "text" | "ocr">("auto");
  const [tags, setTags] = useState("");
  const [sourceFiles, setSourceFiles] = useState<PickedPath[]>([]);
  const [answerFile, setAnswerFile] = useState<PickedPath | null>(null);
  const [busy, setBusy] = useState(false);
  const [autoReport, setAutoReport] = useState<AutoPipelineReport | undefined>();
  const [batchProgress, setBatchProgress] = useState<string | undefined>();
  const [error, setError] = useState<string | undefined>();

  async function pickSource() {
    const picked = await chooseSourceFiles();
    setSourceFiles(picked);
    if (picked.length === 1 && picked[0]?.titleHint) setTitle(picked[0].titleHint);
  }

  async function pickAnswer() {
    setAnswerFile(await chooseSourceFile());
  }

  async function submit() {
    if (!sourceFiles.length) {
      setError("请选择主文件后再创建任务。生产流程不能静默使用 demo 文件。");
      return;
    }
    const unsupported = sourceFiles.find((file) => file.requiresDesktopParser);
    if (unsupported) {
      setError(`无法读取“${unsupported.name}”的真实内容。请重新选择 PDF/DOCX/TXT/MD 文件，系统不会用演示内容替代真实解析。`);
      return;
    }
    setBusy(true);
    setError(undefined);
    setBatchProgress(undefined);
    try {
      let firstJobId: string | undefined;
      let firstRoute: "groups" | "document" | "llm-review" = "groups";
      let latestReport: AutoPipelineReport | undefined;
      const tagList = tags.split(",").map((tag) => tag.trim()).filter(Boolean);
      for (const [index, sourceFile] of sourceFiles.entries()) {
        setBatchProgress(`正在生成 ${index + 1}/${sourceFiles.length}：${sourceFile.name}`);
        const jobTitle = sourceFiles.length === 1
          ? title
          : sourceFile.titleHint || title || sourceFile.name.replace(/\.[^.]+$/, "");
        const job = await createImportJob({ title: jobTitle, category, frequency, tags: tagList });
        await importSourceFile(
          job.jobId,
          sourceFile.path,
          "MainQuestion",
          sourceFile.sizeBytes,
          sourceFile.textContent,
          sourceFile.binaryContentBase64
        );
        if (answerFile) {
          await importSourceFile(
            job.jobId,
            answerFile.path,
            "AnswerKey",
            answerFile.sizeBytes,
            answerFile.textContent,
            answerFile.binaryContentBase64
          );
        }
        const report = await runAutoPipeline(job.jobId, { confidenceThreshold: 0.85, parseMode, target: "editableDraft" });
        latestReport = report;
        if (!firstJobId) {
          firstJobId = job.jobId;
          firstRoute = report.nextRoute === "document"
            ? "document"
            : report.nextRoute === "review"
              ? "llm-review"
              : "groups";
        }
      }
      setAutoReport(latestReport);
      refresh();
      if (firstJobId) go(`/jobs/${firstJobId}/${firstRoute}`);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setBusy(false);
      setBatchProgress(undefined);
    }
  }

  function renderAutoReport(report: AutoPipelineReport) {
    return (
      <dl>
        <dt>任务状态</dt><dd>{jobStatusLabel(report.status)}</dd>
        <dt>当前步骤</dt><dd>{workflowStepLabel(report.currentStep)}</dd>
        <dt>处理结果</dt><dd>{report.userMessage ?? "题稿已生成，可以开始检查和编辑。"}</dd>
        <dt>需要确认的识别结果</dt><dd>{report.llm.suggestionCount} 条，已采用 {report.llm.appliedCount} 条</dd>
        <dt>仍需确认</dt><dd>{report.authoring?.remainingReviewItems ?? 0} 项</dd>
      </dl>
    );
  }

  return (
    <section className="wizard page-enter" data-testid="import-wizard">
      <div className="section-heading">
        <p className="eyebrow">新建题稿</p>
        <h2>新建导题任务</h2>
      </div>
      <div className="wizard-grid">
        <section className="form-section">
          <span className="step-number">01</span>
          <h3>选择本地文件</h3>
          <div className="path-picker">
            <span>
              <strong>主文件 PDF/DOCX/TXT/MD</strong>
              <small data-testid="source-file-path">
                {sourceFiles.length
                  ? sourceFiles.length === 1
                    ? sourceFiles[0].path
                    : `已选择 ${sourceFiles.length} 份：${sourceFiles.map((file) => file.name).slice(0, 3).join("、")}${sourceFiles.length > 3 ? "..." : ""}`
                  : "尚未选择；可一次选择多份 PDF/DOCX/TXT/MD"}
              </small>
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
          <label>识别方式<select data-testid="parse-mode" value={parseMode} onChange={(event) => setParseMode(event.target.value as typeof parseMode)}><option value="auto">自动</option><option value="text">读取文字</option><option value="ocr">识别扫描件文字</option></select></label>
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
          <h3>生成题稿</h3>
          <p>选择一份或多份主文件后，一次点击即可生成可编辑题稿；答案文件可选，通常可随主文件一并导入。</p>
          {batchProgress ? <p data-testid="batch-progress">{batchProgress}</p> : null}
          {error ? <p className="error-text" data-testid="import-error">{error}</p> : null}
          <button className="primary wide" data-testid="create-and-auto-process" disabled={busy || !sourceFiles.length} onClick={submit}>{busy ? "正在生成题稿..." : "开始生成题稿"}</button>
        </section>
      </div>
    </section>
  );
}
