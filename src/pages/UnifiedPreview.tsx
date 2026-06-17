import { useEffect, useMemo, useRef, useState } from "react";
import {
  generatePreviewAssets,
  getJob,
  runPreviewE2e,
  updateAuthoringIr,
  validateAuthoringIr
} from "../api/tauriCommands";
import { go } from "../app/router";
import type {
  AutoPipelineReport,
  GroupKind,
  PreviewAssets,
  QuestionDraft,
  QuestionGroupDraft,
  ReadingAuthoringIr,
  ValidationReport
} from "../types";
import { runtimeModeLabel, validationIssueDisplay, validationLayerLabel } from "../utils/displayLabels";

const EXPORT_INTENT_KEY_PREFIX = "ielts-author-studio.export-intent.";

const groupKinds: GroupKind[] = [
  "true_false_not_given",
  "yes_no_not_given",
  "single_choice",
  "multi_choice",
  "short_answer",
  "sentence_completion",
  "summary_completion",
  "table_completion",
  "matching",
  "heading_matching",
  "matching_information",
  "classification",
  "diagram_completion"
];

const groupKindLabels: Record<GroupKind, string> = {
  single_choice: "单选",
  multi_choice: "多选",
  true_false_not_given: "TRUE/FALSE/NOT GIVEN",
  yes_no_not_given: "YES/NO/NOT GIVEN",
  matching: "匹配题",
  heading_matching: "标题匹配",
  matching_information: "信息匹配",
  classification: "分类题",
  summary_completion: "摘要填空",
  table_completion: "表格填空",
  diagram_completion: "图表填空",
  short_answer: "简答题",
  sentence_completion: "句子填空"
};

type AuditIssue = string | { message?: string; [key: string]: unknown };
type QuestionStatusTone = "ok" | "candidate" | "mismatch" | "unknown";

interface QuestionStatus {
  tone: QuestionStatusTone;
  label: string;
  details: string[];
}

const questionStatusIcon: Record<QuestionStatusTone, string> = {
  ok: "✓",
  candidate: "+",
  mismatch: "!",
  unknown: "?"
};

function buildFallbackSrcDoc(assets: PreviewAssets): string {
  return `<!doctype html><html><head><meta charset="utf-8"><style>body{font-family:Georgia,serif;margin:0;padding:24px;background:#f5f1e8;color:#17211f;line-height:1.6}.layout{display:grid;grid-template-columns:1fr 420px;gap:28px}.pane{background:#fffaf0;border:1px solid #d8cfbf;padding:22px}.choice-row{display:flex;gap:10px;flex-wrap:wrap}.completion-table{width:100%;border-collapse:collapse}.completion-table th,.completion-table td{border:1px solid #c8beaa;padding:8px}.question-umbrella-ranges{padding-left:18px;color:#5d4630}input{font:inherit;padding:6px}</style></head><body><div class="layout"><article class="pane">${assets.source.passage.blocks.map((block) => block.html).join("")}</article><section class="pane">${assets.source.meta.questionIntroHtml}${assets.source.questionGroups.map((group) => group.bodyHtml).join("")}</section></div></body></html>`;
}

function auditIssueText(issue: AuditIssue): string {
  if (typeof issue === "string") return issue;
  return typeof issue.message === "string" ? issue.message : "";
}

function auditIssueKind(issue: AuditIssue): string {
  if (typeof issue === "string") return "";
  return typeof issue.kind === "string" ? issue.kind : "";
}

function auditIssuePath(issue: AuditIssue): string {
  if (typeof issue === "string") return "";
  return typeof issue.path === "string" ? issue.path : "";
}

function issueList(issue: AuditIssue | undefined, key: string): Array<{ message?: string; [key: string]: unknown }> {
  if (!issue || typeof issue === "string") return [];
  const value = issue[key];
  return Array.isArray(value) ? value.filter((item): item is { message?: string; [key: string]: unknown } => Boolean(item && typeof item === "object")) : [];
}

