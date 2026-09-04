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
import { isPhase5EditorEnabled } from "../config/featureFlags";
import { sanitizeHtml } from "../utils/sanitizeHtml";
import {
  applyLlmSuggestion,
  applyVisionAnswerCandidates,
  listLlmProfiles,
  llmExtractGroup
} from "../api/tauriCommands";
import type { AutoPipelineReport, GroupKind, LlmSuggestion, PreviewAssets, QuestionDraft, QuestionGroupDraft, ReadingAuthoringIr, VisionAnswerCandidate, VisionAnswerCandidates } from "../types";

const CLOUD_REVIEW_PENDING_KEY_PREFIX = "ielts-author-studio.cloud-review.";
const CLOUD_REVIEW_FAILED_KEY_PREFIX = "ielts-author-studio.cloud-review.failed.";
const CLOUD_REVIEW_QUEUE_STORAGE_KEY = "ielts-author-studio.cloud-review.queue";
const CLOUD_REVIEW_LEASE_STORAGE_KEY = "ielts-author-studio.cloud-review.lease";
const CLOUD_REVIEW_LEASE_TTL_MS = 15 * 60 * 1000;
const CLOUD_REVIEW_EVENT_NAME = "ielts-author-studio.cloud-review.event";
const CLOUD_REVIEW_WORKER_ID = `worker-${Math.random().toString(36).slice(2)}-${Date.now()}`;
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

function cloudReviewStorage(): Storage | undefined {
  const runtimeWindow = cloudReviewWindow();
  if (!runtimeWindow) return undefined;
  try {
    return runtimeWindow.localStorage;
  } catch {
    return undefined;
  }
}

function cloudReviewErrorText(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  return text.replace(/\s+/g, " ").trim().slice(0, 500) || "云端复核失败";
}

function readCloudReviewQueue(): CloudReviewQueueItem[] {
  const storage = cloudReviewStorage();
  if (!storage) return [];
  const raw = storage.getItem(CLOUD_REVIEW_QUEUE_STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as CloudReviewQueueItem[];
    if (!Array.isArray(parsed)) return [];
    const deduped = new Map<string, CloudReviewQueueItem>();
    for (const item of parsed) {
      if (typeof item?.jobId !== "string" || !item.jobId.trim()) continue;
      const profileId = typeof item.profileId === "string" && item.profileId.trim() ? item.profileId : undefined;
      deduped.set(item.jobId, { jobId: item.jobId, profileId });
    }
    return [...deduped.values()];
  } catch {
    storage.removeItem(CLOUD_REVIEW_QUEUE_STORAGE_KEY);
    return [];
  }
}

function writeCloudReviewQueue(queue: CloudReviewQueueItem[]): void {
  const storage = cloudReviewStorage();
  if (!storage) return;
  try {
    if (queue.length) {
      storage.setItem(CLOUD_REVIEW_QUEUE_STORAGE_KEY, JSON.stringify(queue));
      return;
    }
    storage.removeItem(CLOUD_REVIEW_QUEUE_STORAGE_KEY);
  } catch {
    // A restricted WebView may deny persistent storage; the foreground run
    // still reports its own result, but no durable queue is claimed.
  }
}

function acquireCloudReviewLease(): boolean {
  const storage = cloudReviewStorage();
  if (!storage) return false;
  const now = Date.now();
  try {
    const raw = storage.getItem(CLOUD_REVIEW_LEASE_STORAGE_KEY);
    if (raw) {
      const lease = JSON.parse(raw) as { owner?: string; expiresAt?: number };
      if (lease.owner && lease.owner !== CLOUD_REVIEW_WORKER_ID && typeof lease.expiresAt === "number" && lease.expiresAt > now) {
        return false;
      }
    }
    storage.setItem(CLOUD_REVIEW_LEASE_STORAGE_KEY, JSON.stringify({
      owner: CLOUD_REVIEW_WORKER_ID,
      expiresAt: now + CLOUD_REVIEW_LEASE_TTL_MS
    }));
    const confirmed = JSON.parse(storage.getItem(CLOUD_REVIEW_LEASE_STORAGE_KEY) ?? "{}");
    return confirmed.owner === CLOUD_REVIEW_WORKER_ID;
  } catch {
    return false;
  }
}

