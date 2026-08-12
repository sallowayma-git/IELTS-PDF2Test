import { useState } from "react";
import { createImportJob, getAuthoringV2, importSourceFile, runAutoPipeline } from "../api/tauriCommands";
import { choosePdfFolderSources, chooseSourceFile, chooseSourceFiles, type PickedPath } from "../api/desktopDialogs";
import { go } from "../app/router";
import type { AutoPipelineReport, Frequency, PassageCategory } from "../types";
import { jobStatusLabel, workflowStepLabel } from "../utils/displayLabels";
import { enqueueBackgroundCloudReview, startBackgroundCloudReviewScheduler } from "./UnifiedPreview";
import { isPhase5EditorEnabled } from "../config/featureFlags";

type BusyStage = "idle" | "creating" | "uploading" | "processing";

const MAX_IMPORT_FILE_BYTES = 128 * 1024 * 1024;

const stageLabels: Record<BusyStage, string> = {
  idle: "待开始",
  creating: "创建任务",
  uploading: "导入文件",
  processing: "首轮题稿生成中"
};

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(bytes >= 10 * 1024 * 1024 ? 0 : 1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

function oversizedImportFile(sourceFiles: PickedPath[], answerFile: PickedPath | null): PickedPath | null {
  return [...sourceFiles, ...(answerFile ? [answerFile] : [])].find((file) => Number(file.sizeBytes ?? 0) > MAX_IMPORT_FILE_BYTES) ?? null;
}

function formatImportError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  const match = message.match(/^source_file_too_large:max_bytes=(\d+):size_bytes=(\d+):path=(.+)$/);
  if (!match) return message;

  const [, maxBytesRaw, sizeBytesRaw, path] = match;
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  return `“${name}”超过导入上限（${formatBytes(Number(sizeBytesRaw))}，上限 ${formatBytes(Number(maxBytesRaw))}）。请拆分或压缩后重试。`;
}

function modelStageText(stage: BusyStage): string {
  if (stage === "processing") return "正在完成首轮粗切与题组生成；若 PDF 文字层不完整，会自动补做视觉识别。主流程只等待本地题稿落地，整卷云端复核会自动转入后台队列。";
  if (stage === "uploading") return "正在写入本地源文件和可选答案文件。";
  if (stage === "creating") return "正在建立导题任务。";
  return "选择文件后开始本地粗切。";
}

function backgroundCloudReviewProfileId(sourceFile: PickedPath, report: AutoPipelineReport): string | undefined {
  const profileId = report.llm.profileId;
  if (!profileId || profileId === "profile-local-placeholder") return undefined;
  if (report.quality?.cloudComparison?.attempted) return undefined;
  return /\.pdf$/i.test(sourceFile.name) || /\.pdf$/i.test(sourceFile.path) ? profileId : undefined;
}

function firstRouteFromReport(report: AutoPipelineReport): "preview" | "document" | "groups" | "llm-review" {
  if (report.nextRoute === "document") return "document";
  if (report.nextRoute === "groups") return "groups";
  if (report.nextRoute === "review") return "llm-review";
  return "preview";
}

function renderModelReport(report: AutoPipelineReport, cloudQueued = false) {
  const visionAnswers = report.parser?.visionAnswerExtraction;
  const cloud = report.quality?.cloudComparison;
  return (
    <div className="pipeline-summary">
      <div><span>视觉答案补全</span><strong>{visionAnswers?.attempted ? visionAnswers.applied ? `已补全 ${visionAnswers.answerCount ?? 0} 题` : "已尝试，未安全写入" : "未触发"}</strong></div>
      <div><span>云端整卷对照</span><strong>{cloud?.attempted ? cloud.passed ? "通过" : `${cloud.warningCount ?? cloud.issues?.length ?? 0} 项需确认` : cloudQueued ? "已加入后台队列" : "未触发"}</strong></div>
      <div><span>本地题稿</span><strong>{report.userStatus === "needsConfirmation" ? "已生成，待确认" : "已生成"}</strong></div>
      {visionAnswers?.missingQuestionIds?.length ? <p>仍缺少答案：{visionAnswers.missingQuestionIds.join("、")}</p> : null}
      {cloud?.issues?.slice(0, 3).map((issue, index) => <p key={`${index}-${issue.message}`}>{issue.message ?? "云端对照发现差异，请在题稿编辑页核对。"}</p>)}
    </div>
  );
}

