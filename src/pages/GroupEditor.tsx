import { useEffect, useMemo, useState } from "react";
import { getJob, updateAuthoringIr, validateAuthoringIr } from "../api/tauriCommands";
import { go } from "../app/router";
import type { AutoPipelineReport, GroupKind, QuestionGroupDraft, ReadingAuthoringIr } from "../types";
import { renderGroupBodyHtml } from "../services/templateRenderer";

const groupKinds: GroupKind[] = ["true_false_not_given", "yes_no_not_given", "single_choice", "multi_choice", "short_answer", "sentence_completion", "summary_completion", "table_completion", "matching", "heading_matching", "matching_information", "classification", "diagram_completion"];

const groupKindLabels: Record<GroupKind, string> = {
  single_choice: "单选",
  multi_choice: "多选",
  true_false_not_given: "判断题 TRUE/FALSE/NOT GIVEN",
  yes_no_not_given: "判断题 YES/NO/NOT GIVEN",
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
type QuestionDraftView = QuestionGroupDraft["questions"][number];
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

const EXPORT_INTENT_KEY_PREFIX = "ielts-author-studio.export-intent.";

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
  if (!issue) return [];
  if (typeof issue === "string") return [];
  const value = issue[key];
  return Array.isArray(value) ? value.filter((item): item is { message?: string; [key: string]: unknown } => Boolean(item && typeof item === "object")) : [];
}

