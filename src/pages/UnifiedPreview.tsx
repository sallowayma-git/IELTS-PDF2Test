import { useEffect, useMemo, useRef, useState } from "react";
import {
  generatePreviewAssets,
  getJob,
  resolveSourceReview,
  runCloudReview,
  updateAuthoringIr,
  validateAuthoringIr
} from "../api/tauriCommands";
import { go } from "../app/router";
import { sanitizeHtml } from "../utils/sanitizeHtml";
import type { AutoPipelineReport, GroupKind, QuestionDraft, QuestionGroupDraft, ReadingAuthoringIr } from "../types";

const CLOUD_REVIEW_PENDING_KEY_PREFIX = "ielts-author-studio.cloud-review.";
const CLOUD_REVIEW_FAILED_KEY_PREFIX = "ielts-author-studio.cloud-review.failed.";
const CLOUD_REVIEW_QUEUE_STORAGE_KEY = "ielts-author-studio.cloud-review.queue";
const CLOUD_REVIEW_EVENT_NAME = "ielts-author-studio.cloud-review.event";
type AuditIssue = string | { message?: string; [key: string]: unknown };
type CloudReviewQueueItem = { jobId: string; profileId?: string };
type CloudReviewQueuePhase = "queued" | "running" | "completed" | "failed";
type CloudReviewQueueEventDetail = {
  jobId: string;
  phase: CloudReviewQueuePhase;
  report?: AutoPipelineReport;
  error?: string;
};
type CloudReviewRuntimeWindow = Window & typeof globalThis & {
  __IELTS_CLOUD_REVIEW_WORKER__?: Promise<void> | null;
};

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

type CloudState = "idle" | "queued" | "running" | "done" | "warning" | "unavailable" | "error";

function cloudReviewWindow(): CloudReviewRuntimeWindow | undefined {
  if (typeof window === "undefined") return undefined;
  return window as CloudReviewRuntimeWindow;
}

function readCloudReviewQueue(): CloudReviewQueueItem[] {
  const runtimeWindow = cloudReviewWindow();
  if (!runtimeWindow) return [];
  const raw = runtimeWindow.sessionStorage.getItem(CLOUD_REVIEW_QUEUE_STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as CloudReviewQueueItem[];
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((item): item is CloudReviewQueueItem => !!item?.jobId);
  } catch {
    runtimeWindow.sessionStorage.removeItem(CLOUD_REVIEW_QUEUE_STORAGE_KEY);
    return [];
  }
}

function writeCloudReviewQueue(queue: CloudReviewQueueItem[]): void {
  const runtimeWindow = cloudReviewWindow();
  if (!runtimeWindow) return;
  if (queue.length) {
    runtimeWindow.sessionStorage.setItem(CLOUD_REVIEW_QUEUE_STORAGE_KEY, JSON.stringify(queue));
    return;
  }
  runtimeWindow.sessionStorage.removeItem(CLOUD_REVIEW_QUEUE_STORAGE_KEY);
}

function removeCloudReviewQueueItem(jobId: string): void {
  writeCloudReviewQueue(readCloudReviewQueue().filter((item) => item.jobId !== jobId));
}

function emitCloudReviewQueueEvent(detail: CloudReviewQueueEventDetail): void {
  const runtimeWindow = cloudReviewWindow();
  runtimeWindow?.dispatchEvent(new CustomEvent<CloudReviewQueueEventDetail>(CLOUD_REVIEW_EVENT_NAME, { detail }));
}

export function pendingCloudReviewKey(jobId: string): string {
  return `${CLOUD_REVIEW_PENDING_KEY_PREFIX}${jobId}`;
}

function failedCloudReviewKey(jobId: string): string {
  return `${CLOUD_REVIEW_FAILED_KEY_PREFIX}${jobId}`;
}

export function isCloudReviewQueued(jobId: string): boolean {
  return readCloudReviewQueue().some((item) => item.jobId === jobId);
}