export function ImportWizard({ refresh }: { refresh: () => void }) {
  const [title, setTitle] = useState("");
  const [category, setCategory] = useState<PassageCategory>("P1");
  const [frequency, setFrequency] = useState<Frequency>("medium");
  const [parseMode, setParseMode] = useState<"auto" | "text" | "ocr">("auto");
  const [tags, setTags] = useState("");
  const [sourceFiles, setSourceFiles] = useState<PickedPath[]>([]);
  const [answerFile, setAnswerFile] = useState<PickedPath | null>(null);
  const [busy, setBusy] = useState(false);
  const [busyStage, setBusyStage] = useState<BusyStage>("idle");
  const [autoReport, setAutoReport] = useState<AutoPipelineReport | undefined>();
  const [queuedCloudReviewCount, setQueuedCloudReviewCount] = useState(0);
  const [batchProgress, setBatchProgress] = useState<string | undefined>();
  const [error, setError] = useState<string | undefined>();

  async function pickSource() {
    const picked = await chooseSourceFiles();
    setSourceFiles(picked);
    if (picked.length === 1 && picked[0]?.titleHint) setTitle(picked[0].titleHint);
  }

  async function pickPdfFolder() {
    const picked = await choosePdfFolderSources();
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
    const oversized = oversizedImportFile(sourceFiles, answerFile);
    if (oversized) {
      setError(`“${oversized.name}”超过导入上限（${formatBytes(Number(oversized.sizeBytes ?? 0))}，上限 ${formatBytes(MAX_IMPORT_FILE_BYTES)}）。请拆分或压缩后重试。`);
      return;
    }
    const unsupported = sourceFiles.find((file) => file.requiresDesktopParser);
    if (unsupported) {
      setError(`无法读取“${unsupported.name}”的真实内容。请重新选择 PDF/DOCX/TXT/MD 文件，系统不会用演示内容替代真实解析。`);
      return;
    }
    setBusy(true);
    setBusyStage("creating");
    setError(undefined);
    setQueuedCloudReviewCount(0);
    setBatchProgress(undefined);
    try {
      let firstJobId: string | undefined;
      let firstRoute: "preview" | "document" | "groups" | "llm-review" = "preview";
      let latestReport: AutoPipelineReport | undefined;
      const backgroundCloudReviewJobs: Array<{ jobId: string; profileId: string }> = [];
      const tagList = tags.split(",").map((tag) => tag.trim()).filter(Boolean);
      for (const [index, sourceFile] of sourceFiles.entries()) {
        setBusyStage("creating");
        setBatchProgress(`正在生成 ${index + 1}/${sourceFiles.length}：${sourceFile.name}`);
        const jobTitle = sourceFiles.length === 1
          ? title
          : sourceFile.titleHint || title || sourceFile.name.replace(/\.[^.]+$/, "");
        const job = await createImportJob({ title: jobTitle, category, frequency, tags: tagList });
        setBusyStage("uploading");
        await importSourceFile(
          job.jobId,
          sourceFile.path,
          "MainQuestion",
          sourceFile.sizeBytes,
          sourceFile.textContent,
          sourceFile.binaryContentBase64
        );
        if (answerFile) {
          setBusyStage("uploading");
          await importSourceFile(
            job.jobId,
            answerFile.path,
            "AnswerKey",
            answerFile.sizeBytes,
            answerFile.textContent,
            answerFile.binaryContentBase64
          );
        }
        setBusyStage("processing");
        setBatchProgress(`正在生成 ${index + 1}/${sourceFiles.length}：${sourceFile.name}。当前先执行本地粗切，云复核稍后自动转后台。`);
        const report = await runAutoPipeline(job.jobId, { confidenceThreshold: 0.85, parseMode, executionMode: "localOnly", target: "editableDraft" });
        latestReport = report;
        const cloudReviewProfileId = backgroundCloudReviewProfileId(sourceFile, report);
        if (cloudReviewProfileId) {
          backgroundCloudReviewJobs.push({ jobId: job.jobId, profileId: cloudReviewProfileId });
        }
        if (!firstJobId) {
          firstJobId = job.jobId;
          firstRoute = firstRouteFromReport(report);
        }
      }
      if (backgroundCloudReviewJobs.length) {
        setBatchProgress(`本地题稿已完成，正在把 ${backgroundCloudReviewJobs.length} 个 PDF 加入后台云复核队列。`);
        setQueuedCloudReviewCount(backgroundCloudReviewJobs.length);
        for (const { jobId, profileId } of backgroundCloudReviewJobs) {
          enqueueBackgroundCloudReview(jobId, profileId);
        }
        startBackgroundCloudReviewScheduler();
      }
      setAutoReport(latestReport);
      refresh();
      if (firstJobId) {
        let destination: "preview" | "document" | "groups" | "llm-review" | "authoring-v2" = firstRoute;
        if (isPhase5EditorEnabled()) {
          try {
            await getAuthoringV2(firstJobId);
            destination = "authoring-v2";
          } catch {
            // Keep the legacy preview as a safe fallback when the controlled
            // V2 shadow rollout is not enabled for this packaged runtime.
          }
        }
        go(`/jobs/${firstJobId}/${destination}`);
      }
    } catch (caught) {
      setError(formatImportError(caught));
    } finally {
      setBusy(false);
      setBusyStage("idle");
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
        <dt>视觉答案补全</dt><dd>{report.parser?.visionAnswerExtraction?.attempted ? report.parser.visionAnswerExtraction.applied ? `已补全 ${report.parser.visionAnswerExtraction.answerCount ?? 0} 题` : "已尝试，未安全写入" : "未触发"}</dd>
        <dt>云端对照</dt><dd>{report.quality?.cloudComparison?.attempted ? report.quality.cloudComparison.passed ? "通过" : `${report.quality.cloudComparison.warningCount ?? 0} 项需确认` : queuedCloudReviewCount > 0 ? "已加入后台队列" : "将自动转入后台队列"}</dd>
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
            <button className="ghost small" data-testid="pick-source-folder" onClick={pickPdfFolder}>选择 PDF 文件夹</button>
          </div>
          <div className="path-picker">
            <span>
              <strong>答案文件 可选</strong>
              <small data-testid="answer-file-path">{answerFile ? answerFile.path : "可选，用于答案候选抽取"}</small>
            </span>
            <button className="ghost small" data-testid="pick-answer-file" onClick={pickAnswer}>选择答案文件</button>
          </div>
          {autoReport ? <details data-testid="auto-pipeline-report"><summary>自动处理结果</summary>{renderAutoReport(autoReport)}</details> : null}
        </section>
        <section className="form-section">
          <span className="step-number">02</span>
          <h3>题稿信息</h3>
          <label>标题（可选）<input data-testid="job-title-input" value={title} onChange={(event) => setTitle(event.target.value)} placeholder="默认使用文件名" /></label>
          <details>
            <summary>高级设置</summary>
            <label>Passage 分类<select data-testid="category-select" value={category} onChange={(event) => setCategory(event.target.value as PassageCategory)}><option>P1</option><option>P2</option><option>P3</option></select></label>
            <label>难度<select data-testid="frequency-select" value={frequency} onChange={(event) => setFrequency(event.target.value as Frequency)}><option value="low">low</option><option value="medium">medium</option><option value="high">high</option></select></label>
            <label>标签<input data-testid="tags-input" value={tags} onChange={(event) => setTags(event.target.value)} /></label>
            <label>识别方式<select data-testid="parse-mode" value={parseMode} onChange={(event) => setParseMode(event.target.value as typeof parseMode)}><option value="auto">自动</option><option value="text">读取文字</option><option value="ocr">识别扫描件文字</option></select></label>
          </details>
        </section>
        <section className="form-section contrast">
          <span className="step-number">03</span>
          <h3>生成题稿</h3>
          <p>上传后先直接生成首轮题稿；若 PDF 文字层不完整会自动补做视觉识别，整卷云端复核会自动排入后台，不再阻塞当前页面。</p>
          {busy ? (
            <div className="progress-panel" data-testid="generation-progress-panel">
              <div className="spinner" aria-hidden="true" />
              <div>
                <strong>{stageLabels[busyStage]}</strong>
                {batchProgress ? <p data-testid="batch-progress">{batchProgress}</p> : null}
                <p>{modelStageText(busyStage)}</p>
              </div>
            </div>
          ) : batchProgress ? <p data-testid="batch-progress">{batchProgress}</p> : null}
          {autoReport ? renderModelReport(autoReport, queuedCloudReviewCount > 0) : null}
          {error ? <p className="error-text" data-testid="import-error">{error}</p> : null}
          <button className="primary wide" data-testid="create-and-auto-process" disabled={busy || !sourceFiles.length} onClick={submit}>{busy ? "正在生成本地题稿..." : "开始生成题稿"}</button>
        </section>
      </div>
    </section>
  );
}