function issueStringList(issue: AuditIssue | undefined, key: string): string[] {
  if (!issue) return [];
  if (typeof issue === "string") return [];
  const value = issue[key];
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function issueSummary(issue: AuditIssue | undefined, key: string): Array<{ range?: unknown; kind?: unknown; layoutHint?: unknown; questionIds?: unknown }> {
  if (!issue) return [];
  if (typeof issue === "string") return [];
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

function questionNumber(question: QuestionDraftView): number | undefined {
  return parseQuestionNumber(question.displayNumber) ?? parseQuestionNumber(question.id);
}

function questionIdFor(question: QuestionDraftView): string {
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

function itemAppliesToQuestion(item: { [key: string]: unknown }, question: QuestionDraftView): boolean {
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

function summaryCoversQuestion(cloudIssue: AuditIssue | undefined, key: string, question: QuestionDraftView): boolean {
  const number = questionNumber(question);
  const qid = questionIdFor(question);
  return issueSummary(cloudIssue, key).some((item) => {
    const asRecord = item as { [key: string]: unknown };
    return rangeCovers(asRecord, number) || questionIdsInclude(item.questionIds, qid);
  });
}

function cloudCoversQuestion(cloudIssue: AuditIssue | undefined, question: QuestionDraftView): boolean {
  return summaryCoversQuestion(cloudIssue, "localSummary", question) || summaryCoversQuestion(cloudIssue, "cloudSummary", question);
}

function buildCloudQuestionStatus(question: QuestionDraftView, cloudIssue: AuditIssue | undefined): QuestionStatus {
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

function buildVisionQuestionStatus(question: QuestionDraftView, visionIssue: AuditIssue | undefined): QuestionStatus | undefined {
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

function buildVisionTranscriptionFallbackIssue(
  fallback?: NonNullable<NonNullable<AutoPipelineReport["parser"]>["visionTranscription"]>
): AuditIssue | undefined {
  if (!fallback) return undefined;
  const profileUnavailable = !fallback.profileId || fallback.profileId === "profile-local-placeholder";
  const message = !fallback.attempted && profileUnavailable
    ? "未配置可用云端模型，视觉题目识别未启动；当前仅保留本地解析结果，题干已留空。"
    : !fallback.attempted
      ? "视觉题目识别未启动；当前仅保留本地解析结果，题干已留空。"
      : !fallback.applied
        ? "视觉题目识别已尝试，但未生成可靠题组；当前保留本地解析结果，题干已留空。"
        : fallback.failure
          ? "视觉题目识别未成功完成；当前保留本地解析结果，题干已留空。"
          : "";
  if (!message) return undefined;
  return {
    kind: "vision_transcription_summary",
    path: "$.parser.visionTranscription",
    message,
    attempted: fallback.attempted,
    applied: fallback.applied,
    profileId: fallback.profileId ?? null,
    failure: fallback.failure ?? null,
    confidence: fallback.confidence ?? null,
    warnings: fallback.warnings ?? []
  };
}

function findLatestVisionTranscriptionIssue(
  rawAuditIssues: AuditIssue[],
  fallback?: NonNullable<NonNullable<AutoPipelineReport["parser"]>["visionTranscription"]>
): AuditIssue | undefined {
  const persistedIssue = [...rawAuditIssues]
    .reverse()
    .find((issue) => auditIssueKind(issue) === "vision_transcription_summary" || auditIssuePath(issue).includes("visionTranscription"));
  return persistedIssue ?? buildVisionTranscriptionFallbackIssue(fallback);
}

function formatConfidence(value: number | undefined): string {
  const normalized = typeof value === "number" && Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0;
  return `${Math.round(normalized * 100)}%`;
}

function emptyPromptCount(group: QuestionGroupDraft): number {
  return group.questions.filter((question) => !question.prompt.trim()).length;
}

function missingPromptIdsForGroup(issue: AuditIssue | undefined, group: QuestionGroupDraft): string[] {
  const missingIds = new Set(issueStringList(issue, "missingPromptQuestionIds"));
  return group.questions.filter((question) => missingIds.has(question.id) || !question.prompt.trim()).map((question) => question.id);
}

function visionConfigurationHint(issue: AuditIssue | undefined): string {
  const profileId = issueString(issue, "profileId");
  if (!issueBool(issue, "applied") && (!profileId || profileId === "profile-local-placeholder")) {
    return "当前没有可用的云端视觉模型配置，或默认本地视觉网关不可用。";
  }
  return "";
}

function StatusBadge({ status }: { status: QuestionStatus }) {
  return (
    <span className={`cloud-status-badge ${status.tone}`} title={status.label}>
      <span aria-hidden="true">{questionStatusIcon[status.tone]}</span>
      {status.label}
    </span>
  );
}

function buildGroupPreviewSrcDoc(group: QuestionGroupDraft): string {
  return `<!doctype html><html><head><meta charset="utf-8"><style>
    body{font-family:Georgia,serif;margin:0;padding:16px;background:#fffaf0;color:#17211f;line-height:1.55}
    .reading-question-group{display:block}
    .choice-row{display:flex;gap:8px;flex-wrap:wrap}
    .completion-table{width:100%;border-collapse:collapse}
    .completion-table th,.completion-table td{border:1px solid #d8d0c0;padding:8px}
    input{font:inherit;padding:6px}
    .notes-completion{white-space:pre-wrap}
    .inline-completion input{width:8em;margin:0 4px}
  </style></head><body>${renderGroupBodyHtml(group)}</body></html>`;
}

export function GroupEditor({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [ir, setIr] = useState<ReadingAuthoringIr | undefined>();
  const [pipelineReport, setPipelineReport] = useState<AutoPipelineReport | undefined>();
  const [activeGroupId, setActiveGroupId] = useState<string | undefined>();
  const [exportBusy, setExportBusy] = useState(false);
  const activeGroup = useMemo<QuestionGroupDraft | undefined>(() => ir?.groups.find((group) => group.groupId === activeGroupId) ?? ir?.groups[0], [ir, activeGroupId]);

  async function load() {
    const detail = await getJob(jobId);
    setIr(detail.authoringIr);
    setPipelineReport(detail.pipelineReport);
    setActiveGroupId((current) => current ?? detail.authoringIr?.groups[0]?.groupId);
  }

  useEffect(() => {
    load().catch(console.error);
  }, [jobId]);

  async function save(next: ReadingAuthoringIr) {
    const saved = await updateAuthoringIr(jobId, { ir: next });
    setIr(saved);
    refresh();
  }

  async function validate() {
    await validateAuthoringIr(jobId);
    refresh();
    go(`/jobs/${jobId}/preview`);
  }

  if (!ir || !activeGroup) {
    return <section className="page-enter"><p className="empty">题稿尚未生成。请先开始生成题稿，或在题组确认页保存后进入题稿编辑。</p></section>;
  }

  const currentGroup = activeGroup;
  const rawAuditIssues = ir.audit.issues ?? [];
  const auditIssues = rawAuditIssues.map(auditIssueText).filter(Boolean);
  const cloudIssue = findLatestCloudIssue(rawAuditIssues, pipelineReport?.quality?.cloudComparison);
  const visionTranscriptionIssue = findLatestVisionTranscriptionIssue(rawAuditIssues, pipelineReport?.parser?.visionTranscription);
  const visionAnswerIssue = rawAuditIssues.find((issue) => auditIssueKind(issue) === "vision_answer_extraction_summary");
  const visibleAuditIssues = auditIssues.filter((issue) => issue !== auditIssueText(cloudIssue ?? "") && issue !== auditIssueText(visionAnswerIssue ?? "") && issue !== auditIssueText(visionTranscriptionIssue ?? ""));
  const emptyAnswerCount = currentGroup.questions.filter((question) => {
    const answer = question.answer;
    return Array.isArray(answer) ? answer.length === 0 || answer.every((item) => !String(item).trim()) : !String(answer ?? "").trim();
  }).length;
  const missingPromptIds = missingPromptIdsForGroup(visionTranscriptionIssue, currentGroup);
  const visionConfigHint = visionConfigurationHint(visionTranscriptionIssue);
  const cloudPassed = issueBool(cloudIssue, "attempted") && issueBool(cloudIssue, "passed");
  const cloudQuestionStatuses = Object.fromEntries(currentGroup.questions.map((question) => [question.id, buildCloudQuestionStatus(question, cloudIssue)]));
  const visionQuestionStatuses = Object.fromEntries(currentGroup.questions.map((question) => [question.id, buildVisionQuestionStatus(question, visionAnswerIssue)]));
  const cloudStatusCounts = currentGroup.questions.reduce<Record<QuestionStatusTone, number>>((counts, question) => {
    const tone = cloudQuestionStatuses[question.id]?.tone ?? "unknown";
    counts[tone] += 1;
    return counts;
  }, { ok: 0, candidate: 0, mismatch: 0, unknown: 0 });

  function updateGroup(mutator: (group: QuestionGroupDraft) => QuestionGroupDraft) {
    if (!ir || !currentGroup) return;
    const next: ReadingAuthoringIr = { ...ir, groups: ir.groups.map((group) => (group.groupId === currentGroup.groupId ? mutator(group) : group)) };
    void save(next);
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
    const next: ReadingAuthoringIr = {
      ...ir,
      groups: ir.groups.map((group) => ({
        ...group,
        verified: true,
        questions: group.questions.map((question) => ({ ...question, verified: true }))
      }))
    };
    void save(next);
  }

  async function validateAndExport() {
    if (!ir) return;
    setExportBusy(true);
    try {
      const saved = await updateAuthoringIr(jobId, { ir });
      setIr(saved);
      await validateAuthoringIr(jobId);
      window.sessionStorage.setItem(`${EXPORT_INTENT_KEY_PREFIX}${jobId}`, "single-js");
      refresh();
      go(`/jobs/${jobId}/export`);
    } finally {
      setExportBusy(false);
    }
  }

  return (
    <section className="group-editor page-enter" data-testid="group-editor">
      <div className="section-heading spread">
        <div><p className="eyebrow">题稿编辑</p><h2>题稿编辑</h2></div>
        <div className="button-row">
          <button className="ghost" data-testid="go-llm-review" onClick={() => go(`/jobs/${jobId}/llm-review`)}>需要确认的识别结果</button>
          <button className="ghost" data-testid="verify-current-group" onClick={verifyCurrentGroup}>确认当前题组</button>
          <button className="secondary" data-testid="verify-all-groups" onClick={verifyAllGroups}>全部确认</button>
          <button className="primary" data-testid="validate-and-preview" onClick={validate}>检查并预览</button>
          <button className="primary" data-testid="validate-and-export" disabled={exportBusy} onClick={validateAndExport}>{exportBusy ? "正在导出..." : "直接导出"}</button>
        </div>
      </div>
      <div className="editor-grid">
        <aside className="group-nav">
          {ir.groups.map((group) => (
            <button key={group.groupId} className={group.groupId === activeGroup.groupId ? "active" : ""} onClick={() => setActiveGroupId(group.groupId)}>
              <strong>{group.groupId}</strong>
              <span>Q{group.questionRange?.[0]}-{group.questionRange?.[1]}</span>
              <small>{group.requiresManualQuestionImport ? "题干待补充" : groupKindLabels[group.kind]}</small>
              <small className={`confidence-note ${group.confidence < 0.85 ? "low" : ""}`}>置信度 {formatConfidence(group.confidence)}</small>
            </button>
          ))}
        </aside>
        <section className="form-section editor-form">
          {visionTranscriptionIssue ? (
            <div className="warning-box" data-testid="vision-transcription-summary">
              <strong>视觉题目识别</strong>
              <p>{auditIssueText(visionTranscriptionIssue)}</p>
              {issueStringList(visionTranscriptionIssue, "missingPromptQuestionIds").length ? (
                <small>仍缺少题干：{issueStringList(visionTranscriptionIssue, "missingPromptQuestionIds").slice(0, 12).join("、")}</small>
              ) : null}
              {visionConfigHint ? <small>{visionConfigHint}</small> : null}
            </div>
          ) : null}
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
              <p>视觉模型可能未能从图片答案页安全提取这些题。请补齐答案后再确认导出。</p>
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
              <strong>题干待补充</strong>
              <p>当前只识别到开头总范围 Q{activeGroup.questionRange?.[0]}-{activeGroup.questionRange?.[1]}，尚未检测到每道题的可靠题干。该题组现已保留空题干 {Math.max(missingPromptIds.length, emptyPromptCount(activeGroup))} 题，不再自动补占位文本。</p>
              {visionTranscriptionIssue ? <small>{auditIssueText(visionTranscriptionIssue)}</small> : null}
              {visionConfigHint ? <small>{visionConfigHint}</small> : null}
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
            <label className="inline-check"><input type="checkbox" checked={activeGroup.allowOptionReuse === true} onChange={(event) => updateGroup((group) => ({ ...group, allowOptionReuse: event.target.checked }))} /> 允许选项重复使用</label>
          ) : null}
          <label>题型<select value={activeGroup.kind} onChange={(event) => updateGroup((group) => ({ ...group, kind: event.target.value as GroupKind }))}>{groupKinds.map((kind) => <option key={kind} value={kind}>{groupKindLabels[kind]}</option>)}</select></label>
          <label>说明<textarea value={activeGroup.instruction.join("\n")} onChange={(event) => updateGroup((group) => ({ ...group, instruction: event.target.value.split("\n") }))} /></label>
          <div className="question-stack">
            {activeGroup.questions.map((question, index) => {
              const cloudStatus = cloudQuestionStatuses[question.id];
              const visionStatus = visionQuestionStatuses[question.id];
              const promptMissing = !question.prompt.trim();
              const statusDetails = [
                ...(promptMissing ? ["题干未被可靠提取，当前保持留空，请人工补齐。"] : []),
                ...(cloudStatus?.details ?? []),
                ...(visionStatus?.details ?? [])
              ];
              const noteTone = promptMissing || cloudStatus?.tone === "mismatch" || visionStatus?.tone === "mismatch" ? "mismatch" : cloudStatus?.tone === "candidate" ? "candidate" : "ok";
              return (
                <div className="question-edit" key={question.id}>
                  <div className="question-edit-header">
                    <div>
                      <strong>Q{question.displayNumber}</strong>
                      <small className={`confidence-note ${question.confidence < 0.85 ? "low" : ""}`}>置信度 {formatConfidence(question.confidence)}</small>
                    </div>
                    <div className="question-status-row">
                      {cloudStatus ? <StatusBadge status={cloudStatus} /> : null}
                      {visionStatus ? <StatusBadge status={visionStatus} /> : null}
                    </div>
                  </div>
                  {statusDetails.length ? (
                    <div className={`question-compare-note ${noteTone}`}>
                      {statusDetails.slice(0, 3).map((detail) => <p key={detail}>{detail}</p>)}
                    </div>
                  ) : null}
                  <label>题号<input value={question.displayNumber} onChange={(event) => updateGroup((group) => ({ ...group, questions: group.questions.map((item, i) => i === index ? { ...item, displayNumber: event.target.value } : item) }))} /></label>
                  <label>题干<input value={question.prompt} onChange={(event) => updateGroup((group) => ({ ...group, questions: group.questions.map((item, i) => i === index ? { ...item, prompt: event.target.value } : item) }))} /></label>
                  <label>答案<input value={Array.isArray(question.answer) ? question.answer.join(",") : question.answer ?? ""} onChange={(event) => updateGroup((group) => ({ ...group, questions: group.questions.map((item, i) => i === index ? { ...item, answer: event.target.value } : item) }))} /></label>
                  <label className="inline-check"><input type="checkbox" checked={question.verified} onChange={(event) => updateGroup((group) => ({ ...group, questions: group.questions.map((item, i) => i === index ? { ...item, verified: event.target.checked } : item) }))} /> 人工确认</label>
                </div>
              );
            })}
          </div>
        </section>
        <aside className="live-preview">
          <p className="eyebrow">题组预览</p>
          <iframe className="html-preview-frame" title="group-body-preview" sandbox="" srcDoc={buildGroupPreviewSrcDoc(activeGroup)} />
          <details><summary>生成内容（调试）</summary><pre>{renderGroupBodyHtml(activeGroup)}</pre></details>
        </aside>
      </div>
    </section>
  );
}