export function enqueueBackgroundCloudReview(jobId: string, profileId?: string | null): boolean {
  const normalizedProfileId = profileId && profileId !== "profile-local-placeholder" ? profileId : undefined;
  if (!normalizedProfileId) return false;

  const queue = readCloudReviewQueue();
  const existingIndex = queue.findIndex((item) => item.jobId === jobId);
  if (existingIndex >= 0) {
    if (!queue[existingIndex]?.profileId) {
      queue[existingIndex] = { ...queue[existingIndex], profileId: normalizedProfileId };
      writeCloudReviewQueue(queue);
    }
    return false;
  }

  writeCloudReviewQueue([...queue, { jobId, profileId: normalizedProfileId }]);
  emitCloudReviewQueueEvent({ jobId, phase: "queued" });
  return true;
}

export function startBackgroundCloudReviewScheduler(): void {
  const runtimeWindow = cloudReviewWindow();
  if (!runtimeWindow) return;
  if (runtimeWindow.__IELTS_CLOUD_REVIEW_WORKER__) return;

  runtimeWindow.__IELTS_CLOUD_REVIEW_WORKER__ = (async () => {
    try {
      while (true) {
        const next = readCloudReviewQueue()[0];
        if (!next) break;

        runtimeWindow.sessionStorage.setItem(pendingCloudReviewKey(next.jobId), "1");
        runtimeWindow.sessionStorage.removeItem(failedCloudReviewKey(next.jobId));
        emitCloudReviewQueueEvent({ jobId: next.jobId, phase: "running" });
        try {
          const report = await runCloudReview(next.jobId, next.profileId ? { profileId: next.profileId } : undefined);
          runtimeWindow.sessionStorage.removeItem(failedCloudReviewKey(next.jobId));
          emitCloudReviewQueueEvent({ jobId: next.jobId, phase: "completed", report });
        } catch (error) {
          runtimeWindow.sessionStorage.setItem(
            failedCloudReviewKey(next.jobId),
            error instanceof Error ? error.message : String(error)
          );
          emitCloudReviewQueueEvent({
            jobId: next.jobId,
            phase: "failed",
            error: error instanceof Error ? error.message : String(error)
          });
        } finally {
          runtimeWindow.sessionStorage.removeItem(pendingCloudReviewKey(next.jobId));
          removeCloudReviewQueueItem(next.jobId);
        }
      }
    } finally {
      runtimeWindow.__IELTS_CLOUD_REVIEW_WORKER__ = null;
      if (readCloudReviewQueue().length) startBackgroundCloudReviewScheduler();
    }
  })();
}

function mainSourceIsPdf(ir?: ReadingAuthoringIr): boolean {
  return ir?.exam.sourceFiles?.some((file) => file.role === "MainQuestion" && file.fileType === "pdf") ?? false;
}

function answerText(question: QuestionDraft): string {
  return Array.isArray(question.answer) ? question.answer.join(", ") : question.answer ?? "";
}

function answerValueFromInput(question: QuestionDraft, value: string): QuestionDraft["answer"] {
  if (Array.isArray(question.answer)) {
    return value
      .split(/[,\n，]/)
      .map((item) => item.trim())
      .filter(Boolean);
  }
  return value;
}

function promptPreview(prompt: string): string {
  const normalized = prompt.replace(/\s+/g, " ").trim();
  if (!normalized) return "题干待补充";
  if (normalized.length <= 72) return normalized;
  return `${normalized.slice(0, 72)}...`;
}

function instructionText(group: QuestionGroupDraft): string {
  return group.instruction.join("\n");
}

function questionRangeLabel(group: QuestionGroupDraft): string {
  if (group.questionRange) return `${group.questionRange[0]}-${group.questionRange[1]}`;
  if (group.questions.length === 1) return `${group.questions[0]?.displayNumber ?? "?"}`;
  return `${group.questions.length} 题`;
}