function issueStringList(issue: AuditIssue | undefined, key: string): string[] {
  if (!issue || typeof issue === "string") return [];
  const value = issue[key];
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function issueSummary(issue: AuditIssue | undefined, key: string): Array<{ range?: unknown; kind?: unknown; layoutHint?: unknown; questionIds?: unknown }> {
  if (!issue || typeof issue === "string") return [];
  const value = issue[key];
  return Array.isArray(value) ? value.filter((item): item is { range?: unknown; kind?: unknown; layoutHint?: unknown; questionIds?: unknown } => Boolean(item && typeof item === "object")) : [];
}

function issueBool(issue: AuditIssue | undefined, key: string): boolean {
  if (!issue || typeof issue === "string") return false;
  return issue[key] === true;
}

function issueString(issue: AuditIssue | undefined, key: string): string {
  if (!issue || typeof issue === "string") return "";
  return typeof issue[key] === "string" ? issue[key] : "";
}

function formatSummaryRow(item: { range?: unknown; kind?: unknown; layoutHint?: unknown; questionIds?: unknown }): string {
  const range = Array.isArray(item.range) && item.range.length >= 2 ? `Q${item.range[0]}-${item.range[1]}` : "范围未知";
  const ids = Array.isArray(item.questionIds) ? item.questionIds.join(", ") : "";
  return `${range} · ${String(item.kind ?? "题型未知")} · ${String(item.layoutHint ?? "布局未知")}${ids ? ` · ${ids}` : ""}`;
}

function parseQuestionNumber(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value !== "string") return undefined;
  const match = value.match(/\d+/);
  return match ? Number(match[0]) : undefined;
}

function questionNumber(question: QuestionDraft): number | undefined {
  return parseQuestionNumber(question.displayNumber) ?? parseQuestionNumber(question.id);
}

function questionIdFor(question: QuestionDraft): string {
  return question.id || (questionNumber(question) ? `q${questionNumber(question)}` : "");
}

function rangeCovers(item: { [key: string]: unknown }, number: number | undefined): boolean {
  if (number === undefined || !Array.isArray(item.range) || item.range.length < 2) return false;
  const start = parseQuestionNumber(item.range[0]);
  const end = parseQuestionNumber(item.range[1]) ?? start;
  return start !== undefined && end !== undefined && number >= start && number <= end;
}

function questionIdsInclude(value: unknown, qid: string): boolean {
  return Array.isArray(value) && value.some((item) => item === qid);
}

function itemQuestionNumber(item: { [key: string]: unknown }): number | undefined {
  return parseQuestionNumber(item.questionNumber);
}

function itemHasQuestionScope(item: { [key: string]: unknown }): boolean {
  return itemQuestionNumber(item) !== undefined || Array.isArray(item.range);
}

function itemAppliesToQuestion(item: { [key: string]: unknown }, question: QuestionDraft): boolean {
  const number = questionNumber(question);
  const qid = questionIdFor(question);
  return itemQuestionNumber(item) === number
    || rangeCovers(item, number)
    || questionIdsInclude(item.localQuestionIds, qid)
    || questionIdsInclude(item.cloudQuestionIds, qid)
    || questionIdsInclude(item.expectedQuestionIds, qid);
}