function releaseCloudReviewLease(): void {
  const storage = cloudReviewStorage();
  if (!storage) return;
  try {
    const lease = JSON.parse(storage.getItem(CLOUD_REVIEW_LEASE_STORAGE_KEY) ?? "{}");
    if (lease.owner === CLOUD_REVIEW_WORKER_ID) storage.removeItem(CLOUD_REVIEW_LEASE_STORAGE_KEY);
  } catch {
    storage.removeItem(CLOUD_REVIEW_LEASE_STORAGE_KEY);
  }
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
  startBackgroundCloudReviewScheduler();
  return true;
}

export function startBackgroundCloudReviewScheduler(): void {
  const runtimeWindow = cloudReviewWindow();
  if (!runtimeWindow) return;
  if (runtimeWindow.__IELTS_CLOUD_REVIEW_WORKER__) return;
  if (!readCloudReviewQueue().length || !acquireCloudReviewLease()) return;

  runtimeWindow.__IELTS_CLOUD_REVIEW_WORKER__ = (async () => {
    try {
      while (true) {
        const next = readCloudReviewQueue()[0];
        if (!next) break;

        const storage = cloudReviewStorage();
        storage?.setItem(pendingCloudReviewKey(next.jobId), "1");
        storage?.removeItem(failedCloudReviewKey(next.jobId));
        emitCloudReviewQueueEvent({ jobId: next.jobId, phase: "running" });
        try {
          const report = await runCloudReview(next.jobId, next.profileId ? { profileId: next.profileId } : undefined);
          storage?.removeItem(failedCloudReviewKey(next.jobId));
          emitCloudReviewQueueEvent({ jobId: next.jobId, phase: "completed", report });
        } catch (error) {
          const errorText = cloudReviewErrorText(error);
          storage?.setItem(
            failedCloudReviewKey(next.jobId),
            errorText
          );
          emitCloudReviewQueueEvent({
            jobId: next.jobId,
            phase: "failed",
            error: errorText
          });
        } finally {
          storage?.removeItem(pendingCloudReviewKey(next.jobId));
          removeCloudReviewQueueItem(next.jobId);
        }
      }
    } finally {
      runtimeWindow.__IELTS_CLOUD_REVIEW_WORKER__ = null;
      releaseCloudReviewLease();
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

async function loadPreviewArtifacts(
  jobId: string,
  onCompileError?: (message: string) => void
): Promise<PreviewAssets | undefined> {
  try {
    await validateAuthoringIr(jobId);
  } catch {
    // Validation failures still leave authors on the edit surface; student HTML may still compile.
  }
  try {
    return await generatePreviewAssets(jobId);
  } catch (caught) {
    onCompileError?.(caught instanceof Error ? caught.message : String(caught));
    return undefined;
  }
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

function findLatestVisionAnswerIssue(rawAuditIssues: AuditIssue[]): AuditIssue | undefined {
  return [...rawAuditIssues]
    .reverse()
    .find((issue) => auditIssueKind(issue) === "vision_answer_extraction_summary" || auditIssuePath(issue).includes("visionAnswerExtraction"));
}

function candidateAnswerText(answer: unknown): string {
  if (typeof answer === "string") return answer;
  if (Array.isArray(answer)) return answer.map((item) => String(item)).filter(Boolean).join(" / ");
  if (answer === null || answer === undefined) return "";
  return String(answer);
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
  const [previewAssets, setPreviewAssets] = useState<PreviewAssets | undefined>();
  const [studentPreviewStatus, setStudentPreviewStatus] = useState<"idle" | "loading" | "ready" | "unavailable">("idle");
  const [runtimeError, setRuntimeError] = useState<string | undefined>();
  const [saving, setSaving] = useState(false);
  const [cloudState, setCloudState] = useState<CloudState>("idle");
  const [cloudLabel, setCloudLabel] = useState("等待云端复核");
  const [activeGroupId, setActiveGroupId] = useState<string | undefined>();
  const [activeQuestionId, setActiveQuestionId] = useState<string | undefined>();
  const [visionAnswerCandidates, setVisionAnswerCandidates] = useState<VisionAnswerCandidates | undefined>();
  const [dismissedCandidateNumbers, setDismissedCandidateNumbers] = useState<Set<string>>(new Set());
  const [adoptingCandidates, setAdoptingCandidates] = useState(false);
  const [previewCompileError, setPreviewCompileError] = useState<string | undefined>();
  const [llmProfileId, setLlmProfileId] = useState<string | undefined>();
  const [llmSuggestionBusy, setLlmSuggestionBusy] = useState(false);
  const [llmSuggestion, setLlmSuggestion] = useState<LlmSuggestion | undefined>();
  const [llmSuggestionError, setLlmSuggestionError] = useState<string | undefined>();
  const [suggestionSelection, setSuggestionSelection] = useState({ kind: true, layout: true, questions: true });
  const [dismissedSuggestionQuestions, setDismissedSuggestionQuestions] = useState<Set<string>>(new Set());
  const cloudPollTimer = useRef<number | undefined>(undefined);
  const previewRequestId = useRef(0);

  async function refreshStudentPreview() {
    const requestId = ++previewRequestId.current;
    setStudentPreviewStatus("loading");
    const assets = await loadPreviewArtifacts(jobId, (message) => setPreviewCompileError(message));
    if (requestId !== previewRequestId.current) return;
    if (assets?.source?.questionGroups?.length) {
      setPreviewAssets(assets);
      setStudentPreviewStatus("ready");
      return;
    }
    setPreviewAssets(undefined);
    setStudentPreviewStatus("unavailable");
  }

  async function load(autoWarm = false) {
    const detail = await getJob(jobId);
    const nextIr = detail.authoringIr;
    const nextPipeline = detail.pipelineReport;
    setIr(nextIr);
    setPipelineReport(nextPipeline);
    setVisionAnswerCandidates(detail.visionAnswerCandidates);

    const sourceIsPdf = mainSourceIsPdf(nextIr);
    const nextCloud = cloudStateFromReport(nextPipeline, sourceIsPdf);
    const storage = cloudReviewStorage();
    const pending = storage?.getItem(pendingCloudReviewKey(jobId)) === "1";
    const queued = isCloudReviewQueued(jobId);
    const failedMessage = storage?.getItem(failedCloudReviewKey(jobId));
    // Only a real completion/failure state (done/warning/error) may override
    // the background-review indicators. A missing report (localOnly pipeline
    // minimizes pipeline-report.json) surfaces as "unavailable", which must
    // not mask a running silent review as "未配置云端模型".
    const reachedTerminal = nextCloud.state === "done" || nextCloud.state === "warning" || nextCloud.state === "error";
    if (pending && !reachedTerminal) {
      setCloudState("running");
      setCloudLabel("云端复核中");
    } else if (queued && !reachedTerminal) {
      setCloudState("queued");
      setCloudLabel("已加入云端复核队列");
    } else if (failedMessage && nextCloud.state !== "done" && nextCloud.state !== "warning") {
      setCloudState("error");
      setCloudLabel("云端复核失败");
    } else {
      setCloudState(nextCloud.state);
      setCloudLabel(nextCloud.label);
    }

    if (autoWarm && nextIr) {
      void refreshStudentPreview();
    }
  }

  useEffect(() => {
    load(true).catch((error) => setRuntimeError(error instanceof Error ? error.message : String(error)));
  }, [jobId]);

  useEffect(() => {
    listLlmProfiles()
      .then((items) =>
        setLlmProfileId(
          items.find((profile) => profile.enabled && profile.profileId !== "profile-local-placeholder")?.profileId
        )
      )
      .catch(() => setLlmProfileId(undefined));
  }, []);

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
    const storage = cloudReviewStorage();
    const pending = storage?.getItem(pendingCloudReviewKey(jobId)) === "1";
    if (!pending) return undefined;
    let attempts = 0;
    cloudPollTimer.current = window.setInterval(() => {
      attempts += 1;
      // Bounded polling: if the review never lands (worker died, queue lost),
      // stop after ~4 minutes instead of polling forever.
      if (attempts > 150) {
        storage?.removeItem(pendingCloudReviewKey(jobId));
        if (cloudPollTimer.current) window.clearInterval(cloudPollTimer.current);
        return;
      }
      void getJob(jobId).then((detail) => {
        const nextReport = detail.pipelineReport;
        if (!nextReport?.quality?.cloudComparison?.attempted) return;
        setIr(detail.authoringIr);
        setPipelineReport(nextReport);
        setVisionAnswerCandidates(detail.visionAnswerCandidates);
        const nextCloud = cloudStateFromReport(nextReport, mainSourceIsPdf(detail.authoringIr));
        setCloudState(nextCloud.state);
        setCloudLabel(nextCloud.label);
        storage?.removeItem(pendingCloudReviewKey(jobId));
        if (cloudPollTimer.current) window.clearInterval(cloudPollTimer.current);
      }).catch(() => undefined);
    }, 1600);
    return () => {
      if (cloudPollTimer.current) window.clearInterval(cloudPollTimer.current);
    };
  }, [jobId, pipelineReport?.quality?.cloudComparison?.attempted]);

  useEffect(() => {
    if (!ir || !mainSourceIsPdf(ir)) return;
    // A live queue from a previous page must keep a worker even when this
    // job's report carries no profile (localOnly minimize), so start the
    // scheduler unconditionally before any guard returns.
    startBackgroundCloudReviewScheduler();

    const comparison = pipelineReport?.quality?.cloudComparison;
    if (comparison?.attempted) return;
    const storage = cloudReviewStorage();
    if (storage?.getItem(failedCloudReviewKey(jobId))) return;

    const pending = storage?.getItem(pendingCloudReviewKey(jobId)) === "1";
    if (pending) {
      setCloudState("running");
      setCloudLabel("云端复核中");
      return;
    }
    if (isCloudReviewQueued(jobId)) {
      setCloudState("queued");
      setCloudLabel("已加入云端复核队列");
      return;
    }

    // Silent introduction: prefer the report's profile, fall back to the
    // enabled profile from settings (localOnly reports have a null profile).
    const profileId = pipelineReport?.llm?.profileId ?? llmProfileId;
    if (!profileId || profileId === "profile-local-placeholder") return;
    if (comparison?.failure) return;
    if (enqueueBackgroundCloudReview(jobId, profileId)) {
      setCloudState("queued");
      setCloudLabel("已加入云端复核队列");
    }
  }, [ir, jobId, llmProfileId, pipelineReport?.llm?.profileId, pipelineReport?.quality?.cloudComparison?.attempted, pipelineReport?.quality?.cloudComparison?.failure]);

  const passageBlocks = useMemo(() => ir?.passage.htmlBlocks ?? [], [ir]);
  const rawAuditIssues = ir?.audit.issues ?? [];
  const visionTranscriptionIssue = findLatestVisionTranscriptionIssue(rawAuditIssues, pipelineReport?.parser?.visionTranscription);
  const visionConfigHint = visionConfigurationHint(visionTranscriptionIssue);
  const visionAnswerIssue = findLatestVisionAnswerIssue(rawAuditIssues);
  const suggestionKindDiff = useMemo(() => {
    if (!llmSuggestion || !activeGroup) return undefined;
    const patchList = Array.isArray(llmSuggestion.patch) ? llmSuggestion.patch : [];
    const kindPatch = patchList.find((item) => item?.path === "/kind" && typeof item?.value === "string");
    const suggested = (kindPatch?.value ?? llmSuggestion.kind ?? "").trim();
    if (!suggested || suggested === activeGroup.kind) return undefined;
    return { current: activeGroup.kind, suggested: suggested as GroupKind };
  }, [llmSuggestion, activeGroup]);
  const suggestionLayoutDiff = useMemo(() => {
    if (!llmSuggestion || !activeGroup) return undefined;
    const patchList = Array.isArray(llmSuggestion.patch) ? llmSuggestion.patch : [];
    const layoutPatch = patchList.find((item) => item?.path === "/layout/template" && typeof item?.value === "string");
    const suggested = layoutPatch?.value?.trim();
    if (!suggested) return undefined;
    const current = String((activeGroup.layout as { template?: string } | undefined)?.template ?? "");
    if (suggested === current) return undefined;
    return { current: current || "（未设置）", suggested };
  }, [llmSuggestion, activeGroup]);
  const suggestionQuestionDiffs = useMemo(() => {
    if (!llmSuggestion || !activeGroup) return [];
    const currentById = new Map(activeGroup.questions.map((question) => [question.id, question]));
    return (llmSuggestion.questions ?? [])
      .filter((suggested) => currentById.has(suggested.id))
      .map((suggested) => {
        const current = currentById.get(suggested.id)!;
        const currentPrompt = current.prompt ?? "";
        const suggestedPrompt = typeof suggested.prompt === "string" ? suggested.prompt : currentPrompt;
        const currentInteraction = current.interaction?.type;
        const suggestedInteraction = suggested.interaction?.type ?? currentInteraction;
        return {
          id: suggested.id,
          changed: suggestedPrompt !== currentPrompt || suggestedInteraction !== currentInteraction,
          currentPrompt,
          suggestedPrompt,
          interactionChanged: suggestedInteraction !== currentInteraction,
          currentInteraction,
          suggestedInteraction
        };
      })
      .filter((diff) => diff.changed);
  }, [llmSuggestion, activeGroup]);
  const suggestionApplyPaths = useMemo(() => {
    const paths: string[] = [];
    if (suggestionKindDiff && suggestionSelection.kind) paths.push("kind");
    if (suggestionLayoutDiff && suggestionSelection.layout) paths.push("layout");
    if (suggestionQuestionDiffs.length && suggestionSelection.questions) paths.push("questions");
    return paths;
  }, [suggestionKindDiff, suggestionLayoutDiff, suggestionQuestionDiffs.length, suggestionSelection]);
  const pendingVisionCandidates = useMemo(() => {
    if (!visionAnswerCandidates?.candidates?.length || !ir) return [];
    return visionAnswerCandidates.candidates.filter((candidate) => {
      if (dismissedCandidateNumbers.has(candidate.questionNumber)) return false;
      // Persisted dismissals (recorded by the backend on reject) stay hidden.
      const dismissedAt = candidate as { dismissedAt?: string };
      if (dismissedAt.dismissedAt) return false;
      const questionId = candidate.questionId ?? `q${candidate.questionNumber}`;
      const question = ir.groups.flatMap((group) => group.questions).find((item) => item.id === questionId);
      if (!question) return false;
      const current = candidateAnswerText(question.answer);
      return current.trim().length === 0;
    });
  }, [visionAnswerCandidates, dismissedCandidateNumbers, ir]);
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
  const studentGroupPreview = useMemo(() => {
    if (!activeGroup || !previewAssets?.source?.questionGroups?.length) return undefined;
    return previewAssets.source.questionGroups.find((group) => group.groupId === activeGroup.groupId);
  }, [activeGroup, previewAssets]);
  const incompleteChoiceCount = useMemo(() => {
    if (!activeGroup) return 0;
    return activeGroup.questions.filter((question) => {
      if (question.interaction.type !== "radio" && question.interaction.type !== "checkbox") {
        return false;
      }
      const labels = (question.interaction.options ?? []).map((item) => item.trim()).filter(Boolean);
      if (labels.length >= 2) return false;
      const optionTextCount = Object.values(question.interaction.optionTexts ?? {}).filter((text) => text.trim()).length;
      return optionTextCount < 2;
    }).length;
  }, [activeGroup]);

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

  async function decideVisionCandidate(candidate: VisionAnswerCandidate, accept: boolean) {
    setAdoptingCandidates(true);
    setRuntimeError(undefined);
    try {
      const result = await applyVisionAnswerCandidates(
        jobId,
        [{ questionNumber: candidate.questionNumber, accept }]
      );
      if (result?.authoringIr) setIr(result.authoringIr);
      if (!accept) {
        setDismissedCandidateNumbers((current) => new Set(current).add(candidate.questionNumber));
      }
      await load(false);
      refresh();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setRuntimeError(
        message.includes("vision_answer_candidates_stale")
          ? "视觉答案候选刚被后台复核刷新，请查看最新列表后重试。"
          : message
      );
    } finally {
      setAdoptingCandidates(false);
    }
  }

  async function fetchLlmSuggestion() {
    if (!activeGroup) return;
    if (!llmProfileId) {
      setLlmSuggestionError("尚未配置启用的云端模型；请在设置页添加 OpenAI 兼容模型后再试。");
      return;
    }
    setLlmSuggestionBusy(true);
    setLlmSuggestionError(undefined);
    try {
      const suggestion = await llmExtractGroup(jobId, activeGroup.groupId, llmProfileId);
      setLlmSuggestion(suggestion);
      setSuggestionSelection({ kind: true, layout: true, questions: true });
      setDismissedSuggestionQuestions(new Set());
    } catch (caught) {
      setLlmSuggestion(undefined);
      setLlmSuggestionError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setLlmSuggestionBusy(false);
    }
  }

  async function adoptLlmSuggestion() {
    if (!llmSuggestion) return;
    const selectedQuestions = (llmSuggestion.questions ?? [])
      .map((question) => question.id)
      .filter((id) => !dismissedSuggestionQuestions.has(id));
    setLlmSuggestionBusy(true);
    setRuntimeError(undefined);
    try {
      // Manual adoption after a visible diff review: userConfirmed skips only
      // the automatic confidence gate; evidence/quote checks still apply.
      const saved = await applyLlmSuggestion(jobId, llmSuggestion.suggestionId, suggestionApplyPaths, {
        questionIds: suggestionApplyPaths.includes("questions") ? selectedQuestions : undefined,
        userConfirmed: true
      });
      setLlmSuggestion(undefined);
      setDismissedSuggestionQuestions(new Set());
      setIr(saved);
      refresh();
      void refreshStudentPreview();
    } catch (caught) {
      setRuntimeError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setLlmSuggestionBusy(false);
    }
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
      void refreshStudentPreview();
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
      void refreshStudentPreview();
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
      void refreshStudentPreview();
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
          {isPhase5EditorEnabled() ? <button className="ghost" onClick={() => go(`/jobs/${jobId}/authoring-v2`)}>结构化编辑器 V2</button> : null}
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
      {visionAnswerIssue ? (
        <div className="warning-box compact-banner" data-testid="vision-answer-summary">
          <strong>视觉答案补全</strong>
          <p>{auditIssueText(visionAnswerIssue)}</p>
          {issueStringList(visionAnswerIssue, "missingQuestionIds").length ? (
            <small>仍缺少答案：{issueStringList(visionAnswerIssue, "missingQuestionIds").slice(0, 12).join("、")}</small>
          ) : null}
        </div>
      ) : null}
      {pendingVisionCandidates.length ? (
        <div className="warning-box compact-banner" data-testid="vision-answer-candidates">
          <strong>视觉答案候选（{pendingVisionCandidates.length}）</strong>
          <p>以下是视觉模型从答案页图片识别出的候选答案，尚未写入题稿；请逐题采用或忽略。</p>
          <ul className="vision-answer-candidate-list">
            {pendingVisionCandidates.slice(0, 20).map((candidate) => {
              const quote = candidate.evidence && typeof candidate.evidence === "object" && "quote" in candidate.evidence
                ? String((candidate.evidence as { quote?: unknown }).quote ?? "")
                : "";
              const pageIndex = candidate.evidence && typeof candidate.evidence === "object" && "pageIndex" in candidate.evidence
                ? (candidate.evidence as { pageIndex?: unknown }).pageIndex
                : undefined;
              return (
                <li key={candidate.questionNumber}>
                  <span>
                    <strong>{candidate.questionId ?? `q${candidate.questionNumber}`}</strong>
                    {" "}
                    <del className="ai-diff-del">（空）</del>
                    <ins className="ai-diff-add">{candidateAnswerText(candidate.answer) || "（空候选）"}</ins>
                    {typeof candidate.confidence === "number" ? `（置信度 ${formatConfidence(candidate.confidence)}）` : ""}
                    {quote ? ` · 第 ${pageIndex ?? "?"} 页 “${quote.slice(0, 40)}”` : ""}
                  </span>
                  <span className="vision-answer-candidate-actions">
                    <button data-testid={`vision-answer-adopt-${candidate.questionNumber}`} onClick={() => void decideVisionCandidate(candidate, true)} disabled={adoptingCandidates}>采用</button>
                    <button className="ghost" onClick={() => void decideVisionCandidate(candidate, false)} disabled={adoptingCandidates}>忽略</button>
                  </span>
                </li>
              );
            })}
          </ul>
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
                {activeGroup.llmReview ? (
                  <div className="warning-box group-warning-box" data-testid="group-llm-review">
                    <strong>AI 识别复核：{activeGroup.llmReview.status === "low_confidence" ? "低置信度，需要人工确认" : "自动采用被拦截，需要人工确认"}</strong>
                    <p>
                      模型建议题型：{groupKindLabels[activeGroup.llmReview.suggestedKind as GroupKind] ?? activeGroup.llmReview.suggestedKind ?? "未知"}
                      （置信度 {formatConfidence(activeGroup.llmReview.confidence)}）；本地题稿未被云端结果修改。
                    </p>
                    {activeGroup.llmReview.warnings?.length ? (
                      <small>{activeGroup.llmReview.warnings.join("；")}</small>
                    ) : null}
                  </div>
                ) : null}
                <div className="llm-suggestion-panel" data-testid="llm-suggestion-panel">
                  <div className="pane-heading">
                    <div>
                      <h3>AI 题组建议</h3>
                      <small>{llmProfileId ? "对当前题组重新识别，产出只读建议；需人工采用后才会修改题稿。" : "尚未配置可用的云端模型；可在设置页添加后使用。"}</small>
                    </div>
                    <button
                      className="ghost small"
                      data-testid="fetch-llm-suggestion"
                      onClick={() => void fetchLlmSuggestion()}
                      disabled={llmSuggestionBusy || !activeGroup}
                    >
                      {llmSuggestionBusy ? "识别中..." : "获取 AI 建议"}
                    </button>
                  </div>
                  {llmSuggestionError ? (
                    <p className="empty" data-testid="llm-suggestion-error">{llmSuggestionError}</p>
                  ) : null}
                  {llmSuggestion ? (
                    <div className="llm-suggestion-card" data-testid="llm-suggestion-card">
                      {/* Diff-style review: current value struck in red, AI
                          proposal in green; every section and question is
                          individually opt-in before anything is applied. */}
                      <div className="llm-suggestion-meta">
                        <span>置信度</span>
                        <strong>{formatConfidence(typeof llmSuggestion.confidence === "number" ? llmSuggestion.confidence : undefined)}</strong>
                        {typeof llmSuggestion.confidence === "number" && llmSuggestion.confidence < 0.85 ? (
                          <em className="low-confidence-chip">低置信度：请逐项核对后再应用</em>
                        ) : null}
                      </div>
                      {suggestionKindDiff ? (
                        <label className="llm-diff-section">
                          <input
                            type="checkbox"
                            checked={suggestionSelection.kind}
                            onChange={(event) => setSuggestionSelection((current) => ({ ...current, kind: event.target.checked }))}
                          />
                          <span className="llm-diff-line">
                            题型：<del className="ai-diff-del">{groupKindLabels[suggestionKindDiff.current] ?? suggestionKindDiff.current}</del>
                            <ins className="ai-diff-add">{groupKindLabels[suggestionKindDiff.suggested] ?? suggestionKindDiff.suggested}</ins>
                            <small>（题型应用会同步更新版式模板）</small>
                          </span>
                        </label>
                      ) : null}
                      {suggestionLayoutDiff ? (
                        <label className="llm-diff-section">
                          <input
                            type="checkbox"
                            checked={suggestionSelection.layout}
                            onChange={(event) => setSuggestionSelection((current) => ({ ...current, layout: event.target.checked }))}
                          />
                          <span className="llm-diff-line">
                            版式：<del className="ai-diff-del">{suggestionLayoutDiff.current}</del>
                            <ins className="ai-diff-add">{suggestionLayoutDiff.suggested}</ins>
                          </span>
                        </label>
                      ) : null}
                      {suggestionQuestionDiffs.length ? (
                        <div className="llm-diff-section">
                          <label className="llm-diff-section-head">
                            <input
                              type="checkbox"
                              checked={suggestionSelection.questions}
                              onChange={(event) => setSuggestionSelection((current) => ({ ...current, questions: event.target.checked }))}
                            />
                            <span>题目内容（{suggestionQuestionDiffs.length} 题有变化）</span>
                          </label>
                          {suggestionSelection.questions ? (
                            <ul className="llm-question-diff-list">
                              {suggestionQuestionDiffs.map((diff) => (
                                <li key={diff.id}>
                                  <label className="llm-question-diff-row">
                                    <input
                                      type="checkbox"
                                      checked={!dismissedSuggestionQuestions.has(diff.id)}
                                      onChange={(event) =>
                                        setDismissedSuggestionQuestions((current) => {
                                          const next = new Set(current);
                                          if (event.target.checked) next.delete(diff.id);
                                          else next.add(diff.id);
                                          return next;
                                        })
                                      }
                                    />
                                    <span className="llm-question-diff-text">
                                      <strong>{diff.id}</strong>
                                      <del className="ai-diff-del">{diff.currentPrompt || "（空题干）"}</del>
                                      <ins className="ai-diff-add">{diff.suggestedPrompt || "（空题干）"}</ins>
                                      {diff.interactionChanged ? (
                                        <small>交互类型将同步更新：{diff.currentInteraction ?? "未知"} → {diff.suggestedInteraction ?? "未知"}</small>
                                      ) : null}
                                    </span>
                                  </label>
                                </li>
                              ))}
                            </ul>
                          ) : null}
                        </div>
                      ) : null}
                      {llmSuggestion.warnings?.length ? (
                        <ul className="llm-suggestion-warnings">
                          {llmSuggestion.warnings.slice(0, 6).map((warning, index) => (
                            <li key={index}>{String(warning)}</li>
                          ))}
                        </ul>
                      ) : null}
                      <div className="llm-suggestion-actions">
                        <button
                          data-testid="apply-llm-suggestion"
                          onClick={() => void adoptLlmSuggestion()}
                          disabled={llmSuggestionBusy || !suggestionApplyPaths.length}
                        >
                          应用所选部分{suggestionApplyPaths.length ? `（${suggestionApplyPaths.join(" / ")}）` : ""}
                        </button>
                        <button className="ghost" onClick={() => setLlmSuggestion(undefined)} disabled={llmSuggestionBusy}>忽略</button>
                      </div>
                    </div>
                  ) : null}
                </div>

                {(activeGroup.requiresManualQuestionImport || activeGroupMissingPromptIds.length) ? (
                  <div className="warning-box group-warning-box">
                    <strong>题干待补充</strong>
                    <p>当前题组仍有 {Math.max(activeGroupMissingPromptIds.length, activeGroup.questions.filter((question) => !question.prompt.trim()).length)} 题题干留空。系统不再自动补占位文本，请根据源文档补齐后再确认。空题干会阻断发布。</p>
                    {visionTranscriptionIssue ? <small>{auditIssueText(visionTranscriptionIssue)}</small> : null}
                    {visionConfigHint ? <small>{visionConfigHint}</small> : null}
                  </div>
                ) : null}
                {incompleteChoiceCount > 0 ? (
                  <div className="warning-box group-warning-box" data-testid="incomplete-choice-warning">
                    <strong>选项不完整</strong>
                    <p>当前题组有 {incompleteChoiceCount} 道单选/多选题选项少于 2 个。发布前必须补齐完整选项。</p>
                  </div>
                ) : null}
              </section>

              <section className="student-runtime-pane" data-testid="student-runtime-preview">
                <div className="pane-heading">
                  <div>
                    <h3>学生端渲染对照</h3>
                    <small>与 NAS / 学生运行时同一套 bodyHtml 编译结果</small>
                  </div>
                  <button className="ghost small" type="button" onClick={() => void refreshStudentPreview()} disabled={studentPreviewStatus === "loading"}>
                    {studentPreviewStatus === "loading" ? "编译中..." : "刷新学生预览"}
                  </button>
                </div>
                {studentPreviewStatus === "loading" && !studentGroupPreview ? (
                  <p className="empty">正在编译学生端 HTML…</p>
                ) : null}
                {studentPreviewStatus === "unavailable" ? (
                  <>
                    <p className="empty">学生端预览暂不可用。请先保存完整题稿，或检查校验是否通过运行时门禁。</p>
                    {previewCompileError ? <p className="empty" data-testid="preview-compile-error">编译错误：{previewCompileError}</p> : null}
                  </>
                ) : null}
                {studentGroupPreview ? (
                  <article className="student-runtime-sheet">
                    {studentGroupPreview.leadHtml?.trim() ? (
                      <div
                        className="student-runtime-lead"
                        dangerouslySetInnerHTML={{ __html: sanitizeHtml(studentGroupPreview.leadHtml) }}
                      />
                    ) : null}
                    <div
                      className="student-runtime-body"
                      dangerouslySetInnerHTML={{ __html: sanitizeHtml(studentGroupPreview.bodyHtml || "") }}
                    />
                  </article>
                ) : studentPreviewStatus === "ready" ? (
                  <p className="empty">当前题组尚未出现在编译后的学生端 source 中。</p>
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