function cloudStateFromReport(report?: AutoPipelineReport, sourceIsPdf = false): { state: CloudState; label: string } {
  if (!sourceIsPdf) return { state: "unavailable", label: "仅本地预览" };
  const cloud = report?.quality?.cloudComparison;
  if (!cloud?.profileId || cloud.profileId === "profile-local-placeholder") return { state: "unavailable", label: "未配置云端模型" };
  if (!cloud.attempted) return { state: "idle", label: "等待云端复核" };
  if (cloud.failure) return { state: "error", label: "云端复核失败" };
  if (cloud.passed) return { state: "done", label: "云端复核已完成" };
  return { state: "warning", label: "云端提示有差异" };
}

function cloudStateIsTerminal(state: CloudState): boolean {
  return state === "done" || state === "warning" || state === "unavailable" || state === "error";
}

function startBackgroundArtifacts(jobId: string): void {
  void Promise.all([
    validateAuthoringIr(jobId),
    generatePreviewAssets(jobId)
  ]).catch(() => undefined);
}

function auditIssueText(issue: AuditIssue | undefined): string {
  if (!issue) return "";
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

function issueBool(issue: AuditIssue | undefined, key: string): boolean {
  if (!issue || typeof issue === "string") return false;
  return issue[key] === true;
}

function issueString(issue: AuditIssue | undefined, key: string): string {
  if (!issue || typeof issue === "string") return "";
  return typeof issue[key] === "string" ? issue[key] : "";
}

function issueStringList(issue: AuditIssue | undefined, key: string): string[] {
  if (!issue || typeof issue === "string") return [];
  const value = issue[key];
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
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

export function UnifiedPreview({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [ir, setIr] = useState<ReadingAuthoringIr | undefined>();
  const [pipelineReport, setPipelineReport] = useState<AutoPipelineReport | undefined>();
  const [runtimeError, setRuntimeError] = useState<string | undefined>();
  const [saving, setSaving] = useState(false);
  const [cloudState, setCloudState] = useState<CloudState>("idle");
  const [cloudLabel, setCloudLabel] = useState("等待云端复核");
  const [activeGroupId, setActiveGroupId] = useState<string | undefined>();
  const [activeQuestionId, setActiveQuestionId] = useState<string | undefined>();
  const cloudPollTimer = useRef<number | undefined>(undefined);

  async function load(autoWarm = false) {
    const detail = await getJob(jobId);
    const nextIr = detail.authoringIr;
    const nextPipeline = detail.pipelineReport;
    setIr(nextIr);
    setPipelineReport(nextPipeline);

    const sourceIsPdf = mainSourceIsPdf(nextIr);
    const nextCloud = cloudStateFromReport(nextPipeline, sourceIsPdf);
    const pending = window.sessionStorage.getItem(pendingCloudReviewKey(jobId)) === "1";
    const queued = isCloudReviewQueued(jobId);
    const failedMessage = window.sessionStorage.getItem(failedCloudReviewKey(jobId));
    if (pending && !cloudStateIsTerminal(nextCloud.state)) {
      setCloudState("running");
      setCloudLabel("云端复核中");
    } else if (queued && !cloudStateIsTerminal(nextCloud.state)) {
      setCloudState("queued");
      setCloudLabel("已加入云端复核队列");
    } else if (failedMessage && !cloudStateIsTerminal(nextCloud.state)) {
      setCloudState("error");
      setCloudLabel("云端复核失败");
    } else {
      setCloudState(nextCloud.state);
      setCloudLabel(nextCloud.label);
    }

    if (autoWarm && nextIr) {
      startBackgroundArtifacts(jobId);
    }
  }

  useEffect(() => {
    load(true).catch((error) => setRuntimeError(error instanceof Error ? error.message : String(error)));
  }, [jobId]);

  useEffect(() => {
    const listener = (event: Event) => {
      const detail = (event as CustomEvent<CloudReviewQueueEventDetail>).detail;
      if (!detail || detail.jobId !== jobId) return;

      if (detail.phase === "queued") {
        setCloudState("queued");
        setCloudLabel("已加入云端复核队列");
        return;
      }
      if (detail.phase === "running") {
        setCloudState("running");
        setCloudLabel("云端复核中");
        return;
      }
      if (detail.phase === "completed" && detail.report) {
        void load(false)
          .then(() => {
            setRuntimeError(undefined);
            refresh();
          })
          .catch((error) => setRuntimeError(error instanceof Error ? error.message : String(error)));
        return;
      }
      if (detail.phase === "failed") {
        setCloudState("error");
        setCloudLabel("云端复核失败");
        setRuntimeError(detail.error ?? "云端复核失败");
      }
    };

    window.addEventListener(CLOUD_REVIEW_EVENT_NAME, listener as EventListener);
    return () => window.removeEventListener(CLOUD_REVIEW_EVENT_NAME, listener as EventListener);
  }, [jobId, refresh]);

  useEffect(() => {
    if (!ir?.groups.length) {
      setActiveGroupId(undefined);
      return;
    }
    setActiveGroupId((current) => current && ir.groups.some((group) => group.groupId === current) ? current : ir.groups[0].groupId);
  }, [ir]);

  const activeGroup = useMemo(() => {
    if (!ir?.groups.length) return undefined;
    return ir.groups.find((group) => group.groupId === activeGroupId) ?? ir.groups[0];
  }, [activeGroupId, ir]);

  useEffect(() => {
    if (!activeGroup?.questions.length) {
      setActiveQuestionId(undefined);
      return;
    }
    setActiveQuestionId((current) => current && activeGroup.questions.some((question) => question.id === current) ? current : activeGroup.questions[0].id);
  }, [activeGroup]);

  const activeQuestion = useMemo(() => {
    if (!activeGroup?.questions.length) return undefined;
    return activeGroup.questions.find((question) => question.id === activeQuestionId) ?? activeGroup.questions[0];
  }, [activeGroup, activeQuestionId]);

  useEffect(() => {
    const pending = window.sessionStorage.getItem(pendingCloudReviewKey(jobId)) === "1";
    if (!pending) return undefined;
    cloudPollTimer.current = window.setInterval(() => {
      void getJob(jobId).then((detail) => {
        const nextReport = detail.pipelineReport;
        if (!nextReport?.quality?.cloudComparison?.attempted) return;
        setIr(detail.authoringIr);
        setPipelineReport(nextReport);
        const nextCloud = cloudStateFromReport(nextReport, mainSourceIsPdf(detail.authoringIr));
        setCloudState(nextCloud.state);
        setCloudLabel(nextCloud.label);
        window.sessionStorage.removeItem(pendingCloudReviewKey(jobId));
        if (cloudPollTimer.current) window.clearInterval(cloudPollTimer.current);
      }).catch(() => undefined);
    }, 1600);
    return () => {
      if (cloudPollTimer.current) window.clearInterval(cloudPollTimer.current);
    };
  }, [jobId, pipelineReport?.quality?.cloudComparison?.attempted]);

  useEffect(() => {
    if (!ir || !mainSourceIsPdf(ir)) return;

    const comparison = pipelineReport?.quality?.cloudComparison;
    const profileId = pipelineReport?.llm?.profileId;
    if (comparison?.attempted || !profileId || profileId === "profile-local-placeholder") return;
    if (window.sessionStorage.getItem(failedCloudReviewKey(jobId))) return;

    const pending = window.sessionStorage.getItem(pendingCloudReviewKey(jobId)) === "1";
    if (pending) {
      setCloudState("running");
      setCloudLabel("云端复核中");
      startBackgroundCloudReviewScheduler();
      return;
    }

    if (isCloudReviewQueued(jobId)) {
      setCloudState("queued");
      setCloudLabel("已加入云端复核队列");
      startBackgroundCloudReviewScheduler();
      return;
    }

    if (enqueueBackgroundCloudReview(jobId, profileId)) {
      setCloudState("queued");
      setCloudLabel("已加入云端复核队列");
    }
    startBackgroundCloudReviewScheduler();
  }, [ir, jobId, pipelineReport?.llm?.profileId, pipelineReport?.quality?.cloudComparison?.attempted]);

  const passageBlocks = useMemo(() => ir?.passage.htmlBlocks ?? [], [ir]);
  const rawAuditIssues = ir?.audit.issues ?? [];
  const visionTranscriptionIssue = findLatestVisionTranscriptionIssue(rawAuditIssues, pipelineReport?.parser?.visionTranscription);
  const visionConfigHint = visionConfigurationHint(visionTranscriptionIssue);
  const groupQuestionCount = activeGroup?.questions.length ?? 0;
  const verifiedQuestionCount = activeGroup?.questions.filter((question) => question.verified).length ?? 0;
  const activeGroupMissingPromptIds = activeGroup ? missingPromptIdsForGroup(visionTranscriptionIssue, activeGroup) : [];
  const activeGroupIndex = activeGroup && ir
    ? ir.groups.findIndex((group) => group.groupId === activeGroup.groupId)
    : -1;
  const activeQuestionIndex = activeQuestion && activeGroup
    ? activeGroup.questions.findIndex((question) => question.id === activeQuestion.id)
    : -1;
  const focusedSourceIds = useMemo(() => {
    const ids = activeQuestion?.sourceBlockIds?.length ? activeQuestion.sourceBlockIds : activeGroup?.sourceBlockIds ?? [];
    return new Set(ids);
  }, [activeGroup?.sourceBlockIds, activeQuestion?.sourceBlockIds]);

  function updateGroup(groupId: string, mutator: (group: QuestionGroupDraft) => QuestionGroupDraft) {
    setIr((current) => current ? {
      ...current,
      groups: current.groups.map((group) => group.groupId === groupId ? mutator(group) : group)
    } : current);
  }

  function updateQuestion(groupId: string, questionId: string, mutator: (question: QuestionDraft) => QuestionDraft) {
    setIr((current) => {
      if (!current) return current;
      let nextQuestion: QuestionDraft | undefined;
      const groups = current.groups.map((group) => group.groupId === groupId ? {
        ...group,
        questions: group.questions.map((question) => {
          if (question.id !== questionId) return question;
          nextQuestion = mutator(question);
          return nextQuestion;
        })
      } : group);
      if (!nextQuestion) return current;
      return {
        ...current,
        groups,
        answerKey: {
          ...current.answerKey,
          [questionId]: nextQuestion.answer ?? ""
        }
      };
    });
  }

  function updateQuestionAnswer(groupId: string, questionId: string, value: string) {
    updateQuestion(groupId, questionId, (question) => ({
      ...question,
      answer: answerValueFromInput(question, value)
    }));
  }

  function selectGroup(groupId: string) {
    setActiveGroupId(groupId);
    const nextGroup = ir?.groups.find((group) => group.groupId === groupId);
    setActiveQuestionId(nextGroup?.questions[0]?.id);
  }

  function jumpQuestion(offset: -1 | 1) {
    if (!activeGroup?.questions.length || !activeQuestion) return;
    const currentIndex = activeGroup.questions.findIndex((question) => question.id === activeQuestion.id);
    const nextQuestion = activeGroup.questions[currentIndex + offset];
    if (nextQuestion) setActiveQuestionId(nextQuestion.id);
  }

  function buildFullyVerifiedIr(current: ReadingAuthoringIr): ReadingAuthoringIr {
    return {
      ...current,
      groups: current.groups.map((group) => ({
        ...group,
        verified: true,
        questions: group.questions.map((question) => ({
          ...question,
          verified: true
        }))
      })),
      answerKey: current.groups.reduce((acc, group) => {
        for (const question of group.questions) {
          acc[question.id] = question.answer ?? "";
        }
        return acc;
      }, { ...current.answerKey } as ReadingAuthoringIr["answerKey"])
    };
  }

  async function verifyAllGroups() {
    if (!ir) return;
    const nextIr = buildFullyVerifiedIr(ir);
    setIr(nextIr);
    setSaving(true);
    setRuntimeError(undefined);
    try {
      const saved = await updateAuthoringIr(jobId, { ir: nextIr });
      await resolveSourceReview(jobId, "批量核对确认");
      setIr(saved);
      refresh();
      startBackgroundArtifacts(jobId);
    } catch (error) {
      setRuntimeError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }

  async function saveDraft(nextIr = ir) {
    if (!nextIr) return;
    setSaving(true);
    setRuntimeError(undefined);
    try {
      const saved = await updateAuthoringIr(jobId, { ir: nextIr });
      setIr(saved);
      refresh();
      startBackgroundArtifacts(jobId);
    } catch (error) {
      setRuntimeError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }

  async function openPublishCenter() {
    if (!ir) return;
    setSaving(true);
    setRuntimeError(undefined);
    try {
      const saved = await updateAuthoringIr(jobId, { ir });
      setIr(saved);
      startBackgroundArtifacts(jobId);
      refresh();
      go(`/jobs/${jobId}/export`);
    } catch (error) {
      setRuntimeError(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(false);
    }
  }

  if (!ir) {
    return <section className="page-enter"><p className="empty">题稿尚未生成。请先完成上传并执行本地粗切。</p></section>;
  }

  return (
    <section className="page-enter preview-workspace" data-testid="group-editor" data-view="unified-preview">
      <header className="preview-topbar">
        <div>
          <p className="eyebrow">确认与编辑</p>
          <h2>{ir.exam.title || "题稿预览"}</h2>
        </div>
        <div className="preview-actions">
          <div className={`cloud-indicator ${cloudState === "queued" ? "running" : cloudState}`} title={cloudLabel} aria-label={cloudLabel}>
            <span className="cloud-indicator-dot" aria-hidden="true" />
            <small>{cloudLabel}</small>
          </div>
          <button className="ghost" data-testid="verify-all-groups" onClick={() => void verifyAllGroups()} disabled={saving}>全部标记已核对</button>
          <button className="ghost" onClick={() => void saveDraft()} disabled={saving}>{saving ? "正在保存..." : "保存"}</button>
          <button className="primary" data-testid="validate-and-export" onClick={() => void openPublishCenter()} disabled={saving}>保存并前往发布</button>
        </div>
      </header>

      {runtimeError ? (
        <div className="warning-box compact-banner">
          <strong>当前操作未完成</strong>
          <p>{runtimeError}</p>
        </div>
      ) : null}
      {visionTranscriptionIssue ? (
        <div className="warning-box compact-banner" data-testid="vision-transcription-summary">
          <strong>视觉题目识别</strong>
          <p>{auditIssueText(visionTranscriptionIssue)}</p>
          {issueStringList(visionTranscriptionIssue, "missingPromptQuestionIds").length ? (
            <small>仍缺少题干：{issueStringList(visionTranscriptionIssue, "missingPromptQuestionIds").slice(0, 12).join("、")}</small>
          ) : null}
          {visionConfigHint ? <small>{visionConfigHint}</small> : null}
        </div>
      ) : null}

      <div className="preview-two-pane">
        <section className="preview-passage-pane">
          <div className="pane-heading">
            <div>
              <h3>原文</h3>
              <small>{passageBlocks.length} 个段落块</small>
            </div>
            <small className="passage-focus-note">
              {focusedSourceIds.size
                ? `已高亮 ${focusedSourceIds.size} 个关联段落`
                : "当前题目未绑定来源段落"}
            </small>
          </div>
          <article className="passage-sheet">
            {passageBlocks.map((block) => {
              const hasFocus = focusedSourceIds.size > 0;
              const focused = focusedSourceIds.has(block.blockId);
              return (
                <div
                  key={block.blockId}
                  className={`passage-block ${hasFocus ? (focused ? "active" : "muted") : ""}`}
                >
                  <div className="passage-block-meta">
                    <span>{block.blockId}</span>
                    {focused ? <strong>当前关联</strong> : null}
                  </div>
                  <div dangerouslySetInnerHTML={{ __html: sanitizeHtml(block.html) }} />
                </div>
              );
            })}
          </article>
        </section>

        <section className="preview-groups-pane">
          <div className="pane-heading">
            <div>
              <h3>题组工作台</h3>
              <small>{ir.groups.length} 组题目</small>
            </div>
            {activeGroup ? (
              <small>
                当前第 {activeGroupIndex + 1} 组 · 已核对 {verifiedQuestionCount}/{groupQuestionCount}
              </small>
            ) : null}
          </div>

          <div className="group-tab-row" role="tablist" aria-label="题组切换">
            {ir.groups.map((group, index) => {
              const active = group.groupId === activeGroup?.groupId;
              return (
                <button
                  key={group.groupId}
                  className={`group-tab ${active ? "active" : ""}`}
                  onClick={() => selectGroup(group.groupId)}
                  role="tab"
                  aria-selected={active}
                >
                  <strong>题组 {index + 1}</strong>
                  <span>{questionRangeLabel(group)} · {groupKindLabels[group.kind]}</span>
                  <small>{group.verified ? "已核对" : "待核对"}</small>
                  <small className={`confidence-note ${group.confidence < 0.85 ? "low" : ""}`}>置信度 {formatConfidence(group.confidence)}</small>
                </button>
              );
            })}
          </div>

          {activeGroup ? (
            <div className="preview-groups-workbench">
              <section className="group-meta-panel">
                <div className="group-meta-head">
                  <div>
                    <strong>{groupKindLabels[activeGroup.kind]}</strong>
                    <small>题号范围 {questionRangeLabel(activeGroup)}</small>
                  </div>
                  <label className="inline-check group-verified-toggle">
                    <input
                      type="checkbox"
                      checked={activeGroup.verified}
                      onChange={(event) => updateGroup(activeGroup.groupId, (group) => ({ ...group, verified: event.target.checked }))}
                    />
                    已核对本题组
                  </label>
                </div>

                <div className="group-meta-grid">
                  <label>
                    题型
                    <select
                      value={activeGroup.kind}
                      onChange={(event) => updateGroup(activeGroup.groupId, (group) => ({ ...group, kind: event.target.value as GroupKind }))}
                    >
                      {groupKinds.map((kind) => <option key={kind} value={kind}>{groupKindLabels[kind]}</option>)}
                    </select>
                  </label>
                  <label className="group-instruction-field">
                    题组说明
                    <textarea
                      value={instructionText(activeGroup)}
                      onChange={(event) => updateGroup(activeGroup.groupId, (group) => ({
                        ...group,
                        instruction: event.target.value.split("\n").map((line) => line.trim()).filter(Boolean)
                      }))}
                    />
                  </label>
                </div>

                {activeGroup.reviewWarnings?.length ? (
                  <div className="warning-box group-warning-box">
                    <strong>本题组仍有待确认项</strong>
                    <p>{activeGroup.reviewWarnings.join("；")}</p>
                  </div>
                ) : null}
                {(activeGroup.requiresManualQuestionImport || activeGroupMissingPromptIds.length) ? (
                  <div className="warning-box group-warning-box">
                    <strong>题干待补充</strong>
                    <p>当前题组仍有 {Math.max(activeGroupMissingPromptIds.length, activeGroup.questions.filter((question) => !question.prompt.trim()).length)} 题题干留空。系统不再自动补占位文本，请根据源文档补齐后再确认。</p>
                    {visionTranscriptionIssue ? <small>{auditIssueText(visionTranscriptionIssue)}</small> : null}
                    {visionConfigHint ? <small>{visionConfigHint}</small> : null}
                  </div>
                ) : null}
              </section>

              <div className="question-workbench">
                <aside className="question-list-pane">
                  <div className="question-list-head">
                    <strong>题目列表</strong>
                    <small>左侧选题，右侧细改；答案可选填。</small>
                  </div>
                  <div className="question-list-scroll">
                    {activeGroup.questions.map((question) => {
                      const active = question.id === activeQuestion?.id;
                      return (
                        <article key={question.id} className={`question-list-row ${active ? "active" : ""}`}>
                          <button className="question-row-select" onClick={() => setActiveQuestionId(question.id)}>
                            <div className="question-row-top">
                              <strong>{question.displayNumber}</strong>
                              <div className="question-row-meta">
                                <small className={`confidence-note ${question.confidence < 0.85 ? "low" : ""}`}>置信度 {formatConfidence(question.confidence)}</small>
                                <small className={`question-row-state ${question.verified ? "verified" : ""}`}>
                                  {question.verified ? "已核对" : "待核对"}
                                </small>
                              </div>
                            </div>
                            <p>{promptPreview(question.prompt)}</p>
                          </button>
                          <label className="question-quick-answer">
                            答案（可选）
                            <input
                              value={answerText(question)}
                              onChange={(event) => updateQuestionAnswer(activeGroup.groupId, question.id, event.target.value)}
                              onFocus={() => setActiveQuestionId(question.id)}
                            />
                          </label>
                        </article>
                      );
                    })}
                  </div>
                </aside>

                <section className="question-detail-pane">
                  {activeQuestion ? (
                    <div className="question-detail-card">
                      <div className="question-detail-head">
                        <div>
                          <p className="eyebrow">当前题目</p>
                          <h3>{activeQuestion.displayNumber}</h3>
                          <small>
                            第 {activeQuestionIndex + 1} 题 / 共 {groupQuestionCount} 题 · 交互 {activeQuestion.interaction.type} · 置信度 {formatConfidence(activeQuestion.confidence)}
                          </small>
                        </div>
                        <div className="question-nav-actions">
                          <button className="ghost small" onClick={() => jumpQuestion(-1)} disabled={activeQuestionIndex <= 0}>上一题</button>
                          <button className="ghost small" onClick={() => jumpQuestion(1)} disabled={activeQuestionIndex >= groupQuestionCount - 1}>下一题</button>
                        </div>
                      </div>

                      <div className="source-chip-row">
                        {(activeQuestion.sourceBlockIds.length ? activeQuestion.sourceBlockIds : activeGroup.sourceBlockIds).map((blockId) => (
                          <span key={blockId} className="source-chip">{blockId}</span>
                        ))}
                      </div>

                      {!activeQuestion.prompt.trim() ? (
                        <div className="warning-box">
                          <strong>当前题干为空</strong>
                          <p>视觉题目识别没有给出可靠题干，系统已保留空白，等待人工补齐。</p>
                          {visionConfigHint ? <small>{visionConfigHint}</small> : null}
                        </div>
                      ) : null}

                      <label>
                        题干
                        <textarea
                          value={activeQuestion.prompt}
                          onChange={(event) => updateQuestion(activeGroup.groupId, activeQuestion.id, (question) => ({
                            ...question,
                            prompt: event.target.value
                          }))}
                        />
                      </label>

                      <div className="question-detail-grid">
                        <label>
                          答案（可选）
                          <input
                            value={answerText(activeQuestion)}
                            onChange={(event) => updateQuestionAnswer(activeGroup.groupId, activeQuestion.id, event.target.value)}
                          />
                        </label>
                        <label className="inline-check detail-check">
                          <input
                            type="checkbox"
                            checked={activeQuestion.verified}
                            onChange={(event) => updateQuestion(activeGroup.groupId, activeQuestion.id, (question) => ({
                              ...question,
                              verified: event.target.checked
                            }))}
                          />
                          已核对本题
                        </label>
                      </div>
                    </div>
                  ) : (
                    <p className="empty">当前题组暂无题目。</p>
                  )}
                </section>
              </div>
            </div>
          ) : (
            <p className="empty">当前没有可编辑的题组。</p>
          )}
        </section>
      </div>
    </section>
  );
}