function formatAnswerValue(value: unknown): string {
  if (Array.isArray(value)) return value.map(formatAnswerValue).filter(Boolean).join(", ");
  if (typeof value === "string") return value.trim();
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

function cloudDetailText(item: { message?: string; [key: string]: unknown }): string {
  const message = item.message ?? "云端对照发现差异，请核对。";
  const kind = typeof item.kind === "string" ? item.kind : "";
  const localAnswer = formatAnswerValue(item.localAnswer);
  const cloudAnswer = formatAnswerValue(item.cloudAnswer);
  if (kind === "cloud_answer_mismatch" && (localAnswer || cloudAnswer)) {
    return `${message} 本地：${localAnswer || "空"}；云端：${cloudAnswer || "空"}`;
  }
  if (kind === "cloud_answer_candidate_only" && cloudAnswer) {
    return `云端候选答案：${cloudAnswer}。本地答案为空，请核对图片答案页后决定是否写入。`;
  }
  if (kind === "cloud_group_kind_mismatch") {
    return `${message} 本地题型：${String(item.localKind ?? "未知")}；云端题型：${String(item.cloudKind ?? "未知")}`;
  }
  if (kind === "cloud_group_layout_mismatch") {
    return `${message} 本地布局：${String(item.localLayout ?? "未知")}；云端布局：${String(item.cloudLayout ?? "未知")}`;
  }
  return message;
}

function summaryCoversQuestion(cloudIssue: AuditIssue | undefined, key: string, question: QuestionDraft): boolean {
  const number = questionNumber(question);
  const qid = questionIdFor(question);
  return issueSummary(cloudIssue, key).some((item) => {
    const asRecord = item as { [key: string]: unknown };
    return rangeCovers(asRecord, number) || questionIdsInclude(item.questionIds, qid);
  });
}

function cloudCoversQuestion(cloudIssue: AuditIssue | undefined, question: QuestionDraft): boolean {
  return summaryCoversQuestion(cloudIssue, "localSummary", question) || summaryCoversQuestion(cloudIssue, "cloudSummary", question);
}

function buildCloudQuestionStatus(question: QuestionDraft, cloudIssue: AuditIssue | undefined): QuestionStatus {
  if (!cloudIssue || !issueBool(cloudIssue, "attempted")) {
    return { tone: "unknown", label: "云端未核对", details: [] };
  }
  const allIssues = issueList(cloudIssue, "issues");
  const scopedIssues = allIssues.filter((item) => itemAppliesToQuestion(item, question));
  const scopedObservations = issueList(cloudIssue, "observations").filter((item) => itemAppliesToQuestion(item, question));
  const answerCandidates = scopedObservations.filter((item) => item.kind === "cloud_answer_candidate_only");
  if (scopedIssues.length) {
    return { tone: "mismatch", label: "云端有差异", details: scopedIssues.map(cloudDetailText) };
  }
  if (answerCandidates.length) {
    return { tone: "candidate", label: "云端候选", details: answerCandidates.map(cloudDetailText) };
  }
  if (issueString(cloudIssue, "failure")) {
    return { tone: "unknown", label: "云端未完成", details: [] };
  }
  const hasGlobalIssue = allIssues.some((item) => !itemHasQuestionScope(item));
  if (hasGlobalIssue && !cloudCoversQuestion(cloudIssue, question)) {
    return { tone: "unknown", label: "云端未定位", details: [] };
  }
  if (issueBool(cloudIssue, "passed") || cloudCoversQuestion(cloudIssue, question)) {
    return { tone: "ok", label: "云端一致", details: scopedObservations.filter((item) => item.kind !== "cloud_answer_candidate_only").map(cloudDetailText) };
  }
  return { tone: "unknown", label: "云端未覆盖", details: [] };
}

function buildVisionQuestionStatus(question: QuestionDraft, visionIssue: AuditIssue | undefined): QuestionStatus | undefined {
  if (!visionIssue) return undefined;
  const qid = questionIdFor(question);
  const filled = issueStringList(visionIssue, "filledQuestionIds");
  const missing = issueStringList(visionIssue, "missingQuestionIds");
  if (filled.includes(qid)) {
    return { tone: "ok", label: "视觉已补全", details: ["答案来自图片页视觉识别，发布前仍需人工确认。"] };
  }
  if (missing.includes(qid)) {
    return { tone: "mismatch", label: "视觉缺答案", details: ["视觉模型未能从图片答案页安全提取此题答案。"] };
  }
  return undefined;
}

function findLatestCloudIssue(rawAuditIssues: AuditIssue[], fallback?: NonNullable<AutoPipelineReport["quality"]>["cloudComparison"]): AuditIssue | undefined {
  const persistedIssue = [...rawAuditIssues]
    .reverse()
    .find((issue) => auditIssueKind(issue) === "cloud_comparison_summary" || auditIssuePath(issue).includes("cloudComparison"));
  return persistedIssue ?? fallback;
}

function questionTone(question: QuestionDraft): QuestionStatusTone {
  const answer = Array.isArray(question.answer) ? question.answer.join(", ").trim() : String(question.answer ?? "").trim();
  if (!answer) return "mismatch";
  if (question.verified) return "ok";
  if (question.confidence < 0.85) return "candidate";
  return "unknown";
}

function questionStatusLabel(question: QuestionDraft): string {
  const tone = questionTone(question);
  if (tone === "mismatch") return "缺答案";
  if (tone === "ok") return "已确认";
  if (tone === "candidate") return `低置信 ${Math.round(question.confidence * 100)}%`;
  return `待确认 ${Math.round(question.confidence * 100)}%`;
}

function groupSummary(group: QuestionGroupDraft): { missingAnswers: number; lowConfidence: number } {
  return group.questions.reduce(
    (summary, question) => {
      const answer = Array.isArray(question.answer) ? question.answer.join(", ").trim() : String(question.answer ?? "").trim();
      if (!answer) summary.missingAnswers += 1;
      if (question.confidence < 0.85 && !question.verified) summary.lowConfidence += 1;
      return summary;
    },
    { missingAnswers: 0, lowConfidence: 0 }
  );
}

function findQuestion(ir: ReadingAuthoringIr | undefined, questionId: string | undefined): { group: QuestionGroupDraft; question: QuestionDraft; groupIndex: number; questionIndex: number } | undefined {
  if (!ir || !questionId) return undefined;
  for (const [groupIndex, group] of ir.groups.entries()) {
    for (const [questionIndex, question] of group.questions.entries()) {
      if (question.id === questionId) return { group, question, groupIndex, questionIndex };
    }
  }
  return undefined;
}

function StatusBadge({ status }: { status: QuestionStatus }) {
  return (
    <span className={`cloud-status-badge ${status.tone}`} title={status.label}>
      <span aria-hidden="true">{questionStatusIcon[status.tone]}</span>
      {status.label}
    </span>
  );
}

export function UnifiedPreview({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [assets, setAssets] = useState<PreviewAssets | undefined>();
  const [report, setReport] = useState<ValidationReport | undefined>();
  const [pipelineReport, setPipelineReport] = useState<AutoPipelineReport | undefined>();
  const [ir, setIr] = useState<ReadingAuthoringIr | undefined>();
  const [activeGroupId, setActiveGroupId] = useState<string>("");
  const [selectedQuestionId, setSelectedQuestionId] = useState<string>("");
  const [saving, setSaving] = useState(false);
  const [runtimeError, setRuntimeError] = useState<string | undefined>();
  const [previewBusy, setPreviewBusy] = useState(false);
  const frameRef = useRef<HTMLIFrameElement | null>(null);

  const activeGroup = useMemo(
    () => ir?.groups.find((group) => group.groupId === activeGroupId) ?? ir?.groups[0],
    [ir, activeGroupId]
  );
  const selectedQuestion = useMemo(
    () => findQuestion(ir, selectedQuestionId) ?? findQuestion(ir, activeGroup?.questions[0]?.id),
    [ir, selectedQuestionId, activeGroup]
  );

  async function load(autoGenerate = false) {
    const detail = await getJob(jobId);
    setIr(detail.authoringIr);
    setAssets(detail.previewAssets);
    setReport(detail.validationReport);
    setPipelineReport(detail.pipelineReport);
    const firstGroupId = detail.authoringIr?.groups[0]?.groupId ?? "";
    const firstQuestionId = detail.authoringIr?.groups[0]?.questions[0]?.id ?? "";
    setActiveGroupId((current) => current || firstGroupId);
    setSelectedQuestionId((current) => current || firstQuestionId);
    if (autoGenerate && detail.authoringIr && !detail.previewAssets) {
      await refreshPreview(detail.authoringIr, false);
    }
  }

  useEffect(() => {
    load(true).catch((error) => setRuntimeError(error instanceof Error ? error.message : String(error)));
  }, [jobId]);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      const payload = event.data;
      if (!payload || typeof payload !== "object" || payload.source !== "author_preview_bridge") return;
      if (payload.type === "question-click" && typeof payload.questionId === "string") {
        const match = findQuestion(ir, payload.questionId);
        if (match) {
          setActiveGroupId(match.group.groupId);
          setSelectedQuestionId(match.question.id);
        }
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [ir]);

  useEffect(() => {
    if (!selectedQuestion?.question.id) return;
    frameRef.current?.contentWindow?.postMessage({
      source: "author_editor",
      type: "select-question",
      questionId: selectedQuestion.question.id
    }, "*");
  }, [selectedQuestion?.question.id, assets?.examId]);

  function replaceIr(next: ReadingAuthoringIr) {
    setIr(next);
  }

  function updateGroup(mutator: (group: QuestionGroupDraft) => QuestionGroupDraft) {
    if (!ir || !activeGroup) return;
    replaceIr({
      ...ir,
      groups: ir.groups.map((group) => (group.groupId === activeGroup.groupId ? mutator(group) : group))
    });
  }

  function updateQuestion(mutator: (question: QuestionDraft) => QuestionDraft) {
    if (!selectedQuestion) return;
    updateGroup((group) => ({
      ...group,
      questions: group.questions.map((question, index) => (index === selectedQuestion.questionIndex ? mutator(question) : question))
    }));
  }

  function verifyCurrentGroup() {
    updateGroup((group) => ({
      ...group,
      verified: true,
      questions: group.questions.map((question) => ({ ...question, verified: true }))
    }));
  }

  function verifyAllGroups() {
    if (!ir) return;
    replaceIr({
      ...ir,
      groups: ir.groups.map((group) => ({
        ...group,
        verified: true,
        questions: group.questions.map((question) => ({ ...question, verified: true }))
      }))
    });
  }

  async function saveDraft(nextIr = ir, regeneratePreview = false) {
    if (!nextIr) return;
    setSaving(true);
    setRuntimeError(undefined);
    try {
      const saved = await updateAuthoringIr(jobId, { ir: nextIr });
      setIr(saved);
      refresh();
      if (regeneratePreview) {
        await refreshPreview(saved, true);
      }
    } catch (error) {
      setRuntimeError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }

  async function refreshPreview(nextIr = ir, saveFirst = true) {
    if (!nextIr) return;
    setPreviewBusy(true);
    setRuntimeError(undefined);
    try {
      const latestIr = saveFirst ? await updateAuthoringIr(jobId, { ir: nextIr }) : nextIr;
      if (saveFirst) {
        setIr(latestIr);
        refresh();
      }
      const validation = await validateAuthoringIr(jobId);
      const nextAssets = await generatePreviewAssets(jobId);
      setReport(validation);
      setAssets(nextAssets);
      refresh();
      await load(false);
    } catch (error) {
      setRuntimeError(error instanceof Error ? error.message : String(error));
    } finally {
      setPreviewBusy(false);
    }
  }

  async function runRuntimeCheck() {
    try {
      const next = await runPreviewE2e(jobId);
      setReport(next);
      refresh();
    } catch (error) {
      setRuntimeError(error instanceof Error ? error.message : String(error));
    }
  }

  async function directExport() {
    if (!ir) return;
    setSaving(true);
    setRuntimeError(undefined);
    try {
      const saved = await updateAuthoringIr(jobId, { ir });
      setIr(saved);
      await validateAuthoringIr(jobId);
      window.sessionStorage.setItem(`${EXPORT_INTENT_KEY_PREFIX}${jobId}`, "single-js");
      refresh();
      go(`/jobs/${jobId}/export`);
    } catch (error) {
      setRuntimeError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }

  if (!ir || !activeGroup) {
    return <section className="page-enter"><p className="empty">题稿尚未生成。请先完成上传和自动处理。</p></section>;
  }

  const rawAuditIssues = ir.audit.issues ?? [];
  const auditIssues = rawAuditIssues.map(auditIssueText).filter(Boolean);
  const cloudIssue = findLatestCloudIssue(rawAuditIssues, pipelineReport?.quality?.cloudComparison);
  const visionAnswerIssue = rawAuditIssues.find((issue) => auditIssueKind(issue) === "vision_answer_extraction_summary");
  const visibleAuditIssues = auditIssues.filter((issue) => issue !== auditIssueText(cloudIssue ?? "") && issue !== auditIssueText(visionAnswerIssue ?? ""));
  const emptyAnswerCount = activeGroup.questions.filter((question) => {
    const answer = question.answer;
    return Array.isArray(answer) ? answer.length === 0 || answer.every((item) => !String(item).trim()) : !String(answer ?? "").trim();
  }).length;
  const cloudPassed = issueBool(cloudIssue, "attempted") && issueBool(cloudIssue, "passed");
  const cloudQuestionStatuses = Object.fromEntries(activeGroup.questions.map((question) => [question.id, buildCloudQuestionStatus(question, cloudIssue)]));
  const visionQuestionStatuses = Object.fromEntries(activeGroup.questions.map((question) => [question.id, buildVisionQuestionStatus(question, visionAnswerIssue)]));
  const cloudStatusCounts = activeGroup.questions.reduce<Record<QuestionStatusTone, number>>((counts, question) => {
    const tone = cloudQuestionStatuses[question.id]?.tone ?? "unknown";
    counts[tone] += 1;
    return counts;
  }, { ok: 0, candidate: 0, mismatch: 0, unknown: 0 });
  const selectedCloudStatus = selectedQuestion ? cloudQuestionStatuses[selectedQuestion.question.id] : undefined;
  const selectedVisionStatus = selectedQuestion ? visionQuestionStatuses[selectedQuestion.question.id] : undefined;
  const selectedStatusDetails = [...(selectedCloudStatus?.details ?? []), ...(selectedVisionStatus?.details ?? [])];
  const runtimeMode = report?.runtime?.mode;
  const srcDoc = assets?.runtimeHtml ?? (assets ? buildFallbackSrcDoc(assets) : "<p>正在准备统一阅读页预览...</p>");

  return (
    <section className="page-enter" data-testid="unified-preview">
      <div className="section-heading spread">
        <div>
          <p className="eyebrow">确认与编辑</p>
          <h2>统一阅读页确认与编辑</h2>
        </div>
        <div className="button-row">
          <button className="secondary" data-testid="verify-current-group" onClick={verifyCurrentGroup}>确认当前题组</button>
          <button className="secondary" data-testid="verify-all-groups" onClick={verifyAllGroups}>全部确认</button>
          <button className="primary" data-testid="save-draft" disabled={saving} onClick={() => void saveDraft(ir, false)}>
            {saving ? "正在保存..." : "保存修改"}
          </button>
          <button className="ghost" data-testid="refresh-preview" disabled={previewBusy || saving} onClick={() => void refreshPreview(ir, true)}>
            {previewBusy ? "正在刷新预览..." : "保存并刷新预览"}
          </button>
          <button className="primary" data-testid="direct-export" disabled={saving} onClick={() => void directExport()}>
            直接导出
          </button>
        </div>
      </div>

      {pipelineReport?.authoring?.remainingReviewItems ? (
        <div className="warning-box">
          <strong>当前还有 {pipelineReport.authoring.remainingReviewItems} 项需要确认</strong>
          <p>不再拆成单独页面处理。请直接在右侧查看低置信题目、缺失答案和题型提醒并修订。</p>
        </div>
      ) : null}
      {runtimeError ? <div className="warning-box"><strong>预览或保存未完成</strong><p>{runtimeError}</p></div> : null}

      <div className="editor-grid">
        <aside className="group-nav">
          {ir.groups.map((group) => {
            const summary = groupSummary(group);
            return (
              <button
                key={group.groupId}
                className={group.groupId === activeGroup.groupId ? "active" : ""}
                onClick={() => {
                  setActiveGroupId(group.groupId);
                  setSelectedQuestionId(group.questions[0]?.id ?? "");
                }}
              >
                <strong>{group.groupId}</strong>
                <span>Q{group.questionRange?.[0]}-{group.questionRange?.[1]}</span>
                <small>{group.requiresManualQuestionImport ? "需要补题干" : groupKindLabels[group.kind]}</small>
                <small>{group.verified ? "已确认" : `置信度 ${Math.round(group.confidence * 100)}%`}</small>
                {summary.missingAnswers || summary.lowConfidence || group.reviewWarnings?.length ? (
                  <small>
                    {summary.missingAnswers ? `${summary.missingAnswers} 题缺答案` : ""}
                    {summary.missingAnswers && (summary.lowConfidence || group.reviewWarnings?.length) ? " · " : ""}
                    {summary.lowConfidence ? `${summary.lowConfidence} 题低置信` : ""}
                    {summary.lowConfidence && group.reviewWarnings?.length ? " · " : ""}
                    {group.reviewWarnings?.length ? `${group.reviewWarnings.length} 条题型提醒` : ""}
                  </small>
                ) : null}
              </button>
            );
          })}
        </aside>

        <section className="live-preview">
          <p className="eyebrow">统一阅读页预览</p>
          <iframe
            ref={frameRef}
            className="html-preview-frame unified-runtime-frame"
            title="reading-preview"
            data-testid="reading-preview-frame"
            sandbox=""
            srcDoc={srcDoc}
          />
          <div className="info-box">
            <strong>当前预览运行时</strong>
            <p>{assets?.runtimeHtml ? "已切到 unifiedReadingPage.js 宿主渲染。" : "当前使用内置简化预览回退。"} {runtimeMode ? `检查状态：${runtimeModeLabel(runtimeMode)}。` : ""}</p>
            <small>在左侧预览里点击题目，会同步定位到右侧编辑面板；修改后点击“保存修改”或“保存并刷新预览”即可回写。</small>
          </div>
          <details>
            <summary>高级检查</summary>
            <div className="button-row" style={{ marginTop: 12 }}>
              <button className="ghost" data-testid="run-preview-e2e" disabled={!assets} onClick={() => void runRuntimeCheck()}>
                运行预览检查
              </button>
            </div>
            <dl>
              <dt>当前运行时</dt><dd>{runtimeModeLabel(runtimeMode)}</dd>
              <dt>校验层</dt><dd>{report?.layers.map((layer) => `${validationLayerLabel(layer.layer)}:${layer.issueCount}`).join(" · ") || "未运行"}</dd>
            </dl>
            <pre>{JSON.stringify(report?.runtime ?? {}, null, 2)}</pre>
          </details>
          {report?.issues.length ? (
            <details open>
              <summary>当前校验提醒</summary>
              <div className="issue-list compact" data-testid="preview-issue-list">
                {report.issues.slice(0, 8).map((issue) => {
                  const display = validationIssueDisplay(issue);
                  return (
                    <div key={issue.issueId}>
                      <strong>{display.title}</strong>
                      <small>{display.detail}</small>
                      <small>{display.action}</small>
                    </div>
                  );
                })}
              </div>
            </details>
          ) : null}
        </section>

        <aside className="form-section editor-form">
          <p className="eyebrow">编辑面板</p>
          <h3>{activeGroup.groupId}</h3>
          {visionAnswerIssue ? (
            <div className="info-box" data-testid="vision-answer-summary">
              <strong>视觉答案补全</strong>
              <p>{auditIssueText(visionAnswerIssue)}</p>
              {issueStringList(visionAnswerIssue, "filledQuestionIds").length ? <small>已写入：{issueStringList(visionAnswerIssue, "filledQuestionIds").join("、")}</small> : null}
              {issueStringList(visionAnswerIssue, "missingQuestionIds").length ? <small>仍缺少：{issueStringList(visionAnswerIssue, "missingQuestionIds").join("、")}</small> : null}
            </div>
          ) : null}
          {cloudIssue ? (
            <div className={cloudPassed ? "info-box" : "warning-box"} data-testid="cloud-comparison-summary">
              <div className="comparison-heading">
                <strong>云端整卷对照</strong>
                <span className={`cloud-status-badge ${cloudPassed ? "ok" : "mismatch"}`}>{cloudPassed ? "已通过" : "需核对"}</span>
              </div>
              <p>{auditIssueText(cloudIssue)}</p>
              <div className="comparison-metrics" data-testid="cloud-question-status-counts">
                <span><strong>{cloudStatusCounts.ok}</strong> 一致</span>
                <span><strong>{cloudStatusCounts.candidate}</strong> 候选</span>
                <span><strong>{cloudStatusCounts.mismatch}</strong> 差异</span>
                <span><strong>{cloudStatusCounts.unknown}</strong> 未覆盖</span>
              </div>
              {issueList(cloudIssue, "issues").slice(0, 4).map((issue, index) => <p key={`${index}-${issue.message}`}>{cloudDetailText(issue)}</p>)}
              {issueList(cloudIssue, "observations").filter((issue) => issue.kind !== "cloud_answer_candidate_only").slice(0, 2).map((issue, index) => <small key={`${index}-${issue.message}`}>{cloudDetailText(issue)}</small>)}
              <div className="compare-columns">
                <div>
                  <span>本地题稿</span>
                  {issueSummary(cloudIssue, "localSummary").slice(0, 6).map((item, index) => <small key={`local-${index}`}>{formatSummaryRow(item)}</small>)}
                </div>
                <div>
                  <span>云端对照</span>
                  {issueSummary(cloudIssue, "cloudSummary").slice(0, 6).map((item, index) => <small key={`cloud-${index}`}>{formatSummaryRow(item)}</small>)}
                </div>
              </div>
            </div>
          ) : null}
          {visibleAuditIssues.length ? (
            <div className="warning-box" data-testid="authoring-audit-warnings">
              <strong>需要确认</strong>
              {visibleAuditIssues.slice(0, 6).map((issue, index) => <p key={`${index}-${issue}`}>{issue}</p>)}
            </div>
          ) : null}
          {emptyAnswerCount ? (
            <div className="warning-box" data-testid="empty-answer-warning">
              <strong>当前题组仍有 {emptyAnswerCount} 题缺少答案</strong>
              <p>答案页是图片嵌入时，提取不到文字是正常现象；这里会直接提示缺失题号，用户在当前确认页补齐即可。</p>
            </div>
          ) : null}
          {ir.passage.questionUmbrellaRanges?.length ? (
            <div className="info-box" data-testid="umbrella-ranges">
              <strong>开头总题组范围已纳入</strong>
              {ir.passage.questionUmbrellaRanges.map((range) => (
                <p key={`${range.blockId}-${range.questionRange.join("-")}`}>
                  {range.heading}: Q{range.questionRange[0]}-{range.questionRange[1]}
                </p>
              ))}
            </div>
          ) : null}
          {activeGroup.requiresManualQuestionImport ? (
            <div className="warning-box" data-testid="manual-question-import-warning">
              <strong>需要人工补题</strong>
              <p>当前只识别到开头总范围 Q{activeGroup.questionRange?.[0]}-{activeGroup.questionRange?.[1]}，尚未检测到每道题的题干。请直接在这里补齐具体题干、答案并确认。</p>
            </div>
          ) : null}
          {activeGroup.reviewWarnings?.length ? (
            <div className="warning-box" data-testid="classification-review-warning">
              <strong>题型/选项规则需要确认</strong>
              {activeGroup.reviewWarnings.map((warning) => <p key={warning}>{warning}</p>)}
              {activeGroup.classificationEvidence?.length ? <small>依据段落：{activeGroup.classificationEvidence.join(", ")}</small> : null}
            </div>
          ) : null}
          {activeGroup.sectionEvidence?.length ? (
            <div className="info-box" data-testid="section-evidence">
              <strong>切分证据</strong>
              <small>
                {activeGroup.sectionEvidence.map((item) => `${item.blockId}@p${item.pageIndex}/c${item.column}${item.tableRows ? `/table:${item.tableRows}x${item.tableCols ?? "?"}` : ""}${item.headingLevel ? `/h${item.headingLevel}` : ""}${item.numberingLevel !== undefined ? `/num:${item.numberingLevel}` : ""}`).join(" -> ")}
              </small>
              {activeGroup.continuationEdges?.length ? (
                <small>
                  连续关系：{activeGroup.continuationEdges.map((edge) => `${edge.fromBlockId} 延续到 ${edge.toBlockId}（${edge.reason}）`).join("；")}
                </small>
              ) : null}
            </div>
          ) : null}
          {(activeGroup.kind === "matching" || activeGroup.kind === "heading_matching" || activeGroup.kind === "matching_information" || activeGroup.kind === "classification") ? (
            <label className="inline-check">
              <input type="checkbox" checked={activeGroup.allowOptionReuse === true} onChange={(event) => updateGroup((group) => ({ ...group, allowOptionReuse: event.target.checked }))} />
              允许选项重复使用
            </label>
          ) : null}
          <label>题型
            <select value={activeGroup.kind} onChange={(event) => updateGroup((group) => ({ ...group, kind: event.target.value as GroupKind }))}>
              {groupKinds.map((kind) => <option key={kind} value={kind}>{groupKindLabels[kind]}</option>)}
            </select>
          </label>
          <label>题组说明
            <textarea value={activeGroup.instruction.join("\n")} onChange={(event) => updateGroup((group) => ({ ...group, instruction: event.target.value.split("\n") }))} />
          </label>
          {activeGroup.reviewWarnings?.length ? (
            <div className="warning-box">
              <strong>题型提醒</strong>
              {activeGroup.reviewWarnings.map((warning) => <p key={warning}>{warning}</p>)}
            </div>
          ) : null}

          <div className="question-stack">
            {activeGroup.questions.map((question) => {
              const tone = questionTone(question);
              const active = selectedQuestion?.question.id === question.id;
              const cloudStatus = cloudQuestionStatuses[question.id];
              const visionStatus = visionQuestionStatuses[question.id];
              const statusDetails = [...(cloudStatus?.details ?? []), ...(visionStatus?.details ?? [])];
              const noteTone = cloudStatus?.tone === "mismatch" || visionStatus?.tone === "mismatch" ? "mismatch" : cloudStatus?.tone === "candidate" ? "candidate" : "ok";
              return (
                <button
                  key={question.id}
                  className={active ? "question-edit active-question-edit" : "question-edit"}
                  onClick={() => setSelectedQuestionId(question.id)}
                >
                  <div className="question-edit-header">
                    <strong>Q{question.displayNumber}</strong>
                    <div className="question-status-row">
                      <span className={`cloud-status-badge ${tone}`}>{questionStatusLabel(question)}</span>
                      {cloudStatus ? <StatusBadge status={cloudStatus} /> : null}
                      {visionStatus ? <StatusBadge status={visionStatus} /> : null}
                    </div>
                  </div>
                  {statusDetails.length ? (
                    <div className={`question-compare-note ${noteTone}`}>
                      {statusDetails.slice(0, 2).map((detail) => <p key={detail}>{detail}</p>)}
                    </div>
                  ) : null}
                  <p>{question.prompt || "当前题干为空，请补齐。"}</p>
                </button>
              );
            })}
          </div>

          {selectedQuestion ? (
            <>
              <div className="comparison-heading">
                <strong>Q{selectedQuestion.question.displayNumber}</strong>
                <div className="question-status-row">
                  <span className={`cloud-status-badge ${questionTone(selectedQuestion.question)}`}>{questionStatusLabel(selectedQuestion.question)}</span>
                  {selectedCloudStatus ? <StatusBadge status={selectedCloudStatus} /> : null}
                  {selectedVisionStatus ? <StatusBadge status={selectedVisionStatus} /> : null}
                </div>
              </div>
              {selectedStatusDetails.length ? (
                <div className={`question-compare-note ${selectedCloudStatus?.tone === "mismatch" || selectedVisionStatus?.tone === "mismatch" ? "mismatch" : selectedCloudStatus?.tone === "candidate" ? "candidate" : "ok"}`}>
                  {selectedStatusDetails.map((detail) => <p key={detail}>{detail}</p>)}
                </div>
              ) : null}
              <label>题号
                <input value={selectedQuestion.question.displayNumber} onChange={(event) => updateQuestion((question) => ({ ...question, displayNumber: event.target.value }))} />
              </label>
              <label>题干
                <textarea value={selectedQuestion.question.prompt} onChange={(event) => updateQuestion((question) => ({ ...question, prompt: event.target.value }))} />
              </label>
              <label>答案
                <input
                  value={Array.isArray(selectedQuestion.question.answer) ? selectedQuestion.question.answer.join(", ") : selectedQuestion.question.answer ?? ""}
                  onChange={(event) => updateQuestion((question) => ({ ...question, answer: event.target.value }))}
                />
              </label>
              <label className="inline-check">
                <input
                  type="checkbox"
                  checked={selectedQuestion.question.verified}
                  onChange={(event) => updateQuestion((question) => ({ ...question, verified: event.target.checked }))}
                /> 人工确认
              </label>
              <div className="info-box">
                <strong>识别置信度</strong>
                <p>{Math.round(selectedQuestion.question.confidence * 100)}%</p>
                <small>低于 85% 的题目建议优先核对；这里不再单独拆页面，只在当前确认页提示。</small>
              </div>
            </>
          ) : null}
        </aside>
      </div>
    </section>
  );
}
